//! **Where space minds think** (doc 44 unit MW.11, Q-44-28): off the world thread, on the
//! server's one think pool, as the arena's do.
//!
//! **The rule, which is the arena's.** A think is collected on the snapshot a step leaves
//! and handed to the pool; its intents land at the head of the owner's next step, through
//! the one applier, before the bubble moves again. A step is not taken until the thinks it
//! owes have answered. When the pool is late the owner's step is late and never different:
//! the bubble is *held* where it stands, the step that was due is taken as it was due (its
//! length and its IC second, [`Due`]), and the wall time it waited is not simulated. The
//! world thread does not wait: it
//! asks whether the batch is done ([`BatchHandle::is_done`]) and goes on. A think is a pure
//! function of what was collected ([`CollectedThink`]) and its answer is written at one
//! moment of the owner's own sequence, so which worker ran it, and when, cannot change a
//! command, a memory or an outcome.
//!
//! **Who owes thinks** ([`Owner`]): a bubble (the customs corvette and the raider, or the
//! interdictor, and the backup mind of each hull whose crew has no link); a hull the backup
//! mind flies outside every bubble; a hull's collector drones at a cluster. Each owner has
//! at most one batch in flight, and collects its next only when the last has landed, so an
//! actor thinks once for each step of its owner, which the space tick takes at most once a
//! second.
//!
//! **Two ways to settle** ([`ThinkHost::hold_late_steps`]). The server's world loop runs on
//! the wall clock and *holds*: nothing here blocks it. A host nobody has told so *waits*
//! for its workers when a step is due: that is a rig whose clock is moved by hand, where no
//! wall time passes between two ticks and a held step would only make a story count ticks
//! by chance. It is the arena's own split: its task tests on a virtual clock wait for the
//! real workers (`arena_task`'s `await_thinks`).
//!
//! **A runner is kept for each seal** ([`ThinkHost::job`]): by the digest the seal names,
//! built once and shared by every hull sealed with that source. The source store is
//! addressed by digest and never rewritten, so a digest's runner is never stale.
//!
//! **A tape replay runs no think.** [`super::bubble::BubbleInputTape::replay`] applies the
//! commands on the tape; nothing here is reachable from it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use mush_core::ObjId;
use mush_sim::DomainId;
use smol_str::SmolStr;

use crate::arena::card::line;
use crate::arena::core::{ClassTable, CoreClass, CoreProfile};
use crate::arena::mind::{CollectedThink, MindRunner, MindView, ThinkAnswer, ThinkResult};
use crate::arena::profile_rules::BoundProgram;
use crate::arena::profile_runtime::ProfileToken;
use crate::arena::runtime_card::{CardCatalog, ProfileKey, RuntimeCard, stock_definition};
use crate::arena::state::{ArenaDefinition, ContactId};
use crate::config::{ArenaConfig, ReflexConfig};
use crate::minds::{BatchHandle, MindPool};
use crate::store::TacsimStore;

/// **Who owes a batch of thinks** (the file head): one batch in flight for each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Owner {
    /// The actors of the bubble on this domain.
    Bubble(DomainId),
    /// A hull the backup mind flies outside every bubble.
    Adrift(ObjId),
    /// The collector drones of this hull, at a cluster.
    Cluster(ObjId),
}

/// **Which collection a batch is** (the file head): for a bubble, the IC second it opened
/// and the step the thinks were collected on. A batch whose mark is not the one its owner
/// expects is another bubble's, or another run's, and is dropped unread.
pub type Mark = (i64, u32);

/// Which seat a think is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Thinker {
    /// The customs corvette of a picket bubble.
    Corvette,
    /// The raider of a picket bubble.
    Raider,
    /// The interdictor of a bubble with no customs picket.
    Interdictor,
    /// The backup mind of a hull whose crew has no link.
    Backup,
    /// A collector drone.
    Drone,
}

/// **One actor's think, collected** (the file head): who it is for, the contact rows its
/// view numbered, and the job. `jobs` holds more than one only for the backup seat, which
/// tries its sources in order (her own seal, the house default, the compiled default): each
/// is the same think on another source, and the first that answers without a fault is hers.
#[derive(Debug)]
pub struct Ask {
    /// The seat.
    pub who: Thinker,
    /// The hull thinking.
    pub unit: ObjId,
    /// The think's sequence number, as the actor's record held it at collection.
    pub think_seq: u64,
    /// The ship each contact id of the view names, for the applier.
    pub contacts: BTreeMap<ContactId, String>,
    /// The job, once for each source to try, first choice first.
    pub jobs: Vec<CollectedThink>,
    /// **The mind sealed aboard whose think `jobs[0]` is** (doc 44 unit MW.13), when the
    /// seat's first source is one: the answer says whether it was that mind that answered.
    pub sealed: Option<Arc<SealedMind>>,
}

/// **One actor's think, answered**: [`Ask`] without its jobs, and the first answer that
/// did not fault. `None` when every source faulted, or the batch was lost to a runner's
/// panic: the actor does nothing this step and its record is left as it was.
#[derive(Debug)]
pub struct Answer {
    /// The seat.
    pub who: Thinker,
    /// The hull that thought.
    pub unit: ObjId,
    /// The sequence number the think was collected with.
    pub think_seq: u64,
    /// The ship each contact id of its view names.
    pub contacts: BTreeMap<ContactId, String>,
    /// The first clean answer.
    pub result: Option<ThinkResult>,
    /// **The mind sealed aboard, when the clean answer is its own** (doc 44 unit MW.13):
    /// `None` when a house default answered, or nothing did.
    pub sealed: Option<Arc<SealedMind>>,
}

/// **A step as it fell due**: what the tick that first found it due would have stepped the
/// owner with. A held step is taken with these, whenever it is taken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Due {
    /// The step's length, milliseconds of wall time.
    pub dt_wall_ms: u32,
    /// The world's IC second at that tick.
    pub ic_now_s: i64,
}

/// **What an owner's due step finds** ([`ThinkHost::settle`]).
#[derive(Debug)]
pub enum Owed {
    /// No batch is in flight for this owner under this mark.
    Nothing,
    /// The batch has not answered and the host holds late steps: the step is not taken.
    Held,
    /// The batch has answered: its answers in collection order, and the step to take (as
    /// it first fell due, if it was held).
    Ready {
        /// One for each [`Ask`], in the order they were collected.
        answers: Vec<Answer>,
        /// The step, as it fell due.
        due: Due,
    },
}

/// What the host has done, for a test and for the measurement lane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThinkStats {
    /// Thinks handed over, counting each source of a backup seat.
    pub thinks: u64,
    /// Batches handed over.
    pub batches: u64,
    /// Times a due step was held because its batch had not answered.
    pub held: u64,
    /// Runners built: one for each digest this host has thought with.
    pub runners_built: u64,
}

/// An owner's thinks on their way, or already answered on the submitting thread. Each
/// actor's ask is a pool batch of its own, in the order the asks were collected: the pool
/// loses a whole batch when a runner panics, and one actor's runner must not take the
/// other minds of its bubble with it.
#[derive(Debug)]
enum Flying {
    /// On the pool, a batch for each ask.
    Pooled(Vec<BatchHandle>),
    /// A host with no pool ran them where they were submitted, ask by ask.
    Answered(Vec<Vec<ThinkAnswer>>),
}

impl Flying {
    /// Whether every ask has answered.
    fn is_done(&self) -> bool {
        match self {
            Flying::Pooled(handles) => handles.iter().all(BatchHandle::is_done),
            Flying::Answered(_) => true,
        }
    }
}

/// What is kept of one [`Ask`] while its jobs are out.
#[derive(Debug)]
struct Asked {
    who: Thinker,
    unit: ObjId,
    think_seq: u64,
    contacts: BTreeMap<ContactId, String>,
    /// The mind sealed aboard whose think the ask's first job is ([`Ask::sealed`]).
    sealed: Option<Arc<SealedMind>>,
    /// Struck ([`ThinkHost::forget`]): its answer is not to land.
    struck: bool,
}

/// One owner's batch in flight.
#[derive(Debug)]
struct Flight {
    mark: Mark,
    asked: Vec<Asked>,
    flying: Flying,
    /// The owner's step as it fell due, from the tick that first held it.
    held: Option<Due>,
    /// How many ticks have found the step held.
    held_ticks: u32,
}

/// Held ticks after which a held step is said once in the log, and the interval at which
/// it is said again: a batch that late is a pool that is saturated or a runner that is
/// stuck, and its owner stands still until it answers.
const HELD_WARN_TICKS: u32 = 5;
const HELD_WARN_AGAIN_TICKS: u32 = 60;

/// **The host of every space think** (the file head): the pool, the runners by digest, and
/// the batches in flight. One for each [`TacsimStore`] ([`TacsimStore::space_thinks`]).
/// Nothing in it is persisted: an owner whose record says a think is owed and who finds
/// none in flight (a restart) collects it again from the state it kept.
#[derive(Debug)]
pub struct ThinkHost {
    /// The server's pool. `None` runs a batch on the thread that submits it, and only a
    /// unit test with no world can build such a host (`ThinkHost::inline`).
    pool: Option<Arc<MindPool>>,
    /// Whether a late batch holds its owner's step (the server) or is waited for (a rig).
    hold: AtomicBool,
    runners: Mutex<BTreeMap<[u8; 32], Arc<dyn MindRunner>>>,
    /// The mind sealed aboard each hull, as it was last made ready, with the sortie its
    /// seal names: one for a hull, replaced when her next claim's seal is first thought
    /// with.
    sealed: Mutex<BTreeMap<ObjId, (crate::arena::mind::SortieId, Arc<SealedMind>)>>,
    flights: Mutex<BTreeMap<Owner, Flight>>,
    thinks: AtomicU64,
    batches: AtomicU64,
    held: AtomicU64,
    runners_built: AtomicU64,
}

/// **A mind sealed aboard a hull, made ready to think** (doc 44 unit MW.13; Q-44-26): what
/// its seal says a think of it is held to. The class row is the seal's class (the hull's
/// core, capped by the mind's rating), the card is the seal's opening card, and the rule
/// program is the one the seal bound over the hull's own channels: the first space seal
/// that runs one. The source is the text the mind's claim retains, which the store need
/// not hold.
#[derive(Debug)]
pub struct SealedMind {
    /// The SHA-256 of `source`: the runner's key.
    source_sha256: [u8; 32],
    source: String,
    profile: CoreProfile,
    card: RuntimeCard,
    program: BoundProgram,
}

impl SealedMind {
    /// `seal`'s think terms over `classes`, with the `source` its claim retains. `None` for
    /// a seal that is not a scripted mind of this build's ABI, or whose cards do not
    /// resolve: such a seal does not think, and its seat's next source does.
    #[must_use]
    pub fn of(
        seal: &crate::arena::mind::MindSeal,
        source: &str,
        classes: &ClassTable,
    ) -> Option<SealedMind> {
        if seal.engine != crate::arena::mind::SealEngine::Scripted
            || seal.abi != crate::minds::MIND_ABI
        {
            return None;
        }
        let mut catalog = CardCatalog::default();
        catalog.admit(seal.cards.definition.clone()).ok()?;
        let card = catalog.resolve(seal.cards.selected.key(0)?).ok()?;
        let program = seal.cards.programs.first()?.clone();
        Some(SealedMind {
            source_sha256: seal.source_sha256,
            source: source.to_owned(),
            profile: classes.row(seal.class),
            card,
            program,
        })
    }

    /// The rule program the seal bound.
    #[must_use]
    pub fn program(&self) -> &BoundProgram {
        &self.program
    }

    /// The class row its thinks are held to.
    #[must_use]
    pub fn profile(&self) -> &CoreProfile {
        &self.profile
    }
}

/// **What one think is run with** (doc 44 unit MW.13).
#[derive(Clone, Debug)]
pub enum Source {
    /// A doctrine the store, or the binary, holds under this digest: the Class I row, the
    /// stock card and no rule program ([`ThinkHost::job`]).
    Doctrine([u8; 32]),
    /// A mind sealed aboard, on its own class row, card and rule program.
    Sealed(Arc<SealedMind>),
}

impl Source {
    /// The SHA-256 of the source it thinks with.
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        match self {
            Source::Doctrine(digest) => *digest,
            Source::Sealed(mind) => mind.source_sha256,
        }
    }
}

/// What every doctrine's space think is held to, the same for each: the Class I row, a
/// one-profile card, an empty rule program and a one-cell map. A doctrine's space seal runs
/// no rule program; a mind sealed aboard runs its own ([`SealedMind`]).
#[derive(Debug)]
struct Kit {
    profile: CoreProfile,
    card: RuntimeCard,
    program: BoundProgram,
    map: Arc<ArenaDefinition>,
}

static KIT: LazyLock<Option<Kit>> = LazyLock::new(|| {
    let profile = ClassTable::STOCK.row(CoreClass::I);
    let definition = stock_definition(
        &[(SmolStr::new_static("Line"), line(&ReflexConfig::default()))],
        CoreClass::I,
    )
    .ok()?;
    let mut catalog = CardCatalog::default();
    let digest = catalog.admit(definition).ok()?;
    let card = catalog
        .resolve(ProfileKey {
            definition: digest,
            index: 0,
        })
        .ok()?;
    let program = BoundProgram::bind(&[], &[], &[], &profile, 0).ok()?;
    let map = Arc::new(super::picket::minimal_space_arena_definition()?);
    Some(Kit {
        profile,
        card,
        program,
        map,
    })
});

impl ThinkHost {
    /// A host on `pool`, which waits for late batches until it is told to hold
    /// ([`ThinkHost::hold_late_steps`]).
    #[must_use]
    pub fn on_pool(pool: Arc<MindPool>) -> ThinkHost {
        ThinkHost::with(Some(pool))
    }

    /// A host with no pool: a batch is run on the thread that submits it. For a unit test
    /// that steps a bubble with no world, and for nothing else: no server has one.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn inline() -> ThinkHost {
        ThinkHost::with(None)
    }

    fn with(pool: Option<Arc<MindPool>>) -> ThinkHost {
        ThinkHost {
            pool,
            hold: AtomicBool::new(false),
            runners: Mutex::new(BTreeMap::new()),
            sealed: Mutex::new(BTreeMap::new()),
            flights: Mutex::new(BTreeMap::new()),
            thinks: AtomicU64::new(0),
            batches: AtomicU64::new(0),
            held: AtomicU64::new(0),
            runners_built: AtomicU64::new(0),
        }
    }

    /// **Hold late steps from now on** (the file head): the world loop's call, once, when it
    /// starts to run this world on the wall clock. After it nothing here waits for a
    /// worker.
    pub fn hold_late_steps(&self) {
        self.hold.store(true, Ordering::Relaxed);
    }

    /// Whether a late batch holds its owner's step.
    #[must_use]
    pub fn holds_late_steps(&self) -> bool {
        self.hold.load(Ordering::Relaxed)
    }

    /// How many workers think for this host: the pool's, or none for an inline host.
    #[must_use]
    pub fn workers(&self) -> usize {
        self.pool.as_ref().map_or(0, |pool| pool.workers())
    }

    /// What the host has done so far.
    #[must_use]
    pub fn stats(&self) -> ThinkStats {
        ThinkStats {
            thinks: self.thinks.load(Ordering::Relaxed),
            batches: self.batches.load(Ordering::Relaxed),
            held: self.held.load(Ordering::Relaxed),
            runners_built: self.runners_built.load(Ordering::Relaxed),
        }
    }

    /// **The runner for the source `digest` names**, built the first time this host thinks
    /// with it and kept (the file head). `None` when no source is held under the digest or
    /// it does not compile: that actor does not think.
    fn runner(
        &self,
        store: Option<&TacsimStore>,
        unit: ObjId,
        digest: [u8; 32],
    ) -> Option<Arc<dyn MindRunner>> {
        self.runner_from(unit, digest, &|wanted| {
            crate::minds::content::source_lookup(store, wanted).map(str::to_owned)
        })
    }

    /// [`ThinkHost::runner`] over the source `sources` answers for `digest`: the store's for
    /// a doctrine, the text its claim retains for a mind sealed aboard.
    fn runner_from(
        &self,
        unit: ObjId,
        digest: [u8; 32],
        sources: &dyn Fn(&[u8; 32]) -> Option<String>,
    ) -> Option<Arc<dyn MindRunner>> {
        let mut runners = self.runners.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(runner) = runners.get(&digest) {
            return Some(Arc::clone(runner));
        }
        let built =
            crate::minds::sealed::scripted_runner(unit, &digest, &ArenaConfig::default(), sources);
        match built {
            Ok(runner) => {
                self.runners_built.fetch_add(1, Ordering::Relaxed);
                runners.insert(digest, Arc::clone(&runner));
                Some(runner)
            }
            Err(error) => {
                tracing::warn!(unit = unit.0, %error, "tacsim: a space mind has no runner");
                None
            }
        }
    }

    /// Put `runner` in place of the one `digest` would build: a test's way to make a
    /// worker slow, or to count its calls.
    #[cfg(test)]
    pub(crate) fn install_runner(&self, digest: [u8; 32], runner: Arc<dyn MindRunner>) {
        self.runners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(digest, runner);
    }

    /// Whether `owner`'s batch under `mark` has answered: a test's way to wait for the
    /// workers without settling.
    #[cfg(test)]
    pub(crate) fn is_answered(&self, owner: Owner, mark: Mark) -> bool {
        self.flights
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&owner)
            .filter(|flight| flight.mark == mark)
            .is_some_and(|flight| flight.flying.is_done())
    }

    /// The runner kept for `digest`, if this host has thought with it.
    #[cfg(test)]
    pub(crate) fn kept_runner(&self, digest: &[u8; 32]) -> Option<Arc<dyn MindRunner>> {
        self.runners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(digest)
            .map(Arc::clone)
    }

    /// **One think, frozen** (the file head): `unit` thinking with the source `digest`
    /// names, on `view`, with the memory its last think left. `None` when that source has
    /// no runner.
    #[must_use]
    pub fn job(
        &self,
        store: Option<&TacsimStore>,
        unit: ObjId,
        digest: [u8; 32],
        think_seq: u64,
        view: &Arc<MindView>,
        memory: &[u8],
    ) -> Option<CollectedThink> {
        let kit = KIT.as_ref()?;
        let runner = self.runner(store, unit, digest)?;
        let t_ms = view.t_ms;
        Some(CollectedThink {
            epochs: [0, 0, 0],
            unit,
            think_seq,
            t_ms,
            lands_ms: t_ms.saturating_add(500),
            seed: u64::from(unit.0).wrapping_add(think_seq),
            dropped: false,
            profile: kit.profile,
            view: Arc::clone(view),
            memory: memory.to_vec(),
            map: Arc::clone(&kit.map),
            wrecks: Vec::new(),
            card: kit.card.clone(),
            program: kit.program.clone(),
            profile_token: ProfileToken {
                unit,
                generation: 0,
                key: kit.card.key(),
            },
            runner,
            substep_hash: None,
            ranges_before: BTreeMap::new(),
        })
    }

    /// **The mind sealed aboard `hull`, made ready to think** (doc 44 unit MW.13): built
    /// from `seal` and the `source` its claim retains the first time this host thinks with
    /// that seal, and kept while the hull's seal names the same sortie. `None` when the seal
    /// cannot think ([`SealedMind::of`]).
    #[must_use]
    pub fn sealed_mind(
        &self,
        hull: ObjId,
        seal: &crate::arena::mind::MindSeal,
        source: &str,
        classes: &ClassTable,
    ) -> Option<Arc<SealedMind>> {
        let mut sealed = self.sealed.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((sortie, mind)) = sealed.get(&hull)
            && *sortie == seal.sortie
            && mind.source_sha256 == seal.source_sha256
        {
            return Some(Arc::clone(mind));
        }
        let mind = Arc::new(SealedMind::of(seal, source, classes)?);
        sealed.insert(hull, (seal.sortie, Arc::clone(&mind)));
        Some(mind)
    }

    /// **One think on `source`, frozen** (doc 44 unit MW.13): a doctrine's as
    /// [`ThinkHost::job`] freezes it, or a sealed mind's on its own class row, card and rule
    /// program. `None` when the source has no runner.
    #[must_use]
    pub fn job_on(
        &self,
        store: Option<&TacsimStore>,
        unit: ObjId,
        source: &Source,
        think_seq: u64,
        view: &Arc<MindView>,
        memory: &[u8],
    ) -> Option<CollectedThink> {
        let mind = match source {
            Source::Doctrine(digest) => {
                return self.job(store, unit, *digest, think_seq, view, memory);
            }
            Source::Sealed(mind) => mind,
        };
        let kit = KIT.as_ref()?;
        let runner = self.runner_from(unit, mind.source_sha256, &|wanted| {
            (*wanted == mind.source_sha256).then(|| mind.source.clone())
        })?;
        let t_ms = view.t_ms;
        Some(CollectedThink {
            epochs: [0, 0, 0],
            unit,
            think_seq,
            t_ms,
            lands_ms: t_ms.saturating_add(500),
            seed: u64::from(unit.0).wrapping_add(think_seq),
            dropped: false,
            profile: mind.profile,
            view: Arc::clone(view),
            memory: memory.to_vec(),
            map: Arc::clone(&kit.map),
            wrecks: Vec::new(),
            card: mind.card.clone(),
            program: mind.program.clone(),
            profile_token: ProfileToken {
                unit,
                generation: 0,
                key: mind.card.key(),
            },
            runner,
            substep_hash: None,
            ranges_before: BTreeMap::new(),
        })
    }

    /// **A seat's jobs over its `sources`, first choice first** (doc 44 unit MW.13), and the
    /// mind sealed aboard whose think the first job is, if it is one's. A source with no
    /// runner freezes no job; the sealed mind is named only when its own job was frozen, so
    /// an answer is never read as the sealed mind's when a house default gave it.
    #[must_use]
    pub fn jobs_on(
        &self,
        store: Option<&TacsimStore>,
        unit: ObjId,
        sources: &[Source],
        think_seq: u64,
        view: &Arc<MindView>,
        memory: &[u8],
    ) -> (Vec<CollectedThink>, Option<Arc<SealedMind>>) {
        let mut sealed = None;
        let mut jobs = Vec::with_capacity(sources.len());
        for (index, source) in sources.iter().enumerate() {
            let Some(job) = self.job_on(store, unit, source, think_seq, view, memory) else {
                continue;
            };
            if index == 0
                && let Source::Sealed(mind) = source
            {
                sealed = Some(Arc::clone(mind));
            }
            jobs.push(job);
        }
        (jobs, sealed)
    }

    /// **Hand `asks` to the pool as `owner`'s one batch** under `mark`, in place of any it
    /// had. Answers whether a batch is now in flight: `false` when nothing was asked.
    pub fn submit(&self, owner: Owner, mark: Mark, asks: Vec<Ask>) -> bool {
        let mut jobs: Vec<Vec<CollectedThink>> = Vec::with_capacity(asks.len());
        let mut asked = Vec::with_capacity(asks.len());
        let mut thinks = 0_usize;
        for ask in asks {
            if ask.jobs.is_empty() {
                continue;
            }
            asked.push(Asked {
                who: ask.who,
                unit: ask.unit,
                think_seq: ask.think_seq,
                contacts: ask.contacts,
                sealed: ask.sealed,
                struck: false,
            });
            thinks = thinks.saturating_add(ask.jobs.len());
            jobs.push(ask.jobs);
        }
        let mut flights = self.flights.lock().unwrap_or_else(PoisonError::into_inner);
        if jobs.is_empty() {
            flights.remove(&owner);
            return false;
        }
        self.batches.fetch_add(1, Ordering::Relaxed);
        self.thinks
            .fetch_add(u64::try_from(thinks).unwrap_or(u64::MAX), Ordering::Relaxed);
        let flying = match &self.pool {
            Some(pool) => Flying::Pooled(jobs.into_iter().map(|ask| pool.submit(ask)).collect()),
            None => Flying::Answered(
                jobs.iter()
                    .map(|ask| crate::arena::mind::run_batch_inline(ask))
                    .collect(),
            ),
        };
        flights.insert(
            owner,
            Flight {
                mark,
                asked,
                flying,
                held: None,
                held_ticks: 0,
            },
        );
        true
    }

    /// Whether `owner` has a batch in flight under `mark`, answered or not.
    #[must_use]
    pub fn in_flight(&self, owner: Owner, mark: Mark) -> bool {
        self.flights
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&owner)
            .is_some_and(|flight| flight.mark == mark)
    }

    /// **`owner`'s held step, as it fell due**, if a step of its is held under `mark`: that
    /// step is due at every tick until it is taken, and is taken as this.
    #[must_use]
    pub fn held_step(&self, owner: Owner, mark: Mark) -> Option<Due> {
        self.flights
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&owner)
            .filter(|flight| flight.mark == mark)
            .and_then(|flight| flight.held)
    }

    /// **`owner`'s step `due` is due: what its thinks answer** (the file head). A batch
    /// under another mark is dropped. A host that holds late steps answers [`Owed::Held`]
    /// for a batch that is not done, and remembers the step as it first fell due; any
    /// other host waits for the workers.
    pub fn settle(&self, owner: Owner, mark: Mark, due: Due) -> Owed {
        let flight = {
            let mut flights = self.flights.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(flight) = flights.get_mut(&owner) else {
                return Owed::Nothing;
            };
            if flight.mark != mark {
                flights.remove(&owner);
                return Owed::Nothing;
            }
            let done = flight.flying.is_done();
            if !done && self.holds_late_steps() {
                flight.held.get_or_insert(due);
                flight.held_ticks = flight.held_ticks.saturating_add(1);
                self.held.fetch_add(1, Ordering::Relaxed);
                if flight.held_ticks == HELD_WARN_TICKS
                    || flight.held_ticks % HELD_WARN_AGAIN_TICKS == 0
                {
                    tracing::warn!(
                        ?owner,
                        ticks = flight.held_ticks,
                        thinks = flight.asked.len(),
                        "tacsim: a space step is held for thinks the pool has not answered"
                    );
                }
                return Owed::Held;
            }
            match flights.remove(&owner) {
                Some(flight) => flight,
                None => return Owed::Nothing,
            }
        };
        let due = flight.held.unwrap_or(due);
        // Past this point every ask is done, or this host is a rig's and waits for them.
        let answered: Vec<Vec<ThinkAnswer>> = match flight.flying {
            Flying::Pooled(handles) => handles
                .into_iter()
                .map(|handle| match handle.wait() {
                    Ok(answers) => answers,
                    Err(failed) => {
                        tracing::error!(
                            %failed,
                            ?owner,
                            "tacsim: a space mind's runner panicked; it does nothing this step"
                        );
                        Vec::new()
                    }
                })
                .collect(),
            Flying::Answered(answers) => answers,
        };
        let answers = flight
            .asked
            .into_iter()
            .zip(answered)
            .map(|(asked, sources)| {
                let mut result = None;
                let mut by_sealed = false;
                for (index, answer) in sources.into_iter().enumerate() {
                    if result.is_some() || asked.struck {
                        continue;
                    }
                    if let Some(fault) = &answer.result.fault {
                        tracing::warn!(
                            unit = asked.unit.0,
                            ?fault,
                            "tacsim: a space mind faulted; its next source is tried"
                        );
                    } else {
                        by_sealed = index == 0;
                        result = Some(answer.result);
                    }
                }
                Answer {
                    who: asked.who,
                    unit: asked.unit,
                    think_seq: asked.think_seq,
                    contacts: asked.contacts,
                    result,
                    sealed: asked.sealed.filter(|_| by_sealed),
                }
            })
            .collect();
        Owed::Ready { answers, due }
    }

    /// Drop `owner`'s batch, answered or not: its actor is no longer flown by a mind (a
    /// crew took the helm back), or is gone.
    pub fn discard(&self, owner: Owner) {
        self.flights
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&owner);
    }

    /// **Strike `unit`'s think for the seat `who` out of `owner`'s batch**, leaving the
    /// rest to land: that actor's answer is `None` when the batch answers. A crew that
    /// takes the helm back discards what the backup mind had out for her (Q-44-37).
    pub fn forget(&self, owner: Owner, who: Thinker, unit: ObjId) {
        if let Some(flight) = self
            .flights
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(&owner)
        {
            for asked in &mut flight.asked {
                if asked.who == who && asked.unit == unit {
                    asked.struck = true;
                }
            }
        }
    }

    /// Drop the batch of every hull adrift, and of every hull's drones, that `keep` does
    /// not answer for: a hull that is no longer the seat's to fly outside a bubble, or
    /// whose cluster session has ended, leaves nothing behind.
    pub fn keep_hulls(&self, keep: impl Fn(Owner) -> bool) {
        self.flights
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|owner, _| match owner {
                Owner::Bubble(_) => true,
                Owner::Adrift(_) | Owner::Cluster(_) => keep(*owner),
            });
    }

    /// Drop the batch of every bubble that is not in `live`.
    pub fn keep_bubbles(&self, live: &BTreeSet<DomainId>) {
        self.flights
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|owner, _| match owner {
                Owner::Bubble(domain) => live.contains(domain),
                Owner::Adrift(_) | Owner::Cluster(_) => true,
            });
    }
}

/// **Where a store keeps its host** ([`TacsimStore::space_thinks`]): made on the first
/// think, on the store's own pool, and shared by every clone of the store.
#[derive(Clone, Debug, Default)]
pub struct HostSlot(Arc<std::sync::OnceLock<Arc<ThinkHost>>>);

impl HostSlot {
    /// The host, made on `pool` the first time it is asked for.
    pub(crate) fn get(&self, pool: impl FnOnce() -> Arc<MindPool>) -> Arc<ThinkHost> {
        Arc::clone(self.0.get_or_init(|| Arc::new(ThinkHost::on_pool(pool()))))
    }
}

/// **The host of `w`'s space thinks**, if the world has a TacSim store.
#[must_use]
pub fn host_of(w: &mush_core::World) -> Option<Arc<ThinkHost>> {
    w.module_state::<TacsimStore>()
        .map(TacsimStore::space_thinks)
}

/// **Tell `w`'s space minds that the world runs on the wall clock** (the file head): the
/// server's world loop calls this once before its first tick. From then on a think that
/// has not answered when its owner's next step is due holds that step, and the world
/// thread never waits for a worker. A world nobody calls this for is a rig's.
pub fn run_on_the_wall_clock(w: &mush_core::World) {
    if let Some(host) = host_of(w) {
        host.hold_late_steps();
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Runners a test puts in a host's way, and a mind sealed aboard a ship.

    use smol_str::SmolStr;

    /// **A draft sealed over an ungoverned Class II Dray**, through the one seal entry
    /// (`seal_with_policy`), with `rules` as its opening profile's rule program and `source`
    /// as its script: the seal a claim's capture takes, less the claim. A draft seals only on
    /// an ungoverned core (D-UM-41), so the hull is the Dray's ungoverned twin; a filed mind
    /// on a governed core is the stories' (`tests/domain_field/mw13_ship_mind.rs`).
    pub(crate) fn sealed_draft(
        hull: mush_core::ObjId,
        rules: Vec<crate::arena::schema::AuthoredRule>,
        source: &str,
    ) -> crate::arena::mind::MindSeal {
        use sha2::{Digest as _, Sha256};

        use crate::arena::core::{ClassTable, CoreClass};
        use crate::arena::mind::{SealEngine, SortieId};
        use crate::arena::profile_seal::{MindIdentity, MindOrigin};
        use crate::arena::runtime_card::stock_definition;
        use crate::config::ReflexConfig;
        use crate::definitions::Catalog;
        use crate::definitions::core_replacement::{SHIP_II_UNGOVERNED, fitted_catalog};
        use crate::deliberation::order::CharacterRef;
        use crate::match_::seal::{SealPolicy, seal_with_policy};

        let fielded = Catalog::build_fielded(&[], crate::NS).expect("the fielded content");
        let fitted = fitted_catalog(fielded).expect("the fitted families");
        let spec = fitted
            .units
            .get(SHIP_II_UNGOVERNED)
            .expect("the cored Dray");
        let unit = mush_tactics::TacticalUnit::from_spec(spec, hull).expect("it instantiates");
        let reflex = ReflexConfig::default();
        let card = crate::arena::card::line(&reflex);
        let mut definition = stock_definition(
            &[(SmolStr::new_static("Line"), card.clone())],
            CoreClass::II,
        )
        .expect("the stock definition");
        definition.profiles[0].rules = rules;
        let source_sha256: [u8; 32] = Sha256::digest(source.as_bytes()).into();
        let identity = MindIdentity {
            mind: SmolStr::new_static("Keeper"),
            origin: MindOrigin::Draft {
                character: mush_core::ObjId(77),
                mind: 1,
                revision: 1,
                digest: mush_core::DefinitionDigest::from_bytes(source_sha256),
            },
            source_sha256,
        };
        let principal = Some(CharacterRef::Character {
            id: mush_core::ObjId(77),
        });
        let fallback = None;
        seal_with_policy(
            hull,
            &unit,
            &[],
            SortieId::ic(hull, 100),
            &SealPolicy {
                authored: Some(&definition),
                selected_profiles: &[],
                classes: &ClassTable::STOCK,
                rom_cfg: &reflex,
                principal: &principal,
                card: &card,
                odds_floor_permille: 100,
                engine_for_minds: SealEngine::Scripted,
                rated: CoreClass::II,
                class_cap: CoreClass::III,
                identity: &identity,
                profiles: &[],
                fallback: &fallback,
                impounded: false,
            },
        )
        .expect("the draft seals on the ungoverned core")
    }

    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Condvar, Mutex, PoisonError};

    use crate::arena::mind::{MindRunner, ThinkJob, ThinkResult};

    /// A gate a test opens: a [`Gated`] runner parks on it.
    #[derive(Debug, Default)]
    pub(crate) struct Gate {
        open: Mutex<bool>,
        opened: Condvar,
        /// Thinks that have reached the gate.
        arrived: AtomicU64,
    }

    impl Gate {
        /// Let every parked think through, and every later one.
        pub(crate) fn open(&self) {
            *self.open.lock().unwrap_or_else(PoisonError::into_inner) = true;
            self.opened.notify_all();
        }

        /// Park every think from now on.
        pub(crate) fn close(&self) {
            *self.open.lock().unwrap_or_else(PoisonError::into_inner) = false;
        }

        /// How many thinks have reached the gate.
        pub(crate) fn arrived(&self) -> u64 {
            self.arrived.load(Ordering::SeqCst)
        }

        fn pass(&self) {
            self.arrived.fetch_add(1, Ordering::SeqCst);
            let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
            while !*open {
                open = self
                    .opened
                    .wait(open)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        }
    }

    /// The runner `inner`, behind a gate: a worker that is as slow as the test says.
    #[derive(Debug)]
    pub(crate) struct Gated {
        pub(crate) inner: Arc<dyn MindRunner>,
        pub(crate) gate: Arc<Gate>,
    }

    impl MindRunner for Gated {
        fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
            self.gate.pass();
            self.inner.think(job)
        }
    }

    /// The runner `inner`, which also records the name of each thread it thinks on.
    #[derive(Debug)]
    pub(crate) struct Traced {
        pub(crate) inner: Arc<dyn MindRunner>,
        pub(crate) threads: Mutex<Vec<String>>,
    }

    impl MindRunner for Traced {
        fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
            if let Ok(mut threads) = self.threads.lock() {
                threads.push(
                    std::thread::current()
                        .name()
                        .unwrap_or("unnamed")
                        .to_owned(),
                );
            }
            self.inner.think(job)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use super::test_support::{Gate, Gated};
    use super::*;
    use crate::arena::mind::MindRunner;
    use crate::minds::content::drone_mining_digest;

    /// A drone's view in its bay, with the clock a think `seq` reads.
    fn view(seq: u64) -> Arc<MindView> {
        let mut view = crate::space::sortie::rig::view(ObjId(77));
        view.t_ms = i64::try_from(seq.saturating_mul(1_000)).unwrap_or(0);
        Arc::new(view)
    }

    fn drone_ask(host: &ThinkHost, seq: u64, memory: &[u8]) -> Ask {
        let job = host
            .job(
                None,
                ObjId(77),
                drone_mining_digest().0,
                seq,
                &view(seq),
                memory,
            )
            .expect("the drone's doctrine has a runner");
        Ask {
            who: Thinker::Drone,
            unit: ObjId(77),
            think_seq: seq,
            contacts: BTreeMap::new(),
            jobs: vec![job],
            sealed: None,
        }
    }

    /// Put the drone doctrine's own runner behind a gate on `host`.
    fn gate(host: &ThinkHost) -> Arc<Gate> {
        let digest = drone_mining_digest().0;
        let _ = host.job(None, ObjId(77), digest, 0, &view(0), &[]);
        let inner = host.kept_runner(&digest).expect("the runner is kept");
        let gate = Arc::new(Gate::default());
        let gated: Arc<dyn MindRunner> = Arc::new(Gated {
            inner,
            gate: Arc::clone(&gate),
        });
        host.install_runner(digest, gated);
        gate
    }

    /// Wait until `gate` has seen `n` thinks arrive, so a worker is known to be parked.
    fn parked(gate: &Gate, n: u64) {
        let until = Instant::now() + Duration::from_secs(20);
        while gate.arrived() < n {
            assert!(Instant::now() < until, "a worker never reached the gate");
            std::thread::yield_now();
        }
    }

    /// **Doc 44 unit MW.11: a runner is kept for each seal and not rebuilt each think.**
    /// A hundred thinks of one digest build one runner, and every job carries that one.
    #[test]
    fn mw11_a_runner_is_built_once_for_a_digest_and_kept() {
        let host = ThinkHost::inline();
        let digest = drone_mining_digest().0;
        let mut memory = Vec::new();
        let mut first: Option<Arc<dyn MindRunner>> = None;
        for seq in 0..100_u64 {
            let ask = drone_ask(&host, seq, &memory);
            let runner = Arc::clone(&ask.jobs.first().expect("one job").runner);
            if let Some(first) = &first {
                assert!(
                    Arc::ptr_eq(first, &runner),
                    "think {seq} has the kept runner"
                );
            } else {
                first = Some(runner);
            }
            assert!(host.submit(Owner::Cluster(ObjId(1)), (0, 0), vec![ask]));
            let Owed::Ready { mut answers, .. } =
                host.settle(Owner::Cluster(ObjId(1)), (0, 0), Due::default())
            else {
                panic!("an inline host has answered");
            };
            let result = answers
                .pop()
                .and_then(|answer| answer.result)
                .expect("a clean think");
            memory = result.memory;
        }
        assert_eq!(host.stats().runners_built, 1, "one runner for one digest");
        assert_eq!(host.stats().thinks, 100);
        // Another hull sealed with the same source thinks on the same runner.
        let other = host
            .job(None, ObjId(78), digest, 0, &view(0), &[])
            .expect("a job");
        assert!(Arc::ptr_eq(
            &other.runner,
            first.as_ref().expect("the first")
        ));
        assert_eq!(host.stats().runners_built, 1);
    }

    /// **Doc 44 unit MW.11: a host that holds late steps never waits for a worker**, and a
    /// held step keeps the length it was first due with. With the worker parked, `settle`
    /// answers `Held` at once, however often it is asked; once the worker is through, the
    /// same batch answers, with the first length.
    #[test]
    fn mw11_a_holding_host_does_not_wait_and_a_held_step_keeps_its_length() {
        let host = ThinkHost::on_pool(MindPool::new(1));
        host.hold_late_steps();
        let gate = gate(&host);
        let owner = Owner::Cluster(ObjId(1));
        assert!(host.submit(owner, (0, 0), vec![drone_ask(&host, 0, &[])]));
        parked(&gate, 1);
        let started = Instant::now();
        let first = Due {
            dt_wall_ms: 1_000,
            ic_now_s: 500,
        };
        for _ in 0..1_000 {
            assert!(matches!(host.settle(owner, (0, 0), first), Owed::Held));
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "a thousand settles on a parked worker return at once"
        );
        assert_eq!(host.held_step(owner, (0, 0)), Some(first));
        assert_eq!(host.stats().held, 1_000);
        gate.open();
        // The batch answers when the worker is through; the step is then taken as it first
        // fell due, whatever is due by then.
        let later = Due {
            dt_wall_ms: 7_000,
            ic_now_s: 900,
        };
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            match host.settle(owner, (0, 0), later) {
                Owed::Held => {
                    assert!(Instant::now() < until, "the batch never answered");
                    std::thread::yield_now();
                }
                Owed::Ready { answers, due } => {
                    assert_eq!(due, first, "the step as it first fell due");
                    assert_eq!(answers.len(), 1);
                    assert!(
                        answers
                            .first()
                            .is_some_and(|answer| answer.result.is_some())
                    );
                    break;
                }
                Owed::Nothing => panic!("the batch was in flight"),
            }
        }
        assert!(!host.in_flight(owner, (0, 0)));
    }

    /// **Doc 44 unit MW.11: a rig's host waits for its workers**, so a story that moves the
    /// clock by hand never sees a held step; and a batch under another mark is dropped
    /// unread.
    #[test]
    fn mw11_a_rigs_host_waits_and_a_batch_under_another_mark_is_dropped() {
        let host = ThinkHost::on_pool(MindPool::new(2));
        assert!(!host.holds_late_steps());
        let owner = Owner::Bubble(DomainId(7));
        assert!(host.submit(owner, (100, 1), vec![drone_ask(&host, 0, &[])]));
        let due = Due {
            dt_wall_ms: 1_000,
            ic_now_s: 0,
        };
        assert!(matches!(
            host.settle(owner, (100, 1), due),
            Owed::Ready { due: taken, .. } if taken == due
        ));
        assert_eq!(host.stats().held, 0);
        // Another bubble opens on the same domain: the old batch is not its.
        assert!(host.submit(owner, (100, 1), vec![drone_ask(&host, 0, &[])]));
        assert!(matches!(host.settle(owner, (200, 1), due), Owed::Nothing));
        assert!(!host.in_flight(owner, (100, 1)));
        // A bubble that is gone leaves nothing behind.
        assert!(host.submit(owner, (200, 1), vec![drone_ask(&host, 0, &[])]));
        host.keep_bubbles(&BTreeSet::new());
        assert!(!host.in_flight(owner, (200, 1)));
    }

    /// **Doc 44 unit MW.11: the backup seat's sources are one ask.** The first that answers
    /// without a fault is the answer, and a source that faults is passed over.
    #[test]
    fn mw11_the_first_source_that_answers_clean_is_the_answer() {
        #[derive(Debug)]
        struct Faulting;
        impl MindRunner for Faulting {
            fn think(&self, job: &crate::arena::mind::ThinkJob<'_>) -> ThinkResult {
                ThinkResult {
                    diagnostics: None,
                    rule_sample: None,
                    intents: Vec::new(),
                    memory: job.memory.to_vec(),
                    fault: Some(crate::arena::mind::ThinkFault {
                        cause: SmolStr::new_static("ops"),
                    }),
                    trace: Vec::new(),
                }
            }
        }
        let host = ThinkHost::on_pool(MindPool::new(2));
        let clean = drone_ask(&host, 0, &[]);
        let mut faulting = clean.jobs.first().expect("a job").clone();
        faulting.runner = Arc::new(Faulting);
        let ask = Ask {
            who: Thinker::Backup,
            unit: ObjId(77),
            think_seq: 0,
            contacts: BTreeMap::new(),
            jobs: vec![faulting.clone(), clean.jobs.first().expect("a job").clone()],
            sealed: None,
        };
        let owner = Owner::Adrift(ObjId(77));
        assert!(host.submit(owner, (0, 0), vec![ask]));
        let Owed::Ready { answers, .. } = host.settle(owner, (0, 0), Due::default()) else {
            panic!("a rig's host waits");
        };
        assert_eq!(answers.len(), 1, "one answer for one ask");
        let result = answers
            .first()
            .and_then(|answer| answer.result.as_ref())
            .expect("the second source answered");
        assert!(result.fault.is_none());
        // Every source faulting leaves the actor with no answer.
        let ask = Ask {
            who: Thinker::Backup,
            unit: ObjId(77),
            think_seq: 0,
            contacts: BTreeMap::new(),
            jobs: vec![faulting.clone(), faulting],
            sealed: None,
        };
        assert!(host.submit(owner, (0, 0), vec![ask]));
        let Owed::Ready { answers, .. } = host.settle(owner, (0, 0), Due::default()) else {
            panic!("a rig's host waits");
        };
        assert!(
            answers
                .first()
                .is_some_and(|answer| answer.result.is_none())
        );
    }
}

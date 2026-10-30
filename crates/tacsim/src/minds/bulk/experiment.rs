//! Private candidate execution. Inputs are immutable owner-resolved holdings
//! and the selected hull's frozen facts; no live World or source lookup enters.
use super::{BulkError, invalid, scenarios::Candidate};
use crate::arena::{
    Arena, Controller, MindSeal,
    core::CoreProfile,
    mind::{MindRunner, ThinkJob, ThinkResult},
    profile_runtime::ProfileRuntime,
    profile_seal::{MindIdentity, SealedCards},
    runtime::prepare_profile_runtime,
    runtime_card::CardCatalog,
};
use mush_core::ObjId;
use mush_mind::{Digest, EngineLimits, Sandbox};
use serde::Serialize;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};

pub(in crate::minds) enum Edit {
    Diff(Candidate),
    DiffOff,
    Swap { candidate: Candidate, serial: u64 },
    Possess { serial: u64 },
    Release,
    Do(String),
    Reference,
}
#[derive(Debug)]
pub(super) struct BoundCandidate {
    pub seal: MindSeal,
    pub runtime: ProfileRuntime,
    pub catalog: CardCatalog,
    pub profile: CoreProfile,
    pub runner: Arc<dyn MindRunner>,
}
impl BoundCandidate {
    pub fn prepare(
        arena: &Arena,
        original: &MindSeal,
        candidate: &Candidate,
    ) -> Result<Self, BulkError> {
        let own = arena.unit(original.unit).ok_or(BulkError::Missing)?;
        let class = candidate.held.rated;
        let core = arena
            .core_of(own.id)
            .ok_or_else(|| invalid("selected hull has no active core"))?;
        if class > original.class
            || class > core.class
            || class == crate::arena::core::CoreClass::Zero
        {
            return Err(invalid(
                "candidate class exceeds this frozen sortie's execution allowance or current core",
            ));
        }
        let definition = if let Some(definition) = &candidate.held.definition {
            (**definition).clone()
        } else {
            let card = crate::arena::card::preset(&candidate.profile, &arena.arena_cfg.reflex)
                .ok_or(BulkError::Missing)?;
            crate::arena::runtime_card::stock_definition(
                &[(candidate.profile.clone(), card)],
                class,
            )
            .map_err(|_| invalid("candidate card definition"))?
        };
        let profile = arena.classes.row(class);
        let cards = SealedCards::prepare(
            definition,
            std::slice::from_ref(&candidate.profile),
            &profile,
            &original.weapons,
            &original.channels,
            original.odds_floor_permille,
        )
        .map_err(|_| invalid("candidate card or rules do not fit this hull"))?;
        let mut catalog = arena.cards.clone();
        let mut runtime = prepare_profile_runtime(
            &mut catalog,
            own.id,
            &cards,
            &original.rom,
            &arena.classes,
            class,
            &original.weapons,
            &original.channels,
            original.odds_floor_permille,
        )
        .map_err(|_| invalid("candidate runtime does not fit this hull"))?;
        runtime.generation = arena
            .profiles
            .get(&own.id)
            .ok_or(BulkError::Missing)?
            .generation
            .checked_add(1)
            .ok_or_else(|| invalid("candidate profile generation exhausted"))?;
        let identity = &candidate.held.identity;
        let mut seal = original.clone();
        seal.class = class;
        seal.mind = identity.mind.clone();
        seal.origin = identity.origin.clone();
        seal.source_sha256 = identity.source_sha256;
        seal.requested = MindIdentity {
            mind: identity.mind.clone(),
            origin: identity.origin.clone(),
            source_sha256: identity.source_sha256,
        };
        seal.card = cards.opening_card.clone();
        seal.card_digest = cards.opening_digest;
        seal.cards = cards;
        seal.engine = candidate.held.engine;
        seal.fallback = None;
        seal.verify(&arena.classes, &original.weapons)
            .map_err(|_| invalid("candidate execution seal"))?;
        let runner: Arc<dyn MindRunner> = match candidate.held.engine {
            crate::arena::mind::SealEngine::Native => Arc::new(
                crate::arena::stock_mind::NativeStockMind::with(&arena.arena_cfg.reflex),
            ),
            crate::arena::mind::SealEngine::Scripted => {
                let source = candidate
                    .held
                    .source
                    .as_deref()
                    .unwrap_or(crate::minds::content::STANDARD_RHAI);
                let sandbox = Arc::new(Sandbox::new(
                    &EngineLimits::default(),
                    crate::minds::register_api,
                ));
                Arc::new(
                    crate::minds::RhaiRunner::on(
                        sandbox,
                        &arena.arena_cfg,
                        source,
                        Digest(identity.source_sha256),
                    )
                    .map_err(|_| invalid("candidate source does not compile on this engine"))?,
                )
            }
            _ => return Err(invalid("candidate engine is not supported")),
        };
        Ok(Self {
            seal,
            runtime,
            catalog,
            profile,
            runner,
        })
    }
    /// Runs only on a disposable already-cloned fork. All fallible binding and
    /// generation checks precede normal epoch cancellation and source install.
    pub fn install(self, arena: &mut Arena) -> Result<MindSeal, BulkError> {
        let unit = self.seal.unit;
        let epochs = self
            .seal
            .channels
            .iter()
            .map(|channel| {
                arena
                    .epochs
                    .current(unit, *channel)
                    .checked_add(1)
                    .map(|epoch| (*channel, epoch))
                    .ok_or_else(|| invalid("station epoch exhausted"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        crate::arena::conditions::ConditionTable::prepare_profile_rules(
            unit,
            &self.runtime.program.native_rows(),
            self.profile.slots,
        )
        .map_err(|_| invalid("candidate rule installation"))?;
        let own = arena.unit_mut(unit).ok_or(BulkError::Missing)?;
        let was = own.controller;
        // This checked hull borrow is the first mutation. Cancellation and
        // Live control handoff below use the old runtime rows, not the seal.
        own.seal = self.seal.seal_ref();
        arena.cancel_profile_think(unit);
        // Normal control changes purge queued intents from the former epoch.
        for (channel, epoch) in epochs {
            crate::arena::intents::apply_control(
                arena,
                &crate::arena::intents::Control {
                    unit,
                    channel,
                    epoch,
                    state: crate::arena::intents::ControlState::Live,
                    pilot: None,
                },
            );
        }
        arena.cards = self.catalog;
        arena.profiles.insert(unit, self.runtime);
        let mut mind = crate::arena::mind::MindState::fresh(&arena.arena_cfg.reflex);
        mind.seal_digest = self.seal.digest();
        arena.minds.insert(unit, mind);
        arena.runners.insert(unit, self.runner);
        if was == Controller::Mind {
            arena.set_controller(
                unit,
                Controller::Mind,
                crate::arena::doctrine::MachineState::Startup,
            );
        }
        // The controller handback installed the captured program constrained
        // to surviving hardware. Reapplying full rows here would resurrect
        // rules for destroyed weapons and disagree with the override bits.
        if was != Controller::Mind {
            arena.conditions.clear(unit);
        }
        Ok(self.seal)
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct DiffRow {
    pub think_seq: u64,
    pub at_ms: i64,
    pub reference_intents: usize,
    pub candidate_intents: usize,
    pub candidate_fault: bool,
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct DiffPicture {
    pub mind: String,
    pub from_ms: i64,
    pub thinks: u64,
    pub differences: u64,
    pub omitted: u64,
    pub rows: Vec<DiffRow>,
}
#[derive(Debug)]
struct DiffState {
    unit: ObjId,
    candidate: BoundCandidate,
    memory: Vec<u8>,
    picture: DiffPicture,
    rows: VecDeque<DiffRow>,
}
#[derive(Debug)]
struct Compared {
    reference: Arc<dyn MindRunner>,
    state: Arc<Mutex<DiffState>>,
}
impl MindRunner for Compared {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        let reference = self.reference.think(job);
        let Ok(mut state) = self.state.lock() else {
            return reference;
        };
        if job.unit != state.unit {
            return reference;
        }
        // The exact captured public view/map/time/seed passed to the real
        // reference runner also reaches the candidate. It has its own immutable
        // card/program, budget and fresh independent memory, never reference RAM.
        let candidate = state.candidate.runner.think(&ThinkJob {
            profile: &state.candidate.profile,
            memory: &state.memory,
            dropped: false,
            card: &state.candidate.runtime.card,
            program: &state.candidate.runtime.program,
            profile_token: state.candidate.runtime.token(),
            ..*job
        });
        state.picture.thinks = state.picture.thinks.saturating_add(1);
        let differs = reference.intents != candidate.intents
            || reference.fault.as_ref().map(|f| &f.cause)
                != candidate.fault.as_ref().map(|f| &f.cause);
        if differs {
            state.picture.differences = state.picture.differences.saturating_add(1);
            if state.rows.len() == 64 {
                state.rows.pop_front();
                state.picture.omitted = state.picture.omitted.saturating_add(1);
            }
            state.rows.push_back(DiffRow {
                think_seq: job.think_seq,
                at_ms: job.t_ms,
                reference_intents: reference.intents.len(),
                candidate_intents: candidate.intents.len(),
                candidate_fault: candidate.fault.is_some(),
            });
        }
        state.memory = candidate.memory;
        reference
    }
}
type RunnerPair = (Arc<dyn MindRunner>, Arc<dyn MindRunner>);

#[derive(Debug)]
pub(super) struct Diff {
    state: Arc<Mutex<DiffState>>,
    installed: BTreeMap<Option<ObjId>, RunnerPair>,
}
impl Diff {
    pub fn new(candidate: BoundCandidate, at_ms: i64) -> Self {
        let picture = DiffPicture {
            mind: candidate.seal.mind.to_string(),
            from_ms: at_ms,
            thinks: 0,
            differences: 0,
            omitted: 0,
            rows: Vec::new(),
        };
        Self {
            state: Arc::new(Mutex::new(DiffState {
                unit: candidate.seal.unit,
                candidate,
                memory: Vec::new(),
                picture,
                rows: VecDeque::new(),
            })),
            installed: BTreeMap::new(),
        }
    }
    pub fn install(&mut self, arena: &mut Arena) {
        self.wrap(None, &mut arena.mind_runner);
        for (unit, runner) in &mut arena.runners {
            self.wrap(Some(*unit), runner);
        }
    }
    fn wrap(&mut self, key: Option<ObjId>, runner: &mut Arc<dyn MindRunner>) {
        if self
            .installed
            .get(&key)
            .is_some_and(|old| Arc::ptr_eq(&old.1, runner))
        {
            return;
        }
        let original = Arc::clone(runner);
        let wrapped: Arc<dyn MindRunner> = Arc::new(Compared {
            reference: Arc::clone(runner),
            state: Arc::clone(&self.state),
        });
        *runner = Arc::clone(&wrapped);
        self.installed.insert(key, (original, wrapped));
    }
    pub fn uninstall(&mut self, arena: &mut Arena) {
        if let Some((original, wrapped)) = self.installed.get(&None)
            && Arc::ptr_eq(wrapped, &arena.mind_runner)
        {
            arena.mind_runner = Arc::clone(original);
        }
        for (unit, runner) in &mut arena.runners {
            if let Some((original, wrapped)) = self.installed.get(&Some(*unit))
                && Arc::ptr_eq(wrapped, runner)
            {
                *runner = Arc::clone(original);
            }
        }
        self.installed.clear();
    }
    pub fn reset(&mut self, at_ms: i64) -> Result<(), BulkError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| invalid("candidate comparison unavailable"))?;
        state.memory.clear();
        state.rows.clear();
        state.picture.from_ms = at_ms;
        state.picture.thinks = 0;
        state.picture.differences = 0;
        state.picture.omitted = 0;
        self.installed.clear();
        Ok(())
    }
    pub fn picture(&self) -> Result<DiffPicture, BulkError> {
        let state = self
            .state
            .lock()
            .map_err(|_| invalid("candidate comparison unavailable"))?;
        let mut picture = state.picture.clone();
        picture.rows = state.rows.iter().cloned().collect();
        Ok(picture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::{
        PartyId,
        intents::IntentKind,
        test_support::{arena_of, pose, tank},
    };
    type ObservedJob = (usize, i64, i64, u64, Vec<u8>);
    #[derive(Debug)]
    struct Counting {
        speed: i32,
        seen: Arc<Mutex<Vec<ObservedJob>>>,
    }
    impl MindRunner for Counting {
        fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
            self.seen.lock().unwrap().push((
                Arc::as_ptr(job.view) as usize,
                job.t_ms,
                job.lands_ms,
                job.seed,
                job.memory.to_vec(),
            ));
            let n = job.memory.first().copied().unwrap_or(0).saturating_add(1);
            ThinkResult {
                intents: vec![IntentKind::Speed { cms: self.speed }],
                memory: vec![n],
                fault: None,
                trace: Vec::new(),
                rule_sample: None,
                diagnostics: None,
            }
        }
    }
    fn fixture() -> (Arena, MindSeal) {
        let mut unit = tank(ObjId(4), PartyId::RED, pose(1500, 4500));
        unit.controller = Controller::Mind;
        unit.reseal(&crate::match_::SealTerms::fixture()).unwrap();
        let seal = unit.seal.clone();
        (arena_of(vec![unit]), seal)
    }
    #[test]
    fn comparison_keeps_exact_pictures_independent_memory_and_reference_output() {
        let (mut arena, seal) = fixture();
        let reference_seen = Arc::new(Mutex::new(Vec::new()));
        let candidate_seen = Arc::new(Mutex::new(Vec::new()));
        let reference: Arc<dyn MindRunner> = Arc::new(Counting {
            speed: 1,
            seen: reference_seen.clone(),
        });
        arena.mind_runner = reference.clone();
        arena.runners.insert(seal.unit, reference.clone());
        let candidate = BoundCandidate {
            runtime: arena.profiles[&seal.unit].clone(),
            catalog: arena.cards.clone(),
            profile: arena.classes.row(seal.class),
            seal: seal.clone(),
            runner: Arc::new(Counting {
                speed: 2,
                seen: candidate_seen.clone(),
            }),
        };
        let mut diff = Diff::new(candidate, 0);
        diff.install(&mut arena);
        diff.install(&mut arena); // Installing around a phase barrier must not double-run it.
        let view = Arc::new(arena.mind_view(seal.unit).unwrap());
        let before = crate::arena::state_hash(&arena);
        for n in 0..70 {
            let runtime = &arena.profiles[&seal.unit];
            let result = arena.runners[&seal.unit].think(&ThinkJob {
                unit: seal.unit,
                think_seq: n,
                t_ms: n as i64 * 250,
                lands_ms: (n as i64 + 1) * 250,
                seed: arena.seed,
                dropped: false,
                profile: &arena.classes.row(seal.class),
                view: &view,
                memory: &[99],
                map: &arena.definition,
                card: &runtime.card,
                program: &runtime.program,
                profile_token: runtime.token(),
            });
            assert_eq!(result.memory, vec![100]);
            assert_eq!(result.intents, vec![IntentKind::Speed { cms: 1 }]);
        }
        assert_eq!(crate::arena::state_hash(&arena), before);
        let observed = candidate_seen.lock().unwrap();
        let references = reference_seen.lock().unwrap();
        assert_eq!(observed.len(), 70);
        for (n, (candidate, reference)) in observed.iter().zip(references.iter()).enumerate() {
            assert_eq!(
                (candidate.0, candidate.1, candidate.2, candidate.3),
                (reference.0, reference.1, reference.2, reference.3)
            );
            assert_eq!(candidate.4, if n == 0 { vec![] } else { vec![n as u8] });
            assert_eq!(reference.4, vec![99]);
        }
        drop(observed);
        drop(references);
        let p = diff.picture().unwrap();
        assert_eq!(
            (p.thinks, p.differences, p.omitted, p.rows.len()),
            (70, 70, 6, 64)
        );
        diff.uninstall(&mut arena);
        assert!(Arc::ptr_eq(&arena.mind_runner, &reference));
        assert!(Arc::ptr_eq(&arena.runners[&seal.unit], &reference));
        diff.reset(0).unwrap();
        assert_eq!(diff.picture().unwrap().thinks, 0);
        assert!(diff.state.lock().unwrap().memory.is_empty());
    }
    #[test]
    fn swap_preserves_destroyed_weapon_absence_and_disabled_profile_slot() {
        let mut unit = tank(ObjId(4), PartyId::RED, pose(1500, 4500));
        unit.weapons.push(crate::arena::test_support::cannon());
        unit.controller = Controller::Mind;
        unit.reseal(&crate::match_::SealTerms::fixture()).unwrap();
        let seal = unit.seal.clone();
        let mut arena = arena_of(vec![unit]);
        let mut candidate = Candidate::resolve(
            &mush_roll::RollStore::default(),
            &crate::store::TacsimStore::default(),
            ObjId(99),
            "Standard/Line",
        )
        .unwrap();
        let card = crate::arena::card::preset("Line", &arena.arena_cfg.reflex).unwrap();
        let mut definition =
            crate::arena::runtime_card::stock_definition(&[("Line".into(), card)], seal.class)
                .unwrap();
        definition.profiles[0]
            .rules
            .push(crate::arena::schema::AuthoredRule {
                rule: crate::arena::conditions::Rule::hold_rule("cannon".into(), vec![]),
                think: vec![],
            });
        candidate.held.definition = Some(Arc::new(definition));
        arena.unit_mut(seal.unit).unwrap().weapons.clear();
        let bound = BoundCandidate::prepare(&arena, &seal, &candidate).unwrap();
        assert!(bound.runtime.program.native_rows()[0].is_some());
        bound.install(&mut arena).unwrap();
        assert!(arena.unit(seal.unit).unwrap().weapon("cannon").is_none());
        assert!(
            arena
                .conditions
                .slots(seal.unit)
                .iter()
                .all(Option::is_none)
        );
        assert_ne!(arena.profiles[&seal.unit].overridden & 1, 0);
        assert!(arena.profiles[&seal.unit].program.native_rows()[0].is_some());
    }
    #[test]
    fn frozen_candidate_binding_does_not_mutate_the_reference() {
        let (arena, seal) = fixture();
        let before = crate::arena::state_hash(&arena);
        let candidate = Candidate::resolve(
            &mush_roll::RollStore::default(),
            &crate::store::TacsimStore::default(),
            ObjId(99),
            "Standard/Line",
        )
        .unwrap();
        let bound = BoundCandidate::prepare(&arena, &seal, &candidate).unwrap();
        assert_eq!(bound.seal.unit, seal.unit);
        assert_eq!(crate::arena::state_hash(&arena), before);
        assert!(bound.runtime.generation > arena.profiles[&seal.unit].generation);
        let mut too_small = seal.clone();
        too_small.class = crate::arena::core::CoreClass::Zero;
        assert!(BoundCandidate::prepare(&arena, &too_small, &candidate).is_err());
        assert_eq!(crate::arena::state_hash(&arena), before);
    }
}

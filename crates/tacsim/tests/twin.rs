//! **The twin lane: T0 against T1 over thirty pooled matches** (17b cut M1a.6;
//! doc 17 §17 "Three columns, two gates", §22's M1a gate, §23's first risk;
//! D-UM-3, D-UM-48).
//!
//! T0 is the shipped reflex controller; T1 is the native stock mind
//! (`tacsim::arena::stock_mind::NativeStockMind`), the same doctrine over a
//! hull's view and memory, through the mind schedule and the motor. This file
//! plays them against each other on the proving ground the way the rig plays a
//! duel (`support::shipped_arena`, 16b §0.4: the production check-out, the
//! authored ground and the stock tanks): two tanks a side, fifteen seeds
//! (`2_350 + k`), each seed twice with the sides swapped, `MatchConfig`'s
//! default 900 s cap, stepped at **one substep a pulse** — the cross-tempo
//! invariant (D-16-2) makes every match's bytes those of any cadence, and the
//! shadow below needs the arena exactly as the think saw it. Beside it a
//! **control pool** of the same thirty seeds and sides with T0 on both, so the
//! draw rate has a baseline.
//!
//! **The gate** (the cut's own, doc 17 §22): the 95 per cent Wilson interval
//! on T1's score — its wins plus half its draws, rounded half up — over the
//! thirty contains one half; and T1's pool draws no more than the control's
//! plus six (a fifth of the pool), so a T1 that stalls into timeouts cannot
//! pass on draws. A miss is a STOP for the lead, never a retune here (doc 17
//! §17: the flip is M4's; M1a reports).
//!
//! **Report-only beside it**: the decisive-only interval, Gate B's columns
//! (shots a minute, the median time to the first shot, the states at the end,
//! the path queries and corner refusals of each pool), the timing of a view
//! and a think (doc 17 §23's first risk — measured from the test on a clone,
//! sampled, never in the library) and the thirteen named divergences of doc 17
//! §17, counted on the LIVE arena after each pulse that ran a think: the
//! observer-pure read T1 made (off the recorded think and its view) against
//! the reflex's leaky readers (`#[doc(hidden)] pub` for this file alone).
//!
//! **17b cut M1b.2** put the law on a mind's shot and T1's slots on its
//! hulls, and adds the six divergences that introduces (doc 17 §17; 17b
//! §1(f)): `cond_fire` with its sub-count `cond_stale`, `unwarranted`,
//! `unlawful` (ASSERTED nought: Line's 100 per mille is lawful against any
//! floor the bound allows), `floor_held` and `slot_starved`, and Gate B's
//! `cond_shots` column. **T1's shots a minute do NOT move by the slot's
//! timing alone**: T0 fires ONE weapon a tick (`reflex.rs`' `engage_orders`
//! takes the first ready gun) on ticks `decide_ms` apart, while each armed
//! slot fires its own gun at its own `WeaponReady` — so on a two-gun hull the
//! coaxial gun is no longer serialised behind the main one, and T1 fires
//! shots T0's one-a-tick rule never could. That seventh divergence is counted
//! as `slot_parallel` (and each side's `parallel` on Gate B's line, nought for
//! T0 by construction), a named move for the lead's ruling, never a retune.
//!
//! **Gate A** (17b cuts M2.5 and M2.6; doc 17 §17 "Gate A: T1 ≡ T2 exactly")
//! flies `support::TwinRunner` — T1 and the Rhai Standard Doctrine (T2) on
//! every think, the Rhai fault refused first, then the intents, then the
//! memory bytes, T1's answer flown — over three lanes, every think asserted:
//! the pooled lane's own thirty matches with the twin where the lane flies
//! T1 (`m2_gate_a_over_the_pooled_lanes_matches`, whose record is 17b cut
//! M1b.5's line for line), the mixed armed-and-unarmed lane
//! (`m2_gate_a_on_a_mixed_pool`), and the authored lance field under the
//! lance lane's order table (`m2_gate_a_over_the_authored_lance_field`); and
//! beside them a twin against a twin over the fifteen seeds.
//!
//! **17b cut M2.7** closes Gate A with its third proof, whole-match hashes:
//! each of the pooled lane's thirty plays flown once by T1 and once by T2
//! ALONE (`Flier::T2`, T2's own answers flown), every substep's hash, the
//! ledger, the end and the final state hash equal
//! (`m2_whole_match_hashes_are_equal_between_the_runners_over_the_pool`,
//! `#[ignore]`d past thirty debug seconds and run by a named `ci.yml` step);
//! and prints doc 17 §18's figures over lanes 1 and 3 (`twin_rhai_timing`).

mod support;

use std::sync::{Arc, Mutex};
use std::time::Instant;

use mush_core::ObjId;
use tacsim::arena::conditions::{Arming, FireCondition, Held, HeldWhy, STOCK_CONDITION_SLOTS};
use tacsim::arena::intents::{IntentKind, MotorGoal, Refusal};
use tacsim::arena::ledger::LedgerEvent;
use tacsim::arena::mind::{MindRunner, MindView, ThinkJob, ThinkResult, decode_memory};
use tacsim::arena::motor::stand_off_cell;
use tacsim::arena::perception::{DriveOutcome, FrameNotice, MindObjective};
use tacsim::arena::reflex::{
    ReflexCounters, ReflexState, SCREEN_HALF_ARC_MDEG, ranges, stand_off_want,
};
use tacsim::arena::state::{Arena, ContactId, CostRow, EndReason, Team};
use tacsim::arena::stock_mind::{
    NativeStockMind, StockMemory, Weights, best_hostile_ranked, cell_point, health_permille,
    line_to, row_range_cm, view_ranges, would_overheat,
};
use tacsim::config::ReflexConfig;
use tacsim::deliberation::order::{Objective, Order};

use support::{Muster, wilson_95};

/// The first seed; the lane plays `SEED_0 + k` for `k` in `0..SEEDS`.
const SEED_0: u64 = 2_350;
/// How many seeds (each played twice, the sides swapped).
const SEEDS: u64 = 15;
/// How many thinks of each match are timed on a clone (doc 17 §23).
const TIMED_THINKS: u32 = 40;
/// T1's pool may draw this many more matches than the control's (a fifth).
const DRAW_SLACK: u32 = 6;

// ---------------------------------------------------------------------------
// The recording runner
// ---------------------------------------------------------------------------

/// One think, as the shadow reads it. 17b cut M2.2: the view is the job's
/// own `Arc` (`ThinkJob::view`), shared rather than copied.
#[derive(Clone, Debug)]
struct Thought {
    unit: ObjId,
    think_seq: u64,
    t_ms: i64,
    lands_ms: i64,
    view: Arc<MindView>,
    memory: Vec<u8>,
    result: ThinkResult,
}

/// **The production runner, recorded** — `NativeStockMind` answering every
/// think, each job and answer kept until the lane drains them after the pulse.
/// It is the stock mind and not a stand-in for one (16b §0.4): the answer is
/// the inner runner's, untouched. **17b cut M2.5**: over any runner — T1, T2
/// (the Rhai Standard Doctrine) or Gate A's twin ([`runner_for`]).
#[derive(Debug)]
struct Recording {
    inner: Arc<dyn MindRunner>,
    thoughts: Mutex<Vec<Thought>>,
}

impl Recording {
    fn new(inner: Arc<dyn MindRunner>) -> Arc<Recording> {
        Arc::new(Recording {
            inner,
            thoughts: Mutex::new(Vec::new()),
        })
    }

    fn drain(&self) -> Vec<Thought> {
        self.thoughts
            .lock()
            .map(|mut thoughts| std::mem::take(&mut *thoughts))
            .unwrap_or_default()
    }
}

impl MindRunner for Recording {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        let result = self.inner.think(job);
        if let Ok(mut thoughts) = self.thoughts.lock() {
            thoughts.push(Thought {
                unit: job.unit,
                think_seq: job.think_seq,
                t_ms: job.t_ms,
                lands_ms: job.lands_ms,
                view: Arc::clone(job.view),
                memory: job.memory.to_vec(),
                result: result.clone(),
            });
        }
        result
    }
}

// ---------------------------------------------------------------------------
// One match
// ---------------------------------------------------------------------------

/// Who flies a side. **17b cut M2.5** adds T2 (the Rhai Standard Doctrine,
/// `tacsim::minds::standard_runner`) and `Twin` (Gate A's
/// `support::TwinRunner`: T1 and T2 on every think, T1's answer flown). One
/// runner thinks for every mind of an arena, so a match flies at most one of
/// T1, T2 and `Twin` ([`runner_for`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flier {
    T0,
    T1,
    T2,
    Twin,
}

impl Flier {
    /// Whether the side is flown by a stock mind (T1, T2 or the twin).
    fn is_mind(self) -> bool {
        self != Flier::T0
    }
}

/// **The runner a match's minds think on** (17b cut M2.5): T1's
/// `NativeStockMind` for T0 or T1 (T0 flies no mind), T2's Rhai runner, or
/// an asserting twin; one per arena, so two different mind fliers are
/// refused.
fn runner_for(red: Flier, blue: Flier, cfg: &tacsim::config::ArenaConfig) -> Arc<dyn MindRunner> {
    let mind = match (red, blue) {
        (Flier::T0, other) | (other, Flier::T0) => other,
        (a, b) if a == b => a,
        (a, b) => panic!("one runner an arena: {a:?} and {b:?} cannot share one"),
    };
    match mind {
        Flier::T0 | Flier::T1 => Arc::new(NativeStockMind::with(&cfg.reflex)),
        Flier::T2 => tacsim::minds::standard_runner(cfg)
            .unwrap_or_else(|error| panic!("Standard Doctrine compiles: {error}")),
        Flier::Twin => Arc::new(support::TwinRunner::new(cfg)),
    }
}

/// The thirteen divergences of doc 17 §17, pooled, and the first arena
/// millisecond any of them (`landing` aside, which every think is) fired.
#[derive(Clone, Copy, Debug, Default)]
struct Divergences {
    tiebreak: u64,
    line: u64,
    support: u64,
    reads_dead: u64,
    landing: u64,
    no_line: u64,
    fan: u64,
    order_hint: u64,
    order_end: u64,
    range_round: u64,
    plot_cell: u64,
    deg_round: u64,
    fallback: u64,
    /// **17b cut M1b.2's six** (doc 17 §17, D-UM-16, D-UM-17; 17b §1(f)), each
    /// in [`Divergences::absorb`]'s and the printed line's order: T1 `fired`
    /// lines `by: "cond"` (a slot's shot at a `WeaponReady`, a pass, a share,
    /// a landing, an arming or a startup millisecond where T0 fires on its
    /// tick);
    cond_fire: u64,
    /// of those, the shots whose target is not the target of T1's latest
    /// PLANNED `Lock` before that millisecond (the plot moved between a
    /// retarget think and its landing; T0 retargets and fires in one tick);
    cond_stale: u64,
    /// T1 landings refused `unwarranted` (read off the next think's buffer);
    unwarranted: u64,
    /// T1 armings or citations refused `unlawful` — ASSERTED nought on the
    /// stock lanes: Line's 100 per mille is lawful against any floor the
    /// bound allows;
    unlawful: u64,
    /// thinks that armed where T0's rule (the first weapon ready at the
    /// landing, loaded, in reach and not held by heat — no odds term) would
    /// fire a weapon whose T1 slot `Arena::warrant` holds for `Odds` on the
    /// arena the think saw: the shipped table's `p_min 10` puts a 2 km shot
    /// at about 6 per cent, which T0 fires and Line at the 10 per cent floor
    /// holds — a NAMED divergence D-UM-17 makes unavoidable, counted, never
    /// retuned;
    floor_held: u64,
    /// T0 `fired` lines on a weapon at a component index past
    /// `STOCK_CONDITION_SLOTS` — a gun T1 has no slot for (nought on the
    /// stock tanks).
    slot_starved: u64,
    /// **The seventh** (the M1b.2 review's F1): T1 `fired` lines a
    /// one-weapon-a-tick machine could not have fired — [`unserialised`] over
    /// each T1 hull's shots. T0 picks one gun a tick on ticks `decide_ms`
    /// apart; each armed slot fires its own gun at its own `WeaponReady`, so
    /// the coaxial gun is no longer held behind the main gun's shot. The
    /// rate move Gate B shows beside the timing.
    slot_parallel: u64,
    first_ms: Option<i64>,
    /// Not a divergence: how many thinks read a live order — the basis of
    /// `support`, `order_hint`, `order_end` and `deg_round`, which can only
    /// fire under one.
    ordered_thinks: u64,
}

impl Divergences {
    fn absorb(&mut self, other: &Divergences) {
        self.tiebreak += other.tiebreak;
        self.line += other.line;
        self.support += other.support;
        self.reads_dead += other.reads_dead;
        self.landing += other.landing;
        self.no_line += other.no_line;
        self.fan += other.fan;
        self.order_hint += other.order_hint;
        self.order_end += other.order_end;
        self.range_round += other.range_round;
        self.plot_cell += other.plot_cell;
        self.deg_round += other.deg_round;
        self.fallback += other.fallback;
        self.cond_fire += other.cond_fire;
        self.cond_stale += other.cond_stale;
        self.unwarranted += other.unwarranted;
        self.unlawful += other.unlawful;
        self.floor_held += other.floor_held;
        self.slot_starved += other.slot_starved;
        self.slot_parallel += other.slot_parallel;
        self.ordered_thinks += other.ordered_thinks;
        self.first_ms = match (self.first_ms, other.first_ms) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }

    /// Every counter but `landing`, summed — whether a think diverged.
    fn diverged(&self) -> u64 {
        self.tiebreak
            + self.line
            + self.support
            + self.reads_dead
            + self.no_line
            + self.fan
            + self.order_hint
            + self.order_end
            + self.range_round
            + self.plot_cell
            + self.deg_round
            + self.fallback
    }
}

/// What one side did over a match: Gate B's per-side columns.
#[derive(Clone, Debug, Default)]
struct SideReport {
    shots: u64,
    arena_ms: i64,
    first_shot_ms: Vec<i64>,
    /// The machines' states at the end, by code — a wrecked or disabled
    /// hull counted `disabled` for both fliers (a mind does not think on a
    /// wreck, so its memory keeps the state it last thought in).
    states: [u32; 6],
    /// How many of the side's hulls ended wrecked.
    wrecked: u32,
    /// **17b cut M1b.2**: how many of `shots` were a slot's own (`by: "cond"`).
    cond_shots: u64,
    /// **17b cut M1b.2**: how many of `shots` a one-weapon-a-tick machine
    /// could not have fired ([`unserialised`] per hull) — nought for T0 by
    /// construction, T1's `slot_parallel`.
    parallel: u64,
}

/// What one match came to.
#[derive(Clone, Debug)]
struct Played {
    seed: u64,
    red: Flier,
    end: EndReason,
    t_ms: i64,
    /// The flier of the side that won, or `None` for a draw.
    winner: Option<Flier>,
    sides: [(Flier, SideReport); 2],
    path_queries: u64,
    corner_refusals: u64,
    divergences: Divergences,
    view_us: Vec<u128>,
    think_us: Vec<u128>,
}

/// The kind word and the unit of a journalled line.
fn kind_and_unit(event: &LedgerEvent) -> (String, Option<u32>) {
    let value = serde_json::to_value(&event.kind).unwrap_or_default();
    (
        value
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        value
            .get("unit")
            .and_then(serde_json::Value::as_u64)
            .and_then(|unit| u32::try_from(unit).ok()),
    )
}

/// A `fired` line's `by` word (17b cut M1b.1: `cond` for a slot's shot,
/// `mind` for a mind's cited one), or `None`.
fn fired_by(event: &LedgerEvent) -> Option<String> {
    serde_json::to_value(&event.kind)
        .ok()?
        .get("by")?
        .as_str()
        .map(str::to_owned)
}

/// **The shots a one-weapon-a-tick machine could not have fired** (17b cut
/// M1b.2; the `slot_parallel` divergence): one hull's `fired` lines as
/// `(t_ms, weapon)` in journal order, walked keeping the last shot KEPT — a
/// shot less than `decide_ms` after it is counted (and not kept), any other
/// is kept. T0's reflex fires at most one weapon a tick on ticks `decide_ms`
/// apart, so its count is nought; a gun cannot fire twice inside its own
/// cycle, and no shipped cycle is under `decide_ms`, so what is counted is a
/// SECOND gun fired inside one tick of the first — on a stock tank the
/// coaxial gun's shot the main gun's no longer displaces. Counted once per
/// such shot (a cannon cycle of six ticks: T0 six shots, T1 seven, one
/// counted).
fn unserialised(shots: &[(i64, &str)], decide_ms: i64) -> u64 {
    let mut kept: Option<i64> = None;
    let mut counted = 0_u64;
    for (t_ms, _) in shots {
        match kept {
            Some(at) if t_ms - at < decide_ms => counted += 1,
            _ => kept = Some(*t_ms),
        }
    }
    counted
}

/// One `fired` line as the line rules read it: `(t_ms, unit, weapon, target,
/// by)`.
type FiredLine = (i64, u32, String, Option<u32>, Option<String>);

/// The `fired` lines among `events`, in journal order.
fn fired_lines(events: &[LedgerEvent]) -> Vec<FiredLine> {
    events
        .iter()
        .filter_map(|event| {
            let value = serde_json::to_value(&event.kind).ok()?;
            if value.get("kind").and_then(serde_json::Value::as_str) != Some("fired") {
                return None;
            }
            let as_u32 = |key: &str| {
                value
                    .get(key)
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|id| u32::try_from(id).ok())
            };
            Some((
                event.t_ms,
                as_u32("unit")?,
                value
                    .get("weapon")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                as_u32("target"),
                fired_by(event),
            ))
        })
        .collect()
}

/// `unit`'s shots among `lines`, as [`unserialised`] reads them.
fn shots_of(lines: &[FiredLine], unit: u32) -> Vec<(i64, &str)> {
    lines
        .iter()
        .filter(|(_, by, ..)| *by == unit)
        .map(|(t_ms, _, weapon, ..)| (*t_ms, weapon.as_str()))
        .collect()
}

/// Every `Lock` `thought` planned, `(unit, think ms, the object its row
/// names)`, by the row the think saw — the arena right after the pulse that
/// ran it (one substep a pulse) — pushed onto `planned`.
fn plan_locks(arena: &Arena, thought: &Thought, planned: &mut Vec<(ObjId, i64, ObjId)>) {
    for intent in &thought.result.intents {
        if let IntentKind::Lock { contact } = intent
            && let Some(held) = arena.contacts.resolve(thought.unit, *contact)
        {
            planned.push((thought.unit, thought.t_ms, held.target));
        }
    }
}

/// **17b cut M1b.2's line rules** (the [`Divergences`] fields say each):
/// `cond_fire` and its sub-count `cond_stale` (against the `Lock`s T1's
/// thinks `planned`, [`plan_locks`]), `slot_starved` and `slot_parallel`,
/// read off a match's journal; `flier_of` names who flew a unit.
fn line_divergences(
    arena: &Arena,
    events: &[LedgerEvent],
    flier_of: &dyn Fn(u32) -> Option<Flier>,
    planned: &[(ObjId, i64, ObjId)],
) -> Divergences {
    let mut out = Divergences::default();
    let lines = fired_lines(events);
    for (t_ms, unit, weapon, target, by) in &lines {
        match flier_of(*unit) {
            Some(flier) if flier.is_mind() && by.as_deref() == Some("cond") => {
                out.cond_fire += 1;
                let latest = planned
                    .iter()
                    .filter(|(hull, think_ms, _)| hull.0 == *unit && think_ms < t_ms)
                    .max_by_key(|(_, think_ms, _)| *think_ms)
                    .map(|(_, _, object)| object.0);
                if latest.is_some() && latest != *target {
                    out.cond_stale += 1;
                }
            }
            Some(Flier::T0) => {
                let index = arena
                    .unit(ObjId(*unit))
                    .and_then(|state| state.weapons.iter().position(|row| row.id == *weapon));
                if index.is_some_and(|index| index >= STOCK_CONDITION_SLOTS) {
                    out.slot_starved += 1;
                }
            }
            _ => {}
        }
    }
    let decide_ms = i64::from(arena.arena_cfg.reflex.decide_ms);
    for state in &arena.units {
        if flier_of(state.id.0).is_some_and(Flier::is_mind) {
            out.slot_parallel += unserialised(&shots_of(&lines, state.id.0), decide_ms);
        }
    }
    out
}

/// **One match**: `red` and `blue` fly two tanks each on the proving ground at
/// `seed`, one substep a pulse to the end, with T1's thinks shadowed.
///
/// **17b cut M2.5**: this lane keeps its own `play` — the shadow reads the
/// arena after every pulse, which `support::play` (the lifted one, with a
/// runner argument, for the Gate A lanes) does not offer — over
/// [`play_on`] with the fliers' own runner ([`runner_for`]).
fn play(world: &mush_testkit::TestWorld, seed: u64, red: Flier, blue: Flier) -> Played {
    play_on(world, seed, red, blue, None)
}

/// [`play`] with the minds thinking on `runner` (`None`: [`runner_for`]'s).
fn play_on(
    world: &mush_testkit::TestWorld,
    seed: u64,
    red: Flier,
    blue: Flier,
    runner: Option<Arc<dyn MindRunner>>,
) -> Played {
    let muster = |team: Team, slot: u8, flier: Flier| match flier {
        Flier::T0 => Muster::reflex(team, slot),
        Flier::T1 | Flier::T2 | Flier::Twin => Muster::mind(team, slot),
    };
    let reds = [muster(Team::Red, 1, red), muster(Team::Red, 2, red)];
    let blues = [muster(Team::Blue, 1, blue), muster(Team::Blue, 2, blue)];
    let mut arena = support::shipped_arena_seeded(world, &[&reds, &blues], seed);
    let recording =
        Recording::new(runner.unwrap_or_else(|| runner_for(red, blue, &arena.arena_cfg)));
    arena.mind_runner = recording.clone();
    let cfg = arena.arena_cfg.reflex.clone();
    let timer = NativeStockMind::with(&cfg);
    let mut bytes: Vec<u8> = Vec::new();
    let mut divergences = Divergences::default();
    let mut view_us: Vec<u128> = Vec::new();
    let mut think_us: Vec<u128> = Vec::new();
    let mut timed = 0_u32;
    let mut path_queries = 0_u64;
    let mut corner_refusals = 0_u64;
    let mut wall = arena.now_ms;
    let mut scratch = ReflexCounters::default();
    // 17b cut M1b.2: every `Lock` a T1 think planned, `(unit, think ms, the
    // object its row named)` — what `cond_stale` reads a slot's shot against.
    let mut planned_locks: Vec<(ObjId, i64, ObjId)> = Vec::new();
    // 17b cut M2.5 (17b §5's feas F4): every substep's hash, accumulated
    // after every step — `substep_hashes` is cleared at each step's head —
    // and its length held to the substeps played.
    let (mut hashes, mut substeps): (Vec<u64>, usize) = (Vec::new(), 0);
    let ended = loop {
        let out = arena.step(wall, &[], &[]);
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        hashes.extend(arena.substep_hashes.iter().copied());
        substeps += usize::try_from(out.substeps).unwrap_or(0);
        assert_eq!(
            hashes.len(),
            substeps,
            "one hash a substep played, accumulated over every step"
        );
        path_queries += u64::from(out.reflex.path_queries);
        corner_refusals += u64::from(out.reflex.corner_refusals);
        let thoughts = recording.drain();
        // Doc 17 §23: a view and a think timed on a clone of the arena the
        // think saw, the first `TIMED_THINKS` of the match.
        for thought in &thoughts {
            if timed >= TIMED_THINKS {
                break;
            }
            timed += 1;
            let mut clone = arena.clone();
            let started = Instant::now();
            let view = clone.mind_view(thought.unit);
            view_us.push(started.elapsed().as_micros());
            let Some(view) = view else {
                continue;
            };
            // 17b cut M2.2: wrapped once, as the arena's think arm wraps it.
            let view = Arc::new(view);
            let (memory, card) = clone
                .minds
                .get(&thought.unit)
                .map(|state| (state.memory.clone(), state.card.clone()))
                .unwrap_or_else(|| {
                    (
                        Vec::new(),
                        tacsim::arena::card::line(&clone.arena_cfg.reflex),
                    )
                });
            let job = ThinkJob {
                unit: thought.unit,
                think_seq: thought.think_seq,
                t_ms: thought.t_ms,
                lands_ms: thought.lands_ms,
                seed: clone.seed,
                dropped: false,
                view: &view,
                memory: &memory,
                map: &clone.definition,
                card: &card,
            };
            let started = Instant::now();
            let _ = timer.think(&job);
            think_us.push(started.elapsed().as_micros());
        }
        for thought in &thoughts {
            // The pool orders nothing: no issued order to read T0's side of
            // the order counters against.
            divergences.absorb(&shadow(&arena, &cfg, thought, None, &mut scratch));
            // 17b cut M1b.2: the lock this think planned, by the object its
            // row names now (the row the think saw, one substep a pulse).
            plan_locks(&arena, thought, &mut planned_locks);
        }
        if let Some(ended) = out.ended {
            break ended;
        }
        wall = arena.now_ms + arena.substep_len_ms();
    };
    let events = support::ledger_events(&mut arena, &mut bytes);
    let lines = fired_lines(&events);
    let decide_ms = i64::from(arena.arena_cfg.reflex.decide_ms);
    let fliers = [(Team::Red, red), (Team::Blue, blue)];
    let team_of = |unit: u32| {
        arena
            .units
            .iter()
            .find(|state| state.id.0 == unit)
            .map(|state| state.team)
    };
    let sides = fliers.map(|(team, flier)| {
        let mut report = SideReport {
            arena_ms: ended.t_ms,
            ..SideReport::default()
        };
        let mut first: Option<i64> = None;
        for event in &events {
            let (word, unit) = kind_and_unit(event);
            if word == "fired" && unit.and_then(team_of) == Some(team) {
                report.shots += 1;
                first = Some(first.map_or(event.t_ms, |at| at.min(event.t_ms)));
                if fired_by(event).as_deref() == Some("cond") {
                    report.cond_shots += 1;
                }
            }
        }
        report.first_shot_ms.extend(first);
        for state in arena.units.iter().filter(|state| state.team == team) {
            report.parallel += unserialised(&shots_of(&lines, state.id.0), decide_ms);
            if state.limits.wreck {
                report.wrecked += 1;
            }
            let code = match flier {
                // A wreck, or a hull disabled with no weapons, is `Disabled`
                // whoever flies it: the reflex writes that on its wreck tick,
                // and a mind stops thinking at the wreck with its last state
                // in its memory.
                _ if state.limits.wreck || state.disabled => ReflexState::Disabled.code(),
                Flier::T0 => arena
                    .reflex
                    .unit(state.id)
                    .map_or(ReflexState::Startup, |row| row.state)
                    .code(),
                Flier::T1 | Flier::T2 | Flier::Twin => arena
                    .minds
                    .get(&state.id)
                    .map(|mind| StockMemory::from(&decode_memory(&mind.memory)).state)
                    .unwrap_or_default()
                    .code(),
            };
            if let Some(slot) = report.states.get_mut(usize::from(code)) {
                *slot += 1;
            }
        }
        (flier, report)
    });
    // **17b cut M1b.2**: the counters read off the match's lines.
    let flier_of = |unit: u32| {
        team_of(unit).and_then(|team| {
            fliers
                .iter()
                .find(|(side, _)| *side == team)
                .map(|(_, flier)| *flier)
        })
    };
    divergences.absorb(&line_divergences(
        &arena,
        &events,
        &flier_of,
        &planned_locks,
    ));
    // The two stock fliers' sides, in the order above (red, blue).
    let winner = match ended.reason {
        EndReason::Elimination { winner: Some(team) } => fliers
            .iter()
            .find(|(side, _)| *side == team)
            .map(|(_, flier)| *flier),
        _ => None,
    };
    Played {
        seed,
        red,
        end: ended.reason,
        t_ms: ended.t_ms,
        winner,
        sides,
        path_queries,
        corner_refusals,
        divergences,
        view_us,
        think_us,
    }
}

// ---------------------------------------------------------------------------
// The shadow: T1's observer-pure read against T0's leaky one
// ---------------------------------------------------------------------------

/// **The thirteen divergences of one think**, on the live arena right after
/// the pulse that ran it (one substep a pulse, so the arena is the state the
/// think saw). Each counter is the definition in the module head of
/// `stock_mind.rs`, made a number:
///
/// * `tiebreak` — T1's best two candidates tie on score and the [`ContactId`]
///   order picks a different row from the order of the objects behind them;
/// * `line` — [`line_to`] the target's plot cell and the reflex's
///   `reflex_has_line` to the live target disagree;
/// * `support` — under an issued `Support`, T0's `ordered_target` prefers a
///   target T1 did not settle on, the charge held or — the charge being the
///   hull itself — re-keyed to `Engage` on the order's hint (17b cut L.2; a
///   self-charge fights on doctrine under both, and is counted only where
///   their targets part);
/// * `order_hint` — under an issued `Engage`, T0's `ordered_target` resolves
///   the hint while the view's re-keyed order carries none;
/// * `order_end` — the think set `ignore_until_ms` (17b cut M1b.5; M1a.6's
///   `ignore_order`): an order T0 would have ended
///   that T1 only stops obeying;
/// * `deg_round` — under a `Screen`, a held row's arc verdict off the view's
///   whole degrees differs from the reflex's off the plot's millidegrees.
///
///   The four are read only under an order (`issued` is the order the lane
///   gave the hull, which a mind's view carries only re-keyed); the pool
///   orders nothing, so they count over `ordered_thinks`, printed beside
///   them;
/// * `reads_dead` — a row's `reads_dead` against `Arena::contact_reads_dead`
///   over one state (re-read off a fresh view when a later event of the
///   think's own substep moved the hold);
/// * `landing` — every think, which lands one substep later;
/// * `no_line` — a think that read a `blocked`/`cover` miss notice;
/// * `fan` — a `Goal(Cell)` the motor would fan off its cell;
/// * `range_round` — the view's whole-metre [`view_ranges`] differ from the
///   reflex's centimetre `ranges`, or the utility score's pick differs with no
///   tie to explain it;
/// * `plot_cell` — an `Approach`'s stand-off cell from the plot cell's centre
///   differs from the reflex's from the plot's pose;
/// * `fallback` — a think that performed the waypoint fallback off a
///   `drive_no_route` notice.
///
/// **17b cut M1b.2's six** ([`Divergences`]' fields say each): `unwarranted`,
/// `unlawful` and `floor_held` are read here per think; `cond_fire`,
/// `cond_stale`, `slot_starved` and the seventh, `slot_parallel`, off the
/// match's lines by [`line_divergences`], the first two against the `Lock`s
/// T1's thinks planned ([`plan_locks`]). None of the seven joins
/// `first_divergence_ms`, which stays the M1a thirteen's.
fn shadow(
    arena: &Arena,
    cfg: &ReflexConfig,
    thought: &Thought,
    issued: Option<&Order>,
    scratch: &mut ReflexCounters,
) -> Divergences {
    let mut out = Divergences {
        landing: u64::from(thought.result.intents.len() <= tacsim::arena::mind::MIND_INTENTS_MAX),
        ..Divergences::default()
    };
    let unit = thought.unit;
    let view: &MindView = &thought.view;
    let def = &arena.definition;
    let cell_cm = def.terrain.cell_cm();
    let before = StockMemory::from(&decode_memory(&thought.memory));
    let after = StockMemory::from(&decode_memory(&thought.result.memory));
    // 17b cut M1b.2: `unwarranted` and `unlawful`, the landing refusals the
    // think was handed in its buffer (each notice is read by one think).
    for notice in &view.notices {
        match notice {
            FrameNotice::Refused {
                refusal: Refusal::Unwarranted,
                ..
            } => out.unwarranted += 1,
            FrameNotice::Refused {
                refusal: Refusal::Unlawful,
                ..
            } => out.unlawful += 1,
            _ => {}
        }
    }
    // 17b cut M1b.2: `floor_held` — a think that armed (so `engage_orders`
    // was reached with its gates passed, as T0's tick reaches its own) where
    // T0's rule fires a weapon (the first ready at the landing, loaded, in
    // reach of the target's held range, not held by heat — no odds term)
    // whose slot, as this think armed it, `Arena::warrant` holds for `Odds`
    // on the arena the think saw. A think that plans its FIRST lock reads
    // the arena's lock as it stood (none, or the old one) and so is not
    // counted: the counter is a floor, never an over-count.
    //
    // **17b cut M1b.3**: T1 answers a `Condition` only on a difference with
    // its view's `armed` rows, so "as this think armed it" is the slots it
    // leaves standing — every `Arm` it emitted, else the view's own arming
    // of that weapon (the terms it wanted and found already armed), and none
    // at all beside a `Disarm` (the think that does not reach `engage_orders`
    // with its gates passed clears what is armed, and finds nothing to clear
    // when nothing is).
    let emitted: Vec<&FireCondition> = thought
        .result
        .intents
        .iter()
        .filter_map(|intent| match intent {
            IntentKind::Condition(Arming::Arm(cond)) => Some(cond),
            _ => None,
        })
        .collect();
    let disarmed = thought
        .result
        .intents
        .iter()
        .any(|intent| matches!(intent, IntentKind::Condition(Arming::Disarm { .. })));
    let arms: Vec<FireCondition> = if disarmed {
        Vec::new()
    } else {
        view.weapons
            .iter()
            .take(STOCK_CONDITION_SLOTS)
            .filter_map(|row| {
                emitted
                    .iter()
                    .find(|cond| cond.weapon == row.id)
                    .map(|cond| (*cond).clone())
                    .or_else(|| {
                        row.armed.as_ref().map(|armed| FireCondition {
                            weapon: row.id.clone(),
                            target: armed.target,
                            min_permille: armed.min_permille,
                            aspects: armed.aspects,
                            min_rounds_left: armed.keeps,
                            min_range_m: armed.min_range_m,
                            max_range_m: armed.max_range_m,
                        })
                    })
            })
            .collect()
    };
    if let Some(row) = after
        .target
        .and_then(|target| view.contacts.iter().find(|row| row.id == target))
        && !arms.is_empty()
    {
        let to_landing = thought.lands_ms.saturating_sub(view.t_ms).max(0);
        let range = row_range_cm(row);
        let pick = view.weapons.iter().find(|weapon| {
            weapon.ready_in_ms <= to_landing
                && weapon.rounds > 0
                && weapon
                    .bands_cm
                    .last()
                    .is_some_and(|reach| u64::from(*reach) >= range)
                && !would_overheat(view, weapon)
        });
        if let Some(weapon) = pick
            && let Some(cond) = arms.iter().find(|cond| cond.weapon == weapon.id)
            && matches!(
                arena.warrant(unit, cond, thought.t_ms),
                Err(Held::Why(HeldWhy::Odds { .. }))
            )
        {
            out.floor_held += 1;
        }
    }
    // `no_line` and `fallback`: the notices the think read.
    if view.notices.iter().any(|notice| {
        matches!(notice, FrameNotice::Miss { reason, .. }
            if reason.as_str() == "blocked" || reason.as_str() == "cover")
    }) {
        out.no_line += 1;
    }
    if before.goal.is_some()
        && view.notices.contains(&FrameNotice::Drive {
            outcome: DriveOutcome::NoRoute,
        })
    {
        out.fallback += 1;
    }
    // The order counters, under a live order only.
    if view.order.is_some() {
        out.ordered_thinks += 1;
    }
    if after.ignore_until_ms.is_some() && after.ignore_until_ms != before.ignore_until_ms {
        out.order_end += 1;
    }
    if let Some(order) = issued {
        let leaky = arena.ordered_target(unit, Some(order), &arena.reflex);
        match (order.objective, view.order.as_ref().map(|o| o.objective)) {
            (Objective::Engage { .. }, Some(MindObjective::Engage { target: None }))
                if leaky.is_some() =>
            {
                out.order_hint += 1;
            }
            (Objective::Support { of }, Some(MindObjective::Support { .. })) => {
                let settled = after
                    .target
                    .and_then(|target| arena.contacts.resolve(unit, target))
                    .map(|held| held.target);
                if of == unit || leaky.is_some_and(|target| Some(target) != settled) {
                    out.support += 1;
                }
            }
            // 17b cut L.2: a self-charge lands re-keyed to `Engage` on the
            // order's hint, so the arm above no longer sees it; the counter
            // still does, where T0's target and T1's settled one part.
            (Objective::Support { of }, Some(MindObjective::Engage { .. })) if of == unit => {
                let settled = after
                    .target
                    .and_then(|target| arena.contacts.resolve(unit, target))
                    .map(|held| held.target);
                if leaky.is_some_and(|target| Some(target) != settled) {
                    out.support += 1;
                }
            }
            _ => {}
        }
    }
    if let Some(MindObjective::Screen { facing_mdeg }) = view.order.as_ref().map(|o| o.objective)
        && let Some(state) = arena.unit(unit)
    {
        let arc = |a: u32, b: u32| {
            let apart = (a % 360_000).abs_diff(b % 360_000);
            apart.min(360_000 - apart)
        };
        for row in &view.contacts {
            let Some(held) = arena.contacts.resolve(unit, row.id) else {
                continue;
            };
            let pure =
                arc(row.bearing_deg.saturating_mul(1_000), facing_mdeg) <= SCREEN_HALF_ARC_MDEG;
            let leaky =
                arc(held.seen_at.bearing_from(&state.pose), facing_mdeg) <= SCREEN_HALF_ARC_MDEG;
            if pure != leaky {
                out.deg_round += 1;
            }
        }
    }
    // `fan`: a goal the motor would land elsewhere.
    for intent in &thought.result.intents {
        if let IntentKind::Goal(MotorGoal::Cell { x, y }) = intent
            && arena.fan_goal(unit, (*x, *y)) != (*x, *y)
        {
            out.fan += 1;
        }
    }
    // `reads_dead`: the view's against the reflex's reader. The arena is at
    // the end of the think's substep, and an impact or a pass due later in
    // that substep may already have moved the hold the view was built from;
    // so a disagreement is re-read off a view built NOW (a clone's — only on
    // a disagreement, never per think), and counted only when the two readers
    // still part over one state, or the row the view held is gone.
    let mut late: Option<MindView> = None;
    for row in &view.contacts {
        let Some(said) = row.reads_dead else {
            continue;
        };
        let leaky = arena
            .contacts
            .resolve(unit, row.id)
            .map(|held| arena.contact_reads_dead(held));
        if leaky == Some(said) {
            continue;
        }
        let Some(leaky) = leaky else {
            // The hold expired, or its unit left, later in the substep: not a
            // reading of one state by two readers.
            continue;
        };
        let now = late
            .get_or_insert_with(|| {
                arena
                    .clone()
                    .mind_view(unit)
                    .unwrap_or_else(|| view.clone())
            })
            .contacts
            .iter()
            .find(|again| again.id == row.id)
            .and_then(|again| again.reads_dead);
        if now != Some(leaky) {
            out.reads_dead += 1;
        }
    }
    // `tiebreak` and `range_round`: the utility score's pick, both ways.
    if let Some(state) = arena.unit(unit) {
        let ranked =
            best_hostile_ranked(view, &Weights::of(cfg), health_permille(view), None, |_| {
                true
            });
        let leaky = arena.best_hostile_where(
            unit,
            &arena.reflex,
            arena.reflex_health_permille(unit),
            None,
            |_| true,
        );
        let mut picks_differ = false;
        match ranked.first() {
            Some(&(top, first)) => {
                let tied: Vec<ContactId> = ranked
                    .iter()
                    .take_while(|(score, _)| *score == top)
                    .map(|(_, id)| *id)
                    .collect();
                if tied.len() > 1 {
                    let by_object = tied
                        .iter()
                        .copied()
                        .min_by_key(|id| arena.contacts.resolve(unit, *id).map(|held| held.target));
                    if by_object != Some(first) {
                        out.tiebreak += 1;
                    }
                } else if leaky.map(|(id, _)| id) != Some(first) {
                    picks_differ = true;
                }
            }
            None => picks_differ = leaky.is_some(),
        }
        let leaky_ranges = ranges(state, cell_cm, cfg, None);
        let pure_ranges = view_ranges(view, cfg.engage_band_permille, None);
        if picks_differ || leaky_ranges != pure_ranges {
            out.range_round += 1;
        }
        // `line` and `plot_cell`: on the target T1 settled on.
        if let Some(target) = after.target
            && let Some(row) = view.contacts.iter().find(|row| row.id == target)
            && let Some(held) = arena.contacts.resolve(unit, target)
        {
            let pure = match (view.cell, row.cell) {
                (Some(own), Some(cell)) => line_to(def, own, cell),
                _ => true,
            };
            if pure != arena.reflex_has_line(unit, held.target, scratch) {
                out.line += 1;
            }
            if after.state == ReflexState::Approach {
                let own = (
                    view.x_cm,
                    view.y_cm,
                    i32::try_from(view.z_cm).unwrap_or(i32::MAX),
                );
                let pure_cell = row.cell.and_then(|cell| {
                    let want = stand_off_want(
                        pure_ranges.engage_cm,
                        u64::from(cell_cm.max(1)),
                        row_range_cm(row),
                    );
                    stand_off_cell(def, CostRow::Tracked, own, cell_point(def, cell), want)
                });
                let want = stand_off_want(
                    leaky_ranges.engage_cm,
                    u64::from(cell_cm.max(1)),
                    held.seen_at.range_from(&state.pose),
                );
                if pure_cell != arena.stand_off_cell(unit, &held.seen_at, want) {
                    out.plot_cell += 1;
                }
            }
        }
    }
    if out.diverged() > 0 {
        out.first_ms = Some(thought.t_ms);
    }
    out
}

// ---------------------------------------------------------------------------
// The pool
// ---------------------------------------------------------------------------

/// The median of a list, `0` for none.
fn median(values: &mut [i64]) -> i64 {
    values.sort_unstable();
    values.get(values.len() / 2).copied().unwrap_or(0)
}

/// The `p`th percentile (0–100) of a list, `0` for none.
fn percentile(values: &mut [u128], p: usize) -> u128 {
    values.sort_unstable();
    if values.is_empty() {
        return 0;
    }
    let at = (values.len() * p / 100).min(values.len() - 1);
    values.get(at).copied().unwrap_or(0)
}

/// The end of a match, as the report spells it.
fn end_word(end: EndReason) -> String {
    match end {
        EndReason::Elimination { winner: Some(team) } => {
            format!("elimination_{}", format!("{team:?}").to_lowercase())
        }
        EndReason::Elimination { winner: None } => "elimination_draw".to_owned(),
        other => format!("{other:?}").to_lowercase(),
    }
}

/// Gate B's columns for one side of a pool.
fn gate_b_line(label: &str, reports: &[&SideReport]) -> String {
    let shots: u64 = reports.iter().map(|report| report.shots).sum();
    let minutes_ms: i64 = reports.iter().map(|report| report.arena_ms).sum();
    let shots_per_min = if minutes_ms > 0 {
        shots as f64 * 60_000.0 / minutes_ms as f64
    } else {
        0.0
    };
    let mut firsts: Vec<i64> = reports
        .iter()
        .flat_map(|report| report.first_shot_ms.iter().copied())
        .collect();
    let mut states = [0_u32; 6];
    let wrecked: u32 = reports.iter().map(|report| report.wrecked).sum();
    let cond_shots: u64 = reports.iter().map(|report| report.cond_shots).sum();
    let parallel: u64 = reports.iter().map(|report| report.parallel).sum();
    for report in reports {
        for (total, one) in states.iter_mut().zip(report.states.iter()) {
            *total += *one;
        }
    }
    format!(
        "twin_gate_b: side={label} shots={shots} shots_per_min={shots_per_min:.3} \
         time_to_first_shot_ms={} first_shots={} states_at_end=[startup={},patrol={},approach={},engage={},withdraw={},disabled={}] wrecked={wrecked} \
         cond_shots={cond_shots} parallel={parallel}",
        median(&mut firsts),
        firsts.len(),
        states[0],
        states[1],
        states[2],
        states[3],
        states[4],
        states[5],
    )
}

/// **The pooled lane's record** (17b cut M2.6, lifted out of
/// [`m1a6_t0_versus_t1_over_thirty_pooled_matches_is_inside_a_wilson_interval_containing_half`]
/// so Gate A's first lane prints and compares the same lines): every line
/// the lane prints but the timing, which is the wall clock's — `head` the
/// thirty `twin_match` lines, the thirty `twin_control_match` lines,
/// `twin_pooled`, `twin_divergences` and `twin_divergence_basis`; `tail`
/// Gate B's three side lines and `twin_gate_b_pool` — with the figures the
/// gate asserts. `mind` is the flier of the pool's mind sides (T1, or the
/// twin, whose answer is T1's), printed `t1` either way.
struct PoolRecord {
    head: Vec<String>,
    tail: Vec<String>,
    n: u32,
    score: u32,
    draws: u32,
    control_draws: u32,
    interval: (f64, f64),
    divergences: Divergences,
    control_divergences: Divergences,
}

/// [`PoolRecord`] of `pool` (the mind flier `mind` against T0, both
/// assignments a seed) and `control` (T0 on both sides, each match counted
/// for both assignments).
#[allow(clippy::too_many_lines)]
fn pool_record(pool: &[Played], control: &[Played], mind: Flier) -> PoolRecord {
    let mut head: Vec<String> = Vec::new();
    let mut tail: Vec<String> = Vec::new();
    let n = u32::try_from(pool.len()).unwrap_or(u32::MAX);
    let t1_wins = u32::try_from(
        pool.iter()
            .filter(|played| played.winner == Some(mind))
            .count(),
    )
    .unwrap_or(0);
    let t0_wins = u32::try_from(
        pool.iter()
            .filter(|played| played.winner == Some(Flier::T0))
            .count(),
    )
    .unwrap_or(0);
    let draws = n - t1_wins - t0_wins;
    let score = (2 * t1_wins + draws).div_ceil(2);
    let interval = wilson_95(score, n);
    let decisive = wilson_95(t1_wins, t1_wins + t0_wins);
    let control_red_wins = control
        .iter()
        .filter(|played| {
            matches!(
                played.end,
                EndReason::Elimination {
                    winner: Some(Team::Red)
                }
            )
        })
        .count();
    let control_blue_wins = control
        .iter()
        .filter(|played| {
            matches!(
                played.end,
                EndReason::Elimination {
                    winner: Some(Team::Blue)
                }
            )
        })
        .count();
    let control_draws =
        u32::try_from(control.len() - control_red_wins - control_blue_wins).unwrap_or(u32::MAX);
    for played in pool {
        let t1 = if played.red == mind { "red" } else { "blue" };
        head.push(format!(
            "twin_match: seed={} t1={t1} end={} t_ms={}",
            played.seed,
            end_word(played.end),
            played.t_ms
        ));
    }
    for (at, played) in control.iter().enumerate() {
        head.push(format!(
            "twin_control_match: seed={} run={} end={} t_ms={}",
            played.seed,
            at % 2,
            end_word(played.end),
            played.t_ms
        ));
    }
    head.push(format!(
        "twin_pooled: n={n} t1_wins={t1_wins} t0_wins={t0_wins} draws={draws} \
         interval=[{:.3}, {:.3}] decisive_interval=[{:.3}, {:.3}] control_draws={control_draws} \
         control_red_wins={control_red_wins} control_blue_wins={control_blue_wins}",
        interval.0, interval.1, decisive.0, decisive.1
    ));
    let mut divergences = Divergences::default();
    for played in pool {
        divergences.absorb(&played.divergences);
    }
    head.push(format!(
        "twin_divergences: tiebreak={} line={} support={} reads_dead={} landing={} no_line={} \
         fan={} order_hint={} order_end={} range_round={} plot_cell={} deg_round={} fallback={} \
         cond_fire={} cond_stale={} unwarranted={} unlawful={} floor_held={} slot_starved={} \
         slot_parallel={} first_divergence_ms={}",
        divergences.tiebreak,
        divergences.line,
        divergences.support,
        divergences.reads_dead,
        divergences.landing,
        divergences.no_line,
        divergences.fan,
        divergences.order_hint,
        divergences.order_end,
        divergences.range_round,
        divergences.plot_cell,
        divergences.deg_round,
        divergences.fallback,
        divergences.cond_fire,
        divergences.cond_stale,
        divergences.unwarranted,
        divergences.unlawful,
        divergences.floor_held,
        divergences.slot_starved,
        divergences.slot_parallel,
        divergences
            .first_ms
            .map_or_else(|| "none".to_owned(), |ms| ms.to_string()),
    ));
    let mut control_divergences = Divergences::default();
    for played in control {
        control_divergences.absorb(&played.divergences);
    }
    // What the four order counters were read over: a nought there is a
    // measured nought only over thinks that had an order to read.
    head.push(format!(
        "twin_divergence_basis: thinks={} ordered_thinks={} (support, order_hint, order_end and \
         deg_round read only under an order; the pool orders nothing)",
        divergences.landing, divergences.ordered_thinks
    ));
    for flier in [mind, Flier::T0] {
        let reports: Vec<&SideReport> = pool
            .iter()
            .flat_map(|played| played.sides.iter())
            .filter(|(side, _)| *side == flier)
            .map(|(_, report)| report)
            .collect();
        let label = if flier == mind { "t1" } else { "t0" };
        tail.push(gate_b_line(label, &reports));
    }
    let reports: Vec<&SideReport> = control
        .iter()
        .flat_map(|played| played.sides.iter())
        .map(|(_, report)| report)
        .collect();
    tail.push(gate_b_line("control", &reports));
    tail.push(format!(
        "twin_gate_b_pool: path_queries={} corner_refusals={} control_path_queries={} \
         control_corner_refusals={}",
        pool.iter().map(|p| p.path_queries).sum::<u64>(),
        pool.iter().map(|p| p.corner_refusals).sum::<u64>(),
        control.iter().map(|p| p.path_queries).sum::<u64>(),
        control.iter().map(|p| p.corner_refusals).sum::<u64>(),
    ));
    PoolRecord {
        head,
        tail,
        n,
        score,
        draws,
        control_draws,
        interval,
        divergences,
        control_divergences,
    }
}

/// **The gate of 17b cut M1a** (doc 17 §22): T0 against T1 over thirty pooled
/// matches, inside a Wilson interval containing a half, with T1's draws
/// bounded by a control pool of T0 against T0 — see the module head. Every
/// `twin_*` line is printed once; run it with `--nocapture` to read them.
///
/// **Not ignored and not gated**: forty-five distinct matches (the control's
/// two assignments are one match) of three to four arena minutes each run in
/// about 20 s in a debug build and 2 s in release, inside the four minutes an
/// ordinary run may spend. The gate run is `cargo test --release -p tacsim
/// --test twin -- --nocapture`.
#[test]
fn m1a6_t0_versus_t1_over_thirty_pooled_matches_is_inside_a_wilson_interval_containing_half() {
    let world = support::booted_world();
    let mut pool: Vec<Played> = Vec::new();
    let mut control: Vec<Played> = Vec::new();
    for k in 0..SEEDS {
        let seed = SEED_0 + k;
        pool.push(play(&world, seed, Flier::T0, Flier::T1));
        pool.push(play(&world, seed, Flier::T1, Flier::T0));
        // T0 on both sides is one match whichever side is called which: it
        // is played once and counted for both assignments.
        let twice = play(&world, seed, Flier::T0, Flier::T0);
        control.push(twice.clone());
        control.push(twice);
    }
    let record = pool_record(&pool, &control, Flier::T1);
    for line in &record.head {
        println!("{line}");
    }
    let mut view_us: Vec<u128> = pool
        .iter()
        .flat_map(|p| p.view_us.iter().copied())
        .collect();
    let mut think_us: Vec<u128> = pool
        .iter()
        .flat_map(|p| p.think_us.iter().copied())
        .collect();
    println!(
        "twin_timing: view_us_p50={} view_us_p99={} think_us_p50={} think_us_p99={} samples={}",
        percentile(&mut view_us, 50),
        percentile(&mut view_us, 99),
        percentile(&mut think_us, 50),
        percentile(&mut think_us, 99),
        think_us.len()
    );
    for line in &record.tail {
        println!("{line}");
    }
    let PoolRecord {
        n,
        score,
        draws,
        control_draws,
        interval,
        divergences,
        control_divergences,
        ..
    } = record;
    assert!(
        interval.0 <= 0.5 && 0.5 <= interval.1,
        "T1's score {score}/{n} has a Wilson interval [{:.3}, {:.3}] that excludes one half",
        interval.0,
        interval.1
    );
    assert!(
        draws <= control_draws + DRAW_SLACK,
        "T1's pool drew {draws}, the control {control_draws}: more than {DRAW_SLACK} over"
    );
    // 17b cut M1b.2 (17b §1(f); D-UM-17): Line's 100 per mille is lawful
    // against any floor the bound allows, so on both stock lanes nothing T1
    // arms or cites is ever refused `unlawful` — a non-zero count is a STOP.
    assert_eq!(
        (divergences.unlawful, control_divergences.unlawful),
        (0, 0),
        "a stock mind's arming or citation was refused unlawful"
    );
}

/// `wilson_95` against four values worked by hand: 15 of 30 is `[0.331539,
/// 0.668461]`, 0 of 30 `[0, 0.113517]`, 24 of 30 `[0.626940, 0.904950]` and
/// 9 of 30 `[0.166646, 0.478761]`, to six places; nothing of nothing says
/// nothing.
#[test]
fn m1a6_the_wilson_interval_is_the_hand_worked_one() {
    for (successes, n, low, high) in [
        (15_u32, 30_u32, 0.331_539_f64, 0.668_461_f64),
        (0, 30, 0.0, 0.113_517),
        (24, 30, 0.626_940, 0.904_950),
        (9, 30, 0.166_646, 0.478_761),
    ] {
        let (lo, hi) = wilson_95(successes, n);
        assert!(
            (lo - low).abs() < 1e-6 && (hi - high).abs() < 1e-6,
            "{successes}/{n}: [{lo}, {hi}]"
        );
    }
    assert_eq!(wilson_95(0, 0), (0.0, 1.0));
}

// ---------------------------------------------------------------------------
// The lifted kernels against the reflex's own, on the shipped ground
// ---------------------------------------------------------------------------

/// **`fallback_waypoint` is the motor's fallback** (17b cut M1a.6: the
/// arithmetic `Arena::fall_back_to_waypoint` is now a wrapper of): for every
/// node of the proving ground's graph, a hull standing on it whose failed
/// goal is that node (the onward step, `unit % neighbours`), another cell or
/// none, walking to that node or to none, gets the same waypoint, the same
/// node walked from and the same goal from both, and the row's route is
/// spent. At least one failed goal on a node steps onward.
#[test]
fn m1a6_fallback_waypoint_is_the_motors_fallback_on_the_shipped_graph() {
    use tacsim::arena::motor::{MotorRow, fallback_waypoint};
    let world = support::booted_world();
    let arena = support::shipped_arena(
        &world,
        &[
            &[Muster::reflex(Team::Red, 1)],
            &[Muster::reflex(Team::Blue, 1)],
        ],
    );
    let unit = support::tank(&world, Team::Red, 1);
    let graph = &arena.definition.waypoints;
    assert!(graph.nodes().len() >= 4, "the shipped graph");
    let mut onward = 0_usize;
    for (at, node) in (0_u16..).zip(graph.nodes()) {
        let here = (node.x, node.y);
        for goal in [Some(here), Some((node.x.saturating_add(9), node.y)), None] {
            for waypoint in [Some(at), None] {
                let mut row = MotorRow {
                    goal,
                    waypoint,
                    from_waypoint: Some(at.saturating_add(1)),
                    route: vec![here],
                    ..MotorRow::default()
                };
                arena.fall_back_to_waypoint(unit, &mut row, here);
                let free = fallback_waypoint(
                    &arena.definition,
                    here,
                    goal,
                    waypoint,
                    Some(at.saturating_add(1)),
                    unit.0,
                );
                assert_eq!(
                    (row.waypoint, row.from_waypoint, row.goal),
                    free,
                    "node {at}"
                );
                assert!(row.route.is_empty());
                if goal == Some(here) && free.2 != Some(here) {
                    onward += 1;
                }
            }
        }
    }
    assert!(onward > 0, "a failed node steps onward along the graph");
}

/// **`line_to` against the reflex's `fire_line` on the proving ground** (the
/// divergence `line`): a red stock tank holding a blue plotted on a cell with
/// a clear line reads a line both ways — the plot is the truth. Then the blue
/// drives behind cover, LIVE, with no pass between: the reflex's trace to the
/// target's true cell has lost the line, `line_to` the plot cell has not.
#[test]
fn m1a6_line_to_the_plot_parts_from_fire_line_once_the_target_drives_behind_cover() {
    use tacsim::arena::state::{SeenPose, ShadowContact};
    let world = support::booted_world();
    let mut arena = support::shipped_arena(
        &world,
        &[
            &[Muster::reflex(Team::Red, 1)],
            &[Muster::piloted(Team::Blue, 1)],
        ],
    );
    let red = support::tank(&world, Team::Red, 1);
    let blue = support::tank(&world, Team::Blue, 1);
    let def = arena.definition.clone();
    let Some(own) = support::cell_of(&arena, red) else {
        panic!("red on the window");
    };
    // A clear cell and a blocked one within 60 cells, both drivable.
    let mut clear: Option<(u16, u16)> = None;
    let mut blocked: Option<(u16, u16)> = None;
    'scan: for dy in -60_i32..=60 {
        for dx in -60_i32..=60 {
            if dx.abs().max(dy.abs()) < 4 {
                continue;
            }
            let (Ok(x), Ok(y)) = (
                u16::try_from(i32::from(own.0) + dx),
                u16::try_from(i32::from(own.1) + dy),
            ) else {
                continue;
            };
            if !def.terrain.passable_for(CostRow::Tracked, x, y) {
                continue;
            }
            if line_to(&def, own, (x, y)) {
                clear.get_or_insert((x, y));
            } else {
                blocked.get_or_insert((x, y));
            }
            if clear.is_some() && blocked.is_some() {
                break 'scan;
            }
        }
    }
    let (Some(clear), Some(blocked)) = (clear, blocked) else {
        panic!("the proving ground has open and covered ground near the red entry");
    };
    let put = |arena: &mut Arena, cell: (u16, u16)| {
        if let Some(state) = arena.unit_mut(blue) {
            state.pose = support::at_cell(cell, &state.pose);
        }
    };
    put(&mut arena, clear);
    let plot = arena.unit(blue).map(SeenPose::of);
    let Some(plot) = plot else {
        panic!("blue");
    };
    let id = ContactId(0xB1);
    arena.contacts.insert(
        red,
        id,
        ShadowContact::sighted(
            blue,
            tacsim::arena::state::Classification::Classified { team: Team::Blue },
            0,
            plot,
        ),
    );
    let mut counters = ReflexCounters::default();
    assert!(line_to(&def, own, clear));
    assert!(
        arena.reflex_has_line(red, blue, &mut counters),
        "the plot is the truth"
    );
    put(&mut arena, blocked);
    assert!(
        !arena.reflex_has_line(red, blue, &mut counters),
        "the reflex traces the live cell"
    );
    assert!(
        line_to(&def, own, clear),
        "the stock mind traces the plot it holds"
    );
    assert!(!line_to(&def, own, blocked));
}

/// **The probe's line rules count what they name** (17b cut M1b.2; the M1b.2
/// review's tests/T3): [`line_divergences`] itself — the code the pooled lane
/// reads — over a hand-written journal on the shipped ground, red flown by
/// T1, blue by T0 with a third gun, so each rule is shown counting:
///
/// * red's four `by: "cond"` lines are `cond_fire` 4; the one whose target is
///   not the object of red's latest `Lock` planned STRICTLY before it (a lock
///   planned at the shot's own millisecond is not before it) is `cond_stale`
///   1; red's cited `by: "mind"` line is neither;
/// * red's machine gun 400 ms after its cannon is `slot_parallel` 1 — a
///   second gun inside one `decide_ms` tick, which T0's one-a-tick rule never
///   fires — and blue's like pair is not counted there (T1's divergence),
///   though [`unserialised`] counts it on blue's shots alike;
/// * blue's shot on its third gun (component index 2, past
///   `STOCK_CONDITION_SLOTS`) is `slot_starved` 1.
#[test]
fn m1b2_the_probes_line_rules_count_what_they_name() {
    use tacsim::arena::ledger::EventKind;
    let world = support::booted_world();
    let mut arena = support::shipped_arena(
        &world,
        &[
            &[Muster::mind(Team::Red, 1)],
            &[Muster::reflex(Team::Blue, 1)],
        ],
    );
    let red = support::tank(&world, Team::Red, 1);
    let blue = support::tank(&world, Team::Blue, 1);
    let elsewhere = ObjId(77);
    if let Some(state) = arena.unit_mut(blue) {
        let Some(mut spare) = state.weapons.last().cloned() else {
            panic!("blue's guns");
        };
        spare.id = "spare".into();
        state.weapons.push(spare);
    }
    assert_eq!(
        arena
            .unit(blue)
            .and_then(|state| state.weapons.iter().position(|row| row.id == "spare")),
        Some(STOCK_CONDITION_SLOTS),
        "the precondition: the spare is the first gun past the slots"
    );
    let (cannon, coax) = match arena.unit(red).map(|state| state.weapons.as_slice()) {
        Some([cannon, coax, ..]) => (cannon.id.to_string(), coax.id.to_string()),
        _ => panic!("red's two guns"),
    };
    let line = |seq: u64, t_ms: i64, unit: ObjId, weapon: &str, target: ObjId, by: Option<&str>| {
        LedgerEvent {
            seq,
            t_ms,
            kind: EventKind::Fired {
                unit,
                weapon: weapon.into(),
                shot_seq: seq,
                target,
                range_cm: 0,
                rounds: 1,
                by: by.map(Into::into),
                slot: by.map(|_| 1),
            },
            draws: Vec::new(),
            hash: 0,
        }
    };
    let events = vec![
        line(1, 1_000, red, &cannon, blue, Some("cond")),
        line(2, 1_000, blue, &cannon, red, None),
        line(3, 1_400, red, &coax, blue, Some("cond")),
        line(4, 2_000, blue, "spare", red, None),
        line(5, 2_500, red, &coax, blue, Some("cond")),
        line(6, 2_500, blue, &coax, red, None),
        line(7, 3_600, red, &coax, blue, Some("cond")),
        line(8, 4_700, red, &cannon, elsewhere, Some("mind")),
    ];
    let planned = [(red, 500, blue), (red, 2_500, elsewhere)];
    let flier_of = |unit: u32| {
        if unit == red.0 {
            Some(Flier::T1)
        } else if unit == blue.0 {
            Some(Flier::T0)
        } else {
            None
        }
    };
    let counted = line_divergences(&arena, &events, &flier_of, &planned);
    assert_eq!(
        (
            counted.cond_fire,
            counted.cond_stale,
            counted.slot_starved,
            counted.slot_parallel
        ),
        (4, 1, 1, 1),
        "{counted:?}"
    );
    let decide_ms = i64::from(arena.arena_cfg.reflex.decide_ms);
    assert_eq!(decide_ms, 1_000, "the precondition: the shipped tick");
    let lines = fired_lines(&events);
    assert_eq!(unserialised(&shots_of(&lines, blue.0), decide_ms), 1);
    assert_eq!(unserialised(&shots_of(&lines, red.0), decide_ms), 1);
}

/// **The probe's law counters count on a field that presents nothing** (17b
/// cut M1b.2; the M1b.2 review's tests/T3): the shipped ground at the first
/// seed, red one stock tank flown by T1, blue one flown by T0 — every hull's
/// silhouette taken off, so every shot is at nought per cent (nothing
/// authored for a round to strike) — and blue's two stock guns emptied beside
/// a third, loaded `spare`. Stepped one substep a pulse with T1's thinks
/// recorded and read by [`shadow`] and [`line_divergences`], the code the
/// pooled lane reads, for five arena minutes:
///
/// * `floor_held` — T0's rule would fire at nought on a think where Line's
///   slot holds for `Odds` (D-UM-17's named divergence) — at least once;
/// * `unwarranted` — T1's fresh-lock citation of a target it cannot hit — at
///   least once;
/// * `slot_starved` — T0 firing its third gun — at least once;
/// * and, nothing being warranted, no slot ever fires: `cond_fire`,
///   `cond_stale` and `slot_parallel` nought, and `unlawful` nought (Line's
///   100 per mille is lawful).
#[test]
fn m1b2_the_probes_law_counters_count_on_a_field_that_presents_nothing() {
    const UNTIL_MS: i64 = 300_000;
    let world = support::booted_world();
    let mut arena = support::shipped_arena_seeded(
        &world,
        &[
            &[Muster::mind(Team::Red, 1)],
            &[Muster::reflex(Team::Blue, 1)],
        ],
        SEED_0,
    );
    let red = support::tank(&world, Team::Red, 1);
    let blue = support::tank(&world, Team::Blue, 1);
    for state in &mut arena.units {
        state.profile = tacsim::arena::resolution::UnitProfile::default();
    }
    if let Some(state) = arena.unit_mut(blue) {
        let Some(mut spare) = state.weapons.last().cloned() else {
            panic!("blue's guns");
        };
        spare.id = "spare".into();
        let stock: Vec<_> = state.weapons.iter().map(|row| row.id.clone()).collect();
        for weapon in stock {
            state.ammo.insert(weapon, 0);
        }
        state.ammo.insert(spare.id.clone(), spare.count_per_load);
        state.weapons.push(spare);
    }
    let recording = Recording::new(Arc::new(NativeStockMind::with(&arena.arena_cfg.reflex)));
    arena.mind_runner = recording.clone();
    let cfg = arena.arena_cfg.reflex.clone();
    let mut scratch = ReflexCounters::default();
    let mut counted = Divergences::default();
    let mut planned: Vec<(ObjId, i64, ObjId)> = Vec::new();
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = arena.now_ms;
    while arena.now_ms < UNTIL_MS {
        let out = arena.step(wall, &[], &[]);
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        for thought in &recording.drain() {
            counted.absorb(&shadow(&arena, &cfg, thought, None, &mut scratch));
            plan_locks(&arena, thought, &mut planned);
        }
        if out.ended.is_some() {
            break;
        }
        wall = arena.now_ms + arena.substep_len_ms();
    }
    let events = support::ledger_events(&mut arena, &mut bytes);
    let flier_of = |unit: u32| {
        if unit == red.0 {
            Some(Flier::T1)
        } else if unit == blue.0 {
            Some(Flier::T0)
        } else {
            None
        }
    };
    counted.absorb(&line_divergences(&arena, &events, &flier_of, &planned));
    println!(
        "m1b2_probe_field: t_ms={} floor_held={} unwarranted={} slot_starved={} cond_fire={} \
         unlawful={}",
        arena.now_ms,
        counted.floor_held,
        counted.unwarranted,
        counted.slot_starved,
        counted.cond_fire,
        counted.unlawful
    );
    assert!(counted.floor_held > 0, "floor_held counts: {counted:?}");
    assert!(counted.unwarranted > 0, "unwarranted counts: {counted:?}");
    assert!(counted.slot_starved > 0, "slot_starved counts: {counted:?}");
    assert_eq!(
        (
            counted.cond_fire,
            counted.cond_stale,
            counted.slot_parallel,
            counted.unlawful
        ),
        (0, 0, 0, 0),
        "nothing warranted, nothing unlawful: {counted:?}"
    );
}

// ---------------------------------------------------------------------------
// 17b cut M2.5: Gate A on unarmed hulls; 17b cut M2.6: on armed hulls, over
// three lanes
// ---------------------------------------------------------------------------

/// The mixed lane's cap, arena seconds (17b §1 (iii)).
const MIXED_CAP_S: u32 = 300;

/// The object a journalled line's `key` names (`target` of a `hit`, `unit`
/// of a `unit_disabled`), or `None`.
fn object_at(event: &LedgerEvent, key: &str) -> Option<u32> {
    serde_json::to_value(&event.kind)
        .ok()?
        .get(key)?
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
}

/// **Gate A on unarmed hulls, under fire — and since 17b cut M2.6 on the
/// armed hulls beside them** (17b cuts M2.5 and M2.6; doc 17 §17 "Gate A",
/// §22's italics — *the twin corpus carries unarmed hulls and Gate A covers
/// them*; D-UM-48; 17b §1 (iii)'s mixed lane and §5's adv F9 — Gate A's
/// second lane): the proving ground at the fifteen seeds, ONE armed
/// `Muster::mind` and ONE `Muster::mind_unarmed` a side, every mind flown by
/// `support::TwinRunner` in its ASSERTING mode, to the end or 300 arena
/// seconds (`support::play`, one substep a pulse, the hashes accumulated).
///
/// * **every think of every hull is Gate A**: no Rhai fault and T2's intents
///   and memory bytes T1's — the twin panics on the first that is not, with
///   its printout, and each recorded think is held to it again by the hull's
///   id (17b cut M2.6: the armed hulls' fire half is ported, so the M2.5
///   allowance for a fire-only divergence on an armed hull — the armed count
///   — is retired);
/// * **the unarmed hulls are under fire**: across the seeds some unarmed hull
///   takes a hit, some withdraws, and some rallies or is disabled (a
///   `unit_disabled`/`unit_destroyed` line — a wreck thinks no more, so the
///   line is not a compared think) — the counts are asserted nonzero, so the
///   lane is not a quiet one;
/// * the hulls are what they were mustered as: an unarmed hull's every view
///   carries no weapon row, and each armed hull's first think's does;
/// * the thinks at the `Disabled` edge (state 5), rallied, and in the
///   withdraw dwell (`stand_until_ms` set) are counted for each kind of hull
///   and printed — compared, as every think is; the arms the lanes reach
///   seldom are held by `tacsim::minds::content`'s mirrors
///   (`m2_mirror_the_withdraw_dwell_and_the_rally`,
///   `m2_mirror_the_disabled_edge_rewrites_the_fourteen_keys`);
/// * the Rhai thinks' operations plus fees, p50/p99/max, printed against
///   class I's 1,500 (a max over 1,200 is a STOP) and Class 0's 400 (D-UM-35:
///   where §18's `minds_stock_ci` runs Standard Doctrine; a max over it is a
///   forward STOP for 17b cuts M3/M4, reported, not asserted).
#[test]
#[allow(clippy::too_many_lines)]
fn m2_gate_a_on_a_mixed_pool() {
    let started = Instant::now();
    let world = support::booted_world();
    let unarmed = [
        support::tank(&world, Team::Red, 2),
        support::tank(&world, Team::Blue, 2),
    ];
    let reds = [
        Muster::mind(Team::Red, 1),
        Muster::mind_unarmed(Team::Red, 2),
    ];
    let blues = [
        Muster::mind(Team::Blue, 1),
        Muster::mind_unarmed(Team::Blue, 2),
    ];
    let cfg = tacsim::config::ArenaConfig::default();
    let mut thinks = 0_usize;
    let mut unarmed_thinks = 0_usize;
    let mut armed_thinks = 0_usize;
    let mut budget: Vec<u64> = Vec::new();
    let mut ops: Vec<u64> = Vec::new();
    let (mut hits, mut withdrew, mut rallied, mut disabled) = (0_u32, 0_u32, 0_u32, 0_u32);
    // `[unarmed, armed]` thinks at the `Disabled` edge, rallied, dwelling.
    let (mut at_disabled, mut at_rallied, mut at_dwell) = ([0_u32; 2], [0_u32; 2], [0_u32; 2]);
    for k in 0..SEEDS {
        let seed = SEED_0 + k;
        let twin = Arc::new(support::TwinRunner::new(&cfg));
        let played = support::play(&world, seed, &reds, &blues, twin.clone(), Some(MIXED_CAP_S));
        let thoughts = twin.thoughts();
        thinks += thoughts.len();
        budget.extend(thoughts.iter().map(|thought| thought.ops + thought.fee));
        ops.extend(thoughts.iter().map(|thought| thought.ops));
        let (mut seed_hits, mut seed_withdrew, mut seed_rallied, mut seed_disabled) =
            (0_u32, 0_u32, 0_u32, 0_u32);
        let mut armed_seen: Vec<ObjId> = Vec::new();
        for thought in &thoughts {
            let is_unarmed = unarmed.contains(&thought.unit);
            let kind = usize::from(!is_unarmed);
            at_disabled[kind] += u32::from(thought.state == ReflexState::Disabled.code());
            at_rallied[kind] += u32::from(thought.rallied);
            at_dwell[kind] += u32::from(thought.dwelling);
            // Gate A on every think of every hull (17b cut M2.6).
            assert!(
                !thought.faulted && !thought.diverged,
                "Gate A on #{} at seed {seed} think {}",
                thought.unit.0,
                thought.think_seq
            );
            if is_unarmed {
                unarmed_thinks += 1;
                assert!(
                    !thought.armed,
                    "the unarmed hull #{} at seed {seed} thinks with a weapon row",
                    thought.unit.0
                );
            } else {
                armed_thinks += 1;
                if !armed_seen.contains(&thought.unit) {
                    armed_seen.push(thought.unit);
                    assert!(
                        thought.armed,
                        "the armed hull #{} at seed {seed} first thinks with no weapon row",
                        thought.unit.0
                    );
                }
            }
        }
        for unit in unarmed {
            let mine: Vec<&support::TwinThought> = thoughts
                .iter()
                .filter(|thought| thought.unit == unit)
                .collect();
            seed_withdrew += u32::from(mine.iter().any(|thought| thought.state == 4));
            seed_rallied += u32::from(mine.iter().any(|thought| thought.rallied));
        }
        for event in &played.events {
            let (word, unit) = kind_and_unit(event);
            if word == "hit"
                && object_at(event, "target")
                    .is_some_and(|target| unarmed.iter().any(|hull| hull.0 == target))
            {
                seed_hits += 1;
            }
            if matches!(word.as_str(), "unit_disabled" | "unit_destroyed")
                && unit.is_some_and(|unit| unarmed.iter().any(|hull| hull.0 == unit))
            {
                seed_disabled += 1;
            }
        }
        println!(
            "m2_mixed_match: seed={seed} end={} t_ms={} thinks={} hits_on_unarmed={seed_hits} \
             unarmed_withdrew={seed_withdrew} unarmed_rallied={seed_rallied} \
             unarmed_disabled={seed_disabled} substeps={}",
            played
                .ended
                .map_or_else(|| "cap".to_owned(), |ended| end_word(ended.reason)),
            played.t_ms,
            thoughts.len(),
            played.hashes.len(),
        );
        hits += seed_hits;
        withdrew += seed_withdrew;
        rallied += seed_rallied;
        disabled += seed_disabled;
    }
    let (p50, p99, max) = support::spread(&budget);
    let (ops_p50, ops_p99, ops_max) = support::spread(&ops);
    println!(
        "m2_mixed_pool: thinks={thinks} unarmed_thinks={unarmed_thinks} unarmed_divergences=0 \
         armed_thinks={armed_thinks} armed_divergences=0 (every think asserted) \
         hits_on_unarmed={hits} unarmed_withdrew={withdrew} unarmed_rallied={rallied} \
         unarmed_disabled={disabled}"
    );
    println!(
        "m2_mixed_ops: ops_plus_fee_p50={p50} p99={p99} max={max} ops_p50={ops_p50} \
         ops_p99={ops_p99} ops_max={ops_max} (class I 1500, Class 0 400) over_400={} \
         wall_ms={}",
        budget.iter().filter(|spent| **spent > 400).count(),
        started.elapsed().as_millis()
    );
    println!(
        "m2_mixed_arms: unarmed_disabled_thinks={} unarmed_rallied_thinks={} \
         unarmed_dwell_thinks={} armed_disabled_thinks={} armed_rallied_thinks={} \
         armed_dwell_thinks={} (every one compared)",
        at_disabled[0], at_rallied[0], at_dwell[0], at_disabled[1], at_rallied[1], at_dwell[1]
    );
    assert!(
        unarmed_thinks > 0 && armed_thinks > 0,
        "both kinds of hull thought"
    );
    assert!(hits > 0, "no unarmed hull was hit");
    assert!(withdrew > 0, "no unarmed hull withdrew");
    assert!(
        rallied + disabled > 0,
        "no unarmed hull rallied or was disabled"
    );
    // An immobile unarmed hull thinks on at the `Disabled` edge (a wreck
    // does not): those thinks are compared, so the edge is Gate A's here.
    assert!(
        at_disabled[0] > 0,
        "no unarmed hull thought at the Disabled edge"
    );
    assert!(
        max <= 1_200,
        "a think of {max} ops and fees: past 1,200 is a STOP"
    );
}

/// **The pooled lane's record at 17b cut M1b.5** — every line
/// [`m1a6_t0_versus_t1_over_thirty_pooled_matches_is_inside_a_wilson_interval_containing_half`]
/// prints but the timing ([`pool_record`]), as it ran in release at 17b cut
/// M1b.5's HEAD and unmoved since (17b cut M2.2's report and M2.5's print
/// the same 67 lines). Gate A's first lane flies the twin where the lane
/// flies T1, and the twin answers T1's answer: its record IS this one, line
/// for line, or the twin is not T1.
const M1B5_POOLED_RECORD: [&str; 67] = [
    "twin_match: seed=2350 t1=blue end=elimination_red t_ms=189000",
    "twin_match: seed=2350 t1=red end=elimination_red t_ms=200250",
    "twin_match: seed=2351 t1=blue end=elimination_red t_ms=194250",
    "twin_match: seed=2351 t1=red end=elimination_blue t_ms=206500",
    "twin_match: seed=2352 t1=blue end=elimination_red t_ms=168750",
    "twin_match: seed=2352 t1=red end=elimination_red t_ms=173000",
    "twin_match: seed=2353 t1=blue end=elimination_red t_ms=169750",
    "twin_match: seed=2353 t1=red end=elimination_red t_ms=173000",
    "twin_match: seed=2354 t1=blue end=elimination_red t_ms=173500",
    "twin_match: seed=2354 t1=red end=elimination_blue t_ms=216750",
    "twin_match: seed=2355 t1=blue end=elimination_red t_ms=197250",
    "twin_match: seed=2355 t1=red end=elimination_red t_ms=183500",
    "twin_match: seed=2356 t1=blue end=elimination_red t_ms=172250",
    "twin_match: seed=2356 t1=red end=elimination_red t_ms=201000",
    "twin_match: seed=2357 t1=blue end=elimination_red t_ms=169500",
    "twin_match: seed=2357 t1=red end=elimination_red t_ms=194000",
    "twin_match: seed=2358 t1=blue end=elimination_red t_ms=221750",
    "twin_match: seed=2358 t1=red end=elimination_red t_ms=207000",
    "twin_match: seed=2359 t1=blue end=elimination_red t_ms=190250",
    "twin_match: seed=2359 t1=red end=elimination_red t_ms=199500",
    "twin_match: seed=2360 t1=blue end=elimination_blue t_ms=187500",
    "twin_match: seed=2360 t1=red end=elimination_red t_ms=179750",
    "twin_match: seed=2361 t1=blue end=elimination_red t_ms=196750",
    "twin_match: seed=2361 t1=red end=elimination_red t_ms=172750",
    "twin_match: seed=2362 t1=blue end=elimination_red t_ms=180750",
    "twin_match: seed=2362 t1=red end=elimination_red t_ms=171000",
    "twin_match: seed=2363 t1=blue end=elimination_blue t_ms=204250",
    "twin_match: seed=2363 t1=red end=elimination_red t_ms=214000",
    "twin_match: seed=2364 t1=blue end=elimination_red t_ms=198500",
    "twin_match: seed=2364 t1=red end=elimination_red t_ms=196750",
    "twin_control_match: seed=2350 run=0 end=elimination_red t_ms=182000",
    "twin_control_match: seed=2350 run=1 end=elimination_red t_ms=182000",
    "twin_control_match: seed=2351 run=0 end=elimination_red t_ms=198250",
    "twin_control_match: seed=2351 run=1 end=elimination_red t_ms=198250",
    "twin_control_match: seed=2352 run=0 end=elimination_blue t_ms=181500",
    "twin_control_match: seed=2352 run=1 end=elimination_blue t_ms=181500",
    "twin_control_match: seed=2353 run=0 end=elimination_red t_ms=210750",
    "twin_control_match: seed=2353 run=1 end=elimination_red t_ms=210750",
    "twin_control_match: seed=2354 run=0 end=elimination_red t_ms=161500",
    "twin_control_match: seed=2354 run=1 end=elimination_red t_ms=161500",
    "twin_control_match: seed=2355 run=0 end=elimination_draw t_ms=203250",
    "twin_control_match: seed=2355 run=1 end=elimination_draw t_ms=203250",
    "twin_control_match: seed=2356 run=0 end=elimination_red t_ms=172250",
    "twin_control_match: seed=2356 run=1 end=elimination_red t_ms=172250",
    "twin_control_match: seed=2357 run=0 end=elimination_red t_ms=184500",
    "twin_control_match: seed=2357 run=1 end=elimination_red t_ms=184500",
    "twin_control_match: seed=2358 run=0 end=elimination_red t_ms=189750",
    "twin_control_match: seed=2358 run=1 end=elimination_red t_ms=189750",
    "twin_control_match: seed=2359 run=0 end=elimination_red t_ms=217250",
    "twin_control_match: seed=2359 run=1 end=elimination_red t_ms=217250",
    "twin_control_match: seed=2360 run=0 end=elimination_red t_ms=194500",
    "twin_control_match: seed=2360 run=1 end=elimination_red t_ms=194500",
    "twin_control_match: seed=2361 run=0 end=elimination_red t_ms=187750",
    "twin_control_match: seed=2361 run=1 end=elimination_red t_ms=187750",
    "twin_control_match: seed=2362 run=0 end=elimination_red t_ms=178000",
    "twin_control_match: seed=2362 run=1 end=elimination_red t_ms=178000",
    "twin_control_match: seed=2363 run=0 end=elimination_red t_ms=180750",
    "twin_control_match: seed=2363 run=1 end=elimination_red t_ms=180750",
    "twin_control_match: seed=2364 run=0 end=elimination_red t_ms=183500",
    "twin_control_match: seed=2364 run=1 end=elimination_red t_ms=183500",
    "twin_pooled: n=30 t1_wins=15 t0_wins=15 draws=0 interval=[0.332, 0.668] decisive_interval=[0.332, 0.668] control_draws=2 control_red_wins=26 control_blue_wins=2",
    "twin_divergences: tiebreak=80 line=0 support=0 reads_dead=0 landing=10566 no_line=0 fan=11 order_hint=0 order_end=0 range_round=24 plot_cell=16 deg_round=0 fallback=2 cond_fire=2811 cond_stale=9 unwarranted=0 unlawful=0 floor_held=0 slot_starved=0 slot_parallel=449 first_divergence_ms=101377",
    "twin_divergence_basis: thinks=10566 ordered_thinks=0 (support, order_hint, order_end and deg_round read only under an order; the pool orders nothing)",
    "twin_gate_b: side=t1 shots=2898 shots_per_min=30.491 time_to_first_shot_ms=113627 first_shots=30 states_at_end=[startup=0,patrol=3,approach=0,engage=26,withdraw=0,disabled=31] wrecked=31 cond_shots=2811 parallel=449",
    "twin_gate_b: side=t0 shots=2374 shots_per_min=24.977 time_to_first_shot_ms=113377 first_shots=30 states_at_end=[startup=0,patrol=0,approach=0,engage=26,withdraw=10,disabled=24] wrecked=24 cond_shots=0 parallel=0",
    "twin_gate_b: side=control shots=4750 shots_per_min=25.217 time_to_first_shot_ms=112377 first_shots=60 states_at_end=[startup=0,patrol=0,approach=0,engage=60,withdraw=8,disabled=52] wrecked=46 cond_shots=0 parallel=0",
    "twin_gate_b_pool: path_queries=1135 corner_refusals=0 control_path_queries=1056 control_corner_refusals=0",
];

/// **Gate A over the pooled lane's own thirty matches** (17b cut M2.6; doc
/// 17 §17 "Gate A: T1 ≡ T2 exactly", §22's M2 gate; 17b §1 (iii)'s first
/// lane; 17b §5's feas F7 and adv F3): the pooled lane's matches EXACTLY as
/// it deals them — `(T0, Twin)` and `(Twin, T0)` per seed over the fifteen
/// seeds, beside its control pool of T0 on both sides — with every mind
/// thinking on an ASSERTING `support::TwinRunner`: every think of every mind
/// hull is T1 and T2 on one job, the Rhai fault refused first, then the
/// intents, then the memory bytes, the twin panicking with its printout on
/// the first that is not equal. The twin flies T1's answer, so:
///
/// * Gate A holds on the corpus the lane really plays — the armed stock
///   tanks, their slots armed and cited, every think compared (counted and
///   printed: `thinks_compared`);
/// * **the lane's printed record is [`M1B5_POOLED_RECORD`], line for line**
///   — the Wilson line, T1's shots and rate, `cond_fire`, `slot_parallel`
///   and the rest — the free proof that `Twin` is T1;
/// * the Rhai thinks' operations plus fees, p50/p99/max, printed against
///   class I's 1,500 (past 1,200 is a STOP — the bound 17b cut M2.5's armed
///   report held, which this lane replaces) and Class 0's 400 (a forward STOP
///   for 17b cuts M3/M4, reported).
///
/// Not ignored: its debug wall time is printed (`wall_ms`) and stays inside
/// the thirty seconds an ordinary run may spend on a lane (17b §1 (iii)).
#[test]
fn m2_gate_a_over_the_pooled_lanes_matches() {
    let started = Instant::now();
    let world = support::booted_world();
    let cfg = tacsim::config::ArenaConfig::default();
    let mut pool: Vec<Played> = Vec::new();
    let mut control: Vec<Played> = Vec::new();
    let mut thinks = 0_usize;
    let mut armed = 0_usize;
    let mut budget: Vec<u64> = Vec::new();
    for k in 0..SEEDS {
        let seed = SEED_0 + k;
        for (red, blue) in [(Flier::T0, Flier::Twin), (Flier::Twin, Flier::T0)] {
            let twin = Arc::new(support::TwinRunner::new(&cfg));
            pool.push(play_on(&world, seed, red, blue, Some(twin.clone())));
            let thoughts = twin.thoughts();
            for thought in &thoughts {
                assert!(
                    !thought.faulted && !thought.diverged,
                    "Gate A on #{} at seed {seed} think {}",
                    thought.unit.0,
                    thought.think_seq
                );
            }
            thinks += thoughts.len();
            armed += thoughts.iter().filter(|thought| thought.armed).count();
            budget.extend(thoughts.iter().map(|thought| thought.ops + thought.fee));
        }
        // The lane's control, as it plays it: one match counted for both
        // assignments.
        let twice = play(&world, seed, Flier::T0, Flier::T0);
        control.push(twice.clone());
        control.push(twice);
    }
    let record = pool_record(&pool, &control, Flier::Twin);
    let lines: Vec<&String> = record.head.iter().chain(record.tail.iter()).collect();
    for line in &lines {
        println!("{line}");
    }
    let (p50, p99, max) = support::spread(&budget);
    println!(
        "m2_gate_a_pooled: matches={} thinks_compared={thinks} armed_thinks={armed} \
         divergences=0 ops_plus_fee_p50={p50} p99={p99} max={max} (class I 1500, Class 0 400) \
         over_400={} wall_ms={}",
        pool.len(),
        budget.iter().filter(|spent| **spent > 400).count(),
        started.elapsed().as_millis()
    );
    if let Some(at) = (0..lines.len().max(M1B5_POOLED_RECORD.len()))
        .find(|at| lines.get(*at).map(|line| line.as_str()) != M1B5_POOLED_RECORD.get(*at).copied())
    {
        panic!(
            "the twin-flown lane's record parts from 17b cut M1b.5's at line {at}:\n  flown:    \
             {:?}\n  recorded: {:?}",
            lines.get(at),
            M1B5_POOLED_RECORD.get(at)
        );
    }
    assert_eq!(pool.len(), 30, "the pooled lane's thirty matches");
    // Every mind hull is mustered armed; a hull whose weapons a hit took off
    // thinks on with no weapon row, and those thinks are compared too.
    assert!(thinks > 0 && armed > 0, "the pool's mind hulls are armed");
    assert!(
        max <= 1_200,
        "a think of {max} ops and fees: past 1,200 is a STOP"
    );
}

/// **The extra lane: a twin against a twin** (17b cut M2.6; 17b §1 (iii)'s
/// coverage beside the pooled lane, §5's feas F7): two `Muster::mind` a side,
/// every mind of BOTH sides on one asserting `support::TwinRunner`, one match
/// a seed over the fifteen seeds — NOT swapped, since a twin on both sides
/// called the other way round is the same match — to the end
/// (`support::play`, one substep a pulse, `MatchConfig`'s cap). Every think
/// compared; the thinks counted and printed.
#[test]
fn m2_twin_against_twin_over_the_fifteen_seeds() {
    let started = Instant::now();
    let world = support::booted_world();
    let cfg = tacsim::config::ArenaConfig::default();
    let reds = [Muster::mind(Team::Red, 1), Muster::mind(Team::Red, 2)];
    let blues = [Muster::mind(Team::Blue, 1), Muster::mind(Team::Blue, 2)];
    let mut thinks = 0_usize;
    let mut budget: Vec<u64> = Vec::new();
    for k in 0..SEEDS {
        let seed = SEED_0 + k;
        let twin = Arc::new(support::TwinRunner::new(&cfg));
        let played = support::play(&world, seed, &reds, &blues, twin.clone(), None);
        let thoughts = twin.thoughts();
        assert_eq!(twin.divergences(), (0, 0), "seed {seed}");
        let reds_ids = [
            support::tank(&world, Team::Red, 1),
            support::tank(&world, Team::Red, 2),
        ];
        assert!(
            thoughts
                .iter()
                .any(|thought| reds_ids.contains(&thought.unit))
                && thoughts
                    .iter()
                    .any(|thought| !reds_ids.contains(&thought.unit)),
            "both sides thought on the twin at seed {seed}"
        );
        println!(
            "m2_twin_twin_match: seed={seed} end={} t_ms={} thinks={}",
            played
                .ended
                .map_or_else(|| "cap".to_owned(), |ended| end_word(ended.reason)),
            played.t_ms,
            thoughts.len()
        );
        thinks += thoughts.len();
        budget.extend(thoughts.iter().map(|thought| thought.ops + thought.fee));
    }
    let (p50, p99, max) = support::spread(&budget);
    println!(
        "m2_twin_twin: matches={SEEDS} thinks_compared={thinks} divergences=0 \
         ops_plus_fee_p50={p50} p99={p99} max={max} (class I 1500, Class 0 400) wall_ms={}",
        started.elapsed().as_millis()
    );
    assert!(thinks > 0);
    assert!(
        max <= 1_200,
        "a think of {max} ops and fees: past 1,200 is a STOP"
    );
}

/// The authored lance lane's arena seconds after the script starts
/// (`TACSIM_LANCE_ARENA_S`'s default).
const LANCE_ARENA_S: i64 = 120;

/// The authored lance lane's seeds: `SEED_0 + 0..3`.
const LANCE_SEEDS: u64 = 3;

/// **The authored lane's runner**: the asserting twin, and beside it the Rhai
/// Standard Doctrine with every `line_to` call counted — the shipped script
/// with its three `line_to(view, …)` sites routed through one more function
/// that writes a trace line first (`counted_line_to`), run on the same job
/// after the twin. The twin's answer is flown; the counting runner is held
/// to answer the same intents (it is the same script but for the trace), and
/// its trace lines are the think's `line_to` calls. Each think's view is
/// read for the figures the lane reports: its order's objective, its rows,
/// its notices.
#[derive(Debug)]
struct LanceProbe {
    twin: Arc<support::TwinRunner>,
    counted: tacsim::minds::RhaiRunner,
    seen: Mutex<Vec<LanceThought>>,
}

/// What [`LanceProbe`] read off one think.
#[derive(Clone, Debug)]
struct LanceThought {
    objective: Option<String>,
    rows: usize,
    /// The rows carrying odds (`STOCK_ODDS_ROWS` at most).
    odds_rows: usize,
    notices: usize,
    line_to: usize,
}

/// The shipped script with its `line_to` calls counted ([`LanceProbe`]).
fn counted_line_to_script() -> String {
    let source = tacsim::minds::STANDARD_RHAI;
    assert_eq!(
        source.matches("line_to(view, ").count(),
        3,
        "the stock script's three line_to sites"
    );
    format!(
        "{}\nfn counted_line_to(view, id) {{\n    trace(\"line_to\");\n    line_to(view, id)\n}}\n",
        source.replace("line_to(view, ", "counted_line_to(view, ")
    )
}

impl LanceProbe {
    fn new(cfg: &tacsim::config::ArenaConfig) -> LanceProbe {
        use sha2::{Digest as _, Sha256};
        let source = counted_line_to_script();
        let digest = mush_mind::Digest(Sha256::digest(source.as_bytes()).into());
        LanceProbe {
            twin: Arc::new(support::TwinRunner::new(cfg)),
            counted: tacsim::minds::RhaiRunner::new(cfg, &source, digest)
                .unwrap_or_else(|error| panic!("the counting script compiles: {error}")),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn seen(&self) -> Vec<LanceThought> {
        self.seen
            .lock()
            .map(|seen| seen.clone())
            .unwrap_or_default()
    }
}

impl MindRunner for LanceProbe {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        let answer = self.twin.think(job);
        let counted = self.counted.think(job);
        assert_eq!(
            (counted.fault.as_ref(), &counted.intents),
            (None, &answer.intents),
            "the counting script answers as the twin at #{} think {}",
            job.unit.0,
            job.think_seq
        );
        if let Ok(mut seen) = self.seen.lock() {
            seen.push(LanceThought {
                objective: job.view.order.as_ref().map(|order| {
                    let word = format!("{:?}", order.objective);
                    word.split([' ', '{', '('])
                        .next()
                        .unwrap_or_default()
                        .to_lowercase()
                }),
                rows: job.view.contacts.len(),
                odds_rows: job
                    .view
                    .contacts
                    .iter()
                    .filter(|row| !row.odds.is_empty())
                    .count(),
                notices: job.view.notices.len(),
                line_to: counted
                    .trace
                    .iter()
                    .filter(|line| *line == "line_to")
                    .count(),
            });
        }
        answer
    }
}

/// What one [`fly_lance`] came to.
struct LanceFlown {
    /// The script's start (`support::lance_start_ms`).
    start_ms: i64,
    /// The scripted orders and releases inside the run.
    orders: usize,
    releases: usize,
    /// The releases issued (every one, or the run ended early).
    releases_issued: usize,
    /// The hulls each scripted order and release reached while they were no
    /// wreck: the lines the arena must journal for them.
    ordered_living: usize,
    released_living: usize,
    /// The substeps played (one hash each, accumulated and asserted).
    substeps: usize,
    /// Every ledger line, flushed.
    events: Vec<LedgerEvent>,
    /// How the field ended, or `None` at [`LANCE_ARENA_S`].
    ended: Option<tacsim::arena::state::MatchEnd>,
}

/// **The authored lance lane's run** (17b cut M2.6's
/// [`m2_gate_a_over_the_authored_lance_field`] loop, lifted by 17b cut M2.7
/// so [`twin_rhai_timing`] flies the same field): `arena` (a
/// `support::lance_arena`, its minds on whatever runner the caller set)
/// stepped one substep a pulse for [`LANCE_ARENA_S`] arena seconds after the
/// script's start, the scripted orders and releases issued at their
/// millisecond through the arena's own order path (a release after the
/// orders of its millisecond, the harness's order).
fn fly_lance(arena: &mut Arena) -> LanceFlown {
    let start_ms = support::lance_start_ms(arena);
    let until_ms = start_ms + LANCE_ARENA_S * 1_000;
    let orders = support::lance_orders(&arena.squads, start_ms, until_ms);
    let releases = support::lance_releases(&arena.squads, start_ms, until_ms);
    let (mut next, mut next_release) = (0_usize, 0_usize);
    let mut seq = 0_u64;
    let (mut ordered_living, mut released_living) = (0_usize, 0_usize);
    let living = |arena: &Arena, intents: &[tacsim::arena::intents::Intent]| {
        intents
            .iter()
            .filter(|intent| {
                arena
                    .unit(intent.unit)
                    .is_some_and(|unit| !unit.limits.wreck)
            })
            .count()
    };
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut substeps = 0_usize;
    let mut wall = arena.now_ms;
    let ended = loop {
        let mut intents = Vec::new();
        while let Some((at, squad, objective, horizon)) = orders.get(next) {
            if *at > arena.now_ms {
                break;
            }
            let ordered = support::lance_intents(arena, *squad, *objective, *horizon, &mut seq);
            ordered_living += living(arena, &ordered);
            intents.extend(ordered);
            next += 1;
        }
        // A release after the orders of its millisecond (the harness's
        // order).
        while let Some((at, squad)) = releases.get(next_release) {
            if *at > arena.now_ms {
                break;
            }
            let released = support::lance_release_intents(arena, *squad, &mut seq);
            released_living += living(arena, &released);
            intents.extend(released);
            next_release += 1;
        }
        let out = arena.step(wall, &[], &intents);
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        hashes.extend(arena.substep_hashes.iter().copied());
        substeps += usize::try_from(out.substeps).unwrap_or(0);
        assert_eq!(hashes.len(), substeps, "one hash a substep played");
        if out.ended.is_some() {
            break out.ended;
        }
        if arena.now_ms >= until_ms {
            break None;
        }
        wall = arena.now_ms + arena.substep_len_ms();
    };
    let events = support::ledger_events(arena, &mut bytes);
    LanceFlown {
        start_ms,
        orders: orders.len(),
        releases: releases.len(),
        releases_issued: next_release,
        ordered_living,
        released_living,
        substeps: hashes.len(),
        events,
        ended,
    }
}

/// **Gate A over the authored lance field** (17b cut M2.6; doc 17 §17's
/// "30 seeded authored-cohort runs, armed and unarmed hulls both", §18; 17b
/// §1 (iii)'s third lane, §5's adv F3 and feas F8): the harness's lance lane
/// (`tests/harness.rs`' `measure_lance`) with every hull a mind —
/// `support::lance_arena`: twelve of the fielded catalog a side by the lane's
/// cohort rule (three line tanks, the laser, the biped, the thermal, the ECM
/// and the ECCM variants, cycling), red on column 102 heading east and blue
/// on 156 heading west in the rows' alternation, dealt into lances of four
/// by `squad_size` — on the shipped proving ground at three seeds (`SEED_0 +
/// 0..3`), stepped one substep a pulse for 120 arena seconds after the
/// script's start (`support::lance_start_ms`), under the lance lane's
/// scripted order table (`support::lance_orders`) and its release
/// (`support::lance_releases`) issued at each mark through the arena's own
/// order path (`support::lance_intents`, `support::lance_release_intents`:
/// an `IntentKind::Order` or `ReleaseOrder` a hull, at its epoch, the
/// lance-leader persona's). Every mind of both sides thinks on an ASSERTING
/// twin ([`LanceProbe`]).
///
/// **What the lane's 120 seconds reach of the table, stated**: its opening
/// round (`Engage` all round), its first minute (the first lance of each side
/// ordered `Hold`, the other two `Engage` again, a stagger apart), the
/// release of each side's first lance at the ninetieth second, and the
/// deadline of the third lance's opening order before its first-minute order
/// — as the harness's lane at its defaults. So the ordered thinks read
/// `Engage` and `Hold`, and the thinks past the script's start that read NO
/// order (released or expired: `ranges` off no obeyed order) are counted and
/// asserted. NOT reached, and so not asserted here: `Advance`, `Withdraw`,
/// `Screen` and `Support` (the table's later minutes), a mind's own early
/// ends and `ignore_until_ms` (which only those arms write), and the
/// supporter's leash — the in-source mirrors and `tests/minds.rs`' fixtures
/// under the twin hold them; a longer run is the lead's to rule.
///
/// What this lane carries that the pooled lane's two-tank order-free pool
/// cannot, each asserted: twelve-row views (`STOCK_CONTACT_ROWS`, the
/// contact rows' cap, reached and counted), the odds rows at their cap
/// (`STOCK_ODDS_ROWS`, counted), the notice buffer of a twelve-a-side fight,
/// and squads under standing orders — every scripted order and release
/// reconciled against the arena's own lines (one `ordered` line for each
/// hull ordered while it was no wreck, one `order_released` line for each
/// hull released while it was no wreck). Printed and asserted: the thinks
/// compared and the ORDERED thinks (the view's `order` present) with their
/// objectives, nonzero.
///
/// **Reported, not asserted**: the Rhai thinks' ops and fees p50/p99/max
/// against class I's 1,500 and Class 0's 400 (taken here: the twelve-row
/// views are where the cost is largest), with the count past 1,200; the
/// most `line_to` calls one think made (the cap is
/// `STOCK_LINE_TO_PER_THINK`, 2); the script's function count, the smallest
/// function expression depth it compiles at and its widest scope; the
/// lane's wall time. A think past 1,500 or a third `line_to` is a fault the
/// asserting twin refuses, so this lane's green already bounds both; the
/// 1,200 bound the two-tank lanes assert is NOT held here — at 17b cut M2.6
/// this lane's costliest think is 1,274 (273 thinks past 1,200; the
/// costliest are armed `Engage` thinks on nine- to twelve-row views, the
/// arming alone about 190 of it), named to the lead as the plan's STOP; the
/// bound this lane holds is the lead's ruling.
///
/// **Not ignored**: its debug wall time (printed, `wall_ms`; about fifteen
/// seconds on the blessing host) is inside the thirty seconds a lane may
/// spend in the ordinary sweep (17b §1 (iii)), so it runs in every
/// workspace run and needs no named `ci.yml` step.
#[test]
#[allow(clippy::too_many_lines)]
fn m2_gate_a_over_the_authored_lance_field() {
    let started = Instant::now();
    let world = support::booted_world();
    let mut thinks = 0_usize;
    let mut ordered_thinks = 0_usize;
    let mut ordered_lines = 0_usize;
    let (mut released_lines, mut expired_lines, mut unordered_thinks) = (0_usize, 0_usize, 0_usize);
    let mut objectives: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    let mut budget: Vec<u64> = Vec::new();
    let mut ops: Vec<u64> = Vec::new();
    let (mut rows_max, mut notices_max, mut line_to_max) = (0_usize, 0_usize, 0_usize);
    let (mut rows_capped, mut odds_capped) = (0_usize, 0_usize);
    let mut line_to_thinks = 0_usize;
    for k in 0..LANCE_SEEDS {
        let seed = SEED_0 + k;
        let mut arena = support::lance_arena(&world, seed);
        let lances: Vec<usize> = [Team::Red, Team::Blue]
            .iter()
            .map(|team| {
                arena
                    .squads
                    .iter()
                    .filter(|roster| roster.id.team == *team)
                    .count()
            })
            .collect();
        assert_eq!(lances, vec![3, 3], "three lances of four a side");
        assert!(
            arena.squads.iter().all(|roster| roster.units.len() == 4),
            "lances of four"
        );
        let probe = Arc::new(LanceProbe::new(&arena.arena_cfg));
        arena.mind_runner = probe.clone();
        let LanceFlown {
            start_ms,
            orders,
            releases,
            releases_issued,
            ordered_living,
            released_living,
            substeps,
            events,
            ended,
        } = fly_lance(&mut arena);
        let lines_of = |kind: &str| {
            events
                .iter()
                .filter(|event| kind_and_unit(event).0 == kind)
                .count()
        };
        let seed_ordered = lines_of("ordered");
        let seed_released = lines_of("order_released");
        let seed_expired = lines_of("order_expired");
        // Every scripted order reached every hull it was given while that
        // hull stood (none refused for a stale epoch or a channel); every
        // release likewise.
        assert_eq!(
            seed_ordered, ordered_living,
            "seed {seed}: one `ordered` line a living hull ordered"
        );
        assert_eq!(
            seed_released, released_living,
            "seed {seed}: one `order_released` line a living hull released"
        );
        assert_eq!(
            releases_issued, releases,
            "seed {seed}: every release issued"
        );
        let thoughts = probe.twin.thoughts();
        assert_eq!(probe.twin.divergences(), (0, 0), "seed {seed}");
        let seen = probe.seen();
        assert_eq!(seen.len(), thoughts.len());
        let seed_ordered_thinks = thoughts.iter().filter(|thought| thought.ordered).count();
        unordered_thinks += thoughts
            .iter()
            .filter(|thought| !thought.ordered && thought.t_ms > start_ms)
            .count();
        for one in &seen {
            if let Some(objective) = &one.objective {
                *objectives.entry(objective.clone()).or_default() += 1;
            }
            rows_max = rows_max.max(one.rows);
            rows_capped += usize::from(one.rows == tacsim::arena::mind::STOCK_CONTACT_ROWS);
            odds_capped += usize::from(one.odds_rows == tacsim::arena::STOCK_ODDS_ROWS);
            notices_max = notices_max.max(one.notices);
            line_to_max = line_to_max.max(one.line_to);
            line_to_thinks += usize::from(one.line_to > 0);
        }
        budget.extend(thoughts.iter().map(|thought| thought.ops + thought.fee));
        ops.extend(thoughts.iter().map(|thought| thought.ops));
        println!(
            "m2_lance_match: seed={seed} end={} t_ms={} thinks={} ordered_thinks={} \
             ordered_lines={seed_ordered} released_lines={seed_released} \
             expired_lines={seed_expired} orders_scripted={} releases_scripted={} substeps={}",
            ended.map_or_else(|| "cap".to_owned(), |ended| end_word(ended.reason)),
            arena.now_ms,
            thoughts.len(),
            seed_ordered_thinks,
            orders,
            releases,
            substeps
        );
        thinks += thoughts.len();
        ordered_thinks += seed_ordered_thinks;
        ordered_lines += seed_ordered;
        released_lines += seed_released;
        expired_lines += seed_expired;
    }
    let (p50, p99, max) = support::spread(&budget);
    let (ops_p50, ops_p99, ops_max) = support::spread(&ops);
    let (functions, depth, widest, width) = support::script_figures(tacsim::minds::STANDARD_RHAI);
    println!(
        "m2_gate_a_lance: seeds={LANCE_SEEDS} arena_s={LANCE_ARENA_S} thinks_compared={thinks} \
         ordered_thinks={ordered_thinks} ordered_lines={ordered_lines} \
         released_lines={released_lines} expired_lines={expired_lines} \
         unordered_thinks_after_start={unordered_thinks} divergences=0 \
         objectives={objectives:?} rows_max={rows_max} rows_capped={rows_capped} \
         odds_capped={odds_capped} notices_max={notices_max}"
    );
    println!(
        "m2_gate_a_lance_ops: ops_plus_fee_p50={p50} p99={p99} max={max} ops_p50={ops_p50} \
         ops_p99={ops_p99} ops_max={ops_max} (class I 1500, Class 0 400) over_400={} \
         over_1200={} line_to_max={line_to_max} line_to_thinks={line_to_thinks} \
         (cap {})",
        budget.iter().filter(|spent| **spent > 400).count(),
        budget.iter().filter(|spent| **spent > 1_200).count(),
        tacsim::minds::STOCK_LINE_TO_PER_THINK
    );
    println!(
        "m2_gate_a_lance_script: functions={functions} max_fn_expr_depth={depth} \
         widest_scope={width} ({widest}) wall_ms={}",
        started.elapsed().as_millis()
    );
    assert!(thinks > 0 && ordered_thinks > 0 && ordered_lines > 0);
    for objective in ["engage", "hold"] {
        assert!(
            objectives.get(objective).is_some_and(|count| *count > 0),
            "no think read a {objective} order: {objectives:?}"
        );
    }
    // The ends of an order the lane reaches: the release and the deadline,
    // and minds thinking on under neither.
    assert!(released_lines > 0, "no hull was released");
    assert!(expired_lines > 0, "no order ran out");
    assert!(
        unordered_thinks > 0,
        "no think past the start read no order"
    );
    // The caps this lane exists for, reached.
    assert_eq!(
        rows_max,
        tacsim::arena::mind::STOCK_CONTACT_ROWS,
        "the views reached the contact rows' cap"
    );
    assert!(
        rows_capped > 0 && odds_capped > 0,
        "the rows' and the odds rows' caps were reached"
    );
    // The binding limits (the limits are never raised).
    assert!(functions <= 32 && depth <= 24 && width <= 64);
}

/// **`Flier::T2` flies the Rhai Standard Doctrine** (17b cut M2.5): the
/// runner a T2 side is handed is the `RhaiRunner` over the shipped script,
/// and a twin side's is the twin — one runner an arena.
#[test]
fn m2_the_fliers_name_their_runners() {
    let cfg = tacsim::config::ArenaConfig::default();
    let name = |red: Flier, blue: Flier| format!("{:?}", runner_for(red, blue, &cfg));
    assert!(name(Flier::T0, Flier::T2).starts_with("RhaiRunner"));
    assert!(name(Flier::Twin, Flier::T0).starts_with("TwinRunner"));
    assert!(name(Flier::T1, Flier::T0).starts_with("NativeStockMind"));
    assert!(name(Flier::T2, Flier::T2).starts_with("RhaiRunner"));
}

// ---------------------------------------------------------------------------
// 17b cut M2.7: Gate A on whole-match hashes, and the figures (doc 17 §17,
// §18, §22's M2 gate)
// ---------------------------------------------------------------------------

/// **Gate A on whole-match hashes** (17b cut M2.7; doc 17 §22's M2 gate,
/// "Gate A … on whole-match hashes"; 17b §1 (iii)): each of the pooled
/// lane's thirty plays — `(T0, T1)` and `(T1, T0)` per seed over the fifteen
/// seeds, as the lane deals them — played ONCE with its mind side flown by
/// T1 (`NativeStockMind`) and ONCE by T2 ALONE (`Flier::T2`: the Rhai
/// Standard Doctrine, `tacsim::minds::standard_runner`, nothing native
/// beside it), each through `support::play` (one substep a pulse to the
/// end, every substep's hash accumulated after every step with its length
/// asserted). The two plays of each match are one match: equal substep hash
/// sequences (the length first, then the first substep that parts named),
/// equal ledger lines and ledger digests (the ledger's bytes), the same end
/// and the same final `state_hash`. And each T1 play's end is the one
/// [`M1B5_POOLED_RECORD`] recorded for that match.
///
/// Unlike [`m2_gate_a_over_the_pooled_lanes_matches`], whose twin flies T1's
/// answer, here T2's own answers are flown: a single op, fee or rounding
/// that parted a think would part the hash at its landing.
///
/// **`#[ignore]`d under 17b §1 (iii)'s thirty-second rule**: sixty debug
/// matches measured 30,160 ms run alone (`--test-threads=1`) on the blessing
/// host and about 39 s beside the other targets, past the thirty seconds an
/// ordinary run may spend on a lane; so it is run by the named step
/// `Whole-match mind hashes (17b cut M2.7; Gate A)` of `ci.yml`'s `test`
/// job on both hosts (the gated-test inventory covers it by that verbatim
/// name) and by hand with `cargo test -p tacsim --test twin
/// m2_whole_match_hashes_are_equal_between_the_runners_over_the_pool --
/// --ignored --exact --nocapture`. Its wall time is printed (`wall_ms`).
#[test]
#[ignore = "17b §1 (iii): over thirty debug seconds; run by ci.yml's named step on both hosts"]
fn m2_whole_match_hashes_are_equal_between_the_runners_over_the_pool() {
    let started = Instant::now();
    let world = support::booted_world();
    let cfg = tacsim::config::ArenaConfig::default();
    let side = |team: Team, flier: Flier| match flier {
        Flier::T0 => [Muster::reflex(team, 1), Muster::reflex(team, 2)],
        Flier::T1 | Flier::T2 | Flier::Twin => [Muster::mind(team, 1), Muster::mind(team, 2)],
    };
    let rhai = |flier: Flier| {
        if flier == Flier::T1 { Flier::T2 } else { flier }
    };
    let (mut matches, mut substeps, mut lines) = (0_usize, 0_usize, 0_usize);
    for k in 0..SEEDS {
        let seed = SEED_0 + k;
        for (at, (red, blue)) in [(Flier::T0, Flier::T1), (Flier::T1, Flier::T0)]
            .into_iter()
            .enumerate()
        {
            let (reds, blues) = (side(Team::Red, red), side(Team::Blue, blue));
            let t1 = support::play(
                &world,
                seed,
                &reds,
                &blues,
                runner_for(red, blue, &cfg),
                None,
            );
            let t2 = support::play(
                &world,
                seed,
                &reds,
                &blues,
                runner_for(rhai(red), rhai(blue), &cfg),
                None,
            );
            let named = format!("seed {seed} red {red:?} blue {blue:?}");
            assert_eq!(
                t2.hashes.len(),
                t1.hashes.len(),
                "{named}: the substeps played"
            );
            if let Some(step) =
                (0..t1.hashes.len()).find(|step| t2.hashes.get(*step) != t1.hashes.get(*step))
            {
                panic!(
                    "{named}: substep {step}'s hash parts — T2 {:?} against T1 {:?}",
                    t2.hashes.get(step),
                    t1.hashes.get(step)
                );
            }
            if let Some(line) = (0..t1.events.len().max(t2.events.len()))
                .find(|line| t2.events.get(*line) != t1.events.get(*line))
            {
                panic!(
                    "{named}: ledger line {line} parts — T2 {:?} against T1 {:?}",
                    t2.events.get(line),
                    t1.events.get(line)
                );
            }
            assert_eq!(
                t2.ledger_digest, t1.ledger_digest,
                "{named}: the ledger bytes"
            );
            assert_eq!(t2.ended, t1.ended, "{named}: the end");
            assert_eq!(t2.t_ms, t1.t_ms, "{named}: the stop");
            assert_eq!(
                t2.state_hash, t1.state_hash,
                "{named}: the final state hash"
            );
            let ended = t1
                .ended
                .unwrap_or_else(|| panic!("{named}: the match ends inside its cap"));
            let recorded = format!(
                "twin_match: seed={seed} t1={} end={} t_ms={}",
                if red == Flier::T1 { "red" } else { "blue" },
                end_word(ended.reason),
                ended.t_ms
            );
            let index = usize::try_from(2 * k).unwrap_or(usize::MAX) + at;
            assert_eq!(
                M1B5_POOLED_RECORD.get(index).copied(),
                Some(recorded.as_str()),
                "{named}: the pooled lane's record of this match"
            );
            matches += 1;
            substeps += t1.hashes.len();
            lines += t1.events.len();
        }
    }
    println!(
        "m2_whole_match: matches={matches} plays={} substeps_compared={substeps} \
         lines_compared={lines} divergences=0 build={} wall_ms={}",
        2 * matches,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        started.elapsed().as_millis()
    );
    assert_eq!(matches, 30, "the pooled lane's thirty matches");
}

/// **The timing wrapper of [`twin_rhai_timing`]**: around a lane's runner
/// (the twin, or the lance lane's probe), on every think it times, on the
/// job the runner is handed, what the Rhai runner does around the script —
/// the two `Arc` clones and the card's clone (`view_wrap`), the memory's
/// decode and encode through `mush_mind::memory` (and its round trip
/// through the script's map) — and, on the first [`TIMED_THINKS`] thinks it
/// sees, the same job once more through two recording Rhai runners: the
/// shipped script, and the shipped script with eight `trace` lines at the
/// head of `think` (the trace's cost). The inner runner's answer is flown,
/// untouched; the clock is the test's (doc 17 §23: measured from the test,
/// never in the library).
#[derive(Debug)]
struct Timed {
    inner: Arc<dyn MindRunner>,
    plain: tacsim::minds::RhaiRunner,
    traced: tacsim::minds::RhaiRunner,
    samples: Mutex<TimedSamples>,
}

/// What [`Timed`] measured, in nanoseconds.
#[derive(Clone, Debug, Default)]
struct TimedSamples {
    view_wrap_ns: Vec<u64>,
    mem_ns: Vec<u64>,
    mem_map_ns: Vec<u64>,
    mem_keys: Vec<usize>,
    traced_thinks: u32,
}

/// The shipped script with eight `trace` lines at the head of its `think`.
fn traced_script() -> String {
    let head = "fn think(view, card) {";
    assert_eq!(
        tacsim::minds::STANDARD_RHAI.matches(head).count(),
        1,
        "the stock script's one think"
    );
    let lines: String = (1..=8)
        .map(|n| format!("\n    trace(\"timing line {n}\");"))
        .collect();
    tacsim::minds::STANDARD_RHAI.replacen(head, &format!("{head}{lines}"), 1)
}

impl Timed {
    fn new(inner: Arc<dyn MindRunner>, cfg: &tacsim::config::ArenaConfig) -> Arc<Timed> {
        use sha2::{Digest as _, Sha256};
        let traced = traced_script();
        let digest = mush_mind::Digest(Sha256::digest(traced.as_bytes()).into());
        Arc::new(Timed {
            inner,
            plain: tacsim::minds::RhaiRunner::new(
                cfg,
                tacsim::minds::STANDARD_RHAI,
                tacsim::minds::standard_digest(),
            )
            .unwrap_or_else(|error| panic!("Standard Doctrine compiles: {error}"))
            .recording(),
            traced: tacsim::minds::RhaiRunner::new(cfg, &traced, digest)
                .unwrap_or_else(|error| panic!("the traced script compiles: {error}"))
                .recording(),
            samples: Mutex::new(TimedSamples::default()),
        })
    }

    fn samples(&self) -> TimedSamples {
        self.samples
            .lock()
            .map(|samples| samples.clone())
            .unwrap_or_default()
    }
}

/// Nanoseconds since `started`, saturated.
fn ns_since(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

impl MindRunner for Timed {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        // The runner's wraps (`runner.rs`' `run`): the view's and the map's
        // `Arc`s and the card's clone into its own.
        let started = Instant::now();
        let wrapped = (
            Arc::clone(job.view),
            Arc::clone(job.map),
            Arc::new(job.card.clone()),
        );
        let wrap_ns = ns_since(started);
        drop(std::hint::black_box(wrapped));
        // The memory through the one codec, and through the script's map.
        let mut mem = None;
        if !job.memory.is_empty() {
            let started = Instant::now();
            let decoded = mush_mind::memory::decode(job.memory);
            let bytes = decoded.as_ref().map(mush_mind::memory::encode);
            let mem_ns = ns_since(started);
            if let Some(decoded) = decoded {
                let started = Instant::now();
                let map = mush_mind::memory::to_map(&decoded);
                let back =
                    mush_mind::memory::from_map(&map, &tacsim::arena::mind::STOCK_MEMORY_LIMITS);
                let map_ns = ns_since(started);
                assert_eq!(back.ok().as_ref(), Some(&decoded), "the map round trip");
                assert_eq!(bytes.as_deref(), Some(job.memory), "the codec round trip");
                mem = Some((mem_ns, map_ns, decoded.len()));
            }
        }
        let trace_it = self
            .samples
            .lock()
            .is_ok_and(|samples| samples.traced_thinks < TIMED_THINKS);
        if trace_it {
            let plain = self.plain.think(job);
            let traced = self.traced.think(job);
            assert_eq!(
                (plain.fault.as_ref(), traced.fault.as_ref()),
                (None, None),
                "the timing runners do not fault"
            );
            assert_eq!(
                plain.intents, traced.intents,
                "a trace line changes nothing"
            );
            assert_eq!(traced.trace.len(), 8, "the eight lines traced");
        }
        if let Ok(mut samples) = self.samples.lock() {
            samples.view_wrap_ns.push(wrap_ns);
            if let Some((mem_ns, map_ns, keys)) = mem {
                samples.mem_ns.push(mem_ns);
                samples.mem_map_ns.push(map_ns);
                samples.mem_keys.push(keys);
            }
            samples.traced_thinks += u32::from(trace_it);
        }
        self.inner.think(job)
    }
}

/// `(p50, p99)` of `values` (`0`s for none).
fn p50_p99(values: &[u64]) -> (u64, u64) {
    let (p50, p99, _) = support::spread(values);
    (p50, p99)
}

/// A per-mille figure printed with one decimal (`24.3`).
fn tenths(per_mille: u64) -> String {
    format!("{}.{}", per_mille / 1_000, (per_mille % 1_000) / 100)
}

/// **One lane's figures** (doc 17 §18): its Rhai thinks' ops and fees, the
/// measured ops per microsecond, the Rhai and native `think_us`.
fn lane_figures(lane: &str, thoughts: &[support::TwinThought]) {
    let ops: Vec<u64> = thoughts.iter().map(|thought| thought.ops).collect();
    let fee: Vec<u64> = thoughts.iter().map(|thought| thought.fee).collect();
    let (p50, p99, max) = support::spread(&ops);
    println!(
        "twin_rhai_ops[{lane}]=p50:{p50} p99:{p99} max:{max} ceiling:{} class0:400 \
         thinks:{} over_400:{} (ops alone; ops plus fees is what the ceiling bounds)",
        tacsim::minds::STOCK_OPS_PER_THINK,
        thoughts.len(),
        ops.iter().filter(|ops| **ops > 400).count()
    );
    let budget: Vec<u64> = thoughts
        .iter()
        .map(|thought| thought.ops + thought.fee)
        .collect();
    let (fee_p50, fee_p99) = p50_p99(&fee);
    let (budget_p50, budget_p99, budget_max) = support::spread(&budget);
    println!(
        "twin_rhai_fee[{lane}]=p50:{fee_p50} p99:{fee_p99} ops_plus_fee=p50:{budget_p50} \
         p99:{budget_p99} max:{budget_max}"
    );
    // Ops per microsecond, per think, in thousandths (a think under one
    // microsecond reads nothing).
    let rate: Vec<u64> = thoughts
        .iter()
        .filter(|thought| thought.rhai_us > 0)
        .map(|thought| thought.ops * 1_000 / u64::try_from(thought.rhai_us).unwrap_or(u64::MAX))
        .collect();
    let (rate_p50, rate_p99) = p50_p99(&rate);
    // The slow tail: the first percentile of the rate.
    let mut sorted = rate.clone();
    sorted.sort_unstable();
    let rate_p1 = sorted.get(sorted.len() / 100).copied().unwrap_or_default();
    println!(
        "twin_rhai_ops_per_us[{lane}]=p50:{} p99:{} p1:{} (the whole Rhai think: the wraps, \
         the memory's decode and encode, the script — the first two a few per cent of it, \
         see twin_rhai_script_rate; doc 17 §7 and §18 measured 19.5-24.1 on the blessing \
         host's micro-benchmark, fees calibrated at 24; dev1's is unmeasured and may be \
         markedly lower: the op budget is deterministic everywhere, the CPU lens is wall \
         time on whatever host runs it)",
        tenths(rate_p50),
        tenths(rate_p99),
        tenths(rate_p1)
    );
    let rhai_us: Vec<u64> = thoughts
        .iter()
        .map(|thought| u64::try_from(thought.rhai_us).unwrap_or(u64::MAX))
        .collect();
    let native_us: Vec<u64> = thoughts
        .iter()
        .map(|thought| u64::try_from(thought.native_us).unwrap_or(u64::MAX))
        .collect();
    let (rhai_p50, rhai_p99) = p50_p99(&rhai_us);
    let (native_p50, native_p99) = p50_p99(&native_us);
    println!(
        "twin_rhai_think_us[{lane}]=rhai_p50:{rhai_p50} rhai_p99:{rhai_p99} \
         native_p50:{native_p50} native_p99:{native_p99} (the same thinks; 17b cut M1a.6 of \
         record: think_us 3/10, view_us 4/16)"
    );
}

/// **The figures of doc 17 §18, for the 17b §2 table** (17b cut M2.7;
/// report, never assert — beside [`m1a6_t0_versus_t1_over_thirty_pooled_matches_is_inside_a_wilson_interval_containing_half`]'s
/// `twin_timing`). Over the two Gate A lanes — lane 1, the pooled lane's
/// thirty matches flown `Twin` where it flies T1, and lane 3, the authored
/// lance field (twelve a side, the order table, where a think's cost is
/// largest) — every Rhai think's figures as the asserting twin recorded them
/// (`support::TwinThought`): `ops` p50/p99/max against class I's 1,500 and
/// Class 0's 400 (`twin_rhai_ops`), the fees (`twin_rhai_fee`), the
/// measured ops per microsecond (`twin_rhai_ops_per_us`, against doc 17
/// §7's twenty-four, with dev1's caveat), the Rhai `think_us` beside the
/// native's on the same thinks and beside 17b cut M1a.6's figures of record
/// (`twin_rhai_think_us`); then over both lanes through [`Timed`]: the
/// wraps (`twin_rhai_view_wrap_us`), the memory's codec
/// (`twin_rhai_mem_us`), the trace's cost (`twin_rhai_trace_cost`: a think
/// with eight `trace` lines against the same think with none); one cold
/// compile of the shipped script on a fresh sandbox and the cache's hit
/// (`twin_rhai_compile_us`); the script's figures and the most `line_to`
/// calls a lance think made (`twin_rhai_script`).
///
/// In a release build both lanes are flown whole; a debug build — whose
/// figures are not the ones of record — flies one pooled seed and one lance
/// seed, so the ordinary sweep spends seconds here, and prints
/// `build=debug`. **The figures of record are taken alone**, on a quiet
/// host: `cargo test --release -p tacsim --test twin twin_rhai_timing --
/// --exact --nocapture --test-threads=1` (beside the target's other lanes
/// the wall clock is shared and every `_us` figure reads high). The
/// script's own rate (`twin_rhai_script_rate`) is the think's p50 less the
/// wraps and the codecs: the gap to §7's twenty-four is the script's.
#[test]
#[allow(clippy::too_many_lines)]
fn twin_rhai_timing() {
    let started = Instant::now();
    let release = !cfg!(debug_assertions);
    let (pool_seeds, lance_seeds) = if release {
        (SEEDS, LANCE_SEEDS)
    } else {
        (1, 1)
    };
    let world = support::booted_world();
    let cfg = tacsim::config::ArenaConfig::default();
    let mut samples = TimedSamples::default();
    let mut absorb = |timed: &Timed| {
        let more = timed.samples();
        samples.view_wrap_ns.extend(more.view_wrap_ns);
        samples.mem_ns.extend(more.mem_ns);
        samples.mem_map_ns.extend(more.mem_map_ns);
        samples.mem_keys.extend(more.mem_keys);
        samples.traced_thinks += more.traced_thinks;
    };
    let mut plain: Vec<tacsim::minds::ThinkStat> = Vec::new();
    let mut traced: Vec<tacsim::minds::ThinkStat> = Vec::new();

    // Lane 1: the pooled lane's matches, the twin where it flies T1.
    let mut pooled: Vec<support::TwinThought> = Vec::new();
    for k in 0..pool_seeds {
        let seed = SEED_0 + k;
        for (red, blue) in [(Flier::T0, Flier::Twin), (Flier::Twin, Flier::T0)] {
            let twin = Arc::new(support::TwinRunner::new(&cfg));
            let timed = Timed::new(twin.clone(), &cfg);
            let side = |team: Team, flier: Flier| match flier {
                Flier::T0 => [Muster::reflex(team, 1), Muster::reflex(team, 2)],
                _ => [Muster::mind(team, 1), Muster::mind(team, 2)],
            };
            let _ = support::play(
                &world,
                seed,
                &side(Team::Red, red),
                &side(Team::Blue, blue),
                timed.clone(),
                None,
            );
            assert_eq!(twin.divergences(), (0, 0), "seed {seed}");
            pooled.extend(twin.thoughts());
            absorb(&timed);
            plain.extend(timed.plain.take_stats());
            traced.extend(timed.traced.take_stats());
        }
    }

    // Lane 3: the authored lance field.
    let mut lance: Vec<support::TwinThought> = Vec::new();
    let mut line_to_max = 0_usize;
    for k in 0..lance_seeds {
        let seed = SEED_0 + k;
        let mut arena = support::lance_arena(&world, seed);
        let probe = Arc::new(LanceProbe::new(&arena.arena_cfg));
        let timed = Timed::new(probe.clone(), &arena.arena_cfg);
        arena.mind_runner = timed.clone();
        let _ = fly_lance(&mut arena);
        assert_eq!(probe.twin.divergences(), (0, 0), "seed {seed}");
        lance.extend(probe.twin.thoughts());
        line_to_max = probe
            .seen()
            .iter()
            .map(|one| one.line_to)
            .fold(line_to_max, usize::max);
        absorb(&timed);
        plain.extend(timed.plain.take_stats());
        traced.extend(timed.traced.take_stats());
    }

    lane_figures("pooled", &pooled);
    lane_figures("lance", &lance);
    let both: Vec<support::TwinThought> = pooled.iter().chain(&lance).copied().collect();
    lane_figures("both", &both);

    let (wrap_p50, wrap_p99) = p50_p99(&samples.view_wrap_ns);
    println!(
        "twin_rhai_view_wrap_us=p50:{} p99:{} (ns p50:{wrap_p50} p99:{wrap_p99}; the view's \
         and the map's Arc clones and the card's clone, on the job, {} thinks)",
        wrap_p50 / 1_000,
        wrap_p99 / 1_000,
        samples.view_wrap_ns.len()
    );
    let (mem_p50, mem_p99) = p50_p99(&samples.mem_ns);
    let (map_p50, map_p99) = p50_p99(&samples.mem_map_ns);
    let keys: std::collections::BTreeSet<usize> = samples.mem_keys.iter().copied().collect();
    println!(
        "twin_rhai_mem_us=p50:{} p99:{} (ns p50:{mem_p50} p99:{mem_p99}; mush_mind::memory \
         decode plus encode of the stock memory, keys {keys:?}, {} thinks; the script's map \
         round trip to_map plus from_map ns p50:{map_p50} p99:{map_p99})",
        mem_p50 / 1_000,
        mem_p99 / 1_000,
        samples.mem_ns.len()
    );
    // The script's own rate: the Rhai think's p50 less the wraps and the
    // memory's two codecs (their p50s, measured above on the same jobs), so
    // the gap to §7's twenty-four is read as the script's and not as the
    // think's overhead.
    let think_ns: Vec<u64> = both
        .iter()
        .map(|thought| u64::try_from(thought.rhai_us).unwrap_or(u64::MAX) * 1_000)
        .collect();
    let both_ops: Vec<u64> = both.iter().map(|thought| thought.ops).collect();
    let (think_p50, _) = p50_p99(&think_ns);
    let (ops_p50, _) = p50_p99(&both_ops);
    let around_ns = wrap_p50 + mem_p50 + map_p50;
    let script_ns = think_p50.saturating_sub(around_ns).max(1);
    println!(
        "twin_rhai_script_rate=ops_per_us_p50:{} (ops p50 {ops_p50} over the think's p50 \
         {think_p50} ns less the wraps and the codecs' p50s {around_ns} ns, {} per mille of \
         the think; doc 17 §7's micro-benchmark 19.5-24.1, fees calibrated at 24)",
        tenths(ops_p50 * 1_000_000 / script_ns),
        around_ns * 1_000 / think_p50.max(1)
    );

    // The compile: one cold compile on a fresh sandbox, then the hit.
    let sandbox_started = Instant::now();
    let sandbox = mush_mind::Sandbox::new(
        &mush_mind::EngineLimits {
            max_ops_backstop: tacsim::minds::STOCK_OPS_PER_THINK,
        },
        tacsim::minds::register_api,
    );
    let sandbox_us = sandbox_started.elapsed().as_micros();
    // The digest (a SHA-256 of the bytes, the seal's key) is taken before the
    // clock starts: the cache is keyed by it, and the figures are the
    // compile's and the lookup's alone.
    let digest = tacsim::minds::standard_digest();
    let cold_started = Instant::now();
    let cold = sandbox.compile(
        tacsim::minds::STANDARD_RHAI,
        digest,
        tacsim::minds::MIND_ABI,
    );
    let cold_us = cold_started.elapsed().as_micros();
    let hit_started = Instant::now();
    let hit = sandbox.compile(
        tacsim::minds::STANDARD_RHAI,
        digest,
        tacsim::minds::MIND_ABI,
    );
    let hit_ns = hit_started.elapsed().as_nanos();
    assert!(
        matches!((&cold, &hit), (Ok(cold), Ok(hit)) if Arc::ptr_eq(cold, hit)),
        "the second compile is the cache's hit"
    );
    println!(
        "twin_rhai_compile_us=cold:{cold_us} hit:{} (hit ns {hit_ns}; a fresh sandbox's \
         Sandbox::new with register_api {sandbox_us} us; doc 17 §18: compile plus seal 270 us)",
        hit_ns / 1_000
    );

    // The trace's cost: the same jobs through the shipped script and through
    // it with eight trace lines.
    let ops_of = |stats: &[tacsim::minds::ThinkStat]| -> Vec<u64> {
        stats.iter().map(|stat| stat.ops).collect()
    };
    let us_of = |stats: &[tacsim::minds::ThinkStat]| -> Vec<u64> {
        stats
            .iter()
            .map(|stat| u64::try_from(stat.rhai_us).unwrap_or(u64::MAX))
            .collect()
    };
    let (plain_ops, _) = p50_p99(&ops_of(&plain));
    let (traced_ops, _) = p50_p99(&ops_of(&traced));
    let (plain_us_p50, plain_us_p99) = p50_p99(&us_of(&plain));
    let (traced_us_p50, traced_us_p99) = p50_p99(&us_of(&traced));
    let extra_ops: Vec<u64> = traced
        .iter()
        .zip(&plain)
        .map(|(traced, plain)| traced.ops.saturating_sub(plain.ops))
        .collect();
    let extra: std::collections::BTreeSet<u64> = extra_ops.iter().copied().collect();
    println!(
        "twin_rhai_trace_cost=thinks:{} ops_none_p50:{plain_ops} ops_eight_p50:{traced_ops} \
         extra_ops:{extra:?} us_none:p50:{plain_us_p50} p99:{plain_us_p99} \
         us_eight:p50:{traced_us_p50} p99:{traced_us_p99}",
        plain.len()
    );

    let (functions, depth, widest, width) = support::script_figures(tacsim::minds::STANDARD_RHAI);
    println!(
        "twin_rhai_script=functions:{functions} max_fn_expr_depth:{depth} \
         widest_scope:{width} ({widest}) line_to_max:{line_to_max} (cap {})",
        tacsim::minds::STOCK_LINE_TO_PER_THINK
    );
    println!(
        "twin_rhai_timing: build={} pooled_matches={} lance_seeds={lance_seeds} \
         pooled_thinks={} lance_thinks={} wall_ms={}",
        if release { "release" } else { "debug" },
        2 * pool_seeds,
        pooled.len(),
        lance.len(),
        started.elapsed().as_millis()
    );
    assert!(!pooled.is_empty() && !lance.is_empty() && !plain.is_empty());
}

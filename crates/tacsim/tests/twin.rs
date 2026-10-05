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
/// the inner runner's, untouched.
#[derive(Debug)]
struct Recording {
    inner: NativeStockMind,
    thoughts: Mutex<Vec<Thought>>,
}

impl Recording {
    fn new(cfg: &ReflexConfig) -> Arc<Recording> {
        Arc::new(Recording {
            inner: NativeStockMind::with(cfg),
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

/// Who flies a side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flier {
    T0,
    T1,
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
            Some(Flier::T1) if by.as_deref() == Some("cond") => {
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
        if flier_of(state.id.0) == Some(Flier::T1) {
            out.slot_parallel += unserialised(&shots_of(&lines, state.id.0), decide_ms);
        }
    }
    out
}

/// **One match**: `red` and `blue` fly two tanks each on the proving ground at
/// `seed`, one substep a pulse to the end, with T1's thinks shadowed.
fn play(world: &mush_testkit::TestWorld, seed: u64, red: Flier, blue: Flier) -> Played {
    let muster = |team: Team, slot: u8, flier: Flier| match flier {
        Flier::T0 => Muster::reflex(team, slot),
        Flier::T1 => Muster::mind(team, slot),
    };
    let reds = [muster(Team::Red, 1, red), muster(Team::Red, 2, red)];
    let blues = [muster(Team::Blue, 1, blue), muster(Team::Blue, 2, blue)];
    let mut arena = support::shipped_arena_seeded(world, &[&reds, &blues], seed);
    let recording = Recording::new(&arena.arena_cfg.reflex);
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
    let ended = loop {
        let out = arena.step(wall, &[], &[]);
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
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
                Flier::T1 => arena
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
    let n = u32::try_from(pool.len()).unwrap_or(u32::MAX);
    let t1_wins = u32::try_from(
        pool.iter()
            .filter(|played| played.winner == Some(Flier::T1))
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
    for played in &pool {
        let t1 = if played.red == Flier::T1 {
            "red"
        } else {
            "blue"
        };
        println!(
            "twin_match: seed={} t1={t1} end={} t_ms={}",
            played.seed,
            end_word(played.end),
            played.t_ms
        );
    }
    for (at, played) in control.iter().enumerate() {
        println!(
            "twin_control_match: seed={} run={} end={} t_ms={}",
            played.seed,
            at % 2,
            end_word(played.end),
            played.t_ms
        );
    }
    println!(
        "twin_pooled: n={n} t1_wins={t1_wins} t0_wins={t0_wins} draws={draws} \
         interval=[{:.3}, {:.3}] decisive_interval=[{:.3}, {:.3}] control_draws={control_draws} \
         control_red_wins={control_red_wins} control_blue_wins={control_blue_wins}",
        interval.0, interval.1, decisive.0, decisive.1
    );
    let mut divergences = Divergences::default();
    for played in &pool {
        divergences.absorb(&played.divergences);
    }
    println!(
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
    );
    let mut control_divergences = Divergences::default();
    for played in &control {
        control_divergences.absorb(&played.divergences);
    }
    // What the four order counters were read over: a nought there is a
    // measured nought only over thinks that had an order to read.
    println!(
        "twin_divergence_basis: thinks={} ordered_thinks={} (support, order_hint, order_end and \
         deg_round read only under an order; the pool orders nothing)",
        divergences.landing, divergences.ordered_thinks
    );
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
    for flier in [Flier::T1, Flier::T0] {
        let reports: Vec<&SideReport> = pool
            .iter()
            .flat_map(|played| played.sides.iter())
            .filter(|(side, _)| *side == flier)
            .map(|(_, report)| report)
            .collect();
        let label = if flier == Flier::T1 { "t1" } else { "t0" };
        println!("{}", gate_b_line(label, &reports));
    }
    let reports: Vec<&SideReport> = control
        .iter()
        .flat_map(|played| played.sides.iter())
        .map(|(_, report)| report)
        .collect();
    println!("{}", gate_b_line("control", &reports));
    println!(
        "twin_gate_b_pool: path_queries={} corner_refusals={} control_path_queries={} \
         control_corner_refusals={}",
        pool.iter().map(|p| p.path_queries).sum::<u64>(),
        pool.iter().map(|p| p.corner_refusals).sum::<u64>(),
        control.iter().map(|p| p.path_queries).sum::<u64>(),
        control.iter().map(|p| p.corner_refusals).sum::<u64>(),
    );
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
    let recording = Recording::new(&arena.arena_cfg.reflex);
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

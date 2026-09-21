//! **The arena task, on a real runtime** (16b cut T.2, T1; doc 16 §3.1 "The seam
//! is seven messages", "The arena core is a library", "Failure").
//!
//! Four proofs, each driving the task the way the world half does — a check-out,
//! an inbox, an egress and a stop signal — and nothing else:
//!
//! - **(a)** a two-unit match takes a `Startup` intent and a heading, publishes
//!   frames and ledger chunks, is acknowledged, is asked to stop, and reports
//!   [`ArenaOut::Ended`];
//! - **(b)** a panic injected into `step` ends the task `Void` and **nothing the
//!   faulted pulse produced leaves**;
//! - **(c)** a pulse past `tempo.stall_ms` is voided by the watchdog;
//! - **(d)** the stop signal ends the task **silently** — no result, no frames,
//!   no chunks;
//! - **(e)** (16b cut D.1) [`ArenaOut::Frames`] carries the arena's clock and is
//!   sent when there is a frame **or** when the arena second has moved, so a
//!   match nobody is moving in still publishes its clock — once an arena second,
//!   never twice.
//!
//! # Seeds
//!
//! Each test uses its own match seed, because the two fault-injection hooks in
//! `arena/step.rs` are armed by seed so that two tests running beside each other
//! in this binary cannot arm each other's arena. The seeds are listed in [`SEEDS`]
//! so a fifth test cannot quietly reuse one.

use std::collections::BTreeMap;
use std::time::Duration;

use mush_core::{DefinitionDigest, DefinitionId, DefinitionKey, DefinitionKind, ObjId, OwnerId};
use mush_sim::data::Pose;
use mush_tactics::{
    CellSpec, DefinitionRef, FacetId, FacetSpec, RulesValue, SectionId, SectionSpec, TacticalUnit,
    UnitSpec,
};
use smol_str::SmolStr;
use tacsim::arena::intents::{Intent, IntentKind};
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena::state::{
    Arena, ArenaDefinition, ArenaTerrain, Channel, CustodyOut, CustodyUnit, EndReason, EntryZone,
    Pilot, StopReason, Team, TerrainCell, TrackedRules,
};
use tacsim::arena_task::{ArenaHandle, ArenaMsg, ArenaOut, MatchId, spawn_arena};
use tacsim::config::{ArenaConfig, MatchConfig, SensorsConfig};
use tokio::sync::mpsc;

/// Every seed this file arms an arena with, and which proof owns it. The two
/// hooks in `arena/step.rs` key on the seed, so a duplicate here would let one
/// test's fault land in another's match.
/// **16b cut D.1**: proof (e) arms no hook, so its matches are numbered out of
/// the way rather than listed here — `2_360 + period_ms` for the idle clock and
/// `2_371` for the frame that outranks it.
const SEEDS: [(u64, &str); 4] = [
    (2_350, "the round trip"),
    (2_351, "the injected panic"),
    (2_352, "the stall"),
    (2_353, "the stop signal"),
];

// ---------------------------------------------------------------------------
// Fixtures — the same shapes `tests/arena.rs` builds, for the same reason: the
// crate's `arena::test_support` is `pub(crate)` and invisible here.
// ---------------------------------------------------------------------------

/// The window: 16 by 16 cells of 1,000 cm.
const SIDE: u16 = 16;
/// The cell size, centimetres.
const CELL_CM: u16 = 1_000;
/// The two units.
const RED: ObjId = ObjId(11);
const BLUE: ObjId = ObjId(12);
/// Their pilots.
const RED_PILOT: ObjId = ObjId(101);
const BLUE_PILOT: ObjId = ObjId(102);

fn dref(kind: &str, id: &str) -> DefinitionRef {
    DefinitionRef::new(
        DefinitionKey::new(
            OwnerId::new("tacsim").expect("a canonical owner"),
            DefinitionKind::new(kind).expect("a canonical kind"),
            DefinitionId::new(id).expect("a canonical id"),
        ),
        1,
        DefinitionDigest::from_bytes([0_u8; 32]),
    )
    .expect("version 1 is not zero")
}

fn rules() -> RulesValue {
    RulesValue::new(dref("schema", "tacsim.arena.task"), serde_json::json!({}))
        .expect("an empty rules blob")
}

fn tactical_unit(id: ObjId) -> TacticalUnit {
    let spec = UnitSpec {
        definition: dref("unit", "tacsim.unit.line_tank"),
        sections: vec![SectionSpec {
            id: SectionId::new("hull").expect("a canonical section"),
            condition: mush_tactics::ConditionId::new("intact").expect("a condition"),
            facets: vec![FacetSpec {
                id: FacetId::new("front").expect("a canonical facet"),
                cells: vec![CellSpec {
                    id: mush_tactics::CellId::new("armor").expect("a cell"),
                    min: 0,
                    max: 10,
                    init: 10,
                }],
            }],
            slots: Vec::new(),
            cells: vec![CellSpec {
                id: mush_tactics::CellId::new("internals").expect("a cell"),
                min: 0,
                max: 8,
                init: 8,
            }],
        }],
        unit_cells: Vec::new(),
        components: Vec::new(),
        capabilities: Vec::new(),
        mobility: None,
        rules: rules(),
    };
    TacticalUnit::from_spec(&spec, id).expect("a one-section unit")
}

fn ground() -> TerrainCell {
    TerrainCell {
        class: mush_atlas::TerrainClass::Ground,
        elev_cm: 0,
        move_cost: 100,
    }
}

fn centre(x: u16, y: u16) -> Pose {
    Pose::at(
        i64::from(x) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
        i64::from(y) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
        0,
        rules(),
    )
}

fn definition() -> ArenaDefinition {
    ArenaDefinition {
        id: SmolStr::new("tacsim.arena.task"),
        terrain: ArenaTerrain::uniform(SIDE, SIDE, CELL_CM, ground()).expect("the test window"),
        entry_zones: vec![
            EntryZone {
                team: Team::Red,
                x0: 0,
                y0: 0,
                x1: 1,
                y1: 15,
            },
            EntryZone {
                team: Team::Blue,
                x0: 14,
                y0: 0,
                x1: 15,
                y1: 15,
            },
        ],
        // 16b cut N.1: no authored furniture on this fixture window — the
        // reflex controller is not what these tests are about, and every unit
        // on them is a `Pilot`.
        key_spots: Vec::new(),
        waypoints: tacsim::arena::state::Waypoints::default(),
    }
}

fn tank(id: ObjId, team: Team, pose: Pose) -> CustodyUnit {
    CustodyUnit {
        id,
        team,
        pose,
        mobility: TrackedRules {
            max_speed_cms: 1_000,
            accel_cms2: 200,
            brake_cms2: 400,
            turn_rate_mdeg_s: 30_000,
            reverse_factor_pct: 40,
        },
        sensor_height_cm: 150,
        sensor: None,
        weapons: Vec::new(),
        profile: tacsim::arena::resolution::UnitProfile::default(),
        gunnery: 0,
        controller: tacsim::arena::state::Controller::Pilot,
        // 16b cut Q.1: no pod behind it.
        slot: 0,
        unit: tactical_unit(id),
    }
}

/// Two tanks, two seated pilots, one window, one seed.
fn custody(seed: u64) -> CustodyOut {
    CustodyOut {
        definition: definition(),
        units: vec![
            tank(RED, Team::Red, centre(1, 4)),
            tank(BLUE, Team::Blue, centre(14, 4)),
        ],
        pilots: vec![
            Pilot {
                unit: RED,
                pilot: RED_PILOT,
                station: SmolStr::new("cockpit"),
                epoch: 1,
            },
            Pilot {
                unit: BLUE,
                pilot: BLUE_PILOT,
                station: SmolStr::new("cockpit"),
                epoch: 1,
            },
        ],
        resolution: tacsim::match_::stock_resolution(),
        seed,
    }
}

/// A fast, chatty arena, so a test sees several pulses and several **closed**
/// chunks inside a second of wall time rather than inside an hour.
///
/// Two knobs and no more: a 50 ms period at every tempo, and a 5 ms
/// `position_summary` cadence. The chunk size (16 KiB) is the library's and is
/// not configurable, so the only honest way to close one in a test is to write
/// enough events — a summary every 5 ms of arena time over two units is about
/// ten summaries per 50 ms pulse, which fills a chunk in well under a second.
fn fast_arena() -> ArenaConfig {
    let mut cfg = ArenaConfig::default();
    cfg.tempo.quiet_ms = 50;
    cfg.tempo.alert_ms = 50;
    cfg.tempo.engaged_ms = 50;
    cfg.ledger.summary_ms = 5;
    cfg
}

/// Start one match on the current runtime.
fn start(seed: u64, arena_cfg: &ArenaConfig) -> (ArenaHandle, mpsc::Receiver<ArenaOut>) {
    let (egress_tx, egress_rx) = mpsc::channel(256);
    let (handle, _join) = spawn_arena(
        &tokio::runtime::Handle::current(),
        MatchId(seed),
        custody(seed),
        arena_cfg,
        &MatchConfig::default(),
        &SensorsConfig::default(),
        egress_tx,
    )
    .expect("the test check-out is accepted");
    (handle, egress_rx)
}

/// Every message the arena publishes for `how_long`, or until it ends.
async fn collect(rx: &mut mpsc::Receiver<ArenaOut>, how_long: Duration) -> Vec<ArenaOut> {
    let deadline = tokio::time::Instant::now() + how_long;
    let mut out = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return out;
        }
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Some(message)) => {
                let ended = matches!(message, ArenaOut::Ended { .. });
                out.push(message);
                if ended {
                    return out;
                }
            }
            Ok(None) | Err(_) => return out,
        }
    }
}

/// Every ledger event the collected chunks carry, in order.
fn events(messages: &[ArenaOut]) -> Vec<LedgerEvent> {
    let mut bytes: Vec<u8> = Vec::new();
    for message in messages {
        if let ArenaOut::Ledger { chunk, .. } = message {
            bytes.extend_from_slice(&chunk.bytes);
        }
    }
    String::from_utf8(bytes)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<LedgerEvent>(line).ok())
        .collect()
}

/// The end the arena reported, if it reported one.
fn ended(messages: &[ArenaOut]) -> Option<EndReason> {
    messages.iter().rev().find_map(|message| match message {
        ArenaOut::Ended { end, .. } => Some(end.reason),
        _ => None,
    })
}

// ---------------------------------------------------------------------------
// (a) The round trip
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_match_takes_orders_publishes_frames_and_chunks_and_reports_its_end() {
    let seed = SEEDS[0].0;
    let arena_cfg = fast_arena();
    let (handle, mut rx) = start(seed, &arena_cfg);

    // Two orders in one burst, at the epoch the check-out established.
    handle
        .offer(ArenaMsg::Intent(Intent {
            unit: RED,
            channel: Channel::Helm,
            epoch: 1,
            kind: IntentKind::Startup,
            seq: 1,
        }))
        .expect("the inbox takes a startup");
    handle
        .offer(ArenaMsg::Intent(Intent {
            unit: RED,
            channel: Channel::Helm,
            epoch: 1,
            kind: IntentKind::Heading { mdeg: 90_000 },
            seq: 2,
        }))
        .expect("the inbox takes a heading");

    // Long enough for the 10,000 ms startup to be scheduled, several pulses to
    // run, and the ledger to close at least one chunk of position summaries.
    let mut messages = collect(&mut rx, Duration::from_millis(2_500)).await;

    let frames: Vec<&ArenaOut> = messages
        .iter()
        .filter(|message| matches!(message, ArenaOut::Frames { .. }))
        .collect();
    assert!(
        !frames.is_empty(),
        "the pilots' views changed and the world was told: {messages:#?}"
    );
    let pilots: Vec<ObjId> = messages
        .iter()
        .filter_map(|message| match message {
            ArenaOut::Frames { frames, .. } => Some(frames.iter().map(|(pilot, _)| *pilot)),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(
        pilots.contains(&RED_PILOT) && pilots.contains(&BLUE_PILOT),
        "both seated pilots were addressed: {pilots:?}"
    );
    // The frame is the unit's own, never the other side's.
    for message in &messages {
        if let ArenaOut::Frames { frames, .. } = message {
            for (pilot, frame) in frames {
                let expected = if *pilot == RED_PILOT { RED } else { BLUE };
                assert_eq!(frame.unit, expected, "a pilot's frame is its own unit's");
            }
        }
    }

    let chunks: Vec<u64> = messages
        .iter()
        .filter_map(|message| match message {
            ArenaOut::Ledger { chunk, .. } => Some(chunk.seq),
            _ => None,
        })
        .collect();
    assert!(
        !chunks.is_empty(),
        "at least one 16 KiB chunk closed and left the arena: {chunks:?}"
    );
    assert_eq!(
        chunks,
        (1..=chunks.len() as u64).collect::<Vec<u64>>(),
        "chunks arrive in sequence, from 1, with no gap"
    );
    // And the black box reads: the match opened and the startup was accepted.
    let log = events(&messages);
    assert!(
        log.iter()
            .any(|event| matches!(event.kind, EventKind::MatchStarted)),
        "the ledger's first event is the match opening"
    );
    assert!(
        log.iter()
            .any(|event| matches!(event.kind, EventKind::Startup { unit, .. } if unit == RED)),
        "and the startup the pilot ordered is in it: {:?}",
        log.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>()
    );

    // Acknowledge every chunk, the way the world's ledger lane does.
    let highest = chunks.iter().copied().max().unwrap_or(0);
    handle
        .offer(ArenaMsg::LedgerAck {
            upto_chunk_seq: highest,
        })
        .expect("the inbox takes an acknowledgement");

    // Then abort. Honoured at the next pulse's phase 1, which is one 50 ms
    // period away.
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");
    let tail = collect(&mut rx, Duration::from_millis(1_000)).await;
    assert_eq!(
        ended(&tail),
        Some(EndReason::Abort),
        "the abort is the match's reported end: {tail:#?}"
    );
    // The `aborted` event is in the black box, and the tail left with the match.
    messages.extend(tail);
    let log = events(&messages);
    assert!(
        log.iter()
            .any(|event| matches!(event.kind, EventKind::Aborted)),
        "the abort is journalled: {:?}",
        log.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>()
    );
    // Nothing more is published, and the task has ended.
    assert!(
        collect(&mut rx, Duration::from_millis(300))
            .await
            .is_empty(),
        "an ended match publishes nothing further"
    );
}

// ---------------------------------------------------------------------------
// (b) The injected panic
// ---------------------------------------------------------------------------

/// A panic inside `step` ends the task with `Void`, and nothing the faulted pulse
/// produced leaves.
///
/// The arm is built here rather than through the task, because the hook is armed
/// on an [`Arena`] and the task owns its own: the same seed is what connects them,
/// and this test's seed is its alone (see [`SEEDS`]).
#[tokio::test(flavor = "multi_thread")]
async fn a_panic_in_step_voids_the_match_and_publishes_nothing_from_that_pulse() {
    let seed = SEEDS[1].0;
    let arena_cfg = fast_arena();
    // Arm by seed: a throwaway arena over the same custody, whose only purpose is
    // to hold the seed the hook keys on.
    let mut armed = Arena::new(
        custody(seed),
        &arena_cfg,
        &MatchConfig::default(),
        &SensorsConfig::default(),
    )
    .expect("the arming arena is accepted");
    armed.panic_on_next_step_for_tests();
    drop(armed);

    let (handle, mut rx) = start(seed, &arena_cfg);
    let messages = collect(&mut rx, Duration::from_millis(1_500)).await;

    assert_eq!(
        ended(&messages),
        Some(EndReason::Void),
        "a panicking pulse voids the match: {messages:#?}"
    );
    // The very first pulse is the one that faulted, so there was nothing before
    // it: no chunk (not even `match_started`, which only leaves on a flush) and
    // no frame reached the world.
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, ArenaOut::Ledger { .. })),
        "nothing the faulted pulse produced leaves: {messages:#?}"
    );
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, ArenaOut::Frames { .. })),
        "and no frame either: {messages:#?}"
    );
    assert_eq!(messages.len(), 1, "the `Ended` is the only message");
    // The world keeps its handle and the task is gone.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!handle.is_live(), "the task ended rather than looping");
}

// ---------------------------------------------------------------------------
// (c) The stall
// ---------------------------------------------------------------------------

/// A pulse past `tempo.stall_ms` is voided by the watchdog, after it returns —
/// the arena has no preemption.
#[tokio::test(flavor = "multi_thread")]
async fn a_pulse_past_the_stall_bound_voids_the_match() {
    let seed = SEEDS[2].0;
    let mut arena_cfg = fast_arena();
    arena_cfg.tempo.stall_ms = 30;
    let mut armed = Arena::new(
        custody(seed),
        &arena_cfg,
        &MatchConfig::default(),
        &SensorsConfig::default(),
    )
    .expect("the arming arena is accepted");
    armed.stall_next_step_for_tests(Duration::from_millis(200));
    drop(armed);

    let (_handle, mut rx) = start(seed, &arena_cfg);
    let messages = collect(&mut rx, Duration::from_millis(2_000)).await;

    assert_eq!(
        ended(&messages),
        Some(EndReason::Void),
        "a 200 ms pulse under a 30 ms bound voids the match: {messages:#?}"
    );
    // The stalled pulse itself completed, so its own record is publishable and
    // the stopping pulse journalled the `void`.
    let log = events(&messages);
    assert!(
        log.is_empty()
            || log
                .iter()
                .any(|event| matches!(event.kind, EventKind::Void)),
        "whatever left carries the void, never a clean end: {:?}",
        log.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// (d) The stop signal
// ---------------------------------------------------------------------------

/// The shutdown tail's lever ends the task **silently**: a process that is
/// stopping is not a match that has a result.
#[tokio::test(flavor = "multi_thread")]
async fn the_stop_signal_ends_the_task_without_a_result() {
    let seed = SEEDS[3].0;
    let arena_cfg = fast_arena();
    let (handle, mut rx) = start(seed, &arena_cfg);

    // Let it run, so there is something it could have reported.
    let running = collect(&mut rx, Duration::from_millis(300)).await;
    assert!(
        !running.is_empty(),
        "the match was running before it was stopped"
    );
    assert!(ended(&running).is_none(), "and had not ended: {running:#?}");

    handle.signal_stop();
    let tail = collect(&mut rx, Duration::from_millis(500)).await;
    assert!(
        ended(&tail).is_none(),
        "the stop signal reports no result at all: {tail:#?}"
    );
    assert!(!handle.is_live(), "and the task is gone");
}

// ---------------------------------------------------------------------------
// The seeds themselves
// ---------------------------------------------------------------------------

/// Two proofs cannot share a seed, or one's injected fault lands in the other's
/// match.
#[test]
fn every_proof_has_its_own_match_seed() {
    let mut seen: BTreeMap<u64, &str> = BTreeMap::new();
    for (seed, owner) in SEEDS {
        assert!(
            seed != 0,
            "a seed of 0 disarms the hooks; {owner} needs a real one"
        );
        assert!(
            seen.insert(seed, owner).is_none(),
            "seed {seed} is claimed twice: {owner} and {:?}",
            seen.get(&seed)
        );
    }
}

// ---------------------------------------------------------------------------
// (e) The clock on `Frames` (16b cut D.1, `B-29`)
// ---------------------------------------------------------------------------

/// The same check-out with **nobody seated**, so the arena produces no frame at
/// all and the only thing that can leave on [`ArenaOut::Frames`] is the clock.
fn unseated_custody(seed: u64) -> CustodyOut {
    CustodyOut {
        pilots: Vec::new(),
        ..custody(seed)
    }
}

/// One match with nobody in it, on the current runtime.
fn start_unseated(
    seed: u64,
    arena_cfg: &ArenaConfig,
    sensors_cfg: &SensorsConfig,
) -> (ArenaHandle, mpsc::Receiver<ArenaOut>) {
    let (egress_tx, egress_rx) = mpsc::channel(256);
    let (handle, _join) = spawn_arena(
        &tokio::runtime::Handle::current(),
        MatchId(seed),
        unseated_custody(seed),
        arena_cfg,
        &MatchConfig::default(),
        sensors_cfg,
        egress_tx,
    )
    .expect("the test check-out is accepted");
    (handle, egress_rx)
}

/// A sensors block whose pass cadence is past any window a test here opens: the
/// pass is a due event for **every** unit, staggered by object id, so on the
/// defaults it — and not the tempo — is what decides the pulse rate at `Quiet`.
fn no_passes() -> SensorsConfig {
    SensorsConfig {
        pass_ms: 600_000,
        ..SensorsConfig::default()
    }
}

/// An arena whose period is `period_ms` whatever tempo it picks, which is how a
/// test pins the cadence without having to make two tanks shoot at each other.
///
/// The two recurring due events are pushed past any window a test here opens, so
/// that `next_due_ms` really is the period: the `position_summary` cadence and
/// the reflex controller's `decide_ms`, which on this fixture's ids falls every
/// 1,000 ms and would otherwise be what decides the pulse rate at `Quiet`.
fn arena_at(period_ms: u32) -> ArenaConfig {
    let mut cfg = ArenaConfig::default();
    cfg.tempo.quiet_ms = period_ms;
    cfg.tempo.alert_ms = period_ms;
    cfg.tempo.engaged_ms = period_ms;
    cfg.ledger.summary_ms = 600_000;
    cfg.reflex.decide_ms = 600_000;
    cfg
}

/// Every `Frames` message's `(frame count, now_ms)`, in order.
fn clocks(messages: &[ArenaOut]) -> Vec<(usize, i64)> {
    messages
        .iter()
        .filter_map(|message| match message {
            ArenaOut::Frames { frames, now_ms, .. } => Some((frames.len(), *now_ms)),
            _ => None,
        })
        .collect()
}

/// **`B-29`: an empty `Frames` carries the arena's clock, and is sent when the
/// arena SECOND has moved** (16b cut D.1).
///
/// **This pins the clause, not the shipped cadence.** The clause is only visible
/// where a pulse is shorter than a second, and on the shipped config no pulse is:
/// the per-unit sensor pass and the reflex decision are both due every 1,000 ms
/// and `next_due_ms` is the earlier of a due event and the period, so the shipped
/// pulse rate is about 1,000 ms at all three tempos. [`no_passes`] and
/// [`arena_at`] push those events out past the window so that the period really
/// is the pulse rate, which is the only way to make a 500 ms pulse and watch the
/// clause suppress every other one. The gap between two messages is what is
/// pinned:
///
/// * a 500 ms period: the second moves every OTHER pulse, so the gap is
///   1,000 ms and not 500 — one message an arena second, never two in one;
/// * a 1,000 ms and a 5,000 ms period: the second has moved at every pulse, so
///   the gap is the period and it is one message a pulse.
///
/// What the shipped config does is pinned by
/// [`d1_the_shipped_config_publishes_its_clock_about_once_an_arena_second`].
///
/// The clock runs on tokio's paused timer, so the arena's own wall reference
/// moves in whole periods and nothing here sleeps.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn d1_an_idle_match_publishes_its_clock_once_an_arena_second() {
    for (period_ms, gap_ms) in [(500_u32, 1_000_i64), (1_000, 1_000), (5_000, 5_000)] {
        // A seed of its own per cadence, and none of the four fault seeds.
        let seed = 2_360 + u64::from(period_ms);
        let (_handle, mut rx) = start_unseated(seed, &arena_at(period_ms), &no_passes());
        let messages = collect(&mut rx, Duration::from_millis(10_050)).await;
        let clocks = clocks(&messages);
        assert!(
            clocks.len() >= 3,
            "a {period_ms} ms match published no clock at all: {clocks:?}"
        );
        for (frames, _) in &clocks {
            assert_eq!(
                *frames, 0,
                "a match nobody is seated in produced a frame: {clocks:?}"
            );
        }
        let mut seconds: Vec<i64> = clocks.iter().map(|(_, now_ms)| now_ms / 1_000).collect();
        let published = seconds.len();
        seconds.dedup();
        assert_eq!(
            seconds.len(),
            published,
            "two clocks in one arena second at {period_ms} ms: {clocks:?}"
        );
        // The gap, to within the wall reference's own wobble: the arena's first
        // pulse establishes the reference and the ones after it land on whole
        // periods measured from it, so a boundary can sit a few milliseconds
        // either side of the round number.
        for pair in clocks.windows(2) {
            let [(_, first), (_, next)] = pair else {
                continue;
            };
            assert!(
                (next - first - gap_ms).abs() <= 50,
                "the clock's cadence at a {period_ms} ms period was {} and not {gap_ms}: \
                 {clocks:?}",
                next - first
            );
        }
    }
}

/// **What the SHIPPED config does with the clock clause** (16b cut D.1, `B-29`).
///
/// The test above pins the clause by pushing the recurring due events out of the
/// way; this one changes nothing at all. On the defaults the per-unit sensor pass
/// (`sensors.pass_ms`, 1,000 ms) and the reflex decision (`reflex.decide_ms`,
/// 1,000 ms) are both due more often than `tempo.quiet_ms`, and `next_due_ms` is
/// the earlier of a due event and the period — so the pulse rate is about a
/// second whatever tempo the arena picks, and the board's staleness bound is one
/// arena second and not one tempo period.
///
/// That is the figure the seam is sized by and the figure
/// [`tacsim::arena_task::ArenaOut::Frames`]' own doc states, so it is pinned on
/// the config the game ships rather than on one no server runs.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn d1_the_shipped_config_publishes_its_clock_about_once_an_arena_second() {
    let (_handle, mut rx) =
        start_unseated(2_377, &ArenaConfig::default(), &SensorsConfig::default());
    let messages = collect(&mut rx, Duration::from_millis(10_050)).await;
    let clocks = clocks(&messages);
    assert!(
        clocks.len() >= 5,
        "the shipped config published almost no clock in ten seconds: {clocks:?}"
    );
    for (frames, _) in &clocks {
        assert_eq!(
            *frames, 0,
            "a match nobody is seated in produced a frame: {clocks:?}"
        );
    }
    // Never twice in one arena second — the clause, doing its one job.
    let mut seconds: Vec<i64> = clocks.iter().map(|(_, now_ms)| now_ms / 1_000).collect();
    let published = seconds.len();
    seconds.dedup();
    assert_eq!(
        seconds.len(),
        published,
        "two clocks in one arena second on the shipped config: {clocks:?}"
    );
    // And never a second and a bit apart, which is the staleness bound: a 5,000
    // ms gap here would mean the pulse rate really was `tempo.quiet_ms` and the
    // seam's own doc was over-stating the board's freshness by five times.
    for pair in clocks.windows(2) {
        let [(_, first), (_, next)] = pair else {
            continue;
        };
        assert!(
            next - first <= 1_050,
            "the shipped board went {} ms without a clock: {clocks:?}",
            next - first
        );
    }
}

/// **A pulse with a frame in it is sent whatever the second says** (16b cut
/// D.1), which is the other half of the rule: the clause only ever suppresses an
/// EMPTY message.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn d1_a_pulse_that_carries_a_frame_is_never_suppressed_by_the_clock() {
    // The seated match, at the same 500 ms period the test above showed
    // suppressing every other message.
    let seed = 2_371;
    let (handle, mut rx) = start(seed, &arena_at(500));
    let order = |seq: u64, kind: IntentKind| {
        ArenaMsg::Intent(Intent {
            unit: RED,
            channel: Channel::Helm,
            epoch: 1,
            kind,
            seq,
        })
    };
    handle
        .offer(order(1, IntentKind::Startup))
        .expect("the inbox takes the startup");
    // Past `match.startup_ms`, so the tank is awake and can be told to move.
    let warming = collect(&mut rx, Duration::from_millis(11_050)).await;
    handle
        .offer(order(2, IntentKind::Heading { mdeg: 90_000 }))
        .expect("the inbox takes the heading");
    handle
        .offer(order(3, IntentKind::Speed { cms: 800 }))
        .expect("the inbox takes the speed");
    let moving = collect(&mut rx, Duration::from_millis(3_050)).await;
    let warm_clocks = clocks(&warming);
    let clocks = clocks(&moving);
    assert!(
        clocks.iter().any(|(frames, _)| *frames > 0),
        "the moving platform's pilot was never sent a frame: {clocks:?}"
    );
    // Two messages half a second apart can only exist because the later one
    // carried a frame: the clock clause alone would have suppressed it.
    assert!(
        clocks
            .windows(2)
            .any(|pair| matches!(pair, [(_, first), (frames, next)]
                if *next - *first < 1_000 && *frames > 0)),
        "every message was an arena second apart, so the frame branch never \
         published: {clocks:?}"
    );
    // And the clock never goes backwards, across both windows.
    let all: Vec<(usize, i64)> = warm_clocks.into_iter().chain(clocks).collect();
    assert!(
        all.windows(2).all(|pair| match pair {
            [(_, first), (_, next)] => next >= first,
            _ => true,
        }),
        "the arena's clock went backwards: {all:?}"
    );
}

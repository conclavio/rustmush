//! **The arena task, on a real runtime** (16b cut T.2, T1; doc 16 §3.1 "The seam
//! is seven messages", "The arena core is a library", "Failure").
//!
//! Six proofs, each driving the task the way the world half does — a check-out,
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
//!   never twice;
//! - **(f)** (16b cut AA.3, 16c `B-79`) a stall on the pulse that **ended** the
//!   match voids it too: the result is `Void` with no unit returned, and the
//!   black box keeps the end the arena journalled — `match_ended` for a timeout,
//!   `aborted` for an abort — and no `void` event; while the same terminal pulse
//!   inside the bound keeps its reason and returns both tanks;
//! - **(g)** (16b cut M.1) the seam under backpressure: an end waits for every
//!   chunk and the held frame and reports the digest the chunks reproduce, and
//!   the stop signal releases it; a frame the world could not take is held and
//!   merged, at most sixty-four held notices a pilot and a frame the world can
//!   take never cut; no control is dropped and a displaced
//!   release still runs; a burst is bounded per unit with the newest refused;
//!   the inbox keeps room for a control, a stop and an ack; and a stop that
//!   meets an `Ended` in flight is `Busy` where a task that died is `Gone`;
//! - **(h)** (16b cut AF.2) the snapshot: one on the opening pulse and one a
//!   period of arena time, each decoding to the arena that was encoded, behind
//!   every chunk closed before it, the one of the pulse before an end reaching
//!   the world before the end, none on a pulse that ended the match or stalled,
//!   and none with the rule off. Every fixture above
//!   turns the rule off (`without_snapshots`), so their streams are the ones
//!   they were written against.
//!
//! # Seeds
//!
//! Each test uses its own match seed, because the two fault-injection hooks in
//! `arena/step.rs` fire only for the arena whose seed they were armed with, so
//! one test's FAULT cannot land in another's match. The seeds are listed in
//! [`SEEDS`] so a new test cannot quietly reuse one.
//!
//! Distinct seeds do **not** stop one test's ARMING overwriting another's: each
//! hook stores its seed in one global slot, and arming replaces whatever seed is
//! there. The proofs that arm the stall hook therefore hold [`STALL_HOOK`]
//! from before they arm until their match has ended (16b cut AA.3).

use std::collections::BTreeMap;
use std::time::Duration;

use mush_core::{DefinitionDigest, DefinitionId, DefinitionKey, DefinitionKind, ObjId, OwnerId};
use mush_sim::data::Pose;
use mush_tactics::{
    CellSpec, DefinitionRef, FacetId, FacetSpec, RulesValue, SectionId, SectionSpec, TacticalUnit,
    UnitSpec,
};
use sha2::{Digest as _, Sha256};
use smol_str::SmolStr;
use tacsim::arena::input_tape::{TapeStatus, TapeWrite};
use tacsim::arena::intents::{Control, ControlState, Intent, IntentKind, Refusal};
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena::perception::FrameNotice;
use tacsim::arena::state::{
    Arena, ArenaDefinition, ArenaTerrain, Channel, CustodyOut, CustodyUnit, EndReason, EntryZone,
    PartyId, Pilot, Power, StopReason, TerrainCell, TrackedRules,
};
use tacsim::arena_bridge::{ArenaTasks, ControlRefusal};
use tacsim::arena_task::{ArenaHandle, ArenaMsg, ArenaOut, MatchId, resume_arena, spawn_arena};
use tacsim::config::{ArenaConfig, MatchConfig, SensorsConfig};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Every seed this file arms an arena with, and which proof owns it. The two
/// hooks in `arena/step.rs` key on the seed, so a duplicate here would let one
/// test's fault land in another's match. (A distinct seed does not stop one
/// test's arming overwriting another's — that is [`STALL_HOOK`]'s job.)
/// **16b cut D.1**: proof (e) arms no hook, so its matches are numbered out of
/// the way rather than listed here — `2_360 + period_ms` for the idle clock and
/// `2_371` for the frame that outranks it, and `2_377` for the shipped config's
/// clock. **16b cut AA.3** follows the same convention for its hook-free
/// control: `2_381`, the terminal pulse inside the bound.
///
/// **16b cut M.1** adds its own rows (`2_356` onwards). None of its proofs arms
/// a hook, but a row each keeps one match per proof and the rule in one place.
///
/// **16b cut M.2** adds `2_398` and `2_399` for the same reason, **16b cut
/// U.1** `2_400` and `2_401`, **16b cut AF.2** `2_402` to `2_407`, and **16b
/// cut AF.3** `2_408`, and **17b cut M1b.1** `2_409`, and **17b cut M1b.5**
/// `2_410` and `2_411`.
const SEEDS: [(u64, &str); 33] = [
    (2_350, "the round trip"),
    (2_351, "the injected panic"),
    (2_352, "the stall"),
    (2_353, "the stop signal"),
    (2_354, "the stalled terminal pulse"),
    (2_355, "the stalled abort"),
    (
        2_356,
        "M.1: an end that waits for its chunks and its held frame",
    ),
    (2_357, "M.1: a stop while the end waits"),
    (2_358, "M.1: frames held and merged across a stall"),
    (2_359, "M.1: sixty-four held notices a pilot"),
    (2_390, "M.1: seventeen controls and a release in one window"),
    (2_391, "M.1: a displaced release still runs"),
    (2_392, "M.1: a burst bounded per unit, the newest refused"),
    (2_393, "M.1: the inbox reserve"),
    (2_394, "M.1: the Ended in flight"),
    (2_395, "M.1: the task that died without a result"),
    (2_397, "M.1: a frame the world can take is never cut"),
    (
        2_398,
        "M.2: a pilot's STARTUP through the task, 50 ms periods",
    ),
    (
        2_399,
        "M.2: a pilot's STARTUP through the task, 250 ms periods",
    ),
    (2_400, "U.1: a two-tank match wakes on the substep lattice"),
    (2_401, "U.1: a late wake does not carry its lateness"),
    (
        2_402,
        "AF.2: a snapshot on the first pulse and every five seconds",
    ),
    (
        2_403,
        "AF.2: two chunks then the snapshot on a one-message egress",
    ),
    (2_404, "AF.2: no snapshot with the rule off"),
    (
        2_405,
        "AF.2: the snapshot before the end reaches the world first",
    ),
    (2_406, "AF.2: no snapshot on a pulse that ended the match"),
    (2_407, "AF.2: no snapshot on a stalled pulse"),
    (
        2_408,
        "AF.3: a resumed task's first pulse and first snapshot",
    ),
    (
        2_409,
        "M1b.1: the floor captured at check-out survives a resume",
    ),
    (
        2_410,
        "M1b.5: a card change held to the reserve and refused when stopped",
    ),
    (2_411, "M1b.5: two card changes land in arrival order"),
    (
        2_412,
        "M3.1: the class table captured at check-out, stock at a resume",
    ),
    (
        2_413,
        "M3.5: each scripted seal's runner installed at spawn and resume",
    ),
];

/// **16b cut AA.3**: the stall hook is one global seed slot, and arming it
/// overwrites whatever seed is there — so every proof that arms it holds
/// this from before it arms until its `collect` returns. Tokio's mutex,
/// because the guard lives across `.await`.
static STALL_HOOK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    // 17b cut M3.1 (plan F6): a Class I core in the hull, through
    // `tacsim::definitions::core::core_component` — the shape `bind` lays
    // down from content — so the hull holds two condition slots and a mind
    // on it thinks at Class I's figures, as it did before cores existed.
    let (core, capability) = tacsim::definitions::core::core_component(
        "hull",
        tacsim::arena::core::CoreClass::I,
        dref("component", "tacsim.unit.line_tank.core"),
        dref("schema", tacsim::definitions::unit::CORE_RULES_SCHEMA),
    )
    .expect("a core");
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
        components: vec![core],
        capabilities: vec![capability],
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
                team: PartyId::RED,
                x0: 0,
                y0: 0,
                x1: 1,
                y1: 15,
            },
            EntryZone {
                team: PartyId::BLUE,
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
        modes: Vec::new(),
        homes: Vec::new(),
    }
}

fn tank(id: ObjId, team: PartyId, pose: Pose) -> CustodyUnit {
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
        seal: tacsim::arena::MindSeal::placeholder(),
    }
    .sealed()
}

/// Two tanks, two seated pilots, one window, one seed.
fn custody(seed: u64) -> CustodyOut {
    CustodyOut {
        definition: definition(),
        units: vec![
            tank(RED, PartyId::RED, centre(1, 4)),
            tank(BLUE, PartyId::BLUE, centre(14, 4)),
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
        mode: tacsim::arena::MatchMode::Skirmish,
        host: tacsim::arena::FieldHost::all_hostile(),
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
///
/// **16b cut AF.2**: and no snapshots ([`without_snapshots`]), like every
/// fixture of this file that predates them.
fn fast_arena() -> ArenaConfig {
    let mut cfg = without_snapshots(ArenaConfig::default());
    cfg.tempo.quiet_ms = 50;
    cfg.tempo.alert_ms = 50;
    cfg.tempo.engaged_ms = 50;
    cfg.ledger.summary_ms = 5;
    cfg
}

/// **The think pool every task here runs on** (17b cut M4.1): the process's
/// one, at the stock two workers — the gate fixtures that run tasks at one,
/// two and four workers build their own (`MindPool::new`).
fn pool() -> std::sync::Arc<tacsim::minds::MindPool> {
    tacsim::minds::MindPool::shared(2)
}

/// **`cfg` with the snapshot rule off** (16b cut AF.2): `snapshot_ms = 0`.
///
/// Every proof in this file that predates cut AF.2 counts, indexes or orders
/// the egress's messages — the four capacity-1 backpressure proofs of 16b cut
/// M.1, the index-based `clocks`, `frame_in` and `frames_for`, every U.1 count —
/// and an [`ArenaOut::Snapshot`] on the opening pulse and every 5,000 ms would
/// move all of them for a reason none of them is about. So every pre-existing
/// fixture turns the rule off and those streams stay byte-identical; only the
/// AF.2 proofs at the end of this file turn it on.
fn without_snapshots(mut cfg: ArenaConfig) -> ArenaConfig {
    cfg.snapshot_ms = 0;
    cfg
}

/// Start one match on the current runtime, on the default match rules.
fn start(seed: u64, arena_cfg: &ArenaConfig) -> (ArenaHandle, mpsc::Receiver<ArenaOut>) {
    start_with(seed, arena_cfg, &MatchConfig::default())
}

/// Start one match on the current runtime, on the match rules given.
///
/// **16b cut AA.3**: [`spawn_arena`] does not validate the `MatchConfig` (the
/// 60,000 ms floor on `max_duration_ms` is `MatchConfig::validate`'s, run by the
/// server's config load), which is what lets a test end a match on its opening
/// pulse with `max_duration_ms: 0`.
fn start_with(
    seed: u64,
    arena_cfg: &ArenaConfig,
    match_cfg: &MatchConfig,
) -> (ArenaHandle, mpsc::Receiver<ArenaOut>) {
    let (handle, _join, egress_rx) = start_with_egress(seed, arena_cfg, match_cfg, 256);
    (handle, egress_rx)
}

/// **16b cut M.1**: [`start_with`], on an egress of `capacity` messages, handing
/// back the task's join as well — the backpressure proofs need an egress the
/// world is not draining, and the stop proofs need to see the task end.
fn start_with_egress(
    seed: u64,
    arena_cfg: &ArenaConfig,
    match_cfg: &MatchConfig,
    capacity: usize,
) -> (ArenaHandle, JoinHandle<()>, mpsc::Receiver<ArenaOut>) {
    let (egress_tx, egress_rx) = mpsc::channel(capacity);
    let (handle, join, _callsigns) = spawn_arena(
        &tokio::runtime::Handle::current(),
        MatchId(seed),
        custody(seed),
        arena_cfg,
        match_cfg,
        &SensorsConfig::default(),
        &tacsim::config::MindsConfig::default(),
        &std::collections::BTreeMap::new(),
        pool(),
        egress_tx,
    )
    .expect("the test check-out is accepted");
    (handle, join, egress_rx)
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
    // 16b cut M.1 (`B-48`): what the task's arena has appended by its first
    // step — `match_started`, written by `Arena::new` into the open chunk from
    // the same custody — and never shipped.
    let appended = armed.ledger.digest();
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
    assert!(
        matches!(
            messages.as_slice(),
            [
                ArenaOut::Tape { write: TapeWrite::Open(_), .. },
                ArenaOut::Tape { write: TapeWrite::Mark(mark), .. },
                ArenaOut::Ended { .. }
            ] if matches!(mark.status, TapeStatus::Incomplete { .. })
        ),
        "only the pre-pulse recording header, explicit incomplete marker and result leave: \
         {messages:#?}"
    );
    // 16b cut M.1 (`B-48`): the reported digest is over what LEFT the ledger —
    // nothing here, so the digest of no bytes — and not over the open chunk the
    // faulted pulse never flushed, which `log:match.<id>` could never
    // reproduce.
    let reported = messages.iter().find_map(|message| match message {
        ArenaOut::Ended { digest, .. } => Some(*digest),
        _ => None,
    });
    let nothing: [u8; 32] = Sha256::digest(b"").into();
    assert_eq!(
        reported,
        Some(nothing),
        "Ended.digest is SHA-256 over the chunks that left, of which there are none"
    );
    assert_ne!(
        reported,
        Some(appended),
        "Ended.digest covers the open chunk the faulted pulse never shipped"
    );
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
    // 16b cut AA.3: held until the match has ended, so proof (f)'s armings cannot
    // overwrite this one's in the window before the first pulse.
    let _stall = STALL_HOOK.lock().await;
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
// (f) The stalled terminal pulse (16b cut AA.3, 16c `B-79`)
// ---------------------------------------------------------------------------

/// Match rules whose **opening pulse is the terminal one**: phase 5's `Timeout`
/// test is `now_ms >= max_duration_ms`, which is `0 >= 0` on the first pulse. That
/// pulse integrates nothing but runs every phase, so it journals
/// `match_ended { timeout }`, flushes the chunk and ends the match — and the
/// by-seed stall hook lands on exactly that pulse.
fn ends_on_the_opening_pulse() -> MatchConfig {
    MatchConfig {
        max_duration_ms: 0,
        ..MatchConfig::default()
    }
}

/// The units the arena handed back on its `Ended`, if it reported one.
fn returned(messages: &[ArenaOut]) -> Option<usize> {
    messages.iter().rev().find_map(|message| match message {
        ArenaOut::Ended { units, .. } => Some(units.len()),
        _ => None,
    })
}

/// **A stall on the pulse that ENDED the match voids it** (16b cut AA.3, 16c
/// `B-79`; doc 16 §3.1 "Failure").
///
/// Before AA.3 the task honoured the pulse's own end before it read the
/// watchdog, so a 200 ms pulse under a 30 ms bound that also timed the match out
/// was reported `Timeout` with both tanks' arena-derived cells returned for
/// commit. Now the result is `Void`, no unit comes back (the world restores the
/// check-out snapshots), and — the forced void is not journalled — the black box
/// ends in the arena's own `match_ended { timeout }` with no `void` event.
#[tokio::test(flavor = "multi_thread")]
async fn aa3_a_stall_on_the_pulse_that_ends_the_match_voids_it_and_returns_no_units() {
    // Held until the match has ended: proof (c) and the stalled-abort proof arm
    // the same global slot.
    let _stall = STALL_HOOK.lock().await;
    let seed = SEEDS[4].0;
    let mut arena_cfg = fast_arena();
    arena_cfg.tempo.stall_ms = 30;
    let match_cfg = ends_on_the_opening_pulse();
    let mut armed = Arena::new(
        custody(seed),
        &arena_cfg,
        &match_cfg,
        &SensorsConfig::default(),
    )
    .expect("the arming arena is accepted");
    armed.stall_next_step_for_tests(Duration::from_millis(200));
    drop(armed);

    let (_handle, mut rx) = start_with(seed, &arena_cfg, &match_cfg);
    let messages = collect(&mut rx, Duration::from_millis(2_000)).await;

    assert_eq!(
        ended(&messages),
        Some(EndReason::Void),
        "a 200 ms terminal pulse under a 30 ms bound voids the match: {messages:#?}"
    );
    assert_eq!(
        returned(&messages),
        Some(0),
        "a voided match returns no unit, so the world restores every snapshot: {messages:#?}"
    );
    // The stalled pulse completed, so its own chunk left — and it carries the
    // arena's own end and no `void` event: the forced void is the task's, not
    // the ledger's.
    let log = events(&messages);
    let kinds: Vec<&str> = log.iter().map(|e| e.kind.as_str()).collect();
    assert!(
        log.iter().any(|event| matches!(
            event.kind,
            EventKind::MatchEnded {
                reason: EndReason::Timeout
            }
        )),
        "the black box ends in the arena's own `match_ended {{ timeout }}`: {kinds:?}"
    );
    assert!(
        !log.iter()
            .any(|event| matches!(event.kind, EventKind::Void)),
        "the forced void is not journalled: {kinds:?}"
    );
    // `collect` returns at the first `Ended`, so "exactly one" is proved only by
    // asking again: nothing more arrives, and no stopping pulse ran.
    assert!(
        collect(&mut rx, Duration::from_millis(300))
            .await
            .is_empty(),
        "a voided match publishes nothing further"
    );
}

/// **The control: the same terminal pulse inside the bound keeps its reason and
/// its units** (16b cut AA.3).
///
/// No hook and no lock, on an out-of-the-way seed (see [`SEEDS`]), and
/// [`fast_arena`] UNCHANGED: its `stall_ms` is the default 2,000, so the control
/// does not depend on a debug-build opening pulse finishing inside a tight bound.
#[tokio::test(flavor = "multi_thread")]
async fn aa3_a_fast_pulse_that_ends_the_match_keeps_its_reason_and_its_units() {
    let seed = 2_381;
    let arena_cfg = fast_arena();
    let (_handle, mut rx) = start_with(seed, &arena_cfg, &ends_on_the_opening_pulse());
    let messages = collect(&mut rx, Duration::from_millis(2_000)).await;

    assert_eq!(
        ended(&messages),
        Some(EndReason::Timeout),
        "a terminal pulse inside the bound keeps its own reason: {messages:#?}"
    );
    assert_eq!(
        returned(&messages),
        Some(2),
        "and both tanks come back for the check-in: {messages:#?}"
    );
    let log = events(&messages);
    assert!(
        log.iter().any(|event| matches!(
            event.kind,
            EventKind::MatchEnded {
                reason: EndReason::Timeout
            }
        )),
        "the chunk carries the arena's `match_ended {{ timeout }}`: {:?}",
        log.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>()
    );
}

/// **A stall on the pulse that honours an ABORT voids it too, and the black box
/// ends in `aborted`** (16b cut AA.3, 16c `B-79`).
///
/// The terminal arm is reason-agnostic, and the end the arena journals is not
/// always a `match_ended`: the library's `Stop` path writes `aborted` for an
/// abort. This pins what the black box actually holds when the forced void lands
/// on that path.
///
/// A **current-thread** runtime makes the stalled pulse deterministically the
/// stopping one: the spawned task is not polled until this test first awaits, so
/// the `Stop(Abort)` is already in the inbox, and the loop's biased select takes
/// the inbox before the (already due) opening pulse — whose phase 1 then honours
/// the stop.
#[tokio::test(flavor = "current_thread")]
async fn aa3_a_stall_on_the_pulse_that_honours_an_abort_voids_it_and_ends_in_aborted() {
    // Held until the match has ended: proof (c) and the terminal-pulse proof arm
    // the same global slot.
    let _stall = STALL_HOOK.lock().await;
    let seed = SEEDS[5].0;
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

    let (handle, mut rx) = start(seed, &arena_cfg);
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");
    let messages = collect(&mut rx, Duration::from_millis(2_000)).await;

    assert_eq!(
        ended(&messages),
        Some(EndReason::Void),
        "a 200 ms stopping pulse under a 30 ms bound voids the match: {messages:#?}"
    );
    assert_eq!(
        returned(&messages),
        Some(0),
        "a voided match returns no unit: {messages:#?}"
    );
    let log = events(&messages);
    let kinds: Vec<&str> = log.iter().map(|e| e.kind.as_str()).collect();
    assert!(
        log.last()
            .is_some_and(|event| matches!(event.kind, EventKind::Aborted)),
        "the black box ends in the arena's own `aborted`: {kinds:?}"
    );
    assert!(
        !log.iter()
            .any(|event| matches!(event.kind, EventKind::Void | EventKind::MatchEnded { .. })),
        "no `void` and no `match_ended`: the forced void is the task's: {kinds:?}"
    );
    assert!(
        collect(&mut rx, Duration::from_millis(300))
            .await
            .is_empty(),
        "a voided match publishes nothing further"
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
    let (handle, _join, _callsigns) = spawn_arena(
        &tokio::runtime::Handle::current(),
        MatchId(seed),
        unseated_custody(seed),
        arena_cfg,
        &MatchConfig::default(),
        sensors_cfg,
        &tacsim::config::MindsConfig::default(),
        &std::collections::BTreeMap::new(),
        pool(),
        egress_tx,
    )
    .expect("the test check-out is accepted");
    (handle, egress_rx)
}

/// A sensors block whose pass cadence is past any window a test here opens: the
/// pass is a due event for **every** unit, at its seeded offset inside the
/// cadence (`tacsim::arena::rng::stagger_ms`, 16b cut U.1), so on the defaults
/// it — and not the tempo — is what decides the wakes at `Quiet`.
///
/// "Past any window" is **by seed, not by construction** since U.1: the first
/// pass falls anywhere in the 600 s, and a unit whose offset landed inside a
/// test's ten or so seconds would add wakes. The D.1 idle-clock test asserts it
/// of its seeds before it starts; a new use of this block has to do the same.
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
/// the reflex controller's `decide_ms`, which falls every 1,000 ms on the
/// defaults and would otherwise be what decides the wakes at `Quiet`. As with
/// [`no_passes`], the reflex tick's first offset is seeded inside the 600 s
/// (16b cut U.1), so "past the window" holds on the seeds these tests use.
///
/// **16b cut AF.2**: with no snapshots ([`without_snapshots`]).
fn arena_at(period_ms: u32) -> ArenaConfig {
    let mut cfg = without_snapshots(ArenaConfig::default());
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
/// **This pins the clause, not the shipped cadence.** On the shipped config the
/// per-unit sensor pass and the reflex decision are both due every 1,000 ms, at
/// seeded offsets, and `next_due_ms` wakes the task at the first substep boundary
/// at or after each (16b cut U.1), so the shipped wakes are wherever those
/// offsets round up to and not a period at all. [`no_passes`] and
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
        // A seed of its own per cadence, and none of the seeds in `SEEDS`.
        let seed = 2_360 + u64::from(period_ms);
        // 16b cut U.1: the pushed-out pass and tick land at a seeded offset
        // inside their 600 s cadence, so "past the window" is this seed's to
        // hold, and it is asserted rather than assumed.
        for unit in [RED, BLUE] {
            let offset = tacsim::arena::rng::stagger_ms(seed, unit, 600_000);
            assert!(
                offset > 10_050,
                "seed {seed}: {unit:?}'s first pass and tick fall at {offset} ms, inside the \
                 window"
            );
        }
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
/// way; this one changes nothing but the snapshot rule (16b cut AF.2 turned it
/// off in every fixture of this file that predates it, [`without_snapshots`]:
/// an `ArenaOut::Snapshot` is not a `Frames` and [`clocks`] never reads one, so
/// the clause pinned here is the shipped one either way). On the defaults the per-unit sensor pass
/// (`sensors.pass_ms`, 1,000 ms) and the reflex decision (`reflex.decide_ms`,
/// 1,000 ms) are both due more often than `tempo.quiet_ms`, and `next_due_ms` is
/// the earlier of the next period multiple and the first substep boundary at or
/// after the next due event (16b cut U.1) — so every arena second has a wake
/// whatever tempo the arena picks, and the board's staleness bound is about one
/// arena second and not one tempo period: one second plus the lattice point the
/// earliest pass rounds up to, at most `2,000 − substep` ms (the
/// [`tacsim::arena_task::ArenaOut::Frames`] doc has the arithmetic).
///
/// That is the figure the seam is sized by and the figure
/// [`tacsim::arena_task::ArenaOut::Frames`]' own doc states, so it is pinned on
/// the arena block the game ships — its tempo, sensor and reflex cadences — and
/// not on one no server runs; only `snapshot_ms` is off.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn d1_the_shipped_config_publishes_its_clock_about_once_an_arena_second() {
    // 16b cut AF.2: the shipped arena block with the snapshot rule off
    // ([`without_snapshots`]) — a snapshot is not a clock, and nothing here is
    // about one.
    let (_handle, mut rx) = start_unseated(
        2_377,
        &without_snapshots(ArenaConfig::default()),
        &SensorsConfig::default(),
    );
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
    // And never further apart than the staleness bound: a 5,000 ms gap here
    // would mean the pulse rate really was `tempo.quiet_ms` and the seam's own
    // doc was over-stating the board's freshness by five times.
    //
    // **16b cut U.1**: the bound was a second and 50 ms. The task now wakes on
    // the substep lattice (`Arena::next_due_ms`) and the passes fall at seeded
    // offsets, so a second that no `Quiet` period multiple opens opens at the
    // earliest pass offset rounded up to the lattice (an offset past the last
    // lattice point rounds up to the next second's start). The gap after a
    // period multiple is one second plus that; it is computed off the fixture,
    // held to the general `2,000 − substep` on top, and is **reached** — the
    // five-second boundary is inside the window — so the rule is pinned and
    // not this seed's luck.
    let substep = fixture_substep();
    let earliest = tacsim::arena::rng::stagger_ms(2_377, RED, 1_000)
        .min(tacsim::arena::rng::stagger_ms(2_377, BLUE, 1_000));
    let opens_at = (earliest + substep - 1) / substep * substep % 1_000;
    let bound = 1_000 + opens_at;
    assert!(bound <= 2_000 - substep, "{bound}");
    let mut widest = 0_i64;
    for pair in clocks.windows(2) {
        let [(_, first), (_, next)] = pair else {
            continue;
        };
        widest = widest.max(next - first);
        assert!(
            next - first <= bound,
            "the shipped board went {} ms without a clock, past {bound}: {clocks:?}",
            next - first
        );
    }
    assert_eq!(
        widest, bound,
        "the gap after the Quiet period multiple was not one second plus the first pass's \
         lattice point: {clocks:?}"
    );
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

// ---------------------------------------------------------------------------
// 16b cut M.1 — the seam: the end waits, frames are held, nothing is dropped
// ---------------------------------------------------------------------------

/// One order at the check-out's epoch, on the channel it needs.
fn order(unit: ObjId, kind: IntentKind, seq: u64) -> ArenaMsg {
    ArenaMsg::Intent(Intent {
        unit,
        channel: kind.channel_required(),
        epoch: 1,
        kind,
        seq,
    })
}

/// `FIRE` with a weapon the fixture tanks do not carry: phase 4 refuses it and
/// stages a `Refused` notice for the pilot — a notice that exists nowhere else.
fn fire(unit: ObjId, seq: u64) -> ArenaMsg {
    order(
        unit,
        IntentKind::Fire {
            weapon: SmolStr::new("cannon"),
            slot: None,
        },
        seq,
    )
}

/// `LOCK` on a contact nobody holds: refused `no_contact` in phase 4, said.
fn lock(unit: ObjId, seq: u64) -> ArenaMsg {
    order(
        unit,
        IntentKind::Lock {
            contact: tacsim::arena::state::ContactId(0x5eed),
        },
        seq,
    )
}

/// Every notice for `pilot`, with the index of the message it arrived in.
fn notices_for(messages: &[ArenaOut], pilot: ObjId) -> Vec<(usize, FrameNotice)> {
    messages
        .iter()
        .enumerate()
        .filter_map(|(at, message)| match message {
            ArenaOut::Frames { frames, .. } => Some(
                frames
                    .iter()
                    .filter(|(to, _)| *to == pilot)
                    .flat_map(|(_, frame)| frame.notices.iter().cloned().map(move |n| (at, n)))
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .collect()
}

/// The `Refused` notices among them, as `(order word, refusal)`.
fn refusals_for(messages: &[ArenaOut], pilot: ObjId) -> Vec<(String, Refusal)> {
    notices_for(messages, pilot)
        .into_iter()
        .filter_map(|(_, notice)| match notice {
            FrameNotice::Refused { order, refusal, .. } => Some((order.to_string(), refusal)),
            _ => None,
        })
        .collect()
}

/// The indices of the `Frames` messages that carry a frame for `pilot`.
fn frames_for(messages: &[ArenaOut], pilot: ObjId) -> Vec<usize> {
    messages
        .iter()
        .enumerate()
        .filter(|(_, message)| {
            matches!(message, ArenaOut::Frames { frames, .. }
                if frames.iter().any(|(to, _)| *to == pilot))
        })
        .map(|(at, _)| at)
        .collect()
}

/// **`B-48`: an end waits for every chunk and the held frame, then reports the
/// digest the log can reproduce** (16b cut M.1).
///
/// An egress with one slot after its recording header, which the world does not
/// read while the match closes several
/// chunks: before this cut `Task::end` offered the backlog once with `try_send`
/// and reported `Ledger::digest()`, so the result named bytes the log never got.
/// Now every chunk arrives, in order, then the final held `Frames`, then one
/// `Ended` whose digest is SHA-256 over the chunk bytes as received.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m1_an_end_waits_for_every_chunk_and_the_held_frame_and_reports_the_logs_digest() {
    let seed = SEEDS[6].0;
    let (handle, _join, mut rx) =
        start_with_egress(seed, &fast_arena(), &MatchConfig::default(), 2);
    // The recording header and opening frames fill both slots; everything
    // after waits, preserving the one-frame backpressure premise.
    tokio::time::sleep(Duration::from_millis(500)).await;
    // A refusal staged in a pulse the world cannot take: held, not lost.
    handle.offer(fire(RED, 1)).expect("the inbox takes a fire");
    tokio::time::sleep(Duration::from_millis(3_000)).await;
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");
    // Long enough for the stopping pulse to run and `end` to be waiting.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let messages = collect(&mut rx, Duration::from_millis(10_000)).await;
    let last = messages.len().saturating_sub(1);
    assert!(
        matches!(messages.get(last), Some(ArenaOut::Ended { .. })),
        "the result is the last message: {messages:#?}"
    );
    assert_eq!(
        messages
            .iter()
            .filter(|message| matches!(message, ArenaOut::Ended { .. }))
            .count(),
        1,
        "exactly one result"
    );
    assert!(
        matches!(
            messages.get(last.saturating_sub(1)),
            Some(ArenaOut::Frames { .. })
        ),
        "the held frame is sent before the result: {messages:#?}"
    );
    // The recording header is followed by the opening frame. Ledger chunks
    // and recording checkpoints drain before the held frame and the result.
    assert!(matches!(
        messages.first(),
        Some(ArenaOut::Tape {
            write: TapeWrite::Open(_),
            ..
        })
    ));
    assert!(matches!(messages.get(1), Some(ArenaOut::Frames { .. })));
    let chunk_seqs: Vec<u64> = messages
        .iter()
        .filter_map(|message| match message {
            ArenaOut::Ledger { chunk, .. } => Some(chunk.seq),
            _ => None,
        })
        .collect();
    assert!(
        chunk_seqs.len() >= 3,
        "several chunks closed while the world was not reading: {chunk_seqs:?}"
    );
    assert_eq!(
        chunk_seqs,
        (1..=chunk_seqs.len() as u64).collect::<Vec<u64>>(),
        "every chunk arrives, in sequence, with no gap"
    );
    assert_eq!(
        messages
            .iter()
            .filter(|message| matches!(message, ArenaOut::Frames { .. }))
            .count(),
        2,
        "the opening frame and the one held frame: {messages:#?}"
    );
    // The refusal arrives once, in the held frame, before the end.
    let refused: Vec<(usize, FrameNotice)> = notices_for(&messages, RED_PILOT)
        .into_iter()
        .filter(|(_, notice)| matches!(notice, FrameNotice::Refused { .. }))
        .collect();
    assert_eq!(refused.len(), 1, "the refusal is said once: {refused:?}");
    assert_eq!(refused.first().map(|(at, _)| *at), Some(last - 1));
    // And the digest is the log's.
    let mut log = Sha256::new();
    for message in &messages {
        if let ArenaOut::Ledger { chunk, .. } = message {
            log.update(&chunk.bytes);
        }
    }
    let log: [u8; 32] = log.finalize().into();
    let reported = messages.iter().find_map(|message| match message {
        ArenaOut::Ended { digest, .. } => Some(*digest),
        _ => None,
    });
    assert_eq!(
        reported,
        Some(log),
        "Ended.digest is SHA-256 over the chunks received"
    );
    assert_eq!(ended(&messages), Some(EndReason::Abort));
}

/// **`B-48`: the stop signal ends an end that is waiting on a full egress**
/// (16b cut M.1).
///
/// A stopped wait sends no `Ended` and returns: the `watch` change the wait took
/// never fires again, so falling through to the next send on a full egress would
/// hang the task — and the shutdown tail's join with it.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m1_the_stop_signal_releases_an_end_waiting_on_a_full_egress() {
    let seed = SEEDS[7].0;
    let (handle, join, mut rx) = start_with_egress(seed, &fast_arena(), &MatchConfig::default(), 1);
    tokio::time::sleep(Duration::from_millis(2_000)).await;
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        handle.is_live(),
        "the task is still waiting in `end` on the full egress"
    );
    handle.signal_stop();
    tokio::time::timeout(Duration::from_secs(3), join)
        .await
        .expect("the stop signal ends the waiting task")
        .expect("the task did not panic");
    assert!(!handle.is_live());
    assert!(!handle.has_reported(), "a stopped task reports nothing");
    let tail = collect(&mut rx, Duration::from_millis(500)).await;
    assert!(
        ended(&tail).is_none(),
        "a process that is stopping is owed no result: {tail:#?}"
    );
}

/// **`B-77`: a frame the world could not take is held and merged, never lost**
/// (16b cut M.1).
///
/// The world stops reading across the `StartupComplete` pulse, a refusal, a
/// drive and a release. When it reads again: one message carries `PowerOnline`
/// and its frame reads `Online`; the refusal arrives once; no notice arrives
/// twice; the delivered view is the latest; the released pilot is in nothing
/// sent after the release — and that pilot had a refusal of their own **held**
/// when the release came (a `fire` of BLUE's, staged into a frame the world
/// could not take), so the proof fails if the task sends a held frame to a
/// pilot the world has let go (the leak doc 16 §3.6's observer rule forbids).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m1_frames_the_world_could_not_take_are_held_and_merged() {
    let seed = SEEDS[8].0;
    let match_cfg = MatchConfig {
        startup_ms: 300,
        ..MatchConfig::default()
    };
    let (handle, _join, mut rx) = start_with_egress(seed, &fast_arena(), &match_cfg, 1);
    handle
        .offer(order(RED, IntentKind::Startup, 1))
        .expect("the inbox takes the startup");
    tokio::time::sleep(Duration::from_millis(200)).await;
    handle.offer(fire(RED, 2)).expect("the inbox takes a fire");
    tokio::time::sleep(Duration::from_millis(600)).await;
    handle
        .offer(order(RED, IntentKind::Heading { mdeg: 90_000 }, 3))
        .expect("the inbox takes a heading");
    handle
        .offer(order(RED, IntentKind::Speed { cms: 500 }, 4))
        .expect("the inbox takes a speed");
    // A notice for BLUE's pilot, staged while the world is not reading: it is
    // held, and the release below must take it out of the hold.
    handle.offer(fire(BLUE, 5)).expect("the inbox takes a fire");
    tokio::time::sleep(Duration::from_millis(300)).await;
    handle
        .offer(ArenaMsg::Control(Control {
            unit: BLUE,
            channel: Channel::Helm,
            epoch: 2,
            state: ControlState::Released,
            pilot: Some(BLUE_PILOT),
        }))
        .expect("the inbox takes a control");
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");

    let messages = collect(&mut rx, Duration::from_millis(10_000)).await;
    assert_eq!(ended(&messages), Some(EndReason::Abort), "{messages:#?}");

    // `PowerOnline`, once, on a frame that reads `Online`.
    let online: Vec<usize> = notices_for(&messages, RED_PILOT)
        .into_iter()
        .filter(|(_, notice)| *notice == FrameNotice::PowerOnline)
        .map(|(at, _)| at)
        .collect();
    assert_eq!(online.len(), 1, "power online is said once: {online:?}");
    let carrying = online.first().and_then(|at| messages.get(*at));
    let power = carrying.and_then(|message| match message {
        ArenaOut::Frames { frames, .. } => frames
            .iter()
            .find(|(to, _)| *to == RED_PILOT)
            .map(|(_, frame)| frame.power),
        _ => None,
    });
    assert_eq!(
        power,
        Some(Power::Online),
        "and the frame it rides reads online"
    );

    // The refusal, once.
    assert_eq!(
        refusals_for(&messages, RED_PILOT)
            .iter()
            .filter(|(word, _)| word == "fire")
            .count(),
        1,
        "the refusal is said exactly once"
    );
    // No notice in two messages: every notice this match staged is distinct.
    let said: Vec<FrameNotice> = notices_for(&messages, RED_PILOT)
        .into_iter()
        .map(|(_, notice)| notice)
        .collect();
    for (at, notice) in said.iter().enumerate() {
        assert!(
            !said.iter().skip(at + 1).any(|other| other == notice),
            "{notice:?} was delivered twice: {said:?}"
        );
    }

    // The delivered view is the latest: the last frame RED's pilot was sent
    // stands where the arena returned the tank, and it moved.
    let last_pose = frames_for(&messages, RED_PILOT)
        .last()
        .and_then(|at| messages.get(*at))
        .and_then(|message| match message {
            ArenaOut::Frames { frames, .. } => frames
                .iter()
                .find(|(to, _)| *to == RED_PILOT)
                .map(|(_, frame)| (frame.pose.x_cm, frame.pose.y_cm)),
            _ => None,
        });
    let returned_pose = messages.iter().find_map(|message| match message {
        ArenaOut::Ended { units, .. } => units
            .iter()
            .find(|unit| unit.unit == RED)
            .map(|unit| (unit.pose.x_cm, unit.pose.y_cm)),
        _ => None,
    });
    assert_eq!(last_pose, returned_pose, "the delivered view is the newest");
    assert!(
        last_pose.is_some_and(|(x_cm, _)| x_cm > centre(1, 4).x_cm),
        "the tank drove: {last_pose:?}"
    );

    // The released pilot: in the opening frame, and in nothing sent after —
    // not even the refusal that was held for them when they left.
    let blue = frames_for(&messages, BLUE_PILOT);
    assert!(
        blue.iter().all(|at| *at == 0),
        "a released pilot was sent a held frame: {blue:?}"
    );
    assert_eq!(
        refusals_for(&messages, BLUE_PILOT),
        Vec::new(),
        "a notice held for a released pilot was delivered"
    );
}

/// The one frame for `pilot` in message `at`, if it carries one.
fn frame_in(messages: &[ArenaOut], at: usize, pilot: ObjId) -> Option<&tacsim::arena::step::Frame> {
    match messages.get(at) {
        Some(ArenaOut::Frames { frames, .. }) => frames
            .iter()
            .find(|(to, _)| *to == pilot)
            .map(|(_, frame)| frame),
        _ => None,
    }
}

/// **`B-77`: past sixty-four held notices a pilot, the oldest go** (16b cut M.1).
///
/// The rate raised to sixty-four; the world stops reading after the opening
/// frame. Forty `LOCK`s in one pulse stage forty `no_contact` refusals, held;
/// an arena second later forty `FIRE`s stage forty `no_weapon` refusals, and
/// the merge makes eighty — past the bound only through the merge, since
/// neither pulse alone reaches it. The delivered frame carries the newest
/// sixty-four: the sixteen oldest locks went, the other twenty-four and all
/// forty fires stand, in order. The once-a-match `warn` has no subscriber to be
/// read by in this crate's tests; `Task::held_notice_drops` counts what went.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m1_a_pilot_is_held_at_most_sixty_four_notices() {
    let seed = SEEDS[9].0;
    let mut arena_cfg = fast_arena();
    arena_cfg.intents.max_pending_per_unit = 64;
    let (handle, _join, mut rx) = start_with_egress(seed, &arena_cfg, &MatchConfig::default(), 2);
    // The recording header and opening frame take the slots. No later frame
    // can leave before the final collection below, so notices must merge.
    tokio::time::sleep(Duration::from_millis(100)).await;
    for seq in 1..=40 {
        handle
            .offer(lock(RED, seq))
            .expect("the inbox takes a lock");
    }
    // Past the next arena second, so the seat's rate has room for forty more.
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    for seq in 41..=80 {
        handle
            .offer(fire(RED, seq))
            .expect("the inbox takes a fire");
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");
    let messages = collect(&mut rx, Duration::from_millis(10_000)).await;
    assert_eq!(ended(&messages), Some(EndReason::Abort));
    assert!(matches!(
        messages.first(),
        Some(ArenaOut::Tape {
            write: TapeWrite::Open(_),
            ..
        })
    ));
    let delivered: Vec<usize> = frames_for(&messages, RED_PILOT);
    assert_eq!(delivered.len(), 2, "the opening frame and the held one");
    let at = delivered.last().copied().unwrap_or(0);
    assert_eq!(
        frame_in(&messages, at, RED_PILOT).map(|frame| frame.notices.len()),
        Some(64),
        "eighty refusals were held; sixty-four are kept"
    );
    let mut kept: Vec<(String, Refusal)> = vec![("lock".to_owned(), Refusal::NoContact); 24];
    kept.extend(vec![("fire".to_owned(), Refusal::NoWeapon); 40]);
    assert_eq!(
        refusals_for(messages.get(at..=at).unwrap_or_default(), RED_PILOT),
        kept,
        "the oldest sixteen went, and the rest are in the order they were staged"
    );
}

/// **`B-77`: a frame the world can take is never cut** (16b cut M.1).
///
/// The bound is on what a lagging world lets pile up, not on one pulse: with
/// the egress keeping up, a burst that stages sixty-five refusals for one pilot
/// in one pulse — the rate at sixty-four takes sixty-four `FIRE`s, each refused
/// `no_weapon`, and says `queue_full` for the sixty-fifth — reaches the pilot
/// whole, the oldest (`queue_full`, staged in phase 2) first.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m1_a_frame_the_world_can_take_is_never_cut() {
    let seed = SEEDS[16].0;
    let mut arena_cfg = fast_arena();
    arena_cfg.intents.max_pending_per_unit = 64;
    let (handle, _join, mut rx) = start_with_egress(seed, &arena_cfg, &MatchConfig::default(), 256);
    tokio::time::sleep(Duration::from_millis(100)).await;
    for seq in 1..=70 {
        handle
            .offer(fire(RED, seq))
            .expect("the inbox takes a fire");
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");
    let messages = collect(&mut rx, Duration::from_millis(10_000)).await;
    assert_eq!(ended(&messages), Some(EndReason::Abort));
    let mut told = vec![("fire".to_owned(), Refusal::QueueFull)];
    told.extend(vec![("fire".to_owned(), Refusal::NoWeapon); 64]);
    assert_eq!(
        refusals_for(&messages, RED_PILOT),
        told,
        "sixty-five notices in one pulse the world could take are sixty-five told"
    );
}

/// **`B-50`: seventeen controls and a release in one window lose nothing**
/// (16b cut M.1).
///
/// Before this cut the pre-pulse buffer dropped its **oldest** message when it
/// held `max_pending_per_unit × units` (sixteen here), and a dropped control left
/// the world's epoch above the arena's. So the `Released` goes in FIRST, at epoch
/// 2, with the seventeen `Live`s at epochs 3 to 19 behind it: the old buffer
/// dropped exactly the release (and the `Live` at 3), and RED's pilot went on
/// being sent frames. Now the `Live`s collapse to the highest, the release they
/// displaced is kept and runs first, an order at epoch 19 afterwards is taken
/// (its `startup` is journalled), one at 18 is not, and the released pilot is
/// sent nothing.
///
/// A `Live` stands on top, so the release's hand-over to the hull's sealed mind
/// (the reflex controller until 17b cut M4.3; re-pointed by 17b cut M5.7) is
/// taken back in the same phase 1, before the machine ever ticks, and the one
/// `startup` in the black box is the seat's.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m1_seventeen_controls_and_a_release_collapse_to_the_highest_epoch() {
    let seed = SEEDS[10].0;
    let (handle, _join, mut rx) =
        start_with_egress(seed, &fast_arena(), &MatchConfig::default(), 256);
    handle
        .offer(ArenaMsg::Control(Control {
            unit: RED,
            channel: Channel::Helm,
            epoch: 2,
            state: ControlState::Released,
            pilot: Some(RED_PILOT),
        }))
        .expect("a control always fits");
    for epoch in 3..=19 {
        handle
            .offer(ArenaMsg::Control(Control {
                unit: RED,
                channel: Channel::Helm,
                epoch,
                state: ControlState::Live,
                pilot: None,
            }))
            .expect("a control always fits");
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let startup = |epoch: u64, seq: u64| {
        ArenaMsg::Intent(Intent {
            unit: RED,
            channel: Channel::Helm,
            epoch,
            kind: IntentKind::Startup,
            seq,
        })
    };
    // One below the highest: refused `epoch`, silently.
    handle
        .offer(startup(18, 1))
        .expect("the inbox takes a startup");
    tokio::time::sleep(Duration::from_millis(200)).await;
    handle
        .offer(startup(19, 2))
        .expect("the inbox takes a startup");
    tokio::time::sleep(Duration::from_millis(200)).await;
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");
    let messages = collect(&mut rx, Duration::from_millis(5_000)).await;
    assert_eq!(ended(&messages), Some(EndReason::Abort));
    let startups: Vec<i64> = events(&messages)
        .into_iter()
        .filter(|event| matches!(event.kind, EventKind::Startup { unit } if unit == RED))
        .map(|event| event.t_ms)
        .collect();
    assert_eq!(
        startups.len(),
        1,
        "only the order at the highest epoch was taken: {startups:?}"
    );
    assert!(
        startups.first().is_some_and(|t_ms| *t_ms >= 400),
        "and it was the second one: {startups:?}"
    );
    assert!(
        frames_for(&messages, RED_PILOT).is_empty(),
        "the released pilot was sent a frame"
    );
    assert!(!frames_for(&messages, BLUE_PILOT).is_empty());
}

/// **`B-50`: a release displaced by the next pilot's control still runs**
/// (16b cut M.1): `Released(RED's pilot)` then `Live(another)` at a higher epoch
/// in one window, and RED's pilot is off the roster.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m1_a_release_displaced_in_one_window_still_takes_the_pilot_off_the_roster() {
    let seed = SEEDS[11].0;
    let mut arena_cfg = fast_arena();
    arena_cfg.reflex.decide_ms = 600_000;
    let (handle, _join, mut rx) = start_with_egress(seed, &arena_cfg, &MatchConfig::default(), 256);
    handle
        .offer(ArenaMsg::Control(Control {
            unit: RED,
            channel: Channel::Helm,
            epoch: 2,
            state: ControlState::Released,
            pilot: Some(RED_PILOT),
        }))
        .expect("a control always fits");
    handle
        .offer(ArenaMsg::Control(Control {
            unit: RED,
            channel: Channel::Helm,
            epoch: 3,
            state: ControlState::Live,
            pilot: Some(BLUE_PILOT),
        }))
        .expect("a control always fits");
    tokio::time::sleep(Duration::from_millis(500)).await;
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");
    let messages = collect(&mut rx, Duration::from_millis(5_000)).await;
    assert_eq!(ended(&messages), Some(EndReason::Abort));
    assert!(
        frames_for(&messages, RED_PILOT).is_empty(),
        "the displaced release did not run: RED's pilot was sent a frame"
    );
    assert!(
        !frames_for(&messages, BLUE_PILOT).is_empty(),
        "the other pilot is still sent frames"
    );
}

/// **`T-11`: a burst is bounded per unit, the newest refused, and continuous
/// orders coalesce** (16b cut M.1).
///
/// Twelve discrete orders for RED with one `LOCK` for BLUE in the middle, in one
/// window. RED's pre-pulse buffer keeps its first nine (the rate plus one) and
/// refuses the three newest; the arena takes eight and says `queue_full` for the
/// ninth. So RED's pilot hears the eight earliest orders resolved — four locks
/// and four fires, in order — and one `queue_full` for a `startup`, and no
/// `startup` is journalled; BLUE's `LOCK` resolves untouched. Then two headings
/// and two speeds to a cold tank in one window are one refusal of each, and the
/// same to a warm one leave it on the last of each.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m1_a_burst_is_bounded_per_unit_and_continuous_orders_coalesce() {
    let seed = SEEDS[12].0;
    let (handle, _join, mut rx) =
        start_with_egress(seed, &fast_arena(), &MatchConfig::default(), 256);
    let burst = [
        lock(RED, 1),
        fire(RED, 2),
        lock(RED, 3),
        fire(RED, 4),
        lock(BLUE, 5),
        lock(RED, 6),
        fire(RED, 7),
        lock(RED, 8),
        fire(RED, 9),
        order(RED, IntentKind::Startup, 10),
        order(RED, IntentKind::Startup, 11),
        order(RED, IntentKind::Startup, 12),
        order(RED, IntentKind::Startup, 13),
        order(BLUE, IntentKind::Startup, 14),
    ];
    for message in burst {
        handle.offer(message).expect("the burst fits the inbox");
    }
    let mut messages = collect(&mut rx, Duration::from_millis(11_000)).await;
    assert_eq!(
        refusals_for(&messages, RED_PILOT),
        vec![
            ("startup".to_owned(), Refusal::QueueFull),
            ("lock".to_owned(), Refusal::NoContact),
            ("fire".to_owned(), Refusal::NoWeapon),
            ("lock".to_owned(), Refusal::NoContact),
            ("fire".to_owned(), Refusal::NoWeapon),
            ("lock".to_owned(), Refusal::NoContact),
            ("fire".to_owned(), Refusal::NoWeapon),
            ("lock".to_owned(), Refusal::NoContact),
            ("fire".to_owned(), Refusal::NoWeapon),
        ],
        "the eight earliest resolve, the ninth is refused by the arena, the rest never \
         reach it"
    );
    assert_eq!(
        refusals_for(&messages, BLUE_PILOT),
        vec![("lock".to_owned(), Refusal::NoContact)],
        "another pilot's order is untouched by RED's burst"
    );

    // Two headings and two speeds to the cold RED: one refusal of each.
    let window = messages.len();
    for message in [
        order(RED, IntentKind::Heading { mdeg: 45_000 }, 20),
        order(RED, IntentKind::Heading { mdeg: 90_000 }, 21),
        order(RED, IntentKind::Speed { cms: 300 }, 22),
        order(RED, IntentKind::Speed { cms: 600 }, 23),
    ] {
        handle.offer(message).expect("the inbox takes an order");
    }
    // And the same to BLUE, whose startup has come up by now: the last of each.
    for message in [
        order(BLUE, IntentKind::Heading { mdeg: 90_000 }, 24),
        order(BLUE, IntentKind::Heading { mdeg: 0 }, 25),
        order(BLUE, IntentKind::Speed { cms: 300 }, 26),
        order(BLUE, IntentKind::Speed { cms: 600 }, 27),
    ] {
        handle.offer(message).expect("the inbox takes an order");
    }
    messages.extend(collect(&mut rx, Duration::from_millis(6_000)).await);
    let later = messages.get(window..).unwrap_or_default();
    assert_eq!(
        refusals_for(later, RED_PILOT),
        vec![
            ("heading".to_owned(), Refusal::PowerNotOnline),
            ("speed".to_owned(), Refusal::PowerNotOnline),
        ],
        "one window's two headings and two speeds are one of each"
    );
    let blue = frames_for(&messages, BLUE_PILOT)
        .last()
        .and_then(|at| messages.get(*at))
        .and_then(|message| match message {
            ArenaOut::Frames { frames, .. } => frames
                .iter()
                .find(|(to, _)| *to == BLUE_PILOT)
                .map(|(_, frame)| (frame.pose.heading_mdeg, frame.speed_cms)),
            _ => None,
        });
    assert_eq!(
        blue,
        Some((0, 600)),
        "the last heading and the last speed stand"
    );
    // And the black box never saw RED start up.
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");
    messages.extend(collect(&mut rx, Duration::from_millis(5_000)).await);
    assert!(
        !events(&messages)
            .iter()
            .any(|event| matches!(event.kind, EventKind::Startup { unit } if unit == RED)),
        "a refused startup was journalled"
    );
}

/// **The inbox reserve** (16b cut M.1, `B-50`): with the inbox full of intents,
/// a control, a stop and an ack all still fit, and one more intent does not.
///
/// A current-thread runtime the test never yields on, so the task never drains
/// its inbox.
#[tokio::test(flavor = "current_thread")]
async fn m1_an_inbox_full_of_orders_still_takes_a_control_a_stop_and_an_ack() {
    let seed = SEEDS[13].0;
    let (handle, _join, _rx) = start_with_egress(seed, &fast_arena(), &MatchConfig::default(), 256);
    let mut taken = 0_u64;
    while handle
        .offer(order(RED, IntentKind::Startup, taken + 1))
        .is_ok()
    {
        taken += 1;
        assert!(taken < 1_000, "the inbox never refused an order");
    }
    // `max_pending_per_unit × units`: the reserve is not an intent's.
    assert_eq!(taken, 16);
    // 16b cut I.3: nor a staff tempo floor's — a repeated `@arena/tempo` must
    // not take the slot an `EXIT` needs.
    assert!(
        handle
            .offer(ArenaMsg::TempoFloor {
                period_ms: Some(5_000)
            })
            .is_err(),
        "a tempo floor took the reserve"
    );
    assert!(
        handle
            .offer(ArenaMsg::Control(Control {
                unit: RED,
                channel: Channel::Helm,
                epoch: 2,
                state: ControlState::Released,
                pilot: Some(RED_PILOT),
            }))
            .is_ok(),
        "a control fits behind a full inbox of orders"
    );
    assert!(handle.offer(ArenaMsg::Stop(StopReason::Abort)).is_ok());
    assert!(
        handle
            .offer(ArenaMsg::LedgerAck { upto_chunk_seq: 0 })
            .is_ok()
    );
    assert!(
        handle.offer(order(RED, IntentKind::Startup, 99)).is_err(),
        "and an order still does not"
    );
}

/// **`B-49`: a stop that meets an `Ended` in flight is `Busy`, and one that meets
/// a task that died without a result is `Gone`** (16b cut M.1), on
/// [`ArenaTasks`] itself.
///
/// Before this cut both were `Gone`, and `match_::abort` then finished the match
/// with no units — restoring every tank over the arena's real result, which the
/// world threw away when it arrived.
#[tokio::test(flavor = "multi_thread")]
async fn m1_a_stop_meeting_an_ended_in_flight_is_busy_and_a_dead_task_is_gone() {
    let tasks = ArenaTasks::new(256);
    let mut egress = tasks.take_egress().expect("the egress, once");
    let spawn = |seed: u64| {
        spawn_arena(
            &tokio::runtime::Handle::current(),
            MatchId(seed),
            custody(seed),
            &fast_arena(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
            &tacsim::config::MindsConfig::default(),
            &std::collections::BTreeMap::new(),
            pool(),
            tasks.egress_sender().expect("the egress sender"),
        )
        .expect("the test check-out is accepted")
    };

    // The `Ended` in flight: stopped, reported, never drained.
    let seed = SEEDS[14].0;
    let (handle, join, _callsigns) = spawn(seed);
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");
    let join = {
        let mut join = join;
        tokio::time::timeout(Duration::from_secs(5), &mut join)
            .await
            .expect("the task ends")
            .expect("the task did not panic");
        join
    };
    assert!(!handle.is_live());
    assert!(
        handle.has_reported(),
        "the task handed the world its result"
    );
    assert!(tasks.insert(MatchId(seed), handle, join).is_none());
    assert_eq!(
        tasks.send_stop(MatchId(seed), StopReason::Abort),
        Err(ControlRefusal::Busy),
        "the result is on its way: change nothing"
    );
    let mut ends = 0;
    while let Ok(message) = egress.try_recv() {
        if matches!(message, ArenaOut::Ended { match_id, .. } if match_id == MatchId(seed)) {
            ends += 1;
        }
    }
    assert_eq!(ends, 1, "exactly one result is in the egress");

    // The task that died without one.
    let seed = SEEDS[15].0;
    let (handle, join, _callsigns) = spawn(seed);
    handle.signal_stop();
    let join = {
        let mut join = join;
        tokio::time::timeout(Duration::from_secs(5), &mut join)
            .await
            .expect("the task ends")
            .expect("the task did not panic");
        join
    };
    assert!(!handle.has_reported());
    assert!(tasks.insert(MatchId(seed), handle, join).is_none());
    assert_eq!(
        tasks.send_stop(MatchId(seed), StopReason::Abort),
        Err(ControlRefusal::Gone)
    );
    // And a match the table never held.
    assert_eq!(
        tasks.send_stop(MatchId(9_999), StopReason::Abort),
        Err(ControlRefusal::NoMatch)
    );
}

// ---------------------------------------------------------------------------
// 16b cut M.2 (16c `T-03`) — a pilot's action at the end of the pulse that
// accepted it, on the production shape
// ---------------------------------------------------------------------------

/// What one production-shaped `STARTUP` came to: the clock of the `Frames` the
/// world had before it offered the order, the clock of the first `Frames` whose
/// `RED` frame reads `Starting`, and the `startup` and `power_online` lines'
/// arena milliseconds.
struct M2Startup {
    before_ms: i64,
    accepted_ms: i64,
    startup_ms: i64,
    online_ms: i64,
}

/// The `now_ms` of a `Frames` message.
fn clock_of(message: &ArenaOut) -> Option<i64> {
    match message {
        ArenaOut::Frames { now_ms, .. } => Some(*now_ms),
        _ => None,
    }
}

/// One match of `fast_arena`'s shape with every period at `period_ms` and the
/// **default** `summary_ms` — so nothing but the period, the passes and the
/// startup's own completion wakes the task — offered one `STARTUP` for `RED`
/// after its first `Frames`, and aborted once `Power online.` has been seen.
async fn m2_startup_through_the_task(seed: u64, period_ms: u32, startup_ms: u32) -> M2Startup {
    let mut arena_cfg = fast_arena();
    arena_cfg.tempo.quiet_ms = period_ms;
    arena_cfg.tempo.alert_ms = period_ms;
    arena_cfg.tempo.engaged_ms = period_ms;
    arena_cfg.ledger.summary_ms = ArenaConfig::default().ledger.summary_ms;
    let match_cfg = MatchConfig {
        startup_ms,
        ..MatchConfig::default()
    };
    let (handle, _join, mut rx) = start_with_egress(seed, &arena_cfg, &match_cfg, 256);
    let mut messages: Vec<ArenaOut> = Vec::new();
    // The first `Frames`, which the opening pulse sends because nobody has been
    // told anything yet.
    let before_ms = loop {
        let message = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("the arena published")
            .expect("the egress is open");
        let clock = clock_of(&message);
        messages.push(message);
        if let Some(clock) = clock {
            break clock;
        }
    };
    let offered_at = messages.len();
    handle
        .offer(ArenaMsg::Intent(Intent {
            unit: RED,
            channel: Channel::Helm,
            epoch: 1,
            kind: IntentKind::Startup,
            seq: 1,
        }))
        .expect("the inbox takes a startup");
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("the arena published")
            .expect("the egress is open");
        messages.push(message);
        if notices_for(&messages, RED_PILOT)
            .iter()
            .any(|(_, notice)| matches!(notice, FrameNotice::PowerOnline))
        {
            break;
        }
    }
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");
    messages.extend(collect(&mut rx, Duration::from_secs(10)).await);
    assert_eq!(ended(&messages), Some(EndReason::Abort));

    let accepted_ms = messages
        .iter()
        .skip(offered_at)
        .find_map(|message| match message {
            ArenaOut::Frames { frames, now_ms, .. }
                if frames
                    .iter()
                    .any(|(to, frame)| *to == RED_PILOT && frame.power == Power::Starting) =>
            {
                Some(*now_ms)
            }
            _ => None,
        })
        .expect("a frame read `Starting`");
    let log = events(&messages);
    let line = |kind: &str| -> i64 {
        log.iter()
            .find(|event| event.kind.as_str() == kind)
            .map(|event| event.t_ms)
            .unwrap_or_else(|| panic!("no `{kind}` line"))
    };
    M2Startup {
        before_ms,
        accepted_ms,
        startup_ms: line("startup"),
        online_ms: line("power_online"),
    }
}

/// **`T-03`: a discrete action given by a pilot resolves at the end of the pulse
/// that accepted it** (16b cut M.2; doc 16 §11's D-16-2 as amended) — asserted on
/// the task, where arena time is elapsed wall time and a pulse ends at the
/// earlier of the next period multiple and the first substep boundary at or after
/// the next due event (16b cut U.1), at a 50 ms period and at a 250 ms one.
///
/// The `startup` line's millisecond IS the `now_ms` of the first `Frames` whose
/// frame reads `Starting` — the accepting pulse's end — and it is later than the
/// `Frames` the world held when it offered the order. The completion is a due
/// event, so `power_online` is exactly `startup_ms` after it whatever the
/// period. No value and no modulus is asserted: a production pulse ends where
/// the wall and the lattice put it, so `t_ms % period` does not hold and is not
/// supposed to.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m2_a_pilots_startup_resolves_at_the_end_of_the_pulse_that_accepted_it() {
    const STARTUP_MS: u32 = 300;
    for (seed, period_ms) in [(SEEDS[17].0, 50_u32), (SEEDS[18].0, 250)] {
        let run = m2_startup_through_the_task(seed, period_ms, STARTUP_MS).await;
        assert_eq!(
            run.startup_ms, run.accepted_ms,
            "at a {period_ms} ms period the `startup` line is not the accepting pulse's end"
        );
        assert!(
            run.startup_ms > run.before_ms,
            "at a {period_ms} ms period the order resolved in a pulse the world had already \
             seen ({} against {})",
            run.startup_ms,
            run.before_ms
        );
        assert_eq!(
            run.online_ms,
            run.startup_ms + i64::from(STARTUP_MS),
            "at a {period_ms} ms period the completion is not at its own millisecond"
        );
    }
}

// ---------------------------------------------------------------------------
// 16b cut U.1 (16c `T-22`, Q25) — the task wakes on the substep lattice, from
// its base
// ---------------------------------------------------------------------------

/// The substep the two fixture tanks cut: 1,000 cm/s on 1,000 cm cells of
/// ground at cost 100, so the 250 ms cap.
fn fixture_substep() -> i64 {
    i64::from(tacsim::arena::clock::substep_ms(
        &ArenaConfig::default().tempo,
        CELL_CM,
        1_000,
    ))
}

/// **Every wake is a lattice point, and there are at most `1000 / substep` of
/// them an arena second** (16b cut U.1). A two-tank match at `Quiet` on the
/// shipped config with the snapshot rule off (16b cut AF.2,
/// [`without_snapshots`]; a snapshot rides the egress between wakes and is not
/// a `Frames`, so [`clocks`] would not count it anyway), `RED` started and driven north so that its pilot's view
/// changes on every pulse and every wake therefore publishes a `Frames`: each
/// message's `now_ms` is a multiple of the substep, no arena second carries more
/// than four, and more than one message lands in some second — the proof that
/// the messages are the wakes and not the once-a-second clock.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn u1_a_two_tank_match_wakes_on_the_lattice_at_most_once_a_substep() {
    let seed = SEEDS[19].0;
    let match_cfg = MatchConfig {
        startup_ms: 500,
        ..MatchConfig::default()
    };
    // 16b cut AF.2: with the snapshot rule off ([`without_snapshots`]).
    let (handle, _join, mut rx) = start_with_egress(
        seed,
        &without_snapshots(ArenaConfig::default()),
        &match_cfg,
        256,
    );
    handle
        .offer(order(RED, IntentKind::Startup, 1))
        .expect("the inbox takes the startup");
    let mut messages = collect(&mut rx, Duration::from_millis(2_000)).await;
    handle
        .offer(order(RED, IntentKind::Speed { cms: 1_000 }, 2))
        .expect("the inbox takes the speed");
    messages.extend(collect(&mut rx, Duration::from_millis(6_000)).await);
    let substep = fixture_substep();
    assert_eq!(substep, 250, "the fixture's lattice");
    let clocks = clocks(&messages);
    let off: Vec<i64> = clocks
        .iter()
        .map(|(_, now_ms)| *now_ms)
        .filter(|now_ms| now_ms % substep != 0)
        .collect();
    assert!(
        off.is_empty(),
        "a wake off the lattice: {off:?} of {clocks:?}"
    );
    let mut per_second: BTreeMap<i64, u32> = BTreeMap::new();
    for (_, now_ms) in &clocks {
        *per_second.entry(now_ms / 1_000).or_default() += 1;
    }
    let most = per_second.values().copied().max().unwrap_or(0);
    assert!(
        i64::from(most) <= 1_000 / substep,
        "{most} wakes in one arena second, past 1,000 / {substep}: {clocks:?}"
    );
    assert!(
        most > 1,
        "never more than one message a second, so these were the clock and not the \
         wakes: {clocks:?}"
    );
    // At `Quiet` throughout: nobody sees anybody, so nothing raised the tempo.
    assert!(
        !events(&messages)
            .iter()
            .any(|event| event.kind.as_str() == "tempo"),
        "the match left Quiet"
    );
}

/// **A late wake does not carry its lateness into the next deadline** (16b cut
/// U.1): an idle match whose only wake is the 1,000 ms period, woken three
/// milliseconds late by advancing the paused clock past its deadline, pulses at
/// 1,003 and then at 2,000 — the base plus the next lattice point — and not at
/// 2,003. Before the cut the next wake was measured from the late one, and every
/// late wake moved the rest of the match off the lattice for good.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn u1_a_late_wake_still_asks_for_the_next_lattice_point_from_the_base() {
    let seed = SEEDS[20].0;
    // The period is the only wake only while the pushed-out pass and tick land
    // past the window (see `no_passes`).
    for unit in [RED, BLUE] {
        assert!(tacsim::arena::rng::stagger_ms(seed, unit, 600_000) > 2_000);
    }
    let (_handle, mut rx) = start_unseated(seed, &arena_at(1_000), &no_passes());
    let next_clock =
        |message: Option<ArenaOut>| -> Option<i64> { message.as_ref().and_then(clock_of) };
    // The opening pulse's clock, at the base.
    let opening = loop {
        let message = rx.recv().await;
        assert!(message.is_some(), "the egress closed");
        if let Some(clock) = next_clock(message) {
            break clock;
        }
    };
    assert_eq!(opening, 0);
    // The deadline is the base plus 1,000; the clock jumps three past it.
    tokio::time::advance(Duration::from_millis(1_003)).await;
    let late = loop {
        let message = rx.recv().await;
        assert!(message.is_some(), "the egress closed");
        if let Some(clock) = next_clock(message) {
            break clock;
        }
    };
    assert_eq!(
        late, 1_003,
        "the wake was not late: the fixture proves nothing"
    );
    let back = loop {
        let message = rx.recv().await;
        assert!(message.is_some(), "the egress closed");
        if let Some(clock) = next_clock(message) {
            break clock;
        }
    };
    assert_eq!(
        back, 2_000,
        "the late wake's three milliseconds were carried into the next deadline"
    );
}

// ---------------------------------------------------------------------------
// 16b cut AF.2 (16c N16) — the snapshot rides the egress behind the chunks
// ---------------------------------------------------------------------------

/// Every `Snapshot` message, with its index among `messages`.
fn snapshots(messages: &[ArenaOut]) -> Vec<(usize, &tacsim::arena::Snapshot)> {
    messages
        .iter()
        .enumerate()
        .filter_map(|(at, message)| match message {
            ArenaOut::Snapshot { snapshot, .. } => Some((at, snapshot.as_ref())),
            _ => None,
        })
        .collect()
}

/// **The world receives every chunk closed before a snapshot before that
/// snapshot, and none closed after it** (16b cut AF.2): for each `Snapshot`,
/// every `Ledger` whose sequence is at or below its `chunk_seq` came earlier
/// and every one above it came later — the invariant 16b cut AF.3's resume
/// (`flushed_upto = last_closed_seq`) stands on.
fn assert_snapshots_follow_their_chunks(messages: &[ArenaOut]) {
    let chunks: Vec<(usize, u64)> = messages
        .iter()
        .enumerate()
        .filter_map(|(at, message)| match message {
            ArenaOut::Ledger { chunk, .. } => Some((at, chunk.seq)),
            _ => None,
        })
        .collect();
    for (at, snapshot) in snapshots(messages) {
        for (chunk_at, seq) in &chunks {
            assert_eq!(
                *seq <= snapshot.chunk_seq,
                *chunk_at < at,
                "chunk {seq} (message {chunk_at}) against the snapshot at {} ms closing chunk {} \
                 (message {at})",
                snapshot.at_ms,
                snapshot.chunk_seq
            );
        }
    }
}

/// **A snapshot on the opening pulse and one a period of arena time, each one
/// the arena's own** (16b cut AF.2).
///
/// Three runs, on tokio's paused clock:
///
/// 1. **The cadence.** The shipped arena block (`snapshot_ms = 5,000`) over
///    sixty arena seconds, the world reading everything: one `Snapshot` right
///    after the opening pulse's `Frames` and before any later pulse's message,
///    then one per period — thirteen, the i-th taken in period i — each one
///    decoding against the fixture's ground to an arena whose state hash is
///    the one recorded, and every one following its chunks.
/// 2. **The order behind the chunks.** A one-message egress the world does not
///    read for five seconds while a five-millisecond summary cadence closes
///    several chunks: when it reads, the chunks closed before the five-second
///    snapshot come first, in order, and the snapshot straight after them —
///    `Ledger`, `Ledger`, …, `Snapshot`, with only input-tape writes interleaved
///    since M6 — and the opening pulse's snapshot,
///    replaced while it waited, never arrives.
/// 3. **Off.** `snapshot_ms = 0`: no `Snapshot` in sixty seconds.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn af2_a_snapshot_rides_the_egress_on_the_first_pulse_and_every_five_seconds() {
    // (1) The cadence, on the shipped arena block.
    let seed = SEEDS[21].0;
    let arena_cfg = ArenaConfig::default();
    assert_eq!(arena_cfg.snapshot_ms, 5_000, "the shipped cadence");
    let (_handle, mut rx) = start(seed, &arena_cfg);
    let messages = collect(&mut rx, Duration::from_millis(60_050)).await;
    assert!(ended(&messages).is_none(), "the match ran the minute out");
    let taken = snapshots(&messages);
    assert_eq!(
        taken
            .iter()
            .map(|(_, snapshot)| snapshot.at_ms.div_euclid(5_000))
            .collect::<Vec<i64>>(),
        (0..=12).collect::<Vec<i64>>(),
        "one snapshot a period, the i-th in period i: {:?}",
        taken
            .iter()
            .map(|(_, snapshot)| snapshot.at_ms)
            .collect::<Vec<i64>>()
    );
    let (first_at, first) = taken[0];
    assert_eq!(first.at_ms, 0, "the opening pulse's");
    assert!(
        matches!(
            messages.first(),
            Some(ArenaOut::Tape {
                write: TapeWrite::Open(_),
                ..
            })
        ) && matches!(messages.get(1), Some(ArenaOut::Frames { now_ms: 0, .. })),
        "the frozen recording header precedes the opening pulse's frames"
    );
    assert!(
        messages[..first_at].iter().all(|message| match message {
            ArenaOut::Frames { now_ms, .. } => *now_ms == 0,
            ArenaOut::Ledger { .. } => true,
            ArenaOut::Tape { .. } => true,
            _ => false,
        }),
        "the opening snapshot follows the opening pulse's messages and precedes every later \
         pulse's"
    );
    for (_, snapshot) in &taken {
        let arena = tacsim::arena::Snapshot::decode(&snapshot.document, definition())
            .unwrap_or_else(|error| panic!("the snapshot at {} ms: {error}", snapshot.at_ms));
        assert_eq!(arena.now_ms, snapshot.at_ms);
        assert_eq!(
            tacsim::arena::hash::state_hash(&arena),
            snapshot.state_hash,
            "the decoded arena is the arena the task encoded"
        );
    }
    assert_snapshots_follow_their_chunks(&messages);

    // (2) Two chunks and more, then the snapshot, on a one-message egress.
    let seed = SEEDS[22].0;
    let mut arena_cfg = arena_at(5_000);
    arena_cfg.snapshot_ms = 5_000;
    arena_cfg.ledger.summary_ms = 5;
    let (handle, _join, mut rx) = start_with_egress(seed, &arena_cfg, &MatchConfig::default(), 1);
    // The world does not read while the arena runs its first five seconds.
    tokio::time::sleep(Duration::from_millis(5_050)).await;
    // Then it is stopped, so the end drains the task's whole retry queue in
    // order, waiting on the world as it reads.
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");
    let messages = collect(&mut rx, Duration::from_millis(10_000)).await;
    assert_eq!(ended(&messages), Some(EndReason::Abort));
    let taken = snapshots(&messages);
    assert_eq!(
        taken
            .iter()
            .map(|(_, snapshot)| snapshot.at_ms)
            .collect::<Vec<i64>>(),
        vec![5_000],
        "the opening snapshot was replaced by the newer while it waited"
    );
    let (at, snapshot) = taken[0];
    assert!(
        snapshot.chunk_seq >= 2,
        "the pulses before the snapshot closed two chunks at least"
    );
    let before: Vec<u64> = messages[..at]
        .iter()
        .filter_map(|message| match message {
            ArenaOut::Ledger { chunk, .. } => Some(chunk.seq),
            _ => None,
        })
        .collect();
    assert_eq!(
        before,
        (1..=snapshot.chunk_seq).collect::<Vec<u64>>(),
        "every chunk closed before the snapshot, in order, and nothing after it"
    );
    // M6 input-tape writes share the durable backlog. They may interleave
    // with ledger chunks, but no frame or other message may split the final
    // ledger pair from this snapshot. The exact ledger sequence and snapshot
    // hash/cadence checks above still cover every chunk and snapshot.
    let before_without_tape = messages[..at]
        .iter()
        .filter(|message| !matches!(message, ArenaOut::Tape { .. }))
        .collect::<Vec<_>>();
    assert!(
        matches!(
            &before_without_tape[before_without_tape.len().saturating_sub(2)..],
            [ArenaOut::Ledger { .. }, ArenaOut::Ledger { .. }]
        ),
        "Ledger, Ledger, Snapshot with only input-tape writes interleaved"
    );
    assert_snapshots_follow_their_chunks(&messages);

    // (3) Off.
    let seed = SEEDS[23].0;
    let (_handle, mut rx) = start(seed, &without_snapshots(ArenaConfig::default()));
    let messages = collect(&mut rx, Duration::from_millis(60_050)).await;
    assert!(
        messages
            .iter()
            .any(|message| matches!(message, ArenaOut::Frames { .. })),
        "the match ran"
    );
    assert!(
        snapshots(&messages).is_empty(),
        "no snapshot with snapshot_ms = 0"
    );
}

/// **The snapshot of the pulse before the end reaches the world before the
/// end** (16b cut AF.2).
///
/// A one-message egress the world does not read: the opening pulse's `Frames`
/// takes the slot, and the five-second pulse's snapshot waits in the task's
/// retry queue. The abort's pulse ends the match and takes no snapshot of its
/// own, so when `Task::end` drains the queue, the world reads the waiting
/// `Snapshot`, taken before the end's moment, and then, last, `Ended`.
///
/// "None of its own" holds here by the period rule alone and is not the proof
/// of `Task::pulse`'s `ended`/`stalled` guard: a `Stop` is honoured in phase 1
/// before anything integrates, so an abort's pulse never advances arena time
/// and always stands in the period the pulse before it already snapshotted
/// (asserted below). The guard is proven where a terminal or stalled pulse
/// enters a NEW period —
/// [`af2_a_pulse_that_ends_or_stalls_the_match_takes_no_snapshot`].
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn af2_the_pulse_before_the_end_s_snapshot_reaches_the_world_before_the_end() {
    let seed = SEEDS[24].0;
    let mut arena_cfg = arena_at(5_000);
    arena_cfg.snapshot_ms = 5_000;
    let (handle, _join, mut rx) = start_with_egress(seed, &arena_cfg, &MatchConfig::default(), 1);
    // Past the five-second pulse and short of the ten: the snapshot of the
    // pulse at 5,000 is the one waiting.
    tokio::time::sleep(Duration::from_millis(7_000)).await;
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");
    let messages = collect(&mut rx, Duration::from_millis(10_000)).await;
    let end_ms = messages
        .iter()
        .find_map(|message| match message {
            ArenaOut::Ended { end, .. } => Some(end.t_ms),
            _ => None,
        })
        .expect("the match reported its end");
    assert!(
        matches!(messages.last(), Some(ArenaOut::Ended { .. })),
        "the result is the last message"
    );
    let taken = snapshots(&messages);
    assert_eq!(
        taken
            .iter()
            .map(|(_, snapshot)| snapshot.at_ms)
            .collect::<Vec<i64>>(),
        vec![5_000],
        "one snapshot, the pulse before the end's, and none of the end's own"
    );
    assert!(end_ms > 5_000, "the end came after the snapshot's moment");
    assert_eq!(
        end_ms.div_euclid(5_000),
        1,
        "the abort's pulse stands in the snapshot's own period (a stop never advances \
         arena time), so the period rule alone keeps it from taking one"
    );
    // In that order: the snapshot, and after it no other snapshot, then the end.
    let order: Vec<&str> = messages
        .iter()
        .filter_map(|message| match message {
            ArenaOut::Snapshot { .. } => Some("snapshot"),
            ArenaOut::Ended { .. } => Some("ended"),
            _ => None,
        })
        .collect();
    assert_eq!(order, vec!["snapshot", "ended"]);
    assert_snapshots_follow_their_chunks(&messages);
}

/// **A pulse that ended the match, or stalled, takes no snapshot** (16b cut
/// AF.2): `Task::pulse`'s guard, `ended.is_none() && !stalled`, proven on
/// pulses that enter a period no snapshot was taken in — the opening pulse,
/// period 0 — so the period rule alone would take one and only the guard
/// refuses it. (An abort's pulse cannot prove it: a stop never advances arena
/// time, see [`af2_the_pulse_before_the_end_s_snapshot_reaches_the_world_before_the_end`].)
///
/// 1. **Ended.** [`ends_on_the_opening_pulse`]: the opening pulse times the
///    match out. Its result deletes the documents, and a snapshot of an ended
///    arena is nothing a boot may resume, so none reaches the world before the
///    `Ended`.
/// 2. **Stalled.** The opening pulse is held 200 ms past a 30 ms `stall_ms` by
///    the stall hook (proof (c)'s shape), and the watchdog voids the match: a
///    stalled pulse's arena is void, so it is never snapshotted either.
///
/// Without the guard, each run's opening pulse queues a `Snapshot` at 0 ms that
/// `Task::end` drains ahead of the `Ended`.
#[tokio::test(flavor = "multi_thread")]
async fn af2_a_pulse_that_ends_or_stalls_the_match_takes_no_snapshot() {
    // (1) The pulse that ended the match: inside the default 2,000 ms stall
    // bound, so it is the `ended` half alone that holds.
    let seed = SEEDS[25].0;
    let mut arena_cfg = fast_arena();
    arena_cfg.snapshot_ms = 5_000;
    let (_handle, mut rx) = start_with(seed, &arena_cfg, &ends_on_the_opening_pulse());
    let messages = collect(&mut rx, Duration::from_millis(2_000)).await;
    assert_eq!(
        ended(&messages),
        Some(EndReason::Timeout),
        "the opening pulse ended the match: {messages:#?}"
    );
    assert!(
        snapshots(&messages).is_empty(),
        "the pulse that ended the match took no snapshot, though it opened period 0"
    );

    // (2) The stalled pulse. Held until the match has ended: proofs (c) and
    // (f) arm the same global slot.
    let _stall = STALL_HOOK.lock().await;
    let seed = SEEDS[26].0;
    let mut arena_cfg = fast_arena();
    arena_cfg.snapshot_ms = 5_000;
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
    assert!(
        snapshots(&messages).is_empty(),
        "the stalled pulse took no snapshot, though it opened period 0"
    );
}

// ---------------------------------------------------------------------------
// 16b cut AF.3 (16c N16) — a resumed task's clock
// ---------------------------------------------------------------------------

/// **A resumed task pulses on the lattice after the snapshot's moment, and does
/// not snapshot the period it was decoded in again** (16b cut AF.3;
/// `arena_task::resume_arena`'s two differences from `spawn_arena`).
///
/// One unseated match at a 1,000 ms period with a snapshot every 5,000 ms runs
/// five arena seconds and is stopped; its snapshot at 5,000 ms is decoded
/// against the fixture's ground, flushed as the world flushes it, and handed to
/// `resume_arena`, on tokio's paused clock:
///
/// - **the first pulse is a lattice point away, never at once**: nothing leaves
///   the task for 999 ms of its own base, and its first clock reads 6,000 — the
///   pulse an uninterrupted task would have taken next — not the 5,000 an
///   immediate pulse on an unanchored arena would publish;
/// - **the period the snapshot stands in is already snapshotted**
///   (`last_snapshot_period = now_ms / snapshot_ms`): the first `Snapshot` the
///   resumed task sends is the next period's, at 10,000 ms, and none falls in
///   the snapshot's own period. A task that forgot it would snapshot at 6,000
///   ms, and the head's `at_ms` would move off the moment the live record says
///   it resumed from — the resume-once rule's memory — at every resume.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn af3_a_resumed_task_pulses_on_the_lattice_and_skips_the_snapshot_s_own_period() {
    let seed = SEEDS[27].0;
    // The pushed-out pass and tick fall past the whole window (see `no_passes`).
    for unit in [RED, BLUE] {
        assert!(tacsim::arena::rng::stagger_ms(seed, unit, 600_000) > 20_000);
    }
    let mut arena_cfg = arena_at(1_000);
    arena_cfg.snapshot_ms = 5_000;
    let (handle, mut rx) = start_unseated(seed, &arena_cfg, &no_passes());
    let messages = collect(&mut rx, Duration::from_millis(5_050)).await;
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");
    let document = snapshots(&messages)
        .into_iter()
        .map(|(_, snapshot)| snapshot)
        .find(|snapshot| snapshot.at_ms == 5_000)
        .map(|snapshot| snapshot.document.clone())
        .expect("the five-second snapshot");
    let mut arena = tacsim::arena::Snapshot::decode(&document, definition())
        .expect("the snapshot decodes against its ground");
    assert_eq!(arena.now_ms, 5_000);
    arena.ledger.mark_flushed(arena.ledger.last_closed_seq());

    let (egress_tx, mut rx) = mpsc::channel(256);
    let (_handle, _join) = resume_arena(
        &tokio::runtime::Handle::current(),
        MatchId(seed),
        arena,
        &arena_cfg,
        &std::collections::BTreeMap::new(),
        pool(),
        egress_tx,
    )
    .expect("the resumed arena installs its sealed runners");
    assert!(
        tokio::time::timeout(Duration::from_millis(999), rx.recv())
            .await
            .is_err(),
        "a resumed task pulsed before its first lattice point"
    );
    let messages = collect(&mut rx, Duration::from_millis(11_050)).await;
    let first = messages.iter().find_map(clock_of);
    assert_eq!(
        first,
        Some(6_000),
        "the first resumed pulse is the lattice point after the snapshot's moment"
    );
    let taken: Vec<i64> = snapshots(&messages)
        .iter()
        .map(|(_, snapshot)| snapshot.at_ms)
        .collect();
    assert_eq!(
        taken,
        vec![10_000, 15_000],
        "the snapshot's own period is not snapshotted again; the next ones are"
    );
}

// ---------------------------------------------------------------------------
// (m) 17b cut M1b.1 — the floor captured at check-out
// ---------------------------------------------------------------------------

/// **The Compact floor is captured at check-out and survives a resume** (17b
/// cut M1b.1; doc 17 §5, D-UM-17 as amended: *a policy change never touches a
/// sortie*). A match spawned through [`spawn_arena`] with `[tacsim.minds]`
/// saying 50 per mille carries 50 in every snapshot it sends; the decoded
/// arena takes an arming at 50 and refuses one at 40 `unlawful`. Resumed
/// through [`resume_arena`] — which takes no minds block at all, so a server
/// whose `[tacsim.minds]` now says 100 has no way to reach it — the next
/// snapshot still carries 50, and the resumed match's snapshot at 15,000 has
/// the uninterrupted match's state hash.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m1b1_the_floor_is_captured_at_check_out_and_survives_a_resume() {
    use tacsim::arena::conditions::{ASPECTS_ANY, Arming, FireCondition, SlotTarget};
    use tacsim::arena::state::WeaponSummary;
    use tacsim::arena::weapons::WeaponBand;
    let seed = SEEDS[28].0;
    let mut arena_cfg = arena_at(1_000);
    arena_cfg.snapshot_ms = 5_000;
    let gun = WeaponSummary {
        id: SmolStr::new("cannon"),
        max_range_cm: 250_000,
        cycle_ms: 4_000,
        ammunition: SmolStr::new("tacsim.ammunition.he_105"),
        burst_rounds: 1,
        muzzle_speed_cms: 145_000,
        bands: vec![WeaponBand {
            max_range_cm: 250_000,
            dispersion_mrad: 3,
        }],
        count_per_load: 40,
        damage: 1,
        mount_section: SmolStr::new("turret"),
    };
    let start = |floor: u32| {
        let mut custody = unseated_custody(seed);
        if let Some(unit) = custody.units.iter_mut().find(|unit| unit.id == RED) {
            unit.weapons = vec![gun.clone()];
        }
        let terms = tacsim::match_::SealTerms {
            odds_floor_permille: floor,
            ..tacsim::match_::SealTerms::fixture()
        };
        for unit in &mut custody.units {
            unit.reseal(&terms).expect("the fixture terms seal");
        }
        let (egress_tx, egress_rx) = mpsc::channel(256);
        let (handle, _join, _callsigns) = spawn_arena(
            &tokio::runtime::Handle::current(),
            MatchId(seed),
            custody,
            &arena_cfg,
            &MatchConfig::default(),
            &no_passes(),
            &tacsim::config::MindsConfig {
                odds_floor_permille: floor,
                ..tacsim::config::MindsConfig::default()
            },
            &std::collections::BTreeMap::new(),
            pool(),
            egress_tx,
        )
        .expect("the test check-out is accepted");
        (handle, egress_rx)
    };
    let snapshot_at = |messages: &[ArenaOut], at_ms: i64| {
        snapshots(messages)
            .into_iter()
            .map(|(_, snapshot)| snapshot)
            .find(|snapshot| snapshot.at_ms == at_ms)
            .map(|snapshot| (snapshot.document.clone(), snapshot.state_hash))
            .unwrap_or_else(|| panic!("the {at_ms} ms snapshot"))
    };
    let arming = |min_permille: u16| {
        Arming::Arm(FireCondition {
            weapon: SmolStr::new("cannon"),
            target: SlotTarget::Lock,
            min_permille,
            aspects: ASPECTS_ANY,
            min_rounds_left: 0,
            min_range_m: 0,
            max_range_m: u32::MAX,
        })
    };

    // The match checked out at 50.
    let (handle, mut rx) = start(50);
    let messages = collect(&mut rx, Duration::from_millis(5_050)).await;
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");
    let (document, _) = snapshot_at(&messages, 5_000);
    let mut arena = tacsim::arena::Snapshot::decode(&document, definition())
        .expect("the snapshot decodes against its ground");
    assert_eq!(arena.odds_floor_permille, 50, "captured at check-out");
    let mut probe = arena.clone();
    assert_eq!(probe.resolve_arming(RED, &arming(50), 5_000), Ok(Some(1)));
    assert_eq!(
        probe.resolve_arming(RED, &arming(40), 5_000),
        Err(Refusal::Unlawful)
    );

    // Resumed on a server whose config would now say 100: nothing reaches it.
    arena.ledger.mark_flushed(arena.ledger.last_closed_seq());
    let (egress_tx, mut rx) = mpsc::channel(256);
    let (_resumed, _join) = resume_arena(
        &tokio::runtime::Handle::current(),
        MatchId(seed),
        arena,
        &arena_cfg,
        &std::collections::BTreeMap::new(),
        pool(),
        egress_tx,
    )
    .expect("the resumed arena installs its sealed runners");
    let messages = collect(&mut rx, Duration::from_millis(10_050)).await;
    let (document, resumed_hash) = snapshot_at(&messages, 15_000);
    let resumed = tacsim::arena::Snapshot::decode(&document, definition())
        .expect("the resumed snapshot decodes");
    assert_eq!(resumed.odds_floor_permille, 50, "the resume kept its own");
    let mut probe = resumed.clone();
    assert_eq!(
        probe.resolve_arming(RED, &arming(40), 15_000),
        Err(Refusal::Unlawful)
    );

    // The uninterrupted match, to the same moment.
    let (handle, mut rx) = start(50);
    let messages = collect(&mut rx, Duration::from_millis(15_050)).await;
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");
    let (_, uninterrupted_hash) = snapshot_at(&messages, 15_000);
    assert_eq!(
        resumed_hash, uninterrupted_hash,
        "the resumed match is the uninterrupted one at 15,000"
    );
}

// ---------------------------------------------------------------------------
// 17b cut M1b.5 — a card change through the task (doc 17 §11, §22's M1b)
// ---------------------------------------------------------------------------

/// The change `@mind/live <unit>=<preset>` offers.
fn m1b5_preset(unit: ObjId, name: &str) -> ArenaMsg {
    ArenaMsg::MindCard {
        unit,
        change: tacsim::arena::CardChange::Preset(SmolStr::new(name)),
    }
}

/// **A card change is held to the intent reserve and refused by a stopped
/// task** (17b cut M1b.5), as a `TempoFloor` is: with the inbox down to its
/// reserve a `MindCard` is handed back while a control, a stop and an ack
/// still fit; and once the task has stopped (its inbox closed) the offer is
/// handed back too — the verb's `That match has ended.`.
#[tokio::test(flavor = "current_thread")]
async fn m1b5_a_card_change_is_held_to_the_reserve_and_refused_by_a_stopped_task() {
    let seed = SEEDS[29].0;
    let (handle, join, _rx) = start_with_egress(seed, &fast_arena(), &MatchConfig::default(), 256);
    let mut taken = 0_u64;
    while handle
        .offer(order(RED, IntentKind::Startup, taken + 1))
        .is_ok()
    {
        taken += 1;
        assert!(taken < 1_000, "the inbox never refused an order");
    }
    assert!(
        matches!(
            handle.offer(m1b5_preset(RED, "Sniper")),
            Err(ArenaMsg::MindCard { unit, .. }) if unit == RED
        ),
        "a card change took the reserve"
    );
    assert!(handle.offer(ArenaMsg::Stop(StopReason::Abort)).is_ok());
    tokio::time::timeout(Duration::from_secs(5), join)
        .await
        .expect("the task ends")
        .expect("the task did not panic");
    assert!(!handle.is_live());
    assert!(
        handle.offer(m1b5_preset(RED, "Sniper")).is_err(),
        "a stopped match takes no card change"
    );
}

/// **Two card changes offered back to back land in arrival order** (17b cut
/// M1b.5): the task buffers them per unit in the order they came and stages
/// them before one step, whose phase 2 applies both at one arena millisecond —
/// `Sniper`, then `min_odds 70` on top of it (`Sniper, edited (min_odds 70)`)
/// — and a change for a seated hull is refused `not_mind` on the tape. On ONE
/// thread, so all three offers are in the inbox before the task's first poll
/// (its first pulse is due at once): the biased select drains them before its
/// due sleep fires, and "one pulse" cannot be split by a worker racing the
/// offers.
#[tokio::test(flavor = "current_thread")]
async fn m1b5_two_card_changes_land_in_arrival_order() {
    let seed = SEEDS[30].0;
    let mut custody = custody(seed);
    if let Some(unit) = custody.units.iter_mut().find(|unit| unit.id == RED) {
        unit.controller = tacsim::arena::state::Controller::Mind;
        unit.reseal(&tacsim::match_::SealTerms::fixture())
            .expect("the fixture terms seal");
    }
    custody.pilots.retain(|pilot| pilot.unit != RED);
    let (egress_tx, mut rx) = mpsc::channel(256);
    let (handle, _join, _callsigns) = spawn_arena(
        &tokio::runtime::Handle::current(),
        MatchId(seed),
        custody,
        &fast_arena(),
        &MatchConfig::default(),
        &SensorsConfig::default(),
        &tacsim::config::MindsConfig::default(),
        &std::collections::BTreeMap::new(),
        pool(),
        egress_tx,
    )
    .expect("the test check-out is accepted");
    for message in [
        m1b5_preset(RED, "Sniper"),
        ArenaMsg::MindCard {
            unit: RED,
            change: tacsim::arena::CardChange::Set {
                slider: SmolStr::new("min_odds"),
                value: SmolStr::new("70"),
            },
        },
        m1b5_preset(BLUE, "Sniper"),
    ] {
        assert!(handle.offer(message).is_ok(), "the inbox takes a change");
    }
    let messages = collect(&mut rx, Duration::from_millis(1_500)).await;
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");
    let cards: Vec<(i64, String)> = events(&messages)
        .into_iter()
        .filter_map(|event| match event.kind {
            EventKind::MindCard { unit, name, .. } if unit == RED => {
                Some((event.t_ms, name.to_string()))
            }
            EventKind::MindCardRefused { unit, reason } => {
                Some((event.t_ms, format!("#{} refused {reason}", unit.0)))
            }
            _ => None,
        })
        .collect();
    let at = cards.first().map(|(t_ms, _)| *t_ms).expect("a card line");
    assert_eq!(
        cards,
        vec![
            (at, "Sniper".to_owned()),
            (at, "Sniper, edited (min_odds 70)".to_owned()),
            (at, format!("#{} refused not_mind", BLUE.0)),
        ],
        "arrival order, one pulse"
    );
}

// ---------------------------------------------------------------------------
// (n) 17b cut M3.1 — the class table captured at check-out
// ---------------------------------------------------------------------------

/// One match of `seed` with red flown by a mind, spawned through
/// [`spawn_arena`] under `classes`, stopped after its 5,000 ms snapshot:
/// that snapshot decoded — red's `think_seq` there and the decoded arena's
/// own class table.
async fn mind_match_to_five_seconds(
    seed: u64,
    arena_cfg: &ArenaConfig,
    classes: tacsim::arena::core::ClassTable,
) -> (u64, tacsim::arena::core::ClassTable) {
    let mut custody = unseated_custody(seed);
    if let Some(unit) = custody.units.iter_mut().find(|unit| unit.id == RED) {
        unit.controller = tacsim::arena::state::Controller::Mind;
        unit.reseal(&tacsim::match_::SealTerms::fixture())
            .expect("the fixture terms seal");
    }
    let (egress_tx, mut rx) = mpsc::channel(256);
    let (handle, _join, _callsigns) = spawn_arena(
        &tokio::runtime::Handle::current(),
        MatchId(seed),
        custody,
        arena_cfg,
        &MatchConfig::default(),
        &no_passes(),
        &tacsim::config::MindsConfig {
            classes,
            ..tacsim::config::MindsConfig::default()
        },
        &std::collections::BTreeMap::new(),
        pool(),
        egress_tx,
    )
    .expect("the test check-out is accepted");
    let messages = collect(&mut rx, Duration::from_millis(5_050)).await;
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");
    let document = snapshots(&messages)
        .into_iter()
        .map(|(_, snapshot)| snapshot)
        .find(|snapshot| snapshot.at_ms == 5_000)
        .map(|snapshot| snapshot.document.clone())
        .expect("the 5,000 ms snapshot");
    let arena = tacsim::arena::Snapshot::decode(&document, definition())
        .expect("the snapshot decodes against its ground");
    let think_seq = arena
        .minds
        .get(&RED)
        .map(|row| row.think_seq)
        .expect("red's mind row");
    (think_seq, arena.classes)
}

/// **`[tacsim.minds].classes` is captured at check-out** (17b cut M3.1; doc
/// 17 §9, D-UM-35): a match spawned through [`spawn_arena`] with red a mind
/// (Class I: its core and Standard Doctrine's rating) and a table whose
/// Class I `think_ms` is 500 has thought more often by its 5,000 ms snapshot
/// than the same match spawned on the stock table — every think after the
/// first reads the captured row (`Arena::new` laid the first on the stock
/// grid). **Since 17b cut M3.2 the snapshot carries the table**
/// (`SNAPSHOT_FORMAT` 8): its decode hands back the table the check-out
/// captured — 17b cut M3.1 pinned the opposite (the decode read
/// `ClassTable::STOCK`, the declared gap of `Arena::classes`) under the name
/// `m3_the_class_table_is_captured_at_check_out_and_resumes_stock`, and this
/// cut inverts that last assertion and renames the test.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m3_the_class_table_is_captured_at_check_out_and_carried_by_the_resume() {
    use tacsim::arena::core::{ClassTable, CoreClass};
    let seed = SEEDS[31].0;
    let mut arena_cfg = arena_at(1_000);
    arena_cfg.snapshot_ms = 5_000;
    let quick: ClassTable = serde_json::from_value(serde_json::json!({ "I": { "think_ms": 500 } }))
        .expect("a one-figure table");
    assert_eq!(quick.row(CoreClass::I).think_ms, 500);
    assert_ne!(quick, ClassTable::STOCK);

    let (stock_thinks, stock_table) =
        mind_match_to_five_seconds(seed, &arena_cfg, ClassTable::STOCK).await;
    let (quick_thinks, quick_table) = mind_match_to_five_seconds(seed, &arena_cfg, quick).await;
    assert!(stock_thinks >= 4, "the stock mind thought: {stock_thinks}");
    assert!(
        quick_thinks >= stock_thinks + 3,
        "the captured row's 500 ms: {quick_thinks} thinks against {stock_thinks}"
    );
    assert_eq!(stock_table, ClassTable::STOCK);
    assert_eq!(
        quick_table, quick,
        "a resume reads the table the check-out captured (17b cut M3.2)"
    );
}

/// **The production entry points install each scripted seal's runner, or
/// refuse the match by name** (17b cut M3.5; doc 17 §13). Red is checked out
/// a mind's and sealed `scripted` (`SealTerms::engine_for_minds`) on Standard
/// Doctrine's source; the world hands the task the sources keyed by each
/// SEAL's digest (`arena_bridge`'s `sealed_sources`):
///
/// * `spawn_arena` with no source for the seal is refused
///   `SpawnError::Mind(InstallError::NoSource)` naming red and its seal's
///   digest, and so is one handed the source under its own `source_sha256`
///   (the wrong key) — nothing is spawned;
/// * with the stock source under the seal's digest the match opens;
/// * a snapshot of that arena decodes with `Arena::runners` EMPTY, and
///   `resume_arena` is refused the same way without the source, and resumes
///   with it.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m3_spawn_and_resume_install_each_scripted_seal_s_runner_or_refuse() {
    use tacsim::arena::mind::SealEngine;
    use tacsim::arena_task::SpawnError;
    use tacsim::minds::{InstallError, STANDARD_RHAI};
    let seed = SEEDS[32].0;
    let arena_cfg = fast_arena();
    let mut custody = unseated_custody(seed);
    let terms = tacsim::match_::SealTerms {
        engine_for_minds: SealEngine::Scripted,
        ..tacsim::match_::SealTerms::fixture()
    };
    for unit in custody.units.iter_mut().filter(|unit| unit.id == RED) {
        unit.controller = tacsim::arena::state::Controller::Mind;
        unit.reseal(&terms).expect("the fixture terms seal");
    }
    let seal = custody
        .units
        .iter()
        .find(|unit| unit.id == RED)
        .map(|unit| unit.seal.clone())
        .expect("red's seal");
    assert_eq!(seal.engine, SealEngine::Scripted);
    assert_ne!(seal.digest(), seal.source_sha256, "two different keys");
    let refusal = InstallError::NoSource {
        unit: RED,
        digest: seal.digest(),
    };
    let sources = BTreeMap::from([(seal.digest(), STANDARD_RHAI.to_owned())]);
    let wrong_key = BTreeMap::from([(seal.source_sha256, STANDARD_RHAI.to_owned())]);
    let spawn = |sources: &BTreeMap<[u8; 32], String>| {
        let (egress_tx, _egress_rx) = mpsc::channel(256);
        spawn_arena(
            &tokio::runtime::Handle::current(),
            MatchId(seed),
            custody.clone(),
            &arena_cfg,
            &MatchConfig::default(),
            &SensorsConfig::default(),
            &tacsim::config::MindsConfig::default(),
            sources,
            pool(),
            egress_tx,
        )
        .map(|(handle, _join, _callsigns)| handle)
    };
    for refused in [&BTreeMap::new(), &wrong_key] {
        assert_eq!(
            spawn(refused).err(),
            Some(SpawnError::Mind(refusal.clone())),
            "no runner, no match"
        );
    }
    let handle = spawn(&sources).expect("the scripted seal's source opens the match");
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");

    let arena = Arena::new(
        custody,
        &arena_cfg,
        &MatchConfig::default(),
        &SensorsConfig::default(),
    )
    .expect("the check-out is accepted");
    let snapshot = tacsim::arena::Snapshot::encode(&arena).expect("the arena encodes");
    let decoded = || {
        let mut arena = tacsim::arena::Snapshot::decode(&snapshot.document, definition())
            .expect("the snapshot decodes against its ground");
        arena.ledger.mark_flushed(arena.ledger.last_closed_seq());
        arena
    };
    assert!(decoded().runners.is_empty(), "the decode installs nothing");
    let resume = |sources: &BTreeMap<[u8; 32], String>| {
        let (egress_tx, _egress_rx) = mpsc::channel(256);
        resume_arena(
            &tokio::runtime::Handle::current(),
            MatchId(seed),
            decoded(),
            &arena_cfg,
            sources,
            pool(),
            egress_tx,
        )
        .map(|(handle, _join)| handle)
    };
    assert_eq!(
        resume(&BTreeMap::new()).err(),
        Some(refusal),
        "no runner, no resume"
    );
    let handle = resume(&sources).expect("the scripted seal's source resumes the match");
    handle
        .offer(ArenaMsg::Stop(StopReason::Abort))
        .expect("the inbox takes a stop");
}

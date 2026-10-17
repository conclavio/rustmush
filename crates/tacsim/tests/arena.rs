//! **The arena's determinism, end to end through `Arena::step`** (16b cut A.2,
//! A7; doc 16 §3.1 "Rate independence" and "Determinism", §3.4, §8's *cross-tempo
//! property test*).
//!
//! Ten tests, driving the library directly — no host, no store, no task, which
//! is the whole point of doc 16 §3.1's *the arena core is a library*:
//!
//! - **(a)** one scripted control and intent log, keyed by **arena** time,
//!   replayed at 5,000, 1,000 and 500 ms pulse cadences, producing one ordered
//!   event list and one per-substep hash sequence — including the stop at an
//!   impassable band and the stop against an occupied cell;
//! - **(b)** a tempo change mid-run changes no outcome, and its other half: two
//!   runs at two *different* tempos produce the same motion and the same events;
//! - **(c)** adding a draw label shifts no existing draw;
//! - **(d)** `STARTUP` then a released seat in one burst never starts;
//! - **(e)** a unit at its ceiling covers the same ground however the time is cut
//!   into pulses;
//! - **(f)** `match.max_duration_ms` ends the match, and a further step does
//!   nothing;
//! - **(g)** the window's edge stops a unit with one notice;
//! - **(g2)** 16b cut F.2: the **pilot's** own notice per stop, once, naming which
//!   of doc 16 §3.4 step 4's three refusals it was and never the cell's occupant;
//! - **(g3)** and the condition it is read off survives a substep in which the
//!   tank, still ordered forward, covered no whole centimetre — the pulse
//!   remainder that every pulse in production ends on — while the per-substep
//!   `blocked` flag is off, and the pilot's frame carries the
//!   condition;
//! - **(g4)** 16b cut M.3: one stop against a bank is one `blocked` line with its
//!   cause and one `Held:` line however the pulse is cut and however the tank
//!   turns against it, a second obstacle in the same cell is a second stop, and
//!   reversing off the bank clears the condition inside the cell;
//! - **(a1b)** 16b cut I.3's staff floor: one script at three floors is one
//!   event list and one hash sequence bar the one `tempo_floor` line's period, a
//!   jittered floored run is the clean one, and the task's own wakes under a
//!   floor are the floor apart in wall time and the clamp apart in arena time.
//! - **(af1)** 16b cut AF.1's snapshot: a snapshot after every pulse decoded
//!   and resumed is the uninterrupted run, byte for byte, at three cadences,
//!   under the task's own wakes, across a drop and a reclaim and in a squadded
//!   arena, its squad signals and its ledger's running digests included; every
//!   refusal of the decode by name; the encoding deterministic and integer; and
//!   its shape pinned over two unstepped fixtures.
//!
//! # Why the tempo is pinned, and pinned to `Quiet`
//!
//! `Clock::pin_tempo_for_tests` holds the tempo while the **cadence** varies,
//! which is the property under test: *the tempo decides only how often `step` runs
//! and never an outcome* (D-16-2). It is pinned to `Quiet` because the lag clamp
//! is `max_catch_up_steps × period` of the *current* tempo (doc 16 §3.1 "Clock and
//! lag"), so an arena pinned to `Engaged` and stepped every 5,000 ms would be
//! lagging by construction — and a `lagged` event is a per-**pulse** event, which
//! is exactly the kind of thing the comparison must not be measuring.
//!
//! That is the one honest caveat on "identical per-substep hashes": the hash folds
//! the ledger's event sequence, so an event appended at a *pulse* boundary rather
//! than a substep boundary shifts every later hash. The two such events are
//! `tempo` and `lagged`, and both are artefacts of the cadence itself. Hold them
//! out — pin the tempo, stay inside the clamp — and the remaining substep
//! sequence is identical, which is the claim doc 16 makes.

use std::collections::BTreeMap;

use mush_core::{DefinitionDigest, DefinitionId, DefinitionKey, DefinitionKind, ObjId, OwnerId};
use mush_sim::data::Pose;
use mush_tactics::{
    CellSpec, DefinitionRef, FacetId, FacetSpec, RulesValue, SectionId, SectionSpec, TacticalUnit,
    UnitSpec,
};
use smol_str::SmolStr;
use tacsim::arena::clock::Tempo;
use tacsim::arena::intents::{Control, ControlState, Intent, IntentKind};
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena::motion::BlockedCause;
use tacsim::arena::motor::Refusal;
use tacsim::arena::rng::{DrawLabel, draw};
use tacsim::arena::snapshot::{SNAPSHOT_FORMAT, Snapshot, SnapshotError};
use tacsim::arena::state::{
    Arena, ArenaDefinition, ArenaError, ArenaTerrain, Channel, CustodyOut, CustodyUnit, EndReason,
    EntryZone, Pilot, Power, StopReason, Team, TerrainCell, TrackedRules,
};
use tacsim::arena::step::{Frame, StepOutput};
use tacsim::config::{ArenaConfig, MatchConfig, SensorsConfig};

// ---------------------------------------------------------------------------
// Fixtures
//
// `tacsim::arena`'s own `test_support` is `pub(crate)` and invisible here, and
// the crate's panic budget forbids a shared helper file under `src/`, so an
// integration test builds its own. They are the same shapes, deliberately: the
// 8 by 8 window of the unit tests is too small for a unit to reach a ceiling and
// then cross five cells.
// ---------------------------------------------------------------------------

/// The window: 64 by 64 cells of 1,000 cm, which is 640 m square.
const SIDE: u16 = 64;

/// The cell size, centimetres.
const CELL_CM: u16 = 1_000;

/// The impassable band's column (doc 16 §3.4 step 4's *impassable for `tracked`*).
const BAND_X: u16 = 20;

/// An exact definition identity with a zero digest.
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

/// An empty opaque rules blob.
fn rules() -> RulesValue {
    RulesValue::new(dref("schema", "tacsim.arena.test"), serde_json::json!({}))
        .expect("an empty rules blob")
}

/// The smallest M32 unit the arena can carry.
fn tactical_unit(id: ObjId) -> TacticalUnit {
    tactical_unit_ruled(id, rules())
}

/// [`tactical_unit`]'s one shape under any rules blob — **16b cut AE.3** reads
/// a declared class off it (`UnitClass::read`), so a fixture can field a drone.
fn tactical_unit_ruled(id: ObjId, rules: RulesValue) -> TacticalUnit {
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
        rules,
    };
    TacticalUnit::from_spec(&spec, id).expect("a one-section unit")
}

/// Passable ground at cost 100.
fn ground() -> TerrainCell {
    TerrainCell {
        class: mush_atlas::TerrainClass::Ground,
        elev_cm: 0,
        move_cost: 100,
    }
}

/// A pose at `(x_cm, y_cm)` facing north.
fn pose(x_cm: i64, y_cm: i64) -> Pose {
    Pose::at(x_cm, y_cm, 0, rules())
}

/// The cell centre of `(x, y)`.
fn centre(x: u16, y: u16) -> Pose {
    pose(
        i64::from(x) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
        i64::from(y) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
    )
}

/// The 64 by 64 window with one impassable column at [`BAND_X`].
fn definition() -> ArenaDefinition {
    let mut cells = vec![ground(); usize::from(SIDE) * usize::from(SIDE)];
    for y in 0..SIDE {
        if let Some(cell) = cells.get_mut(usize::from(y) * usize::from(SIDE) + usize::from(BAND_X))
        {
            cell.move_cost = 0;
        }
    }
    ArenaDefinition {
        id: SmolStr::new("tacsim.arena.test"),
        terrain: ArenaTerrain::new(SIDE, SIDE, CELL_CM, cells).expect("the test window"),
        entry_zones: vec![
            EntryZone {
                team: Team::Red,
                x0: 0,
                y0: 0,
                x1: 3,
                y1: 63,
            },
            EntryZone {
                team: Team::Blue,
                x0: 60,
                y0: 0,
                x1: 63,
                y1: 63,
            },
        ],
        // 16b cut N.1: no authored furniture on this fixture window — the
        // reflex controller is not what these tests are about, and every unit
        // on them is a `Pilot`.
        key_spots: Vec::new(),
        waypoints: tacsim::arena::state::Waypoints::default(),
        modes: Vec::new(),
    }
}

/// A stock tank: 1,000 cm/s forward, 40% in reverse, 200/400 cm/s², 30 degrees a
/// second.
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
        // 16b cut W.2: cut A's arena presents nothing and is struck by nothing.
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

/// An arena over [`definition`] with seed 2350 and one seated pilot per unit.
fn arena_of(units: Vec<CustodyUnit>, match_cfg: &MatchConfig) -> Arena {
    let pilots = units
        .iter()
        .enumerate()
        .map(|(seat, unit)| Pilot {
            unit: unit.id,
            pilot: ObjId(1_000 + u32::try_from(seat).unwrap_or(0)),
            station: SmolStr::new("cockpit"),
            epoch: 1,
        })
        .collect();
    Arena::new(
        CustodyOut {
            definition: definition(),
            units,
            pilots,
            resolution: tacsim::match_::stock_resolution(),
            seed: 2_350,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &ArenaConfig::default(),
        match_cfg,
        &SensorsConfig::default(),
    )
    .expect("the test arena")
}

/// The mover, and the parked tank that is nothing but an obstacle.
const MOVER: ObjId = ObjId(4);
const PARKED: ObjId = ObjId(9);

/// One order, on the channel its kind travels on.
fn intent(unit: ObjId, kind: IntentKind, seq: u64) -> Intent {
    Intent {
        unit,
        channel: kind.channel_required(),
        epoch: 1,
        kind,
        seq,
    }
}

// ---------------------------------------------------------------------------
// The replay driver
// ---------------------------------------------------------------------------

/// One entry of a scripted log, keyed by **arena** time.
///
/// It is delivered in the first pulse whose arena time has reached `t_ms`, which
/// is what makes one script mean the same thing at three cadences: every `t_ms`
/// below is a multiple of 5,000 ms, and a pulse boundary at a multiple of 5,000 ms
/// exists at all three cadences, so the order lands at the same arena time in
/// every replay.
struct Beat {
    t_ms: i64,
    controls: Vec<Control>,
    intents: Vec<Intent>,
    /// **The world's `Stop`, delivered at this beat** (16b cut D.3): the message
    /// doc 16 §3.1 has the world send, which the arena honours at the next
    /// pulse's phase 1 and never mid-substep. `None` on every other beat, which
    /// is what [`Beat::at`] gives.
    stop: Option<StopReason>,
    /// **A unit struck at this beat** (16b cut Y.4), before the pulse steps:
    /// [`wound`]'s hull hit, written straight onto the unit's M32 cells. `None`
    /// on every beat but the Y.3/Y.4 script's `Released`. These fixture tanks
    /// carry no gun and no sensor, so nothing in the match can land a round;
    /// the cells a round would have written are what the catch-up report reads,
    /// and they are folded into no state hash (the opaque unit is absent from
    /// it, `hash.rs`'s own argument), so the write moves no hash and no event.
    wound: Option<ObjId>,
    /// **A unit wrecked at this beat** (16b cut AE.3), before the pulse steps:
    /// `Arena::destroy_unit` at the beat's arena time, the rig's own direct
    /// write (`step.rs`'s elimination backstop names it). These fixtures carry
    /// no gun, so nothing in the match can wreck anything; what the AE.3
    /// script needs is a side left with only its drone, at one arena time at
    /// every cadence. `None` on every other beat.
    wreck: Option<ObjId>,
}

impl Beat {
    /// A beat with no stop on it — the shape every script but cut D.3's uses.
    fn at(t_ms: i64, controls: Vec<Control>, intents: Vec<Intent>) -> Beat {
        Beat {
            t_ms,
            controls,
            intents,
            stop: None,
            wound: None,
            wreck: None,
        }
    }

    /// A beat that is nothing but the world asking the arena to stop.
    fn stopping(t_ms: i64, stop: StopReason) -> Beat {
        Beat {
            t_ms,
            controls: Vec::new(),
            intents: Vec::new(),
            stop: Some(stop),
            wound: None,
            wreck: None,
        }
    }
}

/// **The hull hit a [`Beat::wound`] writes** (16b cut Y.4): six of the front
/// facet's ten armour and three of the hull's eight internals, through
/// `mush_tactics::apply` — the one function every cell write goes through.
fn wound(arena: &mut Arena, unit: ObjId) {
    use mush_tactics::{CellAddr, CellId, UnitMutationOp};
    let Some(state) = arena.unit_mut(unit) else {
        return;
    };
    let plan = mush_tactics::UnitMutationPlan::new(
        vec![
            UnitMutationOp::AdjustCell {
                addr: CellAddr::Facet {
                    section: SectionId::new("hull").expect("a section"),
                    facet: FacetId::new("front").expect("a facet"),
                    cell: CellId::new("armor").expect("a cell"),
                },
                delta: -6,
            },
            UnitMutationOp::AdjustCell {
                addr: CellAddr::Section {
                    section: SectionId::new("hull").expect("a section"),
                    cell: CellId::new("internals").expect("a cell"),
                },
                delta: -3,
            },
        ],
        None,
        None,
    )
    .expect("a plan");
    let (post, _) = mush_tactics::apply(&state.unit, &plan, 0).expect("the hit applies");
    state.unit = post;
}

/// One ledger line, normalised for comparison: its arena timestamp, its kind and
/// fields as JSON, the draw where one was made, and the substep hash. The `seq` is
/// dropped because it is the one field a different number of pulses could
/// legitimately renumber.
type Line = (i64, String, Vec<(String, u64)>, u64);

/// The same line with the hash column struck out — what two runs at two different
/// tempos can be compared on, the hash folding the tempo byte by design.
type Bare = (i64, String, Vec<(String, u64)>);

/// One unit as a replay left it: `(unit, x_cm, y_cm, z_cm, heading_mdeg, speed_cms,
/// blocked, linkdead)`.
type Final = (ObjId, i64, i64, i32, u32, i32, bool, bool);

/// What one replay produced: the ledger, the per-substep hashes, the final poses,
/// and the shape of the run.
struct Replay {
    /// **The tape itself** (16b cut AF.1): every byte of every chunk the run
    /// was handed, in order — what [`Replay::events`] is parsed from, kept so a
    /// resumed run can be held to the uninterrupted one byte for byte.
    tape: Vec<u8>,
    /// Every ledger line, normalised.
    events: Vec<Line>,
    /// The state hash of every substep of the whole run, in order.
    hashes: Vec<u64>,
    /// Each unit's final pose, speed and condition, in unit-id order.
    ///
    /// `blocked` and `linkdead` are carried here because
    /// [`tacsim::arena::hash::state_hash`] deliberately folds neither (A.1's
    /// design: the hash is the *quantised world*, and those two are conditions the
    /// ledger records the edges of), so without them a replay that ended with a
    /// unit still held against the band and one that ended with it free would
    /// compare equal.
    finals: Vec<Final>,
    /// How many pulses the replay ran, and how many substeps in total.
    pulses: u32,
    substeps: u32,
    /// **16b cut M.3**: every notice line the mover's pilot was sent, in order —
    /// what the three cadences must also agree on now that the `blocked` line and
    /// the pilot's `Held:` fire on one edge.
    told: Vec<String>,
    /// **16b cut Y.3**: one row per pulse — the arena time the pulse started
    /// at, whether it built the mover's pilot a frame, and whether the mover had
    /// a reflex row once it had stepped. What a drop and a reclaim are read
    /// off: the frames stop and the machine holds the tank between them.
    pulses_seen: Vec<(i64, bool, bool)>,
    /// **16b cut Y.4**: every catch-up report a frame carried to the mover's
    /// pilot, with the arena time of the pulse that built it.
    catchups: Vec<(i64, tacsim::arena::Catchup)>,
    /// **16b cut I.4**: the arena time of every staff picture a pulse built,
    /// in order — empty unless the run was watched.
    observers: Vec<i64>,
    /// **16b cut AF.1**: every squad signal a pulse handed the world
    /// (`StepOutput::deliberation`), in order — the one output the squad watch
    /// and the expired set steer, which neither the tape nor the hash sees.
    /// Empty unless the arena has reflex squads.
    deliberation: Vec<tacsim::arena::SquadSignal>,
}

impl Replay {
    fn kinds(&self) -> Vec<&str> {
        self.events
            .iter()
            .map(|(_, kind, _, _)| kind.as_str())
            .collect()
    }

    fn count(&self, kind: &str) -> usize {
        self.events
            .iter()
            .filter(|(_, encoded, _, _)| encoded.contains(kind))
            .count()
    }
}

/// Drive one arena through `script` at `cadence_ms` until arena time reaches
/// `until_ms`, with the tempo held at whatever `pin` says at that arena time.
fn replay(
    arena: &mut Arena,
    cadence_ms: i64,
    until_ms: i64,
    script: &[Beat],
    pin: &dyn Fn(i64) -> Tempo,
) -> Replay {
    replay_on(
        arena,
        &mut (0_i64..).map(|pulse| pulse.saturating_mul(cadence_ms)),
        until_ms,
        script,
        pin,
    )
}

/// **The same driver over any sequence of wall readings** (16b cut I.3): the
/// opening pulse at the first, and one pulse at each after it, until arena
/// time reaches `until_ms` or the readings run out. [`replay`] is this over
/// `0, cadence, 2 × cadence, …`; the floor's jittered run hands it odd
/// overshoots and an extra wake.
///
/// **16b cut AF.1**: [`replay_on_from`] from the script's first beat, then
/// [`finish_replay`] — the flush that closes the open chunk, which a resume
/// test must not do before it encodes.
fn replay_on(
    arena: &mut Arena,
    walls: &mut dyn Iterator<Item = i64>,
    until_ms: i64,
    script: &[Beat],
    pin: &dyn Fn(i64) -> Tempo,
) -> Replay {
    let (mut replay, _) = replay_on_from(arena, walls, until_ms, script, 0, pin);
    finish_replay(arena, &mut replay);
    replay
}

/// **[`replay_on`] from a script cursor, without the flush** (16b cut AF.1).
///
/// The cursor is the index of the first beat not yet delivered, and the answer
/// carries the one it reached: a resumed segment handed the whole script from
/// `0` would deliver again every beat its first segment consumed (a second
/// `wound`, a stale-epoch control twice), so every resume test hands the second
/// segment the first one's cursor. And it does **not** flush: a first segment
/// that closed its open chunk before the snapshot would carry an empty
/// `LedgerState.open` at every resume point, and on the two-tank script the
/// whole tape is under one chunk, so that buffer is exactly the state a resume
/// has to carry. [`Replay::tape`] and [`Replay::events`] are the bytes the
/// pulses handed out so far; [`finish_replay`] adds the rest.
///
/// **17b cut M4.5a**: each pulse is [`Arena::step_settled`] — the step, then
/// the thinks it handed out answered and written at the sim-time moment the
/// next step's head would write them — so a mind row read, hashed or encoded
/// between two pulses (every resume here encodes after this) is the row the
/// arena task leaves between two of its own. On a field with no mind it is
/// [`Arena::step`] exactly; on one with minds the tape and the hashes are the
/// unsettled step's.
fn replay_on_from(
    arena: &mut Arena,
    walls: &mut dyn Iterator<Item = i64>,
    until_ms: i64,
    script: &[Beat],
    from: usize,
    pin: &dyn Fn(i64) -> Tempo,
) -> (Replay, usize) {
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let Some(mut wall) = walls.next() else {
        panic!("a replay needs at least its opening wall reading");
    };
    let mut next = from;
    let mut pulses = 0_u32;
    let mut substeps = 0_u32;
    let mut told: Vec<String> = Vec::new();
    let mut pulses_seen: Vec<(i64, bool, bool)> = Vec::new();
    let mut catchups: Vec<(i64, tacsim::arena::Catchup)> = Vec::new();
    let mut observers: Vec<i64> = Vec::new();
    let mut deliberation: Vec<tacsim::arena::SquadSignal> = Vec::new();
    loop {
        arena.clock.pin_tempo_for_tests(pin(arena.now_ms));
        let mut controls: Vec<Control> = Vec::new();
        let mut intents: Vec<Intent> = Vec::new();
        while let Some(beat) = script.get(next) {
            if beat.t_ms > arena.now_ms {
                break;
            }
            controls.extend(beat.controls.iter().copied());
            intents.extend(beat.intents.iter().cloned());
            // 16b cut D.3: the world's `Stop`, armed before this pulse steps.
            if let Some(stop) = beat.stop {
                arena.stop(stop);
            }
            // 16b cut Y.4: a hit, before this pulse steps.
            if let Some(unit) = beat.wound {
                wound(arena, unit);
            }
            // 16b cut AE.3: a wreck, before this pulse steps, at the beat's
            // own arena time.
            if let Some(unit) = beat.wreck {
                let at = arena.now_ms;
                let mut journal = Vec::new();
                arena.destroy_unit(unit, at, None, &mut journal);
            }
            next += 1;
        }
        let started = arena.now_ms;
        let out = arena.step_settled(wall, &controls, &intents);
        absorb(&out, &mut bytes);
        hashes.extend(arena.substep_hashes.iter().copied());
        pulses += 1;
        substeps += out.substeps;
        told.extend(heard(&out.frames, MOVER_PILOT));
        pulses_seen.push(seen(arena, started, &out));
        catchups.extend(reported(started, &out));
        observers.extend(out.observer.as_ref().map(|picture| picture.now_ms));
        deliberation.extend(out.deliberation.iter().cloned());
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        let Some(reading) = walls.next() else {
            break;
        };
        wall = reading;
    }
    (
        Replay {
            events: lines(&bytes),
            tape: bytes,
            hashes,
            finals: finals_of(arena),
            pulses,
            substeps,
            told,
            pulses_seen,
            catchups,
            observers,
            deliberation,
        },
        next,
    )
}

/// Each unit's final pose, speed and condition, in unit-id order.
fn finals_of(arena: &Arena) -> Vec<Final> {
    arena
        .units
        .iter()
        .map(|unit| {
            (
                unit.id,
                unit.pose.x_cm,
                unit.pose.y_cm,
                unit.pose.z_cm,
                unit.pose.heading_mdeg,
                unit.speed_cms,
                unit.blocked,
                unit.linkdead,
            )
        })
        .collect()
}

/// **The end of a run** (16b cut AF.1): flush the ledger, as the end of a match
/// does, add the chunks it hands out to the tape, and read the events and the
/// finals again — what [`replay_on`]'s tail always did, for a segment
/// [`replay_on_from`] left open.
fn finish_replay(arena: &mut Arena, replay: &mut Replay) {
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        replay.tape.extend_from_slice(&chunk.bytes);
    }
    replay.events = lines(&replay.tape);
    replay.finals = finals_of(arena);
}

/// One [`Replay::pulses_seen`] row: the pulse's start, whether the mover's
/// pilot was built a frame, and whether the mover is a machine's now (17b
/// cut M4.3: either machine's — a release hands it to a mind).
fn seen(arena: &Arena, started: i64, out: &StepOutput) -> (i64, bool, bool) {
    (
        started,
        out.frames.iter().any(|(pilot, _)| *pilot == MOVER_PILOT),
        arena
            .unit(MOVER)
            .is_some_and(|unit| unit.controller.is_machine()),
    )
}

/// **16b cut Y.4**: the catch-up reports one pulse's frames carried to the
/// mover's pilot, with the pulse's start.
fn reported(started: i64, out: &StepOutput) -> Vec<(i64, tacsim::arena::Catchup)> {
    out.frames
        .iter()
        .filter(|(pilot, _)| *pilot == MOVER_PILOT)
        .filter_map(|(_, frame)| frame.catchup.clone().map(|report| (started, report)))
        .collect()
}

fn absorb(out: &StepOutput, bytes: &mut Vec<u8>) {
    for chunk in &out.ledger_chunks {
        bytes.extend_from_slice(&chunk.bytes);
    }
}

/// Parse the streamed ledger into the comparable form.
fn lines(bytes: &[u8]) -> Vec<Line> {
    String::from_utf8(bytes.to_vec())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<LedgerEvent>(line).ok())
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.draws,
                event.hash,
            )
        })
        .collect()
}

/// The script both (a) and (b) replay: start up cold, drive north into a parked
/// tank, then turn east and drive into the impassable band.
///
/// `STARTUP` is at arena time 0 — the opening pulse, whose `dt` is zero at every
/// cadence — so the completion it schedules lands at exactly `match.startup_ms`
/// in every replay. The two helm orders are at 10,000 and 25,000 ms, both pulse
/// boundaries at all three cadences and both after power is online. The parked
/// tank is north and the band is east, so the log drives into the occupied cell
/// first and the impassable one second.
///
/// The `Control` at 40,000 ms is the seat going **linkdead** while the tank is
/// under way: doc 16 §3.1's honest and dangerous answer is that the tank keeps its
/// continuous orders, so a linkdead tank drives on and hits the band by itself,
/// at the same arena time at every cadence.
///
/// **16b cut M.3** added the 50,000 ms beat (16c `T-04`, `B-27`): the linkdead
/// tank is hard against the band by then, and a heading order 30° off square to
/// it — north of east, so the standing `SPEED 1000` keeps pushing it at the bank
/// and the refusals creep it along the bank while it turns — is the shape of the
/// live smoke's stop that was said twice. A heading is a continuous order, which a
/// linkdead seat still gives. The band stays one `blocked` line and one `Held:`
/// line at every cadence. (North and not south: the tank met the band thirteen
/// centimetres into its row, and a creep south would cross into the row below
/// and be refused a *different* cell, which is a second stop by the rule and not
/// what this script is for; `m3_a_tank_turning_against_a_bank_…` drives the
/// southward creep in the middle of a cell.)
fn script() -> Vec<Beat> {
    vec![
        Beat::at(0, Vec::new(), vec![intent(MOVER, IntentKind::Startup, 1)]),
        Beat::at(
            10_000,
            Vec::new(),
            vec![
                intent(MOVER, IntentKind::Heading { mdeg: 0 }, 2),
                intent(MOVER, IntentKind::Speed { cms: 1_000 }, 3),
            ],
        ),
        Beat::at(
            25_000,
            Vec::new(),
            vec![
                intent(MOVER, IntentKind::Heading { mdeg: 90_000 }, 4),
                intent(MOVER, IntentKind::Speed { cms: 1_000 }, 5),
            ],
        ),
        Beat::at(
            40_000,
            vec![Control {
                unit: MOVER,
                channel: Channel::Helm,
                epoch: 1,
                state: ControlState::Linkdead,
                pilot: None,
            }],
            Vec::new(),
        ),
        Beat::at(
            50_000,
            Vec::new(),
            vec![intent(MOVER, IntentKind::Heading { mdeg: 60_000 }, 6)],
        ),
    ]
}

/// The two units the script drives: a mover in column 5 and a parked tank seven
/// cells north of it, which is an obstacle and nothing else (its power is never
/// brought up, so it cannot move).
fn scripted_arena() -> Arena {
    arena_of(
        vec![
            tank(MOVER, Team::Red, centre(5, 5)),
            tank(PARKED, Team::Blue, centre(5, 12)),
        ],
        &MatchConfig::default(),
    )
}

// ---------------------------------------------------------------------------
// (a) the cross-tempo property test
// ---------------------------------------------------------------------------

#[test]
fn one_scripted_log_at_three_cadences_is_one_event_list_and_one_hash_sequence() {
    let quiet = |_: i64| Tempo::Quiet;
    let mut runs: Vec<(i64, Replay)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = scripted_arena();
        let run = replay(&mut arena, cadence, 60_000, &script(), &quiet);
        assert!(
            arena.unit(MOVER).is_some_and(|unit| unit.linkdead),
            "the scripted control did not reach the arena at {cadence} ms"
        );
        assert_eq!(
            arena.now_ms, 60_000,
            "cadence {cadence} ran to a different time"
        );
        runs.push((cadence, run));
    }

    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };

    // The run is worth asserting before it is compared: a script that did nothing
    // would compare equal three times and prove nothing.
    assert_eq!(
        first.count("startup"),
        1,
        "the log did not start the tank up: {:?}",
        first.kinds()
    );
    // **16b cut M.3 changed this count from four to two, at all three cadences
    // alike** (16c `T-04`; cut E.3 had made it four). The parked tank is one stop
    // again: a traffic refusal is a *hold* that keeps the mover's way and matures
    // into the stop two seconds later, and the standing `SPEED 1000` then
    // re-accelerates it into a second hold and a third — but each of those
    // matures into a stop at the same cell for the same cause, which is no new
    // edge of `held` and journals nothing. The band is one stop, and stays one
    // through the 50,000 ms turn against it. Each line names its cause.
    assert_eq!(
        first.count("\"blocked\""),
        2,
        "the log did not stop twice — once at the parked tank, once at the band: {:?}",
        first.kinds()
    );
    let causes: Vec<&str> = first
        .events
        .iter()
        .filter(|(_, kind, _, _)| kind.contains("\"blocked\""))
        .map(|(_, kind, _, _)| {
            if kind.contains("\"cause\":\"occupied\"") {
                "occupied"
            } else if kind.contains("\"cause\":\"impassable\"") {
                "impassable"
            } else {
                kind.as_str()
            }
        })
        .collect();
    assert_eq!(causes, vec!["occupied", "impassable"]);
    // And the pilot heard the two stops and nothing more about them.
    let held: Vec<&String> = first
        .told
        .iter()
        .filter(|line| line.starts_with("Held:"))
        .collect();
    assert_eq!(
        held,
        vec![
            "Held: something is in the way.",
            "Held: the ground ahead will not take the tank."
        ],
        "{:?}",
        first.told
    );
    assert_eq!(first.count("position_summary"), 12);
    assert_eq!(first.count("match_started"), 1);
    assert!(first.hashes.len() >= 240, "{} substeps", first.hashes.len());
    // The mover ended hard against the band's west edge, having crossed fifteen
    // cells to get there, and the parked tank never moved.
    let mover = first
        .finals
        .iter()
        .find(|(id, ..)| *id == MOVER)
        .copied()
        .expect("the mover");
    assert_eq!(mover.1, 19_999, "the mover is not at the band's boundary");
    assert_eq!(mover.5, 0, "the mover is not stopped");
    // 16b cut M.3: turned 30° off the band by the 50,000 ms beat, and still
    // pushed at it.
    assert_eq!(mover.4, 60_000, "the mover did not turn against the band");
    let parked = first
        .finals
        .iter()
        .find(|(id, ..)| *id == PARKED)
        .copied()
        .expect("the parked tank");
    assert_eq!((parked.1, parked.2, parked.5), (5_500, 12_500, 0));

    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events,
            first.events,
            "the event list differs at {cadence} ms: {:?} against {:?}",
            run.kinds(),
            first.kinds()
        );
        assert_eq!(
            run.hashes,
            first.hashes,
            "the per-substep hash sequence differs at {cadence} ms ({} against {})",
            run.hashes.len(),
            first.hashes.len()
        );
        assert_eq!(
            run.finals, first.finals,
            "the final state differs at {cadence} ms"
        );
        assert_eq!(
            run.substeps, first.substeps,
            "a different number of substeps ran at {cadence} ms"
        );
        // 16b cut M.3: and one notice list — the `Held:` lines come off the same
        // edge the `blocked` lines do.
        assert_eq!(
            run.told, first.told,
            "the pilot was told something different at {cadence} ms"
        );
        // And the cadence really did differ: the pulse counts are the proof that
        // the three runs were not the same run.
        assert_ne!(run.pulses, first.pulses, "{cadence} ms ran the same pulses");
    }
}

/// **16b cut I.4's mandatory cross-tempo test** (16b §11; doc 16 §3.1 "Rate
/// independence", D-16-2; 16c N10): a watched match is the same match. The (a)
/// script with `Arena::watched` set before the run, against the same script
/// unwatched, at 5,000, 1,000 and 500 ms: one event list, one per-substep hash
/// sequence, one set of finals and one notice list — the picture is a view
/// built in phase 5 that no hash folds and nothing that steers reads.
///
/// And the watched run's picture count is the number of distinct arena
/// seconds its pulses landed in, `until_ms / max(cadence, 1_000) + 1`: the
/// driver steps once at wall 0, so the opening pulse builds one for second 0,
/// and at 500 ms the pulse at 500 is in that same second. The unwatched run
/// builds none.
#[test]
fn i4_a_watched_match_is_the_same_match() {
    const UNTIL_MS: i64 = 60_000;
    let quiet = |_: i64| Tempo::Quiet;
    let mut watched_runs: Vec<(i64, Replay)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut plain = scripted_arena();
        let unwatched = replay(&mut plain, cadence, UNTIL_MS, &script(), &quiet);
        let mut seen = scripted_arena();
        seen.watched = true;
        let watched = replay(&mut seen, cadence, UNTIL_MS, &script(), &quiet);

        assert!(
            unwatched.observers.is_empty(),
            "an unwatched run built a picture at {cadence} ms"
        );
        let expected = usize::try_from(UNTIL_MS / cadence.max(1_000) + 1).expect("a count");
        assert_eq!(
            watched.observers.len(),
            expected,
            "at {cadence} ms the watched run built {:?}",
            watched.observers
        );
        let seconds: Vec<i64> = watched
            .observers
            .iter()
            .map(|now_ms| now_ms.div_euclid(1_000))
            .collect();
        assert!(
            seconds.windows(2).all(|pair| pair[0] < pair[1]),
            "two pictures in one arena second at {cadence} ms: {:?}",
            watched.observers
        );

        assert_eq!(
            watched.events,
            unwatched.events,
            "watching changed the event list at {cadence} ms: {:?} against {:?}",
            watched.kinds(),
            unwatched.kinds()
        );
        assert_eq!(
            watched.hashes, unwatched.hashes,
            "watching changed the hash sequence at {cadence} ms"
        );
        assert_eq!(watched.finals, unwatched.finals, "{cadence} ms");
        assert_eq!(watched.told, unwatched.told, "{cadence} ms");
        assert_eq!(watched.substeps, unwatched.substeps, "{cadence} ms");
        watched_runs.push((cadence, watched));
    }
    // And the three watched runs are one match, as the three unwatched are.
    let Some((_, first)) = watched_runs.first() else {
        panic!("three watched runs");
    };
    assert!(first.count("startup") == 1, "{:?}", first.kinds());
    for (cadence, run) in watched_runs.iter().skip(1) {
        assert_eq!(run.events, first.events, "{cadence} ms");
        assert_eq!(run.hashes, first.hashes, "{cadence} ms");
        assert_eq!(run.finals, first.finals, "{cadence} ms");
    }
}

// ---------------------------------------------------------------------------
// (a1) the task's own wake rule: 16b cut U.1 (16c `T-22`, Q25)
// ---------------------------------------------------------------------------

/// **The fourth driver: the loop the task runs** (16b cut U.1). The three
/// drivers above step at a fixed cadence; production steps at
/// [`tacsim::arena::step::StepOutput::next_due_ms`], which since this cut is
/// the earlier of the next period multiple and the first substep boundary at or
/// after the next due event, in absolute arena time. This driver does exactly
/// that — `wall = out.next_due_ms` after every pulse, the opening pulse at
/// `wall = 0` — with the tempo pinned `Quiet` and the `Quiet` period set to
/// `period_ms`, and delivers each beat at the first wake at or after its `t_ms`,
/// which is production's semantics and, every beat being a multiple of 5,000 ms
/// and the period term absolute, a wake exactly on the beat.
///
/// `Quiet` and not the tempo each period belongs to: the state hash folds the
/// tempo byte, so an `Alert` or `Engaged` run could never hash-equal the
/// `Quiet` fixed-cadence replay it is compared with. The period is changed on
/// the arena's own frozen `arena_cfg.tempo.quiet_ms`, which `next_due_ms`
/// reads, before the opening pulse.
///
/// Answers the replay and every arena millisecond it pulsed at after the
/// opening pulse.
fn replay_as_the_task(
    arena: &mut Arena,
    period_ms: u32,
    until_ms: i64,
    script: &[Beat],
) -> (Replay, Vec<i64>) {
    let (replay, woke_at, _) = replay_as_the_task_floored(arena, period_ms, until_ms, script, None);
    (replay, woke_at)
}

/// **[`replay_as_the_task`] under a staff floor** (16b cut I.3): `floor` is set
/// on the clock before the opening pulse, as an `ArenaMsg::TempoFloor` that
/// arrived before the task's first wake would set it, and the third answer is
/// every WALL reading the task woke at, the opening pulse's included — under a
/// floor `next_due_ms` answers a wall interval, and the gaps between these are
/// what that is asserted on.
fn replay_as_the_task_floored(
    arena: &mut Arena,
    period_ms: u32,
    until_ms: i64,
    script: &[Beat],
    floor: Option<u32>,
) -> (Replay, Vec<i64>, Vec<i64>) {
    let (mut replay, woke_at, walls, _) =
        replay_as_the_task_from(arena, period_ms, until_ms, script, 0, floor, 0);
    finish_replay(arena, &mut replay);
    (replay, woke_at, walls)
}

/// **[`replay_as_the_task_floored`] from a script cursor and a wall, without
/// the flush** (16b cut AF.1): [`replay_on_from`]'s shape for the task's own
/// wake rule. `wall0` is the wall the first pulse steps at — `0` for a match's
/// opening pulse, and for a resumed segment the deadline
/// [`Arena::resume_at`] answered. The fourth answer is the cursor it reached.
fn replay_as_the_task_from(
    arena: &mut Arena,
    period_ms: u32,
    until_ms: i64,
    script: &[Beat],
    from: usize,
    floor: Option<u32>,
    wall0: i64,
) -> (Replay, Vec<i64>, Vec<i64>, usize) {
    arena.arena_cfg.tempo.quiet_ms = period_ms;
    arena.clock.set_floor(floor);
    let mut walls: Vec<i64> = Vec::new();
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = wall0;
    let mut next = from;
    let mut pulses = 0_u32;
    let mut substeps = 0_u32;
    let mut told: Vec<String> = Vec::new();
    let mut woke_at: Vec<i64> = Vec::new();
    let mut pulses_seen: Vec<(i64, bool, bool)> = Vec::new();
    let mut catchups: Vec<(i64, tacsim::arena::Catchup)> = Vec::new();
    let mut observers: Vec<i64> = Vec::new();
    let mut deliberation: Vec<tacsim::arena::SquadSignal> = Vec::new();
    loop {
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let mut controls: Vec<Control> = Vec::new();
        let mut intents: Vec<Intent> = Vec::new();
        while let Some(beat) = script.get(next) {
            if beat.t_ms > arena.now_ms {
                break;
            }
            controls.extend(beat.controls.iter().copied());
            intents.extend(beat.intents.iter().cloned());
            if let Some(stop) = beat.stop {
                arena.stop(stop);
            }
            if let Some(unit) = beat.wound {
                wound(arena, unit);
            }
            next += 1;
        }
        // Every pulse but a match's opening one is a wake; a resumed segment's
        // first pulse is one (16b cut AF.1).
        if pulses > 0 || wall0 != 0 {
            woke_at.push(arena.now_ms);
        }
        walls.push(wall);
        let started = arena.now_ms;
        let out = arena.step(wall, &controls, &intents);
        absorb(&out, &mut bytes);
        hashes.extend(arena.substep_hashes.iter().copied());
        pulses += 1;
        substeps += out.substeps;
        told.extend(heard(&out.frames, MOVER_PILOT));
        pulses_seen.push(seen(arena, started, &out));
        catchups.extend(reported(started, &out));
        observers.extend(out.observer.as_ref().map(|picture| picture.now_ms));
        deliberation.extend(out.deliberation.iter().cloned());
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        assert!(
            out.next_due_ms > wall,
            "the task was asked to wake in the past"
        );
        wall = out.next_due_ms;
    }
    let replay = Replay {
        events: lines(&bytes),
        tape: bytes,
        hashes,
        finals: finals_of(arena),
        pulses,
        substeps,
        told,
        pulses_seen,
        catchups,
        observers,
        deliberation,
    };
    (replay, woke_at, walls, next)
}

/// **16b cut U.1's mandatory cross-tempo test** (16b §11; doc 16 §3.1 "Rate
/// independence", D-16-2): the (a) script driven the way the task drives it,
/// waking at `next_due_ms` with the `Quiet` period at 5,000, 1,000 and 500 ms,
/// is one event list, one per-substep hash sequence, one set of finals and one
/// list of notices — and all three are the fixed-cadence replay's at 5,000 ms.
/// Every wake after the opening pulse is on the substep lattice.
#[test]
fn the_task_s_own_wakes_at_three_periods_are_the_fixed_cadence_replay() {
    let quiet = |_: i64| Tempo::Quiet;
    let mut fixed_arena = scripted_arena();
    let fixed = replay(&mut fixed_arena, 5_000, 60_000, &script(), &quiet);
    assert_eq!(fixed.count("\"blocked\""), 2, "the fixed replay is (a)'s");
    let substep = i64::from(tacsim::arena::clock::substep_ms(
        &fixed_arena.arena_cfg.tempo,
        fixed_arena.cell_cm(),
        fixed_arena.max_speed_cms(),
    ));
    assert_eq!(substep, 250, "the fixture's lattice");
    let mut pulse_counts: Vec<u32> = Vec::new();
    for period in [5_000_u32, 1_000, 500] {
        let mut arena = scripted_arena();
        let (run, woke_at) = replay_as_the_task(&mut arena, period, 60_000, &script());
        assert_eq!(
            arena.now_ms, 60_000,
            "period {period} ran to a different time"
        );
        assert!(
            arena.unit(MOVER).is_some_and(|unit| unit.linkdead),
            "the scripted control did not reach the arena at a {period} ms period"
        );
        // Every wake is a lattice point: the task never ends a pulse off it.
        let off: Vec<i64> = woke_at
            .iter()
            .copied()
            .filter(|now| now % substep != 0)
            .collect();
        assert!(
            off.is_empty(),
            "the task woke off the {substep} ms lattice at a {period} ms period: {off:?}"
        );
        // And the period term is absolute: every multiple of it is a wake.
        for multiple in (1..60_000 / i64::from(period)).map(|k| k * i64::from(period)) {
            assert!(
                woke_at.contains(&multiple),
                "no wake at {multiple}, a multiple of the {period} ms period"
            );
        }
        assert_eq!(
            run.events,
            fixed.events,
            "the event list differs at a {period} ms period: {:?} against {:?}",
            run.kinds(),
            fixed.kinds()
        );
        assert_eq!(
            run.hashes,
            fixed.hashes,
            "the per-substep hash sequence differs at a {period} ms period ({} against {})",
            run.hashes.len(),
            fixed.hashes.len()
        );
        assert_eq!(
            run.finals, fixed.finals,
            "the final state differs at a {period} ms period"
        );
        assert_eq!(
            run.told, fixed.told,
            "the pilot was told something different at a {period} ms period"
        );
        pulse_counts.push(run.pulses);
    }
    // The drivers really were different: the task's wakes are not the fixed
    // cadence's pulses.
    assert!(
        pulse_counts.iter().all(|pulses| *pulses != fixed.pulses),
        "a task driver ran the fixed replay's pulses: {pulse_counts:?} against {}",
        fixed.pulses
    );
}

// ---------------------------------------------------------------------------
// (a1b) the staff floor: 16b cut I.3 (16c N10)
// ---------------------------------------------------------------------------

/// **The floor's own script** (16b cut I.3): every beat on the 2,000 ms
/// lattice, so it lands at one arena millisecond whether a floored wake moves
/// 2,000, 1,000 or 500 ms of arena time. The mover starts up cold at 0, drives
/// north into the parked tank from 10,000, turns east at 24,000, and its seat is
/// `Released` at 40,000 — which hands it to its sealed mind (the reflex
/// controller until 17b cut M4.3; re-pointed by 17b cut M5.7), so the
/// machine's thinks and searches run under the floor too.
fn floor_script() -> Vec<Beat> {
    vec![
        Beat::at(0, Vec::new(), vec![intent(MOVER, IntentKind::Startup, 1)]),
        Beat::at(
            10_000,
            Vec::new(),
            vec![
                intent(MOVER, IntentKind::Heading { mdeg: 0 }, 2),
                intent(MOVER, IntentKind::Speed { cms: 1_000 }, 3),
            ],
        ),
        Beat::at(
            24_000,
            Vec::new(),
            vec![intent(MOVER, IntentKind::Heading { mdeg: 90_000 }, 4)],
        ),
        Beat::at(
            40_000,
            vec![Control {
                unit: MOVER,
                channel: Channel::Helm,
                epoch: 1,
                state: ControlState::Released,
                pilot: Some(MOVER_PILOT),
            }],
            Vec::new(),
        ),
    ]
}

/// A replay's events with the one `tempo_floor` line's `period_ms` struck out,
/// and that line's arena time and period — what three runs at three floors are
/// compared on.
fn floor_normalised(run: &Replay) -> (Vec<Line>, Vec<(i64, String)>) {
    let mut floors: Vec<(i64, String)> = Vec::new();
    let events = run
        .events
        .iter()
        .map(|(t_ms, kind, draws, hash)| {
            if kind.contains("\"kind\":\"tempo_floor\"") {
                floors.push((*t_ms, kind.clone()));
                (*t_ms, "tempo_floor".to_owned(), draws.clone(), *hash)
            } else {
                (*t_ms, kind.clone(), draws.clone(), *hash)
            }
        })
        .collect();
    (events, floors)
}

/// **16b cut I.3's mandatory cross-tempo test, the floor** (16b §11; doc 16
/// §3.1 "Rate independence", D-16-2; 16c N10): one scripted log at three
/// FLOORS — 5,000, 1,000 and 500 ms, each with the driver's cadence equal to it
/// and the tempo pinned `Engaged` (a 2,000 ms clamp, so the 5,000 ms floor is
/// slow motion at 2,000 ms of arena time a wake) — is one event list, one
/// per-substep hash sequence and one final state, with exactly one `tempo_floor`
/// line at `t_ms` 0 in each (its `period_ms` the one difference) and no `lagged`
/// line anywhere: a slowed match is not a lagging one.
///
/// Floored runs are compared with each other and never with an unfloored one:
/// the floor is not hashed, but its line moves the ledger's sequence, which
/// every later hash folds (`hash.rs` item 11).
#[test]
fn i3_a_tempo_floor_moves_no_outcome_at_three_floors() {
    let engaged = |_: i64| Tempo::Engaged;
    let mut runs: Vec<(u32, Replay)> = Vec::new();
    for floor in [5_000_u32, 1_000, 500] {
        let mut arena = scripted_arena();
        arena.clock.set_floor(Some(floor));
        let run = replay(
            &mut arena,
            i64::from(floor),
            60_000,
            &floor_script(),
            &engaged,
        );
        assert_eq!(
            arena.now_ms, 60_000,
            "floor {floor} ran to a different time"
        );
        // Every pulse moved `min(2,000, floor)` of arena time, on the lattice.
        let moved = i64::from(floor.min(2_000));
        assert!(
            run.pulses_seen
                .iter()
                .all(|(started, ..)| started % moved == 0),
            "floor {floor}: {:?}",
            run.pulses_seen
        );
        runs.push((floor, run));
    }
    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };
    // Worth comparing: the tank started, drove into the parked tank, turned,
    // and was handed to the machine.
    assert_eq!(first.count("\"startup\""), 1, "{:?}", first.kinds());
    assert!(first.count("\"blocked\"") >= 1, "{:?}", first.kinds());
    assert!(
        first.pulses_seen.iter().any(|(_, _, machine)| *machine),
        "the released seat never reached the machine"
    );
    let (first_events, _) = floor_normalised(first);
    for (floor, run) in &runs {
        let (events, floors) = floor_normalised(run);
        assert_eq!(
            floors,
            vec![(
                0,
                format!("{{\"kind\":\"tempo_floor\",\"period_ms\":{floor}}}")
            )],
            "floor {floor}: one line, at the opening pulse"
        );
        assert_eq!(run.count("\"lagged\""), 0, "floor {floor} journalled lag");
        assert_eq!(
            events, first_events,
            "the event list differs at a {floor} ms floor"
        );
        assert_eq!(
            run.hashes,
            first.hashes,
            "the per-substep hash sequence differs at a {floor} ms floor ({} against {})",
            run.hashes.len(),
            first.hashes.len()
        );
        assert_eq!(run.finals, first.finals, "floor {floor}: the finals");
        assert_eq!(run.told, first.told, "floor {floor}: the pilot's notices");
    }
    // And the floors really did differ: a 5,000 ms floor takes thirty wakes to
    // the minute, a 500 ms one a hundred and twenty.
    let pulses: Vec<u32> = runs.iter().map(|(_, run)| run.pulses).collect();
    assert_eq!(pulses, vec![31, 61, 121]);
}

/// **16b cut I.3: a jittered floor is the same match** (16c N10; the plan's
/// review F-R3/A-R1). The OS timer is never on time, so the driver's wall is
/// advanced by the floor plus an odd overshoot every wake — 3, 7, 1, 9, 5 ms in
/// turn — and one extra wake lands at wall 253 right after the floor was set,
/// the shape a `TempoFloor`'s pull-in has when staff type the verb at an odd
/// moment. Against the clean run at the same 1,000 ms floor: one event list, one
/// per-substep hash sequence, one final state — and every pulse of the jittered
/// run starts on the 250 ms substep lattice, the extra wake's included (it moves
/// the arena exactly one substep, 250, and the next regular wake takes it back
/// to the 1,000 grid). Without the quantisation every overshoot would be arena
/// time, and the beats would land off their millisecond.
#[test]
fn i3_a_jittered_floor_is_the_same_match() {
    let engaged = |_: i64| Tempo::Engaged;
    let mut clean_arena = scripted_arena();
    clean_arena.clock.set_floor(Some(1_000));
    let clean = replay(&mut clean_arena, 1_000, 60_000, &floor_script(), &engaged);

    let overshoots = [3_i64, 7, 1, 9, 5];
    let mut walls: Vec<i64> = vec![0, 253];
    let mut regular = 0_i64;
    for wake in 0..200_usize {
        regular += 1_000 + overshoots[wake % overshoots.len()];
        walls.push(regular);
    }
    let mut jittered_arena = scripted_arena();
    jittered_arena.clock.set_floor(Some(1_000));
    let jittered = replay_on(
        &mut jittered_arena,
        &mut walls.into_iter(),
        60_000,
        &floor_script(),
        &engaged,
    );
    assert_eq!(jittered_arena.now_ms, 60_000);
    let off: Vec<i64> = jittered
        .pulses_seen
        .iter()
        .map(|(started, ..)| *started)
        .filter(|started| started % 250 != 0)
        .collect();
    assert!(off.is_empty(), "pulses started off the lattice: {off:?}");
    assert!(
        jittered
            .pulses_seen
            .iter()
            .any(|(started, ..)| *started == 250),
        "the extra wake did not move the arena one substep"
    );
    assert_eq!(jittered.count("\"lagged\""), 0);
    assert_eq!(
        jittered.events,
        clean.events,
        "the event list differs: {:?} against {:?}",
        jittered.kinds(),
        clean.kinds()
    );
    assert_eq!(
        jittered.hashes,
        clean.hashes,
        "the per-substep hash sequence differs ({} against {})",
        jittered.hashes.len(),
        clean.hashes.len()
    );
    assert_eq!(jittered.finals, clean.finals);
    assert_eq!(jittered.told, clean.told);
    assert_eq!(jittered.pulses, clean.pulses + 1, "one extra wake");
}

/// **16b cut I.3: the task's own wakes under a floor** (16c N10). The task
/// driver, with the `Quiet` period at 500 ms (a 2,000 ms clamp) and a 5,000 ms
/// floor set before the opening pulse: `next_due_ms` answers a wall interval,
/// so every wake is exactly 5,000 wall ms after the last, and every wake moves
/// exactly 2,000 ms of arena time — slow motion at two fifths, on the lattice
/// throughout, with no `lagged` line. And it is the same match the fixed-cadence
/// driver played at a 5,000 ms floor: one event list bar the hash column (the
/// hash folds the tempo byte, and this driver pins `Quiet` where that one pins
/// `Engaged`) and one final state.
#[test]
fn i3_the_task_s_own_wakes_under_a_floor_are_the_floor_apart() {
    let (run, woke_at, walls) = {
        let mut arena = scripted_arena();
        let answer =
            replay_as_the_task_floored(&mut arena, 500, 60_000, &floor_script(), Some(5_000));
        assert_eq!(arena.now_ms, 60_000);
        answer
    };
    let gaps: Vec<i64> = walls.windows(2).map(|pair| pair[1] - pair[0]).collect();
    assert!(
        !gaps.is_empty() && gaps.iter().all(|gap| *gap == 5_000),
        "the wake gaps: {gaps:?}"
    );
    // `woke_at` is the arena time each wake after the opening pulse STARTED at
    // (the first is 0, the opening pulse having integrated nothing); the last
    // wake ended at 60,000.
    let mut arena_steps: Vec<i64> = woke_at.clone();
    arena_steps.push(60_000);
    let moves: Vec<i64> = arena_steps
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect();
    assert!(
        moves.iter().all(|moved| *moved == 2_000),
        "the arena moved {moves:?} a wake"
    );
    assert_eq!(run.count("\"lagged\""), 0);

    let mut fixed_arena = scripted_arena();
    fixed_arena.clock.set_floor(Some(5_000));
    let engaged = |_: i64| Tempo::Engaged;
    let fixed = replay(&mut fixed_arena, 5_000, 60_000, &floor_script(), &engaged);
    let bare = |replay: &Replay| -> Vec<Bare> {
        replay
            .events
            .iter()
            .map(|(t_ms, kind, draws, _)| (*t_ms, kind.clone(), draws.clone()))
            .collect()
    };
    assert_eq!(bare(&run), bare(&fixed));
    assert_eq!(run.finals, fixed.finals);
}

// ---------------------------------------------------------------------------
// (a2) a released seat mid-run: 16b cut F.1
// ---------------------------------------------------------------------------

/// The seated pilot of `MOVER`, as [`arena_of`] hands the roster over.
const MOVER_PILOT: ObjId = ObjId(1_000);

/// [`script`] with the 40,000 ms beat turned from `Linkdead` into **`Released`**,
/// naming the pilot who left (16b cut F.1).
fn released_script() -> Vec<Beat> {
    let mut beats = script();
    // 16b cut M.3: by its arena time, since the script no longer ends on it.
    if let Some(beat) = beats.iter_mut().find(|beat| beat.t_ms == 40_000) {
        beat.controls = vec![Control {
            unit: MOVER,
            channel: Channel::Helm,
            epoch: 1,
            state: ControlState::Released,
            pilot: Some(MOVER_PILOT),
        }];
    }
    beats
}

/// **A `Released` mid-run is rate-independent, and changes no other unit** (16b
/// cut F.1; doc 16 §3.1 "Rate independence", §3.6).
///
/// Cut F.1 made `Released` do two new things — take the pilot off the arena's
/// frame roster and forget their remembered frame — and both of them happen
/// inside `apply_control`, at a pulse boundary, off a message the world sends.
/// This is the proof that neither leaked into the model: one script at three
/// cadences is one event list, one per-substep hash sequence and one final state,
/// and the parked tank on the other side ends exactly where the `Linkdead` run
/// leaves it.
#[test]
fn a_released_seat_mid_run_is_one_event_list_at_three_cadences_and_moves_nobody_else() {
    let quiet = |_: i64| Tempo::Quiet;
    let mut runs: Vec<(i64, Replay)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = scripted_arena();
        assert_eq!(arena.pilots.len(), 2, "one seated pilot per unit");
        let run = replay(&mut arena, cadence, 60_000, &released_script(), &quiet);
        // The control reached the arena, and the pilot is off the roster.
        assert!(
            !arena.pilots.iter().any(|seat| seat.pilot == MOVER_PILOT),
            "the released pilot is still on the frame roster at {cadence} ms"
        );
        assert!(
            arena.pilots.iter().any(|seat| seat.unit == PARKED),
            "and the other tank's pilot is not, at {cadence} ms"
        );
        // And the arena builds them no further frame: one more pulse, and the
        // released pilot is not among the slots it addresses.
        let out = arena.step(60_000 + cadence, &[], &[]);
        assert!(
            !out.frames.iter().any(|(pilot, _)| *pilot == MOVER_PILOT),
            "a frame was built for a pilot who left, at {cadence} ms"
        );
        runs.push((cadence, run));
    }
    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };
    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events,
            first.events,
            "the event list differs at {cadence} ms: {:?} against {:?}",
            run.kinds(),
            first.kinds()
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the per-substep hash sequence differs at {cadence} ms"
        );
        assert_eq!(
            run.finals, first.finals,
            "the final state differs at {cadence} ms"
        );
        assert_ne!(run.pulses, first.pulses, "{cadence} ms ran the same pulses");
    }
    // **No other unit's outcome moved.** The reference is the same script with the
    // same beat as a `Linkdead`, which is the control that changes nothing about
    // who is flying: the parked tank ends identically either way.
    let mut reference = scripted_arena();
    let linkdead = replay(&mut reference, 1_000, 60_000, &script(), &quiet);
    let parked_of = |run: &Replay| {
        run.finals
            .iter()
            .find(|(id, ..)| *id == PARKED)
            .copied()
            .expect("the parked tank")
    };
    assert_eq!(
        parked_of(first),
        parked_of(&linkdead),
        "releasing one seat moved the other side's tank"
    );
}

// ---------------------------------------------------------------------------
// (a3) a dropped seat, reclaimed: 16b cut Y.3
// ---------------------------------------------------------------------------

/// The arena time of [`y3_reclaim_script`]'s linkdead onset.
const Y3_LINKDEAD_MS: i64 = 35_000;
/// ... of its release (the grace run out): [`released_script`]'s own beat.
const Y3_RELEASED_MS: i64 = 40_000;
/// ... of the reclaim.
const Y3_LIVE_MS: i64 = 55_000;
/// ... of the drop's `Released` again, arriving behind the reclaim: stale.
const Y3_STALE_MS: i64 = 60_000;
/// ... and where the replay stops.
const Y3_UNTIL_MS: i64 = 65_000;

/// One control per channel for the mover, naming its pilot.
fn y3_every_channel(epoch: u64, state: ControlState) -> Vec<Control> {
    Channel::ALL
        .iter()
        .map(|channel| Control {
            unit: MOVER,
            channel: *channel,
            epoch,
            state,
            pilot: Some(MOVER_PILOT),
        })
        .collect()
}

/// **[`released_script`] as a drop and a reclaim** (16b cut Y.3; 16c `N4`; doc
/// 07 §1.3): the whole of doc 16 §3.8's "Disconnects" in one log, every beat a
/// multiple of 5,000 ms.
///
/// - 35,000 ms: **`Linkdead` at epoch 2** on every channel, naming the pilot —
///   the line dropped and the grace is running. (16b cut Y.4 takes its catch-up
///   snapshot on this beat; it is scripted here so Y.4 adds assertions and no
///   beats.)
/// - 40,000 ms: `released_script`'s `Released`, now on every channel **at epoch
///   2** — the grace ran out. Not epoch 1 as `released_script` has it: that
///   would be below the `Linkdead`'s epoch, and `apply_control`'s stale gate
///   would make it change nothing. At the `Linkdead`'s own epoch it applies
///   (equal epochs do).
/// - 55,000 ms: **`Live` at epoch 3** on every channel, naming the pilot — the
///   pilot's `BOARD` past the grace (`arena_bridge::reseat`).
/// - 60,000 ms: the drop's **`Released` at epoch 2 again**, every channel,
///   naming the pilot — the race `apply_control`'s stale gate settles: a
///   release the task kept (`buffer_control` holds a displaced `Released`)
///   landing after the reclaim's `Live`. Below the channel's current epoch (3),
///   it purges and changes nothing: the pilot stays seated.
///
/// **16b cut Y.4** strikes the mover's hull on the 40,000 ms beat, before the
/// `Released` applies ([`Beat::wound`]): after the `Linkdead` took the catch-up
/// snapshot and before the grace ran out, so the report lists the hull only if
/// the baseline is the `Linkdead`'s. The write is to cells no hash folds, so
/// Y.3's event list and hash sequence stand.
fn y3_reclaim_script() -> Vec<Beat> {
    let mut beats = released_script();
    if let Some(beat) = beats.iter_mut().find(|beat| beat.t_ms == Y3_RELEASED_MS) {
        beat.controls = y3_every_channel(2, ControlState::Released);
        beat.wound = Some(MOVER);
    }
    let at = beats
        .iter()
        .position(|beat| beat.t_ms > Y3_LINKDEAD_MS)
        .unwrap_or(beats.len());
    beats.insert(
        at,
        Beat::at(
            Y3_LINKDEAD_MS,
            y3_every_channel(2, ControlState::Linkdead),
            Vec::new(),
        ),
    );
    beats.push(Beat::at(
        Y3_LIVE_MS,
        y3_every_channel(3, ControlState::Live),
        Vec::new(),
    ));
    beats.push(Beat::at(
        Y3_STALE_MS,
        y3_every_channel(2, ControlState::Released),
        Vec::new(),
    ));
    beats
}

/// **16b cut Y.3's mandatory cross-tempo test** (16b §11; doc 16 §3.1 "Rate
/// independence", D-16-2): a drop and a reclaim are one event list and one
/// per-substep hash sequence at 5,000, 1,000 and 500 ms.
///
/// Y.3 changed arena behaviour twice — `apply_control`'s stale gate and
/// `Arena::seat_pilot` on a `Live` that names a pilot — and both happen in
/// phase 1 off a message the world sends. So: the three replays agree; the
/// pilot's frames stop at the `Released` and resume at the `Live` (with a whole
/// keyframe); the machine holds the tank between the two and not after — not
/// even once the stale `Released` behind the reclaim has landed, which is the
/// stale gate's whole job; and the other side's tank ends exactly where
/// [`released_script`]'s run leaves it.
///
/// **16b cut Y.4's mandatory cross-tempo coverage** rides the same log: the
/// catch-up report is on the reclaiming pulse's frame and no other, it counts
/// from the `Linkdead` beat, it lists the hull struck between the `Linkdead`
/// and the `Released`, it is one report at every cadence — and the same log
/// with no pilot named and no hull struck is this run's hash sequence and
/// event list, so the report folds into nothing.
#[test]
fn y3_a_dropped_and_reclaimed_seat_is_one_event_list_and_one_hash_sequence_at_three_cadences() {
    let quiet = |_: i64| Tempo::Quiet;
    let mut runs: Vec<(i64, Replay)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = scripted_arena();
        let run = replay(
            &mut arena,
            cadence,
            Y3_UNTIL_MS,
            &y3_reclaim_script(),
            &quiet,
        );
        assert_eq!(
            arena.now_ms, Y3_UNTIL_MS,
            "cadence {cadence} ran to a different time"
        );
        // The pilot is back on the roster, once, and the tank is theirs.
        assert_eq!(
            arena
                .pilots
                .iter()
                .filter(|seat| seat.pilot == MOVER_PILOT && seat.unit == MOVER)
                .count(),
            1,
            "the reclaimed pilot is not on the frame roster once at {cadence} ms"
        );
        assert_eq!(
            arena.unit(MOVER).map(|unit| unit.controller),
            Some(tacsim::arena::state::Controller::Pilot),
            "the machine kept the tank at {cadence} ms"
        );
        assert!(
            arena.unit(MOVER).is_some_and(|unit| !unit.linkdead),
            "the reclaim left the seat linkdead at {cadence} ms"
        );
        // The frames stop after the `Released` and resume after the `Live`.
        let framed = |from: i64, to: i64| {
            run.pulses_seen
                .iter()
                .filter(|(started, framed, _)| *framed && (from..to).contains(started))
                .count()
        };
        assert!(
            framed(0, Y3_RELEASED_MS) > 0,
            "no frame before the drop at {cadence} ms"
        );
        assert_eq!(
            framed(Y3_RELEASED_MS, Y3_LIVE_MS),
            0,
            "a frame was built for a dropped pilot at {cadence} ms"
        );
        assert!(
            run.pulses_seen
                .iter()
                .any(|(started, framed, _)| *framed && *started == Y3_LIVE_MS),
            "the reclaiming pulse sent no frame at {cadence} ms"
        );
        // The stale `Released` did not unseat the reclaimed pilot: the frames
        // go on after it landed.
        assert!(
            framed(Y3_STALE_MS, Y3_UNTIL_MS) > 0,
            "the stale release behind the reclaim stopped the frames at {cadence} ms"
        );
        // The machine's row: absent before the drop, present from the
        // `Released` to the `Live`, gone from the `Live` on — the stale
        // `Released` at 60,000 ms included.
        for (started, _, machine) in &run.pulses_seen {
            let want = (Y3_RELEASED_MS..Y3_LIVE_MS).contains(started);
            assert_eq!(
                *machine, want,
                "the machine's hull at the pulse from {started} ms, at {cadence} ms"
            );
        }
        // **16b cut Y.4**: the catch-up rides the reclaiming pulse's frame and
        // no other — none before the `Live`, none after its first frame.
        assert_eq!(
            run.catchups.iter().map(|(at, _)| *at).collect::<Vec<i64>>(),
            vec![Y3_LIVE_MS],
            "one report, on the reclaim's own frame, at {cadence} ms"
        );
        assert!(
            arena.at_release.is_empty() && arena.pending_catchup.is_empty(),
            "the snapshot and the report are spent at {cadence} ms"
        );
        runs.push((cadence, run));
    }
    // **16b cut Y.4**: the report itself, read once — the three are compared
    // below. Counted from the `Linkdead` beat, not the `Released`; the hull
    // struck between the two is listed; the machine's words were read before
    // its row went.
    let report = runs
        .first()
        .and_then(|(_, run)| run.catchups.first())
        .map(|(_, report)| report.clone())
        .expect("the reclaim's report");
    assert_eq!(report.away_ms, Y3_LIVE_MS - Y3_LINKDEAD_MS);
    assert_eq!(
        report
            .sections
            .iter()
            .map(|(id, then, now)| (
                id.as_str(),
                then.armour,
                now.armour,
                then.internals,
                now.internals
            ))
            .collect::<Vec<_>>(),
        vec![("hull", 10, 4, 8, 5)],
        "the hull struck after the Linkdead's snapshot"
    );
    assert!(
        report.cell_then.is_some() && report.cell_now.is_some(),
        "{report:?}"
    );
    assert_eq!(
        (report.power_then, report.power_now),
        (Power::Online, Power::Online)
    );
    assert!(!report.wreck);
    assert!(report.contacts_new.is_empty() && report.contacts_lost.is_empty());
    assert!(
        report.rounds_spent.is_empty(),
        "the fixture tank carries no gun"
    );
    assert_eq!(
        report.machine.as_ref().map(|words| words.state.as_str()),
        Some("withdraw"),
        "a released tank is the machine's in Withdraw until the reclaim"
    );
    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.catchups,
            runs.first()
                .map(|(_, first)| first.catchups.clone())
                .unwrap_or_default(),
            "the report differs at {cadence} ms"
        );
    }
    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };
    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events,
            first.events,
            "the event list differs at {cadence} ms: {:?} against {:?}",
            run.kinds(),
            first.kinds()
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the per-substep hash sequence differs at {cadence} ms"
        );
        assert_eq!(
            run.finals, first.finals,
            "the final state differs at {cadence} ms"
        );
        assert_eq!(
            run.told, first.told,
            "the pilot was told something different at {cadence} ms"
        );
        assert_ne!(run.pulses, first.pulses, "{cadence} ms ran the same pulses");
    }
    // **No other unit's outcome moved**: the parked tank ends where the
    // released run (F.1's) leaves it.
    let mut reference = scripted_arena();
    let released = replay(
        &mut reference,
        1_000,
        Y3_UNTIL_MS,
        &released_script(),
        &quiet,
    );
    let parked_of = |run: &Replay| {
        run.finals
            .iter()
            .find(|(id, ..)| *id == PARKED)
            .copied()
            .expect("the parked tank")
    };
    assert_eq!(
        parked_of(first),
        parked_of(&released),
        "a drop and a reclaim moved the other side's tank"
    );
    // **16b cut Y.4: the report is a view.** The same log with no control
    // naming a pilot and no hull struck — which takes no snapshot, builds no
    // report and keeps the pilot on the frame roster throughout — is Y.3's
    // hash sequence and event list, and so is the run above.
    let unnamed: Vec<Beat> = y3_reclaim_script()
        .into_iter()
        .map(|beat| Beat {
            controls: beat
                .controls
                .iter()
                .map(|control| Control {
                    pilot: None,
                    ..*control
                })
                .collect(),
            wound: None,
            ..beat
        })
        .collect();
    let mut plain = scripted_arena();
    let plain = replay(&mut plain, 5_000, Y3_UNTIL_MS, &unnamed, &quiet);
    assert!(plain.catchups.is_empty(), "no pilot named, no report");
    assert_eq!(
        plain.hashes, first.hashes,
        "the catch-up moved the hash sequence"
    );
    assert_eq!(
        plain.events, first.events,
        "the catch-up moved the event list"
    );
}

// ---------------------------------------------------------------------------
// (b) a tempo change mid-run changes no outcome
// ---------------------------------------------------------------------------

#[test]
fn a_tempo_change_mid_run_changes_no_outcome() {
    // The tempo rises at the third pulse's arena time in **both** runs — the hash
    // folds the tempo byte by A.1's design, so a comparison that changed the tempo
    // in only one run would be asserting that the hash records the tempo, which it
    // does. What differs is the **cadence**: the reference run keeps stepping every
    // 1,000 ms across the change, and the other adopts the new tempo's own 500 ms
    // period, which is what cut T's timer actually does.
    let flip_at = 2_000_i64;
    let pin = move |now_ms: i64| {
        if now_ms < flip_at {
            Tempo::Quiet
        } else {
            Tempo::Engaged
        }
    };

    let mut steady = scripted_arena();
    let reference = replay(&mut steady, 1_000, 60_000, &script(), &pin);

    // The same script, the same tempo function, a cadence that changes with it.
    let mut arena = scripted_arena();
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = 0_i64;
    let mut next = 0_usize;
    let script = script();
    let mut pulses = 0_u32;
    loop {
        arena.clock.pin_tempo_for_tests(pin(arena.now_ms));
        let mut controls: Vec<Control> = Vec::new();
        let mut intents: Vec<Intent> = Vec::new();
        while let Some(beat) = script.get(next) {
            if beat.t_ms > arena.now_ms {
                break;
            }
            controls.extend(beat.controls.iter().copied());
            intents.extend(beat.intents.iter().cloned());
            // 16b cut D.3: the world's `Stop`, armed before this pulse steps.
            if let Some(stop) = beat.stop {
                arena.stop(stop);
            }
            next += 1;
        }
        let out = arena.step(wall, &controls, &intents);
        absorb(&out, &mut bytes);
        hashes.extend(arena.substep_hashes.iter().copied());
        pulses += 1;
        if arena.now_ms >= 60_000 {
            break;
        }
        wall += if arena.now_ms < flip_at { 1_000 } else { 500 };
    }
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }

    assert_ne!(pulses, reference.pulses, "the cadence did not change");
    assert_eq!(
        lines(&bytes),
        reference.events,
        "the tempo change moved an event"
    );
    assert_eq!(
        hashes, reference.hashes,
        "the tempo change moved a substep's state"
    );
    // And the tempo really did change, in both runs — and both took the scripted
    // control.
    assert_eq!(steady.clock.tempo(), Tempo::Engaged);
    assert_eq!(arena.clock.tempo(), Tempo::Engaged);
    assert!(arena.unit(MOVER).is_some_and(|unit| unit.linkdead));
    assert!(steady.unit(MOVER).is_some_and(|unit| unit.linkdead));
}

#[test]
fn two_different_tempos_produce_the_same_motion() {
    // The other half of (b), and the one the hash cannot state: the two runs are at
    // **different tempos**, so their state hashes differ in the tempo byte by
    // design — what must not differ is anything the tempo is not allowed to decide.
    // So this compares the whole event stream with the hash column struck out, and
    // the final pose of every unit.
    //
    // Without this, a tempo that reached into motion — a speed scaled by the
    // period, say — would pass every other test in this file, because they all hold
    // the tempo equal across the runs they compare.
    let strip = |events: &[Line]| -> Vec<Bare> {
        events
            .iter()
            .map(|(t_ms, kind, draw, _)| (*t_ms, kind.clone(), draw.clone()))
            .collect()
    };

    let mut quiet_arena = scripted_arena();
    let quiet = replay(&mut quiet_arena, 1_000, 60_000, &script(), &|_: i64| {
        Tempo::Quiet
    });
    let mut engaged_arena = scripted_arena();
    let engaged = replay(&mut engaged_arena, 500, 60_000, &script(), &|_: i64| {
        Tempo::Engaged
    });

    assert_eq!(quiet_arena.clock.tempo(), Tempo::Quiet);
    assert_eq!(engaged_arena.clock.tempo(), Tempo::Engaged);
    assert_ne!(engaged.pulses, quiet.pulses);
    assert_eq!(
        strip(&engaged.events),
        strip(&quiet.events),
        "the tempo decided an event"
    );
    assert_eq!(engaged.finals, quiet.finals, "the tempo decided a pose");
    assert_eq!(engaged.substeps, quiet.substeps);
    assert_eq!(engaged.hashes.len(), quiet.hashes.len());
    // And the hashes themselves differ, in the tempo byte and nothing else — which
    // is the proof that the comparison above was not comparing two identical runs.
    assert_ne!(engaged.hashes, quiet.hashes);
}

// ---------------------------------------------------------------------------
// (c) a new draw label shifts nothing
// ---------------------------------------------------------------------------

#[test]
fn adding_a_draw_label_shifts_no_existing_draw() {
    // The value A.1 pinned, taken before any other label exists.
    let before = draw(2_350, &DrawLabel::Hit { shot_seq: 1 });
    assert_eq!(before, 12_072_725_193_963_834_385);

    // Cut S's contact ids and classification draws, made in between.
    let mut drawn: Vec<u64> = Vec::new();
    for viewer in 1..=8_u32 {
        for target in 1..=8_u32 {
            drawn.push(draw(
                2_350,
                &DrawLabel::Contact {
                    viewer: ObjId(viewer),
                    target: ObjId(target),
                },
            ));
            drawn.push(draw(
                2_350,
                &DrawLabel::Classify {
                    observer: ObjId(viewer),
                    target: ObjId(target),
                    pass_seq: 3,
                },
            ));
        }
    }
    assert_eq!(drawn.len(), 128);

    // And the hit draw is the same value: there is no stream whose position any of
    // those consumed (doc 16 §3.1 "Rate independence", §3.5).
    let after = draw(2_350, &DrawLabel::Hit { shot_seq: 1 });
    assert_eq!(after, before);
    // Every label is its own key, so no two of the draws above collided either.
    let unique: std::collections::BTreeSet<u64> = drawn.iter().copied().collect();
    assert_eq!(unique.len(), drawn.len());
    // A different seed is a different match, and a different shot is a different
    // draw.
    assert_ne!(draw(2_351, &DrawLabel::Hit { shot_seq: 1 }), before);
    assert_ne!(draw(2_350, &DrawLabel::Hit { shot_seq: 2 }), before);
}

// ---------------------------------------------------------------------------
// (d) the epoch purge, end to end
// ---------------------------------------------------------------------------

#[test]
fn startup_then_released_in_one_burst_never_starts() {
    // Doc 16 §3.1: *a pilot who types `FIRE` then `EXIT` in one burst never
    // fires.* `STARTUP` is cut A's discrete action, so this is the same burst with
    // the order cut A can resolve.
    let mut arena = scripted_arena();
    let out = arena.step(
        0,
        &[Control {
            unit: MOVER,
            channel: Channel::Helm,
            epoch: 2,
            state: ControlState::Released,
            pilot: None,
        }],
        &[intent(MOVER, IntentKind::Startup, 1)],
    );
    assert!(out.ended.is_none());
    assert_eq!(arena.unit(MOVER).map(|unit| unit.power), Some(Power::Off));
    assert!(arena.pending.is_empty(), "the order was not purged");
    assert_eq!(arena.epochs.current(MOVER, Channel::Helm), 2);
    // **16b cut N.1** amends doc 16 §3.8's *the tank idles*: the released seat goes
    // to its sealed mind (the reflex controller in `Withdraw` until 17b cut M4.3;
    // re-pointed by 17b cut M5.7), which puts its first think on the schedule
    // beside the position summary.
    // 16b cut F.3 adds one sensor pass per unit, which every platform now has
    // because the pass is where its contacts expire. **16b cut M.2** (16c `T-01`)
    // adds the duration bound, a due event since that cut.
    assert_eq!(
        arena.due.len(),
        5,
        "the position summary, the duration bound, the released tank's first tick, two passes"
    );

    // **16b cut D.3 (`B-34`) replaced this block's claim.** It used to assert
    // that the tank was still cold twenty seconds later, and that no `startup`
    // was ever journalled — the released tank standing dark on its entry cell for
    // the whole of a match nobody could then end, which is the defect the row
    // names. The controller now brings its own power up, and the order that was
    // purged by the epoch is the only thing this test is still about: the
    // startup below carries no seq of the pilot's, and `arena.pending` was empty
    // the moment the burst resolved.
    let mut wall = 0_i64;
    for _ in 0..4 {
        wall += 5_000;
        arena.step(wall, &[], &[]);
    }
    assert_eq!(arena.now_ms, 20_000);
    assert_eq!(
        arena.unit(MOVER).map(|unit| unit.power),
        Some(Power::Online),
        "the machine that took the released seat started itself up"
    );
    let mut bytes = Vec::new();
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    let journalled = lines(&bytes);
    // The `startup` that IS journalled is the controller's own, at its first
    // reflex tick, and not the purged order's: the order was refused at the
    // epoch inside the burst's own pulse, which is `arena.now_ms == 0`.
    let startups: Vec<i64> = journalled
        .iter()
        .filter(|(_, kind, _, _)| kind.contains("startup"))
        .map(|(t_ms, _, _, _)| *t_ms)
        .collect();
    assert!(
        startups.iter().all(|t_ms| *t_ms > 0),
        "the purged order started the tank after all: {journalled:?}"
    );
}

/// **The other half of the purge: the epoch the world published still works.**
///
/// This block used to close
/// [`startup_then_released_in_one_burst_never_starts`], where the released tank
/// stood cold for ever and an order at epoch 2 was the only thing that could
/// start it. 16b cut D.3 (`B-34`) gave that tank its own power-up, so the old
/// assertion could no longer be made *after* twenty seconds of stepping — the
/// unit is online by then and a `Startup` means nothing. The claim is the same
/// one, made in the window it is still about: one pulse after the burst, before
/// the controller's first tick at `arena.reflex.decide_ms`.
///
/// A purge is a refusal of the **stale** order, never a channel that stops
/// listening.
#[test]
fn an_order_at_the_epoch_the_purge_published_is_still_admitted() {
    let mut arena = scripted_arena();
    arena.step(
        0,
        &[Control {
            unit: MOVER,
            channel: Channel::Helm,
            epoch: 2,
            state: ControlState::Released,
            pilot: None,
        }],
        &[intent(MOVER, IntentKind::Startup, 1)],
    );
    assert_eq!(arena.unit(MOVER).map(|unit| unit.power), Some(Power::Off));
    assert_eq!(arena.epochs.current(MOVER, Channel::Helm), 2);

    // Half a second later — inside the controller's first `decide_ms` window, so
    // nothing of its own has run yet — the same order at the published epoch.
    arena.step(
        500,
        &[],
        &[Intent {
            unit: MOVER,
            channel: Channel::Helm,
            epoch: 2,
            kind: IntentKind::Startup,
            seq: 2,
        }],
    );
    assert_eq!(
        arena.unit(MOVER).map(|unit| unit.power),
        Some(Power::Starting),
        "the purge closed the channel instead of refusing one stale order"
    );
}

// ---------------------------------------------------------------------------
// (e) rate independence, at the ceiling
// ---------------------------------------------------------------------------

#[test]
fn a_unit_at_its_ceiling_covers_the_same_ground_however_the_time_is_cut() {
    // Five seconds at 1,000 cm/s is 5,000 cm — five whole cells — and it is the
    // same five cells in one pulse of 5,000 ms and in five pulses of 1,000 ms.
    // The tank is set running rather than started up: the claim under test is the
    // distance, not the lifecycle.
    let running = |arena: &mut Arena| {
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        if let Some(unit) = arena.unit_mut(MOVER) {
            unit.power = Power::Online;
            unit.speed_cms = 1_000;
            unit.desired_speed_cms = 1_000;
        }
    };
    let start = centre(30, 5);
    let expected = start.y_cm + 5_000;

    let mut one = arena_of(
        vec![tank(MOVER, Team::Red, centre(30, 5))],
        &MatchConfig::default(),
    );
    running(&mut one);
    one.step(0, &[], &[]);
    let out = one.step(5_000, &[], &[]);
    assert_eq!(out.substeps, 20);
    assert_eq!(one.unit(MOVER).map(|unit| unit.pose.y_cm), Some(expected));
    assert_eq!(one.unit(MOVER).map(|unit| unit.pose.x_cm), Some(start.x_cm));

    let mut five = arena_of(
        vec![tank(MOVER, Team::Red, centre(30, 5))],
        &MatchConfig::default(),
    );
    running(&mut five);
    five.step(0, &[], &[]);
    let mut wall = 0_i64;
    for _ in 0..5 {
        wall += 1_000;
        let out = five.step(wall, &[], &[]);
        assert_eq!(out.substeps, 4);
    }
    assert_eq!(five.unit(MOVER).map(|unit| unit.pose.y_cm), Some(expected));
    assert_eq!(five.now_ms, one.now_ms);
    // The same state, not merely the same distance.
    assert_eq!(five.substep_hashes.len(), 4);
    assert_eq!(
        tacsim::arena::hash::state_hash(&five),
        tacsim::arena::hash::state_hash(&one)
    );
}

// ---------------------------------------------------------------------------
// (f) the duration bound
// ---------------------------------------------------------------------------

#[test]
fn the_duration_bound_ends_the_match_and_a_further_step_does_nothing() {
    let mut arena = arena_of(
        vec![tank(MOVER, Team::Red, centre(30, 5))],
        &MatchConfig {
            max_duration_ms: 12_000,
            ..MatchConfig::default()
        },
    );
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    let mut ended = None;
    for _ in 0..8 {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        if let Some(end) = out.ended {
            ended = Some(end);
            break;
        }
        wall += 5_000;
    }
    let ended = ended.expect("the match timed out");
    assert_eq!(ended.reason, EndReason::Timeout);
    // **16b cut M.2** (16c `T-01`): the bound is a due event, so the match ends
    // at the deadline's own millisecond and not at the end of the pulse that
    // crossed it (15,000, which this assertion read before the cut).
    assert_eq!(
        ended.t_ms, 12_000,
        "the deadline's own millisecond ended it"
    );
    assert_eq!(arena.ended, Some(ended));

    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    let journalled = lines(&bytes);
    assert_eq!(
        journalled.last().map(|(_, kind, _, _)| kind.clone()),
        Some(
            serde_json::to_string(&EventKind::MatchEnded {
                reason: EndReason::Timeout
            })
            .unwrap_or_default()
        ),
        "the ledger's last word is not the end of the match: {journalled:?}"
    );

    // A further step does nothing: no substep, no chunk, no frame, no time.
    let after = arena.step(wall + 5_000, &[], &[]);
    assert_eq!(after.substeps, 0);
    assert!(after.ledger_chunks.is_empty());
    assert!(after.frames.is_empty());
    assert_eq!(after.ended, Some(ended));
    // 16b cut M.2: the pulse stopped at the substep the deadline fell in.
    assert_eq!(arena.now_ms, 12_000);

    // And a `Stop` after the end cannot reopen it either.
    arena.stop(StopReason::Abort);
    let after = arena.step(wall + 10_000, &[], &[]);
    assert_eq!(after.ended.map(|end| end.reason), Some(EndReason::Timeout));
}

// ---------------------------------------------------------------------------
// (g) the window's edge
// ---------------------------------------------------------------------------

#[test]
fn the_window_edge_stops_a_unit_with_one_notice() {
    // Doc 16 §3.2: *Edges stop motion. A unit cannot leave the window.* The order
    // to drive north stands throughout, so the unit spends the rest of the run
    // held against the edge — and that is **one** stop, not one per substep.
    let mut arena = arena_of(
        vec![tank(MOVER, Team::Red, centre(30, 62))],
        &MatchConfig::default(),
    );
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    if let Some(unit) = arena.unit_mut(MOVER) {
        unit.power = Power::Online;
        unit.speed_cms = 1_000;
        unit.desired_speed_cms = 1_000;
    }
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = 0_i64;
    absorb(&arena.step(wall, &[], &[]), &mut bytes);
    for _ in 0..10 {
        wall += 1_000;
        absorb(&arena.step(wall, &[], &[]), &mut bytes);
    }
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    let journalled = lines(&bytes);
    let blocked: Vec<&Line> = journalled
        .iter()
        .filter(|(_, kind, _, _)| kind.contains("blocked"))
        .collect();
    assert_eq!(
        blocked.len(),
        1,
        "the edge was journalled {} times: {journalled:?}",
        blocked.len()
    );
    assert_eq!(
        blocked.first().map(|(_, kind, _, _)| kind.clone()),
        Some(
            serde_json::to_string(&EventKind::Blocked {
                unit: MOVER,
                x: 30,
                y: 63,
                cause: BlockedCause::Edge,
            })
            .unwrap_or_default()
        )
    );
    // Hard against the last centimetre of the top row, stopped, and still held.
    let unit = arena.unit(MOVER).expect("the mover");
    assert_eq!(unit.pose.y_cm, 63_999);
    assert_eq!(unit.speed_cms, 0);
    assert!(unit.blocked);
    assert_eq!(
        arena.definition.terrain.cell_of(&unit.pose),
        Some((30, 63)),
        "the unit is off the window"
    );
    // The index agrees with the terrain about where it is.
    assert_eq!(arena.index.cell_of_unit(MOVER), Some((30, 63)));
    // A frame reached the pilot, and the last one says it is blocked.
    let last: BTreeMap<ObjId, bool> = arena
        .last_frames
        .iter()
        .map(|(pilot, frame)| (*pilot, frame.held))
        .collect();
    assert_eq!(last.get(&ObjId(1_000)), Some(&true));
}

// ---------------------------------------------------------------------------
// (g2) The pilot's own `blocked` notice (16b cut F.2; doc 16 §3.4 step 4)
// ---------------------------------------------------------------------------

/// Every notice line one pilot was sent over a run, in order.
fn heard(collected: &[(ObjId, Frame)], pilot: ObjId) -> Vec<String> {
    collected
        .iter()
        .filter(|(who, _)| *who == pilot)
        .flat_map(|(_, frame)| tacsim::match_::cockpit::notice_lines(frame))
        .collect()
}

/// Drive `MOVER` at `heading_mdeg` for `pulses` pulses of a second, answering
/// every frame the run produced.
fn drive(arena: &mut Arena, heading_mdeg: u32, pulses: u32) -> Vec<(ObjId, Frame)> {
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    if let Some(unit) = arena.unit_mut(MOVER) {
        unit.power = Power::Online;
        unit.pose.heading_mdeg = heading_mdeg;
        unit.desired_heading_mdeg = heading_mdeg;
        unit.speed_cms = 1_000;
        unit.desired_speed_cms = 1_000;
    }
    let mut collected: Vec<(ObjId, Frame)> = Vec::new();
    let mut wall = 0_i64;
    collected.extend(arena.step(wall, &[], &[]).frames);
    for _ in 0..pulses {
        wall += 1_000;
        collected.extend(arena.step(wall, &[], &[]).frames);
    }
    collected
}

#[test]
fn a_stop_tells_the_pilot_once_and_names_what_stopped_it() {
    // Doc 16 §3.4 step 4's three refusals, each with its own sentence, and each
    // said **once per stop** however long the standing order holds the tank
    // against the obstacle. 16b cut F.2: until this cut the only surface for the
    // fact was a `, HELD` suffix a pilot had to type `STATUS` to see.
    //
    // (1) The window's edge: north out of the top row.
    let mut arena = arena_of(
        vec![tank(MOVER, Team::Red, centre(30, 62))],
        &MatchConfig::default(),
    );
    let told = heard(&drive(&mut arena, 0, 10), ObjId(1_000));
    let stops: Vec<&String> = told
        .iter()
        .filter(|line| line.starts_with("Held:"))
        .collect();
    assert_eq!(
        stops,
        vec![&"Held: this is the edge of the arena.".to_owned()],
        "one line per stop, and it names the edge: {told:?}"
    );
    assert!(
        arena.unit(MOVER).is_some_and(|unit| unit.held.is_some()),
        "and the condition stands while the order does"
    );

    // (2) Impassable ground: east into the band at column 20.
    let mut arena = arena_of(
        vec![tank(MOVER, Team::Red, centre(BAND_X - 3, 30))],
        &MatchConfig::default(),
    );
    let told = heard(&drive(&mut arena, 90_000, 10), ObjId(1_000));
    let stops: Vec<&String> = told
        .iter()
        .filter(|line| line.starts_with("Held:"))
        .collect();
    assert_eq!(
        stops,
        vec![&"Held: the ground ahead will not take the tank.".to_owned()],
        "{told:?}"
    );

    // (3) An occupied cell — and the line names neither what nor whose, because
    // the cell may hold a unit this platform has not detected (doc 16 §3.6).
    let mut arena = arena_of(
        vec![
            tank(MOVER, Team::Red, centre(10, 30)),
            tank(PARKED, Team::Blue, centre(13, 30)),
        ],
        &MatchConfig::default(),
    );
    let told = heard(&drive(&mut arena, 90_000, 10), ObjId(1_000));
    let stops: Vec<&String> = told
        .iter()
        .filter(|line| line.starts_with("Held:"))
        .collect();
    assert_eq!(
        stops,
        vec![&"Held: something is in the way.".to_owned()],
        "{told:?}"
    );
    for line in &told {
        assert!(
            !line.contains(&format!("#{}", PARKED.0)) && !line.contains("blue"),
            "the stop names neither the unit nor its side: {line}"
        );
    }
}

/// **One pulse's reading of the held pilot's own frame** (16b cut E.3): the arena
/// millisecond, the speed, the `HELD` condition, the cell and every notice line.
type Reading = (i64, i32, bool, Option<(u16, u16)>, Vec<String>);

/// **16b cut E.3 (16c `B-46`, §5's Q10): another unit in the way holds a tank
/// before it stops it, and what the pilot reads while it holds is decided.**
///
/// A seated pilot drives east into a cell a tank they have never detected is
/// parked in (this fixture's units carry no sensor at all, so the obstacle is as
/// invisible as doc 16 §3.6 ever makes one). Four claims:
///
/// * **nothing is said for two seconds** — no notice at all while the hold runs,
///   and then exactly one `Held: something is in the way.`, which is the same
///   sentence HEAD said, later and never sooner;
/// * **the frame during the hold** carries the speed the tank had, `held`
///   false, a pose that does not leave its own cell, and nothing that names or
///   places the obstacle;
/// * **the frame after the stop** carries `0` and `held`, which is what
///   `STATUS` prints `HELD` off — so the condition follows the stop and not the
///   hold;
/// * **one `blocked` ledger line** for the whole episode, because the pilot's
///   standing order is taken off as soon as the line arrives.
#[test]
fn another_unit_holds_a_tank_for_two_seconds_and_then_stops_it_with_one_line() {
    let mut arena = arena_of(
        vec![
            tank(MOVER, Team::Red, centre(10, 30)),
            tank(PARKED, Team::Blue, centre(12, 30)),
        ],
        &MatchConfig::default(),
    );
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    if let Some(unit) = arena.unit_mut(MOVER) {
        unit.power = Power::Online;
        unit.pose.heading_mdeg = 90_000;
        unit.desired_heading_mdeg = 90_000;
        unit.speed_cms = 1_000;
        unit.desired_speed_cms = 1_000;
    }
    // Half-second pulses, so the two seconds of holding are four readings and not
    // one. Each reading is the mover's own frame.
    let mut bytes: Vec<u8> = Vec::new();
    let mut readings: Vec<Reading> = Vec::new();
    let mut wall = 0_i64;
    for _ in 0..20 {
        let out = arena.step(wall, &[], &[]);
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        let mut said = false;
        for (who, frame) in &out.frames {
            if *who != ObjId(1_000) {
                continue;
            }
            let lines = tacsim::match_::cockpit::notice_lines(frame);
            said |= lines.iter().any(|line| line.starts_with("Held:"));
            readings.push((arena.now_ms, frame.speed_cms, frame.held, frame.cell, lines));
        }
        wall += 500;
        // The pilot has been told; the rest of the episode is what they do next.
        if said {
            break;
        }
    }

    // The pose never leaves the cell it was tested into, hold or stop.
    assert!(
        readings
            .iter()
            .all(|(_, _, _, cell, _)| matches!(cell, Some((x, 30)) if *x <= 11)),
        "the tank crossed into the parked tank's cell: {readings:?}"
    );
    // The line arrives once, and only at the stop.
    let told: Vec<(i64, &String)> = readings
        .iter()
        .flat_map(|(at, _, _, _, lines)| lines.iter().map(move |line| (*at, line)))
        .filter(|(_, line)| line.starts_with("Held:"))
        .collect();
    assert_eq!(
        told.len(),
        1,
        "the pilot was told more than once, or not at all: {told:?}"
    );
    let (said_at, line) = told[0];
    assert_eq!(line, "Held: something is in the way.");
    // The first reading at which the tank had stopped short of the parked one.
    let first_held = readings
        .iter()
        .find(|(_, _, _, cell, _)| matches!(cell, Some((11, 30))))
        .map(|(at, ..)| *at)
        .expect("the tank reached the boundary");
    assert!(
        said_at >= first_held + 2_000,
        "the line arrived {} ms after the tank first met the obstacle, which is inside the \
         two-second hold",
        said_at - first_held
    );

    // The frame during the hold, decided: the speed it had, no `HELD`, and
    // nothing naming the obstacle.
    let holding: Vec<&Reading> = readings
        .iter()
        .filter(|(at, _, _, cell, _)| *at < said_at && matches!(cell, Some((11, 30))))
        .collect();
    assert!(
        holding.len() >= 3,
        "the hold was not read often enough to say anything: {holding:?}"
    );
    assert!(
        holding
            .iter()
            .all(|(_, speed, blocked, _, lines)| *speed == 1_000 && !*blocked && lines.is_empty()),
        "a held tank read a speed it did not have, a `HELD` it had not earned, or a line \
         nobody said: {holding:?}"
    );
    for (_, _, _, _, lines) in &readings {
        for line in lines {
            assert!(
                !line.contains(&format!("#{}", PARKED.0))
                    && !line.contains("blue")
                    && !line.contains("12"),
                "a line named or placed the obstacle: {line}"
            );
        }
    }

    // And after the stop: zero, and the condition the cockpit prints `HELD` off.
    let stopped = readings
        .iter()
        .find(|(at, ..)| *at == said_at)
        .expect("the reading the line came on");
    assert_eq!(
        (stopped.1, stopped.2),
        (0, true),
        "the stop did not zero the speed and raise the condition: {stopped:?}"
    );

    // One `blocked` line for the whole episode: the order comes off at the line,
    // which is what a pilot who has been told does.
    if let Some(unit) = arena.unit_mut(MOVER) {
        unit.desired_speed_cms = 0;
    }
    for _ in 0..8 {
        let out = arena.step(wall, &[], &[]);
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        wall += 500;
    }
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    let stops = String::from_utf8(bytes)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<LedgerEvent>(line).ok())
        .filter(|event| matches!(event.kind, EventKind::Blocked { unit, .. } if unit == MOVER))
        .count();
    assert_eq!(stops, 1, "one episode, one `blocked` line");
}

#[test]
fn a_tank_still_pushing_against_an_obstacle_still_reads_held() {
    // The defect the 2026-09-18 smoke found: `UnitState::blocked` is cleared at
    // the top of every substep and only set again by a substep that reached the
    // boundary, so a tank ordered forward from rest against a bank read `0 kph`
    // with no `HELD` on the substeps it covered no whole centimetre in. The
    // condition the frame carries is now `held`, which a stop sets and which
    // clears only on a crossing into another cell, an in-cell move strictly away
    // from what held it, or the order coming off (16b cut M.3's rule,
    // `tacsim::arena::motion::integrate`).
    let mut arena = arena_of(
        vec![tank(MOVER, Team::Red, centre(BAND_X - 1, 30))],
        &MatchConfig::default(),
    );
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    if let Some(unit) = arena.unit_mut(MOVER) {
        unit.power = Power::Online;
        unit.pose.heading_mdeg = 90_000;
        unit.desired_heading_mdeg = 90_000;
        unit.desired_speed_cms = 1_000;
    }
    // The pulse length is **not** a multiple of the substep, which is what makes
    // this test the one the defect needs. 1,000 cm cells against 1,000 cm/s is a
    // 250 ms substep, so a 1,030 ms pulse ends on a 30 ms remainder — and in that
    // remainder a tank the previous substep stopped accelerates from nought to
    // `200 × 30 / 1000 = 6` cm/s and travels `6 × 30 / 1000 = 0` cm. `advance`
    // returns early on that, so `blocked` — cleared at the top of every substep —
    // is **false** at the end of the pulse while the tank is standing against the
    // bank with a standing order to go through it. Every pulse in production has
    // such a remainder.
    let mut wall = 0_i64;
    arena.step(wall, &[], &[]);
    let mut stopped = false;
    let mut short_substep_pulses = 0_u32;
    let mut frames_held = 0_u32;
    for _ in 0..20 {
        wall += 1_030;
        let out = arena.step(wall, &[], &[]);
        let Some(unit) = arena.unit(MOVER) else {
            continue;
        };
        stopped |= unit.held.is_some();
        if stopped {
            assert!(
                unit.held.is_some(),
                "the condition did not survive a substep that covered no ground"
            );
            // The two facts the cut claims are different, on the pulses where they
            // are: the substep flag is off and the condition is on.
            if !unit.blocked {
                short_substep_pulses += 1;
            }
            // And it is the **condition** the pilot's frame carries, which is what
            // `STATUS` prints `, HELD` off (16b cut F.2's half of brief item 1).
            for (_, frame) in &out.frames {
                assert!(
                    frame.held,
                    "the frame says the tank is free while it stands against the bank"
                );
                frames_held += 1;
            }
        }
    }
    assert!(stopped, "the tank reached the band");
    assert!(
        short_substep_pulses > 0,
        "no pulse ended on a substep that covered no ground, so `held` was never \
         tested against `blocked`"
    );
    assert!(frames_held > 0, "the pilot was sent no frame at all");
    // And the order coming off clears it: a tank told to stop is not being held
    // against anything.
    if let Some(unit) = arena.unit_mut(MOVER) {
        unit.desired_speed_cms = 0;
    }
    wall += 1_030;
    let out = arena.step(wall, &[], &[]);
    assert!(arena.unit(MOVER).is_some_and(|unit| unit.held.is_none()));
    for (_, frame) in &out.frames {
        assert!(!frame.held, "and the frame says so too");
    }
}

// ---------------------------------------------------------------------------
// (g4) one stop against a bank is one edge (16b cut M.3; 16c `T-04`, `B-27`)
// ---------------------------------------------------------------------------

/// What one bank run left: the ledger's `blocked` lines, every `Held:` line the
/// pilot was told, the arena, and every frame the run produced.
struct BankRun {
    blocked: Vec<Line>,
    told: Vec<String>,
    arena: Arena,
    frames: Vec<(ObjId, Frame)>,
}

/// **Drive `MOVER` east into the band at [`BAND_X`] and, once it is stopped
/// there, give it `then` while its speed order stands** (16b cut M.3), with
/// `others` on the field beside it.
///
/// Pulses of `pulse_ms` at `Quiet`. `pulse_ms` is the whole of the defect
/// `B-27` was: at 1,000 ms against a 250 ms substep every pulse cuts into four
/// whole substeps, and a pulse of 1,030 ms ends on a 30 ms tail — which every
/// production pulse has, since arena time is elapsed wall time. In a tail the
/// tank has rebuilt six centimetres a second from its stop and travels a fraction
/// of a centimetre, and `motion`'s travel is a Euclidean division: a fraction
/// **east** is no whole centimetre, but a fraction **south** is minus one. So a
/// tank pushed at the bank on a heading with a southward component moves one
/// whole centimetre along the bank inside its own cell, which cleared the old
/// `held` (any substep that "moved") and made the next refusal of the same bank a
/// new stop, told again: the smoke's `Held: the ground ahead will not take the
/// tank.` twice one second apart.
fn bank_run(
    pulse_ms: i64,
    others: Vec<CustodyUnit>,
    then: IntentKind,
    before: u32,
    after: u32,
) -> BankRun {
    let mut units = vec![tank(MOVER, Team::Red, centre(BAND_X - 1, 30))];
    units.extend(others);
    let mut arena = arena_of(units, &MatchConfig::default());
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    if let Some(unit) = arena.unit_mut(MOVER) {
        unit.power = Power::Online;
        unit.pose.heading_mdeg = 90_000;
        unit.desired_heading_mdeg = 90_000;
        unit.speed_cms = 1_000;
        unit.desired_speed_cms = 1_000;
    }
    let mut bytes: Vec<u8> = Vec::new();
    let mut frames: Vec<(ObjId, Frame)> = Vec::new();
    let mut wall = 0_i64;
    let out = arena.step(wall, &[], &[]);
    absorb(&out, &mut bytes);
    frames.extend(out.frames);
    // Up to the stop: the tank starts half a cell short of the band at full
    // speed, so it is there inside the first pulse; two make sure.
    for _ in 0..2 {
        wall += pulse_ms;
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes);
        frames.extend(out.frames);
    }
    assert_eq!(
        arena.unit(MOVER).map(|unit| unit.pose.x_cm),
        Some(i64::from(BAND_X) * i64::from(CELL_CM) - 1),
        "the tank never reached the band"
    );
    // The order, in the next pulse, and then `before + after` pulses of it.
    for pulse in 0..=before + after {
        wall += pulse_ms;
        let intents = if pulse == 0 {
            vec![intent(MOVER, then.clone(), 10)]
        } else {
            Vec::new()
        };
        let out = arena.step(wall, &[], &intents);
        absorb(&out, &mut bytes);
        frames.extend(out.frames);
    }
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    let blocked = lines(&bytes)
        .into_iter()
        .filter(|(_, kind, _, _)| kind.contains("\"blocked\""))
        .collect();
    let told = heard(&frames, MOVER_PILOT)
        .into_iter()
        .filter(|line| line.starts_with("Held:"))
        .collect();
    BankRun {
        blocked,
        told,
        arena,
        frames,
    }
}

/// The `blocked` line for `MOVER` at `(x, y)` for `cause`, as the ledger writes it.
fn blocked_line(x: u16, y: u16, cause: BlockedCause) -> String {
    serde_json::to_string(&EventKind::Blocked {
        unit: MOVER,
        x,
        y,
        cause,
    })
    .unwrap_or_default()
}

/// **16b cut M.3 (16c `B-27`, `T-04`, §5's Q22): a tank that turns 30° off
/// square to the bank while its speed order still pushes it there is one stop —
/// one `blocked` line, with its cause, and one `Held:` line — however long it
/// pushes.**
///
/// The failing test first, and it failed at HEAD (`c7cab0a9`): over the two
/// pulses after the order the 1,030 ms driver journalled five `blocked` lines and
/// told the pilot three times, and over thirteen pulses sixteen lines and fourteen
/// notices — one per pulse tail, each a one-centimetre slide south along the bank
/// (see [`bank_run`]). At 1,000 ms pulses the same run was one and one, which is
/// why every earlier test of the rule passed. The rule the brief first wrote
/// (clear on an in-cell move whose dot product with the pose-to-held-centre
/// vector is negative) measured three lines over two pulses and fourteen over
/// thirteen on this driver, because a slide past the held cell's centre line
/// reads as "away"; the rule as built measures against the direction from the
/// unit's cell to the held one (`motion::integrate`'s head).
#[test]
fn m3_a_tank_turning_against_a_bank_is_one_stop_one_line_and_one_notice() {
    // Two pulses of it.
    let run = bank_run(
        1_030,
        Vec::new(),
        IntentKind::Heading { mdeg: 120_000 },
        2,
        0,
    );
    assert_eq!(
        run.blocked
            .iter()
            .map(|(_, kind, _, _)| kind.clone())
            .collect::<Vec<String>>(),
        vec![blocked_line(BAND_X, 30, BlockedCause::Impassable)],
        "one stop, one `blocked` line, and it names the cause"
    );
    assert_eq!(
        run.told,
        vec!["Held: the ground ahead will not take the tank.".to_owned()],
        "one stop, one line to the pilot"
    );
    // And eleven more: still one, still in the cell it stopped in.
    let run = bank_run(
        1_030,
        Vec::new(),
        IntentKind::Heading { mdeg: 120_000 },
        2,
        11,
    );
    assert_eq!(run.blocked.len(), 1, "{:?}", run.blocked);
    assert_eq!(run.told.len(), 1, "{:?}", run.told);
    let unit = run.arena.unit(MOVER).expect("the mover");
    assert_eq!(
        run.arena.definition.terrain.cell_of(&unit.pose),
        Some((BAND_X - 1, 30)),
        "the tank left the cell it stopped in, so the run measured nothing"
    );
    assert_eq!(
        unit.pose.x_cm, 19_999,
        "and it is still hard against the bank"
    );
    assert_eq!(unit.pose.heading_mdeg, 120_000, "and it did turn");
    assert!(
        unit.pose.y_cm < 30_500,
        "and it did slide south along the bank, which is the case: {}",
        unit.pose.y_cm
    );
}

/// **16b cut M.3 (16c `T-04`; the plan's review M-03): a second, different
/// obstacle met inside the same cell is a second stop.** A bank stop, then a
/// `HEADING` north toward a tank parked in the neighbouring cell: two `blocked`
/// lines with two causes, and two `Held:` lines.
#[test]
fn m3_a_bank_then_a_parked_tank_in_the_same_cell_is_two_stops() {
    let run = bank_run(
        1_000,
        vec![tank(PARKED, Team::Blue, centre(BAND_X - 1, 31))],
        IntentKind::Heading { mdeg: 0 },
        2,
        8,
    );
    assert_eq!(
        run.blocked
            .iter()
            .map(|(_, kind, _, _)| kind.clone())
            .collect::<Vec<String>>(),
        vec![
            blocked_line(BAND_X, 30, BlockedCause::Impassable),
            blocked_line(BAND_X - 1, 31, BlockedCause::Occupied),
        ],
        "a bank, then something in the way: {:?}",
        run.blocked
    );
    assert_eq!(
        run.told,
        vec![
            "Held: the ground ahead will not take the tank.".to_owned(),
            "Held: something is in the way.".to_owned(),
        ]
    );
    // Both inside the one cell the tank never left.
    let unit = run.arena.unit(MOVER).expect("the mover");
    assert_eq!(
        run.arena.definition.terrain.cell_of(&unit.pose),
        Some((BAND_X - 1, 30))
    );
}

/// **16b cut M.3 (the plan's review M-03c): reversing off the bank drops `HELD`
/// inside the cell.** The travel points strictly away from the held cell, so the
/// condition clears on the first substep that covers a centimetre — the pilot's
/// frame says so while the tank is still in the cell it stopped in, rather than
/// ten metres later.
#[test]
fn m3_reversing_off_the_bank_clears_held_before_the_tank_leaves_its_cell() {
    let run = bank_run(1_000, Vec::new(), IntentKind::Speed { cms: -400 }, 1, 0);
    let unit = run.arena.unit(MOVER).expect("the mover");
    assert!(
        unit.pose.x_cm < 19_999,
        "the tank did not reverse: {}",
        unit.pose.x_cm
    );
    assert_eq!(unit.held, None);
    let last = run
        .frames
        .iter()
        .rev()
        .find(|(pilot, _)| *pilot == MOVER_PILOT)
        .map(|(_, frame)| (frame.held, frame.cell))
        .expect("the pilot was sent a frame");
    assert_eq!(
        last,
        (false, Some((BAND_X - 1, 30))),
        "not held, and still in the cell it stopped in"
    );
    // And it was held before the order: a frame said so.
    assert!(
        run.frames
            .iter()
            .any(|(pilot, frame)| *pilot == MOVER_PILOT && frame.held),
        "the stop was never read as held"
    );
    assert_eq!(run.blocked.len(), 1, "one stop");
}

// ---------------------------------------------------------------------------
// (h) rate independence, with perception in the pulse (16b cut S.1)
// ---------------------------------------------------------------------------

/// **The shipped sensor's numbers with a five-cell reach** (16b cut S.1).
///
/// Everything else is `content/sensors/visual_mk1.json`'s; the reach is short so
/// that a 64-cell window has an *outside*, which is what a scripted approach needs
/// in order to produce a `lost` at all.
fn short_sight() -> tacsim::arena::perception::SensorProfile {
    use mush_atlas::TerrainClass as C;
    use tacsim::arena::perception::{SensorProfile, SignatureFactor};
    let row = |class: C, moving: u32, stopped: u32| SignatureFactor {
        class,
        moving_permille: moving,
        stopped_permille: stopped,
    };
    SensorProfile {
        component: smol_str::SmolStr::new("visual_sensor"),
        max_range_cells: 5,
        max_attenuation: 40,
        classify_after_ms: 3_000,
        sees_wrecks: false,
        // No mesh: two units and one team each, so there is nobody to share with,
        // and leaving it on would only add traces the property is not about.
        share_range_cells: 0,
        signature: vec![row(C::Ground, 1_000, 900), row(C::WoodsHeavy, 800, 500)],
    }
}

/// A tank that sees, and a weapon row so `tempo.alert_range_cm` is derived rather
/// than zero (16b §9).
fn watcher(id: ObjId, team: Team, pose: Pose) -> CustodyUnit {
    CustodyUnit {
        sensor: Some(short_sight()),
        weapons: vec![tacsim::arena::state::WeaponSummary {
            id: SmolStr::new("cannon"),
            max_range_cm: 250_000,
            cycle_ms: 4_000,
            ammunition: SmolStr::new("tacsim.ammunition.he_105"),
            burst_rounds: 1,
            muzzle_speed_cms: 145_000,
            bands: vec![
                tacsim::arena::weapons::WeaponBand {
                    max_range_cm: 150_000,
                    dispersion_mrad: 3,
                },
                tacsim::arena::weapons::WeaponBand {
                    max_range_cm: 250_000,
                    dispersion_mrad: 6,
                },
            ],
            count_per_load: 40,
            // 16b cut W.2: the round's own damage and the section it is mounted in.
            damage: 120,
            mount_section: SmolStr::new("turret"),
        }],
        ..tank(id, team, pose)
    }
    .sealed()
}

/// **The scripted approach**: start up, close on the parked tank, hold, then back
/// away out of the sensor's reach.
///
/// Every beat is at a multiple of 5,000 ms, so it lands at the same arena time at
/// all three cadences — the same rule the approach in [`script`] follows. The
/// reverse leg is what produces the `lost`: `reverse_factor_pct` is 40, so backing
/// away at 400 cm/s takes the tank from one cell out to past five in about
/// 12 seconds, and the TTL (three passes) turns that into one loss.
fn approach() -> Vec<Beat> {
    vec![
        Beat::at(0, Vec::new(), vec![intent(MOVER, IntentKind::Startup, 1)]),
        Beat::at(
            10_000,
            Vec::new(),
            vec![
                intent(MOVER, IntentKind::Heading { mdeg: 0 }, 2),
                intent(MOVER, IntentKind::Speed { cms: 1_000 }, 3),
            ],
        ),
        Beat::at(
            35_000,
            Vec::new(),
            vec![intent(MOVER, IntentKind::Speed { cms: -400 }, 4)],
        ),
    ]
}

#[test]
fn one_scripted_approach_at_three_cadences_is_one_detect_classify_lose_sequence() {
    // The property doc 16 §3.1 states, with cut S.1's due events inside the pulse:
    // the sensor pass is applied at *its own* arena timestamp against the positions
    // the substep boundary committed, so the cadence cannot move a detection, a
    // classification or a loss by one pass.
    //
    // The mover starts twelve cells south of the parked tank with five cells of
    // sight, drives in, and backs out again. The parked tank sees too, so every
    // transition happens twice — once per observer — which is exactly what a
    // per-observer store has to get right.
    let quiet = |_: i64| Tempo::Quiet;
    let mut runs: Vec<(i64, Replay)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = arena_of(
            vec![
                watcher(MOVER, Team::Red, centre(5, 5)),
                watcher(PARKED, Team::Blue, centre(5, 17)),
            ],
            &MatchConfig::default(),
        );
        // Both passes are scheduled at their stagger (`stagger_ms(seed, id,
        // pass_ms)` since 16b cut U.1; it was `id % pass_ms`), one a unit.
        assert_eq!(
            arena
                .due
                .iter()
                .filter(|event| matches!(
                    event.kind,
                    tacsim::arena::state::DueKind::SensorPass { .. }
                ))
                .count(),
            2
        );
        let run = replay(&mut arena, cadence, 60_000, &approach(), &quiet);
        assert_eq!(
            arena.now_ms, 60_000,
            "cadence {cadence} ran to a different time"
        );
        runs.push((cadence, run));
    }

    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };

    // The run is worth comparing: every one of the three transitions happened, on
    // both sides, in that order.
    assert_eq!(first.count("detected"), 2, "{:?}", first.kinds());
    assert_eq!(first.count("classified"), 2, "{:?}", first.kinds());
    assert_eq!(first.count("lost"), 2, "{:?}", first.kinds());
    // `Replay::kinds` answers each event's serialised body, so the lifecycle is
    // picked out by the `kind` field's value inside it.
    let perception: Vec<&'static str> = first
        .kinds()
        .into_iter()
        .filter_map(|encoded| {
            ["detected", "classified", "lost"]
                .into_iter()
                .find(|word| encoded.contains(&format!("\"kind\":\"{word}\"")))
        })
        .collect();
    assert_eq!(
        perception,
        vec![
            "detected",
            "detected",
            "classified",
            "classified",
            "lost",
            "lost"
        ],
        "the lifecycle is out of order: {:?}",
        first.kinds()
    );
    // And every classification carries the pinned draw label.
    let drawn: Vec<&Vec<(String, u64)>> = first
        .events
        .iter()
        .filter(|(_, kind, _, _)| kind.contains("classified"))
        .map(|(_, _, draws, _)| draws)
        .collect();
    assert_eq!(drawn.len(), 2);
    for draws in drawn {
        // 16b cut M.3: a list, and a sensor classification makes exactly one.
        let [(label, _)] = draws.as_slice() else {
            panic!("a classification carries its one draw: {draws:?}");
        };
        assert!(label.starts_with("classify:"), "{label}");
    }

    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events,
            first.events,
            "the event list differs at {cadence} ms: {:?} against {:?}",
            run.kinds(),
            first.kinds()
        );
        assert_eq!(
            run.hashes,
            first.hashes,
            "the per-substep hash sequence differs at {cadence} ms ({} against {})",
            run.hashes.len(),
            first.hashes.len()
        );
        assert_eq!(
            run.finals, first.finals,
            "the final state differs at {cadence} ms"
        );
        assert_ne!(run.pulses, first.pulses, "{cadence} ms ran the same pulses");
    }
}

// ---------------------------------------------------------------------------
// (i) rate independence across a break in line of sight (16b cut F.3)
// ---------------------------------------------------------------------------

/// The canopy's row: heavy woods all the way across, at cost 150 so a tank can
/// drive through it and at an attenuation the five-cell sight cannot see past
/// (the atlas charges 60 a cell of heavy woods against `max_attenuation` 40).
const CANOPY_Y: u16 = 14;

/// The same 64 by 64 window as [`definition`], with the canopy row painted in.
fn canopy_definition() -> ArenaDefinition {
    let mut definition = definition();
    let mut cells: Vec<TerrainCell> = (0..SIDE)
        .flat_map(|y| (0..SIDE).map(move |x| (x, y)))
        .map(|(x, y)| {
            let mut cell = definition.terrain.get(x, y).copied().unwrap_or_else(ground);
            if y == CANOPY_Y {
                cell.class = mush_atlas::TerrainClass::WoodsHeavy;
                cell.move_cost = 150;
            }
            cell
        })
        .collect();
    // The impassable band stays impassable where the canopy crosses it.
    if let Some(cell) =
        cells.get_mut(usize::from(CANOPY_Y) * usize::from(SIDE) + usize::from(BAND_X))
    {
        cell.move_cost = 0;
    }
    definition.terrain = ArenaTerrain::new(SIDE, SIDE, CELL_CM, cells).expect("the canopy window");
    definition
}

/// An arena over [`canopy_definition`], otherwise [`arena_of`]'s.
fn canopy_arena_of(units: Vec<CustodyUnit>) -> Arena {
    let pilots = units
        .iter()
        .enumerate()
        .map(|(seat, unit)| Pilot {
            unit: unit.id,
            pilot: ObjId(1_000 + u32::try_from(seat).unwrap_or(0)),
            station: SmolStr::new("cockpit"),
            epoch: 1,
        })
        .collect();
    Arena::new(
        CustodyOut {
            definition: canopy_definition(),
            units,
            pilots,
            resolution: tacsim::match_::stock_resolution(),
            seed: 2_350,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &ArenaConfig::default(),
        &MatchConfig::default(),
        &SensorsConfig::default(),
    )
    .expect("the canopy arena")
}

/// **The scripted break**: start up, drive north through the canopy until the
/// parked tank is in sight, back away behind the canopy until the contact is
/// lost, and drive back through it to re-acquire.
///
/// Every beat is at a multiple of 5,000 ms, so it lands at the same arena time at
/// all three cadences — the same rule [`script`] and [`approach`] follow.
fn break_and_return() -> Vec<Beat> {
    vec![
        Beat::at(0, Vec::new(), vec![intent(MOVER, IntentKind::Startup, 1)]),
        Beat::at(
            10_000,
            Vec::new(),
            vec![
                intent(MOVER, IntentKind::Heading { mdeg: 0 }, 2),
                intent(MOVER, IntentKind::Speed { cms: 1_000 }, 3),
            ],
        ),
        Beat::at(
            35_000,
            Vec::new(),
            vec![intent(MOVER, IntentKind::Speed { cms: -400 }, 4)],
        ),
        Beat::at(
            60_000,
            Vec::new(),
            vec![intent(MOVER, IntentKind::Speed { cms: 1_000 }, 5)],
        ),
    ]
}

#[test]
fn one_scripted_break_of_sight_at_three_cadences_is_one_lose_and_re_acquire_sequence() {
    // **16b cut F.3's rate-independence lens.** The contact store now carries a
    // plot and a provenance, and both are authoritative state folded into the
    // per-substep hash — so a run in which a target is seen, is lost behind a
    // canopy and is seen again has to produce one event list *and* one hash
    // sequence at all three cadences, or the plot is being recorded against the
    // pulse rather than against the arena timestamp the pass ran at.
    let quiet = |_: i64| Tempo::Quiet;
    let mut runs: Vec<(i64, Replay)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = canopy_arena_of(vec![
            watcher(MOVER, Team::Red, centre(5, 5)),
            watcher(PARKED, Team::Blue, centre(5, 17)),
        ]);
        let run = replay(&mut arena, cadence, 80_000, &break_and_return(), &quiet);
        assert_eq!(
            arena.now_ms, 80_000,
            "cadence {cadence} ran to a different time"
        );
        runs.push((cadence, run));
    }

    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };

    // The run is worth comparing: both platforms acquired, lost and re-acquired.
    assert!(
        first.count("detected") >= 4,
        "the target was never re-acquired: {:?}",
        first.kinds()
    );
    assert!(
        first.count("lost") >= 2,
        "the canopy never broke the track: {:?}",
        first.kinds()
    );
    let lifecycle: Vec<&'static str> = first
        .kinds()
        .into_iter()
        .filter_map(|encoded| {
            ["detected", "lost"]
                .into_iter()
                .find(|word| encoded.contains(&format!("\"kind\":\"{word}\"")))
        })
        .collect();
    let first_lost = lifecycle
        .iter()
        .position(|word| *word == "lost")
        .unwrap_or_else(|| panic!("a loss: {lifecycle:?}"));
    assert!(
        lifecycle[first_lost..].contains(&"detected"),
        "nothing was re-acquired after the loss: {lifecycle:?}"
    );

    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events,
            first.events,
            "the event list differs at {cadence} ms: {:?} against {:?}",
            run.kinds(),
            first.kinds()
        );
        assert_eq!(
            run.hashes,
            first.hashes,
            "the per-substep hash sequence differs at {cadence} ms ({} against {})",
            run.hashes.len(),
            first.hashes.len()
        );
        assert_eq!(
            run.finals, first.finals,
            "the final state differs at {cadence} ms"
        );
    }
}

// ---------------------------------------------------------------------------
// (h) 16b cut D.3 (`B-33`) — the two ends that are ends
// ---------------------------------------------------------------------------

/// **A concede and an unattended match both journal `match_ended`, and both end
/// the match** (16b cut D.3, `B-33`; doc 16 §4's kind list).
///
/// An abort journals `aborted` and a void journals `void`, because neither is a
/// match that reached a conclusion. These two are: a side gave the match up, or
/// the last pilot walked out of it, and in both cases the cells the arena
/// derived are committed and the tanks stand as evidence. So both take the shape
/// a timeout takes — one `match_ended` line carrying its own reason — and a
/// concede names the side that did **not** concede, so nothing downstream has to
/// invert it.
///
/// **17b cut M3.6** extends the loop to EVERY [`StopReason`] — the abort's
/// `aborted`, the world's timeout's `match_ended`, the void's `void` beside
/// the two ends this test was written for — and holds each to the sortie
/// rule: every open sortie closes (`sortie_closed { match_end }`, in unit-id
/// order, at the stop's millisecond under its hash) immediately before the
/// stop's own line, which is still the tape's last (16b cut W.2).
#[test]
fn a_concede_and_an_unattended_match_end_with_their_own_reason() {
    for (stop, reason) in [
        (
            StopReason::Concede { by: Team::Red },
            EndReason::Conceded { winner: Team::Blue },
        ),
        (
            StopReason::Concede { by: Team::Blue },
            EndReason::Conceded { winner: Team::Red },
        ),
        (StopReason::Unattended, EndReason::Unattended),
        (StopReason::Abort, EndReason::Abort),
        (StopReason::Timeout, EndReason::Timeout),
        (StopReason::Void, EndReason::Void),
    ] {
        let mut arena = scripted_arena();
        let mut bytes: Vec<u8> = Vec::new();
        absorb(&arena.step(0, &[], &[]), &mut bytes);
        arena.stop(stop);
        let out = arena.step(5_000, &[], &[]);
        absorb(&out, &mut bytes);
        assert_eq!(out.ended.map(|end| end.reason), Some(reason), "{stop:?}");
        assert_eq!(out.substeps, 0, "{stop:?}");
        assert_eq!(arena.now_ms, 0, "a stop never advances arena time");
        arena.ledger.flush();
        for chunk in arena.ledger.take_chunks() {
            bytes.extend_from_slice(&chunk.bytes);
        }
        let log = lines(&bytes);
        let last = log.last().map(|(_, kind, _, _)| kind.clone());
        let end = match stop {
            StopReason::Abort => EventKind::Aborted,
            StopReason::Void => EventKind::Void,
            _ => EventKind::MatchEnded { reason },
        };
        let expected = serde_json::to_string(&end).expect("an event encodes");
        assert_eq!(
            last,
            Some(expected),
            "{stop:?}: it is an end with its own reason and not an abort"
        );
        m3_closes_before_the_end(&log, &[MOVER, PARKED], end.as_str());
        // And the arena is finished with, exactly as after any other end.
        let after = arena.step(10_000, &[], &[]);
        assert_eq!(after.ended.map(|end| end.reason), Some(reason));
        assert!(after.ledger_chunks.is_empty());
        assert_eq!(after.substeps, 0);
    }
}

/// **Cross-tempo (16b §11, mandatory): one scripted log carrying a stop at a
/// common pulse boundary gives one event list and one hash sequence at 5,000 /
/// 1,000 / 500 ms** (16b cut D.3, `B-33`).
///
/// The stop is delivered at 25,000 ms — a multiple of 5,000, so a pulse boundary
/// exists there at all three cadences — after the log's `STARTUP` and its first
/// helm orders have driven the tank somewhere. What the comparison is about is
/// that the *end* lands at one arena time with one ledger and one hash sequence
/// behind it however the wall clock was cut into pulses: a stop is honoured at
/// phase 1 and never mid-substep (doc 16 §3.1), so the tempo decides nothing
/// about where the match stopped.
#[test]
fn one_scripted_stop_at_three_cadences_is_one_end_one_event_list_and_one_hash_sequence() {
    let quiet = |_: i64| Tempo::Quiet;
    for stop in [
        StopReason::Concede { by: Team::Red },
        StopReason::Unattended,
    ] {
        let mut runs: Vec<(i64, Replay)> = Vec::new();
        for cadence in [5_000_i64, 1_000, 500] {
            let mut arena = scripted_arena();
            let script = vec![
                Beat::at(0, Vec::new(), vec![intent(MOVER, IntentKind::Startup, 1)]),
                Beat::at(
                    10_000,
                    Vec::new(),
                    vec![
                        intent(MOVER, IntentKind::Heading { mdeg: 0 }, 2),
                        intent(MOVER, IntentKind::Speed { cms: 1_000 }, 3),
                    ],
                ),
                Beat::stopping(25_000, stop),
            ];
            let run = replay(&mut arena, cadence, 60_000, &script, &quiet);
            assert_eq!(
                arena.ended.map(|end| end.reason),
                Some(match stop {
                    StopReason::Concede { by } => EndReason::Conceded { winner: by.other() },
                    _ => EndReason::Unattended,
                }),
                "{stop:?} at {cadence} ms did not end the match"
            );
            assert_eq!(
                arena.now_ms, 25_000,
                "{stop:?} at {cadence} ms stopped at a different arena time"
            );
            // 17b cut M3.6: `Arena::stopped` closes both sorties before the
            // stop's own line, at every cadence alike.
            m3_closes_before_the_end(&run.events, &[MOVER, PARKED], "match_ended");
            runs.push((cadence, run));
        }
        let (_, first) = runs.first().expect("three cadences");
        assert!(
            first.count("match_ended") == 1,
            "{stop:?}: {:?}",
            first.kinds()
        );
        for (cadence, run) in runs.iter().skip(1) {
            assert_eq!(
                run.events,
                first.events,
                "{stop:?}: the event list differs at {cadence} ms: {:?} against {:?}",
                run.kinds(),
                first.kinds()
            );
            assert_eq!(
                run.hashes, first.hashes,
                "{stop:?}: the per-substep hash sequence differs at {cadence} ms"
            );
            assert_eq!(
                run.finals, first.finals,
                "{stop:?}: the final state differs at {cadence} ms"
            );
            assert_ne!(
                run.pulses, first.pulses,
                "{stop:?}: {cadence} ms ran the same pulses"
            );
        }
    }
}

/// **16b cut M.2** (16c `T-01`): the duration bound at three cadences. The bound
/// is 22,600 ms — inside a pulse at every cadence and inside a substep — and
/// the tank is under way when it comes, so there is motion to cut off. Before
/// the cut the timeout was settled at the end of whichever pulse crossed it:
/// 25,000 at 5,000 ms, 23,000 at 1,000 and 500. It is a due event now, so it is
/// one `match_ended { timeout }` at 22,600, the ledger's last line, behind one
/// event list and one hash sequence at every cadence.
#[test]
fn m2_a_duration_bound_mid_pulse_is_one_timeout_at_one_millisecond_at_three_cadences() {
    let quiet = |_: i64| Tempo::Quiet;
    let mut runs: Vec<(i64, Replay)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = arena_of(
            vec![
                tank(MOVER, Team::Red, centre(5, 5)),
                tank(PARKED, Team::Blue, centre(5, 40)),
            ],
            &MatchConfig {
                max_duration_ms: 22_600,
                ..MatchConfig::default()
            },
        );
        let script = vec![
            Beat::at(0, Vec::new(), vec![intent(MOVER, IntentKind::Startup, 1)]),
            Beat::at(
                10_000,
                Vec::new(),
                vec![
                    intent(MOVER, IntentKind::Heading { mdeg: 0 }, 2),
                    intent(MOVER, IntentKind::Speed { cms: 1_000 }, 3),
                ],
            ),
        ];
        let run = replay(&mut arena, cadence, 60_000, &script, &quiet);
        assert_eq!(
            arena.ended,
            Some(tacsim::arena::state::MatchEnd {
                reason: EndReason::Timeout,
                t_ms: 22_600
            }),
            "at {cadence} ms the timeout was not at the bound's own millisecond"
        );
        assert_eq!(
            arena.now_ms, 22_750,
            "at {cadence} ms the pulse did not stop at the substep the bound fell in"
        );
        // 17b cut M3.6: the deadline inside a substep closes both sorties
        // first, at the bound's millisecond — the same lines at each cadence
        // (the event lists are compared below).
        m3_closes_before_the_end(&run.events, &[MOVER, PARKED], "match_ended");
        runs.push((cadence, run));
    }
    let (_, first) = runs.first().expect("three cadences");
    assert_eq!(first.count("match_ended"), 1, "{:?}", first.kinds());
    let timeout = serde_json::to_string(&EventKind::MatchEnded {
        reason: EndReason::Timeout,
    })
    .unwrap_or_default();
    assert_eq!(
        first
            .events
            .last()
            .map(|(t_ms, kind, _, _)| (*t_ms, kind.clone())),
        Some((22_600, timeout)),
        "the timeout is the ledger's last line: {:?}",
        first.kinds()
    );
    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events,
            first.events,
            "the event list differs at {cadence} ms: {:?} against {:?}",
            run.kinds(),
            first.kinds()
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the per-substep hash sequence differs at {cadence} ms"
        );
        assert_eq!(
            run.finals, first.finals,
            "the final state differs at {cadence} ms"
        );
        assert_ne!(run.pulses, first.pulses, "{cadence} ms ran the same pulses");
    }
}

// ---------------------------------------------------------------------------
// 16b cut Y.1 — the issuer was state; the spot is not (retired, 17b cut M4.6c)
// ---------------------------------------------------------------------------
//
// `the_hash_folds_the_issuer_and_not_the_spot` stood here from 16b cut Y.1:
// two arenas equal but for a reflex row's standing order's `issuer` hashed
// differently (hash item 3 folded it, because the reflex re-keyed its hint at
// EVERY tick, so the issuer steered later decisions), while the order's `spot`
// and (17b cut L.1) its author `by` moved nothing. **It is retired by the
// owner's STOP C ruling (2026-10-09), in 17b cut M4.6c**, which deleted the
// reflex rows it wrote through and hash item 3 with them: the issuer is now a
// RECORD field beside the mind's order (`MindState::order_issuer`) — D-UM-15
// re-keys a mind's hint once, at write time, in the issuer's scope, so the
// issuer steers nothing after the landing (D-UM-52's `by` is the precedent) —
// carried by the snapshot and never folded; and on a mind row `spot` and `by`
// are not stored at all, so the pin's other halves have no subject left. What
// replaces it: `src/arena/hash.rs`' in-source
// `m3_every_part_of_the_answer_the_drop_and_the_seal_digest_moves_the_hash`
// probes the issuer NOT folded (two rows holding one order, differing only in
// who issued it, hash equal; the order itself is folded); the snapshot carry
// is `m3_a_snapshot_carries_the_minds_with_their_pending_answers` below; and
// the landing's write is `tests/reflex.rs`'
// `t1_live_orders_by_issuer_groups_per_issuer_and_the_machines_under_their_commander`.

// ---------------------------------------------------------------------------
// 16b cut Z.2 — the two squad roles' payloads are state
// ---------------------------------------------------------------------------

/// **16b cut Z.2** (the cut's one hash move): two arenas equal but for a
/// standing `Screen`'s `facing_mdeg` hash differently, and so do two equal but
/// for a `Support`'s charge `of` — the arc a screen watches and the unit a
/// support keeps station on steer the machine, so a replay that lost either
/// must not compare equal. The two roles are two states of their own too.
///
/// **17b cut M4.5a**: the machine is a unit mind, and the order stands on its
/// mind row as the `FrameOrder` it reads (hash item 3b folds the mind row,
/// its order included), where it was a `StandingOrder` on a reflex row. The
/// order is written on the row directly, not through `apply_intent`: the
/// arena re-keys a `Support`'s charge to the hull's own row for it and to
/// `None` for a charge the hull does not hold classified, so two charges the
/// mover has never seen would both read `None` and the "two charges" claim
/// would be vacuous. The row's own scope is the mind's, so the charges are
/// two contact ids of it (`301`, `302`), as the reflex row's were two units.
#[test]
fn the_hash_folds_a_screens_facing_and_a_supports_charge() {
    use tacsim::arena::perception::{FrameOrder, MindObjective};
    use tacsim::arena::state::{ContactId, Controller};
    let under = |objective: MindObjective| {
        let mut mover = tank(MOVER, Team::Red, centre(10, 10));
        mover.controller = Controller::Mind;
        let mut arena = arena_of(
            vec![mover, tank(PARKED, Team::Blue, centre(30, 30))],
            &MatchConfig::default(),
        );
        let Some(row) = arena.minds.get_mut(&MOVER) else {
            panic!("the mover is a mind with a row");
        };
        row.order = Some(FrameOrder {
            squad: SmolStr::new("red-1"),
            objective,
            hold_fire: false,
            max_range_band: None,
            expires_ms: 600_000,
        });
        tacsim::arena::hash::state_hash(&arena)
    };
    let east = under(MindObjective::Screen {
        facing_mdeg: 90_000,
    });
    let west = under(MindObjective::Screen {
        facing_mdeg: 270_000,
    });
    assert_ne!(east, west, "the facing is folded");
    let on_one = under(MindObjective::Support {
        of: Some(ContactId(301)),
    });
    let on_two = under(MindObjective::Support {
        of: Some(ContactId(302)),
    });
    assert_ne!(on_one, on_two, "the charge is folded");
    assert_ne!(east, on_one, "and the two roles are two states");
    assert_eq!(
        under(MindObjective::Screen {
            facing_mdeg: 90_000,
        }),
        east,
        "the fold is a function of the order"
    );
}

// ---------------------------------------------------------------------------
// 16b cut O.1 — a Hold at three cadences (16b §11's mandatory cross-tempo test)
// ---------------------------------------------------------------------------

/// The Hold spot of the O.1 cross-tempo tests: `(13, 30)`, west of the band.
const O1_SPOT: (u16, u16) = (13, 30);

/// The red mover and the blue tank of the O.1 tests.
const O1_RED: ObjId = ObjId(4);
const O1_BLUE: ObjId = ObjId(9);

/// **This file's window with one key spot and a Hold declared on it**:
/// [`O1_SPOT`], three cells round it, ten seconds.
fn o1_hold_definition() -> ArenaDefinition {
    ArenaDefinition {
        key_spots: vec![tacsim::arena::state::KeySpot {
            name: SmolStr::new("The Spot"),
            x: O1_SPOT.0,
            y: O1_SPOT.1,
        }],
        modes: vec![tacsim::arena::ArenaMode::Hold(tacsim::arena::HoldRule {
            spot: 0,
            radius_cells: 3,
            hold_ms: 10_000,
        })],
        ..definition()
    }
}

/// **A Hold match** on [`o1_hold_definition`]: the red mover facing east in
/// column 5 of the spot's row, and `blue` — each unit piloted, with a
/// five-second startup so a `STARTUP` at 0 is online by the 5,000 ms beat.
fn o1_hold_arena(blue: CustodyUnit) -> Arena {
    let mut red = tank(O1_RED, Team::Red, centre(5, O1_SPOT.1));
    red.pose.heading_mdeg = 90_000;
    let units = vec![red, blue];
    let pilots = units
        .iter()
        .enumerate()
        .map(|(seat, unit)| Pilot {
            unit: unit.id,
            pilot: ObjId(1_000 + u32::try_from(seat).unwrap_or(0)),
            station: SmolStr::new("cockpit"),
            epoch: 1,
        })
        .collect();
    Arena::new(
        CustodyOut {
            definition: o1_hold_definition(),
            units,
            pilots,
            resolution: tacsim::match_::stock_resolution(),
            seed: 2_350,
            mode: tacsim::arena::MatchMode::Hold,
        },
        &ArenaConfig::default(),
        &MatchConfig {
            startup_ms: 5_000,
            ..MatchConfig::default()
        },
        &SensorsConfig::default(),
    )
    .expect("the hold arena")
}

/// What one O.1 run produced: the ledger, the per-substep hashes, the Hold
/// clock read at every 5,000 ms boundary, and the end.
struct HoldRun {
    events: Vec<Line>,
    hashes: Vec<u64>,
    clocks: Vec<(i64, Option<(Team, i64)>)>,
    ended: Option<tacsim::arena::state::MatchEnd>,
    pulses: u32,
}

/// Drive `arena` through `script` at `cadence_ms` until arena time reaches
/// `until_ms` or the match ends — [`replay`]'s loop, reading the Hold clock at
/// every 5,000 ms boundary (a pulse boundary at all three cadences).
fn o1_run(arena: &mut Arena, cadence_ms: i64, until_ms: i64, script: &[Beat]) -> HoldRun {
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut clocks: Vec<(i64, Option<(Team, i64)>)> = Vec::new();
    let mut next = 0_usize;
    let mut pulses = 0_u32;
    let mut wall = 0_i64;
    let ended = loop {
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let mut intents: Vec<Intent> = Vec::new();
        while let Some(beat) = script.get(next) {
            if beat.t_ms > arena.now_ms {
                break;
            }
            intents.extend(beat.intents.iter().cloned());
            next += 1;
        }
        let out = arena.step(wall, &[], &intents);
        absorb(&out, &mut bytes);
        hashes.extend(arena.substep_hashes.iter().copied());
        pulses += 1;
        if arena.now_ms % 5_000 == 0 {
            clocks.push((arena.now_ms, arena.hold.and_then(|hold| hold.held_by)));
        }
        if out.ended.is_some() || arena.now_ms >= until_ms {
            break out.ended;
        }
        wall += cadence_ms;
    };
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    HoldRun {
        events: lines(&bytes),
        hashes,
        clocks,
        ended,
        pulses,
    }
}

/// Every `match_ended` line of a run, as `(t_ms, kind)`.
fn o1_ends(run: &HoldRun) -> Vec<(i64, String)> {
    run.events
        .iter()
        .filter(|(_, kind, _, _)| kind.contains("match_ended"))
        .map(|(t_ms, kind, _, _)| (*t_ms, kind.clone()))
        .collect()
}

/// Compare every run with the first: one event list, one hash sequence, one
/// clock at every 5,000 ms boundary, one end — and really three cadences.
fn o1_same_match(runs: &[(i64, HoldRun)]) {
    let Some((_, first)) = runs.first() else {
        panic!("three runs");
    };
    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events, first.events,
            "the event list differs at {cadence} ms"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the hash sequence differs at {cadence} ms"
        );
        assert_eq!(
            run.clocks, first.clocks,
            "the Hold clock differs at {cadence} ms"
        );
        assert_eq!(run.ended, first.ended, "the end differs at {cadence} ms");
        assert_ne!(run.pulses, first.pulses, "{cadence} ms ran the same pulses");
    }
}

/// The red mover's orders: `STARTUP` at 0, drive east at the 5,000 ms beat, and
/// stop at the 15,000 ms beat — inside the square.
fn o1_red_orders() -> Vec<Beat> {
    vec![
        Beat::at(0, Vec::new(), vec![intent(O1_RED, IntentKind::Startup, 1)]),
        Beat::at(
            5_000,
            Vec::new(),
            vec![intent(O1_RED, IntentKind::Speed { cms: 1_000 }, 2)],
        ),
        Beat::at(
            15_000,
            Vec::new(),
            vec![intent(O1_RED, IntentKind::Speed { cms: 0 }, 3)],
        ),
    ]
}

/// **16b cut O.1's first cross-tempo test** (16b §11; doc 16 §3.1, D-16-2;
/// 16c N11): one red mover drives onto the key spot while blue stays off it.
/// At 5,000, 1,000 and 500 ms: one event list, one hash sequence, one clock at
/// every boundary, and exactly one `match_ended { hold: red }` at exactly
/// `since + hold_ms` — a substep boundary plus whole seconds, so on the
/// substep lattice at every cadence.
#[test]
fn o1_a_hold_is_won_at_one_millisecond_at_three_cadences() {
    let mut runs: Vec<(i64, HoldRun)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = o1_hold_arena(tank(O1_BLUE, Team::Blue, centre(50, 30)));
        let run = o1_run(&mut arena, cadence, 60_000, &o1_red_orders());
        // Where the mover stopped: inside the square.
        let cell = arena
            .unit(O1_RED)
            .and_then(|unit| arena.definition.terrain.cell_of(&unit.pose))
            .expect("on the window");
        assert!(
            arena.hold.is_some_and(|hold| hold.square_contains(cell)),
            "the mover stopped at {cell:?}, off the square"
        );
        runs.push((cadence, run));
    }
    o1_same_match(&runs);
    let Some((_, first)) = runs.first() else {
        panic!("three runs");
    };
    let since = first
        .clocks
        .iter()
        .find_map(|(_, held)| *held)
        .map(|(team, since)| {
            assert_eq!(team, Team::Red);
            since
        })
        .expect("red held the spot at a 5,000 ms boundary");
    assert_eq!(since % 250, 0, "the clock opens on a substep boundary");
    let won_at = since + 10_000;
    assert_eq!(
        first.ended,
        Some(tacsim::arena::state::MatchEnd {
            reason: EndReason::Hold { winner: Team::Red },
            t_ms: won_at
        })
    );
    let ends = o1_ends(first);
    assert_eq!(ends.len(), 1, "exactly one end: {ends:?}");
    assert_eq!(ends.first().map(|(t_ms, _)| *t_ms), Some(won_at));
    assert!(
        ends.iter()
            .all(|(_, kind)| kind.contains(r#"{"hold":{"winner":"red"}}"#)),
        "{ends:?}"
    );
    assert!(
        first.events.iter().all(|(t_ms, _, _, _)| *t_ms <= won_at),
        "a line after the win"
    );
    // 17b cut M3.6: a hold won inside a substep (one of the three in-substep
    // ends) closes both sorties first, at the win's millisecond.
    m3_closes_before_the_end(&first.events, &[O1_RED, O1_BLUE], "match_ended");
}

/// **16b cut O.1's second cross-tempo test**: the red mover holds the spot, and
/// a blue mover, ordered south at the 10,000 ms beat, drives into the square
/// before red's clock runs out — the square is contested and the clock drops,
/// at the same substep at every cadence; blue drives on through the square and
/// out of its south side, and red, alone again, re-opens the clock with a
/// later `since` and wins at that one.
#[test]
fn o1_a_contested_hold_resets_and_the_match_runs_on_at_three_cadences() {
    let blue_orders = || {
        let mut script = o1_red_orders();
        script.retain(|beat| beat.t_ms != 0);
        let mut beats = vec![Beat::at(
            0,
            Vec::new(),
            vec![
                intent(O1_RED, IntentKind::Startup, 1),
                intent(O1_BLUE, IntentKind::Startup, 1),
            ],
        )];
        beats.extend(script);
        beats.push(Beat::at(
            10_000,
            Vec::new(),
            vec![intent(O1_BLUE, IntentKind::Speed { cms: 1_000 }, 2)],
        ));
        beats.sort_by_key(|beat| beat.t_ms);
        beats
    };
    let mut runs: Vec<(i64, HoldRun)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        // Blue north of the spot, facing south down its column.
        let mut blue = tank(O1_BLUE, Team::Blue, centre(O1_SPOT.0, 40));
        blue.pose.heading_mdeg = 180_000;
        let mut arena = o1_hold_arena(blue);
        let run = o1_run(&mut arena, cadence, 90_000, &blue_orders());
        runs.push((cadence, run));
    }
    o1_same_match(&runs);
    let Some((_, first)) = runs.first() else {
        panic!("three runs");
    };
    // Red opens a clock, the contest drops it, and red re-opens a later one.
    let opened: Vec<i64> = first
        .clocks
        .iter()
        .filter_map(|(_, held)| *held)
        .map(|(team, since)| {
            assert_eq!(team, Team::Red, "only red is ever alone on the spot");
            since
        })
        .collect();
    let first_since = opened.first().copied().expect("red held the spot");
    let dropped = first
        .clocks
        .iter()
        .skip_while(|(_, held)| held.is_none())
        .any(|(_, held)| held.is_none());
    assert!(
        dropped,
        "the contest never dropped the clock: {:?}",
        first.clocks
    );
    let last_since = opened.last().copied().expect("red held the spot again");
    assert!(
        last_since > first_since,
        "the second clock opened later: {first_since} then {last_since} ({:?})",
        first.clocks
    );
    // No end at the first clock's millisecond; one at the second's.
    let ends = o1_ends(first);
    assert_eq!(
        ends.iter().map(|(t_ms, _)| *t_ms).collect::<Vec<_>>(),
        vec![last_since + 10_000],
        "{ends:?}"
    );
    assert_eq!(
        first.ended.map(|end| end.reason),
        Some(EndReason::Hold { winner: Team::Red })
    );
}

// ---------------------------------------------------------------------------
// 16b cut AE.1 — each unit pays its own row, at three cadences
// ---------------------------------------------------------------------------

/// [`tactical_unit`] with one ground mode of `locomotion` (16b cut AE.1): what
/// the check-out reads a unit's row and slope limit off.
fn tactical_unit_on(id: ObjId, locomotion: &str) -> TacticalUnit {
    use mush_tactics::{
        LocomotionId, MediumId, MobilityMode, MobilityModeId, MobilityProfile, MobilityVocabulary,
    };
    let mode_id = MobilityModeId::new(format!("ground.{locomotion}")).expect("a mode id");
    let mode = MobilityMode::new(
        mode_id.clone(),
        [MediumId::new("ground").expect("a medium")],
        LocomotionId::new(locomotion).expect("a locomotion"),
        rules(),
    );
    let profile = MobilityProfile::new(vec![mode], mode_id, &MobilityVocabulary::base().freeze())
        .expect("a one-mode profile");
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
        mobility: Some(profile),
        rules: rules(),
    };
    TacticalUnit::from_spec(&spec, id).expect("a one-section unit with a mode")
}

/// The walker of the AE.1 script: [`MOVER`], so its pilot is the one the
/// replay driver listens to.
const AE1_WALKER: ObjId = MOVER;
/// The tank beside it.
const AE1_TANK: ObjId = ObjId(5);
/// The car, at the rubble.
const AE1_CAR: ObjId = ObjId(6);

/// **The AE.1 window** (16b cut AE.1): 64 by 64 of grass, priced per row
/// through `ArenaTerrain::priced` and the shipped table, with a band of thick
/// woods at columns 20 to 23 on rows 0 to 29 and a band of rubble at the same
/// columns on rows 30 to 63 — what `from_pack` would build over those classes.
fn ae1_definition() -> ArenaDefinition {
    use mush_atlas::TerrainClass;
    let table = mush_atlas::physics::LocomotionTable::shipped();
    let cell = |class: TerrainClass| TerrainCell {
        class,
        elev_cm: 0,
        move_cost: table
            .cost(class.label(), "tracked", &[])
            .map_or(0, |cost| u16::try_from(cost).unwrap_or(u16::MAX)),
    };
    let mut cells = Vec::with_capacity(usize::from(SIDE) * usize::from(SIDE));
    for y in 0..SIDE {
        for x in 0..SIDE {
            cells.push(cell(if !(20..=23).contains(&x) {
                TerrainClass::Grass
            } else if y < 30 {
                TerrainClass::WoodsHeavy
            } else {
                TerrainClass::Rubble
            }));
        }
    }
    let mut definition = definition();
    definition.terrain =
        ArenaTerrain::priced(SIDE, SIDE, CELL_CM, cells, table).expect("the priced window");
    definition
}

/// The three units, seated, on [`ae1_definition`]: a walker at (5, 10), a tank
/// at (5, 20) and a car at (5, 40), all facing east.
fn ae1_arena() -> Arena {
    let mut walker = tank(AE1_WALKER, Team::Red, centre(5, 10));
    walker.unit = tactical_unit_on(AE1_WALKER, "legged");
    let hull = tank(AE1_TANK, Team::Red, centre(5, 20));
    let mut car = tank(AE1_CAR, Team::Red, centre(5, 40));
    car.unit = tactical_unit_on(AE1_CAR, "wheeled");
    let units = vec![walker, hull, car];
    let pilots = units
        .iter()
        .enumerate()
        .map(|(seat, unit)| Pilot {
            unit: unit.id,
            pilot: ObjId(1_000 + u32::try_from(seat).unwrap_or(0)),
            station: SmolStr::new("cockpit"),
            epoch: 1,
        })
        .collect();
    Arena::new(
        CustodyOut {
            definition: ae1_definition(),
            units,
            pilots,
            resolution: tacsim::match_::stock_resolution(),
            seed: 2_350,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &ArenaConfig::default(),
        &MatchConfig::default(),
        &SensorsConfig::default(),
    )
    .expect("the AE.1 arena")
}

/// The AE.1 script: all three started at arena time 0, then driven east at
/// 1,000 cm/s from 10,000 ms — a pulse boundary at every cadence.
fn ae1_script() -> Vec<Beat> {
    let mut beats = vec![Beat::at(
        0,
        Vec::new(),
        [AE1_WALKER, AE1_TANK, AE1_CAR]
            .into_iter()
            .map(|unit| intent(unit, IntentKind::Startup, 1))
            .collect(),
    )];
    beats.push(Beat::at(
        10_000,
        Vec::new(),
        [AE1_WALKER, AE1_TANK, AE1_CAR]
            .into_iter()
            .flat_map(|unit| {
                [
                    intent(unit, IntentKind::Heading { mdeg: 90_000 }, 2),
                    intent(unit, IntentKind::Speed { cms: 1_000 }, 3),
                ]
            })
            .collect(),
    ));
    beats
}

/// **16b cut AE.1's mandatory cross-tempo test** (16b §11; doc 16 §3.1 "Rate
/// independence", D-16-2; 16c `B-58`): one scripted walk over the thick woods
/// at 5,000, 1,000 and 500 ms is one event list, one per-substep hash sequence,
/// one set of finals and one notice list.
///
/// The script: a walker and a tank driven east at a band of `WoodsHeavy`, and a
/// car driven east at a band of `Rubble`, on a window priced per row. The
/// walker's `legged` row prices the woods at 200 and it crosses; the tank's
/// `tracked` row and the car's `wheeled` row refuse their bands, each one
/// `blocked` line, cause `impassable`, at the band's west edge. Before the
/// replays are compared the run is asserted to have done that, so three runs
/// that did nothing cannot compare equal and prove nothing.
#[test]
fn ae1_one_scripted_walk_over_the_thick_woods_at_three_cadences_is_one_event_list_and_one_hash_sequence()
 {
    use tacsim::arena::CostRow;

    let quiet = |_: i64| Tempo::Quiet;
    let mut runs: Vec<(i64, Replay)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = ae1_arena();
        assert_eq!(
            [AE1_WALKER, AE1_TANK, AE1_CAR].map(|id| arena.unit(id).map(|unit| unit.cost_row)),
            [
                Some(CostRow::Legged),
                Some(CostRow::Tracked),
                Some(CostRow::Wheeled)
            ]
        );
        let run = replay(&mut arena, cadence, 60_000, &ae1_script(), &quiet);
        assert_eq!(
            arena.now_ms, 60_000,
            "cadence {cadence} ran to a different time"
        );
        runs.push((cadence, run));
    }
    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };

    // What the run did: the tank and the car each stopped once, at the band,
    // by the ground; the walker crossed it.
    let blocked: Vec<(ObjId, u16, &str)> = first
        .events
        .iter()
        .filter(|(_, kind, _, _)| kind.contains("\"blocked\""))
        .map(|(_, kind, _, _)| {
            let line: serde_json::Value = serde_json::from_str(kind).unwrap_or_default();
            (
                ObjId(
                    line.get("unit")
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|id| u32::try_from(id).ok())
                        .unwrap_or(0),
                ),
                line.get("x")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|x| u16::try_from(x).ok())
                    .unwrap_or(0),
                if kind.contains("\"cause\":\"impassable\"") {
                    "impassable"
                } else {
                    "other"
                },
            )
        })
        .collect();
    assert_eq!(
        blocked,
        vec![(AE1_TANK, 20, "impassable"), (AE1_CAR, 20, "impassable")],
        "the tank at the woods and the car at the rubble, and nothing else: {:?}",
        first.kinds()
    );
    let x_of = |id: ObjId| {
        first
            .finals
            .iter()
            .find(|(unit, ..)| *unit == id)
            .map(|(_, x, ..)| *x)
            .unwrap_or_else(|| panic!("{id} in the finals"))
    };
    assert!(
        x_of(AE1_WALKER) >= 24 * i64::from(CELL_CM),
        "the walker crossed the woods: {}",
        x_of(AE1_WALKER)
    );
    assert_eq!(x_of(AE1_TANK), 19_999, "the tank is at the band's boundary");
    assert_eq!(x_of(AE1_CAR), 19_999, "the car is at the band's boundary");
    assert!(
        first.told.iter().all(|line| !line.starts_with("Held:")),
        "the walker was held by nothing: {:?}",
        first.told
    );

    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events,
            first.events,
            "the event list differs at {cadence} ms: {:?} against {:?}",
            run.kinds(),
            first.kinds()
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the per-substep hash sequence differs at {cadence} ms"
        );
        assert_eq!(
            run.finals, first.finals,
            "the finals differ at {cadence} ms"
        );
        assert_eq!(
            run.substeps, first.substeps,
            "the substeps differ at {cadence} ms"
        );
        assert_eq!(
            run.told, first.told,
            "the pilot was told otherwise at {cadence} ms"
        );
        assert_ne!(run.pulses, first.pulses, "{cadence} ms ran the same pulses");
    }
}

// ---------------------------------------------------------------------------
// 16b cut AE.3 — a drone a side, at three cadences
// ---------------------------------------------------------------------------

/// Red's drone: [`MOVER`], so its pilot is the one the replay driver hears.
const AE3_RED_DRONE: ObjId = MOVER;
/// Red's tank, behind it.
const AE3_RED_TANK: ObjId = ObjId(5);
/// Blue's drone: [`PARKED`].
const AE3_BLUE_DRONE: ObjId = PARKED;
/// Blue's tank, behind it.
const AE3_BLUE_TANK: ObjId = ObjId(10);

/// **A sensor on the mesh** (16b cut AE.3): [`short_sight`]'s rows at `cells`
/// of reach and thirty cells of link, the shipped `share_range_cells`.
fn ae3_sight(cells: u16) -> tacsim::arena::perception::SensorProfile {
    tacsim::arena::perception::SensorProfile {
        max_range_cells: cells,
        share_range_cells: 30,
        ..short_sight()
    }
}

/// **A drone** (16b cut AE.3): [`tank`] with a five-cell sight on the mesh,
/// no weapon, and a rules blob declaring `"class": "drone"` — the one fact
/// `Arena::new` reads the class off.
fn ae3_drone(id: ObjId, team: Team, at: Pose) -> CustodyUnit {
    CustodyUnit {
        sensor: Some(ae3_sight(5)),
        unit: tactical_unit_ruled(
            id,
            RulesValue::new(
                dref("schema", "tacsim.arena.test"),
                serde_json::json!({ "class": "drone" }),
            )
            .expect("a rules blob"),
        ),
        ..tank(id, team, at)
    }
    .sealed()
}

/// The four units, seated: each side's drone five cells out in front of its
/// tank, the drones in columns 5 and 7 so they pass each other, blue's facing
/// south. The tanks see two cells, so everything they hold of the other side
/// is what their drone shared.
fn ae3_arena() -> Arena {
    let mut blue_drone = ae3_drone(AE3_BLUE_DRONE, Team::Blue, centre(7, 30));
    blue_drone.pose.heading_mdeg = 180_000;
    let mut blue_tank = tank(AE3_BLUE_TANK, Team::Blue, centre(3, 35));
    blue_tank.pose.heading_mdeg = 180_000;
    blue_tank.sensor = Some(ae3_sight(2));
    let mut red_tank = tank(AE3_RED_TANK, Team::Red, centre(3, 0));
    red_tank.sensor = Some(ae3_sight(2));
    arena_of(
        vec![
            ae3_drone(AE3_RED_DRONE, Team::Red, centre(5, 5)),
            red_tank,
            blue_drone,
            blue_tank,
        ],
        &MatchConfig::default(),
    )
}

/// The AE.3 script: every unit started at 0; the two drones driven at each
/// other at 10,000 ms and stopped at 25,000, side by side and in each other's
/// sight; red's tank wrecked at 40,000. Every beat a multiple of 5,000 ms.
fn ae3_script() -> Vec<Beat> {
    let all = [AE3_RED_DRONE, AE3_RED_TANK, AE3_BLUE_DRONE, AE3_BLUE_TANK];
    let drones = [(AE3_RED_DRONE, 0_u32), (AE3_BLUE_DRONE, 180_000)];
    vec![
        Beat::at(
            0,
            Vec::new(),
            all.into_iter()
                .map(|unit| intent(unit, IntentKind::Startup, 1))
                .collect(),
        ),
        Beat::at(
            10_000,
            Vec::new(),
            drones
                .into_iter()
                .flat_map(|(unit, mdeg)| {
                    [
                        intent(unit, IntentKind::Heading { mdeg }, 2),
                        intent(unit, IntentKind::Speed { cms: 1_000 }, 3),
                    ]
                })
                .collect(),
        ),
        Beat::at(
            25_000,
            Vec::new(),
            drones
                .into_iter()
                .map(|(unit, _)| intent(unit, IntentKind::Speed { cms: 0 }, 4))
                .collect(),
        ),
        Beat {
            wreck: Some(AE3_RED_TANK),
            ..Beat::at(40_000, Vec::new(), Vec::new())
        },
    ]
}

/// **16b cut AE.3's mandatory cross-tempo test** (16b §11; doc 16 §3.1 "Rate
/// independence", D-16-2; §2 W2): one scripted match with a drone and a tank a
/// side, the drones scouting ahead, at 5,000, 1,000 and 500 ms is one event
/// list, one per-substep hash sequence and one set of finals.
///
/// What the run does, asserted before the replays are compared so three runs
/// that did nothing cannot compare equal: the drones find each other, each
/// tank holds the other side's drone as the plot its own drone shared (its
/// two-cell sight took nothing; the mesh's copy journals its own `detected`
/// and `classified`, so four of each), and when
/// red's tank is wrecked at 40,000 red has only its drone in action — so red is
/// out and blue wins, one `match_ended` at 40,250, the end of the first
/// substep after the wreck at every cadence.
#[test]
fn ae3_one_scripted_match_with_a_drone_a_side_at_three_cadences_is_one_event_list_and_one_hash_sequence()
 {
    use tacsim::definitions::UnitClass;

    let quiet = |_: i64| Tempo::Quiet;

    // The share, read before the wreck on one run of its own.
    let mut scouted = ae3_arena();
    assert_eq!(
        [AE3_RED_DRONE, AE3_RED_TANK, AE3_BLUE_DRONE, AE3_BLUE_TANK]
            .map(|id| scouted.unit(id).map(|unit| unit.class)),
        [
            Some(UnitClass::Drone),
            Some(UnitClass::Armour),
            Some(UnitClass::Drone),
            Some(UnitClass::Armour)
        ]
    );
    let _ = replay(&mut scouted, 500, 35_000, &ae3_script(), &quiet);
    for (tank, drone) in [
        (AE3_RED_TANK, AE3_BLUE_DRONE),
        (AE3_BLUE_TANK, AE3_RED_DRONE),
    ] {
        let copy = scouted
            .contacts
            .id_of(tank, drone)
            .and_then(|id| scouted.contacts.get(tank, id))
            .copied()
            .unwrap_or_else(|| panic!("{tank} holds nothing of {drone}"));
        assert!(
            copy.own_seen.is_none(),
            "{tank}'s two-cell sight never saw {drone}: the plot is its drone's"
        );
    }
    assert_eq!(scouted.eliminated(), None, "both tanks stand at 35,000");

    let mut runs: Vec<(i64, Replay)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = ae3_arena();
        let run = replay(&mut arena, cadence, 60_000, &ae3_script(), &quiet);
        assert_eq!(
            arena.ended.map(|end| (end.reason, end.t_ms)),
            Some((
                EndReason::Elimination {
                    winner: Some(Team::Blue)
                },
                40_250
            )),
            "cadence {cadence}: red's guns are gone and its drone decides nothing"
        );
        assert!(
            arena
                .unit(AE3_RED_DRONE)
                .is_some_and(|drone| !drone.limits.wreck && !drone.disabled),
            "cadence {cadence}: the drone still runs"
        );
        runs.push((cadence, run));
    }
    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };
    // Four each: the two drones by their own sight, and the two tanks by the
    // mesh — a shared plot opens the tank's row with its own `detected`.
    let found: Vec<(u64, u64)> = first
        .events
        .iter()
        .filter(|(_, kind, _, _)| kind.contains("\"kind\":\"detected\""))
        .map(|(_, kind, _, _)| {
            let line: serde_json::Value = serde_json::from_str(kind).unwrap_or_default();
            let field = |name: &str| {
                line.get(name)
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0)
            };
            (field("viewer"), field("target"))
        })
        .collect();
    assert_eq!(
        found,
        vec![(9, 4), (4, 9), (10, 4), (5, 9)],
        "each drone sees the other, and each tank holds the other drone through its own"
    );
    assert_eq!(first.count("\"classified\""), 4, "{:?}", first.kinds());
    assert_eq!(first.count("match_ended"), 1, "{:?}", first.kinds());

    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events,
            first.events,
            "the event list differs at {cadence} ms: {:?} against {:?}",
            run.kinds(),
            first.kinds()
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the per-substep hash sequence differs at {cadence} ms"
        );
        assert_eq!(
            run.finals, first.finals,
            "the finals differ at {cadence} ms"
        );
        assert_eq!(
            run.substeps, first.substeps,
            "the substeps differ at {cadence} ms"
        );
        // `told` is compared line for line with ONE field masked: the drone
        // pilot's `New contact X1: bearing NNN°, band.` takes its bearing and
        // band from the frame's contact row (`cockpit::notice_line`), read
        // where the drone stands when the frame is built — the end of the
        // pulse — to the plot, and the two drones are passing each other, so
        // that reading is the frame's and not the match's. Every other word of
        // every line, and the order and count of the lines, is the match's
        // and compared exactly.
        assert_eq!(
            ae3_masked(&run.told),
            ae3_masked(&first.told),
            "the pilot was told something different at {cadence} ms"
        );
        assert_ne!(run.pulses, first.pulses, "{cadence} ms ran the same pulses");
    }
    assert!(
        ae3_masked(&first.told)
            .iter()
            .any(|line| line.starts_with("New contact ") && line.ends_with(": bearing ___.")),
        "the drone pilot was told of the other drone: {:?}",
        first.told
    );
}

/// **The pilot's lines with the frame-time reading masked** (16b cut AE.3):
/// a `New contact …: bearing NNN°, band.` line keeps everything before
/// `: bearing` and reads `: bearing ___.` — the bearing and the band are read
/// off the frame's contact row at the end of the pulse, from a moving
/// observer, so they are the frame's and not the match's. Every other line is
/// returned untouched.
fn ae3_masked(told: &[String]) -> Vec<String> {
    told.iter()
        .map(|line| match line.split_once(": bearing ") {
            Some((head, _)) if line.starts_with("New contact ") => {
                format!("{head}: bearing ___.")
            }
            _ => line.clone(),
        })
        .collect()
}

/// **16b cut AE.3's Hold half at three cadences** (16b §11; doc 16 §3.1,
/// D-16-2; §2 W2): a blue drone parked inside the declared square, and O.1's
/// red mover driving onto the spot. A drone holds nothing and contests
/// nothing (`Arena::hold_presence`), so at 5,000, 1,000 and 500 ms the match
/// is one event list, one hash sequence and one Hold clock, and red wins it
/// at the SAME millisecond, with the same clock at every boundary, as O.1's
/// first test where blue's tank stands forty cells off the spot. Before
/// AE.3 the drone contested the square and the Hold never ended.
#[test]
fn ae3_a_drone_on_the_spot_contests_nothing_at_three_cadences() {
    use tacsim::definitions::UnitClass;

    // The reference: O.1's first match, blue's tank far off the spot.
    let mut reference = o1_hold_arena(tank(O1_BLUE, Team::Blue, centre(50, 30)));
    let reference = o1_run(&mut reference, 5_000, 60_000, &o1_red_orders());
    assert_eq!(
        reference.ended.map(|end| end.reason),
        Some(EndReason::Hold { winner: Team::Red })
    );

    let mut runs: Vec<(i64, HoldRun)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        // Two cells south of the spot, off red's row, inside the square.
        let mut arena = o1_hold_arena(ae3_drone(
            O1_BLUE,
            Team::Blue,
            centre(O1_SPOT.0, O1_SPOT.1 + 2),
        ));
        assert_eq!(
            arena.unit(O1_BLUE).map(|unit| unit.class),
            Some(UnitClass::Drone)
        );
        let drone_cell = arena
            .unit(O1_BLUE)
            .and_then(|unit| arena.definition.terrain.cell_of(&unit.pose))
            .expect("on the window");
        assert!(
            arena
                .hold
                .is_some_and(|hold| hold.square_contains(drone_cell)),
            "the drone is parked at {drone_cell:?}, off the square"
        );
        let run = o1_run(&mut arena, cadence, 60_000, &o1_red_orders());
        assert!(
            arena
                .unit(O1_BLUE)
                .is_some_and(|drone| !drone.limits.wreck && !drone.disabled),
            "cadence {cadence}: the drone is on the square to the end"
        );
        runs.push((cadence, run));
    }
    o1_same_match(&runs);
    let Some((_, first)) = runs.first() else {
        panic!("three runs");
    };
    assert_eq!(
        first.clocks, reference.clocks,
        "the drone on the square moved red's clock"
    );
    assert_eq!(
        first.ended, reference.ended,
        "red wins at the millisecond it wins with no drone on the square"
    );
    let ends = o1_ends(first);
    assert_eq!(ends.len(), 1, "exactly one end: {ends:?}");
    assert!(
        ends.iter()
            .all(|(_, kind)| kind.contains(r#"{"hold":{"winner":"red"}}"#)),
        "{ends:?}"
    );
}

// ---------------------------------------------------------------------------
// (af1) the snapshot: 16b cut AF.1 (16c N16; doc 16 §3.1 "Durability")
// ---------------------------------------------------------------------------

/// **Rewrite a snapshot document's state and stamp its digest again** (16b cut
/// AF.1): what a test does to move a field *past* the digest check, so the
/// check under test is the one after it.
fn restamp(document: &mut serde_json::Value, state: &str) {
    use sha2::Digest as _;
    document["state"] = serde_json::Value::String(state.to_owned());
    document["state_digest"] = serde_json::Value::String(tacsim::arena::snapshot::digest_text(
        &sha2::Sha256::digest(state.as_bytes()).into(),
    ));
}

/// A snapshot's state string, parsed.
fn state_of(snapshot: &Snapshot) -> serde_json::Value {
    serde_json::from_slice(snapshot.bytes()).expect("the state string is JSON")
}

/// **Whether the ledger's open chunk rode in the snapshot** — the buffer a
/// flush before the encode would have emptied.
fn carries_open_chunk(snapshot: &Snapshot) -> bool {
    state_of(snapshot)
        .pointer("/ledger/open")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|open| !open.is_empty())
}

/// An empty run, for a resume taken after the last pulse: the continuation is
/// nothing but [`finish_replay`]'s flush.
fn empty_replay() -> Replay {
    Replay {
        tape: Vec::new(),
        events: Vec::new(),
        hashes: Vec::new(),
        finals: Vec::new(),
        pulses: 0,
        substeps: 0,
        told: Vec::new(),
        pulses_seen: Vec::new(),
        catchups: Vec::new(),
        observers: Vec::new(),
        deliberation: Vec::new(),
    }
}

/// **Two segments as one run** (16b cut AF.1): the first segment's tape, hashes,
/// notices, pulse rows and reports followed by the resumed one's, and the
/// resumed one's finals.
fn joined(first: &Replay, second: &Replay) -> Replay {
    let mut tape = first.tape.clone();
    tape.extend_from_slice(&second.tape);
    Replay {
        events: lines(&tape),
        tape,
        hashes: first.hashes.iter().chain(&second.hashes).copied().collect(),
        finals: second.finals.clone(),
        pulses: first.pulses + second.pulses,
        substeps: first.substeps + second.substeps,
        told: first.told.iter().chain(&second.told).cloned().collect(),
        pulses_seen: first
            .pulses_seen
            .iter()
            .chain(&second.pulses_seen)
            .copied()
            .collect(),
        catchups: first
            .catchups
            .iter()
            .chain(&second.catchups)
            .cloned()
            .collect(),
        observers: first
            .observers
            .iter()
            .chain(&second.observers)
            .copied()
            .collect(),
        deliberation: first
            .deliberation
            .iter()
            .chain(&second.deliberation)
            .cloned()
            .collect(),
    }
}

/// What one resume produced: the two segments joined, whether the snapshot
/// carried an open chunk, the snapshot's state, and the resumed ledger's two
/// SHA-256 running digests at the end of the run (`digest`, `shipped_digest`) —
/// what the result record signs, and what neither the tape nor the state hash
/// can see.
struct Resumed {
    run: Replay,
    open_chunk: bool,
    state: serde_json::Value,
    digests: ([u8; 32], [u8; 32]),
}

/// A ledger's two running digests, [`Resumed::digests`]' shape.
fn digests_of(arena: &Arena) -> ([u8; 32], [u8; 32]) {
    (arena.ledger.digest(), arena.ledger.shipped_digest())
}

/// **A match stepped to pulse `k` at `cadence_ms`, snapshotted, decoded and
/// resumed** (16b cut AF.1): the first segment over the walls `0 … k ×
/// cadence` from the script's first beat and without a flush; the encode, with
/// the head's hash held to the arena's and the head read without its state;
/// the decode on `ground`; `resume_at` on the last wall the first segment
/// stepped at; and the rest of the script from the cursor the first segment
/// reached, over the walls from `(k + 1) × cadence` to `until_ms`, then the
/// flush. Asserts the script was consumed exactly once between the two.
fn resume_at_pulse(
    mut arena: Arena,
    ground: ArenaDefinition,
    cadence_ms: i64,
    k: i64,
    until_ms: i64,
    script: &[Beat],
    pin: &dyn Fn(i64) -> Tempo,
) -> Resumed {
    let (first, cursor) = replay_on_from(
        &mut arena,
        &mut (0..=k).map(|pulse| pulse * cadence_ms),
        k * cadence_ms,
        script,
        0,
        pin,
    );
    assert_eq!(
        arena.now_ms,
        k * cadence_ms,
        "the first segment ran to pulse {k}"
    );
    let snapshot = Snapshot::encode(&arena).expect("an arena between two pulses encodes");
    let head = Snapshot::head(&snapshot.document).expect("the head reads");
    assert_eq!(head.state_hash, tacsim::arena::hash::state_hash(&arena));
    assert_eq!(head.state_hash, snapshot.state_hash);
    assert_eq!(
        (head.at_ms, head.ledger_seq, head.chunk_seq),
        (
            arena.now_ms,
            arena.ledger.seq(),
            arena.ledger.last_closed_seq()
        )
    );
    // The head is read without the state: a document whose state is not a
    // state at all answers the same head.
    let mut blind = snapshot.document.clone();
    blind["state"] = serde_json::Value::String("x".to_owned());
    assert_eq!(Snapshot::head(&blind), Ok(head));
    let mut resumed = Snapshot::decode(&snapshot.document, ground).expect("the snapshot decodes");
    // The fixed-cadence driver steps at its own walls and reads no deadline;
    // `resume_at`'s answer is `af1_the_task_s_own_wakes_resume_on_the_lattice`'s.
    let _first_due = resumed.resume_at(k * cadence_ms);
    let (mut second, reached) = if resumed.now_ms < until_ms {
        replay_on_from(
            &mut resumed,
            &mut ((k + 1)..).map(|pulse| pulse * cadence_ms),
            until_ms,
            script,
            cursor,
            pin,
        )
    } else {
        (empty_replay(), cursor)
    };
    finish_replay(&mut resumed, &mut second);
    assert_eq!(
        reached,
        script.len(),
        "the two segments consumed the script exactly once (the first reached {cursor})"
    );
    Resumed {
        run: joined(&first, &second),
        open_chunk: carries_open_chunk(&snapshot),
        state: state_of(&snapshot),
        digests: digests_of(&resumed),
    }
}

/// **Two runs are one match**: one tape, byte for byte and line by line, one
/// per-substep hash sequence, one final state, one notice list, one substep
/// count and one list of squad signals (what the carried squad watch and
/// expired set steer). `what` names the resume in the message.
///
/// **The frame flags of `pulses_seen` are not compared**, deliberately:
/// `Arena::last_frames` is a view the snapshot resets, so a resumed pilot's
/// first frame is a keyframe even where the uninterrupted run's pulse sent a
/// delta or nothing. That is what a pilot's client gets on any reclaim, and it
/// moves no hash and no line — the tape comparison is the proof.
fn same_match(resumed: &Replay, reference: &Replay, what: &str) {
    assert_eq!(
        resumed.tape, reference.tape,
        "{what}: the tape is not byte-identical"
    );
    assert_eq!(
        resumed.events,
        reference.events,
        "{what}: the event list differs: {:?} against {:?}",
        resumed.kinds(),
        reference.kinds()
    );
    assert_eq!(
        resumed.hashes, reference.hashes,
        "{what}: the per-substep hash sequence differs"
    );
    assert_eq!(
        resumed.finals, reference.finals,
        "{what}: the finals differ"
    );
    assert_eq!(resumed.told, reference.told, "{what}: the notices differ");
    assert_eq!(
        resumed.substeps, reference.substeps,
        "{what}: a different number of substeps ran"
    );
    assert_eq!(
        resumed.deliberation, reference.deliberation,
        "{what}: the squad signals differ"
    );
}

/// **16b cut AF.1's first proof** (16c N16; doc 16 §3.1 "Durability"): the (a)
/// script at 5,000 ms, snapshotted after **every** pulse from the opening one
/// to the last, each decoded on its ground and resumed, is the uninterrupted
/// run — one tape byte for byte, one hash sequence, one final state, one notice
/// list — and the script is consumed exactly once across the two segments.
/// Items 1 to 5 of the cut: the module, the derives, the ledger's digest
/// state, the pose checks and the clock's re-anchor.
///
/// A resume after the last pulse is the degenerate one: the continuation is
/// only the flush, from a decoded ledger's open chunk.
#[test]
fn af1_a_snapshot_at_every_pulse_resumes_to_the_same_tape() {
    let quiet = |_: i64| Tempo::Quiet;
    let mut reference_arena = scripted_arena();
    let reference = replay(&mut reference_arena, 5_000, 60_000, &script(), &quiet);
    assert_eq!(
        reference.count("\"blocked\""),
        2,
        "the reference is (a)'s run"
    );
    let mut open_chunks = 0_u32;
    for k in 0..=12_i64 {
        let resumed = resume_at_pulse(
            scripted_arena(),
            definition(),
            5_000,
            k,
            60_000,
            &script(),
            &quiet,
        );
        open_chunks += u32::from(resumed.open_chunk);
        same_match(
            &resumed.run,
            &reference,
            &format!("resumed after pulse {k}"),
        );
        assert_eq!(resumed.run.pulses, reference.pulses, "pulse {k}");
        // The ledger's digest state: the two running SHA-256s the result
        // record is signed with, which no line of the tape and no fold of the
        // hash can see.
        assert_eq!(
            resumed.digests,
            digests_of(&reference_arena),
            "resumed after pulse {k}: the ledger's running digests differ"
        );
    }
    // The two-tank tape is under one chunk, so every resume point carried the
    // ledger's open buffer — the state a flush before the encode would have
    // emptied. At least one is the claim; every one is what it is.
    assert!(
        open_chunks >= 1,
        "no resume point carried the ledger's open chunk"
    );
}

/// **16b cut AF.1's mandatory cross-tempo test** (16b §11; doc 16 §3.1 "Rate
/// independence", D-16-2): test (af1)'s construction at 5,000, 1,000 and 500 ms,
/// each cadence resumed after every pulse of its run — 12, 60 and 120 resume
/// points — and every resumed run one event list, one hash sequence, one final
/// state and one notice list with the uninterrupted run at 5,000 ms. The due
/// heap, the staggered passes and the wake lattice are carried integers in
/// arena time, so the cadence decides nothing across a resume either. The two
/// tanks here carry no gun and no sensor, so no round is in flight at any of
/// these resume points: a shot carried across a resume, at two cadences, is
/// `crates/tacsim/tests/shipped_scale.rs`'
/// `af1_the_shipped_centre_encodes_under_the_expectation`'s.
#[test]
fn af1_resumed_at_every_pulse_at_three_cadences_is_one_event_list_and_one_hash_sequence() {
    let quiet = |_: i64| Tempo::Quiet;
    let mut reference_arena = scripted_arena();
    let reference = replay(&mut reference_arena, 5_000, 60_000, &script(), &quiet);
    for cadence in [5_000_i64, 1_000, 500] {
        let pulses = 60_000 / cadence;
        let mut open_chunks = 0_u32;
        for k in 0..pulses {
            let resumed = resume_at_pulse(
                scripted_arena(),
                definition(),
                cadence,
                k,
                60_000,
                &script(),
                &quiet,
            );
            let what = format!("{cadence} ms, resumed after pulse {k}");
            open_chunks += u32::from(resumed.open_chunk);
            let run = &resumed.run;
            assert_eq!(
                run.events,
                reference.events,
                "{what}: {:?} against {:?}",
                run.kinds(),
                reference.kinds()
            );
            assert_eq!(run.hashes, reference.hashes, "{what}");
            assert_eq!(run.finals, reference.finals, "{what}");
            assert_eq!(run.told, reference.told, "{what}");
            assert_eq!(run.substeps, reference.substeps, "{what}");
            // The cadence really differed: the pulse count is the proof.
            if cadence != 5_000 {
                assert_ne!(run.pulses, reference.pulses, "{what} ran the same pulses");
            }
        }
        assert!(
            open_chunks >= 1,
            "at {cadence} ms no resume point carried the ledger's open chunk"
        );
    }
}

/// **16b cut AF.1 under the task's own wake rule** (16b cut U.1's driver): at
/// the `Quiet` period of 5,000, 1,000 and 500 ms the task is stopped at a
/// lattice pulse (30,000 ms), snapshotted, decoded and resumed; `resume_at`'s
/// answer is the wake the uninterrupted task took next, and the continued run
/// — from the cursor, from that wall — is the uninterrupted one's walls and the
/// fixed-cadence replay's events, hashes and finals.
#[test]
fn af1_the_task_s_own_wakes_resume_on_the_lattice() {
    const STOP_MS: i64 = 30_000;
    let quiet = |_: i64| Tempo::Quiet;
    let mut fixed_arena = scripted_arena();
    let fixed = replay(&mut fixed_arena, 5_000, 60_000, &script(), &quiet);
    let mut open_chunks = 0_u32;
    for period in [5_000_u32, 1_000, 500] {
        let mut whole_arena = scripted_arena();
        let (whole, whole_woke, whole_walls) =
            replay_as_the_task_floored(&mut whole_arena, period, 60_000, &script(), None);

        let mut arena = scripted_arena();
        let (first, first_woke, first_walls, cursor) =
            replay_as_the_task_from(&mut arena, period, STOP_MS, &script(), 0, None, 0);
        assert_eq!(arena.now_ms, STOP_MS, "the task stopped on the lattice");
        let Some(&last_wall) = first_walls.last() else {
            panic!("the first segment stepped");
        };
        let snapshot = Snapshot::encode(&arena).expect("the task's arena encodes");
        open_chunks += u32::from(carries_open_chunk(&snapshot));
        let mut resumed = Snapshot::decode(&snapshot.document, definition()).expect("and decodes");
        let first_due = resumed.resume_at(last_wall);
        let at = whole_walls
            .iter()
            .position(|wall| *wall == last_wall)
            .expect("the uninterrupted task woke there too");
        assert_eq!(
            Some(&first_due),
            whole_walls.get(at + 1),
            "at a {period} ms period the resume wakes where the uninterrupted task did"
        );
        let (mut second, second_woke, second_walls, reached) = replay_as_the_task_from(
            &mut resumed,
            period,
            60_000,
            &script(),
            cursor,
            None,
            first_due,
        );
        finish_replay(&mut resumed, &mut second);
        assert_eq!(reached, script().len(), "the script, exactly once");
        let walls: Vec<i64> = first_walls.iter().chain(&second_walls).copied().collect();
        assert_eq!(walls, whole_walls, "at a {period} ms period");
        let woke: Vec<i64> = first_woke.iter().chain(&second_woke).copied().collect();
        assert_eq!(woke, whole_woke, "at a {period} ms period");
        let run = joined(&first, &second);
        same_match(&run, &whole, &format!("the task at a {period} ms period"));
        assert_eq!(
            digests_of(&resumed),
            digests_of(&whole_arena),
            "at a {period} ms period: the ledger's running digests differ"
        );
        assert_eq!(run.events, fixed.events, "at a {period} ms period");
        assert_eq!(run.hashes, fixed.hashes, "at a {period} ms period");
        assert_eq!(run.finals, fixed.finals, "at a {period} ms period");
    }
    assert!(
        open_chunks >= 1,
        "no stop on the lattice carried the ledger's open chunk"
    );
}

/// **16b cut AF.1 across a drop and a reclaim** (16b cut Y.3/Y.4's script): the
/// seat is released at 40 s and taken back at 55 s, and a snapshot taken
/// between the two — at 40, 45 and 50 s — carries the `at_release` row, so the
/// resumed run's catch-up report is the uninterrupted one's: one report, on the
/// reclaim's own frame, counted from the `Linkdead`. The machine holds the tank
/// in `[40 s, 55 s)` on both, and the tape is one tape.
#[test]
fn af1_a_drop_before_the_snapshot_and_a_reclaim_after_it_is_one_catch_up() {
    let quiet = |_: i64| Tempo::Quiet;
    let mut reference_arena = scripted_arena();
    let reference = replay(
        &mut reference_arena,
        5_000,
        Y3_UNTIL_MS,
        &y3_reclaim_script(),
        &quiet,
    );
    assert_eq!(
        reference
            .catchups
            .iter()
            .map(|(at, _)| *at)
            .collect::<Vec<i64>>(),
        vec![Y3_LIVE_MS],
        "the reference is Y.3's run"
    );
    let machine_between = |run: &Replay| {
        run.pulses_seen
            .iter()
            .all(|(started, _, machine)| *machine == (Y3_RELEASED_MS..Y3_LIVE_MS).contains(started))
    };
    assert!(machine_between(&reference));
    let mut open_chunks = 0_u32;
    for k in [8_i64, 9, 10] {
        let resumed = resume_at_pulse(
            scripted_arena(),
            definition(),
            5_000,
            k,
            Y3_UNTIL_MS,
            &y3_reclaim_script(),
            &quiet,
        );
        let what = format!("resumed after pulse {k} ({} ms)", k * 5_000);
        let rows = resumed
            .state
            .get("at_release")
            .and_then(serde_json::Value::as_object)
            .map_or(0, serde_json::Map::len);
        assert_eq!(rows, 1, "{what}: the release row rode in the snapshot");
        assert_eq!(resumed.run.catchups, reference.catchups, "{what}");
        assert!(
            machine_between(&resumed.run),
            "{what}: {:?}",
            resumed.run.pulses_seen
        );
        same_match(&resumed.run, &reference, &what);
        assert_eq!(
            resumed.digests,
            digests_of(&reference_arena),
            "{what}: the ledger's running digests differ"
        );
        open_chunks += u32::from(resumed.open_chunk);
    }
    assert!(
        open_chunks >= 1,
        "no resume point between the drop and the reclaim carried the ledger's open chunk"
    );
}

/// [`definition`] with one cell's price moved — the ground a snapshot taken on
/// [`definition`] must refuse.
fn definition_repriced() -> ArenaDefinition {
    let mut cells = vec![ground(); usize::from(SIDE) * usize::from(SIDE)];
    for y in 0..SIDE {
        if let Some(cell) = cells.get_mut(usize::from(y) * usize::from(SIDE) + usize::from(BAND_X))
        {
            cell.move_cost = 0;
        }
    }
    if let Some(cell) = cells.get_mut(0) {
        cell.move_cost = 150;
    }
    ArenaDefinition {
        terrain: ArenaTerrain::new(SIDE, SIDE, CELL_CM, cells).expect("the test window"),
        ..definition()
    }
}

/// **Every refusal of the decode, by name** (16b cut AF.1): one document from
/// the (a) script at 20 s, and one edit per refusal — the wrong ground, the
/// wrong locomotion table, a field moved past the digest (the hash refuses it),
/// the same field moved without a new digest (the digest refuses it), another
/// format, another record, a pose off the window, two units in one cell, an
/// unknown key and a `state_hash` that is not a number. And one edit that is
/// **not** refused, pinning the brief's decision: a unit on a cell its row
/// cannot enter, with the hash of the state it is. No lane of a running arena
/// puts one there (motion does not, and since 16b cut AE.1 a staff
/// `@arena/place` refuses it as `impassable`); `CellImpassable` is
/// `Arena::new`'s rule for the world's hand-over, and the decode leaves the
/// arena's own poses to the hash (`Snapshot::decode` says why).
#[test]
fn af1_a_snapshot_refuses_the_wrong_ground_the_wrong_physics_a_moved_field_a_moved_byte_and_another_format()
 {
    let quiet = |_: i64| Tempo::Quiet;
    let mut arena = scripted_arena();
    let _ = replay_on_from(
        &mut arena,
        &mut (0..=4).map(|pulse| pulse * 5_000),
        20_000,
        &script(),
        0,
        &quiet,
    );
    let snapshot = Snapshot::encode(&arena).expect("the arena encodes");
    let document = snapshot.document.clone();
    assert!(
        Snapshot::decode(&document, definition()).is_ok(),
        "as taken, it decodes"
    );
    let state: serde_json::Value = state_of(&snapshot);
    let decode =
        |document: &serde_json::Value| Snapshot::decode(document, definition()).map(|_| ());
    let edited = |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut moved = state.clone();
        edit(&mut moved);
        let mut document = document.clone();
        restamp(&mut document, &moved.to_string());
        document
    };

    // The wrong ground: one cell's price moved, so the digest is another.
    let repriced = definition_repriced();
    let given = tacsim::arena::snapshot::digest_text(&repriced.digest());
    assert_eq!(
        Snapshot::decode(&document, repriced).map(|_| ()),
        Err(SnapshotError::Definition {
            recorded: snapshot.definition_digest.clone(),
            given,
        })
    );
    // The wrong physics.
    let mut other_table = document.clone();
    other_table["physics_digest"] = serde_json::Value::String("0".repeat(64));
    assert_eq!(
        decode(&other_table),
        Err(SnapshotError::Physics {
            recorded: "0".repeat(64),
            loaded: mush_atlas::physics::LOCOMOTION_DIGEST.to_owned(),
        })
    );
    // A moved field with its digest stamped again: the hash refuses it, and says
    // both hashes — the one recorded and the moved arena's.
    let later = edited(&|state| state["now_ms"] = serde_json::json!(20_001));
    let mut twin = arena.clone();
    twin.now_ms = 20_001;
    assert_eq!(
        decode(&later),
        Err(SnapshotError::HashMismatch {
            recorded: snapshot.state_hash,
            decoded: tacsim::arena::hash::state_hash(&twin),
        })
    );
    // The same edit without the new digest: the digest refuses it first.
    let mut unstamped = document.clone();
    unstamped["state"] = later["state"].clone();
    assert!(matches!(
        decode(&unstamped),
        Err(SnapshotError::DigestMismatch { recorded, computed })
            if recorded == document["state_digest"] && computed == later["state_digest"]
    ));
    // Another format, another record. (16b cut AI.1: written against the
    // constant, so the next bump breaks neither arm.)
    let mut format = document.clone();
    format["format"] = serde_json::json!(SNAPSHOT_FORMAT + 1);
    assert_eq!(
        decode(&format),
        Err(SnapshotError::Format {
            found: SNAPSHOT_FORMAT + 1,
            expected: SNAPSHOT_FORMAT
        })
    );
    let mut record = document.clone();
    record["record"] = serde_json::json!("x");
    assert_eq!(
        decode(&record),
        Err(SnapshotError::Record {
            found: "x".to_owned()
        })
    );
    // A pose off the window, and two units in one cell: `Arena::new`'s own
    // refusals, before the hash is asked.
    let off = edited(&|state| state["units"][0]["pose"]["x_cm"] = serde_json::json!(-1));
    assert!(matches!(
        decode(&off),
        Err(SnapshotError::Refused(ArenaError::PoseOffWindow {
            x_cm: -1,
            ..
        }))
    ));
    let stacked = edited(&|state| {
        let first = state["units"][0]["pose"].clone();
        state["units"][1]["pose"] = first;
    });
    assert!(matches!(
        decode(&stacked),
        Err(SnapshotError::Refused(ArenaError::CellOccupied { .. }))
    ));
    // **Not refused**: the mover on the impassable band, with the hash of the
    // arena that is — a twin moved the same way.
    let band_x_cm = i64::from(BAND_X) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2;
    let mut on_band =
        edited(&|state| state["units"][0]["pose"]["x_cm"] = serde_json::json!(band_x_cm));
    let mut twin = arena.clone();
    if let Some(mover) = twin.unit_mut(MOVER) {
        mover.pose.x_cm = band_x_cm;
    }
    on_band["state_hash"] =
        serde_json::Value::String(tacsim::arena::hash::state_hash(&twin).to_string());
    let placed = Snapshot::decode(&on_band, definition())
        .expect("a unit on a cell its row cannot enter is left to the hash, which is its own");
    assert_eq!(
        placed.unit(MOVER).map(|unit| unit.pose.x_cm),
        Some(band_x_cm)
    );
    // An unknown key, and a hash that is not a number.
    let unknown = edited(&|state| state["surplus"] = serde_json::json!(1));
    assert!(
        matches!(decode(&unknown), Err(SnapshotError::Malformed(message)) if message.contains("surplus"))
    );
    let mut wordy = document.clone();
    wordy["state_hash"] = serde_json::json!("x");
    assert!(matches!(decode(&wordy), Err(SnapshotError::Malformed(_))));
    // And a head that says something else than its state: a moved key outside
    // the digest is a moved field too.
    let mut late_head = document.clone();
    late_head["at_ms"] = serde_json::json!(25_000);
    assert!(
        matches!(decode(&late_head), Err(SnapshotError::Malformed(message)) if message.contains("25000"))
    );
}

/// Walk a JSON value: no number that is a float. Answers how many numbers it
/// saw. (It checks no key order: a parsed `serde_json::Value`'s object is
/// sorted by construction, so an order check here could not fail.)
fn integer_only(value: &serde_json::Value, path: &str) -> usize {
    match value {
        serde_json::Value::Number(number) => {
            assert!(!number.is_f64(), "{path} is a float: {number}");
            1
        }
        serde_json::Value::Array(items) => items
            .iter()
            .map(|item| integer_only(item, &format!("{path}[]")))
            .sum(),
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(key, item)| integer_only(item, &format!("{path}.{key}")))
            .sum(),
        _ => 0,
    }
}

/// **The encoding is deterministic and integer** (16b cut AF.1): an arena
/// encoded twice is one string; encoded, decoded and encoded again it is still
/// that string — the encode's borrowed state and the decode's owned one agree
/// field for field; and the parsed state holds no float anywhere (the opaque
/// units' rules blobs included). Those two equalities are the whole proof of
/// the string's determinism. **The string is not key-sorted** and nothing here
/// says it is: a struct writes its fields in declaration order and an
/// enum-keyed map in the enum's order (`snapshot.rs`' module head).
///
/// **17b cut M4.5a**: the squadded arena is [`af1_mind_squad_arena`] — its
/// four tanks unit minds where they were the reflex controller's — so the
/// mind rows (memory bytes, buffered notices, cards, seals) are under both
/// equalities and the integer walk; the claims are unchanged.
#[test]
fn af1_the_encoding_is_deterministic_and_integer() {
    // **The `preserve_order` canary**: `serde_json`'s object sorts whatever
    // order it is built in. The one `Value` in the state — an opaque unit's
    // rules blob — writes its keys in this order, so a `preserve_order`
    // unified into the build would make those bytes follow insertion order.
    let mut map = serde_json::Map::new();
    map.insert("b".to_owned(), serde_json::json!(1));
    map.insert("a".to_owned(), serde_json::json!(2));
    assert_eq!(map.keys().collect::<Vec<_>>(), vec!["a", "b"]);

    let quiet = |_: i64| Tempo::Quiet;
    let mut scripted = scripted_arena();
    let _ = replay_on_from(
        &mut scripted,
        &mut (0..=5).map(|pulse| pulse * 5_000),
        25_000,
        &script(),
        0,
        &quiet,
    );
    // 17b cut M4.5a: the squad flown by minds (its rows, memory bytes and
    // pending answers are then under both equalities).
    let mut squadded = af1_mind_squad_arena();
    let _ = replay_on_from(
        &mut squadded,
        &mut (0..=5).map(|pulse| pulse * 5_000),
        25_000,
        &af1_squad_script(),
        0,
        &quiet,
    );
    for (name, arena, ground) in [
        ("the scripted arena", &scripted, definition()),
        ("the squadded arena", &squadded, definition()),
    ] {
        let once = Snapshot::encode(arena).expect("it encodes");
        let twice = Snapshot::encode(arena).expect("it encodes again");
        assert_eq!(
            once.bytes(),
            twice.bytes(),
            "{name}: two encodes, two strings"
        );
        let decoded = Snapshot::decode(&once.document, ground).expect("it decodes");
        let again = Snapshot::encode(&decoded).expect("the decoded arena encodes");
        assert_eq!(
            once.bytes(),
            again.bytes(),
            "{name}: encode, decode and encode again moved a byte"
        );
        assert_eq!(once.document, again.document, "{name}: and the head");
        let numbers = integer_only(&state_of(&once), "state");
        assert!(numbers > 100, "{name}: {numbers} numbers is not a state");
    }
}

/// The squadded arena's second pair.
const SQUAD_RED_B: ObjId = ObjId(5);
const SQUAD_BLUE_B: ObjId = ObjId(10);

/// **A squadded arena of four unit minds** (16b cut AF.1; 17b cut M4.5a):
/// four armed, sighted tanks checked out `Controller::Mind` and no pilot —
/// red's pair in the south, blue's ten cells north facing it — so `Arena::new`
/// deals each side into one squad under its lowest id, two squads, lays a mind
/// row and schedules a think for each, and the first pulse fills the squad
/// watch. The seal is the controller's to ignore (`seal_for` writes the fixture
/// terms' engine): the native stock mind. The default profile presents no
/// band, so a mind's slot never fires at a tank here (17b cut M4.3's `rig.rs`
/// note); no user reads a shot.
///
/// **17b cut M4.6c**: the only squadded builder. Its reflex twin
/// (`af1_squad_arena`, the same four tanks on the reflex controller, held for
/// the shape pin alone since 17b cut M4.5a) and the controller-taking
/// `af1_squad_arena_on` were deleted with the reflex controller, and
/// [`af1_the_snapshot_shape_is_pinned`] walks this arena unstepped in their
/// place (the conventions' D9), given one motor row and one landed, issued
/// order on the mover first.
fn af1_mind_squad_arena() -> Arena {
    af1_mind_squad_arena_sealing(&[])
}

/// **[`af1_mind_squad_arena`] with the mover sealed on `profiles`** (17b cut
/// M5.2): each its preset over the default doctrine, the first the one that
/// flies — a chosen set, for the shape pin's walk. With none it is
/// [`af1_mind_squad_arena`] byte for byte (the mover sealed under the
/// fixture terms, the Line card alone).
fn af1_mind_squad_arena_sealing(profiles: &[&str]) -> Arena {
    let machine = |mut unit: CustodyUnit| {
        unit.controller = tacsim::arena::state::Controller::Mind;
        unit
    };
    let facing_south = |mut unit: CustodyUnit| {
        unit.pose.heading_mdeg = 180_000;
        unit
    };
    let chosen = |mut unit: CustodyUnit| {
        if !profiles.is_empty() {
            let reflex = tacsim::config::ReflexConfig::default();
            let set = profiles
                .iter()
                .map(|name| {
                    (
                        SmolStr::new(*name),
                        tacsim::arena::preset(name, &reflex).expect("a preset"),
                    )
                })
                .collect();
            unit.reseal(&tacsim::match_::SealTerms::fixture().with_profiles(set))
                .expect("the chosen set seals");
        }
        unit
    };
    Arena::new(
        CustodyOut {
            definition: definition(),
            units: vec![
                chosen(machine(watcher(MOVER, Team::Red, centre(5, 5)))),
                machine(watcher(SQUAD_RED_B, Team::Red, centre(7, 5))),
                machine(facing_south(watcher(PARKED, Team::Blue, centre(5, 15)))),
                machine(facing_south(watcher(
                    SQUAD_BLUE_B,
                    Team::Blue,
                    centre(7, 15),
                ))),
            ],
            pilots: Vec::new(),
            resolution: tacsim::match_::stock_resolution(),
            seed: 2_350,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &ArenaConfig::default(),
        &MatchConfig::default(),
        &SensorsConfig::default(),
    )
    .expect("the squadded arena")
}

/// **Red's commander orders its squad forward** at 5,000 ms, into the blue
/// pair's sight, with a fifteen-second time to live — the way the bridge fans a
/// commander's reply out, one order per unit. The four detect, classify, lock
/// and fire on each other from about 20 s; the order runs out at 20,000 ms, the
/// next tick of each red unit notes it, and the squad sits in `squads_expired`
/// until the 30,000 ms floor lets the arena report it — so the snapshot after
/// the pulse at 25,000 ms carries both tables. **17b cut M4.5a**: on
/// [`af1_mind_squad_arena`]'s minds the order runs out at the first landing at
/// or after its 20,000 ms deadline (`expire_mind_order`: the `order_expired`
/// line and the squad's note), and the four do not fire (no presenting
/// profile); nothing here reads a shot.
fn af1_squad_script() -> Vec<Beat> {
    let order = tacsim::deliberation::Order {
        squad: tacsim::deliberation::SquadId {
            team: Team::Red,
            index: 1,
        },
        objective: tacsim::deliberation::order::Objective::Advance { to: (6, 12) },
        target: None,
        horizon_ms: 60_000,
        constraints: tacsim::deliberation::order::Constraints::default(),
        ttl_ms: 15_000,
        issued_ms: 1,
        issuer: None,
        // 17b cut L.1: red-1's lance-leader.
        by: tacsim::deliberation::CharacterRef::Persona {
            squad: tacsim::deliberation::SquadId {
                team: Team::Red,
                index: 1,
            },
        },
        spot: None,
    };
    vec![Beat::at(
        5_000,
        Vec::new(),
        [MOVER, SQUAD_RED_B]
            .into_iter()
            .map(|unit| Intent {
                unit,
                channel: Channel::Helm,
                epoch: 0,
                kind: IntentKind::Order(Box::new(order.clone())),
                seq: 1,
            })
            .collect(),
    )]
}

/// **16b cut AF.1 on a squadded arena** (16c N16; the plan's second review,
/// F1/AF-X1): every match with a reflex squad fills `Arena::squad_watch` at
/// its first pulse, and its key is `SquadId`, a struct `serde_json` refuses as
/// an object key at run time — so the table is written as pairs, and this is
/// the test that would have failed on an AI-filled match's first snapshot. The
/// squadded arena at 5,000 ms, snapshotted after every pulse: every resume is
/// the uninterrupted run — its squad signals included, the one output the two
/// carried views steer, with the expired order among them — and its ledger's
/// running digests, and at least one resume point carries both the watch
/// and a squad whose order ran out unreported, the watch as `[squad, watch]`
/// pairs naming the squad's team and index.
///
/// **17b cut M4.5a**: the squad is flown by unit minds
/// ([`af1_mind_squad_arena`]), as every AI-filled squad has been since 17b
/// cut M4.3, where it was the reflex controller; every claim is kept. A
/// mind's order runs out at the first landing at or after its deadline and
/// journals `order_expired` and the squad's note there (`expire_mind_order`),
/// so the two `ordered` and two `order_expired` lines, the reported expiry
/// and the carried tables are reached as written; every pulse is settled
/// ([`replay_on_from`]), so no resume point holds a think awaiting its answer.
#[test]
fn af1_a_squadded_arena_snapshots_after_its_first_pulse() {
    let quiet = |_: i64| Tempo::Quiet;
    let mut reference_arena = af1_mind_squad_arena();
    assert_eq!(reference_arena.squads.len(), 2, "one squad a side");
    let reference = replay(
        &mut reference_arena,
        5_000,
        60_000,
        &af1_squad_script(),
        &quiet,
    );
    assert!(
        reference.count("\"ordered\"") >= 2 && reference.count("\"order_expired\"") >= 2,
        "the order landed and ran out: {:?}",
        reference.kinds()
    );
    // The squad signals are what the carried watch and expired set steer, and
    // `same_match` holds every resume to this list: it must hold the expiry the
    // carried set reports.
    assert!(
        reference
            .deliberation
            .iter()
            .any(|signal| signal.reason.as_str() == "order_expired"),
        "the reference reported the expired order: {:?}",
        reference
            .deliberation
            .iter()
            .map(|signal| (signal.at_ms, signal.squad, signal.reason.as_str()))
            .collect::<Vec<_>>()
    );
    let mut both = 0_u32;
    let mut open_chunks = 0_u32;
    for k in 0..=12_i64 {
        let resumed = resume_at_pulse(
            af1_mind_squad_arena(),
            definition(),
            5_000,
            k,
            60_000,
            &af1_squad_script(),
            &quiet,
        );
        let what = format!("the squadded arena resumed after pulse {k}");
        same_match(&resumed.run, &reference, &what);
        assert_eq!(
            resumed.digests,
            digests_of(&reference_arena),
            "{what}: the ledger's running digests differ"
        );
        open_chunks += u32::from(resumed.open_chunk);
        let watch = resumed
            .state
            .get("squad_watch")
            .and_then(serde_json::Value::as_array)
            .expect("the squad watch is an array of pairs");
        assert!(!watch.is_empty(), "{what}: the first pulse fills the watch");
        for pair in watch {
            let squad = pair.get(0).expect("a pair's squad");
            assert!(
                squad.get("team").is_some_and(serde_json::Value::is_string)
                    && squad.get("index").is_some_and(serde_json::Value::is_u64),
                "{what}: {pair}"
            );
        }
        let expired = resumed
            .state
            .get("squads_expired")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len);
        both += u32::from(expired > 0);
    }
    assert!(
        both >= 1,
        "no resume point carried a squad whose order ran out unreported"
    );
    assert!(
        open_chunks >= 1,
        "no resume point of the squadded arena carried the ledger's open chunk"
    );
}

/// One line per key path of a JSON value, `path: kind`, into `out`; an array is
/// descended once, as one `[]` segment for all of its elements — so a pair's key
/// and value, and a list whose elements differ in shape, each leave their own
/// lines, and a list of a hundred alike leaves one set.
fn shape(value: &serde_json::Value, path: &str, out: &mut std::collections::BTreeSet<String>) {
    let kind = match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    };
    out.insert(format!("{path}: {kind}"));
    match value {
        serde_json::Value::Array(items) => {
            let at = format!("{path}.[]");
            for item in items {
                shape(item, &at, out);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, item) in map {
                shape(item, &format!("{path}.{key}"), out);
            }
        }
        _ => {}
    }
}

/// **The structural pin**: SHA-256 of the sorted `path: kind` lines of an
/// unstepped [`scripted_arena`]'s encoded state and an unstepped
/// [`af1_mind_squad_arena`]'s (the reflex squad arena's, `af1_squad_arena`,
/// until 17b cut M4.6c), one set, joined by newlines.
///
/// **Re-blessed by 16b cut AI.1 with `SNAPSHOT_FORMAT` 2**: `ReflexUnit` gained
/// `searched_from` and `refused`, one `null` line each on the squad arena's four
/// reflex rows — 378 lines to 386.
///
/// **Re-blessed by 17b cut M1a.1 with `SNAPSHOT_FORMAT` 3**: the nine driving
/// fields left the reflex rows for `Arena::motors` — nine lines off each of the
/// squad arena's four reflex rows and `state.reflex.window` and
/// `state.reflex.spent` gone (38), and `state.motors`, `state.motors.rows`, one
/// `state.motors.rows.<id>` object with its nine field lines for each of the
/// four machines `Arena::new` adopts, `state.motors.window` and
/// `state.motors.spent` come (44) — 386 lines to 392.
///
/// **Carried unchanged through 17b cut M1a.4's `SNAPSHOT_FORMAT` 4**:
/// `DueKind::MotorTick` and `IntentKind::Goal` are new variants of two carried
/// enums, and neither fixture schedules a motor tick or queues an intent at
/// pulse zero, so the digest and the 392 lines are what they were and only the
/// format moved (the variants' own round trip is
/// `m1a4_a_snapshot_carries_a_motor_tick_and_a_pilots_motor_row`).
///
/// **Re-blessed by 17b cut M1a.5 with `SNAPSHOT_FORMAT` 5**: the carried
/// `minds` section, empty on both unstepped fixtures (neither fields a mind),
/// is the one new line `state.minds: object` — 392 lines to 393. A mind row's
/// own fields, `Controller::Mind` and the two mind due kinds were under the pin
/// on no fixture here until 17b cut M4.6c (the variants' round trip was
/// `m1a5_a_snapshot_carries_the_minds_and_not_their_landings`, 17b cut M3.2's
/// [`m3_a_snapshot_carries_the_minds_with_their_pending_answers`] since).
///
/// **Re-blessed by 17b cut M1b.1 with `SNAPSHOT_FORMAT` 7**: three new
/// carried leaves each unstepped fixture fills — `state.conditions: object`
/// (the transparent slot table, empty), `state.held_said: array` (the
/// held-shot rows through `pairs`, empty) and `state.odds_floor_permille:
/// number` (the floor captured at check-out) — 393 lines to 396. The card
/// inside a mind row, `IntentKind::Fire.slot`, `IntentKind::Condition`, the two
/// slot notices and `RefusalDetail::Floor` were under the pin on no fixture
/// here (the card inside a mind row is, since 17b cut M4.6c). Their round trips: the card, the two slot notices (every `HeldWhy`)
/// and a refusal carrying `RefusalDetail::Floor` in a mind's notice buffer in
/// `tests/conditions.rs`'
/// `m1b1_a_snapshot_carries_slots_a_card_the_floor_and_held_said_and_resumes_the_hash_sequence`;
/// a cited `Fire`, a disarm and an arming in `Arena.pending` (empty between
/// pulses, so no stepped snapshot carries one) in `snapshot.rs`'
/// `af1_every_keyed_map_in_the_tree_is_one_json_writes`.
///
/// **Re-blessed by 17b cut M3.2 with `SNAPSHOT_FORMAT` 8** (D-UM-16, D-UM-23
/// as amended, D-UM-45): 46 new lines, the two carried shapes each unstepped
/// fixture fills — `state.classes: object` with its four rows
/// (`state.classes.0`, `.I`, `.II`, `.III`, each an object and its eight
/// figures `think_ms`, `ops`, `rows`, `odds_rows`, `slots`, `memory_keys`,
/// `line_to`, `profiles` as numbers: 37 lines; the class table captured at
/// check-out, carried) and `state.units.[].seal: object` with its seven
/// fields (`sortie` a number, `digest` an array and its `[]` numbers,
/// `core_class` and `class` strings, `engine` a string, `governed` and
/// `closed` bools: 9 lines; laid down empty on every unit) — 396 lines to
/// 442. The fixtures stay coreless (17b cut M3's F2) and field no mind and
/// arm no slot, so the mind row's new shapes (`PendingThink`'s `epochs`,
/// `intents` and `memory`, `MindState.dropped` and `seal_digest`) and the
/// condition row's (a vector as wide as the core's slots where it was a
/// fixed pair) were under the pin on no fixture here until 17b cut M4.6c
/// walked a mind squad: their round trips are
/// [`m3_a_snapshot_carries_the_minds_with_their_pending_answers`],
/// `tests/minds.rs`' `m3_a_resumed_landing_lands_the_carried_answer` and
/// `snapshot.rs`' `af1_every_keyed_map_in_the_tree_is_one_json_writes`.
///
/// **Re-blessed by 17b cut M4.6c with `SNAPSHOT_FORMAT` 9** (the deletion of
/// the reflex controller; the conventions' D9), once, old → new
/// `5710d7c33ad33929aaa45bb7084ece66fe88a3bf397ea272301dd90abe7cfa2d` (442
/// lines) → `202e7d4f3a6392b65225530c1a2ee2aae2d8023224820edd13139315a9df37e7`
/// (509 lines). The pin walks [`af1_mind_squad_arena`] in place of the reflex
/// squad arena (deleted with the reflex): the same four armed, sighted tanks
/// in two squads, flown by minds — given, before it is encoded, one motor row
/// on the mover (a mind's check-out lays none, where the reflex's adoption
/// laid one per machine) and one squad order with an issuer LANDED on the
/// mover through `apply_intent`'s Order arm. 93 lines left — the
/// `state.reflex` section (`state.reflex`, `state.reflex.cfg` and its
/// thirteen figures, `state.reflex.units` and each of the four reflex rows
/// with its ten fields: 60), the `reflex_tick` due kind with its `unit` and
/// `tick_seq` (3), and the motor rows of the three machines but the mover
/// (`state.motors.rows.<id>` and its nine fields each: 30; the mover's ten
/// lines stay, its row the one laid here); 160 arrived — each of the four
/// mind rows (`state.minds.<id>` and its eleven fields, its card's
/// twenty-four terms and its `seal_digest.[]`: 37 each, 148), the mover's
/// standing order in place of its `null` (`state.minds.4.order` an object
/// with its `FrameOrder` fields — `squad`, `objective` with its tag and its
/// `to.[]`, `hold_fire`, `max_range_band`, `expires_ms` — 9 lines for 1:
/// +8), the `mind_think` due kind with its `unit` and `think_seq` (3), and
/// `state.units.[].seal.doctrine: bool` (1). The owner's STOP C issuer is
/// among the mind rows' fields, a number on the mover
/// (`state.minds.4.order_issuer: number`) and `null` on the three rows
/// holding no order. 442 − 93 + 160 = 509. (The cut's first re-bless, before
/// its review restored the motor row and added the order, read
/// `862ed2b97bcfa2b5fc9461787d19c1251742d6ea7b4516a64d15e2c779b98c8d`, 491
/// lines; it was never committed.)
///
/// **Re-blessed by 17b cut M5.2 with `SNAPSHOT_FORMAT` 10** (the one format
/// move of the M5 cuts; the design note's item 8), once in the commit, old →
/// new `202e7d4f3a6392b65225530c1a2ee2aae2d8023224820edd13139315a9df37e7`
/// (509 lines) →
/// `811a1ce79c8ae983ad8cce0c67b41765449341e76f0811c9596f3b14777d5611` (556
/// lines), reached on the uncommitted tree in two steps: stage B's bless,
/// 509 → `ff8ca920d95f120d924ff935577ec7b0dbadbb2c24b813aba064221539b0a59c`
/// (542 lines), then the orchestrator's D21 ruling (2026-10-11), re-pinned
/// `ff8ca920…` (542) → `811a1ce7…` (556) before the commit so that the walk
/// covers every objective's fields (below); `ff8ca920…` was never
/// committed. Stage B's 542: nine are the format's three carried additions
/// as every unit and row writes them at their defaults:
/// `state.units.[].seal.profiles:
/// array` and its `[]: number` (2), `state.units.[].card_profile: object`
/// with `index: number` and `switched_ms: null` (3) and
/// `state.minds.<id>.order_charge: null` on each of the four rows (4) — 509
/// to 518 on the walk as it stood. The walk then reaches what the pin did not
/// (doc 17 §24's index): the mover is sealed with a chosen set, `[Sniper,
/// Line]` ([`af1_mind_squad_arena_sealing`]), and switched to its second
/// profile through a seat's `MIND PROFILE` (`card_profile.switched_ms:
/// number`, +1); its motor row is given a goal, a two-cell route, one avoided
/// cell, `searched_from`, `searched_ms` and a remembered stop (`Refusal`: its
/// `at`, `cell`, `cause`, `goal` and `until_ms`) — four `null` lines replaced
/// and the route's and the avoid list's elements walked (+16); the mover's
/// standing order is a `Support` of red's second tank capped `Effective`
/// (`max_range_band: string`, a `RangeBand`; `objective.of: null`, the charge
/// held as `order_charge: number`), where it was the script's `Advance`,
/// whose `to` and `to.[]` leave the mover (−1); and red's second tank holds
/// that `Advance` (`state.minds.5.order` an object and its `FrameOrder`
/// fields, 9 lines for 1: +8). 518 + 1 + 16 − 1 + 8 = 542. The D21 ruling's
/// fourteen (closing doc 17 §24's index item — "route and avoid elements,
/// `Refusal`, `RangeBand`, the other objectives"): blue's squad is ordered
/// too, PARKED an `Engage` (`state.minds.9.order` an object and its
/// `FrameOrder` fields with `objective.target: number`, 8 lines for 1: +7)
/// and blue's second tank a `Screen` (`state.minds.10.order`, the same with
/// `objective.facing_mdeg: number`: +7), and the mover's `Support` is
/// re-keyed to its own row for its charge (`objective.of: number` for
/// `null`, 0) — the `Support` and the `Engage` re-keyed against classified
/// rows laid for their landings and dropped before the walk. 542 + 7 + 7 =
/// 556.
const AF1_SHAPE_DIGEST: &str = "811a1ce79c8ae983ad8cce0c67b41765449341e76f0811c9596f3b14777d5611";
/// And how many lines that is.
const AF1_SHAPE_LINES: usize = 556;

/// **The snapshot's shape is pinned** (16b cut AF.1).
///
/// *This pin is structural: it moves only together with a `SNAPSHOT_FORMAT` bump
/// and never alone, and a behaviour change never moves it (the tree at pulse
/// zero is a function of the fixture and the shape, not of motion or
/// perception). Two fixtures are walked, so the pin sees every path either of
/// them fills: [`scripted_arena`]'s bare tanks and [`af1_mind_squad_arena`]'s
/// armed, sighted machines in two squads (since 17b cut M5.2 its mover sealed
/// with a chosen set, [`af1_mind_squad_arena_sealing`]), given before the
/// walk a motor row on the mover holding a goal, a route, an avoided cell,
/// `searched_from` and a remembered stop, a landed order on each of its four
/// minds — a `Support` on the mover, an `Advance` on red's second tank, an
/// `Engage` on PARKED and a `Screen` on blue's second tank (17b cut M5.2's
/// D21 ruling), the `Support` and the `Engage` re-keyed against classified
/// rows laid for their landings and dropped before the walk — and a seat's
/// profile switch on the mover:
/// its weapons, sensors, squad rosters, the motor row's carried fields and
/// their elements, mind rows (card, memory, strikes, drop, seal digest, four
/// standing orders' `FrameOrder` fields — every objective's fields, the
/// `Support`'s `of` and the `Engage`'s `target` as numbers, a `RangeBand` —
/// the owner's STOP C `order_issuer` and STOP E's `order_charge` as
/// numbers), their due thinks, each unit's seal reference with its
/// `doctrine` bit and its sealed set, and each unit's active profile with a
/// switch's stamp are under the pin.*
///
/// *Its limit, read off the 556 lines (a collection empty or an option `None`
/// at pulse zero leaves one line and none for its element): **not under it**
/// are the contacts (`ShadowContact`, `SeenPose`, `OwnSighting`, `Reading`;
/// the rows the `Support` and the `Engage` re-key against at their landings
/// are dropped before the walk),
/// the designator book's rows, the shots (`Shot`, `Resolution`), the pending
/// intents (`Intent` — a seat's `IntentKind::Profile`, 17b cut M5.2, is
/// continuous and never queued there), the release rows and catch-ups
/// (`ReleaseSnapshot`, `Catchup`, `SectionWords`, `MachineWords`), the held
/// losses, `spent_said`, the squad watch (`SquadWatch`, `SquadDigest`) and the
/// expired set, `stop`, `ended` (`MatchEnd`), `hold` (`HoldState`), the
/// ledger's closed queue, the clock's two signal times; on a unit `heat`
/// (`HeatState`), `held` (`Held`), `lock`, `occupied_hold` (`OccupiedHold`),
/// `thermal`, `jammer`, `eccm`, `receiver`, `slope_limit_pct`, the profile's
/// bands and sections (`BandProfile`, `SectionProfile`) and the opaque unit's
/// capabilities, components and mobility; on the motor row the waypoints
/// (`waypoint`, `from_waypoint`) and `hold_retry_ms`, `None` on the one row
/// laid here; and on a mind row (**17b cut M1a.5**) the think in flight
/// (`PendingThink`), its buffered `FrameNotice`s (and so a carried
/// `Refusal`, `profile` among its words since 17b cut M5.2) and its range
/// memory's entries, all empty or `None` at pulse zero. Every objective of a
/// standing order IS under it since 17b cut M5.2's D21 ruling — `Advance`'s
/// `to`, `Engage`'s `target` and `Support`'s `of` as numbers, `Screen`'s
/// `facing_mdeg`, and `Hold` and `Withdraw`, which carry nothing but the tag
/// every walked objective writes. A field added to or dropped from one of
/// the shapes NOT under it moves nothing here: the format is then the author's to bump and the reviewer's to see, and a
/// stale document is refused as `Malformed` — a missing field by the derive, a
/// surplus one by `deny_unknown_fields` on the carried types — rather than by
/// name as `Format`, and never silently resumed. The pin also moves
/// without a shape change when a default map's keys change (a key is a path
/// segment: `arena_cfg.max_units_by_class.drone`, `minds.4`) or when
/// `Arena::new` schedules a different set of due kinds at open. A cut that
/// adds, renames or re-types a carried field re-blesses it and bumps the format
/// in the same change, and a running match on a live box is voided at that
/// reload (deploy/README.md).*
///
/// **Carried unchanged through 17b cut L.1's `SNAPSHOT_FORMAT` 6**: `Order.by`
/// is a field of the standing order, which neither fixture holds at pulse zero,
/// so the digest and the 393 lines are what they were and only the format moved
/// (the round trip is `an_order_at_three_cadences_is_one_event_list_and_one_hash_sequence`
/// in `tests/deliberation.rs`, and [`l1_a_snapshot_carries_the_orders_author_and_refuses_a_format_five_document`]).
#[test]
fn af1_the_snapshot_shape_is_pinned() {
    use sha2::Digest as _;
    use tacsim::arena::intents::{IntentOutcome, IntentSource, apply_intent};
    use tacsim::arena::perception::MindObjective;
    use tacsim::arena::rng::draw_contact_id;
    use tacsim::arena::state::{Classification, SeenPose, ShadowContact};
    use tacsim::deliberation::order::{Objective, RangeBand};
    // 17b cut M5.2: the mover sealed with a chosen set, Sniper then Line, so
    // `SealRef::profiles` is walked as a set of two codes.
    let mut squadded = af1_mind_squad_arena_sealing(&["Sniper", "Line"]);
    // 17b cut M4.6c: a mind's check-out lays no motor row, so the walked
    // arena is given one here — through the table's own door, as the motor
    // fixtures lay one (`MotorTable::ensure`; a goal's landing, the lawful
    // way in a match, refuses a cold hull) — and every carried field of a
    // `MotorRow` stays under the pin. 17b cut M5.2: and the row holds what
    // its fields can hold — a goal, a route of two cells, one avoided cell,
    // the cell its last search started from and when, and a remembered stop
    // (`Refusal`) — so their elements are under the pin too.
    {
        let row = squadded.motors.ensure(MOVER);
        row.goal = Some((5, 7));
        row.route = vec![(5, 6), (5, 7)];
        row.searched_ms = Some(0);
        row.searched_from = Some((5, 5));
        row.avoid = vec![((6, 6), 0)];
        row.refused = Some(Refusal {
            at: (5, 5),
            cell: (6, 6),
            cause: BlockedCause::Impassable,
            goal: (5, 7),
            until_ms: 30_000,
        });
    }
    // 17b cut M5.2 (the orchestrator's D21 ruling): the rows the landings
    // below re-key against — `Arena::rekey_order` keeps a row only when the
    // hull holds it CLASSIFIED — so `Support`'s `of` and `Engage`'s `target`
    // land as numbers: the mover holds red's second tank, and blue's
    // commander (in whose scope an order's hint is written) and PARKED hold
    // the mover. Each is dropped again once the orders stand (below), so the
    // contacts stay out of the walk as before, and the rows read as hulls
    // that have since lost sight of their charge or their target while their
    // orders stand — the state a supporter blind for a TTL is in (STOP E).
    let blue_squad = squadded
        .squad_of(PARKED)
        .map(|squad| (squad.id, squad.commander))
        .expect("blue's squad");
    let held = [
        (MOVER, SQUAD_RED_B, Team::Red),
        (blue_squad.1, MOVER, Team::Red),
        (PARKED, MOVER, Team::Red),
    ];
    for (viewer, target, team) in held {
        let plot = squadded.unit(target).map(SeenPose::of).expect("a tank");
        squadded.contacts.insert(
            viewer,
            draw_contact_id(2_350, viewer, target),
            ShadowContact::sighted(target, Classification::Classified { team }, 0, plot),
        );
    }
    // The squad script's order, as the world lands one — its own helm epoch,
    // through the Order arm, which writes the issuer and (17b cut M5.2) the
    // charge beside it — so the mind rows' `FrameOrder`, `order_issuer` as a
    // number and `order_charge` as a number are under the pin: red's second
    // tank holds the script's `Advance` (no issuer), and the mover a
    // `Support` of it capped at `Effective` (`max_range_band`, a `RangeBand`)
    // with an issuer set (`ObjId(77)`, no tank of this fixture — a record
    // field nothing here reads back), its charge re-keyed to the mover's own
    // row for it (`of` a number) and the unit riding `order_charge`. And
    // (the D21 ruling) blue's squad is ordered too, so every objective's
    // fields are walked: PARKED holds an `Engage` hinted at the mover in its
    // commander's scope (`target` a number, PARKED's own row) and blue's
    // second tank a `Screen` facing south (`facing_mdeg`).
    let order = af1_squad_script()
        .into_iter()
        .flat_map(|beat| beat.intents)
        .find_map(|intent| match intent.kind {
            IntentKind::Order(order) if intent.unit == MOVER => Some(*order),
            _ => None,
        })
        .expect("the squad script orders the mover");
    let support = tacsim::deliberation::Order {
        objective: Objective::Support { of: SQUAD_RED_B },
        issuer: Some(ObjId(77)),
        constraints: tacsim::deliberation::order::Constraints {
            max_range_band: Some(RangeBand::Effective),
            ..order.constraints
        },
        ..order.clone()
    };
    let (blue, blue_commander) = blue_squad;
    let blue_order = |objective: Objective| tacsim::deliberation::Order {
        squad: blue,
        objective,
        by: tacsim::deliberation::CharacterRef::Persona { squad: blue },
        ..order.clone()
    };
    let engage = blue_order(Objective::Engage {
        target_hint: Some(draw_contact_id(2_350, blue_commander, MOVER)),
    });
    let screen = blue_order(Objective::Screen {
        facing_mdeg: 180_000,
    });
    for (unit, kind) in [
        (SQUAD_RED_B, IntentKind::Order(Box::new(order))),
        (MOVER, IntentKind::Order(Box::new(support))),
        (PARKED, IntentKind::Order(Box::new(engage))),
        (SQUAD_BLUE_B, IntentKind::Order(Box::new(screen))),
        // 17b cut M5.2: and a seat's `MIND PROFILE` to the mover's second
        // profile, so `card_profile` is walked off its default (an index and
        // a stamp as numbers).
        (MOVER, IntentKind::Profile { index: 1 }),
    ] {
        let channel = kind.channel_required();
        let issued = Intent {
            unit,
            channel,
            epoch: squadded.epochs.current(unit, channel),
            kind,
            seq: 1,
        };
        assert_eq!(
            apply_intent(&mut squadded, &issued, IntentSource::Seat),
            IntentOutcome::AppliedContinuous,
            "it lands on #{}",
            unit.0
        );
    }
    let mover = squadded.minds.get(&MOVER).expect("the mover's row");
    assert_eq!(
        (mover.order_issuer, mover.order_charge),
        (Some(ObjId(77)), Some(SQUAD_RED_B)),
        "the landing wrote the issuer and the charge"
    );
    assert!(
        mover
            .order
            .as_ref()
            .is_some_and(|order| order.max_range_band.is_some()),
        "the band rides the held order"
    );
    let objectives: Vec<Option<MindObjective>> = [MOVER, PARKED, SQUAD_BLUE_B]
        .iter()
        .map(|unit| {
            squadded
                .minds
                .get(unit)
                .and_then(|row| row.order.as_ref())
                .map(|order| order.objective)
        })
        .collect();
    assert_eq!(
        objectives,
        vec![
            Some(MindObjective::Support {
                of: Some(draw_contact_id(2_350, MOVER, SQUAD_RED_B)),
            }),
            Some(MindObjective::Engage {
                target: Some(draw_contact_id(2_350, PARKED, MOVER)),
            }),
            Some(MindObjective::Screen {
                facing_mdeg: 180_000,
            }),
        ],
        "every objective's fields, the re-keyed rows as numbers"
    );
    // The rows the landings re-keyed against, dropped: no contact rides the
    // walk (the pin's limit, below).
    for (viewer, target, _) in held {
        squadded
            .contacts
            .remove(viewer, draw_contact_id(2_350, viewer, target));
    }
    assert_eq!(squadded.contacts.all().count(), 0, "no contact is walked");
    assert_eq!(
        squadded.unit(MOVER).map(|unit| unit.card_profile),
        Some(tacsim::arena::card::ActiveProfile {
            index: 1,
            switched_ms: Some(0),
        })
    );
    assert_eq!(
        squadded
            .unit(MOVER)
            .map(|unit| unit.seal.profiles.codes().to_vec()),
        Some(vec![2, 0])
    );
    let mut lines = std::collections::BTreeSet::new();
    for arena in [scripted_arena(), squadded] {
        let snapshot = Snapshot::encode(&arena).expect("an unstepped arena encodes");
        shape(&state_of(&snapshot), "state", &mut lines);
    }
    let joined = lines.iter().cloned().collect::<Vec<String>>().join("\n");
    let digest =
        tacsim::arena::snapshot::digest_text(&sha2::Sha256::digest(joined.as_bytes()).into());
    assert_eq!(
        tacsim::arena::snapshot::SNAPSHOT_FORMAT,
        11,
        "a format bump re-blesses the pin below in the same change"
    );
    assert_eq!(
        (digest.as_str(), lines.len()),
        (AF1_SHAPE_DIGEST, AF1_SHAPE_LINES),
        "the snapshot's shape moved: bump SNAPSHOT_FORMAT and re-bless.\n{joined}"
    );
}

/// **17b cut M1a.1: a snapshot carries the motor table, and a format-2
/// document is refused by name** (doc 17 §4, D-UM-3; `snapshot.rs`'
/// `SNAPSHOT_FORMAT` 3).
///
/// The squad arena's four machines are stepped through the squad script; they
/// hold their ground on that window, so the mover's motor row is then written
/// by hand, all nine fields of it — a remembered stop (`Refusal`) included —
/// so the round trip carries something. The document's state carries
/// `motors.rows` keyed by those four units and the reflex rows carry none of
/// the nine driving fields; the decode hands back the same table (its scratch,
/// which is not state, aside), the written row field for field, and the same
/// hash. The same document stamped `"format": 2` — what every snapshot
/// written before this cut says — is refused as `Format { found: 2, expected:
/// 3 }` by both the head read and the decode, before a field of its state is
/// read: 16b cut AF.3's boot reads the head first (`match_::resume`'s `check`)
/// and voids a match whose snapshot it refuses by name, through the same
/// `ResumeRefusal::Snapshot` arm `crates/mushd/tests/arena_custody.rs`'
/// `a_restart_mid_match_voids_a_match_whose_snapshot_is_wrong` pins for a
/// refused decode.
///
/// **17b cut M4.5a**: the squad's four tanks are unit minds
/// ([`af1_mind_squad_arena`]), where they were the reflex controller's. A
/// mind's motor row is laid at its first goal's landing, not at check-out, so
/// the mover's row is the one ITS goal laid by 20,000 ms (asserted to exist
/// before it is written by hand); and the table's keys, which were the four
/// machines the reflex
/// adopted at check-out, are re-derived to the rows the minds' goals laid by
/// 20,000 ms — one per machine a goal drove: red's pair, which the Advance
/// sent for (6, 12), held to the minds whose memory holds a goal (the blue
/// pair patrols with nothing held and nowhere to go, and lays none). The
/// reflex rows the original held free of the
/// nine driving fields are none on minds, so that claim is made of what
/// stands in their place: each of the four MIND rows `state.minds` carries
/// holds none of the nine — a mind steers only through its goal, which the
/// motor table carries.
#[test]
fn m1a1_a_snapshot_carries_the_motors_and_refuses_a_format_two_document() {
    let quiet = |_: i64| Tempo::Quiet;
    let mut arena = af1_mind_squad_arena();
    let _ = replay_on_from(
        &mut arena,
        &mut (0..=4).map(|pulse| pulse * 5_000),
        20_000,
        &af1_squad_script(),
        0,
        &quiet,
    );
    // A row driving somewhere, all nine of its fields written (the remembered
    // stop too), so the round trip below is a round trip of something: the
    // drive is written by hand, exactly as the reflex controller's own fields
    // are by its unit tests.
    let motor = arena
        .motors
        .row_mut(MOVER)
        .expect("the mover's motor row, laid by its own goal's landing");
    motor.goal = Some((9, 9));
    motor.route = vec![(6, 6), (7, 7)];
    motor.searched_ms = Some(19_000);
    motor.searched_from = Some((5, 5));
    motor.avoid = vec![((6, 5), 18_000)];
    motor.waypoint = Some(1);
    motor.from_waypoint = Some(0);
    motor.hold_retry_ms = Some(40_000);
    motor.refused = Some(Refusal {
        at: (5, 5),
        cell: (6, 6),
        cause: BlockedCause::Impassable,
        goal: (9, 9),
        until_ms: 48_000,
    });

    let snapshot = Snapshot::encode(&arena).expect("the arena encodes");
    let state = state_of(&snapshot);
    let rows = state
        .get("motors")
        .and_then(|motors| motors.get("rows"))
        .and_then(serde_json::Value::as_object)
        .expect("the state carries the motor rows");
    let keys: Vec<&str> = rows.keys().map(String::as_str).collect();
    // 17b cut M4.5a: one row per machine that a goal has put on the motor —
    // red's pair, which the Advance sent for (6, 12); the blue pair patrols
    // with nothing held and nowhere to go on this window and lays none.
    let mut goals: Vec<String> = arena
        .minds
        .iter()
        .filter(|(_, row)| {
            tacsim::arena::stock_mind::StockMemory::from(&tacsim::arena::mind::decode_memory(
                &row.memory,
            ))
            .goal
            .is_some()
        })
        .map(|(unit, _)| unit.0.to_string())
        .collect();
    goals.sort();
    assert_eq!(goals, ["4", "5"], "the minds that hold a goal");
    assert_eq!(keys, goals, "one row per machine a goal drove, by unit");
    let mind_rows = state
        .get("minds")
        .and_then(serde_json::Value::as_object)
        .expect("the mind rows");
    assert_eq!(
        mind_rows.keys().map(String::as_str).collect::<Vec<_>>(),
        ["10", "4", "5", "9"],
        "one mind row per machine"
    );
    for (unit, row) in mind_rows {
        for moved in [
            "goal",
            "route",
            "searched_ms",
            "searched_from",
            "avoid",
            "refused",
            "waypoint",
            "from_waypoint",
            "hold_retry_ms",
        ] {
            assert!(
                row.get(moved).is_none(),
                "mind row {unit} carries `{moved}`"
            );
        }
    }

    let decoded = Snapshot::decode(&snapshot.document, (*arena.definition).clone())
        .expect("the document decodes");
    // The table, as the document carries it (its scratch, which is not state
    // and is sized by the first search after a resume, aside).
    assert_eq!(
        serde_json::to_value(&decoded.motors).ok(),
        serde_json::to_value(&arena.motors).ok(),
        "the table came back"
    );
    assert_eq!(
        decoded.motors.row(MOVER),
        arena.motors.row(MOVER),
        "the written row came back field for field"
    );
    assert_eq!(
        tacsim::arena::hash::state_hash(&decoded),
        tacsim::arena::hash::state_hash(&arena)
    );

    let mut two = snapshot.document.clone();
    two["format"] = serde_json::json!(2);
    let refused = SnapshotError::Format {
        found: 2,
        expected: SNAPSHOT_FORMAT,
    };
    // 17b cut M1a.4 moved the format to 4, 17b cut M1a.5 to 5, 17b cut L.1
    // to 6, 17b cut M1b.1 to 7, 17b cut M3.2 to 8, 17b cut M4.6 to 9 and 17b
    // cut M5.2 to 10 (the sealed set, the active profile, STOP E's charge); a
    // format-2 document is still refused by name.
    assert_eq!(SNAPSHOT_FORMAT, 11);
    assert_eq!(Snapshot::head(&two), Err(refused.clone()));
    assert_eq!(
        Snapshot::decode(&two, (*arena.definition).clone()).map(|_| ()),
        Err(refused)
    );
}

/// **17b cut M1a.4: a snapshot carries a motor tick and a pilot's motor row,
/// and a format-3 document is refused by name** (doc 17 §4 D-UM-3, D-UM-10;
/// `snapshot.rs`' `SNAPSHOT_FORMAT` 4).
///
/// The mover of [`scripted_arena`] is a pilot's tank: powered up and handed a
/// seat's `Goal { Cell }` in one pulse, which writes its motor row (a row the
/// reflex controller never laid down), runs the landing's drive and puts one
/// `MotorTick` on the heap. The encoded state carries the tick and the row,
/// and the decode hands back the same heap, the same row and the same hash.
/// The same document stamped `"format": 3` — what every snapshot written
/// between 17b cuts M1a.1 and M1a.4 says — is refused as `Format { found: 3,
/// expected: 4 }` by both the head read and the decode.
#[test]
fn m1a4_a_snapshot_carries_a_motor_tick_and_a_pilots_motor_row() {
    use tacsim::arena::intents::MotorGoal;
    use tacsim::arena::state::DueKind;
    let mut arena = scripted_arena();
    if let Some(unit) = arena.unit_mut(MOVER) {
        unit.power = Power::Online;
    }
    // 17b cut M4.5a: the probe reads the machine state (none: no machine
    // flies a pilot's tank), not the reflex table.
    assert_eq!(arena.machine_state(MOVER), None, "a pilot's tank");
    let _ = arena.step(
        0,
        &[],
        &[intent(
            MOVER,
            IntentKind::Goal(MotorGoal::Cell { x: 5, y: 9 }),
            1,
        )],
    );
    let goal = arena.motors.row(MOVER).and_then(|motor| motor.goal);
    assert_eq!(goal, Some((5, 9)), "the pilot's goal is on the motor row");
    let ticks = |arena: &Arena| {
        arena
            .due
            .iter()
            .filter(|event| event.kind == DueKind::MotorTick { unit: MOVER })
            .map(|event| event.t_ms)
            .collect::<Vec<i64>>()
    };
    assert_eq!(ticks(&arena).len(), 1, "one motor tick on the heap");

    let snapshot = Snapshot::encode(&arena).expect("the arena encodes");
    let state = state_of(&snapshot);
    assert!(
        state
            .get("motors")
            .and_then(|motors| motors.get("rows"))
            .and_then(|rows| rows.get(MOVER.0.to_string()))
            .is_some(),
        "the state carries the pilot's motor row"
    );
    assert!(
        state.to_string().contains("\"motor_tick\""),
        "the state carries the motor tick"
    );
    let decoded = Snapshot::decode(&snapshot.document, (*arena.definition).clone())
        .expect("the document decodes");
    assert_eq!(decoded.motors.row(MOVER), arena.motors.row(MOVER));
    assert_eq!(ticks(&decoded), ticks(&arena));
    assert_eq!(
        tacsim::arena::hash::state_hash(&decoded),
        tacsim::arena::hash::state_hash(&arena)
    );

    let mut three = snapshot.document.clone();
    three["format"] = serde_json::json!(3);
    let refused = SnapshotError::Format {
        found: 3,
        expected: SNAPSHOT_FORMAT,
    };
    // 17b cut M1a.5 moved the format to 5, 17b cut L.1 to 6, 17b cut M1b.1
    // to 7, 17b cut M3.2 to 8, 17b cut M4.6 to 9 and 17b cut M5.2 to 10; a
    // format-3 document is still refused by name.
    assert_eq!(SNAPSHOT_FORMAT, 11);
    assert_eq!(Snapshot::head(&three), Err(refused.clone()));
    assert_eq!(
        Snapshot::decode(&three, (*arena.definition).clone()).map(|_| ()),
        Err(refused)
    );
}

/// **17b cut M3.2: a snapshot carries the minds with their pending answers,
/// and a format-7 and a format-4 document are refused by name** (doc 17 §3,
/// §6, D-UM-23 as amended 2026-10-04; `snapshot.rs`' `SNAPSHOT_FORMAT` 8, 9
/// since 17b cut M4.6).
///
/// Was 17b cut M1a.5's `m1a5_a_snapshot_carries_the_minds_and_not_their_landings`,
/// which put an answer in the scratch table (`Arena::mind_landings`) and
/// asserted it was NOT carried: D-UM-23 as amended inverts that premise — the
/// answer is carried, never recomputed — and 17b cut M3.2 retired the
/// scratch.
///
/// The mover of [`scripted_arena`] is handed to a mind (`Arena::set_controller`,
/// which lays its row down and schedules its first think), and its row is
/// written by hand in every field — a think in flight WITH its answer (the
/// three epochs, two intents, the memory it handed back), memory bytes, two
/// notices, a range, a standing order and (17b cut M4.6, the owner's STOP C)
/// its issuer, a drop and a seal digest; and (17b cut M5.2's review, STOP E)
/// the parked unit is a second mind holding a landed `Support` whose charge
/// (`MindState::order_charge`, folded inside the `Support` arm) is the mover.
/// The encoded state carries `minds` with the rows (the supporter's charge
/// as the mover's id) — `pending` with the answer, field for
/// field — and the `mind_think` on the heap, and no `mind_landings` key; the
/// decode hands back the same rows, the default runner and the same hash. The
/// same document stamped `"format": 7` — what every snapshot written between
/// 17b cuts M1b.1 and M3.2 says — and stamped `"format": 4` are refused as
/// `Format { found, expected: 10 }` by both the head read and the decode
/// (`expected: 8` until 17b cut M4.6's format 9, which added `"format": 8` —
/// every snapshot written between 17b cuts M3.2 and M4.6 — to the refused;
/// `expected: 9` until 17b cut M5.2's format 10, which added `"format": 9`,
/// every snapshot written between 17b cuts M4.6 and M5.2).
#[test]
fn m3_a_snapshot_carries_the_minds_with_their_pending_answers() {
    use tacsim::arena::mind::{DropCause, MindState, PendingThink};
    use tacsim::arena::perception::{FrameNotice, FrameOrder, MindObjective};
    use tacsim::arena::state::{ContactId, Controller};
    let mut arena = scripted_arena();
    arena.set_controller(
        MOVER,
        Controller::Mind,
        tacsim::arena::doctrine::MachineState::Withdraw,
    );
    assert!(arena.minds.contains_key(&MOVER), "the adoption laid a row");
    let answer = PendingThink {
        think_seq: 2,
        t_ms: 2_482,
        epochs: [4, 0, 7],
        intents: vec![
            IntentKind::Startup,
            IntentKind::Lock {
                contact: ContactId(77),
            },
        ],
        memory: b"{\"leg\":2}".to_vec(),
    };
    let row = MindState {
        think_seq: 3,
        pending: Some(answer.clone()),
        memory: vec![0xde, 0xad],
        notices: vec![
            FrameNotice::PowerOnline,
            FrameNotice::Miss {
                contact: None,
                reason: SmolStr::new("moved"),
            },
        ],
        ranges: std::collections::BTreeMap::from([(ContactId(77), 450)]),
        order: Some(FrameOrder {
            squad: SmolStr::new("red-1"),
            objective: MindObjective::Engage {
                target: Some(ContactId(77)),
            },
            hold_fire: true,
            max_range_band: None,
            expires_ms: 40_000,
        }),
        // 17b cut M4.6 (the owner's STOP C): the issuer beside the order,
        // carried (and never folded) — `Some`, so the round trip pins it.
        order_issuer: Some(ObjId(77)),
        // 17b cut M5.2 (STOP E): the charge of a held `Support` — none for
        // this `Engage` (the charge's round trip is the second row's, below:
        // the `af1` walk pins its shape only, and never decodes).
        order_charge: None,
        strikes: 3,
        dropped: Some(DropCause::Strikes),
        seal_digest: [0x5e; 32],
        card: tacsim::arena::card::line(&arena.arena_cfg.reflex),
        // 17b cut M4.3: a cache over the memory, never carried — the decode
        // re-derives it (`Startup` off these bytes).
        doctrine_state: tacsim::arena::doctrine::MachineState::Startup,
    };
    arena.minds.insert(MOVER, row.clone());
    // 17b cut M5.2's review (STOP E): a second mind, PARKED, holding a
    // landed `Support` whose charge is the mover — `order_charge`, the one
    // new HASHED decision field (folded inside the `Support` arm), so the
    // row's field-for-field return and the hash equality below pin it.
    arena.set_controller(
        PARKED,
        Controller::Mind,
        tacsim::arena::doctrine::MachineState::Withdraw,
    );
    let supporter = MindState {
        order: Some(FrameOrder {
            squad: SmolStr::new("red-1"),
            objective: MindObjective::Support {
                of: Some(ContactId(78)),
            },
            hold_fire: false,
            max_range_band: None,
            expires_ms: 40_000,
        }),
        order_issuer: Some(ObjId(77)),
        order_charge: Some(MOVER),
        ..MindState::fresh(&arena.arena_cfg.reflex)
    };
    arena.minds.insert(PARKED, supporter.clone());
    let snapshot = Snapshot::encode(&arena).expect("the arena encodes");
    let state = state_of(&snapshot);
    let carried = state
        .get("minds")
        .and_then(|minds| minds.get(MOVER.0.to_string()))
        .cloned()
        .expect("the state carries the mind's row");
    assert_eq!(
        carried
            .get("pending")
            .cloned()
            .and_then(|pending| serde_json::from_value::<PendingThink>(pending).ok()),
        Some(answer),
        "the row carries the pending answer, field for field"
    );
    assert_eq!(carried.get("dropped"), Some(&serde_json::json!("strikes")));
    assert_eq!(
        carried.get("order_issuer"),
        Some(&serde_json::json!(77)),
        "the row carries its order's issuer"
    );
    assert_eq!(
        state
            .get("minds")
            .and_then(|minds| minds.get(PARKED.0.to_string()))
            .and_then(|row| row.get("order_charge")),
        Some(&serde_json::json!(MOVER.0)),
        "the supporter's row carries its charge"
    );
    assert!(
        state.get("mind_landings").is_none() && state.get("mind_runner").is_none(),
        "no scratch table and no runner"
    );
    assert!(
        state.to_string().contains("\"mind_think\""),
        "the think is on the heap"
    );
    let decoded = Snapshot::decode(&snapshot.document, (*arena.definition).clone())
        .expect("the document decodes");
    assert_eq!(
        decoded.minds.get(&MOVER),
        Some(&row),
        "the row came back, its answer with it"
    );
    assert_eq!(
        decoded.minds.get(&PARKED),
        Some(&supporter),
        "the supporter's row came back, its charge with it"
    );
    // 17b cut M1a.6: the default runner is the native stock mind.
    assert!(
        format!("{:?}", decoded.mind_runner).contains("NativeStockMind"),
        "the default runner"
    );
    assert_eq!(
        tacsim::arena::hash::state_hash(&decoded),
        tacsim::arena::hash::state_hash(&arena)
    );

    // 17b cut M3.2 moved the format to 8, 17b cut M4.6 to 9 and 17b cut M5.2
    // to 10: a format-9 document (every snapshot from 17b cut M4.6 to M5.2 —
    // what the first reload onto M5.2 finds and voids), a format-8 one (17b
    // cut M3.2's to M4.6's), a format-7 one (17b cut M1b.1's) and a format-4
    // one are refused by name.
    assert_eq!(SNAPSHOT_FORMAT, 11);
    for old in [10_u32, 9, 8, 7, 4] {
        let mut stale = snapshot.document.clone();
        stale["format"] = serde_json::json!(old);
        let refused = SnapshotError::Format {
            found: old,
            expected: SNAPSHOT_FORMAT,
        };
        assert_eq!(Snapshot::head(&stale), Err(refused.clone()));
        assert_eq!(
            Snapshot::decode(&stale, (*arena.definition).clone()).map(|_| ()),
            Err(refused)
        );
    }
}

/// **17b cut L.1: a snapshot carries every standing order's author, and a
/// format-5 document is refused by name** (doc 17 §4, D-UM-52; `snapshot.rs`'
/// `SNAPSHOT_FORMAT` 6).
///
/// The squad arena is stepped through [`af1_squad_script`] to 10,000 ms, where
/// red-1's lance-leader's `Advance` (15 s from 5,000 ms) still stands on both of
/// its machines. The encoded state carries the order on both reflex rows with
/// its `by` — the persona, `{"kind":"persona",…}` — and the decode hands back
/// the same standing order, author and all, and the same hash (`by` is not
/// folded, so the hash is no evidence of it: the field-for-field compare is).
/// The same document stamped `"format": 5` — what every snapshot written
/// between 17b cuts M1a.5 and L.1 says — is refused as `Format { found: 5,
/// expected: 6 }` by both the head read and the decode.
///
/// **17b cut M4.5a** (ported by 17b cut M4.3's precedent, `tests/cockpit.rs`'
/// `l1_a_typed_order_carries_the_seats_character`; the orchestrator's Q3): the
/// squad's machines are unit minds ([`af1_mind_squad_arena`]), as every
/// AI-filled squad has been since 17b cut M4.3, and a mind's order names no
/// author — D-UM-52's `by` is the world's watch's (`order_author`) and the
/// tape's, never mind-row state, so the snapshot carries none to round-trip.
/// What the snapshot carries of the order is the `FrameOrder` each mind reads
/// — the squad (`red-1`), the objective (`Advance` to `(6, 12)`), the fire
/// constraints and the deadline (20,000 ms) — and that is pinned here field
/// for field: on both mind rows at 10,000 ms, in the encoded `state.minds`,
/// and back from the decode with the same hash. The author half is declared
/// the world's, not re-pinned; the format-5 refusal is unchanged.
#[test]
fn l1_a_snapshot_carries_the_orders_author_and_refuses_a_format_five_document() {
    use tacsim::arena::perception::{FrameOrder, MindObjective};
    let quiet = |_: i64| Tempo::Quiet;
    let mut arena = af1_mind_squad_arena();
    let _ = replay_on_from(
        &mut arena,
        &mut (0..=2).map(|pulse| pulse * 5_000),
        10_000,
        &af1_squad_script(),
        0,
        &quiet,
    );
    let standing = FrameOrder {
        squad: SmolStr::new("red-1"),
        objective: MindObjective::Advance { to: (6, 12) },
        hold_fire: false,
        max_range_band: None,
        expires_ms: 20_000,
    };
    for unit in [MOVER, SQUAD_RED_B] {
        assert_eq!(
            arena.minds.get(&unit).and_then(|row| row.order.clone()),
            Some(standing.clone()),
            "the order stands on {unit:?}'s mind row"
        );
    }
    let snapshot = Snapshot::encode(&arena).expect("the arena encodes");
    let state = state_of(&snapshot);
    for unit in [MOVER, SQUAD_RED_B] {
        let carried = state
            .get("minds")
            .and_then(|minds| minds.get(unit.0.to_string()))
            .and_then(|row| row.get("order"))
            .cloned()
            .and_then(|order| serde_json::from_value::<FrameOrder>(order).ok());
        assert_eq!(
            carried,
            Some(standing.clone()),
            "the mind row of {unit:?} carries the order"
        );
    }
    let decoded = Snapshot::decode(&snapshot.document, (*arena.definition).clone())
        .expect("the document decodes");
    for unit in [MOVER, SQUAD_RED_B] {
        assert_eq!(
            decoded.minds.get(&unit).map(|row| &row.order),
            arena.minds.get(&unit).map(|row| &row.order),
            "the standing order came back"
        );
    }
    assert_eq!(
        tacsim::arena::hash::state_hash(&decoded),
        tacsim::arena::hash::state_hash(&arena)
    );

    let mut five = snapshot.document.clone();
    five["format"] = serde_json::json!(5);
    let refused = SnapshotError::Format {
        found: 5,
        expected: SNAPSHOT_FORMAT,
    };
    // 17b cut M3.2 moved the format to 8, 17b cut M4.6 to 9 and 17b cut M5.2
    // to 10.
    assert_eq!(SNAPSHOT_FORMAT, 11);
    assert_eq!(Snapshot::head(&five), Err(refused.clone()));
    assert_eq!(
        Snapshot::decode(&five, (*arena.definition).clone()).map(|_| ()),
        Err(refused)
    );
}

// ---------------------------------------------------------------------------
// (af3) the resume releases every seat: 16b cut AF.3 (16c N16)
// ---------------------------------------------------------------------------

/// **What a resume injects**: one `Released` on every channel of every seat the
/// decoded arena still holds, each at the epoch the world's mirror sends — the
/// arena's table plus one, as `send_control` bumps it — naming the pilot, as
/// the boot's phase B delivers them through `control_all_channels`.
fn af3_resume_releases(arena: &Arena) -> Vec<Control> {
    arena
        .pilots
        .iter()
        .flat_map(|seat| {
            Channel::ALL.map(|channel| Control {
                unit: seat.unit,
                channel,
                epoch: arena.epochs.current(seat.unit, channel) + 1,
                state: ControlState::Released,
                pilot: Some(seat.pilot),
            })
        })
        .collect()
}

/// [`y3_reclaim_script`] with the resume's releases inserted at `at` (the
/// resumed segment's cursor), as one beat at arena time `t_ms`: delivered by the
/// first pulse that starts at `t_ms` or later, which in a resumed run is the
/// first resumed pulse.
fn af3_script(at: usize, t_ms: i64, releases: &[Control]) -> Vec<Beat> {
    let mut beats = y3_reclaim_script();
    let at = at.min(beats.len());
    beats.insert(at, Beat::at(t_ms, releases.to_vec(), Vec::new()));
    beats
}

/// **16b cut AF.3's mandatory cross-tempo test** (16b §11; doc 16 §3.1 "Rate
/// independence", D-16-2), **well posed**: a resume is not a no-input property
/// (AF.1's test 2 is that one, at every pulse) — it injects an input, a
/// `Released` on every channel of every seat the snapshot still holds, at the
/// first resumed pulse. So the reference is not the uninterrupted run but the
/// uninterrupted run **whose script injects the same `Released` at the same
/// arena time T**.
///
/// Over [`y3_reclaim_script`]'s shape (the pilot seated, linkdead at 35 s,
/// released at 40 s, back at a new epoch at 55 s), for every resume moment T
/// that is a multiple of 5,000 ms in the run:
///
/// - **(a)** at each of 5,000 / 1,000 / 500 ms, the run snapshotted at T,
///   decoded, re-anchored (`resume_at`) and continued from the cursor with the
///   releases delivered on its first resumed pulse is the uninterrupted run at
///   that cadence with the releases scripted at T — in events, hashes, finals
///   and catch-ups;
/// - **(b)** the three cadences' resumed runs at T are one event list, one hash
///   sequence, one final state and one catch-up list.
#[test]
fn af3_a_resume_with_every_seat_released_at_three_cadences_is_one_event_list_and_one_hash_sequence()
{
    let quiet = |_: i64| Tempo::Quiet;
    let mut released_somewhere = 0_u32;
    for t_ms in (0..Y3_UNTIL_MS).step_by(5_000) {
        let mut runs: Vec<(i64, Replay)> = Vec::new();
        for cadence in [5_000_i64, 1_000, 500] {
            let k = t_ms / cadence;
            let mut arena = scripted_arena();
            let (first, cursor) = replay_on_from(
                &mut arena,
                &mut (0..=k).map(|pulse| pulse * cadence),
                t_ms,
                &y3_reclaim_script(),
                0,
                &quiet,
            );
            assert_eq!(arena.now_ms, t_ms, "{cadence} ms stopped at T = {t_ms}");
            let snapshot = Snapshot::encode(&arena).expect("an arena between two pulses encodes");
            let mut resumed =
                Snapshot::decode(&snapshot.document, definition()).expect("and decodes");
            let _first_due = resumed.resume_at(t_ms);
            let releases = af3_resume_releases(&resumed);
            released_somewhere += u32::from(!releases.is_empty());
            let script = af3_script(cursor, t_ms, &releases);
            let (mut second, reached) = replay_on_from(
                &mut resumed,
                &mut ((k + 1)..).map(|pulse| pulse * cadence),
                Y3_UNTIL_MS,
                &script,
                cursor,
                &quiet,
            );
            finish_replay(&mut resumed, &mut second);
            assert_eq!(reached, script.len(), "the script, exactly once");
            let run = joined(&first, &second);

            // (a) The uninterrupted run with the same releases delivered at the
            // same arena time T: the same arena stepped to T and on, never
            // encoded, handed the releases on its first pulse from T. At T > 0
            // that is exactly the plain replay with the releases scripted at T,
            // and the two are held equal; at T = 0 it is not — the opening
            // pulse at 0 has already taken the script's own beats at 0 when a
            // resume could be taken, so the releases land on the next pulse,
            // which also starts at 0, and a script beat at 0 cannot say that.
            let mut reference_arena = scripted_arena();
            let (head, _) = replay_on_from(
                &mut reference_arena,
                &mut (0..=k).map(|pulse| pulse * cadence),
                t_ms,
                &y3_reclaim_script(),
                0,
                &quiet,
            );
            let (mut tail, _) = replay_on_from(
                &mut reference_arena,
                &mut ((k + 1)..).map(|pulse| pulse * cadence),
                Y3_UNTIL_MS,
                &script,
                cursor,
                &quiet,
            );
            finish_replay(&mut reference_arena, &mut tail);
            let reference = joined(&head, &tail);
            if t_ms > 0 {
                let mut plain_arena = scripted_arena();
                let plain = replay(&mut plain_arena, cadence, Y3_UNTIL_MS, &script, &quiet);
                assert_eq!(
                    reference.events, plain.events,
                    "T = {t_ms}: the two references"
                );
                assert_eq!(
                    reference.hashes, plain.hashes,
                    "T = {t_ms}: the two references"
                );
            }
            let what = format!("resumed at T = {t_ms} ms at {cadence} ms");
            assert_eq!(
                run.events,
                reference.events,
                "{what}: {:?} against {:?}",
                run.kinds(),
                reference.kinds()
            );
            assert_eq!(run.hashes, reference.hashes, "{what}: the hashes");
            assert_eq!(run.finals, reference.finals, "{what}: the finals");
            assert_eq!(run.catchups, reference.catchups, "{what}: the catch-ups");
            // A seat the resume released after the script's own `Live` is the
            // machine's for the rest of the run: nothing seats the pilot again.
            if !releases.is_empty() && t_ms > Y3_LIVE_MS {
                // 17b cut M4.3: the machine is a mind seeded `Withdraw`.
                assert_eq!(
                    resumed.unit(MOVER).map(|unit| unit.controller),
                    Some(tacsim::arena::state::Controller::Mind),
                    "{what}: the released tank is the mind's"
                );
            }
            runs.push((cadence, run));
        }
        // (b) The three cadences are one match.
        let Some((_, first)) = runs.first() else {
            panic!("three runs");
        };
        for (cadence, run) in runs.iter().skip(1) {
            let what = format!("resumed at T = {t_ms} ms: {cadence} ms against 5,000 ms");
            assert_eq!(run.events, first.events, "{what}: the events");
            assert_eq!(run.hashes, first.hashes, "{what}: the hashes");
            assert_eq!(run.finals, first.finals, "{what}: the finals");
            assert_eq!(run.catchups, first.catchups, "{what}: the catch-ups");
            assert_ne!(
                run.pulses, first.pulses,
                "{what}: the cadence really differed"
            );
        }
    }
    assert!(
        released_somewhere >= 3,
        "the resume released a seat at too few resume moments to mean anything"
    );
}

// ---------------------------------------------------------------------------
// 17b cut M3.6 — the sortie lines (doc 17 §13; D-UM-45, D-UM-46)
// ---------------------------------------------------------------------------

/// One line's kind as JSON, for the field reads below.
fn m3_kind(line: &Line) -> serde_json::Value {
    serde_json::from_str(&line.1).unwrap_or(serde_json::Value::Null)
}

/// **The sortie rule at an end** (17b cut M3.6): the tape's LAST line is the
/// end's (`end`: `match_ended`, `aborted` or `void`; 16b cut W.2 kept), and
/// immediately before it stand one `sortie_closed { match_end }` per unit of
/// `open`, in that (unit-id) order, at the end's millisecond and under the
/// end's hash; no other `match_end` close is anywhere on the tape.
fn m3_closes_before_the_end(log: &[Line], open: &[ObjId], end: &str) {
    let Some(last) = log.last() else {
        panic!("an empty tape");
    };
    assert_eq!(
        m3_kind(last)["kind"].as_str(),
        Some(end),
        "the end is the tape's last line: {:?}",
        log.iter().rev().take(4).collect::<Vec<_>>()
    );
    let closes: Vec<&Line> = log
        .iter()
        .filter(|line| {
            let kind = m3_kind(line);
            kind["kind"] == "sortie_closed" && kind["word"] == "match_end"
        })
        .collect();
    assert_eq!(
        closes.len(),
        open.len(),
        "one close per open sortie: {closes:?}"
    );
    let before = log.len() - 1 - open.len();
    for (at, unit) in open.iter().enumerate() {
        let line = &log[before + at];
        let kind = m3_kind(line);
        assert_eq!(kind["kind"], "sortie_closed", "{line:?}");
        assert_eq!(kind["word"], "match_end", "{line:?}");
        assert_eq!(kind["unit"].as_u64(), Some(u64::from(unit.0)), "{line:?}");
        assert_eq!(
            kind["sortie"].as_u64(),
            Some(u64::from(unit.0)),
            "the fixture's sortie `in_match(0, unit)`: {line:?}"
        );
        assert_eq!(
            (line.0, line.3),
            (last.0, last.3),
            "the end's millisecond and hash: {line:?} against {last:?}"
        );
    }
}

/// Every line of an arena's tape so far, the open chunk included.
fn m3_tape(arena: &mut Arena, bytes: &mut Vec<u8>) -> Vec<Line> {
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    lines(bytes)
}

/// **The tape opens with the sorties** (17b cut M3.6; doc 17 §13): right
/// after `match_started`, one `mind_sealed` per unit in unit-id order — the
/// custody units handed over in the other order — at `t_ms` 0 under the
/// opening hash, each the record the check-out sealed
/// (`MindSeal::sealed_line`: the mind, the source's first sixteen base-16
/// characters, the card's digest whole, the class, the governor bit, the
/// licence, no principal on a fixture). The ledger's sequence after
/// `Arena::new` is `1 + units`, which `state_hash` folds.
#[test]
fn m3_the_tape_opens_with_one_mind_sealed_per_unit_after_match_started() {
    let parked = tank(PARKED, Team::Blue, centre(5, 12));
    let mover = tank(MOVER, Team::Red, centre(5, 5));
    let want = [mover.seal.sealed_line(), parked.seal.sealed_line()];
    let mut arena = arena_of(vec![parked, mover], &MatchConfig::default());
    assert_eq!(
        arena.ledger.seq(),
        3,
        "match_started and one mind_sealed per unit"
    );
    let mut bytes = Vec::new();
    let log = m3_tape(&mut arena, &mut bytes);
    let Some(opening) = log.first() else {
        panic!("an empty tape");
    };
    assert_eq!(m3_kind(opening)["kind"], "match_started");
    for (line, want) in log.iter().skip(1).zip(&want) {
        assert_eq!(
            line.1,
            serde_json::to_string(want).expect("an event encodes"),
            "the record, never a view"
        );
        assert_eq!(
            (line.0, line.3),
            (0, opening.3),
            "t 0 under the opening hash"
        );
    }
    let sealed = m3_kind(&log[1]);
    assert_eq!(sealed["unit"], 4);
    assert_eq!(sealed["mind"], "Standard Doctrine");
    assert_eq!(sealed["licence"], "doctrine");
    assert_eq!(sealed["source"].as_str().map(str::len), Some(16));
    assert_eq!(sealed["card_digest"].as_str().map(str::len), Some(64));
    assert!(sealed.get("principal").is_none(), "nobody chose: skipped");
    assert_eq!(m3_kind(&log[2])["unit"], 9);
    assert_eq!(log.len(), 3, "nothing else before the first pulse");
}

/// **An elimination inside a substep, and a destroyed unit's own close**
/// (17b cut M3.6): `Arena::destroy_unit` puts the wreck's
/// `sortie_closed { destroyed }` into the journal its `unit_destroyed` went
/// to, right after it, at the destruction's millisecond; the next substep
/// settles the elimination and closes only the sortie still open, before
/// `match_ended`. The wreck's sortie is not closed a second time.
#[test]
fn m3_a_destroyed_sortie_closes_at_its_destruction_and_not_again_at_the_end() {
    let mut arena = scripted_arena();
    let mut bytes = Vec::new();
    absorb(&arena.step(0, &[], &[]), &mut bytes);
    let mut journal = Vec::new();
    arena.destroy_unit(PARKED, arena.now_ms, None, &mut journal);
    let words: Vec<String> = journal
        .iter()
        .map(|(_, kind, _)| serde_json::to_string(kind).expect("an event encodes"))
        .collect();
    assert_eq!(
        words,
        vec![
            r#"{"kind":"unit_destroyed","unit":9}"#.to_owned(),
            r#"{"kind":"sortie_closed","unit":9,"sortie":9,"word":"destroyed"}"#.to_owned(),
        ]
    );
    assert!(arena.unit(PARKED).is_some_and(|unit| unit.seal.closed));
    // A wreck destroyed again journals nothing, its close included.
    let mut again = Vec::new();
    arena.destroy_unit(PARKED, arena.now_ms, None, &mut again);
    assert!(again.is_empty(), "{again:?}");
    let out = arena.step(5_000, &[], &[]);
    absorb(&out, &mut bytes);
    assert_eq!(
        out.ended.map(|end| end.reason),
        Some(EndReason::Elimination {
            winner: Some(Team::Red)
        })
    );
    assert!(out.substeps > 0, "settled inside a substep");
    let log = m3_tape(&mut arena, &mut bytes);
    m3_closes_before_the_end(&log, &[MOVER], "match_ended");
}

/// **The pulse's backstop ends close too** (17b cut M3.6): a side put out
/// before the opening pulse (a rig's direct `destroy_unit`) is settled by the
/// pulse's elimination backstop, and a `max_duration_ms` of `0` by its
/// duration backstop — both journalled outside a substep, at the pulse's
/// millisecond under its hash, each with every open sortie closed first.
#[test]
fn m3_the_pulse_backstops_close_every_open_sortie_before_the_end() {
    // The elimination backstop.
    let mut arena = scripted_arena();
    let mut journal = Vec::new();
    arena.destroy_unit(PARKED, 0, None, &mut journal);
    let mut bytes = Vec::new();
    let out = arena.step(0, &[], &[]);
    absorb(&out, &mut bytes);
    assert_eq!(out.substeps, 0, "the opening pulse runs no substep");
    assert_eq!(
        out.ended.map(|end| end.reason),
        Some(EndReason::Elimination {
            winner: Some(Team::Red)
        })
    );
    let log = m3_tape(&mut arena, &mut bytes);
    m3_closes_before_the_end(&log, &[MOVER], "match_ended");

    // The duration backstop.
    let mut arena = arena_of(
        vec![
            tank(MOVER, Team::Red, centre(5, 5)),
            tank(PARKED, Team::Blue, centre(5, 12)),
        ],
        &MatchConfig {
            max_duration_ms: 0,
            ..MatchConfig::default()
        },
    );
    let mut bytes = Vec::new();
    let out = arena.step(0, &[], &[]);
    absorb(&out, &mut bytes);
    assert_eq!(out.substeps, 0);
    assert_eq!(out.ended.map(|end| end.reason), Some(EndReason::Timeout));
    let log = m3_tape(&mut arena, &mut bytes);
    m3_closes_before_the_end(&log, &[MOVER, PARKED], "match_ended");
}

/// An arena over [`definition`] with its own ledger bounds.
fn m3_arena_with_ledger(ledger: tacsim::config::LedgerConfig) -> Arena {
    let units = vec![
        tank(MOVER, Team::Red, centre(5, 5)),
        tank(PARKED, Team::Blue, centre(5, 12)),
    ];
    let pilots = units
        .iter()
        .enumerate()
        .map(|(seat, unit)| Pilot {
            unit: unit.id,
            pilot: ObjId(1_000 + u32::try_from(seat).unwrap_or(0)),
            station: SmolStr::new("cockpit"),
            epoch: 1,
        })
        .collect();
    Arena::new(
        CustodyOut {
            definition: definition(),
            units,
            pilots,
            resolution: tacsim::match_::stock_resolution(),
            seed: 2_350,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &ArenaConfig {
            ledger,
            ..ArenaConfig::default()
        },
        &MatchConfig::default(),
        &SensorsConfig::default(),
    )
    .expect("the test arena")
}

/// **The egress overflow's `void` closes too; a ledger-full end closes
/// nothing** (17b cut M3.6). Past `ledger.max_unflushed_chunks` the match is
/// voided at the pulse: every open sortie closes before the `void` line. At
/// the ledger's byte bound the match ends `Timeout` with NO line — the ledger
/// is what ran out — and no sortie closes on the tape either: every seal
/// stays open (declared; the end recorded in the result alone).
#[test]
fn m3_an_overflow_void_closes_and_a_ledger_full_end_closes_nothing() {
    // The overflow: one unflushed chunk allowed, more filled by hand.
    let mut arena = m3_arena_with_ledger(tacsim::config::LedgerConfig {
        max_unflushed_chunks: 1,
        ..tacsim::config::LedgerConfig::default()
    });
    while !arena.ledger.overflowed() {
        let _ = arena
            .ledger
            .append(0, EventKind::Lagged { dt_ms: 0 }, Vec::new(), 0);
    }
    let mut bytes = Vec::new();
    let out = arena.step(0, &[], &[]);
    absorb(&out, &mut bytes);
    assert_eq!(
        out.ended.map(|end| end.reason),
        Some(EndReason::LedgerOverflow)
    );
    let log = m3_tape(&mut arena, &mut bytes);
    m3_closes_before_the_end(&log, &[MOVER, PARKED], "void");

    // The byte bound: the opening lines fit, and the first line after them
    // does not.
    let opening = {
        let mut probe = scripted_arena();
        let mut bytes = Vec::new();
        let _ = m3_tape(&mut probe, &mut bytes);
        u64::try_from(bytes.len()).unwrap_or(u64::MAX)
    };
    let mut arena = m3_arena_with_ledger(tacsim::config::LedgerConfig {
        max_bytes: opening + 8,
        ..tacsim::config::LedgerConfig::default()
    });
    let mut bytes = Vec::new();
    let mut ended = None;
    let mut wall = 0_i64;
    let startup = [intent(MOVER, IntentKind::Startup, 1)];
    for pulse in 0..20 {
        let intents: &[Intent] = if pulse == 0 { &startup } else { &[] };
        let out = arena.step(wall, &[], intents);
        absorb(&out, &mut bytes);
        if let Some(end) = out.ended {
            ended = Some(end);
            break;
        }
        wall += 5_000;
    }
    assert_eq!(
        ended.map(|end| end.reason),
        Some(EndReason::Timeout),
        "the byte bound ended the match"
    );
    let log = m3_tape(&mut arena, &mut bytes);
    assert_eq!(
        log.iter()
            .map(|line| m3_kind(line)["kind"].clone())
            .collect::<Vec<_>>(),
        vec!["match_started", "mind_sealed", "mind_sealed"],
        "no end line and no close: the ledger is what ran out"
    );
    assert!(
        arena.units.iter().all(|unit| !unit.seal.closed),
        "no sortie closed"
    );
}

/// **A resumed match does not re-close** (17b cut M3.6): `SealRef::closed` is
/// carried by the snapshot (17b cut M3.2's shape), so a sortie closed before
/// the snapshot — the parked tank's, at its destruction — is not closed again
/// at the resumed match's end: the abort closes the mover's alone.
#[test]
fn m3_a_resumed_match_carries_its_closes_and_does_not_re_close() {
    let mut arena = scripted_arena();
    let mut bytes = Vec::new();
    absorb(&arena.step(0, &[], &[]), &mut bytes);
    let mut journal = Vec::new();
    arena.destroy_unit(PARKED, arena.now_ms, None, &mut journal);
    let snapshot = Snapshot::encode(&arena).expect("an arena between two pulses encodes");
    let mut resumed =
        Snapshot::decode(&snapshot.document, definition()).expect("the snapshot decodes");
    assert!(
        resumed.unit(PARKED).is_some_and(|unit| unit.seal.closed),
        "the close is carried"
    );
    assert!(resumed.unit(MOVER).is_some_and(|unit| !unit.seal.closed));
    let _ = resumed.resume_at(0);
    resumed.stop(StopReason::Abort);
    let mut tail = Vec::new();
    absorb(&resumed.step(5_000, &[], &[]), &mut tail);
    let log = m3_tape(&mut resumed, &mut tail);
    m3_closes_before_the_end(&log, &[MOVER], "aborted");
}

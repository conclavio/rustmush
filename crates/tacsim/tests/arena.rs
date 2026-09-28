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
use tacsim::arena::rng::{DrawLabel, draw};
use tacsim::arena::state::{
    Arena, ArenaDefinition, ArenaTerrain, Channel, CustodyOut, CustodyUnit, EndReason, EntryZone,
    Pilot, Power, StopReason, Team, TerrainCell, TrackedRules,
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
    }
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
fn replay_on(
    arena: &mut Arena,
    walls: &mut dyn Iterator<Item = i64>,
    until_ms: i64,
    script: &[Beat],
    pin: &dyn Fn(i64) -> Tempo,
) -> Replay {
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let Some(mut wall) = walls.next() else {
        panic!("a replay needs at least its opening wall reading");
    };
    let mut next = 0_usize;
    let mut pulses = 0_u32;
    let mut substeps = 0_u32;
    let mut told: Vec<String> = Vec::new();
    let mut pulses_seen: Vec<(i64, bool, bool)> = Vec::new();
    let mut catchups: Vec<(i64, tacsim::arena::Catchup)> = Vec::new();
    let mut observers: Vec<i64> = Vec::new();
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
            next += 1;
        }
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
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        let Some(reading) = walls.next() else {
            break;
        };
        wall = reading;
    }
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    Replay {
        events: lines(&bytes),
        hashes,
        finals: arena
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
            .collect(),
        pulses,
        substeps,
        told,
        pulses_seen,
        catchups,
        observers,
    }
}

/// One [`Replay::pulses_seen`] row: the pulse's start, whether the mover's
/// pilot was built a frame, and whether the mover is the machine's now.
fn seen(arena: &Arena, started: i64, out: &StepOutput) -> (i64, bool, bool) {
    (
        started,
        out.frames.iter().any(|(pilot, _)| *pilot == MOVER_PILOT),
        arena.reflex.unit(MOVER).is_some(),
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
    arena.arena_cfg.tempo.quiet_ms = period_ms;
    arena.clock.set_floor(floor);
    let mut walls: Vec<i64> = Vec::new();
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = 0_i64;
    let mut next = 0_usize;
    let mut pulses = 0_u32;
    let mut substeps = 0_u32;
    let mut told: Vec<String> = Vec::new();
    let mut woke_at: Vec<i64> = Vec::new();
    let mut pulses_seen: Vec<(i64, bool, bool)> = Vec::new();
    let mut catchups: Vec<(i64, tacsim::arena::Catchup)> = Vec::new();
    let mut observers: Vec<i64> = Vec::new();
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
        if pulses > 0 {
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
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        assert!(
            out.next_due_ms > wall,
            "the task was asked to wake in the past"
        );
        wall = out.next_due_ms;
    }
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    let replay = Replay {
        events: lines(&bytes),
        hashes,
        finals: arena
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
            .collect(),
        pulses,
        substeps,
        told,
        pulses_seen,
        catchups,
        observers,
    };
    (replay, woke_at, walls)
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
/// `Released` at 40,000 — which hands it to the reflex controller, so the
/// machine's ticks and searches run under the floor too.
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
                "the reflex row at the pulse from {started} ms, at {cadence} ms"
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
    // to the reflex controller in `Withdraw`, which puts its first tick on the
    // schedule beside the position summary.
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
        let expected =
            serde_json::to_string(&EventKind::MatchEnded { reason }).expect("an event encodes");
        assert_eq!(
            last,
            Some(expected),
            "{stop:?}: it is an end with its own reason and not an abort"
        );
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
// 16b cut Y.1 — the issuer is state; the spot is not
// ---------------------------------------------------------------------------

/// **16b cut Y.1** (the cut's one hash move): two arenas equal but for a
/// standing order's `issuer` hash differently — the issuer decides which object
/// a hint names, so the two are two states and a replay that lost it must not
/// compare equal — while an order's `spot`, the commander's word for a cell
/// that nothing steering reads, moves nothing.
#[test]
fn the_hash_folds_the_issuer_and_not_the_spot() {
    let told = |issuer: Option<ObjId>, spot: Option<&str>| tacsim::deliberation::Order {
        squad: tacsim::deliberation::SquadId {
            team: Team::Red,
            index: 1,
        },
        objective: tacsim::deliberation::Objective::Advance { to: (12, 9) },
        target: None,
        horizon_ms: 600_000,
        constraints: tacsim::deliberation::Constraints::default(),
        ttl_ms: 600_000,
        issued_ms: 0,
        issuer,
        spot: spot.map(SmolStr::new),
    };
    let under = |order: tacsim::deliberation::Order| {
        let mut arena = arena_of(
            vec![
                tank(MOVER, Team::Red, centre(10, 10)),
                tank(PARKED, Team::Blue, centre(30, 30)),
            ],
            &MatchConfig::default(),
        );
        arena.reflex.set_order(
            MOVER,
            tacsim::deliberation::StandingOrder::accepted(order, 0),
        );
        tacsim::arena::hash::state_hash(&arena)
    };
    let machines = under(told(None, None));
    let pilots = under(told(Some(ObjId(77)), None));
    let another = under(told(Some(ObjId(78)), None));
    assert_ne!(machines, pilots, "the issuer is folded");
    assert_ne!(pilots, another, "and which issuer it is");
    assert_eq!(
        under(told(Some(ObjId(77)), Some("The Ford"))),
        pilots,
        "the spot is the commander's word and steers nothing"
    );
    assert_eq!(under(told(None, Some("The Ford"))), machines);
}

// ---------------------------------------------------------------------------
// 16b cut Z.2 — the two squad roles' payloads are state
// ---------------------------------------------------------------------------

/// **16b cut Z.2** (the cut's one hash move): two arenas equal but for a
/// standing `Screen`'s `facing_mdeg` hash differently, and so do two equal but
/// for a `Support`'s charge `of` — the arc a screen watches and the unit a
/// support keeps station on steer the machine, so a replay that lost either
/// must not compare equal. The two roles are two states of their own too.
#[test]
fn the_hash_folds_a_screens_facing_and_a_supports_charge() {
    let told = |objective: tacsim::deliberation::Objective| tacsim::deliberation::Order {
        squad: tacsim::deliberation::SquadId {
            team: Team::Red,
            index: 1,
        },
        objective,
        target: None,
        horizon_ms: 600_000,
        constraints: tacsim::deliberation::Constraints::default(),
        ttl_ms: 600_000,
        issued_ms: 0,
        issuer: Some(ObjId(77)),
        spot: None,
    };
    let under = |objective: tacsim::deliberation::Objective| {
        let mut arena = arena_of(
            vec![
                tank(MOVER, Team::Red, centre(10, 10)),
                tank(PARKED, Team::Blue, centre(30, 30)),
            ],
            &MatchConfig::default(),
        );
        arena.reflex.set_order(
            MOVER,
            tacsim::deliberation::StandingOrder::accepted(told(objective), 0),
        );
        tacsim::arena::hash::state_hash(&arena)
    };
    let east = under(tacsim::deliberation::Objective::Screen {
        facing_mdeg: 90_000,
    });
    let west = under(tacsim::deliberation::Objective::Screen {
        facing_mdeg: 270_000,
    });
    assert_ne!(east, west, "the facing is folded");
    let on_one = under(tacsim::deliberation::Objective::Support { of: ObjId(301) });
    let on_two = under(tacsim::deliberation::Objective::Support { of: ObjId(302) });
    assert_ne!(on_one, on_two, "the charge is folded");
    assert_ne!(east, on_one, "and the two roles are two states");
    assert_eq!(
        under(tacsim::deliberation::Objective::Screen {
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

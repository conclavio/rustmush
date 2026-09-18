//! **The arena's determinism, end to end through `Arena::step`** (16b cut A.2,
//! A7; doc 16 §3.1 "Rate independence" and "Determinism", §3.4, §8's *cross-tempo
//! property test*).
//!
//! Eight tests, driving the library directly — no host, no store, no task, which
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
//! - **(g)** the window's edge stops a unit with one notice.
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
use tacsim::arena::rng::{DrawLabel, draw};
use tacsim::arena::state::{
    Arena, ArenaDefinition, ArenaTerrain, Channel, CustodyOut, CustodyUnit, EndReason, EntryZone,
    Pilot, Power, StopReason, Team, TerrainCell, TrackedRules,
};
use tacsim::arena::step::StepOutput;
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
            seed: 2_350,
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
}

/// One ledger line, normalised for comparison: its arena timestamp, its kind and
/// fields as JSON, the draw where one was made, and the substep hash. The `seq` is
/// dropped because it is the one field a different number of pulses could
/// legitimately renumber.
type Line = (i64, String, Option<(String, u64)>, u64);

/// The same line with the hash column struck out — what two runs at two different
/// tempos can be compared on, the hash folding the tempo byte by design.
type Bare = (i64, String, Option<(String, u64)>);

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
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = 0_i64;
    let mut next = 0_usize;
    let mut pulses = 0_u32;
    let mut substeps = 0_u32;
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
            next += 1;
        }
        let out = arena.step(wall, &controls, &intents);
        absorb(&out, &mut bytes);
        hashes.extend(arena.substep_hashes.iter().copied());
        pulses += 1;
        substeps += out.substeps;
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
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
    }
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
                event.draw,
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
fn script() -> Vec<Beat> {
    vec![
        Beat {
            t_ms: 0,
            controls: Vec::new(),
            intents: vec![intent(MOVER, IntentKind::Startup, 1)],
        },
        Beat {
            t_ms: 10_000,
            controls: Vec::new(),
            intents: vec![
                intent(MOVER, IntentKind::Heading { mdeg: 0 }, 2),
                intent(MOVER, IntentKind::Speed { cms: 1_000 }, 3),
            ],
        },
        Beat {
            t_ms: 25_000,
            controls: Vec::new(),
            intents: vec![
                intent(MOVER, IntentKind::Heading { mdeg: 90_000 }, 4),
                intent(MOVER, IntentKind::Speed { cms: 1_000 }, 5),
            ],
        },
        Beat {
            t_ms: 40_000,
            controls: vec![Control {
                unit: MOVER,
                channel: Channel::Helm,
                epoch: 1,
                state: ControlState::Linkdead,
            }],
            intents: Vec::new(),
        },
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
    assert_eq!(
        first.count("blocked"),
        2,
        "the log did not stop twice — once against the parked tank, once at the \
         band: {:?}",
        first.kinds()
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
    assert_eq!(mover.4, 90_000, "the mover is not facing east");
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
        // And the cadence really did differ: the pulse counts are the proof that
        // the three runs were not the same run.
        assert_ne!(run.pulses, first.pulses, "{cadence} ms ran the same pulses");
    }
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
        }],
        &[intent(MOVER, IntentKind::Startup, 1)],
    );
    assert!(out.ended.is_none());
    assert_eq!(arena.unit(MOVER).map(|unit| unit.power), Some(Power::Off));
    assert!(arena.pending.is_empty(), "the order was not purged");
    assert_eq!(arena.epochs.current(MOVER, Channel::Helm), 2);
    assert_eq!(arena.due.len(), 1, "nothing but the position summary");

    // Ten seconds of arena time later — `match.startup_ms` — it is still cold.
    let mut wall = 0_i64;
    for _ in 0..4 {
        wall += 5_000;
        arena.step(wall, &[], &[]);
    }
    assert_eq!(arena.now_ms, 20_000);
    assert_eq!(arena.unit(MOVER).map(|unit| unit.power), Some(Power::Off));
    let mut bytes = Vec::new();
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    let journalled = lines(&bytes);
    assert!(
        !journalled
            .iter()
            .any(|(_, kind, _, _)| kind.contains("startup")),
        "a startup was journalled: {journalled:?}"
    );

    // And at the epoch the world published, the same order works.
    arena.step(
        wall + 5_000,
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
        Some(Power::Starting)
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
    assert_eq!(
        ended.t_ms, 15_000,
        "the pulse that crossed the bound ended it"
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
    assert_eq!(arena.now_ms, 15_000);

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
        .map(|(pilot, frame)| (*pilot, frame.blocked))
        .collect();
    assert_eq!(last.get(&ObjId(1_000)), Some(&true));
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
        Beat {
            t_ms: 0,
            controls: Vec::new(),
            intents: vec![intent(MOVER, IntentKind::Startup, 1)],
        },
        Beat {
            t_ms: 10_000,
            controls: Vec::new(),
            intents: vec![
                intent(MOVER, IntentKind::Heading { mdeg: 0 }, 2),
                intent(MOVER, IntentKind::Speed { cms: 1_000 }, 3),
            ],
        },
        Beat {
            t_ms: 35_000,
            controls: Vec::new(),
            intents: vec![intent(MOVER, IntentKind::Speed { cms: -400 }, 4)],
        },
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
        // Both passes are scheduled at `id % pass_ms`, so at 4 ms and 9 ms.
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
    let drawn: Vec<&Option<(String, u64)>> = first
        .events
        .iter()
        .filter(|(_, kind, _, _)| kind.contains("classified"))
        .map(|(_, _, draw, _)| draw)
        .collect();
    assert_eq!(drawn.len(), 2);
    for draw in drawn {
        let (label, _) = draw
            .clone()
            .unwrap_or_else(|| panic!("a classification carries its draw"));
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

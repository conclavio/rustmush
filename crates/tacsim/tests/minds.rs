//! **The mind seam, driven** (17b cut M1a.5; doc 17 §3, §4, §6, §7; D-UM-2,
//! D-UM-8, D-UM-9, D-UM-11, D-UM-12, D-UM-13, D-UM-14, D-UM-15, D-UM-20,
//! D-UM-21, D-UM-47).
//!
//! `crates/tacsim/src/arena/mind.rs` is the arena's half of a unit mind: the
//! state, the view, the think and the landing. What answers a think lives
//! outside the library, so this file drives the seam with a [`ScriptedMind`] —
//! a [`MindRunner`] that answers each hull's think `n` with the intents its
//! script names for `n` and records every job it was handed. It is an input to
//! `Arena::step`, the way a scripted intent log is, and not a stand-in for a
//! production module (16b §0.4): 17b cut M1a.6's native stock mind is the
//! production runner, and builds its twin fixture on this one.
//!
//! The rig is `tests/reflex.rs`' sixteen-cell window, copied (an integration
//! test cannot import another): a five-cell sight, a cannon whose one band
//! reaches the window, a 250 ms substep, decide every 500 ms. The two pins of
//! 17b cut M1a.1 are re-asserted here too, because this cut adds a fold section,
//! three carried variants and a due-loop rule that production must never feel.

mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use mush_core::{DefinitionDigest, DefinitionId, DefinitionKey, DefinitionKind, ObjId, OwnerId};
use mush_sim::data::Pose;
use mush_tactics::{
    CellSpec, DefinitionRef, FacetId, FacetSpec, RulesValue, SectionId, SectionSpec, TacticalUnit,
    UnitSpec,
};
use smol_str::SmolStr;
use tacsim::arena::clock::Tempo;
use tacsim::arena::intents::{
    Control, ControlState, Intent, IntentKind, MotorGoal, Refusal, apply_control,
};
use tacsim::arena::ledger::LedgerEvent;
use tacsim::arena::mind::{
    MIND_INTENTS_MAX, MIND_NOTICES, MindRunner, MindState, MindView, STOCK_THINK_MS, ThinkJob,
    ThinkResult,
};
use tacsim::arena::perception::{
    FrameNotice, MindObjective, RefusalDetail, SensorProfile, SignatureFactor,
};
use tacsim::arena::reflex::{ReflexCounters, ReflexState};
use tacsim::arena::rng::{draw_contact_id, stagger_ms};
use tacsim::arena::snapshot::Snapshot;
use tacsim::arena::state::{
    Arena, ArenaDefinition, ArenaTerrain, Channel, ContactId, Controller, CustodyOut, CustodyUnit,
    DueKind, EntryZone, Pilot, Team, TerrainCell, TrackedRules, Waypoint, Waypoints, WeaponSummary,
};
use tacsim::arena::step::StepOutput;
use tacsim::arena::weapons::WeaponBand;
use tacsim::config::{ArenaConfig, MatchConfig, ReflexConfig, SensorsConfig};
use tacsim::deliberation::order::{Constraints, Objective, Order};
use tacsim::deliberation::squad::SquadId;

/// The window's side, cells.
const SIDE: u16 = 16;
/// One cell, centimetres.
const CELL_CM: u16 = 1_000;
/// The match seed — `tests/reflex.rs`' own.
const SEED: u64 = 2_350;
/// The substep this rig cuts (`tests/reflex.rs` asserts the same).
const SUBSTEP: i64 = 250;
/// The armed mind, and the reflex rig's armed machine.
const RED: ObjId = ObjId(4);
/// The unarmed mind, and the reflex rig's unarmed machine.
const RED2: ObjId = ObjId(6);
/// The pilot both reds can see.
const BLUE: ObjId = ObjId(9);
/// A pilot nobody can see, so a kill of [`BLUE`] does not end the match.
const BLUE2: ObjId = ObjId(12);

// ---------------------------------------------------------------------------
// The rig (`tests/reflex.rs`', copied)
// ---------------------------------------------------------------------------

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
    RulesValue::new(dref("schema", "tacsim.arena.test"), serde_json::json!({}))
        .expect("an empty rules blob")
}

/// `tests/reflex.rs`' eighteen-point tank: one hull section, one front facet.
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

/// The cell centre of `(x, y)`.
fn centre(x: u16, y: u16) -> Pose {
    Pose::at(
        i64::from(x) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
        i64::from(y) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
        0,
        rules(),
    )
}

/// The sixteen-cell window of uniform ground, its two entry columns and the
/// two-node patrol graph on row 8.
fn definition() -> ArenaDefinition {
    ArenaDefinition {
        id: SmolStr::new("tacsim.arena.reflex"),
        terrain: ArenaTerrain::new(
            SIDE,
            SIDE,
            CELL_CM,
            vec![ground(); usize::from(SIDE) * usize::from(SIDE)],
        )
        .expect("the test window"),
        entry_zones: vec![
            EntryZone {
                team: Team::Red,
                x0: 0,
                y0: 0,
                x1: 1,
                y1: SIDE - 1,
            },
            EntryZone {
                team: Team::Blue,
                x0: SIDE - 2,
                y0: 0,
                x1: SIDE - 1,
                y1: SIDE - 1,
            },
        ],
        key_spots: Vec::new(),
        waypoints: Waypoints::new(
            vec![
                Waypoint {
                    id: SmolStr::new("west"),
                    x: 1,
                    y: 8,
                },
                Waypoint {
                    id: SmolStr::new("east"),
                    x: 14,
                    y: 8,
                },
            ],
            &[(SmolStr::new("west"), SmolStr::new("east"))],
        ),
        modes: Vec::new(),
    }
}

/// A five-cell sight with no mesh.
fn sight() -> SensorProfile {
    use mush_atlas::TerrainClass as C;
    let row = |class: C, moving: u32, stopped: u32| SignatureFactor {
        class,
        moving_permille: moving,
        stopped_permille: stopped,
    };
    SensorProfile {
        component: SmolStr::new("visual_sensor"),
        max_range_cells: 5,
        max_attenuation: 40,
        classify_after_ms: 3_000,
        sees_wrecks: false,
        share_range_cells: 0,
        signature: vec![row(C::Ground, 1_000, 1_000), row(C::Grass, 1_000, 800)],
    }
}

/// A cannon whose one band reaches the whole window.
fn cannon() -> WeaponSummary {
    WeaponSummary {
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
        damage: 120,
        mount_section: SmolStr::new("turret"),
    }
}

/// A tank that sees, with `weapons` and `controller` — `tests/reflex.rs`' own
/// helper, which a [`Controller::Mind`] fields as a mind.
fn tank(
    id: ObjId,
    team: Team,
    pose: Pose,
    weapons: Vec<WeaponSummary>,
    controller: Controller,
) -> CustodyUnit {
    CustodyUnit {
        id,
        team,
        pose,
        mobility: TrackedRules {
            max_speed_cms: 1_000,
            accel_cms2: 500,
            brake_cms2: 500,
            turn_rate_mdeg_s: 180_000,
            reverse_factor_pct: 40,
        },
        sensor_height_cm: 150,
        sensor: Some(sight()),
        weapons,
        profile: tacsim::arena::resolution::UnitProfile::default(),
        gunnery: 0,
        controller,
        slot: 0,
        unit: tactical_unit(id),
    }
}

/// `tests/reflex.rs`' doctrine: decide every 500 ms, re-plan at once.
fn reflex_cfg() -> ReflexConfig {
    ReflexConfig {
        decide_ms: 500,
        repath_ms: 0,
        ..ReflexConfig::default()
    }
}

/// An arena of `units` on the window, `seated` pilots by unit (`#1000` up),
/// power up in half a second, the tempo pinned `Quiet`.
fn arena_of(units: Vec<CustodyUnit>, seated: &[ObjId]) -> Arena {
    let pilots = seated
        .iter()
        .enumerate()
        .map(|(seat, unit)| Pilot {
            unit: *unit,
            pilot: ObjId(1_000 + u32::try_from(seat).unwrap_or(0)),
            station: SmolStr::new("cockpit"),
            epoch: 1,
        })
        .collect();
    let mut arena = Arena::new(
        CustodyOut {
            definition: definition(),
            units,
            pilots,
            resolution: tacsim::match_::stock_resolution(),
            seed: SEED,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &ArenaConfig {
            reflex: reflex_cfg(),
            ..ArenaConfig::default()
        },
        &MatchConfig {
            startup_ms: 500,
            ..MatchConfig::default()
        },
        &SensorsConfig::default(),
    )
    .expect("the test arena");
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    arena
}

fn events(arena: &mut Arena, bytes: &mut Vec<u8>) -> Vec<LedgerEvent> {
    arena.ledger.flush();
    for chunk in arena.ledger.take_chunks() {
        bytes.extend_from_slice(&chunk.bytes);
    }
    String::from_utf8(bytes.clone())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<LedgerEvent>(line).ok())
        .collect()
}

/// One intent on the channel that carries it, at the unit's current epoch.
fn intent(arena: &Arena, unit: ObjId, kind: IntentKind, seq: u64) -> Intent {
    let channel = kind.channel_required();
    Intent {
        unit,
        channel,
        epoch: arena.epochs.current(unit, channel),
        kind,
        seq,
    }
}

/// The lattice millisecond of `unit`'s think `n`.
fn think_at(unit: ObjId, n: i64) -> i64 {
    stagger_ms(SEED, unit, STOCK_THINK_MS) + n * STOCK_THINK_MS
}

// ---------------------------------------------------------------------------
// The scripted mind
// ---------------------------------------------------------------------------

/// One scripted think: what it answers, and the memory it hands back (the
/// memory it was handed, echoed, when `None`).
#[derive(Clone, Debug)]
struct Says {
    think_seq: u64,
    intents: Vec<IntentKind>,
    memory: Option<Vec<u8>>,
}

fn says(think_seq: u64, intents: Vec<IntentKind>) -> Says {
    Says {
        think_seq,
        intents,
        memory: None,
    }
}

/// What a think was handed.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen {
    unit: ObjId,
    think_seq: u64,
    t_ms: i64,
    view: MindView,
    memory: Vec<u8>,
}

/// **A [`MindRunner`] that reads its answers off a script** — per hull, the
/// intents and the memory of think `n` — and records every job it is handed.
/// A pure function of the job and the script, as a runner must be.
#[derive(Debug, Default)]
struct ScriptedMind {
    script: BTreeMap<ObjId, Vec<Says>>,
    seen: Mutex<Vec<Seen>>,
}

impl ScriptedMind {
    fn new(script: &[(ObjId, Vec<Says>)]) -> Arc<ScriptedMind> {
        Arc::new(ScriptedMind {
            script: script.iter().cloned().collect(),
            seen: Mutex::new(Vec::new()),
        })
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .map(|seen| seen.clone())
            .unwrap_or_default()
    }
}

impl MindRunner for ScriptedMind {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        if let Ok(mut seen) = self.seen.lock() {
            seen.push(Seen {
                unit: job.unit,
                think_seq: job.think_seq,
                t_ms: job.t_ms,
                view: job.view.clone(),
                memory: job.memory.to_vec(),
            });
        }
        let line = self
            .script
            .get(&job.unit)
            .and_then(|lines| lines.iter().find(|line| line.think_seq == job.think_seq));
        ThinkResult {
            intents: line.map(|line| line.intents.clone()).unwrap_or_default(),
            memory: line
                .and_then(|line| line.memory.clone())
                .unwrap_or_else(|| job.memory.to_vec()),
        }
    }
}

/// How a run is driven: a fixed wall cadence, or the task's own wakes with the
/// `Quiet` period set to the figure given (`tests/reflex.rs`' `Drive`).
#[derive(Clone, Copy, Debug)]
enum Drive {
    Every(i64),
    Task(u32),
}

/// What one run produced.
#[derive(Debug)]
struct Run {
    events: Vec<(i64, String, u64)>,
    hashes: Vec<u64>,
    pulses: u32,
    counters: ReflexCounters,
}

/// Step `arena` to `until_ms` under `drive`.
fn run(arena: &mut Arena, drive: Drive, until_ms: i64) -> Run {
    if let Drive::Task(period_ms) = drive {
        arena.arena_cfg.tempo.quiet_ms = period_ms;
    }
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut counters = ReflexCounters::default();
    let mut wall = arena.now_ms;
    let mut pulses = 0_u32;
    loop {
        let out = arena.step(wall, &[], &[]);
        pulses = pulses.saturating_add(1);
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        counters.absorb(&out.reflex);
        hashes.extend(arena.substep_hashes.iter().copied());
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        wall = match drive {
            Drive::Every(cadence_ms) => wall + cadence_ms,
            Drive::Task(_) => out.next_due_ms,
        };
    }
    let events = events(arena, &mut bytes)
        .into_iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    Run {
        events,
        hashes,
        pulses,
        counters,
    }
}

/// The kind word and the unit of a journalled line, as `run` keeps it.
fn kind_and_unit(json: &str) -> (String, Option<u32>) {
    let value: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
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

/// **A rig driven one substep a pulse**: the arena, the wall it was last
/// stepped at (the first pulse is at wall `0`, which anchors arena time to
/// it) and every ledger byte its pulses handed out.
struct Rig {
    arena: Arena,
    wall: Option<i64>,
    bytes: Vec<u8>,
}

impl Rig {
    fn new(arena: Arena) -> Rig {
        Rig {
            arena,
            wall: None,
            bytes: Vec::new(),
        }
    }

    /// One pulse of one substep, with `controls` and `intents`.
    fn pulse(&mut self, controls: &[Control], intents: &[Intent]) -> StepOutput {
        let wall = self.wall.map_or(0, |wall| wall + SUBSTEP);
        self.wall = Some(wall);
        let out = self.arena.step(wall, controls, intents);
        for chunk in &out.ledger_chunks {
            self.bytes.extend_from_slice(&chunk.bytes);
        }
        out
    }

    /// Pulse until `done` holds after a pulse, at most to `limit_ms` of arena
    /// time; answers the counters of the pulse it held after.
    fn until(&mut self, limit_ms: i64, mut done: impl FnMut(&Arena) -> bool) -> ReflexCounters {
        while self.arena.now_ms < limit_ms {
            let out = self.pulse(&[], &[]);
            if done(&self.arena) {
                return out.reflex;
            }
        }
        panic!("the condition never held by {limit_ms} ms");
    }

    /// Every line journalled so far.
    fn journal(&mut self) -> Vec<LedgerEvent> {
        events(&mut self.arena, &mut self.bytes)
    }

    /// The milliseconds of the `kind` lines of `unit` so far.
    fn lines(&mut self, kind: &str, unit: ObjId) -> Vec<i64> {
        self.journal()
            .iter()
            .filter(|event| {
                let (word, on) =
                    kind_and_unit(&serde_json::to_string(&event.kind).unwrap_or_default());
                word == kind && on == Some(unit.0)
            })
            .map(|event| event.t_ms)
            .collect()
    }
}

/// Whether `unit`'s mind has think `n` in flight.
fn pending(arena: &Arena, unit: ObjId, n: u64) -> bool {
    arena
        .minds
        .get(&unit)
        .and_then(|state| state.pending)
        .is_some_and(|pending| pending.think_seq == n)
}

/// The buffered refusals of `unit`'s mind, as `(order, refusal)`.
fn refusals(arena: &Arena, unit: ObjId) -> Vec<(String, Refusal)> {
    arena
        .minds
        .get(&unit)
        .map(|state| {
            state
                .notices
                .iter()
                .filter_map(|notice| match notice {
                    FrameNotice::Refused { order, refusal, .. } => {
                        Some((order.to_string(), *refusal))
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// (1) and (2): two scripted minds at three cadences; the think's two clocks
// ---------------------------------------------------------------------------

/// The cross-tempo field: two minds of red — the armed one starts, drives,
/// locks, fires and switches a pod it does not have; the unarmed one starts,
/// locks and fires a gun it does not have and switches the same missing pod —
/// a seated blue they can both see, and a blue nobody sees, so a kill ends
/// nothing.
fn two_minds(drive: Drive) -> (Run, Vec<Seen>) {
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                Controller::Mind,
            ),
            tank(RED2, Team::Red, centre(6, 10), Vec::new(), Controller::Mind),
            tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            ),
            tank(
                BLUE2,
                Team::Blue,
                centre(14, 1),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[BLUE],
    );
    let gun = SmolStr::new("cannon");
    let mind = ScriptedMind::new(&[
        (
            RED,
            vec![
                says(0, vec![IntentKind::Startup]),
                says(
                    3,
                    vec![
                        IntentKind::Goal(MotorGoal::Cell { x: 6, y: 5 }),
                        IntentKind::Lock {
                            contact: draw_contact_id(SEED, RED, BLUE),
                        },
                    ],
                ),
                says(
                    5,
                    vec![IntentKind::Fire {
                        weapon: gun.clone(),
                    }],
                ),
                says(6, vec![IntentKind::Emcon { on: true }]),
            ],
        ),
        (
            RED2,
            vec![
                says(0, vec![IntentKind::Startup]),
                says(
                    2,
                    vec![IntentKind::Lock {
                        contact: draw_contact_id(SEED, RED2, BLUE),
                    }],
                ),
                says(4, vec![IntentKind::Fire { weapon: gun }]),
                says(6, vec![IntentKind::Emcon { on: true }]),
            ],
        ),
    ]);
    arena.mind_runner = mind.clone();
    let run = run(&mut arena, drive, 15_000);
    (run, mind.seen())
}

/// **17b cut M1a.5's mandatory cross-tempo property** (16b §11; doc 16 §3.1
/// D-16-2; doc 17 §6 D-UM-20, D-UM-21). Two scripted minds and a seated pilot,
/// replayed at 5,000, 1,000 and 500 ms and on the task's own wakes at the same
/// three periods: one event list, one per-substep hash sequence and one
/// sequence of jobs handed to the runner (each think's millisecond, view and
/// memory). Every `startup`, `locked` and `fired` line of a mind is stamped one
/// substep after its think's lattice millisecond and never on it; the pod the
/// hulls do not carry is refused `no_jammer` into each mind's buffer, and the
/// unarmed hull's `FIRE` `channel`, both read off the next think's view, as is
/// a pilot's `Power online.` notice staged on the hull.
#[test]
fn m1a5_two_scripted_minds_at_three_cadences_are_one_event_list_and_one_hash_sequence() {
    let (first, seen) = two_minds(Drive::Every(5_000));
    let words: Vec<(String, Option<u32>)> = first
        .events
        .iter()
        .map(|(_, json, _)| kind_and_unit(json))
        .collect();
    for want in ["startup", "locked", "fired"] {
        assert!(
            words
                .iter()
                .any(|(word, unit)| word == want && *unit == Some(RED.0)),
            "the armed mind never journalled a {want}: {words:?}"
        );
    }
    assert!(
        first.counters.thinks >= 20,
        "{} thinks",
        first.counters.thinks
    );
    for machine in [RED, RED2] {
        assert_ne!(
            stagger_ms(SEED, machine, STOCK_THINK_MS) % SUBSTEP,
            0,
            "the precondition: #{}'s think is off the lattice",
            machine.0
        );
    }
    let mut stamped = 0_usize;
    for (t_ms, json, _) in &first.events {
        let (word, unit) = kind_and_unit(json);
        if !matches!(word.as_str(), "startup" | "locked" | "fired" | "emcon") {
            continue;
        }
        let Some(unit) = unit.filter(|unit| [RED.0, RED2.0].contains(unit)) else {
            continue;
        };
        let think = stagger_ms(SEED, ObjId(unit), STOCK_THINK_MS);
        assert_eq!(
            (t_ms - SUBSTEP - think).rem_euclid(STOCK_THINK_MS),
            0,
            "a {word} line of #{unit} not one substep after a think: {t_ms}"
        );
        assert_ne!(
            t_ms.rem_euclid(STOCK_THINK_MS),
            think,
            "on the think's own ms"
        );
        stamped += 1;
    }
    assert!(stamped >= 4, "the minds stamped {stamped} actions");
    let told = |unit: ObjId, order: &str, refusal: Refusal| {
        seen.iter().any(|seen| {
            seen.unit == unit
                && seen.view.notices.iter().any(|notice| {
                    matches!(notice, FrameNotice::Refused { order: o, refusal: r, .. }
                        if o == order && *r == refusal)
                })
        })
    };
    assert!(
        told(RED, "emcon", Refusal::NoJammer),
        "the armed mind's pod"
    );
    assert!(
        told(RED2, "emcon", Refusal::NoJammer),
        "the unarmed mind's pod"
    );
    assert!(
        told(RED2, "fire", Refusal::Channel),
        "the unarmed mind's gun"
    );
    assert!(
        seen.iter()
            .any(|seen| seen.view.notices.contains(&FrameNotice::PowerOnline)),
        "a pilot's notice reaches the mind's buffer"
    );
    // The goal is within the motor's arrival window of (6, 6): the drive
    // ends there and says so.
    assert!(
        seen.iter().any(|seen| seen.unit == RED
            && seen.view.cell == Some((6, 6))
            && seen.view.notices.contains(&FrameNotice::Drive {
                outcome: tacsim::arena::DriveOutcome::Arrived
            })),
        "the armed mind drove to its goal and was told it arrived"
    );

    for drive in [
        Drive::Every(1_000),
        Drive::Every(500),
        Drive::Task(5_000),
        Drive::Task(1_000),
        Drive::Task(500),
    ] {
        let (other, other_seen) = two_minds(drive);
        assert_eq!(
            other.events, first.events,
            "the event list diverged at {drive:?}"
        );
        assert_eq!(
            other.hashes, first.hashes,
            "the hash sequence diverged at {drive:?}"
        );
        assert_eq!(other_seen, seen, "a think's job diverged at {drive:?}");
        assert_ne!(other.pulses, first.pulses, "{drive:?} ran the same pulses");
    }
}

/// **A think reads the substep's end and lands one substep later** (doc 17 §6,
/// D-UM-20, D-UM-21). Each job's `t_ms` is its hull's lattice millisecond —
/// the stagger plus a whole number of thinks — and its view's `t_ms` is the
/// end of the substep that millisecond fell in (`0 <= view − job < substep`),
/// the clock every contact age on the view is read on: a plot's age counts
/// back to one of the hull's own passes. The armed mind's `startup` line is at
/// its first think plus one substep. And a `Goal` landed runs the motor once
/// at the landing: the heading the motor orders is on the hull at the end of
/// the landing's substep, and the motor's own tick is on the heap after it.
#[test]
fn m1a5_a_think_sees_the_substeps_end_and_lands_one_substep_later() {
    let (run, seen) = two_minds(Drive::Every(1_000));
    let pass_ms = i64::from(SensorsConfig::default().pass_ms);
    let mut aged = 0_usize;
    for job in &seen {
        assert_eq!(
            job.t_ms,
            think_at(job.unit, i64::try_from(job.think_seq).unwrap_or(0)),
            "#{} think {} off its lattice",
            job.unit.0,
            job.think_seq
        );
        let lag = job.view.t_ms - job.t_ms;
        assert!(
            (0..SUBSTEP).contains(&lag),
            "a view at {} for a think at {}",
            job.view.t_ms,
            job.t_ms
        );
        assert_eq!(job.view.t_ms.rem_euclid(SUBSTEP), 0, "a substep's end");
        for contact in &job.view.contacts {
            let seen_at = job.view.t_ms - contact.age_ms;
            assert!(contact.age_ms >= 0);
            assert_eq!(
                (seen_at - stagger_ms(SEED, job.unit, pass_ms)).rem_euclid(pass_ms),
                0,
                "#{}'s plot of {} is not stamped at one of its passes",
                job.unit.0,
                contact.id
            );
            aged += 1;
        }
    }
    assert!(aged > 0, "the minds held something");
    let first_think = seen
        .iter()
        .find(|job| job.unit == RED && job.think_seq == 0)
        .map(|job| job.t_ms);
    let startup = run.events.iter().find_map(|(t_ms, json, _)| {
        let (word, unit) = kind_and_unit(json);
        (word == "startup" && unit == Some(RED.0)).then_some(*t_ms)
    });
    assert_eq!(startup, first_think.map(|t| t + SUBSTEP));

    // The goal's landing runs the motor once.
    let mut rig = Rig::new(arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(6, 8),
            Vec::new(),
            Controller::Mind,
        )],
        &[],
    ));
    rig.arena.mind_runner = ScriptedMind::new(&[(
        RED,
        vec![
            says(0, vec![IntentKind::Startup]),
            says(1, vec![IntentKind::Goal(MotorGoal::Cell { x: 12, y: 8 })]),
        ],
    )]);
    let think = think_at(RED, 1);
    let land = think + SUBSTEP;
    let _ = rig.until(land, |arena| arena.now_ms >= think);
    assert!(pending(&rig.arena, RED, 1), "think 1 is in flight");
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.desired_heading_mdeg),
        Some(0),
        "facing north until the goal lands"
    );
    let _ = rig.until(land + SUBSTEP, |arena| arena.now_ms >= land);
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.desired_heading_mdeg),
        Some(90_000),
        "the motor's heading, due east, on the hull at the landing's substep"
    );
    assert_eq!(
        rig.arena.motors.row(RED).and_then(|motor| motor.goal),
        Some((12, 8)),
        "the goal is on the motor row"
    );
    assert!(
        rig.arena
            .due
            .iter()
            .any(|event| event.kind == DueKind::MotorTick { unit: RED } && event.t_ms > land),
        "the motor's own tick is on the heap after the landing"
    );
}

// ---------------------------------------------------------------------------
// (3) epochs and a reclaim between a think and its landing
// ---------------------------------------------------------------------------

/// The armed mind and a seated blue it can see: think 0 starts it, think 2
/// locks, think 4 turns it, sets a speed and fires — stepped until think 4 is
/// in flight.
fn epoch_rig() -> Rig {
    let mut rig = Rig::new(arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                Controller::Mind,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            ),
            tank(
                BLUE2,
                Team::Blue,
                centre(14, 1),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[BLUE],
    ));
    rig.arena.mind_runner = ScriptedMind::new(&[(
        RED,
        vec![
            says(0, vec![IntentKind::Startup]),
            says(
                2,
                vec![IntentKind::Lock {
                    contact: draw_contact_id(SEED, RED, BLUE),
                }],
            ),
            says(
                4,
                vec![
                    IntentKind::Heading { mdeg: 90_000 },
                    IntentKind::Speed { cms: 500 },
                    IntentKind::Fire {
                        weapon: SmolStr::new("cannon"),
                    },
                ],
            ),
        ],
    )]);
    let _ = rig.until(10_000, |arena| pending(arena, RED, 4));
    rig
}

/// A control over the armed mind's hull.
fn control(channel: Channel, epoch: u64, state: ControlState, pilot: Option<ObjId>) -> Control {
    Control {
        unit: RED,
        channel,
        epoch,
        state,
        pilot,
    }
}

/// **An epoch raised between a think and its landing refuses the stale
/// thoughts** (D-UM-13; doc 17 §6). Think 4 is taken in one pulse; in the next,
/// before its landing:
///
/// * a `Control { Linkdead }` on the HELM channel at a higher epoch —
///   `apply_control` raises the epoch for every state and its `Linkdead` arm
///   leaves the controller a mind's — and the landing refuses the heading and
///   the speed `epoch`, each appended to the buffer. The `FIRE` on the
///   unraised fire channel passes the epoch gate and then meets the seat state
///   the same control set: a linkdead hull refuses every discrete order, a
///   mind's too, so it is refused `linkdead` (in the buffer) and nothing fires;
/// * the helm epoch raised alone (`EpochTable::raise`, the half of a control
///   that is authority and not seat state): the two helm thoughts are refused
///   `epoch` as before, and the `FIRE` lands — one `fired` line at the
///   landing's millisecond, and no refusal of it.
#[test]
fn m1a5_an_epoch_raised_between_think_and_landing_refuses_the_stale_thoughts() {
    let land = think_at(RED, 4) + SUBSTEP;
    let mut rig = epoch_rig();
    let raised = rig.arena.epochs.current(RED, Channel::Helm) + 1;
    let _ = rig.pulse(
        &[control(Channel::Helm, raised, ControlState::Linkdead, None)],
        &[],
    );
    assert!(rig.arena.now_ms >= land, "the landing's substep ran");
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.controller),
        Some(Controller::Mind),
        "a linkdead control leaves the mind flying"
    );
    assert_eq!(
        refusals(&rig.arena, RED),
        vec![
            ("heading".to_owned(), Refusal::Epoch),
            ("speed".to_owned(), Refusal::Epoch),
            ("fire".to_owned(), Refusal::Linkdead),
        ]
    );
    assert!(rig.lines("fired", RED).is_empty(), "nothing fired");

    let mut rig = epoch_rig();
    let raised = rig.arena.epochs.current(RED, Channel::Helm) + 1;
    rig.arena.epochs.raise(RED, Channel::Helm, raised);
    let _ = rig.pulse(&[], &[]);
    assert!(rig.arena.now_ms >= land, "the landing's substep ran");
    assert_eq!(
        refusals(&rig.arena, RED),
        vec![
            ("heading".to_owned(), Refusal::Epoch),
            ("speed".to_owned(), Refusal::Epoch),
        ]
    );
    assert_eq!(
        rig.lines("fired", RED),
        vec![land],
        "the fire channel was not raised: the shot lands at the landing"
    );
}

/// **A reclaimed seat lands nothing** (D-UM-13; `apply_control`'s `Live` arm).
/// A `Control { Live }` naming a pilot between think 4 and its landing hands
/// the hull to the pilot, which takes the mind's row, its scratch and its two
/// due kinds with it: the landing never comes, nothing of the think is applied
/// or journalled, and no refusal is staged.
#[test]
fn m1a5_a_reclaimed_seat_lands_nothing() {
    let land = think_at(RED, 4) + SUBSTEP;
    let mut rig = epoch_rig();
    let raised = rig.arena.epochs.current(RED, Channel::Helm) + 1;
    let out = rig.pulse(
        &[control(
            Channel::Helm,
            raised,
            ControlState::Live,
            Some(ObjId(2_000)),
        )],
        &[],
    );
    assert!(rig.arena.now_ms >= land, "the landing's millisecond passed");
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.controller),
        Some(Controller::Pilot)
    );
    assert!(!rig.arena.minds.contains_key(&RED), "the mind's row went");
    assert!(
        !rig.arena.mind_landings.contains_key(&RED),
        "and its scratch"
    );
    assert!(
        !rig.arena.due.iter().any(|event| event.kind.is_mind()),
        "and its thinks and landings"
    );
    assert_ne!(
        rig.arena.unit(RED).map(|unit| unit.desired_heading_mdeg),
        Some(90_000),
        "the stale heading never landed"
    );
    assert!(rig.lines("fired", RED).is_empty(), "nothing fired");
    assert!(
        !out.frames.iter().any(|(_, frame)| frame
            .notices
            .iter()
            .any(|notice| matches!(notice, FrameNotice::Refused { .. }))),
        "no refusal told to anybody"
    );
}

// ---------------------------------------------------------------------------
// (4) what a mind may not do
// ---------------------------------------------------------------------------

fn order(squad: SquadId, objective: Objective, issuer: Option<ObjId>, ttl_ms: u32) -> Order {
    Order {
        squad,
        objective,
        target: None,
        horizon_ms: ttl_ms,
        constraints: Constraints::default(),
        ttl_ms,
        issued_ms: 0,
        issuer,
        spot: None,
    }
}

const RED_ONE: SquadId = SquadId {
    team: Team::Red,
    index: 1,
};

/// **A mind is refused `ORDER`, `RELEASE` and `PLACE` with `channel`, and an
/// ungranted station the same** (D-UM-11, D-UM-52's second wall, D-UM-47; doc
/// 17 §22's italic line). The armed mind's think 4 issues an order, a release,
/// a place and two `FIRE`s: the first three are refused `channel` into its
/// buffer and none journals a line; the first shot fires and the second is
/// refused `cycling` — a resolver refusal `Arena::order` used to drop. The
/// unarmed mind's `FIRE` is refused `channel`, not `no_weapon`: its hull has
/// no weapons station. Its `LOCK` is not refused `channel`: the sensors
/// station is there.
#[test]
fn m1a5_a_mind_is_refused_order_release_and_place_with_channel_and_an_ungranted_station_the_same() {
    let mut rig = Rig::new(arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                Controller::Mind,
            ),
            tank(RED2, Team::Red, centre(6, 10), Vec::new(), Controller::Mind),
            tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            ),
            tank(
                BLUE2,
                Team::Blue,
                centre(14, 1),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[BLUE],
    ));
    let gun = SmolStr::new("cannon");
    rig.arena.mind_runner = ScriptedMind::new(&[
        (
            RED,
            vec![
                says(0, vec![IntentKind::Startup]),
                says(
                    2,
                    vec![IntentKind::Lock {
                        contact: draw_contact_id(SEED, RED, BLUE),
                    }],
                ),
                says(
                    4,
                    vec![
                        IntentKind::Order(Box::new(order(RED_ONE, Objective::Hold, None, 30_000))),
                        IntentKind::ReleaseOrder {
                            squad: RED_ONE,
                            issuer: None,
                        },
                        IntentKind::Place { x: 3, y: 3 },
                        IntentKind::Fire {
                            weapon: gun.clone(),
                        },
                        IntentKind::Fire {
                            weapon: gun.clone(),
                        },
                    ],
                ),
            ],
        ),
        (
            RED2,
            vec![
                says(0, vec![IntentKind::Startup]),
                says(
                    2,
                    vec![IntentKind::Lock {
                        contact: draw_contact_id(SEED, RED2, BLUE),
                    }],
                ),
                says(4, vec![IntentKind::Fire { weapon: gun }]),
            ],
        ),
    ]);
    assert_eq!(
        rig.arena.unit(RED2).map(|unit| unit.weapons.is_empty()),
        Some(true),
        "the precondition: no weapons station"
    );
    let land = think_at(RED, 4).max(think_at(RED2, 4)) + SUBSTEP;
    let _ = rig.until(land + SUBSTEP, |arena| arena.now_ms >= land);
    assert_eq!(
        refusals(&rig.arena, RED),
        vec![
            ("order".to_owned(), Refusal::Channel),
            ("release".to_owned(), Refusal::Channel),
            ("place".to_owned(), Refusal::Channel),
            ("fire".to_owned(), Refusal::Cycling),
        ]
    );
    assert_eq!(rig.lines("fired", RED).len(), 1, "the first shot fired");
    let journal = rig.journal();
    for word in ["ordered", "order_released", "placed", "place_refused"] {
        assert!(
            !journal.iter().any(|event| event.kind.as_str() == word),
            "a mind's refused act journalled {word}"
        );
    }
    let unarmed = refusals(&rig.arena, RED2);
    assert!(
        unarmed.contains(&("fire".to_owned(), Refusal::Channel)),
        "{unarmed:?}"
    );
    assert!(
        !unarmed.contains(&("lock".to_owned(), Refusal::Channel)),
        "the sensors station grants the lock: {unarmed:?}"
    );
}

// ---------------------------------------------------------------------------
// (5) D-UM-9: a think is atomic
// ---------------------------------------------------------------------------

/// **A think of seventeen intents lands nothing and keeps its memory** (D-UM-9).
/// Think 0 lands the memory `[1]`, a start-up and a pod switch the hull has no
/// pod for; think 1 answers `MIND_INTENTS_MAX + 1` speeds and the memory
/// `[9, 9]`: nothing lands (no landing on the heap or the scratch, the ordered
/// speed untouched), the memory is still `[1]`, `pending` is cleared,
/// `mind_overflows` counts one and think 2 is on the heap one think later — and
/// is handed `[1]`. **The think is atomic for the buffer too**: what think 1's
/// view drained — landing 0's `no_jammer` refusal and the `Power online.` the
/// start-up staged — is back in the buffer after the fault, and think 2's view
/// carries it (D-UM-8: *a refusal is never silent*, not even past a fault that
/// threw away the memory that had recorded it).
#[test]
fn m1a5_a_think_of_seventeen_intents_lands_nothing_and_keeps_its_memory() {
    let mut rig = Rig::new(arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(6, 8),
            Vec::new(),
            Controller::Mind,
        )],
        &[],
    ));
    let mind = ScriptedMind::new(&[(
        RED,
        vec![
            Says {
                think_seq: 0,
                intents: vec![IntentKind::Startup, IntentKind::Emcon { on: true }],
                memory: Some(vec![1]),
            },
            Says {
                think_seq: 1,
                intents: vec![IntentKind::Speed { cms: 300 }; MIND_INTENTS_MAX + 1],
                memory: Some(vec![9, 9]),
            },
        ],
    )]);
    rig.arena.mind_runner = mind.clone();
    let think = think_at(RED, 1);
    let counters = rig.until(think + SUBSTEP, |arena| arena.now_ms >= think);
    let state = rig.arena.minds.get(&RED).cloned().unwrap_or_default();
    assert_eq!(state.memory, vec![1], "the old memory is kept");
    let drained = mind
        .seen()
        .into_iter()
        .find(|seen| seen.think_seq == 1)
        .map(|seen| seen.view.notices)
        .unwrap_or_default();
    assert!(
        drained.contains(&FrameNotice::PowerOnline)
            && drained.iter().any(|notice| matches!(
                notice,
                FrameNotice::Refused { order, refusal: Refusal::NoJammer, .. } if order == "emcon"
            )),
        "the precondition: think 1 was handed landing 0's refusal and the start-up's notice: {drained:?}"
    );
    assert_eq!(
        state.notices, drained,
        "what the faulted think drained is back in the buffer"
    );
    assert_eq!(state.pending, None);
    assert_eq!(state.think_seq, 2);
    assert_eq!(counters.mind_overflows, 1);
    assert!(!rig.arena.mind_landings.contains_key(&RED));
    assert!(
        !rig.arena
            .due
            .iter()
            .any(|event| matches!(event.kind, DueKind::MindLand { unit, .. } if unit == RED)),
        "no landing scheduled"
    );
    assert!(
        rig.arena
            .due
            .iter()
            .any(|event| event.t_ms == think + STOCK_THINK_MS
                && event.kind
                    == DueKind::MindThink {
                        unit: RED,
                        think_seq: 2
                    }),
        "the next think is on the heap"
    );
    let _ = rig.until(think + 2 * STOCK_THINK_MS, |arena| {
        arena.now_ms >= think + STOCK_THINK_MS
    });
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.desired_speed_cms),
        Some(0)
    );
    let handed = mind.seen().into_iter().find(|seen| seen.think_seq == 2);
    assert_eq!(
        handed.as_ref().map(|seen| seen.memory.clone()),
        Some(vec![1])
    );
    assert!(
        handed.is_some_and(|seen| seen.view.notices.starts_with(&drained)),
        "think 2 reads what think 1 was handed"
    );
}

/// **A think of exactly sixteen intents lands** (D-UM-9's bound is
/// inclusive: *at most 16*). Think 0 starts the hull; think 1 answers
/// `MIND_INTENTS_MAX` speeds, 100 to 250 cm/s, and the memory `[5, 5]`: it is
/// in flight with a landing on the heap one substep on, nothing counted
/// `mind_overflows`, and at the landing the memory is `[5, 5]` and the hull is
/// ordered the last speed.
#[test]
fn m1a5_a_think_of_sixteen_intents_lands_whole() {
    let mut rig = Rig::new(arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(6, 8),
            Vec::new(),
            Controller::Mind,
        )],
        &[],
    ));
    let speeds: Vec<IntentKind> = (0..MIND_INTENTS_MAX)
        .map(|n| IntentKind::Speed {
            cms: 100 + 10 * i32::try_from(n).unwrap_or(0),
        })
        .collect();
    assert_eq!(speeds.len(), 16);
    rig.arena.mind_runner = ScriptedMind::new(&[(
        RED,
        vec![
            says(0, vec![IntentKind::Startup]),
            Says {
                think_seq: 1,
                intents: speeds,
                memory: Some(vec![5, 5]),
            },
        ],
    )]);
    let think = think_at(RED, 1);
    let land = think + SUBSTEP;
    let counters = rig.until(land, |arena| arena.now_ms >= think);
    assert_eq!(counters.mind_overflows, 0, "sixteen is not a fault");
    assert!(pending(&rig.arena, RED, 1), "think 1 is in flight");
    assert!(
        rig.arena.due.iter().any(|event| event.t_ms == land
            && event.kind
                == DueKind::MindLand {
                    unit: RED,
                    think_seq: 1
                }),
        "its landing is on the heap"
    );
    let counters = rig.until(land + SUBSTEP, |arena| arena.now_ms >= land);
    assert_eq!(counters.mind_overflows, 0);
    let state = rig.arena.minds.get(&RED).cloned().unwrap_or_default();
    assert_eq!(state.memory, vec![5, 5], "the new memory landed");
    assert_eq!(state.pending, None);
    assert!(refusals(&rig.arena, RED).is_empty(), "nothing refused");
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.desired_speed_cms),
        Some(250),
        "the sixteenth speed is the one ordered"
    );
}

/// **A mind's intents are never counted on the seat's window** (D-UM-14).
/// The armed mind's think 2 answers ten `LOCK`s — two past the seat's
/// `intents.max_pending_per_unit` of eight a second — and six `DRIVE`s, the
/// continuous kind a seat's window does count (17b cut M1a.4): none is
/// refused `queue_full`, and the hull's window (`orders_window_s`,
/// `orders_in_window`) is exactly what it was before the landing.
#[test]
fn m1a5_a_minds_intents_are_never_counted_on_the_seat_window() {
    let mut rig = Rig::new(arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                Controller::Mind,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            ),
            tank(
                BLUE2,
                Team::Blue,
                centre(14, 1),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[BLUE],
    ));
    let max_pending = rig.arena.arena_cfg.intents.max_pending_per_unit;
    assert_eq!(max_pending, 8, "the precondition: the stock seat rate");
    let blue = draw_contact_id(SEED, RED, BLUE);
    let mut burst: Vec<IntentKind> = vec![IntentKind::Lock { contact: blue }; 10];
    burst.extend(vec![
        IntentKind::Goal(MotorGoal::Cell { x: 6, y: 5 });
        MIND_INTENTS_MAX - 10
    ]);
    assert_eq!(burst.len(), MIND_INTENTS_MAX);
    rig.arena.mind_runner = ScriptedMind::new(&[(
        RED,
        vec![says(0, vec![IntentKind::Startup]), says(2, burst)],
    )]);
    let land = think_at(RED, 2) + SUBSTEP;
    let _ = rig.until(land, |arena| pending(arena, RED, 2));
    let window = |arena: &Arena| {
        arena
            .unit(RED)
            .map(|unit| (unit.orders_window_s, unit.orders_in_window))
    };
    let before = window(&rig.arena);
    let _ = rig.until(land + SUBSTEP, |arena| arena.now_ms >= land);
    assert!(!pending(&rig.arena, RED, 2), "think 2 landed");
    let refused = refusals(&rig.arena, RED);
    assert!(
        !refused
            .iter()
            .any(|(_, refusal)| *refusal == Refusal::QueueFull),
        "a mind spent a seat's rate: {refused:?}"
    );
    assert_eq!(window(&rig.arena), before, "the seat's window is untouched");
    assert!(
        rig.arena
            .motors
            .row(RED)
            .is_some_and(|motor| motor.goal.is_some()),
        "the drives landed"
    );
}

// ---------------------------------------------------------------------------
// (6) the notice buffer
// ---------------------------------------------------------------------------

/// **The notice buffer is bounded, hashed, snapshotted and drained per think**
/// (D-UM-8). Seventeen notices keep the newest sixteen; one notice more moves
/// the state hash, and so do two `Miss` notices that differ only in their
/// reason; a snapshot carries the buffer back; the next think's view carries
/// it and the buffer is empty after.
#[test]
fn m1a5_the_notice_buffer_is_bounded_hashed_snapshotted_and_drained_per_think() {
    let mut rig = Rig::new(arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(6, 8),
            Vec::new(),
            Controller::Mind,
        )],
        &[],
    ));
    let mind = ScriptedMind::new(&[]);
    rig.arena.mind_runner = mind.clone();
    let miss = |reason: &str| FrameNotice::Miss {
        contact: None,
        reason: SmolStr::new(reason),
    };
    let Some(state) = rig.arena.minds.get_mut(&RED) else {
        panic!("a mind's row");
    };
    for n in 0..=MIND_NOTICES {
        state.push_notice(FrameNotice::Emcon {
            on: true,
            radius_m: u32::try_from(n).unwrap_or(0),
        });
    }
    assert_eq!(state.notices.len(), MIND_NOTICES);
    assert_eq!(
        state.notices.first(),
        Some(&FrameNotice::Emcon {
            on: true,
            radius_m: 1
        }),
        "the oldest went"
    );
    let hash = tacsim::arena::hash::state_hash;
    let with = |arena: &Arena, notice: FrameNotice| {
        let mut probe = arena.clone();
        if let Some(state) = probe.minds.get_mut(&RED) {
            state.notices.truncate(1);
            state.push_notice(notice);
        }
        hash(&probe)
    };
    let mut fewer = rig.arena.clone();
    if let Some(state) = fewer.minds.get_mut(&RED) {
        state.notices.pop();
    }
    assert_ne!(
        hash(&fewer),
        hash(&rig.arena),
        "one notice more moves the hash"
    );
    assert_ne!(
        with(&rig.arena, miss("moved")),
        with(&rig.arena, miss("cover")),
        "a miss's reason is folded"
    );

    let snapshot = Snapshot::encode(&rig.arena).expect("the arena encodes");
    let decoded = Snapshot::decode(&snapshot.document, rig.arena.definition.clone())
        .expect("the document decodes");
    assert_eq!(
        decoded.minds.get(&RED),
        rig.arena.minds.get(&RED),
        "carried"
    );
    assert_eq!(hash(&decoded), hash(&rig.arena));

    let buffered = rig
        .arena
        .minds
        .get(&RED)
        .map(|state| state.notices.clone())
        .unwrap_or_default();
    let think = think_at(RED, 0);
    let _ = rig.until(think + SUBSTEP, |arena| arena.now_ms >= think);
    let first = mind.seen().into_iter().next().map(|seen| seen.view.notices);
    assert_eq!(first, Some(buffered), "the view carried the whole buffer");
    assert!(
        rig.arena
            .minds
            .get(&RED)
            .is_some_and(|state| state.notices.is_empty()),
        "and the buffer is empty after"
    );
}

// ---------------------------------------------------------------------------
// (7) D-UM-20 delivered: mind kinds run last at their millisecond
// ---------------------------------------------------------------------------

/// **A millisecond's mind kinds run last, in unit-id order** (D-UM-20,
/// D-UM-21). On a heap cleared of thinks, passes and shares:
///
/// * a `SensorPass` of the armed mind scheduled **after** its `MindThink` at
///   the same millisecond (a higher `seq`, so it pops second) runs **before**
///   it: the think's view holds the contact the pass detected, where a think
///   run in `seq` order would hold nothing;
/// * two minds whose thinks share that millisecond — the higher id scheduled
///   first, so its `seq` is lower — think in unit-id order, and with their
///   landings re-scheduled the other way round they land in unit-id order:
///   the lower id's `startup` line is journalled first;
/// * a `MindLand` and a `MindThink` of one hull at one millisecond — the think
///   scheduled first — land, then think: the think is handed the memory the
///   landing wrote.
#[test]
fn m1a5_mind_kinds_run_last_at_their_millisecond_in_unit_id_order() {
    let mut rig = Rig::new(arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                Controller::Mind,
            ),
            tank(RED2, Team::Red, centre(6, 10), Vec::new(), Controller::Mind),
            tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[BLUE],
    ));
    let mind = ScriptedMind::new(&[
        (
            RED,
            vec![
                Says {
                    think_seq: 0,
                    intents: vec![IntentKind::Startup],
                    memory: Some(vec![7]),
                },
                says(1, Vec::new()),
            ],
        ),
        (RED2, vec![says(0, vec![IntentKind::Startup])]),
    ]);
    rig.arena.mind_runner = mind.clone();
    let _ = rig.arena.due.retain(|kind| {
        !matches!(
            kind,
            DueKind::MindThink { .. } | DueKind::SensorPass { .. } | DueKind::Share { .. }
        )
    });
    let at = 1_000_i64;
    rig.arena.due.schedule(
        at,
        DueKind::MindThink {
            unit: RED2,
            think_seq: 0,
        },
    );
    rig.arena.due.schedule(
        at,
        DueKind::MindThink {
            unit: RED,
            think_seq: 0,
        },
    );
    rig.arena.due.schedule(
        at,
        DueKind::SensorPass {
            unit: RED,
            pass_seq: 1,
        },
    );
    assert!(
        rig.arena.contacts.of(RED).next().is_none(),
        "nothing held yet"
    );
    let _ = rig.until(at + SUBSTEP, |arena| arena.now_ms >= at);
    let seen = mind.seen();
    assert_eq!(
        seen.iter().map(|seen| seen.unit).collect::<Vec<_>>(),
        vec![RED, RED2],
        "thought in unit-id order, whatever their seq"
    );
    let blue = draw_contact_id(SEED, RED, BLUE);
    assert!(
        seen.first().is_some_and(|seen| seen
            .view
            .contacts
            .iter()
            .any(|contact| contact.id == blue)),
        "the pass scheduled after the think ran before it"
    );
    // The two landings the other way round, the higher id first, and the
    // armed mind's next think ahead of its own landing.
    let land = at + SUBSTEP;
    let _ = rig.arena.due.retain(|kind| !kind.is_mind());
    rig.arena.due.schedule(
        land,
        DueKind::MindThink {
            unit: RED,
            think_seq: 1,
        },
    );
    rig.arena.due.schedule(
        land,
        DueKind::MindLand {
            unit: RED2,
            think_seq: 0,
        },
    );
    rig.arena.due.schedule(
        land,
        DueKind::MindLand {
            unit: RED,
            think_seq: 0,
        },
    );
    let _ = rig.until(land + SUBSTEP, |arena| arena.now_ms >= land);
    let order: Vec<u32> = rig
        .journal()
        .iter()
        .filter_map(|event| {
            let (word, unit) =
                kind_and_unit(&serde_json::to_string(&event.kind).unwrap_or_default());
            if word == "startup" { unit } else { None }
        })
        .collect();
    assert_eq!(order, vec![RED.0, RED2.0], "landed in unit-id order");
    let second = mind
        .seen()
        .into_iter()
        .find(|seen| seen.unit == RED && seen.think_seq == 1)
        .map(|seen| seen.memory);
    assert_eq!(second, Some(vec![7]), "landed, then thought");
}

// ---------------------------------------------------------------------------
// (9) an order to a mind
// ---------------------------------------------------------------------------

/// **An order to a mind is re-keyed to its own held contact, or none**
/// (D-UM-15 as decided). A seated pilot's `ORDER … ENGAGE BA` — the hint in
/// the pilot's own scope, `issuer` the pilot's tank — lands on the mind as
/// `Engage { target }` naming **the mind's own row** for the same target once
/// it holds one, and `None` before; `SUPPORT` of the mind's teammate likewise.
/// The `ordered` line carries the issuer unchanged and the frame reads the
/// held order back. `RELEASE` clears it and journals `order_released`; an
/// order whose deadline comes is cleared at the next landing with one
/// `order_expired` line.
#[test]
fn m1a5_an_order_to_a_mind_is_re_keyed_to_its_own_held_contact_or_none() {
    let mut rig = Rig::new(arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                Controller::Mind,
            ),
            tank(
                RED2,
                Team::Red,
                centre(6, 10),
                vec![cannon()],
                Controller::Pilot,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[RED2, BLUE],
    ));
    rig.arena.mind_runner = ScriptedMind::new(&[]);
    let hint = draw_contact_id(SEED, RED2, BLUE);
    let engage = |arena: &Arena, ttl_ms: u32| {
        intent(
            arena,
            RED,
            IntentKind::Order(Box::new(order(
                RED_ONE,
                Objective::Engage {
                    target_hint: Some(hint),
                },
                Some(RED2),
                ttl_ms,
            ))),
            1,
        )
    };
    let held = |arena: &Arena| arena.minds.get(&RED).and_then(|state| state.order.clone());
    let support = |arena: &Arena| {
        intent(
            arena,
            RED,
            IntentKind::Order(Box::new(order(
                RED_ONE,
                Objective::Support { of: RED2 },
                Some(RED2),
                60_000,
            ))),
            1,
        )
    };
    // Before any pass: neither tank holds anything.
    let first = support(&rig.arena);
    let _ = rig.pulse(&[], &[first]);
    assert_eq!(
        held(&rig.arena).map(|order| order.objective),
        Some(MindObjective::Support { of: None }),
        "the teammate not held: no row"
    );
    let first = engage(&rig.arena, 60_000);
    let _ = rig.pulse(&[], &[first]);
    assert_eq!(
        held(&rig.arena).map(|order| order.objective),
        Some(MindObjective::Engage { target: None }),
        "nothing held: no row"
    );
    assert!(
        rig.arena.contacts.of(RED).next().is_none(),
        "the precondition"
    );
    // After the passes: both hold blue.
    let _ = rig.until(5_000, |arena| {
        arena.contacts.id_of(RED, BLUE).is_some() && arena.contacts.id_of(RED2, BLUE).is_some()
    });
    let again = engage(&rig.arena, 60_000);
    let _ = rig.pulse(&[], &[again]);
    let own = rig.arena.contacts.id_of(RED, BLUE);
    assert!(own.is_some());
    assert_ne!(own, Some(hint), "the mind's own id, not the issuer's");
    assert_eq!(
        held(&rig.arena).map(|order| order.objective),
        Some(MindObjective::Engage { target: own }),
        "the mind's own row for the target"
    );
    assert_eq!(
        rig.arena.frame_order(RED),
        held(&rig.arena),
        "the frame reads it back"
    );
    assert!(
        rig.journal().iter().any(|event| {
            let json = serde_json::to_value(&event.kind).unwrap_or_default();
            json.get("kind") == Some(&serde_json::json!("ordered"))
                && json.get("issuer") == Some(&serde_json::json!(RED2.0))
                && json.get("unit") == Some(&serde_json::json!(RED.0))
        }),
        "the ordered line carries the issuer"
    );
    // Support of the teammate, now held: the mind's own row for it.
    let again = support(&rig.arena);
    let _ = rig.pulse(&[], &[again]);
    let charge = rig.arena.contacts.id_of(RED, RED2);
    assert!(charge.is_some());
    assert_eq!(
        held(&rig.arena).map(|order| order.objective),
        Some(MindObjective::Support { of: charge })
    );
    // The release.
    let release = intent(
        &rig.arena,
        RED,
        IntentKind::ReleaseOrder {
            squad: RED_ONE,
            issuer: Some(RED2),
        },
        1,
    );
    let out = rig.pulse(&[], &[release]);
    assert_eq!(held(&rig.arena), None, "released");
    assert_eq!(out.reflex.orders_released, 1);
    assert_eq!(
        rig.journal()
            .iter()
            .filter(|event| event.kind.as_str() == "order_released")
            .count(),
        1
    );
    // An order that runs out: cleared at the next landing, with one line.
    let short = engage(&rig.arena, 1_000);
    let _ = rig.pulse(&[], &[short]);
    let deadline = held(&rig.arena).map_or(0, |order| order.expires_ms);
    assert!(deadline > 0);
    let _ = rig.until(deadline + 3 * STOCK_THINK_MS, |arena| held(arena).is_none());
    let expired: Vec<i64> = rig
        .journal()
        .iter()
        .filter(|event| event.kind.as_str() == "order_expired")
        .map(|event| event.t_ms)
        .collect();
    assert_eq!(expired.len(), 1, "{expired:?}");
    assert!(expired.iter().all(|t_ms| *t_ms >= deadline));
    assert_eq!(
        expired
            .first()
            .map(|t_ms| (t_ms - SUBSTEP - think_at(RED, 0)).rem_euclid(STOCK_THINK_MS)),
        Some(0),
        "at a landing"
    );
}

// ---------------------------------------------------------------------------
// (10) the production hash
// ---------------------------------------------------------------------------

/// **The production hash does not move** (the M1a plan's head; 17b cut M1a.1's
/// two pins, re-asserted by this cut with the same constants). The first: the
/// (j) field of `tests/reflex.rs` — two reflex machines, one armed, a seated
/// tank — at a fixed 1,000 ms cadence for 30 s, rebuilt here from the same
/// fixtures. The second: four stock machines on the proving ground for a
/// minute ([`support::shipped_arena`]). No unit of either is a mind, so the
/// fold's new section, the three new carried variants and the due loop's mind
/// rule add nothing — and they add nothing.
#[test]
fn m1a5_the_production_hash_does_not_move() {
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                Controller::Reflex,
            ),
            tank(
                RED2,
                Team::Red,
                centre(6, 10),
                Vec::new(),
                Controller::Reflex,
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[BLUE],
    );
    let run = run(&mut arena, Drive::Every(1_000), 30_000);
    assert_eq!(run.hashes.len(), 120, "the substeps of 30 s at 250 ms");
    assert_eq!(run.hashes.first().copied(), Some(0x9da6_3882_e319_aec9));
    assert_eq!(run.hashes.last().copied(), Some(0xdb4f_9096_c79f_b530));
    assert_eq!(run.counters.thinks, 0, "no mind thought");

    let world = support::booted_world();
    let mut duel = support::shipped_arena(
        &world,
        &[
            &[
                support::Muster::reflex(Team::Red, 1),
                support::Muster::reflex(Team::Red, 2),
            ],
            &[
                support::Muster::reflex(Team::Blue, 1),
                support::Muster::reflex(Team::Blue, 2),
            ],
        ],
    );
    let _ = support::run_until(&mut duel, 60_000, 1_000, |_, _| {});
    assert_eq!(
        tacsim::arena::hash::state_hash(&duel),
        0xd3e7_ee71_474b_6738,
        "the shipped duel's hash moved"
    );
    assert!(duel.minds.is_empty());
}

/// **A mind mustered on the proving ground** (17b cut M1a.5): the rig's
/// [`support::Muster::mind`] fields a mind exactly as a reflex pod is fielded,
/// and `Arena::new` lays its row down and schedules its first think; with the
/// default runner (`NoMind`) it thinks once an arena second and orders
/// nothing.
#[test]
fn m1a5_a_mind_mustered_on_the_shipped_ground_thinks_on_its_grid() {
    let world = support::booted_world();
    let mut arena = support::shipped_arena(
        &world,
        &[
            &[support::Muster::mind(Team::Red, 1)],
            &[support::Muster::piloted(Team::Blue, 1)],
        ],
    );
    let red = support::tank(&world, Team::Red, 1);
    assert_eq!(
        arena.unit(red).map(|unit| unit.controller),
        Some(Controller::Mind)
    );
    assert!(arena.minds.contains_key(&red));
    assert!(arena.reflex.unit(red).is_none(), "no reflex row");
    let mut thinks = 0_u32;
    let _ = support::run_until(&mut arena, 10_000, 1_000, |_, out| {
        thinks = thinks.saturating_add(out.reflex.thinks);
    });
    assert_eq!(thinks, 10, "one think an arena second");
    assert_eq!(arena.minds.get(&red).map(|state| state.think_seq), Some(10));
    assert_eq!(
        arena.unit(red).map(|unit| unit.power),
        Some(tacsim::arena::state::Power::Off),
        "a mind that thinks nothing orders nothing"
    );
}

// ---------------------------------------------------------------------------
// (11) every controller transition releases the outgoing rows
// ---------------------------------------------------------------------------

/// What `unit` has left on the arena: a reflex row, a mind row, a landing in
/// scratch, a motor row, and its controllers' due kinds.
type Rows = (bool, bool, bool, bool, Vec<&'static str>);

fn rows_of(arena: &Arena, unit: ObjId) -> Rows {
    let kinds = arena
        .due
        .iter()
        .filter_map(|event| match event.kind {
            DueKind::ReflexTick { unit: on, .. } if on == unit => Some("reflex_tick"),
            DueKind::MindThink { unit: on, .. } if on == unit => Some("mind_think"),
            DueKind::MindLand { unit: on, .. } if on == unit => Some("mind_land"),
            DueKind::MotorTick { unit: on } if on == unit => Some("motor_tick"),
            _ => None,
        })
        .collect();
    (
        arena.reflex.unit(unit).is_some(),
        arena.minds.contains_key(&unit),
        arena.mind_landings.contains_key(&unit),
        arena.motors.row(unit).is_some(),
        kinds,
    )
}

/// **Every controller transition releases the outgoing rows** (17b cut M1a.5;
/// `Arena::set_controller`). Reflex → Mind by `set_controller`; Mind → Reflex
/// by a `Released` control with a think in flight; Reflex → Pilot by a `Live`
/// control; Pilot → Mind by `set_controller` with a pilot's `DRIVE` standing;
/// Mind → Pilot by a `Live` control with a think in flight. After each,
/// nothing of the outgoing controller is left — no reflex row or tick, no mind
/// row, scratch, think or landing, no motor row or motor tick — and the
/// incoming controller's first event is on the heap.
#[test]
fn m1a5_every_controller_transition_releases_the_outgoing_rows() {
    let mut rig = Rig::new(arena_of(
        vec![
            tank(RED, Team::Red, centre(6, 8), Vec::new(), Controller::Reflex),
            tank(
                BLUE,
                Team::Blue,
                centre(14, 14),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[],
    ));
    rig.arena.mind_runner = ScriptedMind::new(&[]);
    let _ = rig.until(3_000, |arena| arena.now_ms >= 2_000);
    assert!(rows_of(&rig.arena, RED).0, "the reflex machine flies it");

    // Reflex -> Mind.
    rig.arena
        .set_controller(RED, Controller::Mind, ReflexState::Withdraw);
    assert_eq!(
        rows_of(&rig.arena, RED),
        (false, true, false, false, vec!["mind_think"])
    );

    // Mind -> Reflex, with a think in flight.
    let _ = rig.until(6_000, |arena| arena.mind_landings.contains_key(&RED));
    let epoch = rig.arena.epochs.current(RED, Channel::Helm) + 1;
    let _ = rig.pulse(
        &[control(Channel::Helm, epoch, ControlState::Released, None)],
        &[],
    );
    assert_eq!(
        rows_of(&rig.arena, RED),
        (true, false, false, true, vec!["reflex_tick"])
    );

    // Reflex -> Pilot.
    let _ = rig.pulse(
        &[control(
            Channel::Helm,
            epoch + 1,
            ControlState::Live,
            Some(ObjId(2_000)),
        )],
        &[],
    );
    assert_eq!(
        rows_of(&rig.arena, RED),
        (false, false, false, false, vec![])
    );

    // Pilot -> Mind, with a pilot's `DRIVE` standing.
    if let Some(unit) = rig.arena.unit_mut(RED) {
        unit.power = tacsim::arena::state::Power::Online;
    }
    let drive = intent(
        &rig.arena,
        RED,
        IntentKind::Goal(MotorGoal::Cell { x: 2, y: 8 }),
        1,
    );
    let _ = rig.pulse(&[], &[drive]);
    let (_, _, _, motor, kinds) = rows_of(&rig.arena, RED);
    assert!(motor && kinds == vec!["motor_tick"], "the pilot is driving");
    rig.arena
        .set_controller(RED, Controller::Mind, ReflexState::Withdraw);
    assert_eq!(
        rows_of(&rig.arena, RED),
        (false, true, false, false, vec!["mind_think"])
    );

    // Mind -> Pilot, with a think in flight.
    let _ = rig.until(20_000, |arena| arena.mind_landings.contains_key(&RED));
    let _ = rig.pulse(
        &[control(
            Channel::Helm,
            epoch + 2,
            ControlState::Live,
            Some(ObjId(2_000)),
        )],
        &[],
    );
    assert_eq!(
        rows_of(&rig.arena, RED),
        (false, false, false, false, vec![])
    );
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.controller),
        Some(Controller::Pilot)
    );
}

/// **A resumed landing finds no scratch and clears its `pending` row** (17b
/// cut M1a.5; `snapshot.rs`' `SNAPSHOT_FORMAT` 5): the mind's row round-trips
/// — `pending`, the memory, a range — the landing waiting in scratch does not,
/// and the decoded arena thinks with the default runner; when its landing
/// comes it finds nothing, clears `pending` and lands nothing (the residual
/// 17b cut M3's recompute closes).
#[test]
fn m1a5_a_resumed_landing_finds_no_scratch_and_clears_its_pending_row() {
    let mut rig = Rig::new(arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(6, 8),
            Vec::new(),
            Controller::Mind,
        )],
        &[],
    ));
    rig.arena.mind_runner = ScriptedMind::new(&[(RED, vec![says(0, vec![IntentKind::Startup])])]);
    let think = think_at(RED, 0);
    let _ = rig.until(think + SUBSTEP, |arena| pending(arena, RED, 0));
    if let Some(state) = rig.arena.minds.get_mut(&RED) {
        state.memory = vec![3, 1, 4];
        state.ranges.insert(ContactId(77), 450);
    }
    let snapshot = Snapshot::encode(&rig.arena).expect("the arena encodes");
    let decoded = Snapshot::decode(&snapshot.document, rig.arena.definition.clone())
        .expect("the document decodes");
    assert_eq!(decoded.minds, rig.arena.minds, "the rows came back");
    assert!(decoded.mind_landings.is_empty(), "the scratch did not");
    assert!(format!("{:?}", decoded.mind_runner).contains("NoMind"));
    // Resumed on the decoded arena's own millisecond as the wall
    // (`Arena::resume_at`), so the rig's next pulse is one substep on.
    let mut resumed = Rig::new(decoded);
    let now = resumed.arena.now_ms;
    let _ = resumed.arena.resume_at(now);
    resumed.wall = Some(now);
    let _ = resumed.until(think + 2 * SUBSTEP, |arena| arena.now_ms >= think + SUBSTEP);
    assert!(
        !pending(&resumed.arena, RED, 0),
        "the landing cleared its row"
    );
    assert!(
        resumed.lines("startup", RED).is_empty(),
        "and landed nothing"
    );
    let state: MindState = resumed.arena.minds.get(&RED).cloned().unwrap_or_default();
    assert_eq!(state.memory, vec![3, 1, 4], "the memory is the carried one");
}

// ---------------------------------------------------------------------------
// The landing's edges: a wreck, the mind's own helm, a refusal's numbers
// ---------------------------------------------------------------------------

/// **A wreck cancels its mind's thinks** (17b cut M1a.5; `damage.rs`'
/// `Arena::destroy_unit`). The mind drives (think 1's `DRIVE`, so a motor tick
/// is on the heap) and has think 2 in flight — its landing in the scratch and
/// on the heap — when the hull is destroyed: no think, landing or motor tick of
/// it is left on the heap, the scratch is gone, the row's `pending` is cleared
/// and the row itself stays (what the mind remembered is still folded state);
/// the arena runs on for three seconds and the runner is never called again.
#[test]
fn m1a5_a_wreck_cancels_its_minds_thinks_and_landing() {
    let mut rig = Rig::new(arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(6, 8),
            Vec::new(),
            Controller::Mind,
        )],
        &[],
    ));
    let mind = ScriptedMind::new(&[(
        RED,
        vec![
            says(0, vec![IntentKind::Startup]),
            says(1, vec![IntentKind::Goal(MotorGoal::Cell { x: 12, y: 8 })]),
        ],
    )]);
    rig.arena.mind_runner = mind.clone();
    let _ = rig.until(10_000, |arena| pending(arena, RED, 2));
    assert!(
        rig.arena.mind_landings.contains_key(&RED),
        "the precondition"
    );
    assert!(
        rows_of(&rig.arena, RED).4.contains(&"mind_land")
            && rows_of(&rig.arena, RED).4.contains(&"motor_tick"),
        "the precondition: a landing and a motor tick on the heap"
    );
    let now = rig.arena.now_ms;
    let mut journal: Vec<tacsim::arena::perception::Notice> = Vec::new();
    rig.arena.destroy_unit(RED, now, None, &mut journal);
    let (_, row, scratch, _, kinds) = rows_of(&rig.arena, RED);
    assert!(kinds.is_empty(), "the wreck's due kinds: {kinds:?}");
    assert!(!scratch, "the scratch went with the landing");
    assert!(row, "the row stays");
    assert_eq!(
        rig.arena.minds.get(&RED).and_then(|state| state.pending),
        None,
        "and names no think in flight"
    );
    let asked = mind.seen().len();
    let mut thinks = 0_u32;
    while rig.arena.now_ms < now + 3_000 {
        thinks += rig.pulse(&[], &[]).reflex.thinks;
    }
    assert_eq!(thinks, 0, "a wreck thinks nothing");
    assert_eq!(mind.seen().len(), asked, "the runner was not called");
}

/// **A mind's own heading lets go of its own goal** (17b cut M1a.5; D-UM-10's
/// disengage rule read for a mind: `IntentSource::Mind` beside `Seat`). Think 1
/// lands a `DRIVE` — the goal on the motor row, its tick on the heap — and
/// think 3 a `HEADING 180`: after that landing the goal is gone, the hull is
/// ordered the typed heading, and a second later the motor's tick is off the
/// heap and the heading is still the mind's.
#[test]
fn m1a5_a_minds_own_heading_lets_go_of_its_goal() {
    let mut rig = Rig::new(arena_of(
        vec![tank(
            RED,
            Team::Red,
            centre(6, 8),
            Vec::new(),
            Controller::Mind,
        )],
        &[],
    ));
    rig.arena.mind_runner = ScriptedMind::new(&[(
        RED,
        vec![
            says(0, vec![IntentKind::Startup]),
            says(1, vec![IntentKind::Goal(MotorGoal::Cell { x: 12, y: 8 })]),
            says(3, vec![IntentKind::Heading { mdeg: 180_000 }]),
        ],
    )]);
    let goal = |arena: &Arena| arena.motors.row(RED).and_then(|motor| motor.goal);
    let first = think_at(RED, 1) + SUBSTEP;
    let _ = rig.until(first + SUBSTEP, |arena| arena.now_ms >= first);
    assert!(goal(&rig.arena).is_some(), "the drive landed");
    let land = think_at(RED, 3) + SUBSTEP;
    let _ = rig.until(land, |arena| arena.now_ms >= land - SUBSTEP);
    assert!(
        goal(&rig.arena).is_some(),
        "still driving before the landing"
    );
    let _ = rig.until(land + SUBSTEP, |arena| arena.now_ms >= land);
    assert_eq!(goal(&rig.arena), None, "the mind took its helm back");
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.desired_heading_mdeg),
        Some(180_000)
    );
    let _ = rig.until(land + 2_000, |arena| arena.now_ms >= land + 1_000);
    assert!(
        !rows_of(&rig.arena, RED).4.contains(&"motor_tick"),
        "the motor's tick ended with its goal"
    );
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.desired_heading_mdeg),
        Some(180_000),
        "and nothing steered it back"
    );
}

/// **A landing's `cycling` is told at the landing's millisecond** (D-UM-8; the
/// refusal's numbers are true only where it was refused). A gun of a
/// 3,001 ms cycle fires at landing 4 and is ordered again at landing 7, three
/// seconds on: refused `cycling` one millisecond short of ready, which is
/// `1` second as the cockpit rounds it — not the `0` the substep's end, already
/// past ready, would read.
#[test]
fn m1a5_a_landings_cycling_refusal_is_told_at_the_landing() {
    let gun = WeaponSummary {
        cycle_ms: 3_001,
        ..cannon()
    };
    let mut rig = Rig::new(arena_of(
        vec![
            tank(RED, Team::Red, centre(6, 8), vec![gun], Controller::Mind),
            tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            ),
            tank(
                BLUE2,
                Team::Blue,
                centre(14, 1),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[BLUE],
    ));
    let fire = || IntentKind::Fire {
        weapon: SmolStr::new("cannon"),
    };
    rig.arena.mind_runner = ScriptedMind::new(&[(
        RED,
        vec![
            says(0, vec![IntentKind::Startup]),
            says(
                2,
                vec![IntentKind::Lock {
                    contact: draw_contact_id(SEED, RED, BLUE),
                }],
            ),
            says(4, vec![fire()]),
            says(7, vec![fire()]),
        ],
    )]);
    let again = think_at(RED, 7) + SUBSTEP;
    assert_ne!(
        again.rem_euclid(SUBSTEP),
        0,
        "the precondition: the landing is inside its substep"
    );
    let _ = rig.until(again + SUBSTEP, |arena| arena.now_ms >= again);
    assert_eq!(rig.lines("fired", RED), vec![think_at(RED, 4) + SUBSTEP]);
    let told: Vec<FrameNotice> = rig
        .arena
        .minds
        .get(&RED)
        .map(|state| {
            state
                .notices
                .iter()
                .filter(|notice| matches!(notice, FrameNotice::Refused { .. }))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(
        told,
        vec![FrameNotice::Refused {
            order: SmolStr::new("fire"),
            refusal: Refusal::Cycling,
            weapon: Some(SmolStr::new("cannon")),
            detail: RefusalDetail::Cycling { secs: 1 },
        }]
    );
}

// ---------------------------------------------------------------------------
// A pilot's `DRIVE` dies with the seat (`set_controller` on every transition)
// ---------------------------------------------------------------------------

/// One scripted beat: a seat's order, or a control over the hull.
#[derive(Clone, Debug)]
enum Beat {
    Order(IntentKind),
    Control(ControlState),
}

/// **The seated red, driven** at a fixed `cadence_ms` to `until_ms` through
/// `beats` — each `(t_ms, beat)` delivered in the first pulse whose arena time
/// has reached `t_ms` (every `t_ms` a multiple of 5,000, so one script means
/// one thing at every cadence: `tests/motor.rs`' `drive_run` rule). A control
/// raises the helm epoch by one and names the seat's pilot. Answers the run
/// and the arena it ended on.
fn seat_run(cadence_ms: i64, until_ms: i64, beats: &[(i64, Beat)]) -> (Run, Arena) {
    let mut arena = arena_of(
        vec![
            tank(RED, Team::Red, centre(6, 8), Vec::new(), Controller::Pilot),
            tank(
                BLUE,
                Team::Blue,
                centre(14, 14),
                Vec::new(),
                Controller::Pilot,
            ),
        ],
        &[RED],
    );
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut counters = ReflexCounters::default();
    let mut pulses = 0_u32;
    let mut next = 0_usize;
    let mut seq = 0_u64;
    let mut wall = 0_i64;
    loop {
        let mut intents: Vec<Intent> = Vec::new();
        let mut controls: Vec<Control> = Vec::new();
        while let Some((at, beat)) = beats.get(next) {
            if *at > arena.now_ms {
                break;
            }
            match beat {
                Beat::Order(kind) => {
                    seq += 1;
                    intents.push(intent(&arena, RED, kind.clone(), seq));
                }
                Beat::Control(state) => controls.push(control(
                    Channel::Helm,
                    arena.epochs.current(RED, Channel::Helm) + 1,
                    *state,
                    Some(ObjId(1_000)),
                )),
            }
            next += 1;
        }
        let out = arena.step(wall, &controls, &intents);
        pulses += 1;
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        counters.absorb(&out.reflex);
        hashes.extend(arena.substep_hashes.iter().copied());
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
    }
    let events = events(&mut arena, &mut bytes)
        .into_iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    (
        Run {
            events,
            hashes,
            pulses,
            counters,
        },
        arena,
    )
}

/// **A pilot's `DRIVE` dies with the seat** (17b cut M1a.5's `set_controller`
/// on every transition — the builder's declared behaviour change, on the one
/// production path that reaches it: `apply_control`'s `Released` arm).
///
/// Directly: the seated red powers up and types `DRIVE`, so its motor row
/// holds the goal and the search it ran from (`searched_from`) and its motor
/// tick is on the heap; a `Released` control hands the hull to the reflex
/// controller, and what is left of the unit is the machine's row, a motor row
/// exactly as a fresh adoption lays it down (nothing of the pilot's search
/// carried over) and one reflex tick — no motor tick lingering.
///
/// Across tempo (16b §11): the same script — `STARTUP` at nought, `DRIVE` at
/// 5,000, the seat released at 10,000, the machine flying to 30,000 — at
/// 5,000, 1,000 and 500 ms is one event list and one hash sequence.
#[test]
fn m1a5_a_pilots_drive_dies_with_the_released_seat() {
    use tacsim::arena::motor::MotorRow;
    let drive = IntentKind::Goal(MotorGoal::Cell { x: 12, y: 8 });
    let (_, mut arena) = seat_run(
        1_000,
        6_000,
        &[
            (0, Beat::Order(IntentKind::Startup)),
            (5_000, Beat::Order(drive.clone())),
        ],
    );
    let motor = arena.motors.row(RED).cloned().unwrap_or_default();
    assert!(
        motor.goal.is_some() && motor.searched_from.is_some(),
        "the precondition: the pilot drives, from a search: {motor:?}"
    );
    assert!(
        rows_of(&arena, RED).4.contains(&"motor_tick"),
        "the precondition: its tick is on the heap"
    );
    let release = control(
        Channel::Helm,
        arena.epochs.current(RED, Channel::Helm) + 1,
        ControlState::Released,
        Some(ObjId(1_000)),
    );
    apply_control(&mut arena, &release);
    assert_eq!(
        rows_of(&arena, RED),
        (true, false, false, true, vec!["reflex_tick"]),
        "the machine's row and tick, and nothing of the pilot's drive"
    );
    assert_eq!(
        arena.motors.row(RED),
        Some(&MotorRow::default()),
        "a fresh motor row"
    );

    let beats = [
        (0, Beat::Order(IntentKind::Startup)),
        (5_000, Beat::Order(drive)),
        (10_000, Beat::Control(ControlState::Released)),
    ];
    let (slow, after) = seat_run(5_000, 30_000, &beats);
    assert_eq!(
        after.unit(RED).map(|unit| unit.controller),
        Some(Controller::Reflex),
        "the machine flies it"
    );
    for cadence in [1_000, 500] {
        let (run, _) = seat_run(cadence, 30_000, &beats);
        assert_eq!(run.events, slow.events, "the event list at {cadence} ms");
        assert_eq!(run.hashes, slow.hashes, "the hash sequence at {cadence} ms");
        assert_ne!(run.pulses, slow.pulses, "{cadence} ms ran the same pulses");
    }
}

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
//!
//! **17b cut M1b.0** adds section (7b): the order of a millisecond as amended
//! (every landing, then the heap's events at that millisecond, then the
//! thinks — a landing's beam impact is in the view of a think of its own
//! millisecond, on the lattice's own thinks and at three cadences, and two
//! minds' beams at one millisecond both land) and `MindView.scan_tables` on a
//! mixed-chassis field. Its production line-hash pin lives beside the helpers
//! it replays: `tests/weapons.rs`, `tests/motor.rs` and `tests/reflex.rs`.
//!
//! **17b cut M1b.1** adds section (8b): a mind's arming lands in emission
//! order with its lock and the slot is evaluated ONCE, after every intent of
//! the landing (17b's adversarial F5), and a mind's arming under the floor is
//! refused `unlawful` at its landing with the floor named. The rest of the
//! slots' proofs live in `tests/conditions.rs`.

mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use mush_core::{
    CanonicalId, DefinitionDigest, DefinitionId, DefinitionKey, DefinitionKind, ObjId, OwnerId,
};
use mush_sim::data::Pose;
use mush_tactics::{
    CellSpec, DefinitionRef, FacetId, FacetSpec, RulesValue, SectionId, SectionSpec, TacticalUnit,
    UnitSpec,
};
use smol_str::SmolStr;
use tacsim::arena::clock::Tempo;
use tacsim::arena::command::SquadSignal;
use tacsim::arena::condition::{Condition, Reading};
use tacsim::arena::conditions::{ASPECTS_ANY, Arming, FireCondition, SlotTarget};
use tacsim::arena::intents::{
    Control, ControlState, Intent, IntentKind, IntentOutcome, IntentSource, MotorGoal, Refusal,
    apply_control, apply_intent,
};
use tacsim::arena::ledger::LedgerEvent;
use tacsim::arena::mind::{
    MIND_INTENTS_MAX, MIND_NOTICES, MindRunner, MindState, MindView, STOCK_THINK_MS, ThinkJob,
    ThinkResult,
};
use tacsim::arena::perception::{
    FrameNotice, FrameOrder, MindObjective, RefusalDetail, SensorProfile, SignatureFactor,
};
use tacsim::arena::reflex::{ReflexCounters, ReflexState};
use tacsim::arena::resolution::{BandProfile, Consequence, SectionProfile, UnitProfile};
use tacsim::arena::rng::{draw_contact_id, stagger_ms};
use tacsim::arena::snapshot::Snapshot;
use tacsim::arena::state::{
    Arena, ArenaDefinition, ArenaTerrain, Channel, Classification, ContactId, Controller,
    CustodyOut, CustodyUnit, DueKind, EntryZone, Pilot, Power, SeenPose, ShadowContact, Team,
    TerrainCell, TrackedRules, Waypoint, Waypoints, WeaponSummary,
};
use tacsim::arena::step::StepOutput;
use tacsim::arena::weapons::WeaponBand;
use tacsim::config::{ArenaConfig, MatchConfig, ReflexConfig, SensorsConfig};
use tacsim::definitions::{Catalog, content};
use tacsim::deliberation::order::{CharacterRef, Constraints, Objective, Order};
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
                        slot: None,
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
                says(
                    4,
                    vec![IntentKind::Fire {
                        weapon: gun,
                        slot: None,
                    }],
                ),
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
                        slot: None,
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

/// **17b cut L.1**: the scripted pilot character a pilot-shaped order (one with
/// an `issuer`) names as its author; no unit of these fixtures uses the id.
const PILOT_CHARACTER: ObjId = ObjId(9_001);

/// An order for `squad`, authored as its shape says (17b cut L.1, D-UM-52): a
/// machine order (`issuer: None`) by the squad's lance-leader persona, a pilot's
/// (`issuer: Some(..)`) by [`PILOT_CHARACTER`].
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
        by: issuer.map_or(CharacterRef::Persona { squad }, |_| {
            CharacterRef::Character {
                id: PILOT_CHARACTER,
            }
        }),
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
///
/// **17b cut L.1 re-asserts the second wall with `by` present** (D-UM-11,
/// D-UM-52): the mind's `ORDER` names an author the world's admission would
/// take — red-1's own lance-leader persona, written in the commander's scope —
/// and the arena still refuses it `channel`: who is named as the author never
/// opens the door to a mind, because a mind never orders.
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
    let told = order(RED_ONE, Objective::Hold, None, 30_000);
    assert_eq!(
        (&told.by, told.issuer),
        (&CharacterRef::Persona { squad: RED_ONE }, None),
        "17b cut L.1: the mind's order carries an author the admission would admit"
    );
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
                        IntentKind::Order(Box::new(told)),
                        IntentKind::ReleaseOrder {
                            squad: RED_ONE,
                            issuer: None,
                        },
                        IntentKind::Place { x: 3, y: 3 },
                        IntentKind::Fire {
                            weapon: gun.clone(),
                            slot: None,
                        },
                        IntentKind::Fire {
                            weapon: gun.clone(),
                            slot: None,
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
                says(
                    4,
                    vec![IntentKind::Fire {
                        weapon: gun,
                        slot: None,
                    }],
                ),
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
// (7b) 17b cut M1b.0: the order of a millisecond, and the view's tables
// ---------------------------------------------------------------------------

/// **The shot a mind's beam fixture fires** (17b cut M1b.0): one uncited
/// `FIRE cannon`. Every beam test of this section fires through here and
/// nowhere else, so when 17b cut M1b.2 puts the law on a mind's fire it swaps
/// this body for an arming plus a cited fire and every proof keeps its
/// millisecond.
fn beam_shot(unit: ObjId) -> Vec<IntentKind> {
    // M1b.2 cites a slot here (an arming on the lock, then `Fire` citing it).
    let _ = unit;
    vec![IntentKind::Fire {
        weapon: SmolStr::new("cannon"),
        slot: None,
    }]
}

/// **A beam**: [`cannon`] with no flight time — `muzzle_speed_cms` nought, so
/// `resolve_fire` puts its impact at its own millisecond, the shape of the
/// shipped `laser_mk1` (`delivery: beam`) — doing `damage`.
fn beam(damage: u32) -> WeaponSummary {
    WeaponSummary {
        muzzle_speed_cms: 0,
        damage,
        ..cannon()
    }
}

/// **The reviewer's target profile** (the external review of 2026-10-04,
/// finding 3): one `front` band of one metre square that is all `hull`, and a
/// `hull` with one `front` facet — so a round that lands on a hull facing its
/// shooter strikes the rig's one armoured facet. `lethal` gives the hull
/// `destroy_unit`.
fn front_profile(lethal: bool) -> UnitProfile {
    UnitProfile {
        bands: vec![BandProfile {
            band: SmolStr::new("front"),
            width_cm: 100,
            height_cm: 100,
            sections: vec![(SmolStr::new("hull"), 1)],
        }],
        sections: vec![SectionProfile {
            id: SmolStr::new("hull"),
            facets: vec![SmolStr::new("front")],
            consequences: if lethal {
                vec![Consequence::DestroyUnit]
            } else {
                Vec::new()
            },
            struck_through: None,
        }],
    }
}

/// **Two unit ids whose think lattices stand `gap_ms` apart** (17b cut M1b.0):
/// the first `(a, b)` over `1..=2_000`, `a` then `b` ascending and `a != b`,
/// with `(stagger(b) − stagger(a)).rem_euclid(STOCK_THINK_MS) == gap_ms`. At
/// the substep, `a`'s landing (one substep after its think) IS one of `b`'s
/// thinks — the collision the external review found; at nought the two think,
/// and land, at one millisecond.
fn lattice_pair(gap_ms: i64) -> (ObjId, ObjId) {
    let offsets: Vec<(ObjId, i64)> = (1_u32..=2_000)
        .map(|id| (ObjId(id), stagger_ms(SEED, ObjId(id), STOCK_THINK_MS)))
        .collect();
    offsets
        .iter()
        .find_map(|(a, at_a)| {
            offsets
                .iter()
                .find(|(b, at_b)| b != a && (at_b - at_a).rem_euclid(STOCK_THINK_MS) == gap_ms)
                .map(|(b, _)| (*a, *b))
        })
        .unwrap_or_else(|| panic!("no pair of ids {gap_ms} ms apart on the lattice"))
}

/// The armour left on `view`'s own facets, summed.
fn armor_of(view: &MindView) -> i64 {
    view.sections
        .iter()
        .flat_map(|section| &section.facets)
        .map(|facet| facet.armor)
        .sum()
}

/// **The beam field** (17b cut M1b.0; the reviewer's setup, adopted): `red`, a
/// mind with a [`beam`] of `damage`, at (6, 8) facing east, and `blue`, a mind
/// at (9, 8) facing west with [`front_profile`] — armed with a beam of its own
/// when `mutual`, and then `red` gets the profile too. Both are online, the odds
/// are pinned at certainty (`p_min = p_max = 1,000`), and each armed hull holds
/// the other as a hand-inserted `Classified` row with a lock on it. The
/// thinks are the lattice's own: nothing is retained off the heap and nothing
/// scheduled by hand. Each armed mind says [`beam_shot`] at think `shoot_at`
/// and nothing else.
fn beam_field(
    red: ObjId,
    blue: ObjId,
    damage: u32,
    mutual: bool,
    shoot_at: u64,
) -> (Arena, Arc<ScriptedMind>) {
    let mut red_unit = tank(
        red,
        Team::Red,
        centre(6, 8),
        vec![beam(damage)],
        Controller::Mind,
    );
    let mut blue_unit = tank(
        blue,
        Team::Blue,
        centre(9, 8),
        if mutual {
            vec![beam(damage)]
        } else {
            Vec::new()
        },
        Controller::Mind,
    );
    blue_unit.profile = front_profile(mutual);
    if mutual {
        red_unit.profile = front_profile(true);
    }
    let mut arena = arena_of(vec![red_unit, blue_unit], &[]);
    arena.resolution.p_min = 1_000;
    arena.resolution.p_max = 1_000;
    for (id, heading) in [(red, 90_000_u32), (blue, 270_000)] {
        if let Some(state) = arena.unit_mut(id) {
            state.power = Power::Online;
            state.pose.heading_mdeg = heading;
            state.desired_heading_mdeg = heading;
        }
    }
    let shooters: &[(ObjId, ObjId)] = if mutual {
        &[(red, blue), (blue, red)]
    } else {
        &[(red, blue)]
    };
    let mut script: Vec<(ObjId, Vec<Says>)> = Vec::new();
    for &(viewer, target) in shooters {
        let contact = draw_contact_id(SEED, viewer, target);
        let Some(plot) = arena.unit(target).map(SeenPose::of) else {
            panic!("a fielded target");
        };
        let team = if target == red { Team::Red } else { Team::Blue };
        arena.contacts.insert(
            viewer,
            contact,
            ShadowContact::sighted(target, Classification::Classified { team }, 0, plot),
        );
        assert_eq!(
            arena.resolve_lock(viewer, contact, 0).map(|_| ()),
            Ok(()),
            "#{} holds a lock",
            viewer.0
        );
        script.push((viewer, vec![says(shoot_at, beam_shot(viewer))]));
    }
    let mind = ScriptedMind::new(&script);
    arena.mind_runner = mind.clone();
    (arena, mind)
}

/// The first think of `red`'s whose landing falls on one of `blue`'s thinks,
/// and that think of `blue`'s: `(red's think_seq, blue's think_seq, the
/// millisecond)`.
fn landing_on_a_think(red: ObjId, blue: ObjId) -> (u64, u64, i64) {
    for n in 0_i64..8 {
        let land = think_at(red, n) + SUBSTEP;
        let since = land - think_at(blue, 0);
        if since >= 0 && since.rem_euclid(STOCK_THINK_MS) == 0 {
            return (
                u64::try_from(n).unwrap_or(0),
                u64::try_from(since / STOCK_THINK_MS).unwrap_or(0),
                land,
            );
        }
    }
    panic!("#{}'s landings never meet #{}'s thinks", red.0, blue.0);
}

/// **What the beam-before-think fixture must show at `land`**: one `impact`
/// line, at `land`, after red's `fired` line in `(t_ms, seq)`; blue's think
/// at `land` handed a view whose armour is the damaged figure the arena holds
/// after; and red's landing refused nothing.
fn assert_the_beam_preceded_the_think(
    rig: &mut Rig,
    mind: &ScriptedMind,
    red: ObjId,
    blue: ObjId,
    blue_think: u64,
    land: i64,
) {
    let journal = rig.journal();
    let lines = |word: &str| -> Vec<&LedgerEvent> {
        journal
            .iter()
            .filter(|event| {
                kind_and_unit(&serde_json::to_string(&event.kind).unwrap_or_default()).0 == word
            })
            .collect()
    };
    let fired = lines("fired");
    let impacts = lines("impact");
    assert_eq!(fired.len(), 1, "red fired once: {journal:?}");
    assert_eq!(impacts.len(), 1, "the beam landed once: {journal:?}");
    let (Some(fired), Some(impact)) = (fired.first(), impacts.first()) else {
        panic!("one of each");
    };
    assert_eq!(
        kind_and_unit(&serde_json::to_string(&fired.kind).unwrap_or_default()).1,
        Some(red.0),
        "red's shot"
    );
    assert_eq!(fired.t_ms, land, "the shot at the landing");
    assert_eq!(
        impact.t_ms, land,
        "the beam's impact at its own millisecond"
    );
    assert!(
        (fired.t_ms, fired.seq) < (impact.t_ms, impact.seq),
        "the fired line before the impact"
    );
    let Some(seen) = mind
        .seen()
        .into_iter()
        .find(|seen| seen.unit == blue && seen.think_seq == blue_think)
    else {
        panic!("blue's think {blue_think} ran");
    };
    assert_eq!(
        seen.t_ms, land,
        "blue's think shares the impact's millisecond"
    );
    let Some(after) = rig.arena.mind_view(blue) else {
        panic!("blue's view");
    };
    assert!(
        armor_of(&after) < 10,
        "the beam damaged blue: {}",
        armor_of(&after)
    );
    assert_eq!(
        armor_of(&seen.view),
        armor_of(&after),
        "D-UM-20 as amended: the impact a landing scheduled at the think's millisecond is in \
         the think's view"
    );
    assert_eq!(refusals(&rig.arena, red), Vec::new(), "red's landing");
}

/// **17b cut M1b.0 — a landing's beam impact precedes the thinks of its
/// millisecond** (doc 17 §6 and D-UM-20 as amended 2026-10-04; the external
/// review's finding 3, its regression adopted).
///
/// On the lattice's own thinks: two ids whose stagger offsets stand one
/// substep apart ([`lattice_pair`]), so red's landing one substep after its
/// think IS a think millisecond of blue's — nothing retained off the heap and
/// nothing scheduled by hand. Red's mind says [`beam_shot`] at the first think
/// whose landing meets one of blue's ([`landing_on_a_think`]); the beam's
/// impact is scheduled at the landing's own millisecond, and is applied before
/// blue's think of that millisecond takes its view: one `impact` line at the
/// think's millisecond, after the `fired` line in `(t_ms, seq)`, and blue's
/// view carries the damaged armour the arena holds after. Under cut M1a.5's
/// one pass the think ran first and saw the armour whole.
///
/// The second arm is the reviewer's own form, kept: `RED` and `BLUE` with the
/// thinks, passes and shares retained off the heap and the two thinks put at
/// 750 and 1,000 ms by hand.
#[test]
fn m1b0_a_landings_beam_impact_precedes_the_thinks_of_its_millisecond() {
    // (a) The lattice's own thinks.
    let (red, blue) = lattice_pair(SUBSTEP);
    assert_eq!(
        (stagger_ms(SEED, blue, STOCK_THINK_MS) - stagger_ms(SEED, red, STOCK_THINK_MS))
            .rem_euclid(STOCK_THINK_MS),
        SUBSTEP,
        "the precondition: the two lattices one substep apart"
    );
    let (shoot_at, blue_think, land) = landing_on_a_think(red, blue);
    let (arena, mind) = beam_field(red, blue, 1, false, shoot_at);
    assert_eq!(arena.substep_len_ms(), SUBSTEP, "the rig's substep");
    let mut rig = Rig::new(arena);
    let _ = rig.until(land + STOCK_THINK_MS, |arena| arena.now_ms >= land);
    assert_the_beam_preceded_the_think(&mut rig, &mind, red, blue, blue_think, land);

    // (b) The reviewer's hand-scheduled form.
    let (arena, mind) = beam_field(RED, BLUE, 1, false, 0);
    let mut rig = Rig::new(arena);
    let _ = rig.arena.due.retain(|kind| {
        !matches!(
            kind,
            DueKind::MindThink { .. } | DueKind::SensorPass { .. } | DueKind::Share { .. }
        )
    });
    rig.arena.due.schedule(
        750,
        DueKind::MindThink {
            unit: RED,
            think_seq: 0,
        },
    );
    rig.arena.due.schedule(
        1_000,
        DueKind::MindThink {
            unit: BLUE,
            think_seq: 0,
        },
    );
    let _ = rig.until(1_250, |arena| arena.now_ms >= 1_000);
    assert_the_beam_preceded_the_think(&mut rig, &mind, RED, BLUE, 0, 1_000);
}

/// The lattice beam field of
/// [`m1b0_a_landings_beam_impact_precedes_the_thinks_of_its_millisecond`],
/// run to 5,000 ms under `drive`.
fn beam_before_think(drive: Drive) -> (Run, Vec<Seen>) {
    let (red, blue) = lattice_pair(SUBSTEP);
    let (shoot_at, _, _) = landing_on_a_think(red, blue);
    let (mut arena, mind) = beam_field(red, blue, 1, false, shoot_at);
    let run = run(&mut arena, drive, 5_000);
    (run, mind.seen())
}

/// **17b cut M1b.0's mandatory cross-tempo property** (16b §11; doc 16 §3.1
/// D-16-2; doc 17 §6 D-UM-20 as amended, §22's gate item). The beam field at
/// 5,000, 1,000 and 500 ms and on the task's own wakes at those three periods:
/// one event list, one per-substep hash sequence and one sequence of jobs
/// handed to the runner — so the impact's millisecond, and the view blue's
/// think took at it with the beam's damage in it, are the same at every
/// cadence.
#[test]
fn m1b0_the_beam_before_the_think_holds_at_three_cadences() {
    let (red, blue) = lattice_pair(SUBSTEP);
    let (_, blue_think, land) = landing_on_a_think(red, blue);
    let (first, seen) = beam_before_think(Drive::Every(5_000));
    let impacts: Vec<i64> = first
        .events
        .iter()
        .filter(|(_, json, _)| kind_and_unit(json).0 == "impact")
        .map(|(t_ms, _, _)| *t_ms)
        .collect();
    assert_eq!(impacts, vec![land], "one impact, at the landing");
    let at_impact = seen
        .iter()
        .find(|seen| seen.unit == blue && seen.think_seq == blue_think)
        .map(|seen| (seen.t_ms, armor_of(&seen.view)));
    assert_eq!(
        at_impact,
        Some((land, 9)),
        "blue's think at the impact's millisecond saw the beam's one point"
    );
    for drive in [
        Drive::Every(1_000),
        Drive::Every(500),
        Drive::Task(5_000),
        Drive::Task(1_000),
        Drive::Task(500),
    ] {
        let (other, other_seen) = beam_before_think(drive);
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

/// **17b cut M1b.0 — two minds' beams at one millisecond both land** (doc 17
/// §6 and D-UM-20 as amended: *every* landing runs before the heap's events of
/// its millisecond). Two mind hulls whose stagger offsets are equal
/// ([`lattice_pair`] at nought), each with a lethal beam and locked on the
/// other, say [`beam_shot`] at the same think: they land at one millisecond,
/// and both land before either impact pops — two `fired` lines, both before
/// either `impact` line in `(t_ms, seq)`, two `impact` lines, both hulls
/// wrecks. The simultaneity two `ReflexTick`s at one millisecond have.
///
/// **The two `fired` lines are the proof.** A drain that ran each landing's
/// impact before the next landing would not refuse the second shot: the second
/// hull would be a wreck by then, and `Arena::mind_land` returns before it
/// applies a wreck's landing — no `fired` line and no refusal notice, silently.
/// The buffer reads (no refusal in either mind's buffer) only confirm nothing
/// else in the two shots was refused; they cannot see that drain.
#[test]
fn m1b0_mutual_beams_at_one_millisecond_both_land() {
    let (red, blue) = lattice_pair(0);
    assert_eq!(
        stagger_ms(SEED, red, STOCK_THINK_MS),
        stagger_ms(SEED, blue, STOCK_THINK_MS),
        "the precondition: one think millisecond"
    );
    let (arena, _) = beam_field(red, blue, 120, true, 0);
    let land = think_at(red, 0) + SUBSTEP;
    let mut rig = Rig::new(arena);
    let _ = rig.until(land + STOCK_THINK_MS, |arena| arena.now_ms >= land);
    let lines: Vec<(String, Option<u32>, i64)> = rig
        .journal()
        .iter()
        .map(|event| {
            let (word, unit) =
                kind_and_unit(&serde_json::to_string(&event.kind).unwrap_or_default());
            (word, unit, event.t_ms)
        })
        .collect();
    let fired: Vec<(Option<u32>, i64)> = lines
        .iter()
        .filter(|(word, _, _)| word == "fired")
        .map(|(_, unit, t_ms)| (*unit, *t_ms))
        .collect();
    assert_eq!(
        fired,
        vec![(Some(red.0), land), (Some(blue.0), land)],
        "both fired at the one landing millisecond, in unit-id order: {lines:?}"
    );
    let impacts: Vec<(Option<u32>, i64)> = lines
        .iter()
        .filter(|(word, _, _)| word == "impact")
        .map(|(_, unit, t_ms)| (*unit, *t_ms))
        .collect();
    assert_eq!(
        impacts,
        vec![(Some(red.0), land), (Some(blue.0), land)],
        "both beams impacted at that millisecond: {lines:?}"
    );
    // Both landings before either impact, in `(t_ms, seq)`: the order the
    // proof rests on, read off the lines themselves.
    let mut at = |word: &str| -> Vec<(i64, u64)> {
        rig.journal()
            .iter()
            .filter(|event| {
                kind_and_unit(&serde_json::to_string(&event.kind).unwrap_or_default()).0 == word
            })
            .map(|event| (event.t_ms, event.seq))
            .collect()
    };
    let (fired_at, impact_at) = (at("fired"), at("impact"));
    assert!(
        fired_at
            .iter()
            .all(|shot| impact_at.iter().all(|hit| shot < hit)),
        "every fired line precedes every impact line: {fired_at:?} {impact_at:?}"
    );
    for hull in [red, blue] {
        assert_eq!(
            refusals(&rig.arena, hull),
            Vec::new(),
            "#{}'s landing refused nothing",
            hull.0
        );
        assert!(
            rig.arena.unit(hull).is_some_and(|state| state.limits.wreck),
            "#{} was destroyed",
            hull.0
        );
    }
}

/// **17b cut M1b.0 — a mixed-chassis view carries its own scan tables** (doc
/// 17 §3 and D-UM-4 as amended 2026-10-04; the external review's finding 4).
/// Two minds of red, a shipped line tank and a shipped scout (whose section
/// words differ: the scout's wheels where the tank has tracks), each holding a
/// read, classified row of a blue line tank and of a blue scout — the other
/// chassis and its own. Each view's `scan_tables` has two tables, each row's
/// `condition.table` resolves to its own target chassis's section words, and
/// the tables are the hull's keyframe's (`Arena::frame_scan_tables`, which
/// `Frame::scan_tables` and the keyframe's `scan_tables` key are built from).
#[test]
fn m1b0_a_mixed_chassis_view_carries_its_own_scan_tables() {
    let fielded = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    let chassis = |entry: &str, id: ObjId| -> TacticalUnit {
        let key = CanonicalId::new(entry).expect("a canonical id");
        let (Some(spec), Some(binding)) = (fielded.specs.get(&key), fielded.bindings.get(&key))
        else {
            panic!("{entry} is fielded");
        };
        tacsim::definitions::to_tactical_unit(spec, binding, id)
            .unwrap_or_else(|why| panic!("{entry} instantiates: {why}"))
    };
    let words = |unit: &TacticalUnit| -> Vec<SmolStr> {
        unit.sections()
            .map(|(section, _)| SmolStr::new(section.as_str()))
            .collect()
    };
    let (tank_mind, scout_mind) = (ObjId(4), ObjId(6));
    let (blue_tank, blue_scout) = (ObjId(9), ObjId(12));
    let field = [
        (tank_mind, Team::Red, (6, 8), content::LINE_TANK),
        (scout_mind, Team::Red, (6, 10), content::SCOUT_MK1),
        (blue_tank, Team::Blue, (9, 8), content::LINE_TANK),
        (blue_scout, Team::Blue, (9, 10), content::SCOUT_MK1),
    ];
    let units: Vec<CustodyUnit> = field
        .iter()
        .map(|(id, team, (x, y), entry)| {
            let controller = if *team == Team::Red {
                Controller::Mind
            } else {
                Controller::Pilot
            };
            CustodyUnit {
                unit: chassis(entry, *id),
                ..tank(*id, *team, centre(*x, *y), vec![cannon()], controller)
            }
        })
        .collect();
    let tank_words = words(&chassis(content::LINE_TANK, ObjId(1)));
    let scout_words = words(&chassis(content::SCOUT_MK1, ObjId(1)));
    assert_ne!(
        tank_words, scout_words,
        "the precondition: two vocabularies"
    );
    let mut arena = arena_of(units, &[blue_tank, blue_scout]);
    for viewer in [tank_mind, scout_mind] {
        for target in [blue_tank, blue_scout] {
            let Some(target_state) = arena.unit(target) else {
                panic!("a fielded target");
            };
            let plot = SeenPose::of(target_state);
            let condition = Condition::of(&target_state.unit, 750, 350);
            let mut row = ShadowContact::sighted(
                target,
                Classification::Classified { team: Team::Blue },
                0,
                plot,
            );
            row.seen_condition = Some(Reading {
                at_ms: 0,
                condition,
            });
            arena
                .contacts
                .insert(viewer, draw_contact_id(SEED, viewer, target), row);
        }
    }
    for viewer in [tank_mind, scout_mind] {
        let keyframe = arena.frame_scan_tables(viewer);
        let Some(view) = arena.mind_view(viewer) else {
            panic!("#{}'s view", viewer.0);
        };
        assert_eq!(view.scan_tables.len(), 2, "#{}: two chassis", viewer.0);
        assert_eq!(
            view.scan_tables, keyframe,
            "#{}: the view's tables are the keyframe's",
            viewer.0
        );
        assert_eq!(view.contacts.len(), 2, "#{}: both rows", viewer.0);
        for (target, want) in [(blue_tank, &tank_words), (blue_scout, &scout_words)] {
            let id = draw_contact_id(SEED, viewer, target);
            let table = view
                .contacts
                .iter()
                .find(|contact| contact.id == id)
                .and_then(|contact| contact.condition)
                .and_then(|reading| view.scan_tables.get(usize::from(reading.table)));
            assert_eq!(
                table,
                Some(want),
                "#{}'s row of #{} indexes its own chassis's words",
                viewer.0,
                target.0
            );
        }
    }
}

// ---------------------------------------------------------------------------
// (8b) 17b cut M1b.1: a mind's arming lands in emission order, evaluated once
// ---------------------------------------------------------------------------

/// A slot on the lock at `min_permille`, any aspect, no reserve, any range.
fn slot_on_lock(min_permille: u16) -> FireCondition {
    FireCondition {
        weapon: SmolStr::new("cannon"),
        target: SlotTarget::Lock,
        min_permille,
        aspects: ASPECTS_ANY,
        min_rounds_left: 0,
        min_range_m: 0,
        max_range_m: u32::MAX,
    }
}

/// A target every band of which presents a silhouette of hull, so a shot at
/// any aspect resolves on the curve and the clamp `[p_min, p_max]` decides it.
fn every_band_profile() -> UnitProfile {
    UnitProfile {
        bands: ["front", "left", "right", "rear"]
            .into_iter()
            .map(|band| BandProfile {
                band: SmolStr::new(band),
                width_cm: 300,
                height_cm: 200,
                sections: vec![(SmolStr::new("hull"), 1)],
            })
            .collect(),
        ..front_profile(false)
    }
}

/// **The arming field** (17b cut M1b.1): [`RED`], a mind with a ready
/// [`cannon`], online at (6, 8) facing east, holding [`BLUE`] at (9, 8) and
/// [`BLUE2`] at (9, 10) — each [`every_band_profile`] — as hand-inserted
/// `Classified` rows and LOCKED ON BLUE; the odds pinned at certainty. Red's
/// mind says `says_at_0` at think 0 and nothing else; the two blues are minds
/// that say nothing. Answers the arena, the runner, and red's landing
/// millisecond.
fn arming_field(says_at_0: Vec<IntentKind>) -> (Arena, Arc<ScriptedMind>, i64) {
    let target = |id: ObjId, y: u16| CustodyUnit {
        profile: every_band_profile(),
        ..tank(id, Team::Blue, centre(9, y), Vec::new(), Controller::Mind)
    };
    let units = vec![
        tank(
            RED,
            Team::Red,
            centre(6, 8),
            vec![cannon()],
            Controller::Mind,
        ),
        target(BLUE, 8),
        target(BLUE2, 10),
    ];
    let mut arena = arena_of(units, &[]);
    arena.resolution.p_min = 1_000;
    arena.resolution.p_max = 1_000;
    if let Some(state) = arena.unit_mut(RED) {
        state.power = Power::Online;
        state.pose.heading_mdeg = 90_000;
        state.desired_heading_mdeg = 90_000;
    }
    for target in [BLUE, BLUE2] {
        let Some(plot) = arena.unit(target).map(SeenPose::of) else {
            panic!("a fielded target");
        };
        arena.contacts.insert(
            RED,
            draw_contact_id(SEED, RED, target),
            ShadowContact::sighted(
                target,
                Classification::Classified { team: Team::Blue },
                0,
                plot,
            ),
        );
    }
    assert_eq!(
        arena.resolve_lock(RED, draw_contact_id(SEED, RED, BLUE), 0),
        Ok(()),
        "red holds a lock on blue"
    );
    let mind = ScriptedMind::new(&[(RED, vec![says(0, says_at_0)])]);
    arena.mind_runner = mind.clone();
    (arena, mind, think_at(RED, 0) + SUBSTEP)
}

/// Red's `fired` lines so far, as `(t_ms, target, by, slot)`.
fn red_shots(rig: &mut Rig) -> Vec<(i64, u64, Option<String>, Option<u64>)> {
    rig.journal()
        .iter()
        .filter_map(|event| {
            let value = serde_json::to_value(&event.kind).ok()?;
            (value.get("kind")? == "fired" && value.get("unit")? == u64::from(RED.0)).then(|| {
                (
                    event.t_ms,
                    value
                        .get("target")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0),
                    value
                        .get("by")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                    value.get("slot").and_then(serde_json::Value::as_u64),
                )
            })
        })
        .collect()
}

/// **A mind's arming lands in emission order and its slot is evaluated ONCE,
/// after every intent of the landing** (17b cut M1b.1; doc 17 §5, D-UM-16;
/// 17b's adversarial F5). Red, locked on BLUE with a ready gun, thinks
/// `[Condition(Arm(slot on the lock)), Lock { BLUE2 }]` — the arming BEFORE
/// the lock, as a mind emits them. The arming lands first (S1), the lock
/// second; the slot's one `Landing` evaluation after both reads the NEW lock:
/// the first `fired` line is `by: cond`, `slot: 1`, at BLUE2, at the landing's
/// own millisecond, and no shot ever names BLUE. An evaluation after the
/// landed `Arm` (the F5 defect) would have fired at BLUE.
///
/// And a mind's arming under the floor is refused `unlawful` at its landing,
/// buffered on the mind with its weapon and the floor named
/// (`RefusalDetail::Floor { permille: 100 }`), and arms nothing.
#[test]
fn m1b1_a_minds_arming_lands_in_emission_order_and_is_evaluated_once_after_the_landing() {
    let blue2 = draw_contact_id(SEED, RED, BLUE2);
    let (arena, _mind, land) = arming_field(vec![
        IntentKind::Condition(Arming::Arm(slot_on_lock(100))),
        IntentKind::Lock { contact: blue2 },
    ]);
    let mut rig = Rig::new(arena);
    let _ = rig.until(land + STOCK_THINK_MS, |arena| arena.now_ms >= land);
    assert_eq!(
        rig.arena.unit(RED).and_then(|state| state.lock),
        Some(blue2)
    );
    assert_eq!(rig.arena.conditions.slot_of(RED, "cannon"), Some(1));
    let shots = red_shots(&mut rig);
    assert_eq!(
        shots.first(),
        Some(&(land, u64::from(BLUE2.0), Some("cond".to_owned()), Some(1))),
        "the slot's one evaluation read the landed lock: {shots:?}"
    );
    assert!(
        shots
            .iter()
            .all(|(_, target, _, _)| *target != u64::from(BLUE.0)),
        "no shot at the lock the think dropped: {shots:?}"
    );
    assert_eq!(refusals(&rig.arena, RED), Vec::new(), "red's landing");

    // Under the floor: refused `unlawful` at the landing, the floor named.
    let (arena, _mind, land) =
        arming_field(vec![IntentKind::Condition(Arming::Arm(slot_on_lock(50)))]);
    let mut rig = Rig::new(arena);
    let _ = rig.until(land + STOCK_THINK_MS, |arena| arena.now_ms >= land);
    assert!(rig.arena.conditions.is_empty_for(RED), "nothing armed");
    assert_eq!(red_shots(&mut rig), Vec::new());
    let buffered: Vec<FrameNotice> = rig
        .arena
        .minds
        .get(&RED)
        .map(|state| state.notices.clone())
        .unwrap_or_default();
    assert!(
        buffered.contains(&FrameNotice::Refused {
            order: SmolStr::new("fire"),
            refusal: Refusal::Unlawful,
            weapon: Some(SmolStr::new("cannon")),
            detail: RefusalDetail::Floor { permille: 100 },
        }),
        "the mind's refusal names its weapon and the floor: {buffered:?}"
    );
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
/// and `Arena::new` lays its row down and schedules its first think; it
/// thinks once an arena second. **17b cut M1a.6**: the default runner is the
/// native stock mind, whose first think starts the cold tank (it was `NoMind`,
/// which ordered nothing).
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
        Some(tacsim::arena::state::Power::Starting),
        "the default runner, the native stock mind, starts the cold tank"
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
    // 17b cut M1a.6: the default runner is the native stock mind.
    assert!(format!("{:?}", decoded.mind_runner).contains("NativeStockMind"));
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
        slot: None,
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

// ---------------------------------------------------------------------------
// 17b cut M1a.6: the native stock mind, flying (doc 17 §17, §22; D-UM-3,
// D-UM-21, D-UM-48)
// ---------------------------------------------------------------------------

/// What a stock think was handed and answered: the hull, the lattice
/// millisecond, the view and the result.
type Kept = (ObjId, i64, MindView, ThinkResult);

/// **The production stock mind, its answers kept** — `NativeStockMind` on the
/// arena's own doctrine, answering every think untouched and recording each
/// view and answer (an input recorder, not a stand-in: 16b §0.4).
#[derive(Debug)]
struct Stock {
    inner: tacsim::arena::stock_mind::NativeStockMind,
    kept: Mutex<Vec<Kept>>,
}

impl Stock {
    fn on(arena: &Arena) -> Arc<Stock> {
        Arc::new(Stock {
            inner: tacsim::arena::stock_mind::NativeStockMind::with(&arena.arena_cfg.reflex),
            kept: Mutex::new(Vec::new()),
        })
    }

    fn kept(&self) -> Vec<Kept> {
        self.kept
            .lock()
            .map(|kept| kept.clone())
            .unwrap_or_default()
    }
}

impl MindRunner for Stock {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        let result = self.inner.think(job);
        if let Ok(mut kept) = self.kept.lock() {
            kept.push((job.unit, job.t_ms, job.view.clone(), result.clone()));
        }
        result
    }
}

/// **The stock mind's emission rules** (`stock_mind.rs`' module head): kinds in
/// `Startup, Goal, Heading, Speed, Lock, Fire` order, each at most once, no
/// `Heading` or `Speed` beside a `Goal(Cell)`, never more than four.
fn stock_emission_holds(intents: &[IntentKind]) {
    let rank = |kind: &IntentKind| match kind {
        IntentKind::Startup => 0_u8,
        IntentKind::Goal(_) => 1,
        IntentKind::Heading { .. } => 2,
        IntentKind::Speed { .. } => 3,
        IntentKind::Lock { .. } => 4,
        IntentKind::Fire { .. } => 5,
        other => panic!("the stock mind emitted {other:?}"),
    };
    let ranks: Vec<u8> = intents.iter().map(rank).collect();
    assert!(
        ranks.windows(2).all(|pair| pair.first() < pair.get(1)),
        "{intents:?}"
    );
    if intents
        .iter()
        .any(|kind| matches!(kind, IntentKind::Goal(MotorGoal::Cell { .. })))
    {
        assert!(
            !intents
                .iter()
                .any(|kind| matches!(kind, IntentKind::Heading { .. } | IntentKind::Speed { .. })),
            "{intents:?}"
        );
    }
    // The module head's maximum: `Goal(Hold)`, `Heading`, `Lock`, `Fire`.
    assert!(intents.len() <= 4 && intents.len() <= MIND_INTENTS_MAX);
}

/// `tests/reflex.rs`' `replay_driven` scene with the two reds flown by
/// `controller`: the armed red at (6, 8), the unarmed one at (6, 10), and the
/// seated, unarmed blue at (9, 8) they can both see — **with a ten-cell sight
/// in place of five**. A view holds a contact's range in the keyframe's 50 m
/// steps (D-UM-4; the divergence `range_round`) and never under one step, and
/// a five-cell sight's `engage_cm` is 35 m: on the reflex's own rig the stock
/// mind would approach for ever and never be inside its band. Ten cells make
/// the band 70 m, past one step, and change nothing else of the scene.
fn the_j_field(controller: Controller) -> Arena {
    let seeing = |mut unit: CustodyUnit| {
        unit.sensor = Some(SensorProfile {
            max_range_cells: 10,
            ..sight()
        });
        unit
    };
    arena_of(
        vec![
            seeing(tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                controller,
            )),
            seeing(tank(RED2, Team::Red, centre(6, 10), Vec::new(), controller)),
            seeing(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            )),
        ],
        &[BLUE],
    )
}

/// The (j) field with both reds stock minds, run 30 s under `drive`.
fn stock_minds(drive: Drive) -> (Run, Vec<Kept>) {
    let mut arena = the_j_field(Controller::Mind);
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let run = run(&mut arena, drive, 30_000);
    (run, stock.kept())
}

/// **The gate's twin fixture: two T1 units at three cadences** (doc 17 §22's
/// M1a gate; D-UM-21; 16b §11's mandatory cross-tempo property). The (j)
/// field of `tests/reflex.rs`, its two reds flown by the native stock mind —
/// the armed one finds, classifies, locks and shoots the seated blue; the
/// unarmed one starts and drives — replayed for 30 s at 5,000, 1,000 and
/// 500 ms and on the task's own wakes at the three periods: one event list,
/// one per-substep hash sequence and one sequence of thinks (each view and
/// each answer). Every machine `startup`, `locked` and `fired` line is
/// stamped one substep after a lattice think, never on it; every answer
/// holds the emission rules; and the recording runner is transparent — the
/// arena's own default runner journals the same lines and hashes.
#[test]
fn m1a6_two_stock_minds_at_three_cadences_are_one_event_list_and_one_hash_sequence() {
    let (first, kept) = stock_minds(Drive::Every(5_000));
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
    for want in ["detected", "classified"] {
        assert!(
            words.iter().any(|(word, _)| word == want),
            "the field never journalled a {want}"
        );
    }
    assert!(
        words
            .iter()
            .any(|(word, unit)| word == "startup" && *unit == Some(RED2.0)),
        "the unarmed mind started"
    );
    let mut stamped = 0_usize;
    for (t_ms, json, _) in &first.events {
        let (word, unit) = kind_and_unit(json);
        if !matches!(word.as_str(), "startup" | "locked" | "fired") {
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
    assert!(stamped >= 3, "the minds stamped {stamped} actions");
    assert!(kept.len() >= 50, "{} thinks", kept.len());
    for (_, _, _, result) in &kept {
        stock_emission_holds(&result.intents);
    }
    assert!(
        kept.iter().any(|(unit, _, _, result)| *unit == RED2
            && result
                .intents
                .iter()
                .any(|kind| matches!(kind, IntentKind::Goal(MotorGoal::Cell { .. })))),
        "the unarmed mind drives"
    );
    for drive in [
        Drive::Every(1_000),
        Drive::Every(500),
        Drive::Task(5_000),
        Drive::Task(1_000),
        Drive::Task(500),
    ] {
        let (other, other_kept) = stock_minds(drive);
        assert_eq!(
            other.events, first.events,
            "the event list diverged at {drive:?}"
        );
        assert_eq!(
            other.hashes, first.hashes,
            "the hash sequence diverged at {drive:?}"
        );
        assert_eq!(other_kept, kept, "a think diverged at {drive:?}");
        assert_ne!(other.pulses, first.pulses, "{drive:?} ran the same pulses");
    }
    // The default runner is the stock mind: the same bytes, unrecorded.
    let mut plain = the_j_field(Controller::Mind);
    assert!(format!("{:?}", plain.mind_runner).contains("NativeStockMind"));
    let unrecorded = run(&mut plain, Drive::Every(5_000), 30_000);
    assert_eq!(unrecorded.events, first.events);
    assert_eq!(unrecorded.hashes, first.hashes);
}

/// The unarmed field: a weaponless stock mind at (6, 8), a seated blue it
/// can see at (9, 8); a `HOLD` lands at 10,000 for 10 s and a `WITHDRAW` at
/// 20,000 for 20 s, both delivered by the first pulse to reach their
/// millisecond (multiples of 5,000, so one script at every cadence). Run to
/// 40,000 at `cadence_ms`.
fn unarmed_run(cadence_ms: i64) -> (Run, Vec<Kept>, Arena) {
    let mut arena = arena_of(
        vec![
            tank(RED, Team::Red, centre(6, 8), Vec::new(), Controller::Mind),
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
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let beats = [
        (10_000_i64, Objective::Hold, 10_000_u32),
        (20_000, Objective::Withdraw, 20_000),
    ];
    let mut next = 0_usize;
    let mut seq = 0_u64;
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut counters = ReflexCounters::default();
    let mut pulses = 0_u32;
    let mut wall = 0_i64;
    loop {
        let mut intents: Vec<Intent> = Vec::new();
        while let Some((at, objective, ttl_ms)) = beats.get(next) {
            if *at > arena.now_ms {
                break;
            }
            seq += 1;
            intents.push(intent(
                &arena,
                RED,
                IntentKind::Order(Box::new(order(RED_ONE, *objective, None, *ttl_ms))),
                seq,
            ));
            next += 1;
        }
        let out = arena.step(wall, &[], &intents);
        pulses += 1;
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        counters.absorb(&out.reflex);
        hashes.extend(arena.substep_hashes.iter().copied());
        if arena.now_ms >= 40_000 || out.ended.is_some() {
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
        stock.kept(),
        arena,
    )
}

/// **T1 runs an unarmed hull with its fire half absent, and a fixture proves
/// it** (D-UM-48; doc 17 §22's italic line; Q-UM-12's default). A weaponless
/// stock mind with a hostile in its sight starts up, walks the graph, holds
/// under a commander's `HOLD` (`Goal(Hold)`, the state `patrol`), withdraws
/// under `WITHDRAW` (a goal in its own entry zone, the state `withdraw`), and
/// never emits a `Lock` or a `Fire` — over every recorded answer, so it is not
/// the landing's `channel` refusal that keeps it quiet — nor journals a
/// `locked` or `fired` line; its view carries no weapon row, no odds and no
/// `reads_dead`. At 5,000, 1,000 and 500 ms: one event list, one hash
/// sequence, one sequence of thinks.
#[test]
fn m1a6_an_unarmed_stock_mind_drives_senses_holds_and_never_calls_fire() {
    let (first, kept, arena) = unarmed_run(5_000);
    let start = (6_u16, 8_u16);
    let words: Vec<(i64, String, Option<u32>)> = first
        .events
        .iter()
        .map(|(t_ms, json, _)| {
            let (word, unit) = kind_and_unit(json);
            (*t_ms, word, unit)
        })
        .collect();
    assert!(
        words
            .iter()
            .any(|(_, word, unit)| word == "startup" && *unit == Some(RED.0))
    );
    assert!(
        words.iter().any(|(_, word, _)| word == "classified"),
        "it senses"
    );
    assert!(
        !words.iter().any(
            |(_, word, unit)| matches!(word.as_str(), "locked" | "fired") && *unit == Some(RED.0)
        ),
        "{words:?}"
    );
    assert_eq!(
        words
            .iter()
            .filter(|(_, word, unit)| word == "ordered" && *unit == Some(RED.0))
            .count(),
        2
    );
    let state_of = |result: &ThinkResult| {
        tacsim::arena::stock_mind::StockMemory::from(&tacsim::arena::mind::decode_memory(
            &result.memory,
        ))
        .state
    };
    assert!(
        kept.iter().any(|(_, t_ms, view, result)| *t_ms < 10_000
            && view.cell != Some(start)
            && result
                .intents
                .iter()
                .any(|kind| matches!(kind, IntentKind::Goal(MotorGoal::Cell { .. })))),
        "it patrols before the order"
    );
    assert!(
        kept.iter()
            .any(|(_, t_ms, view, result)| (10_000..20_000).contains(t_ms)
                && view
                    .order
                    .as_ref()
                    .is_some_and(|order| order.objective == MindObjective::Hold)
                && result.intents.first() == Some(&IntentKind::Goal(MotorGoal::Hold))
                && state_of(result) == ReflexState::Patrol),
        "it holds under HOLD"
    );
    assert!(
        kept.iter().any(|(_, t_ms, view, result)| *t_ms >= 20_000
            && view
                .order
                .as_ref()
                .is_some_and(|order| order.objective == MindObjective::Withdraw)
            && state_of(result) == ReflexState::Withdraw
            && result.intents.iter().any(|kind| matches!(
                kind,
                IntentKind::Goal(MotorGoal::Cell { x, .. }) if *x <= 1
            ))),
        "it withdraws under WITHDRAW, to its own entry zone"
    );
    assert!(
        arena
            .unit(RED)
            .and_then(|unit| arena.definition.terrain.cell_of(&unit.pose))
            .is_some_and(|cell| cell.0 < start.0),
        "and it went west"
    );
    for (_, _, view, result) in &kept {
        stock_emission_holds(&result.intents);
        assert!(
            !result
                .intents
                .iter()
                .any(|kind| matches!(kind, IntentKind::Lock { .. } | IntentKind::Fire { .. })),
            "{:?}",
            result.intents
        );
        assert!(view.weapons.is_empty());
        assert!(
            view.contacts
                .iter()
                .all(|row| row.odds.is_empty() && row.reads_dead.is_none())
        );
    }
    for cadence in [1_000_i64, 500] {
        let (other, other_kept, _) = unarmed_run(cadence);
        assert_eq!(other.events, first.events, "diverged at {cadence} ms");
        assert_eq!(other.hashes, first.hashes, "diverged at {cadence} ms");
        assert_eq!(other_kept, kept, "a think diverged at {cadence} ms");
    }
}

/// One red's helm after one pulse of the first-window check.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Helm {
    t_ms: i64,
    unit: u32,
    heading_mdeg: u32,
    speed_cms: i32,
    cell: Option<(u16, u16)>,
    pose: (i64, i64),
}

/// **The oracle check on the first decide window** (doc 17 §17: T1 is
/// Standard Doctrine re-expressed). The (j) field run 5 s once as T0 (the
/// reflex) and once as T1 (the stock mind), one substep a pulse: the
/// machines' `startup` lines are the same units in the same order, T0's
/// stamped at its staggered tick and T1's one substep after its lattice think
/// — the two scheduling changes of doc 17 §17, the `landing` divergence — and,
/// the two grids coinciding on this field, every action T1 lands (`startup`,
/// `locked`, `fired`) is T0's staged one exactly one substep later, with no
/// first divergence among them. **The helm is compared after every landing**
/// (T1's at `t` against T0's at `t - SUBSTEP`): it parts, at the first drive,
/// and the counter that names it is `landing` — the startup's late landing
/// costs T1 a whole think before its first goal, so T1's speed is always one
/// T0 held at most one think earlier and each red first drives within one
/// think of T0's, and the motor then steers a lagging pose; the armed red's
/// stand faces the view's whole degrees (`deg_round`'s facing half). The
/// first millisecond, the lags and the end helms are printed
/// (`stock_window: …`, `stock_window_helm: …`) for the ledger.
#[test]
fn m1a6_the_stock_mind_is_t0_on_the_first_decide_window() {
    // One substep a pulse, so the helm is read after every landing.
    let lines = |controller: Controller| {
        let mut arena = the_j_field(controller);
        let mut helm: Vec<Helm> = Vec::new();
        let mut wall = arena.now_ms;
        let mut bytes: Vec<u8> = Vec::new();
        loop {
            let out = arena.step(wall, &[], &[]);
            for chunk in &out.ledger_chunks {
                bytes.extend_from_slice(&chunk.bytes);
            }
            for id in [RED, RED2] {
                if let Some(unit) = arena.unit(id) {
                    helm.push(Helm {
                        t_ms: arena.now_ms,
                        unit: id.0,
                        heading_mdeg: unit.desired_heading_mdeg,
                        speed_cms: unit.desired_speed_cms,
                        cell: arena.definition.terrain.cell_of(&unit.pose),
                        pose: (unit.pose.x_cm, unit.pose.y_cm),
                    });
                }
            }
            if arena.now_ms >= 5_000 || out.ended.is_some() {
                break;
            }
            wall += SUBSTEP;
        }
        let actions: Vec<(i64, String, u32)> = events(&mut arena, &mut bytes)
            .into_iter()
            .filter_map(|event| {
                let json = serde_json::to_string(&event.kind).unwrap_or_default();
                let (word, unit) = kind_and_unit(&json);
                let unit = unit.filter(|unit| [RED.0, RED2.0].contains(unit))?;
                matches!(word.as_str(), "startup" | "locked" | "fired" | "ordered")
                    .then_some((event.t_ms, word, unit))
            })
            .collect();
        (actions, helm)
    };
    let (t0, t0_helm) = lines(Controller::Reflex);
    let (t1, t1_helm) = lines(Controller::Mind);
    let startups = |actions: &[(i64, String, u32)]| -> Vec<u32> {
        actions
            .iter()
            .filter(|(_, word, _)| word == "startup")
            .map(|(_, _, unit)| *unit)
            .collect()
    };
    assert_eq!(startups(&t0), startups(&t1), "{t0:?} / {t1:?}");
    assert_eq!(startups(&t1).len(), 2);
    let decide = i64::from(reflex_cfg().decide_ms);
    for (t_ms, word, unit) in &t0 {
        if word == "startup" {
            assert_eq!(
                t_ms.rem_euclid(decide),
                stagger_ms(SEED, ObjId(*unit), decide)
            );
        }
    }
    for (t_ms, word, unit) in &t1 {
        if word == "startup" {
            let think = stagger_ms(SEED, ObjId(*unit), STOCK_THINK_MS);
            assert_eq!((t_ms - SUBSTEP - think).rem_euclid(STOCK_THINK_MS), 0);
        }
    }
    // On this field the two grids coincide — each red's stagger is the same
    // millisecond at 500 and at 1,000 — so T1's landed actions are T0's staged
    // ones shifted by exactly one substep, every one of them.
    for unit in [RED, RED2] {
        assert_eq!(
            stagger_ms(SEED, unit, decide),
            stagger_ms(SEED, unit, STOCK_THINK_MS),
            "the precondition"
        );
    }
    let shifted: Vec<(i64, String, u32)> = t0
        .iter()
        .map(|(t_ms, word, unit)| (t_ms + SUBSTEP, word.clone(), *unit))
        .collect();
    assert_eq!(t1, shifted, "T1 is T0 one substep later");
    assert!(
        t1.iter().any(|(_, word, _)| word == "fired"),
        "the window holds a shot"
    );
    // The helm after every landing: T1's at `t` against T0's at `t - SUBSTEP`
    // (the `landing` shift), unit by unit, and the first millisecond they part.
    let at = |helm: &[Helm], t_ms: i64, unit: u32| {
        helm.iter()
            .find(|row| row.t_ms == t_ms && row.unit == unit)
            .cloned()
    };
    let apart: Vec<(Helm, Helm)> = t1_helm
        .iter()
        .filter_map(|row| {
            let twin = at(&t0_helm, row.t_ms - SUBSTEP, row.unit)?;
            ((twin.heading_mdeg, twin.speed_cms) != (row.heading_mdeg, row.speed_cms))
                .then(|| (twin, row.clone()))
        })
        .collect();
    let first_apart = t1
        .iter()
        .zip(shifted.iter())
        .find(|(a, b)| a != b)
        .map(|(a, b)| a.0.min(b.0));
    assert_eq!(first_apart, None, "the actions never part under the shift");
    // **The helm parts, and the counter that names it is `landing`.** The
    // startup lands one substep after the think, so power comes online one
    // substep later than T0's; T0's next tick drives at once, T1's first goal
    // waits for the next lattice think and lands a substep after that — a
    // whole think behind T0's first drive. From there the motor steers each
    // red from a pose that lags T0's, so the headings part by the motor's
    // steering of a lagging pose, never by a different decision: each red's
    // desired speed after every landing is one T0 held at most one think
    // (four substeps) earlier.
    let Some((t0_first, t1_first)) = apart.first() else {
        panic!("the reds drive, so the lagging helm parts somewhere in 5 s");
    };
    assert_eq!(t1_first.speed_cms, 0, "T1 still parked: {t1_first:?}");
    assert_ne!(t0_first.speed_cms, 0, "T0 already driving: {t0_first:?}");
    for row in &t1_helm {
        assert!(
            // (Before the first pulse the hull is parked.)
            (1..=4).any(|k| at(&t0_helm, row.t_ms - k * SUBSTEP, row.unit)
                .map_or(0, |twin| twin.speed_cms)
                == row.speed_cms),
            "T1's speed is T0's of at most one think before: {row:?}"
        );
    }
    let first_drive = |helm: &[Helm], unit: ObjId| {
        helm.iter()
            .find(|row| row.unit == unit.0 && row.speed_cms != 0)
            .map(|row| row.t_ms)
    };
    for unit in [RED, RED2] {
        let (Some(t0_drive), Some(t1_drive)) =
            (first_drive(&t0_helm, unit), first_drive(&t1_helm, unit))
        else {
            panic!("#{} drives in both runs", unit.0);
        };
        assert!(
            t1_drive > t0_drive && t1_drive - t0_drive <= STOCK_THINK_MS,
            "#{}: T1 first drives {t1_drive}, T0 {t0_drive}",
            unit.0
        );
        println!(
            "stock_window_helm: unit={} t0_first_drive_ms={t0_drive} t1_first_drive_ms={t1_drive} \
             lag_ms={}",
            unit.0,
            t1_drive - t0_drive
        );
    }
    // And at the window's end the armed red stands facing the blue in both,
    // T1 on the view's whole degrees (`deg_round`'s facing half) and from a
    // pose that lagged (`landing`).
    let last = |helm: &[Helm]| {
        helm.iter()
            .rev()
            .find(|row| row.unit == RED.0)
            .cloned()
            .unwrap_or_else(|| panic!("the armed red"))
    };
    let (t0_end, t1_end) = (last(&t0_helm), last(&t1_helm));
    assert_eq!((t0_end.speed_cms, t1_end.speed_cms), (0, 0));
    assert_eq!(t1_end.heading_mdeg % 1_000, 0, "whole degrees: {t1_end:?}");
    println!(
        "stock_window: t0={t0:?} t1={t1:?} first_action_divergence_ms={first_apart:?} \
         first_helm_divergence_ms={} counter=landing t0_first={t0_first:?} t1_first={t1_first:?} \
         helm_apart_samples={} t0_end={t0_end:?} t1_end={t1_end:?}",
        t1_first.t_ms,
        apart.len()
    );
}

// ---------------------------------------------------------------------------
// 17b cut L.2: minds in a squad under a lance-leader (doc 17 §4 "Where a
// lance-leader sits", §22 L; D-UM-2, D-UM-15, D-UM-49, D-UM-53)
// ---------------------------------------------------------------------------

/// The lance field's third red: a mind with no sensor and no weapon, so it
/// holds no row of anything — the no-row case of a leader's hint.
const RED3: ObjId = ObjId(7);

/// The millisecond the lance-leader's order lands: a multiple of 5,000, so the
/// first pulse to reach it is the same millisecond at every cadence.
const L2_ORDER_MS: i64 = 5_000;

/// The (j) field's cannon at one point a round, so the seated blue is never
/// destroyed inside a run and every row of it stays live.
fn pea_shooter() -> WeaponSummary {
    WeaponSummary {
        damage: 1,
        ..cannon()
    }
}

/// **The lance field**: [`the_j_field`]'s three tanks with both reds minds —
/// the armed one (a [`pea_shooter`]) at (6, 8), the unarmed one at (6, 10),
/// each with the ten-cell sight — a third red mind ([`RED3`]) at (1, 1) with
/// no sensor and no weapon, and the seated blue at (9, 8).
fn lance_field() -> Arena {
    let seeing = |mut unit: CustodyUnit| {
        unit.sensor = Some(SensorProfile {
            max_range_cells: 10,
            ..sight()
        });
        unit
    };
    let mut blind = tank(RED3, Team::Red, centre(1, 1), Vec::new(), Controller::Mind);
    blind.sensor = None;
    arena_of(
        vec![
            seeing(tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![pea_shooter()],
                Controller::Mind,
            )),
            seeing(tank(
                RED2,
                Team::Red,
                centre(6, 10),
                Vec::new(),
                Controller::Mind,
            )),
            blind,
            seeing(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            )),
        ],
        &[BLUE],
    )
}

/// What one lance run produced: the run itself, each red's row for the blue
/// (its id and class) the moment the order landed, each red's held order
/// right after, and every squad signal the pulses raised.
struct LanceRun {
    run: Run,
    rows_at_order: Vec<(ObjId, Option<ContactId>, Option<Classification>)>,
    held: Vec<(ObjId, Option<FrameOrder>)>,
    signals: Vec<SquadSignal>,
}

/// The lance field run 30 s at a fixed `cadence_ms`, with **the lance-leader's
/// order** landing at [`L2_ORDER_MS`]: red-1's persona (`issuer: None`, `by:
/// Persona`), `ENGAGE` with the hint written in the commander's scope (the
/// commander's own id for the blue), applied the way the bridge fans it —
/// one `IntentKind::Order` per roster unit through `apply_intent` with
/// `IntentSource::Seat`, as `deliberation.rs`' `issue` does — standing 20 s,
/// so it also runs out inside the run.
fn lance_run(cadence_ms: i64) -> LanceRun {
    let mut arena = lance_field();
    let hint = draw_contact_id(SEED, RED, BLUE);
    let order = Order {
        squad: RED_ONE,
        objective: Objective::Engage {
            target_hint: Some(hint),
        },
        target: None,
        horizon_ms: 20_000,
        constraints: Constraints::default(),
        ttl_ms: 20_000,
        issued_ms: 0,
        issuer: None,
        by: CharacterRef::Persona { squad: RED_ONE },
        spot: None,
    };
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut counters = ReflexCounters::default();
    let mut pulses = 0_u32;
    let mut wall = 0_i64;
    let mut signals: Vec<SquadSignal> = Vec::new();
    let mut rows_at_order = Vec::new();
    let mut held = Vec::new();
    let mut ordered = false;
    loop {
        if !ordered && arena.now_ms >= L2_ORDER_MS {
            ordered = true;
            rows_at_order = [RED, RED2, RED3]
                .into_iter()
                .map(|unit| {
                    let id = arena.contacts.id_of(unit, BLUE);
                    let class = id
                        .and_then(|id| arena.contacts.resolve(unit, id))
                        .map(|row| row.class);
                    (unit, id, class)
                })
                .collect();
            let roster: Vec<ObjId> = arena
                .squad_of(RED)
                .map(|roster| roster.units.clone())
                .unwrap_or_default();
            for (seq, unit) in (1_u64..).zip(&roster) {
                let told = intent(
                    &arena,
                    *unit,
                    IntentKind::Order(Box::new(order.clone())),
                    seq,
                );
                assert_eq!(
                    apply_intent(&mut arena, &told, IntentSource::Seat),
                    IntentOutcome::AppliedContinuous,
                    "#{} takes the leader's order",
                    unit.0
                );
            }
            held = roster
                .iter()
                .map(|unit| (*unit, arena.frame_order(*unit)))
                .collect();
        }
        let out = arena.step(wall, &[], &[]);
        pulses += 1;
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        counters.absorb(&out.reflex);
        hashes.extend(arena.substep_hashes.iter().copied());
        signals.extend(out.deliberation);
        if arena.now_ms >= 30_000 || out.ended.is_some() {
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
    LanceRun {
        run: Run {
            events,
            hashes,
            pulses,
            counters,
        },
        rows_at_order,
        held,
        signals,
    }
}

/// **17b cut L.2's mandatory cross-tempo property** (16b §11; doc 16 §3.1
/// D-16-2; doc 17 §22 L, D-UM-15, D-UM-53). Three minds of red are dealt into
/// one squad, the lowest id commanding — the dealing `Controller::is_machine`
/// widened to minds — and the lance-leader's `ENGAGE` lands on all three at
/// one millisecond. Before it lands the two that can see hold the blue as a
/// **classified** row (the classify window is driven first, so the re-key is
/// to an earned row; an unclassified held row is the lead's ruling still owed
/// and is not pinned here) and the third holds none. Then:
///
/// - each holding hull's `FrameOrder` is `Engage` re-keyed to **its own** row
///   — the commander's the hint itself, its wingman's a different id — and the
///   no-row hull's is `Engage { target: None }`;
/// - the three `ordered` lines carry one `t_ms`;
/// - every signal red-1 raises names the three minds, each at `Startup`: the
///   engine reads no mind's memory into another principal's brief (D-UM-49),
///   so the M1a.5 placeholder stands until 17b cut M4;
/// - at 5,000, 1,000 and 500 ms: one event list, one hash sequence, one set of
///   rows and held orders.
#[test]
fn l2_two_minds_are_one_squad_under_a_leader_at_three_cadences() {
    let arena = lance_field();
    let shape: Vec<(String, ObjId, Vec<ObjId>)> = arena
        .squads
        .iter()
        .map(|roster| {
            (
                roster.id.to_string(),
                roster.commander,
                roster.units.clone(),
            )
        })
        .collect();
    assert_eq!(
        shape,
        vec![("red-1".to_string(), RED, vec![RED, RED2, RED3])],
        "the minds are one squad, the lowest id commanding; the seated blue is in none"
    );

    let first = lance_run(5_000);
    let hint = draw_contact_id(SEED, RED, BLUE);
    let wingman_row = draw_contact_id(SEED, RED2, BLUE);
    let blue = Some(Classification::Classified { team: Team::Blue });
    assert_ne!(wingman_row, hint, "the precondition");
    assert_eq!(
        first.rows_at_order,
        vec![
            (RED, Some(hint), blue),
            (RED2, Some(wingman_row), blue),
            (RED3, None, None),
        ],
        "both seeing minds hold the blue classified before the order lands; the third holds \
         nothing"
    );
    let held: Vec<(ObjId, Option<MindObjective>)> = first
        .held
        .iter()
        .map(|(unit, order)| (*unit, order.as_ref().map(|order| order.objective)))
        .collect();
    assert_eq!(
        held,
        vec![
            (RED, Some(MindObjective::Engage { target: Some(hint) })),
            (
                RED2,
                Some(MindObjective::Engage {
                    target: Some(wingman_row)
                })
            ),
            (RED3, Some(MindObjective::Engage { target: None })),
        ],
        "each hull re-keyed to its own row, the no-row hull to none"
    );
    let ordered: Vec<(i64, Option<u32>)> = first
        .run
        .events
        .iter()
        .filter_map(|(t_ms, json, _)| {
            let (word, unit) = kind_and_unit(json);
            (word == "ordered").then_some((*t_ms, unit))
        })
        .collect();
    assert_eq!(
        ordered,
        vec![
            (L2_ORDER_MS, Some(RED.0)),
            (L2_ORDER_MS, Some(RED2.0)),
            (L2_ORDER_MS, Some(RED3.0)),
        ],
        "one ordered line a hull, at one millisecond"
    );
    let reds: Vec<&SquadSignal> = first
        .signals
        .iter()
        .filter(|signal| signal.squad == RED_ONE)
        .collect();
    assert!(!reds.is_empty(), "red-1 asks for orders");
    for signal in &reds {
        assert_eq!(signal.commander, RED);
        let members: Vec<(ObjId, ReflexState)> = signal
            .members
            .iter()
            .map(|member| (member.unit, member.state))
            .collect();
        assert_eq!(
            members,
            vec![
                (RED, ReflexState::Startup),
                (RED2, ReflexState::Startup),
                (RED3, ReflexState::Startup),
            ],
            "a mind member reads Startup: no mind's memory is read for the brief"
        );
    }
    for cadence in [1_000_i64, 500] {
        let other = lance_run(cadence);
        assert_eq!(
            other.run.events, first.run.events,
            "the event list diverged at {cadence} ms"
        );
        assert_eq!(
            other.run.hashes, first.run.hashes,
            "the hash sequence diverged at {cadence} ms"
        );
        assert_eq!(other.rows_at_order, first.rows_at_order, "{cadence} ms");
        assert_eq!(other.held, first.held, "{cadence} ms");
        assert_ne!(other.run.pulses, first.run.pulses, "{cadence} ms");
    }
}

/// **The charge field**: a mind of red at (1, 8) with one short gun — a single
/// 100 m band, one point a round — and a fifteen-cell sight; a seated red at
/// (1, 10) with the same sight and no gun; and the seated blue at (14, 8),
/// 130 m off — in sight, and outside every band the red mind has.
fn charge_field() -> Arena {
    let short = WeaponSummary {
        max_range_cm: 10_000,
        bands: vec![WeaponBand {
            max_range_cm: 10_000,
            dispersion_mrad: 3,
        }],
        ..pea_shooter()
    };
    let seeing = |mut unit: CustodyUnit| {
        unit.sensor = Some(SensorProfile {
            max_range_cells: 15,
            ..sight()
        });
        unit
    };
    arena_of(
        vec![
            seeing(tank(
                RED,
                Team::Red,
                centre(1, 8),
                vec![short],
                Controller::Mind,
            )),
            seeing(tank(
                RED2,
                Team::Red,
                centre(1, 10),
                Vec::new(),
                Controller::Pilot,
            )),
            seeing(tank(
                BLUE,
                Team::Blue,
                centre(14, 8),
                Vec::new(),
                Controller::Pilot,
            )),
        ],
        &[RED2, BLUE],
    )
}

/// How [`charge_run`] charges the red mind.
#[derive(Clone, Copy, Debug)]
enum Charge {
    /// `ORDER … SUPPORT` naming the mind itself, by red-1's lance-leader.
    SelfCharged,
    /// HEAD's re-key of it: a `Support` with no charge, written directly.
    Blind,
}

/// The charge field's red mind, run one substep a pulse to 40 s (or, with
/// `stop_at_fire`, to its first `fired` line), with the native stock mind
/// recorded: `charge` is applied before the first pulse — an intent through
/// the seat's path, or a `FrameOrder` written straight onto the mind.
fn charge_run(charge: Charge, stop_at_fire: bool) -> (Rig, Vec<Kept>) {
    let mut rig = Rig::new(charge_field());
    let stock = Stock::on(&rig.arena);
    rig.arena.mind_runner = stock.clone();
    let first: Vec<Intent> = match charge {
        Charge::SelfCharged => vec![intent(
            &rig.arena,
            RED,
            IntentKind::Order(Box::new(order(
                RED_ONE,
                Objective::Support { of: RED },
                None,
                600_000,
            ))),
            1,
        )],
        Charge::Blind => {
            rig.arena.minds.entry(RED).or_default().order = Some(FrameOrder {
                squad: SmolStr::new("red-1"),
                objective: MindObjective::Support { of: None },
                hold_fire: false,
                max_range_band: None,
                expires_ms: 600_000,
            });
            Vec::new()
        }
    };
    let _ = rig.pulse(&[], &first);
    while rig.arena.now_ms < 40_000 {
        let _ = rig.pulse(&[], &[]);
        if stop_at_fire && !rig.lines("fired", RED).is_empty() {
            break;
        }
    }
    let kept = stock.kept();
    (rig, kept)
}

/// **A self-charged `Support` fights on doctrine** (17b cut L.2; the plan's
/// Q-L6; D-UM-15; M1a.6's residual (3), reachable now that a mind's hull is in
/// a squad). `rekey_order` re-keys `Support { of: self }` through the
/// `Engage` arm's own resolution:
///
/// - unhinted, a mind holds `Engage { target: None }`, and with a classified
///   hostile in sight but outside every band its stock mind enters `Approach`
///   (its memory's state, decoded HERE in the test as the twin does — the
///   engine never decodes it), drives a `Goal(Cell)` at it with the held range
///   falling, and fires once in band;
/// - the contrast, HEAD's `Support { of: None }` written straight onto the
///   mind over the same field, stands where it is: no approach, no lock, no
///   `fired` line — the blind supporter;
/// - hinted, the self-charge is `Engage` on **the mind's own row** for the
///   hint's target (the hint written in the issuing pilot's scope); a
///   `Support` of another unit still re-keys to the charge's held row, or none.
#[test]
fn l2_a_self_charged_support_fights_on_doctrine() {
    let state_of = |result: &ThinkResult| {
        tacsim::arena::stock_mind::StockMemory::from(&tacsim::arena::mind::decode_memory(
            &result.memory,
        ))
        .state
    };
    let blue_row = draw_contact_id(SEED, RED, BLUE);
    // The held range of the blue on each red think that holds it classified.
    let ranges = |kept: &[Kept]| -> Vec<(i64, u64)> {
        kept.iter()
            .filter(|(unit, ..)| *unit == RED)
            .filter_map(|(_, t_ms, view, _)| {
                view.contacts
                    .iter()
                    .find(|row| {
                        row.id == blue_row
                            && row.class == Classification::Classified { team: Team::Blue }
                    })
                    .map(|row| (*t_ms, row.range_m))
            })
            .collect()
    };

    // (1) The self-charge, unhinted.
    let (mut charged, kept) = charge_run(Charge::SelfCharged, true);
    assert_eq!(
        kept.iter()
            .filter(|(unit, ..)| *unit == RED)
            .find_map(|(_, _, view, _)| view.order.as_ref().map(|order| order.objective)),
        Some(MindObjective::Engage { target: None }),
        "the self-charge is held as Engage, unhinted"
    );
    let seen = ranges(&kept);
    let Some(&(_, opening_m)) = seen.first() else {
        panic!("the red mind never held the blue classified");
    };
    assert!(
        opening_m > 100,
        "the blue is outside every band when first classified: {seen:?}"
    );
    assert!(
        kept.iter().any(|(unit, _, _, result)| *unit == RED
            && state_of(result) == ReflexState::Approach
            && result
                .intents
                .iter()
                .any(|kind| matches!(kind, IntentKind::Goal(MotorGoal::Cell { .. })))),
        "the self-charged mind approaches"
    );
    let closest = seen.iter().map(|(_, range_m)| *range_m).min();
    assert!(
        closest.is_some_and(|closest| closest <= 100 && closest < opening_m),
        "the held range falls into the band: {seen:?}"
    );
    let fired = charged.lines("fired", RED);
    assert!(!fired.is_empty(), "and fires once in band");
    let first_approach = kept
        .iter()
        .find(|(unit, _, _, result)| *unit == RED && state_of(result) == ReflexState::Approach)
        .map(|(_, t_ms, ..)| *t_ms);
    assert!(
        first_approach.is_some_and(|approach| fired.iter().all(|t_ms| *t_ms > approach)),
        "it fired after it closed: {first_approach:?} {fired:?}"
    );
    let until_ms = charged.arena.now_ms;

    // (2) The contrast: HEAD's blind supporter over the same field and time.
    let (mut blind, blind_kept) = charge_run(Charge::Blind, false);
    assert!(
        !ranges(&blind_kept).is_empty(),
        "the blind supporter holds the same hostile"
    );
    assert!(
        blind_kept
            .iter()
            .filter(|(unit, t_ms, ..)| *unit == RED && *t_ms <= until_ms)
            .all(|(_, _, _, result)| state_of(result) != ReflexState::Approach),
        "a blind supporter never approaches"
    );
    assert!(blind.lines("fired", RED).is_empty(), "nor fires");
    assert!(blind.lines("locked", RED).is_empty(), "nor locks");
    assert_eq!(
        blind
            .arena
            .unit(RED)
            .and_then(|unit| blind.arena.definition.terrain.cell_of(&unit.pose)),
        Some((1, 8)),
        "it stands where it was"
    );

    // (3) Hinted, and a charge that is another unit.
    let mut rig = Rig::new(charge_field());
    rig.arena.mind_runner = ScriptedMind::new(&[]);
    let _ = rig.until(10_000, |arena| {
        arena.contacts.id_of(RED, BLUE).is_some()
            && arena.contacts.id_of(RED2, BLUE).is_some()
            && arena.contacts.id_of(RED, RED2).is_some()
    });
    let pilot_says_blue = draw_contact_id(SEED, RED2, BLUE);
    let mind_holds_blue = rig.arena.contacts.id_of(RED, BLUE);
    assert_ne!(mind_holds_blue, Some(pilot_says_blue), "the precondition");
    let held = |rig: &Rig| rig.arena.frame_order(RED).map(|order| order.objective);
    let hinted = intent(
        &rig.arena,
        RED,
        IntentKind::Order(Box::new(Order {
            target: Some(pilot_says_blue),
            ..order(RED_ONE, Objective::Support { of: RED }, Some(RED2), 60_000)
        })),
        1,
    );
    let _ = rig.pulse(&[], &[hinted]);
    assert_eq!(
        held(&rig),
        Some(MindObjective::Engage {
            target: mind_holds_blue
        }),
        "a hinted self-charge is Engage on the mind's own row"
    );
    let teammate = intent(
        &rig.arena,
        RED,
        IntentKind::Order(Box::new(order(
            RED_ONE,
            Objective::Support { of: RED2 },
            Some(RED2),
            60_000,
        ))),
        2,
    );
    let _ = rig.pulse(&[], &[teammate]);
    assert_eq!(
        held(&rig),
        Some(MindObjective::Support {
            of: rig.arena.contacts.id_of(RED, RED2)
        }),
        "a Support of another unit: the charge's held row"
    );
    let stranger = intent(
        &rig.arena,
        RED,
        IntentKind::Order(Box::new(order(
            RED_ONE,
            Objective::Support { of: ObjId(77) },
            Some(RED2),
            60_000,
        ))),
        3,
    );
    let _ = rig.pulse(&[], &[stranger]);
    assert_eq!(
        held(&rig),
        Some(MindObjective::Support { of: None }),
        "a charge the mind holds nothing of: none"
    );
}

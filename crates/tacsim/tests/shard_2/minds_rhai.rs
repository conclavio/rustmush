//! **A unit mind written in Rhai, driven** (17b cut M2.4; doc 17 §3, §4, §6,
//! §7, §8, §14, §22's M2 paragraph; D-UM-1, D-UM-4, D-UM-9, D-UM-26,
//! D-UM-29, D-UM-31, D-UM-32).
//!
//! `crates/tacsim/src/minds/runner.rs`' [`RhaiRunner`] is a `MindRunner`
//! over `mush_mind`'s sandbox and the TacSim API (17b cut M2.3), run
//! synchronously at the think point exactly as the native stock mind is.
//! This file sets it on a live arena — the (j) field of `tests/minds.rs`, its
//! rig **copied** here (an integration test cannot import another; the
//! sixteen-cell window, the ten-cell sight, the armed red at (6, 8), the
//! unarmed red at (6, 10), the seated blue at (9, 8) presenting a silhouette)
//! — and drives every think through the arena's own think arm
//! (`Arena::mind_think`) and landing:
//!
//! 1. every getter of every type against the keyframe `status_frame_held`
//!    builds for the same hull, through M2.3's `PARITY` table row by row;
//! 2. every station function landing through the one door in call order,
//!    the trace in no ledger line, the ungranted fire refused `channel` at the
//!    landing with no strike;
//! 3. every fault word with its fixture, each landing nothing, keeping the
//!    memory and counting a strike, and three running dropping the hull to
//!    the native stand-in — with the hash equal, substep by substep, to M2.2's
//!    seventeen-intent baseline;
//! 4. every plot kernel holding the plot while the truth moves, each equal to
//!    its one free function, and `ranges`' order arm both ways;
//! 5. `rand(k)` on the arena's seed under the sixth label;
//! 6. the card type reading the card the arena holds;
//! 7. **the mandatory cross-tempo fixture** (16b §11): a scripted Rhai mind
//!    driving a square, locking and tracing — one event list, one hash
//!    sequence and one trace sequence at 5,000, 1,000 and 500 ms;
//!
//! and that `default_mind_runner` is still the native stock mind: nothing of
//! production names the Rhai runner in this cut.
//!
//! **17b cut M2.7** adds the cross-platform pin (D-UM-28):
//! `m2_the_twin_corpus_hashes_the_same_on_every_host`, `#[ignore]`d and run
//! by name by a step of `ci.yml`'s `test` job on both hosts — the stock
//! corpus flown by the Rhai Standard Doctrine alone and a float-probe mind,
//! each match's digests pinned on the Windows blessing host.
//!
//! Recording a runner's answers (`Recorded`) and replaying them
//! (`ScriptedMind`) are inputs to `Arena::step`, as a scripted intent log is,
//! never a stand-in for a production module (16b §0.4).

/// **17b cut M2.7**: the shipped-scale rig, for the cross-platform pin's
/// stock corpus (`support::play` on the proving ground).
use crate::support;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use mush_core::{DefinitionDigest, DefinitionId, DefinitionKey, DefinitionKind, ObjId, OwnerId};
use mush_mind::rhai::{Engine, INT};
use mush_mind::{Digest, EngineLimits, Sandbox};
use mush_sim::data::Pose;
use mush_tactics::{
    CellSpec, DefinitionRef, FacetId, FacetSpec, RulesValue, SectionId, SectionSpec, TacticalUnit,
    UnitSpec,
};
use serde_json::Value as Json;
use sha2::{Digest as _, Sha256};
use smol_str::SmolStr;
use tacsim::arena::CardChange;
use tacsim::arena::clock::Tempo;
use tacsim::arena::conditions::{ASPECTS_ANY, Arming, FireCondition, SlotTarget};
use tacsim::arena::index::range_between;
use tacsim::arena::intents::{IntentKind, MotorGoal, Refusal};
use tacsim::arena::ledger::LedgerEvent;
use tacsim::arena::mind::{
    MIND_INTENTS_MAX, MIND_STRIKES_TO_DROP, MemValue, Memory, MindRunner, MindView, STOCK_THINK_MS,
    ThinkJob, ThinkResult, decode_memory,
};
use tacsim::arena::motor;
use tacsim::arena::motor::MotorCounters;
use tacsim::arena::perception::{
    FrameNotice, FrameOrder, Jammer, MindObjective, Named, Receiver, SensorProfile,
    SignatureFactor, bearing_of,
};
use tacsim::arena::resolution::{BandProfile, SectionProfile, UnitProfile};
use tacsim::arena::rng::{DrawLabel, draw, stagger_ms};
use tacsim::arena::snapshot::Snapshot;
use tacsim::arena::state::{
    Arena, ArenaDefinition, ArenaTerrain, Channel, Controller, CostRow, CustodyOut, CustodyUnit,
    DueKind, EntryZone, HeatState, KeySpot, Pilot, Team, TerrainCell, TrackedRules, Waypoint,
    Waypoints, WeaponSummary,
};
use tacsim::arena::step::{Frame, StepOutput};
use tacsim::arena::stock_mind::{
    NativeStockMind, Weights, best_hostile, cell_point, health_permille, line_to, view_ranges,
};
use tacsim::arena::weapons::WeaponBand;
use tacsim::arena_task::MatchId;
use tacsim::config::{ArenaConfig, MatchConfig, ReflexConfig, SensorsConfig};
use tacsim::deliberation::order::RangeBand;
use tacsim::match_::cockpit::status_frame_held;
use tacsim::minds::api::{PARITY, contact_int};
use tacsim::minds::stations::UNGRANTED_FIRE;
use tacsim::minds::{
    DROPPED_LINE, RhaiRunner, STOCK_OPS_PER_THINK, ThinkStat, register_api, standard_sandbox,
};

/// The window's side, cells.
const SIDE: u16 = 16;
/// One cell, centimetres.
const CELL_CM: u16 = 1_000;
/// The match seed — `tests/reflex.rs`' own.
const SEED: u64 = 2_350;
/// The substep this rig cuts.
const SUBSTEP: i64 = 250;
/// The armed mind.
const RED: ObjId = ObjId(4);
/// The unarmed mind.
const RED2: ObjId = ObjId(6);
/// The pilot both reds can see.
const BLUE: ObjId = ObjId(9);

// ---------------------------------------------------------------------------
// The rig (`tests/minds.rs`' (j) field, copied)
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

/// `tests/reflex.rs`' eighteen-point tank: one hull section, one front facet
/// — and, with `heat`, a heat cell on the hull (`tests/reflex.rs`' `give_heat`
/// shape).
fn tactical_unit(id: ObjId, heat: Option<(i64, i64)>) -> TacticalUnit {
    let mut cells = vec![CellSpec {
        id: mush_tactics::CellId::new("internals").expect("a cell"),
        min: 0,
        max: 8,
        init: 8,
    }];
    if let Some((now, max)) = heat {
        cells.push(CellSpec {
            id: mush_tactics::CellId::new(tacsim::definitions::unit::HEAT_CELL).expect("a cell"),
            min: 0,
            max,
            init: now,
        });
    }
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
            cells,
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

/// A tank that sees, with `weapons` and `controller`.
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
        profile: UnitProfile::default(),
        gunnery: 0,
        controller,
        slot: 0,
        unit: tactical_unit(id, None),
        seal: tacsim::arena::MindSeal::placeholder(),
    }
    .sealed()
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
fn arena_on(units: Vec<CustodyUnit>, seated: &[ObjId], reflex: ReflexConfig) -> Arena {
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
            reflex,
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

/// A target every band of which presents a silhouette of hull, non-lethal.
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
        sections: vec![SectionProfile {
            id: SmolStr::new("hull"),
            facets: vec![SmolStr::new("front")],
            consequences: Vec::new(),
            struck_through: None,
        }],
    }
}

/// `unit` presenting a silhouette.
fn presenting(unit: CustodyUnit) -> CustodyUnit {
    CustodyUnit {
        profile: every_band_profile(),
        ..unit
    }
    .sealed()
}

/// **The (j) field** (`tests/minds.rs`' `the_j_field`): the armed red at
/// (6, 8), the unarmed red at (6, 10), both flown by `controller` with a
/// ten-cell sight, and the seated blue at (9, 8) presenting a silhouette.
fn the_j_field(controller: Controller) -> Arena {
    let seeing = |mut unit: CustodyUnit| {
        unit.sensor = Some(SensorProfile {
            max_range_cells: 10,
            ..sight()
        });
        unit
    };
    arena_on(
        vec![
            seeing(tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                controller,
            )),
            seeing(tank(RED2, Team::Red, centre(6, 10), Vec::new(), controller)),
            presenting(seeing(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            ))),
        ],
        &[BLUE],
        reflex_cfg(),
    )
}

/// The lattice millisecond of `unit`'s think `n`.
fn think_at(unit: ObjId, n: i64) -> i64 {
    stagger_ms(SEED, unit, STOCK_THINK_MS) + n * STOCK_THINK_MS
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

/// The kind word and the unit of a journalled line.
fn kind_and_unit(json: &str) -> (String, Option<u32>) {
    let value: Json = serde_json::from_str(json).unwrap_or_default();
    (
        value
            .get("kind")
            .and_then(Json::as_str)
            .unwrap_or_default()
            .to_owned(),
        value
            .get("unit")
            .and_then(Json::as_u64)
            .and_then(|unit| u32::try_from(unit).ok()),
    )
}

/// How a run is driven: a fixed wall cadence, or the task's own wakes with the
/// `Quiet` period set to the figure given (`tests/reflex.rs`' `Drive`).
#[derive(Clone, Copy, Debug)]
enum Drive {
    Every(i64),
    Task(u32),
}

/// What one run produced: every ledger line as `(t_ms, json, hash)`, every
/// substep hash (accumulated after every step), the pulses and the counters.
#[derive(Debug)]
struct Run {
    events: Vec<(i64, String, u64)>,
    draws: Vec<(i64, Vec<(String, u64)>)>,
    hashes: Vec<u64>,
    pulses: u32,
    counters: MotorCounters,
}

/// Step `arena` to `until_ms` under `drive`.
fn run(arena: &mut Arena, drive: Drive, until_ms: i64) -> Run {
    if let Drive::Task(period_ms) = drive {
        arena.arena_cfg.tempo.quiet_ms = period_ms;
    }
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut counters = MotorCounters::default();
    let mut wall = arena.now_ms;
    let mut pulses = 0_u32;
    loop {
        let out = arena.step(wall, &[], &[]);
        pulses = pulses.saturating_add(1);
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        counters.absorb(&out.motor);
        hashes.extend(arena.substep_hashes.iter().copied());
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        wall = match drive {
            Drive::Every(cadence_ms) => wall + cadence_ms,
            Drive::Task(_) => out.next_due_ms,
        };
    }
    let journal = events(arena, &mut bytes);
    let draws = journal
        .iter()
        .filter(|event| !event.draws.is_empty())
        .map(|event| (event.t_ms, event.draws.clone()))
        .collect();
    let events = journal
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
        draws,
        hashes,
        pulses,
        counters,
    }
}

/// **A rig driven one substep a pulse**: the arena, the wall it was last
/// stepped at, every ledger byte its pulses handed out, the substep hashes of
/// every pulse and the counters.
struct Rig {
    arena: Arena,
    wall: Option<i64>,
    bytes: Vec<u8>,
    hashes: Vec<Vec<u64>>,
    counters: MotorCounters,
}

impl Rig {
    fn new(arena: Arena) -> Rig {
        Rig {
            arena,
            wall: None,
            bytes: Vec::new(),
            hashes: Vec::new(),
            counters: MotorCounters::default(),
        }
    }

    /// One pulse of one substep.
    fn pulse(&mut self) -> StepOutput {
        let wall = self.wall.map_or(0, |wall| wall + SUBSTEP);
        self.wall = Some(wall);
        // 17b cut M4.1: settled between pulses, as the task's pool settles.
        let out = self.arena.step_settled(wall, &[], &[]);
        for chunk in &out.ledger_chunks {
            self.bytes.extend_from_slice(&chunk.bytes);
        }
        self.hashes.push(self.arena.substep_hashes.clone());
        self.counters.absorb(&out.motor);
        out
    }

    /// Pulse until arena time reaches `t_ms`.
    fn to(&mut self, t_ms: i64) {
        while self.arena.now_ms < t_ms {
            let _ = self.pulse();
        }
    }

    /// Every line journalled so far.
    fn journal(&mut self) -> Vec<LedgerEvent> {
        events(&mut self.arena, &mut self.bytes)
    }
}

// ---------------------------------------------------------------------------
// The Rhai runner, recorded; and a replay of recorded answers
// ---------------------------------------------------------------------------

/// The seal's digest of a source.
fn digest(source: &str) -> Digest {
    Digest(Sha256::digest(source.as_bytes()).into())
}

/// **A recording [`RhaiRunner`] for `source`** on the arena's doctrine and the
/// process's one sandbox.
fn rhai(arena: &Arena, source: &str) -> Arc<RhaiRunner> {
    Arc::new(
        RhaiRunner::new(&arena.arena_cfg, source, digest(source))
            .unwrap_or_else(|error| panic!("{source}: {error}"))
            .recording(),
    )
}

/// One think the recorder saw: the whole job and the answer.
#[derive(Clone, Debug)]
struct Kept {
    unit: ObjId,
    think_seq: u64,
    t_ms: i64,
    lands_ms: i64,
    seed: u64,
    dropped: bool,
    /// 17b cut M3.1: the row the think was held to.
    profile: tacsim::arena::core::CoreProfile,
    view: Arc<MindView>,
    memory: Vec<u8>,
    map: Arc<ArenaDefinition>,
    card: tacsim::arena::runtime_card::RuntimeCard,
    program: tacsim::arena::profile_rules::BoundProgram,
    profile_token: tacsim::arena::profile_runtime::ProfileToken,
    result: ThinkResult,
}

impl Kept {
    /// The job this think was handed, rebuilt.
    fn job(&self) -> ThinkJob<'_> {
        ThinkJob {
            unit: self.unit,
            think_seq: self.think_seq,
            t_ms: self.t_ms,
            lands_ms: self.lands_ms,
            seed: self.seed,
            dropped: self.dropped,
            profile: &self.profile,
            view: &self.view,
            memory: &self.memory,
            map: &self.map,
            card: &self.card,
            program: &self.program,
            profile_token: self.profile_token,
        }
    }

    /// The memory the think answered, decoded.
    fn memory_out(&self) -> Memory {
        decode_memory(&self.result.memory)
    }
}

/// Every think's trace, as `(unit, think_seq, t_ms, trace)`.
type Traces = Vec<(ObjId, u64, i64, Vec<String>)>;

/// **A [`MindRunner`] that answers through `inner` and keeps every job and
/// answer** — an input recorder, as `tests/twin.rs`' `Recording` is.
#[derive(Debug)]
struct Recorded {
    inner: Arc<dyn MindRunner>,
    kept: Mutex<Vec<Kept>>,
}

impl Recorded {
    fn on(inner: Arc<dyn MindRunner>) -> Arc<Recorded> {
        Arc::new(Recorded {
            inner,
            kept: Mutex::new(Vec::new()),
        })
    }

    fn kept(&self) -> Vec<Kept> {
        self.kept
            .lock()
            .map(|kept| kept.clone())
            .unwrap_or_default()
    }

    /// `unit`'s think `n`.
    fn think(&self, unit: ObjId, n: u64) -> Kept {
        self.kept()
            .into_iter()
            .find(|kept| kept.unit == unit && kept.think_seq == n)
            .unwrap_or_else(|| panic!("#{}'s think {n} was run", unit.0))
    }

    /// Every `(unit, think_seq, t_ms, trace)`.
    fn traces(&self) -> Traces {
        self.kept()
            .into_iter()
            .map(|kept| (kept.unit, kept.think_seq, kept.t_ms, kept.result.trace))
            .collect()
    }
}

impl MindRunner for Recorded {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        let result = self.inner.think(job);
        if let Ok(mut kept) = self.kept.lock() {
            kept.push(Kept {
                unit: job.unit,
                think_seq: job.think_seq,
                t_ms: job.t_ms,
                lands_ms: job.lands_ms,
                seed: job.seed,
                dropped: job.dropped,
                profile: *job.profile,
                view: Arc::clone(job.view),
                memory: job.memory.to_vec(),
                map: Arc::clone(job.map),
                card: job.card.clone(),
                program: job.program.clone(),
                profile_token: job.profile_token,
                result: result.clone(),
            });
        }
        result
    }
}

/// **A [`MindRunner`] that reads its answers off a script** — per hull and
/// think, the answer to give — and echoes the memory with nothing to do
/// elsewhere: `tests/minds.rs`' `ScriptedMind`, keyed by `(unit, think)`.
#[derive(Debug, Default)]
struct ScriptedMind {
    answers: BTreeMap<(ObjId, u64), ThinkResult>,
}

impl MindRunner for ScriptedMind {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        self.answers
            .get(&(job.unit, job.think_seq))
            .cloned()
            .unwrap_or_else(|| ThinkResult {
                memory: job.memory.to_vec(),
                ..ThinkResult::default()
            })
    }
}

/// A recorded Rhai run replayed as a [`ScriptedMind`], the trace dropped
/// (the arena drops it too), each faulted think answered by `faulted`.
fn replay(kept: &[Kept], faulted: impl Fn(&Kept) -> ThinkResult) -> Arc<ScriptedMind> {
    Arc::new(ScriptedMind {
        answers: kept
            .iter()
            .map(|kept| {
                let answer = if kept.result.fault.is_some() {
                    faulted(kept)
                } else {
                    ThinkResult {
                        trace: Vec::new(),
                        ..kept.result.clone()
                    }
                };
                ((kept.unit, kept.think_seq), answer)
            })
            .collect(),
    })
}

/// The decoded memory value at `key`.
fn mem(memory: &Memory, key: &str) -> MemValue {
    memory
        .get(key)
        .cloned()
        .unwrap_or_else(|| panic!("no {key} in {memory:?}"))
}

/// A cell as the memory holds it.
fn cell_value(cell: Option<(u16, u16)>) -> MemValue {
    cell.map_or(MemValue::Unit, |(x, y)| {
        MemValue::Ints(vec![i64::from(x), i64::from(y)])
    })
}

/// An optional INT as the memory holds it.
fn opt_int<T: Into<i64>>(value: Option<T>) -> MemValue {
    value.map_or(MemValue::Unit, |value| MemValue::Int(value.into()))
}

/// The id of the first classified hostile row `view` holds.
fn hostile(view: &MindView) -> tacsim::arena::state::ContactId {
    view.contacts
        .iter()
        .find(|row| !row.friendly && row.class != tacsim::arena::state::Classification::Unknown)
        .map(|row| row.id)
        .unwrap_or_else(|| panic!("a classified hostile on {:?}", view.contacts))
}

// ---------------------------------------------------------------------------
// (1) Parity: every getter against the keyframe, through the PARITY table
// ---------------------------------------------------------------------------

/// What a parity script writes for one getter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Store {
    /// The value itself (an INT, a bool, a word, a cell, INTs, `()`).
    Plain,
    /// `type_of` the handle the getter answers (`()` when it answers `()`).
    Handle,
    /// The array's length.
    Len,
    /// An array of words (or `()`), joined with commas.
    Words,
    /// An array of arrays of words, each joined, each ended with `;`.
    Tables,
    /// The `odds` method, on the cannon.
    Odds,
}

/// How each `PARITY` row's getter is read back through memory (a memory
/// holds INTs, bools, words, INT arrays and `()` only).
fn store(ty: &str, getter: &str) -> Store {
    match (ty, getter) {
        ("View", "receiver" | "heat" | "order" | "objective" | "motor")
        | ("Contact", "condition")
        | ("Weapon", "armed") => Store::Handle,
        ("View", "sections" | "weapons" | "notices" | "contacts" | "rules")
        | ("Section", "facets")
        | ("Rule", "predicates") => Store::Len,
        ("View", "scan_tables") => Store::Tables,
        ("Contact", "fatal") => Store::Words,
        ("Contact", "odds") => Store::Odds,
        _ => Store::Plain,
    }
}

/// **The parity script for one handle**: `x` is the handle at `path`, and
/// one bounded batch of getters is written to memory under its own names.
fn parity_source(ty: &str, path: &str, getters: &[&str]) -> String {
    let mut body = String::new();
    for (row_ty, getter, _) in PARITY {
        if *row_ty != ty || !getters.contains(getter) {
            continue;
        }
        let read = match store(ty, getter) {
            Store::Plain => format!("x.{getter}"),
            Store::Handle => format!("type_of(x.{getter})"),
            Store::Len => format!("x.{getter}.len()"),
            Store::Words => format!("words_of(x.{getter})"),
            Store::Tables => format!("tables_of(x.{getter})"),
            Store::Odds => format!("x.{getter}(\"cannon\")"),
        };
        body.push_str(&format!("    this.{getter} = {read};\n"));
    }
    format!(
        "fn words_of(a) {{\n    if type_of(a) == \"()\" {{ return (); }}\n    let s = \"\";\n    for w in a {{ s += w; s += \",\"; }}\n    s\n}}\nfn tables_of(a) {{\n    let s = \"\";\n    for t in a {{ s += words_of(t); s += \";\"; }}\n    s\n}}\nfn think(v, c) {{\n    let x = {path};\n{body}}}\n"
    )
}

/// A keyframe value as the memory would hold it.
fn json_mem(value: Option<&Json>) -> MemValue {
    match value {
        None | Some(Json::Null) => MemValue::Unit,
        Some(Json::Bool(flag)) => MemValue::Bool(*flag),
        Some(Json::Number(number)) => MemValue::Int(
            number
                .as_i64()
                .unwrap_or_else(|| panic!("an integer: {number}")),
        ),
        Some(Json::String(text)) => MemValue::Str(text.clone()),
        Some(Json::Array(items)) => MemValue::Ints(
            items
                .iter()
                .map(|item| item.as_i64().unwrap_or_else(|| panic!("INTs: {items:?}")))
                .collect(),
        ),
        Some(other) => panic!("no memory shape for {other}"),
    }
}

/// A keyframe contact id (the row key's hex) as the script's INT.
fn id_mem(value: Option<&Json>) -> MemValue {
    match value.and_then(Json::as_str) {
        None => MemValue::Unit,
        Some(hex) => MemValue::Int(contact_int(tacsim::arena::state::ContactId(
            u64::from_str_radix(hex, 16).unwrap_or_else(|_| panic!("a contact id: {hex}")),
        ))),
    }
}

/// Words joined as `words_of` joins them.
fn words_mem(value: Option<&Json>) -> MemValue {
    match value {
        None | Some(Json::Null) => MemValue::Unit,
        Some(Json::Array(words)) => MemValue::Str(
            words
                .iter()
                .map(|word| format!("{},", word.as_str().unwrap_or_default()))
                .collect(),
        ),
        Some(other) => panic!("words: {other}"),
    }
}

/// `()` for a null, the handle's type name otherwise.
fn handle_mem(value: Option<&Json>, name: &str) -> MemValue {
    match value {
        None | Some(Json::Null) => MemValue::Str("()".to_owned()),
        Some(_) => MemValue::Str(name.to_owned()),
    }
}

/// An array's length (`0` for an absent key).
fn len_mem(value: Option<&Json>) -> MemValue {
    MemValue::Int(
        value
            .and_then(Json::as_array)
            .map_or(0, |items| i64::try_from(items.len()).unwrap_or(i64::MAX)),
    )
}

/// The aspects mask a keyframe's word list stands for (`Aspect::code`'s
/// bits: front, left, right, rear).
fn aspects_mem(value: Option<&Json>) -> MemValue {
    let words = value.and_then(Json::as_array).cloned().unwrap_or_default();
    MemValue::Int(
        words
            .iter()
            .map(|word| match word.as_str() {
                Some("front") => 1,
                Some("left") => 2,
                Some("right") => 4,
                Some("rear") => 8,
                other => panic!("an aspect word: {other:?}"),
            })
            .sum(),
    )
}

/// **The miss reason a keyframe's sentence says** — `crate::match_::cockpit`'s
/// `miss_words`, read backwards: `Miss on BA: the line was blocked.` says
/// `blocked`.
fn miss_reason(text: &str) -> String {
    const WORDS: [(&str, &str); 7] = [
        ("the target is gone", "target_gone"),
        ("the target is already a wreck", "wreck"),
        ("the target moved", "moved"),
        ("the line was blocked", "blocked"),
        ("the target is behind cover", "cover"),
        ("the round went wide", "dispersion"),
        ("the round found nothing to break", "refused"),
    ];
    let said = text
        .split_once(": ")
        .map_or(text, |(_, said)| said)
        .trim_end_matches('.');
    WORDS
        .iter()
        .find(|(words, _)| *words == said)
        .map_or_else(|| said.to_owned(), |(_, reason)| (*reason).to_owned())
}

/// **What the keyframe says a getter answers** — the `PARITY` row's rendering,
/// one arm per row: `obj` is the handle's own object in the keyframe, `kf` the
/// whole keyframe, `position` the handle's place in its array, `view` the
/// view (for the rows whose rendering names a row by its designator) and
/// `empties` the notices the keyframe filtered out for rendering to nothing
/// (the declared divergence of `View.notices`).
#[allow(clippy::too_many_lines)]
fn expected(
    ty: &str,
    getter: &str,
    (obj, position): (&Json, usize),
    kf: &Json,
    view: &MindView,
    empties: usize,
) -> MemValue {
    let key = |name: &str| obj.get(name);
    let detail = |name: &str| obj.get("detail").and_then(|detail| detail.get(name));
    match (ty, getter) {
        ("View", "team") => {
            let side = kf
                .get("callsign")
                .and_then(Json::as_str)
                .and_then(|callsign| callsign.chars().next());
            MemValue::Str(
                match side {
                    Some('R') => "red",
                    Some('B') => "blue",
                    other => panic!("a callsign's side: {other:?}"),
                }
                .to_owned(),
            )
        }
        ("View", "held") => json_mem(key("blocked")),
        ("View", "ecm") => match key("ecm").and_then(Json::as_str) {
            None => MemValue::Unit,
            Some(word) => MemValue::Bool(word == "on"),
        },
        ("View", "receiver") => handle_mem(key("receiver"), "Receiver"),
        ("View", "heat") => handle_mem(key("heat"), "Heat"),
        ("View", "order") => handle_mem(key("order"), "Order"),
        ("View", "objective") => handle_mem(key("objective"), "Objective"),
        ("View", "motor") => handle_mem(key("motor"), "Motor"),
        ("View", "sections" | "weapons" | "contacts" | "rules") => len_mem(key(getter)),
        ("Rule", "slot") => json_mem(key("slot")),
        ("Rule", "channel") => MemValue::Str(
            if obj["rule"]["action"] == "lock" {
                "sensors"
            } else {
                "fire"
            }
            .to_owned(),
        ),
        ("Rule", "weapon") => json_mem(
            obj.pointer("/rule/weapon")
                .or_else(|| obj.pointer("/rule/condition/weapon")),
        ),
        ("Rule", "target") => json_mem(obj.pointer("/rule/condition/target/target")),
        ("Rule", "contact") => obj
            .pointer("/rule/condition/target/id")
            .and_then(Json::as_u64)
            .map_or(MemValue::Unit, |id| {
                MemValue::Int(contact_int(tacsim::arena::state::ContactId(id)))
            }),
        ("Rule", "min_permille" | "aspects" | "min_range_m" | "max_range_m") => json_mem(
            obj.get("rule")
                .and_then(|rule| rule.get("condition"))
                .and_then(|condition| condition.get(getter)),
        ),
        ("Rule", "keeps") => json_mem(obj.pointer("/rule/condition/min_rounds_left")),
        ("Rule", "predicates") => len_mem(obj.pointer("/rule/predicates")),
        ("Rule", "independent") => MemValue::Bool(
            obj.pointer("/rule/independent")
                .and_then(Json::as_bool)
                .unwrap_or(false),
        ),
        ("Rule", _) => json_mem(obj.get("rule").and_then(|rule| rule.get(getter))),
        ("Predicate", "kind") => json_mem(key("predicate")),
        ("Predicate", "value") => json_mem(
            [
                "min_permille",
                "mask",
                "metres",
                "amount",
                "below_percent",
                "above_percent",
            ]
            .iter()
            .find_map(|name| key(name)),
        ),
        ("View", "notices") => {
            let MemValue::Int(shown) = len_mem(key("notices")) else {
                panic!("a length")
            };
            MemValue::Int(shown + i64::try_from(empties).unwrap_or(0))
        }
        ("View" | "Contact" | "Notice", "lock" | "id" | "contact") => id_mem(key(getter)),
        ("View", "scan_tables") => MemValue::Str(
            key("scan_tables")
                .and_then(Json::as_array)
                .map(|tables| {
                    tables
                        .iter()
                        .map(|table| {
                            let MemValue::Str(words) = words_mem(Some(table)) else {
                                panic!("a table")
                            };
                            format!("{words};")
                        })
                        .collect()
                })
                .unwrap_or_default(),
        ),
        ("Contact", "alias") => match key("alias").and_then(Json::as_str) {
            None => MemValue::Unit,
            Some(word) => MemValue::Int(
                word.trim_start_matches('X')
                    .parse()
                    .unwrap_or_else(|_| panic!("an alias: {word}")),
            ),
        },
        ("Contact", "identified") => {
            MemValue::Bool(key("class").and_then(Json::as_str) != Some("unknown"))
        }
        ("Contact", "condition") => handle_mem(key("condition"), "Condition"),
        ("Contact", "fatal") => words_mem(key("fatal")),
        ("Contact", "odds") => key("odds")
            .and_then(Json::as_array)
            .and_then(|odds| {
                odds.iter()
                    .find(|row| row.get("weapon").and_then(Json::as_str) == Some("cannon"))
            })
            .map_or(MemValue::Unit, |row| json_mem(row.get("p_permille"))),
        ("Weapon", "index") => MemValue::Int(i64::try_from(position).unwrap_or(-1)),
        ("Weapon", "range_band") => match key("range_band") {
            None | Some(Json::Null) => MemValue::Unit,
            Some(band) => MemValue::Ints(
                ["index", "of", "max_range_cm", "dispersion_mrad"]
                    .iter()
                    .map(|name| {
                        band.get(*name)
                            .and_then(Json::as_i64)
                            .unwrap_or_else(|| panic!("range_band.{name}"))
                    })
                    .collect(),
            ),
        },
        ("Weapon", "armed") => handle_mem(key("armed"), "Armed"),
        ("Armed", "aspects") => aspects_mem(key("aspects")),
        ("Armed", "min_range_m") => match key("min_range_m") {
            None | Some(Json::Null) => MemValue::Int(0),
            value => json_mem(value),
        },
        ("Armed", "max_range_m") => match key("max_range_m") {
            None | Some(Json::Null) => MemValue::Int(i64::from(u32::MAX)),
            value => json_mem(value),
        },
        ("Section", "facets") => len_mem(key("facets")),
        ("Receiver", "reach_permille") => match key("reach_percent").and_then(Json::as_i64) {
            Some(percent) => MemValue::Int(percent * 10),
            None => panic!("reach_percent"),
        },
        ("Order", "target" | "of") => match key(getter).and_then(Json::as_str) {
            None => MemValue::Unit,
            Some(designator) => MemValue::Int(contact_int(
                view.contacts
                    .iter()
                    .find(|row| row.designator.to_string() == designator)
                    .map(|row| row.id)
                    .unwrap_or_else(|| panic!("the row the order names: {designator}")),
            )),
        },
        ("Order", "expires_ms") => {
            let left = key("expires_in_ms")
                .and_then(Json::as_i64)
                .expect("expires_in_ms");
            let t_ms = kf.get("t_ms").and_then(Json::as_i64).expect("t_ms");
            if left > 0 {
                MemValue::Int(t_ms + left)
            } else {
                // Clamped at nought: the keyframe says only "passed"; the
                // getter reads the absolute past millisecond, checked by the
                // caller against the order it set.
                let order = view.order.as_ref().expect("an order");
                assert!(order.expires_ms <= t_ms, "a clamp on a live deadline");
                MemValue::Int(order.expires_ms)
            }
        }
        (
            "Notice",
            "radius_m" | "secs" | "heat" | "max" | "floor_permille" | "why" | "floor" | "range_m"
            | "min_m" | "max_m" | "rounds" | "keeps",
        ) => json_mem(detail(getter)),
        ("Notice", "reason") => {
            if key("kind").and_then(Json::as_str) == Some("miss") {
                MemValue::Str(miss_reason(
                    key("text").and_then(Json::as_str).expect("a sentence"),
                ))
            } else {
                MemValue::Unit
            }
        }
        ("Notice", "outcome") => match key("kind").and_then(Json::as_str) {
            Some(kind) if kind.starts_with("drive_") => {
                MemValue::Str(kind.trim_start_matches("drive_").to_owned())
            }
            _ => MemValue::Unit,
        },
        ("Notice", "percent" | "aspect") => match key("kind").and_then(Json::as_str) {
            Some("slot_fired") => json_mem(key(getter)),
            Some("slot_held") => json_mem(detail(getter)),
            _ => MemValue::Unit,
        },
        // Every other row: the key of the same name.
        _ => json_mem(key(getter)),
    }
}

/// One moment's parity: the view and the keyframe of one hull at one arena
/// millisecond.
struct Moment {
    view: Arc<MindView>,
    frame: Frame,
    ranges: BTreeMap<tacsim::arena::state::ContactId, u64>,
    map: Arc<ArenaDefinition>,
    card: tacsim::arena::runtime_card::RuntimeCard,
    program: tacsim::arena::profile_rules::BoundProgram,
    profile_token: tacsim::arena::profile_runtime::ProfileToken,
}

/// **The view and the frame of `unit` now**: the view off one clone
/// (`Arena::mind_view`, which drains and settles), the frame off another —
/// a pilot seated on the hull for one zero-length pulse, the frame the pulse
/// hands that pilot (`Arena::frame_for`, the function `mind_view` builds on)
/// — and the range memory as it stood before the view settled it.
fn moment(arena: &Arena, wall: i64, unit: ObjId) -> Moment {
    let mut viewing = arena.clone();
    let ranges = viewing
        .minds
        .get(&unit)
        .map(|state| state.ranges.clone())
        .expect("a mind row");
    let view = viewing.mind_view(unit).expect("a view");
    let mut framing = arena.clone();
    let watcher = ObjId(77_777);
    framing.pilots.push(Pilot {
        unit,
        pilot: watcher,
        station: SmolStr::new("cockpit"),
        epoch: 1,
    });
    let out = framing.step(wall, &[], &[]);
    let frame = out
        .frames
        .into_iter()
        .find(|(pilot, _)| *pilot == watcher)
        .map(|(_, frame)| frame)
        .expect("the watcher's frame");
    assert_eq!(framing.now_ms, arena.now_ms, "a zero-length pulse");
    assert_eq!(frame.t_ms, view.t_ms, "one millisecond");
    assert_eq!(frame.unit, unit);
    Moment {
        view: Arc::new(view),
        frame,
        ranges,
        map: Arc::clone(&arena.definition),
        card: arena.profiles[&unit].card.clone(),
        program: arena.profiles[&unit].program.clone(),
        profile_token: arena.profiles[&unit].token(),
    }
}

/// One handle a parity script reads: its type, its path, its keyframe object
/// and its place in its array (0 for a handle in none).
struct Handle {
    ty: &'static str,
    path: String,
    obj: Json,
    position: usize,
}

/// The `PARITY` rows a parity read compared, and those of them compared on
/// a value other than `()`.
#[derive(Debug, Default)]
struct Exercised {
    rows: BTreeSet<(String, String)>,
    valued: BTreeSet<(String, String)>,
}

impl Exercised {
    fn extend(&mut self, other: Exercised) {
        self.rows.extend(other.rows);
        self.valued.extend(other.valued);
    }
}

/// **Read every getter of every handle the moment's view carries and compare
/// each with the keyframe through `PARITY`**, answering the rows exercised.
#[allow(clippy::too_many_lines)]
fn parity_at(moment: &Moment, unit: ObjId) -> Exercised {
    let view = &moment.view;
    let keyframe = |notices: Vec<FrameNotice>| {
        let mut frame = moment.frame.clone();
        frame.notices = notices;
        status_frame_held(MatchId(1), &frame, &moment.ranges)
    };
    // The keyframe carries the hull's view's notices: the mind's buffer is
    // what the keyframe of that hull would print.
    let kf = keyframe(view.notices.clone());
    let mut handles = vec![Handle {
        ty: "View",
        path: "v".to_owned(),
        obj: kf.clone(),
        position: 0,
    }];
    let rows = kf
        .get("contacts")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(rows.len(), view.contacts.len(), "one row per contact");
    for (i, row) in view.contacts.iter().enumerate() {
        let obj = rows
            .iter()
            .find(|obj| obj.get("id").and_then(Json::as_str) == Some(row.id.to_string().as_str()))
            .cloned()
            .unwrap_or_else(|| panic!("the keyframe's row for {}", row.id));
        if row.condition.is_some() {
            handles.push(Handle {
                ty: "Condition",
                path: format!("v.contacts[{i}].condition"),
                obj: obj.get("condition").cloned().expect("a reading"),
                position: 0,
            });
        }
        handles.push(Handle {
            ty: "Contact",
            path: format!("v.contacts[{i}]"),
            obj,
            position: i,
        });
    }
    let weapons = kf
        .get("weapons")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(weapons.len(), view.weapons.len());
    for (i, (row, obj)) in view.weapons.iter().zip(weapons).enumerate() {
        if row.armed.is_some() {
            handles.push(Handle {
                ty: "Armed",
                path: format!("v.weapons[{i}].armed"),
                obj: obj.get("armed").cloned().expect("an armed slot"),
                position: 0,
            });
        }
        handles.push(Handle {
            ty: "Weapon",
            path: format!("v.weapons[{i}]"),
            obj,
            position: i,
        });
    }
    let rules = kf
        .get("rules")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        rules.len(),
        view.rules.len(),
        "every native slot is projected"
    );
    for (i, obj) in rules.into_iter().enumerate() {
        for (j, predicate) in obj
            .pointer("/rule/predicates")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            handles.push(Handle {
                ty: "Predicate",
                path: format!("v.rules[{i}].predicates[{j}]"),
                obj: predicate.clone(),
                position: j,
            });
        }
        handles.push(Handle {
            ty: "Rule",
            path: format!("v.rules[{i}]"),
            obj,
            position: i,
        });
    }
    let sections = kf
        .get("sections")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default();
    for (i, (row, obj)) in view.sections.iter().zip(sections).enumerate() {
        let facets = obj
            .get("facets")
            .and_then(Json::as_array)
            .cloned()
            .unwrap_or_default();
        for (j, (_, facet)) in row.facets.iter().zip(facets).enumerate() {
            handles.push(Handle {
                ty: "Facet",
                path: format!("v.sections[{i}].facets[{j}]"),
                obj: facet,
                position: j,
            });
        }
        handles.push(Handle {
            ty: "Section",
            path: format!("v.sections[{i}]"),
            obj,
            position: i,
        });
    }
    for (ty, path, key) in [
        ("Heat", "v.heat", "heat"),
        ("Receiver", "v.receiver", "receiver"),
        ("Order", "v.order", "order"),
        ("Objective", "v.objective", "objective"),
        ("Motor", "v.motor", "motor"),
    ] {
        if let Some(obj) = kf.get(key).filter(|obj| !obj.is_null()) {
            handles.push(Handle {
                ty,
                path: path.to_owned(),
                obj: obj.clone(),
                position: 0,
            });
        }
    }
    // A notice's own keyframe object: the keyframe of that notice alone, so
    // a notice the keyframe filters (an empty sentence) is counted and its
    // neighbours keep their places.
    let mut empties = 0_usize;
    for (i, notice) in view.notices.iter().enumerate() {
        let alone = keyframe(vec![notice.clone()]);
        match alone
            .get("notices")
            .and_then(Json::as_array)
            .and_then(|notices| notices.first())
        {
            Some(obj) => handles.push(Handle {
                ty: "Notice",
                path: format!("v.notices[{i}]"),
                obj: obj.clone(),
                position: i,
            }),
            None => empties += 1,
        }
    }

    let mut exercised = Exercised::default();
    for handle in &handles {
        // The API can grow beyond one Class-I memory map. Probe bounded batches
        // rather than raising the real memory budget or storing opaque handles.
        let getters: Vec<&str> = PARITY
            .iter()
            .filter(|(ty, _, _)| *ty == handle.ty)
            .map(|(_, getter, _)| *getter)
            .collect();
        for batch in getters.chunks(8) {
            let source = parity_source(handle.ty, &handle.path, batch);
            let runner = RhaiRunner::new(&ArenaConfig::default(), &source, digest(&source))
                .unwrap_or_else(|error| panic!("{source}: {error}"));
            let job = ThinkJob {
                unit,
                think_seq: 0,
                t_ms: view.t_ms,
                lands_ms: view.t_ms + SUBSTEP,
                seed: SEED,
                dropped: false,
                profile: &tacsim::arena::core::ClassTable::STOCK
                    .row(tacsim::arena::core::CoreClass::I),
                view: &moment.view,
                memory: &[],
                map: &moment.map,
                card: &moment.card,
                program: &moment.program,
                profile_token: moment.profile_token,
            };
            let result = runner.think(&job);
            assert_eq!(
                result.fault, None,
                "{} at {}: {:?}",
                handle.ty, handle.path, result.trace
            );
            let memory = decode_memory(&result.memory);
            for (ty, getter, rendering) in PARITY {
                if *ty != handle.ty || !batch.contains(getter) {
                    continue;
                }
                let got = mem(&memory, getter);
                let want = expected(
                    ty,
                    getter,
                    (&handle.obj, handle.position),
                    &kf,
                    view,
                    empties,
                );
                assert_eq!(
                    got, want,
                    "{ty}.{getter} at {} (the keyframe's {rendering}): {}",
                    handle.path, handle.obj
                );
                let row = ((*ty).to_owned(), (*getter).to_owned());
                // A value only where the keyframe printed one: not `()`, not a
                // null handle (`"()"`), not a null key read as its default.
                let printed_null = matches!(handle.obj.get(*getter), Some(Json::Null));
                if want != MemValue::Unit && want != MemValue::Str("()".to_owned()) && !printed_null
                {
                    exercised.valued.insert(row.clone());
                }
                exercised.rows.insert(row);
            }
        }
    }
    exercised
}

/// The (j) field for the parity test: the armed red carrying heat (ten of a
/// hundred), the Rhai script below flying both reds.
///
/// The script: start; at the fourth think arm the cannon's S1 on the lock at
/// a certain hit (1,000 per mille — held, the odds under it: a `SlotHeld`)
/// with every term off its default (two rounds kept, 10 m to 2,400 m, so the
/// keyframe prints numbers for `keeps` and both range bounds),
/// drive to (6, 6) (a `Drive` arrived), switch a pod the hull does not carry
/// (refused `no_jammer`) and lock the first classified hostile.
const PARITY_FIELD: &str = r#"
fn think(v, c) {
    if !("k" in this) { this.k = 0; }
    this.k += 1;
    if v.unit != 4 { return; }
    if this.k == 1 { startup(); return; }
    if this.k == 4 {
        arm("cannon", "lock", 1000, 15, 2, 10, 2400);
        goto([6, 6]);
        emcon(true);
        for k in v.contacts { if k.identified && !k.friendly { lock(k.id); } }
    }
}
"#;

/// **17b cut M2.4's parity test** (doc 17 §3, D-UM-4; 17b §1 (ii), §5's feas
/// F5): on the live (j) field, every getter of every type a script can read —
/// written to the think's memory by a script per handle and decoded — equals
/// what the keyframe `status_frame_held` builds for the same hull at the same
/// millisecond carries, through M2.3's `PARITY` table: the key of the same
/// name where the row says so; the miss's reason against its sentence; the
/// drive's outcome against the kind word; `expires_ms` against `t_ms +
/// expires_in_ms`, and — with the deadline already passed, where the keyframe
/// clamps to nought — the absolute past millisecond; `Order.target` against
/// the row whose designator the keyframe prints; `identified` against
/// `class`; `Weapon.index` against the position; `team` against the
/// callsign's side; `range_band` against the object's four figures.
///
/// The armed red carries heat (a heat cell and its `HeatState`, as
/// `tests/reflex.rs` gives one). Six moments: early (two UNKNOWN returns, no
/// lock, no slot); and late — both rows classified with their readings and
/// odds, the cannon's slot armed and held, a lock — with the hull's buffer
/// re-staged with the notices its own thinks were handed (the `no_jammer`
/// refusal, the `SlotHeld` on the odds, the `Drive` arrived), a `Miss` with
/// a `blocked` reason and the other notice shapes written beside them
/// ([`written`]); the armed red given a pod (off) and a degraded receiver as
/// `crate::arena::mind`'s own parity fixture writes them (no (j)-field hull
/// carries either); and a standing order on the hull carrying
/// `max_range_band` and a deadline — an `Engage` on the hostile live, the same
/// with its deadline passed, and an `Advance`, a `Screen` and a `Support` of
/// the wingman. M5rules adds authored Fire/Hold/Lock and all predicate shapes.
/// Every one of `PARITY`'s 149 rows is exercised, and every row
/// is compared at least once on a value the keyframe printed (not `()`, not
/// a null handle, not a null key read as its default).
#[test]
#[allow(clippy::too_many_lines)]
fn m2_every_getter_matches_the_keyframe_through_the_parity_table() {
    let mut arena = the_j_field(Controller::Mind);
    if let Some(state) = arena.unit_mut(RED) {
        state.unit = tactical_unit(RED, Some((10, 100)));
        state.heat = Some(HeatState {
            section: SmolStr::new("hull"),
            max: 100,
            per_s: 0,
            per_shot: BTreeMap::new(),
            carry_milli: 0,
        });
    }
    let recorded = Recorded::on(rhai(&arena, PARITY_FIELD));
    arena.mind_runner = recorded.clone();
    let mut rig = Rig::new(arena);

    // Early: think 0 landed, the returns unknown.
    rig.to(think_at(RED, 0) + 2 * SUBSTEP);
    let early = moment(&rig.arena, rig.wall.unwrap_or(0), RED);
    assert!(
        early
            .view
            .contacts
            .iter()
            .any(|row| row.class == tacsim::arena::state::Classification::Unknown),
        "an unknown row early: {:?}",
        early.view.contacts
    );
    let mut exercised = parity_at(&early, RED);

    // Late: think 5 landed.
    rig.to(think_at(RED, 5) + 2 * SUBSTEP);
    let blue = hostile(&recorded.think(RED, 5).view);
    let staged: Vec<FrameNotice> = [4_u64, 5]
        .iter()
        .flat_map(|n| recorded.think(RED, *n).view.notices.clone())
        .filter(|notice| {
            matches!(
                notice,
                FrameNotice::Refused { .. }
                    | FrameNotice::SlotHeld { .. }
                    | FrameNotice::Drive { .. }
            )
        })
        .collect();
    for (what, kind) in [
        ("a refusal", "refused"),
        ("a held slot", "slot_held"),
        ("an arrival", "drive_arrived"),
    ] {
        assert!(
            staged.iter().any(|notice| notice.kind().as_str() == kind),
            "{what} among the hull's own notices: {staged:?}"
        );
    }
    let named = |id| {
        recorded
            .think(RED, 5)
            .view
            .contacts
            .iter()
            .find(|row| row.id == id)
            .map(|row| row.designator)
            .expect("the row")
    };
    let designator = named(blue);
    let friend = recorded
        .think(RED, 5)
        .view
        .contacts
        .iter()
        .find(|row| row.friendly)
        .map(|row| row.id)
        .expect("the wingman's row");
    let friend_designator = named(friend);
    let now = rig.arena.now_ms;
    let order = FrameOrder {
        squad: SmolStr::new("red-1"),
        objective: MindObjective::Engage { target: Some(blue) },
        hold_fire: true,
        max_range_band: Some(RangeBand::Effective),
        expires_ms: now + 20_000,
    };
    let stage = |arena: &mut Arena, order: FrameOrder| {
        let launch_cell = arena
            .unit(RED)
            .and_then(|unit| arena.definition.terrain.cell_of(&unit.pose));
        if let Some(state) = arena.unit_mut(RED) {
            state.jammer = Some(Jammer {
                radius_cm: 10_000,
                attenuation_cost: 25,
                signature_permille: 150,
            });
            state.ecm_on = false;
            state.receiver = Some(Receiver {
                budget: 30,
                budget_max: 40,
                reach_permille: 755,
            });
        }
        if let Some(state) = arena.minds.get_mut(&RED) {
            for notice in &staged {
                state.push_notice(notice.clone());
            }
            state.push_notice(FrameNotice::Miss {
                launch_cell,
                contact: Some(Named {
                    id: blue,
                    designator,
                }),
                reason: SmolStr::new("blocked"),
            });
            for notice in written(blue, designator, friend, friend_designator) {
                state.push_notice(notice);
            }
            state.order = Some(order);
        }
    };
    let mut live = rig.arena.clone();
    stage(&mut live, order.clone());
    let late = moment(&live, rig.wall.unwrap_or(0), RED);
    assert!(late.view.lock.is_some(), "a lock");
    assert!(late.view.weapons.iter().any(|row| row.armed.is_some()));
    assert!(late.view.contacts.iter().all(|row| row.condition.is_some()));
    assert!(late.view.contacts.iter().any(|row| !row.odds.is_empty()));
    assert!(late.view.heat.is_some() && late.view.receiver.is_some());
    assert!(late.view.notices.len() <= tacsim::arena::mind::MIND_NOTICES);
    exercised.extend(parity_at(&late, RED));

    // The order's other payloads: a cell, a facing and a wingman.
    for objective in [
        MindObjective::Advance { to: (6, 3) },
        MindObjective::Screen {
            facing_mdeg: 135_000,
        },
        MindObjective::Support { of: Some(friend) },
    ] {
        let mut other = rig.arena.clone();
        stage(
            &mut other,
            FrameOrder {
                objective,
                ..order.clone()
            },
        );
        exercised.extend(parity_at(&moment(&other, rig.wall.unwrap_or(0), RED), RED));
    }

    // The deadline passed: the keyframe clamps to nought, the getter reads
    // the absolute past millisecond — the value T1 writes to
    // `ignore_until_ms`.
    let mut passed = rig.arena.clone();
    let past = now - 700;
    stage(
        &mut passed,
        FrameOrder {
            expires_ms: past,
            ..order
        },
    );
    let clamped = moment(&passed, rig.wall.unwrap_or(0), RED);
    let kf = status_frame_held(MatchId(1), &clamped.frame, &clamped.ranges);
    assert_eq!(kf["order"]["expires_in_ms"], 0);
    assert_eq!(
        clamped.view.order.as_ref().map(|order| order.expires_ms),
        Some(past)
    );
    exercised.extend(parity_at(&clamped, RED));

    // Complete authored rules are projected beside the old weapon shorthand.
    // Use the same public frame builders over a real condition table; this is
    // a projection fixture, so it never advances the arena or evaluates rules.
    {
        use tacsim::arena::conditions::{
            Comparison, LockSelector, Rule, RuleBand, RulePredicate, SectionState, TargetState,
        };
        let rules = [
            Rule::fire_rule(
                FireCondition {
                    weapon: "cannon".into(),
                    target: SlotTarget::Contact { id: blue },
                    min_permille: 600,
                    aspects: ASPECTS_ANY,
                    min_rounds_left: 0,
                    min_range_m: 0,
                    max_range_m: u32::MAX,
                },
                vec![
                    RulePredicate::Odds {
                        weapon: Some("cannon".into()),
                        min_permille: 600,
                    },
                    RulePredicate::Aspect { mask: 14 },
                    RulePredicate::Range {
                        comparison: Comparison::Less,
                        metres: 421,
                    },
                    RulePredicate::Target {
                        state: TargetState::Moving,
                    },
                ],
            ),
            Rule::hold_rule(
                "cannon".into(),
                vec![
                    RulePredicate::OwnSection {
                        section: "hull".into(),
                        below_percent: 50,
                    },
                    RulePredicate::Heat { above_percent: 70 },
                    RulePredicate::Rounds {
                        weapon: Some("cannon".into()),
                        comparison: Comparison::AtLeast,
                        amount: 25,
                        percent: true,
                    },
                    RulePredicate::Lock { present: false },
                ],
            ),
            Rule::lock_rule(
                LockSelector::Flank,
                Some("cannon".into()),
                vec![
                    RulePredicate::TargetSection {
                        section: "hull".into(),
                        state: SectionState::Red,
                    },
                    RulePredicate::Band {
                        band: RuleBand::Effective,
                    },
                ],
            ),
        ];
        let mut authored = live.clone();
        authored.conditions.clear(RED);
        for (index, rule) in rules.into_iter().enumerate() {
            authored
                .conditions
                .set_rules(
                    RED,
                    Some(u8::try_from(index + 1).expect("slot")),
                    &[rule],
                    4,
                )
                .expect("bounded explicit rule");
        }
        exercised.extend(parity_at(
            &moment(&authored, rig.wall.unwrap_or(0), RED),
            RED,
        ));
    }

    // These facts come through the real arena projection in a public Hold
    // scenario. Hidden capture ownership/time are deliberately populated too:
    // the parity read can compare only the published objective geometry.
    let mut holding = live.clone();
    holding.mode = tacsim::arena::state::MatchMode::Hold;
    holding.hold = Some(tacsim::arena::state::HoldState {
        spot: (6, 3),
        letter: Some('A'),
        radius_cells: 2,
        hold_ms: 60_000,
        held_by: Some((Team::Red, now)),
    });
    for outcome in [
        None,
        Some(tacsim::arena::perception::DriveOutcome::Arrived),
        Some(tacsim::arena::perception::DriveOutcome::NoRoute),
    ] {
        let motor = holding.motors.ensure(RED);
        motor.requested_goal = Some((6, 3));
        motor.outcome = outcome;
        motor.goal = outcome.is_none().then_some((6, 3));
        exercised.extend(parity_at(
            &moment(&holding, rig.wall.unwrap_or(0), RED),
            RED,
        ));
    }

    let rows: BTreeSet<(String, String)> = PARITY
        .iter()
        .map(|(ty, getter, _)| ((*ty).to_owned(), (*getter).to_owned()))
        .collect();
    assert_eq!(PARITY.len(), 149);
    assert_eq!(
        exercised.rows,
        rows,
        "every PARITY row exercised: {} of {}",
        exercised.rows.len(),
        rows.len()
    );
    // And every row on a value, not only on `()` against a null.
    let unvalued: Vec<&(String, String)> = rows.difference(&exercised.valued).collect();
    assert!(
        unvalued.is_empty(),
        "rows never compared on a value the keyframe printed: {unvalued:?}"
    );
}

/// **Notices written beside the hull's own** (the shapes
/// `crate::minds::fixture` builds), so every `Notice` getter is compared on a
/// value: the three refusal details, a pod's switch, a fired slot, the three
/// other reasons a slot holds, a classification with the name it was told
/// apart from (of the wingman, so `friendly` reads true), a hit on a section
/// spent, and a block's cause.
fn written(
    blue: tacsim::arena::state::ContactId,
    designator: tacsim::arena::state::Designator,
    friend: tacsim::arena::state::ContactId,
    friend_designator: tacsim::arena::state::Designator,
) -> Vec<FrameNotice> {
    use tacsim::arena::conditions::HeldWhy;
    use tacsim::arena::perception::{Aspect, NoticeKind, RefusalDetail};
    let hostile = Named {
        id: blue,
        designator,
    };
    vec![
        FrameNotice::Refused {
            order: SmolStr::new("fire"),
            refusal: Refusal::Cycling,
            weapon: Some(SmolStr::new("cannon")),
            detail: RefusalDetail::Cycling { secs: 2 },
        },
        FrameNotice::Refused {
            order: SmolStr::new("fire"),
            refusal: Refusal::Overheated,
            weapon: Some(SmolStr::new("cannon")),
            detail: RefusalDetail::Overheated {
                heat: 100,
                max: 100,
            },
        },
        FrameNotice::Refused {
            order: SmolStr::new("fire"),
            refusal: Refusal::Unlawful,
            weapon: Some(SmolStr::new("cannon")),
            detail: RefusalDetail::Floor { permille: 100 },
        },
        FrameNotice::Emcon {
            on: true,
            radius_m: 300,
        },
        FrameNotice::SlotFired {
            slot: 1,
            weapon: SmolStr::new("cannon"),
            contact: Some(hostile),
            p_permille: 620,
            aspect: Aspect::Left,
        },
        FrameNotice::SlotHeld {
            slot: 1,
            weapon: SmolStr::new("cannon"),
            why: HeldWhy::Range {
                contact: hostile,
                range_m: 500,
                min_m: 0,
                max_m: 420,
            },
        },
        FrameNotice::SlotHeld {
            slot: 1,
            weapon: SmolStr::new("cannon"),
            why: HeldWhy::Rounds {
                rounds: 2,
                keeps: 2,
            },
        },
        FrameNotice::SlotHeld {
            slot: 1,
            weapon: SmolStr::new("cannon"),
            why: HeldWhy::Aspect {
                contact: hostile,
                aspect: Aspect::Front,
            },
        },
        FrameNotice::Contact {
            kind: NoticeKind::Classified,
            contact: Named {
                id: friend,
                designator: friend_designator,
            },
            class: tacsim::arena::state::Classification::Classified { team: Team::Red },
            was: Some(tacsim::arena::state::Designator::Provisional(1)),
        },
        FrameNotice::Hit {
            contact: Some(hostile),
            section: Some(SmolStr::new("hull")),
            spent: true,
        },
        FrameNotice::Blocked {
            cause: tacsim::arena::motion::BlockedCause::Occupied,
        },
    ]
}

// ---------------------------------------------------------------------------
// (2) Every station function through the one door
// ---------------------------------------------------------------------------

/// A line the trace carries and no ledger line ever may.
const STATION_TRACE: &str = "station trace 7731";

/// **Every station function once**: the armed red starts at its first think,
/// and at its fourth arms the cannon's S1 on the lock, drives to a cell, then
/// to the spot `Ridge`, holds, lets go of its goal, turns, makes way, locks
/// the classified hostile, switches a pod it does not carry, fires S1, lets
/// the slot go and traces; the unarmed red fires a cannon it does not carry
/// at its fourth.
const STATIONS: &str = r#"
fn think(v, c) {
    if !("k" in this) { this.k = 0; }
    this.k += 1;
    if v.unit == 6 {
        if this.k == 4 { fire("cannon", 1); }
        return;
    }
    if this.k == 1 { startup(); return; }
    if this.k == 4 {
        let id = ();
        for k in v.contacts { if k.identified && !k.friendly { id = k.id; } }
        arm("cannon", "lock", 100, 15, 0, 0, 4294967295);
        goto([6, 6]);
        goto("Ridge");
        hold();
        goto_off();
        helm(45000);
        speed(300);
        lock(id);
        emcon(true);
        fire("cannon", 1);
        disarm("cannon");
        trace("station trace 7731");
    }
}
"#;

/// The (j) field with the spot `Ridge` at (4, 4), flown by `runner`.
fn stations_field(runner: Arc<dyn MindRunner>) -> Rig {
    let mut arena = the_j_field(Controller::Mind);
    Arc::make_mut(&mut arena.definition).key_spots = vec![KeySpot {
        name: SmolStr::new("Ridge"),
        x: 4,
        y: 4,
    }];
    arena.mind_runner = runner;
    Rig::new(arena)
}

/// **Every station function lands through the one door** (doc 17 §4, D-UM-9,
/// D-UM-10, D-UM-47; doc 17 §22's italic line). One script calls each station
/// function once:
///
/// * the think answers the intents in call order — `startup` alone at think
///   0, then at think 3 exactly the arming, the cell, the spot's cell, the
///   hold, the release of the goal, the heading, the speed, the lock, the pod,
///   the cited shot and the disarm, each the `IntentKind` T1 emits;
/// * that list is the answer pending on the row (17b cut M3.2), stamped with the hull's
///   channel epochs at the think, and it lands one substep later through
///   `Arena::apply_machine_intents` in that order: the `startup`, `locked`
///   and `fired` (by the mind, S1) lines at their landings, the pod refused
///   `no_jammer` into the next view, the slot gone, the lock held — and a
///   [`ScriptedMind`] answering the same thinks with the same answers runs to
///   the same event list and the same substep hashes, so nothing reached the
///   arena but the intents and the memory;
/// * `trace("…")` is in the think's trace and in NO ledger line;
/// * the unarmed red's `fire("cannon", 1)` is not a fault: the think answers
///   the intent with [`UNGRANTED_FIRE`] in its trace, the landing refuses it
///   `channel`, the refusal is in its next view, and its strikes stay 0.
#[test]
#[allow(clippy::too_many_lines)]
fn m2_every_station_function_lands_through_the_one_door() {
    let probe = the_j_field(Controller::Mind);
    let runner = rhai(&probe, STATIONS);
    let recorded = Recorded::on(runner.clone());
    let mut rig = stations_field(recorded.clone());
    let think = think_at(RED, 3);
    rig.to(think);
    let kept = recorded.think(RED, 3);
    let blue = hostile(&kept.view);
    let expected = vec![
        IntentKind::Condition(Arming::Arm(FireCondition {
            weapon: SmolStr::new("cannon"),
            target: SlotTarget::Lock,
            min_permille: 100,
            aspects: ASPECTS_ANY,
            min_rounds_left: 0,
            min_range_m: 0,
            max_range_m: u32::MAX,
        })),
        IntentKind::Goal(MotorGoal::Cell { x: 6, y: 6 }),
        IntentKind::Goal(MotorGoal::Cell { x: 4, y: 4 }),
        IntentKind::Goal(MotorGoal::Hold),
        IntentKind::Goal(MotorGoal::Off),
        IntentKind::Heading { mdeg: 45_000 },
        IntentKind::Speed { cms: 300 },
        IntentKind::Lock { contact: blue },
        IntentKind::Emcon { on: true },
        IntentKind::Fire {
            weapon: SmolStr::new("cannon"),
            slot: Some(1),
        },
        IntentKind::Condition(Arming::Disarm {
            weapon: Some(SmolStr::new("cannon")),
        }),
    ];
    assert_eq!(
        recorded.think(RED, 0).result.intents,
        vec![IntentKind::Startup]
    );
    assert_eq!(kept.result.fault, None);
    assert_eq!(kept.result.intents, expected, "the call order");
    assert_eq!(kept.result.trace, vec![STATION_TRACE.to_owned()]);
    // The one door's input: the answer on the row's `pending` (17b cut
    // M3.2, D-UM-23 as amended), at the think's epochs, one substep later.
    let landing = rig
        .arena
        .minds
        .get(&RED)
        .and_then(|state| state.pending.clone())
        .expect("think 3 waits to land");
    assert_eq!(landing.think_seq, 3);
    assert_eq!(landing.intents, expected);
    assert_eq!(
        landing.epochs,
        Channel::ALL.map(|channel| rig.arena.epochs.current(RED, channel))
    );
    assert!(rig.arena.due.iter().any(|event| event.kind
        == DueKind::MindLand {
            unit: RED,
            think_seq: 3
        }
        && event.t_ms == think + SUBSTEP));
    rig.to(think_at(RED, 5) + SUBSTEP);

    // Landed, in order, at the landing.
    let journal = rig.journal();
    let at = |kind: &str, unit: ObjId| -> Vec<(i64, Json)> {
        journal
            .iter()
            .filter_map(|event| {
                let json = serde_json::to_value(&event.kind).unwrap_or_default();
                let (word, on) = kind_and_unit(&json.to_string());
                (word == kind && on == Some(unit.0)).then_some((event.t_ms, json))
            })
            .collect()
    };
    let startup: Vec<i64> = at("startup", RED).iter().map(|(t, _)| *t).collect();
    assert_eq!(startup, vec![think_at(RED, 0) + SUBSTEP]);
    let locked: Vec<i64> = at("locked", RED).iter().map(|(t, _)| *t).collect();
    assert_eq!(locked, vec![think + SUBSTEP]);
    let fired = at("fired", RED);
    assert_eq!(fired.len(), 1, "{fired:?}");
    assert_eq!(fired[0].0, think + SUBSTEP);
    assert_eq!(fired[0].1["by"], "mind");
    assert_eq!(fired[0].1["slot"], 1);
    let red = rig.arena.unit(RED).expect("red");
    assert_eq!(red.lock, Some(blue), "the lock held");
    assert_eq!(rig.arena.motors.row(RED).and_then(|row| row.goal), None);
    // The heading and the way the script asked for, after the goal was let
    // go (no later think of the script moves either).
    assert_eq!(
        red.desired_heading_mdeg, 45_000,
        "the helm landed after the goal"
    );
    assert_eq!(red.desired_speed_cms, 300, "the way landed after the goal");
    // Every refusal of the landing comes back as a notice in the next view
    // (`Arena::mind_land`): the pod's is the ONLY one, so the arming, both
    // cells, the hold, the release, the helm, the way, the lock, the shot and
    // the disarm all applied.
    let next = recorded.think(RED, 4);
    let refusals: Vec<(String, Refusal)> = next
        .view
        .notices
        .iter()
        .filter_map(|notice| match notice {
            FrameNotice::Refused { order, refusal, .. } => Some((order.to_string(), *refusal)),
            _ => None,
        })
        .collect();
    assert_eq!(
        refusals,
        vec![("emcon".to_owned(), Refusal::NoJammer)],
        "the pod's refusal, and only it: {:?}",
        next.view.notices
    );
    assert!(
        next.view.weapons.iter().all(|row| row.armed.is_none()),
        "the slot let go"
    );

    // The trace is in no ledger line.
    for event in &journal {
        let line = serde_json::to_string(event).unwrap_or_default();
        assert!(!line.contains(STATION_TRACE), "{line}");
        assert!(!line.contains("no weapons station"), "{line}");
    }

    // The ungranted fire station: an intent, a trace line, a refusal at the
    // landing — never a fault.
    let unarmed = recorded.think(RED2, 3);
    assert_eq!(unarmed.result.fault, None);
    assert_eq!(
        unarmed.result.intents,
        vec![IntentKind::Fire {
            weapon: SmolStr::new("cannon"),
            slot: Some(1),
        }]
    );
    assert_eq!(unarmed.result.trace, vec![UNGRANTED_FIRE.to_owned()]);
    assert!(
        recorded
            .think(RED2, 4)
            .view
            .notices
            .iter()
            .any(|notice| matches!(
                notice,
                FrameNotice::Refused { order, refusal: Refusal::Channel, .. } if order == "fire"
            )),
        "the ungranted station refused channel at its landing"
    );
    for unit in [RED, RED2] {
        assert_eq!(
            rig.arena.minds.get(&unit).map(|state| state.strikes),
            Some(0)
        );
    }
    assert_eq!(rig.counters.mind_faults, 0);

    // The same answers from a script: the same arena.
    let mut again = stations_field(replay(&recorded.kept(), |_| panic!("no fault here")));
    again.to(think_at(RED, 5) + SUBSTEP);
    assert_eq!(again.hashes, rig.hashes, "the substep hashes");
    let replayed: Vec<(i64, String, u64)> = again
        .journal()
        .iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    let original: Vec<(i64, String, u64)> = journal
        .iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    assert_eq!(replayed, original, "the event list");
    assert!(!runner.take_stats().is_empty(), "the runner recorded");
}

// ---------------------------------------------------------------------------
// (3) Every fault word, and the drop
// ---------------------------------------------------------------------------

/// The gate every fault script opens with: only the armed red faults, and
/// its first think writes `n` and lands clean — so every later think faults
/// over a memory of `{"n":1}`.
const GATE: &str = r#"if v.unit != 4 { return; } if !("n" in this) { this.n = 1; return; }"#;

/// How a fault's body is wrapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wrap {
    /// As written.
    Bare,
    /// `try { body } catch { }`.
    Try,
    /// `try { body } catch (e) { this.caught = true; }`.
    TryBound,
    /// `try { act(v); } catch { }` with the body in a helper function (only
    /// for a body that does not write `this`).
    Helper,
}

/// One fault's fixture: its word, the functions it needs, its body, whether
/// it writes `this`, and whether it runs on the second sandbox.
struct Case {
    word: &'static str,
    prelude: &'static str,
    body: &'static str,
    writes_this: bool,
    boom: bool,
}

/// **Every cause word, one fixture each** (doc 17 §8, D-UM-31).
fn cases() -> Vec<Case> {
    fn case(
        word: &'static str,
        prelude: &'static str,
        body: &'static str,
        writes_this: bool,
    ) -> Case {
        Case {
            word,
            prelude,
            body,
            writes_this,
            boom: false,
        }
    }
    vec![
        case("ops", "", "loop { }", false),
        // Thirteen calls deep from `think`: past the twelve call levels.
        case(
            "limit",
            "fn deep(n) { if n > 0 { deep(n - 1) } else { 0 } }",
            "deep(12);",
            false,
        ),
        case("limit", "", "let s = \"x\"; loop { s += s; }", false),
        case("type", "", "helm(1.5);", false),
        case("type", "", "this.x = 1.5;", true),
        // `n` and thirty-two more: a thirty-third key.
        case(
            "memory",
            "",
            "for i in 0..32 { this[\"k\" + i.to_string()] = i; }",
            true,
        ),
        case("intents", "", "for i in 0..17 { helm(i); }", false),
        case("unknown", "", "lock(999);", false),
        case(
            "limit",
            "",
            "let id = v.contacts[0].id; line_to(v, id); line_to(v, id); line_to(v, id);",
            false,
        ),
        Case {
            word: "panic",
            prelude: "",
            body: "boom();",
            writes_this: false,
            boom: true,
        },
    ]
}

/// A fault case's source under `wrap`.
fn fault_source(case: &Case, wrap: Wrap) -> String {
    let (prelude, body) = (case.prelude, case.body);
    match wrap {
        Wrap::Bare => format!("{prelude}\nfn think(v, c) {{ {GATE} {body} }}"),
        Wrap::Try => format!("{prelude}\nfn think(v, c) {{ {GATE} try {{ {body} }} catch {{ }} }}"),
        Wrap::TryBound => format!(
            "{prelude}\nfn think(v, c) {{ {GATE} try {{ {body} }} catch (e) {{ this.caught = true; }} }}"
        ),
        Wrap::Helper => format!(
            "{prelude}\nfn act(v) {{ {body} }}\nfn think(v, c) {{ {GATE} try {{ act(v); }} catch {{ }} }}"
        ),
    }
}

/// **The second sandbox**: the standard one's limits and API, and a
/// test-only `boom()` that panics.
fn boom_sandbox() -> Arc<Sandbox> {
    Arc::new(Sandbox::new(
        &EngineLimits {
            max_ops_backstop: STOCK_OPS_PER_THINK,
        },
        |engine: &mut Engine| {
            register_api(engine);
            engine.register_fn("boom", || -> INT { panic!("boom") });
        },
    ))
}

/// The runner for `source`, on the second sandbox when `boom`.
fn fault_runner(arena: &Arena, source: &str, boom: bool) -> Arc<RhaiRunner> {
    if boom {
        Arc::new(
            RhaiRunner::on(boom_sandbox(), &arena.arena_cfg, source, digest(source))
                .unwrap_or_else(|error| panic!("{source}: {error}"))
                .recording(),
        )
    } else {
        rhai(arena, source)
    }
}

/// One fault run: the (j) field flown by `source` to just past the armed
/// red's think `last`, pulse by pulse; the recorder, the runner, red's row
/// after each pulse, whether red had a landing waiting after each pulse (its
/// answer on the row, a landing on the heap), and the rig.
struct FaultRun {
    recorded: Arc<Recorded>,
    runner: Arc<RhaiRunner>,
    rows: Vec<tacsim::arena::mind::MindState>,
    landings: Vec<(bool, bool)>,
    rig: Rig,
}

/// Whether red has a landing waiting: its answer on the row's `pending` (17b
/// cut M3.2 retired the scratch it sat on), and a `MindLand` on the heap.
fn red_landing(arena: &Arena) -> (bool, bool) {
    (
        arena
            .minds
            .get(&RED)
            .is_some_and(|state| state.pending.is_some()),
        arena
            .due
            .iter()
            .any(|event| matches!(event.kind, DueKind::MindLand { unit, .. } if unit == RED)),
    )
}

fn fault_run(source: &str, boom: bool, last: i64) -> FaultRun {
    let mut arena = the_j_field(Controller::Mind);
    let runner = fault_runner(&arena, source, boom);
    let recorded = Recorded::on(runner.clone());
    arena.mind_runner = recorded.clone();
    let mut rig = Rig::new(arena);
    let mut rows = Vec::new();
    let mut landings = Vec::new();
    while rig.arena.now_ms < think_at(RED, last) + SUBSTEP {
        let _ = rig.pulse();
        rows.push(rig.arena.minds.get(&RED).cloned().expect("red's row"));
        landings.push(red_landing(&rig.arena));
    }
    FaultRun {
        recorded,
        runner,
        rows,
        landings,
        rig,
    }
}

/// The pulse after which red's think `n` had run: the first pulse (wall 0)
/// anchors arena time and runs no substep, and pulse `k` ends at `k`
/// substeps, so the think at `t` is in pulse `ceil(t / SUBSTEP)`.
fn pulse_of_think(n: i64) -> usize {
    usize::try_from((think_at(RED, n) + SUBSTEP - 1) / SUBSTEP).unwrap_or(0)
}

/// **Every fault has its word, lands nothing and counts a strike; three
/// running drop the hull** (doc 17 §8, D-UM-31, D-UM-32; 17b §1 (v)). One
/// script per cause on the (j) field, through `Arena::mind_think`: the armed
/// red's think 0 lands clean (`{"n":1}`), its think 1 runs the body —
///
/// * `loop {}` → `ops`, at the SAME operation count on two runs; a thirteen
///   deep recursion → `limit`; `s += s` → `limit`; `helm(1.5)` → `type`;
///   `this.x = 1.5` → `type`; a thirty-third memory key → `memory`;
///   seventeen `helm`s → `intents`; `lock(999)` → `unknown`; a third
///   `line_to` → `limit`; a native `boom()` that panics, on a SECOND sandbox
///   through `RhaiRunner::on` → `panic`;
/// * each wrapped in `try { } catch { }`, in `catch (e)` writing the memory,
///   and (where the body writes no `this`) in a helper called inside `try`
///   → the same word;
/// * each faulted think answers no intents and its own memory, lands
///   nothing (no answer pending on the row and no landing on the heap at the end of the pulse
///   that ran it — where a clean think's landing still waits, one substep
///   off the lattice — and `pending` clear), keeps `{"n":1}` past the
///   millisecond a landing would have written the memory, and counts a
///   strike (`strikes` 1) and a `mind_fault`;
/// * `exit()` is NOT a fault: the intents buffered before it land;
/// * three faults running drop the hull: `ThinkJob::dropped` from think 4,
///   answered by the native stand-in — equal to `NativeStockMind::think` on
///   the same job — with [`DROPPED_LINE`] its whole trace and no stat
///   recorded, the strikes held at 3 through clean thinks;
/// * and the hash at every substep equals M2.2's baseline re-run here: a
///   [`ScriptedMind`] answering the Rhai run's clean thinks as they were
///   answered and its faulted thinks with SEVENTEEN intents (D-UM-9's
///   overflow), with red's `strikes` written by hand after every pulse to the
///   Rhai run's figure (and found already there) — the Rhai runner's faults
///   take the arena's one fault path and nothing else.
#[test]
#[allow(clippy::too_many_lines)]
fn m2_every_fault_has_its_word_and_three_drop_the_hull() {
    let after_one = pulse_of_think(1);
    for case in cases() {
        let source = fault_source(&case, Wrap::Bare);
        let run = fault_run(&source, case.boom, 1);
        let kept = run.recorded.think(RED, 1);
        assert_eq!(
            kept.result.fault.as_ref().map(|fault| fault.cause.as_str()),
            Some(case.word),
            "{}: {:?}",
            case.body,
            kept.result.trace
        );
        assert!(kept.result.intents.is_empty(), "{}", case.body);
        assert_eq!(kept.result.memory, kept.memory, "{}", case.body);
        assert_eq!(kept.memory, b"{\"n\":1}".to_vec(), "{}", case.body);
        let row = run.rows.get(after_one).expect("the pulse of think 1");
        assert_eq!(row.think_seq, 2, "{}: think 1 ran", case.body);
        assert_eq!(row.strikes, 1, "{}: a strike", case.body);
        assert_eq!(row.pending, None, "{}", case.body);
        // At the end of the pulse that ran think 1 — a clean think's landing
        // would still wait there (the `exit()` case below shows it does).
        assert_eq!(
            run.landings.get(after_one),
            Some(&(false, false)),
            "{}: no answer pending and no landing on the heap",
            case.body
        );
        // Past the millisecond a landing would have written it.
        assert!(run.rig.arena.now_ms >= think_at(RED, 1) + SUBSTEP);
        assert_eq!(
            run.rows.last().map(|row| row.memory.clone()),
            Some(b"{\"n\":1}".to_vec()),
            "{}: the memory kept",
            case.body
        );
        assert_eq!(run.rig.counters.mind_faults, 1, "{}", case.body);
        let stats: Vec<ThinkStat> = run
            .runner
            .take_stats()
            .into_iter()
            .filter(|stat| stat.unit == RED)
            .collect();
        assert_eq!(
            stats
                .iter()
                .map(|stat| (stat.think_seq, stat.faulted))
                .collect::<Vec<_>>(),
            vec![(0, false), (1, true)],
            "{}",
            case.body
        );
        for wrap in [Wrap::Try, Wrap::TryBound, Wrap::Helper] {
            if wrap == Wrap::Helper && case.writes_this {
                continue;
            }
            let wrapped = fault_source(&case, wrap);
            let run = fault_run(&wrapped, case.boom, 1);
            let kept = run.recorded.think(RED, 1);
            assert_eq!(
                kept.result.fault.as_ref().map(|fault| fault.cause.as_str()),
                Some(case.word),
                "{wrap:?} {}: {:?}",
                case.body,
                kept.result.trace
            );
            assert!(kept.result.intents.is_empty());
            assert_eq!(
                run.rows.get(after_one).map(|row| row.strikes),
                Some(1),
                "{wrap:?} {}",
                case.body
            );
            assert_eq!(
                run.landings.get(after_one),
                Some(&(false, false)),
                "{wrap:?} {}: no landing",
                case.body
            );
        }
    }

    // The depth fixture faults on its depth: the same recursion five deep
    // thinks clean.
    let shallow = format!(
        "fn deep(n) {{ if n > 0 {{ deep(n - 1) }} else {{ 0 }} }}\nfn think(v, c) {{ {GATE} deep(5); }}"
    );
    let run = fault_run(&shallow, false, 1);
    assert_eq!(run.recorded.think(RED, 1).result.fault, None);

    // `ops` at the same count on two runs.
    let ops = |_: ()| {
        let run = fault_run(&fault_source(&cases()[0], Wrap::Bare), false, 1);
        run.runner
            .take_stats()
            .into_iter()
            .find(|stat| stat.unit == RED && stat.think_seq == 1)
            .expect("think 1's figures")
    };
    let (one, two) = (ops(()), ops(()));
    assert!(one.faulted);
    assert_eq!((one.ops, one.fee), (two.ops, two.fee), "the same trip");
    assert!(one.ops + one.fee > STOCK_OPS_PER_THINK);

    // `exit()` is a return: the buffered intents land — the start at think
    // 1, the heading (once the power is up) at think 2 — and nothing after
    // the exit runs.
    let source = format!(
        "fn think(v, c) {{ {GATE} if v.power != \"online\" {{ startup(); exit(); trace(\"never\"); }} helm(45000); exit(); helm(0); }}"
    );
    let run = fault_run(&source, false, 2);
    let start = run.recorded.think(RED, 1);
    assert_eq!(start.result.fault, None);
    assert_eq!(start.result.intents, vec![IntentKind::Startup]);
    // The fault cases' landing check is not vacuous: a clean think 1's
    // landing is waiting, its answer on the row and on the heap, at the end of the
    // pulse that ran it.
    assert_eq!(run.landings.get(after_one), Some(&(true, true)));
    assert!(start.result.trace.is_empty(), "nothing after the exit ran");
    let kept = run.recorded.think(RED, 2);
    assert_eq!(kept.result.fault, None);
    assert_eq!(
        kept.result.intents,
        vec![IntentKind::Heading { mdeg: 45_000 }]
    );
    assert!(run.rows.iter().all(|row| row.strikes == 0));
    assert_eq!(
        run.rig
            .arena
            .unit(RED)
            .map(|unit| unit.desired_heading_mdeg),
        Some(45_000),
        "the heading before the exit landed"
    );

    // Three running drop the hull.
    let looping = fault_source(&cases()[0], Wrap::Bare);
    let dropped = fault_run(&looping, false, 6);
    let kept = dropped.recorded.kept();
    let strikes_after = |n: i64| {
        dropped
            .rows
            .get(pulse_of_think(n))
            .map(|row| row.strikes)
            .expect("a row")
    };
    for (n, strikes) in [(0, 0), (1, 1), (2, 2), (3, 3), (4, 3), (5, 3), (6, 3)] {
        assert_eq!(strikes_after(n), strikes, "strikes after think {n}");
        let think = dropped.recorded.think(RED, u64::try_from(n).unwrap_or(0));
        assert_eq!(think.dropped, n >= 4, "ThinkJob::dropped at think {n}");
    }
    assert_eq!(MIND_STRIKES_TO_DROP, 3);
    for n in 4..=6 {
        let think = dropped.recorded.think(RED, n);
        assert_eq!(think.result.trace, vec![DROPPED_LINE.to_owned()]);
        let mut native =
            NativeStockMind::with(&dropped.rig.arena.arena_cfg.reflex).think(&think.job());
        native.trace = vec![DROPPED_LINE.to_owned()];
        assert_eq!(think.result, native, "the stand-in's think {n}");
    }
    // The stand-in's memory is Standard Doctrine's fourteen keys, never the
    // script's `{"n":1}`.
    assert_ne!(
        dropped.recorded.think(RED, 4).result.memory,
        b"{\"n\":1}".to_vec(),
        "the stand-in answered"
    );
    let stats = dropped.runner.take_stats();
    assert!(
        stats
            .iter()
            .filter(|stat| stat.unit == RED)
            .all(|stat| stat.think_seq <= 3),
        "a dropped think records no Rhai figure"
    );

    // The baseline: M2.2's overflow path, the strikes as the Rhai run's.
    let speed = IntentKind::Speed { cms: 300 };
    let baseline = replay(&kept, |kept| ThinkResult {
        diagnostics: None,
        rule_sample: None,
        intents: vec![speed.clone(); MIND_INTENTS_MAX + 1],
        memory: kept.memory.clone(),
        fault: None,
        trace: Vec::new(),
    });
    let mut arena = the_j_field(Controller::Mind);
    arena.mind_runner = baseline;
    let mut flow = Rig::new(arena);
    for (pulse, row) in dropped.rows.iter().enumerate() {
        let _ = flow.pulse();
        if let Some(state) = flow.arena.minds.get_mut(&RED) {
            assert_eq!(
                state.strikes, row.strikes,
                "the overflow counts a strike, as the fault does (pulse {pulse})"
            );
            state.strikes = row.strikes;
        }
    }
    assert_eq!(flow.hashes.len(), dropped.rig.hashes.len());
    for (pulse, (rhai, overflow)) in dropped.rig.hashes.iter().zip(&flow.hashes).enumerate() {
        assert_eq!(rhai, overflow, "the substep hashes of pulse {pulse}");
    }
    assert_eq!(
        (flow.counters.mind_faults, flow.counters.mind_overflows),
        (3, 3)
    );
    assert_eq!(
        (
            dropped.rig.counters.mind_faults,
            dropped.rig.counters.mind_overflows
        ),
        (3, 0)
    );
}

// ---------------------------------------------------------------------------
// (4) Every plot kernel holds the plot while the truth moves
// ---------------------------------------------------------------------------

/// **Every kernel once, into memory**, on the first classified hostile.
const KERNELS: &str = r#"
fn think(v, c) {
    let id = ();
    for k in v.contacts { if k.identified && !k.friendly { id = k.id; } }
    let k = contact(v, id);
    this.health = health_permille(v);
    this.best = best_hostile(v, 3, 2, 1, health_permille(v), (), (), (), ());
    this.line = line_to(v, id);
    this.stand = stand_off_cell(v, id, 3000);
    this.found = k.designator;
    this.odds = k.odds("cannon");
    this.aspect = k.aspect;
    let r = ranges(v, 800, false);
    this.free = [r.engage_cm, r.release_cm, r.stand_cm, r.limit_cm];
    let o = ranges(v, 800, true);
    this.obeyed = [o.engage_cm, o.release_cm, o.stand_cm, o.limit_cm];
    this.bearing = bearing_to(v, k.cell);
    this.range = range_to(v, k.cell);
    this.entry = entry_cell(v);
    this.far = far_entry_cell(v);
    let w = patrol_step(v, false, (), (), ());
    this.walk_wp = w.waypoint;
    this.walk_from = w.from_waypoint;
    this.walk_goal = w.goal;
    this.walk_clear = w.clear_route;
    let f = fall_back_to_waypoint(v, k.cell, (), ());
    this.fall_wp = f.waypoint;
    this.fall_from = f.from_waypoint;
    this.fall_goal = f.goal;
}
"#;

/// **What each kernel's ONE free function answers on `view` and `def`**, as
/// the kernels script writes it.
fn kernels_by_hand(view: &MindView, def: &ArenaDefinition) -> Memory {
    let id = hostile(view);
    let row = view
        .contacts
        .iter()
        .find(|row| row.id == id)
        .expect("the row");
    let here = view.cell.expect("on the window");
    let plot = row.cell.expect("a plot on the window");
    let own = (
        view.x_cm,
        view.y_cm,
        i32::try_from(view.z_cm).unwrap_or(i32::MAX),
    );
    let point = cell_point(def, plot);
    let health = health_permille(view);
    let ints = |ranges: tacsim::arena::doctrine::Ranges| {
        MemValue::Ints(
            [
                ranges.engage_cm,
                ranges.release_cm,
                ranges.stand_cm,
                ranges.limit_cm,
            ]
            .iter()
            .map(|cm| i64::try_from(*cm).unwrap_or(i64::MAX))
            .collect(),
        )
    };
    let walk = motor::patrol_step(
        def,
        view.unit,
        view.cell,
        false,
        motor::Walk {
            waypoint: None,
            from_waypoint: None,
            goal: None,
        },
    );
    let (fall_wp, fall_from, fall_goal) =
        motor::fallback_waypoint(def, here, Some(plot), None, None, view.unit);
    let mut memory = Memory::new();
    let mut put = |key: &str, value: MemValue| {
        memory.insert(key.to_owned(), value);
    };
    put("health", MemValue::Int(i64::from(health)));
    put(
        "best",
        best_hostile(
            view,
            &Weights {
                range: 3,
                facet: 2,
                health: 1,
            },
            health,
            None,
            |_| true,
        )
        .map_or(MemValue::Unit, |id| MemValue::Int(contact_int(id))),
    );
    put("line", MemValue::Bool(line_to(def, here, plot)));
    put(
        "stand",
        cell_value(motor::stand_off_cell(
            def,
            CostRow::Tracked,
            own,
            point,
            3_000,
        )),
    );
    put("found", MemValue::Str(row.designator.to_string()));
    put(
        "odds",
        opt_int(
            row.odds
                .iter()
                .find(|(weapon, _)| weapon == "cannon")
                .map(|(_, p)| *p),
        ),
    );
    put("aspect", MemValue::Str(row.aspect.as_str().to_owned()));
    put("free", ints(view_ranges(view, 800, None)));
    put("obeyed", ints(view_ranges(view, 800, view.order.as_ref())));
    put(
        "bearing",
        MemValue::Int(i64::from(bearing_of(point.0 - own.0, point.1 - own.1))),
    );
    put(
        "range",
        MemValue::Int(
            i64::try_from(range_between(
                own.0, own.1, own.2, point.0, point.1, point.2,
            ))
            .unwrap_or(i64::MAX),
        ),
    );
    put(
        "entry",
        cell_value(motor::entry_cell(def, view.team, CostRow::Tracked, here)),
    );
    put(
        "far",
        cell_value(motor::far_entry_cell(
            def,
            view.team,
            CostRow::Tracked,
            here,
            &[],
        )),
    );
    put("walk_wp", opt_int(walk.walk.waypoint));
    put("walk_from", opt_int(walk.walk.from_waypoint));
    put("walk_goal", cell_value(walk.walk.goal));
    put("walk_clear", MemValue::Bool(walk.clear_route));
    put("fall_wp", opt_int(fall_wp));
    put("fall_from", opt_int(fall_from));
    put("fall_goal", cell_value(fall_goal));
    memory
}

/// The kernels script's answer on one view.
fn kernels_on(runner: &RhaiRunner, view: &Arc<MindView>, arena: &Arena) -> Memory {
    let runtime = &arena.profiles[&RED];
    let card = runtime.card.clone();
    let result = runner.think(&ThinkJob {
        unit: RED,
        think_seq: 0,
        t_ms: view.t_ms,
        lands_ms: view.t_ms + SUBSTEP,
        seed: arena.seed,
        dropped: false,
        profile: &tacsim::arena::core::ClassTable::STOCK.row(tacsim::arena::core::CoreClass::I),
        view,
        memory: &[],
        map: &arena.definition,
        card: &card,
        program: &runtime.program,
        profile_token: runtime.token(),
    });
    assert_eq!(result.fault, None, "{:?}", result.trace);
    decode_memory(&result.memory)
}

/// **Every plot kernel holds the plot while the truth moves** (doc 17 §3
/// "The map is public", D-UM-4; the `m1a6_every_kernel_holds_the_plot_while_
/// the_truth_moves` shape; `tests/motor.rs`' audit for the pickers). On the
/// (j) field with the armed red holding the blue classified and a standing
/// order capping the fight at the `close` band:
///
/// * the view is built; the blue's LIVE pose is moved by hand — three cells
///   off, turned about, onto the very cell red's entry picker answers — and
///   the spatial index rebuilt (no pass between); the view built again is
///   EQUAL, and a script calling `best_hostile`, `line_to`,
///   `stand_off_cell`, `contact`, `odds`, `aspect`, `ranges`, `bearing_to`,
///   `range_to`, `entry_cell`, `far_entry_cell`, `patrol_step` and
///   `fall_back_to_waypoint` answers the same on both: the plot held, the
///   ground held, the index moved;
/// * every answer equals the kernel's ONE free function on the same inputs
///   (the one-body rule);
/// * **`ranges`' order arm**: `ranges(v, band, true)` is `view_ranges(view,
///   band, Some(order))` and `ranges(v, band, false)` is `view_ranges(view,
///   band, None)`, and the two differ here — the arm the port's obeyed-order
///   local selects.
#[test]
fn m2_every_plot_kernel_holds_the_plot_while_the_truth_moves() {
    let mut arena = the_j_field(Controller::Mind);
    arena.mind_runner = Arc::new(ScriptedMind {
        answers: [(
            (RED, 0),
            ThinkResult {
                intents: vec![IntentKind::Startup],
                ..ThinkResult::default()
            },
        )]
        .into_iter()
        .collect(),
    });
    let mut rig = Rig::new(arena);
    rig.to(think_at(RED, 4));
    let blue = hostile(&rig.arena.clone().mind_view(RED).expect("a view"));
    if let Some(state) = rig.arena.minds.get_mut(&RED) {
        state.order = Some(FrameOrder {
            squad: SmolStr::new("red-1"),
            objective: MindObjective::Engage { target: Some(blue) },
            hold_fire: false,
            max_range_band: Some(RangeBand::Close),
            expires_ms: rig.arena.now_ms + 60_000,
        });
    }
    let before = Arc::new(rig.arena.clone().mind_view(RED).expect("a view"));
    let by_hand = kernels_by_hand(&before, &rig.arena.definition);
    let entry = match by_hand.get("entry") {
        Some(MemValue::Ints(cell)) => (
            u16::try_from(cell[0]).unwrap_or(0),
            u16::try_from(cell[1]).unwrap_or(0),
        ),
        other => panic!("an entry cell: {other:?}"),
    };
    let mut moved = rig.arena.clone();
    if let Some(state) = moved.unit_mut(BLUE) {
        let at = centre(entry.0, entry.1);
        state.pose.x_cm = at.x_cm;
        state.pose.y_cm = at.y_cm;
        state.pose.heading_mdeg = 180_000;
    }
    moved.index.rebuild(&moved.definition.terrain, &moved.units);
    let after = Arc::new(moved.clone().mind_view(RED).expect("a view"));
    assert_eq!(before, after, "a live move is not on the view");

    let runner = rhai(&rig.arena, KERNELS);
    let first = kernels_on(&runner, &before, &rig.arena);
    let second = kernels_on(&runner, &after, &moved);
    assert_eq!(first, second, "the truth moved, the answers held");
    assert_eq!(first, by_hand, "each kernel is its one free function");
    assert_eq!(
        kernels_by_hand(&after, &moved.definition),
        by_hand,
        "and so is the free function on the moved arena's ground"
    );
    // The order arm, both ways, and the two differ.
    let order = before.order.as_ref().expect("the order on the view");
    assert_eq!(order.max_range_band, Some(RangeBand::Close));
    assert_ne!(
        view_ranges(&before, 800, Some(order)),
        view_ranges(&before, 800, None),
        "the order must move the ranges here"
    );
    assert_ne!(mem(&first, "obeyed"), mem(&first, "free"));
    // The facts the test leans on.
    assert_eq!(mem(&first, "best"), MemValue::Int(contact_int(blue)));
    assert_eq!(mem(&first, "line"), MemValue::Bool(true));
}

// ---------------------------------------------------------------------------
// (5) rand(k) on the arena's seed
// ---------------------------------------------------------------------------

/// Three `rand(6)` and a fourth `rand(1000)` each think, into memory.
const RAND: &str = r#"
fn think(v, c) {
    this.r = [rand(6), rand(6), rand(6), rand(1000)];
}
"#;

/// The same think drawing nothing.
const NO_RAND: &str = r#"
fn think(v, c) {
    this.r = [0, 0, 0, 0];
}
"#;

/// A rand run on `seed`: the recorder and the run.
fn rand_run(source: &str, seed: u64) -> (Arc<Recorded>, Run) {
    let mut arena = the_j_field(Controller::Mind);
    arena.seed = seed;
    let recorded = Recorded::on(rhai(&arena, source));
    arena.mind_runner = recorded.clone();
    let run = run(&mut arena, Drive::Every(1_000), 8_000);
    (recorded, run)
}

/// **`rand(k)` draws the sixth label on the arena's seed** (D-UM-29; 17b
/// §1 (iv)): at every think `s` of each hull, the calls 0, 1 and 2 of
/// `rand(6)` answer `draw(arena.seed, Mind { unit, think_seq: s, n }) % 6`
/// for `n` 0, 1, 2, and a fourth call `rand(1000)` is `n` 3 — the lane's
/// count moves by exactly the calls made, and starts again at the next think;
/// two runs answer the same; another seed answers otherwise; and the mind's
/// draws are the mind lane's own: no arena line carries a `mind` label, and
/// the arena's own draws are those of a run whose mind draws nothing.
#[test]
fn m2_rand_draws_the_sixth_label_on_the_arenas_seed() {
    let (recorded, first) = rand_run(RAND, SEED);
    let kept = recorded.kept();
    assert!(kept.len() >= 10, "{} thinks", kept.len());
    for think in &kept {
        assert_eq!(think.seed, SEED, "the arena's own seed");
        let label = |n: u32| DrawLabel::Mind {
            unit: think.unit.0,
            think_seq: think.think_seq,
            n,
        };
        let want: Vec<i64> = (0..3)
            .map(|n| i64::try_from(draw(SEED, &label(n)) % 6).unwrap_or(-1))
            .chain(std::iter::once(
                i64::try_from(draw(SEED, &label(3)) % 1_000).unwrap_or(-1),
            ))
            .collect();
        assert_eq!(
            mem(&think.memory_out(), "r"),
            MemValue::Ints(want),
            "#{} think {}",
            think.unit.0,
            think.think_seq
        );
    }
    let answers = |recorded: &Recorded| {
        recorded
            .kept()
            .into_iter()
            .map(|kept| (kept.unit, kept.think_seq, kept.result.memory))
            .collect::<Vec<_>>()
    };
    let (again, second) = rand_run(RAND, SEED);
    assert_eq!(answers(&again), answers(&recorded), "two runs, one answer");
    assert_eq!(second.hashes, first.hashes);
    let (other, _) = rand_run(RAND, SEED ^ 0x5eed_0000_0000_0001);
    assert_ne!(answers(&other), answers(&recorded), "another seed");
    // The mind lane's draws never reach the arena's lines.
    let (_, quiet) = rand_run(NO_RAND, SEED);
    assert!(
        !first.draws.is_empty(),
        "the arena drew (its classifications)"
    );
    assert_eq!(first.draws, quiet.draws, "the arena's own draws");
    for (_, draws) in &first.draws {
        for (label, _) in draws {
            assert!(
                !label.starts_with("mind:"),
                "a mind label on the tape: {label}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// (6) The card type reads the card the arena holds
// ---------------------------------------------------------------------------

/// The card's fields into memory every think.
const CARD: &str = r#"
fn think(v, c) {
    this.withdraw = c.withdraw_permille;
    this.stand = c.stand_min_permille;
    this.stance = c.stance;
}
"#;

/// **The card type reads the card the arena holds** (doc 17 §11, D-UM-27,
/// D-UM-43): a `Rearguard` staged through `Arena::stage_card` lands at the
/// next pulse's start, and the armed red's next think reads
/// `card.withdraw_permille` 700 — every think before it reads Line's, and
/// every job's card is the row's own.
#[test]
fn m2_the_card_type_reads_the_card_the_arena_holds() {
    let mut arena = the_j_field(Controller::Mind);
    let recorded = Recorded::on(rhai(&arena, CARD));
    arena.mind_runner = recorded.clone();
    let line = tacsim::arena::line(&arena.arena_cfg.reflex);
    let rearguard = tacsim::arena::preset("Rearguard", &arena.arena_cfg.reflex).expect("a preset");
    assert_eq!(rearguard.withdraw_permille, 700);
    let mut rig = Rig::new(arena);
    rig.to(think_at(RED, 2) + SUBSTEP);
    rig.arena
        .stage_card(RED, CardChange::Preset(SmolStr::new("Rearguard")));
    rig.to(think_at(RED, 4) + SUBSTEP);
    for think in recorded.kept().iter().filter(|kept| kept.unit == RED) {
        let card = if think.think_seq <= 2 {
            &line
        } else {
            &rearguard
        };
        assert_eq!(think.card.stock(), Some(card), "think {}", think.think_seq);
        let memory = think.memory_out();
        assert_eq!(
            mem(&memory, "withdraw"),
            MemValue::Int(i64::from(card.withdraw_permille)),
            "think {}",
            think.think_seq
        );
        assert_eq!(
            mem(&memory, "stand"),
            MemValue::Int(i64::from(card.stand_min_permille))
        );
    }
    assert_eq!(
        mem(&recorded.think(RED, 3).memory_out(), "withdraw"),
        MemValue::Int(700)
    );
    assert_eq!(
        rig.arena
            .profiles
            .get(&RED)
            .and_then(|runtime| runtime.card.stock())
            .cloned(),
        Some(rearguard)
    );
}

// ---------------------------------------------------------------------------
// (7) The mandatory cross-tempo fixture
// ---------------------------------------------------------------------------

/// **A scripted Rhai mind**: each red drives its own square — a corner, and
/// at the `drive_arrived` notice `hold()` and the next corner at the next
/// think — locks the first classified hostile it holds, keeps `lands_ms()`
/// and its count of arrivals in memory, and traces what it does.
const SQUARE: &str = r#"
fn corner(unit, leg) {
    let square = if unit == 4 { [[3, 4], [3, 7], [6, 7], [6, 4]] } else { [[3, 11], [3, 14], [6, 14], [6, 11]] };
    square[leg % 4]
}
fn think(v, c) {
    if !("leg" in this) {
        this.leg = 0; this.at = -1; this.holds = 0; this.lands = 0;
        startup();
        trace("startup");
        return;
    }
    this.lands = lands_ms();
    if v.power != "online" { return; }
    let arrived = false;
    for n in v.notices { if n.kind == "drive_arrived" { arrived = true; } }
    if arrived && this.at == this.leg {
        hold();
        this.holds += 1;
        this.leg += 1;
        trace("arrived; holding at leg " + this.at.to_string());
    } else if this.at != this.leg {
        goto(corner(v.unit, this.leg));
        this.at = this.leg;
        trace("leg " + this.leg.to_string() + " lands at " + lands_ms().to_string());
    }
    if v.lock == () {
        for k in v.contacts {
            if k.identified && !k.friendly { lock(k.id); trace("lock " + k.designator); break; }
        }
    }
}
"#;

/// The square at `drive`: the run, the trace sequence and the last memory of
/// each red.
fn square_run(drive: Drive) -> (Run, Traces, Vec<Memory>) {
    let mut arena = the_j_field(Controller::Mind);
    let recorded = Recorded::on(rhai(&arena, SQUARE));
    arena.mind_runner = recorded.clone();
    let run = run(&mut arena, drive, 40_000);
    let memories = [RED, RED2]
        .iter()
        .map(|unit| {
            arena
                .minds
                .get(unit)
                .map(|state| decode_memory(&state.memory))
                .expect("a row")
        })
        .collect();
    (run, recorded.traces(), memories)
}

/// **17b cut M2.4's mandatory cross-tempo fixture** (16b §11; doc 16 §3.1
/// D-16-2; doc 17 §6 D-UM-20, D-UM-21): the scripted Rhai mind above on both
/// reds of the (j) field, replayed at 5,000, 1,000 and 500 ms and on the
/// task's own wakes at the same three periods — one event list, one substep
/// hash sequence and one trace sequence. The mind did what it says: each red
/// arrived and held at least twice, the armed red locked the blue, and the
/// memory holds a landing millisecond one substep past a think.
#[test]
fn m2_a_scripted_rhai_mind_is_one_event_list_and_one_hash_sequence_at_three_cadences() {
    let (first, traces, memories) = square_run(Drive::Every(5_000));
    for memory in &memories {
        match memory.get("holds") {
            Some(MemValue::Int(holds)) => assert!(*holds >= 2, "{memory:?}"),
            other => panic!("holds: {other:?}"),
        }
    }
    for (unit, memory) in [RED, RED2].iter().zip(&memories) {
        // `lands_ms()` is the think's landing: its lattice millisecond plus
        // one substep.
        match memory.get("lands") {
            Some(MemValue::Int(lands)) => assert_eq!(
                (lands - SUBSTEP - stagger_ms(SEED, *unit, STOCK_THINK_MS))
                    .rem_euclid(STOCK_THINK_MS),
                0,
                "#{}'s landing {lands}",
                unit.0
            ),
            other => panic!("lands: {other:?}"),
        }
    }
    let words: Vec<(String, Option<u32>)> = first
        .events
        .iter()
        .map(|(_, json, _)| kind_and_unit(json))
        .collect();
    assert!(
        words
            .iter()
            .any(|(word, unit)| word == "locked" && *unit == Some(RED.0)),
        "the armed red locked: {words:?}"
    );
    assert!(
        traces
            .iter()
            .any(|(_, _, _, trace)| trace.iter().any(|line| line.starts_with("arrived"))),
        "an arrival traced"
    );
    assert_eq!(first.counters.mind_faults, 0);
    for drive in [
        Drive::Every(1_000),
        Drive::Every(500),
        Drive::Task(5_000),
        Drive::Task(1_000),
        Drive::Task(500),
    ] {
        let (other, other_traces, other_memories) = square_run(drive);
        assert_eq!(
            other.events, first.events,
            "the event list diverged at {drive:?}"
        );
        assert_eq!(
            other.hashes, first.hashes,
            "the hash sequence diverged at {drive:?}"
        );
        assert_eq!(other_traces, traces, "the trace diverged at {drive:?}");
        assert_eq!(other_memories, memories);
        assert_ne!(other.pulses, first.pulses, "{drive:?} ran the same pulses");
    }
}

// ---------------------------------------------------------------------------
// (8) Nothing of production names the runner
// ---------------------------------------------------------------------------

/// **`default_mind_runner` is untouched** (17b §2's M2.4 "nothing of
/// production"): an arena opened thinks with the native stock mind; one
/// decoded from its snapshot does too — even when a fixture had set a
/// `RhaiRunner` on the arena encoded, since the decode re-installs
/// `default_mind_runner` (17b §5's adversarial F11: a resumed fixture
/// re-applies its runner); the standard sandbox is the process's one.
#[test]
fn m2_the_default_runner_is_still_the_native_stock_mind() {
    let mut arena = the_j_field(Controller::Mind);
    let shown = format!("{:?}", arena.mind_runner);
    assert!(shown.starts_with("NativeStockMind"), "{shown}");
    assert!(!shown.contains("Rhai"));
    let default = tacsim::arena::mind::default_mind_runner(&arena.arena_cfg);
    assert!(format!("{default:?}").starts_with("NativeStockMind"));
    assert!(Arc::ptr_eq(&standard_sandbox(), &standard_sandbox()));

    // A fixture's Rhai runner does not survive a snapshot.
    arena.mind_runner = rhai(&arena, "fn think(v, c) { }");
    assert!(format!("{:?}", arena.mind_runner).starts_with("RhaiRunner"));
    let snapshot = Snapshot::encode(&arena).expect("the arena encodes");
    let decoded = Snapshot::decode(&snapshot.document, (*arena.definition).clone())
        .expect("the document decodes");
    let shown = format!("{:?}", decoded.mind_runner);
    assert!(shown.starts_with("NativeStockMind"), "{shown}");
    assert!(!shown.contains("Rhai"), "{shown}");
}

// ---------------------------------------------------------------------------
// 17b cut M2.5: the port's first think
// ---------------------------------------------------------------------------

/// **A fresh memory writes the fourteen keys as T1 does** (17b cut M2.5;
/// D-UM-30): on the (j) field, cold and then online, each red's FIRST think
/// — from an empty memory — through the native stock mind and through the
/// Rhai Standard Doctrine (`tacsim::minds::STANDARD_RHAI`) on the SAME job:
/// T2 does not fault, its memory bytes are T1's (the prologue writes all
/// fourteen keys of `StockMemory`, so an empty memory's first answer is the
/// default's encoding), the keys are exactly `STOCK_MEMORY_KEY_NAMES`, and
/// its intents are T1's — whole: 17b cut M2.6 ported the fire half, and the
/// `Condition`/`Fire` filter M2.5 held on the armed red is gone.
#[test]
fn m2_a_fresh_memory_writes_the_fourteen_keys_as_t1_does() {
    use tacsim::arena::stock_mind::STOCK_MEMORY_KEY_NAMES;
    let mut keys: Vec<&str> = STOCK_MEMORY_KEY_NAMES.to_vec();
    keys.sort_unstable();
    for online in [false, true] {
        let mut arena = the_j_field(Controller::Mind);
        if online {
            for unit in [RED, RED2] {
                if let Some(state) = arena.unit_mut(unit) {
                    state.power = tacsim::arena::state::Power::Online;
                }
            }
        }
        let t2 = RhaiRunner::new(
            &arena.arena_cfg,
            tacsim::minds::STANDARD_RHAI,
            tacsim::minds::standard_digest(),
        )
        .expect("Standard Doctrine compiles");
        let t1 = NativeStockMind::with(&arena.arena_cfg.reflex);
        for unit in [RED, RED2] {
            let view = Arc::new(arena.clone().mind_view(unit).expect("a view"));
            let runtime = &arena.profiles[&unit];
            let card = runtime.card.clone();
            let job = ThinkJob {
                unit,
                think_seq: 0,
                t_ms: arena.now_ms,
                lands_ms: arena.now_ms + arena.substep_len_ms(),
                seed: arena.seed,
                dropped: false,
                profile: &tacsim::arena::core::ClassTable::STOCK
                    .row(tacsim::arena::core::CoreClass::I),
                view: &view,
                memory: &[],
                map: &arena.definition,
                card: &card,
                program: &runtime.program,
                profile_token: runtime.token(),
            };
            let native = t1.think(&job);
            let rhai = t2.think(&job);
            assert_eq!(
                rhai.fault, None,
                "#{} online {online}: {:?}",
                unit.0, rhai.trace
            );
            assert_eq!(
                String::from_utf8_lossy(&rhai.memory),
                String::from_utf8_lossy(&native.memory),
                "#{} online {online}",
                unit.0
            );
            assert_eq!(
                decode_memory(&rhai.memory)
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<&str>>(),
                keys
            );
            assert_eq!(rhai.intents, native.intents, "#{} online {online}", unit.0);
            if !online {
                assert_eq!(native.intents, vec![IntentKind::Startup]);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 17b cut M2.7: the cross-platform pin (D-UM-28; 17b §1 (vi))
// ---------------------------------------------------------------------------

/// **The float probe** (17b cut M2.7; D-UM-28, doc 17 §23's "Floats across
/// platforms"; 17b §5's feas F12 / adv F6): a scripted mind whose `think`
/// runs a fixed battery of FLOAT expressions — `**` at non-integer exponents
/// (the platform's `powf`, the libm call D-UM-28's risk is about), `/`, `%`,
/// `+`, `-`, `*`, mixed INT and FLOAT — each against its value as this
/// (Windows) host computes it, EXACTLY, and between two bracketing
/// literals; the outcomes folded into an INT bitmask (bit `2k` exact, bit
/// `2k + 1` bracketed, for the [`FLOAT_PROBE_NAMES`]' `k`; then four
/// closing bits), written to `this.probe` and emitted as
/// `helm(bits % 360000)` after a first think's `startup()`. Every operand is
/// scaled by a FLOAT read off the view at think time, so nothing is folded
/// at compile time. No FLOAT reaches memory or an intent (D-UM-28: a type
/// fault).
///
/// **Rhai's FLOAT comparisons are not exact** (rhai 1.26.0 without
/// `unchecked`, `func/builtin.rs`' `impl_float` and `packages/logic.rs`):
/// `x == y` reads `|x - y| / max(|x|, |y|) <= f64::EPSILON`, so a value one
/// unit in the last place away compares EQUAL — the probe would see no libm
/// difference through `==`. So the exact bit tests the difference scaled
/// past the tolerance, `(v - exact) * 1000000.0 == 0.0` (a near-equal
/// subtraction is exact; any nonzero difference scaled by a million is
/// past `EPSILON` against zero), and the closing bits pin the tolerance
/// itself: bit 24 `0.1 + 0.2 == 0.3` TRUE under Rhai's `==`, bit 25
/// `7.5 % 2.2 == 0.9` false (five units apart), bit 26 a mixed INT/FLOAT
/// comparison true, bit 27 `0.1 + 0.2` against `0.3` exactly — false; and
/// the exact test's own proof against `2.0 ** 0.5`'s next float up
/// (`1.4142135623730954`, one unit in the last place): bit 28 Rhai's `==`
/// TRUE, bit 29 the scaled test false.
const FLOAT_PROBE: &str = r#"
fn think(view, card) {
    let one = if view.t_ms >= 0 { 1 } else { 2 };
    let o = one * 1.0;
    let v = [
        (2.0 * o) ** 0.5,
        (10.0 * o) ** -1.5,
        (1.7 * o) ** 2.3,
        (2.0 * o) ** (1.0 / 3.0),
        (0.5 * o) ** 0.1,
        (123.456 * o) ** 0.789,
        (3 * o) / 7.0,
        (7.5 * o) % 2.2,
        (0.1 * o) + 0.2,
        (one + 0.5) * 3,
        (0.001 * o) * 7,
        (9.99 * o) - 9
    ];
    let exact = [
        1.4142135623730951,
        0.03162277660168379,
        3.388695291147646,
        1.2599210498948732,
        0.9330329915368074,
        44.68925627034346,
        0.42857142857142855,
        0.8999999999999995,
        0.30000000000000004,
        4.5,
        0.007,
        0.9900000000000002
    ];
    let low = [1.41421, 0.03162, 3.38869, 1.25992, 0.93303, 44.68925, 0.42857, 0.89999, 0.29999, 4.49999, 0.00699, 0.98999];
    let high = [1.41422, 0.03163, 3.3887, 1.25993, 0.93304, 44.68926, 0.42858, 0.90001, 0.30001, 4.50001, 0.00701, 0.99001];
    let bits = 0;
    let bit = 1;
    for k in 0..12 {
        if (v[k] - exact[k]) * 1000000.0 == 0.0 { bits += bit; }
        bit *= 2;
        if v[k] > low[k] && v[k] < high[k] { bits += bit; }
        bit *= 2;
    }
    if v[8] == 0.3 { bits += bit; }
    bit *= 2;
    if v[7] == 0.9 { bits += bit; }
    bit *= 2;
    if one + 0.5 > 1 { bits += bit; }
    bit *= 2;
    if (v[8] - 0.3) * 1000000.0 == 0.0 { bits += bit; }
    bit *= 2;
    if v[0] == 1.4142135623730954 { bits += bit; }
    bit *= 2;
    if (v[0] - 1.4142135623730954) * 1000000.0 == 0.0 { bits += bit; }
    this.probe = bits;
    if !("started" in this) {
        this.started = true;
        startup();
    }
    helm(bits % 360000);
}
"#;

/// The battery's twelve expressions, bit `2k` (exact) and `2k + 1`
/// (bracketed) of [`FLOAT_PROBE`]'s mask, then the four closing bits — the
/// names a divergence is printed with.
const FLOAT_PROBE_NAMES: [&str; 12] = [
    "2.0 ** 0.5",
    "10.0 ** -1.5",
    "1.7 ** 2.3",
    "2.0 ** (1.0 / 3.0)",
    "0.5 ** 0.1",
    "123.456 ** 0.789",
    "3 / 7.0",
    "7.5 % 2.2",
    "0.1 + 0.2",
    "(1 + 0.5) * 3",
    "0.001 * 7",
    "9.99 - 9",
];

/// **The float probe's per-think bitmask, blessed on this Windows host**:
/// every exact and every bracketed comparison true (bits 0 to 23), Rhai's
/// tolerant `0.1 + 0.2 == 0.3` true (bit 24), `7.5 % 2.2 == 0.9` false (bit
/// 25), the mixed comparison true (bit 26), `0.1 + 0.2` exactly `0.3` false
/// (bit 27), Rhai's `==` against the next float up true (bit 28), the scaled
/// test against it false (bit 29) — `369098751`, the same value on every
/// think of every hull (the battery reads no per-think figure but the scale,
/// which is `1.0`).
const M2_FLOAT_PROBE_BITMASK: i64 = (1 << 25) - 1 + (1 << 26) + (1 << 28);

/// **The cross-platform pins** (17b cut M2.7; D-UM-28), blessed on the
/// Windows host from the first run: per row `(seed, the arena millisecond the
/// match stopped at, the final state_hash, the SHA-256 of every substep hash
/// accumulated over the match as little-endian u64s, the ledger's SHA-256)`.
/// Rows one to three: the stock corpus (`2_350`, `2_351`, `2_352`, two
/// stock tanks a side, the Rhai Standard Doctrine alone on both); row four:
/// the float probe ([`FLOAT_PROBE`], two a side, seed `2_350`).
///
/// **The cut's one declared re-bless** (17b cut M3.6: `mind_sealed` and
/// `sortie_closed` shift the folded ledger sequence (`hash.rs:897`) of every
/// match; the mind row's fold gains D-UM-23's answer, `dropped` and the seal
/// digest; the chassis' core consequence words ride the hit lines and the
/// pilot's sentence; every other byte's encoding is unchanged), blessed on
/// this Windows host, the Linux run CI's. Each row's stop millisecond is
/// unmoved; its state hash, substep digest and ledger digest moved. The
/// tapes, diffed against HEAD `fbc5124a`'s ignoring `seq` and `hash`, differ
/// by exactly four `mind_sealed` after `match_started`, the `sortie_closed`
/// lines (`destroyed` after each `unit_destroyed`, `match_end` before
/// `match_ended`), `destroy_core` after `remove_weapons` on the `hit` and
/// `section_destroyed` of each turret that died — and, on seeds `2_351` and
/// `2_352`, the one `mind_dropped { core }` that turret kill made of a Rhai
/// hull still thinking (mover (d): the mind on the ROM from its next
/// landing; every later line equal). Were, by row: `6_931_363_578_786_187_204`
/// / `b9725b92…` / `a4a3e658…`, `15_956_944_004_013_285_482` / `fade7344…` /
/// `0f7ca61c…`, `13_222_333_907_623_429_933` / `37eb2f9d…` / `c7c50fcc…`,
/// `5_294_377_710_264_198_459` / `1d0d07a9…` / `e6c5f0eb…`.
///
/// **17b cut M4.1's one re-bless of the mind-fixture pins** (the M4 row's
/// finding (1); blessed on the Linux build host, the Windows run CI's): a
/// think is collected at its millisecond and its answer written at the head
/// of the next substep, so the think's substep's `state_hash` folds the
/// pending think's facts and an EMPTY answer where it folded the answer, and
/// a faulted think's `mind_fault` and `mind_dropped` lines carry that
/// substep's hash where they carried a fresh one. Each row's stop
/// millisecond AND its final `state_hash` are unmoved (every answer is
/// written and landed by the end; no byte of the state moved); its substep
/// digest and ledger digest moved. The tapes, diffed against HEAD
/// `5d155e32`'s ignoring `seq` and `hash`, are equal line for line: no line
/// was added, removed or reordered on the stock corpus (no think faults on
/// it). Were, by row: `fc108aa2…` / `7e8b75aa…`, `b1ad9530…` / `bc8c8f22…`,
/// `3f81543e…` / `b4fa5044…`, `0450d808…` / `0b9152d3…`.
///
/// **17b cut M4.3's second declared move of the mind-fixture pins** (the
/// flip; blessed on the Linux build host, the Windows run CI's): Standard
/// Doctrine gained its fifteenth memory key, `ordered` (the release's
/// Withdraw seed and the restoration that reads it), so every memory's bytes
/// — folded by hash item 3c — moved, and `STANDARD_SHA256` with the script;
/// each row's stop millisecond is unmoved; its state hash, substep digest
/// and ledger digest moved, and the probe's bitmask (`369098751`) did not.
/// Were, by row: `4_504_819_013_333_549_817` / `3935703e…` / `f0af53b0…`,
/// `4_318_432_734_286_313_969` / `653a8201…` / `47bba202…`,
/// `8_770_348_364_664_704_415` / `01a0ee5e…` / `3663ed27…`,
/// `2_386_810_340_480_880_612` / `f85eea28…` / `34da2eef…`.
///
/// **M5rules' declared encoding re-bless**: ordered typed rule slots replace
/// the former fire-only fold, and API 2 changes the Standard source and seal
/// identities. Blessed on Windows; all four stopping times and the float probe
/// bitmask remain unchanged. The cross-platform CI lane checks these same pins.
/// M5r: persistent arrival and private formation continuity alter paths.
/// Captured on Windows; the same literal corpus must pass Linux before acceptance.
/// M6: API 4 changes Standard's metadata and sealed source identity. Canonical
/// card/ROM identities, catalog and profile runtime state, own-hit timestamps,
/// and captured pending-think schedules replace the former fixed-card fold.
/// Fresh Windows and Linux measurements agree on all four complete rows below;
/// stopping times and the float probe's 369098751 bitmask are unchanged.
///
/// **Doc 21 cut G.7b's re-bless of the stock corpus** (the atlas's version 3,
/// the showcase relief; captured on Windows, the Linux run CI's): the three
/// stock rows fight on the proving ground, whose terrain digest the state
/// hash folds and whose flanks now rise into hills, so each row's stop
/// millisecond moved with the fight (168,250 / 184,500 / 166,500 → 170,250 /
/// 192,500 / 170,000, the stock pool's record's three) and its state hash,
/// substep digest and ledger digest with it. The float probe's row and its
/// bitmask are unmoved. Were, by row: `4_167_350_372_244_564_158` /
/// `304b5340…` / `1268fc5d…`, `1_964_900_014_152_180_428` / `91884ac2…` /
/// `168c2998…`, `9_027_361_391_885_005_948` / `68516927…` / `5d1fa832…`.
///
/// **17f M7's declared encoding re-bless (mind API 5)**, captured on Windows on
/// the M7 line merged with G.7b's ground and matching the Linux run CI's: M7's
/// `ec886bb4` raised the station API to 5 (`entry_interior_cell`; the retained
/// Registry path keeps API 4 sources as archives), so `MIND_ABI`, the Standard
/// Doctrine's metadata and its sealed source identity moved and every seal the
/// state hash folds moved with them. A bisect over the M7 line found that
/// commit the only mover, and the pins were not moved with it (this test is
/// `#[ignore]`d and the line ran without CI). All four stopping times and the
/// float probe's 369098751 bitmask are unmoved. Were, by row:
/// `12_887_324_004_819_823_052` / `e414b70d…` / `318da4b7…`,
/// `7_649_434_552_267_006_567` / `5f0bdda7…` / `becdcb65…`,
/// `9_329_786_118_378_962_729` / `004bd63f…` / `8de6e161…`,
/// `5_200_400_847_644_332_931` / `6bf446e5…` / `3d5c4a8f…`.
const M2_CROSS_PLATFORM_PINS: [(u64, i64, u64, &str, &str); 4] = [
    (
        2350,
        170250,
        5751437098979270255,
        "0435ea8a5ea54c73685d7fd69ad3925d0691cf49f8256e18ea74e6715b907934",
        "91c474d043c4813fdc28f16d173f6fa02961bde1cbd22eaf513f4874fcba2113",
    ),
    (
        2351,
        192500,
        15239362189155377359,
        "daad5d83c262db2451f0fcfce7f7d72c0e2b719a610c35c5e7cb168c52726b61",
        "929f1c498f2307fe6604c59158e8ddef65005f5444f2b5cb31019c490b03a3d6",
    ),
    (
        2352,
        170000,
        147138284034081668,
        "ea5704215c831ced5ac793598c897adf37dd29177fc0d8ebd2a1fe481484c72c",
        "22d343256c42614362e6ef15bff20fe29e148b05ad7398d77ec67456f0e6e418",
    ),
    (
        2350,
        900000,
        901248092712753737,
        "3935bb86c6d21b77f1ba8168be3d65750615a6c4405d784c72b513caaf252817",
        "de7720f6e56db31c1e67fa82f19939dc556ca198af0f36dd1d3bcba90aa49411",
    ),
];

/// One probe think: `(unit, think_seq, this.probe, the intents)`.
type ProbeThink = (ObjId, u64, Option<i64>, Vec<IntentKind>);

/// The probe's recorder: every think's `(unit, think_seq, this.probe, the
/// intents)`, the Rhai answer flown untouched (an input recorder, 16b §0.4).
#[derive(Debug)]
struct Probed {
    inner: Arc<RhaiRunner>,
    seen: Mutex<Vec<ProbeThink>>,
}

impl MindRunner for Probed {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        let result = self.inner.think(job);
        let probe = match decode_memory(&result.memory).get("probe") {
            Some(MemValue::Int(bits)) => Some(*bits),
            _ => None,
        };
        if let Ok(mut seen) = self.seen.lock() {
            seen.push((job.unit, job.think_seq, probe, result.intents.clone()));
        }
        result
    }
}

/// Lowercase hexadecimal of `bytes`.
fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// One pinned row of a played match.
fn pinned_row(seed: u64, played: &support::Played) -> (u64, i64, u64, String, String) {
    let mut substeps = Sha256::new();
    for hash in &played.hashes {
        substeps.update(hash.to_le_bytes());
    }
    (
        seed,
        played.t_ms,
        played.state_hash,
        hex_of(&substeps.finalize()),
        hex_of(&played.ledger_digest),
    )
}

/// **The twin corpus hashes the same on every host** (17b cut M2.7; doc 17
/// §22's M2 sandbox tests, "the cross-platform hash test"; **D-UM-28**:
/// cross-platform determinism between the Windows blessing host and the
/// Linux build, asserted by a CI test over the twin corpus; doc 17 §23's
/// "Floats across platforms").
///
/// **This is the D-UM-28 gate.** `#[ignore]`d, it is run by name by the
/// named step `Cross-platform mind hashes (17b cut M2.7; D-UM-28)` of
/// `.github/workflows/ci.yml`'s `test` job on BOTH matrix hosts
/// (`ubuntu-latest`, `windows-latest`) before the workspace run — which is
/// how the gated-test inventory (`ci_gated_test_inventory.rs`) finds it
/// covered — and by hand with `-- --ignored --exact --nocapture`.
///
/// (a) **The stock corpus**: seeds 2,350, 2,351 and 2,352 of the shipped
/// proving ground, two stock tanks a side, the Rhai Standard Doctrine alone
/// (`Flier::T2`: `tacsim::minds::standard_runner`) on BOTH sides, one
/// substep a pulse to the end (`support::play`, the substep hashes
/// accumulated after every step); each match's stop millisecond, final
/// `state_hash`, the SHA-256 of its substep hashes and its ledger digest
/// pinned ([`M2_CROSS_PLATFORM_PINS`]). The stock script is FLOAT-FREE by
/// design (D-UM-28: memory and intent arguments are INT; Standard Doctrine
/// computes in integers), so these rows prove the interpreter, the op
/// counts and the integer kernels portable — never a float.
///
/// (b) **The float probe carries the float risk**: [`FLOAT_PROBE`] flown two
/// a side at seed 2,350 to the end, its own row pinned, and every think's
/// bitmask pinned ([`M2_FLOAT_PROBE_BITMASK`]): a host whose `powf` (or any
/// FLOAT built-in) answers another bit fails HERE, by the expression's name,
/// before the row's hashes are compared.
///
/// A Linux divergence fails this test by name on the `ubuntu-latest` host.
/// The pins are blessed on the Windows host; **a re-bless is a declared
/// encoding re-bless with its reason in the 17b ledger row** (a moved
/// script, rhai pin, hash fold or ledger encoding moves rows (a)), never a
/// silent edit — and a float probe that parts between hosts is not re-blessed
/// at all: it is D-UM-28's risk come true, for the lead.
#[test]
#[ignore = "D-UM-28's cross-platform pin: run by name by ci.yml's `Cross-platform mind hashes` step on both hosts"]
fn m2_the_twin_corpus_hashes_the_same_on_every_host() {
    use support::Muster;
    let started = std::time::Instant::now();
    // The pins are against THIS script: the shipped bytes (LF by
    // .gitattributes) and their digest.
    assert_eq!(
        hex_of(&tacsim::minds::standard_digest().0),
        tacsim::minds::STANDARD_SHA256,
        "the Standard Doctrine's bytes"
    );
    let world = support::booted_world();
    let cfg = ArenaConfig::default();
    let reds = [Muster::mind(Team::Red, 1), Muster::mind(Team::Red, 2)];
    let blues = [Muster::mind(Team::Blue, 1), Muster::mind(Team::Blue, 2)];
    let mut rows: Vec<(u64, i64, u64, String, String)> = Vec::new();
    // (a) The stock corpus, T2 against T2.
    for seed in [2_350_u64, 2_351, 2_352] {
        let played = support::play(&world, seed, &reds, &blues, support::rhai_alone(&cfg), None);
        assert!(played.ended.is_some(), "seed {seed} ends");
        rows.push(pinned_row(seed, &played));
    }
    // (b) The float probe.
    let probe = Arc::new(Probed {
        inner: Arc::new(
            RhaiRunner::new(&cfg, FLOAT_PROBE, digest(FLOAT_PROBE))
                .unwrap_or_else(|error| panic!("the float probe compiles: {error}")),
        ),
        seen: Mutex::new(Vec::new()),
    });
    let played = support::play(&world, 2_350, &reds, &blues, probe.clone(), None);
    rows.push(pinned_row(2_350, &played));
    let seen = probe
        .seen
        .lock()
        .map(|seen| seen.clone())
        .unwrap_or_default();
    for (seed, t_ms, state_hash, substeps, ledger) in &rows {
        println!(
            "m2_cross_platform_pin: ({seed}, {t_ms}, {state_hash}, \"{substeps}\", \"{ledger}\"),"
        );
    }
    let thinks = seen.len();
    println!(
        "m2_cross_platform_probe: thinks={thinks} bitmask={:?} pinned={M2_FLOAT_PROBE_BITMASK} \
         wall_ms={} host={}",
        seen.first().and_then(|(_, _, bits, _)| *bits),
        started.elapsed().as_millis(),
        std::env::consts::OS
    );
    // The probe first: a FLOAT that parts names its expression.
    assert!(thinks > 0, "the probe thought");
    for (unit, think_seq, bits, intents) in &seen {
        let Some(bits) = *bits else {
            panic!("#{} think {think_seq} wrote no probe: {intents:?}", unit.0);
        };
        if bits != M2_FLOAT_PROBE_BITMASK {
            let parted: Vec<String> = (0..30_u32)
                .filter(|bit| (bits ^ M2_FLOAT_PROBE_BITMASK) & (1 << bit) != 0)
                .map(
                    |bit| match FLOAT_PROBE_NAMES.get(usize::try_from(bit / 2).unwrap_or(99)) {
                        Some(name) if bit % 2 == 0 => format!("{name} (exact)"),
                        Some(name) => format!("{name} (bracketed)"),
                        None => format!("closing bit {bit}"),
                    },
                )
                .collect();
            panic!(
                "D-UM-28: the float probe parts from the blessing host on {} at #{} think \
                 {think_seq}: {bits} against {M2_FLOAT_PROBE_BITMASK}; parted: {parted:?}",
                std::env::consts::OS,
                unit.0
            );
        }
        assert_eq!(
            intents.last(),
            Some(&IntentKind::Heading {
                mdeg: u32::try_from(bits % 360_000).unwrap_or(0)
            }),
            "#{} think {think_seq} emits the mask as its helm",
            unit.0
        );
    }
    let pinned: Vec<(u64, i64, u64, String, String)> = M2_CROSS_PLATFORM_PINS
        .iter()
        .map(|(seed, t_ms, state_hash, substeps, ledger)| {
            (
                *seed,
                *t_ms,
                *state_hash,
                (*substeps).to_owned(),
                (*ledger).to_owned(),
            )
        })
        .collect();
    for (at, (row, pin)) in rows.iter().zip(&pinned).enumerate() {
        assert_eq!(
            row,
            pin,
            "D-UM-28: row {at} ({}) parts from the blessing host's on {}",
            if at < 3 {
                "the stock corpus"
            } else {
                "the float probe"
            },
            std::env::consts::OS
        );
    }
    assert_eq!(rows.len(), pinned.len());
}

// ---------------------------------------------------------------------------
// 17b cut M3.1 — the row a Rhai think is held to (doc 17 §9, D-UM-35)
// ---------------------------------------------------------------------------

/// **The (j) field with red's core at `class` and every mind rated at
/// `rated`**, red holding blue classified and locked — the hull's own
/// `core_class` cell set by one plan (the cell is the live class
/// `read_core` reads), so its `tactical_unit` core needs no second shape.
/// **Rate a hull's mind at `rated`** (17b cut M4.1): the rating is the
/// seal's (`SealTerms::rated`, read off `UnitState::seal.class` by
/// `Arena::mind_class` against the core's class now), so a rig that rates a
/// hull above the stock rating writes the class the seal would have recorded
/// — `min(rated, core)` — where it moved the arena-wide lever before.
fn rate(arena: &mut Arena, unit: ObjId, rated: tacsim::arena::core::CoreClass) {
    let core = arena
        .core_of(unit)
        .map_or(tacsim::arena::core::CoreClass::Zero, |core| core.class);
    if let Some(state) = arena.unit_mut(unit) {
        state.seal.class = rated.min(core);
    }
}

fn classed_field(
    class: tacsim::arena::core::CoreClass,
    rated: tacsim::arena::core::CoreClass,
) -> Arena {
    use tacsim::arena::state::{Classification, Power, SeenPose, ShadowContact};
    let mut arena = the_j_field(Controller::Mind);
    let addr = mush_tactics::CellAddr::Component {
        component: mush_tactics::ComponentId::new("core").expect("a component"),
        cell: mush_tactics::CellId::new(tacsim::definitions::unit::CORE_CLASS_CELL)
            .expect("a cell"),
    };
    let plan = mush_tactics::UnitMutationPlan::new(
        vec![mush_tactics::UnitMutationOp::SetCell {
            addr,
            value: i64::from(class.ordinal()),
        }],
        None,
        None,
    )
    .expect("a plan");
    if let Some(state) = arena.unit_mut(RED) {
        let (unit, _) = mush_tactics::apply(&state.unit, &plan, 0).expect("the plan applies");
        state.unit = unit;
        state.power = Power::Online;
    }
    rate(&mut arena, RED, rated);
    let plot = arena.unit(BLUE).map(SeenPose::of).expect("blue");
    let contact = tacsim::arena::rng::draw_contact_id(SEED, RED, BLUE);
    arena.contacts.insert(
        RED,
        contact,
        ShadowContact::sighted(
            BLUE,
            Classification::Classified { team: Team::Blue },
            0,
            plot,
        ),
    );
    assert_eq!(
        arena.resolve_lock(RED, contact, 0),
        Ok(()),
        "red locks blue"
    );
    arena
}

/// One think of `source` by red on `arena`, handed red's own row
/// (`Arena::mind_profile`) and `memory`: the answer.
fn think_on_row(arena: &mut Arena, source: &str, memory: &[u8]) -> ThinkResult {
    let profile = arena.mind_profile(RED);
    let view = Arc::new(arena.mind_view(RED).expect("red's view"));
    let runtime = &arena.profiles[&RED];
    let card = runtime.card.clone();
    rhai(arena, source).think(&ThinkJob {
        unit: RED,
        think_seq: 0,
        t_ms: view.t_ms,
        lands_ms: view.t_ms + SUBSTEP,
        seed: arena.seed,
        dropped: false,
        profile: &profile,
        view: &view,
        memory,
        map: &arena.definition,
        card: &card,
        program: &runtime.program,
        profile_token: runtime.token(),
    })
}

/// **`slots()`, the `line_to` cap and the op ceiling are the think's row's**
/// (17b cut M3.1; D-UM-35): rated III on a Class III core, a think reads
/// `slots()` 6, may trace `line_to` six times and faults `limit` at the
/// seventh, and runs a loop Class I's 1,500 operations fault `ops` on; rated
/// I on the same core (`min`), `slots()` reads 2, the third `line_to` is
/// `limit` and the loop is `ops`. A rated-III mind on a II core thinks at II.
/// `fire(w, 3)` cites S3 on the Class III think and is `unknown` on the
/// Class I one.
#[test]
fn m3_slots_line_to_and_the_op_ceiling_are_read_off_the_row() {
    use tacsim::arena::core::{ClassTable, CoreClass};
    let mut three = classed_field(CoreClass::III, CoreClass::III);
    assert_eq!(
        three.mind_profile(RED),
        ClassTable::STOCK.row(CoreClass::III)
    );
    let mut one = classed_field(CoreClass::III, CoreClass::I);
    assert_eq!(one.mind_profile(RED), ClassTable::STOCK.row(CoreClass::I));
    let mut capped = classed_field(CoreClass::II, CoreClass::III);
    assert_eq!(
        capped.mind_class(RED),
        CoreClass::II,
        "the core caps the rating"
    );

    let slots = "fn think(v, c) { this.s = slots(); }";
    for (arena, want) in [(&mut three, 6), (&mut one, 2), (&mut capped, 4)] {
        let result = think_on_row(arena, slots, &[]);
        assert_eq!(result.fault, None);
        assert_eq!(
            mem(&decode_memory(&result.memory), "s"),
            MemValue::Int(want),
            "slots()"
        );
    }

    // `fire`'s slot number is bounded by the same row: S3 is a slot on a
    // Class III think and `unknown` on a Class I one.
    let third = r#"fn think(v, c) { fire("cannon", 3); }"#;
    let cited = think_on_row(&mut three, third, &[]);
    assert_eq!(cited.fault, None, "S3 on Class III");
    assert_eq!(
        cited.intents,
        vec![IntentKind::Fire {
            weapon: SmolStr::new("cannon"),
            slot: Some(3),
        }]
    );
    assert_eq!(
        think_on_row(&mut one, third, &[])
            .fault
            .map(|fault| fault.cause),
        Some("unknown".into()),
        "S3 on Class I"
    );

    let traces = |n: usize| {
        let calls = "let x = line_to(v, v.lock); ".repeat(n);
        format!("fn think(v, c) {{ {calls}this.ok = true; }}")
    };
    assert_eq!(think_on_row(&mut three, &traces(6), &[]).fault, None);
    assert_eq!(
        think_on_row(&mut three, &traces(7), &[])
            .fault
            .map(|fault| fault.cause),
        Some("limit".into()),
        "the seventh line_to on Class III"
    );
    assert_eq!(think_on_row(&mut one, &traces(2), &[]).fault, None);
    assert_eq!(
        think_on_row(&mut one, &traces(3), &[])
            .fault
            .map(|fault| fault.cause),
        Some("limit".into()),
        "the third line_to on Class I"
    );

    let spin = "fn think(v, c) { let x = 0; for i in 0..800 { x += 1; } this.x = x; }";
    let runner = rhai(&three, spin);
    let profile = three.mind_profile(RED);
    let view = Arc::new(three.mind_view(RED).expect("red's view"));
    let runtime = &three.profiles[&RED];
    let card = runtime.card.clone();
    let job = |profile| ThinkJob {
        unit: RED,
        think_seq: 0,
        t_ms: view.t_ms,
        lands_ms: view.t_ms + SUBSTEP,
        seed: three.seed,
        dropped: false,
        profile,
        view: &view,
        memory: &[],
        map: &three.definition,
        card: &card,
        program: &runtime.program,
        profile_token: runtime.token(),
    };
    assert_eq!(runner.think(&job(&profile)).fault, None, "III's 4,500");
    let spent = runner.take_stats();
    let ops = spent.first().map_or(0, |stat| stat.ops + stat.fee);
    assert!(
        ops > STOCK_OPS_PER_THINK && ops <= profile.ops,
        "the loop spends past Class I's ceiling and inside III's: {ops}"
    );
    let class_one = ClassTable::STOCK.row(CoreClass::I);
    assert_eq!(
        runner
            .think(&job(&class_one))
            .fault
            .map(|fault| fault.cause),
        Some("ops".into()),
        "Class I's 1,500"
    );
}

/// **The memory's key cap is the row's, and past a lowered cap no new key is
/// admitted** (17b cut M3.1; D-UM-30, the lead's ruling): at Class I a
/// script writing 33 keys from nothing faults `memory`; a memory of forty
/// keys handed under Class I's 32 passes when the think writes the same
/// forty or drops one, and faults `memory` when it swaps one in — the subset
/// rule, `mush_mind::memory::check_held`, through the runner. And the cap is
/// the row's own: rated III on a Class III core, forty and a hundred and
/// twenty-eight fresh keys pass and a hundred and twenty-nine fault.
#[test]
fn m3_the_memory_cap_is_the_rows_and_admits_no_new_key_past_it() {
    use tacsim::arena::core::CoreClass;
    let mut arena = classed_field(CoreClass::II, CoreClass::I);
    let wide = "fn think(v, c) { for i in 0..33 { this[`k${i}`] = i; } }";
    assert_eq!(
        think_on_row(&mut arena, wide, &[])
            .fault
            .map(|fault| fault.cause),
        Some("memory".into()),
        "33 keys past Class I's 32"
    );
    let forty: Memory = (0..40)
        .map(|i| (format!("k{i:02}"), MemValue::Int(i)))
        .collect();
    let handed = tacsim::arena::mind::encode_memory(&forty);
    let kept = think_on_row(&mut arena, "fn think(v, c) { this.k01 = 7; }", &handed);
    assert_eq!(kept.fault, None, "the forty kept whole");
    assert_eq!(decode_memory(&kept.memory).len(), 40);
    let thinned = think_on_row(
        &mut arena,
        r#"fn think(v, c) { this.remove("k00"); }"#,
        &handed,
    );
    assert_eq!(thinned.fault, None, "one dropped");
    assert_eq!(decode_memory(&thinned.memory).len(), 39);
    let swapped = think_on_row(
        &mut arena,
        r#"fn think(v, c) { this.remove("k00"); this.fresh = 1; }"#,
        &handed,
    );
    assert_eq!(
        swapped.fault.map(|fault| fault.cause),
        Some("memory".into()),
        "a swap past the cap"
    );
    assert_eq!(swapped.memory, handed, "the memory as it was handed");

    // The cap is the ROW's, not Class I's constant: rated III on a Class III
    // core, forty fresh keys from nothing pass and a hundred and twenty-eight
    // fill the row; a hundred and twenty-nine fault `memory`.
    let mut wide_row = classed_field(CoreClass::III, CoreClass::III);
    let fresh =
        |n: usize| format!("fn think(v, c) {{ for i in 0..{n} {{ this[`k${{i}}`] = i; }} }}");
    for n in [40, 128] {
        let result = think_on_row(&mut wide_row, &fresh(n), &[]);
        assert_eq!(result.fault, None, "{n} keys under Class III's 128");
        assert_eq!(decode_memory(&result.memory).len(), n);
    }
    assert_eq!(
        think_on_row(&mut wide_row, &fresh(129), &[])
            .fault
            .map(|fault| fault.cause),
        Some("memory".into()),
        "129 keys past Class III's 128"
    );
}

// ---------------------------------------------------------------------------
// 17b cut M3.2 — D-UM-23 as amended: the answer carried, never recomputed
// ---------------------------------------------------------------------------

/// **The gate fixture's mind**: one hull of the field (`v.unit` 17), whose
/// thinks read a notice its view DRAINED and let it decide both the memory
/// and an intent. Its first think starts the hull; the first think online
/// drives it to the cell (2, 11), three cells from its start, and leaves the
/// helm to the motor; every later
/// think reads the buffer — a `drive_arrived` there writes `this.arrived =
/// t_ms()` and answers `speed(0)`, and from then on a think with no such
/// notice answers `speed(300)` (before the arrival the mind answers nothing,
/// since a mind's `speed` lets go of its own motor's goal).
const GATE_MIND: &str = r#"
fn think(v, c) {
    if v.unit != 17 { return; }
    if !("on" in this) { this.on = t_ms(); startup(); return; }
    if v.power != "online" { return; }
    if !("sent" in this) { this.sent = t_ms(); goto([2, 11]); return; }
    let arrived = false;
    for n in v.notices { if n.kind == "drive_arrived" { arrived = true; } }
    if arrived {
        this.arrived = t_ms();
        speed(0);
    } else if "arrived" in this {
        speed(300);
    }
}
"#;

/// The gate's mind hull: its think falls at `stagger_ms(SEED, 17, 1_000)` =
/// 945 in every second, so the think at 4,945 lies in `(5_000 − 250, 5_000]`.
const GATE_UNIT: ObjId = ObjId(17);
/// The pulse end the snapshot is taken at: a multiple of 5,000, so a pulse
/// end at 5,000, 1,000 and 500 ms alike.
const GATE_END: i64 = 5_000;
/// How far every run of the gate goes.
const GATE_UNTIL: i64 = 15_000;

/// The seated blue's beam: zero flight, one point of damage (so the hull it
/// strikes stays a hull), a two-second cycle.
fn beam() -> WeaponSummary {
    WeaponSummary {
        id: SmolStr::new("laser"),
        max_range_cm: 240_000,
        cycle_ms: 2_000,
        ammunition: SmolStr::new("tacsim.ammunition.laser_charge"),
        burst_rounds: 1,
        muzzle_speed_cms: 0,
        bands: vec![WeaponBand {
            max_range_cm: 240_000,
            dispersion_mrad: 2,
        }],
        count_per_load: 500,
        damage: 1,
        mount_section: SmolStr::new("hull"),
    }
}

/// **The gate's field, on the (j) rig**: the gate's mind hull (17, red) at
/// (2, 8) and the seated blue (9) at (9, 8) with the beam, both with the
/// (j) field's ten-cell sight and presenting a silhouette, the gate's
/// [`GATE_MIND`] on the arena-wide runner.
fn gate_field() -> Arena {
    let seeing = |mut unit: CustodyUnit| {
        unit.sensor = Some(SensorProfile {
            max_range_cells: 10,
            ..sight()
        });
        presenting(unit)
    };
    let mut arena = arena_on(
        vec![
            seeing(tank(
                GATE_UNIT,
                Team::Red,
                centre(2, 8),
                Vec::new(),
                Controller::Mind,
            )),
            seeing(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                vec![beam()],
                Controller::Pilot,
            )),
        ],
        &[BLUE],
        reflex_cfg(),
    );
    arena.mind_runner = rhai(&arena, GATE_MIND);
    arena
}

/// The seated blue's orders at the pulse whose wall is `wall`: its startup at
/// 0, and at [`GATE_END`] a lock on the mind's hull and the beam — phase 4 of
/// the pulse ending at [`GATE_END`] resolves both AT [`GATE_END`] whatever
/// the cadence, after the think at 4,945 and inside its substep.
fn gate_orders(wall: i64) -> Vec<tacsim::arena::intents::Intent> {
    let order = |kind: IntentKind, seq: u64| tacsim::arena::intents::Intent {
        unit: BLUE,
        channel: kind.channel_required(),
        epoch: 1,
        kind,
        seq,
    };
    match wall {
        0 => vec![order(IntentKind::Startup, 1)],
        GATE_END => vec![
            order(
                IntentKind::Lock {
                    contact: tacsim::arena::rng::draw_contact_id(SEED, BLUE, GATE_UNIT),
                },
                2,
            ),
            order(
                IntentKind::Fire {
                    weapon: SmolStr::new("laser"),
                    slot: None,
                },
                3,
            ),
        ],
        _ => Vec::new(),
    }
}

/// Step `arena` on the walls `from`, `from + cadence`, … through `until`,
/// with [`gate_orders`], the tempo pinned: every ledger byte handed out and
/// every substep hash, appended.
fn gate_pulses(
    arena: &mut Arena,
    cadence: i64,
    from: i64,
    until: i64,
    bytes: &mut Vec<u8>,
    hashes: &mut Vec<u64>,
) {
    let mut wall = from;
    while wall <= until {
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let out = arena.step(wall, &[], &gate_orders(wall));
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        hashes.extend(arena.substep_hashes.iter().copied());
        wall += cadence;
    }
}

/// The ledger's open tail flushed into `bytes`, and every line of it.
fn gate_journal(arena: &mut Arena, bytes: &mut Vec<u8>) -> Vec<LedgerEvent> {
    events(arena, bytes)
}

/// A field run to [`GATE_END`] at `cadence`, encoded there and decoded on a
/// fresh ground with the gate's runner re-applied (the decode reinstalls
/// `default_mind_runner`; 17b cut M3.5's seal does it per unit): the bytes
/// and hashes before the encode, the decoded arena and the answer pending at
/// the encode.
fn gate_resumed(
    cadence: i64,
) -> (
    Vec<u8>,
    Vec<u64>,
    Arena,
    Option<tacsim::arena::mind::PendingThink>,
) {
    let mut arena = gate_field();
    let (mut bytes, mut hashes) = (Vec::new(), Vec::new());
    gate_pulses(&mut arena, cadence, 0, GATE_END, &mut bytes, &mut hashes);
    assert_eq!(arena.now_ms, GATE_END, "a pulse end at {cadence} ms");
    let pending = arena
        .minds
        .get(&GATE_UNIT)
        .and_then(|state| state.pending.clone());
    let snapshot = Snapshot::encode(&arena).expect("the arena encodes between pulses");
    let mut decoded = Snapshot::decode(&snapshot.document, definition())
        .expect("the document decodes on a fresh ground");
    assert!(
        format!("{:?}", decoded.mind_runner).starts_with("NativeStockMind"),
        "the decode reinstalls the default runner"
    );
    decoded.mind_runner = rhai(&decoded, GATE_MIND);
    (bytes, hashes, decoded, pending)
}

/// **THE GATE FIXTURE: a resume between a think at N and its landing at
/// N + 1 reproduces the hash sequence, at 5,000, 1,000 and 500 ms** (17b cut
/// M3.2; doc 17 §6 "Snapshot and resume", D-UM-23 as amended 2026-10-04 —
/// the external review's finding 1; the plan's F9 with its geometry
/// corrected).
///
/// **The geometry.** A think at `t` lands at `t + 250` (`Arena::mind_think`
/// schedules the landing one substep on), and the due loop runs every event
/// with `t_ms <=` the substep's end inside that substep
/// (`DueHeap::peek_due`), so a think is still PENDING at a pulse end `end`
/// only when `end − 250 < t <= end`. `end` is [`GATE_END`], 5,000 — a pulse
/// end at all three cadences — and the mind's hull is unit 17, whose think
/// grid is `stagger_ms(2_350, 17, 1_000)` = 945: its think at 4,945 lies in
/// `(4_750, 5_000]` (asserted from `stagger_ms`), lands at 5,195, in the NEXT
/// pulse's first substep, and `pending` is present at the 5,000 ms encode at
/// every cadence (asserted, and its answer is the arrival's: the memory's
/// `arrived` 4,945 and `speed(0)`, because the think at 4,945 drained the
/// motor's `drive_arrived`).
///
/// **The event after the think.** The seated blue's lock and beam, delivered
/// in the pulse that ends at 5,000, are resolved by its phase 4 AT 5,000 —
/// after the think, inside its substep: the `fired` line's millisecond is
/// asserted in `(4_945, 5_000]`.
///
/// **The proof.** Uninterrupted to 15,000 ms, then again with an encode at
/// 5,000, a decode on a fresh ground, the gate's runner re-applied,
/// `resume_at` and on to 15,000: the stitched journal, byte for byte and line
/// for line, and the per-substep hash sequence EQUAL the uninterrupted run's
/// — the carried answer landed at 5,195 at its recorded epochs.
///
/// **The adversarial control, at each cadence.** After the decode the
/// carried answer is thrown away and RECOMPUTED — the answer taken off the
/// row, `Arena::mind_view` and the runner's `think` on a job built from the
/// resumed state (the same seq, the same clocks), the fresh answer written
/// back as `pending` — and landed: the view the think read is gone (its
/// `drive_arrived` drained at 4,945), so the fresh think answers nothing and
/// writes no arrival, and the hash sequence PARTS from the uninterrupted
/// run's at the landing's substep or after (the first parting substep is
/// printed). That is what proves the fixture bites.
#[test]
fn m3_a_resume_between_n_and_n_plus_one_reproduces_the_hash_sequence_at_three_cadences() {
    // The geometry, from the grid.
    let stagger = stagger_ms(SEED, GATE_UNIT, STOCK_THINK_MS);
    assert!(
        (751..=999).contains(&stagger) || stagger == 0,
        "unit 17's think falls in (end − 250, end]: {stagger}"
    );
    let think = GATE_END - STOCK_THINK_MS + stagger;
    assert!(GATE_END - SUBSTEP < think && think <= GATE_END, "{think}");
    let think_seq = u64::try_from((think - stagger) / STOCK_THINK_MS).expect("a seq");
    let lands = think + SUBSTEP;
    // The landing's substep: the first after the encode, ending at 5,250 —
    // the opening pulse at 0 hashes once (its due items at 0), then substep
    // `i` ends at `i × 250`.
    let landing_substep = usize::try_from(GATE_END / SUBSTEP + 1).expect("an index");

    for cadence in [5_000_i64, 1_000, 500] {
        // Uninterrupted.
        let mut whole = gate_field();
        let (mut whole_bytes, mut whole_hashes) = (Vec::new(), Vec::new());
        gate_pulses(
            &mut whole,
            cadence,
            0,
            GATE_UNTIL,
            &mut whole_bytes,
            &mut whole_hashes,
        );
        let whole_events = gate_journal(&mut whole, &mut whole_bytes);
        let fired: Vec<i64> = whole_events
            .iter()
            .filter(|event| {
                kind_and_unit(&serde_json::to_string(&event.kind).unwrap_or_default())
                    == ("fired".to_owned(), Some(BLUE.0))
            })
            .map(|event| event.t_ms)
            .collect();
        assert!(
            fired
                .first()
                .is_some_and(|t_ms| think < *t_ms && *t_ms <= GATE_END),
            "an event after the think inside its substep at {cadence} ms: {fired:?}"
        );
        assert_eq!(
            whole_hashes.len(),
            usize::try_from(GATE_UNTIL / SUBSTEP + 1).expect("a count"),
            "the opening's hash and one a substep"
        );

        // Resumed between the think and its landing.
        let (mut bytes, mut hashes, mut resumed, pending) = gate_resumed(cadence);
        let Some(answer) = pending else {
            panic!("the think at {think} is pending at the {GATE_END} ms encode at {cadence} ms");
        };
        assert_eq!(
            (answer.think_seq, answer.t_ms),
            (think_seq, think),
            "at {cadence} ms"
        );
        let memory = decode_memory(&answer.memory);
        assert_eq!(
            memory.get("arrived"),
            Some(&MemValue::Int(think)),
            "the drained arrival decided the memory at {cadence} ms: {memory:?}"
        );
        assert_eq!(
            answer.intents,
            vec![IntentKind::Speed { cms: 0 }],
            "and the intent"
        );
        assert_eq!(
            resumed
                .minds
                .get(&GATE_UNIT)
                .and_then(|state| state.pending.clone()),
            Some(answer.clone()),
            "the decode carries the answer"
        );
        let control_base = resumed.clone();
        let (mut control_bytes, mut control_hashes) = (bytes.clone(), hashes.clone());
        let _ = resumed.resume_at(GATE_END);
        gate_pulses(
            &mut resumed,
            cadence,
            GATE_END + cadence,
            GATE_UNTIL,
            &mut bytes,
            &mut hashes,
        );
        let resumed_events = gate_journal(&mut resumed, &mut bytes);
        assert_eq!(
            hashes, whole_hashes,
            "the stitched hash sequence at {cadence} ms"
        );
        assert_eq!(
            resumed_events, whole_events,
            "the stitched journal at {cadence} ms"
        );
        assert_eq!(bytes, whole_bytes, "byte for byte at {cadence} ms");
        assert!(
            resumed
                .minds
                .get(&GATE_UNIT)
                .is_some_and(|state| decode_memory(&state.memory).get("arrived")
                    == Some(&MemValue::Int(think))),
            "the carried answer landed at {lands}"
        );

        // The adversarial control: the answer recomputed on the resumed view.
        let mut control = control_base;
        let row = control.minds.get_mut(&GATE_UNIT).expect("the mind's row");
        let carried = row.pending.take().expect("the carried answer");
        let profile = control.mind_profile(GATE_UNIT);
        let view = Arc::new(control.mind_view(GATE_UNIT).expect("the resumed view"));
        let memory = control
            .minds
            .get(&GATE_UNIT)
            .expect("the row")
            .memory
            .clone();
        let runtime = &control.profiles[&GATE_UNIT];
        let card = runtime.card.clone();
        let fresh = control.mind_runner.think(&ThinkJob {
            unit: GATE_UNIT,
            think_seq: carried.think_seq,
            t_ms: carried.t_ms,
            lands_ms: lands,
            seed: control.seed,
            dropped: false,
            profile: &profile,
            view: &view,
            memory: &memory,
            map: &control.definition,
            card: &card,
            program: &runtime.program,
            profile_token: runtime.token(),
        });
        assert_eq!(fresh.fault, None);
        assert!(
            fresh.intents.is_empty() && !decode_memory(&fresh.memory).contains_key("arrived"),
            "the resumed view no longer holds the drained arrival: {:?}",
            fresh.intents
        );
        if let Some(row) = control.minds.get_mut(&GATE_UNIT) {
            row.pending = Some(tacsim::arena::mind::PendingThink {
                intents: fresh.intents,
                memory: fresh.memory,
                rule_sample: fresh.rule_sample,
                ..carried
            });
        }
        let _ = control.resume_at(GATE_END);
        gate_pulses(
            &mut control,
            cadence,
            GATE_END + cadence,
            GATE_UNTIL,
            &mut control_bytes,
            &mut control_hashes,
        );
        assert_ne!(
            control_hashes, whole_hashes,
            "a recomputed answer must not reproduce the run at {cadence} ms"
        );
        let parted = control_hashes
            .iter()
            .zip(&whole_hashes)
            .position(|(a, b)| a != b)
            .expect("the sequences part");
        println!(
            "{cadence} ms: the recomputed answer parts from the run at substep {parted} \
             (ending {} ms); the landing's is {landing_substep} (ending {} ms)",
            i64::try_from(parted).unwrap_or(0) * SUBSTEP,
            i64::try_from(landing_substep).unwrap_or(0) * SUBSTEP
        );
        assert!(
            parted >= landing_substep,
            "parted at {parted}, before the landing's substep {landing_substep}"
        );
    }
}

// ---------------------------------------------------------------------------
// 17b cut M3.4 — the faults, the ROM and the consequences (doc 17 §8, §9;
// D-UM-31, D-UM-32, D-UM-33, D-UM-34)
// ---------------------------------------------------------------------------

/// The second gun the strikes gate's red carries: the cannon under another
/// id, so its Class I core's two slots can both be armed.
fn second_gun() -> WeaponSummary {
    WeaponSummary {
        id: SmolStr::new("gun2"),
        ..cannon()
    }
}

/// **The strikes gate's field**: the (j) field with the armed red carrying a
/// second gun.
fn two_gun_field() -> Arena {
    let seeing = |mut unit: CustodyUnit| {
        unit.sensor = Some(SensorProfile {
            max_range_cells: 10,
            ..sight()
        });
        unit
    };
    arena_on(
        vec![
            seeing(tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon(), second_gun()],
                Controller::Mind,
            )),
            seeing(tank(
                RED2,
                Team::Red,
                centre(6, 10),
                Vec::new(),
                Controller::Mind,
            )),
            presenting(seeing(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            ))),
        ],
        &[BLUE],
        reflex_cfg(),
    )
}

/// One `arm(...)` call that never fires: the lock, odds of 100%, every
/// aspect, no reserve, no range bound.
fn arm_never(weapon: &str) -> String {
    format!("arm(\"{weapon}\", \"lock\", 1000, {ASPECTS_ANY}, 0, 0, 4294967295);")
}

/// **The strikes gate's mind** (M2.4's fault fixtures, three running): red's
/// think 0 arms both guns (S1 the cannon, S2 the second gun) and writes
/// `n`; think 1 locks a contact the view does not hold (`unknown`), think 2
/// helms by a float (`type`), think 3 loops (`ops`, over Class I's 1,500).
fn strikes_mind() -> String {
    strikes_mind_arming("cannon", "gun2", false)
}

/// [`strikes_mind`] arming `first` in S1 and `second` in S2 at think 0 —
/// and, with `boot`, starting the hull there too, so it is online when the
/// ROM takes it.
fn strikes_mind_arming(first: &str, second: &str, boot: bool) -> String {
    let boot = if boot { "startup(); " } else { "" };
    format!(
        "fn think(v, c) {{
    if v.unit != {red} {{ return; }}
    if !(\"n\" in this) {{ this.n = 1; {boot}{first} {second} return; }}
    let t = t_ms();
    if t == {one} {{ lock(999); }}
    if t == {two} {{ helm(1.5); }}
    if t == {three} {{ loop {{ }} }}
}}",
        red = RED.0,
        first = arm_never(first),
        second = arm_never(second),
        one = think_at(RED, 1),
        two = think_at(RED, 2),
        three = think_at(RED, 3),
    )
}

/// A journalled line's JSON with its `mind_fault` cause and `mind_dropped`
/// fault word blanked — what the strikes run and its overflow baseline share.
fn without_words(json: &str) -> String {
    let mut value: Json = serde_json::from_str(json).unwrap_or_default();
    match value.get("kind").and_then(Json::as_str) {
        Some("mind_fault") => value["cause"] = Json::from("-"),
        Some("mind_dropped") => value["fault"] = Json::from("-"),
        _ => {}
    }
    value.to_string()
}

/// **THE GATE FIXTURE: three injected faults drop a unit to the ROM** (17b
/// cut M3.4; doc 17 §8, D-UM-31, D-UM-32; 17b §22's M3 gate). On the (j) rig
/// with red carrying two guns, through `Arena::mind_think`:
///
/// * three `mind_fault` lines at the thinks' milliseconds with the cause
///   words in order (`unknown`, `type`, `ops`) and ONE `mind_dropped {
///   strikes, fault: ops, ops: 1500 }` at the third, under the third's hash;
///   `dropped` is `Some(Strikes)` and the condition row, two slots armed
///   before, is cut to one (the second gun's S2 gone);
/// * the fourth and every later think is answered by the stand-in — equal
///   to `NativeStockMind::think` on the same job at the Class 0 row, no
///   fault, no `ThinkStat`, [`DROPPED_LINE`] the whole trace, the view at
///   most 8 rows with odds on the lock's row only, `slots` 1 — and a clean
///   think never un-drops (every later row still `Some(Strikes)`, the row
///   never wider than one);
/// * the substep hashes equal, pulse by pulse, the seventeen-intent
///   overflow baseline — which journals the same four lines with the word
///   `intents` through the same think arm — and every line of the two tapes
///   is equal but the four lines' words: the fault's cause is the only
///   thing the two paths differ in. (That a think's lines open no action
///   hash and fill no lazy slot — so no LATER line's hash moves — is pinned
///   where it can be seen, under each slot state, by `arena::mind`'s
///   `m3_a_think_s_lines_fill_no_lazy_slot_and_share_a_taken_one`; both
///   tapes here would move together.)
/// * one event list and one hash sequence at 5,000, 1,000 and 500 ms.
#[test]
#[allow(clippy::too_many_lines)]
fn m3_three_injected_faults_drop_a_unit_to_the_rom() {
    use tacsim::arena::core::{ClassTable, CoreClass};
    let source = strikes_mind();
    let last = 7_i64;
    let mut arena = two_gun_field();
    let runner = rhai(&arena, &source);
    let recorded = Recorded::on(runner.clone());
    arena.mind_runner = recorded.clone();
    let mut rig = Rig::new(arena);
    let mut rows = Vec::new();
    let mut widths = Vec::new();
    while rig.arena.now_ms < think_at(RED, last) + SUBSTEP {
        let _ = rig.pulse();
        rows.push(rig.arena.minds.get(&RED).cloned().expect("red's row"));
        widths.push((
            rig.arena.now_ms,
            rig.arena.conditions.slots(RED).len(),
            rig.arena.conditions.slot_of(RED, "gun2"),
        ));
    }
    let journal = rig.journal();
    let lines: Vec<(i64, String, u64)> = journal
        .iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    let minded: Vec<&(i64, String, u64)> = lines
        .iter()
        .filter(|(_, json, _)| {
            let (kind, _) = kind_and_unit(json);
            kind == "mind_fault" || kind == "mind_dropped"
        })
        .collect();
    let as_json = |json: &str| serde_json::from_str::<Json>(json).unwrap_or_default();
    assert_eq!(
        minded
            .iter()
            .map(|(t_ms, json, _)| (*t_ms, as_json(json)))
            .collect::<Vec<_>>(),
        vec![
            (
                think_at(RED, 1),
                serde_json::json!({"kind": "mind_fault", "unit": RED.0, "think_seq": 1, "cause": "unknown"})
            ),
            (
                think_at(RED, 2),
                serde_json::json!({"kind": "mind_fault", "unit": RED.0, "think_seq": 2, "cause": "type"})
            ),
            (
                think_at(RED, 3),
                serde_json::json!({"kind": "mind_fault", "unit": RED.0, "think_seq": 3, "cause": "ops"})
            ),
            (
                think_at(RED, 3),
                serde_json::json!({"kind": "mind_dropped", "unit": RED.0, "cause": "strikes", "fault": "ops", "ops": 1500})
            ),
        ],
        "the four lines"
    );
    assert_eq!(
        minded.get(2).map(|line| line.2),
        minded.get(3).map(|line| line.2),
        "the drop rides the third fault's hash"
    );
    // The row: two slots armed before the drop, one after; dropped for good.
    let drop_pulse = pulse_of_think(3);
    let before = widths
        .get(drop_pulse.saturating_sub(1))
        .copied()
        .expect("the pulse before the drop");
    assert_eq!(
        (before.1, before.2),
        (2, Some(2)),
        "two slots armed before the drop"
    );
    for (now_ms, width, gun2) in widths.iter().skip(drop_pulse) {
        assert!(*width <= 1, "the ROM's one slot at {now_ms}: {width}");
        assert_eq!(*gun2, None, "the second gun's slot cut at {now_ms}");
    }
    for (at, row) in rows.iter().enumerate().skip(drop_pulse) {
        assert_eq!(
            row.dropped,
            Some(tacsim::arena::mind::DropCause::Strikes),
            "a clean think never un-drops (pulse {at})"
        );
    }
    // The stand-in, from think 4.
    let rom = ClassTable::STOCK.row(CoreClass::Zero);
    for n in 0..=3_u64 {
        assert!(!recorded.think(RED, n).dropped, "think {n} is the script's");
    }
    for n in 4..=u64::try_from(last).unwrap_or(0) {
        let think = recorded.think(RED, n);
        assert!(think.dropped, "think {n} is the ROM's");
        assert_eq!(think.profile, rom, "think {n} at Class 0's row");
        assert_eq!(
            (
                think.profile.rows,
                think.profile.odds_rows,
                think.profile.slots
            ),
            (8, 0, 1)
        );
        assert!(think.view.contacts.len() <= 8, "think {n}: eight rows");
        assert!(
            think
                .view
                .contacts
                .iter()
                .all(|row| row.odds.is_empty() || Some(row.id) == think.view.lock),
            "think {n}: the lock's odds only"
        );
        assert_eq!(think.result.fault, None, "the ROM cannot fault");
        assert_eq!(think.result.trace, vec![DROPPED_LINE.to_owned()]);
        let mut native = NativeStockMind::with(&rig.arena.arena_cfg.reflex).think(&think.job());
        native.trace = vec![DROPPED_LINE.to_owned()];
        assert_eq!(think.result, native, "the stand-in's think {n}");
    }
    assert!(
        runner
            .take_stats()
            .iter()
            .filter(|stat| stat.unit == RED)
            .all(|stat| stat.think_seq <= 3),
        "a dropped think records no ThinkStat"
    );

    // The baseline: M2.2's overflow path, answering the faulted thinks with
    // seventeen intents and every other think as the Rhai run answered it.
    let kept = recorded.kept();
    let speed = IntentKind::Speed { cms: 300 };
    let baseline = replay(&kept, |kept| ThinkResult {
        diagnostics: None,
        rule_sample: None,
        intents: vec![speed.clone(); MIND_INTENTS_MAX + 1],
        memory: kept.memory.clone(),
        fault: None,
        trace: Vec::new(),
    });
    let mut arena = two_gun_field();
    arena.mind_runner = baseline;
    let mut flow = Rig::new(arena);
    for _ in 0..rows.len() {
        let _ = flow.pulse();
    }
    assert_eq!(flow.hashes.len(), rig.hashes.len());
    for (pulse, (rhai, overflow)) in rig.hashes.iter().zip(&flow.hashes).enumerate() {
        assert_eq!(rhai, overflow, "the substep hashes of pulse {pulse}");
    }
    let flow_lines: Vec<(i64, String, u64)> = flow
        .journal()
        .iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    assert_eq!(
        flow_lines
            .iter()
            .filter(|(_, json, _)| kind_and_unit(json).0 == "mind_fault")
            .map(|(_, json, _)| as_json(json)["cause"].clone())
            .collect::<Vec<_>>(),
        vec![Json::from("intents"); 3],
        "the overflow's word"
    );
    let blank = |lines: &[(i64, String, u64)]| {
        lines
            .iter()
            .map(|(t_ms, json, hash)| (*t_ms, without_words(json), *hash))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        blank(&lines),
        blank(&flow_lines),
        "every other line, and every hash, as the baseline's"
    );

    // One event list and one hash sequence at three cadences.
    let tempo = |cadence_ms: i64| {
        let mut arena = two_gun_field();
        arena.mind_runner = rhai(&arena, &source);
        run(&mut arena, Drive::Every(cadence_ms), 10_000)
    };
    let slow = tempo(5_000);
    assert_eq!(
        slow.events
            .iter()
            .filter(|(_, json, _)| kind_and_unit(json).0 == "mind_dropped")
            .count(),
        1
    );
    for cadence in [1_000, 500] {
        let other = tempo(cadence);
        assert_eq!(other.events, slow.events, "the event list at {cadence} ms");
        assert_eq!(
            other.hashes, slow.hashes,
            "the hash sequence at {cadence} ms"
        );
        assert_ne!(other.pulses, slow.pulses);
    }
}

/// **The ROM arms its own gun in the slot the failed mind left on another**
/// (17b cut M3.4; D-UM-32: the ROM at the fallback preset). The strikes
/// gate's run with the arming order reversed — the failed mind's think 0
/// arms the SECOND gun in S1 and the cannon (the hull's first weapon) in
/// S2, and starts the hull, so the ROM's first think (4) is engaged and
/// arms at once rather than disarming an offline hull. M6 replaces the failed
/// program with the captured ROM, clearing all old armings at the drop:
///
/// * before the drop S1 is the second gun and S2 the cannon; at the drop
///   the row is empty, with no failed-mind rule left active;
/// * a ROM think arms the cannon (the hull's first weapon, `arming_for`),
///   and from its landing on S1 is the cannon's on the ROM's own terms
///   (the card's odds, not the failed mind's 100%) and the second gun holds
///   no slot — where the arming alone would be refused `NoSlot` at every
///   ROM think, the second gun firing on terms the ROM never set;
/// * no ROM think's view carries a `NoSlot` refusal;
/// * one event list and one hash sequence at 5,000, 1,000 and 500 ms.
#[test]
#[allow(clippy::too_many_lines)]
fn m3_the_rom_arms_its_first_gun_where_the_failed_mind_left_another() {
    let source = strikes_mind_arming("gun2", "cannon", true);
    let last = 7_i64;
    let mut arena = two_gun_field();
    let recorded = Recorded::on(rhai(&arena, &source));
    arena.mind_runner = recorded.clone();
    let mut rig = Rig::new(arena);
    let mut rows: Vec<(i64, Vec<Option<FireCondition>>)> = Vec::new();
    while rig.arena.now_ms < think_at(RED, last) + SUBSTEP {
        let _ = rig.pulse();
        rows.push((
            rig.arena.now_ms,
            rig.arena
                .conditions
                .slots(RED)
                .iter()
                .map(|rule| {
                    rule.as_ref()
                        .and_then(tacsim::arena::conditions::Rule::fire)
                        .cloned()
                })
                .collect(),
        ));
    }
    assert_eq!(
        rig.arena.minds.get(&RED).and_then(|state| state.dropped),
        Some(tacsim::arena::mind::DropCause::Strikes)
    );
    let weapons = |row: &[Option<FireCondition>]| -> Vec<Option<String>> {
        row.iter()
            .map(|slot| slot.as_ref().map(|cond| cond.weapon.to_string()))
            .collect()
    };
    let drop_pulse = pulse_of_think(3);
    let before = rows
        .get(drop_pulse.saturating_sub(1))
        .map(|(_, row)| weapons(row))
        .expect("the pulse before the drop");
    assert_eq!(
        before,
        vec![Some("gun2".to_owned()), Some("cannon".to_owned())],
        "the failed mind's two slots"
    );
    let at_drop = rows
        .get(drop_pulse)
        .map(|(_, row)| weapons(row))
        .expect("the drop's pulse");
    assert!(
        at_drop.is_empty(),
        "ROM replacement clears every failed-mind arming"
    );
    // The ROM's arming of the cannon, and its landing.
    let arms_cannon = |kept: &Kept| {
        kept.result.intents.iter().any(|intent| {
            matches!(
                intent,
                IntentKind::Condition(Arming::Arm(cond)) if cond.weapon == "cannon"
            )
        })
    };
    let first_rom_arming = (4..=u64::try_from(last).unwrap_or(0))
        .map(|n| recorded.think(RED, n))
        .find(|kept| kept.dropped && arms_cannon(kept))
        .expect("a ROM think arms the hull's first gun");
    let landed = rows
        .iter()
        .filter(|(now_ms, _)| *now_ms > first_rom_arming.lands_ms)
        .collect::<Vec<_>>();
    assert!(!landed.is_empty(), "the run reaches past the landing");
    for (now_ms, row) in landed {
        assert_eq!(
            weapons(row),
            vec![Some("cannon".to_owned())],
            "S1 the ROM's cannon at {now_ms}"
        );
        assert!(
            row.iter().flatten().all(|cond| cond.min_permille < 1_000),
            "on the ROM's terms at {now_ms}"
        );
    }
    for n in 4..=u64::try_from(last).unwrap_or(0) {
        let kept = recorded.think(RED, n);
        assert!(
            !kept.view.notices.iter().any(|notice| matches!(
                notice,
                FrameNotice::Refused {
                    refusal: Refusal::NoSlot,
                    ..
                }
            )),
            "think {n} read a NoSlot refusal"
        );
    }

    // One event list and one hash sequence at three cadences.
    let tempo = |cadence_ms: i64| {
        let mut arena = two_gun_field();
        arena.mind_runner = rhai(&arena, &source);
        run(&mut arena, Drive::Every(cadence_ms), 10_000)
    };
    let slow = tempo(5_000);
    assert_eq!(
        slow.events
            .iter()
            .filter(|(_, json, _)| kind_and_unit(json).0 == "mind_dropped")
            .count(),
        1
    );
    for cadence in [1_000, 500] {
        let other = tempo(cadence);
        assert_eq!(other.events, slow.events, "the event list at {cadence} ms");
        assert_eq!(
            other.hashes, slow.hashes,
            "the hash sequence at {cadence} ms"
        );
    }
}

// --- the core's two consequences, on doctored hulls ------------------------

/// A canonical id.
fn canonical(id: &str) -> mush_core::CanonicalId {
    mush_core::CanonicalId::new(id).expect("a canonical id")
}

/// **The fielded catalog with `chassis`' turret list doctored to `turret`**
/// (17b cut M3.4): the shipped content's authored unit, its turret section's
/// consequences replaced — the words no shipped chassis carries before 17b
/// cut M3.6 — so `unit_profile` decodes them through `consequence_of`.
fn doctored_catalog(
    chassis: &str,
    turret: Vec<tacsim::definitions::unit::Consequence>,
) -> tacsim::definitions::Catalog {
    let mut catalog = tacsim::definitions::Catalog::build_fielded(&[], tacsim::NS)
        .expect("the fielded content binds");
    let spec = catalog
        .specs
        .get_mut(&canonical(chassis))
        .expect("the chassis' entry");
    let section = spec
        .sections
        .iter_mut()
        .find(|section| section.id.as_str() == "turret")
        .expect("a turret");
    section.consequences = turret;
    catalog
}

/// **A doctored hull** (`tests/match_lifecycle.rs`' shape): the chassis'
/// binding instantiated as `id` and checked out through
/// `tacsim::match_::custody_unit` on the doctored catalog, at (2, 8) — then
/// every band's table sent to the turret alone (so the shooter's one round
/// finds it), every gun the turret carried remounted on the hull, and
/// `extra` hull guns added (each a copy of the cannon under its own id),
/// sealed again as it now stands.
fn doctored_hull(
    catalog: &tacsim::definitions::Catalog,
    chassis: &str,
    id: ObjId,
    controller: Controller,
    keep_turret_guns: &[&str],
    extra: &[&str],
) -> CustodyUnit {
    let key = canonical(chassis);
    let spec = catalog.specs.get(&key).expect("the chassis' spec");
    let binding = catalog.bindings.get(&key).expect("the chassis' binding");
    let cells = tacsim::definitions::to_tactical_unit(spec, binding, id).expect("a unit");
    let mut unit = tacsim::match_::custody_unit(
        catalog,
        cells,
        id,
        Team::Red,
        centre(2, 8),
        0,
        controller,
        0,
        &tacsim::match_::SealTerms::fixture(),
    )
    .expect("the fixture terms seal");
    for band in &mut unit.profile.bands {
        band.sections = vec![(SmolStr::new("turret"), 1)];
    }
    let template = unit.weapons.first().cloned().expect("a gun");
    for weapon in &mut unit.weapons {
        if !keep_turret_guns.contains(&weapon.id.as_str()) {
            weapon.mount_section = SmolStr::new("hull");
        }
    }
    for gun in extra {
        unit.weapons.push(WeaponSummary {
            id: SmolStr::new(*gun),
            mount_section: SmolStr::new("hull"),
            ..template.clone()
        });
    }
    unit.reseal(&tacsim::match_::SealTerms::fixture())
        .expect("the fixture terms seal");
    unit
}

/// The seated blue's beam, heavy enough to take a turret through its plate
/// and internals in one round.
fn heavy_beam() -> WeaponSummary {
    WeaponSummary {
        damage: 1_000,
        ..beam()
    }
}

/// **The consequence field**: the doctored `victim` and the seated blue at
/// (9, 8) with the heavy beam and the (j) field's ten-cell sight; `seated`
/// pilots the victim too.
fn consequence_field(victim: CustodyUnit, seated: bool) -> Arena {
    let id = victim.id;
    let blue = CustodyUnit {
        sensor: Some(SensorProfile {
            max_range_cells: 10,
            ..sight()
        }),
        ..tank(
            BLUE,
            Team::Blue,
            centre(9, 8),
            vec![heavy_beam()],
            Controller::Pilot,
        )
    }
    .sealed();
    let pilots: Vec<ObjId> = if seated { vec![id, BLUE] } else { vec![BLUE] };
    arena_on(vec![victim, blue], &pilots, reflex_cfg())
}

/// One order at epoch 1.
fn order(unit: ObjId, kind: IntentKind, seq: u64) -> tacsim::arena::intents::Intent {
    tacsim::arena::intents::Intent {
        unit,
        channel: kind.channel_required(),
        epoch: 1,
        kind,
        seq,
    }
}

/// The seated blue's orders against `victim`: its startup at 0, and at
/// [`GATE_END`] a lock and the beam, resolved AT 5,000 at every cadence (in
/// [`beats`]' pulse of no wall time) — the beam's impact a due event at
/// 5,000, applied in the substep after.
fn shooter_orders(victim: ObjId, wall: i64) -> Vec<tacsim::arena::intents::Intent> {
    match wall {
        0 => vec![order(BLUE, IntentKind::Startup, 1)],
        GATE_END => vec![
            order(
                BLUE,
                IntentKind::Lock {
                    contact: tacsim::arena::rng::draw_contact_id(SEED, BLUE, victim),
                },
                2,
            ),
            order(
                BLUE,
                IntentKind::Fire {
                    weapon: SmolStr::new("laser"),
                    slot: None,
                },
                3,
            ),
        ],
        _ => Vec::new(),
    }
}

/// What one beat run produced: every line as `(t_ms, json, hash)` and every
/// substep hash.
type BeatRun = (Vec<(i64, String, u64)>, Vec<u64>);

/// Step `arena` on the walls `0, cadence, …` through `until`, the tempo
/// pinned; `after` sees the arena after every pulse.
///
/// **Each wall's `orders` go in a pulse of no wall time of their own**, after
/// the pulse that brought the arena to that wall — `tests/weapons.rs`'
/// `replay`'s rule, and for its reason: an order handed to an advancing
/// pulse is queued at that pulse's head, so it sits in the state the
/// pulse's earlier substeps hash, and a 5,000 ms pulse would hash it where
/// a 500 ms one does not. Delivered at `dt = 0`, phase 4 resolves it at
/// exactly the wall's millisecond at every cadence and every substep hash
/// before it is the cadence's own; what it schedules (a beam's impact at
/// that millisecond) is a due event, applied in the next substep.
fn beats(
    arena: &mut Arena,
    cadence: i64,
    until: i64,
    orders: &dyn Fn(i64) -> Vec<tacsim::arena::intents::Intent>,
    after: &mut dyn FnMut(&Arena, &StepOutput),
) -> BeatRun {
    let (mut bytes, mut hashes) = (Vec::new(), Vec::new());
    let mut wall = 0;
    while wall <= until {
        for (at, due) in [Vec::new(), orders(wall)].into_iter().enumerate() {
            if at == 1 && due.is_empty() {
                continue;
            }
            arena.clock.pin_tempo_for_tests(Tempo::Quiet);
            let out = arena.step(wall, &[], &due);
            for chunk in &out.ledger_chunks {
                bytes.extend_from_slice(&chunk.bytes);
            }
            hashes.extend(arena.substep_hashes.iter().copied());
            after(arena, &out);
        }
        wall += cadence;
    }
    let lines = events(arena, &mut bytes)
        .into_iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    (lines, hashes)
}

/// The lines of one kind, as `(t_ms, json, hash)`.
fn of_kind<'a>(lines: &'a [(i64, String, u64)], kind: &str) -> Vec<&'a (i64, String, u64)> {
    lines
        .iter()
        .filter(|(_, json, _)| kind_and_unit(json).0 == kind)
        .collect()
}

/// **The consequence fixtures' mind**, for `unit`: its first think starts
/// the hull; its first think online drives it toward (2, 14) and arms the
/// coax (S1), the cannon (S2) and the bow gun (S3) on terms that never fire;
/// its first think after 4,000 locks the first contact it holds — so for a
/// hull whose think falls in `(4_750, 5_000)` that lock is the think IN
/// FLIGHT when the turret dies at 5,000.
fn core_mind(unit: ObjId) -> String {
    format!(
        "fn think(v, c) {{
    if v.unit != {unit} {{ return; }}
    if !(\"on\" in this) {{ this.on = t_ms(); startup(); return; }}
    if v.power != \"online\" {{ return; }}
    if !(\"sent\" in this) {{ this.sent = t_ms(); goto([2, 14]); {coax} {cannon} {bow} return; }}
    if t_ms() > 4000 && !(\"locked\" in this) {{
        for k in v.contacts {{ if !(\"locked\" in this) {{ this.locked = t_ms(); lock(k.id); }} }}
    }}
}}",
        unit = unit.0,
        coax = arm_never("machine_gun"),
        cannon = arm_never("cannon"),
        bow = arm_never("bow"),
    )
}

/// **A doctored line tank** for `id`: the turret list `turret`, the cannon
/// kept in the turret, the coax remounted on the hull and a `bow` gun added
/// on it.
fn doctored_line_tank(
    id: ObjId,
    controller: Controller,
    turret: Vec<tacsim::definitions::unit::Consequence>,
) -> CustodyUnit {
    let catalog = doctored_catalog("tacsim.unit.line_tank", turret);
    doctored_hull(
        &catalog,
        "tacsim.unit.line_tank",
        id,
        controller,
        &["cannon"],
        &["bow"],
    )
}

/// The authored `[remove_weapons, destroy_core]`.
fn destroy_words() -> Vec<tacsim::definitions::unit::Consequence> {
    use tacsim::definitions::unit::Consequence as Authored;
    vec![
        Authored::RemoveWeapons {
            section: canonical("turret"),
        },
        Authored::DestroyCore,
    ]
}

/// A consequence field with `victim` flown by [`core_mind`], recorded.
fn core_mind_field(victim: CustodyUnit) -> (Arena, Arc<Recorded>) {
    let id = victim.id;
    let mut arena = consequence_field(victim, false);
    let recorded = Recorded::on(rhai(&arena, &core_mind(id)));
    arena.mind_runner = recorded.clone();
    (arena, recorded)
}

/// One run of a [`core_mind_field`] at `cadence` to `until`.
fn core_run(victim: CustodyUnit, cadence: i64, until: i64) -> BeatRun {
    let id = victim.id;
    let (mut arena, _) = core_mind_field(victim);
    beats(
        &mut arena,
        cadence,
        until,
        &|wall| shooter_orders(id, wall),
        &mut |_, _| {},
    )
}

/// **THE GATE FIXTURE: `destroy_core` drops one at its next landing, at
/// three cadences** (17b cut M3.4; doc 17 §8, §9; D-UM-9, D-UM-32,
/// D-UM-34). A scripted mind ([`core_mind`]) on a DOCTORED line tank whose
/// turret list carries `[remove_weapons, destroy_core]` ([`doctored_hull`]:
/// every band strikes the turret; the coax and a bow gun on the hull), the
/// seated blue's heavy beam killing the turret at 5,000:
///
/// 1. **A think in flight.** The hull is unit 17, its think at 4,945
///    locking blue — in flight when the round lands at 5,000 — and landing
///    at 5,195:
///    * the `hit` at 5,000 carries `remove_weapons` and `destroy_core`, and
///      after it the hull holds no core;
///    * the think in flight LANDS as thought: its `locked` line at 5,195;
///    * ONE `mind_dropped { core }` on the whole tape, at 5,195, under the
///      landing's own hash (the `locked` line's), with no fault and no
///      budget;
///    * the row, S1 the coax, S2 the cannon and S3 the bow before, is cleared:
///      M6 replaces the failed program with the captured ROM; its first fresh
///      arming then installs the surviving coax in Class 0's one slot;
///    * the next think (5,945) is the ROM's — dropped, at Class 0's row,
///      [`DROPPED_LINE`] its trace — and the think at 4,945 was not;
///    * the hull's `DRIVE` goal and desired speed survive the drop: equal,
///      at 5,250, to a control whose turret carries `remove_weapons` alone
///      (which never drops);
///    * one event list and one hash sequence at 5,000, 1,000 and 500 ms.
/// 2. **No think in flight.** A hull whose think falls in `(4_100, 4_700)`
///    of its second (its lock landed before the round): the drop is at its
///    NEXT think's millisecond, that think already the ROM's; one line; one
///    event list and one hash sequence at three cadences.
/// 3. **A pilot's hull.** The doctored line tank seated, its turret killed:
///    the `hit` carries `destroy_core` and the core is gone, but no mind row
///    and no `mind_dropped` line, ever; and at 10,000 the pilot's verbs are
///    answered — a speed taken, the cannon's `FIRE` refused by name, an
///    arming of the coax laid in S1 (Class 0's one slot).
#[test]
#[allow(clippy::too_many_lines)]
fn m3_destroy_core_drops_one_at_its_next_landing_at_three_cadences() {
    use tacsim::arena::core::{ClassTable, CoreClass};
    use tacsim::definitions::unit::Consequence as Authored;
    // (1) A think in flight.
    let victim = GATE_UNIT;
    let stagger = stagger_ms(SEED, victim, STOCK_THINK_MS);
    let think = GATE_END - STOCK_THINK_MS + stagger;
    assert!(
        GATE_END - SUBSTEP < think && think < GATE_END,
        "unit 17's think is in flight at 5,000: {think}"
    );
    let lands = think + SUBSTEP;
    let (mut arena, recorded) = core_mind_field(doctored_line_tank(
        victim,
        Controller::Mind,
        destroy_words(),
    ));
    assert_eq!(
        arena.unit(victim).map(|state| state.seal.core_class),
        Some(CoreClass::II),
        "the line core, sealed"
    );
    // The hull's row, its motor goal and its desired speed.
    type Seen = (Vec<Option<String>>, Option<(u16, u16)>, i32);
    let mut before: Option<Seen> = None;
    let mut after: Option<Seen> = None;
    let mut snap = |arena: &Arena, _: &StepOutput| {
        let seen = (
            arena
                .conditions
                .slots(victim)
                .iter()
                .map(|slot| {
                    slot.as_ref()
                        .and_then(tacsim::arena::conditions::Rule::fire)
                        .map(|cond| cond.weapon.to_string())
                })
                .collect(),
            arena.motors.row(victim).and_then(|row| row.goal),
            arena
                .unit(victim)
                .map_or(0, |state| state.desired_speed_cms),
        );
        // The round is resolved by the phase 4 of the pulse ENDING at 5,000,
        // so the last picture before it is the pulse ending at 4,750.
        if arena.now_ms == GATE_END - SUBSTEP {
            before = Some(seen);
        } else if arena.now_ms == GATE_END + SUBSTEP {
            after = Some(seen);
        }
    };
    let (lines, _) = beats(
        &mut arena,
        SUBSTEP,
        7_000,
        &|wall| shooter_orders(victim, wall),
        &mut snap,
    );
    let hits = of_kind(&lines, "hit");
    assert_eq!(hits.len(), 1, "one round: {hits:?}");
    let hit: Json = serde_json::from_str(&hits[0].1).unwrap_or_default();
    assert_eq!(hits[0].0, GATE_END);
    assert_eq!(
        hit["consequences"],
        serde_json::json!(["remove_weapons", "destroy_core"]),
        "the hit carries the word: {hit}"
    );
    assert_eq!(arena.core_of(victim), None, "the capability is gone");
    let locked: Vec<_> = of_kind(&lines, "locked")
        .into_iter()
        .filter(|(_, json, _)| kind_and_unit(json).1 == Some(victim.0))
        .collect();
    assert_eq!(
        locked.iter().map(|line| line.0).collect::<Vec<_>>(),
        vec![lands],
        "the think in flight landed as thought"
    );
    let dropped = of_kind(&lines, "mind_dropped");
    assert_eq!(dropped.len(), 1, "one drop on the whole tape: {dropped:?}");
    assert_eq!(dropped[0].0, lands, "at the landing's millisecond");
    assert_eq!(
        serde_json::from_str::<Json>(&dropped[0].1).unwrap_or_default(),
        serde_json::json!({"kind": "mind_dropped", "unit": victim.0, "cause": "core"})
    );
    assert_eq!(dropped[0].2, locked[0].2, "under the landing's hash");
    assert_eq!(
        arena.minds.get(&victim).and_then(|state| state.dropped),
        Some(tacsim::arena::mind::DropCause::Core)
    );
    let (slots_before, goal_before, _) = before.clone().expect("the pulse ending at 4,750");
    assert_eq!(
        slots_before,
        vec![
            Some("machine_gun".to_owned()),
            Some("cannon".to_owned()),
            Some("bow".to_owned()),
            None
        ],
        "three slots armed before the kill, in a row the line core's four wide"
    );
    assert_eq!(goal_before, Some((2, 14)), "the hull is driving");
    let (slots_after, goal_after, speed_after) = after.clone().expect("the pulse ending at 5,250");
    assert!(
        slots_after.is_empty(),
        "ROM replacement clears the failed program"
    );
    let first_rom = recorded.think(victim, 5);
    assert_eq!(first_rom.t_ms, think + STOCK_THINK_MS);
    assert!(first_rom.dropped, "the next think is the ROM's");
    assert_eq!(first_rom.profile, ClassTable::STOCK.row(CoreClass::Zero));
    assert_eq!(first_rom.result.trace, vec![DROPPED_LINE.to_owned()]);
    assert!(
        first_rom.result.intents.iter().any(|intent| matches!(
            intent,
            IntentKind::Condition(Arming::Arm(condition)) if condition.weapon == "machine_gun"
        )),
        "the fresh ROM arms the first surviving gun"
    );
    assert_eq!(
        arena
            .conditions
            .slots(victim)
            .iter()
            .filter_map(|slot| {
                slot.as_ref()
                    .and_then(tacsim::arena::conditions::Rule::fire)
                    .map(|condition| condition.weapon.as_str())
            })
            .collect::<Vec<_>>(),
        vec!["machine_gun"],
        "the fresh ROM arming lands in its one slot"
    );
    let in_flight = recorded.think(victim, 4);
    assert_eq!(in_flight.t_ms, think);
    assert!(!in_flight.dropped, "the think in flight was the script's");
    // The control: the same field, the turret carrying `remove_weapons`
    // alone — no drop — has the same goal and desired speed at 5,250.
    let (mut control, _) = core_mind_field(doctored_line_tank(
        victim,
        Controller::Mind,
        vec![Authored::RemoveWeapons {
            section: canonical("turret"),
        }],
    ));
    let mut held: Option<(Option<(u16, u16)>, i32)> = None;
    let (control_lines, _) = beats(
        &mut control,
        SUBSTEP,
        GATE_END + SUBSTEP,
        &|wall| shooter_orders(victim, wall),
        &mut |arena: &Arena, _: &StepOutput| {
            if arena.now_ms == GATE_END + SUBSTEP {
                held = Some((
                    arena.motors.row(victim).and_then(|row| row.goal),
                    arena
                        .unit(victim)
                        .map_or(0, |state| state.desired_speed_cms),
                ));
            }
        },
    );
    assert!(of_kind(&control_lines, "mind_dropped").is_empty());
    assert_eq!(
        held,
        Some((goal_after, speed_after)),
        "the DRIVE goal and desired speed survive the drop"
    );
    assert_eq!(goal_after, Some((2, 14)));
    // Three cadences.
    let slow = core_run(
        doctored_line_tank(victim, Controller::Mind, destroy_words()),
        5_000,
        10_000,
    );
    assert_eq!(of_kind(&slow.0, "mind_dropped").len(), 1);
    for cadence in [1_000, 500] {
        let other = core_run(
            doctored_line_tank(victim, Controller::Mind, destroy_words()),
            cadence,
            10_000,
        );
        assert_eq!(other.0, slow.0, "the event list at {cadence} ms");
        assert_eq!(other.1, slow.1, "the hash sequence at {cadence} ms");
    }

    // (2) No think in flight: a hull whose think lands before the round.
    let quiet = (18_u32..400)
        .map(ObjId)
        .find(|id| (100..=700).contains(&stagger_ms(SEED, *id, STOCK_THINK_MS)))
        .expect("a hull whose think is not in flight at 5,000");
    let next = GATE_END + stagger_ms(SEED, quiet, STOCK_THINK_MS);
    let (mut arena, recorded) =
        core_mind_field(doctored_line_tank(quiet, Controller::Mind, destroy_words()));
    let (lines, _) = beats(
        &mut arena,
        SUBSTEP,
        7_000,
        &|wall| shooter_orders(quiet, wall),
        &mut |_, _| {},
    );
    assert_eq!(of_kind(&lines, "hit").len(), 1);
    let dropped = of_kind(&lines, "mind_dropped");
    assert_eq!(
        dropped.iter().map(|line| line.0).collect::<Vec<_>>(),
        vec![next],
        "at the next think's millisecond"
    );
    let at_next = recorded
        .kept()
        .into_iter()
        .find(|kept| kept.unit == quiet && kept.t_ms == next)
        .expect("the next think");
    assert!(at_next.dropped, "the dropping think is the ROM's");
    assert_eq!(at_next.result.trace, vec![DROPPED_LINE.to_owned()]);
    let slow = core_run(
        doctored_line_tank(quiet, Controller::Mind, destroy_words()),
        5_000,
        10_000,
    );
    assert_eq!(of_kind(&slow.0, "mind_dropped").len(), 1);
    for cadence in [1_000, 500] {
        let other = core_run(
            doctored_line_tank(quiet, Controller::Mind, destroy_words()),
            cadence,
            10_000,
        );
        assert_eq!(
            other.0, slow.0,
            "no think in flight: the event list at {cadence} ms"
        );
        assert_eq!(
            other.1, slow.1,
            "no think in flight: the hashes at {cadence} ms"
        );
    }

    // (3) A pilot's hull.
    let pilot = ObjId(1_000);
    let mut arena = consequence_field(
        doctored_line_tank(victim, Controller::Pilot, destroy_words()),
        true,
    );
    let verbs = |wall: i64| {
        let mut orders = shooter_orders(victim, wall);
        match wall {
            0 => orders.push(order(victim, IntentKind::Startup, 11)),
            10_000 => orders.extend([
                order(victim, IntentKind::Speed { cms: 300 }, 12),
                order(
                    victim,
                    IntentKind::Fire {
                        weapon: SmolStr::new("cannon"),
                        slot: None,
                    },
                    13,
                ),
                order(
                    victim,
                    IntentKind::Condition(Arming::Arm(FireCondition {
                        weapon: SmolStr::new("machine_gun"),
                        target: SlotTarget::Lock,
                        min_permille: 1_000,
                        aspects: ASPECTS_ANY,
                        min_rounds_left: 0,
                        min_range_m: 0,
                        max_range_m: u32::MAX,
                    })),
                    14,
                ),
            ]),
            _ => {}
        }
        orders
    };
    let mut refused: Vec<(SmolStr, Option<SmolStr>)> = Vec::new();
    let (lines, _) = beats(
        &mut arena,
        SUBSTEP,
        10_500,
        &verbs,
        &mut |arena: &Arena, out: &StepOutput| {
            if arena.now_ms < 10_000 {
                return;
            }
            for (to, frame) in &out.frames {
                if *to != pilot {
                    continue;
                }
                for notice in &frame.notices {
                    if let FrameNotice::Refused { order, weapon, .. } = notice {
                        refused.push((order.clone(), weapon.clone()));
                    }
                }
            }
        },
    );
    let hits = of_kind(&lines, "hit");
    assert_eq!(hits.len(), 1);
    assert!(hits[0].1.contains("destroy_core"), "{}", hits[0].1);
    assert_eq!(arena.core_of(victim), None);
    assert!(
        !arena.minds.contains_key(&victim),
        "a pilot's hull has no row"
    );
    assert!(of_kind(&lines, "mind_dropped").is_empty(), "and no line");
    assert_eq!(
        arena.unit(victim).map(|state| state.desired_speed_cms),
        Some(300),
        "the speed was taken"
    );
    assert!(
        refused
            .iter()
            .any(|(order, weapon)| order == "fire" && weapon.as_deref() == Some("cannon")),
        "the cannon's FIRE is refused by name: {refused:?}"
    );
    assert_eq!(
        arena.conditions.slot_of(victim, "machine_gun"),
        Some(1),
        "the coax armed in Class 0's one slot"
    );
    assert_eq!(arena.slot_capacity(victim), 1);
}

/// **THE GATE FIXTURE, on the SHIPPED content: `destroy_core` on the line
/// tank's turret drops its mind at the next landing** (17b cut M3.6; the M3
/// gate's (e); doc 17 §9, D-UM-34). The fielded catalog AS SHIPPED — no
/// doctored list: `tacsim.pack.line_tank` version 4 authors the turret
/// `[remove_weapons, destroy_core]` itself — instantiated through
/// [`doctored_hull`] only for its geometry (every band to the turret, the coax
/// and a bow gun on the hull), flown by [`core_mind`] as unit 17 and its
/// turret killed by the seated blue's beam at 5,000, at 5,000, 1,000 and 500
/// ms: the `hit` carries the shipped words, the core capability is gone, ONE
/// `mind_dropped { core }` lands at the think in flight's landing, the row is
/// dropped for the rest of the match (`DropCause::Core`), and the three runs
/// are one event list and one hash sequence.
#[test]
fn m3_the_shipped_line_tanks_turret_drops_its_mind_at_three_cadences() {
    let catalog = tacsim::definitions::Catalog::build_fielded(&[], tacsim::NS)
        .expect("the fielded content binds");
    let shipped = catalog
        .specs
        .get(&canonical("tacsim.unit.line_tank"))
        .and_then(|spec| {
            spec.sections
                .iter()
                .find(|section| section.id.as_str() == "turret")
        })
        .map(|section| section.consequences.clone());
    assert_eq!(
        shipped,
        Some(destroy_words()),
        "the shipped turret authors the core's word"
    );
    let victim = GATE_UNIT;
    let think = GATE_END - STOCK_THINK_MS + stagger_ms(SEED, victim, STOCK_THINK_MS);
    let lands = think + SUBSTEP;
    let mut runs: Vec<(i64, BeatRun)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let hull = doctored_hull(
            &catalog,
            "tacsim.unit.line_tank",
            victim,
            Controller::Mind,
            &["cannon"],
            &["bow"],
        );
        let (mut arena, _) = core_mind_field(hull);
        let run = beats(
            &mut arena,
            cadence,
            10_000,
            &|wall| shooter_orders(victim, wall),
            &mut |_, _| {},
        );
        let hits = of_kind(&run.0, "hit");
        assert_eq!(hits.len(), 1, "one round at {cadence} ms: {hits:?}");
        let hit: Json = serde_json::from_str(&hits[0].1).unwrap_or_default();
        assert_eq!(
            hit["consequences"],
            serde_json::json!(["remove_weapons", "destroy_core"]),
            "the shipped words on the hit: {hit}"
        );
        assert_eq!(arena.core_of(victim), None, "the line core is gone");
        let dropped = of_kind(&run.0, "mind_dropped");
        assert_eq!(dropped.len(), 1, "one drop at {cadence} ms: {dropped:?}");
        assert_eq!(dropped[0].0, lands, "at the think in flight's landing");
        assert_eq!(
            serde_json::from_str::<Json>(&dropped[0].1).unwrap_or_default(),
            serde_json::json!({"kind": "mind_dropped", "unit": victim.0, "cause": "core"})
        );
        assert_eq!(
            arena.minds.get(&victim).and_then(|state| state.dropped),
            Some(tacsim::arena::mind::DropCause::Core),
            "dropped for the rest of the match"
        );
        runs.push((cadence, run));
    }
    let Some((_, first)) = runs.first() else {
        panic!("three runs");
    };
    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(run.0, first.0, "the event list at {cadence} ms");
        assert_eq!(run.1, first.1, "the hash sequence at {cadence} ms");
    }
}

/// **`degrade_core` lowers the next think's row, at three cadences** (17b
/// cut M3.4; doc 17 §9, D-UM-34, D-UM-30). A rated-III mind
/// ([`rate`], the seal's class since 17b cut M4.1) on a DOCTORED heavy — its command
/// core (Class III) in the hull, its turret list `[remove_weapons,
/// degrade_core]`, its three guns remounted on the hull and three more added
/// — whose first think writes a hundred memory keys (under III's 128) and
/// arms six slots (III's six); the seated blue's beam kills the turret at
/// 5,000:
///
/// * the `hit` carries `degrade_core`; the core's cell reads 2 (III → II);
/// * the next think after the round is handed Class II's row (and every
///   think before it III's); the row is cut to II's four — S5 and S6 gone,
///   S1 to S4 kept;
/// * the hundred keys are kept whole by the thinks under II's 64 that
///   write no new key, and the one think that swaps a key in (between
///   6,000 and 6,500) faults `memory` — a `mind_fault` line — keeping the
///   hundred;
/// * no drop: the core stands, one class down;
/// * one event list and one hash sequence at 5,000, 1,000 and 500 ms.
#[test]
#[allow(clippy::too_many_lines)]
fn m3_degrade_core_lowers_the_next_think_s_row_at_three_cadences() {
    use tacsim::arena::core::{ClassTable, CoreClass};
    use tacsim::definitions::unit::Consequence as Authored;
    let heavy = GATE_UNIT;
    let words = || {
        vec![
            Authored::RemoveWeapons {
                section: canonical("turret"),
            },
            Authored::DegradeCore,
        ]
    };
    let source = format!(
        "fn think(v, c) {{
    if v.unit != {unit} {{ return; }}
    if !(\"k0\" in this) {{
        for i in 0..100 {{ this[\"k\" + i.to_string()] = i; }}
        {cannon} {coax} {missiles} {w1} {w2} {w3}
        return;
    }}
    let t = t_ms();
    if t >= 6000 && t < 6500 {{ this.remove(\"k0\"); this.fresh = 1; }} else {{ this.k1 = t; }}
}}",
        unit = heavy.0,
        cannon = arm_never("cannon"),
        coax = arm_never("machine_gun"),
        missiles = arm_never("missiles"),
        w1 = arm_never("w1"),
        w2 = arm_never("w2"),
        w3 = arm_never("w3"),
    );
    let field = || {
        let catalog = doctored_catalog("tacsim.unit.heavy_mk1", words());
        let hull = doctored_hull(
            &catalog,
            "tacsim.unit.heavy_mk1",
            heavy,
            Controller::Mind,
            &[],
            &["w1", "w2", "w3"],
        );
        let mut arena = consequence_field(hull, false);
        rate(&mut arena, heavy, CoreClass::III);
        let recorded = Recorded::on(rhai(&arena, &source));
        arena.mind_runner = recorded.clone();
        (arena, recorded)
    };
    let (mut arena, recorded) = field();
    assert_eq!(
        arena.core_of(heavy).map(|core| core.class),
        Some(CoreClass::III),
        "the command core"
    );
    let mut rows: Vec<(i64, Vec<Option<String>>)> = Vec::new();
    let (lines, _) = beats(
        &mut arena,
        SUBSTEP,
        8_000,
        &|wall| shooter_orders(heavy, wall),
        &mut |arena: &Arena, _: &StepOutput| {
            rows.push((
                arena.now_ms,
                arena
                    .conditions
                    .slots(heavy)
                    .iter()
                    .map(|slot| {
                        slot.as_ref()
                            .and_then(tacsim::arena::conditions::Rule::fire)
                            .map(|cond| cond.weapon.to_string())
                    })
                    .collect(),
            ));
        },
    );
    let hits = of_kind(&lines, "hit");
    assert_eq!(hits.len(), 1);
    assert_eq!(
        serde_json::from_str::<Json>(&hits[0].1).unwrap_or_default()["consequences"],
        serde_json::json!(["remove_weapons", "degrade_core"])
    );
    assert_eq!(
        arena.core_of(heavy).map(|core| core.class),
        Some(CoreClass::II),
        "the cell 3 → 2"
    );
    let armed = |at: i64| {
        rows.iter()
            .find(|(now, _)| *now == at)
            .map(|(_, row)| row.clone())
            .expect("a pulse")
    };
    let six: Vec<Option<String>> = ["cannon", "machine_gun", "missiles", "w1", "w2", "w3"]
        .into_iter()
        .map(|gun| Some(gun.to_owned()))
        .collect();
    assert_eq!(armed(GATE_END - SUBSTEP), six, "six slots before the round");
    assert_eq!(
        armed(GATE_END + SUBSTEP),
        six[..4].to_vec(),
        "the row cut to II's four"
    );
    let kept = recorded.kept();
    let thinks: Vec<&Kept> = kept.iter().filter(|kept| kept.unit == heavy).collect();
    let next = thinks
        .iter()
        .find(|kept| kept.t_ms > GATE_END)
        .expect("a think after the round");
    assert_eq!(
        next.profile,
        ClassTable::STOCK.row(CoreClass::II),
        "the next think at II"
    );
    assert!(
        thinks
            .iter()
            .filter(|kept| kept.t_ms <= GATE_END)
            .all(|kept| kept.profile == ClassTable::STOCK.row(CoreClass::III)),
        "every think before at III"
    );
    // The hundred keys, kept whole under II's 64; the swap refused.
    for kept in thinks.iter().filter(|kept| kept.t_ms > GATE_END) {
        let swap = (6_000..6_500).contains(&kept.t_ms);
        let memory = kept.memory_out();
        if swap {
            assert_eq!(
                kept.result.fault.as_ref().map(|fault| fault.cause.as_str()),
                Some("memory"),
                "the swap at {} is refused",
                kept.t_ms
            );
        } else {
            assert_eq!(kept.result.fault, None, "think at {}", kept.t_ms);
            assert_eq!(memory.len(), 100, "kept whole at {}", kept.t_ms);
        }
    }
    let faults = of_kind(&lines, "mind_fault");
    assert_eq!(faults.len(), 1, "{faults:?}");
    assert!(
        faults[0].1.contains("\"cause\":\"memory\""),
        "{}",
        faults[0].1
    );
    assert!(
        arena
            .minds
            .get(&heavy)
            .is_some_and(|state| decode_memory(&state.memory).len() == 100),
        "the hundred kept"
    );
    assert!(of_kind(&lines, "mind_dropped").is_empty(), "no drop");
    // Three cadences.
    let run_at = |cadence: i64| {
        let (mut arena, _) = field();
        beats(
            &mut arena,
            cadence,
            10_000,
            &|wall| shooter_orders(heavy, wall),
            &mut |_, _| {},
        )
    };
    let slow = run_at(5_000);
    // What the compared runs carry: the round's `degrade_core`, the swap's
    // `memory` fault and no drop — and, to 8,000, the very lines of the
    // quarter-second run above.
    let slow_hits = of_kind(&slow.0, "hit");
    assert_eq!(slow_hits.len(), 1, "the round lands at 5,000 ms too");
    assert_eq!(
        serde_json::from_str::<Json>(&slow_hits[0].1).unwrap_or_default()["consequences"],
        serde_json::json!(["remove_weapons", "degrade_core"])
    );
    let slow_faults = of_kind(&slow.0, "mind_fault");
    assert_eq!(slow_faults.len(), 1, "{slow_faults:?}");
    assert!(
        slow_faults[0].1.contains("\"cause\":\"memory\""),
        "{}",
        slow_faults[0].1
    );
    assert!(of_kind(&slow.0, "mind_dropped").is_empty(), "no drop");
    let to_eight = |lines: &[(i64, String, u64)]| {
        lines
            .iter()
            .filter(|(t_ms, _, _)| *t_ms <= 8_000)
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(
        to_eight(&slow.0),
        to_eight(&lines),
        "the 5,000 ms run's lines are the quarter-second run's"
    );
    for cadence in [1_000, 500] {
        let other = run_at(cadence);
        assert_eq!(other.0, slow.0, "the event list at {cadence} ms");
        assert_eq!(other.1, slow.1, "the hash sequence at {cadence} ms");
    }
}

/// `tests/reflex.rs`' tank with NO core: the (j) rig's `tactical_unit`
/// without its core component and capability.
fn coreless_unit(id: ObjId) -> TacticalUnit {
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
    TacticalUnit::from_spec(&spec, id).expect("a coreless unit")
}

/// **A never-cored mind hull runs the ROM from its first think, and no
/// `mind_dropped` line is ever written** (17b cut M3.4; doc 17 §9, D-UM-33):
/// red fielded with no core is sealed `core_class` 0 with the fallback "no
/// core: the ROM"; flown by a scripted runner for ten seconds, every think
/// is handed dropped at Class 0's row and answered by the ROM (its trace
/// [`DROPPED_LINE`], no fault, no `ThinkStat`), and the tape carries no
/// `mind_dropped` and no `mind_fault`.
#[test]
fn m3_a_never_cored_mind_runs_the_rom_from_its_first_think_with_no_drop_line() {
    use tacsim::arena::core::{ClassTable, CoreClass};
    let red = CustodyUnit {
        unit: coreless_unit(RED),
        sensor: Some(SensorProfile {
            max_range_cells: 10,
            ..sight()
        }),
        ..tank(
            RED,
            Team::Red,
            centre(6, 8),
            vec![cannon()],
            Controller::Mind,
        )
    }
    .sealed();
    assert_eq!(red.seal.core_class, CoreClass::Zero);
    assert_eq!(red.seal.fallback.as_deref(), Some("no core: the ROM"));
    let mut arena = arena_on(
        vec![
            red,
            presenting(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            )),
        ],
        &[BLUE],
        reflex_cfg(),
    );
    let source = "fn think(v, c) { trace(\"scripted\"); this.n = 1; }";
    let runner = rhai(&arena, source);
    let recorded = Recorded::on(runner.clone());
    arena.mind_runner = recorded.clone();
    let run = run(&mut arena, Drive::Every(1_000), 10_000);
    let thinks: Vec<Kept> = recorded
        .kept()
        .into_iter()
        .filter(|kept| kept.unit == RED)
        .collect();
    assert!(thinks.len() >= 9, "red thought: {}", thinks.len());
    for think in &thinks {
        assert!(think.dropped, "think {} is the ROM's", think.think_seq);
        assert_eq!(think.profile, ClassTable::STOCK.row(CoreClass::Zero));
        assert_eq!(think.result.trace, vec![DROPPED_LINE.to_owned()]);
        assert_eq!(think.result.fault, None);
    }
    assert!(runner.take_stats().is_empty(), "no Rhai think ran");
    assert!(
        run.events.iter().all(|(_, json, _)| {
            let kind = kind_and_unit(json).0;
            kind != "mind_dropped" && kind != "mind_fault"
        }),
        "no drop line, ever"
    );
    assert_eq!(arena.minds.get(&RED).and_then(|state| state.dropped), None);
}

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
//! Recording a runner's answers (`Recorded`) and replaying them
//! (`ScriptedMind`) are inputs to `Arena::step`, as a scripted intent log is,
//! never a stand-in for a production module (16b §0.4).

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
use tacsim::arena::perception::{
    FrameNotice, FrameOrder, Jammer, MindObjective, Named, Receiver, SensorProfile,
    SignatureFactor, bearing_of,
};
use tacsim::arena::reflex::ReflexCounters;
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
use tacsim::arena::{CardChange, StockCard};
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
    counters: ReflexCounters,
}

impl Rig {
    fn new(arena: Arena) -> Rig {
        Rig {
            arena,
            wall: None,
            bytes: Vec::new(),
            hashes: Vec::new(),
            counters: ReflexCounters::default(),
        }
    }

    /// One pulse of one substep.
    fn pulse(&mut self) -> StepOutput {
        let wall = self.wall.map_or(0, |wall| wall + SUBSTEP);
        self.wall = Some(wall);
        let out = self.arena.step(wall, &[], &[]);
        for chunk in &out.ledger_chunks {
            self.bytes.extend_from_slice(&chunk.bytes);
        }
        self.hashes.push(self.arena.substep_hashes.clone());
        self.counters.absorb(&out.reflex);
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
    view: Arc<MindView>,
    memory: Vec<u8>,
    map: Arc<ArenaDefinition>,
    card: StockCard,
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
            view: &self.view,
            memory: &self.memory,
            map: &self.map,
            card: &self.card,
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
                view: Arc::clone(job.view),
                memory: job.memory.to_vec(),
                map: Arc::clone(job.map),
                card: job.card.clone(),
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
        ("View", "receiver" | "heat" | "order")
        | ("Contact", "condition")
        | ("Weapon", "armed") => Store::Handle,
        ("View", "sections" | "weapons" | "notices" | "contacts") | ("Section", "facets") => {
            Store::Len
        }
        ("View", "scan_tables") => Store::Tables,
        ("Contact", "fatal") => Store::Words,
        ("Contact", "odds") => Store::Odds,
        _ => Store::Plain,
    }
}

/// **The parity script for one handle**: `x` is the handle at `path`, and
/// every getter of `ty` is written to the memory under its own name.
fn parity_source(ty: &str, path: &str) -> String {
    let mut body = String::new();
    for (row_ty, getter, _) in PARITY {
        if *row_ty != ty {
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
        ("View", "sections" | "weapons" | "contacts") => len_mem(key(getter)),
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
    card: StockCard,
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
        card: arena.minds.get(&unit).expect("a mind row").card.clone(),
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
        let source = parity_source(handle.ty, &handle.path);
        let runner = RhaiRunner::new(&ArenaConfig::default(), &source, digest(&source))
            .unwrap_or_else(|error| panic!("{source}: {error}"));
        let job = ThinkJob {
            unit,
            think_seq: 0,
            t_ms: view.t_ms,
            lands_ms: view.t_ms + SUBSTEP,
            seed: SEED,
            dropped: false,
            view: &moment.view,
            memory: &[],
            map: &moment.map,
            card: &moment.card,
        };
        let result = runner.think(&job);
        assert_eq!(
            result.fault, None,
            "{} at {}: {:?}",
            handle.ty, handle.path, result.trace
        );
        let memory = decode_memory(&result.memory);
        for (ty, getter, rendering) in PARITY {
            if *ty != handle.ty {
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
            if want != MemValue::Unit && want != MemValue::Str("()".to_owned()) && !printed_null {
                exercised.valued.insert(row.clone());
            }
            exercised.rows.insert(row);
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
/// the wingman. Every one of `PARITY`'s 114 rows is exercised, and every row
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

    let rows: BTreeSet<(String, String)> = PARITY
        .iter()
        .map(|(ty, getter, _)| ((*ty).to_owned(), (*getter).to_owned()))
        .collect();
    assert_eq!(PARITY.len(), 114);
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
/// * that list is the landing put on the scratch, stamped with the hull's
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
    // The one door's input: the landing on the scratch, at the think's
    // epochs, one substep later.
    let landing = rig
        .arena
        .mind_landings
        .get(&RED)
        .cloned()
        .expect("think 3 waits to land");
    assert_eq!(landing.think_seq, 3);
    assert_eq!(landing.intents, expected);
    assert_eq!(
        landing.epochs,
        Channel::ALL.map(|channel| (channel, rig.arena.epochs.current(RED, channel)))
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
/// after each pulse, whether red had a landing waiting after each pulse (on
/// the scratch, on the heap), and the rig.
struct FaultRun {
    recorded: Arc<Recorded>,
    runner: Arc<RhaiRunner>,
    rows: Vec<tacsim::arena::mind::MindState>,
    landings: Vec<(bool, bool)>,
    rig: Rig,
}

/// Whether red has a landing waiting: on the scratch, and a `MindLand` on
/// the heap.
fn red_landing(arena: &Arena) -> (bool, bool) {
    (
        arena.mind_landings.contains_key(&RED),
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
///   nothing (no landing on the scratch or the heap at the end of the pulse
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
            "{}: no landing on the scratch or the heap",
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
    // landing is waiting, on the scratch and on the heap, at the end of the
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
    let ints = |ranges: tacsim::arena::reflex::Ranges| {
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
    let card = arena.minds.get(&RED).expect("red's row").card.clone();
    let result = runner.think(&ThinkJob {
        unit: RED,
        think_seq: 0,
        t_ms: view.t_ms,
        lands_ms: view.t_ms + SUBSTEP,
        seed: arena.seed,
        dropped: false,
        view,
        memory: &[],
        map: &arena.definition,
        card: &card,
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
        assert_eq!(&think.card, card, "think {}", think.think_seq);
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
        rig.arena.minds.get(&RED).map(|state| state.card.clone()),
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
/// its intents are T1's (the fire half's `Condition`/`Fire` filtered out on
/// the armed red until 17b cut M2.6).
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
            let card = arena
                .minds
                .get(&unit)
                .map(|mind| mind.card.clone())
                .expect("a mind row");
            let job = ThinkJob {
                unit,
                think_seq: 0,
                t_ms: arena.now_ms,
                lands_ms: arena.now_ms + arena.substep_len_ms(),
                seed: arena.seed,
                dropped: false,
                view: &view,
                memory: &[],
                map: &arena.definition,
                card: &card,
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
            let helm: Vec<&IntentKind> = native
                .intents
                .iter()
                .filter(|kind| !matches!(kind, IntentKind::Condition(_) | IntentKind::Fire { .. }))
                .collect();
            assert_eq!(rhai.intents.iter().collect::<Vec<&IntentKind>>(), helm);
            if !online {
                assert_eq!(native.intents, vec![IntentKind::Startup]);
            }
        }
    }
}

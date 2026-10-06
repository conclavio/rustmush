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
//!
//! **17b cut M1b.5** adds the last section: the card T1 reads. The anchor
//! (17b's Q-M6) — the two-T1 fixture's event list and every think's answer
//! recorded at 17b cut M1b.4's HEAD before `NativeStockMind` read its card
//! through `StockCard::as_reflex`, and equal after it and after the memory's
//! `ignore_until_ms` rename — and a non-default doctrine withdrawing where it
//! did; a card staged at a beat landing at the pulse's start with one
//! `mind_card` line at three cadences, the next think re-arming on it and the
//! overlay read (Rearguard withdraws at 70 where Line stood), and the arena's
//! refusals (`not_mind`, a slider's own sentence, its OWN floor's
//! `unlawful`); a preset on an unarmed hull with its fire section inert and
//! no slot; and a newer order obeyed at once after an early end.

mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use mush_core::{
    CanonicalId, DefinitionDigest, DefinitionId, DefinitionKey, DefinitionKind, ObjId, OwnerId,
};
use mush_sim::data::Pose;
use mush_tactics::{
    CellAddr, CellSpec, DefinitionRef, FacetId, FacetSpec, RulesValue, SectionId, SectionSpec,
    TacticalUnit, UnitMutationOp, UnitMutationPlan, UnitSpec,
};
use smol_str::SmolStr;
use tacsim::arena::CardChange;
use tacsim::arena::clock::Tempo;
use tacsim::arena::command::SquadSignal;
use tacsim::arena::condition::{Condition, Reading};
use tacsim::arena::conditions::{
    ASPECTS_ANY, Arming, FireCondition, STOCK_CONDITION_SLOTS, SlotTarget,
};
use tacsim::arena::intents::{
    Control, ControlState, Intent, IntentKind, IntentOutcome, IntentSource, MotorGoal, Refusal,
    apply_control, apply_intent,
};
use tacsim::arena::ledger::LedgerEvent;
use tacsim::arena::mind::{
    MIND_INTENTS_MAX, MIND_NOTICES, MIND_STRIKES_TO_DROP, MindRunner, MindState, MindView,
    STOCK_THINK_MS, ThinkFault, ThinkJob, ThinkResult,
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
use tacsim::arena::weapons::{FrameArming, WeaponBand};
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
    arena_on(units, seated, reflex_cfg())
}

/// [`arena_of`] on the doctrine `reflex` (17b cut M1b.5: a fixture whose
/// minds' Line card is read off a non-default `ReflexConfig`).
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

/// **Whether `viewer` holds `target` and has told it apart** (17b cut M1b.3):
/// the row an order's contact re-keys to (`Arena::rekey_order` keeps only a
/// classified row).
fn told_apart(arena: &Arena, viewer: ObjId, target: ObjId) -> bool {
    arena
        .contacts
        .id_of(viewer, target)
        .and_then(|id| arena.contacts.get(viewer, id))
        .is_some_and(|row| matches!(row.class, Classification::Classified { .. }))
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
// 17b cut M1b.2: a mind's FIRE cites a slot (doc 17 §5, D-UM-17)
// ---------------------------------------------------------------------------

/// **The arming a scripted mind lands before the `Fire` that cites it**: the
/// cannon in S1, on the lock, at the Compact floor (100 per mille), every
/// aspect, no reserve, any range. Since 17b cut M1b.2 an uncited mind `FIRE`
/// has no warrant.
///
/// A slot armed on a held classified lock FIRES at the `Landing` evaluation
/// of the think that armed it when its gun is ready and no cited `Fire`
/// landed in that think, and again at the gun's every `WeaponReady` — so the
/// scripts below land the arming in the same think as the cited `Fire` (the
/// `Fire` after it in the vec, as a mind emits them), and let the slot go
/// ([`disarm_cannon`]) before the gun's next `WeaponReady` wherever the run
/// would reach it, so every recorded `fired` millisecond is the one the
/// uncited script fired at.
fn arm_cannon() -> IntentKind {
    IntentKind::Condition(Arming::Arm(slot_on_lock(100)))
}

/// The cannon's `Fire`, citing S1.
fn cited_fire() -> IntentKind {
    IntentKind::Fire {
        weapon: SmolStr::new("cannon"),
        slot: Some(1),
    }
}

/// The cannon's slot let go.
fn disarm_cannon() -> IntentKind {
    IntentKind::Condition(Arming::Disarm {
        weapon: Some(SmolStr::new("cannon")),
    })
}

/// **`unit` presenting a silhouette** ([`every_band_profile`]): a target with
/// the default profile presents nothing, so `Arena::resolve_seen` answers
/// exactly nought (doc 16 §3.5: *zero presented area is zero*) and every
/// slot on it holds — and every mind citation of it is `unwarranted` — under
/// any lawful odds term (the divergence `floor_held`). The fixtures whose
/// minds fire since 17b cut M1b.2 shoot at a target that can be hit.
fn presenting(unit: CustodyUnit) -> CustodyUnit {
    CustodyUnit {
        profile: every_band_profile(),
        ..unit
    }
}

// ---------------------------------------------------------------------------
// The scripted mind
// ---------------------------------------------------------------------------

/// One scripted think: what it answers, and the memory it hands back (the
/// memory it was handed, echoed, when `None`) — and, since 17b cut M2.2, the
/// fault it answers (D-UM-31: a cause word, or `None` for a clean think).
#[derive(Clone, Debug)]
struct Says {
    think_seq: u64,
    intents: Vec<IntentKind>,
    memory: Option<Vec<u8>>,
    fault: Option<&'static str>,
}

fn says(think_seq: u64, intents: Vec<IntentKind>) -> Says {
    Says {
        think_seq,
        intents,
        memory: None,
        fault: None,
    }
}

/// What a think was handed — and, since 17b cut M2.2, the match seed the job
/// carried (`ThinkJob::seed`, D-UM-29) and whether it said the hull is
/// dropped (`ThinkJob::dropped`).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen {
    unit: ObjId,
    think_seq: u64,
    t_ms: i64,
    view: MindView,
    memory: Vec<u8>,
    seed: u64,
    dropped: bool,
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
                view: (**job.view).clone(),
                memory: job.memory.to_vec(),
                seed: job.seed,
                dropped: job.dropped,
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
            fault: line.and_then(|line| line.fault).map(|cause| ThinkFault {
                cause: SmolStr::new_static(cause),
            }),
            trace: Vec::new(),
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
/// nothing. **17b cut M1b.2**: each `FIRE` is armed and cited in one think,
/// and the seen blue presents a silhouette ([`presenting`]), so the cited
/// shot has odds to be warranted by.
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
            presenting(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            )),
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
    // 17b cut M1b.2: each mind's `FIRE` cites S1, armed in the same think
    // just before it ([`arm_cannon`]'s note); the armed red lets its slot go
    // at think 6, before the cannon's next `WeaponReady`.
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
                says(5, vec![arm_cannon(), cited_fire()]),
                says(6, vec![IntentKind::Emcon { on: true }, disarm_cannon()]),
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
                says(4, vec![arm_cannon(), cited_fire()]),
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
///
/// **17b cut M1b.2** re-scripted both `FIRE`s to arm S1 and cite it in one
/// think ([`arm_cannon`]'s note): the armed red's one shot is `by: "mind"`,
/// `slot: 1`, at the millisecond the uncited script fired it; the unarmed
/// red's arming and its `FIRE` are both refused `channel`.
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
    // 17b cut M1b.2: the one shot is the armed red's think-5 `FIRE`, cited —
    // at think 5's landing, as the uncited script fired it — and nothing else
    // fires (the slot went at think 6).
    let fired: Vec<(i64, Option<String>, Option<u64>)> = first
        .events
        .iter()
        .filter(|(_, json, _)| kind_and_unit(json).0 == "fired")
        .map(|(t_ms, json, _)| {
            let value: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
            (
                *t_ms,
                value
                    .get("by")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                value.get("slot").and_then(serde_json::Value::as_u64),
            )
        })
        .collect();
    assert_eq!(
        fired,
        vec![(think_at(RED, 5) + SUBSTEP, Some("mind".to_owned()), Some(1))]
    );
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
            presenting(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            )),
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
    // 17b cut M1b.2: the `FIRE` cites S1, armed just before it in think 4
    // ([`arm_cannon`]'s note).
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
                    arm_cannon(),
                    cited_fire(),
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
///
/// **17b cut M1b.2**: think 4 arms S1 and its `FIRE` cites it
/// ([`arm_cannon`]'s note) — the stale thoughts are still refused `epoch` at
/// apply, before any resolver gate; under the linkdead control the arming is
/// refused `linkdead` with the `FIRE`; with the helm raised alone the cited
/// shot fires `by: "mind"` at the same millisecond.
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
    // (17b cut M1b.2: the arming is a discrete order on the unraised fire
    // channel too, and meets the same linkdead gate before the `FIRE` does.)
    assert_eq!(
        refusals(&rig.arena, RED),
        vec![
            ("heading".to_owned(), Refusal::Epoch),
            ("speed".to_owned(), Refusal::Epoch),
            ("fire".to_owned(), Refusal::Linkdead),
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
    let cited = rig.journal().iter().any(|event| {
        let value = serde_json::to_value(&event.kind).unwrap_or_default();
        value.get("kind") == Some(&serde_json::json!("fired"))
            && value.get("by") == Some(&serde_json::json!("mind"))
            && value.get("slot") == Some(&serde_json::json!(1))
    });
    assert!(cited, "the shot cites S1 (17b cut M1b.2)");
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
///
/// **17b cut M1b.2**: the armed mind's two `FIRE`s cite S1, armed just before
/// them in the same think ([`arm_cannon`]'s note), so the first fires and the
/// second still reads `cycling` — never `unwarranted`; the unarmed mind's
/// uncited `FIRE` reads `channel` at apply, before the law is asked.
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
            presenting(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            )),
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
                        // 17b cut M1b.2: both `FIRE`s cite S1, armed first.
                        arm_cannon(),
                        cited_fire(),
                        cited_fire(),
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
                fault: None,
            },
            Says {
                think_seq: 1,
                intents: vec![IntentKind::Speed { cms: 300 }; MIND_INTENTS_MAX + 1],
                memory: Some(vec![9, 9]),
                fault: None,
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
                fault: None,
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
    let decoded = Snapshot::decode(&snapshot.document, (*rig.arena.definition).clone())
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
                    fault: None,
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

/// **The shot a mind's beam fixture fires** (17b cut M1b.0): every beam test
/// of this section fires through here and nowhere else. **17b cut M1b.2** put
/// the law on a mind's fire and swapped the body for an arming on the lock
/// and a `FIRE` citing it — the cited shot lands at the same landing, so every
/// proof keeps its millisecond ([`arm_cannon`]'s note; [`beam_field`] lets
/// the slot go at the next think, before the gun's `WeaponReady`).
fn beam_shot(unit: ObjId) -> Vec<IntentKind> {
    let _ = unit;
    vec![
        IntentKind::Condition(Arming::Arm(FireCondition {
            weapon: SmolStr::new("cannon"),
            target: SlotTarget::Lock,
            min_permille: 100,
            aspects: ASPECTS_ANY,
            min_rounds_left: 0,
            min_range_m: 0,
            max_range_m: u32::MAX,
        })),
        IntentKind::Fire {
            weapon: SmolStr::new("cannon"),
            slot: Some(1),
        },
    ]
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
/// and, since 17b cut M1b.2, lets the slot that shot cited go at the next
/// think ([`disarm_cannon`]), and says nothing else.
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
        // 17b cut M1b.2: the slot the shot cited goes at the next think, so
        // the beam's `WeaponReady` a cycle on fires nothing.
        script.push((
            viewer,
            vec![
                says(shoot_at, beam_shot(viewer)),
                says(shoot_at + 1, vec![disarm_cannon()]),
            ],
        ));
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
    // 17b cut M1b.2: the CITED shot ([`beam_shot`]), not the slot's own at the
    // same landing — which would keep the millisecond and hide a refused
    // citation.
    let attribution = serde_json::to_value(&fired.kind).unwrap_or_default();
    assert_eq!(
        (
            attribution.get("by").and_then(serde_json::Value::as_str),
            attribution.get("slot").and_then(serde_json::Value::as_u64)
        ),
        (Some("mind"), Some(1)),
        "the beam is the mind's cited shot: {attribution}"
    );
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
    // 17b cut M1b.2: both are the minds' CITED shots ([`beam_shot`]).
    let cited: Vec<(Option<String>, Option<u64>)> = rig
        .journal()
        .iter()
        .filter_map(|event| {
            let value = serde_json::to_value(&event.kind).ok()?;
            (value.get("kind")? == "fired").then(|| {
                (
                    value
                        .get("by")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                    value.get("slot").and_then(serde_json::Value::as_u64),
                )
            })
        })
        .collect();
    assert_eq!(
        cited,
        vec![(Some("mind".to_owned()), Some(1)); 2],
        "both beams cited S1"
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
    // After the passes: both hold blue — and, since 17b cut M1b.3 (D-UM-17's
    // rule applied to the hint), the mind has told its rows apart: an unknown
    // row re-keys to nothing (`m1b3_an_order_hint_re_keys_only_to_a_classified_row`).
    let _ = rig.until(5_000, |arena| {
        told_apart(arena, RED, BLUE)
            && arena.contacts.id_of(RED2, BLUE).is_some()
            && told_apart(arena, RED, RED2)
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

/// **An order's hint re-keys only to a CLASSIFIED row** (17b cut M1b.3; the
/// lead's ruling on M1a's residual (2), D-UM-17's rule applied to the hint;
/// `Arena::rekey_order`'s one resolution closure and its `Support` arm).
///
/// No tank carries a sensor, so nothing but the test writes a row. The
/// pilot's tank holds blue classified under the hint `ENGAGE` carries; the
/// mind holds blue only as an UNKNOWN row of its own, and its wingman
/// unknown too:
///
/// * `ENGAGE` lands as `Engage { target: None }` — the mind is told nothing
///   of which unknown row is blue;
/// * the row classifies, and the SAME order stays `None` across a landing —
///   **`order_hint_late`, declared**: `rekey_order` runs once, at the order's
///   landing, where T0's `ordered_target` re-resolves its hint every tick (a
///   late re-key needs the hint's object kept on the mind row, a 16c
///   candidate);
/// * a FRESH `ENGAGE` re-keys to the mind's own row;
/// * `SUPPORT` of a held-but-unclassified wingman is `Support { of: None }`,
///   and of the classified wingman its own row;
/// * a self-charge (`SUPPORT` of the mind itself, hinted) resolves through the
///   same closure: `Engage { target: None }` while the hinted row is unknown
///   and the mind's own row once it is classified (17b cut L.2's rule).
#[test]
fn m1b3_an_order_hint_re_keys_only_to_a_classified_row() {
    let blind = |unit: CustodyUnit| CustodyUnit {
        sensor: None,
        ..unit
    };
    let mut rig = Rig::new(arena_of(
        vec![
            blind(tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                Controller::Mind,
            )),
            blind(tank(
                RED2,
                Team::Red,
                centre(6, 10),
                vec![cannon()],
                Controller::Pilot,
            )),
            blind(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            )),
        ],
        &[RED2, BLUE],
    ));
    rig.arena.mind_runner = ScriptedMind::new(&[]);
    // The pilot's row for blue, the hint; the mind's own rows, unknown. Each
    // is (re)sighted NOW before an order, so no row ages out of the store.
    let hint = ContactId(0x77);
    let own_blue = ContactId(0x31);
    let own_wing = ContactId(0x32);
    let hold = |arena: &mut Arena, viewer: ObjId, id: ContactId, target: ObjId, classified| {
        let now = arena.now_ms;
        let state = arena.unit(target).expect("a fielded tank");
        let class = if classified {
            Classification::Classified { team: state.team }
        } else {
            Classification::Unknown
        };
        let plot = SeenPose::of(state);
        arena
            .contacts
            .insert(viewer, id, ShadowContact::sighted(target, class, now, plot));
    };
    // The field as the mind holds it: blue and the wingman, each classified
    // or not.
    let field = |arena: &mut Arena, blue_told: bool, wing_told: bool| {
        hold(arena, RED2, hint, BLUE, true);
        hold(arena, RED, own_blue, BLUE, blue_told);
        hold(arena, RED, own_wing, RED2, wing_told);
    };
    let ordered = |arena: &Arena, objective: Objective, target: Option<ContactId>| {
        let mut told = order(RED_ONE, objective, Some(RED2), 60_000);
        told.target = target;
        intent(arena, RED, IntentKind::Order(Box::new(told)), 1)
    };
    let held = |arena: &Arena| {
        arena
            .minds
            .get(&RED)
            .and_then(|state| state.order.as_ref())
            .map(|order| order.objective)
    };
    let engage = Objective::Engage {
        target_hint: Some(hint),
    };

    // `ENGAGE` before the mind has told its row apart: nothing.
    field(&mut rig.arena, false, false);
    let first = ordered(&rig.arena, engage, None);
    let _ = rig.pulse(&[], &[first]);
    assert_eq!(
        held(&rig.arena),
        Some(MindObjective::Engage { target: None }),
        "an unknown row is not the ordered target"
    );
    // The row classifies; the SAME order is not re-keyed (`order_hint_late`).
    field(&mut rig.arena, true, false);
    while rig.arena.now_ms < think_at(RED, 2) + SUBSTEP {
        let _ = rig.pulse(&[], &[]);
    }
    assert_eq!(
        held(&rig.arena),
        Some(MindObjective::Engage { target: None }),
        "order_hint_late: the standing order keeps its landing's answer"
    );
    assert!(
        rig.arena
            .contacts
            .get(RED, own_blue)
            .is_some_and(|row| matches!(row.class, Classification::Classified { .. })),
        "the precondition: the row was classified across the landings"
    );
    // A fresh order re-keys to the mind's own, now classified, row.
    field(&mut rig.arena, true, false);
    let fresh = ordered(&rig.arena, engage, None);
    let _ = rig.pulse(&[], &[fresh]);
    assert_eq!(
        held(&rig.arena),
        Some(MindObjective::Engage {
            target: Some(own_blue)
        })
    );
    // `SUPPORT` of a held-but-unclassified wingman: no row; classified: its own.
    field(&mut rig.arena, true, false);
    let support = ordered(&rig.arena, Objective::Support { of: RED2 }, None);
    let _ = rig.pulse(&[], &[support]);
    assert_eq!(
        held(&rig.arena),
        Some(MindObjective::Support { of: None }),
        "an unknown charge is no charge"
    );
    field(&mut rig.arena, true, true);
    let support = ordered(&rig.arena, Objective::Support { of: RED2 }, None);
    let _ = rig.pulse(&[], &[support]);
    assert_eq!(
        held(&rig.arena),
        Some(MindObjective::Support { of: Some(own_wing) })
    );
    // A hinted self-charge resolves through the same closure: `None` while the
    // hinted target's own row is unknown, the row once it is classified.
    field(&mut rig.arena, false, true);
    let charge = ordered(&rig.arena, Objective::Support { of: RED }, Some(hint));
    let _ = rig.pulse(&[], &[charge]);
    assert_eq!(
        held(&rig.arena),
        Some(MindObjective::Engage { target: None }),
        "a self-charge fights on doctrine, and an unknown row is no hint"
    );
    field(&mut rig.arena, true, true);
    let charge = ordered(&rig.arena, Objective::Support { of: RED }, Some(hint));
    let _ = rig.pulse(&[], &[charge]);
    assert_eq!(
        held(&rig.arena),
        Some(MindObjective::Engage {
            target: Some(own_blue)
        }),
        "17b cut L.2's self-charge, through the same closure"
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
    let decoded = Snapshot::decode(&snapshot.document, (*rig.arena.definition).clone())
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
///
/// **17b cut M1b.2**: both `FIRE`s cite S1 — the first, armed beside it,
/// fires `by: "mind"`; the slot is let go at think 5, and the second still
/// reads `cycling`, never `unwarranted`: the weapon's own two refusals come
/// before the law.
#[test]
fn m1a5_a_landings_cycling_refusal_is_told_at_the_landing() {
    let gun = WeaponSummary {
        cycle_ms: 3_001,
        ..cannon()
    };
    let mut rig = Rig::new(arena_of(
        vec![
            tank(RED, Team::Red, centre(6, 8), vec![gun], Controller::Mind),
            presenting(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            )),
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
    // 17b cut M1b.2: both `FIRE`s cite S1, armed beside the first; the slot
    // goes at think 5, so the gun's `WeaponReady` one millisecond after
    // landing 7 fires nothing ([`arm_cannon`]'s note).
    let fire = cited_fire;
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
            says(4, vec![arm_cannon(), fire()]),
            says(5, vec![disarm_cannon()]),
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
    // The one shot is the CITED one, not the slot's own at the same
    // millisecond (the landing's one evaluation then finds the gun cycling).
    assert_eq!(
        red_shots(&mut rig),
        vec![(
            think_at(RED, 4) + SUBSTEP,
            u64::from(BLUE.0),
            Some("mind".to_owned()),
            Some(1)
        )],
        "the first citation fired by the mind, citing S1"
    );
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
/// view and answer (an input recorder, not a stand-in: 16b §0.4). **17b cut
/// M2.5**: over any runner ([`Stock::over`]) — the twin of Gate A
/// (`support::TwinRunner`), whose answer IS the native one.
#[derive(Debug)]
struct Stock {
    inner: Arc<dyn MindRunner>,
    kept: Mutex<Vec<Kept>>,
}

impl Stock {
    /// The recorder over the production stock mind the arena flies: the
    /// runner its field was opened with (`default_mind_runner`, T1 —
    /// `NativeStockMind` on the arena's own doctrine). **17b cut M2.6**: it
    /// reads the arena's runner, so a field a fixture builds under the twin
    /// (`support::under_the_twin`, whose field builders call
    /// `support::twin_if_asked` and thereby `support::with_twin` BEFORE this
    /// recorder reads the runner) is recorded over that field's own asserting
    /// twin, whose answer is T1's. Anything else on the arena is a fixture's
    /// own runner and is [`Stock::over`]'s to record, never this one's.
    fn on(arena: &Arena) -> Arc<Stock> {
        let runner = Arc::clone(&arena.mind_runner);
        let name = format!("{runner:?}");
        assert!(
            name.starts_with("NativeStockMind") || name.starts_with("TwinRunner"),
            "Stock::on records the production stock mind (or the twin over it), not {name:.40}"
        );
        Stock::over(runner)
    }

    /// The same recorder over `inner` (17b cut M2.5).
    fn over(inner: Arc<dyn MindRunner>) -> Arc<Stock> {
        Arc::new(Stock {
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
}

impl MindRunner for Stock {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        let result = self.inner.think(job);
        if let Ok(mut kept) = self.kept.lock() {
            kept.push((job.unit, job.t_ms, (**job.view).clone(), result.clone()));
        }
        result
    }
}

/// **The stock mind's emission rules** (`stock_mind.rs`' module head): kinds in
/// `Startup, Condition, Goal, Heading, Speed, Lock, Fire` order, each at most
/// once EXCEPT `Condition` (one per armed weapon, at most
/// [`STOCK_CONDITION_SLOTS`] — 17b cut M1b.2), no `Heading` or `Speed` beside
/// a `Goal(Cell)`, never more than `4 + STOCK_CONDITION_SLOTS`.
fn stock_emission_holds(intents: &[IntentKind]) {
    let rank = |kind: &IntentKind| match kind {
        IntentKind::Startup => 0_u8,
        IntentKind::Condition(_) => 1,
        IntentKind::Goal(_) => 2,
        IntentKind::Heading { .. } => 3,
        IntentKind::Speed { .. } => 4,
        IntentKind::Lock { .. } => 5,
        IntentKind::Fire { .. } => 6,
        other => panic!("the stock mind emitted {other:?}"),
    };
    let ranks: Vec<u8> = intents.iter().map(rank).collect();
    assert!(
        ranks.windows(2).all(|pair| match pair {
            [1, 1] => true,
            [a, b] => a < b,
            _ => false,
        }),
        "{intents:?}"
    );
    assert!(
        ranks.iter().filter(|rank| **rank == 1).count() <= STOCK_CONDITION_SLOTS,
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
    // The module head's maximum: the slots, `Goal(Hold)`, `Heading`, `Lock`,
    // `Fire`.
    assert!(intents.len() <= 4 + STOCK_CONDITION_SLOTS && intents.len() <= MIND_INTENTS_MAX);
}

/// `tests/reflex.rs`' `replay_driven` scene with the two reds flown by
/// `controller`: the armed red at (6, 8), the unarmed one at (6, 10), and the
/// seated, unarmed blue at (9, 8) they can both see — **with a ten-cell sight
/// in place of five**. A view holds a contact's range in the keyframe's 50 m
/// steps (D-UM-4; the divergence `range_round`) and never under one step, and
/// a five-cell sight's `engage_cm` is 35 m: on the reflex's own rig the stock
/// mind would approach for ever and never be inside its band. Ten cells make
/// the band 70 m, past one step, and change nothing else of the scene.
///
/// **17b cut M1b.2**: and the blue [`presenting`] a silhouette. On the
/// default profile it presents nothing, every shot at it is at nought per
/// cent, and T1 — whose slots and citations are held to Line's 10 per cent —
/// would never fire where T0 fires at nought (the divergence `floor_held`);
/// the twin fixture and the first-window oracle compare shots that can land.
/// The profile is non-lethal ([`front_profile`]'s sections without
/// `destroy_unit`), so a hit ends nothing.
fn the_j_field(controller: Controller) -> Arena {
    the_j_field_on(controller, reflex_cfg())
}

/// [`the_j_field`] on the doctrine `reflex` (17b cut M1b.5). **17b cut
/// M2.6**: inside `support::under_the_twin` its minds fly an asserting twin
/// (`support::twin_if_asked`) until a fixture hands the arena another runner.
fn the_j_field_on(controller: Controller, reflex: ReflexConfig) -> Arena {
    let seeing = |mut unit: CustodyUnit| {
        unit.sensor = Some(SensorProfile {
            max_range_cells: 10,
            ..sight()
        });
        unit
    };
    let mut arena = arena_on(
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
        reflex,
    );
    support::twin_if_asked(&mut arena);
    arena
}

/// The (j) field with both reds stock minds, run 30 s under `drive`.
fn stock_minds(drive: Drive) -> (Run, Vec<Kept>) {
    let mut arena = the_j_field(Controller::Mind);
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let run = run(&mut arena, drive, 30_000);
    (run, stock.kept())
}

/// **The two-T1 fixture's `fired` lines as 17b cut M1b.2 records them** —
/// `(t_ms, by)`, every one citing S1 — for 17b cut M1b.3, which makes T1 arm
/// on change and cite `armed.slot` off the view and must leave every one of
/// them where it is (its hashes may move: the armings no longer land every
/// think).
///
/// The first is the fresh-lock think's cited shot at its landing; every
/// later one is the slot's own at the cannon's `WeaponReady`, four seconds
/// (its cycle) apart — the milliseconds the uncited script of 17b cut M1b.1
/// fired at, since the think that lands at a `WeaponReady` planned its shot
/// for it.
const M1B2_TWO_T1_FIRED: &[(i64, &str)] = &[
    (3_732, "mind"),
    (7_732, "cond"),
    (11_732, "cond"),
    (15_732, "cond"),
    (19_732, "cond"),
    (23_732, "cond"),
    (27_732, "cond"),
];

/// The `Arm`s and `Disarm`s of one think's answer.
fn armings(intents: &[IntentKind]) -> Vec<&Arming> {
    intents
        .iter()
        .filter_map(|kind| match kind {
            IntentKind::Condition(arming) => Some(arming),
            _ => None,
        })
        .collect()
}

/// **17b cut M1b.2 on the two-T1 fixture** (D-UM-16, D-UM-17, D-UM-18), as
/// **17b cut M1b.3** made T1 read its own slots off its view (D-UM-4):
///
/// * a think of the armed red answers a `Condition` **only on a difference**
///   between what it wants and what its view's `armed` rows show — an `Arm`
///   only for a weapon of its first [`STOCK_CONDITION_SLOTS`] whose view row
///   is unarmed or armed on other terms (`BestHostile`, Line's 100 per mille,
///   every aspect, no reserve), a `Disarm { None }` only beside an armed view
///   row — and so far fewer thinks answer one than think (M1b.2 answered one
///   every think); the unarmed red never answers a `Condition` (D-UM-48); and
///   every one of them LANDS — no think is handed a refusal of a fire-channel
///   order (the arena's numbering is pinned separately, after every landing,
///   by [`m1b2_two_t1_slot_numbers_are_the_arenas_at_every_landing`]);
/// * a `Fire` is planned only beside a `Lock` of the same think, and cites
///   the slot NUMBER its weapon's view row shows (`armed.slot`), or `i + 1`
///   for a weapon armed by that very think;
/// * every `fired` line of the armed red is cited — `slot: 1` — and is
///   `by: "mind"` exactly when it is the landing of a think that planned a
///   fresh `Lock` and a `Fire`, `by: "cond"` otherwise (the slot's own shot
///   at a `WeaponReady`, a pass or a landing);
/// * the `fired` milliseconds are [`M1B2_TWO_T1_FIRED`], recorded by 17b cut
///   M1b.2 and unmoved by the on-change arming.
fn two_t1_slots_and_citations(run: &Run, kept: &[Kept]) {
    let mut thinks = 0_usize;
    let mut answered = 0_usize;
    let mut cited_off_view = 0_usize;
    let mut shown = 0_usize;
    // The landing millisecond of the armed red's first arming.
    let armed_from = kept
        .iter()
        .find(|(unit, _, _, result)| *unit == RED && !armings(&result.intents).is_empty())
        .map(|(_, t_ms, ..)| t_ms + SUBSTEP);
    for (unit, t_ms, view, result) in kept {
        // Every arming, disarming and citation LANDED: no think was handed a
        // refusal of its fire channel.
        assert!(
            !view.notices.iter().any(|notice| matches!(
                notice,
                FrameNotice::Refused { order, .. } if order == "fire"
            )),
            "a refused fire-channel order in the view at {t_ms}: {:?}",
            view.notices
        );
        let rows = armings(&result.intents);
        if *unit == RED2 {
            assert!(rows.is_empty(), "the unarmed red armed at {t_ms}: {rows:?}");
            continue;
        }
        thinks += 1;
        if !rows.is_empty() {
            answered += 1;
        }
        // 17b cut M1b.3 (D-UM-4): from the first arming's landing on, the
        // view shows the slot — S1, the best hostile — as a pilot's frame
        // does, and that is what the think reads.
        if armed_from.is_some_and(|from| *t_ms >= from) {
            let armed = view.weapons.first().and_then(|row| row.armed.as_ref());
            assert_eq!(
                armed.map(|armed| (armed.slot, armed.target)),
                Some((1, SlotTarget::BestHostile)),
                "the view's armed row at {t_ms}"
            );
            shown += 1;
        }
        let arms: Vec<&FireCondition> = rows
            .iter()
            .filter_map(|arming| match arming {
                Arming::Arm(cond) => Some(cond),
                Arming::Disarm { .. } => None,
            })
            .collect();
        if arms.is_empty() && !rows.is_empty() {
            assert_eq!(
                rows,
                vec![&Arming::Disarm { weapon: None }],
                "one disarm at {t_ms}"
            );
            assert!(
                view.weapons.iter().any(|row| row.armed.is_some()),
                "a disarm only beside an armed view row at {t_ms}"
            );
        } else if !arms.is_empty() {
            assert_eq!(arms.len(), rows.len(), "no disarm beside an arm at {t_ms}");
            let slotted: Vec<&SmolStr> = view
                .weapons
                .iter()
                .take(STOCK_CONDITION_SLOTS)
                .map(|weapon| &weapon.id)
                .collect();
            for cond in arms {
                assert!(slotted.contains(&&cond.weapon), "{} at {t_ms}", cond.weapon);
                assert_eq!(
                    (
                        cond.target,
                        cond.min_permille,
                        cond.aspects,
                        cond.min_rounds_left
                    ),
                    (SlotTarget::BestHostile, 100, ASPECTS_ANY, 0),
                    "Line's discipline at {t_ms}"
                );
                // 17b cut M1b.3: only a row the view shows otherwise.
                let shown = view
                    .weapons
                    .iter()
                    .find(|row| row.id == cond.weapon)
                    .and_then(|row| row.armed.clone());
                assert!(
                    shown
                        .as_ref()
                        .is_none_or(|armed| *armed != FrameArming::of(armed.slot, cond)),
                    "an Arm of the terms the view already shows at {t_ms}: {shown:?}"
                );
            }
        }
        let locks = result
            .intents
            .iter()
            .any(|kind| matches!(kind, IntentKind::Lock { .. }));
        for kind in &result.intents {
            if let IntentKind::Fire { weapon, slot } = kind {
                assert!(locks, "a Fire with no fresh Lock at {t_ms}");
                let at = view.weapons.iter().position(|row| &row.id == weapon);
                let shown = view
                    .weapons
                    .iter()
                    .find(|row| &row.id == weapon)
                    .and_then(|row| row.armed.as_ref())
                    .map(|armed| armed.slot);
                if shown.is_some() {
                    cited_off_view += 1;
                }
                assert_eq!(
                    *slot,
                    shown.or_else(|| at.and_then(|at| u8::try_from(at + 1).ok())),
                    "the view's slot number, else i + 1, at {t_ms}"
                );
            }
        }
    }
    // 17b cut M1b.3: an arming answered only on a difference — far fewer
    // thinks answer one than think.
    assert!(answered >= 1, "the armed red armed: {answered} of {thinks}");
    assert!(shown > 0, "a think read its own armed slot off its view");
    assert!(
        answered * 4 <= thinks,
        "a Condition on {answered} of {thinks} thinks: not only on a difference"
    );
    eprintln!(
        "m1b3 two-T1: {answered} of {thinks} armed-red thinks answered a Condition; \
         {shown} read the armed slot off the view; \
         {cited_off_view} Fire(s) cited off the view's armed row"
    );
    let fired: Vec<(i64, String)> = run
        .events
        .iter()
        .filter_map(|(t_ms, json, _)| {
            let value: serde_json::Value = serde_json::from_str(json).ok()?;
            (value.get("kind")? == "fired" && value.get("unit")? == u64::from(RED.0)).then(|| {
                assert_eq!(
                    value.get("slot").and_then(serde_json::Value::as_u64),
                    Some(1),
                    "an uncited T1 shot at {t_ms}: {json}"
                );
                let by = value
                    .get("by")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                (*t_ms, by)
            })
        })
        .collect();
    for (t_ms, by) in &fired {
        let fresh = kept.iter().any(|(unit, think_ms, _, result)| {
            *unit == RED
                && think_ms + SUBSTEP == *t_ms
                && result
                    .intents
                    .iter()
                    .any(|kind| matches!(kind, IntentKind::Fire { .. }))
        });
        let want = if fresh { "mind" } else { "cond" };
        assert_eq!(by, want, "the shot at {t_ms}");
    }
    assert!(
        fired.iter().any(|(_, by)| by == "mind"),
        "a cited fresh-lock shot: {fired:?}"
    );
    assert!(
        fired.iter().any(|(_, by)| by == "cond"),
        "the slot's own shots: {fired:?}"
    );
    let recorded: Vec<(i64, String)> = M1B2_TWO_T1_FIRED
        .iter()
        .map(|(t_ms, by)| (*t_ms, (*by).to_owned()))
        .collect();
    assert_eq!(fired, recorded, "the recorded fired milliseconds");
}

/// **The armed red's slots ARE its last landed think's armings, at every
/// landing** (17b cut M1b.2: `n = i + 1` equals the arena's stable
/// numbering by construction): the (j) field one substep a pulse for 30 s,
/// and after every pulse `arena.conditions.slots(RED)` read against the
/// armings of the last think of red's whose landing has run **and that
/// answered a `Condition`** (17b cut M1b.3: a think whose view already
/// showed what it wanted answers none, and its slots are the earlier
/// think's) — slot `i + 1` holds the `i`-th `Arm`'s terms, or every slot is
/// empty after a `Disarm` — and the unarmed red holds no slot.
#[test]
fn m1b2_two_t1_slot_numbers_are_the_arenas_at_every_landing() {
    let mut arena = the_j_field(Controller::Mind);
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut rig = Rig::new(arena);
    let mut checked = (0_usize, 0_usize);
    while rig.arena.now_ms < 30_000 {
        let _ = rig.pulse(&[], &[]);
        let now = rig.arena.now_ms;
        assert!(rig.arena.conditions.is_empty_for(RED2), "at {now}");
        let kept = stock.kept();
        let Some((_, think_ms, _, result)) =
            kept.iter().rev().find(|(unit, think_ms, _, result)| {
                *unit == RED && think_ms + SUBSTEP <= now && !armings(&result.intents).is_empty()
            })
        else {
            continue;
        };
        let slots = rig.arena.conditions.slots(RED).to_vec();
        let arms: Vec<Option<FireCondition>> = armings(&result.intents)
            .into_iter()
            .filter_map(|arming| match arming {
                Arming::Arm(cond) => Some(Some(cond.clone())),
                Arming::Disarm { .. } => None,
            })
            .collect();
        if arms.is_empty() {
            assert!(
                slots.iter().all(Option::is_none),
                "disarmed after the think at {think_ms}: {slots:?}"
            );
            checked.1 += 1;
        } else {
            assert_eq!(
                slots.get(..arms.len()),
                Some(arms.as_slice()),
                "slot i + 1 is the think at {think_ms}'s i-th arming"
            );
            assert!(slots.iter().skip(arms.len()).all(Option::is_none));
            checked.0 += 1;
        }
    }
    // 17b cut M1b.3: on change, a `Disarm` is answered only beside an armed
    // view row, and this fixture's red, once armed, never stops wanting its
    // slot — so only the armed half runs here (the disarm on change is pinned
    // in-source, `stock_mind.rs`' `hold_fire` and `Starting` arms over an
    // armed view, and LANDED at three cadences by `tests/conditions.rs`'
    // `m1b3_t1_disarms_once_on_change_when_a_hold_fire_lands_on_its_armed_slot`).
    // M1b.2 disarmed on every think before the first arming.
    assert!(
        checked.0 > 0,
        "the armed half ran: {checked:?} (armed, disarmed)"
    );
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
///
/// **17b cut M1b.2, as 17b cut M1b.3 left it** ([`two_t1_slots_and_citations`]):
/// the armed red answers a `Condition` only where its view's `armed` rows
/// differ from what it wants (an `Arm` per such gun, or a `Disarm` beside an
/// armed row) and the unarmed red never; a `Fire` only beside a fresh `Lock`,
/// citing the slot its view shows, else `i + 1` for a gun armed by that very
/// think; every `fired` line cited, `by: "mind"` on a fresh-lock landing and
/// `by: "cond"` otherwise; the `fired` milliseconds recorded
/// ([`M1B2_TWO_T1_FIRED`]). (This fixture's one cited shot is its arming
/// think's, so it cites `i + 1`; the citation off a view whose number differs
/// from `i + 1` is `tests/conditions.rs`'
/// `m1b3_t1_cites_the_slot_its_view_shows_after_its_s1_gun_is_lost`.)
/// The emission rank with `Condition` is [`stock_emission_holds`]'.
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
    // **17b cut M1b.2**: the slots and the citations.
    two_t1_slots_and_citations(&first, &kept);
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
    unarmed_run_on(cadence_ms, None)
}

/// [`unarmed_run`] with the stock recorder over `runner` (17b cut M2.5: the
/// twin), the native stock mind for `None`.
fn unarmed_run_on(cadence_ms: i64, runner: Option<Arc<dyn MindRunner>>) -> (Run, Vec<Kept>, Arena) {
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
    let stock = match runner {
        Some(runner) => Stock::over(runner),
        None => Stock::on(&arena),
    };
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
    // 17b cut M1b.2: nor a `Condition` — an unarmed hull has no slot to
    // answer (D-UM-48) — and the arena holds none for it.
    assert!(arena.conditions.is_empty_for(RED));
    for (_, _, view, result) in &kept {
        stock_emission_holds(&result.intents);
        assert!(
            !result.intents.iter().any(|kind| matches!(
                kind,
                IntentKind::Lock { .. } | IntentKind::Fire { .. } | IntentKind::Condition(_)
            )),
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

/// **Gate A on the unarmed fixture** (17b cut M2.5; doc 17 §17 "Gate A",
/// §22's italics; D-UM-48): [`unarmed_run`] — the weaponless stock mind that
/// starts up, walks, holds under `HOLD` and withdraws under `WITHDRAW` with a
/// hostile in its sight — flown by `support::TwinRunner` in its asserting
/// mode, so EVERY think is T1 and T2 (`content/minds/standard.rhai`) on the
/// same job, the Rhai fault checked first, then the intents, then the memory
/// bytes. The twin answers T1's answer, so at 5,000, 1,000 and 500 ms the
/// run is the native run's: one event list, one hash sequence, one sequence
/// of thinks — and the twin compared every one of them. The ops and fees of
/// the Rhai thinks are printed against class I's 1,500 and Class 0's 400.
#[test]
fn m2_the_unarmed_stock_mind_in_rhai_is_the_native_one_at_three_cadences() {
    let (native, native_kept, _) = unarmed_run(5_000);
    let cfg = ArenaConfig {
        reflex: reflex_cfg(),
        ..ArenaConfig::default()
    };
    let mut budget: Vec<u64> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let twin = Arc::new(support::TwinRunner::new(&cfg));
        let (run, kept, _) = unarmed_run_on(cadence, Some(twin.clone()));
        assert_eq!(run.events, native.events, "the event list at {cadence} ms");
        assert_eq!(
            run.hashes, native.hashes,
            "the hash sequence at {cadence} ms"
        );
        assert_eq!(kept, native_kept, "a think at {cadence} ms");
        let thoughts = twin.thoughts();
        assert_eq!(thoughts.len(), kept.len(), "the twin saw every think");
        assert_eq!(twin.divergences(), (0, 0));
        assert!(thoughts.iter().all(|thought| !thought.armed));
        budget.extend(thoughts.iter().map(|thought| thought.ops + thought.fee));
    }
    let (p50, p99, max) = support::spread(&budget);
    println!(
        "m2_unarmed_twin: thinks={} ops_plus_fee_p50={p50} p99={p99} max={max} (class I 1500, \
         Class 0 400)",
        budget.len()
    );
    assert!(max <= 1_200, "a think of {max} ops: past 1,200 is a STOP");
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
        let journal = events(&mut arena, &mut bytes);
        let actions: Vec<(i64, String, u32)> = journal
            .iter()
            .filter_map(|event| {
                let json = serde_json::to_string(&event.kind).unwrap_or_default();
                let (word, unit) = kind_and_unit(&json);
                let unit = unit.filter(|unit| [RED.0, RED2.0].contains(unit))?;
                matches!(word.as_str(), "startup" | "locked" | "fired" | "ordered")
                    .then_some((event.t_ms, word, unit))
            })
            .collect();
        // 17b cut M1b.2: each `fired` line's attribution, `(t_ms, by, slot)`.
        let shots: Vec<(i64, Option<String>, Option<u64>)> = journal
            .iter()
            .filter_map(|event| {
                let value = serde_json::to_value(&event.kind).ok()?;
                (value.get("kind")? == "fired").then(|| {
                    (
                        event.t_ms,
                        value
                            .get("by")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned),
                        value.get("slot").and_then(serde_json::Value::as_u64),
                    )
                })
            })
            .collect();
        (actions, helm, shots)
    };
    let (t0, t0_helm, t0_shots) = lines(Controller::Reflex);
    let (t1, t1_helm, t1_shots) = lines(Controller::Mind);
    // T0's shots are a machine's: never attributed (`FiredBy::Machine`, never
    // gated). T1's are each cited: a fresh lock's shot `by: "mind"` at the
    // landing one substep after a lattice think, or the slot's own `by:
    // "cond"` — at a `WeaponReady`, a pass or a landing — both naming S1.
    assert!(
        t0_shots
            .iter()
            .all(|(_, by, slot)| by.is_none() && slot.is_none()),
        "{t0_shots:?}"
    );
    for (t_ms, by, slot) in &t1_shots {
        assert_eq!(*slot, Some(1), "an uncited T1 shot at {t_ms}");
        match by.as_deref() {
            Some("mind") => assert_eq!(
                (t_ms - SUBSTEP - stagger_ms(SEED, RED, STOCK_THINK_MS)).rem_euclid(STOCK_THINK_MS),
                0,
                "a cited shot off a landing at {t_ms}"
            ),
            Some("cond") => {}
            other => panic!("a T1 shot by {other:?} at {t_ms}"),
        }
    }
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
    // 17b cut M1b.2 declared this comparison widened for the `fired` lines
    // (a slot's shot at a `WeaponReady` or a pass where T0 fires on its
    // tick); on this window the one shot is the fresh lock's cited one, at
    // its landing, and the strict form still holds — so it is kept.
    assert_eq!(t1, shifted, "T1 is T0 one substep later");
    assert!(
        t1.iter().any(|(_, word, _)| word == "fired"),
        "the window holds a shot"
    );
    assert!(
        t1_shots
            .iter()
            .any(|(_, by, _)| by.as_deref() == Some("mind")),
        "the window's shot is the fresh lock's cited one: {t1_shots:?}"
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
/// no sensor and no weapon, and the seated blue at (9, 8). **17b cut M2.6**:
/// inside `support::under_the_twin` its minds fly an asserting twin.
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
    let mut arena = arena_of(
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
    );
    support::twin_if_asked(&mut arena);
    arena
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
/// 130 m off — in sight, and outside every band the red mind has. **17b cut
/// M2.6**: inside `support::under_the_twin` its mind flies an asserting twin.
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
    let mut arena = arena_of(
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
            // 17b cut M1b.2: a silhouette, so the mind's slot has odds to
            // fire on ([`presenting`]).
            presenting(seeing(tank(
                BLUE,
                Team::Blue,
                centre(14, 8),
                Vec::new(),
                Controller::Pilot,
            ))),
        ],
        &[RED2, BLUE],
    );
    support::twin_if_asked(&mut arena);
    arena
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
    // 17b cut M1b.3: the mind's rows told apart, which the re-key now asks.
    let _ = rig.until(10_000, |arena| {
        told_apart(arena, RED, BLUE)
            && arena.contacts.id_of(RED2, BLUE).is_some()
            && told_apart(arena, RED, RED2)
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

// ---------------------------------------------------------------------------
// 17b cut M1b.5: the card T1 reads (doc 17 §11, §22; D-UM-43, D-UM-44; 17b's
// Q-M6)
// ---------------------------------------------------------------------------

/// One plan, one armour cell and one internals cell (`tests/reflex.rs`'
/// `set_health`, copied: the eighteen-point tank).
fn set_health(arena: &mut Arena, unit: ObjId, armor: i64, internals: i64) {
    let ops = vec![
        UnitMutationOp::SetCell {
            addr: CellAddr::Facet {
                section: SectionId::new("hull").expect("a section"),
                facet: FacetId::new("front").expect("a facet"),
                cell: mush_tactics::CellId::new("armor").expect("a cell"),
            },
            value: armor,
        },
        UnitMutationOp::SetCell {
            addr: CellAddr::Section {
                section: SectionId::new("hull").expect("a section"),
                cell: mush_tactics::CellId::new("internals").expect("a cell"),
            },
            value: internals,
        },
    ];
    let plan =
        UnitMutationPlan::new(ops, None, Some(SmolStr::new("test:hurt"))).expect("a two-op plan");
    let hurt = {
        let state = arena.unit(unit).expect("the tank");
        mush_tactics::apply(&state.unit, &plan, 0)
            .expect("the plan applies")
            .0
    };
    if let Some(state) = arena.unit_mut(unit) {
        state.unit = hurt;
    }
}

/// **A non-default doctrine** (17b cut M1b.5): `reflex_cfg`'s cadence with
/// every term the card maps moved off the default.
fn m1b5_doctrine() -> ReflexConfig {
    ReflexConfig {
        decide_ms: 500,
        repath_ms: 0,
        withdraw_permille: 700,
        stand_min_permille: 300,
        engage_band_permille: 850,
        rally_ms: 7_000,
        search_ms: 30_000,
        hunt_after_ms: 45_000,
        weight_range: 70,
        weight_facet: 55,
        weight_health: 25,
        ..ReflexConfig::default()
    }
}

/// **The non-default doctrine's withdrawal, as 17b cut M1b.4 flew it**: the
/// lattice millisecond of the first think of [`RED`] whose memory reads
/// `Withdraw` on [`the_j_field_on`] over [`m1b5_doctrine`] with red hurt to
/// 666 per mille at 10,000 ([`card_run`]'s [`CardBeat::Hurt`]), recorded at
/// `69880812` before the overlay edit.
const M1B5_WITHDRAWS_AT: i64 = 10_482;

/// What 17b cut M1b.5's driver does at a beat's millisecond, before the pulse
/// that starts there.
#[derive(Clone, Debug)]
enum CardBeat {
    /// Take `unit` down to `armor` and `internals` of its eighteen points.
    Hurt(ObjId, i64, i64),
    /// Stage a card change for `unit` (`Arena::stage_card`): the arena takes
    /// it at that pulse's start.
    Stage(ObjId, CardChange),
}

/// **17b cut M1b.5's driver**: `arena`'s stock minds recorded ([`Stock`]),
/// stepped at `cadence_ms` to `until_ms`, each beat applied once when the
/// arena's clock first reaches its millisecond (every beat is a multiple of
/// 5,000, so it lands between the same two pulses' arena milliseconds at
/// 5,000, 1,000 and 500 ms). Answers the run, the thinks and the arena.
fn card_run(
    mut arena: Arena,
    cadence_ms: i64,
    until_ms: i64,
    beats: &[(i64, CardBeat)],
) -> (Run, Vec<Kept>, Arena) {
    let stock = Stock::on(&arena);
    arena.mind_runner = stock.clone();
    let mut next = 0_usize;
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut counters = ReflexCounters::default();
    let mut pulses = 0_u32;
    let mut wall = 0_i64;
    loop {
        while let Some((at, beat)) = beats.get(next) {
            if *at > arena.now_ms {
                break;
            }
            match beat {
                CardBeat::Hurt(unit, armor, internals) => {
                    set_health(&mut arena, *unit, *armor, *internals);
                }
                CardBeat::Stage(unit, change) => arena.stage_card(*unit, change.clone()),
            }
            next += 1;
        }
        let out = arena.step(wall, &[], &[]);
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
        stock.kept(),
        arena,
    )
}

/// The stock state a think left `unit`'s memory in.
fn state_after(result: &ThinkResult) -> ReflexState {
    tacsim::arena::stock_mind::StockMemory::from(&tacsim::arena::mind::decode_memory(
        &result.memory,
    ))
    .state
}

/// The lattice milliseconds of `unit`'s thinks that left it withdrawing.
fn withdrawn_thinks(kept: &[Kept], unit: ObjId) -> Vec<i64> {
    kept.iter()
        .filter(|(on, _, _, result)| *on == unit && state_after(result) == ReflexState::Withdraw)
        .map(|(_, t_ms, ..)| *t_ms)
        .collect()
}

/// **T1 reads its card through the overlay, and Line on its doctrine is
/// today's machine** (17b cut M1b.5; doc 17 §11, D-UM-43; 17b's Q-M6 — the
/// anchor's three proofs):
///
/// 1. `line(cfg).as_reflex(cfg) == cfg` — `card.rs`' in-source
///    `m1b5_line_overlays_to_its_own_doctrine_and_each_preset_to_its_terms`,
///    and here for the two doctrines this file flies;
/// 2. **the recorded run**: the two-T1 fixture's event list and every
///    think's answer, recorded at 17b cut M1b.4's HEAD before
///    `NativeStockMind::think` built `job.card.as_reflex(&self.cfg)`
///    ([`M1B5_TWO_T1_EVENTS`], [`M1B5_TWO_T1_INTENTS`]), equal after it. The
///    run's line hashes and substep hashes were recorded too and asserted
///    equal after the overlay edit; they are **RETIRED** since the memory
///    rename of the same sub-cut (`StockMemory.ignore_order: bool` became
///    `ignore_until_ms: Option<i64>`, the lead's ruling on M1a.6's residual
///    (10)): every think's memory now encodes `"ignore_until_ms":null` where
///    it encoded `"ignore_order":false`, and the memory's bytes are folded
///    into the state hash (item 3c), so every hash from the first think on
///    moved while nothing a decision reads did — the events and the answers,
///    which ARE what the machine decided, stay pinned;
/// 3. the twin lane's figures equal 17b cut M1b.4's (`tests/twin.rs`, in
///    release; the report has both).
///
/// And on a **non-default** doctrine ([`m1b5_doctrine`]: every mapped term
/// moved), every mind's row holds `line` of THAT doctrine, and T1 hurt to 666
/// per mille at 10,000 first thinks itself withdrawn at
/// [`M1B5_WITHDRAWS_AT`] — where 17b cut M1b.4's T1 did, under
/// `withdraw_permille` 700 (the default's 350 would have stood).
#[test]
fn m1b5_t1_reads_its_card_through_the_overlay_and_line_is_unchanged() {
    for cfg in [reflex_cfg(), m1b5_doctrine()] {
        assert_eq!(tacsim::arena::line(&cfg).as_reflex(&cfg), cfg);
    }
    let (run, kept) = stock_minds(Drive::Every(5_000));
    let events: Vec<(i64, &str)> = run
        .events
        .iter()
        .map(|(t_ms, json, _)| (*t_ms, json.as_str()))
        .collect();
    assert_eq!(events, M1B5_TWO_T1_EVENTS, "the recorded event list");
    let answers: Vec<(u32, i64, String)> = kept
        .iter()
        .map(|(unit, t_ms, _, result)| {
            (
                unit.0,
                *t_ms,
                serde_json::to_string(&result.intents).unwrap_or_default(),
            )
        })
        .collect();
    let recorded: Vec<(u32, i64, String)> = M1B5_TWO_T1_INTENTS
        .iter()
        .map(|(unit, t_ms, json)| (*unit, *t_ms, (*json).to_owned()))
        .collect();
    assert_eq!(answers, recorded, "the recorded answers");
    // The recorded line hashes and hash sequence: retired by the memory
    // rename (this test's doc).

    // The non-default doctrine: the Line card at its values, and the
    // withdrawal where M1b.4's T1 withdrew.
    let doctrine = m1b5_doctrine();
    let arena = the_j_field_on(Controller::Mind, doctrine.clone());
    for (unit, state) in &arena.minds {
        assert_eq!(state.card, tacsim::arena::line(&doctrine), "#{}", unit.0);
    }
    let (_, kept, _) = card_run(arena, 5_000, 30_000, &[(10_000, CardBeat::Hurt(RED, 6, 6))]);
    assert_eq!(
        withdrawn_thinks(&kept, RED).first(),
        Some(&M1B5_WITHDRAWS_AT),
        "the withdrawal on the non-default doctrine"
    );
    // On the field's own doctrine (withdraw 350) the same hurt stands.
    let (_, kept, _) = card_run(
        the_j_field(Controller::Mind),
        5_000,
        30_000,
        &[(10_000, CardBeat::Hurt(RED, 6, 6))],
    );
    assert_eq!(withdrawn_thinks(&kept, RED), Vec::<i64>::new());
}

/// Every `mind_card` and `mind_card_refused` line of a run, as `(t_ms, the
/// line's object)`.
fn card_lines(run: &Run) -> Vec<(i64, serde_json::Value)> {
    run.events
        .iter()
        .filter_map(|(t_ms, json, _)| {
            let value: serde_json::Value = serde_json::from_str(json).ok()?;
            matches!(
                value.get("kind").and_then(serde_json::Value::as_str),
                Some("mind_card" | "mind_card_refused")
            )
            .then_some((*t_ms, value))
        })
        .collect()
}

/// The `min_permille` of every `Arm` `unit` answered after `after_ms`, by
/// think.
fn arms_after(kept: &[Kept], unit: ObjId, after_ms: i64) -> Vec<(i64, Vec<u16>)> {
    kept.iter()
        .filter(|(on, t_ms, ..)| *on == unit && *t_ms > after_ms)
        .map(|(_, t_ms, _, result)| {
            (
                *t_ms,
                armings(&result.intents)
                    .into_iter()
                    .filter_map(|arming| match arming {
                        Arming::Arm(cond) => Some(cond.min_permille),
                        Arming::Disarm { .. } => None,
                    })
                    .collect(),
            )
        })
        .collect()
}

/// A preset by name over the (j) field's own doctrine.
fn preset_on_field(name: &str) -> tacsim::arena::StockCard {
    tacsim::arena::preset(name, &reflex_cfg()).expect("a preset")
}

/// **A card lands at the pulse's start, and the next think reads it** (17b
/// cut M1b.5; doc 17 §11, §22's M1b, D-UM-17 as amended, D-UM-43; 16b §11's
/// mandatory cross-tempo property):
///
/// * `Rearguard` staged for the armed red at the 5,000 ms beat lands at
///   5,000 — the start of the pulse after the beat — with ONE `mind_card`
///   line carrying the whole card (its name, the three sliders it moved, the
///   card), at 5,000, 1,000 and 500 ms: one event list, one hash sequence,
///   one sequence of thinks;
/// * red's next think re-arms its slot at Rearguard's `min_odds` 50 (500 per
///   mille: the view's armed row held Line's 100, so the on-change arming of
///   17b cut M1b.3 answers it at once);
/// * red, hurt to 666 per mille at 10,000, WITHDRAWS at `break_off` 70 where
///   Line (35) stood (`m1b5_t1_reads_its_card_through_the_overlay_and_line_is_unchanged`'s
///   last run) — the overlay is read;
///
/// and at one cadence: a change staged for a reflex hull journals
/// `mind_card_refused` `not_mind` and changes nothing; a `band_pct 72` the
/// arena refuses journals the `CardError`'s own sentence and keeps the card;
/// **an arena whose `odds_floor_permille` stands ABOVE the card's
/// `min_odds`** — set by hand past 17b cut M1b's bound, as a resumed arena of
/// a later milestone could carry it — refuses `Rearguard` (50 per cent)
/// `unlawful` and keeps the old card, and takes `Sniper` (65); two changes
/// staged in one pulse land in arrival order (`Sniper` then `min_odds 70` →
/// `Sniper, edited (min_odds 70)`).
#[test]
fn m1b5_a_card_lands_at_the_pulses_start_and_the_next_think_reads_it() {
    let rearguard = || {
        vec![
            (
                5_000_i64,
                CardBeat::Stage(RED, CardChange::Preset(SmolStr::new("rearguard"))),
            ),
            (10_000, CardBeat::Hurt(RED, 6, 6)),
        ]
    };
    let (first, kept, arena) = card_run(the_j_field(Controller::Mind), 5_000, 30_000, &rearguard());
    let lines = card_lines(&first);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let (at, line) = &lines[0];
    assert_eq!(*at, 5_000, "at the start of the pulse after the beat");
    assert_eq!(line["kind"], "mind_card");
    assert_eq!(line["unit"], RED.0);
    assert_eq!(line["name"], "Rearguard");
    assert_eq!(
        line["changed"],
        serde_json::json!(["min_odds", "break_off_pct", "rally_s"])
    );
    let card: tacsim::arena::StockCard =
        serde_json::from_value(line["card"].clone()).expect("the whole card");
    assert_eq!(card, preset_on_field("rearguard"));
    assert_eq!(arena.minds.get(&RED).map(|state| &state.card), Some(&card));
    // The next think re-arms at Rearguard's 50 per cent.
    let next = arms_after(&kept, RED, 5_000);
    assert_eq!(
        next.first().map(|(_, arms)| arms.clone()),
        Some(vec![500]),
        "red's next think re-arms at min_odds 50: {next:?}"
    );
    // Withdraws at break_off 70 where Line stood.
    let withdrawn = withdrawn_thinks(&kept, RED);
    assert!(
        withdrawn.first().is_some_and(|t_ms| *t_ms > 10_000),
        "red withdraws once hurt: {withdrawn:?}"
    );
    for cadence in [1_000, 500] {
        let (other, other_kept, _) =
            card_run(the_j_field(Controller::Mind), cadence, 30_000, &rearguard());
        assert_eq!(other.events, first.events, "the event list at {cadence}");
        assert_eq!(other.hashes, first.hashes, "the hash sequence at {cadence}");
        assert_eq!(other_kept, kept, "a think diverged at {cadence}");
    }

    // A reflex hull: refused `not_mind`, nothing changed.
    let (run, _, arena) = card_run(
        the_j_field(Controller::Reflex),
        5_000,
        10_000,
        &[(
            5_000,
            CardBeat::Stage(RED, CardChange::Preset(SmolStr::new("Sniper"))),
        )],
    );
    let lines = card_lines(&run);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(
        (lines[0].1["kind"].clone(), lines[0].1["reason"].clone()),
        (
            serde_json::json!("mind_card_refused"),
            serde_json::json!("not_mind")
        )
    );
    assert!(arena.minds.is_empty());

    // A value the arena refuses: the `CardError`'s own sentence.
    let (run, _, arena) = card_run(
        the_j_field(Controller::Mind),
        5_000,
        10_000,
        &[(
            5_000,
            CardBeat::Stage(
                RED,
                CardChange::Set {
                    slider: SmolStr::new("band_pct"),
                    value: SmolStr::new("72"),
                },
            ),
        )],
    );
    let lines = card_lines(&run);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].1["kind"], "mind_card_refused");
    assert_eq!(lines[0].1["reason"], "band_pct is 30 to 100 in steps of 5");
    assert_eq!(
        arena.minds.get(&RED).map(|state| state.card.clone()),
        Some(preset_on_field("line"))
    );

    // The arena's own floor, past the bound by hand.
    let mut floored = the_j_field(Controller::Mind);
    floored.odds_floor_permille = 600;
    // The refused Rearguard leaves the row's OLD card in place: read the row
    // after the refusal and before anything else lands.
    let (run, _, arena) = card_run(
        floored.clone(),
        5_000,
        10_000,
        &[(
            5_000,
            CardBeat::Stage(RED, CardChange::Preset(SmolStr::new("Rearguard"))),
        )],
    );
    let lines = card_lines(&run);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].1["reason"], "unlawful");
    assert_eq!(
        arena.minds.get(&RED).map(|state| state.card.clone()),
        Some(preset_on_field("line")),
        "a card refused unlawful is never written to the row"
    );
    let (run, _, arena) = card_run(
        floored,
        5_000,
        15_000,
        &[
            (
                5_000,
                CardBeat::Stage(RED, CardChange::Preset(SmolStr::new("Rearguard"))),
            ),
            (
                10_000,
                CardBeat::Stage(RED, CardChange::Preset(SmolStr::new("Sniper"))),
            ),
        ],
    );
    let lines = card_lines(&run);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(
        (
            lines[0].0,
            lines[0].1["kind"].clone(),
            lines[0].1["reason"].clone()
        ),
        (
            5_000,
            serde_json::json!("mind_card_refused"),
            serde_json::json!("unlawful")
        )
    );
    assert_eq!(
        (
            lines[1].0,
            lines[1].1["kind"].clone(),
            lines[1].1["name"].clone()
        ),
        (
            10_000,
            serde_json::json!("mind_card"),
            serde_json::json!("Sniper")
        )
    );
    assert_eq!(
        lines[1].1["changed"],
        serde_json::json!(["band_pct", "min_odds", "reserve_pct"]),
        "Sniper is read against Line, the card the refusal kept — not Rearguard"
    );
    assert_eq!(
        arena.minds.get(&RED).map(|state| state.card.clone()),
        Some(preset_on_field("sniper"))
    );

    // Two changes in one pulse, in arrival order.
    let (run, _, arena) = card_run(
        the_j_field(Controller::Mind),
        5_000,
        10_000,
        &[
            (
                5_000,
                CardBeat::Stage(RED, CardChange::Preset(SmolStr::new("sniper"))),
            ),
            (
                5_000,
                CardBeat::Stage(
                    RED,
                    CardChange::Set {
                        slider: SmolStr::new("min_odds"),
                        value: SmolStr::new("70"),
                    },
                ),
            ),
        ],
    );
    let lines = card_lines(&run);
    let names: Vec<(i64, serde_json::Value, serde_json::Value)> = lines
        .iter()
        .map(|(t_ms, line)| (*t_ms, line["name"].clone(), line["changed"].clone()))
        .collect();
    assert_eq!(
        names,
        vec![
            (
                5_000,
                serde_json::json!("Sniper"),
                serde_json::json!(["band_pct", "min_odds", "reserve_pct"])
            ),
            (
                5_000,
                serde_json::json!("Sniper, edited (min_odds 70)"),
                serde_json::json!(["min_odds"])
            ),
        ]
    );
    assert_eq!(
        arena
            .minds
            .get(&RED)
            .map(|state| state.card.min_odds_permille),
        Some(700)
    );
}

/// **A preset on an unarmed hull: the fire section inert, no slot, nothing
/// refused** (17b cut M1b.5; doc 17 §22's M1b italic line read as "applying
/// the card", §11: *a card section that drives a station the hull lacks is
/// shown inert with the reason … and never refuses*; D-UM-48). The (j) field's
/// unarmed red ([`RED2`]) gets `Sniper` at the 5,000 ms beat, and so does the
/// armed red beside it:
///
/// * the unarmed hull's card lands — one `mind_card` line, named `Sniper` —
///   and nothing refuses (no `mind_card_refused`, no refused notice in any of
///   its views);
/// * its read-back's fire-discipline line is the inert sentence;
/// * it never answers a `Condition`, and the condition table never holds a
///   row for it;
/// * it drives and holds as before: every answer it gives equals the answer of
///   the same field with no card change (Sniper's mapped terms that differ
///   from Line's — the band and the fire discipline — drive nothing on a hull
///   with no gun);
/// * the armed red re-arms its slot at Sniper's `min_odds` 65 at its next
///   think.
#[test]
fn m1b5_an_unarmed_hull_takes_any_preset_with_its_fire_section_inert() {
    let beats = [
        (
            5_000_i64,
            CardBeat::Stage(RED2, CardChange::Preset(SmolStr::new("Sniper"))),
        ),
        (
            5_000,
            CardBeat::Stage(RED, CardChange::Preset(SmolStr::new("Sniper"))),
        ),
    ];
    let (run, kept, arena) = card_run(the_j_field(Controller::Mind), 5_000, 30_000, &beats);
    let lines = card_lines(&run);
    let landed: Vec<(i64, serde_json::Value, serde_json::Value, serde_json::Value)> = lines
        .iter()
        .map(|(t_ms, line)| {
            (
                *t_ms,
                line["kind"].clone(),
                line["unit"].clone(),
                line["name"].clone(),
            )
        })
        .collect();
    assert_eq!(
        landed,
        vec![
            (
                5_000,
                serde_json::json!("mind_card"),
                serde_json::json!(RED.0),
                serde_json::json!("Sniper")
            ),
            (
                5_000,
                serde_json::json!("mind_card"),
                serde_json::json!(RED2.0),
                serde_json::json!("Sniper")
            ),
        ],
        "both land, unit-id order, nothing refused"
    );
    let card = arena
        .minds
        .get(&RED2)
        .map(|state| state.card.clone())
        .expect("the unarmed mind's row");
    assert_eq!(card, preset_on_field("sniper"));
    // The read-back: the fire section inert, with its reason.
    let unarmed = arena.unit(RED2).is_some_and(|unit| unit.weapons.is_empty());
    assert!(unarmed);
    let read = card.read_back(&reflex_cfg(), !unarmed);
    assert_eq!(
        read.get(1).map(String::as_str),
        Some("  fire discipline  no weapons station on this hull")
    );
    // No slot, ever.
    assert!(
        !arena.conditions.rows().any(|(unit, _)| *unit == RED2),
        "the unarmed hull holds no slot"
    );
    for (unit, t_ms, view, result) in &kept {
        if *unit != RED2 {
            continue;
        }
        assert!(
            armings(&result.intents).is_empty(),
            "the unarmed hull answered a Condition at {t_ms}"
        );
        assert!(
            !view
                .notices
                .iter()
                .any(|notice| matches!(notice, FrameNotice::Refused { .. })),
            "a refusal in the unarmed hull's view at {t_ms}"
        );
        stock_emission_holds(&result.intents);
    }
    // It drives and holds as before.
    let (_, plain, _) = card_run(the_j_field(Controller::Mind), 5_000, 30_000, &[]);
    let answers = |kept: &[Kept]| -> Vec<(i64, Vec<IntentKind>)> {
        kept.iter()
            .filter(|(unit, ..)| *unit == RED2)
            .map(|(_, t_ms, _, result)| (*t_ms, result.intents.clone()))
            .collect()
    };
    assert_eq!(
        answers(&kept),
        answers(&plain),
        "the unarmed hull's answers"
    );
    assert!(
        answers(&kept).iter().any(|(_, intents)| intents
            .iter()
            .any(|kind| matches!(kind, IntentKind::Goal(_)))),
        "it drives"
    );
    // The armed hull re-arms at 65 per cent at its next think.
    let next = arms_after(&kept, RED, 5_000);
    assert_eq!(
        next.first().map(|(_, arms)| arms.clone()),
        Some(vec![650]),
        "the armed red's next think: {next:?}"
    );
}

/// **A newer order is obeyed at once after an early end** (17b cut M1b.5; the
/// lead's ruling on M1a.6's residual (10): `StockMemory.ignore_order: bool`
/// became `ignore_until_ms: Option<i64>`, the ENDED order's `expires_ms`). On
/// the (j) field's armed red, online, thought through the production runner
/// with the arena's own view:
///
/// 1. an `Advance` to a cell the motor said no route reaches (the view's
///    `drive_no_route` for the memory's goal) is ended for this hull —
///    `ignore_until_ms` is the order's `expires_ms` (20,000) — and not
///    obeyed: the think falls back to a waypoint;
/// 2. the SAME order on the next view stays ended (not obeyed, the key kept);
/// 3. a second order with another `expires_ms` (30,000) is obeyed at the next
///    think — the goal its `Advance` names — and the key is cleared (the
///    Bool ignored every order until the view carried none);
/// 4. a view with no order clears the key.
#[test]
fn m1b5_a_newer_order_is_obeyed_after_an_early_end() {
    use tacsim::arena::perception::DriveOutcome;
    use tacsim::arena::stock_mind::StockMemory;
    let mut arena = the_j_field(Controller::Mind);
    if let Some(state) = arena.unit_mut(RED) {
        state.power = Power::Online;
    }
    let base = arena.mind_view(RED).expect("red's view");
    let card = preset_on_field("line");
    // 17b cut M2.6: the arena's own runner — the production stock mind the
    // (j) field opened with, or the field's twin under
    // `support::under_the_twin`.
    let mind = Arc::clone(&arena.mind_runner);
    let failed = (12_u16, 3_u16);
    let advance = |to: (u16, u16), expires_ms: i64| FrameOrder {
        squad: SmolStr::new("red-1"),
        objective: MindObjective::Advance { to },
        hold_fire: false,
        max_range_band: None,
        expires_ms,
    };
    let think =
        |view: &MindView, memory: &StockMemory, t_ms: i64| -> (Vec<IntentKind>, StockMemory) {
            let bytes = tacsim::arena::mind::encode_memory(&tacsim::arena::mind::Memory::from(
                memory.clone(),
            ));
            let result = mind.think(&ThinkJob {
                unit: RED,
                think_seq: 1,
                t_ms,
                lands_ms: t_ms + SUBSTEP,
                seed: arena.seed,
                dropped: false,
                view: &Arc::new(view.clone()),
                memory: &bytes,
                map: &arena.definition,
                card: &card,
            });
            (
                result.intents,
                StockMemory::from(&tacsim::arena::mind::decode_memory(&result.memory)),
            )
        };
    let start = StockMemory {
        state: ReflexState::Patrol,
        goal: Some(failed),
        waypoint: Some(1),
        unheld_since_ms: Some(0),
        ..StockMemory::default()
    };
    // 1. The order ended by the failed search.
    let mut view = base.clone();
    view.contacts.clear();
    view.order = Some(advance(failed, 20_000));
    view.notices = vec![FrameNotice::Drive {
        outcome: DriveOutcome::NoRoute,
    }];
    let (ended, memory) = think(&view, &start, 5_000);
    assert_eq!(memory.ignore_until_ms, Some(20_000), "{ended:?}");
    assert!(
        !ended.contains(&IntentKind::Goal(MotorGoal::Cell {
            x: failed.0,
            y: failed.1
        })),
        "the ended Advance is not driven: {ended:?}"
    );
    // 2. The same order again: still ended.
    view.notices.clear();
    let (again, memory) = think(&view, &memory, 6_000);
    assert_eq!(memory.ignore_until_ms, Some(20_000));
    assert!(
        !again.contains(&IntentKind::Goal(MotorGoal::Cell {
            x: failed.0,
            y: failed.1
        })),
        "{again:?}"
    );
    // 3. A newer order: obeyed at once, the key cleared.
    view.order = Some(advance((3, 3), 30_000));
    let (obeyed, newer) = think(&view, &memory, 7_000);
    assert_eq!(newer.ignore_until_ms, None);
    assert!(
        obeyed.contains(&IntentKind::Goal(MotorGoal::Cell { x: 3, y: 3 })),
        "the newer Advance is driven: {obeyed:?}"
    );
    // 4. No order: the key cleared.
    view.order = None;
    let (_, cleared) = think(&view, &memory, 7_000);
    assert_eq!(cleared.ignore_until_ms, None);
}

/// **The anchor's record** (17b cut M1b.5; 17b's Q-M6, proof (ii)): the
/// two-T1 fixture ([`stock_minds`] at 5,000 ms, the (j) field on
/// [`reflex_cfg`] — itself a non-default doctrine) as it ran at 17b cut
/// M1b.4's HEAD `69880812`, BEFORE `NativeStockMind::think` read its card
/// through `StockCard::as_reflex`: every ledger line `(t_ms, kind as JSON)`.
/// (Its line hashes and substep hashes were recorded beside it and are
/// retired: [`m1b5_t1_reads_its_card_through_the_overlay_and_line_is_unchanged`].)
const M1B5_TWO_T1_EVENTS: &[(i64, &str)] = &[
    (0, "{\"kind\":\"match_started\"}"),
    (
        4,
        "{\"kind\":\"detected\",\"viewer\":6,\"contact\":7330038967278110817,\"target\":4}",
    ),
    (
        4,
        "{\"kind\":\"detected\",\"viewer\":6,\"contact\":10007312734772483259,\"target\":9}",
    ),
    (
        189,
        "{\"kind\":\"detected\",\"viewer\":9,\"contact\":15474813515948476951,\"target\":4}",
    ),
    (
        189,
        "{\"kind\":\"detected\",\"viewer\":9,\"contact\":14127509583146488135,\"target\":6}",
    ),
    (254, "{\"kind\":\"startup\",\"unit\":6}"),
    (
        482,
        "{\"kind\":\"detected\",\"viewer\":4,\"contact\":527481421542167805,\"target\":6}",
    ),
    (
        482,
        "{\"kind\":\"detected\",\"viewer\":4,\"contact\":5852833454602713067,\"target\":9}",
    ),
    (732, "{\"kind\":\"startup\",\"unit\":4}"),
    (754, "{\"kind\":\"power_online\",\"unit\":6}"),
    (1232, "{\"kind\":\"power_online\",\"unit\":4}"),
    (
        3004,
        "{\"kind\":\"classified\",\"viewer\":6,\"contact\":7330038967278110817,\"target\":4,\"team\":\"red\"}",
    ),
    (
        3004,
        "{\"kind\":\"classified\",\"viewer\":6,\"contact\":10007312734772483259,\"target\":9,\"team\":\"blue\"}",
    ),
    (
        3189,
        "{\"kind\":\"classified\",\"viewer\":9,\"contact\":15474813515948476951,\"target\":4,\"team\":\"red\"}",
    ),
    (
        3189,
        "{\"kind\":\"classified\",\"viewer\":9,\"contact\":14127509583146488135,\"target\":6,\"team\":\"red\"}",
    ),
    (
        3482,
        "{\"kind\":\"classified\",\"viewer\":4,\"contact\":527481421542167805,\"target\":6,\"team\":\"red\"}",
    ),
    (
        3482,
        "{\"kind\":\"classified\",\"viewer\":4,\"contact\":5852833454602713067,\"target\":9,\"team\":\"blue\"}",
    ),
    (
        3732,
        "{\"kind\":\"locked\",\"unit\":4,\"contact\":5852833454602713067,\"target\":9}",
    ),
    (
        3732,
        "{\"kind\":\"fired\",\"unit\":4,\"weapon\":\"cannon\",\"shot_seq\":1,\"target\":9,\"range_cm\":4116,\"rounds\":1,\"by\":\"mind\",\"slot\":1}",
    ),
    (
        3760,
        "{\"kind\":\"impact\",\"shot_seq\":1,\"unit\":4,\"target\":9}",
    ),
    (
        3760,
        "{\"kind\":\"hit\",\"shot_seq\":1,\"unit\":4,\"target\":9,\"section\":\"hull\",\"facet\":\"front\",\"armor_section\":\"hull\",\"rounds\":1,\"damage\":120,\"armor_left\":0,\"internals_left\":0,\"cause\":\"shot:1\"}",
    ),
    (
        3760,
        "{\"kind\":\"section_destroyed\",\"unit\":9,\"section\":\"hull\",\"by\":4}",
    ),
    (
        5000,
        "{\"kind\":\"position_summary\",\"units\":[[4,4630,8434,0,224848],[6,5000,9000,0,225000],[9,9500,8500,0,0]]}",
    ),
    (
        6750,
        "{\"kind\":\"blocked\",\"unit\":6,\"x\":4,\"y\":8,\"cause\":\"occupied\"}",
    ),
    (
        7732,
        "{\"kind\":\"fired\",\"unit\":4,\"weapon\":\"cannon\",\"shot_seq\":2,\"target\":9,\"range_cm\":4849,\"rounds\":1,\"by\":\"cond\",\"slot\":1}",
    ),
    (
        7765,
        "{\"kind\":\"impact\",\"shot_seq\":2,\"unit\":4,\"target\":9}",
    ),
    (
        7765,
        "{\"kind\":\"hit\",\"shot_seq\":2,\"unit\":4,\"target\":9,\"section\":\"hull\",\"facet\":\"front\",\"armor_section\":\"hull\",\"rounds\":1,\"damage\":120,\"armor_left\":0,\"internals_left\":0,\"cause\":\"shot:2\"}",
    ),
    (
        10000,
        "{\"kind\":\"position_summary\",\"units\":[[4,4653,8350,0,88000],[6,2956,8673,0,249288],[9,9500,8500,0,0]]}",
    ),
    (
        11732,
        "{\"kind\":\"fired\",\"unit\":4,\"weapon\":\"cannon\",\"shot_seq\":3,\"target\":9,\"range_cm\":4849,\"rounds\":1,\"by\":\"cond\",\"slot\":1}",
    ),
    (
        11765,
        "{\"kind\":\"impact\",\"shot_seq\":3,\"unit\":4,\"target\":9}",
    ),
    (
        11765,
        "{\"kind\":\"hit\",\"shot_seq\":3,\"unit\":4,\"target\":9,\"section\":\"hull\",\"facet\":\"front\",\"armor_section\":\"hull\",\"rounds\":1,\"damage\":120,\"armor_left\":0,\"internals_left\":0,\"cause\":\"shot:3\"}",
    ),
    (
        15000,
        "{\"kind\":\"position_summary\",\"units\":[[4,4653,8350,0,88000],[6,5627,7497,0,89847],[9,9500,8500,0,0]]}",
    ),
    (
        15732,
        "{\"kind\":\"fired\",\"unit\":4,\"weapon\":\"cannon\",\"shot_seq\":4,\"target\":9,\"range_cm\":4849,\"rounds\":1,\"by\":\"cond\",\"slot\":1}",
    ),
    (
        15765,
        "{\"kind\":\"impact\",\"shot_seq\":4,\"unit\":4,\"target\":9}",
    ),
    (
        15765,
        "{\"kind\":\"hit\",\"shot_seq\":4,\"unit\":4,\"target\":9,\"section\":\"hull\",\"facet\":\"front\",\"armor_section\":\"hull\",\"rounds\":1,\"damage\":120,\"armor_left\":0,\"internals_left\":0,\"cause\":\"shot:4\"}",
    ),
    (
        19732,
        "{\"kind\":\"fired\",\"unit\":4,\"weapon\":\"cannon\",\"shot_seq\":5,\"target\":9,\"range_cm\":4849,\"rounds\":1,\"by\":\"cond\",\"slot\":1}",
    ),
    (
        19765,
        "{\"kind\":\"impact\",\"shot_seq\":5,\"unit\":4,\"target\":9}",
    ),
    (
        19765,
        "{\"kind\":\"hit\",\"shot_seq\":5,\"unit\":4,\"target\":9,\"section\":\"hull\",\"facet\":\"front\",\"armor_section\":\"hull\",\"rounds\":1,\"damage\":120,\"armor_left\":0,\"internals_left\":0,\"cause\":\"shot:5\"}",
    ),
    (
        20000,
        "{\"kind\":\"position_summary\",\"units\":[[4,4653,8350,0,88000],[6,10627,7500,0,90000],[9,9500,8500,0,0]]}",
    ),
    (
        23732,
        "{\"kind\":\"fired\",\"unit\":4,\"weapon\":\"cannon\",\"shot_seq\":6,\"target\":9,\"range_cm\":4849,\"rounds\":1,\"by\":\"cond\",\"slot\":1}",
    ),
    (
        23765,
        "{\"kind\":\"impact\",\"shot_seq\":6,\"unit\":4,\"target\":9}",
    ),
    (
        23765,
        "{\"kind\":\"hit\",\"shot_seq\":6,\"unit\":4,\"target\":9,\"section\":\"hull\",\"facet\":\"front\",\"armor_section\":\"hull\",\"rounds\":1,\"damage\":120,\"armor_left\":0,\"internals_left\":0,\"cause\":\"shot:6\"}",
    ),
    (
        25000,
        "{\"kind\":\"position_summary\",\"units\":[[4,4653,8350,0,88000],[6,12843,7339,0,295055],[9,9500,8500,0,0]]}",
    ),
    (
        27732,
        "{\"kind\":\"fired\",\"unit\":4,\"weapon\":\"cannon\",\"shot_seq\":7,\"target\":9,\"range_cm\":4849,\"rounds\":1,\"by\":\"cond\",\"slot\":1}",
    ),
    (
        27765,
        "{\"kind\":\"impact\",\"shot_seq\":7,\"unit\":4,\"target\":9}",
    ),
    (
        27765,
        "{\"kind\":\"hit\",\"shot_seq\":7,\"unit\":4,\"target\":9,\"section\":\"hull\",\"facet\":\"front\",\"armor_section\":\"hull\",\"rounds\":1,\"damage\":120,\"armor_left\":0,\"internals_left\":0,\"cause\":\"shot:7\"}",
    ),
    (
        30000,
        "{\"kind\":\"position_summary\",\"units\":[[4,4653,8350,0,88000],[6,7868,7500,0,270000],[9,9500,8500,0,0]]}",
    ),
];

/// Every think's answer, `(unit, lattice ms, intents as JSON)`, recorded with
/// the event list.
const M1B5_TWO_T1_INTENTS: &[(u32, i64, &str)] = &[
    (6, 4, "[\"startup\"]"),
    (4, 482, "[\"startup\"]"),
    (6, 1004, "[{\"goal\":{\"cell\":{\"x\":1,\"y\":8}}}]"),
    (4, 1482, "[{\"goal\":{\"cell\":{\"x\":1,\"y\":8}}}]"),
    (6, 2004, "[]"),
    (4, 2482, "[]"),
    (6, 3004, "[]"),
    (
        4,
        3482,
        "[{\"condition\":{\"arm\":{\"weapon\":\"cannon\",\"target\":{\"target\":\"best_hostile\"},\"min_permille\":100,\"aspects\":15,\"min_rounds_left\":0,\"min_range_m\":0,\"max_range_m\":100}}},{\"goal\":\"hold\"},{\"lock\":{\"contact\":5852833454602713067}},{\"fire\":{\"weapon\":\"cannon\",\"slot\":1}}]",
    ),
    (6, 4004, "[]"),
    (
        4,
        4482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":90000}}]",
    ),
    (6, 5004, "[]"),
    (
        4,
        5482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 6004, "[]"),
    (
        4,
        6482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 7004, "[]"),
    (
        4,
        7482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 8004, "[]"),
    (
        4,
        8482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 9004, "[]"),
    (
        4,
        9482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 10004, "[{\"goal\":{\"cell\":{\"x\":14,\"y\":8}}}]"),
    (
        4,
        10482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 11004, "[]"),
    (
        4,
        11482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 12004, "[]"),
    (
        4,
        12482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 13004, "[]"),
    (
        4,
        13482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 14004, "[]"),
    (
        4,
        14482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 15004, "[]"),
    (
        4,
        15482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 16004, "[]"),
    (
        4,
        16482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 17004, "[]"),
    (
        4,
        17482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 18004, "[]"),
    (
        4,
        18482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 19004, "[]"),
    (
        4,
        19482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 20004, "[]"),
    (
        4,
        20482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 21004, "[]"),
    (
        4,
        21482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 22004, "[]"),
    (
        4,
        22482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 23004, "[{\"goal\":{\"cell\":{\"x\":1,\"y\":8}}}]"),
    (
        4,
        23482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 24004, "[]"),
    (
        4,
        24482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 25004, "[]"),
    (
        4,
        25482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 26004, "[]"),
    (
        4,
        26482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 27004, "[]"),
    (
        4,
        27482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 28004, "[]"),
    (
        4,
        28482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
    (6, 29004, "[]"),
    (
        4,
        29482,
        "[{\"goal\":\"hold\"},{\"heading\":{\"mdeg\":88000}}]",
    ),
];

// ---------------------------------------------------------------------------
// 17b cut M2.2: the seam prepared — the one memory codec, the faults and the
// strikes, the doctrine pair at the arena (doc 17 §6, §8, §11; D-UM-29,
// D-UM-30 as amended, D-UM-31, D-UM-32)
// ---------------------------------------------------------------------------

/// FNV-1a 64 over `bytes` — the per-think memory digest the codec pin
/// records.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

// **The memory bytes of every think of the two-T1 fixture at 1,000 ms and of
// `unarmed_run(1_000)`, RECORDED BEFORE 17b cut M2.2's edit** — on HEAD
// `dec5328b`'s code, whose `encode_memory` was `serde_json::to_vec` over the
// arena's own map — as `(unit, the think's lattice millisecond, FNV-1a 64 of
// the answered memory)`, with the first and the last memory whole. After the
// edit the bytes are `mush_mind::memory::encode`'s, and
// `m2_the_memory_codec_writes_the_native_bytes` holds them equal.
const M2_TWO_T1_MEMORY_DIGESTS: &[(u32, i64, u64)] = &[
    (6, 4, 0x0460_2376_35ff_267b),
    (4, 482, 0x0460_2376_35ff_267b),
    (6, 1_004, 0xaf26_3241_c01f_62e9),
    (4, 1_482, 0x9ef0_1268_31d8_f2ef),
    (6, 2_004, 0xaf26_3241_c01f_62e9),
    (4, 2_482, 0x9ef0_1268_31d8_f2ef),
    (6, 3_004, 0x0b69_c385_48be_b28d),
    (4, 3_482, 0xd39b_f9d4_27c3_9b44),
    (6, 4_004, 0x0b69_c385_48be_b28d),
    (4, 4_482, 0x9742_7018_57be_fb26),
    (6, 5_004, 0x0b69_c385_48be_b28d),
    (4, 5_482, 0x3687_6e51_598d_d3d8),
    (6, 6_004, 0x0b69_c385_48be_b28d),
    (4, 6_482, 0xca0e_1edb_2df5_c8ee),
    (6, 7_004, 0x0b69_c385_48be_b28d),
    (4, 7_482, 0xd9bc_35ad_e7f2_ded4),
    (6, 8_004, 0x0b69_c385_48be_b28d),
    (4, 8_482, 0x6eec_5f4c_4d98_587e),
    (6, 9_004, 0x0b69_c385_48be_b28d),
    (4, 9_482, 0x771a_2b2d_afdb_1730),
    (6, 10_004, 0xf73b_a523_e930_0ad9),
    (4, 10_482, 0x40f1_3df3_024b_2a2e),
    (6, 11_004, 0xf73b_a523_e930_0ad9),
    (4, 11_482, 0x57a3_da44_7d3f_0fb6),
    (6, 12_004, 0xf73b_a523_e930_0ad9),
    (4, 12_482, 0x257b_249c_8632_7ea6),
    (6, 13_004, 0xf73b_a523_e930_0ad9),
    (4, 13_482, 0x2679_b529_0282_9426),
    (6, 14_004, 0xf73b_a523_e930_0ad9),
    (4, 14_482, 0x9b3c_1328_d139_344e),
    (6, 15_004, 0xf73b_a523_e930_0ad9),
    (4, 15_482, 0xc3a7_f046_2688_31a6),
    (6, 16_004, 0xf73b_a523_e930_0ad9),
    (4, 16_482, 0xc9d9_3643_f649_de46),
    (6, 17_004, 0xf73b_a523_e930_0ad9),
    (4, 17_482, 0x7eee_d174_3b41_7cf6),
    (6, 18_004, 0xf73b_a523_e930_0ad9),
    (4, 18_482, 0xbfa5_e130_736c_866e),
    (6, 19_004, 0xf73b_a523_e930_0ad9),
    (4, 19_482, 0x72c1_6789_6964_3aa6),
    (6, 20_004, 0xf73b_a523_e930_0ad9),
    (4, 20_482, 0x6d4c_7873_6dfe_653a),
    (6, 21_004, 0xf73b_a523_e930_0ad9),
    (4, 21_482, 0xc25d_702e_4cce_76ca),
    (6, 22_004, 0xf73b_a523_e930_0ad9),
    (4, 22_482, 0x7385_02d3_ccff_cd96),
    (6, 23_004, 0xf248_9844_b8e9_16f7),
    (4, 23_482, 0xb564_13c0_19cf_98e6),
    (6, 24_004, 0xf248_9844_b8e9_16f7),
    (4, 24_482, 0x4aa8_713b_9c40_cfd2),
    (6, 25_004, 0xf248_9844_b8e9_16f7),
    (4, 25_482, 0x2b7e_80e7_5d95_3a32),
    (6, 26_004, 0xf248_9844_b8e9_16f7),
    (4, 26_482, 0x0011_1673_9237_2806),
    (6, 27_004, 0xf248_9844_b8e9_16f7),
    (4, 27_482, 0x69a3_1ae1_d8b1_0d26),
    (6, 28_004, 0xf248_9844_b8e9_16f7),
    (4, 28_482, 0x76a0_42e7_889c_0d8a),
    (6, 29_004, 0xf248_9844_b8e9_16f7),
    (4, 29_482, 0x5930_3819_3dd5_5bfa),
];
const M2_TWO_T1_FIRST_MEMORY: &[u8] = b"{\"from_waypoint\":null,\"goal\":null,\"hold_retry_ms\":null,\"hunt\":null,\"ignore_until_ms\":null,\"last_plot\":null,\"no_line\":false,\"quiet_since_ms\":null,\"rallied\":false,\"stand_until_ms\":null,\"state\":0,\"target\":null,\"unheld_since_ms\":null,\"waypoint\":null}";
const M2_TWO_T1_LAST_MEMORY: &[u8] = b"{\"from_waypoint\":null,\"goal\":null,\"hold_retry_ms\":null,\"hunt\":null,\"ignore_until_ms\":null,\"last_plot\":[9,8,29482],\"no_line\":false,\"quiet_since_ms\":null,\"rallied\":false,\"stand_until_ms\":null,\"state\":3,\"target\":5852833454602713067,\"unheld_since_ms\":29482,\"waypoint\":0}";
const M2_UNARMED_MEMORY_DIGESTS: &[(u32, i64, u64)] = &[
    (4, 482, 0x0460_2376_35ff_267b),
    (4, 1_482, 0x9ef0_1268_31d8_f2ef),
    (4, 2_482, 0x9ef0_1268_31d8_f2ef),
    (4, 3_482, 0x0b69_c385_48be_b28d),
    (4, 4_482, 0x0b69_c385_48be_b28d),
    (4, 5_482, 0x0b69_c385_48be_b28d),
    (4, 6_482, 0xf73b_a523_e930_0ad9),
    (4, 7_482, 0x89fb_60e5_aa94_69b9),
    (4, 8_482, 0x89fb_60e5_aa94_69b9),
    (4, 9_482, 0x89fb_60e5_aa94_69b9),
    (4, 10_482, 0xd30f_f24d_1871_836d),
    (4, 11_482, 0x8c60_3cbb_ce75_9e94),
    (4, 12_482, 0x72e1_8c26_d680_169b),
    (4, 13_482, 0x67b2_9890_5528_6ba2),
    (4, 14_482, 0xbad4_b209_bb9b_46b9),
    (4, 15_482, 0x9804_10b8_e736_ee80),
    (4, 16_482, 0x01d3_f636_b2d7_1467),
    (4, 17_482, 0x9f69_d73a_09dd_cb8e),
    (4, 18_482, 0x5487_7b0f_a8d3_f2a5),
    (4, 19_482, 0x9a13_d799_f4a5_c9cc),
    (4, 20_482, 0x4176_4b6d_a199_e777),
    (4, 21_482, 0xe87f_a0f7_474f_ea5e),
    (4, 22_482, 0x6d3e_9251_61cb_4849),
    (4, 23_482, 0xf6d8_9b99_f8a4_14d0),
    (4, 24_482, 0x99de_101f_12da_14ab),
    (4, 25_482, 0x5ebb_984a_89f5_cf72),
    (4, 26_482, 0xec8f_92b5_094c_04bd),
    (4, 27_482, 0x36c4_c03f_d7a4_f324),
    (4, 28_482, 0xd30d_566e_b90f_cc0f),
    (4, 29_482, 0x3b6f_5ce9_8c22_f4b6),
    (4, 30_482, 0x718d_8776_4367_8e9c),
    (4, 31_482, 0x5b8e_1190_d5e3_4775),
    (4, 32_482, 0xba07_f361_af5a_4d0a),
    (4, 33_482, 0x202f_89c2_3f88_4203),
    (4, 34_482, 0x470f_a84d_71b6_2fe8),
    (4, 35_482, 0x981b_5a4d_a521_2f81),
    (4, 36_482, 0x21a3_0bd8_416c_47f6),
    (4, 37_482, 0x819b_e775_9e37_074f),
    (4, 38_482, 0x975f_f6f9_d789_db64),
    (4, 39_482, 0x21cf_dc30_4de1_4ffd),
];
const M2_UNARMED_FIRST_MEMORY: &[u8] = b"{\"from_waypoint\":null,\"goal\":null,\"hold_retry_ms\":null,\"hunt\":null,\"ignore_until_ms\":null,\"last_plot\":null,\"no_line\":false,\"quiet_since_ms\":null,\"rallied\":false,\"stand_until_ms\":null,\"state\":0,\"target\":null,\"unheld_since_ms\":null,\"waypoint\":null}";
const M2_UNARMED_LAST_MEMORY: &[u8] = b"{\"from_waypoint\":0,\"goal\":[1,8],\"hold_retry_ms\":null,\"hunt\":null,\"ignore_until_ms\":null,\"last_plot\":null,\"no_line\":false,\"quiet_since_ms\":null,\"rallied\":false,\"stand_until_ms\":null,\"state\":4,\"target\":null,\"unheld_since_ms\":39482,\"waypoint\":1}";

/// A memory as `serde_json` would have written it before 17b cut M2.2: the
/// same pairs as a sorted map of `serde_json::Value`s (`Unit` as `null`,
/// `Ints` as an array of numbers).
fn json_of(memory: &tacsim::arena::mind::Memory) -> BTreeMap<String, serde_json::Value> {
    use tacsim::arena::mind::MemValue;
    memory
        .iter()
        .map(|(key, value)| {
            let value = match value {
                MemValue::Int(n) => serde_json::Value::from(*n),
                MemValue::Bool(flag) => serde_json::Value::Bool(*flag),
                MemValue::Unit => serde_json::Value::Null,
                MemValue::Str(text) => serde_json::Value::String(text.clone()),
                MemValue::Ints(items) => serde_json::Value::Array(
                    items.iter().copied().map(serde_json::Value::from).collect(),
                ),
            };
            (key.clone(), value)
        })
        .collect()
}

/// **A seeded generator of valid stock memories** (17b cut M2.2): splitmix64
/// over a counter, every value kind, keys and strings of printable ASCII
/// with the quote and the backslash drawn often, each bound of
/// `STOCK_MEMORY_LIMITS` reached.
struct MemoryGen {
    state: u64,
}

impl MemoryGen {
    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(1);
        tacsim::arena::rng::splitmix64(self.state)
    }

    /// A number in `0..=high`.
    fn upto(&mut self, high: u64) -> u64 {
        self.next() % (high + 1)
    }

    /// Printable ASCII of `low..=high` bytes, a quote or a backslash one byte
    /// in four.
    fn text(&mut self, low: u64, high: u64) -> String {
        let len = low + self.upto(high - low);
        (0..len)
            .map(|_| match self.upto(7) {
                0 => '"',
                1 => '\\',
                _ => char::from(u8::try_from(0x20 + self.upto(0x7E - 0x20)).unwrap_or(b' ')),
            })
            .collect()
    }

    fn int(&mut self) -> i64 {
        match self.upto(5) {
            0 => i64::MIN,
            1 => i64::MAX,
            2 => 0,
            3 => i64::try_from(self.upto(2_000)).unwrap_or(0) - 1_000,
            _ => i64::from_ne_bytes(self.next().to_ne_bytes()),
        }
    }

    fn value(&mut self) -> tacsim::arena::mind::MemValue {
        use tacsim::arena::mind::MemValue;
        match self.upto(4) {
            0 => MemValue::Int(self.int()),
            1 => MemValue::Bool(self.upto(1) == 1),
            2 => MemValue::Unit,
            3 => MemValue::Str(self.text(0, 32)),
            _ => {
                let len = self.upto(32);
                MemValue::Ints((0..len).map(|_| self.int()).collect())
            }
        }
    }

    fn memory(&mut self) -> tacsim::arena::mind::Memory {
        let keys = self.upto(32);
        let mut memory = tacsim::arena::mind::Memory::new();
        for _ in 0..keys {
            let key = self.text(1, 24);
            let value = self.value();
            memory.insert(key, value);
        }
        memory
    }
}

/// **The one codec writes the native bytes** (17b cut M2.2; doc 17 §8,
/// D-UM-30 as amended; 17b's settled question (i)):
///
/// 1. the memory every think of the two-T1 fixture at 1,000 ms and of
///    `unarmed_run(1_000)` answered is, byte for byte, the memory recorded
///    BEFORE the edit on `serde_json::to_vec` ([`M2_TWO_T1_MEMORY_DIGESTS`],
///    [`M2_UNARMED_MEMORY_DIGESTS`], the first and last memory whole) — the
///    native stock mind writes the same bytes through `mush_mind`'s encoder —
///    and every one decodes, passes `memory_check` and encodes back to
///    itself. **The printable-ASCII rule changes nothing T1 writes**: its
///    fourteen keys are ASCII words and it writes no string value;
/// 2. ten thousand random valid memories from a seeded generator over every
///    value kind encode to exactly `serde_json::to_vec`'s bytes for the same
///    shape and decode back to themselves;
/// 3. `M1B5_TWO_T1_EVENTS`, `M1B5_TWO_T1_INTENTS` and `M1B2_TWO_T1_FIRED`
///    stand, re-asserted by their own tests
///    (`m1b5_t1_reads_its_card_through_the_overlay_and_line_is_unchanged`
///    and `m1b2_two_t1_slot_numbers_are_the_arenas_at_every_landing`'s
///    sibling `m1a6_two_stock_minds_at_three_cadences_are_one_event_list_and_one_hash_sequence`),
///    untouched by this cut.
#[test]
fn m2_the_memory_codec_writes_the_native_bytes() {
    use tacsim::arena::mind::{decode_memory, encode_memory, memory_check};
    for (name, kept, digests, first, last) in [
        (
            "two-T1",
            stock_minds(Drive::Every(1_000)).1,
            M2_TWO_T1_MEMORY_DIGESTS,
            M2_TWO_T1_FIRST_MEMORY,
            M2_TWO_T1_LAST_MEMORY,
        ),
        (
            "unarmed",
            unarmed_run(1_000).1,
            M2_UNARMED_MEMORY_DIGESTS,
            M2_UNARMED_FIRST_MEMORY,
            M2_UNARMED_LAST_MEMORY,
        ),
    ] {
        let got: Vec<(u32, i64, u64)> = kept
            .iter()
            .map(|(unit, t_ms, _, result)| (unit.0, *t_ms, fnv1a(&result.memory)))
            .collect();
        assert_eq!(got, digests, "{name}: every think's memory bytes");
        assert_eq!(
            kept.first().map(|(.., result)| result.memory.as_slice()),
            Some(first),
            "{name}: the first memory whole"
        );
        assert_eq!(
            kept.last().map(|(.., result)| result.memory.as_slice()),
            Some(last),
            "{name}: the last memory whole"
        );
        for (_, t_ms, _, result) in &kept {
            let decoded = mush_mind::memory::decode(&result.memory)
                .unwrap_or_else(|| panic!("{name} at {t_ms}: the memory decodes"));
            assert_eq!(memory_check(&decoded), Ok(()), "{name} at {t_ms}");
            assert_eq!(encode_memory(&decoded), result.memory, "{name} at {t_ms}");
            assert_eq!(decode_memory(&result.memory), decoded, "{name} at {t_ms}");
        }
    }
    let mut generator = MemoryGen { state: 2_350 };
    let mut kinds = [0_usize; 5];
    for at in 0..10_000 {
        let memory = generator.memory();
        for value in memory.values() {
            let kind = match value {
                tacsim::arena::mind::MemValue::Int(_) => 0,
                tacsim::arena::mind::MemValue::Bool(_) => 1,
                tacsim::arena::mind::MemValue::Unit => 2,
                tacsim::arena::mind::MemValue::Str(_) => 3,
                tacsim::arena::mind::MemValue::Ints(_) => 4,
            };
            if let Some(count) = kinds.get_mut(kind) {
                *count += 1;
            }
        }
        assert_eq!(memory_check(&memory), Ok(()), "map {at}: {memory:?}");
        let bytes = mush_mind::memory::encode(&memory);
        let native = serde_json::to_vec(&json_of(&memory)).expect("a map serializes");
        assert_eq!(
            bytes,
            native,
            "map {at}: {}",
            String::from_utf8_lossy(&native)
        );
        assert_eq!(
            mush_mind::memory::decode(&bytes),
            Some(memory.clone()),
            "map {at}"
        );
        assert_eq!(encode_memory(&memory), bytes, "map {at}: the arena's name");
    }
    assert!(
        kinds.iter().all(|count| *count > 1_000),
        "every kind drawn: {kinds:?}"
    );
}

/// The (j) field's red, scripted for the strikes fixture (17b cut M2.2):
/// think 0 starts the hull and lands the memory `[0]`; think 1 switches a pod
/// the hull does not carry (its refusal is the notice the faulted thinks are
/// handed and must put back) and lands `[1]`; each think in `faulted`
/// answers a speed and the memory `[9, 9]` — with the fault word `ops` when
/// `overflow` is false, and as SEVENTEEN speeds with no fault word (D-UM-9's
/// overflow) when it is true — and every other think nothing and the memory
/// `[n]`.
fn strikes_script(faulted: &[u64], overflow: bool) -> Vec<Says> {
    (0..10_u64)
        .map(|n| {
            if faulted.contains(&n) {
                let speed = IntentKind::Speed { cms: 300 };
                Says {
                    think_seq: n,
                    intents: if overflow {
                        vec![speed; MIND_INTENTS_MAX + 1]
                    } else {
                        vec![speed]
                    },
                    memory: Some(vec![9, 9]),
                    fault: (!overflow).then_some("ops"),
                }
            } else {
                Says {
                    think_seq: n,
                    intents: match n {
                        0 => vec![IntentKind::Startup],
                        1 => vec![IntentKind::Emcon { on: true }],
                        _ => Vec::new(),
                    },
                    memory: Some(vec![u8::try_from(n).unwrap_or(u8::MAX)]),
                    fault: None,
                }
            }
        })
        .collect()
}

/// One pulse-by-pulse run of the strikes fixture to just past red's think
/// `last`: the scripted mind, every substep hash, red's `strikes` after
/// every pulse, red's row just before and just after each of its thinks, and
/// the counters. `reset` is the baseline's hand: after every pulse red's
/// `strikes` are written to the figure it names for that pulse.
struct StrikesRun {
    mind: Arc<ScriptedMind>,
    hashes: Vec<Vec<u64>>,
    strikes: Vec<u8>,
    rows: Vec<(MindState, MindState)>,
    counters: ReflexCounters,
    landings_after: Vec<bool>,
    speed: Option<i32>,
}

fn strikes_run(script: Vec<Says>, last: u64, reset: Option<&[u8]>) -> StrikesRun {
    let mind = ScriptedMind::new(&[(RED, script)]);
    let mut rig = Rig::new(the_j_field(Controller::Mind));
    rig.arena.mind_runner = mind.clone();
    let mut out = StrikesRun {
        mind,
        hashes: Vec::new(),
        strikes: Vec::new(),
        rows: Vec::new(),
        counters: ReflexCounters::default(),
        landings_after: Vec::new(),
        speed: None,
    };
    let pulse = |rig: &mut Rig, out: &mut StrikesRun| {
        let step = rig.pulse(&[], &[]);
        out.counters.absorb(&step.reflex);
        if let Some(state) = rig.arena.minds.get_mut(&RED) {
            if let Some(figure) = reset.and_then(|figures| figures.get(out.strikes.len())) {
                // The baseline's hand — and the overflow already counts the
                // strike, so the hand writes the figure the row holds.
                assert_eq!(
                    state.strikes, *figure,
                    "the overflow counts a strike, as the fault does"
                );
                state.strikes = *figure;
            }
            out.strikes.push(state.strikes);
        }
        out.hashes.push(rig.arena.substep_hashes.clone());
    };
    for n in 0..=last {
        let think = think_at(RED, i64::try_from(n).unwrap_or(0));
        while rig.arena.now_ms + SUBSTEP < think {
            pulse(&mut rig, &mut out);
        }
        let before = rig.arena.minds.get(&RED).cloned().expect("red's row");
        while rig.arena.now_ms < think {
            pulse(&mut rig, &mut out);
        }
        let after = rig.arena.minds.get(&RED).cloned().expect("red's row");
        out.landings_after.push(
            rig.arena.mind_landings.contains_key(&RED)
                || rig.arena.due.iter().any(
                    |event| matches!(event.kind, DueKind::MindLand { unit, .. } if unit == RED),
                ),
        );
        out.rows.push((before, after));
    }
    for _ in 0..4 {
        pulse(&mut rig, &mut out);
    }
    out.speed = rig.arena.unit(RED).map(|unit| unit.desired_speed_cms);
    out
}

/// **A faulting runner lands nothing, counts a strike, and three drop the
/// hull** (17b cut M2.2; doc 17 §8, D-UM-31, D-UM-32; D-UM-9's atomic think):
/// a [`ScriptedMind`] answering `fault: Some(ThinkFault { cause: "ops" })` —
/// with a speed and the memory `[9, 9]` beside it — at red's thinks 2, 3
/// and 4 on the (j) field:
///
/// * at each, NOTHING lands: no landing on the scratch or the heap, the
///   memory the last landed think left (`[1]`) kept, the notices the think
///   was handed back at the front of the buffer (think 1's refused pod switch
///   among them, so think 3 is handed think 2's notices again), the range
///   memory as it stood, `pending` clear — and the speed never ordered;
/// * `strikes` reads 1, 2, 3 and `mind_faults` 3 (no `mind_overflows`);
///   `ThinkJob::dropped` is false through think 4 and TRUE from think 5;
///   every job carries the match seed (`ThinkJob::seed`, D-UM-29: the key
///   of a mind's draws) — and a second arena on another seed hands that one;
///   the clean think 5 lands its memory and leaves `strikes` at 3 — the drop
///   is for the rest of the sortie;
/// * **the baseline** — the same script answering SEVENTEEN speeds and no
///   fault word at thinks 2, 3 and 4 (M1a's overflow path), with red's
///   `strikes` written by hand after every pulse to the faulting run's figure
///   (and found already there: the overflow counts a strike) — hashes EQUAL
///   at every substep to the faulting run: the new fault path is M1a's
///   overflow path plus `strikes` and nothing else. The overflow run counts
///   `mind_overflows` 3 AND `mind_faults` 3;
/// * one event list and one hash sequence at 5,000, 1,000 and 500 ms;
/// * a runner faulting at think 2 and clean at 3 reads `strikes` 0 at think
///   4 (D-UM-31's reset), its think 4 not dropped.
#[test]
fn m2_a_faulting_runner_lands_nothing_counts_a_strike_and_three_drop_the_hull() {
    let faulting = strikes_run(strikes_script(&[2, 3, 4], false), 5, None);
    let seen = |run: &StrikesRun, n: u64| {
        run.mind
            .seen()
            .into_iter()
            .find(|seen| seen.unit == RED && seen.think_seq == n)
            .unwrap_or_else(|| panic!("red's think {n} was run"))
    };
    for n in 0..=5_u64 {
        let at = usize::try_from(n).unwrap_or(0);
        let (before, after) = faulting.rows.get(at).cloned().expect("a row a think");
        let handed = seen(&faulting, n);
        assert_eq!(handed.dropped, n >= 5, "ThinkJob::dropped at think {n}");
        assert_eq!(handed.seed, SEED, "ThinkJob::seed at think {n}");
        assert_eq!(after.think_seq, n + 1, "think {n} ran");
        if (2..=4).contains(&n) {
            assert_eq!(after.memory, vec![1], "think {n}: the memory kept");
            assert_eq!(after.memory, before.memory, "think {n}");
            assert_eq!(
                after.ranges, before.ranges,
                "think {n}: the ranges restored"
            );
            assert_eq!(after.pending, None, "think {n}");
            assert!(
                after.notices.starts_with(&handed.view.notices),
                "think {n}: the notices back in front of the buffer: {:?} / {:?}",
                handed.view.notices,
                after.notices
            );
            assert!(
                !faulting.landings_after.get(at).copied().unwrap_or(true),
                "think {n}: nothing to land"
            );
            assert_eq!(after.strikes, u8::try_from(n - 1).unwrap_or(0), "think {n}");
        } else {
            assert!(
                faulting.landings_after.get(at).copied().unwrap_or(false),
                "think {n}: a clean think lands"
            );
        }
    }
    // Think 1's refused pod switch reached think 2, which faulted and put it
    // back, so think 3 was handed it again.
    let two = seen(&faulting, 2).view.notices;
    assert!(
        two.iter().any(|notice| matches!(
            notice,
            FrameNotice::Refused { order, .. } if order == "emcon"
        )),
        "think 2 is handed think 1's refusal: {two:?}"
    );
    assert!(seen(&faulting, 3).view.notices.starts_with(&two));
    assert!(seen(&faulting, 4).view.notices.starts_with(&two));
    // The drop holds through the clean think 5, which lands.
    let (_, five) = faulting.rows.get(5).cloned().expect("think 5");
    assert_eq!(five.strikes, MIND_STRIKES_TO_DROP);
    assert!(five.dropped());
    assert_eq!(faulting.strikes.last().copied(), Some(MIND_STRIKES_TO_DROP));
    assert_eq!(
        (
            faulting.counters.mind_faults,
            faulting.counters.mind_overflows
        ),
        (3, 0)
    );
    // The speed the faulted thinks answered was never ordered.
    assert_eq!(faulting.speed, Some(0));
    // The seed is the arena's own, not a constant: another match seed is
    // the one every job carries.
    let other_seed = SEED ^ 0x5eed_0000_0000_0001;
    let mut reseeded = the_j_field(Controller::Mind);
    assert_eq!(reseeded.seed, SEED);
    reseeded.seed = other_seed;
    let mind = ScriptedMind::new(&[(RED, strikes_script(&[], false))]);
    reseeded.mind_runner = mind.clone();
    let _ = run(&mut reseeded, Drive::Every(1_000), 5_000);
    let jobs = mind.seen();
    assert!(!jobs.is_empty(), "the reseeded arena thought");
    assert!(
        jobs.iter().all(|seen| seen.seed == other_seed),
        "every job carries the arena's seed: {:?}",
        jobs.iter().map(|seen| seen.seed).collect::<Vec<_>>()
    );

    // The baseline: M1a's overflow path, the strikes as the faulting run's.
    let overflow = strikes_run(strikes_script(&[2, 3, 4], true), 5, Some(&faulting.strikes));
    assert_eq!(overflow.hashes.len(), faulting.hashes.len());
    for (at, (fault, flow)) in faulting.hashes.iter().zip(&overflow.hashes).enumerate() {
        assert_eq!(fault, flow, "the substep hashes of pulse {at}");
    }
    assert_eq!(
        (
            overflow.counters.mind_faults,
            overflow.counters.mind_overflows
        ),
        (3, 3),
        "the overflow counts a fault and an overflow"
    );
    assert_eq!(overflow.strikes, faulting.strikes);

    // One event list and one hash sequence at 5,000, 1,000 and 500 ms.
    let tempo = |cadence_ms: i64| {
        let mut arena = the_j_field(Controller::Mind);
        arena.mind_runner = ScriptedMind::new(&[(RED, strikes_script(&[2, 3, 4], false))]);
        let run = run(&mut arena, Drive::Every(cadence_ms), 10_000);
        let strikes = arena.minds.get(&RED).map(|state| state.strikes);
        (run, strikes)
    };
    let (slow, strikes) = tempo(5_000);
    assert_eq!(strikes, Some(MIND_STRIKES_TO_DROP));
    assert_eq!(slow.counters.mind_faults, 3);
    for cadence in [1_000, 500] {
        let (other, other_strikes) = tempo(cadence);
        assert_eq!(other.events, slow.events, "the event list at {cadence} ms");
        assert_eq!(
            other.hashes, slow.hashes,
            "the hash sequence at {cadence} ms"
        );
        assert_eq!(other_strikes, strikes);
        assert_ne!(other.pulses, slow.pulses);
    }

    // A fault at think 2, clean at 3: the strikes reset, think 4 not dropped.
    let reset = strikes_run(strikes_script(&[2], false), 4, None);
    let strikes_after = |n: usize| reset.rows.get(n).map(|(_, after)| after.strikes);
    assert_eq!(strikes_after(2), Some(1));
    assert_eq!(strikes_after(3), Some(0), "a clean think clears a strike");
    assert_eq!(strikes_after(4), Some(0));
    assert!(!seen(&reset, 4).dropped);
}

/// **The arena refuses a broken doctrine pair in the card's own sentence**
/// (17b cut M2.2, the lead's ruling (b); doc 17 §11, D-UM-43): a
/// `break_off_pct 10` staged for the armed red on Line (`break_off_pct` 35,
/// `fight_home_pct` 17.5) is journalled `mind_card_refused` with
/// `CardError::Pair`'s sentence — the same text `@mind/set` wraps — and the
/// row keeps Line.
#[test]
fn m2_the_arena_refuses_a_broken_pair_in_the_cards_own_sentence() {
    let (run, _, arena) = card_run(
        the_j_field(Controller::Mind),
        5_000,
        10_000,
        &[(
            5_000,
            CardBeat::Stage(
                RED,
                CardChange::Set {
                    slider: SmolStr::new("break_off_pct"),
                    value: SmolStr::new("10"),
                },
            ),
        )],
    );
    let lines = card_lines(&run);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].1["kind"], "mind_card_refused");
    assert_eq!(
        lines[0].1["reason"],
        "break_off_pct would put fight_home_pct 17.5 above break_off_pct 10; fight_home_pct \
         is at most break_off_pct"
    );
    assert_eq!(
        arena.minds.get(&RED).map(|state| state.card.clone()),
        Some(preset_on_field("line"))
    );
}

/// **The arena refuses a preset that breaks the pair, by the preset's name**
/// (17b cut M2.2, the lead's ruling (b): the presets pair-valid; doc 17 §11,
/// D-UM-44): on a doctrine the config admits with `fight_home` 75 under
/// `break_off` 80, a staged `Rearguard` — `break_off_pct` 70 over Line's
/// `fight_home_pct` 75 — is journalled `mind_card_refused` with `Pair`'s
/// sentence naming `Rearguard`, and the row keeps Line on that doctrine.
/// Never clamped.
#[test]
fn m2_the_arena_refuses_a_preset_that_breaks_the_pair_by_name() {
    let high = ReflexConfig {
        withdraw_permille: 800,
        stand_min_permille: 750,
        ..reflex_cfg()
    };
    let (run, _, arena) = card_run(
        the_j_field_on(Controller::Mind, high.clone()),
        5_000,
        10_000,
        &[(
            5_000,
            CardBeat::Stage(RED, CardChange::Preset(SmolStr::new("Rearguard"))),
        )],
    );
    let lines = card_lines(&run);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].1["kind"], "mind_card_refused");
    assert_eq!(
        lines[0].1["reason"],
        "Rearguard would put fight_home_pct 75 above break_off_pct 70; fight_home_pct is at \
         most break_off_pct"
    );
    assert_eq!(
        arena.minds.get(&RED).map(|state| state.card.clone()),
        Some(tacsim::arena::line(&high))
    );
}

// ---------------------------------------------------------------------------
// 17b cut M2.6: the T1 fixtures under the twin (doc 17 §17 "Gate A"; 17b
// §1 (iii))
// ---------------------------------------------------------------------------
//
// Each fixture of this file that flies the native stock mind (T1) is run
// once more inside `support::under_the_twin`: the SAME body, its field
// builders ([`the_j_field_on`], [`lance_field`], [`charge_field`]) installing
// an asserting `support::TwinRunner` and its recorder ([`Stock::on`]) wrapping
// the field's runner, that twin, so every think the fixture flies is T1 and
// T2 (the Rhai Standard Doctrine) on one job — the Rhai fault refused first,
// then the intents, then the memory bytes — with T1's answer flown. The
// fixture's own assertions on T1 stand untouched (its recorded constants
// among them: `M1B2_TWO_T1_FIRED`, `M1B5_TWO_T1_EVENTS`,
// `M1B5_TWO_T1_INTENTS`, `M1B5_WITHDRAWS_AT`); `under_the_twin` asserts the
// twins compared at least one think and prints how many. The fixtures that
// field no T1 (the scripted minds of 17b cuts M1a.5, M1b.0, M1b.1 and M1b.3's
// hint re-key, the faulting and pairing fixtures of M2.2) are not repeated.

/// **The two-T1 fixture under the twin** (17b cut M2.6): [`stock_minds`] —
/// the (j) field's two reds, the armed one finding, classifying, locking,
/// arming its slot and citing it — flown by the twin at 5,000, 1,000 and
/// 500 ms and on the task's wakes at the three periods, each equal to the
/// native run (events, the hash sequence, every think's view and answer);
/// [`two_t1_slots_and_citations`] passes on the twin-flown run —
/// [`M1B2_TWO_T1_FIRED`] kept — and its event list and answers are 17b cut
/// M1b.5's anchor ([`M1B5_TWO_T1_EVENTS`], [`M1B5_TWO_T1_INTENTS`]). (The
/// native fixture's last arm — the arena's DEFAULT runner is the native
/// stock mind — is the native fixture's alone: under the twin the field's
/// runner is the twin.)
#[test]
fn m2_the_two_t1_fixture_under_the_twin() {
    let (native, native_kept) = stock_minds(Drive::Every(5_000));
    support::under_the_twin("the two-T1 fixture", || {
        let (first, kept) = stock_minds(Drive::Every(5_000));
        assert_eq!(first.events, native.events, "the native event list");
        assert_eq!(first.hashes, native.hashes, "the native hash sequence");
        assert_eq!(kept, native_kept, "the native thinks");
        two_t1_slots_and_citations(&first, &kept);
        let events: Vec<(i64, &str)> = first
            .events
            .iter()
            .map(|(t_ms, json, _)| (*t_ms, json.as_str()))
            .collect();
        assert_eq!(events, M1B5_TWO_T1_EVENTS, "17b cut M1b.5's anchor");
        let answers: Vec<(u32, i64, String)> = kept
            .iter()
            .map(|(unit, t_ms, _, result)| {
                (
                    unit.0,
                    *t_ms,
                    serde_json::to_string(&result.intents).unwrap_or_default(),
                )
            })
            .collect();
        let recorded: Vec<(u32, i64, String)> = M1B5_TWO_T1_INTENTS
            .iter()
            .map(|(unit, t_ms, json)| (*unit, *t_ms, (*json).to_owned()))
            .collect();
        assert_eq!(answers, recorded, "17b cut M1b.5's recorded answers");
        for drive in [
            Drive::Every(1_000),
            Drive::Every(500),
            Drive::Task(5_000),
            Drive::Task(1_000),
            Drive::Task(500),
        ] {
            let (other, other_kept) = stock_minds(drive);
            assert_eq!(other.events, first.events, "the event list at {drive:?}");
            assert_eq!(other.hashes, first.hashes, "the hash sequence at {drive:?}");
            assert_eq!(other_kept, kept, "a think at {drive:?}");
        }
    });
}

/// [`m1b2_two_t1_slot_numbers_are_the_arenas_at_every_landing`] under the
/// twin (17b cut M2.6).
#[test]
fn m1b2_two_t1_slot_numbers_are_the_arenas_at_every_landing_under_the_twin() {
    support::under_the_twin(
        "m1b2_two_t1_slot_numbers_are_the_arenas_at_every_landing",
        m1b2_two_t1_slot_numbers_are_the_arenas_at_every_landing,
    );
}

/// [`m1a6_the_stock_mind_is_t0_on_the_first_decide_window`] under the twin
/// (17b cut M2.6): the (j) field's T1 window flown by the twin.
#[test]
fn m1a6_the_stock_mind_is_t0_on_the_first_decide_window_under_the_twin() {
    support::under_the_twin(
        "m1a6_the_stock_mind_is_t0_on_the_first_decide_window",
        m1a6_the_stock_mind_is_t0_on_the_first_decide_window,
    );
}

/// [`l2_two_minds_are_one_squad_under_a_leader_at_three_cadences`] under the
/// twin (17b cut M2.6): [`lance_field`]'s three minds under the leader's
/// hinted `ENGAGE`, every think compared.
#[test]
fn l2_two_minds_are_one_squad_under_a_leader_at_three_cadences_under_the_twin() {
    support::under_the_twin(
        "l2_two_minds_are_one_squad_under_a_leader_at_three_cadences",
        l2_two_minds_are_one_squad_under_a_leader_at_three_cadences,
    );
}

/// [`l2_a_self_charged_support_fights_on_doctrine`] under the twin (17b cut
/// M2.6): [`charge_field`]'s self-charged and blind supporters.
#[test]
fn l2_a_self_charged_support_fights_on_doctrine_under_the_twin() {
    support::under_the_twin(
        "l2_a_self_charged_support_fights_on_doctrine",
        l2_a_self_charged_support_fights_on_doctrine,
    );
}

/// [`m1b5_t1_reads_its_card_through_the_overlay_and_line_is_unchanged`] under
/// the twin (17b cut M2.6): the M1b.5 anchor's recorded events and answers
/// and the withdrawal at [`M1B5_WITHDRAWS_AT`] on the non-default doctrine.
#[test]
fn m1b5_t1_reads_its_card_through_the_overlay_and_line_is_unchanged_under_the_twin() {
    support::under_the_twin(
        "m1b5_t1_reads_its_card_through_the_overlay_and_line_is_unchanged",
        m1b5_t1_reads_its_card_through_the_overlay_and_line_is_unchanged,
    );
}

/// [`m1b5_a_card_lands_at_the_pulses_start_and_the_next_think_reads_it`]
/// under the twin (17b cut M2.6): a staged card read at the next think —
/// the re-arm at Rearguard's 500 per mille compared whole.
#[test]
fn m1b5_a_card_lands_at_the_pulses_start_and_the_next_think_reads_it_under_the_twin() {
    support::under_the_twin(
        "m1b5_a_card_lands_at_the_pulses_start_and_the_next_think_reads_it",
        m1b5_a_card_lands_at_the_pulses_start_and_the_next_think_reads_it,
    );
}

/// [`m1b5_an_unarmed_hull_takes_any_preset_with_its_fire_section_inert`]
/// under the twin (17b cut M2.6).
#[test]
fn m1b5_an_unarmed_hull_takes_any_preset_with_its_fire_section_inert_under_the_twin() {
    support::under_the_twin(
        "m1b5_an_unarmed_hull_takes_any_preset_with_its_fire_section_inert",
        m1b5_an_unarmed_hull_takes_any_preset_with_its_fire_section_inert,
    );
}

/// [`m1b5_a_newer_order_is_obeyed_after_an_early_end`] under the twin (17b
/// cut M2.6): the four thinks through the (j) field's twin.
#[test]
fn m1b5_a_newer_order_is_obeyed_after_an_early_end_under_the_twin() {
    support::under_the_twin(
        "m1b5_a_newer_order_is_obeyed_after_an_early_end",
        m1b5_a_newer_order_is_obeyed_after_an_early_end,
    );
}

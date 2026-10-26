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

use crate::support;
use support::scripted::{Kept, Says, ScriptedMind, Seen, Stock, says, think_at};

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
use tacsim::arena::command::{SignalReason, SquadSignal};
use tacsim::arena::condition::{Condition, Reading};
use tacsim::arena::conditions::{
    ASPECTS_ANY, Arming, FireCondition, STOCK_CONDITION_SLOTS, SlotTarget,
};
use tacsim::arena::doctrine::MachineState;
use tacsim::arena::intents::{
    Control, ControlState, Intent, IntentKind, IntentOutcome, IntentSource, MotorGoal, Refusal,
    apply_control, apply_intent,
};
use tacsim::arena::ledger::LedgerEvent;
use tacsim::arena::mind::{
    MIND_INTENTS_MAX, MIND_NOTICES, MIND_STRIKES_TO_DROP, MindRunner, MindState, MindView,
    STOCK_THINK_MS, ThinkJob, ThinkResult, decode_memory, doctrine_state_of, seed_withdraw,
    withdraw_seed,
};
use tacsim::arena::motor::MotorCounters;
use tacsim::arena::perception::{
    FrameNotice, FrameOrder, MindObjective, RefusalDetail, SensorProfile, SignatureFactor,
};
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
    // 17b cut M3.1 (plan F6): a Class I core in the hull, through
    // `tacsim::definitions::core::core_component` — the shape `bind` lays
    // down from content — so the hull holds two condition slots and a mind
    // on it thinks at Class I's figures, as it did before cores existed.
    tactical_unit_fitted(id, Some(tacsim::arena::core::CoreClass::I))
}

/// [`tactical_unit`] with a governed core of `class` in its hull, or with
/// none at all — a hull that never had a core (17b cut M3.1).
fn tactical_unit_fitted(id: ObjId, class: Option<tacsim::arena::core::CoreClass>) -> TacticalUnit {
    let (components, capabilities) = class.map_or_else(
        || (Vec::new(), Vec::new()),
        |class| {
            let (core, capability) = tacsim::definitions::core::core_component(
                "hull",
                class,
                dref("component", "tacsim.unit.line_tank.core"),
                dref("schema", tacsim::definitions::unit::CORE_RULES_SCHEMA),
            )
            .expect("a core");
            (vec![core], vec![capability])
        },
    );
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
        components,
        capabilities,
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
        seal: tacsim::arena::MindSeal::placeholder(),
    }
    .sealed()
}

/// **The same custody sealed as the production fill is** (17b cut M4.3):
/// `SealTerms::production_fixture()` — `PRODUCTION_FILL_ENGINE`, rated I,
/// capped at the default `fill_class` — over the custody as it stands, for
/// the pins that field the production shape.
fn as_fill(mut unit: CustodyUnit) -> CustodyUnit {
    unit.reseal(&tacsim::match_::SealTerms::production_fixture())
        .expect("the production terms seal");
    unit
}

/// **The production installer over Standard Doctrine** (17b cut M4.3): every
/// scripted seal's runner installed on `Arena::runners` from the compiled-in
/// stock source — what `arena_bridge` does for a production match whose every
/// seal names the stock digest; a no-op when the fill engine is `Native`.
fn install_standard(arena: &mut Arena) -> usize {
    let cfg = arena.arena_cfg.clone();
    tacsim::minds::install_sealed_runners(arena, &cfg, &|_| {
        Some(tacsim::minds::STANDARD_RHAI.to_owned())
    })
    .expect("Standard Doctrine installs")
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
    .sealed()
}

// ---------------------------------------------------------------------------
// The scripted mind
// ---------------------------------------------------------------------------

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
        // 17b cut M4.1: settled between pulses, as the task's pool settles.
        let out = arena.step_settled(wall, &[], &[]);
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
    let events: Vec<(i64, String, u64)> = events(arena, &mut bytes)
        .into_iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    // 17b cut M2.7: held to the native run under `support::under_the_rhai_runner`.
    support::note_run(&events, &hashes);
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
        // 17b cut M4.1: settled between pulses, as the task's pool settles.
        let out = self.arena.step_settled(wall, controls, intents);
        for chunk in &out.ledger_chunks {
            self.bytes.extend_from_slice(&chunk.bytes);
        }
        out
    }

    /// Pulse until `done` holds after a pulse, at most to `limit_ms` of arena
    /// time; answers the counters of the pulse it held after.
    fn until(&mut self, limit_ms: i64, mut done: impl FnMut(&Arena) -> bool) -> MotorCounters {
        while self.arena.now_ms < limit_ms {
            let out = self.pulse(&[], &[]);
            if done(&self.arena) {
                return out.motor;
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
        .and_then(|state| state.pending.as_ref())
        .is_some_and(|pending| pending.think_seq == n)
}

/// Whether `unit`'s mind has a think in flight — its answer on the row (17b
/// cut M3.2: `MindState.pending` carries it; the scratch table is retired).
fn in_flight(arena: &Arena, unit: ObjId) -> bool {
    arena
        .minds
        .get(&unit)
        .is_some_and(|state| state.pending.is_some())
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

/// **A carried answer is refused at a raised epoch after a resume** (17b cut
/// M3.2; D-UM-13, D-UM-23 as amended): [`epoch_rig`]'s think 4 in flight,
/// encoded and decoded — the answer rides the row's `pending`, its three
/// epochs with it — and resumed (on the default runner, which the landing
/// never calls). In the resumed arena's next pulse, before the landing, the
/// same `Control { Linkdead }` on the HELM channel at a higher epoch that
/// [`m1a5_an_epoch_raised_between_think_and_landing_refuses_the_stale_thoughts`]
/// sends: the carried answer lands at its RECORDED epochs, so the heading and
/// the speed are refused `epoch` into the mind's buffer through the carried
/// answer, and the arming and the cited `FIRE` meet the linkdead gate — the
/// uninterrupted fixture's four refusals exactly, and nothing fired; the
/// answer is gone off the row. With the helm epoch raised alone instead, the
/// two helm thoughts are refused `epoch` and the cited shot lands at the
/// landing.
#[test]
fn m3_a_carried_answer_is_refused_epoch_at_a_raised_epoch_after_a_resume() {
    let land = think_at(RED, 4) + SUBSTEP;
    let resumed = |rig: &Rig| -> Rig {
        let carried = rig
            .arena
            .minds
            .get(&RED)
            .and_then(|state| state.pending.clone())
            .expect("think 4 in flight at the encode");
        assert_eq!(carried.think_seq, 4);
        assert_eq!(
            carried.intents.len(),
            4,
            "the heading, the speed, the arming, the fire"
        );
        let snapshot = Snapshot::encode(&rig.arena).expect("the arena encodes");
        let decoded = Snapshot::decode(&snapshot.document, (*rig.arena.definition).clone())
            .expect("the document decodes");
        assert_eq!(
            decoded
                .minds
                .get(&RED)
                .and_then(|state| state.pending.clone()),
            Some(carried),
            "the answer came back with its epochs"
        );
        let mut resumed = Rig::new(decoded);
        let now = resumed.arena.now_ms;
        let _ = resumed.arena.resume_at(now);
        resumed.wall = Some(now);
        resumed
    };

    let mut rig = resumed(&epoch_rig());
    let raised = rig.arena.epochs.current(RED, Channel::Helm) + 1;
    let _ = rig.pulse(
        &[control(Channel::Helm, raised, ControlState::Linkdead, None)],
        &[],
    );
    assert!(rig.arena.now_ms >= land, "the landing's substep ran");
    assert_eq!(
        refusals(&rig.arena, RED),
        vec![
            ("heading".to_owned(), Refusal::Epoch),
            ("speed".to_owned(), Refusal::Epoch),
            ("fire".to_owned(), Refusal::Linkdead),
            ("fire".to_owned(), Refusal::Linkdead),
        ],
        "the carried answer refused at its recorded epochs"
    );
    assert!(rig.lines("fired", RED).is_empty(), "nothing fired");
    assert!(!in_flight(&rig.arena, RED), "the answer is off the row");

    let mut rig = resumed(&epoch_rig());
    let raised = rig.arena.epochs.current(RED, Channel::Helm) + 1;
    rig.arena.epochs.raise(RED, Channel::Helm, raised);
    let _ = rig.pulse(&[], &[]);
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
        "the fire channel was not raised: the carried shot lands at the landing"
    );
}

/// **A reclaimed seat lands nothing** (D-UM-13; `apply_control`'s `Live` arm).
/// A `Control { Live }` naming a pilot between think 4 and its landing hands
/// the hull to the pilot, which takes the mind's pending answer (17b cut
/// M3.2) and its two due kinds off the hull: the landing never comes, nothing
/// of the think is applied or journalled, and no refusal is staged. **17b cut
/// M3.5** (doc 17 §12 "Release and possess"): the row itself STAYS, dormant
/// (`Arena::suspend_mind`), with its answer discarded — this test asserted
/// "the mind's row went" until then.
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
    // 17b cut M3.5: the row stays, dormant — and the think in flight, whose
    // answer it carried (17b cut M3.2 retired the scratch beside it), is
    // discarded off it.
    assert!(
        rig.arena
            .minds
            .get(&RED)
            .is_some_and(|state| state.pending.is_none()),
        "the mind's row stays, its answer discarded"
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
/// `[9, 9]`: nothing lands (no landing on the heap and no answer pending, the ordered
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
        launch_cell: None,
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
            .sealed()
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
    let target = |id: ObjId, y: u16| {
        CustodyUnit {
            profile: every_band_profile(),
            ..tank(id, Team::Blue, centre(9, y), Vec::new(), Controller::Mind)
        }
        .sealed()
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
    assert_eq!(out.motor.orders_released, 1);
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

/// **17b cut M5.2's STOP E rig**: [`m1a5_an_order_to_a_mind_is_re_keyed_to_its_own_held_contact_or_none`]'s
/// field — red's mind (`RED`, the supporter) beside its wingman (`RED2`, a
/// pilot's tank, the charge) and blue — on a mind that answers nothing.
fn support_rig() -> Rig {
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
    rig
}

/// A wingman's `SUPPORT` of `RED2` to `RED`'s mind, a minute long.
fn support_of_the_wingman(arena: &Arena) -> Intent {
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
}

/// `RED`'s held order and the charge beside it.
fn held_support(arena: &Arena) -> (Option<FrameOrder>, Option<ObjId>) {
    arena.minds.get(&RED).map_or((None, None), |state| {
        (state.order.clone(), state.order_charge)
    })
}

/// **Pulse until `RED`'s order is gone** — at most a think period and two
/// substeps past the charge leaving at `left_ms` — and read the one
/// `order_expired` line the arena journals for it: `support`, after the
/// leaving and before the order's own deadline.
fn ended_by_its_charge_leaving(rig: &mut Rig, left_ms: i64, expires_ms: i64) {
    let _ = rig.until(left_ms + STOCK_THINK_MS + 2 * SUBSTEP, |arena| {
        held_support(arena).0.is_none()
    });
    assert_eq!(
        held_support(&rig.arena),
        (None, None),
        "the order and its charge went together"
    );
    let expired: Vec<(i64, serde_json::Value)> = rig
        .journal()
        .iter()
        .filter(|event| event.kind.as_str() == "order_expired")
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_value(&event.kind).unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(expired.len(), 1, "one line: {expired:?}");
    let (t_ms, line) = &expired[0];
    assert!(
        *t_ms > left_ms && *t_ms < expires_ms,
        "ended by its charge leaving, not run out: {t_ms}"
    );
    assert_eq!(line.get("objective"), Some(&serde_json::json!("support")));
    assert_eq!(line.get("unit"), Some(&serde_json::json!(RED.0)));
}

/// **17b cut M5.2: a `Support` landed blind is ended when its charge leaves**
/// (doc 17 §24's STOP E, its first residual; the owner's remedy of
/// 2026-10-09, the charge's `ObjId` on the order). A `SUPPORT` of the
/// wingman landed before any pass — the mind holds no row for its charge,
/// so it re-keys to `Support { of: None }` — carries the charge's unit on
/// the row (`MindState::order_charge`). The charge leaves the match (taken
/// out of `Arena::units`, as `arena/catchup.rs`' observer test takes one):
/// the arena ends the order at the mind's next landing, one `order_expired`
/// line naming `support`, inside a think period. Until this cut such an
/// order named no row and stood blind until its `expires_ms`.
#[test]
fn m5_a_support_landed_blind_is_ended_when_its_charge_leaves() {
    let mut rig = support_rig();
    assert!(
        rig.arena.contacts.of(RED).next().is_none(),
        "the precondition: the supporter holds nothing"
    );
    let first = support_of_the_wingman(&rig.arena);
    let _ = rig.pulse(&[], &[first]);
    let (order, charge) = held_support(&rig.arena);
    let order = order.expect("the support landed");
    assert_eq!(
        order.objective,
        MindObjective::Support { of: None },
        "landed blind: no row for its charge"
    );
    assert_eq!(charge, Some(RED2), "the charge's unit, on the order");
    rig.arena.units.retain(|unit| unit.id != RED2);
    let left_ms = rig.arena.now_ms;
    ended_by_its_charge_leaving(&mut rig, left_ms, order.expires_ms);
}

/// **17b cut M5.2: a supporter blind to its charge for a contact TTL is
/// ended when its charge leaves** (doc 17 §24's STOP E, its second residual).
/// The `SUPPORT` lands once the mind has told its wingman apart — `Support {
/// of: Some(row) }` — and the mind then goes blind to it for a contact TTL:
/// its row ages out of its store (taken here as `ShadowContacts::expire`
/// takes one past the TTL). The charge leaves the match: the arena ends the
/// order at the next landing, one `order_expired` line naming `support`.
/// Until this cut the end read the supporter's row for its charge, which was
/// gone, and the order stood blind until its `expires_ms` (the blind stand
/// of `t1_a_support_keeps_station_and_stands_blind`).
#[test]
fn m5_a_supporter_blind_for_a_ttl_is_ended_when_its_charge_leaves() {
    let mut rig = support_rig();
    let _ = rig.until(5_000, |arena| told_apart(arena, RED, RED2));
    let again = support_of_the_wingman(&rig.arena);
    let _ = rig.pulse(&[], &[again]);
    let row = rig
        .arena
        .contacts
        .id_of(RED, RED2)
        .expect("the wingman held");
    let (order, charge) = held_support(&rig.arena);
    let order = order.expect("the support landed");
    assert_eq!(order.objective, MindObjective::Support { of: Some(row) });
    assert_eq!(charge, Some(RED2));
    // Blind for a TTL: the row is gone from the supporter's store.
    assert!(rig.arena.contacts.remove(RED, row).is_some());
    rig.arena.units.retain(|unit| unit.id != RED2);
    let left_ms = rig.arena.now_ms;
    ended_by_its_charge_leaving(&mut rig, left_ms, order.expires_ms);
}

/// **17b cut M5.2: the charge goes wherever the order goes** (STOP E's
/// field, cleared as `MindState::order_issuer` is): a `Support` writes its
/// charge beside the order where it lands; a `RELEASE`, the landing that
/// reaches its deadline and a possess each clear it with the order; any
/// other order, and a self-charge (`Engage`), hold none.
#[test]
fn m5_the_charge_goes_with_the_order_at_a_release_an_expiry_and_a_possess() {
    let mut rig = support_rig();
    let land = |rig: &mut Rig, objective: Objective, ttl_ms: u32| {
        let kind = IntentKind::Order(Box::new(order(RED_ONE, objective, Some(RED2), ttl_ms)));
        let landing = intent(&rig.arena, RED, kind, 1);
        let _ = rig.pulse(&[], &[landing]);
    };
    // A release.
    land(&mut rig, Objective::Support { of: RED2 }, 60_000);
    assert_eq!(held_support(&rig.arena).1, Some(RED2));
    let release = intent(
        &rig.arena,
        RED,
        IntentKind::ReleaseOrder {
            squad: RED_ONE,
            issuer: Some(RED2),
        },
        1,
    );
    let _ = rig.pulse(&[], &[release]);
    assert_eq!(held_support(&rig.arena), (None, None), "a release");
    // The landing that reaches its deadline.
    land(&mut rig, Objective::Support { of: RED2 }, 1_000);
    let deadline = held_support(&rig.arena)
        .0
        .map_or(0, |order| order.expires_ms);
    assert_eq!(held_support(&rig.arena).1, Some(RED2));
    let _ = rig.until(deadline + 3 * STOCK_THINK_MS, |arena| {
        held_support(arena).0.is_none()
    });
    assert_eq!(held_support(&rig.arena), (None, None), "an expiry");
    // Another order, and a self-charge, hold none.
    land(&mut rig, Objective::Hold, 60_000);
    assert!(held_support(&rig.arena).0.is_some());
    assert_eq!(held_support(&rig.arena).1, None, "a hold holds no charge");
    land(&mut rig, Objective::Support { of: RED }, 60_000);
    assert!(matches!(
        held_support(&rig.arena).0.map(|order| order.objective),
        Some(MindObjective::Engage { .. })
    ));
    assert_eq!(held_support(&rig.arena).1, None, "a self-charge is engage");
    // A possess: the row stays, dormant, its order and charge cleared.
    land(&mut rig, Objective::Support { of: RED2 }, 60_000);
    assert_eq!(held_support(&rig.arena).1, Some(RED2));
    rig.arena
        .set_controller(RED, Controller::Pilot, MachineState::Withdraw);
    assert!(rig.arena.minds.contains_key(&RED), "the row stays");
    assert_eq!(held_support(&rig.arena), (None, None), "a possess");
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
    let blind = |unit: CustodyUnit| {
        CustodyUnit {
            sensor: None,
            ..unit
        }
        .sealed()
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
/// two pins, re-asserted by every cut since with the same constants). The
/// first: the (j) field of `tests/reflex.rs` — two machines, one armed, a
/// seated tank — at a fixed 1,000 ms cadence for 30 s, rebuilt here from the
/// same fixtures (from 17b cut M4.3 the two-T1 shape of [`the_j_field`]:
/// every hull seeing ten cells and the seated tank presenting every band, so
/// the minds' slots have odds to fire at; a default profile presents none).
/// The second: four stock machines on the proving ground for a minute
/// ([`support::shipped_arena`]).
///
/// **17b cut M4.3 — the cut's one production re-bless, declared.** The flip
/// made the production fill a `Controller::Mind` sealed with Standard
/// Doctrine on `PRODUCTION_FILL_ENGINE` at `[tacsim.minds].fill_class`, so
/// the production shape IS the mind's: both fields here are re-pointed from
/// `Controller::Reflex` to `Controller::Mind` sealed as the fill is
/// ([`as_fill`], [`support::Muster::fill`]) with its runner installed as a
/// production match installs it ([`install_standard`]), and the three
/// constants are re-blessed ONCE with `tests/reflex.rs`' and
/// `tests/motor.rs`' (the same field and the same duel). The reasons, each a
/// declared fact of the flip and none a change of the fold's encoding: the
/// controller byte folds `2` (Mind) where it folded `1`; the due loop runs
/// `MindThink`/`MindLand` where it ran `ReflexTick`; hash item 3's reflex
/// rows are gone and item 3c's mind rows (memory, strikes, drop, seal
/// digest) fold in their place; the seal digests are the production engine's
/// and the script's digest (D2's fifteenth key) rides every `mind_sealed`
/// line. Old → new: `0x18df_dd94_73aa_5a36` → `0x692d_5b0e_3c04_93a6`,
/// `0x5689_35a8_5830_609d` → `0xd243_f6a1_30c0_00c7`, `0xd25d_39ba_9b04_7064`
/// → `0x65eb_692a_b7a8_a72d`. (The M3.6 re-bless before it: `mind_sealed` and
/// `sortie_closed` shifting the folded ledger sequence, the mind row's fold
/// gaining D-UM-23's answer, `dropped` and the seal digest.)
///
/// **The fill decision — re-pinned, hashes only** (2026-10-09, declared with
/// `tests/reflex.rs`' and `tests/motor.rs`' pins of the same fields).
/// `PRODUCTION_FILL_ENGINE` is `native` (the owner's three ALIEN18 runs all
/// `held=no`), so [`as_fill`] and [`support::Muster::fill`] seal native,
/// [`install_standard`] installs nothing and the minds think on the native
/// stock mind, held equal to the Rhai Standard Doctrine they ran on by Gate
/// A. Each seal's digest folds its engine, so the three constants move and
/// nothing else does: the thinks (60), the faults (0) and the four minds
/// below hold unchanged, and the same fields' tapes (`M1B0_J_LINES`,
/// `M1B0_DUEL_LINES`) are byte-equal but for their `hash` values. Old → new:
/// `0x692d_5b0e_3c04_93a6` → `0xc734_ca3c_2cce_2a0c`, `0xd243_f6a1_30c0_00c7`
/// → `0xbabf_b4c3_71e8_786d`, `0x65eb_692a_b7a8_a72d` →
/// `0x999b_45cb_390f_4af8`. **Hash v2** (16c T-104, 2026-10-12): the state hash's fold moved — `Fold`, eight bytes a multiply, the notices by `fold_value` — so every `hash` moved and nothing else did: `0xc734_ca3c_2cce_2a0c` → `0x6d99_9a2c_d37b_179d`, `0xbabf_b4c3_71e8_786d` →
/// `0xef15_99c8_e01f_1022`, `0x999b_45cb_390f_4af8` → `0xa4cd_3db1_3485_19ef`; the three tapes byte-equal but
/// for their `hash` values, the thinks and faults unchanged.
#[test]
fn m1a5_the_production_hash_does_not_move() {
    let seeing = |mut unit: CustodyUnit| {
        unit.sensor = Some(SensorProfile {
            max_range_cells: 10,
            ..sight()
        });
        unit
    };
    let mut arena = arena_of(
        vec![
            as_fill(seeing(tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                Controller::Mind,
            ))),
            as_fill(seeing(tank(
                RED2,
                Team::Red,
                centre(6, 10),
                Vec::new(),
                Controller::Mind,
            ))),
            as_fill(presenting(seeing(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            )))),
        ],
        &[BLUE],
    );
    let _ = install_standard(&mut arena);
    let run = run(&mut arena, Drive::Every(1_000), 30_000);
    assert_eq!(run.hashes.len(), 120, "the substeps of 30 s at 250 ms");
    let first_hash = run.hashes.first().copied();
    let last_hash = run.hashes.last().copied();
    assert_eq!(
        run.counters.thinks, 60,
        "the two minds' thinks over 30 s at Class I's second"
    );
    assert_eq!(run.counters.mind_faults, 0, "nothing faulted");

    let world = support::booted_world();
    let mut duel = support::shipped_arena(
        &world,
        &[
            &[
                support::Muster::fill(Team::Red, 1),
                support::Muster::fill(Team::Red, 2),
            ],
            &[
                support::Muster::fill(Team::Blue, 1),
                support::Muster::fill(Team::Blue, 2),
            ],
        ],
    );
    assert!(
        duel.units
            .iter()
            .all(|unit| unit.seal.engine == tacsim::match_::PRODUCTION_FILL_ENGINE),
        "every hull sealed on the production engine"
    );
    let _ = support::run_until(&mut duel, 60_000, 1_000, |_, _| {});
    assert_eq!(duel.minds.len(), 4, "four minds");
    // M6 captures full card/catalog, profile/ROM and pending-think state and
    // advances Standard's API/source identity. Keep the encoding pins together
    // so a declared format
    // change exposes every actual in one run; behavior assertions above remain.
    let actual = [
        first_hash,
        last_hash,
        Some(tacsim::arena::hash::state_hash(&duel)),
    ];
    assert_eq!(
        actual,
        [
            Some(0x2fde_0a11_f694_234e),
            Some(0x8dcb_ccd8_aeed_9ed9),
            Some(0xf93e_4819_dcdc_9eab)
        ],
        "production encoding pins (first, last, shipped duel): {actual:x?}"
    );
}

/// **A mind mustered on the proving ground** (17b cut M1a.5): the rig's
/// [`support::Muster::mind`] fields a mind exactly as a reflex pod is fielded,
/// and `Arena::new` lays its row down and schedules its first think; it
/// thinks once an arena second. **17b cut M1a.6**: the default runner is the
/// native stock mind, whose first think starts the cold tank (it was `NoMind`,
/// which ordered nothing). **17b cut M4.5a**: its `no reflex row` probe is
/// dropped — vacuous on a mind's hull since 17b cut M4.3, and the controller
/// and the mind row are asserted beside it.
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
    let mut thinks = 0_u32;
    let _ = support::run_until(&mut arena, 10_000, 1_000, |_, out| {
        thinks = thinks.saturating_add(out.motor.thinks);
    });
    assert_eq!(thinks, 10, "one think an arena second");
    assert_eq!(arena.minds.get(&red).map(|state| state.think_seq), Some(10));
    assert_eq!(
        arena.unit(red).map(|unit| unit.power),
        Some(tacsim::arena::state::Power::Starting),
        "the default runner, the native stock mind, starts the cold tank"
    );
}

/// **An unarmed muster is sealed as it stands** (17b cut M3.3; doc 17 §13,
/// D-UM-45): the seal is a function of the FINISHED custody, so
/// [`support::Muster::mind_unarmed`], which takes the weapon rows off after
/// `custody_unit` sealed the hull, reseals it — the arena it checks out
/// carries the seal of the weaponless custody (no fire channel), not the
/// armed hull's. The expected seals are computed here by the one production
/// computation over the same M32 record, and the arena's `UnitState.seal`
/// and `MindState.seal_digest` are held to each, armed and unarmed.
/// (**17b cut M4.5a**: blue, the opponent no claim reads, is a pilot's hull
/// where it was the reflex machine.)
#[test]
fn m3_an_unarmed_muster_is_sealed_with_no_fire_channel() {
    let world = support::booted_world();
    let w = world.world();
    let red = support::tank(&world, Team::Red, 1);
    let fielded = |muster: support::Muster| {
        support::shipped_arena(
            &world,
            &[&[muster], &[support::Muster::piloted(Team::Blue, 1)]],
        )
    };
    let armed_arena = fielded(support::Muster::mind(Team::Red, 1));
    let unarmed_arena = fielded(support::Muster::mind_unarmed(Team::Red, 1));

    // The production computation over the same record (the pose is not a
    // seal input).
    let catalog = w
        .module_state::<tacsim::TacsimStore>()
        .map(|slot| slot.definitions().clone())
        .unwrap_or_default();
    let cells = mush_tactics::module::unit_of(w, red)
        .expect("the red tank is a tactical unit")
        .clone();
    let (_, pose) = mush_sim::pose_of(w, red).expect("the red tank is placed");
    let terms = tacsim::match_::SealTerms::fixture();
    let mut custody = tacsim::match_::custody_unit(
        &catalog,
        cells,
        red,
        Team::Red,
        pose,
        0,
        Controller::Mind,
        1,
        &terms,
    )
    .expect("the fixture terms seal");
    assert!(!custody.weapons.is_empty(), "the stock tank is armed");
    let armed = custody.seal.clone();
    assert_eq!(
        armed.channels,
        [Channel::Helm, Channel::Sensors, Channel::Fire]
    );
    custody.weapons.clear();
    custody.reseal(&terms).expect("the fixture terms seal");
    let unarmed = custody.seal.clone();
    assert_eq!(unarmed.channels, [Channel::Helm, Channel::Sensors]);
    assert_ne!(armed.digest(), unarmed.digest(), "the channels are sealed");

    for (arena, seal, word) in [
        (&armed_arena, &armed, "armed"),
        (&unarmed_arena, &unarmed, "unarmed"),
    ] {
        assert_eq!(
            arena.unit(red).map(|unit| unit.seal.clone()),
            Some(seal.seal_ref()),
            "the {word} muster's arena seal"
        );
        assert_eq!(
            arena.minds.get(&red).map(|state| state.seal_digest),
            Some(seal.digest()),
            "the {word} muster's mind row"
        );
    }
}

// ---------------------------------------------------------------------------
// (11) every controller transition releases the outgoing rows
// ---------------------------------------------------------------------------

/// What `unit` has left on the arena: a mind row, an answer pending on it
/// (17b cut M3.2: the scratch retired), a motor row, and its controllers'
/// due kinds. (17b cut M4.5a: the reflex row and its `ReflexTick` slot are
/// gone with the controller's fixtures.)
type Rows = (bool, bool, bool, Vec<&'static str>);

fn rows_of(arena: &Arena, unit: ObjId) -> Rows {
    let kinds = arena
        .due
        .iter()
        .filter_map(|event| match event.kind {
            DueKind::MindThink { unit: on, .. } if on == unit => Some("mind_think"),
            DueKind::MindLand { unit: on, .. } if on == unit => Some("mind_land"),
            DueKind::MotorTick { unit: on } if on == unit => Some("motor_tick"),
            _ => None,
        })
        .collect();
    (
        arena.minds.contains_key(&unit),
        in_flight(arena, unit),
        arena.motors.row(unit).is_some(),
        kinds,
    )
}

/// **Every controller transition releases the outgoing rows** (17b cut M1a.5;
/// `Arena::set_controller`). Pilot → Mind by `set_controller` with a pilot's
/// `DRIVE` standing; Mind → Pilot by a `Live` control with a think in flight.
/// After each, nothing of the outgoing controller is left — no mind think,
/// pending answer or landing, no motor row or motor tick — and the incoming
/// controller's first event is on the heap.
///
/// **17b cut M3.5** (doc 17 §12 "Release and possess") moved two steps: the
/// Mind → Reflex step was a `Released` control, which now hands a hull with a
/// row back to its MIND (`apply_control`), so the step is `set_controller`
/// itself — the fixture's own transition, the only one that still removes the
/// row; and Mind → Pilot keeps the mind's row, dormant, with its pending
/// answer, its thinks and its landing gone (it asserted no row until then).
///
/// **17b cut M4.5a**: the three reflex legs — Reflex → Mind, Mind → Reflex
/// with a think in flight, Reflex → Pilot by a `Live` control — are deleted:
/// with the reflex controller's fixtures gone those transitions cease to
/// exist (M4.6 deletes the controller). Red opens a seated pilot's hull (the
/// state the deleted Reflex → Pilot leg left it in: nothing of a machine on
/// the arena), and the two legs that remain are the original's, unchanged.
#[test]
fn m1a5_every_controller_transition_releases_the_outgoing_rows() {
    let mut rig = Rig::new(arena_of(
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
    ));
    rig.arena.mind_runner = ScriptedMind::new(&[]);
    let _ = rig.until(3_000, |arena| arena.now_ms >= 2_000);
    assert_eq!(
        rows_of(&rig.arena, RED),
        (false, false, false, vec![]),
        "a seated pilot flies it: nothing of a machine"
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
    let (_, _, motor, kinds) = rows_of(&rig.arena, RED);
    assert!(motor && kinds == vec!["motor_tick"], "the pilot is driving");
    rig.arena
        .set_controller(RED, Controller::Mind, MachineState::Withdraw);
    assert_eq!(
        rows_of(&rig.arena, RED),
        (true, false, false, vec!["mind_think"])
    );

    // Mind -> Pilot, with a think in flight.
    let _ = rig.until(20_000, |arena| in_flight(arena, RED));
    let epoch = rig.arena.epochs.current(RED, Channel::Helm) + 1;
    let _ = rig.pulse(
        &[control(
            Channel::Helm,
            epoch,
            ControlState::Live,
            Some(ObjId(2_000)),
        )],
        &[],
    );
    // 17b cut M3.5: the mind's row stays, dormant — nothing else of it.
    assert_eq!(rows_of(&rig.arena, RED), (true, false, false, vec![]));
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.controller),
        Some(Controller::Pilot)
    );
}

/// **A resumed landing lands the carried answer** (17b cut M3.2; D-UM-23 as
/// amended 2026-10-04; `snapshot.rs`' `SNAPSHOT_FORMAT` 8 — this test was
/// 17b cut M1a.5's `m1a5_a_resumed_landing_finds_no_scratch_and_clears_its_pending_row`,
/// whose premise, a landing that finds nothing after a resume, the cut
/// retires). Think 0 answers `Startup` and a memory; the snapshot is taken
/// with it in flight: the encoded row carries `pending` with the answer (the
/// think, its millisecond, the three epochs, the intents and the memory), the
/// decoded rows equal the encoded ones, and the decoded arena — on the
/// default runner, which is never asked to think again here — LANDS the
/// carried answer at its millisecond: `pending` taken off the row, the
/// memory the answer's, the `startup` line on the tape at the landing's
/// millisecond.
#[test]
fn m3_a_resumed_landing_lands_the_carried_answer() {
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
        vec![Says {
            memory: Some(b"{\"fixture\":314}".to_vec()),
            ..says(0, vec![IntentKind::Startup])
        }],
    )]);
    let think = think_at(RED, 0);
    let _ = rig.until(think + SUBSTEP, |arena| pending(arena, RED, 0));
    if let Some(state) = rig.arena.minds.get_mut(&RED) {
        state.ranges.insert(ContactId(77), 450);
    }
    let answer = rig
        .arena
        .minds
        .get(&RED)
        .and_then(|state| state.pending.clone())
        .expect("think 0 is in flight at the encode");
    assert_eq!(
        (answer.think_seq, answer.t_ms),
        (0, think),
        "the think and its millisecond"
    );
    assert_eq!(answer.intents, vec![IntentKind::Startup], "its intents");
    assert_eq!(answer.memory, b"{\"fixture\":314}".to_vec(), "its memory");
    assert_eq!(
        answer.epochs,
        Channel::ALL.map(|channel| rig.arena.epochs.current(RED, channel)),
        "its three epochs"
    );
    assert!(rig.lines("startup", RED).is_empty(), "not landed yet");
    let snapshot = Snapshot::encode(&rig.arena).expect("the arena encodes");
    let carried = serde_json::from_slice::<serde_json::Value>(snapshot.bytes())
        .expect("a state")
        .get("minds")
        .and_then(|minds| minds.get(RED.0.to_string()))
        .and_then(|row| row.get("pending"))
        .cloned();
    assert_eq!(
        carried.and_then(|pending| serde_json::from_value(pending).ok()),
        Some(answer.clone()),
        "the encoded row carries the answer"
    );
    let decoded = Snapshot::decode(&snapshot.document, (*rig.arena.definition).clone())
        .expect("the document decodes");
    assert_eq!(decoded.minds, rig.arena.minds, "the rows came back");
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
        "the landing took the answer off its row"
    );
    assert_eq!(
        resumed.lines("startup", RED),
        vec![think + SUBSTEP],
        "and landed it at the landing's millisecond"
    );
    let state: MindState = resumed.arena.minds.get(&RED).cloned().unwrap_or_default();
    assert_eq!(
        state.memory,
        b"{\"fixture\":314}".to_vec(),
        "the memory is the answer's"
    );
}

// ---------------------------------------------------------------------------
// The landing's edges: a wreck, the mind's own helm, a refusal's numbers
// ---------------------------------------------------------------------------

/// **A wreck cancels its mind's thinks** (17b cut M1a.5; `damage.rs`'
/// `Arena::destroy_unit`). The mind drives (think 1's `DRIVE`, so a motor tick
/// is on the heap) and has think 2 in flight — its answer on the row and its
/// landing on the heap — when the hull is destroyed: no think, landing or motor
/// tick of it is left on the heap, the row's `pending` (the answer) is cleared
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
    assert!(in_flight(&rig.arena, RED), "the precondition");
    assert!(
        rows_of(&rig.arena, RED).3.contains(&"mind_land")
            && rows_of(&rig.arena, RED).3.contains(&"motor_tick"),
        "the precondition: a landing and a motor tick on the heap"
    );
    let now = rig.arena.now_ms;
    let mut journal: Vec<tacsim::arena::perception::Notice> = Vec::new();
    rig.arena.destroy_unit(RED, now, None, &mut journal);
    let (row, scratch, _, kinds) = rows_of(&rig.arena, RED);
    assert!(kinds.is_empty(), "the wreck's due kinds: {kinds:?}");
    assert!(!scratch, "the pending answer went with the landing");
    assert!(row, "the row stays");
    assert_eq!(
        rig.arena
            .minds
            .get(&RED)
            .and_then(|state| state.pending.clone()),
        None,
        "and names no think in flight"
    );
    let asked = mind.seen().len();
    let mut thinks = 0_u32;
    while rig.arena.now_ms < now + 3_000 {
        thinks += rig.pulse(&[], &[]).motor.thinks;
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
        !rows_of(&rig.arena, RED).3.contains(&"motor_tick"),
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
    let mut counters = MotorCounters::default();
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
        counters.absorb(&out.motor);
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
        rows_of(&arena, RED).3.contains(&"motor_tick"),
        "the precondition: its tick is on the heap"
    );
    let release = control(
        Channel::Helm,
        arena.epochs.current(RED, Channel::Helm) + 1,
        ControlState::Released,
        Some(ObjId(1_000)),
    );
    apply_control(&mut arena, &release);
    // 17b cut M4.3: the machine is a mind; its motor row is its first
    // landing's, so none stands at the release.
    assert_eq!(
        rows_of(&arena, RED),
        (true, false, false, vec!["mind_think"]),
        "the mind's row and think, and nothing of the pilot's drive"
    );
    assert_eq!(
        arena.motors.row(RED),
        None,
        "no motor row until the mind's first landing"
    );

    let beats = [
        (0, Beat::Order(IntentKind::Startup)),
        (5_000, Beat::Order(drive)),
        (10_000, Beat::Control(ControlState::Released)),
    ];
    let (slow, after) = seat_run(5_000, 30_000, &beats);
    assert_eq!(
        after.unit(RED).map(|unit| unit.controller),
        Some(Controller::Mind),
        "the mind flies it"
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
    // Cards and ROM are captured at custody. Changing ArenaConfig alone must
    // not replace the immutable Line profile admitted by the M6 runtime.
    let terms = tacsim::match_::SealTerms {
        card: tacsim::arena::line(&reflex),
        rom_cfg: reflex.clone(),
        ..tacsim::match_::SealTerms::fixture()
    };
    let seeing = |mut unit: CustodyUnit| {
        unit.sensor = Some(SensorProfile {
            max_range_cells: 10,
            ..sight()
        });
        unit.reseal(&terms)
            .expect("the fixture's selected doctrine and ROM seal");
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
                Arming::Rules { .. } | Arming::Remove { .. } => {
                    panic!("Standard emits shorthand armings")
                }
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
        let slots: Vec<Option<FireCondition>> = rig
            .arena
            .conditions
            .slots(RED)
            .iter()
            .map(|rule| {
                rule.as_ref()
                    .and_then(tacsim::arena::conditions::Rule::fire)
                    .cloned()
            })
            .collect();
        let arms: Vec<Option<FireCondition>> = armings(&result.intents)
            .into_iter()
            .filter_map(|arming| match arming {
                Arming::Arm(cond) => Some(Some(cond.clone())),
                Arming::Disarm { .. } => None,
                Arming::Rules { .. } | Arming::Remove { .. } => {
                    panic!("Standard emits shorthand armings")
                }
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
    let mut counters = MotorCounters::default();
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
        counters.absorb(&out.motor);
        hashes.extend(arena.substep_hashes.iter().copied());
        if arena.now_ms >= 40_000 || out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
    }
    let events: Vec<(i64, String, u64)> = events(&mut arena, &mut bytes)
        .into_iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    // 17b cut M2.7: held to the native run under `support::under_the_rhai_runner`.
    support::note_run(&events, &hashes);
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
                && state_of(result) == MachineState::Patrol),
        "it holds under HOLD"
    );
    assert!(
        kept.iter().any(|(_, t_ms, view, result)| *t_ms >= 20_000
            && view
                .order
                .as_ref()
                .is_some_and(|order| order.objective == MindObjective::Withdraw)
            && state_of(result) == MachineState::Withdraw
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

/// **T0's first decide window on the (j) field, recorded** (17b cut M4.5a;
/// Q16(a)): the `startup`, `locked` and `fired` lines of the two reds, `(t_ms,
/// word, unit)`, that [`m1a6_the_stock_mind_is_t0_on_the_first_decide_window`]
/// read off `the_j_field(Controller::Reflex)` stepped one substep a pulse to
/// 5,000 ms — recorded at the cut that retired T0, where the test had run it.
/// Its one shot (3,482) was unattributed: no `by`, no slot.
const M1A6_T0_ACTIONS: [(i64, &str, u32); 4] = [
    (4, "startup", 6),
    (482, "startup", 4),
    (3_482, "locked", 4),
    (3_482, "fired", 4),
];

/// **T0's helm after every pulse of that window, recorded** (17b cut M4.5a;
/// Q16(a)): each red's `(t_ms, unit, desired heading, desired speed, cell,
/// pose)` after each one-substep pulse from 0 to 5,000 ms — the [`Helm`] rows
/// of the same reflex run, the same at every pass (native, under the twin,
/// under the Rhai runner). T0 first drives #6 at 750 and #4 at 1,000.
const M1A6_T0_HELM: [HelmRow; 42] = [
    (0, 4, 0, 0, (6, 8), (6_500, 8_500)),
    (0, 6, 0, 0, (6, 10), (6_500, 10_500)),
    (250, 4, 0, 0, (6, 8), (6_500, 8_500)),
    (250, 6, 0, 0, (6, 10), (6_500, 10_500)),
    (500, 4, 0, 0, (6, 8), (6_500, 8_500)),
    (500, 6, 0, 0, (6, 10), (6_500, 10_500)),
    (750, 4, 0, 0, (6, 8), (6_500, 8_500)),
    (750, 6, 225_000, 1_000, (6, 10), (6_500, 10_500)),
    (1_000, 4, 270_000, 1_000, (6, 8), (6_500, 8_500)),
    (1_000, 6, 225_000, 1_000, (6, 10), (6_477, 10_522)),
    (1_250, 4, 270_000, 1_000, (6, 8), (6_477, 8_522)),
    (1_250, 6, 221_838, 1_000, (6, 10), (6_415, 10_522)),
    (1_500, 4, 268_623, 1_000, (6, 8), (6_415, 8_522)),
    (1_500, 6, 221_838, 1_000, (6, 10), (6_349, 10_455)),
    (1_750, 4, 268_623, 1_000, (6, 8), (6_321, 8_519)),
    (1_750, 6, 221_588, 1_000, (6, 10), (6_265, 10_362)),
    (2_000, 4, 268_683, 1_000, (6, 8), (6_196, 8_516)),
    (2_000, 6, 221_588, 1_000, (6, 10), (6_162, 10_245)),
    (2_250, 4, 268_683, 1_000, (6, 8), (6_040, 8_513)),
    (2_250, 6, 221_592, 1_000, (6, 10), (6_037, 10_105)),
    (2_500, 4, 269_661, 1_000, (5, 8), (5_853, 8_508)),
    (2_500, 6, 221_592, 1_000, (5, 9), (5_892, 9_941)),
    (2_750, 4, 269_661, 1_000, (5, 8), (5_634, 8_507)),
    (2_750, 6, 224_330, 1_000, (5, 9), (5_726, 9_755)),
    (3_000, 4, 269_611, 1_000, (5, 8), (5_384, 8_506)),
    (3_000, 6, 224_330, 1_000, (5, 9), (5_551, 9_576)),
    (3_250, 4, 269_611, 1_000, (5, 8), (5_134, 8_504)),
    (3_250, 6, 224_354, 1_000, (5, 9), (5_377, 9_397)),
    (3_500, 4, 269_611, 0, (4, 8), (4_884, 8_502)),
    (3_500, 6, 224_354, 1_000, (5, 9), (5_202, 9_218)),
    (3_750, 4, 269_611, 0, (4, 8), (4_665, 8_501)),
    (3_750, 6, 224_355, 1_000, (5, 9), (5_027, 9_039)),
    (4_000, 4, 90_000, 0, (4, 8), (4_478, 8_500)),
    (4_000, 6, 224_355, 1_000, (5, 9), (5_000, 9_000)),
    (4_250, 4, 90_000, 0, (4, 8), (4_368, 8_388)),
    (4_250, 6, 225_000, 1_000, (5, 9), (5_000, 9_000)),
    (4_500, 4, 87_355, 0, (4, 8), (4_369, 8_263)),
    (4_500, 6, 225_000, 1_000, (5, 9), (5_000, 9_000)),
    (4_750, 4, 87_355, 0, (4, 8), (4_435, 8_197)),
    (4_750, 6, 225_000, 1_000, (5, 9), (5_000, 9_000)),
    (5_000, 4, 86_545, 0, (4, 8), (4_498, 8_198)),
    (5_000, 6, 225_000, 1_000, (5, 9), (5_000, 9_000)),
];

/// One recorded [`Helm`]: `(t_ms, unit, heading_mdeg, speed_cms, cell,
/// pose)`.
type HelmRow = (i64, u32, u32, i32, (u16, u16), (i64, i64));

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
///
/// **17b cut M4.5a** (the twin's T0 column retired; Q16(a)): T0's side of
/// the comparison is no longer run. Its window — the four actions and the
/// forty-two helm rows `the_j_field(Controller::Reflex)` produced, natively
/// and under the twin and the Rhai runner alike (T0 flew no mind) — was
/// recorded once at this cut, while `Controller::Reflex` still compiled and
/// with every assertion below passing against the live run, as
/// [`M1A6_T0_ACTIONS`] and [`M1A6_T0_HELM`]; T1 is held to the record
/// exactly as it was held to the run, every claim kept (the startups on T0's
/// staggered tick are now checked on the record). The record is the (j)
/// field's first window action for action, the one pin of it Gate B never
/// carried.
#[test]
fn m1a6_the_stock_mind_is_t0_on_the_first_decide_window() {
    // One substep a pulse, so the helm is read after every landing.
    let lines = |controller: Controller| {
        let mut arena = the_j_field(controller);
        let mut helm: Vec<Helm> = Vec::new();
        let mut wall = arena.now_ms;
        let mut bytes: Vec<u8> = Vec::new();
        let mut hashes: Vec<u64> = Vec::new();
        loop {
            let out = arena.step(wall, &[], &[]);
            for chunk in &out.ledger_chunks {
                bytes.extend_from_slice(&chunk.bytes);
            }
            hashes.extend(arena.substep_hashes.iter().copied());
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
        // 17b cut M2.7: each window's event list and hash sequence held to the
        // native run's under `support::under_the_rhai_runner`.
        let noted: Vec<(i64, String, u64)> = journal
            .iter()
            .map(|event| {
                (
                    event.t_ms,
                    serde_json::to_string(&event.kind).unwrap_or_default(),
                    event.hash,
                )
            })
            .collect();
        support::note_run(&noted, &hashes);
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
    // 17b cut M4.5a: T0 is its recorded window ([`M1A6_T0_ACTIONS`],
    // [`M1A6_T0_HELM`]); only T1 is run.
    let (t1, t1_helm, t1_shots) = lines(Controller::Mind);
    let t0: Vec<(i64, String, u32)> = M1A6_T0_ACTIONS
        .iter()
        .map(|&(t_ms, word, unit)| (t_ms, word.to_owned(), unit))
        .collect();
    let t0_helm: Vec<Helm> = M1A6_T0_HELM
        .iter()
        .map(|&(t_ms, unit, heading_mdeg, speed_cms, cell, pose)| Helm {
            t_ms,
            unit,
            heading_mdeg,
            speed_cms,
            cell: Some(cell),
            pose,
        })
        .collect();
    // T1's shots are each cited: a fresh lock's shot `by: "mind"` at the
    // landing one substep after a lattice think, or the slot's own `by:
    // "cond"` — at a `WeaponReady`, a pass or a landing — both naming S1.
    // (T0's one shot, at 3,482, was a machine's: unattributed,
    // `FiredBy::Machine`, never gated — asserted when it was recorded.)
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
    let mut counters = MotorCounters::default();
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
        counters.absorb(&out.motor);
        hashes.extend(arena.substep_hashes.iter().copied());
        signals.extend(out.deliberation);
        if arena.now_ms >= 30_000 || out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
    }
    let events: Vec<(i64, String, u64)> = events(&mut arena, &mut bytes)
        .into_iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    // 17b cut M2.7: held to the native run under `support::under_the_rhai_runner`.
    support::note_run(&events, &hashes);
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

/// One squad signal as the l2 lens reads it (17b cut M4.3): its arena
/// millisecond, its reason, and each member's unit and doctrine state.
type SignalRead = (i64, SignalReason, Vec<(ObjId, MachineState)>);

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
    }
    let read: Vec<SignalRead> = reds
        .iter()
        .map(|signal| {
            (
                signal.at_ms,
                signal.reason,
                signal
                    .members
                    .iter()
                    .map(|member| (member.unit, member.state))
                    .collect(),
            )
        })
        .collect();
    // 17b cut M4.3: a mind member reads its mind's cached doctrine state
    // (`Arena::machine_state`) — a doctrine STATE word, never the memory
    // (D-UM-49 kept): the opening signal, before any think has landed, reads
    // `Startup` for every hull; the expiry's reads the leader engaged and
    // its wingmen on patrol.
    assert_eq!(
        read,
        vec![
            (
                0,
                SignalReason::NoOrder,
                vec![
                    (RED, MachineState::Startup),
                    (RED2, MachineState::Startup),
                    (RED3, MachineState::Startup),
                ],
            ),
            (
                30_000,
                SignalReason::OrderExpired,
                vec![
                    (RED, MachineState::Engage),
                    (RED2, MachineState::Patrol),
                    (RED3, MachineState::Patrol),
                ],
            ),
        ],
        "a mind member reads its mind's doctrine state at the signal"
    );
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

#[test]
fn l2_a_self_charged_support_expires_once_as_engage() {
    let mut rig = Rig::new(charge_field());
    let first = intent(
        &rig.arena,
        RED,
        IntentKind::Order(Box::new(order(
            RED_ONE,
            Objective::Support { of: RED },
            Some(RED2),
            1_000,
        ))),
        1,
    );
    let _ = rig.pulse(&[], &[first]);
    let held = rig.arena.frame_order(RED).expect("the self-charge landed");
    assert_eq!(held.objective, MindObjective::Engage { target: None });
    let deadline = held.expires_ms;
    assert_eq!(rig.arena.minds[&RED].order_charge, None);
    assert!(rig.lines("order_expired", RED).is_empty());

    let _ = rig.until(deadline + 3 * STOCK_THINK_MS, |arena| {
        arena
            .minds
            .get(&RED)
            .is_some_and(|state| state.order.is_none())
    });
    assert!(
        rig.arena.now_ms >= deadline,
        "the self-charge did not expire early"
    );
    let state = &rig.arena.minds[&RED];
    assert_eq!(state.order_issuer, None);
    assert_eq!(state.order_charge, None);
    let after = rig.arena.now_ms + 3 * STOCK_THINK_MS;
    let _ = rig.until(after + SUBSTEP, |arena| arena.now_ms >= after);
    let expired: Vec<_> = rig
        .journal()
        .into_iter()
        .filter(|event| event.kind.as_str() == "order_expired")
        .collect();
    assert_eq!(expired.len(), 1);
    assert!(expired[0].t_ms >= deadline);
    let line = serde_json::to_value(&expired[0].kind).expect("expiry event");
    assert_eq!(line["unit"], serde_json::json!(RED.0));
    assert_eq!(line["squad"], serde_json::json!("red-1"));
    assert_eq!(line["objective"], serde_json::json!("engage"));
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
            && state_of(result) == MachineState::Approach
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
        .find(|(unit, _, _, result)| *unit == RED && state_of(result) == MachineState::Approach)
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
            .all(|(_, _, _, result)| state_of(result) != MachineState::Approach),
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
    let mut counters = MotorCounters::default();
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
        counters.absorb(&out.motor);
        hashes.extend(arena.substep_hashes.iter().copied());
        if arena.now_ms >= until_ms || out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
    }
    let events: Vec<(i64, String, u64)> = events(&mut arena, &mut bytes)
        .into_iter()
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_string(&event.kind).unwrap_or_default(),
                event.hash,
            )
        })
        .collect();
    // 17b cut M2.7: held to the native run under `support::under_the_rhai_runner`.
    support::note_run(&events, &hashes);
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
fn state_after(result: &ThinkResult) -> MachineState {
    tacsim::arena::stock_mind::StockMemory::from(&tacsim::arena::mind::decode_memory(
        &result.memory,
    ))
    .state
}

/// The lattice milliseconds of `unit`'s thinks that left it withdrawing.
fn withdrawn_thinks(kept: &[Kept], unit: ObjId) -> Vec<i64> {
    kept.iter()
        .filter(|(on, _, _, result)| *on == unit && state_after(result) == MachineState::Withdraw)
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
    for unit in arena.minds.keys() {
        assert_eq!(
            row_card(&arena, *unit),
            Some(tacsim::arena::line(&doctrine)),
            "#{}",
            unit.0
        );
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
                        Arming::Rules { .. } | Arming::Remove { .. } => {
                            panic!("Standard emits shorthand armings")
                        }
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
/// and at one cadence: a change staged for a hull that is not a mind (a
/// pilot's, since 17b cut M4.5a; a reflex hull before it) journals
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
        serde_json::from_value(line["stock"].clone()).expect("the diagnostic stock adapter");
    assert_eq!(card, preset_on_field("rearguard"));
    assert_eq!(row_card(&arena, RED).as_ref(), Some(&card));
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

    // A hull that is not a mind: refused `not_mind`, nothing changed. (17b
    // cut M4.5a: a pilot's hull nobody sits in, where it was a reflex hull —
    // the refusal reads `controller == Mind` and a mind row, so any other
    // controller is the same case.)
    let (run, _, arena) = card_run(
        the_j_field(Controller::Pilot),
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
    assert_eq!(row_card(&arena, RED), Some(preset_on_field("line")));

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
        row_card(&arena, RED),
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
    assert_eq!(row_card(&arena, RED), Some(preset_on_field("sniper")));

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
        row_card(&arena, RED).map(|card| card.min_odds_permille),
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
    let card = row_card(&arena, RED2).expect("the unarmed mind's row");
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
    let runtime = &arena.profiles[&RED];
    let card = runtime.card.clone();
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
    let think = |view: &MindView,
                 memory: &StockMemory,
                 t_ms: i64|
     -> (Vec<IntentKind>, StockMemory) {
        let bytes =
            tacsim::arena::mind::encode_memory(&tacsim::arena::mind::Memory::from(memory.clone()));
        let result = mind.think(&ThinkJob {
            unit: RED,
            think_seq: 1,
            t_ms,
            lands_ms: t_ms + SUBSTEP,
            seed: arena.seed,
            dropped: false,
            profile: &tacsim::arena::core::ClassTable::STOCK.row(tacsim::arena::core::CoreClass::I),
            view: &Arc::new(view.clone()),
            memory: &bytes,
            map: &arena.definition,
            card: &card,
            program: &runtime.program,
            profile_token: runtime.token(),
        });
        // 17b cut M2.7: held to the native think under
        // `support::under_the_rhai_runner`.
        support::note_think(RED, t_ms, &result);
        (
            result.intents,
            StockMemory::from(&tacsim::arena::mind::decode_memory(&result.memory)),
        )
    };
    let start = StockMemory {
        state: MachineState::Patrol,
        goal: Some(failed),
        waypoint: Some(1),
        unheld_since_ms: Some(0),
        ..StockMemory::default()
    };
    // 1. The order ended by the failed search.
    let mut view = base.clone();
    view.contacts.clear();
    view.order = Some(advance(failed, 20_000));
    view.motor = Some(tacsim::arena::step::FrameMotor {
        requested_goal: failed,
        outcome: Some(DriveOutcome::NoRoute),
    });
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
///
/// **The cut's one declared re-bless** (17b cut M3.6: `mind_sealed` and
/// `sortie_closed` shift the folded ledger sequence (`hash.rs:897`) of every
/// match; the mind row's fold gains D-UM-23's answer, `dropped` and the seal
/// digest; the chassis' core consequence words ride the hit lines and the
/// pilot's sentence; every other byte's encoding is unchanged). Re-recorded
/// here from 47 lines to 50: three `mind_sealed` lines after `match_started`
/// (the record carries no hash; the run does not end and no turret dies), every
/// other line byte-equal.
/// M5rules updates only the three Standard source prefixes from 102cc7d27eb411c4
/// to 92987a0c26bc442e (the bounded complete-rule API and Standard adapter).
/// The 50 event timestamps and every other payload byte remain identical.
/// M6 updates those three source prefixes to 9b330c0f5ccd8610 and their card
/// identities to the full authored schema/value/rule digest. A field-by-field
/// comparison of all 50 fresh event rows found no other payload/time changes.
const M1B5_TWO_T1_EVENTS: &[(i64, &str)] = &[
    (0, "{\"kind\":\"match_started\"}"),
    (
        0,
        "{\"kind\":\"mind_sealed\",\"unit\":4,\"sortie\":4,\"mind\":\"Standard Doctrine\",\"source\":\"d318e09bd821a0d9\",\"card_digest\":\"f2491a868a5707936f2abf1db211ed6bba20971b2c364e31433bafb33fc07f90\",\"class\":\"I\",\"governed\":true,\"licence\":\"doctrine\"}",
    ),
    (
        0,
        "{\"kind\":\"mind_sealed\",\"unit\":6,\"sortie\":6,\"mind\":\"Standard Doctrine\",\"source\":\"d318e09bd821a0d9\",\"card_digest\":\"f2491a868a5707936f2abf1db211ed6bba20971b2c364e31433bafb33fc07f90\",\"class\":\"I\",\"governed\":true,\"licence\":\"doctrine\"}",
    ),
    (
        0,
        "{\"kind\":\"mind_sealed\",\"unit\":9,\"sortie\":9,\"mind\":\"Standard Doctrine\",\"source\":\"d318e09bd821a0d9\",\"card_digest\":\"f2491a868a5707936f2abf1db211ed6bba20971b2c364e31433bafb33fc07f90\",\"class\":\"I\",\"governed\":true,\"licence\":\"doctrine\"}",
    ),
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
        10_000,
        "{\"kind\":\"position_summary\",\"units\":[[4,4653,8350,0,88000],[6,2956,8673,0,249288],[9,9500,8500,0,0]]}",
    ),
    (
        11_732,
        "{\"kind\":\"fired\",\"unit\":4,\"weapon\":\"cannon\",\"shot_seq\":3,\"target\":9,\"range_cm\":4849,\"rounds\":1,\"by\":\"cond\",\"slot\":1}",
    ),
    (
        11_765,
        "{\"kind\":\"impact\",\"shot_seq\":3,\"unit\":4,\"target\":9}",
    ),
    (
        11_765,
        "{\"kind\":\"hit\",\"shot_seq\":3,\"unit\":4,\"target\":9,\"section\":\"hull\",\"facet\":\"front\",\"armor_section\":\"hull\",\"rounds\":1,\"damage\":120,\"armor_left\":0,\"internals_left\":0,\"cause\":\"shot:3\"}",
    ),
    (
        15_000,
        "{\"kind\":\"position_summary\",\"units\":[[4,4653,8350,0,88000],[6,5627,7497,0,89847],[9,9500,8500,0,0]]}",
    ),
    (
        15_732,
        "{\"kind\":\"fired\",\"unit\":4,\"weapon\":\"cannon\",\"shot_seq\":4,\"target\":9,\"range_cm\":4849,\"rounds\":1,\"by\":\"cond\",\"slot\":1}",
    ),
    (
        15_765,
        "{\"kind\":\"impact\",\"shot_seq\":4,\"unit\":4,\"target\":9}",
    ),
    (
        15_765,
        "{\"kind\":\"hit\",\"shot_seq\":4,\"unit\":4,\"target\":9,\"section\":\"hull\",\"facet\":\"front\",\"armor_section\":\"hull\",\"rounds\":1,\"damage\":120,\"armor_left\":0,\"internals_left\":0,\"cause\":\"shot:4\"}",
    ),
    (
        19_732,
        "{\"kind\":\"fired\",\"unit\":4,\"weapon\":\"cannon\",\"shot_seq\":5,\"target\":9,\"range_cm\":4849,\"rounds\":1,\"by\":\"cond\",\"slot\":1}",
    ),
    (
        19_765,
        "{\"kind\":\"impact\",\"shot_seq\":5,\"unit\":4,\"target\":9}",
    ),
    (
        19_765,
        "{\"kind\":\"hit\",\"shot_seq\":5,\"unit\":4,\"target\":9,\"section\":\"hull\",\"facet\":\"front\",\"armor_section\":\"hull\",\"rounds\":1,\"damage\":120,\"armor_left\":0,\"internals_left\":0,\"cause\":\"shot:5\"}",
    ),
    (
        20_000,
        "{\"kind\":\"position_summary\",\"units\":[[4,4653,8350,0,88000],[6,10627,7500,0,90000],[9,9500,8500,0,0]]}",
    ),
    (
        23_732,
        "{\"kind\":\"fired\",\"unit\":4,\"weapon\":\"cannon\",\"shot_seq\":6,\"target\":9,\"range_cm\":4849,\"rounds\":1,\"by\":\"cond\",\"slot\":1}",
    ),
    (
        23_765,
        "{\"kind\":\"impact\",\"shot_seq\":6,\"unit\":4,\"target\":9}",
    ),
    (
        23_765,
        "{\"kind\":\"hit\",\"shot_seq\":6,\"unit\":4,\"target\":9,\"section\":\"hull\",\"facet\":\"front\",\"armor_section\":\"hull\",\"rounds\":1,\"damage\":120,\"armor_left\":0,\"internals_left\":0,\"cause\":\"shot:6\"}",
    ),
    (
        25_000,
        "{\"kind\":\"position_summary\",\"units\":[[4,4653,8350,0,88000],[6,12843,7339,0,295055],[9,9500,8500,0,0]]}",
    ),
    (
        27_732,
        "{\"kind\":\"fired\",\"unit\":4,\"weapon\":\"cannon\",\"shot_seq\":7,\"target\":9,\"range_cm\":4849,\"rounds\":1,\"by\":\"cond\",\"slot\":1}",
    ),
    (
        27_765,
        "{\"kind\":\"impact\",\"shot_seq\":7,\"unit\":4,\"target\":9}",
    ),
    (
        27_765,
        "{\"kind\":\"hit\",\"shot_seq\":7,\"unit\":4,\"target\":9,\"section\":\"hull\",\"facet\":\"front\",\"armor_section\":\"hull\",\"rounds\":1,\"damage\":120,\"armor_left\":0,\"internals_left\":0,\"cause\":\"shot:7\"}",
    ),
    (
        30_000,
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
    (6, 4, 6098099545541555949),
    (4, 482, 6098099545541555949),
    (6, 1004, 9982644946626122887),
    (4, 1482, 5519612912102104609),
    (6, 2004, 9982644946626122887),
    (4, 2482, 5519612912102104609),
    (6, 3004, 4267685524575424567),
    (4, 3482, 14727923680749643942),
    (6, 4004, 4267685524575424567),
    (4, 4482, 16951036639100933384),
    (6, 5004, 4267685524575424567),
    (4, 5482, 17332774373539806302),
    (6, 6004, 4267685524575424567),
    (4, 6482, 6632271311621113940),
    (6, 7004, 4267685524575424567),
    (4, 7482, 13718269286102459830),
    (6, 8004, 4267685524575424567),
    (4, 8482, 12618536069372510200),
    (6, 9004, 4267685524575424567),
    (4, 9482, 10325748119616840334),
    (6, 10004, 1057625593564455519),
    (4, 10482, 549518417525068336),
    (6, 11004, 1057625593564455519),
    (4, 11482, 2924154669566716156),
    (6, 12004, 1057625593564455519),
    (4, 12482, 14209113274094202884),
    (6, 13004, 1057625593564455519),
    (4, 13482, 3409402830633305464),
    (6, 14004, 1057625593564455519),
    (4, 14482, 11730236216475113288),
    (6, 15004, 1057625593564455519),
    (4, 15482, 10104064967216821004),
    (6, 16004, 1057625593564455519),
    (4, 16482, 9869136410499005524),
    (6, 17004, 1057625593564455519),
    (4, 17482, 1832347342605900320),
    (6, 18004, 1057625593564455519),
    (4, 18482, 16983423770728313920),
    (6, 19004, 1057625593564455519),
    (4, 19482, 5979536445247340812),
    (6, 20004, 1057625593564455519),
    (4, 20482, 12372907226072322348),
    (6, 21004, 1057625593564455519),
    (4, 21482, 15902047395897125040),
    (6, 22004, 1057625593564455519),
    (4, 22482, 11636299758901054172),
    (6, 23004, 9408727930253721049),
    (4, 23482, 1147831420516726480),
    (6, 24004, 9408727930253721049),
    (4, 24482, 5132644807008374204),
    (6, 25004, 9408727930253721049),
    (4, 25482, 4246149452316905384),
    (6, 26004, 9408727930253721049),
    (4, 26482, 612716631066148972),
    (6, 27004, 9408727930253721049),
    (4, 27482, 17440599378261693272),
    (6, 28004, 9408727930253721049),
    (4, 28482, 965198466461433196),
    (6, 29004, 9408727930253721049),
    (4, 29482, 11224957993845209904),
];
const M2_TWO_T1_FIRST_MEMORY: &[u8] = b"{\"from_waypoint\":null,\"goal\":null,\"hold_retry_ms\":null,\"hunt\":null,\"ignore_until_ms\":null,\"last_plot\":null,\"no_line\":false,\"ordered\":false,\"quiet_since_ms\":null,\"rallied\":false,\"stand_until_ms\":null,\"state\":0,\"target\":null,\"unheld_since_ms\":null,\"waypoint\":null}";
const M2_TWO_T1_LAST_MEMORY: &[u8] = b"{\"from_waypoint\":null,\"goal\":null,\"hold_retry_ms\":null,\"hunt\":null,\"ignore_until_ms\":null,\"last_plot\":[9,8,29482],\"no_line\":false,\"ordered\":false,\"quiet_since_ms\":null,\"rallied\":false,\"stand_until_ms\":null,\"state\":3,\"target\":5852833454602713067,\"unheld_since_ms\":29482,\"waypoint\":0}";
const M2_UNARMED_MEMORY_DIGESTS: &[(u32, i64, u64)] = &[
    (4, 482, 6098099545541555949),
    (4, 1482, 5519612912102104609),
    (4, 2482, 5519612912102104609),
    (4, 3482, 4267685524575424567),
    (4, 4482, 4267685524575424567),
    (4, 5482, 4267685524575424567),
    (4, 6482, 1057625593564455519),
    (4, 7482, 4517132980298021411),
    (4, 8482, 4517132980298021411),
    (4, 9482, 4517132980298021411),
    (4, 10482, 4937722195351905967),
    (4, 11482, 5362147462018813910),
    (4, 12482, 8117936653531440353),
    (4, 13482, 12310400826759644232),
    (4, 14482, 1693821991455957731),
    (4, 15482, 15911883857477556330),
    (4, 16482, 9346234582196756437),
    (4, 17482, 5282301909377335164),
    (4, 18482, 7268762365464231831),
    (4, 19482, 11248367180658170110),
    (4, 20482, 2732820247106746154),
    (4, 21482, 14698838495418361507),
    (4, 22482, 2131862722736019004),
    (4, 23482, 6951781811658369941),
    (4, 24482, 5008651699920213398),
    (4, 25482, 3470323195052966767),
    (4, 26482, 11858471496046690056),
    (4, 27482, 16834059077288617633),
    (4, 28482, 11879328287931271378),
    (4, 29482, 14927110418657134475),
    (4, 30482, 729294553960127493),
    (4, 31482, 14188817823236871084),
    (4, 32482, 1255869739264752595),
    (4, 33482, 6074561442570174234),
    (4, 34482, 13593159480022424209),
    (4, 35482, 7637205705997796920),
    (4, 36482, 16908779923569432031),
    (4, 37482, 13003529768784768454),
    (4, 38482, 14525396527344763981),
    (4, 39482, 16129450518313177076),
];
const M2_UNARMED_FIRST_MEMORY: &[u8] = b"{\"from_waypoint\":null,\"goal\":null,\"hold_retry_ms\":null,\"hunt\":null,\"ignore_until_ms\":null,\"last_plot\":null,\"no_line\":false,\"ordered\":false,\"quiet_since_ms\":null,\"rallied\":false,\"stand_until_ms\":null,\"state\":0,\"target\":null,\"unheld_since_ms\":null,\"waypoint\":null}";
const M2_UNARMED_LAST_MEMORY: &[u8] = b"{\"from_waypoint\":0,\"goal\":[1,8],\"hold_retry_ms\":null,\"hunt\":null,\"ignore_until_ms\":null,\"last_plot\":null,\"no_line\":false,\"ordered\":true,\"quiet_since_ms\":null,\"rallied\":false,\"stand_until_ms\":null,\"state\":4,\"target\":null,\"unheld_since_ms\":39482,\"waypoint\":1}";

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
///    fifteen keys are ASCII words and it writes no string value (**17b cut
///    M4.3**: the fifteenth, `ordered`, moved every memory's bytes — the
///    mind pins' second declared move — and every digest, first and last
///    memory here were re-recorded once with it);
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
    counters: MotorCounters,
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
        counters: MotorCounters::default(),
        landings_after: Vec::new(),
        speed: None,
    };
    let pulse = |rig: &mut Rig, out: &mut StrikesRun| {
        let step = rig.pulse(&[], &[]);
        out.counters.absorb(&step.motor);
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
            in_flight(&rig.arena, RED)
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
/// * at each, NOTHING lands: no answer pending on the row (17b cut M3.2: the
///   answer rides `pending`, the scratch retired) and no landing on the heap, the
///   memory the last landed think left (`[1]`) kept, the notices the think
///   was handed back at the front of the buffer (think 1's refused pod switch
///   among them, so think 3 is handed think 2's notices again), the range
///   memory as it stood, `pending` clear — and the speed never ordered;
/// * `strikes` reads 1, 2, 3 and `mind_faults` 3 (no `mind_overflows`);
///   `ThinkJob::dropped` is false through think 4 and TRUE from think 5;
///   every job carries the match seed (`ThinkJob::seed`, D-UM-29: the key
///   of a mind's draws) — and a second arena on another seed hands that one;
///   the clean think 5 lands its memory and leaves `strikes` at 3 — the drop
///   is for the rest of the sortie (17b cut M3.2: the drop is the row's own
///   `dropped`, `Some(DropCause::Strikes)` from think 4, which the clean
///   think never clears; `strikes == 3` no longer IS the drop);
/// * **the baseline** — the same script answering SEVENTEEN speeds and no
///   fault word at thinks 2, 3 and 4 (M1a's overflow path), with red's
///   `strikes` written by hand after every pulse to the faulting run's figure
///   (and found already there: the overflow counts a strike) — hashes EQUAL
///   at every substep to the faulting run: the new fault path is M1a's
///   overflow path plus `strikes` and nothing else. The overflow run counts
///   `mind_overflows` 3 AND `mind_faults` 3;
/// * one event list and one hash sequence at 5,000, 1,000 and 500 ms — the
///   list carrying, since 17b cut M3.4, the three `mind_fault` lines (`ops`)
///   and the one `mind_dropped { strikes, fault: ops, ops: 1500 }`;
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
    assert_eq!(
        five.dropped,
        Some(tacsim::arena::mind::DropCause::Strikes),
        "the drop's own field (17b cut M3.2)"
    );
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
    // 17b cut M3.4 (D-UM-31, D-UM-32): the event list carries the four lines
    // — a `mind_fault` per faulted think with its word, and the drop beside
    // the third with that word and the row's budget. (Until this cut the
    // list held none of them.)
    let mind_lines: Vec<(i64, String)> = slow
        .events
        .iter()
        .filter(|(_, json, _)| json.contains("\"mind_fault\"") || json.contains("\"mind_dropped\""))
        .map(|(t_ms, json, _)| (*t_ms, json.clone()))
        .collect();
    let fault = |n: i64| {
        (
            think_at(RED, n),
            format!(r#"{{"kind":"mind_fault","unit":4,"think_seq":{n},"cause":"ops"}}"#),
        )
    };
    assert_eq!(
        mind_lines,
        vec![
            fault(2),
            fault(3),
            fault(4),
            (
                think_at(RED, 4),
                r#"{"kind":"mind_dropped","unit":4,"cause":"strikes","fault":"ops","ops":1500}"#
                    .to_owned()
            ),
        ]
    );
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
/// `CardError::Pair`'s sentence — the same text `@mind/live` wraps — and the
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
    assert_eq!(row_card(&arena, RED), Some(preset_on_field("line")));
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
    // The active card is captured at custody, not reconstructed from the
    // arena's current defaults. Seal the high pair on this fixture's hulls.
    let high_sealed = |mut unit: CustodyUnit| {
        unit.sensor = Some(SensorProfile {
            max_range_cells: 10,
            ..sight()
        });
        unit.reseal(&tacsim::match_::SealTerms {
            card: tacsim::arena::line(&high),
            rom_cfg: high.clone(),
            ..tacsim::match_::SealTerms::fixture()
        })
        .expect("the high pair seals");
        unit
    };
    let field = arena_on(
        vec![
            high_sealed(tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                Controller::Mind,
            )),
            high_sealed(tank(
                RED2,
                Team::Red,
                centre(6, 10),
                Vec::new(),
                Controller::Mind,
            )),
            presenting(high_sealed(tank(
                BLUE,
                Team::Blue,
                centre(9, 8),
                Vec::new(),
                Controller::Pilot,
            ))),
        ],
        &[BLUE],
        high.clone(),
    );
    assert_eq!(row_card(&field, RED), Some(tacsim::arena::line(&high)));
    let (run, _, arena) = card_run(
        field,
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
    assert_eq!(row_card(&arena, RED), Some(tacsim::arena::line(&high)));
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

// ---------------------------------------------------------------------------
// 17b cut M2.7: the T1 fixtures under the Rhai runner ALONE (doc 17 §17
// "Gate A", §22's M2 gate: "Gate A on the cross-tempo fixture"; 17b §1 (iii))
// ---------------------------------------------------------------------------
//
// The twin of 17b cut M2.6 flies T1's answer, so a twin-flown fixture is the
// native fixture by construction and proves Gate A only through the twin's
// own comparison. Here the fixtures that field T1 fly the Rhai Standard
// Doctrine ALONE (`support::with_rhai`: `tacsim::minds::standard_runner` on
// the field's own doctrine, nothing native beside it), so every answer the
// arena lands is T2's own — and each run must still be the native run's: one
// event list, one hash sequence (and, where a stock recorder keeps them, one
// sequence of thinks) at every cadence the fixture plays. Two shapes:
//
// * the dedicated siblings, which compare against the native run by hand —
//   [`m2_the_two_t1_fixture_under_the_rhai_runner`] (the two-T1 fixture at
//   all six drives, `M1B2_TWO_T1_FIRED` kept, the M1b.5 anchor),
//   [`m2_the_unarmed_stock_mind_under_the_rhai_runner_at_three_cadences`]
//   and the resumed fixture,
//   [`m2_a_resumed_field_flies_the_rhai_runner_again_after_the_decode`];
// * `support::under_the_rhai_runner` over a fixture body, which plays the
//   body natively and then on the Rhai runner and holds every run the body's
//   helpers note (`run`, `unarmed_run_on`, `lance_run`, `card_run`) and
//   every think `Stock` records to the native pass's, index by index.
//
// The fixtures that field no T1 (the scripted minds of 17b cuts M1a.5,
// M1b.0, M1b.1 and M1b.3's hint re-key, the faulting and pairing fixtures of
// M2.2) are not repeated: a `ScriptedMind` answers its script whoever runs.

/// **The two-T1 fixture on the Rhai runner alone** (17b cut M2.7):
/// [`stock_minds`] — the (j) field's two reds, the armed one finding,
/// classifying, locking, arming its slot and citing it — flown by T2 alone
/// at 5,000, 1,000 and 500 ms and on the task's wakes at the three periods,
/// each equal to the native run (events, the hash sequence, every think's
/// view and answer); [`two_t1_slots_and_citations`] passes on the T2-flown
/// run — [`M1B2_TWO_T1_FIRED`] kept — and its event list and answers are 17b
/// cut M1b.5's anchor ([`M1B5_TWO_T1_EVENTS`], [`M1B5_TWO_T1_INTENTS`]): the
/// answers recorded there are now the Rhai runner's.
#[test]
fn m2_the_two_t1_fixture_under_the_rhai_runner() {
    let (native, native_kept) = stock_minds(Drive::Every(5_000));
    support::under_the_rhai_runner("the two-T1 fixture", || {
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
            assert_eq!(other.events, native.events, "the event list at {drive:?}");
            assert_eq!(
                other.hashes, native.hashes,
                "the hash sequence at {drive:?}"
            );
            assert_eq!(other_kept, native_kept, "a think at {drive:?}");
        }
    });
}

/// **The unarmed fixture on the Rhai runner alone** (17b cut M2.7; D-UM-48):
/// [`unarmed_run`] — the weaponless stock mind that starts up, walks, holds
/// under `HOLD` and withdraws under `WITHDRAW` with a hostile in its sight —
/// with its stock recorder over `support::rhai_alone` on the field's
/// doctrine, so every answer landed is T2's own: at 5,000, 1,000 and 500 ms
/// the native run's event list, hash sequence and sequence of thinks, and
/// the recorder's runner prints as the Rhai one.
#[test]
fn m2_the_unarmed_stock_mind_under_the_rhai_runner_at_three_cadences() {
    let (native, native_kept, _) = unarmed_run(5_000);
    let cfg = ArenaConfig {
        reflex: reflex_cfg(),
        ..ArenaConfig::default()
    };
    for cadence in [5_000_i64, 1_000, 500] {
        let (run, kept, arena) = unarmed_run_on(cadence, Some(support::rhai_alone(&cfg)));
        assert!(
            format!("{:?}", arena.mind_runner).starts_with("Stock { inner: RhaiRunner"),
            "the recorder over the Rhai runner"
        );
        assert_eq!(run.events, native.events, "the event list at {cadence} ms");
        assert_eq!(
            run.hashes, native.hashes,
            "the hash sequence at {cadence} ms"
        );
        assert_eq!(kept, native_kept, "a think at {cadence} ms");
        assert!(!kept.is_empty());
    }
}

/// One run one substep a pulse from wall `wall` to `until_ms`: the ledger
/// bytes, every substep's hash (accumulated after every step) and the wall
/// of the next pulse. The tempo is pinned `Quiet` before every pulse, as
/// `tests/conditions.rs`' resumed run pins it. The (j) field is pinned
/// `Quiet` at construction (`arena_on`), so the uninterrupted run never lets
/// its clock evaluate a tempo; the snapshot carries the clock's tempo and its
/// signals (16b cut AF.1's `ClockState`) but deliberately not the pin, a test
/// lever (`Clock::from_state`'s head: "a test that pins pins again"). Without
/// the re-pin the decoded clock, holding the `Quiet` the pin held and the
/// engaged signals `evaluate` kept recording under it, journals `Quiet` to
/// `Engaged` one substep after the resume — the dropped pin, not a gap in the
/// snapshot: an unpinned match resumes hash-continuous.
fn stepped(arena: &mut Arena, mut wall: i64, until_ms: i64) -> (Vec<u8>, Vec<u64>, i64) {
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    while arena.now_ms < until_ms {
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let out = arena.step(wall, &[], &[]);
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        hashes.extend(arena.substep_hashes.iter().copied());
        wall = arena.now_ms + SUBSTEP;
        if out.ended.is_some() {
            break;
        }
    }
    (bytes, hashes, wall)
}

/// **A resumed field flies the Rhai runner again only because the fixture
/// says so** (17b cut M2.7; 17b §5's adversarial F11): the (j) field's two
/// stock minds flown by T2 alone (`support::with_rhai`), one substep a pulse
/// with the tempo pinned ([`stepped`]), encoded at the first pulse boundary
/// past 15,000 ms with no think in
/// flight (no `pending` row — since 17b cut M3.2 a pending answer would ride
/// the snapshot too, `tests/minds_rhai.rs`' gate fixture resumes across one)
/// and decoded. The decode re-installs `default_mind_runner`
/// — the decoded arena prints `NativeStockMind`, and since T2 ≡ T1 nothing
/// else would show a resumed fixture flying T1 — so the fixture re-applies
/// `support::with_rhai` and asserts the runner's `Debug` name holds
/// `RhaiRunner` before it steps on. **Why it still must, since 17b cut
/// M3.5**: these two hulls are sealed `native` (the fixture terms), and
/// `with_rhai` swaps the arena-wide FALLBACK under them — a lever no seal
/// records, so the decode's re-derived runners (empty) and the sealed
/// installer (which installs nothing for a `native` seal) cannot bring it
/// back. A hull sealed `scripted` needs no lever: its sibling
/// [`m3_a_decode_reinstalls_each_unit_s_runner_from_its_seal`] resumes one on
/// the production installer alone. To 30,000 ms the stitched run (the T2
/// field before the snapshot, the resumed T2 field after it) is the native
/// run's: one hash sequence, one ledger byte for byte, one final state hash.
#[test]
fn m2_a_resumed_field_flies_the_rhai_runner_again_after_the_decode() {
    const UNTIL_MS: i64 = 30_000;
    // The native run, uninterrupted.
    let mut native = the_j_field(Controller::Mind);
    assert!(format!("{:?}", native.mind_runner).starts_with("NativeStockMind"));
    let start = native.now_ms;
    let (mut native_bytes, native_hashes, _) = stepped(&mut native, start, UNTIL_MS);
    let native_events = events(&mut native, &mut native_bytes);

    // The T2 field, to a pulse boundary past 15 s with no think in flight.
    let mut flown = the_j_field(Controller::Mind);
    support::with_rhai(&mut flown);
    let start = flown.now_ms;
    let (mut bytes, mut hashes, mut wall) = stepped(&mut flown, start, 15_000);
    while flown.minds.values().any(|state| state.pending.is_some()) {
        let next = flown.now_ms + SUBSTEP;
        let (more, more_hashes, after) = stepped(&mut flown, wall, next);
        bytes.extend(more);
        hashes.extend(more_hashes);
        wall = after;
    }
    let snapshot = Snapshot::encode(&flown).expect("the arena encodes");
    let mut resumed = Snapshot::decode(&snapshot.document, (*flown.definition).clone())
        .expect("the document decodes");
    let shown = format!("{:?}", resumed.mind_runner);
    assert!(
        shown.starts_with("NativeStockMind"),
        "the decode re-installs the native stock mind: {shown:.40}"
    );
    support::with_rhai(&mut resumed);
    let shown = format!("{:?}", resumed.mind_runner);
    assert!(
        shown.starts_with("RhaiRunner"),
        "the resumed field flies the Rhai runner: {shown:.40}"
    );
    let now = resumed.now_ms;
    let _ = resumed.resume_at(now);
    let (more, more_hashes, _) = stepped(&mut resumed, now + SUBSTEP, UNTIL_MS);
    bytes.extend(more);
    hashes.extend(more_hashes);
    let resumed_events = events(&mut resumed, &mut bytes);
    assert!(
        !native_events.is_empty() && hashes.len() > 100,
        "the run journalled and stepped"
    );
    assert_eq!(hashes, native_hashes, "the stitched hash sequence");
    assert_eq!(resumed_events, native_events, "the stitched ledger");
    assert_eq!(bytes, native_bytes, "byte for byte");
    assert_eq!(
        tacsim::arena::state_hash(&resumed),
        tacsim::arena::state_hash(&native),
        "the final state hash"
    );
}

/// [`m1b2_two_t1_slot_numbers_are_the_arenas_at_every_landing`] on the Rhai
/// runner alone (17b cut M2.7): every recorded think held to the native one.
#[test]
fn m1b2_two_t1_slot_numbers_are_the_arenas_at_every_landing_under_the_rhai_runner() {
    support::under_the_rhai_runner(
        "m1b2_two_t1_slot_numbers_are_the_arenas_at_every_landing",
        m1b2_two_t1_slot_numbers_are_the_arenas_at_every_landing,
    );
}

/// [`m1a6_the_stock_mind_is_t0_on_the_first_decide_window`] on the Rhai
/// runner alone (17b cut M2.7): the (j) field's T1 window flown by T2, held
/// to the fixture's own assertions on T1 against T0 (it notes no run).
#[test]
fn m1a6_the_stock_mind_is_t0_on_the_first_decide_window_under_the_rhai_runner() {
    support::under_the_rhai_runner(
        "m1a6_the_stock_mind_is_t0_on_the_first_decide_window",
        m1a6_the_stock_mind_is_t0_on_the_first_decide_window,
    );
}

/// [`l2_two_minds_are_one_squad_under_a_leader_at_three_cadences`] on the
/// Rhai runner alone (17b cut M2.7): [`lance_field`]'s three minds under the
/// leader's hinted `ENGAGE`, each of the fixture's three runs (5,000, 1,000
/// and 500 ms) the native one's event list and hash sequence.
#[test]
fn l2_two_minds_are_one_squad_under_a_leader_at_three_cadences_under_the_rhai_runner() {
    support::under_the_rhai_runner(
        "l2_two_minds_are_one_squad_under_a_leader_at_three_cadences",
        l2_two_minds_are_one_squad_under_a_leader_at_three_cadences,
    );
}

/// [`l2_a_self_charged_support_fights_on_doctrine`] on the Rhai runner alone
/// (17b cut M2.7): [`charge_field`]'s self-charged and blind supporters,
/// every recorded think held to the native one.
#[test]
fn l2_a_self_charged_support_fights_on_doctrine_under_the_rhai_runner() {
    support::under_the_rhai_runner(
        "l2_a_self_charged_support_fights_on_doctrine",
        l2_a_self_charged_support_fights_on_doctrine,
    );
}

/// [`m1b5_t1_reads_its_card_through_the_overlay_and_line_is_unchanged`] on
/// the Rhai runner alone (17b cut M2.7): the M1b.5 anchor's recorded events
/// and answers, and the withdrawal at [`M1B5_WITHDRAWS_AT`] on the
/// non-default doctrine, flown by T2.
#[test]
fn m1b5_t1_reads_its_card_through_the_overlay_and_line_is_unchanged_under_the_rhai_runner() {
    support::under_the_rhai_runner(
        "m1b5_t1_reads_its_card_through_the_overlay_and_line_is_unchanged",
        m1b5_t1_reads_its_card_through_the_overlay_and_line_is_unchanged,
    );
}

/// [`m1b5_a_card_lands_at_the_pulses_start_and_the_next_think_reads_it`] on
/// the Rhai runner alone (17b cut M2.7) — **M1b.5's card fixture**: a staged
/// `Rearguard` read at the next think and the withdrawal at its `break_off`,
/// at 5,000, 1,000 and 500 ms the same event list, hash sequence and
/// sequence of thinks as the native run's.
#[test]
fn m1b5_a_card_lands_at_the_pulses_start_and_the_next_think_reads_it_under_the_rhai_runner() {
    support::under_the_rhai_runner(
        "m1b5_a_card_lands_at_the_pulses_start_and_the_next_think_reads_it",
        m1b5_a_card_lands_at_the_pulses_start_and_the_next_think_reads_it,
    );
}

/// [`m1b5_an_unarmed_hull_takes_any_preset_with_its_fire_section_inert`] on
/// the Rhai runner alone (17b cut M2.7).
#[test]
fn m1b5_an_unarmed_hull_takes_any_preset_with_its_fire_section_inert_under_the_rhai_runner() {
    support::under_the_rhai_runner(
        "m1b5_an_unarmed_hull_takes_any_preset_with_its_fire_section_inert",
        m1b5_an_unarmed_hull_takes_any_preset_with_its_fire_section_inert,
    );
}

/// [`m1b5_a_newer_order_is_obeyed_after_an_early_end`] on the Rhai runner
/// alone (17b cut M2.7): the four thinks through the (j) field's Rhai runner
/// (direct thinks, held to the fixture's own assertions; no run is noted).
#[test]
fn m1b5_a_newer_order_is_obeyed_after_an_early_end_under_the_rhai_runner() {
    support::under_the_rhai_runner(
        "m1b5_a_newer_order_is_obeyed_after_an_early_end",
        m1b5_a_newer_order_is_obeyed_after_an_early_end,
    );
}

// ---------------------------------------------------------------------------
// 17b cut M3.1 — the core under the mind (doc 17 §9, D-UM-33, D-UM-35)
// ---------------------------------------------------------------------------

/// **What a think was handed, by the row** (17b cut M3.1): its hull, its
/// sequence, its millisecond, `dropped`, the profile and the view's rows.
#[derive(Clone, Debug, PartialEq)]
struct Handed {
    unit: ObjId,
    think_seq: u64,
    t_ms: i64,
    dropped: bool,
    profile: tacsim::arena::core::CoreProfile,
    rows: usize,
}

/// **A runner that thinks nothing and keeps what each think was handed**,
/// in front of `inner` when one is given (its answer is the think's).
#[derive(Debug)]
struct Handing {
    inner: Option<Arc<dyn MindRunner>>,
    seen: Mutex<Vec<(Handed, ThinkResult)>>,
}

impl Handing {
    fn new(inner: Option<Arc<dyn MindRunner>>) -> Arc<Handing> {
        Arc::new(Handing {
            inner,
            seen: Mutex::new(Vec::new()),
        })
    }

    fn seen(&self) -> Vec<(Handed, ThinkResult)> {
        self.seen
            .lock()
            .map(|seen| seen.clone())
            .unwrap_or_default()
    }
}

impl MindRunner for Handing {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        let result = self.inner.as_ref().map_or_else(
            || ThinkResult {
                diagnostics: None,
                rule_sample: None,
                intents: Vec::new(),
                memory: job.memory.to_vec(),
                fault: None,
                trace: Vec::new(),
            },
            |inner| inner.think(job),
        );
        if let Ok(mut seen) = self.seen.lock() {
            seen.push((
                Handed {
                    unit: job.unit,
                    think_seq: job.think_seq,
                    t_ms: job.t_ms,
                    dropped: job.dropped,
                    profile: *job.profile,
                    rows: job.view.contacts.len(),
                },
                result.clone(),
            ));
        }
        result
    }
}

/// `unit`'s `core_class` cell set to `value` by one plan — the live class
/// `read_core` reads (17b cut M3.1).
fn core_cell_at(unit: &TacticalUnit, value: i64) -> TacticalUnit {
    let addr = CellAddr::Component {
        component: mush_tactics::ComponentId::new("core").expect("a component"),
        cell: mush_tactics::CellId::new(tacsim::definitions::unit::CORE_CLASS_CELL)
            .expect("a cell"),
    };
    let plan = UnitMutationPlan::new(vec![UnitMutationOp::SetCell { addr, value }], None, None)
        .expect("a plan");
    mush_tactics::apply(unit, &plan, 0)
        .expect("the plan applies")
        .0
}

/// **A field where red holds fourteen classified hostiles**, locked on the
/// fifth nearest, its own core cell at `core_cell` (set by a plan before the
/// check-out): red at (2, 8) armed with the cannon, the fourteen blues on
/// cells two to fifteen away.
///
/// **17b cut M5.2** (the S1 ruling): the check-out seals a governed core
/// lowered to Class 0 at Class 0 — it flies the ROM, as no core does, and is
/// never over-class (the design note's C-24 as amended) — as it did at
/// 5f505c8.
fn crowded_field(core_cell: i64) -> Arena {
    use tacsim::arena::state::SeenPose;
    let mut red = tank(
        RED,
        Team::Red,
        centre(2, 8),
        vec![cannon()],
        Controller::Mind,
    );
    red.unit = core_cell_at(&red.unit, core_cell);
    red.reseal(&tacsim::match_::SealTerms::fixture())
        .expect("the fixture terms seal");
    let mut units = vec![red];
    let blues: Vec<(ObjId, (u16, u16))> = (0_u16..14)
        .map(|k| {
            (
                ObjId(20 + u32::from(k)),
                (3 + k % 13, if k < 13 { 3 } else { 13 }),
            )
        })
        .collect();
    for (id, (x, y)) in &blues {
        units.push(tank(
            *id,
            Team::Blue,
            centre(*x, *y),
            Vec::new(),
            Controller::Pilot,
        ));
    }
    let mut arena = arena_of(units, &[]);
    if let Some(state) = arena.unit_mut(RED) {
        state.power = Power::Online;
    }
    let mut by_range: Vec<(u64, ObjId, ContactId)> = Vec::new();
    for (id, _) in &blues {
        let plot = arena.unit(*id).map(SeenPose::of).expect("a blue");
        let contact = draw_contact_id(SEED, RED, *id);
        arena.contacts.insert(
            RED,
            contact,
            ShadowContact::sighted(
                *id,
                Classification::Classified { team: Team::Blue },
                0,
                plot,
            ),
        );
        let red = arena.unit(RED).expect("red");
        by_range.push((plot.range_from(&red.pose), *id, contact));
    }
    by_range.sort_unstable();
    let fifth = by_range
        .get(4)
        .map(|(_, _, contact)| *contact)
        .expect("five");
    assert_eq!(
        arena.resolve_lock(RED, fifth, 0),
        Ok(()),
        "red locks the fifth"
    );
    arena
}

/// **The view is cut to the mind's class** (17b cut M3.1; doc 17 §9,
/// D-UM-35, D-UM-4). Red holds fourteen classified hostiles, locked on the
/// fifth nearest. On a Class I mind (rated I on its core) the view carries
/// the nearest twelve and the odds on two — the lock's and the nearest's, the
/// frame's two. With red's core cell set to 0 by a plan before the check-out
/// the mind is Class 0, the ROM: eight rows, the lock's odds alone, and the
/// think is handed `dropped` and Class 0's row.
///
/// **Re-pinned at 17b cut M5.2, back to its 5f505c8 behaviour: a governed
/// Class-0 core flies the ROM** (17b cut M5.2's S1 ruling). M5.2's stage A
/// read the design note's C-24 literally — Standard Doctrine's rating I over
/// a governed core at Class 0 refused `SealError::OverClass { I, Zero }` —
/// and moved this test's plan to after the check-out with that refusal
/// pinned here; the ruling (C-24 amended: "a governed core at Class 0 flies
/// the ROM, as no core does") puts the plan back before the check-out, the
/// hull sealed at Class 0 on its core as lowered, and the pin removed.
#[test]
fn m3_the_view_is_cut_to_the_minds_class() {
    use tacsim::arena::core::{ClassTable, CoreClass};
    for (cell, class, rows) in [(1, CoreClass::I, 12), (0, CoreClass::Zero, 8)] {
        let mut arena = crowded_field(cell);
        assert_eq!(
            arena
                .unit(RED)
                .map(|state| (state.seal.core_class, state.seal.class)),
            Some((class, class)),
            "sealed at the check-out on the core as lowered"
        );
        assert_eq!(arena.mind_class(RED), class);
        let view = arena.mind_view(RED).expect("red's view");
        assert_eq!(view.contacts.len(), rows, "{class:?}'s rows");
        let lock = view.lock.expect("the lock");
        let odds: Vec<ContactId> = view
            .contacts
            .iter()
            .filter(|row| !row.odds.is_empty())
            .map(|row| row.id)
            .collect();
        if class == CoreClass::Zero {
            assert_eq!(odds, vec![lock], "Class 0: the lock's odds alone");
        } else {
            assert_eq!(odds.len(), 2, "Class I: the frame's two");
            assert!(odds.contains(&lock), "the lock among them: {odds:?}");
        }

        let mut arena = crowded_field(cell);
        let handing = Handing::new(None);
        arena.mind_runner = handing.clone();
        let ran = run(&mut arena, Drive::Every(1_000), think_at(RED, 0) + SUBSTEP);
        let (first, _) = handing.seen().first().cloned().expect("a think");
        assert_eq!(
            first.dropped,
            class == CoreClass::Zero,
            "{class:?}'s dropped"
        );
        assert_eq!(first.profile, ClassTable::STOCK.row(class));
        assert_eq!(first.rows, rows);
        let row = arena.minds.get(&RED).expect("red's row");
        assert_eq!(row.strikes, 0, "no fault, no strike");
        let lines: Vec<String> = ran.events.into_iter().map(|(_, kind, _)| kind).collect();
        assert!(
            lines.iter().all(|kind| !kind.contains("drop")),
            "no drop line: {lines:?}"
        );
    }
}

/// **The shipped heavy as red** (the command core, Class III, in its hull),
/// at (2, 8) under `controller`, off the fielded catalog (17b cut M3.1).
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

fn shipped_heavy(controller: Controller) -> CustodyUnit {
    let catalog = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    let key = CanonicalId::new(content::HEAVY_MK1).expect("a canonical id");
    let cells = tacsim::definitions::to_tactical_unit(
        catalog.specs.get(&key).expect("the heavy"),
        catalog.bindings.get(&key).expect("its binding"),
        RED,
    )
    .expect("the heavy instantiates");
    tacsim::match_::custody_unit(
        &catalog,
        cells,
        RED,
        Team::Red,
        centre(2, 8),
        0,
        controller,
        1,
        &tacsim::match_::SealTerms::fixture(),
    )
    .expect("the fixture terms seal")
}

/// **A Class III mind on the heavy thinks every 250 ms** (17b cut M3.1;
/// D-UM-35's `think_ms`, read at each scheduling): the shipped heavy (the
/// command core, Class III) flown by a mind rated III
/// ([`rate`], the seal's class since 17b cut M4.1) is handed Class III's row and thinks
/// a quarter-second apart — its first think on the grid the check-out laid
/// at Standard Doctrine's rating, every one after it at its own row's period.
#[test]
fn m3_a_class_iii_mind_on_the_heavy_thinks_every_250_ms() {
    use tacsim::arena::core::{ClassTable, CoreClass};
    let heavy = shipped_heavy(Controller::Mind);
    let blue = tank(
        BLUE,
        Team::Blue,
        centre(13, 8),
        Vec::new(),
        Controller::Pilot,
    );
    let mut arena = arena_of(vec![heavy, blue], &[BLUE]);
    assert_eq!(
        arena.core_of(RED).map(|core| core.class),
        Some(CoreClass::III)
    );
    assert_eq!(
        arena.mind_class(RED),
        CoreClass::I,
        "Standard Doctrine is rated I"
    );
    rate(&mut arena, RED, CoreClass::III);
    assert_eq!(
        arena.mind_profile(RED),
        ClassTable::STOCK.row(CoreClass::III)
    );
    let handing = Handing::new(None);
    arena.mind_runner = handing.clone();
    let _ = run(&mut arena, Drive::Every(1_000), think_at(RED, 0) + 2_000);
    let seen = handing.seen();
    assert!(
        seen.len() >= 8,
        "eight thinks in two seconds: {}",
        seen.len()
    );
    for pair in seen.windows(2) {
        if let [(a, _), (b, _)] = pair {
            assert_eq!(b.think_seq, a.think_seq + 1);
            assert_eq!(b.t_ms - a.t_ms, 250, "a quarter-second apart");
        }
    }
    assert!(seen.iter().all(
        |(handed, _)| handed.profile == ClassTable::STOCK.row(CoreClass::III) && !handed.dropped
    ));
}

/// **A hull that never had a core thinks on the stand-in from its first
/// think** (17b cut M3.1; D-UM-33: Class 0, the ROM alone): a
/// `Controller::Mind` hull built with no core at all — no `core` component,
/// no `core_class` cell, no `tacsim.core` capability — is handed `dropped`
/// and Class 0's row from its first think, the Rhai runner answers with its
/// native stand-in (`DROPPED_LINE` its whole trace) and no fault, the row
/// takes no strike and no line says it dropped (17b cut M3.4 writes one only
/// on strikes and on a sealed core that is gone). A hull whose core
/// capability was revoked BEFORE the check-out reads the same `None` today
/// and is asserted beside it as its own case: from 17b cut M3.4 a core the
/// seal recorded and lost is the other rule, with a line, and the
/// never-cored case keeps this proof whatever that one becomes.
#[test]
fn m3_a_never_cored_mind_thinks_on_the_stand_in_from_its_first_think() {
    use tacsim::arena::core::{ClassTable, CoreClass};
    use tacsim::definitions::unit::CORE_CAPABILITY;
    let core = mush_tactics::ComponentId::new("core").expect("a component");
    let never = CustodyUnit {
        unit: tactical_unit_fitted(RED, None),
        ..tank(
            RED,
            Team::Red,
            centre(6, 8),
            vec![cannon()],
            Controller::Mind,
        )
    }
    .sealed();
    assert!(
        never.unit.component(&core).is_none(),
        "never fitted: no core component"
    );
    assert!(
        never
            .unit
            .capabilities()
            .all(|key| key.id.as_str() != CORE_CAPABILITY),
        "never fitted: no core capability"
    );
    let mut revoked = tank(
        RED,
        Team::Red,
        centre(6, 8),
        vec![cannon()],
        Controller::Mind,
    );
    let key = revoked
        .unit
        .capabilities()
        .find(|key| key.id.as_str() == CORE_CAPABILITY)
        .cloned()
        .expect("the fixture's core");
    let plan = UnitMutationPlan::new(vec![UnitMutationOp::RevokeCapability { key }], None, None)
        .expect("a plan");
    revoked.unit = mush_tactics::apply(&revoked.unit, &plan, 0)
        .expect("revoked")
        .0;
    revoked
        .reseal(&tacsim::match_::SealTerms::fixture())
        .expect("the fixture terms seal");
    assert!(
        revoked.unit.component(&core).is_some(),
        "the revoked hull keeps its component"
    );

    for (case, red) in [
        ("never cored", never),
        ("revoked before the check-out", revoked),
    ] {
        let blue = tank(
            BLUE,
            Team::Blue,
            centre(9, 8),
            Vec::new(),
            Controller::Pilot,
        );
        let mut arena = arena_of(vec![red, blue], &[BLUE]);
        assert_eq!(arena.core_of(RED), None, "{case}: no core, the ROM alone");
        assert_eq!(arena.mind_class(RED), CoreClass::Zero, "{case}");
        assert_eq!(
            arena.slot_capacity(RED),
            ClassTable::STOCK.row(CoreClass::Zero).slots,
            "{case}: Class 0's one slot"
        );
        let source = "fn think(v, c) { this.never = 1; }";
        let rhai: Arc<dyn MindRunner> = Arc::new(
            tacsim::minds::RhaiRunner::new(
                &arena.arena_cfg,
                source,
                mush_mind::Digest(*b"m3.1 never cored stand-in source"),
            )
            .expect("compiles"),
        );
        let handing = Handing::new(Some(rhai));
        arena.mind_runner = handing.clone();
        let ran = run(&mut arena, Drive::Every(1_000), think_at(RED, 3) + SUBSTEP);
        let seen = handing.seen();
        assert!(seen.len() >= 4, "{case}: four thinks: {}", seen.len());
        assert_eq!(
            seen.first().map(|(handed, _)| handed.think_seq),
            Some(0),
            "{case}: from the first think"
        );
        for (handed, result) in &seen {
            assert!(
                handed.dropped,
                "{case}: think {} is handed dropped",
                handed.think_seq
            );
            assert_eq!(
                handed.profile,
                ClassTable::STOCK.row(CoreClass::Zero),
                "{case}"
            );
            assert_eq!(result.fault, None, "{case}: the stand-in faults nothing");
            assert_eq!(
                result.trace,
                vec![tacsim::minds::DROPPED_LINE.to_owned()],
                "{case}"
            );
        }
        let row = arena.minds.get(&RED).expect("red's row");
        assert_eq!(row.strikes, 0, "{case}: no strike");
        assert!(!row.dropped(), "{case}: not dropped by strikes");
        assert!(
            ran.events.iter().all(|(_, kind, _)| !kind.contains("drop")),
            "{case}: no drop line"
        );
    }
}

/// **A Class III mind and a Class 0 mind at three cadences are one event
/// list and one hash sequence** (17b cut M3.1; 16b §11's cross-tempo rule
/// over the class-driven cadence, the view's cut and the ROM's hand-off).
/// The (j) field's scene with red on a Class III core rated III — thinking
/// every 250 ms, one substep, so every landing shares its millisecond with
/// the next think and runs before it (D-UM-20) — and red2 armed on a core
/// whose cell a plan set to 0 before the check-out (Class 0: the ROM from
/// its first think, eight rows, the lock's odds alone, one slot), both flown
/// by the production stock mind, run 30 s at 5,000, 1,000 and 500 ms and by
/// the task's own wakes at each: one event list, one hash sequence and one
/// sequence of handed thinks (hull, millisecond, `dropped`, row and the
/// view's rows) with one answer each.
#[test]
fn m3_a_class_iii_and_a_class_0_mind_at_three_cadences_are_one_event_list_and_one_hash_sequence() {
    use tacsim::arena::core::{ClassTable, CoreClass};
    let flown = |drive: Drive| {
        let seeing = |mut unit: CustodyUnit| {
            unit.sensor = Some(SensorProfile {
                max_range_cells: 10,
                ..sight()
            });
            unit
        };
        let red = CustodyUnit {
            unit: tactical_unit_fitted(RED, Some(CoreClass::III)),
            ..seeing(tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                Controller::Mind,
            ))
        }
        .sealed();
        let mut red2 = seeing(tank(
            RED2,
            Team::Red,
            centre(6, 10),
            vec![cannon()],
            Controller::Mind,
        ));
        red2.unit = core_cell_at(&red2.unit, 0);
        let blue = presenting(seeing(tank(
            BLUE,
            Team::Blue,
            centre(9, 8),
            Vec::new(),
            Controller::Pilot,
        )));
        let mut arena = arena_of(vec![red, red2, blue], &[BLUE]);
        rate(&mut arena, RED, CoreClass::III);
        assert_eq!(arena.mind_class(RED), CoreClass::III);
        assert_eq!(arena.mind_class(RED2), CoreClass::Zero);
        assert!(format!("{:?}", arena.mind_runner).starts_with("NativeStockMind"));
        let handing = Handing::new(Some(Arc::clone(&arena.mind_runner)));
        arena.mind_runner = handing.clone();
        let ran = run(&mut arena, drive, 30_000);
        (ran, handing.seen())
    };

    let (first, seen) = flown(Drive::Every(5_000));
    let of = |unit: ObjId| -> Vec<Handed> {
        seen.iter()
            .filter(|(handed, _)| handed.unit == unit)
            .map(|(handed, _)| handed.clone())
            .collect()
    };
    let three = of(RED);
    assert!(three.len() >= 100, "Class III's thinks: {}", three.len());
    assert!(
        three.iter().all(
            |handed| !handed.dropped && handed.profile == ClassTable::STOCK.row(CoreClass::III)
        )
    );
    assert!(
        three
            .windows(2)
            .all(|pair| matches!(pair, [a, b] if b.t_ms - a.t_ms == 250)),
        "Class III thinks a substep apart"
    );
    let zero = of(RED2);
    assert!(zero.len() >= 25, "Class 0's thinks: {}", zero.len());
    assert!(zero.iter().all(|handed| handed.dropped
        && handed.profile == ClassTable::STOCK.row(CoreClass::Zero)
        && handed.rows <= ClassTable::STOCK.row(CoreClass::Zero).rows));
    assert!(
        zero.windows(2)
            .all(|pair| matches!(pair, [a, b] if b.t_ms - a.t_ms == 1_000)),
        "Class 0 thinks a second apart"
    );
    let words: Vec<(String, Option<u32>)> = first
        .events
        .iter()
        .map(|(_, json, _)| kind_and_unit(json))
        .collect();
    for (unit, want) in [(RED, "locked"), (RED, "fired"), (RED2, "fired")] {
        assert!(
            words
                .iter()
                .any(|(word, on)| word == want && *on == Some(unit.0)),
            "#{} never journalled a {want}: {words:?}",
            unit.0
        );
    }
    for drive in [
        Drive::Every(1_000),
        Drive::Every(500),
        Drive::Task(5_000),
        Drive::Task(1_000),
        Drive::Task(500),
    ] {
        let (other, other_seen) = flown(drive);
        assert_eq!(
            other.events, first.events,
            "the event list diverged at {drive:?}"
        );
        assert_eq!(
            other.hashes, first.hashes,
            "the hash sequence diverged at {drive:?}"
        );
        assert_eq!(other_seen, seen, "a think diverged at {drive:?}");
        assert_ne!(other.pulses, first.pulses, "{drive:?} ran the same pulses");
    }
}

/// **A mind adopted at Class III is put on Class III's grid** (17b cut M3.1;
/// `Arena::adopt_mind` reads `Arena::mind_profile` at the adoption): the
/// shipped heavy checked out under a pilot, its rating set to III, then
/// handed to a mind (`Arena::set_controller`) — its first think falls on the
/// 250 ms stagger grid, not on Class I's 1,000 ms one, and every think after
/// it is 250 ms on.
#[test]
fn m3_a_mind_adopted_at_class_iii_thinks_on_its_own_grid() {
    use tacsim::arena::core::{ClassTable, CoreClass};
    let grid = |now: i64, cadence: i64| {
        let mut at = now - now.rem_euclid(cadence) + stagger_ms(SEED, RED, cadence);
        if at < now {
            at += cadence;
        }
        at
    };
    let blue = tank(
        BLUE,
        Team::Blue,
        centre(13, 8),
        Vec::new(),
        Controller::Pilot,
    );
    let mut arena = arena_of(vec![shipped_heavy(Controller::Pilot), blue], &[BLUE]);
    assert!(!arena.minds.contains_key(&RED), "no mind at the check-out");
    rate(&mut arena, RED, CoreClass::III);
    let now = arena.now_ms;
    let (at_iii, at_i) = (grid(now, 250), grid(now, STOCK_THINK_MS));
    assert_ne!(at_iii, at_i, "the two grids part on this seed");
    arena.set_controller(RED, Controller::Mind, MachineState::Startup);
    let handing = Handing::new(None);
    arena.mind_runner = handing.clone();
    let _ = run(&mut arena, Drive::Every(1_000), at_i.max(at_iii) + 1_000);
    let seen = handing.seen();
    let (first, _) = seen.first().cloned().expect("a think");
    assert_eq!(first.t_ms, at_iii, "the first think on Class III's grid");
    assert_eq!(first.profile, ClassTable::STOCK.row(CoreClass::III));
    assert!(
        seen.windows(2)
            .all(|pair| matches!(pair, [(a, _), (b, _)] if b.t_ms - a.t_ms == 250)),
        "every think after it a substep on"
    );
}

// ---------------------------------------------------------------------------
// 17b cut M3.5: the dormant row (doc 17 §12 "Release and possess"; D-UM-2,
// D-UM-30, D-UM-32) and the runner by the seal (doc 17 §13)
// ---------------------------------------------------------------------------

/// The pilot who possesses red in the dormant-row fixtures.
const POSSESSOR: ObjId = ObjId(2_000);

/// A contact id nothing holds: a `LOCK` on it is refused `no_contact`.
const STRANGER: ContactId = ContactId(4_242);

/// **The dormant-row field** — [`epoch_rig`]'s three hulls, red flown by a
/// [`ScriptedMind`] answering `script`.
fn possess_field(script: Vec<Says>) -> (Rig, Arc<ScriptedMind>) {
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
    let mind = ScriptedMind::new(&[(RED, script)]);
    rig.arena.mind_runner = mind.clone();
    (rig, mind)
}

/// Whether the heap holds any think or landing of `unit`.
fn mind_events_of(arena: &Arena, unit: ObjId) -> bool {
    arena.due.iter().any(|event| match event.kind {
        DueKind::MindThink { unit: on, .. } | DueKind::MindLand { unit: on, .. } => on == unit,
        _ => false,
    })
}

/// The `Refused` notices of `pilot`'s frames in `out`, as `(order, refusal)`.
fn pilot_refusals(out: &StepOutput, pilot: ObjId) -> Vec<(String, Refusal)> {
    out.frames
        .iter()
        .filter(|(to, _)| *to == pilot)
        .flat_map(|(_, frame)| frame.notices.iter())
        .filter_map(|notice| match notice {
            FrameNotice::Refused { order, refusal, .. } => Some((order.to_string(), *refusal)),
            _ => None,
        })
        .collect()
}

/// **A possess keeps the row dormant and a release resumes it** (17b cut
/// M3.5; doc 17 §12 "Release and possess": `set_controller(Pilot)` does NOT
/// delete the `MindState` row; D-UM-2, D-UM-30).
///
/// Think 0 starts the hull, locks a contact it does not hold (refused
/// `no_contact` into the buffer at the landing) and writes memory; think 1
/// faults (a strike, the drained notice put back in front of the buffer); the
/// row is then given a standing order. A `Live` control naming a pilot takes
/// the hull: the row is byte-equal to the one the possess found — memory,
/// strikes, `dropped`, seal digest, card, ranges and the NOTICE BUFFER —
/// but for its ORDER, cleared at the suspend (17b cut M4.3: an order given
/// to the mind is not the pilot's and does not outlive the seat); no think
/// or landing is on the heap and the unit's seal reference is untouched.
/// While the pilot flies it the pilot's own refused `LOCK` is told to the
/// pilot and never enters the mind's buffer (`stage_notice`'s controller
/// gate), no think is taken, and the row does not move. A `Released` hands
/// the hull back to its MIND (`apply_control`: a row stands) with its memory
/// SEEDED `Withdraw` (17b cut M4.3, `seed_withdraw`: a memory of another
/// shape, as this scripted fixture's bytes are, becomes the stock seed), and
/// the first think after the release is `think_seq` continued, handed the
/// seeded bytes and draining the old buffer — the pilot's notices nowhere in
/// it. The seal reference is the check-out's throughout: a re-seat never
/// re-seals.
///
/// A second possess with a think IN FLIGHT discards its answer: `pending`
/// cleared, the landing off the heap, and the landing's millisecond passes
/// with nothing of the think applied.
#[test]
fn m3_a_possess_keeps_the_row_dormant_and_a_release_resumes_it() {
    let (mut rig, mind) = possess_field(vec![
        Says {
            memory: Some(b"{\"fixture\":314}".to_vec()),
            ..says(
                0,
                vec![IntentKind::Startup, IntentKind::Lock { contact: STRANGER }],
            )
        },
        Says {
            fault: Some("type"),
            ..says(1, Vec::new())
        },
    ]);
    let _ = rig.until(10_000, |arena| {
        arena
            .minds
            .get(&RED)
            .is_some_and(|state| state.think_seq == 2)
    });
    if let Some(state) = rig.arena.minds.get_mut(&RED) {
        state.order = Some(FrameOrder {
            squad: SmolStr::new("red-1"),
            objective: MindObjective::Hold,
            hold_fire: false,
            max_range_band: None,
            expires_ms: 600_000,
        });
    }
    let found = rig.arena.minds.get(&RED).cloned().expect("the mind's row");
    assert_eq!(found.strikes, 1, "think 1's strike");
    assert_eq!(
        found.memory,
        b"{\"fixture\":314}".to_vec(),
        "think 0's memory"
    );
    assert!(found.pending.is_none());
    assert!(
        found.notices.iter().any(|notice| matches!(
            notice,
            FrameNotice::Refused {
                refusal: Refusal::NoContact,
                ..
            }
        )),
        "the mind's own refusal is in its buffer: {:?}",
        found.notices
    );
    assert_ne!(found.seal_digest, [0; 32], "laid with the seal's digest");
    let sealed = rig.arena.unit(RED).map(|unit| unit.seal.clone());
    // 17b cut M4.3: the possess clears the row's order and nothing else.
    let dormant = MindState {
        order: None,
        ..found.clone()
    };

    // The possess.
    let helm = rig.arena.epochs.current(RED, Channel::Helm) + 1;
    let possessed_at = rig.arena.now_ms;
    let _ = rig.pulse(
        &[control(
            Channel::Helm,
            helm,
            ControlState::Live,
            Some(POSSESSOR),
        )],
        &[],
    );
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.controller),
        Some(Controller::Pilot)
    );
    assert_eq!(
        rig.arena.minds.get(&RED),
        Some(&dormant),
        "the row is kept as the possess found it, its order cleared"
    );
    assert!(!mind_events_of(&rig.arena, RED), "no think or landing");

    // Piloted: the pilot's own refusal is the pilot's alone, and nothing of
    // the mind moves for eight seconds — several thinks' worth of grid.
    let lock = intent(&rig.arena, RED, IntentKind::Lock { contact: STRANGER }, 1);
    let out = rig.pulse(&[], &[lock]);
    assert_eq!(
        pilot_refusals(&out, POSSESSOR),
        vec![("lock".to_owned(), Refusal::NoContact)],
        "the pilot is told"
    );
    while rig.arena.now_ms < possessed_at + 8_000 {
        let _ = rig.pulse(&[], &[]);
        assert_eq!(
            rig.arena.minds.get(&RED),
            Some(&dormant),
            "the dormant row never moves (at {})",
            rig.arena.now_ms
        );
        assert!(!mind_events_of(&rig.arena, RED));
    }
    assert!(
        mind.seen()
            .iter()
            .all(|seen| seen.unit != RED || seen.t_ms < possessed_at),
        "no think while piloted"
    );

    // The release: back to the mind, the row as it stood.
    let released_at = rig.arena.now_ms;
    let _ = rig.pulse(
        &[control(
            Channel::Helm,
            helm + 1,
            ControlState::Released,
            Some(POSSESSOR),
        )],
        &[],
    );
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.controller),
        Some(Controller::Mind),
        "a Released over a standing row hands the hull back to its mind"
    );
    let _ = rig.until(released_at + 3_000, |arena| {
        arena
            .minds
            .get(&RED)
            .is_some_and(|state| state.think_seq > found.think_seq)
    });
    let first = mind
        .seen()
        .into_iter()
        .find(|seen| seen.unit == RED && seen.t_ms >= released_at)
        .expect("a think after the release");
    assert_eq!(first.think_seq, found.think_seq, "think_seq continued");
    assert_eq!(
        first.memory,
        seed_withdraw(&found.memory),
        "handed the OLD memory bytes, seeded Withdraw (17b cut M4.3)"
    );
    assert_eq!(
        first.memory,
        withdraw_seed(),
        "a scripted memory becomes the stock seed"
    );
    assert_eq!(
        first.view.notices, found.notices,
        "the old buffer, byte for byte — the pilot's notices nowhere in it"
    );
    assert!(!first.dropped);
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.seal.clone()),
        sealed,
        "a re-seat never re-seals"
    );
    assert_eq!(
        rig.arena.minds.get(&RED).map(|state| state.seal_digest),
        Some(found.seal_digest)
    );

    // A second possess, with a think in flight: its answer is discarded.
    let limit = rig.arena.now_ms + 3_000;
    let _ = rig.until(limit, |arena| in_flight(arena, RED));
    let flying = rig
        .arena
        .minds
        .get(&RED)
        .and_then(|state| state.pending.clone())
        .expect("a think in flight");
    let land = flying.t_ms + SUBSTEP;
    let _ = rig.pulse(
        &[control(
            Channel::Helm,
            helm + 2,
            ControlState::Live,
            Some(POSSESSOR),
        )],
        &[],
    );
    assert!(
        rig.arena
            .minds
            .get(&RED)
            .is_some_and(|state| state.pending.is_none()),
        "pending cleared"
    );
    assert!(
        !mind_events_of(&rig.arena, RED),
        "the landing is off the heap"
    );
    let kept = rig.arena.minds.get(&RED).cloned();
    while rig.arena.now_ms <= land + SUBSTEP {
        let _ = rig.pulse(&[], &[]);
    }
    assert_eq!(rig.arena.minds.get(&RED).cloned(), kept, "nothing landed");
    assert_eq!(rig.arena.unit(RED).map(|unit| unit.seal.clone()), sealed);
}

/// **A dropped hull possessed then released stays dropped** (17b cut M3.5;
/// D-UM-32: the drop is for the rest of the sortie). Three faulted thinks
/// drop the hull — one `mind_dropped` line; a pilot takes it and gives it
/// back; the row's drop is the one the possess found, every think after the
/// release is handed `dropped` (the ROM thinks), and no second `mind_dropped`
/// line is journalled.
#[test]
fn m3_a_dropped_hull_possessed_and_released_stays_dropped() {
    let fault = |n: u64| Says {
        fault: Some("type"),
        ..says(n, Vec::new())
    };
    let (mut rig, mind) = possess_field(vec![fault(0), fault(1), fault(2)]);
    let _ = rig.until(10_000, |arena| {
        arena.minds.get(&RED).is_some_and(MindState::dropped)
    });
    assert_eq!(rig.lines("mind_dropped", RED).len(), 1, "one drop line");
    let found = rig.arena.minds.get(&RED).cloned().expect("the row");
    assert_eq!(
        found.dropped,
        Some(tacsim::arena::DropCause::Strikes),
        "dropped on strikes"
    );
    let helm = rig.arena.epochs.current(RED, Channel::Helm) + 1;
    let _ = rig.pulse(
        &[control(
            Channel::Helm,
            helm,
            ControlState::Live,
            Some(POSSESSOR),
        )],
        &[],
    );
    assert_eq!(rig.arena.minds.get(&RED), Some(&found), "kept, dormant");
    let limit = rig.arena.now_ms + 3_000;
    let _ = rig.until(limit, |arena| arena.now_ms >= limit - 1_000);
    let released_at = rig.arena.now_ms;
    let _ = rig.pulse(
        &[control(
            Channel::Helm,
            helm + 1,
            ControlState::Released,
            Some(POSSESSOR),
        )],
        &[],
    );
    let _ = rig.until(released_at + 5_000, |arena| {
        arena
            .minds
            .get(&RED)
            .is_some_and(|state| state.think_seq >= found.think_seq + 3)
    });
    let after: Vec<Seen> = mind
        .seen()
        .into_iter()
        .filter(|seen| seen.unit == RED && seen.t_ms >= released_at)
        .collect();
    assert!(after.len() >= 3, "{} thinks after the release", after.len());
    assert!(
        after.iter().all(|seen| seen.dropped),
        "every think after the release is the ROM's"
    );
    assert_eq!(
        rig.arena.minds.get(&RED).and_then(|state| state.dropped),
        Some(tacsim::arena::DropCause::Strikes),
        "still dropped"
    );
    assert_eq!(
        rig.lines("mind_dropped", RED).len(),
        1,
        "no second drop line"
    );
}

/// **Who a `Released` hands a hull to** (17b cut M4.3 over M3.5's rule;
/// `apply_control`; doc 17 §12; 16b cut N.1): EVERY hull goes to a mind — a
/// hull seated from its check-out has no row and is laid a fresh one carrying
/// the Withdraw seed (`withdraw_seed`: the stock keys in `Withdraw` under no
/// order, the cache reading the same).
///
/// **17b cut M4.5a**: two reflex halves are gone. The `no reflex row` probe
/// on the released hull is dropped (vacuous: the hull is asserted a mind with
/// its seeded row beside it), and the Mind → Reflex leg (`set_controller`, a
/// fixture's transition alone, removing the mind row) is deleted — with the
/// reflex controller's fixtures gone the transition ceases to exist, and
/// 17b cut M4.6 deletes the controller.
#[test]
fn m4_3_released_hands_every_hull_to_a_mind_seeded_withdraw() {
    let (mut rig, _) = possess_field(Vec::new());
    let _ = rig.until(3_000, |arena| arena.now_ms >= 2_000);
    assert!(
        !rig.arena.minds.contains_key(&BLUE),
        "a seated hull has no row"
    );
    let epoch = rig.arena.epochs.current(BLUE, Channel::Helm) + 1;
    let _ = rig.pulse(
        &[Control {
            unit: BLUE,
            channel: Channel::Helm,
            epoch,
            state: ControlState::Released,
            pilot: Some(ObjId(1_000)),
        }],
        &[],
    );
    assert_eq!(
        rig.arena.unit(BLUE).map(|unit| unit.controller),
        Some(Controller::Mind),
        "a released hull with no row is handed to a fresh mind"
    );
    let laid = rig.arena.minds.get(&BLUE).expect("a mind row laid");
    assert_eq!(laid.doctrine_state, MachineState::Withdraw, "seeded");
    assert_eq!(doctrine_state_of(&laid.memory), MachineState::Withdraw);
    assert!(
        !tacsim::arena::stock_mind::StockMemory::from(&decode_memory(&laid.memory)).ordered,
        "a release's Withdraw is nobody's order"
    );
    assert_eq!(laid.memory, withdraw_seed(), "a fresh row carries the seed");
    assert_eq!(laid.order, None);
    assert_eq!(rig.arena.machine_state(BLUE), Some(MachineState::Withdraw));
}

/// **The possess-and-release run** — [`seat_run`]'s shape over red flown by
/// the native stock mind (the default runner) with only blue seated: each
/// beat a control raising the helm epoch by one and naming pilot `#1000`, or
/// that pilot's order, delivered in the first pulse whose arena time has
/// reached it (every beat a multiple of 5,000).
fn possess_run(cadence_ms: i64, until_ms: i64, beats: &[(i64, Beat)]) -> (Run, Arena) {
    let mut arena = arena_of(
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
        ],
        &[BLUE],
    );
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut counters = MotorCounters::default();
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
        counters.absorb(&out.motor);
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

/// **A possess and a release over a mind hull at three cadences** (17b cut
/// M3.5; 16b §11's mandatory cross-tempo fixture; D-16-2): the native stock
/// mind flies red; a pilot takes it at 5,000, turns it at 10,000 and lets it
/// go at 15,000; the mind resumes on its dormant row to 30,000. At 5,000,
/// 1,000 and 500 ms one event list and one hash sequence — and the mind
/// thought before the possess and after the release, never between.
#[test]
fn m3_a_possess_and_a_release_hold_at_three_cadences() {
    let beats = [
        (5_000, Beat::Control(ControlState::Live)),
        (10_000, Beat::Order(IntentKind::Heading { mdeg: 180_000 })),
        (15_000, Beat::Control(ControlState::Released)),
    ];
    let (first, arena) = possess_run(5_000, 30_000, &beats);
    assert_eq!(
        arena.unit(RED).map(|unit| unit.controller),
        Some(Controller::Mind),
        "the mind flies it at the end"
    );
    let thinks = arena.minds.get(&RED).map_or(0, |state| state.think_seq);
    assert!(
        (15..25).contains(&thinks),
        "thinks before the possess and after the release, none between: {thinks}"
    );
    assert!(!first.events.is_empty());
    for cadence in [1_000, 500] {
        let (other, _) = possess_run(cadence, 30_000, &beats);
        assert_eq!(
            other.events, first.events,
            "the event list diverged at {cadence} ms"
        );
        assert_eq!(
            other.hashes, first.hashes,
            "the hash sequence diverged at {cadence} ms"
        );
        assert_ne!(other.pulses, first.pulses);
    }
}

/// **The mixed shipped field** (17b cut M3.5): red's slot 1 sealed on a
/// scripted mind ([`support::Muster::scripted`]) and slot 2 on the native
/// one ([`support::Muster::mind`]); blue's slot 1 the same pair when `both`,
/// else a seated pilot. Answers the arena (the production installer has run)
/// and the sources it was handed.
fn mixed_field(world: &mush_testkit::TestWorld, both: bool) -> (Arena, BTreeMap<[u8; 32], String>) {
    let red = [
        support::Muster::scripted(Team::Red, 1),
        support::Muster::mind(Team::Red, 2),
    ];
    let blue_minds = [
        support::Muster::scripted(Team::Blue, 1),
        support::Muster::mind(Team::Blue, 2),
    ];
    let blue_pilot = [support::Muster::piloted(Team::Blue, 1)];
    let blue: &[support::Muster] = if both { &blue_minds } else { &blue_pilot };
    let sides: [&[support::Muster]; 2] = [&red, blue];
    let custody = support::shipped_custody_seeded(world, &sides, support::SEED);
    let sources = support::sealed_sources(&custody.units);
    (support::shipped_arena(world, &sides), sources)
}

/// **Each hull thinks on its own runner, by its seal** (17b cut M3.5; doc 17
/// §13; `Arena::runners`, `Arena::runner_for`). On the mixed field the
/// production installer gave the scripted hull — and only it — a
/// `RhaiRunner` over Standard Doctrine's source, compiled under its own
/// digest; the native hull thinks on the arena-wide fallback, which stays the
/// native stock mind. Then the asserting twin (Gate A: T1 and T2 on one job,
/// panicking on the first difference) is installed as the SCRIPTED hull's
/// runner through `runners`, and the fallback is wrapped in a recorder: over
/// twenty seconds every think the twin compared is the scripted hull's, every
/// think the fallback answered is the native hull's, and each thought.
#[test]
fn m3_each_hull_thinks_on_its_own_runner_by_its_seal() {
    let world = support::booted_world();
    let (mut arena, _) = mixed_field(&world, false);
    let scripted = support::tank(&world, Team::Red, 1);
    let native = support::tank(&world, Team::Red, 2);
    assert_eq!(
        arena.unit(scripted).map(|unit| unit.seal.engine),
        Some(tacsim::arena::SealEngine::Scripted)
    );
    assert_eq!(
        arena.unit(native).map(|unit| unit.seal.engine),
        Some(tacsim::arena::SealEngine::Native)
    );
    assert_eq!(
        arena.runners.keys().copied().collect::<Vec<_>>(),
        vec![scripted],
        "the scripted seal alone was given a runner"
    );
    let standard = format!("{:?}", tacsim::minds::standard_digest());
    let shown = format!("{:?}", arena.runner_for(scripted));
    assert!(
        shown.starts_with("RhaiRunner") && shown.contains(&standard),
        "Standard Doctrine's script under its own digest: {shown:.60}"
    );
    assert!(format!("{:?}", arena.runner_for(native)).starts_with("NativeStockMind"));
    assert!(
        format!("{:?}", arena.mind_runner).starts_with("NativeStockMind"),
        "the fallback is untouched"
    );

    // The scripted hull's own runner is admitted as the stock mind because
    // its seal installed it (`Stock::of`).
    let _ = Stock::of(&arena, scripted);
    let twin = Arc::new(support::TwinRunner::new(&arena.arena_cfg));
    arena.runners.insert(scripted, twin.clone());
    let fallback = Stock::on(&arena);
    arena.mind_runner = fallback.clone();
    let _ = support::run_until(&mut arena, 20_000, 1_000, |_, _| {});
    let compared = twin.thoughts();
    assert!(compared.len() >= 15, "{} thinks compared", compared.len());
    assert!(
        compared.iter().all(|thought| thought.unit == scripted),
        "the twin thought for the scripted hull alone"
    );
    assert_eq!(twin.divergences(), (0, 0), "T2 is T1 on every think");
    let answered = fallback.kept();
    assert!(answered.len() >= 15, "{} native thinks", answered.len());
    assert!(
        answered.iter().all(|(unit, ..)| *unit == native),
        "the fallback thought for the native hull alone"
    );
}

/// **A decode reinstalls each unit's runner from its seal** (17b cut M3.5;
/// doc 17 §13; the sibling of
/// [`m2_a_resumed_field_flies_the_rhai_runner_again_after_the_decode`], M2's
/// residual closed): the mixed field on both sides — one scripted seal and
/// one native a side — stepped one substep a pulse with the tempo pinned
/// ([`stepped`]), encoded at the first pulse boundary past 15,000 ms with no
/// think in flight and decoded. The decoded arena's runners are re-derived
/// EMPTY and its fallback the native stock mind; NO `with_rhai` is applied —
/// the production installer alone (`support::install_sealed`, i.e.
/// `tacsim::minds::install_sealed_runners`, over the sources the custody's
/// seals name) gives each scripted hull a `RhaiRunner` again and the native
/// hulls nothing. Each hull's runner is wrapped in a pass-through recorder:
/// to 30,000 ms every scripted hull's think is answered by its `RhaiRunner`
/// and every native hull's by the native fallback; and the stitched run is
/// the uninterrupted one's — one hash sequence, one ledger byte for byte,
/// one final state hash.
#[test]
fn m3_a_decode_reinstalls_each_unit_s_runner_from_its_seal() {
    const UNTIL_MS: i64 = 30_000;
    let world = support::booted_world();
    let scripted = [
        support::tank(&world, Team::Red, 1),
        support::tank(&world, Team::Blue, 1),
    ];
    let native = [
        support::tank(&world, Team::Red, 2),
        support::tank(&world, Team::Blue, 2),
    ];

    // The uninterrupted run.
    let (mut straight, _) = mixed_field(&world, true);
    let (mut straight_bytes, straight_hashes, _) = stepped(&mut straight, 0, UNTIL_MS);
    let straight_events = events(&mut straight, &mut straight_bytes);

    // The same field, to a pulse boundary past 15 s with no think in flight.
    let (mut flown, sources) = mixed_field(&world, true);
    assert_eq!(sources.len(), 2, "two scripted seals name a source");
    let (mut bytes, mut hashes, mut wall) = stepped(&mut flown, 0, 15_000);
    while flown.minds.values().any(|state| state.pending.is_some()) {
        let next = flown.now_ms + SUBSTEP;
        let (more, more_hashes, after) = stepped(&mut flown, wall, next);
        bytes.extend(more);
        hashes.extend(more_hashes);
        wall = after;
    }
    let snapshot = Snapshot::encode(&flown).expect("the arena encodes");
    let mut resumed = Snapshot::decode(&snapshot.document, (*flown.definition).clone())
        .expect("the document decodes");
    assert!(resumed.runners.is_empty(), "re-derived empty at the decode");
    assert!(format!("{:?}", resumed.mind_runner).starts_with("NativeStockMind"));
    assert_eq!(
        support::install_sealed(&mut resumed, &sources),
        2,
        "the production installer alone"
    );
    // Each hull's runner, recorded: a scripted hull's admitted by `Stock::of`
    // only because its seal installed a Standard Doctrine `RhaiRunner`; a
    // native hull's the fallback, the native stock mind.
    let mut recorders: Vec<(ObjId, Arc<Stock>)> = Vec::new();
    for unit in scripted {
        recorders.push((unit, Stock::of(&resumed, unit)));
    }
    for unit in native {
        assert!(
            !resumed.runners.contains_key(&unit),
            "a native seal installs nothing"
        );
        let shown = format!("{:?}", resumed.runner_for(unit));
        assert!(
            shown.starts_with("NativeStockMind"),
            "#{}'s runner is the fallback: {shown:.40}",
            unit.0
        );
        recorders.push((unit, Stock::over(Arc::clone(resumed.runner_for(unit)))));
    }
    for (unit, recorder) in &recorders {
        resumed.runners.insert(*unit, recorder.clone());
    }
    let now = resumed.now_ms;
    let _ = resumed.resume_at(now);
    let (more, more_hashes, _) = stepped(&mut resumed, now + SUBSTEP, UNTIL_MS);
    bytes.extend(more);
    hashes.extend(more_hashes);
    let resumed_events = events(&mut resumed, &mut bytes);
    for (unit, recorder) in &recorders {
        let kept = recorder.kept();
        assert!(kept.len() >= 10, "#{} thought {} times", unit.0, kept.len());
        assert!(
            kept.iter().all(|(on, ..)| on == unit),
            "#{}'s runner thought for it alone",
            unit.0
        );
    }
    assert!(
        !straight_events.is_empty() && hashes.len() > 100,
        "the run journalled and stepped"
    );
    assert_eq!(hashes, straight_hashes, "the stitched hash sequence");
    assert_eq!(resumed_events, straight_events, "the stitched ledger");
    assert_eq!(bytes, straight_bytes, "byte for byte");
    assert_eq!(
        tacsim::arena::state_hash(&resumed),
        tacsim::arena::state_hash(&straight),
        "the final state hash"
    );
}

/// **A `Released` over a hull its mind is flying changes no controller** (17b
/// cut M3.5; doc 17 §12, `apply_control`'s `Released` rule): the control's
/// own half still stands — the speed is zeroed and the helm epoch raised —
/// but the hull is handed to the controller it already has
/// (`set_controller(Mind → Mind)` returns at once). With think 1 IN FLIGHT:
///
/// * after the control the controller is still `Mind`, the row is
///   byte-equal to the one the control found (its `pending` answer
///   included), the landing is still on the heap, and `desired_speed_cms` is
///   0 where think 0 had set 500;
/// * the landing then lands: the row takes think 1's memory and its speed is
///   refused `epoch` into the buffer (the control raised the helm epoch
///   after the think was taken — D-UM-13), so the speed stays 0;
/// * the next think, taken at the new epoch, sets the speed again.
#[test]
fn m3_a_released_over_a_hull_its_mind_flies_changes_no_controller() {
    let (mut rig, _) = possess_field(vec![
        Says {
            memory: Some(vec![7]),
            ..says(0, vec![IntentKind::Speed { cms: 500 }])
        },
        Says {
            memory: Some(vec![8]),
            ..says(1, vec![IntentKind::Speed { cms: 600 }])
        },
        says(2, vec![IntentKind::Speed { cms: 700 }]),
    ]);
    if let Some(unit) = rig.arena.unit_mut(RED) {
        unit.power = Power::Online;
    }
    let _ = rig.until(10_000, |arena| {
        arena
            .minds
            .get(&RED)
            .and_then(|state| state.pending.as_ref())
            .is_some_and(|pending| pending.think_seq == 1)
    });
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.desired_speed_cms),
        Some(500),
        "think 0 landed its speed"
    );
    let found = rig.arena.minds.get(&RED).cloned().expect("the mind's row");
    assert!(
        mind_events_of(&rig.arena, RED),
        "the landing is on the heap"
    );

    let epoch = rig.arena.epochs.current(RED, Channel::Helm) + 1;
    apply_control(
        &mut rig.arena,
        &control(Channel::Helm, epoch, ControlState::Released, None),
    );
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.controller),
        Some(Controller::Mind),
        "the mind keeps the hull"
    );
    assert_eq!(
        rig.arena.minds.get(&RED),
        Some(&found),
        "the row is untouched, its answer still pending"
    );
    assert!(
        rig.arena.due.iter().any(|event| matches!(
            event.kind,
            DueKind::MindLand { unit, think_seq: 1 } if unit == RED
        )),
        "the landing still stands"
    );
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.desired_speed_cms),
        Some(0),
        "the release zeroes the speed"
    );

    let _ = rig.until(10_000, |arena| !in_flight(arena, RED));
    let landed = rig.arena.minds.get(&RED).expect("the mind's row");
    assert_eq!(landed.memory, vec![8], "think 1's answer landed");
    assert_eq!(landed.think_seq, found.think_seq, "no think since");
    assert!(
        refusals(&rig.arena, RED).contains(&("speed".to_owned(), Refusal::Epoch)),
        "{:?}",
        refusals(&rig.arena, RED)
    );
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.desired_speed_cms),
        Some(0),
        "the stale speed is refused"
    );

    let _ = rig.until(20_000, |arena| {
        arena
            .unit(RED)
            .is_some_and(|unit| unit.desired_speed_cms == 700)
    });
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.desired_speed_cms),
        Some(700),
        "think 2 flies it again"
    );
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.controller),
        Some(Controller::Mind)
    );
}

// ---------------------------------------------------------------------------
// 17b cut M5.2: the sealed set and the active profile (doc 17 §11, §12;
// D-UM-44; the owner's OQ-3) — `IntentKind::Profile`, the row on the active
// profile's card, the `Withdraw` seed gated on the doctrine bit
// ---------------------------------------------------------------------------

/// The pilot seated in red by [`profile_field`] (`arena_of`'s first seat).
const RED_PILOT: ObjId = ObjId(1_000);

/// **`unit` sealed with `names` as its profiles** — each its preset over this
/// rig's doctrine, the first the one that flies at the release — under the
/// fixture terms otherwise (`SealTerms::with_profiles`).
fn sealed_with(mut unit: CustodyUnit, names: &[&str]) -> CustodyUnit {
    let reflex = reflex_cfg();
    let profiles = names
        .iter()
        .map(|name| {
            (
                SmolStr::new(*name),
                tacsim::arena::preset(name, &reflex).expect("a preset"),
            )
        })
        .collect();
    unit.reseal(&tacsim::match_::SealTerms::fixture().with_profiles(profiles))
        .expect("the profiles seal");
    unit
}

/// **The profile field**: red armed and sealed with `names`, flown by
/// `controller` — seated by [`RED_PILOT`] when it is `Controller::Pilot` —
/// and a presenting blue pilot it can see.
fn profile_field(controller: Controller, names: &[&str]) -> Arena {
    let red = sealed_with(
        tank(RED, Team::Red, centre(6, 8), vec![cannon()], controller),
        names,
    );
    let blue = presenting(tank(
        BLUE,
        Team::Blue,
        centre(9, 8),
        Vec::new(),
        Controller::Pilot,
    ));
    let seated: &[ObjId] = if controller == Controller::Pilot {
        &[RED, BLUE]
    } else {
        &[BLUE]
    };
    arena_of(vec![red, blue], seated)
}

/// **The card every think was handed** — `(unit, t_ms, card)` per think —
/// over the arena's own runner, which answers untouched (an input recorder,
/// 16b §0.4).
#[derive(Debug)]
struct CardsSeen {
    inner: Arc<dyn MindRunner>,
    seen: Mutex<Vec<(ObjId, i64, tacsim::arena::StockCard)>>,
}

impl MindRunner for CardsSeen {
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        if let Ok(mut seen) = self.seen.lock() {
            seen.push((
                job.unit,
                job.t_ms,
                job.card.stock().expect("stock fixture card").clone(),
            ));
        }
        self.inner.think(job)
    }
}

impl CardsSeen {
    /// The cards `unit`'s thinks were handed, in order.
    fn of(&self, unit: ObjId) -> Vec<tacsim::arena::StockCard> {
        self.seen
            .lock()
            .map(|seen| {
                seen.iter()
                    .filter(|(on, ..)| *on == unit)
                    .map(|(_, _, card)| card.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Wrap `arena`'s runner in a [`CardsSeen`].
fn cards_seen(arena: &mut Arena) -> Arc<CardsSeen> {
    let seen = Arc::new(CardsSeen {
        inner: Arc::clone(&arena.mind_runner),
        seen: Mutex::new(Vec::new()),
    });
    arena.mind_runner = seen.clone();
    seen
}

/// `unit`'s active profile.
fn active(arena: &Arena, unit: ObjId) -> Option<tacsim::arena::card::ActiveProfile> {
    arena.unit(unit).map(|state| state.card_profile)
}

/// `unit`'s row's card.
fn row_card(arena: &Arena, unit: ObjId) -> Option<tacsim::arena::StockCard> {
    arena
        .unit(unit)
        .and_then(|state| arena.active_card(state))
        .and_then(|card| card.stock())
        .cloned()
}

/// The `mind_card` lines journalled so far, as `(t_ms, line)`.
fn mind_card_lines(rig: &mut Rig) -> Vec<(i64, serde_json::Value)> {
    rig.journal()
        .iter()
        .filter(|event| event.kind.as_str() == "mind_card")
        .map(|event| {
            (
                event.t_ms,
                serde_json::to_value(&event.kind).unwrap_or_default(),
            )
        })
        .collect()
}

/// A `MIND PROFILE` from red's seat, switching to `index`.
fn switch_to(arena: &Arena, index: u8, seq: u64) -> Intent {
    intent(arena, RED, IntentKind::Profile { index }, seq)
}

/// Red's seat released by its pilot, at the next helm epoch.
fn release_red(arena: &Arena) -> Control {
    control(
        Channel::Helm,
        arena.epochs.current(RED, Channel::Helm) + 1,
        ControlState::Released,
        Some(RED_PILOT),
    )
}

/// **The fill flies its lead's preset** (17b cut M5.2; the design note's
/// items 5 and 10; Q-UM-22's check): a mind hull sealed with one profile,
/// Sniper — the shape `match_::start` seals a side's fill in when its lead
/// chose `SIM AI MIND Standard PROFILE Sniper` — is laid at check-out on
/// Sniper's card over the arena's own doctrine (`Arena::adopt_mind` through
/// `Arena::active_card`), and its first think is handed that card; its seal
/// reference holds the set as preset codes (`[2]`). The same hull sealed on
/// the Line card alone is laid on Line, as every hull always was. Nothing
/// the persona's deliberated orders assume reads the card (orders steer the
/// objective; the card is the mind's) — checked here by the order landing
/// on the Sniper row exactly as on a Line one.
#[test]
fn m5_the_fill_flies_the_leads_preset() {
    let mut rig = Rig::new(profile_field(Controller::Mind, &["Sniper"]));
    let seen = cards_seen(&mut rig.arena);
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit
            .seal
            .profiles
            .names()
            .map(str::to_owned)
            .collect::<Vec<_>>()),
        Some(vec!["Sniper".to_owned()]),
        "the sealed ordered names, with catalog keys retained in the seal"
    );
    assert_eq!(
        row_card(&rig.arena, RED),
        Some(preset_on_field("sniper")),
        "laid on Sniper at check-out"
    );
    let _ = rig.until(5_000, |arena| {
        arena
            .minds
            .get(&RED)
            .is_some_and(|state| state.think_seq >= 1)
    });
    assert_eq!(
        seen.of(RED).first(),
        Some(&preset_on_field("sniper")),
        "the first think is handed Sniper"
    );
    let hold = intent(
        &rig.arena,
        RED,
        IntentKind::Order(Box::new(order(RED_ONE, Objective::Hold, None, 60_000))),
        1,
    );
    let _ = rig.pulse(&[], &[hold]);
    assert!(
        rig.arena
            .minds
            .get(&RED)
            .and_then(|state| state.order.as_ref())
            .is_some_and(|order| order.objective == MindObjective::Hold),
        "the lead's order lands on the Sniper row as on any"
    );
    let line = Rig::new(profile_field(Controller::Mind, &[]));
    assert_eq!(
        row_card(&line.arena, RED),
        Some(preset_on_field("line")),
        "the Line card alone lays Line"
    );
    assert_eq!(
        line.arena.unit(RED).map(|unit| unit
            .seal
            .profiles
            .names()
            .map(str::to_owned)
            .collect::<Vec<_>>()),
        Some(vec!["Line".to_owned()])
    );
}

/// **The row's card digests as its sealed profile, for every preset** (17b
/// cut M5.2; the design note's item 5): a hull sealed with each of the six
/// presets alone is laid a row whose card — derived from the arena's own
/// doctrine at the stock config, as Line always was — is byte for byte the
/// card the seal digested (`MindSeal::card_digest`, and the set's one
/// complete schema/values/rules bytes and exact immutable catalog key).
#[test]
fn m5_the_row_card_digests_as_the_sealed_profile_for_every_preset() {
    use sha2::{Digest as _, Sha256};
    for name in tacsim::arena::card::PRESETS {
        let unit = sealed_with(
            tank(
                RED,
                Team::Red,
                centre(6, 8),
                vec![cannon()],
                Controller::Mind,
            ),
            &[name],
        );
        let seal = unit.seal.clone();
        let arena = arena_of(
            vec![
                unit,
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
        let card = arena
            .unit(RED)
            .and_then(|unit| arena.active_card(unit))
            .expect("the exact runtime card");
        let bytes = card.canonical_bytes().expect("hydrated complete card");
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        assert_eq!(bytes, seal.card, "{name}: exact opening card bytes");
        assert_eq!(digest, seal.card_digest, "{name}: the sealed complete card");
        assert_eq!(seal.cards.selected.names().collect::<Vec<_>>(), vec![name]);
        assert_eq!(seal.cards.selected.key(0), Some(card.key()));
        assert_eq!(
            card.stock(),
            Some(&preset_on_field(name)),
            "{name}: stock adapter"
        );
    }
}

/// **Past the grace, a pilot's hull flies its sealed profile** (17b cut
/// M5.2; doc 17 §12 "Release and possess"; the design note's item 5): red,
/// seated and sealed `[Sniper, Line]`, has no mind row while its pilot flies
/// it; the `Released` that ends the grace hands it to a mind laid on its
/// FIRST profile, Sniper — seeded `Withdraw` (a Standard Doctrine seal) —
/// and that mind's first think is handed Sniper's card.
#[test]
fn m5_past_the_grace_a_pilots_hull_flies_its_sealed_profile() {
    let mut rig = Rig::new(profile_field(Controller::Pilot, &["Sniper", "Line"]));
    let seen = cards_seen(&mut rig.arena);
    for _ in 0..4 {
        let _ = rig.pulse(&[], &[]);
    }
    assert!(!rig.arena.minds.contains_key(&RED), "no row under a pilot");
    let released = release_red(&rig.arena);
    let _ = rig.pulse(&[released], &[]);
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.controller),
        Some(Controller::Mind)
    );
    let row = rig.arena.minds.get(&RED).expect("a mind's row");
    assert_eq!(
        row_card(&rig.arena, RED),
        Some(preset_on_field("sniper")),
        "the first profile"
    );
    assert_eq!(
        doctrine_state_of(&row.memory),
        MachineState::Withdraw,
        "seeded Withdraw: a Standard Doctrine seal"
    );
    let _ = rig.until(5_000, |arena| {
        arena
            .minds
            .get(&RED)
            .is_some_and(|state| state.think_seq >= 1)
    });
    assert_eq!(seen.of(RED).first(), Some(&preset_on_field("sniper")));
}

/// **A switch while seated is flown at the release** (17b cut M5.2; doc 17
/// §11, D-UM-44; the design note's item 4): red's pilot, sealed `[Sniper,
/// Line]`, switches to Line (`IntentKind::Profile { index: 1 }`) — refused
/// nothing, told nothing — and the arena writes the active profile (index
/// 1, stamped at the pulse's start) and journals ONE `mind_card` line there:
/// `Line`, the sliders the switch moved off Sniper, the whole card. No row
/// stands under the pilot, so none is written. The `Released` lays the mind
/// on Line, and its first think is handed Line's card.
#[test]
fn m5_a_profile_switch_while_seated_is_flown_at_the_release() {
    let mut rig = Rig::new(profile_field(Controller::Pilot, &["Sniper", "Line"]));
    let seen = cards_seen(&mut rig.arena);
    for _ in 0..4 {
        let _ = rig.pulse(&[], &[]);
    }
    let at = rig.arena.now_ms;
    let switch = switch_to(&rig.arena, 1, 1);
    let out = rig.pulse(&[], &[switch]);
    assert!(pilot_refusals(&out, RED_PILOT).is_empty(), "told nothing");
    assert_eq!(
        active(&rig.arena, RED),
        Some(tacsim::arena::card::ActiveProfile {
            index: 1,
            switched_ms: Some(at),
        })
    );
    assert!(!rig.arena.minds.contains_key(&RED), "no row is laid by it");
    let lines = mind_card_lines(&mut rig);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let (t_ms, line) = &lines[0];
    assert_eq!(*t_ms, at, "at the pulse's start");
    assert_eq!(line["unit"], RED.0);
    assert_eq!(line["name"], "Line");
    // Profile changes report canonical authored field names; stock slider
    // edits retain their human-facing adapter names in their own tests.
    assert_eq!(
        line["changed"],
        serde_json::json!(["engage_band_permille", "min_odds_permille", "reserve_pct"])
    );
    let card: tacsim::arena::StockCard =
        serde_json::from_value(line["stock"].clone()).expect("the diagnostic stock adapter");
    assert_eq!(card, preset_on_field("line"));
    let released = release_red(&rig.arena);
    let _ = rig.pulse(&[released], &[]);
    assert_eq!(row_card(&rig.arena, RED), Some(preset_on_field("line")));
    let _ = rig.until(at + 5_000, |arena| {
        arena
            .minds
            .get(&RED)
            .is_some_and(|state| state.think_seq >= 1)
    });
    assert_eq!(seen.of(RED).first(), Some(&preset_on_field("line")));
}

/// **The profile driver**: [`profile_field`] with red seated and sealed
/// `[Line, Sniper]`, its runner recorded ([`CardsSeen`]), stepped at
/// `cadence_ms` to `until_ms` through `beats` — each delivered in the first
/// pulse whose arena time has reached it ([`seat_run`]'s rule; every beat a
/// multiple of 5,000). A control raises the helm epoch and names the seat's
/// pilot. Answers the run, the cards seen and the arena.
fn profile_run(
    cadence_ms: i64,
    until_ms: i64,
    beats: &[(i64, Beat)],
) -> (Run, Vec<tacsim::arena::StockCard>, Arena) {
    let mut arena = profile_field(Controller::Pilot, &["Line", "Sniper"]);
    let seen = cards_seen(&mut arena);
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut counters = MotorCounters::default();
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
                    Some(RED_PILOT),
                )),
            }
            next += 1;
        }
        let out = arena.step_settled(wall, &controls, &intents);
        pulses += 1;
        for chunk in &out.ledger_chunks {
            bytes.extend_from_slice(&chunk.bytes);
        }
        counters.absorb(&out.motor);
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
        seen.of(RED),
        arena,
    )
}

/// **A profile switch is one event list and one hash sequence at three
/// cadences** (17b cut M5.2; 16b §11's mandatory cross-tempo property, 17b
/// §0.4; the design note's gate evidence): red's pilot, sealed `[Line,
/// Sniper]`, switches to Sniper at 5,000 and is released at 10,000; the
/// mind flies Sniper to 30,000. Stepped (settled) at 5,000, 1,000 and 500 ms:
/// one event list, one hash sequence, one card sequence — and the one
/// `mind_card` line at 5,000 names Sniper.
#[test]
fn m5_a_profile_switch_is_one_event_list_and_one_hash_sequence_at_three_cadences() {
    let beats = [
        (5_000, Beat::Order(IntentKind::Profile { index: 1 })),
        (10_000, Beat::Control(ControlState::Released)),
    ];
    let (first, cards, arena) = profile_run(5_000, 30_000, &beats);
    let switched: Vec<(i64, serde_json::Value)> = first
        .events
        .iter()
        .filter_map(|(t_ms, json, _)| {
            let value: serde_json::Value = serde_json::from_str(json).ok()?;
            (value["kind"] == "mind_card").then_some((*t_ms, value))
        })
        .collect();
    assert_eq!(switched.len(), 1, "{switched:?}");
    assert_eq!(switched[0].0, 5_000);
    assert_eq!(switched[0].1["name"], "Sniper");
    assert_eq!(
        active(&arena, RED),
        Some(tacsim::arena::card::ActiveProfile {
            index: 1,
            switched_ms: Some(5_000),
        })
    );
    assert_eq!(row_card(&arena, RED), Some(preset_on_field("sniper")));
    assert!(cards.len() >= 15, "{} thinks", cards.len());
    assert!(
        cards.iter().all(|card| *card == preset_on_field("sniper")),
        "every think after the release flies Sniper"
    );
    for cadence in [1_000, 500] {
        let (other, other_cards, _) = profile_run(cadence, 30_000, &beats);
        assert_eq!(other.events, first.events, "the event list at {cadence}");
        assert_eq!(other.hashes, first.hashes, "the hash sequence at {cadence}");
        assert_eq!(other_cards, cards, "the cards at {cadence}");
        assert_ne!(other.pulses, first.pulses);
    }
}

/// **A second switch inside ten seconds is refused `profile`** (17b cut
/// M5.2; doc 17 D-UM-44: "at most once per 10 s"; the design note's item
/// 4): red's pilot, sealed `[Sniper, Line]`, switches to Line; a switch back
/// 9,750 ms later is refused `profile` — told to the pilot as `MIND refused.
/// That profile cannot be switched to now.` — and changes nothing, journals
/// nothing; at 10,000 ms it lands. So are refused: an index past the sealed
/// set, the profile already active, and a hull with no seal
/// (`SealRef::none`).
#[test]
fn m5_a_second_switch_inside_ten_seconds_is_refused_profile() {
    let mut rig = Rig::new(profile_field(Controller::Pilot, &["Sniper", "Line"]));
    for _ in 0..4 {
        let _ = rig.pulse(&[], &[]);
    }
    let first_at = rig.arena.now_ms;
    let switch = switch_to(&rig.arena, 1, 1);
    let _ = rig.pulse(&[], &[switch]);
    let landed = active(&rig.arena, RED);
    assert_eq!(landed.map(|profile| profile.index), Some(1));
    while rig.arena.now_ms < first_at + 9_750 {
        let _ = rig.pulse(&[], &[]);
    }
    let lines_before = mind_card_lines(&mut rig).len();
    let again = switch_to(&rig.arena, 0, 2);
    let out = rig.pulse(&[], &[again]);
    assert_eq!(
        pilot_refusals(&out, RED_PILOT),
        vec![("mind".to_owned(), Refusal::Profile)],
        "told to the pilot"
    );
    let told: Vec<String> = out
        .frames
        .iter()
        .filter(|(to, _)| *to == RED_PILOT)
        .flat_map(|(_, frame)| tacsim::match_::cockpit::notice_lines(frame))
        .collect();
    assert_eq!(
        told,
        vec!["MIND refused. That profile cannot be switched to now.".to_owned()]
    );
    assert_eq!(active(&rig.arena, RED), landed, "nothing changed");
    assert_eq!(mind_card_lines(&mut rig).len(), lines_before);
    assert_eq!(rig.arena.now_ms, first_at + 10_000);
    let at_ten = switch_to(&rig.arena, 0, 3);
    let out = rig.pulse(&[], &[at_ten]);
    assert!(pilot_refusals(&out, RED_PILOT).is_empty());
    assert_eq!(
        active(&rig.arena, RED),
        Some(tacsim::arena::card::ActiveProfile {
            index: 0,
            switched_ms: Some(first_at + 10_000),
        }),
        "ten seconds on, it lands"
    );
    // Past the set, and the one already active.
    for refused in [2, 0] {
        let mut later = Rig::new(profile_field(Controller::Pilot, &["Sniper", "Line"]));
        let _ = later.pulse(&[], &[]);
        let switch = switch_to(&later.arena, refused, 1);
        assert_eq!(
            apply_intent(&mut later.arena, &switch, IntentSource::Seat),
            IntentOutcome::Refused(Refusal::Profile),
            "index {refused}"
        );
        assert_eq!(
            active(&later.arena, RED),
            Some(tacsim::arena::card::ActiveProfile::FIRST)
        );
    }
    // No seal: `SealRef::none`'s zero digest, over the hull's own two-profile
    // set kept as sealed — so the switch to index 1 (in the set, not the
    // active one, no stamp) is refused by the digest alone (17b cut M5.2's
    // review: switching to index 0 would be refused as already active).
    let mut unsealed = Rig::new(profile_field(Controller::Pilot, &["Sniper", "Line"]));
    let _ = unsealed.pulse(&[], &[]);
    if let Some(unit) = unsealed.arena.unit_mut(RED) {
        assert_eq!(unit.seal.profiles.len(), 2, "the set stands");
        unit.seal.digest = tacsim::arena::mind::SealRef::none().digest;
    }
    assert_eq!(
        active(&unsealed.arena, RED),
        Some(tacsim::arena::card::ActiveProfile::FIRST)
    );
    let switch = switch_to(&unsealed.arena, 1, 1);
    assert_eq!(
        apply_intent(&mut unsealed.arena, &switch, IntentSource::Seat),
        IntentOutcome::Refused(Refusal::Profile),
        "a hull with no seal"
    );
    assert_eq!(
        active(&unsealed.arena, RED),
        Some(tacsim::arena::card::ActiveProfile::FIRST)
    );
}

/// **A cold or immobile hull switches** (17b cut M5.2; the design note's
/// item 4: the switch's own arm comes before the continuous gates — the
/// profile is what the mind will fly, not motion). On a cold tank a heading
/// is refused `power_not_online` and the switch lands; on a tank whose
/// tracks are out (motion disabled) a heading is refused `motion_disabled`
/// and the switch lands.
#[test]
fn m5_a_cold_or_immobile_hull_switches() {
    for (power, immobile, heading_refused) in [
        (Power::Off, false, Refusal::PowerNotOnline),
        (Power::Online, true, Refusal::MotionDisabled),
    ] {
        let mut arena = profile_field(Controller::Pilot, &["Sniper", "Line"]);
        if let Some(unit) = arena.unit_mut(RED) {
            unit.power = power;
            unit.limits.motion_disabled = immobile;
        }
        let heading = intent(&arena, RED, IntentKind::Heading { mdeg: 90_000 }, 1);
        assert_eq!(
            apply_intent(&mut arena, &heading, IntentSource::Seat),
            IntentOutcome::Refused(heading_refused)
        );
        let switch = switch_to(&arena, 1, 2);
        assert_eq!(
            apply_intent(&mut arena, &switch, IntentSource::Seat),
            IntentOutcome::AppliedContinuous,
            "{power:?}, immobile {immobile}: the switch lands"
        );
        assert_eq!(active(&arena, RED).map(|profile| profile.index), Some(1));
    }
}

/// **A switch and a heading in one pulse both land** (17b cut M5.2; the
/// design note's item 4: the switch keys under `continuous_code` 5, so a
/// `HEADING` of the same pulse keeps its own key — the buffer half is
/// `arena_task`'s in-source `m5_a_profile_switch_keys_on_its_own_code`):
/// both handed to one pulse's phase 2, the heading is set and the profile
/// switched.
#[test]
fn m5_a_switch_and_a_heading_in_one_pulse_both_land() {
    let mut rig = Rig::new(profile_field(Controller::Pilot, &["Sniper", "Line"]));
    let startup = intent(&rig.arena, RED, IntentKind::Startup, 1);
    let _ = rig.pulse(&[], &[startup]);
    let _ = rig.until(2_000, |arena| {
        arena
            .unit(RED)
            .is_some_and(|unit| unit.power == Power::Online)
    });
    let heading = intent(&rig.arena, RED, IntentKind::Heading { mdeg: 90_000 }, 2);
    let switch = switch_to(&rig.arena, 1, 3);
    let out = rig.pulse(&[], &[heading, switch]);
    assert!(pilot_refusals(&out, RED_PILOT).is_empty());
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.desired_heading_mdeg),
        Some(90_000)
    );
    assert_eq!(
        active(&rig.arena, RED).map(|profile| profile.index),
        Some(1)
    );
}

/// **A switch from a mind or a slot is refused `channel`** (17b cut M5.2;
/// the design note's item 4: a mind does not switch its own profile and no
/// station function builds one): a `Profile` applied as a mind's, a
/// machine's or a condition slot's is refused `channel` and switches
/// nothing; a seat's lands.
#[test]
fn m5_a_profile_switch_from_a_mind_or_a_slot_is_refused_channel() {
    let mut arena = profile_field(Controller::Pilot, &["Sniper", "Line"]);
    let switch = switch_to(&arena, 1, 1);
    for source in [
        IntentSource::Mind,
        IntentSource::Machine,
        IntentSource::Cond,
    ] {
        assert_eq!(
            apply_intent(&mut arena, &switch, source),
            IntentOutcome::Refused(Refusal::Channel),
            "{source:?}"
        );
        assert_eq!(
            active(&arena, RED),
            Some(tacsim::arena::card::ActiveProfile::FIRST)
        );
    }
    assert_eq!(
        apply_intent(&mut arena, &switch, IntentSource::Seat),
        IntentOutcome::AppliedContinuous
    );
}

/// **A release hands a non-Doctrine mind its own memory** (17b cut M5.2;
/// the design note's item 6; reachable only with a fixture mind, the owner's
/// OQ-2): [`possess_field`]'s red, its seal's identity renamed off Standard
/// Doctrine (`SealRef::doctrine` false), its scripted mind writing memory
/// `[3, 1, 4]`; possessed and released, the dormant row is adopted again
/// with its memory UNTOUCHED — no `Withdraw` seed — where the same field
/// under a Standard Doctrine seal is handed the stock seed.
#[test]
fn m5_a_release_hands_a_non_doctrine_mind_its_own_memory() {
    for doctrine in [false, true] {
        let (mut rig, _mind) = possess_field(vec![Says {
            memory: Some(b"{\"fixture\":314}".to_vec()),
            ..says(0, vec![IntentKind::Startup])
        }]);
        if !doctrine && let Some(unit) = rig.arena.unit_mut(RED) {
            unit.seal.doctrine = false;
        }
        let _ = rig.until(5_000, |arena| {
            arena
                .minds
                .get(&RED)
                .is_some_and(|state| state.think_seq >= 1 && state.pending.is_none())
        });
        assert_eq!(
            rig.arena.minds.get(&RED).map(|state| state.memory.clone()),
            Some(b"{\"fixture\":314}".to_vec())
        );
        let epoch = rig.arena.epochs.current(RED, Channel::Helm) + 1;
        let _ = rig.pulse(
            &[control(
                Channel::Helm,
                epoch,
                ControlState::Live,
                Some(POSSESSOR),
            )],
            &[],
        );
        assert_eq!(
            rig.arena.unit(RED).map(|unit| unit.controller),
            Some(Controller::Pilot)
        );
        let epoch = rig.arena.epochs.current(RED, Channel::Helm) + 1;
        let _ = rig.pulse(
            &[control(
                Channel::Helm,
                epoch,
                ControlState::Released,
                Some(POSSESSOR),
            )],
            &[],
        );
        let memory = rig
            .arena
            .minds
            .get(&RED)
            .map(|state| state.memory.clone())
            .expect("the dormant row, adopted again");
        if doctrine {
            assert_eq!(memory, withdraw_seed(), "Standard Doctrine is seeded");
        } else {
            assert_eq!(
                memory,
                b"{\"fixture\":314}".to_vec(),
                "its own memory, untouched"
            );
        }
    }
}

/// **A dormant row's card follows the switch** (17b cut M5.2; the design
/// note's item 4): red, a mind sealed `[Sniper, Line]` and flying Sniper, is
/// possessed — its row stays, dormant, on Sniper — and its pilot switches to
/// Line: the dormant row's card is Line at once (the `mind_card` line's
/// `changed` read off the row's own card), and the release adopts the row
/// flying it, its next think handed Line.
#[test]
fn m5_a_dormant_rows_card_follows_the_switch() {
    let mut rig = Rig::new(profile_field(Controller::Mind, &["Sniper", "Line"]));
    let seen = cards_seen(&mut rig.arena);
    let _ = rig.until(5_000, |arena| {
        arena
            .minds
            .get(&RED)
            .is_some_and(|state| state.think_seq >= 1)
    });
    assert_eq!(row_card(&rig.arena, RED), Some(preset_on_field("sniper")));
    let epoch = rig.arena.epochs.current(RED, Channel::Helm) + 1;
    let _ = rig.pulse(
        &[control(
            Channel::Helm,
            epoch,
            ControlState::Live,
            Some(POSSESSOR),
        )],
        &[],
    );
    assert_eq!(
        rig.arena.unit(RED).map(|unit| unit.controller),
        Some(Controller::Pilot)
    );
    let switch = switch_to(&rig.arena, 1, 1);
    let out = rig.pulse(&[], &[switch]);
    assert!(pilot_refusals(&out, POSSESSOR).is_empty());
    assert_eq!(
        row_card(&rig.arena, RED),
        Some(preset_on_field("line")),
        "the dormant row's card"
    );
    let lines = mind_card_lines(&mut rig);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(
        lines[0].1["changed"],
        serde_json::json!(["engage_band_permille", "min_odds_permille", "reserve_pct"])
    );
    let thought = seen.of(RED).len();
    let epoch = rig.arena.epochs.current(RED, Channel::Helm) + 1;
    let _ = rig.pulse(
        &[control(
            Channel::Helm,
            epoch,
            ControlState::Released,
            Some(POSSESSOR),
        )],
        &[],
    );
    let at = rig.arena.now_ms;
    let _ = rig.until(at + 5_000, |_| seen.of(RED).len() > thought);
    assert_eq!(
        seen.of(RED).get(thought),
        Some(&preset_on_field("line")),
        "the release flies the switched profile"
    );
}

/// **A resume across a snapshot holding a switched profile and a chosen set
/// reproduces the hash sequence** (17b cut M5.2; the design note's gate
/// evidence; AF.3's resume, format 10): red's pilot, sealed `[Line,
/// Sniper]`, switches to Sniper at the first second; the arena is encoded
/// two seconds later — the document carries the sealed set (`[0, 2]`) and
/// the active profile (index 1, stamped) — decoded, resumed, and driven on
/// with the same beats (the release at 4,000 ms) to 12,000 ms. Every
/// substep's hash after the resume is the uninterrupted run's, the decoded
/// hull reads the set and the profile it was encoded with, and the released
/// mind flies Sniper in both.
#[test]
fn m5_a_resume_across_a_switched_profile_reproduces_the_hash_sequence() {
    /// Pulse `rig` to `until_ms`, switching at 1,000 and releasing at 4,000
    /// (each in the first pulse that reaches it), answering every substep
    /// hash from `from_ms` on.
    fn drive(rig: &mut Rig, until_ms: i64, from_ms: i64) -> Vec<u64> {
        let mut hashes = Vec::new();
        while rig.arena.now_ms < until_ms {
            let now = rig.arena.now_ms;
            let mut intents = Vec::new();
            let mut controls = Vec::new();
            if now == 1_000 {
                intents.push(switch_to(&rig.arena, 1, 1));
            }
            if now == 4_000 {
                controls.push(release_red(&rig.arena));
            }
            let _ = rig.pulse(&controls, &intents);
            if rig.arena.now_ms > from_ms {
                hashes.extend(rig.arena.substep_hashes.iter().copied());
            }
        }
        hashes
    }
    let mut whole = Rig::new(profile_field(Controller::Pilot, &["Line", "Sniper"]));
    let uninterrupted = drive(&mut whole, 12_000, 3_000);

    let mut first = Rig::new(profile_field(Controller::Pilot, &["Line", "Sniper"]));
    let _ = drive(&mut first, 3_000, 3_000);
    let switched = tacsim::arena::card::ActiveProfile {
        index: 1,
        switched_ms: Some(1_000),
    };
    assert_eq!(active(&first.arena, RED), Some(switched));
    let snapshot = Snapshot::encode(&first.arena).expect("the arena encodes");
    let decoded = Snapshot::decode(&snapshot.document, (*first.arena.definition).clone())
        .expect("format 10 decodes");
    assert_eq!(active(&decoded, RED), Some(switched), "the active profile");
    assert_eq!(
        decoded.unit(RED).map(|unit| unit
            .seal
            .profiles
            .names()
            .map(str::to_owned)
            .collect::<Vec<_>>()),
        Some(vec!["Line".to_owned(), "Sniper".to_owned()]),
        "the chosen set"
    );
    assert_eq!(
        tacsim::arena::hash::state_hash(&decoded),
        tacsim::arena::hash::state_hash(&first.arena),
        "the decode is the encoded state"
    );
    let mut resumed = Rig::new(decoded);
    // The tempo pin is the rig's, not the arena's state: a decode does not
    // carry it ([`stepped`]'s rule).
    resumed.arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let now = resumed.arena.now_ms;
    let _ = resumed.arena.resume_at(now);
    resumed.wall = first.wall;
    let stitched = drive(&mut resumed, 12_000, 3_000);
    assert!(!uninterrupted.is_empty());
    assert_eq!(stitched, uninterrupted, "one hash sequence");
    for rig in [&whole, &resumed] {
        assert_eq!(row_card(&rig.arena, RED), Some(preset_on_field("sniper")));
        assert_eq!(
            rig.arena.unit(RED).map(|unit| unit.controller),
            Some(Controller::Mind)
        );
    }
    assert_eq!(
        tacsim::arena::hash::state_hash(&resumed.arena),
        tacsim::arena::hash::state_hash(&whole.arena)
    );
}

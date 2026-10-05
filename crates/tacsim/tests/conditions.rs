//! **Condition slots, through `Arena::step`** (17b cut M1b.1; doc 17 §5 "The
//! mechanism" and "The law", D-UM-16, D-UM-17 as amended 2026-10-04, D-UM-19;
//! doc 16 §3.1's rate independence, 16b §11's mandatory cross-tempo test).
//!
//! A condition slot is the principal's shot decision written down and
//! evaluated by the arena at the weapon's own `WeaponReady`, at the passes
//! that refresh the lock's plot, at a lock, an arming, a landing and a startup
//! — each at its own arena millisecond and never once per pulse. These tests
//! drive the library directly, in the `tests/weapons.rs` (h) harness's shape
//! (copied, not shared: an integration test file is its own crate):
//!
//! 1. the MANDATORY cross-tempo proof: a seat's arming resolves at the end of
//!    the pulse that accepted it and the slot's second shot lands on the
//!    `WeaponReady` millisecond — mid-pulse at the 5,000 ms cadence — at
//!    5,000 / 1,000 / 500 ms and on the task's own wakes, one event list and
//!    one hash sequence;
//! 2. a `LOCK` and an arming typed in ONE non-zero-`dt` pulse land in that
//!    order (the arming is discrete), and a typed `FIRE` then an arming finds
//!    the gun cycling;
//! 3. the production pins, re-run where they live (see the test's doc);
//! 4. a pass that turns the plot to a flank fires the slot at the pass's
//!    millisecond; a wingman's share at the share's; a startup at the
//!    startup's;
//! 5. the floor at the landing: `Unlawful` with the floor named, the captured
//!    floor (not a constant), a `Contact` target refused with the fallback;
//! 6. (in `tests/arena_task.rs`, where the task lives) the floor captured at
//!    check-out survives a resume;
//! 7. an unclassified lock is idle, `BestHostile` never fires on a friendly;
//! 8. a standing `hold_fire` stops a slot and never a pilot;
//! 9. the seat window counts an arming, and a third weapon is `NoSlot`;
//! 10. slots die with the controller and with the turret, and survive a
//!     linkdead grace;
//! 11. (in-source, `conditions.rs`) the evaluator's cost;
//! 12. a snapshot carries the slots, a card, the floor and `held_said`.

use std::sync::Arc;

use mush_core::{DefinitionDigest, DefinitionId, DefinitionKey, DefinitionKind, ObjId, OwnerId};
use mush_sim::data::Pose;
use mush_tactics::{
    CellSpec, DefinitionRef, FacetId, FacetSpec, RulesValue, SectionId, SectionSpec, TacticalUnit,
    UnitSpec,
};
use smol_str::SmolStr;
use tacsim::arena::clock::Tempo;
use tacsim::arena::conditions::{
    ASPECTS_ANY, Arming, FireCondition, HeldWhy, STOCK_CONDITION_SLOTS, SlotTarget, aspect_mask,
};
use tacsim::arena::intents::{Control, ControlState, Intent, IntentKind, Refusal};
use tacsim::arena::ledger::LedgerEvent;
use tacsim::arena::perception::{
    Aspect, FrameNotice, RefusalDetail, SensorProfile, SignatureFactor,
};
use tacsim::arena::reflex::ReflexCounters;
use tacsim::arena::resolution::{BandProfile, SectionProfile, UnitProfile};
use tacsim::arena::rng::{draw_contact_id, stagger_ms};
use tacsim::arena::state::{
    Arena, ArenaDefinition, ArenaTerrain, Channel, Classification, ContactId, Controller,
    CustodyOut, CustodyUnit, EntryZone, Pilot, Power, SeenPose, ShadowContact, Team, TerrainCell,
    TrackedRules, WeaponSummary,
};
use tacsim::arena::step::StepOutput;
use tacsim::arena::weapons::WeaponBand;
use tacsim::arena::{Snapshot, SnapshotError};
use tacsim::config::{ArenaConfig, MatchConfig, SensorsConfig};
use tacsim::match_::cockpit;

// ---------------------------------------------------------------------------
// Fixtures (the `tests/weapons.rs` shapes)
// ---------------------------------------------------------------------------

/// The window: 16 by 16 cells of 1,000 cm.
const SIDE: u16 = 16;
/// The cell size, centimetres.
const CELL_CM: u16 = 1_000;
/// The match seed.
const SEED: u64 = 2_350;
/// The shooter, its wingman, and two targets.
const RED: ObjId = ObjId(4);
const RED2: ObjId = ObjId(5);
const BLUE: ObjId = ObjId(9);
const BLUE2: ObjId = ObjId(12);
/// The pilot `arena_of` seats on the first unit it is handed.
const RED_PILOT: ObjId = ObjId(1_000);
/// The odds every shot of these fixtures resolves at unless a test says
/// otherwise: `p_min = p_max`, so the plot answers exactly this.
const ODDS: u32 = 500;
/// The arena's sensor cadence (`[tacsim.sensors].pass_ms`'s default).
const PASS_MS: i64 = 1_000;

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

/// The smallest M32 unit the arena carries: one hull section.
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
                    max: 10_000,
                    init: 10_000,
                }],
            }],
            slots: Vec::new(),
            cells: vec![CellSpec {
                id: mush_tactics::CellId::new("internals").expect("a cell"),
                min: 0,
                max: 10_000,
                init: 10_000,
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

/// The cell centre of `(x, y)`, facing `heading_mdeg`.
fn facing(x: u16, y: u16, heading_mdeg: u32) -> Pose {
    let mut pose = Pose::at(
        i64::from(x) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
        i64::from(y) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
        0,
        rules(),
    );
    pose.heading_mdeg = heading_mdeg;
    pose
}

fn definition() -> ArenaDefinition {
    ArenaDefinition {
        id: SmolStr::new("tacsim.arena.test"),
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
        waypoints: tacsim::arena::state::Waypoints::default(),
        modes: Vec::new(),
    }
}

/// A five-cell sight that shares `share_cells` cells.
fn sight(share_cells: u16) -> SensorProfile {
    use mush_atlas::TerrainClass as C;
    SensorProfile {
        component: SmolStr::new("visual_sensor"),
        max_range_cells: 5,
        max_attenuation: 40,
        classify_after_ms: 3_000,
        sees_wrecks: false,
        share_range_cells: share_cells,
        signature: vec![SignatureFactor {
            class: C::Ground,
            moving_permille: 1_000,
            stopped_permille: 1_000,
        }],
    }
}

/// A cannon in the turret: a four-second cycle, forty rounds.
fn cannon() -> WeaponSummary {
    WeaponSummary {
        id: SmolStr::new("cannon"),
        max_range_cm: 250_000,
        cycle_ms: 4_000,
        ammunition: SmolStr::new("tacsim.ammunition.he_105"),
        burst_rounds: 1,
        muzzle_speed_cms: 145_000,
        bands: vec![
            WeaponBand {
                max_range_cm: 150_000,
                dispersion_mrad: 3,
            },
            WeaponBand {
                max_range_cm: 250_000,
                dispersion_mrad: 6,
            },
        ],
        count_per_load: 40,
        damage: 1,
        mount_section: SmolStr::new("turret"),
    }
}

/// A machine gun on the hull: a one-second cycle, two hundred rounds.
fn machine_gun() -> WeaponSummary {
    WeaponSummary {
        id: SmolStr::new("machine_gun"),
        cycle_ms: 1_000,
        count_per_load: 200,
        mount_section: SmolStr::new("hull"),
        ..cannon()
    }
}

/// **A target every band of which presents a silhouette**, so a shot at any
/// aspect resolves on the curve and the clamp `[p_min, p_max]` decides it.
fn profile() -> UnitProfile {
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

/// One tank.
fn unit(
    id: ObjId,
    team: Team,
    pose: Pose,
    weapons: Vec<WeaponSummary>,
    sensor: Option<SensorProfile>,
) -> CustodyUnit {
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
        sensor,
        weapons,
        profile: profile(),
        gunnery: 0,
        controller: Controller::Pilot,
        slot: 0,
        unit: tactical_unit(id),
    }
}

/// Power in half a second.
fn match_cfg() -> MatchConfig {
    MatchConfig {
        startup_ms: 500,
        ..MatchConfig::default()
    }
}

/// An arena with a seated pilot per unit (`#1000`, `#1001`, … in the order
/// given) and the odds pinned at [`ODDS`].
fn arena_of(units: Vec<CustodyUnit>) -> Arena {
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
    let mut arena = Arena::new(
        CustodyOut {
            definition: definition(),
            units,
            pilots,
            resolution: tacsim::match_::stock_resolution(),
            seed: SEED,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &ArenaConfig::default(),
        &match_cfg(),
        &SensorsConfig::default(),
    )
    .expect("the test arena");
    arena.resolution.p_min = ODDS;
    arena.resolution.p_max = ODDS;
    arena
}

/// **The duel**: red armed with a cannon at (5, 8), blue unarmed four cells
/// east facing `blue_heading` (north shows red its left flank, west its front).
fn duel(blue_heading: u32) -> Arena {
    arena_of(vec![
        unit(
            RED,
            Team::Red,
            facing(5, 8, 90_000),
            vec![cannon()],
            Some(sight(0)),
        ),
        unit(
            BLUE,
            Team::Blue,
            facing(9, 8, blue_heading),
            Vec::new(),
            Some(sight(0)),
        ),
    ])
}

fn intent(unit: ObjId, kind: IntentKind, seq: u64) -> Intent {
    Intent {
        unit,
        channel: kind.channel_required(),
        epoch: 1,
        kind,
        seq,
    }
}

fn contact_of(viewer: ObjId, target: ObjId) -> ContactId {
    draw_contact_id(SEED, viewer, target)
}

/// A slot's terms on the lock.
fn cond(weapon: &str, min_permille: u16, aspects: u8) -> FireCondition {
    FireCondition {
        weapon: SmolStr::new(weapon),
        target: SlotTarget::Lock,
        min_permille,
        aspects,
        min_rounds_left: 0,
        min_range_m: 0,
        max_range_m: u32::MAX,
    }
}

/// An arming of `cond`.
fn arm(cond: FireCondition) -> IntentKind {
    IntentKind::Condition(Arming::Arm(cond))
}

// ---------------------------------------------------------------------------
// The driver
// ---------------------------------------------------------------------------

/// How a run is driven: a fixed wall cadence, or the task's own wakes (each
/// pulse ends at `StepOutput::next_due_ms`, or at the next beat when that is
/// sooner — the world's beat is what wakes the task).
#[derive(Clone, Copy, Debug)]
enum Drive {
    Every(i64),
    Task,
}

/// The four drives every cross-tempo proof here runs.
const DRIVES: [Drive; 4] = [
    Drive::Every(5_000),
    Drive::Every(1_000),
    Drive::Every(500),
    Drive::Task,
];

/// One scripted beat at an arena millisecond on every drive's lattice: its
/// controls and its intents, delivered in a pulse of no wall time.
struct Beat {
    t_ms: i64,
    controls: Vec<Control>,
    intents: Vec<Intent>,
}

fn beat(t_ms: i64, intents: Vec<Intent>) -> Beat {
    Beat {
        t_ms,
        controls: Vec::new(),
        intents,
    }
}

/// What a run produced.
#[derive(Debug)]
struct Run {
    /// Every line: `(t_ms, kind and fields as JSON, hash)`.
    events: Vec<(i64, String, u64)>,
    /// Every substep's hash.
    hashes: Vec<u64>,
    /// The counters, summed.
    counters: ReflexCounters,
    /// Every notice the red pilot was sent.
    notices: Vec<FrameNotice>,
    /// How many pulses it took.
    pulses: u32,
}

impl Run {
    /// The `fired` lines, parsed.
    fn fired(&self) -> Vec<(i64, serde_json::Value)> {
        self.events
            .iter()
            .filter_map(|(t_ms, json, _)| {
                let value: serde_json::Value = serde_json::from_str(json).ok()?;
                (value.get("kind")? == "fired").then_some((*t_ms, value))
            })
            .collect()
    }
}

fn absorb(out: &StepOutput, bytes: &mut Vec<u8>, notices: &mut Vec<FrameNotice>) {
    for chunk in &out.ledger_chunks {
        bytes.extend_from_slice(&chunk.bytes);
    }
    for (pilot, frame) in &out.frames {
        if *pilot == RED_PILOT {
            notices.extend(frame.notices.iter().cloned());
        }
    }
}

fn lines(arena: &mut Arena, bytes: &mut Vec<u8>) -> Vec<LedgerEvent> {
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

/// **Drive `arena` through `script` under `drive` to `until_ms`**, the tempo
/// pinned `Quiet` (a tempo line is a per-pulse artefact of the cadence), each
/// beat in a zero-`dt` pulse of its own at its own millisecond — the
/// `tests/weapons.rs` (h) driver's rule, so a discrete order resolves at the
/// millisecond the script names at every drive.
fn replay(arena: &mut Arena, drive: Drive, until_ms: i64, script: &[Beat]) -> Run {
    let mut bytes: Vec<u8> = Vec::new();
    let mut notices: Vec<FrameNotice> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut counters = ReflexCounters::default();
    let mut wall = 0_i64;
    let mut next = 0_usize;
    let mut pulses = 0_u32;
    let mut next_due = 0_i64;
    // The opening pulse anchors arena time to wall `0` (the clock's first
    // step); a script with no beat at `0` gets an empty one, so a wall reading
    // is an arena millisecond on every drive.
    if script.first().is_none_or(|beat| beat.t_ms > 0) {
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes, &mut notices);
        counters.absorb(&out.reflex);
        hashes.extend(arena.substep_hashes.iter().copied());
        pulses += 1;
        next_due = out.next_due_ms;
    }
    loop {
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        while let Some(beat) = script.get(next) {
            if beat.t_ms > arena.now_ms {
                break;
            }
            next += 1;
            let out = arena.step(wall, &beat.controls, &beat.intents);
            absorb(&out, &mut bytes, &mut notices);
            counters.absorb(&out.reflex);
            hashes.extend(arena.substep_hashes.iter().copied());
            pulses += 1;
            next_due = out.next_due_ms;
        }
        if arena.now_ms >= until_ms {
            break;
        }
        let beat_at = script.get(next).map_or(i64::MAX, |beat| beat.t_ms);
        wall = match drive {
            Drive::Every(cadence_ms) => wall + cadence_ms,
            Drive::Task => next_due.max(wall + 1).min(beat_at).min(until_ms),
        };
        let out = arena.step(wall, &[], &[]);
        absorb(&out, &mut bytes, &mut notices);
        counters.absorb(&out.reflex);
        hashes.extend(arena.substep_hashes.iter().copied());
        pulses += 1;
        next_due = out.next_due_ms;
        if out.ended.is_some() {
            break;
        }
    }
    let events = lines(arena, &mut bytes)
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
        counters,
        notices,
        pulses,
    }
}

/// Run `make()` under every drive and assert one event list and one hash
/// sequence; answers the first run.
fn at_every_drive(make: impl Fn() -> Arena, until_ms: i64, script: impl Fn() -> Vec<Beat>) -> Run {
    let mut runs: Vec<(Drive, Run)> = Vec::new();
    for drive in DRIVES {
        let mut arena = make();
        let run = replay(&mut arena, drive, until_ms, &script());
        runs.push((drive, run));
    }
    let (_, first) = runs.remove(0);
    for (drive, run) in &runs {
        if let Some(at) = (0..run.events.len().max(first.events.len()))
            .find(|at| run.events.get(*at) != first.events.get(*at))
        {
            panic!(
                "the event list differs at {drive:?}, line {at}: {:?} against {:?}",
                run.events.get(at),
                first.events.get(at)
            );
        }
        assert_eq!(
            run.hashes, first.hashes,
            "the hash sequence differs at {drive:?}"
        );
        assert_eq!(
            run.counters.cond_fires, first.counters.cond_fires,
            "the slot shots differ at {drive:?}"
        );
        assert_ne!(run.pulses, first.pulses, "{drive:?} ran the same pulses");
    }
    first
}

/// The `fired` lines `by: cond`, as `(t_ms, slot, target)`.
fn cond_shots(run: &Run) -> Vec<(i64, u64, u64)> {
    run.fired()
        .into_iter()
        .filter(|(_, line)| line.get("by").and_then(serde_json::Value::as_str) == Some("cond"))
        .map(|(t_ms, line)| {
            (
                t_ms,
                line.get("slot")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                line.get("target")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 1. The mandatory cross-tempo proof
// ---------------------------------------------------------------------------

/// The duel's script: `STARTUP` at 0; at 5,000 — the contact classified at
/// about 4,000 — `LOCK` and then the arming, typed in that order.
fn armed_at_5000(cond: FireCondition) -> Vec<Beat> {
    let contact = contact_of(RED, BLUE);
    vec![
        beat(0, vec![intent(RED, IntentKind::Startup, 1)]),
        beat(
            5_000,
            vec![
                intent(RED, IntentKind::Lock { contact }, 2),
                intent(RED, arm(cond), 3),
            ],
        ),
    ]
}

/// **The mandatory cross-tempo proof** (17b cut M1b.1; 16b §11; doc 17 §5,
/// D-UM-16): a seat's arming resolves at the END of the pulse that accepted it
/// — 5,000, the replay's zero-`dt` beat — and its `Arming` evaluation fires at
/// once (`by: cond`, `slot: 1`); the slot's second shot lands at exactly
/// `5,000 + cycle_ms` = 9,000, the `WeaponReady` millisecond, which at the
/// 5,000 ms cadence is MID-PULSE (the proof), and not at the 10,000 boundary
/// after it. One event list and one hash sequence at 5,000 / 1,000 / 500 ms and
/// on the task's own wakes; two slot shots; the rounds conserved.
#[test]
fn m1b1_a_conditioned_shot_resolves_on_the_weapon_ready_millisecond_at_three_cadences() {
    let run = at_every_drive(
        || duel(0),
        10_000,
        || armed_at_5000(cond("cannon", 100, ASPECTS_ANY)),
    );
    let cycle = i64::from(cannon().cycle_ms);
    assert_eq!(
        cond_shots(&run),
        vec![
            (5_000, 1, u64::from(BLUE.0)),
            (5_000 + cycle, 1, u64::from(BLUE.0))
        ],
        "the arming's own shot at 5,000 and the WeaponReady's at 9,000"
    );
    assert_eq!(run.counters.cond_fires, 2);
    assert!(
        run.notices
            .iter()
            .all(|notice| !matches!(notice, FrameNotice::Refused { .. })),
        "nothing the pilot typed was refused: {:?}",
        run.notices
    );
    // Every shot of the run was the slot's.
    assert_eq!(run.fired().len(), 2);
    let rounds: u64 = run
        .fired()
        .iter()
        .filter_map(|(_, line)| line.get("rounds").and_then(serde_json::Value::as_u64))
        .sum();
    let mut arena = duel(0);
    let _ = replay(
        &mut arena,
        Drive::Every(500),
        10_000,
        &armed_at_5000(cond("cannon", 100, ASPECTS_ANY)),
    );
    assert_eq!(
        u64::from(
            arena
                .unit(RED)
                .map_or(0, |state| state.weapon_ammo("cannon"))
        ) + rounds,
        40,
        "rounds fired plus rounds left is the load"
    );
    assert_eq!(
        arena.conditions.slot_of(RED, "cannon"),
        Some(1),
        "the slot stands"
    );
}

// ---------------------------------------------------------------------------
// 2. A lock and an arming in one pulse
// ---------------------------------------------------------------------------

/// Red, blue and a second blue at (9, 10), all classified and red locked on
/// blue, online, at 5,000; answers the arena and the wall it stands at.
fn two_targets_locked_on_the_first() -> (Arena, i64) {
    let mut arena = arena_of(vec![
        unit(
            RED,
            Team::Red,
            facing(5, 8, 90_000),
            vec![cannon()],
            Some(sight(0)),
        ),
        unit(BLUE, Team::Blue, facing(9, 8, 0), Vec::new(), None),
        unit(BLUE2, Team::Blue, facing(9, 10, 0), Vec::new(), None),
    ]);
    let mut wall = 0_i64;
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let _ = arena.step(wall, &[], &[intent(RED, IntentKind::Startup, 1)]);
    while arena.now_ms < 5_000 {
        wall += 500;
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let _ = arena.step(wall, &[], &[]);
    }
    for target in [BLUE, BLUE2] {
        assert!(
            arena
                .contacts
                .get(RED, contact_of(RED, target))
                .is_some_and(|held| matches!(held.class, Classification::Classified { .. })),
            "red holds #{} classified",
            target.0
        );
    }
    let _ = arena.step(
        wall,
        &[],
        &[intent(
            RED,
            IntentKind::Lock {
                contact: contact_of(RED, BLUE),
            },
            2,
        )],
    );
    assert_eq!(
        arena.unit(RED).and_then(|state| state.lock),
        Some(contact_of(RED, BLUE))
    );
    (arena, wall)
}

/// The `fired` lines of `arena`'s ledger so far, as `(t_ms, target, by)`.
fn fired_lines(arena: &mut Arena) -> Vec<(i64, u64, Option<String>)> {
    let mut bytes = Vec::new();
    lines(arena, &mut bytes)
        .into_iter()
        .filter_map(|event| {
            let value = serde_json::to_value(&event.kind).ok()?;
            (value.get("kind")? == "fired").then(|| {
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
                )
            })
        })
        .collect()
}

/// **A `LOCK` and an arming typed in ONE non-zero-`dt` pulse land in that
/// order** (17b cut M1b.1; 17b's adversarial F7): red locked on BA1 types
/// `LOCK BA2` then `FIRE cannon WHEN …` inside one 5,000 ms pulse at the
/// `Quiet` tempo. Both are discrete, resolved at the pulse's end in arrival
/// order, so the arming's first evaluation reads the NEW lock: the slot's
/// first shot is at BA2 and no shot ever names BA1. And the reverse — a typed
/// `FIRE` then the arming in one pulse: the typed shot fires first, and the
/// slot's evaluation at the arming finds the gun cycling (no second shot at
/// that millisecond; the slot's own fires at the `WeaponReady` after it).
#[test]
fn m1b1_a_lock_and_an_arming_typed_in_one_pulse_land_in_that_order() {
    let (mut arena, wall) = two_targets_locked_on_the_first();
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let _ = arena.step(
        wall + 5_000,
        &[],
        &[
            intent(
                RED,
                IntentKind::Lock {
                    contact: contact_of(RED, BLUE2),
                },
                3,
            ),
            intent(RED, arm(cond("cannon", 100, ASPECTS_ANY)), 4),
        ],
    );
    let at = arena.now_ms;
    assert_eq!(at, 10_000, "one 5,000 ms pulse");
    let mut wall_now = wall + 5_000;
    while arena.now_ms < 20_000 {
        wall_now += 500;
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let _ = arena.step(wall_now, &[], &[]);
    }
    let fired = fired_lines(&mut arena);
    assert_eq!(
        fired.first(),
        Some(&(at, u64::from(BLUE2.0), Some("cond".to_owned()))),
        "the slot's first shot is at the new lock, at the pulse's end"
    );
    assert!(
        fired
            .iter()
            .all(|(_, target, _)| *target == u64::from(BLUE2.0)),
        "no shot ever names the old lock: {fired:?}"
    );

    // The reverse: a typed FIRE then the arming in one pulse.
    let (mut arena, wall) = two_targets_locked_on_the_first();
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let _ = arena.step(
        wall + 5_000,
        &[],
        &[
            intent(RED, IntentKind::fire("cannon"), 3),
            intent(RED, arm(cond("cannon", 100, ASPECTS_ANY)), 4),
        ],
    );
    let at = arena.now_ms;
    let mut wall_now = wall + 5_000;
    while arena.now_ms < 16_000 {
        wall_now += 500;
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let _ = arena.step(wall_now, &[], &[]);
    }
    let fired = fired_lines(&mut arena);
    assert_eq!(
        fired,
        vec![
            (at, u64::from(BLUE.0), None),
            (
                at + i64::from(cannon().cycle_ms),
                u64::from(BLUE.0),
                Some("cond".to_owned())
            ),
        ],
        "the typed shot first; the slot found the gun cycling and fired at its WeaponReady"
    );
}

// ---------------------------------------------------------------------------
// 3. The production pins
// ---------------------------------------------------------------------------

/// **The production hash and line hashes do not move** (17b cut M1b.1; the
/// lead's binding: a unit with no slot folds nothing new). The pins live where
/// they were recorded and are re-run by the gate, never re-recorded here: M1a.1's
/// two substep pins (`tests/reflex.rs`' `m1a1_the_moved_motor_folds_the_same_bytes`,
/// `tests/motor.rs`' `m1a1_the_shipped_duel_hashes_as_before`),
/// `tests/minds.rs`' `m1a5_the_production_hash_does_not_move`, and M1b.0's
/// three `m1b0_the_production_line_hashes_do_not_move` (`tests/weapons.rs`,
/// `tests/motor.rs`, `tests/reflex.rs`) — a trigger that TAKES the lazy action
/// hash early shows in their next action line.
///
/// What this test adds, end to end through `Arena::step`: the seat duel,
/// stepped every 500 ms through a startup, a lock, two typed shots, their
/// `WeaponReady`s and red's passes (every trigger arm but a share's and a
/// landing's), with no slot armed — after EVERY pulse the slot table and
/// `held_said` hold no row (so item 3d and item 3e fold no byte at any pulse),
/// and the same duel on an arena whose captured floor is 50 hashes equal to the
/// floor-100 one after every pulse and journals the same lines (the floor is
/// never folded); no slot shot and no held slot is counted; the seat's two
/// `fired` lines carry no `by` or `slot`. What it cannot see from outside the
/// library is a trigger that only SETS `ActionHash::Pending` for a hull with no
/// slot (`action_hash` is crate-private): `conditions.rs`' in-source
/// `m1b1_a_hull_with_no_slot_takes_no_hash_at_any_trigger` pins the
/// evaluator's own early return for each `ConditionTrigger`, and every call
/// site calls the evaluator with no hash opened of its own.
#[test]
fn m1b1_the_production_hash_and_line_hashes_do_not_move() {
    let contact = contact_of(RED, BLUE);
    let beats = [
        (0_i64, vec![intent(RED, IntentKind::Startup, 1)]),
        (
            5_000,
            vec![
                intent(RED, IntentKind::Lock { contact }, 2),
                intent(RED, IntentKind::fire("cannon"), 3),
            ],
        ),
        (10_000, vec![intent(RED, IntentKind::fire("cannon"), 4)]),
    ];
    let mut arenas = [duel(0), duel(0)];
    arenas[1].odds_floor_permille = 50;
    let mut counters = ReflexCounters::default();
    let mut bytes: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
    let mut wall = 0_i64;
    let mut pulses = 0_u32;
    while wall <= 20_000 {
        let delivered: &[Intent] = beats
            .iter()
            .find(|(at, _)| *at == wall)
            .map_or(&[], |(_, intents)| intents.as_slice());
        for (at, arena) in arenas.iter_mut().enumerate() {
            arena.clock.pin_tempo_for_tests(Tempo::Quiet);
            let out = arena.step(wall, &[], delivered);
            for chunk in &out.ledger_chunks {
                bytes[at].extend_from_slice(&chunk.bytes);
            }
            if at == 0 {
                counters.absorb(&out.reflex);
            }
            assert!(
                arena.conditions.rows().next().is_none(),
                "a slot row at wall {wall}"
            );
            assert!(arena.held_said.is_empty(), "a held row at wall {wall}");
        }
        let [hundred, fifty] = &arenas;
        assert_eq!(
            tacsim::arena::state_hash(fifty),
            tacsim::arena::state_hash(hundred),
            "the floor folded at wall {wall}"
        );
        assert_eq!(fifty.substep_hashes, hundred.substep_hashes);
        pulses += 1;
        wall += 500;
    }
    assert_eq!(pulses, 41);
    assert_eq!(counters.cond_fires, 0);
    assert_eq!(counters.cond_held, 0);
    let [hundred, fifty] = &mut arenas;
    let [bytes_hundred, bytes_fifty] = &mut bytes;
    let lines_hundred = lines(hundred, bytes_hundred);
    assert_eq!(lines_hundred, lines(fifty, bytes_fifty), "the same lines");
    let fired: Vec<serde_json::Value> = lines_hundred
        .iter()
        .filter_map(|event| serde_json::to_value(&event.kind).ok())
        .filter(|value| value.get("kind").is_some_and(|kind| kind == "fired"))
        .collect();
    assert_eq!(fired.len(), 2, "the seat's two shots: {fired:?}");
    assert!(
        fired
            .iter()
            .all(|line| line.get("by").is_none() && line.get("slot").is_none()),
        "a seat's `fired` line carries no attribution"
    );
}

// ---------------------------------------------------------------------------
// 4. The passes, the share, the startup
// ---------------------------------------------------------------------------

/// Whether `t_ms` is one of `unit`'s sensor-pass milliseconds.
fn is_pass_of(unit: ObjId, t_ms: i64) -> bool {
    (t_ms - stagger_ms(SEED, unit, PASS_MS)).rem_euclid(PASS_MS) == 0
}

/// **A pass that turns the plot to a flank fires the slot at the pass's
/// millisecond** (17b cut M1b.1; doc 17 §5, 17b §1(d)), at three cadences and
/// on the task's wakes:
///
/// * red's slot fires only at `left|right`; blue faces red (its front) and
///   nothing fires through two cycles; at 15,000 blue turns north and the
///   first slot shot is at one of RED's OWN `SensorPass` milliseconds after
///   the turn — the pass that refreshed the plot;
/// * a sightless red fed by a wingman's share fires at the wingman's pass —
///   the `Share`'s millisecond — never at one of its own;
/// * a slot armed while red is still `Starting` fires at the
///   `StartupComplete` millisecond, 500 ms after the startup.
#[test]
fn m1b1_a_slot_fires_when_a_pass_turns_the_plot_to_a_flank() {
    let flank = aspect_mask(&[Aspect::Left, Aspect::Right]);
    let turn = |unit_seq: u64| {
        beat(
            15_000,
            vec![intent(BLUE, IntentKind::Heading { mdeg: 0 }, unit_seq)],
        )
    };
    // (a) Red's own pass.
    let run = at_every_drive(
        || duel(270_000),
        25_000,
        || {
            let mut script = armed_at_5000(cond("cannon", 100, flank));
            if let Some(first) = script.first_mut() {
                first.intents.push(intent(BLUE, IntentKind::Startup, 9));
            }
            script.push(turn(10));
            script
        },
    );
    let shots = cond_shots(&run);
    let Some(&(first, _, _)) = shots.first() else {
        panic!("the slot never fired: {:?}", run.counters);
    };
    assert!(first > 15_000, "nothing fired while blue showed its front");
    assert!(
        is_pass_of(RED, first),
        "the slot fired at {first}, which is not one of red's passes"
    );
    assert!(
        run.counters.cond_held > 0,
        "it held for the aspect before the turn"
    );

    // (b) A sightless red, fed by a wingman's share.
    let wing = || {
        arena_of(vec![
            unit(RED, Team::Red, facing(5, 8, 90_000), vec![cannon()], None),
            unit(BLUE, Team::Blue, facing(9, 8, 270_000), Vec::new(), None),
            unit(
                RED2,
                Team::Red,
                facing(5, 7, 90_000),
                Vec::new(),
                Some(sight(5)),
            ),
        ])
    };
    let run = at_every_drive(wing, 25_000, || {
        let mut script = armed_at_5000(cond("cannon", 100, flank));
        if let Some(first) = script.first_mut() {
            first.intents.push(intent(BLUE, IntentKind::Startup, 9));
        }
        script.push(turn(10));
        script
    });
    let shots = cond_shots(&run);
    let Some(&(first, _, _)) = shots.first() else {
        panic!("the shared slot never fired: {:?}", run.counters);
    };
    assert!(first > 15_000);
    assert!(
        is_pass_of(RED2, first),
        "the slot fired at {first}, which is not the wingman's pass (its share)"
    );
    assert!(
        !is_pass_of(RED, first),
        "the two staggers coincide; pick other ids"
    );

    // (c) A slot armed while starting fires when power comes up.
    let contact = contact_of(RED, BLUE);
    let run = at_every_drive(
        || duel(0),
        10_000,
        || {
            vec![beat(
                5_000,
                vec![
                    intent(RED, IntentKind::Startup, 1),
                    intent(RED, IntentKind::Lock { contact }, 2),
                    intent(RED, arm(cond("cannon", 100, ASPECTS_ANY)), 3),
                ],
            )]
        },
    );
    assert_eq!(
        cond_shots(&run).first().map(|(t, _, _)| *t),
        Some(5_000 + i64::from(match_cfg().startup_ms)),
        "the slot fired at the StartupComplete millisecond"
    );
}

// ---------------------------------------------------------------------------
// 5. The floor at the landing
// ---------------------------------------------------------------------------

/// Red online and locked on blue at 5,000, answering the arena and its wall.
fn locked_duel(floor: u32) -> (Arena, i64) {
    let mut arena = duel(0);
    arena.odds_floor_permille = floor;
    let run = replay(
        &mut arena,
        Drive::Every(500),
        5_000,
        &[
            beat(0, vec![intent(RED, IntentKind::Startup, 1)]),
            beat(
                5_000,
                vec![intent(
                    RED,
                    IntentKind::Lock {
                        contact: contact_of(RED, BLUE),
                    },
                    2,
                )],
            ),
        ],
    );
    assert_eq!(run.fired().len(), 0);
    // The replay delivered the 5,000 beat before it stopped.
    let wall = arena.now_ms;
    assert_eq!(wall, 5_000);
    assert_eq!(
        arena.unit(RED).and_then(|state| state.lock),
        Some(contact_of(RED, BLUE))
    );
    (arena, wall)
}

/// The red pilot's `refused` notices on one zero-`dt` pulse delivering
/// `kinds`, each with the frame it rode.
fn refused_now(
    arena: &mut Arena,
    wall: i64,
    kinds: Vec<IntentKind>,
) -> Vec<(FrameNotice, tacsim::arena::step::Frame)> {
    let intents: Vec<Intent> = kinds
        .into_iter()
        .enumerate()
        .map(|(n, kind)| intent(RED, kind, 100 + u64::try_from(n).unwrap_or(0)))
        .collect();
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let out = arena.step(wall, &[], &intents);
    out.frames
        .iter()
        .filter(|(pilot, _)| *pilot == RED_PILOT)
        .flat_map(|(_, frame)| {
            frame
                .notices
                .iter()
                .filter(|notice| matches!(notice, FrameNotice::Refused { .. }))
                .map(|notice| (notice.clone(), frame.clone()))
        })
        .collect()
}

/// The sentence a notice reads as, on the frame it rode.
fn sentence(refused: &(FrameNotice, tacsim::arena::step::Frame)) -> String {
    cockpit::notice_line(&refused.1, &refused.0, &[])
}

/// **The floor at the landing of a seat's arming** (17b cut M1b.1; doc 17 §5,
/// D-UM-17 as amended; refuse-not-clamp): on a floor-100 arena an arming at
/// 50 per mille is refused `unlawful` from phase 4 with the weapon and the
/// floor named — `FIRE cannon refused. The Compact floor is 10%; a condition
/// may not ask for less.` — and arms nothing; at 100 it lands as S1. An arena
/// whose captured floor is 50 takes 50 and refuses 40 (the arena's floor, not
/// a constant). A `Contact` target is refused `unlawful` with the fallback
/// sentence for a held id and an unheld one alike. A slot armed AT the floor
/// fires at the floor's odds and holds under them.
#[test]
fn m1b1_an_arming_under_the_floor_is_refused_unlawful_at_the_landing_and_a_contact_target_too() {
    let (mut arena, wall) = locked_duel(100);
    // Hold the slot back from firing: no aspect at all would be unlawful? No —
    // the aspect term is not a law term, so a front-only slot on a flank holds.
    let front = aspect_mask(&[Aspect::Front]);
    let refused = refused_now(&mut arena, wall, vec![arm(cond("cannon", 50, front))]);
    let [notice] = refused.as_slice() else {
        panic!("one refusal: {refused:?}");
    };
    let FrameNotice::Refused {
        order,
        refusal,
        weapon,
        detail,
    } = &notice.0
    else {
        panic!("a refusal");
    };
    assert_eq!(
        (order.as_str(), *refusal, weapon.as_deref(), *detail),
        (
            "fire",
            Refusal::Unlawful,
            Some("cannon"),
            RefusalDetail::Floor { permille: 100 }
        )
    );
    assert_eq!(
        sentence(notice),
        "FIRE cannon refused. The Compact floor is 10%; a condition may not ask for less."
    );
    assert!(arena.conditions.is_empty_for(RED), "no slot armed");
    let refused = refused_now(&mut arena, wall, vec![arm(cond("cannon", 100, front))]);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(arena.conditions.slot_of(RED, "cannon"), Some(1));

    // The captured floor, not a constant.
    let (mut arena, wall) = locked_duel(50);
    assert!(refused_now(&mut arena, wall, vec![arm(cond("cannon", 50, front))]).is_empty());
    assert_eq!(arena.conditions.slot_of(RED, "cannon"), Some(1));
    let refused = refused_now(&mut arena, wall, vec![arm(cond("cannon", 40, front))]);
    assert!(
        matches!(
            refused.as_slice(),
            [(
                FrameNotice::Refused {
                    refusal: Refusal::Unlawful,
                    detail: RefusalDetail::Floor { permille: 50 },
                    ..
                },
                _
            )]
        ),
        "{refused:?}"
    );
    assert_eq!(
        sentence(&refused[0]),
        "FIRE cannon refused. The Compact floor is 5%; a condition may not ask for less."
    );

    // A `Contact` target: one word and one sentence, held id or not.
    let (mut arena, wall) = locked_duel(100);
    for id in [contact_of(RED, BLUE), ContactId(0x0bad_cafe)] {
        let refused = refused_now(
            &mut arena,
            wall,
            vec![arm(FireCondition {
                target: SlotTarget::Contact { id },
                ..cond("cannon", 100, ASPECTS_ANY)
            })],
        );
        let [notice] = refused.as_slice() else {
            panic!("one refusal: {refused:?}");
        };
        assert!(matches!(
            &notice.0,
            FrameNotice::Refused {
                refusal: Refusal::Unlawful,
                detail: RefusalDetail::None,
                ..
            }
        ));
        assert_eq!(
            sentence(notice),
            "FIRE cannon refused. The Compact floor forbids that condition."
        );
    }
    assert!(arena.conditions.is_empty_for(RED));

    // At the floor's odds: fires at 100, holds at 90.
    for (odds, fires) in [(100_u32, true), (90, false)] {
        let (mut arena, wall) = locked_duel(100);
        arena.resolution.p_min = odds;
        arena.resolution.p_max = odds;
        let _ = refused_now(
            &mut arena,
            wall,
            vec![arm(cond("cannon", 100, ASPECTS_ANY))],
        );
        let fired = fired_lines(&mut arena);
        assert_eq!(!fired.is_empty(), fires, "at {odds} per mille: {fired:?}");
        let warrant = arena
            .conditions
            .slots(RED)
            .first()
            .cloned()
            .flatten()
            .map(|slot| arena.warrant(RED, &slot, arena.now_ms));
        if !fires {
            assert!(
                matches!(
                    warrant,
                    Some(Err(tacsim::arena::conditions::Held::Why(HeldWhy::Odds {
                        permille: 90,
                        floor: 100
                    })))
                ),
                "{warrant:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 7. The target rules
// ---------------------------------------------------------------------------

/// **A slot is idle on an unclassified lock and fires only on its lock** (17b
/// cut M1b.1; D-UM-17 extended to the default target): a `Lock` slot armed on
/// an `X` row never fires through the classify window and fires at the first
/// evaluation after the row is told apart — red's own pass that classified it;
/// a `BestHostile` slot whose lock is a friendly never fires, and fires once
/// the lock is a classified hostile; the warrant answers the first failing
/// term in order (rounds, the ROUNDED range, aspect, odds) and never looks at
/// readiness.
#[test]
fn m1b1_a_slot_is_idle_on_an_unclassified_lock_and_fires_only_on_its_lock() {
    let mut arena = duel(0);
    let mut wall = 0_i64;
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let _ = arena.step(wall, &[], &[intent(RED, IntentKind::Startup, 1)]);
    let contact = contact_of(RED, BLUE);
    while arena.contacts.get(RED, contact).is_none() {
        wall += 250;
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let _ = arena.step(wall, &[], &[]);
    }
    assert_eq!(
        arena.contacts.get(RED, contact).map(|held| held.class),
        Some(Classification::Unknown)
    );
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let _ = arena.step(
        wall,
        &[],
        &[
            intent(RED, IntentKind::Lock { contact }, 2),
            intent(RED, arm(cond("cannon", 100, ASPECTS_ANY)), 3),
        ],
    );
    assert_eq!(arena.unit(RED).and_then(|state| state.lock), Some(contact));
    assert!(fired_lines(&mut arena).is_empty(), "idle on an X row");
    while arena
        .contacts
        .get(RED, contact)
        .is_some_and(|held| held.class == Classification::Unknown)
    {
        wall += 250;
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let _ = arena.step(wall, &[], &[]);
        assert!(
            arena
                .contacts
                .get(RED, contact)
                .is_none_or(|held| held.class != Classification::Unknown)
                || fired_lines(&mut arena).is_empty(),
            "fired on an unclassified lock"
        );
    }
    let classified_at = arena
        .contacts
        .get(RED, contact)
        .map(|held| held.last_seen_ms)
        .unwrap_or_default();
    let fired = fired_lines(&mut arena);
    assert_eq!(
        fired.first().map(|(t, _, by)| (*t, by.clone())),
        Some((classified_at, Some("cond".to_owned()))),
        "fired at the pass that told it apart"
    );
    assert!(is_pass_of(RED, classified_at));

    // BestHostile: a friendly lock never fires; a hostile one does.
    let mut arena = arena_of(vec![
        unit(
            RED,
            Team::Red,
            facing(5, 8, 90_000),
            vec![cannon()],
            Some(sight(0)),
        ),
        unit(BLUE, Team::Blue, facing(9, 8, 0), Vec::new(), None),
        unit(RED2, Team::Red, facing(5, 10, 0), Vec::new(), None),
    ]);
    for (target, team) in [(BLUE, Team::Blue), (RED2, Team::Red)] {
        let plot = arena
            .unit(target)
            .map(SeenPose::of)
            .expect("a fielded unit");
        arena.contacts.insert(
            RED,
            contact_of(RED, target),
            ShadowContact::sighted(target, Classification::Classified { team }, 0, plot),
        );
    }
    if let Some(state) = arena.unit_mut(RED) {
        state.power = Power::Online;
    }
    let best = FireCondition {
        target: SlotTarget::BestHostile,
        ..cond("cannon", 100, ASPECTS_ANY)
    };
    assert_eq!(
        arena.resolve_lock(RED, contact_of(RED, RED2), 0),
        Ok(()),
        "a lock on the friendly"
    );
    assert_eq!(
        arena.resolve_arming(RED, &Arming::Arm(best.clone()), 0),
        Ok(Some(1))
    );
    assert_eq!(
        arena.warrant(RED, &best, 0),
        Err(tacsim::arena::conditions::Held::Idle),
        "a friendly lock is no best hostile"
    );
    let lock = |arena: &mut Arena, target: ObjId| {
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let _ = arena.step(
            0,
            &[],
            &[intent(
                RED,
                IntentKind::Lock {
                    contact: contact_of(RED, target),
                },
                5,
            )],
        );
    };
    lock(&mut arena, RED2);
    assert!(fired_lines(&mut arena).is_empty());
    lock(&mut arena, BLUE);
    assert_eq!(
        fired_lines(&mut arena),
        vec![(0, u64::from(BLUE.0), Some("cond".to_owned()))],
        "the Lock trigger fired the best-hostile slot on the hostile lock"
    );

    // The warrant, term by term, in order; readiness is never a term.
    let (arena, _) = locked_duel(100);
    let at = arena.now_ms;
    let held = |slot: FireCondition| arena.warrant(RED, &slot, at);
    let failing = FireCondition {
        min_rounds_left: 40,
        min_range_m: 1_000,
        aspects: aspect_mask(&[Aspect::Front]),
        min_permille: 900,
        ..cond("cannon", 100, ASPECTS_ANY)
    };
    use tacsim::arena::conditions::Held;
    assert_eq!(
        held(failing.clone()),
        Err(Held::Why(HeldWhy::Rounds {
            rounds: 40,
            keeps: 40
        }))
    );
    let failing = FireCondition {
        min_rounds_left: 0,
        ..failing
    };
    let Err(Held::Why(HeldWhy::Range {
        range_m,
        min_m,
        max_m,
        ..
    })) = held(failing.clone())
    else {
        panic!("the range term");
    };
    // The duel's plot stands four cells (40 m) off: on the lock's 10 m step
    // the rounded figure is the raw one, so this pins the term's place only.
    assert_eq!((range_m, min_m, max_m), (40, 1_000, u32::MAX));
    // **The ROUNDED range**: the plot moved to 47 m and stamped at `at` (age
    // nought, so the lock's 10 m step) — the pilot's `RANGE` prints 50, and
    // the term reads 50, not the raw 47. A window of exactly [50, 50] passes
    // the range term (the aspect, next, answers), which the raw 47 would not.
    let mut moved = arena.clone();
    let red_x = moved.unit(RED).map(|state| state.pose.x_cm).unwrap_or(0);
    if let Some(row) = moved.contacts.get_mut(RED, contact_of(RED, BLUE)) {
        row.seen_at.x_cm = red_x + 4_700;
        row.last_seen_ms = at;
    }
    assert_eq!(
        moved
            .aim_at(RED, contact_of(RED, BLUE))
            .map(|aim| aim.range_cm),
        Some(4_700),
        "the raw range"
    );
    assert!(
        matches!(
            moved.warrant(RED, &failing, at),
            Err(Held::Why(HeldWhy::Range { range_m: 50, .. }))
        ),
        "the rounded figure: {:?}",
        moved.warrant(RED, &failing, at)
    );
    let exactly_fifty = FireCondition {
        min_range_m: 50,
        max_range_m: 50,
        ..failing.clone()
    };
    assert!(
        matches!(
            moved.warrant(RED, &exactly_fifty, at),
            Err(Held::Why(HeldWhy::Aspect { .. }))
        ),
        "50 m is inside [50, 50]: {:?}",
        moved.warrant(RED, &exactly_fifty, at)
    );
    let raw_window = FireCondition {
        min_range_m: 46,
        max_range_m: 48,
        ..failing.clone()
    };
    assert!(
        matches!(
            moved.warrant(RED, &raw_window, at),
            Err(Held::Why(HeldWhy::Range { range_m: 50, .. }))
        ),
        "the raw 47 is never read: {:?}",
        moved.warrant(RED, &raw_window, at)
    );
    let failing = FireCondition {
        min_range_m: 0,
        ..failing
    };
    assert!(matches!(
        held(failing.clone()),
        Err(Held::Why(HeldWhy::Aspect {
            aspect: Aspect::Left,
            ..
        }))
    ));
    let failing = FireCondition {
        aspects: ASPECTS_ANY,
        ..failing
    };
    assert_eq!(
        held(failing),
        Err(Held::Why(HeldWhy::Odds {
            permille: ODDS,
            floor: 900
        }))
    );
    // A cycling gun is no warrant failure.
    let mut cycling = arena.clone();
    if let Some(state) = cycling.unit_mut(RED) {
        state.ready_at_ms.insert(SmolStr::new("cannon"), at + 3_000);
    }
    assert_eq!(
        cycling.warrant(RED, &cond("cannon", 100, ASPECTS_ANY), at),
        Ok((contact_of(RED, BLUE), ODDS, Aspect::Left))
    );
}

// ---------------------------------------------------------------------------
// 8. Hold fire
// ---------------------------------------------------------------------------

/// **A standing `hold_fire` stops a slot and never a pilot** (17b cut M1b.1;
/// doc 17 §5's precedence, D-UM-19): a Reflex-flown hull given a slot through
/// the arena's own `resolve_arming` (the arm production cannot reach, pinned)
/// under a persona order with `hold_fire` never fires a slot shot; once the
/// order is dropped the slot fires at its next evaluation. A piloted hull's
/// typed `FIRE` under no order fires. (`tests/reflex.rs`' own hold-fire test
/// is unchanged; the mind's half is 17b cut M1b.2's.)
#[test]
fn m1b1_hold_fire_stops_a_slot_and_never_a_pilot() {
    use tacsim::deliberation::order::{CharacterRef, Constraints, Objective, Order, StandingOrder};
    let (mut arena, mut wall) = locked_duel(100);
    let squad = tacsim::deliberation::squad::SquadId {
        team: Team::Red,
        index: 1,
    };
    arena.set_controller(
        RED,
        Controller::Reflex,
        tacsim::arena::reflex::ReflexState::Patrol,
    );
    let order = Order {
        squad,
        objective: Objective::Hold,
        target: None,
        horizon_ms: 600_000,
        constraints: Constraints {
            hold_fire: true,
            ..Constraints::default()
        },
        ttl_ms: 600_000,
        issued_ms: 0,
        issuer: None,
        by: CharacterRef::Persona { squad },
        spot: None,
    };
    arena
        .reflex
        .set_order(RED, StandingOrder::accepted(order, arena.now_ms));
    // The machine keeps the lock its seat took (`set_controller` keeps it).
    if arena.unit(RED).and_then(|state| state.lock).is_none() {
        let _ = arena.resolve_lock(RED, contact_of(RED, BLUE), arena.now_ms);
    }
    assert_eq!(
        arena.resolve_arming(
            RED,
            &Arming::Arm(cond("cannon", 100, ASPECTS_ANY)),
            arena.now_ms
        ),
        Ok(Some(1))
    );
    let until = arena.now_ms + 10_000;
    while arena.now_ms < until {
        wall += 500;
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let _ = arena.step(wall, &[], &[]);
    }
    let shots = fired_lines(&mut arena);
    assert!(
        shots.iter().all(|(_, _, by)| by.is_none()),
        "no slot shot under hold_fire: {shots:?}"
    );
    assert!(
        shots.is_empty(),
        "nor a machine's under hold_fire: {shots:?}"
    );
    // The positive control: through the whole window the lock stood, the slot
    // stood and its terms held — so `hold_fire` is the one reason nothing
    // fired, and not an idle slot.
    assert_eq!(
        arena.unit(RED).and_then(|state| state.lock),
        Some(contact_of(RED, BLUE)),
        "the lock stood through the hold"
    );
    assert_eq!(arena.conditions.slot_of(RED, "cannon"), Some(1));
    assert_eq!(
        arena
            .warrant(RED, &cond("cannon", 100, ASPECTS_ANY), arena.now_ms)
            .map(|(contact, _, _)| contact),
        Ok(contact_of(RED, BLUE)),
        "the slot's terms held"
    );
    // The order dropped: the slot fires at its next evaluation.
    arena.reflex.drop_to_doctrine(RED, true);
    if arena.unit(RED).and_then(|state| state.lock).is_none() {
        let _ = arena.resolve_lock(RED, contact_of(RED, BLUE), arena.now_ms);
    }
    let released = arena.now_ms;
    let until = released + 3_000;
    while arena.now_ms < until {
        wall += 250;
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let _ = arena.step(wall, &[], &[]);
    }
    let shots = fired_lines(&mut arena);
    assert!(
        shots
            .iter()
            .any(|(t, _, by)| *t >= released && by.as_deref() == Some("cond")),
        "the slot fired once the order went: {shots:?}"
    );

    // A pilot's typed FIRE under no order fires.
    let (mut arena, wall) = locked_duel(100);
    let _ = refused_now(&mut arena, wall, vec![IntentKind::fire("cannon")]);
    assert_eq!(fired_lines(&mut arena).len(), 1);
}

// ---------------------------------------------------------------------------
// 9. The seat window and the count
// ---------------------------------------------------------------------------

/// **An arming counts on the seat window, and a third weapon is `NoSlot`**
/// (17b cut M1b.1): `max_pending_per_unit + 1` seat armings in one arena
/// second, the last refused `queue_full` at phase 2 (as a `LOCK`'s) and read
/// with its weapon, `FIRE cannon refused. …`; the authored heavy
/// (`tacsim.unit.heavy_mk1`, three weapons — `cannon`, `machine_gun`,
/// `missiles`) arms two and the third is refused from phase 4 as `FIRE
/// missiles refused. Every slot is armed; FIRE <weapon> OFF frees one.`, while
/// an under-floor third is `Unlawful` (the law before the count); a re-arm
/// keeps the slot's number, replaces its terms and clears its
/// `held_said` rows; a disarm of S1 leaves S2 numbered 2 and a new arming
/// takes S1; `Disarm { None }` empties the key.
#[test]
fn m1b1_arming_counts_on_the_seat_window_and_a_third_weapon_is_no_slot() {
    let (mut arena, wall) = locked_duel(100);
    // A fresh arena second: the `LOCK` at 5,000 counted on that one's window.
    let wall = wall + 1_000;
    arena.clock.pin_tempo_for_tests(Tempo::Quiet);
    let _ = arena.step(wall, &[], &[]);
    let bound = arena.arena_cfg.intents.max_pending_per_unit;
    let front = aspect_mask(&[Aspect::Front]);
    let kinds: Vec<IntentKind> = (0..=bound)
        .map(|_| arm(cond("cannon", 100, front)))
        .collect();
    let refused = refused_now(&mut arena, wall, kinds);
    assert_eq!(
        refused
            .iter()
            .filter_map(|(notice, _)| notice.refusal())
            .collect::<Vec<_>>(),
        vec![Refusal::QueueFull],
        "the one past the window, told at phase 2"
    );
    // Told at phase 2, it names its weapon as every `FIRE` refusal does.
    assert_eq!(
        refused.iter().map(sentence).collect::<Vec<_>>(),
        vec!["FIRE cannon refused. Too many orders at once; try that again.".to_owned()],
        "the arming's queue_full reads as a FIRE refusal with its weapon"
    );

    // The heavy.
    let catalog = tacsim::definitions::Catalog::build_fielded(&[], tacsim::NS)
        .expect("the fielded content binds");
    let key = mush_core::CanonicalId::new("tacsim.unit.heavy_mk1").expect("a canonical id");
    let spec = catalog.specs.get(&key).expect("the heavy's entry");
    let binding = catalog.bindings.get(&key).expect("the heavy's binding");
    let cells = tacsim::definitions::to_tactical_unit(spec, binding, RED).expect("the heavy");
    let heavy = tacsim::match_::custody_unit(
        &catalog,
        cells,
        RED,
        Team::Red,
        facing(5, 8, 90_000),
        0,
        Controller::Pilot,
        1,
    );
    let weapons: Vec<String> = heavy.weapons.iter().map(|w| w.id.to_string()).collect();
    assert_eq!(weapons, ["cannon", "machine_gun", "missiles"]);
    let mut arena = arena_of(vec![
        heavy,
        unit(BLUE, Team::Blue, facing(9, 8, 0), Vec::new(), None),
    ]);
    let refused = refused_now(
        &mut arena,
        0,
        vec![
            arm(cond("cannon", 100, front)),
            arm(cond("machine_gun", 100, front)),
            arm(cond("missiles", 100, front)),
        ],
    );
    let [notice] = refused.as_slice() else {
        panic!("one refusal: {refused:?}");
    };
    assert_eq!(notice.0.refusal(), Some(Refusal::NoSlot));
    assert_eq!(
        sentence(notice),
        "FIRE missiles refused. Every slot is armed; FIRE <weapon> OFF frees one."
    );
    assert_eq!(
        (
            arena.conditions.slot_of(RED, "cannon"),
            arena.conditions.slot_of(RED, "machine_gun"),
            arena.conditions.slot_of(RED, "missiles")
        ),
        (Some(1), Some(2), None)
    );
    // The law before the count: an under-floor arming onto a full table is
    // `Unlawful`, naming the floor, and never `NoSlot`.
    assert_eq!(
        arena.resolve_arming(RED, &Arming::Arm(cond("missiles", 50, front)), 0),
        Err(Refusal::Unlawful)
    );
    // A re-arm: same number, new terms, its held rows gone.
    arena.held_said.insert((RED, 1, ContactId(7)), 1_000);
    arena.held_said.insert((RED, 2, ContactId(7)), 1_000);
    assert_eq!(
        arena.resolve_arming(RED, &Arming::Arm(cond("cannon", 300, front)), 0),
        Ok(Some(1))
    );
    assert_eq!(
        arena.conditions.slots(RED).first().cloned().flatten(),
        Some(cond("cannon", 300, front))
    );
    assert_eq!(
        arena.held_said.keys().copied().collect::<Vec<_>>(),
        vec![(RED, 2, ContactId(7))],
        "the re-armed slot's rows went; the other's stayed"
    );
    // A disarm of S1 leaves S2; a new arming takes S1.
    assert_eq!(
        arena.resolve_arming(
            RED,
            &Arming::Disarm {
                weapon: Some(SmolStr::new("cannon"))
            },
            0
        ),
        Ok(None)
    );
    assert_eq!(arena.conditions.slot_of(RED, "machine_gun"), Some(2));
    assert_eq!(
        arena.resolve_arming(RED, &Arming::Arm(cond("missiles", 100, front)), 0),
        Ok(Some(1))
    );
    assert_eq!(
        arena.resolve_arming(RED, &Arming::Disarm { weapon: None }, 0),
        Ok(None)
    );
    assert!(arena.conditions.is_empty_for(RED));
    assert!(arena.held_said.is_empty());
    assert_eq!(STOCK_CONDITION_SLOTS, 2);
}

// ---------------------------------------------------------------------------
// 10. The slots' life
// ---------------------------------------------------------------------------

/// **Slots die with the controller and with the turret, and survive a grace**
/// (17b cut M1b.1): a seat's arming then `Released` leaves no slot and the hash
/// sequence from that pulse on equals a never-armed run's; `Mind → Pilot`
/// clears them; `RemoveWeapons` on the turret takes the cannon's slot and
/// leaves the hull machine gun's with its number; a pilot gone `Linkdead`
/// still has the slot fire through the grace (the `Cond` exemption).
#[test]
fn m1b1_slots_die_with_the_controller_and_with_the_turret_and_survive_a_grace() {
    let contact = contact_of(RED, BLUE);
    let released = |armed: bool| {
        let mut beats = vec![beat(0, vec![intent(RED, IntentKind::Startup, 1)])];
        let mut at_5000 = vec![intent(RED, IntentKind::Lock { contact }, 2)];
        // The never-armed run sends a disarm of nothing in the arming's place,
        // so the seat's window counts the same orders on both.
        at_5000.push(intent(
            RED,
            if armed {
                // A slot that holds (front only, blue shows its left).
                arm(cond("cannon", 100, aspect_mask(&[Aspect::Front])))
            } else {
                IntentKind::Condition(Arming::Disarm { weapon: None })
            },
            3,
        ));
        beats.push(beat(5_000, at_5000));
        beats.push(Beat {
            t_ms: 10_000,
            controls: Channel::ALL
                .iter()
                .map(|channel| Control {
                    unit: RED,
                    channel: *channel,
                    epoch: 2,
                    state: ControlState::Released,
                    pilot: Some(RED_PILOT),
                })
                .collect(),
            intents: Vec::new(),
        });
        beats
    };
    let mut armed = duel(0);
    let with = replay(&mut armed, Drive::Every(500), 20_000, &released(true));
    let mut bare = duel(0);
    let without = replay(&mut bare, Drive::Every(500), 20_000, &released(false));
    assert!(armed.conditions.is_empty_for(RED), "the release cleared it");
    assert!(with.counters.cond_held > 0, "it was armed and holding");
    let after = |run: &Run| -> Vec<(i64, String, u64)> {
        run.events
            .iter()
            .filter(|(t, _, _)| *t > 10_000)
            .cloned()
            .collect()
    };
    assert_eq!(
        after(&with),
        after(&without),
        "the same lines after the release"
    );
    assert_eq!(
        tacsim::arena::state_hash(&armed),
        tacsim::arena::state_hash(&bare),
        "and the same state"
    );

    // Mind -> Pilot.
    let (mut arena, _) = locked_duel(100);
    arena.set_controller(
        RED,
        Controller::Mind,
        tacsim::arena::reflex::ReflexState::Patrol,
    );
    let _ = arena.resolve_arming(RED, &Arming::Arm(cond("cannon", 100, ASPECTS_ANY)), 0);
    assert!(!arena.conditions.is_empty_for(RED));
    // A held row stamped under the outgoing principal, and another hull's.
    arena.held_said.insert((RED, 1, ContactId(7)), 1_000);
    arena.held_said.insert((BLUE, 1, ContactId(7)), 1_000);
    arena.set_controller(
        RED,
        Controller::Pilot,
        tacsim::arena::reflex::ReflexState::Patrol,
    );
    assert!(
        arena.conditions.is_empty_for(RED),
        "Mind -> Pilot cleared it"
    );
    assert_eq!(
        arena.held_said.keys().copied().collect::<Vec<_>>(),
        vec![(BLUE, 1, ContactId(7))],
        "the outgoing principal's held rows went with its slots; another hull's stayed"
    );

    // The turret.
    let mut arena = arena_of(vec![
        unit(
            RED,
            Team::Red,
            facing(5, 8, 90_000),
            vec![cannon(), machine_gun()],
            Some(sight(0)),
        ),
        unit(BLUE, Team::Blue, facing(9, 8, 0), Vec::new(), None),
    ]);
    let front = aspect_mask(&[Aspect::Front]);
    let _ = arena.resolve_arming(RED, &Arming::Arm(cond("cannon", 100, front)), 0);
    let _ = arena.resolve_arming(RED, &Arming::Arm(cond("machine_gun", 100, front)), 0);
    arena.held_said.insert((RED, 1, ContactId(7)), 1_000);
    let mut journal = Vec::new();
    arena.apply_consequence(
        RED,
        &tacsim::arena::resolution::Consequence::RemoveWeapons {
            section: SmolStr::new("turret"),
        },
        0,
        None,
        &mut journal,
    );
    assert_eq!(arena.conditions.slot_of(RED, "cannon"), None);
    assert_eq!(
        arena.conditions.slot_of(RED, "machine_gun"),
        Some(2),
        "the gun that stands keeps its number"
    );
    assert!(arena.held_said.is_empty(), "the gone slot's rows went too");

    // A linkdead grace: the slot still fires at its WeaponReady.
    let mut script = armed_at_5000(cond("cannon", 100, ASPECTS_ANY));
    script.push(Beat {
        t_ms: 5_000,
        controls: Channel::ALL
            .iter()
            .map(|channel| Control {
                unit: RED,
                channel: *channel,
                epoch: 1,
                state: ControlState::Linkdead,
                pilot: Some(RED_PILOT),
            })
            .collect(),
        intents: Vec::new(),
    });
    let mut arena = duel(0);
    let run = replay(&mut arena, Drive::Every(500), 10_000, &script);
    assert!(arena.unit(RED).is_some_and(|state| state.linkdead));
    assert_eq!(
        cond_shots(&run)
            .iter()
            .map(|(t, _, _)| *t)
            .collect::<Vec<_>>(),
        vec![5_000, 9_000],
        "the second shot fired through the grace"
    );
}

// ---------------------------------------------------------------------------
// 12. The snapshot
// ---------------------------------------------------------------------------

/// **A snapshot carries the slots, a card, the floor and `held_said`, and the
/// resume continues the hash sequence** (17b cut M1b.1; `SNAPSHOT_FORMAT` 7):
/// an arena with an armed seat slot, a mind hull whose card was edited, a
/// floor of 50, a `held_said` map populated by hand (the `pairs` path) and, in
/// the mind's notice buffer, the two slot notices (every `HeldWhy`) and a
/// refusal carrying `RefusalDetail::Floor` round-trips at format 7; stepped on,
/// the resumed arena's substep hashes and ledger bytes equal the uninterrupted
/// one's; a format-6 document is refused by name. (`IntentKind::Fire.slot` and
/// `IntentKind::Condition` ride `Arena.pending`, empty between pulses: their
/// round trip is `snapshot.rs`' `af1_every_keyed_map_in_the_tree_is_one_json_writes`.)
#[test]
fn m1b1_a_snapshot_carries_slots_a_card_the_floor_and_held_said_and_resumes_the_hash_sequence() {
    let (mut arena, mut wall) = locked_duel(50);
    // A mind on blue, thinking nothing, its card edited.
    arena.mind_runner = Arc::new(tacsim::arena::mind::NoMind);
    arena.set_controller(
        BLUE,
        Controller::Mind,
        tacsim::arena::reflex::ReflexState::Patrol,
    );
    if let Some(state) = arena.minds.get_mut(&BLUE) {
        state.card.min_odds_permille = 300;
        state.card.stance = tacsim::arena::card::Stance::Orbit;
    }
    let _ = refused_now(
        &mut arena,
        wall,
        vec![arm(cond("cannon", 50, aspect_mask(&[Aspect::Front])))],
    );
    assert_eq!(arena.conditions.slot_of(RED, "cannon"), Some(1));
    arena
        .held_said
        .insert((RED, 1, contact_of(RED, BLUE)), 4_000);
    // A pulse boundary with no think in flight (the landing scratch is reset).
    loop {
        wall += 250;
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let _ = arena.step(wall, &[], &[]);
        if arena
            .minds
            .get(&BLUE)
            .is_some_and(|state| state.pending.is_none())
        {
            break;
        }
    }
    // The carried notice shapes format 7 added, buffered on blue's mind by
    // hand (nothing stages a slot notice before 17b cut M1b.4): a slot's shot
    // with and without a contact, a held slot under each of the four whys, and
    // a refusal carrying the floor.
    let named = tacsim::arena::perception::Named {
        id: contact_of(BLUE, RED),
        designator: tacsim::arena::state::Designator::Slot {
            team: Team::Red,
            slot: 1,
        },
    };
    let mut carried = vec![
        FrameNotice::SlotFired {
            slot: 1,
            weapon: SmolStr::new("cannon"),
            contact: Some(named),
            p_permille: 620,
            aspect: Aspect::Left,
        },
        FrameNotice::SlotFired {
            slot: 2,
            weapon: SmolStr::new("machine_gun"),
            contact: None,
            p_permille: 950,
            aspect: Aspect::Rear,
        },
        FrameNotice::Refused {
            order: SmolStr::new("fire"),
            refusal: Refusal::Unlawful,
            weapon: Some(SmolStr::new("cannon")),
            detail: RefusalDetail::Floor { permille: 50 },
        },
    ];
    for why in [
        HeldWhy::Odds {
            permille: 540,
            floor: 600,
        },
        HeldWhy::Aspect {
            contact: named,
            aspect: Aspect::Front,
        },
        HeldWhy::Range {
            contact: named,
            range_m: 1_200,
            min_m: 200,
            max_m: 600,
        },
        HeldWhy::Rounds {
            rounds: 3,
            keeps: 5,
        },
    ] {
        carried.push(FrameNotice::SlotHeld {
            slot: 1,
            weapon: SmolStr::new("cannon"),
            why,
        });
    }
    if let Some(state) = arena.minds.get_mut(&BLUE) {
        state.notices.extend(carried.iter().cloned());
    }
    let snapshot = Snapshot::encode(&arena).expect("an arena between pulses encodes");
    assert_eq!(snapshot.document["format"], 7);
    let mut resumed =
        Snapshot::decode(&snapshot.document, definition()).expect("the document decodes");
    resumed.mind_runner = Arc::new(tacsim::arena::mind::NoMind);
    assert_eq!(resumed.odds_floor_permille, 50);
    assert_eq!(resumed.conditions, arena.conditions);
    assert_eq!(resumed.held_said, arena.held_said);
    assert_eq!(
        resumed.minds.get(&BLUE).map(|state| state.card.clone()),
        arena.minds.get(&BLUE).map(|state| state.card.clone())
    );
    let resumed_notices = resumed
        .minds
        .get(&BLUE)
        .map(|state| state.notices.clone())
        .unwrap_or_default();
    assert!(
        resumed_notices.ends_with(&carried),
        "the slot notices and the floor detail round-trip: {resumed_notices:?}"
    );
    assert_eq!(
        Some(&resumed_notices),
        arena.minds.get(&BLUE).map(|state| &state.notices)
    );
    let state = snapshot.document["state"].as_str().unwrap_or_default();
    assert!(
        state.contains("\"held_said\":[["),
        "the pairs form: {state}"
    );
    resumed.resume_at(wall);
    let mut hashes: [Vec<u64>; 2] = [Vec::new(), Vec::new()];
    let mut bytes: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
    for (at, one) in [&mut arena, &mut resumed].into_iter().enumerate() {
        let mut wall_now = wall;
        while one.now_ms < 20_000 {
            wall_now += 500;
            one.clock.pin_tempo_for_tests(Tempo::Quiet);
            let out = one.step(wall_now, &[], &[]);
            for chunk in &out.ledger_chunks {
                bytes[at].extend_from_slice(&chunk.bytes);
            }
            hashes[at].extend(one.substep_hashes.iter().copied());
        }
    }
    assert_eq!(hashes[0], hashes[1], "the resumed hash sequence");
    // The ledger holds the tail of an open chunk; flush both and read every
    // line each handed out since the snapshot.
    let [first, second] = &mut bytes;
    let after = lines(&mut arena, first);
    let resumed_after = lines(&mut resumed, second);
    assert!(!after.is_empty(), "the run journalled");
    assert_eq!(after, resumed_after, "the resumed lines");
    assert_eq!(bytes[0], bytes[1], "byte for byte");
    assert_eq!(
        tacsim::arena::state_hash(&arena),
        tacsim::arena::state_hash(&resumed)
    );

    // A format-6 document is refused by name.
    let mut old = snapshot.document.clone();
    old["format"] = serde_json::json!(6);
    assert_eq!(
        Snapshot::head(&old).map(|head| head.format),
        Err(SnapshotError::Format {
            found: 6,
            expected: 7
        })
    );
    assert!(matches!(
        Snapshot::decode(&old, definition()),
        Err(SnapshotError::Format {
            found: 6,
            expected: 7
        })
    ));
}

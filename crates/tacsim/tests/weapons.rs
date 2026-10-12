//! **Weapons and the shot, through `Arena::step`** (16b cut W.1, W1; doc 16 §3.5
//! "Acceptance and cost" and "Scheduled impact", §3.7's `LOCK`, `FIRE` and
//! `WEAPONSTATUS`, §3.1's tempo table and "Rate independence").
//!
//! Ten tests, driving the library directly — no host, no store, no task, the same
//! shape `arena.rs` and `perception.rs` take:
//!
//! - **(a)** `FIRE` is refused for want of a lock, a finished cycle, a round and
//!   power, **in doc 16 §3.7's order** (power last), and a refused order leaves
//!   nothing behind: no round spent, no cycle started, no line in the black box;
//! - **(b)** an accepted shot sets `ready_at_ms`, spends the burst, and schedules
//!   **exactly one** impact at `range × 1000 / muzzle_speed_cms`;
//! - **(c)** a beam's impact is scheduled at `now`;
//! - **(d)** a shot whose line is blocked is **accepted** and costs its round —
//!   doc 16 §3.5's *there is no free probe of line of sight*;
//! - **(e)** the `Engaged` tempo is entered on a shot and held for
//!   `tempo.engaged_hold_ms`;
//! - **(f)** `WEAPONSTATUS` reads the frame's readiness, rounds and the lock's band;
//! - **(g)** a contact the platform loses clears the lock, with one notice;
//! - **(h)** the **cross-tempo property** with a scripted `LOCK` and `FIRE`: one
//!   `locked`/`fired`/`impact`/`miss` sequence and one per-substep hash sequence at
//!   5,000, 1,000 and 500 ms;
//! - **(i)** ammunition is conserved across a scripted run of four shots: rounds
//!   fired plus rounds left equals the load;
//! - **(j)** **a lock taken on a provisional number survives the rename** (16b cut
//!   Q.6, the owner's ruling of 2026-09-20): `X1` becomes `BA` and the lock, the
//!   contact id and the target are the same three things they were;
//! - **(k)** **16b cut I.3's two cross-tempo tests for the staff place**: a place
//!   at a beat is one `placed` line and one pose at three cadences, and a place
//!   with a round in flight to or from the unit is one `place_refused` line with
//!   `in_flight` that moves nothing and leaves the round where it falls; and a
//!   beam fired in a floored pulse of no time lands in that pulse, as it does
//!   in one that ran a substep (the phase-4 drain under a staff floor);
//! - **(l)** **17b cut M1b.0's production line-hash pin, the seat half**: the
//!   (h) script journals, at three cadences, the very lines it journalled at
//!   `e23694b0`, each with its hash (`tests/motor.rs` holds the shipped
//!   duel's half, `tests/reflex.rs` the reflex shots').
//!
//! # Two places a test reaches into the arena's own state, and why
//!
//! The arena's fields are public (see `arena/state.rs`), and two of these tests use
//! that deliberately:
//!
//! * **(d)** writes the shadow contact store directly. The claim under test is that
//!   *acceptance does not trace*, so how the platform came to hold a contact through
//!   a wall is beside the point — the mesh's own path (a teammate shares what it can
//!   see) is cut S's test, and building it here would make a test about `FIRE` fail
//!   for a reason about perception.
//! * **(g)** moves the target. A 16-cell window has no outside for a 600 m sight, so
//!   the way to make a contact expire is to put its target out of the *short* sight
//!   these fixtures carry, which is what a tank driving over a ridge does and what a
//!   test can do in one line.

use std::collections::BTreeMap;

use mush_core::{DefinitionDigest, DefinitionId, DefinitionKey, DefinitionKind, ObjId, OwnerId};
use mush_sim::data::Pose;
use mush_tactics::{
    CellSpec, DefinitionRef, FacetId, FacetSpec, RulesValue, SectionId, SectionSpec, TacticalUnit,
    UnitSpec,
};
use smol_str::SmolStr;
use tacsim::arena::clock::Tempo;
use tacsim::arena::intents::{Control, Intent, IntentKind, Refusal};
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena::perception::{NoticeKind, SensorProfile, SignatureFactor};
use tacsim::arena::rng::draw_contact_id;
use tacsim::arena::state::{
    Arena, ArenaDefinition, ArenaTerrain, Classification, ContactId, CustodyOut, CustodyUnit,
    Designator, EntryZone, Pilot, Power, SeenPose, ShadowContact, Team, TerrainCell, TrackedRules,
    WeaponSummary,
};
use tacsim::arena::step::{Frame, StepOutput};
use tacsim::arena::weapons::WeaponBand;
use tacsim::config::{ArenaConfig, MatchConfig, SensorsConfig};
use tacsim::match_::cockpit;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// The window: 16 by 16 cells of 1,000 cm, which is 160 m square — room for a wall
/// between two tanks and no more, because nothing here is about distance.
const SIDE: u16 = 16;

/// The cell size, centimetres.
const CELL_CM: u16 = 1_000;

/// The match seed, which is the key of every contact id a test computes.
const SEED: u64 = 2_350;

/// The shooter and the target.
const RED: ObjId = ObjId(4);
const BLUE: ObjId = ObjId(9);

/// The pilots `arena_of` seats, in unit-id order.
const RED_PILOT: ObjId = ObjId(1_000);

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

/// The smallest M32 unit the arena can carry. Cut W.1 applies no plan to it —
/// damage is cut W.3's — so one section is enough.
fn tactical_unit(id: ObjId) -> TacticalUnit {
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
            cells: vec![CellSpec {
                id: mush_tactics::CellId::new("internals").expect("a cell"),
                min: 0,
                max: 8,
                init: 8,
            }],
        }],
        unit_cells: Vec::new(),
        components: vec![core],
        capabilities: vec![capability],
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

/// An impassable, opaque wall — what makes a line of sight `BlockedBy`.
fn wall() -> TerrainCell {
    TerrainCell {
        class: mush_atlas::TerrainClass::Wall,
        elev_cm: 0,
        move_cost: 0,
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

/// The window, with `paint`ed cells over uniform ground.
fn definition(paint: &[(u16, u16, TerrainCell)]) -> ArenaDefinition {
    let mut cells = vec![ground(); usize::from(SIDE) * usize::from(SIDE)];
    for (x, y, cell) in paint {
        if let Some(slot) = cells.get_mut(usize::from(*y) * usize::from(SIDE) + usize::from(*x)) {
            *slot = *cell;
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
        // 16b cut N.1: no authored furniture on this fixture window — the
        // reflex controller is not what these tests are about, and every unit
        // on them is a `Pilot`.
        key_spots: Vec::new(),
        waypoints: tacsim::arena::state::Waypoints::default(),
        modes: Vec::new(),
    }
}

/// **A five-cell sight**, so a 16-cell window has an outside and a contact can be
/// made to expire (test (g)). Everything else is
/// `content/sensors/visual_mk1.json`'s.
fn sight() -> SensorProfile {
    use mush_atlas::TerrainClass as C;
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
        // No mesh: one unit a side, so there is nobody to share with.
        share_range_cells: 0,
        signature: vec![row(C::Ground, 1_000, 1_000), row(C::Grass, 1_000, 800)],
    }
}

/// **A cannon**: 2.5 km of reach in two bands, a four-second cycle, one round a
/// pull, a forty-round load, and 1,450 m/s at the muzzle.
///
/// Close to `content/weapons/cannon_105.json` and deliberately not equal to it: the
/// authored numbers are cut C's and a test that asserted them would be asserting the
/// content.
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
        // 16b cut W.2: the round's damage and the section it is mounted in. These
        // fixtures carry no authored silhouette and no facet table, so every shot
        // resolves at zero and every arrival is a `dispersion` miss — which is what
        // keeps this file about acceptance and `crates/tacsim/tests/resolution.rs`
        // about what a round does.
        damage: 120,
        mount_section: SmolStr::new("turret"),
    }
}

/// A tank that sees and shoots, with `weapons` for what it shoots with.
fn scout(id: ObjId, team: Team, pose: Pose, weapons: Vec<WeaponSummary>) -> CustodyUnit {
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
        sensor: Some(sight()),
        weapons,
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

/// A match short enough to test: power comes up in half a second.
fn match_cfg() -> MatchConfig {
    MatchConfig {
        startup_ms: 500,
        ..MatchConfig::default()
    }
}

/// An arena over `paint`ed ground with one seated pilot per unit (`#1000`, `#1001`,
/// … in unit order).
fn arena_of(
    units: Vec<CustodyUnit>,
    paint: &[(u16, u16, TerrainCell)],
    match_cfg: &MatchConfig,
) -> Arena {
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
            definition: definition(paint),
            units,
            pilots,
            resolution: tacsim::match_::stock_resolution(),
            seed: SEED,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &ArenaConfig::default(),
        match_cfg,
        &SensorsConfig::default(),
    )
    .expect("the test arena")
}

/// Two tanks four cells apart on clear ground, the red one armed.
fn duel() -> Arena {
    arena_of(
        vec![
            scout(RED, Team::Red, centre(5, 8), vec![cannon()]),
            scout(BLUE, Team::Blue, centre(9, 8), Vec::new()),
        ],
        &[],
        &match_cfg(),
    )
}

/// One order, on the channel its kind travels on, under the seated epoch.
fn intent(unit: ObjId, kind: IntentKind, seq: u64) -> Intent {
    Intent {
        unit,
        channel: kind.channel_required(),
        epoch: 1,
        kind,
        seq,
    }
}

/// **Where `target` actually stands, as a contact's plot** (16b cut F.3).
///
/// These fixtures plant a contact by hand, and the plot they plant is the
/// target's true pose — so every range and bearing they assert is the one they
/// asserted before the contact store learned the difference.
fn plot_of(arena: &Arena, target: mush_core::ObjId) -> SeenPose {
    arena.unit(target).map(SeenPose::of).unwrap_or_default()
}

/// The contact id `viewer` holds `target` under in this match.
fn contact_of(viewer: ObjId, target: ObjId) -> ContactId {
    draw_contact_id(SEED, viewer, target)
}

/// Step until `until_ms` of arena time at `cadence_ms`, absorbing the ledger, and
/// answering every frame each pilot was sent.
fn run(
    arena: &mut Arena,
    cadence_ms: i64,
    until_ms: i64,
    bytes: &mut Vec<u8>,
) -> Vec<(ObjId, Frame)> {
    let mut wall = arena.now_ms;
    let mut frames: Vec<(ObjId, Frame)> = Vec::new();
    while arena.now_ms < until_ms {
        let out = arena.step(wall, &[], &[]);
        absorb(&out, bytes);
        frames.extend(out.frames);
        if out.ended.is_some() {
            break;
        }
        wall += cadence_ms;
    }
    frames
}

/// One pulse with `intents`, absorbing the ledger.
fn pulse(arena: &mut Arena, wall_ms: i64, intents: &[Intent], bytes: &mut Vec<u8>) -> StepOutput {
    let out = arena.step(wall_ms, &[], intents);
    absorb(&out, bytes);
    out
}

fn absorb(out: &StepOutput, bytes: &mut Vec<u8>) {
    for chunk in &out.ledger_chunks {
        bytes.extend_from_slice(&chunk.bytes);
    }
}

/// Everything journalled so far, whatever is still open included.
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

/// How many events of one kind word were journalled.
fn count(events: &[LedgerEvent], kind: &str) -> usize {
    events
        .iter()
        .filter(|event| event.kind.as_str() == kind)
        .count()
}

/// Bring both tanks online and let the first sensor pass run, answering the wall
/// reading to carry on from.
fn warm(arena: &mut Arena, bytes: &mut Vec<u8>) -> i64 {
    let mut wall = 0_i64;
    pulse(
        arena,
        wall,
        &[
            intent(RED, IntentKind::Startup, 1),
            intent(BLUE, IntentKind::Startup, 2),
        ],
        bytes,
    );
    // One second of arena time: `match.startup_ms` is 500 and the first pass of each
    // unit is at its stagger inside `sensors.pass_ms` (`stagger_ms`, 16b cut U.1),
    // so both are behind us.
    while arena.now_ms < 1_000 {
        wall += 500;
        pulse(arena, wall, &[], bytes);
    }
    assert_eq!(
        arena.unit(RED).map(|state| state.power),
        Some(Power::Online),
        "the red tank never came online"
    );
    assert!(
        arena.contacts.get(RED, contact_of(RED, BLUE)).is_some(),
        "the red tank never detected the blue one"
    );
    wall
}

// ---------------------------------------------------------------------------
// (a) The refusals, in doc 16 section 3.7's order
// ---------------------------------------------------------------------------

#[test]
fn fire_is_refused_for_a_lock_a_cycle_a_round_and_power_in_that_order() {
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    // Cold, unlocked, loaded, ready. **Power is checked last** (doc 16 §3.7's
    // refusal column), so the answer is the lock.
    assert_eq!(arena.unit(RED).map(|state| state.power), Some(Power::Off));
    assert_eq!(
        arena.resolve_fire(RED, "cannon", arena.now_ms),
        Err(Refusal::NoLock)
    );
    // A weapon the unit does not carry is refused before anything about the shot.
    assert_eq!(
        arena.resolve_fire(RED, "trebuchet", arena.now_ms),
        Err(Refusal::NoWeapon)
    );
    // A tank with nothing to shoot with is the same answer, not a panic.
    assert_eq!(
        arena.resolve_fire(BLUE, "cannon", arena.now_ms),
        Err(Refusal::NoWeapon)
    );

    let wall = warm(&mut arena, &mut bytes);
    let contact = contact_of(RED, BLUE);
    pulse(
        &mut arena,
        wall + 500,
        &[intent(RED, IntentKind::Lock { contact }, 3)],
        &mut bytes,
    );
    assert_eq!(arena.unit(RED).and_then(|state| state.lock), Some(contact));

    // Cycling, with a lock and rounds and power.
    let cycling_until = arena.now_ms + 1;
    if let Some(state) = arena.unit_mut(RED) {
        state
            .ready_at_ms
            .insert(SmolStr::new("cannon"), cycling_until);
    }
    assert_eq!(
        arena.resolve_fire(RED, "cannon", arena.now_ms),
        Err(Refusal::Cycling)
    );
    // Empty, ready.
    if let Some(state) = arena.unit_mut(RED) {
        state.ready_at_ms.insert(SmolStr::new("cannon"), 0);
        state.ammo.insert(SmolStr::new("cannon"), 0);
    }
    assert_eq!(
        arena.resolve_fire(RED, "cannon", arena.now_ms),
        Err(Refusal::Empty)
    );
    // Loaded, ready, locked — and cold, which is the last thing asked.
    if let Some(state) = arena.unit_mut(RED) {
        state.ammo.insert(SmolStr::new("cannon"), 40);
        state.power = Power::Off;
    }
    assert_eq!(
        arena.resolve_fire(RED, "cannon", arena.now_ms),
        Err(Refusal::PowerNotOnline)
    );

    // **A refused order leaves nothing behind**, and that is what the pulse is for:
    // the same `FIRE` through `step` spends no round, starts no cycle and writes no
    // line.
    let before = arena.unit(RED).map(|state| state.weapon_ammo("cannon"));
    let out = pulse(
        &mut arena,
        wall + 1_000,
        &[intent(
            RED,
            IntentKind::Fire {
                weapon: SmolStr::new("cannon"),
                slot: None,
            },
            4,
        )],
        &mut bytes,
    );
    assert!(out.ended.is_none());
    assert_eq!(
        arena.unit(RED).map(|state| state.weapon_ammo("cannon")),
        before
    );
    assert_eq!(
        arena
            .unit(RED)
            .map(|state| state.weapon_ready_at_ms("cannon")),
        Some(0)
    );
    assert!(arena.shots.is_empty(), "nothing is in flight");
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "fired"), 0, "a refused order writes no line");
    assert_eq!(count(&log, "locked"), 1, "the lock did land");
}

// ---------------------------------------------------------------------------
// (b) An accepted shot
// ---------------------------------------------------------------------------

#[test]
fn an_accepted_shot_sets_the_cycle_spends_the_burst_and_schedules_one_impact() {
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    let contact = contact_of(RED, BLUE);
    // The lock and the shot in one burst: the queue is the pilot's order, so phase 4
    // resolves the lock and then the shot, in this pulse.
    let out = pulse(
        &mut arena,
        wall + 500,
        &[
            intent(RED, IntentKind::Lock { contact }, 3),
            intent(
                RED,
                IntentKind::Fire {
                    weapon: SmolStr::new("cannon"),
                    slot: None,
                },
                4,
            ),
        ],
        &mut bytes,
    );
    let fired_at = arena.now_ms;
    assert_eq!(out.tempo, Tempo::Engaged, "a shot is an engaged signal");

    // The two cells the shot moved.
    let state = arena.unit(RED).expect("the red tank");
    assert_eq!(state.weapon_ammo("cannon"), 39, "one round, one burst");
    assert_eq!(
        state.weapon_ready_at_ms("cannon"),
        fired_at + i64::from(cannon().cycle_ms)
    );
    assert!(!state.weapon_ready("cannon", fired_at));

    // Exactly one shot in flight, at the range it was fired at, and exactly one
    // `Impact` on the heap for it — plus the `WeaponReady` that wakes the pulse when
    // the breech is free.
    assert_eq!(arena.shots.len(), 1);
    let shot = arena.shot(1).cloned().expect("shot 1");
    assert_eq!((shot.shooter, shot.target, shot.rounds), (RED, BLUE, 1));
    assert_eq!(shot.range_cm, 4_000, "four cells of 1,000 cm");
    assert_eq!((shot.aim_x_cm, shot.aim_y_cm), (9_500, 8_500));
    assert_eq!(shot.aim_cell, Some((9, 8)));
    let flight = 4_000_i64 * 1_000 / i64::from(cannon().muzzle_speed_cms);
    assert_eq!(shot.impact_at_ms, fired_at + flight);
    assert_eq!(
        arena
            .due
            .iter()
            .filter(|event| matches!(
                event.kind,
                tacsim::arena::state::DueKind::Impact { shot_seq: 1 }
            ))
            .count(),
        1,
        "exactly one impact for one shot"
    );
    assert_eq!(
        arena
            .due
            .iter()
            .filter(|event| matches!(
                event.kind,
                tacsim::arena::state::DueKind::WeaponReady { .. }
            ))
            .map(|event| event.t_ms)
            .collect::<Vec<_>>(),
        vec![fired_at + i64::from(cannon().cycle_ms)]
    );

    // The `fired` line carries the shot's sequence, its target and its range.
    let log = events(&mut arena, &mut bytes);
    let entry = log
        .iter()
        .find(|event| event.kind.as_str() == "fired")
        .cloned()
        .expect("one fired line");
    assert_eq!(
        entry.kind,
        EventKind::Fired {
            unit: RED,
            weapon: SmolStr::new("cannon"),
            shot_seq: 1,
            target: BLUE,
            range_cm: 4_000,
            rounds: 1,
            by: None,
            slot: None,
        }
    );
    assert!(entry.draws.is_empty(), "cut W.1 draws nothing");

    // The impact arrives, leaves the table, and journals cut W.1's placeholder.
    let _ = run(&mut arena, 500, fired_at + 1_000, &mut bytes);
    assert!(arena.shots.is_empty(), "the shot left the table");
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "impact"), 1);
    let miss = log
        .iter()
        .find(|event| event.kind.as_str() == "miss")
        .cloned()
        .expect("one miss line");
    // 16b cut W.2: these fixtures present no silhouette at all, so the round is
    // aimed correctly and finds nothing — doc 16 §3.5's dispersion, resolved rather
    // than deferred.
    assert_eq!(
        miss.kind,
        EventKind::Miss {
            shot_seq: 1,
            reason: SmolStr::new(tacsim::arena::damage::DISPERSION),
        }
    );
    // And the cycle really did run down, by itself.
    let _ = run(
        &mut arena,
        500,
        fired_at + i64::from(cannon().cycle_ms) + 500,
        &mut bytes,
    );
    assert!(
        arena
            .unit(RED)
            .is_some_and(|state| state.weapon_ready("cannon", arena.now_ms)),
        "the cannon is ready again"
    );
}

// ---------------------------------------------------------------------------
// (c) A beam
// ---------------------------------------------------------------------------

#[test]
fn a_beam_is_scheduled_at_now() {
    let beam = WeaponSummary {
        id: SmolStr::new("laser"),
        muzzle_speed_cms: 0,
        ..cannon()
    };
    let mut arena = arena_of(
        vec![
            scout(RED, Team::Red, centre(5, 8), vec![beam]),
            scout(BLUE, Team::Blue, centre(9, 8), Vec::new()),
        ],
        &[],
        &match_cfg(),
    );
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    let contact = contact_of(RED, BLUE);
    let out = pulse(
        &mut arena,
        wall + 500,
        &[
            intent(RED, IntentKind::Lock { contact }, 3),
            intent(
                RED,
                IntentKind::Fire {
                    weapon: SmolStr::new("laser"),
                    slot: None,
                },
                4,
            ),
        ],
        &mut bytes,
    );
    // **16b cut M.2** (16c `B-51`): the round is no longer left in flight for
    // the next pulse to find — the phase-4 drain lands it in the pulse that fired
    // it, so what this test reads is the ledger and not `Arena::shot`. (It read
    // `arena.shot(1)` and compared its two stamps before the cut; the shot is
    // resolved by the time the pulse returns now.)
    assert!(arena.shot(1).is_none(), "the beam is still in flight");
    assert_eq!(
        out.substeps, 3,
        "the pulse's two substeps and the phase-4 drain"
    );
    let log = events(&mut arena, &mut bytes);
    let fired = log
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::Fired {
                shot_seq: 1,
                range_cm,
                ..
            } => Some((event.t_ms, *range_cm)),
            _ => None,
        })
        .expect("the `fired` line");
    assert!(fired.1 > 0, "and it still has a range");
    let impact = log
        .iter()
        .find(|event| matches!(event.kind, EventKind::Impact { shot_seq: 1, .. }))
        .map(|event| event.t_ms);
    assert_eq!(impact, Some(fired.0), "a beam has no flight time");
}

// ---------------------------------------------------------------------------
// (d) A blocked shot
// ---------------------------------------------------------------------------

#[test]
fn a_blocked_shot_is_accepted_and_costs_its_round() {
    // A wall right between them, so the line is blocked at acceptance and the two
    // tanks never see each other.
    let mut arena = arena_of(
        vec![
            scout(RED, Team::Red, centre(5, 8), vec![cannon()]),
            scout(BLUE, Team::Blue, centre(9, 8), Vec::new()),
        ],
        &[(7, 8, wall())],
        &match_cfg(),
    );
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall_ms = 0_i64;
    pulse(
        &mut arena,
        wall_ms,
        &[intent(RED, IntentKind::Startup, 1)],
        &mut bytes,
    );
    while arena.now_ms < 1_000 {
        wall_ms += 500;
        pulse(&mut arena, wall_ms, &[], &mut bytes);
    }
    let contact = contact_of(RED, BLUE);
    assert!(
        arena.contacts.get(RED, contact).is_none(),
        "nothing is seen through a wall: this is why the store is written by hand"
    );
    // The platform holds the contact anyway — see the module head.
    arena.contacts.insert(
        RED,
        contact,
        ShadowContact::sighted(
            BLUE,
            Classification::Classified { team: Team::Blue },
            arena.now_ms,
            plot_of(&arena, BLUE),
        ),
    );
    let before = arena
        .unit(RED)
        .map(|state| state.weapon_ammo("cannon"))
        .unwrap_or(0);
    pulse(
        &mut arena,
        wall_ms + 500,
        &[
            intent(RED, IntentKind::Lock { contact }, 3),
            intent(
                RED,
                IntentKind::Fire {
                    weapon: SmolStr::new("cannon"),
                    slot: None,
                },
                4,
            ),
        ],
        &mut bytes,
    );
    // **Accepted**: doc 16 §3.5's *there is no free probe of line of sight*. The
    // round and the cycle are spent and the impact is on the heap; cut W.2 is what
    // turns it into a miss naming the wall.
    assert_eq!(
        arena.unit(RED).map(|state| state.weapon_ammo("cannon")),
        Some(before.saturating_sub(1)),
        "the round is spent whatever the ground says"
    );
    assert_eq!(arena.shots.len(), 1);
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "fired"), 1);
}

// ---------------------------------------------------------------------------
// (e) The tempo
// ---------------------------------------------------------------------------

#[test]
fn the_engaged_tempo_is_entered_on_a_shot_and_held() {
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = warm(&mut arena, &mut bytes);
    // Two tanks holding each other inside `tempo.alert_range_cm` is `Alert` and
    // nothing more (doc 16 §3.1's table), which is what makes the rise to `Engaged`
    // attributable to the shot.
    assert_eq!(arena.clock.tempo(), Tempo::Alert);
    let contact = contact_of(RED, BLUE);
    wall += 500;
    let out = pulse(
        &mut arena,
        wall,
        &[
            intent(RED, IntentKind::Lock { contact }, 3),
            intent(
                RED,
                IntentKind::Fire {
                    weapon: SmolStr::new("cannon"),
                    slot: None,
                },
                4,
            ),
        ],
        &mut bytes,
    );
    assert_eq!(out.tempo, Tempo::Engaged);
    let fired_at = arena.now_ms;
    // It holds for `tempo.engaged_hold_ms` of arena time, and is left at exactly it.
    let hold = i64::from(ArenaConfig::default().tempo.engaged_hold_ms);
    while arena.now_ms < fired_at + hold - 500 {
        wall += 500;
        let out = pulse(&mut arena, wall, &[], &mut bytes);
        assert_eq!(
            out.tempo,
            Tempo::Engaged,
            "the hold broke at {} ms",
            arena.now_ms
        );
    }
    while arena.now_ms < fired_at + hold + 500 {
        wall += 500;
        pulse(&mut arena, wall, &[], &mut bytes);
    }
    // **16b cut N.1** made doc 16 §3.1's third `Engaged` trigger live — *hostile
    // units are within weapons range of each other* — and a 250 km cannon reaches
    // every cell of an eight-cell window. So by the time the shot's own hold has run
    // out the red tank has told the blue one apart and is holding it in range, and
    // the tempo stays `Engaged` on that trigger rather than falling back.
    assert_eq!(arena.clock.tempo(), Tempo::Engaged);
    assert!(
        arena.hostile_in_weapons_range(),
        "the red tank holds the blue one classified, inside its own reach"
    );
    // Take the gun off it and the third trigger goes with it. The two still hold
    // each other inside `tempo.alert_range_cm`, so one `engaged_hold_ms` later the
    // arena is back at `Alert` — the two holds are independent (doc 16 §3.1).
    //
    // The weapon rows alone first, the rounds kept — a turret shot off (16b cut
    // F.1) — and the trigger is gone already: the reach is the guns', never the
    // magazines'.
    if let Some(state) = arena.unit_mut(RED) {
        state.weapons.clear();
    }
    assert!(
        arena
            .unit(RED)
            .is_some_and(|state| state.armed_at_checkout()),
        "the rounds are still aboard"
    );
    assert!(!arena.hostile_in_weapons_range());
    // **16b cut AP.2**: and that state is a side that came in armed with no gun
    // left, which is out — the match would end at the next substep and its
    // clock stand, and this test is about the tempo's triggers, not about who
    // wins. So the gun's record goes too (`ammo`, which is what
    // `UnitState::armed_at_checkout` reads): red is then a side that never had
    // a gun, which the clause leaves standing.
    assert_eq!(arena.eliminated(), Some(Some(Team::Blue)));
    if let Some(state) = arena.unit_mut(RED) {
        state.ammo.clear();
    }
    assert!(arena.ended.is_none() && arena.eliminated().is_none());
    assert!(!arena.hostile_in_weapons_range());
    let quiet_from = arena.now_ms;
    while arena.now_ms < quiet_from + hold + 500 {
        wall += 500;
        pulse(&mut arena, wall, &[], &mut bytes);
    }
    assert_eq!(arena.clock.tempo(), Tempo::Alert);
    let log = events(&mut arena, &mut bytes);
    assert!(
        log.iter().any(|event| matches!(
            event.kind,
            EventKind::Tempo {
                to: Tempo::Engaged,
                ..
            }
        )),
        "the rise is a ledger event"
    );
}

// ---------------------------------------------------------------------------
// (f) `WEAPONSTATUS`
// ---------------------------------------------------------------------------

#[test]
fn weaponstatus_reads_the_frames_readiness_rounds_and_band() {
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = warm(&mut arena, &mut bytes);
    let contact = contact_of(RED, BLUE);
    wall += 500;
    let out = pulse(
        &mut arena,
        wall,
        &[intent(RED, IntentKind::Lock { contact }, 3)],
        &mut bytes,
    );
    let frame = out
        .frames
        .iter()
        .find(|(pilot, _)| *pilot == RED_PILOT)
        .map(|(_, frame)| frame.clone())
        .expect("the red pilot's frame");
    assert_eq!(frame.lock, Some(contact), "the frame carries the lock");
    let rows = cockpit::frame_weapons(&frame);
    assert_eq!(rows.len(), 1);
    let row = rows.first().cloned().expect("the cannon");
    assert!(row.ready && row.ready_in_ms == 0);
    assert_eq!((row.rounds, row.rounds_max), (40, 40));
    assert_eq!(
        row.range_band.map(|band| (band.index, band.of)),
        Some((1, 2)),
        "four cells is the inner band"
    );
    let line = row.render(&rows);
    assert!(line.contains("cannon"), "{line}");
    assert!(line.contains("ready"), "{line}");
    assert!(line.contains("40 of 40 round(s)"), "{line}");
    assert!(line.contains("band 1 of 2"), "{line}");

    // After the shot: cycling, one round down, and the countdown in seconds.
    wall += 500;
    let out = pulse(
        &mut arena,
        wall,
        &[intent(
            RED,
            IntentKind::Fire {
                weapon: SmolStr::new("cannon"),
                slot: None,
            },
            4,
        )],
        &mut bytes,
    );
    let frame = out
        .frames
        .iter()
        .find(|(pilot, _)| *pilot == RED_PILOT)
        .map(|(_, frame)| frame.clone())
        .expect("the red pilot's frame");
    let rows = cockpit::frame_weapons(&frame);
    let row = rows.first().cloned().expect("the cannon");
    assert!(!row.ready);
    assert_eq!(row.rounds, 39);
    assert_eq!(row.ready_in_ms, i64::from(cannon().cycle_ms));
    let line = row.render(&rows);
    assert!(line.contains("cycling (4 s)"), "{line}");
    assert!(line.contains("39 of 40 round(s)"), "{line}");
}

// ---------------------------------------------------------------------------
// (g) A lost contact clears the lock
// ---------------------------------------------------------------------------

#[test]
fn a_contact_the_platform_loses_clears_the_lock_with_one_notice() {
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = warm(&mut arena, &mut bytes);
    let contact = contact_of(RED, BLUE);
    wall += 500;
    pulse(
        &mut arena,
        wall,
        &[intent(RED, IntentKind::Lock { contact }, 3)],
        &mut bytes,
    );
    assert_eq!(arena.unit(RED).and_then(|state| state.lock), Some(contact));

    // The target drives out of a five-cell sight — see the module head.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(14, 8);
    }
    // Long enough for the time-to-live and for the loss notice's own hysteresis.
    let mut frames: Vec<(ObjId, Frame)> = Vec::new();
    let until = arena.now_ms + arena.contact_ttl_ms() + 3_000;
    while arena.now_ms < until {
        wall += 500;
        let out = pulse(&mut arena, wall, &[], &mut bytes);
        frames.extend(out.frames);
    }
    assert!(
        arena.contacts.get(RED, contact).is_none(),
        "the contact expired"
    );
    assert_eq!(
        arena.unit(RED).and_then(|state| state.lock),
        None,
        "and the lock went with it"
    );
    // One `lock_lost` notice, once.
    let told: Vec<NoticeKind> = frames
        .iter()
        .filter(|(pilot, _)| *pilot == RED_PILOT)
        .flat_map(|(_, frame)| frame.notices.iter())
        .filter(|notice| notice.contact() == Some(contact))
        .map(tacsim::arena::perception::FrameNotice::kind)
        .collect();
    assert_eq!(
        told.iter()
            .filter(|kind| **kind == NoticeKind::LockLost)
            .count(),
        1,
        "one lock-lost notice, once: {told:?}"
    );
    let any = frames
        .iter()
        .find(|(pilot, _)| *pilot == RED_PILOT)
        .map(|(_, frame)| frame.clone())
        .unwrap_or_else(|| panic!("the red pilot was sent a frame"));
    assert!(
        cockpit::notice_line(
            &any,
            // 16b cut Q.1: a notice names the contact by id and by the short
            // designator its sentence says.
            &tacsim::arena::perception::FrameNotice::Contact {
                kind: NoticeKind::LockLost,
                contact: tacsim::arena::perception::Named {
                    id: contact,
                    designator: tacsim::arena::state::Designator::Provisional(1),
                },
                class: Classification::Unknown,
                was: None,
            },
            &[]
        )
        .contains("Lock on"),
        "and it has a line of its own"
    );
    // A `FIRE` afterwards is refused for want of the lock it used to have.
    assert_eq!(
        arena.resolve_fire(RED, "cannon", arena.now_ms),
        Err(Refusal::NoLock)
    );
}

// ---------------------------------------------------------------------------
// (h) The cross-tempo property, with a shot in it
// ---------------------------------------------------------------------------

/// One entry of a scripted log, keyed by **arena** time, as `arena.rs`'s driver
/// does it: every `t_ms` is a multiple of 5,000 ms, and a pulse boundary at a
/// multiple of 5,000 ms exists at all three cadences, so one script means the same
/// thing in every replay.
struct Beat {
    t_ms: i64,
    intents: Vec<Intent>,
}

/// One ledger line, normalised for comparison: its arena timestamp, its kind and
/// fields as JSON, the draw where one was made, and the substep hash. The `seq` is
/// dropped because it is the one field a different number of pulses could
/// legitimately renumber — `arena.rs`'s own driver does the same.
type Line = (i64, String, Vec<(String, u64)>, u64);

/// What one replay produced: the ledger (normalised), the per-substep hashes, the
/// rounds fired and left, and how many pulses it took.
struct Replay {
    events: Vec<Line>,
    hashes: Vec<u64>,
    rounds_fired: u32,
    rounds_left: u32,
    shots: u64,
    pulses: u32,
    /// **16b cut M.1**: every `Refused` notice the red pilot was sent, as
    /// `(arena ms of the pulse, order word, refusal)`.
    refusals: Vec<(i64, String, Refusal)>,
    /// **16b cut O.3**: the reflex controller's path searches over the whole
    /// replay, off every pulse's `StepOutput::reflex`.
    path_queries: u32,
}

/// **16b cut M.1**: the red pilot's `Refused` notices in one pulse's frames.
fn red_refusals(out: &StepOutput, now_ms: i64, into: &mut Vec<(i64, String, Refusal)>) {
    for (pilot, frame) in &out.frames {
        if *pilot != RED_PILOT {
            continue;
        }
        for notice in &frame.notices {
            if let tacsim::arena::perception::FrameNotice::Refused { order, refusal, .. } = notice {
                into.push((now_ms, order.to_string(), *refusal));
            }
        }
    }
}

/// Drive `arena` through `script` at `cadence_ms` until `until_ms`, with the tempo
/// **pinned** — the property under test is that the cadence decides nothing, and a
/// `tempo` event is a per-pulse artefact of the cadence itself (`arena.rs`'s note).
///
/// # Why a beat is delivered in a pulse of no wall time
///
/// Doc 16 §3.1 puts phase 4 — *resolve newly queued discrete actions* — **after**
/// phase 3 advances the clock, so a discrete action takes effect at the arena time
/// its pulse **ended** at. That is a fact about the phase order and not about the
/// cut: a `FIRE` handed to a 5,000 ms pulse standing at 10,000 ms resolves at 15,000,
/// and the same order handed to a 500 ms pulse resolves at 10,500.
///
/// `arena.rs`'s own script sidesteps it by ordering `STARTUP` at arena time 0, where
/// `dt` is zero at every cadence, and by keeping every later order continuous (phase
/// 2 applies those *before* the clock moves). A script with a shot in it cannot do
/// that, so this driver does the same thing explicitly: a beat is delivered by
/// stepping with the **unchanged** wall reading, which is a pulse of `dt = 0` — no
/// substep, no motion, no due event — and phase 4 then resolves at exactly the arena
/// millisecond the script names. Everything after that (the flight time, the impact's
/// own timestamp, the cycle) is a due event and is rate-independent by construction,
/// which is the property under test.
///
/// **16b cut M.1**: each beat is its own zero-`dt` pulse, so a script may put two
/// beats at one `t_ms` to mean two pulses inside one arena second. No script
/// before that cut had two beats at one time, so none of theirs changed.
fn replay(arena: &mut Arena, cadence_ms: i64, until_ms: i64, script: &[Beat]) -> Replay {
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = 0_i64;
    let mut next = 0_usize;
    let mut pulses = 0_u32;
    let mut ended = false;
    let mut refusals: Vec<(i64, String, Refusal)> = Vec::new();
    let mut path_queries = 0_u32;
    loop {
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        // 16b cut M.1 (`T-82`): each due beat in a zero-`dt` pulse of its own,
        // so two beats at one `t_ms` are two pulses inside one arena second —
        // the case a bound drained once a pulse cannot tell from a rate.
        while let Some(beat) = script.get(next) {
            if ended || beat.t_ms > arena.now_ms {
                break;
            }
            next += 1;
            let out = arena.step(wall, &[] as &[Control], &beat.intents);
            absorb(&out, &mut bytes);
            red_refusals(&out, arena.now_ms, &mut refusals);
            path_queries = path_queries.saturating_add(out.motor.path_queries);
            hashes.extend(arena.substep_hashes.iter().copied());
            pulses += 1;
            ended = out.ended.is_some();
        }
        if ended || arena.now_ms >= until_ms {
            break;
        }
        wall += cadence_ms;
        let out = arena.step(wall, &[] as &[Control], &[]);
        absorb(&out, &mut bytes);
        red_refusals(&out, arena.now_ms, &mut refusals);
        path_queries = path_queries.saturating_add(out.motor.path_queries);
        hashes.extend(arena.substep_hashes.iter().copied());
        pulses += 1;
        if out.ended.is_some() || arena.now_ms >= until_ms {
            break;
        }
    }
    let log = events(arena, &mut bytes);
    Replay {
        events: log
            .iter()
            .map(|event| {
                (
                    event.t_ms,
                    serde_json::to_string(&event.kind).unwrap_or_default(),
                    event.draws.clone(),
                    event.hash,
                )
            })
            .collect(),
        hashes,
        rounds_fired: log
            .iter()
            .filter_map(|event| match &event.kind {
                EventKind::Fired { rounds, .. } => Some(*rounds),
                _ => None,
            })
            .sum(),
        rounds_left: arena
            .unit(RED)
            .map(|state| state.weapon_ammo("cannon"))
            .unwrap_or(0),
        shots: arena.next_shot_seq,
        pulses,
        refusals,
        path_queries,
    }
}

/// Start up, then lock and fire four times, each at a multiple of 5,000 ms — the
/// cannon's cycle is 4,000 ms, so every one of the four is accepted.
fn script() -> Vec<Beat> {
    let contact = contact_of(RED, BLUE);
    let mut beats = vec![Beat {
        t_ms: 0,
        intents: vec![intent(RED, IntentKind::Startup, 1)],
    }];
    for (n, at) in [10_000_i64, 15_000, 20_000, 25_000].into_iter().enumerate() {
        let seq = u64::try_from(n).unwrap_or(0).saturating_mul(2);
        beats.push(Beat {
            t_ms: at,
            intents: vec![
                intent(RED, IntentKind::Lock { contact }, seq + 2),
                intent(
                    RED,
                    IntentKind::Fire {
                        weapon: SmolStr::new("cannon"),
                        slot: None,
                    },
                    seq + 3,
                ),
            ],
        });
    }
    beats
}

/// The two tanks the script fights: stationary, four cells apart, the red one armed.
/// `match.startup_ms` is the default 10,000, so power is online exactly when the
/// first shot is ordered.
fn scripted_arena() -> Arena {
    arena_of(
        vec![
            scout(RED, Team::Red, centre(5, 8), vec![cannon()]),
            scout(BLUE, Team::Blue, centre(9, 8), Vec::new()),
        ],
        &[],
        &MatchConfig::default(),
    )
}

#[test]
fn one_scripted_lock_and_fire_at_three_cadences_is_one_shot_sequence() {
    let mut runs: Vec<(i64, Replay)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = scripted_arena();
        let run = replay(&mut arena, cadence, 40_000, &script());
        assert_eq!(
            arena.now_ms, 40_000,
            "cadence {cadence} ran to a different time"
        );
        assert!(arena.shots.is_empty(), "every shot arrived");
        runs.push((cadence, run));
    }
    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };

    // The run is worth comparing: one lock, four shots, four arrivals, four
    // placeholder misses, and the rounds to show it.
    let kinds: Vec<&str> = first
        .events
        .iter()
        .filter_map(|(_, encoded, _, _)| {
            ["locked", "fired", "impact", "miss"]
                .into_iter()
                .find(|word| encoded.contains(&format!("\"kind\":\"{word}\"")))
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            "locked", "fired", "impact", "miss", "fired", "impact", "miss", "fired", "impact",
            "miss", "fired", "impact", "miss",
        ],
        "the shot lifecycle is out of order: {kinds:?}"
    );
    assert_eq!(first.shots, 4, "four shots were accepted");
    assert_eq!(first.rounds_left, 36, "four rounds of forty spent");

    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events, first.events,
            "the event list differs at {cadence} ms"
        );
        assert_eq!(
            run.hashes,
            first.hashes,
            "the per-substep hash sequence differs at {cadence} ms ({} against {})",
            run.hashes.len(),
            first.hashes.len()
        );
        assert_eq!(run.rounds_left, first.rounds_left);
        assert_eq!(run.shots, first.shots);
        // And the cadence really did differ.
        assert_ne!(run.pulses, first.pulses, "{cadence} ms ran the same pulses");
    }
}

/// One line of [`M1B0_SEAT_LINES`]: the arena millisecond, the kind and its
/// fields as JSON, the draws, and the line's hash — the shape [`Line`] holds.
type PinnedLine = (i64, &'static str, &'static [(&'static str, u64)], u64);

/// **The (h) seat script's ledger, every line with its hash, recorded at
/// `e23694b0`** (17b cut M1b.0; the M1b plan's §1(a) and (c)) — before the cut
/// touched the due loop, off this very replay at 5,000 ms. The line hash is the
/// substep's state hash for a line journalled outside an action run and the
/// run's one lazy hash inside one (`ActionHash`, 16b cut M.3) — the state at
/// the run's first `Arena::event_hash`. The action hash is folded into no
/// state hash, so a change that merely set `ActionHash::Pending` and journalled
/// nothing moves no byte here; one that TOOK the lazy hash earlier than before
/// moves the hash of the next action line of its run wherever the state
/// changed between the two. What this pins is the seat script's bytes (its
/// four shots are the pilot's, journalled in a pulse's action run); the
/// reflex shots are `tests/reflex.rs`' half, and that no trigger opens the
/// action hash for a hull with no slot is asserted where a trigger is built
/// (17b cut M1b.1).
///
/// **The cut's one declared re-bless** (17b cut M3.6: `mind_sealed` and
/// `sortie_closed` shift the folded ledger sequence (`hash.rs:897`) of every
/// match; the mind row's fold gains D-UM-23's answer, `dropped` and the seal
/// digest; the chassis' core consequence words ride the hit lines and the
/// pilot's sentence; every other byte's encoding is unchanged). Re-recorded
/// here from 28 lines to 30: two `mind_sealed` lines after `match_started` (the
/// script runs to 40 s and does not end, so no close; no turret dies), every
/// line's hash re-blessed with them and every other field byte-equal (the diff,
/// ignoring the hash, is the two inserted lines alone).
///
/// **Hash v2** (16c T-104, 2026-10-12): the state hash's fold moved — `Fold`, eight bytes a multiply, the notices by `fold_value` — so every `hash` moved and nothing else did (30 lines to 30): every line's `hash` re-blessed, every other field
/// byte-equal, the draws included (the RNG kept its fold).
const M1B0_SEAT_LINES: &[PinnedLine] = &[
    (
        0,
        r#"{"kind":"match_started"}"#,
        &[],
        16_864_243_904_418_883_163,
    ),
    (
        0,
        r#"{"kind":"mind_sealed","unit":4,"sortie":4,"mind":"Standard Doctrine","source":"102cc7d27eb411c4","card_digest":"1a04f1f74acd80a328e492b698b258f521ba884bd390eaac1ad432fb41d5686b","class":"I","governed":true,"licence":"doctrine"}"#,
        &[],
        16_864_243_904_418_883_163,
    ),
    (
        0,
        r#"{"kind":"mind_sealed","unit":9,"sortie":9,"mind":"Standard Doctrine","source":"102cc7d27eb411c4","card_digest":"1a04f1f74acd80a328e492b698b258f521ba884bd390eaac1ad432fb41d5686b","class":"I","governed":true,"licence":"doctrine"}"#,
        &[],
        16_864_243_904_418_883_163,
    ),
    (
        0,
        r#"{"kind":"startup","unit":4}"#,
        &[],
        132_802_854_331_395_357,
    ),
    (
        189,
        r#"{"kind":"detected","viewer":9,"contact":15474813515948476951,"target":4}"#,
        &[],
        1_767_219_762_276_043_751,
    ),
    (
        482,
        r#"{"kind":"detected","viewer":4,"contact":5852833454602713067,"target":9}"#,
        &[],
        4_580_186_289_168_034_412,
    ),
    (
        3189,
        r#"{"kind":"classified","viewer":9,"contact":15474813515948476951,"target":4,"team":"red"}"#,
        &[("classify:9:4:4", 3_682_179_631_169_270_960)],
        9_993_988_096_693_796_259,
    ),
    (
        3482,
        r#"{"kind":"classified","viewer":4,"contact":5852833454602713067,"target":9,"team":"blue"}"#,
        &[("classify:4:9:4", 2_838_647_583_109_313_249)],
        5_032_493_853_592_318_783,
    ),
    (
        5000,
        r#"{"kind":"position_summary","units":[[4,5500,8500,0,0],[9,9500,8500,0,0]]}"#,
        &[],
        5_105_265_943_172_187_261,
    ),
    (
        10_000,
        r#"{"kind":"power_online","unit":4}"#,
        &[],
        14_716_691_599_043_491_425,
    ),
    (
        10_000,
        r#"{"kind":"position_summary","units":[[4,5500,8500,0,0],[9,9500,8500,0,0]]}"#,
        &[],
        14_716_691_599_043_491_425,
    ),
    (
        10_000,
        r#"{"kind":"locked","unit":4,"contact":5852833454602713067,"target":9}"#,
        &[],
        4_821_439_620_345_010_158,
    ),
    (
        10_000,
        r#"{"kind":"fired","unit":4,"weapon":"cannon","shot_seq":1,"target":9,"range_cm":4000,"rounds":1}"#,
        &[],
        4_821_439_620_345_010_158,
    ),
    (
        10_027,
        r#"{"kind":"impact","shot_seq":1,"unit":4,"target":9}"#,
        &[],
        1_011_889_909_339_419_373,
    ),
    (
        10_027,
        r#"{"kind":"miss","shot_seq":1,"reason":"dispersion"}"#,
        &[("hit:1", 12_072_725_193_963_834_385)],
        1_011_889_909_339_419_373,
    ),
    (
        15_000,
        r#"{"kind":"position_summary","units":[[4,5500,8500,0,0],[9,9500,8500,0,0]]}"#,
        &[],
        9_382_623_964_402_073_433,
    ),
    (
        15_000,
        r#"{"kind":"fired","unit":4,"weapon":"cannon","shot_seq":2,"target":9,"range_cm":4000,"rounds":1}"#,
        &[],
        14_419_275_062_875_186_313,
    ),
    (
        15_027,
        r#"{"kind":"impact","shot_seq":2,"unit":4,"target":9}"#,
        &[],
        10_181_783_062_471_351_146,
    ),
    (
        15_027,
        r#"{"kind":"miss","shot_seq":2,"reason":"dispersion"}"#,
        &[("hit:2", 11_307_540_289_106_727_687)],
        10_181_783_062_471_351_146,
    ),
    (
        20_000,
        r#"{"kind":"position_summary","units":[[4,5500,8500,0,0],[9,9500,8500,0,0]]}"#,
        &[],
        6_428_168_536_042_985_444,
    ),
    (
        20_000,
        r#"{"kind":"fired","unit":4,"weapon":"cannon","shot_seq":3,"target":9,"range_cm":4000,"rounds":1}"#,
        &[],
        3_968_727_613_042_776_565,
    ),
    (
        20_027,
        r#"{"kind":"impact","shot_seq":3,"unit":4,"target":9}"#,
        &[],
        14_700_892_910_747_024_774,
    ),
    (
        20_027,
        r#"{"kind":"miss","shot_seq":3,"reason":"dispersion"}"#,
        &[("hit:3", 17_241_375_169_118_599_602)],
        14_700_892_910_747_024_774,
    ),
    (
        25_000,
        r#"{"kind":"position_summary","units":[[4,5500,8500,0,0],[9,9500,8500,0,0]]}"#,
        &[],
        8_633_050_311_660_299_238,
    ),
    (
        25_000,
        r#"{"kind":"fired","unit":4,"weapon":"cannon","shot_seq":4,"target":9,"range_cm":4000,"rounds":1}"#,
        &[],
        14_226_938_488_314_099_088,
    ),
    (
        25_027,
        r#"{"kind":"impact","shot_seq":4,"unit":4,"target":9}"#,
        &[],
        15_438_213_128_306_917_757,
    ),
    (
        25_027,
        r#"{"kind":"miss","shot_seq":4,"reason":"dispersion"}"#,
        &[("hit:4", 9_783_297_951_381_390_390)],
        15_438_213_128_306_917_757,
    ),
    (
        30_000,
        r#"{"kind":"position_summary","units":[[4,5500,8500,0,0],[9,9500,8500,0,0]]}"#,
        &[],
        8_431_990_535_153_678_772,
    ),
    (
        35_000,
        r#"{"kind":"position_summary","units":[[4,5500,8500,0,0],[9,9500,8500,0,0]]}"#,
        &[],
        9_328_937_775_602_117_459,
    ),
    (
        40_000,
        r#"{"kind":"position_summary","units":[[4,5500,8500,0,0],[9,9500,8500,0,0]]}"#,
        &[],
        16_724_664_096_353_168_875,
    ),
];

/// **17b cut M1b.0: the production line-hash pin, the seat half.** The (h)
/// script — a seat's `STARTUP`, then `LOCK` and `FIRE` four times, the shipped
/// duel's shape with no arming and no mind — journals at 5,000, 1,000 and 500 ms
/// exactly the twenty-eight lines it journalled at `e23694b0`, each with the
/// hash it carried there. Production holds no mind event and no slot, so the
/// two-phase drain of this cut moves nothing here; every later sub-cut of M1b
/// re-asserts this pin, and a move is a defect in that sub-cut, never a
/// re-bless (`tests/motor.rs` holds the shipped duel's half,
/// `tests/reflex.rs` the reflex shots').
#[test]
fn m1b0_the_production_line_hashes_do_not_move() {
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = scripted_arena();
        let run = replay(&mut arena, cadence, 40_000, &script());
        let pinned: Vec<Line> = M1B0_SEAT_LINES
            .iter()
            .map(|(t_ms, kind, draws, hash)| {
                (
                    *t_ms,
                    (*kind).to_owned(),
                    draws
                        .iter()
                        .map(|(label, value)| ((*label).to_owned(), *value))
                        .collect(),
                    *hash,
                )
            })
            .collect();
        assert_eq!(
            run.events, pinned,
            "the seat script's ledger lines moved at {cadence} ms"
        );
    }
}

/// **16b cut M.1** (`T-82`): the order bound is a rate in ARENA seconds, so it
/// refuses the same orders at every cadence.
///
/// Eight `STARTUP`s at arena 0 and a ninth at 5,000 are nine orders in two arena
/// seconds, and none is refused; nine at arena 0 in one pulse are nine in one,
/// and the ninth is refused `queue_full`; and eight at arena 0 then a ninth at
/// arena 0 **in a second pulse** are still nine in one arena second, and the
/// ninth is refused — at 5,000, 1,000 and 500 ms alike, with one event list and
/// one hash sequence, the seat-order window and count being folded into the
/// hash. The third script is the one only a rate passes: before this cut the
/// bound was a queue drained once a pulse, so the second pulse found the queue
/// empty and took the ninth, and what a pilot could send in a burst was a
/// function of the pulse rate.
#[test]
fn m1_the_order_rate_refuses_the_same_orders_at_every_cadence() {
    let startups = |range: std::ops::RangeInclusive<u64>| -> Vec<Intent> {
        range
            .map(|seq| intent(RED, IntentKind::Startup, seq))
            .collect()
    };
    let spread = vec![
        Beat {
            t_ms: 0,
            intents: startups(1..=8),
        },
        Beat {
            t_ms: 5_000,
            intents: startups(9..=9),
        },
    ];
    let burst = vec![Beat {
        t_ms: 0,
        intents: startups(1..=9),
    }];
    let two_pulses = vec![
        Beat {
            t_ms: 0,
            intents: startups(1..=8),
        },
        Beat {
            t_ms: 0,
            intents: startups(9..=9),
        },
    ];
    for (name, script, refused) in [
        ("eight then one", spread, Vec::new()),
        (
            "nine at once",
            burst,
            vec![(0_i64, "startup".to_owned(), Refusal::QueueFull)],
        ),
        (
            "eight then one in the same arena second",
            two_pulses,
            vec![(0_i64, "startup".to_owned(), Refusal::QueueFull)],
        ),
    ] {
        let mut runs: Vec<(i64, Replay)> = Vec::new();
        for cadence in [5_000_i64, 1_000, 500] {
            let mut arena = scripted_arena();
            let run = replay(&mut arena, cadence, 10_000, &script);
            assert_eq!(
                run.refusals, refused,
                "{name}: the refusals at {cadence} ms"
            );
            runs.push((cadence, run));
        }
        let Some((_, first)) = runs.first() else {
            panic!("three replays");
        };
        assert!(
            first
                .events
                .iter()
                .any(|(_, encoded, _, _)| encoded.contains("\"kind\":\"startup\"")),
            "{name}: the startup was journalled"
        );
        for (cadence, run) in runs.iter().skip(1) {
            assert_eq!(
                run.events, first.events,
                "{name}: the event list differs at {cadence} ms"
            );
            assert_eq!(
                run.hashes, first.hashes,
                "{name}: the per-substep hash sequence differs at {cadence} ms"
            );
            assert_ne!(run.pulses, first.pulses, "{cadence} ms ran the same pulses");
        }
    }
}

// ---------------------------------------------------------------------------
// (i) Conservation
// ---------------------------------------------------------------------------

#[test]
fn ammunition_is_conserved_across_a_scripted_run() {
    let mut arena = scripted_arena();
    let load: u32 = arena
        .unit(RED)
        .map(|state| {
            state
                .weapons
                .iter()
                .map(|weapon| weapon.count_per_load)
                .sum()
        })
        .unwrap_or(0);
    assert_eq!(load, 40);
    let run = replay(&mut arena, 1_000, 40_000, &script());
    // Every `fired` line is one pull, and its `rounds` field is what that pull
    // spent, so the sum of them plus what is left is the load — no round is
    // invented and none is lost.
    let spent = run.rounds_fired;
    assert_eq!(spent, 4, "four pulls of one round");
    assert_eq!(
        spent.saturating_add(run.rounds_left),
        load,
        "rounds fired plus rounds left is the load"
    );
    // And the live cells agree with the black box, per weapon.
    let cells: BTreeMap<SmolStr, u32> = arena
        .unit(RED)
        .map(|state| state.ammo.clone())
        .unwrap_or_default();
    assert_eq!(
        cells.get(&SmolStr::new("cannon")).copied(),
        Some(load.saturating_sub(spent))
    );
}

// ---------------------------------------------------------------------------
// (k) The staff place at a beat (16b cut I.3; 16c N10)
// ---------------------------------------------------------------------------

/// A staff place of `unit` on `(x, y)`, stamped as the unit's own seat on
/// `helm` at the seated epoch — what `@arena/place` sends.
fn place(unit: ObjId, x: u16, y: u16, seq: u64) -> Intent {
    intent(unit, IntentKind::Place { x, y }, seq)
}

/// Every `placed` and `place_refused` line of a replay, as `(t_ms, kind)`.
fn place_lines(run: &Replay) -> Vec<(i64, String)> {
    run.events
        .iter()
        .filter(|(_, kind, _, _)| {
            kind.contains("\"kind\":\"placed\"") || kind.contains("\"kind\":\"place_refused\"")
        })
        .map(|(t_ms, kind, _, _)| (*t_ms, kind.clone()))
        .collect()
}

/// **16b cut I.3's mandatory cross-tempo test, the place** (16b §11; D-16-2):
/// a scripted `Place` of the target at a beat is one `placed` line at the
/// beat's own millisecond and one pose — the cell's centre — at 5,000, 1,000
/// and 500 ms, with one event list and one per-substep hash sequence. The red
/// tank then locks and fires on the moved tank from its own sensor's next track
/// of it, and that is the same at every cadence too: the place moved the unit,
/// the next pass moved the track, and the contact kept its id.
#[test]
fn i3_a_place_at_a_beat_is_one_line_and_one_pose_at_three_cadences() {
    let contact = contact_of(RED, BLUE);
    let script = vec![
        Beat {
            t_ms: 0,
            intents: vec![intent(RED, IntentKind::Startup, 1)],
        },
        Beat {
            t_ms: 10_000,
            intents: vec![place(BLUE, 8, 11, 2)],
        },
        Beat {
            t_ms: 15_000,
            intents: vec![
                intent(RED, IntentKind::Lock { contact }, 3),
                intent(
                    RED,
                    IntentKind::Fire {
                        weapon: SmolStr::new("cannon"),
                        slot: None,
                    },
                    4,
                ),
            ],
        },
    ];
    let mut runs: Vec<(i64, Replay)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = scripted_arena();
        let run = replay(&mut arena, cadence, 25_000, &script);
        assert_eq!(
            arena
                .unit(BLUE)
                .map(|state| (state.pose.x_cm, state.pose.y_cm)),
            Some((8_500, 11_500)),
            "cadence {cadence}: the pose is the cell's centre"
        );
        assert_eq!(arena.index.occupant(8, 11), Some(BLUE));
        assert!(
            arena.contacts.get(RED, contact).is_some(),
            "cadence {cadence}: the red tank still holds the blue one under its id"
        );
        runs.push((cadence, run));
    }
    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };
    assert_eq!(
        place_lines(first),
        vec![(
            10_000,
            "{\"kind\":\"placed\",\"unit\":9,\"x\":8,\"y\":11,\"from_x\":9,\"from_y\":8}"
                .to_owned()
        )]
    );
    assert_eq!(first.shots, 1, "the moved tank was shot at");
    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events, first.events,
            "the event list differs at {cadence} ms"
        );
        assert_eq!(
            run.hashes,
            first.hashes,
            "the per-substep hash sequence differs at {cadence} ms ({} against {})",
            run.hashes.len(),
            first.hashes.len()
        );
        assert_ne!(run.pulses, first.pulses, "{cadence} ms ran the same pulses");
    }
}

/// **16b cut I.3's second cross-tempo test: a round in flight refuses a place
/// at either end of it** (16c N10; the plan's review A-R3), at 5,000, 1,000 and
/// 500 ms, in two cases — the TARGET placed, and the SHOOTER placed, whose pose
/// the impact traces its line from.
///
/// `FIRE` and the place share ONE beat on purpose. A beat is delivered in the
/// first pulse at or after its `t_ms`, and the cannon's round is in the air for
/// under thirty milliseconds at four cells, so a place scripted even a hundred
/// milliseconds after the shot reaches the arena at the next pulse — 15,000 at a
/// 5,000 ms cadence, 10,500 at 500 — after the impact at every cadence, and
/// there is nothing in flight to refuse it. In one beat, phase 4 drains the
/// shooter's `LOCK` and `FIRE` first (the shooter's id is below the target's,
/// and a unit's own queue is in the order it was given), so the round is in
/// `shots` when the place resolves.
///
/// Each case: one `place_refused` line with `in_flight` at the beat's
/// millisecond, the placed unit's pose untouched, the shot's `fired`, `impact`
/// and `miss` exactly where they fall with no place at all, and one event list
/// and one hash sequence across the three cadences.
#[test]
fn i3_a_place_under_a_round_in_flight_is_refused_at_three_cadences() {
    let contact = contact_of(RED, BLUE);
    let shot = |extra: Option<Intent>| -> Vec<Beat> {
        let mut intents = vec![
            intent(RED, IntentKind::Lock { contact }, 2),
            intent(
                RED,
                IntentKind::Fire {
                    weapon: SmolStr::new("cannon"),
                    slot: None,
                },
                3,
            ),
        ];
        intents.extend(extra);
        vec![
            Beat {
                t_ms: 0,
                intents: vec![intent(RED, IntentKind::Startup, 1)],
            },
            Beat {
                t_ms: 10_000,
                intents,
            },
        ]
    };
    // What the shot does with no place at all: its lines, without their hash
    // (a `place_refused` line moves the ledger's sequence, which every later
    // hash folds).
    let shot_lines = |run: &Replay| -> Vec<(i64, String)> {
        run.events
            .iter()
            .filter(|(_, kind, _, _)| {
                ["fired", "impact", "miss", "hit"]
                    .iter()
                    .any(|word| kind.contains(&format!("\"kind\":\"{word}\"")))
            })
            .map(|(t_ms, kind, _, _)| (*t_ms, kind.clone()))
            .collect()
    };
    let baseline = {
        let mut arena = scripted_arena();
        replay(&mut arena, 1_000, 20_000, &shot(None))
    };
    assert_eq!(baseline.shots, 1);
    let unplaced = shot_lines(&baseline);
    assert!(
        unplaced
            .iter()
            .any(|(_, kind)| kind.contains("\"kind\":\"impact\"")),
        "the round arrived: {unplaced:?}"
    );
    for (who, cell, home) in [
        (BLUE, (9_u16, 12_u16), (9_500_i64, 8_500_i64)),
        (RED, (5, 12), (5_500, 8_500)),
    ] {
        let script = shot(Some(place(who, cell.0, cell.1, 4)));
        let mut runs: Vec<(i64, Replay)> = Vec::new();
        for cadence in [5_000_i64, 1_000, 500] {
            let mut arena = scripted_arena();
            let run = replay(&mut arena, cadence, 20_000, &script);
            assert_eq!(
                arena
                    .unit(who)
                    .map(|state| (state.pose.x_cm, state.pose.y_cm)),
                Some(home),
                "#{} moved at {cadence} ms",
                who.0
            );
            assert!(
                run.refusals.is_empty(),
                "a place is never said: {:?}",
                run.refusals
            );
            runs.push((cadence, run));
        }
        let Some((_, first)) = runs.first() else {
            panic!("three replays");
        };
        assert_eq!(
            place_lines(first),
            vec![(
                10_000,
                format!(
                    "{{\"kind\":\"place_refused\",\"unit\":{},\"x\":{},\"y\":{},\"reason\":\"in_flight\"}}",
                    who.0, cell.0, cell.1
                )
            )],
            "#{}",
            who.0
        );
        assert_eq!(
            shot_lines(first),
            unplaced,
            "#{}: the round fell where it falls with no place",
            who.0
        );
        for (cadence, run) in runs.iter().skip(1) {
            assert_eq!(
                run.events, first.events,
                "#{}: the event list differs at {cadence} ms",
                who.0
            );
            assert_eq!(
                run.hashes, first.hashes,
                "#{}: the per-substep hash sequence differs at {cadence} ms",
                who.0
            );
        }
    }
}

/// **16b cut I.3: a beam fired in a floored pulse of no time lands in it**, as
/// the same `FIRE` at the same arena millisecond does in a pulse that ran a
/// substep.
///
/// Under a staff floor a production pulse can run no substep at all — the
/// deadline pulled in on a set, a lift or an abort, or any wake inside a
/// substep, cut back to the lattice point it began on — and the phase-4 drain
/// used to run only on a pulse that ran one. A beam `FIRE` resolved in such a
/// pulse then waited out the floor and landed in the next wake's first substep,
/// against positions a substep later: the outcome turned on the wall moment
/// staff typed the verb. Two runs under one 1,000 ms floor, the red tank firing
/// its laser at arena 1,500 both times: once in the pulse that reaches 1,500,
/// once in a pulse of no time a hundred wall milliseconds after it. The round
/// is resolved before the pulse returns in both, its `impact` at the `fired`
/// millisecond, and every line of the two tapes but the tempo's own is the same.
#[test]
fn i3_a_beam_fired_in_a_floored_pulse_of_no_time_lands_in_it() {
    let beam = WeaponSummary {
        id: SmolStr::new("laser"),
        muzzle_speed_cms: 0,
        ..cannon()
    };
    let contact = contact_of(RED, BLUE);
    let fire = [
        intent(RED, IntentKind::Lock { contact }, 3),
        intent(
            RED,
            IntentKind::Fire {
                weapon: SmolStr::new("laser"),
                slot: None,
            },
            4,
        ),
    ];
    let mut tapes = Vec::new();
    for no_time in [false, true] {
        let mut arena = arena_of(
            vec![
                scout(RED, Team::Red, centre(5, 8), vec![beam.clone()]),
                scout(BLUE, Team::Blue, centre(9, 8), Vec::new()),
            ],
            &[],
            &match_cfg(),
        );
        let mut bytes: Vec<u8> = Vec::new();
        let wall = warm(&mut arena, &mut bytes);
        assert_eq!(arena.now_ms, 1_000);
        arena.clock.set_floor(Some(1_000));
        if no_time {
            pulse(&mut arena, wall + 500, &[], &mut bytes);
            assert_eq!(arena.now_ms, 1_500, "the floored wake ends on the lattice");
            let out = pulse(&mut arena, wall + 600, &fire, &mut bytes);
            assert_eq!(arena.now_ms, 1_500, "a wake inside a substep moves no time");
            assert_eq!(out.substeps, 1, "the phase-4 drain, and nothing else");
        } else {
            let out = pulse(&mut arena, wall + 500, &fire, &mut bytes);
            assert_eq!(arena.now_ms, 1_500);
            assert_eq!(out.substeps, 3, "two substeps and the phase-4 drain");
        }
        assert!(
            arena.shot(1).is_none(),
            "no_time {no_time}: the beam was left in flight past its pulse"
        );
        let log = events(&mut arena, &mut bytes);
        let fired = log
            .iter()
            .find(|event| matches!(event.kind, EventKind::Fired { shot_seq: 1, .. }))
            .map(|event| event.t_ms);
        let impact = log
            .iter()
            .find(|event| matches!(event.kind, EventKind::Impact { shot_seq: 1, .. }))
            .map(|event| event.t_ms);
        assert_eq!(fired, Some(1_500), "no_time {no_time}");
        assert_eq!(
            impact, fired,
            "no_time {no_time}: a beam has no flight time"
        );
        assert_eq!(count(&log, "lagged"), 0, "no_time {no_time}");
        // Each line as it reads, less its place in the ledger and the hash
        // that place moves (`hash.rs` item 11): the tempo lines fall in a
        // different order in the two runs.
        let tape: Vec<LedgerEvent> = log
            .into_iter()
            .filter(|event| !matches!(event.kind.as_str(), "tempo" | "tempo_floor"))
            .map(|event| LedgerEvent {
                seq: 0,
                hash: 0,
                ..event
            })
            .collect();
        tapes.push(tape);
    }
    assert_eq!(
        tapes.first(),
        tapes.get(1),
        "the pulse the beam was fired in decided its outcome"
    );
}

// ---------------------------------------------------------------------------
// (j) A lock survives the rename (16b cut Q.6; the owner's ruling of 2026-09-20)
// ---------------------------------------------------------------------------

/// **A lock taken on a provisional number is the same lock after the contact is
/// told apart** (16b cut Q.6; doc 16 §3.5, §3.6, W5).
///
/// The owner's ruling on cut Q's short designators: *"new contact X1 until
/// identified is fine, but if someone locks X1 and it becomes BA then their lock
/// should continue, since it is the same contact even though the letters
/// changed."* Cut Q.1 built it that way by construction — the arena only ever
/// holds [`IntentKind::Lock`] with a [`ContactId`], and the designator is a
/// rendering of the row rather than a handle on it — but nothing said so end to
/// end, and a rename that silently dropped a lock is exactly the kind of thing a
/// pilot finds out about when his `FIRE` is refused.
///
/// So this drives the whole transition through `Arena::step`: the lock is taken
/// while the return is `X1`, the classification clock runs out, and afterwards
/// the **same** `ContactId` is locked, the frame renders it `BA`, the only line
/// the rename produced is the one that announces the new name, and a `FIRE` with
/// no re-lock is accepted against the same object.
///
/// The counter-proof is in the same test: no `lock_lost` and no `lost` line
/// anywhere, in the ledger or in the pilot's own notices. Those are the two
/// events that *would* break a lock (16b cut W.1 clears it on the `lost`), and
/// the claim is precisely that a rename is neither of them.
#[test]
fn a_lock_taken_on_a_provisional_number_survives_the_contact_being_told_apart() {
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    let contact = contact_of(RED, BLUE);

    // (1) One second in, against a `classify_after_ms` of three: the return is
    //     held and not told apart, so its only name is this platform's own
    //     provisional number.
    assert_eq!(
        arena.contacts.get(RED, contact).map(|held| held.class),
        Some(Classification::Unknown),
        "the return is still provisional at one second"
    );

    // (2) The lock, taken on `X1`.
    let out = pulse(
        &mut arena,
        wall + 500,
        &[intent(RED, IntentKind::Lock { contact }, 3)],
        &mut bytes,
    );
    assert_eq!(
        arena.unit(RED).and_then(|state| state.lock),
        Some(contact),
        "the lock landed"
    );
    let locked_frame = out
        .frames
        .iter()
        .find(|(pilot, _)| *pilot == RED_PILOT)
        .map(|(_, frame)| frame.clone())
        .expect("taking a lock changes the gunner's view");
    assert_eq!(
        cockpit::lock_designator(&locked_frame),
        Some(Designator::Provisional(1)),
        "the lock is rendered under the number the platform gave the return"
    );
    assert_eq!(
        cockpit::lock_designator(&locked_frame).map(|named| named.to_string()),
        Some("X1".to_owned())
    );

    // (3) Past `classify_after_ms`. Everything the gunner is told from here on is
    //     the rename's doing: the tanks are stationary, loaded and online.
    let frames = run(&mut arena, 500, 5_000, &mut bytes);
    let told: Vec<&'static str> = frames
        .iter()
        .filter(|(pilot, _)| *pilot == RED_PILOT)
        .flat_map(|(_, frame)| frame.notices.iter().map(|notice| notice.kind().as_str()))
        .collect();
    assert_eq!(
        told,
        vec!["classified"],
        "the rename said one thing, and it was the new name"
    );

    // (4) The lock is the same lock, on the same opaque id, and the platform's
    //     row for it now carries the other side's slot designator.
    assert_eq!(
        arena.unit(RED).and_then(|state| state.lock),
        Some(contact),
        "the rename dropped the lock"
    );
    assert_eq!(
        arena.contacts.get(RED, contact).map(|held| held.class),
        Some(Classification::Classified { team: Team::Blue }),
        "the return was never told apart"
    );
    let named_frame = frames
        .iter()
        .rev()
        .find(|(pilot, _)| *pilot == RED_PILOT)
        .map(|(_, frame)| frame.clone())
        .expect("the classification changes the gunner's view");
    let renamed = cockpit::lock_designator(&named_frame).expect("the lock is still on a held row");
    assert_eq!(
        renamed,
        Designator::Slot {
            team: Team::Blue,
            slot: 1
        },
        "the lock now renders as the target's own callsign"
    );
    assert_eq!(renamed.to_string(), "BA");

    // (5) Neither of the two events that break a lock happened — in the black box
    //     or in the pilot's lines.
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "lost"), 0, "nothing was lost");
    assert_eq!(count(&log, "locked"), 1, "one lock, taken once");
    assert_eq!(
        count(&log, "classified"),
        2,
        "each platform told the other apart once"
    );
    assert!(
        !told.contains(&"lock_lost") && !told.contains(&"lost"),
        "the rename broke the lock: {told:?}"
    );

    // (5b) And the order **repeated** on the renamed row is not a second lock.
    //      This is the arena half of what a pilot does at the cockpit when he
    //      types `LOCK BA` on a contact he already locked as `X1`:
    //      `Arena::resolve_lock` answers `Ok` for a lock already held without
    //      touching the state or the black box, so the ledger still holds one
    //      `locked` line. The gunner **is** answered — phase 4 stages the notice
    //      on every accepted order, which is where the hit chance comes from
    //      (16b cut F.2) — so the line is the order's own reply and the ledger is
    //      what says whether anything moved.
    let repeat_at = arena.now_ms + 500;
    let again = pulse(
        &mut arena,
        repeat_at,
        &[intent(RED, IntentKind::Lock { contact }, 4)],
        &mut bytes,
    );
    assert_eq!(
        arena.unit(RED).and_then(|state| state.lock),
        Some(contact),
        "the repeat moved the lock"
    );
    let answered: Vec<&'static str> = again
        .frames
        .iter()
        .filter(|(pilot, _)| *pilot == RED_PILOT)
        .flat_map(|(_, frame)| frame.notices.iter().map(|notice| notice.kind().as_str()))
        .collect();
    assert_eq!(
        answered,
        vec!["locked"],
        "a repeat answers with the lock and says nothing else"
    );
    let log = events(&mut arena, &mut bytes);
    assert_eq!(
        count(&log, "locked"),
        1,
        "the repeat journalled a second lock"
    );
    assert_eq!(count(&log, "lock_lost"), 0, "and dropped the first");

    // (6) And a `FIRE` with no re-lock is accepted, against the same object.
    let before = arena.unit(RED).map(|state| state.weapon_ammo("cannon"));
    let trigger_at = arena.now_ms + 500;
    pulse(
        &mut arena,
        trigger_at,
        &[intent(
            RED,
            IntentKind::Fire {
                weapon: SmolStr::new("cannon"),
                slot: None,
            },
            5,
        )],
        &mut bytes,
    );
    assert_eq!(
        arena.unit(RED).map(|state| state.weapon_ammo("cannon")),
        before.map(|rounds| rounds.saturating_sub(1)),
        "the shot cost its round"
    );
    let shot = arena
        .shots
        .values()
        .next()
        .cloned()
        .expect("one round in flight");
    assert_eq!(shot.contact, contact, "fired on the lock it already held");
    assert_eq!(shot.target, BLUE, "and it resolved against the same tank");
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "fired"), 1, "one pull, with no second lock");
    assert_eq!(count(&log, "locked"), 1, "and no lock was re-taken");
}

// ---------------------------------------------------------------------------
// (l) 16b cut O.3 — the range ends when the last target is out of action
// ---------------------------------------------------------------------------

/// The first shooter, on the second target.
const RED2: ObjId = ObjId(5);
/// The target the first shooter faces head on.
const TARGET_FRONT: ObjId = ObjId(11);
/// The target the second shooter stands behind.
const TARGET_REAR: ObjId = ObjId(12);

/// **One fielded unit, checked out as `match_::start` checks one out**: the M32
/// unit off the fielded catalog's own binding, and every row the arena reads
/// decoded by the production functions (`custody_unit`).
fn fielded_unit(
    catalog: &tacsim::definitions::Catalog,
    entry: &str,
    id: ObjId,
    team: Team,
    pose: Pose,
    controller: tacsim::arena::state::Controller,
    slot: u8,
) -> CustodyUnit {
    let key = mush_core::CanonicalId::new(entry).expect("a canonical entry id");
    let spec = catalog.specs.get(&key).expect("the unit entry");
    let binding = catalog.bindings.get(&key).expect("the unit binding");
    let cells = tacsim::definitions::to_tactical_unit(spec, binding, id).expect("the unit");
    tacsim::match_::custody_unit(
        catalog,
        cells,
        id,
        team,
        pose,
        0,
        controller,
        slot,
        &tacsim::match_::SealTerms::fixture(),
    )
    .expect("the fixture terms seal")
}

/// The cell centre of `(x, y)`, facing `heading_mdeg`.
fn facing(x: u16, y: u16, heading_mdeg: u32) -> Pose {
    Pose {
        heading_mdeg,
        ..centre(x, y)
    }
}

/// **A range, as `SIM RANGE 2` fields one** (16b cut O.3; 16c N13): two stock
/// tanks, seated, on red, and two `target_mk1`s with nobody in them on blue —
/// so a unit mind's, as `match_::start` checks out any unseated unit since 17b
/// cut M4.3 — checked out at [`tacsim::arena::MatchMode::Range`]. Red 1 stands
/// four cells in front of the first target, which faces it; red 2 stands four
/// cells behind the second, which faces away — the rear band, hull 40, engine
/// 30, turret 30.
///
/// **17b cut M4.6b — on the unit mind** (the owner's STOP B ruling of
/// 2026-10-09, "power up, then park"; the reflex controller flew it from 17b
/// cut M4.5a until then). A target's mind reads its view mobile while cold,
/// so its first think starts it up; once online the view reads it `immobile`
/// (no authored speed) with no weapon rows — the stock mind's `Disabled` edge
/// — and it parks: no goal, no search, no turn, so the rear target keeps its
/// rear to its shooter, which is this field's premise. Until this cut the
/// view's `immobile` was `limits.motion_disabled` alone and 17b cut M4.5a held
/// this field on the reflex, reading that a target's mind would search a
/// graph and turn in place (30,000 mdeg/s); on this field — no waypoint graph,
/// and a target with no sight has no contact to turn to — the claims hold
/// with or without the view's STOP B reading (17b cut M4.6b checked both).
fn range_arena() -> Arena {
    range_field(
        tacsim::definitions::content::TARGET_MK1,
        tacsim::arena::state::Controller::Mind,
    )
}

/// [`range_arena`]'s field with `blue` standing on the targets' two pods — the
/// target itself, or (for the `spent` contrast) an armed stock tank — checked
/// out under `controller` (a unit mind for every user since 17b cut M4.6b;
/// 17b cut M4.5a held [`range_arena`] on the reflex for STOP B until then).
fn range_field(blue: &str, controller: tacsim::arena::state::Controller) -> Arena {
    use tacsim::arena::state::Controller;
    let catalog = tacsim::definitions::Catalog::build_fielded(&[], tacsim::NS)
        .expect("the fielded content binds");
    let line_tank = tacsim::definitions::content::LINE_TANK;
    let target = blue;
    let units = vec![
        fielded_unit(
            &catalog,
            line_tank,
            RED,
            Team::Red,
            facing(6, 4, 90_000),
            Controller::Pilot,
            1,
        ),
        fielded_unit(
            &catalog,
            line_tank,
            RED2,
            Team::Red,
            facing(6, 10, 90_000),
            Controller::Pilot,
            2,
        ),
        fielded_unit(
            &catalog,
            target,
            TARGET_FRONT,
            Team::Blue,
            facing(10, 4, 270_000),
            controller,
            1,
        ),
        fielded_unit(
            &catalog,
            target,
            TARGET_REAR,
            Team::Blue,
            facing(10, 10, 90_000),
            controller,
            2,
        ),
    ];
    let pilots = [RED, RED2]
        .into_iter()
        .enumerate()
        .map(|(seat, unit)| Pilot {
            unit,
            pilot: ObjId(1_000 + u32::try_from(seat).unwrap_or(0)),
            station: SmolStr::new("cockpit"),
            epoch: 1,
        })
        .collect();
    Arena::new(
        CustodyOut {
            definition: definition(&[]),
            units,
            pilots,
            resolution: tacsim::match_::resolution_for(&catalog),
            seed: SEED,
            mode: tacsim::arena::MatchMode::Range,
        },
        &ArenaConfig::default(),
        &MatchConfig::default(),
        &SensorsConfig::default(),
    )
    .expect("the range")
}

/// Both shooters start up at 0 and, from 15,000 (`match.startup_ms` is 10,000
/// and the first sensor passes and the classification are behind them), lock
/// and fire on their own target at every 10,000 ms beat — the cannon cycles in
/// 6,000, so every shot the script orders is ready. The beats run long past the
/// end: once the last target is out of action the match ends, and the replay
/// stops at the pulse that ended it at every cadence.
fn range_script() -> Vec<Beat> {
    let cannon = || IntentKind::Fire {
        weapon: SmolStr::new("cannon"),
        slot: None,
    };
    let mut beats = vec![Beat {
        t_ms: 0,
        intents: vec![
            intent(RED, IntentKind::Startup, 1),
            intent(RED2, IntentKind::Startup, 1),
        ],
    }];
    for n in 0..30_u64 {
        let at = 15_000 + i64::try_from(n).unwrap_or(0) * 10_000;
        let seq = n * 2 + 2;
        beats.push(Beat {
            t_ms: at,
            intents: vec![
                intent(
                    RED,
                    IntentKind::Lock {
                        contact: contact_of(RED, TARGET_FRONT),
                    },
                    seq,
                ),
                intent(RED, cannon(), seq + 1),
                intent(
                    RED2,
                    IntentKind::Lock {
                        contact: contact_of(RED2, TARGET_REAR),
                    },
                    seq,
                ),
                intent(RED2, cannon(), seq + 1),
            ],
        });
    }
    beats
}

/// **A range ends when its last target is out of action, at every cadence** (16b
/// cut O.3; 16c N13; doc 16 §3.8, 16b §11's mandatory cross-tempo test).
///
/// The range adds no rule of its own: it is fought under the ordinary
/// elimination rule, and a target is out of action as any unit is — a wreck, or
/// disabled with no weapons. So this drives two real stock tanks against two
/// real targets off the fielded catalog, one target shot head on until its hull
/// goes (a wreck) and one shot from behind until its engine goes (disabled: no
/// gun and now no motion), and asserts one event list and one per-substep hash
/// sequence at 5,000, 1,000 and 500 ms; exactly one `match_ended`, an
/// elimination with red the winner; never `spent` (a target is not armed at
/// check-out, `Arena::spent`); and no path search at all from the targets, which
/// are the only machines on the field (the inert rule: on the reflex until 17b
/// cut M4.6b, `reflex.rs`; since, a target's unit mind, which starts up and
/// then parks on Standard Doctrine's `Disabled` edge once its view reads it
/// `immobile` — the owner's STOP B ruling). Every claim is the reflex
/// fixture's, unchanged; the targets are minds through [`range_arena`].
#[test]
fn o3_a_range_ends_when_the_last_target_is_out_of_action_at_three_cadences() {
    let mut runs: Vec<(i64, Replay)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = range_arena();
        let run = replay(&mut arena, cadence, 400_000, &range_script());
        assert!(
            arena.ended.is_some(),
            "cadence {cadence}: the range did not end by {}",
            arena.now_ms
        );
        runs.push((cadence, run));
    }
    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };
    let lines = |kind: &str| -> Vec<&Line> {
        first
            .events
            .iter()
            .filter(|(_, encoded, _, _)| encoded.contains(&format!("\"kind\":\"{kind}\"")))
            .collect()
    };
    let ended = lines("match_ended");
    assert_eq!(ended.len(), 1, "exactly one end: {ended:?}");
    assert!(
        ended
            .iter()
            .all(|(_, encoded, _, _)| encoded.contains(r#"{"elimination":{"winner":"red"}}"#)),
        "the range ends by elimination, red the winner: {ended:?}"
    );
    assert!(
        first
            .events
            .iter()
            .all(|(_, encoded, _, _)| !encoded.contains("\"spent\"")),
        "a range is never `spent`"
    );
    // One target wrecked, the other disabled — and the disabled one by its engine,
    // the one section behind the hull's rear facet.
    let destroyed = lines("unit_destroyed");
    let disabled = lines("unit_disabled");
    assert!(
        destroyed
            .iter()
            .any(|(_, encoded, _, _)| encoded.contains(&format!("\"unit\":{}", TARGET_FRONT.0))),
        "the target shot head on is a wreck: {destroyed:?}"
    );
    assert!(
        disabled
            .iter()
            .any(|(_, encoded, _, _)| encoded.contains(&format!("\"unit\":{}", TARGET_REAR.0))),
        "the target shot from behind is disabled: {disabled:?}"
    );
    assert!(
        lines("section_destroyed").iter().any(|(_, encoded, _, _)| {
            encoded.contains(&format!("\"unit\":{}", TARGET_REAR.0))
                && encoded.contains("\"section\":\"engine\"")
        }),
        "and it is its engine that went"
    );
    assert_eq!(first.path_queries, 0, "a target searches for nothing");
    // **17b cut M3.6** (doc 17 §13; D-UM-45, D-UM-46): the sortie lines on a
    // REAL damage path. Each `unit_destroyed` — a round's wreck, through
    // `apply_hit`, `apply_consequence` and `destroy_unit` — is followed at
    // once by that unit's `sortie_closed { destroyed }` at the destruction's
    // millisecond under its hash, and the wreck closes once; every other unit
    // closes `match_end` immediately before the end's line, under the end's
    // millisecond and hash, and the end stays the tape's last line. The event
    // lists are one list at the three cadences (below), so this holds at each.
    let unit_of = |encoded: &str| -> Option<u64> {
        serde_json::from_str::<serde_json::Value>(encoded)
            .ok()?
            .get("unit")?
            .as_u64()
    };
    let is = |line: &Line, kind: &str| line.1.contains(&format!("\"kind\":\"{kind}\""));
    let units = lines("mind_sealed").len();
    assert_eq!(units, 4, "one seal per unit at the opening");
    let mut wrecked: Vec<u64> = Vec::new();
    for (at, line) in first.events.iter().enumerate() {
        if !is(line, "unit_destroyed") {
            continue;
        }
        let unit = unit_of(&line.1).unwrap_or_else(|| panic!("a unit on {line:?}"));
        let Some(close) = first.events.get(at + 1) else {
            panic!("a line after the destruction of #{unit}");
        };
        assert!(
            is(close, "sortie_closed")
                && close.1.contains("\"word\":\"destroyed\"")
                && unit_of(&close.1) == Some(unit),
            "#{unit}'s destruction is followed by its own close: {close:?}"
        );
        assert_eq!(
            (close.0, close.3),
            (line.0, line.3),
            "#{unit}'s close at the destruction's millisecond under its hash"
        );
        wrecked.push(unit);
    }
    assert!(
        wrecked.contains(&u64::from(TARGET_FRONT.0)),
        "the target shot head on closed at its wreck: {wrecked:?}"
    );
    let closes = lines("sortie_closed");
    assert_eq!(closes.len(), units, "one close per sortie: {closes:?}");
    for unit in &wrecked {
        assert_eq!(
            closes
                .iter()
                .filter(|close| unit_of(&close.1) == Some(*unit))
                .count(),
            1,
            "#{unit} is not closed again at the end"
        );
    }
    let at_end = units - wrecked.len();
    let Some(end) = first.events.last() else {
        panic!("a tape");
    };
    assert!(is(end, "match_ended"), "the end is the last line: {end:?}");
    let tail: Vec<&Line> = first.events.iter().rev().skip(1).take(at_end).collect();
    assert_eq!(tail.len(), at_end);
    assert!(
        tail.iter().all(|close| is(close, "sortie_closed")
            && close.1.contains("\"word\":\"match_end\"")
            && (close.0, close.3) == (end.0, end.3)),
        "the open sorties close right before the end, at its millisecond under its hash: \
         {tail:?}"
    );

    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events, first.events,
            "the event list differs at {cadence} ms"
        );
        assert_eq!(
            run.hashes,
            first.hashes,
            "the per-substep hash sequence differs at {cadence} ms ({} against {})",
            run.hashes.len(),
            first.hashes.len()
        );
        assert_eq!(run.shots, first.shots);
        assert_eq!(run.path_queries, 0);
        assert_ne!(run.pulses, first.pulses, "{cadence} ms ran the same pulses");
    }
}

/// **Running dry never ends a range** (16b cut O.3; 16c N13; the help's
/// "running dry does not end it"), and the same dry shooters against armed
/// tanks would.
///
/// `Arena::spent` ends a match only when **both** sides came into it armed
/// (`UnitState::armed_at_checkout`) and nothing in action has a round left. A
/// target never carried a gun, so a range whose shooters have emptied every
/// weapon is not a dry field: it runs on until `SIM ABORT` or the last target
/// is out of action. The contrast — the same field with two stock tanks on the
/// targets' pods, every weapon on both sides emptied — ends `spent` at once,
/// so the first half is a claim about the targets and not about a field that
/// was never dry.
///
/// **17b cut M4.5a**: the targets — and the contrast's two stock tanks — are
/// checked out `Controller::Mind`, as `match_::start` checks out every
/// unseated hull since 17b cut M4.3, where they were the reflex controller's.
/// Every claim is the same: `spent` and the elimination read the field's arms
/// and condition, not its controller, so what a target's mind does — since
/// 17b cut M4.6b (the owner's STOP B ruling) start up and then park, until
/// then start up and turn in place — changes nothing asserted.
#[test]
fn o3_a_range_whose_shooters_run_dry_is_not_spent() {
    let startup = vec![Beat {
        t_ms: 0,
        intents: vec![
            intent(RED, IntentKind::Startup, 1),
            intent(RED2, IntentKind::Startup, 1),
        ],
    }];
    let empty = |arena: &mut Arena| {
        for unit in &mut arena.units {
            for rounds in unit.ammo.values_mut() {
                *rounds = 0;
            }
        }
    };

    // The range: red dry, the targets never armed.
    let mut arena = range_field(
        tacsim::definitions::content::TARGET_MK1,
        tacsim::arena::state::Controller::Mind,
    );
    empty(&mut arena);
    assert!(
        arena
            .units
            .iter()
            .filter(|unit| unit.team == Team::Red)
            .all(|unit| unit.armed_at_checkout() && !unit.ammo.is_empty()),
        "the shooters came in armed"
    );
    let run = replay(&mut arena, 1_000, 60_000, &startup);
    assert!(!arena.spent(), "a range with dry shooters read as spent");
    assert_eq!(arena.settle_spent(), None);
    assert_eq!(arena.ended, None, "the range ended by {}", arena.now_ms);
    assert!(arena.now_ms >= 60_000, "the replay ran its minute");
    assert!(
        run.events
            .iter()
            .all(|(_, encoded, _, _)| !encoded.contains("\"kind\":\"match_ended\"")),
        "nothing ended the range"
    );
    for target in [TARGET_FRONT, TARGET_REAR] {
        assert!(!arena.out_of_action(target), "{target:?} is still standing");
    }

    // The contrast: armed tanks on the targets' pods, and every weapon dry.
    let mut arena = range_field(
        tacsim::definitions::content::LINE_TANK,
        tacsim::arena::state::Controller::Mind,
    );
    empty(&mut arena);
    assert!(arena.spent(), "two dry armed sides are a dry field");
    let _ = replay(&mut arena, 1_000, 60_000, &startup);
    assert_eq!(
        arena.ended.map(|end| end.reason),
        Some(tacsim::arena::EndReason::Spent),
        "the contrast is a spent draw"
    );
}

// ---------------------------------------------------------------------------
// 16b cut AP.2 — a disarming hit ends the match at its own substep
// ---------------------------------------------------------------------------

/// **Two stock tanks, head on, both seated** (16b cut AP.2): red at `(6, 4)`
/// facing east, blue four cells east of it facing west, each checked out off
/// the fielded catalog as `match_::start` checks one out — so both came in
/// armed (`UnitState::armed_at_checkout`). Blue's pilot never starts up.
///
/// Blue's **front band** is narrowed to its turret alone: the stock band splits
/// a round between the hull and the turret by the seeded facet draw, and this
/// fixture is about the turret's `remove_weapons`, so every round that hits
/// finds the turret and the fifth one through it takes blue's only gun.
fn ap2_duel() -> Arena {
    use tacsim::arena::state::Controller;
    let catalog = tacsim::definitions::Catalog::build_fielded(&[], tacsim::NS)
        .expect("the fielded content binds");
    let line_tank = tacsim::definitions::content::LINE_TANK;
    let red = fielded_unit(
        &catalog,
        line_tank,
        RED,
        Team::Red,
        facing(6, 4, 90_000),
        Controller::Pilot,
        1,
    );
    let mut blue = fielded_unit(
        &catalog,
        line_tank,
        BLUE,
        Team::Blue,
        facing(10, 4, 270_000),
        Controller::Pilot,
        1,
    );
    let mut narrowed = false;
    for band in &mut blue.profile.bands {
        if band.band == "front" {
            band.sections = vec![(SmolStr::new("turret"), 100)];
            narrowed = true;
        }
    }
    assert!(narrowed, "the stock tank has a front band");
    let pilots = [RED, BLUE]
        .into_iter()
        .enumerate()
        .map(|(seat, unit)| Pilot {
            unit,
            pilot: ObjId(1_000 + u32::try_from(seat).unwrap_or(0)),
            station: SmolStr::new("cockpit"),
            epoch: 1,
        })
        .collect();
    Arena::new(
        CustodyOut {
            definition: definition(&[]),
            units: vec![red, blue],
            pilots,
            resolution: tacsim::match_::resolution_for(&catalog),
            seed: SEED,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &ArenaConfig::default(),
        &MatchConfig::default(),
        &SensorsConfig::default(),
    )
    .expect("the duel")
}

/// Red starts up at 0 and, from 15,000, locks blue and fires at every 10,000 ms
/// beat (the cannon cycles in 6,000); the beats run long past the end.
fn ap2_script() -> Vec<Beat> {
    let mut beats = vec![Beat {
        t_ms: 0,
        intents: vec![intent(RED, IntentKind::Startup, 1)],
    }];
    for n in 0..30_u64 {
        let at = 15_000 + i64::try_from(n).unwrap_or(0) * 10_000;
        let seq = n * 2 + 2;
        beats.push(Beat {
            t_ms: at,
            intents: vec![
                intent(
                    RED,
                    IntentKind::Lock {
                        contact: contact_of(RED, BLUE),
                    },
                    seq,
                ),
                intent(
                    RED,
                    IntentKind::Fire {
                        weapon: SmolStr::new("cannon"),
                        slot: None,
                    },
                    seq + 1,
                ),
            ],
        });
    }
    beats
}

/// **A disarming hit ends the match at its own substep, at every cadence**
/// (16b cut AP.2; 16c `B-87`; doc 16 §3.8 read for the side; 16b §11's
/// mandatory cross-tempo test).
///
/// Red shoots blue's turret away. Blue came in armed, still drives and is
/// neither a wreck nor disabled — and it is out: a side that came in armed and
/// has no gun left is eliminated the substep its last gun goes. So the black
/// box holds exactly one `match_ended`, an elimination with red the winner,
/// stamped at the end of the substep the turret was destroyed in (W.2's rule
/// for every elimination: `settle_elimination` stamps the substep it ran in),
/// with no `unit_destroyed`, no `unit_disabled` and no `spent`; and the same
/// script at 5,000, 1,000 and 500 ms gives one event list and one per-substep
/// hash sequence — so the end is a fact of arena time, not of the pulse that
/// carried it.
#[test]
fn ap2_a_disarming_hit_ends_the_match_at_its_own_substep_at_every_tempo() {
    let mut runs: Vec<(i64, Replay)> = Vec::new();
    let mut substep = 0_i64;
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = ap2_duel();
        assert!(
            arena
                .units
                .iter()
                .all(tacsim::arena::state::UnitState::armed_at_checkout),
            "both tanks came in armed"
        );
        substep = i64::from(tacsim::arena::clock::substep_ms(
            &ArenaConfig::default().tempo,
            arena.cell_cm(),
            arena.max_speed_cms(),
        ));
        let run = replay(&mut arena, cadence, 400_000, &ap2_script());
        assert_eq!(
            arena.ended.map(|end| end.reason),
            Some(tacsim::arena::EndReason::Elimination {
                winner: Some(Team::Red)
            }),
            "cadence {cadence}: the disarmed side is out by {}",
            arena.now_ms
        );
        let blue = arena.unit(BLUE).unwrap_or_else(|| panic!("blue"));
        assert!(
            blue.weapons.is_empty() && !blue.limits.wreck && !blue.disabled,
            "cadence {cadence}: blue lost its gun and nothing else"
        );
        assert!(!arena.out_of_action(BLUE), "a disarmed tank still drives");
        runs.push((cadence, run));
    }
    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };
    let lines = |kind: &str| -> Vec<&Line> {
        first
            .events
            .iter()
            .filter(|(_, encoded, _, _)| encoded.contains(&format!("\"kind\":\"{kind}\"")))
            .collect()
    };
    let ended = lines("match_ended");
    assert_eq!(ended.len(), 1, "exactly one end: {ended:?}");
    assert!(
        ended
            .iter()
            .all(|(_, encoded, _, _)| encoded.contains(r#"{"elimination":{"winner":"red"}}"#)),
        "the disarmed side is eliminated, red the winner: {ended:?}"
    );
    let turret: Vec<&Line> = lines("section_destroyed")
        .into_iter()
        .filter(|(_, encoded, _, _)| {
            encoded.contains(&format!("\"unit\":{}", BLUE.0))
                && encoded.contains("\"section\":\"turret\"")
        })
        .collect();
    assert_eq!(turret.len(), 1, "blue's turret went once: {turret:?}");
    let (Some((lost_at, _, _, _)), Some((ended_at, _, _, _))) = (turret.first(), ended.first())
    else {
        panic!("a turret line and an end line");
    };
    // The end is the turret's own substep: the first substep boundary at or
    // after the impact, which is the same arena millisecond at every cadence.
    assert!(substep > 0, "a substep length");
    let boundary = (lost_at + substep - 1) / substep * substep;
    assert_eq!(
        *ended_at, boundary,
        "the end is stamped at the end of the substep the turret went in ({lost_at} ms, substep {substep} ms)"
    );
    for word in ["unit_destroyed", "unit_disabled"] {
        assert!(lines(word).is_empty(), "no `{word}`: the tank still drives");
    }
    assert!(
        first
            .events
            .iter()
            .all(|(_, encoded, _, _)| !encoded.contains("\"spent\"")),
        "never `spent`: blue's magazine is full"
    );

    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events, first.events,
            "the event list differs at {cadence} ms"
        );
        assert_eq!(
            run.hashes,
            first.hashes,
            "the per-substep hash sequence differs at {cadence} ms ({} against {})",
            run.hashes.len(),
            first.hashes.len()
        );
        assert_eq!(run.shots, first.shots);
        assert_ne!(run.pulses, first.pulses, "{cadence} ms ran the same pulses");
    }
}

// ---------------------------------------------------------------------------
// 16b cut AE.2 — the missile and the autocannon at three cadences
// ---------------------------------------------------------------------------

/// **A flat field for the AE.2 replays** (16b cut AE.2): 64 cells by 64 of
/// ground, room for a missile at 500 m and for a tank driving 150 m across the
/// run — with, when `walled`, [`AE2_WALL`] across it.
fn ae2_field(walled: bool) -> ArenaDefinition {
    const SIDE_AE2: u16 = 64;
    let (row, from, to) = AE2_WALL;
    let mut cells = Vec::with_capacity(usize::from(SIDE_AE2) * usize::from(SIDE_AE2));
    for y in 0..SIDE_AE2 {
        for x in 0..SIDE_AE2 {
            cells.push(if walled && y == row && (from..=to).contains(&x) {
                wall()
            } else {
                ground()
            });
        }
    }
    ArenaDefinition {
        id: SmolStr::new("tacsim.arena.ae2"),
        terrain: ArenaTerrain::new(SIDE_AE2, SIDE_AE2, CELL_CM, cells).expect("the field"),
        entry_zones: vec![
            EntryZone {
                team: Team::Red,
                x0: 0,
                y0: 0,
                x1: 1,
                y1: SIDE_AE2 - 1,
            },
            EntryZone {
                team: Team::Blue,
                x0: SIDE_AE2 - 2,
                y0: 0,
                x1: SIDE_AE2 - 1,
                y1: SIDE_AE2 - 1,
            },
        ],
        key_spots: Vec::new(),
        waypoints: tacsim::arena::state::Waypoints::default(),
        modes: Vec::new(),
    }
}

/// **The wall of scene (3)** (16b cut AE.2), as `(row, first column, last
/// column)`: an opaque run of [`wall`] one row north of a heavy driving east
/// along row 4, beginning one column past the cell (10, 4) it fires from at
/// 5,000 ms. The line from that cell to the tank at (14, 22) passes west of the
/// wall's end; the line from (11, 4), where the heavy is when the round lands
/// 740 ms later, runs into it.
const AE2_WALL: (u16, u16, u16) = (5, 11, 40);

/// **A shooter of `shooter`'s type and a stock tank, both seated and both
/// already awake** (16b cut AE.2), on [`ae2_field`], each at `(x, y, heading,
/// speed)` and so perhaps already under way. The power and the motion are laid
/// down before the first pulse — state the three replays share — so the script
/// is the fire orders alone.
fn ae2_arena(shooter: &str, red: (u16, u16, u32, i32), blue: (u16, u16, u32, i32)) -> Arena {
    ae2_arena_on(ae2_field(false), shooter, red, blue)
}

/// [`ae2_arena`] on a field of the caller's.
fn ae2_arena_on(
    field: ArenaDefinition,
    shooter: &str,
    red: (u16, u16, u32, i32),
    blue: (u16, u16, u32, i32),
) -> Arena {
    use tacsim::arena::state::Controller;
    let catalog = tacsim::definitions::Catalog::build_fielded(&[], tacsim::NS)
        .expect("the fielded content binds");
    let units = vec![
        fielded_unit(
            &catalog,
            shooter,
            RED,
            Team::Red,
            facing(red.0, red.1, red.2),
            Controller::Pilot,
            1,
        ),
        fielded_unit(
            &catalog,
            tacsim::definitions::content::LINE_TANK,
            BLUE,
            Team::Blue,
            facing(blue.0, blue.1, blue.2),
            Controller::Pilot,
            1,
        ),
    ];
    let pilots = [RED, BLUE]
        .into_iter()
        .enumerate()
        .map(|(seat, unit)| Pilot {
            unit,
            pilot: ObjId(1_000 + u32::try_from(seat).unwrap_or(0)),
            station: SmolStr::new("cockpit"),
            epoch: 1,
        })
        .collect();
    let mut arena = Arena::new(
        CustodyOut {
            definition: field,
            units,
            pilots,
            resolution: tacsim::match_::resolution_for(&catalog),
            seed: SEED,
            mode: tacsim::arena::MatchMode::Skirmish,
        },
        &ArenaConfig::default(),
        &MatchConfig::default(),
        &SensorsConfig::default(),
    )
    .expect("the AE.2 arena");
    for (id, (_, _, heading, speed)) in [(RED, red), (BLUE, blue)] {
        if let Some(state) = arena.unit_mut(id) {
            state.power = Power::Online;
            state.desired_heading_mdeg = heading;
            state.speed_cms = speed;
            state.desired_speed_cms = speed;
        }
    }
    arena
}

/// Lock the blue tank and fire `weapon` at each of `at`, one beat apiece.
fn ae2_script(weapon: &str, at: &[i64]) -> Vec<Beat> {
    let contact = contact_of(RED, BLUE);
    at.iter()
        .enumerate()
        .map(|(n, t_ms)| {
            let seq = u64::try_from(n).unwrap_or(0).saturating_mul(2) + 1;
            Beat {
                t_ms: *t_ms,
                intents: vec![
                    intent(RED, IntentKind::Lock { contact }, seq),
                    intent(
                        RED,
                        IntentKind::Fire {
                            weapon: SmolStr::new(weapon),
                            slot: None,
                        },
                        seq + 1,
                    ),
                ],
            }
        })
        .collect()
}

/// **One scene at 5,000, 1,000 and 500 ms** (16b §11): one event list and one
/// per-substep hash sequence, on a different number of pulses each time.
/// Answers the first replay's events, each as `(t_ms, JSON)`.
fn ae2_three(
    label: &str,
    until_ms: i64,
    build: &dyn Fn() -> Arena,
    script: &[Beat],
) -> Vec<(i64, serde_json::Value)> {
    let mut runs: Vec<(i64, Replay)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = build();
        let run = replay(&mut arena, cadence, until_ms, script);
        assert_eq!(arena.now_ms, until_ms, "{label}: {cadence} ms ran short");
        assert!(arena.shots.is_empty(), "{label}: every round arrived");
        runs.push((cadence, run));
    }
    let Some((_, first)) = runs.first() else {
        panic!("{label}: three replays");
    };
    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events, first.events,
            "{label}: the event list differs at {cadence} ms"
        );
        assert_eq!(
            run.hashes,
            first.hashes,
            "{label}: the per-substep hash sequence differs at {cadence} ms ({} against {})",
            run.hashes.len(),
            first.hashes.len()
        );
        assert_ne!(
            run.pulses, first.pulses,
            "{label}: {cadence} ms ran the same pulses"
        );
    }
    first
        .events
        .iter()
        .filter_map(|(t_ms, encoded, _, _)| {
            serde_json::from_str::<serde_json::Value>(encoded)
                .ok()
                .map(|value| (*t_ms, value))
        })
        .collect()
}

/// The lines of one kind, as `(t_ms, JSON)`.
fn ae2_lines(events: &[(i64, serde_json::Value)], kind: &str) -> Vec<(i64, serde_json::Value)> {
    events
        .iter()
        .filter(|(_, value)| value.get("kind").and_then(serde_json::Value::as_str) == Some(kind))
        .cloned()
        .collect()
}

/// One number off a line.
fn ae2_number(line: &serde_json::Value, key: &str) -> Option<i64> {
    line.get(key).and_then(serde_json::Value::as_i64)
}

/// **The mandatory cross-tempo test of 16b cut AE.2** (16b §11; doc 16 §3.1
/// "Rate independence", §3.5 "Scheduled impact").
///
/// The heavy's missile at 250 m/s, in three scenes, each replayed at 5,000,
/// 1,000 and 500 ms — one event list and one hash sequence every time:
///
/// 1. **the standing hit**: the tank stands broadside 200 m off, and the round
///    lands 800 ms after it was fired, a `hit` of 350;
/// 2. **the moving miss**: the tank crosses at 15 m/s 500 m off, and the round
///    arrives about 2 s later to a target cells from where it was — a `miss`
///    for `moved`;
/// 3. **the shooter that moved**: the heavy itself is under way at 10 m/s along
///    [`AE2_WALL`] when it fires at a standing tank broadside 185 m off, from
///    the last cell with a line past the wall's end, and is a cell further on,
///    behind the wall, when the round lands — traced from where it left, so a
///    `hit`, where a trace from where it ended (HEAD's rule before this cut)
///    would read `blocked`; the witness beside it pins the launch cell on the
///    record and the blocked line at the impact, so the scene tells the two
///    rules apart;
///
/// and the scout car's autocannon, three bursts of three at 5,000 ms beats:
/// three `fired` lines of three rounds and 231 of 240 left.
#[test]
fn ae2_one_missile_at_three_cadences_lands_at_one_millisecond_with_one_outcome() {
    use tacsim::definitions::content::{HEAVY_MK1, SCOUT_MK1};

    // (1) The standing hit.
    let events = ae2_three(
        "the standing hit",
        10_000,
        &|| ae2_arena(HEAVY_MK1, (8, 4, 0, 0), (8, 24, 90_000, 0)),
        &ae2_script("missiles", &[5_000]),
    );
    let fired = ae2_lines(&events, "fired");
    assert_eq!(fired.len(), 1, "one missile: {fired:?}");
    assert_eq!(
        fired
            .first()
            .and_then(|(_, line)| ae2_number(line, "range_cm")),
        Some(20_000)
    );
    assert_eq!(
        ae2_lines(&events, "impact")
            .iter()
            .map(|(t_ms, _)| *t_ms)
            .collect::<Vec<_>>(),
        vec![5_800],
        "20,000 cm at 25,000 cm/s is 800 ms"
    );
    assert_eq!(
        ae2_lines(&events, "hit")
            .iter()
            .map(|(t_ms, line)| (*t_ms, ae2_number(line, "damage")))
            .collect::<Vec<_>>(),
        vec![(5_800, Some(350))],
        "a standing broadside target takes the round at the impact millisecond"
    );

    // (2) The moving miss.
    let events = ae2_three(
        "the moving miss",
        10_000,
        &|| ae2_arena(HEAVY_MK1, (4, 4, 0, 0), (4, 54, 90_000, 1_500)),
        &ae2_script("missiles", &[5_000]),
    );
    let range = ae2_lines(&events, "fired")
        .first()
        .and_then(|(_, line)| ae2_number(line, "range_cm"))
        .expect("the missile's range");
    assert!(range > 50_000, "fired at the crossing tank: {range}");
    assert_eq!(
        ae2_lines(&events, "miss")
            .iter()
            .map(|(t_ms, line)| (
                *t_ms,
                line.get("reason")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            ))
            .collect::<Vec<_>>(),
        vec![(
            5_000 + range * 1_000 / 25_000,
            Some(tacsim::arena::damage::MOVED.to_owned())
        )],
        "the tank walked out from under it"
    );

    // (3) The shooter that moved: fired past the wall's end, landed from
    // behind it.
    let walled = || {
        ae2_arena_on(
            ae2_field(true),
            HEAVY_MK1,
            (5, 4, 90_000, 1_000),
            (14, 22, 90_000, 0),
        )
    };
    let script = ae2_script("missiles", &[5_000]);
    let events = ae2_three("the shooter that moved", 10_000, &walled, &script);
    let outcomes: Vec<String> = events
        .iter()
        .filter_map(
            |(_, line)| match line.get("kind").and_then(serde_json::Value::as_str) {
                Some("hit") => Some("hit".to_owned()),
                Some("miss") => line
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                _ => None,
            },
        )
        .collect();
    assert_eq!(
        outcomes,
        vec!["hit".to_owned()],
        "the round was traced from where it left, past the wall's end"
    );
    // The witness that the scene tells the two rules apart: the heavy fired
    // from a cell with a clear line, and stands in another, behind the wall,
    // when its round lands.
    let mut arena = walled();
    let _ = replay(&mut arena, 500, 5_500, &script);
    let shot = arena
        .shots
        .values()
        .next()
        .cloned()
        .expect("the missile in flight");
    assert_eq!(
        (shot.from_cell, shot.from_x_cm, shot.from_y_cm),
        (Some((10, 4)), 10_500, 4_500),
        "fired from (10, 4) at 5,000 ms, and the record says so"
    );
    let mut wall_ms = arena.now_ms;
    while arena.now_ms < shot.impact_at_ms {
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        wall_ms += 100;
        let _ = arena.step(wall_ms, &[] as &[Control], &[]);
    }
    let there = arena
        .unit(RED)
        .map(|state| {
            (
                state.pose.x_cm / i64::from(CELL_CM),
                state.pose.y_cm / i64::from(CELL_CM),
            )
        })
        .expect("the heavy");
    assert_eq!(there, (11, 4), "a cell on by the impact");
    assert!(
        !arena.fire_line(RED, BLUE).0.is_clear_or_partial(),
        "from where the heavy stands at the impact, the wall blocks the line"
    );

    // (4) The autocannon: three bursts.
    let script = ae2_script("autocannon", &[5_000, 10_000, 15_000]);
    let events = ae2_three(
        "the autocannon",
        20_000,
        &|| ae2_arena(SCOUT_MK1, (8, 4, 0, 0), (8, 24, 90_000, 0)),
        &script,
    );
    assert_eq!(
        ae2_lines(&events, "fired")
            .iter()
            .map(|(_, line)| ae2_number(line, "rounds"))
            .collect::<Vec<_>>(),
        vec![Some(3), Some(3), Some(3)]
    );
    let mut arena = ae2_arena(SCOUT_MK1, (8, 4, 0, 0), (8, 24, 90_000, 0));
    let _ = replay(&mut arena, 1_000, 20_000, &script);
    assert_eq!(
        arena.unit(RED).map(|state| state.weapon_ammo("autocannon")),
        Some(231)
    );
}

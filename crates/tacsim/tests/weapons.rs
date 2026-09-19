//! **Weapons and the shot, through `Arena::step`** (16b cut W.1, W1; doc 16 §3.5
//! "Acceptance and cost" and "Scheduled impact", §3.7's `LOCK`, `FIRE` and
//! `WEAPONSTATUS`, §3.1's tempo table and "Rate independence").
//!
//! Nine tests, driving the library directly — no host, no store, no task, the same
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
//!   fired plus rounds left equals the load.
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
    EntryZone, Pilot, Power, SeenPose, ShadowContact, Team, TerrainCell, TrackedRules,
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
        unit: tactical_unit(id),
    }
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
    // unit is at `id % sensors.pass_ms`, so both are behind us.
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
    assert_eq!(arena.resolve_fire(RED, "cannon"), Err(Refusal::NoLock));
    // A weapon the unit does not carry is refused before anything about the shot.
    assert_eq!(arena.resolve_fire(RED, "trebuchet"), Err(Refusal::NoWeapon));
    // A tank with nothing to shoot with is the same answer, not a panic.
    assert_eq!(arena.resolve_fire(BLUE, "cannon"), Err(Refusal::NoWeapon));

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
    assert_eq!(arena.resolve_fire(RED, "cannon"), Err(Refusal::Cycling));
    // Empty, ready.
    if let Some(state) = arena.unit_mut(RED) {
        state.ready_at_ms.insert(SmolStr::new("cannon"), 0);
        state.ammo.insert(SmolStr::new("cannon"), 0);
    }
    assert_eq!(arena.resolve_fire(RED, "cannon"), Err(Refusal::Empty));
    // Loaded, ready, locked — and cold, which is the last thing asked.
    if let Some(state) = arena.unit_mut(RED) {
        state.ammo.insert(SmolStr::new("cannon"), 40);
        state.power = Power::Off;
    }
    assert_eq!(
        arena.resolve_fire(RED, "cannon"),
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
        }
    );
    assert!(entry.draw.is_none(), "cut W.1 draws nothing");

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
    pulse(
        &mut arena,
        wall + 500,
        &[
            intent(RED, IntentKind::Lock { contact }, 3),
            intent(
                RED,
                IntentKind::Fire {
                    weapon: SmolStr::new("laser"),
                },
                4,
            ),
        ],
        &mut bytes,
    );
    let shot = arena.shot(1).cloned().expect("shot 1");
    assert_eq!(
        shot.impact_at_ms, shot.fired_at_ms,
        "a beam has no flight time"
    );
    assert!(shot.range_cm > 0, "and it still has a range");
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
    if let Some(state) = arena.unit_mut(RED) {
        state.weapons.clear();
    }
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
    let line = row.render();
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
    let row = cockpit::frame_weapons(&frame)
        .first()
        .cloned()
        .expect("the cannon");
    assert!(!row.ready);
    assert_eq!(row.rounds, 39);
    assert_eq!(row.ready_in_ms, i64::from(cannon().cycle_ms));
    let line = row.render();
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
            &tacsim::arena::perception::FrameNotice::Contact {
                kind: NoticeKind::LockLost,
                contact,
                class: Classification::Unknown,
            },
            &[]
        )
        .contains("Lock on"),
        "and it has a line of its own"
    );
    // A `FIRE` afterwards is refused for want of the lock it used to have.
    assert_eq!(arena.resolve_fire(RED, "cannon"), Err(Refusal::NoLock));
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
type Line = (i64, String, Option<(String, u64)>, u64);

/// What one replay produced: the ledger (normalised), the per-substep hashes, the
/// rounds fired and left, and how many pulses it took.
struct Replay {
    events: Vec<Line>,
    hashes: Vec<u64>,
    rounds_fired: u32,
    rounds_left: u32,
    shots: u64,
    pulses: u32,
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
fn replay(arena: &mut Arena, cadence_ms: i64, until_ms: i64, script: &[Beat]) -> Replay {
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = 0_i64;
    let mut next = 0_usize;
    let mut pulses = 0_u32;
    let mut ended = false;
    loop {
        arena.clock.pin_tempo_for_tests(Tempo::Quiet);
        let mut intents: Vec<Intent> = Vec::new();
        while let Some(beat) = script.get(next) {
            if beat.t_ms > arena.now_ms {
                break;
            }
            intents.extend(beat.intents.iter().cloned());
            next += 1;
        }
        if !intents.is_empty() {
            let out = arena.step(wall, &[] as &[Control], &intents);
            absorb(&out, &mut bytes);
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
                    event.draw.clone(),
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

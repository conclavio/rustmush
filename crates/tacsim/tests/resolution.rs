//! **The resolution and the damage, through `Arena::step`** (16b cut W.2, W2 and
//! W3; doc 16 §3.5 "Resolution", "Acceptance and cost", "Scheduled impact" and
//! "Damage", §3.3 "A wreck", "Facet attribution" and "Silhouette", §3.8 "Team
//! elimination", §3.1 "Custody").
//!
//! Twenty-nine tests. Twenty-eight drive the library directly — no host, no store,
//! no task — the same shape `arena.rs`, `perception.rs` and `weapons.rs` take; the
//! last is the **money rule** and needs a world, because the claim is about one.
//!
//! - **(a0)** the fixtures below are the shipped packs' own numbers, pinned, so a
//!   content edit fails here rather than quietly changing what every other test
//!   means;
//! - **(a)** doc 16 §3.5's five steps on fixed inputs: a pinned table of angular
//!   size against error cone, and the probability each comes out at;
//! - **(a2)** step 5: the percentage `LOCK` and `WEAPONSTATUS` show before any shot,
//!   through the frame and the rendered line;
//! - **(b)** a target that presents nothing resolves at zero, whatever the curve's
//!   floor says;
//! - **(c)** a shot at a target behind **full cover** is accepted, costs its round
//!   and its cycle, and misses `cover`;
//! - **(d)** a target that leaves the predicted cell misses `moved`, and one that
//!   stays a cell away does not;
//! - **(e)** a target that turns in flight re-resolves against the **same** draw;
//! - **(f)** the facet draw lands inside the band's authored weights, over pinned
//!   seeds and with a wide tolerance;
//! - **(g)** the armour floors, the overflow reaches the internals, the section's
//!   consequence fires, and a hull through its internals is a wreck **in one plan**;
//! - **(h)** a turret shot off takes the weapons with it, and an immobile disarmed
//!   tank is `unit_disabled`;
//! - **(i)** team elimination ends the match with the winner;
//! - **(j)** the **cross-tempo property** with the whole pipeline in it: one
//!   `fired`/`impact`/`hit` sequence and one per-substep hash sequence at 5,000,
//!   1,000 and 500 ms;
//! - **(k)** ammunition is conserved across a scripted run that hits;
//! - **(l)** the check-in's shape: the returned units carry their cells, their poses
//!   and their outcomes, and the plan the world applies is the diff;
//! - **(m)** the **money rule**: no instrument moves while a match runs;
//! - **(n1)** 16b cut F.2: the shooter is told the round landed and the victim is
//!   told where it landed, and **neither line carries the other pilot's half** —
//!   the shooter learns nothing of the internals and the victim learns nothing of
//!   the shooter;
//! - **(n2)** a miss carries the ledger's own reason in plain words;
//! - **(n3)** the kill line, the hull's own sentence, and `section_destroyed`
//!   **once** however many further rounds land on a spent section;
//! - **(n4)** every refusal the arena makes at apply reaches the pilot, in the
//!   cockpit's own words, and an order refused before it was ever queued stays
//!   silent;
//! - **(n5)** the percentage doc 16 §3.5 step 5 promises *on `LOCK` before any
//!   shot*, printed when the lock resolves;
//! - **(n5b)** each weapon's row carries **its own** chance and `LOCK` quotes the
//!   one a bare `FIRE` would take, on a platform carrying two;
//! - **(n6)** the cross-tempo property over the **notices**: one scripted duel
//!   gives each pilot one ordered list of lines at 5,000, 1,000 and 500 ms, and a
//!   second run covers the two notices produced outside the journal scan — a stop
//!   against a bank and an order refused at apply — plus the spent-section ledger;
//! - **(n7)** the frame's own status block: every section, its condition, its
//!   internals and its facets' armour against the cells they are read from, the
//!   armour a **struck-through** section's round really passed, and `STATUS`
//!   inside eighty columns.
//!
//! # Why these fixtures carry the stock tank's numbers
//!
//! The arithmetic under test is *content against engine*: 350 cm of presented
//! frontage at 1 km against a 3 mrad cone is a number doc 16 and
//! `content/resolution.json`'s own note both work out, and a test that invented its
//! own silhouettes would prove the engine self-consistent and nothing else. So the
//! fixtures here are `content/units/line_tank.json`'s and
//! `content/resolution.json`'s figures written out, and **(a0)** pins them equal to
//! the shipped packs — if the content moves, that test fails and says so, rather
//! than every number below quietly meaning something different.

use std::collections::BTreeMap;

use mush_core::{DefinitionDigest, DefinitionId, DefinitionKey, DefinitionKind, ObjId, OwnerId};
use mush_sim::data::Pose;
use mush_tactics::{
    CapabilityId, CapabilityInit, CapabilityKey, CellAddr, CellId, CellSpec, ComponentId,
    ComponentSpec, ConditionId, DefinitionRef, FacetId, FacetSpec, InstallPoint, RulesValue,
    SectionId, SectionSpec, TacticalUnit, UnitMutationOp, UnitMutationPlan, UnitSpec,
};
use smol_str::SmolStr;
use tacsim::arena::clock::Tempo;
use tacsim::arena::condition::{Condition, ConditionBand, Reading};
use tacsim::arena::damage::{self, UnitReturn};
use tacsim::arena::intents::{Control, Intent, IntentKind};
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena::perception::{SensorProfile, SignatureFactor};
use tacsim::arena::resolution::{BandProfile, Consequence, SectionProfile, UnitProfile};
use tacsim::arena::rng::{DrawLabel, draw_contact_id, draw_permille};
use tacsim::arena::state::{
    Arena, ArenaDefinition, ArenaTerrain, Classification, ContactId, CustodyOut, CustodyUnit,
    EndReason, EntryZone, OwnSighting, Pilot, Power, SeenPose, ShadowContact, Side, Team,
    TerrainCell, TrackedRules, WeaponSummary,
};
use tacsim::arena::step::StepOutput;
use tacsim::arena::weapons::WeaponBand;
use tacsim::config::{ArenaConfig, MatchConfig, SensorsConfig};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// The window: 16 by 16 cells of 1,000 cm.
const SIDE: u16 = 16;

/// The cell size, centimetres.
const CELL_CM: u16 = 1_000;

/// The match seed, the key of every draw a test computes.
const SEED: u64 = 2_350;

/// The shooter and the target.
const RED: ObjId = ObjId(4);
const BLUE: ObjId = ObjId(9);

/// The pilot `arena_of` seats on the red tank.
const RED_PILOT: ObjId = ObjId(1_000);

/// And on the blue one — the victim, in every test of 16b cut F.2's notices.
const BLUE_PILOT: ObjId = ObjId(1_001);

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

/// **The stock tank's structure**, as `content/units/line_tank.json` authors it:
/// five sections with their armour and internals, the cannon, the sight, and the
/// power capability a wreck loses. Test (a0) pins the numbers equal to the pack.
fn line_tank_unit(id: ObjId) -> TacticalUnit {
    let facet = |name: &str, armor: i64| FacetSpec {
        id: FacetId::new(name).expect("a canonical facet"),
        cells: vec![CellSpec {
            id: CellId::new("armor").expect("a cell"),
            min: 0,
            max: armor,
            init: armor,
        }],
    };
    let section = |name: &str, facets: Vec<FacetSpec>, internals: i64| SectionSpec {
        id: SectionId::new(name).expect("a canonical section"),
        condition: ConditionId::new("intact").expect("a condition"),
        facets,
        slots: Vec::new(),
        cells: vec![CellSpec {
            id: CellId::new("internals").expect("a cell"),
            min: 0,
            max: internals,
            init: internals,
        }],
    };
    let component = |name: &str, cells: Vec<CellSpec>| ComponentSpec {
        id: ComponentId::new(name).expect("a component"),
        definition: dref("component", &format!("tacsim.unit.line_tank.{name}")),
        install: InstallPoint::Section(SectionId::new("turret").expect("a canonical section")),
        condition: ConditionId::new("intact").expect("a condition"),
        cells,
        asset: None,
    };
    let spec = UnitSpec {
        definition: dref("unit", "tacsim.unit.line_tank"),
        sections: vec![
            section(
                "hull",
                vec![
                    facet("front", 400),
                    facet("left", 250),
                    facet("right", 250),
                    facet("rear", 150),
                    facet("top", 100),
                ],
                200,
            ),
            section(
                "turret",
                vec![
                    facet("front", 350),
                    facet("left", 220),
                    facet("right", 220),
                    facet("rear", 140),
                ],
                150,
            ),
            section("left_track", vec![facet("left", 120)], 100),
            section("right_track", vec![facet("right", 120)], 100),
            section("engine", Vec::new(), 120),
        ],
        unit_cells: Vec::new(),
        components: vec![
            component(
                "cannon",
                vec![
                    CellSpec {
                        id: CellId::new("ammo").expect("a cell"),
                        min: 0,
                        max: 40,
                        init: 40,
                    },
                    CellSpec {
                        id: CellId::new("ready_at_ms").expect("a cell"),
                        min: 0,
                        max: 3_600_000,
                        init: 0,
                    },
                ],
            ),
            component(
                "visual_sensor",
                vec![CellSpec {
                    id: CellId::new("condition").expect("a cell"),
                    min: 0,
                    max: 100,
                    init: 100,
                }],
            ),
        ],
        capabilities: vec![CapabilityInit {
            key: CapabilityKey {
                id: CapabilityId::new("tacsim.power").expect("a capability"),
                source: None,
            },
            rules: rules(),
        }],
        mobility: None,
        rules: rules(),
    };
    TacticalUnit::from_spec(&spec, id).expect("the stock tank")
}

/// The stock tank's facet table and silhouettes, as the arena carries them.
fn line_tank_profile() -> UnitProfile {
    let band = |name: &str, width: u32, height: u32, rows: &[(&str, u16)]| BandProfile {
        band: SmolStr::new(name),
        width_cm: width,
        height_cm: height,
        sections: rows
            .iter()
            .map(|(section, weight)| (SmolStr::new(*section), *weight))
            .collect(),
    };
    let section = |name: &str, facets: &[&str], consequences: Vec<Consequence>| SectionProfile {
        id: SmolStr::new(name),
        facets: facets.iter().map(|facet| SmolStr::new(*facet)).collect(),
        consequences,
        struck_through: None,
    };
    UnitProfile {
        bands: vec![
            band("front", 350, 250, &[("hull", 50), ("turret", 50)]),
            band(
                "left",
                700,
                250,
                &[("hull", 40), ("left_track", 30), ("turret", 30)],
            ),
            band(
                "right",
                700,
                250,
                &[("hull", 40), ("right_track", 30), ("turret", 30)],
            ),
            band(
                "rear",
                350,
                250,
                &[("hull", 40), ("engine", 30), ("turret", 30)],
            ),
            band("top", 700, 350, &[("hull", 50), ("turret", 50)]),
        ],
        sections: vec![
            section(
                "hull",
                &["front", "left", "right", "rear", "top"],
                vec![Consequence::DestroyUnit],
            ),
            section(
                "turret",
                &["front", "left", "right", "rear"],
                vec![Consequence::RemoveWeapons {
                    section: SmolStr::new("turret"),
                }],
            ),
            // 16b cut M.4 (`B-47`): the pair rule, as the version-2 pack
            // authors it on both tracks.
            section(
                "left_track",
                &["left"],
                vec![
                    Consequence::Slow { percent: 50 },
                    Consequence::NoTurnToward { side: Side::Left },
                    Consequence::DisableMotionWhenAllOf {
                        sections: vec![SmolStr::new("left_track"), SmolStr::new("right_track")],
                    },
                ],
            ),
            section(
                "right_track",
                &["right"],
                vec![
                    Consequence::Slow { percent: 50 },
                    Consequence::NoTurnToward { side: Side::Right },
                    Consequence::DisableMotionWhenAllOf {
                        sections: vec![SmolStr::new("left_track"), SmolStr::new("right_track")],
                    },
                ],
            ),
            SectionProfile {
                id: SmolStr::new("engine"),
                facets: Vec::new(),
                consequences: vec![Consequence::DisableMotion],
                struck_through: Some((SmolStr::new("hull"), SmolStr::new("rear"))),
            },
        ],
    }
}

/// Passable ground at cost 100.
fn ground() -> TerrainCell {
    cell(mush_atlas::TerrainClass::Ground)
}

/// A **passable** cell of `class`, so a test can stand a tank on ground that gives
/// cover. In play the authoring is what puts a hull-down tank behind a low wall;
/// here it is one line, and cover is the property under test and not the tile.
fn cell(class: mush_atlas::TerrainClass) -> TerrainCell {
    TerrainCell {
        class,
        elev_cm: 0,
        move_cost: 100,
    }
}

/// A pose at `(x_cm, y_cm)` facing north.
fn pose(x_cm: i64, y_cm: i64) -> Pose {
    Pose::at(x_cm, y_cm, 0, rules())
}

/// The centre of cell `(x, y)`.
fn centre(x: u16, y: u16) -> Pose {
    pose(
        i64::from(x) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
        i64::from(y) * i64::from(CELL_CM) + i64::from(CELL_CM) / 2,
    )
}

/// The window with `paint`ed cells over uniform ground.
fn definition(paint: &[(u16, u16, TerrainCell)]) -> ArenaDefinition {
    let mut cells = vec![ground(); usize::from(SIDE) * usize::from(SIDE)];
    for (x, y, painted) in paint {
        if let Some(slot) = cells.get_mut(usize::from(*y) * usize::from(SIDE) + usize::from(*x)) {
            *slot = *painted;
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

/// A five-cell sight, so a 16-cell window has an outside.
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
        share_range_cells: 0,
        signature: vec![
            row(C::Ground, 1_000, 1_000),
            row(C::Rubble, 1_000, 1_000),
            row(C::Wall, 1_000, 1_000),
        ],
    }
}

/// `content/weapons/cannon_105.json` and `content/ammunition/he_105.json`, joined
/// as the arena carries them, with the cycle shortened to four seconds so a test
/// fires more than once inside a scripted minute.
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
        damage: 120,
        mount_section: SmolStr::new("turret"),
    }
}

/// The coaxial machine gun: ten rounds a pull, so the burst path is exercised.
fn machine_gun() -> WeaponSummary {
    WeaponSummary {
        id: SmolStr::new("machine_gun"),
        max_range_cm: 120_000,
        cycle_ms: 1_000,
        ammunition: SmolStr::new("tacsim.ammunition.ball_762"),
        burst_rounds: 10,
        muzzle_speed_cms: 85_000,
        bands: vec![
            WeaponBand {
                max_range_cm: 60_000,
                dispersion_mrad: 8,
            },
            WeaponBand {
                max_range_cm: 120_000,
                dispersion_mrad: 16,
            },
        ],
        count_per_load: 2_000,
        damage: 3,
        mount_section: SmolStr::new("turret"),
    }
}

/// One stock tank with `weapons`.
fn tank(id: ObjId, team: Team, pose: Pose, weapons: Vec<WeaponSummary>) -> CustodyUnit {
    CustodyUnit {
        id,
        team,
        pose,
        mobility: TrackedRules {
            max_speed_cms: 1_000,
            accel_cms2: 1_000,
            brake_cms2: 1_000,
            turn_rate_mdeg_s: 90_000,
            reverse_factor_pct: 40,
        },
        sensor_height_cm: 150,
        sensor: Some(sight()),
        weapons,
        profile: line_tank_profile(),
        gunnery: 1,
        controller: tacsim::arena::state::Controller::Pilot,
        // 16b cut Q.1: no pod behind it.
        slot: 0,
        unit: line_tank_unit(id),
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

/// An arena over `paint`ed ground with one seated pilot per unit.
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

/// The red tank's cell, and the blue one's: four cells apart on the **north-south**
/// axis, both facing north.
///
/// The axis is the whole reason the fixture is shaped this way. Red stands due south
/// of blue, so the round arrives at blue's **rear** band — 350 cm of presented
/// frontage, which is the figure doc 16 §3.3 authors and
/// `content/resolution.json`'s own note works its example out on. A flank shot is
/// 700 cm and a different arithmetic, and the tests that want one turn the target
/// rather than moving it.
const RED_CELL: (u16, u16) = (8, 5);
/// The blue tank's cell.
const BLUE_CELL: (u16, u16) = (8, 9);

/// Two stock tanks four cells apart on clear ground, the red one armed with the
/// cannon.
fn duel() -> Arena {
    arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(RED_CELL.0, RED_CELL.1),
                vec![cannon()],
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(BLUE_CELL.0, BLUE_CELL.1),
                Vec::new(),
            ),
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

/// The contact id `viewer` holds `target` under in this match.
fn contact_of(viewer: ObjId, target: ObjId) -> ContactId {
    draw_contact_id(SEED, viewer, target)
}

/// **One unit's public callsign** (16b cut Q.1) — what every viewer that has told
/// it apart calls it, and what a kill line says after the contact is gone.
fn callsign(arena: &Arena, unit: ObjId) -> String {
    arena
        .unit(unit)
        .map(|state| state.callsign.to_string())
        .unwrap_or_else(|| panic!("#{} is not in the match", unit.0))
}

/// **The short name one platform's own frame gives a target** (16b cut Q.1).
///
/// Every pilot-facing line names a contact by its designator now, and the
/// designator is read off the live contact's class — so a test that pinned the
/// sixteen digits asks for this instead, off the same arena the line came from,
/// rather than hard-coding `BA` and quietly passing when the naming breaks.
fn named(arena: &Arena, viewer: ObjId, target: ObjId) -> String {
    let id = contact_of(viewer, target);
    arena
        .frame_contacts(viewer)
        .into_iter()
        .find(|row| row.id == id)
        .map(|row| row.designator.to_string())
        .unwrap_or_else(|| panic!("#{} holds no contact on #{}", viewer.0, target.0))
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

/// The `reason` of every `miss` journalled, in order.
fn misses(events: &[LedgerEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Miss { reason, .. } => Some(reason.to_string()),
            _ => None,
        })
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
    while arena.now_ms < 1_000 {
        wall += 500;
        pulse(arena, wall, &[], bytes);
    }
    assert_eq!(
        arena.unit(RED).map(|state| state.power),
        Some(Power::Online),
        "the red tank never came online"
    );
    wall
}

/// **Where `target` actually stands, as a contact's plot** (16b cut F.3).
///
/// These fixtures plant a contact by hand, and the plot they plant is the
/// target's true pose — so every range and bearing they assert is the one they
/// asserted before the contact store learned the difference.
fn plot_of(arena: &Arena, target: ObjId) -> SeenPose {
    arena.unit(target).map(SeenPose::of).unwrap_or_default()
}

/// Hand `viewer` a classified contact on `target` — the store written by hand, for
/// the same reason `weapons.rs` writes it: a test about a *shot* must not fail for a
/// reason about perception.
fn hold(arena: &mut Arena, viewer: ObjId, target: ObjId) -> ContactId {
    let id = contact_of(viewer, target);
    let now = arena.now_ms;
    arena.contacts.insert(
        viewer,
        id,
        ShadowContact::sighted(
            target,
            Classification::Classified { team: Team::Blue },
            now,
            plot_of(arena, target),
        ),
    );
    id
}

/// One cell's value on a unit.
fn cell_of(arena: &Arena, unit: ObjId, addr: &CellAddr) -> Option<i64> {
    arena
        .unit(unit)?
        .unit
        .cell(addr)
        .map(|scalar| scalar.current)
}

fn armor(section: &str, facet: &str) -> CellAddr {
    CellAddr::Facet {
        section: SectionId::new(section).expect("a section"),
        facet: FacetId::new(facet).expect("a facet"),
        cell: CellId::new("armor").expect("a cell"),
    }
}

fn internals(section: &str) -> CellAddr {
    CellAddr::Section {
        section: SectionId::new(section).expect("a section"),
        cell: CellId::new("internals").expect("a cell"),
    }
}

/// One entry of the shipped content, as JSON.
fn pack(relative: &str) -> serde_json::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("content")
        .join(relative);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} is readable: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} is JSON: {error}", path.display()))
}

// ---------------------------------------------------------------------------
// (a0) The fixtures are the content's own numbers
// ---------------------------------------------------------------------------

#[test]
fn the_fixtures_carry_the_shipped_packs_numbers() {
    // The resolution table: the engine's documented fallback against the pack.
    let shipped = pack("resolution.json");
    let authored = shipped
        .pointer("/entries/tacsim.resolution.stock/def")
        .cloned()
        .unwrap_or_else(|| panic!("the pack declares the stock resolution"));
    let stock = tacsim::match_::stock_resolution();
    let curve: Vec<[u32; 2]> = serde_json::from_value(
        authored
            .get("hit_curve")
            .cloned()
            .unwrap_or_else(|| panic!("a curve")),
    )
    .unwrap_or_else(|error| panic!("the curve decodes: {error}"));
    assert_eq!(
        stock.hit_curve, curve,
        "arena_def::stock_resolution has drifted from content/resolution.json"
    );
    assert_eq!(
        serde_json::to_value(stock.error_terms).ok(),
        authored.get("error_terms").cloned(),
        "the four weights have drifted"
    );
    assert_eq!(
        (stock.p_min, stock.p_max),
        (
            authored
                .get("p_min")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as u32,
            authored
                .get("p_max")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as u32
        ),
        "the clamps have drifted"
    );

    // The tank: the armour, the internals and the silhouettes this file resolves
    // against.
    let tank = pack("units/line_tank.json");
    let def = tank
        .pointer("/entries/tacsim.unit.line_tank/def")
        .cloned()
        .unwrap_or_else(|| panic!("the pack declares the line tank"));
    let arena = duel();
    let profile = &arena.unit(BLUE).unwrap_or_else(|| panic!("blue")).profile;
    let silhouettes = def
        .get("silhouettes")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(profile.bands.len(), silhouettes.len());
    for row in &silhouettes {
        let band = row
            .get("band")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let carried = profile
            .band(band)
            .unwrap_or_else(|| panic!("the fixture carries the {band} band"));
        assert_eq!(
            (u64::from(carried.width_cm), u64::from(carried.height_cm)),
            (
                row.get("width_cm")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                row.get("height_cm")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0)
            ),
            "the {band} silhouette has drifted from content/units/line_tank.json"
        );
    }
    for row in def
        .get("sections")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let id = row
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        assert_eq!(
            cell_of(&arena, BLUE, &internals(id)),
            row.get("internals").and_then(serde_json::Value::as_i64),
            "the {id} internals have drifted"
        );
        for facet in row
            .get("facets")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            let name = facet
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            assert_eq!(
                cell_of(&arena, BLUE, &armor(id, name)),
                facet.get("armor").and_then(serde_json::Value::as_i64),
                "the {id}.{name} armour has drifted"
            );
        }
    }

    // And the ammunition's damage, which is what every arithmetic below spends.
    let he = pack("ammunition/he_105.json");
    assert_eq!(
        he.pointer("/entries/tacsim.ammunition.he_105/def/damage")
            .and_then(serde_json::Value::as_u64),
        Some(u64::from(cannon().damage)),
        "the round's damage has drifted from content/ammunition/he_105.json"
    );

    // The miss vocabulary is pinned here and nowhere else.
    assert_eq!(
        damage::MISS_REASONS,
        [
            "target_gone",
            "wreck",
            "moved",
            "blocked",
            "cover",
            "dispersion",
            "refused",
        ],
        "doc 16 §3.5's miss reasons changed; a seventh is a reviewed diff"
    );
}

// ---------------------------------------------------------------------------
// (a) The five steps on fixed inputs
// ---------------------------------------------------------------------------

#[test]
fn the_five_steps_are_the_pinned_table_of_angular_size_against_error() {
    let arena = duel();
    // Step 3's curve, read directly: `x = angular × 1000 / error` in the same unit
    // at both ends, so the ratio is what the table is indexed by. Every row is
    // `(angular_urad, error_urad, x_permille, p_permille)`, and every `p` is the
    // shipped curve's own interpolation at that `x`.
    let table: [(u32, u32, u32, u32); 9] = [
        // A target a quarter the width of the cone: below the curve's first knee,
        // and the floor is what answers (p_min 10).
        (1_000, 4_000, 250, 50),
        // Exactly as wide as the cone: the curve's 1,000 point, 350 per mille.
        (4_000, 4_000, 1_000, 350),
        // The stock tank's rear at 1 km against a 3 mrad band and a green gunner.
        (3_500, 4_000, 875, 300),
        // Its flank at the same range: twice the frontage, most of the way up.
        (7_000, 4_000, 1_750, 537),
        // The same flank at 2 km through the outer band and the gunnery term.
        (3_500, 7_000, 500, 150),
        // Point blank: past the last knee, and the ceiling answers (p_max 950).
        (100_000, 4_000, 25_000, 950),
        // A hopeless shot: below the first point, and the floor answers.
        (10, 16_000, 0, 10),
        // The curve's own knees, exactly.
        (2_000, 4_000, 500, 150),
        (8_000, 4_000, 2_000, 600),
    ];
    for (angular, error, want_x, want_p) in table {
        let (x, p) = arena.hit_permille(angular, error);
        assert_eq!(
            (x, p),
            (want_x, want_p),
            "the curve moved at {angular} µrad against {error} µrad"
        );
    }

    // And steps 1 and 2 producing exactly those numbers, end to end: blue faces
    // north and red is due south of it, so a shot at 1 km presents the rear's
    // 350 cm against 3 mrad of band plus 1 mrad of green gunnery.
    let taken = arena.resolve_shot(RED, BLUE, "cannon", 100_000, mush_atlas::Cover::None);
    assert_eq!(taken.band, "rear");
    assert_eq!(taken.presented_width_cm, 350);
    assert_eq!(taken.presented_area_cm2, 350 * 250);
    assert_eq!(taken.angular_urad, 3_500);
    assert_eq!(taken.error_urad, 4_000);
    assert_eq!(taken.x_permille, 875);
    assert_eq!(taken.p_permille, 300);
    // Step 5: what the cockpit would show, off the same arithmetic.
    let mut arena = duel();
    let contact = hold(&mut arena, RED, BLUE);
    if let Some(state) = arena.unit_mut(RED) {
        state.lock = Some(contact);
    }
    // Four cells is 4,000 cm, where the rear's 350 cm is 875 µrad against the same
    // 4,000 µrad cone — well past the curve's last knee, so the ceiling answers.
    // **16b cut F.2**: per weapon, off the one aim the frame resolves.
    let aim = arena
        .lock_aim(RED)
        .unwrap_or_else(|| panic!("the red tank has a lock"));
    assert_eq!(aim.target, BLUE);
    assert_eq!(
        arena
            .frame_weapons(RED, Some(&aim))
            .iter()
            .find(|weapon| weapon.id == "cannon")
            .and_then(|weapon| weapon.p_permille),
        Some(950)
    );
}

// ---------------------------------------------------------------------------
// (a2) Step 5: the percentage the cockpit shows before any shot
// ---------------------------------------------------------------------------

#[test]
fn the_cockpit_shows_the_chance_against_the_lock_before_any_shot() {
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    // The real contact, off the real sensor pass: four cells is inside the
    // fixture's five-cell sight.
    let contact = contact_of(RED, BLUE);
    assert!(
        arena.contacts.get(RED, contact).is_some(),
        "the red tank detected the blue one"
    );
    let out = pulse(
        &mut arena,
        wall + 500,
        &[intent(RED, IntentKind::Lock { contact }, 3)],
        &mut bytes,
    );
    let frame = out
        .frames
        .iter()
        .find(|(pilot, _)| *pilot == RED_PILOT)
        .map(|(_, frame)| frame.clone())
        .unwrap_or_else(|| panic!("the red pilot's frame"));
    assert_eq!(frame.lock, Some(contact));
    // 4,000 cm against the rear's 350 cm is 87,500 µrad against a 4,000 µrad cone,
    // which is past the curve's last knee: the ceiling answers (p_max 950).
    // **16b cut F.2**: the number lives on the weapon's own row now.
    assert_eq!(
        frame
            .weapons
            .iter()
            .find(|weapon| weapon.id == "cannon")
            .and_then(|weapon| weapon.p_permille),
        Some(950)
    );
    // The one sentence `LOCK` and `WEAPONSTATUS` both print.
    let line = tacsim::match_::cockpit::lock_line(&frame)
        .unwrap_or_else(|| panic!("a held lock has a line"));
    // 16b cut Q.1: the short designator, never the sixteen digits.
    assert!(line.contains(&named(&arena, RED, BLUE)), "{line}");
    assert!(!line.contains(&contact.to_string()), "{line}");
    assert!(line.contains("95% to hit"), "{line}");
    // And the panel's own copy of the same number.
    let payload = tacsim::match_::cockpit::status_frame(tacsim::arena_task::MatchId(1), &frame);
    assert_eq!(
        payload
            .get("lock_p_permille")
            .and_then(serde_json::Value::as_u64),
        Some(950)
    );
    assert_eq!(
        payload
            .get("lock_percent")
            .and_then(serde_json::Value::as_u64),
        Some(95)
    );
    // **16b cut Q.1's additions to the same payload**, asserted here because the
    // TUI (Q.4) and the web (Q.5) are both pinned to this shape by Q.3's fixture
    // set: a golden blessed off a wrong payload would agree with itself for ever.
    assert_eq!(
        payload.get("callsign").and_then(serde_json::Value::as_str),
        Some(callsign(&arena, RED).as_str()),
        "the platform's own public name: {payload}"
    );
    assert_eq!(
        payload.get("cell").and_then(serde_json::Value::as_array),
        frame
            .cell
            .map(|(x, y)| vec![serde_json::json!(x), serde_json::json!(y)])
            .as_ref(),
        "the cell a panel draws the `@` in: {payload}"
    );
    let designator = named(&arena, RED, BLUE);
    assert_eq!(
        payload
            .get("lock_designator")
            .and_then(serde_json::Value::as_str),
        Some(designator.as_str()),
        "the lock's own name, the one `lock_line` says: {payload}"
    );
    // The lock keeps its sixteen digits as the row key beside it, never displayed.
    assert_eq!(
        payload.get("lock").and_then(serde_json::Value::as_str),
        Some(contact.to_string().as_str())
    );
    let plot = frame
        .contacts
        .iter()
        .find(|held| held.id == contact)
        .unwrap_or_else(|| panic!("the frame's own row for the lock"));
    let coarse = tacsim::match_::cockpit::range_m_rounded(
        plot.range_cm,
        tacsim::match_::cockpit::CONTACT_RANGE_STEP_M,
    );
    // **16b cut D.4** (`T-20`): the lock's step is derived from the plot's own
    // age, and this frame's plot is fresh, so it is the floor.
    let fine_step = tacsim::match_::cockpit::lock_range_step_m(plot.age_ms);
    assert_eq!(
        fine_step,
        tacsim::match_::cockpit::LOCK_RANGE_STEP_M,
        "a plot this fresh earns the floor: {} ms",
        plot.age_ms
    );
    let fine = tacsim::match_::cockpit::range_m_rounded(plot.range_cm, fine_step);
    assert_ne!(
        coarse, fine,
        "the fixture was chosen so the finer step is visible: {} cm",
        plot.range_cm
    );
    assert_eq!(
        payload
            .get("lock_range_m")
            .and_then(serde_json::Value::as_u64),
        Some(fine),
        "the lock reads to ten metres and the scope to fifty: {payload}"
    );
    let row = payload
        .get("contacts")
        .and_then(serde_json::Value::as_array)
        .and_then(|rows| rows.first())
        .unwrap_or_else(|| panic!("the one contact's row: {payload}"));
    assert_eq!(
        row.get("id").and_then(serde_json::Value::as_str),
        Some(contact.to_string().as_str()),
        "the stable row key stays the sixteen digits"
    );
    assert_eq!(
        row.get("designator").and_then(serde_json::Value::as_str),
        Some(designator.as_str())
    );
    assert_eq!(
        row.get("range_m").and_then(serde_json::Value::as_u64),
        Some(coarse),
        "a scope row keeps the coarse step"
    );
    // The alias is the number behind a renamed row, spelled as the pilot would
    // have typed it — and `null` while the number IS the name.
    let alias = row.get("alias").cloned().unwrap_or(serde_json::Value::Null);
    if designator.starts_with('X') {
        assert_eq!(alias, serde_json::Value::Null, "{payload}");
    } else {
        assert_eq!(alias, serde_json::json!("X1"), "{payload}");
    }
    // And a notice carries the name beside the row key.
    let notice = payload
        .get("notices")
        .and_then(serde_json::Value::as_array)
        .and_then(|notices| {
            notices.iter().find(|notice| {
                notice.get("kind").and_then(serde_json::Value::as_str) == Some("locked")
            })
        })
        .unwrap_or_else(|| panic!("the lock's own notice: {payload}"));
    assert_eq!(
        notice.get("contact").and_then(serde_json::Value::as_str),
        Some(contact.to_string().as_str())
    );
    assert_eq!(
        notice.get("designator").and_then(serde_json::Value::as_str),
        Some(designator.as_str())
    );

    // **The two read verbs' own sentences** (16b cut Q.1), off this same frame.
    //
    // The verbs are `named_row` and then these: the step a lock earns lives here,
    // so inverting the branch fails here rather than in a live approach where the
    // plot moves between the two readings.
    let rows = tacsim::match_::cockpit::frame_contacts(&frame);
    let held_row = rows
        .iter()
        .find(|row| row.id == contact)
        .cloned()
        .unwrap_or_else(|| panic!("the contact's own row"));
    assert_eq!(
        tacsim::match_::cockpit::range_line(&frame, &held_row, 1_000),
        format!("Range to {designator}: {fine} m."),
        "a locked contact reads to the nearest ten metres"
    );
    let mut unlocked = frame.clone();
    unlocked.lock = None;
    assert_eq!(
        tacsim::match_::cockpit::range_line(&unlocked, &held_row, 1_000),
        format!("Range to {designator}: {coarse} m."),
        "and the rest of the scope to fifty"
    );
    assert_eq!(
        tacsim::match_::cockpit::bearing_line(&held_row, 1_000),
        format!(
            "Bearing to {designator}: {:03}° (rel {:03}°).",
            held_row.bearing_deg, held_row.relative_deg
        )
    );
    // And the staleness clause, which is a clause of both sentences and not a
    // sentence of its own: a plot older than one sensor pass says how old it is.
    let mut stale = held_row.clone();
    stale.age_ms = 4_200;
    // **16b cut D.4** (`T-20`): a four-second plot cannot carry ten metres — the
    // target could have moved seventy — so the lock's step is the scope's own by
    // the time the clause is printed. The clause and the step are two readings of
    // the same age, which is the point of deriving the step from it.
    let stale_metres = tacsim::match_::cockpit::range_m_rounded(
        plot.range_cm,
        tacsim::match_::cockpit::lock_range_step_m(stale.age_ms),
    );
    assert_eq!(
        stale_metres, coarse,
        "a four-second lock reads like the scope"
    );
    assert_eq!(
        tacsim::match_::cockpit::range_line(&frame, &stale, 1_000),
        format!("Range to {designator}: {stale_metres} m, plotted 4 s ago.")
    );
    assert_eq!(
        tacsim::match_::cockpit::bearing_line(&stale, 1_000),
        format!(
            "Bearing to {designator}: {:03}° (rel {:03}°), plotted 4 s ago.",
            stale.bearing_deg, stale.relative_deg
        )
    );
    assert_eq!(
        tacsim::match_::cockpit::range_line(&frame, &stale, 5_000),
        format!("Range to {designator}: {stale_metres} m."),
        "a plot inside the pass it was taken in says nothing about its age"
    );

    // **And the map's marker for the same contact** (16b cut Q.1): the grid
    // shape's `id` is display text, so it is the designator and never the
    // sixteen digits; the `name` is still the class word and nothing else.
    let (markers, glyphs) = tacsim::match_::cockpit::contact_markers(&frame, (0, 0), (SIDE, SIDE));
    let marker = markers
        .first()
        .unwrap_or_else(|| panic!("one marker for the one contact: {markers:?}"));
    assert_eq!(
        marker.get("id").and_then(serde_json::Value::as_str),
        Some(designator.as_str())
    );
    assert_eq!(
        marker.get("name").and_then(serde_json::Value::as_str),
        Some(held_row.class.as_str()),
        "a marker says the class word and nothing more about what is standing there"
    );
    assert!(
        !serde_json::Value::Array(markers.clone())
            .to_string()
            .contains(&contact.to_string()),
        "the sixteen digits reached the map: {markers:?}"
    );
    assert_eq!(
        marker.get("cell").and_then(serde_json::Value::as_array),
        held_row
            .cell
            .map(|(x, y)| vec![
                serde_json::json!(u32::from(x)),
                serde_json::json!(u32::from(y))
            ])
            .as_ref(),
        "the plot's own cell, off the window's corner"
    );
    assert!(!glyphs.is_empty(), "the window earned a legend entry");
    // With no lock there is no line and no number, rather than a nought that means
    // *unknown*.
    let mut bare = frame.clone();
    bare.lock = None;
    for weapon in &mut bare.weapons {
        weapon.p_permille = None;
    }
    assert_eq!(tacsim::match_::cockpit::lock_line(&bare), None);
    // **16b cut F.2**: and the row itself carries the same number, which is what
    // makes `WEAPONSTATUS` per weapon rather than per platform.
    let rows = tacsim::match_::cockpit::frame_weapons(&frame);
    let cannon = rows
        .iter()
        .find(|row| row.id == "cannon")
        .unwrap_or_else(|| panic!("the cannon's row"));
    assert_eq!(cannon.p_permille, Some(950));
    assert!(
        cannon.render(&rows).contains("95% to hit"),
        "{}",
        cannon.render(&rows)
    );
}

// ---------------------------------------------------------------------------
// (b) Zero presented area is zero
// ---------------------------------------------------------------------------

#[test]
fn a_target_that_presents_nothing_resolves_at_zero_whatever_the_floor_says() {
    let mut arena = duel();
    // The curve's floor is 10 per mille and never zero, so this is the one case
    // that has to be answered before the clamp (doc 16 §3.5 step 3).
    assert_eq!(arena.resolution.p_min, 10);
    if let Some(state) = arena.unit_mut(BLUE) {
        state.profile = UnitProfile::default();
    }
    let taken = arena.resolve_shot(RED, BLUE, "cannon", 100_000, mush_atlas::Cover::None);
    assert_eq!(taken.presented_width_cm, 0);
    assert_eq!(taken.presented_area_cm2, 0);
    assert_eq!(taken.p_permille, 0);
    // And so does a target under full cover, which is the same arithmetic.
    let arena = duel();
    let blind = arena.resolve_shot(RED, BLUE, "cannon", 100_000, mush_atlas::Cover::Full);
    assert_eq!(blind.p_permille, 0);
    // Partial cover halves the frontage and so the angular size.
    let half = arena.resolve_shot(RED, BLUE, "cannon", 100_000, mush_atlas::Cover::Partial);
    assert_eq!(half.presented_width_cm, 175);
    assert_eq!(half.angular_urad, 1_750);
    assert_eq!(half.presented_area_cm2, 175 * 250);
    assert!(half.p_permille > 0 && half.p_permille < 300);
}

// ---------------------------------------------------------------------------
// (c) Full cover costs a round and a cycle and misses `cover`
// ---------------------------------------------------------------------------

#[test]
fn a_shot_at_a_target_behind_full_cover_costs_a_round_and_a_cycle_and_misses_cover() {
    // The blue tank stands on a wall cell: the ground under it is full cover, and
    // the line to it is clear (a trace's endpoints never block their own line).
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(RED_CELL.0, RED_CELL.1),
                vec![cannon()],
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(BLUE_CELL.0, BLUE_CELL.1),
                Vec::new(),
            ),
        ],
        &[(
            BLUE_CELL.0,
            BLUE_CELL.1,
            cell(mush_atlas::TerrainClass::Wall),
        )],
        &match_cfg(),
    );
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    let contact = hold(&mut arena, RED, BLUE);
    let before = arena
        .unit(RED)
        .map(|state| state.weapon_ammo("cannon"))
        .unwrap_or(0);
    pulse(
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
    // **Accepted** (doc 16 §3.5: *there is no free probe of line of sight*): the
    // round and the cycle are spent.
    let state = arena.unit(RED).unwrap_or_else(|| panic!("red"));
    assert_eq!(state.weapon_ammo("cannon"), before.saturating_sub(1));
    assert!(!state.weapon_ready("cannon", arena.now_ms));
    assert_eq!(arena.shots.len(), 1, "and it is in flight");
    // The resolution took the cover into account and came out at zero.
    assert_eq!(
        arena.shot(1).map(|shot| shot.resolution.p_permille),
        Some(0)
    );
    // And the impact says which of the six reasons it was.
    let mut wall_ms = wall + 500;
    while arena.now_ms < wall + 2_000 {
        wall_ms += 500;
        pulse(&mut arena, wall_ms, &[], &mut bytes);
    }
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "hit"), 0);
    assert_eq!(misses(&log), vec![damage::COVER.to_owned()]);
    assert!(
        arena.unit(BLUE).is_some_and(|state| state
            .unit
            .cell(&armor("hull", "rear"))
            .is_some_and(|cell| cell.current == 150)),
        "and nothing was scratched"
    );
}

// ---------------------------------------------------------------------------
// (d) A target that leaves the predicted cell misses `moved`
// ---------------------------------------------------------------------------

#[test]
fn a_target_that_leaves_the_predicted_cell_misses_moved_and_one_cell_is_still_a_hit() {
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    let contact = hold(&mut arena, RED, BLUE);
    pulse(
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
    let aimed = arena
        .shot(1)
        .and_then(|shot| shot.aim_cell)
        .unwrap_or_else(|| panic!("the shot names a cell"));
    assert_eq!(aimed, BLUE_CELL);
    // Four cells away, which is three cells past the one-cell slack.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(BLUE_CELL.0, BLUE_CELL.1 + 4);
    }
    let mut wall_ms = wall + 500;
    while !arena.shots.is_empty() && arena.now_ms < wall + 5_000 {
        wall_ms += 500;
        pulse(&mut arena, wall_ms, &[], &mut bytes);
    }
    let log = events(&mut arena, &mut bytes);
    assert_eq!(misses(&log), vec![damage::MOVED.to_owned()]);

    // Exactly one cell away is still within the prediction, and the round lands.
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    let contact = hold(&mut arena, RED, BLUE);
    pulse(
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
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose = centre(BLUE_CELL.0, BLUE_CELL.1 + 1);
    }
    let mut wall_ms = wall + 500;
    while !arena.shots.is_empty() && arena.now_ms < wall + 5_000 {
        wall_ms += 500;
        pulse(&mut arena, wall_ms, &[], &mut bytes);
    }
    let log = events(&mut arena, &mut bytes);
    assert!(
        misses(&log).is_empty(),
        "one cell of drift is inside the prediction: {:?}",
        misses(&log)
    );
    assert_eq!(count(&log, "hit"), 1, "and at 95 % it lands");
}

// ---------------------------------------------------------------------------
// (e) A target that turns re-resolves against the same draw
// ---------------------------------------------------------------------------

#[test]
fn a_target_that_turns_in_flight_re_resolves_against_the_same_draw() {
    let arena = duel();
    // The draw is a function of the shot's sequence and nothing else, which is
    // what "the same `hit` draw" means (doc 16 §3.5 "Draw labels").
    let u = u32::from(draw_permille(SEED, &DrawLabel::Hit { shot_seq: 1 }));
    // The rear at 1 km, and the same shot re-resolved against the flank: a wider
    // face, the same cone, the same roll.
    let rear = arena.resolve_shot(RED, BLUE, "cannon", 100_000, mush_atlas::Cover::None);
    let flank = arena.reresolve(BLUE, &rear, "left", 100_000);
    assert_eq!(rear.band, "rear");
    assert_eq!((rear.presented_width_cm, rear.p_permille), (350, 300));
    assert_eq!((flank.presented_width_cm, flank.p_permille), (700, 537));
    assert_eq!(flank.error_urad, rear.error_urad, "the same cone");
    // The roll is the same number at both probabilities, so the band decides the
    // outcome and the draw does not move.
    assert_eq!(
        arena.rounds_landed(1, 1, rear.p_permille),
        u32::from(u < rear.p_permille)
    );
    assert_eq!(
        arena.rounds_landed(1, 1, flank.p_permille),
        u32::from(u < flank.p_permille)
    );
    assert_eq!(arena.shot_draw(1, 1).0, "hit:1");

    // End to end: the round is fired at the rear and arrives at the flank, and the
    // `hit` line says which facet it passed.
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    let contact = hold(&mut arena, RED, BLUE);
    pulse(
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
    assert_eq!(
        arena.shot(1).map(|shot| shot.resolution.band.to_string()),
        Some("rear".to_owned())
    );
    // Blue turns to face south: red, due south of it, is now dead ahead.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose.heading_mdeg = 180_000;
    }
    let mut wall_ms = wall + 500;
    while !arena.shots.is_empty() && arena.now_ms < wall + 5_000 {
        wall_ms += 500;
        pulse(&mut arena, wall_ms, &[], &mut bytes);
    }
    let log = events(&mut arena, &mut bytes);
    let hit = log
        .iter()
        .find(|event| event.kind.as_str() == "hit")
        .cloned()
        .unwrap_or_else(|| panic!("the round landed: {:?}", misses(&log)));
    match hit.kind {
        EventKind::Hit { facet, .. } => assert_eq!(
            facet, "front",
            "the round arrived at the face the target was showing when it got there"
        ),
        other => panic!("not a hit: {other:?}"),
    }
    // 16b cut M.3 (16c `T-07`): the shot's own draw, then the facet's.
    assert_eq!(
        hit.draws
            .iter()
            .map(|(label, _)| label.as_str())
            .collect::<Vec<&str>>(),
        vec!["hit:1", "facet:1"]
    );
}

// ---------------------------------------------------------------------------
// (f) The facet draw lands inside the band's weights
// ---------------------------------------------------------------------------

#[test]
fn the_facet_draw_lands_inside_the_bands_authored_weights() {
    // The `left` band sends 40 % to the hull, 30 % to the near track and 30 % to
    // the turret (doc 16 §3.3's default table, as the content authors it). Over
    // 600 pinned seeds the frequencies should be near those, and the tolerance is
    // deliberately wide: the claim is that the weights are *read*, not that a hash
    // is a good RNG.
    let mut tally: BTreeMap<String, u32> = BTreeMap::new();
    let trials = 600_u64;
    let mut arena = duel();
    for trial in 1..=trials {
        // The seed is what the draw is keyed on, and the shot's sequence is the
        // other half; walking the seed is what gives 600 independent draws of the
        // *same* question. The multiplier keeps consecutive trials from sharing low
        // bits, and both are pinned, so this test is the same 600 draws every run.
        arena.seed = trial.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let (section, _) = arena
            .choose_section(BLUE, "left", 1)
            .unwrap_or_else(|| panic!("the left band has sections"));
        *tally.entry(section.to_string()).or_default() += 1;
    }
    let share = |name: &str| -> u32 {
        tally
            .get(name)
            .copied()
            .unwrap_or(0)
            .saturating_mul(100)
            .saturating_div(u32::try_from(trials).unwrap_or(1))
    };
    assert_eq!(
        tally.keys().cloned().collect::<Vec<_>>(),
        vec![
            "hull".to_owned(),
            "left_track".to_owned(),
            "turret".to_owned()
        ],
        "only the sections the band names were ever chosen"
    );
    assert!(
        (25..=55).contains(&share("hull")),
        "the hull's 40 % came out at {}",
        share("hull")
    );
    assert!(
        (15..=45).contains(&share("left_track")),
        "the near track's 30 % came out at {}",
        share("left_track")
    );
    assert!(
        (15..=45).contains(&share("turret")),
        "the turret's 30 % came out at {}",
        share("turret")
    );
}

// ---------------------------------------------------------------------------
// (g) The armour, the overflow, the consequence and the wreck
// ---------------------------------------------------------------------------

#[test]
fn the_armour_floors_the_overflow_reaches_the_internals_and_the_wreck_is_one_plan() {
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    let contact = hold(&mut arena, RED, BLUE);
    // Red is due south of blue, so every round arrives at the rear band: the
    // hull's rear facet is 150 and its internals 200, and every round is 120. So
    // the first round leaves 30 of armour, the second floors it and spends 90 of
    // overflow, and the third finishes the section. Take the turret out of the
    // table so every round goes to the hull and the arithmetic is a straight line.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.profile.bands = state
            .profile
            .bands
            .iter()
            .map(|band| BandProfile {
                sections: vec![(SmolStr::new("hull"), 1)],
                ..band.clone()
            })
            .collect();
    }
    let mut wall_ms = wall;
    let mut fired = 0_u32;
    while fired < 6 {
        wall_ms += 500;
        let ready = arena
            .unit(RED)
            .is_some_and(|state| state.weapon_ready("cannon", arena.now_ms));
        let intents = if ready {
            fired += 1;
            vec![
                intent(RED, IntentKind::Lock { contact }, u64::from(fired) * 2),
                intent(
                    RED,
                    IntentKind::Fire {
                        weapon: SmolStr::new("cannon"),
                        slot: None,
                    },
                    u64::from(fired) * 2 + 1,
                ),
            ]
        } else {
            Vec::new()
        };
        pulse(&mut arena, wall_ms, &intents, &mut bytes);
        // The lock outlives the contact's own time-to-live in this rig, because the
        // store is written by hand; refresh it so the shots keep being accepted.
        if let Some(held) = arena.contacts.get_mut(RED, contact) {
            held.last_seen_ms = arena.now_ms;
        }
        if arena.unit(BLUE).is_some_and(|state| state.limits.wreck) {
            break;
        }
    }
    // Let the last round arrive.
    for _ in 0..8 {
        wall_ms += 500;
        pulse(&mut arena, wall_ms, &[], &mut bytes);
    }
    let log = events(&mut arena, &mut bytes);
    let hits: Vec<(u32, u32, u32)> = log
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Hit {
                damage,
                armor_left,
                internals_left,
                ..
            } => Some((*damage, *armor_left, *internals_left)),
            _ => None,
        })
        .collect();
    assert!(
        hits.len() >= 3,
        "three rounds at 95 % should land: {hits:?}"
    );
    assert_eq!(
        hits.first().copied(),
        Some((120, 30, 200)),
        "the first round takes 120 of 150 and leaves the internals alone: {hits:?}"
    );
    // The armour never goes negative and never comes back.
    let armour: Vec<u32> = hits.iter().map(|(_, armor, _)| *armor).collect();
    assert!(
        armour.windows(2).all(|pair| pair[0] >= pair[1]),
        "the armour floors rather than going negative: {hits:?}"
    );
    assert_eq!(armour.last().copied(), Some(0));
    assert!(
        hits.iter().any(|(_, _, internals)| *internals == 110),
        "the second round's 90 of overflow reached the internals: {hits:?}"
    );
    assert_eq!(
        hits.last().map(|(_, _, internals)| *internals),
        Some(0),
        "and the last of them finished the section: {hits:?}"
    );
    // The section went, with its consequence, and the unit with it.
    assert_eq!(count(&log, "section_destroyed"), 1);
    assert_eq!(count(&log, "unit_destroyed"), 1);
    let destroyed = log
        .iter()
        .find(|event| event.kind.as_str() == "section_destroyed")
        .cloned()
        .unwrap_or_else(|| panic!("the hull went"));
    match destroyed.kind {
        EventKind::SectionDestroyed {
            unit,
            section,
            by,
            consequences,
        } => {
            assert_eq!(unit, BLUE);
            assert_eq!(section, "hull");
            // 16b cut D.4 (`T-08`): the round that did it is on the line.
            assert_eq!(by, Some(RED));
            assert_eq!(consequences, vec![SmolStr::new("destroy_unit")]);
        }
        other => panic!("not a section: {other:?}"),
    }
    // **One plan**: the wreck's condition and its revocation are in the same
    // mutation as the cells, so the unit has never been a wreck with its power on.
    let state = arena.unit(BLUE).unwrap_or_else(|| panic!("the wreck"));
    assert!(state.limits.wreck && state.limits.motion_disabled);
    assert_eq!(state.power, Power::Off);
    assert!(state.weapons.is_empty());
    assert!(
        !state
            .unit
            .has_capability(&CapabilityId::new("tacsim.power").expect("a capability")),
        "the wreck lost its power capability"
    );
    assert_eq!(
        state
            .unit
            .section(&SectionId::new("hull").expect("a section"))
            .map(|section| section.condition.as_str().to_owned()),
        Some(damage::WRECK_CONDITION.to_owned()),
        "and carries the condition doc 16 §3.3 makes a wreck"
    );
    // The predicate the world reads it through — the one `@unit/show`, a check-in and
    // cut W.3's story test ask.
    assert!(damage::is_wreck(&state.unit));
    assert!(
        arena
            .unit(RED)
            .is_some_and(|red| !damage::is_wreck(&red.unit)),
        "and the tank that did it is not one"
    );
    // A wreck is never a target again.
    assert_eq!(cell_of(&arena, BLUE, &internals("hull")), Some(0));
    // Every `hit` line carries its plan's cause, which is what ties it to the M32
    // journal entry the same plan wrote.
    let causes: Vec<String> = log
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Hit {
                cause, shot_seq, ..
            } => Some(format!("{cause}|{shot_seq}")),
            _ => None,
        })
        .collect();
    for entry in &causes {
        let (cause, seq) = entry.split_once('|').unwrap_or(("", ""));
        assert_eq!(cause, format!("shot:{seq}"), "the cause names the shot");
    }
}

// ---------------------------------------------------------------------------
// (h) A turret takes the weapons with it
// ---------------------------------------------------------------------------

#[test]
fn a_turret_shot_off_takes_the_weapons_and_an_immobile_disarmed_tank_is_disabled() {
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(RED_CELL.0, RED_CELL.1),
                vec![cannon()],
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(BLUE_CELL.0, BLUE_CELL.1),
                vec![machine_gun()],
            ),
        ],
        &[],
        &match_cfg(),
    );
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    let contact = hold(&mut arena, RED, BLUE);
    // Every round to the turret.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.profile.bands = state
            .profile
            .bands
            .iter()
            .map(|band| BandProfile {
                sections: vec![(SmolStr::new("turret"), 1)],
                ..band.clone()
            })
            .collect();
    }
    // The turret's rear facet is 140 and its internals 150: three rounds.
    let mut wall_ms = wall;
    for n in 0..4_u64 {
        wall_ms += 500;
        let ready = arena
            .unit(RED)
            .is_some_and(|state| state.weapon_ready("cannon", arena.now_ms));
        let intents = if ready {
            vec![
                intent(RED, IntentKind::Lock { contact }, n * 2 + 10),
                intent(
                    RED,
                    IntentKind::Fire {
                        weapon: SmolStr::new("cannon"),
                        slot: None,
                    },
                    n * 2 + 11,
                ),
            ]
        } else {
            Vec::new()
        };
        pulse(&mut arena, wall_ms, &intents, &mut bytes);
        if let Some(held) = arena.contacts.get_mut(RED, contact) {
            held.last_seen_ms = arena.now_ms;
        }
        for _ in 0..8 {
            wall_ms += 500;
            pulse(&mut arena, wall_ms, &[], &mut bytes);
            if let Some(held) = arena.contacts.get_mut(RED, contact) {
                held.last_seen_ms = arena.now_ms;
            }
        }
        if arena
            .unit(BLUE)
            .is_some_and(|state| state.weapons.is_empty())
        {
            break;
        }
    }
    let state = arena.unit(BLUE).unwrap_or_else(|| panic!("blue"));
    assert!(
        state.weapons.is_empty(),
        "the turret went and took the gun with it"
    );
    assert!(state.ready_at_ms.is_empty());
    // **16b cut F.1**: the rounds the gun still had are kept, because
    // `Arena::returned_units` is what folds them back into the unit's own `ammo`
    // cell at check-in — a barrel shot off half-empty used to come back full.
    assert!(
        !state.ammo.is_empty(),
        "the gone gun's rounds are still on the record for the check-in"
    );
    assert!(
        !state.limits.wreck,
        "and it is not a wreck: it can still drive"
    );
    assert!(!state.disabled, "nor disabled, for the same reason");
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "unit_disabled"), 0);
    // Now take its engine out, which is what makes it disabled.
    let mut journal = Vec::new();
    let now = arena.now_ms;
    arena.apply_consequence(BLUE, &Consequence::DisableMotion, now, None, &mut journal);
    arena.settle_disabled(BLUE, now, &mut journal);
    assert_eq!(arena.unit(BLUE).map(|state| state.disabled), Some(true));
    assert!(
        journal
            .iter()
            .any(|(_, kind, _)| matches!(kind, EventKind::UnitDisabled { .. })),
        "and it is said once: {journal:?}"
    );
    // Which is doc 16 §3.8's elimination, because blue was the whole blue side.
    assert_eq!(arena.eliminated(), Some(Some(Team::Red)));
}

// ---------------------------------------------------------------------------
// (i) Team elimination ends the match
// ---------------------------------------------------------------------------

#[test]
fn team_elimination_ends_the_match_with_the_winner() {
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    assert_eq!(arena.eliminated(), None, "both sides are standing");
    // Wreck the blue tank the way a hit would, and step once: phase 5 is where the
    // match ends.
    let mut journal = Vec::new();
    let now = arena.now_ms;
    arena.destroy_unit(BLUE, now, None, &mut journal);
    assert_eq!(arena.eliminated(), Some(Some(Team::Red)));
    let out = pulse(&mut arena, wall + 500, &[], &mut bytes);
    let end = out.ended.unwrap_or_else(|| panic!("the match ended"));
    assert_eq!(
        end.reason,
        EndReason::Elimination {
            winner: Some(Team::Red)
        }
    );
    let log = events(&mut arena, &mut bytes);
    assert!(
        log.iter().any(|event| matches!(
            event.kind,
            EventKind::MatchEnded {
                reason: EndReason::Elimination {
                    winner: Some(Team::Red)
                }
            }
        )),
        "and said so in the black box"
    );
    // A match whose last two units die together is a draw and not a red win.
    let mut arena = duel();
    let mut journal = Vec::new();
    arena.destroy_unit(RED, 0, None, &mut journal);
    arena.destroy_unit(BLUE, 0, None, &mut journal);
    assert_eq!(arena.eliminated(), Some(None));
}

// ---------------------------------------------------------------------------
// (j) The cross-tempo property, with the whole pipeline in it
// ---------------------------------------------------------------------------

/// One scripted beat, keyed by **arena** time, at a multiple of 5,000 ms.
struct Beat {
    t_ms: i64,
    intents: Vec<Intent>,
}

/// One ledger line, normalised for comparison: the timestamp, the kind as JSON,
/// the draw and the substep hash. The `seq` is dropped, as `arena.rs`'s own driver
/// drops it.
type Line = (i64, String, Vec<(String, u64)>, u64);

/// What one replay produced.
struct Replay {
    events: Vec<Line>,
    hashes: Vec<u64>,
    /// The sum of every `fired` line's `rounds` — what the black box says was spent.
    rounds_fired: u32,
    rounds_left: u32,
    armor_left: Option<i64>,
    internals_left: Option<i64>,
    /// How the match ended, if it did.
    ended: Option<EndReason>,
    pulses: u32,
    /// **Every notice line each pilot was sent, in order** (16b cut F.2) — the
    /// cross-tempo property read on the pilot's own surface rather than on the
    /// black box, which is the thing a person actually experiences.
    notices: BTreeMap<ObjId, Vec<String>>,
}

/// Drive `arena` through `script` at `cadence_ms`, with the tempo pinned and each
/// beat delivered in a **zero-length** pulse — W.1's rule, kept: phase 4 resolves a
/// discrete action at the arena time its pulse ended at, so a beat handed to a
/// pulse of no wall time resolves at exactly the millisecond the script names.
fn replay(arena: &mut Arena, cadence_ms: i64, until_ms: i64, script: &[Beat]) -> Replay {
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut wall = 0_i64;
    let mut next = 0_usize;
    let mut pulses = 0_u32;
    let mut ended = false;
    let mut notices: BTreeMap<ObjId, Vec<String>> = BTreeMap::new();
    let absorb_notices = |out: &StepOutput, into: &mut BTreeMap<ObjId, Vec<String>>| {
        for (pilot, frame) in &out.frames {
            let lines = tacsim::match_::cockpit::notice_lines(frame);
            if !lines.is_empty() {
                into.entry(*pilot).or_default().extend(lines);
            }
        }
    };
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
            absorb_notices(&out, &mut notices);
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
        absorb_notices(&out, &mut notices);
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
        ended: log.iter().rev().find_map(|event| match event.kind {
            EventKind::MatchEnded { reason } => Some(reason),
            _ => None,
        }),
        rounds_left: arena
            .unit(RED)
            .map(|state| state.weapon_ammo("cannon"))
            .unwrap_or(0),
        armor_left: cell_of(arena, BLUE, &armor("hull", "rear")),
        internals_left: cell_of(arena, BLUE, &internals("hull")),
        pulses,
        notices,
    }
}

/// Start up, hold the contact, then lock and fire four times at multiples of
/// 5,000 ms.
fn script() -> Vec<Beat> {
    let contact = contact_of(RED, BLUE);
    let mut beats = vec![Beat {
        t_ms: 0,
        intents: vec![intent(RED, IntentKind::Startup, 1)],
    }];
    for (n, at) in [15_000_i64, 20_000, 25_000, 30_000].into_iter().enumerate() {
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

/// The two tanks the script fights, with every round sent to the hull so the
/// arithmetic is a straight line, and blue's contact held from the first pulse so
/// the perception pass is not what the property is about.
fn scripted_arena() -> Arena {
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(RED_CELL.0, RED_CELL.1),
                vec![cannon()],
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(BLUE_CELL.0, BLUE_CELL.1),
                Vec::new(),
            ),
        ],
        &[],
        &MatchConfig::default(),
    );
    if let Some(state) = arena.unit_mut(BLUE) {
        state.profile.bands = state
            .profile
            .bands
            .iter()
            .map(|band| BandProfile {
                sections: vec![(SmolStr::new("hull"), 1)],
                ..band.clone()
            })
            .collect();
    }
    let contact = contact_of(RED, BLUE);
    arena.contacts.insert(
        RED,
        contact,
        ShadowContact {
            // Far enough ahead that the pass's time-to-live never expires it: the
            // property under test is the shot's, not the sensor's.
            last_seen_ms: i64::MAX / 4,
            own_seen: Some(OwnSighting {
                at_ms: i64::MAX / 4,
                plot: plot_of(&arena, BLUE),
                // 16b cut Q.2: no close look has been taken; this fixture is
                // about a shot and not about what a gunner can read.
                condition: None,
            }),
            ..ShadowContact::sighted(
                BLUE,
                Classification::Classified { team: Team::Blue },
                0,
                plot_of(&arena, BLUE),
            )
        },
    );
    arena
}

#[test]
fn one_scripted_engagement_at_three_cadences_is_one_hit_sequence() {
    let mut runs: Vec<(i64, Replay)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = scripted_arena();
        let run = replay(&mut arena, cadence, 45_000, &script());
        assert!(arena.shots.is_empty(), "every shot arrived at {cadence} ms");
        runs.push((cadence, run));
    }
    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };
    // The run is worth comparing, and what makes it worth comparing is that the
    // **script does not finish**: the rear hull is 150 of armour and 200 of
    // internals, every round is 120, so the third round kills the blue tank, the
    // blue side is eliminated and the fourth beat is never resolved. That is the
    // strongest form of the property — the tempo does not decide the *outcome* of
    // the match, never mind the outcome of a shot.
    let kinds: Vec<&str> = first
        .events
        .iter()
        .filter_map(|(_, encoded, _, _)| {
            ["fired", "impact", "hit", "miss"]
                .into_iter()
                .find(|word| encoded.contains(&format!("\"kind\":\"{word}\"")))
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            "fired", "impact", "hit", "fired", "impact", "hit", "fired", "impact", "hit",
        ],
        "three shots, three arrivals, three hits, in order: {kinds:?}"
    );
    assert_eq!(first.rounds_fired, 3);
    assert_eq!(first.rounds_left, 37, "three rounds of forty spent");
    assert_eq!(first.armor_left, Some(0), "the rear armour is gone");
    assert_eq!(first.internals_left, Some(0), "and so are the internals");
    assert_eq!(
        first.ended,
        Some(EndReason::Elimination {
            winner: Some(Team::Red)
        }),
        "the blue side was eliminated"
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
        assert_eq!(run.rounds_fired, first.rounds_fired);
        assert_eq!(run.rounds_left, first.rounds_left);
        assert_eq!(run.armor_left, first.armor_left, "the damage differs");
        assert_eq!(run.internals_left, first.internals_left);
        assert_eq!(run.ended, first.ended, "the match ended differently");
        assert_ne!(run.pulses, first.pulses, "{cadence} ms ran the same pulses");
    }
}

/// Ground no tracked unit may enter (doc 16 §3.4 step 4's *impassable*).
fn impassable() -> TerrainCell {
    TerrainCell {
        class: mush_atlas::TerrainClass::Wall,
        elev_cm: 0,
        move_cost: 0,
    }
}

/// A tank with a bank to run into and a pilot whose orders the arena refuses.
///
/// The two notices 16b cut F.2 produces **outside** the damage pipeline: the stop
/// is staged in the motion loop and the refusal in pulse phase 4, so neither is
/// covered by the duel's cross-tempo run, which never moves a tank and never has
/// an order rejected.
fn held_and_refused_arena() -> Arena {
    let bank: Vec<(u16, u16, TerrainCell)> = (0..SIDE).map(|y| (6, y, impassable())).collect();
    arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(RED_CELL.0, RED_CELL.1),
                vec![cannon()],
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(BLUE_CELL.0, BLUE_CELL.1),
                Vec::new(),
            ),
        ],
        &bank,
        &match_cfg(),
    )
}

/// Startup, then west into the bank, then two orders the arena refuses at apply:
/// a `FIRE` with no lock and a `LOCK` on an id the platform does not hold.
fn held_and_refused_script() -> Vec<Beat> {
    vec![
        Beat {
            t_ms: 0,
            intents: vec![intent(RED, IntentKind::Startup, 1)],
        },
        Beat {
            t_ms: 10_000,
            intents: vec![
                intent(RED, IntentKind::Heading { mdeg: 270_000 }, 2),
                intent(RED, IntentKind::Speed { cms: 1_000 }, 3),
            ],
        },
        Beat {
            t_ms: 20_000,
            intents: vec![intent(
                RED,
                IntentKind::Fire {
                    weapon: SmolStr::new("cannon"),
                    slot: None,
                },
                4,
            )],
        },
        Beat {
            t_ms: 25_000,
            intents: vec![intent(
                RED,
                IntentKind::Lock {
                    contact: ContactId(7),
                },
                5,
            )],
        },
    ]
}

#[test]
fn one_drive_into_a_bank_and_two_refused_orders_are_one_list_at_three_cadences() {
    // 16b §11 and doc 16 §3.1: the tempo decides nothing, including what a pilot
    // is told. The duel's own cross-tempo run covers the notices that come off the
    // journal scan; this one covers the other two — a stop, which is staged in the
    // motion loop because its hysteresis needs the pre-stop condition, and a
    // refusal, which is staged in phase 4. Since 16b cut M.3 (16c `T-04`)
    // `unit.held` is folded into the state hash, so the hash sequence also
    // catches a divergence in the stop; a refusal writes no hashed state at all,
    // so for it this list is still the only thing that can.
    let mut runs: Vec<(i64, Replay)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = held_and_refused_arena();
        runs.push((
            cadence,
            replay(&mut arena, cadence, 40_000, &held_and_refused_script()),
        ));
    }
    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };
    let told = first.notices.get(&RED_PILOT).cloned().unwrap_or_default();
    // The run is worth comparing: the tank stopped **once** against the bank and
    // was told so once however long it went on pushing, and both refusals arrived
    // in the cockpit's own words.
    assert_eq!(
        told.iter()
            .filter(|line| line.starts_with("Held:"))
            .collect::<Vec<&String>>(),
        vec![&"Held: the ground ahead will not take the tank.".to_owned()],
        "{told:?}"
    );
    assert!(
        told.contains(&"FIRE cannon refused. No lock: LOCK a contact first.".to_owned()),
        "{told:?}"
    );
    assert!(
        told.iter().any(|line| line.starts_with("LOCK refused.")
            && line.contains(tacsim::match_::cockpit::NO_SUCH_CONTACT)),
        "{told:?}"
    );
    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.notices, first.notices,
            "the notices differ at {cadence} ms"
        );
        assert_eq!(
            run.events, first.events,
            "the event list differs at {cadence} ms"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the per-substep hash sequence differs at {cadence} ms"
        );
    }
}

#[test]
fn a_spent_section_is_one_ledger_at_three_cadences() {
    // 16b cut F.2's item 3 changed what the black box records for a round that
    // arrives at a section with nothing left to break — one `section_destroyed`,
    // on the transition — so the change gets the cross-tempo cover every arena
    // behaviour gets.
    let mut runs: Vec<(i64, Replay)> = Vec::new();
    // **16b cut Q.6**: and what the shooter's own result row (doc 16 §4) counted,
    // which is read off the arena rather than off the black box — the two are
    // deliberately different numbers now.
    let mut counted: Vec<(u32, u32)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = spent_section_arena();
        let run = replay(&mut arena, cadence, 60_000, &spent_section_script());
        counted.push(
            arena
                .unit(RED)
                .map(|state| (state.shots_fired, state.hits))
                .unwrap_or_default(),
        );
        runs.push((cadence, run));
    }
    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };
    let destroyed = first
        .events
        .iter()
        .filter(|(_, encoded, _, _)| encoded.contains("\"kind\":\"section_destroyed\""))
        .count();
    let hits = first
        .events
        .iter()
        .filter(|(_, encoded, _, _)| encoded.contains("\"kind\":\"hit\""))
        .count();
    assert_eq!(destroyed, 1, "one destruction, whatever landed afterwards");
    // **16b cut Q.6**: every `hit` line **after** the destruction is a round that
    // arrived at a section already at zero armour and zero internals. The ledger
    // records each of them, truthfully — the round landed — and the shooter's
    // result row counts none of them, because it is what a pilot reads his own
    // gunnery off and nothing was taken off anything. `skip_while` stops **at**
    // the `section_destroyed` line, which sits after its own `hit`, so the round
    // that finished the turret is on the right side of the count.
    let spent = first
        .events
        .iter()
        .skip_while(|(_, encoded, _, _)| !encoded.contains("\"kind\":\"section_destroyed\""))
        .filter(|(_, encoded, _, _)| encoded.contains("\"kind\":\"hit\""))
        .count();
    assert!(
        spent >= 1,
        "no round landed on the spent turret: {hits} hits"
    );
    assert_eq!(
        counted.first().map(|row| row.1),
        Some(u32::try_from(hits.saturating_sub(spent)).unwrap_or(0)),
        "the result row counted a round that took nothing off anything"
    );
    assert_eq!(
        counted.first().map(|row| row.0),
        Some(8),
        "the eight shots the script fires are all accepted"
    );
    for (cadence, row) in [5_000_i64, 1_000, 500].iter().zip(counted.iter()).skip(1) {
        assert_eq!(
            Some(row),
            counted.first().as_ref().copied(),
            "the shooter's shots or hits differ at {cadence} ms"
        );
    }
    // 140 of rear armour and 150 of internals against a 120-round: the third
    // finishes the turret and the five after it arrive at a section with nothing
    // left to break. Before this cut each of those wrote a second destruction.
    assert!(
        hits >= 5,
        "the run has no rounds after the turret went: {hits}"
    );
    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events, first.events,
            "the event list differs at {cadence} ms"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the per-substep hash sequence differs at {cadence} ms"
        );
        assert_eq!(
            run.notices, first.notices,
            "the notices differ at {cadence} ms"
        );
    }
}

/// The scripted duel with every round sent to blue's **turret**, which the third
/// one finishes: the turret's consequence takes weapons blue does not have, so
/// blue lives and the rounds after it land on a spent section.
fn spent_section_arena() -> Arena {
    let mut arena = scripted_arena();
    if let Some(state) = arena.unit_mut(BLUE) {
        state.profile.bands = state
            .profile
            .bands
            .iter()
            .map(|band| BandProfile {
                sections: vec![(SmolStr::new("turret"), 1)],
                ..band.clone()
            })
            .collect();
    }
    arena
}

/// Eight beats of lock-and-fire, five seconds apart — past the three that finish
/// the turret.
fn spent_section_script() -> Vec<Beat> {
    let contact = contact_of(RED, BLUE);
    let mut beats = vec![Beat {
        t_ms: 0,
        intents: vec![intent(RED, IntentKind::Startup, 1)],
    }];
    for (n, at) in [
        15_000_i64, 20_000, 25_000, 30_000, 35_000, 40_000, 45_000, 50_000,
    ]
    .into_iter()
    .enumerate()
    {
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

// ---------------------------------------------------------------------------
// (n) What a pilot is told (16b cut F.2; doc 16 §5)
// ---------------------------------------------------------------------------

/// Every notice line one pilot was sent by one pulse, in order.
fn told(out: &StepOutput, pilot: ObjId) -> Vec<String> {
    out.frames
        .iter()
        .filter(|(who, _)| *who == pilot)
        .flat_map(|(_, frame)| tacsim::match_::cockpit::notice_lines(frame))
        .collect()
}

/// The frames one pilot was sent by one pulse — the same projection [`told`]
/// takes, stopped one step earlier, for the tests that read the wire payload a
/// panel decodes rather than the sentence a line client prints.
fn seen_by(out: &StepOutput, pilot: ObjId) -> Vec<tacsim::arena::step::Frame> {
    out.frames
        .iter()
        .filter(|(who, _)| *who == pilot)
        .map(|(_, frame)| frame.clone())
        .collect()
}

/// Warm the duel, hand red a contact on blue, and fire once — answering every
/// notice line each pilot was sent from the shot to its arrival.
fn one_shot(arena: &mut Arena, bytes: &mut Vec<u8>) -> BTreeMap<ObjId, Vec<String>> {
    let wall = warm(arena, bytes);
    let contact = hold(arena, RED, BLUE);
    let mut heard: BTreeMap<ObjId, Vec<String>> = BTreeMap::new();
    let mut wall_ms = wall + 500;
    let out = pulse(
        arena,
        wall_ms,
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
        bytes,
    );
    for pilot in [RED_PILOT, BLUE_PILOT] {
        heard.entry(pilot).or_default().extend(told(&out, pilot));
    }
    for _ in 0..8 {
        wall_ms += 500;
        let out = pulse(arena, wall_ms, &[], bytes);
        for pilot in [RED_PILOT, BLUE_PILOT] {
            heard.entry(pilot).or_default().extend(told(&out, pilot));
        }
        if let Some(held) = arena.contacts.get_mut(RED, contact) {
            held.last_seen_ms = arena.now_ms;
        }
    }
    heard
}

#[test]
fn the_shooter_is_told_the_round_landed_and_the_victim_where_it_landed() {
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let heard = one_shot(&mut arena, &mut bytes);
    let shooter = heard.get(&RED_PILOT).cloned().unwrap_or_default();
    let victim = heard.get(&BLUE_PILOT).cloned().unwrap_or_default();
    let contact = contact_of(RED, BLUE);

    // The shooter: the round landed, on the contact their own platform holds —
    // and, since **16b cut Q.2**, which section it found. The contact is
    // classified (`hold` plants it so), which is what earns the word; the section
    // is the turret for the reason the victim's line below is pinned to it.
    assert!(
        shooter.contains(&format!("Hit on {}: turret.", named(&arena, RED, BLUE))),
        "the shooter was told the round landed: {shooter:?}"
    );
    // And nothing about what it did. A gunner sees the strike, not the internals.
    for line in &shooter {
        assert!(
            !line.contains("armour") && !line.contains("facet") && !line.contains("internals"),
            "the shooter reads the target's damage: {line}"
        );
    }

    // The victim: their own section and facet, and what is left there.
    let struck = victim
        .iter()
        .find(|line| line.starts_with("Hit on the "))
        .cloned()
        .unwrap_or_else(|| panic!("the victim was told nothing: {victim:?}"));
    assert!(
        struck.contains("facet") && struck.contains("armour"),
        "{struck}"
    );
    // Red is due south of blue, so the round arrives at the rear band, whose
    // fixture weights are hull 40, engine 30 and turret 30. The facet draw is keyed
    // on the shot's own sequence against a fixed seed, so which of the three it
    // finds is not a coin toss: it is the turret, and pinning the one section is
    // what makes this assertion able to fail.
    assert!(struck.contains("turret"), "{struck}");

    // **The observer rule, sentence by sentence.** Nothing in anything the victim
    // was told identifies or locates the shooter: not its object number, not the
    // contact id blue holds it under, not a bearing, not a side, not a weapon.
    let blues_contact_on_red = contact_of(BLUE, RED);
    for line in &victim {
        if line.starts_with("New contact") || line.starts_with("Contact ") {
            // The contact lifecycle is cut S.2's surface and has its own test.
            continue;
        }
        for forbidden in [
            format!("#{}", RED.0),
            contact.to_string(),
            blues_contact_on_red.to_string(),
            "red".to_owned(),
            "bearing".to_owned(),
            "cannon".to_owned(),
            "°".to_owned(),
        ] {
            assert!(
                !line.contains(&forbidden),
                "a hit notice carries `{forbidden}`: {line}"
            );
        }
    }
}

#[test]
fn a_miss_carries_the_ledgers_own_reason_in_plain_words() {
    // The blue tank stands on a wall cell: doc 16 §3.5 accepts the shot, spends
    // the round and resolves it as a miss with the blocker as its reason.
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(RED_CELL.0, RED_CELL.1),
                vec![cannon()],
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(BLUE_CELL.0, BLUE_CELL.1),
                Vec::new(),
            ),
        ],
        &[(
            BLUE_CELL.0,
            BLUE_CELL.1,
            cell(mush_atlas::TerrainClass::Wall),
        )],
        &match_cfg(),
    );
    let mut bytes: Vec<u8> = Vec::new();
    let heard = one_shot(&mut arena, &mut bytes);
    let shooter = heard.get(&RED_PILOT).cloned().unwrap_or_default();
    assert!(
        shooter.contains(&format!(
            "Miss on {}: the target is behind cover.",
            named(&arena, RED, BLUE)
        )),
        "the shooter was told why: {shooter:?}"
    );
    // And the victim is told nothing at all: a miss is not a thing that happened
    // to them.
    let victim = heard.get(&BLUE_PILOT).cloned().unwrap_or_default();
    assert!(
        !victim.iter().any(|line| line.starts_with("Hit on")),
        "{victim:?}"
    );
    // The black box agrees with the line.
    assert_eq!(misses(&events(&mut arena, &mut bytes)), vec!["cover"]);
}

#[test]
fn a_section_is_destroyed_once_however_many_rounds_find_it_afterwards() {
    // Every round to the turret: 140 of rear armour and 150 of internals against
    // 120 a round, so the third finishes it and the turret's consequence takes
    // blue's (non-existent) weapons rather than the tank. Blue survives, so the
    // rounds after that land on a section with nothing left to break.
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    let contact = hold(&mut arena, RED, BLUE);
    if let Some(state) = arena.unit_mut(BLUE) {
        state.profile.bands = state
            .profile
            .bands
            .iter()
            .map(|band| BandProfile {
                sections: vec![(SmolStr::new("turret"), 1)],
                ..band.clone()
            })
            .collect();
    }
    let mut heard: Vec<String> = Vec::new();
    let mut shooter: Vec<String> = Vec::new();
    // The frames the victim was sent, for the wire half below (16b cut B.1).
    let mut victim: Vec<tacsim::arena::step::Frame> = Vec::new();
    let mut wall_ms = wall;
    let mut fired = 0_u32;
    while fired < 8 {
        wall_ms += 500;
        let ready = arena
            .unit(RED)
            .is_some_and(|state| state.weapon_ready("cannon", arena.now_ms));
        let intents = if ready {
            fired += 1;
            vec![
                intent(RED, IntentKind::Lock { contact }, u64::from(fired) * 2),
                intent(
                    RED,
                    IntentKind::Fire {
                        weapon: SmolStr::new("cannon"),
                        slot: None,
                    },
                    u64::from(fired) * 2 + 1,
                ),
            ]
        } else {
            Vec::new()
        };
        let out = pulse(&mut arena, wall_ms, &intents, &mut bytes);
        heard.extend(told(&out, BLUE_PILOT));
        shooter.extend(told(&out, RED_PILOT));
        victim.extend(seen_by(&out, BLUE_PILOT));
        if let Some(held) = arena.contacts.get_mut(RED, contact) {
            held.last_seen_ms = arena.now_ms;
        }
    }
    for _ in 0..12 {
        wall_ms += 500;
        let out = pulse(&mut arena, wall_ms, &[], &mut bytes);
        heard.extend(told(&out, BLUE_PILOT));
        shooter.extend(told(&out, RED_PILOT));
        victim.extend(seen_by(&out, BLUE_PILOT));
    }
    let log = events(&mut arena, &mut bytes);
    let hits = count(&log, "hit");
    assert!(
        hits >= 5,
        "the run has rounds after the turret went: {hits}"
    );
    assert_eq!(
        count(&log, "section_destroyed"),
        1,
        "the turret was destroyed once, not once per round that found it afterwards"
    );
    // And the victim was told once, in plain words.
    let lost: Vec<&String> = heard
        .iter()
        .filter(|line| line.starts_with("The turret is destroyed"))
        .collect();
    assert_eq!(
        lost,
        vec![&"The turret is destroyed: its weapons are gone.".to_owned()],
        "{heard:?}"
    );
    // The rounds that arrived afterwards are still hits, and still say so — a
    // round that finds nothing left to break is not a miss. **16b cut D.2**
    // (`B-15`) changed the words they say it in: only the rounds that found
    // armour say `armour gone`, and the rounds after the turret went say so in
    // their own sentence, once per [`SPENT_NOTICE_MS`]. Both halves are pinned
    // exactly, because the loose bound this assertion used to carry survived the
    // change and stopped covering the rounds it was written for.
    assert_eq!(
        heard
            .iter()
            .filter(|line| line.contains("armour gone"))
            .count(),
        2,
        "only the rounds that found armour say `armour gone`: {heard:?}"
    );
    assert!(
        !heard
            .iter()
            .any(|line| line.contains("armour gone, 0 of 0 inside")),
        "a round on a spent section is not reported as one that got through: {heard:?}"
    );
    assert_eq!(
        heard
            .iter()
            .filter(|line| *line == "Hit on the turret, which is already destroyed.")
            .count(),
        3,
        "the rounds after the turret went are one line per window: {heard:?}"
    );
    // **16b cut Q.2**: the shooter was told the turret went, once — the owner's
    // ask of 2026-09-19, which was that an attacker read nothing between
    // `Hit on …` and `… destroyed.` The line is the ledger's one
    // `section_destroyed` projected through the shooter's contact, so it cannot
    // be said once per round that lands on the spent section afterwards.
    let watched: Vec<&String> = shooter
        .iter()
        .filter(|line| line.ends_with("turret is destroyed."))
        .collect();
    assert_eq!(
        watched,
        vec![&format!(
            "{}'s turret is destroyed.",
            named(&arena, RED, BLUE)
        )],
        "{shooter:?}"
    );
    // The shooter was never told the *tank* was destroyed, because it was not.
    assert!(
        !shooter
            .iter()
            .any(|line| line.contains("destroyed") && !line.contains("turret")),
        "{shooter:?}"
    );
    // **16b cut B.1** (`T-86`): the same run on the wire, off the frames
    // `Arena::step` staged for the victim. On `struck`, `struck_spent` and
    // `section_lost` the published `section` is one of that same frame's own
    // `sections[].id`, and the notice names no contact, no designator and no old
    // name — so it is never a contact's section and nothing in it is the
    // shooter's.
    let mut published: BTreeMap<String, usize> = BTreeMap::new();
    for frame in &victim {
        let payload = tacsim::match_::cockpit::status_frame(tacsim::arena_task::MatchId(1), frame);
        let own: Vec<&serde_json::Value> = payload["sections"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|section| &section["id"])
            .collect();
        for notice in payload["notices"].as_array().into_iter().flatten() {
            let kind = notice["kind"].as_str().unwrap_or_default();
            if !["struck", "struck_spent", "section_lost"].contains(&kind) {
                continue;
            }
            assert!(
                notice["section"].is_string() && own.contains(&&notice["section"]),
                "{kind}'s section is not one of the victim's own {own:?}: {notice}"
            );
            for empty in ["contact", "designator", "was"] {
                assert!(notice[empty].is_null(), "{kind}.{empty}: {notice}");
            }
            *published.entry(kind.to_owned()).or_default() += 1;
        }
    }
    assert!(
        published.get("struck").is_some_and(|n| *n >= 2),
        "the rounds that found armour: {published:?}"
    );
    assert_eq!(published.get("struck_spent"), Some(&3), "{published:?}");
    assert_eq!(published.get("section_lost"), Some(&1), "{published:?}");
}

#[test]
fn the_frame_carries_the_tanks_own_status_block_and_status_prints_it_in_eighty_columns() {
    // Doc 16 §5's status block — *armor and internals per section as current over
    // max* — which is the whole of the recorded open point K7. Each number is
    // pinned against the cell it is read from, because a block that is merely
    // *present* proves nothing: `armor` and `armor_max` swapped, or a section
    // with no internals cell reporting a nought, would look exactly the same to a
    // test that only asked whether the word `hull` appeared.
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    let contact = hold(&mut arena, RED, BLUE);
    let whole = arena.frame_sections(BLUE);
    assert_eq!(
        whole
            .iter()
            .map(|section| section.id.as_str())
            .collect::<Vec<&str>>(),
        vec!["engine", "hull", "left_track", "right_track", "turret"],
        "every section, in section-id order"
    );
    let block = |id: &str| {
        whole
            .iter()
            .find(|section| section.id == id)
            .cloned()
            .unwrap_or_else(|| panic!("{id}'s block"))
    };
    let hull = block("hull");
    assert_eq!(hull.condition, "intact");
    assert_eq!(hull.internals, Some((200, 200)));
    assert_eq!(
        hull.facets
            .iter()
            .map(|facet| (facet.id.as_str(), facet.armor, facet.armor_max))
            .collect::<Vec<(&str, i64, i64)>>(),
        vec![
            ("front", 400, 400),
            ("left", 250, 250),
            ("rear", 150, 150),
            ("right", 250, 250),
            ("top", 100, 100),
        ],
        "the authored maxima, in facet-id order, current over max and not the \
         other way round"
    );
    let engine = block("engine");
    assert!(
        engine.facets.is_empty(),
        "the engine declares no armour of its own (doc 16 §3.3)"
    );
    assert_eq!(
        engine.internals,
        Some((120, 120)),
        "and still reports what it has inside"
    );
    for line in tacsim::match_::cockpit::section_lines(&whole) {
        assert!(
            line.chars().count() <= 80,
            "the block ran past eighty columns: {line:?}"
        );
    }

    // A section a unit declares no internals cell for reports **nothing** rather
    // than a nought that would read as destroyed, and a long facet run is wrapped
    // rather than allowed to run off an eighty-column terminal.
    let wide = tacsim::arena::step::FrameSection {
        id: SmolStr::new("hull"),
        condition: SmolStr::new("intact"),
        internals: None,
        facets: (0..9)
            .map(|n| tacsim::arena::step::FrameFacet {
                id: SmolStr::new(format!("facet_{n}")),
                armor: 1_000,
                armor_max: 1_000,
            })
            .collect(),
    };
    let lines = tacsim::match_::cockpit::section_lines(std::slice::from_ref(&wide));
    assert!(lines.len() > 2, "nine facets fit on one line: {lines:?}");
    assert!(
        !lines[0].contains("internals"),
        "a section with no internals cell printed one: {:?}",
        lines[0]
    );
    for line in &lines {
        assert!(
            line.chars().count() <= 80,
            "the wrap let a line past eighty columns: {line:?}"
        );
    }

    // And it **moves**, on the frame the pilot is actually sent: one 120-round
    // into blue's rear band takes 120 off the facet it found.
    let mut wall_ms = wall + 500;
    let mut sent: Vec<tacsim::arena::step::FrameSection> = Vec::new();
    let out = pulse(
        &mut arena,
        wall_ms,
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
    for (pilot, frame) in &out.frames {
        if *pilot == BLUE_PILOT {
            sent.clone_from(&frame.sections);
        }
    }
    for _ in 0..8 {
        wall_ms += 500;
        let out = pulse(&mut arena, wall_ms, &[], &mut bytes);
        for (pilot, frame) in &out.frames {
            if *pilot == BLUE_PILOT {
                sent.clone_from(&frame.sections);
            }
        }
        if let Some(held) = arena.contacts.get_mut(RED, contact) {
            held.last_seen_ms = arena.now_ms;
        }
    }
    assert_eq!(
        sent,
        arena.frame_sections(BLUE),
        "the frame's block is the cells' own, not a copy that drifted"
    );
    let struck: Vec<(&str, &str, i64, i64)> = sent
        .iter()
        .flat_map(|section| {
            section.facets.iter().map(|facet| {
                (
                    section.id.as_str(),
                    facet.id.as_str(),
                    facet.armor,
                    facet.armor_max,
                )
            })
        })
        .filter(|(_, _, armor, max)| armor < max)
        .collect();
    assert_eq!(
        struck.len(),
        1,
        "exactly one facet took the round: {struck:?}"
    );
    let (section, facet, armor, max) = struck[0];
    assert_eq!(
        armor,
        max - 120,
        "the cannon's 120 came off {section}'s {facet}: {armor} of {max}"
    );
    assert_eq!(facet, "rear", "red stands due south of blue");
}

#[test]
fn a_round_struck_through_another_sections_armour_quotes_that_sections_numbers() {
    // Doc 16 §3.3: the engine declares **no facets** and is *struck through*
    // `hull.rear`, so `plan_hit` spends the round on the hull's rear armour and
    // the `armor_left` the ledger carries is that cell's. The victim's line has to
    // read its maximum from the same cell: reading it at `engine/rear/armor` — an
    // address the tank does not declare — answered nought, and a pilot shot in the
    // back read `30 of 0 armour left`, a ratio with a zero denominator in the one
    // sentence this cut exists to deliver. Thirty per cent of every rear-band hit
    // on the stock tank picks the engine.
    let mut arena = duel();
    if let Some(state) = arena.unit_mut(BLUE) {
        state.profile.bands = state
            .profile
            .bands
            .iter()
            .map(|band| BandProfile {
                sections: vec![(SmolStr::new("engine"), 1)],
                ..band.clone()
            })
            .collect();
    }
    let mut bytes: Vec<u8> = Vec::new();
    let heard = one_shot(&mut arena, &mut bytes);
    let victim = heard.get(&BLUE_PILOT).cloned().unwrap_or_default();
    // 150 of rear armour, one 120-round: 30 left, of 150.
    assert!(
        victim.contains(
            &"Hit on the engine, through the hull's rear facet: 30 of 150 armour left.".to_owned()
        ),
        "{victim:?}"
    );
    for line in &victim {
        assert!(
            !line.contains("of 0 armour"),
            "a maximum of nought under a positive current: {line}"
        );
    }
    // And the sentence is about the cell the round really moved.
    assert_eq!(cell_of(&arena, BLUE, &armor("hull", "rear")), Some(30));
    assert_eq!(
        cell_of(&arena, BLUE, &internals("engine")),
        Some(120),
        "the armour held, so nothing overflowed into the engine"
    );
}

#[test]
fn each_weapon_carries_its_own_chance_and_the_lock_quotes_the_one_fire_would_take() {
    // 16b cut F.2's item 4, and the defect it exists to close: cut W.2 put **one**
    // number on the frame — the best of the platform's weapons — so a pilot who
    // read `90% to hit` and reached for the coaxial during the cannon's cycle
    // fired at a number twenty points lower. A one-weapon fixture cannot tell the
    // two sourcings apart, because the maximum over one weapon is that weapon.
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(RED_CELL.0, 1),
                vec![cannon(), machine_gun()],
            ),
            tank(BLUE, Team::Blue, centre(RED_CELL.0, 13), Vec::new()),
        ],
        &[],
        &match_cfg(),
    );
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    let contact = hold(&mut arena, RED, BLUE);
    let out = pulse(
        &mut arena,
        wall + 500,
        &[intent(RED, IntentKind::Lock { contact }, 3)],
        &mut bytes,
    );
    let frame = out
        .frames
        .iter()
        .rev()
        .find(|(pilot, _)| *pilot == RED_PILOT)
        .map(|(_, frame)| frame.clone())
        .unwrap_or_else(|| panic!("the red pilot's frame"));
    // What the arena would draw each of them against, taken the way a shot takes
    // it: one aim, then the five steps per weapon.
    let aim = arena
        .lock_aim(RED)
        .unwrap_or_else(|| panic!("the red tank has a lock"));
    let drawn_against = |weapon: &str| {
        arena
            .resolve_shot(RED, aim.target, weapon, aim.range_cm, aim.cover)
            .p_permille
    };
    let row = |id: &str| {
        frame
            .weapons
            .iter()
            .find(|weapon| weapon.id == id)
            .and_then(|weapon| weapon.p_permille)
            .unwrap_or_else(|| panic!("{id} has a chance against a held lock"))
    };
    assert_eq!(row("cannon"), drawn_against("cannon"));
    assert_eq!(row("machine_gun"), drawn_against("machine_gun"));
    // Twelve cells of it: the cannon's 3 mrad band against the coaxial's 8 puts
    // them two hundred per mille apart, which the platform's maximum hid.
    assert!(
        row("machine_gun") < row("cannon"),
        "the coaxial reads the cannon's chance: {} against {}",
        row("machine_gun"),
        row("cannon")
    );
    // Each row prints its own, and `LOCK` prints the one a bare `FIRE` would take
    // — `DEFAULT_WEAPON`, the cannon — and not the best of the two.
    let rows = tacsim::match_::cockpit::frame_weapons(&frame);
    for (id, permille) in [
        ("cannon", row("cannon")),
        ("machine_gun", row("machine_gun")),
    ] {
        let rendered = rows
            .iter()
            .find(|row| row.id == id)
            .map(|row| row.render(&rows))
            .unwrap_or_else(|| panic!("{id}'s row"));
        assert!(
            rendered.contains(&format!("{}% to hit", permille / 10)),
            "{rendered}"
        );
    }
    let line = tacsim::match_::cockpit::lock_line(&frame)
        .unwrap_or_else(|| panic!("a held lock has a line"));
    assert_eq!(
        line,
        // 16b cut Q.1: the short designator where the sixteen digits were.
        format!(
            "Locked on {}, {}% to hit.",
            named(&arena, RED, BLUE),
            row("cannon") / 10
        ),
        "`LOCK` quotes the weapon a bare `FIRE` fires"
    );
    // And the notice the arena staged when the lock resolved says the same thing.
    assert!(
        told(&out, RED_PILOT).contains(&line),
        "{:?}",
        told(&out, RED_PILOT)
    );
}

#[test]
fn two_locks_in_one_pulse_each_name_their_own_contact() {
    // `LOCK` is a discrete, queued intent, so a pilot who types two of them inside
    // one pulse period — 5,000 ms at `Quiet`, which is where a match sits before
    // the first shot — has both drained by phase 4. The frame carries the second
    // one's lock, so a line rendered off the frame would print the second id
    // twice and never mention the first.
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    let first = hold(&mut arena, RED, BLUE);
    let second = ContactId(first.0.wrapping_add(1));
    let now = arena.now_ms;
    arena.contacts.insert(
        RED,
        second,
        ShadowContact::sighted(
            BLUE,
            Classification::Classified { team: Team::Blue },
            now,
            plot_of(&arena, BLUE),
        ),
    );
    let out = pulse(
        &mut arena,
        wall + 500,
        &[
            intent(RED, IntentKind::Lock { contact: first }, 3),
            intent(RED, IntentKind::Lock { contact: second }, 4),
        ],
        &mut bytes,
    );
    let lines: Vec<String> = told(&out, RED_PILOT)
        .into_iter()
        .filter(|line| line.starts_with("Locked on "))
        .collect();
    assert_eq!(lines.len(), 2, "one line per lock taken: {lines:?}");
    // **16b cut Q.1**: the *identity* half of this property moved off the line
    // and onto the notice. Both of these contacts are held on the same tank, so
    // both legitimately render `BA` — a designator names the target, not the row
    // — and what the notice must still get right is which contact it was staged
    // for. That is the id it carries, and the two are distinct.
    let staged: Vec<ContactId> = out
        .frames
        .iter()
        .filter(|(pilot, _)| *pilot == RED_PILOT)
        .flat_map(|(_, frame)| frame.notices.iter())
        .filter(|notice| notice.kind() == tacsim::arena::perception::NoticeKind::Locked)
        .filter_map(tacsim::arena::perception::FrameNotice::contact)
        .collect();
    assert_eq!(
        staged,
        vec![first, second],
        "the first lock's notice named the second one's contact"
    );
    // The superseded lock is said bare: quoting a chance against a contact the
    // pilot has already stopped aiming at would be the same lie cut W.2's stale
    // frame told.
    let named = named(&arena, RED, BLUE);
    assert_eq!(lines[0], format!("Locked on {named}."));
    assert!(lines[1].contains("% to hit"), "{lines:?}");
}

#[test]
fn the_shooter_is_told_when_the_round_finishes_the_target() {
    let mut arena = scripted_arena();
    let run = replay(&mut arena, 1_000, 45_000, &script());
    let shooter = run.notices.get(&RED_PILOT).cloned().unwrap_or_default();
    let victim = run.notices.get(&BLUE_PILOT).cloned().unwrap_or_default();
    assert_eq!(
        run.ended,
        Some(EndReason::Elimination {
            winner: Some(Team::Red)
        }),
        "the script kills the blue tank"
    );
    assert!(
        // 16b cut Q.1: the kill line names the target by its public callsign,
        // which is what the shooter had it classified as.
        shooter.contains(&format!("{} destroyed.", callsign(&arena, BLUE))),
        "the shooter was told the target went: {shooter:?}"
    );
    // The victim's last word is their own hull, and it names the consequence.
    assert!(
        victim.contains(&"The hull is destroyed: the tank is finished.".to_owned()),
        "{victim:?}"
    );
    // **16b cut D.2** (`B-03`): and then the pilot's own one line, off the
    // ledger's `unit_destroyed`, which until this cut nobody was ever sent — a
    // pilot whose tank died simply stopped being told anything, which is three
    // of the six smoke matches. After that, nothing: a wreck's contact store is
    // emptied and its pilot is told no more (doc 16 §3.3).
    let after: Vec<&String> = victim
        .iter()
        .skip_while(|line| !line.starts_with("The hull is destroyed"))
        .skip(1)
        .collect();
    assert_eq!(
        after,
        vec![&tacsim::match_::cockpit::OWN_DESTROYED.to_owned()],
        "the wreck's pilot was told the wrong thing: {victim:?}"
    );
    // And it names nothing at all about the shooter.
    let callsign = callsign(&arena, RED);
    assert!(
        !tacsim::match_::cockpit::OWN_DESTROYED.contains(callsign.to_string().as_str()),
        "the victim's line names the shooter"
    );
}

#[test]
fn every_refusal_the_arena_makes_at_apply_reaches_the_pilot() {
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = warm(&mut arena, &mut bytes);
    let contact = hold(&mut arena, RED, BLUE);

    // (1) No lock.
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
            2,
        )],
        &mut bytes,
    );
    assert!(
        told(&out, RED_PILOT)
            .contains(&"FIRE cannon refused. No lock: LOCK a contact first.".to_owned()),
        "{:?}",
        told(&out, RED_PILOT)
    );

    // (2) A contact the platform does not hold — one byte-identical refusal.
    wall += 500;
    let out = pulse(
        &mut arena,
        wall,
        &[intent(
            RED,
            IntentKind::Lock {
                contact: ContactId(7),
            },
            3,
        )],
        &mut bytes,
    );
    let lines = told(&out, RED_PILOT);
    assert!(
        lines
            .iter()
            .any(|line| line.contains(tacsim::match_::cockpit::NO_SUCH_CONTACT)),
        "{lines:?}"
    );

    // (3) A weapon the tank does not carry.
    wall += 500;
    let out = pulse(
        &mut arena,
        wall,
        &[intent(
            RED,
            IntentKind::Fire {
                weapon: SmolStr::new("railgun"),
                slot: None,
            },
            4,
        )],
        &mut bytes,
    );
    assert!(
        told(&out, RED_PILOT)
            .iter()
            .any(|line| line
                == "FIRE railgun refused. That unit carries no weapon called `railgun`."),
        "{:?}",
        told(&out, RED_PILOT)
    );

    // (4) Two shots in one pulse: the second is refused for the cycle, with the
    // seconds the cockpit's own pre-check would have printed.
    wall += 500;
    let out = pulse(
        &mut arena,
        wall,
        &[
            intent(RED, IntentKind::Lock { contact }, 5),
            intent(
                RED,
                IntentKind::Fire {
                    weapon: SmolStr::new("cannon"),
                    slot: None,
                },
                6,
            ),
            intent(
                RED,
                IntentKind::Fire {
                    weapon: SmolStr::new("cannon"),
                    slot: None,
                },
                7,
            ),
        ],
        &mut bytes,
    );
    let lines = told(&out, RED_PILOT);
    assert!(
        lines.contains(&"FIRE cannon refused. Still cycling: 4 s to go.".to_owned()),
        "{lines:?}"
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("FIRE cannon refused"))
            .count(),
        1,
        "one refusal for one refused order: {lines:?}"
    );

    // (5) Out of rounds.
    if let Some(state) = arena.unit_mut(RED) {
        state.ammo.insert(SmolStr::new("cannon"), 0);
        state.ready_at_ms.insert(SmolStr::new("cannon"), 0);
    }
    wall += 500;
    if let Some(held) = arena.contacts.get_mut(RED, contact) {
        held.last_seen_ms = arena.now_ms;
    }
    let out = pulse(
        &mut arena,
        wall,
        &[intent(
            RED,
            IntentKind::Fire {
                weapon: SmolStr::new("cannon"),
                slot: None,
            },
            8,
        )],
        &mut bytes,
    );
    assert!(
        told(&out, RED_PILOT).contains(
            &"FIRE cannon refused. Empty: that weapon has nothing left to fire.".to_owned()
        ),
        "{:?}",
        told(&out, RED_PILOT)
    );

    // (6) Power off, which is the refusal doc 16 §3.7 puts last.
    if let Some(state) = arena.unit_mut(RED) {
        state.ammo.insert(SmolStr::new("cannon"), 10);
        state.power = Power::Off;
    }
    wall += 500;
    if let Some(held) = arena.contacts.get_mut(RED, contact) {
        held.last_seen_ms = arena.now_ms;
    }
    let out = pulse(
        &mut arena,
        wall,
        &[intent(
            RED,
            IntentKind::Fire {
                weapon: SmolStr::new("cannon"),
                slot: None,
            },
            9,
        )],
        &mut bytes,
    );
    assert!(
        told(&out, RED_PILOT)
            .contains(&"FIRE cannon refused. Power not online: STARTUP first.".to_owned()),
        "{:?}",
        told(&out, RED_PILOT)
    );
}

#[test]
fn an_order_the_arena_never_queued_and_a_shot_the_controller_took_are_both_silent() {
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let mut wall = warm(&mut arena, &mut bytes);

    // Doc 16 §3.1's burst: the `Control` is applied in phase 1 and raises the
    // epoch, so the `FIRE` behind it is refused at apply and **never queued**.
    // The pilot who let the seat go is told nothing about the shot they did not
    // take, and that is by design.
    wall += 500;
    let out = arena.step(
        wall,
        &[Control {
            unit: RED,
            channel: tacsim::arena::state::Channel::Fire,
            epoch: 2,
            state: tacsim::arena::intents::ControlState::Live,
            pilot: None,
        }],
        &[intent(
            RED,
            IntentKind::Fire {
                weapon: SmolStr::new("cannon"),
                slot: None,
            },
            2,
        )],
    );
    absorb(&out, &mut bytes);
    assert!(
        !told(&out, RED_PILOT)
            .iter()
            .any(|line| line.contains("refused")),
        "{:?}",
        told(&out, RED_PILOT)
    );

    // And the shape the reflex controller resolves in — straight into
    // `resolve_fire`, inside a substep, never through pulse phase 4 — stages
    // nothing for anybody.
    assert!(arena.resolve_fire(RED, "cannon", arena.now_ms).is_err());
    wall += 500;
    let out = pulse(&mut arena, wall, &[], &mut bytes);
    assert!(
        !told(&out, RED_PILOT)
            .iter()
            .any(|line| line.contains("refused")),
        "{:?}",
        told(&out, RED_PILOT)
    );
}

#[test]
fn the_percentage_is_printed_when_the_lock_resolves() {
    // Doc 16 §3.5 step 5: *the cockpit shows `p_permille` ... on `LOCK` before any
    // shot*. It resolves in the arena a pulse after the verb, so the line is a
    // notice at the lock's own arena timestamp — which is what the first `LOCK`
    // never printed.
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    let contact = contact_of(RED, BLUE);
    let out = pulse(
        &mut arena,
        wall + 500,
        &[intent(RED, IntentKind::Lock { contact }, 3)],
        &mut bytes,
    );
    let lines = told(&out, RED_PILOT);
    assert!(
        lines.contains(&format!(
            "Locked on {}, 95% to hit.",
            named(&arena, RED, BLUE)
        )),
        "the first lock prints the chance: {lines:?}"
    );
}

#[test]
fn one_scripted_duel_at_three_cadences_tells_each_pilot_one_list_of_lines() {
    let mut runs: Vec<(i64, Replay)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = scripted_arena();
        runs.push((cadence, replay(&mut arena, cadence, 45_000, &script())));
    }
    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };
    // The run is worth comparing: both pilots were told several things.
    assert!(
        first
            .notices
            .get(&RED_PILOT)
            .is_some_and(|lines| lines.len() >= 3),
        "{:?}",
        first.notices
    );
    assert!(
        first
            .notices
            .get(&BLUE_PILOT)
            .is_some_and(|lines| lines.len() >= 3),
        "{:?}",
        first.notices
    );
    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.notices, first.notices,
            "the notices differ at {cadence} ms"
        );
    }
}

// ---------------------------------------------------------------------------
// (k) Conservation, with hits in it
// ---------------------------------------------------------------------------

#[test]
fn ammunition_is_conserved_across_a_scripted_engagement() {
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
    let run = replay(&mut arena, 1_000, 45_000, &script());
    // Every `fired` line is one pull and its `rounds` field is what that pull spent,
    // so the sum of them plus what is left is the load — no round is invented and
    // none is lost, however the engagement ended.
    assert_eq!(run.rounds_fired, 3, "three pulls of one round");
    assert_eq!(
        run.rounds_fired.saturating_add(run.rounds_left),
        load,
        "rounds fired plus rounds left is the load"
    );
    // And the live cell agrees with the black box.
    assert_eq!(
        arena.unit(RED).map(|state| state.weapon_ammo("cannon")),
        Some(load.saturating_sub(run.rounds_fired))
    );
    // A burst conserves too: ten rounds a pull, and what lands is a sample of the
    // ten and never more.
    let arena = duel();
    for seq in 1..20_u64 {
        assert!(
            arena.rounds_landed(seq, 10, 500) <= 10,
            "a burst of ten cannot land eleven"
        );
    }
    assert_eq!(arena.rounds_landed(1, 10, 1_000), 10);
    assert_eq!(arena.rounds_landed(1, 10, 0), 0);
}

// ---------------------------------------------------------------------------
// (l) The check-in's shape
// ---------------------------------------------------------------------------

#[test]
fn the_returned_units_carry_their_cells_their_poses_and_their_outcomes() {
    let mut arena = duel();
    let mut journal = Vec::new();
    // One unit wrecked, one intact, and a counter on each.
    if let Some(state) = arena.unit_mut(RED) {
        state.shots_fired = 5;
        state.hits = 2;
        state.ammo.insert(SmolStr::new("cannon"), 35);
    }
    // **16b cut F.1**: and the unit about to be destroyed has fired too. Before
    // this cut `destroy_unit` cleared the `ammo` table, so a tank killed at round
    // twelve checked in with a full magazine — the readiness review found exactly
    // that on the real binary. Both sides of this test carry spent rounds now, so
    // the wreck's half is a claim and not an accident of the fixture.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.ammo.insert(SmolStr::new("cannon"), 12);
    }
    arena.destroy_unit(BLUE, 1_000, None, &mut journal);
    let returned: Vec<UnitReturn> = arena.returned_units();
    assert_eq!(returned.len(), 2);
    let red = returned
        .iter()
        .find(|back| back.unit == RED)
        .cloned()
        .unwrap_or_else(|| panic!("red came back"));
    let blue = returned
        .iter()
        .find(|back| back.unit == BLUE)
        .cloned()
        .unwrap_or_else(|| panic!("blue came back"));
    assert_eq!((red.shots, red.hits), (5, 2));
    assert!(!red.wreck && !red.disabled);
    assert!(blue.wreck, "the wreck comes back a wreck");
    // `destroy_unit` here is the arena's half only — the M32 mark is the impact
    // plan's — so the returned cells carry the flag and the condition is the impact's
    // to write (test (g) is where the two meet).
    assert!(!damage::is_wreck(&red.cells));
    assert_eq!(
        (red.pose.x_cm, red.pose.y_cm),
        (8_500, 5_500),
        "and at the pose the arena left it"
    );
    // The rounds went home.
    let ammo = CellAddr::Component {
        component: ComponentId::new("cannon").expect("a component"),
        cell: CellId::new("ammo").expect("a cell"),
    };
    assert_eq!(red.cells.cell(&ammo).map(|cell| cell.current), Some(35));
    // For the wreck too: a destroyed tank's rounds are the check-in's evidence,
    // exactly as the survivor's are.
    assert_eq!(
        blue.cells.cell(&ammo).map(|cell| cell.current),
        Some(12),
        "the destroyed tank checked in with a magazine it had already spent"
    );

    // And the plan the world applies is the **diff** against the unit it still
    // holds: nothing for a unit that did not change, and exactly the cells,
    // conditions and revocations that did.
    let held = line_tank_unit(RED);
    assert!(
        tacsim::match_::returned_plan(&held, &held).is_none(),
        "an unchanged unit needs no plan"
    );
    let plan = tacsim::match_::returned_plan(&held, &red.cells)
        .unwrap_or_else(|| panic!("the spent rounds are a change"));
    assert_eq!(plan.ops().len(), 1, "one cell moved: {:?}", plan.ops());
    assert_eq!(plan.cause(), Some("arena:checkin"));
    // Applying it really lands the arena's number.
    let (post, _) = mush_tactics::apply(&held, &plan, 0)
        .unwrap_or_else(|error| panic!("the returned cells apply: {error}"));
    assert_eq!(post.cell(&ammo).map(|cell| cell.current), Some(35));
}

// ---------------------------------------------------------------------------
// (m) The money rule
// ---------------------------------------------------------------------------

mod money {
    //! **No instrument moves while a match runs** (16b §8's cut W verify list; doc
    //! 16 §3.1: *the world task remains the single writer for … the economy*).
    //!
    //! The only test in this file that needs a world, because the claim is about
    //! one: a match is fought on the production composition root, from `SIM TEAM`
    //! through `Running` to the check-in, and every account's posted balance and
    //! every instrument is compared before and after. The arena cannot reach the
    //! store at all (`crates/mush-contract-tests/tests/arena_library.rs` is the
    //! gate), so what this adds is the **world** half: the check-in applies cells
    //! and poses and writes a result, and none of that is a transaction.

    use std::collections::BTreeMap;
    use std::sync::Arc;

    use mush_core::{
        AccountId, AccountIdentity, Audience, Ctx, GOD, ObjId, Outbox, ParsedArgs, SwitchSet,
    };
    use mush_proto::OutputEvent;
    use mush_testkit::TestWorld;
    use tacsim::TacsimConfig;
    use tacsim::match_::{self, MatchState};

    /// A fast arena and a short boarding window, so the match runs in
    /// milliseconds.
    fn config() -> TacsimConfig {
        let mut cfg = TacsimConfig {
            enabled: true,
            ..TacsimConfig::default()
        };
        cfg.arena.tempo.quiet_ms = 100;
        cfg.arena.tempo.alert_ms = 100;
        cfg.arena.tempo.engaged_ms = 100;
        cfg.match_.startup_ms = 200;
        cfg.match_.board_timeout_ms = 5_000;
        cfg
    }

    /// The production roster, as `match_lifecycle.rs` composes it (the sheet is
    /// left out for its reason: a job-payload handler cannot be registered twice
    /// in one process).
    fn roster() -> Vec<Box<dyn mush_core::GameModule>> {
        use tacsim::composition as comp;

        let llm = Arc::new(
            mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
                .expect("a disabled LLM service composes"),
        );
        let onboarding = mush_core::OnboardingConfig::default();
        let game = tacsim::GameEconomyConfig::default();
        let cfg = config();
        let mut mods = comp::server_modules_with(
            mush_weather::WeatherConfig::default(),
            mush_economy::EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &onboarding,
            &game,
            comp::recovery_policy(&cfg.recovery),
        );
        mods.push(comp::worldgen_module_with(
            Arc::clone(&llm),
            mush_worldgen::WorldgenConfig::default(),
        ));
        mods.push(comp::npc_module_with(
            comp::npc_agent_adapter(),
            &mush_config::config::NpcCfg::default(),
        ));
        mods.push(comp::tacsim_module(cfg));
        mods.push(comp::content_module_with(true, true).expect("the content module composes"));
        mods
    }

    /// The boot-hydrated character → account map.
    #[derive(Default)]
    struct Accounts(BTreeMap<ObjId, AccountId>);

    impl AccountIdentity for Accounts {
        fn account_of(&self, character: ObjId) -> Option<AccountId> {
            self.0.get(&character).copied()
        }
    }

    /// Everything the economy holds, as one comparable fingerprint: the journal's
    /// own length, every instrument, and every account's posted and held amounts.
    ///
    /// The journal length is the load-bearing half. Doc 12 makes the double-entry
    /// journal the economy's only authority — a balance is a projection of it — so
    /// "no instrument moved" is exactly "no entry was appended", and an
    /// implementation that moved money and then moved it back would still fail here.
    fn money(tw: &TestWorld) -> Vec<String> {
        let Some(store) = tw.world().module_state::<mush_economy::EconomyStore>() else {
            return vec!["no economy at all".to_owned()];
        };
        let snapshot = store.snapshot();
        let mut out: Vec<String> = vec![format!("journal {}", store.journal_len())];
        out.extend(
            snapshot
                .instruments()
                .map(|instrument| format!("instrument {}", instrument.id)),
        );
        out.extend(snapshot.accounts().map(|account| {
            format!(
                "account {} {} posted={:?} held={:?}",
                account.id, account.instrument, account.posted_balance, account.held_amount
            )
        }));
        out.sort();
        out
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn no_instrument_moves_while_a_match_runs() {
        let mut tw = TestWorld::with_modules(roster());
        let lobby = match_::centre_object(tw.world(), match_::LOBBY_ENTRY)
            .expect("the OOC pack carries the simulator lobby");
        let mut map = Accounts::default();
        let mut pilots = Vec::new();
        for n in 0..2 {
            let pilot = tw.spawn_player(&format!("Pilot{n}"));
            tw.world_mut()
                .move_to(pilot, lobby)
                .expect("a player walks into the lobby");
            map.0.insert(
                pilot,
                AccountId::mint(&format!("account-{n}"), i64::from(n)).expect("an account id"),
            );
            pilots.push(pilot);
        }
        *tw.world_mut()
            .module_state_mut::<mush_sim::AccountOracle>()
            .expect("the simulation is composed") = mush_sim::AccountOracle::new(Arc::new(map));
        let mut egress = tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .map(|registry| registry.tasks().clone())
            .and_then(|tasks| tasks.take_egress());

        // Selection → Boarding → Running, through the real verbs.
        let say = |tw: &mut TestWorld, actor: ObjId, line: &str| -> String {
            let events: Vec<(Audience, OutputEvent)> = tw.run(actor, line);
            tw.seen_by(&events, actor).join("\n")
        };

        // **Something to be unchanged.** This roster leaves the character sheet out
        // (see `roster`), and the sheet is what registers the game's own credit at
        // boot — so the fingerprint would otherwise be one empty list compared with
        // another, which is not a test of anything. One instrument and one account
        // per pilot, through the real staff verbs, and the comparison has teeth.
        say(&mut tw, GOD, "@instrument/register cr=2");
        for pilot in &pilots {
            say(&mut tw, GOD, &format!("@account/open #{}=cr", pilot.0));
        }
        let before = money(&tw);
        assert!(
            before.len() >= 3,
            "one instrument, two accounts and the journal: {before:?}"
        );
        say(&mut tw, pilots[0], "sim team red");
        say(&mut tw, pilots[1], "sim team blue");
        let said = say(&mut tw, pilots[0], "sim start");
        assert!(said.contains("boarding"), "{said}");
        for pilot in &pilots {
            let pod = match_::pod_of(tw.world(), *pilot).expect("the pod is locked");
            let unit = match_::tank_of(tw.world(), pod).expect("the pod's tank");
            let match_id = match_::centre_of(tw.world())
                .and_then(|centre| centre.match_id)
                .expect("a boarding centre has a match id");
            let mut out = Outbox::new();
            let engine = tw.engine_mut();
            let mut ctx = Ctx {
                w: &mut engine.world,
                actor: *pilot,
                switches: SwitchSet::default(),
                args: ParsedArgs::default(),
                out: &mut out,
                reg: &engine.registry,
                module: Some(smol_str::SmolStr::new(tacsim::NS)),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            let _ = match_::slot_seated(&mut ctx, match_id, *pilot, unit)
                .expect("the seat is reported");
        }
        assert_eq!(
            match_::centre_of(tw.world()).map(|centre| centre.state),
            Some(MatchState::Running),
            "the trigger fired"
        );

        // Let the arena run, then abort and let the check-in land. The world loop's
        // `Wake::ArenaEgress` arm, written out, as `match_lifecycle.rs` writes it.
        let pump = |tw: &mut TestWorld,
                    egress: &mut Option<
            tokio::sync::mpsc::Receiver<tacsim::arena_task::ArenaOut>,
        >| {
            let Some(tasks) = tw
                .world()
                .module_state::<tacsim::arena_bridge::ArenaRegistry>()
                .map(|registry| registry.tasks().clone())
            else {
                return;
            };
            let mut staged = 0_usize;
            if let Some(rx) = egress.as_mut() {
                while staged < tacsim::arena_bridge::MAX_EGRESS_PER_DRAIN {
                    match rx.try_recv() {
                        Ok(message) => {
                            if !tasks.stage(message) {
                                break;
                            }
                            staged += 1;
                        }
                        Err(_) => break,
                    }
                }
            }
            if staged == 0 {
                return;
            }
            tw.apply(mush_core::WorldCommand::ModuleAsyncDone {
                module: smol_str::SmolStr::new_static(tacsim::arena_bridge::MODULE),
                actor: GOD,
                correlation: 0,
                payload: tacsim::arena_bridge::EGRESS_PAYLOAD.to_owned(),
            });
            tasks.stamp_ledger_writes(1);
            tasks.ack_ledger_upto(1);
        };

        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            pump(&mut tw, &mut egress);
        }
        say(&mut tw, pilots[1], "sim abort");
        for _ in 0..400 {
            if match_::centre_of(tw.world()).map(|centre| centre.state) == Some(MatchState::Aborted)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            pump(&mut tw, &mut egress);
        }
        assert_eq!(
            match_::centre_of(tw.world()).map(|centre| centre.state),
            Some(MatchState::Aborted),
            "the check-in landed"
        );

        // **The money rule**: not one instrument, not one balance.
        assert_eq!(
            money(&tw),
            before,
            "a match moved money: doc 16 §3.1 makes the world the single writer of \
             the economy and a match is not a transaction"
        );
    }
}

/// **The advertised chance is read off the plot, and the shot off the tank**
/// (16b cut F.3).
///
/// `LOCK`'s sentence and every `WEAPONSTATUS` row quote one number, and it is
/// advice: every target-side input of it — the range, the cover, the face the
/// target is showing and the speed it is crossing at — comes off the contact's
/// own plot. Read live, a tank that turned or accelerated while nobody could see
/// it would move that number every pulse, `FrameWeapon::same_as` would report the
/// difference, and the pilot would be pushed a frame each time: an unsighted
/// target's manoeuvre, read off a platform that has detected nothing since.
#[test]
fn the_advertised_chance_is_read_off_the_plot_and_the_shot_off_the_tank() {
    let mut arena = duel();
    let contact = hold(&mut arena, RED, BLUE);
    if let Some(state) = arena.unit_mut(RED) {
        state.lock = Some(contact);
    }
    // At the range the file's own pinned row uses — 1 km, where the rear's 350 cm
    // is 3,500 µrad against a 4,000 µrad cone and the curve answers 300 — rather
    // than the fixture's four cells, where both clamps swallow every difference.
    let mut aim = arena
        .lock_aim(RED)
        .unwrap_or_else(|| panic!("the red tank has a lock"));
    aim.range_cm = 100_000;
    let advice = arena.resolve_seen(RED, &aim, "cannon");
    assert_eq!(
        (advice.band.as_str(), advice.error_urad, advice.p_permille),
        ("rear", 4_000, 300),
        "the plot has blue facing north and standing still"
    );
    let before = arena.frame_weapons(RED, Some(&aim));

    // Blue swings east and accelerates across the line of fire. Nothing has
    // sighted it since, so the plot — and everything read off it — is unmoved.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose.heading_mdeg = 90_000;
        state.speed_cms = 1_500;
    }
    let after = arena.resolve_seen(RED, &aim, "cannon");
    assert_eq!(
        (after.band.as_str(), after.error_urad, after.p_permille),
        (advice.band.as_str(), advice.error_urad, advice.p_permille),
        "the advertised chance moved with a tank nobody has seen"
    );
    let rows = arena.frame_weapons(RED, Some(&aim));
    assert!(
        before
            .iter()
            .zip(&rows)
            .all(|(was, now)| was.same_as(now) && now.p_permille == Some(300)),
        "the pilot would have been pushed a frame for a manoeuvre they cannot see"
    );

    // And the fixture really can tell the two apart: the *shot* is resolved
    // against the tank that is there, so its band and its cone both moved.
    let taken = arena.resolve_shot(RED, BLUE, "cannon", aim.range_cm, aim.cover);
    assert_ne!(taken.band.as_str(), "rear", "blue turned its flank");
    assert!(
        taken.error_urad > advice.error_urad,
        "blue is crossing at 15 m/s: {} against {}",
        taken.error_urad,
        advice.error_urad
    );
}

// ---------------------------------------------------------------------------
// (n8) the cross-tempo property, with the gunner's words and the readings in it
//      — 16b cut Q.2
//
// 16b §11 makes this mandatory for every cut that changes arena behaviour, and
// this cut changes five things the property is about: the section word on a hit,
// the `target_section_lost` line, the reading a close look records, the reading a
// share carries, and the band the gunner's own round writes in. All five are
// folded into the state hash through the contact store, and all five are things a
// pilot reads.
//
// So the run below is scripted to contain all five at once — a shooter, a BLIND
// teammate that can only ever learn what the mesh hands it, and a target whose
// turret is taken apart round by round while both of them watch.
// ---------------------------------------------------------------------------

/// **The blind teammate** (16b cut Q.2): a red tank with no sensor of its own, so
/// everything on its scope arrived over the mesh and a reading it holds can only
/// be the one the shooter's own sight took.
const MATE: ObjId = ObjId(13);

/// Its pilot. `arena_of` seats `ObjId(1000 + n)` in the order the units are
/// handed over, so the trio below is red, blue, mate.
const MATE_PILOT: ObjId = ObjId(1_002);

/// The shooter's sight, with enough reach for the duel's forty metres to be both
/// a detection and a **close look**, and a mesh short enough that only the tank
/// standing beside it hears anything.
///
/// `sight()` reaches five cells, which is fifty metres: the duel is inside it but
/// the close look — half the effective reach, `condition_reach_permille` — is
/// twenty-five, so nothing would ever be read. Ten cells makes the close look
/// fifty metres and the duel's forty a look.
fn scan_sight() -> SensorProfile {
    SensorProfile {
        max_range_cells: 10,
        share_range_cells: 5,
        ..sight()
    }
}

/// **Two red tanks and a blue one**, with every round sent to the blue turret.
///
/// The turret and not the hull, for the reason
/// `a_section_is_destroyed_once_however_many_rounds_find_it_afterwards` picks it:
/// 140 of rear armour and 150 of internals against 120 a round, so the third
/// round finishes it and its consequence takes blue's (non-existent) weapons
/// rather than the tank. The match therefore runs on past the section that went,
/// which is what lets the checkpoints below sit at fixed arena timestamps instead
/// of at whatever moment a kill happened to end the run.
fn meshed_trio() -> Arena {
    let mut arena = arena_of(
        vec![
            CustodyUnit {
                sensor: Some(scan_sight()),
                ..tank(
                    RED,
                    Team::Red,
                    centre(RED_CELL.0, RED_CELL.1),
                    vec![cannon()],
                )
            }
            .sealed(),
            tank(
                BLUE,
                Team::Blue,
                centre(BLUE_CELL.0, BLUE_CELL.1),
                Vec::new(),
            ),
            CustodyUnit {
                // Blind: it runs a pass (that is where its contacts expire) and
                // detects nothing, so its scope is the mesh and nothing else.
                sensor: None,
                ..tank(
                    MATE,
                    Team::Red,
                    centre(RED_CELL.0 - 1, RED_CELL.1),
                    Vec::new(),
                )
            }
            .sealed(),
        ],
        &[],
        &match_cfg(),
    );
    if let Some(state) = arena.unit_mut(BLUE) {
        state.profile.bands = state
            .profile
            .bands
            .iter()
            .map(|band| BandProfile {
                sections: vec![(SmolStr::new("turret"), 1)],
                ..band.clone()
            })
            .collect();
    }
    arena
}

/// The scripted log: all three tanks start up, and the shooter locks and fires
/// four times.
///
/// **Every beat is at a multiple of five seconds of arena time**, which is a pulse
/// boundary at 5,000, 1,000 and 500 ms alike — the rule every cross-tempo script
/// in this tree follows, because a beat at any other millisecond would be
/// delivered at a different arena time at each cadence and the three runs would
/// diverge for the script's reasons and not the engine's. The first shot is at ten
/// seconds, comfortably past `classify_after_ms`, so every round in the run is
/// fired at a contact the shooter has told apart.
fn scan_script() -> Vec<Beat> {
    let contact = contact_of(RED, BLUE);
    let mut beats = vec![Beat {
        t_ms: 0,
        intents: vec![
            intent(RED, IntentKind::Startup, 1),
            intent(BLUE, IntentKind::Startup, 2),
            intent(MATE, IntentKind::Startup, 3),
        ],
    }];
    for (n, at) in [10_000_i64, 15_000, 20_000, 25_000].into_iter().enumerate() {
        let seq = u64::try_from(n)
            .unwrap_or(0)
            .saturating_mul(2)
            .saturating_add(4);
        beats.push(Beat {
            t_ms: at,
            intents: vec![
                intent(RED, IntentKind::Lock { contact }, seq),
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

/// **What one platform's scope says about the target's hull at one moment**: the
/// compact band spelling and the age of the *look* behind it, or `None` for a
/// platform holding no reading.
type ScanColumn = (i64, ObjId, Option<(String, i64)>);

/// What one replay of [`scan_script`] produced.
#[derive(Debug, PartialEq, Eq)]
struct Scanned {
    events: Vec<Line>,
    hashes: Vec<u64>,
    notices: BTreeMap<ObjId, Vec<String>>,
    /// Every checkpoint, at multiples of five seconds of arena time.
    columns: Vec<ScanColumn>,
    pulses: u32,
}

/// Drive [`meshed_trio`] through [`scan_script`] at `cadence_ms`, with the tempo
/// pinned, recording the ledger, the substep hashes, every pilot's lines and both
/// red platforms' readings at every five-second checkpoint.
fn scanned(cadence_ms: i64, until_ms: i64) -> Scanned {
    let mut arena = meshed_trio();
    let script = scan_script();
    let mut bytes: Vec<u8> = Vec::new();
    let mut hashes: Vec<u64> = Vec::new();
    let mut notices: BTreeMap<ObjId, Vec<String>> = BTreeMap::new();
    let mut columns: Vec<ScanColumn> = Vec::new();
    let mut wall = 0_i64;
    let mut next = 0_usize;
    let mut pulses = 0_u32;
    let mut ended = false;
    let absorb_notices = |out: &StepOutput, into: &mut BTreeMap<ObjId, Vec<String>>| {
        for (pilot, frame) in &out.frames {
            let lines = tacsim::match_::cockpit::notice_lines(frame);
            if !lines.is_empty() {
                into.entry(*pilot).or_default().extend(lines);
            }
        }
    };
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
            // A zero-length pulse, so a scripted order resolves at exactly the
            // millisecond the script names at every cadence.
            let out = arena.step(wall, &[] as &[Control], &intents);
            absorb(&out, &mut bytes);
            absorb_notices(&out, &mut notices);
            hashes.extend(arena.substep_hashes.iter().copied());
            pulses = pulses.saturating_add(1);
            ended = out.ended.is_some();
        }
        if ended || arena.now_ms >= until_ms {
            break;
        }
        wall += cadence_ms;
        let out = arena.step(wall, &[] as &[Control], &[]);
        absorb(&out, &mut bytes);
        absorb_notices(&out, &mut notices);
        hashes.extend(arena.substep_hashes.iter().copied());
        pulses = pulses.saturating_add(1);
        // The checkpoint: every five seconds of ARENA time, which is a pulse
        // boundary at 5,000, 1,000 and 500 ms alike.
        if arena.now_ms % 5_000 == 0 {
            for platform in [RED, MATE] {
                columns.push((arena.now_ms, platform, reading_of(&arena, platform, BLUE)));
            }
        }
        if out.ended.is_some() || arena.now_ms >= until_ms {
            break;
        }
    }
    let log = events(&mut arena, &mut bytes);
    Scanned {
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
        notices,
        columns,
        pulses,
    }
}

/// The band spelling and the look's age, off the frame `platform`'s pilot would
/// be sent — so a checkpoint reads the same surface `SCAN` reads and not the
/// store behind it.
fn reading_of(arena: &Arena, platform: ObjId, target: ObjId) -> Option<(String, i64)> {
    let id = contact_of(platform, target);
    arena
        .frame_contacts(platform)
        .into_iter()
        .find(|row| row.id == id)
        .and_then(|row| row.condition)
        .map(|reading| (reading.bands.letters(), reading.age_ms))
}

#[test]
fn one_meshed_engagement_at_three_cadences_is_one_set_of_gunners_words_and_readings() {
    let mut runs: Vec<(i64, Scanned)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        runs.push((cadence, scanned(cadence, 35_000)));
    }
    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };
    let shooter = first.notices.get(&RED_PILOT).cloned().unwrap_or_default();

    // ---- the run really carried the five things this cut moved -------------

    // 1. The section word, on a contact the shooter has told apart.
    let named = "BA";
    assert!(
        shooter
            .iter()
            .any(|line| line == &format!("Hit on {named}: turret.")),
        "the gunner was never told which section a round found: {shooter:?}"
    );
    // 2. The target-section line, once, and never the victim's own sentence.
    assert_eq!(
        shooter
            .iter()
            .filter(|line| *line == &format!("{named}'s turret is destroyed."))
            .count(),
        1,
        "{shooter:?}"
    );
    assert!(
        !shooter.iter().any(|line| line.starts_with("The turret")),
        "the gunner was shown the victim's own consequence list: {shooter:?}"
    );
    // ...which the victim really was sent, so the two halves are two sentences.
    let victim = first.notices.get(&BLUE_PILOT).cloned().unwrap_or_default();
    assert!(
        victim
            .iter()
            .any(|line| line.starts_with("The turret is destroyed")),
        "{victim:?}"
    );
    // 3 and 4. Both red platforms held a reading, and the blind one's can only
    //          have come over the mesh.
    let held = |platform: ObjId| -> Vec<(i64, Option<(String, i64)>)> {
        first
            .columns
            .iter()
            .filter(|(_, who, _)| *who == platform)
            .map(|(t_ms, _, reading)| (*t_ms, reading.clone()))
            .collect()
    };
    let shooters = held(RED);
    let blind = held(MATE);
    assert!(
        shooters.iter().any(|(_, reading)| reading.is_some()),
        "the shooter never read the target: {shooters:?}"
    );
    assert!(
        blind.iter().any(|(_, reading)| reading.is_some()),
        "the mesh never carried a reading to the blind teammate: {blind:?}"
    );
    // 5. The band the gunner watched go is `destroyed` on its own reading. The
    //    stock tank's sections read `engine, hull, left_track, right_track,
    //    turret`, so the turret is the fifth letter — and every other round went
    //    to the turret too, so nothing else moved.
    let last = shooters
        .iter()
        .filter_map(|(_, reading)| reading.clone())
        .next_back()
        .unwrap_or_else(|| panic!("the shooter held a reading: {shooters:?}"));
    assert_eq!(last.0, "ggggd", "{shooters:?}");

    // ---- the guard: three cadences really are three runs -------------------
    let counts: Vec<u32> = runs.iter().map(|(_, run)| run.pulses).collect();
    assert!(
        counts.windows(2).all(|pair| pair.first() != pair.get(1)),
        "the three replays ran the same number of pulses: {counts:?}"
    );

    // ---- and they agree, event for event, hash for hash, line for line -----
    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events, first.events,
            "the events differ at {cadence} ms"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the hashes differ at {cadence} ms"
        );
        assert_eq!(
            run.notices, first.notices,
            "the pilots' lines differ at {cadence} ms"
        );
        assert_eq!(
            run.columns, first.columns,
            "the readings differ at {cadence} ms"
        );
    }

    // The blind teammate was told about the contact and about nothing else: it
    // fired nothing, so it has no gunner's words at all.
    let watcher = first.notices.get(&MATE_PILOT).cloned().unwrap_or_default();
    assert!(!watcher.is_empty(), "the mesh told the teammate nothing");
    for line in &watcher {
        assert!(
            !line.starts_with("Hit on") && !line.contains("is destroyed"),
            "the teammate was told about a round it did not fire: {line}"
        );
    }
}

// ---------------------------------------------------------------------------
// (n9) the gunner's own eyes — 16b cut Q.2
// ---------------------------------------------------------------------------

/// Plant a reading on `viewer`'s contact on `target`: every section green, taken
/// at `at_ms`.
///
/// By hand, because the point of the tests below is what happens to a reading the
/// platform is **not** refreshing. `duel()`'s sight reaches five cells and the two
/// tanks are four apart, so the close look — half that reach — never covers them
/// and no pass in these runs can take a reading of its own. Anything that changes
/// in the bands below is therefore the gunner's own round and nothing else.
fn plant_reading(arena: &mut Arena, viewer: ObjId, target: ObjId, at_ms: i64) -> Condition {
    let green = Condition::default().with_band(4, ConditionBand::Green);
    let id = contact_of(viewer, target);
    if let Some(held) = arena.contacts.get_mut(viewer, id) {
        held.seen_condition = Some(Reading {
            at_ms,
            condition: green,
        });
        if let Some(own) = held.own_seen.as_mut() {
            own.condition = held.seen_condition;
        }
    }
    green
}

/// What `viewer` holds on `target`, straight out of the store.
fn contact_state(arena: &Arena, viewer: ObjId, target: ObjId) -> ShadowContact {
    let id = contact_of(viewer, target);
    arena
        .contacts
        .get(viewer, id)
        .copied()
        .unwrap_or_else(|| panic!("#{} holds no contact on #{}", viewer.0, target.0))
}

/// Fire at the turret until it is gone, answering every line the shooter was
/// sent **and every frame it was sent** — the second is what the wire-surface
/// test below reads, so the notices a panel decodes are the notices this same
/// run produced.
fn shoot_the_turret_off(
    arena: &mut Arena,
    bytes: &mut Vec<u8>,
    wall: i64,
) -> (Vec<String>, Vec<tacsim::arena::step::Frame>) {
    let contact = contact_of(RED, BLUE);
    if let Some(state) = arena.unit_mut(BLUE) {
        state.profile.bands = state
            .profile
            .bands
            .iter()
            .map(|band| BandProfile {
                sections: vec![(SmolStr::new("turret"), 1)],
                ..band.clone()
            })
            .collect();
    }
    let mut shooter: Vec<String> = Vec::new();
    let mut frames: Vec<tacsim::arena::step::Frame> = Vec::new();
    let mut wall_ms = wall;
    let mut fired = 0_u32;
    while fired < 4 {
        wall_ms += 500;
        let ready = arena
            .unit(RED)
            .is_some_and(|state| state.weapon_ready("cannon", arena.now_ms));
        let intents = if ready {
            fired += 1;
            vec![
                intent(RED, IntentKind::Lock { contact }, u64::from(fired) * 2),
                intent(
                    RED,
                    IntentKind::Fire {
                        weapon: SmolStr::new("cannon"),
                        slot: None,
                    },
                    u64::from(fired) * 2 + 1,
                ),
            ]
        } else {
            Vec::new()
        };
        let out = pulse(arena, wall_ms, &intents, bytes);
        shooter.extend(told(&out, RED_PILOT));
        frames.extend(seen_by(&out, RED_PILOT));
        // The contact is kept alive by hand, exactly as the sibling tests keep
        // it: what is under test is the reading, not the time to live.
        if let Some(held) = arena.contacts.get_mut(RED, contact) {
            held.last_seen_ms = arena.now_ms;
        }
    }
    for _ in 0..8 {
        wall_ms += 500;
        let out = pulse(arena, wall_ms, &[], bytes);
        shooter.extend(told(&out, RED_PILOT));
        frames.extend(seen_by(&out, RED_PILOT));
        if let Some(held) = arena.contacts.get_mut(RED, contact) {
            held.last_seen_ms = arena.now_ms;
        }
    }
    (shooter, frames)
}

#[test]
fn a_section_the_gunner_watched_go_is_written_into_the_reading_they_hold() {
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    let _ = hold(&mut arena, RED, BLUE);
    let planted = plant_reading(&mut arena, RED, BLUE, 1_000);
    assert_eq!(planted.letters(), "ggggg", "the planted reading is whole");

    let (shooter, _) = shoot_the_turret_off(&mut arena, &mut bytes, wall);
    assert!(
        shooter
            .iter()
            .any(|line| line == &format!("{}'s turret is destroyed.", named(&arena, RED, BLUE))),
        "the gunner was never told the turret went: {shooter:?}"
    );

    let held = contact_state(&arena, RED, BLUE);
    let read = held
        .seen_condition
        .unwrap_or_else(|| panic!("the reading survived"));
    // The turret is the fifth section in the stock tank's own order (`engine,
    // hull, left_track, right_track, turret`), and it is the only band that
    // moved: `SCAN` cannot contradict the line the pilot was just sent, and it
    // cannot invent anything else either.
    assert_eq!(read.condition.letters(), "ggggd");
    // **The stamp is untouched.** This is not a look — it is one fact seen
    // through a gunsight folded into a picture that is otherwise as old as it
    // says it is, and a reading that claimed to be fresh would be the gunner
    // being told their twenty-second-old picture is current.
    assert_eq!(read.at_ms, 1_000, "the look's own stamp moved");
    // And what may travel on the mesh is untouched too: a teammate is handed what
    // this platform's **sensors** read, never what its gunner inferred.
    assert_eq!(
        held.own_seen.and_then(|own| own.condition),
        Some(Reading {
            at_ms: 1_000,
            condition: planted
        }),
        "the gunner's inference reached the mesh"
    );
}

#[test]
fn a_gunner_with_no_reading_earns_none_by_shooting() {
    // The other half of the rule: the write-in corrects a picture, it does not
    // create one. A platform that has never been close enough to read the hull
    // has not earned a per-section picture by setting fire to part of it.
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    let _ = hold(&mut arena, RED, BLUE);
    assert!(
        contact_state(&arena, RED, BLUE).seen_condition.is_none(),
        "the fixture starts with no reading"
    );
    let (shooter, _) = shoot_the_turret_off(&mut arena, &mut bytes, wall);
    assert!(
        shooter
            .iter()
            .any(|line| line.ends_with("turret is destroyed.")),
        "{shooter:?}"
    );
    assert!(
        contact_state(&arena, RED, BLUE).seen_condition.is_none(),
        "shooting a section off bought the gunner a reading"
    );
    // ...and the frame says so, in the words `SCAN` prints.
    let row = arena
        .frame_contacts(RED)
        .into_iter()
        .find(|row| row.id == contact_of(RED, BLUE))
        .unwrap_or_else(|| panic!("the row is still held"));
    assert!(row.condition.is_none());
}

#[test]
fn the_wire_frame_carries_the_new_notice_with_its_designator_and_its_section() {
    // The sentence is the line client's; **this** is what the TUI and the web
    // decode, and Q.3 freezes it into the one fixture set all three renderers
    // read. So the kind word, the designator and the section are pinned as
    // *fields* — a panel that wants to flash the section a gunner just took off
    // a target must never have to parse it back out of `text`.
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    let _ = hold(&mut arena, RED, BLUE);
    let designator = named(&arena, RED, BLUE);
    let (_, frames) = shoot_the_turret_off(&mut arena, &mut bytes, wall);

    let notices: Vec<serde_json::Value> = frames
        .iter()
        .map(|frame| tacsim::match_::cockpit::status_frame(tacsim::arena_task::MatchId(1), frame))
        .filter_map(|payload| {
            payload
                .get("notices")
                .and_then(serde_json::Value::as_array)
                .cloned()
        })
        .flatten()
        .collect();
    let lost: Vec<&serde_json::Value> = notices
        .iter()
        .filter(|notice| {
            notice.get("kind").and_then(serde_json::Value::as_str) == Some("target_section_lost")
        })
        .collect();
    assert_eq!(lost.len(), 1, "one notice, once: {notices:?}");
    let notice = lost.first().unwrap_or_else(|| panic!("one notice"));
    assert_eq!(
        notice.get("designator").and_then(serde_json::Value::as_str),
        Some(designator.as_str())
    );
    assert_eq!(
        notice.get("section").and_then(serde_json::Value::as_str),
        Some("turret")
    );
    assert_eq!(
        notice.get("text").and_then(serde_json::Value::as_str),
        Some(format!("{designator}'s turret is destroyed.").as_str())
    );
    assert!(
        notice.get("contact").is_some_and(|id| !id.is_null()),
        "the row key a panel matches against: {notice}"
    );

    // **16b cut B.1** (`T-86`): the old-name key is on every notice, and `null`
    // on all three shapes this run sends the gunner — the lost section, the
    // hits and the rest — because none of them quotes an old name (only an
    // `identified` or an `unidentified again` line does).
    for notice in &notices {
        assert_eq!(
            notice.get("was"),
            Some(&serde_json::Value::Null),
            "a notice that quotes no old name published one: {notice}"
        );
    }

    // The gunner's hit lines carry the same key, under the same gate; every
    // other kind the gunner is sent carries `null` there. (16b cut B.1 widened
    // the key to the victim's own `struck`, `struck_spent` and `section_lost`,
    // where it is the victim's OWN section; this run reads the shooter's frames
    // only, so none of those is here.)
    let hits: Vec<&serde_json::Value> = notices
        .iter()
        .filter(|notice| notice.get("kind").and_then(serde_json::Value::as_str) == Some("hit"))
        .collect();
    assert!(!hits.is_empty(), "no hit notice: {notices:?}");
    for hit in hits {
        assert_eq!(
            hit.get("section").and_then(serde_json::Value::as_str),
            Some("turret"),
            "a classified hit published no section: {hit}"
        );
    }
    for notice in &notices {
        let kind = notice
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if kind == "hit" || kind == "target_section_lost" {
            continue;
        }
        assert_eq!(
            notice.get("section"),
            Some(&serde_json::Value::Null),
            "{kind} published a section of the target: {notice}"
        );
    }
}

/// Hand `viewer` an **unidentified** contact on `target` — [`hold`]'s twin, and
/// the state every gate in this cut is about.
///
/// A contact is not a durable object: this is exactly what a platform holds after
/// a first detection, and again after a classified contact is lost and
/// re-detected, so it is a state a live shooter reaches over and over.
fn hold_unknown(arena: &mut Arena, viewer: ObjId, target: ObjId) -> ContactId {
    let id = contact_of(viewer, target);
    let now = arena.now_ms;
    arena.contacts.insert(
        viewer,
        id,
        ShadowContact::sighted(target, Classification::Unknown, now, plot_of(arena, target)),
    );
    id
}

#[test]
fn an_unidentified_target_earns_no_section_word_and_no_section_lost_line() {
    // **The gate this whole sub-cut exists for.** A section id is a chassis
    // vocabulary — `left_track` against `left_leg` — so `Hit on X1: left_track.`
    // would tell a gunner the blip is tracked rather than legged, which is
    // identification by gunfire and is what doc 16 §2 W5 prices at
    // `classify_after_ms` of looking. The same word in
    // `X1's left_track is destroyed.` gives away exactly as much.
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    let _ = hold_unknown(&mut arena, RED, BLUE);
    // The shooter's own sight is taken off it, so no pass of this run can tell
    // the return apart behind the test's back — which is the state under test
    // and, on a live map, a gunner firing on a plot a teammate handed over.
    // `shoot_the_turret_off` keeps the row alive by hand, exactly as its
    // siblings do.
    if let Some(state) = arena.unit_mut(RED) {
        state.sensor = None;
    }
    let (shooter, _) = shoot_the_turret_off(&mut arena, &mut bytes, wall);
    assert!(
        matches!(
            contact_state(&arena, RED, BLUE).class,
            Classification::Unknown
        ),
        "the contact classified itself: the gate is not under test"
    );

    // The run really did what it is here to prove something about: a section
    // came apart, and the victim was told so.
    let journal = events(&mut arena, &mut bytes);
    assert!(
        count(&journal, "section_destroyed") >= 1,
        "no section was destroyed: {shooter:?}"
    );

    // The shooter is told the rounds landed, by the platform's own name for the
    // return and with no section word on any of them...
    let hits: Vec<&String> = shooter
        .iter()
        .filter(|line| line.starts_with("Hit on "))
        .collect();
    assert!(!hits.is_empty(), "the gunner was told nothing: {shooter:?}");
    let designator = named(&arena, RED, BLUE);
    for line in &hits {
        assert_eq!(
            *line,
            &format!("Hit on {designator}."),
            "a section word on a return the platform has not identified"
        );
    }
    // ...and not one word about the section that came apart.
    for line in &shooter {
        assert!(
            !line.contains("is destroyed."),
            "an unidentified target earned a section-destroyed line: {line}"
        );
    }

    // Nor was anything written into a reading on its behalf: the write-in is
    // under the same gate as the line it corrects.
    assert!(
        contact_state(&arena, RED, BLUE).seen_condition.is_none(),
        "an unidentified contact earned hashed state off a gunner's shot"
    );
}

#[test]
fn the_three_things_a_gunner_can_be_told_about_where_a_round_went() {
    // The two ungated sentences, pinned where they are built. The third —
    // `Hit on BA: turret.` — is what every fixture above produces; these two are
    // the states a duel cannot be driven into on purpose, because a contact that
    // is held is held and one that is not is not.
    let mut arena = duel();
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    let contact = hold(&mut arena, RED, BLUE);
    let out = pulse(&mut arena, wall + 500, &[], &mut bytes);
    let frame = out
        .frames
        .iter()
        .find(|(who, _)| *who == RED_PILOT)
        .map(|(_, frame)| frame.clone())
        .unwrap_or_else(|| panic!("the red pilot was sent a frame"));
    let rows = tacsim::match_::cockpit::frame_contacts(&frame);
    let named = tacsim::arena::perception::Named {
        id: contact,
        designator: tacsim::arena::state::Designator::Provisional(2),
    };
    let line = |notice: tacsim::arena::perception::FrameNotice| {
        tacsim::match_::cockpit::notice_line(&frame, &notice, &rows)
    };
    assert_eq!(
        line(tacsim::arena::perception::FrameNotice::Hit {
            contact: Some(named),
            section: None,
            spent: false,
        }),
        "Hit on X2."
    );
    assert_eq!(
        line(tacsim::arena::perception::FrameNotice::Hit {
            contact: None,
            section: None,
            spent: false,
        }),
        "Hit."
    );
    // And the third, off the same builder, so the three live side by side.
    assert_eq!(
        line(tacsim::arena::perception::FrameNotice::Hit {
            contact: Some(named),
            section: Some(SmolStr::new("turret")),
            spent: false,
        }),
        "Hit on X2: turret."
    );
    // **16b cut D.2** (`B-15`): and the fourth, which only the gunner who has
    // told the target apart ever reads.
    assert_eq!(
        line(tacsim::arena::perception::FrameNotice::Hit {
            contact: Some(named),
            section: Some(SmolStr::new("turret")),
            spent: true,
        }),
        "Hit on X2: turret, already destroyed."
    );
}

// ---------------------------------------------------------------------------
// (s) 16b cut D.2: the refusals, the wreck and the spent section reach the pilot
// ---------------------------------------------------------------------------

/// A third tank, on the blue side, whose pilot goes on typing after it dies.
const GREY: ObjId = ObjId(11);

/// The pilot `arena_of` seats on it — the third seat, so the third number.
const GREY_PILOT: ObjId = ObjId(1_002);

/// Its cell: seven north of red, which is past red's own five-cell sight, so the
/// contact this fixture hands red is the only reason red can shoot at it.
const GREY_CELL: (u16, u16) = (8, 12);

/// **16b cut D.2's scripted fixture**: the spent-turret duel with a third tank
/// the script kills in one round, red carrying the coaxial beside the cannon so
/// that one beat can put two rounds into the same spent section inside the
/// notice window.
///
/// Every round into blue goes to its **turret**, which the script's third shot
/// finishes — the turret's consequence takes weapons blue does not have, so blue
/// lives and every later round lands on a section with nothing left to break.
/// Every round into grey goes to its **hull**, whose internals are one point from
/// gone, so the first round that lands finishes the tank: the hull's consequence
/// is `destroy_unit`.
fn d2_arena() -> Arena {
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(RED_CELL.0, RED_CELL.1),
                vec![cannon(), machine_gun()],
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(BLUE_CELL.0, BLUE_CELL.1),
                Vec::new(),
            ),
            tank(
                GREY,
                Team::Blue,
                centre(GREY_CELL.0, GREY_CELL.1),
                Vec::new(),
            ),
        ],
        &[],
        &MatchConfig::default(),
    );
    for (unit, section) in [(BLUE, "turret"), (GREY, "hull")] {
        if let Some(state) = arena.unit_mut(unit) {
            state.profile.bands = state
                .profile
                .bands
                .iter()
                .map(|band| BandProfile {
                    sections: vec![(SmolStr::new(section), 1)],
                    ..band.clone()
                })
                .collect();
        }
    }
    spend_section(&mut arena, GREY, "hull", 1);
    for target in [BLUE, GREY] {
        let contact = contact_of(RED, target);
        let plot = plot_of(&arena, target);
        arena.contacts.insert(
            RED,
            contact,
            ShadowContact {
                // Far enough ahead that no pass expires it: this fixture is about
                // what a pilot is told, not about the sensor.
                last_seen_ms: i64::MAX / 4,
                own_seen: Some(OwnSighting {
                    at_ms: i64::MAX / 4,
                    plot,
                    condition: None,
                }),
                ..ShadowContact::sighted(
                    target,
                    Classification::Classified { team: Team::Blue },
                    0,
                    plot,
                )
            },
        );
    }
    arena
}

/// Zero every facet's armour on `section` and leave `leave` internals in it,
/// through one [`mush_tactics::UnitMutationPlan`] — which is the only way a
/// unit's cells move (D-16-6).
///
/// `leave` of **one** means the next round that lands finishes the section (and
/// fires its consequence); `leave` of **nought** means the section is already
/// spent and every round that lands on it finds nothing to break.
fn spend_section(arena: &mut Arena, unit: ObjId, section: &str, leave: i64) {
    let facets = arena
        .unit(unit)
        .and_then(|state| state.profile.section(section))
        .map(|profile| profile.facets.clone())
        .unwrap_or_else(|| panic!("#{} has no {section}", unit.0));
    let mut ops: Vec<UnitMutationOp> = Vec::new();
    for facet in facets {
        let addr = armor(section, facet.as_str());
        if arena
            .unit(unit)
            .is_some_and(|state| state.unit.cell(&addr).is_some())
        {
            ops.push(UnitMutationOp::SetCell { addr, value: 0 });
        }
    }
    ops.push(UnitMutationOp::SetCell {
        addr: internals(section),
        value: leave,
    });
    let plan = UnitMutationPlan::new(ops, None, Some(SmolStr::new("test:hurt")))
        .expect("the plan is well formed");
    let hurt = {
        let state = arena
            .unit(unit)
            .unwrap_or_else(|| panic!("#{} is not in the match", unit.0));
        mush_tactics::apply(&state.unit, &plan, 0)
            .expect("the plan applies")
            .0
    };
    let condition = tacsim::arena::condition::Condition::of(&hurt, 700, 350);
    if let Some(state) = arena.unit_mut(unit) {
        state.unit = hurt;
        state.condition = condition;
    }
}

/// The script: one round finishes grey, grey's pilot goes on ordering, eight
/// rounds go into blue's turret (the third finishes it), one beat puts a second
/// round into the spent turret inside the notice window, and one beat bursts nine
/// discrete orders at one live unit.
fn d2_script() -> Vec<Beat> {
    let blue = contact_of(RED, BLUE);
    let grey = contact_of(RED, GREY);
    let mut beats = vec![
        Beat {
            t_ms: 0,
            intents: vec![intent(RED, IntentKind::Startup, 1)],
        },
        Beat {
            t_ms: 15_000,
            intents: vec![
                intent(RED, IntentKind::Lock { contact: grey }, 2),
                intent(
                    RED,
                    IntentKind::Fire {
                        weapon: SmolStr::new("cannon"),
                        slot: None,
                    },
                    3,
                ),
            ],
        },
        // **16b cut D.2** (`B-03`): the pilot in the wreck goes on typing, and
        // phase 2 answers all three. Before this cut every one of them vanished.
        Beat {
            t_ms: 20_000,
            intents: vec![
                intent(GREY, IntentKind::Startup, 4),
                intent(GREY, IntentKind::Heading { mdeg: 90_000 }, 5),
                intent(
                    GREY,
                    IntentKind::Fire {
                        weapon: SmolStr::new("cannon"),
                        slot: None,
                    },
                    6,
                ),
            ],
        },
    ];
    for (n, at) in [
        25_000_i64, 30_000, 35_000, 40_000, 45_000, 50_000, 55_000, 60_000,
    ]
    .into_iter()
    .enumerate()
    {
        let seq = 10 + u64::try_from(n).unwrap_or(0) * 4;
        let mut intents = vec![
            intent(RED, IntentKind::Lock { contact: blue }, seq),
            intent(
                RED,
                IntentKind::Fire {
                    weapon: SmolStr::new("cannon"),
                    slot: None,
                },
                seq + 1,
            ),
        ];
        // The one beat that fires twice: two rounds into the same spent section
        // inside one `SPENT_NOTICE_MS`, which is the window engaging.
        if at == 45_000 {
            intents.push(intent(
                RED,
                IntentKind::Fire {
                    weapon: SmolStr::new("machine_gun"),
                    slot: None,
                },
                seq + 2,
            ));
        }
        beats.push(Beat { t_ms: at, intents });
    }
    // Nine discrete orders for one live unit in one beat:
    // `intents.max_pending_per_unit` is eight, so the ninth is refused.
    beats.push(Beat {
        t_ms: 65_000,
        intents: (0..9)
            .map(|n| intent(RED, IntentKind::Lock { contact: blue }, 100 + n))
            .collect(),
    });
    beats
}

/// **The mandatory cross-tempo property for 16b cut D.2** (16b §11; doc 16
/// §3.1's rate independence).
///
/// One scripted log at 5,000 / 1,000 / 500 ms gives one event list, one hash
/// sequence — which now folds `spent_said` — and **one ordered notice list per
/// pilot**. The notices are compared as one list over the whole run and never
/// grouped by frame: how notices are grouped into frames is per pulse by design,
/// their order and their content are not.
#[test]
fn d2_the_refusals_the_wreck_and_the_spent_section_are_one_list_at_three_cadences() {
    let mut runs: Vec<(i64, Replay)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = d2_arena();
        runs.push((cadence, replay(&mut arena, cadence, 70_000, &d2_script())));
    }
    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };
    let shooter = first.notices.get(&RED_PILOT).cloned().unwrap_or_default();
    let victim = first.notices.get(&BLUE_PILOT).cloned().unwrap_or_default();
    let dead = first.notices.get(&GREY_PILOT).cloned().unwrap_or_default();
    let count =
        |lines: &[String], needle: &str| lines.iter().filter(|line| line.contains(needle)).count();

    // (1) `B-03`: the pilot in the wreck is told, once, and then every order is
    // refused aloud with its own word.
    assert_eq!(
        count(&dead, tacsim::match_::cockpit::OWN_DESTROYED),
        1,
        "{dead:?}"
    );
    for order in ["STARTUP", "HEADING", "FIRE cannon"] {
        assert_eq!(
            count(&dead, &format!("{order} refused. Your tank is a wreck.")),
            1,
            "{order} was not refused aloud exactly once: {dead:?}"
        );
    }
    // And nothing about the tank that shot it.
    let red_callsign = callsign(&d2_arena(), RED);
    for line in &dead {
        assert!(
            !line.contains(&red_callsign),
            "the wreck's pilot was told who shot him: {line}"
        );
    }

    // (2) `B-03`: exactly one `queue_full` for the nine-order burst, at every
    // cadence. The queue is drained totally every pulse and is empty at the start
    // of every phase 2, so nine discrete orders in one beat overflow it once
    // however fast the arena is pulsing.
    assert_eq!(count(&shooter, "Too many orders at once"), 1, "{shooter:?}");

    // (3) `B-15`: the victim and the classified gunner each read one sentence per
    // spent section per five seconds, and the pair fired in one beat is one line.
    //
    // The rounds that found nothing left to break are the ones after **blue's
    // turret** went, which is the run's SECOND `section_destroyed`: the first is
    // grey's hull at 15,000 ms, and skipping to that one would retain the three
    // rounds that landed on a live turret and make the count unfailable.
    let turret_gone = first
        .events
        .iter()
        .enumerate()
        .filter(|(_, (_, encoded, _, _))| encoded.contains("section_destroyed"))
        .map(|(at, _)| at)
        .nth(1)
        .unwrap_or_else(|| panic!("the run takes grey's hull and then blue's turret"));
    let spent_rounds = first
        .events
        .get(turret_gone + 1..)
        .unwrap_or_default()
        .iter()
        .filter(|(_, encoded, _, _)| encoded.contains("\"kind\":\"hit\""))
        .count();
    assert_eq!(spent_rounds, 6, "rounds on a spent turret");
    let said = count(&victim, "Hit on the turret, which is already destroyed.");
    // Pinned exactly, not bounded: one line per round except the pair fired in
    // one beat, which is the window engaging. A bound would hold with the window
    // torn out.
    assert_eq!(
        said, 5,
        "one line per spent round bar the pair inside one window: \
         {said} lines for {spent_rounds} rounds"
    );
    assert_eq!(
        said,
        count(&shooter, ": turret, already destroyed."),
        "the two readers are under one window"
    );
    // The victim's own line names nothing but his own tank.
    assert!(
        !victim
            .iter()
            .any(|line| line.contains("already destroyed") && line.contains("BA")),
        "{victim:?}"
    );
    // And the first round on a FRESH section still says what it always did.
    assert!(
        victim
            .iter()
            .any(|line| line.contains("armour gone") || line.contains("armour left")),
        "{victim:?}"
    );
    assert!(
        victim
            .iter()
            .any(|line| line.starts_with("The turret is destroyed")),
        "{victim:?}"
    );

    // (4) And the whole of it is the same at every cadence.
    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.notices, first.notices,
            "the notices differ at {cadence} ms"
        );
        assert_eq!(
            run.events, first.events,
            "the event list differs at {cadence} ms"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the per-substep hash sequence differs at {cadence} ms"
        );
        assert_ne!(run.pulses, first.pulses, "{cadence} ms ran the same pulses");
    }
}

/// The spent-turret duel again, with the gunner holding the target
/// **unidentified** — and the turret spent before the first round where
/// `spend` says so.
///
/// It is the pair the observer-rule claim is made over: the same eight rounds
/// into a target whose turret is already gone, and into one whose turret is
/// whole.
fn unidentified_gunner_arena(spend: bool) -> Arena {
    let mut arena = arena_of(
        vec![
            tank(
                RED,
                Team::Red,
                centre(RED_CELL.0, RED_CELL.1),
                vec![cannon()],
            ),
            tank(
                BLUE,
                Team::Blue,
                centre(BLUE_CELL.0, BLUE_CELL.1),
                Vec::new(),
            ),
        ],
        &[],
        &MatchConfig::default(),
    );
    if let Some(state) = arena.unit_mut(BLUE) {
        state.profile.bands = state
            .profile
            .bands
            .iter()
            .map(|band| BandProfile {
                sections: vec![(SmolStr::new("turret"), 1)],
                ..band.clone()
            })
            .collect();
    }
    if spend {
        spend_section(&mut arena, BLUE, "turret", 0);
    }
    // The gunner's own sight is taken away, so the contact planted below stays
    // **unidentified** for the whole run: the shipped sight tells a target apart
    // three seconds after it finds one, and what this fixture is about is the
    // gunner who never has.
    if let Some(state) = arena.unit_mut(RED) {
        state.sensor = None;
    }
    let contact = contact_of(RED, BLUE);
    let plot = plot_of(&arena, BLUE);
    arena.contacts.insert(
        RED,
        contact,
        ShadowContact {
            last_seen_ms: i64::MAX / 4,
            own_seen: Some(OwnSighting {
                at_ms: i64::MAX / 4,
                plot,
                condition: None,
            }),
            // **Unidentified**, which is the whole of this fixture: a gunner who
            // has not told the target apart is told no section word at all.
            ..ShadowContact::sighted(BLUE, Classification::Unknown, 0, plot)
        },
    );
    arena
}

/// **The unidentified gunner cannot tell a spent section from a live one** (16b
/// cut D.2, `B-15`; doc 16 §1, §3.6).
///
/// The same eight rounds into a target whose turret is already gone and into one
/// whose turret is whole: his notice list is **equal in count and in text**.
/// Nothing is suppressed for him and no window entry is keyed on him, and both
/// are decisions rather than oversights — a line withheld after a round he
/// watched land would say "that one hit something already dead", and a
/// per-section key would let him count the target's destroyed sections by
/// counting lines per five seconds. Both are condition facts the close look is
/// the price of.
#[test]
fn d2_an_unidentified_gunner_reads_the_same_rounds_on_a_spent_and_a_live_target() {
    let mut runs: Vec<(bool, Replay, Arena)> = Vec::new();
    for spend in [true, false] {
        let mut arena = unidentified_gunner_arena(spend);
        let run = replay(&mut arena, 1_000, 60_000, &spent_section_script());
        runs.push((spend, run, arena));
    }
    let Some((_, spent, spent_arena)) = runs.first() else {
        panic!("two runs");
    };
    let Some((_, fresh, fresh_arena)) = runs.get(1) else {
        panic!("two runs");
    };
    let told = |run: &Replay| run.notices.get(&RED_PILOT).cloned().unwrap_or_default();
    let on_spent = told(spent);
    let on_fresh = told(fresh);
    assert!(
        on_spent
            .iter()
            .filter(|line| line.starts_with("Hit on"))
            .count()
            >= 4,
        "the run has to land rounds to be worth comparing: {on_spent:?}"
    );
    assert_eq!(
        on_spent, on_fresh,
        "the gunner could tell a spent section from a live one"
    );
    for line in &on_spent {
        assert!(
            !line.contains("already destroyed"),
            "an unidentified gunner was told a section was spent: {line}"
        );
        assert!(
            !line.contains("turret"),
            "an unidentified gunner was given a section word: {line}"
        );
    }
    // And no window entry is keyed on him, in either run — so nothing about him
    // could ever be suppressed.
    for arena in [spent_arena, fresh_arena] {
        assert!(
            arena.spent_said.keys().all(|(told, _, _)| *told != RED),
            "a window entry was keyed on the unidentified gunner: {:?}",
            arena.spent_said
        );
    }
    // The victim, by contrast, IS under the window: the two runs differ on his
    // surface, which is what makes the gunner's equality a claim about the
    // observer rule rather than about nothing having happened.
    assert_ne!(
        spent.notices.get(&BLUE_PILOT),
        fresh.notices.get(&BLUE_PILOT),
        "the victim read the same thing either way"
    );
}

/// **The shooter rides the two destruction lines, at three cadences** (16b
/// section 11's mandatory cross-tempo test for 16b cut D.4, `T-08`; doc 16
/// section 3.1 "Rate independence", section 4).
///
/// The same scripted duel the engagement property runs, read for the thing this
/// cut moved: `section_destroyed` and `unit_destroyed` now carry `by`, and the
/// gunner's two lines are staged off it instead of off whichever impact the
/// substep's scratch happened to be holding. Both are facts of arena time — the
/// round resolves at its own due event inside a substep — so the three cadences
/// have to agree on one event list, one per-substep hash sequence and one
/// ordered list of lines per pilot.
#[test]
fn d4_one_scripted_duel_carries_the_shooter_on_its_ledger_at_three_cadences() {
    let mut runs: Vec<(i64, Replay)> = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = scripted_arena();
        runs.push((cadence, replay(&mut arena, cadence, 45_000, &script())));
    }
    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };

    // The run is worth comparing: it takes a section off and finishes the tank.
    let carried = |kind: &str| -> Vec<String> {
        first
            .events
            .iter()
            .filter(|(_, json, _, _)| json.contains(&format!("\"kind\":\"{kind}\"")))
            .map(|(_, json, _, _)| json.clone())
            .collect()
    };
    let sections = carried("section_destroyed");
    let units = carried("unit_destroyed");
    assert_eq!(sections.len(), 1, "one section went: {sections:?}");
    assert_eq!(units.len(), 1, "and the tank with it: {units:?}");
    for json in sections.iter().chain(units.iter()) {
        assert!(
            json.contains(&format!("\"by\":{}", RED.0)),
            "the shooter is not on the line: {json}"
        );
    }
    // And the gunner read both of his lines off it.
    let told = first.notices.get(&RED_PILOT).cloned().unwrap_or_default();
    assert!(
        told.iter().any(|line| line.ends_with("destroyed.")),
        "the gunner was told nothing about the kill: {told:?}"
    );

    // The guard the sibling tests carry, and then the three equalities.
    let counts: Vec<u32> = runs.iter().map(|(_, run)| run.pulses).collect();
    assert!(
        counts.windows(2).all(|pair| pair.first() != pair.get(1)),
        "the three replays ran the same number of pulses: {counts:?}"
    );
    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events, first.events,
            "the event list differs at {cadence} ms"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the per-substep hash sequence differs at {cadence} ms"
        );
        assert_eq!(
            run.notices, first.notices,
            "the pilots' lines differ at {cadence} ms"
        );
    }
}

// ---------------------------------------------------------------------------
// 16b cut M.4: a shot past the last band, and a tank on no tracks
// ---------------------------------------------------------------------------

/// **16b cut M.4 (16c `B-52`, §5's Q19), end to end through the impact.** Red's
/// cannon is given one band a metre long, so the duel's range is past it: the
/// `FIRE` is accepted and paid for, the shot in flight carries a `beyond`
/// resolution at nought, blue turns in flight to show its flank, and the impact —
/// `damage.rs`'s `reresolve` of a turned target — keeps it blind: the round
/// misses. Before the cut the flank re-resolved through the silhouette and got
/// the curve's 1 per cent floor back.
#[test]
fn m4_a_shot_past_the_last_band_is_paid_for_and_stays_blind_when_the_target_turns() {
    let mut arena = duel();
    if let Some(state) = arena.unit_mut(RED) {
        for weapon in &mut state.weapons {
            weapon.bands = vec![WeaponBand {
                max_range_cm: 100,
                dispersion_mrad: 3,
            }];
        }
    }
    let mut bytes: Vec<u8> = Vec::new();
    let wall = warm(&mut arena, &mut bytes);
    let contact = hold(&mut arena, RED, BLUE);
    let rounds_before = arena
        .unit(RED)
        .map(|state| state.weapon_ammo("cannon"))
        .unwrap_or(0);
    pulse(
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
    let taken = arena
        .shot(1)
        .map(|shot| shot.resolution.clone())
        .unwrap_or_else(|| panic!("the FIRE was accepted"));
    assert!(taken.beyond, "the range is past the one band");
    assert_eq!(taken.p_permille, 0);
    assert_eq!(taken.band, "rear");
    assert_eq!(
        arena.unit(RED).map(|state| state.weapon_ammo("cannon")),
        Some(rounds_before.saturating_sub(1)),
        "the round is spent: there is no free probe"
    );
    // Blue turns to show red its flank while the round is in the air.
    if let Some(state) = arena.unit_mut(BLUE) {
        state.pose.heading_mdeg = 90_000;
    }
    let mut wall_ms = wall + 500;
    while !arena.shots.is_empty() && arena.now_ms < wall + 5_000 {
        wall_ms += 500;
        pulse(&mut arena, wall_ms, &[], &mut bytes);
    }
    let log = events(&mut arena, &mut bytes);
    assert_eq!(count(&log, "hit"), 0, "a beyond shot never lands");
    assert_eq!(
        misses(&log),
        vec!["dispersion".to_owned()],
        "it misses, as any round the draw does not land"
    );
}

/// The duel of `scripted_arena`, with every band of blue's aimed at its two
/// tracks and red's rounds heavy enough to destroy a track outright — so the
/// facet draw decides only which track a round finds, and blue, which carries no
/// weapon, is **disabled** the moment its second track goes.
fn two_track_arena() -> Arena {
    let mut arena = scripted_arena();
    if let Some(state) = arena.unit_mut(BLUE) {
        state.profile.bands = state
            .profile
            .bands
            .iter()
            .map(|band| BandProfile {
                sections: vec![
                    (SmolStr::new("left_track"), 1),
                    (SmolStr::new("right_track"), 1),
                ],
                ..band.clone()
            })
            .collect();
    }
    if let Some(state) = arena.unit_mut(RED) {
        for weapon in &mut state.weapons {
            weapon.damage = 1_000;
        }
    }
    arena
}

/// **16b cut M.4 (16c `B-47`): a tank on no tracks, at three cadences.**
///
/// Red fires eight rounds at blue's tracks. The second track to go fires the
/// version-2 pack's `disable_motion_when_all_of`, so blue — unarmed — is
/// `unit_disabled`, the blue side is eliminated and the match ends; its pilot is
/// told `the tank cannot move`. Before the cut both tracks gone left a tank
/// slowed and turning one way, which is not disabled, and the match ran on. The
/// cross-tempo property (16b §11, mandatory): one event list, one hash sequence
/// and one notice list at 5,000, 1,000 and 500 ms.
#[test]
fn m4_both_tracks_shot_off_stop_the_tank_at_three_cadences() {
    let mut runs: Vec<(i64, Replay)> = Vec::new();
    let mut limits = Vec::new();
    for cadence in [5_000_i64, 1_000, 500] {
        let mut arena = two_track_arena();
        let run = replay(&mut arena, cadence, 60_000, &spent_section_script());
        limits.push(arena.unit(BLUE).map(|state| state.limits));
        runs.push((cadence, run));
    }
    let Some((_, first)) = runs.first() else {
        panic!("three replays");
    };
    let destroyed: Vec<&String> = first
        .events
        .iter()
        .filter(|(_, encoded, _, _)| encoded.contains("\"kind\":\"section_destroyed\""))
        .map(|(_, encoded, _, _)| encoded)
        .collect();
    assert_eq!(destroyed.len(), 2, "both tracks, once each: {destroyed:?}");
    assert!(
        destroyed.iter().any(|line| line.contains("left_track"))
            && destroyed.iter().any(|line| line.contains("right_track")),
        "{destroyed:?}"
    );
    assert!(
        destroyed
            .iter()
            .all(|line| line.contains("disable_motion_when_all_of")),
        "each track's line carries its whole authored list: {destroyed:?}"
    );
    assert!(
        first
            .events
            .iter()
            .any(|(_, encoded, _, _)| encoded.contains("\"kind\":\"unit_disabled\"")),
        "an unarmed tank on no tracks is disabled"
    );
    assert_eq!(
        first.ended,
        Some(EndReason::Elimination {
            winner: Some(Team::Red)
        }),
        "and the blue side is eliminated"
    );
    let told = first.notices.get(&BLUE_PILOT).cloned().unwrap_or_default();
    assert!(
        told.iter()
            .any(|line| line.contains("the tank cannot move")),
        "blue's pilot is told the tank cannot move: {told:?}"
    );
    let stopped = limits
        .first()
        .copied()
        .flatten()
        .unwrap_or_else(|| panic!("blue is still in the arena"));
    assert!(stopped.no_turn_left && stopped.no_turn_right && stopped.motion_disabled);
    for (cadence, run) in runs.iter().skip(1) {
        assert_eq!(
            run.events, first.events,
            "the event list differs at {cadence} ms"
        );
        assert_eq!(
            run.hashes, first.hashes,
            "the per-substep hash sequence differs at {cadence} ms"
        );
        assert_eq!(
            run.notices, first.notices,
            "the notices differ at {cadence} ms"
        );
        assert_eq!(run.ended, first.ended);
    }
    assert!(
        limits
            .iter()
            .all(|at| *at == limits.first().copied().flatten())
    );
}

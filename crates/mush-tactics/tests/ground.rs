//! **S7 — the TacSim ground driver (doc 14 §5.4).**
//!
//! `@domain/create <room>=tactics.ground` opens a ten-metre window over the
//! same tiles a street fight is fought on at one metre; a mech engagement on
//! imported relief composites from that relief; an indirect shot clears the
//! ridge a direct one cannot; the ruleset's locomotions cost from the table.

use std::collections::BTreeSet;

use mush_atlas::cells::Cell;
use mush_atlas::physics::{Impassable, LocomotionTable, StepKind};
use mush_atlas::record::{RecordKey, SurfaceRef};
use mush_atlas::terrain::TerrainClass;
use mush_atlas::trace::{Eye, direct_fire_path, indirect_fire_path};
use mush_atlas::{AtlasModule, AtlasState, starter};
use mush_content::{Appliers, ContentModule, parse_pack};
use mush_core::{
    Audience, CanonicalId, Effect, Effects, GOD, GameModule, Inbound, ObjId, ObjKind, Outbox,
    WorldCommand,
};
use mush_proto::OutputEvent;
use mush_sim::SimulationModule;
use mush_tactics::TacticsModule;
use mush_tactics::ground::{GROUND_DRIVER, INDIRECT_APEX_CM, ground_move_cost, line_of_fire};
use mush_testkit::TestWorld;
use serde_json::{Value, json};

fn cid(s: &str) -> CanonicalId {
    CanonicalId::new(s).expect("a canonical id")
}

fn modules(extra_packs: Vec<String>) -> Vec<Box<dyn GameModule>> {
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
    let mut packs = starter::packs().expect("the shipped packs load");
    for p in extra_packs {
        packs.push(parse_pack(&p).expect("an extra pack parses"));
    }
    let content = ContentModule::new()
        .with_appliers(appliers)
        .with_boot_packs(packs);
    vec![
        Box::new(TacticsModule::new()),
        Box::new(mush_npc::NpcModule::new()),
        Box::new(SimulationModule::new()),
        Box::new(AtlasModule::new().with_shelf(starter::packs().expect("shelf"))),
        Box::new(content),
    ]
}

fn lot() -> TestWorld {
    TestWorld::with_modules(modules(Vec::new()))
}

fn room_named(tw: &TestWorld, name: &str) -> ObjId {
    tw.world()
        .iter()
        .find(|o| o.kind == ObjKind::Room && o.name == name)
        .map(|o| o.id)
        .unwrap_or_else(|| panic!("no room named {name}"))
}

fn text(tw: &TestWorld, events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    tw.seen_by(events, who).join("\n")
}

/// Run the composite the atlas deferred, so the window has its cells.
fn finish(tw: &mut TestWorld, actor: ObjId, effects: &[Effect]) {
    let Some((correlation, payload)) = effects.iter().find_map(|e| match e {
        Effect::ModuleAsync {
            module,
            correlation,
            payload,
            ..
        } if module.as_str() == mush_atlas::NS => Some((*correlation, payload.clone())),
        _ => None,
    }) else {
        panic!("the atlas deferred no composite");
    };
    let outcome = mush_atlas::run_async(&payload);
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut().step(
        Inbound::Apply(WorldCommand::ModuleAsyncDone {
            module: mush_atlas::NS.into(),
            actor,
            correlation,
            payload: outcome,
        }),
        &mut out,
        &mut fx,
    );
}

fn atlas(tw: &TestWorld) -> &AtlasState {
    tw.world()
        .module_state::<AtlasState>()
        .expect("the atlas is composed")
}

/// The one open window's domain and entry.
fn the_window(tw: &TestWorld) -> (u64, mush_atlas::WindowEntry) {
    let st = atlas(tw);
    assert_eq!(
        st.windows.open.len(),
        1,
        "one window: {:?}",
        st.windows.open.keys()
    );
    let (d, e) = st.windows.open.iter().next().expect("a window");
    (*d, e.clone())
}

/// A region with a ridge: a 4×4 field at ten metres where column 1 stands
/// thirty metres tall, and a neighborhood with a room either side of it.
fn relief_packs() -> Vec<String> {
    let ridge = (0..4)
        .map(|_| "0 3000 0 0".to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let mut region = json!({
        "manifest": { "id": "region.ridge", "version": 1, "kind": "region",
                      "digest": "0".repeat(64), "requires": [] },
        "entries": {
            "region": { "kind": "region", "def": {
                "name": "Ridge Country", "kind": "surface", "body": "earth",
                "extent": [[0, 0], [400, 400]],
                "background": { "surface": "paved", "mix": [["grass", 8]], "relief_cm": 0,
                                "relief_period_cm": 800, "seed": 1, "water_cm": null },
                "ground_resolution_cm": 1000 } },
            "locality": { "kind": "locality", "def": { "region": "region", "name": "The Pass",
                "kind": "district", "footprint": [[0,0],[400,0],[400,400],[0,400]] } },
            "field": { "kind": "heightfield", "def": {
                "region": "region", "origin": [0, 0], "resolution_cm": 1000,
                "cols": 4, "rows": 4, "data": ridge } }
        }
    });
    let mut neighborhood = json!({
        "manifest": { "id": "neighborhood.pass", "version": 1, "kind": "neighborhood",
                      "digest": "0".repeat(64), "requires": [{ "id": "region.ridge", "version": 1 }] },
        "entries": {
            "neighborhood": { "kind": "neighborhood", "def": {
                "region": "region.ridge.region", "locality": "region.ridge.locality", "name": "The Pass",
                "setting": "rural", "footprint": [[0,0],[40,0],[40,40],[0,40]],
                "room_budget": { "target": 2, "maximum": 2 }, "hydration": { "enabled": false } } },
            "room.west": { "kind": "room", "def": { "name": "West Slope", "surface": "ground",
                "footprint": { "rect": [[0, 0], [10, 40]] } } },
            "room.east": { "kind": "room", "def": { "name": "East Slope", "surface": "ground",
                "footprint": { "rect": [[20, 0], [40, 40]] } } }
        }
    });
    for doc in [&mut region, &mut neighborhood] {
        let unsigned = serde_json::to_string(&*doc).expect("json");
        let digest = parse_pack(&unsigned)
            .expect("parses")
            .computed_digest()
            .expect("digests")
            .to_hex();
        doc["manifest"]["digest"] = Value::String(digest);
    }
    vec![
        serde_json::to_string(&region).expect("json"),
        serde_json::to_string(&neighborhood).expect("json"),
    ]
}

#[test]
fn a_ground_domain_opens_a_ten_metre_window_over_the_same_tiles_a_street_fight_uses() {
    let mut tw = lot();
    let lobby = room_named(&tw, "Lobby");
    let pilot = tw.spawn_player("Natasha");
    tw.run(GOD, &format!("@teleport #{}=#{}", pilot.0, lobby.0));
    tw.run(GOD, &format!("@behavior/add #{}=sim.domain", lobby.0));
    let (events, effects) =
        tw.run_with_effects(GOD, &format!("@domain/create #{}={GROUND_DRIVER}", lobby.0));
    let seen = text(&tw, &events, GOD);
    assert!(seen.contains("Created domain"), "{seen}");
    assert!(
        !seen.contains("Driver tactics.ground:"),
        "the driver opened its window: {seen}"
    );
    finish(&mut tw, GOD, &effects);
    let (_, entry) = the_window(&tw);
    assert_eq!(entry.frame.cell_cm, 1000, "the ground driver's cell");
    assert!(entry.anchors.contains(&lobby));
    let composite = entry.composite.as_ref().expect("composited");
    let ground_pins: BTreeSet<RecordKey> = composite.pins.iter().map(|(k, _)| k.clone()).collect();
    assert!(!ground_pins.is_empty(), "the lot's tiles are pinned");
    // The pilot stands on the window.
    assert!(
        mush_sim::pose_of(tw.world(), pilot).is_some(),
        "the occupant was placed"
    );
    // Retire the domain: the driver closes its window.
    let domain = mush_sim::windows(tw.world())
        .next()
        .map(|(d, _)| d)
        .expect("a sim window");
    let events = tw.run(GOD, &format!("@domain/retire {}", domain.0));
    let seen = text(&tw, &events, GOD);
    assert!(seen.contains("retired"), "{seen}");
    assert!(
        atlas(&tw).windows.open.is_empty(),
        "the window closed with the domain"
    );
    // A street fight over the same room, at one metre, pins the same tiles.
    let (events, effects) = tw.run_with_effects(GOD, &format!("@atlas/window open #{}", lobby.0));
    let seen = text(&tw, &events, GOD);
    assert!(
        effects.iter().any(
            |e| matches!(e, Effect::ModuleAsync { module, .. } if module.as_str() == mush_atlas::NS)
        ),
        "the street window opens after the ground domain retired: {seen}"
    );
    finish(&mut tw, GOD, &effects);
    let (_, street) = the_window(&tw);
    assert_eq!(street.frame.cell_cm, 100);
    let street_pins: BTreeSet<RecordKey> = street
        .composite
        .as_ref()
        .expect("composited")
        .pins
        .iter()
        .map(|(k, _)| k.clone())
        .collect();
    assert!(
        ground_pins.intersection(&street_pins).count() > 0,
        "one relief, two cell sizes: {ground_pins:?} vs {street_pins:?}"
    );
}

#[test]
fn an_unknown_driver_is_refused_naming_the_registered_ones() {
    let mut tw = lot();
    let lobby = room_named(&tw, "Lobby");
    tw.run(GOD, &format!("@behavior/add #{}=sim.domain", lobby.0));
    let events = tw.run(GOD, &format!("@domain/create #{}=tactics.orbital", lobby.0));
    let seen = text(&tw, &events, GOD);
    assert!(
        seen.contains("Unknown driver") && seen.contains(GROUND_DRIVER),
        "{seen}"
    );
    assert!(atlas(&tw).windows.open.is_empty());
    // The passive driver still works and opens nothing.
    let events = tw.run(GOD, &format!("@domain/create #{}=sim.passive", lobby.0));
    let seen = text(&tw, &events, GOD);
    assert!(seen.contains("Created domain"), "{seen}");
    assert!(atlas(&tw).windows.open.is_empty());
}

#[test]
fn a_mech_engagement_on_imported_relief_composites_at_ten_metres_and_an_indirect_shot_clears_the_ridge()
 {
    let mut tw = TestWorld::with_modules(modules(relief_packs()));
    let west = room_named(&tw, "West Slope");
    let east = room_named(&tw, "East Slope");
    let mech = tw.spawn_player("Kai");
    let target = tw.spawn_player("Rae");
    tw.run(GOD, &format!("@teleport #{}=#{}", mech.0, west.0));
    tw.run(GOD, &format!("@teleport #{}=#{}", target.0, east.0));
    tw.run(GOD, &format!("@behavior/add #{}=sim.domain", west.0));
    let (events, effects) =
        tw.run_with_effects(GOD, &format!("@domain/create #{}={GROUND_DRIVER}", west.0));
    assert!(text(&tw, &events, GOD).contains("Created domain"));
    finish(&mut tw, GOD, &effects);
    let (_, entry) = the_window(&tw);
    assert_eq!(entry.frame.cell_cm, 1000);
    let composite = entry.composite.as_ref().expect("composited");
    let ground = composite.surface(&SurfaceRef::Ground).expect("the ground");
    // The relief the field imported, at ten metres: column 1 is the ridge.
    let ridge_x = ground
        .cell_of(mush_atlas::geom::PointCm { x: 1_500, y: 500 })
        .expect("on the map")
        .0;
    let west_x = ground
        .cell_of(mush_atlas::geom::PointCm { x: 500, y: 500 })
        .expect("on the map")
        .0;
    let east_x = ground
        .cell_of(mush_atlas::geom::PointCm { x: 2_500, y: 500 })
        .expect("on the map")
        .0;
    assert_eq!(ground.cells.get(ridge_x, 0).map(|c| c.elev_cm), Some(3000));
    assert_eq!(ground.cells.get(west_x, 0).map(|c| c.elev_cm), Some(0));
    assert_eq!(ground.cells.get(east_x, 0).map(|c| c.elev_cm), Some(0));
    assert!(
        composite
            .pins
            .iter()
            .any(|(k, _)| k == &RecordKey::new(cid("region.ridge"), cid("field.tile.0")))
    );
    // A direct shot across the ridge stops at it; an arc at the apex clears it.
    let direct = direct_fire_path(
        ground,
        &[],
        Eye::standing((west_x, 0)),
        Eye::standing((east_x, 0)),
    );
    assert!(
        !direct.reaches(),
        "the ridge stops a direct shot: {direct:?}"
    );
    let arc = indirect_fire_path(
        ground,
        &[],
        Eye::standing((west_x, 0)),
        Eye::standing((east_x, 0)),
        INDIRECT_APEX_CM,
    );
    assert!(arc.reaches(), "the arc clears the ridge: {arc:?}");
    // Both slopes are on the window, so the driver's read answers for the pair.
    if let Some(fire) = line_of_fire(tw.world(), mech, target) {
        assert!(fire.range_cells >= 1);
        assert!(
            fire.indirect || !fire.direct,
            "an arc reaches where a line may not"
        );
    }
}

#[test]
fn the_ground_drivers_locomotions_cost_from_the_table_at_ten_metres() {
    let table = LocomotionTable::shipped();
    let flat = Cell {
        class: TerrainClass::Ground,
        elev_cm: 0,
        cover: 0,
        room: 0,
    };
    let up_a_metre = Cell {
        class: TerrainClass::Ground,
        elev_cm: 100,
        cover: 0,
        room: 0,
    };
    let ridge = Cell {
        class: TerrainClass::Rock,
        elev_cm: 3000,
        cover: 0,
        room: 0,
    };
    for locomotion in mush_tactics::ground::GROUND_LOCOMOTIONS {
        // Level ground at ten metres: every ground locomotion has a price.
        let level = ground_move_cost(table, flat, flat, StepKind::Orthogonal, 1000, locomotion);
        assert!(level.is_ok(), "{locomotion} on level ground: {level:?}");
        // A metre's rise over ten metres is a ten percent slope: passable, dearer.
        let gentle = ground_move_cost(
            table,
            flat,
            up_a_metre,
            StepKind::Orthogonal,
            1000,
            locomotion,
        );
        assert!(gentle.is_ok(), "{locomotion} up a gentle slope: {gentle:?}");
        assert!(gentle.clone().unwrap_or(0) >= level.clone().unwrap_or(0));
    }
    // Thirty metres up over ten metres is a cliff to tracks and wheels; a rotor
    // does not climb at all — its rows say what the ground says to it.
    let tracked = ground_move_cost(table, flat, ridge, StepKind::Orthogonal, 1000, "tracked");
    assert!(
        matches!(tracked, Err(Impassable::Slope { .. })),
        "{tracked:?}"
    );
    let wheeled = ground_move_cost(table, flat, ridge, StepKind::Orthogonal, 1000, "wheeled");
    assert!(
        matches!(
            wheeled,
            Err(Impassable::Slope { .. }) | Err(Impassable::Class(_))
        ),
        "{wheeled:?}"
    );
    // The same cells at one metre are a three-thousand-percent slope too: one
    // relief, one answer, whatever the cell size says of the run.
    let fine = ground_move_cost(table, flat, ridge, StepKind::Orthogonal, 100, "tracked");
    assert!(fine.is_err());
}

//! **Doc 18 cut IC1** — the three room- and neighborhood-level fields the trade
//! run's station packs carry, applied for real:
//!
//! - `RoomDef.markers` (D-18-19): object markers on the room, exactly the pack's;
//! - `RoomDef.short_desc` (18a's *Short.* line): the existing `short_desc` room
//!   attribute the NPC module reads, with `+atlas` unchanged;
//! - `NeighborhoodDef.ambience` (18a §1.6): stored on the neighborhood record and
//!   shown on no player surface.

use mush_atlas::def::PORT_DOCK_MARKER;
use mush_atlas::{AtlasModule, AtlasState};
use mush_content::{Appliers, ContentModule, Pack, parse_pack};
use mush_core::{GOD, GameModule, ObjId, ObjKind};
use mush_testkit::TestWorld;
use serde_json::{Value, json};

fn signed(mut doc: Value) -> Pack {
    doc["manifest"]["digest"] = Value::String("0".repeat(64));
    let unsigned = serde_json::to_string(&doc).expect("json");
    let digest = parse_pack(&unsigned)
        .expect("parses")
        .computed_digest()
        .expect("digests")
        .to_hex();
    doc["manifest"]["digest"] = Value::String(digest);
    parse_pack(&serde_json::to_string(&doc).expect("json")).expect("parses")
}

fn region() -> Pack {
    signed(json!({
        "manifest": { "id": "region.town", "version": 1, "kind": "region", "digest": "", "requires": [] },
        "entries": {
            "region.town": { "kind": "region", "def": {
                "name": "The Town", "kind": "surface", "body": "earth",
                "extent": [[0, 0], [400, 400]], "datum_m": 0.0,
                "background": { "surface": "paved", "mix": [], "relief_cm": 0,
                                "relief_period_cm": 800, "seed": 1 },
                "ground_resolution_cm": 1000 } },
            "locality.town": { "kind": "locality", "def": {
                "region": "region.town", "name": "Town", "kind": "district",
                "footprint": [[0, 0], [400, 0], [400, 400], [0, 400]] } }
        }
    }))
}

const SHORT: &str = "A long gallery over the berths, pressure doors at its back.";
const AMBIENCE: [&str; 2] = [
    "A pressure door cycles in the distance, a long sigh and a clank.",
    "The lights flicker once, and steady.",
];

/// The dock at `version`, with `room` and `nb` edited onto the Gallery's def and
/// the neighborhood's def. A Limbo door keeps it reachable whatever the markers.
fn dock(version: u32, room: impl FnOnce(&mut Value), nb: impl FnOnce(&mut Value)) -> Pack {
    let mut doc = json!({
        "manifest": { "id": "neighborhood.dock", "version": version, "kind": "neighborhood",
                      "digest": "", "requires": [{ "id": "region.town", "version": 1 }] },
        "entries": {
            "neighborhood": { "kind": "neighborhood", "def": {
                "region": "region.town", "locality": "locality.town", "name": "The Dock",
                "setting": "station", "footprint": [[200, 0], [400, 0], [400, 200], [200, 200]],
                "room_budget": { "target": 2, "maximum": 2 } } },
            "room.gallery": { "kind": "room", "def": { "name": "Gallery", "surface": "ground",
                "kind": "indoor", "footprint": { "rect": [[230, 10], [260, 40]] },
                "desc": "A long gallery." } },
            "room.hall": { "kind": "room", "def": { "name": "Hall", "surface": "ground",
                "kind": "indoor", "footprint": { "rect": [[260, 10], [290, 40]] } } },
            "exit.limbo_gallery": { "kind": "exit", "def": { "from": "limbo", "to": "room.gallery",
                "names": ["Dock <DK>;dk", "Limbo <X>;x;limbo"] } },
            "exit.gallery_hall": { "kind": "exit", "def": { "from": "room.gallery", "to": "room.hall",
                "names": ["Hall <H>;h;hall", "Gallery <G>;g;gallery"] } }
        }
    });
    room(&mut doc["entries"]["room.gallery"]["def"]);
    nb(&mut doc["entries"]["neighborhood"]["def"]);
    signed(doc)
}

fn modules(shelf: Vec<Pack>, boot: Vec<Pack>) -> Vec<Box<dyn GameModule>> {
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
    vec![
        Box::new(mush_npc::NpcModule::new()),
        Box::new(AtlasModule::new().with_shelf(shelf)),
        Box::new(
            ContentModule::new()
                .with_appliers(appliers)
                .with_boot_packs(boot),
        ),
    ]
}

fn said(
    tw: &TestWorld,
    events: &[(mush_core::Audience, mush_proto::OutputEvent)],
    who: ObjId,
) -> String {
    tw.seen_by(events, who).join("\n")
}

/// Plan and apply the shelved dock as staff; panics with the plan when it does
/// not offer a fingerprint.
fn replan(tw: &mut TestWorld) {
    let events = tw.run(GOD, "@atlas/pack plan neighborhood.dock");
    let planned = said(tw, &events, GOD);
    let fingerprint = planned
        .lines()
        .find_map(|l| l.trim().strip_prefix("fingerprint "))
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("no fingerprint in:\n{planned}"))
        .to_string();
    let events = tw.run(GOD, &format!("@atlas/pack apply {fingerprint}"));
    let applied = said(tw, &events, GOD);
    assert!(applied.contains("Applied"), "{planned}\n{applied}");
}

fn room_named(tw: &TestWorld, name: &str) -> ObjId {
    tw.world()
        .iter()
        .find(|o| o.kind == ObjKind::Room && o.name == name)
        .map(|o| o.id)
        .unwrap_or_else(|| panic!("no room {name}"))
}

fn attr(tw: &TestWorld, obj: ObjId, name: &str) -> Option<String> {
    tw.world()
        .get(obj)
        .and_then(|o| o.attrs.get(name))
        .and_then(|a| a.as_text().map(str::to_string))
}

fn ambience(tw: &TestWorld) -> Vec<String> {
    tw.world()
        .module_state::<AtlasState>()
        .expect("atlas")
        .records
        .neighborhoods
        .values()
        .find(|n| n.name == "The Dock")
        .map(|n| n.ambience.clone())
        .expect("the dock's record")
}

#[test]
fn ic1_room_markers_are_object_markers_and_follow_the_pack() {
    let v1 = dock(1, |r| r["markers"] = json!(["port.dock"]), |_| {});
    let v2 = dock(2, |_| {}, |_| {});
    let mut tw = TestWorld::with_modules(modules(vec![v2], vec![region(), v1]));
    let gallery = room_named(&tw, "Gallery");
    let hall = room_named(&tw, "Hall");
    assert!(tw.world().has_marker(gallery, PORT_DOCK_MARKER));
    assert!(!tw.world().has_marker(hall, PORT_DOCK_MARKER));
    // A version without the marker takes it off the same room.
    replan(&mut tw);
    assert_eq!(room_named(&tw, "Gallery"), gallery, "updated in place");
    assert!(!tw.world().has_marker(gallery, PORT_DOCK_MARKER));
}

#[test]
fn ic1_a_rooms_short_desc_is_the_short_desc_attribute() {
    let v1 = dock(1, |r| r["short_desc"] = json!(SHORT), |_| {});
    let v2 = dock(2, |r| r["short_desc"] = json!(""), |_| {});
    let mut tw = TestWorld::with_modules(modules(vec![v2], vec![region(), v1]));
    let gallery = room_named(&tw, "Gallery");
    assert_eq!(attr(&tw, gallery, "short_desc").as_deref(), Some(SHORT));
    assert_eq!(attr(&tw, gallery, "short"), None, "no new attribute name");
    assert_eq!(
        attr(&tw, gallery, "desc").as_deref(),
        Some("A long gallery.")
    );

    // `+atlas` is byte-identical to the same pack without the field: the line is a
    // room attribute, not a view change.
    let mut plain =
        TestWorld::with_modules(modules(Vec::new(), vec![region(), dock(1, |_| {}, |_| {})]));
    let staff = |tw: &mut TestWorld| {
        let room = room_named(tw, "Gallery");
        tw.world_mut().move_to(GOD, room).expect("god moves");
        let events = tw.run(GOD, "+atlas");
        said(tw, &events, GOD)
    };
    let with = staff(&mut tw);
    let without = staff(&mut plain);
    assert!(with.contains("The Dock"), "{with}");
    assert_eq!(with, without);
    assert!(!with.contains(SHORT), "{with}");

    // A version with the field blank removes the attribute.
    tw.world_mut()
        .move_to(GOD, mush_core::LIMBO)
        .expect("god moves");
    replan(&mut tw);
    assert_eq!(attr(&tw, gallery, "short_desc"), None);
}

#[test]
fn ic1_ambience_is_stored_on_the_neighborhood_and_shown_nowhere() {
    let v1 = dock(1, |_| {}, |n| n["ambience"] = json!(AMBIENCE));
    let v2 = dock(2, |_| {}, |_| {});
    let mut tw = TestWorld::with_modules(modules(vec![v2], vec![region(), v1]));
    assert_eq!(ambience(&tw), AMBIENCE.map(str::to_string).to_vec());

    // No player surface carries a line: staff's and a player's `+atlas`, and a
    // `look` in the room.
    let alice = tw.spawn_player("Alice");
    let gallery = room_named(&tw, "Gallery");
    tw.world_mut().move_to(alice, gallery).expect("alice moves");
    tw.world_mut().move_to(GOD, gallery).expect("god moves");
    let mut seen = String::new();
    for (who, line) in [(GOD, "+atlas"), (alice, "+atlas"), (alice, "look")] {
        let events = tw.run(who, line);
        seen.push_str(&said(&tw, &events, who));
        seen.push('\n');
    }
    assert!(seen.contains("The Dock"), "{seen}");
    for line in AMBIENCE {
        assert!(!seen.contains(line), "{line} shown:\n{seen}");
    }

    // A version without the lines clears the list.
    tw.world_mut()
        .move_to(GOD, mush_core::LIMBO)
        .expect("god moves");
    replan(&mut tw);
    assert!(ambience(&tw).is_empty());
}

#[test]
fn ic1_ambience_over_its_bounds_is_refused() {
    let lines: Vec<String> = (0..25).map(|n| format!("Line {n}.")).collect();
    for (what, list, why) in [
        ("25 lines", json!(lines), "more than 24 ambience lines"),
        (
            "a blank line",
            json!(["A line.", "  "]),
            "neighborhood.ambience is empty or longer than 512 bytes",
        ),
    ] {
        let pack = dock(1, |_| {}, |n| n["ambience"] = list);
        let (world, registry) = mush_testkit::boot_world();
        let mut engine = mush_core::Engine::with_modules(
            world,
            registry,
            modules(Vec::new(), vec![region(), pack]),
        )
        .expect("the modules register");
        let refused = engine.modules_loaded(
            std::collections::HashMap::new(),
            &mut mush_core::Outbox::new(),
            &mut mush_core::Effects::new(),
        );
        // Refused by the ambience check itself, not by anything else in the pack.
        let error = refused.expect_err(what).to_string();
        assert!(
            error.contains(why),
            "{what}: refused for another reason: {error}"
        );
        assert!(
            !engine
                .world
                .iter()
                .any(|o| o.kind == ObjKind::Room && o.name == "Gallery"),
            "{what}: nothing of the pack's rooms was written"
        );
    }
}

//! The G lens (plan §3.1): can any pack pass validation with an overlap or an
//! orphan? Overlapping sites, overlapping rooms, a room that leaves its building,
//! an exit between rooms that do not touch, an unknown entry kind, and a hyphenated
//! id — each refused naming the entry, and each leaving the world as it was.

use mush_atlas::{AtlasModule, AtlasState, starter};
use mush_content::{Appliers, ContentModule, ContentState, PackError, parse_pack};
use mush_core::{CanonicalId, GOD, GameModule, ObjKind};
use mush_testkit::TestWorld;
use serde_json::{Value, json};

fn modules() -> Vec<Box<dyn GameModule>> {
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
    vec![
        Box::new(mush_npc::NpcModule::new()),
        Box::new(AtlasModule::new()),
        Box::new(
            ContentModule::new()
                .with_appliers(appliers)
                .with_boot_packs(vec![parse_pack(starter::REGION_JSON).expect("region")]),
        ),
    ]
}

/// A neighborhood pack on the lot with the entries given, re-digested.
fn neighborhood_pack(id: &str, entries: Value) -> String {
    let mut doc = json!({
        "manifest": { "id": id, "version": 1, "kind": "neighborhood",
                      "digest": "0000000000000000000000000000000000000000000000000000000000000000",
                      "requires": [{ "id": "region.limbo", "version": 2 }] },
        "entries": entries
    });
    let unsigned = serde_json::to_string(&doc).expect("json");
    let digest = parse_pack(&unsigned)
        .expect("parses")
        .computed_digest()
        .expect("digests")
        .to_hex();
    doc["manifest"]["digest"] = Value::String(digest);
    serde_json::to_string(&doc).expect("json")
}

fn base_entries() -> serde_json::Map<String, Value> {
    let v = json!({
        "neighborhood": { "kind": "neighborhood", "def": {
            "region": "region.limbo", "locality": "locality.lot", "name": "Test Block",
            "setting": "urban", "footprint": [[150,300],[250,300],[250,380],[150,380]],
            "room_budget": { "target": 2, "maximum": 4 } } },
        "site.a": { "kind": "site", "def": { "name": "A", "site_kind": "site.office",
            "footprint": [[160,310],[190,340],[190,340],[160,340]] } },
        "room.yard": { "kind": "room", "def": { "name": "Yard", "surface": "ground",
            "footprint": { "rect": [[200,310],[240,350]] } } },
        "room.yard2": { "kind": "room", "def": { "name": "Second Yard", "surface": "ground",
            "footprint": { "rect": [[200,350],[240,370]] } } },
        // S3: the validator wants every public room reached, so the base map has a
        // way in from Limbo and a way between its yards.
        "exit.limbo_yard": { "kind": "exit", "def": { "from": "limbo", "to": "room.yard",
            "names": ["Yard <Y>;y;yard", "Limbo <L>;l;limbo"] } },
        "exit.yards": { "kind": "exit", "def": { "from": "room.yard", "to": "room.yard2",
            "names": ["Second <2>;2", "First <1>;1"] } }
    });
    let Value::Object(map) = v else {
        panic!("object")
    };
    map
}

fn apply_via_command(tw: &mut TestWorld, pack: &str, json: &str) -> String {
    // Put it on the shelf, then plan and apply as staff.
    let parsed = mush_content::load_pack(json).expect("loads");
    tw.world_mut()
        .module_state_mut::<AtlasState>()
        .expect("atlas")
        .shelf
        .insert(parsed.manifest().id().clone(), parsed);
    let events = tw.run(GOD, &format!("@atlas/pack plan {pack}"));
    let said = tw.seen_by(&events, GOD).join("\n");
    let fingerprint = said
        .lines()
        .find_map(|l| l.trim().strip_prefix("fingerprint "))
        .and_then(|rest| rest.split_whitespace().next())
        .map(str::to_string);
    match fingerprint {
        Some(fp) => {
            let events = tw.run(GOD, &format!("@atlas/pack apply {fp}"));
            format!("{said}\n{}", tw.seen_by(&events, GOD).join("\n"))
        }
        None => said,
    }
}

fn objects(tw: &TestWorld) -> usize {
    tw.world()
        .iter()
        .filter(|o| !o.flags.contains(mush_core::Flags::GOING))
        .count()
}

#[test]
fn a_clean_block_applies_and_overlapping_rooms_are_refused_naming_both() {
    let mut tw = TestWorld::with_modules(modules());
    let mut entries = base_entries();
    // Fix the degenerate site polygon of the base map into a real rectangle.
    entries.insert(
        "site.a".into(),
        json!({ "kind": "site", "def": { "name": "A", "site_kind": "site.office",
        "footprint": [[160,310],[190,310],[190,340],[160,340]] } }),
    );
    let clean = neighborhood_pack("neighborhood.block", Value::Object(entries.clone()));
    let said = apply_via_command(&mut tw, "neighborhood.block", &clean);
    assert!(said.contains("Applied neighborhood.block"), "{said}");
    assert!(!said.contains("STOPPED"), "{said}");
    assert_eq!(
        tw.world()
            .iter()
            .filter(|o| o.kind == ObjKind::Room)
            .count(),
        3
    );

    // A second pack whose room overlaps the first pack's yard: the validator refuses
    // it at the plan (S3), naming the other room, and nothing is made or latched.
    let mut bad = base_entries();
    bad.remove("site.a");
    bad.remove("room.yard2");
    bad.remove("exit.yards");
    bad.insert(
        "neighborhood".into(),
        json!({ "kind": "neighborhood", "def": {
        "region": "region.limbo", "locality": "locality.lot", "name": "Next Block",
        "setting": "urban", "footprint": [[150,300],[250,300],[250,380],[150,380]],
        "room_budget": { "target": 1, "maximum": 2 } } }),
    );
    bad.insert(
        "room.yard".into(),
        json!({ "kind": "room", "def": { "name": "Overlapping Yard",
        "surface": "ground", "footprint": { "rect": [[210,320],[230,340]] } } }),
    );
    let overlapping = neighborhood_pack("neighborhood.next", Value::Object(bad));
    let before = objects(&tw);
    let said = apply_via_command(&mut tw, "neighborhood.next", &overlapping);
    assert!(said.contains("geometry.room_overlap"), "{said}");
    assert!(said.contains("overlaps room"), "{said}");
    assert!(said.contains("room.yard"), "{said}");
    assert!(said.contains("Fix the blocking findings"), "{said}");
    assert_eq!(objects(&tw), before, "the overlapping room was not made");
    let content = tw.world().module_state::<ContentState>().expect("content");
    assert!(
        content
            .latches
            .applied(&CanonicalId::new("neighborhood.next").expect("id"))
            .is_none(),
        "a pack the validator refuses is never latched"
    );
}

#[test]
fn overlapping_sites_and_a_room_outside_its_building_are_refused() {
    let mut tw = TestWorld::with_modules(modules());
    let mut entries = base_entries();
    entries.insert(
        "site.a".into(),
        json!({ "kind": "site", "def": { "name": "A", "site_kind": "site.office",
        "footprint": [[160,310],[190,310],[190,340],[160,340]] } }),
    );
    entries.insert(
        "site.b".into(),
        json!({ "kind": "site", "def": { "name": "B", "site_kind": "site.shop",
        "footprint": [[180,320],[200,320],[200,345],[180,345]] } }),
    );
    let pack = neighborhood_pack("neighborhood.block", Value::Object(entries));
    let said = apply_via_command(&mut tw, "neighborhood.block", &pack);
    assert!(said.contains("overlaps site"), "11a M9: {said}");

    let mut tw = TestWorld::with_modules(modules());
    let mut entries = base_entries();
    entries.insert(
        "site.a".into(),
        json!({ "kind": "site", "def": { "name": "A", "site_kind": "site.office",
        "footprint": [[160,310],[190,310],[190,340],[160,340]] } }),
    );
    entries.insert(
        "structure.a".into(),
        json!({ "kind": "structure", "def": { "site": "site.a",
        "footprint": [[160,310],[190,310],[190,340],[160,340]], "floors": [{ "index": 0 }] } }),
    );
    entries.insert("room.big".into(), json!({ "kind": "room", "def": { "name": "Too Big", "site": "site.a",
        "surface": { "floor": ["structure.a", 0] }, "footprint": { "rect": [[162,312],[199,338]] } } }));
    let pack = neighborhood_pack("neighborhood.block", Value::Object(entries));
    let said = apply_via_command(&mut tw, "neighborhood.block", &pack);
    assert!(said.contains("leaves structure"), "{said}");
}

#[test]
fn an_exit_between_rooms_that_do_not_touch_needs_a_distance() {
    let mut tw = TestWorld::with_modules(modules());
    let mut entries = base_entries();
    entries.remove("site.a");
    entries.insert(
        "room.far".into(),
        json!({ "kind": "room", "def": { "name": "Far Yard", "surface": "ground",
        "footprint": { "rect": [[160,310],[180,330]] } } }),
    );
    entries.insert(
        "exit.gap".into(),
        json!({ "kind": "exit", "def": { "from": "room.yard", "to": "room.far",
        "names": ["Far <F>;f", "Yard <Y>;y"] } }),
    );
    let pack = neighborhood_pack("neighborhood.block", Value::Object(entries.clone()));
    let said = apply_via_command(&mut tw, "neighborhood.block", &pack);
    assert!(said.contains("do not touch"), "{said}");
    assert!(said.contains("distance_m"), "{said}");

    // With a distance it is a journey portal, and the rooms that touch get a
    // threshold on their shared edge without being told.
    let mut tw = TestWorld::with_modules(modules());
    entries.insert(
        "exit.gap".into(),
        json!({ "kind": "exit", "def": { "from": "room.yard", "to": "room.far",
        "names": ["Far <F>;f", "Yard <Y>;y"], "portal": { "distance_m": 25 } } }),
    );
    entries.insert(
        "exit.yards".into(),
        json!({ "kind": "exit", "def": { "from": "room.yard", "to": "room.yard2",
        "names": ["Second <2>;2", "First <1>;1"] } }),
    );
    let pack = neighborhood_pack("neighborhood.block", Value::Object(entries));
    let said = apply_via_command(&mut tw, "neighborhood.block", &pack);
    assert!(said.contains("Applied"), "{said}");
    assert!(!said.contains("STOPPED"), "{said}");
    let atlas = tw.world().module_state::<AtlasState>().expect("atlas");
    let gap = atlas
        .records
        .portals
        .values()
        .find(|p| p.key.entry.as_str() == "exit.gap")
        .expect("portal");
    assert_eq!(gap.distance_m, 25);
    assert!(matches!(
        gap.time,
        mush_atlas::record::TimePolicy::Timed { .. }
    ));
    let yards = atlas
        .records
        .portals
        .values()
        .find(|p| p.key.entry.as_str() == "exit.yards")
        .expect("portal");
    let point = yards.point();
    assert!(
        (point.1 - 350.0).abs() < 0.01,
        "on the shared edge y = 350: {point:?}"
    );
    assert!(point.0 > 200.0 && point.0 < 240.0, "{point:?}");
    assert_eq!(yards.exits.len(), 2);
}

#[test]
fn an_unknown_kind_and_a_hyphenated_id_are_refused_naming_the_entry() {
    let mut entries = base_entries();
    entries.insert(
        "thing.odd".into(),
        json!({ "kind": "spaceship", "def": { "name": "Odd" } }),
    );
    let pack = neighborhood_pack("neighborhood.block", Value::Object(entries));
    let parsed = mush_content::load_pack(&pack).expect("parses: kinds are checked at apply");
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas");
    let refused = appliers.kinds().check(&parsed).expect_err("refused");
    assert!(
        matches!(refused, PackError::UnknownEntryKind { .. }),
        "{refused}"
    );
    assert!(refused.to_string().contains("spaceship"), "{refused}");

    let hyphen = r#"{"manifest":{"id":"neighborhood.block","version":1,"kind":"neighborhood","digest":"0000000000000000000000000000000000000000000000000000000000000000","requires":[]},"entries":{"room-yard":{"kind":"room","def":{}}}}"#;
    let refused = parse_pack(hyphen).expect_err("refused");
    assert!(refused.to_string().contains("room-yard"), "{refused}");
}

#[test]
fn background_is_pure() {
    use mush_atlas::{SurfaceRef, background};
    let tw = TestWorld::with_modules(modules());
    let atlas = tw.world().module_state::<AtlasState>().expect("atlas");
    let region = atlas.records.regions.values().next().expect("the lot");
    let a = background(region, &SurfaceRef::Ground, 12_345, 6_789);
    let b = background(region, &SurfaceRef::Ground, 12_345, 6_789);
    assert_eq!(a, b);
    assert_eq!(a.elev_cm, 0, "the lot is flat at datum");
}

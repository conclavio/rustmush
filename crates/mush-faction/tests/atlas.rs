//! S2b (doc 14 D-14-28): a control region may stand on an atlas place, and
//! `+territory` shows that place's footprint.

use mush_atlas::AtlasModule;
use mush_comsys::ComsysModule;
use mush_content::{Appliers, ContentModule, parse_pack};
use mush_core::{Audience, GOD, GameModule};
use mush_faction::FactionModule;
use mush_proto::OutputEvent;
use mush_testkit::TestWorld;
use serde_json::{Value, json};

fn signed(mut doc: Value) -> String {
    doc["manifest"]["digest"] = Value::String("0".repeat(64));
    let unsigned = serde_json::to_string(&doc).expect("json");
    let digest = parse_pack(&unsigned)
        .expect("parses")
        .computed_digest()
        .expect("digests")
        .to_hex();
    doc["manifest"]["digest"] = Value::String(digest);
    serde_json::to_string(&doc).expect("json")
}

fn packs() -> Vec<String> {
    let region = signed(json!({
        "manifest": { "id": "region.city", "version": 1, "kind": "region", "digest": "", "requires": [] },
        "entries": {
            "region.city": { "kind": "region", "def": {
                "name": "The City", "kind": "surface", "body": "earth",
                "extent": [[0, 0], [400, 400]], "datum_m": 0.0,
                "background": { "surface": "paved", "mix": [["grass", 8]], "relief_cm": 0,
                                "relief_period_cm": 800, "seed": 1 },
                "ground_resolution_cm": 1000 } },
            "locality.city": { "kind": "locality", "def": {
                "region": "region.city", "name": "The Old Town", "kind": "district",
                "footprint": [[0, 0], [400, 0], [400, 400], [0, 400]] } }
        }
    }));
    let waterfront = signed(json!({
        "manifest": { "id": "neighborhood.waterfront", "version": 1, "kind": "neighborhood", "digest": "",
                      "requires": [{ "id": "region.city", "version": 1 }] },
        "entries": {
            "neighborhood": { "kind": "neighborhood", "def": {
                "region": "region.city", "locality": "locality.city", "name": "Waterfront",
                "setting": "urban", "footprint": [[0, 0], [120, 0], [120, 80], [0, 80]],
                "room_budget": { "target": 1, "maximum": 4 } } },
            "room.docks": { "kind": "room", "def": { "name": "Docks", "surface": "ground",
                "footprint": { "rect": [[10, 10], [50, 50]] } } }
        }
    }));
    vec![region, waterfront]
}

fn world() -> TestWorld {
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    let packs = packs()
        .iter()
        .map(|p| parse_pack(p).expect("pack"))
        .collect();
    let modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(ComsysModule::new()),
        Box::new(AtlasModule::new()),
        Box::new(
            ContentModule::new()
                .with_appliers(appliers)
                .with_boot_packs(packs),
        ),
        Box::new(FactionModule::new()),
    ];
    TestWorld::with_modules(modules)
}

fn texts(tw: &TestWorld, events: &[(Audience, OutputEvent)]) -> String {
    tw.seen_by(events, GOD).join("\n")
}

#[test]
fn a_control_region_on_an_atlas_place_renders_its_footprint_in_territory() {
    let mut tw = world();
    let out = tw.run(GOD, "@region/create Harbour");
    assert!(
        texts(&tw, &out).contains("Created region Harbour"),
        "{}",
        texts(&tw, &out)
    );

    // Nowhere yet: +territory names the region and no map.
    let out = tw.run(GOD, "+territory");
    let listing = texts(&tw, &out);
    assert!(listing.contains("Harbour — unclaimed"), "{listing}");
    assert!(!listing.contains("[map:"), "{listing}");

    // On the map, by name: the footprint shows.
    let out = tw.run(GOD, "@region/atlas Harbour=Waterfront");
    assert!(
        texts(&tw, &out).contains("now stands on Waterfront (neighborhood.waterfront)"),
        "{}",
        texts(&tw, &out)
    );
    let out = tw.run(GOD, "+territory");
    let listing = texts(&tw, &out);
    assert!(
        listing.contains("[map: neighborhood Waterfront, 120 m × 80 m at (0, 0)]"),
        "{listing}"
    );

    // A site is too small a thing to stand on; a region by id works; empty clears.
    let out = tw.run(GOD, "@region/atlas Harbour=Nowhere");
    assert!(
        texts(&tw, &out).contains("No neighborhood, locality or region"),
        "{}",
        texts(&tw, &out)
    );
    let out = tw.run(GOD, "@region/atlas Harbour=region.city");
    assert!(
        texts(&tw, &out).contains("The City (region.city)"),
        "{}",
        texts(&tw, &out)
    );
    let out = tw.run(GOD, "+territory");
    assert!(
        texts(&tw, &out).contains("[map: region The City, 400 m × 400 m"),
        "{}",
        texts(&tw, &out)
    );
    let out = tw.run(GOD, "@region/atlas Harbour=");
    assert!(
        texts(&tw, &out).contains("stands on no atlas place"),
        "{}",
        texts(&tw, &out)
    );

    // The reference is on the record's document.
    let ops = tw.take_module_ops();
    let last_region = ops
        .iter()
        .rev()
        .find_map(|op| match &op.op {
            mush_core::ModuleWrite::JsonSet { key, value } if key.starts_with("region:") => {
                Some(value.clone())
            }
            _ => None,
        })
        .expect("the region was persisted");
    assert!(
        last_region.get("atlas").is_some_and(Value::is_null),
        "{last_region}"
    );
}

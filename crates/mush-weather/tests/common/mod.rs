//! The S2b weather fixture: an atlas with a coast region and two neighborhoods,
//! so cells have places to key on. Downtown holds a Plaza and a Court (outdoors,
//! joined north/south), an Awning (sheltered), a Cellar (indoors) and an Alley
//! (outdoors); the Dunes hold a Pier.

#![allow(dead_code)]

use std::time::Duration;

use mush_atlas::AtlasModule;
use mush_content::{Appliers, ContentModule, parse_pack};
use mush_core::{GOD, GameModule, ObjId, ObjKind};
use mush_testkit::TestWorld;
use mush_weather::{WeatherConfig, WeatherModule, WeatherState};
use serde_json::{Value, json};
use smol_str::SmolStr;

pub const CLIMATE: &str = "temperate-coastal";
pub const COAST: &str = "region.coast";
pub const DOWNTOWN: &str = "neighborhood.downtown";
pub const DUNES: &str = "neighborhood.dunes";

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

pub fn packs() -> Vec<String> {
    let region = signed(json!({
        "manifest": { "id": COAST, "version": 1, "kind": "region", "digest": "", "requires": [] },
        "entries": {
            "region.coast": { "kind": "region", "def": {
                "name": "Coast", "kind": "surface", "body": "earth",
                "extent": [[0, 0], [400, 400]], "datum_m": 0.0,
                "background": { "surface": "paved", "mix": [["grass", 8]], "relief_cm": 0,
                                "relief_period_cm": 800, "seed": 1 },
                "ground_resolution_cm": 1000 } },
            "locality.coast": { "kind": "locality", "def": {
                "region": "region.coast", "name": "The Coast", "kind": "district",
                "footprint": [[0, 0], [400, 0], [400, 400], [0, 400]] } }
        }
    }));
    let downtown = signed(json!({
        "manifest": { "id": DOWNTOWN, "version": 1, "kind": "neighborhood", "digest": "",
                      "requires": [{ "id": COAST, "version": 1 }] },
        "entries": {
            "neighborhood": { "kind": "neighborhood", "def": {
                "region": "region.coast", "locality": "locality.coast", "name": "Downtown",
                "setting": "urban", "footprint": [[0, 0], [200, 0], [200, 200], [0, 200]],
                "room_budget": { "target": 5, "maximum": 8 } } },
            "room.plaza": { "kind": "room", "def": { "name": "Plaza", "surface": "ground",
                "footprint": { "rect": [[10, 10], [50, 50]] }, "desc": "A wide plaza." } },
            "room.court": { "kind": "room", "def": { "name": "Court", "surface": "ground",
                "footprint": { "rect": [[50, 10], [90, 50]] } } },
            "room.awning": { "kind": "room", "def": { "name": "Awning", "surface": "ground",
                "footprint": { "rect": [[90, 10], [130, 50]] }, "flags": ["sheltered"] } },
            "room.cellar": { "kind": "room", "def": { "name": "Cellar", "surface": "ground",
                "footprint": { "rect": [[10, 60], [50, 100]] }, "kind": "indoor" } },
            "room.alley": { "kind": "room", "def": { "name": "Alley", "surface": "ground",
                "footprint": { "rect": [[50, 60], [90, 100]] } } },
            "exit.plaza_court": { "kind": "exit", "def": { "from": "room.plaza", "to": "room.court",
                "names": ["North <N>;north;n", "South <S>;south;s"] } }
        }
    }));
    let dunes = signed(json!({
        "manifest": { "id": DUNES, "version": 1, "kind": "neighborhood", "digest": "",
                      "requires": [{ "id": COAST, "version": 1 }] },
        "entries": {
            "neighborhood": { "kind": "neighborhood", "def": {
                "region": "region.coast", "locality": "locality.coast", "name": "Dunes",
                "setting": "rural", "footprint": [[200, 0], [400, 0], [400, 200], [200, 200]],
                "room_budget": { "target": 2, "maximum": 4 } } },
            "room.pier": { "kind": "room", "def": { "name": "Pier", "surface": "ground",
                "footprint": { "rect": [[210, 10], [250, 50]] } } }
        }
    }));
    vec![region, downtown, dunes]
}

/// The atlas, the content module with the fixture packs, and a weather module
/// with the given config.
pub fn modules(weather: WeatherModule) -> Vec<Box<dyn GameModule>> {
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    let packs = packs()
        .iter()
        .map(|p| parse_pack(p).expect("fixture pack parses"))
        .collect();
    vec![
        Box::new(AtlasModule::new()),
        Box::new(
            ContentModule::new()
                .with_appliers(appliers)
                .with_boot_packs(packs),
        ),
        Box::new(weather),
    ]
}

/// A world with the fixture and a weather module: `ambient` sets the ambient
/// probability (per-mille) and `tick_secs` the coarse-tick interval.
pub fn world(tick_secs: u64, ambient: u32) -> TestWorld {
    let module = WeatherModule::with_config(WeatherConfig {
        tick: Duration::from_secs(tick_secs),
        ambient_permille: ambient,
        ..WeatherConfig::default()
    });
    TestWorld::with_modules(modules(module))
}

/// A fixture room by name.
pub fn room(tw: &TestWorld, name: &str) -> ObjId {
    tw.world()
        .iter()
        .find(|o| o.kind == ObjKind::Room && o.name == name)
        .unwrap_or_else(|| panic!("no room named {name}"))
        .id
}

/// Opt Downtown into `CLIMATE`, returning the Plaza.
pub fn downtown_with_climate(tw: &mut TestWorld) -> ObjId {
    tw.run(GOD, &format!("@weather/climate {DOWNTOWN}={CLIMATE}"));
    room(tw, "Plaza")
}

/// `(state, pinned)` of a place's cell.
pub fn cell_state(tw: &TestWorld, place: &str) -> Option<(SmolStr, bool)> {
    let id = mush_core::CanonicalId::new(place).expect("place id");
    tw.world()
        .module_state::<WeatherState>()
        .and_then(|s| s.cell(&id))
        .map(|c| (c.state.clone(), c.pinned))
}

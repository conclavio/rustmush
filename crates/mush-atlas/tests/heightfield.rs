//! **S7 — heightfield import (doc 14 §3.5).**
//!
//! A region pack's `heightfield` entry is a grid of elevations over a rectangle
//! of the region, rasterized at apply to one ground tile at the entry's own
//! resolution. The same tile is what a window composites from at any cell size,
//! which is what lets a 10 m engagement and a 1 m street fight read one relief.

use mush_atlas::cells::base64_encode;
use mush_atlas::geom::{PointCm, RectCm};
use mush_atlas::record::{RecordKey, SurfaceRef, TileOwner};
use mush_atlas::window::{WindowFrame, capture, composite};
use mush_atlas::{AtlasModule, AtlasState};
use mush_content::{Appliers, ContentModule, ContentState, parse_pack};
use mush_core::{CanonicalId, GameModule};
use mush_testkit::TestWorld;
use serde_json::{Value, json};

fn cid(s: &str) -> CanonicalId {
    CanonicalId::new(s).expect("a canonical id")
}

/// A region pack of one 400 m square surface region and whatever entries follow.
fn region_pack(extra: Value) -> String {
    let mut entries = json!({
        "region": { "kind": "region", "def": {
            "name": "Relief", "kind": "surface", "body": "earth",
            "extent": [[0, 0], [400, 400]],
            "background": { "surface": "paved", "mix": [["grass", 8]], "relief_cm": 0,
                            "relief_period_cm": 800, "seed": 1, "water_cm": null },
            "ground_resolution_cm": 1000 } }
    });
    if let (Some(base), Some(more)) = (entries.as_object_mut(), extra.as_object()) {
        for (k, v) in more {
            base.insert(k.clone(), v.clone());
        }
    }
    let mut doc = json!({
        "manifest": { "id": "region.relief", "version": 1, "kind": "region",
                      "digest": "0".repeat(64), "requires": [] },
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

fn boot(pack: &str) -> TestWorld {
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    let modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(AtlasModule::new()),
        Box::new(
            ContentModule::new()
                .with_appliers(appliers)
                .with_boot_packs(vec![parse_pack(pack).expect("the pack parses")]),
        ),
    ];
    TestWorld::with_modules(modules)
}

fn atlas(tw: &TestWorld) -> &AtlasState {
    tw.world()
        .module_state::<AtlasState>()
        .expect("the atlas is composed")
}

/// A 4×4 field at 10 m: row `y` (south first) holds `y*100 + x*10` centimetres.
fn text_grid() -> String {
    (0..4)
        .map(|y| {
            (0..4)
                .map(|x| (y * 100 + x * 10).to_string())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn field_entry(data: &str, encoding: &str, extra: Value) -> Value {
    let mut def = json!({
        "region": "region", "origin": [100, 100], "resolution_cm": 1000,
        "cols": 4, "rows": 4, "encoding": encoding, "data": data
    });
    if let (Some(d), Some(more)) = (def.as_object_mut(), extra.as_object()) {
        for (k, v) in more {
            d.insert(k.clone(), v.clone());
        }
    }
    json!({ "field": { "kind": "heightfield", "def": def } })
}

fn field_tile_key() -> RecordKey {
    RecordKey::new(cid("region.relief"), cid("field.tile.0"))
}

#[test]
fn a_text_grid_imports_to_a_ground_tile_at_its_resolution() {
    let tw = boot(&region_pack(field_entry(&text_grid(), "text", json!({}))));
    let state = atlas(&tw);
    let tile = state
        .records
        .tiles
        .get(&field_tile_key())
        .expect("the imported tile");
    assert_eq!(tile.owner, TileOwner::Heightfield);
    assert_eq!(tile.surface, SurfaceRef::Ground);
    assert_eq!(tile.resolution_cm, 1000);
    assert_eq!((tile.cells.width(), tile.cells.height()), (4, 4));
    assert_eq!(tile.cells.get(0, 0).map(|c| c.elev_cm), Some(0));
    assert_eq!(tile.cells.get(3, 0).map(|c| c.elev_cm), Some(30));
    assert_eq!(tile.cells.get(0, 3).map(|c| c.elev_cm), Some(300));
    assert_eq!(tile.cells.get(3, 3).map(|c| c.elev_cm), Some(330));
    // The class is the region's own ground, not something the field authored.
    let class = tile.cells.get(1, 1).map(|c| c.class).expect("a cell");
    assert_ne!(class, mush_atlas::terrain::TerrainClass::Void);
    // The tile sits where the field said, in region metres.
    let bbox = tile.bbox.to_cm().expect("a box");
    assert_eq!(
        (bbox.min.x, bbox.min.y, bbox.max.x, bbox.max.y),
        (10_000, 10_000, 14_000, 14_000)
    );
    // And the region lists it beside its own procedural tile.
    let region = state
        .records
        .regions
        .get(&RecordKey::new(cid("region.relief"), cid("region")))
        .expect("the region");
    assert!(region.ground.contains(&field_tile_key()));
    assert_eq!(
        region.ground.len(),
        2,
        "the region's own tile and the import"
    );
    // The latch holds the field as applied.
    let content = tw.world().module_state::<ContentState>().expect("content");
    let applied = content
        .latches
        .applied(&cid("region.relief"))
        .expect("latched");
    assert!(applied.entries().contains_key(&cid("field")));
}

#[test]
fn a_le16_grid_imports_to_the_same_cells_as_its_text_twin() {
    let mut bytes = Vec::new();
    for y in 0..4i16 {
        for x in 0..4i16 {
            bytes.extend_from_slice(&(y * 100 + x * 10).to_le_bytes());
        }
    }
    let le16 = boot(&region_pack(field_entry(
        &base64_encode(&bytes),
        "le16",
        json!({}),
    )));
    let text = boot(&region_pack(field_entry(&text_grid(), "text", json!({}))));
    let a = atlas(&le16)
        .records
        .tiles
        .get(&field_tile_key())
        .expect("le16")
        .clone();
    let b = atlas(&text)
        .records
        .tiles
        .get(&field_tile_key())
        .expect("text")
        .clone();
    assert_eq!(a.cells, b.cells);
}

#[test]
fn scale_and_datum_shift_every_sample() {
    // Samples in metres above a field datum of two metres.
    let tw = boot(&region_pack(field_entry(
        &text_grid(),
        "text",
        json!({ "scale_cm": 100, "datum_cm": 200 }),
    )));
    // Too tall: 330 m + 2 m is past the tile's ±327 m — so only the low rows fit.
    // (The whole field is refused, and nothing is imported.)
    assert!(!atlas(&tw).records.tiles.contains_key(&field_tile_key()));
    let small = (0..4)
        .map(|_| "1 2 3 4".to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let tw = boot(&region_pack(field_entry(
        &small,
        "text",
        json!({ "scale_cm": 100, "datum_cm": 200 }),
    )));
    let tile = atlas(&tw)
        .records
        .tiles
        .get(&field_tile_key())
        .expect("imported");
    assert_eq!(tile.cells.get(0, 0).map(|c| c.elev_cm), Some(300));
    assert_eq!(tile.cells.get(3, 0).map(|c| c.elev_cm), Some(600));
}

#[test]
fn a_grid_whose_samples_disagree_with_its_shape_is_refused_naming_the_entry() {
    let short = "0 10 20\n100 110 120";
    let tw = boot(&region_pack(field_entry(short, "text", json!({}))));
    assert!(!atlas(&tw).records.tiles.contains_key(&field_tile_key()));
    let content = tw.world().module_state::<ContentState>().expect("content");
    let applied = content
        .latches
        .applied(&cid("region.relief"))
        .expect("latched");
    let anomaly = applied.anomaly().unwrap_or_default();
    assert!(
        anomaly.contains("field") && anomaly.contains("16 samples"),
        "the anomaly names the entry and the count: {anomaly}"
    );
    // The region itself still applied: a bad field costs the field, not the region.
    assert!(
        applied.entries().contains_key(&cid("region")),
        "the region applied before the field"
    );
}

#[test]
fn the_imported_relief_composites_at_1000_and_at_100_cm_from_the_same_tile() {
    let tw = boot(&region_pack(field_entry(&text_grid(), "text", json!({}))));
    let state = atlas(&tw);
    let region = RecordKey::new(cid("region.relief"), cid("region"));
    let frame_at = |cell_cm: u16| WindowFrame {
        region: region.clone(),
        surfaces: vec![SurfaceRef::Ground],
        bbox: RectCm {
            min: PointCm {
                x: 10_000,
                y: 10_000,
            },
            max: PointCm {
                x: 14_000,
                y: 14_000,
            },
        },
        cell_cm,
    };
    // Ten-metre cells: one window cell per sample.
    let coarse = composite(&capture(&state.records, &frame_at(1000)).expect("captured"))
        .expect("composited");
    let ground = &coarse.surfaces[0];
    assert_eq!((ground.grid.width, ground.grid.height), (4, 4));
    assert_eq!(ground.cells.get(0, 0).map(|c| c.elev_cm), Some(0));
    assert_eq!(ground.cells.get(2, 1).map(|c| c.elev_cm), Some(120));
    assert_eq!(ground.cells.get(3, 3).map(|c| c.elev_cm), Some(330));
    assert!(coarse.pins.iter().any(|(k, _)| k == &field_tile_key()));
    // One-metre cells: every cell inside a sample's square reads that sample.
    let fine =
        composite(&capture(&state.records, &frame_at(100)).expect("captured")).expect("composited");
    let ground = &fine.surfaces[0];
    assert_eq!((ground.grid.width, ground.grid.height), (40, 40));
    assert_eq!(ground.cells.get(0, 0).map(|c| c.elev_cm), Some(0));
    assert_eq!(ground.cells.get(9, 9).map(|c| c.elev_cm), Some(0));
    assert_eq!(ground.cells.get(10, 0).map(|c| c.elev_cm), Some(10));
    assert_eq!(ground.cells.get(25, 15).map(|c| c.elev_cm), Some(120));
    assert_eq!(ground.cells.get(39, 39).map(|c| c.elev_cm), Some(330));
    assert!(fine.pins.iter().any(|(k, _)| k == &field_tile_key()));
}

#[test]
fn an_imported_relief_outranks_the_regions_own_ground_at_the_same_resolution() {
    // A flat field of 250 cm where the region's procedural ground would say 0.
    let flat = (0..4)
        .map(|_| "250 250 250 250".to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let tw = boot(&region_pack(field_entry(&flat, "text", json!({}))));
    let state = atlas(&tw);
    let region = RecordKey::new(cid("region.relief"), cid("region"));
    // A window over the field and beyond it, at the region's own 1000 cm.
    let frame = WindowFrame {
        region: region.clone(),
        surfaces: vec![SurfaceRef::Ground],
        bbox: RectCm {
            min: PointCm { x: 8_000, y: 8_000 },
            max: PointCm {
                x: 16_000,
                y: 16_000,
            },
        },
        cell_cm: 1000,
    };
    let out = composite(&capture(&state.records, &frame).expect("captured")).expect("composited");
    let ground = &out.surfaces[0];
    // Inside the field: the import. Outside it: the region's flat background.
    assert_eq!(ground.cells.get(2, 2).map(|c| c.elev_cm), Some(250));
    assert_eq!(ground.cells.get(5, 5).map(|c| c.elev_cm), Some(250));
    assert_eq!(ground.cells.get(0, 0).map(|c| c.elev_cm), Some(0));
    assert_eq!(ground.cells.get(7, 7).map(|c| c.elev_cm), Some(0));
}

// ---------------------------------------------------------------------------
// C.1 (16b §8 cut C; doc 16 §3.2 "Terrain"): the authored class grid
// ---------------------------------------------------------------------------

/// The `classes` companion's on-disk form, which is the tile cell RLE with a
/// class code as its only payload: base64 over `[cols u16][rows u16]` and then
/// `[count u16][class u8]` per run, all little-endian, the southern row first.
/// Code 0 (`void`) means "the region's recipe keeps this cell".
fn encode_classes(cols: u16, rows: u16, codes: &[u8]) -> String {
    assert_eq!(codes.len(), usize::from(cols) * usize::from(rows));
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&cols.to_le_bytes());
    bytes.extend_from_slice(&rows.to_le_bytes());
    let mut run: Option<(u8, u16)> = None;
    let flush = |bytes: &mut Vec<u8>, run: Option<(u8, u16)>| {
        if let Some((code, n)) = run {
            bytes.extend_from_slice(&n.to_le_bytes());
            bytes.push(code);
        }
    };
    for code in codes {
        match run {
            Some((prev, n)) if prev == *code && n < u16::MAX => run = Some((prev, n + 1)),
            other => {
                flush(&mut bytes, other);
                run = Some((*code, 1));
            }
        }
    }
    flush(&mut bytes, run);
    base64_encode(&bytes)
}

/// An 8×8 field at 10 m, flat, whose `extra` is merged into the entry's def.
fn field8(extra: Value) -> Value {
    let flat = (0..8)
        .map(|_| "0 0 0 0 0 0 0 0".to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let mut def = json!({
        "region": "region", "origin": [100, 100], "resolution_cm": 1000,
        "cols": 8, "rows": 8, "encoding": "text", "data": flat
    });
    if let (Some(d), Some(more)) = (def.as_object_mut(), extra.as_object()) {
        for (k, v) in more {
            d.insert(k.clone(), v.clone());
        }
    }
    json!({ "field": { "kind": "heightfield", "def": def } })
}

/// A two-class 8×8 grid: a road along the southern two rows, a pond in the
/// south-west, and `void` — no authored class — everywhere else.
fn two_class_codes() -> Vec<u8> {
    let road = mush_atlas::terrain::TerrainClass::Road.code();
    let water = mush_atlas::terrain::TerrainClass::Water.code();
    let mut codes = vec![0_u8; 64];
    for x in 0..8usize {
        codes[x] = road;
        codes[8 + x] = road;
    }
    for y in 2..4usize {
        for x in 0..4usize {
            codes[y * 8 + x] = water;
        }
    }
    codes
}

#[test]
fn an_authored_class_grid_lands_per_cell_and_the_recipe_keeps_the_rest() {
    use mush_atlas::terrain::TerrainClass;

    let codes = two_class_codes();
    let authored = boot(&region_pack(field8(
        json!({ "classes": encode_classes(8, 8, &codes) }),
    )));
    let recipe_only = boot(&region_pack(field8(json!({}))));
    let tile = |tw: &TestWorld| {
        atlas(tw)
            .records
            .tiles
            .get(&field_tile_key())
            .expect("the imported tile")
            .clone()
    };
    let with = tile(&authored);
    let without = tile(&recipe_only);
    assert_eq!((with.cells.width(), with.cells.height()), (8, 8));

    let mut painted = 0_usize;
    let mut kept = 0_usize;
    for y in 0..8_u16 {
        for x in 0..8_u16 {
            let at = usize::from(y) * 8 + usize::from(x);
            let mine = with.cells.get(x, y).expect("a cell");
            let theirs = without.cells.get(x, y).expect("a cell");
            assert_eq!(mine.elev_cm, theirs.elev_cm, "the relief is untouched");
            match TerrainClass::from_code(codes[at]) {
                Some(TerrainClass::Void) | None => {
                    assert_eq!(
                        mine.class, theirs.class,
                        "({x},{y}) is not authored, so the recipe keeps it"
                    );
                    kept += 1;
                }
                Some(class) => {
                    assert_eq!(mine.class, class, "({x},{y}) is authored");
                    painted += 1;
                }
            }
        }
    }
    assert_eq!((painted, kept), (24, 40));
    // And the two authored classes really are both there, and are not what the
    // recipe would have said.
    assert_eq!(
        with.cells.get(0, 0).map(|c| c.class),
        Some(TerrainClass::Road)
    );
    assert_eq!(
        with.cells.get(0, 2).map(|c| c.class),
        Some(TerrainClass::Water)
    );
    assert!(
        without
            .cells
            .iter()
            .all(|(_, _, c)| c.class != TerrainClass::Road && c.class != TerrainClass::Water),
        "the recipe paints neither road nor water here"
    );
}

#[test]
fn a_class_grid_of_the_wrong_shape_or_an_unknown_code_is_refused_naming_the_entry() {
    let anomaly_of = |pack: &str| -> String {
        let tw = boot(pack);
        assert!(
            !atlas(&tw).records.tiles.contains_key(&field_tile_key()),
            "a refused field imports nothing"
        );
        tw.world()
            .module_state::<ContentState>()
            .expect("content")
            .latches
            .applied(&cid("region.relief"))
            .expect("latched")
            .anomaly()
            .unwrap_or_default()
            .to_string()
    };

    // The grid declares 4×8 where the samples are 8×8.
    let mut codes = two_class_codes();
    codes.truncate(32);
    let wrong = anomaly_of(&region_pack(field8(
        json!({ "classes": encode_classes(4, 8, &codes) }),
    )));
    assert!(
        wrong.contains("field") && wrong.contains("4×8") && wrong.contains("8×8"),
        "the anomaly names the entry and both shapes: {wrong}"
    );

    // A code no class owns.
    let mut codes = two_class_codes();
    codes[0] = 200;
    let unknown = anomaly_of(&region_pack(field8(
        json!({ "classes": encode_classes(8, 8, &codes) }),
    )));
    assert!(
        unknown.contains("field") && unknown.contains("200"),
        "the anomaly names the entry and the code: {unknown}"
    );
}

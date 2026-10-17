//! **Doc 18 cut IC1 acceptance** — the trade run's two IC stations, the IC door
//! and the IC-return rule, over the production composition (doc 18 §4.5, §4.6,
//! §4.7; D-18-3, D-18-19, D-18-35, D-18-49; 18a §0–§2).
//!
//! 1. the four station packs load, verify and are pinned (re-sign in place with
//!    `TACSIM_WRITE_CONTENT=1`, as `tests/content.rs` does);
//! 2. the production boot list and the atlas shelf carry them in order;
//! 3. they validate clean on the production roster;
//! 4. every door shares an edge and every exitless public seam is walled (pure,
//!    over 18a §0.4's boxes);
//! 5. the walls are walls on the rasterized floor, and a door's seam is open;
//! 6. Stickney validates only because of its `port.dock` marker;
//! 7. a fresh world boots them;
//! 8. a kept world (dev1) takes them at its next boot and a third boot applies
//!    nothing;
//! 9. every public room is walkable by its authored aliases, and the three
//!    locked ways refuse a player and admit staff;
//! 10. the IC door refuses a pilot who left at Stickney, and `+ic` returns them;
//! 11. `+ic` and the door for a Ceres pilot and a new one;
//! 12. the principals never wake.
//!
//! Every test that boots the stations composes `content_module_with(true, true)`
//! through [`production_roster_with_starter`]; `support::roster_booting` is used
//! only for test 8's first boot, which needs the old boot list. Fixtures place
//! players with `World::move_to`, which emits no `Moved` and so stamps nothing;
//! `@teleport` would stamp.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mush_atlas::def::{ExitDef, FeatureDef, RoomDef, SurfaceDef};
use mush_atlas::raster::{Grid, shared_threshold};
use mush_atlas::{
    AtlasState, PointCm, RecordKey, Shape, SurfaceRef, TerrainClass, ValidateContext, blocking,
    validate,
};
use mush_content::{ContentState, Pack, parse_pack};
use mush_core::{Audience, CanonicalId, GOD, LIMBO, ObjId, ObjKind};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, fold_module_ops, fold_module_ops_into};
use serde_json::Value;
use tacsim::TacsimConfig;
use tacsim::definitions::content::{
    CERES_PACK, CERES_REGION_PACK, CERES_ROOMS, LANE_PACK, STATION_SOURCES, STICKNEY_PACK,
    STICKNEY_REGION_PACK, STICKNEY_ROOMS, boot_packs, station_packs,
};
use tacsim::ic_return::{self, PORTS};

use support::ground::{UNSIGNED, sign};

// ---------------------------------------------------------------------------
// The pins
// ---------------------------------------------------------------------------

/// **The version each station pack ships at**: 1, but 2 for
/// `neighborhood.stickney_yards`, which doc 18 cut IC8 re-published (the P11
/// fittings card's text, lead 2c).
fn station_version(pack: &str) -> u32 {
    if pack == STICKNEY_PACK { 2 } else { 1 }
}

/// The pinned digests, in `STATION_SOURCES` order. An edit to a pack changes its
/// digest; the failure message prints the new values so a *reviewed* edit is a
/// line here plus the `digest` field in the pack itself.
const PINNED: [(&str, &str); 4] = [
    (
        CERES_REGION_PACK,
        "80b18bc047d703b5c6f2899155d4f914b68a190ab73a0e5695ffbaab6a4f83f0",
    ),
    (
        CERES_PACK,
        "383e0b135c1723100853df6b2cbe9bd3ee98eb6d752c0d00a825a365301dc1cf",
    ),
    (
        STICKNEY_REGION_PACK,
        "07e25b41e8e66f29e6d26773472f3783216f3c8d8967f5f2c091075047eeb2b1",
    ),
    (
        STICKNEY_PACK,
        "39aab74e2f7a56e2b71ee7cffcdd34168ad706fbc1317240d989d2dc95c42d42",
    ),
];

/// The files under `crates/tacsim/content/`, in `STATION_SOURCES` order.
const FILES: [&str; 4] = [
    "atlas/region.ceres_freeport.json",
    "atlas/neighborhood.ceres_freeport.json",
    "atlas/region.stickney_yards.json",
    "atlas/neighborhood.stickney_yards.json",
];

fn content_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("content")
}

fn writing() -> bool {
    std::env::var("TACSIM_WRITE_CONTENT").is_ok_and(|v| v == "1")
}

// ---------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------

fn config() -> TacsimConfig {
    TacsimConfig {
        enabled: true,
        ..TacsimConfig::default()
    }
}

/// The production roster, composed as `mush_server::boot` composes it, with the
/// starter area and the game's boot packs on — `content_module_with(true, true)`,
/// so the four station packs boot after the proving ground. A copy of
/// `tests/cockpit.rs`'s `roster_with`: the character sheet is left out for the
/// reason given there (its chargen payload handler cannot be registered twice in
/// one process), which also leaves out the onboarding gate, so a spawned player
/// walks.
fn production_roster_with_starter(cfg: TacsimConfig) -> Vec<Box<dyn mush_core::GameModule>> {
    use tacsim::composition as comp;

    let llm = Arc::new(
        mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
            .expect("a disabled LLM service composes"),
    );
    let onboarding = mush_core::OnboardingConfig::default();
    let game = tacsim::GameEconomyConfig::default();
    // Doc 18 cut IC5: `tacsim` the third quantity delegate, the inventory cell
    // handed to the layer below.
    let (mut mods, inventory) = comp::server_modules_with_tacsim(
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
    // Doc 18 cut IC2 (lead ruling R2): the production composition loads the game's
    // seed packs (the lane) on the same switch the content latch's boot list rides.
    mods.push(comp::tacsim_module_wired(cfg, true, inventory));
    mods.push(comp::content_module_with(true, true).expect("the content module composes"));
    mods
}

fn booted() -> TestWorld {
    TestWorld::with_modules(production_roster_with_starter(config()))
}

fn said(tw: &TestWorld, events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    tw.seen_by(events, who).join("\n")
}

fn atlas(tw: &TestWorld) -> &AtlasState {
    tw.world().module_state::<AtlasState>().expect("atlas")
}

fn content(tw: &TestWorld) -> &ContentState {
    tw.world().module_state::<ContentState>().expect("content")
}

fn id(text: &str) -> CanonicalId {
    CanonicalId::new(text).expect("a canonical id")
}

/// The object a pack entry made, through the content latch (`limbo` is `#0`).
fn obj(tw: &TestWorld, pack: &str, entry: &str) -> ObjId {
    if entry == "limbo" {
        return LIMBO;
    }
    mush_atlas::travel::latched_object(tw.world(), &RecordKey::new(id(pack), id(entry)))
        .unwrap_or_else(|| panic!("{pack}.{entry} is not latched"))
}

fn location(tw: &TestWorld, who: ObjId) -> Option<ObjId> {
    tw.world().get(who).and_then(|o| o.location)
}

/// One latch as the kept-world test compares it: version, digest, entry → object.
type LatchView = (u32, String, BTreeMap<String, Option<ObjId>>);

/// Every latch, by pack.
fn latches(tw: &TestWorld) -> BTreeMap<String, LatchView> {
    content(tw)
        .latches
        .all()
        .map(|a| {
            (
                a.pack().to_string(),
                (
                    a.version(),
                    a.digest().to_hex(),
                    a.entries()
                        .iter()
                        .map(|(k, e)| (k.to_string(), e.object()))
                        .collect(),
                ),
            )
        })
        .collect()
}

fn live_objects(tw: &TestWorld) -> usize {
    tw.world()
        .iter()
        .filter(|o| !o.flags.contains(mush_core::Flags::GOING))
        .count()
}

/// The decoded JSON of a station pack's file.
fn pack_json(pack: &str) -> Value {
    let (_, json) = STATION_SOURCES
        .iter()
        .find(|(name, _)| *name == pack)
        .expect("a station pack");
    serde_json::from_str(json).expect("the pack is JSON")
}

/// The entries of `kind` in a station pack, by id.
fn entries_of(pack: &str, kind: &str) -> BTreeMap<String, Value> {
    pack_json(pack)["entries"]
        .as_object()
        .expect("entries")
        .iter()
        .filter(|(_, e)| e["kind"] == kind)
        .map(|(k, e)| (k.clone(), e["def"].clone()))
        .collect()
}

fn rooms_of(pack: &str) -> BTreeMap<String, RoomDef> {
    entries_of(pack, "room")
        .into_iter()
        .map(|(k, v)| (k, serde_json::from_value(v).expect("a room def")))
        .collect()
}

fn exits_of(pack: &str) -> BTreeMap<String, ExitDef> {
    entries_of(pack, "exit")
        .into_iter()
        .map(|(k, v)| (k, serde_json::from_value(v).expect("an exit def")))
        .collect()
}

fn walls_of(pack: &str) -> BTreeMap<String, FeatureDef> {
    entries_of(pack, "feature")
        .into_iter()
        .map(|(k, v)| {
            (
                k,
                serde_json::from_value::<FeatureDef>(v).expect("a feature def"),
            )
        })
        .filter(|(_, f)| f.feature_kind == mush_atlas::record::FeatureKind::Wall)
        .collect()
}

fn floor_of(def: &RoomDef) -> i16 {
    match &def.surface {
        SurfaceDef::Floor(_, index) => *index,
        SurfaceDef::Ground => panic!("a station room stands on a floor"),
    }
}

fn shape_of(def: &RoomDef) -> Shape {
    def.footprint
        .to_footprint()
        .shape()
        .expect("a room footprint")
}

fn cm(p: mush_atlas::Point2) -> PointCm {
    p.to_cm().expect("a point")
}

fn same_segment(a: (PointCm, PointCm), b: (PointCm, PointCm)) -> bool {
    a == b || (a.0 == b.1 && a.1 == b.0)
}

/// The first `;`-segment after an exit name's display segment.
fn first_alias(name: &str) -> &str {
    name.split(';')
        .nth(1)
        .map(str::trim)
        .unwrap_or_else(|| panic!("{name} has no alias"))
}

/// Validate every station pack against the production world it is applied in.
fn station_findings(tw: &TestWorld) -> Vec<(String, Vec<mush_atlas::Finding>)> {
    let w = tw.world();
    station_packs()
        .expect("the station packs verify")
        .iter()
        .map(|pack| {
            let cx = ValidateContext::for_pack(w, atlas(tw), content(tw), pack);
            (pack.manifest().id().to_string(), validate(pack, &cx))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 1. The pins
// ---------------------------------------------------------------------------

#[test]
fn ic1_the_station_packs_load_and_their_digests_are_pinned() {
    if writing() {
        // Re-sign **in place**, as `tests/content.rs` does: the declared digest's
        // hex is swapped for the placeholder and then for the computed value, so
        // the authored formatting survives and the diff of a re-sign is one line.
        for relative in FILES {
            let path = content_dir().join(relative);
            let raw = std::fs::read_to_string(&path).expect("the content file exists");
            let declared = parse_pack(&raw)
                .expect("the pack parses")
                .manifest()
                .digest()
                .to_hex();
            let unsigned = raw.replace(&declared, UNSIGNED);
            std::fs::write(&path, sign(&unsigned)).expect("the content file is writable");
        }
        return;
    }
    // Parsed without the digest compare, so one run reports every moved digest at
    // once — the content digest is what is pinned; `load_pack` is what boot uses.
    let mut report = Vec::new();
    for ((name, json), (pack, pinned)) in STATION_SOURCES.iter().zip(PINNED) {
        let parsed = parse_pack(json).expect(name);
        assert_eq!(parsed.manifest().id().as_str(), pack);
        assert_eq!(parsed.manifest().version(), station_version(pack), "{pack}");
        let actual = parsed.computed_digest().expect("digests").to_hex();
        if actual != pinned {
            report.push(format!("    ({pack:?}, {actual:?}),"));
        }
    }
    assert!(
        report.is_empty(),
        "station pack digests moved; if the edit was reviewed, pin them here and in each \
         pack's `digest` field:\n{}",
        report.join("\n")
    );
    assert_eq!(station_packs().expect("the station packs verify").len(), 4);
}

#[test]
fn ic1_the_declared_digest_is_the_content_digest() {
    for (name, json) in STATION_SOURCES {
        let pack = parse_pack(json).expect(name);
        assert_eq!(
            pack.computed_digest().expect("digests").to_hex(),
            pack.manifest().digest().to_hex(),
            "{name}: the digest field must be re-derived, never hand-edited"
        );
    }
}

// ---------------------------------------------------------------------------
// 2. The boot list and the shelf
// ---------------------------------------------------------------------------

#[test]
fn ic1_the_boot_list_and_the_shelf_carry_the_stations_in_order() {
    use tacsim::composition::boot_pack_list;

    let ids = |packs: &[Pack]| -> Vec<String> {
        packs
            .iter()
            .map(|p| p.manifest().id().to_string())
            .collect()
    };
    let full = boot_pack_list(true, true).expect("the boot list loads");
    let mut want: Vec<String> = mush_atlas::starter::SOURCES
        .iter()
        .map(|(_, json)| {
            parse_pack(json)
                .expect("starter")
                .manifest()
                .id()
                .to_string()
        })
        .collect();
    want.extend(ids(&boot_packs().expect("the game packs")));
    want.extend(STATION_SOURCES.iter().map(|(name, _)| (*name).to_string()));
    // Doc 18 cut IC2: the lane after the stations (the game's seven boot packs).
    want.push(LANE_PACK.to_string());
    assert_eq!(ids(&full), want);
    assert_eq!(full.len(), 10);
    // Region packs require nothing; each neighborhood requires its region at 1.
    for pack in &full[5..] {
        let requires: Vec<(String, u32)> = pack
            .manifest()
            .requires()
            .iter()
            .map(|r| (r.id.to_string(), r.version))
            .collect();
        match pack.manifest().id().as_str() {
            CERES_REGION_PACK | STICKNEY_REGION_PACK => assert!(requires.is_empty()),
            CERES_PACK => assert_eq!(requires, vec![(CERES_REGION_PACK.to_string(), 1)]),
            STICKNEY_PACK => assert_eq!(requires, vec![(STICKNEY_REGION_PACK.to_string(), 1)]),
            // Doc 18 cut IC2: the lane requires nothing.
            LANE_PACK => assert!(requires.is_empty()),
            other => panic!("unexpected {other}"),
        }
    }
    // The stations are game content: no game content, no stations.
    let starter_only = ids(&boot_pack_list(true, false).expect("the boot list loads"));
    assert_eq!(starter_only.len(), 3);
    for (name, _) in STATION_SOURCES {
        assert!(!starter_only.iter().any(|p| p == name));
    }
    assert!(!starter_only.iter().any(|p| p == LANE_PACK), "nor the lane");
    // On the production roster the shelf lists all four, latched.
    let mut tw = booted();
    let events = tw.run(GOD, "@atlas/pack list");
    let out = said(&tw, &events, GOD);
    for (name, _) in STATION_SOURCES {
        assert!(
            out.contains(&format!(" {name}@{} (", station_version(name))),
            "{name}:\n{out}"
        );
    }
    // Doc 18 cut IC2: and the lane after them (at version 3 since cut IC8).
    assert!(
        out.contains(&format!(" {LANE_PACK}@3 (")),
        "{LANE_PACK}:\n{out}"
    );
}

// ---------------------------------------------------------------------------
// 3. Validate clean
// ---------------------------------------------------------------------------

#[test]
fn ic1_the_station_packs_validate_clean() {
    let tw = booted();
    for (pack, findings) in station_findings(&tw) {
        assert_eq!(
            blocking(&findings),
            0,
            "{pack} blocks: {:#?}\nwarnings: {:#?}",
            findings
                .iter()
                .filter(|f| f.blocks())
                .map(mush_atlas::Finding::line)
                .collect::<Vec<_>>(),
            findings
                .iter()
                .filter(|f| f.severity == mush_atlas::Severity::Warning)
                .map(mush_atlas::Finding::line)
                .collect::<Vec<_>>()
        );
        // As the starter's model test pins (`mush-atlas/tests/validate.rs`): no
        // warning either. A banned theme word only warns, and both stations ship
        // a `banned` list beside the 2350 base, so this is their only gate. The
        // `secrets.staff_only` lines are info and pass.
        let warnings: Vec<_> = findings
            .iter()
            .filter(|f| f.severity == mush_atlas::Severity::Warning)
            .map(mush_atlas::Finding::line)
            .collect();
        assert!(warnings.is_empty(), "{pack} warns: {warnings:#?}");
    }
}

// ---------------------------------------------------------------------------
// 4. The adjacency check over 18a §0.4's boxes
// ---------------------------------------------------------------------------

#[test]
fn ic1_every_door_shares_an_edge_and_every_exitless_seam_is_walled() {
    let expected: [(&str, &[&str]); 2] = [
        (
            CERES_PACK,
            &[
                "feature.wall_c01_c08",
                "feature.wall_c01_c10",
                "feature.wall_c02_c10",
                "feature.wall_c05_c06",
                "feature.wall_c06_c09",
                "feature.wall_c08_c10",
            ],
        ),
        (
            STICKNEY_PACK,
            &[
                "feature.wall_p01_p10",
                "feature.wall_p02_p10",
                "feature.wall_p03_p05",
                "feature.wall_p05_p08",
                "feature.wall_p08_p11",
                "feature.wall_p09_p12",
                "feature.wall_p11_p12",
            ],
        ),
    ];
    for (pack, want_walls) in expected {
        let rooms = rooms_of(pack);
        let exits = exits_of(pack);
        let walls = walls_of(pack);
        // Every plain same-floor exit joins two footprints that share an edge.
        let mut joined: BTreeSet<(String, String)> = BTreeSet::new();
        for (entry, exit) in &exits {
            let (Some(a), Some(b)) = (rooms.get(&exit.from), rooms.get(&exit.to)) else {
                continue; // the Limbo door
            };
            joined.insert((exit.from.clone(), exit.to.clone()));
            joined.insert((exit.to.clone(), exit.from.clone()));
            if floor_of(a) != floor_of(b) {
                assert!(
                    exit.portal.at.is_some(),
                    "{entry}: a cross-deck exit gives portal.at"
                );
                continue;
            }
            assert!(
                exit.portal.at.is_none(),
                "{entry}: a same-floor exit gives no portal.at"
            );
            assert!(
                shared_threshold(&shape_of(a), &shape_of(b)).is_some(),
                "{entry}: {} and {} share no edge",
                exit.from,
                exit.to
            );
        }
        // Every pair of public rooms on one floor that share an edge and have no
        // exit carries a wall along exactly that seam — and every wall is such a seam.
        let public: Vec<(&String, &RoomDef)> = rooms
            .iter()
            .filter(|(_, d)| d.access == mush_atlas::Access::Public)
            .collect();
        let mut seams: Vec<(String, String, (PointCm, PointCm))> = Vec::new();
        for (i, (ka, a)) in public.iter().enumerate() {
            for (kb, b) in public.iter().skip(i + 1) {
                if floor_of(a) != floor_of(b) || joined.contains(&((*ka).clone(), (*kb).clone())) {
                    continue;
                }
                if let Some(seam) = shared_threshold(&shape_of(a), &shape_of(b)) {
                    seams.push(((*ka).clone(), (*kb).clone(), seam));
                }
            }
        }
        let mut matched: BTreeSet<String> = BTreeSet::new();
        for (a, b, seam) in &seams {
            let wall = walls.iter().find(|(_, f)| {
                let line = f.polyline.as_ref().expect("a wall is a polyline");
                line.len() == 2 && same_segment((cm(line[0]), cm(line[1])), *seam)
            });
            let Some((wall_id, _)) = wall else {
                panic!("{pack}: the seam {a}|{b} has no wall feature");
            };
            matched.insert(wall_id.clone());
        }
        let all: BTreeSet<String> = walls.keys().cloned().collect();
        assert_eq!(matched, all, "{pack}: every wall lies on an exitless seam");
        assert_eq!(
            all,
            want_walls
                .iter()
                .map(|w| (*w).to_string())
                .collect::<BTreeSet<_>>(),
            "{pack}: 18a's wall list"
        );
    }
    // The validator agrees: no `geometry.portal_gap` (the applier refuses the same).
    let tw = booted();
    for (pack, findings) in station_findings(&tw) {
        assert!(
            !findings.iter().any(|f| f.code == "geometry.portal_gap"),
            "{pack}: {findings:#?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 5. Walled on the map
// ---------------------------------------------------------------------------

#[test]
fn ic1_the_seams_are_walled_on_the_map() {
    let tw = booted();
    let records = &atlas(&tw).records;
    for (pack, structure) in [
        (CERES_PACK, "structure.freeport_hub"),
        (STICKNEY_PACK, "structure.yard_rim"),
    ] {
        let surface = SurfaceRef::Floor {
            structure: RecordKey::new(id(pack), id(structure)),
            index: 1,
        };
        let tiles: Vec<_> = records
            .tiles
            .values()
            .filter(|t| t.surface == surface)
            .collect();
        assert!(!tiles.is_empty(), "{pack}: the Public Deck rasterized");
        let class_at = |p: PointCm| -> TerrainClass {
            for tile in &tiles {
                let bbox = tile.bbox.to_cm().expect("a bbox");
                if p.x < bbox.min.x || p.y < bbox.min.y || p.x >= bbox.max.x || p.y >= bbox.max.y {
                    continue;
                }
                let grid = Grid {
                    origin: bbox.min,
                    resolution_cm: tile.resolution_cm,
                    width: tile.cells.width(),
                    height: tile.cells.height(),
                };
                let (x, y) = grid.cell_of(p).expect("inside the tile");
                return tile.cells.get(x, y).expect("a cell").class;
            }
            panic!("{pack}: no tile holds {p:?}");
        };
        // Every cell the rasterizer's own walk yields along a wall seam is Wall.
        for (wall_id, wall) in walls_of(pack) {
            let line = wall.polyline.expect("a polyline");
            let ends = [cm(line[0]), cm(line[1])];
            let tile = tiles
                .iter()
                .find(|t| {
                    let b = t.bbox.to_cm().expect("a bbox");
                    ends.iter().all(|p| {
                        p.x >= b.min.x && p.y >= b.min.y && p.x <= b.max.x && p.y <= b.max.y
                    })
                })
                .unwrap_or_else(|| panic!("{wall_id}: no tile holds the seam"));
            let bbox = tile.bbox.to_cm().expect("a bbox");
            let grid = Grid {
                origin: bbox.min,
                resolution_cm: tile.resolution_cm,
                width: tile.cells.width(),
                height: tile.cells.height(),
            };
            let cells = grid.cells_along(&ends);
            assert!(!cells.is_empty(), "{wall_id}");
            for (x, y) in cells {
                assert_eq!(
                    tile.cells.get(x, y).map(|c| c.class),
                    Some(TerrainClass::Wall),
                    "{pack}: {wall_id} cell ({x}, {y}) is not walled on +map"
                );
            }
        }
        // At a same-floor door's threshold midpoint, neither cell straddling the
        // seam is Wall (seams lie on whole metres, so the midpoint is on a cell
        // boundary: never "the cell at the midpoint").
        let rooms = rooms_of(pack);
        for (entry, exit) in exits_of(pack) {
            let (Some(a), Some(b)) = (rooms.get(&exit.from), rooms.get(&exit.to)) else {
                continue;
            };
            if floor_of(a) != 1 || floor_of(b) != 1 {
                continue;
            }
            let (s, e) = shared_threshold(&shape_of(a), &shape_of(b)).expect("a threshold");
            let m = PointCm {
                x: (s.x + e.x) / 2,
                y: (s.y + e.y) / 2,
            };
            let half = 50;
            let (p, q) = if s.x == e.x {
                (
                    PointCm {
                        x: m.x - half,
                        y: m.y,
                    },
                    PointCm {
                        x: m.x + half,
                        y: m.y,
                    },
                )
            } else {
                (
                    PointCm {
                        x: m.x,
                        y: m.y - half,
                    },
                    PointCm {
                        x: m.x,
                        y: m.y + half,
                    },
                )
            };
            for side in [p, q] {
                assert_ne!(
                    class_at(side),
                    TerrainClass::Wall,
                    "{pack}: {entry}'s threshold is walled at {side:?}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 6. Stickney and its dock marker
// ---------------------------------------------------------------------------

#[test]
fn ic1_stickney_validates_only_because_of_its_dock_marker() {
    let tw = booted();
    let stickney = station_packs()
        .expect("the station packs")
        .into_iter()
        .find(|p| p.manifest().id().as_str() == STICKNEY_PACK)
        .expect("Stickney");
    let cx = ValidateContext::for_pack(tw.world(), atlas(&tw), content(&tw), &stickney);
    let findings = validate(&stickney, &cx);
    assert_eq!(blocking(&findings), 0, "{findings:#?}");

    // The same pack with P01's markers removed, re-signed.
    let mut doc = pack_json(STICKNEY_PACK);
    let markers = doc["entries"]["room.p01_berth_gallery"]["def"]
        .as_object_mut()
        .expect("P01's def")
        .remove("markers");
    assert_eq!(markers, Some(serde_json::json!(["port.dock"])));
    doc["manifest"]["digest"] = Value::String(UNSIGNED.to_string());
    let unmarked =
        mush_content::load_pack(&sign(&serde_json::to_string_pretty(&doc).expect("json")))
            .expect("the edited pack verifies");
    let cx = ValidateContext::for_pack(tw.world(), atlas(&tw), content(&tw), &unmarked);
    let findings = validate(&unmarked, &cx);
    let unreachable: BTreeSet<String> = findings
        .iter()
        .filter(|f| f.code == "topology.unreachable")
        .filter_map(|f| f.entry.clone())
        .collect();
    let public: BTreeSet<String> = rooms_of(STICKNEY_PACK)
        .into_iter()
        .filter(|(_, d)| d.access == mush_atlas::Access::Public)
        .map(|(k, _)| k)
        .collect();
    assert_eq!(public.len(), 11);
    assert_eq!(unreachable, public, "{findings:#?}");
}

// ---------------------------------------------------------------------------
// 7. A fresh world
// ---------------------------------------------------------------------------

/// The 18a tables' NPCs (the thirteen) and principals (the nine), by entry.
const CAST: [&str; 13] = [
    "npc.oduya",
    "npc.halvorsen",
    "npc.qureshi",
    "npc.sondergaard",
    "npc.akintola",
    "npc.mbatha",
    "npc.petrossian",
    "npc.ilunga_brandt",
    "npc.achterberg",
    "npc.castillo_haugen",
    "npc.adeyinka_rusk",
    "npc.okonkwo",
    "npc.abubakar_lange",
];
const PRINCIPALS: [&str; 9] = [
    "npc.wen",
    "npc.varga",
    "npc.ndiaye",
    "npc.osei_ferrante",
    "npc.holm",
    "npc.kalu",
    "npc.castellanos",
    "npc.tanaka",
    "npc.bask",
];

fn npc_pack(entry: &str) -> &'static str {
    if entries_of(CERES_PACK, "npc").contains_key(entry) {
        CERES_PACK
    } else {
        STICKNEY_PACK
    }
}

fn profile(tw: &TestWorld, npc: ObjId) -> mush_npc::AgentProfile {
    let value = tw
        .world()
        .get(npc)
        .and_then(|o| o.attrs.get(mush_npc::PROFILE_ATTR))
        .and_then(|a| match &a.value {
            mush_core::attr::AttrValue::Json(v) => Some(v.clone()),
            mush_core::attr::AttrValue::Text(_) => None,
        })
        .expect("a profile");
    serde_json::from_value(value).expect("the profile decodes")
}

fn station_rooms(tw: &TestWorld) -> BTreeMap<ObjId, &'static str> {
    atlas(tw)
        .records
        .roomgeo
        .values()
        .filter_map(|g| match g.neighborhood.pack.as_str() {
            CERES_PACK => Some((g.room, CERES_PACK)),
            STICKNEY_PACK => Some((g.room, STICKNEY_PACK)),
            _ => None,
        })
        .collect()
}

#[test]
fn ic1_a_fresh_world_boots_the_stations() {
    let mut tw = booted();
    for (name, _) in STATION_SOURCES {
        let latch = content(&tw)
            .latches
            .applied(&id(name))
            .unwrap_or_else(|| panic!("{name} is latched"));
        assert_eq!(latch.version(), station_version(name), "{name}");
        assert_eq!(latch.anomaly(), None, "{name}");
    }
    let rooms = station_rooms(&tw);
    assert_eq!(
        rooms.values().filter(|p| **p == CERES_PACK).count(),
        CERES_ROOMS
    );
    assert_eq!(
        rooms.values().filter(|p| **p == STICKNEY_PACK).count(),
        STICKNEY_ROOMS
    );
    // `port.dock` on C01 and P01 and no other room.
    let c01 = obj(&tw, CERES_PACK, "room.c01_berth_gallery");
    let p01 = obj(&tw, STICKNEY_PACK, "room.p01_berth_gallery");
    let docks: BTreeSet<ObjId> = tw
        .world()
        .iter()
        .filter(|o| o.kind == ObjKind::Room && tw.world().has_marker(o.id, "port.dock"))
        .map(|o| o.id)
        .collect();
    assert_eq!(docks, BTreeSet::from([c01, p01]));
    // C01's one-line description is 18a's *Short.* line, on `short_desc`.
    assert_eq!(
        tw.world()
            .get(c01)
            .and_then(|o| o.attrs.get("short_desc"))
            .and_then(|a| a.as_text()),
        Some(
            "A long armourglass gallery over the visiting berths, pressure doors to the lobby at its back."
        )
    );
    // Hydration off; twelve ambience lines each.
    for pack in [CERES_PACK, STICKNEY_PACK] {
        let nb = atlas(&tw)
            .records
            .neighborhoods
            .get(&RecordKey::new(id(pack), id("neighborhood")))
            .expect("the neighborhood record");
        assert!(!nb.hydration.enabled, "{pack}");
        assert_eq!(nb.ambience.len(), 12, "{pack}");
    }
    // 22 NPCs in the 25 rooms: the thirteen enabled, fronting real providers, with
    // fallbacks; the nine principals disabled.
    let npcs: Vec<ObjId> = tw
        .world()
        .iter()
        .filter(|o| o.kind == ObjKind::Npc && o.location.is_some_and(|l| rooms.contains_key(&l)))
        .map(|o| o.id)
        .collect();
    assert_eq!(npcs.len(), 22);
    let providers = ["atlas", "travel", "help", "market"];
    for entry in CAST {
        let p = profile(&tw, obj(&tw, npc_pack(entry), entry));
        assert!(p.enabled, "{entry}");
        assert!(!p.fallback_lines.is_empty(), "{entry}");
        assert!(
            p.interfaces.iter().all(|i| providers.contains(&i.as_str())),
            "{entry}: {:?}",
            p.interfaces
        );
    }
    for entry in PRINCIPALS {
        let p = profile(&tw, obj(&tw, npc_pack(entry), entry));
        assert!(!p.enabled, "{entry}");
        assert!(p.interfaces.is_empty(), "{entry}");
    }
    // Three secrets, six storefront items.
    let secrets = atlas(&tw)
        .records
        .secrets
        .keys()
        .filter(|k| [CERES_PACK, STICKNEY_PACK].contains(&k.pack.as_str()))
        .count();
    assert_eq!(secrets, 3);
    let c15 = obj(&tw, CERES_PACK, "room.c15_maintenance");
    let p15 = obj(&tw, STICKNEY_PACK, "room.p15_maintenance");
    let items = tw
        .world()
        .iter()
        .filter(|o| {
            o.kind == ObjKind::Thing && (o.location == Some(c15) || o.location == Some(p15))
        })
        .count();
    // Six storefront items, and since doc 18 cut IC8 each port's stock clerk (a DARK
    // Thing in the staff spine).
    assert_eq!(items, 8);
    // A non-staff player in Limbo types `c` and stands in Port Watch Arrivals.
    let pilot = tw.spawn_player("Pilot");
    let events = tw.run(pilot, "c");
    assert_eq!(
        location(&tw, pilot),
        Some(obj(&tw, CERES_PACK, "room.c03_arrivals")),
        "{}",
        said(&tw, &events, pilot)
    );
}

// ---------------------------------------------------------------------------
// 8. A kept world
// ---------------------------------------------------------------------------

#[test]
fn ic1_a_kept_world_takes_the_stations_at_its_next_boot() {
    // First boot: the old list, the starter's three and the proving ground's two.
    let mut old = mush_atlas::starter::packs().expect("the starter packs");
    old.extend(boot_packs().expect("the game packs"));
    let mut tw = TestWorld::with_modules(support::roster_booting(config(), old));
    let walker = tw.spawn_player("Walker");
    let _ = tw.run(walker, "o");
    let ooc = location(&tw, walker).expect("somewhere");
    assert_ne!(ooc, LIMBO, "`o` walked into the OOC area");
    let before = latches(&tw);
    assert_eq!(before.len(), 5, "{:?}", before.keys().collect::<Vec<_>>());
    let mut store = fold_module_ops(tw.take_module_ops());

    // Second boot: a new binary over the kept store.
    let mut tw = tw
        .reboot_into(production_roster_with_starter(config()), store.clone())
        .expect("a kept world boots the new list without refusal");
    let after = latches(&tw);
    for (pack, latch) in &before {
        assert_eq!(
            after.get(pack),
            Some(latch),
            "{pack} was re-applied or changed"
        );
    }
    for (name, _) in STATION_SOURCES {
        assert_eq!(
            after.get(name).map(|l| l.0),
            Some(station_version(name)),
            "{name}"
        );
    }
    // Doc 18 cut IC2: the lane too (was nine latches), at version 3 since cut IC8.
    assert_eq!(after.get(LANE_PACK).map(|l| l.0), Some(3), "{LANE_PACK}");
    assert_eq!(after.len(), 10);
    assert_eq!(
        location(&tw, walker),
        Some(ooc),
        "the walker stands where they stood"
    );
    let fresh = tw.spawn_player("Fresh");
    let events = tw.run(fresh, "c");
    assert_eq!(
        location(&tw, fresh),
        Some(obj(&tw, CERES_PACK, "room.c03_arrivals")),
        "{}",
        said(&tw, &events, fresh)
    );

    // Third boot: the store is every batch drained since the first boot.
    fold_module_ops_into(&mut store, tw.take_module_ops());
    let objects = live_objects(&tw);
    let next = tw.world().iter().map(|o| o.id.0).max();
    let latched = latches(&tw);
    let tw = tw
        .reboot_into(production_roster_with_starter(config()), store)
        .expect("the third boot boots");
    assert_eq!(live_objects(&tw), objects, "a third boot creates no object");
    assert_eq!(tw.world().iter().map(|o| o.id.0).max(), next);
    assert_eq!(latches(&tw), latched, "a third boot changes no latch");
}

// ---------------------------------------------------------------------------
// 9. Every public room is walkable
// ---------------------------------------------------------------------------

#[test]
fn ic1_every_public_room_is_walkable_by_its_authored_aliases() {
    let mut tw = booted();
    let walker = tw.spawn_player("Walker");
    let mut locked = Vec::new();
    for pack in [CERES_PACK, STICKNEY_PACK] {
        for (entry, exit) in exits_of(pack) {
            let from = obj(&tw, pack, &exit.from);
            let to = obj(&tw, pack, &exit.to);
            let there = first_alias(&exit.names[0]);
            let back = first_alias(&exit.names[1]);
            if exit.lock.is_some() {
                locked.push(entry.clone());
                tw.world_mut().move_to(walker, from).expect("placed");
                let events = tw.run(walker, there);
                assert_eq!(
                    location(&tw, walker),
                    Some(from),
                    "{entry}: a player passed a staff lock: {}",
                    said(&tw, &events, walker)
                );
                tw.world_mut().move_to(GOD, from).expect("placed");
                let events = tw.run(GOD, there);
                assert_eq!(
                    location(&tw, GOD),
                    Some(to),
                    "{entry}: {}",
                    said(&tw, &events, GOD)
                );
                let events = tw.run(GOD, back);
                assert_eq!(
                    location(&tw, GOD),
                    Some(from),
                    "{entry}: {}",
                    said(&tw, &events, GOD)
                );
                continue;
            }
            // The Limbo leg by a fresh player, whom no earlier leg has stamped.
            let who = if from == LIMBO {
                tw.spawn_player("Newcomer")
            } else {
                tw.world_mut().move_to(walker, from).expect("placed");
                walker
            };
            let events = tw.run(who, there);
            assert_eq!(
                location(&tw, who),
                Some(to),
                "{entry}: `{there}` from {}: {}",
                exit.from,
                said(&tw, &events, who)
            );
            let events = tw.run(who, back);
            assert_eq!(
                location(&tw, who),
                Some(from),
                "{entry}: `{back}` back from {}: {}",
                exit.to,
                said(&tw, &events, who)
            );
        }
    }
    locked.sort();
    assert_eq!(locked, ["exit.c05_c16", "exit.c14_c15", "exit.p07_p15"]);
}

// ---------------------------------------------------------------------------
// 10–11. The IC door and `+ic`
// ---------------------------------------------------------------------------

#[test]
fn ic1_the_ic_door_refuses_a_pilot_who_left_at_stickney() {
    let mut tw = booted();
    let ooc = obj(&tw, mush_atlas::starter::OOC_AREA_PACK, "room.welcome_hall");
    let p01 = obj(&tw, STICKNEY_PACK, "room.p01_berth_gallery");
    let p04 = obj(&tw, STICKNEY_PACK, "room.p04_concourse");
    let pilot = tw.spawn_player("Pilot");
    tw.world_mut().move_to(pilot, p04).expect("placed");
    // Limbo's global `o`, typed at Stickney, is a way out of the port: stamped.
    let _ = tw.run(pilot, "o");
    assert_eq!(location(&tw, pilot), Some(ooc));
    assert_eq!(ic_return::last_port(tw.world(), pilot), Some(&PORTS[1]));
    let _ = tw.run(pilot, "x");
    assert_eq!(location(&tw, pilot), Some(LIMBO));
    // The door refuses with doc 18 §4.5's line.
    let events = tw.run(pilot, "c");
    let out = said(&tw, &events, pilot);
    assert!(
        out.contains("You left the corridor at Stickney Yards; +ic returns you there."),
        "{out}"
    );
    assert_eq!(location(&tw, pilot), Some(LIMBO));
    // `+ic` puts the pilot back in the berth gallery.
    let events = tw.run(pilot, "+ic");
    let out = said(&tw, &events, pilot);
    assert!(
        out.contains("You return to the corridor at Stickney Yards."),
        "{out}"
    );
    assert_eq!(location(&tw, pilot), Some(p01));
    // Limbo's global `c`, typed in P01 (which has no local `c`), meets the door.
    let events = tw.run(pilot, "c");
    let out = said(&tw, &events, pilot);
    assert!(
        out.contains(
            "There is no door from Stickney Yards to Ceres Freeport; the way there is by ship."
        ),
        "{out}"
    );
    assert_eq!(location(&tw, pilot), Some(p01));
    let events = tw.run(pilot, "+ic");
    assert!(said(&tw, &events, pilot).contains("You are already in the corridor."));
    assert_eq!(location(&tw, pilot), Some(p01));
    // `home` from Stickney stamps too: the skip the rule exists to close.
    let second = tw.spawn_player("Second");
    tw.world_mut().move_to(second, p04).expect("placed");
    let _ = tw.run(second, "home");
    assert_eq!(location(&tw, second), Some(LIMBO));
    assert_eq!(ic_return::last_port(tw.world(), second), Some(&PORTS[1]));
    assert!(tw.world().has_marker(second, "tacsim.last_port.stickney"));
    assert!(!tw.world().has_marker(second, "tacsim.last_port.ceres"));
}

#[test]
fn ic1_ic_and_the_door_for_a_ceres_pilot_and_a_new_one() {
    let mut tw = booted();
    let c03 = obj(&tw, CERES_PACK, "room.c03_arrivals");
    let pilot = tw.spawn_player("Pilot");
    assert_eq!(ic_return::last_port(tw.world(), pilot), None);
    let _ = tw.run(pilot, "c");
    assert_eq!(location(&tw, pilot), Some(c03));
    let _ = tw.run(pilot, "x");
    assert_eq!(location(&tw, pilot), Some(LIMBO));
    assert_eq!(ic_return::last_port(tw.world(), pilot), Some(&PORTS[0]));
    let _ = tw.run(pilot, "c");
    assert_eq!(
        location(&tw, pilot),
        Some(c03),
        "a Ceres stamp passes the door"
    );
    let _ = tw.run(pilot, "x");
    let events = tw.run(pilot, "+ic");
    assert!(said(&tw, &events, pilot).contains("You return to the corridor at Ceres Freeport."));
    assert_eq!(location(&tw, pilot), Some(c03));
    // A character with no stamp goes to Ceres, where the Limbo door goes.
    let fresh = tw.spawn_player("Fresh");
    let _ = tw.run(fresh, "+ic");
    assert_eq!(location(&tw, fresh), Some(c03));
}

// ---------------------------------------------------------------------------
// 12. The principals never wake
// ---------------------------------------------------------------------------

#[test]
fn ic1_the_principals_never_wake() {
    let mut tw = booted();
    let c05 = obj(&tw, CERES_PACK, "room.c05_registry");
    let c16 = obj(&tw, CERES_PACK, "room.c16_authority");
    let kalu = obj(&tw, CERES_PACK, "npc.kalu");
    let wen = obj(&tw, CERES_PACK, "npc.wen");
    let oduya = obj(&tw, CERES_PACK, "npc.oduya");
    for principal in PRINCIPALS {
        assert!(
            !profile(&tw, obj(&tw, npc_pack(principal), principal)).enabled,
            "{principal}"
        );
    }
    tw.set_now(1_000);
    let pilot = tw.spawn_player("Pilot");
    tw.world_mut().move_to(pilot, c05).expect("placed");
    tw.world_mut().move_to(GOD, c16).expect("placed");
    let _ = tw.run(pilot, "say Matthias Kalu, a word about Thursdays?");
    let _ = tw.run(GOD, "say Tomasz Wen, a word?");
    let _ = tw.run(pilot, "say Ilse Oduya-Brandt, how do I enrol?");
    // Past every cooldown and coalescing window, driving each NPC turn the tick
    // defers exactly as the world task does: the off-loop runner on the LLM seam
    // (here the shipped default, `[llm].enabled = false`, no route, no provider),
    // then `ModuleAsyncDone` back through the module's completion. A disabled
    // service is an error to the turn, which degrades to one fallback line
    // (`mush-npc/src/turn.rs` `degrade`), so the enabled NPC's answer is observable.
    let llm = mush_llm::LlmService::builder()
        .global_enabled(false)
        .build();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime for the off-loop turn");
    let mut heard = Vec::new();
    let mut turns: Vec<ObjId> = Vec::new();
    for step in 1..=12 {
        let now = 1_000 + step * 30;
        tw.set_now(now);
        let mut out = mush_core::Outbox::new();
        let mut fx = mush_core::Effects::new();
        tw.engine_mut().fire_due(now * 1_000, &mut out, &mut fx);
        heard.extend(out.take());
        for effect in fx.drain() {
            let mush_core::Effect::ModuleAsync {
                module,
                actor,
                correlation,
                payload,
            } = effect
            else {
                continue;
            };
            if module.as_str() != "npc" {
                continue;
            }
            turns.push(actor);
            let outcome = rt.block_on(mush_npc::run_npc_turn_async(&llm, &payload));
            heard.extend(tw.apply(mush_core::WorldCommand::ModuleAsyncDone {
                module,
                actor,
                correlation,
                payload: outcome,
            }));
        }
    }
    let spoken_by = |name: &str| -> Vec<String> {
        heard
            .iter()
            .filter_map(|(_, ev)| match ev {
                OutputEvent::Narrative {
                    actor: Some(a),
                    text,
                    ..
                } if a.name == name => Some(text.to_plain_text()),
                _ => None,
            })
            .collect()
    };
    let host = tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .expect("the NPC host");
    for (name, principal) in [("Matthias Kalu", kalu), ("Tomasz Wen", wen)] {
        let woke = host
            .runtime(principal)
            .is_some_and(|rt| !rt.mailbox.is_empty() || rt.in_flight || rt.last_turn_at.is_some());
        assert!(!woke, "{name} woke");
        assert!(!turns.contains(&principal), "{name} took a turn");
        let spoke = spoken_by(name);
        assert!(spoke.is_empty(), "{name} spoke: {spoke:?}");
    }
    // The enabled NPC took the stimulus, fired her turn, and with the LLM off
    // answered the room with one of her pack's fallback lines, verbatim.
    assert!(turns.contains(&oduya), "Oduya never fired a turn");
    let fallbacks: Vec<String> = entries_of(CERES_PACK, "npc")["npc.oduya"]["profile"]["fallback"]
        .as_array()
        .expect("Oduya's fallback lines")
        .iter()
        .filter_map(|l| l.as_str().map(str::to_owned))
        .collect();
    assert!(!fallbacks.is_empty(), "Oduya ships fallback lines");
    let said_by_oduya = spoken_by("Ilse Oduya-Brandt");
    assert!(
        said_by_oduya
            .iter()
            .any(|t| fallbacks.iter().any(|f| t.contains(f.as_str()))),
        "Oduya did not answer with a fallback line: {said_by_oduya:?}"
    );
}

//! **Doc 18 cut IC1 acceptance** — the trade run's two IC stations, the IC door
//! and the IC-return rule, over the production composition (doc 18 §4.5, §4.6,
//! §4.7; D-18-3, D-18-19, D-18-35, D-18-49; 18a §0–§2).
//!
//! 1. the station packs (the corridor's four and, since doc 19 cut A.8, the moon
//!    ports' six) load, verify and are pinned (re-sign in place with
//!    `TACSIM_WRITE_CONTENT=1`, as `tests/content.rs` does);
//! 2. the production boot list and the atlas shelf carry them in order;
//! 3. they validate clean on the production roster;
//! 4. every door shares an edge and every exitless public seam is walled (pure,
//!    over 18a §0.4's boxes);
//! 5. the walls are walls on the rasterized floor each stands on, a door's seam
//!    is open, and every cross-deck portal point stands strictly inside both its
//!    rooms and clear of every restricted room;
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

use mush_atlas::def::{ExitDef, FeatureDef, PortalAt, RoomDef, SurfaceDef};
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
    AITKEN_PACK, AITKEN_REGION_PACK, AITKEN_ROOMS, CERES_PACK, CERES_REGION_PACK, CERES_ROOMS,
    LANE_PACK, LANE_VERSION, LIGEIA_PACK, LIGEIA_REGION_PACK, LIGEIA_ROOMS, STATION_SOURCES,
    STICKNEY_PACK, STICKNEY_REGION_PACK, STICKNEY_ROOMS, VOLTAIRE_PACK, VOLTAIRE_REGION_PACK,
    VOLTAIRE_ROOMS, boot_packs, station_packs,
};
use tacsim::ic_return::{self, PORTS};

use support::ground::{UNSIGNED, sign};

// ---------------------------------------------------------------------------
// The pins
// ---------------------------------------------------------------------------

/// **The version each station pack ships at**: the region packs at 1;
/// `neighborhood.ceres_freeport` at 2 and `neighborhood.stickney_yards` at 3 since
/// doc 18 cut IC12 re-published both (the five 18a details under plain keys — C09
/// `hull registry board`, C14 `sailing plan forms`, P05 `sailing plan pad`, P11
/// `yard time board` and `insurers mark` — and Ceres' verb meta-text struck);
/// Stickney was 2 from cut IC8 (the P11 fittings card's text, lead 2c). Doc 18
/// cut IC14 (row 50, lead ruling L26) re-published Stickney at 4 with three verb
/// meta-text clauses struck — P04 `shift boards`, P07 `intake board`, P08 `price
/// board` — and nothing else ([`stickney_v3_json`] rebuilds v3 to its old pin).
fn station_version(pack: &str) -> u32 {
    match pack {
        CERES_PACK => 2,
        STICKNEY_PACK => 4,
        _ => 1,
    }
}

/// The pinned digests, in `STATION_SOURCES` order. An edit to a pack changes its
/// digest; the failure message prints the new values so a *reviewed* edit is a
/// line here plus the `digest` field in the pack itself.
const PINNED: [(&str, &str); 10] = [
    (
        CERES_REGION_PACK,
        "80b18bc047d703b5c6f2899155d4f914b68a190ab73a0e5695ffbaab6a4f83f0",
    ),
    (
        CERES_PACK,
        "641e1a3983447cca1a05bed6a839f98fe68c2afa683e0deee0c5e4e2904d8538",
    ),
    (
        STICKNEY_REGION_PACK,
        "07e25b41e8e66f29e6d26773472f3783216f3c8d8967f5f2c091075047eeb2b1",
    ),
    (
        STICKNEY_PACK,
        "f1bdc803c5adca043d576013fb5c94bc2b4b6f0434e91f82fe123fcbab2f16f6",
    ),
    // Doc 19 cut A.8: the three moon stations.
    (
        AITKEN_REGION_PACK,
        "4dd766c3ee4a31b0558502b93e2999366e4df3151cf878068bf61767af47f9c7",
    ),
    (
        AITKEN_PACK,
        "7835fd46cb5d2e6fb6c8110e3e17ff3173fc2abf7c16e9a2ff257eebbaf42eaa",
    ),
    (
        VOLTAIRE_REGION_PACK,
        "b95670c974f68c50eaa7ba794e0810de145ab63a81dc8df252239b6bc273b619",
    ),
    (
        VOLTAIRE_PACK,
        "790d3363d00cdbdf72dc2c753e587b9f6f86b36cd17e89e71ded6ed02e4e4bfc",
    ),
    (
        LIGEIA_REGION_PACK,
        "73123abe208afc43b9dff7424fd3d3d004c35698025db3f8b71ed09d107861fe",
    ),
    (
        LIGEIA_PACK,
        "7ef48113fdf50c208c62c102228da9f017f4ae7e8deecb4871000bb2de9436c0",
    ),
];

/// The five neighborhood packs (doc 19 cut A.8: the three moon stations joined).
const NEIGHBORHOODS: [&str; 5] = [
    CERES_PACK,
    STICKNEY_PACK,
    AITKEN_PACK,
    VOLTAIRE_PACK,
    LIGEIA_PACK,
];

/// Stickney v3's pinned digest (doc 18 cut IC12), the content [`stickney_v3_json`]
/// rebuilds.
const STICKNEY_V3_DIGEST: &str = "df2e5af11f18c4a5c71bea9e0b5b187b7cdb97c037aecc6b433b9ee3a0d1b2eb";

/// The files under `crates/tacsim/content/`, in `STATION_SOURCES` order.
const FILES: [&str; 10] = [
    "atlas/region.ceres_freeport.json",
    "atlas/neighborhood.ceres_freeport.json",
    "atlas/region.stickney_yards.json",
    "atlas/neighborhood.stickney_yards.json",
    "atlas/region.aitken_highport.json",
    "atlas/neighborhood.aitken_highport.json",
    "atlas/region.voltaire_anchorage.json",
    "atlas/neighborhood.voltaire_anchorage.json",
    "atlas/region.ligeia_terminal.json",
    "atlas/neighborhood.ligeia_terminal.json",
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
/// so the ten station packs boot after the proving ground. A copy of
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
    assert_eq!(
        station_packs().expect("the station packs verify").len(),
        10,
        "the corridor's four and doc 19 cut A.8's six"
    );
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
    // Doc 18 cut IC2: the lane after the stations (the game's thirteen boot packs
    // since doc 19 cut A.8).
    want.push(LANE_PACK.to_string());
    assert_eq!(ids(&full), want);
    assert_eq!(
        full.len(),
        16,
        "three starter, two proving ground, ten stations, the lane"
    );
    // Region packs require nothing; each neighborhood requires its region at 1.
    for pack in &full[5..] {
        let requires: Vec<(String, u32)> = pack
            .manifest()
            .requires()
            .iter()
            .map(|r| (r.id.to_string(), r.version))
            .collect();
        match pack.manifest().id().as_str() {
            CERES_REGION_PACK | STICKNEY_REGION_PACK | AITKEN_REGION_PACK
            | VOLTAIRE_REGION_PACK | LIGEIA_REGION_PACK => assert!(requires.is_empty()),
            CERES_PACK => assert_eq!(requires, vec![(CERES_REGION_PACK.to_string(), 1)]),
            STICKNEY_PACK => assert_eq!(requires, vec![(STICKNEY_REGION_PACK.to_string(), 1)]),
            // Doc 19 cut A.8: the moon stations.
            AITKEN_PACK => assert_eq!(requires, vec![(AITKEN_REGION_PACK.to_string(), 1)]),
            VOLTAIRE_PACK => assert_eq!(requires, vec![(VOLTAIRE_REGION_PACK.to_string(), 1)]),
            LIGEIA_PACK => assert_eq!(requires, vec![(LIGEIA_REGION_PACK.to_string(), 1)]),
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
    // On the production roster the shelf lists all ten, latched.
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
        out.contains(&format!(" {LANE_PACK}@{LANE_VERSION} (")),
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
    let expected: [(&str, &[&str]); 5] = [
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
        // Doc 19 cut A.8: the moon stations' walls.
        (AITKEN_PACK, &["feature.wall_l01_l12"]),
        (
            VOLTAIRE_PACK,
            &[
                "feature.wall_d01_d04",
                "feature.wall_d01_d06",
                "feature.wall_d02_d04",
                "feature.wall_d03_d05",
                "feature.wall_d05_d08",
                "feature.wall_d08_d09",
                "feature.wall_d09_d10",
            ],
        ),
        (
            LIGEIA_PACK,
            &[
                "feature.wall_t01_t04",
                "feature.wall_t01_t05",
                "feature.wall_t05_t07",
                "feature.wall_t10_t06",
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
    // Doc 19 cut A.8: all five stations, each wall and each door on its own deck
    // (Aitken's one wall stands on the Drum Deck, floor 0; every other on floor 1).
    let mut walls_checked = 0;
    for (pack, structure) in [
        (CERES_PACK, "structure.freeport_hub"),
        (STICKNEY_PACK, "structure.yard_rim"),
        (AITKEN_PACK, "structure.aitken_drum"),
        (VOLTAIRE_PACK, "structure.moorings_block"),
        (LIGEIA_PACK, "structure.ligeia_spar"),
    ] {
        let on_structure = |surface: &SurfaceDef| -> i16 {
            match surface {
                SurfaceDef::Floor(s, index) if s == structure => *index,
                other => panic!("{pack}: {other:?} is not on {structure}"),
            }
        };
        let tiles_of = |index: i16| {
            let surface = SurfaceRef::Floor {
                structure: RecordKey::new(id(pack), id(structure)),
                index,
            };
            let tiles: Vec<_> = records
                .tiles
                .values()
                .filter(|t| t.surface == surface)
                .collect();
            assert!(!tiles.is_empty(), "{pack}: floor {index} rasterized");
            tiles
        };
        let class_at = |index: i16, p: PointCm| -> TerrainClass {
            for tile in tiles_of(index) {
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
            panic!("{pack}: no floor {index} tile holds {p:?}");
        };
        // Every cell the rasterizer's own walk yields along a wall seam is Wall,
        // on the floor the wall is authored on.
        for (wall_id, wall) in walls_of(pack) {
            let index = on_structure(&wall.surface);
            let tiles = tiles_of(index);
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
                .unwrap_or_else(|| panic!("{wall_id}: no floor {index} tile holds the seam"));
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
                    "{pack}: {wall_id} cell ({x}, {y}) on floor {index} is not walled on +map"
                );
            }
            walls_checked += 1;
        }
        // At a same-floor door's threshold midpoint, neither cell straddling the
        // seam is Wall (seams lie on whole metres, so the midpoint is on a cell
        // boundary: never "the cell at the midpoint") — on whichever floor the
        // door joins its two rooms.
        let rooms = rooms_of(pack);
        for (entry, exit) in exits_of(pack) {
            let (Some(a), Some(b)) = (rooms.get(&exit.from), rooms.get(&exit.to)) else {
                continue;
            };
            let index = on_structure(&a.surface);
            if index != on_structure(&b.surface) {
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
                    class_at(index, side),
                    TerrainClass::Wall,
                    "{pack}: {entry}'s threshold on floor {index} is walled at {side:?}"
                );
            }
        }
    }
    // 18a's thirteen corridor walls and doc 19 cut A.8's twelve.
    assert_eq!(walls_checked, 6 + 7 + 1 + 7 + 4);
}

/// Whether `p` lies on an edge of `shape` (a disc's boundary is outside
/// [`Shape::contains`] already).
fn on_edge(shape: &Shape, p: PointCm) -> bool {
    match shape {
        Shape::Polygon(poly) => poly.edges().any(|(a, b)| {
            let cross = i128::from(b.x - a.x) * i128::from(p.y - a.y)
                - i128::from(b.y - a.y) * i128::from(p.x - a.x);
            cross == 0
                && p.x >= a.x.min(b.x)
                && p.x <= a.x.max(b.x)
                && p.y >= a.y.min(b.y)
                && p.y <= a.y.max(b.y)
        }),
        Shape::Disc(_) => false,
    }
}

/// **Doc 19 cut A.8 (review)**: the validator skips geometry for an exit that
/// gives `portal.at` and the applier takes the point as given (design §4.1), so
/// the packs must place it right: every cross-deck exit's point lies strictly
/// inside both rooms' footprints, and outside (not on the edge of) every
/// restricted room on the decks it spans other than its own two ends.
#[test]
fn ic1_every_cross_deck_portal_stands_inside_both_rooms_and_clear_of_restricted_ones() {
    let mut checked = 0;
    for pack in NEIGHBORHOODS {
        let rooms = rooms_of(pack);
        for (entry, exit) in exits_of(pack) {
            let (Some(a), Some(b)) = (rooms.get(&exit.from), rooms.get(&exit.to)) else {
                continue; // the Limbo door
            };
            let (fa, fb) = (floor_of(a), floor_of(b));
            if fa == fb {
                continue;
            }
            let ends: Vec<mush_atlas::Point3> = match exit.portal.at.clone() {
                Some(PortalAt::Point(p)) => vec![p],
                Some(PortalAt::Segment(p, q)) => vec![p, q],
                None => panic!("{pack}: {entry}: a cross-deck exit gives portal.at"),
            };
            let (lo, hi) = (fa.min(fb), fa.max(fb));
            for end in ends {
                let p = cm(mush_atlas::Point2(end.0, end.1));
                for (room, def) in [(&exit.from, a), (&exit.to, b)] {
                    let shape = shape_of(def);
                    assert!(
                        shape.contains(p) && !on_edge(&shape, p),
                        "{pack}: {entry}'s portal {p:?} is not strictly inside {room}"
                    );
                }
                for (room, def) in &rooms {
                    if room == &exit.from
                        || room == &exit.to
                        || def.access != mush_atlas::Access::Restricted
                        || !(lo..=hi).contains(&floor_of(def))
                    {
                        continue;
                    }
                    let shape = shape_of(def);
                    assert!(
                        !shape.contains(p) && !on_edge(&shape, p),
                        "{pack}: {entry}'s portal {p:?} stands in the restricted {room}"
                    );
                }
            }
            checked += 1;
        }
    }
    // Ceres's two, Stickney's one; Aitken's two, Voltaire's one, Ligeia's one.
    assert_eq!(checked, 2 + 1 + 2 + 1 + 1);
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
const CAST: [&str; 34] = [
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
    // Doc 19 cut A.8: the moon stations' cast.
    "npc.albrecht_sato",
    "npc.batista_okeke",
    "npc.devereux_anand",
    "npc.ferro_wainwright",
    "npc.holt_iversen",
    "npc.kowalczyk_amadi",
    "npc.mwangi_eriksen",
    "npc.okafor_thistlewood",
    "npc.rasmussen_oto",
    "npc.abiodun_fleck",
    "npc.kettunen_ade",
    "npc.rautio_grant",
    "npc.strand_villanueva",
    "npc.szabo_mensah",
    "npc.tsai_barros",
    "npc.abara_vos",
    "npc.adichie_brun",
    "npc.marangoni_teague",
    "npc.oyelowo_krauss",
    "npc.quayle_ibekwe",
    "npc.tupou_lindahl",
];
const PRINCIPALS: [&str; 18] = [
    "npc.wen",
    "npc.varga",
    "npc.ndiaye",
    "npc.osei_ferrante",
    "npc.holm",
    "npc.kalu",
    "npc.castellanos",
    "npc.tanaka",
    "npc.bask",
    // Doc 19 cut A.8: the moon stations' principals.
    "npc.ferreira_lund",
    "npc.kwarteng_fairweather",
    "npc.moravec_ansah",
    "npc.rourke_haddad",
    "npc.bergstrom_nwachukwu",
    "npc.delgado_mertens",
    "npc.okwuosa_brandao",
    "npc.asante_pryce",
    "npc.varela_nwosu",
];

fn npc_pack(entry: &str) -> &'static str {
    STATION_SOURCES
        .iter()
        .map(|(pack, _)| *pack)
        .find(|pack| entries_of(pack, "npc").contains_key(entry))
        .unwrap_or_else(|| panic!("{entry} is an NPC of no station pack"))
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
        .filter_map(|g| {
            STATION_SOURCES
                .iter()
                .find(|(pack, _)| *pack == g.neighborhood.pack.as_str())
                .map(|(pack, _)| (g.room, *pack))
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
    // Doc 19 cut A.8: the moon stations.
    assert_eq!(
        rooms.values().filter(|p| **p == AITKEN_PACK).count(),
        AITKEN_ROOMS
    );
    assert_eq!(
        rooms.values().filter(|p| **p == VOLTAIRE_PACK).count(),
        VOLTAIRE_ROOMS
    );
    assert_eq!(
        rooms.values().filter(|p| **p == LIGEIA_PACK).count(),
        LIGEIA_ROOMS
    );
    // `port.dock` on the five galleries and no other room (doc 19 cut A.8: L01,
    // D01 and T01 joined C01 and P01).
    let c01 = obj(&tw, CERES_PACK, "room.c01_berth_gallery");
    let p01 = obj(&tw, STICKNEY_PACK, "room.p01_berth_gallery");
    let l01 = obj(&tw, AITKEN_PACK, "room.l01_berth_gallery");
    let d01 = obj(&tw, VOLTAIRE_PACK, "room.d01_berth_gallery");
    let t01 = obj(&tw, LIGEIA_PACK, "room.t01_berth_gallery");
    let docks: BTreeSet<ObjId> = tw
        .world()
        .iter()
        .filter(|o| o.kind == ObjKind::Room && tw.world().has_marker(o.id, "port.dock"))
        .map(|o| o.id)
        .collect();
    assert_eq!(docks, BTreeSet::from([c01, p01, l01, d01, t01]));
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
    for pack in NEIGHBORHOODS {
        let nb = atlas(&tw)
            .records
            .neighborhoods
            .get(&RecordKey::new(id(pack), id("neighborhood")))
            .expect("the neighborhood record");
        assert!(!nb.hydration.enabled, "{pack}");
        assert_eq!(nb.ambience.len(), 12, "{pack}");
    }
    // 22 NPCs in the corridor's 25 rooms and the moons' own (doc 19 cut A.8): the cast
    // enabled, fronting real providers, with fallbacks; the principals disabled.
    let npcs: Vec<ObjId> = tw
        .world()
        .iter()
        .filter(|o| o.kind == ObjKind::Npc && o.location.is_some_and(|l| rooms.contains_key(&l)))
        .map(|o| o.id)
        .collect();
    // Doc 19 cut A.8: the moon stations' 30 (cast and principals) joined the 22.
    assert_eq!(npcs.len(), 52);
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
        .filter(|k| NEIGHBORHOODS.contains(&k.pack.as_str()))
        .count();
    assert_eq!(secrets, 6, "three of the corridor and the moons' 3");
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
    assert_eq!(
        after.get(LANE_PACK).map(|l| l.0),
        Some(LANE_VERSION),
        "{LANE_PACK}"
    );
    assert_eq!(after.len(), 16, "five, ten stations and the lane");
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
    for pack in NEIGHBORHOODS {
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
    assert_eq!(
        locked,
        [
            "exit.c05_c16",
            "exit.c14_c15",
            "exit.d07_d15",
            "exit.l08_l15",
            "exit.p07_p15",
            "exit.t11_t15"
        ]
    );
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

// ---------------------------------------------------------------------------
// Doc 18 cut IC12: the five details, the old versions, and `+ic`'s refusals
// ---------------------------------------------------------------------------

/// The five 18a details cut IC12 ships, as `(pack, room entry, key, opening words)`.
const IC12_DETAILS: [(&str, &str, &str, &str); 5] = [
    (
        CERES_PACK,
        "room.c09_tycho",
        "hull registry board",
        "Registry marks, hull names, insured value, title holder.",
    ),
    (
        CERES_PACK,
        "room.c14_dockmaster",
        "sailing plan forms",
        "DEPART AT. LOAD: WHAT, FROM WHERE, WHEN.",
    ),
    (
        STICKNEY_PACK,
        "room.p05_harbourmaster",
        "sailing plan pad",
        "Departure at, destination, manifest sealed Y/N,",
    ),
    (
        STICKNEY_PACK,
        "room.p11_service",
        "yard time board",
        "Drydock-days by cradle.",
    ),
    (
        STICKNEY_PACK,
        "room.p11_service",
        "insurers mark",
        "TYCHO MUTUAL ASSURANCE & SALVAGE",
    ),
];

/// The one money figure a station detail may print: the Yard's list price for a
/// new Dray, a fiction figure with no live render (cut IC12 lead ruling L8, owner
/// item O3). Every live market figure is on a board or a listing, never painted.
const FICTION_FIGURE: &str = "120,000cr";

/// Whether `text` paints a money figure other than [`FICTION_FIGURE`]: a digit
/// directly followed by `cr`.
fn paints_a_price(text: &str) -> bool {
    let scrubbed = text.replace(FICTION_FIGURE, "");
    scrubbed
        .as_bytes()
        .windows(3)
        .any(|w| w[0].is_ascii_digit() && w[1] == b'c' && w[2] == b'r')
}

/// **The five details ship under plain keys** (doc 18 cut IC12 items 4.1–4.2; 18a
/// §1.3 C09/C14, §2.3 P05/P11; lead ruling L7: the 18a names with hyphens and the
/// apostrophe dropped). A fresh boot latches Ceres at 2 and Stickney at 4; a player
/// standing in each room reads each detail by its key; no detail in either pack
/// paints a market figure or names an engine word; and no detail in either pack
/// names a verb (Ceres since IC12, Stickney since its v4, doc 18 cut IC14 row 50).
#[test]
fn ic12_the_five_details_ship_under_plain_keys() {
    let mut tw = booted();
    for (name, _) in STATION_SOURCES {
        assert_eq!(
            content(&tw).latches.applied(&id(name)).map(|l| l.version()),
            Some(station_version(name)),
            "{name}"
        );
    }
    let reader = tw.spawn_player("Reader");
    for (pack, room, key, opening) in IC12_DETAILS {
        let text = rooms_of(pack)[room]
            .details
            .get(key)
            .cloned()
            .unwrap_or_else(|| panic!("{room} ships `{key}`"));
        assert!(text.starts_with(opening), "{room} `{key}`: {text}");
        let here = obj(&tw, pack, room);
        tw.world_mut().move_to(reader, here).expect("placed");
        let events = tw.run(reader, &format!("look {key}"));
        let out = said(&tw, &events, reader);
        assert!(out.contains(opening), "`look {key}` in {room}: {out}");
    }
    for pack in NEIGHBORHOODS {
        for (room, def) in rooms_of(pack) {
            for (key, text) in &def.details {
                assert!(
                    !paints_a_price(text),
                    "{pack} {room} `{key}` paints a price: {text}"
                );
                for word in ["LookRendered", "look_rendered"] {
                    assert!(!text.contains(word), "{pack} {room} `{key}`: {word}");
                }
            }
        }
    }
    // The verb meta-text is struck (lead ruling L8's "engine word"; Stickney's
    // three clauses by IC14's v4): no `+` directly before a letter anywhere in a
    // detail of either pack — a verb in parentheses or quotes is caught as surely
    // as one standing alone.
    for pack in NEIGHBORHOODS {
        for (room, def) in rooms_of(pack) {
            for (key, text) in &def.details {
                assert!(
                    !names_a_verb(text),
                    "{pack} {room} `{key}` names a verb: {text}"
                );
            }
        }
    }
}

/// Stickney's three v3 clauses IC14's v4 struck, as `(room, detail, v4 text, v3
/// text)`: each v3 text is its v4 text with the clause and its punctuation back.
const STICKNEY_V3_CLAUSES: [(&str, &str, &str, &str); 3] = [
    (
        "room.p04_concourse",
        "shift boards",
        " marked YARD PERSONNEL.",
        " marked YARD PERSONNEL (+atlas has the same map, in words).",
    ),
    (
        "room.p07_receiving",
        "intake board",
        "the figures are the board's own and change",
        "the figures are the board's own (it is what +market/list prints here) and change",
    ),
    (
        "room.p08_exchange",
        "price board",
        "The figures are Kasei's and change when Kasei says so.",
        "The figures are Kasei's and change when Kasei says so; +market/list has them to the credit.",
    ),
];

/// Put Stickney's three struck clauses back into `doc` (the v4 document).
fn restore_stickney_clauses(doc: &mut Value) {
    for (room, key, v4, v3) in STICKNEY_V3_CLAUSES {
        let text = doc["entries"][room]["def"]["details"][key]
            .as_str()
            .unwrap_or_else(|| panic!("{room} `{key}`"))
            .to_owned();
        assert_eq!(text.matches(v4).count(), 1, "{room} `{key}`: {text}");
        doc["entries"][room]["def"]["details"][key] = serde_json::json!(text.replace(v4, v3));
    }
}

/// The bytes of Stickney v3 (doc 18 cut IC12), rebuilt from the shipped v4: the
/// three clauses restored and the version put back, re-signed. Checked against
/// v3's pinned digest, so v4 differs from v3 by exactly the three clauses.
fn stickney_v3_json() -> String {
    let mut doc = pack_json(STICKNEY_PACK);
    doc["manifest"]["digest"] = serde_json::json!(UNSIGNED);
    restore_stickney_clauses(&mut doc);
    doc["manifest"]["version"] = serde_json::json!(3);
    sign(&serde_json::to_string_pretty(&doc).expect("serializes"))
}

/// **Doc 18 cut IC14 row 50 (lead ruling L26): Stickney v4 strikes three clauses
/// and nothing else.** The v4 document with the three clauses restored and the
/// version put back is v3's content exactly (its digest is v3's pin), and a world
/// latched at v3 takes v4 through `@atlas/pack plan` then `apply`, making no
/// object, after which the three boards read without a verb.
#[test]
fn ic14_stickney_v4_strikes_three_clauses_and_nothing_else() {
    let v3 = stickney_v3_json();
    let parsed = parse_pack(&v3).expect("the rebuild parses");
    assert_eq!(parsed.manifest().version(), 3);
    assert_eq!(
        parsed.computed_digest().expect("digests").to_hex(),
        STICKNEY_V3_DIGEST,
        "v4 is v3 less the three clauses"
    );
    let mut old = mush_atlas::starter::packs().expect("the starter packs");
    old.extend(boot_packs().expect("the game packs"));
    for (name, json) in STATION_SOURCES {
        let bytes = if name == STICKNEY_PACK {
            v3.clone()
        } else {
            json.to_owned()
        };
        old.push(mush_content::load_pack(&bytes).expect("the station pack verifies"));
    }
    let mut tw = TestWorld::with_modules(support::roster_booting(config(), old));
    let version = |tw: &TestWorld| {
        content(tw)
            .latches
            .applied(&id(STICKNEY_PACK))
            .map(mush_content::Applied::version)
    };
    assert_eq!(version(&tw), Some(3));
    let store = fold_module_ops(tw.take_module_ops());
    let mut tw = tw
        .reboot_into(production_roster_with_starter(config()), store)
        .expect("a kept world boots the new binary");
    assert_eq!(version(&tw), Some(3), "the boot pass never upgrades");
    let objects = live_objects(&tw);
    let (plan, apply) = plan_and_apply(&mut tw, STICKNEY_PACK);
    assert!(
        apply.contains(&format!("Applied {STICKNEY_PACK}@4: ")),
        "{plan}\n{apply}"
    );
    assert!(!apply.contains("STOPPED"), "{apply}");
    assert_eq!(version(&tw), Some(4));
    assert_eq!(live_objects(&tw), objects, "an update makes no object");
    let reader = tw.spawn_player("Reader");
    for (room, key, v4, _) in STICKNEY_V3_CLAUSES {
        let here = obj(&tw, STICKNEY_PACK, room);
        tw.world_mut().move_to(reader, here).expect("placed");
        let events = tw.run(reader, &format!("look {key}"));
        let out = said(&tw, &events, reader);
        assert!(out.contains(v4.trim()), "`look {key}` in {room}: {out}");
        assert!(!names_a_verb(&out), "`look {key}` in {room}: {out}");
    }
}

/// Whether `text` names a verb: a `+` directly followed by an ASCII letter, at any
/// position (`(+market/sell)` and `"+ship"` as well as `+atlas`).
fn names_a_verb(text: &str) -> bool {
    text.as_bytes()
        .windows(2)
        .any(|w| w[0] == b'+' && w[1].is_ascii_alphabetic())
}

/// Ceres v1's concourse map detail, struck in v2 and restored by
/// [`old_station_json`].
const CERES_V1_MAP: &str = "The Public Deck and the Freight Deck in cutaway, with a pulsing dot \
                            at the pedestal. It is also what +atlas shows, and the map says so \
                            in small letters: ATLAS SERVICE AVAILABLE.";

/// The bytes of a station pack's previous published version, rebuilt from the
/// shipped one: Ceres v1 (cut IC1) and Stickney v2 (cut IC8) — the five details
/// removed, Ceres' four struck clauses and Stickney's three (doc 18 cut IC14)
/// restored, the version put back, re-signed.
/// The rebuild is checked against the old pinned digests, so it is exactly what a
/// kept world latched.
fn old_station_json(pack: &str) -> String {
    let mut doc = pack_json(pack);
    doc["manifest"]["digest"] = serde_json::json!(UNSIGNED);
    for (owner, room, key, _) in IC12_DETAILS {
        if owner == pack {
            doc["entries"][room]["def"]["details"]
                .as_object_mut()
                .expect("details")
                .remove(key);
        }
    }
    if pack == CERES_PACK {
        doc["manifest"]["version"] = serde_json::json!(1);
        let board = doc["entries"]["room.c07_exchange"]["def"]["details"]["price board"]
            .as_str()
            .expect("the price board")
            .to_owned();
        doc["entries"]["room.c07_exchange"]["def"]["details"]["price board"] =
            serde_json::json!(format!("{board} This is what +market/list prints."));
        let schedules = doc["entries"]["room.c08_chandlery"]["def"]["details"]["loading schedules"]
            .as_str()
            .expect("the schedules")
            .trim_end_matches('.')
            .to_owned();
        doc["entries"]["room.c08_chandlery"]["def"]["details"]["loading schedules"] = serde_json::json!(
            format!("{schedules}; +ship/plan is the version that counts.")
        );
        doc["entries"]["room.c04_concourse"]["def"]["details"]["map"] =
            serde_json::json!(CERES_V1_MAP);
        let chit = doc["entries"]["room.c08_chandlery"]["def"]["details"]["chit board"]
            .as_str()
            .expect("the chit board")
            .replace("sells them here.", "sells them here (+market/sell).");
        doc["entries"]["room.c08_chandlery"]["def"]["details"]["chit board"] =
            serde_json::json!(chit);
    } else {
        restore_stickney_clauses(&mut doc);
        doc["manifest"]["version"] = serde_json::json!(2);
    }
    sign(&serde_json::to_string_pretty(&doc).expect("serializes"))
}

/// `@atlas/pack plan <pack>` then `@atlas/pack apply <fingerprint>`, as GOD.
fn plan_and_apply(tw: &mut TestWorld, pack: &str) -> (String, String) {
    let events = tw.run(GOD, &format!("@atlas/pack plan {pack}"));
    let plan = said(tw, &events, GOD);
    let fingerprint = plan
        .split("@atlas/pack apply ")
        .nth(1)
        .map(|rest| rest.split_whitespace().next().unwrap_or("").to_owned())
        .unwrap_or_default();
    let events = tw.run(GOD, &format!("@atlas/pack apply {fingerprint}"));
    let apply = said(tw, &events, GOD);
    (plan, apply)
}

/// **A world latched at the old station versions takes the new ones through plan
/// and apply** (doc 18 cut IC12 item 4.3; doc 18 §4.7 "the boot pass never upgrades
/// a latched pack"). Ceres v1 and Stickney v2 are rebuilt from the shipped bytes to
/// their pinned digests; a world boots them; a new binary booted over the kept store
/// leaves both latches where they were; `@atlas/pack plan` then `apply` takes Ceres
/// to 2 and Stickney to its shipped version (4 since doc 18 cut IC14), after which
/// the five details read in their rooms and no object was made.
#[test]
fn ic12_a_world_latched_at_the_old_station_versions_takes_the_new_through_plan_and_apply() {
    let ceres_v1 = old_station_json(CERES_PACK);
    let stickney_v2 = old_station_json(STICKNEY_PACK);
    for (json, version, pinned) in [
        (
            &ceres_v1,
            1,
            "383e0b135c1723100853df6b2cbe9bd3ee98eb6d752c0d00a825a365301dc1cf",
        ),
        (
            &stickney_v2,
            2,
            "39aab74e2f7a56e2b71ee7cffcdd34168ad706fbc1317240d989d2dc95c42d42",
        ),
    ] {
        let parsed = parse_pack(json).expect("the rebuild parses");
        assert_eq!(parsed.manifest().version(), version);
        assert_eq!(
            parsed.computed_digest().expect("digests").to_hex(),
            pinned,
            "the rebuild is the published version's content"
        );
    }
    // First boot: the starter, the game's packs and the OLD stations.
    let mut old = mush_atlas::starter::packs().expect("the starter packs");
    old.extend(boot_packs().expect("the game packs"));
    for (name, json) in STATION_SOURCES {
        let bytes = match name {
            CERES_PACK => ceres_v1.clone(),
            STICKNEY_PACK => stickney_v2.clone(),
            _ => json.to_owned(),
        };
        old.push(mush_content::load_pack(&bytes).expect("the station pack verifies"));
    }
    let mut tw = TestWorld::with_modules(support::roster_booting(config(), old));
    let version = |tw: &TestWorld, pack: &str| {
        content(tw)
            .latches
            .applied(&id(pack))
            .map(mush_content::Applied::version)
    };
    assert_eq!(version(&tw, CERES_PACK), Some(1));
    assert_eq!(version(&tw, STICKNEY_PACK), Some(2));
    let store = fold_module_ops(tw.take_module_ops());
    // Second boot: the new binary over the kept store never upgrades a latch.
    let mut tw = tw
        .reboot_into(production_roster_with_starter(config()), store)
        .expect("a kept world boots the new binary");
    assert_eq!(
        version(&tw, CERES_PACK),
        Some(1),
        "the boot pass never upgrades"
    );
    assert_eq!(
        version(&tw, STICKNEY_PACK),
        Some(2),
        "the boot pass never upgrades"
    );
    let objects = live_objects(&tw);
    // The operator's two acts per pack.
    for (pack, to) in [
        (CERES_PACK, station_version(CERES_PACK)),
        (STICKNEY_PACK, station_version(STICKNEY_PACK)),
    ] {
        let (plan, apply) = plan_and_apply(&mut tw, pack);
        assert!(
            apply.contains(&format!("Applied {pack}@{to}: ")),
            "{pack}:\n{plan}\n{apply}"
        );
        assert!(!apply.contains("STOPPED"), "{apply}");
        assert_eq!(version(&tw, pack), Some(to), "{pack}");
    }
    assert_eq!(live_objects(&tw), objects, "an update makes no object");
    let reader = tw.spawn_player("Reader");
    for (pack, room, key, opening) in IC12_DETAILS {
        let here = obj(&tw, pack, room);
        tw.world_mut().move_to(reader, here).expect("placed");
        let events = tw.run(reader, &format!("look {key}"));
        let out = said(&tw, &events, reader);
        assert!(out.contains(opening), "`look {key}` in {room}: {out}");
    }
}

/// What `who` was told, line by line, blank lines dropped.
fn told(tw: &TestWorld, events: &[(Audience, OutputEvent)], who: ObjId) -> Vec<String> {
    tw.seen_by(events, who)
        .into_iter()
        .filter(|line| !line.trim().is_empty())
        .collect()
}

/// **`+ic` from something that is not a character** answers exactly
/// [`ic_return::NOT_A_CHARACTER`] and moves nothing (doc 18 cut IC12 item 1.9; IC1's
/// untested arm, carry-forward line 5). The tree has no puppet `@force` (the verb of
/// that name is the campaign's order of battle), so the Thing types the line the way
/// any non-player actor's command reaches dispatch: the engine run as that actor.
#[test]
fn ic12_ic_refuses_a_non_character() {
    let mut tw = booted();
    let thing = tw
        .world_mut()
        .create(ObjKind::Thing, "Crate", GOD, Some(LIMBO))
        .expect("a thing");
    let events = tw.run(thing, "+ic");
    assert_eq!(
        told(&tw, &events, thing),
        vec![ic_return::NOT_A_CHARACTER.to_owned()]
    );
    assert_eq!(location(&tw, thing), Some(LIMBO), "nothing moved");
}

/// **`+ic` on a world without a corridor** answers exactly
/// [`ic_return::NO_CORRIDOR`] (doc 18 cut IC12 item 1.9): the layer is enabled but
/// no station pack is applied — the boot list is the starter and the game's boot
/// packs only (`support::roster_booting`, test 8's first boot) — so Ceres' arrival
/// room is on no latch.
#[test]
fn ic12_ic_refuses_a_world_without_a_corridor() {
    let mut old = mush_atlas::starter::packs().expect("the starter packs");
    old.extend(boot_packs().expect("the game packs"));
    let mut tw = TestWorld::with_modules(support::roster_booting(config(), old));
    assert!(
        content(&tw).latches.applied(&id(CERES_PACK)).is_none(),
        "no station on this world"
    );
    let pilot = tw.spawn_player("Pilot");
    let events = tw.run(pilot, "+ic");
    assert_eq!(
        told(&tw, &events, pilot),
        vec![ic_return::NO_CORRIDOR.to_owned()]
    );
    assert_eq!(location(&tw, pilot), Some(LIMBO), "nothing moved");
}

/// **The IC door has no staff exemption** (doc 18 §4.5, D-18-3; `ic_return.rs`'s
/// "No staff exemption"; cut IC12 item 1.9): a WIZARD who stepped out at Stickney
/// is refused at the door with `left_at_line`'s Stickney text, exactly as a player.
#[test]
fn ic12_the_ic_door_has_no_staff_exemption() {
    let mut tw = booted();
    let p04 = obj(&tw, STICKNEY_PACK, "room.p04_concourse");
    let staff = tw.spawn_player("Warden");
    tw.world_mut()
        .get_mut(staff)
        .expect("the staffer")
        .flags
        .insert(mush_core::Flags::WIZARD);
    tw.world_mut().move_to(staff, p04).expect("placed");
    let _ = tw.run(staff, "o");
    assert_eq!(ic_return::last_port(tw.world(), staff), Some(&PORTS[1]));
    let _ = tw.run(staff, "x");
    assert_eq!(location(&tw, staff), Some(LIMBO));
    let events = tw.run(staff, "c");
    let out = said(&tw, &events, staff);
    assert!(out.contains(&ic_return::left_at_line(&PORTS[1])), "{out}");
    assert_eq!(location(&tw, staff), Some(LIMBO), "the door held");
}

/// **`+ic` inside the corridor answers `ALREADY_IN`** (doc 18 cut IC14 row 37; doc 18
/// §4.5): a character standing in Ceres' concourse types `+ic` and hears exactly
/// [`ic_return::ALREADY_IN`], and nothing moves.
#[test]
fn ic14_ic_inside_the_corridor_answers_already_in() {
    let mut tw = booted();
    let c04 = obj(&tw, CERES_PACK, "room.c04_concourse");
    let pilot = tw.spawn_player("Pilot");
    tw.world_mut().move_to(pilot, c04).expect("placed");
    let events = tw.run(pilot, "+ic");
    assert_eq!(
        told(&tw, &events, pilot),
        vec![ic_return::ALREADY_IN.to_owned()]
    );
    assert_eq!(location(&tw, pilot), Some(c04), "not moved");
}

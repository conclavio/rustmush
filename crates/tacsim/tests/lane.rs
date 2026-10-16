//! **Doc 18 cut IC2 acceptance — the lane and the sky seam** (doc 18 §4.4, §4.8;
//! D-18-43, D-18-45, D-18-48; lead rulings R1–R5 of the IC2 brief).
//!
//! `tacsim.lane.ceres_phobos@2` places Ceres, Mars and Pallas at their true
//! heliocentric longitudes and distances on one real date (2351-05-13 on the
//! game calendar, fetched by `content/sky/fetch_horizons.py`) and Kirkwood on
//! the Ceres–Mars chord at 2.06 AU; its two routes' `base_s` are the legs that
//! sky derives at the Dray's 226 km/s (doc 18 cut IC3's re-fitted cruise, lead
//! ruling L0a). This file pins the pack, derives its legs
//! back from the sky (retired with lane v1 at P-A), checks the circular sky's
//! two readings against each other, holds a body at its own epoch, proves the
//! optional sky field moves no fixture, holds one drive class to one law, keeps
//! an operator file off the lane's id, and boots the lane on the production
//! composition and reads it through `@corridor/show`.
//!
//! **The version rule (doc 18 cut IC3, lead ruling L0b).** A published version
//! is never mutated (the store's rule, `boot_reconcile`): IC2's v1 was published
//! to every enabled world that booted it, so IC3 publishes v2 (the routes
//! re-timed for 226 km/s, and the two `port` entries) and IC8 will publish v3
//! (the vendors). A world that latched v1 keeps v1's ledger row and takes v2 by
//! `@atlas/pack plan tacsim.lane.ceres_phobos` / `@atlas/pack apply` — the boot
//! pass applies only never-latched packs. Lane symbols are addressed by
//! (`LANE_PACK`, version), never by bare id. Only v2 is a game source.
//!
//! No test here asserts a passage duration (D-18-50): the legs are the sky's
//! output, never an invariant.

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mush_content::ContentState;
use mush_core::{CanonicalId, GOD, ObjId, ObjKind};
use mush_testkit::{TestWorld, fold_module_ops};
use tacsim::config::{LoadedPack, PackSource};
use tacsim::definitions::content::{
    DRAY_JSON, LANE_JSON, LANE_PACK, LANE_ROUTE_BACK, LANE_ROUTE_OUT, LANE_VERSION,
    game_seed_sources,
};
use tacsim::definitions::{Catalog, DriveLaw};
use tacsim::pack::{PackEntry, RouteEntry, RulesPack, load_pack};
use tacsim::{
    AU_KM, BodyDef, Ephemeris, EphemerisConfig, Sky, TacsimConfig, TacsimStore, epoch_date,
    fixture, verify_sources, verify_sources_with,
};

use support::ground::{UNSIGNED, sign};

/// **The lane's pin**, v2 (doc 18 cut IC3): a new version beside v1's ledger
/// row, never v1 re-signed in place (lead ruling L0b). IC8's v3 is pinned anew.
const LANE_DIGEST: &str = "3f7e1a962eedf4bf4fcdd06c83ba893c555c11579dd1b35a20901bef596354e0";

/// The date the lane's bodies stand at (the script's chord minimum, R4).
const LANE_DATE: &str = "2351-05-13";

/// The three fixture pins of E7 cut 1 and E7r2 (`e7r1.rs`, `e7r2.rs`).
const CORE_DIGEST: &str = "db2702631983de33bf61e3018b7cede2add8c5aa127ae49ca26e0256bd1a6049";
const CORRIDOR_DIGEST: &str = "6ad21275d8ba59a6b742cf354e2efafe840cacaa34f8e6c84ac057b9997f8d69";
const CORRIDOR_V2_DIGEST: &str = "f89b7310b1d30e7b38a5a6cf9fe3e22330065c797dcf750e9cb8dc8554177041";

fn content_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("content")
}

fn writing() -> bool {
    std::env::var("TACSIM_WRITE_CONTENT").is_ok_and(|v| v == "1")
}

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

fn lane() -> RulesPack {
    load_pack(LANE_JSON).expect("the lane verifies")
}

fn route_of<'a>(pack: &'a RulesPack, id: &str) -> &'a RouteEntry {
    match pack.entry(&cid(id)) {
        Some(PackEntry::Route(route)) => route,
        other => panic!("{id} is a route: {other:?}"),
    }
}

/// The body a symbolic place stands on, or `None`.
fn body_of(pack: &RulesPack, place: &CanonicalId) -> Option<CanonicalId> {
    match pack.entry(place) {
        Some(PackEntry::Location(def)) => def.body.clone(),
        _ => None,
    }
}

/// The lane's bodies, in id order, with their authored numbers.
fn bodies(pack: &RulesPack) -> Vec<(CanonicalId, BodyDef)> {
    pack.entries()
        .iter()
        .filter_map(|(id, entry)| match entry {
            PackEntry::Body(def) => Some((id.clone(), *def)),
            _ => None,
        })
        .collect()
}

/// Kilometres as AU to three decimals, as `@corridor/show` prints them.
fn au_text(km: u64) -> String {
    let milli = (u128::from(km) * 1_000 + u128::from(AU_KM) / 2) / u128::from(AU_KM);
    format!("{}.{:03}", milli / 1_000, milli % 1_000)
}

/// **The legs a route's sky derives**: the route walked as body pairs (the
/// origin's body, then each leg's destination's), the distance between each pair
/// at the origin body's epoch through [`Sky::distance_km`], and
/// `ceil(km / kms)`. Refuses, naming the place, when a place has no body.
/// Retired with lane v1 at P-A, when the passage planner replaces it.
fn derived_legs(pack: &RulesPack, route: &RouteEntry, kms: u32) -> Result<Vec<u64>, String> {
    let sky = Sky::from_pack(pack, &EphemerisConfig::default());
    let body =
        |place: &CanonicalId| body_of(pack, place).ok_or_else(|| format!("{place} has no body"));
    let mut from = body(&route.from)?;
    let mut out = Vec::with_capacity(route.legs.len());
    for leg in &route.legs {
        let to = body(&leg.to)?;
        let t = sky
            .epoch_of(&from)
            .ok_or_else(|| format!("{from} is not in the sky"))?;
        let km = sky
            .distance_km(&from, t, &to, t)
            .ok_or_else(|| format!("{from} to {to} has no distance"))?;
        out.push(km.div_ceil(u64::from(kms)));
        from = to;
    }
    Ok(out)
}

/// The coast rate the fielded catalog's drive table gives a route's class.
fn coast_kms(class: &CanonicalId) -> u32 {
    let catalog = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    match catalog.drive_for_class(class) {
        Some(DriveLaw::Coast { kms, .. }) => *kms,
        None => panic!("no drive flies {class}"),
    }
}

// ---------------------------------------------------------------------------
// The pin
// ---------------------------------------------------------------------------

/// **The lane is pinned** (doc 18 cut IC2; R4; cut IC3, L0a/L0b): a campaign
/// seed at version 2 requiring nothing, exactly the twelve entries — the `sol`
/// region, four bodies, three places, two routes re-timed for 226 km/s and the
/// two ports; every body carries the same `epoch_ic`, which is 2351-05-13 on the
/// game calendar.
#[test]
fn ic2_the_lane_pack_is_pinned() {
    let relative = "lane/ceres_phobos.json";
    if writing() {
        let path = content_dir().join(relative);
        let raw = std::fs::read_to_string(&path).expect("the content file exists");
        let declared = mush_content::parse_pack(&raw)
            .expect("the pack parses")
            .manifest()
            .digest()
            .to_hex();
        let signed = sign(&raw.replace(&declared, UNSIGNED));
        std::fs::write(&path, &signed).expect("the content file is writable");
        let digest = mush_content::parse_pack(&signed)
            .expect("the signed pack parses")
            .manifest()
            .digest()
            .to_hex();
        println!("    const LANE_DIGEST: &str = \"{digest}\";");
        return;
    }
    let pack = lane();
    let manifest = pack.manifest();
    assert_eq!(manifest.id().as_str(), LANE_PACK);
    assert_eq!(manifest.version(), LANE_VERSION);
    assert_eq!(manifest.version(), 2);
    assert_eq!(manifest.kind(), tacsim::PackKind::CampaignSeed);
    assert!(manifest.requires().is_empty(), "the lane requires nothing");
    assert_eq!(
        manifest.digest().to_hex(),
        LANE_DIGEST,
        "the lane's digest moved; re-review the content and re-pin it"
    );
    let raw = mush_content::parse_pack(LANE_JSON).expect("parses");
    let mut kinds: Vec<(String, String)> = raw
        .entries()
        .iter()
        .map(|(id, entry)| (id.to_string(), entry.kind().to_string()))
        .collect();
    kinds.sort();
    assert_eq!(
        kinds,
        vec![
            ("ceres".to_owned(), "body".to_owned()),
            ("ceres_freeport".to_owned(), "location".to_owned()),
            ("ceres_port".to_owned(), "port".to_owned()),
            ("ceres_stickney".to_owned(), "route".to_owned()),
            ("kirkwood".to_owned(), "body".to_owned()),
            ("kirkwood_depot".to_owned(), "location".to_owned()),
            ("mars".to_owned(), "body".to_owned()),
            ("pallas".to_owned(), "body".to_owned()),
            ("sol".to_owned(), "region".to_owned()),
            ("stickney_ceres".to_owned(), "route".to_owned()),
            ("stickney_port".to_owned(), "port".to_owned()),
            ("stickney_yards".to_owned(), "location".to_owned()),
        ],
        "the twelve entries: v1's ten and the two ports"
    );
    let epochs: Vec<Option<i64>> = bodies(&pack).iter().map(|(_, def)| def.epoch_ic).collect();
    assert_eq!(epochs.len(), 4);
    let first = epochs
        .first()
        .copied()
        .flatten()
        .expect("every body carries epoch_ic");
    assert!(epochs.iter().all(|e| *e == Some(first)), "{epochs:?}");
    assert_eq!(epoch_date(first), LANE_DATE);
    let places: Vec<(String, String, String, Option<String>)> = pack
        .entries()
        .iter()
        .filter_map(|(id, entry)| match entry {
            PackEntry::Location(def) => Some((
                id.to_string(),
                def.label.clone(),
                def.role.to_string(),
                def.body.as_ref().map(ToString::to_string),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        places,
        vec![
            (
                "ceres_freeport".into(),
                "Ceres Freeport".into(),
                "port".into(),
                Some("ceres".into())
            ),
            (
                "kirkwood_depot".into(),
                "Kirkwood Depot".into(),
                "depot".into(),
                Some("kirkwood".into())
            ),
            (
                "stickney_yards".into(),
                "Stickney Yards".into(),
                "port".into(),
                Some("mars".into())
            ),
        ]
    );
    // The four leg seconds, `ceil(km / 226)` of IC2's chords at the epoch
    // (doc 18 cut IC3, L0a).
    let seconds = |id: &str| -> Vec<(String, u32)> {
        route_of(&pack, id)
            .legs
            .iter()
            .map(|leg| (leg.to.to_string(), leg.base_s))
            .collect()
    };
    assert_eq!(
        seconds(LANE_ROUTE_OUT),
        vec![
            ("kirkwood_depot".to_owned(), 537_293),
            ("stickney_yards".to_owned(), 411_975)
        ]
    );
    assert_eq!(
        seconds(LANE_ROUTE_BACK),
        vec![
            ("kirkwood_depot".to_owned(), 411_975),
            ("ceres_freeport".to_owned(), 537_293)
        ]
    );
    for id in [LANE_ROUTE_OUT, LANE_ROUTE_BACK] {
        let route = route_of(&pack, id);
        assert_eq!(route.drive_class.as_str(), "coast_hauler", "{id}");
        assert_eq!(route.propellant, 0, "{id}");
        assert!(route.controller.is_none(), "{id}");
        assert!(
            route
                .legs
                .iter()
                .all(|leg| leg.hazard == 0 && leg.interdiction == 0),
            "{id}"
        );
    }
}

// ---------------------------------------------------------------------------
// The sky
// ---------------------------------------------------------------------------

/// **Every authored leg is the leg the sky derives** (doc 18 §4.8, D-18-42):
/// both lane routes walked body to body at the origin's epoch, `ceil(km / kms)`
/// with `kms` from the fielded drive table, within 100 s of the authored
/// `base_s`; and a place without a body is refused by name. Retired with lane v1
/// at P-A.
#[test]
fn lane_legs_derive_at_epoch() {
    let pack = lane();
    for id in [LANE_ROUTE_OUT, LANE_ROUTE_BACK] {
        let route = route_of(&pack, id);
        let kms = coast_kms(&route.drive_class);
        let derived = derived_legs(&pack, route, kms).unwrap_or_else(|why| panic!("{id}: {why}"));
        assert_eq!(derived.len(), route.legs.len());
        for (leg, want) in route.legs.iter().zip(&derived) {
            assert!(
                u64::from(leg.base_s).abs_diff(*want) <= 100,
                "{id} -> {}: authored {}s, derived {want}s",
                leg.to,
                leg.base_s
            );
        }
    }
    // The same pack in memory with Kirkwood Depot unanchored: the helper names it.
    let mut entries: BTreeMap<CanonicalId, PackEntry> = pack.entries().clone();
    if let Some(PackEntry::Location(def)) = entries.get_mut(&cid("kirkwood_depot")) {
        def.body = None;
    }
    let bare = RulesPack::new(pack.manifest().clone(), entries).expect("the pack builds");
    let route = route_of(&bare, LANE_ROUTE_OUT);
    let refused = derived_legs(&bare, route, 226).expect_err("a bodiless place is refused");
    assert!(refused.contains("kirkwood_depot"), "{refused}");
}

/// **The circular sky's two readings agree** (D-18-43): under rate 1000 and
/// rate 0, a year either side of the lane's epoch and at it, the distance
/// between every pair of the four bodies is the Euclidean distance between their
/// positions to within 2 km, every `z` is 0, and so is one mixed-instant pair.
#[test]
fn sky_circular_agrees_with_the_law_of_cosines() {
    let pack = lane();
    let ids: Vec<CanonicalId> = bodies(&pack).into_iter().map(|(id, _)| id).collect();
    let year = 365 * 86_400;
    for rate in [1_000, 0] {
        let sky = Sky::from_pack(&pack, &EphemerisConfig { epoch: 0, rate });
        let e = sky.epoch_of(&cid("ceres")).expect("ceres is in the sky");
        let euclid = |a: &CanonicalId, ta: i64, b: &CanonicalId, tb: i64| -> f64 {
            let (pa, pb) = (
                sky.position_km(a, ta).expect("a position"),
                sky.position_km(b, tb).expect("a position"),
            );
            assert_eq!((pa.z, pb.z), (0, 0), "the circular sky is coplanar");
            #[allow(clippy::cast_precision_loss)]
            let (dx, dy) = ((pa.x - pb.x) as f64, (pa.y - pb.y) as f64);
            (dx * dx + dy * dy).sqrt()
        };
        for t in [e - year, e, e + year] {
            for (i, a) in ids.iter().enumerate() {
                for b in ids.iter().skip(i + 1) {
                    let km = sky.distance_km(a, t, b, t).expect("a distance");
                    #[allow(clippy::cast_precision_loss)]
                    let gap = (km as f64 - euclid(a, t, b, t)).abs();
                    assert!(gap <= 2.0, "rate {rate}, t {t}, {a}-{b}: {gap} km apart");
                }
            }
        }
        let (a, b) = (cid("ceres"), cid("mars"));
        let km = sky.distance_km(&a, e, &b, e + year).expect("a distance");
        #[allow(clippy::cast_precision_loss)]
        let gap = (km as f64 - euclid(&a, e, &b, e + year)).abs();
        assert!(gap <= 2.0, "rate {rate}, the mixed pair: {gap} km apart");
    }
}

/// **A body holds its phase at its own epoch** (D-18-48): under rate 1000 with
/// the config epoch at 0, every lane body reads its authored phase at its
/// `epoch_ic` and again (within 1 mdeg) one period later; a fixture body
/// without `epoch_ic` (`tacsim.corridor@1`'s Mars) still advances from the
/// config epoch.
#[test]
fn body_epoch_holds_its_phase_at_that_instant() {
    let cfg = EphemerisConfig {
        epoch: 0,
        rate: 1_000,
    };
    let pack = lane();
    let sky = Ephemeris::from_pack(&pack, &cfg);
    let turn = 360_000_i64;
    for (id, def) in bodies(&pack) {
        let epoch = def.epoch_ic.expect("a lane body carries epoch_ic");
        assert_eq!(
            sky.phase_mdeg_at(&id, epoch),
            Some(def.phase_mdeg),
            "{id} at its own epoch"
        );
        let period_s = i64::try_from(def.period_mdays * 86_400 / 1_000).expect("in range");
        let later = sky.phase_mdeg_at(&id, epoch + period_s).expect("a phase");
        let off = (later - def.phase_mdeg).rem_euclid(turn);
        assert!(
            off.min(turn - off) <= 1,
            "{id} one period on: {later} against {}",
            def.phase_mdeg
        );
    }
    let corridor = load_pack(fixture::CORRIDOR_JSON).expect("the corridor verifies");
    let fixture_sky = Ephemeris::from_pack(&corridor, &cfg);
    let mars = cid("mars");
    assert_eq!(
        fixture_sky.body(&mars).map(|b| b.epoch_ic),
        Some(None),
        "a fixture body has no epoch of its own"
    );
    assert_eq!(fixture_sky.phase_mdeg_at(&mars, 0), Some(130_000));
    assert_ne!(
        fixture_sky.phase_mdeg_at(&mars, 100 * 86_400),
        Some(130_000),
        "it advances from the config epoch"
    );
}

/// **The optional sky field moves no fixture** (doc 18 cut IC2; reviewer F6).
///
/// The re-encode property was checked on the base tree first (07232d09): the
/// two corridor fixtures rebuild from their typed entries to their authored
/// digest, and `tacsim.core@1` does **not** — its `formation` entries re-encode
/// `allowed` and `required_support`, which are serialized defaults unrelated to
/// bodies — so this is the brief's branch B: every fixture `body` entry,
/// decoded to `BodyDef` and re-serialized, is its authored JSON object exactly
/// (no `epoch_ic` key), the three declared digests are the e7r1/e7r2 pins, and
/// the corridors' rebuild — the packs that carry bodies — still holds.
#[test]
fn fixture_digests_unmoved_by_optional_sky_fields() {
    use tacsim::pack::RulesPackWire;

    for (name, json, pinned, rebuilds) in [
        ("tacsim.core@1", fixture::CORE_JSON, CORE_DIGEST, false),
        (
            "tacsim.corridor@1",
            fixture::CORRIDOR_JSON,
            CORRIDOR_DIGEST,
            true,
        ),
        (
            "tacsim.corridor@2",
            fixture::CORRIDOR_V2_JSON,
            CORRIDOR_V2_DIGEST,
            true,
        ),
    ] {
        let pack = load_pack(json).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(pack.manifest().digest().to_hex(), pinned, "{name}'s pin");
        assert_eq!(
            pack.computed_digest().expect("digests").to_hex(),
            pinned,
            "{name}: the declared digest is the computed one"
        );
        let raw: serde_json::Value = serde_json::from_str(json).expect("JSON");
        let entries = raw
            .get("entries")
            .and_then(serde_json::Value::as_object)
            .expect("entries");
        for (id, entry) in entries {
            if entry.get("kind").and_then(serde_json::Value::as_str) != Some("body") {
                continue;
            }
            let authored = entry.get("def").expect("a def").clone();
            let def: BodyDef = serde_json::from_value(authored.clone()).expect("a body");
            assert_eq!(def.epoch_ic, None, "{name}.{id}");
            let again = serde_json::to_value(def).expect("encodes");
            assert_eq!(again, authored, "{name}.{id} re-encodes as authored");
            assert!(again.get("epoch_ic").is_none());
        }
        if rebuilds {
            let wire = RulesPackWire::from(pack.clone());
            let rebuilt = RulesPack::with_foreign(
                pack.manifest().clone(),
                pack.entries().clone(),
                &wire.entries.foreign,
            )
            .expect("rebuilds");
            assert_eq!(
                rebuilt.computed_digest().expect("digests").to_hex(),
                pinned,
                "{name} rebuilds from its typed entries to its authored digest"
            );
        }
    }
    let with = BodyDef::from_authored(2.0, 1_000.0, 10.0, Some(74_152_540_800))
        .expect("a body with an epoch");
    let encoded = serde_json::to_value(with).expect("encodes");
    assert_eq!(
        encoded.get("epoch_ic"),
        Some(&serde_json::json!(74_152_540_800_i64))
    );
    let without = BodyDef::from_authored(2.0, 1_000.0, 10.0, None).expect("a body");
    assert!(
        serde_json::to_value(without)
            .expect("encodes")
            .get("epoch_ic")
            .is_none()
    );
    assert!(
        BodyDef::from_authored(2.0, 1_000.0, 10.0, Some(1_000_000_000_001)).is_err(),
        "an epoch past the bound is refused"
    );
}

// ---------------------------------------------------------------------------
// The drive table and the game-packs list
// ---------------------------------------------------------------------------

/// The Dray's JSON as another operator pack: its id, its entry and its coast
/// rate changed, re-signed so the digest is not what refuses it.
fn dray_copy(kms: u32) -> LoadedPack {
    let mut doc: serde_json::Value = serde_json::from_str(DRAY_JSON).expect("the Dray is JSON");
    let manifest = doc.get_mut("manifest").expect("a manifest");
    manifest["id"] = serde_json::json!("test.pack.dray_b");
    manifest["digest"] = serde_json::json!(UNSIGNED);
    let entries = doc
        .get_mut("entries")
        .and_then(serde_json::Value::as_object_mut)
        .expect("entries");
    let mut def = entries
        .remove("tacsim.unit.dray")
        .expect("the Dray's entry");
    def["def"]["logistics"]["drive"]["law"]["coast"]["kms"] = serde_json::json!(kms);
    entries.insert("test.unit.dray_b".to_owned(), def);
    let signed = sign(&serde_json::to_string_pretty(&doc).expect("serializes"));
    LoadedPack {
        pack: load_pack(&signed).expect("the copy verifies"),
        source: "<test dray_b>".to_owned(),
    }
}

/// **One drive class, one law** (Q-18-5, D-18-45): the fielded catalog answers
/// the Dray's coast law for `coast_hauler` and nothing for a class no hull
/// declares; a second hull flying `coast_hauler` at another speed is refused
/// naming the class and both units; at the same law it is accepted and the
/// table is unchanged.
#[test]
fn one_drive_class_one_law() {
    let fielded = Catalog::build_fielded(&[], tacsim::NS).expect("the fielded content binds");
    assert_eq!(
        fielded.drive_for_class(&cid("coast_hauler")),
        Some(&DriveLaw::Coast {
            kms: 226,
            burn_permille: 105
        })
    );
    assert_eq!(fielded.drive_for_class(&cid("fusion_torch")), None);

    let refused = Catalog::build_fielded(&[dray_copy(300)], tacsim::NS)
        .err()
        .map(|error| error.to_string())
        .expect("a second law for one class is refused");
    for word in ["coast_hauler", "tacsim.unit.dray", "test.unit.dray_b"] {
        assert!(refused.contains(word), "{word}: {refused}");
    }
    assert!(refused.contains("one class, one law"), "{refused}");

    let same = Catalog::build_fielded(&[dray_copy(226)], tacsim::NS)
        .expect("the same law twice is one law");
    assert_eq!(same.drives, fielded.drives);
}

/// **An operator seed cannot claim the lane's id** (lead ruling R2; reviewer
/// F3): the lane's own bytes, re-signed, as an operator file `ops/lane.json`
/// load alone (no game sources, no collision), and are refused once the game's
/// seed sources are read first, by the existing collision message naming the
/// operator file as the offender and the built-in lane as the holder.
#[test]
fn ic2_an_operator_seed_cannot_claim_the_lane_id() {
    let mut doc: serde_json::Value = serde_json::from_str(LANE_JSON).expect("the lane is JSON");
    doc["manifest"]["digest"] = serde_json::json!(UNSIGNED);
    let signed = sign(&serde_json::to_string_pretty(&doc).expect("serializes"));
    let mut cfg: TacsimConfig = fixture::config();
    cfg.seeds.push(PackSource {
        name: "ops/lane.json".to_owned(),
        bytes: signed,
    });
    verify_sources(&cfg).expect("alone, the operator file collides with nothing");
    let refused = verify_sources_with(&cfg, &game_seed_sources())
        .expect_err("the operator file collides with the lane");
    assert!(
        refused.starts_with("[tacsim] ops/lane.json: declares tacsim.lane.ceres_phobos@2, which "),
        "{refused}"
    );
    assert!(
        refused.contains("<built-in tacsim/content/lane/ceres_phobos.json> already declares"),
        "{refused}"
    );
}

// ---------------------------------------------------------------------------
// The lane on the production composition
// ---------------------------------------------------------------------------

/// The production roster as `mush_server::boot` composes it with the game on —
/// `tacsim_module_wired(cfg, true, inventory)` (doc 18 cut IC5) and
/// `content_module_with(true, true)` — a copy
/// of `tests/stations.rs`'s `production_roster_with_starter` (an integration test
/// cannot import another).
fn production_roster(cfg: TacsimConfig) -> Vec<Box<dyn mush_core::GameModule>> {
    use tacsim::composition as comp;

    let llm = Arc::new(
        mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
            .expect("a disabled LLM service composes"),
    );
    let onboarding = mush_core::OnboardingConfig::default();
    let game = tacsim::GameEconomyConfig::default();
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
    mods.push(comp::tacsim_module_wired(cfg, true, inventory));
    mods.push(comp::content_module_with(true, true).expect("the content module composes"));
    mods
}

/// `fixture::config()` with both corridor versions as operator seeds, under an
/// ephemeris configuration.
fn lane_config(ephemeris: EphemerisConfig) -> TacsimConfig {
    let mut cfg = fixture::config();
    cfg.seeds = vec![
        PackSource {
            name: "<built-in tacsim.corridor>".to_owned(),
            bytes: fixture::CORRIDOR_JSON.to_owned(),
        },
        PackSource {
            name: "<built-in tacsim.corridor@2>".to_owned(),
            bytes: fixture::CORRIDOR_V2_JSON.to_owned(),
        },
    ];
    cfg.time.ephemeris = ephemeris;
    cfg
}

fn store(tw: &TestWorld) -> &TacsimStore {
    tw.world()
        .module_state::<TacsimStore>()
        .expect("tacsim state")
}

/// The chord the lane's sky derives between Ceres and Mars at the lane epoch.
fn lane_chord() -> u64 {
    let sky = Sky::from_pack(&lane(), &EphemerisConfig::default());
    let (ceres, mars) = (cid("ceres"), cid("mars"));
    let e = sky.epoch_of(&ceres).expect("ceres");
    sky.distance_km(&ceres, e, &mars, e).expect("a chord")
}

fn latches(tw: &TestWorld) -> BTreeMap<String, (u32, String)> {
    tw.world()
        .module_state::<ContentState>()
        .expect("content")
        .latches
        .all()
        .map(|l| (l.pack().to_string(), (l.version(), l.digest().to_hex())))
        .collect()
}

fn live_objects(tw: &TestWorld) -> usize {
    tw.world()
        .iter()
        .filter(|o| !o.flags.contains(mush_core::Flags::GOING))
        .count()
}

/// What `@corridor/show <route>` should print for a lane route, derived here
/// from the pack, its sky and the rooms the content latch placed (never typed
/// in): the header's two endpoint rooms and one whole line per leg — its
/// number, the room it reaches, its authored seconds and its AU at the origin
/// body's epoch — so each leg's figures are tied to that leg's own line.
fn expected_lines(id: &str, placed: &BTreeMap<CanonicalId, ObjId>) -> Vec<String> {
    let pack = lane();
    let route = route_of(&pack, id);
    let sky = Sky::from_pack(&pack, &EphemerisConfig::default());
    let room = |place: &CanonicalId| placed.get(place).expect("a placed room").0;
    let last = route.legs.last().expect("a leg");
    let mut out = vec![format!(
        "Corridor {id}: #{} -> #{}, 2 legs, drive class coast_hauler, propellant 0.",
        room(&route.from),
        room(&last.to)
    )];
    let mut from = body_of(&pack, &route.from).expect("a body");
    for (index, leg) in route.legs.iter().enumerate() {
        let to = body_of(&pack, &leg.to).expect("a body");
        let t = sky.epoch_of(&from).expect("an epoch");
        let km = sky.distance_km(&from, t, &to, t).expect("a distance");
        out.push(format!(
            "  leg {}: -> #{} ({}s unladen); {from} to {to}, {} AU at the epoch.",
            index + 1,
            room(&leg.to),
            leg.base_s,
            au_text(km)
        ));
        from = to;
    }
    out
}

/// **The lane boots and `@corridor/show` reads the constant sky** (doc 18 cut
/// IC2; §4.8, D-18-45; reviewer F4).
///
/// On the production composition with the game on and both corridor fixtures as
/// operator seeds: the content latch holds the lane at version 2 with three
/// placed rooms (doc 18 cut IC3: the ports place no object); `@corridor/show` on both lane routes, as a player, prints two
/// legs, the drive class and propellant, the chord line ending
/// `constant sky at epoch 2351-05-13 (T17).` and each leg's authored seconds and
/// derived AU, and never "frozen"; GOD's `@tacsim/pack list` lists the lane
/// beside the untouched fixtures; the lane route's pinned chord is the sky's
/// chord at the lane epoch on this world and on one booted at rate 1000 (the pin
/// is taken at the body's own epoch, never the config epoch); the corridor@1
/// route still pins exactly `pinned_km`; and a second boot over the folded store
/// creates no object and changes no latch.
#[test]
fn ic2_the_lane_boots_and_corridor_show_reads_the_constant_sky() {
    let mut tw =
        TestWorld::with_modules(production_roster(lane_config(EphemerisConfig::default())));

    // The content latch holds the lane, three places made rooms.
    let applied = tw
        .world()
        .module_state::<ContentState>()
        .and_then(|c| c.latches.applied(&cid(LANE_PACK)))
        .expect("the lane is latched");
    assert_eq!(applied.version(), LANE_VERSION);
    let placed = applied.placed();
    assert_eq!(placed.len(), 3, "{placed:?}");
    for (symbol, room) in &placed {
        let kind = tw.world().get(*room).map(|o| o.kind);
        assert_eq!(kind, Some(ObjKind::Room), "{symbol}");
    }

    // The chord pin, at the lane epoch.
    let chord = lane_chord();
    let route = store(&tw)
        .route_in_world(tw.world(), &cid(LANE_ROUTE_OUT))
        .expect("the lane route materializes");
    assert_eq!(route.distance_km(), Some(chord));
    assert_eq!(
        route.bodies(),
        Some(&(cid("ceres"), cid("mars"))),
        "the chord's body pair"
    );

    // The route's source is the lane itself (reviewer F1): `@corridor/show`
    // reads its bodies and sky from the pack that materialized the route, and
    // names no pack for a route no loaded pack materializes.
    let (source, entry) = store(&tw)
        .route_source(tw.world(), &route)
        .expect("the lane route has a source");
    assert_eq!(source.manifest().id(), &cid(LANE_PACK));
    assert_eq!(entry.from, cid("ceres_freeport"));
    let mut forged = serde_json::to_value(&route).expect("a route encodes");
    let first = forged["legs"][0]["base_s"]
        .as_u64()
        .expect("a leg's seconds");
    forged["legs"][0]["base_s"] = serde_json::json!(first + 1);
    let forged: mush_sim::RouteDef = serde_json::from_value(forged).expect("a route decodes");
    assert!(
        store(&tw).route_source(tw.world(), &forged).is_none(),
        "a route no loaded pack materialized has no source"
    );

    // `@corridor/show` as a player.
    let pilot = tw.spawn_player("Pilot");
    for id in [LANE_ROUTE_OUT, LANE_ROUTE_BACK] {
        let events = tw.run(pilot, &format!("@corridor/show {id}"));
        let out = tw.seen_by(&events, pilot).join("\n");
        assert!(
            out.contains("2 legs, drive class coast_hauler, propellant 0."),
            "{out}"
        );
        assert!(
            out.contains(&format!(
                "direct chord at the epoch: {chord} km ({} AU) between ",
                au_text(chord)
            )),
            "{out}"
        );
        assert!(
            out.contains(&format!("; constant sky at epoch {LANE_DATE} (T17).")),
            "{out}"
        );
        // Whole lines: the header starts with its line (the holder follows), and
        // every leg line is one line of the output exactly.
        let shown: Vec<&str> = out.lines().collect();
        for line in expected_lines(id, &placed) {
            if line.starts_with("Corridor ") {
                assert!(shown.iter().any(|l| l.starts_with(&line)), "{line}\n{out}");
            } else {
                assert!(shown.contains(&line.as_str()), "{line}\n{out}");
            }
        }
        assert!(!out.contains("frozen"), "{out}");
    }

    // The pack list, as staff.
    let events = tw.run(GOD, "@tacsim/pack list");
    let out = tw.seen_by(&events, GOD).join("\n");
    for (line, digest) in [
        (
            "tacsim.lane.ceres_phobos@2 [seed]",
            lane().manifest().digest().to_hex(),
        ),
        ("tacsim.core@1 [rules]", CORE_DIGEST.to_owned()),
        ("tacsim.corridor@1 [seed]", CORRIDOR_DIGEST.to_owned()),
        ("tacsim.corridor@2 [seed]", CORRIDOR_V2_DIGEST.to_owned()),
    ] {
        assert!(out.contains(line), "{line}\n{out}");
        assert!(
            out.contains(&format!("digest {}", &digest[..12])),
            "{line}\n{out}"
        );
    }

    // A second boot over the folded store makes nothing and moves no latch.
    let before_objects = live_objects(&tw);
    let before_latches = latches(&tw);
    let folded = fold_module_ops(tw.take_module_ops());
    let mut tw = tw
        .reboot_into(
            production_roster(lane_config(EphemerisConfig::default())),
            folded,
        )
        .expect("the second boot boots");
    assert_eq!(
        live_objects(&tw),
        before_objects,
        "a second boot creates no object"
    );
    assert_eq!(
        latches(&tw),
        before_latches,
        "a second boot changes no latch"
    );

    // The fixture still pins bit-identically: seed corridor@1 through the content
    // latch, and its cross-body route carries exactly the old `pinned_km`.
    let events = tw.run(GOD, "@tacsim/seed tacsim.corridor@1 commit");
    let seeded = tw.seen_by(&events, GOD).join("\n");
    let corridor = load_pack(fixture::CORRIDOR_JSON).expect("the corridor verifies");
    let fixture_route = store(&tw)
        .route_in_world(tw.world(), &cid(fixture::ROUTE))
        .unwrap_or_else(|| panic!("the corridor route materializes:\n{seeded}"));
    let pinned = Ephemeris::from_pack(&corridor, &EphemerisConfig::default())
        .pinned_km(&cid("ceres"), &cid("mars"))
        .expect("a pinned distance");
    assert_eq!(fixture_route.distance_km(), Some(pinned));
    // Its source is the corridor's own pack, never the lane's (reviewer F1).
    let (source, _) = store(&tw)
        .route_source(tw.world(), &fixture_route)
        .expect("the corridor route has a source");
    assert_eq!(source.manifest().id(), corridor.manifest().id());
    assert_eq!(source.manifest().version(), 1);

    // And a world whose sky moves pins the lane at the same chord: the pin is
    // taken at the body's own epoch, never at the config epoch.
    let moving = TestWorld::with_modules(production_roster(lane_config(EphemerisConfig {
        epoch: 0,
        rate: 1_000,
    })));
    let route = store(&moving)
        .route_in_world(moving.world(), &cid(LANE_ROUTE_OUT))
        .expect("the lane route materializes");
    assert_eq!(route.distance_km(), Some(chord));
}

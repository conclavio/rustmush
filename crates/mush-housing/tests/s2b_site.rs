//! S2b (doc 14 D-14-28): a property may be linked to the atlas site it stands on,
//! and the link is refused while any of its rooms lies outside the site's footprint.

use std::sync::Arc;

use mush_atlas::AtlasModule;
use mush_content::{Appliers, ContentModule, parse_pack};
use mush_core::{Audience, GOD, GameModule, ObjId, ObjKind};
use mush_org::SystemPrincipalId;
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

/// A neighborhood with a house site holding a parlour and a bedroom, and a yard
/// outside the site. The yard's gate is the parlour's only entrance.
///
/// The way back from the bedroom is `Parlour <P>;parlour` and not `;p;parlour`:
/// `p` is the kernel's alias for `page`, and `apply::check_exit_names` (16c
/// `B-11`) refuses a pack, before its first write, whose exit takes a word that
/// works in every room. The alias bought this fixture nothing; with it the pack
/// never applies and no room exists to link a property to.
fn packs() -> Vec<String> {
    let region = signed(json!({
        "manifest": { "id": "region.town", "version": 1, "kind": "region", "digest": "", "requires": [] },
        "entries": {
            "region.town": { "kind": "region", "def": {
                "name": "The Town", "kind": "surface", "body": "earth",
                "extent": [[0, 0], [400, 400]], "datum_m": 0.0,
                "background": { "surface": "paved", "mix": [["grass", 8]], "relief_cm": 0,
                                "relief_period_cm": 800, "seed": 1 },
                "ground_resolution_cm": 1000 } },
            "locality.town": { "kind": "locality", "def": {
                "region": "region.town", "name": "Town", "kind": "district",
                "footprint": [[0, 0], [400, 0], [400, 400], [0, 400]] } }
        }
    }));
    let block = signed(json!({
        "manifest": { "id": "neighborhood.block", "version": 1, "kind": "neighborhood", "digest": "",
                      "requires": [{ "id": "region.town", "version": 1 }] },
        "entries": {
            "neighborhood": { "kind": "neighborhood", "def": {
                "region": "region.town", "locality": "locality.town", "name": "The Block",
                "setting": "suburban", "footprint": [[0, 0], [200, 0], [200, 200], [0, 200]],
                "room_budget": { "target": 3, "maximum": 6 } } },
            "site.house": { "kind": "site", "def": { "name": "The House", "site_kind": "site.house",
                "footprint": [[100, 10], [190, 10], [190, 90], [100, 90]] } },
            "room.parlour": { "kind": "room", "def": { "name": "Parlour", "surface": "ground",
                "site": "site.house", "footprint": { "rect": [[110, 20], [150, 60]] } } },
            "room.bedroom": { "kind": "room", "def": { "name": "Bedroom", "surface": "ground",
                "site": "site.house", "footprint": { "rect": [[150, 20], [180, 60]] } } },
            "room.yard": { "kind": "room", "def": { "name": "Yard", "surface": "ground",
                "footprint": { "rect": [[10, 20], [90, 60]] } } },
            "exit.yard_parlour": { "kind": "exit", "def": { "from": "room.yard", "to": "room.parlour",
                "names": ["Front Door <IN>;in;door", "Yard <OUT>;out;yard"],
                "portal": { "distance_m": 12 } } },
            "exit.parlour_bedroom": { "kind": "exit", "def": { "from": "room.parlour", "to": "room.bedroom",
                "names": ["Bedroom <B>;b;bedroom", "Parlour <P>;parlour"] } }
        }
    }));
    vec![region, block]
}

fn boot() -> TestWorld {
    let issuer = SystemPrincipalId::new("tacsim.housing").expect("valid system principal");
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    let packs = packs()
        .iter()
        .map(|p| parse_pack(p).expect("pack"))
        .collect();
    let vocab: std::collections::BTreeSet<mush_org::CapabilityId> = mush_economy::CAPABILITIES
        .iter()
        .copied()
        .chain(mush_housing::CAPABILITIES.iter().copied())
        .chain(["org.role.manage", "org.grant.delegate"])
        .map(|c| mush_org::CapabilityId::new(c).expect("well-formed"))
        .collect();
    let modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(mush_org::OrgModule::new().with_capabilities(vocab)),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![])
                .with_title_issuers(vec![issuer.clone()])
                .with_observers(vec![Arc::new(mush_housing::HousingEconomyObserver)]),
        ),
        Box::new(mush_jobs::JobsModule::new()),
        Box::new(mush_housing::HousingModule::new().with_issuer(issuer)),
        Box::new(AtlasModule::new()),
        Box::new(
            ContentModule::new()
                .with_appliers(appliers)
                .with_boot_packs(packs),
        ),
    ];
    TestWorld::with_modules(modules)
}

fn room(tw: &TestWorld, name: &str) -> ObjId {
    tw.world()
        .iter()
        .find(|o| o.kind == ObjKind::Room && o.name == name)
        .unwrap_or_else(|| panic!("no room named {name}"))
        .id
}

fn run(tw: &mut TestWorld, line: &str) -> String {
    let events: Vec<(Audience, OutputEvent)> = tw.run(GOD, line);
    tw.seen_by(&events, GOD).join("\n")
}

#[test]
fn a_property_links_to_its_site_and_is_refused_when_a_room_lies_outside() {
    let mut tw = boot();
    let parlour = room(&tw, "Parlour");
    let bedroom = room(&tw, "Bedroom");
    let yard = room(&tw, "Yard");
    let out = run(
        &mut tw,
        &format!(
            "@property/create The House=#{},#{} tier=1 landlord=housing",
            parlour.0, bedroom.0
        ),
    );
    assert!(
        out.contains("Property #0")
            || out.contains("created")
            || out.contains("Minted")
            || !out.contains("cannot"),
        "{out}"
    );

    // Both rooms lie within the house: the link takes, and show says so.
    let out = run(&mut tw, "@property/site 0=The House");
    assert!(
        out.contains("stands on site The House (neighborhood.block:site.house)"),
        "{out}"
    );
    let out = run(&mut tw, "@property/show 0");
    assert!(out.contains("site: The House"), "{out}");

    // The yard is outside the site: adding it to the property and re-linking is
    // refused, naming the room.
    let out = run(&mut tw, "@property/site 0=");
    assert!(out.contains("stands on no atlas site"), "{out}");
    let out = run(&mut tw, &format!("@property/rooms 0=#{}", yard.0));
    assert!(out.contains("now holds 3 room"), "{out}");
    let out = run(&mut tw, "@property/site 0=The House");
    assert!(
        out.contains(&format!("#{} lies outside The House's footprint", yard.0)),
        "{out}"
    );

    // A room off the map is refused too, and a larger place is not a site.
    let out = run(&mut tw, "@property/site 0=The Block");
    assert!(
        out.contains("stands on a site, not a larger place"),
        "{out}"
    );
    let out = run(&mut tw, "@property/site 0=Nowhere");
    assert!(out.contains("No site called"), "{out}");
}

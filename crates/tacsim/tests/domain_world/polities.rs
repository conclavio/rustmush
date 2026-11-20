//! **Doc 50 cut KC.0 acceptance — the polities, the regions and the control read** (Q-50-1 to
//! Q-50-4, Q-50-52).
//!
//! A fresh world on the production roster, the game's own boot packs applied, no staff step:
//! the six polities stand as factions from the lane organizations' tags; each lane organization
//! answers to its tag's polity; the lane's regions block has seeded the Kirkwood approach, Ceres
//! and Stickney with doc 11 §5's holders; the control read answers who holds a place, by kind;
//! an enforcer reads its own polity's papers where it claims, else the claimant's; `+territory`
//! shows the regions; and a later apply of the block never undoes a flip.

use crate::support;

use std::collections::BTreeMap;

use mush_content::EntryApplier;
use mush_core::CanonicalId;
use mush_faction::{ControlKind, ControlWrite, FactionId, FactionState};
use mush_testkit::TestWorld;
use tacsim::TacsimStore;
use tacsim::definitions::content::{LANE_JSON, LANE_PACK};
use tacsim::polity::{RegionsApplier, control_at, papers_authority, polity, polity_of_org};

/// The six polity tags (doc 50a C-3), and the faction each becomes.
const POLITIES: [(&str, &str); 6] = [
    ("freeport", "Freeport"),
    ("republic", "Republic"),
    ("concord", "Concord"),
    ("syndicate", "Syndicate"),
    ("company", "Company"),
    ("townships", "Townships"),
];

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

fn factions(tw: &TestWorld) -> &FactionState {
    tw.world().module_state::<FactionState>().expect("factions")
}

/// The faction named `name`.
fn named(tw: &TestWorld, name: &str) -> FactionId {
    factions(tw)
        .faction_by_name(name)
        .unwrap_or_else(|| panic!("no faction {name}"))
        .id
}

/// Every lane organization's id and `OrgId`, from the port maps.
fn lane_orgs(tw: &TestWorld) -> BTreeMap<String, mush_org::OrgId> {
    tw.world()
        .module_state::<TacsimStore>()
        .expect("the store")
        .port_maps()
        .filter(|map| map.pack.as_str() == LANE_PACK)
        .flat_map(|map| {
            map.orgs
                .iter()
                .map(|(id, row)| (id.to_string(), mush_org::OrgId(row.org)))
        })
        .collect()
}

/// The faction of lane organization `id` (made by the seed for an org holder).
fn org_faction(tw: &TestWorld, id: &str) -> FactionId {
    let org = lane_orgs(tw)[id];
    factions(tw)
        .factions
        .values()
        .find(|f| f.org == Some(org))
        .unwrap_or_else(|| panic!("no faction for lane org {id}"))
        .id
}

/// Every lane organization's polity tag, from the shipped lane.
fn org_tags() -> BTreeMap<String, String> {
    let doc: serde_json::Value = serde_json::from_str(LANE_JSON).expect("the lane is JSON");
    let mut tags = BTreeMap::new();
    for entry in doc["entries"].as_object().expect("entries").values() {
        if entry["kind"] == "port" {
            for org in entry["def"]["orgs"].as_array().into_iter().flatten() {
                tags.insert(
                    org["id"].as_str().expect("id").to_owned(),
                    org["faction"].as_str().expect("tag").to_owned(),
                );
            }
        }
    }
    tags
}

#[test]
fn kc0_a_fresh_world_has_six_polities_seeded_regions_and_a_control_read() {
    let mut tw = TestWorld::with_modules(support::production_roster());

    // Q-50-2: the six polities, made from content, no backing organization.
    for (tag, name) in POLITIES {
        let f = &factions(&tw).factions[&named(&tw, name)];
        assert_eq!(
            f.org, None,
            "{name} is a polity, not an organization's faction"
        );
        assert_eq!(polity(tw.world(), tag), Some(f.id), "{tag}");
    }
    // The table: every lane organization answers to its tag's polity.
    let orgs = lane_orgs(&tw);
    let tags = org_tags();
    assert_eq!(orgs.len(), tags.len(), "every lane org made: {orgs:?}");
    for (id, tag) in &tags {
        let want = polity(tw.world(), tag);
        assert!(want.is_some(), "{tag}");
        assert_eq!(
            polity_of_org(tw.world(), orgs[id]),
            want,
            "{id} answers to {tag}"
        );
    }

    let freeport = named(&tw, "Freeport");
    let republic = named(&tw, "Republic");
    let coop = org_faction(&tw, "coop");

    // Q-50-3, doc 11 §5: the Kirkwood approach.
    let kirkwood = control_at(tw.world(), &cid("kirkwood_depot")).expect("Kirkwood is seeded");
    assert_eq!(kirkwood.de_jure, vec![freeport]);
    assert_eq!(kirkwood.de_facto, Some(coop), "the Co-op runs the depot");
    assert_eq!(kirkwood.economic, vec![org_faction(&tw, "tycho_mutual")]);
    assert!(kirkwood.covert.is_empty());
    assert_eq!(
        kirkwood.de_facto_since_ic,
        Some(
            factions(&tw).control[&kirkwood.region]
                .iter()
                .find(|r| r.kind == ControlKind::DeFacto)
                .expect("held")
                .since_ic
        )
    );
    let held = factions(&tw).control[&kirkwood.region]
        .iter()
        .find(|r| r.kind == ControlKind::DeFacto)
        .cloned()
        .expect("held");
    assert_eq!(
        held.contested_by,
        vec![republic],
        "Republic pickets contest"
    );

    // Ceres: one region for the place and its port.
    let ceres = control_at(tw.world(), &cid("ceres_freeport")).expect("Ceres is seeded");
    assert_eq!(
        control_at(tw.world(), &cid("ceres_port")),
        Some(ceres.clone())
    );
    assert_eq!(
        (ceres.de_jure.clone(), ceres.de_facto),
        (vec![freeport], Some(freeport))
    );
    assert_eq!(
        ceres.economic,
        vec![
            org_faction(&tw, "occator"),
            org_faction(&tw, "ceres_clearing")
        ]
    );

    // Stickney: the Republic in all four kinds, and Ceres Clearing's paper.
    let stickney = control_at(tw.world(), &cid("stickney_yards")).expect("Stickney is seeded");
    assert_eq!(
        (
            stickney.de_jure.clone(),
            stickney.de_facto,
            stickney.covert.clone()
        ),
        (vec![republic], Some(republic), vec![republic])
    );
    assert_eq!(
        stickney.economic,
        vec![republic, org_faction(&tw, "ceres_clearing")]
    );

    // A place in no row has no reading (Isidis and the moons come with their own units).
    assert_eq!(control_at(tw.world(), &cid("aitken_highport")), None);

    // Q-50-52: at Kirkwood the Republic's picket reads the Freeport's papers; the Freeport its own.
    let depot = cid("kirkwood_depot");
    assert_eq!(
        papers_authority(tw.world(), &depot, republic),
        Some(freeport)
    );
    assert_eq!(
        papers_authority(tw.world(), &depot, freeport),
        Some(freeport)
    );

    // `+territory` shows the seeded regions to anyone (Q-50-9's date beside each hold).
    let player = tw.spawn_player("Hauler");
    let text = tw
        .run(player, "+territory")
        .iter()
        .map(|(_, event)| mush_testkit::text_of(event))
        .collect::<Vec<_>>()
        .join("\n");
    for line in [
        "Kirkwood approach — Freeport (de jure since ",
        "Ceres station — Freeport (de jure since ",
        "Phobos and Stickney Yards — Republic (de jure since ",
        ", contested by Republic)",
    ] {
        assert!(text.contains(line), "{line:?} missing from:\n{text}");
    }

    // Q-50-52 with two claimants: an enforcer that claims reads its own papers.
    support::with_ctx(&mut tw, |ctx| {
        mush_faction::write_control(
            ctx,
            ControlWrite::new(kirkwood.region, republic, ControlKind::DeJure),
        )
    })
    .expect("a second claimant");
    assert_eq!(
        papers_authority(tw.world(), &depot, republic),
        Some(republic)
    );
    assert_eq!(
        papers_authority(tw.world(), &depot, freeport),
        Some(freeport)
    );

    // A flip stands through a later apply of the block: the seed writes a region once.
    support::with_ctx(&mut tw, |ctx| {
        mush_faction::write_control(
            ctx,
            ControlWrite::new(kirkwood.region, republic, ControlKind::DeFacto),
        )
    })
    .expect("the flip");
    let pack = mush_content::load_pack(LANE_JSON).expect("the lane");
    let id = cid("regions");
    let entry = pack.entries()[&id].clone();
    support::with_ctx(&mut tw, |ctx| {
        RegionsApplier.update(ctx, &pack, &id, &entry, None, &BTreeMap::new())
    })
    .expect("the block re-applies");
    let after = control_at(tw.world(), &depot).expect("still seeded");
    assert_eq!(
        after.de_facto,
        Some(republic),
        "a re-apply never undoes a flip"
    );
    // Doc 50 cut OP.1: the Isidis Concession is the fourth.
    assert_eq!(factions(&tw).regions.len(), 4, "no region made twice");
}

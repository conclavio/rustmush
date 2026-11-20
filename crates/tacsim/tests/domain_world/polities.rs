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

/// A player's view of a command, as one text.
fn seen(tw: &mut TestWorld, who: mush_core::ObjId, line: &str) -> String {
    tw.run(who, line)
        .iter()
        .map(|(_, event)| mush_testkit::text_of(event))
        .collect::<Vec<_>>()
        .join("\n")
}

/// One picket incident's canon draft, as the picket writes it, with `cause` as its cause id.
fn incident(cause: &str) -> mush_canon::CanonDraft {
    mush_canon::CanonDraft {
        kind: "space.customs_fine".into(),
        summary: format!(
            "Republic Customs Corvette fined a hull at the Kirkwood picket ({cause})."
        ),
        entities: Vec::new(),
        scope: mush_canon::CanonScope::Public,
        ic_time: 0,
        source: cause.into(),
        detail: serde_json::Value::Null,
    }
}

/// **Doc 50 cut KC.1** (Q-50-12, Q-50-13, Q-50-51): Customs Assertion stands on the Republic's
/// board from world start at 1 of 6; each picket incident moves it one through its cause id, a
/// retried incident moves it no further; the sixth fills it through the engine's one path, so the
/// Republic takes the Kirkwood approach de facto, the Co-op is moved out, the hold is dated in IC
/// time, the front resolves, and `+territory` and `+chronicle` show it.
#[test]
fn kc1_picket_incidents_fill_customs_assertion_and_the_republic_takes_kirkwood() {
    let mut tw = TestWorld::with_modules(support::production_roster());
    let republic = named(&tw, "Republic");
    let coop = org_faction(&tw, "coop");
    let depot = cid("kirkwood_depot");
    assert_eq!(tacsim::polity::customs_board(tw.world()), Some(republic));

    // Seeded at world start on the Republic's board, 1 of 6, visible on `+clocks`.
    let (front, clock) = {
        let sim = tw
            .world()
            .module_state::<mush_faction::SimState>()
            .expect("fronts");
        let front = sim
            .fronts
            .values()
            .find(|f| f.title == mush_faction::CUSTOMS_ASSERTION_TITLE)
            .cloned()
            .expect("Customs Assertion is seeded");
        let clock = sim
            .clocks
            .values()
            .find(|c| c.front == front.id)
            .cloned()
            .expect("its clock");
        (front, clock)
    };
    assert_eq!(front.faction, republic, "the Republic's board (Q-50-13)");
    assert_eq!((clock.filled, clock.segments), (1, 6), "Q-50-12");
    assert_eq!(
        clock.on_complete_control,
        Some((
            control_at(tw.world(), &depot).expect("Kirkwood").region,
            ControlKind::DeFacto
        ))
    );
    let player = tw.spawn_player("Hauler");
    assert!(
        seen(&mut tw, player, "+clocks").contains("1/6"),
        "the front shows from the first day"
    );

    let record = |tw: &mut TestWorld, cause: &str| {
        support::with_ctx(tw, |ctx| {
            mush_faction::record_customs_assertion(ctx, republic, incident(cause))
        })
        .expect("the incident records")
    };
    let filled = |tw: &TestWorld| {
        tw.world()
            .module_state::<mush_faction::SimState>()
            .expect("fronts")
            .clocks[&clock.id]
            .filled
    };
    for n in 1..=4u8 {
        record(&mut tw, &format!("space.picket.fine/9/{n}"));
        assert_eq!(filled(&tw), 1 + n);
    }
    // A retried incident moves nothing; the Co-op still holds.
    record(&mut tw, "space.picket.fine/9/4");
    assert_eq!(filled(&tw), 5, "a cause counts once");
    assert_eq!(
        control_at(tw.world(), &depot).expect("Kirkwood").de_facto,
        Some(coop)
    );

    // The sixth segment: the flip, on the engine's path.
    record(&mut tw, "space.picket.run/9/77");
    let ic = tw.world().game_now_unix();
    let after = control_at(tw.world(), &depot).expect("Kirkwood");
    assert_eq!(
        after.de_facto,
        Some(republic),
        "the Republic holds the approach"
    );
    assert_eq!(after.de_facto_since_ic, Some(ic), "dated at the flip, IC");
    assert!(
        !factions(&tw).control[&after.region]
            .iter()
            .any(|r| r.faction == coop && r.kind == ControlKind::DeFacto),
        "the Co-op is moved out"
    );
    let sim = tw
        .world()
        .module_state::<mush_faction::SimState>()
        .expect("fronts");
    assert!(sim.fronts[&front.id].resolved, "the front resolves");
    assert!(sim.clocks[&clock.id].completed);

    // A later incident still records its line and moves nothing.
    record(&mut tw, "space.picket.fine/9/99");
    let canon = tw
        .world()
        .module_state::<mush_canon::CanonState>()
        .expect("canon");
    assert_eq!(
        canon
            .recent()
            .iter()
            .filter(|e| e.kind == "space.customs_fine")
            .count(),
        6,
        "four incidents, the sixth segment's and the late one; the retry wrote no line"
    );

    let territory = seen(&mut tw, player, "+territory");
    assert!(
        territory.contains("Kirkwood approach — Freeport (de jure since ")
            && territory.contains("Republic (de facto since "),
        "{territory}"
    );
    let chronicle = seen(&mut tw, player, "+chronicle");
    assert!(
        chronicle.contains(mush_faction::CUSTOMS_ASSERTION_CONSEQUENCE),
        "{chronicle}"
    );
}

/// A fresh production world first booted at IC month `month` (the test clock runs IC at the
/// real second), so a count from month zero would not pass for a count from the seeding month.
fn booted_at_month(month: i64) -> TestWorld {
    let (world, registry) = mush_testkit::boot_world();
    let mut engine = mush_core::Engine::with_modules(world, registry, support::production_roster())
        .expect("the roster registers");
    engine.set_now(month * tacsim::concession::IC_MONTH_S + 3_600);
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    engine
        .modules_loaded(std::collections::HashMap::new(), &mut out, &mut fx)
        .expect("the world boots");
    TestWorld::from_engine(engine)
}

/// Sweep the world one IC month (and a minute) ahead and fire what is due.
fn a_month_later(tw: &mut TestWorld) {
    let rate = i64::from(tw.world().world_timeline().current().rate.max(1));
    let real = (tacsim::concession::IC_MONTH_S + 60 * rate) / rate;
    let t = tw.world().now() + real;
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1_000));
}

/// **Doc 50 cut OP.2 acceptance: the Isidis fronts and their typed causes** (Q-50-20, Q-50-61;
/// doc 11 §6). On a fresh production world:
/// - the Concession Review stands on the Republic's board and Hold the Concession on the
///   Freeport's, each at two of six, with no flip of their own, both on `+clocks`;
/// - the Assembly's monthly tick moves the Review once an IC month, never twice for one month;
/// - a Co-op pool hull flies the Freeport's flag and an untitled object none;
/// - a seizure ruling on the Freeport-flagged hull moves the Review once for its cause, and one on
///   any other hull moves nothing;
/// - the Review fills on the engine's path, records its consequence and resolves its front, while
///   Hold the Concession stays where it opened.
#[test]
#[allow(clippy::too_many_lines)]
fn op2_the_concession_review_moves_by_the_assembly_and_by_a_flagged_seizure() {
    use tacsim::concession::{
        ASSEMBLY_CAUSE, CONCESSION_REVIEW_CONSEQUENCE, CONCESSION_REVIEW_TITLE,
        HOLD_THE_CONCESSION_TITLE, assembly_votes, hold_clock, hull_flag, ic_month,
        record_flagged_seizure, review_clock,
    };

    let mut tw = booted_at_month(5);
    let republic = named(&tw, "Republic");
    let freeport = named(&tw, "Freeport");
    let review = review_clock(tw.world()).expect("the Concession Review is seeded");
    let hold = hold_clock(tw.world()).expect("Hold the Concession is seeded");
    let fronts = |tw: &TestWorld| {
        tw.world()
            .module_state::<mush_faction::SimState>()
            .expect("fronts")
            .clone()
    };
    let sim = fronts(&tw);
    assert_eq!(sim.fronts[&review.front].faction, republic);
    assert_eq!(sim.fronts[&review.front].title, CONCESSION_REVIEW_TITLE);
    assert_eq!(sim.fronts[&hold.front].faction, freeport);
    assert_eq!(sim.fronts[&hold.front].title, HOLD_THE_CONCESSION_TITLE);
    assert_eq!((review.filled, review.segments), (2, 6), "two of six");
    assert_eq!((hold.filled, hold.segments), (2, 6), "two of six");
    assert_eq!(
        review.on_complete_control, None,
        "the operation flips, not the clock"
    );
    assert_eq!(hold.on_complete_control, None);
    let start = ic_month(tw.world().world_ic_now().at);
    assert!(start >= 5, "booted at month {start}");
    assert_eq!(
        review.causes,
        vec![smol_str::SmolStr::new(format!("{ASSEMBLY_CAUSE}{start}"))],
        "the seeding month's vote is the anchor"
    );
    let player = tw.spawn_player("Hauler");
    let clocks = seen(&mut tw, player, "+clocks");
    for line in [
        "Assembly votes & seized Freeport haulers [2/6] — Republic",
        "Reinforcements, stockpiles & haulers through the pickets [2/6] — Freeport",
    ] {
        assert!(clocks.contains(line), "{line:?} missing from: {clocks}");
    }

    let filled = |tw: &TestWorld, clock: &mush_faction::Clock| fronts(tw).clocks[&clock.id].filled;
    // The same month again moves nothing.
    assert_eq!(support::with_ctx(&mut tw, assembly_votes), 0);
    assert_eq!(filled(&tw, &review), 2);
    // A month later the Assembly's tick votes once; a second read in that month votes no more.
    a_month_later(&mut tw);
    assert_eq!(filled(&tw, &review), 3, "the monthly vote, on the tick");
    assert_eq!(support::with_ctx(&mut tw, assembly_votes), 0);
    assert_eq!(filled(&tw, &review), 3);

    // The flag (Q-50-61): a Co-op pool hull is titled to a Freeport organization.
    let pool_hull = tw
        .world()
        .module_state::<TacsimStore>()
        .expect("the store")
        .port_maps()
        .find(|map| map.entry.as_str() == "ceres_port")
        .and_then(|map| map.hulls.first().map(|row| mush_core::ObjId(row.hull)))
        .expect("a Co-op pool hull");
    assert_eq!(hull_flag(tw.world(), pool_hull), Some(freeport));
    assert_eq!(hull_flag(tw.world(), player), None, "untitled: no flag");

    // A seizure ruling on it moves the Review once for its cause.
    let cause = format!("space.picket.seize/{}/77", pool_hull.0);
    assert!(support::with_ctx(&mut tw, |ctx| record_flagged_seizure(
        ctx, pool_hull, &cause
    )));
    assert_eq!(filled(&tw, &review), 4);
    assert!(!support::with_ctx(&mut tw, |ctx| record_flagged_seizure(
        ctx, pool_hull, &cause
    )));
    assert_eq!(filled(&tw, &review), 4, "a cause counts once");
    assert!(!support::with_ctx(&mut tw, |ctx| record_flagged_seizure(
        ctx,
        player,
        "space.picket.seize/9/78"
    )));
    assert_eq!(filled(&tw, &review), 4, "an unflagged hull moves nothing");
    // A hull titled to a Republic organization flies the Republic's flag and moves nothing.
    let cutter = tw.spawn_player("Cutter");
    let yards = lane_orgs(&tw)["stickney_yards"];
    support::with_ctx(&mut tw, |ctx| {
        mush_economy::issue_title(
            ctx,
            cutter,
            mush_org::PrincipalId::Organization(yards),
            None,
            mush_org::SystemPrincipalId::new("tacsim.registry").expect("the issuer"),
            None,
        )
    })
    .expect("the title issues");
    assert_eq!(hull_flag(tw.world(), cutter), Some(republic));
    let cause = format!("space.picket.seize/{}/79", cutter.0);
    assert!(!support::with_ctx(&mut tw, |ctx| record_flagged_seizure(
        ctx, cutter, &cause
    )));
    assert_eq!(
        filled(&tw, &review),
        4,
        "a Republic-flagged hull moves nothing"
    );

    // Two more months fill it: the consequence on the engine's one path.
    a_month_later(&mut tw);
    assert_eq!(filled(&tw, &review), 5);
    a_month_later(&mut tw);
    let sim = fronts(&tw);
    assert!(sim.clocks[&review.id].completed, "the Review fills");
    assert!(sim.fronts[&review.front].resolved, "its front resolves");
    assert_eq!(
        sim.clocks[&hold.id].filled, 2,
        "Hold the Concession is not moved"
    );
    let canon = tw
        .world()
        .module_state::<mush_canon::CanonState>()
        .expect("canon")
        .recent();
    assert!(
        canon
            .iter()
            .any(|e| e.summary.contains(CONCESSION_REVIEW_CONSEQUENCE)),
        "the consequence is recorded"
    );
    // After the fill, nothing moves it.
    a_month_later(&mut tw);
    assert_eq!(support::with_ctx(&mut tw, assembly_votes), 0);
}

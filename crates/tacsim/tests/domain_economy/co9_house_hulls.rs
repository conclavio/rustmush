//! **Doc 47 cut CO.9 — hulls for Houses**, over the composed lane (`driving`'s stocked rig),
//! through registered commands and the port tick.
//!
//! 1. **Staff seed a House's hull** the way the launch bring-up does: `@org/create Meridiani`,
//!    then `@tacsim/port seed ceres_port dray MER-01=org:Meridiani`. The port's fires carry the
//!    commission through laying, docking and the title: the Dray stands docked at Ceres
//!    Freeport titled to Meridiani on its root scope, her hold Meridiani's, a register row.
//!    A second seed under the same mark is refused.
//! 2. **The House's grants**, staff's `@org/grant`: its pilot holds `unit.deploy` on the root
//!    scope, its officer `asset.title.transfer` there (what lets a member read the House's
//!    holdings), a Coprates captain holds `unit.deploy` on Coprates.
//! 3. **The holdings**: the officer's `@asset/list owner=org:Meridiani` lists the Dray.
//! 4. **Refused**: a deckhand with no grant, and the Coprates captain (the right word on the
//!    wrong House), each find no helm and cannot board her.
//! 5. **Taken out**: the pilot boards from the berth gallery, files `+ship/undock stickney`, is
//!    her helm holder under way (the deckhand still has none), and docks at Stickney Yards.
//! 6. **Nothing a player owns changes**: every title held by a character — a Co-op captain's
//!    charter hull among them — reads the same after as before.
//!
//! **Seeds** (doc 43 rule 2.3 #4): the propellant and provisions in the House's hold (a House
//! buys stores through its procurement, which is not this cut's), the actors' places (moves).

use std::collections::BTreeMap;

use mush_core::{GOD, ObjId};
use mush_org::PrincipalId;
use tacsim::port::commission::{Step, YardCommission};

use crate::driving_rig::{
    Rig, cid, file_and_launch, fire, land_quietly, pilot, run, seed_pool, station_room, stocked,
    teleport, to_ic,
};

const MARK: &str = "MER-01";

fn org_named(rig: &Rig, name: &str) -> mush_org::OrgId {
    let store = rig
        .tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .expect("the org store");
    store
        .state()
        .orgs()
        .find(|org| org.name.as_str() == name)
        .map(|org| org.id)
        .unwrap_or_else(|| panic!("no organization {name}"))
}

fn seeded(rig: &Rig) -> Option<YardCommission> {
    tacsim::port::commission::commissions(rig.tw.world())
        .into_iter()
        .find(|record| record.mark == MARK)
}

/// Every title a character holds, by object.
fn player_titles(rig: &Rig) -> BTreeMap<ObjId, PrincipalId> {
    rig.tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("economy")
        .snapshot()
        .assets()
        .filter(|asset| matches!(asset.title_owner, PrincipalId::Object(_)))
        .map(|asset| (asset.object, asset.title_owner.clone()))
        .collect()
}

fn helm(rig: &Rig, who: ObjId) -> Option<ObjId> {
    tacsim::port::coop::helm_of(rig.tw.world(), who).map(|helm| helm.hull)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn co9_a_house_is_seeded_a_hull_and_its_pilot_takes_her_out() {
    let mut rig = stocked();

    // A Co-op captain with a charter hull: a player's holding the House's must not touch.
    let captain = pilot(&mut rig, "Charterer", 1);
    let out = run(&mut rig.tw, captain, "+coop/enrol Long Ox");
    assert!(out.contains("now the Long Ox"), "{out}");
    let charter_hull = helm(&rig, captain).expect("the captain's charter hull");
    let charter_title = title_of(&rig, charter_hull);
    assert!(charter_title.is_some(), "the charter hull is titled");
    // And a title of their own (staff's fixture: a thing titled to them).
    let made = run(&mut rig.tw, GOD, "@create Charterer's Locker");
    let locker = made
        .split('#')
        .nth(1)
        .and_then(|rest| {
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()
        })
        .map(ObjId)
        .unwrap_or_else(|| panic!("a thing: {made}"));
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/title #{}=#{}", locker.0, captain.0),
    );

    // ---- (1) The House and its seeded hull ----
    for house in ["Meridiani", "Coprates"] {
        let out = run(&mut rig.tw, GOD, &format!("@org/create {house}"));
        assert!(out.contains("Created org #"), "{out}");
    }
    let meridiani = org_named(&rig, "Meridiani");
    let before = player_titles(&rig);
    assert_eq!(
        before.get(&locker),
        Some(&PrincipalId::Object(captain)),
        "the captain holds a title before the House is seeded"
    );
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@tacsim/port seed ceres_port dray {MARK}=org:Meridiani"),
    );
    assert!(out.contains("Seeded order "), "{out}");
    let again = run(
        &mut rig.tw,
        GOD,
        &format!("@tacsim/port seed ceres_port dray {MARK}=org:Meridiani"),
    );
    assert!(
        again.contains("already a hull's"),
        "a mark seeds once: {again}"
    );
    for _ in 0..30 {
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
        if seeded(&rig).is_some_and(|record| record.step == Step::Done) {
            break;
        }
    }
    let record = seeded(&rig).expect("the seed's commission");
    assert_eq!(
        record.step,
        Step::Done,
        "laid up within 30 fires: {record:?}"
    );
    let dray = record.object().expect("the Dray");
    let asset = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| e.snapshot().asset(dray).cloned())
        .expect("her title");
    let root = rig
        .tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| s.state().org(meridiani).map(|o| o.root_scope))
        .expect("Meridiani's root scope");
    assert_eq!(asset.title_owner, PrincipalId::Organization(meridiani));
    assert_eq!(asset.scope, Some(root), "titled on the House's root scope");
    assert!(
        matches!(tacsim::port::ship::berth_of(rig.tw.world(), dray),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry == cid("ceres_port")),
        "docked at Ceres Freeport"
    );
    assert_eq!(
        tacsim::port::owned::owned(rig.tw.world(), dray).map(|row| row.mark.as_str()),
        Some(MARK),
        "a register row under her mark"
    );

    // ---- (2) The House's people ----
    let gallery = station_room(&rig.tw, "ceres_port", "room.c01_berth_gallery");
    let flyer = pilot(&mut rig, "Housepilot", 2);
    let officer = pilot(&mut rig, "Houseofficer", 3);
    let deckhand = pilot(&mut rig, "Deckhand", 4);
    let rival = pilot(&mut rig, "Rival", 5);
    for (who, org, capability) in [
        (flyer, "Meridiani", "unit.deploy"),
        (officer, "Meridiani", "asset.title.transfer"),
        (rival, "Coprates", "unit.deploy"),
    ] {
        let out = run(
            &mut rig.tw,
            GOD,
            &format!("@org/grant #{}={org} {capability}", who.0),
        );
        assert!(out.contains("Granted "), "{out}");
    }

    // ---- (3) The House's holdings ----
    let holdings = run(&mut rig.tw, officer, "@asset/list owner=org:Meridiani");
    assert!(
        holdings.contains(&format!("#{}", dray.0)),
        "the officer reads the Dray in the House's holdings: {holdings}"
    );

    // ---- (4) Refused: no grant, and the right word on the wrong House ----
    for who in [deckhand, rival] {
        assert_eq!(helm(&rig, who), None, "#{} has no helm", who.0);
        teleport(&mut rig.tw, who, gallery);
        let out = run(&mut rig.tw, who, "+ship/board");
        assert!(
            !out.contains("You cycle the lock into the "),
            "#{} is refused the House's hull: {out}",
            who.0
        );
    }

    // ---- (5) The pilot takes her out ----
    assert_eq!(
        helm(&rig, captain),
        Some(charter_hull),
        "the seed and the grants leave the captain's charter alone"
    );
    assert_eq!(
        helm(&rig, flyer),
        Some(dray),
        "the pilot's helm is the Dray"
    );
    let hold = mush_economy::VaultId(record.hold.expect("her hold"));
    seed_pool(
        &mut rig,
        hold,
        "propellant",
        60,
        mush_inventory::LotId(9_900),
    );
    seed_pool(
        &mut rig,
        hold,
        "provisions",
        40,
        mush_inventory::LotId(9_901),
    );
    teleport(&mut rig.tw, flyer, gallery);
    let out = run(&mut rig.tw, flyer, "+ship/board");
    assert!(out.contains("You cycle the lock into the "), "{out}");
    let ticket = file_and_launch(&mut rig, flyer, dray, "+ship/undock stickney");
    assert_eq!(
        tacsim::port::coop::helm_holder(rig.tw.world(), dray),
        Some(flyer),
        "the member flying her commands her"
    );
    assert_eq!(helm(&rig, deckhand), None, "the deckhand still has no helm");
    let _ = to_ic(&mut rig, ticket.plan.depart_ic + ticket.plan.total_s);
    let _ = to_ic(
        &mut rig,
        ticket.plan.depart_ic + ticket.plan.total_s + 3_600,
    );
    assert!(
        matches!(tacsim::port::ship::berth_of(rig.tw.world(), dray),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry == cid("stickney_port")),
        "the House's Dray docked at Stickney Yards"
    );
    // The pilot flies her and trades nothing from the House's hold: a part in her hold, and the
    // Yards' bid for it, refused to the pilot without office.
    const PLANT: &str = "tacsim.part.plant.light_mk1";
    crate::driving_rig::seed_lot(&mut rig, hold, PLANT, 1, mush_inventory::LotId(9_902));
    crate::in10_pools::settle(&mut rig, 4);
    let ashore = run(&mut rig.tw, flyer, "+ship/leave");
    assert!(!ashore.contains("Usage"), "{ashore}");
    let bid = crate::driving_rig::port_map(&rig.tw, "stickney_port").vendors
        [&cid("stickney_yards")]
        .bids[&cid(PLANT)]
        .listing;
    let sold = run(&mut rig.tw, flyer, &format!("+market/sell {bid}=1"));
    assert!(
        !sold.contains("Sold ") && sold.contains("Name the inventory endpoint"),
        "a pilot without office is offered no House hold to sell from: {sold}"
    );

    // ---- (6) Nothing a player owns changed ----
    assert_eq!(
        player_titles(&rig),
        before,
        "every character's title is as it was"
    );
    assert_eq!(
        title_of(&rig, charter_hull),
        charter_title,
        "the captain's charter hull is titled as it was"
    );
    // The rig's clock runs the passage at rate 1, past the Co-op's seven real days of dormancy:
    // the captain's idle charter ends by the clock (`Dormant`) and by nothing the House did.
    let ended = crate::driving_rig::store(&rig.tw)
        .enrolment_of(captain)
        .map(|e| e.ended.clone());
    assert!(
        matches!(
            ended,
            None | Some(None) | Some(Some(tacsim::port::coop::EndCause::Dormant))
        ),
        "the captain's charter ends only by its own clock: {ended:?}"
    );
}

fn title_of(rig: &Rig, obj: ObjId) -> Option<PrincipalId> {
    rig.tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| e.snapshot().asset(obj).map(|a| a.title_owner.clone()))
}

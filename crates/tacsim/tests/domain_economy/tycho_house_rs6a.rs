//! **Doc 41 cut RS.6a — Tycho Mutual's house** (Q-41-15, Q-41-17), over the composed lane
//! (`driving`'s stocked rig): lane v28's `recovery.house` is laid up by the port tick as two yard
//! commissions titled to Tycho Mutual — the 600 t lifter docked at Ceres Freeport, the 130 t
//! crawler in its recovery bay — with the house's dispatcher in the port's host room, and Tycho's
//! capital is 500,000 cr. Nothing here is a fixture: the lane and the port tick do it all.

use mush_core::ObjId;
use mush_org::PrincipalId;

use crate::driving_rig::{Rig, cid, fire, land_quietly, port_map, stocked};

fn title(rig: &Rig, hull: ObjId) -> Option<PrincipalId> {
    rig.tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| e.snapshot().asset(hull).map(|a| a.title_owner.clone()))
}

/// **The house lays up on the port tick**, and a second pass makes nothing more.
#[test]
fn rs6a_tycho_lays_up_its_lifter_and_crawler_at_ceres() {
    let mut rig = stocked();
    let ceres = cid("ceres_port");
    let mut house = None;
    for _ in 0..60 {
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
        if port_map(&rig.tw, "ceres_port")
            .house
            .as_ref()
            .is_some_and(|row| row.stowed)
        {
            house = tacsim::port::house::hulls(rig.tw.world(), &ceres);
            break;
        }
    }
    let (lifter, crawler) = house.expect("the house is laid up within a minute of fires");
    let map = port_map(&rig.tw, "ceres_port");
    let tycho = map
        .orgs
        .iter()
        .find(|(id, _)| id.as_str() == "tycho_mutual")
        .map(|(_, row)| PrincipalId::Organization(mush_org::OrgId(row.org)))
        .expect("Tycho's org row");
    assert_eq!(
        title(&rig, lifter),
        Some(tycho.clone()),
        "the lifter is Tycho's"
    );
    assert_eq!(title(&rig, crawler), Some(tycho), "the crawler is Tycho's");
    // The lifter docked at Ceres, the crawler in her bay.
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), lifter),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry == ceres
        ),
        "the lifter is docked at Ceres"
    );
    assert_eq!(
        tacsim::surface::act::where_is(rig.tw.world(), crawler),
        tacsim::surface::act::Where::Bay { hull: lifter },
        "the crawler rides in the lifter's bay"
    );
    let unit = |obj: ObjId| {
        rig.tw
            .world()
            .module_state::<mush_tactics::UnitStore>()
            .and_then(|units| units.units.get(&obj).map(|u| u.schema().to_string()))
    };
    // The unit's schema names its design (`tacsim:unit:{design}@{version}#{digest}`).
    assert!(
        unit(lifter).is_some_and(
            |s| s.contains(&format!(":{}@", tacsim::definitions::content::TYCHO_LIFTER))
        ),
        "{:?}",
        unit(lifter)
    );
    assert!(
        unit(crawler).is_some_and(|s| s.contains(&format!(
            ":{}@",
            tacsim::definitions::content::TYCHO_CRAWLER
        ))),
        "{:?}",
        unit(crawler)
    );
    let dispatcher =
        tacsim::port::house::dispatcher(rig.tw.world(), &ceres).expect("the dispatcher");
    assert_eq!(
        rig.tw.world().get(dispatcher).and_then(|o| o.location),
        map.host.map(ObjId),
        "the dispatcher waits in the port's host room"
    );
    // Q-41-17: Tycho's capital is 500,000 cr.
    let account = map
        .orgs
        .iter()
        .find(|(id, _)| id.as_str() == "tycho_mutual")
        .and_then(|(_, row)| row.account)
        .map(mush_economy::AccountId)
        .expect("Tycho's account");
    let balance = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .account(account)
                .map(|a| a.posted_balance.minor())
        })
        .expect("the balance");
    assert!(balance >= 50_000_000, "500,000 cr capital: {balance}");
    // Another minute of fires makes nothing more.
    let before = tacsim::port::commission::commissions(rig.tw.world()).len();
    for _ in 0..10 {
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
    }
    assert_eq!(
        tacsim::port::commission::commissions(rig.tw.world()).len(),
        before,
        "the house seeds once"
    );
    assert_eq!(
        tacsim::port::house::hulls(rig.tw.world(), &ceres),
        Some((lifter, crawler))
    );
}

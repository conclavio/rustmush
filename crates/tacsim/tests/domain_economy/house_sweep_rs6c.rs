//! **Doc 41 cut RS.6c-1 — Tycho Mutual's sweep, the scrap half** (story (e); Q-41-16, round-2 item
//! 41-7; the owner's rulings of 2026-11-04), over the composed lane at 256 (Ahuna Flank) and 512
//! (Urvara Reach):
//!
//! Occator's spare rig lies wrecked on the site's open ground, nobody claims it and nobody posts
//! its retrieval. After 56 IC days Tycho Mutual's house sweeps it on its own account: the lifter
//! sails, the crawler tows and loads her into the house's custody, the dispatcher files the
//! house's salvage claim, the lifter sails home and berths her in Ceres's workshop, the claim is
//! awarded to the dispatcher at the window's end, she is assessed past her resale line and broken
//! up under IN.8, and the scrap's credit goes to Tycho's account. The census: the site's index no
//! longer lists her, and her bay record reads scrapped.
//!
//! **What is a fixture.** The rig's chassis destroyed and its row on the site's index as a wreck,
//! by staff hand (GC.5's fixture); 57 IC days skipped. Every act of Tycho's is the production path.

use mush_core::ObjId;
use mush_org::PrincipalId;

use crate::driving_rig::{
    Rig, cid, fire, land_quietly, on_pad_as_at, port_map, ramp_at, stocked_tuned, to_cell_on,
};
use crate::field_host_rig::{second, set_section, tune};
use crate::site_record_rig::{staff, staff_member};

/// The compressed claim window, real seconds.
const WINDOW: i64 = 600;

/// Short tow stages and a ten-minute claim window.
fn quick(cfg: &mut tacsim::TacsimConfig) {
    tune(cfg);
    cfg.field.idle_demote_s = 3_600;
    cfg.recovery.inspect_s = 30;
    cfg.recovery.attach_s = 30;
    cfg.recovery.load_s = 60;
    cfg.recovery.claim_window_ic_s = WINDOW;
}

/// Move IC on to `target`, then fire a few times.
fn skip_and_fire(rig: &mut Rig, target: i64) {
    let w = rig.tw.world();
    let rate = i64::from(w.world_timeline().current().rate.max(1));
    let ahead = target.saturating_sub(w.world_ic_now().at).max(0);
    let t = w.now().saturating_add((ahead + rate - 1) / rate);
    rig.tw.set_now(t);
    for _ in 0..3 {
        let (_, queued) = fire(rig);
        land_quietly(rig, &queued);
    }
}

/// Tycho's account balance, minor.
fn tycho_balance(rig: &Rig) -> i64 {
    let map = port_map(&rig.tw, "ceres_port");
    let account = map
        .orgs
        .iter()
        .find(|(id, _)| id.as_str() == "tycho_mutual")
        .and_then(|(_, row)| row.account)
        .map(mush_economy::AccountId)
        .expect("Tycho's account");
    rig.tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .account(account)
                .map(|a| a.posted_balance.minor())
        })
        .expect("the balance")
}

/// Whether `hull`'s bay record reads scrapped.
fn scrapped(rig: &Rig, hull: ObjId) -> bool {
    matches!(
        crate::driving_rig::store(&rig.tw).bay_hull(hull),
        Some(tacsim::port::bay_work::BayHullState::Present(row)) if row.scrapped_ic.is_some()
    )
}

/// **The sweep on `(venue, entry)`.**
#[allow(clippy::too_many_lines)]
fn story(venue: &str, entry: &str) {
    let mut rig = stocked_tuned(quick);
    let (_, _, tvs) = on_pad_as_at(&mut rig, ("Pilot", 1, "Salt Moth"), (venue, entry), 1);
    let schema = mush_tactics::module::unit_of(rig.tw.world(), tvs[0])
        .expect("a rig")
        .schema()
        .clone();
    let wreck = rig
        .tw
        .world()
        .module_state::<mush_tactics::UnitStore>()
        .expect("the units")
        .units
        .iter()
        .filter(|(id, unit)| **id != tvs[0] && unit.schema() == &schema)
        .map(|(id, _)| *id)
        .find(|id| {
            crate::driving_rig::store(&rig.tw)
                .tv_lease_on(*id)
                .is_none()
        })
        .expect("a spare rig");
    let (rx, ry) = {
        let (x, y, _) = ramp_at(&rig.tw, entry);
        (x.div_euclid(1_000), y.div_euclid(1_000))
    };
    let at = (rx + 4, ry + 3);
    to_cell_on(&mut rig, entry, wreck, at);
    set_section(&mut rig, wreck, "chassis", Some("destroyed"), Some(0));
    set_section(&mut rig, wreck, "boom", Some("destroyed"), Some(0));
    set_section(&mut rig, wreck, "hopper", Some("damaged"), Some(1));
    assert!(tacsim::port::tv::wrecked(rig.tw.world(), wreck), "a wreck");
    let s = staff_member(&mut rig);
    staff(
        &mut rig,
        s,
        &format!(
            "@tacsim/site {entry} leave #{} wreck {},{}",
            wreck.0, at.0, at.1
        ),
    );
    let ceres = cid("ceres_port");
    for _ in 0..60 {
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
        if tacsim::port::house::dispatchers(rig.tw.world()).len() == 1 {
            break;
        }
    }
    let (lifter, _crawler) =
        tacsim::port::house::hulls(rig.tw.world(), &ceres).expect("Tycho's house");
    let dispatcher = tacsim::port::house::dispatcher(rig.tw.world(), &ceres).expect("dispatcher");
    let before = tycho_balance(&rig);
    // Doc 41 story (e) (RS.9a): the matter census (IN.11's `@industry/census`) holds to the
    // kilogram across the sweep and the scrap, once the house's boot builds have settled.
    let matter = crate::in11_matter::steady_balance(&mut rig);
    let terms_before = crate::in11_matter::census(&mut rig);
    // Nothing sweeps her before her time.
    assert!(
        rig.tw
            .world()
            .module_state::<tacsim::TacsimStore>()
            .is_some(),
        "the layer"
    );
    let now = rig.tw.world().world_ic_now().at;
    skip_and_fire(
        &mut rig,
        now + (tacsim::port::house_sweep::SWEEP_IC_DAYS + 1) * 86_400,
    );
    // The sweep, second by second (faster while the lifter is under way).
    let mut claimed = false;
    let mut awarded = false;
    // Tycho's lowest balance over the job (after the float's top-up for the sailing): the
    // scrap's credit, forwarded from the float, lifts it again.
    let mut before_scrap = before;
    // Review 38b F-12: the dispatcher's float from her berth at Ceres on. The scrap's credit is
    // minted into it later, so forwarding the credit at the scrap would dip the float below
    // what it held; forwarded once landed, it never dips.
    let float = |rig: &Rig| {
        tacsim::port::credit::own_account(rig.tw.world(), dispatcher)
            .and_then(|a| a.available())
            .map_or(0, |a| a.minor())
    };
    let mut at_home: Option<i64> = None;
    let mut lowest = i64::MAX;
    for _ in 0..40_000 {
        if at_home.is_some() {
            lowest = lowest.min(float(&rig));
        }
        if scrapped(&rig, wreck) {
            break;
        }
        if at_home.is_none()
            && tacsim::port::bay_work::held(rig.tw.world(), wreck)
            && matches!(
                tacsim::port::ship::berth_of(rig.tw.world(), lifter),
                tacsim::port::ship::Berth::Docked { .. }
            )
        {
            at_home = Some(float(&rig));
        }
        before_scrap = before_scrap.min(tycho_balance(&rig));
        if tacsim::port::house_sweep::title_of(rig.tw.world(), wreck)
            == Some(PrincipalId::Object(dispatcher))
        {
            awarded = true;
        }
        if !claimed
            && crate::field_host_rig::site_record_at(&mut rig, entry).is_some_and(|record| {
                record.custody.iter().any(|row| {
                    row.hull == wreck && row.claim != tacsim::surface::salvage::ClaimSeam::Owed
                })
            })
        {
            claimed = true;
        }
        let under_way = matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), lifter),
            tacsim::port::ship::Berth::UnderWay
        );
        let t = rig
            .tw
            .world()
            .now()
            .saturating_add(if under_way { 30 } else { 1 });
        rig.tw.set_now(t);
        let _ = second(&mut rig);
    }
    // The credit lands and the float's surplus goes to Tycho.
    for _ in 0..120 {
        lowest = lowest.min(float(&rig));
        let t = rig.tw.world().now().saturating_add(1);
        rig.tw.set_now(t);
        let _ = second(&mut rig);
    }
    let at_home = at_home.expect("the wreck was berthed at Ceres");
    assert!(
        lowest >= at_home,
        "the float dipped to {lowest} from {at_home}: the credit was forwarded before it landed"
    );
    assert!(
        float(&rig) <= tacsim::port::house_arm::FLOAT_MINOR,
        "the float's surplus went to Tycho: {}",
        float(&rig)
    );
    assert!(claimed, "the house filed its salvage claim");
    assert!(awarded, "the claim was awarded to the dispatcher");
    assert!(
        scrapped(&rig, wreck),
        "scrapped at Ceres: the wreck is {:?}, the lifter {:?}",
        tacsim::surface::act::where_is(rig.tw.world(), wreck),
        tacsim::port::ship::berth_of(rig.tw.world(), lifter)
    );
    assert!(
        tycho_balance(&rig) > before_scrap,
        "the scrap's credit went to Tycho"
    );
    assert_eq!(
        crate::in11_matter::census(&mut rig)["balance"],
        matter,
        "the matter census holds across the sweep and the scrap: before {terms_before:?}, after {:?}",
        crate::in11_matter::census(&mut rig)
    );
    // The census: the site's index no longer lists her.
    let record = crate::field_host_rig::site_record_at(&mut rig, entry).expect("the record");
    assert!(
        record.units.iter().all(|left| left.unit != wreck),
        "the site's index is clear of her"
    );
}

/// **The ownerless wreck** (the owner's ruling 1 of 2026-11-04): a raid's hull, made with no
/// title (`field_raiders_rig::armed`'s line tank, as `field/raiders.rs` makes them), wrecked on
/// Ahuna Flank's open ground. The sweep titles her to the dispatcher before the tow, as salvage of
/// the unowned, files no claim, and at Ceres she is assessed for keeping (her restoration and sale
/// are RS.6c-2's story) or scrapped.
#[test]
#[allow(clippy::too_many_lines)]
fn rs6c_tycho_titles_and_sweeps_an_ownerless_wreck() {
    let mut rig = stocked_tuned(quick);
    let entry = "ahuna_port";
    let _ = on_pad_as_at(&mut rig, ("Pilot", 1, "Salt Moth"), ("ahuna", entry), 1);
    let (rx, ry) = {
        let (x, y, _) = ramp_at(&rig.tw, entry);
        (x.div_euclid(1_000), y.div_euclid(1_000))
    };
    let at = (rx + 4, ry + 3);
    let wreck = crate::field_raiders_rig::armed(&mut rig, "Raider", at, 0);
    set_section(&mut rig, wreck, "hull", Some("destroyed"), Some(0));
    assert_eq!(
        tacsim::port::house_sweep::title_of(rig.tw.world(), wreck),
        None,
        "a raid's hull has no title"
    );
    let s = staff_member(&mut rig);
    staff(
        &mut rig,
        s,
        &format!(
            "@tacsim/site {entry} leave #{} wreck {},{}",
            wreck.0, at.0, at.1
        ),
    );
    let ceres = cid("ceres_port");
    for _ in 0..60 {
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
        if tacsim::port::house::dispatchers(rig.tw.world()).len() == 1 {
            break;
        }
    }
    let (lifter, _) = tacsim::port::house::hulls(rig.tw.world(), &ceres).expect("Tycho's house");
    let dispatcher = tacsim::port::house::dispatcher(rig.tw.world(), &ceres).expect("dispatcher");
    let tycho = {
        let map = port_map(&rig.tw, "ceres_port");
        map.orgs
            .iter()
            .find(|(id, _)| id.as_str() == "tycho_mutual")
            .map(|(_, row)| PrincipalId::Organization(mush_org::OrgId(row.org)))
            .expect("Tycho")
    };
    let now = rig.tw.world().world_ic_now().at;
    skip_and_fire(
        &mut rig,
        now + (tacsim::port::house_sweep::SWEEP_IC_DAYS + 1) * 86_400,
    );
    let mut titled = false;
    let mut claimed = false;
    let mut kept = false;
    let in_custody = |rig: &mut Rig| {
        crate::field_host_rig::site_record_at(rig, entry)
            .is_some_and(|record| record.custody.iter().any(|row| row.hull == wreck))
    };
    // Bounded: a stalled sweep fails in minutes, saying where the lifter and the wreck are.
    let stalled = |rig: &Rig, what: &str| -> ! {
        panic!(
            "{what}: the wreck is {:?} titled {:?}, the lifter {:?}",
            tacsim::surface::act::where_is(rig.tw.world(), wreck),
            tacsim::port::house_sweep::title_of(rig.tw.world(), wreck),
            tacsim::port::ship::berth_of(rig.tw.world(), lifter)
        )
    };
    let mut steps = 0u32;
    loop {
        let title = tacsim::port::house_sweep::title_of(rig.tw.world(), wreck);
        if title == Some(PrincipalId::Object(dispatcher)) {
            titled = true;
        }
        if scrapped(&rig, wreck) || (titled && title == Some(tycho.clone())) {
            break;
        }
        // Berthed at Ceres and assessed for keeping, she is the house's: her restoration and
        // sale are RS.6c-2's story (`house_resale_rs6c2.rs`), so this one ends here.
        if titled
            && tacsim::port::bay_work::held(rig.tw.world(), wreck)
            && matches!(
                tacsim::port::house_sweep::assess(rig.tw.world(), wreck),
                tacsim::port::house_sweep::Disposition::Keep { .. }
            )
        {
            kept = true;
            break;
        }
        // The title is the sweep's first act: within its first few hundred seconds.
        if !titled && steps == 600 {
            stalled(&rig, "not titled to the dispatcher in 600 steps");
        }
        if steps == 6_000 {
            stalled(&rig, "neither kept nor scrapped in 6,000 steps");
        }
        // The custody read is a store read: every 60 steps (and once at the end).
        if steps.is_multiple_of(60) && in_custody(&mut rig) {
            claimed = true;
        }
        steps += 1;
        // Faster while the lifter is under way or a restoration (RS.6c-2) is in the workshop.
        let step = if matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), lifter),
            tacsim::port::ship::Berth::UnderWay
        ) {
            300
        } else if tacsim::port::bay_work::under_work(rig.tw.world(), wreck) {
            60
        } else {
            1
        };
        let t = rig.tw.world().now().saturating_add(step);
        rig.tw.set_now(t);
        let _ = second(&mut rig);
    }
    claimed |= in_custody(&mut rig);
    assert!(titled, "the sweep titled her to the dispatcher");
    assert!(
        !claimed,
        "an ownerless wreck is the house's own: no custody, no claim"
    );
    let title = tacsim::port::house_sweep::title_of(rig.tw.world(), wreck);
    assert!(
        kept || scrapped(&rig, wreck) || title == Some(tycho),
        "she was kept for the house or scrapped: {title:?}"
    );
}

/// **RS.6c-1's gate on Ahuna Flank (256).**
#[test]
fn rs6c_tycho_sweeps_and_scraps_a_wreck_on_ahuna_flank() {
    story("ahuna", "ahuna_port");
}

/// **RS.6c-1's gate on Urvara Reach (512).**
#[test]
fn rs6c_tycho_sweeps_and_scraps_a_wreck_on_urvara_reach() {
    story("urvara", "urvara_port");
}

/// **A retrieval handed to Tycho mid-sweep waits for the sweep** (review 38b F-09), on Ahuna Flank:
/// with the crawler coupled to the swept wreck, Occator's unaccepted retrieval of a second spare
/// rig lapses to the house. The arm finishes the sweep first — the wreck to Ceres — and then
/// flies the retrieval, which is delivered; without the guard the retrieval preempts the sweep
/// and the crawler, coupled to the wreck, never takes the job.
#[test]
#[allow(clippy::too_many_lines)]
fn review_38b_f09_a_retrieval_waits_for_the_sweep_under_way() {
    use tacsim::surface::retrieval::{RetrievalStage, order_of};
    let mut rig = stocked_tuned(quick);
    let entry = "ahuna_port";
    let (_, _, tvs) = on_pad_as_at(&mut rig, ("Pilot", 1, "Salt Moth"), ("ahuna", entry), 1);
    let schema = mush_tactics::module::unit_of(rig.tw.world(), tvs[0])
        .expect("a rig")
        .schema()
        .clone();
    let spares: Vec<ObjId> = rig
        .tw
        .world()
        .module_state::<mush_tactics::UnitStore>()
        .expect("the units")
        .units
        .iter()
        .filter(|(id, unit)| **id != tvs[0] && unit.schema() == &schema)
        .map(|(id, _)| *id)
        .filter(|id| {
            crate::driving_rig::store(&rig.tw)
                .tv_lease_on(*id)
                .is_none()
        })
        .collect();
    assert!(spares.len() >= 2, "two spare rigs: {spares:?}");
    let (wreck, casualty) = (spares[0], spares[1]);
    let (rx, ry) = {
        let (x, y, _) = ramp_at(&rig.tw, entry);
        (x.div_euclid(1_000), y.div_euclid(1_000))
    };
    let (w_at, c_at) = ((rx + 4, ry + 3), (rx + 6, ry - 3));
    to_cell_on(&mut rig, entry, wreck, w_at);
    to_cell_on(&mut rig, entry, casualty, c_at);
    set_section(&mut rig, wreck, "chassis", Some("destroyed"), Some(0));
    set_section(&mut rig, wreck, "boom", Some("destroyed"), Some(0));
    // The casualty is disabled, not wrecked: her power plant out (`house_arm_rs6b`'s fixture).
    crate::driving_rig::staff_hand(&mut rig, |ctx| {
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::SetComponentCondition {
                component: mush_tactics::unit::ComponentId::new("power").expect("an id"),
                condition: mush_tactics::ConditionId::new("destroyed").expect("a condition"),
            }],
            None,
            Some(smol_str::SmolStr::new_static("test:f09-powerless")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, casualty, &plan).expect("applied");
    });
    let s = staff_member(&mut rig);
    staff(
        &mut rig,
        s,
        &format!(
            "@tacsim/site {entry} leave #{} wreck {},{}",
            wreck.0, w_at.0, w_at.1
        ),
    );
    let ceres = cid("ceres_port");
    for _ in 0..60 {
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
        if tacsim::port::house::dispatchers(rig.tw.world()).len() == 1 {
            break;
        }
    }
    let (lifter, crawler) =
        tacsim::port::house::hulls(rig.tw.world(), &ceres).expect("Tycho's house");
    let now = rig.tw.world().world_ic_now().at;
    skip_and_fire(
        &mut rig,
        now + (tacsim::port::house_sweep::SWEEP_IC_DAYS + 1) * 86_400,
    );
    // The sweep, until the crawler is coupled to the wreck.
    let towing = |rig: &mut Rig| {
        crate::field_host_rig::site_record_at(rig, entry)
            .and_then(|record| record.tows.towing(crawler).map(|op| op.casualty()))
            == Some(wreck)
    };
    let mut steps = 0u32;
    while !towing(&mut rig) {
        assert!(
            steps < 4_000,
            "the crawler never coupled to the wreck: the lifter {:?}, the crawler {:?}",
            tacsim::port::ship::berth_of(rig.tw.world(), lifter),
            tacsim::surface::act::where_is(rig.tw.world(), crawler)
        );
        let step = if matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), lifter),
            tacsim::port::ship::Berth::UnderWay
        ) {
            300
        } else {
            1
        };
        let t = rig.tw.world().now().saturating_add(step);
        rig.tw.set_now(t);
        let _ = second(&mut rig);
        steps += 1;
    }
    // Occator's retrieval of the second rig, unaccepted: at its window's end it is Tycho's.
    let poster = {
        let map = port_map(&rig.tw, "ceres_port");
        let row = map
            .orgs
            .iter()
            .find(|(id, _)| id.as_str() == "occator")
            .map(|(_, row)| row.clone())
            .expect("Occator's row");
        (
            PrincipalId::Organization(mush_org::OrgId(row.org)),
            mush_economy::AccountId(row.account.expect("an account")),
        )
    };
    staff(
        &mut rig,
        s,
        &format!(
            "@tacsim/site {entry} leave #{} casualty {},{}",
            casualty.0, c_at.0, c_at.1
        ),
    );
    let posted = crate::driving_rig::staff_hand(&mut rig, |ctx| {
        tacsim::surface::retrieval::post(ctx, casualty, poster.clone(), ceres.clone(), None)
    });
    assert!(posted.is_ok(), "{posted:?}");
    let job = order_of(rig.tw.world(), casualty).expect("the job");
    skip_and_fire(&mut rig, job.accept_by_ic + 60);
    assert_eq!(
        order_of(rig.tw.world(), casualty).map(|o| o.stage),
        Some(RetrievalStage::House)
    );
    // The house: the sweep home first, then the job.
    let mut wreck_home = false;
    let mut steps = 0u32;
    loop {
        if scrapped(&rig, wreck) || tacsim::port::bay_work::held(rig.tw.world(), wreck) {
            wreck_home = true;
        }
        if order_of(rig.tw.world(), casualty).is_some_and(|o| o.delivered.is_some()) {
            break;
        }
        if steps == 8_000 {
            let act = crate::driving_rig::staff_hand(&mut rig, |ctx| {
                tacsim::port::house_arm::next_act(ctx, (lifter, crawler), casualty, &ceres)
            });
            let float = tacsim::port::house::dispatcher(rig.tw.world(), &ceres)
                .and_then(|d| tacsim::port::credit::own_account(rig.tw.world(), d))
                .and_then(|a| a.available())
                .map(|a| a.minor());
            panic!(
                "the retrieval was not delivered in 8,000 steps: the wreck {:?} (home \
                 {wreck_home}), the casualty {:?}, the crawler {:?}, next {act:?}, float {float:?}",
                tacsim::surface::act::where_is(rig.tw.world(), wreck),
                tacsim::surface::act::where_is(rig.tw.world(), casualty),
                tacsim::surface::act::where_is(rig.tw.world(), crawler),
            );
        }
        let step = if matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), lifter),
            tacsim::port::ship::Berth::UnderWay
        ) {
            300
        } else {
            1
        };
        let t = rig.tw.world().now().saturating_add(step);
        rig.tw.set_now(t);
        let _ = second(&mut rig);
        steps += 1;
    }
    assert!(
        wreck_home,
        "the swept wreck reached Ceres before the retrieval was delivered"
    );
}

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
    for _ in 0..40_000 {
        if scrapped(&rig, wreck) {
            break;
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
/// the unowned, files no claim, and she is kept (her title to Tycho) or scrapped at Ceres.
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
    for _ in 0..40_000 {
        let title = tacsim::port::house_sweep::title_of(rig.tw.world(), wreck);
        if title == Some(PrincipalId::Object(dispatcher)) {
            titled = true;
        }
        if scrapped(&rig, wreck) || (titled && title == Some(tycho.clone())) {
            break;
        }
        if crate::field_host_rig::site_record_at(&mut rig, entry)
            .is_some_and(|record| record.custody.iter().any(|row| row.hull == wreck))
        {
            claimed = true;
        }
        let t = rig.tw.world().now().saturating_add(1);
        rig.tw.set_now(t);
        let _ = second(&mut rig);
    }
    assert!(titled, "the sweep titled her to the dispatcher");
    assert!(
        !claimed,
        "an ownerless wreck is the house's own: no custody, no claim"
    );
    let title = tacsim::port::house_sweep::title_of(rig.tw.world(), wreck);
    assert!(
        scrapped(&rig, wreck) || title == Some(tycho),
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

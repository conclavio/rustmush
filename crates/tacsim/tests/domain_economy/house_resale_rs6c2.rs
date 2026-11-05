//! **Doc 41 cut RS.6c-2 — Tycho Mutual's sweep, the resale half** (story (e); Q-41-16; the
//! owner's defaults of 2026-11-04: the supply leg to Stickney, the sale in the buyout's shape),
//! over the composed lane at 256 (Ahuna Flank) and 512 (Urvara Reach):
//!
//! Occator's spare rig lies wrecked on the site's open ground, her chassis gone and nothing else
//! touched; nobody claims it and nobody posts its retrieval. After 56 IC days Tycho Mutual's house
//! sweeps it as RS.6c-1 does, and at Ceres she assesses under her resale line, so she is **kept**:
//! the lifter sails to Stickney for the restoration's structural alloys and yard fittings, comes
//! home, the dispatcher takes the workshop's order, the lifter waits docked until the work is
//! done, and her title passes to Tycho. `+salvage/forsale` lists her at her reference price; the
//! pilot buys her with `+salvage/buy <hull> go`, the price is paid into Tycho's account and her
//! title passes to the pilot at the next settlement.
//!
//! **What is a fixture.** The rig's chassis destroyed and its row on the site's index as a wreck,
//! by staff hand (GC.5's fixture); 57 IC days skipped; the pilot's money minted. Every act of
//! Tycho's and the pilot's is the production path.

use mush_core::{GOD, ObjId};
use mush_org::PrincipalId;

use crate::driving_rig::{
    Rig, cid, fire, land_quietly, on_pad_as_at, port_map, ramp_at, run, stocked_tuned, to_cell_on,
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

/// Tycho's `(principal, account)`.
fn tycho(rig: &Rig) -> (PrincipalId, mush_economy::AccountId) {
    let map = port_map(&rig.tw, "ceres_port");
    map.orgs
        .iter()
        .find(|(id, _)| id.as_str() == "tycho_mutual")
        .map(|(_, row)| {
            (
                PrincipalId::Organization(mush_org::OrgId(row.org)),
                mush_economy::AccountId(row.account.expect("Tycho's account")),
            )
        })
        .expect("Tycho")
}

/// An account's posted balance, minor.
fn balance(rig: &Rig, account: mush_economy::AccountId) -> i64 {
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

/// The orders taken on `hull`'s bay record.
fn orders(rig: &Rig, hull: ObjId) -> u32 {
    match crate::driving_rig::store(&rig.tw).bay_hull(hull) {
        Some(tacsim::port::bay_work::BayHullState::Present(row)) => row.orders,
        _ => 0,
    }
}

/// **The resale on `(venue, entry)`.**
#[allow(clippy::too_many_lines)]
fn story(venue: &str, entry: &str) {
    let mut rig = stocked_tuned(quick);
    let (pilot, _, tvs) = on_pad_as_at(&mut rig, ("Pilot", 1, "Salt Moth"), (venue, entry), 1);
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
    let (house, house_account) = tycho(&rig);
    let now = rig.tw.world().world_ic_now().at;
    skip_and_fire(
        &mut rig,
        now + (tacsim::port::house_sweep::SWEEP_IC_DAYS + 1) * 86_400,
    );
    // The sweep, the supply leg and the restoration, second by second (faster while the lifter
    // is under way or the work is open).
    let mut awarded = false;
    let mut kept = None;
    let mut supplied = false;
    for _ in 0..20_000 {
        let w = rig.tw.world();
        let title = tacsim::port::house_sweep::title_of(w, wreck);
        if title == Some(house.clone()) {
            break;
        }
        if title == Some(PrincipalId::Object(dispatcher)) {
            awarded = true;
            // Berthed at Ceres (a berth holds her): the arm's assessment.
            if kept.is_none() && tacsim::port::bay_work::held(w, wreck) {
                kept = Some(tacsim::port::house_sweep::assess(w, wreck));
            }
        }
        assert!(
            !tacsim::port::bay_work::scrapped(w, wreck),
            "she was scrapped: {kept:?}"
        );
        if let tacsim::port::ship::Berth::Docked { map, .. } =
            tacsim::port::ship::berth_of(w, lifter)
            && map.entry.as_str() == "stickney_port"
        {
            supplied = true;
        }
        let step = if matches!(
            tacsim::port::ship::berth_of(w, lifter),
            tacsim::port::ship::Berth::UnderWay
        ) {
            300
        } else if tacsim::port::bay_work::under_work(w, wreck) {
            60
        } else {
            1
        };
        let t = w.now().saturating_add(step);
        rig.tw.set_now(t);
        let _ = second(&mut rig);
    }
    if tacsim::port::house_sweep::title_of(rig.tw.world(), wreck) != Some(house.clone()) {
        // A stall: what the workshop says to the dispatcher, standing where she is berthed.
        let room = rig.tw.world().get(wreck).and_then(|o| o.location);
        let berth = format!("{:?}", tacsim::port::ship::berth_of(rig.tw.world(), lifter));
        if let Some(room) = room {
            let _ = rig.tw.world_mut().move_to(dispatcher, room);
        }
        let quote = run(&mut rig.tw, dispatcher, &format!("+bay/quote #{}", wreck.0));
        let order = run(
            &mut rig.tw,
            dispatcher,
            &format!("+bay/order #{}=chassis", wreck.0),
        );
        panic!(
            "stalled: kept {kept:?}, supplied {supplied}, orders {}, lifter {berth:?}\nquote: {quote}\norder: {order}",
            orders(&rig, wreck)
        );
    }
    assert!(awarded, "the claim was awarded to the dispatcher");
    assert!(
        matches!(
            kept,
            Some(tacsim::port::house_sweep::Disposition::Keep { .. })
        ),
        "assessed under her resale line: {kept:?}"
    );
    assert!(supplied, "the lifter ran the supply leg to Stickney");
    assert!(orders(&rig, wreck) >= 1, "the workshop restored her");
    assert!(
        !tacsim::port::tv::wrecked(rig.tw.world(), wreck),
        "restored, she is no wreck"
    );
    assert_eq!(
        tacsim::port::house_sweep::title_of(rig.tw.world(), wreck),
        Some(house.clone()),
        "her title passed to Tycho"
    );
    // For sale at her reference price.
    let listing = tacsim::port::house_sale::for_sale(rig.tw.world())
        .into_iter()
        .find(|l| l.hull == wreck)
        .expect("she is for sale");
    let listed = run(&mut rig.tw, pilot, "+salvage/forsale");
    assert!(listed.contains(&format!("#{}", wreck.0)), "{listed}");
    let preview = run(&mut rig.tw, pilot, &format!("+salvage/buy #{}", wreck.0));
    assert!(preview.contains("Add go"), "{preview}");
    // The pilot's money (a fixture), then the buy.
    let account = tacsim::port::credit::own_account(rig.tw.world(), pilot)
        .expect("the pilot's account")
        .id;
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@econ/mint {treasury}={} {}",
            account.0,
            listing.price_minor + 100_000
        ),
    );
    assert!(!out.contains("Usage"), "{out}");
    let before = balance(&rig, house_account);
    let filed = run(&mut rig.tw, pilot, &format!("+salvage/buy #{} go", wreck.0));
    assert!(filed.contains("Registry writes the sale"), "{filed}");
    let again = run(&mut rig.tw, pilot, &format!("+salvage/buy #{} go", wreck.0));
    assert!(!again.contains("Registry writes the sale"), "{again}");
    for _ in 0..600 {
        if tacsim::port::house_sweep::title_of(rig.tw.world(), wreck)
            == Some(PrincipalId::Object(pilot))
        {
            break;
        }
        let t = rig.tw.world().now().saturating_add(1);
        rig.tw.set_now(t);
        let _ = second(&mut rig);
    }
    assert_eq!(
        tacsim::port::house_sweep::title_of(rig.tw.world(), wreck),
        Some(PrincipalId::Object(pilot)),
        "her title passed to the pilot"
    );
    assert_eq!(
        balance(&rig, house_account) - before,
        i64::try_from(listing.price_minor).expect("price"),
        "the price was paid into Tycho's account"
    );
    assert!(
        tacsim::port::house_sale::for_sale(rig.tw.world())
            .iter()
            .all(|l| l.hull != wreck),
        "sold, she is off the list"
    );
}

/// **RS.6c-2's gate on Ahuna Flank (256).**
#[test]
fn rs6c2_tycho_restores_and_sells_a_wreck_from_ahuna_flank() {
    story("ahuna", "ahuna_port");
}

/// **RS.6c-2's gate on Urvara Reach (512).**
#[test]
fn rs6c2_tycho_restores_and_sells_a_wreck_from_urvara_reach() {
    story("urvara", "urvara_port");
}

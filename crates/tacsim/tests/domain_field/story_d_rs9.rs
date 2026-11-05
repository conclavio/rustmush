//! **Doc 41 story (d) — the carrier's hull loss, retrieved** (§4.9; cut RS.9a), over the composed
//! lane at 256 (Ahuna Flank) and 512 (Urvara Reach):
//!
//! A Co-op Dray-P on her pad loses her hull (GC.3's loss): she is a wreck that never lifts, and
//! the two rigs in her bay are wrecks with her. Wrecked while docked, she is set down on her pad
//! cell (the owner's ruling of 2026-11-05). No shipped tug can take her (the floor line;
//! `field_carrier::every_carrier_is_past_twice_any_tugs_tonnage` is the table's check), so her
//! retrieval goes unaccepted to Tycho Mutual. The house's 130 t
//! crawler tows her, laden with her two wrecked rigs, into the 600 t lifter; the lifter berths her
//! in Ceres's workshop (C14), the delivery pays the fee, and `+bay` quotes her rebuild.
//!
//! **What is a fixture.** The hull's loss by staff hand (GC.3's table read off a section at its
//! floor; the fought loss is GC.3's own story), the carrier and her rigs on the site's index as
//! wrecks (GC.5's fixture), and IC skipped past the job's acceptance window. Every act of the
//! house's and the players' is the production path.

use mush_core::ObjId;
use tacsim::surface::retrieval::{RetrievalStage, order_of};

use crate::driving_rig::{Rig, cid, fire, land_quietly, staff_hand, stocked_tuned};
use crate::field_host_rig::{second, set_section, tune};
use crate::site_record_rig::{staff, staff_member};

/// Short tow stages.
fn quick(cfg: &mut tacsim::TacsimConfig) {
    tune(cfg);
    cfg.field.idle_demote_s = 3_600;
    cfg.recovery.inspect_s = 30;
    cfg.recovery.attach_s = 30;
    cfg.recovery.load_s = 60;
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

/// **Story (d) on `(venue, entry)`.**
#[allow(clippy::too_many_lines)]
fn story(venue: &str, entry: &str) {
    let mut rig = stocked_tuned(quick);
    // She flies out with the propellant the leg takes and little more: her laden mass is her dry
    // hull and her two rigs (doc 41 §4.9's 256 t), under the crawler's twice-its-own 260 t.
    let (_pilot, row, tvs) = crate::driving_rig::on_pad_loaded_at(
        &mut rig,
        ("Pilot", 1, "Salt Moth"),
        (venue, entry),
        |rig, pilot| {
            for _ in 0..2 {
                crate::driving_rig::lease_one(rig, pilot);
            }
        },
        (5, 4),
    );
    assert_eq!(tvs.len(), 2);
    let carrier = ObjId(row.hull);
    // The hull's loss: the Dray-P's hull at its floor, and both rigs in her bay wrecked with her.
    set_section(
        &mut rig,
        carrier,
        "hull",
        Some(tacsim::arena::damage::WRECK_CONDITION),
        Some(0),
    );
    for tv in &tvs {
        set_section(&mut rig, *tv, "chassis", Some("destroyed"), Some(0));
    }
    assert!(
        mush_tactics::module::unit_of(rig.tw.world(), carrier)
            .is_some_and(tacsim::arena::damage::is_wreck),
        "the carrier is a wreck"
    );
    // Wrecked while docked, she is set down on her pad cell (the field's close does this for a
    // fought loss; the owner's ruling of 2026-11-05).
    let pose = staff_hand(&mut rig, |ctx| {
        tacsim::surface::tow::set_down_on_pad(ctx, carrier)
    })
    .expect("set down on her pad");
    let at = (pose.x_cm.div_euclid(1_000), pose.y_cm.div_euclid(1_000));
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), carrier),
            tacsim::port::ship::Berth::Docked { .. }
        ),
        "set down on her pad, she is still docked there"
    );
    let s = staff_member(&mut rig);
    staff(
        &mut rig,
        s,
        &format!(
            "@tacsim/site {entry} leave #{} wreck {},{}",
            carrier.0, at.0, at.1
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
    // Her title holder posts the job; nobody accepts it; at the window's end it is Tycho's.
    let owner = tacsim::port::house_sweep::title_of(rig.tw.world(), carrier).expect("a title");
    let account = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .accounts_of(&owner)
                .find(|a| a.instrument.as_str() == tacsim::CREDIT)
                .map(|a| a.id)
        })
        .expect("the title holder's account");
    let posted = staff_hand(&mut rig, |ctx| {
        tacsim::surface::retrieval::post(
            ctx,
            carrier,
            (owner.clone(), account),
            ceres.clone(),
            None,
        )
    });
    assert!(posted.is_ok(), "{posted:?}");
    let job = order_of(rig.tw.world(), carrier).expect("the job");
    skip_and_fire(&mut rig, job.accept_by_ic + 60);
    assert_eq!(
        order_of(rig.tw.world(), carrier).map(|o| o.stage),
        Some(RetrievalStage::House)
    );
    let mut steps = 0u32;
    loop {
        if order_of(rig.tw.world(), carrier).is_some_and(|o| o.delivered.is_some()) {
            break;
        }
        if steps == 8_000 {
            let act = staff_hand(&mut rig, |ctx| {
                tacsim::port::house_arm::next_act(ctx, (lifter, crawler), carrier, &ceres)
            });
            panic!(
                "not delivered in 8,000 steps: next {act:?}; the carrier {:?}; the crawler {:?}; \
                 the lifter {:?}",
                tacsim::surface::act::where_is(rig.tw.world(), carrier),
                tacsim::surface::act::where_is(rig.tw.world(), crawler),
                match tacsim::port::ship::berth_of(rig.tw.world(), lifter) {
                    tacsim::port::ship::Berth::Docked { map, .. } => map.entry.to_string(),
                    other => format!("{other:?}"),
                },
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
    let floor = crate::driving_rig::station_room(&rig.tw, "ceres_port", "room.c14_dockmaster");
    assert_eq!(
        rig.tw.world().get(carrier).and_then(|o| o.location),
        Some(floor),
        "berthed in C14"
    );
    // The delivery pays: the job closes on its settlement.
    for step in 0..600 {
        // She stays berthed: the simulation does not take a carrier the workshop holds.
        assert_eq!(
            rig.tw.world().get(carrier).and_then(|o| o.location),
            Some(floor),
            "she left the workshop at step {step}"
        );
        if order_of(rig.tw.world(), carrier).is_some_and(|o| o.stage == RetrievalStage::Closed) {
            break;
        }
        let t = rig.tw.world().now().saturating_add(1);
        rig.tw.set_now(t);
        let _ = second(&mut rig);
    }
    assert_eq!(
        order_of(rig.tw.world(), carrier).map(|o| o.stage),
        Some(RetrievalStage::Closed),
        "the delivery settled and paid Tycho"
    );
    // And the workshop quotes her rebuild.
    let quote = crate::driving_rig::run(
        &mut rig.tw,
        mush_core::GOD,
        &format!("+bay/quote #{}", carrier.0),
    );
    assert!(quote.contains("rebuild"), "{quote}");
}

/// **Story (d) on Urvara Reach (512).**
#[test]
fn rs9_story_d_the_carrier_wreck_is_retrieved_from_urvara_reach() {
    story("urvara", "urvara_port");
}

/// **Story (d) on Ahuna Flank (256).**
#[test]
fn rs9_story_d_the_carrier_wreck_is_retrieved_from_ahuna_flank() {
    story("ahuna", "ahuna_port");
}

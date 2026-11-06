//! **Doc 41 story (d) — the carrier's hull loss, retrieved** (§4.9; cut RS.9a), over the composed
//! lane at 256 (Ahuna Flank) and 512 (Urvara Reach):
//!
//! A Co-op Dray-P on her pad loses her hull (GC.3's loss): she is a wreck that never lifts, and
//! the two rigs in her bay are wrecks with her. Wrecked while docked, she is set down on her pad
//! cell and indexed there by the field host's tick (the owner's ruling of 2026-11-05; no field
//! runs, so the close does not). No shipped tug can take her (the floor line;
//! `field_carrier::every_carrier_is_past_twice_any_tugs_tonnage` is the table's check), so her
//! pilot's `+salvage/retrieve` goes unaccepted to Tycho Mutual. The house's 130 t
//! crawler tows her, laden with her two wrecked rigs, into the 600 t lifter; the lifter berths her
//! in Ceres's workshop (C14) with the two rigs still aboard, the delivery pays Tycho the escrowed
//! fee, and `+bay/quote` prices her hull's rebuild.
//!
//! **What is a fixture.** The hull's loss by staff hand (doc 36a §8's damage seam: GC.3's table
//! read off a section at its floor; the fought loss is GC.3's own story), and IC skipped past the
//! job's acceptance window. The set-down, the bay rigs' wreck and the index are the tick's; every
//! act of the house's and the pilot's is the production path.

use mush_core::ObjId;
use tacsim::surface::retrieval::{RetrievalStage, order_of};

use crate::driving_rig::{Rig, cid, fire, land_quietly, run, staff_hand, stocked_tuned};
use crate::field_host_rig::{second, set_section, tune};

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
    let (pilot, row, tvs) = crate::driving_rig::on_pad_loaded_at(
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
    // The hull's loss: the Dray-P's hull at its floor (the damage seam). Nothing else is staff's.
    set_section(
        &mut rig,
        carrier,
        "hull",
        Some(tacsim::arena::damage::WRECK_CONDITION),
        Some(0),
    );
    assert!(
        mush_tactics::module::unit_of(rig.tw.world(), carrier)
            .is_some_and(tacsim::arena::damage::is_wreck),
        "the carrier is a wreck"
    );
    // Wrecked while docked, outside a field: the field host's tick sets her down on her pad cell,
    // wrecks her bay rigs with her and indexes her there (the owner's ruling of 2026-11-05).
    let mut indexed = None;
    for _ in 0..30 {
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
        indexed = crate::field_host_rig::site_record_at(&mut rig, entry).and_then(|record| {
            record
                .units
                .iter()
                .find(|row| row.unit == carrier)
                .map(|row| (row.condition, row.pose.cell()))
        });
        if indexed.is_some() {
            break;
        }
    }
    let (condition, at) = indexed.expect("the tick indexes her on the site");
    assert_eq!(
        condition,
        tacsim::surface::site_record::LeftCondition::Wreck
    );
    assert_eq!(
        at,
        crate::custody_rig::ramp_cell_on(&rig, entry),
        "indexed on her ramp's cell"
    );
    let (on, pose) = mush_sim::pose_of(rig.tw.world(), carrier).expect("placed");
    assert_eq!(
        on,
        crate::driving_rig::site_domain(&rig.tw, entry),
        "on the site's ground"
    );
    assert_eq!(
        (pose.x_cm.div_euclid(1_000), pose.y_cm.div_euclid(1_000)),
        at,
        "standing where she is indexed"
    );
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), carrier),
            tacsim::port::ship::Berth::Docked { .. }
        ),
        "set down on her pad, she is still docked there"
    );
    for tv in &tvs {
        assert!(
            mush_tactics::module::unit_of(rig.tw.world(), *tv)
                .is_some_and(tacsim::arena::damage::is_wreck),
            "her bay rig #{} is a wreck with her",
            tv.0
        );
    }
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
    // Her pilot posts the job: the Trust holds her title under the charter, and her live tenant
    // answers for her (the lead's ruling of 2026-11-06), posting in their own name, from their
    // own account; a stranger may not. The tenant is told of claims on her. Nobody accepts it; at
    // the window's end it is Tycho's.
    let hull = format!("#{}", carrier.0);
    let stranger = crate::driving_rig::pilot(&mut rig, "Stranger", 7);
    let refused = run(&mut rig.tw, stranger, &format!("+salvage/retrieve {hull}"));
    assert!(
        refused.contains(tacsim::surface::retrieval::NOT_YOUR_HULL),
        "{refused}"
    );
    let owner = tacsim::port::house_sweep::title_of(rig.tw.world(), carrier).expect("a title");
    assert!(
        matches!(owner, mush_org::PrincipalId::Organization(_)),
        "the Trust holds her title: {owner:?}"
    );
    assert!(
        tacsim::surface::claim::answerers_of(rig.tw.world(), carrier, &owner).contains(&pilot),
        "her live charter's tenant answers for her (and is told of claims)"
    );
    let own = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .accounts_of(&mush_org::PrincipalId::Object(pilot))
                .find(|a| a.instrument.as_str() == tacsim::CREDIT)
                .map(|a| a.id.0)
        })
        .expect("the pilot's own account");
    let (posted_before, available_before) = (balance(&rig, own), available(&rig, own));
    let preview = run(&mut rig.tw, pilot, &format!("+salvage/retrieve {hull}"));
    assert!(preview.contains("Post a retrieval"), "{preview}");
    let posted = run(&mut rig.tw, pilot, &format!("+salvage/retrieve {hull} go"));
    assert!(posted.contains("posted for"), "{posted}");
    let mut job = None;
    for _ in 0..30 {
        if job.is_some() {
            break;
        }
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
        job = order_of(rig.tw.world(), carrier);
    }
    let job = job.expect("the job");
    assert!(job.fee_minor > 0, "a fee: {job:?}");
    assert_eq!(
        job.poster,
        mush_org::PrincipalId::Object(pilot),
        "the tenant posts in their own name"
    );
    assert_eq!(job.poster_account, own, "from their own account");
    assert_eq!(
        available_before - available(&rig, own),
        job.fee_minor,
        "the fee is held off the tenant's account at the post"
    );
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
    // Her two wrecked rigs came home aboard her.
    let aboard: Vec<ObjId> = tacsim::port::ship::hull_view(rig.tw.world(), carrier)
        .map(|view| {
            tacsim::port::vehicle::rigs_in_bay(rig.tw.world(), &view)
                .into_iter()
                .map(|r| r.tv)
                .collect()
        })
        .unwrap_or_default();
    for tv in &tvs {
        assert!(
            aboard.contains(tv),
            "her rig #{} is aboard her: {aboard:?}",
            tv.0
        );
    }
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
        "the delivery settled"
    );
    // The escrowed fee is paid out of the tenant's account at the close: the settled contract's one
    // consideration (the account also pays the charter's own dues across the window).
    let _ = posted_before;
    let contract = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .contract(mush_economy::ContractId(job.contract))
                .map(|c| c.state)
        });
    assert!(
        matches!(contract, Some(mush_economy::ContractState::Settled { .. })),
        "the job's contract settled to the house that took it: {contract:?}"
    );
    let paid_by = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .contract(mush_economy::ContractId(job.contract))
                .map(|c| {
                    c.consent
                        .terms
                        .consideration
                        .iter()
                        .map(|f| (f.from.clone(), f.account.0, f.amount.minor()))
                        .collect::<Vec<_>>()
                })
        })
        .unwrap_or_default();
    assert_eq!(
        paid_by,
        vec![(mush_org::PrincipalId::Object(pilot), own, job.fee_minor)],
        "the tenant paid the fee from their own account"
    );
    // And the workshop quotes her hull's rebuild, priced.
    let quote = run(&mut rig.tw, pilot, &format!("+bay/quote {hull}"));
    let rebuild = quote
        .lines()
        .find(|line| line.contains("hull") && line.contains("rebuild"))
        .unwrap_or_else(|| panic!("no rebuild of her hull in the quote:\n{quote}"));
    assert!(
        rebuild.contains("cr"),
        "the rebuild is priced: {rebuild}\n{quote}"
    );
}

/// A credit account's posted balance, minor units.
fn balance(rig: &Rig, account: u64) -> i64 {
    rig.tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .account(mush_economy::AccountId(account))
                .map(|a| a.posted_balance.minor())
        })
        .expect("the account")
}

/// A credit account's available balance, minor units (its holds deducted).
fn available(rig: &Rig, account: u64) -> i64 {
    rig.tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .account(mush_economy::AccountId(account))
                .and_then(|a| a.available())
                .map(|a| a.minor())
        })
        .expect("the account")
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

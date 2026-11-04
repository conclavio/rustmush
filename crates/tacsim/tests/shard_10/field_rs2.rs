//! **Doc 41 cut RS.2 — the bay away from port (story a, the bay half)** (Q-41-5..7, Q-38-16,
//! Q-36-35), over the composed world (`driving`'s keyed rig), the arena's egress pumped by hand
//! as `field_rs1.rs` pumps it, on both grounds of `grounds_rig` (Ahuna Flank at 256, Urvara Reach
//! at 512).
//!
//! The captain leases Occator's escort and a prospector into the Dray-P and sails to the site.
//! On the pad the escort, still in her bay, has a damaged cannon, a hull down 40 points and a
//! destroyed machine gun.
//!
//! 1. `+bay` quotes her in the carrier's bay away from port: the mend and the armour at twice the
//!    workshop's time, no fee, and the machine gun's line naming the workshops, Ceres Freeport
//!    (one hop) before Stickney Yards (two).
//! 2. `+bay/order …=replace` is refused with that line (Q-36-35; IN.6's store does not reach a
//!    carrier).
//! 3. `+bay/order …=mend,armor` is taken with no fee contract (Q-41-7), drawing the bale's tonnes
//!    from her own hold, due at twice the tariff's time.
//! 4. A stranger's mark opens a field over the prospector on the site: the order is paused, its
//!    time left kept, and resumes when the field settles (Q-41-5); no order is taken meanwhile.
//! 5. The Dray-P sails for Ceres, the prospector left on the ground, and the order finishes under
//!    way: the cannon mended, the hull whole, the tonnes gone from her hold, nobody paid.
//!
//! **What is a fixture.** The escort's damage, by staff hand, standing in for a fight (GC.3's
//! precedent); the hold's goods, seeded; the hound the stranger sits in. Every act after is the
//! production command path.

use mush_core::ObjId;
use tacsim::arena::ledger::LedgerEvent;
use tacsim::port::bay_work::{BayHullState, BayOrder, FacilityClass, OrderStatus};

use crate::driving_rig::{
    Rig, act_go, command, file_and_launch, finish, hold_pool, hold_vault_of, land, on_pad_as_at,
    on_pad_with_at, ramp_at, run, seed_hold, staff_hand, stocked_tuned, store, to_cell_on, to_ic,
    warm,
};
use crate::field_host_rig::{
    armed_on, egress, seat, second, set_section, site_record_at, task, tune, until,
};
use crate::grounds_rig::{AHUNA_FLANK, Ground, URVARA_REACH};

/// Staff money for the two leases' deposits (`bay_gc8`'s fixture).
fn mint(rig: &mut Rig, pilot: ObjId) {
    let treasury = crate::driving_rig::port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let account = tacsim::port::credit::own_account(rig.tw.world(), pilot)
        .map(|a| a.id.0)
        .expect("the pilot's account");
    let _ = run(
        &mut rig.tw,
        mush_core::GOD,
        &format!("@econ/mint {treasury}={account} 10000000"),
    );
}

/// `hull`'s open or last bay order.
fn order_of(rig: &Rig, hull: ObjId) -> Option<BayOrder> {
    match store(&rig.tw).bay_hull(hull) {
        Some(BayHullState::Present(row)) => row.order.clone(),
        _ => None,
    }
}

/// One part's condition on `unit`, by its local id.
fn part(rig: &Rig, unit: ObjId, local: &str) -> String {
    let cells = mush_tactics::module::unit_of(rig.tw.world(), unit).expect("the unit");
    cells
        .components()
        .find(|(id, _)| id.as_str().rsplit('.').next() == Some(local))
        .map(|(_, c)| c.condition.as_str().to_owned())
        .unwrap_or_else(|| panic!("no {local} on #{}", unit.0))
}

/// Staff set one part's condition on `unit`.
fn condition(rig: &mut Rig, unit: ObjId, local: &str, condition: &str) {
    let component = mush_tactics::module::unit_of(rig.tw.world(), unit)
        .and_then(|cells| {
            cells
                .components()
                .find(|(id, _)| id.as_str().rsplit('.').next() == Some(local))
                .map(|(id, _)| id.clone())
        })
        .unwrap_or_else(|| panic!("no {local} on #{}", unit.0));
    staff_hand(rig, |ctx| {
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::SetComponentCondition {
                component,
                condition: mush_tactics::ConditionId::new(condition).expect("a condition"),
            }],
            None,
            Some(smol_str::SmolStr::new_static("test:rs2-damage")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// The escort's hull internals.
fn internals(rig: &Rig, unit: ObjId) -> (i64, i64) {
    let addr = mush_tactics::CellAddr::Section {
        section: mush_tactics::SectionId::new("hull").expect("an id"),
        cell: mush_tactics::CellId::new(tacsim::definitions::unit::INTERNALS_CELL).expect("an id"),
    };
    mush_tactics::module::unit_of(rig.tw.world(), unit)
        .and_then(|cells| cells.cell(&addr).map(|c| (c.current, c.max)))
        .expect("the hull's internals")
}

/// **RS.2's story on `ground`.**
#[allow(clippy::too_many_lines)] // one story, in order
async fn bay_on(ground: &Ground) {
    let mut rig = stocked_tuned(|cfg| {
        tune(cfg);
        cfg.field.idle_demote_s = 15;
    });
    // The captain's party: the escort (slot 1) and a prospector (slot 2) in the Dray-P's bay.
    let (a, row, tvs) = on_pad_with_at(
        &mut rig,
        ("Pilot", 1, "Salt Moth"),
        ground.site(),
        |rig, pilot| {
            mint(rig, pilot);
            crate::driving_rig::at_counter(rig, pilot);
            let escort = run(&mut rig.tw, pilot, "+tv/lease escort go");
            assert!(
                escort.contains("Occator Water & Propellant leases you "),
                "{escort}"
            );
            crate::driving_rig::lease_one(rig, pilot);
        },
    );
    let (escort, prospect) = (tvs[0], tvs[1]);
    let hull = ObjId(row.hull);
    let vault = hold_vault_of(&rig.tw, &row);
    seed_hold(&mut rig, &row, "structural_alloys", 2, 9_601);
    seed_hold(&mut rig, &row, "industrial_parts", 2, 9_602);

    // The fixture: the cannon damaged, the hull down 40 points, the machine gun destroyed.
    condition(&mut rig, escort, "cannon", "damaged");
    condition(&mut rig, escort, "machine_gun", "destroyed");
    let (full, _) = internals(&rig, escort);
    set_section(&mut rig, escort, "hull", Some("damaged"), Some(full - 40));

    // (1) The quote in her bay, away from port: the minor work at 2x, no fee, and the machine
    // gun's line naming the workshops nearest first by the lane.
    let quote = run(&mut rig.tw, a, &format!("+bay #{}", escort.0));
    assert!(
        quote.contains("stands in a carrier bay away from port") && quote.contains("no fee"),
        "{quote}"
    );
    assert!(
        quote.contains("mend ") && quote.contains("armor "),
        "{quote}"
    );
    let line = quote
        .lines()
        .find(|l| l.contains("machine_gun is destroyed: a workshop must replace it — "))
        .unwrap_or_else(|| panic!("no workshop line: {quote}"))
        .trim()
        .to_owned();
    let (ceres, stickney) = (
        line.find("Ceres Freeport").expect("Ceres named"),
        line.find("Stickney Yards").expect("Stickney named"),
    );
    assert!(ceres < stickney, "nearest first: {line}");

    // (2) The replace refused with that line.
    let refused = run(&mut rig.tw, a, &format!("+bay/order #{}=replace", escort.0));
    assert!(refused.contains(&line), "{refused}");
    assert!(order_of(&rig, escort).is_none(), "nothing was taken");

    // (3) The mend and the armour, no fee, from her hold, at twice the time.
    let taken = run(
        &mut rig.tw,
        a,
        &format!("+bay/order #{}=mend,armor", escort.0),
    );
    assert!(
        taken.contains("Order 1 taken in the carrier bay")
            && taken.contains("No fee away from port"),
        "{taken}"
    );
    let order = order_of(&rig, escort).expect("the order");
    assert_eq!(
        (order.status, order.class, order.contract, order.fee_minor),
        (OrderStatus::Open, FacilityClass::CarrierBay, None, 0),
        "no fee contract away from port (Q-41-7)"
    );
    assert_eq!(
        order.munitions_vault,
        Some(vault.0),
        "drawn from her own hold"
    );
    let workshop_secs: i64 = order.jobs.iter().map(|j| j.secs).sum();
    assert_eq!(
        order.due_ic - order.started_ic,
        tacsim::port::bay_work::CARRIER_TIME_FACTOR * workshop_secs,
        "twice the workshop's time"
    );
    let drawn = |good: &str| {
        order
            .goods
            .iter()
            .find(|g| g.good.as_str() == good)
            .map_or(0, |g| g.units)
    };
    assert_eq!(
        (drawn("structural_alloys"), drawn("industrial_parts")),
        (1, 1),
        "the bale opens with a tonne of each from her hold"
    );

    // (4) A field over the prospector on the site: the order waits under it.
    let _ = act_go(&mut rig, a, "+tv/2 launch go");
    let _ = finish(&mut rig, prospect, a);
    let (rx, ry, _) = ramp_at(&rig.tw, ground.entry);
    let ramp = (rx.div_euclid(1_000), ry.div_euclid(1_000));
    to_cell_on(&mut rig, ground.entry, prospect, (ramp.0 + 4, ramp.1));
    let (b, _, _) = on_pad_as_at(&mut rig, ("Second", 2, "Red Moth"), ground.site(), 0);
    let hound = armed_on(
        &mut rig,
        ground.entry,
        "Red Hound",
        (ramp.0 - 8, ramp.1),
        90_000,
    );
    seat(&mut rig, hound, b);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    let (out, queued) = command(&mut rig, b, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| task(rig)
            .is_some())
        .await,
        "the field opens"
    );
    assert!(
        until(&mut rig, &mut rx, &mut lines, 30, |rig, _| order_of(
            rig, escort
        )
        .is_some_and(|o| o.paused.is_some()))
        .await,
        "the field pauses the bay's work"
    );
    let left = order_of(&rig, escort)
        .and_then(|o| o.paused)
        .expect("the time left");
    assert!(left > 0, "the time left is kept");
    let shown = run(&mut rig.tw, a, &format!("+bay #{}", escort.0));
    assert!(shown.contains("waiting under a field"), "{shown}");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 180, |rig, _| task(rig)
            .is_none()
            && site_record_at(rig, ground.entry)
                .is_some_and(|record| record.open_field().is_none()))
        .await,
        "the field settles"
    );
    for _ in 0..12 {
        if order_of(&rig, escort).is_some_and(|o| o.paused.is_none()) {
            break;
        }
        let _ = second(&mut rig);
    }
    let resumed = order_of(&rig, escort).expect("the order survives the field");
    assert_eq!(resumed.status, OrderStatus::Open, "{resumed:?}");
    assert!(resumed.paused.is_none(), "the work resumes on open ground");
    assert!(
        resumed.due_ic - crate::driving_rig::ic_now(&rig.tw) <= left
            && resumed.due_ic > order.due_ic,
        "due its time left after the resume, later than first quoted"
    );

    // (5) The crew works on the site until half an hour before the work is due (the hop to
    // Ceres is shorter than the work), then she sails, the prospector left on the ground; the
    // order finishes under way.
    let _ = to_ic(&mut rig, resumed.due_ic - 1_800);
    assert_eq!(
        order_of(&rig, escort).map(|o| o.status),
        Some(OrderStatus::Open),
        "still working on the site"
    );
    let t = file_and_launch(&mut rig, a, hull, "+ship/undock ceres leave=2");
    assert!(
        resumed.due_ic > t.plan.depart_ic && resumed.due_ic < t.plan.depart_ic + t.plan.total_s,
        "the work falls inside the passage: due {} departing {} for {} s",
        resumed.due_ic,
        t.plan.depart_ic,
        t.plan.total_s
    );
    assert!(matches!(
        tacsim::port::ship::berth_of(rig.tw.world(), hull),
        tacsim::port::ship::Berth::UnderWay
    ));
    for at in [
        resumed.due_ic + 60,
        resumed.due_ic + 120,
        resumed.due_ic + 600,
    ] {
        let _ = to_ic(&mut rig, at);
        if order_of(&rig, escort).is_some_and(|o| o.status != OrderStatus::Open) {
            break;
        }
    }
    let done = order_of(&rig, escort).expect("the order");
    assert_eq!(done.status, OrderStatus::Done, "{done:?}");
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::UnderWay
        ),
        "the work finished under way"
    );
    assert_eq!(part(&rig, escort, "cannon"), "intact", "the cannon mended");
    assert_eq!(internals(&rig, escort).0, full, "the hull whole");
    assert_eq!(
        (
            hold_pool(&mut rig.tw, vault, "structural_alloys"),
            hold_pool(&mut rig.tw, vault, "industrial_parts")
        ),
        (1, 1),
        "a tonne of each from her hold into the bale"
    );
    assert_eq!(done.contract, None, "nobody was paid");
}

/// **RS.2 at 256**: Ahuna Flank.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rs2_ahuna_the_bay_mends_on_the_site_waits_under_a_field_and_finishes_under_way() {
    bay_on(&AHUNA_FLANK).await;
}

/// **RS.2 at 512**: Urvara Reach.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rs2_urvara_the_bay_mends_on_the_site_waits_under_a_field_and_finishes_under_way() {
    bay_on(&URVARA_REACH).await;
}

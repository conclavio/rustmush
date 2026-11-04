//! **Doc 39 cut IN.6 — the part lot at the workshop** (Q-39-11, Q-39-12), over the composed
//! world (`driving`'s keyed rig), through the production command paths.
//!
//! The story (doc 39 §6, the IN.6 row; GC.8's refit run at Ceres): a First Captain leases
//! Occator's escort into their Dray-P and brings it to the Dockmaster's yard (C14, Occator's
//! workshop, at 1.2x) with its cannon destroyed.
//!
//! 1. The clerk has seeded Occator's stores: four `cannon_105` lots, each with a `Seeded` row,
//!    listed one by one as lot asks.
//! 2. `+bay` quotes the replace at 480 of labour and the part at its catalogue 28,000, from the
//!    workshop's stores.
//! 3. `+bay/order …=replace` escrows 28,480: the order takes the lowest-grade clean lot from the
//!    stores, its lot ask withdrawn and the lot held for the order until the draw.
//! 4. The work done, the cannon is whole, the lot is consumed, its row reads `Seeded` with the
//!    escort it went into, and the stores count one less: four, then three.
//! 5. Three more refits empty the stores; the fifth replace refuses `NO_PART`, at the quote and
//!    at the order.
//!
//! **Seeds** (doc 43 rule 2.3 #4; doc 36a §8): the destroyed cannon is staff damage through the
//! unit's one plan path (GC.8's fixture); the captain's money is staff's `@econ/mint`. Every claim
//! is reached by the player's command.
//!
//! **No site**: the cut is port-side, so it has no 256/512 pair (IN.2's and IN.4's precedent).

use mush_core::{GOD, ObjId};
use mush_inventory::LotId;
use mush_inventory::record::EndpointRef;
use mush_tactics::field_ops::ComponentCondition;
use mush_tactics::{UnitMutationOp, UnitMutationPlan};
use tacsim::port::bay_work::{BayHullState, OrderStatus};

use crate::driving_rig::{
    Rig, at_counter, cid, drain, fire, land_all, land_quietly, port_map, prospector, rigs_of, run,
    staff_hand, station_room, stocked, store, typed,
};

const CERES: &str = "ceres_port";
const C14: &str = "room.c14_dockmaster";
const CANNON: &str = "tacsim.part.weapon.cannon_105";

fn unit(rig: &Rig, hull: ObjId) -> mush_tactics::TacticalUnit {
    mush_tactics::module::unit_of(rig.tw.world(), hull)
        .cloned()
        .expect("a unit")
}

fn component_id(u: &mush_tactics::TacticalUnit, local: &str) -> mush_tactics::ComponentId {
    u.components()
        .map(|(id, _)| id.clone())
        .find(|id| id.as_str().ends_with(&format!(".{local}")) || id.as_str() == local)
        .unwrap_or_else(|| panic!("a {local} part"))
}

fn mint(rig: &mut Rig, pilot: ObjId) {
    let treasury = port_map(&rig.tw, CERES).treasury.expect("a treasury");
    let account = tacsim::port::credit::own_account(rig.tw.world(), pilot)
        .map(|a| a.id.0)
        .expect("the pilot's account");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={account} 20000000"),
    );
}

fn order(rig: &Rig, hull: ObjId) -> tacsim::port::bay_work::BayOrder {
    match store(&rig.tw).bay_hull(hull) {
        Some(BayHullState::Present(row)) => row.order.clone().expect("an order"),
        other => panic!("the bay record: {other:?}"),
    }
}

fn handles(rig: &Rig) -> tacsim::port::bay_work::act::BayActHandles {
    rig.tw
        .world()
        .module_state::<tacsim::port::bay_work::act::BayActHandles>()
        .cloned()
        .expect("the bay's handles")
}

/// Occator's storefront vault: the workshop's stores (Q-39-11).
fn stores(rig: &Rig) -> u64 {
    port_map(&rig.tw, CERES).vendors[&cid("occator")].vault
}

/// The seeded cannon lots, in seed order.
fn seeded(rig: &Rig) -> Vec<LotId> {
    let row = &port_map(&rig.tw, CERES).vendors[&cid("occator")];
    row.stock
        .iter()
        .filter(|s| s.good.as_str() == CANNON)
        .flat_map(|s| {
            let id = tacsim::port::families::StockIssueId::parse(&s.id).expect("an issue id");
            (0..s.qty).map(move |n| tacsim::port::stock::seed_lot_id(&id, n))
        })
        .collect()
}

/// **The stores' count of cannons**: the seeded lots still at the storefront.
fn in_stores(rig: &mut Rig) -> usize {
    let inventory = handles(rig).inventory.expect("the inventory");
    let storefront = EndpointRef(mush_economy::VaultId(stores(rig)));
    seeded(rig)
        .into_iter()
        .filter(|lot| {
            typed(&mut rig.tw, &inventory.lots, lot).is_some_and(|l| l.endpoint() == storefront)
        })
        .count()
}

fn provenance(rig: &mut Rig, lot: LotId) -> tacsim::port::bay_work::act::PartProvenance {
    let parts = handles(rig).parts;
    typed(&mut rig.tw, &parts, &lot).expect("the part's provenance")
}

fn listed(rig: &Rig, lot: LotId) -> bool {
    rig.tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .is_some_and(|m| m.lot_listings().any(|l| l.source.exact_lot() == Some(lot)))
}

/// Staff destroy the escort's cannon (GC.8's fixture).
fn wreck_cannon(rig: &mut Rig, escort: ObjId, cannon: &mush_tactics::ComponentId) {
    let u = unit(rig, escort);
    let ops = vec![UnitMutationOp::SetComponentCondition {
        component: cannon.clone(),
        condition: ComponentCondition::Destroyed
            .to_condition_id()
            .expect("destroyed"),
    }];
    let plan = UnitMutationPlan::new(ops, Some(u.revision()), None).expect("a plan");
    staff_hand(rig, |ctx| {
        mush_tactics::module::apply_plan(ctx, escort, &plan).expect("the damage lands");
    });
}

/// `+bay/order …=replace`, then fires until the order is done.
fn refit(rig: &mut Rig, pilot: ObjId, escort: ObjId) -> tacsim::port::bay_work::BayOrder {
    let taken = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/order #{}=replace", escort.0),
    );
    assert!(taken.contains("taken in the workshop"), "{taken}");
    let queued = drain(&mut rig.tw);
    land_all(rig, &queued);
    let due = order(rig, escort).due_ic;
    for _ in 0..12 {
        if order(rig, escort).status == OrderStatus::Done {
            return order(rig, escort);
        }
        let now = rig.tw.world().world_ic_now().at;
        if now < due + 60 {
            let _ = crate::driving_rig::to_ic(rig, due + 60);
        } else {
            let (_, queued) = fire(rig);
            land_quietly(rig, &queued);
        }
    }
    panic!("the refit did not finish: {:?}", order(rig, escort));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in6_a_refit_at_ceres_takes_the_cannon_from_occators_stores() {
    let mut rig = stocked();
    let (pilot, _row) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    mint(&mut rig, pilot);
    at_counter(&mut rig, pilot);
    let signed = run(&mut rig.tw, pilot, "+tv/lease escort go");
    assert!(signed.contains("leases you OWP-ES-01"), "{signed}");
    let escort = rigs_of(&rig.tw, pilot)[0];

    // (1) The stores: four seeded cannons, each its own lot with a `Seeded` row; the clerk lists
    // each as its own lot ask once its other work is done (the first is enough here).
    let lots = seeded(&rig);
    assert_eq!(lots.len(), 4, "Occator seeds four cannons (Q-39-12)");
    for _ in 0..60 {
        if listed(&rig, lots[0]) {
            break;
        }
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
    }
    assert!(listed(&rig, lots[0]), "the first lot is its own ask");
    assert_eq!(in_stores(&mut rig), 4);
    for lot in &lots {
        let row = provenance(&mut rig, *lot);
        assert_eq!(row.origin_word(), "seeded");
        assert!(row.fitted_into.is_none());
    }

    // (2) The casualty, berthed at the Dockmaster's yard; the quote names the part's price
    // beside the labour.
    let u = unit(&rig, escort);
    let cannon = component_id(&u, "cannon");
    wreck_cannon(&mut rig, escort, &cannon);
    let workshop = station_room(&rig.tw, CERES, C14);
    staff_hand(&mut rig, |ctx| {
        let _ = mush_sim::unplace(ctx, pilot, "staff");
        ctx.w.move_to(pilot, workshop).expect("into the room");
    });
    let berthed = run(&mut rig.tw, pilot, &format!("+bay/berth #{}", escort.0));
    assert!(berthed.contains("berthed in the workshop"), "{berthed}");
    let quote = run(&mut rig.tw, pilot, &format!("+bay #{}", escort.0));
    let (_, priced) = tacsim::port::bay_work::quote_here(rig.tw.world(), escort).expect("a quote");
    let replace = priced
        .jobs
        .iter()
        .find(|j| matches!(&j.job, tacsim::port::bay_work::BayJob::Replace { component, .. } if *component == cannon))
        .expect("the cannon's replace");
    assert_eq!(
        replace.cost_minor, 48_000,
        "400 of labour at Occator's 1.2x"
    );
    assert_eq!(
        replace.part_minor, 2_800_000,
        "the cannon's catalogue 28,000"
    );
    assert!(
        quote.contains("replace") && quote.contains("from the workshop's stores"),
        "{quote}"
    );

    // (3) The order: 28,480 escrowed, the first lot taken from the stores, its ask withdrawn.
    let done = refit(&mut rig, pilot, escort);
    assert_eq!(
        done.fee_minor, 2_848_000,
        "480 of labour and 28,000 for the part"
    );
    assert_eq!(done.part_vault, Some(stores(&rig)));
    let taken = done
        .jobs
        .iter()
        .find_map(|j| match &j.job {
            tacsim::port::bay_work::BayJob::Replace { lot, .. } => *lot,
            _ => None,
        })
        .expect("the store's lot");
    assert_eq!(taken, lots[0], "the lowest-grade lot, first in seed order");
    assert!(!listed(&rig, taken), "its ask came down");

    // (4) The cannon whole; the lot consumed, its row `Seeded` with the escort it went into; the
    // stores one less.
    let after = tacsim::field_ops::assess_tacsim_unit(&unit(&rig, escort), None).expect("assessed");
    assert_eq!(
        after.component_conditions[&cannon],
        ComponentCondition::Intact
    );
    let row = provenance(&mut rig, taken);
    assert_eq!(row.origin_word(), "seeded");
    assert_eq!(row.fitted_into, Some(escort));
    assert_eq!(
        in_stores(&mut rig),
        3,
        "the store's count after the order is four less one"
    );

    // (5) Three more refits empty the stores; the fifth replace refuses `NO_PART`.
    for left in [2, 1, 0] {
        wreck_cannon(&mut rig, escort, &cannon);
        let _ = refit(&mut rig, pilot, escort);
        assert_eq!(in_stores(&mut rig), left);
    }
    wreck_cannon(&mut rig, escort, &cannon);
    let no_part = "The workshop has no 105 mm cannon mount in its stores";
    let quote = run(&mut rig.tw, pilot, &format!("+bay #{}", escort.0));
    assert!(quote.contains(no_part), "{quote}");
    let refused = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/order #{}=replace", escort.0),
    );
    assert!(refused.contains(no_part), "{refused}");
    assert_eq!(
        order(&rig, escort).status,
        OrderStatus::Done,
        "nothing was taken"
    );
}

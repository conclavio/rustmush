//! **Doc 39 cut IN.2 — the component lifecycle**, over the composed world (`driving`'s keyed
//! rig), through the production command paths.
//!
//! The story (doc 39 §6, the IN.2 row: sell → buy → fit): two captains sail their Dray-Ps from
//! Ceres to Stickney Yards, the seller with a titled spare escort in the bay, the buyer with a
//! titled scout. At the Service Counter (P11, Stickney Yards' workshop):
//!
//! 1. `+bay/strip` takes the escort's visual sight off: one lot of
//!    `tacsim.part.sensor.visual_mk1` with a `Stripped` row naming the escort.
//! 2. `@inv/split` on it refuses at once: a part is one identified lot (Q-39-27).
//! 3. The seller lists **that lot** on the yard's board (`@market/vendor stock … lot=<lot>`): the
//!    listing holds the lot itself, so the bay will not take it into an order meanwhile.
//! 4. The buyer quotes and accepts it: the sale hands **the lot itself** to the buyer's berthed
//!    hold — the same id, its row unchanged — never a tranche.
//! 5. The buyer's scout's sight is wrecked; `+bay/fit` holds the lot for the order and fits it.
//!    The row reads the same lot id with both hulls: the escort it came off and the scout it went
//!    into.
//! 6. A listing withdrawn releases its lot where it lies, and a hot part refuses at the fit: the
//!    seller strips the escort's coaxial mount, lists and withdraws it, its claim is
//!    vacated, and `+bay/fit` refuses it.
//!
//! **Seeds** (doc 43 rule 2.3 #4; doc 36a §8): the escort and the scout reach their captains'
//! titles by staff (`@asset/transfer`, `@unit/create` + `@asset/title`) and the bays by staff
//! placement, as IN.1's story does; the seller is made a broker of Stickney Yards
//! (`@org/grant … economy.account.post`), which is what lets them list on the yard's board; the
//! wrecked sight is staff damage through the unit's one plan path; the vacated claim is staff's
//! `vacate_claim` (GC.8's seam). Every claim — the strip, the refusal, the listing, the sale, the
//! fit, the hot refusal — is reached by the player's command.
//!
//! **No site**: the cut is port-side, so it has no 256/512 pair (IN.4's precedent).

use mush_core::{GOD, ObjId};
use mush_inventory::record::EndpointRef;
use mush_tactics::field_ops::ComponentCondition;
use mush_tactics::{UnitMutationOp, UnitMutationPlan};
use tacsim::port::HullRow;
use tacsim::port::bay_work::{BayHullState, OrderStatus};

use crate::driving_rig::{
    Rig, at_counter, board, cid, drain, file_and_launch, fire, hold_vault_of, land_all,
    land_quietly, load, port_map, prospector, run, staff_hand, station_room, stocked, store, to_ic,
    typed,
};

const STICKNEY: &str = "stickney_port";
const SIGHT_PART: &str = "tacsim.part.sensor.visual_mk1";
const MOUNT_PART: &str = "tacsim.part.weapon.mg_coax";

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

/// Staff put `who` in `room`: into a hull's interior space, or into a plain port room.
fn put(rig: &mut Rig, who: ObjId, room: ObjId, interior: bool) {
    staff_hand(rig, |ctx| {
        if interior {
            mush_sim::place_interior(ctx, who, room, None).expect("into the space");
        } else {
            let _ = mush_sim::unplace(ctx, who, "staff");
            ctx.w.move_to(who, room).expect("into the room");
        }
    });
}

fn mint(rig: &mut Rig, pilot: ObjId) {
    let treasury = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let account = tacsim::port::credit::own_account(rig.tw.world(), pilot)
        .map(|a| a.id.0)
        .expect("the pilot's account");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={account} 10000000"),
    );
}

fn record(rig: &Rig, hull: ObjId) -> tacsim::port::bay_work::BayHull {
    match store(&rig.tw).bay_hull(hull) {
        Some(BayHullState::Present(row)) => row.clone(),
        other => panic!("the bay record: {other:?}"),
    }
}

/// A command, its submissions landed, then fires until `hull`'s record owes nothing.
fn settled(rig: &mut Rig, who: ObjId, line: &str, hull: ObjId) -> String {
    let out = run(&mut rig.tw, who, line);
    let queued = drain(&mut rig.tw);
    land_all(rig, &queued);
    for _ in 0..6 {
        if matches!(store(&rig.tw).bay_hull(hull), Some(BayHullState::Present(r)) if r.owed.is_none())
        {
            return out;
        }
        let (_, queued) = fire(rig);
        land_quietly(rig, &queued);
    }
    panic!("{line}: the bay's act did not settle\n{out}");
}

fn inventory(rig: &Rig) -> mush_inventory::InventoryFamilies {
    rig.tw
        .world()
        .module_state::<tacsim::port::bay_work::act::BayActHandles>()
        .cloned()
        .and_then(|h| h.inventory)
        .expect("the inventory")
}

fn lot_record(rig: &mut Rig, lot: mush_inventory::LotId) -> Option<mush_inventory::Lot> {
    let inventory = inventory(rig);
    typed(&mut rig.tw, &inventory.lots, &lot)
}

fn hold_on(rig: &mut Rig, lot: mush_inventory::LotId) -> Option<mush_inventory::LotHold> {
    let inventory = inventory(rig);
    typed(&mut rig.tw, &inventory.lot_holds, &lot)
}

fn provenance(
    rig: &mut Rig,
    lot: mush_inventory::LotId,
) -> tacsim::port::bay_work::act::PartProvenance {
    let handles = rig
        .tw
        .world()
        .module_state::<tacsim::port::bay_work::act::BayActHandles>()
        .cloned()
        .expect("the bay's handles");
    typed(&mut rig.tw, &handles.parts, &lot).expect("the part's provenance")
}

fn market(rig: &Rig) -> &mush_market::MarketStore {
    rig.tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .expect("the market")
}

/// A thing made by staff, its id.
fn made(rig: &mut Rig, name: &str) -> ObjId {
    let out = run(&mut rig.tw, GOD, &format!("@create {name}"));
    out.split('#')
        .nth(1)
        .and_then(|rest| {
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()
        })
        .map(ObjId)
        .unwrap_or_else(|| panic!("a thing: {out}"))
}

/// **The two captains**: the seller's Dray-P with a titled spare escort of Occator's pool in
/// its bay; the buyer's Dray-P and a titled scout (which has no `logistics`, so staff put it in
/// the buyer's bay at Stickney).
fn captains(rig: &mut Rig) -> ((ObjId, HullRow, ObjId), (ObjId, HullRow, ObjId)) {
    let (seller, seller_row) = prospector(rig, "Seller", 1, "Salt Moth");
    mint(rig, seller);
    at_counter(rig, seller);
    let escort_schema = cid("tacsim.unit.tv_escort");
    let escort = rig
        .tw
        .world()
        .module_state::<mush_tactics::UnitStore>()
        .expect("the units")
        .units
        .iter()
        .find(|(_, u)| u.schema().key().id.as_str() == escort_schema.as_str())
        .map(|(id, _)| *id)
        .expect("a spare escort in Occator's pool");
    let moved = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/transfer #{}=#{}", escort.0, seller.0),
    );
    assert!(!moved.contains("refused"), "{moved}");
    put(rig, escort, ObjId(seller_row.rooms[&cid("bay")]), true);

    let (buyer, buyer_row) = prospector(rig, "Buyer", 2, "Red Moth");
    mint(rig, buyer);
    at_counter(rig, buyer);
    let scout = made(rig, "Quick Wren");
    let unit = run(
        &mut rig.tw,
        GOD,
        &format!("@unit/create #{}=tacsim.unit.scout_mk1", scout.0),
    );
    assert!(
        mush_tactics::module::unit_of(rig.tw.world(), scout).is_some(),
        "{unit}"
    );
    let titled = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/title #{}=#{}", scout.0, buyer.0),
    );
    assert!(!titled.contains("refused"), "{titled}");
    ((seller, seller_row, escort), (buyer, buyer_row, scout))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn in2_a_sight_stripped_at_stickney_is_sold_bought_and_fitted_to_a_scout() {
    let mut rig = stocked();
    let ((seller, seller_row, escort), (buyer, buyer_row, scout)) = captains(&mut rig);
    let seller_hold = hold_vault_of(&rig.tw, &seller_row);
    let buyer_hold = hold_vault_of(&rig.tw, &buyer_row);

    // ---- Both to Stickney Yards ----
    let mut arrive = 0;
    for (pilot, row, seed) in [(seller, &seller_row, 9_500), (buyer, &buyer_row, 9_510)] {
        load(&mut rig, row, 60, 40, seed);
        board(&mut rig, pilot);
        let t = file_and_launch(&mut rig, pilot, ObjId(row.hull), "+ship/undock stickney");
        arrive = arrive.max(t.plan.depart_ic + t.plan.total_s);
    }
    let _ = to_ic(&mut rig, arrive);
    let _ = to_ic(&mut rig, arrive + 3_600);
    for row in [&seller_row, &buyer_row] {
        assert!(
            matches!(
                tacsim::port::ship::berth_of(rig.tw.world(), ObjId(row.hull)),
                tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == STICKNEY
            ),
            "docked at Stickney Yards"
        );
    }
    put(&mut rig, scout, ObjId(buyer_row.rooms[&cid("bay")]), true);
    let workshop = station_room(&rig.tw, STICKNEY, "room.p11_service");
    for (pilot, hull) in [(seller, escort), (buyer, scout)] {
        put(&mut rig, pilot, workshop, false);
        let berthed = run(&mut rig.tw, pilot, &format!("+bay/berth #{}", hull.0));
        assert!(berthed.contains("berthed in the workshop"), "{berthed}");
    }

    // (1) The escort's sight comes off as a traced part.
    let out = settled(
        &mut rig,
        seller,
        &format!("+bay/strip #{}=visual_sensor", escort.0),
        escort,
    );
    assert!(out.contains("goes to the carrier's hold"), "{out}");
    let lot = tacsim::port::bay_work::act::lot_id(escort, 1, 0).expect("the lot");
    let found = lot_record(&mut rig, lot).expect("the part's lot");
    assert_eq!(found.commodity().as_str(), SIGHT_PART);
    assert_eq!(found.endpoint(), EndpointRef(seller_hold));
    let stripped = provenance(&mut rig, lot);
    assert_eq!(stripped.parent_hull(), Some(escort));
    assert_eq!(stripped.origin_word(), "stripped");

    // (2) A part never splits.
    let split = run(&mut rig.tw, seller, &format!("@inv/split {lot}=1"));
    assert!(split.contains("that is a part"), "{split}");
    assert!(drain(&mut rig.tw).is_empty(), "the refusal stages nothing");

    // (3) The seller, a broker of the yard, lists that lot on the yard's board.
    let vendor = port_map(&rig.tw, STICKNEY).vendors[&cid("stickney_yards")]
        .vendor
        .expect("the yard's vendor");
    let org = market(&rig)
        .vendor(mush_market::VendorId(vendor))
        .map(|v| v.org)
        .expect("the yard's org");
    let granted = run(
        &mut rig.tw,
        GOD,
        &format!("@org/grant #{}=#{} economy.account.post", seller.0, org.0),
    );
    assert!(!granted.contains("Permission denied"), "{granted}");
    let listed = run(
        &mut rig.tw,
        seller,
        &format!(
            "@market/vendor stock #{vendor}=tacsim:{SIGHT_PART} price=1500000 endpoint={} lot={lot}",
            seller_hold.0
        ),
    );
    let queued = drain(&mut rig.tw);
    assert_eq!(queued.len(), 1, "the listing holds its lot: {listed}");
    land_all(&mut rig, &queued);
    let listing = market(&rig)
        .lot_listings()
        .find(|l| l.source.exact_lot() == Some(lot))
        .map(|l| l.id)
        .unwrap_or_else(|| panic!("the lot's listing: {listed}"));
    let hold = hold_on(&mut rig, lot).expect("the listing holds the lot");
    assert_eq!(hold.holder(), mush_market::listing_holder_text(listing));
    // Held for the sale, no order may take it.
    let refused = run(
        &mut rig.tw,
        seller,
        &format!("+bay/fit #{}=visual_sensor {lot}", escort.0),
    );
    assert!(refused.contains("held for a sale"), "{refused}");

    // (4) The buyer buys it: the lot itself moves to their hold.
    let quoted = run(&mut rig.tw, buyer, &format!("+market/quote {listing}"));
    assert!(quoted.contains("Quote #"), "{quoted}");
    let quote = market(&rig)
        .quotes_of_buyer(&mush_org::PrincipalId::Object(buyer))
        .map(|q| q.id)
        .max()
        .expect("a quote");
    let accepted = run(&mut rig.tw, buyer, &format!("+market/accept {quote}"));
    let queued = drain(&mut rig.tw);
    assert!(!queued.is_empty(), "the hand-over is staged: {accepted}");
    land_all(&mut rig, &queued);
    let moved = lot_record(&mut rig, lot).expect("the same lot, sold");
    assert_eq!(
        moved.endpoint(),
        EndpointRef(buyer_hold),
        "in the buyer's hold"
    );
    assert_eq!(moved.commodity().as_str(), SIGHT_PART);
    assert_eq!(moved.quantity(), 1);
    assert!(hold_on(&mut rig, lot).is_none(), "the sale ended the hold");
    assert_eq!(
        provenance(&mut rig, lot),
        stripped,
        "the row follows the lot"
    );

    // (5) The scout's sight is wrecked; the bought one fits it.
    let s = unit(&rig, scout);
    let sight = component_id(&s, "visual_sensor");
    let plan = UnitMutationPlan::new(
        vec![UnitMutationOp::SetComponentCondition {
            component: sight.clone(),
            condition: ComponentCondition::Destroyed
                .to_condition_id()
                .expect("destroyed"),
        }],
        Some(s.revision()),
        None,
    )
    .expect("a plan");
    staff_hand(&mut rig, |ctx| {
        mush_tactics::module::apply_plan(ctx, scout, &plan).expect("the damage lands");
    });
    let taken = run(
        &mut rig.tw,
        buyer,
        &format!("+bay/fit #{}=visual_sensor {lot}", scout.0),
    );
    assert!(taken.contains("taken in the workshop"), "{taken}");
    let order = record(&rig, scout).order.expect("an order");
    let _ = to_ic(&mut rig, order.started_ic + 60);
    assert_eq!(
        hold_on(&mut rig, lot).map(|h| h.holder().to_owned()),
        Some(tacsim::port::bay_work::act::bay_holder(scout, order.seq)),
        "the order holds its part"
    );
    for at in [60, 600, 1_200] {
        let _ = to_ic(&mut rig, order.due_ic + at);
    }
    let order = record(&rig, scout).order.expect("an order");
    assert_eq!(order.status, OrderStatus::Done, "{order:?}");
    let assessment =
        tacsim::field_ops::assess_tacsim_unit(&unit(&rig, scout), None).expect("assessed");
    assert_eq!(
        assessment.component_conditions[&sight],
        ComponentCondition::Intact,
        "the escort's sight is the scout's now"
    );
    assert!(lot_record(&mut rig, lot).is_none(), "the lot is consumed");
    assert!(hold_on(&mut rig, lot).is_none(), "and its hold with it");
    let fitted = provenance(&mut rig, lot);
    assert_eq!(fitted.lot, lot);
    assert_eq!(fitted.parent_hull(), Some(escort), "the hull it came off");
    assert_eq!(fitted.fitted_into, Some(scout), "the hull it went into");

    // (6) A hot part refuses at the fit.
    let out = settled(
        &mut rig,
        seller,
        &format!("+bay/strip #{}=machine_gun", escort.0),
        escort,
    );
    assert!(out.contains("goes to the carrier's hold"), "{out}");
    let mount = tacsim::port::bay_work::act::lot_id(escort, 2, 0).expect("the lot");
    // Listed and withdrawn first: the withdrawal's release gives the lot back where it lies.
    let listed = run(
        &mut rig.tw,
        seller,
        &format!(
            "@market/vendor stock #{vendor}=tacsim:{MOUNT_PART} price=900000 endpoint={} lot={mount}",
            seller_hold.0
        ),
    );
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    let mount_listing = market(&rig)
        .lot_listings()
        .find(|l| l.source.exact_lot() == Some(mount))
        .map(|l| l.id)
        .unwrap_or_else(|| panic!("the mount's listing: {listed}"));
    assert!(
        hold_on(&mut rig, mount).is_some(),
        "the listing holds the mount"
    );
    let withdrawn = run(
        &mut rig.tw,
        seller,
        &format!("@market/vendor withdraw {mount_listing}"),
    );
    let queued = drain(&mut rig.tw);
    assert_eq!(queued.len(), 1, "the release is staged: {withdrawn}");
    land_all(&mut rig, &queued);
    assert!(
        hold_on(&mut rig, mount).is_none(),
        "the withdrawal released it"
    );
    assert_eq!(
        lot_record(&mut rig, mount).map(|l| l.endpoint()),
        Some(EndpointRef(seller_hold)),
        "where it lay"
    );
    staff_hand(&mut rig, |ctx| {
        tacsim::port::bay_work::strip::vacate_claim(ctx, seller, escort, mount, 7, buyer)
            .expect("the claim is vacated");
    });
    for _ in 0..6 {
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
    }
    assert!(provenance(&mut rig, mount).is_hot());
    let refused = run(
        &mut rig.tw,
        seller,
        &format!("+bay/fit #{}=machine_gun {mount}", escort.0),
    );
    assert!(refused.contains("That part is hot"), "{refused}");
}

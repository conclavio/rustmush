//! **Doc 36a lane 2 — the bay after the external review** (doc 36a §3 P0-6, P1-2, §4 P1-7,
//! P2-4, P2-5, §5 items 1 and 4): one regression test an item, each driven through `+bay/*`
//! as a player types it, over the composed world (`driving`'s keyed rig) at Ceres.
//!
//! **What is a fixture.** The hulls' damage is staff's through the unit's one plan path
//! (GC.8a's precedent), a spare of Occator's pool is titled by `@asset/transfer` and put into
//! a bay by hand (GC.8b's), and a salvor's custody row is seeded on a lane site's record
//! (GC.5's and GC.6's precedent: the custody is the tow's and the load's to write, and the
//! thing these tests claim — the bay's rights over a hull in custody — is reached by `+bay`).
//! No standing bid buys munitions in the content, so the draw that must fail is starved by a
//! second order against the same hold, not by a sale.

use mush_core::{GOD, ObjId};

use mush_org::PrincipalId;
use mush_tactics::UnitMutationOp;
use mush_tactics::field_ops::ComponentCondition;
use tacsim::port::HullRow;
use tacsim::port::bay_work::{BayHull, BayHullState, OrderStatus};
use tacsim::surface::salvage::{ClaimSeam, SalvageCustody};
use tacsim::surface::site_record::{SiteKey, SiteRecord};

use crate::bay_gc8::{
    bay_room, buy, component_id, damage, empty_magazine, in_bay, lot_record, magazine, mint,
    order_of, provenance, put, section_id, unit, wreck_section,
};
use crate::driving::{
    Rig, at_counter, cid, fire, handles, hold_pool, hold_vault_of, land_quietly, pilot, prospector,
    reseed_typed, rigs_of, run, seed_hold, station_room, stocked, store, to_ic,
};
use crate::field_host::site_record_at;

const CERES: &str = "ceres_port";
/// Ceres' workshop room (C14, Occator's).
const C14: &str = "room.c14_dockmaster";
/// The lane site whose record carries the seeded custody rows.
const URVARA: &str = "urvara_port";

/// The hull's bay record.
fn bay_record(rig: &Rig, hull: ObjId) -> BayHull {
    match store(&rig.tw).bay_hull(hull) {
        Some(BayHullState::Present(row)) => row.clone(),
        other => panic!("the bay record of #{}: {other:?}", hull.0),
    }
}

/// The hull's assessment.
fn assess(rig: &Rig, hull: ObjId) -> mush_tactics::field_ops::FieldUnitAssessment {
    tacsim::field_ops::assess_tacsim_unit(&unit(rig, hull), None).expect("assessed")
}

/// Who holds the hull's title.
fn title_of(rig: &Rig, hull: ObjId) -> Option<PrincipalId> {
    rig.tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| e.snapshot().asset(hull).map(|a| a.title_owner.clone()))
}

/// A spare of Occator's pool of `like`'s design, under no lease.
fn spare_escort(rig: &Rig, like: ObjId) -> ObjId {
    let schema = unit(rig, like).schema().clone();
    rig.tw
        .world()
        .module_state::<mush_tactics::UnitStore>()
        .expect("the units")
        .units
        .iter()
        .filter(|(id, u)| **id != like && u.schema() == &schema)
        .map(|(id, _)| *id)
        .find(|id| store(&rig.tw).tv_lease_on(*id).is_none())
        .expect("a spare escort in Occator's pool")
}

/// Fire the port tick until `hull`'s record owes nothing.
fn settle_until(rig: &mut Rig, hull: ObjId) {
    for _ in 0..8 {
        if bay_record(rig, hull).owed.is_none() {
            return;
        }
        let (_, queued) = fire(rig);
        land_quietly(rig, &queued);
    }
    panic!("#{}'s bay act did not settle", hull.0);
}

/// **The captain's Dray-P at Ceres with two escorts in its bay**: Occator's leased one in slot
/// 1 and a spare of the pool titled to the captain in slot 2.
fn two_escorts(rig: &mut Rig) -> (ObjId, HullRow, ObjId, ObjId) {
    let (pilot, row) = prospector(rig, "Pilot", 1, "Salt Moth");
    mint(rig, pilot);
    at_counter(rig, pilot);
    let signed = run(&mut rig.tw, pilot, "+tv/lease escort go");
    assert!(signed.contains("leases you OWP-ES-01"), "{signed}");
    let escort = rigs_of(&rig.tw, pilot)[0];
    let spare = spare_escort(rig, escort);
    let moved = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/transfer #{}=#{}", spare.0, pilot.0),
    );
    assert_eq!(
        title_of(rig, spare),
        Some(PrincipalId::Object(pilot)),
        "{moved}"
    );
    put(rig, spare, bay_room(&row), true);
    (pilot, row, escort, spare)
}

/// **Both escorts' cannons spent, the hold carrying 105 mm HE for one**, and a rearm ordered on
/// each from the carrier's bay — the leased escort's first, the spare's halfway to its due, so
/// the first draw empties the hold the second draws from. Returns the two orders' hulls.
fn two_rearms_one_load(rig: &mut Rig) -> (ObjId, HullRow, ObjId, ObjId) {
    let (pilot, row, escort, spare) = two_escorts(rig);
    let cannon = component_id(&unit(rig, escort), "cannon");
    let rounds = u64::try_from(magazine(&unit(rig, escort), &cannon).1).expect("a magazine");
    buy(rig, pilot, "coop", "ammo_he_105", rounds);
    for hull in [escort, spare] {
        let c = component_id(&unit(rig, hull), "cannon");
        damage(rig, hull, vec![empty_magazine(&c)]);
    }
    put(rig, pilot, bay_room(&row), true);
    let first = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/order #{}=rearm", escort.0),
    );
    assert!(first.contains("taken in the carrier bay"), "{first}");
    let due = order_of(rig, escort).due_ic;
    let now = rig.tw.world().world_ic_now().at;
    let _ = to_ic(rig, now + (due - now) / 2);
    put(rig, pilot, bay_room(&row), true);
    let second = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/order #{}=rearm", spare.0),
    );
    assert!(second.contains("taken in the carrier bay"), "{second}");
    assert!(
        order_of(rig, spare).due_ic > due,
        "the spare's order is due later"
    );
    (pilot, row, escort, spare)
}

/// The leased escort's draw lands and its work is done; the hold is empty.
fn first_draw_lands(rig: &mut Rig, row: &HullRow, escort: ObjId) {
    let due = order_of(rig, escort).due_ic;
    let _ = to_ic(rig, due + 60);
    let _ = to_ic(rig, due + 600);
    assert_eq!(order_of(rig, escort).status, OrderStatus::Done);
    let vault = hold_vault_of(&rig.tw, row);
    assert_eq!(hold_pool(&mut rig.tw, vault, "ammo_he_105"), 0);
}

/// Fire until the spare's order is cancelled (its draw given up).
fn given_up(rig: &mut Rig, spare: ObjId) -> tacsim::port::bay_work::BayOrder {
    for _ in 0..10 {
        if order_of(rig, spare).status == OrderStatus::Cancelled {
            break;
        }
        let (_, queued) = fire(rig);
        land_quietly(rig, &queued);
    }
    let cancelled = order_of(rig, spare);
    assert_eq!(cancelled.status, OrderStatus::Cancelled, "{cancelled:?}");
    assert!(
        cancelled
            .note
            .as_deref()
            .is_some_and(|n| n.contains("could not supply")),
        "{cancelled:?}"
    );
    cancelled
}

/// **36a §5.1 — a given-up act does not brick the hull.** The spare's rearm finds the hold
/// empty: its draw is refused three times and the order cancelled, nothing drawn. The record's
/// count then follows the ledger, so the title holder's `+bay/strip` of the spare's cannon
/// still mints its lot at the ledger's next ordinal — and the cannon stays on the hull until
/// that mint has landed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b36a_1_a_given_up_draw_does_not_brick_the_hull() {
    let mut rig = stocked();
    let (pilot, row, escort, spare) = two_rearms_one_load(&mut rig);
    first_draw_lands(&mut rig, &row, escort);
    let due = order_of(&rig, spare).due_ic;
    let _ = to_ic(&mut rig, due + 60);
    let _ = given_up(&mut rig, spare);
    let record = bay_record(&rig, spare);
    assert!(record.owed.is_none(), "{record:?}");
    assert_eq!(
        record.acts, 0,
        "the record's count follows the ledger, which landed nothing: {record:?}"
    );
    // The title holder berths the spare at C14 and strips its cannon.
    let workshop = station_room(&rig.tw, CERES, C14);
    put(&mut rig, pilot, workshop, false);
    let berthed = run(&mut rig.tw, pilot, &format!("+bay/berth #{}", spare.0));
    assert!(berthed.contains("berthed in the workshop"), "{berthed}");
    let cannon = component_id(&unit(&rig, spare), "cannon");
    let out = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/strip #{}=cannon", spare.0),
    );
    assert!(out.contains("goes to the carrier's hold"), "{out}");
    assert_ne!(
        assess(&rig, spare).component_conditions[&cannon],
        ComponentCondition::Missing,
        "the cannon stays on the hull until the mint lands"
    );
    settle_until(&mut rig, spare);
    let lot = tacsim::port::bay_work::act::lot_id(spare, 1, 0).expect("the lot");
    let found = lot_record(&mut rig, lot).expect("the cannon's lot is minted");
    assert_eq!(found.commodity().as_str(), "tacsim.part.cannon");
    assert_eq!(provenance(&mut rig, lot).slot, cannon);
    assert_eq!(
        assess(&rig, spare).component_conditions[&cannon],
        ComponentCondition::Missing,
        "the cannon came off when the mint landed"
    );
    assert_eq!(bay_record(&rig, spare).acts, 1);
}

/// **36a §5.1 (b) — the hull takes a new order after the give-up.** The trigger needs no bad
/// intent: `take` checks the hold but reserves nothing, so two rearms against one load are
/// enough, and the draw that loses is given up. The hold restocked, the spare is ordered again
/// from the carrier's bay: its draw lands at the ledger's next ordinal and the work is done.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b36a_1_b_the_hull_takes_a_new_order_after_the_give_up() {
    let mut rig = stocked();
    let (pilot, row, escort, spare) = two_rearms_one_load(&mut rig);
    first_draw_lands(&mut rig, &row, escort);
    let due = order_of(&rig, spare).due_ic;
    let _ = to_ic(&mut rig, due + 60);
    let _ = given_up(&mut rig, spare);
    let cannon = component_id(&unit(&rig, spare), "cannon");
    let rounds = u64::try_from(magazine(&unit(&rig, spare), &cannon).1).expect("a magazine");
    // (Out of the bay by staff's hand: a teleport does not leave a ship's interior.)
    let counter = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    put(&mut rig, pilot, counter, false);
    buy(&mut rig, pilot, "coop", "ammo_he_105", rounds);
    put(&mut rig, pilot, bay_room(&row), true);
    let again = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/order #{}=rearm", spare.0),
    );
    assert!(again.contains("taken in the carrier bay"), "{again}");
    first_draw_lands(&mut rig, &row, spare);
    let (loaded, capacity) = magazine(&unit(&rig, spare), &cannon);
    assert_eq!(loaded, capacity, "the spare is rearmed");
    assert_eq!(
        bay_record(&rig, spare).acts,
        1,
        "the draw landed at the ledger's next ordinal"
    );
}

/// The cannon jammed, through the unit's one plan path.
fn jam_cannon(rig: &mut Rig, hull: ObjId) -> mush_tactics::ComponentId {
    let cannon = component_id(&unit(rig, hull), "cannon");
    damage(
        rig,
        hull,
        vec![UnitMutationOp::SetComponentCondition {
            component: cannon.clone(),
            condition: ComponentCondition::Jammed
                .to_condition_id()
                .expect("jammed"),
        }],
    );
    cannon
}

/// **36a P0-6 (a) — a doomed order is refused at the counter.** The escort's turret is
/// destroyed, and its parts with it; berthed at C14, the lessee orders the parts replaced
/// without the turret's rebuild. The workshop engine would refuse that work at completion (the
/// host section does not stand) — after the parts were drawn from the hold — so the order is
/// refused now, with the engine's reason, and no fee is held. Ordered with the rebuild, it is
/// taken.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b36a_2_a_doomed_order_is_refused_at_the_counter() {
    let mut rig = stocked();
    let (pilot, row, escort, _) = two_escorts(&mut rig);
    seed_hold(&mut rig, &row, "industrial_parts", 40, 9_700);
    let u = unit(&rig, escort);
    let turret = section_id(&u, "turret");
    damage(&mut rig, escort, wreck_section(&u, &turret));
    let workshop = station_room(&rig.tw, CERES, C14);
    put(&mut rig, pilot, workshop, false);
    let berthed = run(&mut rig.tw, pilot, &format!("+bay/berth #{}", escort.0));
    assert!(berthed.contains("berthed in the workshop"), "{berthed}");
    let quote = run(&mut rig.tw, pilot, &format!("+bay #{}", escort.0));
    assert!(
        quote.contains("replace cannon") && quote.contains("rebuild turret"),
        "{quote}"
    );
    let refused = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/order #{}=replace", escort.0),
    );
    assert!(
        refused.contains("cannot be done as ordered") && refused.contains("reconstruct"),
        "{refused}"
    );
    assert!(
        bay_record(&rig, escort).open().is_none(),
        "nothing was taken: {:?}",
        bay_record(&rig, escort)
    );
    let taken = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/order #{}=rebuild,replace", escort.0),
    );
    assert!(taken.contains("taken in the workshop"), "{taken}");
    let cancelled = run(&mut rig.tw, pilot, &format!("+bay/cancel #{}", escort.0));
    assert!(cancelled.contains("cancelled"), "{cancelled}");
}

/// **36a P0-6 (b) — no cancel while the draw settles.** The spare's rearm comes due on an empty
/// hold: its draw is owed and refused, and until the bay gives the act up the order is not
/// cancelled under it (SETTLING); once given up there is nothing left to cancel.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b36a_2_b_no_cancel_while_the_draw_settles() {
    let mut rig = stocked();
    let (pilot, row, escort, spare) = two_rearms_one_load(&mut rig);
    first_draw_lands(&mut rig, &row, escort);
    let due = order_of(&rig, spare).due_ic;
    let _ = to_ic(&mut rig, due + 60);
    let record = bay_record(&rig, spare);
    assert!(record.owed.is_some(), "the draw is owed: {record:?}");
    assert_eq!(order_of(&rig, spare).status, OrderStatus::Open);
    let refused = run(&mut rig.tw, pilot, &format!("+bay/cancel #{}", spare.0));
    assert!(refused.contains("settling"), "{refused}");
    assert_eq!(order_of(&rig, spare).status, OrderStatus::Open);
    let _ = given_up(&mut rig, spare);
    let gone = run(&mut rig.tw, pilot, &format!("+bay/cancel #{}", spare.0));
    assert!(gone.contains("no open order"), "{gone}");
}

/// **36a P0-6 (c) — no cancel once the draw has landed either.** The second window: the
/// escort's draw has landed (the hold is empty) but the tick has not yet settled it into the
/// work. A cancel now would refund the fee and leave the goods gone, so it is refused
/// (SETTLING) until the next tick completes the order.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b36a_2_c_no_cancel_once_the_draw_has_landed() {
    let mut rig = stocked();
    let (pilot, row, escort, _) = two_rearms_one_load(&mut rig);
    let due = order_of(&rig, escort).due_ic;
    let _ = to_ic(&mut rig, due + 60);
    let vault = hold_vault_of(&rig.tw, &row);
    assert_eq!(
        hold_pool(&mut rig.tw, vault, "ammo_he_105"),
        0,
        "the draw has landed"
    );
    let record = bay_record(&rig, escort);
    assert!(record.owed.is_some(), "not yet settled: {record:?}");
    assert_eq!(order_of(&rig, escort).status, OrderStatus::Open);
    let refused = run(&mut rig.tw, pilot, &format!("+bay/cancel #{}", escort.0));
    assert!(refused.contains("settling"), "{refused}");
    assert_eq!(order_of(&rig, escort).status, OrderStatus::Open);
    let _ = to_ic(&mut rig, due + 600);
    let order = order_of(&rig, escort);
    assert_eq!(order.status, OrderStatus::Done, "{order:?}");
    let cannon = component_id(&unit(&rig, escort), "cannon");
    let (loaded, capacity) = magazine(&unit(&rig, escort), &cannon);
    assert_eq!(loaded, capacity, "the escort is rearmed");
}

/// **A hull in a salvor's custody**: the custody row for `hull`, held by `holder` aboard
/// `carrier`, written on Urvara's site record (the load's record; GC.5's shape).
fn seed_custody(rig: &mut Rig, hull: ObjId, holder: ObjId, carrier: ObjId) {
    let now = rig.tw.world().world_ic_now().at;
    let mut record = site_record_at(rig, URVARA).unwrap_or_else(|| SiteRecord::new(cid(URVARA)));
    record.custody.retain(|row| row.hull != hull);
    record.custody.push(SalvageCustody {
        hull,
        holder,
        carrier,
        recovered_by: carrier,
        recovered_ic: now,
        left_ic: None,
        claim: ClaimSeam::Owed,
        operation: None,
        pin: None,
        file: None,
        told: vec![],
        evidence: vec![],
    });
    record.custody.sort_by_key(|row| row.hull);
    let h = handles(&rig.tw);
    reseed_typed(rig, &h.families.sites, &SiteKey(cid(URVARA)), record);
}

/// The custody row for `hull` taken off Urvara's record: the claim vacated.
fn vacate_custody(rig: &mut Rig, hull: ObjId) {
    let mut record = site_record_at(rig, URVARA).expect("the site record");
    record.custody.retain(|row| row.hull != hull);
    let h = handles(&rig.tw);
    reseed_typed(rig, &h.families.sites, &SiteKey(cid(URVARA)), record);
}

/// **Salvor S holds owner O's hull in S's bay**: S's Dray-P at Ceres with S's leased escort in
/// slot 1 and, in slot 2, a spare of the pool titled to O (`@asset/transfer`) under S's live
/// custody (the seeded row). Returns S, S's hull row, O and the hull.
fn salvor_holds_owners_hull(rig: &mut Rig) -> (ObjId, HullRow, ObjId, ObjId) {
    let (s, s_row) = prospector(rig, "Second", 2, "Red Moth");
    mint(rig, s);
    at_counter(rig, s);
    let signed = run(&mut rig.tw, s, "+tv/lease escort go");
    assert!(signed.contains("leases you OWP-ES-01"), "{signed}");
    let s_escort = rigs_of(&rig.tw, s)[0];
    let o = pilot(rig, "Owner", 3);
    let hull = spare_escort(rig, s_escort);
    let moved = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/transfer #{}=#{}", hull.0, o.0),
    );
    assert_eq!(title_of(rig, hull), Some(PrincipalId::Object(o)), "{moved}");
    put(rig, hull, bay_room(&s_row), true);
    seed_custody(rig, hull, s, ObjId(s_row.hull));
    (s, s_row, o, hull)
}

/// **36a P1-2 — custody caps the owner, and the berth binds to its carrier.** Owner O's
/// escort sits in salvor S's bay under S's live custody. O holds no right at the bay (no
/// berth, no cancel); S acts as the salvor alone (berths her out of S's bay, orders her unjam,
/// may neither strip nor scrap). Once the claim is vacated, O's title stands again and O
/// cancels S's order, but the berth still names S's carrier: O cannot stow her into S's bay
/// nor strip a part into S's hold, and S, with no right left, cannot stow her either.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b36a_3_a_custody_caps_the_owner_and_binds_the_berth() {
    let mut rig = stocked();
    let (s, s_row, o, hull) = salvor_holds_owners_hull(&mut rig);
    let _ = jam_cannon(&mut rig, hull);
    let workshop = station_room(&rig.tw, CERES, C14);

    // O, the title holder, holds no right while S's custody is live.
    put(&mut rig, o, workshop, false);
    let quote = run(&mut rig.tw, o, &format!("+bay #{}", hull.0));
    assert!(quote.contains("You hold no right"), "{quote}");
    let refused = run(&mut rig.tw, o, &format!("+bay/berth #{}", hull.0));
    assert!(refused.contains("no right"), "{refused}");

    // S, the salvor, berths her out of S's own bay and orders the repair; S neither strips nor
    // scraps.
    put(&mut rig, s, workshop, false);
    let quote = run(&mut rig.tw, s, &format!("+bay #{}", hull.0));
    assert!(quote.contains("Your right: salvage custody"), "{quote}");
    let berthed = run(&mut rig.tw, s, &format!("+bay/berth #{}", hull.0));
    assert!(berthed.contains("berthed in the workshop"), "{berthed}");
    let refused = run(&mut rig.tw, s, &format!("+bay/strip #{}=cannon", hull.0));
    assert!(refused.contains("does not reach taking parts"), "{refused}");
    let refused = run(&mut rig.tw, s, &format!("+bay/scrap #{}", hull.0));
    assert!(
        refused.contains("Only the hull's title holder"),
        "{refused}"
    );
    let taken = run(&mut rig.tw, s, &format!("+bay/order #{}=unjam", hull.0));
    assert!(taken.contains("taken in the workshop"), "{taken}");
    let refused = run(&mut rig.tw, o, &format!("+bay/cancel #{}", hull.0));
    assert!(refused.contains("no right"), "{refused}");
    assert_eq!(order_of(&rig, hull).status, OrderStatus::Open);

    // The claim is vacated: O's title stands again, S's right is gone.
    vacate_custody(&mut rig, hull);
    // (The cancel's one line goes to the order's requester, S.)
    let _ = run(&mut rig.tw, o, &format!("+bay/cancel #{}", hull.0));
    assert_eq!(order_of(&rig, hull).status, OrderStatus::Cancelled);
    let refused = run(&mut rig.tw, s, &format!("+bay/stow #{}", hull.0));
    assert!(refused.contains("no right"), "{refused}");
    // ... but the berth still names S's carrier, which serves O nothing.
    let refused = run(&mut rig.tw, o, &format!("+bay/stow #{}", hull.0));
    assert!(refused.contains("not yours to use"), "{refused}");
    assert!(!in_bay(&rig, hull, bay_room(&s_row)));
    let refused = run(&mut rig.tw, o, &format!("+bay/strip #{}=cannon", hull.0));
    assert!(refused.contains("not yours to use"), "{refused}");
    let record = bay_record(&rig, hull);
    assert!(
        record.berth.is_some() && record.owed.is_none(),
        "{record:?}"
    );
    let cannon = component_id(&unit(&rig, hull), "cannon");
    assert_ne!(
        assess(&rig, hull).component_conditions[&cannon],
        ComponentCondition::Missing
    );
}

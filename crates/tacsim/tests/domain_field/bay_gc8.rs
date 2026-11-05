//! **Doc 36 cut GC.8a — the refit run (story c) and the bay's rights**, over the composed world
//! (`driving`'s keyed rig), through the production command paths.
//!
//! The story: a First Captain leases Occator's escort and a prospector rig into their Dray-P at
//! Ceres and sails to Stickney Yards. The escort comes in with its engine destroyed and its
//! cannon jammed (a staff fixture standing in for GC.2's component outcomes), the rig with its
//! chassis armour down.
//!
//! 1. In the carrier's bay the escort's quote does the unjamming and names the rebuilt engine as
//!    workshop work.
//! 2. `+bay/berth` takes it out of the bay into the Service Counter (P11, Stickney Yards'
//!    workshop): its location is the room, its slot is free.
//! 3. `+bay/order …=all` escrows the tariff's fee and holds the hull; a W-clock freeze stops
//!    the work; resumed, it finishes on the IC clock, a new engine fitted and the cannon cleared.
//! 4. The rig is armour-patched in the carrier's bay at twice the workshop's time.
//! 5. `+bay/stow` puts the escort back in its slot, unheld, and the fee is Stickney Yards'.
//!
//! GC.8b adds the rearm to the run: the escort's magazines come home empty, the captain buys
//! 105 mm HE and 7.62 mm ball at the Co-op chandlery before sailing, and the workshop order
//! draws them — and the work's `industrial_parts` — from the Dray's hold at completion. Then
//! two gates of their own: a stripped part's provenance and a refused hot part, and scrap.
//!
//! **The hot part is an unwired seam** (doc 36a §8, lane 4). `strip::vacate_claim`, which flags
//! a part hot, has no production caller: GC.6's vacating contest never reaches it, because a
//! salvor's custody reaches no strip, so no part comes off under a claim in play. The hot-part
//! gate below flags its lot by staff hand through that seam, and proves only the refusal of a
//! part so flagged, not any way a player makes one.

use mush_core::{GOD, ObjId};
use mush_economy::VaultId;
use mush_inventory::record::EndpointRef;
use mush_tactics::field_ops::ComponentCondition;
use mush_tactics::{CellAddr, UnitMutationOp, UnitMutationPlan};
use tacsim::port::HullRow;
use tacsim::port::bay_work::{BayHullState, FacilityClass, OrderStatus};

use crate::driving_rig::{
    Rig, at_counter, board, cid, drain, file_and_launch, fire, hold_pool, hold_vault_of, land_all,
    land_quietly, load, port_map, prospector, rigs_of, run, seed_hold, staff_hand, station_room,
    stocked, store, to_ic, typed,
};

/// **Buy `qty` of `good` from `vendor` at Ceres** through the market's own quote, accept and
/// draw, into the buyer's chartered hold (GC.7's path).
pub(crate) fn buy(rig: &mut Rig, pilot: ObjId, vendor: &str, good: &str, qty: u64) {
    fn market(rig: &Rig) -> &mush_market::MarketStore {
        rig.tw
            .world()
            .module_state::<mush_market::MarketStore>()
            .expect("the market")
    }
    let vault = port_map(&rig.tw, "ceres_port").vendors[&cid(vendor)].vault;
    let listing = market(rig)
        .lot_listing_of(EndpointRef(VaultId(vault)), &cid(good))
        .unwrap_or_else(|| panic!("{vendor} lists {good}"));
    let quoted = run(
        &mut rig.tw,
        pilot,
        &format!("+market/quote {listing}={qty}"),
    );
    assert!(quoted.contains("Quote #"), "{quoted}");
    let quote = market(rig)
        .quotes_of_buyer(&mush_org::PrincipalId::Object(pilot))
        .map(|q| q.id)
        .max()
        .expect("a quote");
    let accepted = run(&mut rig.tw, pilot, &format!("+market/accept {quote}"));
    let draw = drain(&mut rig.tw);
    assert!(!draw.is_empty(), "{accepted}");
    land_all(rig, &draw);
    for _ in 0..30 {
        if market(rig)
            .quote(quote)
            .is_some_and(|q| q.drawn_at.is_some())
        {
            return;
        }
        let (_, queued) = fire(rig);
        land_quietly(rig, &queued);
    }
    panic!("the {good} was not drawn into the hold");
}

/// A part's magazine, `(current, max)`.
pub(crate) fn magazine(
    u: &mush_tactics::TacticalUnit,
    part: &mush_tactics::ComponentId,
) -> (i64, i64) {
    let cell = u
        .cell(&CellAddr::Component {
            component: part.clone(),
            cell: mush_tactics::CellId::new("ammo").expect("the ammo cell"),
        })
        .expect("a magazine");
    (cell.current, cell.max)
}

/// The magazine of `part` emptied.
pub(crate) fn empty_magazine(part: &mush_tactics::ComponentId) -> UnitMutationOp {
    UnitMutationOp::SetCell {
        addr: CellAddr::Component {
            component: part.clone(),
            cell: mush_tactics::CellId::new("ammo").expect("the ammo cell"),
        },
        value: 0,
    }
}

pub(crate) const STICKNEY: &str = "stickney_port";

pub(crate) fn order_of(rig: &Rig, hull: ObjId) -> tacsim::port::bay_work::BayOrder {
    match store(&rig.tw).bay_hull(hull) {
        Some(BayHullState::Present(row)) => row.order.clone().expect("an order"),
        other => panic!("the bay record: {other:?}"),
    }
}

pub(crate) fn unit(rig: &Rig, hull: ObjId) -> mush_tactics::TacticalUnit {
    mush_tactics::module::unit_of(rig.tw.world(), hull)
        .cloned()
        .expect("a unit")
}

pub(crate) fn section_id(u: &mush_tactics::TacticalUnit, local: &str) -> mush_tactics::SectionId {
    u.sections()
        .map(|(id, _)| id.clone())
        .find(|id| id.as_str() == local || id.as_str().ends_with(&format!(".{local}")))
        .unwrap_or_else(|| panic!("a {local} section"))
}

pub(crate) fn component_id(
    u: &mush_tactics::TacticalUnit,
    local: &str,
) -> mush_tactics::ComponentId {
    u.components()
        .map(|(id, _)| id.clone())
        .find(|id| id.as_str() == local || id.as_str().ends_with(&format!(".{local}")))
        .unwrap_or_else(|| panic!("a {local} part"))
}

/// Staff damage through the unit's one plan path.
pub(crate) fn damage(rig: &mut Rig, hull: ObjId, ops: Vec<UnitMutationOp>) {
    let u = unit(rig, hull);
    let plan = UnitMutationPlan::new(ops, Some(u.revision()), None).expect("a plan");
    staff_hand(rig, |ctx| {
        mush_tactics::module::apply_plan(ctx, hull, &plan).expect("the damage lands");
    });
}

/// Every cell of `section` down to its least: the section destroyed.
pub(crate) fn wreck_section(
    u: &mush_tactics::TacticalUnit,
    section: &mush_tactics::SectionId,
) -> Vec<UnitMutationOp> {
    u.cells()
        .filter(|(addr, _)| {
            matches!(addr, CellAddr::Section { section: s, .. } | CellAddr::Facet { section: s, .. } if s == section)
        })
        .map(|(addr, cell)| UnitMutationOp::SetCell {
            addr: addr.clone(),
            value: cell.min,
        })
        .collect()
}

/// `points` off the first section cell of `section` that has them.
pub(crate) fn dent(
    u: &mush_tactics::TacticalUnit,
    section: &mush_tactics::SectionId,
    points: i64,
) -> UnitMutationOp {
    let (addr, cell) = u
        .cells()
        .find(|(addr, cell)| {
            matches!(addr, CellAddr::Section { section: s, .. } if s == section)
                && cell.current - cell.min >= points
        })
        .expect("a section cell with the points");
    UnitMutationOp::SetCell {
        addr: addr.clone(),
        value: cell.current - points,
    }
}

/// **`hull` worn** (doc 36a P2-4): every section cell set to a quarter of its points, a hull
/// brought back shot up, its sections standing and its parts salvageable. (Doc 39 cut IN.8 took
/// the 100 kg a point that once made a whole escort scrap to 299 t; a worn one now yields a few
/// hundred kilograms of its own structure.)
pub(crate) fn worn(rig: &mut Rig, hull: ObjId) {
    let u = unit(rig, hull);
    let ops: Vec<UnitMutationOp> = u
        .cells()
        .filter(|(addr, cell)| {
            matches!(addr, CellAddr::Section { .. } | CellAddr::Facet { .. })
                && cell.current - cell.min > 4
        })
        .map(|(addr, cell)| UnitMutationOp::SetCell {
            addr: addr.clone(),
            value: cell.min + (cell.current - cell.min) / 4,
        })
        .collect();
    damage(rig, hull, ops);
}

pub(crate) fn mint(rig: &mut Rig, pilot: ObjId) {
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

/// The Dray-P with the escort in slot 1 and a prospector rig in slot 2, still at Ceres.
pub(crate) fn leased(rig: &mut Rig) -> (ObjId, HullRow, ObjId, ObjId) {
    let (pilot, row) = prospector(rig, "Pilot", 1, "Salt Moth");
    mint(rig, pilot);
    at_counter(rig, pilot);
    let signed = run(&mut rig.tw, pilot, "+tv/lease escort go");
    assert!(signed.contains("leases you OWP-ES-01"), "{signed}");
    let second = run(&mut rig.tw, pilot, "+tv/lease go");
    assert!(second.contains("leases you OWP-TV-"), "{second}");
    let rigs = rigs_of(&rig.tw, pilot);
    assert_eq!(rigs.len(), 2, "two rigs in the bay");
    (pilot, row, rigs[0], rigs[1])
}

/// **Staff put `who` in `room`**: into a hull's interior space (`interior`), or out of any
/// space into a plain port room.
pub(crate) fn put(rig: &mut Rig, who: ObjId, room: ObjId, interior: bool) {
    staff_hand(rig, |ctx| {
        if interior {
            mush_sim::place_interior(ctx, who, room, None).expect("into the space");
        } else {
            let _ = mush_sim::unplace(ctx, who, "staff");
            ctx.w.move_to(who, room).expect("into the room");
        }
    });
}

/// The hull's bay room.
pub(crate) fn bay_room(row: &HullRow) -> ObjId {
    ObjId(row.rooms[&crate::driving_rig::cid("bay")])
}

pub(crate) fn in_bay(rig: &Rig, tv: ObjId, room: ObjId) -> bool {
    matches!(
        crate::driving_rig::parent(&rig.tw, tv),
        mush_sim::data::SpatialParent::Interior { space, .. } if space == room
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gc8_the_refit_run_at_stickney_and_an_armour_patch_in_the_bay() {
    let mut rig = stocked();
    let (pilot, row, escort, prospect) = leased(&mut rig);
    let hull = ObjId(row.hull);
    let bay = bay_room(&row);
    let vault = hold_vault_of(&rig.tw, &row);

    // ---- GC.8b: a load of munitions bought at the Co-op chandlery, into the Dray's hold ----
    let u = unit(&rig, escort);
    let (cannon, gun) = (component_id(&u, "cannon"), component_id(&u, "machine_gun"));
    let he_rounds = u64::try_from(magazine(&u, &cannon).1).expect("a magazine");
    let ball_boxes = u64::try_from(magazine(&u, &gun).1)
        .expect("a magazine")
        .div_ceil(100);
    buy(&mut rig, pilot, "coop", "ammo_he_105", he_rounds);
    buy(&mut rig, pilot, "coop", "ammo_ball_762", ball_boxes);
    assert_eq!(hold_pool(&mut rig.tw, vault, "ammo_he_105"), he_rounds);
    assert_eq!(hold_pool(&mut rig.tw, vault, "ammo_ball_762"), ball_boxes);
    // The work's industrial parts ride in the hold too (the yard sells them at Stickney; GC.7's
    // gate covers the buying, so staff stock them here).
    seed_hold(&mut rig, &row, "industrial_parts", 40, 9_700);
    // Doc 39 cut IN.1: each hull's first repair opens its bale with a tonne of alloys too.
    seed_hold(&mut rig, &row, "structural_alloys", 2, 9_701);

    // ---- To Stickney Yards ----
    load(&mut rig, &row, 60, 40, 9_500);
    board(&mut rig, pilot);
    let t = file_and_launch(&mut rig, pilot, hull, "+ship/undock stickney");
    let _ = to_ic(&mut rig, t.plan.depart_ic + t.plan.total_s);
    let _ = to_ic(&mut rig, t.plan.depart_ic + t.plan.total_s + 3_600);
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == STICKNEY
        ),
        "docked at Stickney Yards"
    );

    // ---- The casualty: the engine destroyed, the cannon jammed, both magazines spent; the
    // rig's armour down ----
    let u = unit(&rig, escort);
    let engine = section_id(&u, "engine");
    let mut ops = wreck_section(&u, &engine);
    ops.push(UnitMutationOp::SetComponentCondition {
        component: cannon.clone(),
        condition: ComponentCondition::Jammed
            .to_condition_id()
            .expect("jammed"),
    });
    ops.push(empty_magazine(&cannon));
    ops.push(empty_magazine(&gun));
    damage(&mut rig, escort, ops);
    let p = unit(&rig, prospect);
    let chassis = section_id(&p, "chassis");
    damage(&mut rig, prospect, vec![dent(&p, &chassis, 50)]);

    // (1) In the carrier's bay: the unjam is offered, the engine is a workshop's.
    put(&mut rig, pilot, bay, true);
    let quote = run(&mut rig.tw, pilot, &format!("+bay #{}", escort.0));
    assert!(quote.contains("stands in a carrier bay"), "{quote}");
    assert!(quote.contains("unjam"), "{quote}");
    assert!(
        quote.contains(&format!("rebuild {}", engine.as_str()))
            && quote.contains("needs a workshop"),
        "{quote}"
    );
    assert!(quote.contains("Your right: lessee"), "{quote}");
    let refused = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/order #{}=rebuild", escort.0),
    );
    assert!(refused.contains("needs a workshop"), "{refused}");

    // (2) Berthed at the Service Counter: the location is the workshop, the slot is free.
    let workshop = station_room(&rig.tw, STICKNEY, "room.p11_service");
    put(&mut rig, pilot, workshop, false);
    let berthed = run(&mut rig.tw, pilot, &format!("+bay/berth #{}", escort.0));
    assert!(berthed.contains("berthed in the workshop"), "{berthed}");
    assert_eq!(
        rig.tw.world().get(escort).and_then(|o| o.location),
        Some(workshop)
    );
    assert!(!in_bay(&rig, escort, bay), "the slot is free");
    let moved = run(
        &mut rig.tw,
        GOD,
        &format!("@teleport #{}=#{}", escort.0, bay.0),
    );
    assert!(moved.contains("held in the bay"), "{moved}");
    assert_eq!(
        rig.tw.world().get(escort).and_then(|o| o.location),
        Some(workshop),
        "a berthed hull is held against ordinary moves"
    );

    // (3) The workshop quote and the order.
    let quote = run(&mut rig.tw, pilot, &format!("+bay #{}", escort.0));
    assert!(quote.contains("stands in a workshop"), "{quote}");
    // Doc 39 cut IN.4: the floor, Stickney's four berths at 1,228.77 cr a day each, over
    // the work's hours.
    let (here, priced) =
        tacsim::port::bay_work::quote_here(rig.tw.world(), escort).expect("a quote");
    let (cost, berths) = here
        .cost
        .expect("Stickney's workshop is priced by the model");
    assert_eq!(berths, 4);
    let secs = tacsim::port::bay_work::BayQuote::secs(&priced.jobs, FacilityClass::Workshop);
    let floor = cost.floor_minor_for_secs(u64::try_from(secs).expect("secs"), berths);
    assert!(
        floor > 0 && floor < tacsim::port::bay_work::BayQuote::cost_minor(&priced.jobs),
        "the floor is under the tariff's labour at 1x: {floor}"
    );
    assert!(
        floor_line(&quote).contains(&format!("{}", floor / 100)),
        "+bay prints the floor of {floor} minor: {quote}"
    );
    assert!(
        quote.contains(&format!("rebuild {}", engine.as_str())),
        "{quote}"
    );
    assert!(
        quote.contains(&format!("{he_rounds} ammo_he_105"))
            && quote.contains("From the carrier's hold"),
        "the quote names what the hold supplies: {quote}"
    );
    assert!(
        quote.contains("jammed"),
        "the doll shows the cannon: {quote}"
    );
    let taken = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/order #{}=rebuild,unjam,rearm", escort.0),
    );
    assert!(taken.contains("Order 1 taken in the workshop"), "{taken}");
    let order = order_of(&rig, escort);
    assert_eq!(order.status, OrderStatus::Open);
    assert_eq!(order.class, FacilityClass::Workshop);
    assert_eq!(
        order.munitions_vault,
        Some(vault.0),
        "drawn from the Dray's hold"
    );
    let drawn = |good: &str| {
        order
            .goods
            .iter()
            .find(|g| g.good.as_str() == good)
            .map_or(0, |g| g.units)
    };
    assert_eq!(drawn("ammo_he_105"), he_rounds);
    assert_eq!(drawn("ammo_ball_762"), ball_boxes);
    let parts_t = drawn("industrial_parts");
    assert!(parts_t > 0, "a rebuilt engine consumes parts: {order:?}");
    // Stickney's multiplier is 1x: an 8-hour shell plus its points, a 2-hour unjam.
    assert!(order.due_ic - order.started_ic >= 10 * 3_600, "{order:?}");
    let stow_held = run(&mut rig.tw, pilot, &format!("+bay/stow #{}", escort.0));
    assert!(stow_held.contains("held for work"), "{stow_held}");

    // A W-clock freeze stops the work.
    let _ = run(&mut rig.tw, GOD, "@tacsim/time freeze");
    for _ in 0..3 {
        let now = rig.tw.world().now();
        rig.tw.set_now(now + 86_400);
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
    }
    assert_eq!(
        order_of(&rig, escort).status,
        OrderStatus::Open,
        "frozen, the work waits"
    );
    let _ = run(&mut rig.tw, GOD, "@tacsim/time resume");
    let _ = to_ic(&mut rig, order.due_ic + 60);
    let _ = to_ic(&mut rig, order.due_ic + 600);
    let done = order_of(&rig, escort);
    assert_eq!(done.status, OrderStatus::Done, "{done:?}");
    let after = unit(&rig, escort);
    let assessment = tacsim::field_ops::assess_tacsim_unit(&after, None).expect("assessed");
    assert_eq!(
        assessment.section_conditions[&engine],
        mush_tactics::field_ops::SectionCondition::Intact,
        "a new engine"
    );
    assert_eq!(
        assessment.component_conditions[&cannon],
        ComponentCondition::Intact,
        "the cannon cleared"
    );
    // GC.8b: the magazines full, drawn from the hold, all of it.
    for part in [&cannon, &gun] {
        let (current, max) = magazine(&after, part);
        assert_eq!(current, max, "{part} rearmed");
    }
    assert_eq!(hold_pool(&mut rig.tw, vault, "ammo_he_105"), 0);
    assert_eq!(hold_pool(&mut rig.tw, vault, "ammo_ball_762"), 0);
    assert_eq!(
        hold_pool(&mut rig.tw, vault, "industrial_parts"),
        40 - parts_t,
        "the engine's parts"
    );

    // (4) The rig's armour in the carrier's bay, at twice the time.
    put(&mut rig, pilot, bay, true);
    let patch = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/order #{}=armor", prospect.0),
    );
    assert!(patch.contains("taken in the carrier bay"), "{patch}");
    let order = order_of(&rig, prospect);
    assert_eq!(order.class, FacilityClass::CarrierBay);
    let workshop_secs: i64 = order.jobs.iter().map(|j| j.secs).sum();
    assert_eq!(order.due_ic - order.started_ic, 2 * workshop_secs);
    // The draw of its parts lands on the due fire, the work on the next.
    let _ = to_ic(&mut rig, order.due_ic + 60);
    let _ = to_ic(&mut rig, order.due_ic + 600);
    assert_eq!(order_of(&rig, prospect).status, OrderStatus::Done);
    assert_eq!(
        hold_pool(&mut rig.tw, vault, "industrial_parts"),
        40 - parts_t - 1,
        "fifty points of armour take a tonne of parts from the hold"
    );

    // (5) Stowed back in its slot, unheld; the fee is the operator's.
    put(&mut rig, pilot, workshop, false);
    let stowed = run(&mut rig.tw, pilot, &format!("+bay/stow #{}", escort.0));
    assert!(stowed.contains("stowed back"), "{stowed}");
    assert!(in_bay(&rig, escort, bay), "back in the bay");
    assert!(!tacsim::port::bay_work::held(rig.tw.world(), escort));
    // It fields again: unheld, whole and armed.
    let fit = tacsim::field_ops::assess_tacsim_unit(&unit(&rig, escort), None).expect("assessed");
    assert_eq!(
        fit.state,
        mush_tactics::field_ops::OperationalState::Operational,
        "{fit:?}"
    );
    let contract = mush_economy::ContractId(done.contract.expect("a fee contract at port"));
    for _ in 0..4 {
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
    }
    let state = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| e.snapshot().contract(contract).map(|c| c.state))
        .expect("the fee contract");
    assert!(
        matches!(
            state,
            mush_economy::ContractState::Active | mush_economy::ContractState::Settled { .. }
        ),
        "{state:?}"
    );
}

/// **The rights** (Q-36-33) at Ceres' workshop (C14, Occator's): a stranger may do nothing; the
/// lessee berths and repairs; a holder of Occator's `unit.maintain_refit` repairs too.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gc8_who_may_have_a_hull_worked_on() {
    let mut rig = stocked();
    let (pilot, row, escort, _) = leased(&mut rig);
    let bay = bay_room(&row);
    let u = unit(&rig, escort);
    let cannon = component_id(&u, "cannon");
    damage(
        &mut rig,
        escort,
        vec![UnitMutationOp::SetComponentCondition {
            component: cannon,
            condition: ComponentCondition::Jammed
                .to_condition_id()
                .expect("jammed"),
        }],
    );
    // A stranger in the bay holds no right.
    let stranger = crate::driving_rig::pilot(&mut rig, "Stranger", 2);
    put(&mut rig, stranger, bay, true);
    let quote = run(&mut rig.tw, stranger, &format!("+bay #{}", escort.0));
    assert!(quote.contains("You hold no right"), "{quote}");
    let refused = run(
        &mut rig.tw,
        stranger,
        &format!("+bay/order #{}=unjam", escort.0),
    );
    assert!(refused.contains("no right"), "{refused}");
    // The lessee's right: repair and rearm, so the built refit bay is not theirs.
    put(&mut rig, pilot, bay, true);
    let quote = run(&mut rig.tw, pilot, &format!("+bay #{}", escort.0));
    assert!(
        quote.contains("Your right: lessee (repair and rearm)"),
        "{quote}"
    );
    // Doc 39 cut IN.4: docked at Ceres, the bay's quote prints the port workshop's floor, the
    // unjam's two IC hours of a Ceres berth (1,228.77 cr a day over two) = 51.20, billed 52.
    assert!(floor_line(&quote).contains("52"), "{quote}");
    assert!(!tacsim::port::bay_work::permits(
        tacsim::port::bay_work::Right::Lessee,
        tacsim::port::bay_work::Act::Refit
    ));
    // A holder of Occator's `unit.maintain_refit`: the grant is read through Org.
    let mechanic = crate::driving_rig::pilot(&mut rig, "Mechanic", 3);
    let enrolled = run(&mut rig.tw, mechanic, "+coop/enrol Grey Heron");
    assert!(enrolled.contains("now the Grey Heron"), "{enrolled}");
    let occator = port_map(&rig.tw, "ceres_port").orgs[&crate::driving_rig::cid("occator")].org;
    let granted = run(
        &mut rig.tw,
        GOD,
        &format!("@org/grant #{}=#{occator} unit.maintain_refit", mechanic.0),
    );
    let _ = granted;
    mint(&mut rig, mechanic);
    put(&mut rig, mechanic, bay, true);
    let quote = run(&mut rig.tw, mechanic, &format!("+bay #{}", escort.0));
    assert!(
        quote.contains("Your right: a maintain/refit grant"),
        "{granted}\n{quote}"
    );
    let taken = run(
        &mut rig.tw,
        mechanic,
        &format!("+bay/order #{}=unjam", escort.0),
    );
    assert!(taken.contains("taken in the carrier bay"), "{taken}");
    // A second order on a held hull is refused, and the lessee may cancel it.
    let again = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/order #{}=unjam", escort.0),
    );
    assert!(again.contains("held for work"), "{again}");
    let cancelled = run(&mut rig.tw, pilot, &format!("+bay/cancel #{}", escort.0));
    let _ = cancelled;
    assert_eq!(order_of(&rig, escort).status, OrderStatus::Cancelled);
    assert!(!tacsim::port::bay_work::held(rig.tw.world(), escort));
}

/// **A mechanic with Occator's `unit.maintain_refit`**, enrolled, funded, standing in `room`.
/// The `floor` line of a `+bay` quote (doc 39 cut IN.4).
pub(crate) fn floor_line(quote: &str) -> &str {
    quote
        .lines()
        .find(|line| line.trim_start().starts_with("floor "))
        .unwrap_or_else(|| panic!("no floor line: {quote}"))
}

pub(crate) fn mechanic(rig: &mut Rig, room: ObjId) -> ObjId {
    let mechanic = crate::driving_rig::pilot(rig, "Mechanic", 3);
    let enrolled = run(&mut rig.tw, mechanic, "+coop/enrol Grey Heron");
    assert!(enrolled.contains("now the Grey Heron"), "{enrolled}");
    let occator = port_map(&rig.tw, "ceres_port").orgs[&cid("occator")].org;
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@org/grant #{}=#{occator} unit.maintain_refit", mechanic.0),
    );
    mint(rig, mechanic);
    put(rig, mechanic, room, false);
    mechanic
}

/// A command, its submissions landed, then fires until `hull`'s record owes nothing.
pub(crate) fn settled(rig: &mut Rig, who: ObjId, line: &str, hull: ObjId) -> String {
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

pub(crate) fn bay_handles(rig: &Rig) -> tacsim::port::bay_work::act::BayActHandles {
    rig.tw
        .world()
        .module_state::<tacsim::port::bay_work::act::BayActHandles>()
        .cloned()
        .expect("the bay's handles")
}

pub(crate) fn provenance(
    rig: &mut Rig,
    lot: mush_inventory::LotId,
) -> tacsim::port::bay_work::act::PartProvenance {
    let handles = bay_handles(rig);
    typed(&mut rig.tw, &handles.parts, &lot).expect("the part's provenance")
}

pub(crate) fn lot_record(rig: &mut Rig, lot: mush_inventory::LotId) -> Option<mush_inventory::Lot> {
    let inventory = bay_handles(rig).inventory.expect("the inventory");
    typed(&mut rig.tw, &inventory.lots, &lot)
}

/// **GC.8b's provenance gate** (Q-36-37, Q-F6) at Ceres' workshop (C14, Occator's). The lessee
/// berths the escort; a holder of Occator's `unit.maintain_refit` strips its machine gun and
/// its cannon. Each becomes a one-unit lot of its part in the Dray's hold carrying its parent
/// hull, its slot (now `Missing`), its condition and its claim reference. The machine gun's
/// claim is vacated (the staff fixture standing in for GC.5/GC.6's contest), so it is hot and
/// the bay refuses to fit it; the clean cannon is fitted back from its lot, which the work
/// consumes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gc8b_a_stripped_part_carries_its_provenance_and_a_hot_part_is_refused() {
    let mut rig = stocked();
    let (pilot, row, escort, _) = leased(&mut rig);
    let vault = hold_vault_of(&rig.tw, &row);
    let workshop = station_room(&rig.tw, "ceres_port", "room.c14_dockmaster");
    put(&mut rig, pilot, workshop, false);
    let berthed = run(&mut rig.tw, pilot, &format!("+bay/berth #{}", escort.0));
    assert!(berthed.contains("berthed in the workshop"), "{berthed}");
    // The lessee may not strip.
    let refused = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/strip #{}=machine_gun", escort.0),
    );
    assert!(refused.contains("does not reach"), "{refused}");
    let mechanic = mechanic(&mut rig, workshop);
    let u = unit(&rig, escort);
    let (cannon, gun) = (component_id(&u, "cannon"), component_id(&u, "machine_gun"));

    // (1) The machine gun comes off into the hold, with its provenance.
    let out = settled(
        &mut rig,
        mechanic,
        &format!("+bay/strip #{}=machine_gun", escort.0),
        escort,
    );
    assert!(out.contains("goes to the carrier's hold"), "{out}");
    let gun_lot = tacsim::port::bay_work::act::lot_id(escort, 1, 0).expect("the lot");
    let lot = lot_record(&mut rig, gun_lot).expect("the part's lot");
    assert_eq!(lot.endpoint(), EndpointRef(vault), "in the Dray's hold");
    assert_eq!(lot.commodity().as_str(), "tacsim.part.weapon.mg_coax");
    assert_eq!(lot.quantity(), 1);
    let part = provenance(&mut rig, gun_lot);
    let strip = part.stripped().expect("a stripped part").clone();
    assert_eq!(strip.parent_hull, escort);
    assert_eq!(strip.slot, gun);
    assert_eq!(part.condition, ComponentCondition::Intact);
    assert_eq!(
        strip.claim, None,
        "no salvage claim: the grant holder stripped it"
    );
    assert!(!part.is_hot());
    assert_eq!(strip.stripped_by, mechanic);
    let assessment =
        tacsim::field_ops::assess_tacsim_unit(&unit(&rig, escort), None).expect("assessed");
    assert_eq!(
        assessment.component_conditions[&gun],
        ComponentCondition::Missing,
        "the slot reads missing"
    );

    // (2) The cannon too.
    let _ = settled(
        &mut rig,
        mechanic,
        &format!("+bay/strip #{}=cannon", escort.0),
        escort,
    );
    let cannon_lot = tacsim::port::bay_work::act::lot_id(escort, 2, 0).expect("the lot");
    assert_eq!(
        provenance(&mut rig, cannon_lot)
            .stripped()
            .map(|s| s.slot.clone()),
        Some(cannon.clone())
    );

    // (3) The machine gun's claim is vacated: it is hot, and the bay will not fit it.
    staff_hand(&mut rig, |ctx| {
        tacsim::port::bay_work::strip::vacate_claim(ctx, mechanic, escort, gun_lot, 7, pilot)
            .expect("the claim is vacated");
    });
    for _ in 0..6 {
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
    }
    let part = provenance(&mut rig, gun_lot);
    assert!(part.is_hot(), "{part:?}");
    let refused = run(
        &mut rig.tw,
        mechanic,
        &format!("+bay/fit #{}=machine_gun {gun_lot}", escort.0),
    );
    assert!(refused.contains("That part is hot"), "{refused}");
    assert!(
        lot_record(&mut rig, gun_lot).is_some(),
        "the hot part stays in the hold"
    );

    // (4) The clean cannon is fitted back from its lot; the work consumes the lot.
    let taken = run(
        &mut rig.tw,
        mechanic,
        &format!("+bay/fit #{}=cannon {cannon_lot}", escort.0),
    );
    assert!(taken.contains("taken in the workshop"), "{taken}");
    let order = order_of(&rig, escort);
    // Doc 39 cut IN.2: the fit holds its lot first (one fire), then draws and completes.
    let _ = to_ic(&mut rig, order.due_ic + 60);
    let _ = to_ic(&mut rig, order.due_ic + 600);
    let _ = to_ic(&mut rig, order.due_ic + 1_200);
    let order = order_of(&rig, escort);
    assert_eq!(order.status, OrderStatus::Done, "{order:?}");
    let assessment =
        tacsim::field_ops::assess_tacsim_unit(&unit(&rig, escort), None).expect("assessed");
    assert_eq!(
        assessment.component_conditions[&cannon],
        ComponentCondition::Intact
    );
    assert_eq!(
        assessment.component_conditions[&gun],
        ComponentCondition::Missing
    );
    assert!(
        lot_record(&mut rig, cannon_lot).is_none(),
        "the lot is consumed"
    );
    assert_eq!(provenance(&mut rig, cannon_lot).fitted_into, Some(escort));
}

/// **GC.8b's scrap gate** (Q-36-39) at Ceres' workshop. A spare escort of Occator's pool is
/// titled to the captain and stowed in the Dray's bay; berthed, the title holder scraps it: since
/// doc 39 cut IN.8 its structural yield becomes `structural_alloys` in the operator's (Occator's)
/// stock and each salvageable part a lot with its provenance in its stores, the Dray's hold
/// untouched; the hull is an irrecoverable shell and its berth free. The lessee may not scrap a
/// leased hull; remains are scrapped on the site (GC.6).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gc8b_scrap_breaks_a_workshop_hull_into_alloys_and_parts() {
    let mut rig = stocked();
    let (pilot, row, escort, _) = leased(&mut rig);
    let vault = hold_vault_of(&rig.tw, &row);
    let bay = bay_room(&row);
    let workshop = station_room(&rig.tw, "ceres_port", "room.c14_dockmaster");
    // The leased escort is berthed, freeing its slot for a titled spare.
    put(&mut rig, pilot, workshop, false);
    let berthed = run(&mut rig.tw, pilot, &format!("+bay/berth #{}", escort.0));
    assert!(berthed.contains("berthed in the workshop"), "{berthed}");
    let refused = run(&mut rig.tw, pilot, &format!("+bay/scrap #{}", escort.0));
    assert!(
        refused.contains("Only the hull's title holder"),
        "{refused}"
    );
    let schema = unit(&rig, escort).schema().clone();
    let spare = rig
        .tw
        .world()
        .module_state::<mush_tactics::UnitStore>()
        .expect("the units")
        .units
        .iter()
        .find(|(id, u)| **id != escort && u.schema() == &schema)
        .map(|(id, _)| *id)
        .expect("a spare escort in Occator's pool");
    let moved = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/transfer #{}=#{}", spare.0, pilot.0),
    );
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .and_then(|e| e.snapshot().asset(spare).map(|a| a.title_owner.clone())),
        Some(mush_org::PrincipalId::Object(pilot)),
        "{moved}"
    );
    put(&mut rig, spare, bay, true);
    let berthed = run(&mut rig.tw, pilot, &format!("+bay/berth #{}", spare.0));
    assert!(berthed.contains("berthed in the workshop"), "{berthed}");
    let before = tacsim::field_ops::assess_tacsim_unit(&unit(&rig, spare), None).expect("assessed");
    let salvageable = before
        .component_conditions
        .values()
        .filter(|c| c.is_salvageable())
        .count();

    let out = settled(&mut rig, pilot, &format!("+bay/scrap #{}", spare.0), spare);
    assert!(out.contains("is broken up"), "{out}");
    let stores = port_map(&rig.tw, "ceres_port").vendors[&cid("occator")].vault;
    let alloys = lot_record(
        &mut rig,
        tacsim::port::bay_work::act::lot_id(spare, 1, 0).expect("the lot"),
    )
    .expect("the alloys' lot");
    assert_eq!(alloys.commodity().as_str(), "structural_alloys");
    assert_eq!(alloys.quantity(), 1, "1,539 kg: a tonne, {alloys:?}");
    assert_eq!(alloys.endpoint(), EndpointRef(VaultId(stores)));
    assert_eq!(
        hold_pool(&mut rig.tw, vault, "structural_alloys"),
        0,
        "nothing in the Dray's hold"
    );
    let mut parts = 0;
    for row in 2..=salvageable + 1 {
        let lot = tacsim::port::bay_work::act::lot_id(spare, 1, row).expect("the lot");
        let Some(found) = lot_record(&mut rig, lot) else {
            continue;
        };
        assert!(found.commodity().as_str().starts_with("tacsim.part."));
        assert_eq!(found.endpoint(), EndpointRef(VaultId(stores)));
        let part = provenance(&mut rig, lot);
        assert_eq!(part.parent_hull(), Some(spare));
        assert!(part.stripped().is_some_and(|s| s.scrapped));
        parts += 1;
    }
    assert!(parts > 0, "salvageable parts become lots: {out}");
    let after = tacsim::field_ops::assess_tacsim_unit(&unit(&rig, spare), None).expect("assessed");
    assert_eq!(
        after.state,
        mush_tactics::field_ops::OperationalState::Irrecoverable
    );
    match store(&rig.tw).bay_hull(spare) {
        Some(BayHullState::Present(r)) => {
            assert!(r.scrapped_ic.is_some() && r.berth.is_none(), "{r:?}");
        }
        other => panic!("the bay record: {other:?}"),
    }
    let again = run(&mut rig.tw, pilot, &format!("+bay/scrap #{}", spare.0));
    assert!(again.contains("has been scrapped"), "{again}");
    // Remains are scrapped where they lie on the site (doc 36 cut GC.6, `field_gc6.rs`).
}

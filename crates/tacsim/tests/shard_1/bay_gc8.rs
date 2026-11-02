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
//! Rearming from munitions lots is GC.8b's, so this run's magazines are not touched.

use mush_core::{GOD, ObjId};
use mush_tactics::field_ops::ComponentCondition;
use mush_tactics::{CellAddr, UnitMutationOp, UnitMutationPlan};
use tacsim::port::HullRow;
use tacsim::port::bay_work::{BayHullState, FacilityClass, OrderStatus};

use crate::driving::{
    Rig, at_counter, board, file_and_launch, fire, land_quietly, load, port_map, prospector,
    rigs_of, run, staff_hand, station_room, stocked, store, to_ic,
};

const STICKNEY: &str = "stickney_port";

fn order_of(rig: &Rig, hull: ObjId) -> tacsim::port::bay_work::BayOrder {
    match store(&rig.tw).bay_hull(hull) {
        Some(BayHullState::Present(row)) => row.order.clone().expect("an order"),
        other => panic!("the bay record: {other:?}"),
    }
}

fn unit(rig: &Rig, hull: ObjId) -> mush_tactics::TacticalUnit {
    mush_tactics::module::unit_of(rig.tw.world(), hull)
        .cloned()
        .expect("a unit")
}

fn section_id(u: &mush_tactics::TacticalUnit, local: &str) -> mush_tactics::SectionId {
    u.sections()
        .map(|(id, _)| id.clone())
        .find(|id| id.as_str() == local || id.as_str().ends_with(&format!(".{local}")))
        .unwrap_or_else(|| panic!("a {local} section"))
}

fn component_id(u: &mush_tactics::TacticalUnit, local: &str) -> mush_tactics::ComponentId {
    u.components()
        .map(|(id, _)| id.clone())
        .find(|id| id.as_str() == local || id.as_str().ends_with(&format!(".{local}")))
        .unwrap_or_else(|| panic!("a {local} part"))
}

/// Staff damage through the unit's one plan path.
fn damage(rig: &mut Rig, hull: ObjId, ops: Vec<UnitMutationOp>) {
    let u = unit(rig, hull);
    let plan = UnitMutationPlan::new(ops, Some(u.revision()), None).expect("a plan");
    staff_hand(rig, |ctx| {
        mush_tactics::module::apply_plan(ctx, hull, &plan).expect("the damage lands");
    });
}

/// Every cell of `section` down to its least: the section destroyed.
fn wreck_section(
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
fn dent(
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

/// The Dray-P with the escort in slot 1 and a prospector rig in slot 2, still at Ceres.
fn leased(rig: &mut Rig) -> (ObjId, HullRow, ObjId, ObjId) {
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

/// The hull's bay room.
fn bay_room(row: &HullRow) -> ObjId {
    ObjId(row.rooms[&crate::driving::cid("bay")])
}

fn in_bay(rig: &Rig, tv: ObjId, room: ObjId) -> bool {
    matches!(
        crate::driving::parent(&rig.tw, tv),
        mush_sim::data::SpatialParent::Interior { space, .. } if space == room
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gc8_the_refit_run_at_stickney_and_an_armour_patch_in_the_bay() {
    let mut rig = stocked();
    let (pilot, row, escort, prospect) = leased(&mut rig);
    let hull = ObjId(row.hull);
    let bay = bay_room(&row);

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

    // ---- The casualty: the engine destroyed, the cannon jammed; the rig's armour down ----
    let u = unit(&rig, escort);
    let engine = section_id(&u, "engine");
    let cannon = component_id(&u, "cannon");
    let mut ops = wreck_section(&u, &engine);
    ops.push(UnitMutationOp::SetComponentCondition {
        component: cannon.clone(),
        condition: ComponentCondition::Jammed
            .to_condition_id()
            .expect("jammed"),
    });
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
    assert!(
        quote.contains(&format!("rebuild {}", engine.as_str())),
        "{quote}"
    );
    let taken = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/order #{}=rebuild,unjam", escort.0),
    );
    assert!(taken.contains("Order 1 taken in the workshop"), "{taken}");
    let order = order_of(&rig, escort);
    assert_eq!(order.status, OrderStatus::Open);
    assert_eq!(order.class, FacilityClass::Workshop);
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
    let _ = to_ic(&mut rig, order.due_ic + 600);
    assert_eq!(order_of(&rig, prospect).status, OrderStatus::Done);

    // (5) Stowed back in its slot, unheld; the fee is the operator's.
    put(&mut rig, pilot, workshop, false);
    let stowed = run(&mut rig.tw, pilot, &format!("+bay/stow #{}", escort.0));
    assert!(stowed.contains("stowed back"), "{stowed}");
    assert!(in_bay(&rig, escort, bay), "back in the bay");
    assert!(!tacsim::port::bay_work::held(rig.tw.world(), escort));
    let contract = mush_economy::ContractId(done.contract);
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
    let stranger = crate::driving::pilot(&mut rig, "Stranger", 2);
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
    assert!(!tacsim::port::bay_work::permits(
        tacsim::port::bay_work::Right::Lessee,
        tacsim::port::bay_work::Act::Refit
    ));
    // A holder of Occator's `unit.maintain_refit`: the grant is read through Org.
    let mechanic = crate::driving::pilot(&mut rig, "Mechanic", 3);
    let enrolled = run(&mut rig.tw, mechanic, "+coop/enrol Grey Heron");
    assert!(enrolled.contains("now the Grey Heron"), "{enrolled}");
    let occator = port_map(&rig.tw, "ceres_port").orgs[&crate::driving::cid("occator")].org;
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

//! **Doc 39 cut IN.1 — accounting and identity**, over the composed world (`driving`'s keyed
//! rig), through the production command paths.
//!
//! The story (doc 39 §6, the IN.1 row): a captain's Dray-P sails from Ceres to Stickney Yards
//! with a spare escort of Occator's pool in its bay, titled to the captain; at Stickney a heavy,
//! also the captain's, joins it in the bay. At the Service Counter (P11, Stickney Yards'
//! workshop):
//!
//! 1. `+bay/strip` takes the escort's cannon off: one lot of `tacsim.part.weapon.cannon_105`
//!    (the hardware row its slot names, Q-39-3/35), its line naming it at 2.4 t, and the hold's
//!    load line moves by it (`+ship/hold` lists the mount; the part never pools, so the hold
//!    reads it from the lot tally).
//! 2. The heavy's own cannon is wrecked; `+bay/fit` takes the escort's into it — a part fits
//!    every slot that names its row, whatever the unit.
//! 3. A one-point armour patch on the heavy opens its bale (§4.10): a tonne of alloys and a tonne
//!    of fittings come out of the hold once, and the point's alloys and fittings at the heavy's
//!    own figures go into the work (doc 39 cut IN.8); a second one-point patch draws nothing
//!    from the hold.
//!
//! **Seeds** (doc 43 rule 2.3 #4; doc 36a §8): the spare escort and the heavy reach the captain's
//! title by staff (`@asset/transfer`, `@unit/create` + `@asset/title`) and the bay by staff
//! placement, as GC.8b's scrap gate does; the heavy's wrecked cannon and the armour dents are
//! staff damage through the unit's one plan path; the hold's alloys and fittings are seeded (GC.7's
//! gate covers buying them). Every claim — the strip, the lot, the load line, the fit, the bale's
//! draw — is reached by the player's command.
//!
//! The matter pins (`.mint_origins(` and the closed receipt kinds) are `domain_economy`'s
//! (`mill::sm12a_only_the_clerk_the_foundry_and_the_mill_create_matter`,
//! `mill::in1_the_receipt_kinds_a_mint_may_carry_are_closed`).

use mush_core::{GOD, ObjId};
use mush_inventory::record::EndpointRef;
use mush_tactics::field_ops::ComponentCondition;
use mush_tactics::{CellAddr, UnitMutationOp, UnitMutationPlan};
use tacsim::port::HullRow;
use tacsim::port::bay_work::{Bale, BayHullState, OrderStatus};

use crate::driving_rig::{
    Rig, at_counter, board, cid, drain, file_and_launch, fire, hold_pool, hold_vault_of, land_all,
    land_quietly, load, port_map, prospector, run, seed_hold, staff_hand, station_room, stocked,
    store, to_ic, typed,
};

const STICKNEY: &str = "stickney_port";
const CANNON_PART: &str = "tacsim.part.weapon.cannon_105";

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

/// Staff damage through the unit's one plan path.
fn damage(rig: &mut Rig, hull: ObjId, ops: Vec<UnitMutationOp>) {
    let u = unit(rig, hull);
    let plan = UnitMutationPlan::new(ops, Some(u.revision()), None).expect("a plan");
    staff_hand(rig, |ctx| {
        mush_tactics::module::apply_plan(ctx, hull, &plan).expect("the damage lands");
    });
}

/// One point off the first armoured facet that has it: a one-point armour patch's worth.
pub(crate) fn dent_one(rig: &mut Rig, hull: ObjId) {
    let u = unit(rig, hull);
    let (addr, cell) = u
        .cells()
        .find(|(addr, cell)| matches!(addr, CellAddr::Facet { .. }) && cell.current > cell.min)
        .map(|(addr, cell)| (addr.clone(), *cell))
        .expect("an armoured facet");
    damage(
        rig,
        hull,
        vec![UnitMutationOp::SetCell {
            addr,
            value: cell.current - 1,
        }],
    );
}

/// Staff put `who` in `room`: into a hull's interior space, or into a plain port room.
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

pub(crate) fn record(rig: &Rig, hull: ObjId) -> tacsim::port::bay_work::BayHull {
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

/// The order on `hull`, run to its due time and done.
pub(crate) fn done(rig: &mut Rig, hull: ObjId) -> tacsim::port::bay_work::BayOrder {
    let order = record(rig, hull).order.expect("an order");
    let _ = to_ic(rig, order.due_ic + 60);
    let _ = to_ic(rig, order.due_ic + 600);
    // Doc 39 cut IN.2: a fit holds its lot a fire before it draws.
    let _ = to_ic(rig, order.due_ic + 1_200);
    let order = record(rig, hull).order.expect("an order");
    assert_eq!(order.status, OrderStatus::Done, "{order:?}");
    order
}

fn lot_record(rig: &mut Rig, lot: mush_inventory::LotId) -> Option<mush_inventory::Lot> {
    let handles = rig
        .tw
        .world()
        .module_state::<tacsim::port::bay_work::act::BayActHandles>()
        .cloned()
        .expect("the bay's handles");
    let inventory = handles.inventory.expect("the inventory");
    typed(&mut rig.tw, &inventory.lots, &lot)
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

/// **The captain's Dray-P at Ceres with a titled spare escort in its bay, and a titled heavy**
/// (which has no `logistics`, so no carrier sails with it: staff put it in the bay at Stickney).
pub(crate) fn two_titled_hulls(rig: &mut Rig) -> (ObjId, HullRow, ObjId, ObjId) {
    let (pilot, row) = prospector(rig, "Pilot", 1, "Salt Moth");
    mint(rig, pilot);
    at_counter(rig, pilot);
    let bay = ObjId(row.rooms[&cid("bay")]);
    // A spare escort of Occator's pool, titled to the captain (GC.8b's scrap gate's seed).
    let escort_schema = cid("tacsim.unit.tv_escort");
    let spare = rig
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
        &format!("@asset/transfer #{}=#{}", spare.0, pilot.0),
    );
    assert!(!moved.contains("refused"), "{moved}");
    put(rig, spare, bay, true);
    // A heavy, made and titled by staff.
    let made = run(&mut rig.tw, GOD, "@create Iron Lantern");
    let heavy = made
        .split('#')
        .nth(1)
        .and_then(|rest| {
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()
        })
        .map(ObjId)
        .unwrap_or_else(|| panic!("a thing: {made}"));
    let unit = run(
        &mut rig.tw,
        GOD,
        &format!("@unit/create #{}=tacsim.unit.heavy_mk1", heavy.0),
    );
    assert!(
        mush_tactics::module::unit_of(rig.tw.world(), heavy).is_some(),
        "{unit}"
    );
    let titled = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/title #{}=#{}", heavy.0, pilot.0),
    );
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .and_then(|e| e.snapshot().asset(heavy).map(|a| a.title_owner.clone())),
        Some(mush_org::PrincipalId::Object(pilot)),
        "{titled}"
    );
    (pilot, row, spare, heavy)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn in1_a_cannon_stripped_at_stickney_fits_a_heavy_and_a_patch_opens_a_bale_once() {
    let mut rig = stocked();
    let (pilot, row, escort, heavy) = two_titled_hulls(&mut rig);
    let hull = ObjId(row.hull);
    let vault = hold_vault_of(&rig.tw, &row);
    // The repairs' bale is opened from the hold (staff stock it; GC.7's gate covers buying).
    seed_hold(&mut rig, &row, "structural_alloys", 2, 9_700);
    seed_hold(&mut rig, &row, "industrial_parts", 2, 9_701);

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

    // ---- Both berthed at the Service Counter ----
    put(&mut rig, heavy, ObjId(row.rooms[&cid("bay")]), true);
    let workshop = station_room(&rig.tw, STICKNEY, "room.p11_service");
    put(&mut rig, pilot, workshop, false);
    for berthing in [escort, heavy] {
        let berthed = run(&mut rig.tw, pilot, &format!("+bay/berth #{}", berthing.0));
        assert!(berthed.contains("berthed in the workshop"), "{berthed}");
    }
    let hold_before = run(&mut rig.tw, pilot, "+ship/hold");
    assert!(!hold_before.contains("cannon mount"), "{hold_before}");

    // (1) The escort's cannon comes off as the 105 mm row's part, at 2.4 t.
    let out = settled(
        &mut rig,
        pilot,
        &format!("+bay/strip #{}=cannon", escort.0),
        escort,
    );
    assert!(out.contains("goes to the carrier's hold"), "{out}");
    let lot = tacsim::port::bay_work::act::lot_id(escort, 1, 0).expect("the lot");
    let found = lot_record(&mut rig, lot).expect("the part's lot");
    assert_eq!(found.commodity().as_str(), CANNON_PART);
    assert_eq!(found.quantity(), 1);
    assert_eq!(found.endpoint(), EndpointRef(vault), "in the Dray's hold");
    let part = provenance(&mut rig, lot);
    assert_eq!(part.parent_hull(), Some(escort));
    assert_eq!(
        part.definition.key().id.as_str(),
        "tacsim.weapon.cannon_105",
        "the part is its hardware row, not the escort's slot"
    );
    // The hold's load line moves by the mount's catalogue mass.
    let hold_after = run(&mut rig.tw, pilot, "+ship/hold");
    assert!(
        hold_after.contains("105 mm cannon mount: 2.4 t"),
        "{hold_after}"
    );
    assert_ne!(hold_before, hold_after);
    assert_eq!(
        hold_pool(&mut rig.tw, vault, CANNON_PART),
        0,
        "a part never pools"
    );

    // (2) The heavy's cannon wrecked; the escort's fits it.
    let h = unit(&rig, heavy);
    let cannon = component_id(&h, "cannon");
    damage(
        &mut rig,
        heavy,
        vec![UnitMutationOp::SetComponentCondition {
            component: cannon.clone(),
            condition: ComponentCondition::Destroyed
                .to_condition_id()
                .expect("destroyed"),
        }],
    );
    let taken = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/fit #{}=cannon {lot}", heavy.0),
    );
    assert!(taken.contains("taken in the workshop"), "{taken}");
    let _ = done(&mut rig, heavy);
    let assessment =
        tacsim::field_ops::assess_tacsim_unit(&unit(&rig, heavy), None).expect("assessed");
    assert_eq!(
        assessment.component_conditions[&cannon],
        ComponentCondition::Intact,
        "the escort's cannon is the heavy's now"
    );
    assert!(lot_record(&mut rig, lot).is_none(), "the lot is consumed");
    assert_eq!(provenance(&mut rig, lot).fitted_into, Some(heavy));
    assert!(record(&rig, heavy).bale.is_none(), "a fit opens no bale");

    // (3) A one-point armour patch opens the heavy's bale: a tonne of each good, once; a point
    //     draws the heavy's own figures (doc 39 cut IN.8), rounded up.
    let budget = store(&rig.tw)
        .definitions()
        .budget_of("tacsim.unit.heavy_mk1")
        .cloned()
        .expect("the heavy's budget");
    let (a1, f1) = (budget.structure_kg_for(1), budget.fittings_kg_for(1));
    dent_one(&mut rig, heavy);
    let quote = run(&mut rig.tw, pilot, &format!("+bay #{}", heavy.0));
    assert!(
        quote.contains("1 structural_alloys") && quote.contains("1 industrial_parts"),
        "{quote}"
    );
    assert!(
        quote.contains(&format!(
            "Her bale after it: {} kg of alloys and {} kg of fittings left.",
            1_000 - a1,
            1_000 - f1
        )),
        "{quote}"
    );
    let taken = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/order #{}=armor", heavy.0),
    );
    assert!(taken.contains("taken in the workshop"), "{taken}");
    let first = done(&mut rig, heavy);
    assert_eq!(first.goods.len(), 2, "{first:?}");
    assert_eq!(hold_pool(&mut rig.tw, vault, "structural_alloys"), 1);
    assert_eq!(hold_pool(&mut rig.tw, vault, "industrial_parts"), 1);
    assert_eq!(
        record(&rig, heavy).bale,
        Some(Bale {
            alloys_kg: 1_000 - a1,
            fittings_kg: 1_000 - f1,
        })
    );

    // The second one-point patch draws nothing from the hold: the bale covers it.
    dent_one(&mut rig, heavy);
    let taken = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/order #{}=armor", heavy.0),
    );
    assert!(taken.contains("taken in the workshop"), "{taken}");
    let second = done(&mut rig, heavy);
    assert!(second.goods.is_empty(), "{second:?}");
    assert_eq!(hold_pool(&mut rig.tw, vault, "structural_alloys"), 1);
    assert_eq!(hold_pool(&mut rig.tw, vault, "industrial_parts"), 1);
    assert_eq!(
        record(&rig, heavy).bale,
        Some(Bale {
            alloys_kg: 1_000 - 2 * a1,
            fittings_kg: 1_000 - 2 * f1,
        })
    );
}

/// **The bale's rule** (doc 39 §4.10, Q-39-26), unit by unit: an unopened bale opens with a tonne
/// of each good; a line takes whole tonnes only when it runs short; no kilograms, no draw.
#[test]
fn in1_the_bale_opens_once_and_tops_up_by_whole_tonnes() {
    let need = |alloys_kg, fittings_kg| Bale {
        alloys_kg,
        fittings_kg,
    };
    assert_eq!(Bale::draw(None, need(0, 0)), None);
    let opened = Bale::draw(None, need(0, 10)).expect("a draw");
    assert_eq!((opened.alloys_t, opened.fittings_t), (1, 1));
    assert_eq!(opened.after, need(1_000, 990));
    let again = Bale::draw(Some(opened.after), need(0, 10)).expect("a draw");
    assert_eq!((again.alloys_t, again.fittings_t), (0, 0));
    assert_eq!(again.after, need(1_000, 980));
    // 2,500 kg of fittings on a fresh bale: the opening tonne and two more, 500 kg left.
    let big = Bale::draw(None, need(86, 2_500)).expect("a draw");
    assert_eq!((big.alloys_t, big.fittings_t), (1, 3));
    assert_eq!(big.after, need(914, 500));
}

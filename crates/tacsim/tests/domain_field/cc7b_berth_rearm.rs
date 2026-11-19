//! **Doc 49 unit CC.7 part 2 — a ship's space magazines rearmed at a berth** (Q-49-12), over the
//! composed world (`driving`'s rig), through the production command paths.
//!
//! A ship docked at a port with a workshop is her own bay subject. `+bay/order <ship>=rearm`
//! loads her short magazines with PL.3's space munitions goods: from her hold where it carries
//! them, and the shortfall bought from the workshop's stores at the goods' price, in the order's
//! escrowed fee beside the tariff's labour. The draw counts the rounds' kilograms into the matter
//! census's `loaded`, and its balance holds.

use std::collections::BTreeMap;

use mush_core::ObjId;
use mush_economy::VaultId;
use mush_inventory::LotId;
use tacsim::port::HullRow;
use tacsim::port::bay_work::OrderStatus;
use tacsim::port::bay_work::magazines::{Magazine, SHORT_OF_ROUNDS, magazines};

use crate::bay_gc8::{damage, dent, mint, order_of, section_id, unit};
use crate::driving_rig::{
    Rig, advance_to, board, cid, fire, hold_pool, hold_vault_of, land_quietly, pilot, port_map,
    run, seed_hold, seed_pool, staff_hand, stocked, store, to_ic,
};

/// The census line's terms, by name (`ex0_matter_sinks`').
fn census(rig: &mut Rig) -> BTreeMap<String, i128> {
    let out = run(&mut rig.tw, mush_core::GOD, "@industry/census");
    let line = out
        .lines()
        .find(|l| l.contains(tacsim::port::census::CENSUS_LINE))
        .unwrap_or_else(|| panic!("a census line: {out}"));
    line.split_whitespace()
        .filter_map(|word| word.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.parse().expect("a number")))
        .collect()
}

/// The census once its balance holds still for three reads a few fires apart.
fn steady(rig: &mut Rig) -> BTreeMap<String, i128> {
    let mut before = census(rig);
    let mut still = 0;
    for _ in 0..60 {
        for _ in 0..5 {
            let (_, queued) = fire(rig);
            land_quietly(rig, &queued);
        }
        let now = census(rig);
        if now["balance"] == before["balance"] {
            still += 1;
            if still >= 3 {
                return now;
            }
        } else {
            still = 0;
        }
        before = now;
    }
    panic!("the matter census never held still: {before:?}");
}

/// Kilograms in one unit of `good`.
fn kg(rig: &Rig, good: &str) -> i128 {
    store(&rig.tw)
        .goods()
        .and_then(|goods| goods.good(&cid(good)))
        .and_then(tacsim::definitions::GoodDef::mass_kg_per_unit)
        .map(i128::from)
        .unwrap_or_else(|| panic!("{good} has a mass"))
}

/// An enrolled pilot and her chartered Dray, docked at Ceres, which has a workshop.
fn chartered(rig: &mut Rig, who: &str, seed: i64, name: &str) -> (ObjId, HullRow) {
    let pilot = pilot(rig, who, seed);
    let out = run(&mut rig.tw, pilot, &format!("+coop/enroll {name}"));
    assert!(out.contains(&format!("now the {name}")), "{out}");
    let charter = tacsim::port::live_charter(rig.tw.world(), pilot).expect("a charter");
    let row = port_map(&rig.tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|row| row.hull == charter.hull.0)
        .expect("the hull's row");
    mint(rig, pilot);
    (pilot, row)
}

/// The workshop's stores at Ceres: its operator's storefront.
fn stores(rig: &Rig) -> VaultId {
    let map = port_map(&rig.tw, "ceres_port");
    let operator = map
        .vendors
        .iter()
        .find(|(id, _)| id.as_str() == "occator")
        .map(|(_, row)| row.vault)
        .expect("Occator's storefront at Ceres");
    VaultId(operator)
}

/// Her magazines, by mount id.
fn held(rig: &Rig, hull: ObjId) -> BTreeMap<String, Magazine> {
    magazines(&unit(rig, hull))
        .into_iter()
        .map(|magazine| (magazine.mount.clone(), magazine))
        .collect()
}

/// Staff set what her magazines hold: `left` by mount id, the rest full.
fn spend(rig: &mut Rig, hull: ObjId, left: &[(&str, u32)]) {
    let rounds: BTreeMap<String, u32> = left
        .iter()
        .map(|(mount, left)| ((*mount).to_owned(), *left))
        .collect();
    let op = tacsim::space::custody::magazine_op(&unit(rig, hull), &rounds)
        .expect("the record")
        .expect("a change");
    damage(rig, hull, vec![op]);
}

/// The pilot's available funds, minor.
fn available(rig: &Rig, who: ObjId) -> i64 {
    tacsim::port::credit::own_account(rig.tw.world(), who)
        .and_then(|a| a.available())
        .map(|a| a.minor())
        .expect("the pilot's account")
}

/// The labour of `rounds` at the workshop where `hull` stands: the tariff's per-round cost at
/// the operator's multiplier, as the quote prices it.
fn labour(rig: &Rig, hull: ObjId, rounds: u64) -> u64 {
    let facility = tacsim::port::bay_work::facility_of(rig.tw.world(), hull).expect("a facility");
    let tariff = tacsim::definitions::workshop::workshop_tariff()
        .expect("the tariff")
        .with_multiplier_permille(facility.multiplier_permille)
        .expect("the multiplier");
    rounds * tariff.cost_per_ammo_round
}

/// The price of one unit of `good` from the stores, minor: its catalogue reference.
fn price(rig: &Rig, good: &str) -> u64 {
    store(&rig.tw)
        .goods()
        .and_then(|goods| goods.good(&cid(good)))
        .map(|def| def.reference_major * 100)
        .expect("a price")
}

/// **The Dray's point defence**: one rotary short three whole boxes, the other short less than
/// one. One box is in her hold, two are bought from the stores; the short rotary waits.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn cc7b_a_dray_rearms_her_point_defence_from_her_hold_and_the_stores() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    let full = held(&rig, hull);
    assert_eq!(
        full.keys().map(String::as_str).collect::<Vec<_>>(),
        ["pdc_dorsal", "pdc_ventral"],
        "the Dray's magazines are her two rotaries: she carries no tubes"
    );
    let stow = full["pdc_dorsal"].stow;
    assert!(
        stow >= 1_500 + 500,
        "a rotary stows more than 2,000: {stow}"
    );
    spend(
        &mut rig,
        hull,
        &[("pdc_dorsal", stow - 1_500), ("pdc_ventral", stow - 300)],
    );
    seed_hold(&mut rig, &row, "pd_box", 1, 9_101);
    let stores = stores(&rig);
    seed_pool(&mut rig, stores, "pd_box", 5, LotId(9_102));
    let hold = hold_vault_of(&rig.tw, &row);
    board(&mut rig, pilot);

    // The quote: one rearm, three boxes; the short rotary has none.
    let quote = run(&mut rig.tw, pilot, &format!("+bay #{}", hull.0));
    assert!(
        quote.contains("rearm pdc_dorsal (1500 rounds, 3 pd_box)"),
        "{quote}"
    );
    assert!(!quote.contains("rearm pdc_ventral"), "{quote}");
    assert!(
        quote.contains(
            "Rounds from her hold: 1 pd_box; bought from the workshop's stores: 2 pd_box"
        ),
        "{quote}"
    );

    let before = steady(&mut rig);
    let funds = available(&rig, pilot);
    let taken = run(&mut rig.tw, pilot, &format!("+bay/order #{}=rearm", hull.0));
    assert!(taken.contains("taken in the workshop"), "{taken}");
    assert!(
        taken.contains("Bought from the workshop's stores: 2 pd_box."),
        "{taken}"
    );
    let order = order_of(&rig, hull);
    let bought = 2 * price(&rig, "pd_box");
    assert_eq!(
        u64::try_from(order.fee_minor).expect("a fee"),
        labour(&rig, hull, 1_500) + bought,
        "the fee is the labor and the two boxes bought"
    );
    assert_eq!(
        available(&rig, pilot),
        funds - order.fee_minor,
        "held in escrow"
    );

    let _ = to_ic(&mut rig, order.due_ic + 60);
    let _ = to_ic(&mut rig, order.due_ic + 600);
    assert_eq!(order_of(&rig, hull).status, OrderStatus::Done);
    let after = held(&rig, hull);
    assert_eq!(after["pdc_dorsal"].left, stow, "rearmed full");
    assert_eq!(
        after["pdc_ventral"].left,
        stow - 300,
        "less than a box short: it waits"
    );
    assert_eq!(hold_pool(&mut rig.tw, hold, "pd_box"), 0, "her box drawn");
    assert_eq!(hold_pool(&mut rig.tw, stores, "pd_box"), 3, "two bought");
    assert_eq!(
        available(&rig, pilot),
        funds - order.fee_minor,
        "charged once"
    );

    // The census: the three boxes are `loaded`, and its balance holds.
    let census_after = steady(&mut rig);
    assert_eq!(
        census_after["loaded"] - before["loaded"],
        3 * kg(&rig, "pd_box"),
        "before {before:?}, after {census_after:?}"
    );
    assert_eq!(census_after["balance"], before["balance"]);
}

/// **A hull with tubes** (the corvette's fit on her unit): torpedoes from her hold and from the
/// stores, spinal rounds all bought, and every magazine full after.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn cc7b_a_corvette_rearms_her_tubes_and_spinal_gun_from_her_hold_and_the_stores() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 2, "Tin Kettle");
    let hull = ObjId(row.hull);
    let catalog = tacsim::builtin_definitions().expect("the catalog");
    let spec = catalog
        .bindings
        .get(&cid("tacsim.unit.corvette"))
        .expect("the corvette binds")
        .spec
        .clone();
    staff_hand(&mut rig, |ctx| {
        mush_tactics::module::refit_unit(ctx, hull, &spec).expect("she takes the corvette's unit");
    });
    let full = held(&rig, hull);
    let (tubes, spinal) = (full["torp_bow"].stow, full["rail_spinal"].stow);
    spend(
        &mut rig,
        hull,
        &[("torp_bow", tubes - 6), ("rail_spinal", spinal - 4)],
    );
    seed_hold(&mut rig, &row, "torpedo", 4, 9_201);
    let stores = stores(&rig);
    seed_pool(&mut rig, stores, "torpedo", 5, LotId(9_202));
    seed_pool(&mut rig, stores, "spinal_round", 10, LotId(9_203));
    let hold = hold_vault_of(&rig.tw, &row);
    board(&mut rig, pilot);

    let before = steady(&mut rig);
    let funds = available(&rig, pilot);
    let taken = run(&mut rig.tw, pilot, &format!("+bay/order #{}=rearm", hull.0));
    assert!(taken.contains("taken in the workshop"), "{taken}");
    assert!(
        taken.contains("Bought from the workshop's stores: 4 spinal_round, 2 torpedo."),
        "{taken}"
    );
    let order = order_of(&rig, hull);
    let bought = 2 * price(&rig, "torpedo") + 4 * price(&rig, "spinal_round");
    assert_eq!(
        u64::try_from(order.fee_minor).expect("a fee"),
        labour(&rig, hull, 6 + 4) + bought,
        "the fee is the labor and the rounds bought"
    );

    let _ = to_ic(&mut rig, order.due_ic + 60);
    let _ = to_ic(&mut rig, order.due_ic + 600);
    assert_eq!(order_of(&rig, hull).status, OrderStatus::Done);
    let after = held(&rig, hull);
    assert_eq!(after["torp_bow"].left, tubes);
    assert_eq!(after["rail_spinal"].left, spinal);
    assert!(
        tacsim::space::custody::kept_magazines(&unit(&rig, hull)).is_empty(),
        "every magazine full: her unit keeps no record"
    );
    assert_eq!(hold_pool(&mut rig.tw, hold, "torpedo"), 0);
    assert_eq!(hold_pool(&mut rig.tw, stores, "torpedo"), 3);
    assert_eq!(hold_pool(&mut rig.tw, stores, "spinal_round"), 6);
    assert_eq!(available(&rig, pilot), funds - order.fee_minor);

    let census_after = steady(&mut rig);
    assert_eq!(
        census_after["loaded"] - before["loaded"],
        6 * kg(&rig, "torpedo") + 4 * kg(&rig, "spinal_round"),
        "before {before:?}, after {census_after:?}"
    );
    assert_eq!(census_after["balance"], before["balance"]);
}

/// **Too few rounds anywhere**: the order is refused before any escrow, naming what the hold
/// and the stores carry.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn cc7b_a_rearm_her_hold_and_the_stores_cannot_cover_is_refused() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 3, "Salt Moth");
    let hull = ObjId(row.hull);
    let stow = held(&rig, hull)["pdc_dorsal"].stow;
    spend(&mut rig, hull, &[("pdc_dorsal", stow - 1_500)]);
    seed_hold(&mut rig, &row, "pd_box", 1, 9_301);
    board(&mut rig, pilot);
    let funds = available(&rig, pilot);
    let refused = run(&mut rig.tw, pilot, &format!("+bay/order #{}=rearm", hull.0));
    let at = stores(&rig);
    let in_store = hold_pool(&mut rig.tw, at, "pd_box");
    assert!(in_store < 2, "the rig's stores hold fewer than two boxes");
    let want = SHORT_OF_ROUNDS
        .replace("{need}", "3")
        .replace("{good}", "pd_box")
        .replace("{hold}", "1")
        .replace("{store}", &in_store.to_string());
    assert!(refused.contains(&want), "{refused}");
    assert_eq!(available(&rig, pilot), funds, "nothing held");
    assert_eq!(held(&rig, hull)["pdc_dorsal"].left, stow - 1_500);
}

/// A crit on record (`tacsim.space.subsystems`): her dorsal rotary stripped, as a check-in from
/// a bubble writes it.
fn crit_record(u: &mush_tactics::TacticalUnit) -> mush_tactics::UnitMutationOp {
    let schema = u
        .capabilities()
        .next()
        .and_then(|k| u.capability(k))
        .map_or_else(|| u.schema().clone(), |rv| rv.schema().clone());
    mush_tactics::UnitMutationOp::GrantCapability {
        key: crits_key(),
        rules: mush_core::RulesValue::new(
            schema,
            serde_json::json!({ "crits": [], "stripped_pdcs": ["pdc_dorsal"], "slow_pct": 100 }),
        )
        .expect("the record"),
    }
}

fn crits_key() -> mush_tactics::CapabilityKey {
    mush_tactics::CapabilityKey {
        id: mush_tactics::CapabilityId::new(tacsim::space::custody::SPACE_SUBSYSTEMS_CAPABILITY)
            .expect("an id"),
        source: None,
    }
}

/// Her hull's heat sink, `(current, max)`: the radiator a space hull's check-in writes down.
fn sink(rig: &Rig, hull: ObjId) -> Option<(i64, i64)> {
    let u = unit(rig, hull);
    let addr = mush_tactics::CellAddr::Section {
        section: section_id(&u, "hull"),
        cell: mush_tactics::CellId::new(tacsim::definitions::unit::HEAT_SINK_CELL).expect("a cell"),
    };
    u.cell(&addr).map(|cell| (cell.current, cell.max))
}

/// **48e review rounds:G4 and G5: a repair clears her crits and her radiator; a rearm does not.**
/// A rearm-only order loads rounds and leaves the crit record and the heat sink as they were,
/// where it once healed the radiator for the labor of a few rounds (G5). An armour repair clears
/// the crits, where the work once revoked them on its own copy of the hull and never wrote it
/// (G4).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn rounds_g4_g5_a_repair_clears_her_crits_and_a_rearm_does_not() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    let stow = held(&rig, hull)["pdc_dorsal"].stow;
    spend(&mut rig, hull, &[("pdc_dorsal", stow - 500)]);
    let crit = crit_record(&unit(&rig, hull));
    damage(&mut rig, hull, vec![crit]);
    let hot = sink(&rig, hull).map(|(_, max)| {
        let u = unit(&rig, hull);
        let addr = mush_tactics::CellAddr::Section {
            section: section_id(&u, "hull"),
            cell: mush_tactics::CellId::new(tacsim::definitions::unit::HEAT_SINK_CELL)
                .expect("a cell"),
        };
        damage(
            &mut rig,
            hull,
            vec![mush_tactics::UnitMutationOp::SetCell {
                addr,
                value: max / 2,
            }],
        );
        max / 2
    });
    seed_hold(&mut rig, &row, "pd_box", 1, 9_101);
    board(&mut rig, pilot);

    let taken = run(&mut rig.tw, pilot, &format!("+bay/order #{}=rearm", hull.0));
    assert!(taken.contains("taken in the workshop"), "{taken}");
    let order = order_of(&rig, hull);
    let _ = to_ic(&mut rig, order.due_ic + 60);
    let _ = to_ic(&mut rig, order.due_ic + 600);
    assert_eq!(order_of(&rig, hull).status, OrderStatus::Done);
    assert_eq!(held(&rig, hull)["pdc_dorsal"].left, stow, "rearmed");
    assert!(
        unit(&rig, hull).capability(&crits_key()).is_some(),
        "a rearm repairs no crit"
    );
    if let Some(hot) = hot {
        assert_eq!(
            sink(&rig, hull).map(|(current, _)| current),
            Some(hot),
            "a rearm heals no radiator"
        );
    }

    let u = unit(&rig, hull);
    damage(&mut rig, hull, vec![dent(&u, &section_id(&u, "hull"), 10)]);
    seed_hold(&mut rig, &row, "structural_alloys", 2, 9_102);
    seed_hold(&mut rig, &row, "industrial_parts", 40, 9_103);
    let taken = run(&mut rig.tw, pilot, &format!("+bay/order #{}=armor", hull.0));
    assert!(taken.contains("taken in the workshop"), "{taken}");
    let order = order_of(&rig, hull);
    let _ = to_ic(&mut rig, order.due_ic + 60);
    let _ = to_ic(&mut rig, order.due_ic + 600);
    assert_eq!(order_of(&rig, hull).status, OrderStatus::Done);
    assert!(
        unit(&rig, hull).capability(&crits_key()).is_none(),
        "the repair cleared her crits"
    );
    if hot.is_some() {
        assert_eq!(
            sink(&rig, hull).map(|(current, max)| current == max),
            Some(true),
            "and her radiator"
        );
    }
}

/// **48e review rounds:G1 and G13: an order canceled after its draw landed gives the rounds back;
/// the quote's "all" carries the rounds bought.** The draw lands, then staff touch her hull before
/// the work is written, so the order is canceled at its completion. Once that cancel took the
/// fee's refund and kept the goods; now her box goes back to her hold and the two bought go back
/// to the stores, nothing is loaded, and the census counts nothing `loaded`.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn rounds_g1_g13_a_canceled_order_gives_its_rounds_back() {
    let mut rig = stocked();
    let (pilot, row) = chartered(&mut rig, "Pilot", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    let stow = held(&rig, hull)["pdc_dorsal"].stow;
    spend(&mut rig, hull, &[("pdc_dorsal", stow - 1_500)]);
    seed_hold(&mut rig, &row, "pd_box", 1, 9_101);
    let stores = stores(&rig);
    seed_pool(&mut rig, stores, "pd_box", 5, LotId(9_102));
    let hold = hold_vault_of(&rig.tw, &row);
    board(&mut rig, pilot);

    // G13: "all" is the labor and the two boxes bought.
    let quote = run(&mut rig.tw, pilot, &format!("+bay #{}", hull.0));
    let all = labour(&rig, hull, 1_500) + 2 * price(&rig, "pd_box");
    let line = quote
        .lines()
        .find(|line| line.trim_start().starts_with("all "))
        .unwrap_or_else(|| panic!("an all line: {quote}"));
    assert!(
        line.contains(&format!("{}.{:02}cr", all / 100, all % 100)),
        "all is {all} minor: {line}"
    );

    let taken = run(&mut rig.tw, pilot, &format!("+bay/order #{}=rearm", hull.0));
    assert!(taken.contains("taken in the workshop"), "{taken}");
    let order = order_of(&rig, hull);
    let before = steady(&mut rig);
    // The order comes due and its draw lands.
    let (_, queued) = advance_to(&mut rig, order.due_ic);
    assert!(
        !queued.is_empty(),
        "the draw is submitted at the order's due time"
    );
    land_quietly(&mut rig, &queued);
    assert_eq!(hold_pool(&mut rig.tw, hold, "pd_box"), 0, "her box drawn");
    assert_eq!(hold_pool(&mut rig.tw, stores, "pd_box"), 3, "two bought");
    // Staff touch her hull before the work is written: its revision moves.
    spend(
        &mut rig,
        hull,
        &[("pdc_dorsal", stow - 1_500), ("pdc_ventral", stow - 1)],
    );
    let _ = to_ic(&mut rig, order.due_ic + 60);
    let _ = to_ic(&mut rig, order.due_ic + 600);
    assert_eq!(order_of(&rig, hull).status, OrderStatus::Cancelled);
    assert_eq!(hold_pool(&mut rig.tw, hold, "pd_box"), 1, "her box is back");
    assert_eq!(
        hold_pool(&mut rig.tw, stores, "pd_box"),
        5,
        "the two bought are back"
    );
    assert_eq!(
        held(&rig, hull)["pdc_dorsal"].left,
        stow - 1_500,
        "nothing loaded"
    );
    let after = steady(&mut rig);
    assert_eq!(
        after["loaded"], before["loaded"],
        "nothing loaded: {after:?}"
    );
    assert_eq!(after["balance"], before["balance"]);
}

//! **Doc 48 cut EX.0 — the matter census's sinks** (Q-48-31), over the composed world
//! (`driving`'s keyed rig), through the production command paths.
//!
//! GC.8b's refit run, read by the census. The pilot leases Occator's escort into their Dray-P at
//! Ceres, buys the escort's munitions at the Co-op chandlery and sails to Stickney Yards. The
//! departure burn spends propellant and provisions: `burned` moves by the ticket's tonnes and
//! crew-days. The escort comes in with both magazines spent and is rearmed in the carrier's bay
//! from the hold: `loaded` moves by the munitions' kilograms. The census counts the propellant,
//! the provisions and the munitions now (they were outside it before EX.0), and its balance holds
//! across the whole window.

use std::collections::BTreeMap;

use mush_core::{GOD, ObjId};
use tacsim::port::bay_work::OrderStatus;

use crate::bay_gc8::{
    STICKNEY, bay_room, buy, component_id, damage, empty_magazine, leased, magazine, order_of, put,
    unit,
};
use crate::driving_rig::{
    Rig, board, cid, file_and_launch, fire, hold_pool, hold_vault_of, land_quietly, load, run,
    stocked, store, to_ic,
};

/// The census line's terms, by name.
fn census(rig: &mut Rig) -> BTreeMap<String, i128> {
    let out = run(&mut rig.tw, GOD, "@industry/census");
    let line = out
        .lines()
        .find(|l| l.contains(tacsim::port::census::CENSUS_LINE))
        .unwrap_or_else(|| panic!("a census line: {out}"));
    line.split_whitespace()
        .filter_map(|word| word.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.parse().expect("a number")))
        .collect()
}

/// The census once its balance holds still for three reads a few fires apart (IN.11's window).
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

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn ex0_the_census_holds_across_a_burn_and_a_rearm() {
    let mut rig = stocked();
    let (pilot, row, escort, _rig) = leased(&mut rig);
    let hull = ObjId(row.hull);
    let vault = hold_vault_of(&rig.tw, &row);

    // ---- Before the window: the escort's magazines spent (rounds are not goods), her munitions
    // bought (a transfer the census holds), the tank and the locker seeded by staff (a mint the
    // census cannot see, so it lands before the first read) ----
    let u = unit(&rig, escort);
    let (cannon, gun) = (component_id(&u, "cannon"), component_id(&u, "machine_gun"));
    let he_rounds = u64::try_from(magazine(&u, &cannon).1).expect("a magazine");
    let ball_boxes = u64::try_from(magazine(&u, &gun).1)
        .expect("a magazine")
        .div_ceil(100);
    damage(
        &mut rig,
        escort,
        vec![empty_magazine(&cannon), empty_magazine(&gun)],
    );
    buy(&mut rig, pilot, "coop", "ammo_he_105", he_rounds);
    buy(&mut rig, pilot, "coop", "ammo_ball_762", ball_boxes);
    load(&mut rig, &row, 60, 40, 9_500);
    board(&mut rig, pilot);
    let before = steady(&mut rig);
    for term in ["burned", "loaded", "seized", "consumed"] {
        assert!(
            before.contains_key(term),
            "the line names {term}: {before:?}"
        );
    }
    // The money census (Q-48-17) files the ports' institutions as lane holdings, by the
    // categories tacsim's registration installs.
    let money = run(&mut rig.tw, GOD, "@econ/census 1");
    let lane: i128 = money
        .lines()
        .find(|l| l.starts_with("  held "))
        .and_then(|l| l.split_whitespace().find_map(|w| w.strip_prefix("lane=")))
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("a holdings line: {money}"));
    assert!(lane > 0, "the lane's capital is lane holdings: {money}");

    // ---- The burn: Ceres to Stickney Yards ----
    let ticket = file_and_launch(&mut rig, pilot, hull, "+ship/undock stickney");
    let arrive = ticket.plan.depart_ic + ticket.plan.total_s;
    let _ = to_ic(&mut rig, arrive);
    let _ = to_ic(&mut rig, arrive + 3_600);
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == STICKNEY
        ),
        "docked at Stickney Yards"
    );
    let burned = i128::from(ticket.plan.burn_t) * kg(&rig, "propellant")
        + i128::from(ticket.plan.crew_days) * kg(&rig, "provisions");
    assert!(burned > 0, "the passage burned something: {ticket:?}");

    // ---- The rearm, in the carrier's bay, from the hold ----
    put(&mut rig, pilot, bay_room(&row), true);
    let taken = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/order #{}=rearm", escort.0),
    );
    assert!(taken.contains("taken in the carrier bay"), "{taken}");
    let order = order_of(&rig, escort);
    let _ = to_ic(&mut rig, order.due_ic + 60);
    let _ = to_ic(&mut rig, order.due_ic + 600);
    assert_eq!(order_of(&rig, escort).status, OrderStatus::Done);
    let after_unit = unit(&rig, escort);
    for part in [&cannon, &gun] {
        let (current, max) = magazine(&after_unit, part);
        assert_eq!(current, max, "{part} rearmed");
    }
    assert_eq!(hold_pool(&mut rig.tw, vault, "ammo_he_105"), 0);
    assert_eq!(hold_pool(&mut rig.tw, vault, "ammo_ball_762"), 0);
    let loaded = i128::from(he_rounds) * kg(&rig, "ammo_he_105")
        + i128::from(ball_boxes) * kg(&rig, "ammo_ball_762");

    // ---- The census: each sink moved by what it took, and the balance held ----
    let after = steady(&mut rig);
    assert_eq!(
        after["burned"] - before["burned"],
        burned,
        "the burn's propellant and provisions: before {before:?}, after {after:?}"
    );
    assert_eq!(
        after["loaded"] - before["loaded"],
        loaded,
        "the rearm's munitions: before {before:?}, after {after:?}"
    );
    // Doc 48 unit EX.9: the consumers in this run are the Lunar Habitat Board, which uses up
    // 16 crew-days of its provisions an IC day, and (doc 50 cut OP.4) the Isidis tank farm, 2 t
    // of propellant an IC day (lane v54; 1 t at v53, 20 t before doc 50 cut OP.8's retune).
    let provisions_kg = store(&rig.tw)
        .goods()
        .and_then(|goods| {
            goods
                .good(&mush_core::CanonicalId::new("provisions").expect("an id"))
                .and_then(|g| g.mass_kg_per_unit())
        })
        .expect("provisions weigh");
    let propellant_kg = store(&rig.tw)
        .goods()
        .and_then(|goods| {
            goods
                .good(&mush_core::CanonicalId::new("propellant").expect("an id"))
                .and_then(|g| g.mass_kg_per_unit())
        })
        .expect("propellant weighs");
    let used = after["consumed"] - before["consumed"];
    let farm_day = 2 * i128::from(propellant_kg);
    let board_day = 16 * i128::from(provisions_kg);
    assert!(
        used >= 0 && (0..=used / board_day).any(|days| (used - days * board_day) % farm_day == 0),
        "only the farm's whole days of propellant and the Board's whole days of provisions are used up: {used}"
    );
    assert_eq!(
        after["balance"], before["balance"],
        "the census holds across the burn and the rearm: before {before:?}, after {after:?}"
    );
}

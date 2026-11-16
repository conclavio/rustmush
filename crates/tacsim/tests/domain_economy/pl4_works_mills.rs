//! **Doc 48 unit PL.4 — the mills and the refine as works recipes** (Q-48-24), over the composed
//! world (`driving`'s keyed rig), through the production command paths.
//!
//! The houses' three conversions — the Freeport Exchange's metal mill (5 : 2), Occator's ice mill
//! (5 : 2) and Kasei's refine (5 : 4) — are public slots at the works too (the recipe pack's v5
//! rows `metals`, `volatiles` and `structural_alloys`), at the houses' ratios and the cost-model
//! fee. The houses keep their own mill rows.
//!
//! **Seeds** (doc 43 rule 2.3 #4): the captain's money (`@econ/mint`) and the metal ore in the
//! hold (a miner's goods; staff's `seed_hold`). Every claim — the quotes, the orders, the
//! deliveries, the works' runs and the collections — is reached by the player's commands and the
//! works' production.

use mush_core::ObjId;
use tacsim::definitions::content::LANE_JSON;
use tacsim::pack::{PackEntry, load_pack};

use crate::driving_rig::{
    Rig, cid, hold_pool, hold_vault_of, ic_now, prospector, run, seed_hold, stocked, store,
};
use crate::pl2_producers::{contract, ic_to, mint, order_of, works, works_fire};

const METAL_ORE: &str = "metal_ore";
const METALS: &str = "metals";
const ALLOYS: &str = "structural_alloys";

/// **One order at the Ceres works, run to the hold**: `+works/order {recipe} {qty}`, deliver
/// `input_t` of `input` from the docked hold, let the works run it and deliver, collect the
/// output into the hold. The order's id.
fn order_through(
    rig: &mut Rig,
    captain: ObjId,
    recipe: &str,
    qty: u32,
    (input, input_t): (&str, u64),
) -> u64 {
    let before: Vec<u64> = store(&rig.tw)
        .works_contracts()
        .into_iter()
        .map(|c| c.id)
        .collect();
    let booked = works(rig, captain, &format!("+works/order {recipe} {qty}"), 1);
    assert!(booked.contains("booked at the works"), "{booked}");
    let id = store(&rig.tw)
        .works_contracts()
        .into_iter()
        .map(|c| c.id)
        .find(|id| !before.contains(id))
        .expect("the order");
    works(
        rig,
        captain,
        &format!("+works/deliver {id}={input} {input_t}"),
        1,
    );
    // The works runs the order's work units an IC hour each; jump an hour at a time to its end.
    let start = ic_now(&rig.tw);
    for hour in 1..=48 {
        let done = order_of(rig, id).is_some_and(|o| {
            matches!(
                o.state(),
                mush_industry::WorkOrderState::AwaitingDelivery
                    | mush_industry::WorkOrderState::Completed { .. }
            )
        });
        if done {
            break;
        }
        ic_to(rig, start + hour * 3_600);
    }
    for _ in 0..6 {
        if contract(rig, id).delivered_ic.is_some() {
            break;
        }
        let _ = works_fire(rig);
    }
    assert!(
        contract(rig, id).delivered_ic.is_some(),
        "{recipe} x{qty} delivered"
    );
    // A pooled output is dissolved in the escrow before it can be collected (IN.5's rule).
    let mut collected = String::new();
    for _ in 0..12 {
        collected = works(rig, captain, &format!("+works/collect {id}"), 2);
        if collected.contains(tacsim::port::works::INTO_HOLD) {
            break;
        }
    }
    assert!(
        collected.contains(tacsim::port::works::INTO_HOLD),
        "{collected}"
    );
    id
}

/// **Doc 48 unit PL.4: a captain mills their own metal ore at the works and refines the metals**
/// (Q-48-24). At Ceres, with 20 t of metal ore in the docked hold:
///
/// 1. `+works/quote metals 2` bills the 20 t at the Exchange's ratio (5 : 2) for four work units.
/// 2. The order booked, the ore delivered and the works run, the captain collects 8 t of metals
///    into the hold.
/// 3. `+works/quote structural_alloys` bills 5 t of metals at Kasei's ratio (5 : 4) for nine work
///    units; booked, delivered, run and collected, the hold holds 4 t of alloys and the 3 t of
///    metals left over.
/// 4. The houses keep their own rows: the Exchange's metal mill, Occator's ice mill and Kasei's
///    refine are the lane's as before.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pl4_a_captain_mills_metal_ore_at_the_works_then_refines_the_metals() {
    let mut rig = stocked();
    let (captain, row) = prospector(&mut rig, "Okafor", 1, "Slag Wren");
    mint(&mut rig, captain);
    let hold = hold_vault_of(&rig.tw, &row);
    seed_hold(&mut rig, &row, METAL_ORE, 20, 7_901);

    // 1-2. The ore milled at the works' public slot.
    let quote = run(&mut rig.tw, captain, "+works/quote metals 2");
    assert!(
        quote.contains("bill 20 metal_ore") && quote.contains("(4 work units)"),
        "{quote}"
    );
    order_through(&mut rig, captain, METALS, 2, (METAL_ORE, 20));
    assert_eq!(hold_pool(&mut rig.tw, hold, METAL_ORE), 0);
    assert_eq!(
        hold_pool(&mut rig.tw, hold, METALS),
        8,
        "milled five to two"
    );

    // 3. The metals refined.
    let quote = run(&mut rig.tw, captain, "+works/quote structural_alloys");
    assert!(
        quote.contains("bill 5 metals") && quote.contains("(9 work units)"),
        "{quote}"
    );
    order_through(&mut rig, captain, ALLOYS, 1, (METALS, 5));
    assert_eq!(hold_pool(&mut rig.tw, hold, METALS), 3);
    assert_eq!(
        hold_pool(&mut rig.tw, hold, ALLOYS),
        4,
        "refined five to four"
    );

    // 4. The houses' own rows unchanged.
    let lane = load_pack(LANE_JSON).expect("the lane verifies");
    let houses: Vec<(String, String, String)> = ["ceres_port", "stickney_port"]
        .iter()
        .flat_map(|entry| {
            let Some(PackEntry::Port(def)) = lane.entry(&cid(entry)) else {
                panic!("{entry} is a port entry");
            };
            def.clone()
                .vendors
                .into_iter()
                .flat_map(|vendor| {
                    vendor
                        .mill
                        .into_iter()
                        .filter(|mill| !mill.kind.is_process())
                        .map(move |mill| {
                            (
                                vendor.org.to_string(),
                                mill.ore.to_string(),
                                mill.into.to_string(),
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect();
    for (org, ore, into) in [
        ("freeport_exchange", METAL_ORE, METALS),
        ("occator", "ice_ore", "volatiles"),
        ("occator", "volatiles", "propellant"),
        ("kasei", METALS, ALLOYS),
    ] {
        assert!(
            houses
                .iter()
                .any(|(o, i, p)| o == org && i == ore && p == into),
            "{org}'s {ore} row: {houses:?}"
        );
    }
}

/// The quote's fee, credits, from `+works/quote`'s line (`…; fee 89.00cr; booking fee …`).
fn quoted_fee(quote: &str) -> f64 {
    quote
        .split(tacsim::port::works::FEE_CLAUSE)
        .nth(1)
        .and_then(|rest| rest.split("cr").next())
        .and_then(|fee| fee.trim().parse().ok())
        .unwrap_or_else(|| panic!("a fee in {quote}"))
}

/// **Doc 48 unit PL.4: each conversion's fee is about its house's own spread** (Q-48-24, "at the
/// cost-model fee"): at the Ceres works, each recipe quotes its bill at the house's ratio, and
/// its fee lands within 15% under what the house makes on the same batch at its own port's
/// prices — the Exchange's metal ore bid (70) and metals ask (200), Occator's ice ore bid (35)
/// and water ask (100), Kasei's metals bid (280) and alloys ask (450). Milling one's own ore at
/// the works is then about even with the house's chain at its port, and pays where the product
/// is dearer or the house's bid is full.
#[test]
fn pl4_each_conversion_costs_about_its_houses_spread() {
    let mut rig = stocked();
    let (captain, _) = prospector(&mut rig, "Okafor", 1, "Slag Wren");
    for (recipe, bill, units, spread) in [
        ("metals", "bill 10 metal_ore", 2, 4.0 * 200.0 - 10.0 * 70.0),
        ("volatiles", "bill 20 ice_ore", 2, 8.0 * 100.0 - 20.0 * 35.0),
        (
            "structural_alloys",
            "bill 5 metals",
            9,
            4.0 * 450.0 - 5.0 * 280.0,
        ),
    ] {
        let quote = run(&mut rig.tw, captain, &format!("+works/quote {recipe}"));
        assert!(
            quote.contains(bill) && quote.contains(&format!("({units} work units)")),
            "{recipe}: {quote}"
        );
        let fee = quoted_fee(&quote);
        println!("PL.4: {recipe} costs {fee:.2} cr a batch against the house's {spread:.0} cr");
        assert!(
            fee <= spread && fee >= 0.85 * spread,
            "{recipe}: {fee:.2} cr against {spread:.0}"
        );
    }
}

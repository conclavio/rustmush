//! **Doc 48 unit PL.7: a governed core from the chain** (Q-48-27, with doc 48b's correction, and
//! Q-48-48), on the combined M8 world (the production `LaunchComposition`, CO.6's world).
//!
//! A certified die takes its chain goods from the importer's depot when the shop's stock agent
//! issues it: two instrument boards and a tonne of strategic ore for Class II, by the hardware
//! catalogue's `die_chain`. So `+core/buy` refuses an order the depot cannot cut a die for, and
//! names the bill. With the goods back, the order goes in: the counter's die takes them in its
//! own bundle, and a restart reads the depot as it was left.
//!
//! **What staff do**: top the captain's wallet up (the M8 world's standing practice), and take
//! strategic ore out of the depot and put a tonne back, by the inventory's own debit and credit,
//! as an operator stocks it (the rig stocks it at the start, `m8_world::DEPOT_STOCK`). **What the
//! story does by hand**: it moves the clock to run the counter's tick.

use mush_core::ObjId;
use mush_industry::core_production::DieClass;
use serde_json::json;
use tacsim::hardware::counter::{CORE_ACCEPTED, CORE_NO_STOCK, working};

use crate::co6_cored_ship::{pump, world};
use crate::excursions::run;
use crate::m8_end_user::{credit, fund};
use crate::m8_world::{DEPOT_STOCK, M8World};

const BOARDS: &str = "instrument_boards";
const ORE: &str = "strategic_ore";

/// The head of a template, before its first `{`.
fn head(template: &str) -> &str {
    template.split('{').next().expect("a head")
}

fn families(world: &M8World) -> mush_inventory::InventoryFamilies {
    world
        .rig
        .tw
        .world()
        .module_state::<mush_inventory::InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone()
}

/// The importer's depot's pool of `good`.
fn depot(world: &mut M8World, good: &str) -> u64 {
    let families = families(world);
    let endpoint = mush_inventory::EndpointRef(world.deployment.hardware.depot);
    let mut txn = world
        .rig
        .tw
        .engine_mut()
        .action_transaction(&families.pools)
        .expect("an inventory transaction");
    families
        .pool(
            &mut txn,
            endpoint,
            &mush_core::CanonicalId::new(good).expect("an id"),
        )
        .expect("the pool reads")
        .map_or(0, |pool| pool.quantity)
}

/// Staff take `units` of `good` out of the depot (negative) or put them in, by the inventory's
/// own debit and credit, kept for a restart.
fn stock(world: &mut M8World, good: &str, units: i64) {
    let families = families(world);
    let endpoint = mush_inventory::EndpointRef(world.deployment.hardware.depot);
    let good = mush_core::CanonicalId::new(good).expect("an id");
    let engine = world.rig.tw.engine_mut();
    let prepared = {
        let mut txn = engine
            .action_transaction(&families.pools)
            .expect("an inventory transaction");
        let amount = units.unsigned_abs();
        if units < 0 {
            families.debit(&mut txn, endpoint, &good, amount)
        } else {
            families.credit(&mut txn, endpoint, &good, amount)
        }
        .expect("staff's stock");
        txn.prepare().expect("prepares")
    };
    let (ops, install) = prepared.into_parts();
    engine.install_prepared_action(install).expect("installs");
    crate::excursions::keep(&mut world.rig, ops);
}

/// Staff hold the die lot `lot` where it lies (`hold`), or release that hold, by the inventory's
/// own lot hold, kept for a restart.
fn hold_die(world: &mut M8World, lot: mush_inventory::LotId, hold: bool) {
    const HOLDER: &str = "staff.e48";
    let families = families(world);
    let engine = world.rig.tw.engine_mut();
    let prepared = {
        let mut txn = engine
            .action_transaction(&families.lot_holds)
            .expect("an inventory transaction");
        if hold {
            let at = txn
                .get(&families.lots, &lot)
                .expect("the lot reads")
                .map(mush_inventory::Lot::endpoint)
                .expect("the die lot");
            families
                .hold_lot(&mut txn, at, lot, HOLDER)
                .expect("staff's hold");
        } else {
            families
                .release_lot(&mut txn, lot, HOLDER)
                .expect("staff's release");
        }
        txn.prepare().expect("prepares")
    };
    let (ops, install) = prepared.into_parts();
    engine.install_prepared_action(install).expect("installs");
    crate::excursions::keep(&mut world.rig, ops);
}

/// The matter census's terms, by name (doc 48e: `opening`, `cored`, `partial` among them).
fn census(world: &mut M8World) -> std::collections::BTreeMap<String, i128> {
    let out = run(&mut world.rig.tw, mush_core::GOD, "@industry/census");
    let line = out
        .lines()
        .find(|l| l.contains(tacsim::port::census::CENSUS_LINE))
        .unwrap_or_else(|| panic!("a census line: {out}"));
    line.split_whitespace()
        .filter_map(|word| word.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.parse().expect("a number")))
        .collect()
}

fn at_work(world: &M8World, who: ObjId) -> bool {
    working(world.rig.tw.world(), who).is_some_and(|stage| stage.contains("at work"))
}

/// **A Class II core takes its die's chain goods from the depot**, and the counter refuses an
/// order the depot cannot cut a die for.
#[test]
fn pl7_a_core_takes_its_die_s_chain_goods_from_the_depot() {
    let mut world = world();
    let captain = world.pilot;
    fund(&mut world, captain, 900_000);
    assert_eq!(
        (depot(&mut world, BOARDS), depot(&mut world, ORE)),
        (DEPOT_STOCK, DEPOT_STOCK),
        "the rig's stock"
    );
    // The depot's ore taken out: the shop cannot cut a Class II die, and says what it needs.
    stock(
        &mut world,
        ORE,
        -i64::try_from(DEPOT_STOCK).expect("a count"),
    );
    let before = credit(&world, captain).1;
    for line in ["+core/buy II", "+core/buy II go"] {
        let refused = run(&mut world.rig.tw, captain, line);
        assert!(
            refused.contains(head(CORE_NO_STOCK))
                && refused.contains("2 instrument_boards and 1 strategic_ore"),
            "{line}: {refused}"
        );
    }
    assert!(
        working(world.rig.tw.world(), captain).is_none(),
        "nothing queued"
    );
    assert_eq!(credit(&world, captain).1, before, "nothing paid");
    // Nor can the stock agent cut one by hand: the issue is refused and writes nothing.
    let agent = world.deployment.hardware.agents[1].actor;
    assert!(
        world
            .try_perform("industry", "issue_core_die", agent, json!({"class": "ii"}))
            .is_err(),
        "no die without its chain goods"
    );
    assert_eq!(depot(&mut world, BOARDS), DEPOT_STOCK, "the boards stay");
    // A tonne back: the order goes in, and the counter's die takes its bill from the depot.
    stock(&mut world, ORE, 1);
    let ordered = run(&mut world.rig.tw, captain, "+core/buy II go");
    assert!(
        ordered.contains("The counter takes your order"),
        "{ordered}"
    );
    let heard = pump(&mut world, captain, 60, |w| at_work(w, w.pilot));
    assert!(heard.contains(head(CORE_ACCEPTED)), "{heard}");
    assert_eq!(
        (depot(&mut world, BOARDS), depot(&mut world, ORE)),
        (DEPOT_STOCK - 2, 0),
        "two boards and the tonne of ore went into the die"
    );
    // A restart reads the depot as it was left. The counter's request was RAM and is gone; the
    // depot holds no ore for another die, but the order under way needs none, so asking again
    // picks it up and no second die is cut.
    let mut world = world.reboot();
    assert_eq!(
        (depot(&mut world, BOARDS), depot(&mut world, ORE)),
        (DEPOT_STOCK - 2, 0)
    );
    let again = run(&mut world.rig.tw, captain, "+core/buy II go");
    assert!(again.contains("The counter takes your order"), "{again}");
    let _ = pump(&mut world, captain, 60, |w| at_work(w, w.pilot));
    assert_eq!(
        depot(&mut world, BOARDS),
        DEPOT_STOCK - 2,
        "the order under way cut no second die"
    );
}

/// **Doc 48e, production G14: a die the shop issued and never sold is offered again.** The stock
/// agent cuts a Class II die and offers it to the captain, who lets the offer lapse; the world
/// restarts, forgetting the counter's record of the dies it issued; and staff take the depot's
/// ore out, so no new die can be cut. `+core/buy II go` still goes in, and the counter offers the
/// die already in the agent's stock: the captain's core is at work and the depot gives up
/// nothing. Before, the lapsed offer and the restart each stranded the die, and the counter cut
/// another from the finite depot (or, the depot short, gave the order up).
///
/// A lapsed offer is no order under way, so with its die not to hand and the depot short the
/// counter refuses at once. And a buyer whose money is held is refused at the counter, before any
/// die is drawn: the
/// counter reads what the account has available, not its posted balance. The matter census
/// holds its balance across the die's issue (services G28): the depot's opening stock is its
/// source and the die's chain goods its sink.
#[test]
fn e48_a_lapsed_or_forgotten_die_is_offered_again() {
    let mut world = world();
    let captain = world.pilot;
    fund(&mut world, captain, 900_000);
    let agent = world.deployment.hardware.agents[1].actor;
    // Services G28 (production G16): the census counts the depot's opening stock as a source
    // and the die's chain goods as its sink, so a die issue leaves the balance where it was.
    let before = census(&mut world);
    let (_, die) = world.perform(
        "industry",
        "issue_core_die",
        agent,
        json!({"class": DieClass::II}),
    );
    let lot: mush_inventory::LotId =
        serde_json::from_value(die["lot"].clone()).expect("the die's lot");
    let after = census(&mut world);
    assert!(
        before["opening"] > 0,
        "the depot's opening stock: {before:?}"
    );
    assert!(
        after["cored"] > before["cored"],
        "the die's chain goods: {after:?}"
    );
    assert_eq!(
        after["balance"], before["balance"],
        "{before:?} -> {after:?}"
    );
    assert_eq!(after["partial"], 0, "every scan read whole: {after:?}");
    let _ = world.perform(
        "industry",
        "offer_core_integration",
        agent,
        json!({"requester": captain.0, "class": DieClass::II, "supplier_lot": lot.to_string()}),
    );
    let mut world = world.reboot();
    // The offer lapses unaccepted.
    let expires = mush_industry::core_production::counter::view(world.rig.tw.world(), captain)
        .and_then(|view| view.integrations.first().map(|line| line.expires_utc))
        .expect("the offer");
    let lapsed = world.rig.tw.world().now().max(expires) + 1;
    world.rig.tw.set_now(lapsed);
    let boards = depot(&mut world, BOARDS);
    let ore = depot(&mut world, ORE);
    stock(&mut world, ORE, -i64::try_from(ore).expect("a count"));

    // A lapsed offer is no order under way: with its die not to hand (held), and the depot
    // short, the counter refuses at once rather than take an order it cannot cut a die for.
    hold_die(&mut world, lot, true);
    let refused = run(&mut world.rig.tw, captain, "+core/buy II");
    assert!(refused.contains(head(CORE_NO_STOCK)), "{refused}");
    hold_die(&mut world, lot, false);

    // Held money is not money to pay with.
    let (account, posted) = credit(&world, captain);
    let hold = run(
        &mut world.rig.tw,
        captain,
        &format!("@hold/place {}={posted} the deposit", account.0),
    );
    assert!(!hold.contains("Usage"), "{hold}");
    let refused = run(&mut world.rig.tw, captain, "+core/buy II");
    assert!(refused.contains("That costs "), "{refused}");
    let hold_id = hold
        .split_whitespace()
        .find_map(|word| {
            word.trim_matches(|c: char| !c.is_ascii_digit())
                .parse::<u64>()
                .ok()
        })
        .unwrap_or_else(|| panic!("a hold id: {hold}"));
    let released = run(
        &mut world.rig.tw,
        captain,
        &format!("@hold/release {hold_id}"),
    );
    assert!(!released.contains("Usage"), "{released}");

    let ordered = run(&mut world.rig.tw, captain, "+core/buy II go");
    assert!(
        ordered.contains("The counter takes your order"),
        "the spare die needs nothing from the depot: {ordered}"
    );
    let heard = pump(&mut world, captain, 60, |w| at_work(w, w.pilot));
    assert!(heard.contains(head(CORE_ACCEPTED)), "{heard}");
    assert_eq!(
        (depot(&mut world, BOARDS), depot(&mut world, ORE)),
        (boards, 0),
        "no second die was cut"
    );
}

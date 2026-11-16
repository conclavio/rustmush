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
    // A restart reads the depot as it was left, and the work goes on.
    let mut world = world.reboot();
    assert_eq!(
        (depot(&mut world, BOARDS), depot(&mut world, ORE)),
        (DEPOT_STOCK - 2, 0)
    );
    assert!(
        at_work(&world, captain),
        "the order stands after the restart"
    );
}

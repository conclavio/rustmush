//! **Doc 39 cut IN.11 — the matter census** (§4.10; Q-39-8), over the composed world.

use mush_core::GOD;

use crate::driving_rig::{run, stocked};

/// The census line's terms, by name.
pub(crate) fn census(
    rig: &mut crate::driving_rig::Rig,
) -> std::collections::BTreeMap<String, i128> {
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

/// The build lines: `(order, inputs, hull, waste)` a commission, kilograms.
pub(crate) fn builds(rig: &mut crate::driving_rig::Rig) -> Vec<(i128, i128, i128, i128)> {
    let out = run(&mut rig.tw, GOD, "@industry/census");
    out.lines()
        .filter(|l| l.contains(tacsim::port::census::BUILD_LINE))
        .map(|line| {
            let terms: std::collections::BTreeMap<&str, i128> = line
                .split_whitespace()
                .filter_map(|word| word.split_once('='))
                .map(|(k, v)| (k, v.parse().expect("a number")))
                .collect();
            (
                terms["order"],
                terms["inputs"],
                terms["hull"],
                terms["waste"],
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in11_probe_the_census_holds_still_on_a_settled_world() {
    let mut rig = stocked();
    for _ in 0..40 {
        let t = rig.tw.world().now().saturating_add(6);
        rig.tw.set_now(t);
        let _ = rig.tw.fire_due(t.saturating_mul(1_000));
        let _ = crate::driving_rig::drain(&mut rig.tw);
    }
    let a = census(&mut rig);
    eprintln!("in11 A {a:?}");
    for _ in 0..40 {
        let t = rig.tw.world().now().saturating_add(6);
        rig.tw.set_now(t);
        let _ = rig.tw.fire_due(t.saturating_mul(1_000));
        let _ = crate::driving_rig::drain(&mut rig.tw);
    }
    let b = census(&mut rig);
    eprintln!("in11 B {b:?}");
    assert_eq!(a["balance"], b["balance"]);
}

/// A captain with an account and no hull, on the Service Counter's floor at Stickney.
fn free_captain(rig: &mut crate::driving_rig::Rig) -> mush_core::ObjId {
    let captain = crate::driving_rig::pilot(rig, "Matter", 7);
    let out = run(&mut rig.tw, captain, "+coop/enrol Long Ox");
    assert!(out.contains("now the Long Ox"), "{out}");
    let _ = run(&mut rig.tw, captain, "+coop/release");
    let treasury = crate::driving_rig::port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let account = tacsim::port::credit::own_account(rig.tw.world(), captain)
        .map(|a| a.id)
        .expect("the captain's account");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={} 10000000", account.0),
    );
    let floor = crate::driving_rig::station_room(&rig.tw, "stickney_port", "room.p11_service");
    crate::driving_rig::teleport(&mut rig.tw, captain, floor);
    crate::in10_pools::settle(rig, 2);
    captain
}

/// **The failure between the inputs' consumption and the hull's title, under a restart**
/// (§4.9's adversarial set): an escort ordered at the yard, its bill laid in the escrow (the
/// seed), built, and collected; the consumption lands and the world restarts before the port
/// fire lays her up. After the restart the commission resumes from its record and finishes once:
/// one hull, one title, one register row, and the census across the window holds still.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn in11_a_restart_between_consumption_and_title_finishes_the_hull_once() {
    use tacsim::port::commission::Step;
    let mut rig = stocked();
    let captain = free_captain(&mut rig);
    let out = run(&mut rig.tw, captain, "+yard/order tv_escort");
    assert!(out.contains("booked at the yard"), "{out}");
    let _ = crate::in10_pools::pump(&mut rig);
    crate::in10_pools::settle(&mut rig, 1);
    let order = crate::driving_rig::store(&rig.tw)
        .works_contracts()
        .into_iter()
        .find(|c| c.requester == captain)
        .map(|c| c.id)
        .expect("the order");
    let escrow = mush_economy::VaultId(order);
    crate::driving_rig::seed_pool(
        &mut rig,
        escrow,
        "structural_alloys",
        3,
        mush_inventory::LotId(9_810),
    );
    crate::driving_rig::seed_pool(
        &mut rig,
        escrow,
        "industrial_parts",
        1,
        mush_inventory::LotId(9_811),
    );
    for (i, part) in [
        "tacsim.part.weapon.cannon_105",
        "tacsim.part.weapon.mg_coax",
        "tacsim.part.sensor.visual_mk1",
        "tacsim.part.plant.vehicle_mk1",
    ]
    .iter()
    .enumerate()
    {
        crate::driving_rig::seed_lot(
            &mut rig,
            escrow,
            part,
            1,
            mush_inventory::LotId(9_812 + i as u64),
        );
    }
    crate::in10_pools::settle(&mut rig, 2);
    // The window opens once the seeds stand (a seed is a mint the census cannot see).
    let before = census(&mut rig);
    crate::in10_pools::days_on(&mut rig, 3);
    // The collection: the commission's record, then the consumption, landed; no fire after.
    let out = run(&mut rig.tw, captain, &format!("+yard/collect {order}"));
    assert!(out.contains(tacsim::port::works::COMMISSIONS_AS), "{out}");
    let refused = crate::in10_pools::pump(&mut rig);
    assert!(refused.is_empty(), "{refused:?}");
    let record = tacsim::port::commission::commission(rig.tw.world(), order)
        .cloned()
        .expect("the commission");
    assert!(
        matches!(record.step, Step::Consuming | Step::Laying),
        "between the consumption and the title: {record:?}"
    );
    assert!(
        record
            .object()
            .is_none_or(|hull| { tacsim::port::owned::owned(rig.tw.world(), hull).is_none() })
    );
    // The restart.
    let mut rig = crate::driving_rig::reboot(rig, &[captain]);
    crate::in10_pools::settle(&mut rig, 6);
    let done = tacsim::port::commission::commission(rig.tw.world(), order)
        .cloned()
        .expect("the commission after the restart");
    assert_eq!(done.step, Step::Done, "{done:?}");
    let hull = done.object().expect("the escort");
    assert!(
        tacsim::port::owned::owned(rig.tw.world(), hull).is_some(),
        "one register row"
    );
    assert_eq!(
        tacsim::port::owned::rows(rig.tw.world())
            .filter(|row| row.provenance == order)
            .count(),
        1,
        "the commission made one hull"
    );
    let after = census(&mut rig);
    eprintln!("in11 restart before {before:?}\nin11 restart after {after:?}");
    assert_eq!(
        before["balance"], after["balance"],
        "matter is neither made nor lost"
    );
    let build = builds(&mut rig);
    assert!(
        build
            .iter()
            .all(|(_, inputs, dry, waste)| inputs == &(dry + waste)),
        "inputs = the hull + the waste: {build:?}"
    );
}

/// **Two orders for one bale** (§4.9's adversarial set): with a patch taken on the heavy and its
/// bale opened by it, a second `+bay/order` on the same hull is refused while the first stands, so
/// no two orders ever draw one bale; once the first is done the second draws the bale alone, and
/// the hold gives up its tonne once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in11_two_orders_for_one_bale_draw_it_once() {
    use crate::driving_rig::{
        board, file_and_launch, hold_pool, hold_vault_of, load, seed_hold, to_ic,
    };
    use crate::in1_accounting::{dent_one, done, record, two_titled_hulls};
    let mut rig = stocked();
    let (pilot, row, _escort, heavy) = two_titled_hulls(&mut rig);
    let hull = mush_core::ObjId(row.hull);
    let vault = hold_vault_of(&rig.tw, &row);
    seed_hold(&mut rig, &row, "structural_alloys", 2, 9_700);
    seed_hold(&mut rig, &row, "industrial_parts", 2, 9_701);
    load(&mut rig, &row, 60, 40, 9_500);
    board(&mut rig, pilot);
    let t = file_and_launch(&mut rig, pilot, hull, "+ship/undock stickney");
    let _ = to_ic(&mut rig, t.plan.depart_ic + t.plan.total_s);
    let _ = to_ic(&mut rig, t.plan.depart_ic + t.plan.total_s + 3_600);
    // The heavy berthed at the Service Counter.
    crate::in1_accounting::put(
        &mut rig,
        heavy,
        mush_core::ObjId(row.rooms[&crate::driving_rig::cid("bay")]),
        true,
    );
    let workshop = crate::driving_rig::station_room(&rig.tw, "stickney_port", "room.p11_service");
    crate::in1_accounting::put(&mut rig, pilot, workshop, false);
    let berthed = run(&mut rig.tw, pilot, &format!("+bay/berth #{}", heavy.0));
    assert!(berthed.contains("berthed in the workshop"), "{berthed}");
    dent_one(&mut rig, heavy);
    let first = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/order #{}=armor", heavy.0),
    );
    assert!(first.contains("taken in the workshop"), "{first}");
    // The second order on the same hull, while the first stands: refused, nothing drawn.
    let second = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/order #{}=armor", heavy.0),
    );
    eprintln!("in11 second order: {second}");
    assert!(second.contains("held for work in progress"), "{second}");
    let one = done(&mut rig, heavy);
    assert_eq!(one.goods.len(), 2, "the first opens the bale: {one:?}");
    let opened = record(&rig, heavy).bale.expect("the bale");
    assert_eq!(hold_pool(&mut rig.tw, vault, "structural_alloys"), 1);
    // Now the second, alone: it draws the bale, never the hold.
    dent_one(&mut rig, heavy);
    let again = run(
        &mut rig.tw,
        pilot,
        &format!("+bay/order #{}=armor", heavy.0),
    );
    assert!(again.contains("taken in the workshop"), "{again}");
    let two = done(&mut rig, heavy);
    assert!(two.goods.is_empty(), "the bale covers the second: {two:?}");
    assert_eq!(hold_pool(&mut rig.tw, vault, "structural_alloys"), 1);
    assert_eq!(hold_pool(&mut rig.tw, vault, "industrial_parts"), 1);
    let after = record(&rig, heavy).bale.expect("the bale");
    assert!(
        after.alloys_kg < opened.alloys_kg && after.fittings_kg < opened.fittings_kg,
        "the second draws the bale down: {opened:?} → {after:?}"
    );
}

/// **A part lot sells into a standing bid** (doc 39 cut IN.11, lane v30's light-plant bid at
/// Stickney Yards): `+market/sell <bid>=1` hands the very lot over to the Yards' stores, pays
/// the bid's price, and the census holds across the sale; a second `=2` is refused (one part a
/// sale).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in11_a_light_plant_lot_sells_into_the_yards_bid() {
    use crate::driving_rig::{
        board, cid, drain, file_and_launch, hold_vault_of, land_all, land_quietly, load, port_map,
        sweep, to_ic,
    };
    use crate::in1_accounting::two_titled_hulls;
    const PLANT: &str = "tacsim.part.plant.light_mk1";
    let mut rig = stocked();
    let (pilot, row, _escort, _heavy) = two_titled_hulls(&mut rig);
    let hull = mush_core::ObjId(row.hull);
    let vault = hold_vault_of(&rig.tw, &row);
    // One plant sails aboard (a part in the hold weighs in the plan and the burn alike).
    crate::driving_rig::seed_lot(&mut rig, vault, PLANT, 1, mush_inventory::LotId(9_900));
    load(&mut rig, &row, 60, 40, 9_500);
    crate::in10_pools::settle(&mut rig, 2);
    board(&mut rig, pilot);
    let t = file_and_launch(&mut rig, pilot, hull, "+ship/undock stickney");
    let _ = to_ic(&mut rig, t.plan.depart_ic + t.plan.total_s);
    let _ = to_ic(&mut rig, t.plan.depart_ic + t.plan.total_s + 3_600);
    crate::in10_pools::settle(&mut rig, 4);
    // A second at Stickney (the seeds; made at the works in the bot's story).
    crate::driving_rig::seed_lot(&mut rig, vault, PLANT, 1, mush_inventory::LotId(9_901));
    crate::in10_pools::settle(&mut rig, 2);
    let ashore = run(&mut rig.tw, pilot, "+ship/leave");
    assert!(!ashore.contains("Usage"), "{ashore}");
    let before = census(&mut rig);
    let yards = port_map(&rig.tw, "stickney_port").vendors[&cid("stickney_yards")].clone();
    let bid = yards.bids[&cid(PLANT)].listing;
    let two = run(&mut rig.tw, pilot, &format!("+market/sell {bid}=2"));
    assert!(two.contains(tacsim_market_one_lot()), "{two}");
    let out = run(&mut rig.tw, pilot, &format!("+market/sell {bid}=1"));
    assert!(out.contains("Sold "), "{out}");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    for _ in 0..12 {
        sweep(&mut rig.tw, 31);
        let queued = drain(&mut rig.tw);
        land_quietly(&mut rig, &queued);
    }
    let lots_at = |rig: &mut crate::driving_rig::Rig, v: u64| {
        lots_of(&rig.tw, mush_economy::VaultId(v), PLANT)
    };
    assert_eq!(lots_at(&mut rig, vault.0), 1, "one plant left aboard");
    assert_eq!(
        lots_at(&mut rig, yards.vault),
        1,
        "the lot is in the Yards' stores"
    );
    let after = census(&mut rig);
    assert_eq!(
        before["balance"], after["balance"],
        "a sale makes and loses nothing"
    );
}

/// mush-market's one-lot refusal.
fn tacsim_market_one_lot() -> &'static str {
    mush_market::ONE_LOT_A_SALE
}

/// The lots of `good` at `vault`.
fn lots_of(tw: &mush_testkit::TestWorld, vault: mush_economy::VaultId, good: &str) -> usize {
    let inventory = tw
        .world()
        .module_state::<tacsim::port::PortHandles>()
        .and_then(|h| h.inventory.clone())
        .expect("the inventory families");
    let endpoint = mush_inventory::record::EndpointRef(vault);
    let state = tw.engine().action_state();
    let mut n = 0;
    let mut cursor: Option<String> = None;
    loop {
        let page = state
            .scan_family(&inventory.lots, cursor.as_deref(), 256)
            .expect("the lots read");
        n += page
            .entries()
            .iter()
            .filter(|e| e.value().endpoint() == endpoint && e.value().commodity().as_str() == good)
            .count();
        match page.next_after().map(ToString::to_string) {
            Some(next) => cursor = Some(next),
            None => return n,
        }
    }
}

/// One order at the front `verb` (`+works` or `+yard`) run through: booked, its bill delivered
/// from the captain's docked hold line by line, the clock stepped an IC hour at a time until the
/// arm delivers it, and collected; the order's number.
fn work(
    rig: &mut crate::driving_rig::Rig,
    captain: mush_core::ObjId,
    verb: &str,
    recipe: &str,
    bill: &[(&str, u64)],
) -> u64 {
    use crate::in5_production::{contract, ic_to, works, works_fire};
    let before: Vec<u64> = crate::driving_rig::store(&rig.tw)
        .works_contracts()
        .into_iter()
        .map(|c| c.id)
        .collect();
    let booked = works(rig, captain, &format!("{verb}/order {recipe}"), 1);
    assert!(booked.contains(" booked at the "), "{booked}");
    let id = crate::driving_rig::store(&rig.tw)
        .works_contracts()
        .into_iter()
        .map(|c| c.id)
        .find(|id| !before.contains(id))
        .expect("the order");
    for (good, units) in bill {
        let out = works(
            rig,
            captain,
            &format!("{verb}/deliver {id}={good} {units}"),
            1,
        );
        assert!(
            out.contains(tacsim::port::works::INTO_ESCROW),
            "{good}: {out}"
        );
    }
    let start = crate::driving_rig::ic_now(&rig.tw);
    for hour in 1..=24 * 6 {
        if contract(rig, id).delivered_ic.is_some() {
            break;
        }
        ic_to(rig, start + hour * 3_600);
        let _ = works_fire(rig);
    }
    assert!(
        contract(rig, id).delivered_ic.is_some(),
        "{recipe} is delivered: {:?}",
        contract(rig, id)
    );
    for _ in 0..4 {
        let out = works(rig, captain, &format!("{verb}/collect {id}"), 2);
        if !out.contains(tacsim::port::works::SETTLING) {
            break;
        }
    }
    id
}

/// **The port half of the matter story** (doc 39 cut IN.11, §4.9: the chain's stories are
/// port-side, so the port half runs once and the mining half at both sizes, in the bot's
/// `in11_` stories): ore → concentrate → alloys (Kasei's refine), the alloys and the carbon →
/// fittings, fittings and carbon → light plants, strategic ore, boards and fittings → sights, and
/// alloys, fittings, a plant and a sight → a prospector at the yard; a light plant sold into the
/// Yards' bid and another fitted into the prospector's destroyed plant slot. The census holds
/// across the whole window and every build reads inputs = the hull + the waste.
///
/// **Seeds**: the captain's money, the passage's propellant and provisions, and one tonne of
/// instrument boards in the hold (Crisium's ask is at Luna, off this passage; the bot's story
/// buys it there), all before the census's window opens; the plant destroyed by staff before
/// the fit (a fight's stand-in).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn in11_the_port_half_ore_to_parts_to_a_prospector_conserves_matter() {
    use crate::driving_rig::{
        board, cid, hold_pool, hold_vault_of, port_map, prospector, seed_pool, station_room,
        teleport,
    };
    use crate::in5_production::{ashore, buy, fire_landing, listed, mint, sail, sell};
    const CERES: &str = "ceres_port";
    const STICKNEY: &str = "stickney_port";
    const PLANT: &str = "tacsim.part.plant.light_mk1";
    const SIGHT: &str = "tacsim.part.sensor.visual_mk1";
    let mut rig = stocked();
    let (captain, row) = prospector(&mut rig, "Ore", 3, "Assay");
    let _ = mint(&mut rig, captain);
    let hold = hold_vault_of(&rig.tw, &row);
    seed_pool(
        &mut rig,
        hold,
        "instrument_boards",
        1,
        mush_inventory::LotId(9_950),
    );
    crate::in10_pools::settle(&mut rig, 3);
    // ---- The window opens ----
    let before = census(&mut rig);

    // ---- Ceres: concentrate, carbon and strategic ore at the asks ----
    let exchange = station_room(&rig.tw, CERES, "room.c07_exchange");
    teleport(&mut rig.tw, captain, exchange);
    buy(&mut rig, captain, CERES, "freeport_exchange", "metals", 20);
    buy(&mut rig, captain, CERES, "freeport_works", "carbon_ore", 2);
    buy(
        &mut rig,
        captain,
        CERES,
        "freeport_works",
        "strategic_ore",
        2,
    );

    // ---- Stickney: the concentrate into Kasei's bid, refined, and the alloys bought ----
    board(&mut rig, captain);
    sail(
        &mut rig,
        captain,
        &row,
        "+ship/undock stickney",
        STICKNEY,
        9_500,
    );
    ashore(&mut rig, captain);
    let kasei = station_room(&rig.tw, STICKNEY, "room.p08_exchange");
    teleport(&mut rig.tw, captain, kasei);
    for _ in 0..20 {
        if listed(&rig, STICKNEY, "kasei", "structural_alloys").is_some() {
            break;
        }
        fire_landing(&mut rig);
    }
    let alloys_before =
        listed(&rig, STICKNEY, "kasei", "structural_alloys").expect("Kasei's alloy ask");
    let _ = sell(&mut rig, captain, STICKNEY, "kasei", "metals");
    for _ in 0..40 {
        if listed(&rig, STICKNEY, "kasei", "structural_alloys") >= Some(alloys_before + 16) {
            break;
        }
        fire_landing(&mut rig);
    }
    buy(
        &mut rig,
        captain,
        STICKNEY,
        "kasei",
        "structural_alloys",
        10,
    );
    buy(
        &mut rig,
        captain,
        STICKNEY,
        "stickney_yards",
        "industrial_parts",
        1,
    );

    // ---- The works: fittings, light plants, sights ----
    let _ = work(
        &mut rig,
        captain,
        "+works",
        "fittings",
        &[("structural_alloys", 5), ("carbon_ore", 1)],
    );
    let _ = work(
        &mut rig,
        captain,
        "+works",
        "plant_light",
        &[("industrial_parts", 1), ("carbon_ore", 1)],
    );
    let _ = work(
        &mut rig,
        captain,
        "+works",
        "sight_visual",
        &[
            ("strategic_ore", 2),
            ("instrument_boards", 1),
            ("industrial_parts", 3),
        ],
    );
    assert_eq!(lots_of(&rig.tw, hold, PLANT), 5, "five light plants aboard");
    assert_eq!(lots_of(&rig.tw, hold, SIGHT), 10, "ten sights aboard");

    // ---- The yard: a prospector from the captain's own matter ----
    let counter = station_room(&rig.tw, STICKNEY, "room.p11_service");
    teleport(&mut rig.tw, captain, counter);
    let order = work(
        &mut rig,
        captain,
        "+yard",
        "tv_prospector",
        &[
            ("structural_alloys", 5),
            ("industrial_parts", 2),
            (PLANT, 1),
            (SIGHT, 1),
        ],
    );
    crate::in10_pools::settle(&mut rig, 8);
    let built = tacsim::port::commission::commission(rig.tw.world(), order)
        .cloned()
        .expect("the commission");
    assert_eq!(
        built.step,
        tacsim::port::commission::Step::Done,
        "{built:?}"
    );
    let rig_obj = built.object().expect("the prospector");
    assert_eq!(hold_pool(&mut rig.tw, hold, "structural_alloys"), 0);
    assert_eq!(hold_pool(&mut rig.tw, hold, "industrial_parts"), 0);

    // ---- A light plant sold into the Yards' bid ----
    let bid = port_map(&rig.tw, STICKNEY).vendors[&cid("stickney_yards")].bids[&cid(PLANT)].listing;
    let out = run(&mut rig.tw, captain, &format!("+market/sell {bid}=1"));
    assert!(out.contains("Sold "), "{out}");
    let _ = crate::in10_pools::pump(&mut rig);
    for _ in 0..12 {
        crate::driving_rig::sweep(&mut rig.tw, 31);
        let _ = crate::in10_pools::pump(&mut rig);
    }
    assert_eq!(
        lots_of(&rig.tw, hold, PLANT),
        3,
        "one in the prospector, one sold"
    );

    // ---- The fit: the prospector's plant destroyed (staff), a plant fitted from the hold ----
    crate::driving_rig::staff_hand(&mut rig, |ctx| {
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::SetComponentCondition {
                component: mush_tactics::unit::ComponentId::new("power").expect("an id"),
                condition: mush_tactics::ConditionId::new("destroyed").expect("a condition"),
            }],
            None,
            Some(smol_str::SmolStr::new_static("test:in11-fit")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, rig_obj, &plan).expect("applied");
    });
    let mark = built.mark.clone();
    // Into the captain's Dray-P's bay, whose hold the fit draws from (IN.9's stow).
    let stowed = run(
        &mut rig.tw,
        captain,
        &format!("+bay/stow {mark}=#{}", row.hull),
    );
    assert!(stowed.contains("is stowed in the"), "{stowed}");
    crate::in10_pools::settle(&mut rig, 2);
    // Berthed at the Service Counter from her carrier, whose hold the work draws from.
    let counter = station_room(&rig.tw, STICKNEY, "room.p11_service");
    teleport(&mut rig.tw, captain, counter);
    let berthed = run(&mut rig.tw, captain, &format!("+bay/berth #{}", rig_obj.0));
    assert!(berthed.contains("berthed in the workshop"), "{berthed}");
    crate::in10_pools::settle(&mut rig, 2);
    let quote = run(&mut rig.tw, captain, &format!("+bay #{}", rig_obj.0));
    eprintln!("in11 +bay {mark}:\n{quote}");
    let out = run(
        &mut rig.tw,
        captain,
        &format!("+bay/fit #{}=power {PLANT}", rig_obj.0),
    );
    assert!(out.contains("taken in the workshop"), "{out}");
    crate::in10_pools::days_on(&mut rig, 1);
    crate::in10_pools::settle(&mut rig, 4);
    assert_eq!(
        lots_of(&rig.tw, hold, PLANT),
        2,
        "a plant fitted from the hold"
    );

    // ---- The window closes ----
    let after = census(&mut rig);
    eprintln!("in11 port half before {before:?}\nin11 port half after {after:?}");
    assert_eq!(
        before["balance"], after["balance"],
        "matter is neither made nor lost"
    );
    let build = builds(&mut rig);
    assert!(
        build.iter().any(|(o, ..)| *o == i128::from(order)),
        "the prospector's build: {build:?}"
    );
    assert!(
        build
            .iter()
            .all(|(_, inputs, dry, waste)| *inputs == dry + waste),
        "inputs = the hull + the waste: {build:?}"
    );
}

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

/// The build lines: `(inputs, hull, waste)` a commission, kilograms.
pub(crate) fn builds(rig: &mut crate::driving_rig::Rig) -> Vec<(i128, i128, i128)> {
    let out = run(&mut rig.tw, GOD, "@industry/census");
    out.lines()
        .filter(|l| l.contains(tacsim::port::census::BUILD_LINE))
        .map(|line| {
            let terms: std::collections::BTreeMap<&str, i128> = line
                .split_whitespace()
                .filter_map(|word| word.split_once('='))
                .map(|(k, v)| (k, v.parse().expect("a number")))
                .collect();
            (terms["inputs"], terms["hull"], terms["waste"])
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
            .all(|(inputs, dry, waste)| inputs == &(dry + waste)),
        "inputs = the hull + the waste: {build:?}"
    );
}

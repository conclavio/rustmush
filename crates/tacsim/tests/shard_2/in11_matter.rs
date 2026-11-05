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

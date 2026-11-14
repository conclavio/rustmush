#![cfg(test)]

//! Doc 46 CT.7: the core refit's validator gains the bay refit's scenarios where the core refit
//! has the same mechanism (`bay_refit/tests.rs`). The two validators are not one (their files
//! are about half alike; the cut note says why), so these are the core refit's own, on CT.0's
//! rig (`ct0_snapshot.rs`). The bay's lot, part and census scenarios have no core counterpart;
//! the core refit's Accept refusals, its displaced core and its Cancel are walked by CT.0.
use super::ct0_snapshot::{Order, accept, defs, reference, rig};
use super::*;
use mush_testkit::snapshot::Snapshot;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// The protocol is pinned to its three participants and four actions, in order, under Industry.
#[test]
fn ud4b_the_protocol_pins_its_participants_and_actions() -> TestResult {
    let d = defs()?;
    let industry = owner_id()?;
    Protocol::new(&industry, d.participants.clone(), d.actions.clone())?;
    let [a, b, c] = d.participants.clone();
    assert!(
        Protocol::new(
            &industry,
            [b.clone(), a.clone(), c.clone()],
            d.actions.clone()
        )
        .is_err()
    );
    let bay = reference("industry", "owner_participant", "bay_refit", 1)?;
    assert!(
        Protocol::new(&industry, [bay, b, c], d.actions.clone()).is_err(),
        "the bay refit's owner is not the core refit's"
    );
    let [w, x, y, z] = d.actions.clone();
    assert!(Protocol::new(&industry, d.participants.clone(), [x, w, y, z]).is_err());
    assert!(Protocol::new(&OwnerId::new("tacsim")?, d.participants, d.actions).is_err());
    Ok(())
}

/// A cold boot audits an accepted order against its reservation and its counterpart rows: each
/// one missing is refused, and so is an exchange receipt no installed order owns.
#[test]
fn ud4b_a_cold_boot_audits_the_order_and_its_counterparts() -> TestResult {
    let d = defs()?;
    let mut w = rig(&d)?;
    let order = Order {
        n: 1,
        at: 0,
        journal: 10,
        byte: 1,
        revision: 3,
        incoming: 50,
        displaced: None,
    };
    let mut snap = Snapshot::new();
    accept(&mut snap, &mut w, &d, "accepted", &order)?;
    w.cold(&mut snap)?;
    let mut bare = rig(&d)?;
    bare.rows = w.rows.clone();
    assert!(
        bare.cold(&mut snap).is_err(),
        "an order without its reservation"
    );
    for family in [ASSET, HULL, SCHEDULE, JOB] {
        let mut short = rig(&d)?;
        short.reservations = w.reservations.clone();
        short.rows = w
            .rows
            .iter()
            .filter(|(k, _)| k.family.as_str() != family)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        assert!(short.cold(&mut snap).is_err(), "{family} missing");
    }
    let mut orphan = rig(&d)?;
    orphan.reservations = w.reservations.clone();
    orphan.rows = w.rows.clone();
    let k = key(EXCHANGE, format!("{:016x}", 9))?;
    orphan
        .rows
        .insert(k.resolve(&orphan.reg)?, (k, json!({"format": 1})));
    assert!(
        orphan.cold(&mut snap).is_err(),
        "an exchange receipt of no order"
    );
    Ok(())
}

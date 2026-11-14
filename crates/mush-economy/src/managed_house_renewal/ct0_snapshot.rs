#![cfg(test)]

//! Doc 46 CT.0: the House renewal twin's snapshot. The walk is the one both issuers run
//! (`crate::renewal_twin::tests::ct0_walk`), over the House rig (`super::tests`).
use crate::renewal_twin::House;
use crate::renewal_twin::tests::{TestResult, ct0_walk};

#[test]
fn ct0_renewal_house_snapshot() -> TestResult {
    let snap = ct0_walk::run::<House>()?;
    snap.check(env!("CARGO_MANIFEST_DIR"), "tests/ct0/renewal_house.snap")?;
    Ok(())
}

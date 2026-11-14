//! Doc 46 CT.0: the personal renewal twin's snapshot. The walk is the one both issuers run
//! (`crate::renewal_twin::tests::ct0_walk`).
use super::TestResult;
use crate::renewal_twin::Personal;
use crate::renewal_twin::tests::ct0_walk;

#[test]
fn ct0_renewal_personal_snapshot() -> TestResult {
    let snap = ct0_walk::run::<Personal>()?;
    snap.check(
        env!("CARGO_MANIFEST_DIR"),
        "tests/ct0/renewal_personal.snap",
    )?;
    Ok(())
}

//! Doc 46 CT.0: the House renewal walk, pinned in `tests/ct0/renewal_house.snap`. The walk
//! is shared with the personal twin (`renewal_twin/tests/ct0_walk.rs`).
use super::*;

#[test]
fn ct0_renewal_house_walk() -> TestResult {
    twin::ct0_walk::walk::<House>("house", "tests/ct0/renewal_house.snap")
}

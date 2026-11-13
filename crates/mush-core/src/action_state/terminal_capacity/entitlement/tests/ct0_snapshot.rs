//! Doc 46 CT.0: the personal purchase walk (OriginalV3, DerivedV1 and both Follow recipes),
//! pinned in `tests/ct0/purchase_personal.snap`. The walk is shared with the House twin
//! (`ct0_kit::purchase_walk!`).
use super::*;

crate::action_state::terminal_capacity::tests::ct0_kit::purchase_walk! {
    test: ct0_purchase_personal_walk,
    file: "tests/ct0/purchase_personal.snap",
    twin: "personal",
    protocol: entitlement_purchase,
    fixture: fixture_with_follow(0, 0, true),
    declare: |owner: &OwnerId, p: &Protocol| {
        Protocol::new(
            owner,
            p.manifest.clone(),
            p.participants.clone(),
            p.original.clone(),
            p.derived.clone(),
        )
    },
    modes: [Original, Derived, OriginalFollow, DerivedFollow],
    faults: [
        (MissingEconomics, true),
        (CrossedEconomics, true),
        (ChangedFunding, true),
        (RefundEconomics, false),
        (Clock, true),
        (ReusedFundingCommit, true),
    ],
}

//! Doc 46 CT.0: the House purchase walk (House, two-seat Pin and two-seat Follow), pinned in
//! `tests/ct0/purchase_house.snap`. The walk is shared with the personal twin
//! (`ct0_kit::purchase_walk!`).
use super::*;

crate::action_state::terminal_capacity::tests::ct0_kit::purchase_walk! {
    test: ct0_purchase_house_walk,
    file: "tests/ct0/purchase_house.snap",
    twin: "house",
    protocol: house_purchase,
    fixture: fixture_with_follow(0, 0, true, true),
    declare: |owner: &OwnerId, p: &Protocol| {
        Protocol::new(
            owner,
            p.manifest.clone(),
            p.participants.clone(),
            p.house.clone(),
        )
    },
    modes: [House, TwoSeatPin, TwoSeatFollow],
    faults: [
        (MissingEconomics, true),
        (CrossedEconomics, true),
        (ChangedFunding, true),
        (RefundEconomics, false),
        (Clock, true),
        (BackwardClock, true),
        (ReusedFundingCommit, true),
    ],
}

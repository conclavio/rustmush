//! Doc 46 CT.0: the House renewal walk, pinned in `tests/ct0/renewal_house.snap`. The walk
//! is shared with the personal twin (`ct0_kit::renewal_walk!`).
use super::*;

crate::action_state::terminal_capacity::tests::ct0_kit::renewal_walk! {
    test: ct0_renewal_house_walk,
    file: "tests/ct0/renewal_house.snap",
    twin: "house",
    protocol: house_mind_renewal,
    domains: house,
    follow_seed: house::tests::follow_renewal_birth_seed(),
}

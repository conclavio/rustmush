//! mushd's integration tests: group `sim_lenses` (test-strategy cut T.3, 2026-10-26).
//!
//! One test binary for the modules below. Each was a `tests/<name>.rs`
//! target of its own until the merge: every integration binary links the
//! whole engine (300-730 MB each), and 373 of them were 110 GB of the CI
//! box's target dir against 11 GB of compiled dependencies. A module is the
//! former file, unchanged but for its paths; its tests are `<module>::<fn>`.
//!
//! The simulator lenses over the all-in-one binary: the arena's custody and
//! story, the front door, the observer rule, the sim_* journeys (cockpit,
//! lobby, net, range, reflex, rematch, solo, units, weapons), typeahead and
//! attach_room. Several hold rows in state-slot-owners.toml under their new
//! paths.
//!
//! One module: `cargo nextest run -p mushd --test sim_lenses -E 'test(/^arena_custody::/)'`
//! or `cargo test -p mushd --test sim_lenses arena_custody::` (a substring filter, so no
//! module here may be a suffix of another's name — the merge refuses that).
//! One test: `... --test sim_lenses arena_custody::<fn> -- --exact`. Sweeps run under nextest,
//! one process per test; under plain multi-threaded `cargo test` these modules
//! share one process and its statics (locks, global subscribers).

mod support;

#[path = "sim_lenses/arena_custody.rs"]
mod arena_custody;
#[path = "sim_lenses/arena_story.rs"]
mod arena_story;
#[path = "sim_lenses/attach_room.rs"]
mod attach_room;
#[path = "sim_lenses/front_door.rs"]
mod front_door;
#[path = "sim_lenses/observer_rule.rs"]
mod observer_rule;
#[path = "sim_lenses/sim_cockpit.rs"]
mod sim_cockpit;
#[path = "sim_lenses/sim_lobby.rs"]
mod sim_lobby;
#[path = "sim_lenses/sim_net.rs"]
mod sim_net;
#[path = "sim_lenses/sim_range.rs"]
mod sim_range;
#[path = "sim_lenses/sim_reflex.rs"]
mod sim_reflex;
#[path = "sim_lenses/sim_rematch.rs"]
mod sim_rematch;
#[path = "sim_lenses/sim_solo.rs"]
mod sim_solo;
#[path = "sim_lenses/sim_units.rs"]
mod sim_units;
#[path = "sim_lenses/sim_weapons.rs"]
mod sim_weapons;
#[path = "sim_lenses/typeahead.rs"]
mod typeahead;

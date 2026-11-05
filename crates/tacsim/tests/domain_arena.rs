//! tacsim's integration tests: the `domain_arena` binary, the arena, cockpit, match lifecycle, minds and definitions.
//!
//! One of four domain binaries (doc 43 cut TS.12, adopted by D-43-5 on 2026-11-04). Each module
//! below was a `tests/<name>.rs` target of its own until test-strategy cut T.3 merged them (every
//! integration binary links the whole engine), and it lives at `tests/domain_arena/<name>.rs`. The CI's ten
//! `tacsim-integration-N` cells build all four binaries and run `cargo nextest run --partition
//! hash:N/10` over them, so the balance is the hash's and nothing is ever resharded: **a new test is
//! a `mod` line here**, in the domain it belongs to (rule 2.3 #3). A module imports only from its
//! own binary; the field helpers are the `*_rig` modules under tests/support/field/, declared in
//! every root.
//!
//! One module: `cargo nextest run -p tacsim --test domain_arena -E 'test(/^<module>::/)'` or
//! `cargo test -p tacsim --test domain_arena <module>::` (a substring filter, so no module here may be a
//! suffix of another's name; `ci_tacsim_shards` refuses that). One test: `... --test domain_arena
//! <module>::<fn> -- --exact`. Sweeps run under nextest, one process per test; under plain
//! multi-threaded `cargo test` these modules share one process and its statics.

#[path = "support/field/claim_rig.rs"]
mod claim_rig;
#[path = "support/field/custody_rig.rs"]
mod custody_rig;
#[path = "support/field/driving_rig.rs"]
mod driving_rig;
#[path = "support/field/field_host_rig.rs"]
mod field_host_rig;
#[path = "support/field/field_raiders_rig.rs"]
mod field_raiders_rig;
#[path = "support/geology_keyed.rs"]
mod geology_keyed;
#[path = "support/field/grounds_rig.rs"]
mod grounds_rig;
#[path = "support/field/site_record_rig.rs"]
mod site_record_rig;
mod support;
#[path = "support/vertex.rs"]
mod vertex;

#[path = "domain_arena/arena.rs"]
mod arena;
#[path = "domain_arena/arena_task.rs"]
mod arena_task;
#[path = "domain_arena/capacity.rs"]
mod capacity;
#[path = "domain_arena/cockpit.rs"]
mod cockpit;
#[path = "domain_arena/composition_edges.rs"]
mod composition_edges;
#[path = "domain_arena/conditions.rs"]
mod conditions;
#[path = "domain_arena/content.rs"]
mod content;
#[path = "domain_arena/core_replacement.rs"]
mod core_replacement;
#[path = "domain_arena/definitions.rs"]
mod definitions;
#[path = "domain_arena/deliberation.rs"]
mod deliberation;
#[path = "domain_arena/designs.rs"]
mod designs;
#[path = "domain_arena/extensions.rs"]
mod extensions;
#[path = "domain_arena/field_scale.rs"]
mod field_scale;
#[path = "domain_arena/hulls.rs"]
mod hulls;
#[path = "domain_arena/match_lifecycle.rs"]
mod match_lifecycle;
#[path = "domain_arena/minds.rs"]
mod minds;
#[path = "domain_arena/minds_pool.rs"]
mod minds_pool;
#[path = "domain_arena/minds_rhai.rs"]
mod minds_rhai;
#[path = "domain_arena/motor.rs"]
mod motor;
#[path = "domain_arena/parties.rs"]
mod parties;
#[path = "domain_arena/perception.rs"]
mod perception;
#[path = "domain_arena/reflex.rs"]
mod reflex;
#[path = "domain_arena/replay.rs"]
mod replay;
#[path = "domain_arena/report.rs"]
mod report;
#[path = "domain_arena/resolution.rs"]
mod resolution;
#[path = "domain_arena/sheet.rs"]
mod sheet;
#[path = "domain_arena/shipped_scale.rs"]
mod shipped_scale;
#[path = "domain_arena/sight.rs"]
mod sight;
#[path = "domain_arena/twin.rs"]
mod twin;
#[path = "domain_arena/weapons.rs"]
mod weapons;

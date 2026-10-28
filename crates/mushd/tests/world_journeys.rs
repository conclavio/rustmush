//! mushd's integration tests: group `world_journeys` (test-strategy cut T.3, 2026-10-26).
//!
//! One test binary for the modules below. Each was a `tests/<name>.rs`
//! target of its own until the merge: every integration binary links the
//! whole engine (300-730 MB each), and 373 of them were 110 GB of the CI
//! box's target dir against 11 GB of compiled dependencies. A module is the
//! former file, unchanged but for its paths; its tests are `<module>::<fn>`.
//!
//! The world journeys over SSH and the web: admin, the clock, intel, legal
//! actions, the market and the original minds, the mind market's web twin
//! (17g cut C3c), modules, NPCs, onboarding,
//! perception, power use, scripting, weather, worldgen and the web bridge.
//! `npc_smoke` and `worldgen_smoke` are the owner-funded live-provider
//! opt-ins (RUN_LLM_SMOKE; ci_gated_test_inventory's PAID_OPT_INS names
//! them under this group).
//!
//! One module: `cargo nextest run -p mushd --test world_journeys -E 'test(/^admin::/)'`
//! or `cargo test -p mushd --test world_journeys admin::` (a substring filter, so no
//! module here may be a suffix of another's name — the merge refuses that).
//! One test: `... --test world_journeys admin::<fn> -- --exact`. Sweeps run under nextest,
//! one process per test; under plain multi-threaded `cargo test` these modules
//! share one process and its statics (locks, global subscribers).

mod support;

#[path = "world_journeys/admin.rs"]
mod admin;
#[path = "world_journeys/clock_e2e.rs"]
mod clock_e2e;
#[path = "world_journeys/intel_e2e.rs"]
mod intel_e2e;
#[path = "world_journeys/legal_action_web.rs"]
mod legal_action_web;
#[path = "world_journeys/market_original_minds_web.rs"]
mod market_original_minds_web;
#[path = "world_journeys/mind_market_web.rs"]
mod mind_market_web;
#[path = "world_journeys/modules_e2e.rs"]
mod modules_e2e;
#[path = "world_journeys/npc_e2e.rs"]
mod npc_e2e;
#[path = "world_journeys/npc_smoke.rs"]
mod npc_smoke;
#[path = "world_journeys/onboarding_e2e.rs"]
mod onboarding_e2e;
#[path = "world_journeys/perception_e2e.rs"]
mod perception_e2e;
#[path = "world_journeys/power_use_e2e.rs"]
mod power_use_e2e;
#[path = "world_journeys/scripting_e2e.rs"]
mod scripting_e2e;
#[path = "world_journeys/ssh_e2e.rs"]
mod ssh_e2e;
#[path = "world_journeys/weather_e2e.rs"]
mod weather_e2e;
#[path = "world_journeys/web_bridge.rs"]
mod web_bridge;
#[path = "world_journeys/web_e2e.rs"]
mod web_e2e;
#[path = "world_journeys/worldgen_e2e.rs"]
mod worldgen_e2e;
#[path = "world_journeys/worldgen_smoke.rs"]
mod worldgen_smoke;

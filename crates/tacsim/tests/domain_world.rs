//! tacsim's integration tests: the `domain_world` binary, knowing, vehicles, voyages, the chart, the sky and geology.
//!
//! One of four domain binaries (doc 43 cut TS.12, adopted by D-43-5 on 2026-11-04). Each module
//! below was a `tests/<name>.rs` target of its own until test-strategy cut T.3 merged them (every
//! integration binary links the whole engine), and it lives at `tests/domain_world/<name>.rs`. The CI's ten
//! `tacsim-integration-N` cells build all four binaries and run `cargo nextest run --partition
//! hash:N/10` over them, so the balance is the hash's and nothing is ever resharded: **a new test is
//! a `mod` line here**, in the domain it belongs to (rule 2.3 #3). A module imports only from its
//! own binary; the field helpers are the `*_rig` modules under tests/support/field/, declared in
//! every root.
//!
//! One module: `cargo nextest run -p tacsim --test domain_world -E 'test(/^<module>::/)'` or
//! `cargo test -p tacsim --test domain_world <module>::` (a substring filter, so no module here may be a
//! suffix of another's name; `ci_tacsim_shards` refuses that). One test: `... --test domain_world
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

#[path = "domain_world/chart.rs"]
mod chart;
#[path = "domain_world/ephemeris.rs"]
mod ephemeris;
#[path = "domain_world/geology.rs"]
mod geology;
#[path = "domain_world/ic_sortie_voyages.rs"]
mod ic_sortie_voyages;
#[path = "domain_world/knowing.rs"]
mod knowing;
#[path = "domain_world/minds_at_size.rs"]
mod minds_at_size;
#[path = "domain_world/moon_ports.rs"]
mod moon_ports;
#[path = "domain_world/passage.rs"]
mod passage;
#[path = "domain_world/s9_corridor.rs"]
mod s9_corridor;
#[path = "domain_world/sky.rs"]
mod sky;
#[path = "domain_world/vehicles.rs"]
mod vehicles;
#[path = "domain_world/voyage.rs"]
mod voyage;

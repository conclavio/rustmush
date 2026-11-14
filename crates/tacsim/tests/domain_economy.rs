//! tacsim's integration tests: the `domain_economy` binary, the surface, ports, market, claims, plans, mining, drones and industry.
//!
//! One of four domain binaries (doc 43 cut TS.12, adopted by D-43-5 on 2026-11-04). Each module
//! below was a `tests/<name>.rs` target of its own until test-strategy cut T.3 merged them (every
//! integration binary links the whole engine), and it lives at `tests/domain_economy/<name>.rs`. The CI's ten
//! `tacsim-integration-N` cells build all four binaries and run `cargo nextest run --partition
//! hash:N/10` over them, so the balance is the hash's and nothing is ever resharded: **a new test is
//! a `mod` line here**, in the domain it belongs to (rule 2.3 #3). A module imports only from its
//! own binary; the field helpers are the `*_rig` modules under tests/support/field/, declared in
//! every root.
//!
//! One module: `cargo nextest run -p tacsim --test domain_economy -E 'test(/^<module>::/)'` or
//! `cargo test -p tacsim --test domain_economy <module>::` (a substring filter, so no module here may be a
//! suffix of another's name; `ci_tacsim_shards` refuses that). One test: `... --test domain_economy
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

#[path = "domain_economy/claims.rs"]
mod claims;
#[path = "domain_economy/co9_house_hulls.rs"]
mod co9_house_hulls;
#[path = "domain_economy/commerce.rs"]
mod commerce;
#[path = "domain_economy/consent_rs10.rs"]
mod consent_rs10;
#[path = "domain_economy/coop.rs"]
mod coop;
#[path = "domain_economy/deposits.rs"]
mod deposits;
#[path = "domain_economy/drones.rs"]
mod drones;
#[path = "domain_economy/e7c1.rs"]
mod e7c1;
#[path = "domain_economy/e7r1.rs"]
mod e7r1;
#[path = "domain_economy/e7r2.rs"]
mod e7r2;
#[path = "domain_economy/economy.rs"]
mod economy;
#[path = "domain_economy/house_arm_rs6b.rs"]
mod house_arm_rs6b;
#[path = "domain_economy/house_resale_rs6c2.rs"]
mod house_resale_rs6c2;
#[path = "domain_economy/house_sweep_rs6c.rs"]
mod house_sweep_rs6c;
#[path = "domain_economy/in10_pools.rs"]
mod in10_pools;
#[path = "domain_economy/in11_matter.rs"]
mod in11_matter;
#[path = "domain_economy/in1_accounting.rs"]
mod in1_accounting;
#[path = "domain_economy/in2_lifecycle.rs"]
mod in2_lifecycle;
#[path = "domain_economy/in3_order_contract.rs"]
mod in3_order_contract;
#[path = "domain_economy/in5_production.rs"]
mod in5_production;
#[path = "domain_economy/in6_part_lot.rs"]
mod in6_part_lot;
#[path = "domain_economy/in7_part_recipes.rs"]
mod in7_part_recipes;
#[path = "domain_economy/in8_mass_scrap.rs"]
mod in8_mass_scrap;
#[path = "domain_economy/landing.rs"]
mod landing;
#[path = "domain_economy/lane.rs"]
mod lane;
#[path = "domain_economy/mill.rs"]
mod mill;
#[path = "domain_economy/mining.rs"]
mod mining;
#[path = "domain_economy/orders.rs"]
mod orders;
#[path = "domain_economy/plans.rs"]
mod plans;
#[path = "domain_economy/plumbing.rs"]
mod plumbing;
#[path = "domain_economy/port_security_rs7.rs"]
mod port_security_rs7;
#[path = "domain_economy/port_tick.rs"]
mod port_tick;
#[path = "domain_economy/ports.rs"]
mod ports;
#[path = "domain_economy/price.rs"]
mod price;
#[path = "domain_economy/quarry.rs"]
mod quarry;
#[path = "domain_economy/reset.rs"]
mod reset;
#[path = "domain_economy/sale.rs"]
mod sale;
#[path = "domain_economy/sites.rs"]
mod sites;
#[path = "domain_economy/stations.rs"]
mod stations;
#[path = "domain_economy/tycho_house_rs6a.rs"]
mod tycho_house_rs6a;
#[path = "domain_economy/unloading.rs"]
mod unloading;
#[path = "domain_economy/yard_in9.rs"]
mod yard_in9;

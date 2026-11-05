//! tacsim's integration tests: the `domain_economy` binary of doc 43 cut TS.12's partition
//! EXPERIMENT (a measurement branch, never merged). The ten shard roots are four
//! domain roots over the same files (no file moved); the ten CI cells run
//! `cargo nextest run --partition hash:k/10` over all four, so balance is the
//! hash's and resharding is never needed. What it costs is the link: every
//! cell builds all four binaries.

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

#[path = "shard_5/claims.rs"]
mod claims;
#[path = "shard_10/commerce.rs"]
mod commerce;
#[path = "shard_10/coop.rs"]
mod coop;
#[path = "shard_3/deposits.rs"]
mod deposits;
#[path = "shard_3/drones.rs"]
mod drones;
#[path = "shard_3/e7c1.rs"]
mod e7c1;
#[path = "shard_2/e7r1.rs"]
mod e7r1;
#[path = "shard_2/e7r2.rs"]
mod e7r2;
#[path = "shard_4/economy.rs"]
mod economy;
#[path = "shard_2/in10_pools.rs"]
mod in10_pools;
#[path = "shard_2/in1_accounting.rs"]
mod in1_accounting;
#[path = "shard_2/in2_lifecycle.rs"]
mod in2_lifecycle;
#[path = "shard_2/in3_order_contract.rs"]
mod in3_order_contract;
#[path = "shard_2/in5_production.rs"]
mod in5_production;
#[path = "shard_2/in6_part_lot.rs"]
mod in6_part_lot;
#[path = "shard_2/in7_part_recipes.rs"]
mod in7_part_recipes;
#[path = "shard_2/in8_mass_scrap.rs"]
mod in8_mass_scrap;
#[path = "shard_10/landing.rs"]
mod landing;
#[path = "shard_4/lane.rs"]
mod lane;
#[path = "shard_9/mill.rs"]
mod mill;
#[path = "shard_7/mining.rs"]
mod mining;
#[path = "shard_2/moon_ports.rs"]
mod moon_ports;
#[path = "shard_3/orders.rs"]
mod orders;
#[path = "shard_6/plans.rs"]
mod plans;
#[path = "shard_10/plumbing.rs"]
mod plumbing;
#[path = "shard_5/port_security_rs7.rs"]
mod port_security_rs7;
#[path = "shard_3/port_tick.rs"]
mod port_tick;
#[path = "shard_10/ports.rs"]
mod ports;
#[path = "shard_10/price.rs"]
mod price;
#[path = "shard_9/quarry.rs"]
mod quarry;
#[path = "shard_10/reset.rs"]
mod reset;
#[path = "shard_9/sale.rs"]
mod sale;
#[path = "shard_5/sites.rs"]
mod sites;
#[path = "shard_10/stations.rs"]
mod stations;
#[path = "shard_7/tycho_house_rs6a.rs"]
mod tycho_house_rs6a;
#[path = "shard_8/unloading.rs"]
mod unloading;
#[path = "shard_6/yard_in9.rs"]
mod yard_in9;

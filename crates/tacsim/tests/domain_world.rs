//! tacsim's integration tests: the `domain_world` binary of doc 43 cut TS.12's partition
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

#[path = "shard_4/chart.rs"]
mod chart;
#[path = "shard_3/ephemeris.rs"]
mod ephemeris;
#[path = "shard_4/geology.rs"]
mod geology;
#[path = "shard_10/ic_sortie_voyages.rs"]
mod ic_sortie_voyages;
#[path = "shard_4/knowing.rs"]
mod knowing;
#[path = "shard_1/passage.rs"]
mod passage;
#[path = "shard_1/s9_corridor.rs"]
mod s9_corridor;
#[path = "shard_4/sky.rs"]
mod sky;
#[path = "shard_6/vehicles.rs"]
mod vehicles;
#[path = "shard_10/voyage.rs"]
mod voyage;

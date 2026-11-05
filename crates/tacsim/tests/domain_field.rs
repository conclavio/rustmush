//! tacsim's integration tests: the `domain_field` binary of doc 43 cut TS.12's partition
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

#[path = "shard_1/bay_36a.rs"]
mod bay_36a;
#[path = "shard_1/bay_gc8.rs"]
mod bay_gc8;
#[path = "shard_8/crew_custody.rs"]
mod crew_custody;
#[path = "shard_1/crew_exit.rs"]
mod crew_exit;
#[path = "shard_1/crew_treatment.rs"]
mod crew_treatment;
#[path = "shard_1/driving.rs"]
mod driving;
#[path = "shard_8/escort_lease.rs"]
mod escort_lease;
#[path = "shard_9/excursions.rs"]
mod excursions;
#[path = "shard_10/field_carrier.rs"]
mod field_carrier;
#[path = "shard_1/field_crash.rs"]
mod field_crash;
#[path = "shard_9/field_director.rs"]
mod field_director;
#[path = "shard_2/field_fr1.rs"]
mod field_fr1;
#[path = "shard_2/field_fr2.rs"]
mod field_fr2;
#[path = "shard_10/field_fr4.rs"]
mod field_fr4;
#[path = "shard_10/field_fr5.rs"]
mod field_fr5;
#[path = "shard_1/field_gc10.rs"]
mod field_gc10;
#[path = "shard_1/field_gc2.rs"]
mod field_gc2;
#[path = "shard_1/field_gc3.rs"]
mod field_gc3;
#[path = "shard_1/field_gc4.rs"]
mod field_gc4;
#[path = "shard_1/field_gc5.rs"]
mod field_gc5;
#[path = "shard_1/field_gc6.rs"]
mod field_gc6;
#[path = "shard_1/field_host.rs"]
mod field_host;
#[path = "shard_1/field_raiders.rs"]
mod field_raiders;
#[path = "shard_10/field_rs1.rs"]
mod field_rs1;
#[path = "shard_10/field_rs2.rs"]
mod field_rs2;
#[path = "shard_10/field_rs5.rs"]
mod field_rs5;
#[path = "shard_9/field_story.rs"]
mod field_story;
#[path = "shard_9/ic_sortie_surface.rs"]
mod ic_sortie_surface;
#[path = "shard_9/m8_leaks.rs"]
mod m8_leaks;
#[path = "shard_9/m8_world.rs"]
mod m8_world;
#[path = "shard_1/one_picture.rs"]
mod one_picture;
#[path = "shard_1/review_36a.rs"]
mod review_36a;
#[path = "shard_2/salvage_close.rs"]
mod salvage_close;
#[path = "shard_1/site_record.rs"]
mod site_record;
#[path = "shard_2/testimony.rs"]
mod testimony;
#[path = "shard_2/twin.rs"]
mod twin;
#[path = "shard_7/vacated_return_rs4.rs"]
mod vacated_return_rs4;

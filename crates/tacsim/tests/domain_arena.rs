//! tacsim's integration tests: the `domain_arena` binary of doc 43 cut TS.12's partition
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

#[path = "shard_10/arena.rs"]
mod arena;
#[path = "shard_4/arena_task.rs"]
mod arena_task;
#[path = "shard_3/capacity.rs"]
mod capacity;
#[path = "shard_7/cockpit.rs"]
mod cockpit;
#[path = "shard_3/composition_edges.rs"]
mod composition_edges;
#[path = "shard_4/conditions.rs"]
mod conditions;
#[path = "shard_2/content.rs"]
mod content;
#[path = "shard_4/core_replacement.rs"]
mod core_replacement;
#[path = "shard_3/definitions.rs"]
mod definitions;
#[path = "shard_10/deliberation.rs"]
mod deliberation;
#[path = "shard_3/designs.rs"]
mod designs;
#[path = "shard_2/extensions.rs"]
mod extensions;
#[path = "shard_1/field_scale.rs"]
mod field_scale;
#[path = "shard_8/hulls.rs"]
mod hulls;
#[path = "shard_8/match_lifecycle.rs"]
mod match_lifecycle;
#[path = "shard_2/minds.rs"]
mod minds;
#[path = "shard_2/minds_pool.rs"]
mod minds_pool;
#[path = "shard_2/minds_rhai.rs"]
mod minds_rhai;
#[path = "shard_10/motor.rs"]
mod motor;
#[path = "shard_4/parties.rs"]
mod parties;
#[path = "shard_1/perception.rs"]
mod perception;
#[path = "shard_2/reflex.rs"]
mod reflex;
#[path = "shard_1/replay.rs"]
mod replay;
#[path = "shard_3/report.rs"]
mod report;
#[path = "shard_4/resolution.rs"]
mod resolution;
#[path = "shard_1/sheet.rs"]
mod sheet;
#[path = "shard_1/shipped_scale.rs"]
mod shipped_scale;
#[path = "shard_2/sight.rs"]
mod sight;
#[path = "shard_2/weapons.rs"]
mod weapons;

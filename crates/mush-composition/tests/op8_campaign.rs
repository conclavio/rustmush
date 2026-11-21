//! **Doc 50 cut OP.8: the empty run and the second acceptance story** (Q-50-47, with its
//! correction and the owner's amendment of 2026-11-20).
//!
//! Both stories need the real operation frame: the campaign declares, opens its window, reads the
//! tank farm at the close through TacSim's probe and settles who won. TacSim has no campaign edge,
//! so they live here, where the composition root joins the two. This binary composes TacSim's own
//! test roster (`driving`'s rig, by path, as mush-server's `commerce_composition` takes
//! `vertex.rs`) and seats the campaign beside it as `mush_composition::server_modules_with` does:
//! [`mush_composition::campaign_operations::CampaignLink`] over the campaign module, with TacSim's
//! close probe and the composition's defender notice.
//!
//! - `empty_run`: nobody connected. The Assembly's votes fill the Concession Review, the arm
//!   declares, the house carries the Freeport arm's hauls through the picket, and the operation
//!   resolves into control and the import duty with no staff or player step.
//! - `newcomer`: two accounts. A hauler takes the arm's work beside the house's traffic, and a
//!   player who never joined arrives at Isidis after the operation and reads who won from the
//!   terms.
//!
//! One module: `cargo nextest run -p mush-composition --test op8_campaign -E 'test(/^empty_run::/)'`.

#[path = "../../tacsim/tests/support/field/driving_rig.rs"]
mod driving_rig;
#[path = "../../tacsim/tests/support/geology_keyed.rs"]
mod geology_keyed;
#[path = "../../tacsim/tests/support/mod.rs"]
mod support;

#[path = "op8_campaign/rig.rs"]
mod rig;

#[path = "op8_campaign/empty_run.rs"]
mod empty_run;
#[path = "op8_campaign/newcomer.rs"]
mod newcomer;

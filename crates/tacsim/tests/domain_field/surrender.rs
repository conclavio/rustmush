//! **Doc 42b cut SU.3 — the check and the yield**, over both grounds (`surrender_rig`): a raider
//! crew's morale breaks under a lost carrier and a holder, and the hull leaves the field as a
//! casualty; a crew at 1,000 holds; with no holder nobody yields; a restart redraws nothing.

use crate::grounds_rig::{AHUNA_FLANK, URVARA_REACH};
use crate::surrender_rig::{Case, surrender};

/// **The yield at 256**: the crew at 0 yields to the Red Hound's party and its hull stands as a
/// casualty; the crew at 1,000 fights on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn su3_a_stranded_crew_yields_to_a_holder() {
    surrender(&AHUNA_FLANK, Case::Yields).await;
}

/// **The yield at 512**.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn su3_a_stranded_crew_yields_to_a_holder_at_512() {
    surrender(&URVARA_REACH, Case::Yields).await;
}

/// **(s2) Morale holds at 256**: both crews at 1,000.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn su3_morale_holds_at_full_resolve() {
    surrender(&AHUNA_FLANK, Case::Holds).await;
}

/// **(s2) at 512**.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn su3_morale_holds_at_full_resolve_at_512() {
    surrender(&URVARA_REACH, Case::Holds).await;
}

/// **(s3) No holder, no yield at 256**: no carrier of the stranger's on the site.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn su3_no_holder_no_yield() {
    surrender(&AHUNA_FLANK, Case::NoHolder).await;
}

/// **(s3) at 512**.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn su3_no_holder_no_yield_at_512() {
    surrender(&URVARA_REACH, Case::NoHolder).await;
}

/// **(s5) A restart at 256**: the step and the yield are durable.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn su3_a_restart_redraws_nothing() {
    surrender(&AHUNA_FLANK, Case::Restart).await;
}

/// **(s5) at 512**.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn su3_a_restart_redraws_nothing_at_512() {
    surrender(&URVARA_REACH, Case::Restart).await;
}

/// **(s1) at 256** (doc 42b cut SU.4): the yielded hull is towed in, its crew captured, handed
/// over at Ceres with no fee, and out of play at release.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn su4_a_yielded_crew_is_taken_in_and_handed_over() {
    crate::surrender_rig::taken_in(&AHUNA_FLANK).await;
}

/// **(s1) at 512**.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn su4_a_yielded_crew_is_taken_in_and_handed_over_at_512() {
    crate::surrender_rig::taken_in(&URVARA_REACH).await;
}

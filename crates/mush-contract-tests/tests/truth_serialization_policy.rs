//! Compile-time tripwire for the private-truth half of the projection treaty.
//!
//! Durable internal records may retain authoritative subjects and targets, but
//! the resolved wrappers handed to providers must never become generic wire
//! values. Public projections are exercised separately by the K8.6 CI job.

use mush_core::{ResolvedActionInvocation, ResolvedAuthoritativeOffer};
use serde::{Serialize, de::DeserializeOwned};
use static_assertions::assert_not_impl_any;

assert_not_impl_any!(ResolvedActionInvocation: Serialize, DeserializeOwned);
assert_not_impl_any!(ResolvedAuthoritativeOffer: Serialize, DeserializeOwned);

#[test]
fn private_truth_wrappers_have_no_serde_contract() {
    // The assertions above are evaluated by the compiler. Keep one named test
    // so the dedicated CI job reports an explicit policy result.
    assert!(std::any::type_name::<ResolvedActionInvocation>().contains("ResolvedActionInvocation"));
    assert!(
        std::any::type_name::<ResolvedAuthoritativeOffer>().contains("ResolvedAuthoritativeOffer")
    );
}

//! Compile-time tripwire for the private-truth half of the projection treaty.
//!
//! Durable internal records may retain authoritative subjects and targets, but
//! the resolved wrappers handed to providers must never become generic wire
//! values. Public projections are exercised separately by the K8.6 CI job.

use mush_core::{GeologyMac, GeologySecret, ResolvedActionInvocation, ResolvedAuthoritativeOffer};
use serde::{Serialize, de::DeserializeOwned};
use static_assertions::assert_not_impl_any;
use tacsim::surface::geology::{FieldGround, PocketRow, ProvinceGround};
use tacsim::surface::ground::{SiteGeology, SiteGround};
use tacsim::surface::opening::OpeningRow;

assert_not_impl_any!(ResolvedActionInvocation: Serialize, DeserializeOwned);
assert_not_impl_any!(ResolvedAuthoritativeOffer: Serialize, DeserializeOwned);

// Doc 20 cut SM.1 (§3.2, Q-20-12, §16): the geology key and its keyed HMAC state never leave
// the process, and the ground's truth never serializes — readings are re-derived at render and
// the store never holds truth ("no surface that reveals truth").
assert_not_impl_any!(GeologySecret: Serialize, DeserializeOwned);
assert_not_impl_any!(GeologyMac: Serialize, DeserializeOwned);
assert_not_impl_any!(ProvinceGround: Serialize, DeserializeOwned);
assert_not_impl_any!(PocketRow: Serialize, DeserializeOwned);
assert_not_impl_any!(FieldGround: Serialize, DeserializeOwned);
assert_not_impl_any!(SiteGeology: Serialize, DeserializeOwned);
assert_not_impl_any!(SiteGround: Serialize, DeserializeOwned);
// Doc 20 cut SM.6 (§3.4; reviewer F6): a site's opening plan row carries the gross and the
// pockets' worked shares — derived truth, held server-side, never a wire value. (The pocket
// cursor, `tacsim.pockets`, is a durable family and must serialise; it is never projected.)
assert_not_impl_any!(OpeningRow: Serialize, DeserializeOwned);

#[test]
fn private_truth_wrappers_have_no_serde_contract() {
    // The assertions above are evaluated by the compiler. Keep one named test
    // so the dedicated CI job reports an explicit policy result.
    assert!(std::any::type_name::<ResolvedActionInvocation>().contains("ResolvedActionInvocation"));
    assert!(
        std::any::type_name::<ResolvedAuthoritativeOffer>().contains("ResolvedAuthoritativeOffer")
    );
    // Doc 20 cut SM.1: the geology secret, its keyed state and the five ground-truth types.
    for name in [
        std::any::type_name::<GeologySecret>(),
        std::any::type_name::<GeologyMac>(),
        std::any::type_name::<ProvinceGround>(),
        std::any::type_name::<PocketRow>(),
        std::any::type_name::<FieldGround>(),
        std::any::type_name::<SiteGeology>(),
        std::any::type_name::<SiteGround>(),
        // Doc 20 cut SM.6: the opening plan's row.
        std::any::type_name::<OpeningRow>(),
    ] {
        assert!(name.contains("Geology") || name.contains("Ground") || name.contains("Row"));
    }
}

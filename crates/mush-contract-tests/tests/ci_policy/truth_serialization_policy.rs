//! Compile-time tripwire for the private-truth half of the projection treaty.
//!
//! Durable internal records may retain authoritative subjects and targets, but
//! the resolved wrappers handed to providers must never become generic wire
//! values. Public projections are exercised separately by the K8.6 CI job.

use mush_core::{GeologyMac, GeologySecret, ResolvedActionInvocation, ResolvedAuthoritativeOffer};
use serde::{Serialize, de::DeserializeOwned};
use static_assertions::assert_not_impl_any;
use tacsim::port::claims::{ClaimPlan, PlanCore};
use tacsim::surface::geology::{FieldGround, PocketRow, ProvinceGround};
use tacsim::surface::ground::{SiteGeology, SiteGround};
use tacsim::surface::mine::MinePlan;
use tacsim::surface::opening::OpeningRow;
use tacsim::surface::plan::{PlanAct, PlanMove};

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
// Doc 20 cut SM.7: the mining plan (it holds the pocket's true class and tonnes). The run record
// (`tacsim.excursion`) and the start's fixed parameters carry the same figures and serialise — a
// durable family and server-only fixed parameters, never projected (the cursor's precedent).
assert_not_impl_any!(MinePlan: Serialize, DeserializeOwned);
// Doc 20 cut SM.10 (the brief's 3.6 item 17, reviewer F8): a claim plan names the block's opened
// deposits — a deposit's existence is a barren-ness bit (SM.6's D14) — so the plan and its compared
// core never serialise. The block record (`tacsim.claims`) names no deposit.
assert_not_impl_any!(ClaimPlan: Serialize, DeserializeOwned);
assert_not_impl_any!(PlanCore: Serialize, DeserializeOwned);
// Doc 20 cut SM.11 (the brief's 3.11, D23; reviewer F5): a fill plan's next move and its act hold a
// mining plan (the pocket's true class and tonnes), so neither serialises. The fill plan itself
// (`FillPlan`, on the rig's `tacsim.excursion` record) and the start's fixed parameters
// (`PlanFixed`, `FillFixed`) serialise — a durable family and server-only fixed parameters — and
// hold no truth: step words, addresses, a cursor, a tally of what landed, never a class or a reserve.
assert_not_impl_any!(PlanMove: Serialize, DeserializeOwned);
assert_not_impl_any!(PlanAct: Serialize, DeserializeOwned);
// Doc 20 cut SM.12b (the brief's step 45): the house yard's port-map row (`YardRow`), its standing
// licence pointer (`HouseLicence`) and the dispatch family's record (`DispatchLedger`) are durable
// records and serialise; none holds truth — hull and rig numbers, a thing id, an account id and its
// window, a count of loads and tonnes landed, never a pocket's class or a reserve. The dispatcher
// files through the player's compiler, whose plan and move types stay above. Nothing new is
// `!Serialize`.

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
        // Doc 20 cut SM.7: the mining plan (it holds the pocket's true class and tonnes).
        std::any::type_name::<MinePlan>(),
        // Doc 20 cut SM.10: the claim plan and its compared core.
        std::any::type_name::<ClaimPlan>(),
        std::any::type_name::<PlanCore>(),
        // Doc 20 cut SM.11: a fill plan's move and its act.
        std::any::type_name::<PlanMove>(),
        std::any::type_name::<PlanAct>(),
    ] {
        assert!(
            name.contains("Geology")
                || name.contains("Ground")
                || name.contains("Row")
                || name.contains("Plan") // "Plan": SM.7's MinePlan, SM.10's ClaimPlan and PlanCore, SM.11's PlanMove and PlanAct
        );
    }
}

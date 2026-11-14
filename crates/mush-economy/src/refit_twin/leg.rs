//! Which refit is paid for: the one thing the bay refit's fee leg and the core refit's paid
//! leg differ in.
//!
//! [`Leg`] is sealed. Its two implementations are [`Bay`] (`crate::bay_refit`) and [`Core`]
//! (`crate::core_refit`), each in its own family's `leg.rs`; the five owner files of this
//! module (`fold`, `install`, `money`, `plans`, `receipt`) are written once over it and resolved
//! at compile time. There is no `dyn`, no callback and no registration: the closed lists that
//! name the two legs (`EconomyRequest`, `EconomyOutput`, the engine's plan guard, the fold's two
//! fields, the boot's two flags) stay two hand-written entries each, and a leg's functions
//! below say which entry is its own.
//!
//! What stays two: each family's subject, policy, attributions, configuration and authority
//! checks (`subject.rs`, `consent.rs`, `authority.rs`, `bridge.rs`), and the two records whose
//! serialized field names differ (the correlation's `Proof`, `refit` against `replacement`, and
//! the terminal receipt's result, the same). They are this trait's associated types.

use super::{
    CurrentFacts,
    money::ClosedPlan,
    plans::{FundingIntent, Phase},
    receipt::{PaymentHistory, ReceiptAudit},
};
use crate::engine::ValidateEnv;
use crate::fold::FoldState;
use crate::module::EconomyStore;
use crate::plan::PlanResult;
use crate::{AccountId, Amount, EconomySnapshot, InstrumentId};
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId, World};
use mush_org::SystemPrincipalId;
use serde::{Serialize, de::DeserializeOwned};
use std::fmt::{Debug, Display};

/// A component refit in a facility bay (doc 31 cut UD.4b): the fee is the supplier's quote and
/// is paid to the bay's provider.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bay;

/// A certified core fitted into a hull (doc 17h): the fee is the recipe's, paid to the recipe's
/// site, and the incoming core comes from a completed core manufacture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Core;

// The seal: a public trait in a private module, implemented here for the two legs only.
// (Named apart from `renewal_twin`'s `sealed`, whose census counts that module's lines.)
mod members {
    pub trait Member {}
    impl Member for super::Bay {}
    impl Member for super::Core {}
}

/// One accepted order's money terms. Each family keeps its own (`BaySubject`,
/// `RefitSubject`); the owner reads what both have through this.
pub trait Subject: Clone + Debug + Eq + Serialize + DeserializeOwned {
    fn order(&self) -> u64;
    fn offer(&self) -> CommitToken;
    fn requester(&self) -> ObjId;
    fn payer(&self) -> AccountId;
    fn hull(&self) -> ObjId;
    fn accepted_at(&self) -> &ClockStamp;
    fn expires_at(&self) -> &ClockStamp;
}

/// The fee terms. Each family keeps its own (`BayPaymentPolicy`, `RefitPaymentPolicy`).
pub trait Policy: Clone + Debug + Eq + Serialize + DeserializeOwned {
    fn issuer(&self) -> &SystemPrincipalId;
    fn instrument(&self) -> &InstrumentId;
}

/// The supplier's acknowledged offer. Each family keeps its own.
pub trait Supplier: Clone + Debug + Eq + Serialize + DeserializeOwned {
    fn observed_utc(&self) -> i64;
    fn expires_utc(&self) -> i64;
}

/// **The paid leg of a refit.** Sealed: [`Bay`] and [`Core`] are the only two.
///
/// The constants are each family's identity strings, byte for byte as its files held them;
/// the functions are either a family's own records and checks, forwarded, or one of the
/// places the two families' owners differed in logic (the last group), each called where its
/// check was made.
///
/// It cannot be named outside this crate:
///
/// ```compile_fail
/// use mush_economy::refit_twin::Leg;
/// ```
pub trait Leg: members::Member + Copy + Debug + Default + Eq + Send + Sync + 'static {
    /// The kernel's three paid actions, by id.
    const ACCEPT_ACTION: &'static str;
    const INSTALL_ACTION: &'static str;
    const CANCEL_ACTION: &'static str;
    /// The idempotency and ruleset namespace of every plan this leg posts.
    const NAMESPACE: &'static str;
    /// The kind of the reservation hold.
    const HOLD_PURPOSE: &'static str;
    const MAX_ORDERS: usize;
    const HISTORY_BYTES: usize;
    /// The owner's one refusal sentence.
    const REFUSED: &'static str;
    /// The plan's description, and the memo of each of an Install's two lines.
    const PLAN_DESCRIPTION: &'static str;
    const LINE_MEMO: &'static str;
    /// The replayed histories' decoder: what it expects, and its three refusals.
    const HISTORIES_EXPECTING: &'static str;
    const HISTORY_LIMIT: &'static str;
    const HISTORY_DUPLICATE: &'static str;
    const HISTORY_KEY: &'static str;

    /// The family's error (`BayRefitError`, `RefitError`) and the four of its variants the
    /// owner answers.
    type Error: Copy + Debug + Display;
    const INVALID: Self::Error;
    const INTEGRITY: Self::Error;
    const INCOMPLETE: Self::Error;
    const CAPACITY: Self::Error;

    type Subject: Subject;
    type Policy: Policy;
    type Supplier: Supplier;
    /// The acknowledged ready order (`BayOrderAttribution`, `RefitOrderAttribution`).
    type Order: Clone + Debug + Eq;
    /// The composed leg (`BayRefitConfig`, `CoreRefitConfig`).
    type Config;
    /// The correlation a plan carries: the family's own record, `refit` or `replacement` in
    /// its Install phase.
    type Proof: Serialize + DeserializeOwned;
    /// The terminal receipt's result: the family's own record, the same difference.
    type Result: Clone + Debug + Eq + Serialize + DeserializeOwned;
    /// The Install and Cancel requests, each the family's own.
    type Installation;
    type Cancellation;

    // -- The family's store entries and its own code, forwarded.

    /// Which of the fold's two fields is this leg's.
    fn receipts(fold: &FoldState) -> &ReceiptAudit<Self>;
    fn receipts_mut(fold: &mut FoldState) -> &mut ReceiptAudit<Self>;
    /// The engine's entry for this leg's closed plan.
    fn validate(
        proof: &ClosedPlan<Self>,
        snapshot: &EconomySnapshot,
        env: &ValidateEnv<'_>,
    ) -> PlanResult;
    /// The family's authority checks (`authority.rs`).
    fn funding(
        config: &Self::Config,
        state: &EconomyStore,
        world: &World,
        subject: &Self::Subject,
        initial: bool,
    ) -> Result<(FundingIntent<Self>, CurrentFacts<Self>), Self::Error>;
    fn installation(
        config: &Self::Config,
        state: &EconomyStore,
        world: &World,
        intent: &FundingIntent<Self>,
        acceptance: CommitToken,
    ) -> Result<CurrentFacts<Self>, Self::Error>;
    fn policy(config: &Self::Config) -> &Self::Policy;
    fn cleanup_ready(config: &Self::Config, world: &World) -> Result<(), Self::Error>;
    fn matches_policy(subject: &Self::Subject, policy: &Self::Policy) -> Result<(), Self::Error>;
    fn supplier_validate(
        supplier: &Self::Supplier,
        policy: &Self::Policy,
        subject: &Self::Subject,
    ) -> Result<(), Self::Error>;
    fn action_ref(policy: &Self::Policy, action: &str) -> Result<DefinitionRef, Self::Error>;
    /// The family's bounded canonical encoding (`subject::bytes`).
    fn bytes<T: Serialize>(value: &T, max: usize) -> Result<Vec<u8>, Self::Error>;
    fn installation_acceptance(request: &Self::Installation) -> CommitToken;
    fn cancellation_acceptance(request: &Self::Cancellation) -> CommitToken;

    // -- The two records whose field names differ.

    fn seal(phase: Phase<Self>) -> Self::Proof;
    fn open(proof: Self::Proof) -> Phase<Self>;
    fn installed(part: DefinitionDigest) -> Self::Result;
    fn cancelled() -> Self::Result;
    /// What an Install fitted; `None` for a Cancel.
    fn result_part(result: &Self::Result) -> Option<DefinitionDigest>;
    /// What the acknowledged ready order will fit.
    fn part(order: &Self::Order) -> DefinitionDigest;

    // -- Where the two owners differed in logic.

    /// The fee: the bay's quote, or the core recipe's own fee.
    fn price(intent: &FundingIntent<Self>) -> Result<Amount, Self::Error>;
    /// Who is paid: the quoted bay's provider, or the recipe's site's.
    fn provider_account(intent: &FundingIntent<Self>) -> Result<AccountId, Self::Error>;
    /// A commit token the subject already spent besides its offer: the core refit's
    /// manufacture acceptance. The bay has none.
    fn commit_reused(subject: &Self::Subject, commit: CommitToken) -> bool;
    /// What no two retained histories may share: the bay's hull while its order is open, the
    /// core refit's incoming core once installed.
    fn exclusive(history: &PaymentHistory<Self>) -> Option<ObjId>;
    /// A fresh Accept conflicts with a retained history beyond the order, offer, hold and
    /// open-hull rules both legs have: the core refit's incoming core, unless that history
    /// was cancelled. The bay has none.
    fn conflicts(history: &PaymentHistory<Self>, intent: &FundingIntent<Self>) -> bool;
}

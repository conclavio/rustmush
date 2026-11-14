//! Who issues a renewal: the one thing the personal and the House renewal family differ in.
//!
//! [`Issuer`] is sealed. Its two implementations are [`Personal`]
//! (`crate::managed_renewal`) and [`House`] (`crate::managed_house_renewal`), each in its own
//! family's file; the owner functions in this module are written once over it and resolved at
//! compile time. There is no `dyn`, no callback and no registration: the closed lists that
//! name the two families (`EconomyRequest`, `EconomyOutput`, the plan guard, the store's and
//! the fold's fields, the managed-consent wire) stay two hand-written entries each, and an
//! issuer's functions below say which entry is its own.

use super::{
    RenewalMarkerV1, funding::ClosedPlan, history::OfferBook, receipt::RenewalReceiptAudit,
};
use crate::contract::managed::ManagedConsent;
use crate::engine::ValidateEnv;
use crate::fold::FoldState;
use crate::managed_entitlement::Recipient;
use crate::managed_paid_resale::PaidBirthRouteV1;
use crate::managed_purchase::PurchasePayout;
use crate::module::EconomyStore;
use crate::plan::PlanResult;
use crate::{AccountId, Amount, EconomySnapshot, Holder, InstrumentId};
use mush_core::participant::ParticipantError;
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, ObjId, World};
use mush_org::OrgScopeId;
use serde::{Serialize, de::DeserializeOwned};
use std::collections::BTreeSet;
use std::fmt::Debug;

/// A player renews their own mind licence: the seller is a character, the charter and any
/// royalty parent's charter are personal titles, and the remainder is the seller's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Personal;

/// A House licence is renewed: the charter is an organisation's title on one of its scopes
/// and the remainder is that organisation's treasury there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct House;

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::Personal {}
    impl Sealed for super::House {}

    /// What [`super::Issuer::bind`] asks of its caller. Nothing outside this crate can make
    /// one, so the one function of the trait that hands out the snapshot mutably cannot be
    /// called from another crate, though the trait is public (doc 46 CT.5).
    pub struct BindKey(());
    pub(crate) const fn bind_key() -> BindKey {
        BindKey(())
    }
}
pub use sealed::BindKey;
pub(crate) use sealed::bind_key;

/// The predecessor a renewal stands on. The two issuers' records differ by one field (the
/// personal basis carries the royalty chain's digest), so each family keeps its own
/// `RenewalBasisV1` and the owner functions read what both have through this.
pub trait Basis: Clone + Debug + Eq + Serialize + DeserializeOwned {
    fn validate(&self) -> Result<(), ParticipantError>;
    fn licence(&self) -> u64;
    fn predecessor_generation(&self) -> u64;
    fn predecessor_operation(&self) -> CommitToken;
    fn prior_quote(&self) -> CommitToken;
    /// The four fields the Roll's paired owner compares with its own binding (doc 46 CT.5).
    fn next_generation(&self) -> u64;
    fn prior_quote_digest(&self) -> DefinitionDigest;
    fn product_digest(&self) -> DefinitionDigest;
    fn template_digest(&self) -> DefinitionDigest;
    fn grant_economics_digest(&self) -> DefinitionDigest;
}

/// The disclosed terms of one renewal. A seller, a lineage and up to three royalty parents
/// for the personal issuer; an organisation and a scope for the House. Each family keeps its
/// own `RenewalOfferTermsV1`, with its own payout computation and its own check of the
/// issuer's current authority (`subject.rs`).
pub trait Terms: Clone + Debug + Eq + Serialize + DeserializeOwned {
    fn buyer(&self) -> Holder;
    fn charter(&self) -> ObjId;
    /// The charter's title revision the terms were offered at.
    fn title_revision(&self) -> u64;
    fn payer(&self) -> AccountId;
    fn instrument(&self) -> &InstrumentId;
    fn price(&self) -> Amount;
    fn remainder(&self) -> &Recipient;
    fn not_before(&self) -> &ClockStamp;
    fn expires_at(&self) -> &ClockStamp;
    /// Who is paid what when the offer is granted.
    fn payouts(&self) -> Result<Vec<PurchasePayout>, ParticipantError>;
    /// Every account a grant pays, the remainder's last.
    fn recipients(&self) -> Vec<&Recipient>;
    /// The issuer still holds what the terms disclose: a player's titles for the personal
    /// issuer, the organisation store and the scoped title for the House.
    fn check_current(&self, state: &EconomyStore, world: &World) -> Result<(), ParticipantError>;
}

/// One renewal offer: a basis and its terms. Each family keeps its own `RenewalOffer`.
pub trait Offer: Clone + Debug + Eq + Serialize + DeserializeOwned {
    type Basis: Basis;
    type Terms: Terms;
    fn basis(&self) -> &Self::Basis;
    fn terms(&self) -> &Self::Terms;
    fn validate(&self) -> Result<(), ParticipantError>;
    fn digest(&self) -> Result<DefinitionDigest, ParticipantError>;
}

/// The issuer of a renewal family.
///
/// Sealed: nothing outside this file can implement it, so the set is exactly [`Personal`] and
/// [`House`]. Since doc 46 CT.5 the module is public, so that the Roll's and the market's twins
/// can name the generic types at their own issuer; a third implementation is still refused,
/// since its supertrait cannot be named from another crate:
///
/// ```compile_fail
/// struct Guild;
/// impl mush_economy::renewal_twin::Issuer for Guild {}
/// ```
///
/// The constants are the family's identity strings. Each holds the value its family's file
/// held before the two were merged, and a persisted byte depends on every one of them.
pub trait Issuer: sealed::Sealed + Copy + Debug + Default + Eq + 'static {
    /// The five money and offer actions, and the service's unfunded close.
    const DRAFT_ACTION: &'static str;
    const CLOSE_ACTION: &'static str;
    const FUND_ACTION: &'static str;
    const GRANT_ACTION: &'static str;
    const REFUND_ACTION: &'static str;
    const SERVICE_CLOSE_ACTION: &'static str;
    /// The idempotency and cause namespace of the family's journal entries.
    const NAMESPACE: &'static str;
    /// The key prefixes of the offer record and of the service closure.
    const OFFER_PREFIX: &'static str;
    const SERVICE_PREFIX: &'static str;
    /// The `protocol` name in the payload of the five actions, and of the service's.
    const PROTOCOL: &'static str;
    const SERVICE_PROTOCOL: &'static str;
    /// The digest domain of an offer.
    const OFFER_DOMAIN: &'static str;
    /// The stored contract's name and the funding plan's description.
    const CONTRACT_NAME: &'static str;
    const FUNDING_DESCRIPTION: &'static str;
    /// The purchase family a renewed licence was born in, for a transferred holder's basis.
    const BIRTH_ROUTE: PaidBirthRouteV1;

    type Basis: Basis;
    type Terms: Terms;
    type Offer: Offer<Basis = Self::Basis, Terms = Self::Terms>;

    /// This issuer's offer book in the store.
    fn book(state: &EconomyStore) -> &OfferBook<Self>;
    /// The three things a published offer writes, borrowed apart for the consuming bind.
    /// Callable only inside this crate: the key cannot be made outside it.
    fn bind(
        state: &mut EconomyStore,
        key: BindKey,
    ) -> (&mut EconomySnapshot, &mut OfferBook<Self>, &mut u64);
    /// This issuer's receipt audit in the fold.
    fn receipts(fold: &FoldState) -> &RenewalReceiptAudit<Self>;
    fn receipts_mut(fold: &mut FoldState) -> &mut RenewalReceiptAudit<Self>;
    /// Whether a real boot checked this issuer's inventory.
    fn inventory_checked(state: &EconomyStore) -> bool;
    /// This issuer's variant of the managed consent, read and built.
    fn marker(consent: &ManagedConsent) -> Option<&RenewalMarkerV1<Self>>;
    fn consent(marker: RenewalMarkerV1<Self>) -> Result<ManagedConsent, ParticipantError>;
    /// The engine's validation under this issuer's plan guard.
    fn validate(
        proof: &ClosedPlan<Self>,
        snapshot: &EconomySnapshot,
        env: &ValidateEnv<'_>,
    ) -> PlanResult;

    /// The scope the contract is filed under: none for a personal renewal, the issuing
    /// organisation's scope for a House one.
    fn contract_scope(terms: &Self::Terms) -> Option<OrgScopeId>;
    /// The titles an offer and a sale stand on: the charter, and for the personal issuer the
    /// charter of every royalty parent that is paid.
    fn titles(terms: &Self::Terms) -> BTreeSet<ObjId>;
    /// Whether a retry is of the same issuer's side as the offer it follows: the same
    /// lineage, or the same organisation and scope.
    fn same_line(row: &Self::Terms, before: &Self::Terms) -> bool;
}

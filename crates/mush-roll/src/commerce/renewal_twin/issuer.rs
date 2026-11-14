//! Who issues a renewal, as the Roll sees it.
//!
//! [`Issuer`] is sealed and has exactly two implementations, Economy's `Personal` and
//! `House`, each in its family's `mod.rs` (`commerce::renewal`, `commerce::house_renewal`).
//! It carries the Roll's identity strings and the family's own record types. [`Rules`] is the
//! rest of what an issuer decides: which entry of a closed list is its own (its two state
//! families, its economics family, its root anchor and request, its Economy audit) and the
//! checks only one family has. It is private to the crate, because what it names is.
//!
//! There is no `dyn`, no callback and no registration: the generic owner is resolved at
//! compile time, and the closed lists (`CommerceFamilies`, `RootRequest`, `RootAnchor`) stay
//! two hand-written entries each.

use super::super::{
    CharterId, Holder, LicenceId, LineId, RevisionIdentity, TermGeneration,
    entitlement::history::Birth,
    families::CommerceFamilies,
    paid::{PaidTermRecord, PaidTerms, PersonalPaidLicence},
    participant::RootRequest,
    records::{PublicationError, Record},
    roots::RootAnchor,
};
use super::records::RenewalHeadV1;
use super::roots::RenewalRoots;
use mush_core::participant::OwnerPrepareContext;
use mush_core::read_service::{OwnerReadError, OwnerReadService};
use mush_core::{
    ActionTxn, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ModError, ModuleCtx,
    PrepareCtx, Registrar, StateFamilyHandle,
};
use mush_economy::managed_paid_resale::RenewalHolderBasisV1;
use mush_economy::renewal_twin::{RenewalAudit, RenewalAuditError};
use mush_economy::{Amount, ContractId, EconomyStore, InstrumentId};
use serde::{Serialize, de::DeserializeOwned};
use std::fmt::Debug;

mod sealed {
    pub trait Sealed {}
    impl Sealed for mush_economy::renewal_twin::Personal {}
    impl Sealed for mush_economy::renewal_twin::House {}
}

/// The record bounds every renewal record of an issuer has.
pub trait Row: Clone + Debug + PartialEq + Eq + Serialize + DeserializeOwned + 'static {}
impl<T: Clone + Debug + PartialEq + Eq + Serialize + DeserializeOwned + 'static> Row for T {}

/// The issuer of a Roll renewal family.
///
/// Sealed: nothing outside this file can implement it, so the set is exactly Economy's
/// `Personal` and `House`. From another crate the trait cannot be implemented, as its
/// supertrait cannot be named:
///
/// ```compile_fail
/// struct Guild;
/// impl mush_roll::commerce::renewal_twin::Issuer for Guild {}
/// ```
///
/// The constants are the Roll's own identity strings. Each holds the value its family's files
/// held before the two were merged, and a persisted byte depends on every one of them. The
/// action ids, the protocol names, the offer's digest domain and the receipt bound are
/// Economy's constants of the same issuer, which held the same values.
pub trait Issuer: sealed::Sealed + mush_economy::renewal_twin::Issuer + Send + Sync {
    /// The state family of the renewal operation and of the renewal head.
    const OPERATION_FAMILY: &'static str;
    const HEAD_FAMILY: &'static str;
    /// The digest domains of a predecessor term and of a renewal template in a binding.
    const PREDECESSOR_DOMAIN: &'static str;
    const TEMPLATE_DOMAIN: &'static str;

    /// The family's own binding, operation and unfunded closure, each its own `checked!`
    /// record.
    type Binding: Binding<Self>;
    type Operation: Operation<Self>;
    type Closure: Closure;
    /// The term economics a renewed generation is captured in, and its grant.
    type Economics: TermEconomics<Grant = Self::Grant>;
    type Grant: Row;
    /// The family's reader, and the fresh facts it observes.
    type Reader: OwnerReadService;
    type Facts: Facts<Self>;
    /// What a fresh renewal's admission carries: nothing for the personal issuer, the House
    /// purchase composition's buyer admission for the House.
    type Admission: Clone + Default + Send + Sync + 'static;
    /// What a renewal chain is checked against beside the licence: nothing for the personal
    /// issuer, the House that issued the licence for the House.
    type Line: Copy + Debug + Eq;
}

/// A renewal's binding: the predecessor and the template it renews.
pub trait Binding<I: Issuer>: Row {
    fn licence(&self) -> LicenceId;
    fn holder(&self) -> Holder;
    fn charter(&self) -> CharterId;
    fn line(&self) -> LineId;
    fn birth_operation(&self) -> CommitToken;
    fn predecessor(&self) -> TermGeneration;
    fn next(&self) -> TermGeneration;
    fn predecessor_operation(&self) -> CommitToken;
    fn revision(&self) -> RevisionIdentity;
    fn terms(&self) -> PaidTerms;
    fn term_seconds(&self) -> i64;
    fn grace_seconds(&self) -> i64;
    fn product_digest(&self) -> DefinitionDigest;
    fn holder_basis(&self) -> Option<RenewalHolderBasisV1>;
    fn grant(&self) -> &I::Grant;
    fn price_minor(&self) -> i64;
    fn validate(&self) -> Result<(), PublicationError>;
    fn template_digest(&self) -> Result<DefinitionDigest, PublicationError>;
    /// The grant's digest, the renewal template's instrument and price.
    fn grant_digest(&self) -> Result<DefinitionDigest, PublicationError>;
    fn template_instrument(&self) -> &InstrumentId;
    fn template_price(&self) -> Amount;
    fn matches_birth(&self, licence: &PersonalPaidLicence) -> bool;
    fn accepted_pointer(
        &self,
        generation: TermGeneration,
    ) -> Result<PersonalPaidLicence, PublicationError>;
    fn check_predecessor(
        &self,
        licence: &PersonalPaidLicence,
        term: &PaidTermRecord,
        economics: &I::Economics,
    ) -> Result<(), PublicationError>;
    /// The binding's economics captured at `generation` by `operation`.
    fn economics_at(
        &self,
        generation: TermGeneration,
        operation: CommitToken,
    ) -> Result<I::Economics, PublicationError>;
}

/// A renewal operation, as the generic owner reads and advances it.
pub trait Operation<I: Issuer>: Row {
    fn operation(&self) -> CommitToken;
    fn action(&self) -> &DefinitionRef;
    fn attempt(&self) -> super::records::RenewalAttemptV1;
    fn binding(&self) -> &I::Binding;
    fn offer_digest(&self) -> DefinitionDigest;
    fn contract(&self) -> ContractId;
    fn consent_hash(&self) -> &[u8; 32];
    fn created_at_ic(&self) -> &ClockStamp;
    fn created_at_utc(&self) -> i64;
    fn state(&self) -> &super::records::RenewalStateV1<I>;
    fn follows(&self, old: &Self) -> Result<(), PublicationError>;
    fn granted_rows(
        &self,
    ) -> Result<(PersonalPaidLicence, PaidTermRecord, I::Economics), PublicationError>;
    /// The same operation in another state, every other field as it is.
    fn with_state(&self, state: super::records::RenewalStateV1<I>) -> Self;
    /// A drafted operation of format 1.
    #[allow(clippy::too_many_arguments)]
    fn drafted(
        operation: CommitToken,
        action: DefinitionRef,
        attempt: super::records::RenewalAttemptV1,
        binding: I::Binding,
        offer_digest: DefinitionDigest,
        contract: ContractId,
        consent_hash: [u8; 32],
        created_at_ic: ClockStamp,
        created_at_utc: i64,
    ) -> Self;
}

/// An unfunded close, as the generic owner reads and writes it.
pub trait Closure: Row {
    fn commit(&self) -> CommitToken;
    fn action(&self) -> &DefinitionRef;
    fn at_ic(&self) -> &ClockStamp;
    fn posted_at_utc(&self) -> i64;
    fn reason(&self) -> mush_economy::renewal_twin::UnfundedCloseReason;
    fn new(
        commit: CommitToken,
        action: DefinitionRef,
        at_ic: ClockStamp,
        posted_at_utc: i64,
        reason: mush_economy::renewal_twin::UnfundedCloseReason,
    ) -> Self;
}

/// A generation's term economics: the entitlement purchase's for the personal issuer, the
/// House purchase's for the House.
pub trait TermEconomics: Row {
    type Grant;
    fn licence(&self) -> LicenceId;
    fn generation(&self) -> TermGeneration;
    fn operation(&self) -> CommitToken;
}

/// The fresh facts the family's reader observes for a draft, a fund or a grant.
pub trait Facts<I: Issuer>: Clone + Send + Sync + 'static {
    fn binding(&self) -> &I::Binding;
    fn prior_quote(&self) -> CommitToken;
    fn prior_quote_digest(&self) -> DefinitionDigest;
    fn check_fresh(
        &self,
        ctx: &OwnerPrepareContext<'_>,
        amount: Option<i64>,
    ) -> Result<(), PublicationError>;
}

/// What an issuer decides that the generic owner asks it, each at the place its check was.
/// Private to the crate: the closed lists it names are.
pub(crate) trait Rules:
    Issuer<Operation = <Self as Rules>::StoredOperation, Economics = <Self as Rules>::StoredEconomics>
{
    /// The issuer's operation and economics rows, as the crate's stores hold them.
    type StoredOperation: Operation<Self> + Record;
    type StoredEconomics: TermEconomics<Grant = Self::Grant> + Record;
    /// The issuer's two state families and its economics family.
    fn operations(f: &CommerceFamilies) -> &StateFamilyHandle<String, Self::Operation>;
    fn heads(f: &CommerceFamilies) -> &StateFamilyHandle<String, RenewalHeadV1<Self>>;
    fn economics(f: &CommerceFamilies) -> &StateFamilyHandle<String, Self::Economics>;
    /// The admission headroom of a next generation's economics row.
    fn admission_capacity(
        family: &StateFamilyHandle<String, Self::Economics>,
        row: &Self::Economics,
    ) -> Result<(), PublicationError>;
    /// Economy's audit of this issuer's offers, and its proof that none are retained.
    fn audit(economy: &EconomyStore) -> Result<RenewalAudit<'_, Self>, RenewalAuditError>;
    fn require_no_history(economy: &EconomyStore) -> Result<(), RenewalAuditError>;
    /// The root anchor of an operation, and the participant request of its roots.
    fn anchor(operation: CommitToken) -> RootAnchor;
    fn root_request(roots: RenewalRoots<Self>) -> RootRequest;

    /// The reader's request for a licence's fresh facts, and the facts out of its answer.
    fn own(licence: LicenceId) -> <Self::Reader as OwnerReadService>::Request;
    fn into_facts(
        read: <Self::Reader as OwnerReadService>::Output,
    ) -> Result<Self::Facts, OwnerReadError>;

    /// The writer's grants beyond the five families it delegates: the House's read of the
    /// entitlement economics its `absent_next` asks.
    fn install_extra(reg: &mut Registrar<'_>, f: &CommerceFamilies) -> Result<(), ModError>;
    /// The accounts an observed renewal pairs (doc 46 §4.3 7): a distinct requester and
    /// seller pair for the personal issuer, the requester's own account and the purchase
    /// composition's buyer admission for the House (§4.3 5).
    fn observe_accounts(
        ctx: &mut PrepareCtx<'_>,
        facts: &Self::Facts,
        admission: &Self::Admission,
    ) -> Result<(), PublicationError>;
    /// Whether a next generation already has the House's own economics row (the House) or
    /// nothing more (the personal issuer), after the paid term and the entitlement economics.
    fn next_present(
        txn: &mut ActionTxn<'_>,
        f: &CommerceFamilies,
        key: &String,
    ) -> Result<bool, PublicationError>;
    /// The issuer's side of an offer against the sale's facts (§4.3 1): its lineage, seller
    /// and royalty parents, or its organisation and scope.
    fn offer_differs_from_sale(
        offer: &<Self as mush_economy::renewal_twin::Issuer>::Offer,
        facts: &Self::Facts,
    ) -> bool;
    /// The issuer's side of an offer against the binding: the royalty chain's digest, or the
    /// organisation and scope of the House that issued it.
    fn offer_differs_from_binding(
        binding: &Self::Binding,
        offer: &<Self as mush_economy::renewal_twin::Issuer>::Offer,
    ) -> bool;
    /// At every fresh step's fence, after the facts' own check: the House's buyer admission.
    fn admit_fresh(
        admission: &Self::Admission,
        ctx: &OwnerPrepareContext<'_>,
        facts: &Self::Facts,
    ) -> Result<(), PublicationError>;

    /// The chain's line for a licence (nothing, or the House its birth names), and whether a
    /// row is off it.
    fn line(
        ctx: &ModuleCtx<'_>,
        f: &CommerceFamilies,
        licence: &PersonalPaidLicence,
    ) -> Result<Self::Line, PublicationError>;
    fn off_line(binding: &Self::Binding, line: Self::Line) -> bool;
    /// The audit's join of an operation to the birth it renews (§4.3 4: the birth variant):
    /// `Mismatch` when the birth is the other issuer's or differs from the binding.
    fn check_birth(
        birth: &Birth<'_>,
        binding: &Self::Binding,
        offer: &<Self as mush_economy::renewal_twin::Issuer>::Offer,
    ) -> Result<(), PublicationError>;
    /// The audit's inverted check (§4.3 4): a later generation of a licence this issuer has
    /// no renewal of must be the other issuer's.
    fn other_issuers_birth(birth: &Birth<'_>) -> bool;
    /// The issuer's seat count against its template (the personal licence has one seat).
    fn seats_differ(terms: PaidTerms, grant: &Self::Grant) -> Result<bool, PublicationError>;
}

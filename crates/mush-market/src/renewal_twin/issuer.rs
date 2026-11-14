//! Who issues a renewal, as the market sees it.
//!
//! [`Issuer`] has exactly two implementations, Economy's `Personal` and `House`, each in its
//! family's `mod.rs` (`mind_renewal`, `mind_house_renewal`). It is a subtrait of the Roll's
//! issuer and of the Roll writer's public face, so one issuer type names a renewal family
//! across the three crates. It carries the market's identity strings, the family's two
//! records, and the closed lists only the family can name: its Economy requests and outputs,
//! its completion and capacity calls, its reader, and the rules only it has (the payee and
//! the parent royalties of its offer, the House purchase template it admits).
//!
//! There is no `dyn`, no callback and no registration: the generic owner is resolved at
//! compile time.

use super::config::RenewalConfig;
use super::records::{OperationRow, ReceiptRow};
use mush_core::participant::ParticipantError;
use mush_core::read_service::{OwnerReadError, OwnerReadHandle, OwnerReadService};
use mush_core::{ActionTxn, CommitToken, DefinitionRef, ModError, PrepareCtx, Registrar, World};
use mush_economy::managed_entitlement::{GrossRecipientCut, Recipient};
use mush_economy::managed_paid_resale::RenewalHolderBasisV1;
use mush_economy::registry_filing::{EconomyOutput, EconomyRequest};
use mush_economy::renewal_twin::{
    RenewalAudit, RenewalAuditError, RenewalFundingOutput, RenewalMoneyRequest, RenewalOfferOutput,
    RenewalServiceCloseReason, RenewalTerminalOutput, UnfundedCloseReason,
};
use mush_economy::{AccountId, EconomyStore, Holder, InstrumentId};
use mush_roll::commerce::renewal::reader::RenewalStop;
use mush_roll::commerce::{LicenceId, TermGeneration, WorldIcSeconds};
use std::fmt::Debug;

mod sealed {
    pub trait Sealed {}
    impl Sealed for mush_economy::renewal_twin::Personal {}
    impl Sealed for mush_economy::renewal_twin::House {}
}

/// Economy's offer, its terms, and the Roll's operation, at an issuer.
pub(crate) type Offer<I> = <I as mush_economy::renewal_twin::Issuer>::Offer;
pub(crate) type Terms<I> = <I as mush_economy::renewal_twin::Issuer>::Terms;
pub(crate) type RollOperation<I> = <I as mush_roll::commerce::renewal_twin::Issuer>::Operation;
/// The reader's request and answer, at an issuer.
pub(crate) type ReadRequest<I> =
    <<I as mush_roll::commerce::renewal_twin::Issuer>::Reader as OwnerReadService>::Request;
pub(crate) type Read<I> =
    <<I as mush_roll::commerce::renewal_twin::Issuer>::Reader as OwnerReadService>::Output;

/// What an offer of either issuer has besides its payee, its parents and its terms' own
/// fields, in the order the offer is built.
pub(crate) struct OfferParts {
    pub buyer: Holder,
    pub payer: AccountId,
    pub instrument: InstrumentId,
    pub remainder: Recipient,
    pub now: mush_core::ClockStamp,
    pub expires_at: mush_core::ClockStamp,
}

/// The issuer of a market renewal family.
///
/// Sealed: the set is exactly Economy's `Personal` and `House`. The constants are the
/// market's own identity strings; each holds the value its family's files held before the
/// two were merged. The binding's protocol string and the action ids are Economy's constants
/// of the same issuer, which held the same values.
pub(crate) trait Issuer:
    sealed::Sealed
    + mush_roll::commerce::renewal_twin::Issuer
    + mush_roll::commerce::renewal_twin::Writes
    + Debug
{
    /// The state families of the operation and of the receipt.
    const OPERATION_FAMILY: &'static str;
    const RECEIPT_FAMILY: &'static str;
    /// The policy's definition kind, the outcome's id and the binding's kind.
    const POLICY_KIND: &'static str;
    const OUTCOME_ID: &'static str;
    const BINDING_KIND: &'static str;
    /// The offer's line selectors, a transition each in `Step::ALL`'s order, and the
    /// service's.
    const VERBS: [&'static str; 5];
    const SERVICE_VERB: &'static str;
    /// The service's command, its help, its tick, and the digest domains of its close intent
    /// and of a submission.
    const COMMAND: &'static str;
    const HELP: &'static str;
    const SERVICE_TICK: &'static str;
    const INTENT_DOMAIN: &'static str;
    const SUBMISSION_DOMAIN: &'static str;

    /// The family's operation and receipt, each its own `checked!` record.
    type Renewal: OperationRow<Self>;
    type Receipt: ReceiptRow<Self>;
    /// What the configuration carries beside the policy: nothing for the personal issuer,
    /// the House purchase policies a renewal template must name for the House.
    type Purchase: Clone + Debug + Send + Sync + 'static;
    /// The reader's inventory and its entry.
    type Inventory: Inventory<Entry = Self::Entry>;
    type Entry: Entry;

    /// The definitions the policy references after the manifest.
    fn purchase_references(purchase: &Self::Purchase) -> Vec<DefinitionRef>;
    /// Whether an observed renewal's template is one this configuration admits (§4.3 5):
    /// always for the personal issuer, the configured House purchase policy for the House.
    fn admits(config: &RenewalConfig<Self>, facts: &Self::Facts) -> bool;

    /// The offer's remainder recipient (§4.3 3): the seller's personal account for the
    /// personal issuer, the issuing House's treasury for the House.
    fn remainder(
        config: &RenewalConfig<Self>,
        e: &EconomyStore,
        world: &World,
        facts: &Self::Facts,
        instrument: &InstrumentId,
    ) -> Result<Recipient, String>;
    /// The offer itself: the personal issuer's parent royalties and lineage, the House's
    /// organisation and scope, each in its own terms.
    fn offer(
        config: &RenewalConfig<Self>,
        e: &EconomyStore,
        world: &World,
        facts: &Self::Facts,
        parts: OfferParts,
    ) -> Result<Offer<Self>, String>;
    /// Economy's bounded decode of the offer.
    fn decode_offer(bytes: &[u8]) -> Result<Offer<Self>, ParticipantError>;
    /// The terms' registry and venue cuts, and the sum of its parent royalties.
    fn registry(terms: &Terms<Self>) -> &GrossRecipientCut;
    fn venue(terms: &Terms<Self>) -> Option<&GrossRecipientCut>;
    fn parent_permille(terms: &Terms<Self>) -> u32;
    /// The service's status line.
    fn status_line(view: &crate::mind_renewal::player::RenewalStatusView) -> String;

    /// The issuer's Economy requests, one a variant of the closed request list.
    fn draft_request(offer: Offer<Self>) -> EconomyRequest;
    fn retry_request(offer: Offer<Self>, previous_operation: CommitToken) -> EconomyRequest;
    fn holder_draft_request(offer: Offer<Self>, basis: RenewalHolderBasisV1) -> EconomyRequest;
    fn holder_retry_request(
        offer: Offer<Self>,
        basis: RenewalHolderBasisV1,
        previous_operation: CommitToken,
    ) -> EconomyRequest;
    fn funding_request(money: RenewalMoneyRequest) -> EconomyRequest;
    fn grant_request(money: RenewalMoneyRequest) -> EconomyRequest;
    fn refund_request(money: RenewalMoneyRequest) -> EconomyRequest;
    fn close_request(operation: CommitToken, reason: UnfundedCloseReason) -> EconomyRequest;
    fn service_close_request(
        operation: CommitToken,
        reason: RenewalServiceCloseReason,
    ) -> EconomyRequest;
    /// The issuer's outputs out of Economy's answer.
    fn offer_output(output: EconomyOutput) -> Result<RenewalOfferOutput<Self>, ParticipantError>;
    fn funding_output(
        output: EconomyOutput,
    ) -> Result<RenewalFundingOutput<Self>, ParticipantError>;
    fn terminal_output(
        output: EconomyOutput,
    ) -> Result<RenewalTerminalOutput<Self>, ParticipantError>;
    /// Economy's audit of the issuer's offers, and its proof that none are retained.
    fn economy_audit(e: &EconomyStore) -> Result<RenewalAudit<'_, Self>, RenewalAuditError>;
    fn require_no_economy_history(e: &EconomyStore) -> Result<(), RenewalAuditError>;

    /// The kernel's completion reservation at a draft, and its advance after.
    fn reserve_completion(
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
    ) -> Result<(), mush_core::action_state::ActionStateError>;
    fn advance_completion(
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        operation: CommitToken,
    ) -> Result<(), mush_core::action_state::ActionStateError>;
    /// The kernel's capacity declaration, without and with the service's close.
    fn capacity(
        reg: &mut Registrar<'_>,
        manifest: DefinitionRef,
        participants: [DefinitionRef; 2],
        actions: [DefinitionRef; 5],
    ) -> Result<(), ModError>;
    fn capacity_with_service(
        reg: &mut Registrar<'_>,
        manifest: DefinitionRef,
        participants: [DefinitionRef; 2],
        actions: [DefinitionRef; 5],
        service_close: DefinitionRef,
    ) -> Result<(), ModError>;

    /// The reader's registration, its four requests, and the three answers out of a read.
    fn request_reader(
        reg: &mut Registrar<'_>,
        manifest: &DefinitionRef,
        service: bool,
    ) -> Result<OwnerReadHandle<Self::Reader>, ModError>;
    fn read_own(licence: LicenceId) -> ReadRequest<Self>;
    fn read_status(licence: LicenceId) -> ReadRequest<Self>;
    fn read_inventory(after: Option<String>) -> ReadRequest<Self>;
    fn read_operation(operation: CommitToken) -> ReadRequest<Self>;
    fn facts_of(read: Read<Self>) -> Result<Self::Facts, OwnerReadError>;
    fn inventory_of(read: Read<Self>) -> Result<Self::Inventory, OwnerReadError>;
    fn operation_of(read: Read<Self>) -> Result<RollOperation<Self>, OwnerReadError>;
    /// The earliest safe replacement time the facts captured.
    fn term_end(facts: &Self::Facts) -> WorldIcSeconds;
}

/// A page of the reader's inventory.
pub(crate) trait Inventory {
    type Entry;
    fn entries(&self) -> &[Self::Entry];
    fn into_single(self) -> Result<Self::Entry, OwnerReadError>;
    fn next_after(&self) -> Option<&str>;
}

/// One licence of the reader's inventory.
pub(crate) trait Entry {
    fn stop(&self) -> Option<RenewalStop>;
    fn licence(&self) -> LicenceId;
    fn holder(&self) -> Holder;
    fn generation(&self) -> TermGeneration;
    fn term_end(&self) -> WorldIcSeconds;
    fn last_operation(&self) -> Option<CommitToken>;
    fn holder_generation(&self) -> u64;
    fn attempted(&self) -> bool;
}

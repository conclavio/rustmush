//! Explicit House-issued personal Pin renewal composition. Decoded data conveys no owner
//! authority. Providers and paired boot exist only under the optional config;
//! the separately enabled service installs bounded continuations and controls,
//! never automatic acceptance.
//!
//! Doc 46 CT.6: the House instantiation. The market's renewal composition is written once for
//! both issuers in `renewal_twin`; this module is the House issuer's implementation of the
//! twin's trait (its identity strings, its records, its Economy and kernel entries, its
//! reader, and the rules only it has: the House treasury as payee, the House purchase policies
//! a renewal template must name, an offer without parent royalties), and every path its
//! callers name, now the twin's types and functions at the House issuer.

mod records {
    crate::renewal_twin::records::records!(
        mush_economy::renewal_twin::House,
        mush_roll::commerce::house_renewal::protocol
    );
}
mod reader_face {
    crate::renewal_twin::reader::reader_face!(mush_roll::commerce::house_renewal::reader);
}

use crate::mind_renewal::player::{
    self, RenewalPlayerOutcome, RenewalPlayerStep, RenewalStatusView,
};
use crate::renewal_twin::{self as twin, Issuer, OfferParts};
use mush_core::action::{AuthorityGuard, AuthorityGuardContext};
use mush_core::participant::ParticipantError;
use mush_core::read_service::{OwnerReadError, OwnerReadHandle};
use mush_core::{
    ActionTxn, CmdError, CommitToken, ContinuationHandle, Ctx, DefinitionRef, ModError, ModuleCtx,
    ObjId, PrepareCtx, Registrar, World,
};
use mush_economy::managed_entitlement::{GrossRecipientCut, Recipient};
use mush_economy::managed_house_renewal::{RenewalBasisV1, RenewalOffer, RenewalOfferTermsV1};
use mush_economy::managed_paid_resale::RenewalHolderBasisV1;
use mush_economy::registry_filing::{EconomyOutput, EconomyRequest};
use mush_economy::renewal_twin::{
    House, RenewalAudit, RenewalAuditError, RenewalFundingOutput, RenewalMoneyRequest,
    RenewalOfferOutput, RenewalServiceCloseReason, RenewalTerminalOutput, UnfundedCloseReason,
};
use mush_economy::{AccountStatus, EconomyStore, InstrumentId};
use mush_roll::commerce::house_purchase::{HouseProtocol, HouseRenewalTemplateV1};
use mush_roll::commerce::house_renewal::reader::{
    RenewalFacts, RenewalInventory, RenewalInventoryEntry, RenewalReader, RenewalRequest,
};
use mush_roll::commerce::house_renewal::{RenewalOperationV1, writer::RenewalWriter};
use mush_roll::commerce::{LicenceId, WorldIcSeconds};

pub use crate::renewal_twin::config::RenewalPolicy;
pub use records::{MindRenewalOperationV1, MindRenewalReceiptV1};

/// The House issuer's renewal composition (`renewal_twin::config::RenewalConfig`).
pub type RenewalConfig = twin::config::RenewalConfig<House>;
pub(crate) type Families = twin::families::Families<House>;

/// The House purchase policies a renewal template must name: the House's one-seat policy,
/// and its two-seat policy when the House sells two-seat Pins.
#[derive(Clone, Debug)]
pub(crate) struct HousePurchase {
    purchase_policy: DefinitionRef,
    two_seat_purchase_policy: Option<DefinitionRef>,
}

impl RenewalConfig {
    pub fn new(
        house: &crate::mind_house::HouseConfig,
        writer: RenewalWriter,
        policy: RenewalPolicy,
    ) -> Result<Self, ModError> {
        policy.validate()?;
        let manifest = house.manifest().clone();
        let participants = house.participants().clone();
        let purchase = HousePurchase {
            purchase_policy: house.policy_ref()?,
            two_seat_purchase_policy: house
                .has_two_seat_pins()
                .then(|| house.policy_ref_for_protocol(HouseProtocol::HouseV2))
                .transpose()?,
        };
        let writer = writer.with_purchase_admission(&house.writer())?;
        Self::of(manifest, participants, writer, policy, purchase)
    }
    pub(crate) fn purchase_policy(&self) -> &DefinitionRef {
        &self.purchase.purchase_policy
    }
    pub(crate) fn two_seat_purchase_policy(&self) -> Option<&DefinitionRef> {
        self.purchase.two_seat_purchase_policy.as_ref()
    }
    pub(crate) fn accepts_purchase_template(&self, template: &HouseRenewalTemplateV1) -> bool {
        match template.format() {
            1 => template.policy() == &self.purchase.purchase_policy,
            2 => self.purchase.two_seat_purchase_policy.as_ref() == Some(template.policy()),
            _ => false,
        }
    }
}

pub(crate) fn require_disabled(ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
    twin::audit::require_disabled::<House>(ctx)
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    config: &RenewalConfig,
) -> Result<Families, ModError> {
    twin::registration::register(reg, config)
}
pub(crate) fn enable_service(ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
    twin::service::enable::<House>(ctx)
}
pub(crate) fn service_guard_current(
    ctx: &AuthorityGuardContext<'_>,
    guard: &AuthorityGuard,
) -> bool {
    twin::service::guard_current::<House>(ctx, guard)
}

/// 17g C2d (C2 plan §2.1's renewal exception): one player step on the
/// actor's own House Pin renewal (C2d's front for `+minds/renew`). It is the
/// `@housemindrenewal` command's own player branch (`step`): the renewal reader's
/// holder check, the service's admission, retained entry, idempotency key and
/// submission, unchanged; a funded offer is still granted or refunded by the
/// service's continuation. Every refusal is the service's one uniform line,
/// `Renewal is unavailable or has changed.`, for a foreign licence and a
/// missing one alike.
///
/// 17g C2h: it reads through the market's own renewal read and families, so
/// it answers only in the market's own context; `+minds/renew` reaches the
/// same `step` through the service's delegated `front` continuation
/// (`player_front`), which runs under the market's identity.
pub fn player_step(
    ctx: &mut Ctx<'_>,
    licence: LicenceId,
    step_kind: RenewalPlayerStep,
) -> Result<RenewalPlayerOutcome, CmdError> {
    twin::service::player_step::<House>(ctx, licence, step_kind)
}

/// 17g C2h (the lead's ruling R3, the C2e plan §5.1): the player front's
/// handle, when the service is composed and enabled. TacSim's `+minds/renew`
/// schedules it for the actor whose typed line it is dispatching (the
/// delegation this service declared at composition admits nothing else).
#[must_use]
pub fn player_front(ctx: &Ctx<'_>) -> Option<ContinuationHandle> {
    twin::service::player_front::<House>(ctx)
}

/// `player_front`, read from the world itself (the service's state slot and
/// its runtime), so a composition test can prove the handle TacSim would be
/// handed after activation without a dispatched context.
#[must_use]
pub fn player_front_in(world: &World) -> Option<ContinuationHandle> {
    twin::service::player_front_in::<House>(world)
}

// 17g C2d: crate-visible for `mind_renewal`'s player-step test, which
// checks the House service's sentences against these fixtures.
#[cfg(test)]
pub(crate) fn command_line(outcome: RenewalPlayerOutcome) -> String {
    twin::service::command_line::<House>(outcome)
}
#[cfg(test)]
pub(crate) fn status_view(
    term_end_ic: i64,
    row: Option<&MindRenewalOperationV1>,
) -> RenewalStatusView {
    twin::service::status_view::<House>(term_end_ic, row)
}

impl Issuer for House {
    const OPERATION_FAMILY: &'static str = "market_house_mind_renewal_operation";
    const RECEIPT_FAMILY: &'static str = "market_house_mind_renewal_receipt";
    const POLICY_KIND: &'static str = "house_renewal_policy";
    const OUTCOME_ID: &'static str = "house_mind_renewal_recorded";
    const BINDING_KIND: &'static str = "house_mind_renewal_binding";
    const VERBS: [&'static str; 5] = [
        "house-renewal-draft",
        "house-renewal-fund",
        "house-renewal-grant",
        "house-renewal-refund",
        "house-renewal-close",
    ];
    const SERVICE_VERB: &'static str = "house-renewal-service";
    const COMMAND: &'static str = "@housemindrenewal";
    const HELP: &'static str = "@housemindrenewal/status <licence> — inspect your retained offer. /request asks for an offer; /accept explicitly pays it; /decline closes it; /retry requests a fresh offer after closure. Processing is acknowledged asynchronously.";
    const SERVICE_TICK: &'static str = "house_mind_renewal_service";
    const INTENT_DOMAIN: &'static str = "market.house_renewal_service_intent.v1";
    const SUBMISSION_DOMAIN: &'static str = "market.house_renewal_submission.v1";

    type Renewal = MindRenewalOperationV1;
    type Receipt = MindRenewalReceiptV1;
    type Purchase = HousePurchase;
    type Inventory = RenewalInventory;
    type Entry = RenewalInventoryEntry;

    fn purchase_references(purchase: &HousePurchase) -> Vec<DefinitionRef> {
        std::iter::once(purchase.purchase_policy.clone())
            .chain(purchase.two_seat_purchase_policy.iter().cloned())
            .collect()
    }
    fn admits(config: &RenewalConfig, facts: &RenewalFacts) -> bool {
        config.accepts_purchase_template(facts.binding().grant().renewal())
    }
    fn remainder(
        config: &RenewalConfig,
        e: &EconomyStore,
        _: &World,
        facts: &RenewalFacts,
        instrument: &InstrumentId,
    ) -> Result<Recipient, String> {
        let binding = facts.binding();
        if !config.accepts_purchase_template(binding.grant().renewal()) {
            return Err(twin::records::invalid());
        }
        let issuer = binding.issuer();
        let principal = issuer.principal();
        let snapshot = e.snapshot();
        let treasury = {
            let mut treasuries = snapshot.accounts_of(&principal).filter(|a| {
                a.instrument == *instrument
                    && a.owner == principal
                    && a.scope == Some(issuer.scope())
                    && a.status == AccountStatus::Active
                    && a.approval.is_none()
            });
            let treasury = treasuries.next().ok_or_else(twin::records::invalid)?;
            if treasuries.next().is_some() {
                return Err(twin::records::invalid());
            }
            treasury.id
        };
        Ok(Recipient {
            account: treasury,
            owner: principal,
        })
    }
    fn offer(
        config: &RenewalConfig,
        _: &EconomyStore,
        _: &World,
        facts: &RenewalFacts,
        parts: OfferParts,
    ) -> Result<RenewalOffer, String> {
        let invalid = |_| twin::records::invalid();
        let binding = facts.binding();
        let issuer = binding.issuer();
        Ok(RenewalOffer {
            format: 1,
            basis: RenewalBasisV1 {
                format: 1,
                licence: binding.licence().get(),
                predecessor_generation: binding.predecessor().get(),
                next_generation: binding.next().get(),
                predecessor_operation: binding.predecessor_operation(),
                prior_quote: facts.prior_quote(),
                prior_quote_digest: facts.prior_quote_digest(),
                product_digest: binding.product_digest(),
                template_digest: binding.template_digest().map_err(invalid)?,
                grant_economics_digest: binding.grant().digest().map_err(invalid)?,
            },
            terms: RenewalOfferTermsV1 {
                format: 1,
                buyer: parts.buyer,
                organization: issuer.organization(),
                scope: issuer.scope(),
                charter: ObjId(binding.charter().get()),
                title_revision: facts.sale().title_revision(),
                payer: parts.payer,
                instrument: parts.instrument.clone(),
                price: binding.grant().renewal().price(),
                registry: config.policy.registry.clone(),
                venue: config.policy.venue.clone(),
                remainder: parts.remainder,
                not_before: parts.now,
                expires_at: parts.expires_at,
            },
        })
    }
    fn decode_offer(bytes: &[u8]) -> Result<RenewalOffer, ParticipantError> {
        RenewalOffer::decode(bytes)
    }
    fn registry(terms: &RenewalOfferTermsV1) -> &GrossRecipientCut {
        &terms.registry
    }
    fn venue(terms: &RenewalOfferTermsV1) -> Option<&GrossRecipientCut> {
        terms.venue.as_ref()
    }
    fn parent_permille(_: &RenewalOfferTermsV1) -> u32 {
        // A House Pin's offer carries no parent royalties.
        0
    }
    fn status_line(view: &RenewalStatusView) -> String {
        player::house_status_line(view)
    }

    fn draft_request(offer: RenewalOffer) -> EconomyRequest {
        EconomyRequest::HouseRenewalDraft(offer)
    }
    fn retry_request(offer: RenewalOffer, previous_operation: CommitToken) -> EconomyRequest {
        EconomyRequest::HouseRenewalRetry {
            offer,
            previous_operation,
        }
    }
    fn holder_draft_request(offer: RenewalOffer, basis: RenewalHolderBasisV1) -> EconomyRequest {
        EconomyRequest::HouseRenewalHolderDraft { offer, basis }
    }
    fn holder_retry_request(
        offer: RenewalOffer,
        basis: RenewalHolderBasisV1,
        previous_operation: CommitToken,
    ) -> EconomyRequest {
        EconomyRequest::HouseRenewalHolderRetry {
            offer,
            basis,
            previous_operation,
        }
    }
    fn funding_request(money: RenewalMoneyRequest) -> EconomyRequest {
        EconomyRequest::HouseRenewalFunding(money)
    }
    fn grant_request(money: RenewalMoneyRequest) -> EconomyRequest {
        EconomyRequest::HouseRenewalGrant(money)
    }
    fn refund_request(money: RenewalMoneyRequest) -> EconomyRequest {
        EconomyRequest::HouseRenewalRefund(money)
    }
    fn close_request(operation: CommitToken, reason: UnfundedCloseReason) -> EconomyRequest {
        EconomyRequest::HouseRenewalClose { operation, reason }
    }
    fn service_close_request(
        operation: CommitToken,
        reason: RenewalServiceCloseReason,
    ) -> EconomyRequest {
        EconomyRequest::HouseRenewalServiceClose { operation, reason }
    }
    fn offer_output(output: EconomyOutput) -> Result<RenewalOfferOutput<House>, ParticipantError> {
        output.house_renewal_offer()
    }
    fn funding_output(
        output: EconomyOutput,
    ) -> Result<RenewalFundingOutput<House>, ParticipantError> {
        output.house_renewal_funding()
    }
    fn terminal_output(
        output: EconomyOutput,
    ) -> Result<RenewalTerminalOutput<House>, ParticipantError> {
        output.house_renewal_terminal()
    }
    fn economy_audit(e: &EconomyStore) -> Result<RenewalAudit<'_, House>, RenewalAuditError> {
        e.house_renewal_audit()
    }
    fn require_no_economy_history(e: &EconomyStore) -> Result<(), RenewalAuditError> {
        e.require_no_house_renewal_history()
    }

    fn reserve_completion(
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
    ) -> Result<(), mush_core::action_state::ActionStateError> {
        ctx.reserve_house_mind_renewal_completion(txn)
    }
    fn advance_completion(
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        operation: CommitToken,
    ) -> Result<(), mush_core::action_state::ActionStateError> {
        ctx.advance_house_mind_renewal_completion(txn, operation)
    }
    fn capacity(
        reg: &mut Registrar<'_>,
        manifest: DefinitionRef,
        participants: [DefinitionRef; 2],
        actions: [DefinitionRef; 5],
    ) -> Result<(), ModError> {
        reg.house_mind_renewal_capacity(manifest, participants, actions)
    }
    fn capacity_with_service(
        reg: &mut Registrar<'_>,
        manifest: DefinitionRef,
        participants: [DefinitionRef; 2],
        actions: [DefinitionRef; 5],
        service_close: DefinitionRef,
    ) -> Result<(), ModError> {
        reg.house_mind_renewal_capacity_with_service(manifest, participants, actions, service_close)
    }

    fn request_reader(
        reg: &mut Registrar<'_>,
        manifest: &DefinitionRef,
        service: bool,
    ) -> Result<OwnerReadHandle<RenewalReader>, ModError> {
        RenewalReader::request_configured(reg, manifest, service)
    }
    fn read_own(licence: LicenceId) -> RenewalRequest {
        RenewalRequest::own(licence)
    }
    fn read_status(licence: LicenceId) -> RenewalRequest {
        RenewalRequest::status(licence)
    }
    fn read_inventory(after: Option<String>) -> RenewalRequest {
        RenewalRequest::inventory(after)
    }
    fn read_operation(operation: CommitToken) -> RenewalRequest {
        RenewalRequest::operation(operation)
    }
    fn facts_of(read: twin::Read<House>) -> Result<RenewalFacts, OwnerReadError> {
        read.into_facts()
    }
    fn inventory_of(read: twin::Read<House>) -> Result<RenewalInventory, OwnerReadError> {
        read.into_inventory()
    }
    fn operation_of(read: twin::Read<House>) -> Result<RenewalOperationV1, OwnerReadError> {
        read.into_operation()
    }
    fn term_end(facts: &RenewalFacts) -> WorldIcSeconds {
        facts.predecessor_term_end()
    }
}

// Doc 46 CT.6: test-only paths. The family's tests name the composition's items by the module
// paths they had before CT.6 moved them into `renewal_twin`; these are those paths, at this
// family's issuer, so no test body changed. Nothing outside the tests reads them.
#[cfg(test)]
mod actions {
    use mush_roll::commerce::house_renewal::protocol::Transition;
    pub(crate) fn parameters(
        kind: Transition,
    ) -> Result<Vec<mush_core::ProjectedParameter>, String> {
        crate::renewal_twin::actions::parameters(kind.step())
    }
    fn verb(kind: Transition) -> &'static str {
        crate::renewal_twin::actions::verb::<mush_economy::renewal_twin::House>(kind.step())
    }
    mod presentation_tests {
        use super::*;
        #[test]
        fn renewal_transitions_have_unique_line_selectors() {
            let verbs: std::collections::BTreeSet<_> = [
                Transition::Draft,
                Transition::Fund,
                Transition::Grant,
                Transition::Refund,
                Transition::Close,
            ]
            .into_iter()
            .map(verb)
            .collect();
            assert_eq!(
                verbs,
                std::collections::BTreeSet::from([
                    "house-renewal-draft",
                    "house-renewal-fund",
                    "house-renewal-grant",
                    "house-renewal-refund",
                    "house-renewal-close",
                ])
            );
        }
    }
}
#[cfg(test)]
mod capacity {
    pub(crate) fn check(
        f: &super::Families,
        row: &super::MindRenewalOperationV1,
    ) -> Result<(), String> {
        crate::renewal_twin::capacity::check(f, row)
    }
}
#[cfg(test)]
mod families {
    pub(crate) use crate::renewal_twin::families::family;
}
#[cfg(test)]
pub(crate) mod service {
    pub(crate) use crate::renewal_twin::service::ct0_house as ct0_service;
    use mush_economy::renewal_twin::House;
    pub(crate) type CloseIntent = crate::renewal_twin::service::CloseIntent<House>;
    pub(crate) fn command_line(outcome: crate::mind_renewal::RenewalPlayerOutcome) -> String {
        crate::renewal_twin::service::command_line::<House>(outcome)
    }
    pub(crate) fn status_view(
        term_end_ic: i64,
        row: Option<&super::MindRenewalOperationV1>,
    ) -> crate::mind_renewal::RenewalStatusView {
        crate::renewal_twin::service::status_view::<House>(term_end_ic, row)
    }
}

#[cfg(test)]
pub(crate) mod tests;

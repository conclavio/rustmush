//! Explicit personal Pin renewal composition. Decoded data conveys no owner
//! authority. Providers and paired boot exist only under the optional config;
//! the separately enabled service installs bounded continuations and controls,
//! never automatic acceptance.
//!
//! Doc 46 CT.6: the personal instantiation. The market's renewal composition is written once
//! for both issuers in `renewal_twin`; this module is the personal issuer's implementation of
//! the twin's trait (its identity strings, its records, its Economy and kernel entries, its
//! reader, and the rules only it has: the seller as payee and the royalty parents of its
//! offers), the player words both families share (`player`), and every path its callers
//! name, now the twin's types and functions at the personal issuer.
pub mod player;

mod records {
    crate::renewal_twin::records::records!(
        mush_economy::renewal_twin::Personal,
        mush_roll::commerce::renewal::protocol
    );
}
mod reader_face {
    crate::renewal_twin::reader::reader_face!(mush_roll::commerce::renewal::reader);
}

use crate::renewal_twin::{self as twin, Issuer, OfferParts};
use mush_core::action::AuthorityGuard;
use mush_core::action::AuthorityGuardContext;
use mush_core::participant::ParticipantError;
use mush_core::read_service::{OwnerReadError, OwnerReadHandle};
use mush_core::{
    ActionTxn, CmdError, CommitToken, ContinuationHandle, Ctx, DefinitionRef, ModError, ModuleCtx,
    ObjId, PrepareCtx, Registrar, World,
};
use mush_economy::managed_entitlement::{
    GrossRecipientCut, Recipient, RoyaltyRecipientCut, RoyaltyTitleRecipient,
};
use mush_economy::managed_paid_resale::RenewalHolderBasisV1;
use mush_economy::managed_renewal::{
    RenewalBasisV1, RenewalLineageV1, RenewalOffer, RenewalOfferTermsV1,
};
use mush_economy::registry_filing::{EconomyOutput, EconomyRequest};
use mush_economy::renewal_twin::{
    Personal, RenewalAudit, RenewalAuditError, RenewalFundingOutput, RenewalMoneyRequest,
    RenewalOfferOutput, RenewalServiceCloseReason, RenewalTerminalOutput, UnfundedCloseReason,
};
use mush_economy::{AssetLifecycle, EconomyStore, InstrumentId};
use mush_roll::commerce::renewal::reader::{
    RenewalFacts, RenewalInventory, RenewalInventoryEntry, RenewalReader, RenewalRequest,
};
use mush_roll::commerce::renewal::{RenewalOperationV1, writer::RenewalWriter};
use mush_roll::commerce::{LicenceId, WorldIcSeconds, purchase::PurchaseParticipants};

pub use crate::renewal_twin::config::RenewalPolicy;
pub use player::{RenewalPlayerOutcome, RenewalPlayerStep, RenewalState, RenewalStatusView};
pub use records::{MindRenewalOperationV1, MindRenewalReceiptV1};

/// The personal issuer's renewal composition (`renewal_twin::config::RenewalConfig`).
pub type RenewalConfig = twin::config::RenewalConfig<Personal>;
pub(crate) type Families = twin::families::Families<Personal>;

impl RenewalConfig {
    pub fn new(
        manifest: DefinitionRef,
        participants: PurchaseParticipants,
        writer: RenewalWriter,
        policy: RenewalPolicy,
    ) -> Result<Self, ModError> {
        policy.validate()?;
        Self::of(manifest, participants, writer, policy, ())
    }
}

pub(crate) fn require_disabled(ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
    twin::audit::require_disabled::<Personal>(ctx)
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    config: &RenewalConfig,
) -> Result<Families, ModError> {
    twin::registration::register(reg, config)
}
pub(crate) fn enable_service(ctx: &ModuleCtx<'_>) -> Result<(), ModError> {
    twin::service::enable::<Personal>(ctx)
}
pub(crate) fn service_guard_current(
    ctx: &AuthorityGuardContext<'_>,
    guard: &AuthorityGuard,
) -> bool {
    twin::service::guard_current::<Personal>(ctx, guard)
}

/// 17g C2d (C2 plan §2.1's renewal exception): one player step on the
/// actor's own personal renewal (C2d's front for `+minds/renew`). It is the
/// `@mindrenewal` command's own player branch (`step`): the renewal reader's
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
    twin::service::player_step::<Personal>(ctx, licence, step_kind)
}

/// 17g C2h (the lead's ruling R3, the C2e plan §5.1): the player front's
/// handle, when the service is composed and enabled. TacSim's `+minds/renew`
/// schedules it for the actor whose typed line it is dispatching (the
/// delegation this service declared at composition admits nothing else).
#[must_use]
pub fn player_front(ctx: &Ctx<'_>) -> Option<ContinuationHandle> {
    twin::service::player_front::<Personal>(ctx)
}

/// `player_front`, read from the world itself (the service's state slot and
/// its runtime), so a composition test can prove the handle TacSim would be
/// handed after activation without a dispatched context.
#[must_use]
pub fn player_front_in(world: &World) -> Option<ContinuationHandle> {
    twin::service::player_front_in::<Personal>(world)
}

impl Issuer for Personal {
    const OPERATION_FAMILY: &'static str = "market_mind_renewal_operation";
    const RECEIPT_FAMILY: &'static str = "market_mind_renewal_receipt";
    const POLICY_KIND: &'static str = "renewal_policy";
    const OUTCOME_ID: &'static str = "mind_renewal_recorded";
    const BINDING_KIND: &'static str = "mind_renewal_binding";
    const VERBS: [&'static str; 5] = [
        "mind-renewal-draft",
        "mind-renewal-fund",
        "mind-renewal-grant",
        "mind-renewal-refund",
        "mind-renewal-close",
    ];
    const SERVICE_VERB: &'static str = "mind-renewal-service";
    const COMMAND: &'static str = "@mindrenewal";
    const HELP: &'static str = "@mindrenewal/status <license> — inspect your retained offer. /request asks for an offer; /accept explicitly pays it; /decline closes it; /retry requests a fresh offer after closure. Processing is acknowledged asynchronously.";
    const SERVICE_TICK: &'static str = "personal_mind_renewal_service";
    const INTENT_DOMAIN: &'static str = "market.renewal_service_intent.v1";
    const SUBMISSION_DOMAIN: &'static str = "market.renewal_submission.v1";

    type Renewal = MindRenewalOperationV1;
    type Receipt = MindRenewalReceiptV1;
    type Purchase = ();
    type Inventory = RenewalInventory;
    type Entry = RenewalInventoryEntry;

    fn purchase_references((): &()) -> Vec<DefinitionRef> {
        Vec::new()
    }
    fn admits(_: &RenewalConfig, _: &RenewalFacts) -> bool {
        true
    }
    fn remainder(
        _: &RenewalConfig,
        e: &EconomyStore,
        _: &World,
        facts: &RenewalFacts,
        instrument: &InstrumentId,
    ) -> Result<Recipient, String> {
        let seller = facts.sale().seller();
        Ok(Recipient {
            account: twin::offer::personal(e.snapshot(), instrument, seller)?.id,
            owner: mush_org::PrincipalId::Object(seller),
        })
    }
    fn offer(
        config: &RenewalConfig,
        e: &EconomyStore,
        world: &World,
        facts: &RenewalFacts,
        parts: OfferParts,
    ) -> Result<RenewalOffer, String> {
        let invalid = |_| twin::records::invalid();
        let instrument = &parts.instrument;
        let mut parents = Vec::with_capacity(3);
        for edge in facts.sale().royalty().edges() {
            if parents.len() == 3 {
                return Err(twin::records::invalid());
            }
            let charter = ObjId(edge.charter().get());
            let permille = edge.royalty_permille();
            let title = if permille == 0 {
                None
            } else {
                let title = e
                    .snapshot()
                    .asset(charter)
                    .ok_or_else(twin::records::invalid)?;
                let mush_org::PrincipalId::Object(owner) = &title.title_owner else {
                    return Err(twin::records::invalid());
                };
                if title.scope.is_some()
                    || title.frozen
                    || title.stolen.is_some()
                    || title.lifecycle != AssetLifecycle::Active
                    || world
                        .get(charter)
                        .is_none_or(|o| o.flags.contains(mush_core::Flags::GOING))
                    || world.get(*owner).is_none_or(|o| {
                        o.kind != mush_core::ObjKind::Player
                            || o.flags.contains(mush_core::Flags::GOING)
                    })
                {
                    return Err(twin::records::invalid());
                }
                Some(RoyaltyTitleRecipient {
                    title_revision: title.revision,
                    recipient: Recipient {
                        account: twin::offer::personal(e.snapshot(), instrument, *owner)?.id,
                        owner: mush_org::PrincipalId::Object(*owner),
                    },
                })
            };
            parents.push(RoyaltyRecipientCut {
                charter,
                permille,
                title,
            });
        }
        let binding = facts.binding();
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
                royalty_chain_digest: binding.grant().royalty_chain().digest(),
            },
            terms: RenewalOfferTermsV1 {
                format: 1,
                lineage: if facts.sale().parent().is_some() {
                    RenewalLineageV1::Derived
                } else {
                    RenewalLineageV1::Original
                },
                buyer: parts.buyer,
                seller: facts.sale().seller(),
                charter: ObjId(binding.charter().get()),
                title_revision: facts.sale().title_revision(),
                payer: parts.payer,
                instrument: parts.instrument.clone(),
                price: binding.grant().renewal().price(),
                registry: config.policy.registry.clone(),
                venue: config.policy.venue.clone(),
                parents,
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
    fn parent_permille(terms: &RenewalOfferTermsV1) -> u32 {
        terms
            .parents
            .iter()
            .map(|p| u32::from(p.permille))
            .sum::<u32>()
    }
    fn status_line(view: &RenewalStatusView) -> String {
        player::personal_status_line(view)
    }

    fn draft_request(offer: RenewalOffer) -> EconomyRequest {
        EconomyRequest::RenewalDraft(offer)
    }
    fn retry_request(offer: RenewalOffer, previous_operation: CommitToken) -> EconomyRequest {
        EconomyRequest::RenewalRetry {
            offer,
            previous_operation,
        }
    }
    fn holder_draft_request(offer: RenewalOffer, basis: RenewalHolderBasisV1) -> EconomyRequest {
        EconomyRequest::RenewalHolderDraft { offer, basis }
    }
    fn holder_retry_request(
        offer: RenewalOffer,
        basis: RenewalHolderBasisV1,
        previous_operation: CommitToken,
    ) -> EconomyRequest {
        EconomyRequest::RenewalHolderRetry {
            offer,
            basis,
            previous_operation,
        }
    }
    fn funding_request(money: RenewalMoneyRequest) -> EconomyRequest {
        EconomyRequest::RenewalFunding(money)
    }
    fn grant_request(money: RenewalMoneyRequest) -> EconomyRequest {
        EconomyRequest::RenewalGrant(money)
    }
    fn refund_request(money: RenewalMoneyRequest) -> EconomyRequest {
        EconomyRequest::RenewalRefund(money)
    }
    fn close_request(operation: CommitToken, reason: UnfundedCloseReason) -> EconomyRequest {
        EconomyRequest::RenewalClose { operation, reason }
    }
    fn service_close_request(
        operation: CommitToken,
        reason: RenewalServiceCloseReason,
    ) -> EconomyRequest {
        EconomyRequest::RenewalServiceClose { operation, reason }
    }
    fn offer_output(
        output: EconomyOutput,
    ) -> Result<RenewalOfferOutput<Personal>, ParticipantError> {
        output.renewal_offer()
    }
    fn funding_output(
        output: EconomyOutput,
    ) -> Result<RenewalFundingOutput<Personal>, ParticipantError> {
        output.renewal_funding()
    }
    fn terminal_output(
        output: EconomyOutput,
    ) -> Result<RenewalTerminalOutput<Personal>, ParticipantError> {
        output.renewal_terminal()
    }
    fn economy_audit(e: &EconomyStore) -> Result<RenewalAudit<'_, Personal>, RenewalAuditError> {
        e.renewal_audit()
    }
    fn require_no_economy_history(e: &EconomyStore) -> Result<(), RenewalAuditError> {
        e.require_no_renewal_history()
    }

    fn reserve_completion(
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
    ) -> Result<(), mush_core::action_state::ActionStateError> {
        ctx.reserve_mind_renewal_completion(txn)
    }
    fn advance_completion(
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        operation: CommitToken,
    ) -> Result<(), mush_core::action_state::ActionStateError> {
        ctx.advance_mind_renewal_completion(txn, operation)
    }
    fn capacity(
        reg: &mut Registrar<'_>,
        manifest: DefinitionRef,
        participants: [DefinitionRef; 2],
        actions: [DefinitionRef; 5],
    ) -> Result<(), ModError> {
        reg.mind_renewal_capacity(manifest, participants, actions)
    }
    fn capacity_with_service(
        reg: &mut Registrar<'_>,
        manifest: DefinitionRef,
        participants: [DefinitionRef; 2],
        actions: [DefinitionRef; 5],
        service_close: DefinitionRef,
    ) -> Result<(), ModError> {
        reg.mind_renewal_capacity_with_service(manifest, participants, actions, service_close)
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
    fn facts_of(read: twin::Read<Personal>) -> Result<RenewalFacts, OwnerReadError> {
        read.into_facts()
    }
    fn inventory_of(read: twin::Read<Personal>) -> Result<RenewalInventory, OwnerReadError> {
        read.into_inventory()
    }
    fn operation_of(read: twin::Read<Personal>) -> Result<RenewalOperationV1, OwnerReadError> {
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
    use mush_roll::commerce::renewal::protocol::Transition;
    pub(crate) fn parameters(
        kind: Transition,
    ) -> Result<Vec<mush_core::ProjectedParameter>, String> {
        crate::renewal_twin::actions::parameters(kind.step())
    }
    fn verb(kind: Transition) -> &'static str {
        crate::renewal_twin::actions::verb::<mush_economy::renewal_twin::Personal>(kind.step())
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
                    "mind-renewal-draft",
                    "mind-renewal-fund",
                    "mind-renewal-grant",
                    "mind-renewal-refund",
                    "mind-renewal-close",
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
    pub(crate) use super::Families;
    pub(crate) use crate::renewal_twin::families::family;
}
#[cfg(test)]
mod offer {
    pub(crate) use crate::renewal_twin::offer::personal;
}
#[cfg(test)]
mod provider {
    pub(crate) use crate::renewal_twin::provider::{Request, require_due};
    use mush_roll::commerce::renewal::protocol::Transition;
    pub(crate) fn decode(kind: Transition, value: &serde_json::Value) -> Result<Request, String> {
        crate::renewal_twin::provider::decode(kind.step(), value)
    }
}
#[cfg(test)]
pub(crate) mod service {
    pub(crate) use crate::renewal_twin::service::ct0_personal as ct0_service;
    use mush_economy::renewal_twin::Personal;
    pub(crate) type CloseIntent = crate::renewal_twin::service::CloseIntent<Personal>;
    pub(crate) fn command_line(outcome: crate::mind_renewal::RenewalPlayerOutcome) -> String {
        crate::renewal_twin::service::command_line::<Personal>(outcome)
    }
    pub(crate) fn status_view(
        term_end_ic: i64,
        row: Option<&super::MindRenewalOperationV1>,
    ) -> crate::mind_renewal::RenewalStatusView {
        crate::renewal_twin::service::status_view::<Personal>(term_end_ic, row)
    }
}

#[cfg(test)]
pub(crate) mod tests;

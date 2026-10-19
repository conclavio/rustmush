//! Closed renewal owner preparation and explicitly configured Economy dispatch.
//! Paired Roll/Market capacity and final audits remain separate prerequisites;
//! default composition grants no renewal action.

use mush_core::participant::{OwnerPrepareContext, ParticipantError};
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, DefinitionRef};
use serde::{Deserialize, Serialize};

use crate::contract::managed::ManagedConsent;
use crate::contract::{
    ConsentSurface, Consideration, ContractRoles, ContractState, ContractTerms, EconomicContract,
    Expiry, PayoutShare, Share, consent_hash,
};
use crate::module::EconomyStore;
use crate::record::EconomicVisibility;
use crate::{ContractId, EconomicClock, EconomicDeadline};
use mush_org::PrincipalId;

mod subject;
pub use subject::{RenewalBasisV1, RenewalOffer, RenewalOfferTermsV1};
pub(crate) mod audit;
pub(crate) mod bridge;
pub use audit::{RenewalAudit, RenewalClosureView, RenewalOfferView};
pub(crate) mod funding;
pub(crate) mod history;
pub(crate) mod install;
pub(crate) mod service;
pub use service::{RenewalServiceCloseReason, RenewalServiceClosureV1, SERVICE_CLOSE_ACTION};
pub(crate) mod money_install;
pub(crate) mod receipt;
pub(crate) use funding::ClosedHouseRenewalPlan;
pub use funding::{RenewalFundingOutput, RenewalMoneyRequest, RenewalTerminalOutput};
pub use receipt::{
    MAX_RENEWAL_RECEIPT_BYTES, RenewalAuditError, RenewalFundingReceiptV1, RenewalHistory,
    RenewalOutcome, RenewalTerminalReceiptV1,
};

/// Root-selected provisional launch default; no scheduler is activated here.
pub const DEFAULT_RENEWAL_OFFER_IC_SECONDS: i64 = 259_200;
pub const MAX_RENEWAL_OFFER_IC_SECONDS: i64 = 259_200;
/// Outer limit for plain renewal-offer bytes, before parsing any value.
pub const MAX_RENEWAL_OFFER_BYTES: usize = 8_192;
pub(crate) const DRAFT_ACTION: &str = "draft_house_mind_renewal";
pub(crate) const CLOSE_ACTION: &str = "close_house_mind_renewal";
pub(crate) const FUND_ACTION: &str = "fund_house_mind_renewal";
pub(crate) const GRANT_ACTION: &str = "settle_house_mind_renewal";
pub(crate) const REFUND_ACTION: &str = "refund_house_mind_renewal";
pub(crate) const NAMESPACE: &str = "market.mind.house_renewal.v1";
fn refused() -> ParticipantError {
    ParticipantError::refused("The renewal offer is unavailable or has changed.")
}
fn action_matches(action: &DefinitionRef, id: &str) -> bool {
    action.key().owner.as_str() == "market"
        && action.key().kind.as_str() == "action"
        && action.key().id.as_str() == id
        && action.version() == 1
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RenewalMarkerV1 {
    format: u16,
    operation: CommitToken,
    effect: DefinitionRef,
    basis: RenewalBasisV1,
    terms_digest: DefinitionDigest,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkerWire {
    format: u16,
    operation: CommitToken,
    effect: DefinitionRef,
    basis: RenewalBasisV1,
    terms_digest: DefinitionDigest,
}
impl<'de> Deserialize<'de> for RenewalMarkerV1 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = MarkerWire::deserialize(d)?;
        let v = Self {
            format: w.format,
            operation: w.operation,
            effect: w.effect,
            basis: w.basis,
            terms_digest: w.terms_digest,
        };
        v.validate().map_err(serde::de::Error::custom)?;
        Ok(v)
    }
}
impl RenewalMarkerV1 {
    pub(crate) fn new(
        operation: CommitToken,
        effect: DefinitionRef,
        offer: &RenewalOffer,
    ) -> Result<Self, ParticipantError> {
        let value = Self {
            format: 1,
            operation,
            effect,
            basis: offer.basis.clone(),
            terms_digest: offer.digest()?,
        };
        value.validate()?;
        Ok(value)
    }
    pub(crate) fn validate(&self) -> Result<(), ParticipantError> {
        self.basis.validate()?;
        if self.format != 1
            || !action_matches(&self.effect, DRAFT_ACTION)
            || self.operation == self.basis.predecessor_operation
            || self.operation == self.basis.prior_quote
        {
            return Err(refused());
        }
        Ok(())
    }
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub fn effect(&self) -> &DefinitionRef {
        &self.effect
    }
    pub const fn basis(&self) -> &RenewalBasisV1 {
        &self.basis
    }
    pub const fn terms_digest(&self) -> DefinitionDigest {
        self.terms_digest
    }
}

/// Read-only attempt identity. Decoding data never authorizes a new offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RenewalAttemptV1 {
    Automatic,
    BuyerRetry {
        previous_operation: CommitToken,
    },
    HolderAutomatic {
        basis: crate::managed_paid_resale::RenewalHolderBasisV1,
    },
    HolderRetry {
        basis: crate::managed_paid_resale::RenewalHolderBasisV1,
        previous_operation: CommitToken,
    },
}

impl RenewalAttemptV1 {
    pub const fn holder_basis(self) -> Option<crate::managed_paid_resale::RenewalHolderBasisV1> {
        match self {
            Self::HolderAutomatic { basis } | Self::HolderRetry { basis, .. } => Some(basis),
            _ => None,
        }
    }
    pub const fn previous_operation(self) -> Option<CommitToken> {
        match self {
            Self::BuyerRetry { previous_operation }
            | Self::HolderRetry {
                previous_operation, ..
            } => Some(previous_operation),
            _ => None,
        }
    }
    pub const fn record_format(self) -> u16 {
        match self {
            Self::Automatic => 1,
            Self::BuyerRetry { .. } => 2,
            Self::HolderAutomatic { .. } => 3,
            Self::HolderRetry { .. } => 4,
        }
    }
}

/// Opaque actual owner output; no decoder, clone or public constructor.
pub struct RenewalOfferOutput {
    operation: CommitToken,
    contract: ContractId,
    consent_hash: [u8; 32],
    at_ic: ClockStamp,
    closed: Option<UnfundedCloseReason>,
    service_closed: Option<RenewalServiceClosureV1>,
    attempt: RenewalAttemptV1,
    offer_digest: DefinitionDigest,
    commit: CommitToken,
    action: DefinitionRef,
    posted_at_utc: i64,
}
impl RenewalOfferOutput {
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub const fn contract(&self) -> ContractId {
        self.contract
    }
    pub const fn consent_hash(&self) -> [u8; 32] {
        self.consent_hash
    }
    pub fn at_ic(&self) -> &ClockStamp {
        &self.at_ic
    }
    pub const fn closed(&self) -> Option<UnfundedCloseReason> {
        self.closed
    }
    pub fn service_closed(&self) -> Option<&RenewalServiceClosureV1> {
        self.service_closed.as_ref()
    }
    pub const fn attempt(&self) -> RenewalAttemptV1 {
        self.attempt
    }
    /// Digest of the exact validated offer in the actual prepared owner image.
    pub const fn offer_digest(&self) -> DefinitionDigest {
        self.offer_digest
    }
    /// Actual preparation identity: Draft on creation, Close on unfunded closure.
    /// The permanent operation remains the original Draft commit in both cases.
    pub const fn commit(&self) -> CommitToken {
        self.commit
    }
    pub fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub const fn posted_at_utc(&self) -> i64 {
        self.posted_at_utc
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnfundedCloseReason {
    Declined,
    Expired,
}

pub(crate) fn prepare_offer(
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: RenewalOffer,
) -> Result<
    (
        RenewalOfferOutput,
        install::OfferPatch,
        Vec<mush_core::authoritative::ReplayWrite>,
    ),
    ParticipantError,
> {
    install::offer(state, &install::OfferContext::from(context), request)
}
/// Explicit buyer retry; the closed owner proves the exact previous terminal.
/// A decoded operation/link alone supplies no permission to create a contract.
pub(crate) fn prepare_retry(
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: RenewalOffer,
    previous_operation: CommitToken,
) -> Result<
    (
        RenewalOfferOutput,
        install::OfferPatch,
        Vec<mush_core::authoritative::ReplayWrite>,
    ),
    ParticipantError,
> {
    install::retry(
        state,
        &install::OfferContext::from(context),
        request,
        previous_operation,
    )
}

pub(crate) fn prepare_holder_offer(
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    offer: RenewalOffer,
    basis: crate::managed_paid_resale::RenewalHolderBasisV1,
    previous_operation: Option<CommitToken>,
) -> Result<
    (
        RenewalOfferOutput,
        install::OfferPatch,
        Vec<mush_core::authoritative::ReplayWrite>,
    ),
    ParticipantError,
> {
    install::holder_offer(
        state,
        &install::OfferContext::from(context),
        offer,
        basis,
        previous_operation,
    )
}

pub(crate) fn prepare_close(
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    operation: CommitToken,
    reason: UnfundedCloseReason,
) -> Result<
    (
        RenewalOfferOutput,
        install::OfferPatch,
        Vec<mush_core::authoritative::ReplayWrite>,
    ),
    ParticipantError,
> {
    install::close(
        state,
        &install::OfferContext::from(context),
        operation,
        reason,
    )
}

pub(crate) fn prepare_service_close(
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    operation: CommitToken,
    reason: RenewalServiceCloseReason,
) -> Result<
    (
        RenewalOfferOutput,
        install::OfferPatch,
        Vec<mush_core::authoritative::ReplayWrite>,
    ),
    ParticipantError,
> {
    install::service_close(
        state,
        &install::OfferContext::from(context),
        operation,
        reason,
    )
}

/// Private closed owner boundary; participant activation remains a separate cut.
pub(crate) fn prepare_funding(
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: RenewalMoneyRequest,
) -> Result<
    (
        RenewalFundingOutput,
        money_install::RenewalMoneyPatch,
        Vec<mush_core::authoritative::ReplayWrite>,
    ),
    ParticipantError,
> {
    let (output, patch, writes) =
        money_install::prepare_images(state, context, request, funding::Transition::Fund)?;
    let funding::MoneyOutput::Funded(output) = output else {
        return Err(refused());
    };
    Ok((output, patch, writes))
}
pub(crate) fn prepare_terminal(
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: RenewalMoneyRequest,
    outcome: RenewalOutcome,
) -> Result<
    (
        RenewalTerminalOutput,
        money_install::RenewalMoneyPatch,
        Vec<mush_core::authoritative::ReplayWrite>,
    ),
    ParticipantError,
> {
    let transition = if outcome == RenewalOutcome::Granted {
        funding::Transition::Grant
    } else {
        funding::Transition::Refund
    };
    let (output, patch, writes) =
        money_install::prepare_images(state, context, request, transition)?;
    let funding::MoneyOutput::Terminal(output) = output else {
        return Err(refused());
    };
    Ok((output, patch, writes))
}

fn contract(record: &history::OfferRecord) -> Result<EconomicContract, ParticipantError> {
    let offer = &record.offer;
    offer.validate()?;
    let marker = RenewalMarkerV1::new(record.operation, record.action.clone(), offer)?;
    let buyer = PrincipalId::Object(offer.terms.buyer);
    let issuer = PrincipalId::Organization(offer.terms.organization);
    let consent = ConsentSurface {
        fact_consent: None,
        managed: Some(ManagedConsent::house_renewal_offer(marker)?),
        parties: vec![buyer.clone()],
        terms: ContractTerms {
            consideration: vec![Consideration {
                from: buyer.clone(),
                account: offer.terms.payer,
                amount: offer.terms.price,
            }],
            deliverables: Vec::new(),
            split: offer
                .terms
                .payouts()?
                .into_iter()
                .map(|p| PayoutShare {
                    to: p.account,
                    share: Share::Exact(p.amount),
                })
                .collect(),
            remainder_to: offer.terms.remainder.account,
            on_expiry: Expiry::Refund,
            outcomes: Vec::new(),
        },
        conditions: Vec::new(),
        deadline: Some(EconomicDeadline {
            clock: EconomicClock::WorldIc,
            at: offer.terms.expires_at.at,
        }),
        visibility: EconomicVisibility::Parties,
        scope: Some(offer.terms.scope),
        adjudicator: None,
        evidence: Vec::new(),
    };
    let consent_hash = consent_hash(&issuer, &consent);
    let state = match record.closure.as_ref().map(|c| c.reason) {
        None => ContractState::Offered,
        Some(UnfundedCloseReason::Declined) => ContractState::Cancelled,
        Some(UnfundedCloseReason::Expired) => ContractState::Expired,
    };
    Ok(EconomicContract {
        id: record.contract,
        issuer,
        name: "House mind renewal offer".into(),
        consent,
        consent_hash,
        acceptances: Vec::new(),
        satisfied: Vec::new(),
        escrow: Vec::new(),
        state,
        settle_attempts: 0,
        created_at: record.created_at_utc,
        amendments: 0,
        generation_started_at: record.created_at_utc,
        disputed_at: None,
        canon_published: None,
        roster_locked: false,
        deposits: Vec::new(),
        agreements: Vec::new(),
        pending: None,
        roles: ContractRoles::default(),
        revision: if record.closure.is_some() { 2 } else { 1 },
    })
}

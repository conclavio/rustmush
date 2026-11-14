//! The renewal owner, written once for both issuers (doc 46, CT.4).
//!
//! Closed renewal owner preparation and explicitly configured Economy dispatch.
//! Paired Roll/Market capacity and final audits remain separate prerequisites;
//! default composition grants no renewal action.
//!
//! A mind licence is renewed by one of two families: the personal one
//! (`crate::managed_renewal`) and the House one (`crate::managed_house_renewal`). They were
//! two copies of the same owner. Everything they share is here, generic over the sealed
//! [`Issuer`]; what an issuer alone decides (its terms and their payout, its current
//! authority, its identity strings, which of the store's fields are its own) is its
//! implementation of that trait, in its family's file. The two family modules are the two
//! instantiation points: every path a caller or another crate names is still there.

use mush_core::participant::{OwnerPrepareContext, ParticipantError};
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, Flags, ObjId, ObjKind};
use serde::{Deserialize, Serialize};

use crate::contract::{
    ConsentSurface, Consideration, ContractRoles, ContractState, ContractTerms, EconomicContract,
    Expiry, PayoutShare, Share, consent_hash,
};
use crate::module::EconomyStore;
use crate::record::EconomicVisibility;
use crate::{ContractId, EconomicClock, EconomicDeadline};

mod issuer;
pub use issuer::{Basis, House, Issuer, Offer, Personal, Terms};
pub(crate) mod audit;
pub(crate) mod bridge;
pub use audit::{RenewalAudit, RenewalClosureView, RenewalOfferView};
pub(crate) mod funding;
pub(crate) mod history;
pub(crate) mod install;
pub(crate) mod service;
pub use service::{RenewalServiceCloseReason, RenewalServiceClosureV1};
#[cfg(test)]
pub(crate) mod bridge_registration_tests;
pub(crate) mod money_install;
pub(crate) mod receipt;
#[cfg(test)]
pub(crate) mod tests;
pub(crate) mod wire;
pub(crate) use funding::ClosedPlan;
pub use funding::{RenewalFundingOutput, RenewalMoneyRequest, RenewalTerminalOutput};
pub use receipt::{
    MAX_RENEWAL_RECEIPT_BYTES, RenewalAuditError, RenewalFundingReceiptV1, RenewalHistory,
    RenewalOutcome, RenewalTerminalReceiptV1,
};

/// Root-selected provisional launch default; no scheduler is activated here. 28 IC days (3
/// until the owner's rule of 2026-11-04 that a timer waiting on an absent player's response
/// allows at least 28 IC days).
pub const DEFAULT_RENEWAL_OFFER_IC_SECONDS: i64 = 28 * 86_400;
/// The bound on an offer's lifetime, at the default (28 IC days).
pub const MAX_RENEWAL_OFFER_IC_SECONDS: i64 = 28 * 86_400;
/// Outer limit for plain renewal-offer bytes, before parsing any value.
pub const MAX_RENEWAL_OFFER_BYTES: usize = 8_192;
pub(crate) fn refused() -> ParticipantError {
    ParticipantError::refused("The renewal offer is unavailable or has changed.")
}
fn action_matches(action: &DefinitionRef, id: &str) -> bool {
    action.key().owner.as_str() == "market"
        && action.key().kind.as_str() == "action"
        && action.key().id.as_str() == id
        && action.version() == 1
}
pub(crate) fn player(world: &mush_core::World, id: ObjId) -> bool {
    world
        .get(id)
        .is_some_and(|o| o.kind == ObjKind::Player && !o.flags.contains(Flags::GOING))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(bound = "")]
pub struct RenewalMarkerV1<I: Issuer> {
    format: u16,
    operation: CommitToken,
    effect: DefinitionRef,
    basis: I::Basis,
    terms_digest: DefinitionDigest,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, bound = "")]
struct MarkerWire<I: Issuer> {
    format: u16,
    operation: CommitToken,
    effect: DefinitionRef,
    basis: I::Basis,
    terms_digest: DefinitionDigest,
}
impl<'de, I: Issuer> Deserialize<'de> for RenewalMarkerV1<I> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = MarkerWire::<I>::deserialize(d)?;
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
impl<I: Issuer> RenewalMarkerV1<I> {
    pub(crate) fn new(
        operation: CommitToken,
        effect: DefinitionRef,
        offer: &I::Offer,
    ) -> Result<Self, ParticipantError> {
        let value = Self {
            format: 1,
            operation,
            effect,
            basis: offer.basis().clone(),
            terms_digest: offer.digest()?,
        };
        value.validate()?;
        Ok(value)
    }
    pub(crate) fn validate(&self) -> Result<(), ParticipantError> {
        self.basis.validate()?;
        if self.format != 1
            || !action_matches(&self.effect, I::DRAFT_ACTION)
            || self.operation == self.basis.predecessor_operation()
            || self.operation == self.basis.prior_quote()
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
    pub const fn basis(&self) -> &I::Basis {
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
pub struct RenewalOfferOutput<I: Issuer> {
    operation: CommitToken,
    contract: ContractId,
    consent_hash: [u8; 32],
    at_ic: ClockStamp,
    closed: Option<UnfundedCloseReason>,
    service_closed: Option<RenewalServiceClosureV1<I>>,
    attempt: RenewalAttemptV1,
    offer_digest: DefinitionDigest,
    commit: CommitToken,
    action: DefinitionRef,
    posted_at_utc: i64,
}
impl<I: Issuer> RenewalOfferOutput<I> {
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
    pub fn service_closed(&self) -> Option<&RenewalServiceClosureV1<I>> {
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

pub(crate) fn prepare_offer<I: Issuer>(
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: I::Offer,
) -> OfferPrepared<I> {
    install::offer(state, &install::OfferContext::from(context), request)
}
/// Explicit buyer retry; the closed owner proves the exact previous terminal.
/// A decoded operation/link alone supplies no permission to create a contract.
pub(crate) fn prepare_retry<I: Issuer>(
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: I::Offer,
    previous_operation: CommitToken,
) -> OfferPrepared<I> {
    install::retry(
        state,
        &install::OfferContext::from(context),
        request,
        previous_operation,
    )
}

pub(crate) fn prepare_holder_offer<I: Issuer>(
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    offer: I::Offer,
    basis: crate::managed_paid_resale::RenewalHolderBasisV1,
    previous_operation: Option<CommitToken>,
) -> OfferPrepared<I> {
    install::holder_offer(
        state,
        &install::OfferContext::from(context),
        offer,
        basis,
        previous_operation,
    )
}

pub(crate) fn prepare_close<I: Issuer>(
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    operation: CommitToken,
    reason: UnfundedCloseReason,
) -> OfferPrepared<I> {
    install::close(
        state,
        &install::OfferContext::from(context),
        operation,
        reason,
    )
}

pub(crate) fn prepare_service_close<I: Issuer>(
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    operation: CommitToken,
    reason: RenewalServiceCloseReason,
) -> OfferPrepared<I> {
    install::service_close(
        state,
        &install::OfferContext::from(context),
        operation,
        reason,
    )
}

/// Private closed owner boundary; participant activation remains a separate cut.
pub(crate) fn prepare_funding<I: Issuer>(
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: RenewalMoneyRequest,
) -> MoneyPrepared<I, RenewalFundingOutput<I>> {
    let (output, patch, writes) =
        money_install::prepare_images(state, context, request, funding::Transition::Fund)?;
    let funding::MoneyOutput::Funded(output) = output else {
        return Err(refused());
    };
    Ok((output, patch, writes))
}
pub(crate) fn prepare_terminal<I: Issuer>(
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: RenewalMoneyRequest,
    outcome: RenewalOutcome,
) -> MoneyPrepared<I, RenewalTerminalOutput<I>> {
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

fn contract<I: Issuer>(
    record: &history::OfferRecord<I>,
) -> Result<EconomicContract, ParticipantError> {
    let offer = &record.offer;
    offer.validate()?;
    let marker = RenewalMarkerV1::<I>::new(record.operation, record.action.clone(), offer)?;
    let buyer = offer.terms().buyer().principal();
    let consent = ConsentSurface {
        fact_consent: None,
        managed: Some(I::consent(marker)?),
        parties: vec![buyer.clone()],
        terms: ContractTerms {
            consideration: vec![Consideration {
                from: buyer.clone(),
                account: offer.terms().payer(),
                amount: offer.terms().price(),
            }],
            deliverables: Vec::new(),
            split: offer
                .terms()
                .payouts()?
                .into_iter()
                .map(|p| PayoutShare {
                    to: p.account,
                    share: Share::Exact(p.amount),
                })
                .collect(),
            remainder_to: offer.terms().remainder().account,
            on_expiry: Expiry::Refund,
            outcomes: Vec::new(),
        },
        conditions: Vec::new(),
        deadline: Some(EconomicDeadline {
            clock: EconomicClock::WorldIc,
            at: offer.terms().expires_at().at,
        }),
        visibility: EconomicVisibility::Parties,
        scope: I::contract_scope(offer.terms()),
        adjudicator: None,
        evidence: Vec::new(),
    };
    let consent_hash = consent_hash(&buyer, &consent);
    let state = match record.closure.as_ref().map(|c| c.reason) {
        None => ContractState::Offered,
        Some(UnfundedCloseReason::Declined) => ContractState::Cancelled,
        Some(UnfundedCloseReason::Expired) => ContractState::Expired,
    };
    Ok(EconomicContract {
        id: record.contract,
        issuer: buyer,
        name: I::CONTRACT_NAME.into(),
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
        real_deadline_extension_secs: 0,
        paused_real_remaining_secs: None,
    })
}

/// What preparing an offer, a retry or a closure answers: the owner's output, the patch the
/// consuming bind installs, and the writes a replay carries.
pub(crate) type OfferPrepared<I> = Result<
    (
        RenewalOfferOutput<I>,
        Box<install::OfferPatch<I>>,
        Vec<mush_core::authoritative::ReplayWrite>,
    ),
    ParticipantError,
>;
/// The same for a funding or a terminal, whose outputs differ.
pub(crate) type MoneyPrepared<I, O> = Result<
    (
        O,
        Box<money_install::RenewalMoneyPatch<I>>,
        Vec<mush_core::authoritative::ReplayWrite>,
    ),
    ParticipantError,
>;

/// A family's face, expanded once in each family's module.
///
/// The rest of Economy, and the Roll and the market, name the two families by module and by
/// hand (`crate::managed_renewal::prepare_offer`, `managed_house_renewal::RenewalOfferView`,
/// the two fields of the store). This gives each module what its callers name: the generic
/// types at the family's issuer under their old names, the names both families share, the
/// identity constants, and the owner's entry points as ordinary functions. `$plan` is the
/// name the engine's plan guard has for the family's closed plan.
macro_rules! entry_points {
    ($issuer:ty, $plan:ident) => {
        pub use $crate::renewal_twin::{
            DEFAULT_RENEWAL_OFFER_IC_SECONDS, MAX_RENEWAL_OFFER_BYTES,
            MAX_RENEWAL_OFFER_IC_SECONDS, MAX_RENEWAL_RECEIPT_BYTES, RenewalAttemptV1,
            RenewalAuditError, RenewalClosureView, RenewalMoneyRequest, RenewalOutcome,
            RenewalServiceCloseReason, UnfundedCloseReason,
        };
        pub type RenewalMarkerV1 = $crate::renewal_twin::RenewalMarkerV1<$issuer>;
        pub type RenewalOfferOutput = $crate::renewal_twin::RenewalOfferOutput<$issuer>;
        pub type RenewalAudit<'a> = $crate::renewal_twin::RenewalAudit<'a, $issuer>;
        pub type RenewalOfferView<'a> = $crate::renewal_twin::RenewalOfferView<'a, $issuer>;
        pub type RenewalServiceClosureV1 = $crate::renewal_twin::RenewalServiceClosureV1<$issuer>;
        pub type RenewalFundingOutput = $crate::renewal_twin::RenewalFundingOutput<$issuer>;
        pub type RenewalTerminalOutput = $crate::renewal_twin::RenewalTerminalOutput<$issuer>;
        pub type RenewalFundingReceiptV1 = $crate::renewal_twin::RenewalFundingReceiptV1<$issuer>;
        pub type RenewalTerminalReceiptV1 = $crate::renewal_twin::RenewalTerminalReceiptV1<$issuer>;
        pub type RenewalHistory = $crate::renewal_twin::RenewalHistory<$issuer>;
        pub(crate) type $plan = $crate::renewal_twin::ClosedPlan<$issuer>;

        pub const SERVICE_CLOSE_ACTION: &str =
            <$issuer as $crate::renewal_twin::Issuer>::SERVICE_CLOSE_ACTION;
        pub(crate) const DRAFT_ACTION: &str =
            <$issuer as $crate::renewal_twin::Issuer>::DRAFT_ACTION;
        pub(crate) const CLOSE_ACTION: &str =
            <$issuer as $crate::renewal_twin::Issuer>::CLOSE_ACTION;
        pub(crate) const FUND_ACTION: &str = <$issuer as $crate::renewal_twin::Issuer>::FUND_ACTION;
        pub(crate) const GRANT_ACTION: &str =
            <$issuer as $crate::renewal_twin::Issuer>::GRANT_ACTION;
        pub(crate) const REFUND_ACTION: &str =
            <$issuer as $crate::renewal_twin::Issuer>::REFUND_ACTION;
        pub(crate) const NAMESPACE: &str = <$issuer as $crate::renewal_twin::Issuer>::NAMESPACE;

        pub(crate) fn prepare_offer(
            state: &$crate::module::EconomyStore,
            context: &mush_core::participant::OwnerPrepareContext<'_>,
            request: RenewalOffer,
        ) -> $crate::renewal_twin::OfferPrepared<$issuer> {
            $crate::renewal_twin::prepare_offer::<$issuer>(state, context, request)
        }
        /// Explicit buyer retry; the closed owner proves the exact previous terminal.
        /// A decoded operation/link alone supplies no permission to create a contract.
        pub(crate) fn prepare_retry(
            state: &$crate::module::EconomyStore,
            context: &mush_core::participant::OwnerPrepareContext<'_>,
            request: RenewalOffer,
            previous_operation: mush_core::CommitToken,
        ) -> $crate::renewal_twin::OfferPrepared<$issuer> {
            $crate::renewal_twin::prepare_retry::<$issuer>(
                state,
                context,
                request,
                previous_operation,
            )
        }
        pub(crate) fn prepare_holder_offer(
            state: &$crate::module::EconomyStore,
            context: &mush_core::participant::OwnerPrepareContext<'_>,
            offer: RenewalOffer,
            basis: $crate::managed_paid_resale::RenewalHolderBasisV1,
            previous_operation: Option<mush_core::CommitToken>,
        ) -> $crate::renewal_twin::OfferPrepared<$issuer> {
            $crate::renewal_twin::prepare_holder_offer::<$issuer>(
                state,
                context,
                offer,
                basis,
                previous_operation,
            )
        }
        pub(crate) fn prepare_close(
            state: &$crate::module::EconomyStore,
            context: &mush_core::participant::OwnerPrepareContext<'_>,
            operation: mush_core::CommitToken,
            reason: UnfundedCloseReason,
        ) -> $crate::renewal_twin::OfferPrepared<$issuer> {
            $crate::renewal_twin::prepare_close::<$issuer>(state, context, operation, reason)
        }
        pub(crate) fn prepare_service_close(
            state: &$crate::module::EconomyStore,
            context: &mush_core::participant::OwnerPrepareContext<'_>,
            operation: mush_core::CommitToken,
            reason: RenewalServiceCloseReason,
        ) -> $crate::renewal_twin::OfferPrepared<$issuer> {
            $crate::renewal_twin::prepare_service_close::<$issuer>(
                state, context, operation, reason,
            )
        }
        /// Private closed owner boundary; participant activation remains a separate cut.
        pub(crate) fn prepare_funding(
            state: &$crate::module::EconomyStore,
            context: &mush_core::participant::OwnerPrepareContext<'_>,
            request: RenewalMoneyRequest,
        ) -> $crate::renewal_twin::MoneyPrepared<$issuer, RenewalFundingOutput> {
            $crate::renewal_twin::prepare_funding::<$issuer>(state, context, request)
        }
        pub(crate) fn prepare_terminal(
            state: &$crate::module::EconomyStore,
            context: &mush_core::participant::OwnerPrepareContext<'_>,
            request: RenewalMoneyRequest,
            outcome: RenewalOutcome,
        ) -> $crate::renewal_twin::MoneyPrepared<$issuer, RenewalTerminalOutput> {
            $crate::renewal_twin::prepare_terminal::<$issuer>(state, context, request, outcome)
        }

        pub(crate) mod audit {
            pub(crate) fn read(
                state: &$crate::EconomyStore,
            ) -> Result<super::RenewalAudit<'_>, super::RenewalAuditError> {
                $crate::renewal_twin::audit::read::<$issuer>(state)
            }
        }
        pub(crate) mod bridge {
            pub(crate) type Configuration = $crate::renewal_twin::bridge::Configuration<$issuer>;
            pub(crate) fn documents(
                kv: &[(smol_str::SmolStr, serde_json::Value)],
                enabled: bool,
            ) -> Result<Vec<(String, Vec<u8>)>, mush_core::ModError> {
                $crate::renewal_twin::bridge::documents::<$issuer>(kv, enabled)
            }
            pub(crate) fn checkpoint_complete(
                fold: &$crate::fold::FoldState,
                rows: &[$crate::checkpoint::IdemRow],
                enabled: bool,
            ) -> bool {
                $crate::renewal_twin::bridge::checkpoint_complete::<$issuer>(fold, rows, enabled)
            }
            pub(crate) fn recover(
                fold: &mut $crate::fold::FoldState,
                snapshot: &mut $crate::EconomySnapshot,
                contracts: &mut std::collections::BTreeMap<
                    $crate::ContractId,
                    $crate::contract::EconomicContract,
                >,
                holds: &std::collections::BTreeMap<$crate::HoldId, $crate::record::Hold>,
                documents: &[(String, Vec<u8>)],
                enabled: bool,
            ) -> Result<super::history::OfferBook, mush_core::ModError> {
                $crate::renewal_twin::bridge::recover::<$issuer>(
                    fold, snapshot, contracts, holds, documents, enabled,
                )
            }
            pub(crate) fn require_absent(
                state: &$crate::EconomyStore,
            ) -> Result<(), super::RenewalAuditError> {
                $crate::renewal_twin::bridge::require_absent::<$issuer>(state)
            }
        }
        pub(crate) mod history {
            pub(crate) type OfferBook = $crate::renewal_twin::history::OfferBook<$issuer>;
        }
        pub(crate) mod install {
            pub type OfferPatch = $crate::renewal_twin::install::OfferPatch<$issuer>;
            pub type BoundOffer<'a> = $crate::renewal_twin::install::BoundOffer<'a, $issuer>;
        }
        pub(crate) mod money_install {
            pub type RenewalMoneyPatch =
                $crate::renewal_twin::money_install::RenewalMoneyPatch<$issuer>;
            pub type BoundRenewalMoney<'a> =
                $crate::renewal_twin::money_install::BoundRenewalMoney<'a, $issuer>;
        }
        pub(crate) mod receipt {
            pub type RenewalReceiptAudit =
                $crate::renewal_twin::receipt::RenewalReceiptAudit<$issuer>;
        }
        pub(crate) mod service {
            pub(crate) const PREFIX: &str =
                <$issuer as $crate::renewal_twin::Issuer>::SERVICE_PREFIX;
        }
    };
}
pub(crate) use entry_points;

/// What the two families' `subject.rs` files had word for word, expanded once in each: the
/// basis's shape check, the terms' window check, the offer's decode, validation and digest,
/// and the three record traits over the family's own `checked!` records. The records
/// themselves, their payout computation and their check of the issuer's current authority
/// are each family's own, beside the call.
macro_rules! subject_records {
    ($issuer:ty) => {
        impl RenewalBasisV1 {
            pub fn validate(&self) -> Result<(), mush_core::participant::ParticipantError> {
                if self.format != 1
                    || self.licence == 0
                    || self.predecessor_generation == 0
                    || self.predecessor_generation.checked_add(1) != Some(self.next_generation)
                    || self.predecessor_operation == self.prior_quote
                {
                    return Err($crate::renewal_twin::refused());
                }
                Ok(())
            }
        }
        impl $crate::renewal_twin::Basis for RenewalBasisV1 {
            fn validate(&self) -> Result<(), mush_core::participant::ParticipantError> {
                RenewalBasisV1::validate(self)
            }
            fn licence(&self) -> u64 {
                self.licence
            }
            fn predecessor_generation(&self) -> u64 {
                self.predecessor_generation
            }
            fn predecessor_operation(&self) -> mush_core::CommitToken {
                self.predecessor_operation
            }
            fn prior_quote(&self) -> mush_core::CommitToken {
                self.prior_quote
            }
        }

        impl RenewalOfferTermsV1 {
            pub fn validate(&self) -> Result<(), mush_core::participant::ParticipantError> {
                if self.format != 1
                    || !matches!(self.not_before.clock, mush_core::ClockRef::WorldIc { generation } if generation != 0)
                    || self.not_before.clock != self.expires_at.clock
                    || self
                        .expires_at
                        .at
                        .checked_sub(self.not_before.at)
                        .is_none_or(|n| {
                            !(1..=$crate::renewal_twin::MAX_RENEWAL_OFFER_IC_SECONDS).contains(&n)
                        })
                {
                    return Err($crate::renewal_twin::refused());
                }
                self.payouts()?;
                Ok(())
            }
        }
        impl $crate::renewal_twin::Terms for RenewalOfferTermsV1 {
            fn buyer(&self) -> $crate::Holder {
                self.buyer
            }
            fn charter(&self) -> mush_core::ObjId {
                self.charter
            }
            fn payer(&self) -> $crate::AccountId {
                self.payer
            }
            fn instrument(&self) -> &$crate::InstrumentId {
                &self.instrument
            }
            fn price(&self) -> $crate::Amount {
                self.price
            }
            fn remainder(&self) -> &$crate::managed_entitlement::Recipient {
                &self.remainder
            }
            fn not_before(&self) -> &mush_core::ClockStamp {
                &self.not_before
            }
            fn expires_at(&self) -> &mush_core::ClockStamp {
                &self.expires_at
            }
            fn payouts(
                &self,
            ) -> Result<
                Vec<$crate::managed_purchase::PurchasePayout>,
                mush_core::participant::ParticipantError,
            > {
                RenewalOfferTermsV1::payouts(self)
            }
            fn recipients(&self) -> Vec<&$crate::managed_entitlement::Recipient> {
                RenewalOfferTermsV1::recipients(self)
            }
            fn check_current(
                &self,
                state: &$crate::module::EconomyStore,
                world: &mush_core::World,
            ) -> Result<(), mush_core::participant::ParticipantError> {
                RenewalOfferTermsV1::check_current(self, state, world)
            }
        }

        impl RenewalOffer {
            /// Plain data only. Public byte ingress must use this bounded, typed-first
            /// decoder; Deserialize exists for the already bounded owner document.
            pub fn decode(bytes: &[u8]) -> Result<Self, mush_core::participant::ParticipantError> {
                if bytes.len() > $crate::renewal_twin::MAX_RENEWAL_OFFER_BYTES {
                    return Err($crate::renewal_twin::refused());
                }
                let value: Self =
                    serde_json::from_slice(bytes).map_err(|_| $crate::renewal_twin::refused())?;
                value.validate()?;
                let raw: serde_json::Value =
                    serde_json::from_slice(bytes).map_err(|_| $crate::renewal_twin::refused())?;
                if serde_json::to_value(&value)
                    .map_err(|_| mush_core::participant::ParticipantError::Encoding)?
                    != raw
                {
                    return Err($crate::renewal_twin::refused());
                }
                Ok(value)
            }
            pub fn validate(&self) -> Result<(), mush_core::participant::ParticipantError> {
                if self.format != 1 {
                    return Err($crate::renewal_twin::refused());
                }
                self.basis.validate()?;
                self.terms.validate()
            }
            /// Canonical identity of validated plain data, never economic authority.
            /// Paired owners compare it with the digest in the opaque prepared output.
            pub fn digest(
                &self,
            ) -> Result<mush_core::DefinitionDigest, mush_core::participant::ParticipantError> {
                self.validate()?;
                mush_core::DefinitionDigest::for_canonical_value(
                    <$issuer as $crate::renewal_twin::Issuer>::OFFER_DOMAIN,
                    &serde_json::to_value(self)
                        .map_err(|_| mush_core::participant::ParticipantError::Encoding)?,
                )
                .map_err(|_| $crate::renewal_twin::refused())
            }
        }
        impl $crate::renewal_twin::Offer for RenewalOffer {
            type Basis = RenewalBasisV1;
            type Terms = RenewalOfferTermsV1;
            fn basis(&self) -> &RenewalBasisV1 {
                &self.basis
            }
            fn terms(&self) -> &RenewalOfferTermsV1 {
                &self.terms
            }
            fn validate(&self) -> Result<(), mush_core::participant::ParticipantError> {
                RenewalOffer::validate(self)
            }
            fn digest(
                &self,
            ) -> Result<mush_core::DefinitionDigest, mush_core::participant::ParticipantError> {
                RenewalOffer::digest(self)
            }
        }
    };
}
pub(crate) use subject_records;

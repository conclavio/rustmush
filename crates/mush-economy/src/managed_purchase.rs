//! Closed original-personal purchase economic preparation.
//!
//! Request values are data, not posting authority. The registered Market
//! provider must resolve the exact acknowledged quote, owner proof and retained
//! account observation before requesting this leg. This module derives the
//! actual consent and escrow plan, validates real E4 state, and returns inert
//! after-images. Only the combined typed/E4/Roll installation may publish them.

use std::collections::BTreeMap;

use mush_core::participant::{OwnerPrepareContext, ParticipantError};
use mush_core::{
    ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId, ObjKind,
};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};

use crate::api::WorldAuth;
use crate::contract::managed::ManagedConsent;
use crate::contract::{
    Acceptance, ConsentSurface, Consideration, ContractRoles, ContractState, ContractTerms,
    EconomicContract, Expiry, PayoutShare, Share, check_offerable, consent_hash,
};
use crate::engine::{Delta, NoCustody, NoFacts, ValidateEnv};
use crate::ids::{AccountId, ContractId, HoldId, InstrumentId, TransactionId};
use crate::module::{EconomyStore, JOURNAL_LOG};
use crate::plan::{EconomyPlan, HoldOp, PlanResult, Prepared};
use crate::record::{
    AccountStatus, AssetLifecycle, EconomicCause, EconomicTransaction, EconomicVisibility,
    HoldPurpose, IdempotencyKey, InstrumentStatus,
};

pub(crate) const NAMESPACE: &str = "market.mind.purchase.v1";
const MAX_PAYOUTS: usize = 3;
pub(crate) mod subject;
use subject::{PurchaseSubject, SubjectTerminal};
pub(crate) fn closed_namespace(namespace: &str) -> bool {
    matches!(
        namespace,
        NAMESPACE
            | crate::managed_entitlement::NAMESPACE
            | crate::managed_house_purchase::NAMESPACE
    )
}

/// A private, exact plan carrier. No public authorization callback or caller
/// boolean can enter the managed branch of the pure engine.
pub(crate) struct ClosedPurchasePlan {
    plan: EconomyPlan,
    contract: ContractId,
    before_contract: Option<EconomicContract>,
    namespace: &'static str,
}
impl ClosedPurchasePlan {
    pub(crate) fn plan(&self) -> &EconomyPlan {
        &self.plan
    }
    pub(crate) fn check(
        &self,
        plan: &EconomyPlan,
        snapshot: &crate::EconomySnapshot,
    ) -> Result<(), crate::EconomyError> {
        if plan != &self.plan
            || plan.idempotency.namespace.as_str() != self.namespace
            || !matches!(&plan.cause, EconomicCause::Ruleset { namespace, .. } if namespace.as_str() == self.namespace)
            || snapshot.contract(self.contract) != self.before_contract.as_ref()
            || self
                .before_contract
                .as_ref()
                .is_some_and(|contract| contract.consent.managed.is_none())
        {
            return Err(crate::EconomyError::ManagedContract);
        }
        Ok(())
    }
}

fn refused() -> ParticipantError {
    ParticipantError::refused("No original personal purchase is available under these terms.")
}

/// One exact positive credit from the already derived quote split. The owner
/// is rechecked against the actual E4 account, not interpreted as a credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurchasePayout {
    pub account: AccountId,
    pub owner: PrincipalId,
    pub amount: crate::Amount,
}

/// Source-free input supplied only by the closed Market quote consumer. This
/// is deliberately not Deserialize and is not accepted as command parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginalPurchase {
    pub quote: CommitToken,
    pub quote_digest: DefinitionDigest,
    pub product_digest: DefinitionDigest,
    pub buyer: ObjId,
    pub seller: ObjId,
    pub charter: ObjId,
    pub title_revision: u64,
    pub payer: AccountId,
    pub instrument: InstrumentId,
    pub price: crate::Amount,
    pub payouts: Vec<PurchasePayout>,
    pub quoted_at_utc: i64,
    pub expires_at_utc: i64,
}

impl OriginalPurchase {
    pub(crate) fn validate_shape(&self) -> Result<(), ParticipantError> {
        if self.buyer.0 == 0
            || self.seller.0 == 0
            || self.buyer == self.seller
            || self.charter.0 == 0
            || self.title_revision == 0
            || self.price.is_negative()
            || self.price.is_zero()
            || self.payouts.is_empty()
            || self.payouts.len() > MAX_PAYOUTS
            || self
                .expires_at_utc
                .checked_sub(self.quoted_at_utc)
                .is_none_or(|duration| !(1..=600).contains(&duration))
        {
            return Err(refused());
        }
        let mut total = 0_i128;
        let mut previous = None;
        let mut seller = false;
        for payout in &self.payouts {
            if payout.account == self.payer
                || payout.amount.is_negative()
                || payout.amount.is_zero()
                || previous.is_some_and(|id| id >= payout.account)
            {
                return Err(refused());
            }
            seller |= payout.owner == PrincipalId::Object(self.seller);
            previous = Some(payout.account);
            total = total
                .checked_add(payout.amount.widen())
                .ok_or_else(refused)?;
        }
        if !seller || total != self.price.widen() {
            return Err(refused());
        }
        Ok(())
    }

    fn check_current(
        &self,
        state: &EconomyStore,
        context: &FundingContext<'_>,
    ) -> Result<(), ParticipantError> {
        self.validate_shape()?;
        let world = context.world;
        let action = context.action;

        if state.frozen
            || action.key().owner.as_str() != "market"
            || action.key().kind.as_str() != "action"
            || action.key().id.as_str() != "purchase_original_mind"
            || !matches!(action.version(), 1 | 2)
            || context.actor != self.buyer
            || context.principal != &(mush_core::PrincipalId::Object { id: self.buyer })
            || self.quote == context.commit
            || world.now() < self.quoted_at_utc
            || world.now() >= self.expires_at_utc
            || [self.buyer, self.seller].iter().any(|id| {
                world.get(*id).is_none_or(|object| {
                    object.kind != ObjKind::Player || object.flags.contains(mush_core::Flags::GOING)
                })
            })
            || world
                .get(self.charter)
                .is_none_or(|object| object.flags.contains(mush_core::Flags::GOING))
        {
            return Err(refused());
        }
        let snapshot = &state.snapshot;
        let title = snapshot.asset(self.charter).ok_or_else(refused)?;
        if title.title_owner != PrincipalId::Object(self.seller)
            || title.scope.is_some()
            || title.revision != self.title_revision
            || title.lifecycle != AssetLifecycle::Active
            || title.frozen
            || title.stolen.is_some()
        {
            return Err(refused());
        }
        let payer = snapshot.account(self.payer).ok_or_else(refused)?;
        if payer.owner != PrincipalId::Object(self.buyer)
            || payer.scope.is_some()
            || payer.instrument != self.instrument
            || payer.status != AccountStatus::Active
            || payer.approval.is_some()
        {
            return Err(refused());
        }
        if snapshot
            .instrument(&self.instrument)
            .is_none_or(|instrument| {
                instrument.status != InstrumentStatus::Active || !instrument.transferable
            })
        {
            return Err(refused());
        }
        for payout in &self.payouts {
            let account = snapshot.account(payout.account).ok_or_else(refused)?;
            if account.owner != payout.owner
                || account.instrument != self.instrument
                || account.status != AccountStatus::Active
            {
                return Err(refused());
            }
        }
        Ok(())
    }
}

/// Actual funding evidence stored in both paired operation records. Structural
/// decoding validates its clock but supplies no payment/admission authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "FundingReceiptWire")]
pub struct FundingReceipt {
    contract: ContractId,
    hold: HoldId,
    transaction: TransactionId,
    journal_seq: u64,
    consent_hash: [u8; 32],
    at_ic: ClockStamp,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FundingReceiptWire {
    contract: ContractId,
    hold: HoldId,
    transaction: TransactionId,
    journal_seq: u64,
    consent_hash: [u8; 32],
    at_ic: ClockStamp,
}
impl TryFrom<FundingReceiptWire> for FundingReceipt {
    type Error = &'static str;
    fn try_from(value: FundingReceiptWire) -> Result<Self, Self::Error> {
        if !matches!(value.at_ic.clock, ClockRef::WorldIc { generation } if generation != 0) {
            return Err("purchase receipt requires a versioned WorldIc stamp");
        }
        Ok(Self {
            contract: value.contract,
            hold: value.hold,
            transaction: value.transaction,
            journal_seq: value.journal_seq,
            consent_hash: value.consent_hash,
            at_ic: value.at_ic,
        })
    }
}
impl FundingReceipt {
    pub const fn contract(&self) -> ContractId {
        self.contract
    }
    pub const fn hold(&self) -> HoldId {
        self.hold
    }
    pub const fn transaction(&self) -> TransactionId {
        self.transaction
    }
    pub const fn journal_seq(&self) -> u64 {
        self.journal_seq
    }
    pub const fn consent_hash(&self) -> &[u8; 32] {
        &self.consent_hash
    }
    pub fn at_ic(&self) -> &ClockStamp {
        &self.at_ic
    }
}

/// Non-deserializable proof that the existing pure E4 engine produced these
/// exact after-images. It remains inert until the complete composed install.
#[derive(Clone, Debug)]
pub struct FundingOutput {
    operation: CommitToken,
    marker: ManagedConsent,
    receipt: FundingReceipt,
}
impl FundingOutput {
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub fn marker(&self) -> &ManagedConsent {
        &self.marker
    }
    pub fn receipt(&self) -> &FundingReceipt {
        &self.receipt
    }
}

pub(crate) struct FundingPreparation {
    pub output: FundingOutput,
    pub request: PurchaseSubject,
    pub plan: EconomyPlan,
    pub validated: Prepared,
    pub contract: EconomicContract,
    pub delta: Delta,
    pub transaction: EconomicTransaction,
    pub next_hold: u64,
    pub next_contract: u64,
}

fn contract(
    subject: &PurchaseSubject,
    marker: ManagedConsent,
    id: ContractId,
    hold: HoldId,
    now: i64,
) -> Result<EconomicContract, ParticipantError> {
    subject.validate_shape()?;
    let request = subject.common();
    if subject.marker(marker.operation(), marker.effect().clone())? != marker {
        return Err(refused());
    }
    let buyer = PrincipalId::Object(request.buyer);
    let remainder = request.payouts.first().ok_or_else(refused)?.account;
    let consent = ConsentSurface {
        fact_consent: None,
        managed: Some(marker),
        // The buyer consents to this escrow. Seller licence issuance consent is
        // the exact acknowledged listing and current Roll title/rights proof;
        // this builder never forges an E4 acceptance for the seller/payees.
        parties: vec![buyer.clone()],
        terms: ContractTerms {
            consideration: vec![Consideration {
                from: buyer.clone(),
                account: request.payer,
                amount: request.price,
            }],
            deliverables: Vec::new(),
            split: request
                .payouts
                .iter()
                .map(|payout| PayoutShare {
                    to: payout.account,
                    share: Share::Exact(payout.amount),
                })
                .collect(),
            remainder_to: remainder,
            on_expiry: Expiry::Refund,
            outcomes: Vec::new(),
        },
        conditions: Vec::new(),
        deadline: Some(crate::EconomicDeadline::real(request.expires_at_utc)),
        visibility: EconomicVisibility::Parties,
        scope: None,
        adjudicator: None,
        evidence: Vec::new(),
    };
    let hash = consent_hash(&buyer, &consent);
    Ok(EconomicContract {
        id,
        issuer: buyer.clone(),
        name: subject.contract_name().into(),
        consent,
        consent_hash: hash,
        acceptances: vec![Acceptance {
            party: buyer,
            at: now,
            consent_hash: hash,
        }],
        satisfied: Vec::new(),
        escrow: vec![hold],
        state: ContractState::ReadyToSettle,
        settle_attempts: 0,
        created_at: now,
        amendments: 0,
        generation_started_at: now,
        disputed_at: None,
        canon_published: None,
        roster_locked: true,
        deposits: Vec::new(),
        agreements: Vec::new(),
        pending: None,
        roles: ContractRoles::default(),
        revision: 1,
    })
}

/// Funding is a real one-hold E4 plan under the actual buyer's WorldAuth. A
/// future-ID hold is not published separately from its managed contract. The
/// complete owner participant must serialize/bind both plus the typed roots
/// before applying this preparation; this function never mutates live state.
pub(crate) fn prepare_funding(
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: impl Into<PurchaseSubject>,
) -> Result<FundingPreparation, ParticipantError> {
    prepare_funding_in(
        state,
        &FundingContext {
            world: context.world(),
            actor: context.invocation().actor(),
            principal: context.invocation().principal(),
            action: context.action(),
            commit: context.commit_token(),
        },
        request,
    )
}

pub(crate) struct FundingContext<'a> {
    pub(crate) world: &'a mush_core::World,
    pub(crate) actor: ObjId,
    pub(crate) principal: &'a mush_core::PrincipalId,
    pub(crate) action: &'a DefinitionRef,
    pub(crate) commit: CommitToken,
}

pub(crate) fn prepare_funding_in(
    state: &EconomyStore,
    context: &FundingContext<'_>,
    request: impl Into<PurchaseSubject>,
) -> Result<FundingPreparation, ParticipantError> {
    let subject = request.into();
    subject.check_current(state, context)?;
    subject.check_contract_capacity(state)?;
    let request = subject.common();
    let namespace = subject.namespace();
    let world = context.world;
    let operation = context.commit;
    let id = ContractId(state.next_contract);
    let hold = HoldId(state.next_hold);
    let next_contract = state.next_contract.checked_add(1).ok_or_else(refused)?;
    let next_hold = state.next_hold.checked_add(1).ok_or_else(refused)?;
    if state.snapshot.contract(id).is_some() || state.snapshot.hold(hold).is_some() {
        return Err(refused());
    }
    let marker = subject.marker(operation, context.action.clone())?;
    let contract = contract(&subject, marker.clone(), id, hold, world.now())?;
    check_offerable(&contract, &state.snapshot).map_err(|_| refused())?;
    let mut plan = EconomyPlan::new(
        IdempotencyKey::new(namespace, format!("{}/fund", operation.to_hex())),
        PrincipalId::Object(request.buyer),
        EconomicCause::Ruleset {
            namespace: namespace.into(),
            correlation: receipt::funding_correlation(
                operation,
                context.action,
                &subject,
                world.world_ic_now(),
            )?
            .into(),
        },
        Vec::new(),
    );
    plan.description = subject.funding_description().into();
    plan.visibility = contract.consent.visibility.clone();
    plan.hold_ops.push(HoldOp::Place {
        account: request.payer,
        amount: request.price,
        beneficiary: Some(crate::api::contract_principal()),
        purpose: HoldPurpose::Escrow { contract: id },
        expires: None,
    });
    let auth = WorldAuth { w: world };
    let gate = state.key_gate();
    let environment = ValidateEnv {
        auth: &auth,
        custody: &NoCustody,
        facts: &NoFacts,
        now: world.now(),
        ic_now: world.game_now_unix(),
        next_hold_id: hold.0,
        spent: BTreeMap::new(),
        keys: &gate,
    };
    let proof = ClosedPurchasePlan {
        plan: plan.clone(),
        contract: id,
        before_contract: None,
        namespace,
    };
    let validated = match crate::engine::validate_purchase(&proof, &state.snapshot, &environment) {
        PlanResult::Post(prepared) => *prepared,
        PlanResult::NeedsApproval(_) | PlanResult::AlreadyPosted(_) | PlanResult::Refused(_) => {
            return Err(refused());
        }
    };
    let transaction_id = TransactionId(state.next_transaction);
    let journal_seq = state.journal.next_seq(JOURNAL_LOG);
    let (delta, transaction) = crate::engine::apply_delta(
        &state.snapshot,
        &validated,
        transaction_id,
        journal_seq,
        world.now(),
    );
    if delta.holds.len() != 1
        || delta.holds.first().is_none_or(|actual| {
            actual.id != hold
                || actual.account != request.payer
                || actual.amount != request.price
                || actual.purpose != (HoldPurpose::Escrow { contract: id })
                || !actual.status.is_held()
        })
        || !delta.assets.is_empty()
        || delta.consumed_approval.is_some()
        || delta.reversed.is_some()
        || !transaction.custody_effects.is_empty()
    {
        return Err(refused());
    }
    let receipt = FundingReceipt {
        contract: id,
        hold,
        transaction: transaction_id,
        journal_seq,
        consent_hash: contract.consent_hash,
        at_ic: world.world_ic_now(),
    };
    Ok(FundingPreparation {
        output: FundingOutput {
            operation,
            marker,
            receipt,
        },
        request: subject,
        plan,
        validated,
        contract,
        delta,
        transaction,
        next_hold,
        next_contract,
    })
}

pub(crate) mod install;
mod receipt;
pub use install::{BoundPurchase, PurchasePatch};
pub use receipt::{
    PurchaseAuditError, PurchaseHistory, PurchaseReceiptAudit, PurchaseTerminalHistory,
};
mod terminal;
pub use terminal::{PurchaseOutcome, PurchaseTerminal, TerminalOutput};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod boot_tests;

//! Unregistered same-contract funding and terminal preparation. Only actual
//! owner context supplies actor/action/commit. Plain selectors grant no rights.
use super::{
    FUND_ACTION, GRANT_ACTION, NAMESPACE, REFUND_ACTION, action_matches,
    history::OfferRecord,
    install::OfferContext,
    receipt::{self, RenewalFundingReceiptV1, RenewalOutcome, RenewalTerminalReceiptV1},
    refused,
};
use crate::api::WorldAuth;
use crate::contract::{ContractState, EconomicContract};
use crate::engine::{AuthEval, Delta, NoCustody, NoFacts, ValidateEnv};
use crate::module::{EconomyStore, JOURNAL_LOG};
use crate::plan::{EconomyPlan, HoldOp, PlanResult, Prepared};
use crate::record::{Account, EconomicCause, EconomicTransaction, Hold, HoldEffectKind};
use crate::{Amount, ContractId, TransactionId};
use mush_core::CommitToken;
use mush_core::participant::{OwnerPrepareContext, ParticipantError};
use mush_org::PrincipalId;
use std::collections::BTreeMap;

/// Only acknowledged identity; no replacement offer, wallet or amount.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenewalMoneyRequest {
    pub operation: CommitToken,
    pub contract: ContractId,
    pub consent_hash: [u8; 32],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Transition {
    Fund,
    Grant,
    Refund,
}
impl Transition {
    fn action(self) -> &'static str {
        match self {
            Self::Fund => FUND_ACTION,
            Self::Grant => GRANT_ACTION,
            Self::Refund => REFUND_ACTION,
        }
    }
    pub(super) const fn sale(self) -> bool {
        !matches!(self, Self::Refund)
    }
}
/// Actual output, deliberately neither Clone nor Deserialize.
pub struct RenewalFundingOutput {
    operation: CommitToken,
    receipt: RenewalFundingReceiptV1,
}
impl RenewalFundingOutput {
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub fn receipt(&self) -> &RenewalFundingReceiptV1 {
        &self.receipt
    }
}
pub struct RenewalTerminalOutput {
    operation: CommitToken,
    receipt: RenewalTerminalReceiptV1,
}
impl RenewalTerminalOutput {
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub fn receipt(&self) -> &RenewalTerminalReceiptV1 {
        &self.receipt
    }
}
pub(super) enum MoneyOutput {
    Funded(RenewalFundingOutput),
    Terminal(RenewalTerminalOutput),
}
/// Fixed exact plan and actual before image, never caller-constructible.
pub(crate) struct ClosedRenewalPlan {
    plan: EconomyPlan,
    before: EconomicContract,
}
impl ClosedRenewalPlan {
    pub(crate) fn plan(&self) -> &EconomyPlan {
        &self.plan
    }
    pub(crate) fn check(
        &self,
        plan: &EconomyPlan,
        snapshot: &crate::EconomySnapshot,
    ) -> Result<(), crate::EconomyError> {
        if plan != &self.plan
            || plan.idempotency.namespace.as_str() != NAMESPACE
            || !matches!(&plan.cause,EconomicCause::Ruleset{namespace,..} if namespace.as_str()==NAMESPACE)
            || snapshot.contract(self.before.id) != Some(&self.before)
            || self
                .before
                .consent
                .managed
                .as_ref()
                .and_then(|m| m.renewal_offer_marker())
                .is_none()
        {
            return Err(crate::EconomyError::ManagedContract);
        }
        Ok(())
    }
}
pub(super) struct MoneyPreparation {
    pub output: MoneyOutput,
    pub request: RenewalMoneyRequest,
    pub transition: Transition,
    pub origin: OfferRecord,
    pub plan: EconomyPlan,
    pub validated: Prepared,
    pub before_contract: EconomicContract,
    pub contract: EconomicContract,
    pub delta: Delta,
    pub transaction: EconomicTransaction,
    pub next_hold: u64,
}

pub(super) fn prepare(
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: RenewalMoneyRequest,
    transition: Transition,
) -> Result<Box<MoneyPreparation>, ParticipantError> {
    prepare_in(state, &OfferContext::from(context), request, transition)
}
pub(super) fn prepare_in(
    state: &EconomyStore,
    context: &OfferContext<'_>,
    request: RenewalMoneyRequest,
    transition: Transition,
) -> Result<Box<MoneyPreparation>, ParticipantError> {
    let origin = state
        .renewal_offers
        .get(request.operation)
        .ok_or_else(refused)?;
    origin.validate()?;
    let terms = &origin.offer.terms;
    let now = context.world.world_ic_now();
    if state.frozen
        || origin.closure.is_some()
        || origin.contract != request.contract
        || origin.consent_hash != request.consent_hash
        || !action_matches(context.action, transition.action())
        || [
            origin.operation,
            origin.offer.basis.predecessor_operation,
            origin.offer.basis.prior_quote,
        ]
        .contains(&context.commit)
        || context.actor != terms.buyer
        || context.principal != &(mush_core::PrincipalId::Object { id: terms.buyer })
        || !super::subject::player(context.world, terms.buyer)
        || !matches!(now.clock,mush_core::ClockRef::WorldIc{generation} if generation!=0)
        || context.world.now() < origin.created_at_utc
    {
        return Err(refused());
    }
    state
        .fold
        .renewal_receipts
        .require_complete()
        .map_err(|_| refused())?;
    let before = state
        .snapshot
        .contract(request.contract)
        .ok_or_else(refused)?
        .clone();
    if transition.sale() {
        terms.check_current(state, context.world)?;
        if now.clock != origin.created_at_ic.clock
            || now.at < origin.created_at_ic.at
            || now.at < terms.not_before.at
            || now.at >= terms.expires_at.at
        {
            return Err(refused());
        }
    }
    if transition == Transition::Fund {
        prepare_funding(state, context, request, transition, origin, before, now)
    } else {
        prepare_terminal(state, context, request, transition, origin, before, now)
    }
}

#[inline(never)]
fn prepare_funding(
    state: &EconomyStore,
    context: &OfferContext<'_>,
    request: RenewalMoneyRequest,
    transition: Transition,
    origin: &OfferRecord,
    before: EconomicContract,
    now: mush_core::ClockStamp,
) -> Result<Box<MoneyPreparation>, ParticipantError> {
    origin.verify_documents(&state.snapshot)?;
    if state
        .fold
        .renewal_receipts
        .receipts()
        .map_err(|_| refused())?
        .any(|h| h.operation() == request.operation)
    {
        return Err(refused());
    }
    let plan = receipt::funding_plan(origin, context.commit, context.action, &now)?;
    let proof = ClosedRenewalPlan {
        plan: plan.clone(),
        before: before.clone(),
    };
    let auth = WorldAuth { w: context.world };
    let keys = state.key_gate();
    let env = ValidateEnv {
        auth: &auth,
        custody: &NoCustody,
        facts: &NoFacts,
        now: context.world.now(),
        ic_now: context.world.game_now_unix(),
        next_hold_id: state.next_hold,
        spent: BTreeMap::new(),
        keys: &keys,
    };
    let validated = posted(crate::engine::validate_renewal(
        &proof,
        &state.snapshot,
        &env,
    ))?;
    let (delta, transaction) = apply(state, context, &validated)?;
    let [effect] = transaction.hold_effects.as_slice() else {
        return Err(refused());
    };
    if effect.kind != HoldEffectKind::Placed || effect.hold.id.0 != state.next_hold {
        return Err(refused());
    }
    let next_hold = state.next_hold.checked_add(1).ok_or_else(refused)?;
    let prepared_history = state.fold.renewal_receipts.prepare(&transaction)?;
    let history = prepared_history.row();
    if history.origin() != origin {
        return Err(refused());
    }
    let after = history.contract().map_err(|_| refused())?;
    // Every accepted fund must leave complete terminal evidence room.
    history.require_terminal_room().map_err(|_| refused())?;
    let output = MoneyOutput::Funded(RenewalFundingOutput {
        operation: origin.operation,
        receipt: history.funding().clone(),
    });
    Ok(Box::new(MoneyPreparation {
        output,
        request,
        transition,
        origin: origin.clone(),
        plan,
        validated,
        before_contract: before,
        contract: after,
        delta,
        transaction,
        next_hold,
    }))
}

#[inline(never)]
fn prepare_terminal(
    state: &EconomyStore,
    context: &OfferContext<'_>,
    request: RenewalMoneyRequest,
    transition: Transition,
    origin: &OfferRecord,
    before: EconomicContract,
    now: mush_core::ClockStamp,
) -> Result<Box<MoneyPreparation>, ParticipantError> {
    let terms = &origin.offer.terms;
    let history = state
        .fold
        .renewal_receipts
        .get(request.operation)
        .map_err(|_| refused())?;
    if history.origin() != origin
        || history.terminal().is_some()
        || context.commit == history.funding().commit()
        || context.world.now() < history.funding().posted_at_utc()
    {
        return Err(refused());
    }
    let hold = state
        .snapshot
        .hold(history.funding().hold())
        .ok_or_else(refused)?;
    history
        .verify_documents(&before, hold)
        .map_err(|_| refused())?;
    let outcome = if transition == Transition::Grant {
        RenewalOutcome::Granted
    } else {
        RenewalOutcome::Refunded
    };
    if transition == Transition::Grant
        && (now.clock != history.funding().at_ic().clock || now.at < history.funding().at_ic().at)
    {
        return Err(refused());
    }
    let auth = TerminalAuth {
        buyer: PrincipalId::Object(terms.buyer),
        hold: hold.clone(),
        payouts: if transition.sale() {
            terms.payouts()?
        } else {
            Vec::new()
        },
        outcome,
    };
    let keys = state.key_gate();
    let env = ValidateEnv {
        auth: &auth,
        custody: &NoCustody,
        facts: &NoFacts,
        now: context.world.now(),
        ic_now: context.world.game_now_unix(),
        next_hold_id: state.next_hold,
        spent: BTreeMap::new(),
        keys: &keys,
    };
    let plan = receipt::terminal_plan(history, context.commit, context.action, &now, outcome)?;
    let mut native = if transition == Transition::Grant {
        crate::contract::settlement_plan(&before, &state.snapshot, &env)
    } else {
        let mut c = before.clone();
        c.state = ContractState::Cancelled;
        crate::contract::refund_plan(&c, &state.snapshot)
    }
    .map_err(|_| refused())?;
    native.idempotency = plan.idempotency.clone();
    native.cause = plan.cause.clone();
    if native != plan {
        return Err(refused());
    }
    let proof = ClosedRenewalPlan {
        plan: plan.clone(),
        before: before.clone(),
    };
    let validated = posted(crate::engine::validate_renewal(
        &proof,
        &state.snapshot,
        &env,
    ))?;
    let (delta, transaction) = apply(state, context, &validated)?;
    let prepared_history = state.fold.renewal_receipts.prepare(&transaction)?;
    let history = prepared_history.row();
    let after = history.contract().map_err(|_| refused())?;
    let receipt = history.terminal().ok_or_else(refused)?.clone();
    let output = MoneyOutput::Terminal(RenewalTerminalOutput {
        operation: origin.operation,
        receipt,
    });
    Ok(Box::new(MoneyPreparation {
        output,
        request,
        transition,
        origin: origin.clone(),
        plan,
        validated,
        before_contract: before,
        contract: after,
        delta,
        transaction,
        next_hold: state.next_hold,
    }))
}
fn posted(result: PlanResult) -> Result<Prepared, ParticipantError> {
    match result {
        PlanResult::Post(p) => Ok(*p),
        PlanResult::NeedsApproval(_) | PlanResult::AlreadyPosted(_) | PlanResult::Refused(_) => {
            Err(refused())
        }
    }
}
fn apply(
    state: &EconomyStore,
    context: &OfferContext<'_>,
    validated: &Prepared,
) -> Result<(Delta, EconomicTransaction), ParticipantError> {
    let (delta, tx) = crate::engine::apply_delta(
        &state.snapshot,
        validated,
        TransactionId(state.next_transaction),
        state.journal.next_seq(JOURNAL_LOG),
        context.world.now(),
    );
    if tx.hold_effects.len() != 1
        || !delta.assets.is_empty()
        || !tx.custody_effects.is_empty()
        || delta.consumed_approval.is_some()
        || delta.reversed.is_some()
    {
        return Err(refused());
    }
    Ok((delta, tx))
}
struct TerminalAuth {
    buyer: PrincipalId,
    hold: Hold,
    payouts: Vec<crate::managed_purchase::PurchasePayout>,
    outcome: RenewalOutcome,
}
impl AuthEval for TerminalAuth {
    fn authorize(&self, actor: &PrincipalId, account: &Account, delta: Amount) -> bool {
        actor == &self.buyer
            && self.outcome == RenewalOutcome::Granted
            && !delta.is_negative()
            && !delta.is_zero()
            && self
                .payouts
                .iter()
                .any(|p| p.account == account.id && p.owner == account.owner && p.amount == delta)
    }
    fn authorize_hold(&self, actor: &PrincipalId, hold: &Hold, op: &HoldOp) -> bool {
        if actor != &self.buyer || hold != &self.hold {
            return false;
        }
        match (self.outcome, op) {
            (RenewalOutcome::Granted, HoldOp::Capture { hold: id, amount }) => {
                *id == hold.id && *amount == hold.amount
            }
            (RenewalOutcome::Refunded, HoldOp::Release { hold: id }) => *id == hold.id,
            _ => false,
        }
    }
}

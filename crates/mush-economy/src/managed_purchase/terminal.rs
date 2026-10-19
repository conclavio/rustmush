//! Exact managed escrow capture/release preparation. The real Market provider
//! owns grant eligibility; this economic leg cannot create an entitlement.

use super::*;
use crate::engine::AuthEval;
use crate::record::{Account, Hold, HoldEffectKind};

/// Frozen acknowledged facts, not request text. The provider loads these from
/// the two permanent operation rows and checks their complete subject equality.
#[derive(Clone, Debug)]
pub struct PurchaseTerminal {
    pub operation: CommitToken,
    pub purchase: OriginalPurchase,
    pub funding: FundingReceipt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PurchaseOutcome {
    Granted,
    Refunded,
}

/// Constructed only from the actual validated terminal journal post-image.
#[derive(Clone, Debug)]
pub struct TerminalOutput {
    operation: CommitToken,
    commit: CommitToken,
    outcome: PurchaseOutcome,
    contract: ContractId,
    hold: HoldId,
    transaction: TransactionId,
    journal_seq: u64,
    issued_at_ic: Option<ClockStamp>,
}
impl TerminalOutput {
    pub const fn operation(&self) -> CommitToken {
        self.operation
    }
    pub const fn commit(&self) -> CommitToken {
        self.commit
    }
    pub const fn outcome(&self) -> PurchaseOutcome {
        self.outcome
    }
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
    pub fn issued_at_ic(&self) -> Option<&ClockStamp> {
        self.issued_at_ic.as_ref()
    }
}

/// Only this module constructs the authorization. Amounts and the sole hold
/// come from the actual live consent/escrow after existing E4 plan builders ran.
struct TerminalAuth {
    buyer: PrincipalId,
    hold: Hold,
    payouts: Vec<PurchasePayout>,
    outcome: PurchaseOutcome,
}
impl AuthEval for TerminalAuth {
    fn authorize(&self, actor: &PrincipalId, account: &Account, delta: crate::Amount) -> bool {
        actor == &self.buyer
            && self.outcome == PurchaseOutcome::Granted
            && !delta.is_negative()
            && !delta.is_zero()
            && self.payouts.iter().any(|payout| {
                payout.account == account.id
                    && payout.owner == account.owner
                    && payout.amount == delta
            })
    }
    fn authorize_hold(&self, actor: &PrincipalId, hold: &Hold, op: &HoldOp) -> bool {
        if actor != &self.buyer || hold != &self.hold {
            return false;
        }
        match (self.outcome, op) {
            (PurchaseOutcome::Granted, HoldOp::Capture { hold: id, amount }) => {
                *id == hold.id && *amount == hold.amount
            }
            (PurchaseOutcome::Refunded, HoldOp::Release { hold: id }) => *id == hold.id,
            _ => false,
        }
    }
}

pub(crate) struct TerminalPreparation {
    pub output: TerminalOutput,
    pub request: SubjectTerminal,
    pub outcome: PurchaseOutcome,
    pub plan: EconomyPlan,
    pub validated: Prepared,
    pub before_contract: EconomicContract,
    pub contract: EconomicContract,
    pub delta: Delta,
    pub transaction: EconomicTransaction,
}

pub(super) fn prepare(
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: impl Into<SubjectTerminal>,
    outcome: PurchaseOutcome,
) -> Result<TerminalPreparation, ParticipantError> {
    prepare_in(
        state,
        &FundingContext {
            world: context.world(),
            actor: context.invocation().actor(),
            principal: context.invocation().principal(),
            action: context.action(),
            commit: context.commit_token(),
        },
        request,
        outcome,
    )
}

pub(super) fn prepare_in(
    state: &EconomyStore,
    context: &FundingContext<'_>,
    request: impl Into<SubjectTerminal>,
    outcome: PurchaseOutcome,
) -> Result<TerminalPreparation, ParticipantError> {
    let request = request.into();
    let subject = &request.purchase;
    subject.validate_shape()?;
    let purchase = subject.common();
    let namespace = subject.namespace();
    let transition = match outcome {
        PurchaseOutcome::Granted => "grant",
        PurchaseOutcome::Refunded => "refund",
    };
    if state.frozen
        || !subject.matches_action(context.action, transition)
        || context.commit == request.operation
        || (context.actor != purchase.buyer
            && (outcome != PurchaseOutcome::Refunded || context.actor != mush_core::GOD))
        || context.principal != &(mush_core::PrincipalId::Object { id: context.actor })
        || context.world.get(context.actor).is_none_or(|object| {
            object.kind != ObjKind::Player || object.flags.contains(mush_core::Flags::GOING)
        })
    {
        return Err(refused());
    }
    let actual = state
        .snapshot
        .contract(request.funding.contract)
        .ok_or_else(refused)?;
    let marker = actual.consent.managed.as_ref().ok_or_else(refused)?;
    if marker.operation() != request.operation
        || marker.quote() != purchase.quote
        || marker.quote_digest() != purchase.quote_digest
        || marker.product_digest() != purchase.product_digest
        || actual.consent_hash != request.funding.consent_hash
    {
        return Err(refused());
    }
    let expected = contract(
        subject,
        marker.clone(),
        actual.id,
        request.funding.hold,
        actual.created_at,
    )?;
    if actual != &expected {
        return Err(refused());
    }
    let hold = state
        .snapshot
        .hold(request.funding.hold)
        .ok_or_else(refused)?;
    if hold.account != purchase.payer
        || hold.amount != purchase.price
        || !hold.status.is_held()
        || hold.purpose
            != (HoldPurpose::Escrow {
                contract: actual.id,
            })
        || hold.placed_by != PrincipalId::Object(purchase.buyer)
        || hold.beneficiary != Some(crate::api::contract_principal())
        || hold.expires.is_some()
        || hold.holder
            != IdempotencyKey::new(namespace, format!("{}/fund", request.operation.to_hex()))
        || hold.revision != 1
        || hold.last_journal_seq != request.funding.journal_seq
    {
        return Err(refused());
    }
    // Historical proof survives hot-tail eviction and is opened only by actual
    // journal/checkpoint replay. Current contracts/holds cannot invent a debit.
    let history = state
        .fold
        .purchase_receipts
        .get(request.operation)
        .map_err(|_| refused())?;
    if history.terminal().is_some() || !history.matches_subject(subject, &request.funding) {
        return Err(refused());
    }
    history
        .verify_documents(actual, hold)
        .map_err(|_| refused())?;
    if outcome == PurchaseOutcome::Granted {
        if request.funding.at_ic.clock != context.world.world_ic_now().clock {
            return Err(refused());
        }
        subject.check_current(
            state,
            &FundingContext {
                world: context.world,
                actor: purchase.buyer,
                principal: &mush_core::PrincipalId::Object { id: purchase.buyer },
                action: marker.effect(),
                commit: request.operation,
            },
        )?;
    }
    let auth = TerminalAuth {
        buyer: PrincipalId::Object(purchase.buyer),
        hold: hold.clone(),
        payouts: purchase.payouts.clone(),
        outcome,
    };
    let gate = state.key_gate();
    let environment = ValidateEnv {
        auth: &auth,
        custody: &NoCustody,
        facts: &NoFacts,
        now: context.world.now(),
        ic_now: context.world.game_now_unix(),
        next_hold_id: state.next_hold,
        spent: BTreeMap::new(),
        keys: &gate,
    };
    let mut plan = match outcome {
        PurchaseOutcome::Granted => {
            crate::contract::settlement_plan(actual, &state.snapshot, &environment)
        }
        PurchaseOutcome::Refunded => {
            let mut cancelled = actual.clone();
            cancelled.state = ContractState::Cancelled;
            crate::contract::refund_plan(&cancelled, &state.snapshot)
        }
    }
    .map_err(|_| refused())?;
    let transition = match outcome {
        PurchaseOutcome::Granted => "grant",
        PurchaseOutcome::Refunded => "refund",
    };
    plan.idempotency = IdempotencyKey::new(
        namespace,
        format!("{}/{transition}", request.operation.to_hex()),
    );
    plan.cause = EconomicCause::Ruleset {
        namespace: namespace.into(),
        correlation: receipt::terminal_correlation(
            request.operation,
            context.commit,
            outcome,
            context.world.world_ic_now(),
        )?
        .into(),
    };
    let proof = ClosedPurchasePlan {
        plan: plan.clone(),
        contract: actual.id,
        before_contract: Some(actual.clone()),
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
        context.world.now(),
    );
    let kind = match outcome {
        PurchaseOutcome::Granted => HoldEffectKind::Captured,
        PurchaseOutcome::Refunded => HoldEffectKind::Released,
    };
    if transaction.hold_effects.len() != 1
        || transaction
            .hold_effects
            .first()
            .is_none_or(|effect| effect.kind != kind || effect.hold.id != hold.id)
        || !delta.assets.is_empty()
        || !transaction.custody_effects.is_empty()
        || delta.consumed_approval.is_some()
        || delta.reversed.is_some()
        || (outcome == PurchaseOutcome::Refunded && !transaction.lines.is_empty())
    {
        return Err(refused());
    }
    let mut contract = actual.clone();
    contract.state = match outcome {
        PurchaseOutcome::Granted => ContractState::Settled {
            transaction: transaction_id,
        },
        PurchaseOutcome::Refunded => ContractState::Cancelled,
    };
    contract.revision = contract.revision.checked_add(1).ok_or_else(refused)?;
    let output = TerminalOutput {
        operation: request.operation,
        commit: context.commit,
        outcome,
        contract: actual.id,
        hold: hold.id,
        transaction: transaction_id,
        journal_seq,
        issued_at_ic: (outcome == PurchaseOutcome::Granted).then(|| context.world.world_ic_now()),
    };
    Ok(TerminalPreparation {
        output,
        request,
        outcome,
        plan,
        validated,
        before_contract: actual.clone(),
        contract,
        delta,
        transaction,
    })
}

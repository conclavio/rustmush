//! Live native fee hold/capture/release under the same actual E4 owner.
use super::{CurrentFacts, FundingRequest, Leg, Policy, Subject, refused};
use super::{
    plans::{self, FundingIntent},
    receipt::{FundingReceipt, PaymentHistory, TerminalReceipt},
};
use crate::api::WorldAuth;
use crate::engine::{AuthEval, Delta, NoCustody, NoFacts, ValidateEnv};
use crate::plan::{EconomyPlan, HoldOp, PlanResult, Prepared};
use crate::{Account, Amount, EconomySnapshot, EconomyStore, Hold, TransactionId};
use mush_core::participant::{OwnerPreparationBinding, OwnerPrepareContext, ParticipantError};
use mush_org::PrincipalId;
use std::collections::BTreeMap;
use std::marker::PhantomData;

pub enum Request<M: Leg> {
    Fund(Box<FundingRequest<M>>),
    Install(M::Installation),
    Cancel(M::Cancellation),
}
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Transition {
    Fund,
    Install,
    Cancel,
}
impl Transition {
    pub fn action<M: Leg>(self) -> &'static str {
        match self {
            Self::Fund => M::ACCEPT_ACTION,
            Self::Install => M::INSTALL_ACTION,
            Self::Cancel => M::CANCEL_ACTION,
        }
    }
}
/// Opaque actual owner outputs. Raw decoded histories cannot create them.
pub struct FundingOutput<M: Leg> {
    pub(super) history: Box<PaymentHistory<M>>,
}
impl<M: Leg> FundingOutput<M> {
    pub fn subject(&self) -> &M::Subject {
        self.history.subject()
    }
    pub fn receipt(&self) -> &FundingReceipt {
        self.history.funding()
    }
}
pub struct TerminalOutput<M: Leg> {
    pub(super) history: Box<PaymentHistory<M>>,
    receipt: Box<TerminalReceipt<M>>,
}
impl<M: Leg> TerminalOutput<M> {
    pub fn subject(&self) -> &M::Subject {
        self.history.subject()
    }
    pub fn receipt(&self) -> &TerminalReceipt<M> {
        &self.receipt
    }
}
pub enum Output<M: Leg> {
    Funded(FundingOutput<M>),
    Terminal(TerminalOutput<M>),
}
pub(super) struct MoneyPreparation<M: Leg> {
    pub output: Option<Output<M>>,
    pub history: PaymentHistory<M>,
    pub transition: Transition,
    pub binding: OwnerPreparationBinding,
    pub before_history: Option<PaymentHistory<M>>,
    pub live: Option<CurrentFacts<M>>,
    pub plan: EconomyPlan,
    pub validated: Prepared,
    pub delta: Delta,
    pub transaction: crate::EconomicTransaction,
    pub next_hold: u64,
}
/// Exact owner-only permission to the native engine. It never authorizes a
/// replacement plan, unrelated hold or title that already exists.
pub struct ClosedPlan<M: Leg> {
    plan: EconomyPlan,
    before_hold: Option<Hold>,
    leg: PhantomData<M>,
}
impl<M: Leg> ClosedPlan<M> {
    pub(crate) fn plan(&self) -> &EconomyPlan {
        &self.plan
    }
    pub(crate) fn check(
        &self,
        plan: &EconomyPlan,
        snapshot: &EconomySnapshot,
    ) -> Result<(), crate::EconomyError> {
        if plan != &self.plan
            || plan.idempotency.namespace != M::NAMESPACE
            || self
                .before_hold
                .as_ref()
                .is_some_and(|h| snapshot.hold(h.id) != Some(h))
            || !plan.asset_ops.is_empty()
            || !plan.custody_ops.is_empty()
        {
            return Err(crate::EconomyError::ManagedContract);
        }
        Ok(())
    }
}
fn identity<M: Leg>(
    config: &M::Config,
    ctx: &OwnerPrepareContext<'_>,
    phase: Transition,
    requester: mush_core::ObjId,
) -> Result<(), ParticipantError> {
    let inv = ctx.invocation();
    if ctx.world().get(requester).is_none_or(|o| {
        o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
    }) {
        return Err(refused::<M>());
    }
    if ctx.action()
        != &M::action_ref(M::policy(config), phase.action::<M>()).map_err(|_| refused::<M>())?
        || inv.actor() != requester
        || inv.principal() != &(mush_core::PrincipalId::Object { id: requester })
        || inv.subject() != &mush_core::PerceivedRef::SelfActor
        || !inv.targets().is_empty()
        || inv.idempotency().scope().owner.as_str() != "industry"
    {
        return Err(refused::<M>());
    }
    Ok(())
}
pub(super) fn prepare<M: Leg>(
    config: &M::Config,
    state: &EconomyStore,
    ctx: &OwnerPrepareContext<'_>,
    request: Request<M>,
) -> Result<Box<MoneyPreparation<M>>, ParticipantError> {
    M::receipts(&state.fold)
        .require_complete()
        .map_err(|_| refused::<M>())?;
    let (intent, transition, before_history, live) = match request {
        Request::Fund(request) => {
            identity::<M>(config, ctx, Transition::Fund, request.subject.requester())?;
            let (intent, live) = M::funding(config, state, ctx.world(), &request.subject, true)
                .map_err(|_| refused::<M>())?;
            (intent, Transition::Fund, None, Some(live))
        }
        Request::Install(request) => {
            let acceptance = M::installation_acceptance(&request);
            let history = M::receipts(&state.fold)
                .get(acceptance)
                .map_err(|_| refused::<M>())?
                .clone();
            identity::<M>(
                config,
                ctx,
                Transition::Install,
                history.subject().requester(),
            )?;
            if history.terminal().is_some() {
                return Err(refused::<M>());
            }
            history
                .verify_hold(state.snapshot())
                .map_err(|_| refused::<M>())?;
            let live = M::installation(config, state, ctx.world(), &history.intent, acceptance)
                .map_err(|_| refused::<M>())?;
            (
                history.intent.as_ref().clone(),
                Transition::Install,
                Some(history),
                Some(live),
            )
        }
        Request::Cancel(request) => {
            let history = M::receipts(&state.fold)
                .get(M::cancellation_acceptance(&request))
                .map_err(|_| refused::<M>())?
                .clone();
            identity::<M>(
                config,
                ctx,
                Transition::Cancel,
                history.subject().requester(),
            )?;
            M::cleanup_ready(config, ctx.world()).map_err(|_| refused::<M>())?;
            if history.terminal().is_some() {
                return Err(refused::<M>());
            }
            history
                .verify_hold(state.snapshot())
                .map_err(|_| refused::<M>())?;
            (
                history.intent.as_ref().clone(),
                Transition::Cancel,
                Some(history),
                None,
            )
        }
    };
    let at = ctx.world().world_ic_now();
    let plan = if let Some(old) = &before_history {
        plans::terminal(
            &intent,
            old.funding.commit,
            old.funding.hold.id,
            ctx.commit_token(),
            ctx.action(),
            &at,
            live.as_ref().and_then(|v| v.order.as_ref()).map(M::part),
        )
    } else {
        plans::funding(&intent, ctx.commit_token(), ctx.action())
    }
    .map_err(|_| refused::<M>())?;
    let before_hold = before_history.as_ref().map(|h| h.funding.hold.clone());
    let proof = ClosedPlan {
        plan: plan.clone(),
        before_hold: before_hold.clone(),
        leg: PhantomData,
    };
    let validated = validate(state, ctx, &intent, transition, &proof)?;
    let (delta, transaction) = crate::engine::apply_delta(
        state.snapshot(),
        &validated,
        TransactionId(state.next_transaction),
        state.journal.next_seq(crate::module::JOURNAL_LOG),
        ctx.world().now(),
    );
    let history = M::receipts(&state.fold)
        .prepare(&transaction)
        .map_err(|_| refused::<M>())?
        .row()
        .clone();
    let next_hold = if transition == Transition::Fund {
        state.next_hold.checked_add(1).ok_or_else(refused::<M>)?
    } else {
        state.next_hold
    };
    if transaction.hold_effects.len() != 1
        || transaction.hold_effects.first().is_none_or(|e| {
            e.hold.id.0
                != if transition == Transition::Fund {
                    state.next_hold
                } else {
                    history.funding.hold.id.0
                }
        })
        || delta.consumed_approval.is_some()
        || delta.reversed.is_some()
        || !transaction.custody_effects.is_empty()
    {
        return Err(refused::<M>());
    }
    let output = if transition == Transition::Fund {
        Output::Funded(FundingOutput {
            history: Box::new(history.clone()),
        })
    } else {
        Output::Terminal(TerminalOutput {
            receipt: Box::new(history.terminal().cloned().ok_or_else(refused::<M>)?),
            history: Box::new(history.clone()),
        })
    };
    Ok(Box::new(MoneyPreparation {
        output: Some(output),
        history,
        transition,
        binding: ctx.preparation_binding(),
        before_history,
        live,
        plan,
        validated,
        delta,
        transaction,
        next_hold,
    }))
}
fn validate<M: Leg>(
    state: &EconomyStore,
    ctx: &OwnerPrepareContext<'_>,
    intent: &FundingIntent<M>,
    phase: Transition,
    proof: &ClosedPlan<M>,
) -> Result<Prepared, ParticipantError> {
    let auth = Auth {
        world: WorldAuth { w: ctx.world() },
        intent,
        phase,
        hold: proof.before_hold.as_ref(),
    };
    let keys = state.key_gate();
    let payer = intent.subject.payer();
    let mut spent = BTreeMap::new();
    if let Some(window) = state
        .snapshot()
        .account(payer)
        .and_then(|a| a.approval.as_ref())
        .and_then(|a| a.window.as_ref())
    {
        spent.insert(
            payer,
            state.spent_in_window(payer, ctx.world().now(), window.period_secs),
        );
    }
    let env = ValidateEnv {
        auth: &auth,
        custody: &NoCustody,
        facts: &NoFacts,
        now: ctx.world().now(),
        ic_now: ctx.world().game_now_unix(),
        next_hold_id: state.next_hold,
        spent,
        keys: &keys,
    };
    match M::validate(proof, state.snapshot(), &env) {
        PlanResult::Post(p) => Ok(*p),
        PlanResult::NeedsApproval(_) | PlanResult::AlreadyPosted(_) | PlanResult::Refused(_) => {
            Err(refused::<M>())
        }
    }
}
pub(super) fn recheck<M: Leg>(
    config: &M::Config,
    state: &EconomyStore,
    ctx: &OwnerPrepareContext<'_>,
    prepared: &MoneyPreparation<M>,
) -> Result<(), ParticipantError> {
    if !ctx.matches_preparation(&prepared.binding) {
        return Err(refused::<M>());
    }
    identity::<M>(
        config,
        ctx,
        prepared.transition,
        prepared.history.subject().requester(),
    )?;
    let intent = prepared.history.intent.as_ref();
    let current = match prepared.transition {
        Transition::Fund => {
            // Historical accepted_at remains fixed. Recheck the exact old offer
            // against monotonic current time, not a replacement accepted intent.
            let (actual, facts) = M::funding(config, state, ctx.world(), &intent.subject, false)
                .map_err(|_| refused::<M>())?;
            if &actual != intent {
                return Err(refused::<M>());
            }
            facts
        }
        Transition::Install => M::installation(
            config,
            state,
            ctx.world(),
            intent,
            prepared.history.funding.commit,
        )
        .map_err(|_| refused::<M>())?,
        Transition::Cancel => {
            M::cleanup_ready(config, ctx.world()).map_err(|_| refused::<M>())?;
            return Ok(());
        }
    };
    let old = prepared.live.as_ref().ok_or_else(refused::<M>)?;
    if current.accounts != old.accounts
        || current.instruments != old.instruments
        || current.titles != old.titles
        || current.order != old.order
        || current.supplier != old.supplier
        || current.at.clock != old.at.clock
        || current.at.at < old.at.at
        || current.utc < old.utc
    {
        return Err(refused::<M>());
    }
    let proof = ClosedPlan {
        plan: prepared.plan.clone(),
        before_hold: prepared
            .before_history
            .as_ref()
            .map(|h| h.funding.hold.clone()),
        leg: PhantomData,
    };
    if validate(state, ctx, intent, prepared.transition, &proof)? != prepared.validated {
        return Err(refused::<M>());
    }
    Ok(())
}
struct Auth<'a, M: Leg> {
    world: WorldAuth<'a>,
    intent: &'a FundingIntent<M>,
    phase: Transition,
    hold: Option<&'a Hold>,
}
impl<M: Leg> AuthEval for Auth<'_, M> {
    fn authorize(&self, actor: &PrincipalId, account: &Account, delta: Amount) -> bool {
        if actor != &PrincipalId::System(self.intent.policy.issuer().clone()) {
            return false;
        }
        let subject = &self.intent.subject;
        if account.id == subject.payer() {
            return delta.checked_neg() == self.intent.price().ok()
                && account.owner == PrincipalId::Object(subject.requester())
                && (self.phase == Transition::Install
                    || self.world.authorize(
                        &PrincipalId::Object(subject.requester()),
                        account,
                        delta,
                    ));
        }
        // The provider is credited the leg's fee ([`Leg::price`]: the bay's quote, the core
        // recipe's own fee, never a fixed fitting fee), at Install only.
        self.phase == Transition::Install
            && self
                .intent
                .provider_account()
                .is_ok_and(|provider| account.id == provider)
            && self.intent.price().is_ok_and(|price| delta == price)
    }
    fn authorize_hold(&self, actor: &PrincipalId, hold: &Hold, op: &HoldOp) -> bool {
        if actor != &PrincipalId::System(self.intent.policy.issuer().clone())
            || Some(hold) != self.hold
        {
            return false;
        }
        match (self.phase, op) {
            (Transition::Install, HoldOp::Capture { hold: id, amount }) => {
                *id == hold.id && self.intent.price().is_ok_and(|price| *amount == price)
            }
            (Transition::Cancel, HoldOp::Release { hold: id }) => *id == hold.id,
            _ => false,
        }
    }
}

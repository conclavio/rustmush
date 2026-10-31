//! Live native bay fee hold/capture/release under the same actual E4 owner.
use super::{
    ACCEPT_ACTION, BayCancellationRequest, BayFundingReceipt, BayFundingRequest,
    BayInstallationRequest, BayRefitConfig, BayTerminalReceipt, CANCEL_ACTION, INSTALL_ACTION,
    refused,
};
use super::{
    authority::{self, CurrentFacts},
    plans::{self, FundingIntent},
    receipt::BayPaymentHistory,
};
use crate::api::WorldAuth;
use crate::engine::{AuthEval, Delta, NoCustody, NoFacts, ValidateEnv};
use crate::plan::{EconomyPlan, HoldOp, PlanResult, Prepared};
use crate::{Account, Amount, EconomySnapshot, EconomyStore, Hold, TransactionId};
use mush_core::participant::{OwnerPreparationBinding, OwnerPrepareContext, ParticipantError};
use mush_org::PrincipalId;
use std::collections::BTreeMap;

pub(crate) enum Request {
    Fund(Box<BayFundingRequest>),
    Install(BayInstallationRequest),
    Cancel(BayCancellationRequest),
}
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Transition {
    Fund,
    Install,
    Cancel,
}
impl Transition {
    pub fn action(self) -> &'static str {
        match self {
            Self::Fund => ACCEPT_ACTION,
            Self::Install => INSTALL_ACTION,
            Self::Cancel => CANCEL_ACTION,
        }
    }
}
/// Opaque actual owner outputs. Raw decoded histories cannot create them.
pub struct BayFundingOutput {
    pub(super) history: Box<BayPaymentHistory>,
}
impl BayFundingOutput {
    pub fn subject(&self) -> &super::BaySubject {
        self.history.subject()
    }
    pub fn receipt(&self) -> &BayFundingReceipt {
        self.history.funding()
    }
}
pub struct BayTerminalOutput {
    pub(super) history: Box<BayPaymentHistory>,
    receipt: Box<BayTerminalReceipt>,
}
impl BayTerminalOutput {
    pub fn subject(&self) -> &super::BaySubject {
        self.history.subject()
    }
    pub fn receipt(&self) -> &BayTerminalReceipt {
        &self.receipt
    }
}
pub(crate) enum Output {
    Funded(BayFundingOutput),
    Terminal(BayTerminalOutput),
}
pub(super) struct MoneyPreparation {
    pub output: Option<Output>,
    pub history: BayPaymentHistory,
    pub transition: Transition,
    pub binding: OwnerPreparationBinding,
    pub before_history: Option<BayPaymentHistory>,
    pub live: Option<CurrentFacts>,
    pub plan: EconomyPlan,
    pub validated: Prepared,
    pub delta: Delta,
    pub transaction: crate::EconomicTransaction,
    pub next_hold: u64,
}
/// Exact owner-only permission to the native engine. It never authorizes a
/// replacement plan, unrelated hold or title that already exists.
pub(crate) struct ClosedBayPlan {
    plan: EconomyPlan,
    before_hold: Option<Hold>,
}
impl ClosedBayPlan {
    pub(crate) fn plan(&self) -> &EconomyPlan {
        &self.plan
    }
    pub(crate) fn check(
        &self,
        plan: &EconomyPlan,
        snapshot: &EconomySnapshot,
    ) -> Result<(), crate::EconomyError> {
        if plan != &self.plan
            || plan.idempotency.namespace != super::NAMESPACE
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
fn identity(
    config: &BayRefitConfig,
    ctx: &OwnerPrepareContext<'_>,
    phase: Transition,
    requester: mush_core::ObjId,
) -> Result<(), ParticipantError> {
    let inv = ctx.invocation();
    if ctx.world().get(requester).is_none_or(|o| {
        o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
    }) {
        return Err(refused());
    }
    if ctx.action()
        != &config
            .policy
            .action_ref(phase.action())
            .map_err(|_| refused())?
        || inv.actor() != requester
        || inv.principal() != &(mush_core::PrincipalId::Object { id: requester })
        || inv.subject() != &mush_core::PerceivedRef::SelfActor
        || !inv.targets().is_empty()
        || inv.idempotency().scope().owner.as_str() != "industry"
    {
        return Err(refused());
    }
    Ok(())
}
pub(super) fn prepare(
    config: &BayRefitConfig,
    state: &EconomyStore,
    ctx: &OwnerPrepareContext<'_>,
    request: Request,
) -> Result<Box<MoneyPreparation>, ParticipantError> {
    state
        .fold
        .bay_refit_receipts
        .require_complete()
        .map_err(|_| refused())?;
    let (intent, transition, before_history, live) = match request {
        Request::Fund(request) => {
            identity(config, ctx, Transition::Fund, request.subject.requester)?;
            let (intent, live) =
                authority::funding(config, state, ctx.world(), &request.subject, true)
                    .map_err(|_| refused())?;
            (intent, Transition::Fund, None, Some(live))
        }
        Request::Install(request) => {
            let history = state
                .fold
                .bay_refit_receipts
                .get(request.acceptance)
                .map_err(|_| refused())?
                .clone();
            identity(
                config,
                ctx,
                Transition::Install,
                history.subject().requester,
            )?;
            if history.terminal().is_some() {
                return Err(refused());
            }
            history
                .verify_hold(state.snapshot())
                .map_err(|_| refused())?;
            let live = authority::installation(
                config,
                state,
                ctx.world(),
                &history.intent,
                request.acceptance,
            )
            .map_err(|_| refused())?;
            (
                history.intent.as_ref().clone(),
                Transition::Install,
                Some(history),
                Some(live),
            )
        }
        Request::Cancel(request) => {
            let history = state
                .fold
                .bay_refit_receipts
                .get(request.acceptance)
                .map_err(|_| refused())?
                .clone();
            identity(config, ctx, Transition::Cancel, history.subject().requester)?;
            config.cleanup_ready(ctx.world()).map_err(|_| refused())?;
            if history.terminal().is_some() {
                return Err(refused());
            }
            history
                .verify_hold(state.snapshot())
                .map_err(|_| refused())?;
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
            live.as_ref()
                .and_then(|v| v.order.as_ref())
                .map(|v| v.refit),
        )
    } else {
        plans::funding(&intent, ctx.commit_token(), ctx.action())
    }
    .map_err(|_| refused())?;
    let before_hold = before_history.as_ref().map(|h| h.funding.hold.clone());
    let proof = ClosedBayPlan {
        plan: plan.clone(),
        before_hold: before_hold.clone(),
    };
    let validated = validate(state, ctx, &intent, transition, &proof)?;
    let (delta, transaction) = crate::engine::apply_delta(
        state.snapshot(),
        &validated,
        TransactionId(state.next_transaction),
        state.journal.next_seq(crate::module::JOURNAL_LOG),
        ctx.world().now(),
    );
    let history = state
        .fold
        .bay_refit_receipts
        .prepare(&transaction)
        .map_err(|_| refused())?
        .row()
        .clone();
    let next_hold = if transition == Transition::Fund {
        state.next_hold.checked_add(1).ok_or_else(refused)?
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
        return Err(refused());
    }
    let output = if transition == Transition::Fund {
        Output::Funded(BayFundingOutput {
            history: Box::new(history.clone()),
        })
    } else {
        Output::Terminal(BayTerminalOutput {
            receipt: Box::new(history.terminal().cloned().ok_or_else(refused)?),
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
fn validate(
    state: &EconomyStore,
    ctx: &OwnerPrepareContext<'_>,
    intent: &FundingIntent,
    phase: Transition,
    proof: &ClosedBayPlan,
) -> Result<Prepared, ParticipantError> {
    let auth = BayAuth {
        world: WorldAuth { w: ctx.world() },
        intent,
        phase,
        hold: proof.before_hold.as_ref(),
    };
    let keys = state.key_gate();
    let mut spent = BTreeMap::new();
    if let Some(window) = state
        .snapshot()
        .account(intent.subject.payer)
        .and_then(|a| a.approval.as_ref())
        .and_then(|a| a.window.as_ref())
    {
        spent.insert(
            intent.subject.payer,
            state.spent_in_window(intent.subject.payer, ctx.world().now(), window.period_secs),
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
    match crate::engine::validate_bay_refit(proof, state.snapshot(), &env) {
        PlanResult::Post(p) => Ok(*p),
        PlanResult::NeedsApproval(_) | PlanResult::AlreadyPosted(_) | PlanResult::Refused(_) => {
            Err(refused())
        }
    }
}
pub(super) fn recheck(
    config: &BayRefitConfig,
    state: &EconomyStore,
    ctx: &OwnerPrepareContext<'_>,
    prepared: &MoneyPreparation,
) -> Result<(), ParticipantError> {
    if !ctx.matches_preparation(&prepared.binding) {
        return Err(refused());
    }
    identity(
        config,
        ctx,
        prepared.transition,
        prepared.history.subject().requester,
    )?;
    let intent = prepared.history.intent.as_ref();
    let current = match prepared.transition {
        Transition::Fund => {
            // Historical accepted_at remains fixed. Recheck the exact old offer
            // against monotonic current time, not a replacement accepted intent.
            let (actual, facts) =
                authority::funding(config, state, ctx.world(), &intent.subject, false)
                    .map_err(|_| refused())?;
            if &actual != intent {
                return Err(refused());
            }
            facts
        }
        Transition::Install => authority::installation(
            config,
            state,
            ctx.world(),
            intent,
            prepared.history.funding.commit,
        )
        .map_err(|_| refused())?,
        Transition::Cancel => {
            config.cleanup_ready(ctx.world()).map_err(|_| refused())?;
            return Ok(());
        }
    };
    let old = prepared.live.as_ref().ok_or_else(refused)?;
    if current.accounts != old.accounts
        || current.instruments != old.instruments
        || current.titles != old.titles
        || current.order != old.order
        || current.supplier != old.supplier
        || current.at.clock != old.at.clock
        || current.at.at < old.at.at
        || current.utc < old.utc
    {
        return Err(refused());
    }
    let proof = ClosedBayPlan {
        plan: prepared.plan.clone(),
        before_hold: prepared
            .before_history
            .as_ref()
            .map(|h| h.funding.hold.clone()),
    };
    if validate(state, ctx, intent, prepared.transition, &proof)? != prepared.validated {
        return Err(refused());
    }
    Ok(())
}
struct BayAuth<'a> {
    world: WorldAuth<'a>,
    intent: &'a FundingIntent,
    phase: Transition,
    hold: Option<&'a Hold>,
}
impl AuthEval for BayAuth<'_> {
    fn authorize(&self, actor: &PrincipalId, account: &Account, delta: Amount) -> bool {
        if actor != &PrincipalId::System(self.intent.policy.issuer.clone()) {
            return false;
        }
        if account.id == self.intent.subject.payer {
            return delta.checked_neg() == self.intent.price().ok()
                && account.owner == PrincipalId::Object(self.intent.subject.requester)
                && (self.phase == Transition::Install
                    || self.world.authorize(
                        &PrincipalId::Object(self.intent.subject.requester),
                        account,
                        delta,
                    ));
        }
        // The provider is credited the subject's quoted fee, at Install only.
        self.phase == Transition::Install
            && self
                .intent
                .provider_account()
                .is_ok_and(|provider| account.id == provider)
            && self.intent.price().is_ok_and(|price| delta == price)
    }
    fn authorize_hold(&self, actor: &PrincipalId, hold: &Hold, op: &HoldOp) -> bool {
        if actor != &PrincipalId::System(self.intent.policy.issuer.clone())
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

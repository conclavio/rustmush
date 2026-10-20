//! Closed native owner preparation. The paired Market/Roll producer proves
//! seller offer, current paid holder and claims; Economy proves actual money.
use super::history::PaidResaleHistoryV1;
use super::plans::Intent;
use super::*;
use crate::api::WorldAuth;
use crate::contract::{EconomicContract, check_offerable};
use crate::engine::{AuthEval, Delta, NoCustody, NoFacts, ValidateEnv};
use crate::plan::{EconomyPlan, HoldOp, PlanResult, Prepared};
use crate::record::{
    Account, AccountStatus, AssetLifecycle, EconomicAsset, EconomicTransaction, Hold,
    InstrumentStatus,
};
use crate::{AccountId, Amount, ContractId, EconomySnapshot, EconomyStore, HoldId, TransactionId};
use mush_core::participant::{OwnerPreparationBinding, OwnerPrepareContext};
use mush_core::{CommitToken, DefinitionDigest, DefinitionRef, ObjId, World};
use mush_org::{PrincipalId, SystemPrincipalId};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct PaidResaleConfig {
    pub(crate) issuer: SystemPrincipalId,
    pub(crate) service: ObjId,
    pub(crate) actions: [DefinitionRef; 5],
    pub(crate) revoke: Option<DefinitionRef>,
}
impl PaidResaleConfig {
    pub fn new(
        issuer: SystemPrincipalId,
        service: ObjId,
        actions: [DefinitionRef; 5],
        revoke: Option<DefinitionRef>,
    ) -> Result<Self, ParticipantError> {
        let v = Self {
            issuer,
            service,
            actions,
            revoke,
        };
        v.validate()?;
        Ok(v)
    }
    pub(crate) fn validate(&self) -> Result<(), ParticipantError> {
        if self.service.0 == 0
            || self
                .actions
                .iter()
                .zip([OFFER, ACCEPT, FUND, SETTLE, REFUND])
                .any(|(a, id)| !action_matches(a, id))
            || self.revoke.as_ref().is_some_and(|a| !registry_action(a))
        {
            return Err(refused());
        }
        Ok(())
    }
}
/// This request is supplied by the registered paired provider, not decoded as
/// public command parameters. Commit, actor and clocks come from its real ctx.
#[derive(Clone)]
pub struct PaidResaleAcceptanceRequest {
    pub terms: PaidResaleTermsV1,
    pub offer_digest: DefinitionDigest,
    pub payer: AccountId,
    pub claims: PaidResaleClaimsV1,
    pub mode: PaidResaleModeV1,
}
#[derive(Clone)]
pub struct PaidResaleTerminalRequest {
    pub acceptance: CommitToken,
    pub decision: PaidResaleDecisionV1,
}
#[derive(Clone)]
pub(crate) enum Request {
    Accept(Box<PaidResaleAcceptanceRequest>),
    Terminal(Box<PaidResaleTerminalRequest>),
}
pub struct PaidResaleOutput {
    pub(super) history: Box<PaidResaleHistoryV1>,
}
impl PaidResaleOutput {
    pub fn history(&self) -> &PaidResaleHistoryV1 {
        &self.history
    }
}
pub(super) struct Preparation {
    pub output: PaidResaleOutput,
    pub binding: OwnerPreparationBinding,
    pub request: Request,
    pub before_history: Option<PaidResaleHistoryV1>,
    pub before_contract: Option<EconomicContract>,
    pub before_hold: Option<Hold>,
    pub before_titles: Vec<EconomicAsset>,
    pub plan: EconomyPlan,
    pub validated: Prepared,
    pub contract: EconomicContract,
    pub delta: Delta,
    pub transaction: EconomicTransaction,
    pub next_hold: u64,
    pub next_contract: u64,
}
pub(crate) struct ClosedPaidResalePlan {
    plan: EconomyPlan,
    contract: ContractId,
    before_contract: Option<EconomicContract>,
    hold: Option<HoldId>,
    before_hold: Option<Hold>,
}
impl ClosedPaidResalePlan {
    pub(super) fn retained(
        plan: EconomyPlan,
        intent: &Intent,
        before_contract: Option<EconomicContract>,
        before_hold: Option<Hold>,
    ) -> Self {
        Self {
            plan,
            contract: intent.contract,
            hold: intent.hold,
            before_contract,
            before_hold,
        }
    }
    pub(crate) fn plan(&self) -> &EconomyPlan {
        &self.plan
    }
    pub(crate) fn check(
        &self,
        plan: &EconomyPlan,
        snapshot: &EconomySnapshot,
    ) -> Result<(), crate::EconomyError> {
        if plan != &self.plan
            || plan.idempotency.namespace.as_str() != NAMESPACE
            || snapshot.contract(self.contract) != self.before_contract.as_ref()
            || self.hold.and_then(|id| snapshot.hold(id)) != self.before_hold.as_ref()
            || !plan.asset_ops.is_empty()
            || !plan.custody_ops.is_empty()
            || self.before_contract.as_ref().is_some_and(|c| {
                c.consent
                    .managed
                    .as_ref()
                    .and_then(|m| m.paid_resale_marker())
                    .is_none()
            })
        {
            return Err(crate::EconomyError::ManagedContract);
        }
        Ok(())
    }
}
fn live(world: &World, id: ObjId) -> bool {
    world.get(id).is_some_and(|o| {
        o.kind == mush_core::ObjKind::Player && !o.flags.contains(mush_core::Flags::GOING)
    })
}
fn identity(
    ctx: &OwnerPrepareContext<'_>,
    action: &DefinitionRef,
    actor: ObjId,
) -> Result<(), ParticipantError> {
    let inv = ctx.invocation();
    if ctx.action() != action
        || inv.actor() != actor
        || inv.principal() != &(mush_core::PrincipalId::Object { id: actor })
        || inv.subject() != &mush_core::PerceivedRef::SelfActor
        || !inv.targets().is_empty()
        || inv.idempotency().scope().owner != action.key().owner
        || !live(ctx.world(), actor)
    {
        return Err(refused());
    }
    Ok(())
}
pub(super) fn recipients(c: &PaidResaleConsentV1) -> Vec<&crate::managed_entitlement::Recipient> {
    let t = &c.terms;
    let mut roles = Vec::with_capacity(6);
    roles.extend(t.registry.recipient.iter());
    roles.extend(t.venue.iter().flat_map(|x| x.recipient.iter()));
    roles.extend(
        t.parents
            .iter()
            .filter_map(|x| x.title.as_ref().map(|t| &t.recipient)),
    );
    roles.push(&t.remainder);
    roles
}
pub(super) fn recipient_available(
    state: &EconomyStore,
    world: &World,
    c: &PaidResaleConsentV1,
) -> bool {
    live(world, c.terms.buyer)
        && state.snapshot.account(c.payer).is_some_and(|a| {
            a.owner == PrincipalId::Object(c.terms.buyer)
                && a.scope.is_none()
                && a.instrument == c.terms.instrument
                && a.status == AccountStatus::Active
                && a.approval.is_none()
        })
}
pub(super) fn monetary_current(
    state: &EconomyStore,
    world: &World,
    c: &PaidResaleConsentV1,
) -> bool {
    if !recipient_available(state, world, c)
        || state
            .snapshot
            .instrument(&c.terms.instrument)
            .is_none_or(|i| i.status != InstrumentStatus::Active || !i.transferable)
    {
        return false;
    }
    if recipients(c).into_iter().any(|r| {
        state.snapshot.account(r.account).is_none_or(|a| {
            a.owner != r.owner
                || a.instrument != c.terms.instrument
                || a.status != AccountStatus::Active
                || !crate::api::credit_allowed(world, a)
        })
    }) {
        return false;
    }
    for cut in &c.terms.parents {
        let Some(expected) = &cut.title else {
            continue;
        };
        let Some(title) = state.snapshot.asset(cut.charter) else {
            return false;
        };
        if title.title_owner != expected.recipient.owner
            || title.revision != expected.title_revision
            || title.scope.is_some()
            || title.lifecycle != AssetLifecycle::Active
            || title.frozen
            || title.frozen_reason.is_some()
            || title.stolen.is_some()
            || !title.encumbrances.is_empty()
            || world
                .get(cut.charter)
                .is_none_or(|o| o.flags.contains(mush_core::Flags::GOING))
        {
            return false;
        }
    }
    true
}
pub(super) fn current(
    config: &PaidResaleConfig,
    state: &EconomyStore,
    ctx: &OwnerPrepareContext<'_>,
    c: &PaidResaleConsentV1,
    decision: Option<&PaidResaleDecisionV1>,
) -> Result<(), ParticipantError> {
    config.validate()?;
    c.validate()?;
    state.fold.paid_resale_receipts.require_complete()?;
    if state.frozen {
        return Err(refused());
    }
    let now = ctx.world().world_ic_now();
    if let Some(d) = decision {
        d.matches_consent(c)?;
        let compound = matches!(d.reason,PaidResaleDecisionKindV1::RegistryRevoked{sanction,..} if sanction==d.operation);
        let expected = if compound {
            config.revoke.as_ref().ok_or_else(refused)?
        } else {
            &config.actions[if d.reason.settles() { 3 } else { 4 }]
        };
        identity(
            ctx,
            expected,
            if compound { d.actor } else { config.service },
        )?;
        if d.operation != ctx.commit_token()
            || d.action != *ctx.action()
            || d.actor != ctx.invocation().actor()
            || now.clock != d.at_ic.clock
            || now.at < d.at_ic.at
            || ctx.world().now() < d.posted_at_utc
        {
            return Err(refused());
        }
        match &d.reason {
            PaidResaleDecisionKindV1::DeadlineReached { .. } if !c.deadline_reached(&now)? => {
                return Err(refused());
            }
            PaidResaleDecisionKindV1::ClockChanged {} if now.clock == c.accepted_at_ic.clock => {
                return Err(refused());
            }
            PaidResaleDecisionKindV1::RecipientUnavailable {}
                if recipient_available(state, ctx.world(), c) =>
            {
                return Err(refused());
            }
            PaidResaleDecisionKindV1::MonetaryBindingChanged {}
                if monetary_current(state, ctx.world(), c) =>
            {
                return Err(refused());
            }
            _ => (),
        }
        if d.reason.settles() && !monetary_current(state, ctx.world(), c) {
            return Err(refused());
        }
    } else {
        identity(
            ctx,
            &config.actions[if c.mode == PaidResaleModeV1::Immediate {
                1
            } else {
                2
            }],
            c.terms.buyer,
        )?;
        if c.operation != ctx.commit_token()
            || c.effect != *ctx.action()
            || c.terms.offer_action != config.actions[0]
            || !live(ctx.world(), c.terms.seller)
            || !monetary_current(state, ctx.world(), c)
            || now.clock != c.accepted_at_ic.clock
            || now.at < c.accepted_at_ic.at
            || ctx.world().now() < c.accepted_at_utc
            || ctx.world().now() >= c.terms.expires_at_utc
        {
            return Err(refused());
        }
    }
    Ok(())
}
struct Selected {
    request: Request,
    intent: Intent,
    before_history: Option<PaidResaleHistoryV1>,
    before_contract: Option<EconomicContract>,
    before_hold: Option<Hold>,
    decision: Option<PaidResaleDecisionV1>,
    next_hold: u64,
    next_contract: u64,
}

#[inline(never)]
fn select(
    config: &PaidResaleConfig,
    state: &EconomyStore,
    ctx: &OwnerPrepareContext<'_>,
    request: Request,
) -> Result<Box<Selected>, ParticipantError> {
    let (intent, before_history, before_contract, before_hold, decision, next_hold, next_contract) =
        match &request {
            Request::Accept(r) => {
                let accepted = ctx.world().world_ic_now();
                let c = PaidResaleConsentV1 {
                    format: 1,
                    operation: ctx.commit_token(),
                    effect: ctx.action().clone(),
                    offer_digest: r.offer_digest,
                    terms: r.terms.clone(),
                    payer: r.payer,
                    mode: r.mode,
                    accepted_at_ic: accepted.clone(),
                    accepted_at_utc: ctx.world().now(),
                    deadline: r.terms.deadline(&accepted)?,
                    claims: r.claims.clone(),
                };
                current(config, state, ctx, &c, None)?;
                let contract = ContractId(state.next_contract);
                let hold = (c.mode == PaidResaleModeV1::Pending).then_some(HoldId(state.next_hold));
                if state.snapshot.contract(contract).is_some()
                    || hold.is_some_and(|id| state.snapshot.hold(id).is_some())
                {
                    return Err(refused());
                }
                (
                    Intent {
                        consent: c,
                        issuer: config.issuer.clone(),
                        contract,
                        hold,
                    },
                    None,
                    None,
                    None,
                    None,
                    if hold.is_some() {
                        state.next_hold.checked_add(1).ok_or_else(refused)?
                    } else {
                        state.next_hold
                    },
                    state.next_contract.checked_add(1).ok_or_else(refused)?,
                )
            }
            Request::Terminal(r) => {
                let history = state.fold.paid_resale_receipts.get(r.acceptance)?.clone();
                if history.outcome() != PaidResaleOutcome::Funded
                    || history.intent().issuer != config.issuer
                {
                    return Err(refused());
                }
                current(config, state, ctx, history.consent(), Some(&r.decision))?;
                if r.decision.at_ic != ctx.world().world_ic_now()
                    || r.decision.posted_at_utc != ctx.world().now()
                {
                    return Err(refused());
                }
                let contract = state
                    .snapshot
                    .contract(history.contract_id())
                    .cloned()
                    .ok_or_else(refused)?;
                let hold = state
                    .snapshot
                    .hold(history.acceptance().hold().ok_or_else(refused)?)
                    .cloned()
                    .ok_or_else(refused)?;
                history.verify_documents(&contract, Some(&hold))?;
                (
                    history.intent().clone(),
                    Some(history),
                    Some(contract),
                    Some(hold),
                    Some(r.decision.clone()),
                    state.next_hold,
                    state.next_contract,
                )
            }
        };
    Ok(Box::new(Selected {
        request,
        intent,
        before_history,
        before_contract,
        before_hold,
        decision,
        next_hold,
        next_contract,
    }))
}

pub(super) fn prepare(
    config: &PaidResaleConfig,
    state: &EconomyStore,
    ctx: &OwnerPrepareContext<'_>,
    request: Request,
) -> Result<Box<Preparation>, ParticipantError> {
    let Selected {
        request,
        intent,
        before_history,
        before_contract,
        before_hold,
        decision,
        next_hold,
        next_contract,
    } = *select(config, state, ctx, request)?;
    let before_titles = if decision.as_ref().is_none_or(|d| d.reason.settles()) {
        intent
            .consent
            .terms
            .parents
            .iter()
            .filter(|c| c.title.is_some())
            .map(|c| state.snapshot.asset(c.charter).cloned().ok_or_else(refused))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    let plan = match &decision {
        Some(d) => plans::terminal(&intent, d)?,
        None => plans::acceptance(&intent)?,
    };
    let proof = ClosedPaidResalePlan::retained(
        plan.clone(),
        &intent,
        before_contract.clone(),
        before_hold.clone(),
    );
    let validated = validate(
        state,
        ctx.world(),
        &intent,
        decision.as_ref(),
        before_hold.as_ref(),
        &proof,
    )?;
    let (delta, transaction) = crate::engine::apply_delta(
        &state.snapshot,
        &validated,
        TransactionId(state.next_transaction),
        state.journal.next_seq(crate::module::JOURNAL_LOG),
        ctx.world().now(),
    );
    if delta.consumed_approval.is_some()
        || delta.reversed.is_some()
        || !delta.assets.is_empty()
        || delta.holds.len() != usize::from(intent.hold.is_some())
        || !transaction.custody_effects.is_empty()
    {
        return Err(refused());
    }
    let history = state.fold.paid_resale_receipts.prepare(&transaction)?;
    let contract = history.contract()?;
    if decision.is_none() {
        check_offerable(&contract, &state.snapshot).map_err(|_| refused())?;
    }
    Ok(Box::new(Preparation {
        output: PaidResaleOutput {
            history: Box::new(history),
        },
        binding: ctx.preparation_binding(),
        request,
        before_history,
        before_contract,
        before_hold,
        before_titles,
        plan,
        validated,
        contract,
        delta,
        transaction,
        next_hold,
        next_contract,
    }))
}
pub(super) fn validate(
    state: &EconomyStore,
    world: &World,
    intent: &Intent,
    decision: Option<&PaidResaleDecisionV1>,
    hold: Option<&Hold>,
    proof: &ClosedPaidResalePlan,
) -> Result<Prepared, ParticipantError> {
    let ordinary = WorldAuth { w: world };
    let terminal = hold.map(|h| TerminalAuth {
        intent,
        hold: h,
        settle: decision.is_some_and(|d| d.reason.settles()),
        payouts: intent.consent.terms.payouts(intent.consent.payer),
    });
    let auth: &dyn AuthEval = match &terminal {
        Some(t) => t,
        None => &ordinary,
    };
    let gate = state.key_gate();
    let mut spent = BTreeMap::new();
    if let Some(window) = state
        .snapshot
        .account(intent.consent.payer)
        .and_then(|a| a.approval.as_ref())
        .and_then(|a| a.window.as_ref())
    {
        spent.insert(
            intent.consent.payer,
            state.spent_in_window(intent.consent.payer, world.now(), window.period_secs),
        );
    }
    let env = ValidateEnv {
        auth,
        custody: &NoCustody,
        facts: &NoFacts,
        now: world.now(),
        ic_now: world.game_now_unix(),
        next_hold_id: state.next_hold,
        spent,
        keys: &gate,
    };
    match crate::engine::validate_paid_resale(proof, &state.snapshot, &env) {
        PlanResult::Post(p) => Ok(*p),
        _ => Err(refused()),
    }
}
struct TerminalAuth<'a> {
    intent: &'a Intent,
    hold: &'a Hold,
    settle: bool,
    payouts: Result<Vec<crate::managed_purchase::PurchasePayout>, ParticipantError>,
}
impl AuthEval for TerminalAuth<'_> {
    fn authorize(&self, actor: &PrincipalId, account: &Account, delta: Amount) -> bool {
        actor == &PrincipalId::System(self.intent.issuer.clone())
            && self.settle
            && delta.minor() > 0
            && self.payouts.as_ref().is_ok_and(|p| {
                p.iter().any(|p| {
                    p.account == account.id && p.owner == account.owner && p.amount == delta
                })
            })
    }
    fn authorize_hold(&self, actor: &PrincipalId, hold: &Hold, op: &HoldOp) -> bool {
        if actor != &PrincipalId::System(self.intent.issuer.clone()) || hold != self.hold {
            return false;
        }
        match (self.settle, op) {
            (true, HoldOp::Capture { hold: id, amount }) => {
                *id == hold.id && *amount == hold.amount
            }
            (false, HoldOp::Release { hold: id }) => *id == hold.id,
            _ => false,
        }
    }
}

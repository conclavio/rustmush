//! Actual native preparation. Only the exact registered paired Roll producer
//! can invoke this owner branch; neither request nor history is a capability.
use super::history::CharterPendingHistory;
use super::plans::FundingIntent;
use super::*;
use crate::api::WorldAuth;
use crate::contract::{EconomicContract, check_offerable};
use crate::engine::{AuthEval, Delta, NoCustody, NoFacts, ValidateEnv};
use crate::plan::{AssetOp, EconomyPlan, HoldOp, PlanResult, Prepared};
use crate::record::{
    Account, AccountStatus, AssetLifecycle, CatalogRef, EconomicAsset, EconomicTransaction, Hold,
    InstrumentStatus,
};
use crate::{AccountId, Amount, ContractId, EconomySnapshot, EconomyStore, HoldId, TransactionId};
use mush_core::participant::{OwnerPreparationBinding, OwnerPrepareContext};
use mush_core::{CommitToken, DefinitionRef, ObjId, World};
use mush_org::{PrincipalId, SystemPrincipalId};
use std::collections::BTreeMap;

/// Explicit trusted composition only. The paired Roll registration must repeat
/// these exact full refs and service identity before exposing the action chain.
#[derive(Clone, Debug)]
pub struct CharterPendingConfig {
    pub(crate) issuer: SystemPrincipalId,
    pub(crate) catalog: CatalogRef,
    pub(crate) service: ObjId,
    pub(crate) fund: DefinitionRef,
    pub(crate) settle: DefinitionRef,
    pub(crate) refund: DefinitionRef,
    pub(crate) revoke: Option<DefinitionRef>,
}
impl CharterPendingConfig {
    pub fn new(
        issuer: SystemPrincipalId,
        catalog: CatalogRef,
        service: ObjId,
        actions: [DefinitionRef; 3],
        revoke: Option<DefinitionRef>,
    ) -> Result<Self, ParticipantError> {
        let [fund, settle, refund] = actions;
        let config = Self {
            issuer,
            catalog,
            service,
            fund,
            settle,
            refund,
            revoke,
        };
        config.validate()?;
        Ok(config)
    }
    pub(crate) fn validate(&self) -> Result<(), ParticipantError> {
        if self.service.0 == 0
            || !action_matches(&self.fund, FUND_ACTION, 1)
            || !action_matches(&self.settle, SETTLE_ACTION, 1)
            || !action_matches(&self.refund, REFUND_ACTION, 1)
            || self
                .revoke
                .as_ref()
                .is_some_and(|a| !action_matches(a, "registry_term_revocation", 1))
        {
            return Err(refused());
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct CharterPendingFundingRequest {
    pub terms: CharterPendingTermsV1,
    pub payer: AccountId,
}
#[derive(Clone)]
pub struct CharterPendingTerminalRequest {
    pub funding: CommitToken,
    pub decision: CharterPendingDecisionV1,
}
#[derive(Clone)]
pub(crate) enum Request {
    Fund(CharterPendingFundingRequest),
    Terminal(CharterPendingTerminalRequest),
}
/// Actual prepared owner output; no Deserialize or public constructor.
pub struct CharterPendingOutput {
    pub(super) history: Box<CharterPendingHistory>,
}
impl CharterPendingOutput {
    pub fn history(&self) -> &CharterPendingHistory {
        &self.history
    }
}
pub(super) struct Preparation {
    pub output: CharterPendingOutput,
    pub binding: OwnerPreparationBinding,
    pub request: Request,
    pub before_history: Option<CharterPendingHistory>,
    pub before_contract: Option<EconomicContract>,
    pub before_hold: Option<Hold>,
    pub before_title: EconomicAsset,
    pub plan: EconomyPlan,
    pub validated: Prepared,
    pub contract: EconomicContract,
    pub delta: Delta,
    pub transaction: EconomicTransaction,
    pub next_hold: u64,
    pub next_contract: u64,
}
/// Private exact plan and before images, never constructed by client data.
pub(crate) struct ClosedCharterPendingPlan {
    plan: EconomyPlan,
    contract: ContractId,
    before_contract: Option<EconomicContract>,
    hold: HoldId,
    before_hold: Option<Hold>,
    title: EconomicAsset,
}
impl ClosedCharterPendingPlan {
    pub(super) fn retained(
        plan: EconomyPlan,
        intent: &FundingIntent,
        before_contract: Option<EconomicContract>,
        before_hold: Option<Hold>,
        title: EconomicAsset,
    ) -> Self {
        Self {
            plan,
            contract: intent.contract,
            before_contract,
            hold: intent.hold,
            before_hold,
            title,
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
            || snapshot.hold(self.hold) != self.before_hold.as_ref()
            || snapshot.asset(self.title.object) != Some(&self.title)
            || snapshot.managed_titles.charter(self.title.object).is_none()
            || !plan.custody_ops.is_empty()
            || self.before_contract.as_ref().is_some_and(|c| {
                c.consent
                    .managed
                    .as_ref()
                    .and_then(|m| m.charter_pending_marker())
                    .is_none()
            })
        {
            return Err(crate::EconomyError::ManagedContract);
        }
        Ok(())
    }
}
fn live(world: &World, actor: ObjId) -> bool {
    world.get(actor).is_some_and(|o| {
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
        || inv.idempotency().scope().owner.as_str() != "roll"
        || !live(ctx.world(), actor)
    {
        return Err(refused());
    }
    Ok(())
}
fn title<'a>(
    state: &'a EconomyStore,
    c: &CharterPendingConsentV1,
    catalog: &CatalogRef,
) -> Result<&'a EconomicAsset, ParticipantError> {
    state
        .fold
        .charter_transfer_receipts
        .require_complete()
        .map_err(|_| refused())?;
    let lineage = state
        .fold
        .charter_transfer_receipts
        .lineage(c.terms.charter)
        .ok_or_else(refused)?;
    let t = state.snapshot.asset(c.terms.charter).ok_or_else(refused)?;
    if lineage.birth().commit() != c.terms.birth
        || lineage.title() != t
        || state
            .snapshot
            .managed_titles
            .charter(c.terms.charter)
            .is_none()
        || t.object != c.terms.charter
        || t.title_owner != PrincipalId::Object(c.terms.seller)
        || t.revision != c.terms.title_revision
        || t.scope.is_some()
        || t.frozen
        || t.frozen_reason.is_some()
        || t.stolen.is_some()
        || t.lifecycle != AssetLifecycle::Active
        || !t.encumbrances.is_empty()
        || t.catalog.as_ref() != Some(catalog)
    {
        return Err(refused());
    }
    Ok(t)
}
fn recipients(state: &EconomyStore, world: &World, c: &CharterPendingConsentV1) -> bool {
    if !live(world, c.terms.buyer) {
        return false;
    }
    let Some(payer) = state.snapshot.account(c.payer) else {
        return false;
    };
    let Some(seller) = state.snapshot.account(c.terms.seller_account) else {
        return false;
    };
    payer.owner == PrincipalId::Object(c.terms.buyer)
        && payer.scope.is_none()
        && payer.status == AccountStatus::Active
        && payer.instrument == c.terms.instrument
        && seller.owner == PrincipalId::Object(c.terms.seller)
        && seller.scope.is_none()
        && seller.status == AccountStatus::Active
        && seller.instrument == c.terms.instrument
        && state
            .snapshot
            .instrument(&c.terms.instrument)
            .is_some_and(|i| i.status == InstrumentStatus::Active)
        && WorldAuth { w: world }.authorize_title_receipt(
            &PrincipalId::Object(c.terms.buyer),
            &PrincipalId::Object(c.terms.buyer),
            None,
        )
}
pub(super) fn current(
    config: &CharterPendingConfig,
    state: &EconomyStore,
    ctx: &OwnerPrepareContext<'_>,
    consent: &CharterPendingConsentV1,
    decision: Option<&CharterPendingDecisionV1>,
) -> Result<(), ParticipantError> {
    config.validate()?;
    consent.validate()?;
    state.fold.charter_pending_receipts.require_complete()?;
    if state.frozen || !state.is_title_issuer(&config.issuer) {
        return Err(refused());
    }
    title(state, consent, &config.catalog)?;
    let now = ctx.world().world_ic_now();
    if let Some(d) = decision {
        d.matches_consent(consent)?;
        let compound = matches!(d.reason, CharterPendingDecisionKindV1::RegistryRevoked { sanction, .. } if sanction == d.operation);
        let expected = if compound {
            config.revoke.as_ref().ok_or_else(refused)?
        } else if d.reason.settles() {
            &config.settle
        } else {
            &config.refund
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
        // Repeat the reason's actual native predicate. Claims and the real
        // sanction are independently proved by retained Roll owner custody.
        match d.reason {
            CharterPendingDecisionKindV1::DeadlineReached { .. } => {
                if !consent.deadline_reached(&now)? {
                    return Err(refused());
                }
            }
            CharterPendingDecisionKindV1::ClockChanged => {
                if now.clock == consent.accepted_at_ic.clock {
                    return Err(refused());
                }
            }
            CharterPendingDecisionKindV1::RecipientUnavailable => {
                if recipients(state, ctx.world(), consent) {
                    return Err(refused());
                }
            }
            _ => (),
        }
        if d.reason.settles() && !recipients(state, ctx.world(), consent) {
            return Err(refused());
        }
    } else {
        identity(ctx, &config.fund, consent.terms.buyer)?;
        if consent.operation != ctx.commit_token()
            || consent.effect != *ctx.action()
            || !live(ctx.world(), consent.terms.seller)
            || !recipients(state, ctx.world(), consent)
            || now.clock != consent.accepted_at_ic.clock
            || now.at < consent.accepted_at_ic.at
            || ctx.world().now() < consent.accepted_at_utc
            || ctx.world().now() >= consent.terms.expires_at_utc
        {
            return Err(refused());
        }
        let op = AssetOp::TransferTitle {
            object: consent.terms.charter,
            to: PrincipalId::Object(consent.terms.buyer),
            to_scope: None,
        };
        if !(WorldAuth { w: ctx.world() }).authorize_asset(
            &PrincipalId::Object(consent.terms.seller),
            state.snapshot.asset(consent.terms.charter),
            &op,
        ) {
            return Err(refused());
        }
    }
    Ok(())
}
pub(super) fn prepare(
    config: &CharterPendingConfig,
    state: &EconomyStore,
    ctx: &OwnerPrepareContext<'_>,
    request: Request,
) -> Result<Box<Preparation>, ParticipantError> {
    let (intent, before_history, before_contract, before_hold, decision, next_hold, next_contract) =
        match &request {
            Request::Fund(r) => {
                let accepted = ctx.world().world_ic_now();
                let consent = CharterPendingConsentV1 {
                    format: 1,
                    operation: ctx.commit_token(),
                    effect: ctx.action().clone(),
                    terms: r.terms.clone(),
                    payer: r.payer,
                    deadline: r.terms.deadline(&accepted)?,
                    accepted_at_ic: accepted,
                    accepted_at_utc: ctx.world().now(),
                };
                current(config, state, ctx, &consent, None)?;
                let contract = ContractId(state.next_contract);
                let hold = HoldId(state.next_hold);
                if state.snapshot.contract(contract).is_some()
                    || state.snapshot.hold(hold).is_some()
                {
                    return Err(refused());
                }
                (
                    FundingIntent {
                        consent,
                        issuer: config.issuer.clone(),
                        catalog: config.catalog.clone(),
                        contract,
                        hold,
                    },
                    None,
                    None,
                    None,
                    None,
                    state.next_hold.checked_add(1).ok_or_else(refused)?,
                    state.next_contract.checked_add(1).ok_or_else(refused)?,
                )
            }
            Request::Terminal(r) => {
                let history = state.fold.charter_pending_receipts.get(r.funding)?.clone();
                if history.terminal().is_some()
                    || history.intent().issuer != config.issuer
                    || history.intent().catalog != config.catalog
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
                    .hold(history.funding().hold())
                    .cloned()
                    .ok_or_else(refused)?;
                history.verify_documents(&contract, &hold)?;
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
    let before_title = title(state, &intent.consent, &intent.catalog)?.clone();
    let plan = match &decision {
        Some(d) => plans::terminal(&intent, d)?,
        None => plans::funding(&intent)?,
    };
    let proof = ClosedCharterPendingPlan {
        plan: plan.clone(),
        contract: intent.contract,
        before_contract: before_contract.clone(),
        hold: intent.hold,
        before_hold: before_hold.clone(),
        title: before_title.clone(),
    };
    let validated = validate(
        state,
        ctx.world(),
        &intent,
        decision.as_ref(),
        before_hold.as_ref(),
        &proof,
    )?;
    let (delta, tx) = crate::engine::apply_delta(
        &state.snapshot,
        &validated,
        TransactionId(state.next_transaction),
        state.journal.next_seq(crate::module::JOURNAL_LOG),
        ctx.world().now(),
    );
    if delta.consumed_approval.is_some()
        || delta.reversed.is_some()
        || delta.holds.len() != 1
        || delta.assets.len() != usize::from(decision.as_ref().is_some_and(|d| d.reason.settles()))
    {
        return Err(refused());
    }
    let history = state.fold.charter_pending_receipts.prepare(&tx)?;
    let contract = history.contract()?;
    if decision.is_none() {
        check_offerable(&contract, &state.snapshot).map_err(|_| refused())?;
    }
    Ok(Box::new(Preparation {
        output: CharterPendingOutput {
            history: Box::new(history),
        },
        binding: ctx.preparation_binding(),
        request,
        before_history,
        before_contract,
        before_hold,
        before_title,
        plan,
        validated,
        contract,
        delta,
        transaction: tx,
        next_hold,
        next_contract,
    }))
}
pub(super) fn validate(
    state: &EconomyStore,
    world: &World,
    intent: &FundingIntent,
    decision: Option<&CharterPendingDecisionV1>,
    hold: Option<&Hold>,
    proof: &ClosedCharterPendingPlan,
) -> Result<Prepared, ParticipantError> {
    let funding_auth = WorldAuth { w: world };
    let terminal_auth = hold.map(|hold| TerminalAuth {
        world,
        intent,
        hold,
        settle: decision.is_some_and(|d| d.reason.settles()),
    });
    let auth: &dyn AuthEval = match &terminal_auth {
        Some(a) => a,
        None => &funding_auth,
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
    match crate::engine::validate_charter_pending(proof, &state.snapshot, &env) {
        PlanResult::Post(p) => Ok(*p),
        _ => Err(refused()),
    }
}
struct TerminalAuth<'a> {
    world: &'a World,
    intent: &'a FundingIntent,
    hold: &'a Hold,
    settle: bool,
}
impl AuthEval for TerminalAuth<'_> {
    fn authorize(&self, actor: &PrincipalId, account: &Account, delta: Amount) -> bool {
        actor == &PrincipalId::System(self.intent.issuer.clone())
            && self.settle
            && account.id == self.intent.consent.terms.seller_account
            && account.owner == PrincipalId::Object(self.intent.consent.terms.seller)
            && account.scope.is_none()
            && delta == self.intent.consent.terms.price
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
    fn authorize_asset(
        &self,
        actor: &PrincipalId,
        asset: Option<&EconomicAsset>,
        op: &AssetOp,
    ) -> bool {
        let t = &self.intent.consent.terms;
        actor == &PrincipalId::System(self.intent.issuer.clone())
            && self.settle
            && op
                == &(AssetOp::TransferTitle {
                    object: t.charter,
                    to: PrincipalId::Object(t.buyer),
                    to_scope: None,
                })
            && asset.is_some_and(|a| {
                a.object == t.charter
                    && a.title_owner == PrincipalId::Object(t.seller)
                    && a.revision == t.title_revision
            })
    }
    fn authorize_title_receipt(
        &self,
        actor: &PrincipalId,
        to: &PrincipalId,
        scope: Option<mush_org::OrgScopeId>,
    ) -> bool {
        let buyer = PrincipalId::Object(self.intent.consent.terms.buyer);
        actor == &PrincipalId::System(self.intent.issuer.clone())
            && self.settle
            && to == &buyer
            && scope.is_none()
            && WorldAuth { w: self.world }.authorize_title_receipt(&buyer, to, scope)
    }
}

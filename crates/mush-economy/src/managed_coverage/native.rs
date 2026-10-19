//! The registered E4 leg repeats actual financial/title checks. Public request
//! fields are inert data; only exact trusted paired producer grants enter here.
use super::{plans::BindingIntent, *};
use crate::api::WorldAuth;
use crate::contract::{EconomicContract, check_offerable};
use crate::engine::{AuthEval, Delta, NoCustody, NoFacts, ValidateEnv};
use crate::plan::{AssetOp, EconomyPlan, HoldOp, PlanResult, Prepared};
use crate::record::{
    Account, AccountStatus, EconomicAsset, EconomicTransaction, Hold, InstrumentStatus,
};
use crate::{Amount, ContractId, EconomySnapshot, EconomyStore, HoldId, TransactionId};
use mush_core::participant::{OwnerPreparationBinding, OwnerPrepareContext};
use mush_core::{CommitToken, ObjId, World};
use mush_org::{OrgStatus, OrgStore, PrincipalId, ScopeStatus};
use std::any::TypeId;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct CoverageConfig {
    pub(crate) service: ObjId,
    pub(crate) actions: [DefinitionRef; 3],
}
impl CoverageConfig {
    pub fn new(service: ObjId, actions: [DefinitionRef; 3]) -> Result<Self, ParticipantError> {
        let value = Self { service, actions };
        value.validate()?;
        Ok(value)
    }
    pub(crate) fn validate(&self) -> Result<(), ParticipantError> {
        if self.service.0 == 0
            || self
                .actions
                .iter()
                .zip([BIND_ACTION, CLAIM_ACTION, RELEASE_ACTION])
                .any(|(a, id)| !action_matches(a, id))
        {
            return Err(refused());
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct CoverageBindRequest {
    pub basis: CoverageBasisV1,
    pub terms: CoverageQuoteTermsV1,
}
#[derive(Clone, Debug)]
pub struct CoverageTerminalRequest {
    pub binding: CommitToken,
    pub decision: CoverageDecisionV1,
}
#[derive(Clone)]
pub(crate) enum Request {
    Bind(CoverageBindRequest),
    Terminal(CoverageTerminalRequest),
}
/// Deliberately no Deserialize, Clone or public constructor.
pub struct CoverageOutput {
    pub(super) history: CoverageHistory,
}
impl CoverageOutput {
    pub fn history(&self) -> &CoverageHistory {
        &self.history
    }
}

pub(super) struct Preparation {
    pub output: CoverageOutput,
    pub binding: OwnerPreparationBinding,
    pub request: Request,
    pub before_history: Option<CoverageHistory>,
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
pub(crate) struct ClosedCoveragePlan {
    plan: EconomyPlan,
    contract: ContractId,
    before_contract: Option<EconomicContract>,
    hold: HoldId,
    before_hold: Option<Hold>,
    title: EconomicAsset,
}
impl ClosedCoveragePlan {
    pub(crate) fn plan(&self) -> &EconomyPlan {
        &self.plan
    }
    pub(super) fn retained(
        plan: EconomyPlan,
        intent: &BindingIntent,
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
    pub(crate) fn check(
        &self,
        plan: &EconomyPlan,
        snapshot: &EconomySnapshot,
    ) -> Result<(), crate::EconomyError> {
        if plan != &self.plan
            || plan.idempotency.namespace.as_str() != NAMESPACE
            || !matches!(&plan.cause,crate::record::EconomicCause::Ruleset{namespace,..}if namespace.as_str()==NAMESPACE)
            || snapshot.contract(self.contract) != self.before_contract.as_ref()
            || snapshot.hold(self.hold) != self.before_hold.as_ref()
            || snapshot.asset(self.title.object) != Some(&self.title)
            || snapshot.managed_titles.contains(self.title.object)
            || !plan.custody_ops.is_empty()
            || self.before_contract.as_ref().is_some_and(|c| {
                c.consent
                    .managed
                    .as_ref()
                    .and_then(|m| m.hull_coverage_marker())
                    .is_none()
            })
        {
            return Err(crate::EconomyError::ManagedContract);
        }
        Ok(())
    }
}
fn player(world: &World, actor: ObjId) -> bool {
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
        || inv.idempotency().scope().owner.as_str() != "tacsim"
        || !player(ctx.world(), actor)
    {
        return Err(refused());
    }
    Ok(())
}
fn accounts<'a>(
    state: &'a EconomyStore,
    c: &CoverageConsentV1,
) -> Result<(&'a Account, &'a Account), ParticipantError> {
    let t = &c.terms;
    let buyer = state.snapshot.account(t.payer).ok_or_else(refused)?;
    let insurer = state.snapshot.account(t.treasury).ok_or_else(refused)?;
    if buyer.owner != PrincipalId::Object(t.buyer)
        || buyer.scope.is_some()
        || insurer.owner != PrincipalId::Organization(t.insurer)
        || insurer.scope != Some(t.scope)
        || buyer.instrument != t.instrument
        || insurer.instrument != t.instrument
        || buyer.status != AccountStatus::Active
        || insurer.status != AccountStatus::Active
        || state
            .snapshot
            .instrument(&t.instrument)
            .is_none_or(|i| i.status != InstrumentStatus::Active || !i.transferable)
    {
        return Err(refused());
    }
    Ok((buyer, insurer))
}
pub(super) fn current(
    config: &CoverageConfig,
    state: &EconomyStore,
    ctx: &OwnerPrepareContext<'_>,
    c: &CoverageConsentV1,
    decision: Option<&CoverageDecisionV1>,
) -> Result<(), ParticipantError> {
    config.validate()?;
    c.validate()?;
    state.fold.coverage_receipts.require_complete()?;
    let world = ctx.world();
    let now = world.world_ic_now();
    if state.frozen || world.now() < c.accepted_at_utc {
        return Err(refused());
    }
    if let Some(d) = decision {
        d.matches_consent(c)?;
        let cancelled = matches!(d.reason, CoverageDecisionKindV1::Cancelled { .. });
        identity(
            ctx,
            &config.actions[if d.reason.pays() { 1 } else { 2 }],
            if cancelled {
                c.terms.buyer
            } else {
                config.service
            },
        )?;
        if d.operation != ctx.commit_token()
            || d.action != *ctx.action()
            || d.actor != ctx.invocation().actor()
            || now.clock != d.at_ic.clock
            || now.at < d.at_ic.at
            || world.now() < d.posted_at_utc
        {
            return Err(refused());
        }
        // Frozen accepted terms remain payable even after publication/curator
        // rights change. Only actual native recipient coherence still applies.
        if d.reason.pays() {
            accounts(state, c)?;
        }
    } else {
        identity(ctx, &config.actions[0], c.terms.buyer)?;
        if c.operation != ctx.commit_token()
            || now.clock != c.terms.starts_at.clock
            || now.at < c.terms.starts_at.at
            || c.effect != *ctx.action()
            || world.now() >= c.terms.expires_at_utc
            || now.at >= c.terms.ends_at.at
            || c.terms.adjudicator != config.service
            || !player(world, c.terms.adjudicator)
            || world
                .module_state_owner(TypeId::of::<OrgStore>())
                .is_none_or(|o| o.as_str() != "org")
        {
            return Err(refused());
        }
        let orgs = world.module_state::<OrgStore>().ok_or_else(refused)?;
        let org = orgs.state().org(c.terms.insurer).ok_or_else(refused)?;
        let scope = orgs.state().scope(c.terms.scope).ok_or_else(refused)?;
        if org.status != OrgStatus::Active
            || scope.org != org.id
            || scope.status != ScopeStatus::Active
        {
            return Err(refused());
        }
        let (buyer, insurer) = accounts(state, c)?;
        // Never fund the promised liability out of the premium being charged in
        // this same plan. Both pre-images must independently cover their debit.
        if buyer.available().is_none_or(|a| a < c.terms.premium)
            || insurer.available().is_none_or(|a| a < c.terms.liability)
        {
            return Err(refused());
        }
        let auth = WorldAuth { w: world };
        if !auth.authorize(
            &PrincipalId::Object(c.terms.buyer),
            buyer,
            c.terms.premium.checked_neg().ok_or_else(refused)?,
        ) || !auth.authorize(
            &PrincipalId::Organization(c.terms.insurer),
            insurer,
            c.terms.liability.checked_neg().ok_or_else(refused)?,
        ) {
            return Err(refused());
        }
    }
    Ok(())
}

pub(super) fn prepare(
    config: &CoverageConfig,
    state: &EconomyStore,
    ctx: &OwnerPrepareContext<'_>,
    request: Request,
) -> Result<Preparation, ParticipantError> {
    let (
        intent,
        before_history,
        before_contract,
        before_hold,
        before_title,
        decision,
        next_hold,
        next_contract,
    ) = match &request {
        Request::Bind(r) => {
            let consent = CoverageConsentV1 {
                format: 1,
                operation: ctx.commit_token(),
                effect: ctx.action().clone(),
                accepted_at_utc: ctx.world().now(),
                basis: r.basis.clone(),
                terms: r.terms.bind_window(ctx.world().world_ic_now())?,
            };
            current(config, state, ctx, &consent, None)?;
            let contract = ContractId(state.next_contract);
            let hold = HoldId(state.next_hold);
            if state.snapshot.contract(contract).is_some() || state.snapshot.hold(hold).is_some() {
                return Err(refused());
            }
            let title = state
                .snapshot
                .asset(consent.basis.hull)
                .cloned()
                .ok_or_else(refused)?;
            (
                BindingIntent {
                    consent,
                    contract,
                    hold,
                    title: title.clone(),
                },
                None,
                None,
                None,
                title,
                None,
                state.next_hold.checked_add(1).ok_or_else(refused)?,
                state.next_contract.checked_add(1).ok_or_else(refused)?,
            )
        }
        Request::Terminal(r) => {
            let history = state.fold.coverage_receipts.get(r.binding)?.clone();
            if history.terminal().is_some() {
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
                .hold(history.binding().hold())
                .cloned()
                .ok_or_else(refused)?;
            let title = state
                .snapshot
                .asset(history.consent().basis.hull)
                .cloned()
                .ok_or_else(refused)?;
            history.verify_documents(&contract, &hold)?;
            (
                history.intent().clone(),
                Some(history),
                Some(contract),
                Some(hold),
                title,
                Some(r.decision.clone()),
                state.next_hold,
                state.next_contract,
            )
        }
    };
    let plan = match &decision {
        Some(d) => plans::terminal(&intent, d, &before_title)?,
        None => plans::bind(&intent)?,
    };
    let proof = ClosedCoveragePlan::retained(
        plan.clone(),
        &intent,
        before_contract.clone(),
        before_hold.clone(),
        before_title.clone(),
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
        || delta.assets.len() != 1
        || delta.holds.len() != 1
        || !transaction.custody_effects.is_empty()
    {
        return Err(refused());
    }
    let history = state.fold.coverage_receipts.prepare(&transaction)?;
    let contract = history.contract()?;
    if decision.is_none() {
        check_offerable(&contract, &state.snapshot).map_err(|_| refused())?;
    }
    Ok(Preparation {
        output: CoverageOutput { history },
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
        transaction,
        next_hold,
        next_contract,
    })
}

pub(super) fn validate(
    state: &EconomyStore,
    world: &World,
    intent: &BindingIntent,
    decision: Option<&CoverageDecisionV1>,
    hold: Option<&Hold>,
    proof: &ClosedCoveragePlan,
) -> Result<Prepared, ParticipantError> {
    let auth = CoverageAuth {
        world,
        intent,
        decision,
        hold,
        plan: proof.plan(),
    };
    let mut spent = BTreeMap::new();
    // Both buyer premium and insurer reservation retain real approval windows.
    for id in [intent.consent.terms.payer, intent.consent.terms.treasury] {
        if let Some(window) = state
            .snapshot
            .account(id)
            .and_then(|a| a.approval.as_ref())
            .and_then(|a| a.window.as_ref())
        {
            spent.insert(
                id,
                state.spent_in_window(id, world.now(), window.period_secs),
            );
        }
    }
    let gate = state.key_gate();
    let env = ValidateEnv {
        auth: &auth,
        custody: &NoCustody,
        facts: &NoFacts,
        now: world.now(),
        ic_now: world.game_now_unix(),
        next_hold_id: state.next_hold,
        spent,
        keys: &gate,
    };
    match crate::engine::validate_hull_coverage(proof, &state.snapshot, &env) {
        PlanResult::Post(p) => Ok(*p),
        _ => Err(refused()),
    }
}

/// The exact private plan is already fixed by the registered paired owner. This
/// adapter supplies only its accepted insurer reservation and collateral rights;
/// generic account/asset entry points cannot construct it.
struct CoverageAuth<'a> {
    world: &'a World,
    intent: &'a BindingIntent,
    decision: Option<&'a CoverageDecisionV1>,
    hold: Option<&'a Hold>,
    plan: &'a EconomyPlan,
}
impl AuthEval for CoverageAuth<'_> {
    fn authorize(&self, actor: &PrincipalId, account: &Account, delta: Amount) -> bool {
        if actor != &self.plan.actor {
            return false;
        }
        let t = &self.intent.consent.terms;
        if let Some(d) = self.decision {
            return d.reason.pays()
                && delta == t.liability
                && account.id == t.payer
                && account.owner == PrincipalId::Object(t.buyer)
                && account.scope.is_none()
                && account.instrument == t.instrument;
        }
        let ordinary = WorldAuth { w: self.world };
        if account.id == t.payer && delta == t.premium.checked_neg().unwrap_or(Amount::ZERO) {
            return account.owner == PrincipalId::Object(t.buyer)
                && ordinary.authorize(&PrincipalId::Object(t.buyer), account, delta);
        }
        if account.id == t.treasury
            && account.owner == PrincipalId::Organization(t.insurer)
            && account.scope == Some(t.scope)
            && account.instrument == t.instrument
        {
            return (delta == t.premium
                || delta == t.liability.checked_neg().unwrap_or(Amount::ZERO))
                && ordinary.authorize(&PrincipalId::Organization(t.insurer), account, delta);
        }
        false
    }
    fn authorize_hold(&self, actor: &PrincipalId, hold: &Hold, op: &HoldOp) -> bool {
        if actor != &self.plan.actor || self.hold != Some(hold) {
            return false;
        }
        match (self.decision.map(|d| d.reason.pays()), op) {
            (Some(true), HoldOp::Capture { hold: id, amount }) => {
                *id == self.intent.hold && *amount == self.intent.consent.terms.liability
            }
            (Some(false), HoldOp::Release { hold: id }) => *id == self.intent.hold,
            _ => false,
        }
    }
    fn authorize_asset(
        &self,
        actor: &PrincipalId,
        asset: Option<&EconomicAsset>,
        op: &AssetOp,
    ) -> bool {
        actor == &self.plan.actor
            && asset.is_some_and(|a| {
                a.object == self.intent.consent.basis.hull
                    && a.title_owner == PrincipalId::Object(self.intent.consent.terms.buyer)
                    && a.scope.is_none()
            })
            && self.plan.asset_ops.contains(op)
    }
    fn authorize_title_receipt(
        &self,
        actor: &PrincipalId,
        to: &PrincipalId,
        to_scope: Option<mush_org::OrgScopeId>,
    ) -> bool {
        actor == &self.plan.actor
            && self.decision.is_some_and(|d| d.reason.pays())
            && to == &PrincipalId::Organization(self.intent.consent.terms.insurer)
            && to_scope == Some(self.intent.consent.terms.scope)
    }
}

//! Real supplier offer, buyer escrow and exact scheduled refit transitions.
use super::{Error, authority, bad, owner, records::*, runtime::Runtime};
use crate::core_production::{
    CoreOriginMode, DieClass,
    participant::{OriginParticipant, Readiness, Request as OwnerRequest},
};
use crate::workorder::{WorkOrderId, derive_work_order_id, wo_holder};
use mush_core::action::{ActionError, ActionErrorCode, ActionProvider, AuthoritativeActionOutput};
use mush_core::participant::ParticipantHandle;
use mush_core::read_service::OwnerReadHandle;
use mush_core::{
    ActionInvocation, ActionTxn, CommitToken, DefinitionRef, ModError, ObjId, PerceivedRef,
    PrepareCtx, Registrar, RulesValue,
};
use mush_economy::{
    AccountId,
    core_refit::{RefitCancellationRequest, RefitFundingRequest, RefitInstallationRequest},
    registry_filing::{EconomyRequest, InitialFilingParticipant as EconomyParticipant},
};
use mush_tactics::component_replacement::{ComponentReplacementParticipant, ReplacementRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OfferSelector {
    requester: ObjId,
    hull: ObjId,
    incoming: ObjId,
    manufacture: CommitToken,
    class: DieClass,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptSelector {
    offer: CommitToken,
    payer: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OrderSelector {
    order: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AdvanceSelector {
    pub(super) order: WorkOrderId,
    pub(super) observed: mush_core::ClockStamp,
    pub(super) restart: bool,
}
fn decode<T: Serialize + serde::de::DeserializeOwned>(v: &Value) -> Result<T, Error> {
    mush_core::canonical_json_bytes_with_limit(v, 2048).map_err(bad)?;
    let s: T = serde_json::from_value(v.clone()).map_err(bad)?;
    if serde_json::to_value(&s).map_err(bad)? != *v {
        return Err(Error::Invalid);
    }
    Ok(s)
}
fn order_id(s: &str) -> Result<WorkOrderId, Error> {
    let id = WorkOrderId::parse(s).ok_or(Error::Invalid)?;
    if id.to_string() != s {
        return Err(Error::Invalid);
    }
    Ok(id)
}
fn payer(s: &str) -> Result<AccountId, Error> {
    let id = s.parse::<u64>().map_err(bad)?;
    if id.to_string() != s {
        return Err(Error::Invalid);
    }
    Ok(AccountId(id))
}
fn validate(inv: &ActionInvocation, index: usize) -> Result<(), Error> {
    if (index != 3 && inv.principal() != &(mush_core::PrincipalId::Object { id: inv.actor() }))
        || inv.subject() != &PerceivedRef::SelfActor
        || !inv.targets().is_empty()
        || inv.idempotency().scope().owner.as_str() != crate::NS
    {
        return Err(Error::Invalid);
    }
    match index {
        0 => {
            if inv.parameters() != &json!({}) {
                return Err(Error::Invalid);
            }
        }
        1 => {
            decode::<OfferSelector>(inv.parameters())?;
        }
        2 => {
            payer(&decode::<AcceptSelector>(inv.parameters())?.payer)?;
        }
        3 => {
            decode::<AdvanceSelector>(inv.parameters())?;
        }
        4 | 5 => {
            order_id(&decode::<OrderSelector>(inv.parameters())?.order)?;
        }
        _ => return Err(Error::Invalid),
    }
    Ok(())
}
pub(crate) struct Provider {
    runtime: Arc<Runtime>,
    index: usize,
    owner: Option<ParticipantHandle<OriginParticipant>>,
    economy: Option<ParticipantHandle<EconomyParticipant>>,
    tactics:
        Option<ParticipantHandle<ComponentReplacementParticipant<super::RefitReplacementPolicy>>>,
    readiness: OwnerReadHandle<Readiness>,
    refusal: ActionError,
}
type PreparedTransition = (&'static str, Option<CommitToken>, Option<WorkOrderId>);

impl Provider {
    #[inline(never)]
    fn prepare_initialize(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<PreparedTransition, Error> {
        let r = &self.runtime;
        let offer_id = None;
        let order = None;
        let state = {
            if r.config.mode != CoreOriginMode::Initialize
                || inv.actor() != mush_core::GOD
                || txn.get(&r.families.deployment, &()).map_err(bad)?.is_some()
                || !txn.family_is_empty(&r.families.offers).map_err(bad)?
                || !txn.family_is_empty(&r.families.orders).map_err(bad)?
                || !txn.family_is_empty(&r.families.assets).map_err(bad)?
                || !txn.family_is_empty(&r.families.hulls).map_err(bad)?
            {
                return Err(Error::Unavailable);
            }
            let out = self.owner(ctx, owner::Request::Initialize)?;
            if out.order.is_some() {
                return Err(Error::Integrity);
            }
            txn.insert(&r.families.deployment, &(), out.index)
                .map_err(bad)?;
            "initialized"
        };
        Ok((state, offer_id, order))
    }

    #[inline(never)]
    fn prepare_offer(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
        commit: CommitToken,
        action: &DefinitionRef,
    ) -> Result<PreparedTransition, Error> {
        let r = &self.runtime;
        let offer_id;
        let order = None;
        let state = {
            let s: OfferSelector = decode(inv.parameters())?;
            let index = txn
                .get(&r.families.deployment, &())
                .map_err(bad)?
                .cloned()
                .ok_or(Error::Unavailable)?;
            index.validate()?;
            if index.offers >= MAX_ORDERS
                || txn.get(&r.families.offers, &commit).map_err(bad)?.is_some()
            {
                return Err(Error::Capacity);
            }
            let live = authority::current(
                ctx.world().map_err(bad)?,
                &r.config,
                s.requester,
                s.hull,
                s.incoming,
                s.manufacture,
                s.class,
            )?;
            let row = RefitOffer {
                format: 1,
                sequence: index.offers.checked_add(1).ok_or(Error::Capacity)?,
                operation: commit,
                action: action.clone(),
                operator: inv.actor(),
                requester: s.requester,
                policy: r.config.payment.clone(),
                facility: live.facility,
                recipe: live.recipe,
                core_acceptance: s.manufacture,
                incoming: s.incoming,
                hull: s.hull,
                unit_revision: live.unit_revision,
                before: live.before,
                component: r.config.component.clone(),
                displaced: live.displaced,
                observed_ic: live.at,
                observed_utc: live.utc,
                expires_utc: live.utc.checked_add(OFFER_SECONDS).ok_or(Error::Capacity)?,
            };
            row.validate()?;
            let out = self.owner(ctx, owner::Request::Offer(Box::new(row.clone())))?;
            if out.index != index.offer(&row)? || out.order.is_some() {
                return Err(Error::Integrity);
            }
            txn.insert(&r.families.offers, &commit, row).map_err(bad)?;
            txn.replace(&r.families.deployment, &(), out.index)
                .map_err(bad)?;
            offer_id = Some(commit);
            "offered"
        };
        Ok((state, offer_id, order))
    }

    #[inline(never)]
    fn prepare_accept(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
        commit: CommitToken,
        action: &DefinitionRef,
    ) -> Result<PreparedTransition, Error> {
        let r = &self.runtime;
        let offer_id;
        let order;
        let state = {
            let s: AcceptSelector = decode(inv.parameters())?;
            let offer = txn
                .get(&r.families.offers, &s.offer)
                .map_err(bad)?
                .cloned()
                .ok_or(Error::Unavailable)?;
            if offer.requester != inv.actor() {
                return Err(Error::Unavailable);
            }
            let index = txn
                .get(&r.families.deployment, &())
                .map_err(bad)?
                .cloned()
                .ok_or(Error::Unavailable)?;
            index.validate()?;
            let id = derive_work_order_id(inv.request(), 0).map_err(bad)?;
            if index.accepted >= MAX_ORDERS
                || txn.get(&r.families.orders, &id).map_err(bad)?.is_some()
                || txn
                    .get(&r.industry.work_orders, &id)
                    .map_err(bad)?
                    .is_some()
                || txn.get(&r.manufacture.orders, &id).map_err(bad)?.is_some()
            {
                return Err(Error::Capacity);
            }
            let at = ctx.world().map_err(bad)?.world_ic_now();
            let intent = AcceptedIntent {
                order: id,
                offer,
                payer: payer(&s.payer)?,
                accepted_at: at.clone(),
                expires_at: mush_inventory::reservation_deadline(&at).map_err(bad)?,
            };
            authority::accept(ctx.world().map_err(bad)?, &r.config, &intent)?;
            let capacity = r
                .industry
                .reserve_slot(
                    txn,
                    intent.offer.policy.factory,
                    intent.offer.facility.maximum_slots(),
                    &wo_holder(id).map_err(bad)?,
                    &at,
                )
                .map_err(bad)?;
            let payment = ctx
                .prepare_owner(
                    self.economy.as_ref().ok_or(Error::Unavailable)?,
                    EconomyRequest::RefitFunding(RefitFundingRequest {
                        subject: intent.subject()?,
                    }),
                )
                .map_err(bad)?
                .refit_funding()
                .map_err(bad)?;
            let row = RefitOrder {
                format: 1,
                sequence: index.accepted.checked_add(1).ok_or(Error::Capacity)?,
                accepted: commit,
                action: action.clone(),
                progress: super::progress::RefitProgress::new(
                    id,
                    at,
                    intent.offer.recipe.work_units(),
                )?,
                intent,
                funding: payment.receipt().clone(),
                capacity,
                terminal: None,
                installation: None,
            };
            row.validate()?;
            super::capacity::terminal_room(ctx.world().map_err(bad)?, r, &row)?;
            let out = self.owner(
                ctx,
                owner::Request::Accept {
                    order: Box::new(row.clone()),
                    payment: Box::new(payment),
                },
            )?;
            if out.index != index.accept(&row)? || out.order.as_ref() != Some(&row) {
                return Err(Error::Integrity);
            }
            r.families.accept_heads(txn, &row)?;
            r.families.publish_schedule(txn, &row)?;
            txn.insert(&r.families.orders, &id, row).map_err(bad)?;
            txn.replace(&r.families.deployment, &(), out.index)
                .map_err(bad)?;
            ctx.reserve_core_refit_completion(txn, id.0).map_err(bad)?;
            offer_id = Some(s.offer);
            order = Some(id);
            "accepted"
        };
        Ok((state, offer_id, order))
    }

    #[inline(never)]
    fn prepare_advance(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<PreparedTransition, Error> {
        let r = &self.runtime;
        let offer_id = None;
        let order;
        let state = {
            let s: AdvanceSelector = decode(inv.parameters())?;
            let before = self.before(txn, s.order, inv.actor())?;
            if s.observed != ctx.world().map_err(bad)?.world_ic_now() {
                return Err(Error::Unavailable);
            }
            let cursor = before.progress.cursor();
            let mush_core::ScheduleEvaluation::Advance(advance) = cursor
                .plan_due(
                    cursor.revision(),
                    &s.observed,
                    if s.restart {
                        mush_core::ScheduleDriveCause::Restart
                    } else {
                        mush_core::ScheduleDriveCause::Live
                    },
                )
                .map_err(bad)?
            else {
                return Err(Error::Unavailable);
            };
            let out = self.owner(
                ctx,
                owner::Request::Advance {
                    before: Box::new(before),
                    advance,
                },
            )?;
            let row = out.order.ok_or(Error::Integrity)?;
            // Core owns the schedule advance; the owner's reproduced
            // progress must equal its already seeded typed postimage.
            if txn
                .get(
                    &r.families.schedules,
                    &super::progress::schedule_id(s.order)?,
                )
                .map_err(bad)?
                != Some(row.progress.cursor())
            {
                return Err(Error::Integrity);
            }
            txn.replace(&r.families.orders, &s.order, row)
                .map_err(bad)?;
            ctx.advance_core_refit_completion(txn, s.order.0)
                .map_err(bad)?;
            order = Some(s.order);
            "advanced"
        };
        Ok((state, offer_id, order))
    }

    #[inline(never)]
    fn prepare_install(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<PreparedTransition, Error> {
        let r = &self.runtime;
        let offer_id = None;
        let order;
        let state = {
            let id = order_id(&decode::<OrderSelector>(inv.parameters())?.order)?;
            let before = self.before(txn, id, inv.actor())?;
            if !before.progress.production().is_complete() {
                return Err(Error::Unavailable);
            }
            // Both actual owners prepare before the closed Core physical image. Industry
            // publication below is typed data; a third native image is neither requested
            // nor permitted by the Core exchange protocol.
            let tactics = ctx
                .prepare_owner(
                    self.tactics.as_ref().ok_or(Error::Unavailable)?,
                    ReplacementRequest::new(
                        before.intent.offer.hull,
                        before.intent.offer.unit_revision,
                        id,
                    ),
                )
                .map_err(bad)?;
            let payment = ctx
                .prepare_owner(
                    self.economy.as_ref().ok_or(Error::Unavailable)?,
                    EconomyRequest::RefitInstallation(RefitInstallationRequest {
                        acceptance: before.accepted,
                    }),
                )
                .map_err(bad)?
                .refit_terminal()
                .map_err(bad)?;
            let physical = ctx
                .exchange_core_component(&r.exchanges, txn, id.0)
                .map_err(bad)?;
            let row = before.install(&payment, &tactics, &physical)?;
            r.industry
                .release_slot(txn, row.intent.offer.policy.factory, row.capacity, 1)
                .map_err(bad)?;
            r.families.terminal_heads(txn, &row)?;
            txn.replace(&r.families.orders, &id, row).map_err(bad)?;
            ctx.advance_core_refit_completion(txn, id.0).map_err(bad)?;
            order = Some(id);
            "installed"
        };
        Ok((state, offer_id, order))
    }

    #[inline(never)]
    fn prepare_cancel(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<PreparedTransition, Error> {
        let r = &self.runtime;
        let offer_id = None;
        let order;
        let state = {
            let id = order_id(&decode::<OrderSelector>(inv.parameters())?.order)?;
            let before = self.before(txn, id, inv.actor())?;
            let payment = ctx
                .prepare_owner(
                    self.economy.as_ref().ok_or(Error::Unavailable)?,
                    EconomyRequest::RefitCancellation(RefitCancellationRequest {
                        acceptance: before.accepted,
                    }),
                )
                .map_err(bad)?
                .refit_terminal()
                .map_err(bad)?;
            let out = self.owner(
                ctx,
                owner::Request::Cancel {
                    before: Box::new(before),
                    payment: Box::new(payment),
                },
            )?;
            let row = out.order.ok_or(Error::Integrity)?;
            r.industry
                .release_slot(txn, row.intent.offer.policy.factory, row.capacity, 1)
                .map_err(bad)?;
            r.families.terminal_heads(txn, &row)?;
            txn.replace(&r.families.orders, &id, row).map_err(bad)?;
            ctx.advance_core_refit_completion(txn, id.0).map_err(bad)?;
            order = Some(id);
            "cancelled"
        };
        Ok((state, offer_id, order))
    }

    fn action(&self) -> Result<&DefinitionRef, Error> {
        self.runtime.actions.get(self.index).ok_or(Error::Integrity)
    }
    fn owner(
        &self,
        ctx: &mut PrepareCtx<'_>,
        request: owner::Request,
    ) -> Result<Box<owner::Output>, Error> {
        ctx.prepare_owner(
            self.owner.as_ref().ok_or(Error::Unavailable)?,
            OwnerRequest::Refit(request),
        )
        .map_err(bad)?
        .refit()
        .map_err(bad)
    }
    fn before(
        &self,
        txn: &mut ActionTxn<'_>,
        id: WorkOrderId,
        actor: ObjId,
    ) -> Result<RefitOrder, Error> {
        let row = txn
            .get(&self.runtime.families.orders, &id)
            .map_err(bad)?
            .cloned()
            .ok_or(Error::Unavailable)?;
        row.validate()?;
        if row.intent.offer.requester != actor || row.terminal.is_some() {
            return Err(Error::Unavailable);
        }
        Ok(row)
    }
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, Error> {
        validate(inv, self.index)?;
        let r = &self.runtime;
        if self.index != 0 && self.index != 5 && r.config.mode != CoreOriginMode::Active {
            return Err(Error::Unavailable);
        }
        // Actual K ingress uses its scoped scheduler identity, proved by the owner.
        if self.index != 3 {
            ctx.read_owner_preimage(txn, &self.readiness, ())
                .map_err(bad)?;
        }
        let commit = ctx.commit_token();
        let action = self.action()?;
        let (state, offer_id, order) = match self.index {
            0 => self.prepare_initialize(inv, txn, ctx)?,
            1 => self.prepare_offer(inv, txn, ctx, commit, action)?,
            2 => self.prepare_accept(inv, txn, ctx, commit, action)?,
            3 => self.prepare_advance(inv, txn, ctx)?,
            4 => self.prepare_install(inv, txn, ctx)?,
            5 => self.prepare_cancel(inv, txn, ctx)?,
            _ => return Err(Error::Invalid),
        };
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(
                r.outcome.clone(),
                json!({"operation":commit,"offer":offer_id,"order":order,"state":state}),
            )
            .map_err(bad)?,
        )
        .with_definitions(vec![action.clone(), r.outcome.clone()])
        .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        validate(inv, self.index).map_err(|_| self.refusal.clone())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.prepare(inv, txn, ctx)
            .map_err(|_| self.refusal.clone())
    }
    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        use mush_core::{ProjectedParameter, ProjectedParameterKind as Kind};
        let r = &self.runtime;
        if self.action().ok() != Some(action)
            || self.index == 3
            || query.truth_subject() != &(mush_core::SubjectKey::Object { id: query.actor() })
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let Some(projection) = engine
            .world
            .module_state::<crate::core_production::participant::CoreProductionState>()
            .filter(|s| s.ready)
            .and_then(|s| s.refit.as_ref())
            .filter(|p| p.ready)
        else {
            return Ok(None);
        };
        if authority::cleanup(&engine.world, &r.config).is_err()
            || (self.index != 0 && self.index != 5 && r.config.mode != CoreOriginMode::Active)
        {
            return Ok(None);
        }
        let fail = || self.refusal.clone();
        let parameter = |name: &str, kind| -> Result<ProjectedParameter, ActionError> {
            Ok(ProjectedParameter {
                id: mush_core::CanonicalId::new(name).map_err(|_| fail())?,
                required: true,
                kind,
            })
        };
        let mut parameters = Vec::new();
        match self.index {
            0 => {
                if query.actor() != mush_core::GOD
                    || r.config.mode != CoreOriginMode::Initialize
                    || projection.index.is_some()
                {
                    return Ok(None);
                }
            }
            1 => {
                if projection
                    .index
                    .as_ref()
                    .is_none_or(|i| i.offers >= MAX_ORDERS)
                    || authority::allow(&engine.world, &r.config, query.actor()).is_err()
                {
                    return Ok(None);
                }
                let mut classes = Vec::new();
                for class in DieClass::ALL {
                    let Some(recipe) = r.config.recipes.resolve(&engine.world, class) else {
                        continue;
                    };
                    if recipe.validate().is_err()
                        || recipe.definition() != &r.config.payment.recipe
                        || r.config
                            .recipes
                            .published_digest(&engine.world, recipe.pack())
                            != Some(recipe.pack().digest())
                    {
                        continue;
                    }
                    classes.push(
                        serde_json::to_value(class)
                            .map_err(|_| fail())?
                            .as_str()
                            .ok_or_else(fail)?
                            .to_owned(),
                    );
                }
                if classes.is_empty() {
                    return Ok(None);
                }
                parameters.push(parameter("class", Kind::Choice { choices: classes })?);
                for name in ["requester", "hull", "incoming"] {
                    parameters.push(parameter(
                        name,
                        Kind::Integer {
                            minimum: 0,
                            maximum: i64::from(u32::MAX),
                        },
                    )?);
                }
                parameters.push(parameter("manufacture", Kind::Text { maximum_bytes: 32 })?);
            }
            2 => {
                if projection
                    .index
                    .as_ref()
                    .is_none_or(|i| i.accepted >= MAX_ORDERS)
                {
                    return Ok(None);
                }
                let mut offers = Vec::new();
                for row in projection.offers.values() {
                    if row.requester != query.actor()
                        || row
                            .require_time(&engine.world.world_ic_now(), engine.world.now())
                            .is_err()
                        || projection
                            .orders
                            .values()
                            .any(|o| o.intent.offer.operation == row.operation)
                        || authority::offer(&engine.world, &r.config, row).is_err()
                    {
                        continue;
                    }
                    offers.push(row.operation.to_hex());
                    if offers.len() == 64 {
                        break;
                    }
                }
                let Some(economy) = engine.world.module_state::<mush_economy::EconomyStore>()
                else {
                    return Ok(None);
                };
                let principal = mush_org::PrincipalId::Object(query.actor());
                let accounts = economy
                    .snapshot()
                    .accounts_of(&principal)
                    .filter(|a| {
                        a.scope.is_none()
                            && a.status == mush_economy::AccountStatus::Active
                            && a.approval.is_none()
                            && a.instrument == r.config.payment.instrument
                    })
                    .take(64)
                    .map(|a| a.id.0.to_string())
                    .collect::<Vec<_>>();
                if offers.is_empty() || accounts.is_empty() {
                    return Ok(None);
                }
                parameters.push(parameter("offer", Kind::Choice { choices: offers })?);
                parameters.push(parameter("payer", Kind::Choice { choices: accounts })?);
            }
            4 | 5 => {
                let mut orders = Vec::new();
                for (id, view) in &projection.orders {
                    if view.intent.offer.requester != query.actor() {
                        continue;
                    }
                    let Some(row) = engine
                        .action_state()
                        .get(&r.families.orders, id)
                        .map_err(|_| fail())?
                    else {
                        return Err(fail());
                    };
                    // Install publishes typed/native terminal evidence with exactly the
                    // Economy and Tactics images. The older Industry projection may
                    // still retain its ready preimage until cold reconstruction.
                    if row.terminal.is_some() {
                        continue;
                    }
                    if row != view.as_ref() {
                        return Err(fail());
                    }
                    if self.index == 4
                        && (!row.progress.production().is_complete()
                            || authority::install(&engine.world, &r.config, &row.intent).is_err())
                    {
                        continue;
                    }
                    orders.push(id.to_string());
                    if orders.len() == 64 {
                        break;
                    }
                }
                if orders.is_empty() {
                    return Ok(None);
                }
                parameters.push(parameter("order", Kind::Choice { choices: orders })?);
            }
            _ => return Ok(None),
        }
        let definitions = engine.registry.definitions().ok_or_else(fail)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| fail())?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| fail())?;
        if self.index != 5 {
            guards
                .add_definition(r.config.payment.recipe.clone(), definitions)
                .map_err(|_| fail())?;
        }
        let (id, label) = match self.index {
            0 => ("initialize-core-refit", "Initialize core refitting"),
            1 => ("offer-core-refit", "Offer core refit"),
            2 => ("accept-core-refit", "Accept core refit"),
            4 => ("install-core-component", "Install completed core refit"),
            5 => ("cancel-core-refit", "Cancel core refit"),
            _ => return Ok(None),
        };
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: json!({}),
            parameters: mush_core::ProjectedParameterSchema::new(parameters).map_err(|_| fail())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: vec![],
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                id,
                label,
                "Use the acknowledged supplier offer and exact refit work order.",
            )
            .map_err(|_| fail())?,
            guards: guards.finish().map_err(|_| fail())?,
        }))
    }
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    runtime: Runtime,
    readiness: OwnerReadHandle<Readiness>,
) -> Result<(), ModError> {
    let runtime = Arc::new(runtime);
    for (index, action) in runtime.actions.iter().cloned().enumerate() {
        let owner = if index == 4 {
            None
        } else {
            Some(reg.request_owner_participant::<OriginParticipant>(action.clone())?)
        };
        let economy = if matches!(index, 2 | 4 | 5) {
            Some(reg.request_owner_participant::<EconomyParticipant>(action.clone())?)
        } else {
            None
        };
        let tactics = if index == 4 {
            Some(reg.request_owner_participant::<ComponentReplacementParticipant<super::RefitReplacementPolicy>>(action.clone())?)
        } else {
            None
        };
        reg.action_provider(
            action,
            Provider {
                runtime: runtime.clone(),
                index,
                owner,
                economy,
                tactics,
                readiness: readiness.clone(),
                refusal: ActionError::new(
                    ActionErrorCode::ProviderRejected,
                    false,
                    "Core refit is unavailable or changed.",
                )
                .map_err(|e| ModError::msg(e.to_string()))?,
            },
        )?;
    }
    Ok(())
}

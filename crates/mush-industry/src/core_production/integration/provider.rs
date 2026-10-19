//! Actual operator offer, buyer acceptance and fixed order transitions.
//! No decoded receipt or caller postimage can replace an owner output.
use super::{
    IntegrationError as Error, authority, bad,
    offer::{FactoryOffer, OFFER_SECONDS, OfferedInput},
    owner,
    records::*,
    runtime::Runtime,
};
use crate::core_production::records::DieCertificate;
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
    core_integration::{CoreCancellationRequest, CoreCompletionRequest, CoreFundingRequest},
    registry_filing::{EconomyRequest, InitialFilingParticipant as EconomyParticipant},
};
use mush_inventory::{Lineage, Lot, LotId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OfferSelector {
    requester: ObjId,
    class: DieClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    supplier_lot: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptSelector {
    offer: CommitToken,
    payer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_lot: Option<String>,
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
fn decode<T: Serialize + serde::de::DeserializeOwned>(value: &Value) -> Result<T, Error> {
    mush_core::canonical_json_bytes_with_limit(value, 2048).map_err(bad)?;
    let row: T = serde_json::from_value(value.clone()).map_err(bad)?;
    if serde_json::to_value(&row).map_err(bad)? != *value {
        return Err(Error::Invalid);
    }
    Ok(row)
}
fn lot_id(text: &str) -> Result<LotId, Error> {
    let id = LotId::parse(text).ok_or(Error::Invalid)?;
    if id.to_string() != text {
        return Err(Error::Invalid);
    }
    Ok(id)
}
fn parse_order_id(text: &str) -> Result<WorkOrderId, Error> {
    let id = WorkOrderId::parse(text).ok_or(Error::Invalid)?;
    if id.to_string() != text {
        return Err(Error::Invalid);
    }
    Ok(id)
}
fn payer_id(text: &str) -> Result<AccountId, Error> {
    let id = text.parse::<u64>().map_err(bad)?;
    if id.to_string() != text {
        return Err(Error::Invalid);
    }
    Ok(AccountId(id))
}
fn identity(inv: &ActionInvocation, advance: bool) -> Result<(), Error> {
    if (!advance && inv.principal() != &(mush_core::PrincipalId::Object { id: inv.actor() }))
        || inv.subject() != &PerceivedRef::SelfActor
        || !inv.targets().is_empty()
        || inv.idempotency().scope().owner.as_str() != crate::NS
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn validate(inv: &ActionInvocation, index: usize) -> Result<(), Error> {
    identity(inv, index == 4)?;
    match index {
        0 => {
            if inv.parameters() != &json!({}) {
                return Err(Error::Invalid);
            }
        }
        1 => {
            let s = decode::<OfferSelector>(inv.parameters())?;
            if let Some(id) = s.supplier_lot {
                lot_id(&id)?;
            }
        }
        2 => {
            let s = decode::<AcceptSelector>(inv.parameters())?;
            payer_id(&s.payer)?;
            if let Some(id) = s.input_lot {
                lot_id(&id)?;
            }
        }
        4 => {
            decode::<AdvanceSelector>(inv.parameters())?;
        }
        3 | 5 | 6 => {
            parse_order_id(&decode::<OrderSelector>(inv.parameters())?.order)?;
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
    readiness: OwnerReadHandle<Readiness>,
    refusal: ActionError,
}
impl Provider {
    fn owner(
        &self,
        ctx: &mut PrepareCtx<'_>,
        request: owner::Request,
    ) -> Result<owner::Output, Error> {
        ctx.prepare_owner(
            self.owner.as_ref().ok_or(Error::Unavailable)?,
            OwnerRequest::Integration(request),
        )
        .map_err(bad)?
        .integration()
        .map_err(bad)
    }
    fn certificate(
        &self,
        txn: &mut ActionTxn<'_>,
        id: LotId,
    ) -> Result<(DieCertificate, Lot), Error> {
        let lot = txn
            .get(&self.runtime.industry.inventory.lots, &id)
            .map_err(bad)?
            .cloned()
            .ok_or(Error::Unavailable)?;
        let Lineage::Origin { source, .. } = lot.lineage() else {
            return Err(Error::Invalid);
        };
        let c = txn
            .get(&self.runtime.origins.certificates, &source.id.to_string())
            .map_err(bad)?
            .cloned()
            .ok_or(Error::Unavailable)?;
        c.matches_lot(&lot).map_err(bad)?;
        Ok((c, lot))
    }
    fn before(
        &self,
        txn: &mut ActionTxn<'_>,
        order: WorkOrderId,
        actor: ObjId,
    ) -> Result<CoreIntegrationOrder, Error> {
        let before = txn
            .get(&self.runtime.families.orders, &order)
            .map_err(bad)?
            .cloned()
            .ok_or(Error::Unavailable)?;
        before.validate()?;
        if before.intent.requester != actor || before.terminal.is_some() {
            return Err(Error::Unavailable);
        }
        Ok(before)
    }
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, Error> {
        validate(inv, self.index)?;
        let r = &self.runtime;
        if self.index != 0 && self.index != 6 && r.config.mode != CoreOriginMode::Active {
            return Err(Error::Unavailable);
        }
        ctx.read_owner_preimage(txn, &self.readiness, ())
            .map_err(bad)?;
        let commit = ctx.commit_token();
        let action = r.actions.get(self.index).ok_or(Error::Integrity)?;
        let mut offer_id = None;
        let mut order_id = None;
        let mut object = None;
        let state = match self.index {
            0 => {
                if r.config.mode != CoreOriginMode::Initialize
                    || inv.actor() != mush_core::GOD
                    || txn.get(&r.families.deployment, &()).map_err(bad)?.is_some()
                    || !txn.family_is_empty(&r.families.offers).map_err(bad)?
                    || !txn.family_is_empty(&r.families.orders).map_err(bad)?
                {
                    return Err(Error::Unavailable);
                }
                // Core's empty-history image must be prepared before the owner image. It
                // creates no object; both exact deployment markers share this ACK.
                ctx.initialize_core_outputs(&r.outputs, txn).map_err(bad)?;
                let after = self.owner(ctx, owner::Request::Initialize)?;
                if after.order.is_some() || after.input.is_some() {
                    return Err(Error::Integrity);
                }
                txn.insert(&r.families.deployment, &(), after.index)
                    .map_err(bad)?;
                "initialized"
            }
            1 => {
                let selected: OfferSelector = decode(inv.parameters())?;
                let index = txn
                    .get(&r.families.deployment, &())
                    .map_err(bad)?
                    .cloned()
                    .ok_or(Error::Unavailable)?;
                index.validate()?;
                if index.offered >= MAX_ORDERS
                    || txn.get(&r.families.offers, &commit).map_err(bad)?.is_some()
                {
                    return Err(Error::Capacity);
                }
                let input = match selected.supplier_lot {
                    Some(id) => {
                        let (certificate, lot) = self.certificate(txn, lot_id(&id)?)?;
                        OfferedInput::SupplierStock { certificate, lot }
                    }
                    None => OfferedInput::RequesterSupplied {},
                };
                let live = authority::offer(
                    ctx.world().map_err(bad)?,
                    &r.definitions,
                    &r.config,
                    inv.actor(),
                    selected.requester,
                    selected.class,
                    &input,
                )?;
                let row = FactoryOffer {
                    format: 1,
                    sequence: index.offered.checked_add(1).ok_or(Error::Capacity)?,
                    operation: commit,
                    action: action.clone(),
                    operator: inv.actor(),
                    requester: selected.requester,
                    policy: r.config.payment.clone(),
                    facility: live.facility,
                    recipe: live.recipe,
                    input,
                    observed_ic: live.at,
                    observed_utc: live.utc,
                    expires_utc: live.utc.checked_add(OFFER_SECONDS).ok_or(Error::Capacity)?,
                };
                row.validate()?;
                let after = self.owner(ctx, owner::Request::Offer(row.clone()))?;
                if after.index != index.offer(&row)?
                    || after.order.is_some()
                    || after.input.is_some()
                {
                    return Err(Error::Integrity);
                }
                txn.insert(&r.families.offers, &commit, row).map_err(bad)?;
                txn.replace(&r.families.deployment, &(), after.index)
                    .map_err(bad)?;
                offer_id = Some(commit);
                "offered"
            }
            2 => {
                let selected: AcceptSelector = decode(inv.parameters())?;
                let offer = txn
                    .get(&r.families.offers, &selected.offer)
                    .map_err(bad)?
                    .cloned()
                    .ok_or(Error::Unavailable)?;
                offer.validate()?;
                if offer.requester != inv.actor() {
                    return Err(Error::Unavailable);
                }
                let (certificate, lot) = match (&offer.input, selected.input_lot) {
                    (OfferedInput::SupplierStock { certificate, lot }, None) => {
                        let (c, current) = self.certificate(txn, lot.id())?;
                        if &c != certificate || &current != lot {
                            return Err(Error::Unavailable);
                        }
                        (c, current)
                    }
                    (OfferedInput::RequesterSupplied {}, Some(id)) => {
                        self.certificate(txn, lot_id(&id)?)?
                    }
                    _ => return Err(Error::Invalid),
                };
                let index = txn
                    .get(&r.families.deployment, &())
                    .map_err(bad)?
                    .cloned()
                    .ok_or(Error::Unavailable)?;
                let order = derive_work_order_id(inv.request(), 0).map_err(bad)?;
                if index.accepted >= MAX_ORDERS
                    || txn.get(&r.families.orders, &order).map_err(bad)?.is_some()
                    || txn
                        .get(&r.industry.work_orders, &order)
                        .map_err(bad)?
                        .is_some()
                {
                    return Err(Error::Capacity);
                }
                let at = ctx.world().map_err(bad)?.world_ic_now();
                let intent = AcceptedIntent {
                    format: 1,
                    order,
                    offer: offer.operation,
                    offer_digest: offer.digest()?,
                    requester: inv.actor(),
                    payer: payer_id(&selected.payer)?,
                    policy: offer.policy.clone(),
                    facility: offer.facility.clone(),
                    recipe: offer.recipe.clone(),
                    certificate,
                    lot,
                    accepted_at: at.clone(),
                    expires_at: mush_inventory::reservation_deadline(&at).map_err(bad)?,
                };
                intent.matches_offer(&offer)?;
                authority::accept(
                    ctx.world().map_err(bad)?,
                    &r.definitions,
                    &r.config,
                    &offer,
                    &intent,
                )?;
                let capacity = r
                    .industry
                    .reserve_slot(
                        txn,
                        intent.policy.factory,
                        intent.facility.maximum_slots(),
                        &wo_holder(order).map_err(bad)?,
                        &at,
                    )
                    .map_err(bad)?;
                let payment = ctx
                    .prepare_owner(
                        self.economy.as_ref().ok_or(Error::Unavailable)?,
                        EconomyRequest::CoreFunding(CoreFundingRequest {
                            subject: intent.subject()?,
                        }),
                    )
                    .map_err(bad)?
                    .core_funding()
                    .map_err(bad)?;
                let row = CoreIntegrationOrder {
                    format: 1,
                    sequence: index.accepted.checked_add(1).ok_or(Error::Capacity)?,
                    progress: super::progress::CoreProgress::new(
                        order,
                        at,
                        intent.recipe.work_units(),
                    )?,
                    intent,
                    accepted: commit,
                    action: action.clone(),
                    funding: payment.receipt().clone(),
                    capacity,
                    output: None,
                    terminal: None,
                };
                row.validate()?;
                let after = self.owner(
                    ctx,
                    owner::Request::Accept {
                        order: row.clone(),
                        payment,
                    },
                )?;
                if after.order.as_ref() != Some(&row) || after.index != index.accept(&row)? {
                    return Err(Error::Integrity);
                }
                let witness = after.input.ok_or(Error::Integrity)?;
                let input = r.inputs.publish(txn, ctx, &witness).map_err(bad)?;
                row.verify_input(&input)?;
                txn.insert(&r.families.orders, &order, row.clone())
                    .map_err(bad)?;
                txn.replace(&r.families.deployment, &(), after.index)
                    .map_err(bad)?;
                r.families.publish_schedule(txn, &row)?;
                ctx.reserve_core_integration_completion(txn, order.0)
                    .map_err(bad)?;
                offer_id = Some(offer.operation);
                order_id = Some(order);
                "accepted"
            }
            3 => {
                let selected: OrderSelector = decode(inv.parameters())?;
                let selected_order = parse_order_id(&selected.order)?;
                let mut before = self.before(txn, selected_order, inv.actor())?;
                let output = ctx
                    .reserve_core_output(&r.outputs, txn, selected_order.0)
                    .map_err(bad)?;
                let reserved = ReservedOutput::capture(&output, &before.intent, commit, action)?;
                if before.output.as_ref().is_some_and(|old| old != &reserved) {
                    return Err(Error::Integrity);
                }
                object = Some(output.object());
                before.output = Some(reserved);
                before.validate()?;
                txn.replace(&r.families.orders, &selected_order, before)
                    .map_err(bad)?;
                ctx.advance_core_integration_completion(txn, selected_order.0)
                    .map_err(bad)?;
                order_id = Some(selected_order);
                "output_reserved"
            }
            4 => {
                let selected: AdvanceSelector = decode(inv.parameters())?;
                let selected_order = selected.order;
                let before = self.before(txn, selected_order, inv.actor())?;
                if selected.observed != ctx.world().map_err(bad)?.world_ic_now() {
                    return Err(Error::Unavailable);
                }
                let cursor = before.progress.cursor();
                let mush_core::ScheduleEvaluation::Advance(advance) = cursor
                    .plan_due(
                        cursor.revision(),
                        &selected.observed,
                        if selected.restart {
                            mush_core::ScheduleDriveCause::Restart
                        } else {
                            mush_core::ScheduleDriveCause::Live
                        },
                    )
                    .map_err(bad)?
                else {
                    return Err(Error::Unavailable);
                };
                let after = self.owner(
                    ctx,
                    owner::Request::Advance {
                        before,
                        advance: *advance,
                    },
                )?;
                let row = after.order.ok_or(Error::Integrity)?;
                if after.input.is_some() {
                    return Err(Error::Integrity);
                }
                txn.replace(&r.families.orders, &selected_order, row.clone())
                    .map_err(bad)?;
                txn.replace(
                    &r.families.schedules,
                    &super::progress::schedule_id(selected_order)?,
                    row.progress.cursor().clone(),
                )
                .map_err(bad)?;
                ctx.advance_core_integration_completion(txn, selected_order.0)
                    .map_err(bad)?;
                order_id = Some(selected_order);
                "advanced"
            }
            5 | 6 => {
                let selected: OrderSelector = decode(inv.parameters())?;
                let selected_order = parse_order_id(&selected.order)?;
                let before = self.before(txn, selected_order, inv.actor())?;
                let request = if self.index == 5 {
                    if before.output.is_none() || !before.progress.production().is_complete() {
                        return Err(Error::Unavailable);
                    }
                    let output = ctx
                        .consume_core_output(&r.outputs, txn, selected_order.0)
                        .map_err(bad)?;
                    object = Some(output.object());
                    EconomyRequest::CoreCompletion(CoreCompletionRequest {
                        acceptance: before.accepted,
                        output,
                    })
                } else {
                    EconomyRequest::CoreCancellation(CoreCancellationRequest {
                        acceptance: before.accepted,
                    })
                };
                let payment = ctx
                    .prepare_owner(self.economy.as_ref().ok_or(Error::Unavailable)?, request)
                    .map_err(bad)?
                    .core_terminal()
                    .map_err(bad)?;
                let after = self.owner(ctx, owner::Request::Terminal { before, payment })?;
                let row = after.order.ok_or(Error::Integrity)?;
                let witness = after.input.ok_or(Error::Integrity)?;
                let input = r.inputs.publish(txn, ctx, witness.as_ref()).map_err(bad)?;
                row.verify_input(&input)?;
                r.industry
                    .release_slot(txn, row.intent.policy.factory, row.capacity, 1)
                    .map_err(bad)?;
                r.families.remove_schedule(txn, selected_order)?;
                txn.replace(&r.families.orders, &selected_order, row)
                    .map_err(bad)?;
                ctx.advance_core_integration_completion(txn, selected_order.0)
                    .map_err(bad)?;
                order_id = Some(selected_order);
                if self.index == 5 {
                    "completed"
                } else {
                    "cancelled"
                }
            }
            _ => return Err(Error::Invalid),
        };
        Ok(AuthoritativeActionOutput::new(RulesValue::new(r.outcome.clone(),json!({"operation":commit,"offer":offer_id,"order":order_id,"object":object,"state":state})).map_err(bad)?).with_definitions(vec![action.clone(),r.outcome.clone()]).with_eligible_witnesses(vec![inv.principal().clone()]))
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
        if Some(action) != r.actions.get(self.index)
            || self.index == 4
            || query.truth_subject() != &(mush_core::SubjectKey::Object { id: query.actor() })
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let Some(projection) = engine
            .world
            .module_state::<crate::core_production::participant::CoreProductionState>()
            .filter(|s| s.ready)
            .and_then(|s| s.integration.as_ref())
            .filter(|p| p.ready)
        else {
            return Ok(None);
        };
        if authority::cleanup(&engine.world, &r.definitions).is_err() {
            return Ok(None);
        }
        if self.index != 0 && self.index != 6 && r.config.mode != CoreOriginMode::Active {
            return Ok(None);
        }
        let fail = || self.refusal.clone();
        let parameter = |name: &str, required, kind| -> Result<ProjectedParameter, ActionError> {
            Ok(ProjectedParameter {
                id: mush_core::CanonicalId::new(name).map_err(|_| fail())?,
                required,
                kind,
            })
        };
        let mut parameters = vec![];
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
                    .is_none_or(|i| i.offered >= MAX_ORDERS)
                {
                    return Ok(None);
                }
                let mut classes = vec![];
                for class in DieClass::ALL {
                    if authority::can_offer(
                        &engine.world,
                        &r.definitions,
                        &r.config,
                        query.actor(),
                        class,
                    )
                    .is_ok()
                    {
                        classes.push(
                            serde_json::to_value(class)
                                .map_err(|_| fail())?
                                .as_str()
                                .ok_or_else(fail)?
                                .to_owned(),
                        )
                    }
                }
                if classes.is_empty() {
                    return Ok(None);
                }
                parameters.push(parameter("class", true, Kind::Choice { choices: classes })?);
                parameters.push(parameter(
                    "requester",
                    true,
                    Kind::Integer {
                        minimum: 0,
                        maximum: i64::from(u32::MAX),
                    },
                )?);
                parameters.push(parameter(
                    "supplier_lot",
                    false,
                    Kind::Text { maximum_bytes: 20 },
                )?);
            }
            2 => {
                if projection
                    .index
                    .as_ref()
                    .is_none_or(|i| i.accepted >= MAX_ORDERS)
                {
                    return Ok(None);
                }
                let mut offers = vec![];
                for row in projection.offers.values() {
                    if row.requester != query.actor()
                        || row.policy != r.config.payment
                        || row
                            .require_current_time(&engine.world.world_ic_now(), engine.world.now())
                            .is_err()
                        || projection
                            .orders
                            .values()
                            .any(|o| o.intent.offer == row.operation)
                        || authority::offer(
                            &engine.world,
                            &r.definitions,
                            &r.config,
                            row.operator,
                            row.requester,
                            row.recipe.class(),
                            &row.input,
                        )
                        .is_err()
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
                let payer = mush_org::PrincipalId::Object(query.actor());
                let accounts = economy
                    .snapshot()
                    .accounts_of(&payer)
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
                parameters.push(parameter("offer", true, Kind::Choice { choices: offers })?);
                parameters.push(parameter(
                    "payer",
                    true,
                    Kind::Choice { choices: accounts },
                )?);
                parameters.push(parameter(
                    "input_lot",
                    false,
                    Kind::Text { maximum_bytes: 20 },
                )?);
            }
            3 | 5 | 6 => {
                let mut orders = vec![];
                for (id, view) in &projection.orders {
                    if view.intent.requester != query.actor() || view.terminal.is_some() {
                        continue;
                    }
                    let Some(row) = engine
                        .action_state()
                        .get(&r.families.orders, id)
                        .map_err(|_| fail())?
                    else {
                        return Err(fail());
                    };
                    if super::state::OrderView::capture(row) != **view {
                        return Err(fail());
                    }
                    if self.index != 6
                        && authority::complete(
                            &engine.world,
                            &r.definitions,
                            &r.config,
                            &row.intent,
                        )
                        .is_err()
                    {
                        continue;
                    }
                    if self.index == 5
                        && (row.output.is_none() || !row.progress.production().is_complete())
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
                parameters.push(parameter("order", true, Kind::Choice { choices: orders })?);
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
        if self.index != 6 {
            guards
                .add_definition(r.config.payment.recipe.clone(), definitions)
                .map_err(|_| fail())?;
        }
        let (id, label) = match self.index {
            0 => ("initialize-core-factory", "Initialize core factory"),
            1 => ("offer-core-work", "Offer core integration"),
            2 => ("accept-core-work", "Accept core integration"),
            3 => ("reserve-core-output", "Reserve core output"),
            5 => ("complete-core-work", "Complete core integration"),
            6 => ("cancel-core-work", "Cancel core integration"),
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
                "Use an exact supplier offer and acknowledged work order.",
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
    let error = |e| ModError::msg(format!("core integration registration: {e}"));
    let runtime = Arc::new(runtime);
    for index in 0..super::ACTIONS.len() {
        let action = runtime.actions[index].clone();
        let owner = if index == 3 {
            None
        } else {
            Some(reg.request_owner_participant::<OriginParticipant>(action.clone())?)
        };
        let economy = if matches!(index, 2 | 5 | 6) {
            Some(reg.request_owner_participant::<EconomyParticipant>(action.clone())?)
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
                readiness: readiness.clone(),
                refusal: ActionError::new(
                    ActionErrorCode::ProviderRejected,
                    false,
                    "Core manufacture is unavailable or changed.",
                )
                .map_err(error)?,
            },
        )?;
    }
    Ok(())
}

//! Closed additional requests of the existing actual Industry participant.
use super::{
    IntegrationError as Error,
    authority::{self, Live},
    config::CoreFactoryConfig,
    offer::FactoryOffer,
    records::*,
    state::{OrderView, Projection},
};
use crate::core_production::{
    CoreOriginMode, authority::Definitions, participant::CoreProductionState,
};
use mush_core::participant::{OwnerPreparationBinding, OwnerPrepareContext, ParticipantError};
use mush_core::{GOD, ReplayWrite};
use mush_economy::core_integration::{CoreFundingOutput, CoreTerminalOutput};
use mush_inventory::core_input::{CoreInputRequest, CoreInputWitness, RegisteredCoreInputs};
use std::sync::Arc;

pub(crate) enum Request {
    Initialize,
    Deliver {
        before: Box<CoreIntegrationOrder>,
        output: Box<mush_core::core_delivery::DeliveredCoreOutput>,
    },
    Offer(Box<FactoryOffer>),
    Accept {
        order: Box<CoreIntegrationOrder>,
        payment: Box<CoreFundingOutput>,
    },
    Advance {
        before: Box<CoreIntegrationOrder>,
        advance: Box<mush_core::ScheduleAdvance>,
    },
    Terminal {
        before: Box<CoreIntegrationOrder>,
        payment: Box<CoreTerminalOutput>,
    },
}
pub(crate) struct Output {
    pub(crate) index: IntegrationIndex,
    pub(crate) order: Option<CoreIntegrationOrder>,
    pub(crate) input: Option<Arc<CoreInputWitness>>,
    pub(crate) delivered: Option<super::delivery::Record>,
}
enum Fence {
    Initialize,
    Offer(Box<FactoryOffer>, Box<Live>),
    Accept(Box<FactoryOffer>, Box<AcceptedIntent>, Box<Live>),
    Advance(Box<AcceptedIntent>, Box<mush_core::ScheduleAdvance>),
    Complete(Box<AcceptedIntent>, Box<Live>),
    Cancel,
    Deliver(Box<CoreIntegrationOrder>),
}
pub(crate) struct Patch {
    pub(crate) after: Projection,
    pub(crate) generation: u64,
    pub(crate) next_generation: u64,
    binding: OwnerPreparationBinding,
    before_index: Option<IntegrationIndex>,
    before_delivery: Option<super::delivery::Index>,
    fence: Fence,
    input: Option<Arc<CoreInputWitness>>,
}
fn refused() -> ParticipantError {
    ParticipantError::refused("core integration authority or acknowledged order changed")
}
fn identity(
    ctx: &OwnerPrepareContext<'_>,
    advance: Option<&mush_core::ScheduleAdvance>,
) -> Result<(), ParticipantError> {
    let inv = ctx.invocation();
    if let Some(advance) = advance {
        let ingress =
            mush_core::scheduled_action_ingress(ctx.action(), advance, inv.idempotency().epoch())
                .map_err(|_| refused())?;
        if inv.request() != ingress.request()
            || inv.principal() != ingress.principal()
            || inv.idempotency() != ingress.idempotency()
        {
            return Err(refused());
        }
    } else if inv.principal() != &(mush_core::PrincipalId::Object { id: inv.actor() }) {
        return Err(refused());
    }
    if inv.subject() != &mush_core::PerceivedRef::SelfActor
        || !inv.targets().is_empty()
        || inv.idempotency().scope().owner.as_str() != crate::NS
    {
        return Err(refused());
    }
    Ok(())
}
// Same dependencies as the personal Readiness service, resolved from the
// immutable actual Origin adapter configuration. This creates no actor or
// service capability and does not substitute an Object principal for K.
fn scheduled_readiness(
    ctx: &OwnerPrepareContext<'_>,
    defs: &Definitions,
    content: &mush_core::DefinitionRef,
) -> Result<(), ParticipantError> {
    if ctx.world().get(ctx.invocation().actor()).is_none() {
        return Err(refused());
    }
    let registry = defs.0.get().ok_or_else(refused)?;
    for name in ["industry", "org", "economy", "inventory"] {
        registry
            .require_owner_ready(&mush_core::OwnerId::new(name).map_err(|_| refused())?)
            .map_err(|_| refused())?;
    }
    registry
        .require_owner_ready(&content.key().owner)
        .map_err(|_| refused())
}
fn current_order(
    projection: &Projection,
    before: &CoreIntegrationOrder,
    ctx: &OwnerPrepareContext<'_>,
) -> Result<(), ParticipantError> {
    before.validate().map_err(|_| refused())?;
    if before.terminal.is_some()
        || ctx.invocation().actor() != before.intent.requester
        || projection
            .orders
            .get(&before.intent.order)
            .is_none_or(|o| o.as_ref() != &OrderView::capture(before))
    {
        return Err(refused());
    }
    Ok(())
}
struct OwnerPhase<'a, 'w> {
    ctx: &'a OwnerPrepareContext<'w>,
    config: &'a CoreFactoryConfig,
    defs: &'a Definitions,
    content: &'a mush_core::DefinitionRef,
    inputs: &'a RegisteredCoreInputs,
    projection: &'a Projection,
    after: &'a mut Projection,
    order_output: &'a mut Option<CoreIntegrationOrder>,
    input: &'a mut Option<Arc<CoreInputWitness>>,
    delivered: &'a mut Option<super::delivery::Record>,
}

impl OwnerPhase<'_, '_> {
    #[inline(never)]
    fn prepare_initialize(self) -> Result<Fence, ParticipantError> {
        let Self {
            ctx,
            config,
            defs,
            projection,
            after,
            ..
        } = self;
        Ok({
            if config.mode != CoreOriginMode::Initialize
                || ctx.invocation().actor() != GOD
                || ctx.action() != &config.action(super::INITIALIZE).map_err(|_| refused())?
                || projection.index.is_some()
                || projection.delivery_index.is_some()
                || !projection.deliveries.is_empty()
                || !projection.offers.is_empty()
                || !projection.orders.is_empty()
            {
                return Err(refused());
            }
            authority::cleanup(ctx.world(), defs).map_err(|_| refused())?;
            after.index = Some(
                IntegrationIndex::new(ctx.commit_token(), config.payment.clone())
                    .map_err(|_| refused())?,
            );
            if config.payment.delivery.is_some() {
                after.delivery_index =
                    Some(super::delivery::Index::new(ctx.commit_token()).map_err(|_| refused())?);
            }
            Fence::Initialize
        })
    }

    #[inline(never)]
    fn prepare_deliver(
        self,
        before: Box<CoreIntegrationOrder>,
        output: Box<mush_core::core_delivery::DeliveredCoreOutput>,
    ) -> Result<Fence, ParticipantError> {
        let Self {
            ctx,
            config,
            defs,
            projection,
            after,
            order_output,
            delivered,
            ..
        } = self;
        Ok({
            let (before, output) = (*before, *output);
            before.validate().map_err(|_| refused())?;
            if config.mode != CoreOriginMode::Active
                || config.payment.delivery.is_none()
                || before.intent.requester != ctx.invocation().actor()
                || output.commit() != ctx.commit_token()
                || output.action() != ctx.action()
                || ctx.action()
                    != &config
                        .action(super::delivery::ACTION)
                        .map_err(|_| refused())?
                || projection
                    .orders
                    .get(&before.intent.order)
                    .is_none_or(|r| r.as_ref() != &OrderView::capture(&before))
                || projection.deliveries.contains_key(&before.intent.order)
            {
                return Err(refused());
            }
            authority::cleanup(ctx.world(), defs).map_err(|_| refused())?;
            let h =
                mush_economy::core_integration::require_core_delivery(ctx.world(), before.accepted)
                    .map_err(|_| refused())?;
            before.verify_payment(h).map_err(|_| refused())?;
            let index = projection.delivery_index.as_ref().ok_or_else(refused)?;
            let record = super::delivery::Record::capture(
                index.delivered.checked_add(1).ok_or_else(refused)?,
                &before,
                &output,
            )
            .map_err(|_| refused())?;
            after.delivery_index = Some(index.append(&record).map_err(|_| refused())?);
            after
                .deliveries
                .insert(before.intent.order, Arc::new(record.clone()));
            *delivered = Some(record);
            *order_output = Some(before.clone());
            Fence::Deliver(Box::new(before))
        })
    }

    #[inline(never)]
    fn prepare_offer(self, row: Box<FactoryOffer>) -> Result<Fence, ParticipantError> {
        let Self {
            ctx,
            config,
            defs,
            projection,
            after,
            ..
        } = self;
        Ok({
            let row = *row;
            row.validate().map_err(|_| refused())?;
            if config.mode != CoreOriginMode::Active
                || (config.payment.delivery.is_some() && projection.delivery_index.is_none())
                || row.policy != config.payment
                || row.operation != ctx.commit_token()
                || row.action != *ctx.action()
                || row.operator != ctx.invocation().actor()
                || projection.offers.contains_key(&row.operation)
            {
                return Err(refused());
            }
            let live = authority::offer(
                ctx.world(),
                defs,
                config,
                row.operator,
                row.requester,
                row.recipe.class(),
                &row.input,
            )
            .map_err(|_| refused())?;
            if live.at != row.observed_ic
                || live.utc != row.observed_utc
                || live.recipe != row.recipe
                || live.facility != row.facility
            {
                return Err(refused());
            }
            after.index = Some(
                projection
                    .index
                    .as_ref()
                    .ok_or_else(refused)?
                    .offer(&row)
                    .map_err(|_| refused())?,
            );
            after.offers.insert(row.operation, Arc::new(row.clone()));
            Fence::Offer(Box::new(row), Box::new(live))
        })
    }

    #[inline(never)]
    fn prepare_accept(
        self,
        order: Box<CoreIntegrationOrder>,
        payment: Box<CoreFundingOutput>,
    ) -> Result<Fence, ParticipantError> {
        let Self {
            ctx,
            config,
            defs,
            inputs,
            projection,
            after,
            order_output,
            input,
            ..
        } = self;
        Ok({
            let (order, payment) = (*order, *payment);
            order.validate().map_err(|_| refused())?;
            if config.mode != CoreOriginMode::Active
                || (config.payment.delivery.is_some() && projection.delivery_index.is_none())
                || order.accepted != ctx.commit_token()
                || order.action != *ctx.action()
                || order.intent.requester != ctx.invocation().actor()
                || order.output.is_some()
                || order.terminal.is_some()
                || order.progress
                    != super::progress::CoreProgress::new(
                        order.intent.order,
                        order.intent.accepted_at.clone(),
                        order.intent.recipe.work_units(),
                    )
                    .map_err(|_| refused())?
                || order.intent.accepted_at != ctx.world().world_ic_now()
                || payment.subject() != &order.intent.subject().map_err(|_| refused())?
                || payment.receipt() != &order.funding
                || projection
                    .orders
                    .values()
                    .filter(|o| {
                        o.terminal.is_none() && o.intent.requester == order.intent.requester
                    })
                    .count()
                    >= crate::workorder::MAX_WORK_ORDERS_PER_REQUESTER
                || projection.orders.contains_key(&order.intent.order)
                || projection
                    .orders
                    .values()
                    .any(|o| o.intent.offer == order.intent.offer)
            {
                return Err(refused());
            }
            let offer = projection
                .offers
                .get(&order.intent.offer)
                .ok_or_else(refused)?;
            let live = authority::accept(ctx.world(), defs, config, offer, &order.intent)
                .map_err(|_| refused())?;
            *input = Some(
                inputs
                    .capture(
                        ctx,
                        CoreInputRequest::Reserve {
                            identity: order.intent.input_identity().map_err(|_| refused())?,
                            lot: order.intent.lot.id(),
                            endpoint: order.intent.lot.endpoint(),
                        },
                    )
                    .map_err(|_| refused())?,
            );
            after.index = Some(
                projection
                    .index
                    .as_ref()
                    .ok_or_else(refused)?
                    .accept(&order)
                    .map_err(|_| refused())?,
            );
            after
                .orders
                .insert(order.intent.order, Arc::new(OrderView::capture(&order)));
            let fence = Fence::Accept(
                Box::new((**offer).clone()),
                Box::new(order.intent.clone()),
                Box::new(live),
            );
            *order_output = Some(order);
            fence
        })
    }

    #[inline(never)]
    fn prepare_advance(
        self,
        before: Box<CoreIntegrationOrder>,
        advance: Box<mush_core::ScheduleAdvance>,
    ) -> Result<Fence, ParticipantError> {
        let Self {
            ctx,
            config,
            defs,
            content,
            projection,
            after,
            order_output,
            ..
        } = self;
        Ok({
            let (before, advance) = (*before, *advance);
            let mut stage = "owner-readiness";
            let result: Result<Fence, ParticipantError> = (|| {
                scheduled_readiness(ctx, defs, content)?;
                stage = "owner-current-order";
                current_order(projection, &before, ctx)?;
                stage = "owner-action-clock";
                if config.mode != CoreOriginMode::Active
                    || ctx.action() != &config.action(super::ADVANCE).map_err(|_| refused())?
                    || advance.observed_at() != &ctx.world().world_ic_now()
                {
                    return Err(refused());
                }
                stage = "owner-cleanup";
                authority::cleanup(ctx.world(), defs).map_err(|_| refused())?;
                stage = "owner-progress";
                let progress = before
                    .progress
                    .advance(ctx.commit_token(), &advance)
                    .map_err(|_| refused())?;
                let next = CoreIntegrationOrder { progress, ..before };
                stage = "owner-postimage";
                next.validate().map_err(|_| refused())?;
                after
                    .orders
                    .insert(next.intent.order, Arc::new(OrderView::capture(&next)));
                let fence = Fence::Advance(Box::new(next.intent.clone()), Box::new(advance));
                *order_output = Some(next);
                Ok(fence)
            })();
            result.inspect_err(|_| tracing::debug!(target: "core_integration_preparation", stage, "Core integration preparation refused"))?
        })
    }

    #[inline(never)]
    fn prepare_terminal(
        self,
        before: Box<CoreIntegrationOrder>,
        payment: Box<CoreTerminalOutput>,
    ) -> Result<Fence, ParticipantError> {
        let Self {
            ctx,
            config,
            defs,
            inputs,
            projection,
            after,
            order_output,
            input,
            ..
        } = self;
        Ok({
            let (before, payment) = (*before, *payment);
            current_order(projection, &before, ctx)?;
            if payment.subject() != &before.intent.subject().map_err(|_| refused())?
                || payment.receipt().commit() != ctx.commit_token()
                || payment.receipt().action() != ctx.action()
            {
                return Err(refused());
            }
            let complete = matches!(
                payment.receipt().result(),
                mush_economy::core_integration::CoreTerminalResult::Completed { .. }
            );
            let fence = if complete {
                if config.mode != CoreOriginMode::Active
                    || !before.progress.production().is_complete()
                {
                    return Err(refused());
                }
                let live = authority::complete(ctx.world(), defs, config, &before.intent)
                    .map_err(|_| refused())?;
                *input = Some(
                    inputs
                        .capture(
                            ctx,
                            CoreInputRequest::Consume {
                                identity: before.intent.input_identity().map_err(|_| refused())?,
                                endpoint: before.intent.lot.endpoint(),
                                expires: before.intent.expires_at.clone(),
                            },
                        )
                        .map_err(|_| refused())?,
                );
                Fence::Complete(Box::new(before.intent.clone()), Box::new(live))
            } else {
                if ctx.action() != &config.action(super::CANCEL).map_err(|_| refused())? {
                    return Err(refused());
                }
                authority::cleanup(ctx.world(), defs).map_err(|_| refused())?;
                *input = Some(
                    inputs
                        .capture(
                            ctx,
                            CoreInputRequest::Release {
                                identity: before.intent.input_identity().map_err(|_| refused())?,
                            },
                        )
                        .map_err(|_| refused())?,
                );
                Fence::Cancel
            };
            let next = CoreIntegrationOrder {
                terminal: Some(payment.receipt().clone()),
                ..before
            };
            next.validate().map_err(|_| refused())?;
            after
                .orders
                .insert(next.intent.order, Arc::new(OrderView::capture(&next)));
            *order_output = Some(next);
            fence
        })
    }
}

pub(crate) fn prepare(
    state: &CoreProductionState,
    ctx: &OwnerPrepareContext<'_>,
    config: &CoreFactoryConfig,
    defs: &Definitions,
    content: &mush_core::DefinitionRef,
    inputs: &RegisteredCoreInputs,
    request: Request,
) -> Result<(Box<Output>, Box<Patch>, Vec<ReplayWrite>), ParticipantError> {
    identity(
        ctx,
        match &request {
            Request::Advance { advance, .. } => Some(advance.as_ref()),
            _ => None,
        },
    )?;
    let projection = state
        .integration
        .as_ref()
        .filter(|p| p.ready)
        .ok_or_else(refused)?;
    if !state.ready {
        return Err(refused());
    }
    let mut after = projection.clone();
    let mut order_output = None;
    let mut input = None;
    let mut delivered = None;
    let phase = OwnerPhase {
        ctx,
        config,
        defs,
        content,
        inputs,
        projection,
        after: &mut after,
        order_output: &mut order_output,
        input: &mut input,
        delivered: &mut delivered,
    };
    let fence = match request {
        Request::Initialize => phase.prepare_initialize()?,
        Request::Deliver { before, output } => phase.prepare_deliver(before, output)?,
        Request::Offer(row) => phase.prepare_offer(row)?,
        Request::Accept { order, payment } => phase.prepare_accept(order, payment)?,
        Request::Advance { before, advance } => phase.prepare_advance(before, advance)?,
        Request::Terminal { before, payment } => phase.prepare_terminal(before, payment)?,
    };
    let index = after.index.clone().ok_or_else(refused)?;
    let mut writes = vec![ReplayWrite::ModuleJsonSet {
        namespace: mush_core::OwnerId::new(crate::NS).map_err(|_| refused())?,
        key: mush_core::AuthoritativeKey::parse_canonical(INDEX_KEY).map_err(|_| refused())?,
        value: encode(&index, INDEX_BYTES).map_err(|_| refused())?,
    }];
    if after.delivery_index != projection.delivery_index {
        let delivery = after.delivery_index.as_ref().ok_or_else(refused)?;
        writes.push(ReplayWrite::ModuleJsonSet {
            namespace: mush_core::OwnerId::new(crate::NS).map_err(|_| refused())?,
            key: mush_core::AuthoritativeKey::parse_canonical(super::delivery::INDEX_KEY)
                .map_err(|_| refused())?,
            value: encode(delivery, super::delivery::INDEX_BYTES).map_err(|_| refused())?,
        });
    }
    let output = Box::new(Output {
        index,
        order: order_output,
        input: input.clone(),
        delivered,
    });
    let patch = Box::new(Patch {
        after,
        generation: state.generation,
        next_generation: state.generation.checked_add(1).ok_or_else(refused)?,
        binding: ctx.preparation_binding(),
        before_index: projection.index.clone(),
        before_delivery: projection.delivery_index.clone(),
        fence,
        input,
    });
    Ok((output, patch, writes))
}
impl Patch {
    pub(crate) fn check_local(&self, state: &CoreProductionState) -> Result<(), ParticipantError> {
        if !state.ready
            || state.generation != self.generation
            || state.integration.as_ref().is_none_or(|p| {
                !p.ready || p.index != self.before_index || p.delivery_index != self.before_delivery
            })
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(crate) fn recheck(
        &self,
        state: &CoreProductionState,
        ctx: &OwnerPrepareContext<'_>,
        config: &CoreFactoryConfig,
        defs: &Definitions,
        content: &mush_core::DefinitionRef,
        inputs: &RegisteredCoreInputs,
    ) -> Result<(), ParticipantError> {
        self.check_local(state)?;
        identity(
            ctx,
            match &self.fence {
                Fence::Advance(_, advance) => Some(advance.as_ref()),
                _ => None,
            },
        )?;
        if !ctx.matches_preparation(&self.binding) {
            return Err(refused());
        }
        let mapped = |r: Result<(), Error>| r.map_err(|_| refused());
        match &self.fence {
            Fence::Initialize => {
                if config.mode != CoreOriginMode::Initialize || ctx.invocation().actor() != GOD {
                    return Err(refused());
                }
                mapped(authority::cleanup(ctx.world(), defs))?;
            }
            Fence::Offer(row, old) => {
                if config.mode != CoreOriginMode::Active {
                    return Err(refused());
                }
                let now = authority::offer(
                    ctx.world(),
                    defs,
                    config,
                    row.operator,
                    row.requester,
                    row.recipe.class(),
                    &row.input,
                )
                .map_err(|_| refused())?;
                mapped(row.require_current_time(&now.at, now.utc))?;
                mapped(authority::unchanged(old, &now))?;
            }
            Fence::Accept(offer, intent, old) => {
                if config.mode != CoreOriginMode::Active {
                    return Err(refused());
                }
                let now = authority::accept(ctx.world(), defs, config, offer, intent)
                    .map_err(|_| refused())?;
                mapped(authority::unchanged(old, &now))?;
            }
            Fence::Advance(intent, _) => {
                scheduled_readiness(ctx, defs, content)?;
                mapped(authority::cleanup(ctx.world(), defs))?;
                let now = ctx.world().world_ic_now();
                if config.mode != CoreOriginMode::Active
                    || now.clock != intent.accepted_at.clock
                    || now.at < intent.accepted_at.at
                    || now.at >= intent.expires_at.at
                {
                    return Err(refused());
                }
            }
            Fence::Complete(intent, old) => {
                if config.mode != CoreOriginMode::Active {
                    return Err(refused());
                }
                let now = authority::complete(ctx.world(), defs, config, intent)
                    .map_err(|_| refused())?;
                mapped(authority::unchanged(old, &now))?;
            }
            Fence::Cancel => mapped(authority::cleanup(ctx.world(), defs))?,
            Fence::Deliver(order) => {
                if config.mode != CoreOriginMode::Active
                    || config.payment.delivery.is_none()
                    || ctx.invocation().actor() != order.intent.requester
                    || ctx.action()
                        != &config
                            .action(super::delivery::ACTION)
                            .map_err(|_| refused())?
                {
                    return Err(refused());
                }
                mapped(authority::cleanup(ctx.world(), defs))?;
                let history = mush_economy::core_integration::require_core_delivery(
                    ctx.world(),
                    order.accepted,
                )
                .map_err(|_| refused())?;
                mapped(order.verify_payment(history))?;
            }
        }
        if let Some(input) = &self.input {
            inputs.recheck(ctx, input).map_err(|_| refused())?;
        }
        Ok(())
    }
}

//! Additional closed requests on the one existing Industry owner slot. Install
//! has no request here: its exact two native images are Economy and Tactics.
use super::{
    CoreRefitFactoryConfig, Error,
    authority::{self, Live},
    bad,
    records::*,
    state::Projection,
};
use crate::core_production::{CoreOriginMode, participant::CoreProductionState};
use mush_core::{
    ReplayWrite,
    participant::{OwnerPreparationBinding, OwnerPrepareContext, ParticipantError},
};
use mush_economy::core_refit::{RefitFundingOutput, RefitTerminalOutput, RefitTerminalResult};
use std::sync::Arc;
pub(crate) enum Request {
    Initialize,
    Offer(RefitOffer),
    Accept {
        order: RefitOrder,
        payment: RefitFundingOutput,
    },
    Advance {
        before: RefitOrder,
        advance: mush_core::ScheduleAdvance,
    },
    Cancel {
        before: RefitOrder,
        payment: RefitTerminalOutput,
    },
}
pub(crate) struct Output {
    pub(crate) index: RefitIndex,
    pub(crate) order: Option<RefitOrder>,
}
enum Fence {
    Initialize,
    Offer(RefitOffer, Live),
    Accept(AcceptedIntent, Live),
    Advance(AcceptedIntent, mush_core::ScheduleAdvance),
    Cancel,
}
pub(crate) struct Patch {
    pub(crate) after: Projection,
    pub(crate) next_generation: u64,
    generation: u64,
    before_index: Option<RefitIndex>,
    binding: OwnerPreparationBinding,
    fence: Fence,
}
fn refused() -> ParticipantError {
    ParticipantError::refused("Core refit owner preparation changed")
}
fn identity(
    ctx: &OwnerPrepareContext<'_>,
    advance: Option<&mush_core::ScheduleAdvance>,
) -> Result<(), ParticipantError> {
    let i = ctx.invocation();
    if let Some(a) = advance {
        let ingress = mush_core::scheduled_action_ingress(ctx.action(), a, i.idempotency().epoch())
            .map_err(|_| refused())?;
        if i.request() != ingress.request()
            || i.principal() != ingress.principal()
            || i.idempotency() != ingress.idempotency()
        {
            return Err(refused());
        }
    } else if i.principal() != &(mush_core::PrincipalId::Object { id: i.actor() }) {
        return Err(refused());
    }
    if i.subject() != &mush_core::PerceivedRef::SelfActor
        || !i.targets().is_empty()
        || i.idempotency().scope().owner.as_str() != crate::NS
    {
        return Err(refused());
    }
    Ok(())
}
fn pending(
    p: &Projection,
    before: &RefitOrder,
    ctx: &OwnerPrepareContext<'_>,
) -> Result<(), ParticipantError> {
    before.validate().map_err(|_| refused())?;
    if before.terminal.is_some()
        || before.intent.offer.requester != ctx.invocation().actor()
        || p.orders
            .get(&before.intent.order)
            .is_none_or(|v| v.as_ref() != before)
    {
        return Err(refused());
    }
    let history = ctx
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .ok_or_else(refused)?
        .core_refit_history(before.accepted)
        .map_err(|_| refused())?;
    before.verify_payment(history).map_err(|_| refused())
}
pub(crate) fn prepare(
    state: &CoreProductionState,
    ctx: &OwnerPrepareContext<'_>,
    c: &CoreRefitFactoryConfig,
    request: Request,
) -> Result<(Output, Patch, Vec<ReplayWrite>), ParticipantError> {
    identity(
        ctx,
        match &request {
            Request::Advance { advance, .. } => Some(advance),
            _ => None,
        },
    )?;
    authority::cleanup(ctx.world(), c).map_err(|_| refused())?;
    let p = state
        .refit
        .as_ref()
        .filter(|p| p.ready)
        .ok_or_else(refused)?;
    if !state.ready {
        return Err(refused());
    }
    let mut after = p.clone();
    let mut order_out = None;
    let fence = match request {
        Request::Initialize => {
            if c.mode != CoreOriginMode::Initialize
                || ctx.invocation().actor() != mush_core::GOD
                || ctx.action() != &c.action(super::INITIALIZE).map_err(|_| refused())?
                || p.index.is_some()
                || !p.offers.is_empty()
                || !p.orders.is_empty()
            {
                return Err(refused());
            }
            after.index = Some(
                RefitIndex::new(ctx.commit_token(), c.payment.clone()).map_err(|_| refused())?,
            );
            Fence::Initialize
        }
        Request::Offer(row) => {
            if c.mode != CoreOriginMode::Active
                || row.operation != ctx.commit_token()
                || row.action != *ctx.action()
                || row.operator != ctx.invocation().actor()
                || p.offers.contains_key(&row.operation)
            {
                return Err(refused());
            }
            let live = authority::offer(ctx.world(), c, &row).map_err(|_| refused())?;
            if row.observed_ic != live.at || row.observed_utc != live.utc {
                return Err(refused());
            }
            after.index = Some(
                p.index
                    .as_ref()
                    .ok_or_else(refused)?
                    .offer(&row)
                    .map_err(|_| refused())?,
            );
            after.offers.insert(row.operation, Arc::new(row.clone()));
            Fence::Offer(row, live)
        }
        Request::Accept { order, payment } => {
            order.validate().map_err(|_| refused())?;
            let id = order.intent.order;
            let economy = ctx
                .world()
                .module_state::<mush_economy::EconomyStore>()
                .ok_or_else(refused)?;
            let mut outstanding = 0usize;
            for prior in p
                .orders
                .values()
                .filter(|v| v.intent.offer.requester == order.intent.offer.requester)
            {
                let h = economy
                    .core_refit_history(prior.accepted)
                    .map_err(|_| refused())?;
                if h.terminal().is_none() {
                    outstanding = outstanding.checked_add(1).ok_or_else(refused)?;
                }
            }
            if c.mode != CoreOriginMode::Active
                || order.accepted != ctx.commit_token()
                || order.action != *ctx.action()
                || order.intent.offer.requester != ctx.invocation().actor()
                || order.terminal.is_some()
                || order.installation.is_some()
                || order.intent.accepted_at != ctx.world().world_ic_now()
                || order.progress
                    != super::progress::RefitProgress::new(
                        id,
                        order.intent.accepted_at.clone(),
                        order.intent.offer.recipe.work_units(),
                    )
                    .map_err(|_| refused())?
                || payment.subject() != &order.intent.subject().map_err(|_| refused())?
                || payment.receipt() != &order.funding
                || p.offers
                    .get(&order.intent.offer.operation)
                    .is_none_or(|v| v.as_ref() != &order.intent.offer)
                || p.orders.contains_key(&id)
                || p.orders
                    .values()
                    .any(|v| v.intent.offer.operation == order.intent.offer.operation)
                || outstanding >= crate::workorder::MAX_WORK_ORDERS_PER_REQUESTER
            {
                return Err(refused());
            }
            let live = authority::accept(ctx.world(), c, &order.intent).map_err(|_| refused())?;
            after.index = Some(
                p.index
                    .as_ref()
                    .ok_or_else(refused)?
                    .accept(&order)
                    .map_err(|_| refused())?,
            );
            after.orders.insert(id, Arc::new(order.clone()));
            let fence = Fence::Accept(order.intent.clone(), live);
            order_out = Some(order);
            fence
        }
        Request::Advance { before, advance } => {
            pending(p, &before, ctx)?;
            if c.mode != CoreOriginMode::Active
                || ctx.action() != &c.action(super::ADVANCE).map_err(|_| refused())?
                || advance.observed_at() != &ctx.world().world_ic_now()
            {
                return Err(refused());
            }
            let progress = before
                .progress
                .advance(ctx.commit_token(), &advance)
                .map_err(|_| refused())?;
            let next = RefitOrder { progress, ..before };
            next.validate().map_err(|_| refused())?;
            after
                .orders
                .insert(next.intent.order, Arc::new(next.clone()));
            let fence = Fence::Advance(next.intent.clone(), advance);
            order_out = Some(next);
            fence
        }
        Request::Cancel { before, payment } => {
            pending(p, &before, ctx)?;
            if ctx.action() != &c.action(super::CANCEL).map_err(|_| refused())?
                || payment.receipt().commit() != ctx.commit_token()
                || payment.receipt().action() != ctx.action()
                || payment.subject() != &before.intent.subject().map_err(|_| refused())?
                || !matches!(
                    payment.receipt().result(),
                    RefitTerminalResult::Cancelled {}
                )
            {
                return Err(refused());
            }
            let next = RefitOrder {
                terminal: Some(payment.receipt().clone()),
                ..before
            };
            next.validate().map_err(|_| refused())?;
            after
                .orders
                .insert(next.intent.order, Arc::new(next.clone()));
            order_out = Some(next);
            Fence::Cancel
        }
    };
    let index = after.index.clone().ok_or_else(refused)?;
    let writes = vec![ReplayWrite::ModuleJsonSet {
        namespace: mush_core::OwnerId::new(crate::NS).map_err(|_| refused())?,
        key: mush_core::AuthoritativeKey::parse_canonical(INDEX_KEY).map_err(|_| refused())?,
        value: encode(&index, INDEX_BYTES).map_err(|_| refused())?,
    }];
    Ok((
        Output {
            index,
            order: order_out,
        },
        Patch {
            after,
            next_generation: state.generation.checked_add(1).ok_or_else(refused)?,
            generation: state.generation,
            before_index: p.index.clone(),
            binding: ctx.preparation_binding(),
            fence,
        },
        writes,
    ))
}
impl Patch {
    pub(crate) fn check_local(&self, s: &CoreProductionState) -> Result<(), ParticipantError> {
        if !s.ready
            || s.generation != self.generation
            || s.refit
                .as_ref()
                .is_none_or(|p| !p.ready || p.index != self.before_index)
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(crate) fn recheck(
        &self,
        s: &CoreProductionState,
        ctx: &OwnerPrepareContext<'_>,
        c: &CoreRefitFactoryConfig,
    ) -> Result<(), ParticipantError> {
        self.check_local(s)?;
        identity(
            ctx,
            match &self.fence {
                Fence::Advance(_, a) => Some(a),
                _ => None,
            },
        )?;
        if !ctx.matches_preparation(&self.binding) {
            return Err(refused());
        }
        authority::cleanup(ctx.world(), c).map_err(|_| refused())?;
        match &self.fence {
            Fence::Initialize => {
                if c.mode != CoreOriginMode::Initialize
                    || ctx.invocation().actor() != mush_core::GOD
                {
                    return Err(refused());
                }
            }
            Fence::Offer(row, old) => {
                if c.mode != CoreOriginMode::Active {
                    return Err(refused());
                }
                let now = authority::offer(ctx.world(), c, row).map_err(|_| refused())?;
                row.require_time(&now.at, now.utc).map_err(|_| refused())?;
                authority::unchanged(old, &now).map_err(|_| refused())?
            }
            Fence::Accept(intent, old) => {
                if c.mode != CoreOriginMode::Active {
                    return Err(refused());
                }
                let now = authority::accept(ctx.world(), c, intent).map_err(|_| refused())?;
                authority::unchanged(old, &now).map_err(|_| refused())?
            }
            Fence::Advance(intent, _) => {
                let now = ctx.world().world_ic_now();
                if c.mode != CoreOriginMode::Active
                    || now.clock != intent.accepted_at.clock
                    || now.at < intent.accepted_at.at
                    || now.at >= intent.expires_at.at
                {
                    return Err(refused());
                }
            }
            Fence::Cancel => {}
        }
        Ok(())
    }
}

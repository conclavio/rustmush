//! **The Industry bay owner** (`industry/bay_refit` v1, pinned by the
//! kernel): Initialize, Offer, Accept, Advance and Cancel prepare here and
//! publish the admission census as the owner's native image. Install has no
//! request here: its two native images are Economy's and Tactics'.
use super::BayRefitConfig;
use super::authority::{self, Live};
use super::records::*;
use super::state::BayRefitState;
use mush_core::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPreparationBinding,
    OwnerPrepareContext, ParticipantError,
};
use mush_core::{ReplayWrite, ScheduleAdvance};
use mush_economy::bay_refit::{BayFundingOutput, BayTerminalOutput, BayTerminalResult};
use std::sync::Arc;

pub(crate) enum Request {
    Initialize,
    Offer(Box<BayOffer>),
    Accept {
        order: Box<BayOrder>,
        payment: Box<BayFundingOutput>,
    },
    Advance {
        before: Box<BayOrder>,
        advance: Box<ScheduleAdvance>,
    },
    Cancel {
        before: Box<BayOrder>,
        payment: Box<BayTerminalOutput>,
    },
}
pub(crate) struct Output {
    pub(crate) index: BayIndex,
    pub(crate) order: Option<Box<BayOrder>>,
}
enum Fence {
    Initialize,
    Offer(Box<BayOffer>, Box<Live>),
    Accept(Box<BayIntent>, Box<Live>),
    Advance(Box<BayIntent>, Box<ScheduleAdvance>),
    Cancel,
}
pub(crate) struct Patch {
    after: BayRefitState,
    generation: u64,
    before_index: Option<BayIndex>,
    binding: OwnerPreparationBinding,
    fence: Fence,
}
fn refused() -> ParticipantError {
    ParticipantError::refused("bay refit owner preparation changed")
}
fn identity(
    ctx: &OwnerPrepareContext<'_>,
    advance: Option<&ScheduleAdvance>,
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
/// An order still pending, exactly as acknowledged, with its Economy history.
fn pending(
    p: &BayRefitState,
    c: &BayRefitConfig,
    before: &BayOrder,
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
        .bay_refit_history(before.accepted)
        .map_err(|_| refused())?;
    before
        .verify_payment(history, &c.payment)
        .map_err(|_| refused())
}

#[inline(never)]
fn prepare_initialize(
    ctx: &OwnerPrepareContext<'_>,
    c: &BayRefitConfig,
    p: &BayRefitState,
    after: &mut BayRefitState,
) -> Result<Fence, ParticipantError> {
    if ctx.invocation().actor() != mush_core::GOD
        || ctx.action() != &c.action(super::INITIALIZE).map_err(|_| refused())?
        || p.index.is_some()
        || !p.offers.is_empty()
        || !p.orders.is_empty()
    {
        return Err(refused());
    }
    after.index = Some(BayIndex::new(ctx.commit_token(), c.policy.clone()).map_err(|_| refused())?);
    Ok(Fence::Initialize)
}
#[inline(never)]
#[allow(clippy::boxed_local)]
fn prepare_offer(
    ctx: &OwnerPrepareContext<'_>,
    c: &BayRefitConfig,
    p: &BayRefitState,
    after: &mut BayRefitState,
    row: Box<BayOffer>,
) -> Result<Fence, ParticipantError> {
    if row.operation != ctx.commit_token()
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
    after.offers.insert(row.operation, Arc::new((*row).clone()));
    Ok(Fence::Offer(row, Box::new(live)))
}
#[inline(never)]
#[allow(clippy::boxed_local)]
fn prepare_accept(
    ctx: &OwnerPrepareContext<'_>,
    c: &BayRefitConfig,
    p: &BayRefitState,
    after: &mut BayRefitState,
    order: Box<BayOrder>,
    payment: Box<BayFundingOutput>,
) -> Result<(Fence, Box<BayOrder>), ParticipantError> {
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
        if economy
            .bay_refit_history(prior.accepted)
            .map_err(|_| refused())?
            .terminal()
            .is_none()
        {
            outstanding = outstanding.checked_add(1).ok_or_else(refused)?;
        }
    }
    if order.accepted != ctx.commit_token()
        || order.action != *ctx.action()
        || order.intent.offer.requester != ctx.invocation().actor()
        || order.terminal.is_some()
        || order.installation.is_some()
        || order.intent.accepted_at != ctx.world().world_ic_now()
        || order.progress
            != super::progress::BayProgress::new(
                id,
                order.intent.accepted_at.clone(),
                order.intent.offer.recipe.work_units,
            )
            .map_err(|_| refused())?
        || payment.subject() != &order.intent.subject(&c.payment).map_err(|_| refused())?
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
    after.orders.insert(id, Arc::new((*order).clone()));
    Ok((
        Fence::Accept(Box::new(order.intent.clone()), Box::new(live)),
        order,
    ))
}
#[inline(never)]
#[allow(clippy::boxed_local)]
fn prepare_advance(
    ctx: &OwnerPrepareContext<'_>,
    c: &BayRefitConfig,
    p: &BayRefitState,
    after: &mut BayRefitState,
    before: Box<BayOrder>,
    advance: Box<ScheduleAdvance>,
) -> Result<(Fence, Box<BayOrder>), ParticipantError> {
    pending(p, c, &before, ctx)?;
    if ctx.action() != &c.action(super::ADVANCE).map_err(|_| refused())?
        || advance.observed_at() != &ctx.world().world_ic_now()
    {
        return Err(refused());
    }
    let progress = before
        .progress
        .advance(ctx.commit_token(), &advance)
        .map_err(|_| refused())?;
    let next = BayOrder {
        progress,
        ..*before
    };
    next.validate().map_err(|_| refused())?;
    after
        .orders
        .insert(next.intent.order, Arc::new(next.clone()));
    Ok((
        Fence::Advance(Box::new(next.intent.clone()), advance),
        Box::new(next),
    ))
}
#[inline(never)]
#[allow(clippy::boxed_local)]
fn prepare_cancel(
    ctx: &OwnerPrepareContext<'_>,
    c: &BayRefitConfig,
    p: &BayRefitState,
    after: &mut BayRefitState,
    before: Box<BayOrder>,
    payment: Box<BayTerminalOutput>,
) -> Result<(Fence, Box<BayOrder>), ParticipantError> {
    pending(p, c, &before, ctx)?;
    if ctx.action() != &c.action(super::CANCEL).map_err(|_| refused())?
        || payment.receipt().commit() != ctx.commit_token()
        || payment.receipt().action() != ctx.action()
        || payment.subject() != &before.intent.subject(&c.payment).map_err(|_| refused())?
        || !matches!(payment.receipt().result(), BayTerminalResult::Cancelled {})
    {
        return Err(refused());
    }
    let next = BayOrder {
        terminal: Some(payment.receipt().clone()),
        ..*before
    };
    next.validate().map_err(|_| refused())?;
    after
        .orders
        .insert(next.intent.order, Arc::new(next.clone()));
    Ok((Fence::Cancel, Box::new(next)))
}

/// The registered owner participant.
pub(crate) struct BayOwner {
    pub(crate) config: BayRefitConfig,
}
pub(crate) struct Bound<'a> {
    state: &'a mut BayRefitState,
    after: BayRefitState,
}
impl BoundParticipant for Bound<'_> {
    fn apply(self) {
        *self.state = self.after;
    }
}
impl OwnerParticipant for BayOwner {
    type State = BayRefitState;
    type Request = Request;
    type Output = Box<Output>;
    type Patch = Box<Patch>;
    type Bound<'a> = Bound<'a>;
    fn prepare(
        &self,
        state: &BayRefitState,
        ctx: &OwnerPrepareContext<'_>,
        request: Request,
    ) -> Result<OwnerPreparation<Box<Output>, Box<Patch>>, ParticipantError> {
        let c = &self.config;
        identity(
            ctx,
            match &request {
                Request::Advance { advance, .. } => Some(advance.as_ref()),
                _ => None,
            },
        )?;
        authority::cleanup(ctx.world(), c).map_err(|_| refused())?;
        if !state.ready {
            return Err(refused());
        }
        let mut after = state.clone();
        let (fence, order) = match request {
            Request::Initialize => (prepare_initialize(ctx, c, state, &mut after)?, None),
            Request::Offer(row) => (prepare_offer(ctx, c, state, &mut after, row)?, None),
            Request::Accept { order, payment } => {
                let (f, o) = prepare_accept(ctx, c, state, &mut after, order, payment)?;
                (f, Some(o))
            }
            Request::Advance { before, advance } => {
                let (f, o) = prepare_advance(ctx, c, state, &mut after, before, advance)?;
                (f, Some(o))
            }
            Request::Cancel { before, payment } => {
                let (f, o) = prepare_cancel(ctx, c, state, &mut after, before, payment)?;
                (f, Some(o))
            }
        };
        let index = after.index.clone().ok_or_else(refused)?;
        after.generation = state.generation.checked_add(1).ok_or_else(refused)?;
        let writes = vec![ReplayWrite::ModuleJsonSet {
            namespace: mush_core::OwnerId::new(crate::NS).map_err(|_| refused())?,
            key: mush_core::AuthoritativeKey::parse_canonical(INDEX_KEY).map_err(|_| refused())?,
            value: encode(&index, INDEX_BYTES).map_err(|_| refused())?,
        }];
        Ok(OwnerPreparation::new(
            Box::new(Output { index, order }),
            Box::new(Patch {
                after,
                generation: state.generation,
                before_index: state.index.clone(),
                binding: ctx.preparation_binding(),
                fence,
            }),
            writes,
        ))
    }
    fn recheck(
        &self,
        s: &BayRefitState,
        ctx: &OwnerPrepareContext<'_>,
        patch: &Box<Patch>,
    ) -> Result<(), ParticipantError> {
        let c = &self.config;
        patch.check_local(s)?;
        identity(
            ctx,
            match &patch.fence {
                Fence::Advance(_, a) => Some(a),
                _ => None,
            },
        )?;
        if !ctx.matches_preparation(&patch.binding) {
            return Err(refused());
        }
        authority::cleanup(ctx.world(), c).map_err(|_| refused())?;
        match &patch.fence {
            Fence::Initialize => {
                if ctx.invocation().actor() != mush_core::GOD {
                    return Err(refused());
                }
            }
            Fence::Offer(row, old) => {
                let now = authority::offer(ctx.world(), c, row).map_err(|_| refused())?;
                row.require_time(&now.at, now.utc).map_err(|_| refused())?;
                authority::unchanged(old, &now).map_err(|_| refused())?;
            }
            Fence::Accept(intent, old) => {
                let now = authority::accept(ctx.world(), c, intent).map_err(|_| refused())?;
                authority::unchanged(old, &now).map_err(|_| refused())?;
            }
            Fence::Advance(intent, _) => {
                let now = ctx.world().world_ic_now();
                if now.clock != intent.accepted_at.clock
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
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut BayRefitState,
        patch: Box<Patch>,
    ) -> Result<Bound<'a>, ParticipantError> {
        patch.check_local(state)?;
        Ok(Bound {
            state,
            after: patch.after,
        })
    }
    fn accepts_write(&self, w: &ReplayWrite) -> bool {
        matches!(w, ReplayWrite::ModuleJsonSet { namespace, key, value }
            if namespace.as_str() == crate::NS
                && key.as_str() == INDEX_KEY
                && decode::<BayIndex>(value, INDEX_BYTES).is_ok_and(|r| r.validate().is_ok()))
    }
}
impl Patch {
    fn check_local(&self, s: &BayRefitState) -> Result<(), ParticipantError> {
        if !s.ready || s.generation != self.generation || s.index != self.before_index {
            return Err(refused());
        }
        Ok(())
    }
}

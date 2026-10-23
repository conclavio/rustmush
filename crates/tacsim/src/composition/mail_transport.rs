//! Read-only composition adapters. Each owner persists only its own half of the
//! handoff; neither adapter mutates state or supplies a second delivery clock.
use mush_core::{Ctx, ObjId, World};
use mush_propagation::{EnvelopeState, dispatch, record::DispatchKind};

#[derive(Debug)]
pub struct MailTransport;

impl dispatch::Sources for MailTransport {
    fn requests(&self, world: &World) -> Option<Vec<dispatch::Request>> {
        Some(
            mush_mail::transit::intents(world)?
                .into_iter()
                .map(|intent| dispatch::Request {
                    reference: dispatch::DispatchRef {
                        version: 1,
                        key: intent.key,
                        bytes: intent.bytes,
                        origin: intent.origin,
                        fallback: intent.fallback,
                    },
                    kind: DispatchKind::Mail,
                    issuer: intent.from,
                    recipient: intent.to,
                    issued: intent.at,
                })
                .collect(),
        )
    }
}

impl mush_mail::transit::Transport for MailTransport {
    fn capacity(&self, world: &World, additional: usize) -> bool {
        dispatch::has_capacity(world, additional)
    }
    fn endpoints(&self, ctx: &Ctx<'_>, actors: &[ObjId]) -> Vec<Option<ObjId>> {
        let metric = mush_propagation::routing::snapshot_in(ctx, &ctx.w.world_ic_now());
        let state = ctx.state::<mush_sim::SpatialState>();
        actors
            .iter()
            .map(|&actor| {
                state.and_then(|state| match &metric {
                    Some(metric) => metric.endpoint(ctx.w, state, actor),
                    None => mush_propagation::context::place_of(state, ctx.w, actor),
                })
            })
            .collect()
    }
    fn receipt(&self, world: &World, key: &str) -> Option<mush_mail::transit::Receipt> {
        let envelope = dispatch::find(world, key)?;
        // Never consume a receipt produced during this same coarse step. This
        // leaves the transport receipt durable before the next owner applies it,
        // including local mail and returning confirmations.
        if envelope.advanced().at >= world.world_ic_now().at {
            return None;
        }
        let arrived = match envelope.state() {
            EnvelopeState::Delivered => true,
            EnvelopeState::Expired => false,
            _ => return None,
        };
        Some(mush_mail::transit::Receipt {
            arrived,
            origin: envelope
                .transit()
                .map(|t| t.position)
                .or_else(|| envelope.dispatch_ref().and_then(|d| d.origin)),
        })
    }
}

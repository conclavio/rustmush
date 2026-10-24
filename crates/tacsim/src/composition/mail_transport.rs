//! Read-only composition adapters. Each owner persists only its own half of the
//! handoff; neither adapter mutates state or supplies a second delivery clock.
use mush_core::{Ctx, ObjId, World};
use mush_propagation::{EnvelopeState, dispatch, record::DispatchKind};

#[derive(Debug)]
pub struct MailTransport;

impl dispatch::Sources for MailTransport {
    fn requests(&self, world: &World) -> Option<Vec<dispatch::Request>> {
        let mut requests: Vec<_> = mush_mail::transit::intents(world)?
            .into_iter()
            .map(|intent| dispatch::Request {
                reservation_actor: intent.reservation_actor,
                reservation_representation: intent.reservation_representation,
                representation: intent.representation,
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
            .collect();
        requests.extend(
            mush_comsys::transit::intents(world)?
                .into_iter()
                .map(|intent| dispatch::Request {
                    reservation_actor: intent.from,
                    reservation_representation: intent.representation,
                    representation: intent.representation,
                    reference: dispatch::DispatchRef {
                        version: 1,
                        key: intent.key,
                        bytes: intent.bytes,
                        origin: intent.origin,
                        fallback: None,
                    },
                    kind: DispatchKind::Channel,
                    issuer: intent.from,
                    recipient: intent.to,
                    issued: intent.at,
                }),
        );
        Some(requests)
    }
}

impl mush_comsys::transit::Transport for MailTransport {
    fn representation(
        &self,
        w: &World,
        a: ObjId,
    ) -> Result<Option<mush_core::RelayRepresentation>, mush_core::TransportRefusal> {
        mush_propagation::access::selected(w, a)
    }
    fn capacity(
        &self,
        w: &World,
        a: ObjId,
        r: Option<mush_core::RelayRepresentation>,
        n: usize,
    ) -> Result<(), mush_core::TransportRefusal> {
        mush_propagation::admission::check(
            w,
            a,
            r,
            mush_propagation::admission::Producer::Channel,
            n,
        )
    }
    fn endpoint(&self, world: &World, actor: ObjId) -> Option<ObjId> {
        crate::relay_metric::source_address(world, actor)
    }
    fn receipt(&self, world: &World, key: &str) -> Option<bool> {
        let e = dispatch::find(world, key)?;
        if e.advanced().at >= world.world_ic_now().at {
            return None;
        }
        match e.state() {
            EnvelopeState::Delivered => Some(true),
            EnvelopeState::Expired => Some(false),
            _ => None,
        }
    }
}

impl mush_mail::transit::Transport for MailTransport {
    fn representation(
        &self,
        w: &World,
        a: ObjId,
    ) -> Result<Option<mush_core::RelayRepresentation>, mush_core::TransportRefusal> {
        mush_propagation::access::selected(w, a)
    }
    fn capacity(
        &self,
        w: &World,
        a: ObjId,
        r: Option<mush_core::RelayRepresentation>,
        n: usize,
    ) -> Result<(), mush_core::TransportRefusal> {
        mush_propagation::admission::check(w, a, r, mush_propagation::admission::Producer::Mail, n)
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

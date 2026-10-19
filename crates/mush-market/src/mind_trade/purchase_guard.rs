//! Current venue admission before writer custody. Exact quote/product data is
//! held by the typed transaction's global revision/reservation and checked by
//! the closed purchase verifier; this guard closes the independent World input.
//!
//! `take_staged_action` evaluates it immediately before queue submission. The
//! world task holds the sole Engine while sending, then CommitPending freezes
//! World mutations, connection hooks, timers and logical clocks until the exact
//! receipt. The actual PortVenueGate is World-only. Replay never calls this
//! gate after durable intent owns the frozen images.

use std::sync::Arc;

use mush_core::action::{
    ActionError, AuthorityGuard, AuthorityGuardContext, AuthorityGuardValidator,
};
use mush_core::mechanics::{CanonicalId, OwnerId};
use mush_core::{ObjId, World};

use super::OriginalQuoteConfig;

#[derive(Clone)]
pub(crate) struct PurchaseGuard {
    config: OriginalQuoteConfig,
    gate: Arc<dyn crate::VenueGate>,
    capability: CanonicalId,
    owner: OwnerId,
}
impl PurchaseGuard {
    pub(crate) fn new(
        config: OriginalQuoteConfig,
        gate: Arc<dyn crate::VenueGate>,
    ) -> Result<Self, mush_core::ModError> {
        // Complete digest, not a truncated u64 policy fingerprint. The registry
        // freezes the actual gate/config and supplies the readiness identity.
        let capability = CanonicalId::new(config.policy_digest().map_err(super::error)?.to_hex())
            .map_err(super::error)?;
        Ok(Self {
            config,
            gate,
            capability,
            owner: OwnerId::new(crate::NS).map_err(super::error)?,
        })
    }
    pub(super) fn current(&self, world: &World, actor: ObjId) -> Result<(), String> {
        if world.get(actor).is_none_or(|o| {
            o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
        }) {
            return Err("purchase requester is unavailable".into());
        }
        let venue = crate::MarketId::new(&self.config.policy.venue.name)
            .ok_or("purchase venue is unavailable")?;
        let room = world
            .containing_room(actor)
            .ok_or("purchase requester has no venue")?;
        if self.gate.venue_of_room(world, room).as_ref() != Some(&venue) {
            return Err("purchase requester left its quoted venue".into());
        }
        self.gate
            .admit(world, actor, &venue, None, crate::Presence::Here)
            .map_err(|_| "purchase venue no longer admits requester".into())
    }
    pub(super) fn witness(&self, actor: ObjId) -> AuthorityGuard {
        AuthorityGuard::Capability {
            provider: self.owner.clone(),
            actor,
            capability: self.capability.clone(),
            generation: 1,
        }
    }
}
impl AuthorityGuardValidator for PurchaseGuard {
    fn is_current(
        &self,
        ctx: &AuthorityGuardContext<'_>,
        guard: &AuthorityGuard,
    ) -> Result<bool, ActionError> {
        let AuthorityGuard::Capability {
            provider,
            actor,
            capability,
            generation,
        } = guard
        else {
            return Ok(false);
        };
        Ok(provider == &self.owner
            && capability == &self.capability
            && *generation == 1
            && self.current(ctx.world(), *actor).is_ok())
    }
}

//! One closed policy witness for this opt-in. The shared Market capability
//! validator routes the old and new exact identities without changing either.
use super::EntitlementConfig;
use mush_core::action::{
    ActionError, AuthorityGuard, AuthorityGuardContext, AuthorityGuardValidator,
};
use mush_core::mechanics::{CanonicalId, OwnerId};
use mush_core::{ObjId, World};
use std::sync::Arc;
#[derive(Clone)]
pub(crate) struct EntitlementGuard {
    config: EntitlementConfig,
    gate: Arc<dyn crate::VenueGate>,
    capability: CanonicalId,
    owner: OwnerId,
}
impl EntitlementGuard {
    pub(crate) fn new(
        config: EntitlementConfig,
        gate: Arc<dyn crate::VenueGate>,
    ) -> Result<Self, mush_core::ModError> {
        let capability =
            CanonicalId::new(config.policy_ref()?.digest().to_hex()).map_err(super::error)?;
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
            return Err("entitlement requester unavailable".into());
        }
        let venue = &self.config.policy.venue_name;
        let room = world
            .containing_room(actor)
            .ok_or("entitlement requester has no venue")?;
        if self.gate.venue_of_room(world, room).as_ref() != Some(venue) {
            return Err("entitlement requester left the venue".into());
        }
        self.gate
            .admit(world, actor, venue, None, crate::Presence::Here)
            .map_err(|_| "entitlement venue unavailable".into())
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
impl AuthorityGuardValidator for EntitlementGuard {
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

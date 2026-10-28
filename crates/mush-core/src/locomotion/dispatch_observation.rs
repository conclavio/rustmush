//! Process-local custody of one genuine command capture and its admitted route.
//!
//! The registry calls the command gates and movement hooks before writer
//! custody. Installation checks only the already-recorded local preimages; no
//! command, alias, gate, or movement callback is replayed after acknowledgement.

use crate::action::{ActionInvocation, CommitToken};
use crate::action_state::TypedStateLens;
use crate::authoritative::AuthoritativeBundle;
use crate::mechanics::{DefinitionRef, OwnerId};
use crate::participant::ParticipantError;
use crate::{HookPoint, Registry, World};

use super::{CapturedLocomotion, LocomotionAdmission, RequestKind};

/// Retained only by the exact prepared transaction and its installation.
/// There is no caller constructor, clone, serializer, or public attach seam.
#[derive(Debug)]
pub(crate) struct BoundDispatchObservation {
    capture: CapturedLocomotion,
    invocation: ActionInvocation,
    action: DefinitionRef,
    commit: CommitToken,
    admission: LocomotionAdmission,
}

impl BoundDispatchObservation {
    pub(crate) fn new(
        capture: CapturedLocomotion,
        invocation: ActionInvocation,
        action: DefinitionRef,
        commit: CommitToken,
        admission: LocomotionAdmission,
    ) -> Result<Self, ParticipantError> {
        let actor = super::authenticated_actor(&invocation)?;
        let request = capture.request();
        let same_request = match (&request.kind, &admission.request.kind) {
            (RequestKind::Home, RequestKind::Home) => true,
            (RequestKind::ExitName(captured), RequestKind::ExitName(admitted)) => {
                captured == admitted
            }
            _ => false,
        };
        if capture.actor() != actor
            || admission.actor() != actor
            || admission.binding.invocation != invocation
            || admission.binding.action != action
            || admission.binding.commit != commit
            || !same_request
        {
            return Err(ParticipantError::Authority);
        }
        Ok(Self {
            capture,
            invocation,
            action,
            commit,
            admission,
        })
    }

    pub(crate) fn owner(&self) -> &OwnerId {
        &self.action.key().owner
    }

    pub(crate) fn recheck(
        &self,
        registry: &Registry,
        world: &World,
        lens: TypedStateLens<'_>,
    ) -> Result<(), ParticipantError> {
        self.capture.recheck(registry, world)?;
        super::recheck(
            world,
            &self.invocation,
            &self.action,
            self.commit,
            registry.hooks(HookPoint::MoveAttempt),
            lens,
            &self.admission,
        )
    }

    /// Called after every final guard callback. A later hook or validator can
    /// freeze an owner whose earlier callback appeared to allow the movement.
    pub(crate) fn require_ready(&self, registry: &Registry) -> Result<(), ParticipantError> {
        registry
            .action_providers()
            .ok_or(ParticipantError::Unavailable)?
            .get_exact(&self.action)
            .map_err(|_| ParticipantError::Unavailable)?;
        for hook in registry.hooks(HookPoint::MoveAttempt) {
            let owner = OwnerId::parse_canonical(hook.module.as_str())
                .map_err(|_| ParticipantError::Unavailable)?;
            registry
                .require_mechanics_owner_ready(&owner)
                .map_err(|_| ParticipantError::Unavailable)?;
        }
        Ok(())
    }

    /// A bundle cannot reuse the captured line for another semantic request,
    /// actor's idempotency scope, provider, action, or persistence token.
    pub(crate) fn matches_bundle(&self, bundle: &AuthoritativeBundle) -> bool {
        bundle.token() == self.commit
            && bundle.request() == self.invocation.request()
            && bundle.request_hash() == self.invocation.request_hash()
            && bundle.idempotency() == self.invocation.idempotency()
            && bundle.action() == &self.action
            && bundle.provider() == self.owner()
    }

    /// Callback-free final bind. ReadTrace validates live candidate complexity
    /// before equality, and includes missing objects as exact preimages.
    pub(crate) fn local(&self, world: &World) -> Result<(), ParticipantError> {
        if self.capture.actor() != self.invocation.actor()
            || self.admission.binding.invocation != self.invocation
            || self.admission.binding.action != self.action
            || self.admission.binding.commit != self.commit
        {
            return Err(ParticipantError::Authority);
        }
        self.capture.local(world)?;
        if !self.admission.trace.matches(world) {
            return Err(ParticipantError::refused(
                "captured locomotion facts changed before installation",
            ));
        }
        Ok(())
    }
}

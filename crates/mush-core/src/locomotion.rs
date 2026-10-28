//! Inert, authenticated locomotion admission groundwork.
//!
//! An admission records the current route, Basic lock and real MoveAttempt
//! decision for one actual preparation. It grants no publication capability:
//! there is no object image, movement, receipt or observer completion here.
//! Explicit command capture records the actual dispatch gates and route
//! without installing a production movement path. Future movement composition
//! must preserve the existing post-movement narrative and observer contract.

pub(crate) mod dispatch;
pub(crate) mod dispatch_observation;
pub(crate) mod image;
pub mod transition;
pub use dispatch::CapturedLocomotion;

use crate::action::{ActionInvocation, CommitToken};
use crate::action_state::TypedStateLens;
use crate::commands::movement::{exit_lock_allows_with, find_exit_with};
use crate::mechanics::DefinitionRef;
use crate::module::{HookReg, check_move_attempt_via};
use crate::object_read::{ObjectRead, ReadLimit, ReadTrace, TrackedRead};
use crate::participant::ParticipantError;
use crate::{ObjId, PerceivedRef, PrincipalId, World};

/// Maximum UTF-8 bytes in an admitted exit-name request.
pub const MAX_LOCOMOTION_EXIT_NAME_BYTES: usize = 512;

#[derive(Clone, Debug)]
enum RequestKind {
    ExitName(String),
    Home,
}

/// A bounded route request. The caller cannot supply an actor, exit object or
/// destination. HOME has an explicit constructor and cannot be an exit query.
#[derive(Clone, Debug)]
pub struct LocomotionRequest {
    kind: RequestKind,
}

impl LocomotionRequest {
    /// Request the same exact, case-insensitive exit-name lookup as ordinary
    /// movement. Input is not rewritten or interpreted as a command line.
    pub fn exit_name(name: &str) -> Result<Self, ParticipantError> {
        if name.is_empty()
            || name.len() > MAX_LOCOMOTION_EXIT_NAME_BYTES
            || name.trim() != name
            || name.chars().any(char::is_control)
            || name.eq_ignore_ascii_case("home")
        {
            return Err(refused("invalid locomotion exit name"));
        }
        Ok(Self {
            kind: RequestKind::ExitName(name.to_owned()),
        })
    }

    /// Request the authenticated actor's actual current home.
    #[must_use]
    pub const fn home() -> Self {
        Self {
            kind: RequestKind::Home,
        }
    }
}

/// A successful read-only decision. AlreadyHome intentionally carries no
/// admission and never invokes the movement hooks.
#[derive(Debug)]
pub enum LocomotionDecision {
    AlreadyHome,
    Admitted(LocomotionAdmission),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Route {
    actor: ObjId,
    source: Option<ObjId>,
    exit: Option<ObjId>,
    destination: ObjId,
}

/// An opaque decision for one actual invocation, action and commit. It is not
/// serializable or constructible by a caller, and exposes no moving capability.
pub struct LocomotionAdmission {
    binding: Box<Binding>,
    request: LocomotionRequest,
    route: Route,
    trace: ReadTrace,
}

struct Binding {
    invocation: ActionInvocation,
    action: DefinitionRef,
    commit: CommitToken,
}

impl std::fmt::Debug for LocomotionAdmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocomotionAdmission")
            .field("route", &self.route)
            .finish_non_exhaustive()
    }
}

impl LocomotionAdmission {
    #[must_use]
    pub const fn actor(&self) -> ObjId {
        self.route.actor
    }

    #[must_use]
    pub const fn source(&self) -> Option<ObjId> {
        self.route.source
    }

    #[must_use]
    pub const fn exit(&self) -> Option<ObjId> {
        self.route.exit
    }

    #[must_use]
    pub const fn destination(&self) -> ObjId {
        self.route.destination
    }
}

fn refused(message: &str) -> ParticipantError {
    ParticipantError::refused(message)
}

fn read_limit(_: ReadLimit) -> ParticipantError {
    refused("locomotion facts exceed proof limits")
}

fn authenticated_actor(invocation: &ActionInvocation) -> Result<ObjId, ParticipantError> {
    let actor = invocation.actor();
    if invocation.principal() != &(PrincipalId::Object { id: actor })
        || invocation.subject() != &PerceivedRef::SelfActor
        || !invocation.targets().is_empty()
    {
        return Err(ParticipantError::Authority);
    }
    Ok(actor)
}

fn select_route(
    reader: &mut TrackedRead<'_>,
    actor: ObjId,
    request: &LocomotionRequest,
) -> Result<Option<Route>, ParticipantError> {
    let object = reader
        .object(actor)
        .map_err(read_limit)?
        .filter(|object| object.kind.is_actor())
        .ok_or(ParticipantError::Authority)?;
    let source = object.location;
    let (exit, destination) = match &request.kind {
        RequestKind::ExitName(name) => {
            let exit = find_exit_with(reader, actor, name)
                .map_err(read_limit)?
                .ok_or_else(|| refused("no matching locomotion exit"))?;
            let destination = reader
                .object(exit)
                .map_err(read_limit)?
                .and_then(|object| object.destination);
            // The ordinary traversal tests the Basic lock before reporting an
            // unlinked exit, and consults movement hooks only after both pass.
            if !exit_lock_allows_with(reader, actor, exit).map_err(read_limit)? {
                return Err(refused("locomotion exit lock refused"));
            }
            (
                Some(exit),
                destination.ok_or_else(|| refused("locomotion exit is unlinked"))?,
            )
        }
        RequestKind::Home => {
            // Keep the source's full optional image for revalidation without
            // inventing an inverse-membership rule for ordinary HOME.
            if let Some(source) = source {
                reader.object(source).map_err(read_limit)?;
            }
            let home = object
                .home
                .ok_or_else(|| refused("locomotion actor has no home"))?;
            if reader.object(home).map_err(read_limit)?.is_none() {
                return Err(refused("locomotion home is missing"));
            }
            // Preserve the existing home check order, including an actor with
            // no current location. AlreadyHome produces no moving admission.
            if source == Some(home) {
                return Ok(None);
            }
            (None, home)
        }
    };
    Ok(Some(Route {
        actor,
        source,
        exit,
        destination,
    }))
}

/// Root preparation glue supplies the actual authenticated context, registered
/// hook slice and committed typed lens. No provider-supplied image is accepted.
pub(crate) fn derive(
    world: &World,
    invocation: &ActionInvocation,
    action: &DefinitionRef,
    commit: CommitToken,
    hooks: &[HookReg],
    lens: TypedStateLens<'_>,
    request: LocomotionRequest,
) -> Result<LocomotionDecision, ParticipantError> {
    if lens.store().is_none() {
        return Err(ParticipantError::Unavailable);
    }
    let actor = authenticated_actor(invocation)?;
    let mut reader = TrackedRead::new(world);
    let Some(route) = select_route(&mut reader, actor, &request)? else {
        // Even an inert decision cannot swallow a poisoned proof reader.
        reader.finish().map_err(read_limit)?;
        return Ok(LocomotionDecision::AlreadyHome);
    };
    check_move_attempt_via(hooks, world, actor, route.exit, route.destination, lens)
        .map_err(|_| refused("locomotion movement hook refused"))?;
    crate::world::can_move_to_with(&mut reader, actor, route.destination)
        .map_err(read_limit)?
        .map_err(|_| refused("locomotion destination cannot accept the actor"))?;
    let trace = reader.finish().map_err(read_limit)?;
    Ok(LocomotionDecision::Admitted(LocomotionAdmission {
        binding: Box::new(Binding {
            invocation: invocation.clone(),
            action: action.clone(),
            commit,
        }),
        request,
        route,
        trace,
    }))
}

/// Compare exact admitted facts, then repeat the shared route/lock policy and
/// real hooks. Hook-owned arbitrary World reads are not mirrored in the trace.
pub(crate) fn recheck(
    world: &World,
    invocation: &ActionInvocation,
    action: &DefinitionRef,
    commit: CommitToken,
    hooks: &[HookReg],
    lens: TypedStateLens<'_>,
    admission: &LocomotionAdmission,
) -> Result<(), ParticipantError> {
    if admission.binding.invocation != *invocation
        || admission.binding.action != *action
        || admission.binding.commit != commit
    {
        return Err(ParticipantError::Authority);
    }
    if !admission.trace.matches(world) {
        return Err(refused("locomotion facts changed"));
    }
    match derive(
        world,
        invocation,
        action,
        commit,
        hooks,
        lens,
        admission.request.clone(),
    )? {
        LocomotionDecision::Admitted(current) if current.route == admission.route => Ok(()),
        _ => Err(refused("locomotion route changed")),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod dispatch_tests;

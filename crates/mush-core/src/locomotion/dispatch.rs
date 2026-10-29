//! Opaque evidence from the actual command pipeline. This is an inert route
//! request, not a moving capability or a replacement for ordinary dispatch.

use std::sync::Arc;

use crate::cmd::{BuiltinLocomotion, Registry, check_command_gates};
use crate::commands::movement::find_exit_with;
use crate::object_read::{ObjectRead, ReadTrace, TrackedRead};
use crate::participant::ParticipantError;
use crate::{ObjId, World};

use super::{LocomotionRequest, RequestKind, read_limit, refused};

pub(crate) const MAX_GATE_LINES: usize = 32;
pub(crate) const MAX_GATE_BYTES: usize = 32 * 1024;

/// Only the shared dispatcher records the lines actually accepted by gates.
/// Once a bound is exceeded, a later short rewrite cannot recover a capture.
pub(crate) struct GateTranscript {
    lines: Vec<String>,
    bytes: usize,
    exceeded: bool,
}

impl GateTranscript {
    pub(crate) fn new() -> Self {
        Self {
            lines: Vec::new(),
            bytes: 0,
            exceeded: false,
        }
    }

    pub(crate) fn record(&mut self, line: &str) {
        if self.exceeded {
            return;
        }
        if self.lines.len() >= MAX_GATE_LINES
            || line.len() > crate::MAX_INGRESS_LINE_BYTES
            || line.len() > MAX_GATE_BYTES.saturating_sub(self.bytes)
        {
            self.exceeded = true;
            return;
        }
        self.bytes += line.len();
        self.lines.push(line.to_owned());
    }

    fn finish(self) -> Result<Vec<String>, ParticipantError> {
        if self.exceeded || self.lines.is_empty() {
            return Err(refused(
                "locomotion command transcript exceeds proof limits",
            ));
        }
        Ok(self.lines)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    HomeKeyword,
    WholeLineExit,
    Builtin {
        index: usize,
        kind: BuiltinLocomotion,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ObservedRoute {
    source: Option<ObjId>,
    exit: Option<ObjId>,
    destination: Option<ObjId>,
}

struct Selection {
    origin: Origin,
    request: LocomotionRequest,
    route: ObservedRoute,
}

/// A command route selected by Core's actual dispatch pipeline. The original
/// actor, passed command-gate lines and exact lookup facts cannot be supplied
/// or rewritten by a caller. Capturing does not check a Basic lock, invoke a
/// movement hook, move an object, or produce a durable action.
pub struct CapturedLocomotion {
    registry: Arc<()>,
    actor: ObjId,
    lines: Vec<String>,
    selection: Selection,
    trace: ReadTrace,
}

impl std::fmt::Debug for CapturedLocomotion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CapturedLocomotion")
            .field("actor", &self.actor)
            .field("route", &self.selection.route)
            .finish_non_exhaustive()
    }
}

impl CapturedLocomotion {
    /// The immutable actor supplied to the actual dispatch invocation.
    #[must_use]
    pub const fn actor(&self) -> ObjId {
        self.actor
    }

    pub(crate) fn request(&self) -> LocomotionRequest {
        self.selection.request.clone()
    }

    /// Callback-free validation for acknowledged installation. Includes the
    /// negative whole-line lookup facts that an exit-name admission alone
    /// would not observe. Command gates and aliases never run after ACK.
    pub(crate) fn local(&self, world: &World) -> Result<(), ParticipantError> {
        if !self.trace.matches(world) {
            return Err(refused("locomotion dispatch facts changed"));
        }
        Ok(())
    }

    pub(crate) fn recheck(
        &self,
        registry: &Registry,
        world: &World,
    ) -> Result<(), ParticipantError> {
        if !Arc::ptr_eq(&self.registry, &registry.continuation_identity) {
            return Err(ParticipantError::Authority);
        }
        self.local(world)?;
        for line in &self.lines {
            check_command_gates(registry, world, self.actor, line)
                .map_err(|_| refused("locomotion command gate refused"))?;
        }
        // No AliasStage is repeated: it is a mutable dispatch operation. Only
        // downstream precedence and the actual built-in spec are re-resolved.
        let line = self
            .lines
            .last()
            .ok_or_else(|| refused("locomotion command transcript missing"))?;
        let mut reader = TrackedRead::new(world);
        let current = select(registry, world, self.actor, line, &mut reader)?
            .ok_or_else(|| refused("locomotion command route changed"))?;
        reader.finish().map_err(read_limit)?;
        if current.origin != self.selection.origin
            || current.route != self.selection.route
            || !same_request(&current.request, &self.selection.request)
        {
            return Err(refused("locomotion command route changed"));
        }
        Ok(())
    }
}

fn same_request(left: &LocomotionRequest, right: &LocomotionRequest) -> bool {
    match (&left.kind, &right.kind) {
        (RequestKind::Home, RequestKind::Home) => true,
        (RequestKind::ExitName(left), RequestKind::ExitName(right)) => left == right,
        _ => false,
    }
}

/// Called only at the movement boundary of the shared command dispatcher.
pub(crate) fn capture(
    registry: &Registry,
    world: &World,
    actor: ObjId,
    transcript: GateTranscript,
) -> Result<Option<CapturedLocomotion>, ParticipantError> {
    let lines = transcript.finish()?;
    let line = lines
        .last()
        .ok_or_else(|| refused("locomotion command transcript missing"))?;
    let mut reader = TrackedRead::new(world);
    let Some(selection) = select(registry, world, actor, line, &mut reader)? else {
        reader.finish().map_err(read_limit)?;
        return Ok(None);
    };
    let trace = reader.finish().map_err(read_limit)?;
    Ok(Some(CapturedLocomotion {
        registry: Arc::clone(&registry.continuation_identity),
        actor,
        lines,
        selection,
        trace,
    }))
}

fn select(
    registry: &Registry,
    world: &World,
    actor: ObjId,
    line: &str,
    reader: &mut TrackedRead<'_>,
) -> Result<Option<Selection>, ParticipantError> {
    let (origin, request) = if line.trim().eq_ignore_ascii_case("home") {
        (Origin::HomeKeyword, LocomotionRequest::home())
    } else if find_exit_with(reader, actor, line.trim())
        .map_err(read_limit)?
        .is_some()
    {
        (
            Origin::WholeLineExit,
            LocomotionRequest::exit_name(line.trim())?,
        )
    } else {
        let Some((index, kind, args)) = registry
            .locomotion_entry(world, actor, line)
            .map_err(|reason| refused(&reason))?
        else {
            return Ok(None);
        };
        let request = match kind {
            BuiltinLocomotion::Home => LocomotionRequest::home(),
            BuiltinLocomotion::Goto if args.raw().eq_ignore_ascii_case("home") => {
                LocomotionRequest::home()
            }
            BuiltinLocomotion::Goto => LocomotionRequest::exit_name(args.raw())?,
        };
        (Origin::Builtin { index, kind }, request)
    };
    let object = reader
        .object(actor)
        .map_err(read_limit)?
        .filter(|object| object.kind.is_actor())
        .ok_or(ParticipantError::Authority)?;
    let source = object.location;
    let home = object.home;
    if let Some(source) = source {
        reader.object(source).map_err(read_limit)?;
    }
    let (exit, destination) = match &request.kind {
        RequestKind::Home => (None, home),
        RequestKind::ExitName(name) => {
            let exit = find_exit_with(reader, actor, name)
                .map_err(read_limit)?
                .ok_or_else(|| refused("You can't go that way."))?;
            let destination = reader
                .object(exit)
                .map_err(read_limit)?
                .and_then(|exit| exit.destination);
            (Some(exit), destination)
        }
    };
    if let Some(destination) = destination {
        reader.object(destination).map_err(read_limit)?;
    }
    Ok(Some(Selection {
        origin,
        request,
        route: ObservedRoute {
            source,
            exit,
            destination,
        },
    }))
}

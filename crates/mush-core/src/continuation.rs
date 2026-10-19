//! Owner-bound command continuations for asynchronous module work. These run
//! the exact registered command under normal permission gates without feeding
//! synthetic text into a player's input context. They confer no business or
//! publication authority: handlers must still recheck their retained subject.

use std::sync::Arc;

use smol_str::SmolStr;

use crate::{CmdError, Ctx, ObjId, World, mechanics::OwnerId};

/// Opaque capability minted only during actual module composition for one of
/// that module's authoritative commands. No serde or public constructor exists.
#[derive(Clone, Debug)]
pub struct ContinuationHandle {
    pub(crate) registry: Arc<()>,
    pub(crate) index: usize,
    pub(crate) owner: OwnerId,
    pub(crate) primary: SmolStr,
    pub(crate) switches: Vec<SmolStr>,
    pub(crate) prefix: String,
}
impl PartialEq for ContinuationHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.registry, &other.registry)
            && self.index == other.index
            && self.switches == other.switches
    }
}
impl Eq for ContinuationHandle {}

/// Scheduler payload. Its actor, argument and exact handler cannot be changed
/// by a transport or constructed from command text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContinuationRequest {
    pub(crate) handle: ContinuationHandle,
    actor: ObjId,
    pub(crate) argument: String,
}
impl ContinuationRequest {
    pub(crate) const fn actor(&self) -> ObjId {
        self.actor
    }
}

pub(crate) fn live_actor(world: &World, actor: ObjId) -> bool {
    world.get(actor).is_some_and(|object| {
        object.kind == crate::ObjKind::Player && !object.flags.contains(crate::Flags::GOING)
    })
}

impl Ctx<'_> {
    /// Queue a bounded continuation as a retained subject's actual actor. Only
    /// the registering module's immutable dispatch stamp can use this handle.
    /// The module must derive `actor` from its own retained job/operation and
    /// the handler must recheck it. No actor or verb is accepted from transport.
    ///
    /// Success queues a deferred request, not an execution/absence receipt.
    /// Ordinary per-actor wait limits and handler rollback still apply; a
    /// dropped or refused timer never proves that authoritative work is absent.
    pub fn schedule_continuation(
        &mut self,
        handle: &ContinuationHandle,
        actor: ObjId,
        argument: &str,
        delay_ms: i64,
    ) -> Result<(), CmdError> {
        let refused = || CmdError::msg("Continuation unavailable.");
        let caller = self.typed_state.dispatch_caller().ok_or_else(refused)?;
        if caller.module() != &handle.owner
            || !self.reg.continuation_ready(handle)
            || !live_actor(self.w, actor)
            || delay_ms < 0
            || handle
                .prefix
                .len()
                .checked_add(1)
                .and_then(|n| n.checked_add(argument.len()))
                .is_none_or(|n| n > crate::MAX_INGRESS_LINE_BYTES)
        {
            return Err(refused());
        }
        self.out.defer(crate::Deferred::ScheduleIn {
            delay_ms,
            kind: crate::ScheduledKind::CommandContinuation(ContinuationRequest {
                handle: handle.clone(),
                actor,
                argument: argument.to_string(),
            }),
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests;

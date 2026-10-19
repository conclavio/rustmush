//! One-shot legacy completion bound to the actual requesting connection.
//!
//! This is separate from authoritative continuations: it grants no business
//! permission and never permits a module to choose an actor or connection.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use crate::{CmdError, ConnId, Ctx, ObjId, World};

/// Maximum wall lifetime of a captured completion, including worker time.
pub const CONNECTION_COMPLETION_TTL: Duration = Duration::from_secs(120);

/// Exact module-owned LegacyPostImage handler and fixed switches. Only the
/// registrar can construct this value; it does not itself identify a player.
#[derive(Clone, Debug)]
pub struct ConnectionCompletionHandle(pub(crate) crate::ContinuationHandle);

/// Captured only during actual connected Cmd/Invoke ingress. Its handler,
/// bounded argument, actor, physical connection and lease cannot be replaced.
/// Clones share one queued attempt and one handler-entry allowance.
#[derive(Clone, Debug)]
pub struct ConnectionCompletion(Arc<Captured>);

#[derive(Debug)]
struct Captured {
    handle: ConnectionCompletionHandle,
    actor: ObjId,
    conn: ConnId,
    lease: u64,
    argument: String,
    expires: Instant,
    queued: AtomicBool,
    entered: AtomicBool,
    identity: Arc<()>,
}

/// Opaque scheduler payload. Dropping the last copy before handler entry
/// releases the queue marker, including deferred rollback and queue refusal.
#[derive(Clone, Debug)]
pub struct ConnectionCompletionRequest(Arc<Queued>);

#[derive(Debug)]
struct Queued(ConnectionCompletion);
impl Drop for Queued {
    fn drop(&mut self) {
        self.0.0.queued.store(false, Ordering::Release);
    }
}
impl PartialEq for ConnectionCompletionRequest {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for ConnectionCompletionRequest {}

/// Core-private physical ingress/entry stamp. World clones intentionally do
/// not inherit it: a cloned compatibility world is not a live client dispatch.
#[derive(Debug, Default)]
pub(crate) struct ConnectionDispatchSlot(Option<ConnectionDispatch>);
impl Clone for ConnectionDispatchSlot {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl ConnectionDispatchSlot {
    pub(crate) fn get(&self) -> Option<&ConnectionDispatch> {
        self.0.as_ref()
    }
    pub(crate) fn set(&mut self, value: Option<ConnectionDispatch>) {
        self.0 = value;
    }
}
#[derive(Clone, Debug)]
pub(crate) struct ConnectionDispatch {
    actor: ObjId,
    conn: ConnId,
    lease: u64,
    completion: Option<Arc<()>>,
}
impl ConnectionDispatch {
    pub(crate) fn ingress(world: &World, actor: ObjId, conn: ConnId) -> Option<Self> {
        let (bound, lease) = world.connection_binding(conn)?;
        (bound == actor).then_some(Self {
            actor,
            conn,
            lease,
            completion: None,
        })
    }
}

impl ConnectionCompletion {
    /// Still bound to the original connection and within its absolute lifetime.
    /// This is transport custody only, never current business permission.
    #[must_use]
    pub fn is_live(&self, world: &World) -> bool {
        Instant::now() < self.0.expires
            && crate::continuation::live_actor(world, self.0.actor)
            && world.connection_binding(self.0.conn) == Some((self.0.actor, self.0.lease))
    }

    /// Whether core is currently entering this exact captured completion.
    /// A manually typed command or another ticket cannot satisfy this check.
    #[must_use]
    pub fn is_current(&self, ctx: &Ctx<'_>) -> bool {
        self.is_live(ctx.w)
            && ctx.typed_state.dispatch_caller().is_some_and(|caller| {
                caller.actor() == self.0.actor && caller.module() == &self.0.handle.0.owner
            })
            && ctx.w.connection_dispatch().is_some_and(|stamp| {
                stamp.actor == self.0.actor
                    && stamp.conn == self.0.conn
                    && stamp.lease == self.0.lease
                    && stamp
                        .completion
                        .as_ref()
                        .is_some_and(|id| Arc::ptr_eq(id, &self.0.identity))
            })
    }

    /// Original physical reply destination. Call `is_current` and the owner's
    /// fresh permission checks before producing private output or mutations.
    #[must_use]
    pub fn connection(&self) -> ConnId {
        self.0.conn
    }

    /// A handler has already entered; scheduling it again is always refused.
    #[must_use]
    pub fn has_entered(&self) -> bool {
        self.0.entered.load(Ordering::Acquire)
    }
}

impl ConnectionCompletionRequest {
    pub(crate) fn actor(&self) -> ObjId {
        self.0.0.0.actor
    }
    pub(crate) fn ticket(&self) -> &ConnectionCompletion {
        &self.0.0
    }
    pub(crate) fn handle(&self) -> &crate::ContinuationHandle {
        &self.ticket().0.handle.0
    }
    pub(crate) fn argument(&self) -> &str {
        &self.ticket().0.argument
    }
    pub(crate) fn enter(&self, world: &World) -> Option<ConnectionDispatch> {
        let ticket = self.ticket();
        if !ticket.is_live(world) || ticket.0.entered.swap(true, Ordering::AcqRel) {
            return None;
        }
        Some(ConnectionDispatch {
            actor: ticket.0.actor,
            conn: ticket.0.conn,
            lease: ticket.0.lease,
            completion: Some(Arc::clone(&ticket.0.identity)),
        })
    }
}

impl Ctx<'_> {
    /// Capture the actual ingress connection for this module's exact registered
    /// completion handler. Public actor/module/dispatching_conn fields are not
    /// authority. Timers, detached contexts and completion re-entry cannot mint.
    pub fn capture_connection_completion(
        &self,
        handle: &ConnectionCompletionHandle,
        argument: &str,
    ) -> Result<ConnectionCompletion, CmdError> {
        let refused = || CmdError::msg("Connection completion unavailable.");
        let caller = self.typed_state.dispatch_caller().ok_or_else(refused)?;
        let stamp = self.w.connection_dispatch().ok_or_else(refused)?;
        if caller.module() != &handle.0.owner
            || caller.actor() != stamp.actor
            || stamp.completion.is_some()
            || !self.reg.connection_completion_ready(handle)
            || !crate::continuation::live_actor(self.w, stamp.actor)
            || self.w.connection_binding(stamp.conn) != Some((stamp.actor, stamp.lease))
            || handle
                .0
                .prefix
                .len()
                .checked_add(1)
                .and_then(|n| n.checked_add(argument.len()))
                .is_none_or(|n| n > crate::MAX_INGRESS_LINE_BYTES)
        {
            return Err(refused());
        }
        Ok(ConnectionCompletion(Arc::new(Captured {
            handle: handle.clone(),
            actor: stamp.actor,
            conn: stamp.conn,
            lease: stamp.lease,
            argument: argument.to_owned(),
            expires: Instant::now() + CONNECTION_COMPLETION_TTL,
            queued: AtomicBool::new(false),
            entered: AtomicBool::new(false),
            identity: Arc::new(()),
        })))
    }

    /// Queue the captured handler immediately, at most once concurrently.
    /// Ordinary per-actor queue limits still apply. Success is not an execution
    /// receipt; the module retains its bounded job until entry or expiry.
    pub fn schedule_connection_completion(
        &mut self,
        ticket: &ConnectionCompletion,
    ) -> Result<(), CmdError> {
        let refused = || CmdError::msg("Connection completion unavailable.");
        if !self
            .typed_state
            .dispatch_caller()
            .is_some_and(|caller| caller.module() == &ticket.0.handle.0.owner)
            || !self.reg.connection_completion_ready(&ticket.0.handle)
            || !ticket.is_live(self.w)
            || ticket.has_entered()
        {
            return Err(refused());
        }
        if ticket
            .0
            .queued
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(());
        }
        self.out.defer(crate::Deferred::ScheduleIn {
            delay_ms: 0,
            kind: crate::ScheduledKind::ConnectionCompletion(ConnectionCompletionRequest(
                Arc::new(Queued(ticket.clone())),
            )),
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests;

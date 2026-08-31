//! The gateway-side server → gateway delivery transport: route resolved
//! `(SendTarget, OutputEvent)` batches onto the [`SessionManager`] and police
//! slow clients / reap link-dead sessions (D5), feeding the resulting
//! `ConnDown`/`SessionGone` back to the server as inbound over the boundary
//! writer.
//!
//! The world already resolved audience/perception server-side, so this only maps
//! each [`SendTarget`] onto the session layer (character fan-out with `Arc`
//! sharing — zero-copy — a single connection, or a global broadcast). Slow-client
//! detection lives here because the gateway owns the connection output buffers.
//!
//! ## Shared policing core
//!
//! The slow-client policing + link-dead reaping is a single implementation
//! ([`transmit_and_police`] + [`spawn_linkdead_reaper`], producing
//! transport-neutral [`ConnDownPost`]s) shared by BOTH delivery paths: this
//! crate's [`SessionDelivery`] (split gateway) and `mushd`'s in-process sink
//! (all-in-one). Only the feedback destination differs — each caller supplies
//! closures that post the policed `ConnDown`/`SessionGone` through its own
//! transport (here: the boundary writer as [`ToServer::Inbound`]; all-in-one:
//! the world loop's own inbound queue). The shared core lives here rather than
//! in `mush-session` because it needs the `mush-core`/`mush-ipc` vocabulary
//! (`ObjId`, `DisconnectKind`, `SendTarget`) and spawns tokio timers, both of
//! which `mush-session` deliberately does without; `mushd` already depends on
//! this crate, and `mush-server` does not, so hosting it here keeps the
//! forked-server dependency set clean.

use std::future::Future;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mush_core::{DisconnectKind, ObjId};
use mush_ipc::{SendTarget, ToServer, WireInbound};
use mush_proto::OutputEvent;
use mush_session::{CharKey, ConnId, DetachOutcome, DetachReason, SessionManager};

use crate::ToServerTx;

/// Widen a core [`ObjId`] to the session layer's opaque [`CharKey`].
#[must_use]
pub fn char_key(id: ObjId) -> CharKey {
    CharKey(u64::from(id.0))
}

/// Narrow a [`CharKey`] back to the core [`ObjId`] it was widened from.
#[must_use]
pub fn char_key_obj(key: CharKey) -> ObjId {
    ObjId(u32::try_from(key.0).unwrap_or(u32::MAX))
}

/// Deliver one event straight to a connection, bypassing the world task (login
/// screens, error notices). A **gateway-side** helper: the SSH/web bridges own
/// the [`SessionManager`] and call it before a connection has attached to an
/// actor.
pub fn send_event_to_conn(sessions: &SessionManager, conn: ConnId, event: OutputEvent) {
    let _ = sessions.send_to_conn(conn, Arc::new(event));
}

/// One policed slow connection's engine-bound `ConnDown`, transport-neutral: its
/// detach already happened synchronously; this carries only the inbound the
/// engine must step. Each delivery path maps it onto its own inbound vocabulary
/// ([`ToServer::Inbound`] over the boundary, or the all-in-one world loop's
/// queue).
pub struct ConnDownPost {
    /// The connection that overflowed and was detached.
    pub conn: u64,
    /// The character it was bound to.
    pub actor: ObjId,
    /// Whether this was the actor's last connection (`Boot`) or not (`Lost`).
    pub reason: DisconnectKind,
}

/// Route a resolved delivery batch onto `sessions` and detach every overflowed
/// connection as a slow client (D5), in the deterministic sorted order, arming a
/// link-dead reaper (via `spawn_reaper`, called with the session's grace) for
/// each session the detach left link-dead. Returns the resulting
/// [`ConnDownPost`]s in that order for the caller to post through its own
/// transport.
///
/// The batch carries `Arc<OutputEvent>` so a `Room` event resolved once is
/// delivered to every recipient without a clone.
pub fn transmit_and_police(
    sessions: &Arc<SessionManager>,
    sends: Vec<(SendTarget, Arc<OutputEvent>)>,
    mut spawn_reaper: impl FnMut(Duration),
) -> Vec<ConnDownPost> {
    let mut slow = Vec::new();
    for (target, ev) in sends {
        match target {
            SendTarget::Char(ch) => {
                let report = sessions.send_to_char(CharKey(ch), ev);
                if report.delivered == 0 && report.dropped.is_empty() {
                    tracing::debug!(char = ch, "output to actor with no live connections dropped");
                }
                slow.extend(report.dropped);
            }
            SendTarget::Conn(conn) => {
                let _ = sessions.send_to_conn(ConnId(conn), ev);
            }
            SendTarget::All => {
                let report = sessions.broadcast_all(ev);
                slow.extend(report.dropped);
            }
        }
    }
    slow.sort_unstable();
    slow.dedup();
    // Detach each overflowed connection (synchronously) in the sorted order,
    // arming its reaper if it went link-dead, and hand the `ConnDown`s back to
    // the caller to post in that same deterministic order.
    slow.into_iter()
        .filter_map(|conn| police_slow_client(sessions, conn, &mut spawn_reaper))
        .collect()
}

/// Detach one overflowed connection as a slow client and return the engine's
/// `ConnDown` for the caller to post (D5). Dropping the session's handle inside
/// [`SessionManager::detach`] closes its transport once the queue drains. Calls
/// `spawn_reaper(grace)` when the detach left the session link-dead. Returns
/// `None` when the connection was already gone or carried no resolvable actor.
fn police_slow_client(
    sessions: &Arc<SessionManager>,
    conn: ConnId,
    spawn_reaper: &mut impl FnMut(Duration),
) -> Option<ConnDownPost> {
    let actor = sessions
        .session_of_conn(conn)
        .and_then(|s| sessions.character_of(s))
        .map(char_key_obj);
    let outcome = sessions.detach(conn, DetachReason::SlowClient)?;
    tracing::warn!(conn = conn.0, "slow client disconnected (output buffer overflow)");
    let actor = actor?;
    let reason = match outcome {
        DetachOutcome::Closed => DisconnectKind::Boot,
        DetachOutcome::OthersRemain | DetachOutcome::Linkdead { .. } => DisconnectKind::Lost,
    };
    if let DetachOutcome::Linkdead { grace } = outcome {
        spawn_reaper(grace);
    }
    Some(ConnDownPost {
        conn: conn.0,
        actor,
        reason,
    })
}

/// After `grace`, reap every expired link-dead session and feed each reaped
/// actor to `on_expired` (D5). A reconnect that races the timer revives the
/// session and [`SessionManager::take_expired`] skips it. `on_expired` returns
/// [`ControlFlow::Break`] to stop the sweep early (the all-in-one's weak-sender
/// path uses this when the world loop's inbound queue is already gone).
pub fn spawn_linkdead_reaper<F, Fut>(sessions: Arc<SessionManager>, grace: Duration, mut on_expired: F)
where
    F: FnMut(ObjId) -> Fut + Send + 'static,
    Fut: Future<Output = ControlFlow<()>> + Send,
{
    tokio::spawn(async move {
        tokio::time::sleep(grace).await;
        for expired in sessions.take_expired(Instant::now()) {
            let actor = char_key_obj(expired.character);
            tracing::info!(%actor, account = %expired.account, "linkdead grace expired");
            if on_expired(actor).await.is_break() {
                return;
            }
        }
    });
}

/// The gateway's server → gateway delivery transport (see the module docs).
#[derive(Clone)]
pub struct SessionDelivery {
    sessions: Arc<SessionManager>,
    /// Where policed connections' `ConnDown`/`SessionGone` are posted: the
    /// boundary writer, crossing back to the forked server as inbound. The
    /// wrapper bumps the shared `to_server` depth gauge on every post (M18), so
    /// policed feedback counts toward the queue occupancy like any producer.
    boundary: ToServerTx,
}

impl SessionDelivery {
    /// Wrap the gateway's session manager and the boundary writer's inbox.
    #[must_use]
    pub(crate) fn new(sessions: Arc<SessionManager>, boundary: ToServerTx) -> SessionDelivery {
        SessionDelivery { sessions, boundary }
    }

    /// Route a resolved delivery batch onto the session manager, self-policing
    /// overflowed connections (D5), and post each policed `ConnDown` back to the
    /// server over the boundary in the deterministic sorted order.
    pub fn transmit(&self, sends: Vec<(SendTarget, Arc<OutputEvent>)>) {
        let reaper_sessions = Arc::clone(&self.sessions);
        let reaper_boundary = self.boundary.clone();
        let posts = transmit_and_police(&self.sessions, sends, move |grace| {
            let boundary = reaper_boundary.clone();
            spawn_linkdead_reaper(Arc::clone(&reaper_sessions), grace, move |actor| {
                let boundary = boundary.clone();
                async move {
                    let _ = boundary.send(ToServer::Inbound(WireInbound::SessionGone { actor }));
                    ControlFlow::Continue(())
                }
            });
        });
        for post in posts {
            let _ = self.boundary.send(ToServer::Inbound(WireInbound::ConnDown {
                conn: post.conn,
                actor: post.actor,
                reason: post.reason,
            }));
        }
    }

    /// Set/clear a character's flood-exempt flag on the gateway throttle.
    pub fn set_flood_exempt(&self, char: u64, exempt: bool) {
        self.sessions.set_flood_exempt(CharKey(char), exempt);
    }
}

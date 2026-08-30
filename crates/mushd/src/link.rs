//! The gateway<->server boundary transport (milestone M16a).
//!
//! `mushd` already has an in-process seam between the **gateway** (connection
//! I/O: the SSH/web bridges + [`mush_session::SessionManager`]) and the
//! **server** (the single-writer world task owning the [`mush_core::Engine`]).
//! M16 splits that seam across a process boundary for zero-downtime restarts;
//! this module is the SAFE FIRST STAGE (M16a): it makes the two halves speak
//! the [`mush_ipc`] protocol **in-process**, so the boundary is one
//! socket-swap away from the fork while the single binary keeps behaving
//! byte-for-byte identically.
//!
//! Two directions, two seam objects:
//!
//! - [`GatewayLink`] — **gateway → server**. The bridges hold an
//!   `Arc<dyn GatewayLink>` instead of a concrete [`WorldHandle`] and send
//!   [`ToServer`] messages / correlated [`ServiceReq`]s through it. The
//!   in-process impl ([`impl GatewayLink for WorldHandle`](WorldHandle))
//!   converts each protocol message back into a [`WorldMsg`](crate::world_task::WorldMsg)
//!   and sends it over the existing mpsc; M16b swaps in a socket client that
//!   serializes [`ToServer`] and reads [`ToGateway::Reply`](mush_ipc::ToGateway)
//!   back off the wire.
//! - [`InProcessTransport`] — **server → gateway**. The world loop's
//!   `resolve_sends` (which stays server-side, reading `World` for the
//!   M11/M14 perception gating) produces `(SendTarget, OutputEvent)` pairs —
//!   each one a [`ToGateway::Deliver`](mush_ipc::ToGateway) — and hands them
//!   here; [`InProcessTransport::transmit`] is the gateway-side consumer that
//!   maps each [`SendTarget`] onto the [`SessionManager`]. M16b replaces this
//!   with the socket writer that serializes [`ToGateway`](mush_ipc::ToGateway)
//!   frames and moves slow-client detection to the gateway side.
//!
//! The in-process delivery batch is carried as `(SendTarget, Arc<OutputEvent>)`
//! rather than owned [`ToGateway::Deliver`](mush_ipc::ToGateway) values on
//! purpose: a `Room` audience fans one `Arc` out to every recipient with no
//! per-recipient clone, so the hot path keeps its zero-copy behavior (invariant
//! S1: no perf regression). That pair *is* the content of a `ToGateway::Deliver`
//! stream, minus the MessagePack framing M16b will add — and the codec that
//! carries the real bytes is already proven by `mush-ipc`'s round-trip tests.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use mush_core::{DisconnectKind, Inbound, ObjId};
use mush_ipc::{
    DetachCause, SendTarget, ServiceReq, ServiceResp, ToGateway, ToServer, WireInbound,
};
use mush_proto::OutputEvent;
use mush_session::{CharKey, ConnId, DetachOutcome, DetachReason, SessionManager};
use tokio::sync::mpsc::{UnboundedSender, WeakSender};

use crate::world_task::{WorldHandle, WorldMsg, char_key_obj};

/// The gateway's protocol-typed handle to the server half.
///
/// Everything the gateway sends across the boundary goes through this trait,
/// so the bridges name only the [`mush_ipc`] vocabulary — never the in-process
/// [`WorldHandle`] / [`WorldMsg`](crate::world_task::WorldMsg) plumbing. The
/// one in-process implementor is [`WorldHandle`]; M16b adds a socket-client
/// implementor that talks to the forked server process.
///
/// Object-safe (returns [`BoxFuture`], the workspace's `ObjectStore` idiom)
/// so the bridges can hold an `Arc<dyn GatewayLink>`.
pub trait GatewayLink: Send + Sync {
    /// Send one one-way [`ToServer`] message — a live-input [`WireInbound`]
    /// arm ([`ToServer::Inbound`]) or operator [`ToServer::Control`]. Resolves
    /// to `false` when the server is gone (shutdown in progress).
    ///
    /// [`WireInbound`]: mush_ipc::WireInbound
    fn to_server(&self, msg: ToServer) -> BoxFuture<'_, bool>;

    /// Issue a correlated service request ([`ServiceReq`]) and await its
    /// [`ServiceResp`]. Resolves to `None` only when the request could not be
    /// delivered at all; per-request failure (e.g. player-creation refusal) is
    /// carried inside the returned [`ServiceResp`].
    fn request(&self, req: ServiceReq) -> BoxFuture<'_, Option<ServiceResp>>;
}

/// The in-process [`GatewayLink`]: converts each protocol message back into a
/// [`WorldMsg`](crate::world_task::WorldMsg) and sends it over the existing
/// world-task mpsc, preserving byte-for-byte behavior. The correlated service
/// requests reuse [`WorldHandle`]'s oneshot round-trips (id correlation is the
/// transport's job in M16b; in-process the oneshot *is* the correlation).
impl GatewayLink for WorldHandle {
    fn to_server(&self, msg: ToServer) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            match msg {
                // The cross-boundary input subset: convert back to the shared
                // `Inbound` and queue it exactly as the direct call did.
                ToServer::Inbound(wire) => self.inbound(Inbound::from(wire)).await,
                // Operator control (`@shutdown`, `@dump`) crosses as the
                // top-level `ToServer::Control`; in-process it re-enters the
                // world loop as `Inbound::Control`, the same message the
                // console path sends.
                ToServer::Control(ctrl) => self.inbound(Inbound::Control(ctrl)).await,
                // Session bookkeeping (Attach/Detach/ReAnnounce) exists for the
                // standalone gateway of M16b/M16c to rebuild the server's
                // connection map after a restart. In-process the
                // `SessionManager` is shared, so the server already learns of
                // every attach/detach through `ConnUp`/`ConnDown`; these arms
                // are therefore never sent on the in-process path. Accept and
                // ignore so an early sender is a no-op, not a panic.
                ToServer::Attach { .. } | ToServer::Detach { .. } | ToServer::ReAnnounce(_) => true,
                // A correlated request must go through `request` (it needs a
                // reply); routing one here is a programming error.
                ToServer::Request { .. } => {
                    debug_assert!(false, "ToServer::Request must be sent via GatewayLink::request");
                    true
                }
            }
        })
    }

    fn request(&self, req: ServiceReq) -> BoxFuture<'_, Option<ServiceResp>> {
        Box::pin(async move {
            Some(match req {
                ServiceReq::CreatePlayer { name } => {
                    ServiceResp::CreatePlayer(self.create_player(&name).await.map_err(Into::into))
                }
                ServiceReq::NameFree { name } => ServiceResp::NameFree(self.name_free(&name).await),
                ServiceReq::InInputContext { actor } => {
                    ServiceResp::InInputContext(self.in_input_context(actor).await)
                }
                ServiceReq::Barrier => {
                    self.barrier().await;
                    ServiceResp::Barrier
                }
            })
        })
    }
}

/// The in-process server → gateway delivery transport.
///
/// Holds the shared [`SessionManager`]; [`transmit`](InProcessTransport::transmit)
/// is the gateway-side consumer of the [`ToGateway::Deliver`](mush_ipc::ToGateway)
/// payloads the world loop's `resolve_sends` produced. This is the swap point
/// for M16b: the standalone gateway replaces this with a socket reader that
/// decodes `ToGateway` frames and runs the same [`SendTarget`] → session
/// routing on its own `SessionManager`.
/// Where a [`InProcessTransport`] routes the server's output.
///
/// The all-in-one binary uses [`Sink::Session`] — the shared
/// [`SessionManager`], exactly as before. M16b's forked server uses
/// [`Sink::Socket`], which serializes each server→gateway message as a
/// [`ToGateway`] frame onto the boundary; the standalone `mush-gateway` decodes
/// them and runs the [`SendTarget`] → session routing on *its* manager.
#[derive(Clone)]
enum Sink {
    /// In-process: the shared session manager (all-in-one `mushd`).
    Session(Arc<SessionManager>),
    /// Cross-process: the socket writer task's inbox (`mush-server`).
    Socket(UnboundedSender<ToGateway>),
}

/// Where a [`Sink::Session`] transport posts the `ConnDown`/`SessionGone` its
/// slow-client policing and link-dead reaper generate (D5).
///
/// The policing is one implementation, shared by both transports' session sink
/// — the all-in-one world loop's transport and the forked gateway's transport
/// both route delivery through the same [`InProcessTransport::transmit`]. Only
/// the direction the resulting inbound travels differs:
///
/// - [`Local`](InboundFeedback::Local) — all-in-one: straight into the world
///   loop's own inbound queue. Held as a [`WeakSender`] (never a strong
///   [`WorldHandle`]): the world loop keeps this transport for its whole
///   lifetime, so a strong sender here would keep its own inbound channel alive
///   forever and the loop could never observe shutdown. Sent from a detached
///   task so the synchronous `transmit` (which runs *on* the world loop) never
///   awaits its own bounded channel.
/// - [`Boundary`](InboundFeedback::Boundary) — split gateway: serialized as a
///   [`ToServer::Inbound`] onto the boundary writer, crossing back to the forked
///   server exactly as any other gateway→server input.
#[derive(Clone)]
enum InboundFeedback {
    /// All-in-one: post into the world loop's inbound queue (weak, so it never
    /// keeps the loop alive).
    Local(WeakSender<WorldMsg>),
    /// Split gateway: serialize as `ToServer::Inbound` onto the boundary writer.
    Boundary(UnboundedSender<ToServer>),
}

/// One policed slow connection's engine-bound `ConnDown`. Its detach already
/// happened synchronously inside [`police_slow_client`]; this is only the
/// inbound the engine must step to run the announce + disconnect bookkeeping.
struct ConnDownPost {
    conn: u64,
    actor: ObjId,
    reason: DisconnectKind,
}

impl InboundFeedback {
    /// Post a batch of policed connections' `ConnDown`s back to the engine **in
    /// order** — the deterministic sorted order [`transmit`] produced (it
    /// sorts+dedups the overflow list before policing). Ordering matters when
    /// several connections overflow in one delivery pass: the all-in-one link
    /// used to step each `ConnDown` synchronously in that sorted order, so the
    /// batch is posted through a SINGLE task here rather than one racing task
    /// per connection.
    fn post_conn_downs(&self, posts: Vec<ConnDownPost>) {
        if posts.is_empty() {
            return;
        }
        match self {
            InboundFeedback::Local(weak) => {
                // `transmit` is synchronous and runs on the world loop; spawn so
                // the bounded inbound sends are never awaited from inside the
                // loop (they are picked up on the loop's next iterations). One
                // task drains the whole batch in order, preserving the
                // deterministic ordering the synchronous path had. Upgrade only
                // transiently — a live sender must never outlive the loop.
                let Some(tx) = weak.upgrade() else { return };
                tokio::spawn(async move {
                    for p in posts {
                        let _ = tx
                            .send(WorldMsg::Inbound(Inbound::ConnDown {
                                conn: p.conn,
                                actor: p.actor,
                                reason: p.reason,
                            }))
                            .await;
                    }
                });
            }
            InboundFeedback::Boundary(tx) => {
                for p in posts {
                    let _ = tx.send(ToServer::Inbound(WireInbound::ConnDown {
                        conn: p.conn,
                        actor: p.actor,
                        reason: p.reason,
                    }));
                }
            }
        }
    }

    /// Post a `SessionGone` for a reaped link-dead session back to the engine.
    fn session_gone(&self, actor: ObjId) {
        match self {
            InboundFeedback::Local(weak) => {
                let Some(tx) = weak.upgrade() else { return };
                tokio::spawn(async move {
                    let _ = tx
                        .send(WorldMsg::Inbound(Inbound::SessionGone { actor }))
                        .await;
                });
            }
            InboundFeedback::Boundary(tx) => {
                let _ = tx.send(ToServer::Inbound(WireInbound::SessionGone { actor }));
            }
        }
    }
}

/// Detach one overflowed connection as a slow client and return the engine's
/// `ConnDown` for the caller to post (D5). Shared by both transports' session
/// sink, so neither the world loop nor the gateway reader reimplements it.
/// Dropping the session's handle inside [`SessionManager::detach`] closes its
/// transport once the queue drains. Arms a link-dead reaper when the detach
/// left the session link-dead. Returns `None` (nothing to post) when the
/// connection was already gone or carried no resolvable actor.
///
/// The detach happens synchronously here; the returned `ConnDown` is posted by
/// [`transmit`] as one ordered batch so simultaneous overflows keep their
/// deterministic order.
fn police_slow_client(
    sessions: &Arc<SessionManager>,
    feedback: &InboundFeedback,
    conn: ConnId,
) -> Option<ConnDownPost> {
    // Resolve the actor before the detach forgets the mapping.
    let actor = sessions
        .session_of_conn(conn)
        .and_then(|s| sessions.character_of(s))
        .map(char_key_obj);
    let Some(outcome) = sessions.detach(conn, DetachReason::SlowClient) else {
        return None; // already detached (duplicate report or concurrent close)
    };
    tracing::warn!(conn = conn.0, "slow client disconnected (output buffer overflow)");
    let actor = actor?;
    let reason = match outcome {
        // Session already gone underneath us: clean full disconnect.
        DetachOutcome::Closed => DisconnectKind::Boot,
        DetachOutcome::OthersRemain | DetachOutcome::Linkdead { .. } => DisconnectKind::Lost,
    };
    if let DetachOutcome::Linkdead { grace } = outcome {
        arm_linkdead_reaper(Arc::clone(sessions), feedback.clone(), grace);
    }
    Some(ConnDownPost {
        conn: conn.0,
        actor,
        reason,
    })
}

/// After `grace`, reap every expired link-dead session and feed the engine its
/// `SessionGone` (D5). The gateway layer owns the expiry timers now, so this is
/// the one reaper the slow-client path arms — shared by both transports. A
/// reconnect that races the timer revives the session and
/// [`SessionManager::take_expired`] skips it.
fn arm_linkdead_reaper(sessions: Arc<SessionManager>, feedback: InboundFeedback, grace: Duration) {
    tokio::spawn(async move {
        tokio::time::sleep(grace).await;
        for expired in sessions.take_expired(Instant::now()) {
            let actor = char_key_obj(expired.character);
            tracing::info!(%actor, account = %expired.account, "linkdead grace expired");
            feedback.session_gone(actor);
        }
    });
}

#[derive(Clone)]
pub struct InProcessTransport {
    sink: Sink,
    /// Slow-client / link-dead policing feedback (a [`Sink::Session`] transport
    /// only). When present, [`transmit`](InProcessTransport::transmit)
    /// self-polices overflowed connections here (D5) rather than returning them
    /// for a caller to police; the world loop therefore never touches the
    /// session layer to police. `None` on a [`Sink::Socket`] transport (the
    /// forked server never polices — its gateway does) and on a bare test
    /// transport (which gets the slow list back to assert on).
    feedback: Option<InboundFeedback>,
}

impl InProcessTransport {
    /// Wrap the shared session manager as the in-process gateway. No policing
    /// feedback is wired yet: [`spawn_persist_with_gateway`] attaches the
    /// world-loop feedback once the inbound queue exists
    /// ([`with_local_feedback`](InProcessTransport::with_local_feedback)), and
    /// the split gateway attaches boundary feedback
    /// ([`with_boundary_feedback`](InProcessTransport::with_boundary_feedback)).
    /// Left bare (as tests build it), `transmit` returns the slow list for the
    /// caller instead of self-policing.
    ///
    /// [`spawn_persist_with_gateway`]: crate::world_task::spawn_persist_with_gateway
    #[must_use]
    pub fn new(sessions: Arc<SessionManager>) -> InProcessTransport {
        InProcessTransport {
            sink: Sink::Session(sessions),
            feedback: None,
        }
    }

    /// The forked-server gateway (M16b): every server→gateway message is
    /// serialized as a [`ToGateway`] frame onto `tx`, which a socket-writer
    /// task drains to the boundary. No [`SessionManager`] is held server-side
    /// in this mode — the gateway process owns it, so this transport never
    /// polices.
    #[must_use]
    pub fn new_socket(tx: UnboundedSender<ToGateway>) -> InProcessTransport {
        InProcessTransport {
            sink: Sink::Socket(tx),
            feedback: None,
        }
    }

    /// Attach the all-in-one slow-client / link-dead feedback: policed
    /// connections' `ConnDown`/`SessionGone` go into the world loop's own
    /// inbound queue via `tx` (a [`WeakSender`], so this transport — which the
    /// world loop owns for its whole lifetime — never keeps the loop's inbound
    /// channel alive). A no-op on a [`Sink::Socket`] transport (the forked
    /// server does not police).
    pub fn with_local_feedback(&mut self, tx: WeakSender<WorldMsg>) {
        if matches!(self.sink, Sink::Session(_)) {
            self.feedback = Some(InboundFeedback::Local(tx));
        }
    }

    /// Attach the split-gateway slow-client / link-dead feedback: policed
    /// connections' `ConnDown`/`SessionGone` are serialized as
    /// [`ToServer::Inbound`] onto the boundary writer `tx`. A no-op on a
    /// [`Sink::Socket`] transport.
    pub fn with_boundary_feedback(&mut self, tx: UnboundedSender<ToServer>) {
        if matches!(self.sink, Sink::Session(_)) {
            self.feedback = Some(InboundFeedback::Boundary(tx));
        }
    }

    /// Whether this transport writes to the boundary socket (forked server)
    /// rather than a local [`SessionManager`]. The world loop's session-control
    /// paths (`@boot`) branch on this: in socket mode they emit a fire-and-forget
    /// [`ToGateway::CloseSession`] the gateway resolves locally, instead of
    /// reaching into a `SessionManager` the forked server does not hold.
    #[must_use]
    pub fn is_socket(&self) -> bool {
        matches!(self.sink, Sink::Socket(_))
    }

    /// Route a resolved delivery batch onto the session manager, returning the
    /// connections whose output buffer overflowed (slow clients) for the
    /// caller to police.
    ///
    /// Each `(SendTarget, event)` pair is one
    /// [`ToGateway::Deliver`](mush_ipc::ToGateway); world-side audience
    /// resolution (room membership, `@ignore`, M11/M14 perception gating)
    /// already ran in `resolve_sends`, so this only maps the [`SendTarget`]
    /// back onto the session layer (character fan-out, single connection, or
    /// global broadcast). The batch carries `Arc<OutputEvent>` so a `Room`
    /// event resolved once is delivered to every recipient without a clone.
    pub fn transmit(&self, sends: Vec<(SendTarget, Arc<OutputEvent>)>) -> Vec<ConnId> {
        let sessions = match &self.sink {
            Sink::Session(s) => s,
            // Forked server: serialize each pair as a `ToGateway::Deliver`
            // frame and hand it to the socket writer. Slow-client detection is
            // the gateway's job (it owns the connection buffers), so the socket
            // path reports no local slow clients and never polices (D5).
            Sink::Socket(tx) => {
                for (target, ev) in sends {
                    let _ = tx.send(ToGateway::Deliver {
                        target,
                        event: (*ev).clone(),
                    });
                }
                return Vec::new();
            }
        };
        let mut slow = Vec::new();
        for (target, ev) in sends {
            match target {
                SendTarget::Char(ch) => {
                    let report = sessions.send_to_char(CharKey(ch), ev);
                    if report.delivered == 0 && report.dropped.is_empty() {
                        // Link-dead actor (or a race with a disconnect): the
                        // event is lost. Resume buffers are an M2+ concern
                        // (PLAN: mush-session "resume buffers"); make the loss
                        // observable at least.
                        tracing::debug!(
                            char = ch,
                            "output to actor with no live connections dropped"
                        );
                    }
                    slow.extend(report.dropped);
                }
                SendTarget::Conn(conn) => {
                    // Pre-auth / connection-addressed events. An unknown conn is
                    // fine (it may have raced a disconnect).
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
        // D5: self-police the overflowed connections here, in the gateway-layer
        // code both transports' session sink shares — the world loop no longer
        // reaches into the session manager to disconnect slow clients. A bare
        // transport with no feedback wired (tests) hands the list back instead.
        match &self.feedback {
            Some(feedback) => {
                // Detach each overflowed connection (synchronously) in the
                // sorted order above, collecting their `ConnDown`s, then post
                // the whole batch back to the engine in that order — a single
                // ordered hand-off, so simultaneous overflows keep the
                // deterministic ordering the pre-fork synchronous path had.
                let posts: Vec<ConnDownPost> = slow
                    .into_iter()
                    .filter_map(|conn| police_slow_client(sessions, feedback, conn))
                    .collect();
                feedback.post_conn_downs(posts);
                Vec::new()
            }
            None => slow,
        }
    }

    /// Route one out-of-band delivery through the seam — a single
    /// [`ToGateway::Deliver`](mush_ipc::ToGateway).
    ///
    /// The world loop's audience-resolved output goes through
    /// [`transmit`](InProcessTransport::transmit); this is the path for the
    /// server's *unaddressed* one-off sends (read-only refusals, wizard
    /// store-health broadcasts, staff-op confirmations) that are not driven by
    /// an `Audience` but still cross the boundary as `Deliver`. Slow-client
    /// policing is intentionally skipped for these handful-of-lines sends (the
    /// session layer still counts any drops), matching their prior direct
    /// `send_to_char`/`send_to_conn` behavior.
    pub fn deliver(&self, target: SendTarget, event: Arc<OutputEvent>) {
        let _ = self.transmit(vec![(target, event)]);
    }

    /// Apply a [`ToGateway::SetFloodExempt`](mush_ipc::ToGateway): set or clear
    /// a character's flood-exempt flag on the gateway's throttle.
    ///
    /// The world loop drives this after each `Cmd` so a paste into an input
    /// context bypasses the burst cap. In-process it is the same
    /// [`SessionManager::set_flood_exempt`] call the world loop used to make
    /// directly; M16b serializes the arm across the socket instead.
    pub fn set_flood_exempt(&self, char: u64, exempt: bool) {
        match &self.sink {
            Sink::Session(s) => s.set_flood_exempt(CharKey(char), exempt),
            Sink::Socket(tx) => {
                let _ = tx.send(ToGateway::SetFloodExempt { char, exempt });
            }
        }
    }

    /// Emit a fire-and-forget [`ToGateway::CloseSession`]: the gateway resolves
    /// `char` → its live connections on *its* [`SessionManager`], drops them,
    /// and reports each resulting `ConnDown` back to the server as inbound. Used
    /// by `@boot` in split mode instead of the in-process `close_session` walk —
    /// the world loop never blocks awaiting a reply (D4).
    ///
    /// A no-op in [`Sink::Session`] mode: the all-in-one path closes sessions
    /// directly through the shared manager and never calls this.
    pub fn close_session(&self, char: u64, cause: DetachCause) {
        if let Sink::Socket(tx) = &self.sink {
            let _ = tx.send(ToGateway::CloseSession { char, cause });
        }
    }

    /// Emit a fire-and-forget [`ToGateway::Detach`] for one connection
    /// (slow-client policing in split mode). A no-op in [`Sink::Session`] mode.
    pub fn detach_conn(&self, conn: u64, cause: DetachCause) {
        if let Sink::Socket(tx) = &self.sink {
            let _ = tx.send(ToGateway::Detach { conn, cause });
        }
    }

    /// Emit a fire-and-forget [`ToGateway::CloseWebSessions`]: the gateway
    /// resolves each character to its live connections on *its*
    /// [`SessionManager`], delivers `notice` (if any) to each **structured**
    /// (web/token) connection, drops it, and reports each resulting `ConnDown`
    /// back to the server as inbound. Used by `@newpassword` logout-everywhere
    /// in split mode instead of the in-process walk of a session manager the
    /// forked server does not hold — the world round-trip never blocks awaiting
    /// a reply (D4).
    ///
    /// A no-op in [`Sink::Session`] mode: the all-in-one path walks the shared
    /// manager directly and never calls this.
    pub fn close_web_sessions(
        &self,
        chars: Vec<u64>,
        notice: Option<OutputEvent>,
        cause: DetachCause,
    ) {
        if let Sink::Socket(tx) = &self.sink {
            let _ = tx.send(ToGateway::CloseWebSessions {
                chars,
                notice,
                cause,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    use mush_proto::{ClientCaps, ClientKind, ColorDepth, OutputEvent, RichText};
    use mush_session::ConnectionHandle;
    use tokio::sync::mpsc;

    fn caps() -> ClientCaps {
        ClientCaps {
            kind: ClientKind::Ssh,
            color: ColorDepth::Ansi256,
            width: 80,
            height: 24,
            utf8: true,
            structured: false,
        }
    }

    fn sys(text: &str) -> Arc<OutputEvent> {
        Arc::new(OutputEvent::Sys(RichText::plain(text)))
    }

    /// D5 in SPLIT mode: a session-sink transport wired with *boundary* feedback
    /// self-polices an overflowed connection — it detaches the slow client and
    /// emits a `ToServer::Inbound(ConnDown)` onto the boundary writer, without
    /// the world loop (a different process here) touching the session layer.
    #[tokio::test]
    async fn boundary_transport_polices_slow_client_over_the_seam() {
        let sessions = Arc::new(SessionManager::new());
        let char: u64 = 5;
        // A tiny (1-slot) output buffer that nobody drains: the second event to
        // this character overflows it.
        let (tx, _rx) = mpsc::channel::<Arc<OutputEvent>>(1);
        let handle = ConnectionHandle::new(ConnId(9), IpAddr::V4(Ipv4Addr::LOCALHOST), caps(), tx);
        sessions.attach(handle, CharKey(char), "acct".to_string());

        // Boundary feedback: the split gateway's slow-client ConnDown is a
        // `ToServer::Inbound` on this channel.
        let (to_server_tx, mut to_server_rx) = mpsc::unbounded_channel::<ToServer>();
        let mut transport = InProcessTransport::new(Arc::clone(&sessions));
        transport.with_boundary_feedback(to_server_tx);

        // Two events to the character in one pass: the first fills the buffer,
        // the second overflows it and triggers the policing.
        transport.transmit(vec![
            (SendTarget::Char(char), sys("first")),
            (SendTarget::Char(char), sys("overflow")),
        ]);

        // The slow connection was detached locally...
        assert!(
            sessions.session_of_conn(ConnId(9)).is_none(),
            "the overflowed connection must be detached by the gateway transport"
        );
        // ...and its ConnDown was reported back across the seam as inbound.
        let msg = to_server_rx.try_recv().expect("a ConnDown crossed the seam");
        match msg {
            ToServer::Inbound(WireInbound::ConnDown { conn, reason, .. }) => {
                assert_eq!(conn, 9);
                // Last-connection unclean detach → link-dead → Lost.
                assert_eq!(reason, DisconnectKind::Lost);
            }
            other => panic!("expected ConnDown over the seam, got {other:?}"),
        }
    }

    /// F1 (all-in-one ordering pinned): when several connections overflow in a
    /// single delivery pass, the all-in-one `Local` feedback posts their
    /// `ConnDown`s back to the world loop's inbound queue in the deterministic
    /// **sorted** connection order — the same order the pre-fork synchronous
    /// path stepped them — rather than in the racing order of one spawned task
    /// per connection. Connections are attached out of order (30, 10, 20) to
    /// prove the sort.
    #[tokio::test]
    async fn local_feedback_posts_conn_downs_in_sorted_order() {
        let sessions = Arc::new(SessionManager::new());
        // Each connection on its own character with a 1-slot buffer nobody
        // drains; keep every receiver alive so the channels read full, not
        // closed.
        let specs = [(30u64, 100u64), (10, 101), (20, 102)];
        let mut keep = Vec::new();
        for (conn, ch) in specs {
            let (tx, rx) = mpsc::channel::<Arc<OutputEvent>>(1);
            keep.push(rx);
            let handle =
                ConnectionHandle::new(ConnId(conn), IpAddr::V4(Ipv4Addr::LOCALHOST), caps(), tx);
            sessions.attach(handle, CharKey(ch), "acct".to_string());
        }

        // Stand in for the world loop's inbound queue; the Local feedback holds
        // a weak sender, so keep the strong end alive for the pass.
        let (world_tx, mut world_rx) = mpsc::channel::<WorldMsg>(64);
        let mut transport = InProcessTransport::new(Arc::clone(&sessions));
        transport.with_local_feedback(world_tx.downgrade());

        // Two events per character in ONE pass: the first fills the buffer, the
        // second overflows it and marks the connection slow.
        let mut batch = Vec::new();
        for (_conn, ch) in specs {
            batch.push((SendTarget::Char(ch), sys("fill")));
            batch.push((SendTarget::Char(ch), sys("overflow")));
        }
        transport.transmit(batch);

        // The single batch task posts the three ConnDowns in sorted conn order.
        let mut got = Vec::new();
        for _ in 0..specs.len() {
            match world_rx.recv().await.expect("a ConnDown must be posted") {
                WorldMsg::Inbound(Inbound::ConnDown { conn, .. }) => got.push(conn),
                _ => panic!("expected a ConnDown inbound"),
            }
        }
        assert_eq!(
            got,
            vec![10, 20, 30],
            "simultaneous overflows must post ConnDowns in deterministic sorted order"
        );
        drop(keep);
    }

    /// A bare session-sink transport (no feedback wired, as some tests build it)
    /// returns the slow list to the caller rather than self-policing.
    #[tokio::test]
    async fn bare_transport_returns_slow_list() {
        let sessions = Arc::new(SessionManager::new());
        let char: u64 = 6;
        let (tx, _rx) = mpsc::channel::<Arc<OutputEvent>>(1);
        let handle = ConnectionHandle::new(ConnId(3), IpAddr::V4(Ipv4Addr::LOCALHOST), caps(), tx);
        sessions.attach(handle, CharKey(char), "acct".to_string());

        let transport = InProcessTransport::new(Arc::clone(&sessions));
        let slow = transport.transmit(vec![
            (SendTarget::Char(char), sys("first")),
            (SendTarget::Char(char), sys("overflow")),
        ]);
        assert_eq!(slow, vec![ConnId(3)], "bare transport hands the slow list back");
        // No feedback wired, so the transport did NOT detach it.
        assert!(sessions.session_of_conn(ConnId(3)).is_some());
    }
}

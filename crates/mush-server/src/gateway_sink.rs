//! The server-side view of the server → gateway delivery seam.
//!
//! The world loop produces `(SendTarget, OutputEvent)` pairs (audience already
//! resolved server-side) plus fire-and-forget session control, and hands them to
//! a [`GatewaySink`]. Two implementors exist:
//!
//! - [`SocketSink`] (here, forked server): routes every message to the gateway
//!   links that should see it (doc 33 §4.3, cut GG.2) and serializes each as a
//!   [`ToGateway`] frame onto that link's writer through its [`LinkSink`]. It
//!   owns no `SessionManager` — the gateway processes do — so it never polices
//!   and the session-walk / count methods are empty.
//! - the in-process sink (in `mushd`, the all-in-one): routes each pair onto the
//!   shared `mush_session::SessionManager` with `Arc` fan-out
//!   (zero-copy), self-polices slow clients back into the world loop, and
//!   answers the session walks / counts against the real manager.
//!
//! The trait is defined here (server-side) so the world loop names no
//! `mush-session` type; its in-process implementor lives in `mushd`, which
//! depends on both halves.
//!
//! # Routing by target (doc 33 §4.3)
//!
//! A frame addressed to one connection — a `Deliver` to [`SendTarget::Conn`],
//! `SetOutputBrackets` — goes to the link that owns the connection and nowhere
//! else; one addressed to a character, a room, the world or a relay —
//! `Deliver` to `Char`/`All`/`Relay`, `SetFloodExempt`, `CloseSession`,
//! `CloseWebSessions`, `DiscordControl` — goes to every link, and a gateway
//! that holds no connection of the character ignores it, as it already does
//! for a character it never saw. The owner of a connection is the link that
//! announced it (`ConnUp`, `Attach`, `ReAnnounce`); a connection no live link
//! owns has nobody to deliver to, so its frame is dropped and said so at debug.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use mush_config::metrics::Metrics;
use mush_ipc::{DetachCause, DiscordControlOp, SendTarget, ToGateway};
use mush_proto::OutputEvent;
use tokio::sync::Notify;
use tokio::sync::mpsc::Sender;
use tokio::sync::mpsc::error::TrySendError;

use crate::links::LinkRegistry;

/// The world loop's output + session-control sink (see the module docs).
///
/// Methods use only the `mush-ipc` / core vocabulary (`u64` char/conn ids,
/// [`SendTarget`], [`OutputEvent`]) so the world task carries no session-layer
/// types. The session-walk methods ([`close_char_connections`],
/// [`structured_conns_of`], [`detach_conn_local`]) and [`session_counts`] are
/// only meaningful for the in-process sink; the socket sink returns empty /
/// zero (the forked server holds no `SessionManager`, and the world loop only
/// calls them on the `!is_socket()` branch).
///
/// [`close_char_connections`]: GatewaySink::close_char_connections
/// [`structured_conns_of`]: GatewaySink::structured_conns_of
/// [`detach_conn_local`]: GatewaySink::detach_conn_local
/// [`session_counts`]: GatewaySink::session_counts
pub trait GatewaySink: Send + Sync {
    /// Route a resolved delivery batch (each pair is one
    /// [`ToGateway::Deliver`]). The in-process sink self-polices overflowed
    /// connections; the socket sink serializes each pair to the boundary.
    fn transmit(&self, sends: Vec<(SendTarget, Arc<OutputEvent>)>);

    /// One out-of-band delivery through the seam (unaddressed one-off sends).
    fn deliver(&self, target: SendTarget, event: Arc<OutputEvent>) {
        self.transmit(vec![(target, event)]);
    }

    /// Mirror an accepted connection setting before the reply completion barrier.
    fn set_output_brackets(&self, _conn: u64, _brackets: mush_proto::OutputBrackets) {}

    /// Set/clear a character's flood-exempt flag on the gateway throttle.
    fn set_flood_exempt(&self, char: u64, exempt: bool);

    /// Ask the gateway's Discord bridge to follow, unfollow or re-tune a
    /// character's Discord follow (programme D3). Fire-and-forget; a sink with
    /// no Discord bridge drops it, and the bridge tells the character what
    /// happened.
    fn discord_control(&self, _char: u64, _op: DiscordControlOp) {}

    /// Whether this sink writes to the boundary socket (forked server) rather
    /// than a local `mush_session::SessionManager`. The world
    /// loop's session-control paths branch on this.
    fn is_socket(&self) -> bool;

    /// Fire-and-forget `@boot` in split mode: the gateway resolves `char` to its
    /// connections, drops them, and reports each `ConnDown` back as inbound. A
    /// no-op on the in-process sink (which walks the manager directly via
    /// [`close_char_connections`](GatewaySink::close_char_connections)).
    fn close_session(&self, char: u64, cause: DetachCause);

    /// Fire-and-forget `@newpassword` logout-everywhere in split mode. A no-op
    /// on the in-process sink.
    fn close_web_sessions(&self, chars: Vec<u64>, notice: Option<OutputEvent>, cause: DetachCause);

    /// In-process `@boot`: close every connection of `char` and return their
    /// connection ids (so the world loop can step each `ConnDown`). Empty on the
    /// socket sink (never called there — the world loop takes the socket branch).
    fn close_char_connections(&self, char: u64) -> Vec<u64>;

    /// In-process `@newpassword`: the `(conn, char)` pairs of every **structured**
    /// (web) connection of `chars`. Empty on the socket sink.
    fn structured_conns_of(&self, chars: &[u64]) -> Vec<(u64, u64)>;

    /// In-process detach of one connection (the `@newpassword` web-boot walk). A
    /// no-op on the socket sink.
    fn detach_conn_local(&self, conn: u64);

    /// `(sessions, connections)` counts for the world gauge. `(0, 0)` on the
    /// socket sink (the gateway publishes these post-fork).
    fn session_counts(&self) -> (i64, i64);
}

/// One gateway link's outbox: every server → gateway frame for THAT gateway is
/// queued here and drained to its socket by the link's writer task. Before
/// GG.2 this was the whole `SocketSink` (there was one link); now the
/// [`SocketSink`] the world holds routes each message to one or every link's
/// `LinkSink`, and the link's reader uses its own for correlated replies and
/// request dispositions, so a reply goes back on the link that asked.
///
/// # Bounded delivery (M18)
///
/// `tx` is a **bounded** channel ([`Config::ipc_to_gw_cap`]). Every `ToGateway`
/// frame is must-deliver (a `Deliver` keyframe, a correlated `Reply`, sticky
/// `SetFloodExempt` state, or a security `CloseSession`/`CloseWebSessions`), so
/// there is NEVER a per-message drop: on a `Full` channel the whole link is
/// torn down instead (the gateway's re-attach re-renders every session,
/// healing the loss). The world loop is the producer and must never `.await`
/// the boundary (D4), so `send` uses `try_send` and the
/// teardown decision + the socket drop live entirely in the writer task, which
/// waits on `teardown`.
///
/// [`Config::ipc_to_gw_cap`]: mush_config::config::Config::ipc_to_gw_cap
#[derive(Clone)]
pub struct LinkSink {
    tx: Sender<ToGateway>,
    /// Shared metrics registry — the same one the world loop reads for `@stats`.
    /// Records the `to_gw` queue high-water on every send.
    metrics: Arc<Metrics>,
    /// One-shot teardown trigger the writer task waits on. Notified the first
    /// time (and every subsequent time) `try_send` sees a `Full` channel; the
    /// writer returns, drops its socket half, and the link EOFs — the gateway's
    /// gap → reconnect → ReAnnounce → re-attach path heals it.
    teardown: Arc<Notify>,
    /// Count of frames that hit a full `to_gw` and forced a teardown. Drives the
    /// warn-once-per-crossing latch and gives tests a deterministic handle.
    overflow: Arc<AtomicU64>,
    /// Edge latch for the high-water warn: set when depth first crosses the
    /// threshold, cleared when it drops back below (so the warn is per-crossing,
    /// never per-push).
    hw_warned: Arc<AtomicBool>,
    /// The channel capacity (`to_gw_cap`), for the depth gauge + high-water.
    cap: usize,
    /// High-water warn threshold, percent of `cap` (edge-gated).
    high_water_pct: u8,
}

impl LinkSink {
    /// Wrap one link's writer inbox. `tx` is the bounded `to_gw` sender;
    /// `teardown` is the SAME [`Notify`] the link's writer task waits on;
    /// `metrics` is the process's registry; `cap`/`high_water_pct` come from
    /// `[ipc]`.
    #[must_use]
    pub fn new(
        tx: Sender<ToGateway>,
        metrics: Arc<Metrics>,
        teardown: Arc<Notify>,
        cap: usize,
        high_water_pct: u8,
    ) -> LinkSink {
        LinkSink {
            tx,
            metrics,
            teardown,
            overflow: Arc::new(AtomicU64::new(0)),
            hw_warned: Arc::new(AtomicBool::new(false)),
            cap,
            high_water_pct,
        }
    }

    /// Enqueue one must-deliver `ToGateway` frame with `try_send` (never
    /// blocks, never awaits — D4). On success it observes the queue depth for
    /// the high-water gauge + edge-gated warn. On `Full` it never drops-and-
    /// continues: it counts the overflow, records the high-water at `cap`, logs
    /// a warn once per crossing, and notifies the writer to TEAR THE LINK
    /// DOWN (the re-attach heals the lost frame). On `Closed` the link is
    /// already gone (the writer returned and dropped the receiver), so it is
    /// silently ignored — matching the pre-M18 `let _ = send(..)`.
    pub fn send(&self, msg: ToGateway) {
        match self.tx.try_send(msg) {
            Ok(()) => {
                let depth = self.cap.saturating_sub(self.tx.capacity()) as u64;
                self.metrics.observe_ipc_to_gw_depth(depth);
                let threshold = self.cap.saturating_mul(self.high_water_pct as usize) / 100;
                if threshold > 0 && depth >= threshold as u64 {
                    if !self.hw_warned.swap(true, Ordering::Relaxed) {
                        tracing::warn!(
                            dir = "to_gw",
                            queue_len = depth,
                            cap = self.cap,
                            "ipc queue high-water crossed"
                        );
                    }
                } else {
                    self.hw_warned.store(false, Ordering::Relaxed);
                }
            }
            Err(TrySendError::Full(_)) => {
                let prev = self.overflow.fetch_add(1, Ordering::Relaxed);
                self.metrics.ipc_to_gw_overflow();
                self.metrics.observe_ipc_to_gw_depth(self.cap as u64);
                if prev == 0 {
                    tracing::warn!(
                        dir = "to_gw",
                        cap = self.cap,
                        "ipc queue overflow; tearing down the link"
                    );
                }
                self.teardown.notify_one();
            }
            Err(TrySendError::Closed(_)) => {
                // The link is already torn down; the gateway's re-attach
                // re-renders every session, so a frame lost here is recovered.
            }
        }
    }

    /// Total frames that hit a full `to_gw` and forced a teardown, since boot.
    /// Test-only observation handle (the surfaced production signals are the
    /// warn log + the `to_gw` high-water gauge reaching `cap`).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn overflow_count(&self) -> u64 {
        self.overflow.load(Ordering::Relaxed)
    }
}

/// The forked-server sink the world loop holds: route each server → gateway
/// message to the link(s) that should see it (module docs) and serialize it as
/// a [`ToGateway`] frame on each one's [`LinkSink`]. Holds no `SessionManager`
/// (the gateway processes own theirs), so it never polices and the
/// session-walk / count methods are empty.
///
/// Every send is `try_send` through the link's bounded outbox (D4: the world
/// loop never awaits the boundary); a full outbox tears THAT link down, never
/// the others and never the world.
#[derive(Clone)]
pub struct SocketSink {
    links: Arc<LinkRegistry>,
}

impl SocketSink {
    /// A sink over the server's link registry.
    #[must_use]
    pub fn new(links: Arc<LinkRegistry>) -> SocketSink {
        SocketSink { links }
    }

    /// The registry this sink routes over.
    #[must_use]
    pub fn links(&self) -> &Arc<LinkRegistry> {
        &self.links
    }

    /// A connection-addressed frame: the owning link only. A connection no
    /// live link owns has nobody to deliver to; the frame is dropped and the
    /// drop said at debug (a connection that went down a moment ago is the
    /// ordinary case).
    fn to_conn(&self, conn: u64, msg: ToGateway) {
        if !self.links.send_to_conn(conn, msg) {
            tracing::debug!(conn, "frame for a connection no link owns dropped");
        }
    }

    /// A character-, room-, world- or relay-addressed frame: every link.
    fn to_every_link(&self, msg: &ToGateway) {
        self.links.broadcast(msg);
    }
}

impl GatewaySink for SocketSink {
    fn set_output_brackets(&self, conn: u64, brackets: mush_proto::OutputBrackets) {
        self.to_conn(conn, ToGateway::SetOutputBrackets { conn, brackets });
    }

    fn transmit(&self, sends: Vec<(SendTarget, Arc<OutputEvent>)>) {
        for (target, ev) in sends {
            let msg = ToGateway::Deliver {
                target: target.clone(),
                event: (*ev).clone(),
            };
            match target {
                SendTarget::Conn(conn) => self.to_conn(conn, msg),
                SendTarget::Char(_) | SendTarget::All | SendTarget::Relay(_) => {
                    self.to_every_link(&msg);
                }
            }
        }
    }

    fn set_flood_exempt(&self, char: u64, exempt: bool) {
        self.to_every_link(&ToGateway::SetFloodExempt { char, exempt });
    }

    fn discord_control(&self, char: u64, op: DiscordControlOp) {
        self.to_every_link(&ToGateway::DiscordControl { char, op });
    }

    fn is_socket(&self) -> bool {
        true
    }

    fn close_session(&self, char: u64, cause: DetachCause) {
        self.to_every_link(&ToGateway::CloseSession { char, cause });
    }

    fn close_web_sessions(&self, chars: Vec<u64>, notice: Option<OutputEvent>, cause: DetachCause) {
        self.to_every_link(&ToGateway::CloseWebSessions {
            chars,
            notice,
            cause,
        });
    }

    fn close_char_connections(&self, _char: u64) -> Vec<u64> {
        Vec::new()
    }

    fn structured_conns_of(&self, _chars: &[u64]) -> Vec<(u64, u64)> {
        Vec::new()
    }

    fn detach_conn_local(&self, _conn: u64) {}

    fn session_counts(&self) -> (i64, i64) {
        (0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::ObjId;
    use mush_ipc::{LinkVersion, ServiceResp};
    use mush_proto::RichText;
    use tokio::sync::mpsc;

    /// A link sink over a fresh bounded channel whose receiver is NEVER polled —
    /// so the channel fills at exactly `cap` and the next send overflows.
    /// Returns the held receiver (keep it alive to stay `Full`, not `Closed`),
    /// the shared teardown the writer would wait on, and the metrics registry.
    fn sink(
        cap: usize,
    ) -> (
        LinkSink,
        mpsc::Receiver<ToGateway>,
        Arc<Notify>,
        Arc<Metrics>,
    ) {
        let (tx, rx) = mpsc::channel::<ToGateway>(cap);
        let metrics = Arc::new(Metrics::new());
        let teardown = Arc::new(Notify::new());
        let s = LinkSink::new(tx, Arc::clone(&metrics), Arc::clone(&teardown), cap, 80);
        (s, rx, teardown, metrics)
    }

    fn exempt() -> ToGateway {
        ToGateway::SetFloodExempt {
            char: 1,
            exempt: true,
        }
    }

    /// A full `to_gw` NEVER silently drops (I1): the overflowing frame counts,
    /// records the high-water at cap, and signals the writer to tear the
    /// link down — the writer's `teardown.notified()` becomes ready.
    #[tokio::test]
    async fn full_to_gw_tears_down_never_drops_silently() {
        let cap = 4;
        let (s, _rx, teardown, metrics) = sink(cap);
        // Fill exactly to capacity — at cap is not yet an overflow.
        for _ in 0..cap {
            s.send(exempt());
        }
        assert_eq!(s.overflow_count(), 0, "at capacity is not an overflow");
        // The next must-deliver frame triggers a teardown, not a per-message drop.
        s.send(exempt());
        assert_eq!(s.overflow_count(), 1);
        // notify_one stored a permit, so the writer's wait resolves immediately.
        tokio::time::timeout(std::time::Duration::from_secs(1), teardown.notified())
            .await
            .expect("teardown must be signalled on to_gw overflow");
        // The high-water reached cap and surfaces via @stats / /metrics.
        assert_eq!(metrics.snapshot().ipc_queue_high_water_to_gw, cap as u64);
        // The overflow is counted in the shared registry (surfaced via @stats /
        // /metrics), not only in the LinkSink's local latch counter.
        assert_eq!(metrics.snapshot().ipc_to_gw_overflow_total, 1);
    }

    /// A correlated `Reply` is must-deliver too: on a full channel it must tear
    /// down, never be dropped (a dropped `Reply` would hang its `ServiceReq`
    /// oneshot — I1).
    #[tokio::test]
    async fn reply_frame_tears_down_on_full() {
        let cap = 1;
        let (s, _rx, teardown, _m) = sink(cap);
        s.send(exempt()); // fills the single slot
        s.send(ToGateway::Reply {
            id: 7,
            resp: ServiceResp::NameFree(true),
        });
        assert_eq!(s.overflow_count(), 1);
        tokio::time::timeout(std::time::Duration::from_secs(1), teardown.notified())
            .await
            .expect("a full channel must tear down rather than drop a Reply");
    }

    /// After the link is torn down the receiver is dropped, so `try_send`
    /// returns `Closed`: that is silently ignored (the re-attach heals the loss)
    /// and is NOT counted as an overflow nor a fresh teardown.
    #[tokio::test]
    async fn closed_channel_is_ignored_not_a_teardown() {
        let (s, rx, teardown, _m) = sink(4);
        drop(rx); // receiver gone → every send is Closed
        for _ in 0..10 {
            s.send(exempt());
        }
        assert_eq!(s.overflow_count(), 0, "Closed is not an overflow");
        // The teardown is not signalled by a Closed send.
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), teardown.notified())
                .await
                .is_err(),
            "Closed must not trigger a teardown notify"
        );
    }

    /// The high-water warn is edge-gated: the depth gauge tracks true occupancy
    /// (it rises as the un-drained channel fills). Below cap there is no
    /// overflow and no teardown.
    #[tokio::test]
    async fn high_water_tracks_true_occupancy_below_cap() {
        let cap = 8;
        let (s, _rx, _teardown, metrics) = sink(cap);
        for _ in 0..(cap - 1) {
            s.send(exempt());
        }
        assert_eq!(s.overflow_count(), 0);
        // Depth reached cap-1 (nothing drained), recorded as the high-water.
        assert_eq!(
            metrics.snapshot().ipc_queue_high_water_to_gw,
            (cap - 1) as u64
        );
    }

    // ---- routing by target (doc 33 §4.3) ----------------------------------

    /// Two links with one connection each, and the world's sink over them.
    fn two_links() -> (
        SocketSink,
        mpsc::Receiver<ToGateway>,
        mpsc::Receiver<ToGateway>,
    ) {
        let registry = Arc::new(LinkRegistry::new(Arc::new(Metrics::new())));
        let open = |conn: u64, actor: u32| {
            let (tx, rx) = mpsc::channel::<ToGateway>(16);
            let link = registry.open_link(
                LinkSink::new(
                    tx,
                    Arc::new(Metrics::new()),
                    Arc::new(Notify::new()),
                    16,
                    80,
                ),
                LinkVersion::unknown(),
            );
            assert_eq!(link.claim(conn, ObjId(actor)), crate::links::Claim::Owned);
            rx
        };
        let ra = open(1, 10);
        let rb = open(2, 20);
        (SocketSink::new(registry), ra, rb)
    }

    fn line(text: &str) -> Arc<OutputEvent> {
        Arc::new(OutputEvent::Sys(RichText::plain(text)))
    }

    fn drain(rx: &mut mpsc::Receiver<ToGateway>) -> Vec<ToGateway> {
        let mut out = Vec::new();
        while let Ok(m) = rx.try_recv() {
            out.push(m);
        }
        out
    }

    /// A `Deliver` to a connection reaches the link that owns it and no other;
    /// one to a character, to all, or to a relay reaches every link.
    #[tokio::test]
    async fn deliveries_route_by_target() {
        let (sink, mut ra, mut rb) = two_links();
        sink.transmit(vec![
            (SendTarget::Conn(1), line("to a's connection")),
            (SendTarget::Conn(2), line("to b's connection")),
            (SendTarget::Char(10), line("to the character")),
            (SendTarget::All, line("to the world")),
        ]);
        let a = drain(&mut ra);
        let b = drain(&mut rb);
        assert_eq!(
            a.len(),
            3,
            "a: its connection, the character, the world: {a:?}"
        );
        assert_eq!(
            b.len(),
            3,
            "b: its connection, the character, the world: {b:?}"
        );
        assert!(format!("{a:?}").contains("to a's connection"));
        assert!(!format!("{a:?}").contains("to b's connection"));
        assert!(format!("{b:?}").contains("to b's connection"));
        assert!(!format!("{b:?}").contains("to a's connection"));
    }

    /// Connection settings follow the connection; character-addressed control
    /// goes everywhere. A connection nobody owns is dropped, not misrouted.
    #[tokio::test]
    async fn control_frames_route_by_target() {
        let (sink, mut ra, mut rb) = two_links();
        sink.set_output_brackets(2, mush_proto::OutputBrackets::default());
        assert!(drain(&mut ra).is_empty(), "a owns no connection 2");
        assert_eq!(drain(&mut rb).len(), 1);
        sink.set_flood_exempt(10, true);
        sink.close_session(10, DetachCause::Booted);
        sink.close_web_sessions(vec![10, 20], None, DetachCause::Booted);
        sink.discord_control(10, DiscordControlOp::Unfollow);
        assert_eq!(drain(&mut ra).len(), 4);
        assert_eq!(drain(&mut rb).len(), 4);
        sink.set_output_brackets(99, mush_proto::OutputBrackets::default());
        sink.deliver(SendTarget::Conn(99), line("nobody"));
        assert!(drain(&mut ra).is_empty());
        assert!(drain(&mut rb).is_empty());
    }
}

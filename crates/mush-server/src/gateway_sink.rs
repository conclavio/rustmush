//! The server-side view of the server → gateway delivery seam.
//!
//! The world loop produces `(SendTarget, OutputEvent)` pairs (audience already
//! resolved server-side) plus fire-and-forget session control, and hands them to
//! a [`GatewaySink`]. Two implementors exist:
//!
//! - [`SocketSink`] (here, forked server): serializes every message as a
//!   [`ToGateway`] frame onto the boundary writer. It owns no `SessionManager` —
//!   the gateway process does — so it never polices and the session-walk /
//!   count methods are empty.
//! - the in-process sink (in `mushd`, the all-in-one): routes each pair onto the
//!   shared [`SessionManager`](mush_session::SessionManager) with `Arc` fan-out
//!   (zero-copy), self-polices slow clients back into the world loop, and
//!   answers the session walks / counts against the real manager.
//!
//! The trait is defined here (server-side) so the world loop names no
//! `mush-session` type; its in-process implementor lives in `mushd`, which
//! depends on both halves.

use std::sync::Arc;

use mush_ipc::{DetachCause, SendTarget, ToGateway};
use mush_proto::OutputEvent;
use tokio::sync::mpsc::UnboundedSender;

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

    /// Set/clear a character's flood-exempt flag on the gateway throttle.
    fn set_flood_exempt(&self, char: u64, exempt: bool);

    /// Whether this sink writes to the boundary socket (forked server) rather
    /// than a local [`SessionManager`](mush_session::SessionManager). The world
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

/// The forked-server sink: serialize every server → gateway message as a
/// [`ToGateway`] frame onto `tx`, drained to the boundary socket by the writer
/// task. Holds no `SessionManager` (the gateway process owns it), so it never
/// polices and the session-walk / count methods are empty.
#[derive(Clone)]
pub struct SocketSink {
    tx: UnboundedSender<ToGateway>,
}

impl SocketSink {
    /// Wrap the boundary writer's inbox.
    #[must_use]
    pub fn new(tx: UnboundedSender<ToGateway>) -> SocketSink {
        SocketSink { tx }
    }
}

impl GatewaySink for SocketSink {
    fn transmit(&self, sends: Vec<(SendTarget, Arc<OutputEvent>)>) {
        for (target, ev) in sends {
            let _ = self.tx.send(ToGateway::Deliver {
                target,
                event: (*ev).clone(),
            });
        }
    }

    fn set_flood_exempt(&self, char: u64, exempt: bool) {
        let _ = self.tx.send(ToGateway::SetFloodExempt { char, exempt });
    }

    fn is_socket(&self) -> bool {
        true
    }

    fn close_session(&self, char: u64, cause: DetachCause) {
        let _ = self.tx.send(ToGateway::CloseSession { char, cause });
    }

    fn close_web_sessions(&self, chars: Vec<u64>, notice: Option<OutputEvent>, cause: DetachCause) {
        let _ = self.tx.send(ToGateway::CloseWebSessions {
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

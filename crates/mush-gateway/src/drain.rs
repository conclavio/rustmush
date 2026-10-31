//! The gateway's side of the two-generation drain (doc 33 §4.5–§4.7, cut
//! GG.4).
//!
//! The mechanism — the registry, the schedule, the warnings, the deadline,
//! the finish, the signals and the marker — is [`mush_drain`]'s, shared with
//! `sshdoorway`. This module says what a gateway session is to it and what
//! the gateway does when a drain begins:
//!
//! - A **session** is one attached connection ([`GatewaySession`]): SSH on
//!   the public port, SSH through the doorway's trusted socket, or a web
//!   socket. A warning is an [`OutputEvent::Sys`] line sent through the
//!   session manager like every other line, so it lands in the line
//!   editor's frame, in the TUI, in the compositor's frame, and seq-stamped
//!   in the web client's ring. The close is the transport's own: SSH is sent
//!   [`OutputEvent::Disconnect`], which its pump ends on; web is asked for a
//!   1012 close frame ([`mush_net_web::ConnCloser::service_restart`]), which
//!   the client treats as transient and reconnects from with its resume
//!   sequence (Q-33-4). Either way the transport reports the disconnect, the
//!   session goes link-dead in the world, and the player's reconnect lands on
//!   the new generation with their character where they left it.
//! - The **host** ([`GatewayHost`]) gives the lifecycle the drain's settings
//!   (the doorway's defaults, Q-33-2, unless a test shortens them) and, when
//!   a drain or a finish begins, stops the listeners (the kernel migrates the
//!   queued connections to the other generation under `tcp_migrate_req = 1`)
//!   and the Discord bot (one generation holds the lock, §4.5).
//!
//! Only an **attached** connection counts as live (§4.6: "the connection is
//! the unit"). A connection still at the login prompt is not registered: it
//! has no character to keep, the warnings would be noise before a password,
//! and the process exit at the deadline ends it like a dropped line. The
//! gateway's `Drain` is created in `serve` and handed to every bridge, so the
//! three transports count into one registry.

use std::sync::Arc;

use futures::future::BoxFuture;
use mush_drain::lifecycle::{DrainConfig, Host};
use mush_net_web::ConnCloser;
use mush_proto::{OutputEvent, RichText};
use mush_session::{ConnId, SessionManager};

use crate::delivery::send_event_to_conn;

/// What a session is told as it is closed at the deadline (or at a finish).
pub const CLOSE_REASON: &str =
    "This gateway is restarting; reconnect any time, your character is where you left it.";

/// This generation's lifecycle and session registry.
pub type GatewayDrain = mush_drain::Drain<GatewaySession>;

/// Held for every attached connection; the generation may exit when none
/// remain.
pub type LiveGuard = mush_drain::LiveGuard<GatewaySession>;

/// How a connection is closed at the deadline: by its transport's own means.
#[derive(Clone, Debug)]
pub enum CloseWay {
    /// SSH, public or trusted: an [`OutputEvent::Disconnect`], which the
    /// connection's output pump ends on (then "session ended" and the cut).
    Disconnect,
    /// Web: a 1012 close frame with [`CLOSE_REASON`].
    Web(ConnCloser),
}

/// One attached connection as the drain reaches it.
pub struct GatewaySession {
    conn: ConnId,
    sessions: Arc<SessionManager>,
    close: CloseWay,
}

impl std::fmt::Debug for GatewaySession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewaySession")
            .field("conn", &self.conn.0)
            .field("close", &self.close)
            .finish_non_exhaustive()
    }
}

impl GatewaySession {
    /// The drain's view of `conn`: its session manager (the delivery path)
    /// and how to close it.
    #[must_use]
    pub fn new(conn: ConnId, sessions: Arc<SessionManager>, close: CloseWay) -> GatewaySession {
        GatewaySession {
            conn,
            sessions,
            close,
        }
    }
}

impl mush_drain::Session for GatewaySession {
    /// A `Sys` line through the normal delivery path: in-frame on every
    /// transport, seq-stamped for the web client's resume.
    fn inject<'a>(&'a self, line: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            send_event_to_conn(
                &self.sessions,
                self.conn,
                OutputEvent::Sys(RichText::tag("system", line)),
            );
        })
    }

    fn close(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            match &self.close {
                CloseWay::Disconnect => send_event_to_conn(
                    &self.sessions,
                    self.conn,
                    OutputEvent::Disconnect {
                        reason: RichText::tag("system", CLOSE_REASON),
                    },
                ),
                CloseWay::Web(closer) => closer.service_restart(CLOSE_REASON),
            }
        })
    }
}

/// What the gateway does when its generation stops accepting: close the
/// listeners and stop the Discord bot. Built by `serve`, called once.
pub type StopAccepting = Box<dyn FnOnce() + Send>;

/// The gateway as the lifecycle's host.
pub struct GatewayHost {
    cfg: DrainConfig,
    stop: Option<StopAccepting>,
}

impl std::fmt::Debug for GatewayHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayHost")
            .field("deadline_secs", &self.cfg.deadline.as_secs())
            .field("stop_pending", &self.stop.is_some())
            .finish()
    }
}

impl GatewayHost {
    /// A host with the drain's settings and what to do when accepting stops.
    #[must_use]
    pub fn new(cfg: DrainConfig, stop: StopAccepting) -> GatewayHost {
        GatewayHost {
            cfg,
            stop: Some(stop),
        }
    }

    /// Run the stop once, now (the shutdown tail, after the lifecycle ended
    /// without a drain or finish — the keeper ending on its own).
    pub fn stop_now(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
    }
}

impl Host for GatewayHost {
    fn drain_config(&self) -> DrainConfig {
        self.cfg.clone()
    }

    /// `SIGHUP` is reserved (doc 33 §4.5): logged, nothing else. A deploy of
    /// the server half is `systemctl restart <inst>mush-server`.
    fn reload(&mut self) {
        tracing::info!(
            "SIGHUP received: reserved (doc 33 §4.5), ignored; restart the server with \
             `systemctl restart <inst>mush-server`"
        );
    }

    fn stopped_accepting(&mut self) {
        self.stop_now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use mush_drain::Session;
    use mush_proto::{ClientCaps, ClientKind, ColorDepth};
    use mush_session::{CharKey, ConnectionHandle};
    use tokio::sync::mpsc;

    fn attached() -> (
        Arc<SessionManager>,
        ConnId,
        mpsc::Receiver<Arc<OutputEvent>>,
    ) {
        let sessions = Arc::new(SessionManager::new());
        let (tx, rx) = mpsc::channel(8);
        let conn = ConnId(41);
        let handle = ConnectionHandle::new(
            conn,
            "127.0.0.1".parse().expect("an address"),
            ClientCaps {
                kind: ClientKind::Ssh,
                color: ColorDepth::None,
                width: 120,
                height: 40,
                utf8: true,
                structured: false,
            },
            tx,
        );
        sessions.attach(handle, CharKey(7), "acct".to_string());
        (sessions, conn, rx)
    }

    /// A warning is a `Sys` line on the connection; the SSH close is the
    /// `Disconnect` the pump ends on, and the web close is the 1012 ask.
    #[tokio::test]
    async fn warnings_are_sys_lines_and_the_close_is_the_transports_own() {
        let (sessions, conn, mut rx) = attached();
        let ssh = GatewaySession::new(conn, Arc::clone(&sessions), CloseWay::Disconnect);
        ssh.inject("the gateway is restarting in 1m").await;
        let ev = rx.recv().await.expect("a line");
        assert!(
            matches!(&*ev, OutputEvent::Sys(t) if t.to_plain_text().contains("restarting in 1m")),
            "{ev:?}"
        );
        ssh.close().await;
        let ev = rx.recv().await.expect("the close");
        assert!(
            matches!(&*ev, OutputEvent::Disconnect { reason } if reason.to_plain_text() == CLOSE_REASON),
            "{ev:?}"
        );
        let (closer, watch) = ConnCloser::new();
        let web = GatewaySession::new(conn, sessions, CloseWay::Web(closer));
        web.close().await;
        let asked = watch.borrow().clone().expect("the web close");
        assert_eq!(asked.code, 1012);
        assert_eq!(asked.reason, CLOSE_REASON);
    }

    /// The host's stop runs once, whether the lifecycle or the tail asks.
    #[test]
    fn the_host_stops_accepting_once() {
        let count = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&count);
        let mut host = GatewayHost::new(
            DrainConfig {
                deadline: Duration::from_secs(5),
                ..DrainConfig::default()
            },
            Box::new(move || {
                c.fetch_add(1, Ordering::SeqCst);
            }),
        );
        assert_eq!(host.drain_config().deadline, Duration::from_secs(5));
        host.reload();
        host.stopped_accepting();
        host.stop_now();
        host.stopped_accepting();
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}

//! The all-in-one in-process server → gateway delivery sink.
//!
//! `mushd` runs the gateway and server halves in ONE process over a shared
//! [`SessionManager`], so the world loop's [`GatewaySink`] here routes each
//! resolved `(SendTarget, OutputEvent)` pair straight onto that manager with
//! `Arc` fan-out (zero-copy — invariant S1) instead of serializing it to a
//! socket, self-polices slow clients back into the world loop's own inbound
//! queue (D5), and answers `@boot`/`@newpassword`'s session walks + the session
//! gauges against the real manager.
//!
//! The slow-client policing and link-dead reaping is the SHARED core in
//! `mush-gateway`
//! ([`transmit_and_police`](mush_gateway::delivery::transmit_and_police) and
//! [`spawn_linkdead_reaper`](mush_gateway::delivery::spawn_linkdead_reaper)).
//! This sink supplies only where a policed `ConnDown`/`SessionGone` is posted.
//! It targets this loop's own inbound queue, reached through a weak sender: the
//! world loop owns this sink for its whole lifetime, so a strong sender would
//! keep that inbound queue alive and shutdown could never be observed.

use std::ops::ControlFlow;
use std::sync::Arc;

use mush_core::Inbound;
use mush_ipc::{DetachCause, DiscordControlOp, SendTarget};
use mush_proto::OutputEvent;
use mush_server::gateway_sink::GatewaySink;
use mush_server::world_task::WorldMsg;
use mush_session::{CharKey, ConnId, DetachReason, SessionManager};
use tokio::sync::mpsc::WeakSender;

use mush_gateway::controls::GatewayControls;
use mush_gateway::delivery::{ConnDownPost, spawn_linkdead_reaper, transmit_and_police};

/// The all-in-one in-process [`GatewaySink`]: the shared session manager plus a
/// WEAK sender into the world loop's own inbound queue for policing feedback.
#[derive(Clone)]
pub struct InProcessSink {
    sessions: Arc<SessionManager>,
    /// Policed connections' `ConnDown`/`SessionGone` go here. A [`WeakSender`]:
    /// the world loop owns this sink, so a strong handle would keep its own
    /// inbound channel alive and it could never shut down.
    feedback: WeakSender<WorldMsg>,
    /// The gateway-side control hooks (the Discord bridge, programme D3).
    controls: Option<Arc<GatewayControls>>,
}

impl InProcessSink {
    /// Wrap the shared session manager and the world loop's inbound weak sender.
    #[must_use]
    pub fn new(sessions: Arc<SessionManager>, feedback: WeakSender<WorldMsg>) -> InProcessSink {
        InProcessSink {
            sessions,
            feedback,
            controls: None,
        }
    }

    /// Route the server's gateway-side controls (`discord_control`) to
    /// `controls`, where the Discord bridge registers at start-up.
    #[must_use]
    pub fn with_controls(mut self, controls: Arc<GatewayControls>) -> InProcessSink {
        self.controls = Some(controls);
        self
    }

    /// Post a batch of policed connections' `ConnDown`s back to the loop **in
    /// order** (the deterministic sorted order [`transmit_and_police`] produced)
    /// through a SINGLE task, preserving the pre-fork synchronous ordering.
    /// Upgrade only transiently — a live sender must never outlive the loop.
    fn post_conn_downs(&self, posts: Vec<ConnDownPost>) {
        if posts.is_empty() {
            return;
        }
        let Some(tx) = self.feedback.upgrade() else {
            return;
        };
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
}

impl GatewaySink for InProcessSink {
    fn transmit(&self, sends: Vec<(SendTarget, Arc<OutputEvent>)>) {
        // The world loop owns this sink, so the reaper (and its `SessionGone`
        // feedback) must go through a WEAK sender it upgrades only transiently —
        // a live sender would keep the loop's own inbound queue alive forever.
        let reaper_sessions = Arc::clone(&self.sessions);
        let reaper_feedback = self.feedback.clone();
        let posts = transmit_and_police(&self.sessions, sends, move |grace| {
            let feedback = reaper_feedback.clone();
            spawn_linkdead_reaper(Arc::clone(&reaper_sessions), grace, move |actor| {
                let feedback = feedback.clone();
                async move {
                    let Some(tx) = feedback.upgrade() else {
                        return ControlFlow::Break(());
                    };
                    let _ = tx
                        .send(WorldMsg::Inbound(Inbound::SessionGone { actor }))
                        .await;
                    ControlFlow::Continue(())
                }
            });
        });
        self.post_conn_downs(posts);
    }

    fn set_flood_exempt(&self, char: u64, exempt: bool) {
        self.sessions.set_flood_exempt(CharKey(char), exempt);
    }

    fn discord_control(&self, char: u64, op: DiscordControlOp) {
        match &self.controls {
            Some(c) => c.discord_control(char, op),
            None => tracing::debug!(char, ?op, "discord control with no registry; dropped"),
        }
    }

    fn is_socket(&self) -> bool {
        false
    }

    fn close_session(&self, _char: u64, _cause: DetachCause) {}

    fn close_web_sessions(
        &self,
        _chars: Vec<u64>,
        _notice: Option<OutputEvent>,
        _cause: DetachCause,
    ) {
    }

    fn close_char_connections(&self, char: u64) -> Vec<u64> {
        let mut conns = Vec::new();
        if let Some(sess) = self.sessions.session_of_char(CharKey(char)) {
            for handle in self.sessions.close_session(sess) {
                conns.push(handle.id.0);
            }
        }
        conns
    }

    fn structured_conns_of(&self, chars: &[u64]) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        for &char in chars {
            let Some(sess) = self.sessions.session_of_char(CharKey(char)) else {
                continue;
            };
            for handle in self.sessions.conns_of_session(sess) {
                if handle.is_structured() {
                    out.push((handle.id.0, char));
                }
            }
        }
        out
    }

    fn detach_conn_local(&self, conn: u64) {
        self.sessions.detach(ConnId(conn), DetachReason::Booted);
    }

    fn session_counts(&self) -> (i64, i64) {
        (
            self.sessions.session_count() as i64,
            self.sessions.connection_count() as i64,
        )
    }
}

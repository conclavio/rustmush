//! The transport-agnostic half of the game bridges: everything the SSH
//! bridge ([`crate::ssh_bridge::SshBridge`]) and the web bridge
//! ([`crate::web_bridge::WebBridge`]) share — account auth/creation over
//! the [`AuthBackend`], the session attach/enter-game flow, input handling
//! (sanitation, `QUIT`, flood), the disconnect/linkdead flow, and the
//! [`IpGuard`] pass-throughs.
//!
//! ## Division of labor (identical for both transports)
//!
//! - The **transport** owns its socket, its wire (ANSI render vs. JSON
//!   envelopes — the [`Sink`](mush_session::Sink) it builds the
//!   [`ConnectionHandle`] with), and closes when the session detaches
//!   (output sender dropped) or a `Disconnect` event arrives.
//! - The **bridge core** owns everything game-side: argon2 checks via the
//!   [`AuthBackend`] (on the blocking pool), the [`SessionManager`]
//!   attach/detach lifecycle, flood control, input sanitation, `QUIT`,
//!   linkdead grace reaping, and the [`IpGuard`].
//! - Failure counting: the core **never** records auth failures itself;
//!   the transport calls [`GameBridge::auth_failed`] for every failed
//!   attempt, so each failure is counted exactly once. Likewise the
//!   transport gates on [`GameBridge::is_ip_blocked`] before every auth
//!   attempt.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mush_core::{DisconnectKind, ObjId, PerceivedRef, ProjectedLegalAction, ProjectedTargetSchema};
use mush_ipc::{
    ActionSend, ConnectionAdmission, GatewayAction, GatewayLink, InputSend, ServiceReq,
    ServiceResp, ToServer, WireInbound,
};
// Shared with the SSH transport's own line cap (`mush-net-ssh::editor`) so
// both agree on one bound (MEDIUM finding).
use mush_net_ssh::MAX_LINE_CHARS;
use mush_proto::{
    ActionStatusId, MAX_VIEW_WIRE_INTEGER, ObjRef, OutputEvent, PanelFrameKind,
    ProjectedActionReceipt, ProjectedActionStatus, ProjectedCommitToken, RespondChoice, RichText,
    ViewEnvelope,
};
use mush_session::{
    ConnId, ConnectionHandle, DetachOutcome, DetachReason, FloodVerdict, SendToConn, SessId,
    SessionManager, sanitize_line,
};
use smol_str::SmolStr;

use mush_config::ipguard::IpGuard;

use crate::accounts::{AuthBackend, AuthOutcome, CreateErr};
use crate::delivery::{char_key, char_key_obj, send_event_to_conn};
use crate::poison::locked;

/// QUIT/boot may wait briefly for an exact delivery acknowledgement, but a
/// wedged world must not hold a transport forever. Busy admission is retried a
/// small, fixed number of times inside one total deadline; every other failure
/// is an explicit degraded teardown.
const DELIVERY_BARRIER_ATTEMPTS: usize = 3;
const DELIVERY_BARRIER_TIMEOUT: Duration = Duration::from_secs(5);
const DELIVERY_BARRIER_RETRY_DELAY: Duration = Duration::from_millis(25);

/// Hard cap on one narrative-backlog reply (interface plan I12), mirroring the
/// comsys `MAX_BACKLOG_LIMIT`. The ring itself is only `resume_ring` deep, so a
/// single pull can never exceed it.
const MAX_NARRATIVE_BACKLOG: usize = 200;

fn render_legal_action_list(offers: &[ProjectedLegalAction]) -> String {
    let mut text = String::from("Available actions:");
    for offer in offers {
        let presentation = offer.presentation();
        text.push_str("\n  +act ");
        text.push_str(&presentation.verb);
        text.push_str(" — ");
        text.push_str(&presentation.label);
        if !presentation.help.is_empty() {
            text.push_str(": ");
            text.push_str(&presentation.help);
        }
    }
    text
}

#[derive(Debug, PartialEq)]
struct LineActionSelection {
    verb: String,
    subject: PerceivedRef,
    parameters: serde_json::Value,
}

fn strip_ascii_keyword<'a>(input: &'a str, keyword: &str) -> Option<&'a str> {
    let (head, tail) = input.split_once(char::is_whitespace)?;
    head.eq_ignore_ascii_case(keyword).then(|| tail.trim())
}

fn parse_line_subject(argument: &str) -> Result<PerceivedRef, &'static str> {
    let id = argument
        .trim()
        .strip_prefix('#')
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value != 0)
        .ok_or("Action subjects use #<object>.")?;
    Ok(PerceivedRef::KnownObject {
        id: ObjId(id),
        revision: 1,
    })
}

fn parse_line_action(argument: &str) -> Result<LineActionSelection, &'static str> {
    let argument = argument.trim();
    let (verb, mut rest) = argument
        .split_once(char::is_whitespace)
        .map_or((argument, ""), |(verb, rest)| (verb, rest.trim()));
    if verb.is_empty() {
        return Err("Usage: +act <verb> [on #<object>] [with <json object>]");
    }
    let subject = if let Some(after_on) = strip_ascii_keyword(rest, "on") {
        let (object, after_object) = after_on
            .split_once(char::is_whitespace)
            .map_or((after_on, ""), |(object, tail)| (object, tail.trim()));
        let subject =
            parse_line_subject(object).map_err(|_| "Action subjects use on #<object>.")?;
        rest = after_object;
        subject
    } else {
        PerceivedRef::SelfActor
    };
    if let Some(after_with) = strip_ascii_keyword(rest, "with") {
        rest = after_with.trim();
    }
    let parameters = if rest.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(rest).map_err(|_| "Action parameters must be one JSON object.")?
    };
    if !parameters.is_object() {
        return Err("Action parameters must be one JSON object.");
    }
    Ok(LineActionSelection {
        verb: verb.to_owned(),
        subject,
        parameters,
    })
}

/// One character offered on a character-select surface (welcome screen,
/// web character picker). The `id` is a widened `ObjId`, opaque to
/// transports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharSummary {
    /// Character key (a widened `ObjId`; opaque to the transport).
    pub id: u64,
    /// Character name.
    pub name: String,
}

/// Result of an account credential check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthResult {
    /// Credentials good.
    Ok {
        /// Account name in registered display case.
        account: String,
        /// The account's characters.
        characters: Vec<CharSummary>,
    },
    /// Unknown account or wrong password (deliberately indistinguishable).
    Denied,
}

/// Why [`GameBridge::enter_game`] refused.
///
/// (The per-IP concurrent-connection cap is enforced by the transport at
/// accept time via its `connection_opened` hook, not here.)
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConnectErr {
    /// The character does not belong to that account.
    #[error("that is not your character")]
    NotYourCharacter,
    /// The world task is gone (shutdown in progress).
    #[error("the game is shutting down")]
    WorldGone,
    /// The split boundary is live, but bounded lifecycle/IPC custody is full.
    #[error("the game is busy; please try again in a moment")]
    Busy,
}

/// What the transport should do after feeding a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineOutcome {
    /// Keep reading.
    Continue,
    /// Flush pending output and close the connection (player quit).
    Close,
}

/// What [`GameBridge::enter_game`] established.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnterOutcome {
    /// The session the connection joined (fresh or existing).
    pub sess: SessId,
    /// The highest envelope seq already stamped for the session **before**
    /// this connection's own entry output (0 for a fresh session) — the
    /// web `SessionStart::resume_from` baseline; meaningless-but-harmless
    /// for ANSI transports.
    pub resume_from: u64,
}

/// The shared game-side bridge core. Both transport bridges hold one and
/// delegate; it is generic over [`AuthBackend`] so the store-backed
/// implementation and test fakes slot in identically.
pub struct GameBridge<A: AuthBackend + 'static> {
    pub(crate) accounts: Arc<A>,
    pub(crate) sessions: Arc<SessionManager>,
    /// The protocol-typed handle to the server half (M16a). The bridges speak
    /// `mush-ipc` [`ToServer`]/[`ServiceReq`] through this trait object, never a
    /// concrete world handle; the in-process impl wraps the world-task mpsc, and
    /// M16b swaps in a socket client.
    pub(crate) link: Arc<dyn GatewayLink>,
    pub(crate) guard: Arc<IpGuard>,
    /// Public semantic-view generation for this gateway lifetime. A gateway
    /// restart changes it, so replayed deltas can never attach to pre-restart
    /// client state accidentally.
    view_generation: u64,
    /// Per-connection/per-view sequence. Entries are removed with the
    /// connection, keeping projection bookkeeping bounded by live transport
    /// state rather than historical connection IDs.
    view_sequences: Mutex<HashMap<(u64, &'static str), u64>>,
}

impl<A: AuthBackend + 'static> GameBridge<A> {
    /// Wire the core. Takes the in-process world handle (any [`GatewayLink`],
    /// e.g. `mush_server::world_task::WorldHandle`) and holds it behind the
    /// trait, so every game-side call crosses the boundary as a `mush-ipc`
    /// protocol message.
    pub fn new(
        accounts: Arc<A>,
        sessions: Arc<SessionManager>,
        world: impl GatewayLink + 'static,
        guard: Arc<IpGuard>,
    ) -> GameBridge<A> {
        Self::with_link(accounts, sessions, Arc::new(world), guard)
    }

    /// Wire the core with an explicit [`GatewayLink`] (M16b). The all-in-one
    /// [`new`](GameBridge::new) wraps an in-process world handle; the forked
    /// `mush-gateway` passes a socket-client link that serializes every
    /// `mush-ipc` message to the server process.
    pub fn with_link(
        accounts: Arc<A>,
        sessions: Arc<SessionManager>,
        link: Arc<dyn GatewayLink>,
        guard: Arc<IpGuard>,
    ) -> GameBridge<A> {
        GameBridge {
            accounts,
            sessions,
            link,
            guard,
            view_generation: (rand::random::<u64>() & MAX_VIEW_WIRE_INTEGER).max(1),
            view_sequences: Mutex::new(HashMap::new()),
        }
    }

    fn next_view_sequence(&self, conn: ConnId, view: &'static str) -> Option<u64> {
        let mut sequences = locked(&self.view_sequences, "semantic-view sequence table");
        // Some server-initiated closes bypass `GameBridge::disconnected`.
        // Opportunistic exact pruning against the authoritative local
        // connection table keeps this side index bounded in those paths too.
        sequences.retain(|(owned_conn, _), _| {
            self.sessions.session_of_conn(ConnId(*owned_conn)).is_some()
        });
        let sequence = sequences.entry((conn.0, view)).or_insert(0);
        let next = sequence
            .checked_add(1)
            .filter(|next| *next <= MAX_VIEW_WIRE_INTEGER)?;
        *sequence = next;
        Some(next)
    }

    fn forget_view_sequences(&self, conn: ConnId) {
        locked(&self.view_sequences, "semantic-view sequence table")
            .retain(|(owned_conn, _), _| *owned_conn != conn.0);
    }

    /// Await the exact world-delivery barrier used before a local transport
    /// teardown. This is deliberately bounded: an exact `Barrier` is the only
    /// success response, `Busy` gets a few retries, and boundary/protocol/world
    /// failures return `false` for an explicit degraded detach.
    pub(crate) async fn delivery_barrier(&self, conn: ConnId, operation: &'static str) -> bool {
        let deadline = Instant::now() + DELIVERY_BARRIER_TIMEOUT;
        for attempt in 1..=DELIVERY_BARRIER_ATTEMPTS {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let response =
                match tokio::time::timeout(remaining, self.link.request(ServiceReq::Barrier)).await
                {
                    Ok(response) => response,
                    Err(_) => {
                        tracing::warn!(conn = conn.0, operation, "delivery barrier timed out");
                        return false;
                    }
                };
            match response {
                Some(ServiceResp::Barrier) => return true,
                Some(ServiceResp::Busy) if attempt < DELIVERY_BARRIER_ATTEMPTS => {
                    tracing::warn!(
                        conn = conn.0,
                        operation,
                        attempt,
                        "delivery barrier admission busy; retrying"
                    );
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    tokio::time::sleep(DELIVERY_BARRIER_RETRY_DELAY.min(remaining)).await;
                }
                Some(ServiceResp::Busy) => {
                    tracing::warn!(
                        conn = conn.0,
                        operation,
                        attempts = DELIVERY_BARRIER_ATTEMPTS,
                        "delivery barrier remained busy; degrading to bounded teardown"
                    );
                    return false;
                }
                Some(ServiceResp::Unavailable) | None => {
                    tracing::warn!(
                        conn = conn.0,
                        operation,
                        "delivery barrier unavailable; degrading to bounded teardown"
                    );
                    return false;
                }
                Some(other) => {
                    tracing::error!(
                        conn = conn.0,
                        operation,
                        ?other,
                        "delivery barrier returned the wrong response type"
                    );
                    return false;
                }
            }
        }
        tracing::warn!(
            conn = conn.0,
            operation,
            "delivery barrier deadline elapsed; degrading to bounded teardown"
        );
        false
    }

    /// The actor a connection is playing, if attached.
    pub fn actor_of(&self, conn: ConnId) -> Option<ObjId> {
        self.sessions
            .session_of_conn(conn)
            .and_then(|s| self.sessions.character_of(s))
            .map(char_key_obj)
    }

    /// Route a client's answer to a server-initiated request (interface plan
    /// I10, D-I-21). Shares the per-connection input flood bucket so a Respond
    /// flood cannot reach the world loop past the throttle, never idle-stamps,
    /// and drops a throttled or unattached answer silently (the server applies
    /// the deadline default). Advisory, one-way. Used by both bridges.
    pub async fn respond(&self, conn: u64, id: u64, epoch: String, choice: RespondChoice) {
        let conn_id = ConnId(conn);
        if self.sessions.check_input(conn_id) == FloodVerdict::Throttled {
            return;
        }
        let Some(actor) = self.actor_of(conn_id) else {
            return;
        };
        self.link
            .to_server(ToServer::Inbound(WireInbound::Respond {
                epoch,
                conn,
                actor,
                id,
                choice,
            }))
            .await;
    }

    async fn legal_actions_for(
        &self,
        conn: ConnId,
        actor: ObjId,
        subject: PerceivedRef,
    ) -> Result<Vec<ProjectedLegalAction>, ActionSend> {
        match self
            .link
            .request(ServiceReq::LegalActions {
                conn: conn.0,
                actor,
                subject,
            })
            .await
        {
            Some(ServiceResp::LegalActions(Ok(offers))) => Ok(offers),
            Some(ServiceResp::LegalActions(Err(_))) => Err(ActionSend::Rejected),
            Some(ServiceResp::Busy) => Err(ActionSend::Busy),
            Some(ServiceResp::Unavailable) | None => Err(ActionSend::Gone),
            Some(other) => {
                tracing::error!(
                    conn = conn.0,
                    ?other,
                    "legal-action query returned wrong response type"
                );
                Err(ActionSend::Gone)
            }
        }
    }

    fn action_receipt(&self, conn: ConnId, actor: ObjId, receipt: &ProjectedActionReceipt) {
        match receipt.view() {
            Ok(view) => {
                let commit = ProjectedCommitToken::new(receipt.commit())
                    .expect("validated projected receipt has a valid commit token");
                let event = Arc::new(OutputEvent::View(view));
                if self.sessions.send_acknowledgeable_to_conn(
                    conn,
                    commit.clone(),
                    Arc::clone(&event),
                ) == SendToConn::Unknown
                {
                    let _ = self.sessions.retain_acknowledgeable_for_char(
                        char_key(actor),
                        commit,
                        event,
                    );
                }
            }
            Err(error) => {
                tracing::error!(conn = conn.0, error = %error, "projected action receipt frame failed");
                send_event_to_conn(
                    &self.sessions,
                    conn,
                    OutputEvent::Sys(RichText::tag(
                        "error",
                        "The action committed, but its result could not be displayed.",
                    )),
                );
            }
        }
    }

    fn action_error(&self, conn: ConnId, actor: ObjId, outcome: ActionSend) {
        let message = match outcome {
            ActionSend::Accepted => return,
            ActionSend::Owned(status) => {
                send_event_to_conn(
                    &self.sessions,
                    conn,
                    OutputEvent::Sys(RichText::tag(
                        "system",
                        format!("Action accepted. Transaction: {status}"),
                    )),
                );
                return;
            }
            ActionSend::Finished(receipt) => {
                self.action_receipt(conn, actor, &receipt);
                return;
            }
            ActionSend::Busy => "The game is busy; try that action again.",
            ActionSend::Gone if self.link.is_restarting() => {
                "The server is restarting; your action was not submitted."
            }
            ActionSend::Gone => "The game is unavailable; your action was not submitted.",
            ActionSend::Rejected => "That action is no longer available.",
        };
        send_event_to_conn(
            &self.sessions,
            conn,
            OutputEvent::Sys(RichText::tag("error", message)),
        );
    }

    async fn handle_legal_action_line(&self, conn: ConnId, actor: ObjId, line: &str) -> bool {
        let (command, argument) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        if command.eq_ignore_ascii_case("action/status")
            || command.eq_ignore_ascii_case("+action/status")
        {
            let status = match ActionStatusId::new(argument.trim()) {
                Ok(status) => status,
                Err(_) => {
                    send_event_to_conn(
                        &self.sessions,
                        conn,
                        OutputEvent::Sys(RichText::tag(
                            "system",
                            "Usage: action/status <transaction>",
                        )),
                    );
                    return true;
                }
            };
            match self
                .link
                .request(ServiceReq::ActionStatus {
                    conn: conn.0,
                    actor,
                    status,
                })
                .await
            {
                Some(ServiceResp::ActionStatus(ProjectedActionStatus::Committed(receipt))) => {
                    self.action_receipt(conn, actor, &receipt);
                }
                Some(ServiceResp::ActionStatus(ProjectedActionStatus::Pending { reference })) => {
                    send_event_to_conn(
                        &self.sessions,
                        conn,
                        OutputEvent::Sys(RichText::tag(
                            "system",
                            format!("Action is still pending. Transaction: {reference}"),
                        )),
                    );
                }
                Some(ServiceResp::ActionStatus(ProjectedActionStatus::Expired)) => {
                    send_event_to_conn(
                        &self.sessions,
                        conn,
                        OutputEvent::Sys(RichText::tag(
                            "system",
                            "That transaction is outside the retained status horizon.",
                        )),
                    );
                }
                Some(ServiceResp::ActionStatus(ProjectedActionStatus::Unavailable {
                    reference,
                })) => {
                    send_event_to_conn(
                        &self.sessions,
                        conn,
                        OutputEvent::Sys(RichText::tag(
                            "error",
                            format!("That transaction needs recovery. Transaction: {reference}"),
                        )),
                    );
                }
                Some(ServiceResp::ActionStatus(ProjectedActionStatus::Unknown)) => {
                    send_event_to_conn(
                        &self.sessions,
                        conn,
                        OutputEvent::Sys(RichText::tag(
                            "error",
                            "No accessible transaction has that reference.",
                        )),
                    );
                }
                Some(ServiceResp::Busy) => self.action_error(conn, actor, ActionSend::Busy),
                Some(ServiceResp::Unavailable) | None => {
                    self.action_error(conn, actor, ActionSend::Gone);
                }
                Some(other) => {
                    tracing::error!(
                        conn = conn.0,
                        ?other,
                        "action status returned wrong response type"
                    );
                    self.action_error(conn, actor, ActionSend::Gone);
                }
            }
            return true;
        }
        if command.eq_ignore_ascii_case("+actions") {
            let subject = if argument.trim().is_empty() {
                PerceivedRef::SelfActor
            } else {
                match parse_line_subject(argument) {
                    Ok(subject) => subject,
                    Err(message) => {
                        send_event_to_conn(
                            &self.sessions,
                            conn,
                            OutputEvent::Sys(RichText::tag(
                                "system",
                                format!("{message} Usage: +actions [#<object>]"),
                            )),
                        );
                        return true;
                    }
                }
            };
            match self.legal_actions_for(conn, actor, subject).await {
                Ok(offers) if offers.is_empty() => send_event_to_conn(
                    &self.sessions,
                    conn,
                    OutputEvent::Sys(RichText::tag("system", "No actions are available.")),
                ),
                Ok(offers) => {
                    let text = render_legal_action_list(&offers);
                    send_event_to_conn(
                        &self.sessions,
                        conn,
                        OutputEvent::Sys(RichText::tag("system", text)),
                    );
                }
                Err(error) => self.action_error(conn, actor, error),
            }
            return true;
        }
        if !command.eq_ignore_ascii_case("+act") {
            return false;
        }
        let selection = match parse_line_action(argument) {
            Ok(selection) => selection,
            Err(message) => {
                send_event_to_conn(
                    &self.sessions,
                    conn,
                    OutputEvent::Sys(RichText::tag("system", message)),
                );
                return true;
            }
        };
        let offers = match self
            .legal_actions_for(conn, actor, selection.subject.clone())
            .await
        {
            Ok(offers) => offers,
            Err(error) => {
                self.action_error(conn, actor, error);
                return true;
            }
        };
        let mut matching = offers.into_iter().filter(|offer| {
            offer
                .presentation()
                .verb
                .eq_ignore_ascii_case(&selection.verb)
        });
        let Some(offer) = matching.next() else {
            self.action_error(conn, actor, ActionSend::Rejected);
            return true;
        };
        if matching.next().is_some() {
            tracing::error!(%actor, verb = selection.verb, "projected legal-action verb is ambiguous");
            self.action_error(conn, actor, ActionSend::Rejected);
            return true;
        }
        if !matches!(offer.targets(), ProjectedTargetSchema::None) {
            send_event_to_conn(
                &self.sessions,
                conn,
                OutputEvent::Sys(RichText::tag(
                    "system",
                    "That action needs a structured target selector.",
                )),
            );
            return true;
        }
        let outcome = self
            .link
            .submit_action(GatewayAction {
                conn: conn.0,
                actor,
                offered_action: offer.offer(),
                subject: offer.subject().clone(),
                targets: Vec::new(),
                parameters: selection.parameters,
            })
            .await;
        self.action_error(conn, actor, outcome);
        true
    }

    pub(crate) async fn legal_actions_panel(&self, conn: ConnId, id: u64, actor: ObjId) {
        let event = match self
            .legal_actions_for(conn, actor, PerceivedRef::SelfActor)
            .await
        {
            Ok(offers) => match serde_json::to_value(offers).and_then(|data| {
                let sequence = self
                    .next_view_sequence(conn, "legal.actions")
                    .ok_or_else(|| {
                        serde_json::Error::io(std::io::Error::other(
                            "legal-action view sequence exhausted",
                        ))
                    })?;
                ViewEnvelope::keyframe("legal.actions", 1, self.view_generation, sequence, data)
                    .map_err(|error| serde_json::Error::io(std::io::Error::other(error)))
            }) {
                Ok(view) => OutputEvent::View(view),
                Err(error) => {
                    tracing::error!(conn = conn.0, error = %error, "legal-action panel serialization failed");
                    OutputEvent::Sys(RichText::tag("error", "Legal actions are unavailable."))
                }
            },
            Err(ActionSend::Busy) => {
                OutputEvent::Sys(RichText::tag("error", "The game is busy; try again."))
            }
            Err(ActionSend::Gone) => {
                OutputEvent::Sys(RichText::tag("error", "The game is unavailable."))
            }
            Err(ActionSend::Rejected) => {
                OutputEvent::Sys(RichText::tag("error", "Legal actions are unavailable."))
            }
            Err(ActionSend::Accepted | ActionSend::Owned(_) | ActionSend::Finished(_)) => {
                unreachable!("successful disposition is not a legal-action query error")
            }
        };
        send_event_to_conn(
            &self.sessions,
            conn,
            OutputEvent::Reply {
                id,
                event: Box::new(event),
            },
        );
    }

    /// Answer a [`mush_proto::Query::NarrativeBacklog`] locally (interface plan I12) from
    /// the caller's own resume ring — the ring is a per-viewer, already
    /// perceive/@ignore-gated backlog, so the reply carries only lines this
    /// viewer already saw. Read-only and conn-scoped (never re-enters the
    /// ring). The flood bucket is charged strictly (ignoring flood-exempt),
    /// since a locally-answered query has no world-side backpressure.
    pub(crate) fn narrative_backlog_reply(
        &self,
        conn: ConnId,
        id: u64,
        before_seq: Option<u64>,
        limit: u32,
    ) {
        let Some(sess) = self.sessions.session_of_conn(conn) else {
            return;
        };
        let limit = (limit as usize).clamp(1, MAX_NARRATIVE_BACKLOG);
        let bl = self
            .sessions
            .backlog_matching(sess, before_seq, limit, |ev| {
                matches!(ev, OutputEvent::Narrative { .. })
            })
            .unwrap_or_default();
        // A per-page byte budget (I12 verify fold): a ring pre-filled with
        // long lines must not amplify one pull into a multi-megabyte reply
        // multiplied by the connection's queue. Past the budget the page is
        // cut short and `more` set, so the client simply pages again. At
        // least one line always ships, so a single oversized line can never
        // stall paging.
        const MAX_NARRATIVE_BACKLOG_BYTES: usize = 256 * 1024;
        let mut more = bl.more;
        let mut bytes = 0usize;
        let mut lines: Vec<serde_json::Value> = Vec::with_capacity(bl.lines.len());
        for env in &bl.lines {
            let OutputEvent::Narrative { kind, actor, text } = &env.event else {
                continue;
            };
            let size = text.to_plain_text().len();
            if !lines.is_empty() && bytes + size > MAX_NARRATIVE_BACKLOG_BYTES {
                more = true;
                break;
            }
            bytes += size;
            lines.push(serde_json::json!({
                "seq": env.seq,
                "ts": env.ts,
                "kind": kind,
                "actor": actor,
                "text": text,
            }));
        }
        let data = serde_json::json!({
            "backlog": {
                "before_seq": before_seq,
                "more": more,
                "evicted": bl.evicted,
                "lines": lines,
            }
        });
        send_event_to_conn(
            &self.sessions,
            conn,
            OutputEvent::Reply {
                id,
                event: Box::new(OutputEvent::Panel {
                    feed: SmolStr::new_static("narrative"),
                    kind: PanelFrameKind::Delta,
                    data,
                }),
            },
        );
    }

    /// Check account credentials. A blocked address is denied outright.
    /// (The backend runs the argon2id verify on the blocking pool.)
    pub async fn auth_account(&self, account: &str, password: &str, addr: IpAddr) -> AuthResult {
        if self.guard.is_blocked(addr) {
            return AuthResult::Denied;
        }
        match self.accounts.verify(account, password).await {
            AuthOutcome::Ok {
                account,
                characters,
            } => AuthResult::Ok {
                account,
                characters: characters
                    .into_iter()
                    .map(|c| CharSummary {
                        id: u64::from(c.id.0),
                        name: c.name,
                    })
                    .collect(),
            },
            AuthOutcome::BadCredentials => AuthResult::Denied,
        }
    }

    /// Create an account plus its first character.
    pub async fn create_account(
        &self,
        account: &str,
        password: &str,
        char_name: &str,
        addr: IpAddr,
    ) -> Result<CharSummary, CreateErr> {
        if self.guard.is_blocked(addr) {
            return Err(CreateErr::Io("your address is blocked".to_string()));
        }
        // Validation + argon2 hash + store reservation + ack-gated world
        // round trip; the backend hops CPU-heavy work to the blocking pool.
        let created = self.accounts.create(account, password, char_name).await?;
        Ok(CharSummary {
            id: u64::from(created.id.0),
            name: created.name,
        })
    }

    /// Create an additional character on an authenticated account. Cap and
    /// ack gating live in the backend.
    pub async fn create_character(
        &self,
        account: &str,
        char_name: &str,
    ) -> Result<CharSummary, CreateErr> {
        let created = self.accounts.create_character(account, char_name).await?;
        Ok(CharSummary {
            id: u64::from(created.id.0),
            name: created.name,
        })
    }

    /// Attach an already-authenticated, already-authorized connection
    /// handle to a character and bring it into the game: session attach,
    /// world `ConnUp` (announces connect/reconnect), `SessionInfo`, and the
    /// attach look (the core `room` query, answered to this one connection).
    /// The caller has verified the character belongs to
    /// the account and built the handle with the right
    /// [`Sink`](mush_session::Sink) for its wire.
    pub async fn enter_game(
        &self,
        handle: ConnectionHandle,
        actor: ObjId,
        account: &str,
        char_name: &str,
        caps_structured: bool,
    ) -> Result<EnterOutcome, ConnectErr> {
        let conn_id = handle.id;
        // Reserve split-process lifecycle custody BEFORE `SessionManager`
        // mutation. Once this succeeds, ConnUp is must-deliver; an overloaded
        // boundary refuses here, so a failed admission cannot revive and then
        // accidentally destroy a link-dead session during rollback.
        match self.link.try_reserve_connection(conn_id.0, actor) {
            ConnectionAdmission::Reserved => {}
            ConnectionAdmission::Busy => return Err(ConnectErr::Busy),
            ConnectionAdmission::Gone => return Err(ConnectErr::WorldGone),
        }
        let (sess, outcome) = self
            .sessions
            .attach(handle, char_key(actor), account.to_string());
        // The resume baseline is captured BEFORE this connection's own
        // entry output advances the session counter.
        let resume_from = self.sessions.last_seq(sess).unwrap_or(0);
        let (connections, from_linkdead) = match outcome {
            mush_session::AttachOutcome::New => (1, false),
            mush_session::AttachOutcome::Rejoined {
                others,
                from_linkdead,
            } => (others + 1, from_linkdead),
        };

        // World side: connection up (announces connect/reconnect), then the
        // attach look (the `room` query below) so the player sees where they
        // are.
        if !self
            .link
            .to_server(ToServer::Inbound(WireInbound::ConnUp {
                conn: conn_id.0,
                actor,
                caps_structured,
                from_linkdead,
            }))
            .await
        {
            self.link.cancel_connection_reservation(conn_id.0);
            self.rollback_local_attach(conn_id, from_linkdead);
            return Err(ConnectErr::WorldGone);
        }
        // `ConnUp` may run persistent connect/reconnect hooks. Fence their
        // exact durable outcome before publishing the local session or asking
        // any gameplay/world-read question: hard-mode correctly refuses those
        // reads while the hook's commit is still pending, and treating that
        // refusal as "not in an input context" would either skip the initial
        // room view or show it over a live input context's prompt.
        if !self.delivery_barrier(conn_id, "connect readiness").await {
            self.compensate_announced_attach(conn_id, actor, from_linkdead)
                .await;
            return Err(ConnectErr::WorldGone);
        }

        // A barrier proves ordering/durability, but it does not by itself prove
        // the preceding lifecycle message was admitted (hard boot/recovery
        // modes may refuse connection hooks) or that its actor existed. Require
        // the exact engine-side association before publishing SessionInfo.
        if !matches!(
            self.link
                .request(ServiceReq::ConnectionReady {
                    conn: conn_id.0,
                    actor,
                })
                .await,
            Some(ServiceResp::ConnectionReady(true))
        ) {
            self.compensate_announced_attach(conn_id, actor, from_linkdead)
                .await;
            return Err(ConnectErr::WorldGone);
        }

        send_event_to_conn(
            &self.sessions,
            conn_id,
            OutputEvent::SessionInfo {
                character: ObjRef::new(actor.to_string(), char_name.to_string()),
                account: account.to_string(),
                connections: u8::try_from(connections).unwrap_or(u8::MAX),
            },
        );
        // **The attach look** (16b cut B.4, `B-14`): the player is shown their
        // room by the core `room` QUERY, not by a `look` command. A command runs
        // as the actor and its output goes to the actor — to EVERY connection
        // attached to the character — so a second connection's attach printed
        // the whole room on the first one too. A query's answer is a
        // `Reply { id: 0, event: Room }` the world sends to this one connection
        // (`SendTarget::Conn`, by the query path's own construction); `id` 0 is
        // the server-initiated reply nobody correlates, and it is registered
        // nowhere. The `LookRendered` hook lines a typed `look` appends are not
        // on this view; a typed `look` is unchanged.
        //
        // The guard stands as it stood for the old `look`: skip the view when
        // the `ConnUp` above dropped the actor into an input context (the
        // chargen wizard's connect-time drop; or a mail compose that survived a
        // linkdead drop, on reconnect), and let the context's own prompt stand.
        // Fail closed on Busy/unavailable/unexpected responses: the view is
        // sent only after an exact negative answer.
        if matches!(
            self.link
                .request(ServiceReq::InInputContext { actor })
                .await,
            Some(ServiceResp::InInputContext(false))
        ) {
            self.link
                .to_server(ToServer::Inbound(WireInbound::Query {
                    conn: conn_id.0,
                    actor,
                    id: 0,
                    name: SmolStr::new_static("room"),
                    args: serde_json::json!({}),
                }))
                .await;
        }
        tracing::info!(conn = conn_id.0, %actor, account, "character connected");
        Ok(EnterOutcome { sess, resume_from })
    }

    /// One input line from an attached connection.
    pub async fn line(&self, conn: ConnId, line: &str) -> LineOutcome {
        // Idle bookkeeping first — even a throttled keypress is activity.
        self.sessions.record_input(conn);
        // Cheap byte-length reject BEFORE the O(n) `sanitize_line` scan: a
        // frame whose byte length exceeds `MAX_LINE_CHARS * 4` (the max UTF-8
        // bytes per char) cannot possibly be within the char cap, so drop it
        // without scanning the whole (up to 64 KiB) frame. The SSH editor caps
        // upstream; only the web path can deliver a frame this large here, so
        // this bounds the sanitize cost a client can force per frame.
        if line.len() > MAX_LINE_CHARS.saturating_mul(4) {
            send_event_to_conn(
                &self.sessions,
                conn,
                OutputEvent::Sys(RichText::tag("error", "Line too long.")),
            );
            return LineOutcome::Continue;
        }
        let line = sanitize_line(line);
        // SSH's LineEditor never lets a submitted line grow past
        // `MAX_LINE_CHARS` (bell + drop beyond, mid-typing); the web path
        // has no such gate upstream of this shared entry point — only the
        // 64 KiB WS frame cap — so enforce the same char bound here, where
        // both transports meet (MEDIUM finding). Measured pre-trim so
        // padding can't hide an oversize paste under the cap.
        if line.chars().count() > MAX_LINE_CHARS {
            send_event_to_conn(
                &self.sessions,
                conn,
                OutputEvent::Sys(RichText::tag("error", "Line too long.")),
            );
            return LineOutcome::Continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return LineOutcome::Continue;
        }
        // QUIT is checked before the flood throttle: a flooding client must
        // always be able to leave.
        if trimmed.eq_ignore_ascii_case("QUIT") {
            self.quit(conn).await;
            return LineOutcome::Close;
        }
        if self.sessions.check_input(conn) == FloodVerdict::Throttled {
            send_event_to_conn(
                &self.sessions,
                conn,
                OutputEvent::Sys(RichText::tag("error", "You're doing that too fast.")),
            );
            return LineOutcome::Continue;
        }
        let Some(actor) = self.actor_of(conn) else {
            tracing::warn!(conn = conn.0, "line from unattached connection dropped");
            return LineOutcome::Continue;
        };
        if self.handle_legal_action_line(conn, actor, trimmed).await {
            return LineOutcome::Continue;
        }
        match self
            .link
            .to_server_input(ToServer::Inbound(WireInbound::Cmd {
                conn: conn.0,
                actor,
                line: trimmed.to_string(),
            }))
            .await
        {
            InputSend::Sent => {}
            // M18 input-shed: the gateway->server queue is full (a wedged or slow
            // server), so this Cmd was dropped with a logged warn + counter bump.
            // Tell the player it did not land — the connection is NOT dropped.
            // Distinct from the restart-gap and shutdown cases below.
            InputSend::Shed => {
                send_event_to_conn(
                    &self.sessions,
                    conn,
                    OutputEvent::Sys(RichText::tag("system", "The server is busy; try again.")),
                );
            }
            // D3 (M16c): during a server-restart gap the boundary is down, so the
            // send is refused. Surface a clear "restarting" notice instead of
            // silently dropping the line — the connection is NOT dropped. The
            // in-process all-in-one link never reports `is_restarting` (its
            // default is `false`) and never sheds, so its behaviour is unchanged
            // (a `Gone` there is a shutdown, already terminal — stay silent).
            InputSend::Gone => {
                if self.link.is_restarting() {
                    send_event_to_conn(
                        &self.sessions,
                        conn,
                        OutputEvent::Sys(RichText::tag(
                            "system",
                            "The server is restarting; hold on — your input was not sent.",
                        )),
                    );
                }
            }
        }
        LineOutcome::Continue
    }

    /// A read-only world query from an attached connection — the shared half
    /// of the SSH TUI's structured `Inbound::Query` path (milestone M15),
    /// mirroring the web bridge's `request` seam. Unlike [`line`](Self::line)
    /// this does **not** stamp idle (`record_input`): a background panel /
    /// widget refresh is not the player typing, so a WHO-rail poll must never
    /// reset the actor's idle time. It still shares the per-connection input
    /// flood bucket ([`check_input`](SessionManager::check_input)) so a widget
    /// cannot spam the world task past the throttle. A throttled or unattached
    /// query is dropped silently (the widget simply keeps its last data).
    ///
    /// The engine answers with an [`OutputEvent::Reply`] addressed to `conn`
    /// only, wrapping the query handler's event; permissions are evaluated as
    /// the connection's own actor, so a widget can never read another actor's
    /// data (invariant T3).
    pub async fn query(&self, conn: u64, id: u64, name: SmolStr, args: serde_json::Value) {
        let conn_id = ConnId(conn);
        // Interface plan I12: the narrative backlog is answered LOCALLY from the
        // resume ring (the world cannot read the gateway ring), gated by a
        // STRICT flood check that ignores flood-exempt — a locally-answered
        // query has no world-side backpressure. Dispatched before the generic,
        // exempt-honouring `check_input`.
        if name == "narrative.backlog" {
            if self.sessions.check_query_flood(conn_id) == FloodVerdict::Throttled {
                return;
            }
            let before_seq = args.get("before_seq").and_then(serde_json::Value::as_u64);
            let limit = args
                .get("limit")
                .and_then(serde_json::Value::as_u64)
                .and_then(|l| u32::try_from(l).ok())
                .unwrap_or(0);
            self.narrative_backlog_reply(conn_id, id, before_seq, limit);
            return;
        }
        if self.sessions.check_input(conn_id) == FloodVerdict::Throttled {
            return;
        }
        let Some(actor) = self.actor_of(conn_id) else {
            tracing::warn!(conn, "query from unattached connection dropped");
            return;
        };
        if name == "legal.actions" {
            self.legal_actions_panel(conn_id, id, actor).await;
            return;
        }
        self.link
            .to_server(ToServer::Inbound(WireInbound::Query {
                conn,
                actor,
                id,
                name,
                args,
            }))
            .await;
    }

    /// The transport lost (or closed) a connection. (Its IP-guard slot is
    /// released by the transport's own accept/drop accounting, not here.)
    pub async fn disconnected(&self, conn: ConnId) {
        let actor = self.actor_of(conn);
        let outcome = self.sessions.detach(conn, DetachReason::Lost);
        self.forget_view_sequences(conn);
        let (Some(actor), Some(outcome)) = (actor, outcome) else {
            // Already detached (QUIT raced the transport teardown) or never
            // attached: nothing game-side.
            return;
        };
        match outcome {
            DetachOutcome::OthersRemain | DetachOutcome::Linkdead { .. } => {
                // The engine announces "has gone link-dead." when this was
                // the actor's last connection.
                self.link
                    .to_server(ToServer::Inbound(WireInbound::ConnDown {
                        conn: conn.0,
                        actor,
                        reason: DisconnectKind::Lost,
                    }))
                    .await;
            }
            DetachOutcome::Closed => {
                // Session vanished underneath us (e.g. concurrent close):
                // clean scoped cleanup for this connection, then a full
                // disconnect (which the engine ignores if a fresh session's
                // connections are live — treaty ruling 3 re-validation).
                self.link
                    .to_server(ToServer::Inbound(WireInbound::ConnDown {
                        conn: conn.0,
                        actor,
                        reason: DisconnectKind::Boot,
                    }))
                    .await;
                self.link
                    .to_server(ToServer::Inbound(WireInbound::SessionGone { actor }))
                    .await;
            }
        }
        if let DetachOutcome::Linkdead { grace } = outcome {
            tracing::info!(conn = conn.0, %actor, grace_s = grace.as_secs(), "linkdead");
            self.arm_linkdead_reaper(grace);
        }
    }

    /// Auth gate: is the address currently blocked (tempban or siteban)?
    #[must_use]
    pub fn is_ip_blocked(&self, addr: IpAddr) -> bool {
        self.guard.is_blocked(addr)
    }

    /// Record one failed **password** attempt against an IP — the counter
    /// that arms the tempban. A refused web token is not one of these; see
    /// [`GameBridge::token_refused`].
    pub fn auth_failed(&self, addr: IpAddr) {
        self.guard.record_failure(addr);
    }

    /// Record one refused **bearer token** against an IP, on the guard's
    /// separate counter (`mush_config::ipguard`): it may arm a short
    /// cool-off, and can never tempban.
    pub fn token_refused(&self, addr: IpAddr) {
        self.guard.record_token_failure(addr);
    }

    /// Whether the address is inside a refused-token cool-off.
    #[must_use]
    pub fn is_token_throttled(&self, addr: IpAddr) -> bool {
        self.guard.is_token_throttled(addr)
    }

    /// How much of the address's tempban is left, for a transport that tells
    /// a refused client when to come back. `None` when it is not tempbanned
    /// (a siteban answers [`GameBridge::siteban_active`] instead).
    #[must_use]
    pub fn tempban_remaining(&self, addr: IpAddr) -> Option<Duration> {
        self.guard.tempban_remaining(addr)
    }

    /// Whether a persisted siteban currently covers the address.
    #[must_use]
    pub fn siteban_active(&self, addr: IpAddr) -> bool {
        self.guard
            .siteban_active_at(addr, mush_config::unix_now_s())
    }

    /// The `QUIT` flow, serialized through the world task so a quitter's
    /// in-flight output is never dropped:
    ///
    /// 1. `ConnDown(Quit)` is queued **behind** every line the connection
    ///    already sent; the engine processes those first (their output
    ///    reaches this connection's queue), then announces the clean
    ///    disconnect ("has disconnected." on the last connection) and
    ///    forgets the conn.
    /// 2. A world barrier guarantees all of that has been delivered.
    /// 3. Only then the farewell + `Disconnect` event go out and the session
    ///    detaches. Detaching drops the session's `ConnectionHandle` (and
    ///    with it the output sender), which is what tells the transport's
    ///    pump to close the socket after draining.
    pub(crate) async fn quit(&self, conn: ConnId) {
        if let Some(actor) = self.actor_of(conn) {
            self.link
                .to_server(ToServer::Inbound(WireInbound::ConnDown {
                    conn: conn.0,
                    actor,
                    reason: DisconnectKind::Quit,
                }))
                .await;
            self.delivery_barrier(conn, "quit").await;
        }
        send_event_to_conn(
            &self.sessions,
            conn,
            OutputEvent::Disconnect {
                reason: RichText::tag("system", "Goodbye."),
            },
        );
        self.sessions.detach(conn, DetachReason::Quit);
        self.forget_view_sequences(conn);
    }

    /// Arm one reaper for a session that just went linkdead: after the
    /// grace period, every expired linkdead session is closed and its
    /// character fully disconnected. Reattachment in the meantime revives
    /// the session and [`SessionManager::take_expired`] skips it.
    pub(crate) fn arm_linkdead_reaper(&self, grace: std::time::Duration) {
        let sessions = Arc::clone(&self.sessions);
        let link = Arc::clone(&self.link);
        tokio::spawn(async move {
            tokio::time::sleep(grace).await;
            for expired in sessions.take_expired(Instant::now()) {
                let actor = char_key_obj(expired.character);
                tracing::info!(%actor, account = %expired.account, "linkdead grace expired");
                link.to_server(ToServer::Inbound(WireInbound::SessionGone { actor }))
                    .await;
            }
        });
    }

    /// Undo a failed local session attach.
    ///
    /// This helper changes only the local session table; a caller whose
    /// `ConnUp` entered world custody must enqueue its compensating `ConnDown`
    /// first. If the attach had revived a link-dead session, detach it as
    /// unclean so the prior session and resume ring are restored, then re-arm
    /// expiry. A failed brand-new attach is closed cleanly and leaves no
    /// phantom local session.
    pub(crate) fn rollback_local_attach(&self, conn: ConnId, revived_linkdead: bool) {
        let reason = if revived_linkdead {
            DetachReason::Lost
        } else {
            DetachReason::Booted
        };
        if let Some(DetachOutcome::Linkdead { grace }) = self.sessions.detach(conn, reason) {
            tracing::info!(
                conn = conn.0,
                grace_s = grace.as_secs(),
                "failed reconnect rolled back to linkdead"
            );
            self.arm_linkdead_reaper(grace);
        }
        self.forget_view_sequences(conn);
    }

    /// Undo an attach after `ConnUp` entered world custody. The compensating
    /// lifecycle edge is must-deliver once a boundary accepts it; if the
    /// boundary has vanished, restart resynchronization omits the local
    /// connection after the rollback below.
    async fn compensate_announced_attach(
        &self,
        conn: ConnId,
        actor: ObjId,
        revived_linkdead: bool,
    ) {
        let reason = if revived_linkdead {
            DisconnectKind::Lost
        } else {
            DisconnectKind::Boot
        };
        if !self
            .link
            .to_server(ToServer::Inbound(WireInbound::ConnDown {
                conn: conn.0,
                actor,
                reason,
            }))
            .await
        {
            tracing::warn!(
                conn = conn.0,
                %actor,
                "connect cleanup retained only by restart resynchronization"
            );
        }
        self.rollback_local_attach(conn, revived_linkdead);
    }
}

#[cfg(test)]
mod projection_tests {
    use super::{parse_line_action, render_legal_action_list};
    use mush_contract_tests::{
        assert_projection_text_clean, install_toy_focus, toy_brace_offers, toy_definitions,
        toy_world_with_state,
    };
    use mush_core::{GOD, ObjId, OfferKeyRing, OfferSecret, PerceivedRef};

    #[test]
    fn line_action_parser_accepts_self_and_staff_subject_parameters() {
        let own =
            parse_line_action(r#"willpower-roll with {"pool":"strength+brawl","difficulty":6}"#)
                .unwrap();
        assert_eq!(own.verb, "willpower-roll");
        assert_eq!(own.subject, PerceivedRef::SelfActor);
        assert_eq!(own.parameters["pool"], "strength+brawl");

        let staff =
            parse_line_action(r#"condition-damage ON #42 WITH {"amount":3,"kind":"physical"}"#)
                .unwrap();
        assert_eq!(
            staff.subject,
            PerceivedRef::KnownObject {
                id: ObjId(42),
                revision: 1,
            }
        );
        assert_eq!(staff.parameters["amount"], 3);

        assert!(parse_line_action("brace with []").is_err());
        assert!(parse_line_action("condition-damage on Nick with {}").is_err());
    }

    #[test]
    fn line_action_list_renders_only_projected_presentation() {
        let (mut world, families) = toy_world_with_state();
        install_toy_focus(world.engine_mut(), &families, GOD, 3);
        world.engine_mut().set_offer_keys(
            OfferKeyRing::new(OfferSecret::new(1, [0x53; 32]).unwrap(), []).unwrap(),
        );
        let offers = toy_brace_offers(world.engine(), GOD, 11, 4)
            .unwrap()
            .projected();
        let rendered = render_legal_action_list(&offers);

        assert!(rendered.contains("+act brace — Brace"));
        assert_projection_text_clean(
            "line legal-action list",
            &rendered,
            &["scene_turn", &toy_definitions().brace.digest().to_string()],
        );
    }
}

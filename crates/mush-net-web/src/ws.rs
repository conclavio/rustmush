//! `GET /ws` — the WebSocket session: handshake, resume, and the pump.
//!
//! ## Handshake
//!
//! The token travels **only** in the `Sec-WebSocket-Protocol` subprotocol
//! `rustmush.v1.<token>` — browsers cannot set arbitrary headers on a
//! WebSocket, but they can set this one. An earlier `?token=` query-string
//! fallback was removed (LOW finding): reverse proxies log full request
//! URIs by default, and nothing ships that used the fallback. Never logged
//! either way: the trace layer's span carries only the path (see
//! `app::scrubbed_span`), and this module never puts the token or the query
//! string into an event. The server echoes the token back as the selected
//! subprotocol — required for browsers to accept the handshake; the
//! response only reaches the client that sent it.
//!
//! `?char=<key>` selects the character; `?resume=<last_seq>` (optional)
//! asks for a gap replay instead of a fresh attach. Resume is
//! **connection-setup only**: a [`ClientCommand::Resume`] arriving on an
//! established connection is a protocol error (close 1008).
//!
//! ## Frame order after upgrade
//!
//! 1. `hello` (proto version, server name, features, panels)
//! 2. when resuming: `resumed {replayed}` — `true` and the gap replay
//!    follows, or `false` (gap outgrew the ring / plain `?resume` fell
//!    back) and the game sends fresh `Room` + panel snapshots instead
//! 3. the live envelope stream
//!
//! ## Out-of-band envelopes (`seq: 0`)
//!
//! Envelopes stamped by the session layer carry seq ≥ 1. Frames this crate
//! generates itself — `hello`, `resumed`, keepalive `ping` — are
//! connection-scoped, not part of the session's resumable stream, and carry
//! `seq: 0`; clients must not treat 0 as a resume point.
//!
//! ## The picture (doc 21 cut G.3b; doc 28 P2)
//!
//! Every web picture (`Cockpit`, `Overhead`, and the bucket-B scenes) is
//! rendered client-side by `mush-pixel-wasm` from the `Panel` keyframes this
//! socket already forwards. [`ClientCommand::Picture`] stays on the wire as a
//! no-op so an older bundle asking for a frame is not closed as a protocol
//! error.
//!
//! ## Keepalive & backpressure
//!
//! App-level `ping` every `ping_interval_ms` (proxies eat WS protocol
//! pings); a client that leaves `max_missed_pongs` pings unanswered is
//! closed as dead. Outbound envelopes come from a bounded queue; a socket
//! too slow to accept a frame within `send_timeout_ms` is closed (1013) —
//! the session layer's resume ring lets it recover the tail on reconnect.
//!
//! ## Refusing a token so the client can hear it
//!
//! A browser cannot see the HTTP status of a failed WebSocket upgrade: the
//! WebSocket API reports every pre-upgrade refusal as close 1006, which is
//! indistinguishable from a network blip, so a client told "your token is
//! dead" that way retries forever. A refused token therefore **completes
//! the upgrade** (echoing the offered subprotocol, which is what lets the
//! browser accept it) and then closes with
//! [`close_code::BAD_SESSION`] and [`SESSION_ENDED`] — the same 4401 the
//! attach-time refusals use, which the SPA already treats as fatal: it drops
//! the stored token and returns to the login form. The cost is one WS
//! handshake per refusal, which the cool-off below bounds.
//!
//! ## Only a dead credential may be fatal
//!
//! 4401 is expensive — the client answers it by **deleting its token**, and
//! nothing the player can do brings it back. So this route spends it only on
//! a refusal that is genuinely about the credential, and every arm of the
//! handshake is typed to say which it is:
//!
//! - [`WebHooks::validate_token`] answers a [`TokenVerdict`], so a store
//!   that cannot be reached ([`TokenVerdict::Unavailable`]) is refused with
//!   a plain 503 and counts nothing. Folding it into "refused" would have
//!   signed every web player out, with a valid 7-day token, the moment the
//!   instance Redis blinked during a deploy.
//! - [`WebHooks::connect_character`] answers an [`AttachErr`], so an
//!   overloaded or restarting game half maps to 1013/1012 — the same two
//!   codes the resume arm has always used for the same two conditions
//!   ([`ResumeErr::Busy`], [`ResumeErr::Unavailable`]). An ordinary
//!   server-half reload must not log a web player out.
//!
//! The rule in one line: **transient is 1006/503/1012/1013 and keeps the
//! token (4429 names a token cool-off); 4401 means the credential itself is gone.**
//!
//! ## Abuse protection (HIGH finding: `/ws` previously had none)
//!
//! A refused token counts one [`WebHooks::token_refused`] against the peer
//! — **not** [`WebHooks::auth_failed`]. It used to count as a credential
//! failure, which meant an open tab holding a token the server no longer
//! knew could tempban its own address out of SSH for fifteen minutes (dev1,
//! 2026-09-20). A 256-bit token is not guessable; a refused one is a stale
//! client. `IpGuard` keeps the two counters apart — passwords on the
//! five-in-ten-minutes tempban, refused tokens on a much looser cool-off —
//! and this route honours the cool-off via
//! [`WebHooks::is_token_throttled`] *before* it asks the store about a
//! token, so probing is cheap to refuse and can never ban anyone.
//!
//! Every upgrade also claims a per-IP concurrent-connection slot via
//! [`WebHooks::connection_opened`] before the handshake completes (over-cap
//! is refused pre-upgrade) and releases it via
//! [`WebHooks::connection_closed`] exactly once, however the socket task
//! ends — see [`ConnSlot`].

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use mush_proto::{ClientCommand, Envelope, OutputEvent, PROTO_VERSION};
use tokio::sync::{mpsc, watch};
use tokio::time::MissedTickBehavior;

use crate::app::AppState;
use crate::config::WebConfig;
use crate::hooks::{
    AttachErr, CloseRequest, ConnCloser, NewWebConn, ResumeErr, TokenVerdict, WebHooks,
};
use crate::ip::client_ip;

/// Subprotocol prefix carrying the auth token.
const SUBPROTO_PREFIX: &str = "rustmush.v1.";

/// The close reason a refused token carries. One plain sentence: the client
/// shows it (or its own wording) on the login form, so it says what happened
/// and what to do, and never why the token was refused.
pub const SESSION_ENDED: &str = "Your session has ended; please sign in again.";

/// Browser-visible explanation of a temporary refused-token cool-off.
pub const TOKEN_COOLOFF: &str = "Too many reconnects from this network; retrying in one minute.";

/// Close reason for a refusal the game will outgrow on its own: bounded
/// admission is full right now. Retryable — the client keeps its token.
const SERVER_BUSY: &str = "server busy; try again";

/// Close reason for a refusal caused by the game boundary being gone
/// mid-setup (a server-half restart). Retryable — the client keeps its
/// token. Shared by the resume arm and the fresh-attach arm so one
/// condition never gets two different answers depending on which arm the
/// reconnect happened to land in.
const GAME_RESTARTING: &str = "game restarting; reconnect shortly";

/// Close codes used by this module.
pub(crate) mod close_code {
    /// RFC 6455 policy violation — protocol errors (mid-connection resume,
    /// malformed/binary/oversize frames).
    pub const PROTOCOL: u16 = 1008;
    /// RFC 6455 "try again later" — client too slow to keep up.
    pub const SLOW: u16 = 1013;
    /// RFC 6455 "service restart" — the game boundary disappeared during
    /// connection setup; reconnecting shortly is safe.
    pub const SERVICE_RESTART: u16 = 1012;
    /// Application: token/session no longer valid at attach time.
    pub const BAD_SESSION: u16 = 4401;
    /// Application: refused-token cool-off; retain the credential and retry later.
    pub const TOKEN_COOLOFF: u16 = 4429;
    /// RFC 6455 going away — keepalive timed out or the game side closed
    /// the connection.
    pub const GOING_AWAY: u16 = 1001;
}

/// Query parameters of `GET /ws`, hand-parsed so the raw query never
/// transits a rejection body or a log line. A `token=` pair is deliberately
/// **not** recognized here (LOW finding: the fallback was removed) — it
/// falls into the ignored default case like any other unknown parameter.
#[derive(Debug, Default, PartialEq, Eq)]
struct WsParams {
    char_key: Option<u64>,
    resume: Option<u64>,
}

impl WsParams {
    fn parse(query: &str) -> Self {
        let mut out = Self::default();
        for pair in query.split('&') {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            let v = percent_decode(v);
            match k {
                "char" => out.char_key = v.parse().ok(),
                "resume" => out.resume = v.parse().ok(),
                _ => {}
            }
        }
        out
    }
}

/// Minimal `%XX` + `+` decoder (tokens are URL-safe, but be liberal).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let decoded = bytes
                    .get(i + 1..i + 3)
                    .and_then(|h| std::str::from_utf8(h).ok())
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                match decoded {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Extracts the token from a `Sec-WebSocket-Protocol: rustmush.v1.<token>`
/// offer (the header may list several protocols, comma-separated).
fn subprotocol_token(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::SEC_WEBSOCKET_PROTOCOL)?.to_str().ok()?;
    value
        .split(',')
        .map(str::trim)
        .find_map(|p| p.strip_prefix(SUBPROTO_PREFIX))
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

/// Milliseconds since the Unix epoch (for out-of-band envelope stamps).
fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Wraps a connection-scoped event in an out-of-band (`seq: 0`) envelope.
fn oob(event: OutputEvent) -> Envelope {
    Envelope::new(0, unix_ms(), event)
}

/// RAII guard for the per-IP concurrent-connection slot claimed by
/// [`WebHooks::connection_opened`] in `ws_route`, before the upgrade
/// completes. Its `Drop` calls [`WebHooks::connection_closed`] exactly
/// once, however the connection ends — an early refusal return in
/// `ws_route` (missing/invalid token, bad char, refused attach) or the end
/// of [`handle_socket`] for an upgraded socket — mirroring the
/// `Drop`-based `ConnHandler` release pattern `mush-net-ssh` uses against
/// the same per-IP counter.
struct ConnSlot {
    hooks: Arc<dyn WebHooks>,
    ip: IpAddr,
}

impl Drop for ConnSlot {
    fn drop(&mut self) {
        self.hooks.connection_closed(self.ip);
    }
}

/// `GET /ws` handler: pre-upgrade checks, then hand off to the pump.
pub(crate) async fn ws_route(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    ws: WebSocketUpgrade,
) -> Response {
    let ip = client_ip(peer.ip(), &headers, &state.config.trusted_proxies);
    if state.hooks.is_ip_blocked(ip) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !state.hooks.connection_opened(ip) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    // Claimed: released by `Drop` on every return from here on, including
    // the early refusals below (missing token, bad char, invalid token).
    let slot = ConnSlot {
        hooks: Arc::clone(&state.hooks),
        ip,
    };

    let params = WsParams::parse(query.as_deref().unwrap_or(""));
    let Some(token) = subprotocol_token(&headers) else {
        return (StatusCode::UNAUTHORIZED, "missing token").into_response();
    };
    let Some(character) = params.char_key else {
        return (StatusCode::BAD_REQUEST, "missing or invalid char parameter").into_response();
    };
    let cap = state.config.max_frame_bytes;
    let mut ws = ws.max_message_size(cap).max_frame_size(cap);
    ws = ws.protocols([format!("{SUBPROTO_PREFIX}{token}")]);

    if state.hooks.is_token_throttled(ip) {
        // Upgrade so the browser can distinguish cool-off from an opaque
        // network failure. The ordinary slot cap bounds this handshake too;
        // never query the store or increment either failure counter.
        return ws.on_upgrade(move |mut socket| async move {
            let _slot = slot;
            close(&mut socket, close_code::TOKEN_COOLOFF, TOKEN_COOLOFF).await;
        });
    }

    match state.hooks.validate_token(&token).await {
        TokenVerdict::Valid(_) => {}
        TokenVerdict::Refused => {
            // A refused token is a stale client, not a guess: it goes on the
            // cool-off counter, never on the one that tempbans (module docs).
            state.hooks.token_refused(ip);
            // Upgrade first, then close 4401 — a pre-upgrade 401 reaches a
            // browser as an opaque 1006 the client cannot tell from a dropped
            // network, which is how one open tab reconnected itself into a ban.
            return ws.on_upgrade(move |mut socket| async move {
                let _slot = slot;
                close(&mut socket, close_code::BAD_SESSION, SESSION_ENDED).await;
            });
        }
        TokenVerdict::Unavailable => {
            // The store could not answer, so we know NOTHING about this
            // token. Still fail closed — but pre-upgrade, as a 503, which is
            // exactly the shape a browser already treats as transient (1006:
            // keep the token, keep the backoff). Deliberately cheaper than
            // the 4401 path above: during a store outage every open tab is
            // reconnecting at once, and spending a WS handshake apiece on a
            // condition the client can only wait out would add load to the
            // thing that is already failing. Nothing is counted against the
            // address either — see `WebHooks::token_refused`.
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    }

    let resume = params.resume;
    ws.on_upgrade(move |socket| handle_socket(state, socket, ip, token, character, resume, slot))
}

/// What to say right after `hello`: nothing (fresh attach), or
/// `resumed {replayed}` plus the gap replay.
struct Handshake {
    replayed: bool,
    replay: Vec<Arc<Envelope>>,
}

async fn handle_socket(
    state: AppState,
    mut socket: WebSocket,
    ip: IpAddr,
    token: String,
    character: u64,
    resume: Option<u64>,
    // Held for its `Drop` only — releases the per-IP connection slot when
    // this function returns, however it returns.
    _conn_slot: ConnSlot,
) {
    let cfg = Arc::clone(&state.config);
    let hooks = Arc::clone(&state.hooks);
    let conn_id = state.next_conn.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel::<Arc<Envelope>>(cfg.send_queue.max(1));
    // The closer stays alive here for the pump's whole run: a game side that
    // dropped its clones must read as "no close asked", never as a closed
    // channel the pump would spin on.
    let (closer, close_rx) = ConnCloser::new();
    let new_conn = || NewWebConn {
        id: conn_id,
        addr: ip,
        tx: tx.clone(),
        closer: closer.clone(),
    };

    let handshake = if let Some(last_seq) = resume {
        match hooks.resume(new_conn(), &token, character, last_seq).await {
            Ok(ok) => Some(Handshake {
                replayed: true,
                replay: ok.replayed,
            }),
            Err(ResumeErr::TooOld) => {
                // The gap outgrew the ring: fall back to a fresh attach and
                // tell the client to re-hydrate (fresh snapshots follow
                // from the game side).
                match hooks.connect_character(new_conn(), &token, character).await {
                    Ok(_start) => Some(Handshake {
                        replayed: false,
                        replay: Vec::new(),
                    }),
                    Err(err) => {
                        close_attach_err(&mut socket, err).await;
                        return;
                    }
                }
            }
            Err(ResumeErr::Busy) => {
                close(&mut socket, close_code::SLOW, SERVER_BUSY).await;
                return;
            }
            Err(ResumeErr::Unavailable) => {
                close(&mut socket, close_code::SERVICE_RESTART, GAME_RESTARTING).await;
                return;
            }
            Err(ResumeErr::BadSession) => {
                close(&mut socket, close_code::BAD_SESSION, "session expired").await;
                return;
            }
        }
    } else {
        match hooks.connect_character(new_conn(), &token, character).await {
            Ok(_start) => None,
            Err(err) => {
                close_attach_err(&mut socket, err).await;
                return;
            }
        }
    };
    // Keep no sender of our own: `rx` must drain to None once the game
    // side drops the connection's handle.
    drop(tx);

    tracing::info!(conn = conn_id, %ip, resumed = resume.is_some(), "web connection attached");
    pump(&cfg, &*hooks, &mut socket, rx, close_rx, conn_id, handshake).await;
    drop(closer);
    // Attached connections always report exactly one disconnect.
    hooks.disconnected(conn_id).await;
    tracing::info!(conn = conn_id, "web connection closed");
}

/// The connection loop: hello/resume handshake, then fan outbound envelopes,
/// inbound commands, and keepalive through one `select!`.
async fn pump(
    cfg: &WebConfig,
    hooks: &dyn WebHooks,
    socket: &mut WebSocket,
    mut rx: mpsc::Receiver<Arc<Envelope>>,
    mut close_rx: watch::Receiver<Option<CloseRequest>>,
    conn_id: u64,
    handshake: Option<Handshake>,
) {
    let send_timeout = Duration::from_millis(cfg.send_timeout_ms.max(1));

    let hello = oob(OutputEvent::Hello {
        proto: PROTO_VERSION,
        server: cfg.server_name.clone(),
        features: cfg.features.clone(),
        panels: cfg.panels.clone(),
        asset_pack: cfg.asset_pack.clone(),
    });
    if !send_env(socket, &hello, send_timeout).await {
        return;
    }
    if let Some(h) = handshake {
        let resumed = oob(OutputEvent::Resumed {
            replayed: h.replayed,
        });
        if !send_env(socket, &resumed, send_timeout).await {
            return;
        }
        for env in &h.replay {
            if !send_env(socket, env, send_timeout).await {
                return;
            }
        }
    }

    let period = Duration::from_millis(cfg.ping_interval_ms.max(1));
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut last_nonce: u64 = 0;
    let mut outstanding_pings: u32 = 0;

    loop {
        tokio::select! {
            out = rx.recv() => match out {
                Some(env) => {
                    if !send_env(socket, &env, send_timeout).await {
                        tracing::info!(conn = conn_id, "closing slow/broken web client");
                        close(socket, close_code::SLOW, "too slow").await;
                        return;
                    }
                }
                None => {
                    // The game side dropped every handle: session closed,
                    // boot, or shutdown.
                    close(socket, close_code::GOING_AWAY, "session closed").await;
                    return;
                }
            },
            changed = close_rx.changed() => {
                // Doc 33 §4.5: the game side asked for a close with a code
                // of its own (a draining gateway's 1012). `Err` is every
                // sender gone, which the owner of this connection never
                // lets happen while the pump runs; it is not a close.
                let asked = changed.ok().and_then(|()| close_rx.borrow_and_update().clone());
                if let Some(req) = asked {
                    tracing::info!(conn = conn_id, code = req.code, "closing web client as the game side asked");
                    close(socket, req.code, &req.reason).await;
                    return;
                }
            }
            msg = socket.recv() => match msg {
                None => return, // transport gone
                Some(Err(err)) => {
                    // Includes tungstenite capacity errors: a frame over
                    // max_frame_bytes is a protocol error.
                    tracing::warn!(conn = conn_id, error = %err, "ws receive error");
                    close(socket, close_code::PROTOCOL, "protocol error").await;
                    return;
                }
                Some(Ok(Message::Close(_))) => return,
                Some(Ok(Message::Text(text))) => {
                    if mush_proto::validate_invoke_json(text.as_bytes()).is_err() {
                        close(socket, close_code::PROTOCOL, "oversized or deeply nested frame").await;
                        return;
                    }
                    match serde_json::from_str::<ClientCommand>(text.as_str()) {
                        Ok(ClientCommand::Line(line)) => hooks.line(conn_id, line).await,
                        Ok(ClientCommand::Invoke { id, request }) => hooks.invoke(conn_id, id, request).await,
                        Ok(ClientCommand::Req { id, q }) => hooks.request(conn_id, id, q).await,
                        Ok(ClientCommand::Ack { commit }) => {
                            hooks.acknowledge(conn_id, commit).await;
                        }
                        Ok(ClientCommand::Pong { nonce }) => {
                            if nonce != last_nonce {
                                tracing::debug!(conn = conn_id, nonce, last_nonce, "stale pong nonce");
                            }
                            outstanding_pings = 0;
                        }
                        Ok(ClientCommand::Caps(caps)) => {
                            // Ignored for now: web caps drive nothing server-side
                            // yet (rendering is client-side); logged for later.
                            tracing::debug!(conn = conn_id, ?caps, "client caps declared (ignored)");
                        }
                        Ok(ClientCommand::Picture { on, look_mdeg, kind }) => {
                            // Ignored (doc 28 P2): every web picture is rendered
                            // client-side by `mush-pixel-wasm`; kept on the wire
                            // as a no-op so an older bundle asking for a frame is
                            // not closed as a protocol error.
                            tracing::debug!(
                                conn = conn_id,
                                on,
                                look_mdeg,
                                ?kind,
                                "client picture request (ignored)"
                            );
                        }
                        Ok(ClientCommand::Composing { active }) => {
                            // A presence hint (interface plan I4, D-I-11): the
                            // binary rate-limits and fans it out; never an
                            // idle stamp, never a protocol error.
                            hooks.composing(conn_id, active).await;
                        }
                        Ok(ClientCommand::Respond { id, epoch, choice }) => {
                            // A server-initiated interaction reply (interface
                            // plan I10, D-I-21): the engine correlates it by
                            // id and applies the choice; a stale/forged id is
                            // ignored engine-side, never a protocol error.
                            hooks.respond(conn_id, id, epoch, choice).await;
                        }
                        Ok(ClientCommand::Resume { .. }) => {
                            tracing::warn!(conn = conn_id, "mid-connection resume is a protocol error");
                            close(socket, close_code::PROTOCOL, "resume is connection-setup only (?resume=)").await;
                            return;
                        }
                        Err(err) => {
                            tracing::warn!(conn = conn_id, error = %err, "malformed client frame");
                            close(socket, close_code::PROTOCOL, "malformed frame").await;
                            return;
                        }
                    }
                }
                Some(Ok(Message::Binary(_))) => {
                    tracing::warn!(conn = conn_id, "binary frame is a protocol error");
                    close(socket, close_code::PROTOCOL, "binary frames not supported").await;
                    return;
                }
                // WS-level Ping is answered by axum automatically; Pong is
                // uninteresting (keepalive is app-level).
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            },
            _ = ping.tick() => {
                if outstanding_pings >= cfg.max_missed_pongs {
                    tracing::info!(conn = conn_id, "keepalive timeout ({outstanding_pings} unanswered pings)");
                    close(socket, close_code::GOING_AWAY, "keepalive timeout").await;
                    return;
                }
                last_nonce = last_nonce.wrapping_add(1);
                outstanding_pings += 1;
                let ping_env = oob(OutputEvent::Ping { nonce: last_nonce });
                if !send_env(socket, &ping_env, send_timeout).await {
                    return;
                }
            }
        }
    }
}

/// Serializes and sends one envelope; `false` means the connection is done
/// for (send error or slow-client timeout).
async fn send_env(socket: &mut WebSocket, env: &Envelope, timeout: Duration) -> bool {
    let json = match serde_json::to_string(env) {
        Ok(json) => json,
        Err(err) => {
            tracing::error!(error = %err, "envelope serialization failed");
            return false;
        }
    };
    match tokio::time::timeout(timeout, socket.send(Message::Text(json.into()))).await {
        Ok(Ok(())) => true,
        Ok(Err(err)) => {
            tracing::debug!(error = %err, "ws send failed");
            false
        }
        Err(_elapsed) => false,
    }
}

/// Closes an upgraded socket for a refused fresh attach, spending 4401 —
/// the code that costs the client its stored token — on the one variant
/// that is actually about the credential. `Busy` and `Unavailable` get the
/// same retryable codes the resume arm gives the identical conditions: an
/// ordinary server-half reload, or a reconnect landing on full admission,
/// must not sign a player out.
async fn close_attach_err(socket: &mut WebSocket, err: AttachErr) {
    match err {
        AttachErr::Refused(reason) => {
            close(socket, close_code::BAD_SESSION, &reason).await;
        }
        AttachErr::Busy => close(socket, close_code::SLOW, SERVER_BUSY).await,
        AttachErr::Unavailable => {
            close(socket, close_code::SERVICE_RESTART, GAME_RESTARTING).await;
        }
    }
}

/// Best-effort close frame (bounded — a dead peer must not hold the task).
async fn close(socket: &mut WebSocket, code: u16, reason: &str) {
    let frame = CloseFrame {
        code,
        reason: reason.to_string().into(),
    };
    let _ = tokio::time::timeout(
        Duration::from_secs(1),
        socket.send(Message::Close(Some(frame))),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn parses_char_and_resume_ignoring_a_token_param() {
        // A `token=` pair is deliberately ignored (LOW finding: the query
        // fallback was removed) — it must not resurrect as a new field.
        let p = WsParams::parse("token=abc%2B1&char=7&resume=42");
        assert_eq!(p.char_key, Some(7));
        assert_eq!(p.resume, Some(42));
    }

    #[test]
    fn tolerates_missing_and_garbage() {
        let p = WsParams::parse("");
        assert_eq!(p, WsParams::default());
        let p = WsParams::parse("char=notanumber&resume=&other=x");
        assert_eq!(p.char_key, None);
        assert_eq!(p.resume, None);
    }

    #[test]
    fn percent_decode_handles_plus_and_truncated_escape() {
        assert_eq!(percent_decode("a+b%20c"), "a b c");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn subprotocol_token_extracts() {
        let mut h = HeaderMap::new();
        h.insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_static("rustmush.v1.tok123"),
        );
        assert_eq!(subprotocol_token(&h).as_deref(), Some("tok123"));

        let mut h = HeaderMap::new();
        h.insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_static("something-else, rustmush.v1.tok456"),
        );
        assert_eq!(subprotocol_token(&h).as_deref(), Some("tok456"));

        let mut h = HeaderMap::new();
        h.insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_static("rustmush.v1."),
        );
        assert_eq!(subprotocol_token(&h), None);
        assert_eq!(subprotocol_token(&HeaderMap::new()), None);
    }

    #[test]
    fn oob_envelopes_are_seq_zero() {
        let env = oob(OutputEvent::Ping { nonce: 9 });
        assert_eq!(env.seq, 0);
        assert!(env.ts > 0);
    }
}

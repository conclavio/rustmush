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
//! ## Keepalive & backpressure
//!
//! App-level `ping` every `ping_interval_ms` (proxies eat WS protocol
//! pings); a client that leaves `max_missed_pongs` pings unanswered is
//! closed as dead. Outbound envelopes come from a bounded queue; a socket
//! too slow to accept a frame within `send_timeout_ms` is closed (1013) —
//! the session layer's resume ring lets it recover the tail on reconnect.
//!
//! ## Abuse protection (HIGH finding: `/ws` previously had none)
//!
//! An invalid/expired token counts one [`WebHooks::auth_failed`] against the
//! peer, exactly like a failed `/api/auth/login` — so `IpGuard`'s tempban
//! engages on WS token probing. Every upgrade also claims a per-IP
//! concurrent-connection slot via [`WebHooks::connection_opened`] before the
//! handshake completes (over-cap is refused pre-upgrade) and releases it
//! via [`WebHooks::connection_closed`] exactly once, however the socket
//! task ends — see [`ConnSlot`].

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use mush_proto::{ClientCommand, Envelope, OutputEvent, PROTO_VERSION};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

use crate::app::AppState;
use crate::config::WebConfig;
use crate::hooks::{NewWebConn, ResumeErr, WebHooks};
use crate::ip::client_ip;

/// Subprotocol prefix carrying the auth token.
const SUBPROTO_PREFIX: &str = "rustmush.v1.";

/// Close codes used by this module.
mod close_code {
    /// RFC 6455 policy violation — protocol errors (mid-connection resume,
    /// malformed/binary/oversize frames).
    pub const PROTOCOL: u16 = 1008;
    /// RFC 6455 "try again later" — client too slow to keep up.
    pub const SLOW: u16 = 1013;
    /// Application: token/session no longer valid at attach time.
    pub const BAD_SESSION: u16 = 4401;
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
    if state.hooks.validate_token(&token).await.is_none() {
        // Token probing over /ws counts like any other auth failure, so
        // IpGuard's tempban engages here too (HIGH finding).
        state.hooks.auth_failed(ip);
        return (StatusCode::UNAUTHORIZED, "invalid token").into_response();
    }

    let cap = state.config.max_frame_bytes;
    let mut ws = ws.max_message_size(cap).max_frame_size(cap);
    ws = ws.protocols([format!("{SUBPROTO_PREFIX}{token}")]);
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
    let new_conn = || NewWebConn {
        id: conn_id,
        addr: ip,
        tx: tx.clone(),
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
                    Err(reason) => {
                        close(&mut socket, close_code::BAD_SESSION, &reason).await;
                        return;
                    }
                }
            }
            Err(ResumeErr::BadSession) => {
                close(&mut socket, close_code::BAD_SESSION, "session expired").await;
                return;
            }
        }
    } else {
        match hooks.connect_character(new_conn(), &token, character).await {
            Ok(_start) => None,
            Err(reason) => {
                close(&mut socket, close_code::BAD_SESSION, &reason).await;
                return;
            }
        }
    };
    // Keep no sender of our own: `rx` must drain to None once the game
    // side drops the connection's handle.
    drop(tx);

    tracing::info!(conn = conn_id, %ip, resumed = resume.is_some(), "web connection attached");
    pump(&cfg, &*hooks, &mut socket, rx, conn_id, handshake).await;
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
    conn_id: u64,
    handshake: Option<Handshake>,
) {
    let send_timeout = Duration::from_millis(cfg.send_timeout_ms.max(1));

    let hello = oob(OutputEvent::Hello {
        proto: PROTO_VERSION,
        server: cfg.server_name.clone(),
        features: cfg.features.clone(),
        panels: cfg.panels.clone(),
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
                    match serde_json::from_str::<ClientCommand>(text.as_str()) {
                        Ok(ClientCommand::Line(line)) => hooks.line(conn_id, line).await,
                        Ok(ClientCommand::Req { id, q }) => hooks.request(conn_id, id, q).await,
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

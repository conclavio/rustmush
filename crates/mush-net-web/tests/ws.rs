//! WebSocket integration tests: real sockets against a bound ephemeral
//! port, tokio-tungstenite as the client, [`MockWebHooks`] as the game.

mod common;

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use common::{CHAR_KEY, MockWebHooks, TOKEN, spawn_server, sys_env, test_config};
use futures::{SinkExt, StreamExt};
use mush_net_web::{AttachErr, ResumeErr, ResumeOk, WEB_CONN_ID_BASE, WebConfig};
use mush_proto::{ClientCommand, Envelope, OutputEvent, ProjectedCommitToken, Query};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::error::ProtocolError;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

const SUBPROTO: &str = "rustmush.v1.tok-nick";

/// Connects with the token in the `Sec-WebSocket-Protocol` subprotocol.
async fn connect_ws(
    addr: std::net::SocketAddr,
    query: &str,
) -> Result<
    (
        Ws,
        tokio_tungstenite::tungstenite::handshake::client::Response,
    ),
    WsError,
> {
    let mut req = format!("ws://{addr}/ws?{query}")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("sec-websocket-protocol", HeaderValue::from_static(SUBPROTO));
    connect_async(req).await
}

/// Next JSON envelope (skips WS-level control frames).
async fn recv_env(ws: &mut Ws) -> Envelope {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .expect("timed out waiting for a frame")
            .expect("stream ended")
            .expect("ws error");
        match msg {
            Message::Text(text) => return serde_json::from_str(text.as_str()).unwrap(),
            Message::Close(frame) => panic!("unexpected close: {frame:?}"),
            _ => {}
        }
    }
}

/// Reads until the server's close frame; returns `(code, reason)`.
async fn recv_close(ws: &mut Ws) -> (u16, String) {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .expect("timed out waiting for close");
        match msg {
            Some(Ok(Message::Close(Some(frame)))) => {
                return (frame.code.into(), frame.reason.to_string());
            }
            Some(Ok(_)) => {}
            other => panic!("stream ended without a close frame: {other:?}"),
        }
    }
}

async fn send_cmd(ws: &mut Ws, cmd: &ClientCommand) {
    ws.send(Message::Text(serde_json::to_string(cmd).unwrap().into()))
        .await
        .unwrap();
}

fn expect_hello(env: &Envelope) {
    assert_eq!(env.seq, 0, "hello is out-of-band (seq 0)");
    match &env.event {
        OutputEvent::Hello {
            proto,
            server,
            features,
            panels,
        } => {
            assert_eq!(*proto, mush_proto::PROTO_VERSION);
            assert!(server.starts_with("rustmush"));
            assert_eq!(features, &["mail".to_string(), "channels".to_string()]);
            assert_eq!(panels, &["wod.sheet".to_string()]);
        }
        other => panic!("expected hello, got {other:?}"),
    }
}

#[tokio::test]
async fn hello_is_first_frame_and_subprotocol_is_echoed() {
    let hooks = Arc::new(MockWebHooks::new());
    let addr = spawn_server(hooks.clone(), test_config()).await;

    let (mut ws, resp) = connect_ws(addr, "char=7").await.unwrap();
    assert_eq!(
        resp.headers()
            .get("sec-websocket-protocol")
            .and_then(|v| v.to_str().ok()),
        Some(SUBPROTO),
        "browsers require the offered subprotocol to be selected"
    );

    expect_hello(&recv_env(&mut ws).await);

    // The attach carried our token, character, and a conn id from the web
    // id range.
    let st = hooks.state();
    assert_eq!(st.connects.len(), 1);
    let (conn, token, character) = st.connects[0].clone();
    assert!(conn >= WEB_CONN_ID_BASE);
    assert_eq!(token, TOKEN);
    assert_eq!(character, CHAR_KEY);
}

#[tokio::test]
async fn token_via_query_is_no_longer_accepted() {
    // LOW finding: the `?token=` fallback was removed (proxies log full
    // URIs by default). No subprotocol header at all: the query-string
    // token is silently ignored, so the upgrade fails as if no token was
    // presented at all.
    let hooks = Arc::new(MockWebHooks::new());
    let addr = spawn_server(hooks, test_config()).await;

    let err = connect_async(format!("ws://{addr}/ws?token={TOKEN}&char=7"))
        .await
        .unwrap_err();
    assert_http_status(err, 401);
}

#[tokio::test]
async fn line_req_and_receipt_ack_reach_the_hooks() {
    let hooks = Arc::new(MockWebHooks::new());
    let addr = spawn_server(hooks.clone(), test_config()).await;
    let (mut ws, _) = connect_ws(addr, "char=7").await.unwrap();
    expect_hello(&recv_env(&mut ws).await);

    send_cmd(&mut ws, &ClientCommand::Line("look".into())).await;
    send_cmd(
        &mut ws,
        &ClientCommand::Req {
            id: 11,
            q: Query::Who,
        },
    )
    .await;
    let commit = ProjectedCommitToken::new("0123456789abcdef0123456789abcdef").unwrap();
    send_cmd(
        &mut ws,
        &ClientCommand::Ack {
            commit: commit.clone(),
        },
    )
    .await;

    hooks
        .wait_for("line + request + acknowledgement forwarded", |st| {
            st.lines.len() == 1 && st.requests.len() == 1 && st.acknowledgements.len() == 1
        })
        .await;
    let st = hooks.state();
    assert_eq!(st.lines[0].1, "look");
    let (_, id, ref q) = st.requests[0];
    assert_eq!(id, 11);
    assert_eq!(*q, Query::Who);
    assert_eq!(st.acknowledgements[0].1, commit);
}

#[tokio::test]
async fn live_envelopes_flow_to_the_client() {
    let hooks = Arc::new(MockWebHooks::new());
    let addr = spawn_server(hooks.clone(), test_config()).await;
    let (mut ws, _) = connect_ws(addr, "char=7").await.unwrap();
    expect_hello(&recv_env(&mut ws).await);

    // The game side (mock) pushes a stamped envelope down the conn's tx.
    let tx = {
        let st = hooks.state();
        st.conns.values().next().unwrap().clone()
    };
    tx.send(sys_env(1, "the game speaks")).await.unwrap();

    let env = recv_env(&mut ws).await;
    assert_eq!(env.seq, 1);
    match env.event {
        OutputEvent::Sys(ref text) => assert_eq!(text.to_plain_text(), "the game speaks"),
        ref other => panic!("expected sys, got {other:?}"),
    }
}

#[tokio::test]
async fn resume_replays_the_gap_in_order() {
    let hooks = Arc::new(MockWebHooks::new());
    hooks.state().resume_result = Some(Ok(ResumeOk {
        replayed: vec![sys_env(42, "missed one"), sys_env(43, "missed two")],
        live_from: 44,
    }));
    let addr = spawn_server(hooks.clone(), test_config()).await;

    let (mut ws, _) = connect_ws(addr, "char=7&resume=41").await.unwrap();
    expect_hello(&recv_env(&mut ws).await);

    let resumed = recv_env(&mut ws).await;
    assert_eq!(resumed.seq, 0);
    assert_eq!(
        resumed.event,
        OutputEvent::Resumed { replayed: true },
        "resumed must announce the replay before the gap"
    );

    let a = recv_env(&mut ws).await;
    let b = recv_env(&mut ws).await;
    assert_eq!((a.seq, b.seq), (42, 43));

    let st = hooks.state();
    assert_eq!(st.resumes.len(), 1);
    let (_, ref token, character, last_seq) = st.resumes[0];
    assert_eq!(token, TOKEN);
    assert_eq!(character, CHAR_KEY);
    assert_eq!(last_seq, 41);
    assert!(st.connects.is_empty(), "no fresh attach on a clean resume");
}

#[tokio::test]
async fn resume_too_old_falls_back_to_fresh_attach() {
    let hooks = Arc::new(MockWebHooks::new());
    hooks.state().resume_result = Some(Err(ResumeErr::TooOld));
    let addr = spawn_server(hooks.clone(), test_config()).await;

    let (mut ws, _) = connect_ws(addr, "char=7&resume=1").await.unwrap();
    expect_hello(&recv_env(&mut ws).await);

    let resumed = recv_env(&mut ws).await;
    assert_eq!(
        resumed.event,
        OutputEvent::Resumed { replayed: false },
        "too-old gap: the client must re-hydrate from the snapshots that follow"
    );
    let st = hooks.state();
    assert_eq!(st.resumes.len(), 1, "resume was attempted");
    assert_eq!(st.connects.len(), 1, "then fell back to connect_character");
}

#[tokio::test]
async fn resume_bad_session_closes_4401_without_attach() {
    let hooks = Arc::new(MockWebHooks::new());
    // Default mock resume behavior is BadSession.
    let addr = spawn_server(hooks.clone(), test_config()).await;

    let (mut ws, _) = connect_ws(addr, "char=7&resume=5").await.unwrap();
    let (code, _reason) = recv_close(&mut ws).await;
    assert_eq!(code, 4401);

    tokio::time::sleep(Duration::from_millis(50)).await;
    let st = hooks.state();
    assert!(
        st.disconnects.is_empty(),
        "never attached -> no disconnected callback"
    );
}

#[tokio::test]
async fn resume_busy_closes_1013_without_fresh_attach() {
    let hooks = Arc::new(MockWebHooks::new());
    hooks.state().resume_result = Some(Err(ResumeErr::Busy));
    let addr = spawn_server(hooks.clone(), test_config()).await;

    let (mut ws, _) = connect_ws(addr, "char=7&resume=5").await.unwrap();
    let (code, reason) = recv_close(&mut ws).await;
    assert_eq!(code, 1013, "temporary admission pressure is retryable");
    assert_eq!(reason, "server busy; try again");

    tokio::time::sleep(Duration::from_millis(50)).await;
    let st = hooks.state();
    assert_eq!(st.resumes.len(), 1, "resume was attempted exactly once");
    assert!(
        st.connects.is_empty(),
        "Busy must not fall back to a fresh attach and bypass resume admission"
    );
    assert!(
        st.disconnects.is_empty(),
        "never attached -> no disconnected callback"
    );
}

#[tokio::test]
async fn resume_unavailable_closes_1012_without_fresh_attach() {
    let hooks = Arc::new(MockWebHooks::new());
    hooks.state().resume_result = Some(Err(ResumeErr::Unavailable));
    let addr = spawn_server(hooks.clone(), test_config()).await;

    let (mut ws, _) = connect_ws(addr, "char=7&resume=5").await.unwrap();
    let (code, reason) = recv_close(&mut ws).await;
    assert_eq!(code, 1012, "boundary loss is a service-restart condition");
    assert_eq!(reason, "game restarting; reconnect shortly");

    tokio::time::sleep(Duration::from_millis(50)).await;
    let st = hooks.state();
    assert_eq!(st.resumes.len(), 1, "resume was attempted exactly once");
    assert!(
        st.connects.is_empty(),
        "Unavailable must not fall back to a fresh attach during the same outage"
    );
    assert!(
        st.disconnects.is_empty(),
        "never attached -> no disconnected callback"
    );
}

#[tokio::test]
async fn connect_refusal_closes_4401_with_reason() {
    let hooks = Arc::new(MockWebHooks::new());
    hooks.state().connect_err = Some(AttachErr::Refused("That character is unavailable.".into()));
    let addr = spawn_server(hooks.clone(), test_config()).await;

    let (mut ws, _) = connect_ws(addr, "char=7").await.unwrap();
    let (code, reason) = recv_close(&mut ws).await;
    assert_eq!(code, 4401);
    assert_eq!(reason, "That character is unavailable.");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(hooks.state().disconnects.is_empty());
}

/// A transient attach refusal is not a dead credential. 4401 costs the
/// client its stored token, so the two conditions that mean "not now" — full
/// admission, and the server half gone mid-setup, which is what an ordinary
/// split `reload` looks like from here — must arrive as the same retryable
/// codes the resume arm has always used, on BOTH attach arms (fresh, and the
/// too-old resume fallback).
#[tokio::test]
async fn a_busy_or_restarting_game_never_spends_the_fatal_close() {
    for (err, expected_code, expected_reason) in [
        (AttachErr::Busy, 1013, "server busy; try again"),
        (
            AttachErr::Unavailable,
            1012,
            "game restarting; reconnect shortly",
        ),
    ] {
        // The fresh-attach arm.
        let hooks = Arc::new(MockWebHooks::new());
        hooks.state().connect_err = Some(err.clone());
        let addr = spawn_server(hooks.clone(), test_config()).await;
        let (mut ws, _) = connect_ws(addr, "char=7").await.unwrap();
        let (code, reason) = recv_close(&mut ws).await;
        assert_eq!(code, expected_code, "{err:?} is retryable, not fatal");
        assert_eq!(reason, expected_reason);

        // The too-old-resume fallback arm reaches the same hook and must
        // answer the same way.
        let hooks = Arc::new(MockWebHooks::new());
        {
            let mut st = hooks.state();
            st.resume_result = Some(Err(ResumeErr::TooOld));
            st.connect_err = Some(err.clone());
        }
        let addr = spawn_server(hooks.clone(), test_config()).await;
        let (mut ws, _) = connect_ws(addr, "char=7&resume=1").await.unwrap();
        let (code, reason) = recv_close(&mut ws).await;
        assert_eq!(code, expected_code);
        assert_eq!(reason, expected_reason);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(hooks.state().disconnects.is_empty(), "never attached");
    }
}

#[tokio::test]
async fn mid_connection_resume_is_a_protocol_error() {
    let hooks = Arc::new(MockWebHooks::new());
    let addr = spawn_server(hooks.clone(), test_config()).await;
    let (mut ws, _) = connect_ws(addr, "char=7").await.unwrap();
    expect_hello(&recv_env(&mut ws).await);

    send_cmd(&mut ws, &ClientCommand::Resume { last_seq: 5 }).await;
    let (code, _) = recv_close(&mut ws).await;
    assert_eq!(code, 1008);
    hooks
        .wait_for("disconnected after protocol error", |st| {
            st.disconnects.len() == 1
        })
        .await;
}

#[tokio::test]
async fn oversize_frame_closes_the_connection() {
    let hooks = Arc::new(MockWebHooks::new());
    let config = test_config();
    let cap = config.max_frame_bytes;
    let addr = spawn_server(hooks.clone(), config).await;
    let (mut ws, _) = connect_ws(addr, "char=7").await.unwrap();
    expect_hello(&recv_env(&mut ws).await);

    // Exceed the configured transport cap even when authored-source support
    // raises it above the ordinary line-command limit.
    let big = format!("{{\"t\":\"line\",\"d\":\"{}\"}}", "x".repeat(cap + 1));
    ws.send(Message::Text(big.into())).await.unwrap();

    // The server's frame-size guard rejects this from the frame header,
    // before consuming the declared payload. Its 1008 close is therefore
    // best-effort: on Windows, dropping a TCP socket with unread inbound
    // bytes can surface to the peer as WSAECONNRESET before the close frame.
    // Either outcome proves the transport was closed; when the close frame
    // wins the race, its protocol code must still be correct.
    let close = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Close(Some(frame)))) => {
                    break Some(u16::from(frame.code));
                }
                Some(Ok(Message::Close(None))) => panic!("server close omitted the protocol code"),
                Some(Err(WsError::Io(err)))
                    if matches!(
                        err.kind(),
                        std::io::ErrorKind::BrokenPipe
                            | std::io::ErrorKind::ConnectionAborted
                            | std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::UnexpectedEof
                    ) =>
                {
                    break None;
                }
                Some(Err(WsError::Protocol(ProtocolError::ResetWithoutClosingHandshake)))
                | Some(Err(WsError::ConnectionClosed))
                | None => break None,
                Some(Err(err)) => panic!("unexpected websocket error during teardown: {err}"),
                Some(Ok(_)) => {}
            }
        }
    })
    .await
    .expect("timed out waiting for oversize connection teardown");
    if let Some(code) = close {
        assert_eq!(code, 1008);
    }
    hooks
        .wait_for("disconnected after oversize", |st| {
            st.disconnects.len() == 1
        })
        .await;
    assert!(
        hooks.state().lines.is_empty(),
        "the line never went through"
    );
}

#[tokio::test]
async fn binary_and_malformed_frames_close_1008() {
    for payload in [
        Message::Binary(vec![1u8, 2, 3].into()),
        Message::Text("{not json".to_string().into()),
    ] {
        let hooks = Arc::new(MockWebHooks::new());
        let addr = spawn_server(hooks.clone(), test_config()).await;
        let (mut ws, _) = connect_ws(addr, "char=7").await.unwrap();
        expect_hello(&recv_env(&mut ws).await);

        ws.send(payload).await.unwrap();
        let (code, _) = recv_close(&mut ws).await;
        assert_eq!(code, 1008);
    }
}

#[tokio::test]
async fn unanswered_pings_close_the_connection() {
    let hooks = Arc::new(MockWebHooks::new());
    let config = WebConfig {
        ping_interval_ms: 60,
        max_missed_pongs: 2,
        ..test_config()
    };
    let addr = spawn_server(hooks.clone(), config).await;
    let (mut ws, _) = connect_ws(addr, "char=7").await.unwrap();
    expect_hello(&recv_env(&mut ws).await);

    let mut pings = 0u32;
    let code = loop {
        let msg = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .expect("timed out")
            .expect("stream ended")
            .expect("ws error");
        match msg {
            Message::Text(text) => {
                let env: Envelope = serde_json::from_str(text.as_str()).unwrap();
                if let OutputEvent::Ping { .. } = env.event {
                    assert_eq!(env.seq, 0, "pings are out-of-band");
                    pings += 1;
                }
            }
            Message::Close(Some(frame)) => break u16::from(frame.code),
            _ => {}
        }
    };
    assert_eq!(code, 1001);
    assert_eq!(pings, 2, "two pings went unanswered before the cutoff");
    hooks
        .wait_for("disconnected after keepalive timeout", |st| {
            st.disconnects.len() == 1
        })
        .await;
}

#[tokio::test]
async fn answered_pings_keep_the_connection_alive() {
    let hooks = Arc::new(MockWebHooks::new());
    let config = WebConfig {
        ping_interval_ms: 40,
        max_missed_pongs: 2,
        ..test_config()
    };
    let addr = spawn_server(hooks.clone(), config).await;
    let (mut ws, _) = connect_ws(addr, "char=7").await.unwrap();
    expect_hello(&recv_env(&mut ws).await);

    // Answer every ping for ~10 intervals — far past the 2-missed cutoff.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(400);
    while tokio::time::Instant::now() < deadline {
        let Ok(msg) = tokio::time::timeout(Duration::from_millis(60), ws.next()).await else {
            continue;
        };
        let msg = msg.expect("stream ended").expect("ws error");
        if let Message::Text(text) = msg {
            let env: Envelope = serde_json::from_str(text.as_str()).unwrap();
            if let OutputEvent::Ping { nonce } = env.event {
                send_cmd(&mut ws, &ClientCommand::Pong { nonce }).await;
            }
        } else if let Message::Close(frame) = msg {
            panic!("connection closed despite pongs: {frame:?}");
        }
    }

    // Still alive: a line still reaches the game.
    send_cmd(&mut ws, &ClientCommand::Line("still here".into())).await;
    hooks
        .wait_for("line after keepalive exchange", |st| !st.lines.is_empty())
        .await;
}

#[tokio::test]
async fn game_dropping_the_conn_closes_the_socket() {
    let hooks = Arc::new(MockWebHooks::new());
    let addr = spawn_server(hooks.clone(), test_config()).await;
    let (mut ws, _) = connect_ws(addr, "char=7").await.unwrap();
    expect_hello(&recv_env(&mut ws).await);

    // The game side drops its sender (boot / session close).
    hooks.state().conns.clear();

    let (code, reason) = recv_close(&mut ws).await;
    assert_eq!(code, 1001);
    assert_eq!(reason, "session closed");
}

#[tokio::test]
async fn client_disconnect_reports_disconnected_once() {
    let hooks = Arc::new(MockWebHooks::new());
    let addr = spawn_server(hooks.clone(), test_config()).await;
    let (mut ws, _) = connect_ws(addr, "char=7").await.unwrap();
    expect_hello(&recv_env(&mut ws).await);

    ws.close(None).await.unwrap();
    hooks
        .wait_for("disconnected after client close", |st| {
            st.disconnects.len() == 1
        })
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(hooks.state().disconnects.len(), 1, "exactly once");
}

#[tokio::test]
async fn ws_rejects_bad_token_blocked_ip_and_missing_char() {
    // Invalid token, present via the subprotocol: the upgrade COMPLETES and
    // the server closes 4401. A pre-upgrade 401 reaches a browser as an
    // opaque 1006 it cannot tell from a dropped network, so the client kept
    // retrying the dead token — which is how an open tab banned its own
    // address (dev1, 2026-09-20). And the refusal counts on `token_refused`,
    // never on the tempban counter.
    let hooks = Arc::new(MockWebHooks::new());
    let addr = spawn_server(hooks.clone(), test_config()).await;
    let mut req = format!("ws://{addr}/ws?char=7")
        .into_client_request()
        .unwrap();
    req.headers_mut().insert(
        "sec-websocket-protocol",
        HeaderValue::from_static("rustmush.v1.wrong-token"),
    );
    let (mut ws, _resp) = connect_async(req).await.expect("upgrade completes");
    let (code, reason) = recv_close(&mut ws).await;
    assert_eq!(code, 4401, "the client must be able to recognise this");
    assert_eq!(reason, mush_net_web::SESSION_ENDED);
    {
        let st = hooks.state();
        assert!(
            st.auth_failed.is_empty(),
            "a refused token must never feed the counter that tempbans"
        );
        assert_eq!(st.token_refused, vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]);
        assert!(st.connects.is_empty(), "never attached");
        assert!(st.disconnects.is_empty(), "never attached -> no disconnect");
    }

    // Missing token entirely -> 401, but NOT an auth failure (nothing was
    // ever presented to validate).
    let hooks = Arc::new(MockWebHooks::new());
    let addr = spawn_server(hooks.clone(), test_config()).await;
    let err = connect_async(format!("ws://{addr}/ws?char=7"))
        .await
        .unwrap_err();
    assert_http_status(err, 401);
    assert!(hooks.state().auth_failed.is_empty());

    // Missing char -> 400.
    let hooks = Arc::new(MockWebHooks::new());
    let addr = spawn_server(hooks, test_config()).await;
    let err = connect_ws(addr, "nochar=1").await.unwrap_err();
    assert_http_status(err, 400);

    // Blocked IP -> 403 pre-upgrade, token never validated.
    let hooks = Arc::new(MockWebHooks::new());
    hooks
        .state()
        .blocked
        .insert(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let addr = spawn_server(hooks.clone(), test_config()).await;
    let err = connect_ws(addr, "char=7").await.unwrap_err();
    assert_http_status(err, 403);
    assert!(hooks.state().connects.is_empty());
}

/// A store that cannot answer must never be reported as a dead session.
/// The client answers 4401 by DELETING its token, so folding "the store is
/// down" into "your token is refused" would sign every open tab out — each
/// holding a valid, unrevoked 7-day credential — the moment the instance
/// Redis blinked during a deploy. The upgrade is still refused (fail
/// closed); it is refused transiently, and counted nowhere.
#[tokio::test]
async fn a_store_that_cannot_answer_is_transient_not_a_dead_session() {
    let hooks = Arc::new(MockWebHooks::new());
    hooks.state().token_store_down = true;
    let addr = spawn_server(hooks.clone(), test_config()).await;

    // The token here is the mock's GOOD one; only the store is broken.
    let err = connect_ws(addr, "char=7").await.unwrap_err();
    assert_http_status(err, 503);

    let st = hooks.state();
    assert!(st.connects.is_empty(), "fail closed: nothing was attached");
    assert!(
        st.token_refused.is_empty(),
        "an outage must not arm the cool-off against the players waiting it out"
    );
    assert!(st.auth_failed.is_empty(), "and never the tempban counter");
}

/// The refused-token cool-off: browser-visible 4429 before token lookup. The route must not confuse this with the
/// tempban (403) or with a dead token (4401) — it is the one genuinely
/// transient refusal of the three, so the client is right to back off.
#[tokio::test]
async fn ws_honours_the_refused_token_cooloff_without_touching_the_token() {
    let hooks = Arc::new(MockWebHooks::new());
    hooks
        .state()
        .token_throttled
        .insert(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let addr = spawn_server(hooks.clone(), test_config()).await;

    // Even a PERFECTLY GOOD token is refused while the cool-off runs.
    let (mut ws, _) = connect_ws(addr, "char=7").await.expect("cool-off upgrades");
    let (code, reason) = recv_close(&mut ws).await;
    assert_eq!(code, 4429);
    assert_eq!(reason, mush_net_web::TOKEN_COOLOFF);
    hooks
        .wait_for("cool-off slot released", |st| st.live_conns == 0)
        .await;
    let st = hooks.state();
    assert!(st.connects.is_empty());
    assert_eq!(st.conn_opens.len(), 1, "the upgrade is bounded by a slot");
    assert_eq!(st.token_checks, 0, "cool-off never queries the token store");
    assert!(
        st.token_refused.is_empty(),
        "a refusal the cool-off itself caused must not extend the cool-off"
    );
    assert!(st.auth_failed.is_empty(), "and never a credential failure");
}

/// HIGH finding: `/ws` previously had no per-IP concurrent-connection cap.
#[tokio::test]
async fn ws_enforces_the_per_ip_connection_cap() {
    let hooks = Arc::new(MockWebHooks::new());
    hooks.state().conn_cap = Some(1);
    let addr = spawn_server(hooks.clone(), test_config()).await;

    let (mut ws1, _) = connect_ws(addr, "char=7")
        .await
        .expect("first connection fits the cap");
    expect_hello(&recv_env(&mut ws1).await);

    // A second concurrent connection from the same IP is over the cap —
    // refused before the upgrade, so the game side never sees an attach
    // attempt for it.
    let err = connect_ws(addr, "char=7").await.unwrap_err();
    assert_http_status(err, 429);
    assert_eq!(hooks.state().connects.len(), 1, "only the first attached");

    // Closing the first connection releases its slot for a new one.
    ws1.close(None).await.unwrap();
    hooks
        .wait_for("slot released", |st| st.live_conns == 0)
        .await;
    let (mut ws2, _) = connect_ws(addr, "char=7")
        .await
        .expect("slot freed after the first connection closed");
    expect_hello(&recv_env(&mut ws2).await);
}

fn assert_http_status(err: WsError, expected: u16) {
    match err {
        WsError::Http(resp) => assert_eq!(resp.status().as_u16(), expected),
        other => panic!("expected HTTP {expected}, got {other:?}"),
    }
}

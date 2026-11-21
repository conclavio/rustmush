//! M16d (W1): the split gateway serves the WEB client.
//!
//! `mush-server` (world task + persistence + modules, the game sheet composed) and
//! `mush-gateway` (SSH listener + WEB listener + session layer + accounts) run
//! as two independent tokio runtimes on their own OS threads, joined by a real
//! localhost TCP boundary speaking the `mush-ipc` protocol. A real WebSocket
//! client then:
//!
//! - registers an account over the gateway's REST surface (which reserves it in
//!   the gateway store and creates the Player over the `CreatePlayer`
//!   `ServiceReq` across the boundary),
//! - opens a WS through the gateway and receives the out-of-band `hello` frame
//!   whose `features`/`panels` are the SAME manifest the all-in-one advertises
//!   (who / channels / mail / `<ruleset>.sheet`), delivered over IPC from the
//!   server's boot Hello — NOT empty, and
//! - plays: a `say` typed on the WS round-trips through the world loop on the
//!   server process and echoes back to the same WS connection.
//!
//! This is the W1 proof (web serves in split); the restart-survival proof (W2)
//! is a separate test.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_proto::{ClientCommand, Envelope, OutputEvent};
use mush_store::{InMemoryStore, ObjectStore};
use mush_testkit::text_of;
use serde_json::{Value, json};

use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use mush_config::config::Config;
use mush_gateway::GatewayOptions;

const STEP_TIMEOUT: Duration = Duration::from_secs(60);

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mush-websplit-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The SERVER config: in-memory, ephemeral IPC, the game sheet composed so the boot
/// Hello manifest carries the `<ruleset>.sheet` feed + `pilot` feature.
fn server_config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ssh.listen = "127.0.0.1:0".to_string();
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    cfg
}

/// The GATEWAY config: in-memory, ephemeral SSH + WEB listeners (the web table
/// binds `127.0.0.1:0`; `web_ready` reports the OS-assigned port). `auth_burst`
/// 0 disables the auth rate limiter for the test's register/login.
fn gateway_config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ssh.listen = "127.0.0.1:0".to_string();
    let mut web = toml::value::Table::new();
    web.insert(
        "listen".to_string(),
        toml::Value::String("127.0.0.1:0".to_string()),
    );
    web.insert("auth_burst".to_string(), toml::Value::Integer(0));
    cfg.web = toml::Value::Table(web);
    cfg
}

/// Boot a server + gateway pair on their own runtimes (as the binaries do) and
/// return the bound SSH + WEB addresses plus the thread handles.
async fn boot_split_web(
    gw_store: Arc<dyn ObjectStore>,
) -> (
    SocketAddr,
    SocketAddr,
    std::thread::JoinHandle<()>,
    std::thread::JoinHandle<()>,
) {
    let (ipc_tx, ipc_rx) = tokio::sync::oneshot::channel();
    let server_cfg = server_config("server");
    let server_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(mush_server::serve(server_cfg, Some(ipc_tx)))
            .unwrap();
    });
    let endpoint = tokio::time::timeout(Duration::from_secs(30), ipc_rx)
        .await
        .expect("server bound its IPC socket")
        .expect("ipc endpoint");

    let (ssh_tx, ssh_rx) = tokio::sync::oneshot::channel();
    let (web_tx, web_rx) = tokio::sync::oneshot::channel();
    let gateway_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut gw_cfg = gateway_config("gateway");
        gw_cfg.ipc.listen = endpoint.describe();
        rt.block_on(mush_gateway::serve(
            gw_cfg,
            GatewayOptions {
                store: Some(gw_store),
                ssh_ready: Some(ssh_tx),
                web_ready: Some(web_tx),
                siteban_refresh: None,
                give_up_after: None,
                generation: None,
                ..Default::default()
            },
        ))
        .unwrap();
    });
    let ssh_addr = tokio::time::timeout(Duration::from_secs(30), ssh_rx)
        .await
        .expect("gateway bound SSH")
        .expect("ssh addr");
    let web_addr = tokio::time::timeout(Duration::from_secs(30), web_rx)
        .await
        .expect("gateway bound WEB")
        .expect("web addr");
    (ssh_addr, web_addr, server_thread, gateway_thread)
}

async fn post_json(url: &str, body: Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(url)
        .json(&body)
        .send()
        .await
        .unwrap()
}

/// Register an account + first character over the gateway's REST surface;
/// returns (token, char key).
async fn register(
    web: SocketAddr,
    account: &str,
    password: &str,
    char_name: &str,
) -> (String, u64) {
    let url = format!("http://{web}/api/auth/register");
    let resp = post_json(
        &url,
        json!({"account": account, "password": password, "char_name": char_name}),
    )
    .await;
    assert_eq!(resp.status(), 200, "register must succeed across the split");
    let body: Value = resp.json().await.unwrap();
    (
        body["token"].as_str().unwrap().to_string(),
        body["characters"][0]["key"].as_u64().unwrap(),
    )
}

/// Open a WS with the token in the `Sec-WebSocket-Protocol` subprotocol (the
/// SPA's path) and `?char=<key>`.
async fn connect_ws(web: SocketAddr, token: &str, char_key: u64) -> Result<Ws, WsError> {
    let mut req = format!("ws://{web}/ws?char={char_key}")
        .into_client_request()
        .unwrap();
    req.headers_mut().insert(
        "sec-websocket-protocol",
        HeaderValue::from_str(&format!("rustmush.v1.{token}")).unwrap(),
    );
    connect_async(req).await.map(|(ws, _resp)| ws)
}

/// Next JSON envelope (skips WS control frames); panics on close.
async fn recv_env(ws: &mut Ws) -> Envelope {
    loop {
        let msg = tokio::time::timeout(STEP_TIMEOUT, ws.next())
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

/// Read envelopes until one's rendered text contains `needle`.
async fn expect_env(ws: &mut Ws, needle: &str) -> Envelope {
    let mut seen = Vec::new();
    loop {
        let env = recv_env(ws).await;
        let text = text_of(&env.event);
        if text.contains(needle) {
            return env;
        }
        seen.push(text);
        assert!(
            seen.len() < 200,
            "no envelope containing {needle:?} in {seen:?}"
        );
    }
}

/// Expect the out-of-band `hello` (seq 0) as the first frame; return
/// (features, panels).
async fn expect_hello(ws: &mut Ws) -> (Vec<String>, Vec<String>) {
    let env = recv_env(ws).await;
    assert_eq!(env.seq, 0, "hello is out-of-band");
    match env.event {
        OutputEvent::Hello {
            features, panels, ..
        } => (features, panels),
        other => panic!("expected hello first, got {other:?}"),
    }
}

async fn send_line(ws: &mut Ws, line: &str) {
    let cmd = ClientCommand::Line(line.to_string());
    ws.send(Message::Text(serde_json::to_string(&cmd).unwrap().into()))
        .await
        .unwrap();
}

/// W1: a WS client connects THROUGH the split gateway, authenticates with a
/// token, receives the correct NON-EMPTY hello manifest (the same features /
/// panels the all-in-one advertises — who / channels / mail / `<ruleset>.sheet`
/// — carried from the server's boot Hello over IPC), and plays (a command
/// round-trips over the split boundary).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn web_client_serves_through_split_gateway() {
    let gw_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let (_ssh_addr, web_addr, server_thread, gateway_thread) =
        boot_split_web(Arc::clone(&gw_store)).await;

    // Register + connect a WS through the gateway.
    let (token, key) = register(web_addr, "nick", "hunter42", "Nick").await;
    let mut ws = connect_ws(web_addr, &token, key)
        .await
        .expect("ws connect through the split gateway");

    // The hello manifest is NON-EMPTY and matches the all-in-one's surface: the
    // per-module features (comsys→channels, mail) and panel feeds (who, mail,
    // comsys), plus the game sheet's feature + feed — proving the server
    // shipped its real manifest over IPC and the gateway wired it into WebConfig.
    let (features, panels) = expect_hello(&mut ws).await;
    assert!(
        !features.is_empty(),
        "features must not be empty: {features:?}"
    );
    assert!(!panels.is_empty(), "panels must not be empty: {panels:?}");
    for f in ["channels", "mail", "pilot"] {
        assert!(
            features.iter().any(|x| x == f),
            "feature {f} in {features:?}"
        );
    }
    for p in ["comsys", "mail", "who"] {
        assert!(panels.iter().any(|x| x == p), "panel {p} in {panels:?}");
    }
    assert!(
        panels.iter().any(|p| p.ends_with(".sheet")),
        "a <ruleset>.sheet feed in the split manifest: {panels:?}"
    );

    // The entry stream crosses the boundary: SessionInfo then the room render.
    let si = expect_env(&mut ws, "Connected as Nick (nick)").await;
    assert!(matches!(si.event, OutputEvent::SessionInfo { .. }));
    let room = expect_env(&mut ws, "Limbo(#0)").await;
    // 16b cut B.4 (`B-14`): the attach look is the core `room` query's answer,
    // `Reply { id: 0, event: Room }`, sent to this one connection.
    assert!(matches!(
        &room.event,
        OutputEvent::Reply { id: 0, event } if matches!(**event, OutputEvent::Room(_))
    ));

    // Play: a say typed on the WS round-trips through the world loop on the
    // SERVER process and echoes back to this same WS connection.
    send_line(&mut ws, "say hi over the split web").await;
    let echo = expect_env(&mut ws, "Nick says, \"hi over the split web\"").await;
    assert!(matches!(echo.event, OutputEvent::Narrative { .. }));
    assert!(
        echo.seq > room.seq,
        "seqs stay monotonic across the round trip"
    );

    drop(ws);
    let _ = (server_thread, gateway_thread);
}

/// Login to an existing account over the gateway's REST surface and mint a
/// fresh session token.
async fn login(web: SocketAddr, account: &str, password: &str) -> String {
    let url = format!("http://{web}/api/auth/login");
    let resp = post_json(&url, json!({"account": account, "password": password})).await;
    assert_eq!(resp.status(), 200, "login must succeed across the split");
    let body: Value = resp.json().await.unwrap();
    body["token"].as_str().unwrap().to_string()
}

/// EXT-P1-3(b): two overlapping gateway generations (`Generation::A` and
/// `Generation::B`) connected to one `mush-server` and sharing one token store.
/// Logging out `token_1` via **Generation B** must propagate over IPC through
/// `mush-server` and immediately boot the live WS session authenticated by
/// `token_1` on **Generation A**, while leaving another live WS session on
/// Generation A authenticated by `token_2` unaffected.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cross_generation_logout_boots_live_ws_on_peer_generation() {
    use super::harness::{GatewaySpec, GatewayUnit, ServerUnit};
    use mush_config::generation::Generation;

    let server_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let server = ServerUnit::new(server_store, || server_config("crossgen-server"));
    server.start().await;

    let shared_gw_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let mut cfg_a = gateway_config("crossgen-gw-a");
    server.aim(&mut cfg_a);
    let mut cfg_b = gateway_config("crossgen-gw-b");
    server.aim(&mut cfg_b);

    let gw_a = GatewayUnit::start_with(
        cfg_a,
        Arc::clone(&shared_gw_store),
        GatewaySpec {
            generation: Some(Generation::A),
            web: true,
            ..GatewaySpec::default()
        },
    )
    .await;
    let gw_b = GatewayUnit::start_with(
        cfg_b,
        Arc::clone(&shared_gw_store),
        GatewaySpec {
            generation: Some(Generation::B),
            web: true,
            ..GatewaySpec::default()
        },
    )
    .await;

    let web_a = gw_a.web_addr();
    let web_b = gw_b.web_addr();

    // Mint two distinct bearer tokens for the same account; attach both on
    // Generation A.
    let (token1, char_key) = register(web_a, "alice", "hunter42", "Alice").await;
    let token2 = login(web_a, "alice", "hunter42").await;

    let mut ws1 = connect_ws(web_a, &token1, char_key)
        .await
        .expect("ws1 connect on gen A");
    let _ = expect_hello(&mut ws1).await;
    let _ = expect_env(&mut ws1, "Connected as Alice (alice)").await;
    let _ = expect_env(&mut ws1, "Limbo(#0)").await;

    let mut ws2 = connect_ws(web_a, &token2, char_key)
        .await
        .expect("ws2 connect on gen A");
    let _ = expect_hello(&mut ws2).await;
    let _ = expect_env(&mut ws2, "Connected as Alice (alice)").await;
    let _ = expect_env(&mut ws2, "Limbo(#0)").await;

    // Logout `token1` via Generation B (the peer generation).
    let logout_resp = reqwest::Client::new()
        .post(format!("http://{web_b}/api/auth/logout"))
        .bearer_auth(&token1)
        .send()
        .await
        .unwrap();
    assert_eq!(logout_resp.status(), 204, "logout on gen B must succeed");

    // `ws1` on Generation A must receive Disconnect and close promptly.
    let mut saw_disconnect = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let next = tokio::time::timeout_at(deadline, ws1.next())
            .await
            .expect("ws1 on gen A must close promptly after logout on gen B");
        match next {
            Some(Ok(Message::Text(text))) => {
                let env: Envelope = serde_json::from_str(text.as_str()).unwrap();
                if matches!(env.event, OutputEvent::Disconnect { .. }) {
                    saw_disconnect = true;
                }
            }
            Some(Ok(Message::Close(_))) | None => break,
            Some(Ok(_)) => {}
            Some(Err(_)) => break,
        }
    }
    assert!(
        saw_disconnect,
        "ws1 on gen A must receive Disconnect before closing"
    );

    // `ws2` (other bearer token) on Generation A stays alive and playable.
    send_line(&mut ws2, "say still connected after peer logout").await;
    let _ = expect_env(
        &mut ws2,
        "Alice says, \"still connected after peer logout\"",
    )
    .await;

    // And `token1` can no longer reconnect on Generation A or B.
    let mut stale = connect_ws(web_a, &token1, char_key)
        .await
        .expect("stale token upgrades to close 4401");
    let close_frame = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(msg) = stale.next().await {
            if let Ok(Message::Close(frame)) = msg {
                return frame;
            }
        }
        None
    })
    .await
    .expect("stale ws closes promptly");
    assert_eq!(
        close_frame.map(|f| u16::from(f.code)),
        Some(4401),
        "revoked token must be rejected with 4401"
    );

    drop(ws2);
    gw_a.stop().await;
    gw_b.stop().await;
    server.stop().await;
}

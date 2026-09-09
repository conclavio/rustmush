//! The M16d (W2) PROOF: a WEB (WebSocket) connection SURVIVES a real server
//! restart — the WS analog of
//! [`restart_survival::server_restart_keeps_connections_and_world`].
//!
//! The gateway supervises the server (launched here as a sibling thread over an
//! ephemeral localhost IPC boundary — the same shape as a child process, and
//! deterministic on Windows) AND serves the web client on its own ephemeral
//! HTTP/WS listener. A real WebSocket client connects THROUGH the gateway,
//! authenticates with a token, receives the hello manifest, and plays; then the
//! server is RESTARTED through the gateway's programmatic [`RestartTrigger`] (the
//! cross-platform stand-in for the Unix `SIGHUP` deploy signal). The proof
//! asserts:
//!
//! - the WS connection SURVIVES — it is never sent a Close frame (`recv_env`
//!   panics on a close, so a dropped WS fails the test);
//! - the player is told "restarting" over the SAME WS, is pushed a fresh room
//!   render from the reloaded world, and sees "Reconnected" only after the
//!   roster replay's readiness barrier (W2/D2/D3), all as ordinary JSON
//!   envelopes;
//! - world state is INTACT across the restart — a room the player `@dig`ged
//!   before the restart is still reachable afterwards (R1: the server reloads its
//!   world from the shared store, exactly as it would from Redis);
//! - the player can keep playing — a `say` typed on the WS still round-trips
//!   through the world loop on the (respawned) server process afterwards.
//!
//! The server here is launched with a SHARED in-memory store carried across every
//! (re)boot, so world objects + durable module state survive a restart without a
//! live Redis on the dev box (the Redis path is byte-identical — the server just
//! reloads the world on boot either way). The player is the seeded God wizard
//! (so `@dig` may open exits from Limbo, which a mortal could not).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use futures::{SinkExt, StreamExt};
use mush_core::GOD;
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use mush_config::accounts::hash_password;
use mush_config::config::Config;
use mush_gateway::{GatewayOptions, LaunchedServer, RestartTrigger, ServerLauncher, ServerSource};
use mush_proto::{ClientCommand, Envelope, OutputEvent};
use mush_testkit::text_of;
use serde_json::{Value, json};

const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const GM_PASSWORD: &str = "gm-secret-1";

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

fn scratch_dir(tag: &str) -> PathBuf {
    // A UNIQUE dir per call (not just per PID+tag): every server launch — across
    // concurrent tests AND every restart — gets its own data_dir, so the
    // remove_dir_all below never races a sibling launch still using the path.
    // (Lifted verbatim from restart_survival.rs, where a shared dir was an
    // intermittent, load-dependent fast failure.)
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("mush-webrestart-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The SERVER config: in-memory, ephemeral IPC, the `wod` ruleset loaded so the
/// boot Hello manifest carries the `<ruleset>.sheet` feed + `wod` feature — the
/// same rich manifest the all-in-one advertises, re-delivered on every reboot.
fn server_config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ssh.listen = "127.0.0.1:0".to_string();
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    cfg.rulesets.enabled = vec!["wod".to_string()];
    cfg
}

/// The GATEWAY config: in-memory, ephemeral SSH + WEB listeners (the web table
/// binds `127.0.0.1:0`; `web_ready` reports the OS-assigned port). `auth_burst`
/// 0 disables the auth rate limiter for the test's login.
fn gateway_config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ssh.listen = "127.0.0.1:0".to_string();
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    cfg.rulesets.enabled = vec![];
    let mut web = toml::value::Table::new();
    web.insert(
        "listen".to_string(),
        toml::Value::String("127.0.0.1:0".to_string()),
    );
    web.insert("auth_burst".to_string(), toml::Value::Integer(0));
    cfg.web = toml::Value::Table(web);
    cfg
}

/// Boot one real `mush-server` on its own thread + runtime over an ephemeral IPC
/// endpoint, sharing `store` so the world survives across restarts. Each restart
/// re-runs this against the SAME store. `exited` fires when that server's `serve`
/// returns.
async fn launch_thread_server(store: Arc<dyn ObjectStore>) -> Result<LaunchedServer, String> {
    let cfg = server_config("server");
    let (ipc_tx, ipc_rx) = oneshot::channel();
    let (exit_tx, exit_rx) = oneshot::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let _ = rt.block_on(mush_server::serve_with_store(
            cfg,
            Some(ipc_tx),
            Some(store),
        ));
        let _ = exit_tx.send(());
    });
    let endpoint = tokio::time::timeout(Duration::from_secs(30), ipc_rx)
        .await
        .map_err(|_| "server ipc bind timed out".to_string())?
        .map_err(|_| "server dropped its ipc oneshot".to_string())?;
    Ok(LaunchedServer {
        endpoint,
        exited: Box::pin(async move {
            let _ = exit_rx.await;
        }),
    })
}

/// The restart-survival launcher: each `launch` boots a fresh `mush-server`
/// against the ONE shared store, so the world survives every restart.
struct ThreadLauncher {
    store: Arc<dyn ObjectStore>,
}

impl ServerLauncher for ThreadLauncher {
    fn launch(&self) -> BoxFuture<'static, Result<LaunchedServer, String>> {
        let store = Arc::clone(&self.store);
        Box::pin(launch_thread_server(store))
    }
}

/// The gateway's own KV store (accounts), seeded with the God wizard so the web
/// login resolves to a wizard who may `@dig` exits from Limbo.
async fn seeded_gw_store() -> Arc<dyn ObjectStore> {
    let gw_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    gw_store
        .create_account(AccountRec {
            id: None,
            name: "boss".to_string(),
            argon2_hash: hash_password(GM_PASSWORD).unwrap(),
            created: 0,
            characters: vec![CharacterRef {
                id: GOD,
                name: "God".to_string(),
            }],
            ssh_keys: vec![],
            discord: None,
        })
        .await
        .unwrap();
    gw_store
}

/// Start the gateway (on its own thread/runtime, as the binary does) supervising
/// `launcher` AND serving the web client on an ephemeral listener; return the
/// bound WEB address plus the programmatic restart trigger. The gateway thread is
/// detached — `serve` blocks until Ctrl-C, which never comes in-test; dropping at
/// test end tears the process down.
async fn start_split_web(launcher: Box<dyn ServerLauncher>) -> (SocketAddr, RestartTrigger) {
    let gw_store = seeded_gw_store().await;
    let (web_tx, web_rx) = oneshot::channel();
    let (restart_tx, restart_rx) = oneshot::channel::<RestartTrigger>();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(mush_gateway::serve(
            gateway_config("gateway"),
            GatewayOptions {
                source: ServerSource::Restartable(launcher),
                store: Some(gw_store),
                ssh_ready: None,
                web_ready: Some(web_tx),
                restart_ready: Some(restart_tx),
                siteban_refresh: None,
            },
        ))
        .unwrap();
    });
    let web_addr = tokio::time::timeout(Duration::from_secs(30), web_rx)
        .await
        .expect("gateway bound WEB")
        .expect("web addr");
    let trigger = tokio::time::timeout(Duration::from_secs(30), restart_rx)
        .await
        .expect("supervisor handed back a restart trigger")
        .expect("restart trigger");
    (web_addr, trigger)
}

async fn post_json(url: &str, body: Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(url)
        .json(&body)
        .send()
        .await
        .unwrap()
}

/// Log in over the gateway's REST surface; returns (token, God's char key).
async fn login(web: SocketAddr, account: &str, password: &str) -> (String, u64) {
    let url = format!("http://{web}/api/auth/login");
    let resp = post_json(&url, json!({"account": account, "password": password})).await;
    assert_eq!(resp.status(), 200, "login must succeed across the split");
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

/// Next non-keepalive JSON envelope (skips WS control frames and answers the
/// protocol's application-level Ping); PANICS on close — a WS close is the
/// exact W2 failure (the connection must never be dropped across a restart).
async fn recv_env(ws: &mut Ws) -> Envelope {
    loop {
        let msg = tokio::time::timeout(STEP_TIMEOUT, ws.next())
            .await
            .expect("timed out waiting for a frame")
            .expect("stream ended")
            .expect("ws error");
        match msg {
            Message::Text(text) => {
                let env: Envelope = serde_json::from_str(text.as_str()).unwrap();
                if let OutputEvent::Ping { nonce } = &env.event {
                    let pong = ClientCommand::Pong { nonce: *nonce };
                    ws.send(Message::Text(serde_json::to_string(&pong).unwrap().into()))
                        .await
                        .expect("send application keepalive pong");
                    continue;
                }
                return env;
            }
            Message::Close(frame) => panic!("WS was CLOSED (W2 violation): {frame:?}"),
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

/// Consume the restart replay until both the fresh room and the final readiness
/// notice have arrived. ReAnnounce produces the room before its exact Barrier
/// resolves, but the two gateway/server writer tasks need not expose a stable
/// presentation order; neither envelope may be discarded while seeking the
/// other.
async fn expect_ready_replay(ws: &mut Ws, room_needle: &str) -> Envelope {
    let mut reconnected = false;
    let mut room = None;
    let mut seen = Vec::new();
    while !reconnected || room.is_none() {
        let env = recv_env(ws).await;
        let text = text_of(&env.event);
        if text.contains("Reconnected") {
            reconnected = true;
        }
        if text.contains(room_needle) {
            assert!(
                matches!(&env.event, OutputEvent::Room(_)),
                "restart room marker came from a non-Room event"
            );
            room = Some(env);
        } else {
            seen.push(text);
        }
        assert!(
            seen.len() < 200,
            "restart replay did not contain both Reconnected and {room_needle:?}: {seen:?}"
        );
    }
    room.expect("loop exits only with the replay room")
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

/// W2: a WS connection SURVIVES a real server restart. It connects through the
/// split gateway, plays, then rides a restart WITHOUT ever being closed —
/// receiving restarting → Reconnected → a fresh room render as ordinary
/// envelopes, with the pre-restart world intact and play continuing after.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn web_connection_survives_server_restart_with_world_intact() {
    // The SERVER's world store — SHARED across every restart, so the world
    // survives (stands in for Redis on the dev box).
    let server_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let (web_addr, trigger) = start_split_web(Box::new(ThreadLauncher {
        store: Arc::clone(&server_store),
    }))
    .await;

    // --- Log in over REST + open a WS through the gateway as the God wizard. ---
    let (token, key) = login(web_addr, "boss", GM_PASSWORD).await;
    let mut ws = connect_ws(web_addr, &token, key)
        .await
        .expect("ws connect through the split gateway");

    // The hello manifest is NON-EMPTY and matches the all-in-one's surface (the
    // per-module features comsys→channels/mail + the wod ruleset's sheet, panel
    // feeds who/comsys/mail + <ruleset>.sheet) — proving the server shipped its
    // real manifest over IPC at boot and the gateway wired it into WebConfig.
    let (features, panels) = expect_hello(&mut ws).await;
    assert!(
        !features.is_empty(),
        "features must not be empty: {features:?}"
    );
    assert!(!panels.is_empty(), "panels must not be empty: {panels:?}");
    for f in ["channels", "wod"] {
        assert!(
            features.iter().any(|x| x == f),
            "feature {f} in {features:?}"
        );
    }
    assert!(panels.iter().any(|p| p == "who"), "who panel in {panels:?}");
    assert!(
        panels.iter().any(|p| p.ends_with(".sheet")),
        "a <ruleset>.sheet feed in the split manifest: {panels:?}"
    );

    // Entry stream crosses the boundary: SessionInfo then the live room render.
    let si = expect_env(&mut ws, "Connected as God").await;
    assert!(matches!(si.event, OutputEvent::SessionInfo { .. }));
    let room = expect_env(&mut ws, "Limbo(#0)").await;
    assert!(matches!(room.event, OutputEvent::Room(_)));

    // --- A mutating command that must survive the restart: dig a room with a
    //     two-way exit (God controls Limbo, so exits open). ---
    send_line(&mut ws, "@dig Vault = east;e, west;w").await;
    expect_env(&mut ws, "Vault created with room number").await;

    // Speech round-trips through the world loop on the SERVER process before the
    // restart, echoing back to this same WS.
    send_line(&mut ws, "say pre-restart").await;
    let pre = expect_env(&mut ws, "God says, \"pre-restart\"").await;
    assert!(matches!(pre.event, OutputEvent::Narrative { .. }));

    // --- RESTART the server (the programmatic SIGHUP stand-in). ---
    assert!(trigger.restart().await, "restart must be accepted");

    // The WS is told the game is restarting, is pushed a fresh room render, and
    // receives Reconnected only after the exact readiness barrier — WITHOUT ever
    // being closed (W2). Replay render and final notice are adjacent but their
    // presentation order is deliberately not a client contract, so retain both
    // while consuming the stream. The gateway generates the system lines; the
    // look comes from the respawned server.
    expect_env(&mut ws, "restarting").await;
    let relook = expect_ready_replay(&mut ws, "Limbo(#0)").await;
    assert!(matches!(relook.event, OutputEvent::Room(_)));

    // --- World state INTACT (R1): the Vault @dug before the restart is still
    //     reachable. Walk east into it. ---
    send_line(&mut ws, "east").await;
    let vault = expect_env(&mut ws, "Vault").await;
    assert!(matches!(vault.event, OutputEvent::Room(_)));

    // --- Play CONTINUES after the restart: a say still round-trips through the
    //     respawned world loop and echoes back to this WS. ---
    send_line(&mut ws, "say post-restart").await;
    let post = expect_env(&mut ws, "God says, \"post-restart\"").await;
    assert!(matches!(post.event, OutputEvent::Narrative { .. }));
    assert!(post.seq > pre.seq, "seqs stay monotonic across the restart");

    drop(ws);
}

//! The M4 exit criterion end-to-end, over real sockets: "SSH and web user
//! in the same room."
//!
//! Boots the whole server in-process — store-seeded world, the real module
//! roster, the persist world task, store-backed accounts, the SSH listener
//! (russh), AND the web ingress (`mush_net_web::router` served by axum with
//! connect-info, exactly as `serve_with_shutdown` serves it) — on ephemeral
//! ports, then drives it with a real SSH client, a real HTTP client
//! (reqwest), and a real WebSocket client (tokio-tungstenite).
//!
//! Everything is ungated/in-memory; the Redis-gated suites cover the store.
//!
//! Inventory:
//! - REST auth: register / login / bad-password 401 / logout 204, and the
//!   tower_governor 429 after the burst.
//! - The same-room proof: SSH say → WS `Narrative` envelope; WS `Line` →
//!   SSH render. Plus the `Req` surface (Who, PanelSubscribe→keyframe,
//!   ChannelBacklog, MailList) against the real module query handlers.
//! - Resume: drop the WS, miss output, reconnect with `?resume=` → exactly
//!   the gap replays in order; a gap beyond the ring falls back to
//!   `resumed {replayed: false}` + fresh snapshots.
//! - Revocation is live: `POST /api/auth/logout` boots the token's WS;
//!   `@newpassword` boots the account's web connections and kills both
//!   tokens.
//! - The embedded SPA serves: `/` is the app shell, hashed assets are
//!   immutable, `/healthz` answers.

mod support;

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use mush_core::{Engine, GOD};
use mush_net_web::WebConfig;
use mush_proto::{ClientCommand, Envelope, OutputEvent, Query};
use mush_session::{SessionConfig, SessionManager};
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use mush_testkit::text_of;
use serde_json::{Value, json};

use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use mushd::accounts::{Accounts, hash_password};
use mushd::ipguard::IpGuard;
use mushd::modules;
use mushd::persist::load_or_seed_world;
use mushd::ssh_bridge::{SshBridge, SshListener, start_listener};
use mushd::staff::{StaffQueue, register_server_commands};
use mushd::web_bridge::WebBridge;
use mushd::world_task::{self, PersistHandles, ServerDeps};

use support::{STEP_TIMEOUT, Term, play_client, scratch_dir, strip_ansi};

const GM_PASSWORD: &str = "gm-secret-1";
const BANNER: &str = "Welcome to the web e2e MUSH!";

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

// ---------------------------------------------------------------------------
// Rig: the full production wiring on ephemeral ports
// ---------------------------------------------------------------------------

struct Rig {
    ssh: SocketAddr,
    web: SocketAddr,
    dir: PathBuf,
    _ssh_listener: SshListener,
    _web_task: tokio::task::JoinHandle<()>,
    _handles: PersistHandles,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Rig {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.web)
    }
}

/// Boot everything `main.rs` boots (minus TLS/limiter-GC): seeded world +
/// boss/God account, module roster with query handlers, staff commands,
/// persist world task, SSH listener, and the web router with the same
/// features/panels manifest. `auth_burst` 0 disables the auth rate limiter
/// (most tests; the dedicated rate-limit test turns it on).
async fn boot(tag: &str, session_config: SessionConfig, auth_burst: u32) -> Rig {
    boot_inner(tag, session_config, auth_burst, Vec::new()).await
}

/// Boot with the wod ruleset loaded too (adds the `wod.sheet`/`wod.roll`
/// feeds + `+chargen`/`+sheet` surface), for the sheet-over-web test.
async fn boot_wod(tag: &str, session_config: SessionConfig, auth_burst: u32) -> Rig {
    let wod = modules::ruleset_modules(&["wod".to_string()]).expect("wod ruleset");
    boot_inner(tag, session_config, auth_burst, wod).await
}

async fn boot_inner(
    tag: &str,
    session_config: SessionConfig,
    auth_burst: u32,
    extra_modules: Vec<Box<dyn mush_core::GameModule>>,
) -> Rig {
    let dir = scratch_dir(tag);
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    store
        .create_account(AccountRec {
            name: "boss".to_string(),
            argon2_hash: hash_password(GM_PASSWORD).unwrap(),
            created: 0,
            characters: vec![CharacterRef {
                id: GOD,
                name: "God".to_string(),
            }],
        })
        .await
        .unwrap();
    let world = load_or_seed_world(&*store).await.expect("boot world");
    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff).unwrap();
    let mut all_modules = modules::server_modules();
    all_modules.extend(extra_modules);
    let mut engine = Engine::with_modules(world, registry, all_modules).expect("modules");
    modules::boot_modules(&mut engine, &*store)
        .await
        .expect("module boot");

    // The hello manifest, exactly as main.rs builds it.
    let mut panels: Vec<String> = engine
        .registry
        .panel_feeds()
        .iter()
        .map(|(f, _)| f.to_string())
        .collect();
    panels.push("who".to_string());
    let features: Vec<String> = engine
        .modules()
        .iter()
        .map(|m| match m.name() {
            "comsys" => "channels".to_string(),
            other => other.to_string(),
        })
        .collect();

    let sessions = Arc::new(SessionManager::with_config(session_config));
    let guard = Arc::new(IpGuard::new());
    let handles = world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        ServerDeps {
            store: Arc::clone(&store),
            guard: Arc::clone(&guard),
            staff,
        },
    );
    let accounts = Arc::new(Accounts::new(Arc::clone(&store), handles.handle.clone(), 3));

    let ssh_bridge = Arc::new(SshBridge::new(
        Arc::clone(&accounts),
        Arc::clone(&sessions),
        handles.handle.clone(),
        Arc::clone(&guard),
    ));
    let ssh_listener = start_listener(
        "127.0.0.1:0".parse().unwrap(),
        &dir,
        BANNER.to_string(),
        ssh_bridge,
    )
    .await
    .expect("ssh listener");
    let ssh = ssh_listener.addr();

    let web_bridge: Arc<dyn mush_net_web::WebHooks> = Arc::new(WebBridge::new(
        accounts,
        sessions,
        handles.handle.clone(),
        guard,
        store,
        panels.clone(),
    ));
    let web_config = WebConfig {
        server_name: "rustmush e2e".to_string(),
        features,
        panels,
        auth_burst,
        ..WebConfig::default()
    };
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let web = listener.local_addr().unwrap();
    let app = mush_net_web::router(web_bridge, web_config);
    let web_task = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    Rig {
        ssh,
        web,
        dir,
        _ssh_listener: ssh_listener,
        _web_task: web_task,
        _handles: handles,
    }
}

// ---------------------------------------------------------------------------
// Client helpers
// ---------------------------------------------------------------------------

async fn post_json(url: &str, body: Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(url)
        .json(&body)
        .send()
        .await
        .unwrap()
}

/// Register an account + first character over REST; returns (token, key).
async fn register(rig: &Rig, account: &str, password: &str, char_name: &str) -> (String, u64) {
    let resp = post_json(
        &rig.url("/api/auth/register"),
        json!({"account": account, "password": password, "char_name": char_name}),
    )
    .await;
    assert_eq!(resp.status(), 200, "register must succeed");
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["account"], account);
    assert_eq!(body["characters"][0]["name"], char_name);
    (
        body["token"].as_str().unwrap().to_string(),
        body["characters"][0]["key"].as_u64().unwrap(),
    )
}

/// Open a WS with the token in the `Sec-WebSocket-Protocol` subprotocol
/// (the SPA's path) and any extra query (e.g. `&resume=42`).
async fn connect_ws(rig: &Rig, token: &str, char_key: u64, extra: &str) -> Result<Ws, WsError> {
    let mut req = format!("ws://{}/ws?char={char_key}{extra}", rig.web)
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

/// Read frames until the server's close frame; returns `(code, reason)`
/// and every stamped envelope seen on the way.
async fn recv_close(ws: &mut Ws) -> (u16, String, Vec<Envelope>) {
    let mut seen = Vec::new();
    loop {
        let msg = tokio::time::timeout(STEP_TIMEOUT, ws.next())
            .await
            .expect("timed out waiting for close");
        match msg {
            Some(Ok(Message::Close(Some(frame)))) => {
                return (frame.code.into(), frame.reason.to_string(), seen);
            }
            Some(Ok(Message::Text(text))) => {
                seen.push(serde_json::from_str(text.as_str()).unwrap());
            }
            Some(Ok(_)) => {}
            None => panic!("stream ended without a close frame; saw {seen:?}"),
            Some(Err(err)) => panic!("ws error waiting for close: {err}"),
        }
    }
}

async fn send_cmd(ws: &mut Ws, cmd: &ClientCommand) {
    ws.send(Message::Text(serde_json::to_string(cmd).unwrap().into()))
        .await
        .unwrap();
}

async fn send_line(ws: &mut Ws, line: &str) {
    send_cmd(ws, &ClientCommand::Line(line.to_string())).await;
}

/// Expect the out-of-band `hello` (seq 0) as the first frame.
async fn expect_hello(ws: &mut Ws) -> (Vec<String>, Vec<String>) {
    let env = recv_env(ws).await;
    assert_eq!(env.seq, 0, "hello is out-of-band");
    match env.event {
        OutputEvent::Hello {
            server,
            features,
            panels,
            ..
        } => {
            assert_eq!(server, "rustmush e2e");
            (features, panels)
        }
        other => panic!("expected hello first, got {other:?}"),
    }
}

/// God's SSH terminal, connected through the welcome screen.
async fn god_terminal(rig: &Rig) -> (russh::client::Handle<support::AcceptAll>, Term) {
    let (handle, mut god) = play_client(rig.ssh).await;
    god.expect(BANNER).await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God (account boss, 1 connection).")
        .await;
    god.expect("Limbo(#0)").await;
    (handle, god)
}

// ---------------------------------------------------------------------------
// REST auth + rate limiting
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn rest_auth_flow_register_login_logout() {
    let rig = boot("rest-auth", SessionConfig::default(), 0).await;

    let (token, key) = register(&rig, "nick", "hunter42", "Nick").await;
    assert_eq!(token.len(), 64);
    assert!(key > 1, "a fresh Player id, not God");

    // Duplicate name → 409; weak password → 422.
    let dup = post_json(
        &rig.url("/api/auth/register"),
        json!({"account": "nick", "password": "hunter42", "char_name": "Other"}),
    )
    .await;
    assert_eq!(dup.status(), 409);
    let weak = post_json(
        &rig.url("/api/auth/register"),
        json!({"account": "zoe", "password": "abc", "char_name": "Zoe"}),
    )
    .await;
    assert_eq!(weak.status(), 422);

    // Login: good creds 200 with a fresh token; bad password a uniform 401.
    let ok = post_json(
        &rig.url("/api/auth/login"),
        json!({"account": "nick", "password": "hunter42"}),
    )
    .await;
    assert_eq!(ok.status(), 200);
    let body: Value = ok.json().await.unwrap();
    assert_eq!(body["characters"][0]["key"], key);
    assert_ne!(body["token"], token, "each login mints its own token");

    let bad = post_json(
        &rig.url("/api/auth/login"),
        json!({"account": "nick", "password": "wrong"}),
    )
    .await;
    assert_eq!(bad.status(), 401);
    let ghost = post_json(
        &rig.url("/api/auth/login"),
        json!({"account": "ghost", "password": "wrong"}),
    )
    .await;
    assert_eq!(ghost.status(), 401);

    // Logout: 204, idempotent, missing bearer 400.
    let client = reqwest::Client::new();
    let out = client
        .post(rig.url("/api/auth/logout"))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(out.status(), 204);
    let no_bearer = client
        .post(rig.url("/api/auth/logout"))
        .send()
        .await
        .unwrap();
    assert_eq!(no_bearer.status(), 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_routes_rate_limit_to_429() {
    // Burst 3: three attempts pass (as 401s), the fourth never reaches the
    // hooks. Kept under IpGuard's 5-failure tempban window.
    let rig = boot("rest-429", SessionConfig::default(), 3).await;
    let url = rig.url("/api/auth/login");
    let body = json!({"account": "nobody", "password": "wrong"});
    for _ in 0..3 {
        assert_eq!(post_json(&url, body.clone()).await.status(), 401);
    }
    assert_eq!(post_json(&url, body).await.status(), 429);
}

// ---------------------------------------------------------------------------
// The M4 demo: SSH and web user in the same room (+ the Req surface)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)] // one scripted journey, like the M1 e2e
async fn ssh_and_web_user_in_the_same_room() {
    let rig = boot("journey", SessionConfig::default(), 0).await;
    let (_god_handle, mut god) = god_terminal(&rig).await;

    // Nick registers over REST and connects over WS.
    let (token, key) = register(&rig, "nick", "hunter42", "Nick").await;
    let mut ws = connect_ws(&rig, &token, key, "").await.expect("ws connect");
    let (features, panels) = expect_hello(&mut ws).await;
    for f in ["channels", "mail", "jobs"] {
        assert!(
            features.iter().any(|x| x == f),
            "feature {f} in {features:?}"
        );
    }
    for p in ["comsys", "mail", "who"] {
        assert!(panels.iter().any(|x| x == p), "panel {p} in {panels:?}");
    }

    // Entry stream: SessionInfo then the room render, stamped and ordered.
    let si = expect_env(&mut ws, "Connected as Nick (nick)").await;
    assert!(matches!(si.event, OutputEvent::SessionInfo { .. }));
    assert!(si.seq >= 1);
    let room = expect_env(&mut ws, "Limbo(#0)").await;
    assert!(matches!(room.event, OutputEvent::Room(_)));
    assert!(room.seq > si.seq);
    god.expect("Nick has connected.").await;

    // SSH → web: God's say arrives as a stamped Narrative envelope.
    god.send_line("say hello from ssh").await;
    let say = expect_env(&mut ws, "God says, \"hello from ssh\"").await;
    assert!(matches!(say.event, OutputEvent::Narrative { .. }));
    assert!(say.seq > room.seq, "seqs stay monotonic");

    // Web → SSH: Nick's Line renders on God's terminal.
    send_line(&mut ws, "say hi from the web").await;
    god.expect("Nick says, \"hi from the web\"").await;
    // ...and echoes back to Nick's own scrollback.
    expect_env(&mut ws, "Nick says, \"hi from the web\"").await;

    // Typed WHO over the structured wire renders as a Who event.
    send_line(&mut ws, "WHO").await;
    let who = expect_env(&mut ws, "Nick").await;
    let OutputEvent::Who(entries) = &who.event else {
        panic!("expected Who, got {:?}", who.event);
    };
    assert!(entries.iter().any(|e| e.name == "God"));
    assert!(entries.iter().any(|e| e.name == "Nick"));

    // Req Who → correlated Reply.
    send_cmd(
        &mut ws,
        &ClientCommand::Req {
            id: 1,
            q: Query::Who,
        },
    )
    .await;
    let env = expect_env(&mut ws, "[reply 1]").await;
    let OutputEvent::Reply { id: 1, event } = &env.event else {
        panic!("expected Reply 1, got {:?}", env.event);
    };
    let OutputEvent::Who(entries) = event.as_ref() else {
        panic!("expected Who reply, got {event:?}");
    };
    assert!(entries.iter().any(|e| e.name == "Nick"));

    // PanelSubscribe("comsys") acks with the comsys.state keyframe.
    send_cmd(
        &mut ws,
        &ClientCommand::Req {
            id: 2,
            q: Query::PanelSubscribe {
                feed: "comsys".into(),
            },
        },
    )
    .await;
    let env = expect_env(&mut ws, "[reply 2]").await;
    let OutputEvent::Reply { id: 2, event } = &env.event else {
        panic!("expected Reply 2, got {:?}", env.event);
    };
    let OutputEvent::Panel { feed, data, .. } = event.as_ref() else {
        panic!("expected Panel keyframe, got {event:?}");
    };
    assert_eq!(feed, "comsys");
    assert!(
        data["channels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "Public"),
        "auto-joined Public in {data}"
    );

    // Speak on Public (live Channel frame), then pull the backlog.
    send_line(&mut ws, "pub hello channel tab").await;
    let chan = expect_env(&mut ws, "hello channel tab").await;
    assert!(matches!(chan.event, OutputEvent::Channel { .. }));
    send_cmd(
        &mut ws,
        &ClientCommand::Req {
            id: 3,
            q: Query::ChannelBacklog {
                channel: "Public".into(),
                before_seq: None,
                limit: 10,
            },
        },
    )
    .await;
    let env = expect_env(&mut ws, "[reply 3]").await;
    let OutputEvent::Reply { id: 3, event } = &env.event else {
        panic!("expected Reply 3, got {:?}", env.event);
    };
    let OutputEvent::Panel { feed, data, .. } = event.as_ref() else {
        panic!("expected Panel, got {event:?}");
    };
    assert_eq!(feed, "comsys");
    assert!(
        data["backlog"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["text"].as_str().unwrap().contains("hello channel tab")),
        "backlog carries the message: {data}"
    );

    // God mails Nick; the notice streams live, and MailList shows it.
    god.send_line("@mail/quick Nick=Web test/A body sent over ssh")
        .await;
    god.expect("sent to Nick.").await;
    expect_env(&mut ws, "Web test").await;
    send_cmd(
        &mut ws,
        &ClientCommand::Req {
            id: 4,
            q: Query::MailList { folder: None },
        },
    )
    .await;
    let env = expect_env(&mut ws, "[reply 4]").await;
    let OutputEvent::Reply { id: 4, event } = &env.event else {
        panic!("expected Reply 4, got {:?}", env.event);
    };
    let OutputEvent::Panel { feed, data, .. } = event.as_ref() else {
        panic!("expected Panel, got {event:?}");
    };
    assert_eq!(feed, "mail");
    assert_eq!(data["folder"], "inbox");
    assert!(
        data["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["subject"] == "Web test"),
        "inbox lists the mail: {data}"
    );

    // Clean exit: QUIT closes the socket after the farewell.
    send_line(&mut ws, "QUIT").await;
    let (code, _reason, seen) = recv_close(&mut ws).await;
    assert_eq!(code, 1001, "game-side close is 1001 going-away");
    assert!(
        seen.iter()
            .any(|e| matches!(e.event, OutputEvent::Disconnect { .. })),
        "farewell Disconnect before the close: {seen:?}"
    );
    god.expect("Nick has disconnected.").await;
}

// ---------------------------------------------------------------------------
// Resume
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn ws_resume_replays_exactly_the_gap() {
    let rig = boot("resume", SessionConfig::default(), 0).await;
    let (_god_handle, mut god) = god_terminal(&rig).await;

    let (token, key) = register(&rig, "nick", "hunter42", "Nick").await;
    let mut ws = connect_ws(&rig, &token, key, "").await.expect("ws connect");
    expect_hello(&mut ws).await;
    let last_seq = expect_env(&mut ws, "Limbo(#0)").await.seq;
    god.expect("Nick has connected.").await;

    // Drop the socket; wait until the server has noticed (linkdead) so the
    // missed output is stamped strictly after our resume point.
    ws.close(None).await.unwrap();
    drop(ws);
    god.expect("Nick has gone link-dead.").await;
    god.send_line("say missed one").await;
    god.expect("God says, \"missed one\"").await;
    god.send_line("say missed two").await;
    god.expect("God says, \"missed two\"").await;

    // Reconnect with ?resume=<last_seq>: hello, resumed{true}, then the gap
    // — exactly the two missed narratives, in order, gap-free.
    let mut ws = connect_ws(&rig, &token, key, &format!("&resume={last_seq}"))
        .await
        .expect("ws resume");
    expect_hello(&mut ws).await;
    let resumed = recv_env(&mut ws).await;
    assert_eq!(resumed.seq, 0);
    assert!(
        matches!(resumed.event, OutputEvent::Resumed { replayed: true }),
        "expected resumed(replayed=true), got {:?}",
        resumed.event
    );
    let first = recv_env(&mut ws).await;
    let second = recv_env(&mut ws).await;
    assert!(
        text_of(&first.event).contains("God says, \"missed one\""),
        "first replayed envelope: {:?}",
        first.event
    );
    assert!(
        text_of(&second.event).contains("God says, \"missed two\""),
        "second replayed envelope: {:?}",
        second.event
    );
    assert!(first.seq > last_seq, "replay starts past the resume point");
    assert_eq!(second.seq, first.seq + 1, "the gap is contiguous");

    // The session is live again: the reconnect announce reaches God, and
    // fresh output flows with seqs continuing past the replay.
    god.expect("Nick has reconnected.").await;
    god.send_line("say back with you").await;
    let live = expect_env(&mut ws, "God says, \"back with you\"").await;
    assert!(live.seq > second.seq);

    // The client can still speak.
    send_line(&mut ws, "say resumed fine").await;
    god.expect("Nick says, \"resumed fine\"").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ws_resume_too_old_falls_back_to_rehydrate() {
    // A tiny ring so a burst of output evicts the resume point.
    let rig = boot(
        "resume-old",
        SessionConfig {
            resume_ring: 8,
            ..SessionConfig::default()
        },
        0,
    )
    .await;
    let (_god_handle, mut god) = god_terminal(&rig).await;

    let (token, key) = register(&rig, "nick", "hunter42", "Nick").await;
    let mut ws = connect_ws(&rig, &token, key, "").await.expect("ws connect");
    expect_hello(&mut ws).await;
    let last_seq = expect_env(&mut ws, "Limbo(#0)").await.seq;
    ws.close(None).await.unwrap();
    drop(ws);
    god.expect("Nick has gone link-dead.").await;
    for i in 0..12 {
        god.send_line(&format!("say flood {i}")).await;
    }
    god.expect("God says, \"flood 11\"").await;

    // The gap outgrew the ring: the transport falls back to a fresh attach
    // and says resumed{replayed: false}; fresh snapshots (the room render)
    // follow instead of a replay.
    let mut ws = connect_ws(&rig, &token, key, &format!("&resume={last_seq}"))
        .await
        .expect("ws resume");
    expect_hello(&mut ws).await;
    let resumed = recv_env(&mut ws).await;
    assert!(
        matches!(resumed.event, OutputEvent::Resumed { replayed: false }),
        "expected resumed(replayed=false), got {:?}",
        resumed.event
    );
    let room = expect_env(&mut ws, "Limbo(#0)").await;
    assert!(matches!(room.event, OutputEvent::Room(_)));
    assert!(
        room.seq > last_seq + 12,
        "the fresh render is stamped after the flood ({} vs {last_seq}+12)",
        room.seq
    );
    god.expect("Nick has reconnected.").await;
}

// ---------------------------------------------------------------------------
// Live revocation: logout and @newpassword kill attached web connections
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn logout_kills_the_live_ws() {
    let rig = boot("logout-kill", SessionConfig::default(), 0).await;
    let (token, key) = register(&rig, "nick", "hunter42", "Nick").await;
    let mut ws = connect_ws(&rig, &token, key, "").await.expect("ws connect");
    expect_hello(&mut ws).await;
    expect_env(&mut ws, "Limbo(#0)").await;

    let out = reqwest::Client::new()
        .post(rig.url("/api/auth/logout"))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(out.status(), 204);

    // The attached WS dies: a final Disconnect envelope, then 1001.
    let (code, reason, seen) = recv_close(&mut ws).await;
    assert_eq!(code, 1001);
    assert_eq!(reason, "session closed");
    assert!(
        seen.iter().any(|e| matches!(
            &e.event,
            OutputEvent::Disconnect { reason } if reason.to_plain_text().contains("logged out")
        )),
        "Disconnect(logged out) before the close: {seen:?}"
    );

    // And the token is gone: reconnecting with it is refused pre-upgrade.
    match connect_ws(&rig, &token, key, "").await {
        Err(WsError::Http(resp)) => assert_eq!(resp.status(), 401),
        other => panic!("expected HTTP 401, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn newpassword_boots_the_accounts_web_connections() {
    let rig = boot("newpass-kill", SessionConfig::default(), 0).await;
    let (_god_handle, mut god) = god_terminal(&rig).await;

    let (token, key) = register(&rig, "nick", "hunter42", "Nick").await;
    // A second token for the same account (another device).
    let login: Value = post_json(
        &rig.url("/api/auth/login"),
        json!({"account": "nick", "password": "hunter42"}),
    )
    .await
    .json()
    .await
    .unwrap();
    let token2 = login["token"].as_str().unwrap().to_string();

    let mut ws = connect_ws(&rig, &token, key, "").await.expect("ws connect");
    expect_hello(&mut ws).await;
    expect_env(&mut ws, "Limbo(#0)").await;
    god.expect("Nick has connected.").await;

    let mark = god.mark();
    god.send_line("@newpassword Nick=n3w-pass-9").await;
    god.expect_after(mark, "Password for Nick (account nick) changed")
        .await;
    // Collapse the renderer's word-wrapping before substring checks.
    let confirm = strip_ansi(&god.seen[mark..])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        confirm.contains("2 web session tokens revoked"),
        "both tokens die: {confirm}"
    );
    assert!(
        confirm.contains("1 live web connection booted"),
        "the live WS is booted: {confirm}"
    );

    // The WS saw why, then closed.
    let (code, reason, seen) = recv_close(&mut ws).await;
    assert_eq!(code, 1001);
    assert_eq!(reason, "session closed");
    let texts: Vec<String> = seen.iter().map(|e| text_of(&e.event)).collect();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("Your password has been changed by God.")),
        "notice before the boot: {texts:?}"
    );
    assert!(
        seen.iter().any(|e| matches!(
            &e.event,
            OutputEvent::Disconnect { reason }
                if reason.to_plain_text().contains("password has been changed")
        )),
        "Disconnect carries the reason: {texts:?}"
    );

    // Both tokens are dead; the old password too; the new one works.
    for t in [&token, &token2] {
        match connect_ws(&rig, t, key, "").await {
            Err(WsError::Http(resp)) => assert_eq!(resp.status(), 401),
            other => panic!("expected HTTP 401, got {other:?}"),
        }
    }
    let old = post_json(
        &rig.url("/api/auth/login"),
        json!({"account": "nick", "password": "hunter42"}),
    )
    .await;
    assert_eq!(old.status(), 401);
    let new = post_json(
        &rig.url("/api/auth/login"),
        json!({"account": "nick", "password": "n3w-pass-9"}),
    )
    .await;
    assert_eq!(new.status(), 200);
}

// ---------------------------------------------------------------------------
// The embedded SPA and /healthz
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn spa_and_healthz_are_served() {
    let rig = boot("spa", SessionConfig::default(), 0).await;
    let client = reqwest::Client::new();

    // The app shell at /, no-cache so deploys are picked up.
    let index = client.get(rig.url("/")).send().await.unwrap();
    assert_eq!(index.status(), 200);
    assert!(
        index
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/html")),
        "index is html"
    );
    assert_eq!(
        index
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("no-cache")
    );
    let html = index.text().await.unwrap();
    assert!(html.contains("<div id=\"app\">"), "the SPA shell: {html}");

    // The hashed bundle it references serves immutable.
    let js_path = html
        .split('"')
        .find(|s| s.starts_with("/assets/") && s.ends_with(".js"))
        .expect("index.html references a hashed js bundle")
        .to_string();
    let js = client.get(rig.url(&js_path)).send().await.unwrap();
    assert_eq!(js.status(), 200);
    assert_eq!(
        js.headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("public, max-age=31536000, immutable")
    );
    assert!(
        js.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/javascript")),
        "bundle is javascript"
    );

    // SPA deep links fall back to the shell; the API namespace never does.
    let deep = client.get(rig.url("/some/route")).send().await.unwrap();
    assert_eq!(deep.status(), 200);
    assert!(deep.text().await.unwrap().contains("<div id=\"app\">"));
    let api = client.get(rig.url("/api/nope")).send().await.unwrap();
    assert_eq!(api.status(), 404);

    // /healthz answers with uptime.
    let health: Value = client
        .get(rig.url("/healthz"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["ok"], true);
    assert!(health["uptime_s"].is_u64());
}

// ---------------------------------------------------------------------------
// The M6b web deliverable: a character sheet reaches the web client.
// ---------------------------------------------------------------------------

/// With the wod ruleset loaded, `Query::Sheet` routes to the ruleset's
/// `<ruleset>.sheet` handler (not the pre-M6 "unavailable" bridge stub), so
/// the schema-driven web Sheet panel actually gets data. Regression guard for
/// the web_bridge routing gap the M6b web agent flagged.
#[tokio::test(flavor = "multi_thread")]
async fn sheet_query_routes_to_the_ruleset_over_web() {
    let rig = boot_wod("sheet", SessionConfig::default(), 0).await;
    let (_god_handle, mut god) = god_terminal(&rig).await;

    // The hello manifest advertises the ruleset's sheet feed + "wod" feature,
    // which is what gates the web client into asking for a sheet at all.
    let (token, key) = register(&rig, "mina", "hunter42", "Mina").await;
    let mut ws = connect_ws(&rig, &token, key, "").await.expect("ws connect");
    let (features, panels) = expect_hello(&mut ws).await;
    assert!(
        features.iter().any(|f| f == "wod"),
        "wod feature: {features:?}"
    );
    let sheet_feed = panels
        .iter()
        .find(|p| p.ends_with(".sheet"))
        .cloned()
        .expect("a <ruleset>.sheet feed in the manifest");
    // Drain the entry stream (SessionInfo + room render).
    let _ = expect_env(&mut ws, "Connected as Mina").await;
    let _ = expect_env(&mut ws, "Limbo(#0)").await;
    god.expect("Mina has connected.").await;

    // Staff give Mina a sheet by setting a stat (creates the sheet:wod doc).
    god.send_line("@sheet/set Mina/strength=3").await;
    god.expect("Set Mina/strength").await;

    // The web client asks for its sheet; the reply is the ruleset handler's
    // keyframe, carrying the schema + data — NOT the old "unavailable" Sys.
    send_cmd(
        &mut ws,
        &ClientCommand::Req {
            id: 7,
            q: Query::Sheet,
        },
    )
    .await;
    let env = expect_env(&mut ws, "[reply 7]").await;
    let OutputEvent::Reply { id: 7, event } = &env.event else {
        panic!("expected Reply 7, got {:?}", env.event);
    };
    let OutputEvent::Panel { feed, data, .. } = event.as_ref() else {
        panic!("expected a sheet Panel keyframe, got {event:?}");
    };
    assert_eq!(feed.as_str(), sheet_feed, "reply rides the sheet feed");
    assert!(
        data.get("schema").is_some(),
        "sheet payload carries the schema: {data}"
    );
    assert_eq!(
        data["data"]["strength"], 3,
        "the staff-set strength reached the web sheet: {data}"
    );
}

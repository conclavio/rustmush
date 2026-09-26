//! **The side nets, over the wire** (16b cut Z.4; 16c `N8`; doc 16 §1, §3.7
//! "Boarding", §5).
//!
//! Each side's pilots share one comsys channel for the match — `RedNet`,
//! `BlueNet` — joined at the `Boarding → Running` trigger under their callsign
//! (the alias `ra`, the title `RA`) and left with the seat. The claims this file
//! makes are claims about what real players on real accounts read, so it drives
//! the production composition over **real SSH** and **a real web socket**, with
//! two red pilots and one blue:
//!
//! 1. the join line arrives with the **last** `BOARD`, just ahead of `Good
//!    hunting`, and not before that `BOARD`;
//! 2. `ra contact east` from Ada is read by Cy, red's other pilot, as `[RedNet]
//!    RA Ada: contact east` — comsys's own line, on line mode, and on the web
//!    client as the `Channel` event its channel screen already renders;
//! 3. Bo, on blue, reads **nothing** of it — not on SSH, where a sentinel after
//!    the line closes the window, and not on his web session, where no `Channel`
//!    envelope naming `RedNet` arrives;
//! 4. Bo's `clist` and `@channel/who RedNet` read **byte-identically** before
//!    and after a red `EXIT` — `RedNet` absent, `No such channel.` — which is
//!    16b cut D.1's redaction held on comsys's surfaces too;
//! 5. the `EXIT` itself reads the removal line.
//!
//! # Why this file has its own rig
//!
//! The observer rig's helpers are private to `observer_rule.rs`, and its
//! witnesses sit where one scenario needs them. Moving them into
//! `support/mod.rs` would touch every one of that file's four thousand lines of
//! callers for a second consumer; this rig is `sim_lobby.rs`'s boot (three
//! pilot accounts, the lobby walk) with `observer_rule.rs`'s web listener bolted
//! on, about two hundred lines, and it walks the lobby — `SIM TEAM`, `SIM
//! START`, `BOARD` — because a staff `@arena/start` joins nobody.

mod support;

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use mush_core::module::GameModule;
use mush_core::{Engine, GOD, LIMBO, ObjId, ObjKind};
use mush_llm::{LlmProvider, LlmService, MockProvider};
use mush_proto::{ClientCommand, Envelope, OutputEvent};
use mush_session::{SessionConfig, SessionManager};
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use mush_testkit::text_of;
use mushd::accounts::{Accounts, hash_password};
use mushd::ipguard::IpGuard;
use mushd::modules;
use mushd::persist::load_or_seed_world;
use mushd::ssh_bridge::{SshBridge, SshListener, start_listener};
use mushd::staff::{StaffQueue, register_server_commands};
use mushd::world_task::{self, PersistHandles, ServerDeps};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use support::{STEP_TIMEOUT, Term, play_client, scratch_dir, strip_ansi};

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

const GM_PASSWORD: &str = "gm-secret-1";
const PILOT_PASSWORD: &str = "pilot-secret-1";

/// The world's transient authoritative refusal: a commit is in flight.
const COMMIT_PENDING: &str = "please retry in a moment";
/// `BOARD`'s one transient: the control-lease mirror is refreshed by a 1 Hz tick.
const CONTROL_UNKNOWN: &str = "control state unknown";
/// How many times a line is re-sent on a transient before the test gives up.
const RETRY_BUDGET: usize = 40;

/// The three pilots, created before the world task spawns so their ids are
/// stable: Ada and Cy fly red (`RA`, `RB`), Bo flies blue (`BA`).
const ADA: ObjId = ObjId(2);
const BO: ObjId = ObjId(3);
const CY: ObjId = ObjId(4);

/// comsys's own lines, spelled out: this is a test over a real wire, and the
/// sentence a pilot reads is what it asserts.
const RED_JOINED_ADA: &str = "You have been joined to channel RedNet (alias: ra).";
const RED_JOINED_CY: &str = "You have been joined to channel RedNet (alias: rb).";
const BLUE_JOINED_BO: &str = "You have been joined to channel BlueNet (alias: ba).";
const RED_REMOVED: &str = "You have been removed from channel RedNet.";
/// The one line a net line reads as, on line mode.
const ADA_ON_THE_NET: &str = "[RedNet] RA Ada: contact east";

/// A sentinel whose refusal closes a window: the world answers commands in the
/// order it took them, so everything that reached the terminal before the
/// refusal is in the window and nothing of it is still in flight.
const SENTINEL: &str = "zzq-net-sentinel";
/// The parser's refusal, which is the sentinel's answer.
const HUH: &str = "Huh?";
/// The web client's own sentinel: a `say`, whose answer carries a token no
/// other stream produces.
const WEB_SENTINEL: &str = "zzq-net-web-sentinel";

// ---------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------

struct Server {
    addr: SocketAddr,
    web: SocketAddr,
    dir: PathBuf,
    _listener: SshListener,
    _handles: PersistHandles,
    _web: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Server {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.web)
    }
}

/// A fast arena with a boarding window no test here races: the match starts on
/// the last `BOARD`, never on the deadline.
fn tacsim_config() -> tacsim::TacsimConfig {
    let mut cfg = tacsim::TacsimConfig {
        enabled: true,
        ..tacsim::TacsimConfig::default()
    };
    cfg.arena.tempo.quiet_ms = 100;
    cfg.arena.tempo.alert_ms = 100;
    cfg.arena.tempo.engaged_ms = 100;
    cfg.arena.ledger.summary_ms = 200;
    cfg.match_.startup_ms = 300;
    cfg.match_.board_timeout_ms = 120_000;
    cfg
}

/// The production module list, in production order, with the starter area on.
fn module_list(
    onboarding: &mush_core::OnboardingConfig,
    llm: &Arc<LlmService>,
) -> Vec<Box<dyn GameModule>> {
    let mut mods = modules::server_modules();
    mods.push(modules::worldgen_module(Arc::clone(llm)));
    mods.push(modules::npc_module(modules::npc_agent_adapter()));
    mods.extend(modules::sheet_modules(onboarding, &Default::default()));
    mods.push(modules::tacsim_module(tacsim_config()));
    mods.push(modules::content_module_with(true, true).expect("the content module composes"));
    mods
}

async fn boot(tag: &str) -> Server {
    let dir = scratch_dir(tag);
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    store
        .create_account(AccountRec {
            id: None,
            name: "boss".to_string(),
            argon2_hash: hash_password(GM_PASSWORD).expect("a hash"),
            created: 0,
            characters: vec![CharacterRef {
                id: GOD,
                name: "God".to_string(),
            }],
            ssh_keys: vec![],
            discord: None,
        })
        .await
        .expect("the staff account");

    let mut world = load_or_seed_world(&*store).await.expect("boot world");
    for (id, name) in [(ADA, "Ada"), (BO, "Bo"), (CY, "Cy")] {
        let made = world
            .create(ObjKind::Player, name, GOD, Some(LIMBO))
            .expect("a pilot");
        assert_eq!(made, id, "pilot id assumption");
        store
            .create_account(AccountRec {
                id: None,
                name: name.to_lowercase(),
                argon2_hash: hash_password(PILOT_PASSWORD).expect("a hash"),
                created: i64::from(id.0),
                characters: vec![CharacterRef {
                    id,
                    name: name.to_string(),
                }],
                ssh_keys: vec![],
                discord: None,
            })
            .await
            .expect("a pilot account");
    }

    let onboarding = mush_core::OnboardingConfig {
        chargen_required: false,
        start_room: LIMBO,
        start_room_unapproved: LIMBO,
    };
    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff).expect("server commands");
    let mock: Arc<dyn LlmProvider> = Arc::new(MockProvider::new("openrouter"));
    let llm = Arc::new(
        LlmService::builder()
            .provider("openrouter", mock)
            .global_enabled(false)
            .build(),
    );

    let mut engine =
        Engine::with_modules(world, registry, module_list(&onboarding, &llm)).expect("modules");
    modules::boot_modules(&mut engine, &*store)
        .await
        .expect("module boot");
    // The post-composition upgrade `mush_server::boot` performs: the character →
    // account oracle, without which every claim fails closed.
    let identity: Arc<dyn mush_core::AccountIdentity> = Arc::new(
        mush_server::accounts_identity::hydrate(&*store)
            .await
            .expect("the account map hydrates"),
    );
    engine.set_account_identity(Arc::clone(&identity));
    if let Some(slot) = engine.world.module_state_mut::<mush_sim::AccountOracle>() {
        *slot = mush_sim::AccountOracle::new(identity);
    }
    // The hello manifest, from the one producer `main.rs` builds it with.
    let (features, panels) = mush_server::boot::build_hello_manifest(&engine);

    let sessions = Arc::new(SessionManager::with_config(SessionConfig::default()));
    let guard = Arc::new(IpGuard::new());
    let metrics = Arc::new(mush_net_web::Metrics::new());
    let handles = world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        ServerDeps {
            store: Arc::clone(&store),
            guard: Arc::clone(&guard),
            staff,
            llm: Some(Arc::clone(&llm)),
            quarantine_spool: mushd::quarantine::QuarantineSpool::in_memory(),
            metrics: Arc::clone(&metrics),
            outbox_adapters: Vec::new(),
        },
    );
    let accounts = Arc::new(Accounts::new(Arc::clone(&store), handles.handle.clone(), 6));
    let bridge = Arc::new(SshBridge::new(
        Arc::clone(&accounts),
        Arc::clone(&sessions),
        handles.handle.clone(),
        Arc::clone(&guard),
    ));
    let listener = start_listener(
        "127.0.0.1:0".parse().expect("a loopback address"),
        &dir,
        "Welcome to the simulator.".to_string(),
        bridge,
    )
    .await
    .expect("ssh listener");
    let addr = listener.addr();

    let web_bridge: Arc<dyn mush_net_web::WebHooks> = Arc::new(
        mushd::web_bridge::WebBridge::new(
            accounts,
            sessions,
            handles.handle.clone(),
            guard,
            Arc::clone(&store),
            panels.clone(),
        )
        .with_metrics(Arc::clone(&metrics)),
    );
    let web_config = mush_net_web::WebConfig {
        server_name: "rustmush nets".to_string(),
        features,
        panels,
        metrics_enabled: true,
        ..mush_net_web::WebConfig::default()
    };
    let web_listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("a web port");
    let web = web_listener.local_addr().expect("the web address");
    let app = mush_net_web::router(web_bridge, web_config);
    let web_task = tokio::spawn(async move {
        let _ = axum::serve(
            web_listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });

    Server {
        addr,
        web,
        dir,
        _listener: listener,
        _handles: handles,
        _web: web_task,
    }
}

// ---------------------------------------------------------------------------
// The SSH half
// ---------------------------------------------------------------------------

async fn login(
    addr: SocketAddr,
    account: &str,
    password: &str,
    character: &str,
) -> (russh::client::Handle<support::AcceptAll>, Term) {
    let (handle, mut term) = play_client(addr).await;
    term.expect("Welcome to the simulator.").await;
    term.send_line(&format!("connect {account} {password}"))
        .await;
    term.expect(&format!("Connected as {character}")).await;
    (handle, term)
}

/// Run one line and wait for `needle`, retrying the two transients.
async fn say(term: &mut Term, line: &str, needle: &str) {
    assert!(
        !line.contains(needle),
        "`{needle}` appears in `{line}`'s own echo; pick a needle from the ANSWER"
    );
    for _ in 0..RETRY_BUDGET {
        let mark = term.mark();
        term.send_line(line).await;
        match term
            .expect_one_of_after(mark, &[needle, COMMIT_PENDING, CONTROL_UNKNOWN])
            .await
        {
            0 => return,
            _ => tokio::time::sleep(Duration::from_millis(200)).await,
        }
    }
    panic!("`{line}` was refused as transient {RETRY_BUDGET} times running");
}

/// Run one line, wait for `needle`, and hand back everything that arrived.
async fn ask(term: &mut Term, line: &str, needle: &str) -> String {
    let mark = term.mark();
    say(term, line, needle).await;
    strip_ansi(&term.seen[mark..])
}

/// **Everything the terminal received from `mark` to the sentinel's refusal** —
/// a whole window, never half of one.
async fn close_window(term: &mut Term, mark: usize) -> String {
    for _ in 0..RETRY_BUDGET {
        let at = term.mark();
        term.send_line(SENTINEL).await;
        if term.expect_one_of_after(at, &[HUH, COMMIT_PENDING]).await == 0 {
            return strip_ansi(&term.seen[mark..]);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("the sentinel was never answered")
}

/// **One line's whole answer**: the line, then the sentinel, and everything
/// between.
async fn ask_whole(term: &mut Term, line: &str) -> String {
    let mark = term.mark();
    term.send_line(line).await;
    close_window(term, mark).await
}

/// **What blue can read about red's net** (16b cut Z.4): the `clist` rows and
/// the `@channel/who RedNet` answer, cut out of their windows so the echo of
/// the typed line and the sentinel's refusal are not part of the comparison —
/// and so a cockpit notice that happens to arrive in the window cannot make two
/// identical answers differ.
async fn blue_view_of_red(bo: &mut Term) -> (Vec<String>, Vec<String>) {
    let clist = ask_whole(bo, "clist").await;
    let rows: Vec<String> = clist
        .lines()
        .map(str::trim_end)
        .skip_while(|line| !line.starts_with("Channel "))
        .take_while(|line| !line.starts_with("-- "))
        .map(str::to_owned)
        .collect();
    assert!(!rows.is_empty(), "no clist table in: {clist}");
    let who = ask_whole(bo, "@channel/who RedNet").await;
    let answer: Vec<String> = who
        .lines()
        .map(str::trim_end)
        .filter(|line| line.contains("No such channel") || line.contains("RedNet"))
        .filter(|line| !line.contains("@channel/who"))
        .map(str::to_owned)
        .collect();
    (rows, answer)
}

// ---------------------------------------------------------------------------
// The web half
// ---------------------------------------------------------------------------

async fn post_json(url: &str, body: Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(url)
        .json(&body)
        .send()
        .await
        .expect("the web accepts a post")
}

async fn connect_ws(server: &Server, token: &str, char_key: u64) -> Result<Ws, WsError> {
    let mut req = format!("ws://{}/ws?char={char_key}", server.web)
        .into_client_request()
        .expect("a ws request");
    req.headers_mut().insert(
        "sec-websocket-protocol",
        HeaderValue::from_str(&format!("rustmush.v1.{token}")).expect("a subprotocol"),
    );
    connect_async(req).await.map(|(ws, _resp)| ws)
}

async fn recv_env(ws: &mut Ws) -> Envelope {
    loop {
        let msg = tokio::time::timeout(STEP_TIMEOUT, ws.next())
            .await
            .expect("timed out waiting for a frame")
            .expect("the stream ended")
            .expect("a ws error");
        match msg {
            Message::Text(text) => {
                return serde_json::from_str(text.as_str()).expect("an envelope");
            }
            Message::Close(frame) => panic!("unexpected close: {frame:?}"),
            _ => {}
        }
    }
}

async fn send_web(ws: &mut Ws, line: &str) {
    ws.send(Message::Text(
        serde_json::to_string(&ClientCommand::Line(line.to_string()))
            .expect("a command")
            .into(),
    ))
    .await
    .expect("the socket takes a line");
}

/// Open a web session for an existing account and drain its entry stream.
async fn web_session(server: &Server, account: &str, password: &str, character: &str) -> Ws {
    let login = post_json(
        &server.url("/api/auth/login"),
        json!({ "account": account, "password": password }),
    )
    .await;
    assert_eq!(login.status(), 200, "{account} must be able to log in");
    let body: Value = login.json().await.expect("a login body");
    let token = body["token"].as_str().expect("a token").to_string();
    let key = body["characters"][0]["key"].as_u64().expect("a char key");
    let mut ws = connect_ws(server, &token, key)
        .await
        .expect("the web socket connects");
    let hello = recv_env(&mut ws).await;
    assert_eq!(hello.seq, 0, "hello is out of band");
    assert!(matches!(hello.event, OutputEvent::Hello { .. }));
    for _ in 0..10 {
        let env = recv_env(&mut ws).await;
        if text_of(&env.event).contains(&format!("Connected as {character}")) {
            break;
        }
    }
    ws
}

/// **Every envelope the socket is sent up to its own sentinel's echo**, with
/// the keepalive answered on the way: the whole window since the last read.
async fn web_window(ws: &mut Ws) -> Vec<Envelope> {
    send_web(ws, &format!("say {WEB_SENTINEL}")).await;
    let mut seen = Vec::new();
    for _ in 0..4_000 {
        let env = recv_env(ws).await;
        if let OutputEvent::Ping { nonce } = &env.event {
            let pong = ClientCommand::Pong { nonce: *nonce };
            ws.send(Message::Text(
                serde_json::to_string(&pong).expect("a pong encodes").into(),
            ))
            .await
            .expect("the socket takes a pong");
            continue;
        }
        let done = text_of(&env.event).contains(WEB_SENTINEL);
        seen.push(env);
        if done {
            return seen;
        }
    }
    panic!("the web client never answered its sentinel")
}

/// Whether an envelope is a channel line on `channel`.
fn is_channel_line(env: &Envelope, channel: &str) -> bool {
    matches!(&env.event, OutputEvent::Channel { channel: on, .. } if on.as_str() == channel)
}

// ---------------------------------------------------------------------------
// The scenario
// ---------------------------------------------------------------------------

/// One object's dbref, out of an `@find` listing.
fn dbref_of(listing: &str, name: &str) -> u32 {
    listing
        .lines()
        .find_map(|line| {
            let rest = line.split_once(name)?.1;
            let inside = rest.split_once("(#")?.1;
            inside.split(')').next()?.trim().parse::<u32>().ok()
        })
        .unwrap_or_else(|| panic!("no dbref for {name:?} in:\n{listing}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sides_net_carries_its_pilots_words_to_its_own_side_and_nothing_to_the_other() {
    let server = boot("sim-net").await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, mut cy) = login(server.addr, "cy", PILOT_PASSWORD, "Cy").await;
    let mut red_web = web_session(&server, "cy", PILOT_PASSWORD, "Cy").await;
    let mut blue_web = web_session(&server, "bo", PILOT_PASSWORD, "Bo").await;

    // Everybody into their team room BEFORE the match starts, for the reason
    // `sim_lobby.rs`' `d3_a_lead_concedes_and_the_board_names_the_winner` gives.
    let listing = ask(&mut god, "@find Red Room", "END OF LIST").await;
    let red_room = dbref_of(&listing, "Red Room");
    let listing = ask(&mut god, "@find Blue Room", "END OF LIST").await;
    let blue_room = dbref_of(&listing, "Blue Room");
    for (pilot, room) in [(ADA, red_room), (CY, red_room), (BO, blue_room)] {
        say(
            &mut god,
            &format!("@teleport #{}=#{room}", pilot.0),
            "Teleported",
        )
        .await;
    }

    // Before any match, blue's view of red's net is already nothing.
    let before = blue_view_of_red(&mut bo).await;
    assert!(
        !before.0.iter().any(|row| row.contains("RedNet")),
        "{before:?}"
    );
    assert_eq!(before.1, vec!["No such channel.".to_owned()]);
    let bo_start = bo.mark();

    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut cy, "SIM TEAM red", "takes red slot 2").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
    let ada_mark = ada.mark();
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "BOARD", "climb into").await;
    say(&mut cy, "BOARD", "climb into").await;
    // (1) Two of three aboard: nobody is on a net yet.
    let early = close_window(&mut ada, ada_mark).await;
    assert!(!early.contains("You have been joined"), "{early}");
    say(&mut bo, "BOARD", "climb into").await;
    // The join rides the check-out's batch ahead of `Good hunting`, so the
    // line every client waits on to know the match is theirs comes last.
    ada.expect("Good hunting").await;
    assert!(strip_ansi(&ada.seen[ada_mark..]).contains(RED_JOINED_ADA));
    cy.expect(RED_JOINED_CY).await;
    bo.expect(BLUE_JOINED_BO).await;
    let ada_seen = strip_ansi(&ada.seen[ada_mark..]);
    let hunting = ada_seen.find("Good hunting").expect("the start");
    let joined = ada_seen.find(RED_JOINED_ADA).expect("the join");
    assert!(
        joined < hunting,
        "the join is said ahead of the start's last line:\n{ada_seen}"
    );
    // Nobody is told of the other side's net.
    let bo_window = close_window(&mut bo, bo_start).await;
    assert!(!bo_window.contains("RedNet"), "{bo_window}");
    assert!(bo_window.contains(BLUE_JOINED_BO), "{bo_window}");
    let _ = web_window(&mut blue_web).await;
    let _ = web_window(&mut red_web).await;

    // (2) Ada speaks on red's net, and Cy reads comsys's own line.
    let bo_mark = bo.mark();
    let cy_mark = cy.mark();
    say(&mut ada, "ra contact east", ADA_ON_THE_NET).await;
    cy.expect_after(cy_mark, ADA_ON_THE_NET).await;
    // ... and on the web, as the `Channel` event the channel screen renders.
    let red_seen = web_window(&mut red_web).await;
    assert!(
        red_seen.iter().any(|env| matches!(
            &env.event,
            OutputEvent::Channel { channel, title, actor, text, .. }
                if channel.as_str() == "RedNet"
                    && title.as_deref() == Some("RA")
                    && actor.name == "Ada"
                    && text.to_plain_text() == "contact east"
        )),
        "red's web session did not receive the line: {:?}",
        red_seen
            .iter()
            .map(|env| text_of(&env.event))
            .collect::<Vec<_>>()
    );
    // (3) Bo reads nothing of it, on either wire.
    let bo_window = close_window(&mut bo, bo_mark).await;
    assert!(!bo_window.contains("RedNet"), "{bo_window}");
    assert!(!bo_window.contains("contact east"), "{bo_window}");
    let blue_seen = web_window(&mut blue_web).await;
    assert!(
        !blue_seen.iter().any(|env| is_channel_line(env, "RedNet")),
        "a Channel envelope naming RedNet reached blue's web session"
    );
    assert!(
        !blue_seen
            .iter()
            .any(|env| text_of(&env.event).contains("contact east")),
        "red's words reached blue's web session"
    );

    // (4) and (5): a red pilot climbs out, reads the removal line, and blue's
    // view of red's net does not move by a byte.
    // (Blue's own clist grew its own net's row at the start; red's is absent.)
    let during = blue_view_of_red(&mut bo).await;
    assert!(
        !during.0.iter().any(|row| row.contains("RedNet")),
        "{during:?}"
    );
    assert!(
        during.0.iter().any(|row| row.contains("BlueNet")),
        "{during:?}"
    );
    assert_eq!(during.1, before.1, "blue reads red's net mid-match");
    say(&mut ada, "EXIT", "You climb out of").await;
    ada.expect(RED_REMOVED).await;
    let after = blue_view_of_red(&mut bo).await;
    assert_eq!(after, during, "a red EXIT changed what blue can read");
    let blue_seen = web_window(&mut blue_web).await;
    assert!(
        !blue_seen.iter().any(|env| is_channel_line(env, "RedNet")),
        "a Channel envelope naming RedNet reached blue's web session"
    );
    // Cy is still on red's net, and still reads it.
    let cy_mark = cy.mark();
    say(&mut cy, "rb still here", "[RedNet] RB Cy: still here").await;
    let cy_window = close_window(&mut cy, cy_mark).await;
    assert!(
        cy_window.contains("[RedNet] RB Cy: still here"),
        "{cy_window}"
    );
}

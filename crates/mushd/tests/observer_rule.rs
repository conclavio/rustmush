//! **The observer rule, surface by surface** (16b cut S.2; doc 16 §1 A1's
//! observer assertions, §3.6 "World-side exclusion" and "What a pilot sees", §8).
//!
//! Doc 16 §1 is explicit that this is *never asserted as one blanket claim*: it
//! puts a witness in every seat the world offers and asks each of them
//! separately. So this file boots the production composition once, over **real
//! SSH** and **a real web socket**, and seats five witnesses:
//!
//! | witness | seat | surface |
//! |---|---|---|
//! | Cy | a bystander, standing in the muster room | `look` |
//! | God | a staffer, standing in the muster room | `look`, `@unit/show`, `@unit/status`, `@spatial`, `@domain`, `+map` |
//! | Bo | the pilot on the other side | `CONTACTS`, `VIEW` |
//! | Ada | the pilot whose tank it is | `@unit/status` on her own deployed tank |
//! | God again | the same account through the web client | `look`, `@unit/show`, `+map` |
//!
//! Every one of them is asked the **same question twice**: once before the match,
//! when the tanks are ordinary world objects and every surface names them — which
//! is what makes the second answer mean something — and once while the arena holds
//! them, when nothing about them may cross on any surface.
//!
//! Then the two pilots drive toward each other until S.1's sensor pass detects,
//! and the last assertion is the other half of doc 16 §3.6: what the pilot *does*
//! see is exactly the contact and nothing more — an opaque id, a class, a bearing,
//! a band and an age, and no name, no object number and no exact range.
//!
//! **16b cut W.3 added the other half of the rule**: it holds through the
//! *shooting* as well as through the approach.
//! [`nothing_of_the_shooting_crosses_to_the_world_or_to_the_target`] puts two
//! rounds from the red tank into the blue one at forty metres and asks the same
//! witnesses the same questions while it happens — and asks the **target's own
//! pilot** too, who holds a contact on its attacker and is still told nothing at
//! all about the round that hit it.
//!
//! # Why the tanks are moved before the match starts
//!
//! The shipped proving ground musters red at cell 6 and blue at cell 249, 2.4 km
//! apart, and the stock sensor reaches 600 m. Driving that distance is a minute of
//! wall time for nothing: the property under test is *detection happens when the
//! range closes*, not *a tank can cross a map*. So staff re-pose the two slot-1
//! tanks 70 cells apart on the east-west road before boarding —
//! `arena_def::opening_pose` honours a world pose that is on the window and
//! passable — which leaves them out of every reach at the opening pulse and a few
//! seconds of driving away from the first one.

mod support;

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use mush_core::module::GameModule;
use mush_core::{Engine, GOD, LIMBO, ObjId, ObjKind};
use mush_llm::{LlmProvider, LlmService, MockProvider};
use mush_proto::{ClientCommand, Envelope, OutputEvent, Query};
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

use support::{STEP_TIMEOUT, Term, names_side, play_client, scratch_dir, strip_ansi};

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

const GM_PASSWORD: &str = "gm-secret-1";
const PILOT_PASSWORD: &str = "pilot-secret-1";

/// The world's transient authoritative refusal: a commit is in flight.
const COMMIT_PENDING: &str = "please retry in a moment";
/// `BOARD`'s one transient: the control-lease mirror is refreshed by a 1 Hz tick.
const CONTROL_UNKNOWN: &str = "control state unknown";
/// How many times a line is re-sent on a transient before the test gives up.
const RETRY_BUDGET: usize = 40;
/// How many times a poll waits for the physics: 120 × 250 ms is half a minute,
/// which is four times what the two tanks need to close 70 cells.
const DRIVE_BUDGET: usize = 120;

/// The two pilots and the bystander, created before the world task spawns so
/// their ids are stable.
const ADA: ObjId = ObjId(2);
const BO: ObjId = ObjId(3);
const CY: ObjId = ObjId(4);

/// The arena's muster room, as the shipped pack labels it.
const MUSTER: &str = "The Proving Ground";
/// Slot 1 of each side, as the shipped pack names them.
const RED_TANK: &str = "Red Tank 1";
const BLUE_TANK: &str = "Blue Tank 1";

/// What `CONTACTS` says when the platform holds nothing.
const NO_CONTACTS: &str = "No contacts.";
/// The existence-hiding line `@unit/status` answers a viewer with no station.
const NO_STATION: &str = "You have no station on that unit.";
/// What `@unit/show` answers for an object it has no unit for.
const NOT_A_UNIT: &str = "It is not a tactical unit.";
/// What `@spatial` answers for an object it has no placement for.
const NOT_SPATIAL: &str = "not a spatial entity.";
/// **The one refusal for a contact id the platform does not hold** (doc 16 §3.6:
/// *unresolvable, unowned or expired* are one answer). Spelled out rather than
/// imported, so a test over SSH asserts the sentence a pilot reads and not a
/// constant the code could rename underneath it.
const NO_SUCH_CONTACT: &str = "No such contact.";

/// The cell each slot-1 tank is re-posed to: 70 cells apart on the east-west
/// road, which is outside the stock sensor's 60 cells at every signature factor.
const RED_CELL: (i64, i64) = (90, 97);
const BLUE_CELL: (i64, i64) = (160, 97);

// ---------------------------------------------------------------------------
// The rig — `sim_cockpit.rs`' boot with `web_e2e.rs`' listener bolted on
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

/// A fast arena, as `sim_cockpit.rs` configures one.
///
/// **16b cut Q.1**: `designate` is `[tacsim.sensors].designate_on_detection`, the
/// one key that relaxes the observer rule — it names a contact by its team and
/// centre slot at the first pass instead of by this platform's own provisional
/// number. It is a parameter rather than a constant because the key is shipped
/// into the rendered instance template and an operator can turn it on, and a
/// surface nothing has ever run end to end is a surface nobody has checked.
fn tacsim_config(designate: bool) -> tacsim::TacsimConfig {
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
    cfg.sensors.designate_on_detection = designate;
    cfg
}

/// The production module list, in production order, with the starter area on.
fn module_list(
    onboarding: &mush_core::OnboardingConfig,
    llm: &Arc<LlmService>,
    designate: bool,
) -> Vec<Box<dyn GameModule>> {
    let mut mods = modules::server_modules();
    mods.push(modules::worldgen_module(Arc::clone(llm)));
    mods.push(modules::npc_module(modules::npc_agent_adapter()));
    mods.extend(modules::ruleset_modules(onboarding, &Default::default()));
    mods.push(modules::tacsim_module(tacsim_config(designate)));
    mods.push(modules::content_module_with(true, true).expect("the content module composes"));
    mods
}

/// The shipped composition, with the observer rule at its default strength.
async fn boot(tag: &str) -> Server {
    boot_with(tag, false).await
}

/// The same composition over a named `[tacsim.sensors].designate_on_detection`
/// (16b cut Q.1).
async fn boot_with(tag: &str, designate: bool) -> Server {
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
            .expect("a character");
        assert_eq!(made, id, "character id assumption");
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
            .expect("a character account");
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
        Engine::with_modules(world, registry, module_list(&onboarding, &llm, designate))
            .expect("modules");
    modules::boot_modules(&mut engine, &*store)
        .await
        .expect("module boot");
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
            store,
            panels.clone(),
        )
        .with_metrics(Arc::clone(&metrics)),
    );
    let web_config = mush_net_web::WebConfig {
        server_name: "rustmush observer".to_string(),
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

/// **A whole answer, never half of one.**
///
/// `look`'s room render is one event but several lines, and a needle taken from
/// its first line can return before the contents line has arrived — which would
/// make an absence assertion pass for the wrong reason. So the line is followed by
/// a deliberate nonsense command, and the slice runs to *its* refusal: the world
/// answers commands in the order it took them, so everything `line` produced is in
/// the slice and nothing of it is still in flight.
const SENTINEL: &str = "zzq-observer-sentinel";
/// The parser's refusal, which is the sentinel's answer.
const HUH: &str = "Huh?";

/// The escape sequence a full-screen client sends to take the alternate screen.
const ALT_SCREEN: &str = "\u{1b}[?1049h";

async fn ask_whole(term: &mut Term, line: &str) -> String {
    for _ in 0..RETRY_BUDGET {
        let mark = term.mark();
        term.send_line(line).await;
        term.send_line(SENTINEL).await;
        // The sentinel is a parser refusal and needs no authority, but it rides
        // the same lane: a world mid-commit answers BOTH lines with the transient
        // and the `Huh?` never comes. That is a retry, not a failure.
        if term.expect_one_of_after(mark, &[HUH, COMMIT_PENDING]).await == 0 {
            return strip_ansi(&term.seen[mark..]);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("`{line}` never produced a whole answer")
}

/// One object's dbref, out of an `@find` listing or a `@domain` row.
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

/// The domain id whose host is `host`, out of an `@domain` listing.
fn domain_of(listing: &str, host: &str) -> u64 {
    listing
        .lines()
        .find(|line| line.contains(&format!("host {host}")))
        .and_then(|line| {
            let rest = line.split_once('#')?.1;
            rest.split_whitespace().next()?.parse::<u64>().ok()
        })
        .unwrap_or_else(|| panic!("no domain hosted on {host:?} in:\n{listing}"))
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

/// Read envelopes until one's rendered text contains `needle`, and hand back
/// **everything** that arrived — the whole answer, so an assertion can be made
/// about what was *not* in it.
async fn ask_web(ws: &mut Ws, line: &str, needle: &str) -> String {
    send_web(ws, line).await;
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..200 {
        let env = recv_env(ws).await;
        let text = text_of(&env.event);
        let done = text.contains(needle);
        seen.push(text);
        if done {
            return seen.join("\n");
        }
    }
    panic!(
        "no envelope containing {needle:?} after `{line}` in:\n{}",
        seen.join("\n")
    )
}

/// **The web client's own sentinel.**
///
/// The `Huh?` refusal is the same sentence whoever provoked it, and this account is
/// connected twice — every answer the staffer's SSH terminal asks for is delivered
/// to this socket too. So the web's sentinel is a `say`, whose *answer* carries a
/// token no other stream can produce, and the reader stops on that.
const WEB_SENTINEL: &str = "zzq-web-sentinel";

/// **The web client's room render, and only that.**
///
/// Collecting texts would mix the staffer's own earlier sentences into an absence
/// assertion — see [`WEB_SENTINEL`] — so this keeps the last
/// [`OutputEvent::Room`] before the sentinel's echo. The world answers commands in
/// the order it took them, so that render is the answer to *this* `look`.
async fn look_web(ws: &mut Ws) -> String {
    send_web(ws, "look").await;
    send_web(ws, &format!("say {WEB_SENTINEL}")).await;
    let mut room = String::new();
    for _ in 0..400 {
        let env = recv_env(ws).await;
        let text = text_of(&env.event);
        if matches!(env.event, OutputEvent::Room(_)) {
            room = text.clone();
        }
        if text.contains(WEB_SENTINEL) {
            return room;
        }
    }
    panic!("the web client never answered the sentinel")
}

// ---------------------------------------------------------------------------
// The two arena feeds, as a structured client sees them (16b cut Q.3)
// ---------------------------------------------------------------------------

/// The cockpit feed, as doc 16 §5 names it. Spelled out rather than imported,
/// for the reason [`NO_SUCH_CONTACT`] is: this is a test over a real wire.
const COCKPIT_FEED: &str = "arena.cockpit";
/// The terrain feed, which is only ever the answer to a query.
const MAP_FEED: &str = "arena.map";

/// What either feed answers somebody who is not flying.
fn unseated(feed: &str) -> Value {
    match feed {
        MAP_FEED => json!({
            "schema": {
                "id": "arena.map",
                "version": 1,
                "shape": "terrain",
                "retain": "latest",
            },
            "seated": false,
        }),
        _ => json!({
            "schema": { "id": "arena.cockpit", "version": 1, "retain": "latest" },
            "seated": false,
        }),
    }
}

/// One `PanelSubscribe` over a real socket, answered by the registered query
/// handler and correlated by `id`.
async fn panel_query(ws: &mut Ws, id: u64, feed: &str) -> Value {
    let request = ClientCommand::Req {
        id,
        q: Query::PanelSubscribe { feed: feed.into() },
    };
    ws.send(Message::Text(
        serde_json::to_string(&request)
            .expect("a request encodes")
            .into(),
    ))
    .await
    .expect("the socket takes a request");
    for _ in 0..400 {
        let env = recv_env(ws).await;
        let OutputEvent::Reply {
            id: answered,
            event,
        } = &env.event
        else {
            continue;
        };
        if *answered != id {
            continue;
        }
        match event.as_ref() {
            OutputEvent::Panel {
                feed: answered_feed,
                data,
                ..
            } => {
                assert_eq!(answered_feed.as_str(), feed, "answered on its own feed");
                return data.clone();
            }
            other => panic!("a {feed} subscribe must answer a keyframe, got {other:?}"),
        }
    }
    panic!("no reply {id} for {feed}")
}

/// The next `arena.cockpit` keyframe this socket is **pushed**, which is the
/// per-pulse frame the arena builds for this pilot.
///
/// The app-level keepalive is answered along the way: this is the one reader in
/// the file that may wait longer than `ping_interval_ms` without the test
/// sending anything, and an unanswered ping closes the socket.
async fn pushed_cockpit(ws: &mut Ws) -> Value {
    for _ in 0..600 {
        let env = recv_env(ws).await;
        match &env.event {
            OutputEvent::Panel { feed, data, .. } if feed.as_str() == COCKPIT_FEED => {
                return data.clone();
            }
            OutputEvent::Ping { nonce } => {
                let pong = ClientCommand::Pong { nonce: *nonce };
                ws.send(Message::Text(
                    serde_json::to_string(&pong).expect("a pong encodes").into(),
                ))
                .await
                .expect("the socket takes a pong");
            }
            _ => {}
        }
    }
    panic!("the arena never pushed a cockpit frame")
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
    // Hello (seq 0), then the session line, then the room.
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

// ---------------------------------------------------------------------------
// The scenario
// ---------------------------------------------------------------------------

/// Teleport the pilots into the lobby and the two watchers into the muster room.
async fn seat_the_witnesses(god: &mut Term) -> (u32, u32, u32, u64) {
    let listing = ask(god, "@find Simulator Lobby", "END OF LIST").await;
    let lobby = dbref_of(&listing, "Simulator Lobby");
    for pilot in [ADA, BO] {
        say(
            god,
            &format!("@teleport #{}=#{lobby}", pilot.0),
            "Teleported",
        )
        .await;
    }
    let listing = ask(god, &format!("@find {RED_TANK}"), "END OF LIST").await;
    let red = dbref_of(&listing, RED_TANK);
    let listing = ask(god, &format!("@find {BLUE_TANK}"), "END OF LIST").await;
    let blue = dbref_of(&listing, BLUE_TANK);
    // The muster room is the room the arena applier made and hosts the domain on,
    // and it is where the tanks stand.
    let listing = ask(god, "@domain", &format!("host {MUSTER}")).await;
    let domain = domain_of(&listing, MUSTER);
    let listing = ask(god, &format!("@find {MUSTER}"), "END OF LIST").await;
    let muster = dbref_of(&listing, MUSTER);
    say(god, &format!("@teleport #{}=#{muster}", CY.0), "Teleported").await;
    // Teleporting YOURSELF renders the room you land in.
    say(god, &format!("@teleport #{}=#{muster}", GOD.0), MUSTER).await;
    (red, blue, muster, domain)
}

/// Re-pose the two slot-1 tanks 70 cells apart on the road (see the module head).
async fn close_the_gap(god: &mut Term, red: u32, blue: u32, domain: u64) {
    for (unit, (x, y), heading) in [(red, RED_CELL, 90_000), (blue, BLUE_CELL, 270_000)] {
        let pose = format!(
            "{{\"x_cm\":{},\"y_cm\":{},\"heading_mdeg\":{heading}}}",
            x * 1_000 + 500,
            y * 1_000 + 500
        );
        say(
            god,
            &format!("@spatial/place/domain #{unit}={domain},{pose}"),
            "Placed",
        )
        .await;
    }
}

/// Claim, start, walk and board: the state the match runs from.
async fn take_off(ada: &mut Term, bo: &mut Term) {
    say(ada, "SIM TEAM red", "takes red slot 1").await;
    say(bo, "SIM TEAM blue", "takes blue slot 1").await;
    say(ada, "SIM START", "is boarding").await;
    say(ada, "red", "Red Room").await;
    say(bo, "blue", "Blue Room").await;
    say(ada, "BOARD", "climb into").await;
    say(bo, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;
}

/// Bring a tank's power up and point it at the other one.
async fn drive(term: &mut Term, heading: u32) {
    say(term, "STARTUP", "Startup sequence").await;
    for _ in 0..RETRY_BUDGET {
        let mark = term.mark();
        term.send_line(&format!("HEADING {heading}")).await;
        if term
            .expect_one_of_after(
                mark,
                &[
                    &format!("Steering to {heading}"),
                    "Power not online",
                    // 16b cut F.1: the same refusal while the startup is running.
                    "Power is still coming up",
                ],
            )
            .await
            == 0
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    say(term, "SPEED 54", "Making 54 kph").await;
}

/// Everything a surface must never say about a unit an arena holds.
fn names_nothing(seen: &str, red: u32, blue: u32, what: &str) {
    for leak in [
        RED_TANK.to_string(),
        BLUE_TANK.to_string(),
        format!("#{red}"),
        format!("#{blue}"),
    ] {
        assert!(!seen.contains(&leak), "{what} leaked {leak:?}:\n{seen}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_about_an_undetected_unit_crosses_on_any_surface() {
    let server = boot("observer-rule").await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, mut cy) = login(server.addr, "cy", PILOT_PASSWORD, "Cy").await;
    let mut web = web_session(&server, "boss", GM_PASSWORD, "God").await;

    let (red, blue, _muster, domain) = seat_the_witnesses(&mut god).await;

    // -----------------------------------------------------------------
    // The control: before the match, every surface names the tanks.
    // -----------------------------------------------------------------
    let seen = ask_whole(&mut god, "look").await;
    assert!(
        seen.contains(RED_TANK) && seen.contains(BLUE_TANK),
        "the staffer's `look` names the tanks before the match:\n{seen}"
    );
    let seen = ask_whole(&mut cy, "look").await;
    assert!(
        seen.contains(RED_TANK),
        "and so does the bystander's:\n{seen}"
    );
    let seen = ask(&mut god, &format!("@unit/show #{red}"), "mobility").await;
    assert!(
        seen.contains(RED_TANK) && seen.contains("revision"),
        "`@unit/show` inspects it:\n{seen}"
    );
    let seen = ask(&mut god, &format!("@spatial #{red}"), "domain").await;
    assert!(!seen.contains(NOT_SPATIAL), "`@spatial` places it:\n{seen}");
    let seen = ask(&mut god, "@domain", &format!("host {MUSTER}")).await;
    assert!(
        seen.contains(&format!("#{domain} ")),
        "`@domain` lists the muster:\n{seen}"
    );
    let seen = look_web(&mut web).await;
    assert!(
        seen.contains(RED_TANK),
        "and the web client's room render names them:\n{seen}"
    );

    // -----------------------------------------------------------------
    // The match. Two tanks, seventy cells apart, under custody.
    // -----------------------------------------------------------------
    close_the_gap(&mut god, red, blue, domain).await;
    take_off(&mut ada, &mut bo).await;

    // The placement took, which is what makes "before detection" true.
    let seen = ask(&mut ada, "STATUS", "m east").await;
    assert!(
        seen.contains(&format!("at {} m east", RED_CELL.0 * 10 + 5)),
        "the tank opened where staff re-posed it:\n{seen}"
    );

    // -----------------------------------------------------------------
    // Surface by surface, while the arena holds them and before detection.
    // -----------------------------------------------------------------
    let seen = ask_whole(&mut god, "look").await;
    assert!(
        seen.contains(MUSTER),
        "the staffer really looked:
{seen}"
    );
    names_nothing(&seen, red, blue, "the staffer's `look`");
    let seen = ask_whole(&mut cy, "look").await;
    assert!(
        seen.contains(MUSTER),
        "the bystander really looked:
{seen}"
    );
    names_nothing(&seen, red, blue, "the bystander's `look`");
    assert!(
        seen.contains("Red Tank 2"),
        "and the six tanks nobody deployed are still there, which is what makes          the two that are gone a fact about custody:
{seen}"
    );

    let seen = ask(&mut god, &format!("@unit/show #{red}"), NOT_A_UNIT).await;
    assert!(seen.contains(NOT_A_UNIT), "{seen}");
    let seen = ask(&mut god, &format!("@spatial #{red}"), NOT_SPATIAL).await;
    assert!(seen.contains(NOT_SPATIAL), "{seen}");
    // The muster is the only domain this world has, so the listing is empty — and
    // `@domain/status` on it answers the sentence a domain that never existed gets.
    let seen = ask(&mut god, "@domain", "No domains.").await;
    assert!(
        !seen.contains(&format!("host {MUSTER}")),
        "`@domain` still lists the muster:\n{seen}"
    );
    let seen = ask(
        &mut god,
        &format!("@domain/status {domain}"),
        &format!("No domain #{domain}."),
    )
    .await;
    assert!(seen.contains("No domain"), "{seen}");
    let seen = ask(&mut god, &format!("@unit/status #{red}"), NO_STATION).await;
    assert!(
        seen.contains(NO_STATION),
        "a staffer holds no station, so the answer is the existence-hiding one:\n{seen}"
    );
    let seen = ask(&mut god, "+map", "not on a map").await;
    names_nothing(&seen, red, blue, "the staffer's `+map`");

    // Doc 16 §3.6: *"deployed" to a viewer stationed on it*.
    let seen = ask(&mut ada, &format!("@unit/status #{red}"), "deployed").await;
    assert!(
        seen.contains("deployed"),
        "the pilot's own tank answers `deployed`:\n{seen}"
    );
    let seen = ask(&mut ada, &format!("@unit/status #{blue}"), NO_STATION).await;
    assert!(
        seen.contains(NO_STATION),
        "and the other side's answers the existence-hiding line:\n{seen}"
    );

    // The pilot on the other side, before detection.
    let seen = ask(&mut bo, "CONTACTS", NO_CONTACTS).await;
    names_nothing(&seen, red, blue, "the other pilot's `CONTACTS`");
    let seen = ask(&mut ada, "CONTACTS", NO_CONTACTS).await;
    names_nothing(&seen, red, blue, "the pilot's `CONTACTS`");

    // **16b cut Q.1: naming a contact you do not hold tells you nothing.**
    //
    // A designator is short and guessable in a way sixteen base-16 digits were
    // not — `BA` is the blue side's first pod, and anybody who has read the
    // board knows that — so the refusal has to be the load-bearing part. Every
    // shape of name the platform does not hold takes the same path, byte for
    // byte: a slot designator whose tank is out there and undetected, a
    // provisional number nothing has issued, a designator of a tank that does
    // not exist, the sixteen digits that used to be typable, and a word that is
    // not a designator at all.
    let mut refusals: Vec<String> = Vec::new();
    for line in [
        "LOCK BA",
        "LOCK RA",
        "LOCK X1",
        "LOCK ZZ",
        "LOCK ffffffffffffffff",
        "LOCK zzzz",
        "RANGE BA",
        "BEARING BA",
    ] {
        let seen = ask(&mut ada, line, NO_SUCH_CONTACT).await;
        refusals.push(seen.trim_start_matches(line).trim().to_owned());
    }
    let first = refusals.first().cloned().unwrap_or_default();
    for (line, answer) in [
        "LOCK BA",
        "LOCK RA",
        "LOCK X1",
        "LOCK ZZ",
        "LOCK ffffffffffffffff",
        "LOCK zzzz",
        "RANGE BA",
        "BEARING BA",
    ]
    .into_iter()
    .zip(refusals.iter())
    {
        assert_eq!(
            answer, &first,
            "{line} answered something other than the one byte-identical refusal"
        );
    }
    assert_eq!(first, NO_SUCH_CONTACT, "and the refusal is that sentence");
    // With no lock and no name the two read verbs say their own line, which is
    // NOT `FIRE`'s: `FIRE` takes a weapon and never a contact.
    for line in ["RANGE", "BEARING"] {
        let seen = ask(&mut ada, line, "No lock").await;
        assert!(
            seen.contains("No lock: LOCK a contact first, or name one."),
            "{line} with neither a lock nor a name:\n{seen}"
        );
    }
    let seen = ask(&mut ada, "VIEW", "Arena window").await;
    names_nothing(&seen, red, blue, "the pilot's `VIEW`");
    assert!(
        !seen.contains(" x ") && !seen.contains("hostile"),
        "no hostile marker before detection:\n{seen}"
    );

    // The same account, through the web client.
    let seen = look_web(&mut web).await;
    assert!(
        seen.contains(MUSTER),
        "the web client really looked:
{seen}"
    );
    names_nothing(&seen, red, blue, "the web client's `look`");
    // `@unit/show` answers a Sys line, and the needle IS the whole answer: the web
    // client is told exactly what the line client is told.
    let seen = ask_web(&mut web, &format!("@unit/show #{red}"), NOT_A_UNIT).await;
    assert!(seen.contains(NOT_A_UNIT), "{seen}");

    // -----------------------------------------------------------------
    // Detection: drive toward each other until the pass finds something.
    // -----------------------------------------------------------------
    // **16b cut Q.1**: everything the red pilot is sent from here to the first
    // held row, with nothing of their own typing in it but `CONTACTS`. The
    // assertions below about what may not reach a pilot are measured over this
    // window rather than over the whole transcript, which carries the refusal
    // battery's own echoes.
    let detect_mark = ada.mark();
    drive(&mut ada, 90).await;
    drive(&mut bo, 270).await;

    // **The wait is on the acquisition notice, not on a poll of `CONTACTS`.**
    //
    // Doc 16 §3.6's `acquired` is pushed the moment the pass finds something,
    // while she is driving and without her asking — so there is no round trip
    // between the detection and this read, and the name in it is the name the
    // return was given when it was found. A `CONTACTS` poll would be a race
    // against `classify_after_ms`, three seconds, and losing it would silently
    // move this window past the rename: the provisional phase would go unproved
    // rather than fail.
    ada.expect_after(detect_mark, "New contact ").await;
    // Everything the pilot was sent from the drive to that notice, and nothing
    // after it — the window every pre-classification assertion is measured over.
    let unresolved = strip_ansi(&ada.seen[detect_mark..ada.seen.len()]);
    names_nothing(&unresolved, red, blue, "the acquisition notice");

    // **The provisional phase, which is what W5 buys** — asserted, never skipped.
    //
    // There is no mesh in a two-tank match, so every contact either platform
    // holds is one its own sensors found, and such a return is unresolved for the
    // sensor's whole `classify_after_ms` before it is told apart. So the name the
    // acquisition quoted is a provisional one; a run in which it is not is this
    // cut breaking.
    //
    // While the return is held and not yet told apart the row is this platform's
    // own `X<n>`, nothing the pilot has been sent says which side it is or which
    // pod it came out of, and `LOCK BA` is still the byte-identical refusal —
    // because a row carries the slot designator only while its contact is
    // classified *now*. The refusal is typed in the same breath as the notice, so
    // it is answered while the row really is unknown.
    let provisional = acquired_designator(&unresolved)
        .unwrap_or_else(|| panic!("a designator in the acquisition notice:\n{unresolved}"));
    assert!(
        provisional.starts_with('X'),
        "an unresolved return was named by team and slot when it was found: \
         {provisional}\n{unresolved}"
    );
    assert!(
        !unresolved.contains("BA"),
        "the blue side's slot spelling reached the pilot before classification:\n{unresolved}"
    );
    assert!(
        !names_side(&unresolved, "blue"),
        "the other side's word reached the pilot before classification:\n{unresolved}"
    );
    let refused = ask(&mut ada, "LOCK BA", NO_SUCH_CONTACT).await;
    assert_eq!(
        refused.trim_start_matches("LOCK BA").trim(),
        NO_SUCH_CONTACT,
        "a slot designator resolved against a row that is still unknown"
    );

    // And then the table. `" s"` is the age column, which only the row has.
    let held = ask(&mut ada, "CONTACTS", " s").await;
    names_nothing(&held, red, blue, "the pilot's held `CONTACTS`");
    assert!(held.contains("bearing"), "{held}");
    assert!(
        held.contains("close") || held.contains("effective") || held.contains("far"),
        "a range band read against the fired weapon's own bands:\n{held}"
    );
    // **16b cut F.3**: and a range beside the band, deliberately coarse. Doc 16
    // §3.6 gives `CONTACTS` a band and no range, and read strictly that left a
    // pilot with no distance information at all; the rounding to fifty metres is
    // what a passive sight is allowed to say, and it is the one thing on this
    // surface a pilot can act on.
    let row = held
        .lines()
        .find(|line| line.contains("bearing") && line.contains(" m "))
        .unwrap_or_else(|| panic!("a contact row with a range on it:\n{held}"));
    /// The number before a unit word on the row, if there is exactly one.
    fn column(row: &str, unit: &str) -> Option<u64> {
        let words: Vec<&str> = row.split_whitespace().collect();
        let found: Vec<u64> = words
            .windows(2)
            .filter(|pair| pair.get(1).copied() == Some(unit))
            .filter_map(|pair| pair.first().and_then(|word| word.parse::<u64>().ok()))
            .collect();
        match found.as_slice() {
            [one] => Some(*one),
            _ => None,
        }
    }
    let range_m = column(row, "m").unwrap_or_else(|| panic!("one range, in metres: {row}"));
    assert_eq!(
        range_m % 50,
        0,
        "the range is finer than a passive sight may say: {row}"
    );
    // The age column is the plot's staleness, so it can never exceed the
    // time-to-live the contact is held for (`pass_ms × lost_after_passes`, three
    // seconds on the shipped sensor). A row that showed where the target *is*
    // rather than where it was last seen would have no honest number to put here.
    let age_s = column(row, "s").unwrap_or_else(|| panic!("one age, in seconds: {row}"));
    assert!(age_s <= 3, "the plot is older than the time to live: {row}");
    // **The plot can only ever be somewhere this platform could see.** The shipped
    // sight reaches 600 m and the mesh carries a classified contact one 300 m hop,
    // so nothing a pilot is shown can be plotted past 900 m — a row that reported
    // where the target *is* would have no such ceiling, because a held contact
    // outlives the sighting by the whole time to live and a tank covers ground in
    // it. This is what the surface can say about *nothing newer than the last
    // sighting*; the plot standing still across a break in the sighting is proved
    // deterministically in `tacsim/tests/perception.rs`, which can stop the sensor
    // pass and this scripted run cannot.
    assert!(
        range_m <= 900,
        "the plot is further than any sight or mesh hop on this map reaches: {row}"
    );
    // And the band and the range are two readings of **one** plot, so they agree.
    //
    // **16b cut D.1** changed which distance the word is about (`B-25`): it is
    // thirds of the LESSER of the platform's own sight and the fired weapon's
    // last band, and on every shipped platform the sight is the lesser — 600 m
    // against the cannon's 3 km. So `close` is inside 200 m, `effective` inside
    // 400 m, and `far` is everything out to the 900 m ceiling above, which is
    // the first time a shipped scope could print that word at all.
    let band = ["close", "effective", "far", "beyond"]
        .into_iter()
        .find(|word| row.contains(word))
        .unwrap_or_else(|| panic!("one band word: {row}"));
    let inside_m = match band {
        "close" => 200,
        "effective" => 400,
        "far" => 900,
        _ => panic!("nothing on this map can be plotted past the cannon's last band: {row}"),
    };
    assert!(
        range_m <= inside_m,
        "the band and the range disagree, so they are not one plot: {row}"
    );
    // **16b cut Q.1 inverted this assertion.** It used to read "the opaque
    // sixteen-digit id is the contact's only name"; the digits are not a pilot's
    // word any more, and nothing a session is *sent* carries one. Scoped to what
    // is sent deliberately, and measured from `detect_mark` so the only lines in
    // it are the pilot's own `CONTACTS` and what the arena answered: the ledger,
    // the world's own mirror and the LLM squad brief all still carry the
    // sixteen digits, and none of them is a pilot.
    let sent = strip_ansi(&ada.seen[detect_mark..]);
    no_sixteen_digit_word(&sent, "everything the pilot was sent while detecting");

    // **16b cut Q.1: the far end of `STARTUP`.** `drive` typed `STARTUP` inside
    // this window, so the line that says the sequence finished is in it — and it
    // is a notice addressed to the platform's own pilots, so it reaches the other
    // pilot about *their* tank and neither watcher about either.
    assert!(
        sent.contains("Power online."),
        "the pilot was never told their power came up:\n{sent}"
    );
    assert!(
        strip_ansi(&bo.seen).contains("Power online."),
        "the other pilot was never told about their own tank"
    );
    for (who, transcript) in [
        ("the bystander", strip_ansi(&cy.seen)),
        ("the staffer", strip_ansi(&god.seen)),
    ] {
        assert!(
            !transcript.contains("Power online."),
            "{who} was told a tank came online"
        );
    }
    assert!(
        held.contains("unknown") || held.contains("blue"),
        "a class word, or `unknown` until it classifies:\n{held}"
    );

    // **`LOCK`** (doc 16 §3.7): the id the platform holds resolves, and every other
    // id is the one refusal. The arena's phase 4 still drops the `Lock` order — cut
    // W is what gives it an effect — so what is asserted here is the resolution and
    // the refusal, which is S.2's half of the verb.
    //
    // 16b cut Q.1: the lock is taken under the **provisional** number read off the
    // first listing, which is what the rename block below then proves survives.
    let contact = provisional.clone();
    let seen = ask(&mut ada, &format!("LOCK {contact}"), "Locking").await;
    // The echo of the typed line is in this slice, so the needle is the *answer*'s
    // own word rather than the designator — which the rename block below pins as a
    // whole sentence, under both spellings.
    assert!(seen.contains("Locking "), "{seen}");
    no_sixteen_digit_word(&seen, "the answer to a `LOCK`");
    let mine = ask(&mut ada, &format!("LOCK {contact}"), "Locking").await;
    let unowned = ask(&mut ada, "LOCK ffffffffffffffff", NO_SUCH_CONTACT).await;
    let malformed = ask(&mut ada, "LOCK zzzz", NO_SUCH_CONTACT).await;
    assert!(mine.contains("Locking"), "{mine}");
    assert_eq!(
        unowned.trim_start_matches("LOCK ffffffffffffffff").trim(),
        malformed.trim_start_matches("LOCK zzzz").trim(),
        "one byte-identical refusal for a name this platform does not hold, whatever \
         shape it came in"
    );

    // **16b cut Q.1: the rename, and what survives it.**
    //
    // Three seconds of holding the return is enough to tell it apart. The row
    // then reads the target's public callsign, the rename is one line quoting
    // what the pilot has been calling it, the new spelling is accepted
    // case-folded, and the **provisional number the lock was taken under still
    // resolves** — which is the whole reason the book keeps it.
    // The *row* has to be re-read after the rename, not the transcript: the
    // `identified` notice arrives on its own and a window that merely contains
    // the word `blue` may still carry the `X<n>` row printed before it.
    let mut classified = String::new();
    for _ in 0..RETRY_BUDGET {
        let seen = ask(&mut ada, "CONTACTS", "(rel ").await;
        if seen.contains("blue") {
            classified = seen;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        !classified.is_empty(),
        "the return was never told apart in {RETRY_BUDGET} polls"
    );
    let renamed = designator_of(&classified)
        .unwrap_or_else(|| panic!("a contact designator in:\n{classified}"));
    assert_ne!(
        renamed, contact,
        "the classified row still reads as the provisional number:\n{classified}"
    );
    assert!(
        strip_ansi(&ada.seen).contains(&format!("Contact {contact} identified: {renamed}, blue.")),
        "the rename is one sentence quoting both names:\n{classified}"
    );
    let folded = ask(
        &mut ada,
        &format!("LOCK {}", renamed.to_ascii_lowercase()),
        "Locking",
    )
    .await;
    assert!(
        folded.contains(&format!("Locking {renamed}.")),
        "a designator is case-folded on the way in:\n{folded}"
    );
    let alias = ask(&mut ada, &format!("LOCK {contact}"), "Locking").await;
    assert!(
        alias.contains(&format!("Locking {renamed}.")),
        "the number the lock was taken under still resolves, and answers the new \
         name:\n{alias}"
    );
    // And the two read verbs answer about the lock with no argument at all, and
    // they answer with a **number**: the prefix alone would be satisfied by a
    // sentence that had stopped saying how far away anything was. The locked
    // reading is to the nearest ten metres (16b cut Q.1's `LOCK_RANGE_STEP_M`);
    // the step the lock earns is pinned exactly, against the row's own coarse
    // figure, in `tacsim/tests/resolution.rs`, which can hold the plot still and
    // this scripted run cannot.
    let ranged = ask(&mut ada, "RANGE", "Range to").await;
    let head = format!("Range to {renamed}: ");
    let tail = ranged
        .split_once(&head)
        .map(|(_, tail)| tail.to_owned())
        .unwrap_or_else(|| panic!("RANGE defaults to the lock:\n{ranged}"));
    let metres: u64 = tail
        .split_whitespace()
        .next()
        .and_then(|word| word.parse().ok())
        .unwrap_or_else(|| panic!("a range in metres: {tail}"));
    assert_eq!(metres % 10, 0, "a locked plot reads to ten metres: {tail}");
    let bearing = ask(&mut ada, "BEARING", "Bearing to").await;
    let head = format!("Bearing to {renamed}: ");
    let tail = bearing
        .split_once(&head)
        .map(|(_, tail)| tail.to_owned())
        .unwrap_or_else(|| panic!("BEARING defaults to the lock:\n{bearing}"));
    assert!(
        tail.starts_with(|c: char| c.is_ascii_digit()) && tail.contains("° (rel "),
        "a bearing absolute and off the platform's own nose: {tail}"
    );
    // And the map draws a marker for it without naming anything — once the contact
    // is inside the window, which is narrower (78 cells) than the sensor reaches
    // (60 cells each way), so the two go on closing until it is.
    let mut window = String::new();
    for _ in 0..DRIVE_BUDGET {
        let seen = ask(&mut ada, "VIEW", "Arena window").await;
        if seen.contains("contact") || seen.contains("hostile") || seen.contains("friendly") {
            window = seen;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        !window.is_empty(),
        "the window never drew a marker for the contact it holds"
    );
    names_nothing(&window, red, blue, "the pilot's held `VIEW`");

    // The world is still told nothing.
    let seen = ask_whole(&mut god, "look").await;
    names_nothing(&seen, red, blue, "the staffer's `look` after detection");
    let seen = look_web(&mut web).await;
    names_nothing(&seen, red, blue, "the web client's `look` after detection");

    say(&mut ada, "SIM ABORT", "aborts match").await;
}

/// **The observer rule with the switch on** (16b cut Q.1;
/// `[tacsim.sensors].designate_on_detection`).
///
/// The key is shipped as `false` into the rendered instance template and into
/// `deploy/dev2`, so an operator can turn it on with a server-child restart and no
/// build — and what it turns on is a *relaxation of the observer rule on four
/// player-facing surfaces at once*. `tacsim/tests/perception.rs` proves what the
/// arena does with it; this is the composition a pilot would actually be sitting
/// in, so that the conditioned half of the rule has a lens of its own rather than
/// a library test and an operator's hope.
///
/// With the switch on the return is named by its team and centre slot at the
/// **first** pass: no provisional number is ever issued, `LOCK BA` resolves before
/// `classify_after_ms` has run at all, and there is no rename to say — the
/// `classified` transition still happens in the arena and still reaches the black
/// box, and the pilot is simply not told a second time what they are already
/// calling it. What does **not** relax: the sixteen digits are still not a pilot's
/// word, and a name the platform does not hold is still the one refusal.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_switch_names_a_contact_at_the_first_pass_over_the_real_composition() {
    let server = boot_with("observer-designate", true).await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;

    let (red, blue, _muster, domain) = seat_the_witnesses(&mut god).await;
    close_the_gap(&mut god, red, blue, domain).await;
    take_off(&mut ada, &mut bo).await;

    // Before anything is held, the refusal is what it always was: the switch
    // relaxes what a *held* contact is called and nothing else.
    let refused = ask(&mut ada, "LOCK BA", NO_SUCH_CONTACT).await;
    assert_eq!(
        refused.trim_start_matches("LOCK BA").trim(),
        NO_SUCH_CONTACT,
        "the switch made an undetected tank nameable"
    );

    let detect_mark = ada.mark();
    drive(&mut ada, 90).await;
    drive(&mut bo, 270).await;
    let mut held = String::new();
    let mut first_end = detect_mark;
    for _ in 0..DRIVE_BUDGET {
        let mark = ada.mark();
        ada.send_line("CONTACTS").await;
        if ada.expect_one_of_after(mark, &["(rel ", NO_CONTACTS]).await == 0 {
            held = strip_ansi(&ada.seen[mark..]);
            first_end = ada.seen.len();
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        !held.is_empty(),
        "the two tanks never detected each other in {DRIVE_BUDGET} polls"
    );

    // **The first row already carries the slot designator.** The blue tank is in
    // slot 1 of its side, so the name is the team letter and the first slot.
    let named = designator_of(&held).unwrap_or_else(|| panic!("a contact designator in:\n{held}"));
    assert_eq!(
        named, "BA",
        "with the switch on the first pass names the target's team and pod:\n{held}"
    );
    // And no provisional number was issued: nothing the pilot has been sent
    // carries one, and the row itself is not an `X<n>`.
    let sent = strip_ansi(&ada.seen[detect_mark..first_end]);
    for n in 1_u16..=4 {
        let provisional = format!("X{n}");
        assert!(
            !sent.contains(&provisional),
            "a provisional number was issued with the switch on ({provisional}):\n{sent}"
        );
    }
    // The digits are still nobody's handle, switch or no switch.
    no_sixteen_digit_word(&sent, "everything the pilot was sent with the switch on");

    // The slot spelling resolves at once — before `classify_after_ms` could have
    // run — and it is the only spelling there is, so the number is refused.
    let seen = ask(&mut ada, &format!("LOCK {named}"), "Locking").await;
    assert!(
        seen.contains(&format!("Locking {named}.")),
        "the slot designator resolves at the first pass:\n{seen}"
    );
    let refused = ask(&mut ada, "LOCK X1", NO_SUCH_CONTACT).await;
    assert_eq!(
        refused.trim_start_matches("LOCK X1").trim(),
        NO_SUCH_CONTACT,
        "a provisional number resolved against a book that never issued one"
    );

    // The read verbs answer about the lock, under the same name — once the arena
    // has recorded it. Phase 4 takes the order and the cockpit reads the lock back
    // off the **next** frame, so a bare `RANGE` sent in the same breath as the
    // `LOCK` is honestly answered `No lock…`: a wait, not a failure (16b cut K.2's
    // mirror).
    for _ in 0..DRIVE_BUDGET {
        if ask_whole(&mut ada, "WEAPONSTATUS")
            .await
            .contains("Locked on")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let ranged = ask(&mut ada, "RANGE", "Range to").await;
    assert!(
        ranged.contains(&format!("Range to {named}: ")),
        "RANGE defaults to the lock:\n{ranged}"
    );
    let bearing = ask(&mut ada, "BEARING", "Bearing to").await;
    assert!(
        bearing.contains(&format!("Bearing to {named}: ")),
        "BEARING defaults to the lock:\n{bearing}"
    );

    // **And there is never a rename to say.** The classification still happens —
    // the row's class word turns into the side — but the pilot is told nothing
    // about it, because the name it has been calling the contact does not change.
    let mut classified = String::new();
    for _ in 0..RETRY_BUDGET {
        let seen = ask(&mut ada, "CONTACTS", "(rel ").await;
        if seen.contains("blue") {
            classified = seen;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        !classified.is_empty(),
        "the return was never told apart in {RETRY_BUDGET} polls"
    );
    assert_eq!(
        designator_of(&classified).as_deref(),
        Some("BA"),
        "the name moved across a classification that had nothing to rename:\n{classified}"
    );
    let whole = strip_ansi(&ada.seen);
    assert!(
        !whole.contains(" identified: "),
        "a rename was said for a contact that was never provisional:\n{whole}"
    );

    say(&mut ada, "SIM ABORT", "aborts match").await;
}

// ---------------------------------------------------------------------------
// The shooting (16b cut W.3)
// ---------------------------------------------------------------------------

/// **The two arena feeds are the observer rule too** (16b cut Q.3; doc 16 §5).
///
/// Cut Q.3 gave the cockpit a second feed and a pull, and a panel client reads
/// the *payload* rather than the sentence a terminal prints — so the assertions
/// the rest of this file makes about text have to be made about JSON as well,
/// over the real composition, through a real socket.
///
/// Three witnesses, each asked the same two questions:
///
/// | witness | seat | what they may have |
/// |---|---|---|
/// | Ada | the pilot, before her sensors have found anything | her own tank, and nothing about Bo's |
/// | Cy | a mortal bystander in the muster room | `seated: false` on both feeds |
/// | God | a staffer, who is not flying either | `seated: false` on both feeds |
///
/// Ada's socket is a **second connection on her own session** — she is on SSH
/// too — which is exactly how a player runs the web client beside a terminal, and
/// is what lets the pushed per-pulse frame be captured at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn neither_arena_feed_tells_a_panel_client_about_a_unit_it_has_not_found() {
    let server = boot("observer-feeds").await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, _cy) = login(server.addr, "cy", PILOT_PASSWORD, "Cy").await;

    let (red, blue, _muster, domain) = seat_the_witnesses(&mut god).await;
    close_the_gap(&mut god, red, blue, domain).await;

    // Ada's panel client, beside her terminal, opened **before** she boards: the
    // arena sends a frame when the numbers change, and a cold tank standing
    // still changes nothing after the opening pulse, so a client that attached
    // afterwards would wait for ever. (A real one asks — see the query below —
    // which is the whole reason the resync handlers exist.)
    let mut ada_web = web_session(&server, "ada", PILOT_PASSWORD, "Ada").await;
    take_off(&mut ada, &mut bo).await;

    // The two tanks are 70 cells apart and neither has moved, so nothing is held
    // by anybody.
    let cockpit = pushed_cockpit(&mut ada_web).await;
    let map = panel_query(&mut ada_web, 601, MAP_FEED).await;

    // (a) The pushed cockpit frame is her own tank and the scope it holds, which
    // is nothing.
    assert_eq!(
        cockpit.get("contacts"),
        Some(&json!([])),
        "an empty scope, before detection: {cockpit}"
    );
    assert_eq!(
        cockpit.get("callsign").and_then(Value::as_str),
        Some("RA"),
        "her own public name is hers to have: {cockpit}"
    );
    assert_eq!(
        cockpit.get("unit").and_then(Value::as_u64),
        Some(u64::from(red)),
        "and her own tank's object number, which is her own tank: {cockpit}"
    );

    // (b) The map is the ground, whole, and names no unit at all — not even her.
    assert!(
        map.get("units").is_none(),
        "the map carries no `units` key by construction: {map}"
    );
    assert!(
        map.get("you").is_none(),
        "nor the asker's own marker: {map}"
    );
    assert!(
        map.get("rows")
            .and_then(Value::as_array)
            .is_some_and(|rows| !rows.is_empty()),
        "but it IS the ground: {map}"
    );

    // (c) Neither payload says one word about the tank she has not found: not
    // its object number, not its name, not its pilot's name, not its callsign.
    for (what, payload) in [("the cockpit frame", &cockpit), ("the map", &map)] {
        let whole = payload.to_string();
        names_nothing(&whole, red, blue, what);
        no_sixteen_digit_word(&whole, what);
        for leak in ["Bo", "\"BA\"", "Ada"] {
            assert!(!whole.contains(leak), "{what} leaked {leak:?}:\n{whole}");
        }
    }
    // `names_nothing` also refuses `#{red}` and `Red Tank 1`, so the two
    // assertions above are the whole rule on these two surfaces: a frame names
    // its own unit by the bare number it carries in `unit`, and never by a dbref
    // or a name.

    // (d) A mortal bystander and a staffer ask the same two questions and are
    // told the one thing that is theirs: they are not flying. Nothing about the
    // match, its ground, or whether one is running at all.
    for (who, account) in [("the bystander", "cy"), ("the staffer", "boss")] {
        let password = if account == "boss" {
            GM_PASSWORD
        } else {
            PILOT_PASSWORD
        };
        let character = if account == "boss" { "God" } else { "Cy" };
        let mut ws = web_session(&server, account, password, character).await;
        assert_eq!(
            panel_query(&mut ws, 610, MAP_FEED).await,
            unseated(MAP_FEED),
            "{who} asked for the map"
        );
        assert_eq!(
            panel_query(&mut ws, 611, COCKPIT_FEED).await,
            unseated(COCKPIT_FEED),
            "{who} asked for the cockpit"
        );
    }
}

/// **The retain marker is one contract spelled in two crates** (16b cut Q.3).
///
/// The producer is `tacsim::match_::cockpit::RETAIN_LATEST` and the consumer is
/// `mush_session::RETAIN_LATEST`; the game crate does not depend on the
/// gateway's session manager, so the string — and the `schema.retain` path it
/// sits at — is the whole of what ties them. Either suite alone stays green
/// through a rename of its own constant, and production quietly stops
/// coalescing: two keyframes a second put a 512-entry resume ring back to
/// holding about four minutes of cockpit and nothing else.
///
/// `mushd` is a crate that sees both, so this is where they are held to each
/// other — by the string, and by driving payloads the game really builds through
/// a real [`SessionManager`] ring.
#[test]
fn the_retain_marker_the_arena_sends_is_the_one_the_session_ring_reads() {
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::Instant;

    use mush_proto::{ClientCaps, ClientKind, ColorDepth, RichText};
    use mush_session::{CharKey, ConnId, ConnectionHandle};

    assert_eq!(
        tacsim::match_::cockpit::RETAIN_LATEST,
        mush_session::RETAIN_LATEST,
        "the arena's marker and the ring's are one string"
    );

    let manager = SessionManager::with_config(SessionConfig::default());
    let (tx, _rx) = tokio::sync::mpsc::channel(256);
    let handle = ConnectionHandle::new_structured_at(
        ConnId(1),
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        ClientCaps {
            kind: ClientKind::Web,
            color: ColorDepth::TrueColor,
            width: 0,
            height: 0,
            utf8: true,
            structured: true,
        },
        tx,
        Instant::now(),
    );
    let (sess, _) = manager.attach(handle, CharKey(1), "ada".into());

    // A line, then a hundred cockpit frames and a hundred maps — payloads the
    // game's own builders produce, on the feeds it really sends them on — then
    // another line.
    let panel = |feed: &str, data: Value| OutputEvent::Panel {
        feed: feed.into(),
        kind: mush_proto::PanelFrameKind::Keyframe,
        data,
    };
    manager.send_event_to_char(CharKey(1), OutputEvent::Sys(RichText::plain("before")));
    for _ in 0..100 {
        manager.send_event_to_char(
            CharKey(1),
            panel(COCKPIT_FEED, tacsim::match_::cockpit::cockpit_cleared()),
        );
        manager.send_event_to_char(
            CharKey(1),
            panel(MAP_FEED, tacsim::match_::cockpit::map_unseated()),
        );
    }
    manager.send_event_to_char(CharKey(1), OutputEvent::Sys(RichText::plain("after")));

    let ring = manager
        .replay_since(sess, 0)
        .expect("a coalesced ring is never TooOld");
    let feeds: Vec<String> = ring
        .iter()
        .filter_map(|envelope| match &envelope.event {
            OutputEvent::Panel { feed, .. } => Some(feed.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(
        feeds,
        vec![COCKPIT_FEED.to_owned(), MAP_FEED.to_owned()],
        "one survivor per feed, and the two lines untouched around them"
    );
    assert_eq!(ring.len(), 4, "the two lines are still in the ring");
}

/// The cells the two slot-1 tanks are re-posed to for the shooting case: **four
/// cells apart** on the same road, both pointed east so the blue tank presents
/// its rear.
///
/// The gap above is about the *approach*, which the test above owns; this one is
/// about the **shot**, so the two tanks start inside the sensor's reach and the
/// wait is for a round to leave the barrel rather than for a contact to appear.
/// Forty metres of authored open grass is also what makes the shot land: doc 16
/// §3.5's presented silhouette against a 3 mrad band at that range is a near
/// certainty, so the damage this lens then goes looking for is not a coin toss.
const RED_MUZZLE: (i64, i64) = (90, 97);
const BLUE_MUZZLE: (i64, i64) = (94, 97);

/// How many rounds the red tank puts into the blue one. Two, because doc 16
/// §3.3's rear hull is 150 armour over 200 internals against 120 a round: two
/// hits mark it and **cannot** destroy it, so the match is still running when the
/// witnesses below are asked what they saw.
const ROUNDS: usize = 2;

/// Put the two tanks four cells apart on the road, nose to tail.
async fn close_to_muzzle_range(god: &mut Term, red: u32, blue: u32, domain: u64) {
    for (unit, (x, y)) in [(red, RED_MUZZLE), (blue, BLUE_MUZZLE)] {
        let pose = format!(
            "{{\"x_cm\":{},\"y_cm\":{},\"heading_mdeg\":90000}}",
            x * 1_000 + 500,
            y * 1_000 + 500
        );
        say(
            god,
            &format!("@spatial/place/domain #{unit}={domain},{pose}"),
            "Placed",
        )
        .await;
    }
}

/// Bring one tank's power up without moving it.
async fn power_up(term: &mut Term) {
    say(term, "STARTUP", "Startup sequence").await;
    for _ in 0..RETRY_BUDGET {
        let seen = ask(term, "STATUS", "power").await;
        if seen.contains("power online") {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("the tank never came online");
}

/// Wait until `CONTACTS` holds something, and answer the row's short designator
/// (16b cut Q.1).
async fn first_contact(term: &mut Term) -> String {
    for _ in 0..DRIVE_BUDGET {
        let mark = term.mark();
        term.send_line("CONTACTS").await;
        if term
            .expect_one_of_after(
                mark,
                // 16b cut Q.1: the relative bearing, which only a row carries —
                // the acquisition notice quotes an absolute bearing of its own.
                &["(rel ", NO_CONTACTS, COMMIT_PENDING, CONTROL_UNKNOWN],
            )
            .await
            == 0
        {
            let seen = strip_ansi(&term.seen[mark..]);
            if let Some(named) = designator_of(&seen) {
                return named;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("nothing was ever detected")
}

/// **The short designator in a `CONTACTS` listing** (16b cut Q.1): the first word
/// of a contact row, `X<n>` while the return is unresolved and the team letter
/// with the target's centre slot once it has been told apart.
fn designator_of(listing: &str) -> Option<String> {
    listing
        .lines()
        .find(|line| line.contains("bearing ") && line.contains("(rel "))
        .and_then(|line| line.split_whitespace().next())
        .map(str::to_owned)
}

/// **The designator an acquisition notice quoted** (16b cut Q.1): the word after
/// `New contact `, whether the sentence goes on to a bearing (`New contact X1:
/// bearing 090°, effective.`) or stops there (`New contact X1.`).
///
/// This is the one reading of the name that cannot lose a race with
/// `classify_after_ms`: the notice is pushed at the detection, so what it says is
/// what the return was called when it was found, however late the test reads it.
fn acquired_designator(sent: &str) -> Option<String> {
    sent.split_once("New contact ")
        .map(|(_, tail)| tail)
        .and_then(|tail| tail.split([':', '.', ' ', '\r', '\n']).next())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
}

/// **Nothing a session is sent carries a sixteen-digit contact id** (16b cut
/// Q.1), which is the inverse of the assertion cut S.2 wrote.
///
/// The digits are still the arena's own identity and still travel in the ledger,
/// in the world's frame mirror and in the LLM squad brief. None of those is a
/// pilot, and this is about pilots.
fn no_sixteen_digit_word(seen: &str, what: &str) {
    for word in seen.split(|c: char| !c.is_ascii_hexdigit()) {
        assert!(
            word.len() != 16 || u64::from_str_radix(word, 16).is_err(),
            "{what} carried a sixteen-digit contact id ({word}):\n{seen}"
        );
    }
}

/// Every word that would betray the shooting to somebody outside the cockpit that
/// took it.
///
/// **16b cut F.2** added the second row: a pilot's notices — the shooter's
/// outcome line, the victim's damage line, a stop, a refusal — are addressed to
/// one actor and reach nobody else, on any surface.
fn says_nothing_of_the_shooting(seen: &str, contact: &str, what: &str) {
    for leak in [
        contact,
        "Firing",
        "Locked on",
        "Locking",
        "round(s)",
        "Hit on",
        "Miss on",
        "is destroyed",
        "armour",
        "refused",
        "Held:",
    ] {
        assert!(
            !seen.contains(leak),
            "{what} leaked {leak:?} out of the arena:\n{seen}"
        );
    }
}

/// **The other half of the observer rule** (doc 16 §1 A1, §3.6; 16b cut W.3): the
/// rule holds through the *shooting*, not only through the approach.
///
/// The test above proves that nothing about an **undetected** unit crosses. This
/// one lets the red tank lock the blue one and put two rounds into it, and then
/// asks the same witnesses the same questions while the shooting is going on:
///
/// * the **bystander** in the muster room, who is not in the match at all;
/// * the **staffer**, on `look`, `+map`, `@unit/show` and `@unit/status`;
/// * the **web client**, on the same account through the other transport;
/// * and the **target's own pilot**, who holds a contact on its attacker and is
///   told **that it was hit and where** — doc 16 §5's notice feed, which 16b cut
///   F.2 built — and still nothing that identifies or locates the shooter: no
///   object number, no name, no contact id, no bearing, no side, no weapon.
///
/// The last of those is the half that changed. Cut W.3 recorded the honest gap —
/// the frame carried no damage surface, so the target was told nothing at all —
/// and this cut closes it without loosening the rule by a word: where a round
/// came from is a thing only a sensor may tell.
///
/// The absence assertions are only worth making if something really happened, so
/// the last act is an abort and a staff read of the checked-in tank: a facet whose
/// armour is below its authored maximum is the proof that two rounds landed on a
/// unit nobody outside the arena could see being shot.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_of_the_shooting_crosses_to_the_world_or_to_the_target() {
    let server = boot("observer-fight").await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, mut cy) = login(server.addr, "cy", PILOT_PASSWORD, "Cy").await;
    let mut web = web_session(&server, "boss", GM_PASSWORD, "God").await;

    let (red, blue, _muster, domain) = seat_the_witnesses(&mut god).await;
    close_to_muzzle_range(&mut god, red, blue, domain).await;
    take_off(&mut ada, &mut bo).await;
    power_up(&mut ada).await;
    power_up(&mut bo).await;

    // Drain what the walk-in queued: a room render that arrives *after* a mark
    // but answers a command sent *before* it would make an absence assertion fail
    // for a reason that has nothing to do with the arena.
    let _ = ask_whole(&mut cy, "look").await;
    let _ = ask_whole(&mut god, "look").await;

    // Everything the two watchers and the target are shown from here on.
    let watcher_mark = cy.mark();
    let staff_mark = god.mark();
    let target_mark = bo.mark();

    let contact = first_contact(&mut ada).await;
    say(&mut ada, &format!("LOCK {contact}"), "Locking").await;
    // The arena records the lock in its own phase 4 and the cockpit reads it back
    // off the **next** frame, so `FIRE` before that is refused for the lock — a
    // wait, not a failure (16b cut K.2's mirror).
    for _ in 0..DRIVE_BUDGET {
        if ask_whole(&mut ada, "WEAPONSTATUS")
            .await
            .contains("Locked on")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    // One round at a time, paced by the cockpit's own count: `FIRE` pre-checks
    // against the **last frame**, so a second one sent before the frame carrying
    // the first shot's cycle arrives is accepted here and dropped by the arena.
    // Waiting for the round to disappear from the table is what makes each pass of
    // this loop one round that really left the barrel.
    let mut spent = String::new();
    for round in 1..=ROUNDS {
        for _ in 0..RETRY_BUDGET {
            let mark = ada.mark();
            ada.send_line("FIRE").await;
            if ada
                .expect_one_of_after(
                    mark,
                    &[
                        "Firing cannon",
                        "Still cycling",
                        "No lock",
                        COMMIT_PENDING,
                        CONTROL_UNKNOWN,
                    ],
                )
                .await
                == 0
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let want = format!("{} of 40 round(s)", 40 - round);
        spent = String::new();
        for _ in 0..DRIVE_BUDGET {
            let seen = ask_whole(&mut ada, "WEAPONSTATUS").await;
            if seen.contains(&want) {
                spent = seen;
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        assert!(
            !spent.is_empty(),
            "round {round} never left the barrel:
{}",
            strip_ansi(&ada.seen[ada.seen.len().saturating_sub(3_000)..])
        );
    }
    // The shooter's own cockpit is the one place the rounds are visible.
    assert!(
        spent.contains(&format!("{} of 40 round(s)", 40 - ROUNDS)),
        "the red tank really fired: {spent}"
    );

    // ---- the bystander -------------------------------------------------
    let seen = ask_whole(&mut cy, "look").await;
    assert!(
        seen.contains(MUSTER),
        "the bystander really looked:\n{seen}"
    );
    names_nothing(&seen, red, blue, "the bystander's `look` mid-fight");
    says_nothing_of_the_shooting(&seen, &contact, "the bystander's `look` mid-fight");
    says_nothing_of_the_shooting(
        &strip_ansi(&cy.seen[watcher_mark..]),
        &contact,
        "the bystander's whole transcript",
    );

    // ---- the staffer, on every read the world offers -------------------
    let seen = ask_whole(&mut god, "look").await;
    names_nothing(&seen, red, blue, "the staffer's `look` mid-fight");
    let seen = ask(&mut god, "+map", "not on a map").await;
    names_nothing(&seen, red, blue, "the staffer's `+map` mid-fight");
    says_nothing_of_the_shooting(&seen, &contact, "the staffer's `+map` mid-fight");
    let seen = ask(&mut god, &format!("@unit/show #{blue}"), NOT_A_UNIT).await;
    assert!(seen.contains(NOT_A_UNIT), "{seen}");
    let seen = ask(&mut god, &format!("@unit/status #{blue}"), NO_STATION).await;
    assert!(seen.contains(NO_STATION), "{seen}");
    says_nothing_of_the_shooting(
        &strip_ansi(&god.seen[staff_mark..]),
        &contact,
        "the staffer's whole transcript",
    );

    // ---- the same account, through the web client ----------------------
    let seen = look_web(&mut web).await;
    assert!(
        seen.contains(MUSTER),
        "the web client really looked:\n{seen}"
    );
    names_nothing(&seen, red, blue, "the web client's `look` mid-fight");
    says_nothing_of_the_shooting(&seen, &contact, "the web client's `look` mid-fight");
    let seen = ask_web(&mut web, &format!("@unit/show #{blue}"), NOT_A_UNIT).await;
    assert!(seen.contains(NOT_A_UNIT), "{seen}");

    // ---- the target's own pilot ----------------------------------------
    //
    // It holds a contact on its attacker — forty metres of open grass is well
    // inside the stock sensor — and it is told, at each impact, that it was hit
    // and where. What it is **not** told is anything at all about who fired: no
    // name, no object number, not the contact id it holds the shooter under, not
    // a bearing, not a side, not a weapon.
    let seen = ask(&mut bo, "CONTACTS", "bearing").await;
    names_nothing(&seen, red, blue, "the target's `CONTACTS`");
    let target = strip_ansi(&bo.seen[target_mark..]);
    assert!(
        target.contains("Hit on the "),
        "the target was told nothing about the rounds that hit it:\n{target}"
    );
    // The name *the target itself* holds the shooter under is the one a leak
    // would most plausibly carry, so it is named explicitly.
    //
    // **16b cut Q.1** took the shooter's own name for the target off both lists
    // below, and the reason is the cut's: a slot designator is the **target's
    // own public callsign** — doc 16 §5's amendment makes it lobby knowledge —
    // so "the victim must not read `BA`" is a rule about the victim's own name
    // and proves nothing. What must not cross is who fired, which is what
    // `bos_contact`, the tank's name, its object number and its side are here
    // for.
    let bos_contact = first_contact(&mut bo).await;
    let told: Vec<&str> = target
        .lines()
        .filter(|line| line.starts_with("Hit on the ") || line.starts_with("The "))
        .collect();
    assert!(!told.is_empty(), "{target}");
    for line in &told {
        for leak in [
            RED_TANK.to_string(),
            format!("#{red}"),
            bos_contact.clone(),
            "bearing".to_string(),
            "cannon".to_string(),
            "°".to_string(),
        ] {
            assert!(
                !line.contains(&leak),
                "a hit notice carried {leak:?} about the shooter: {line}"
            );
        }
        // 16b cut F.3: the side is forbidden as a **word**, so a future sentence
        // that happens to spell it (`cleared`) is not a false failure.
        assert!(
            !names_side(line, "red"),
            "a hit notice named the shooter's side: {line}"
        );
    }
    // Over the target's WHOLE transcript, so `bos_contact` is deliberately not on
    // this list: the target's own `CONTACTS` is in the window, and its own scope
    // is exactly where that name belongs. The per-line list above is where it
    // has to be absent.
    //
    // **16b cut Q.1** puts the shooter's own sentences back on it in the shape
    // they are actually said in. The bare designator could not stay — under this
    // cut it is the victim's own public callsign, which the victim's own
    // `CONTACTS` legitimately prints — but `Hit on BA.` is a whole sentence only
    // the *shooter* is ever sent, and nothing the victim does can produce it. It
    // is listed under both spellings, because the contact was told apart while
    // the rounds were in the air and the sentence names whatever the row read as
    // at the moment it was staged.
    //
    // **16b cut Q.2** put the section word on the shooter's line, so the bare
    // `Hit on BA.` spelling is no longer the one a classified contact produces —
    // both are listed, and the `Hit on BA: ` **prefix** is listed beside them so
    // the assertion cannot quietly become vacuous the next time the sentence
    // gains a clause. `BA's turret is destroyed.` is the shooter's other new
    // sentence; the victim's own is `The turret is destroyed: …`, which is a
    // different string and is legitimately in the victim's transcript.
    let shot_at = first_contact(&mut ada).await;
    for leak in [
        RED_TANK.to_string(),
        format!("#{red}"),
        format!("Hit on {contact}."),
        format!("Hit on {shot_at}."),
        format!("Hit on {contact}: "),
        format!("Hit on {shot_at}: "),
        format!("{shot_at} destroyed."),
        format!("{shot_at}'s "),
        "Firing".to_string(),
        "Locked on".to_string(),
        "to hit".to_string(),
        "damage".to_string(),
    ] {
        assert!(
            !target.contains(&leak),
            "the target was told {leak:?} about the round that hit it:\n{target}"
        );
    }
    // And the shooter is not shown the victim's half either: `Hit on the …` is
    // the victim's sentence and `Hit on <contact>.` is the shooter's.
    let shooter = strip_ansi(&ada.seen);
    assert!(
        !shooter.contains("Hit on the "),
        "the shooter was shown the target's own damage:\n{shooter}"
    );

    // ---- the gunner, and the one word this cut gave them ---------------
    //
    // **16b cut Q.2**: the shooter is told which section a round found and which
    // section came apart, and *only* for a contact their own platform holds AND
    // has told apart — because a section id is a chassis vocabulary
    // (`left_track` against `left_leg`), so a section word on an unidentified
    // blip is identification by gunfire, which doc 16 §2 W5 prices at
    // `classify_after_ms` of looking.
    let gunner = strip_ansi(&ada.seen);
    let outcomes: Vec<&str> = gunner
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("Hit on ") && !line.starts_with("Hit on the "))
        .collect();
    assert!(
        !outcomes.is_empty(),
        "the gunner was told nothing:\n{gunner}"
    );
    for line in &outcomes {
        // A provisional return earns no section word at all.
        if line.starts_with("Hit on X") {
            assert!(
                !line.contains(':'),
                "a section word on a return the platform has not identified: {line}"
            );
        }
        // And whatever the line says, it carries no facet and nothing of the
        // target's own gauges.
        for leak in ["armour", "facet", "internals", "of "] {
            assert!(
                !line.contains(leak),
                "the gunner's line carried {leak:?}: {line}"
            );
        }
        // The section word itself is a word and never a number. The *designator*
        // is a number when it is provisional (`Hit on X1.`), which is the
        // platform's own name for the return and is exactly what cut Q.1 made it;
        // what may not carry one is the part of the line that is about the
        // target's hull.
        if let Some((_, section)) = line.split_once(": ") {
            assert!(
                !section.chars().any(|c| c.is_ascii_digit()),
                "the section word carried a number: {line}"
            );
        }
    }
    no_sixteen_digit_word(&gunner, "the gunner's transcript");

    // ---- `SCAN`, on the shooter's own scope ----------------------------
    //
    // It defaults to the platform's lock, which is the contact the rounds above
    // were fired at, so this is the reading the gunner earned by closing to
    // muzzle range.
    //
    // The answer is a header line and then the run of bands, sent as separate
    // lines, so the read is retried until the **whole** answer is in the window:
    // a reader that stopped at the header would be asserting about half of it.
    // The stock tank's five sections do not fit on one 76-column line, so the
    // last of them is the needle.
    //
    // The wait is on the reading itself and **not** on either outcome: the two
    // tanks are forty metres apart on open grass, which is well inside
    // `condition_reach_permille` of the stock sight's reach, so a `SCAN` that
    // still said `No reading on …` here would be the verb failing and not a
    // fixture the test does not control. Every claim below is therefore
    // unconditional.
    let mut scanned = String::new();
    for _ in 0..DRIVE_BUDGET {
        let said = ask(&mut ada, "SCAN", &shot_at).await;
        if said.contains("turret") {
            scanned = said;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        scanned.contains("Scan of "),
        "SCAN of the lock never read the target: {scanned}"
    );
    no_sixteen_digit_word(&scanned, "SCAN of the lock");
    names_nothing(&scanned, red, blue, "SCAN of the lock");
    // Never a number other than the age of the look: a band line names a section
    // and a word, and between them the runs name all five of the stock tank's.
    let runs: Vec<&str> = scanned
        .lines()
        .filter(|line| line.starts_with("  "))
        .collect();
    assert!(!runs.is_empty(), "SCAN printed no bands: {scanned}");
    for line in &runs {
        assert!(
            !line.chars().any(|c| c.is_ascii_digit()),
            "a band line carried a number: {line}"
        );
    }
    let bands = runs.join(" ");
    for section in ["hull", "turret", "left_track", "right_track", "engine"] {
        assert!(
            bands.contains(section),
            "SCAN left out the {section}: {bands}"
        );
    }

    // Every shape of name the platform does not hold takes the one
    // byte-identical refusal, and `SCAN` may not be the verb that tells a pilot
    // which tanks are out there: it has three different answers of its own for a
    // contact it *does* hold, so a fourth for one it does not would be the leak.
    let mut refusals: Vec<String> = Vec::new();
    for name in ["ZZ", "X9", "RA", "0123456789abcdef", "zzzz"] {
        let said = ask(&mut ada, &format!("SCAN {name}"), NO_SUCH_CONTACT).await;
        assert!(
            !said.contains("Scan of ") && !said.contains("not identified"),
            "SCAN {name} answered something about a contact: {said}"
        );
        // The **answer** and not the window: what arrives between the mark and
        // the refusal may still carry the tail of the previous read's own lines,
        // and the claim here is that the four refusals are one sentence.
        let answer = said.split_once(&format!("SCAN {name}")).map_or_else(
            || said.trim().to_owned(),
            |(_, tail)| tail.trim().to_owned(),
        );
        assert_eq!(answer, NO_SUCH_CONTACT, "SCAN {name}: {said}");
        refusals.push(answer);
    }
    let first = refusals.first().cloned().unwrap_or_default();
    for said in &refusals {
        assert_eq!(
            said, &first,
            "SCAN answered two different refusals for two names it does not hold"
        );
    }

    // ---- and the target's pilot reads none of it -----------------------
    //
    // The victim is told what it cost *their* tank (`The turret is destroyed:
    // …`), which is their own physical knowledge; the gunner's sentence about
    // the same section is a different string and is the shooter's alone.
    let target_now = strip_ansi(&bo.seen[target_mark..]);
    for line in target_now.lines() {
        assert!(
            !line.trim().starts_with("Scan of "),
            "the target read the gunner's scan: {line}"
        );
    }

    // ---- and it really was hit -----------------------------------------
    say(&mut ada, "SIM ABORT", "aborts match").await;
    ada.expect("Its units are checked in.").await;
    let mut marked = String::new();
    for _ in 0..DRIVE_BUDGET {
        let seen = ask_whole(&mut god, &format!("@unit/show #{blue}")).await;
        if damaged_facet(&seen) {
            marked = seen;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        !marked.is_empty(),
        "the checked-in tank carries no mark of the two rounds that hit it:\n{}",
        strip_ansi(&god.seen[god.seen.len().saturating_sub(3_000)..])
    );
}

/// Whether any facet's armour cell in a `@unit/show` listing is below its
/// authored maximum — the world-side mark of a round that landed.
fn damaged_facet(listing: &str) -> bool {
    listing.lines().any(|line| {
        let Some(rest) = line.split_once("cell facet/") else {
            return false;
        };
        let Some((_, values)) = rest.1.split_once(": ") else {
            return false;
        };
        let Some((current, max)) = values.trim().split_once('/') else {
            return false;
        };
        match (current.parse::<i64>(), max.parse::<i64>()) {
            (Ok(current), Ok(max)) => current < max,
            _ => false,
        }
    })
}

// ---------------------------------------------------------------------------
// The TUI witness
// ---------------------------------------------------------------------------

/// **The last tank of the muster's contents list, as the TUI leaves it.**
///
/// The TUI paints cells, so it writes each word of a line at its own cursor
/// position and the spaces between them are never sent: `Blue Tank 4,` arrives as
/// `BlueTank4,` once the escape sequences are stripped. Every assertion about TUI
/// text therefore runs against [`squeezed`] output, and this is the needle that
/// says a room render has finished arriving.
const TUI_LAST_TANK: &str = "BlueTank4";

/// TUI text with every space taken out — see [`TUI_LAST_TANK`]. A name assertion
/// made against the un-squeezed text would be **vacuous**, because `Red Tank 1`
/// with its spaces never appears on this surface at all.
fn squeezed(raw: &str) -> String {
    strip_ansi(raw)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// One line through the TUI's input box, read until the room render has landed
/// whole.
///
/// The box is opened **once**, by the bare Return the caller sends after `@tui`: a
/// submission leaves the TUI in input mode, and an *empty* submission is what puts
/// it back in nav mode — where the next line's characters would be eaten as
/// hotkeys (`r` opens the roll prompt, which is how this was found out).
async fn tui_ask(term: &mut Term, line: &str) -> String {
    let mark = term.mark();
    term.send_line(line).await;
    term.expect_after(mark, TUI_LAST_TANK).await;
    squeezed(&term.seen[mark..])
}

/// Everything the TUI must never say about a unit an arena holds.
fn tui_names_nothing(seen: &str, red: u32, blue: u32) {
    for leak in [
        "RedTank1".to_string(),
        "BlueTank1".to_string(),
        format!("#{red}"),
        format!("#{blue}"),
    ] {
        assert!(
            !seen.contains(&leak),
            "the TUI leaked {leak:?}:
{seen}"
        );
    }
}

/// **The full-screen surface** (doc 16 §1: *the TUI panels*).
///
/// `@tui` is intercepted transport-side, below the hooks boundary, so a `mushd`
/// session enters the real TUI over the same SSH connection, and its scene screen
/// renders the narrative stream — which is where a room render lands. So the witness
/// is the one the line client already has: the bystander standing in the muster
/// room, asked to `look` before the match and again while it runs.
///
/// What this covers is the bystander's SCENE screen, which is the narrative
/// surface. The full-screen cockpit — the tenth screen 16b cut Q.4 bound to
/// `arena.cockpit` — has a lens of its own below
/// ([`the_tui_cockpit_shows_a_pilot_no_more_than_their_own_sensors_found`]),
/// with a pilot in the seat and this same bystander at the tab strip.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tui_witness_is_shown_the_tanks_and_then_nothing_about_the_two_in_the_match() {
    let server = boot("observer-tui").await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, mut cy) = login(server.addr, "cy", PILOT_PASSWORD, "Cy").await;
    let (red, blue, _muster, domain) = seat_the_witnesses(&mut god).await;

    // Into the TUI, and the alternate screen says so.
    cy.send_line("@tui").await;
    cy.expect("rustmush").await;
    assert!(
        cy.seen.contains(ALT_SCREEN),
        "the alternate screen was not entered"
    );
    // The input box, opened once (see `tui_ask`).
    cy.send_line("").await;

    // The control: the TUI shows the bystander all eight tanks.
    let seen = tui_ask(&mut cy, "look").await;
    assert!(
        seen.contains("RedTank1") && seen.contains("BlueTank1"),
        "the TUI renders the room the bystander is standing in:
{seen}"
    );

    // The match.
    close_the_gap(&mut god, red, blue, domain).await;
    take_off(&mut ada, &mut bo).await;

    // And now nothing about the two tanks the arena holds.
    let seen = tui_ask(&mut cy, "look").await;
    assert!(
        seen.contains("TheProvingGround"),
        "the TUI really re-rendered the room:
{seen}"
    );
    assert!(
        seen.contains("RedTank2"),
        "the six tanks nobody deployed are still in it:
{seen}"
    );
    tui_names_nothing(&seen, red, blue);

    say(&mut ada, "SIM ABORT", "aborts match").await;
}

/// **The cockpit on the full-screen surface** (doc 16 §1: *the TUI panels*, §5;
/// 16b cut Q.4).
///
/// The witness above is a bystander, and the note it used to carry was that
/// there was nothing more to ask: the TUI bound no screen to `arena.cockpit`,
/// so there was no full-screen cockpit panel to assert about. 16b cut Q.4 built
/// one — a tenth screen on digit `0`, its map composed on the client out of the
/// pulled terrain and the frame's own markers — and this is its lens.
///
/// Two witnesses, each asked the question the rule is about:
///
/// * **Ada**, a pilot sitting in her own cockpit with the full screen up, asked
///   twice: once with the other tank seventy cells away and outside every
///   reach, when her screen must carry no trace of it at all — no name, no
///   object number, no designator, not the callsign it answers to on its own
///   side — and once after it has driven into her sensor's reach, when it must
///   carry the designator her own platform issued and nothing else.
/// * **Cy**, a bystander on the same arena server, who has no seat: the arena
///   pushes `arena.cockpit` to seated pilots and to nobody else, so the tenth
///   tab is not on his tab strip at all and the digit that would reach it does
///   nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_tui_cockpit_shows_a_pilot_no_more_than_their_own_sensors_found() {
    let server = boot("observer-cockpit").await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, mut cy) = login(server.addr, "cy", PILOT_PASSWORD, "Cy").await;
    let (red, blue, _muster, domain) = seat_the_witnesses(&mut god).await;
    close_the_gap(&mut god, red, blue, domain).await;
    take_off(&mut ada, &mut bo).await;
    // Her power comes up in LINE mode, before the full screen takes over: a
    // cold platform's sensors find nothing, and the question here is what a
    // WARM one's screen says about a tank it has not found yet.
    power_up(&mut ada).await;

    // ---- the bystander's tab strip -------------------------------------
    //
    // Asked first, while the arena still holds nothing of his: he has no seat
    // now and is given none later, so his answer cannot go stale.
    let opened = cy.mark();
    cy.send_line("@tui").await;
    // The right-hand block of the tab strip carries the character's own name,
    // and at eighty columns nothing else on the scene screen does — so this is
    // the strip itself having painted, not some pane that mentions him.
    cy.expect_after(opened, "Cy").await;
    let strip = squeezed(&cy.seen[opened..]);
    assert!(
        strip.contains("89"),
        "the ungated tabs are on the strip:\n{strip}"
    );
    assert!(
        !strip.contains("890"),
        "a bystander is sent no cockpit frame and earns no tenth tab:\n{strip}"
    );
    // And the digit itself is inert: nothing a cockpit pane would ever say
    // appears after it.
    let pressed = cy.mark();
    cy.send_line("0").await;
    cy.expect_after(pressed, "history").await;
    let after_zero = squeezed(&cy.seen[pressed..]);
    for cockpit_word in ["Callsign", "Notinacockpit", "Waitingforthearena"] {
        assert!(
            !after_zero.contains(cockpit_word),
            "the digit reached a cockpit for a bystander ({cockpit_word}):\n{after_zero}"
        );
    }

    // ---- the pilot, before anything has been found ----------------------
    let opened = ada.mark();
    ada.send_line("@tui").await;
    // `Callsign` is the status pane's first row, and it paints on the cockpit
    // screen alone — which the FIRST frame of the match opens by itself.
    ada.expect_after(opened, "Callsign").await;
    let before = squeezed(&ada.seen[opened..]);
    assert!(
        before.contains("CallsignRA"),
        "her own tank's callsign is hers to have:\n{before}"
    );
    assert!(
        before.contains("Nolock"),
        "the status pane painted whole:\n{before}"
    );
    tui_names_nothing(&before, red, blue);
    // The id scraper reads the text with its SPACES still in it. Its mechanism
    // is a split on every non-hex-digit character, and squeezing a screen puts
    // the words hard against each other: `BA 1111111111111111 blue` squeezed is
    // one nineteen-character hex-ish word, and an assertion on a word's LENGTH
    // would sail past the very id it is looking for.
    no_sixteen_digit_word(
        &strip_ansi(&ada.seen[opened..]),
        "the TUI cockpit before detection",
    );
    for trace in ["X1", "BA"] {
        assert!(
            !before.contains(trace),
            "the screen carried {trace:?} before anything was detected:\n{before}"
        );
    }
    // **The pulled terrain arrived.** `arena.map` is answered only to a query
    // the client sends itself, so this one needle covers the whole seam — the
    // cockpit frame asking, the gateway forwarding, the bridge answering, and
    // the store keeping it — and its absence is what a dark map looks like.
    // The heading line replaces the placeholder the pane opens on, and nothing
    // else on this screen prints a capital `Map`.
    ada.expect_after(opened, "Map").await;
    let with_ground = squeezed(&ada.seen[opened..]);
    assert!(
        with_ground.contains("Mapheading") || with_ground.contains("Map\u{2014}heading"),
        "the map pane composed the pulled ground:\n{with_ground}"
    );

    // ---- and after the other tank drives into reach ---------------------
    let closing = ada.mark();
    drive(&mut bo, 270).await;
    // The provisional designator her own platform issued, or the callsign the
    // row is renamed to the moment the return is told apart: either is a word
    // she may have, and nothing else on the screen may identify the tank.
    let named = ada.expect_one_of_after(closing, &["X1", "BA"]).await;
    let after = squeezed(&ada.seen[closing..]);
    assert!(
        after.contains(if named == 0 { "X1" } else { "BA" }),
        "{after}"
    );
    tui_names_nothing(&after, red, blue);
    no_sixteen_digit_word(
        &strip_ansi(&ada.seen[closing..]),
        "the TUI cockpit after detection",
    );
}

/// **`B-01`: the board is the observer rule too** (16b cut D.1; doc 16 §1, §5).
///
/// `SIM STATUS` is the one selector that answers in every state and from every
/// room of the centre **including the pods**, so it was the one surface on which
/// a seated pilot could read the other side's standing: `, pod Blue Pod 1,
/// seated` before, `, left` the moment the human on `BA` dropped and a machine
/// took the tank. No sensor of theirs told them any of it.
///
/// Over the real front door, from inside the pod, because that is where a pilot
/// types it and because the reader is the whole of the rule.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_board_withholds_the_other_sides_standing_from_a_seated_pilot() {
    let server = boot("observer-board").await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;

    seat_the_witnesses(&mut god).await;
    take_off(&mut ada, &mut bo).await;

    /// One side's row off a board.
    fn row(board: &str, head: &str) -> String {
        board
            .lines()
            .find(|line| line.trim_start().starts_with(head))
            .unwrap_or_default()
            .to_owned()
    }

    // Typed from inside the pod, while the match runs.
    let board = strip_ansi(&ask(&mut ada, "SIM STATUS", "blue:").await);
    let red = row(&board, "red:");
    let blue = row(&board, "blue:");
    assert!(
        red.contains(", pod ") && red.contains(", seated"),
        "a pilot was refused their OWN side's standing: {board}"
    );
    // The callsign and the pilot's name stay: both were public through the whole
    // of `Selection` and neither can change while the match runs.
    assert!(blue.contains("(BA)") && blue.contains("Bo"), "{board}");
    for word in [", pod ", ", seated", ", left"] {
        assert!(
            !blue.contains(word),
            "a seated pilot read `{word}` about the other side: {board}"
        );
    }

    // Bo climbs out, which mid-match is final and is exactly the fact `B-01` is
    // about. Ada's board does not move by one byte; staff read the truth.
    say(&mut bo, "EXIT", "climb").await;
    let after = strip_ansi(&ask(&mut ada, "SIM STATUS", "blue:").await);
    assert_eq!(
        row(&after, "blue:"),
        blue,
        "the other side's row moved when its pilot left:\n{after}"
    );
    let staff = strip_ansi(&ask(&mut god, "@arena/status", "blue:").await);
    assert!(
        row(&staff, "blue:").contains(", left"),
        "staff were refused the whole board: {staff}"
    );
}

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
    mods.extend(modules::ruleset_modules(onboarding, &Default::default()));
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
        Engine::with_modules(world, registry, module_list(&onboarding, &llm)).expect("modules");
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
    drive(&mut ada, 90).await;
    drive(&mut bo, 270).await;
    let mut held = String::new();
    for _ in 0..DRIVE_BUDGET {
        // Either answer ends the wait: a contact line carries a bearing, and an
        // empty scope says so. Polling on one needle alone would block for the
        // whole step timeout on the ordinary case.
        let mark = ada.mark();
        ada.send_line("CONTACTS").await;
        if ada
            .expect_one_of_after(mark, &["bearing", NO_CONTACTS])
            .await
            == 0
        {
            held = strip_ansi(&ada.seen[mark..]);
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        !held.is_empty(),
        "the two tanks never detected each other in {DRIVE_BUDGET} polls"
    );

    // The first thing that crossed was the notice — doc 16 §3.6's `acquired`,
    // on its own line, while she was driving and without her asking.
    names_nothing(&held, red, blue, "the acquisition notice");
    assert!(
        strip_ansi(&ada.seen).contains("New contact"),
        "the acquisition notice reached the pilot:\n{held}"
    );
    // And then the table. `" s"` is the age column, which only the row has.
    let held = ask(&mut ada, "CONTACTS", " s").await;
    names_nothing(&held, red, blue, "the pilot's held `CONTACTS`");
    assert!(held.contains("bearing"), "{held}");
    assert!(
        held.contains("close") || held.contains("effective") || held.contains("far"),
        "a range band and not a range:\n{held}"
    );
    assert!(
        held.split(|c: char| !c.is_ascii_hexdigit())
            .any(|word| word.len() == 16),
        "the opaque sixteen-digit id is the contact's only name:\n{held}"
    );
    assert!(
        held.contains("unknown") || held.contains("blue"),
        "a class word, or `unknown` until it classifies:\n{held}"
    );

    // **`LOCK`** (doc 16 §3.7): the id the platform holds resolves, and every other
    // id is the one refusal. The arena's phase 4 still drops the `Lock` order — cut
    // W is what gives it an effect — so what is asserted here is the resolution and
    // the refusal, which is S.2's half of the verb.
    let contact = held
        .split(|c: char| !c.is_ascii_hexdigit())
        .find(|word| word.len() == 16)
        .unwrap_or_else(|| panic!("a contact id to lock in:\n{held}"))
        .to_owned();
    let seen = ask(&mut ada, &format!("LOCK {contact}"), "Locking").await;
    assert!(seen.contains(&contact), "{seen}");
    let mine = ask(&mut ada, &format!("LOCK {contact}"), "Locking").await;
    let unowned = ask(&mut ada, "LOCK ffffffffffffffff", NO_SUCH_CONTACT).await;
    let malformed = ask(&mut ada, "LOCK zzzz", NO_SUCH_CONTACT).await;
    assert!(mine.contains("Locking"), "{mine}");
    assert_eq!(
        unowned.trim_start_matches("LOCK ffffffffffffffff").trim(),
        malformed.trim_start_matches("LOCK zzzz").trim(),
        "one byte-identical refusal for an id this platform does not hold, whatever \
         shape it came in"
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

// ---------------------------------------------------------------------------
// The shooting (16b cut W.3)
// ---------------------------------------------------------------------------

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

/// Wait until `CONTACTS` holds something, and answer the opaque id.
async fn first_contact(term: &mut Term) -> String {
    for _ in 0..DRIVE_BUDGET {
        let mark = term.mark();
        term.send_line("CONTACTS").await;
        if term
            .expect_one_of_after(
                mark,
                &["bearing", NO_CONTACTS, COMMIT_PENDING, CONTROL_UNKNOWN],
            )
            .await
            == 0
        {
            let seen = strip_ansi(&term.seen[mark..]);
            if let Some(id) = seen
                .split(|c: char| !c.is_ascii_hexdigit())
                .find(|word| word.len() == 16)
            {
                return id.to_owned();
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("nothing was ever detected")
}

/// Every word that would betray the shooting to somebody outside the cockpit that
/// took it.
fn says_nothing_of_the_shooting(seen: &str, contact: &str, what: &str) {
    for leak in [contact, "Firing", "Locked on", "Locking", "round(s)"] {
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
///   still told nothing of the round that hit it — there is no damage or hit feed
///   in the cockpit at all (doc 16 §5 asks for one; the frame carries none).
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
    // inside the stock sensor — and that is **all** it holds: no name, no object
    // number, and not one word about the two rounds that hit it.
    let seen = ask(&mut bo, "CONTACTS", "bearing").await;
    names_nothing(&seen, red, blue, "the target's `CONTACTS`");
    let target = strip_ansi(&bo.seen[target_mark..]);
    for leak in [
        RED_TANK.to_string(),
        format!("#{red}"),
        contact.clone(),
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
/// What this does **not** cover, and why: the TUI binds no screen to
/// `tacsim.frame`, so there is no full-screen cockpit panel for this cut to assert
/// about. The pilot's arena window on a terminal is `VIEW`'s line-rendered grid,
/// which the test above asserts.
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

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

use support::wait_until;

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

use support::{STEP_TIMEOUT, Term, names_side, play_client_sized, scratch_dir, strip_ansi};

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

const GM_PASSWORD: &str = "gm-secret-1";
const PILOT_PASSWORD: &str = "pilot-secret-1";

/// The world's transient authoritative refusal: a commit is in flight.
const COMMIT_PENDING: &str = "please retry in a moment";
/// `BOARD`'s one transient: the control-lease mirror is refreshed by a 1 Hz tick.
const CONTROL_UNKNOWN: &str = "control state unknown";
/// How many times a line is re-sent on a transient before the test gives up.
const RETRY_BUDGET: usize = 40;

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
/// **The one refusal for a squad a pilot may not order** (16b cut Y.1), spelled
/// out for the same reason.
const NO_SUCH_SQUAD: &str = "No such squad.";

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
    /// **16b cut V.3**: the store `boot` made, so the report column can read the
    /// black box the world wrote and compare what each pilot is told with it.
    store: Arc<dyn ObjectStore>,
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
    cfg.packs = observer_packs(OBSERVER_CLASSIFY_MS);
    cfg
}

/// Keep the provisional phase long enough for its SSH/web assertions. This is
/// an explicitly authored test sensor and unit, selected only by the two tests
/// that inspect the transition; shipped definitions remain unchanged.
const OBSERVER_CLASSIFY_MS: u32 = 15_000;

fn observer_packs(classify_after_ms: u32) -> Vec<tacsim::config::PackSource> {
    use tacsim::definitions::content;

    fn signed(mut document: Value, name: &str) -> tacsim::config::PackSource {
        document["manifest"]["digest"] = Value::String("0".repeat(64));
        let text = serde_json::to_string(&document).expect("test pack JSON");
        let digest = mush_content::parse_pack(&text)
            .expect("test pack shape")
            .computed_digest()
            .expect("test pack digest")
            .to_hex();
        document["manifest"]["digest"] = Value::String(digest);
        tacsim::config::PackSource {
            name: name.to_owned(),
            bytes: serde_json::to_string(&document).expect("signed test pack JSON"),
        }
    }

    let mut sensor: Value = serde_json::from_str(content::VISUAL_MK1_JSON).unwrap();
    sensor["manifest"]["id"] = Value::String("tacsim.pack.observer_slow".to_owned());
    let mut entry = sensor["entries"]
        .as_object_mut()
        .unwrap()
        .remove("tacsim.sensor.visual_mk1")
        .unwrap();
    entry["def"]["classify_after_ms"] = Value::from(classify_after_ms);
    entry["def"]["note"] = Value::String("Test-controlled provisional-contact window.".to_owned());
    sensor["entries"]["tacsim.sensor.observer_slow"] = entry;

    let unit_source = content::LINE_TANK_JSON
        .replace("tacsim.pack.line_tank", "tacsim.pack.observer_tank")
        .replace("tacsim.unit.line_tank", "tacsim.unit.observer_tank")
        .replace("tacsim.pack.visual_mk1", "tacsim.pack.observer_slow")
        .replace("tacsim.sensor.visual_mk1", "tacsim.sensor.observer_slow");
    let unit: Value = serde_json::from_str(&unit_source).unwrap();
    let mut sources = content::fielded_sources();
    sources.push(signed(sensor, "observer_slow.json"));
    sources.push(signed(unit, "observer_tank.json"));
    sources
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
    mods.extend(modules::sheet_modules(onboarding, &Default::default()));
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
    boot_grace(tag, designate, SessionConfig::default().linkdead_grace).await
}

/// The same, with the session layer's link-dead grace named too (16b cut Y.4),
/// so a test can drop a pilot past it in seconds and not minutes.
async fn boot_grace(tag: &str, designate: bool, grace: Duration) -> Server {
    boot_over(tag, designate, grace, None).await
}

/// **The same, over a store an earlier server wrote** (16b cut AF.3): `reuse`
/// boots the composition again over that store and seeds nothing — the
/// restart half of the resume's lens, as `sim_rematch.rs` boots twice.
async fn boot_over(
    tag: &str,
    designate: bool,
    grace: Duration,
    reuse: Option<Arc<dyn ObjectStore>>,
) -> Server {
    let dir = scratch_dir(tag);
    let seed = reuse.is_none();
    let store: Arc<dyn ObjectStore> = reuse.unwrap_or_else(|| Arc::new(InMemoryStore::new()));
    if seed {
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
    }

    let mut world = load_or_seed_world(&*store).await.expect("boot world");
    for (id, name) in [(ADA, "Ada"), (BO, "Bo"), (CY, "Cy")] {
        if !seed {
            break;
        }
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

    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: grace,
        ..SessionConfig::default()
    }));
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
        store,
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
    login_sized(addr, account, password, character, 80, 24).await
}

/// [`login`] on a PTY of `cols` by `rows`. A witness that opens `@tui` logs in
/// at 120 by 34, the floor (16b cut AB.1): narrower, the TUI paints one sentence
/// and no screen at all.
async fn login_sized(
    addr: SocketAddr,
    account: &str,
    password: &str,
    character: &str,
    cols: u32,
    rows: u32,
) -> (russh::client::Handle<support::AcceptAll>, Term) {
    let (handle, mut term) = play_client_sized(addr, cols, rows).await;
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

/// **A `SIM HISTORY` row holds its public words and nothing else** (16b cut V.2).
///
/// `flat` is the answer with its whitespace collapsed (the session wraps a row
/// past eighty columns). The row runs from `head` — its id and outcome words —
/// to its `UTC`, and on to the `(you flew ..)` suffix when `flew` is `Some`;
/// every word between the two must be one the pinned format prints there — the
/// duration's `<n> h|m|s` pairs, `<red> v <blue>`, the date and the time — so a
/// word the format does not print is a leak, whatever it is.
fn history_row_is_public(flat: &str, id: u64, head: &str, flew: Option<&str>) {
    let start = flat
        .find(head)
        .unwrap_or_else(|| panic!("no row starting {head:?} in: {flat}"));
    let rest = flat.get(start + head.len()..).unwrap_or_default();
    let tail = flew.map_or_else(
        || " UTC".to_owned(),
        |callsign| format!(" UTC (you flew {callsign})"),
    );
    let end = rest
        .find(&tail)
        .unwrap_or_else(|| panic!("row #{id} does not end {tail:?}: {flat}"));
    let words: Vec<&str> = rest
        .get(..end)
        .unwrap_or_default()
        .split_whitespace()
        .collect();
    let digits = |word: &str| !word.is_empty() && word.bytes().all(|b| b.is_ascii_digit());
    // The duration: one or more `<n> <unit>` pairs.
    let mut at = 0;
    while let (Some(n), Some(unit)) = (words.get(at), words.get(at + 1)) {
        if digits(n) && ["h", "m", "s"].contains(unit) {
            at += 2;
        } else {
            break;
        }
    }
    assert!(at >= 2, "row #{id} has no duration: {words:?}");
    // Then `<red> v <blue>`, the date and the time, and nothing else.
    let date = |word: &str| {
        let parts: Vec<&str> = word.split('-').collect();
        parts.len() == 3 && parts.iter().all(|part| digits(part))
    };
    let clock = |word: &str| {
        word.split_once(':')
            .is_some_and(|(hours, minutes)| digits(hours) && digits(minutes))
    };
    assert!(
        matches!(
            words.get(at..).unwrap_or_default(),
            [red, "v", blue, day, time]
                if digits(red) && digits(blue) && date(day) && clock(time)
        ),
        "row #{id} carries a word its public format does not print: {words:?}"
    );
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

/// The next `arena.cockpit` keyframe this socket is pushed in the **grid** shape
/// — the one `VIEW` sends — answering the keepalive along the way (16b cut
/// B.4).
async fn pushed_view(ws: &mut Ws) -> Value {
    for _ in 0..600 {
        let env = recv_env(ws).await;
        match &env.event {
            OutputEvent::Panel { feed, data, .. }
                if feed.as_str() == COCKPIT_FEED
                    && data.pointer("/schema/shape").and_then(Value::as_str) == Some("grid") =>
            {
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
    panic!("`VIEW` never reached the panel client")
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
    take_off_with_sensor(ada, bo, false).await;
}

async fn take_off_with_sensor(ada: &mut Term, bo: &mut Term, controlled_delay: bool) {
    say(ada, "SIM TEAM red", "takes red slot 1").await;
    say(bo, "SIM TEAM blue", "takes blue slot 1").await;
    if controlled_delay {
        say(ada, "SIM UNIT observer_tank", "will field observer_tank").await;
        say(bo, "SIM UNIT observer_tank", "will field observer_tank").await;
    }
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
    wait_until(STEP_TIMEOUT, "the tank to accept its heading", async || {
        let mark = term.mark();
        term.send_line(&format!("HEADING {heading}")).await;
        term.expect_one_of_after(
            mark,
            &[
                &format!("Steering to {heading}"),
                "Power not online",
                // 16b cut F.1: the same refusal while the startup is running.
                "Power is still coming up",
                COMMIT_PENDING,
                CONTROL_UNKNOWN,
            ],
        )
        .await
            == 0
    })
    .await;
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
    take_off_with_sensor(&mut ada, &mut bo, true).await;

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

    // **16b cut B.4** (`B-14`): the attach look is the core `room` query, and it
    // is held to what `look` is held to. The bystander attaches a SECOND
    // connection: it is shown her own room through the same perception hooks —
    // the muster, the undeployed tanks, and nothing about the two in custody —
    // and her first connection is shown nothing of it.
    let first_mark = cy.mark();
    let (_c2, mut cy_again) = login(server.addr, "cy", PILOT_PASSWORD, "Cy").await;
    cy_again.expect(MUSTER).await;
    cy_again.expect("Red Tank 2").await;
    let attached = strip_ansi(&cy_again.seen);
    names_nothing(
        &attached,
        red,
        blue,
        "the bystander's second connection's attach",
    );
    // The first connection, fenced by a parser refusal of its own: a room the
    // attach had sent it would be queued before that `Huh?`. The refusal is a
    // command's answer and so reaches both connections; the room did not.
    // The fence must land (a deadline, not a budget that can run out quietly):
    // without its `Huh?` the absence check below would prove nothing.
    wait_until(STEP_TIMEOUT, "the first connection's fence", async || {
        let mark = cy.mark();
        cy.send_line(SENTINEL).await;
        cy.expect_one_of_after(mark, &[HUH, COMMIT_PENDING]).await == 0
    })
    .await;
    let first = strip_ansi(&cy.seen[first_mark..]);
    assert!(
        !first.contains(MUSTER),
        "the second connection's attach printed the room on the first:\n{first}"
    );
    drop(cy_again);

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
    // **16b cut U.4 (16c `T-67`)**: `@find`, the listing that hands out the
    // dbref every other staff verb takes. The tanks are God's (the arena applier
    // makes them as the booting actor), so this is their owner's `@find` as well
    // as staff's: a held unit is counted out and named nowhere.
    let seen = ask(&mut god, &format!("@find {RED_TANK}"), "END OF LIST").await;
    assert!(
        seen.contains("(0 found)"),
        "`@find` counted a held unit:\n{seen}"
    );
    // The terminal echoes the typed line, which names the tank; the answer is
    // everything after it.
    let answered: String = seen
        .lines()
        .filter(|line| !line.trim_start().starts_with("@find"))
        .collect::<Vec<_>>()
        .join("\n");
    names_nothing(&answered, red, blue, "the staffer's `@find`");

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

    // **16b cut V.1**: `STATUS` names the ground nearest her own pose — the ford,
    // twenty-six cells west of her re-posed cell on the same road. (Asked before
    // the battery's window below opens, because `STATUS` rightly names her own
    // tank, which `names_nothing` would refuse. Whole, fenced by the sentinel,
    // so its damage lines cannot spill into the battery's first answer.)
    let seen = ask_whole(&mut ada, "STATUS").await;
    assert!(
        seen.contains("nearest spot: The Ford (A), 260 m at 270°"),
        "`STATUS` names the nearest spot:\n{seen}"
    );
    // Everything she is sent from here to the end of the pre-detection battery
    // is held to `names_nothing` at the end of it.
    let battery_mark = ada.mark();

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
        // 16b cut V.1: the second resolution arm tells nothing either — a
        // designator of nothing and a spot that does not exist are one answer.
        "RANGE ZZ",
        "RANGE The Fjord",
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
        "RANGE ZZ",
        "RANGE The Fjord",
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
    // **16b cut Y.1: `ORDER` refuses the squad before it looks at a contact.**
    //
    // This match fields no machine, so nobody holds a squad, and authority is
    // refused before the target is resolved: an `ENGAGE` at a tank that is out
    // there and undetected, an `ENGAGE` at nothing, the other side's squad and a
    // squad that does not exist all answer one byte-identical `No such squad.`
    // — a pilot who may not order learns nothing about the scope by naming a
    // contact, and nothing about the other side by naming its squads.
    //
    // **16b cut Z.2**: and so do the two squad roles and the release — a
    // `SUPPORT` naming the other side's callsign is refused on the squad before
    // the callsign is looked at, so it says nothing of whether `BA` flies — and
    // no answer names a tank.
    let orders = [
        "ORDER red-1 ENGAGE BA",
        "ORDER red-1 ENGAGE ZZ",
        "ORDER blue-1 HOLD",
        "ORDER red-9 HOLD",
        "ORDER red-1 SUPPORT BA",
        "ORDER red-1 SCREEN 90",
        "ORDER red-1 RELEASE",
    ];
    let mut answers: Vec<String> = Vec::new();
    for line in orders {
        let seen = ask(&mut ada, line, NO_SUCH_SQUAD).await;
        names_nothing(&seen, red, blue, line);
        answers.push(seen.trim_start_matches(line).trim().to_owned());
    }
    // The battery is one pilot typing fast: the session's flood bucket (ten
    // lines a second, thirty to a burst) is what the three Z.2 lines above
    // spent, and the later `STARTUP` / `HEADING` burst would otherwise meet
    // `You're doing that too fast.` Half a second refills five lines.
    tokio::time::sleep(Duration::from_millis(500)).await;
    for (line, answer) in orders.into_iter().zip(answers.iter()) {
        assert_eq!(
            answer, NO_SUCH_SQUAD,
            "{line} answered something other than the one byte-identical squad refusal"
        );
    }
    // With no lock and no name the two read verbs say their own line, which is
    // NOT `FIRE`'s: `FIRE` takes a weapon and never a contact.
    for line in ["RANGE", "BEARING"] {
        let seen = ask(&mut ada, line, "No lock").await;
        assert!(
            seen.contains("No lock: LOCK a contact first, or name one."),
            "{line} with neither a lock nor a name:\n{seen}"
        );
    }
    // **16b cut V.1**: a named spot answers, before any detection, because the
    // ground is surveyed: the ford is 26 cells due west of her.
    let seen = ask(&mut ada, "RANGE The Ford", "Range to The Ford (A): ").await;
    assert!(
        seen.contains("Range to The Ford (A): 260 m."),
        "`RANGE The Ford` before detection:\n{seen}"
    );
    // The window's own lines arrive after the panel: the named spots' line comes
    // last, and all five spots are off a window that runs from column 71 to 109.
    // The line wraps, and a needle taken from its opening could return before
    // the wrapped rest has arrived, so the answer is read whole (to the
    // sentinel's refusal) and the sentence is cut at its one full stop.
    let seen = ask_whole(&mut ada, "VIEW").await;
    names_nothing(&seen, red, blue, "the pilot's `VIEW`");
    assert!(
        !seen.contains(" x ") && !seen.contains("hostile"),
        "no hostile marker before detection:\n{seen}"
    );
    // The terminal wraps the line at eighty columns, so it is read back from
    // its opening to the end of the answer with the wraps undone.
    let off = seen
        .find("Off the window:")
        .and_then(|at| seen.get(at..))
        .and_then(|rest| rest.find('.').and_then(|stop| rest.get(..=stop)))
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ");
    assert_eq!(
        off,
        "Off the window: A The Ford, 260 m at 270°; B The Crossroads, 390 m at 090°; \
         C North Barn, 980 m at 009°; E East Compound, 1290 m at 066°; \
         F The Bluff, 560 m at 075°.",
        "the off-window line names the spots, in authored order:\n{seen}"
    );
    // Spots only: every clause is a spot's letter and name, and nothing else.
    for clause in off.trim_start_matches("Off the window: ").split("; ") {
        assert!(
            [
                "A The Ford, ",
                "B The Crossroads, ",
                "C North Barn, ",
                "E East Compound, ",
                "F The Bluff, "
            ]
            .iter()
            .any(|spot| clause.starts_with(spot)),
            "the off-window line named something that is not a spot: {clause:?}"
        );
    }
    // **16b cut AE.1**: the fifth spot, one letter, read from each pilot's own
    // pose. The Bluff (144, 111) is 54 cells east and 14 north of Ada at
    // (90, 97) — 557 m, 560 to the ten, at 075° — and 16 cells west and 14 north
    // of Bo at (160, 97) — 212 m, 210 to the ten, at 311°. Bo's window (columns
    // 141 to 179, rows to 106) does not draw it either, so his off-window line
    // carries it too, under the same letter.
    let seen = ask(&mut ada, "RANGE F", "Range to The Bluff (F): ").await;
    assert!(seen.contains("Range to The Bluff (F): 560 m."), "{seen}");
    let seen = ask(&mut bo, "RANGE The Bluff", "Range to The Bluff (F): ").await;
    assert!(seen.contains("Range to The Bluff (F): 210 m."), "{seen}");
    let seen = ask(&mut bo, "BEARING f", "Bearing to The Bluff (F): ").await;
    assert!(seen.contains("Bearing to The Bluff (F): 311°"), "{seen}");
    let seen = ask_whole(&mut bo, "VIEW").await;
    names_nothing(&seen, red, blue, "Bo's `VIEW`");
    let bo_off = seen
        .find("Off the window:")
        .and_then(|at| seen.get(at..))
        .and_then(|rest| rest.find('.').and_then(|stop| rest.get(..=stop)))
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ");
    assert!(
        bo_off.contains("F The Bluff, 210 m at 311°"),
        "Bo's off-window line carries the fifth spot under its letter:\n{seen}"
    );
    assert_eq!(
        bo_off.matches("The Bluff").count(),
        1,
        "one letter, once:\n{seen}"
    );
    // And the whole battery — the refusals, the named spot, the window — says
    // nothing about either tank.
    names_nothing(
        &strip_ansi(&ada.seen[battery_mark..]),
        red,
        blue,
        "the pre-detection battery",
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
    // against `classify_after_ms`, explicitly sized by this fixture, and losing it would
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
    // The test-configured observation window tells the return apart. The row
    // then reads the target's public callsign, the rename is one line quoting
    // what the pilot has been calling it, the new spelling is accepted
    // case-folded, and the **provisional number the lock was taken under still
    // resolves** — which is the whole reason the book keeps it.
    // The *row* has to be re-read after the rename, not the transcript: the
    // `identified` notice arrives on its own and a window that merely contains
    // the word `blue` may still carry the `X<n>` row printed before it.
    let mut classified = String::new();
    wait_until(
        STEP_TIMEOUT,
        "the controlled sensor to classify its contact",
        async || {
            classified = ask(&mut ada, "CONTACTS", "(rel ").await;
            classified.contains("blue")
        },
    )
    .await;
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
    // is inside the window, which is narrower (39 cells, 16b cut B.4) than the sensor reaches
    // (60 cells each way), so the two go on closing until it is.
    // A marker is drawn when the window's `legend:` line names a contact glyph
    // (the legend lists only the markers the window drew) and nothing plots
    // beyond it; "contact" alone would match the beyond-window line, which is
    // said exactly when the one held contact is NOT drawn. The whole answer is
    // read (`ask_whole`), so the absence check below runs on all of it.
    let mut window = String::new();
    wait_until(
        STEP_TIMEOUT,
        "the window to draw a marker for the contact it holds",
        async || {
            let seen = ask_whole(&mut ada, "VIEW").await;
            let drawn = seen
                .lines()
                .filter(|line| line.contains("legend:"))
                .any(|line| {
                    ["x hostile", "? contact", "+ friendly"]
                        .iter()
                        .any(|entry| line.contains(entry))
                });
            if drawn && !seen.contains("beyond this window") {
                window = seen;
                true
            } else {
                false
            }
        },
    )
    .await;
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

    // **16b cut U.4**: and once the check-in has cleared the markers, `@find`
    // names the tank again.
    wait_until(
        STEP_TIMEOUT,
        "`@find` to list the checked-in tank",
        async || {
            let seen = ask(&mut god, &format!("@find {RED_TANK}"), "END OF LIST").await;
            seen.contains(&format!("(#{red})")) && seen.contains("(1 found)")
        },
    )
    .await;

    // **16b cut V.3 (16c N2)**: and the debrief of a match in which nothing was
    // fired names nothing but the reader's own tank — no other callsign, no
    // shot, and none of what the pilot's platform never learned.
    let report = sim_report(&mut ada, "SIM REPORT").await;
    assert!(report.contains("You flew RA:"), "{report}");
    assert!(!report.contains("cannon at"), "{report}");
    // The one lock the tape holds was taken under the provisional number,
    // before the platform told the contact apart (the re-locks by the new
    // name journal nothing: it is the same contact), so it is named as it was
    // AT the lock — not as the platform knew it by the end.
    assert!(report.contains("locked a contact."), "{report}");
    for callsign in ["BA", "BB", "BC", "BD", "RB", "RC", "RD"] {
        assert!(
            !words_of(&report).contains(&callsign),
            "the report named {callsign}:\n{report}"
        );
    }
    report_is_the_pilots_own(&report, red, blue, "Ada's report of the approach");
}

/// **The observer rule with the switch on** (16b cut Q.1;
/// `[tacsim.sensors].designate_on_detection`).
///
/// The key is shipped as `false` into the rendered instance template and into
/// the instance template, so an operator can turn it on with a server-child restart and no
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
    take_off_with_sensor(&mut ada, &mut bo, true).await;

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
    wait_until(
        STEP_TIMEOUT,
        "the designated contact to arrive",
        async || {
            let mark = ada.mark();
            ada.send_line("CONTACTS").await;
            if ada
                .expect_one_of_after(
                    mark,
                    &["(rel ", NO_CONTACTS, COMMIT_PENDING, CONTROL_UNKNOWN],
                )
                .await
                == 0
            {
                held = strip_ansi(&ada.seen[mark..]);
                first_end = ada.seen.len();
                return true;
            }
            false
        },
    )
    .await;
    assert!(
        !held.is_empty(),
        "the two tanks never detected each other within the observation deadline"
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
    wait_until(
        STEP_TIMEOUT,
        "the cockpit to publish the lock",
        async || {
            ask_whole(&mut ada, "WEAPONSTATUS")
                .await
                .contains("Locked on")
        },
    )
    .await;
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
    wait_until(
        STEP_TIMEOUT,
        "the controlled sensor to classify its contact",
        async || {
            classified = ask(&mut ada, "CONTACTS", "(rel ").await;
            classified.contains("blue")
        },
    )
    .await;
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
    // **16b cut V.1**: and the ground's named spots, which are surveyed ground
    // exactly as the terrain is — the five the pack authors (the fifth is 16b
    // cut AE.1's), in its order, in arena cells, each with its map letter.
    // Places, never units.
    assert_eq!(
        map.get("spots"),
        Some(&json!([
            { "letter": "A", "name": "The Ford", "cell": [64, 97] },
            { "letter": "B", "name": "The Crossroads", "cell": [129, 97] },
            { "letter": "C", "name": "North Barn", "cell": [106, 194] },
            { "letter": "E", "name": "East Compound", "cell": [208, 148] },
            { "letter": "F", "name": "The Bluff", "cell": [144, 111] },
        ])),
        "the map names the shipped ground's five spots: {map}"
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
    // 16b cut B.1 (`T-86`): an old name on a notice is a name the pilot was
    // sent, and she has been sent none yet. This frame is taken before anything
    // is detected and carries no notices, so here the call is a GUARD on the
    // socket's surface and not the proof: any `was` at all would fail it. The
    // proof runs on frames `Arena::step` staged through a real rename and
    // regression — `tacsim/tests/perception.rs`'s
    // `b1_the_published_old_name_is_a_name_the_pilot_was_sent` — and on the
    // regression's published frame further down this file.
    every_was_is_a_held_designator(&cockpit, &[], "the cockpit frame");
    // `names_nothing` also refuses `#{red}` and `Red Tank 1`, so the two
    // assertions above are the whole rule on these two surfaces: a frame names
    // its own unit by the bare number it carries in `unit`, and never by a dbref
    // or a name.

    // (e) **16b cut B.4** (`B-24`, the `T-60` footer slip): `VIEW`'s raw payload,
    // as her panel client receives it, gained three keys — and every one is a
    // fact she already holds. `origin_cell` is the window's corner, read off her
    // OWN pose (corner plus `you` is her own cell on the pushed frame); `extent`
    // is the arena's size, which is lobby knowledge (doc 16 §3.2: the proving
    // ground is surveyed ground) and exactly the map she has just pulled;
    // `cell_columns` is how a line client prints a cell. None of them is about
    // the tank she has not found.
    let printed = ask(&mut ada, "VIEW", "Arena window").await;
    assert!(
        printed.contains("m a cell, north up"),
        "the line client's footer is in arena cells: {printed}"
    );
    names_nothing(&printed, red, blue, "the pilot's printed `VIEW`");
    let view = pushed_view(&mut ada_web).await;
    assert_eq!(view.get("cell_columns"), Some(&json!(2)), "{view}");
    let pair = |value: &Value, key: &str| -> Option<(u64, u64)> {
        let cell = value.get(key)?.as_array()?;
        Some((cell.first()?.as_u64()?, cell.get(1)?.as_u64()?))
    };
    let (ox, oy) = pair(&view, "origin_cell").expect("the window's corner");
    let (yx, yy) = pair(&view, "you").expect("her own marker");
    assert_eq!(
        Some((ox + yx, oy + yy)),
        pair(&cockpit, "cell"),
        "the corner plus her marker is her own cell and nothing else: {view}"
    );
    assert_eq!(
        pair(&view, "extent"),
        Some((
            map.get("width").and_then(Value::as_u64).unwrap_or(0),
            map.get("height").and_then(Value::as_u64).unwrap_or(0),
        )),
        "the extent is the ground she was already given: {view}"
    );
    let units = view
        .get("units")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(units.len(), 1, "her own marker and no other: {view}");
    let whole = view.to_string();
    no_sixteen_digit_word(&whole, "the raw `VIEW` payload");
    for leak in [
        BLUE_TANK.to_string(),
        format!("#{blue}"),
        "Bo".to_string(),
        "\"BA\"".to_string(),
    ] {
        assert!(
            !whole.contains(&leak),
            "the raw `VIEW` payload leaked {leak:?}:\n{whole}"
        );
    }

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

    // **16b cut B.5 (`T-31`)**: the survivors live in the per-feed latest slot,
    // not the ring. The ring (read alone through the backlog, which never sees
    // the slot) holds the two lines and not one frame.
    let ring_alone = manager
        .backlog_matching(sess, None, 1_000, |_| true)
        .expect("the ring is readable");
    assert_eq!(
        ring_alone.lines.len(),
        2,
        "the ring holds the two lines and no marked keyframe"
    );
    assert!(
        ring_alone
            .lines
            .iter()
            .all(|envelope| !matches!(envelope.event, OutputEvent::Panel { .. })),
        "no cockpit or map frame is a ring entry"
    );

    // And a REAL seated `arena.cockpit` frame — the committed fixture
    // `status_frame` blessed, the one all three clients are pinned against —
    // replaces the cleared one in the slot and is the one cockpit frame a
    // client resuming from before it is replayed.
    let seated: Value = serde_json::from_str(include_str!(
        "../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"
    ))
    .expect("the committed cockpit fixture parses");
    assert_eq!(
        seated.pointer("/schema/retain").and_then(Value::as_str),
        Some(mush_session::RETAIN_LATEST),
        "the fixture carries the marker the slot reads"
    );
    let before = manager.last_seq(sess).expect("the session");
    manager.send_event_to_char(CharKey(1), panel(COCKPIT_FEED, seated.clone()));
    let replayed = manager
        .replay_since(sess, before)
        .expect("a slot replacement is never TooOld");
    assert_eq!(replayed.len(), 1, "the seated frame alone");
    assert!(
        matches!(
            &replayed[0].event,
            OutputEvent::Panel { feed, data, .. } if feed.as_str() == COCKPIT_FEED && *data == seated
        ),
        "the replayed frame is the seated one, byte for byte"
    );
    let from_zero = manager.replay_since(sess, 0).expect("never TooOld");
    let cockpit_frames: Vec<&Value> = from_zero
        .iter()
        .filter_map(|envelope| match &envelope.event {
            OutputEvent::Panel { feed, data, .. } if feed.as_str() == COCKPIT_FEED => Some(data),
            _ => None,
        })
        .collect();
    assert_eq!(
        cockpit_frames,
        vec![&seated],
        "from zero, the seated frame is the only cockpit frame — the cleared ones are gone"
    );
    assert_eq!(
        manager
            .backlog_matching(sess, None, 1_000, |_| true)
            .expect("the ring is readable")
            .lines
            .len(),
        2,
        "and it never entered the ring"
    );
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
    wait_until(STEP_TIMEOUT, "the tank to come online", async || {
        ask(term, "STATUS", "power").await.contains("power online")
    })
    .await;
}
/// Wait until `CONTACTS` holds something, and answer the row's short designator
/// (16b cut Q.1).
async fn first_contact(term: &mut Term) -> String {
    let mut contact = None;
    wait_until(
        STEP_TIMEOUT,
        "the sensor to publish a contact",
        async || {
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
                contact = designator_of(&seen);
            }
            contact.is_some()
        },
    )
    .await;
    contact.expect("waited for a contact")
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

// ---------------------------------------------------------------------------
// 16b cut V.3 — the report column
// ---------------------------------------------------------------------------

/// **Every event of one match's black box**, read back from the store the world
/// wrote it to — `arena_story.rs`' walk, copied because an integration test
/// cannot import another.
async fn ledger_events(store: &Arc<dyn ObjectStore>, match_id: u64) -> Vec<Value> {
    let log = format!("match.{match_id}");
    let mut events = Vec::new();
    for chunk in 0..256 {
        let key = mush_core::chunk_key(&log, chunk);
        let Some(value) = store
            .module_json_get("tacsim", key.as_str())
            .await
            .expect("the store read")
        else {
            break;
        };
        let mut lines: Vec<String> = Vec::new();
        collect_lines(&value, &mut lines);
        for blob in lines {
            for line in blob.lines().filter(|line| !line.trim().is_empty()) {
                events.push(
                    serde_json::from_str::<Value>(line).unwrap_or_else(|error| {
                        panic!("a ledger line is not JSON ({error}): {line}")
                    }),
                );
            }
        }
    }
    events
}

/// Every `lines` string in a chunk document, depth first (`arena_story.rs`').
fn collect_lines(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                match (key.as_str(), child.as_str()) {
                    ("lines", Some(text)) => out.push(text.to_owned()),
                    _ => collect_lines(child, out),
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_lines(item, out);
            }
        }
        _ => {}
    }
}

/// **What a whole report ends on**: its tail's end line, which is the last thing
/// the completion says.
const REPORT_ENDS: [&str; 3] = [
    "the match was aborted.",
    "the match ended: ",
    "the match was voided.",
];

/// **`SIM REPORT`, read whole** (16b cut V.3).
///
/// The report document rides the same batch as the result, and the store's read
/// lane is not ordered against its write lane, so a read that outruns the commit
/// answers `still being written` — the honest wait is to ask again, which is
/// what [`wait_until`] does here, never a sleep. Once the end line has arrived a
/// sentinel is sent and read to its refusal, so everything the completion said
/// is in the slice.
async fn sim_report(term: &mut Term, line: &str) -> String {
    let mut report = String::new();
    wait_until(STEP_TIMEOUT, "the report to be read whole", async || {
        let mark = term.mark();
        term.send_line(line).await;
        let mut needles: Vec<&str> = REPORT_ENDS.to_vec();
        needles.extend(["still being written", "already being read", COMMIT_PENDING]);
        let at = term.expect_one_of_after(mark, &needles).await;
        if at >= REPORT_ENDS.len() {
            return false;
        }
        term.send_line(SENTINEL).await;
        term.expect_after(mark, HUH).await;
        report = strip_ansi(&term.seen[mark..]);
        true
    })
    .await;
    report
}

/// **`@arena/report`, read whole** (16b cut V.3): the page's header opens with
/// the seed, and a sentinel read to its refusal after it brings in the rest of
/// what the completion said.
async fn staff_page(term: &mut Term, line: &str) -> String {
    let mark = term.mark();
    say(term, line, "seed ").await;
    term.send_line(SENTINEL).await;
    term.expect_after(mark, HUH).await;
    strip_ansi(&term.seen[mark..])
}

/// The words of a transcript, split on everything that is not a letter or a
/// digit — what a callsign is looked for among.
fn words_of(text: &str) -> Vec<&str> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect()
}

/// **A report names no unit its platform had not told apart** (16b cut V.3):
/// every callsign in `report` other than `own` is one `viewer` classified on
/// the tape.
fn names_only_what_it_classified(
    report: &str,
    own: &str,
    viewer: u32,
    callsigns: &[(&str, u32)],
    events: &[Value],
) {
    let words = words_of(report);
    for (callsign, unit) in callsigns {
        if *callsign == own || !words.contains(callsign) {
            continue;
        }
        assert!(
            events.iter().any(|event| {
                event.get("kind").and_then(Value::as_str) == Some("classified")
                    && event.get("viewer").and_then(Value::as_u64) == Some(u64::from(viewer))
                    && event.get("target").and_then(Value::as_u64) == Some(u64::from(*unit))
            }),
            "the report of #{viewer} names {callsign}, which its platform never told \
             apart:\n{report}"
        );
    }
}

/// **Nothing a pilot's report may carry** (16b cut V.3; 16c N2): no contact id,
/// no draw, no seed, no hash, no range, no damage figure, no object number.
fn report_is_the_pilots_own(report: &str, red: u32, blue: u32, what: &str) {
    no_sixteen_digit_word(report, what);
    for leak in ["seed", "hash", "draw", " m:", "damage"] {
        assert!(!report.contains(leak), "{what} carried {leak:?}:\n{report}");
    }
    for tank in [red, blue] {
        assert!(
            !report.contains(&format!("#{tank}")),
            "{what} named an object:\n{report}"
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
    wait_until(
        STEP_TIMEOUT,
        "the cockpit to publish the lock",
        async || {
            ask_whole(&mut ada, "WEAPONSTATUS")
                .await
                .contains("Locked on")
        },
    )
    .await; // One round at a time, paced by the cockpit's own count: `FIRE` pre-checks
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
        wait_until(
            STEP_TIMEOUT,
            "the fired round to leave the magazine",
            async || {
                spent = ask_whole(&mut ada, "WEAPONSTATUS").await;
                spent.contains(&want)
            },
        )
        .await;
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
    // The shooter's magazine counts a round when it leaves the barrel; the
    // target is told when it lands, a flight and a frame later, pushed on its
    // own channel. So the notice is waited for (`expect_after` reads the channel
    // until it arrives) and never read once — a read that races the impact
    // fails only under load, when the other servers of this binary slow the
    // world between the two.
    bo.expect_after(target_mark, "Hit on the ").await;
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
    //
    // The impact the target was told of above tells the shooter too, on the
    // shooter's own channel, which nothing orders against the target's: waited
    // for as the target's was. The shooter is never sent `Hit on the `, so the
    // first `Hit on ` in the transcript is the shooter's own sentence.
    ada.expect("Hit on ").await;
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
    wait_until(
        STEP_TIMEOUT,
        "the scan's complete condition bands",
        async || {
            scanned = ask(&mut ada, "SCAN", &shot_at).await;
            scanned.contains("turret")
        },
    )
    .await;
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
    wait_until(
        STEP_TIMEOUT,
        "the checked-in tank's damage to persist",
        async || {
            marked = ask_whole(&mut god, &format!("@unit/show #{blue}")).await;
            damaged_facet(&marked)
        },
    )
    .await;
    assert!(
        !marked.is_empty(),
        "the checked-in tank carries no mark of the two rounds that hit it:\n{}",
        strip_ansi(&god.seen[god.seen.len().saturating_sub(3_000)..])
    );

    // ---- the report column (16b cut V.3; 16c N2, Q37) -------------------
    //
    // After the match each pilot reads their own debrief, and it is the same
    // fog as the cockpit: the shooter's own shots and their outcomes, the
    // target's own hits, and nothing of the other side's — no shooter, no
    // range, no damage figure, no contact id, no object number, no draw and no
    // seed. A callsign in either report is one that pilot's platform told apart
    // on the tape the world wrote.
    // A chunk reaches the store only at 16 KiB or at the end's flush, and the
    // store's read lane is not ordered against its write lane, so the tape is
    // read once its terminal `aborted` line is there (`arena_custody.rs`' wait):
    // a partial tape would make the classification set below too small.
    let mut events: Vec<Value> = Vec::new();
    wait_until(STEP_TIMEOUT, "the flushed black box", async || {
        events = ledger_events(&server.store, 1).await;
        events
            .iter()
            .any(|event| event.get("kind").and_then(Value::as_str) == Some("aborted"))
    })
    .await;
    assert!(
        events
            .iter()
            .any(|event| event.get("kind").and_then(Value::as_str) == Some("fired")),
        "the tape holds the shots"
    );
    let callsigns = [("RA", red), ("BA", blue)];
    let adas = sim_report(&mut ada, "SIM REPORT").await;
    assert!(adas.contains("cannon at"), "{adas}");
    assert!(adas.contains(": hit"), "{adas}");
    assert!(adas.contains("You flew RA:"), "{adas}");
    report_is_the_pilots_own(&adas, red, blue, "Ada's report");
    names_only_what_it_classified(&adas, "RA", red, &callsigns, &events);
    let bos = sim_report(&mut bo, "SIM REPORT").await;
    assert!(bos.contains("hit on the "), "{bos}");
    assert!(bos.contains("You flew BA:"), "{bos}");
    assert!(bos.contains("You fired nothing."), "{bos}");
    for leak in ["cannon at", "You fired 1", " at "] {
        assert!(
            !bos.contains(leak),
            "the target's report carried {leak:?}:\n{bos}"
        );
    }
    assert!(
        !words_of(&bos).contains(&"RA"),
        "the target's report named the shooter:\n{bos}"
    );
    report_is_the_pilots_own(&bos, red, blue, "Bo's report");
    names_only_what_it_classified(&bos, "BA", blue, &callsigns, &events);

    // A bystander in the centre flew nothing, and is told exactly that.
    let listing = ask(&mut god, "@find Simulator Lobby", "END OF LIST").await;
    let lobby = dbref_of(&listing, "Simulator Lobby");
    say(
        &mut god,
        &format!("@teleport #{}=#{lobby}", CY.0),
        "Teleported",
    )
    .await;
    let said = ask(&mut cy, "SIM REPORT 1", "did not fly").await;
    assert!(said.contains("You did not fly in match 1."), "{said}");

    // Staff read the tape itself: the seed and the object numbers are theirs.
    let page = staff_page(&mut god, "@arena/report 1").await;
    assert!(page.contains("seed "), "{page}");
    assert!(page.contains(&format!("#{blue}")), "{page}");
    assert!(page.contains("fired unit=RA("), "{page}");
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
/// position and the spaces between them are never sent: `Blue Tank 6` arrives as
/// `BlueTank6` once the escape sequences are stripped. Every assertion about TUI
/// text therefore runs against [`squeezed`] output, and this is the needle that
/// says a room render has finished arriving.
const TUI_LAST_TANK: &str = "BlueTank6";

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
    let (_c, mut cy) = login_sized(server.addr, "cy", PILOT_PASSWORD, "Cy", 120, 34).await;
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

    // The control: the TUI shows the bystander all twelve tanks.
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
    let (_a, mut ada) = login_sized(server.addr, "ada", PILOT_PASSWORD, "Ada", 120, 34).await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, mut cy) = login_sized(server.addr, "cy", PILOT_PASSWORD, "Cy", 120, 34).await;
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
    // `Cy` marks the first paint after `@tui` (the strip's right-hand block
    // and the WHO rail both carry the name at 120 columns); it is the
    // named-tab needle below that proves the strip painted.
    cy.expect_after(opened, "Cy").await;
    let strip = squeezed(&cy.seen[opened..]);
    assert!(
        strip.contains("8damage9unit"),
        "the ungated tabs are on the strip:\n{strip}"
    );
    assert!(
        !strip.contains("0cockpit"),
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

    // ---- and when she climbs out (16b cut B.3, `B-63`, 16c Q21) ----------
    //
    // The tab goes with the seat: the cleared frame moves her off screen 0 to
    // the scene, the digit is inert afterwards, and the cockpit's empty state
    // is never painted — a screen that says `Not in a cockpit.` is a tab that
    // outlived the seat. The TUI paints only the cells that change, so the
    // screen she is on is read off the help overlay's first line (`scene
    // screen`), which names the active screen whole.
    ada.send_line("").await; // the box, empty: the focused map pane has no row
    let leaving = ada.mark();
    ada.send_line("EXIT").await;
    ada.expect_after(leaving, "climb").await;
    // Every tab is named at 120 columns; `scene` after the mark is the strip
    // re-painting the tab whose highlight the cleared frame moved, and the
    // help overlay below is what proves which screen is active.
    ada.expect_after(leaving, "scene").await;
    ada.send_line("").await; // an empty line: back to nav
    // `0` reaches nothing now, and ONE help overlay proves both halves: its
    // first line names the active screen whole (the TUI paints only the cells
    // that change, so the strip alone cannot say which tab is lit). Had the
    // cleared frame left her on the cockpit, or had `0` found a cockpit tab to
    // open, it would read `cockpit screen`. The overlay is opened once and
    // read before anything closes it: a close and a re-open land in one paint
    // with no difference in it, so there would be nothing to read.
    let pressed = ada.mark();
    ada.send_line("0").await; // the digit, inert; its Return opens the box
    ada.send_line("").await; // back to nav
    ada.send_line("?").await; // help; the Return is swallowed while it is up
    ada.expect_after(pressed, "screen").await;
    let after_zero = squeezed(&ada.seen[pressed..]);
    assert!(
        after_zero.contains("scenescreen") && !after_zero.contains("cockpitscreen"),
        "the cleared frame left her on the cockpit, or the digit reached one:\n{after_zero}"
    );
    for cockpit_word in ["Callsign", "Waitingforthearena"] {
        assert!(
            !after_zero.contains(cockpit_word),
            "a cockpit pane painted after the seat went ({cockpit_word}):\n{after_zero}"
        );
    }
    assert!(
        !squeezed(&ada.seen[opened..]).contains("Notinacockpit"),
        "the cockpit's empty state was painted: the tab outlived the seat"
    );
    // A fresh full screen, out of the tank: its entry queries ask the cockpit
    // feed and she is answered `seated: false`, which paints nothing. So wait
    // for a line painted AFTER that answer — a second `EXIT`, typed once the
    // screen is up, is answered after every entry query ahead of it (one
    // world, one inbox, one connection's replies in order) — and only then
    // read the strip: a tab that answer had wrongly raised would be on screen
    // by now, as `0 cockpit`.
    ada.send_line("?").await; // close the help; the Return opens the box
    ada.send_line("@notui").await;
    let reopened = ada.mark();
    ada.send_line("@tui").await;
    ada.expect_after(reopened, "Ada").await;
    ada.send_line("").await; // the box
    let probed = ada.mark();
    ada.send_line("EXIT").await;
    ada.expect_after(probed, "pod.").await;
    let strip = squeezed(&ada.seen[reopened..]);
    assert!(
        strip.contains("9unit") && !strip.contains("0cockpit"),
        "no tenth tab once out of the tank:\n{strip}"
    );
}

/// **`B-01`: the board is the observer rule too** (16b cut D.1; doc 16 §1, §5).
///
/// `SIM STATUS` is the one selector that answers in every state and from every
/// room of the centre **including the pods**, so it was the one surface on which
/// a seated pilot could read the other side's standing: `, pod Blue Pod 1,
/// seated` before, `, left` the moment the human on `BA` dropped and a machine
/// took the tank. No sensor of theirs told them any of it. Since cut Y.3 a drop
/// reads `flown by doctrine` on the full board and an `EXIT` `left`; the test
/// drops `BA`'s line and then climbs out, and the other side reads neither.
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
    // **16b cut Y.3**: and not the drop's word either — a dropped pilot's
    // row reads `flown by doctrine` on the full board, which is as much the
    // other side's standing as `left` is.
    for word in [", pod ", ", seated", ", left", "doctrine"] {
        assert!(
            !blue.contains(word),
            "a seated pilot read `{word}` about the other side: {board}"
        );
    }

    // **16b cut Y.3**: Bo's line drops — a clean `QUIT` is a drop, reclaimable,
    // and the machine flies BA meanwhile. Staff read `flown by doctrine`; Ada's
    // board does not move by one byte and carries neither word.
    bo.send_line("QUIT").await;
    support::wait_arena_pilot_out(&mut god, "BA", "Bo", "flown by doctrine").await;
    let dropped = strip_ansi(&ask(&mut ada, "SIM STATUS", "blue:").await);
    assert_eq!(
        row(&dropped, "blue:"),
        blue,
        "the other side's row moved when its pilot dropped:\n{dropped}"
    );
    for word in ["left", "doctrine"] {
        assert!(
            !row(&dropped, "blue:").contains(word),
            "a seated pilot read `{word}` about the other side's drop:\n{dropped}"
        );
    }

    // Bo comes back to the pod the drop left him in and climbs out, which
    // mid-match is final (even after a drop) and is exactly the fact `B-01` is
    // about. Ada's board does not move by one byte; staff read the truth.
    let (_b2, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    bo.expect("BOARD takes it back.").await;
    say(&mut bo, "EXIT", "climb").await;
    let after = strip_ansi(&ask(&mut ada, "SIM STATUS", "blue:").await);
    assert_eq!(
        row(&after, "blue:"),
        blue,
        "the other side's row moved when its pilot left:\n{after}"
    );
    for word in ["left", "doctrine"] {
        assert!(
            !row(&after, "blue:").contains(word),
            "a seated pilot read `{word}` about the other side mid-match:\n{after}"
        );
    }
    // The `EXIT` after the drop is the one-way route, so staff read `left`
    // and no longer the drop's word.
    let staff = strip_ansi(&ask(&mut god, "@arena/status", "blue:").await);
    assert!(
        row(&staff, "blue:").contains(", left") && !row(&staff, "blue:").contains("doctrine"),
        "staff were refused the whole board: {staff}"
    );
}

/// **16b cut D.2's three new victim lines name nothing** (doc 16 §1, §3.6).
///
/// `B-03` and `B-15` gave a pilot three sentences about their **own** tank —
/// it is a wreck, it can neither move nor shoot, a round found a section that
/// was already gone — and a pilot's own tank is outside the observer rule
/// entirely. What the rule still forbids is the other half of each of those
/// moments: where the round came from. So the check is on the **shape** as much
/// as on the words: none of the three types has a field a contact, an object
/// number, a bearing, a side or a weapon could be carried in, so there is
/// nothing for a later renderer to leak.
///
/// The frame-JSON half of this — that each of the seven new notice keys is
/// `null` on these three, and that none of them carries a sixteen-digit word —
/// is `tacsim`'s own `d2_the_notice_object_carries_the_sentence_s_own_parts`,
/// which has the one scene every renderer is held to.
#[test]
fn the_victims_own_lines_name_nothing_but_the_victims_own_tank() {
    use tacsim::arena::perception::{FrameNotice, NoticeKind};
    use tacsim::match_::cockpit;

    // (1) The two sentences, whole. Neither names a shooter, a side, a bearing,
    // a weapon or a contact, and neither carries a number at all.
    for line in [cockpit::OWN_DESTROYED, cockpit::OWN_DISABLED] {
        no_sixteen_digit_word(line, "a victim's own line");
        for leak in [
            "bearing",
            "contact",
            "red",
            "blue",
            "cannon",
            "machine_gun",
            "laser",
            "X1",
            "RA",
            "BA",
            "#",
        ] {
            assert!(
                !line.contains(leak),
                "the victim's own line leaked {leak:?}:\n{line}"
            );
        }
        assert!(
            !line.chars().any(|c| c.is_ascii_digit()),
            "the victim's own line carried a number:\n{line}"
        );
    }

    // (2) And the shapes: none of the three can carry a contact, so nothing a
    // renderer does with them can name one.
    let spent = FrameNotice::StruckSpent {
        section: smol_str::SmolStr::new("left_track"),
    };
    for notice in [
        FrameNotice::OwnDestroyed,
        FrameNotice::OwnDisabled,
        spent.clone(),
    ] {
        assert_eq!(notice.contact(), None, "{notice:?}");
        assert_eq!(notice.designator(), None, "{notice:?}");
        assert_eq!(notice.named(), None, "{notice:?}");
        assert_eq!(notice.weapon(), None, "{notice:?}");
        assert_eq!(notice.refusal(), None, "{notice:?}");
        assert_eq!(notice.cause(), None, "{notice:?}");
        assert_eq!(notice.order(), None, "{notice:?}");
    }
    // **16b cut B.1** (`T-86`) replaced cut D.2's "the victim's own section is
    // deliberately NOT on the gunner's `section` key" with the stronger pin:
    // on the victim's three kinds the section IS published, and it is one of
    // the pilot's OWN frame's `sections[].id` and never a contact's — the
    // notice names no contact at all. No reader highlights by the key alone;
    // `kind` says whose section it is. Pinned over the published JSON below —
    // a unit pin on `status_frame`'s copying; the same property over frames
    // `Arena::step` staged for a real victim is in `tacsim/tests/resolution.rs`'s
    // `a_section_is_destroyed_once_however_many_rounds_find_it_afterwards`.
    assert_eq!(
        spent.section().map(smol_str::SmolStr::as_str),
        Some("left_track")
    );
    assert_eq!(spent.was(), None);
    victims_sections_are_their_own();
    assert_eq!(spent.kind(), NoticeKind::StruckSpent);
    assert_eq!(NoticeKind::StruckSpent.as_str(), "struck_spent");
    assert_eq!(NoticeKind::OwnDestroyed.as_str(), "destroyed");
    assert_eq!(NoticeKind::OwnDisabled.as_str(), "disabled");
}

/// **The victim's `section`, on the wire, is the victim's own** (16b cut B.1,
/// `T-86`): on `struck`, `struck_spent` and `section_lost` the published
/// `section` is one of the pilot's OWN frame's `sections[].id`, and the notice
/// carries no `contact`, no `designator` and no `was` — so it is never a
/// contact's section, and nothing in it names the shooter.
fn victims_sections_are_their_own() {
    use tacsim::arena::perception::FrameNotice;
    use tacsim::arena::state::ContactId;
    use tacsim::arena::step::{FrameFacet, FrameSection};
    use tacsim::match_::cockpit;

    let word = smol_str::SmolStr::new;
    let mut frame = d4_stub_frame(ContactId(0x0123_4567_89ab_cdef));
    frame.sections = vec![
        FrameSection {
            id: word("hull"),
            condition: word("intact"),
            internals: Some((60, 60)),
            facets: vec![FrameFacet {
                id: word("front"),
                armor: 55,
                armor_max: 80,
            }],
        },
        FrameSection {
            id: word("left_track"),
            condition: word("destroyed"),
            internals: Some((0, 24)),
            facets: Vec::new(),
        },
    ];
    frame.notices = vec![
        FrameNotice::Struck {
            section: word("hull"),
            armor_section: word("hull"),
            facet: word("front"),
            armor_left: 55,
            armor_max: 80,
            internals_left: 60,
            internals_max: 60,
        },
        FrameNotice::StruckSpent {
            section: word("left_track"),
        },
        FrameNotice::SectionLost {
            section: word("left_track"),
            consequences: Vec::new(),
        },
    ];
    let payload = cockpit::status_frame(tacsim::arena_task::MatchId(1), &frame);
    let own: Vec<Value> = payload["sections"]
        .as_array()
        .map(|sections| sections.iter().map(|s| s["id"].clone()).collect())
        .unwrap_or_default();
    let notices = payload["notices"].as_array().cloned().unwrap_or_default();
    assert_eq!(notices.len(), 3, "{payload}");
    for notice in &notices {
        let kind = notice["kind"].as_str().unwrap_or_default();
        assert!(
            ["struck", "struck_spent", "section_lost"].contains(&kind),
            "{notice}"
        );
        assert!(
            own.contains(&notice["section"]),
            "{kind}'s section is not one of the pilot's own {own:?}: {notice}"
        );
        for empty in ["contact", "designator", "was"] {
            assert_eq!(notice[empty], Value::Null, "{kind}.{empty}: {notice}");
        }
        no_sixteen_digit_word(&notice.to_string(), "a victim's own notice object");
    }
}

/// **Every `was` a raw cockpit frame carries is a name the pilot was sent**
/// (16b cut B.1, `T-86`): a designator — `X<n>` or a two-letter slot name —
/// that the pilot's own frames hold or held (`held`), and never sixteen digits
/// or an object number. `null` passes: most notices quote no old name.
fn every_was_is_a_held_designator(payload: &Value, held: &[&str], what: &str) {
    let notices = payload["notices"].as_array().cloned().unwrap_or_default();
    for notice in &notices {
        let was = &notice["was"];
        if was.is_null() {
            continue;
        }
        let name = was
            .as_str()
            .unwrap_or_else(|| panic!("{what}: `was` is a string: {notice}"));
        let provisional = name
            .strip_prefix('X')
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
        let slot = name.len() == 2 && name.chars().all(|c| c.is_ascii_uppercase());
        assert!(
            provisional || slot,
            "{what}: `was` is not a designator: {notice}"
        );
        assert!(
            held.contains(&name),
            "{what}: `was` {name} is a name the pilot was never sent ({held:?}): {notice}"
        );
        // The key itself, not the whole object: `contact` is the row key a
        // panel matches on and is sixteen digits by design (16b cut Q.1).
        no_sixteen_digit_word(name, what);
    }
}

/// **16b cut D.3's two new announcements name sides and people, and nothing
/// else** (doc 16 §1, §5).
///
/// A concede and an unattended end are said to the **whole centre** — the three
/// rooms and the twelve pods — so they reach a seated pilot in the middle of a
/// fight, which is exactly where a leak would matter. Both are built from three
/// things and no others: a character's display name, a side word, and the match
/// id. Every one of those is already public — the name and the side through the
/// whole of `Selection`, the id on every board — and neither sentence has a
/// field an object number, a cell, a bearing, a designator or a contact could be
/// carried in.
///
/// Over the real front door, from inside the pod, and read over **every byte** of
/// the losing side's session rather than at one moment.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_ends_of_a_match_name_sides_and_pilots_and_nothing_of_the_field() {
    let server = boot("observer-ends").await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;

    // Bo's own object numbers, so the sweep below can look for them by name —
    // taken before the match, since 16b cut U.4 (16c `T-67`) `@find` names
    // nothing an arena holds.
    let (red_tank, blue_tank, _muster, _domain) = seat_the_witnesses(&mut god).await;
    take_off(&mut ada, &mut bo).await;

    // The second witness for that rule: during `Running`, God's `@find` counts
    // the blue tank out and names it nowhere.
    let listing = ask(&mut god, &format!("@find {BLUE_TANK}"), "END OF LIST").await;
    assert!(
        listing.contains("(0 found)") && !listing.contains(&format!("#{blue_tank}")),
        "`@find` named a unit an arena holds:\n{listing}"
    );

    // From inside the pod: red gives the match up, and blue — the side that did
    // not — reads the announcement.
    let mark = bo.mark();
    say(&mut ada, "SIM CONCEDE", "concedes for the red side").await;
    bo.expect("Match 1 goes to blue when the arena answers.")
        .await;
    bo.expect("Its units are checked in.").await;
    let heard = strip_ansi(&bo.seen[mark..]);

    // What it DOES say: a person, a side, a match.
    assert!(heard.contains("Ada concedes for the red side."), "{heard}");
    // What it may not: any unit, any position, any contact.
    for leak in [
        RED_TANK, BLUE_TANK, "bearing", "cell", " m east", " m north",
    ] {
        assert!(
            !heard.contains(leak),
            "the concede announcement leaked {leak:?}:\n{heard}"
        );
    }
    assert!(
        !heard.contains(&format!("#{blue_tank}")),
        "the concede announcement named an object:\n{heard}"
    );
    no_sixteen_digit_word(&heard, "the concede announcement");

    // And the board that follows it says who won and why, in the same two public
    // words and no third.
    let board = strip_ansi(&ask(&mut bo, "SIM STATUS", "blue:").await);
    assert!(board.contains("blue wins: red conceded"), "{board}");
    for leak in [RED_TANK, BLUE_TANK] {
        assert!(
            !board.contains(leak),
            "the terminal board leaked {leak:?}:\n{board}"
        );
    }
    no_sixteen_digit_word(&board, "the terminal board");

    // **16b cut V.2 (16c N2)**: and the centre's history of it is the same public
    // outcome — who won and why, how long, how many a side, when, and the
    // reader's own callsign — and nothing of the field. The row's `#1` is the
    // match id; no `#` on it may be the tank's object number. The result was
    // written in the check-in action whose line Bo has just read, so the ring
    // already holds it.
    let history = ask_whole(&mut bo, "SIM HISTORY").await;
    // Flattened: a row with its suffix runs past the terminal's eighty columns
    // and the session wraps it; the two-space layout is pinned in `tacsim`.
    let flat = history.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("#1 blue won (red conceded) ") && flat.contains(" 1 v 1 "),
        "{history}"
    );
    assert!(
        flat.contains(" (you flew BA)"),
        "the reader's own callsign: {history}"
    );
    for leak in [
        RED_TANK, BLUE_TANK, "bearing", "cell", " m east", " m north", "contact",
    ] {
        assert!(
            !history.contains(leak),
            "`SIM HISTORY` leaked {leak:?}:\n{history}"
        );
    }
    // Neither side's tank by object number: not the reader's own and not the
    // other side's, which is the leak this rule exists for.
    for tank in [red_tank, blue_tank] {
        assert!(
            !history.contains(&format!("#{tank}")),
            "`SIM HISTORY` named an object:\n{history}"
        );
    }
    no_sixteen_digit_word(&history, "`SIM HISTORY`");
    // And no bare number either: the row is read word by word against its whole
    // public shape, so any word it does not account for — an object number
    // without its `#`, a pilot's name, a coordinate — fails it.
    history_row_is_public(&flat, 1, "#1 blue won (red conceded)", Some("BA"));

    // **16b cut V.3 (16c N2)**: and Bo's own debrief of it — the same public
    // outcome at its head, what Bo's own tank did (nothing), and the end in the
    // same words, with nothing of the field.
    let report = sim_report(&mut bo, "SIM REPORT").await;
    let flat = report.split_whitespace().collect::<Vec<_>>().join(" ");
    for want in [
        "blue won (red conceded)",
        "You flew BA: intact; 0 shots, 0 hits.",
        "You fired nothing.",
        "the match ended: red conceded.",
    ] {
        assert!(flat.contains(want), "{want:?} missing from:\n{report}");
    }
    report_is_the_pilots_own(&report, red_tank, blue_tank, "Bo's report of the concede");

    // The unattended end's sentence is the same shape with the person taken out
    // of it, so there is even less in it: it names the match and nothing else.
    // Its live proof is `crates/mushd/tests/sim_solo.rs`, which has the 10 s clock
    // to wait on and sweeps the bytes it really sends; what is pinned here is
    // that the text carries nothing of the field — read off
    // **`lobby::unattended_line`, the production sentence**, and not a copy of it
    // kept in this file, which is a copy that can stop matching what the centre
    // says without anything failing.
    let unattended = tacsim::match_::unattended_line(tacsim::arena_task::MatchId(1));
    assert!(unattended.contains("Match 1 ends:"), "{unattended}");
    for leak in [RED_TANK, BLUE_TANK, "bearing", "red", "blue", "#"] {
        assert!(
            !unattended.contains(leak),
            "the unattended announcement leaked {leak:?}: {unattended}"
        );
    }
    no_sixteen_digit_word(&unattended, "the unattended announcement");
}

/// **One frame carrying one contact row and nothing else** (16b cut D.4), so the
/// one renderer can be asked for a sentence without a booted arena.
///
/// `notice_line` reads the frame for a lock's percentage and for the friendly
/// mark and reads neither on a contact notice, so every field below is a value
/// the sentence under test cannot reach.
fn d4_stub_frame(lock: tacsim::arena::state::ContactId) -> tacsim::arena::step::Frame {
    let schema = mush_core::DefinitionRef::new(
        mush_core::DefinitionKey::new(
            mush_core::OwnerId::new("tacsim").expect("a canonical owner"),
            mush_core::DefinitionKind::new("schema").expect("a canonical kind"),
            mush_core::DefinitionId::new("tacsim.arena.observer").expect("a canonical id"),
        ),
        1,
        mush_core::DefinitionDigest::from_bytes([0_u8; 32]),
    )
    .expect("version 1 is not zero");
    let posture =
        mush_sim::RulesValue::new(schema, serde_json::json!({})).expect("an empty posture");
    tacsim::arena::step::Frame {
        unit: mush_core::ObjId(101),
        callsign: tacsim::arena::state::Designator::Slot {
            team: tacsim::arena::state::Team::Red,
            slot: 1,
        },
        cell: Some((6, 4)),
        team: tacsim::arena::state::Team::Red,
        pose: mush_sim::data::Pose::at(6_500, 4_500, 0, posture),
        speed_cms: 0,
        power: tacsim::arena::state::Power::Online,
        held: false,
        wreck: false,
        immobile: false,
        tempo: tacsim::arena::clock::Tempo::Quiet,
        sight_cm: 60_000,
        contacts: Vec::new(),
        scan_tables: Vec::new(),
        notices: Vec::new(),
        lock: Some(lock),
        weapons: Vec::new(),
        sections: Vec::new(),
        heat: None,
        receiver: None,
        // 16b cut O.2: no pod.
        ecm: None,
        // 16b cut Y.4: no reclaim here.
        catchup: None,
    }
}

/// **16b cut D.4's two new surfaces name the pilot's own machine and the names
/// that pilot has already been sent** (doc 16 §1, §3.6; 16c §5 Q16 and Q17).
///
/// Two different halves of the same rule.
///
/// **The receiver** (`B-22`) is a fact about the pilot's own platform, which doc
/// 16 §5 puts outside the observer rule entirely — as `sections`, `heat` and
/// `sight_m` already are. What the rule still forbids is the other half of the
/// moment: **who** is jamming, and **from where**. So the check is on the shape
/// as much as on the words — `FrameNotice::Receiver` has no field a contact, an
/// object number, a bearing, a cell or a side could be carried in, and the
/// sentence and the `STATUS` line carry none of them either.
///
/// It is deliberately **not** checked for "no count": the three integers are
/// `B-22`'s own fix (*the observer's own attenuation budget against its unjammed
/// value*) and they are there on purpose. `crate::arena::perception::Receiver`'s
/// doc comment declares what they let a pilot infer — the edge of a bubble is
/// walkable, and a deeper degradation is readable — and 16c §5 Q17 carries both
/// to the owner with the tightening named.
///
/// **The regression line** (`B-20`) is the other half: it names two designators
/// and nothing else, and both of them are names this pilot has already been sent
/// — the one their own row is reading now, and the one it read a moment ago,
/// which they have been typing. And the one it read a moment ago **stops
/// working** the instant the line is printed, because accepting it would say BA
/// is still out there.
///
/// The frame-JSON half of the receiver — that the key is `null` on a clean
/// platform and exactly three integers on a jammed one — is `tacsim`'s own
/// `d4_the_frame_carries_a_receiver_key_and_the_clean_scene_is_null`, which has
/// the one scene every renderer is held to.
#[test]
fn the_receiver_and_the_regression_name_only_what_that_pilot_already_holds() {
    use tacsim::arena::perception::{FrameNotice, Named, NoticeKind, Receiver};
    use tacsim::arena::state::{Classification, ContactId, Designator, Team};
    use tacsim::match_::cockpit;

    // (1) The two sentences and the `STATUS` line, whole.
    let receiver = Receiver {
        budget: 15,
        budget_max: 40,
        reach_permille: 600,
    };
    for line in [
        cockpit::RECEIVER_DEGRADED.to_owned(),
        cockpit::RECEIVER_CLEAR.to_owned(),
        cockpit::receiver_line(receiver),
    ] {
        no_sixteen_digit_word(&line, "the receiver's own line");
        for leak in [
            "bearing", "cell", "contact", "red", "blue", "team", "X1", "X2", "RA", "BA", "#",
            "pod", "jammer", "emitter",
        ] {
            assert!(
                !line.contains(leak),
                "the receiver's line leaked {leak:?}:\n{line}"
            );
        }
    }
    // The `STATUS` line's integers are the platform's own and are there on
    // purpose — see this test's head.
    assert_eq!(
        cockpit::receiver_line(receiver),
        "  receiver degraded: 15 of 40 through cover, 60% reach"
    );

    // (2) And the shape: the notice cannot carry a contact, so nothing a
    //     renderer does with it can name one.
    for notice in [
        FrameNotice::Receiver { degraded: true },
        FrameNotice::Receiver { degraded: false },
    ] {
        assert_eq!(notice.contact(), None, "{notice:?}");
        assert_eq!(notice.designator(), None, "{notice:?}");
        assert_eq!(notice.named(), None, "{notice:?}");
        assert_eq!(notice.section(), None, "{notice:?}");
        assert_eq!(notice.weapon(), None, "{notice:?}");
        assert_eq!(notice.order(), None, "{notice:?}");
        assert_eq!(notice.cause(), None, "{notice:?}");
        assert_eq!(notice.refusal(), None, "{notice:?}");
    }
    assert_eq!(
        FrameNotice::Receiver { degraded: true }.kind().as_str(),
        "receiver_degraded"
    );
    assert_eq!(
        FrameNotice::Receiver { degraded: false }.kind().as_str(),
        "receiver_clear"
    );

    // (3) The regression line, and the refusal that follows it. The row is
    //     `X2` now and was `BA` a moment ago; both are names this pilot has been
    //     sent, and nothing else is in the sentence.
    let id = ContactId(0x1111_1111_1111_1111);
    let row = tacsim::match_::cockpit::ContactRow {
        id,
        designator: Designator::Provisional(2),
        alias: None,
        class: smol_str::SmolStr::new("unknown"),
        bearing_deg: 45,
        relative_deg: 0,
        band: smol_str::SmolStr::new("far"),
        range_m: 450,
        age_ms: 0,
        cell: Some((36, 14)),
        friendly: false,
    };
    let regressed = FrameNotice::Contact {
        kind: NoticeKind::Unidentified,
        contact: Named {
            id,
            designator: Designator::Provisional(2),
        },
        class: Classification::Unknown,
        was: Some(Designator::Slot {
            team: Team::Blue,
            slot: 1,
        }),
    };
    assert_eq!(regressed.kind().as_str(), "unidentified");
    // The sentence itself is pinned in `tacsim`; what is pinned here is that it
    // carries nothing beyond the two names.
    let rows = [row.clone()];
    let line = cockpit::notice_line(&d4_stub_frame(id), &regressed, &rows);
    no_sixteen_digit_word(&line, "the regression line");
    for leak in ["bearing", "cell", "045", "450", "blue", "#"] {
        assert!(
            !line.contains(leak),
            "the regression line leaked {leak:?}:\n{line}"
        );
    }

    // **16b cut B.1** (`T-86`): the raw frame carries the old name as a field
    // now, so the same rule is asked of the JSON a panel decodes. The frame the
    // line arrives on holds the row as `X2`; the frame before it held it as `BA`
    // — the only two names this notice may quote, and both designators.
    let earlier = tacsim::arena::perception::FrameContact {
        id,
        designator: Designator::Slot {
            team: Team::Blue,
            slot: 1,
        },
        alias: Some(2),
        class: Classification::Classified { team: Team::Blue },
        bearing_mdeg: 45_000,
        range_cm: 45_000,
        age_ms: 0,
        cell: Some((36, 14)),
        condition: None,
    };
    let now = tacsim::arena::perception::FrameContact {
        designator: Designator::Provisional(2),
        alias: None,
        class: Classification::Unknown,
        ..earlier
    };
    let mut before = d4_stub_frame(id);
    before.contacts = vec![earlier];
    let mut after = d4_stub_frame(id);
    after.contacts = vec![now];
    after.notices = vec![regressed.clone()];
    let held: Vec<String> = [&before, &after]
        .iter()
        .flat_map(|frame| cockpit::frame_contacts(frame))
        .map(|row| row.designator.to_string())
        .collect();
    let held: Vec<&str> = held.iter().map(String::as_str).collect();
    assert_eq!(held, ["BA", "X2"]);
    let published = cockpit::status_frame(tacsim::arena_task::MatchId(1), &after);
    let notice = published["notices"][0].clone();
    assert_eq!(notice["kind"], json!("unidentified"), "{published}");
    assert_eq!(notice["designator"], json!("X2"), "{published}");
    assert_eq!(notice["was"], json!("BA"), "{published}");
    every_was_is_a_held_designator(&published, &held, "the regression's frame");
    // The sentence the web ring composes from those two fields says the same
    // two names and nothing else (`noticeLine`: `BA unidentified again: X2`).

    // **And the old name is not an alias.** `LOCK BA` after the line is the one
    // byte-identical refusal, and `LOCK X2` — the name the row reads now —
    // resolves.
    assert_eq!(cockpit::resolve_designator(&rows, "BA"), None);
    assert_eq!(cockpit::resolve_designator(&rows, "ba"), None);
    assert_eq!(cockpit::resolve_designator(&rows, "X2"), Some(id));
    // Which is the sentence the cockpit answers with, spelled out here the way
    // this file spells every sentence it asserts.
    assert_eq!(
        NO_SUCH_CONTACT,
        tacsim::arena::perception::contact_refusal()
    );
}

/// **16b cut E.3's traffic hold tells a pilot nothing about what is in the way**
/// (doc 16 §1, §3.6; 16c `B-46`).
///
/// The cut changed *when* `Held: something is in the way.` arrives, so the
/// observer rule has to be asked again. Two halves, and neither needs a live
/// match to answer:
///
/// * **during the hold there is no line at all.** A hold is by construction the
///   substep that reports no cause, and `crate::arena::step` stages the pilot's
///   blocked notice off `MotionOutcome::held_edge`, which only a stop with a
///   cause can set (16b cut M.3) — so a pilot held by a unit
///   they have not detected is sent nothing for up to
///   `[tacsim.arena].occupied_stop_ms`, which is later than HEAD told them and
///   never sooner. `tacsim`'s own
///   `another_unit_holds_a_tank_for_two_seconds_and_then_stops_it_with_one_line`
///   is that half driven end to end through `Arena::step`, with the frame during
///   the hold pinned as well.
/// * **after it, the one line names and places nothing.** The sentence is the
///   same sentence it always was, and the notice it is rendered from has no field
///   an object number, a name, a contact, a designator, a side or a cell could
///   travel in — so there is nothing for a later renderer to leak either.
#[test]
fn e3_the_traffic_stops_one_line_names_nothing_and_places_nothing() {
    use tacsim::arena::motion::BlockedCause;
    use tacsim::arena::perception::{FrameNotice, NoticeKind};
    use tacsim::arena::state::ContactId;
    use tacsim::match_::cockpit;

    let notice = FrameNotice::Blocked {
        cause: BlockedCause::Occupied,
    };
    let frame = d4_stub_frame(ContactId(0x0123_4567_89ab_cdef));
    let line = cockpit::notice_line(&frame, &notice, &[]);
    assert_eq!(line, "Held: something is in the way.");
    no_sixteen_digit_word(&line, "the traffic stop's line");
    for leak in [
        "bearing", "contact", "red", "blue", "X1", "RA", "BA", "#", "cell", "unit",
    ] {
        assert!(
            !line.contains(leak),
            "the traffic stop's line leaked {leak:?}:\n{line}"
        );
    }
    assert!(
        !line.chars().any(|c| c.is_ascii_digit()),
        "the traffic stop's line carried a number:\n{line}"
    );

    // And the shape: the notice can carry a cause and nothing else, so no
    // renderer of it can name or place the unit that was in the way.
    assert_eq!(notice.cause(), Some(BlockedCause::Occupied));
    assert_eq!(notice.contact(), None);
    assert_eq!(notice.designator(), None);
    assert_eq!(notice.named(), None);
    assert_eq!(notice.weapon(), None);
    assert_eq!(notice.section(), None);
    assert_eq!(notice.refusal(), None);
    assert_eq!(notice.order(), None);
    assert_eq!(notice.kind(), NoticeKind::Blocked);
}

/// **16b cut AE.1: the slope's hold names nothing and places nothing** (doc 16
/// §1, §3.6; 16c `T-18`).
///
/// The Bluff is the first ground on the shipped map a tank can be refused by
/// slope, so `Held: the slope ahead is too steep for the tank.` is a sentence a
/// pilot now reads in play and the observer rule asks it again: it names no
/// unit, carries no number and no cell — it cannot say how high the face is or
/// where — and the notice it is rendered from has no field any of those could
/// travel in. And it is the one sentence for every chassis: a car refused a face
/// reads the tank's word (a named residual, not a leak).
#[test]
fn ae1_the_slope_hold_names_nothing_and_places_nothing() {
    use tacsim::arena::motion::BlockedCause;
    use tacsim::arena::perception::{FrameNotice, NoticeKind};
    use tacsim::arena::state::ContactId;
    use tacsim::match_::cockpit;

    let notice = FrameNotice::Blocked {
        cause: BlockedCause::TooSteep,
    };
    let frame = d4_stub_frame(ContactId(0x0123_4567_89ab_cdef));
    let line = cockpit::notice_line(&frame, &notice, &[]);
    assert_eq!(line, "Held: the slope ahead is too steep for the tank.");
    no_sixteen_digit_word(&line, "the slope's line");
    for leak in [
        "bearing", "contact", "red", "blue", "X1", "RA", "BA", "#", "cell", "unit", "Bluff",
        "metre", " m ", "%",
    ] {
        assert!(
            !line.contains(leak),
            "the slope's line leaked {leak:?}:\n{line}"
        );
    }
    assert!(
        !line.chars().any(|c| c.is_ascii_digit()),
        "the slope's line carried a number:\n{line}"
    );
    assert_eq!(notice.cause(), Some(BlockedCause::TooSteep));
    assert_eq!(notice.contact(), None);
    assert_eq!(notice.designator(), None);
    assert_eq!(notice.named(), None);
    assert_eq!(notice.kind(), NoticeKind::Blocked);
}

// ---------------------------------------------------------------------------
// 16b cut Y.4: the catch-up report is projected through the observer rule
// ---------------------------------------------------------------------------

/// The range in metres a pilot's `CONTACTS` prints for the row named `who`, if
/// it holds one.
fn contact_range_m(listing: &str, who: &str) -> Option<u64> {
    let row = strip_ansi(listing)
        .lines()
        .find(|line| line.trim_start().starts_with(&format!("{who} ")))?
        .to_string();
    let words: Vec<&str> = row.split_whitespace().collect();
    let at = words.iter().position(|word| *word == "m")?;
    words.get(at.checked_sub(1)?)?.parse().ok()
}

/// **A reclaimed pilot's catch-up names nothing the platform never held** (16b
/// cut Y.4; doc 16 §1, §3.6; doc 07 §1.3 step 5).
///
/// Ada's `RA` and Bo's `BA` are re-posed seventy cells apart on the road —
/// outside the stock sight — with `BA` WEST of `RA`, between it and red's entry
/// zone. Ada takes off and her line drops before either platform holds
/// anything: the catch-up's snapshot is taken with nothing held. Past the
/// two-second grace the machine has `RA` and withdraws it west, toward `BA`,
/// and Bo drives east: the report must have a contact to say, and it must be
/// one the platform legitimately holds. (A re-posing while Ada is dropped is
/// not possible — a unit an arena holds is not spatial, which is this file's
/// own first assertion — so the gap is closed by the two tanks driving.)
///
/// Ada comes back and `BOARD`s: the block says `Back aboard RA` and `Contacts:
/// new BA; none lost.` — the platform's own designator for the tank it now
/// holds — and the block names no tank and no object number, while nothing
/// Ada was sent in the window names the other side's tank or its object number
/// or carries a sixteen-digit id (her own tank's name is hers: the reclaim's
/// own line says it). Her web client's pushed frame carries the
/// same report as an object whose `contacts_new` is exactly that frame's own
/// contact designators, whose `contacts_lost` is empty, and whose keys name no
/// unit. Bo, who never dropped, never reads `Back aboard`.
///
/// The report's two cells are `RA`'s own: `cell_then` is where it was placed,
/// exactly, and `cell_now` is where it stood when the `Live` re-seated Ada —
/// pulse phase 1, before that pulse's motion, which is what makes the report
/// one figure at every cadence (`tacsim/tests/arena.rs`'s Y.3 cross-tempo
/// test). The carrying frame is built after the motion, and after any pulse a
/// lagging world held (`merge_held`), and `RA` is still under way — the machine
/// was withdrawing it — so the frame's own cell may be further along; the two
/// are held to within what the tank can drive between the `BOARD` and the
/// frame's arrival, and never to one cell.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reclaimed_pilots_catch_up_names_nothing_the_platform_never_held() {
    let server = boot_grace("observer-catchup", true, Duration::from_secs(2)).await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (ada_handle, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (red, blue, _muster, domain) = seat_the_witnesses(&mut god).await;

    // `RA` where `close_the_gap` puts blue, `BA` where it puts red: seventy
    // cells apart on the road, `BA` on `RA`'s way home.
    for (unit, (x, y), heading) in [(red, BLUE_CELL, 270_000), (blue, RED_CELL, 90_000)] {
        let pose = format!(
            "{{\"x_cm\":{},\"y_cm\":{},\"heading_mdeg\":{heading}}}",
            x * 1_000 + 500,
            y * 1_000 + 500
        );
        say(
            &mut god,
            &format!("@spatial/place/domain #{unit}={domain},{pose}"),
            "Placed",
        )
        .await;
    }
    take_off(&mut ada, &mut bo).await;
    let seen = ask(&mut ada, "CONTACTS", NO_CONTACTS).await;
    assert!(seen.contains(NO_CONTACTS), "RA holds nothing yet: {seen}");

    // Ada's line drops; past the grace the machine has `RA`.
    drop(ada);
    drop(ada_handle);
    support::wait_arena_pilot_out(&mut god, "RA", "Ada", "flown by doctrine").await;

    // Bo drives east, the machine drives `RA` west: wait until Bo's own plot of
    // `RA` is well inside the stock sight, so `RA`'s (the same sight, both on
    // the road and moving) holds `BA` too.
    drive(&mut bo, 90).await;
    wait_until(
        Duration::from_secs(90),
        "the two tanks to close inside the stock sight",
        async || {
            let listing = ask_whole(&mut bo, "CONTACTS").await;
            contact_range_m(&listing, "RA").is_some_and(|range| range <= 500)
        },
    )
    .await;

    // Ada comes back: told, and then `BOARD` once the floor has passed.
    let (_a2, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    ada.expect("is flying on doctrine; BOARD takes it back.")
        .await;
    let window = ada.mark();
    let mut web = web_session(&server, "ada", PILOT_PASSWORD, "Ada").await;
    // The earliest `BOARD` that may have re-seated her: every attempt but one
    // refused as `still being handed over`, which sends the arena nothing.
    let mut boarded: Option<std::time::Instant> = None;
    wait_until(STEP_TIMEOUT, "BOARD to take the tank back", async || {
        let mark = ada.mark();
        let sent = std::time::Instant::now();
        ada.send_line("BOARD").await;
        let answer = ada
            .expect_one_of_after(
                mark,
                &[
                    "yours again where it stands",
                    "is still being handed over",
                    COMMIT_PENDING,
                    CONTROL_UNKNOWN,
                ],
            )
            .await;
        if answer != 1 {
            boarded.get_or_insert(sent);
        }
        answer == 0
    })
    .await;
    let boarded = boarded.expect("the BOARD that re-seated Ada");
    ada.expect("Back aboard RA").await;
    ada.expect("  Power ").await;
    let sent = strip_ansi(&ada.seen[window..]);
    // The block: its first line to its power line (and a wreck line after it).
    let block: String = sent
        .get(sent.find("Back aboard RA").unwrap_or(0)..)
        .unwrap_or_default()
        .lines()
        .scan(false, |past_power, line| {
            if *past_power && !line.contains("Your tank is a wreck.") {
                return None;
            }
            *past_power |= line.trim_start().starts_with("Power ");
            Some(format!("{line}\n"))
        })
        .collect();
    assert!(
        block.contains("Contacts: new BA; none lost."),
        "the report names the one contact the platform holds:\n{block}"
    );
    names_nothing(&block, red, blue, "the catch-up block");
    // Everything else Ada was sent in the window: her own tank is named — the
    // reclaim's own line says `Red Tank 1 is yours again` — and nothing of the
    // other side's is, nor any contact's sixteen digits.
    for leak in [BLUE_TANK.to_string(), format!("#{blue}")] {
        assert!(
            !sent.contains(&leak),
            "Ada's window leaked {leak:?}:\n{sent}"
        );
    }
    no_sixteen_digit_word(&sent, "Ada's reclaim and its catch-up");

    // The web half: the pushed frame's report is the frame's own scope.
    let mut report = Value::Null;
    let mut frame = Value::Null;
    for _ in 0..40 {
        frame = pushed_cockpit(&mut web).await;
        if frame.get("catchup").is_some_and(Value::is_object) {
            report = frame.get("catchup").cloned().unwrap_or_default();
            break;
        }
    }
    assert!(report.is_object(), "no pushed frame carried the report");
    let mut held: Vec<String> = frame
        .get("contacts")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.get("designator").and_then(Value::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let mut gained: Vec<String> = report
        .get("contacts_new")
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    held.sort();
    gained.sort();
    assert_eq!(
        gained, held,
        "the report's new contacts are the frame's own"
    );
    assert_eq!(gained, vec!["BA".to_string()]);
    assert_eq!(
        report.get("contacts_lost"),
        Some(&json!([])),
        "nothing was held to lose"
    );
    let mut keys: Vec<&str> = report
        .as_object()
        .map(|object| object.keys().map(String::as_str).collect())
        .unwrap_or_default();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "away_s",
            "cell_now",
            "cell_then",
            "contacts_lost",
            "contacts_more",
            "contacts_new",
            "machine",
            "power_now",
            "power_then",
            "rounds_more",
            "rounds_spent",
            "sections",
            "sections_more",
            "wreck",
        ],
        "the report's keys name no unit"
    );
    let flat = report.to_string();
    names_nothing(&flat, red, blue, "the pushed catch-up");
    no_sixteen_digit_word(&flat, "the pushed catch-up");
    for key in ["\"unit\"", "\"target\"", "\"pilot\"", "\"name\""] {
        assert!(
            !flat.contains(key),
            "the pushed catch-up carried {key}: {flat}"
        );
    }

    // No position of blue's: the report's two cells are `RA`'s own — where it
    // stood when the line dropped (it had not moved since it was placed) and
    // where it stood when the reclaim re-seated Ada — and every cell the block
    // prints is one of those two.
    let travelled = boarded.elapsed();
    let pair = |value: &Value, key: &str| -> Option<(u64, u64)> {
        let cell = value.get(key)?.as_array()?;
        Some((cell.first()?.as_u64()?, cell.get(1)?.as_u64()?))
    };
    let placed = (
        u64::try_from(BLUE_CELL.0).unwrap_or(0),
        u64::try_from(BLUE_CELL.1).unwrap_or(0),
    );
    assert_eq!(
        pair(&report, "cell_then"),
        Some(placed),
        "the cell RA stood in at the drop: {report}"
    );
    let framed = pair(&frame, "cell");
    assert!(framed.is_some(), "the carrying frame places RA: {frame}");
    let reseated = pair(&report, "cell_now");
    assert!(reseated.is_some(), "the report places RA now: {report}");
    // The report's cell is taken in the reclaiming pulse's phase 1 and the
    // frame's after its motion, so the two are one tank's track: no further
    // apart than `RA` can drive from the `BOARD` to the frame's arrival, plus
    // two pulses' capped integration (the one the `Live` landed in reaches back
    // before it, and its wall reading may precede the `BOARD` by a drain).
    let reach = reclaim_reach_cells(travelled);
    let apart = framed
        .zip(reseated)
        .map(|((fx, fy), (rx, ry))| fx.abs_diff(rx).max(fy.abs_diff(ry)));
    assert!(
        apart.is_some_and(|cells| cells <= reach),
        "RA's own cell now is {apart:?} cells from the carrying frame's, more than the {reach} \
         it could drive in {travelled:?}: {report}\n{frame}"
    );
    let printed = catchup_cells(&block);
    assert!(!printed.is_empty(), "the block prints RA's cells:\n{block}");
    for cell in &printed {
        assert!(
            Some(*cell) == reseated || *cell == placed,
            "the block printed a cell that is not the report's own, {cell:?}:\n{block}"
        );
    }

    // Bo never dropped, and is never told a report. `ask_whole` reads Bo's
    // channel up to a sentinel typed after Ada's block was sent, so anything
    // the report's frame sent Bo is in `seen` before the assertion reads it.
    let drained = ask_whole(&mut bo, "STATUS").await;
    assert!(!drained.is_empty(), "Bo's channel answered");
    let seen = strip_ansi(&bo.seen);
    assert!(
        !seen.contains("Back aboard"),
        "a pilot who never dropped read a catch-up:\n{seen}"
    );
}

/// **How many cells a tank can move between a reclaim and its report's frame**
/// (16b cut Y.4), `travelled` being the wall time from the `BOARD` to the
/// frame's arrival.
///
/// Arena time never runs ahead of the wall (`tacsim::arena::clock::Clock::advance`
/// clamps a pulse's integration to `max_catch_up_steps × period`), so the tank's travel
/// is at most its content's top speed over `travelled` plus two capped pulses:
/// the reclaiming pulse's own integration reaches back before the `Live`, and
/// its wall reading may precede the `BOARD` by the drain that followed it. A
/// move of `d` centimetres crosses at most `d / cell + 1` cell boundaries on
/// either axis.
fn reclaim_reach_cells(travelled: Duration) -> u64 {
    /// The greatest `max_speed_cms` anywhere in a definition.
    fn top(value: &Value) -> u64 {
        match value {
            Value::Object(map) => map
                .iter()
                .map(|(key, value)| {
                    if key == "max_speed_cms" {
                        value.as_u64().unwrap_or(0)
                    } else {
                        top(value)
                    }
                })
                .max()
                .unwrap_or(0),
            Value::Array(items) => items.iter().map(top).max().unwrap_or(0),
            _ => 0,
        }
    }
    // Both witnesses are line tanks: the stock one and `observer_packs`' copy.
    let tank: Value = serde_json::from_str(tacsim::definitions::content::LINE_TANK_JSON)
        .expect("the line tank's definition");
    let speed_cms = top(&tank);
    assert!(speed_cms > 0, "the line tank declares a top speed");
    let arena = tacsim_config(true).arena;
    let longest_ms = arena
        .tempo
        .quiet_ms
        .max(arena.tempo.alert_ms)
        .max(arena.tempo.engaged_ms);
    let pulse_cap_ms = u64::from(longest_ms) * u64::from(arena.max_catch_up_steps.max(1));
    let wall_ms = u64::try_from(travelled.as_millis()).unwrap_or(u64::MAX);
    let arena_ms = wall_ms.saturating_add(pulse_cap_ms.saturating_mul(2));
    // The window's cell, as this file places units: `x * 1_000 + 500`.
    let travel_cm = speed_cms.saturating_mul(arena_ms) / 1_000;
    travel_cm / 1_000 + 1
}

/// Every `(x, y)` cell a catch-up block prints, in order.
fn catchup_cells(block: &str) -> Vec<(u64, u64)> {
    block
        .split('(')
        .skip(1)
        .filter_map(|rest| {
            let (inside, _) = rest.split_once(')')?;
            let (x, y) = inside.split_once(", ")?;
            Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 16b cut Z.4 (16c N8): the side nets, as one more column
// ---------------------------------------------------------------------------

/// **The side's net carries a pilot's words and nothing else, and none of it to
/// the other side** (16b cut Z.4; doc 16 §1, §5).
///
/// The net is a surface the observer rule has to be asked about like every
/// other: while Ada speaks on red's net, everything Bo — the other side's pilot
/// — receives in the window passes [`names_nothing`] and carries neither the
/// net's name nor its words; and the line Ada reads back is comsys's own and
/// holds her callsign, her name and her words, and nothing a sensor would have
/// had to find: no tank name, no object number.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn z4_a_net_line_names_the_callsign_the_speaker_and_the_words_and_reaches_no_one_else() {
    let server = boot("observer-net").await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;

    let (red, blue, _muster, _domain) = seat_the_witnesses(&mut god).await;
    take_off(&mut ada, &mut bo).await;
    ada.expect("You have been joined to channel RedNet (alias: ra).")
        .await;
    bo.expect("You have been joined to channel BlueNet (alias: ba).")
        .await;

    let bo_mark = bo.mark();
    let ada_mark = ada.mark();
    say(&mut ada, "ra contact east", "[RedNet] RA Ada: contact east").await;
    // Blue's window closes on its own sentinel's refusal.
    let seen = ask_whole(&mut bo, SENTINEL).await;
    let window = strip_ansi(&bo.seen[bo_mark..]);
    assert!(
        window.contains(&seen),
        "the window is the whole of what Bo read"
    );
    names_nothing(&window, red, blue, "blue while red spoke on its net");
    for leak in ["RedNet", "contact east", "RA Ada"] {
        assert!(
            !window.contains(leak),
            "blue read {leak:?} while red spoke on its net:\n{window}"
        );
    }

    // Red's own line: the net, the callsign, the name and the words — and not
    // one thing more.
    let ada_seen = strip_ansi(&ada.seen[ada_mark..]);
    let line = ada_seen
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("[RedNet]"))
        .expect("Ada reads her own line back");
    assert_eq!(line, "[RedNet] RA Ada: contact east");
    names_nothing(&ada_seen, red, blue, "red's own net line");
}

// ---------------------------------------------------------------------------
// 16b cut I.1 — the replay: public, both sides, and nobody by name (16c N9, Q37)
// ---------------------------------------------------------------------------

/// The replay feed, spelled out for the reason [`MAP_FEED`] is.
const REPLAY_FEED: &str = "arena.replay";

/// **The replay column** (16b cut I.1; 16c N9, Q37).
///
/// A running match has no replay, and its one refusal must say nothing else:
/// while match 1 RUNS, `SIM REPLAY 1` from both its pilots, the bystander and the
/// staffer answers `Match 1 is still running.` and nothing that names a tank,
/// a callsign or a contact id. The witnesses are walked into the Simulator Lobby
/// first — `seat_the_witnesses` leaves Cy and God in the muster room, where
/// every spelling of `SIM` answers only where the simulator is.
///
/// After the match, the replay is **public and objective**: Cy, who flew
/// nothing, reads both callsigns and `both sides` — and no object mark, no
/// pilot's name and no sixteen-digit word, because the replay names every tank
/// by the callsign the arena gave it and reads nothing any sensor saw. Over
/// Cy's web socket the `arena.replay` query answers the whole document to the
/// same standard; over Ada's — seated in match 2 and having typed no `SIM
/// REPLAY` — it answers `available: false`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn i1_a_replay_is_public_names_nobody_and_a_running_match_has_none() {
    let server = boot("observer-replay").await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, mut cy) = login(server.addr, "cy", PILOT_PASSWORD, "Cy").await;
    let (red, blue, _muster, _domain) = seat_the_witnesses(&mut god).await;
    let listing = ask(&mut god, "@find Simulator Lobby", "END OF LIST").await;
    let lobby = dbref_of(&listing, "Simulator Lobby");
    say(
        &mut god,
        &format!("@teleport #{}=#{lobby}", CY.0),
        "Teleported",
    )
    .await;
    say(
        &mut god,
        &format!("@teleport #{}=#{lobby}", GOD.0),
        "Simulator Lobby",
    )
    .await;

    take_off(&mut ada, &mut bo).await;

    // (1) While match 1 runs: the one refusal, from every seat, and nothing else.
    for (who, term) in [
        ("Ada", &mut ada),
        ("Bo", &mut bo),
        ("Cy", &mut cy),
        ("God", &mut god),
    ] {
        // The answer from the line's own echo — its FIRST `SIM REPLAY 1`, so
        // a leaked reply's own `Next: SIM REPLAY 1 next` can never become the
        // cut point and hide what came before it — to the sentinel's `Huh?`: a
        // witness that has read nothing yet is handed its whole transcript,
        // whose earlier lines (`once both sides have a pilot`) are not the
        // answer.
        let whole = ask_whole(term, "SIM REPLAY 1").await;
        let answer = whole
            .split_once("SIM REPLAY 1")
            .map_or_else(|| whole.clone(), |(_, answer)| answer.to_owned());
        let seen = answer
            .rsplit_once(HUH)
            .map_or_else(|| answer.clone(), |(before, _)| before.to_owned());
        assert!(
            seen.contains("Match 1 is still running."),
            "{who} was not told the match is running:\n{seen}"
        );
        for leak in [
            "Replay of",
            "keyframe",
            "both sides",
            "Off the window",
            "Shots since",
            "Wrecked since",
            "Next:",
        ] {
            assert!(!seen.contains(leak), "{who} was told {leak:?}:\n{seen}");
        }
        for callsign in ["RA", "BA"] {
            assert!(
                !words_of(&seen).contains(&callsign),
                "{who}'s refusal named {callsign}:\n{seen}"
            );
        }
        names_nothing(&seen, red, blue, &format!("{who}'s refused `SIM REPLAY`"));
        no_sixteen_digit_word(&seen, &format!("{who}'s refused `SIM REPLAY`"));
    }

    // Let the arena clock move, so the tape holds keyframes after the opening.
    wait_until(
        STEP_TIMEOUT,
        "the arena clock to pass a second",
        async || {
            let staff = ask(&mut god, "@arena/status", "ledger writes awaiting").await;
            support::arena_clock_seconds(&staff).is_some_and(|seconds| seconds >= 1)
        },
    )
    .await;
    say(&mut ada, "SIM ABORT", "aborts match").await;
    cy.expect("Its units are checked in.").await;

    // (2) After it: public, both sides, nobody by name.
    let seen = ask(&mut cy, "SIM REPLAY 1", "Next: SIM REPLAY 1 next").await;
    let flat = seen.split_whitespace().collect::<Vec<&str>>().join(" ");
    assert!(
        flat.contains("both sides as the arena recorded them"),
        "{flat}"
    );
    for callsign in ["RA", "BA"] {
        assert!(
            words_of(&seen).contains(&callsign),
            "Cy's replay does not name {callsign}:\n{seen}"
        );
    }
    // No object mark: the printed grid may draw a wall as `#`, so the mark is
    // `#` before a digit.
    assert!(
        !seen.as_bytes().windows(2).any(|pair| {
            pair.first() == Some(&b'#') && pair.get(1).is_some_and(u8::is_ascii_digit)
        }),
        "Cy's replay carried an object mark:\n{seen}"
    );
    for name in ["Ada", "Bo"] {
        assert!(
            !words_of(&seen).contains(&name),
            "Cy's replay named {name}:\n{seen}"
        );
    }
    names_nothing(&seen, red, blue, "Cy's `SIM REPLAY 1`");
    no_sixteen_digit_word(&seen, "Cy's `SIM REPLAY 1`");

    // Over Cy's web socket: the whole document, to the same standard.
    let mut cy_web = web_session(&server, "cy", PILOT_PASSWORD, "Cy").await;
    let document = panel_query(&mut cy_web, 701, REPLAY_FEED).await;
    assert_eq!(
        document.pointer("/schema/shape").and_then(Value::as_str),
        Some("replay"),
        "{document}"
    );
    assert_eq!(document.get("match").and_then(Value::as_u64), Some(1));
    let whole = document.to_string();
    for callsign in ["\"RA\"", "\"BA\""] {
        assert!(
            whole.contains(callsign),
            "the document lacks {callsign}: {whole}"
        );
    }
    for leak in ["#", "Ada", "\"Bo\"", "seed", "draw"] {
        assert!(
            !whole.contains(leak),
            "the document carried {leak:?}: {whole}"
        );
    }
    names_nothing(&whole, red, blue, "the `arena.replay` document");
    no_sixteen_digit_word(&whole, "the `arena.replay` document");

    // (3) Ada, seated in match 2 and having typed no `SIM REPLAY`: nothing.
    for pilot in [ADA, BO] {
        say(
            &mut god,
            &format!("@teleport #{}=#{lobby}", pilot.0),
            "Teleported",
        )
        .await;
    }
    take_off(&mut ada, &mut bo).await;
    let mut ada_web = web_session(&server, "ada", PILOT_PASSWORD, "Ada").await;
    let answer = panel_query(&mut ada_web, 702, REPLAY_FEED).await;
    assert_eq!(answer.get("available"), Some(&json!(false)), "{answer}");
    assert_eq!(
        answer.get("reason").and_then(Value::as_str),
        Some("You have no replay open; type SIM REPLAY <match>."),
        "{answer}"
    );
    say(&mut ada, "SIM ABORT", "aborts match").await;
}

// ---------------------------------------------------------------------------
// 16b cut I.2 — the replay on screen 0 (16c N9, Q37)
// ---------------------------------------------------------------------------

/// **A whole TUI frame, squeezed** (16b cut I.2). The TUI paints only the
/// cells that change, so a needle read off an ordinary paint can lose a letter
/// that happened to be on screen at that cell already; a resize clears and
/// repaints every cell. This resizes the PTY to `cols` by 34 — callers
/// alternate the width, at or over the 120-column floor, so every call IS a
/// resize — and answers the frame, squeezed, once its last row (the legend:
/// `? help` in nav mode, `Esc nav` in the box) has arrived.
async fn tui_frame(term: &mut Term, cols: u32) -> String {
    let mark = term.mark();
    term.resize(cols, 34).await;
    term.expect_one_of_after(mark, &["help", "Escnav"]).await;
    squeezed(&term.seen[mark..])
}

/// **The replay column on the full-screen surface and the web** (16b cut I.2;
/// 16c N9, Q37).
///
/// Cy, who flew nothing, walked into the Simulator Lobby as I.1's column walks
/// him, in `@tui` at 120 by 34: after match 1 ends he types `SIM REPLAY 1` and
/// screen 0 opens by itself with the replay on it — `RA` and `BA` by callsign,
/// `Both sides shown`, the picture's own legend. `tui_names_nothing` is NOT the
/// lens here: the replay names both tanks by callsign on purpose; what it must
/// never carry is an object mark (the room's own dbref on the status line is
/// the one `#` and a digit on the frame) or a pilot's name. Over his web socket
/// the parameter-less `arena.map` answers the ground of match 1 — the cursor's
/// clause.
///
/// Then `x` closes the picture, and during match 2 his `SIM REPLAY 2` is
/// refused: screen 0 does not open, the refusal lands on the scene screen, and
/// — because a refusal drops the cursor — the web socket's `arena.replay`
/// answers `available: false` with the no-cursor sentence (never match 1's
/// document by accident) and `arena.map` the unseated pair.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn i2_the_replay_screen_shows_both_sides_and_a_refusal_opens_nothing() {
    let server = boot("observer-replay-screen").await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, mut cy) = login_sized(server.addr, "cy", PILOT_PASSWORD, "Cy", 120, 34).await;
    let (red, blue, _muster, _domain) = seat_the_witnesses(&mut god).await;
    let listing = ask(&mut god, "@find Simulator Lobby", "END OF LIST").await;
    let lobby = dbref_of(&listing, "Simulator Lobby");
    say(
        &mut god,
        &format!("@teleport #{}=#{lobby}", CY.0),
        "Teleported",
    )
    .await;

    // Match 1, fought for a second of arena time and aborted.
    take_off(&mut ada, &mut bo).await;
    wait_until(
        STEP_TIMEOUT,
        "the arena clock to pass a second",
        async || {
            let staff = ask(&mut god, "@arena/status", "ledger writes awaiting").await;
            support::arena_clock_seconds(&staff).is_some_and(|seconds| seconds >= 1)
        },
    )
    .await;
    say(&mut ada, "SIM ABORT", "aborts match").await;
    cy.expect("Its units are checked in.").await;

    // Into the TUI, the box opened once (see `tui_ask`), and the replay asked.
    let opened = cy.mark();
    cy.send_line("@tui").await;
    cy.expect_after(opened, "Cy").await;
    cy.send_line("").await;
    cy.send_line("SIM REPLAY 1").await;
    // Whole frames until screen 0 has opened and the document and the ground
    // have both landed on it.
    let mut cols = 120;
    let mut frame = String::new();
    wait_until(
        STEP_TIMEOUT,
        "the replay's document and ground to be drawn",
        async || {
            cols = if cols == 121 { 122 } else { 121 };
            frame = tui_frame(&mut cy, cols).await;
            frame.contains("Bothsidesshown") && frame.contains("Replay1\u{2014}")
        },
    )
    .await;
    assert!(
        frame.contains("Bothsidesshown:matchover."),
        "the status pane says both sides:\n{frame}"
    );
    for row in ["RAred", "BAblue"] {
        assert!(
            frame.contains(row),
            "the contacts pane names {row:?} by callsign:\n{frame}"
        );
    }
    // The box is open (the line was submitted from it): back to nav, and the
    // legend is the picture's own five, whole. Read in a retry like every other
    // frame here: the Enter's own diff paint can race the window change's whole
    // one, and a diff carries the legend without the tab strip.
    let legend = ",prev\u{b7}.next\u{b7}ufocus\u{b7}xclose\u{b7}ppull\u{b7}Entertype\u{b7}?help";
    cy.send_line("").await;
    wait_until(
        STEP_TIMEOUT,
        "a whole frame with the picture's legend",
        async || {
            cols = if cols == 121 { 122 } else { 121 };
            frame = tui_frame(&mut cy, cols).await;
            frame.contains(legend) && frame.contains("0cockpit")
        },
    )
    .await;
    assert!(frame.contains(legend), "the picture's legend:\n{frame}");
    assert!(
        frame.contains("0cockpit"),
        "screen 0's tab stands:\n{frame}"
    );
    // No object mark but the room's own dbref on the status line, and no pilot.
    let marks = frame.replace(&format!("#{lobby}"), "");
    assert!(
        !marks.as_bytes().windows(2).any(|pair| {
            pair.first() == Some(&b'#') && pair.get(1).is_some_and(u8::is_ascii_digit)
        }),
        "the replay screen carried an object mark:\n{frame}"
    );
    // The frame is read squeezed, so `Bo` is looked for with the status pane's
    // `Both` taken out. (The raw transcript is no lens here: it holds screen
    // 1's paints too, whose WHO rail lists everybody connected.)
    let named = frame.replace("Both", "");
    for pilot in ["Ada", "Bo"] {
        assert!(
            !named.contains(pilot),
            "the replay screen named {pilot}:\n{frame}"
        );
    }
    for number in [red, blue] {
        assert!(
            !frame.contains(&format!("#{number}")),
            "the replay screen carried a tank's object number:\n{frame}"
        );
    }

    // Over Cy's web socket: the parameter-less ground ask answers match 1's.
    let mut cy_web = web_session(&server, "cy", PILOT_PASSWORD, "Cy").await;
    let map = panel_query(&mut cy_web, 801, MAP_FEED).await;
    assert_eq!(map.get("match").and_then(Value::as_u64), Some(1), "{map}");
    assert_ne!(map.get("seated"), Some(&json!(false)), "{map}");

    // `x` closes the picture (the Return after it opens the box on the scene).
    cy.send_line("x").await;

    // Match 2: a running match has no replay, and the refusal opens nothing.
    for pilot in [ADA, BO] {
        say(
            &mut god,
            &format!("@teleport #{}=#{lobby}", pilot.0),
            "Teleported",
        )
        .await;
    }
    take_off(&mut ada, &mut bo).await;
    cy.send_line("SIM REPLAY 2").await;
    wait_until(
        STEP_TIMEOUT,
        "the refusal to land on the scene",
        async || {
            cols = if cols == 121 { 122 } else { 121 };
            tui_frame(&mut cy, cols)
                .await
                .contains("Match2isstillrunning.")
        },
    )
    .await;
    // Which screen: the help overlay's first line names it whole, read off a
    // whole frame.
    cy.send_line("").await; // back to nav
    cy.send_line("?").await; // help; the Return is swallowed while it is up
    let mut help = String::new();
    wait_until(STEP_TIMEOUT, "the help overlay to be drawn", async || {
        cols = if cols == 121 { 122 } else { 121 };
        help = tui_frame(&mut cy, cols).await;
        help.contains("scenescreen") || help.contains("cockpitscreen")
    })
    .await;
    assert!(
        help.contains("scenescreen") && !help.contains("cockpitscreen"),
        "the refused `SIM REPLAY 2` opened screen 0:\n{help}"
    );
    cy.send_line("?").await;

    // And the refusal dropped the cursor: no document of match 1 by accident,
    // and no ground.
    let answer = panel_query(&mut cy_web, 802, REPLAY_FEED).await;
    assert_eq!(answer.get("available"), Some(&json!(false)), "{answer}");
    assert_eq!(
        answer.get("reason").and_then(Value::as_str),
        Some("You have no replay open; type SIM REPLAY <match>."),
        "{answer}"
    );
    let map = panel_query(&mut cy_web, 803, MAP_FEED).await;
    assert_eq!(map, unseated(MAP_FEED), "{map}");

    say(&mut ada, "SIM ABORT", "aborts match").await;
}

// ---------------------------------------------------------------------------
// 16b cut I.3 — a staff place is told to nobody (16c N10)
// ---------------------------------------------------------------------------

/// **`@arena/place`, over the first cell of `cells` the world takes** (16b cut
/// I.3): the world refuses a cell off the window or with no way out in its own
/// sentence, and the arena's refusals are on the tape, so the first answer
/// `Placing` is the cell. Answers it.
async fn i3_place(god: &mut Term, unit: u32, cells: &[(u16, u16)]) -> (u16, u16) {
    for (x, y) in cells {
        for _ in 0..RETRY_BUDGET {
            let mark = god.mark();
            god.send_line(&format!("@arena/place #{unit}={x},{y}"))
                .await;
            match god
                .expect_one_of_after(
                    mark,
                    &[
                        "Placing",
                        "impassable, or has no way out",
                        "off the arena window",
                        COMMIT_PENDING,
                    ],
                )
                .await
            {
                0 => return (*x, *y),
                3 => tokio::time::sleep(Duration::from_millis(200)).await,
                _ => break,
            }
        }
    }
    panic!("the world took none of {cells:?}");
}

/// The running match's id, off `@arena/status`'s `match N:` row.
async fn i3_match_id(god: &mut Term) -> u64 {
    let status = ask(god, "@arena/status", "ledger writes awaiting").await;
    status
        .lines()
        .find_map(|line| line.strip_prefix("match ")?.split(':').next()?.parse().ok())
        .unwrap_or_else(|| panic!("no match row in:\n{status}"))
}

/// What a place must never be to a pilot.
fn i3_says_no_place(seen: &str, red: u32, blue: u32, what: &str) {
    names_nothing(seen, red, blue, what);
    for leak in ["place", "Placing", "in_flight"] {
        assert!(!seen.contains(leak), "{what} carried {leak:?}:\n{seen}");
    }
}

/// **16b cut I.3: a staff place is journalled always and announced never** (16c
/// N10; doc 16 §1, §5), over the real composition and real SSH.
///
/// God moves Bo's tank while the match runs. Everything Ada and Bo are sent
/// from before the place to the end of the test names neither tank, and says
/// neither `place`, `Placing` nor `in_flight`. Ada held Bo as a classified
/// contact before the place, and every `CONTACTS` after it answers that same
/// designator or none (the loss path, if the new cell is out of reach): a place
/// mints no new name — ids and designators are drawn once per viewer and target
/// for the match.
///
/// Then the refusal. Under a 5,000 ms floor pilots' orders wait for the next
/// wake, so Ada's `FIRE` at Bo and God's place of Bo's tank, sent back to back,
/// land in one pulse, and phase 4 resolves Ada's shot first (her tank's id is
/// the lower): the round is in flight when the place resolves, and the tape
/// says `place_refused` with `in_flight` — while Bo's stream carries no
/// `in_flight`, no `refused` and no `place`: a refusal naming a round in flight
/// would tell the target of an impact its own sensors never saw.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn i3_a_place_is_told_to_nobody_and_its_refusal_leaks_no_round() {
    let server = boot("observer-place").await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;

    let (red, blue, _muster, domain) = seat_the_witnesses(&mut god).await;
    assert!(
        red < blue,
        "phase 4 drains in unit-id order; this test needs the shooter's id below the target's \
         ({red} against {blue})"
    );
    close_to_muzzle_range(&mut god, red, blue, domain).await;
    take_off(&mut ada, &mut bo).await;
    power_up(&mut ada).await;
    let id = i3_match_id(&mut god).await;

    // Ada holds Bo, told apart.
    let mut contact = String::new();
    wait_until(STEP_TIMEOUT, "Ada to tell Bo's tank apart", async || {
        contact = first_contact(&mut ada).await;
        !contact.starts_with('X')
    })
    .await;
    let ada_mark = ada.mark();
    let bo_mark = bo.mark();

    // ---- the place ------------------------------------------------------
    let placed = i3_place(&mut god, blue, &[(96, 97), (96, 98), (96, 96), (97, 97)]).await;
    // A few sensor passes: the contact is the same designator, or gone.
    for _ in 0..4 {
        tokio::time::sleep(Duration::from_millis(1_000)).await;
        let mark = ada.mark();
        ada.send_line("CONTACTS").await;
        if ada
            .expect_one_of_after(
                mark,
                &["(rel ", NO_CONTACTS, COMMIT_PENDING, CONTROL_UNKNOWN],
            )
            .await
            == 0
        {
            let listing = strip_ansi(&ada.seen[mark..]);
            assert_eq!(
                designator_of(&listing).as_deref(),
                Some(contact.as_str()),
                "the place renamed the contact:\n{listing}"
            );
        }
    }

    // ---- a place under a round in flight ---------------------------------
    assert!(
        ask(
            &mut god,
            &format!("@arena/tempo {id}=5000"),
            "under a floor"
        )
        .await
        .contains("under a floor of 5000 ms")
    );
    say(&mut ada, &format!("LOCK {contact}"), "Locking").await;
    wait_until(
        STEP_TIMEOUT,
        "the cockpit to publish the lock",
        async || {
            ask_whole(&mut ada, "WEAPONSTATUS")
                .await
                .contains("Locked on")
        },
    )
    .await;
    // Back to back: both wait in the task's buffer for the same wake.
    let fire_mark = ada.mark();
    let god_mark = god.mark();
    ada.send_line("FIRE").await;
    // Back onto the cell it stood on before the first place: passable, with a
    // way out, and empty now.
    god.send_line(&format!(
        "@arena/place #{blue}={}, {}",
        BLUE_MUZZLE.0, BLUE_MUZZLE.1
    ))
    .await;
    assert_eq!(
        ada.expect_one_of_after(fire_mark, &["Firing cannon", "refused", COMMIT_PENDING])
            .await,
        0,
        "the shot was not taken: {}",
        strip_ansi(&ada.seen[fire_mark..])
    );
    assert_eq!(
        god.expect_one_of_after(
            god_mark,
            &["Placing", "impassable", "off the arena", COMMIT_PENDING]
        )
        .await,
        0,
        "the second place was not sent: {}",
        strip_ansi(&god.seen[god_mark..])
    );
    // The round leaves the barrel at the next wake, a floor away at most.
    wait_until(STEP_TIMEOUT, "the round to leave the barrel", async || {
        ask_whole(&mut ada, "WEAPONSTATUS")
            .await
            .contains("39 of 40 round(s)")
    })
    .await;
    say(
        &mut god,
        &format!("@arena/tempo {id}=off"),
        "its own tempo again",
    )
    .await;
    // A second for the lifted match to deliver whatever the impact tells Bo.
    tokio::time::sleep(Duration::from_millis(1_500)).await;

    // ---- what the two pilots were sent ------------------------------------
    let ada_seen = strip_ansi(&ada.seen[ada_mark..]);
    i3_says_no_place(&ada_seen, red, blue, "Ada's transcript after the place");
    let bo_seen = strip_ansi(&bo.seen[bo_mark..]);
    i3_says_no_place(&bo_seen, red, blue, "Bo's transcript after the place");
    assert!(
        !bo_seen.contains("refused"),
        "the target was told of a refusal:\n{bo_seen}"
    );
    no_sixteen_digit_word(&ada_seen, "Ada's transcript after the place");
    no_sixteen_digit_word(&bo_seen, "Bo's transcript after the place");

    // ---- the tape --------------------------------------------------------
    say(&mut god, &format!("@arena/abort {id}"), "is stopping").await;
    let mut events = Vec::new();
    wait_until(STEP_TIMEOUT, "both places to reach the tape", async || {
        events = ledger_events(&server.store, id).await;
        events
            .iter()
            .any(|event| event.get("kind").and_then(Value::as_str) == Some("place_refused"))
    })
    .await;
    let places: Vec<&Value> = events
        .iter()
        .filter(|event| {
            matches!(
                event.get("kind").and_then(Value::as_str),
                Some("placed" | "place_refused")
            )
        })
        .collect();
    assert_eq!(places.len(), 2, "{places:?}");
    assert_eq!(
        places[0].get("kind").and_then(Value::as_str),
        Some("placed")
    );
    assert_eq!(
        places[0].get("unit").and_then(Value::as_u64),
        Some(u64::from(blue))
    );
    assert_eq!(
        (
            places[0].get("x").and_then(Value::as_u64),
            places[0].get("y").and_then(Value::as_u64)
        ),
        (Some(u64::from(placed.0)), Some(u64::from(placed.1)))
    );
    assert_eq!(
        places[1].get("kind").and_then(Value::as_str),
        Some("place_refused")
    );
    assert_eq!(
        places[1].get("reason").and_then(Value::as_str),
        Some("in_flight"),
        "{:?}",
        places[1]
    );
    let floors = events
        .iter()
        .filter(|event| event.get("kind").and_then(Value::as_str) == Some("tempo_floor"))
        .count();
    assert_eq!(floors, 2, "the set and the lift are on the tape");
}

// ---------------------------------------------------------------------------
// 16b cut I.4 — `@arena/watch`: a staff surface, and nobody else's (16c N10)
// ---------------------------------------------------------------------------

/// The staff picture's feed, spelled out for the reason [`MAP_FEED`] is.
const OBSERVER_FEED: &str = "arena.observer";

/// The next envelope on this socket, the app-level keepalive answered on the
/// way (a watch may run longer than `ping_interval_ms` with the test sending
/// nothing, and an unanswered ping closes the socket).
async fn web_next(ws: &mut Ws) -> Envelope {
    loop {
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
        return env;
    }
}

/// Every envelope up to and including the first whose text carries `needle`.
async fn web_until(ws: &mut Ws, needle: &str) -> Vec<Envelope> {
    let mut seen = Vec::new();
    for _ in 0..4_000 {
        let env = web_next(ws).await;
        let done = text_of(&env.event).contains(needle);
        seen.push(env);
        if done {
            return seen;
        }
    }
    panic!("no envelope carried {needle:?}")
}

/// The `arena.observer` payload an event carries, bare or inside a reply.
fn observer_payload(event: &OutputEvent) -> Option<Value> {
    match event {
        OutputEvent::Panel { feed, data, .. } if feed.as_str() == OBSERVER_FEED => {
            Some(data.clone())
        }
        OutputEvent::Reply { event, .. } => observer_payload(event),
        _ => None,
    }
}

/// **16b cut I.4: the staff picture reaches the staffer who asked for it and
/// nobody else, on any surface** (16c N10; doc 16 §1, §5), over the real
/// composition, real SSH and real web sockets.
///
/// God watches match 1 from his web session: its `arena.observer` keyframes
/// name both tanks and both object numbers — a staff surface, asserted so. For
/// ten arena seconds of them, Ada's web socket (a pilot of the match), fenced
/// by a sentinel line sent after them, receives NO `arena.observer` envelope;
/// Cy's `arena.observer` query (a bystander) answers `watching: false`; God's
/// own SSH line client, watching in parallel (the same character), prints no
/// line naming a tank, a callsign or a row of the feed — the payload has no
/// shape; and Ada's `@arena/watch 1` is `Permission denied.`. After `@arena/watch
/// off` God's web socket is sent `watching: false` and — fenced by a second
/// sentinel two arena seconds later — nothing more.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn i4_a_watch_reaches_the_staffer_who_asked_and_no_one_else() {
    let server = boot("observer-watch").await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (red, blue, _muster, _domain) = seat_the_witnesses(&mut god).await;
    take_off(&mut ada, &mut bo).await;
    let id = i3_match_id(&mut god).await;
    let mut god_web = web_session(&server, "boss", GM_PASSWORD, "God").await;
    let mut ada_web = web_session(&server, "ada", PILOT_PASSWORD, "Ada").await;
    let mut cy_web = web_session(&server, "cy", PILOT_PASSWORD, "Cy").await;

    // A pilot is not staff.
    ask(
        &mut ada,
        &format!("@arena/watch {id}"),
        "Permission denied.",
    )
    .await;

    // ---- the watch, from God's web session --------------------------------
    let god_mark = god.mark();
    send_web(&mut god_web, &format!("@arena/watch {id}")).await;
    let mut replied = false;
    let mut pictures: Vec<Value> = Vec::new();
    for _ in 0..4_000 {
        let env = web_next(&mut god_web).await;
        if text_of(&env.event).contains(&format!("You are watching match {id}:")) {
            replied = true;
        }
        if let Some(data) = observer_payload(&env.event)
            && data.get("units").is_some()
        {
            pictures.push(data);
        }
        let first = pictures.first().and_then(|p| p["now_ms"].as_i64());
        let last = pictures.last().and_then(|p| p["now_ms"].as_i64());
        if replied && first.zip(last).is_some_and(|(f, l)| l >= f + 10_000) {
            break;
        }
    }
    assert!(replied, "the watch was not answered");
    assert!(
        pictures.len() >= 5,
        "ten arena seconds brought {} pictures",
        pictures.len()
    );
    let picture = pictures.last().cloned().expect("a picture");
    assert_eq!(picture["match"], json!(id), "{picture}");
    let rows = picture["units"].as_array().cloned().unwrap_or_default();
    for (callsign, tank) in [("RA", red), ("BA", blue)] {
        assert!(
            rows.iter()
                .any(|row| row[0] == json!(callsign) && row[5] == json!(tank)),
            "the staff picture names {callsign} as #{tank}: {picture}"
        );
    }

    // ---- Ada's web socket, fenced: no `arena.observer` envelope ----------
    send_web(&mut ada_web, &format!("say {WEB_SENTINEL}")).await;
    let ada_saw = web_until(&mut ada_web, WEB_SENTINEL).await;
    assert!(ada_saw.len() > 1, "the fence read nothing");
    for env in &ada_saw {
        assert!(
            observer_payload(&env.event).is_none(),
            "a pilot was sent the staff picture: {:?}",
            env.event
        );
    }

    // ---- Cy, a bystander: nothing to watch -------------------------------
    assert_eq!(
        panel_query(&mut cy_web, 901, OBSERVER_FEED).await,
        json!({
            "schema": { "id": "arena.observer", "version": 1, "retain": "latest" },
            "watching": false,
        })
    );

    // ---- God's SSH line client, watching in parallel: none of the feed ----
    ask_whole(&mut god, SENTINEL).await;
    let god_seen = strip_ansi(&god.seen[god_mark..]);
    assert!(
        god_seen.contains(&format!("You are watching match {id}:")),
        "the line client reads the reply:\n{god_seen}"
    );
    names_nothing(&god_seen, red, blue, "God's line client while watching");
    for leak in ["\"units\"", "now_ms", "in_flight", "(live)", "Watching"] {
        assert!(
            !god_seen.contains(leak),
            "God's line client printed {leak:?} of the feed:\n{god_seen}"
        );
    }
    for callsign in ["RA", "BA"] {
        assert!(
            !words_of(&god_seen).contains(&callsign),
            "God's line client named {callsign}:\n{god_seen}"
        );
    }

    // ---- off: `watching: false`, and nothing more -------------------------
    send_web(&mut god_web, "@arena/watch off").await;
    let off = web_until(&mut god_web, &format!("You stop watching match {id}.")).await;
    assert!(
        off.iter()
            .filter_map(|env| observer_payload(&env.event))
            .any(|data| data.get("watching") == Some(&json!(false))),
        "the cleared picture did not arrive"
    );
    support::wait_arena_seconds(&mut god, 2).await;
    send_web(&mut god_web, &format!("say {WEB_SENTINEL}")).await;
    let after = web_until(&mut god_web, WEB_SENTINEL).await;
    for env in &after {
        assert!(
            observer_payload(&env.event).is_none(),
            "the watcher was sent the feed after `off`: {:?}",
            env.event
        );
    }
    say(&mut god, &format!("@arena/abort {id}"), "is stopping").await;
}

// ---------------------------------------------------------------------------
// 16b cut O.1 — the Hold rule is public, its clock reaches no pilot (16c N11)
// ---------------------------------------------------------------------------

/// The words a Hold clock would be told in; none may reach a pilot while the
/// match runs (16b cut O.1, plan items 10, 17). **The key spot's name is NOT on
/// this list, on purpose**: V.1 prints every spot's name and letter in the map
/// legend, in VIEW's off-window line, to `RANGE`/`BEARING <spot>` and on
/// `STATUS`'s `nearest spot:` — public ground by doc 16 §3.2, and pinned as
/// such elsewhere in this file — so the clock's secrecy is the frame's key set,
/// Bo's board shape and the absence of these words, never the name.
const O1_CLOCK_WORDS: [&str; 6] = [
    "holds ",
    "holding",
    "held since",
    "since ",
    "seconds left",
    "remaining",
];

/// The board's line under Hold, whitespace collapsed.
const O1_BOARD_LINE: &str = "mode: hold — keep The Crossroads (B) uncontested for 120 s";

/// A running board read, as **every** line of the board — the contiguous block
/// from its `Simulator centre` head through its `arena time` row, nothing
/// filtered out of it — with every digit made `#`: its shape, which is the
/// same across two matches whatever their ids and clocks. A row a later change
/// adds anywhere in the block is therefore in the shape, and a Hold board that
/// carried one would no longer be the skirmish board plus the `mode:` line.
fn o1_board_shape(answer: &str) -> Vec<String> {
    let lines: Vec<&str> = answer.lines().map(str::trim).collect();
    let head = lines
        .iter()
        .position(|line| line.starts_with("Simulator centre"))
        .unwrap_or_else(|| panic!("no board head in {answer:?}"));
    let clock = lines
        .iter()
        .skip(head)
        .position(|line| line.starts_with("arena time"))
        .map(|at| head + at)
        .unwrap_or_else(|| panic!("no arena time row in {answer:?}"));
    lines
        .get(head..=clock)
        .unwrap_or_default()
        .iter()
        .map(|line| {
            line.chars()
                .map(|c| if c.is_ascii_digit() { '#' } else { c })
                .collect::<String>()
        })
        .collect()
}

/// **Everything that has reached `term` so far is in its scrollback** (16b cut
/// O.1): a parser refusal sent now is answered after every line the world
/// queued for this session before it, so once its `Huh?` has arrived nothing
/// earlier is still in flight. Retried on the transient, as [`ask_whole`] is.
async fn o1_drain(term: &mut Term) {
    for _ in 0..RETRY_BUDGET {
        let mark = term.mark();
        term.send_line(SENTINEL).await;
        if term.expect_one_of_after(mark, &[HUH, COMMIT_PENDING]).await == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("the sentinel was never answered");
}

/// **The pilot's cockpit frame, asked for** (16b cut O.1): the `arena.cockpit`
/// query answers the world's mirror of the last frame, so it does not wait on a
/// pulse that changed something to push one. Asked until the arena has sent the
/// pilot a frame at all.
async fn o1_cockpit(ws: &mut Ws, first_id: u64) -> Value {
    let mut id = first_id;
    let mut answer = Value::Null;
    wait_until(
        STEP_TIMEOUT,
        "the pilot's first cockpit frame",
        async || {
            id += 1;
            answer = panel_query(ws, id, COCKPIT_FEED).await;
            answer != unseated(COCKPIT_FEED)
        },
    )
    .await;
    answer
}

/// The top-level keys of a cockpit frame.
fn o1_keys(frame: &Value) -> Vec<String> {
    let mut keys: Vec<String> = frame
        .as_object()
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default();
    keys.sort();
    keys
}

/// **16b cut O.1 (16c N11; plan items 10, 17, 28)**: the Hold RULE is public in
/// selection and its CLOCK reaches no pilot.
///
/// In `Selection` God, Cy and both pilots read the board's `mode:` line and the
/// `Mode set to hold` announcement. During the Hold match — red posed on The
/// Crossroads, alone there — a pilot's cockpit frame has exactly the key set a
/// skirmish's frame has; Bo's board read is the skirmish board's shape plus
/// exactly the `mode:` line; and nothing Ada or Bo receive while it runs names a
/// unit or carries a clock word. The match is ended with `SIM ABORT`, and the
/// history row after is public: `aborted [hold]`, with no wait.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn o1_the_hold_rule_is_public_and_its_clock_reaches_no_pilot() {
    let server = boot("observer-hold").await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, mut cy) = login(server.addr, "cy", PILOT_PASSWORD, "Cy").await;
    let (red, blue, _muster, _domain) = seat_the_witnesses(&mut god).await;
    let listing = ask(&mut god, "@find Simulator Lobby", "END OF LIST").await;
    let lobby = dbref_of(&listing, "Simulator Lobby");
    say(
        &mut god,
        &format!("@teleport #{}=#{lobby}", CY.0),
        "Teleported",
    )
    .await;
    say(
        &mut god,
        &format!("@teleport #{}=#{lobby}", GOD.0),
        "Simulator Lobby",
    )
    .await;

    // Match 1, a skirmish: the reference frame and the reference board.
    take_off(&mut ada, &mut bo).await;
    let mut ada_web = web_session(&server, "ada", PILOT_PASSWORD, "Ada").await;
    let skirmish_keys = o1_keys(&o1_cockpit(&mut ada_web, 800).await);
    drop(ada_web);
    let skirmish_board = o1_board_shape(&ask_whole(&mut bo, "SIM STATUS").await);
    say(&mut ada, "SIM ABORT", "aborts match").await;
    cy.expect("Its units are checked in.").await;
    for pilot in [ADA, BO] {
        say(
            &mut god,
            &format!("@teleport #{}=#{lobby}", pilot.0),
            "Teleported",
        )
        .await;
    }

    // Match 2's selection: the rule, set and read by everybody.
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
    say(&mut ada, "SIM MODE hold", "elimination still ends it").await;
    for term in [&mut bo, &mut cy, &mut god] {
        term.expect("Mode set to hold").await;
    }
    for (who, term) in [
        ("Ada", &mut ada),
        ("Bo", &mut bo),
        ("Cy", &mut cy),
        ("God", &mut god),
    ] {
        let board = ask(term, "SIM STATUS", "uncontested for 120 s").await;
        let flat = board.split_whitespace().collect::<Vec<&str>>().join(" ");
        assert!(
            flat.contains(O1_BOARD_LINE),
            "{who} did not read the rule: {flat}"
        );
    }
    // Red's slot 1 opens on the spot; blue's on its authored muster, 120 cells
    // off it, so red is the square's one occupant from the first substep.
    say(
        &mut god,
        "@arena/pose RA=129 97 90",
        "opens at cell (129, 97) heading 90",
    )
    .await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "red", "Red Room").await;
    say(&mut bo, "blue", "Blue Room").await;
    // Each pilot's transcript of match 2 starts here, before its `BOARD`:
    // match 1 already put a `Good hunting` in both scrollbacks, so a wait on
    // the whole scrollback would return at once and read nothing of this one.
    let ada_from = ada.mark();
    let bo_from = bo.mark();
    say(&mut ada, "BOARD", "climb into").await;
    say(&mut bo, "BOARD", "climb into").await;
    ada.expect_after(ada_from, "Good hunting").await;
    bo.expect_after(bo_from, "Good hunting").await;

    // The frame: the skirmish's keys, and not one more.
    let mut ada_web = web_session(&server, "ada", PILOT_PASSWORD, "Ada").await;
    let mut bo_web = web_session(&server, "bo", PILOT_PASSWORD, "Bo").await;
    let mut frames: Vec<Value> = Vec::new();
    for (ws, id) in [(&mut ada_web, 820), (&mut bo_web, 840)] {
        let frame = o1_cockpit(ws, id).await;
        assert_eq!(
            o1_keys(&frame),
            skirmish_keys,
            "a Hold frame's keys differ from a skirmish's: {frame}"
        );
        frames.push(frame);
    }
    // Some arena time with red alone on the spot.
    wait_until(
        STEP_TIMEOUT,
        "the arena clock to pass two seconds",
        async || {
            let staff = ask(&mut god, "@arena/status", "ledger writes awaiting").await;
            support::arena_clock_seconds(&staff).is_some_and(|seconds| seconds >= 2)
        },
    )
    .await;
    // Bo's board: the skirmish's shape, plus exactly the `mode:` line.
    let held_board = o1_board_shape(&ask_whole(&mut bo, "SIM STATUS").await);
    let mode_lines: Vec<&String> = held_board
        .iter()
        .filter(|line| line.starts_with("mode:"))
        .collect();
    assert_eq!(mode_lines.len(), 1, "{held_board:?}");
    let without: Vec<String> = held_board
        .iter()
        .filter(|line| !line.starts_with("mode:"))
        .cloned()
        .collect();
    assert_eq!(
        without, skirmish_board,
        "the Hold board is not the skirmish board plus the rule"
    );
    // A second frame each, read while red holds the spot.
    for (ws, id) in [(&mut ada_web, 860), (&mut bo_web, 880)] {
        let frame = o1_cockpit(ws, id).await;
        assert_eq!(o1_keys(&frame), skirmish_keys, "{frame}");
        frames.push(frame);
    }
    // Nothing either pilot received while it ran names a unit or tells a clock.
    // Both sessions are drained first, so every line the world queued for
    // either pilot while red held the spot is in its scrollback. Each
    // transcript from its match-2 `Good hunting` on: what arrives before it is
    // the boarding (the pod's own room text among it), not the match — and
    // the drain's own `Huh?` in it is the proof the slice was read to its end.
    o1_drain(&mut ada).await;
    o1_drain(&mut bo).await;
    let running = |who: &str, seen: String| {
        let (_, after) = seen
            .rsplit_once("Good hunting.")
            .unwrap_or_else(|| panic!("{who}'s match-2 transcript has no `Good hunting.`"));
        assert!(
            after.contains(HUH),
            "{who}'s transcript was not read to its end: {after:?}"
        );
        after.to_owned()
    };
    let seen = format!(
        "{}\n{}\n{}",
        running("Ada", strip_ansi(&ada.seen[ada_from..])),
        running("Bo", strip_ansi(&bo.seen[bo_from..])),
        frames
            .iter()
            .map(Value::to_string)
            .collect::<Vec<String>>()
            .join("\n")
    );
    names_nothing(
        &seen,
        red,
        blue,
        "a pilot's lines and frames in a Hold match",
    );
    for word in O1_CLOCK_WORDS {
        assert!(
            !seen.contains(word),
            "a pilot was told the clock ({word:?}):\n{seen}"
        );
    }

    // Ended by `SIM ABORT`: the row is public and tagged, with no wait.
    say(&mut ada, "SIM ABORT", "aborts match").await;
    cy.expect("Match 2 is over").await;
    let history = ask_whole(&mut cy, "SIM HISTORY").await;
    let flat = history.split_whitespace().collect::<Vec<&str>>().join(" ");
    history_row_is_public(&flat, 2, "#2 aborted [hold]", None);
    let history = ask_whole(&mut ada, "SIM HISTORY").await;
    let flat = history.split_whitespace().collect::<Vec<&str>>().join(" ");
    history_row_is_public(&flat, 2, "#2 aborted [hold]", Some("RA"));
}

// ---------------------------------------------------------------------------
// 16b cut O.2 — the pod's switch is its own pilot's; everybody else learns only
// through their own receiver (16c N12)
// ---------------------------------------------------------------------------

/// **The words the pod's switch is said in** (16b cut O.2, plan item 28): its
/// verb and its lines (`EMCON`, `emcon`), the refusal's sentence (`jamming
/// pod`), the two notice kinds and the refusal word. None may reach Bo while
/// the match runs.
///
/// **Two words are deliberately NOT on it.** `ecm` is a key every cockpit frame
/// carries — `null` on a platform with no pod — so the assertion on it is that
/// Bo's value is `null`, never that the key is absent; and `pod` is the
/// boarding announcement's and the slot rows' word. **Nor is the corpus the
/// `Selection` window**: `Slot red 1 will field line_tank_ecm.`, the board's
/// `[line_tank_ecm]` and `SIM UNIT`'s `Fieldable units:` list are lobby-public
/// by 16b cut Q.8 (an opponent may answer a choice), so the sweep starts at the
/// running match's `Good hunting.` and not before.
const O2_EMCON_WORDS: [&str; 6] = [
    "emcon",
    "EMCON",
    "jamming pod",
    "emcon_on",
    "emcon_off",
    "no_jammer",
];

/// **16b cut O.2 (16c N12; plan items 22, 28, 38)**: Ada flies the ECM tank and
/// Bo stands inside its bubble. While the match runs Bo's frames carry `ecm` as
/// `null` and never `"on"` or `"off"`, and nothing he receives carries a word of
/// the switch; he learns of the pod only through his own receiver — degraded
/// while Ada's pod radiates, and `Receiver clear.` after her `EMCON off` by the
/// ordinary path. Ada's own frames carry her switch and her lines carry her
/// notice. Cy, who flies nothing, is answered the unseated keyframe. Every line
/// and frame names no unit.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn o2_the_pods_switch_is_told_to_its_pilot_and_bo_learns_only_through_his_receiver() {
    let server = boot("observer-emcon").await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, _cy) = login(server.addr, "cy", PILOT_PASSWORD, "Cy").await;
    let (red, blue, _muster, _domain) = seat_the_witnesses(&mut god).await;

    // Selection: Ada chooses the pod-carrying tank, and staff poses both slot-1
    // tanks four cells apart on the road — Bo inside the pod's 60 m.
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
    say(
        &mut ada,
        "SIM UNIT line_tank_ecm",
        "Slot red 1 will field line_tank_ecm.",
    )
    .await;
    say(
        &mut god,
        "@arena/pose RA=100 97 90",
        "RA opens at cell (100, 97) heading 90 for the next match.",
    )
    .await;
    say(
        &mut god,
        "@arena/pose BA=104 97 270",
        "BA opens at cell (104, 97) heading 270 for the next match.",
    )
    .await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "red", "Red Room").await;
    say(&mut bo, "blue", "Blue Room").await;
    say(&mut ada, "BOARD", "climb into").await;
    say(&mut bo, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;
    bo.expect("Good hunting").await;

    let mut ada_web = web_session(&server, "ada", PILOT_PASSWORD, "Ada").await;
    let mut bo_web = web_session(&server, "bo", PILOT_PASSWORD, "Bo").await;
    let mut cy_web = web_session(&server, "cy", PILOT_PASSWORD, "Cy").await;
    let mut bo_frames: Vec<Value> = Vec::new();

    // Ada's pod radiates once her power is up; Bo, inside the bubble, reads
    // his own receiver degraded — and nothing else about it.
    power_up(&mut ada).await;
    power_up(&mut bo).await;
    let mut seen = String::new();
    wait_until(STEP_TIMEOUT, "Bo's receiver to read degraded", async || {
        seen = ask_whole(&mut bo, "STATUS").await;
        seen.contains("receiver degraded")
    })
    .await;
    assert!(
        !seen.contains("emcon"),
        "Bo's STATUS carried the other tank's switch: {seen}"
    );
    bo.expect("Receiver degraded: something is jamming the sight.")
        .await;
    let mut query = 900_u64;
    let mut jammed = Value::Null;
    wait_until(
        STEP_TIMEOUT,
        "Bo's frame to carry the receiver",
        async || {
            query += 1;
            jammed = panel_query(&mut bo_web, query, COCKPIT_FEED).await;
            jammed.get("receiver").is_some_and(Value::is_object)
        },
    )
    .await;
    bo_frames.push(jammed.clone());
    let ada_on = o1_cockpit(&mut ada_web, 950).await;
    assert_eq!(
        ada_on.get("ecm"),
        Some(&json!("on")),
        "Ada's own frame does not carry her switch: {ada_on}"
    );

    // The switch, said to Ada alone.
    let ada_mark = ada.mark();
    let bo_mark = bo.mark();
    say(
        &mut ada,
        "EMCON off",
        "EMCON going off; the pod is silent from the next pulse.",
    )
    .await;
    ada.expect_after(
        ada_mark,
        "EMCON off: the pod is silent; your own sight clears, and nothing hides you.",
    )
    .await;
    // Bo's receiver clears by the ordinary path: his own pass, his own line.
    bo.expect_after(bo_mark, "Receiver clear.").await;
    let mut clear = Value::Null;
    wait_until(
        STEP_TIMEOUT,
        "Bo's frame to drop the receiver",
        async || {
            query += 1;
            clear = panel_query(&mut bo_web, query, COCKPIT_FEED).await;
            clear.get("receiver") == Some(&Value::Null)
        },
    )
    .await;
    bo_frames.push(clear.clone());
    let seen = ask_whole(&mut bo, "STATUS").await;
    assert!(
        !seen.contains("receiver degraded"),
        "Bo's receiver did not clear: {seen}"
    );
    let mut ada_off = Value::Null;
    wait_until(
        STEP_TIMEOUT,
        "Ada's frame to carry the switch off",
        async || {
            query += 1;
            ada_off = panel_query(&mut ada_web, query, COCKPIT_FEED).await;
            ada_off.get("ecm") == Some(&json!("off"))
        },
    )
    .await;

    // Bo's frames: the key is there, and it is null.
    for frame in &bo_frames {
        assert_eq!(
            frame.get("ecm"),
            Some(&Value::Null),
            "Bo's frame carried a switch that is not his: {frame}"
        );
    }
    // Cy flies nothing: the unseated keyframe, and nothing of the switch.
    let cy_frame = panel_query(&mut cy_web, 990, COCKPIT_FEED).await;
    assert_eq!(cy_frame, unseated(COCKPIT_FEED), "{cy_frame}");

    // Everything Bo received while the match ran — his lines and his frames —
    // carries none of the switch's words; and nothing anybody received names
    // a unit.
    o1_drain(&mut ada).await;
    o1_drain(&mut bo).await;
    let running = |who: &str, seen: String| {
        let (_, after) = seen
            .rsplit_once("Good hunting.")
            .unwrap_or_else(|| panic!("{who}'s transcript has no `Good hunting.`"));
        assert!(
            after.contains(HUH),
            "{who}'s transcript was not read to its end: {after:?}"
        );
        after.to_owned()
    };
    let bo_lines = running("Bo", strip_ansi(&bo.seen));
    let bo_seen = format!(
        "{}\n{}\n{}",
        bo_lines,
        bo_frames
            .iter()
            .map(Value::to_string)
            .collect::<Vec<String>>()
            .join("\n"),
        cy_frame
    );
    for word in O2_EMCON_WORDS {
        assert!(
            !bo_seen.contains(word),
            "Bo was told of the other tank's switch ({word:?}):\n{bo_seen}"
        );
    }
    let ada_seen = running("Ada", strip_ansi(&ada.seen));
    assert!(
        ada_seen.contains("EMCON off: the pod is silent"),
        "{ada_seen}"
    );
    // The frames name no unit at all. The transcripts carry each pilot's own
    // `STATUS` (`power_up` reads it), which names that pilot's OWN tank — the
    // pilot's own platform is outside the observer rule (doc 16 §5) — so each
    // transcript is held to naming nothing of the OTHER side's tank.
    let frames = format!(
        "{}\n{ada_on}\n{ada_off}\n{cy_frame}",
        bo_frames
            .iter()
            .map(Value::to_string)
            .collect::<Vec<String>>()
            .join("\n")
    );
    names_nothing(&frames, red, blue, "the frames around the pod's switch");
    for (who, seen, theirs, dbref) in [
        ("Bo", &bo_lines, RED_TANK, red),
        ("Ada", &ada_seen, BLUE_TANK, blue),
    ] {
        for leak in [theirs.to_owned(), format!("#{dbref}")] {
            assert!(
                !seen.contains(&leak),
                "{who}'s lines around the pod's switch leaked {leak:?}:\n{seen}"
            );
        }
    }
    // A frame carries each contact's row key by design (it is never printed);
    // no LINE either pilot read carries one.
    no_sixteen_digit_word(&format!("{bo_lines}\n{ada_seen}"), "the pod's switch");

    say(&mut ada, "SIM ABORT", "aborts match").await;
}

// ---------------------------------------------------------------------------
// 16b cut O.3 — the range is public in selection, and the report is the
// pilot's own (16c N13, N2)
// ---------------------------------------------------------------------------

/// The board's range line, flattened as `ask`'s answers are read.
const O3_BOARD_LINE: &str = "mode: range — 1 target on the blue side";

/// The `SIM RANGE 1` announcement with red claimed — its head, which a terminal
/// does not wrap (the whole sentence runs past eighty columns; the whole of it
/// is pinned verbatim in `tacsim/tests/cockpit.rs`).
const O3_SET: &str = "Range set: 1 target stands on the blue side;";

/// **16b cut O.3 (16c N13, N2; plan items 21, 31)**: the range is public in
/// `Selection` and the debrief is the pilot's own.
///
/// God, Cy and Ada read the `Range set:` announcement and the board's
/// `mode: range` line while the lobby is being set up — both public, as the
/// Hold rule is. Ada then shoots at the one target from behind it until a round
/// is told to her as a hit, and aborts. Bo, who holds no slot and flew nothing,
/// is refused `SIM REPORT` for that match with the one sentence the centre gives
/// anybody who did not fly it. Ada's report carries her own hit rate and no line
/// about a round the target fired — it fired none — and passes
/// `report_is_the_pilots_own`; every line any of them read names no unit.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn o3_the_range_is_public_in_selection_and_the_report_is_the_pilots_own() {
    let server = boot("observer-range").await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, mut cy) = login(server.addr, "cy", PILOT_PASSWORD, "Cy").await;
    let (red, blue, _muster, _domain) = seat_the_witnesses(&mut god).await;
    let listing = ask(&mut god, "@find Simulator Lobby", "END OF LIST").await;
    let lobby = dbref_of(&listing, "Simulator Lobby");
    say(
        &mut god,
        &format!("@teleport #{}=#{lobby}", CY.0),
        "Teleported",
    )
    .await;
    say(
        &mut god,
        &format!("@teleport #{}=#{lobby}", GOD.0),
        "Simulator Lobby",
    )
    .await;

    // Selection: the range, set by Ada and read by everybody.
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    let marks = (god.mark(), cy.mark());
    say(&mut ada, "SIM RANGE 1", O3_SET).await;
    god.expect_after(marks.0, O3_SET).await;
    cy.expect_after(marks.1, O3_SET).await;
    let mut public = String::new();
    for (who, term) in [("Ada", &mut ada), ("Cy", &mut cy), ("God", &mut god)] {
        let board = ask(term, "SIM STATUS", "target on the blue side").await;
        let flat = board.split_whitespace().collect::<Vec<&str>>().join(" ");
        assert!(
            flat.contains(O3_BOARD_LINE),
            "{who} did not read the range: {flat}"
        );
        public.push_str(&board);
    }
    public.push_str(O3_SET);
    names_nothing(&public, red, blue, "the range's public lines");

    // The shooter behind the target, both facing east.
    say(
        &mut god,
        "@arena/pose BA=130 97 90",
        "BA opens at cell (130, 97) heading 90 for the next match.",
    )
    .await;
    say(
        &mut god,
        "@arena/pose RA=126 97 90",
        "RA opens at cell (126, 97) heading 90 for the next match.",
    )
    .await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "red", "Red Room").await;
    let from = ada.mark();
    say(&mut ada, "BOARD", "climb into").await;
    ada.expect_after(from, "Good hunting").await;
    power_up(&mut ada).await;
    wait_until(STEP_TIMEOUT, "the target to be told apart", async || {
        ask_whole(&mut ada, "CONTACTS").await.contains("BA ")
    })
    .await;
    // The lock is the arena's at its next pulse: a `FIRE` sent before it lands
    // is refused `No lock`, so the lock is waited for on the lock's own line.
    let locking = ada.mark();
    say(&mut ada, "LOCK BA", "Locking").await;
    ada.expect_after(locking, "Locked on BA").await;
    let mut hit = false;
    for _ in 0..20 {
        wait_until(
            Duration::from_secs(12),
            "the cannon to be ready",
            async || {
                ask_whole(&mut ada, "WEAPONSTATUS")
                    .await
                    .lines()
                    .any(|line| line.contains("cannon") && line.contains("ready"))
            },
        )
        .await;
        let shot = ada.mark();
        ada.send_line("FIRE").await;
        if ada
            .expect_one_of_after(shot, &["Hit on BA", "Miss on BA", "refused"])
            .await
            == 0
        {
            hit = true;
            break;
        }
    }
    assert!(hit, "twenty rounds at a standing target and not one hit");
    // Her cockpit names her own tank (`STATUS` does) and never the target's
    // object: the target is `BA`, as her sensors told it apart.
    let cockpit = strip_ansi(&ada.seen[from..]);
    for leak in [BLUE_TANK.to_string(), format!("#{blue}")] {
        assert!(
            !cockpit.contains(&leak),
            "Ada's cockpit on the range leaked {leak:?}:\n{cockpit}"
        );
    }
    no_sixteen_digit_word(&cockpit, "Ada's cockpit on the range");

    say(&mut ada, "SIM ABORT", "aborts match").await;
    cy.expect("Match 1 is over").await;

    // Bo flew nothing: one sentence.
    let refused = ask(&mut bo, "SIM REPORT 1", "You did not fly in match 1.").await;
    names_nothing(&refused, red, blue, "Bo's refused report");

    // Ada's report: her own rate, and nothing of a target's rounds.
    let report = sim_report(&mut ada, "SIM REPORT").await;
    let flat = report.split_whitespace().collect::<Vec<_>>().join(" ");
    let line = flat
        .split("You flew RA: ")
        .nth(1)
        .and_then(|rest| rest.split('.').next())
        .unwrap_or_else(|| panic!("no `You flew RA:` in:\n{report}"));
    let rate: u64 = line
        .rsplit_once('(')
        .and_then(|(_, tail)| tail.split_once("%)"))
        .and_then(|(per_cent, _)| per_cent.parse().ok())
        .unwrap_or_else(|| panic!("`You flew RA: {line}` carries no rate"));
    assert!((1..=100).contains(&rate), "{rate}% in:\n{report}");
    assert!(
        !flat.contains("hit on ") && !flat.contains("your hull"),
        "the report names a round the target fired:\n{report}"
    );
    report_is_the_pilots_own(&report, red, blue, "Ada's report of the range");
    names_nothing(&report, red, blue, "Ada's report of the range");

    // And the ring says it was a range, publicly.
    let history = ask_whole(&mut cy, "SIM HISTORY").await;
    let flat = history.split_whitespace().collect::<Vec<&str>>().join(" ");
    history_row_is_public(&flat, 1, "#1 aborted [range]", None);
}

/// Everything `line` answered and nothing else: [`ask_whole`]'s slice from the
/// LAST echo of `line` (a retried line is echoed again) to the sentinel's
/// `Huh?`, with the sentinel's own echo taken out (the terminal echoes both
/// typed lines before either is answered), trimmed. The world answers in the
/// order it took the lines, so the slice is the whole answer, not the part of
/// it before a needle. A transient in it (a world mid-commit answered `line`,
/// not the sentinel) is a retry, not an answer.
async fn whole_macro_answer(term: &mut Term, line: &str) -> String {
    for _ in 0..RETRY_BUDGET {
        let seen = ask_whole(term, line).await;
        let (_, after) = seen
            .rsplit_once(line)
            .unwrap_or_else(|| panic!("no echo of `{line}` in:\n{seen}"));
        let (before_huh, _) = after
            .split_once(HUH)
            .unwrap_or_else(|| panic!("no sentinel answer after `{line}` in:\n{seen}"));
        let answer = before_huh.replacen(SENTINEL, "", 1);
        let answer = answer.trim();
        if answer.contains(COMMIT_PENDING) || answer.contains(CONTROL_UNKNOWN) {
            tokio::time::sleep(Duration::from_millis(200)).await;
            continue;
        }
        return answer.to_owned();
    }
    panic!("`{line}` was refused as transient {RETRY_BUDGET} times running")
}

/// **`@macro`'s refusal on a name names nothing** (16b cut AD.1, 16c N14; doc 16
/// §5 applied to a refusal).
///
/// A bystander asks for macros named after words that collide with very
/// different things — a core verb, the kernel's `home` and `quit`, tacsim's bare
/// `board`, and one of their own channel aliases — and every answer is the one
/// `TAKEN` sentence, byte for byte. Then after `@` and `+` words: two real staff
/// verbs of the production composition (`@arena`, `@unit`) and two that exist
/// nowhere, and every answer is the one `BAD_NAME` sentence, because the name's
/// characters are checked before anything is consulted. Nothing in either group
/// says what was hit. No match is started.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_taken_name_is_refused_the_same_way_whatever_it_collided_with() {
    let server = boot("observer-macro").await;
    let (_c, mut cy) = login(server.addr, "cy", PILOT_PASSWORD, "Cy").await;

    // Comsys seeds Public on the first boot and joins every existing character
    // to it as `pub`; Cy existed before the modules booted. The bare alias is
    // claimed and prompts, which proves the membership without a broadcast.
    say(&mut cy, "pub", "What do you want to say on Public?").await;

    let taken = mush_core::commands::macros::TAKEN;
    let bad_name = mush_core::commands::macros::BAD_NAME;
    let mut groups: Vec<Vec<String>> = Vec::new();
    for (words, expected) in [
        (&["look", "home", "quit", "board", "pub"][..], taken),
        (&["@arena", "@unit", "@zzqnotaverb", "+zzq"][..], bad_name),
    ] {
        let mut answers = Vec::new();
        for word in words {
            let line = format!("@macro {word}=say x");
            let answer = whole_macro_answer(&mut cy, &line).await;
            // The WHOLE answer, echo to sentinel, is the one sentence: nothing
            // before or after it names what the word collided with.
            assert_eq!(answer, expected, "`{line}` answered:\n{answer}");
            for leak in ["arena", "unit", "board"] {
                assert!(
                    !answer.to_ascii_lowercase().contains(leak),
                    "`{line}` named what it hit:\n{answer}"
                );
            }
            answers.push(answer);
        }
        for pair in answers.windows(2) {
            assert_eq!(pair[0], pair[1], "one sentence, byte for byte");
        }
        groups.push(answers);
    }
    assert_eq!(groups.len(), 2);
}

// ---------------------------------------------------------------------------
// 16b cut AE.2 — a round in flight tells its target nothing, and a miss on a
// contact the shooter no longer holds says no more than `Miss.` (16c N15)
// ---------------------------------------------------------------------------

/// Where the missile witnesses stand (16b cut AE.2): Bo's heavy at (160, 140)
/// facing south, Ada's stock tank 200 m south of it at (160, 120) facing east,
/// so it shows the heavy its whole left side.
const AE2_SHOOTER_POSE: &str = "@arena/pose BA=160 140 180";
const AE2_TARGET_POSE: &str = "@arena/pose RA=160 120 90";

/// **Where staff puts Ada's tank for the lost-contact shot** (16b cut AE.2):
/// 110 cells south of Bo's heavy, behind the compound at columns 150 to 170 and
/// rows 36 to 52 — out of every sight, and 1.1 km away, so the missile is 4.4 s
/// in the air and Bo's contact on the tank (last refreshed before the place, at
/// the same wake) expires three sensor passes into the flight.
const AE2_BEHIND_THE_COMPOUND: (u16, u16) = (160, 30);

/// The lines of `seen` a pilot was told, without the sentinel's echo and its
/// refusal — what [`o1_drain`] adds to a transcript to prove it complete.
fn ae2_told(seen: &str) -> Vec<String> {
    seen.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.contains(SENTINEL) && !line.starts_with(HUH))
        .map(str::to_owned)
        .collect()
}

/// **A missile tells its target nothing in the air, and a shooter that has lost
/// the contact reads `Miss.` and no reason** (16b cut AE.2; doc 16 §1, §5 and
/// §3.5; 16c N2), over the production composition and real SSH.
///
/// Bo flies the heavy and Ada a stock tank 200 m off, broadside to him; each
/// holds the other, told apart, before anything is fired.
///
/// 1. **In flight, nothing.** Bo fires his rack at Ada until a round lands. For
///    every shot, what Ada is sent from the moment before Bo's `FIRE missiles`
///    to the moment Bo is told the outcome — drained to the last line with a
///    sentinel — is nothing at all for a miss, and for the hit only her own
///    tank's lines: `Hit on the …` and, if a section went, `The … is
///    destroyed`; no shooter, no callsign, no object number, no weapon.
/// 2. **The lost contact.** Under a 5,000 ms staff floor, God's `@arena/place`
///    of Ada's tank to behind the compound, 1.1 km away, and Bo's `FIRE
///    missiles` land at one wake; phase 4 takes the place first (her tank's id
///    is the lower) and the round flies at where the tank now is. Bo's contact
///    expires three sensor passes into the 4.4 s flight, and the round finds the
///    compound's wall. Bo reads `Miss.` — no `moved`, no `blocked`, no `cover`:
///    where the target went and what stood round it are not his to know once
///    the contact is gone. Ada reads nothing of it.
/// 3. **The debrief says the same.** Bo's `SIM REPORT` closes that shot as
///    `missiles at a contact: miss.` and carries no miss reason for an unheld
///    target anywhere.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ae2_a_missile_in_flight_tells_its_target_nothing_and_a_lost_contact_reads_miss() {
    let server = boot("observer-ae2").await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (red, blue, _muster, _domain) = seat_the_witnesses(&mut god).await;
    assert!(
        red < blue,
        "phase 4 drains in unit-id order; this test needs the target's id below the shooter's \
         ({red} against {blue})"
    );

    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
    say(&mut bo, "SIM UNIT heavy_mk1", "will field heavy_mk1").await;
    say(
        &mut god,
        AE2_SHOOTER_POSE,
        "BA opens at cell (160, 140) heading 180 for the next match.",
    )
    .await;
    say(
        &mut god,
        AE2_TARGET_POSE,
        "RA opens at cell (160, 120) heading 90 for the next match.",
    )
    .await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "red", "Red Room").await;
    say(&mut bo, "blue", "Blue Room").await;
    say(&mut ada, "BOARD", "climb into").await;
    say(&mut bo, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;
    power_up(&mut ada).await;
    power_up(&mut bo).await;
    let id = i3_match_id(&mut god).await;

    // Each holds the other, told apart.
    for term in [&mut ada, &mut bo] {
        wait_until(
            STEP_TIMEOUT,
            "the other tank to be told apart",
            async || !first_contact(term).await.starts_with('X'),
        )
        .await;
    }
    let locking = bo.mark();
    say(&mut bo, "LOCK RA", "Locking").await;
    bo.expect_after(locking, "Locked on RA").await;

    // ---- (1) in flight, nothing --------------------------------------------
    let mut hit = false;
    for _ in 0..4 {
        wait_until(STEP_TIMEOUT, "the rack to be ready", async || {
            ask_whole(&mut bo, "WEAPONSTATUS")
                .await
                .contains("missiles  ready")
        })
        .await;
        o1_drain(&mut ada).await;
        let from = ada.mark();
        let shot = bo.mark();
        bo.send_line("FIRE missiles").await;
        bo.expect_after(shot, "Firing missiles.").await;
        let outcome = bo
            .expect_one_of_after(shot, &["Hit on RA", "Miss on RA", "refused"])
            .await;
        assert_ne!(
            outcome,
            2,
            "the rack was refused: {}",
            strip_ansi(&bo.seen[shot..])
        );
        o1_drain(&mut ada).await;
        let told = ae2_told(&strip_ansi(&ada.seen[from..]));
        let whole = told.join("\n");
        for leak in [
            "BA".to_owned(),
            "missile".to_owned(),
            "Firing".to_owned(),
            "bearing".to_owned(),
            BLUE_TANK.to_owned(),
            format!("#{blue}"),
            format!("#{red}"),
        ] {
            assert!(
                !whole.contains(&leak),
                "the target's pilot was told {leak:?} about a round fired at her:\n{whole}"
            );
        }
        if outcome == 0 {
            assert!(
                told.iter().any(|line| line.starts_with("Hit on the ")),
                "the hit did not reach its target's pilot as her own tank's line:\n{whole}"
            );
            assert!(
                told.iter().all(|line| line.starts_with("Hit on the ")
                    || (line.starts_with("The ") && line.contains(" is destroyed"))),
                "the target's pilot read something beside her own tank's lines:\n{whole}"
            );
            hit = true;
            break;
        }
        assert!(
            told.is_empty(),
            "a missile that missed told its target something:\n{whole}"
        );
    }
    assert!(
        hit,
        "four missiles at a broadside tank 200 m off and not one hit"
    );

    // ---- (2) the contact lost in flight -------------------------------------
    wait_until(STEP_TIMEOUT, "the rack to be ready again", async || {
        ask_whole(&mut bo, "WEAPONSTATUS")
            .await
            .contains("missiles  ready")
    })
    .await;
    assert!(
        ask(
            &mut god,
            &format!("@arena/tempo {id}=5000"),
            "under a floor"
        )
        .await
        .contains("under a floor of 5000 ms")
    );
    o1_drain(&mut ada).await;
    let from = ada.mark();
    let fire_mark = bo.mark();
    let god_mark = god.mark();
    // Back to back: both wait in the task's buffer for the same wake.
    bo.send_line("FIRE missiles").await;
    god.send_line(&format!(
        "@arena/place #{red}={},{}",
        AE2_BEHIND_THE_COMPOUND.0, AE2_BEHIND_THE_COMPOUND.1
    ))
    .await;
    bo.expect_after(fire_mark, "Firing missiles.").await;
    assert_eq!(
        god.expect_one_of_after(
            god_mark,
            &["Placing", "impassable", "off the arena", COMMIT_PENDING]
        )
        .await,
        0,
        "the place was not sent: {}",
        strip_ansi(&god.seen[god_mark..])
    );
    // The round leaves the rack at the next wake, a floor away at most. Then the
    // floor is lifted — a 5,000 ms floor over this file's 100 ms tempo is slow
    // motion — and the rest, the contact's expiry three passes in and the
    // impact 4.4 s after the shot, happens at its own arena milliseconds
    // whatever the wall does.
    wait_until(STEP_TIMEOUT, "the missile to leave the rack", async || {
        ask_whole(&mut bo, "WEAPONSTATUS")
            .await
            .contains("missiles  cycling")
    })
    .await;
    say(
        &mut god,
        &format!("@arena/tempo {id}=off"),
        "its own tempo again",
    )
    .await;
    wait_until(STEP_TIMEOUT, "Bo to be told the outcome", async || {
        o1_drain(&mut bo).await;
        ae2_told(&strip_ansi(&bo.seen[fire_mark..]))
            .iter()
            .any(|line| line.starts_with("Hit") || line.starts_with("Miss"))
    })
    .await;
    let told = ae2_told(&strip_ansi(&bo.seen[fire_mark..]));
    let whole = told.join("\n");
    assert!(
        told.iter().any(|line| line == "Miss."),
        "a miss on a contact Bo no longer holds is the one word:\n{whole}"
    );
    for leak in ["moved", "blocked", "cover", "Miss:", "Miss on", "Hit"] {
        assert!(
            !whole.contains(leak),
            "Bo was told {leak:?} about a target he no longer holds:\n{whole}"
        );
    }
    o1_drain(&mut ada).await;
    let told = ae2_told(&strip_ansi(&ada.seen[from..]));
    let whole = told.join("\n");
    // Her own sensor losing the heavy (`Contact BA lost.`) is hers to read;
    // nothing about the round or the place is.
    for leak in ["Hit on", "Miss", "missile", "place", "Placing", "in_flight"] {
        assert!(
            !whole.contains(leak),
            "Ada was told {leak:?} about a round that went into a wall:\n{whole}"
        );
    }

    // ---- (3) the debrief -----------------------------------------------------
    say(&mut god, &format!("@arena/abort {id}"), "is stopping").await;
    let mut events = Vec::new();
    wait_until(
        STEP_TIMEOUT,
        "the place and the miss to reach the tape",
        async || {
            events = ledger_events(&server.store, id).await;
            events.iter().any(|event| {
                event.get("kind").and_then(Value::as_str) == Some("miss")
                    && event.get("reason").and_then(Value::as_str) == Some("blocked")
            })
        },
    )
    .await;
    assert!(
        events.iter().any(|event| {
            event.get("kind").and_then(Value::as_str) == Some("placed")
                && event.get("unit").and_then(Value::as_u64) == Some(u64::from(red))
        }),
        "the place was refused, so the round was not fired at where the tank went"
    );
    let report = sim_report(&mut bo, &format!("SIM REPORT {id}")).await;
    // The report itself, from its first line: the slice `sim_report` answers
    // starts at the command and may carry what the centre said before it (the
    // pod's hatch, a channel's `removed`).
    let body = report
        .split_once("Reading your report")
        .map_or(report.as_str(), |(_, body)| body);
    let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("missiles at a contact: miss."),
        "the lost contact's shot is not `miss.` in Bo's report:\n{report}"
    );
    for leak in ["a contact: miss:", "blocked", "moved", "cover"] {
        assert!(
            !flat.contains(leak),
            "Bo's report carried {leak:?} for a target he did not hold:\n{report}"
        );
    }
    report_is_the_pilots_own(&report, red, blue, "Bo's report of the missiles");
    no_sixteen_digit_word(&strip_ansi(&ada.seen), "Ada's transcript");
    no_sixteen_digit_word(&strip_ansi(&bo.seen), "Bo's transcript");
}

// ---------------------------------------------------------------------------
// 16b cut AE.3 — the drone's refusals and its mesh, through the lens
// ---------------------------------------------------------------------------

/// The cap's refusal at the shipped default (16b cut AE.3).
const AE3_CAP: &str = "A side fields at most 1 drone; pick another type.";

/// The `NoGun` refusal for red (16b cut AE.3).
const AE3_NO_GUN: &str = "The red side fields no gun; a side needs a machine that carries one.";

/// Where the AE.3 witnesses stand, on the east-west road: Ada's drone 45 cells
/// west of Bo's tank (inside the drone's 60-cell sight), Cy's tank 25 cells
/// behind the drone (inside the mesh's 30-cell link) and 70 cells from Bo
/// (outside every sight of its own), so whatever Cy holds of Bo is the plot
/// the drone shared.
const AE3_DRONE_POSE: &str = "@arena/pose RA=115 97 90";
const AE3_TANK_POSE: &str = "@arena/pose RB=90 97 90";
const AE3_BLUE_POSE: &str = "@arena/pose BA=160 97 270";

/// **The drone's two refusals reach their caller alone, and its mesh is the
/// mesh** (16b cut AE.3; doc 16 §1, §5 and §2 W2).
///
/// 1. **The cap.** Ada and Cy claim red and Ada chooses the drone; Cy's own
///    `SIM UNIT drone_mk1` is refused with the cap's sentence, which reaches
///    Cy alone and names no slot, no pilot and no object.
/// 2. **`NoGun`.** Cy gives his slot up, leaving red a lone drone against Bo,
///    and Ada's `SIM START` is refused with the `NoGun` sentence — hers alone,
///    naming a side and nothing in it.
/// 3. **The mesh.** Cy claims red again, staff poses the three machines on the
///    road, and they board. Ada's drone holds Bo's tank on its own sight, and
///    Cy's tank — 700 m from it, past its own 600 m — holds it too, as an
///    ordinary row whose plot is further than its own sight: the shared plot,
///    and nothing more than any teammate's share would give it. From the moment
///    they climbed in, neither Bo nor Cy was sent the word `drone`, and no
///    transcript carries a bare contact id.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ae3_the_drones_refusals_are_the_callers_and_its_mesh_is_the_mesh() {
    let server = boot("observer-ae3").await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let listing = ask(&mut god, "@find Simulator Lobby", "END OF LIST").await;
    let lobby = dbref_of(&listing, "Simulator Lobby");
    for pilot in [ADA, BO, CY] {
        say(
            &mut god,
            &format!("@teleport #{}=#{lobby}", pilot.0),
            "Teleported",
        )
        .await;
    }
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, mut cy) = login(server.addr, "cy", PILOT_PASSWORD, "Cy").await;

    // ---- (1) the cap --------------------------------------------------------
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut cy, "SIM TEAM red", "takes red slot 2").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
    say(
        &mut ada,
        "SIM UNIT drone_mk1",
        "Slot red 1 will field drone_mk1.",
    )
    .await;
    o1_drain(&mut ada).await;
    o1_drain(&mut bo).await;
    let (ada_from, bo_from) = (ada.mark(), bo.mark());
    let seen = ask(&mut cy, "SIM UNIT drone_mk1", AE3_CAP).await;
    let refusal = seen
        .lines()
        .find(|line| line.contains(AE3_CAP))
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    assert_eq!(refusal, AE3_CAP, "the refusal is the one sentence: {seen}");
    for leak in ["Ada", "Bo", "slot", RED_TANK, BLUE_TANK, "#"] {
        assert!(
            !refusal.contains(leak),
            "the cap's refusal named {leak:?}: {refusal}"
        );
    }
    o1_drain(&mut ada).await;
    o1_drain(&mut bo).await;
    for (who, term, from) in [("Ada", &ada, ada_from), ("Bo", &bo, bo_from)] {
        assert!(
            !strip_ansi(&term.seen[from..]).contains(AE3_CAP),
            "{who} was sent Cy's refusal"
        );
    }

    // ---- (2) no gun ---------------------------------------------------------
    say(&mut cy, "SIM LEAVE", "gives up red slot 2").await;
    o1_drain(&mut bo).await;
    o1_drain(&mut cy).await;
    let (bo_from, cy_from) = (bo.mark(), cy.mark());
    let seen = ask(&mut ada, "SIM START", AE3_NO_GUN).await;
    let refusal = seen
        .lines()
        .find(|line| line.contains(AE3_NO_GUN))
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    assert_eq!(refusal, AE3_NO_GUN, "{seen}");
    for leak in ["Ada", "Bo", "Cy", "drone", "slot", RED_TANK, "#"] {
        assert!(
            !refusal.contains(leak),
            "the NoGun refusal named {leak:?}: {refusal}"
        );
    }
    o1_drain(&mut bo).await;
    o1_drain(&mut cy).await;
    for (who, term, from) in [("Bo", &bo, bo_from), ("Cy", &cy, cy_from)] {
        assert!(
            !strip_ansi(&term.seen[from..]).contains(AE3_NO_GUN),
            "{who} was sent Ada's refusal"
        );
    }

    // ---- (3) the mesh -------------------------------------------------------
    say(&mut cy, "SIM TEAM red", "takes red slot 2").await;
    say(
        &mut god,
        AE3_DRONE_POSE,
        "RA opens at cell (115, 97) heading 90 for the next match.",
    )
    .await;
    say(
        &mut god,
        AE3_TANK_POSE,
        "RB opens at cell (90, 97) heading 90 for the next match.",
    )
    .await;
    say(
        &mut god,
        AE3_BLUE_POSE,
        "BA opens at cell (160, 97) heading 270 for the next match.",
    )
    .await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "red", "Red Room").await;
    say(&mut cy, "red", "Red Room").await;
    say(&mut bo, "blue", "Blue Room").await;
    let (bo_from, cy_from) = (bo.mark(), cy.mark());
    say(&mut ada, "BOARD", "climb into").await;
    say(&mut cy, "BOARD", "climb into").await;
    say(&mut bo, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;
    power_up(&mut ada).await;
    power_up(&mut cy).await;
    power_up(&mut bo).await;

    // The drone holds BA on its own sight, and carries nothing that fires.
    let mut drone_row = String::new();
    wait_until(STEP_TIMEOUT, "the drone to tell BA apart", async || {
        drone_row = ask_whole(&mut ada, "CONTACTS").await;
        contact_range_m(&drone_row, "BA").is_some()
    })
    .await;
    let drone_m = contact_range_m(&drone_row, "BA").expect("the drone's row");
    assert!(
        drone_m <= 600,
        "the drone saw BA with its own 600 m sight: {drone_row}"
    );
    // Cy's tank holds it too — past its own sight, so through the mesh.
    let mut tank_row = String::new();
    wait_until(STEP_TIMEOUT, "the tank to hold BA", async || {
        tank_row = ask_whole(&mut cy, "CONTACTS").await;
        contact_range_m(&tank_row, "BA").is_some()
    })
    .await;
    let tank_m = contact_range_m(&tank_row, "BA").expect("the tank's row");
    assert!(
        (600..=900).contains(&tank_m),
        "a plot past the tank's own 600 m and inside one mesh hop is the drone's share: \
         {tank_row}"
    );
    let row = strip_ansi(&tank_row)
        .lines()
        .find(|line| line.trim_start().starts_with("BA "))
        .unwrap_or_default()
        .to_owned();
    for leak in ["drone", "Ada", "shared", "#"] {
        assert!(
            !row.contains(leak),
            "the shared row says more than any teammate's share would ({leak:?}): {row}"
        );
    }
    assert!(
        !row.split_whitespace().any(|word| word == "RA"),
        "the shared row names the teammate that shared it: {row}"
    );

    // Bo's tank holds the drone on its own sight (450 m of road, inside its
    // 600 m): his row for it names a side and nothing of what the machine is.
    let mut bo_row = String::new();
    wait_until(STEP_TIMEOUT, "Bo's tank to tell RA apart", async || {
        bo_row = ask_whole(&mut bo, "CONTACTS").await;
        contact_range_m(&bo_row, "RA").is_some()
    })
    .await;
    let row = strip_ansi(&bo_row)
        .lines()
        .find(|line| line.trim_start().starts_with("RA "))
        .unwrap_or_default()
        .to_owned();
    for leak in ["drone", "Ada", "#"] {
        assert!(
            !row.contains(leak),
            "Bo's row for the drone says {leak:?}: {row}"
        );
    }

    // Everything sent to Bo and Cy since they climbed in, read to the end.
    o1_drain(&mut bo).await;
    o1_drain(&mut cy).await;
    for (who, term, from) in [("Bo", &bo, bo_from), ("Cy", &cy, cy_from)] {
        let seen = strip_ansi(&term.seen[from..]);
        assert!(
            !seen.contains("drone"),
            "{who} was told the word `drone` once aboard: {seen}"
        );
    }
    no_sixteen_digit_word(&strip_ansi(&ada.seen), "Ada's transcript");
    no_sixteen_digit_word(&strip_ansi(&bo.seen), "Bo's transcript");
    no_sixteen_digit_word(&strip_ansi(&cy.seen), "Cy's transcript");
}

// ---------------------------------------------------------------------------
// 16b cut AF.3 — the resume says nothing it may not
// ---------------------------------------------------------------------------

/// **Stop a server the way a crash stops it** (16b cut AF.3): its arena tasks
/// told to stop, then its world task and its store writer aborted and joined,
/// before any of its sessions is dropped. Dropping the `Server` alone detaches
/// those tasks rather than ending them, and a world that is still running when
/// the pilots' sessions go handles their disconnects — releasing every seat and
/// taking one more snapshot — which is a clean shutdown the resume would then
/// read, not the crash under test.
async fn crash(server: &mut Server) {
    if let Some(arenas) = &server._handles.arenas {
        arenas.signal_stop_all();
    }
    server._handles.world_join.abort();
    let _ = (&mut server._handles.world_join).await;
    server._handles.writer_join.abort();
    let _ = (&mut server._handles.writer_join).await;
}

/// **The resume's surfaces name nothing** (16b cut AF.3; doc 16 §1, §3.6, §5):
/// the composed server, restarted over its store mid-match as `sim_rematch.rs`
/// boots twice, asked the observer rule's questions of every new surface.
///
/// - Each pilot's two lines — the resume line and the doctrine line — name
///   their own tank and nothing else: Ada's never say `Blue Tank 1`, Bo's never
///   `Red Tank 1`.
/// - The other side's board row a pilot reads is Y.3's reduced row, byte for
///   byte: index, callsign and pilot, and no `doctrine` or `left`.
/// - A bystander in the lobby reads nothing of the resume at all.
/// - After Ada takes her tank back, every designator she does not hold still
///   answers the one byte-identical `No such contact.`
/// - Staff read the `resumed:` row on `@arena/status`; a pilot's
///   `@arena/status` is refused with the same bytes it was before the restart.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn af3_the_resume_names_nothing_a_pilot_or_a_bystander_may_not_read() {
    let mut first = boot("observer-resume-a").await;
    let (_g, mut god) = login(first.addr, "boss", GM_PASSWORD, "God").await;
    let listing = ask(&mut god, "@find Simulator Lobby", "END OF LIST").await;
    let lobby = dbref_of(&listing, "Simulator Lobby");
    for who in [ADA, BO, CY] {
        say(
            &mut god,
            &format!("@teleport #{}=#{lobby}", who.0),
            "Teleported",
        )
        .await;
    }
    let (_a, mut ada) = login(first.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(first.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let refused_before = ask(&mut ada, "@arena/status", "Permission denied.").await;
    take_off(&mut ada, &mut bo).await;
    let store = Arc::clone(&first.store);
    wait_until(STEP_TIMEOUT, "the snapshot to land", async || {
        store
            .module_json_get("tacsim", "tacsim:snapshot:1")
            .await
            .expect("the store read")
            .is_some()
    })
    .await;
    crash(&mut first).await;
    drop(ada);
    drop(bo);
    drop(god);
    drop(first);

    let second = boot_over(
        "observer-resume-b",
        false,
        SessionConfig::default().linkdead_grace,
        Some(Arc::clone(&store)),
    )
    .await;
    let (_g2, mut god) = login(second.addr, "boss", GM_PASSWORD, "God").await;
    let (_c2, mut cy) = login(second.addr, "cy", PILOT_PASSWORD, "Cy").await;
    let (_a2, mut ada) = login(second.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b2, mut bo) = login(second.addr, "bo", PILOT_PASSWORD, "Bo").await;
    ada.expect(&format!(
        "{RED_TANK} is flying on doctrine; BOARD takes it back."
    ))
    .await;
    bo.expect(&format!(
        "{BLUE_TANK} is flying on doctrine; BOARD takes it back."
    ))
    .await;
    // Two more boarding ticks for anything late, then everything read to the end.
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    for term in [&mut ada, &mut bo, &mut cy] {
        say(term, SENTINEL, HUH).await;
    }
    // Flattened: the line client wraps at eighty columns, and the resume line
    // is longer than that.
    let flat = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let (ada_seen, bo_seen, cy_seen) = (
        flat(&strip_ansi(&ada.seen)),
        flat(&strip_ansi(&bo.seen)),
        flat(&strip_ansi(&cy.seen)),
    );
    assert!(ada_seen.contains(tacsim::match_::RESUME_LINE), "{ada_seen}");
    assert!(bo_seen.contains(tacsim::match_::RESUME_LINE), "{bo_seen}");
    assert!(
        !ada_seen.contains(BLUE_TANK),
        "Ada read the other tank: {ada_seen}"
    );
    assert!(
        !bo_seen.contains(RED_TANK),
        "Bo read the other tank: {bo_seen}"
    );
    for word in ["resumed", "doctrine", RED_TANK, BLUE_TANK] {
        assert!(
            !cy_seen.contains(word),
            "the bystander read {word:?} of the resume: {cy_seen}"
        );
    }

    // The other side's row: Y.3's reduced row, byte for byte.
    let board = strip_ansi(&ask(&mut ada, "SIM STATUS", "blue:").await);
    let blue_row = board
        .lines()
        .find(|line| line.trim_start().starts_with("blue:"))
        .unwrap_or_default()
        .trim()
        .to_owned();
    assert_eq!(blue_row, "blue: 1 (BA): Bo", "{board}");

    // Ada takes her tank back; every unheld designator answers one sentence.
    wait_until(STEP_TIMEOUT, "the tank to come back", async || {
        let mark = ada.mark();
        ada.send_line("BOARD").await;
        ada.expect_one_of_after(
            mark,
            &[
                "yours again where it stands",
                "still being handed over",
                COMMIT_PENDING,
                CONTROL_UNKNOWN,
            ],
        )
        .await
            == 0
    })
    .await;
    // Everything the reclaim says — the pod, the catch-up — read to the end
    // first, so each refusal below is read alone.
    ada.expect("Back aboard").await;
    say(&mut ada, SENTINEL, HUH).await;
    let mut refusals: Vec<String> = Vec::new();
    for line in ["LOCK BA", "LOCK X1", "LOCK ZZ", "RANGE BA", "BEARING BA"] {
        let seen = ask(&mut ada, line, NO_SUCH_CONTACT).await;
        refusals.push(seen.trim_start_matches(line).trim().to_owned());
    }
    for answer in &refusals {
        assert_eq!(answer, NO_SUCH_CONTACT, "{refusals:?}");
    }

    // Staff read the resume; a pilot is refused as before.
    say(&mut god, "@arena/status", "resumed: from a snapshot at").await;
    let refused_after = ask(&mut ada, "@arena/status", "Permission denied.").await;
    let refusal = |seen: &str| {
        seen.rsplit("@arena/status")
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    assert_eq!(
        refusal(&refused_after),
        refusal(&refused_before),
        "a pilot's refusal is the same bytes with the resumed row present"
    );
}

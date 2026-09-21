//! **One human against the machines, over real SSH** (16b cut Q.7; doc 16 §3.8
//! "Lifecycle", §5; 16b §9's cut Q row).
//!
//! The owner's ask, end to end: *one human cannot start a match against AI fill
//! (both sides need a seated pilot)*. With `[tacsim.match].ai_fill` on a lone
//! pilot now claims a slot, starts the match, boards, brings power up and fights
//! a tank nobody is sitting in — and the machine is **sized** to the game the
//! humans turned up for, so alone that is one against one and not a four a side
//! battle with seven machines in it (`crates/mushd/tests/sim_reflex.rs` proves
//! the sizing; this proves the solo lane).
//!
//! # What it proves that nothing else can
//!
//! `crates/tacsim/tests/match_lifecycle.rs` walks the lobby over the production
//! composition root, and `crates/mushd/tests/sim_reflex.rs` drives the AI fill
//! over SSH with **two** pilots. Neither can show the one thing the owner asked
//! for, because both needed a pilot a side to get a match at all. This boots the
//! production composition with one pilot in the world, and every line below is a
//! line a player types.
//!
//! # The observer rule comes first
//!
//! Doc 16 §1's A1 and §5: *nothing tells a pilot anything about a unit their
//! platform has not detected*. The lone-pilot lane is exactly where a shortcut
//! would be tempting, because the *board* now says a machine is out there. It
//! says how many and nothing else, and the surfaces
//! `crates/mushd/tests/observer_rule.rs` checks say no more here than they do
//! there: `CONTACTS` and `VIEW` never name another unit, a designator nobody is
//! flying is one byte-identical refusal, and no line of the whole session
//! carries a contact id. **Timing is not part of the claim**: a sensor pass runs
//! whether or not the platform has power and the first one falls at `id %
//! sensors.pass_ms`, so "before detection" belongs to the file that controls its
//! own muster and not to this one.
//!
//! # Why staff re-poses the tanks
//!
//! The shipped `arenas/proving_ground.json` musters the two sides a kilometre
//! apart, which is a minute of driving before anything happens. A test with a
//! wall-clock budget parks them in sight of each other first — the same
//! `@spatial/place/domain` route `sim_weapons.rs` and `sim_reflex.rs` use, and
//! the same `opening_pose` rule carries it into the match.
//!
//! # Why the match is aborted rather than fought to a kill
//!
//! Doc 16 §3.5's resolution is a per-mille roll: *whether* a given thirty seconds
//! of shooting kills a stock tank is a draw, and a test that asserted it would be
//! a test that fails on a bad afternoon. So the fight is real and the **result
//! record** is what is asserted: the match ran, the machine engaged, and the
//! centre sealed a record with an outcome in it.

mod support;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_core::module::GameModule;
use mush_core::{Engine, GOD, LIMBO, ObjId, ObjKind};
use mush_llm::{LlmProvider, LlmService, MockProvider};
use mush_session::{SessionConfig, SessionManager};
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use mushd::accounts::{Accounts, hash_password};
use mushd::ipguard::IpGuard;
use mushd::modules;
use mushd::persist::load_or_seed_world;
use mushd::ssh_bridge::{SshBridge, SshListener, start_listener};
use mushd::staff::{StaffQueue, register_server_commands};
use mushd::world_task::{self, PersistHandles, ServerDeps};
use serde_json::Value;

use support::{Term, play_client, scratch_dir, strip_ansi};

const GM_PASSWORD: &str = "gm-secret-1";
const PILOT_PASSWORD: &str = "pilot-secret-1";

/// The world's transient authoritative refusal: a commit is in flight.
const COMMIT_PENDING: &str = "please retry in a moment";
/// `BOARD`'s one transient: the control-lease mirror is a 1 Hz projection.
const CONTROL_UNKNOWN: &str = "control state unknown";
/// How many times a line is re-sent on a transient before the test gives up.
const RETRY_BUDGET: usize = 40;

/// The one pilot character, created before the world task spawns so its id is
/// stable.
const ADA: ObjId = ObjId(2);

/// Where staff parks the two tanks, in centimetres of the arena's own frame.
/// Row 97 is the east-west road both sides muster on since 16b cut F.4, and two
/// hundred metres is inside the stock cannon's band and outside its own hull.
const RED_X_CM: i64 = 100_500;
const BLUE_X_CM: i64 = 120_500;
const ROW_Y_CM: i64 = 97_500;
/// Due east and due west, millidegrees: each tank starts pointed at the other,
/// so the stock thirty degrees a second of turn is not what the test measures.
const EAST_MDEG: u32 = 90_000;
const WEST_MDEG: u32 = 270_000;

// ---------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------

struct Server {
    addr: SocketAddr,
    dir: PathBuf,
    store: Arc<dyn ObjectStore>,
    _listener: SshListener,
    _handles: PersistHandles,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A fast arena with the AI fill on: a short startup, a frequent position
/// summary and a quick reflex cadence, so what the test waits for is the fight
/// and not the clock.
fn tacsim_config() -> tacsim::TacsimConfig {
    let mut cfg = tacsim::TacsimConfig {
        enabled: true,
        ..tacsim::TacsimConfig::default()
    };
    cfg.arena.tempo.quiet_ms = 100;
    cfg.arena.tempo.alert_ms = 100;
    cfg.arena.tempo.engaged_ms = 100;
    cfg.arena.ledger.summary_ms = 200;
    cfg.arena.reflex.decide_ms = 200;
    cfg.arena.reflex.repath_ms = 0;
    cfg.match_.startup_ms = 300;
    cfg.match_.board_timeout_ms = 120_000;
    // The whole point of the file.
    cfg.match_.ai_fill = true;
    // **16b cut D.3 (`B-33`)**: the configured floor, so a match every pilot has
    // left is closed in ten real seconds rather than in the shipped five
    // minutes. It changes nothing for a pilot who stays aboard.
    cfg.match_.unattended_ms = 10_000;
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
    for (id, name) in [(ADA, "Ada")] {
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
    let identity: Arc<dyn mush_core::AccountIdentity> = Arc::new(
        mush_server::accounts_identity::hydrate(&*store)
            .await
            .expect("the account map hydrates"),
    );
    engine.set_account_identity(Arc::clone(&identity));
    if let Some(slot) = engine.world.module_state_mut::<mush_sim::AccountOracle>() {
        *slot = mush_sim::AccountOracle::new(identity);
    }

    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: Duration::from_secs(120),
        ..SessionConfig::default()
    }));
    let guard = Arc::new(IpGuard::new());
    let handles = world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        ServerDeps {
            store: Arc::clone(&store),
            guard: Arc::clone(&guard),
            staff,
            llm: Some(Arc::clone(&llm)),
            quarantine_spool: mushd::quarantine::QuarantineSpool::in_memory(),
            metrics: Arc::new(mush_net_web::Metrics::new()),
            outbox_adapters: Vec::new(),
        },
    );
    let accounts = Arc::new(Accounts::new(Arc::clone(&store), handles.handle.clone(), 6));
    let bridge = Arc::new(SshBridge::new(
        accounts,
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
    Server {
        addr,
        dir,
        store,
        _listener: listener,
        _handles: handles,
    }
}

/// Connect one account and wait for its character to land.
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

/// Put staff and the one pilot in the simulator lobby, and park the two tanks
/// in sight of each other on the muster road.
async fn open_the_centre(god: &mut Term) {
    let listing = ask(god, "@find Simulator Lobby", "END OF LIST").await;
    let lobby = dbref_of(&listing, "Simulator Lobby");
    say(god, &format!("@teleport #{}=#{lobby}", ADA.0), "Teleported").await;
    say(
        god,
        &format!("@teleport #{}=#{lobby}", GOD.0),
        "Simulator Lobby",
    )
    .await;

    let listing = ask(god, "@find Proving Ground", "END OF LIST").await;
    let ground = dbref_of(&listing, "Proving Ground");
    for (name, x_cm, heading) in [
        ("Red Tank 1", RED_X_CM, EAST_MDEG),
        ("Blue Tank 1", BLUE_X_CM, WEST_MDEG),
    ] {
        let listing = ask(god, &format!("@find {name}"), "END OF LIST").await;
        let tank = dbref_of(&listing, name);
        say(
            god,
            &format!(
                "@spatial/place/domain #{tank}=#{ground},{{\"x_cm\":{x_cm},\"y_cm\":{ROW_Y_CM},\
                 \"heading_mdeg\":{heading}}}"
            ),
            "Placed",
        )
        .await;
    }
}

/// Every ledger line one match wrote, parsed.
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
                if let Ok(event) = serde_json::from_str::<Value>(line) {
                    events.push(event);
                }
            }
        }
    }
    events
}

/// Every `"lines"` string in one chunk document, in document order.
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
        Value::Array(items) => items.iter().for_each(|item| collect_lines(item, out)),
        _ => {}
    }
}

fn count(events: &[Value], kind: &str) -> usize {
    events
        .iter()
        .filter(|event| event.get("kind").and_then(Value::as_str) == Some(kind))
        .count()
}

/// How many units the match's position summaries carry.
fn units_in_summary(events: &[Value]) -> usize {
    events
        .iter()
        .filter(|event| event.get("kind").and_then(Value::as_str) == Some("position_summary"))
        .filter_map(|event| Some(event.get("units")?.as_array()?.len()))
        .max()
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_pilot_alone_starts_a_match_and_fights_the_machine() {
    let server = boot("sim-solo").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;

    // ---- the board, before anything is claimed ----
    let seen = ask(&mut ada, "SIM STATUS", "idle").await;
    assert!(seen.contains("Simulator centre"), "{seen}");

    // ---- one claim, and a match ----
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    let seen = ask(&mut ada, "SIM STATUS", "machines take the unclaimed pods").await;
    assert!(
        seen.contains("AI fill: each side fields 1; machines take the unclaimed pods."),
        "the board says what the fill is before the match starts: {seen}"
    );
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "red", "Red Room").await;
    say(&mut ada, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;

    // ---- the observer rule ----
    //
    // The machine is out there and the board has said so; what the board has NOT
    // said is where it is or what it is called, and nothing in the cockpit may.
    //
    // **Every claim here holds at any arena millisecond**, deliberately. A
    // sensor pass is not gated on power (`Arena::detections` reads a wreck and a
    // sensor and nothing else) and a unit's first pass falls at `id %
    // sensors.pass_ms`, so "before anything is detected" on two tanks two
    // hundred metres apart is a race against a dbref;
    // `crates/mushd/tests/observer_rule.rs` controls its own timing and makes
    // the cold-platform claim there. What cannot race is a byte-identical
    // refusal for a designator this platform will never hold — blue fields one
    // machine, in its lowest pod, so `BB` is nobody — and the rule that no
    // surface ever names another unit, which the sweep at the end of this test
    // reads over every byte the session saw.
    let seen = ask(&mut ada, "LOCK BB", "No such contact.").await;
    assert!(
        seen.contains("No such contact."),
        "a guessed designator is one byte-identical refusal: {seen}"
    );
    // `CONTACTS` is waited on by its own echo, because whether it answers `No
    // contacts.` or a row is a fact about the sensor stagger; what it may never
    // answer is a name.
    let mark = ada.mark();
    ada.send_line("CONTACTS").await;
    ada.expect_after(mark, "CONTACTS").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let seen = strip_ansi(&ada.seen[mark..]);
    assert!(
        !seen.contains("Blue Tank"),
        "the contact list never names another unit: {seen}"
    );
    let seen = ask(&mut ada, "VIEW", "Arena window").await;
    assert!(
        !seen.contains("Blue Tank"),
        "the window never names another unit: {seen}"
    );

    // ---- power, and the fight ----
    say(&mut ada, "STARTUP", "Startup sequence").await;
    let mut online = false;
    for _ in 0..RETRY_BUDGET {
        let seen = ask(&mut ada, "STATUS", "power").await;
        if seen.contains("power online") {
            online = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(online, "the tank never came online");

    // Ada drives at the machine and shoots whenever the breech is free. The
    // machine is doing the same thing, off its own controller, and neither of
    // them was told anything about the other that its own sensors did not find.
    //
    // The lock is taken under the designator the cockpit prints, and 16b cut
    // Q.1's rename is what makes `LOCK BA` the right line the moment the contact
    // is identified — the arena holds the contact id and not the word, so a lock
    // taken as `X1` survives the tank becoming `BA`.
    //
    // Every line is followed by a wait on its own **echo**, which always comes
    // back whatever the answer is. That is not politeness: the client's SSH
    // session only advances while somebody is awaiting the channel, so a loop
    // that sent thirty lines without reading one would stall the window and hang
    // the test rather than failing it.
    say(&mut ada, "SPEED 10", "Making").await;
    let start = ada.mark();
    let mut ended = false;
    for _ in 0..30 {
        for line in ["LOCK BA", "FIRE"] {
            let mark = ada.mark();
            ada.send_line(line).await;
            ada.expect_after(mark, line).await;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
        if strip_ansi(&ada.seen[start..]).contains("is over") {
            ended = true;
            break;
        }
    }
    let fight = strip_ansi(&ada.seen[start..]);
    assert!(
        fight.contains("identified: BA") || fight.contains("Locked on BA"),
        "the machine was never identified, so there was no fight: {fight}"
    );

    // ---- the recorded result ----
    //
    // One of them may already be a wreck — the resolution is a per-mille roll and
    // a stock tank at two hundred metres does not survive many of them — and a
    // match that has ended cannot be aborted. Either way the centre seals a
    // record with an outcome in it, which is what this asserts.
    //
    // The abort is **sent and not waited on**: a match that ended a
    // millisecond ago refuses it (`may_abort` is false on a terminal centre),
    // and a test that retried that refusal would be waiting out its own budget
    // for a record it already has.
    let _ = ended;
    ada.send_line("SIM ABORT").await;
    tokio::time::sleep(Duration::from_millis(3_000)).await;
    // The record is read from the **store** and not from a terminal: doc 16 §4's
    // result document is what "a recorded result" means, and a second read over
    // SSH would only be a second chance for a sixty-second timeout.
    let key = tacsim::match_::result_key(tacsim::arena_task::MatchId(1));
    let record = server
        .store
        .module_json_get("tacsim", key.as_str())
        .await
        .expect("the store read")
        .unwrap_or_else(|| panic!("no result document at {key}"));
    let outcome = record
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    // Doc 16 §4 fixes the vocabulary at these six words, and 16b cut Q.7 did not
    // widen it: `EndReason::Spent` is carried as `draw` with its reason on the
    // board, so `spent` can never appear here and `void` can.
    assert!(
        ["red", "blue", "draw", "abort", "timeout", "void"].contains(&outcome.as_str()),
        "the centre sealed a record with an outcome in it: {record}"
    );

    // Doc 16 §3.1's egress is ack-gated and chunked, so what is in the store is
    // whatever CLOSED — the tail of a short match is still in the ring when the
    // record is written, which is why the end is asserted on the result document
    // above and the fight on the chunks that did land.
    let mut events = Vec::new();
    for _ in 0..10 {
        events = ledger_events(&server.store, 1).await;
        if count(&events, "fired") > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert_eq!(
        units_in_summary(&events),
        2,
        "one pilot alone is a one against one: {:?}",
        events
            .iter()
            .filter_map(|event| event.get("kind").and_then(Value::as_str))
            .take(20)
            .collect::<Vec<_>>()
    );
    assert!(
        count(&events, "startup") >= 2,
        "the machine started itself up beside her: {} startups",
        count(&events, "startup")
    );
    assert!(
        count(&events, "detected") > 0,
        "the two of them found each other"
    );
    assert!(
        count(&events, "fired") > 0,
        "and somebody pulled a trigger: {:?}",
        events
            .iter()
            .filter_map(|event| event.get("kind").and_then(Value::as_str))
            .collect::<Vec<_>>()
    );

    // ---- the observer rule, over every byte the session saw ----
    //
    // Doc 16 §1's A1 read over the whole transcript rather than at one moment:
    // whatever Ada's platform found and whenever it found it, the machine was
    // never named and its contact id was never printed. This is the claim that
    // does not race the sensor stagger.
    let whole = strip_ansi(&ada.seen);
    assert!(
        !whole.contains("Blue Tank"),
        "a surface named the machine by its object name"
    );
    no_sixteen_digit_word(&whole);
}

/// **Nothing ever printed a bare contact id** (doc 16 §5): sixteen hexadecimal
/// digits in a word of their own, which is what `ContactId`'s `Display` writes
/// and what no player-facing line may carry.
fn no_sixteen_digit_word(seen: &str) {
    for word in seen.split(|c: char| !c.is_ascii_hexdigit()) {
        assert!(
            word.len() != 16 || u64::from_str_radix(word, 16).is_err(),
            "a line carried a sixteen-digit contact id ({word})"
        );
    }
}

// ---------------------------------------------------------------------------
// 16b cut D.3 (`B-33`) — the match a lone pilot walks out of ends itself
// ---------------------------------------------------------------------------

/// **A match every pilot has left ends itself** (16b cut D.3, `B-33`), over the
/// real front door and the real clock.
///
/// This is the lane the row was written for and the only file that can show it:
/// doc 16 §11's N.1 amendment makes `EXIT` one-way during `Running`, so the tank
/// goes to the reflex controller and its pilot cannot take it back — and with
/// the AI fill on and one human in the centre, that leaves a match with **nobody
/// in it who could abort it**. Before this cut it ran to
/// `match.max_duration_ms`, an hour in which no second crew could claim a slot.
///
/// `[tacsim.match].unattended_ms` is this file's own 10 s (the configured floor),
/// so the wait below is real seconds of the server's own 1 Hz lobby tick and not
/// a clock a test moved.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn d3_the_match_a_lone_pilot_walks_out_of_ends_itself() {
    let server = boot("sim-solo-unattended").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;

    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "red", "Red Room").await;
    say(&mut ada, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;

    // She climbs out mid-match, and is told it is final.
    say(&mut ada, "EXIT", "You climb out of").await;
    ada.expect("you cannot take it back").await;

    // Nobody is left in it, so the centre closes it by itself — the announcement
    // reaches the team room she is standing in, like every other centre line.
    let mark = ada.mark();
    ada.expect("Match 1 ends: every pilot has left it.").await;
    ada.expect("Its units are checked in.").await;

    // **The observer rule over the bytes the end really sent** (doc 16 §1, §5).
    // `crates/mushd/tests/observer_rule.rs` sweeps the sentence the production
    // code builds; this is the other half — what a player standing in the centre
    // actually received when the centre closed a match by itself. It names the
    // match and nothing of the field: no tank, no object number, no bearing, no
    // position, no contact id.
    let ending = strip_ansi(&ada.seen[mark..]);
    for leak in ["Blue Tank", "Red Tank", "bearing", " m east", " m north"] {
        assert!(
            !ending.contains(leak),
            "the unattended end leaked {leak:?}:\n{ending}"
        );
    }
    assert!(
        !ending
            .split('#')
            .skip(1)
            .any(|rest| rest.starts_with(|c: char| c.is_ascii_digit())),
        "the unattended end named an object:\n{ending}"
    );
    no_sixteen_digit_word(&ending);

    // The board says which end it was, in the words the reason earns.
    let seen = ask(&mut ada, "SIM STATUS", "blue:").await;
    let status: String = seen.split_whitespace().collect::<Vec<&str>>().join(" ");
    assert!(
        status.contains("ended (match 1) — timeout: every pilot had left"),
        "{status}"
    );

    // And the result document carries the arena's own reason beside the outcome.
    let key = tacsim::match_::result_key(tacsim::arena_task::MatchId(1));
    let record = server
        .store
        .module_json_get("tacsim", key.as_str())
        .await
        .expect("the store read")
        .unwrap_or_else(|| panic!("no result document at {key}"));
    assert_eq!(
        record.get("outcome").and_then(Value::as_str),
        Some("timeout"),
        "{record}"
    );
    assert_eq!(
        record.get("reason").and_then(Value::as_str),
        Some("unattended"),
        "{record}"
    );

    // `SIM TEAM` opens the next one, which is the whole point of ending it.
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
}

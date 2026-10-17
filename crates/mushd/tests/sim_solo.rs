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
//! whether or not the platform has power and the first one falls at
//! `stagger_ms(seed, id, sensors.pass_ms)` (a seeded offset inside the cadence,
//! 16b cut U.1), so "before detection" belongs to the file that controls its
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

use support::{STEP_TIMEOUT, wait_until};

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

use support::{Term, play_client, scratch_dir, strip_ansi, unwrapped};

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
    mods.extend(modules::sheet_modules(onboarding, &Default::default()));
    mods.push(modules::tacsim_module(tacsim_config()));
    mods.push(modules::content_module_with(true, true).expect("the content module composes"));
    mods
}

async fn boot(tag: &str) -> Server {
    boot_on(tag, None).await
}

/// [`boot`] over a store of its own, or — **16b cut AH.1** — over one a
/// previous boot left (`arena_custody.rs`' shape): with `Some(store)` neither
/// account nor Ada is made again, and `load_or_seed_world` loads the world the
/// store holds.
async fn boot_on(tag: &str, store: Option<Arc<dyn ObjectStore>>) -> Server {
    let dir = scratch_dir(tag);
    let fresh = store.is_none();
    let store: Arc<dyn ObjectStore> = match store {
        Some(store) => store,
        None => {
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
            store
        }
    };

    let mut world = load_or_seed_world(&*store).await.expect("boot world");
    // A store a previous boot left already holds Ada and her account.
    let pilots: &[(ObjId, &str)] = if fresh { &[(ADA, "Ada")] } else { &[] };
    for &(id, name) in pilots {
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

/// The nonsense line [`ask_whole`] fences an answer with (16b cut AG.1), and the
/// parser's refusal, which is its answer.
const SENTINEL: &str = "zzq-solo-sentinel";
/// As [`SENTINEL`].
const HUH: &str = "Huh?";

/// **A whole answer, never half of one** (16b cut AG.1; `observer_rule.rs`'s
/// twin — a test binary cannot import another): the line, then [`SENTINEL`],
/// and the slice runs to the sentinel's `Huh?`, so everything the line produced
/// is in it and nothing is still in flight. A world mid-commit answers both
/// lines with the transient, which is a retry.
async fn ask_whole(term: &mut Term, line: &str) -> String {
    for _ in 0..RETRY_BUDGET {
        let mark = term.mark();
        term.send_line(line).await;
        term.send_line(SENTINEL).await;
        if term.expect_one_of_after(mark, &[HUH, COMMIT_PENDING]).await == 0 {
            return strip_ansi(&term.seen[mark..]);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("`{line}` never produced a whole answer")
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
/// in sight of each other on the muster road. Answers the two tanks' dbrefs,
/// red then blue (16b cut I.3: `@find` omits a unit an arena holds, so they are
/// read here, before the match).
async fn open_the_centre(god: &mut Term) -> (u32, u32) {
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
    let mut tanks = Vec::new();
    for (name, x_cm, heading) in [
        ("Red Tank 1", RED_X_CM, EAST_MDEG),
        ("Blue Tank 1", BLUE_X_CM, WEST_MDEG),
    ] {
        let listing = ask(god, &format!("@find {name}"), "END OF LIST").await;
        let tank = dbref_of(&listing, name);
        tanks.push(tank);
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
    match tanks.as_slice() {
        [red, blue] => (*red, *blue),
        _ => panic!("two tanks"),
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
    // sensor and nothing else) and a unit's first pass falls at
    // `stagger_ms(seed, id, sensors.pass_ms)` (16b cut U.1), so "before anything
    // is detected" on two tanks two hundred metres apart is a race against a
    // seeded offset;
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
    // Wait for `CONTACTS` to answer either no contacts or a row, retrying the
    // command's transients within one deadline. Only a row carries `(rel `;
    // an unsolicited acquisition notice's bearing cannot satisfy this wait.
    // Either sensor-stagger outcome must still withhold the other unit's name.
    let mark = ada.mark();
    wait_until(
        STEP_TIMEOUT,
        "the solo contact list to answer",
        async || {
            let response_mark = ada.mark();
            ada.send_line("CONTACTS").await;
            ada.expect_one_of_after(
                response_mark,
                &["No contacts.", "(rel ", COMMIT_PENDING, CONTROL_UNKNOWN],
            )
            .await
                < 2
        },
    )
    .await;
    let seen = strip_ansi(&ada.seen[mark..]);
    assert!(
        !seen.contains("Blue Tank"),
        "the contact list never names another unit: {seen}"
    );
    // Boarding can finish before the first frame reaches the world. A dark
    // panel is a real transient; wait for the whole window within one deadline.
    let mut seen = String::new();
    wait_until(STEP_TIMEOUT, "the first solo window", async || {
        seen = ask_whole(&mut ada, "VIEW").await;
        seen.contains("Arena window")
    })
    .await;
    assert!(
        !seen.contains("Blue Tank"),
        "the window never names another unit: {seen}"
    );
    // **16b cut AG.1**: and the same window in sentences. Only what holds at
    // any arena millisecond is asserted — the brief's shape and order (between
    // the ground and the spots either the one `No contacts in the window.` or
    // contact sentences, never both: either sensor-stagger outcome), no line
    // inside it that opened no sentence, no unit's name, and the off-window
    // sentence before the brief's own `End of brief.` (16b cut AP.4), which
    // reads only her own unpowered pose and the fixed spots:
    // The Ford (64, 97) is 36 cells west of (100, 97) and off every window
    // there. Nothing waits for a contact and no designator is compared across
    // reads: the Q.1 rename and the fight's end both race such a wait. The
    // `VIEW` above was read whole; measure this slice from the brief's own header.
    let seen = ask_whole(&mut ada, "VIEW BRIEF").await;
    let at = seen
        .find("Window around RA:")
        .unwrap_or_else(|| panic!("the brief's header: {seen}"));
    let brief = &seen[at..];
    let sentences = support::brief_sentences(brief);
    support::assert_brief_shape(&sentences, "the solo brief");
    assert_eq!(
        support::brief_strays(brief),
        Vec::<String>::new(),
        "a line inside the brief opened no sentence:\n{brief}"
    );
    assert!(
        !brief.contains("Blue Tank"),
        "the brief never names another unit: {brief}"
    );
    assert_eq!(
        sentences.last().map(String::as_str),
        Some(support::BRIEF_END),
        "the brief closes on its end line: {brief}"
    );
    assert!(
        sentences.iter().rev().nth(1).is_some_and(
            |sentence| sentence.starts_with("Off the window: A The Ford, 360 m at 270°")
        ),
        "the off-window sentence is the brief's last before its end line: {brief}"
    );
    no_sixteen_digit_word(brief);

    // ---- power, and the fight ----
    say(&mut ada, "STARTUP", "Startup sequence").await;
    // Each poll read WHOLE: a needle of `power` is also the panel hint's and the
    // previous poll's, and a poll that returned on either would leave its
    // `STATUS` in flight into whatever is marked next.
    wait_until(STEP_TIMEOUT, "the solo tank to come online", async || {
        let seen = ask_whole(&mut ada, "STATUS").await;
        seen.contains("power online")
    })
    .await;

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
    // The record is read from the **store** and not from a terminal: doc 16 §4's
    // result document is what "a recorded result" means, and a second read over
    // SSH would only be a second chance for a sixty-second timeout.
    let key = tacsim::match_::result_key(tacsim::arena_task::MatchId(1));
    let mut record = None;
    wait_until(
        STEP_TIMEOUT,
        "the solo match result to persist",
        async || {
            record = server
                .store
                .module_json_get("tacsim", key.as_str())
                .await
                .expect("the store read");
            record.is_some()
        },
    )
    .await;
    let record = record.expect("waited for the result");
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
    wait_until(
        STEP_TIMEOUT,
        "the solo match's fired event to persist",
        async || {
            events = ledger_events(&server.store, 1).await;
            count(&events, "fired") > 0
        },
    )
    .await;
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
/// goes to its sealed mind (the reflex controller until 17b cut M4.3) and its
/// pilot cannot take it back — and with
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
    let exit_mark = ada.mark();
    say(&mut ada, "EXIT", "You climb out of").await;
    ada.expect("you cannot take it back").await;
    // **16b cut B.4** (`B-38`): and is then shown the team room she is standing
    // in (its `#id` is hers to read). The window the observer sweep below reads
    // starts after that view, so it holds what the unattended END sent.
    ada.expect_after(exit_mark, "Exits").await;

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

// ---------------------------------------------------------------------------
// 16b cut Y.1 — a lone pilot orders their wingman
// ---------------------------------------------------------------------------

/// **16b cut Y.1**: over the production composition and the real front door, a
/// lone pilot with the AI fill orders the machine beside her.
///
/// `SIM AI 2` fields her and one machine on red (`red-1`, a squad of one) and
/// two machines on blue (`blue-1`). Blue's first tank is parked seventy cells
/// down the road, past the stock sight, so nothing of blue is held while the
/// orders are typed. She leads red, so `ORDER red-1 HOLD` is hers and is
/// acknowledged in the doctrine's words; the black box carries an `ordered`
/// line with her own tank as the issuer; `WITHDRAW HOLD FIRE` is acknowledged
/// too; `ENGAGE BA` before her platform holds `BA` is byte for byte what `LOCK
/// ZZ` answers (here she does own the squad, so it is the contact that is
/// refused); and the other side's squad is `No such squad.`. **16b cut Z.2**:
/// then `SCREEN 90`, `SUPPORT RA` and `RELEASE` are acknowledged verbatim, and
/// `SUPPORT BA` is byte for byte `SUPPORT ZZ`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn y1_a_lone_pilot_orders_their_wingman() {
    let server = boot("sim-solo-order").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;
    // Blue's first tank, seventy cells east of Ada's: outside the stock sensor's
    // sixty at every signature factor, as `observer_rule.rs` parks it.
    let listing = ask(&mut god, "@find Proving Ground", "END OF LIST").await;
    let ground = dbref_of(&listing, "Proving Ground");
    let listing = ask(&mut god, "@find Blue Tank 1", "END OF LIST").await;
    let blue = dbref_of(&listing, "Blue Tank 1");
    say(
        &mut god,
        &format!(
            "@spatial/place/domain #{blue}=#{ground},{{\"x_cm\":{},\"y_cm\":{ROW_Y_CM},\
             \"heading_mdeg\":{WEST_MDEG}}}",
            RED_X_CM + 70_000
        ),
        "Placed",
    )
    .await;
    let listing = ask(&mut god, "@find Red Tank 1", "END OF LIST").await;
    let own = dbref_of(&listing, "Red Tank 1");

    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut ada, "SIM AI 2", "AI fill set to 2 a side.").await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "red", "Red Room").await;
    say(&mut ada, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;
    // **16b cut AH.1**: a fresh account reads the panel hint on its first frame,
    // one pulse after `Good hunting`; fenced here so it cannot land inside the
    // byte-exact slices below.
    ada.expect("Hint: the panel").await;

    // A contact her platform does not hold: `LOCK ZZ`'s refusal, byte for byte.
    let mut answers: Vec<String> = Vec::new();
    for line in ["LOCK ZZ", "ORDER red-1 ENGAGE BA"] {
        let seen = ask(&mut ada, line, "No such contact.").await;
        answers.push(seen.trim_start_matches(line).trim().to_owned());
    }
    assert_eq!(answers[0], "No such contact.");
    assert_eq!(
        answers[1], answers[0],
        "`ORDER red-1 ENGAGE BA` before detection is `LOCK ZZ`'s answer"
    );

    let seen = ask(&mut ada, "ORDER red-1 HOLD", "order to it.").await;
    let seen = unwrapped(&seen);
    assert!(
        seen.contains(
            "Ordered red-1 to hold: it stops where it stands and fires nothing. It stands for \
             10 minutes or until your next order to it."
        ),
        "{seen}"
    );
    let seen = ask(&mut ada, "ORDER red-1 WITHDRAW HOLD FIRE", "order to it.").await;
    let seen = unwrapped(&seen);
    assert!(
        seen.contains(
            "Ordered red-1 to withdraw, holding fire: it breaks for its entry zone and fires \
             nothing. It stands for 10 minutes or until your next order to it."
        ),
        "{seen}"
    );
    let seen = ask(&mut ada, "ORDER blue-1 HOLD", "No such squad.").await;
    assert_eq!(
        seen.trim_start_matches("ORDER blue-1 HOLD").trim(),
        "No such squad."
    );

    // **16b cut Z.2 (16c N6, Q44)**: the screen, the support of her own tank,
    // one byte-identical refusal for the other side's callsign and for a word
    // that is no callsign, and the release — each through the front door.
    let seen = ask(&mut ada, "ORDER red-1 SCREEN 90", "order to it.").await;
    let seen = unwrapped(&seen);
    assert!(
        seen.contains(
            "Ordered red-1 to screen 090°: it holds where it stands, faces 090° and fires on \
             what comes inside its arc without giving chase. It stands for 10 minutes or until \
             your next order to it."
        ),
        "{seen}"
    );
    // 17b cut M5.6 (OQ-6; STOP D, the text made true): `… and fires first on
    // what RA is fighting.` until that cut.
    let seen = ask(&mut ada, "ORDER red-1 SUPPORT RA", "order to it.").await;
    let seen = unwrapped(&seen);
    assert!(
        seen.contains(
            "Ordered red-1 to support RA: it keeps within 150 m of RA and fights what it finds \
             there. It stands for 10 minutes or until your next order to it."
        ),
        "{seen}"
    );
    let mut refusals: Vec<String> = Vec::new();
    for line in ["ORDER red-1 SUPPORT BA", "ORDER red-1 SUPPORT ZZ"] {
        let seen = ask(&mut ada, line, "that callsign.").await;
        refusals.push(seen.trim_start_matches(line).trim().to_owned());
    }
    assert_eq!(refusals[0], "No tank of your side flies that callsign.");
    assert_eq!(
        refusals[1], refusals[0],
        "the other side's callsign and a word that is no callsign are one sentence"
    );

    // The black box: the order that landed, with her own tank as its issuer.
    // What is in the store is what CLOSED, so the wait is on the chunk.
    let mut events = Vec::new();
    wait_until(
        STEP_TIMEOUT,
        "the solo match's ordered line to persist",
        async || {
            events = ledger_events(&server.store, 1).await;
            count(&events, "ordered") > 0
        },
    )
    .await;
    // Which of her two orders it is depends on whether a pulse fell between
    // them: the task's inbox keeps a continuous order latest-wins per unit, and
    // an order that never landed journals nothing.
    let hold = events
        .iter()
        .find(|event| event.get("kind").and_then(Value::as_str) == Some("ordered"))
        .unwrap_or_else(|| panic!("no `ordered` line: {events:?}"));
    assert_eq!(
        hold.get("issuer").and_then(Value::as_u64),
        Some(u64::from(own)),
        "the issuer is her own tank: {hold}"
    );
    assert_eq!(hold.get("squad").and_then(Value::as_str), Some("red-1"));
    assert_eq!(hold.get("ttl_ms").and_then(Value::as_u64), Some(600_000));
    assert_ne!(
        hold.get("unit").and_then(Value::as_u64),
        Some(u64::from(own)),
        "it landed on the machine, not on her tank: {hold}"
    );

    // **16b cut Z.2 (16c Q44)**: the release, typed only once an order is on
    // the black box — a `RELEASE` shares an order's slot in the task's inbox,
    // so typed in the same pulse as the orders above it would be the one word
    // of theirs that landed, and nothing would ever be journalled.
    let seen = ask(&mut ada, "ORDER red-1 RELEASE", "again.").await;
    let seen = unwrapped(&seen);
    // 17b cut M5.3: `to doctrine: it fights on its own reflexes …` until that
    // cut — a release names no mind.
    assert!(
        seen.contains(
            "Released red-1 to its mind: it fights on its own until it is ordered again."
        ),
        "{seen}"
    );
    no_sixteen_digit_word(&strip_ansi(&ada.seen));
}

// ---------------------------------------------------------------------------
// 16b cut I.3 — staff moves the machine (16c N10)
// ---------------------------------------------------------------------------

/// **16b cut I.3: `@arena/place` of the machine's tank, over SSH**, and the tape
/// records it.
///
/// A lone pilot's match; staff move the machine a few cells up the road, as the
/// unit's own seat, the moment the match is running — before the machine can
/// have a round in the air. The world answers `Placing`; the pilot is told
/// nothing; and once the match is over the black box the world wrote carries one
/// `placed` line naming the machine's tank and the cell.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn i3_staff_place_the_machine_and_the_tape_records_it() {
    let server = boot("sim-solo-place").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let (_red, blue) = open_the_centre(&mut god).await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "red", "Red Room").await;
    say(&mut ada, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;
    let ada_mark = ada.mark();

    // The machine stands at cell (120, 97); the first cell up the road the
    // world takes is where it goes.
    let mut placed = None;
    'cells: for (x, y) in [(124_u16, 97_u16), (124, 98), (124, 96), (123, 97)] {
        for _ in 0..RETRY_BUDGET {
            let mark = god.mark();
            god.send_line(&format!("@arena/place #{blue}={x},{y}"))
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
                0 => {
                    placed = Some((x, y));
                    break 'cells;
                }
                3 => tokio::time::sleep(Duration::from_millis(200)).await,
                _ => break,
            }
        }
    }
    let placed = placed.expect("the world took one of the cells");
    // The session wraps the reply at eighty columns, so it is read flat.
    let said = strip_ansi(&god.seen)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        said.contains(&format!(
            "Placing #{blue} at cell ({}, {}): the arena answers at its next pulse and the tape \
             records it.",
            placed.0, placed.1
        )),
        "{said}"
    );
    // Two seconds of ARENA time — read off `@arena/status`, never a wall sleep —
    // so pulses have run since the place was offered and it resolved, then the
    // end, so the tape's tail lands. (A place still buffered at the abort would
    // be `place_refused` with `stopped`, which the assertion below would name.)
    support::wait_arena_seconds(&mut god, 2).await;
    ada.send_line("SIM ABORT").await;
    let mut events = Vec::new();
    wait_until(STEP_TIMEOUT, "the place to reach the tape", async || {
        events = ledger_events(&server.store, 1).await;
        count(&events, "placed") + count(&events, "place_refused") > 0
    })
    .await;
    let line = events
        .iter()
        .find(|event| event.get("kind").and_then(Value::as_str) == Some("placed"))
        .unwrap_or_else(|| {
            panic!(
                "the place was refused: {:?}",
                events.iter().find(
                    |event| event.get("kind").and_then(Value::as_str) == Some("place_refused")
                )
            )
        });
    assert_eq!(
        line.get("unit").and_then(Value::as_u64),
        Some(u64::from(blue))
    );
    assert_eq!(
        (
            line.get("x").and_then(Value::as_u64),
            line.get("y").and_then(Value::as_u64)
        ),
        (Some(u64::from(placed.0)), Some(u64::from(placed.1)))
    );
    assert_eq!(count(&events, "placed"), 1);
    // And the pilot heard nothing of it.
    let heard = strip_ansi(&ada.seen[ada_mark..]);
    for word in ["Placing", "place"] {
        assert!(!heard.contains(word), "the pilot heard `{word}`: {heard}");
    }
}

// ---------------------------------------------------------------------------
// 16b cut AH.1 (16c N18) — the first-time hints (six since 17b cut M5.6), once per account
// ---------------------------------------------------------------------------

/// Connect a **two-character** account and pick `character` off the menu
/// (`front_door.rs`' walk): [`login`] waits only on `Connected as`, which a
/// menu never prints until a name is typed.
async fn login_picking(
    addr: SocketAddr,
    account: &str,
    password: &str,
    character: &str,
) -> (russh::client::Handle<support::AcceptAll>, Term) {
    let (handle, mut term) = play_client(addr).await;
    term.expect("Welcome to the simulator.").await;
    term.send_line(&format!("connect {account} {password}"))
        .await;
    term.expect(&format!("Characters on {account}:")).await;
    term.send_line(character).await;
    term.expect(&format!("Connected as {character}")).await;
    (handle, term)
}

/// Every stored `tacsim:tutorial:` document, read straight from the store (not
/// through `@tutorial`, which answers from RAM).
async fn tutorial_documents(store: &Arc<dyn ObjectStore>) -> Vec<Value> {
    let keys = store.module_keys("tacsim").await.expect("the store read");
    let mut documents = Vec::new();
    for key in keys
        .iter()
        .filter(|key| key.starts_with(tacsim::match_::tutorial::KEY_PREFIX))
    {
        if let Some(document) = store
            .module_json_get("tacsim", key)
            .await
            .expect("the store read")
        {
            documents.push(document);
        }
    }
    documents
}

/// Staff teleport one character into the Simulator Lobby, by name.
async fn teleport_to_lobby(god: &mut Term, name: &str) {
    let listing = ask(god, "@find Simulator Lobby", "END OF LIST").await;
    let lobby = dbref_of(&listing, "Simulator Lobby");
    let listing = ask(god, &format!("@find {name}"), "END OF LIST").await;
    let who = dbref_of(&listing, name);
    say(god, &format!("@teleport #{who}=#{lobby}"), "Teleported").await;
}

/// **16b cut AH.1**: over SSH on the production composition, a first account
/// reads the six hints once, in order, across one short match — the sixth,
/// `mind`'s (17b cut M5.6), as the match ends; an alt made with `@newchar` on
/// the same account reads none; and a reboot over the same store keeps the
/// flags — the alt's `@tutorial off` is the account's.
///
/// Was "the five hints" and `5 of 5` at 16b cut AH.1; six and `5 of 6` before
/// the end, `6 of 6` after it, since `HintPoint::Mind` (17b cut M5.6, the design
/// note's item 2: "the sixth said after the first finished match").
///
/// Every wait on a hint is on a prefix narrower than the test PTY's eighty
/// columns, and the audit is over the unwrapped transcript (`hints_read`),
/// because five of the six sentences wrap. The enemy is parked seventy cells
/// down the road, past the stock sight, as `y1` parks it: a hostile told apart
/// before `STARTUP` would leave the contact point unspent for the whole match.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ah1_a_first_account_reads_six_hints_once_an_alt_none_and_a_restart_keeps_the_flags() {
    let server = boot("sim-solo-hints-a").await;
    // Ada first, so the staff teleport below is a move a live session sees.
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;
    ada.expect("Hint: SIM TEAM").await;
    let listing = ask(&mut god, "@find Proving Ground", "END OF LIST").await;
    let ground = dbref_of(&listing, "Proving Ground");
    let listing = ask(&mut god, "@find Blue Tank 1", "END OF LIST").await;
    let blue = dbref_of(&listing, "Blue Tank 1");
    say(
        &mut god,
        &format!(
            "@spatial/place/domain #{blue}=#{ground},{{\"x_cm\":{},\"y_cm\":{ROW_Y_CM},\
             \"heading_mdeg\":{WEST_MDEG}}}",
            RED_X_CM + 70_000
        ),
        "Placed",
    )
    .await;

    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    let mark = ada.mark();
    say(&mut ada, "SIM START", "is boarding").await;
    ada.expect_after(mark, "Hint: walk into").await;
    say(&mut ada, "red", "Red Room").await;
    say(&mut ada, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;
    ada.expect("Hint: the panel").await;
    let mark = ada.mark();
    say(&mut ada, "STARTUP", "Startup sequence").await;
    ada.expect_after(mark, "Power online.").await;
    ada.expect_after(mark, "Hint: VIEW draws").await;

    // Drive at the machine and lock and fire, as the main test does, until the
    // platform tells it apart. Every line waits on its own echo, which keeps
    // the channel read.
    say(&mut ada, "SPEED 54", "Making").await;
    let start = ada.mark();
    for _ in 0..60 {
        for line in ["LOCK BA", "FIRE"] {
            let mark = ada.mark();
            ada.send_line(line).await;
            ada.expect_after(mark, line).await;
        }
        let fight = strip_ansi(&ada.seen[start..]);
        if fight.contains("identified: BA") || fight.contains("Locked on BA") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    let fight = strip_ansi(&ada.seen[start..]);
    assert!(
        fight.contains("identified: BA") || fight.contains("Locked on BA"),
        "the machine was never identified: {fight}"
    );
    ada.expect("Hint: LOCK").await;
    let seen = ask_whole(&mut ada, "@tutorial").await;
    assert!(
        seen.contains("Hints are on; you have read 5 of 7."),
        "{seen}"
    );

    // The end, as the main test ends it: sent unwaited, then the result
    // document, so the centre is terminal before the alt walks in (a stopping
    // centre would gate the lobby point and make the alt's silence vacuous).
    let end = ada.mark();
    ada.send_line("SIM ABORT").await;
    // 17b cut M5.6: the sixth, said by the check-in in the result's own action.
    ada.expect_after(end, "Hint: SIM MIND").await;
    let key = tacsim::match_::result_key(tacsim::arena_task::MatchId(1));
    wait_until(
        STEP_TIMEOUT,
        "the hints match's result to persist",
        async || {
            server
                .store
                .module_json_get("tacsim", key.as_str())
                .await
                .expect("the store read")
                .is_some()
        },
    )
    .await;
    let read = support::hints_read(&ada.seen);
    assert_eq!(
        read,
        tacsim::match_::tutorial::HINT_LINES
            .iter()
            .map(|line| (*line).to_owned())
            .collect::<Vec<_>>(),
        "the six, once each, in order"
    );
    for line in &read {
        assert!(!line.contains("Blue Tank"), "{line}");
    }
    no_sixteen_digit_word(&read.join("\n"));

    // Each hint's OWN mark reaches the store, before any `@tutorial` switch
    // rewrites a whole document from RAM: Ada's six were written by the
    // lobby observer, `/start`, `on_frames` (panel, power, contact) and the
    // check-in (mind, 17b cut M5.6), and
    // God's lobby-only document by the lobby observer alone. The restart leg
    // below cannot prove this — the alt's switch rewrites Ada's document.
    let every_word = tacsim::match_::tutorial::HintPoint::ALL
        .iter()
        .map(|point| Value::from(point.as_str()))
        .collect::<Vec<_>>();
    let lobby_only = vec![Value::from(
        tacsim::match_::tutorial::HintPoint::Lobby.as_str(),
    )];
    wait_until(STEP_TIMEOUT, "every hint's own mark to land", async || {
        let documents = tutorial_documents(&server.store).await;
        let holds = |shown: &[Value]| {
            documents.iter().any(|document| {
                document.get("off") == Some(&Value::Bool(false))
                    && document
                        .get("shown")
                        .and_then(Value::as_array)
                        .map(Vec::as_slice)
                        == Some(shown)
            })
        };
        holds(&every_word) && holds(&lobby_only)
    })
    .await;

    // ---- the alt: the same person, who has read them all ----
    say(&mut ada, "@newchar Ava", "created Ava").await;
    let (_v, mut ava) = login_picking(server.addr, "ada", PILOT_PASSWORD, "Ava").await;
    teleport_to_lobby(&mut god, "Ava").await;
    let seen = ask_whole(&mut ava, "@tutorial").await;
    assert!(
        seen.contains("Hints are on; you have read 6 of 7."),
        "{seen}"
    );
    assert!(
        !strip_ansi(&ava.seen).contains("Hint:"),
        "an alt read a hint: {}",
        strip_ansi(&ava.seen)
    );
    say(&mut ava, "@tutorial off", "stay unsaid").await;
    // The switch is the ACCOUNT's document; waited for by content, because
    // God read the lobby hint in `open_the_centre` and holds one of his own.
    let store = Arc::clone(&server.store);
    wait_until(STEP_TIMEOUT, "the tutorial record to land", async || {
        tutorial_documents(&store)
            .await
            .iter()
            .any(|document| document.get("off") == Some(&Value::Bool(true)))
    })
    .await;
    drop(ava);
    drop(ada);
    drop(god);
    drop(server);

    // ---- the restart: the flags are the store's ----
    let server = boot_on("sim-solo-hints-b", Some(store)).await;
    let (_a2, mut ada) = login_picking(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let seen = ask_whole(&mut ada, "@tutorial").await;
    assert!(
        seen.contains("Hints are off; you have read 6 of 7."),
        "{seen}"
    );
    let (_g2, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    teleport_to_lobby(&mut god, "Ada").await;
    let seen = ask_whole(&mut ada, "@tutorial").await;
    assert!(seen.contains("Hints are off"), "{seen}");
    assert!(
        !strip_ansi(&ada.seen).contains("Hint:"),
        "a hint after the restart: {}",
        strip_ansi(&ada.seen)
    );
}

// ---------------------------------------------------------------------------
// 17b cut M5.7 — M5's gate transcript: a pilot seals a mind, climbs out, and
// the tank is flown by it (the M5 design note's §4.1)
// ---------------------------------------------------------------------------

/// [`login`] on a PTY of 120 columns by 40 — the floor a client is built to,
/// `sim_weapons.rs`' `login_wide` — so every line a pilot reads (each under
/// 120 columns, the M5 conventions' §3) arrives on one line of its own.
async fn login_wide(
    addr: SocketAddr,
    account: &str,
    password: &str,
    character: &str,
) -> (russh::client::Handle<support::AcceptAll>, Term) {
    let (handle, mut term) = support::play_client_sized(addr, 120, 40).await;
    term.expect("Welcome to the simulator.").await;
    term.send_line(&format!("connect {account} {password}"))
        .await;
    term.expect(&format!("Connected as {character}")).await;
    (handle, term)
}

/// The lines of one [`ask_whole`] answer: everything up to the sentinel's
/// `Huh?`, without the two echoes (the line's and the sentinel's, which the
/// session echoes as they are typed, before either is answered), each
/// trimmed of its line end, the blank ones dropped.
fn m5_answer_lines(whole: &str, line: &str) -> Vec<String> {
    let body = whole.rsplit_once(HUH).map_or(whole, |(before, _)| before);
    body.lines()
        .map(str::trim_end)
        .filter(|said| {
            let said = said.trim();
            !said.is_empty() && said != line && said != SENTINEL
        })
        .map(str::to_owned)
        .collect()
}

/// The ends a `SIM REPORT` closes on (`observer_rule.rs`' `REPORT_ENDS`; a
/// test binary cannot import another).
const M5_REPORT_ENDS: [&str; 3] = [
    "the match was aborted.",
    "the match ended: ",
    "the match was voided.",
];

/// **`SIM REPORT`, read whole** (`observer_rule.rs`' `sim_report`): asked
/// again while the result is not yet the centre's to read (`still being
/// written`, a read in flight, or no remembered match yet), then fenced by
/// the sentinel once its end line has arrived.
async fn m5_sim_report(term: &mut Term) -> String {
    const AGAIN: [&str; 5] = [
        "still being written",
        "already being read",
        "You have not flown a match the centre remembers.",
        "is still running.",
        COMMIT_PENDING,
    ];
    for _ in 0..RETRY_BUDGET {
        let mark = term.mark();
        term.send_line("SIM REPORT").await;
        let mut needles: Vec<&str> = M5_REPORT_ENDS.to_vec();
        needles.extend(AGAIN);
        if term.expect_one_of_after(mark, &needles).await < M5_REPORT_ENDS.len() {
            term.send_line(SENTINEL).await;
            term.expect_after(mark, HUH).await;
            return strip_ansi(&term.seen[mark..]);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!(
        "`SIM REPORT` was never read whole:\n{}",
        strip_ansi(&term.seen)
    )
}

/// **The arena's own clock**, in whole seconds, as staff's `@arena/status`
/// prints it (`support::arena_clock_seconds`; the bridge mirror's `now_ms`,
/// the clock a report's span is stamped on).
async fn m5_arena_clock(god: &mut Term) -> u64 {
    let mut clock = None;
    wait_until(STEP_TIMEOUT, "the arena clock to read", async || {
        let mark = god.mark();
        god.send_line("@arena/status").await;
        if god
            .expect_one_of_after(
                mark,
                &["ledger writes awaiting", COMMIT_PENDING, CONTROL_UNKNOWN],
            )
            .await
            != 0
        {
            return false;
        }
        clock = support::arena_clock_seconds(&strip_ansi(&god.seen[mark..]));
        clock.is_some()
    })
    .await;
    clock.expect("waited for the clock")
}

/// **A preset's card digest, as the seal takes it**: SHA-256 over
/// `serde_json::to_vec` of the preset over THIS rig's own arena doctrine
/// (`match_::start` builds each profile's card with `card::preset(name,
/// &arena_cfg.reflex)`), in the tape's sixty-four lower-case hex digits.
fn m5_card_digest(preset: &str) -> String {
    use sha2::{Digest as _, Sha256};
    let doctrine = tacsim_config().arena.reflex;
    let card = tacsim::arena::card::preset(preset, &doctrine)
        .unwrap_or_else(|| panic!("{preset} is one of the six presets"));
    let bytes = serde_json::to_vec(&card).expect("a card serialises");
    Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// One unit's tape lines of one kind, in tape order.
fn m5_lines_of<'a>(events: &'a [Value], kind: &str, unit: u32) -> Vec<&'a Value> {
    events
        .iter()
        .filter(|event| event.get("kind").and_then(Value::as_str) == Some(kind))
        .filter(|event| event.get("unit").and_then(Value::as_u64) == Some(u64::from(unit)))
        .collect()
}

/// **17b cut M5.7: M5's gate, in line mode on the local rig** (the M5 design
/// note's §4.1; doc 17 §22's first gate item): over SSH on the production
/// composition with the AI fill — the lone pilot against the machine, the
/// field Q-UM-15 names as unaffected — a pilot chooses a mind and two of its
/// profiles in the lobby, the match's start seals them, `MIND` reads them in
/// the seat, a profile she did not seal is refused, she climbs out of a COLD
/// tank, and the mind she chose flies it: it starts the tank itself, and her
/// report says from when it flew.
///
/// The eleven steps, each asserted on her terminal: `SIM TEAM red`; the bare
/// `SIM MIND` (her default, the `Fits` line and the pool line); `SIM MIND
/// Sentinel` refused as not held; `SIM MIND Standard PROFILE Sniper, Line`;
/// her board row naming it; `SIM START` with no `Humans on the other side`
/// line (Doctrine is never substituted); `BOARD`'s climb, the seal line and
/// `Good hunting`, in that order; `MIND` in its four lines (at most seven),
/// each under 120 columns and none wrapped, naming Sniper and `dormant while
/// you fly`; `MIND PROFILE Hunter` refused in one line; `EXIT` two arena
/// seconds in, with no `STARTUP` ever typed, her `STATUS` reading `power
/// off` just before it and no `Power online.` told her while she sat, staff
/// reading the arena clock on either side of it; and, after the rig's own
/// ten-second unattended end, `SIM REPORT`'s `Your mind` block — the mind as
/// sealed, and the span from her exit's second to the end.
///
/// Then the tape the world wrote (`log:match.1`): her hull's `mind_sealed`
/// carries the Sniper preset's card digest over the rig's own doctrine (the
/// sealed card is the chosen one; the Sniper and Line cards asserted to
/// differ) and the fill's carries Line's; her hull's one `startup` and one
/// `power_online` stand at or after the start of her exit's arena second (the
/// report's `m:ss`, floored: the tape keeps no release line, so the order is
/// read to the second). With the cold panel just before the `EXIT` and no
/// `Power online.` in the seat, the hull was cold when she left, so only its
/// mind started it: **the tank flown by it**. No line Ada read carries a
/// sixteen-digit word, and the run
/// is under the M1b transcript's 150 s (measured under 30 s in debug, so it is
/// not `#[ignore]`d). `help minds` is not typed: the solo rig serves no help
/// topics; `match_::cockpit`'s `m5_help_minds_opens_with_three_true_lines`
/// pins the page.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn m5_a_pilot_seals_a_mind_climbs_out_and_the_tank_is_flown_by_it() {
    const SEALED: &str =
        "Sealed for this sortie: Standard Doctrine (Sniper, Line), Class I; MIND reads it.";
    const GOOD_HUNTING: &str = "Match 1 is running. Good hunting.";
    let began = std::time::Instant::now();
    let server = boot("sim-solo-m5").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    // The tanks stay where the shipped ground musters them, out of every
    // sensor's reach of each other, so nothing finds her hull before the
    // end; their dbrefs are read before the match (16b cut I.3: `@find`
    // omits a unit an arena holds).
    let mut tanks = Vec::new();
    for name in ["Red Tank 1", "Blue Tank 1"] {
        let listing = ask(&mut god, &format!("@find {name}"), "END OF LIST").await;
        tanks.push(dbref_of(&listing, name));
    }
    let (red, blue) = match tanks.as_slice() {
        [red, blue] => (*red, *blue),
        _ => panic!("two tanks"),
    };
    teleport_to_lobby(&mut god, "Ada").await;
    let (_a, mut ada) = login_wide(server.addr, "ada", PILOT_PASSWORD, "Ada").await;

    // (1) The claim.
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;

    // (2) The bare read: her default, what fits her slot, and the pool.
    let read = m5_answer_lines(&ask_whole(&mut ada, "SIM MIND").await, "SIM MIND");
    let said = |lines: &[String], line: &str| lines.iter().any(|seen| seen.trim() == line);
    assert!(
        said(
            &read,
            "Yours: none; a tank you leave flies Standard Doctrine on Line."
        ),
        "{read:#?}"
    );
    assert!(
        said(
            &read,
            "Fits line_tank (II): Standard Doctrine, up to two of Line, Sentry, Sniper, \
             Skirmisher, Hunter or Rearguard."
        ),
        "{read:#?}"
    );
    assert!(
        read.iter().any(|line| {
            let line = line.trim();
            line.starts_with("Red's half of the centre's pool is ")
                && line.ends_with(" are spoken for.")
        }),
        "the pool line: {read:#?}"
    );

    // (3) A mind she does not hold.
    let refused = m5_answer_lines(
        &ask_whole(&mut ada, "SIM MIND Sentinel").await,
        "SIM MIND Sentinel",
    );
    assert!(
        said(
            &refused,
            "You hold no mind called Sentinel; SIM MIND lists the minds you hold."
        ),
        "{refused:#?}"
    );

    // (4) The choice: Standard Doctrine, Sniper first.
    let chose = m5_answer_lines(
        &ask_whole(&mut ada, "SIM MIND Standard PROFILE Sniper, Line").await,
        "SIM MIND Standard PROFILE Sniper, Line",
    );
    assert!(
        said(
            &chose,
            "Your slot will carry Standard Doctrine (Sniper, Line)."
        ),
        "{chose:#?}"
    );

    // (5) Her own row names it to her.
    let board = m5_answer_lines(&ask_whole(&mut ada, "SIM STATUS").await, "SIM STATUS");
    assert!(
        board.iter().any(|line| {
            line.trim_start().starts_with("red: ")
                && line.ends_with(", mind Standard Doctrine (Sniper, Line)")
        }),
        "her row ends with her mind: {board:#?}"
    );

    // (6) The start: Doctrine is never substituted, so no `Humans on the
    // other side` (asserted over the whole transcript at the end too).
    let start = ada.mark();
    say(&mut ada, "SIM START", "is boarding").await;

    // (7) The climb, the seal line and `Good hunting`, in that order.
    say(&mut ada, "red", "Red Room").await;
    let board_mark = ada.mark();
    say(&mut ada, "BOARD", "climb into").await;
    ada.expect_after(board_mark, GOOD_HUNTING).await;
    let boarded = strip_ansi(&ada.seen[board_mark..]);
    let at = |needle: &str| {
        boarded
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} was not said:\n{boarded}"))
    };
    let (climbed, sealed, hunting) = (
        at("You climb into Red Pod 1."),
        at(SEALED),
        at(GOOD_HUNTING),
    );
    assert!(
        climbed < sealed && sealed < hunting,
        "the climb, the seal line, then `Good hunting`:\n{boarded}"
    );
    // A fresh account reads the panel hint on its first frame, one pulse
    // after `Good hunting` (16b cut AH.1); fenced here so it cannot land
    // inside the block read below — and once it is said, a frame is held.
    ada.expect_after(board_mark, "Hint: the panel").await;

    // (8) `MIND`: at most seven lines, each under 120 columns, naming Sniper
    // and dormant while she flies. The session wraps every line at her 120
    // columns, so a line past them would arrive as two: the block's shape is
    // pinned whole — exactly its four lines, by their heads — so a wrapped
    // continuation fails it, and the width check reads whole lines (the
    // review of 17b cut M5.7).
    let block = m5_answer_lines(&ask_whole(&mut ada, "MIND").await, "MIND");
    let heads: Vec<&str> = block
        .iter()
        .map(|line| line.split_once(':').map_or("", |(head, _)| head))
        .collect();
    assert_eq!(
        heads,
        ["Mind", "Profile", "Channels", "Rules"],
        "MIND reads in its four lines (at most seven), none wrapped: {block:#?}"
    );
    for line in &block {
        assert!(line.chars().count() < 120, "{line:?}");
    }
    assert!(
        block.iter().any(|line| line.contains("Sniper")),
        "{block:#?}"
    );
    assert!(
        block
            .iter()
            .any(|line| line.contains("dormant while you fly")),
        "{block:#?}"
    );

    // (9) A profile she did not seal, refused in one line.
    let refused = m5_answer_lines(
        &ask_whole(&mut ada, "MIND PROFILE Hunter").await,
        "MIND PROFILE Hunter",
    );
    assert_eq!(
        refused,
        vec!["MIND refused. Hunter is not sealed on this mind; it carries Sniper and Line."],
        "one line"
    );

    // (10) `EXIT`, cold: no `STARTUP` was ever typed. She sits two arena
    // seconds first — read off staff's `@arena/status`, never a wall sleep —
    // so the tape's claim below (nothing started her hull before her exit's
    // second) covers a stretch of the match and is never true by an exit at
    // `0:00`; staff read the clock on either side of the `EXIT`.
    support::wait_arena_seconds(&mut god, 2).await;
    let before_s = m5_arena_clock(&mut god).await;
    assert!(before_s >= 2, "two arena seconds in the seat: {before_s}");
    // Her own panel, read just before she leaves: the hull is still off
    // after two seconds of her seat (the review of 17b cut M5.7: the tape
    // below places the startup only to her exit's second).
    let status = m5_answer_lines(&ask_whole(&mut ada, "STATUS").await, "STATUS");
    assert!(
        status.iter().any(|line| {
            line.trim_start().starts_with("Red Tank 1") && line.contains(": power off, ")
        }),
        "the hull is cold just before the EXIT: {status:#?}"
    );
    let exit_mark = ada.mark();
    say(&mut ada, "EXIT", "You climb out of").await;
    ada.expect_after(exit_mark, "you cannot take it back.")
        .await;
    let after_s = m5_arena_clock(&mut god).await;
    let left = strip_ansi(&ada.seen[exit_mark..]);
    assert!(
        left.lines().any(|line| line.trim()
            == "You climb out of Red Pod 1. Red Tank 1 is on its own for the rest of the \
                match; you cannot take it back."),
        "{left}"
    );
    // Nothing powered her hull while she sat: a startup in the seat is told
    // to its pilot as `Power online.` (the duel above reads it in line mode),
    // and none was, from `Good hunting` to her climb-out line.
    let seated = strip_ansi(&ada.seen[board_mark..]);
    let sat = seated
        .find(GOOD_HUNTING)
        .zip(seated.find("You climb out of Red Pod 1."))
        .and_then(|(from, to)| seated.get(from..to))
        .unwrap_or_else(|| panic!("her seat, from `Good hunting` to her climb-out:\n{seated}"));
    assert!(
        !sat.contains("Power online."),
        "her hull powered up while she sat in it:\n{sat}"
    );

    // (11) The rig's ten-second unattended end, then her report's block.
    ada.expect_after(exit_mark, "Match 1 ends: every pilot has left it.")
        .await;
    ada.expect_after(exit_mark, "Its units are checked in.")
        .await;
    let report = m5_sim_report(&mut ada).await;
    let lines: Vec<&str> = report.lines().map(str::trim_end).collect();
    let head = lines
        .iter()
        .position(|line| line.trim() == "Your mind: Standard Doctrine (Sniper, Line).")
        .unwrap_or_else(|| panic!("no `Your mind` block:\n{report}"));
    let span = lines.get(head + 1).copied().unwrap_or_default().trim();
    let from = span
        .strip_prefix("It flew your tank from ")
        .and_then(|rest| rest.strip_suffix(", when you climbed out, to the end."))
        .unwrap_or_else(|| panic!("the span line under the head: {span:?}\n{report}"));
    let exit_s = from
        .split_once(':')
        .and_then(|(m, s)| Some(m.parse::<u64>().ok()? * 60 + s.parse::<u64>().ok()?))
        .unwrap_or_else(|| panic!("an m:ss stamp: {from:?}"));
    assert!(
        (before_s..=after_s).contains(&exit_s),
        "the span opens at her exit's arena second: {before_s} <= {exit_s} <= {after_s}"
    );
    assert!(
        lines.get(head + 2).is_some_and(|line| {
            let line = line.trim();
            line == "It fired nothing." || line.starts_with("Its shots: ")
        }),
        "the shots line under the span:\n{report}"
    );

    // ---- the tape: the sealed card is the chosen one, and the mind flew ----
    let mut events = Vec::new();
    wait_until(
        STEP_TIMEOUT,
        "her hull's power line to reach the tape",
        async || {
            events = ledger_events(&server.store, 1).await;
            !m5_lines_of(&events, "power_online", red).is_empty()
                && !m5_lines_of(&events, "mind_sealed", blue).is_empty()
        },
    )
    .await;
    // The two cards differ, or the seal checks below could not tell the
    // chosen card from the default (the review of 17b cut M5.7).
    assert_ne!(
        m5_card_digest("Sniper"),
        m5_card_digest("Line"),
        "the Sniper and Line presets are two cards"
    );
    let digest_of = |unit: u32| -> Vec<String> {
        m5_lines_of(&events, "mind_sealed", unit)
            .iter()
            .filter_map(|event| event.get("card_digest").and_then(Value::as_str))
            .map(str::to_owned)
            .collect()
    };
    assert_eq!(
        digest_of(red),
        vec![m5_card_digest("Sniper")],
        "her hull's seal carries the Sniper card over the rig's doctrine"
    );
    assert_eq!(
        digest_of(blue),
        vec![m5_card_digest("Line")],
        "the fill's seal carries Line's"
    );
    let t_ms = |event: &Value| event.get("t_ms").and_then(Value::as_i64).unwrap_or(-1);
    let exit_ms = i64::try_from(exit_s * 1_000).expect("a small clock");
    let startups = m5_lines_of(&events, "startup", red);
    let online = m5_lines_of(&events, "power_online", red);
    assert_eq!(
        (startups.len(), online.len()),
        (1, 1),
        "one startup and one power line for her hull: {startups:?} {online:?}"
    );
    // To the second: `exit_ms` is the start of her exit's arena second (the
    // report's `m:ss`, floored), so this bound alone cannot order a startup
    // inside that second against the `EXIT`; with the two seconds of her
    // seat it rules out a mind awake while she sat (it would have started
    // the hull at its first think), and the panel's `power off` just before
    // the `EXIT` and the absent `Power online.` above close the second.
    assert!(
        startups.iter().all(|event| t_ms(event) >= exit_ms)
            && online.iter().all(|event| t_ms(event) >= exit_ms),
        "nothing started her hull before her exit's second ({exit_ms} ms): \
         {startups:?} {online:?}"
    );
    assert!(
        t_ms(startups[0]) < t_ms(online[0]),
        "the startup, then the power: {startups:?} {online:?}"
    );

    // ---- the whole transcript ----
    let whole = strip_ansi(&ada.seen);
    assert!(
        !strip_ansi(&ada.seen[start..]).contains("Humans on the other side"),
        "Doctrine is never substituted"
    );
    // The script never types `STARTUP` (whose reply this is): a check on the
    // script, not on the hull — the hull's coldness is the panel's `power
    // off` and the absent `Power online.` above.
    assert!(
        !whole.contains("Startup sequence"),
        "she never started the tank herself"
    );
    no_sixteen_digit_word(&whole);
    println!(
        "==== Ada ====\n{whole}\n==== tape ====\nher hull's card_digest {} (Sniper), the \
         fill's {} (Line); her exit at {exit_s} s, her hull's startup at {} ms and power_online \
         at {} ms\n==== end ({} s) ====",
        m5_card_digest("Sniper"),
        m5_card_digest("Line"),
        t_ms(startups[0]),
        t_ms(online[0]),
        began.elapsed().as_secs()
    );
    assert!(
        began.elapsed() < Duration::from_secs(150),
        "the transcript took {:?}",
        began.elapsed()
    );
}

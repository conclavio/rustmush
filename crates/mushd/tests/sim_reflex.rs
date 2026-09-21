//! **The reflex controller's two world-side claims, over real SSH** (16b cut N.1,
//! N1; doc 16 §6 "AI", §3.8 "Disconnects" and its §11 amendment; doc 07 §1.1's
//! fallback controller).
//!
//! `crates/tacsim/tests/reflex.rs` proves the machine itself — every edge of doc
//! 16 §6's `Startup → Patrol → Approach → Engage → Withdraw → Disabled`, the path
//! budget and the cross-tempo property — by driving `Arena::step` directly. What
//! it cannot prove is that the **world half** reaches it: that K.2's released seat
//! really becomes a `Control { Released }` at the arena seam, and that a lobby
//! that fields fewer pilots than pods really hands the empty ones over at
//! check-out. Both of those are wiring, and wiring is what this file drives, on
//! the production composition, with real accounts in real pods.
//!
//! 1. **The fallback controller.** A pilot boards, brings power up, and then their
//!    line drops for good. Past the linkdead grace the seat is released — and the
//!    tank, which had been sitting still, **drives itself home**. That is doc 16
//!    §3.8's amendment: K4's "the tank idles" was standing in for this until the
//!    controller existed.
//! 2. **The lobby's AI fill, and the size 16b cut Q.7 gave it.** With
//!    `[tacsim.match].ai_fill` on, `SIM AI 4` fields all eight pods: the six
//!    nobody claimed arrive at the check-out with no seated pilot, which is what
//!    makes them the controller's, and they start themselves up without anybody
//!    typing `STARTUP`. Cut N.1 did that for every match; cut Q.7 made it the
//!    size somebody asked for, and the **default** is the larger side's claimed
//!    slots — so the third test here is two pilots on the **same** side getting
//!    two machines on the other one.
//!
//! # Why the tank is moved before the match starts
//!
//! `Withdraw`'s goal is the unit's **own entry zone**, and the shipped
//! `arenas/proving_ground.json` musters the red tanks *inside* it (column 6 of a
//! zone sixteen cells deep, on row 97 since 16b cut F.4). A tank that is already
//! home has nowhere to withdraw
//! to and correctly does nothing, which would make test (1) assert a behaviour
//! that is indistinguishable from the old one. So staff puts Red Tank 1 out in the
//! middle of the window first, facing west — the same `@spatial/place/domain`
//! route `sim_weapons.rs` uses to muster two tanks together, and the same
//! `opening_pose` rule carries it into the match.
//!
//! Facing **west** is deliberate too: the stock tank turns at 30 degrees a second,
//! so a tank that had to come about would spend six seconds doing it and this test
//! would be measuring the turn rate. Pointed the right way already, the only thing
//! between "sitting still" and "moving" is the controller.
//!
//! # Why both tests abort the match before they read the ledger
//!
//! Doc 16 §3.1's egress is **ack-gated and chunked**: the arena hands the world
//! half whole 16 KiB chunks as they close, and the world appends them on its own
//! writer lane. A match a few seconds old has written nothing to the store yet, so
//! a mid-match read of `log:match.1` is an empty answer and not a failing one.
//! `SIM ABORT` is what flushes the tail, and both tests below read the record of a
//! match that is over.

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

/// The two pilot characters, created before the world task spawns so their ids
/// are stable.
const ADA: ObjId = ObjId(2);
const BO: ObjId = ObjId(3);

/// Where staff parks Red Tank 1 for test (1): the middle of the window, in
/// centimetres of the arena's own frame, facing west. See the module head.
const PARKED_X_CM: i64 = 100_500;
/// The row it is parked on — Red Tank 1's own, so the ground under it is the
/// ground the pack authored for a tank to stand on. 16b cut F.4 moved that row
/// from 118 to 97, the east-west road.
const PARKED_Y_CM: i64 = 97_500;
/// Due west, millidegrees.
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

/// A fast arena: a short startup so a pilot is flying within a test's patience, a
/// frequent position summary so the ledger is a usable record of where a tank
/// went, and a quick reflex cadence so the controller's first decision is not
/// what the test is waiting for.
fn tacsim_config(ai_fill: bool) -> tacsim::TacsimConfig {
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
    cfg.match_.ai_fill = ai_fill;
    cfg
}

/// The production module list, in production order, with the starter area on.
fn module_list(
    onboarding: &mush_core::OnboardingConfig,
    llm: &Arc<LlmService>,
    ai_fill: bool,
) -> Vec<Box<dyn GameModule>> {
    let mut mods = modules::server_modules();
    mods.push(modules::worldgen_module(Arc::clone(llm)));
    mods.push(modules::npc_module(modules::npc_agent_adapter()));
    mods.extend(modules::ruleset_modules(onboarding, &Default::default()));
    mods.push(modules::tacsim_module(tacsim_config(ai_fill)));
    mods.push(modules::content_module_with(true, true).expect("the content module composes"));
    mods
}

async fn boot(tag: &str, grace: Duration, ai_fill: bool) -> Server {
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
    for (id, name) in [(ADA, "Ada"), (BO, "Bo")] {
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

    let mut engine = Engine::with_modules(world, registry, module_list(&onboarding, &llm, ai_fill))
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

    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: grace,
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

/// Walk both pilots into the simulator lobby and put staff in there with them.
async fn open_the_centre(god: &mut Term) {
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
    say(
        god,
        &format!("@teleport #{}=#{lobby}", GOD.0),
        "Simulator Lobby",
    )
    .await;
}

/// Claim, start, walk and board: the state both tests begin from.
async fn take_off(ada: &mut Term, bo: &mut Term) {
    take_off_sized(ada, bo, None).await;
}

/// The same, with **16b cut Q.7**'s `SIM AI <n>` asked for between the claims and
/// the start — the one point in the lifecycle the fill's size can be set.
async fn take_off_sized(ada: &mut Term, bo: &mut Term, ai: Option<u8>) {
    say(ada, "SIM TEAM red", "takes red slot 1").await;
    say(bo, "SIM TEAM blue", "takes blue slot 1").await;
    if let Some(size) = ai {
        say(ada, &format!("SIM AI {size}"), "AI fill set to").await;
    }
    say(ada, "SIM START", "is boarding").await;
    say(ada, "red", "Red Room").await;
    say(bo, "blue", "Blue Room").await;
    say(ada, "BOARD", "climb into").await;
    say(bo, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;
}

/// Bring power up, waiting for the arena to say it is online.
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

/// One unit's east coordinate through the run, in ledger order, out of the
/// `position_summary` lines.
fn x_track(events: &[Value], unit: u32) -> Vec<i64> {
    events
        .iter()
        .filter(|event| event.get("kind").and_then(Value::as_str) == Some("position_summary"))
        .filter_map(|event| {
            event
                .get("units")?
                .as_array()?
                .iter()
                .find(|row| {
                    row.as_array()
                        .and_then(|row| row.first())
                        .and_then(Value::as_u64)
                        == Some(u64::from(unit))
                })?
                .as_array()?
                .get(1)?
                .as_i64()
        })
        .collect()
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

fn count(events: &[Value], kind: &str) -> usize {
    events
        .iter()
        .filter(|event| event.get("kind").and_then(Value::as_str) == Some(kind))
        .count()
}

// ---------------------------------------------------------------------------
// (1) The fallback controller
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn past_the_grace_the_reflex_controller_drives_the_tank_home() {
    let server = boot("sim-reflex-grace", Duration::from_secs(2), false).await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;

    // Park Red Tank 1 out in the middle of the window, pointed west — see the
    // module head for both halves of why.
    let listing = ask(&mut god, "@find Proving Ground", "END OF LIST").await;
    let ground = dbref_of(&listing, "Proving Ground");
    let listing = ask(&mut god, "@find Red Tank 1", "END OF LIST").await;
    let red = dbref_of(&listing, "Red Tank 1");
    say(
        &mut god,
        &format!(
            "@spatial/place/domain #{red}=#{ground},{{\"x_cm\":{PARKED_X_CM},\"y_cm\":{PARKED_Y_CM},\"heading_mdeg\":{WEST_MDEG}}}"
        ),
        "Placed",
    )
    .await;

    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (ada_handle, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    take_off(&mut ada, &mut bo).await;
    power_up(&mut ada).await;
    // Ada gives no speed order at all: the tank is online, pointed west, and
    // standing still. Whatever moves it after this is not a pilot.
    tokio::time::sleep(Duration::from_millis(600)).await;

    // The line goes, for good.
    drop(ada);
    drop(ada_handle);
    // The grace is two seconds and the reaper sleeps exactly that; four is the
    // margin, and six more is the controller's ground.
    tokio::time::sleep(Duration::from_secs(10)).await;
    // The match ran on without her, and only then is the black box readable.
    let seen = ask(&mut god, "@arena/status", "match 1").await;
    assert!(
        seen.contains("running"),
        "the match runs on without her: {seen}"
    );
    say(&mut bo, "SIM ABORT", "aborts match").await;
    tokio::time::sleep(Duration::from_millis(1_000)).await;

    let events = ledger_events(&server.store, 1).await;
    let track = x_track(&events, red);
    assert!(
        track.len() >= 4,
        "the ledger should carry the tank's whole run: {track:?}"
    );
    let parked = *track.first().unwrap_or(&PARKED_X_CM);
    let ended = *track.last().unwrap_or(&parked);
    assert!(
        ended < parked - 300,
        "past the grace the tank should have driven itself west toward the red entry \
         zone; it was parked at {parked} cm and ended at {ended} cm ({} samples)",
        track.len()
    );
    // It stood still while the seat was hers: the opening samples are all one
    // place, which is what makes the movement afterwards the controller's and not
    // a dead man's order coasting out.
    assert!(
        track.iter().take(3).all(|x| *x == parked),
        "the tank moved before the line dropped: {track:?}"
    );
}

// ---------------------------------------------------------------------------
// (2) The lobby's AI fill
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_ai_fill_on_sim_ai_four_fields_all_eight_pods() {
    let server = boot("sim-reflex-fill", Duration::from_secs(120), true).await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    // **16b cut Q.7**: the fill is **sized** now. Cut N.1 fielded a side's whole
    // four pods whatever the humans had asked for, which handed a solo player a
    // four a side battle with seven machines in it; the default is the larger
    // side's claimed slots, so these two pilots would get a one against one.
    // `SIM AI 4` is how somebody asks for what N.1 always did.
    take_off_sized(&mut ada, &mut bo, Some(4)).await;

    // The six pods nobody claimed went over with the two that were claimed, and
    // the check-out made them the controller's because they arrived with no seated
    // pilot. Nobody typed `STARTUP` for them.
    tokio::time::sleep(Duration::from_secs(3)).await;
    // And the board says so, in the one line the slots cannot.
    let seen = ask(&mut ada, "SIM STATUS", "machines take the unclaimed pods").await;
    assert!(seen.contains("running"), "{seen}");
    assert!(
        seen.contains("AI fill: each side fields 4; machines take the unclaimed pods."),
        "the board says what the fill is: {seen}"
    );
    say(&mut ada, "SIM ABORT", "aborts match").await;
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    let events = ledger_events(&server.store, 1).await;
    assert_eq!(
        units_in_summary(&events),
        8,
        "`SIM AI 4` should field all eight pods with `ai_fill` on: {:?}",
        events
            .iter()
            .filter_map(|event| event.get("kind").and_then(Value::as_str))
            .take(20)
            .collect::<Vec<_>>()
    );
    assert!(
        count(&events, "startup") >= 6,
        "the six machines should have started themselves up: {} startups",
        count(&events, "startup")
    );
}

/// **16b cut Q.7**: and with nobody asking for a size, the fill is the larger
/// side's claimed slots — two friends on red get **two machines on blue**.
///
/// Both pilots claim the **same** side deliberately. One a side would field no
/// machine at all (each side already has its one), so a test written that way
/// would pass with the fill switched off and prove nothing about the sizing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_ai_fill_on_the_default_size_is_the_larger_side() {
    let server = boot("sim-reflex-sized", Duration::from_secs(120), true).await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut bo, "SIM TEAM red", "takes red slot 2").await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "red", "Red Room").await;
    // **Both pilots walk into the SAME room here**, which is what makes this the
    // one walk in the tree whose needle has to be the room's own description and
    // not its name. Bo is standing in the lobby watching Ada leave, so the line
    // `Ada goes Red Room <R>.` lands in Bo's terminal — and `say` would take that
    // for Bo's own arrival, return without retrying the transient `COMMIT_PENDING`
    // Bo's walk can be answered with, and leave Bo in the lobby for `BOARD` to
    // fail on sixty seconds later. The description cannot be said by anybody
    // else's movement.
    say(&mut bo, "red", "Four pods in a row under a red stripe").await;
    say(&mut ada, "BOARD", "climb into").await;
    say(&mut bo, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;

    tokio::time::sleep(Duration::from_secs(3)).await;
    let seen = ask(&mut ada, "SIM STATUS", "machines take the unclaimed pods").await;
    assert!(
        seen.contains("AI fill: each side fields 2; machines take the unclaimed pods."),
        "two on red is a two a side: {seen}"
    );
    say(&mut ada, "SIM ABORT", "aborts match").await;
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    let events = ledger_events(&server.store, 1).await;
    assert_eq!(
        units_in_summary(&events),
        4,
        "red's two claimed pods and blue's two machines: {:?}",
        events
            .iter()
            .filter_map(|event| event.get("kind").and_then(Value::as_str))
            .take(20)
            .collect::<Vec<_>>()
    );
    assert!(
        count(&events, "startup") >= 2,
        "the two machines should have started themselves up: {} startups",
        count(&events, "startup")
    );
}

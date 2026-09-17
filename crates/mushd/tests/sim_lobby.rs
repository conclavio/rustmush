//! **The lobby lens** (16b cut K.1; 16b §8 cut K, K1; doc 16 §3.7 "Pods", §3.8
//! "Lifecycle").
//!
//! Everything cut K.1 claims about the *lobby* is a claim about real players on
//! real accounts in real rooms: that a slot belongs to an account and not to a
//! character, that the pods lock to the pilots who hold the slots, that a
//! boarding nobody answers times out and aborts, that `SIM` means nothing
//! outside the three rooms C.1 authored, and that a staff reset is idempotent.
//! None of that can be shown by calling a function, so this drives the whole
//! stack over **real SSH**, the way `arena_custody.rs` and `onboarding_e2e.rs`
//! do.
//!
//! # What this lens does not reach, and why
//!
//! The `Boarding → Running` **trigger** needs a pilot in a pod, and the verb
//! that puts one there — `BOARD` — is cut K.2's. Nothing in a shipped build can
//! report a seat yet, and inventing a staff verb to do it would be a second lane
//! K.2 would have to retire. So the trigger, the check-out and the check-in are
//! proved in `crates/tacsim/tests/match_lifecycle.rs`, which calls
//! [`tacsim::match_::slot_seated`] the way cut K.2 will, over the same
//! production composition root.
//!
//! What this file proves over SSH is the half a cockpit is not needed for:
//! `Idle → Selection → Boarding → Aborted → Idle`, twice in a row; the account
//! rules; the boarding deadline; the gate; and — through T.3's staff shortcut,
//! which now runs through this cut's machine — a `Running` match the centre
//! knows about, aborted from the lobby and reset by staff.

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

use support::{Term, play_client, scratch_dir, strip_ansi};

const GM_PASSWORD: &str = "gm-secret-1";
const PILOT_PASSWORD: &str = "pilot-secret-1";

/// The world's transient authoritative refusal: a commit is in flight.
const COMMIT_PENDING: &str = "please retry in a moment";
/// How many times a line is re-sent on that refusal before the test gives up.
const RETRY_BUDGET: usize = 8;

/// The three pilot characters, created before the world task spawns so their
/// ids are stable.
const ADA: ObjId = ObjId(2);
const BO: ObjId = ObjId(3);
const CY: ObjId = ObjId(4);

/// The boarding window this lens runs at — doc 16 ships two minutes.
const BOARD_MS: u32 = 3_000;

/// The one line the gate refuses `SIM` with outside the centre.
const OUTSIDE: &str = "The simulator is through the lobby.";

// ---------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------

struct Server {
    addr: SocketAddr,
    dir: PathBuf,
    _listener: SshListener,
    _handles: PersistHandles,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A fast tacsim layer with a three-second boarding window.
fn tacsim_config() -> tacsim::TacsimConfig {
    let mut cfg = tacsim::TacsimConfig {
        enabled: true,
        ..tacsim::TacsimConfig::default()
    };
    cfg.arena.tempo.quiet_ms = 100;
    cfg.arena.tempo.alert_ms = 100;
    cfg.arena.tempo.engaged_ms = 100;
    cfg.arena.ledger.summary_ms = 100;
    cfg.match_.startup_ms = 200;
    cfg.match_.board_timeout_ms = BOARD_MS;
    cfg
}

/// The production module list, in production order, with the **starter area**
/// on: the simulator centre is three rooms and nine items of the atlas-owned OOC
/// pack (C.1), and without them there is no lobby to stand in.
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
    // Three pilots on three accounts. Created before the task spawns, which is
    // what fixes their ids and what lets the boot-hydrated account oracle see
    // them: doc 10 R3's map is built once, at boot, from the store.
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
    // The post-composition upgrade `mush_server::boot` performs: the
    // character → account oracle, without which every claim fails closed.
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

/// Run one line and wait for `needle`, retrying the transient authoritative
/// refusal. Every needle is chosen so it cannot appear in the line's own echo.
async fn say(term: &mut Term, line: &str, needle: &str) {
    assert!(
        !line.contains(needle),
        "`{needle}` appears in `{line}`'s own echo; pick a needle from the ANSWER"
    );
    for _ in 0..RETRY_BUDGET {
        let mark = term.mark();
        term.send_line(line).await;
        if term
            .expect_one_of_after(mark, &[needle, COMMIT_PENDING])
            .await
            == 0
        {
            return;
        }
    }
    panic!("`{line}` was refused as commit-pending {RETRY_BUDGET} times running");
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

/// Walk every pilot into the simulator lobby, and answer its dbref.
async fn open_the_centre(god: &mut Term) -> u32 {
    let listing = ask(god, "@find Simulator Lobby", "END OF LIST").await;
    let lobby = dbref_of(&listing, "Simulator Lobby");
    for pilot in [ADA, BO, CY] {
        say(
            god,
            &format!("@teleport #{}=#{lobby}", pilot.0),
            "Teleported",
        )
        .await;
    }
    lobby
}

// ---------------------------------------------------------------------------
// (1) The walk, twice, and the account rules
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_pilots_walk_selection_and_boarding_twice_and_a_slot_belongs_to_an_account() {
    let server = boot("sim-lobby-walk").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;

    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, mut cy) = login(server.addr, "cy", PILOT_PASSWORD, "Cy").await;

    for run in 1..=2 {
        // `Idle -> Selection`, and the centre says so to everyone in it.
        say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
        say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;

        // One account, one slot — whichever side it reaches for.
        say(&mut ada, "SIM TEAM blue", "already hold red slot 1").await;

        // A third account takes the *next* slot, never a taken one.
        say(&mut cy, "SIM TEAM red", "takes red slot 2").await;
        say(&mut cy, "SIM LEAVE", "gives up red slot 2").await;

        // The status a pilot reads, anywhere in the centre.
        let status = ask(&mut ada, "SIM STATUS", "blue:").await;
        assert!(status.contains("selection"), "run {run}: {status}");
        assert!(status.contains("Ada"), "run {run}: {status}");
        assert!(status.contains("Bo"), "run {run}: {status}");

        // A pilot who is not a side's first cannot start it.
        say(&mut cy, "SIM START", "first pilot or staff").await;

        // `Selection -> Boarding`: the pods lock, slot n to pod n.
        say(&mut ada, "SIM START", "is boarding").await;
        let status = ask(&mut ada, "SIM STATUS", "blue:").await;
        assert!(status.contains("boarding"), "run {run}: {status}");
        assert!(status.contains("Red Pod 1"), "run {run}: {status}");
        assert!(status.contains("Blue Pod 1"), "run {run}: {status}");

        // `Boarding -> Aborted`, by a participant, and the pods come back.
        say(&mut bo, "SIM ABORT", "calls the match off").await;
        let status = ask(&mut ada, "SIM STATUS", "blue:").await;
        assert!(status.contains("aborted"), "run {run}: {status}");
        assert!(status.contains("red: nobody"), "run {run}: {status}");

        // `Aborted -> Idle`: the staff reset, which run 2 then proves.
        say(&mut god, "@arena/reset", "centre is reset").await;
    }

    // And the second reset says nothing changed.
    say(&mut god, "@arena/reset", "already idle; nothing changed").await;
}

// ---------------------------------------------------------------------------
// (2) The gate and the boarding deadline
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sim_is_refused_outside_the_centre_and_a_boarding_nobody_answers_aborts() {
    let server = boot("sim-lobby-gate").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let lobby = open_the_centre(&mut god).await;

    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;

    // God is standing in Limbo, so even staff are refused there: the gate is
    // about the room, not about the power.
    say(&mut god, "SIM STATUS", OUTSIDE).await;
    say(&mut god, "sim/status", OUTSIDE).await;

    // A pilot walked out of the centre is refused the same way, and the same
    // verb inside it is the lobby's own.
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut god, &format!("@teleport #{}=#0", BO.0), "Teleported").await;
    say(&mut bo, "SIM ABORT", OUTSIDE).await;

    // Nobody boards. The deadline drops both unseated slots, which empties both
    // sides, and an empty side aborts (doc 16 §3.8). The deadline is a due world
    // tick, so this is wall time and nothing has to be poked.
    tokio::time::sleep(Duration::from_millis(u64::from(BOARD_MS) + 2_500)).await;
    let status = ask(&mut ada, "SIM STATUS", "blue:").await;
    assert!(status.contains("aborted"), "{status}");
    assert!(status.contains("red: nobody"), "{status}");

    // And the centre is reusable at once: a reset, and a fresh claim.
    say(&mut god, "@arena/reset", "centre is reset").await;
    say(
        &mut god,
        &format!("@teleport #{}=#{lobby}", BO.0),
        "Teleported",
    )
    .await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
}

// ---------------------------------------------------------------------------
// (3) The staff shortcut, through this cut's machine
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_staff_shortcut_runs_a_match_the_centre_knows_about() {
    let server = boot("sim-lobby-staff").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let lobby = open_the_centre(&mut god).await;

    // The arena pack's own tanks, found the way a staffer finds anything.
    let listing = ask(&mut god, "@find Red Tank 1", "END OF LIST").await;
    let red = dbref_of(&listing, "Red Tank 1");
    let listing = ask(&mut god, "@find Blue Tank 1", "END OF LIST").await;
    let blue = dbref_of(&listing, "Blue Tank 1");

    say(
        &mut god,
        &format!("@arena/start tacsim.arena.proving_ground=#{red}/#{blue}"),
        "is running on",
    )
    .await;

    // `@arena/status` now prints the centre the lobby prints, not a second one.
    let status = ask(&mut god, "@arena/status", "ledger writes awaiting").await;
    assert!(status.contains("running"), "{status}");

    // A lobby claim is refused while the centre holds a match, and so is a
    // second shortcut.
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    say(&mut ada, "SIM TEAM red", "centre is running").await;
    say(
        &mut god,
        &format!("@arena/start tacsim.arena.proving_ground=#{red}/#{blue}"),
        "holding a match",
    )
    .await;
    // A reset never drops custody; only the check-in gives the units back.
    say(&mut god, "@arena/reset", "abort` it first").await;

    // Staff standing in the lobby may `SIM ABORT` it, and the check-in follows.
    // Teleporting *yourself* answers with the destination's room view, not with
    // the third-person "Teleported." somebody else's move prints.
    say(
        &mut god,
        &format!("@teleport #{}=#{lobby}", GOD.0),
        "Simulator Lobby",
    )
    .await;
    say(&mut god, "SIM ABORT", "aborts match").await;
    god.expect("Its units are checked in.").await;

    let status = ask(&mut god, "SIM STATUS", "blue:").await;
    assert!(status.contains("aborted"), "{status}");
    say(&mut god, "@arena/reset", "centre is reset").await;
    say(&mut god, "@arena/reset", "already idle; nothing changed").await;
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
}

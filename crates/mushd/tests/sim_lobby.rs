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

/// **16b cut Z.3**: four more pilots, for the one test that fills a side of six
/// and needs a seventh account to be refused. Created after the three, so the
/// three keep their ids on every boot.
const MORE_PILOTS: [(ObjId, &str); 4] = [
    (ObjId(5), "Dee"),
    (ObjId(6), "Eli"),
    (ObjId(7), "Fay"),
    (ObjId(8), "Gus"),
];

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
fn tacsim_config(ai_fill: bool) -> tacsim::TacsimConfig {
    let mut cfg = tacsim::TacsimConfig {
        enabled: true,
        ..tacsim::TacsimConfig::default()
    };
    // **16b cut D.1**: the claim line and the two bare reads branch on it.
    cfg.match_.ai_fill = ai_fill;
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
    ai_fill: bool,
) -> Vec<Box<dyn GameModule>> {
    let mut mods = modules::server_modules();
    mods.push(modules::worldgen_module(Arc::clone(llm)));
    mods.push(modules::npc_module(modules::npc_agent_adapter()));
    mods.extend(modules::sheet_modules(onboarding, &Default::default()));
    mods.push(modules::tacsim_module(tacsim_config(ai_fill)));
    mods.push(modules::content_module_with(true, true).expect("the content module composes"));
    mods
}

async fn boot(tag: &str) -> Server {
    boot_with(tag, false).await
}

/// The same server, with `[tacsim.match].ai_fill` on or off (16b cut D.1).
async fn boot_with(tag: &str, ai_fill: bool) -> Server {
    boot_with_pilots(tag, ai_fill, &[]).await
}

/// The same server with `more` pilot accounts beside the three (16b cut Z.3).
async fn boot_with_pilots(tag: &str, ai_fill: bool, more: &[(ObjId, &str)]) -> Server {
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
    // Three pilots on three accounts, created before the task spawns, which is
    // what fixes their ids and lets the boot hydration see them in one pass. It
    // is a shortcut, not a requirement: since 16b cut F.4 the map learns of a
    // character as the world makes it, so the front door does this too (see
    // `crates/mushd/tests/front_door.rs`).
    for (id, name) in [(ADA, "Ada"), (BO, "Bo"), (CY, "Cy")]
        .into_iter()
        .chain(more.iter().copied())
    {
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

        // A pilot who does not lead a side cannot start it.
        say(
            &mut cy,
            "SIM START",
            "Only a side's lead or staff can start the match.",
        )
        .await;

        // **16b cut Q.7**: this server fields no machines, so the sixth selector
        // says so and the board says nothing at all about a fill.
        say(&mut ada, "SIM AI", "AI fill is off on this server.").await;
        let status = ask(&mut ada, "SIM STATUS", "blue:").await;
        assert!(
            !status.contains("AI fill:"),
            "run {run}: a board with no fill behind it says nothing about one: {status}"
        );

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
    // tick; wait for the board's actual transition under load.
    let mut status = String::new();
    wait_until(
        STEP_TIMEOUT,
        "the unanswered boarding to abort",
        async || {
            status = ask(&mut ada, "SIM STATUS", "blue:").await;
            status.contains("aborted")
        },
    )
    .await;
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

/// One answer with the terminal's own wrapping taken back out, so a sentence
/// longer than the width can be compared as one string (16b cut D.1).
fn one_line(said: &str) -> String {
    said.split_whitespace().collect::<Vec<&str>>().join(" ")
}

// ---------------------------------------------------------------------------
// (4) 16b cut D.1 — the two bare reads, the claim line and the board's clock
// ---------------------------------------------------------------------------

/// **`B-28` and `B-30`, over the real front door** (16b cut D.1).
///
/// The two reads a player is most likely to type before they know what the
/// simulator is — "what can I fly?" and "how many machines?" — answered from
/// every standing a player can have in the centre, and the claim line that says
/// what `SIM START` will actually do on this server.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn d1_the_bare_reads_and_the_claim_line_on_a_server_that_fields_machines() {
    let server = boot_with("sim-lobby-d1-fill", true).await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;

    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, mut cy) = login(server.addr, "cy", PILOT_PASSWORD, "Cy").await;

    // `Idle`, no slot, not a lead: both reads answer.
    say(&mut cy, "SIM UNIT", "Fieldable units: ").await;
    let listing = ask(&mut cy, "SIM UNIT", "Fieldable units: ").await;
    assert!(
        !listing.contains("Yours:"),
        "a reader with no slot was told what they are flying: {listing}"
    );
    say(&mut cy, "SIM AI", "AI fill: ").await;

    // **The claim line branches on the fill** (`B-30`): on this server
    // `SIM START` does not wait for the other side.
    // The needle is short because the line wraps at the terminal's width; the
    // whole sentence is compared below, with the wrap taken back out.
    let said = ask(&mut ada, "SIM TEAM red", "the machines take the pods").await;
    assert!(
        one_line(&said).contains(
            "Ada takes red slot 1. `SIM START` begins the match; the machines take the pods \
             nobody claims."
        ),
        "{said}"
    );
    say(&mut bo, "SIM TEAM red", "takes red slot 2").await;

    // A non-lead who holds a slot reads both, and is refused the SET with its
    // own sentence — not `SIM START`'s.
    let listing = ask(&mut bo, "SIM UNIT", "Fieldable units: ").await;
    assert!(listing.contains("Yours: "), "{listing}");
    say(&mut bo, "SIM AI", "AI fill: ").await;
    say(
        &mut bo,
        "SIM AI 3",
        "Only a side's lead or staff can set the AI fill.",
    )
    .await;

    // And in `Boarding`, where neither read was served at all before this cut.
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut cy, "SIM UNIT", "Fieldable units: ").await;
    say(&mut cy, "SIM AI", "AI fill: ").await;
    say(&mut ada, "SIM ABORT", "calls the match off").await;
}

/// **`B-30` the other way round**: with the fill off the claim line is byte for
/// byte the one it always was (16b cut D.1).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn d1_with_the_fill_off_the_claim_line_is_unchanged() {
    let server = boot("sim-lobby-d1-nofill").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;

    let said = ask(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    assert!(
        said.contains(
            "Ada takes red slot 1. `SIM START` begins the match once both sides have a \
                       pilot."
        ),
        "{said}"
    );
    assert!(
        !said.contains("machines take the pods"),
        "a server with no fill advertised one: {said}"
    );
}

/// **`B-29`: the board's clock is the arena's own and it moves** (16b cut D.1).
///
/// Nobody drives: the point of the row is that the figure used to be the last
/// closed ledger chunk's timestamp, which is `0 ms` until a chunk closes and a
/// minute stale after that. The clock now rides every `Frames` message, and a
/// match with nobody moving in it still sends one an arena second.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn d1_the_boards_arena_clock_moves_while_nobody_drives() {
    let server = boot("sim-lobby-d1-clock").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;
    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;

    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "red", "Red Room").await;
    say(&mut bo, "blue", "Blue Room").await;
    say(&mut ada, "BOARD", "climb into").await;
    say(&mut bo, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;

    /// The `m:ss` on the board, as whole seconds.
    fn clock_s(board: &str) -> Option<i64> {
        let row = board.lines().find(|line| line.contains("arena time "))?;
        let (_, rest) = row.split_once("arena time ")?;
        let text = rest.trim();
        let (minutes, seconds) = text.split_once(':')?;
        Some(minutes.trim().parse::<i64>().ok()? * 60 + seconds.trim().parse::<i64>().ok()?)
    }

    let board = ask(&mut ada, "SIM STATUS", "arena time ").await;
    assert!(
        !board.contains(" ms"),
        "the raw milliseconds are still on the board: {board}"
    );
    let first = clock_s(&board).unwrap_or_else(|| panic!("an `m:ss` clock on the board: {board}"));

    // Nobody types anything at the tanks. The clock moves anyway.
    wait_until(STEP_TIMEOUT, "the idle arena clock to move", async || {
        let board = ask(&mut ada, "SIM STATUS", "arena time ").await;
        let now = clock_s(&board).unwrap_or_else(|| panic!("an `m:ss` clock: {board}"));
        assert!(now >= first, "the board's clock went backwards: {board}");
        now > first
    })
    .await;

    // And `@arena/status` keeps the ledger's own figure AND prints the clock
    // beside it, so an operator can tell the two apart.
    let staff = ask(&mut god, "@arena/status", "ledger writes awaiting").await;
    assert!(staff.contains(" ms, clock "), "{staff}");

    say(&mut ada, "SIM ABORT", "aborts match").await;
}

// ---------------------------------------------------------------------------
// (5) 16b cut D.3 — the lifecycle's ends, over the real front door
// ---------------------------------------------------------------------------

/// **`B-33`: a side's lead concedes, over SSH** (16b cut D.3).
///
/// Doc 16 §3.8's elimination is *a wreck, or disabled with no weapons*, so a
/// side that is beaten but not dead has nothing to do but drive about until
/// `match.max_duration_ms`. `SIM ABORT` is the wrong word for that — an abort is
/// a match that did not happen — so a concede is a **result**: the other side
/// wins it, the board says who and why, and the units check back in exactly as
/// they do after an elimination.
///
/// Red has two pilots, so this also walks the refusal a **non-lead** reads.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn d3_a_lead_concedes_and_the_board_names_the_winner() {
    let server = boot("sim-lobby-concede").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;

    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, mut cy) = login(server.addr, "cy", PILOT_PASSWORD, "Cy").await;

    // Everybody is put in their team room BEFORE the match starts: the boarding
    // window this file runs at is three seconds, and two pilots walking into the
    // same room cannot wait on that room's name — each would match the other's
    // `Ada goes Red Room <R>.`
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

    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut cy, "SIM TEAM red", "takes red slot 2").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "BOARD", "climb into").await;
    say(&mut cy, "BOARD", "climb into").await;
    say(&mut bo, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;

    // Red's second pilot may not give red's match away.
    say(
        &mut cy,
        "SIM CONCEDE",
        "Only a side's lead can concede for it.",
    )
    .await;
    // Red's first may, and the whole centre is told.
    say(&mut ada, "SIM CONCEDE", "concedes for the red side").await;
    bo.expect("Match 1 goes to blue when the arena answers.")
        .await;
    bo.expect("Its units are checked in.").await;

    let status = one_line(&ask(&mut bo, "SIM STATUS", "blue:").await);
    assert!(
        status.contains("ended (match 1) — blue wins: red conceded"),
        "{status}"
    );
    // It is an end and not an abort, so the next match is opened the ordinary
    // way.
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
}

/// **`B-78`: a lead the boarding deadline drops leaves a lead who can concede**
/// (16b cut AA.2), over SSH, through the real `BOARD` and the real deadline.
///
/// Before AA.2 the deadline dropped Ada's slot and kept her as red's lead, so red
/// could not concede for the rest of the match: Cy read `Only a side's first
/// pilot can concede for it.` and Ada `You hold no slot in the simulator.` Now
/// Cy, red's next claimant, leads red and her concede is the result it always
/// was.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn aa2_a_lead_dropped_at_the_deadline_leaves_a_lead_who_can_concede() {
    let server = boot("sim-lobby-aa2-heir").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;

    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, mut cy) = login(server.addr, "cy", PILOT_PASSWORD, "Cy").await;

    // Into the team rooms BEFORE the match starts, for the reason
    // `d3_a_lead_concedes_and_the_board_names_the_winner` gives.
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

    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut cy, "SIM TEAM red", "takes red slot 2").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut cy, "BOARD", "climb into").await;
    say(&mut bo, "BOARD", "climb into").await;
    // Ada, red's lead, never boards. The deadline is a due world tick, so this is
    // wall time and nothing has to be poked.
    tokio::time::sleep(Duration::from_millis(u64::from(BOARD_MS) + 2_500)).await;
    cy.expect("never boarded").await;
    cy.expect("Good hunting").await;

    // The dropped lead holds no slot, so no side.
    say(
        &mut ada,
        "SIM CONCEDE",
        "You hold no slot in the simulator.",
    )
    .await;
    // Red's next claimant leads red now, and her concede is a result.
    say(&mut cy, "SIM CONCEDE", "concedes for the red side").await;
    bo.expect("Match 1 goes to blue when the arena answers.")
        .await;
    bo.expect("Its units are checked in.").await;

    let status = one_line(&ask(&mut bo, "SIM STATUS", "blue:").await);
    assert!(
        status.contains("ended (match 1) — blue wins: red conceded"),
        "{status}"
    );
}

/// **A lead who `EXIT`s mid-match hands the side on** (16b cut B.4, cut AA's
/// residual), over SSH, through the real `BOARD` and the real `EXIT`.
///
/// Before this cut `slot_left` marked Ada's slot `left` and kept her as red's
/// lead, so no succession ran: Cy, still in her tank, read the lead refusal and
/// red could not concede. Now the leave resettles the leads — Ada, standing in
/// the team room, is refused in the lead's own words, and Cy's concede is the
/// result it always was.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn b4_a_lead_who_exits_mid_match_hands_the_side_on() {
    let server = boot("sim-lobby-b4-exit").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    open_the_centre(&mut god).await;

    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, mut cy) = login(server.addr, "cy", PILOT_PASSWORD, "Cy").await;

    // Into the team rooms BEFORE the match starts, for the reason
    // `d3_a_lead_concedes_and_the_board_names_the_winner` gives.
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

    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut cy, "SIM TEAM red", "takes red slot 2").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut ada, "BOARD", "climb into").await;
    say(&mut cy, "BOARD", "climb into").await;
    say(&mut bo, "BOARD", "climb into").await;
    ada.expect("Good hunting").await;

    // Red's lead leaves her tank for good.
    say(&mut ada, "EXIT", "You climb out of").await;
    // She is in the team room, in the centre, holding a slot that has left: she
    // leads nothing now, and is told so in the lead's words.
    say(
        &mut ada,
        "SIM CONCEDE",
        "Only a side's lead can concede for it.",
    )
    .await;
    // Cy, red's next claimant still in a tank, leads red and may concede.
    say(&mut cy, "SIM CONCEDE", "concedes for the red side").await;
    bo.expect("Match 1 goes to blue when the arena answers.")
        .await;
    bo.expect("Its units are checked in.").await;

    let status = one_line(&ask(&mut bo, "SIM STATUS", "blue:").await);
    assert!(
        status.contains("ended (match 1) — blue wins: red conceded"),
        "{status}"
    );
}

/// **`T-60`: `SIM ABORT` before a match is running, and the board's doubled
/// word** (16b cut D.3).
///
/// In `Selection` there is no match — no id, no pod lock, nothing checked out —
/// and one claimant's `SIM ABORT` nevertheless wiped every other claimant's
/// slot. Staff may still call a `Selection` off, because a stuck lobby is what a
/// staff lever is for.
///
/// And the board's head no longer says the same thing twice: `aborted (match 3)
/// — abort` was the state and then the outcome word for the same fact.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn d3_sim_abort_is_refused_in_selection_and_the_board_drops_the_doubled_word() {
    let server = boot("sim-lobby-abort").await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let lobby = open_the_centre(&mut god).await;

    let (_a, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    let (_b, mut bo) = login(server.addr, "bo", PILOT_PASSWORD, "Bo").await;
    let (_c, mut cy) = login(server.addr, "cy", PILOT_PASSWORD, "Cy").await;

    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut cy, "SIM TEAM red", "takes red slot 2").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;

    // A claimant is refused, and nobody's slot goes.
    say(
        &mut cy,
        "SIM ABORT",
        "Nothing is running yet. `SIM LEAVE` gives your slot up.",
    )
    .await;
    let status = ask(&mut ada, "SIM STATUS", "blue:").await;
    assert!(status.contains("selection"), "{status}");
    assert!(
        status.contains("Cy"),
        "a refused abort dropped a slot: {status}"
    );
    assert!(status.contains("Bo"), "{status}");
    // `SIM LEAVE` is what the sentence says to type, and it works.
    say(&mut cy, "SIM LEAVE", "gives up red slot 2").await;

    // Staff standing in the lobby still may, and the board's head carries the
    // state alone — there is no match id in `Selection` and no second word for
    // the same fact.
    say(
        &mut god,
        &format!("@teleport #{}=#{lobby}", GOD.0),
        "Simulator Lobby",
    )
    .await;
    say(&mut god, "SIM ABORT", "calls the match off").await;
    let status = one_line(&ask(&mut god, "SIM STATUS", "blue:").await);
    assert!(status.contains("Simulator centre: aborted"), "{status}");
    assert!(
        !status.contains("— abort"),
        "the board said the same thing twice: {status}"
    );

    // And from `Boarding`, where there IS a match id, the head names it and
    // still says `abort` no second time.
    say(&mut god, "@arena/reset", "centre is reset").await;
    say(&mut ada, "SIM TEAM red", "takes red slot 1").await;
    say(&mut bo, "SIM TEAM blue", "takes blue slot 1").await;
    say(&mut ada, "SIM START", "is boarding").await;
    say(&mut bo, "SIM ABORT", "calls the match off").await;
    let status = one_line(&ask(&mut god, "SIM STATUS", "blue:").await);
    assert!(
        status.contains("Simulator centre: aborted (match 1)"),
        "{status}"
    );
    assert!(
        !status.contains("— abort"),
        "the board said the same thing twice: {status}"
    );
}

// ---------------------------------------------------------------------------
// (5) 16b cut Z.3 — six a side
// ---------------------------------------------------------------------------

/// **A side takes six claims, and the fill is one to six** (16b cut Z.3; 16c
/// N7), over the real front door and the production composition.
///
/// Six accounts claim red slots 1 to 6 and the seventh is refused with the
/// side-full sentence; the lead's `SIM AI 7` is refused with the range sentence
/// and `SIM AI 6` is taken; and the board lists all six red slots by index and
/// callsign.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn z3_a_side_takes_six_claims_and_the_fill_is_one_to_six() {
    let server = boot_with_pilots("sim-lobby-z3-six", true, &MORE_PILOTS).await;
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let lobby = open_the_centre(&mut god).await;
    for (pilot, _) in MORE_PILOTS {
        say(
            &mut god,
            &format!("@teleport #{}=#{lobby}", pilot.0),
            "Teleported",
        )
        .await;
    }

    let mut pilots = Vec::new();
    for name in ["Ada", "Bo", "Cy", "Dee", "Eli", "Fay", "Gus"] {
        let (handle, term) = login(server.addr, &name.to_lowercase(), PILOT_PASSWORD, name).await;
        pilots.push((handle, term, name));
    }
    for (slot, (_, term, name)) in pilots.iter_mut().take(6).enumerate() {
        say(
            term,
            "SIM TEAM red",
            &format!("{name} takes red slot {}", slot + 1),
        )
        .await;
    }
    let (_, gus, _) = pilots.last_mut().expect("seven pilots");
    say(
        gus,
        "SIM TEAM red",
        "The red side is full: it takes 6 pilot(s).",
    )
    .await;
    say(gus, "SIM TEAM blue", "Gus takes blue slot 1").await;

    let (_, ada, _) = pilots.first_mut().expect("seven pilots");
    say(ada, "SIM AI 7", "AI fill is 1 to 6 a side.").await;
    say(ada, "SIM AI 6", "AI fill set to 6 a side.").await;

    let status = ask(ada, "SIM STATUS", "AI fill: each side fields").await;
    let status = one_line(&status);
    for row in [
        "1 (RA): Ada",
        "2 (RB): Bo",
        "3 (RC): Cy",
        "4 (RD): Dee",
        "5 (RE): Eli",
        "6 (RF): Fay",
        "1 (BA): Gus",
    ] {
        assert!(status.contains(row), "{row}: {status}");
    }
    assert!(status.contains("AI fill: each side fields 6;"), "{status}");
}

//! **A player who types ahead is served** (16b cut B.5, `B-13`; owner default
//! Q7).
//!
//! Before this cut a line that arrived while another command's commit was in
//! flight was refused at once — `The world is committing another action; please
//! retry in a moment.` — so four lines sent in one burst lost three, and every
//! rig in the tree carried a retry loop a player does not have. The world task
//! now holds such a line in a bounded defer queue (four a connection, 64 in all,
//! 250 ms from its arrival) and serves it when the receipt lands.
//!
//! **This file is the story, not the proof.** Over real SSH, on the production
//! module list, one player sends four lines in ONE write — the first a
//! committing verb (`@desc` sets an attribute and answers `Set.`), so the three
//! behind it meet its commit on any store — and reads the four answers in the
//! order typed with no retry anywhere. Whether the three actually met the commit
//! on a given run depends on how fast the in-memory store answers; the proof
//! that a line behind a held commit is served, that a fifth is refused, that an
//! over-long commit refuses in a player's words and that a barrier waits behind
//! queued lines is in `crates/mush-server/src/world_task.rs`'s held-receipt loop
//! tests, where the test holds the receipt itself.
//!
//! The rig is `sim_lobby.rs`'s, cut down to one pilot: the simulator lobby is
//! where `SIM STATUS` answers with the board. It installs no account oracle,
//! because nothing here claims a slot.

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

/// The phrase both commit refusals end with — the old immediate one and the
/// cut's timed-out one. The story asserts it never appears.
const RETRY: &str = "please retry in a moment";
/// How many times the SETUP (never the story) re-sends a line on that refusal.
const RETRY_BUDGET: usize = 8;

/// The one pilot, created before the world task spawns so its id is stable.
const ADA: ObjId = ObjId(2);

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

/// The production module list, in production order, with the starter area on:
/// the simulator lobby is part of the atlas-owned OOC pack.
fn module_list(
    onboarding: &mush_core::OnboardingConfig,
    llm: &Arc<LlmService>,
) -> Vec<Box<dyn GameModule>> {
    let mut mods = modules::server_modules();
    mods.push(modules::worldgen_module(Arc::clone(llm)));
    mods.push(modules::npc_module(modules::npc_agent_adapter()));
    mods.extend(modules::sheet_modules(onboarding, &Default::default()));
    mods.push(modules::tacsim_module(tacsim::TacsimConfig {
        enabled: true,
        ..tacsim::TacsimConfig::default()
    }));
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
    let made = world
        .create(ObjKind::Player, "Ada", GOD, Some(LIMBO))
        .expect("a pilot");
    assert_eq!(made, ADA, "pilot id assumption");
    store
        .create_account(AccountRec {
            id: None,
            name: "ada".to_string(),
            argon2_hash: hash_password(PILOT_PASSWORD).expect("a hash"),
            created: i64::from(ADA.0),
            characters: vec![CharacterRef {
                id: ADA,
                name: "Ada".to_string(),
            }],
            ssh_keys: vec![],
            discord: None,
        })
        .await
        .expect("the pilot account");

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

/// SETUP ONLY: run one line and wait for `needle`, re-sending on a commit
/// refusal (the staff account's teleport can meet the first-boot pack apply).
async fn say(term: &mut Term, line: &str, needle: &str) {
    for _ in 0..RETRY_BUDGET {
        let mark = term.mark();
        term.send_line(line).await;
        if term.expect_one_of_after(mark, &[needle, RETRY]).await == 0 {
            return;
        }
    }
    panic!("`{line}` was refused as commit-pending {RETRY_BUDGET} times running");
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

/// The byte offset of `needle` in `text` at or after `from`.
fn find_from(text: &str, from: usize, needle: &str) -> usize {
    text.get(from..)
        .and_then(|rest| rest.find(needle))
        .map(|at| from + at)
        .unwrap_or_else(|| panic!("{needle:?} not found after byte {from} in:\n{text}"))
}

/// **Four lines in one write, four answers in order, no retry.**
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn four_lines_typed_ahead_behind_a_commit_are_all_answered_in_order() {
    let server = boot("typeahead").await;

    // Setup: staff walk Ada into the simulator lobby, where `SIM STATUS` reads
    // the board.
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let mark = god.mark();
    say(&mut god, "@find Simulator Lobby", "END OF LIST").await;
    let lobby = dbref_of(&strip_ansi(&god.seen[mark..]), "Simulator Lobby");
    say(
        &mut god,
        &format!("@teleport #{}=#{lobby}", ADA.0),
        "Teleported",
    )
    .await;

    let (_ada_handle, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    // Let the attach look land before the mark, so everything after it is the
    // answer to the burst.
    ada.expect("Simulator Lobby").await;

    // The story: ONE write carrying four lines. The first commits (an
    // attribute set), so the three behind it meet its commit.
    let mark = ada.mark();
    ada.send_line("@desc me=Typing ahead.\rlook\rsim status\rwho")
        .await;
    // The WHO header's last column is the last of the four answers to start.
    ada.expect_after(mark, "Doing").await;
    // And wait for the whole WHO to finish (its footer), so a late refusal of
    // any line would have had its chance to arrive.
    ada.expect_after(mark, "in your room.").await;

    let heard = strip_ansi(&ada.seen[mark..]);
    assert!(
        !heard.contains(RETRY),
        "no line of the burst was refused: {heard}"
    );
    let set = find_from(&heard, 0, "Set.");
    let room = find_from(&heard, set, "Simulator Lobby");
    let board = find_from(&heard, room, "Simulator centre:");
    let who = find_from(&heard, board, "Doing");
    assert!(
        set < room && room < board && board < who,
        "`Set.`, the room, the board and the WHO header, in the order typed: {heard}"
    );
}

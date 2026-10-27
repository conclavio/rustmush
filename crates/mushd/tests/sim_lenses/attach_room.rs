//! **The attach look reaches one connection** (16b cut B.4, 16c `B-14`).
//!
//! A connection that attaches to a character already in the world is shown the
//! room it stands in. That used to be a real `look` dispatched as the actor — and
//! a command's output goes to the actor, which is every connection attached to
//! the character — so a player who opened a second terminal (or a browser tab)
//! had the whole room printed again on the first one, mid-sentence.
//!
//! Cut B.4 made the attach look the core `room` QUERY, answered
//! `Reply { id: 0, event: Room }` to the one connection that asked. This drives
//! it over **real SSH** through the production gateway path
//! (`mush_gateway::bridge_common::GameBridge::enter_game`): Ada attaches twice,
//! the second connection reads the room, and the first reads nothing of it.

use crate::support;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_core::{Engine, GOD, LIMBO, ObjId, ObjKind};
use mush_llm::{LlmProvider, LlmService, MockProvider};
use mush_session::SessionManager;
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
const PLAYER_PASSWORD: &str = "player-secret-1";

/// The world's transient authoritative refusal: a commit is in flight.
const COMMIT_PENDING: &str = "please retry in a moment";
/// How many times a line is re-sent on that refusal before the test gives up.
const RETRY_BUDGET: usize = 40;

/// Ada, created before the world task spawns so her id is stable.
const ADA: ObjId = ObjId(2);

/// The room everybody starts in, as its render heads it.
const LIMBO_HEAD: &str = "Limbo";

/// A line nothing registers, whose parser refusal fences a stream.
const SENTINEL: &str = "zzq-attach-sentinel";
/// The parser's refusal.
const HUH: &str = "Huh?";

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

/// The production server modules and nothing of the game: the attach look is a
/// core query, and a room is a room.
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
        .expect("a player");
    assert_eq!(made, ADA, "player id assumption");
    store
        .create_account(AccountRec {
            id: None,
            name: "ada".to_string(),
            argon2_hash: hash_password(PLAYER_PASSWORD).expect("a hash"),
            created: 1,
            characters: vec![CharacterRef {
                id: ADA,
                name: "Ada".to_string(),
            }],
            ssh_keys: vec![],
            discord: None,
        })
        .await
        .expect("a player account");

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
        Engine::with_modules(world, registry, modules::server_modules()).expect("modules");
    modules::boot_modules(&mut engine, &*store)
        .await
        .expect("module boot");

    let sessions = Arc::new(SessionManager::new());
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
        "Welcome to the attach lens.".to_string(),
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
    term.expect("Welcome to the attach lens.").await;
    term.send_line(&format!("connect {account} {password}"))
        .await;
    term.expect(&format!("Connected as {character}")).await;
    (handle, term)
}

/// Run one line and wait for `needle`, retrying the commit transient.
async fn say(term: &mut Term, line: &str, needle: &str) {
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
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("`{line}` was refused as transient {RETRY_BUDGET} times running");
}

/// **Two connections, one character: the second attach's room is the second
/// connection's alone** (16b cut B.4, `B-14`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_connections_attach_look_is_not_printed_on_the_first() {
    let server = boot("attach-room").await;
    let (_g, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    // Give Limbo an exit, so the room's render carries an `Exits` fact the first
    // connection could be caught printing.
    say(&mut god, "@dig Annex = east;e, west;w", "Annex created").await;

    // Connection A: Ada's first attach is shown her room, exits and all.
    let (_a, mut a) = login(server.addr, "ada", PLAYER_PASSWORD, "Ada").await;
    a.expect(LIMBO_HEAD).await;
    a.expect("Exits").await;
    let mark = a.mark();

    // Connection B: the same character, a second time. It is told it is the
    // second connection, and it is shown the room.
    let (_b, mut b) = login(server.addr, "ada", PLAYER_PASSWORD, "Ada").await;
    b.expect("2 connections").await;
    b.expect(LIMBO_HEAD).await;
    b.expect("Exits").await;
    b.expect("east").await;

    // A's stream after the mark, fenced by a parser refusal of its own: the
    // world answers A's line after everything it sent before it, so a room the
    // attach had addressed to the actor would be in the slice before the `Huh?`.
    say(&mut a, SENTINEL, HUH).await;
    let after = strip_ansi(&a.seen[mark..]);
    assert!(
        !after.contains(LIMBO_HEAD),
        "B's attach printed the room's name on A:\n{after}"
    );
    assert!(
        !after.contains("Exits"),
        "B's attach printed the room's exits on A:\n{after}"
    );
    // The session line is B's own too (it always was, conn-addressed).
    assert!(
        !after.contains("Connected as Ada"),
        "B's session line reached A:\n{after}"
    );

    // And a typed `look` is unchanged: it answers on the connection that typed
    // it (and, being a command, on every connection of the actor).
    let mark = b.mark();
    say(&mut a, "look", "Exits").await;
    b.expect_after(mark, LIMBO_HEAD).await;
}

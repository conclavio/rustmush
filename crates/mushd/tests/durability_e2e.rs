//! The M2 exit criterion, end to end: kill -9 the REAL server binary
//! mid-play, restart it on the same store, and the world — including the
//! last acked command — plus the accounts survive.
//!
//! Three tests:
//!
//! - `kill9_mid_play_world_survives_redis_gated` — spawns the actual
//!   `mushd` binary (`CARGO_BIN_EXE_mushd`) against a live Redis (own
//!   logical db so parallel test binaries never collide), drives a real SSH
//!   journey (account + character creation, `@dig`, `say`, walking into the
//!   new room), hard-kills the process (SIGKILL / `TerminateProcess` — no
//!   shutdown path runs), makes Redis persist-and-reload its dataset
//!   (best-effort `DEBUG RELOAD` — the closest a shared CI service gets to
//!   "kill both processes"), restarts mushd, and asserts everything
//!   survived. Gated on `REDIS_URL`; skips with a note otherwise.
//! - `clean_shutdown_drains_and_snapshots` — the orderly twin: in-game
//!   `@shutdown` (Wizard) makes the child exit 0 after draining the writer
//!   and triggering the shutdown BGSAVE (both asserted from its log).
//!   Runs against Redis when `REDIS_URL` is set, the in-memory backend
//!   otherwise — so the child-process path is exercised on every machine.
//! - `kill9_journey_local_twin_in_memory` — the identical journey through
//!   the direct world-task harness (no child process) against the
//!   in-memory store, with the "kill" as a hard task abort; keeps the whole
//!   logic covered on machines without Redis.
//!
//! Disconnect semantics asserted here (the documented M2 behavior):
//! characters stay where they were — there is no send-home on disconnect
//! or on server restart.

mod support;

use std::sync::Arc;
use std::time::Duration;

use mush_core::{Attr, Engine, GOD, Inbound, LIMBO, ObjId, ObjKind};
use mush_proto::{ClientCaps, ClientKind, ColorDepth, OutputEvent};
use mush_session::{ConnId, ConnectionHandle, SessionManager};
use mush_store::{InMemoryStore, ObjectStore};
use mush_testkit::text_of;
use tokio::sync::mpsc;

use mushd::accounts::{Accounts, AuthBackend, AuthOutcome};
use mushd::ipguard::IpGuard;
use mushd::persist::load_or_seed_world;
use mushd::staff::{StaffQueue, register_server_commands};
use mushd::world_task::{self, PersistHandles, ServerDeps, WorldHandle, char_key};

use support::{
    ChildServer, parse_id_after, play_client, redis_url_with_db, scratch_dir, seed_boss_account,
    strip_ansi, write_config,
};

const GM_PASSWORD: &str = "gm-secret-1";

/// Child-process tests serialize: BGSAVE is instance-wide on Redis, so a
/// `@dump` in one test must not race the shutdown BGSAVE of another.
static CHILD_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ---------------------------------------------------------------------------
// 1. kill -9 the real binary mid-play (REDIS_URL-gated)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn kill9_mid_play_world_survives_redis_gated() {
    let Some(base) = std::env::var("REDIS_URL").ok().filter(|s| !s.is_empty()) else {
        eprintln!(
            "skipping kill9_mid_play_world_survives_redis_gated: REDIS_URL not set \
             (no live Redis on this machine)"
        );
        return;
    };
    let _serialize = CHILD_LOCK.lock().await;
    let url = redis_url_with_db(&base, 13);
    let admin = mush_store::RedisStore::connect(&url)
        .await
        .expect("REDIS_URL set but connect failed");
    admin.wipe_namespaces().await.expect("wipe test db");
    admin.close().await.ok();

    let dir = scratch_dir("kill9");
    seed_boss_account(&dir.join("data"), GM_PASSWORD);
    let cfg = write_config(&dir, &format!("store = \"redis\"\nredis_url = \"{url}\""));

    // --- first life: the journey ------------------------------------------
    let mut server = ChildServer::spawn(&cfg);
    let addr = server.ssh_addr().await;

    let (_god_handle, mut god) = play_client(addr).await;
    god.expect("connect <account> <password>").await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God").await;
    god.expect("Limbo #0").await;

    let (_nick_handle, mut nick) = play_client(addr).await;
    nick.send_line("create Nick hunter42").await;
    nick.expect("Connected as Nick").await;
    nick.expect("Limbo #0").await;
    god.expect("Nick has connected.").await;

    god.send_line("@dig Vault = east;e, west;w").await;
    god.expect("Vault created with room number #").await;
    let vault_id = parse_id_after(&strip_ansi(&god.seen), "Vault created with room number #");
    god.send_line("@desc here = A featureless white plain.")
        .await;
    god.expect("Set.").await;

    nick.send_line("say remember the orb").await;
    god.expect("Nick says, \"remember the orb\"").await;
    nick.send_line("east").await;
    nick.expect(&format!("Vault #{vault_id}")).await;
    god.expect("Nick goes east.").await;

    god.send_line("@create orb").await;
    god.expect("Created: orb(#").await;
    let orb_id = parse_id_after(&strip_ansi(&god.seen), "Created: orb(#");

    // Ack barrier: `@dump` rides the writer queue, so its acknowledgement
    // proves every batch above it is in Redis — "the last acked command".
    god.send_line("@dump").await;
    god.expect("@dump: snapshot acknowledged by the store.")
        .await;

    // --- kill -9: no shutdown hook, no drain, no goodbye ------------------
    server.kill_hard();
    drop(god);
    drop(nick);

    // Approximate PLAN's "kill both processes" as far as a shared CI Redis
    // allows: DEBUG RELOAD makes Redis serialize the dataset to disk and
    // reload it synchronously, exercising the persist-and-recover path a
    // Redis restart would take — without killing the (shared) service
    // container. Best-effort: Redis 7+ can disable DEBUG
    // (`enable-debug-command`); when refused, the test degrades to
    // recovery-from-Redis-RAM and says so.
    {
        use fred::interfaces::ClientLike;
        use fred::types::{ClusterHash, CustomCommand, Value};
        let admin = mush_store::RedisStore::connect(&url)
            .await
            .expect("reconnect for DEBUG RELOAD");
        let reload = CustomCommand::new_static("DEBUG", ClusterHash::Random, false);
        match admin
            .client()
            .custom::<Value, _>(reload, vec!["RELOAD"])
            .await
        {
            Ok(_) => eprintln!("DEBUG RELOAD: Redis persisted and reloaded the dataset"),
            Err(e) => {
                eprintln!("DEBUG RELOAD unavailable ({e}); recovery below is from Redis RAM only")
            }
        }
        admin.close().await.ok();
    }

    // --- second life: same store, everything back -------------------------
    let mut server2 = ChildServer::spawn(&cfg);
    let addr2 = server2.ssh_addr().await;

    let (_god2_handle, mut god2) = play_client(addr2).await;
    god2.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god2.expect("Connected as God").await;
    god2.expect("Limbo #0").await;
    // @desc survived.
    god2.expect("A featureless white plain.").await;

    // The account password still verifies — and a wrong one still fails.
    let (_nick2_handle, mut nick2) = play_client(addr2).await;
    nick2.send_line("connect Nick wrong-password").await;
    nick2
        .expect("Either that account does not exist, or the password is wrong.")
        .await;
    nick2.send_line("connect Nick hunter42").await;
    nick2.expect("Connected as Nick").await;
    // Nick is exactly where the crash left him: in the dug room (documented
    // M2 behavior — no send-home on disconnect/restart), with the same id.
    nick2.expect(&format!("Vault #{vault_id}")).await;

    // The exits survived both ways: walk back into Limbo.
    let mark = nick2.mark();
    nick2.send_line("west").await;
    nick2.expect_after(mark, "Limbo #0").await;
    god2.expect("Nick has arrived.").await;

    // next_id did not regress: a fresh @create must not alias any
    // pre-crash object (orb held the highest pre-crash id).
    god2.send_line("@create relic").await;
    god2.expect("Created: relic(#").await;
    let relic_id = parse_id_after(&strip_ansi(&god2.seen), "Created: relic(#");
    assert!(
        relic_id > orb_id,
        "next_id regressed after kill -9: relic got #{relic_id}, pre-crash orb was #{orb_id}"
    );

    server2.kill_hard();
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 2. Clean shutdown of the real binary: @shutdown → drain → BGSAVE → exit 0
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn clean_shutdown_drains_and_snapshots() {
    let _serialize = CHILD_LOCK.lock().await;
    let dir = scratch_dir("shutdown");
    seed_boss_account(&dir.join("data"), GM_PASSWORD);
    // Redis when available (own logical db), in-memory otherwise — the
    // child-process shutdown path runs on every machine either way.
    let store_section = match std::env::var("REDIS_URL").ok().filter(|s| !s.is_empty()) {
        Some(base) => {
            let url = redis_url_with_db(&base, 12);
            let admin = mush_store::RedisStore::connect(&url)
                .await
                .expect("REDIS_URL set but connect failed");
            admin.wipe_namespaces().await.expect("wipe test db");
            admin.close().await.ok();
            format!("store = \"redis\"\nredis_url = \"{url}\"")
        }
        None => {
            eprintln!(
                "clean_shutdown_drains_and_snapshots: REDIS_URL not set; \
                 running the child against the in-memory backend"
            );
            "store = \"memory\"".to_string()
        }
    };
    let cfg = write_config(&dir, &store_section);

    let mut server = ChildServer::spawn(&cfg);
    let addr = server.ssh_addr().await;
    let (_handle, mut god) = play_client(addr).await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God").await;
    // Leave something for the writer to have written through.
    god.send_line("@create keepsake").await;
    god.expect("Created: keepsake(#").await;

    god.send_line("@shutdown").await;
    // The engine's ControlMsg::Shutdown announce reaches the player before
    // the world task exits.
    god.expect("GAME: Shutdown in progress.").await;

    let status = server.wait_exit(Duration::from_secs(30)).await;
    assert!(
        status.success(),
        "clean shutdown must exit 0, got {status:?}; log:\n{}",
        server.log_snapshot()
    );
    // The PLAN shutdown order, as logged: world stops, the writer drains
    // every pending batch, then the store snapshot is triggered.
    server.await_log("world task shut down cleanly").await;
    server.await_log("store writer drained and exiting").await;
    server.await_log("shutdown BGSAVE triggered").await;
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 3. The local (ungated) twin: same journey, direct world-task harness
// ---------------------------------------------------------------------------

type EventRx = mpsc::Receiver<Arc<OutputEvent>>;

fn caps() -> ClientCaps {
    ClientCaps {
        kind: ClientKind::Ssh,
        color: ColorDepth::Ansi16,
        width: 80,
        height: 24,
        utf8: true,
        structured: false,
    }
}

async fn expect_text(rx: &mut EventRx, needle: &str) {
    let fut = async {
        loop {
            match rx.recv().await {
                Some(ev) => {
                    if text_of(&ev).contains(needle) {
                        return;
                    }
                }
                None => panic!("event channel closed while waiting for {needle:?}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(60), fut)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {needle:?}"));
}

/// Attach a fake connection for `actor` and announce it to the world.
async fn attach_direct(
    sessions: &SessionManager,
    world: &WorldHandle,
    conn: u64,
    actor: ObjId,
    account: &str,
) -> EventRx {
    let (tx, rx) = mpsc::channel(256);
    let handle = ConnectionHandle::new(ConnId(conn), [127, 0, 0, 9].into(), caps(), tx);
    sessions.attach(handle, char_key(actor), account.to_string());
    assert!(
        world
            .inbound(Inbound::ConnUp {
                conn,
                actor,
                caps_structured: false,
                from_linkdead: false,
            })
            .await
    );
    rx
}

async fn cmd(world: &WorldHandle, conn: u64, actor: ObjId, line: &str) {
    assert!(
        world
            .inbound(Inbound::Cmd {
                conn,
                actor,
                line: line.into(),
            })
            .await
    );
}

/// Boot a full rig (registry incl. staff commands) from whatever the store
/// holds, seeding when empty.
async fn boot_rig(store: &Arc<dyn ObjectStore>) -> (Arc<SessionManager>, PersistHandles) {
    let world = load_or_seed_world(&**store).await.expect("boot world");
    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff).unwrap();
    let engine = Engine::new(world, registry);
    let sessions = Arc::new(SessionManager::new());
    let handles = world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        ServerDeps {
            metrics: std::sync::Arc::new(mush_net_web::Metrics::new()),
            store: Arc::clone(store),
            guard: Arc::new(IpGuard::new()),
            staff,
            llm: None,
            quarantine_spool: mushd::quarantine::QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
        },
    );
    (sessions, handles)
}

#[tokio::test(flavor = "multi_thread")]
async fn kill9_journey_local_twin_in_memory() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());

    // --- first life -------------------------------------------------------
    let (sessions, handles) = boot_rig(&store).await;
    let world = handles.handle.clone();

    // Account + character through the real backend (ack-gated creation).
    let accounts = Accounts::new(Arc::clone(&store), world.clone(), 3);
    let nick = accounts
        .create("nick", "hunter42", "Nick")
        .await
        .expect("create account+character")
        .id;

    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "boss").await;
    let mut nick_rx = attach_direct(&sessions, &world, 2, nick, "nick").await;
    expect_text(&mut god_rx, "Nick has connected.").await;

    cmd(&world, 1, GOD, "@dig Vault = east;e, west;w").await;
    expect_text(&mut god_rx, "Vault created with room number #").await;
    cmd(&world, 1, GOD, "@desc here = A featureless white plain.").await;
    expect_text(&mut god_rx, "Set.").await;

    cmd(&world, 2, nick, "say remember the orb").await;
    expect_text(&mut god_rx, "Nick says, \"remember the orb\"").await;
    cmd(&world, 2, nick, "east").await;
    expect_text(&mut nick_rx, "Vault(#").await;
    expect_text(&mut god_rx, "Nick goes east.").await;

    cmd(&world, 1, GOD, "@create orb").await;
    expect_text(&mut god_rx, "Created: orb(#").await;

    // Ack barrier, same as the SSH journey: @dump's acknowledgement is
    // ordered behind every batch above.
    cmd(&world, 1, GOD, "@dump").await;
    expect_text(&mut god_rx, "@dump: snapshot acknowledged by the store.").await;

    // --- kill -9 equivalent: abort both tasks, no drain, no goodbye -------
    let PersistHandles {
        handle,
        world_join,
        writer_join,
        ..
    } = handles;
    world_join.abort();
    writer_join.abort();
    drop(handle);
    drop(world);

    // --- second life: reload from the same store --------------------------
    let mut reloaded = load_or_seed_world(&*store).await.expect("reload");
    assert_eq!(
        reloaded.resolve_player("Nick"),
        Some(nick),
        "the character survived with the same id"
    );
    let vault = reloaded
        .iter()
        .find(|o| o.kind == ObjKind::Room && o.name == "Vault")
        .expect("the dug room survived")
        .id;
    // Documented disconnect semantics: characters stay where they were —
    // Nick is still in the Vault, not sent home.
    assert_eq!(
        reloaded.get(nick).unwrap().location,
        Some(vault),
        "Nick must still be in the dug room after the crash"
    );
    // Exits both ways, desc, and the orb all survived.
    let east = reloaded
        .exits_of(LIMBO)
        .iter()
        .copied()
        .find(|&e| reloaded.get(e).unwrap().name.starts_with("east"))
        .expect("east exit survived");
    assert_eq!(reloaded.get(east).unwrap().destination, Some(vault));
    let west = reloaded
        .exits_of(vault)
        .iter()
        .copied()
        .find(|&e| reloaded.get(e).unwrap().name.starts_with("west"))
        .expect("west exit survived");
    assert_eq!(reloaded.get(west).unwrap().destination, Some(LIMBO));
    assert_eq!(
        reloaded.attr(LIMBO, "desc").and_then(Attr::as_text),
        Some("A featureless white plain."),
        "@desc survived"
    );
    let orb_id = reloaded
        .iter()
        .find(|o| o.kind == ObjKind::Thing && o.name == "orb")
        .expect("orb survived")
        .id;
    // next_id did not regress: a fresh create continues past every
    // persisted id instead of aliasing one.
    let max_id = reloaded.iter().map(|o| o.id.0).max().unwrap();
    assert!(orb_id.0 <= max_id);
    assert!(
        reloaded.next_id() > max_id,
        "next_id ({}) must exceed every persisted id ({max_id})",
        reloaded.next_id()
    );
    let probe = reloaded
        .create(ObjKind::Thing, "probe", GOD, Some(LIMBO))
        .unwrap();
    assert!(probe.0 > max_id);

    // --- and the store still runs a live, playable server ----------------
    let (sessions2, handles2) = boot_rig(&store).await;
    let world2 = handles2.handle.clone();
    let accounts2 = Accounts::new(Arc::clone(&store), world2.clone(), 3);
    match accounts2.verify("nick", "hunter42").await {
        AuthOutcome::Ok {
            account,
            characters,
        } => {
            assert_eq!(account, "nick");
            assert_eq!(characters.len(), 1);
            assert_eq!(characters[0].id, nick);
            assert_eq!(characters[0].name, "Nick");
        }
        AuthOutcome::BadCredentials => panic!("the account password must still verify"),
    }
    assert_eq!(
        accounts2.verify("nick", "wrong-password").await,
        AuthOutcome::BadCredentials
    );
    let mut nick_rx2 = attach_direct(&sessions2, &world2, 7, nick, "nick").await;
    cmd(&world2, 7, nick, "look").await;
    expect_text(&mut nick_rx2, &format!("Vault(#{}", vault.0)).await;
}

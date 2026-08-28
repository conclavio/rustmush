//! M2 write-through persistence, driven through the REAL world task:
//! boot-seed-persist-reload round trips, ack-gated creation (including the
//! dropped-batch-must-not-ack case), the read-only (pause-mutations)
//! policy (including exit-alias bypass and creation refusal), `@destroy`
//! sweep-to-store-delete, ordered sitebans, and `@dump` — all against the
//! in-memory store locally; the boot round trip also runs against a live
//! Redis when `REDIS_URL` is set (skips cleanly otherwise).

mod support;

use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use mush_core::{Engine, GOD, Inbound, LIMBO, ObjId, ObjKind};
use mush_proto::{ClientCaps, ClientKind, ColorDepth, OutputEvent};
use mush_session::{ConnId, ConnectionHandle, SessionManager};
use mush_store::{
    AccountRec, BanRec, InMemoryStore, ObjectStore, StoreError, TokenRecord, WorldSnapshot,
    WriteBatch, WriteReceipt,
};
use mush_testkit::text_of;
use tokio::sync::mpsc;

use mushd::accounts::{Accounts, AuthBackend};
use mushd::ipguard::IpGuard;
use mushd::persist::load_or_seed_world;
use mushd::staff::{StaffQueue, register_server_commands};
use mushd::world_task::{self, PersistHandles, ServerDeps, WorldHandle, char_key};

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

fn ip(last: u8) -> IpAddr {
    IpAddr::from([127, 0, 0, last])
}

async fn expect_text(rx: &mut EventRx, needle: &str) -> Arc<OutputEvent> {
    let fut = async {
        loop {
            match rx.recv().await {
                Some(ev) => {
                    if text_of(&ev).contains(needle) {
                        return ev;
                    }
                }
                None => panic!("event channel closed while waiting for {needle:?}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(60), fut)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {needle:?}"))
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
    let handle = ConnectionHandle::new(ConnId(conn), ip(9), caps(), tx);
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

/// Boot a full server rig (registry incl. staff commands) from whatever the
/// store holds, seeding when empty.
async fn boot_rig(
    store: &Arc<dyn ObjectStore>,
    guard: &Arc<IpGuard>,
) -> (Arc<SessionManager>, PersistHandles) {
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
            store: Arc::clone(store),
            guard: Arc::clone(guard),
            staff,
        },
    );
    (sessions, handles)
}

/// Like [`boot_rig`], but booting the full production **game modules**
/// (`Engine::with_modules` + `boot_modules`) — the module-op write-through
/// and reboot-rebuild path (weather cells, script recompile) the M5 twin
/// exercises over a real store.
async fn boot_module_rig(
    store: &Arc<dyn ObjectStore>,
    guard: &Arc<IpGuard>,
) -> (Arc<SessionManager>, PersistHandles) {
    let world = load_or_seed_world(&**store).await.expect("boot world");
    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff).unwrap();
    let mut engine = Engine::with_modules(world, registry, mushd::modules::server_modules())
        .expect("module registration");
    mushd::modules::boot_modules(&mut engine, &**store)
        .await
        .expect("module boot");
    let sessions = Arc::new(SessionManager::new());
    let handles = world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        ServerDeps {
            store: Arc::clone(store),
            guard: Arc::clone(guard),
            staff,
        },
    );
    (sessions, handles)
}

async fn god_cmd(world: &WorldHandle, line: &str) {
    assert!(
        world
            .inbound(Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: line.into(),
            })
            .await
    );
}

// ---------------------------------------------------------------------------
// 1. Boot → seed → play → reload round trip (shared with the Redis variant)
// ---------------------------------------------------------------------------

/// First boot seeds an empty store; a session builds a room, sets an attr,
/// and creates a Player (whose ack proves every earlier batch landed —
/// writes are strictly ordered). A second boot from the same store must see
/// the identical world, including `next_id`, and be fully playable.
async fn boot_round_trip(store: Arc<dyn ObjectStore>) {
    // --- first boot: empty store -> seeded minimal world -----------------
    let guard = Arc::new(IpGuard::new());
    let (sessions, handles) = boot_rig(&store, &guard).await;
    let world = handles.handle.clone();
    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;

    god_cmd(&world, "@dig Vault = east;e, west;w").await;
    god_cmd(&world, "@desc here = A featureless white plain.").await;
    god_cmd(&world, "@create orb").await;
    expect_text(&mut god_rx, "Vault created").await;

    // The ack-gated create is queued behind every batch above; its return
    // means the store is current through it.
    let nick = world.create_player("Nick").await.expect("create Nick");

    // --- second boot: a fresh world hydrated from the store --------------
    let mut reloaded = load_or_seed_world(&*store).await.expect("reload");
    assert_eq!(reloaded.resolve_player("Nick"), Some(nick));
    assert_eq!(reloaded.resolve_player("God"), Some(GOD));
    let vault = reloaded
        .iter()
        .find(|o| o.kind == ObjKind::Room && o.name == "Vault")
        .expect("Vault survived")
        .id;
    let exit_names: Vec<&str> = reloaded
        .exits_of(LIMBO)
        .iter()
        .map(|&e| reloaded.get(e).unwrap().name.as_str())
        .collect();
    assert!(
        exit_names.iter().any(|n| n.starts_with("east")),
        "east exit into the Vault survived: {exit_names:?}"
    );
    let east = reloaded.exits_of(LIMBO)[0];
    assert_eq!(reloaded.get(east).unwrap().destination, Some(vault));
    assert_eq!(
        reloaded
            .attr(LIMBO, "desc")
            .and_then(mush_core::Attr::as_text),
        Some("A featureless white plain."),
        "@desc survived"
    );
    assert!(
        reloaded
            .iter()
            .any(|o| o.kind == ObjKind::Thing && o.name == "orb"),
        "@create orb survived"
    );
    // The id allocator resumes exactly: the next created object continues
    // the sequence instead of aliasing an existing id.
    let expected_next = reloaded.next_id();
    let probe = reloaded
        .create(ObjKind::Thing, "probe", GOD, Some(LIMBO))
        .unwrap();
    assert_eq!(probe.0, expected_next);
    assert!(
        expected_next > nick.0,
        "next_id ({expected_next}) must exceed every persisted id"
    );

    // --- and the reloaded world is live: boot a second world task on a
    // fresh store copy of it and talk through it. -------------------------
    let store2: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let world2 = load_or_seed_world(&*store).await.expect("reload again");
    let engine2 = Engine::new(world2, world_task::base_registry());
    let sessions2 = Arc::new(SessionManager::new());
    let handles2 = world_task::spawn_persist(
        engine2,
        Arc::clone(&sessions2),
        ServerDeps {
            store: store2,
            guard: Arc::new(IpGuard::new()),
            staff: Arc::new(StaffQueue::new()),
        },
    );
    let mut nick_rx = attach_direct(&sessions2, &handles2.handle, 7, nick, "nick").await;
    assert!(
        handles2
            .handle
            .inbound(Inbound::Cmd {
                conn: 7,
                actor: nick,
                line: "look".into(),
            })
            .await
    );
    expect_text(&mut nick_rx, "Limbo(#0)").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn boot_seed_persist_reload_round_trip_in_memory() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    boot_round_trip(store).await;
}

/// The same round trip against a live Redis — the kill-9 surrogate the M2
/// exit criterion wants (the store outlives the first world task; the
/// second boot rebuilds from Redis alone). Gated on `REDIS_URL`.
///
/// Runs in its own logical database (14): cargo runs test *binaries* in
/// parallel, so sharing db 0 with mush-store's redis suite (which wipes the
/// same namespaces) would race.
#[tokio::test(flavor = "multi_thread")]
async fn boot_round_trip_redis_gated() {
    let Some(url) = std::env::var("REDIS_URL").ok().filter(|s| !s.is_empty()) else {
        eprintln!(
            "skipping boot_round_trip_redis_gated: REDIS_URL not set \
             (no live Redis on this machine)"
        );
        return;
    };
    let url = support::redis_url_with_db(&url, 14);
    let store = mush_store::RedisStore::connect(&url)
        .await
        .expect("connect to REDIS_URL");
    store.wipe_namespaces().await.expect("wipe test namespaces");
    boot_round_trip(Arc::new(store)).await;
}

/// The M5 module-op persistence twin, parameterized over the store: a booted
/// server with the standard game modules assigns a climate to a zone and pins
/// its weather (a `mod:weather:*` cell, written through), and stores a scripted
/// object (a `script:*` attribute on `obj:*`). A second boot rebuilds from the
/// store alone — `boot_modules` restores the weather cell, and the script
/// module recompiles the object from its persisted attribute — and both are
/// live again. Exercised both in-memory (ungated) and against a live Redis.
async fn m5_module_round_trip(store: Arc<dyn ObjectStore>) {
    let guard = Arc::new(IpGuard::new());

    // --- first boot: build a weather zone + a scripted object. -----------
    let (sessions, handles) = boot_module_rig(&store, &guard).await;
    let world = handles.handle.clone();
    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;

    // A Zone Master Object, a room in it (teleport there so `here` resolves),
    // and a pinned storm — the pin persists a weather cell through the same
    // write-through path as any command.
    god_cmd(&world, "@create Downtown").await;
    god_cmd(&world, "@set Downtown=ZONE_MASTER").await;
    god_cmd(&world, "@dig/teleport Plaza").await;
    god_cmd(&world, "@zone here=Downtown").await;
    god_cmd(&world, "@set here=OUTDOORS").await;
    god_cmd(&world, "@weather/climate here=temperate-coastal").await;
    god_cmd(&world, "@weather here=storm").await;
    expect_text(&mut god_rx, "Pinned").await;

    // A scripted object: its script lives in a `script:*` attribute on the
    // object doc, so it rides `obj:*` persistence with no module namespace.
    god_cmd(&world, "@create parrot").await;
    god_cmd(
        &world,
        r#"@set parrot/script:command:squawk=mush::pemit(mush::caller(), "SQUAWK survived the reboot")"#,
    )
    .await;

    // Fence: an ack-gated Player create returns only once the store has
    // acknowledged its batch, and the writer is strictly ordered — so every
    // fire-and-forget batch above (the weather cell, the script attribute)
    // has certainly landed in the store by the time this returns.
    world.create_player("Fence").await.expect("fence create");

    // --- second boot: rebuild from the store alone. ----------------------
    let (sessions2, handles2) = boot_module_rig(&store, &guard).await;
    let world2 = handles2.handle.clone();
    let mut god_rx2 = attach_direct(&sessions2, &world2, 2, GOD, "god").await;

    // The weather cell survived: @weather/list reports the pinned storm.
    // Key on "[pinned]" — only the list line carries it (the room render and
    // its "Overhead: a storm" conditions line also mention the zone/state).
    god_cmd(&world2, "@weather/list").await;
    let ev = expect_text(&mut god_rx2, "[pinned]").await;
    let listed = text_of(&ev);
    assert!(
        listed.contains("storm") && listed.contains("Downtown"),
        "reboot must rebuild the pinned storm cell: {listed:?}"
    );

    // The scripted object recompiled from its persisted attribute and runs.
    god_cmd(&world2, "squawk").await;
    expect_text(&mut god_rx2, "SQUAWK survived the reboot").await;
}

/// The M5 module round trip over an in-memory store — ungated, so CI proves
/// the assembled `boot_modules` + world-task reload path (namespace keys, boot
/// order) without a live Redis. The constituent logic is unit-covered in the
/// module crates; this locks the integration-level wiring.
#[tokio::test(flavor = "multi_thread")]
async fn m5_module_persistence_round_trip_in_memory() {
    m5_module_round_trip(Arc::new(InMemoryStore::new())).await;
}

/// The same round trip over a live Redis — the reboot-from-Redis-alone
/// surrogate. Gated on `REDIS_URL`; runs in db 15 (dbs 12–14 are claimed by
/// the durability and boot-round-trip gated tests, which run in parallel).
#[tokio::test(flavor = "multi_thread")]
async fn m5_module_persistence_round_trip_redis_gated() {
    let Some(url) = std::env::var("REDIS_URL").ok().filter(|s| !s.is_empty()) else {
        eprintln!(
            "skipping m5_module_persistence_round_trip_redis_gated: REDIS_URL not set \
             (no live Redis on this machine)"
        );
        return;
    };
    let url = support::redis_url_with_db(&url, 15);
    let store = mush_store::RedisStore::connect(&url)
        .await
        .expect("connect to REDIS_URL");
    store.wipe_namespaces().await.expect("wipe test namespaces");
    m5_module_round_trip(Arc::new(store)).await;
}

// ---------------------------------------------------------------------------
// A store wrapper with injectable behavior (gate / fail), for the ack and
// read-only tests. Only `write` misbehaves; everything else delegates.
// ---------------------------------------------------------------------------

const MODE_NORMAL: u8 = 0;
const MODE_HOLD: u8 = 1;
const MODE_FAIL: u8 = 2;
const MODE_CLOSED: u8 = 3;

struct FlakyStore {
    inner: InMemoryStore,
    mode: AtomicU8,
}

impl FlakyStore {
    fn new() -> FlakyStore {
        FlakyStore {
            inner: InMemoryStore::new(),
            mode: AtomicU8::new(MODE_NORMAL),
        }
    }

    fn set_mode(&self, mode: u8) {
        self.mode.store(mode, Ordering::SeqCst);
    }
}

impl ObjectStore for FlakyStore {
    fn load_world(&self) -> BoxFuture<'_, Result<Option<WorldSnapshot>, StoreError>> {
        self.inner.load_world()
    }

    fn write(&self, batch: WriteBatch) -> BoxFuture<'_, Result<WriteReceipt, StoreError>> {
        Box::pin(async move {
            loop {
                match self.mode.load(Ordering::SeqCst) {
                    MODE_HOLD => tokio::time::sleep(Duration::from_millis(10)).await,
                    MODE_FAIL => {
                        // A transient-shaped failure (the writer must retry it).
                        return Err(StoreError::Redis(fred::error::Error::new(
                            fred::error::ErrorKind::IO,
                            "injected write failure",
                        )));
                    }
                    MODE_CLOSED => return Err(StoreError::Closed),
                    _ => return self.inner.write(batch).await,
                }
            }
        })
    }

    fn create_account(&self, rec: AccountRec) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.create_account(rec)
    }

    fn get_account<'a>(
        &'a self,
        name: &'a str,
    ) -> BoxFuture<'a, Result<Option<AccountRec>, StoreError>> {
        self.inner.get_account(name)
    }

    fn link_character<'a>(
        &'a self,
        account: &'a str,
        id: ObjId,
        char_name: &'a str,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        self.inner.link_character(account, id, char_name)
    }

    fn list_accounts(&self) -> BoxFuture<'_, Result<Vec<AccountRec>, StoreError>> {
        self.inner.list_accounts()
    }

    fn set_account_password<'a>(
        &'a self,
        account: &'a str,
        argon2_hash: &'a str,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.set_account_password(account, argon2_hash)
    }

    fn set_ban<'a>(
        &'a self,
        ip: &'a str,
        until_unix: Option<i64>,
        reason: &'a str,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        self.inner.set_ban(ip, until_unix, reason)
    }

    fn get_bans(&self) -> BoxFuture<'_, Result<Vec<BanRec>, StoreError>> {
        self.inner.get_bans()
    }

    fn clear_ban<'a>(&'a self, ip: &'a str) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.clear_ban(ip)
    }

    fn token_put(&self, hash: String, rec: TokenRecord) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.token_put(hash, rec)
    }

    fn token_get<'a>(
        &'a self,
        hash: &'a str,
    ) -> BoxFuture<'a, Result<Option<TokenRecord>, StoreError>> {
        self.inner.token_get(hash)
    }

    fn token_touch<'a>(
        &'a self,
        hash: &'a str,
        new_expires: i64,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.token_touch(hash, new_expires)
    }

    fn token_del<'a>(&'a self, hash: &'a str) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.token_del(hash)
    }

    fn tokens_del_account<'a>(
        &'a self,
        account: &'a str,
    ) -> BoxFuture<'a, Result<u64, StoreError>> {
        self.inner.tokens_del_account(account)
    }

    fn module_json_get<'a>(
        &'a self,
        ns: &'a str,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Option<serde_json::Value>, StoreError>> {
        self.inner.module_json_get(ns, key)
    }

    fn module_json_list<'a>(
        &'a self,
        ns: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, serde_json::Value)>, StoreError>> {
        self.inner.module_json_list(ns)
    }

    fn stream_range<'a>(
        &'a self,
        ns: &'a str,
        key: &'a str,
        count_latest: u32,
    ) -> BoxFuture<'a, Result<Vec<(String, serde_json::Value)>, StoreError>> {
        self.inner.stream_range(ns, key, count_latest)
    }

    fn get_schema_version(&self) -> BoxFuture<'_, Result<Option<u32>, StoreError>> {
        self.inner.get_schema_version()
    }

    fn set_schema_version(&self, version: u32) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.set_schema_version(version)
    }

    fn bgsave(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.bgsave()
    }

    fn drain(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.drain()
    }

    fn close(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.close()
    }
}

// ---------------------------------------------------------------------------
// 2. Ack-gated account creation (receipt ordering)
// ---------------------------------------------------------------------------

/// Account creation must not report success while its world batch is still
/// unacknowledged: with the store's `write` held, `Accounts::create` (and
/// the underlying `create_player`) stays pending; releasing the store
/// completes it, and the Player object is then durably in the store.
#[tokio::test(flavor = "multi_thread")]
async fn account_creation_confirmation_waits_for_store_ack() {
    let flaky = Arc::new(FlakyStore::new());
    let store: Arc<dyn ObjectStore> = Arc::clone(&flaky) as Arc<dyn ObjectStore>;
    let guard = Arc::new(IpGuard::new());
    let (_sessions, handles) = boot_rig(&store, &guard).await;
    let accounts = Accounts::new(Arc::clone(&store), handles.handle.clone(), 3);

    // Hold every write, then start the creation.
    flaky.set_mode(MODE_HOLD);
    let create = tokio::spawn(async move { accounts.create("nick", "hunter42", "Nick").await });

    // Give it ample time to finish if it (wrongly) skipped the gate. The
    // account reservation is a direct store call (not a batch), so only the
    // Player batch can be what it is waiting on.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        !create.is_finished(),
        "creation confirmed before the store acknowledged the Player batch"
    );

    // Release the store: the ack flows, the creation completes, and the
    // Player object is really in the store.
    flaky.set_mode(MODE_NORMAL);
    let created = create.await.unwrap().expect("creation succeeds after ack");
    assert_eq!(created.name, "Nick");
    let snapshot = store.load_world().await.unwrap().expect("world persisted");
    assert!(
        snapshot
            .objects
            .iter()
            .any(|o| o.id == created.id && o.name == "Nick"),
        "the created Player must be durably stored before confirmation"
    );
}

// ---------------------------------------------------------------------------
// 3. Read-only mode entry/exit on store failure
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn store_failure_enters_read_only_and_recovery_exits() {
    let flaky = Arc::new(FlakyStore::new());
    let store: Arc<dyn ObjectStore> = Arc::clone(&flaky) as Arc<dyn ObjectStore>;
    let guard = Arc::new(IpGuard::new());
    let (sessions, handles) = boot_rig(&store, &guard).await;
    let world = handles.handle.clone();
    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;

    // Break the store, then mutate: the batch fails, the writer reports,
    // the world alerts its Wizards and pauses mutations.
    flaky.set_mode(MODE_FAIL);
    god_cmd(&world, "@create doomed-orb").await;
    expect_text(&mut god_rx, "store write failure").await;

    // Mutating commands are refused...
    god_cmd(&world, "@create another-orb").await;
    expect_text(&mut god_rx, "database is currently unavailable").await;
    // ...but reads and speech still work from RAM.
    god_cmd(&world, "look").await;
    expect_text(&mut god_rx, "Limbo(#0)").await;
    god_cmd(&world, "say still here").await;
    expect_text(&mut god_rx, "still here").await;

    // Store comes back: the writer's retry succeeds (backoff <= 5s), the
    // world announces recovery, and mutations flow again — including the
    // batch that was stuck, which must land in the store.
    flaky.set_mode(MODE_NORMAL);
    expect_text(&mut god_rx, "store writes recovered").await;
    god_cmd(&world, "@create recovery-orb").await;
    expect_text(&mut god_rx, "Created: recovery-orb").await;

    // Everything (including the once-stuck doomed-orb batch) is persisted.
    let nick = world.create_player("Probe").await.unwrap(); // ordering barrier
    let snapshot = store.load_world().await.unwrap().unwrap();
    for name in ["doomed-orb", "recovery-orb"] {
        assert!(
            snapshot.objects.iter().any(|o| o.name == name),
            "{name} must be persisted after recovery"
        );
    }
    assert!(snapshot.objects.iter().any(|o| o.id == nick));
}

/// A batch dropped because the store is closed must NEVER ack as success:
/// the durable creation gate has to report failure, not confirm a Player
/// that was never persisted (which would strand the account with a
/// dangling character on the next boot).
#[tokio::test(flavor = "multi_thread")]
async fn closed_store_durable_ack_reports_failure() {
    let flaky = Arc::new(FlakyStore::new());
    let store: Arc<dyn ObjectStore> = Arc::clone(&flaky) as Arc<dyn ObjectStore>;
    let guard = Arc::new(IpGuard::new());
    let (_sessions, handles) = boot_rig(&store, &guard).await;

    // The store goes away (abnormal teardown) under a pending creation.
    flaky.set_mode(MODE_CLOSED);
    let result = handles.handle.create_player("Ghost").await;
    assert!(
        result.is_err(),
        "a dropped creation batch must not be confirmed: {result:?}"
    );

    // And the store really never got the Player.
    flaky.set_mode(MODE_NORMAL);
    let snapshot = store.load_world().await.unwrap().expect("seeded world");
    assert!(
        !snapshot.objects.iter().any(|o| o.name == "Ghost"),
        "the unacked Player must not be in the store"
    );
}

// ---------------------------------------------------------------------------
// 3b. @destroy reaches the store as a delete (sweep wired into the flush)
// ---------------------------------------------------------------------------

/// `@destroy` must actually delete the object from the store: the world
/// task sweeps GOING husks into tombstones as part of the same flush, so
/// the batch carries a store delete (before this wiring, destroyed objects
/// lingered in RAM and in Redis forever).
#[tokio::test(flavor = "multi_thread")]
async fn destroy_sweeps_and_deletes_from_store() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let guard = Arc::new(IpGuard::new());
    let (sessions, handles) = boot_rig(&store, &guard).await;
    let world = handles.handle.clone();
    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;

    god_cmd(&world, "@create doomed").await;
    expect_text(&mut god_rx, "Created: doomed(#").await;
    god_cmd(&world, "@destroy doomed").await;
    expect_text(&mut god_rx, "Destroyed.").await;

    // Writer barrier: @dump's ack is ordered behind the destroy batch.
    god_cmd(&world, "@dump").await;
    expect_text(&mut god_rx, "@dump: snapshot acknowledged by the store.").await;

    let snapshot = store.load_world().await.unwrap().expect("world persisted");
    assert!(
        !snapshot.objects.iter().any(|o| o.name == "doomed"),
        "@destroy must delete the object doc from the store"
    );
    // The id was consumed (tombstone, never recycled) and the survivor set
    // still hydrates.
    assert!(snapshot.next_id > 2, "the doomed id stayed consumed");
    let reloaded = load_or_seed_world(&*store).await.expect("reboot");
    assert!(
        reloaded.iter().all(|o| o.name != "doomed"),
        "a reboot must not resurrect the destroyed object"
    );
}

// ---------------------------------------------------------------------------
// 3c. Read-only mode vs exit aliases and creation flows
// ---------------------------------------------------------------------------

/// Dispatch matches exits BEFORE the command table, so an exit aliased to
/// an allowlisted verb (`library;l` is a classic build pattern) must be
/// refused in read-only mode — traversing it would move the player. And
/// `CreatePlayer` (welcome-screen `create`) must fail closed instead of
/// mutating RAM behind a stuck writer.
#[tokio::test(flavor = "multi_thread")]
async fn read_only_refuses_exit_aliases_and_creation() {
    let flaky = Arc::new(FlakyStore::new());
    let store: Arc<dyn ObjectStore> = Arc::clone(&flaky) as Arc<dyn ObjectStore>;
    let guard = Arc::new(IpGuard::new());
    let (sessions, handles) = boot_rig(&store, &guard).await;
    let world = handles.handle.clone();
    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;

    // While healthy: dig a room whose exit is aliased "l".
    god_cmd(&world, "@dig Lounge = library;l, out;o").await;
    expect_text(&mut god_rx, "Lounge created").await;

    // Break the store; a mutation trips the writer into read-only mode.
    flaky.set_mode(MODE_FAIL);
    god_cmd(&world, "@create tripwire").await;
    expect_text(&mut god_rx, "store write failure").await;

    // "l" resolves to the exit, not to `look` — refused.
    god_cmd(&world, "l").await;
    expect_text(&mut god_rx, "database is currently unavailable").await;
    // Still in Limbo, and plain `look` still works from RAM.
    god_cmd(&world, "look").await;
    expect_text(&mut god_rx, "Limbo(#0)").await;

    // Creation flows fail closed while read-only.
    let err = world.create_player("Nope").await.unwrap_err();
    assert_eq!(
        err,
        mushd::world_task::CreatePlayerErr::StoreUnavailable,
        "CreatePlayer must be refused in read-only mode"
    );

    // Recovery lifts the gate: the same "l" now traverses the exit.
    flaky.set_mode(MODE_NORMAL);
    expect_text(&mut god_rx, "store writes recovered").await;
    god_cmd(&world, "l").await;
    expect_text(&mut god_rx, "Lounge(#").await;
}

// ---------------------------------------------------------------------------
// 4. Sitebans: @siteban persists + IpGuard consults; boot-time load
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn siteban_persists_updates_ipguard_and_clears() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let guard = Arc::new(IpGuard::new());
    let (sessions, handles) = boot_rig(&store, &guard).await;
    let world = handles.handle.clone();
    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;
    let banned: IpAddr = "10.9.8.7".parse().unwrap();

    // Wizard sets a timed ban: persisted, mirrored, confirmed.
    god_cmd(&world, "@siteban 10.9.8.7=1h flooding the login").await;
    expect_text(&mut god_rx, "@siteban: 10.9.8.7 banned until unix").await;
    assert!(guard.is_blocked(banned), "IpGuard must consult the new ban");
    let bans = store.get_bans().await.unwrap();
    assert_eq!(bans.len(), 1);
    assert_eq!(bans[0].ip, "10.9.8.7");
    assert_eq!(bans[0].reason, "flooding the login");
    assert!(bans[0].until_unix.is_some());

    // Listing shows it.
    god_cmd(&world, "@siteban").await;
    expect_text(&mut god_rx, "Active sitebans:").await;

    // A mortal is refused by dispatch and nothing changes.
    let nick = world.create_player("Nick").await.unwrap();
    let mut nick_rx = attach_direct(&sessions, &world, 2, nick, "nick").await;
    assert!(
        world
            .inbound(Inbound::Cmd {
                conn: 2,
                actor: nick,
                line: "@siteban 1.2.3.4".into(),
            })
            .await
    );
    expect_text(&mut nick_rx, "Permission denied.").await;
    assert_eq!(store.get_bans().await.unwrap().len(), 1);

    // Clearing lifts it everywhere.
    god_cmd(&world, "@siteban/clear 10.9.8.7").await;
    expect_text(&mut god_rx, "@siteban: ban on 10.9.8.7 lifted.").await;
    assert!(!guard.is_blocked(banned));
    assert!(store.get_bans().await.unwrap().is_empty());
}

/// Back-to-back `@siteban` then `@siteban/clear` must land in issue order:
/// staff store ops flow through ONE sequential executor, so the clear can
/// never lose the race to the set (which detached per-op tasks allowed).
#[tokio::test(flavor = "multi_thread")]
async fn siteban_set_then_immediate_clear_lands_cleared() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let guard = Arc::new(IpGuard::new());
    let (sessions, handles) = boot_rig(&store, &guard).await;
    let world = handles.handle.clone();
    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;
    let target: IpAddr = "10.3.3.3".parse().unwrap();

    // No awaiting between the two commands: they race only if execution is
    // unordered.
    god_cmd(&world, "@siteban 10.3.3.3=1h oops").await;
    god_cmd(&world, "@siteban/clear 10.3.3.3").await;

    // Confirmations arrive in issue order...
    expect_text(&mut god_rx, "@siteban: 10.3.3.3 banned until unix").await;
    expect_text(&mut god_rx, "@siteban: ban on 10.3.3.3 lifted.").await;
    // ...and the final state is CLEARED everywhere.
    assert!(store.get_bans().await.unwrap().is_empty());
    assert!(!guard.is_blocked(target));
}

/// The boot path: persisted bans reach the IpGuard before any connection is
/// accepted (main's wiring is `get_bans` → `load_sitebans`; this exercises
/// the same steps against a pre-populated store).
#[tokio::test(flavor = "multi_thread")]
async fn persisted_bans_load_into_ipguard_at_boot() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    store.set_ban("10.4.4.4", None, "old enemy").await.unwrap();
    store
        .set_ban("10.5.5.5", Some(mushd::unix_now_s() + 3600), "timed")
        .await
        .unwrap();

    let guard = Arc::new(IpGuard::new());
    let bans = store.get_bans().await.unwrap();
    guard.load_sitebans(
        bans.into_iter()
            .filter_map(|b| b.ip.parse().ok().map(|ip| (ip, b.until_unix))),
    );
    assert!(guard.is_blocked("10.4.4.4".parse().unwrap()));
    assert!(guard.is_blocked("10.5.5.5".parse().unwrap()));
    assert!(!guard.is_blocked("10.6.6.6".parse().unwrap()));
}

// ---------------------------------------------------------------------------
// 5. @dump rides the writer queue and confirms after the ack
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn dump_announces_and_acks() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let guard = Arc::new(IpGuard::new());
    let (sessions, handles) = boot_rig(&store, &guard).await;
    let world = handles.handle.clone();
    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;

    god_cmd(&world, "@dump").await;
    expect_text(&mut god_rx, "GAME: Dumping database.").await;
    expect_text(&mut god_rx, "@dump: snapshot acknowledged by the store.").await;
}

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
use std::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use mush_core::{
    AuthoritativeBundle, AuthoritativeBundleDraft, AuthoritativeSemanticEvent, CanonicalHash,
    CanonicalId, ClockRef, ClockStamp, CommitToken, DefinitionDescriptor, DefinitionId,
    DefinitionKind, DefinitionRegistryBuilder, Engine, GOD, IdempotencyKey, IdempotencyScope,
    Inbound, LIMBO, ObjId, ObjKind, OwnerId, PreparedCausalRef, PrincipalId, RequestUid,
    RulesValue, ScopedIdempotencyKey, WorldCommand,
};
use mush_economy::VaultId;
use mush_inventory::{EndpointRef, InventoryError, InventoryFamilies, InventoryHandles};
use mush_llm::{
    CompletionResponse, ContentBlock, LlmProvider, LlmService, MockProvider, RouteConfig,
    StopReason, Usage,
};
use mush_npc::{AppliedAction, NpcAction, TurnOutcome};
use mush_proto::{ClientCaps, ClientKind, ColorDepth, OutputEvent};
use mush_session::{ConnId, ConnectionHandle, SessionManager};
use mush_store::{
    AccountRec, BanRec, InMemoryStore, ObjectStore, StoreError, TokenRecord, WorldSnapshot,
    WriteBatch, WriteReceipt,
};
use mush_testkit::text_of;
use mushd::persist::{PersistenceIntegrityLatch, StoreJob, StoreWriter, spawn_store_writer};
use smol_str::SmolStr;
use tokio::sync::mpsc;

use mushd::accounts::{Accounts, AuthBackend};
use mushd::ipguard::IpGuard;
use mushd::persist::load_or_seed_world;
use mushd::staff::{StaffQueue, register_server_commands};
use mushd::world_task::{self, PersistHandles, ServerDeps, WorldHandle, WorldReadError, char_key};

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
            metrics: std::sync::Arc::new(mush_net_web::Metrics::new()),
            store: Arc::clone(store),
            guard: Arc::clone(guard),
            staff,
            llm: None,
            quarantine_spool: mushd::quarantine::QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
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
    // S1/S2b: the content module composes LAST, as main.rs boots it, so the
    // starter lot is applied on the first boot (and latched, so the second boot
    // rebuilds it from the store rather than applying it again).
    let mut mods = mushd::modules::server_modules();
    mods.push(mushd::modules::content_module(true).expect("content module"));
    let mut engine = Engine::with_modules(world, registry, mods).expect("module registration");
    mushd::modules::boot_modules(&mut engine, &**store)
        .await
        .expect("module boot");
    let sessions = Arc::new(SessionManager::new());
    let handles = world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        ServerDeps {
            metrics: std::sync::Arc::new(mush_net_web::Metrics::new()),
            store: Arc::clone(store),
            guard: Arc::clone(guard),
            staff,
            llm: None,
            quarantine_spool: mushd::quarantine::QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
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

/// Submit one ordinary world-mutating command and cross its exact durable
/// delivery boundary before another command is allowed to race its receipt.
async fn god_cmd_committed(world: &WorldHandle, line: &str) {
    god_cmd(world, line).await;
    assert!(world.barrier().await, "world mutation commit barrier");
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

    god_cmd_committed(&world, "@dig Vault = east;e, west;w").await;
    god_cmd_committed(&world, "@desc here = A featureless white plain.").await;
    god_cmd_committed(&world, "@create orb").await;
    expect_text(&mut god_rx, "Vault created").await;

    // Each command above crossed its exact commit barrier; player creation is
    // independently acknowledgement-gated too.
    let nick = world
        .create_player("Nick", None)
        .await
        .expect("create Nick");

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
            metrics: std::sync::Arc::new(mush_net_web::Metrics::new()),
            store: store2,
            guard: Arc::new(IpGuard::new()),
            staff: Arc::new(StaffQueue::new()),
            llm: None,
            quarantine_spool: mushd::quarantine::QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
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
    god_cmd_committed(&world, "@create Downtown").await;
    god_cmd_committed(&world, "@set Downtown=ZONE_MASTER").await;
    god_cmd_committed(&world, "@dig/teleport Plaza").await;
    god_cmd_committed(&world, "@zone here=Downtown").await;
    god_cmd_committed(&world, "@set here=OUTDOORS").await;
    // S2b: the cell keys on the atlas's OOC Area, not the zone.
    god_cmd_committed(&world, "@weather/climate OOC Area=temperate-coastal").await;
    god_cmd_committed(&world, "@weather OOC Area=storm").await;
    expect_text(&mut god_rx, "Pinned").await;

    // A scripted object: its script lives in a `script:*` attribute on the
    // object doc, so it rides `obj:*` persistence with no module namespace.
    god_cmd_committed(&world, "@create parrot").await;
    god_cmd_committed(
        &world,
        r#"@set parrot/script:command:squawk=mush::pemit(mush::caller(), "SQUAWK survived the reboot")"#,
    )
    .await;

    // Every prior mutation crossed its exact barrier; player creation is an
    // independently acknowledgement-gated final persistence probe.
    world
        .create_player("Fence", None)
        .await
        .expect("fence create");

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
        listed.contains("storm") && listed.contains("OOC Area"),
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

// ---------------------------------------------------------------------------
// H4 (E2-D30): the economy journal and an inventory pool over a reboot
// (`docs/design/11-h-implementation-plan.md` §3.4). Each runs in-memory
// (ungated) and against a live Redis — the reboot-from-Redis-alone surrogate.
// ---------------------------------------------------------------------------

/// Open an account, post, place a hold and create an obligation on one boot; a
/// second boot from the store alone shows the same balance, hold and
/// obligation, and comes up spending — the boot's contiguity and conservation
/// checks passed, or the economy would be frozen and the post refused.
async fn economy_journal_round_trip(store: Arc<dyn ObjectStore>) {
    let guard = Arc::new(IpGuard::new());
    let (sessions, handles) = boot_module_rig(&store, &guard).await;
    let world = handles.handle.clone();
    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;

    // A scale-0 instrument, so amounts render as plain integers.
    god_cmd_committed(&world, "@instrument/register gold=0").await;
    god_cmd_committed(&world, "@account/system faucet=gold").await; // account 0
    god_cmd_committed(&world, "@account/open #1=gold").await; // account 1: God
    god_cmd_committed(&world, "@account/system till=gold").await; // account 2
    god_cmd_committed(&world, "@econ/mint 0=1 500").await;
    god_cmd_committed(&world, "@hold/place 1=100 earnest").await;
    let hold = support::parse_id_after(
        &text_of(&*expect_text(&mut god_rx, "Reserved: hold #").await),
        "hold #",
    );
    god_cmd_committed(&world, "@obligation/create 1=2 10 every=manual:600").await;
    let obligation = support::parse_id_after(
        &text_of(&*expect_text(&mut god_rx, " created: ").await),
        "Obligation #",
    );
    // Every prior mutation crossed its exact barrier; player creation is an
    // independently acknowledgement-gated final persistence probe.
    world
        .create_player("Fence", None)
        .await
        .expect("fence create");

    // --- second boot: rebuild from the store alone. ----------------------
    let (sessions2, handles2) = boot_module_rig(&store, &guard).await;
    let world2 = handles2.handle.clone();
    let mut god_rx2 = attach_direct(&sessions2, &world2, 2, GOD, "god").await;
    god_cmd(&world2, "@account/show 1").await;
    let shown = text_of(&*expect_text(&mut god_rx2, "balance").await);
    assert!(
        shown.contains("500"),
        "the balance did not survive: {shown:?}"
    );
    god_cmd(&world2, &format!("@hold/show {hold}")).await;
    let shown = text_of(&*expect_text(&mut god_rx2, &format!("hold #{hold} on")).await);
    assert!(
        shown.contains("100"),
        "the hold did not survive whole: {shown:?}"
    );
    god_cmd(&world2, &format!("@obligation/show {obligation}")).await;
    let shown = text_of(&*expect_text(&mut god_rx2, &format!("obligation #{obligation} ")).await);
    assert!(
        shown.contains("owes") && shown.contains("10"),
        "the obligation did not survive whole: {shown:?}"
    );
    // Still spending: a frozen economy refuses every post.
    god_cmd_committed(&world2, "@econ/mint 0=1 1").await;
    god_cmd(&world2, "@account/show 1").await;
    let shown = text_of(&*expect_text(&mut god_rx2, "balance").await);
    assert!(
        shown.contains("501"),
        "the rebooted economy is not posting: {shown:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn economy_journal_round_trip_in_memory() {
    economy_journal_round_trip(Arc::new(InMemoryStore::new())).await;
}

/// The same round trip over a live Redis. Gated on `REDIS_URL`; runs in db 3
/// (dbs 1, 2 and 4–15 are claimed by the other gated suites).
#[tokio::test(flavor = "multi_thread")]
async fn economy_journal_round_trip_redis_gated() {
    let Some(url) = std::env::var("REDIS_URL").ok().filter(|s| !s.is_empty()) else {
        eprintln!(
            "skipping economy_journal_round_trip_redis_gated: REDIS_URL not set \
             (no live Redis on this machine)"
        );
        return;
    };
    let url = support::redis_url_with_db(&url, 3);
    let store = mush_store::RedisStore::connect(&url)
        .await
        .expect("connect to REDIS_URL");
    store.wipe_namespaces().await.expect("wipe test namespaces");
    economy_journal_round_trip(Arc::new(store)).await;
}

/// Boot the production module composition on `store` and hand back the engine
/// itself — the first half of [`boot_module_rig`], without the world task — for
/// a rig that must reach a K state family through its module's own handles.
async fn boot_engine(store: &Arc<dyn ObjectStore>) -> Engine {
    let world = load_or_seed_world(&**store).await.expect("boot world");
    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff).unwrap();
    let mut engine = Engine::with_modules(world, registry, mushd::modules::server_modules())
        .expect("module registration");
    mushd::modules::boot_modules(&mut engine, &**store)
        .await
        .expect("module boot");
    engine
}

fn inventory_families(engine: &Engine) -> InventoryFamilies {
    engine
        .world
        .module_state::<InventoryHandles>()
        .expect("inventory publishes its handles")
        .0
        .clone()
}

/// The store writer a rig drives directly, exactly as `spawn_persist` spawns it.
fn spawn_writer(store: &Arc<dyn ObjectStore>) -> (StoreWriter, tokio::task::JoinHandle<()>) {
    let (signals, _signal_rx) = world_task::writer_signal_channel();
    spawn_store_writer(
        Arc::clone(store),
        signals,
        Arc::new(mush_config::metrics::Metrics::new()),
        Arc::new(AtomicU64::new(0)),
        mushd::quarantine::QuarantineSpool::in_memory(),
        PersistenceIntegrityLatch::new(),
    )
}

/// One inventory verb, persisted the way the world task persists a prepared
/// K state-family action: its replay writes ride an authoritative bundle
/// through the real store writer (the store refuses the legacy lane for
/// `action-state/v1` keys), and the images are installed on the exact receipt.
/// An `Err` from the verb is a test failure, not a silent skip.
async fn apply_inventory<T>(
    engine: &mut Engine,
    writer: &StoreWriter,
    families: &InventoryFamilies,
    step: u8,
    body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> Result<T, InventoryError>,
) -> T {
    let (value, prepared) = {
        let mut txn = engine
            .action_transaction(&families.pools)
            .expect("inventory transaction");
        let value = body(&mut txn).expect("the inventory verb is accepted");
        (value, txn.prepare().expect("the transaction prepares"))
    };
    let owner = prepared.owner().clone();
    let writes = prepared.authoritative_replay_writes();
    assert!(!writes.is_empty(), "a pool write persists something");
    let (_, installation) = prepared.into_parts();

    // The bundle around those exact writes, provided by the families' owner.
    let mut definitions = DefinitionRegistryBuilder::new();
    let action = definitions
        .register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("action").unwrap(),
                DefinitionId::new("round_trip_step").unwrap(),
                1,
                serde_json::json!({}),
            )
            .unwrap(),
        )
        .unwrap();
    let outcome = definitions
        .register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("outcome").unwrap(),
                DefinitionId::new("round_trip_stepped").unwrap(),
                1,
                serde_json::json!({}),
            )
            .unwrap(),
        )
        .unwrap();
    let request = RequestUid::from_bytes([0x40 + step; 16]);
    let principal = PrincipalId::Object { id: GOD };
    let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: CommitToken::from_bytes([0x50 + step; 16]),
        request,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: principal.clone(),
                owner: owner.clone(),
                namespace: CanonicalId::new("round_trip_step").unwrap(),
            },
            0,
            IdempotencyKey::Request(request),
        ),
        ingress_hash: CanonicalHash::from_bytes([0x60 + step; 32]),
        request_hash: CanonicalHash::from_bytes([0x70 + step; 32]),
        plan_hash: CanonicalHash::from_bytes([0x80 + step; 32]),
        action: action.clone(),
        provider: owner,
        cause: PreparedCausalRef::new(request, None, 0).unwrap(),
        definitions: vec![action.clone(), outcome.clone()],
        writes,
        outcome: RulesValue::new(outcome.clone(), serde_json::json!({ "step": step })).unwrap(),
        resolution: None,
        semantic_events: vec![
            AuthoritativeSemanticEvent::root(
                PreparedCausalRef::new(request, None, 0).unwrap(),
                &action,
                RulesValue::new(outcome, serde_json::json!({ "step": step })).unwrap(),
                vec![principal.clone()],
            )
            .unwrap(),
        ],
        eligible_witnesses: vec![principal],
        durable_outbox: vec![],
    })
    .expect("the inventory writes bundle");

    let (ownership, _owned) = tokio::sync::oneshot::channel();
    let (notify, receipt) = tokio::sync::oneshot::channel();
    writer
        .send(StoreJob::Authoritative {
            bundle: Box::new(bundle),
            ownership,
            notify,
        })
        .await
        .expect("the writer is running");
    receipt
        .await
        .expect("the writer answers")
        .expect("the inventory bundle finalizes");
    engine
        .install_prepared_action(installation)
        .expect("the fixture installs");
    value
}

/// Credit a pool and reserve part of it on one boot; a second boot from the
/// store alone answers the same availability and still holds the reservation
/// (E7 cut 2's pool pair on K's `ReservationBook`).
async fn inventory_pool_round_trip(store: Arc<dyn ObjectStore>) {
    let mut engine = boot_engine(&store).await;
    let families = inventory_families(&engine);
    let (writer, join) = spawn_writer(&store);
    let at = EndpointRef(VaultId(7));
    let metals = CanonicalId::new("metals").expect("commodity");
    let holder = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: PrincipalId::Object { id: GOD },
            owner: OwnerId::new("inventory").expect("owner"),
            namespace: CanonicalId::new("round_trip").expect("namespace"),
        },
        0,
        IdempotencyKey::Request(RequestUid::from_bytes([0x44; 16])),
    );
    let stamp = ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at: 1_000,
    };
    apply_inventory(&mut engine, &writer, &families, 1, |txn| {
        families.credit(txn, at, &metals, 100).map(|_| ())
    })
    .await;
    let reservation = apply_inventory(&mut engine, &writer, &families, 2, |txn| {
        families.reserve(txn, at, &metals, &holder, 30, &stamp)
    })
    .await;
    let before = {
        let mut txn = engine
            .action_transaction(&families.pools)
            .expect("inventory transaction");
        families
            .available(&mut txn, at, &metals)
            .expect("available")
    };
    assert_eq!(before, 70, "the hold reduced availability");
    drop(engine);
    drop(writer);
    join.await.expect("the writer drained and exited");

    // --- second boot: rebuild from the store alone. ----------------------
    let mut engine2 = boot_engine(&store).await;
    let families2 = inventory_families(&engine2);
    let mut txn = engine2
        .action_transaction(&families2.pools)
        .expect("inventory transaction");
    assert_eq!(
        families2
            .available(&mut txn, at, &metals)
            .expect("available"),
        70,
        "the pool and its hold did not survive the reboot"
    );
    let record = families2
        .reservation(&mut txn, at, &metals, reservation)
        .expect("the pool pair is readable")
        .expect("the reservation survived the reboot");
    assert_eq!(record.id(), reservation);
}

#[tokio::test(flavor = "multi_thread")]
async fn inventory_pool_round_trip_in_memory() {
    inventory_pool_round_trip(Arc::new(InMemoryStore::new())).await;
}

/// The same round trip over a live Redis. Gated on `REDIS_URL`; runs in db 4.
#[tokio::test(flavor = "multi_thread")]
async fn inventory_pool_round_trip_redis_gated() {
    let Some(url) = std::env::var("REDIS_URL").ok().filter(|s| !s.is_empty()) else {
        eprintln!(
            "skipping inventory_pool_round_trip_redis_gated: REDIS_URL not set \
             (no live Redis on this machine)"
        );
        return;
    };
    let url = support::redis_url_with_db(&url, 4);
    let store = mush_store::RedisStore::connect(&url)
        .await
        .expect("connect to REDIS_URL");
    store.wipe_namespaces().await.expect("wipe test namespaces");
    inventory_pool_round_trip(Arc::new(store)).await;
}

// ---------------------------------------------------------------------------
// H1: the scene archive over the module store-read seam
// (`docs/design/11-h-implementation-plan.md` §3.1). A scene played and closed
// on one boot is read back on the next — when its log tail is no longer in
// RAM — through `+timestop/log/archive` and `+recap/archive`: the world task
// performs the read off-loop against the store and re-enters the typed
// completion. Exercised in-memory (ungated) and against a live Redis.
// ---------------------------------------------------------------------------

async fn scene_archive_round_trip(store: Arc<dyn ObjectStore>) {
    let guard = Arc::new(IpGuard::new());

    // --- first boot: play a short scene and close it. -------------------
    let (sessions, handles) = boot_module_rig(&store, &guard).await;
    let world = handles.handle.clone();
    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;
    god_cmd_committed(&world, "+timestop").await;
    expect_text(&mut god_rx, "a timestop begins").await;
    god_cmd_committed(&world, "say the vault is empty").await;
    god_cmd_committed(&world, "+timestop/end").await;
    expect_text(&mut god_rx, "Time resumes").await;
    // Every prior mutation crossed its exact barrier; player creation is an
    // independently acknowledgement-gated final persistence probe.
    world
        .create_player("Archivist", None)
        .await
        .expect("archivist create");

    // --- second boot: the closed scene's log is on disk only. -----------
    let (sessions2, handles2) = boot_module_rig(&store, &guard).await;
    let world2 = handles2.handle.clone();
    let mut god_rx2 = attach_direct(&sessions2, &world2, 2, GOD, "god").await;
    god_cmd(&world2, "+timestop/log").await;
    expect_text(&mut god_rx2, "+timestop/log/archive").await;
    god_cmd(&world2, "+timestop/log/archive").await;
    expect_text(&mut god_rx2, "Reading page 0").await;
    // The read ran off the world loop and re-entered as the completion.
    expect_text(&mut god_rx2, "archive page 0").await;
    expect_text(&mut god_rx2, "the vault is empty").await;
    god_cmd(&world2, "+recap/archive").await;
    expect_text(&mut god_rx2, "as you witnessed it").await;
    expect_text(&mut god_rx2, "the vault is empty").await;
    // The world stayed live throughout.
    god_cmd(&world2, "WHO").await;
    expect_text(&mut god_rx2, "God").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn scene_archive_reads_cold_chunks_in_memory() {
    scene_archive_round_trip(Arc::new(InMemoryStore::new())).await;
}

/// The same round trip over a live Redis: the reboot-from-Redis-alone
/// surrogate for the archive. Gated on `REDIS_URL`; runs in db 5 (dbs 6–15
/// are claimed by the other gated suites, which run in parallel).
#[tokio::test(flavor = "multi_thread")]
async fn scene_archive_reads_cold_chunks_redis_gated() {
    let Some(url) = std::env::var("REDIS_URL").ok().filter(|s| !s.is_empty()) else {
        eprintln!(
            "skipping scene_archive_reads_cold_chunks_redis_gated: REDIS_URL not set \
             (no live Redis on this machine)"
        );
        return;
    };
    let url = support::redis_url_with_db(&url, 5);
    let store = mush_store::RedisStore::connect(&url)
        .await
        .expect("connect to REDIS_URL");
    store.wipe_namespaces().await.expect("wipe test namespaces");
    scene_archive_round_trip(Arc::new(store)).await;
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
// M9 onboarding persistence twin: a submitted chargen sheet is a `sheet:tacsim`
// attribute on the player's obj doc (treaty ruling 7), so it rides obj:*
// write-through; the `pilot-chargen` approval JOB is a `jobs:*` module doc. This
// proves BOTH the Pending sheet AND — after staff approval on a rebooted
// server — the Approved sheet survive a reboot-from-store-alone. Exercised
// in-memory (ungated) and against a live Redis.
//
// 16b cut R-b: the journey used to be 45 guided-wizard lines fed into an input
// context. The game's chargen has no wizard and no session — the in-progress
// sheet IS the state — so the same journey is now seven ordinary commands, and
// what is being persisted is the same document either way.
// ---------------------------------------------------------------------------

/// The command sequence that spends the six-point pool exactly, names the
/// pilot, and files the sheet (the same allocation `onboarding_e2e` drives over
/// SSH): gunnery 4 / piloting 3 / sensors 2 is `3 + 2 + 1 == 6` dots above the
/// free one every rating starts at.
const PILOT_CHARGEN: &[&str] = &[
    "+chargen",
    "+stat name=Ada Renard",
    "+stat callsign=Ladder",
    "+stat gunnery=4",
    "+stat piloting=3",
    "+stat sensors=2",
    "+chargen/submit",
];

/// Extract the job id from a `Submitted for approval as job N.` line.
fn parse_job_id(line: &str) -> u64 {
    let idx = line.find("as job ").expect("submit line names a job");
    let tail = &line[idx + "as job ".len()..];
    tail.chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .expect("job id parses")
}

/// Boot a rig with the jobs + character-ruleset modules, `chargen_required` on
/// and both start rooms = Limbo (the distinct-room approval move is covered
/// ungated by the ruleset's own onboarding suite and by `onboarding_e2e`; this
/// twin isolates sheet + approval *persistence*).
async fn boot_onboarding_rig(
    store: &Arc<dyn ObjectStore>,
    guard: &Arc<IpGuard>,
) -> (Arc<SessionManager>, PersistHandles) {
    let world = load_or_seed_world(&**store).await.expect("boot world");
    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff).unwrap();
    let onboarding = mush_core::OnboardingConfig {
        chargen_required: true,
        start_room: LIMBO,
        start_room_unapproved: LIMBO,
    };
    let mut mods: Vec<Box<dyn mush_core::GameModule>> =
        vec![Box::new(mush_jobs::JobsModule::new())];
    mods.extend(mushd::modules::sheet_modules(
        &onboarding,
        &Default::default(),
    ));
    let mut engine = Engine::with_modules(world, registry, mods).expect("module registration");
    mushd::modules::boot_modules(&mut engine, &**store)
        .await
        .expect("module boot");
    let sessions = Arc::new(SessionManager::new());
    let handles = world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        ServerDeps {
            metrics: std::sync::Arc::new(mush_net_web::Metrics::new()),
            store: Arc::clone(store),
            guard: Arc::clone(guard),
            staff,
            llm: None,
            quarantine_spool: mushd::quarantine::QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
        },
    );
    (sessions, handles)
}

/// Read `who`'s sheet from a bare-reloaded world (no task) — the
/// reboot-from-store-alone assertion path.
fn reloaded_sheet(world: &mush_core::World, who: ObjId) -> mush_ruleset::SheetDoc {
    let attr = world
        .attr(who, "sheet:tacsim")
        .and_then(mush_core::Attr::as_json)
        .expect("a persisted sheet:tacsim attr");
    serde_json::from_value(attr.clone()).expect("sheet decodes")
}

async fn onboarding_round_trip(store: Arc<dyn ObjectStore>) {
    let guard = Arc::new(IpGuard::new());

    // --- first boot: a new player is greeted into chargen, builds a ------
    // --- pilot, and submits it for approval. -----------------------------
    let (sessions, handles) = boot_onboarding_rig(&store, &guard).await;
    let world = handles.handle.clone();
    // The ack-gated create fires the ObjCreated observer (placed in the chargen
    // area, = Limbo here) and lands the creation batch in the store.
    let nick = world
        .create_player("Nick", None)
        .await
        .expect("create Nick");
    // Connecting greets Nick and points at +chargen (the Connected hook).
    let mut nick_rx = attach_direct(&sessions, &world, 2, nick, "nick").await;
    assert!(
        world.barrier().await,
        "onboarding connect-hook commit barrier"
    );
    let mut submitted = None;
    for line in PILOT_CHARGEN {
        assert!(
            world
                .inbound(Inbound::Cmd {
                    conn: 2,
                    actor: nick,
                    line: (*line).into(),
                })
                .await
        );
        assert!(world.barrier().await, "chargen mutation commit barrier");
        if *line == "+chargen/submit" {
            submitted = Some(expect_text(&mut nick_rx, "Submitted for approval as job").await);
        } else {
            // The exact barrier makes this command's prompt/output observable;
            // consume it now so the bounded fake client is not mistaken for a
            // slow connection during the journey.
            while nick_rx.try_recv().is_ok() {}
        }
    }
    let ev = submitted.expect("chargen submit acknowledgement");
    let job = parse_job_id(&text_of(&ev));
    // Every chargen action crossed its exact barrier; creation independently
    // proves the world remains writable after the submitted sheet/job commit.
    world
        .create_player("Fence", None)
        .await
        .expect("fence create");

    // --- reboot #1 (store alone): the sheet is Pending, unapproved. -------
    let reloaded = load_or_seed_world(&*store).await.expect("reload 1");
    let doc = reloaded_sheet(&reloaded, nick);
    assert!(
        matches!(doc.approved, mush_ruleset::ApprovalState::Pending(j) if j == job),
        "the submitted sheet survives a reboot still Pending (job {job}): {:?}",
        doc.approved
    );
    assert_eq!(
        doc.text("callsign"),
        Some("Ladder"),
        "sheet content (the identity block) survives the reboot"
    );
    assert_eq!(doc.number("gunnery"), Some(4), "the bought ratings survive");
    assert_eq!(doc.number("piloting"), Some(3));
    assert_eq!(doc.number("sensors"), Some(2));

    // --- reboot #2 (task): staff approve the persisted job. --------------
    let (sessions2, handles2) = boot_onboarding_rig(&store, &guard).await;
    let world2 = handles2.handle.clone();
    let mut god_rx = attach_direct(&sessions2, &world2, 3, GOD, "god").await;
    god_cmd(&world2, &format!("+job/approve {job}=Welcome aboard.")).await;
    expect_text(&mut god_rx, "approved").await;
    // Fence the approval's sheet-stamp write.
    world2
        .create_player("Fence2", None)
        .await
        .expect("fence2 create");

    // --- reboot #3 (store alone): the approval survived. -----------------
    let reloaded2 = load_or_seed_world(&*store).await.expect("reload 2");
    let doc2 = reloaded_sheet(&reloaded2, nick);
    assert!(
        doc2.approved.is_approved(),
        "the approval survives a reboot: {:?}",
        doc2.approved
    );
}

/// The onboarding twin over an in-memory store — ungated, so CI proves the
/// sheet-attr + job-doc reload path without a live Redis.
#[tokio::test(flavor = "multi_thread")]
async fn onboarding_persistence_round_trip_in_memory() {
    onboarding_round_trip(Arc::new(InMemoryStore::new())).await;
}

/// The onboarding twin over a live Redis — reboot-from-Redis-alone. Gated on
/// `REDIS_URL`; runs in db 7 (dbs 8–15 and 0 are claimed by the other gated
/// suites, which run in parallel).
#[tokio::test(flavor = "multi_thread")]
async fn onboarding_persistence_round_trip_redis_gated() {
    let Some(url) = std::env::var("REDIS_URL").ok().filter(|s| !s.is_empty()) else {
        eprintln!(
            "skipping onboarding_persistence_round_trip_redis_gated: REDIS_URL not set \
             (no live Redis on this machine)"
        );
        return;
    };
    let url = support::redis_url_with_db(&url, 7);
    let store = mush_store::RedisStore::connect(&url)
        .await
        .expect("connect to REDIS_URL");
    store.wipe_namespaces().await.expect("wipe test namespaces");
    onboarding_round_trip(Arc::new(store)).await;
}

// ---------------------------------------------------------------------------
// M7 worldgen persistence twin: a zone THEME is a plain object attribute
// (`worldgen.theme`, treaty ruling 8), so it rides obj:* write-through; a
// `@gen` DRAFT is a `worldgen:*` module doc, so it rides the module-op
// write-through and is reloaded by `boot_modules` -> `on_world_loaded`. This
// proves BOTH survive a reboot-from-store-alone and the reloaded draft still
// accepts. The LLM is a network-free MockProvider injected as the world task's
// service (never a real adapter/key) — the real async defer -> re-entry path.
// ---------------------------------------------------------------------------

/// A `worldgen_room` route the mock serves (any non-`anthropic/` model id
/// derives to the `openrouter` provider the mock registers under).
fn wg_service(mock: &Arc<MockProvider>) -> Arc<LlmService> {
    let concrete = Arc::clone(mock);
    let p: Arc<dyn LlmProvider> = concrete;
    Arc::new(
        LlmService::builder()
            .route(
                "worldgen_room",
                RouteConfig {
                    model: "z-ai/glm-5.3".into(),
                    max_tokens: 1500,
                    timeout_s: 60,
                    reasoning: Some("low".into()),
                    reasoning_max_tokens: None,
                },
            )
            .provider("openrouter", p)
            .global_enabled(true)
            .build(),
    )
}

/// A canned `submit_room` tool call carrying `desc`.
fn wg_tool_resp(desc: &str) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::ToolUse {
            id: "t1".into(),
            name: "submit_room".into(),
            input: serde_json::json!({ "desc": desc }),
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage {
            input_tokens: 120,
            output_tokens: 60,
            ..Usage::default()
        },
        model: "test/mock".into(),
    }
}

/// Boot a full server rig with the built-in modules **plus the worldgen
/// module** (backed by `llm`) and that same service on the world task, so a
/// `@gen`'s deferred `Effect::ModuleAsync` runs off-loop through
/// `run_worldgen_async` and re-enters — exactly as `main.rs` wires it.
async fn boot_worldgen_rig(
    store: &Arc<dyn ObjectStore>,
    guard: &Arc<IpGuard>,
    llm: Arc<LlmService>,
) -> (Arc<SessionManager>, PersistHandles) {
    let world = load_or_seed_world(&**store).await.expect("boot world");
    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff).unwrap();
    let mut mods = mushd::modules::server_modules();
    mods.push(mushd::modules::worldgen_module(Arc::clone(&llm)));
    let mut engine =
        Engine::with_modules(world, registry, mods).expect("module + worldgen registration");
    mushd::modules::boot_modules(&mut engine, &**store)
        .await
        .expect("module boot");
    let sessions = Arc::new(SessionManager::new());
    let handles = world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        ServerDeps {
            metrics: std::sync::Arc::new(mush_net_web::Metrics::new()),
            store: Arc::clone(store),
            guard: Arc::clone(guard),
            staff,
            llm: Some(llm),
            quarantine_spool: mushd::quarantine::QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
        },
    );
    (sessions, handles)
}

/// Set a zone theme, `@gen` a room draft (async), fence so both writes land,
/// then reboot from the store alone and assert the theme resolves and the
/// reloaded draft still accepts. Ungated + Redis-gated below.
async fn worldgen_round_trip(store: Arc<dyn ObjectStore>) {
    const DRAFT_DESC: &str =
        "A wide plaza survives the reboot, its dry fountain choked with leaves.";
    const THEME_STYLE: &str = "sun-bleached art-deco boardwalk";
    let guard = Arc::new(IpGuard::new());

    // --- first boot: theme a zone, generate + persist a draft. -----------
    let mock = Arc::new(MockProvider::new("openrouter"));
    mock.push_response(wg_tool_resp(DRAFT_DESC));
    let llm = wg_service(&mock);
    let (sessions, handles) = boot_worldgen_rig(&store, &guard, Arc::clone(&llm)).await;
    let world = handles.handle.clone();
    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;

    // The world theme (S2b: an atlas document, `mod:atlas:theme:world`).
    god_cmd_committed(&world, "@create Downtown").await;
    god_cmd_committed(&world, "@set Downtown=ZONE_MASTER").await;
    god_cmd_committed(
        &world,
        &format!("@atlas/theme world=style_guide:{THEME_STYLE}"),
    )
    .await;
    expect_text(&mut god_rx, "Theme style_guide set").await;

    // A room in the zone (teleport in so `here` resolves), then @gen it — the
    // MockProvider returns the canned draft through the real async path.
    god_cmd_committed(&world, "@dig/teleport Plaza").await;
    god_cmd_committed(&world, "@zone here=Downtown").await;
    god_cmd(&world, "@gen here=A wide plaza with a dry fountain.").await;
    // "Draft ready" is delivered in the same world-loop iteration that queues
    // the draft's persist batch, so seeing it means the batch is queued.
    expect_text(&mut god_rx, "Draft ready").await;

    // Fence: an ack-gated create returns only once the strictly-ordered writer
    // has acknowledged its batch — so the draft's fire-and-forget batch (queued
    // before it) has certainly landed in the store.
    world
        .create_player("Fence", None)
        .await
        .expect("fence create");

    // --- second boot: rebuild from the store alone. ----------------------
    let mock2 = Arc::new(MockProvider::new("openrouter"));
    let (sessions2, handles2) = boot_worldgen_rig(&store, &guard, wg_service(&mock2)).await;
    let world2 = handles2.handle.clone();
    let mut god_rx2 = attach_direct(&sessions2, &world2, 2, GOD, "god").await;

    // The theme survived (the atlas's world theme document): it resolves for the
    // reloaded Plaza, where God still stands (his teleport persisted).
    god_cmd(&world2, "@atlas/theme/show here").await;
    let ev = expect_text(&mut god_rx2, THEME_STYLE).await;
    assert!(
        text_of(&ev).contains(THEME_STYLE),
        "reboot must resolve the persisted zone theme"
    );

    // The draft survived (a worldgen module doc reloaded by boot_modules ->
    // on_world_loaded): accepting it commits the desc with NO new LLM call.
    god_cmd(&world2, "@gen/accept").await;
    expect_text(&mut god_rx2, "Accepted").await;
    god_cmd(&world2, "look").await;
    expect_text(&mut god_rx2, "survives the reboot").await;
    assert_eq!(
        mock2.call_count(),
        0,
        "reboot accept must not call the provider"
    );
}

/// The worldgen round trip over an in-memory store — ungated, so CI proves the
/// theme (obj attr) + draft (module doc) write-through and reload wiring
/// without a live Redis.
#[tokio::test(flavor = "multi_thread")]
async fn worldgen_persistence_round_trip_in_memory() {
    worldgen_round_trip(Arc::new(InMemoryStore::new())).await;
}

/// The same round trip over a live Redis — theme obj docs and worldgen module
/// docs survive a reboot from Redis alone. Gated on `REDIS_URL`; runs in db 9
/// (dbs 10–15 are claimed by the other gated tests, which run in parallel).
#[tokio::test(flavor = "multi_thread")]
async fn worldgen_persistence_round_trip_redis_gated() {
    let Some(url) = std::env::var("REDIS_URL").ok().filter(|s| !s.is_empty()) else {
        eprintln!(
            "skipping worldgen_persistence_round_trip_redis_gated: REDIS_URL not set \
             (no live Redis on this machine)"
        );
        return;
    };
    let url = support::redis_url_with_db(&url, 9);
    let store = mush_store::RedisStore::connect(&url)
        .await
        .expect("connect to REDIS_URL");
    store.wipe_namespaces().await.expect("wipe test namespaces");
    worldgen_round_trip(Arc::new(store)).await;
}

// ---------------------------------------------------------------------------
// M6a sheet-persistence twin: a character sheet is an object attribute (treaty
// ruling 7), so it rides the SAME obj write-through as any @set and reloads
// with the world — no module namespace, no separate boot load. This proves
// the reboot round trip and that the reloaded sheet still validates.
//
// 16b cut R-b: there used to be a second twin below this one that built a
// Vampire, a Mage, a Werewolf, a Changeling, a Ghoul and a Kinfolk, rebooted,
// and re-validated each against the per-template schema its own `template`
// field selected. The game has ONE schema and no template dispatch, so the six
// per-template cases have no subject left and are gone; the claim they shared
// with this twin — the sheet is an ordinary attr that survives a bare store
// reload and still validates — is proved once, here, over the one schema.
// ---------------------------------------------------------------------------

/// Boot a full server rig with the built-in modules **plus the game's character
/// ruleset** (the composition `mush_server::boot::build_engine` makes).
async fn boot_sheet_rig(
    store: &Arc<dyn ObjectStore>,
    guard: &Arc<IpGuard>,
) -> (Arc<SessionManager>, PersistHandles) {
    let world = load_or_seed_world(&**store).await.expect("boot world");
    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff).unwrap();
    let mut mods = mushd::modules::server_modules();
    mods.extend(mushd::modules::sheet_modules(
        &mush_core::OnboardingConfig::default(),
        &Default::default(),
    ));
    let mut engine =
        Engine::with_modules(world, registry, mods).expect("module + ruleset registration");
    mushd::modules::boot_modules(&mut engine, &**store)
        .await
        .expect("module boot");
    let sessions = Arc::new(SessionManager::new());
    let handles = world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        ServerDeps {
            metrics: std::sync::Arc::new(mush_net_web::Metrics::new()),
            store: Arc::clone(store),
            guard: Arc::clone(guard),
            staff,
            llm: None,
            quarantine_spool: mushd::quarantine::QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
        },
    );
    (sessions, handles)
}

/// Build a character's sheet through the live `@sheet/set` command path, fence
/// on an ack-gated create so every sheet write has landed, then reboot from the
/// store alone and assert the sheet survived, holds its values, and still
/// validates against the ruleset schema. Every section of the sheet is written
/// — the free-text identity block, the rating dots, and a staff-only standing
/// field a player could never set — so the reload covers each field TYPE the
/// schema declares, which is what the six deleted per-template cases were
/// really exercising. Ungated + Redis-gated below.
async fn sheet_round_trip(store: Arc<dyn ObjectStore>) {
    use mush_ruleset::{Ruleset, SheetDoc};

    let guard = Arc::new(IpGuard::new());

    // --- first boot: create a character, staff-build their sheet. ---------
    let (sessions, handles) = boot_sheet_rig(&store, &guard).await;
    let world = handles.handle.clone();
    let _god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;

    let nick = world
        .create_player("Nick", None)
        .await
        .expect("create Nick");
    // Staff-edit the sheet (@sheet/set is Royalty-gated; God qualifies). The
    // sheet is an attr on Nick's object, so these writes ride obj:* through.
    god_cmd_committed(&world, "@sheet/set Nick/callsign=Ladder").await;
    god_cmd_committed(&world, "@sheet/set Nick/gunnery=4").await;
    god_cmd_committed(&world, "@sheet/set Nick/piloting=3").await;
    god_cmd_committed(&world, "@sheet/set Nick/reputation=2").await;
    god_cmd_committed(&world, "@sheet/set Nick/certifications:Lance Lead=veteran").await;

    // Every sheet edit crossed its exact barrier; creation independently
    // verifies the world remains writable afterward.
    world
        .create_player("Fence", None)
        .await
        .expect("fence create");

    // --- reboot: hydrate a fresh world from the store alone. --------------
    let reloaded = load_or_seed_world(&*store).await.expect("reload");
    let attr = reloaded
        .attr(nick, "sheet:tacsim")
        .and_then(mush_core::Attr::as_json)
        .expect("the sheet attribute survived the reboot");
    let doc: SheetDoc = serde_json::from_value(attr.clone()).expect("sheet decodes");
    assert_eq!(doc.text("callsign"), Some("Ladder"), "callsign survived");
    assert_eq!(doc.number("gunnery"), Some(4), "gunnery survived");
    assert_eq!(doc.number("piloting"), Some(3), "piloting survived");
    assert_eq!(
        doc.number("sensors"),
        Some(1),
        "an untouched rating reloads at its schema default, not absent"
    );
    assert_eq!(doc.number("reputation"), Some(2), "reputation survived");
    assert_eq!(
        doc.map("certifications").expect("the map field survived")["Lance Lead"],
        serde_json::json!("veteran")
    );

    // And the reloaded sheet still validates against the game's schema.
    let ruleset = tacsim::sheet::PilotRuleset::new();
    mush_ruleset::validate(ruleset.sheet_schema(), &doc.data)
        .expect("reloaded sheet must validate against the ruleset schema");

    // The first world task + writer are reaped when the runtime shuts down
    // at test end (as the other reboot twins here rely on).
    drop(handles);
}

/// The sheet-persistence twin over an in-memory store — ungated, so CI proves
/// the reboot round trip without a live Redis.
#[tokio::test(flavor = "multi_thread")]
async fn sheet_persistence_round_trip_in_memory() {
    sheet_round_trip(Arc::new(InMemoryStore::new())).await;
}

/// The same twin over a live Redis — the reboot-from-Redis-alone surrogate.
/// Gated on `REDIS_URL`; runs in db 11 (dbs 12–15 are claimed by the other
/// gated tests, which run in parallel).
#[tokio::test(flavor = "multi_thread")]
async fn sheet_persistence_round_trip_redis_gated() {
    let Some(url) = std::env::var("REDIS_URL").ok().filter(|s| !s.is_empty()) else {
        eprintln!(
            "skipping sheet_persistence_round_trip_redis_gated: REDIS_URL not set \
             (no live Redis on this machine)"
        );
        return;
    };
    let url = support::redis_url_with_db(&url, 11);
    let store = mush_store::RedisStore::connect(&url)
        .await
        .expect("connect to REDIS_URL");
    store.wipe_namespaces().await.expect("wipe test namespaces");
    sheet_round_trip(Arc::new(store)).await;
}

// ---------------------------------------------------------------------------
// A store wrapper with injectable behavior (gate / fail), for the ack and
// read-only tests. Only `write` misbehaves; everything else delegates.
// ---------------------------------------------------------------------------

const MODE_NORMAL: u8 = 0;
const MODE_HOLD: u8 = 1;
const MODE_FAIL: u8 = 2;
const MODE_CLOSED: u8 = 3;
const MODE_WRONG_RECEIPT: u8 = 4;

struct FlakyStore {
    inner: InMemoryStore,
    mode: AtomicU8,
    /// WS07-S8: every snapshot this store is asked for, counted — so a test
    /// can say that a shutdown tail took none.
    bgsaves: AtomicUsize,
}

impl FlakyStore {
    fn new() -> FlakyStore {
        FlakyStore {
            inner: InMemoryStore::new(),
            mode: AtomicU8::new(MODE_NORMAL),
            bgsaves: AtomicUsize::new(0),
        }
    }

    fn set_mode(&self, mode: u8) {
        self.mode.store(mode, Ordering::SeqCst);
    }

    fn bgsaves(&self) -> usize {
        self.bgsaves.load(Ordering::SeqCst)
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
                    MODE_WRONG_RECEIPT => {
                        let receipt = self.inner.write(batch).await?;
                        return Ok(WriteReceipt {
                            serial: receipt.serial.saturating_add(1),
                        });
                    }
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

    fn set_account_ssh_keys<'a>(
        &'a self,
        account: &'a str,
        keys: &'a [mush_store::SshKeyRec],
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.set_account_ssh_keys(account, keys)
    }

    fn set_account_discord<'a>(
        &'a self,
        account: &'a str,
        link: Option<&'a mush_store::DiscordLinkRec>,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.set_account_discord(account, link)
    }

    fn discord_user_reserve<'a>(
        &'a self,
        user_id: u64,
        account: &'a str,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        self.inner.discord_user_reserve(user_id, account)
    }

    fn discord_user_release(&self, user_id: u64) -> BoxFuture<'_, Result<bool, StoreError>> {
        self.inner.discord_user_release(user_id)
    }

    fn account_by_discord_user(
        &self,
        user_id: u64,
    ) -> BoxFuture<'_, Result<Option<mush_store::AccountRec>, StoreError>> {
        self.inner.account_by_discord_user(user_id)
    }

    fn discord_code_put(
        &self,
        hash: String,
        rec: mush_store::DiscordCodeRecord,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.discord_code_put(hash, rec)
    }

    fn discord_code_take<'a>(
        &'a self,
        hash: &'a str,
    ) -> BoxFuture<'a, Result<Option<mush_store::DiscordCodeRecord>, StoreError>> {
        self.inner.discord_code_take(hash)
    }

    fn discord_ban_put(
        &self,
        rec: mush_store::DiscordBanRec,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.discord_ban_put(rec)
    }

    fn discord_ban_get(
        &self,
        user_id: u64,
    ) -> BoxFuture<'_, Result<Option<mush_store::DiscordBanRec>, StoreError>> {
        self.inner.discord_ban_get(user_id)
    }

    fn discord_ban_del(&self, user_id: u64) -> BoxFuture<'_, Result<bool, StoreError>> {
        self.inner.discord_ban_del(user_id)
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

    fn put_tempban_lift(
        &self,
        lift: mush_store::TempbanLift,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.put_tempban_lift(lift)
    }
    fn get_tempban_lifts(&self) -> BoxFuture<'_, Result<Vec<mush_store::TempbanLift>, StoreError>> {
        self.inner.get_tempban_lifts()
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
        self.bgsaves.fetch_add(1, Ordering::SeqCst);
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

/// Ordinary mutation presentation is governed by the same receipt boundary as
/// typed account creation. While the write is held, gameplay/world reads get a
/// typed Busy refusal (never tentative RAM and never a false negative), and
/// neither the success line nor a later barrier may overtake it.
///
/// **16b cut B.5**: the `who` query and the `name_free` read now wait in the
/// world's defer queue first and are refused only when the held write outlives
/// its 250 ms bound — with exactly today's refusals (the query's typed reply
/// wraps the `CommitPending` sentence, the read answers `Busy`), because only a
/// typed line hears the new timed-out sentence. The one-second bound on the
/// read covers that wait.
#[tokio::test(flavor = "multi_thread")]
async fn ordinary_success_and_barrier_wait_for_exact_store_ack() {
    let flaky = Arc::new(FlakyStore::new());
    let store: Arc<dyn ObjectStore> = Arc::clone(&flaky) as Arc<dyn ObjectStore>;
    let guard = Arc::new(IpGuard::new());
    let (sessions, handles) = boot_rig(&store, &guard).await;
    let world = handles.handle.clone();
    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;
    assert!(world.barrier().await, "connection setup barrier");

    flaky.set_mode(MODE_HOLD);
    god_cmd(&world, "@create held-orb").await;
    let barrier_world = world.clone();
    let barrier = tokio::spawn(async move { barrier_world.barrier().await });

    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(
        !barrier.is_finished(),
        "barrier completed before the prior write's exact receipt"
    );
    assert!(
        world
            .inbound(Inbound::Query {
                conn: 1,
                actor: GOD,
                id: 991,
                name: "who".into(),
                args: serde_json::json!({}),
            })
            .await
    );
    expect_text(&mut god_rx, "committing another action").await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), world.name_free("Probe"))
            .await
            .expect("world read receives a bounded response"),
        Err(WorldReadError::Busy),
        "a pending commit must not expose tentative world state or invent a name answer"
    );
    while let Ok(event) = god_rx.try_recv() {
        assert!(
            !text_of(&event).contains("Created: held-orb"),
            "success presentation escaped before durable acknowledgement"
        );
    }

    flaky.set_mode(MODE_NORMAL);
    expect_text(&mut god_rx, "Created: held-orb").await;
    assert!(
        barrier.await.expect("barrier task joins"),
        "barrier completes after output release"
    );
    assert_eq!(
        world.name_free("Probe").await,
        Ok(true),
        "world reads resume after the exact receipt"
    );
    let snapshot = store.load_world().await.unwrap().expect("seeded world");
    assert!(snapshot.objects.iter().any(|o| o.name == "held-orb"));
}

/// A store receipt is an exact token, not a generic "some write happened"
/// acknowledgement. Even if the backend applied the bytes, returning another
/// serial aborts presentation/effects and integrity-freezes the world.
#[tokio::test(flavor = "multi_thread")]
async fn wrong_receipt_serial_never_releases_success() {
    let flaky = Arc::new(FlakyStore::new());
    let store: Arc<dyn ObjectStore> = Arc::clone(&flaky) as Arc<dyn ObjectStore>;
    let guard = Arc::new(IpGuard::new());
    let (sessions, handles) = boot_rig(&store, &guard).await;
    let world = handles.handle.clone();
    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;
    assert!(world.barrier().await, "connection setup barrier");

    flaky.set_mode(MODE_WRONG_RECEIPT);
    god_cmd(&world, "@create false-ack-orb").await;

    let saw_created = tokio::time::timeout(Duration::from_secs(10), async {
        let mut leaked = false;
        loop {
            let event = god_rx.recv().await.expect("wizard connection remains open");
            let text = text_of(&event);
            leaked |= text.contains("Created: false-ack-orb");
            if text.contains("success output and external effects were suppressed") {
                return leaked;
            }
        }
    })
    .await
    .expect("bounded integrity-failure response");
    assert!(
        !saw_created,
        "a mismatched receipt released commit-bound success presentation"
    );
    assert!(
        world.barrier().await,
        "failure release precedes the barrier"
    );

    god_cmd(&world, "@create compounded-divergence").await;
    expect_text(&mut god_rx, "integrity-frozen").await;
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

    // The exact failed write remains in writer custody and keeps the world in
    // CommitPending until it can retry. Every ordinary command is refused
    // against that tentative RAM image, including otherwise read-only lines.
    // 16b cut B.5: each waits in the defer queue first, and the failing write
    // outlives the 250 ms bound, so each is refused in the timed-out words.
    god_cmd(&world, "@create another-orb").await;
    expect_text(&mut god_rx, "busy too long for that line").await;
    god_cmd(&world, "look").await;
    expect_text(&mut god_rx, "busy too long for that line").await;
    god_cmd(&world, "say still here").await;
    expect_text(&mut god_rx, "busy too long for that line").await;

    // Store comes back: the writer's retry succeeds (backoff <= 5s), the
    // world announces recovery, and mutations flow again — including the
    // batch that was stuck, which must land in the store.
    flaky.set_mode(MODE_NORMAL);
    expect_text(&mut god_rx, "store writes recovered").await;
    god_cmd_committed(&world, "@create recovery-orb").await;
    expect_text(&mut god_rx, "Created: recovery-orb").await;

    // Everything (including the once-stuck doomed-orb batch) is persisted.
    let nick = world.create_player("Probe", None).await.unwrap(); // ordering barrier
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
    let result = handles.handle.create_player("Ghost", None).await;
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

/// A transiently failed mutation leaves tentative RAM behind an exact pending
/// receipt. Exit aliases, reads, and typed creation must all receive Busy until
/// that predecessor commits; none may inspect or extend the tentative image.
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

    // Break the store; a mutation trips the writer health signal while its
    // exact receipt remains pending.
    flaky.set_mode(MODE_FAIL);
    god_cmd(&world, "@create tripwire").await;
    expect_text(&mut god_rx, "store write failure").await;

    // "l" resolves to the exit, not to `look` — refused behind the pending
    // commit. 16b cut B.5: it waits in the defer queue first and the failing
    // write outlives the 250 ms bound, so it is refused in the timed-out words.
    god_cmd(&world, "l").await;
    expect_text(&mut god_rx, "busy too long for that line").await;
    // Plain `look` is also refused: it may not observe tentative RAM.
    god_cmd(&world, "look").await;
    expect_text(&mut god_rx, "busy too long for that line").await;

    // Typed creation gets the equivalent retryable Busy result.
    let err = world.create_player("Nope", None).await.unwrap_err();
    assert_eq!(
        err,
        mushd::world_task::CreatePlayerErr::Busy,
        "CreatePlayer must be refused while the predecessor receipt is pending"
    );

    // Recovery lifts the gate: the same "l" now traverses the exit.
    flaky.set_mode(MODE_NORMAL);
    expect_text(&mut god_rx, "store writes recovered").await;
    // StoreStatus(true) and the exact receipt use separate control lanes; the
    // advisory recovery notice may arrive first. Cross the receipt fence
    // before asserting that ordinary input is admitted again.
    assert!(
        world.barrier().await,
        "recovery notice must be followed by the exact commit receipt"
    );
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
    let nick = world.create_player("Nick", None).await.unwrap();
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

/// The all-in-one keeps the SINGLE shared guard: `@siteban` mirrors into the
/// very `IpGuard` the listeners consult, so it bites instantly and needs no
/// refresh at all. The forked gateway does need one (its guard is a different
/// object over the same store), and the reload it runs must be safely
/// repeatable — `load_sitebans` replaces the mirror wholesale, so running it
/// over and over neither multiplies entries nor un-bans anything.
#[tokio::test(flavor = "multi_thread")]
async fn repeated_ban_reloads_leave_the_shared_guard_unchanged() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let guard = Arc::new(IpGuard::new());
    let (sessions, handles) = boot_rig(&store, &guard).await;
    let world = handles.handle.clone();
    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;
    let banned: IpAddr = "10.9.8.7".parse().unwrap();

    god_cmd(&world, "@siteban 10.9.8.7=1h flooding the login").await;
    expect_text(&mut god_rx, "@siteban: 10.9.8.7 banned until unix").await;
    // In-process: one guard, no seam, no latency — the mirror is already right.
    assert!(guard.is_blocked(banned));
    assert_eq!(guard.siteban_count(), 1);

    // The gateway's refresh step, run five times over the same store.
    for _ in 0..5 {
        let bans = store.get_bans().await.unwrap();
        guard.load_sitebans(
            bans.into_iter()
                .filter_map(|b| b.ip.parse().ok().map(|ip| (ip, b.until_unix))),
        );
        assert_eq!(
            guard.siteban_count(),
            1,
            "a wholesale reload must not multiply mirror entries"
        );
        assert!(guard.is_blocked(banned), "a reload must not un-ban");
    }

    // And a clear survives the same treatment.
    god_cmd(&world, "@siteban/clear 10.9.8.7").await;
    expect_text(&mut god_rx, "@siteban: ban on 10.9.8.7 lifted.").await;
    for _ in 0..3 {
        let bans = store.get_bans().await.unwrap();
        guard.load_sitebans(
            bans.into_iter()
                .filter_map(|b| b.ip.parse().ok().map(|ip| (ip, b.until_unix))),
        );
        assert_eq!(guard.siteban_count(), 0);
        assert!(!guard.is_blocked(banned));
    }
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

// ---------------------------------------------------------------------------
// 6. M8 NPC agent persistence twin: an NPC's staff profile is an obj attr, its
// rolling summary is an obj attr, and its durable `remember` memories are
// `mod:npc:mem:*` module docs. All three must survive a reboot from the store
// alone. The Remember + Summary are applied through the REAL completion path
// (an injected `WorldCommand::ModuleAsyncDone` — the same typed re-entry the
// off-loop turn produces), so the module-op + attr write-through is exercised
// end to end; no LLM/network is involved.
// ---------------------------------------------------------------------------

/// A never-called `npc_dialogue_major` mock service (the twin injects
/// completions directly, so the provider is never invoked — but the world task
/// still wires a service the way `main.rs` does).
fn npc_service() -> Arc<LlmService> {
    let mock = Arc::new(MockProvider::new("openrouter"));
    let p: Arc<dyn LlmProvider> = mock;
    Arc::new(
        LlmService::builder()
            .route(
                "npc_dialogue_major",
                RouteConfig {
                    model: "z-ai/glm-5.3".into(),
                    max_tokens: 1024,
                    timeout_s: 30,
                    reasoning: Some("low".into()),
                    reasoning_max_tokens: None,
                },
            )
            .provider("openrouter", p)
            .global_enabled(true)
            .build(),
    )
}

/// Boot the full production module set with the `npc` module (grounded by the
/// game's sheet) and that character ruleset — exactly as `main.rs` assembles
/// them — plus the service on the world task.
async fn boot_npc_rig(
    store: &Arc<dyn ObjectStore>,
    guard: &Arc<IpGuard>,
    llm: Arc<LlmService>,
) -> (Arc<SessionManager>, PersistHandles) {
    let world = load_or_seed_world(&**store).await.expect("boot world");
    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff).unwrap();
    let mut mods = mushd::modules::server_modules();
    mods.push(mushd::modules::worldgen_module(Arc::clone(&llm)));
    mods.push(mushd::modules::npc_module(
        mushd::modules::npc_agent_adapter(),
    ));
    mods.extend(mushd::modules::sheet_modules(
        &mush_core::OnboardingConfig::default(),
        &Default::default(),
    ));
    let mut engine = Engine::with_modules(world, registry, mods).expect("npc module registration");
    mushd::modules::boot_modules(&mut engine, &**store)
        .await
        .expect("module boot");
    let sessions = Arc::new(SessionManager::new());
    let handles = world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        ServerDeps {
            metrics: std::sync::Arc::new(mush_net_web::Metrics::new()),
            store: Arc::clone(store),
            guard: Arc::clone(guard),
            staff,
            llm: Some(llm),
            quarantine_spool: mushd::quarantine::QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
        },
    );
    (sessions, handles)
}

/// Re-enter a finished NPC turn as the typed completion the world task applies
/// (the same `WorldCommand::ModuleAsyncDone` the off-loop turn re-enters as).
async fn npc_complete(world: &WorldHandle, npc: ObjId, correlation: u64, outcome: &TurnOutcome) {
    let payload = serde_json::to_string(outcome).unwrap();
    assert!(
        world
            .inbound(Inbound::Apply(WorldCommand::ModuleAsyncDone {
                module: SmolStr::new("npc"),
                actor: npc,
                correlation,
                payload,
            }))
            .await
    );
}

async fn npc_agent_round_trip(store: Arc<dyn ObjectStore>) {
    const PERSONA: &str = "a gruff barkeep who never forgets a tab";
    const NOTE: &str = "owes the bar forty coins for last week";
    const SUMMARY: &str =
        "Alice has been drinking here all evening and is running a large unpaid tab.";
    let guard = Arc::new(IpGuard::new());

    // --- first boot: create + configure an NPC, then apply a remember + a
    // summary through the real completion path. --------------------------
    let (sessions, handles) = boot_npc_rig(&store, &guard, npc_service()).await;
    let world = handles.handle.clone();
    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;

    god_cmd(&world, "@agent/create Bartender").await;
    let ev = expect_text(&mut god_rx, "Created NPC agent Bartender").await;
    let bartender = ObjId(support::parse_id_after(&text_of(&ev), "(#"));
    god_cmd(&world, &format!("@agent/set Bartender/persona={PERSONA}")).await;
    expect_text(&mut god_rx, "Set persona on Bartender").await;

    // A durable memory (a `mod:npc:mem:*` module doc) and a rolling summary
    // (an `agent:memory:summary` obj attr), both via the typed re-entry.
    npc_complete(
        &world,
        bartender,
        1,
        &TurnOutcome::Ok {
            asker: None,
            npc: bartender,
            actions: vec![AppliedAction {
                action: NpcAction::Remember {
                    subject: "Alice".into(),
                    note: NOTE.into(),
                },
            }],
        },
    )
    .await;
    npc_complete(
        &world,
        bartender,
        2,
        &TurnOutcome::Summary {
            npc: bartender,
            summary: SUMMARY.into(),
        },
    )
    .await;

    // Apply messages carry transferred semantic custody and may be retained
    // behind one another. Cross both exact receipts before issuing an ordinary
    // mutation, which would otherwise be refused while the second is pending.
    assert!(world.barrier().await, "NPC completion commit barrier");

    // Player creation is independently acknowledgement-gated after both
    // completion batches have landed.
    world
        .create_player("Fence", None)
        .await
        .expect("fence create");

    // --- second boot: rebuild from the store alone. ----------------------
    let (sessions2, handles2) = boot_npc_rig(&store, &guard, npc_service()).await;
    let world2 = handles2.handle.clone();
    let mut god_rx2 = attach_direct(&sessions2, &world2, 2, GOD, "god").await;

    // `@agent Bartender` shows the reloaded profile (obj attr), the reloaded
    // summary (obj attr), and the durable memory count (a `mod:npc:*` doc
    // reloaded by boot_modules -> on_world_loaded into the long-term store).
    god_cmd(&world2, "@agent Bartender").await;
    expect_text(&mut god_rx2, PERSONA).await;
    expect_text(&mut god_rx2, "Alice has been drinking here").await;
    expect_text(&mut god_rx2, "Durable memories: 1 subject").await;
}

/// The NPC agent round trip over an in-memory store — ungated, so CI proves the
/// profile + summary (obj attrs) and the durable memory (module doc)
/// write-through and reload wiring without a live Redis.
#[tokio::test(flavor = "multi_thread")]
async fn npc_agent_persistence_round_trip_in_memory() {
    npc_agent_round_trip(Arc::new(InMemoryStore::new())).await;
}

/// The same round trip over a live Redis — the NPC's profile/summary obj docs
/// and its `mod:npc:mem:*` module docs survive a reboot from Redis alone. Gated
/// on `REDIS_URL`; runs in db 8 (dbs 9–15 are claimed by the other gated tests,
/// which run in parallel).
#[tokio::test(flavor = "multi_thread")]
async fn npc_agent_persistence_round_trip_redis_gated() {
    let Some(url) = std::env::var("REDIS_URL").ok().filter(|s| !s.is_empty()) else {
        eprintln!(
            "skipping npc_agent_persistence_round_trip_redis_gated: REDIS_URL not set \
             (no live Redis on this machine)"
        );
        return;
    };
    let url = support::redis_url_with_db(&url, 8);
    let store = mush_store::RedisStore::connect(&url)
        .await
        .expect("connect to REDIS_URL");
    store.wipe_namespaces().await.expect("wipe test namespaces");
    npc_agent_round_trip(Arc::new(store)).await;
}

// ---------------------------------------------------------------------------
// WS07-S8. An unclean world exit takes no shutdown BGSAVE
// ---------------------------------------------------------------------------

/// The trap the world-exit term closes: a world task that dies mid-flight still
/// drops its job sender, so the writer drains perfectly and the integrity latch
/// stays clear. On the old two-term gate that row snapshotted — promoting a
/// half-applied world to the on-disk truth.
///
/// This replays `mushd`'s real shutdown tail (main.rs) over a counting store:
/// a positive control proves the rig can and does snapshot, and then the tail
/// after an aborted world task adds nothing to the count.
#[tokio::test(flavor = "multi_thread")]
async fn an_unclean_world_exit_takes_no_shutdown_bgsave() {
    let flaky = Arc::new(FlakyStore::new());
    let store: Arc<dyn ObjectStore> = Arc::clone(&flaky) as Arc<dyn ObjectStore>;
    let guard = Arc::new(IpGuard::new());
    let (sessions, handles) = boot_rig(&store, &guard).await;
    let world = handles.handle.clone();
    let mut god_rx = attach_direct(&sessions, &world, 1, GOD, "god").await;
    assert!(world.barrier().await, "connection setup barrier");

    // Positive control: this store really reaches BGSAVE and really counts it.
    god_cmd(&world, "@dump").await;
    expect_text(&mut god_rx, "@dump: snapshot acknowledged by the store.").await;
    assert_eq!(flaky.bgsaves(), 1, "@dump must have snapshotted");

    let PersistHandles {
        handle,
        world_join,
        writer_join,
        outbox_join,
        persistence_integrity,
        world_exit,
        // 16b cut T.2: this proof is about the store, not the arena; a
        // composition with no running match has nothing to join.
        arenas: _,
    } = handles;

    // `kill -9` shape, in-process: the world task stops without ever reaching
    // the statement that publishes a clean exit.
    world_join.abort();
    assert!(
        world_join.await.is_err(),
        "the world task must really be gone"
    );

    // The rest of main.rs's tail, in order: drop the handles, let the writer
    // drain, drain the store, then the gated snapshot.
    drop(handle);
    drop(world);
    outbox_join.abort();
    let writer_drained = writer_join.await.is_ok();
    store.drain().await.expect("store drain");
    let integrity_latched = persistence_integrity.is_latched();

    // The two terms the old gate had are both innocent here — which is exactly
    // why they were not enough.
    assert!(
        writer_drained,
        "the writer still drains after an aborted world task"
    );
    assert!(
        !integrity_latched,
        "nothing breached persistence integrity here"
    );
    assert!(
        !world_exit.is_clean(),
        "an aborted world task never reaches its clean-exit publication"
    );

    // `shutdown_bgsave_allowed` (main.rs, unit-tested there over the full
    // truth table) is this condition; the branch is live, so a wrongly-clean
    // flag would show up as a second BGSAVE.
    if world_exit.is_clean() && writer_drained && !integrity_latched {
        store.bgsave().await.expect("shutdown bgsave");
    }
    assert_eq!(
        flaky.bgsaves(),
        1,
        "the shutdown tail must add no BGSAVE after an unclean world exit"
    );
}

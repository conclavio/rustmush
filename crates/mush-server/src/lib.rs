//! # mush-server — the forked SERVER half (milestone M16b)
//!
//! The single-writer world task and everything behind it (persistence, game
//! modules, rulesets, the LLM layer, staff ops) running as its own process,
//! speaking the [`mush_ipc`] boundary protocol over a socket to
//! [`mush-gateway`]. It owns **no** connection listeners — SSH and web live in
//! the gateway.
//!
//! It reuses the `mushd` library wholesale for boot (store open, migrations,
//! world load, module assembly, the world task itself) and adds only the
//! server-side socket endpoint: a [`ToGateway::Hello`] handshake on connect,
//! then two directions of traffic —
//!
//! - **inbound** ([`ToServer`]): the gateway's live [`WireInbound`], correlated
//!   [`ServiceReq`]s (answered off the world loop, so the loop never blocks on
//!   the boundary — D4), and operator control;
//! - **outbound** ([`ToGateway`]): the world loop's resolved output (via the
//!   socket variant of `mushd`'s `InProcessTransport`), plus fire-and-forget
//!   session control (`SetFloodExempt`, `CloseSession`).
//!
//! The world loop's delivery seam is `mushd`'s
//! `InProcessTransport::new_socket`, so the entire world task is byte-for-byte
//! the same code the all-in-one binary runs; only the sink differs.
//!
//! [`mush-gateway`]: https://docs.rs/mush-gateway
//! [`ToServer`]: mush_ipc::ToServer
//! [`WireInbound`]: mush_ipc::WireInbound
//! [`ServiceReq`]: mush_ipc::ServiceReq

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use std::time::Duration;

use mush_config::accounts::import_accounts_json;
use mush_config::config::{Config, StoreBackend, StoreReceiptPolicy};
use mush_config::decision_keys::DurableDecisionKeyStore;
use mush_config::ipguard::IpGuard;
use mush_config::metrics::Metrics;
use mush_config::offer_keys::DurableOfferKeyStore;
use mush_config::resolution_keys::DurableResolutionKeyStore;
use mush_core::{ControlMsg, Engine, Inbound, OfferRouteTable, OfferSubmission};
use mush_ipc::{
    BoxWrite, FrameReader, GatewayInput, GatewayRequest, GatewayRequestDisposition, IPC_VERSION,
    IpcEndpoint, LegalActionsError, MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES,
    MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES_PER_CONNECTION, MAX_IN_FLIGHT_GATEWAY_INPUTS,
    MAX_IN_FLIGHT_GATEWAY_INPUTS_PER_CONNECTION, MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS,
    MAX_IN_FLIGHT_SERVICE_REQUESTS, RequestUid, ServiceReq, ServiceResp, ToGateway, ToServer,
};
use mush_llm::LlmService;
use mush_store::{
    BundleStatus, InMemoryStore, ObjectStore, RedisStore, core_migrations, run_migrations,
};
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};

pub mod accounts;
pub mod action_coordinator;
pub mod config_ext;
// E4c: the economy -> canon evidence bridge (08c §6.1 item 3).
pub mod decision_dispatch;
pub mod durable_outbox;
pub mod econ_alerts;
pub mod econ_canon;
pub mod econ_inbox;
pub mod gateway_sink;
pub mod help;
pub mod modules;
pub mod persist;
mod process_fault;
pub mod quarantine;
pub mod retained;
pub mod staff;
pub mod store_read;
pub mod timeline;
pub mod world_task;

use crate::action_coordinator::{ActionCoordinator, ActionPreflight};
use crate::gateway_sink::SocketSink;
use crate::persist::AuthoritativeCommitFailure;
use crate::persist::{load_or_seed_world, reconcile_authoritative_for_writable_boot};
use crate::quarantine::QuarantineSpool;
use crate::staff::{StaffQueue, register_server_commands_with_cap};
use crate::world_task::{
    CreatePlayerErr, LegalActionReadError, ServerDeps, WorldHandle, WorldReadError,
};

pub use gateway_sink::{GatewaySink, SocketSink as ServerSocketSink};

/// A human-readable server identity string, sent in [`ToGateway::Hello`].
fn server_id() -> String {
    format!("mush-server {}", env!("CARGO_PKG_VERSION"))
}

/// Open the configured store backend (fail fast when Redis is unreachable).
async fn open_store(config: &Config) -> Result<Arc<dyn ObjectStore>, String> {
    let receipt_policy = config.store_receipt_policy().map_err(|e| e.to_string())?;
    match config.store_backend().map_err(|e| e.to_string())? {
        StoreBackend::Redis => {
            let url = &config.store.redis_url;
            tracing::info!(%url, "connecting to Redis");
            let store = RedisStore::connect_read_only(url)
                .await
                .map_err(|e| format!("cannot reach Redis at {url}: {e}"))?;
            if receipt_policy == StoreReceiptPolicy::HostPowerLoss {
                store
                    .require_host_power_loss_receipts()
                    .await
                    .map_err(|e| format!("Redis does not satisfy [store].receipt_policy: {e}"))?;
            }
            Ok(Arc::new(store))
        }
        StoreBackend::Memory => {
            if receipt_policy == StoreReceiptPolicy::HostPowerLoss {
                return Err(
                    "[store].receipt_policy=\"host_power_loss\" requires the Redis backend"
                        .to_owned(),
                );
            }
            tracing::warn!("[store].store = \"memory\": nothing will survive a restart");
            Ok(Arc::new(InMemoryStore::new()))
        }
    }
}

/// Open the narrow pre-boot side of the configured store for quarantine
/// recovery.
///
/// Redis connection setup deliberately skips search-index bootstrap; schema
/// migrations, world seeds, imports, and module boot remain downstream of the
/// exact-match decision. The returned store is kept private and is handed to
/// [`QuarantineSpool`] only through its target-read plus exact-bundle recovery
/// interface.
async fn open_quarantine_recovery_store(config: &Config) -> Result<Arc<dyn ObjectStore>, String> {
    match config.store_backend().map_err(|error| error.to_string())? {
        StoreBackend::Redis => {
            let url = &config.store.redis_url;
            let store = RedisStore::connect_read_only(url)
                .await
                .map_err(|error| format!("cannot read Redis quarantine targets: {error}"))?;
            Ok(Arc::new(store))
        }
        StoreBackend::Memory => Ok(Arc::new(InMemoryStore::new())),
    }
}

/// Resolve exact authoritative matches, then enforce the quarantine gate.
///
/// This is the complete pre-write recovery plane shared by the split server
/// and all-in-one `mushd`. Legacy seals use `module_json_get`; K3 bundle
/// records may invoke only exact token/hash status and idempotent reconcile.
/// Any missing/different/unreadable target, invalid or full evidence archive,
/// or unresolved active record prevents migrations and every other ordinary
/// boot mutation.
///
/// # Errors
///
/// Returns an operator-facing recovery error while the spool's lifetime lock
/// remains held by the caller.
pub async fn prepare_quarantine_for_writable_boot(
    config: &Config,
    spool: &mut QuarantineSpool,
    supplied_store: Option<&dyn ObjectStore>,
) -> Result<(), String> {
    spool
        .require_bound_path(&config.server.data_dir.join("quarantine"))
        .map_err(|error| format!("quarantine spool preflight failed: {error}"))?;
    if spool.opened_state().unresolved_records == 0 {
        return Ok(());
    }

    let report = match supplied_store {
        Some(reader) => spool.reconcile_exact_matches(reader).await,
        None => {
            let reader = open_quarantine_recovery_store(config).await?;
            spool.reconcile_exact_matches(&*reader).await
        }
    }
    .map_err(|error| format!("quarantine exact-target recovery failed: {error}"))?;
    tracing::info!(
        examined = report.examined,
        resolved = report.resolved,
        remaining_records = report.remaining.unresolved_records,
        remaining_bytes = report.remaining.unresolved_bytes,
        "quarantine exact-target recovery pass complete"
    );
    spool
        .require_resolved_for_boot()
        .map_err(|error| format!("quarantine recovery required: {error}"))
}

/// Boot the full engine (world load/seed + modules + rulesets), exactly the
/// roster the all-in-one binary builds. Returns the engine plus the
/// [`StaffQueue`] its `@siteban`/`@boot`/... handlers push into — the SAME
/// queue the world loop must drain (`ServerDeps::staff`), matching `main.rs`.
async fn build_engine(
    config: &Config,
    store: &Arc<dyn ObjectStore>,
    _validated_spool: &QuarantineSpool,
) -> Result<(Engine, Arc<StaffQueue>), String> {
    let offer_keys = DurableOfferKeyStore::new(config.offer_key_dir())
        .load_or_create()
        .map_err(|error| format!("offer key provisioning failed: {error}"))?;
    tracing::info!(
        current_version = offer_keys.current_version(),
        key_dir = %config.offer_key_dir().display(),
        "durable legal-action offer keys loaded"
    );
    let resolution_keys = DurableResolutionKeyStore::new(config.resolution_key_dir())
        .load_or_create()
        .map_err(|error| format!("resolution key provisioning failed: {error}"))?;
    tracing::info!(
        current_version = resolution_keys.current_version(),
        key_dir = %config.resolution_key_dir().display(),
        "durable authoritative resolution keys loaded"
    );
    let decision_keys = DurableDecisionKeyStore::new(config.decision_key_dir())
        .load_or_create()
        .map_err(|error| format!("decision key provisioning failed: {error}"))?;
    tracing::info!(
        current_version = decision_keys.current_version(),
        key_dir = %config.decision_key_dir().display(),
        "durable delayed-decision keys loaded"
    );
    let authoritative = store.authoritative_store().ok_or_else(|| {
        "configured store does not implement K3 exact-token durability; refusing writable boot"
            .to_owned()
    })?;
    let recovery = reconcile_authoritative_for_writable_boot(authoritative).await?;
    tracing::info!(
        indexed = recovery.indexed,
        retained = recovery.retained,
        retained_bytes = recovery.retained_bytes,
        retention_counters_repaired = recovery.retention_counters_repaired,
        reconciled = recovery.reconciled,
        already_finalized = recovery.already_finalized,
        checkpointed = recovery.checkpointed,
        checkpoint_deferred_outbox = recovery.checkpoint_deferred_outbox,
        scan_pages = recovery.scan_pages,
        "authoritative intent recovery barrier complete"
    );
    store
        .bootstrap_writable()
        .await
        .map_err(|error| format!("store writable bootstrap failed: {error}"))?;
    let version = run_migrations(&**store, &core_migrations())
        .await
        .map_err(|e| format!("store migrations failed: {e}"))?;
    tracing::info!(schema = version, "store schema current");
    let imported = import_accounts_json(&**store, &config.server.data_dir).await?;
    if imported > 0 {
        tracing::info!(imported, "M1 accounts.json imported");
    }
    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands_with_cap(&mut registry, &staff, config.game.max_characters as usize)
        .map_err(|e| format!("server command registration: {e}"))?;
    match crate::help::load_help_topics(&mut registry, &config.server.help_dir) {
        Ok(n) => tracing::info!(topics = n, "help topics loaded"),
        Err(e) => tracing::warn!(error = %e, "help topics failed to load; continuing"),
    }
    let llm_config = crate::config_ext::llm_config(&config.llm)?;
    let llm =
        Arc::new(LlmService::from_config(&llm_config).map_err(|e| format!("llm config: {e}"))?);
    let onboarding = crate::config_ext::onboarding_ruleset(&config.onboarding);
    // E5c: `[wod.economy]`, validated here (an unknown mode is a hard boot error).
    // `None` when no WoD ruleset is enabled — that is what gates the housing title
    // issuer (08d §2).
    let wod_economy = crate::config_ext::wod_economy_config(&config.wod)?;
    let wod_selected = config
        .rulesets
        .enabled
        .iter()
        .any(|r| r.trim().eq_ignore_ascii_case("wod"));
    // E6a: `[shadowrun.economy]`, validated here (an unknown lifestyle tier is a hard
    // boot error). `None` when no Shadowrun ruleset is enabled — that is what gates
    // the run board's economy observer (08f §2's `issuers` pattern).
    let sr_economy = crate::config_ext::shadowrun_economy_config(&config.shadowrun)?;
    let sr_selected = config
        .rulesets
        .enabled
        .iter()
        .any(|r| r.trim().eq_ignore_ascii_case("shadowrun"));
    // SP1 / D-LBK-7: a Shadowrun server needs `sr.month` declared or no lifestyle
    // obligation can exist, so the host contributes the stanza — unless the operator
    // declared one of that name themselves, in which case theirs stands.
    let contributed: Vec<(String, mush_economy::DeclaredCadence)> = if sr_selected {
        crate::config_ext::shadowrun_cadences(&sr_economy)
    } else {
        Vec::new()
    };
    let contributed: Vec<(&str, mush_economy::DeclaredCadence)> = contributed
        .iter()
        .map(|(n, d)| (n.as_str(), d.clone()))
        .collect();
    let mut mods = modules::server_modules_with(
        crate::config_ext::weather_config(&config.weather),
        crate::config_ext::economy_config_with(&config.economy, &contributed)?,
        &onboarding,
        wod_selected.then_some(&wod_economy),
        sr_selected.then_some(&sr_economy),
    );
    mods.push(modules::worldgen_module_with(
        Arc::clone(&llm),
        crate::config_ext::worldgen_config(&config.worldgen),
    ));
    let npc_grounding = modules::npc_agent_adapter(&config.rulesets.enabled);
    mods.push(modules::npc_module(npc_grounding));
    let rulesets = modules::ruleset_modules(
        &config.rulesets.enabled,
        &onboarding,
        &wod_economy,
        &sr_economy,
    )
    .map_err(|e| format!("ruleset selection: {e}"))?;
    mods.extend(rulesets);
    // Compose and freeze every mechanics registry before any persisted world
    // record is loaded. Rules-bearing attrs must never be interpreted against
    // a half-built definition/bridge set (K1 / C1 boot ordering).
    let composition =
        Engine::compose_modules(registry, mods).map_err(|e| format!("module registration: {e}"))?;
    let mut world = load_or_seed_world(&**store).await?;
    let clock = config.game.game_clock().map_err(|e| e.to_string())?;
    timeline::hydrate_world_timeline(&mut world, &**store, clock).await?;
    let mut engine = Engine::from_composition(world, composition);
    engine.set_offer_keys(offer_keys);
    engine.set_resolution_keys(resolution_keys);
    engine.set_decision_keys(decision_keys);
    modules::boot_modules(&mut engine, &**store).await?;
    Ok((engine, staff))
}

/// Run the server: boot the world, then bind the IPC socket and serve one
/// gateway connection for the life of the boundary. Returns when the gateway
/// disconnects (clean EOF) or the world shuts down.
///
/// `bound` (when given) is signalled with the actually-bound endpoint once the
/// listener is up — the two-process test binds `127.0.0.1:0` and reads the
/// OS-assigned port back through it.
///
/// # Errors
///
/// Boot failures (store unreachable, bad config, module registration) and
/// fatal I/O on the listener.
pub async fn serve(
    config: Config,
    bound: Option<oneshot::Sender<IpcEndpoint>>,
) -> Result<(), String> {
    serve_with_store(config, bound, None).await
}

/// Like [`serve`], but with a caller-supplied object store instead of opening
/// one from config. `None` opens the configured backend (the [`serve`] path,
/// unchanged). `Some(store)` reuses the given store — the restart-survival test
/// passes ONE shared store across every server (re)boot so world objects +
/// durable module state survive a restart exactly as they would on Redis (R1),
/// without needing a live Redis on the dev box.
///
/// # Errors
///
/// Boot failures (store unreachable, bad config, module registration) and
/// fatal I/O on the listener.
pub async fn serve_with_store(
    config: Config,
    bound: Option<oneshot::Sender<IpcEndpoint>>,
    store: Option<Arc<dyn ObjectStore>>,
) -> Result<(), String> {
    // K0 integrity preflight is deliberately the first filesystem-backed boot
    // action. A partial, unreadable, malformed, or hash-invalid spool prevents
    // migrations, world seeding, and module boot writes.
    let quarantine_spool = QuarantineSpool::open(config.server.data_dir.join("quarantine"))
        .map_err(|e| format!("quarantine spool preflight failed: {e}"))?;
    serve_with_quarantine_spool(config, bound, store, quarantine_spool).await
}

/// Run the server with an already-open quarantine spool.
///
/// The standalone binary uses this entry point so the structurally validated
/// spool and its exclusive lifetime lock survive logging/runtime setup and the
/// transition into recovery and normal service. Library callers normally use
/// [`serve`] or [`serve_with_store`].
///
/// # Errors
///
/// Boot failures (including unresolved quarantine), store failures, and fatal
/// I/O on the listener.
pub async fn serve_with_quarantine_spool(
    config: Config,
    bound: Option<oneshot::Sender<IpcEndpoint>>,
    store: Option<Arc<dyn ObjectStore>>,
    mut quarantine_spool: QuarantineSpool,
) -> Result<(), String> {
    let endpoint = config.ipc_endpoint().map_err(|e| e.to_string())?;
    prepare_quarantine_for_writable_boot(&config, &mut quarantine_spool, store.as_deref()).await?;
    let store = match store {
        Some(s) => s,
        None => open_store(&config).await?,
    };
    let (engine, staff) = build_engine(&config, &store, &quarantine_spool).await?;
    let offer_routes = Arc::new(
        engine
            .registry
            .action_providers()
            .ok_or_else(|| "legal-action provider registry is unavailable".to_owned())?
            .offer_route_table(),
    );

    // The web hello manifest, collected while the registry is still in hand — a
    // verbatim port of `mushd::main`'s assembly (crates/mushd/src/main.rs): every
    // module-registered panel feed plus the engine-side "who", and one feature
    // id per module (comsys advertises as "channels"). Shipped to the gateway in
    // the boot [`ToGateway::Hello`] so the split gateway can build the web
    // client's `hello` frame with the SAME features/panels the all-in-one
    // advertises.
    let (features, panels) = build_hello_manifest(&engine);

    // The server -> gateway boundary: every `ToGateway` the world loop (and the
    // request handlers) produce is queued here and drained to the socket by the
    // writer task. BOUNDED (M18, `[ipc].to_gw_cap`): every `to_gw` frame is
    // must-deliver, so it is NEVER per-message dropped — on overflow the whole
    // boundary is torn down (the M16c supervisor re-attach re-renders every
    // session, healing the loss). The world loop's `transmit` still never awaits
    // the boundary (D4): the [`SocketSink`] uses `try_send`, and the teardown
    // decision + socket drop live in the writer task, which waits on `teardown`.
    // The forked server holds no `SessionManager` — the gateway process owns it —
    // so the world loop's session control (`@boot`, `@newpassword`) goes out as
    // fire-and-forget `ToGateway` frames through the [`SocketSink`] too.
    let to_gw_cap = config.ipc_to_gw_cap();
    let write_timeout = Duration::from_millis(config.ipc_write_timeout_ms());
    let high_water_pct = config.ipc_high_water_warn_pct();
    let (to_gw_tx, to_gw_rx) = mpsc::channel::<ToGateway>(to_gw_cap);

    let guard = Arc::new(IpGuard::new());
    let mut deps = ServerDeps::in_memory();
    deps.store = Arc::clone(&store);
    deps.guard = Arc::clone(&guard);
    // The world loop must drain the SAME staff queue the registry handlers push
    // into (matching `mushd::main`), or `@boot`/`@siteban`/... would be enqueued
    // to an orphan queue and never execute.
    deps.staff = staff;
    // Unified quarantine spool: a separate failure domain from the store,
    // under the configured data dir. Confirmed poison seals and complete K3
    // bundles are fsynced here before the writer enters its integrity halt.
    deps.quarantine_spool = quarantine_spool;

    // Cloned out BEFORE `deps` is moved into the world spawn: the writer task and
    // the [`SocketSink`] stamp shed/timeout/high-water onto the SAME registry the
    // world loop reads for `@stats` (never a second registry). `teardown` is the
    // one-shot the [`SocketSink`] notifies on overflow and the writer waits on.
    let metrics = Arc::clone(&deps.metrics);
    let teardown = Arc::new(Notify::new());
    let sink = SocketSink::new(
        to_gw_tx.clone(),
        Arc::clone(&metrics),
        Arc::clone(&teardown),
        to_gw_cap,
        high_water_pct,
    );
    // The reader's detached `Reply` sends ride the SAME sink (shared teardown +
    // overflow counter) so a full channel tears the boundary down there too — a
    // dropped `Reply` would hang its `ServiceReq` oneshot (I1).
    let reply_sink = sink.clone();
    let ingress_store = Arc::clone(&store);
    let handles = world_task::spawn_persist_with_gateway(engine, deps, move |_weak_inbound| {
        // The forked server never polices (it owns no connection buffers), so it
        // ignores the loop's inbound weak sender and just serializes to the boundary.
        Arc::new(sink)
    });
    let world = handles.handle;
    let world_join = handles.world_join;
    let store_writer_join = handles.writer_join;
    let outbox_join = handles.outbox_join;

    let listener = mush_ipc::listen(&endpoint)
        .await
        .map_err(|e| format!("ipc listen on {}: {e}", endpoint.describe()))?;
    let actual = listener
        .tcp_addr()
        .map_or_else(|| endpoint.clone(), IpcEndpoint::Tcp);
    tracing::info!(ipc = %listener.describe(), "mush-server up; awaiting gateway");
    if let Some(tx) = bound {
        let _ = tx.send(actual);
    }

    // One gateway connection per boundary (M16b; M16c adds restart/re-attach).
    let (reader_half, writer_half) = listener
        .accept()
        .await
        .map_err(|e| format!("ipc accept: {e}"))?;
    tracing::info!("gateway connected");

    // Writer: greet (with the panel manifest), then drain the world's output to
    // the socket. Exits when every `to_gw_tx` (held by the world loop + request
    // handlers) is dropped, OR early on a per-frame write timeout / a `teardown`
    // notification (a `to_gw` overflow) — either drops the writer's socket half,
    // EOFing the boundary into the supervise → relaunch → re-attach heal (M18).
    let ipc_writer_join = tokio::spawn(writer_loop(
        writer_half,
        to_gw_rx,
        features,
        panels,
        Arc::clone(&metrics),
        Arc::clone(&teardown),
        write_timeout,
    ));

    // Reader: pump the gateway's `ToServer` traffic into the world. Runs until
    // the gateway closes the boundary (clean EOF) or a protocol fault.
    let reader_result = reader_loop(
        reader_half,
        world.clone(),
        ingress_store,
        offer_routes,
        reply_sink,
        &metrics,
    )
    .await;
    match reader_result {
        Ok(()) => tracing::info!("gateway disconnected (clean EOF)"),
        Err(e) => tracing::warn!(error = %e, "ipc reader ended on error"),
    }

    // Boundary gone: shut the world down cleanly, then let the writer flush its
    // Bye and exit (it stops when the world drops its `to_gw_tx` clones).
    world.inbound(Inbound::Control(ControlMsg::Shutdown)).await;
    let _ = world_join.await;
    drop(to_gw_tx);
    drop(world);
    let _ = outbox_join.await;
    // Keep custody of the actual persistence writer (distinct from the IPC
    // writer above) and wait for it to drain before this runtime can return.
    // Detaching it here would let process teardown discard a batch after the
    // world believed shutdown sequencing had transferred custody.
    let _ = store_writer_join.await;
    let _ = ipc_writer_join.await;
    tracing::info!("mush-server exiting");
    Ok(())
}

/// Build the web `hello` manifest from the booted engine — a verbatim port of
/// `mushd::main`'s assembly: every module-registered panel feed plus the
/// engine-side `"who"`, and one feature id per module (comsys advertises as
/// `"channels"`). Returns `(features, panels)`.
fn build_hello_manifest(engine: &Engine) -> (Vec<String>, Vec<String>) {
    let mut panels: Vec<String> = engine
        .registry
        .panel_feeds()
        .iter()
        .map(|(feed, _)| feed.to_string())
        .collect();
    panels.push("who".to_string());
    panels.push("legal.actions".to_string());
    let features: Vec<String> = engine
        .modules()
        .iter()
        .map(|m| match m.name() {
            "comsys" => "channels".to_string(),
            other => other.to_string(),
        })
        .collect();
    (features, panels)
}

/// The socket writer: send [`ToGateway::Hello`] (carrying the panel manifest),
/// then forward each queued message. On the channel closing (world gone) it
/// sends a final [`ToGateway::Bye`] and returns.
///
/// Two M18 teardown paths return early, each dropping `w` (the boundary EOFs,
/// which the gateway supervisor heals via relaunch → ReAnnounce → re-attach):
///
/// - **write timeout** — each [`write_frame`](mush_ipc::write_frame) is wrapped
///   in `write_timeout`; a connected-but-not-draining peer that blocks past it
///   is wedged, so we count it (`ipc_write_timeouts_total`), warn, and return.
/// - **`to_gw` overflow** — the [`SocketSink`] `notify`s `teardown` when it hits
///   a full channel; the drain `select!` takes that arm and returns.
///
/// Never skip-a-message-and-continue: a full socket buffer means the next write
/// blocks too, and skipping could drop a keyframe — the boundary teardown +
/// re-attach is the correct heal.
async fn writer_loop(
    mut w: BoxWrite,
    mut rx: mpsc::Receiver<ToGateway>,
    features: Vec<String>,
    panels: Vec<String>,
    metrics: Arc<Metrics>,
    teardown: Arc<Notify>,
    write_timeout: Duration,
) {
    // The Hello must reach the gateway before any frame; honor the write
    // timeout so a peer that never reads cannot wedge the handshake forever.
    if !write_or_teardown(
        &mut w,
        &ToGateway::Hello {
            proto: IPC_VERSION,
            server: server_id(),
            features,
            panels,
        },
        &metrics,
        write_timeout,
        "failed to send Hello; boundary down",
    )
    .await
    {
        return;
    }
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Some(msg) => {
                    if !write_or_teardown(
                        &mut w,
                        &msg,
                        &metrics,
                        write_timeout,
                        "ipc write failed; boundary down",
                    )
                    .await
                    {
                        return;
                    }
                }
                // Every `to_gw_tx` dropped (world gone): clean shutdown.
                None => break,
            },
            // A `to_gw` overflow: the sink notified us. Drop `w` (return) to EOF
            // the boundary — the supervisor re-attach re-renders every session.
            () = teardown.notified() => {
                tracing::warn!(dir = "to_gw", "ipc queue overflow; tearing down boundary");
                return;
            }
        }
    }
    // Best-effort Bye on a clean shutdown — timeout-guarded so a wedged peer
    // cannot hang the exit.
    let _ = tokio::time::timeout(
        write_timeout,
        mush_ipc::write_frame(&mut w, &ToGateway::Bye),
    )
    .await;
}

/// Write one frame under `write_timeout`. Returns `true` on success; on a write
/// error or an elapsed timeout it logs a warn (counting the timeout as an
/// `ipc_write_timeouts_total`) and returns `false` so the caller tears the
/// boundary down by returning (dropping the socket).
async fn write_or_teardown(
    w: &mut BoxWrite,
    msg: &ToGateway,
    metrics: &Metrics,
    write_timeout: Duration,
    err_context: &str,
) -> bool {
    match tokio::time::timeout(write_timeout, mush_ipc::write_frame(w, msg)).await {
        Ok(Ok(())) => true,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "{err_context}");
            false
        }
        Err(_) => {
            metrics.ipc_write_timeout();
            tracing::warn!(
                timeout_ms = write_timeout.as_millis() as u64,
                "ipc write timed out; peer wedged, tearing down boundary"
            );
            false
        }
    }
}

/// The socket reader: decode each [`ToServer`] frame and drive the world.
///
/// Correlated [`ServiceReq`]s are handled in detached tasks so a request whose
/// reply the world produces asynchronously never blocks the reader — and,
/// critically, the world loop itself never awaits the boundary (D4): it only
/// ever *receives* here.
#[derive(Clone)]
struct ServiceRequestLimiter {
    slots: Arc<Semaphore>,
    /// Edge latch so a hostile raw peer cannot convert Busy refusals into a
    /// warning-log flood. A successful admission rearms the warning.
    overload_warned: Arc<AtomicBool>,
}

impl ServiceRequestLimiter {
    fn new() -> Self {
        Self {
            slots: Arc::new(Semaphore::new(MAX_IN_FLIGHT_SERVICE_REQUESTS)),
            overload_warned: Arc::new(AtomicBool::new(false)),
        }
    }

    fn try_admit(&self) -> Result<OwnedSemaphorePermit, ServiceResp> {
        match Arc::clone(&self.slots).try_acquire_owned() {
            Ok(permit) => {
                self.overload_warned.store(false, Ordering::Relaxed);
                Ok(permit)
            }
            Err(_) => {
                if !self.overload_warned.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        cap = MAX_IN_FLIGHT_SERVICE_REQUESTS,
                        "ipc service-request handlers full; refusing request"
                    );
                }
                Err(ServiceResp::Busy)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ServerConnectionClaims {
    count: usize,
    bytes: usize,
}

#[derive(Clone, Debug)]
struct ServerLegacyClaim {
    request: GatewayRequest,
    bytes: usize,
}

/// Server-side bounded pre-ownership custody.
///
/// Legacy commands remain here until the gateway releases its replay copy.
/// Coordinator actions remain here until exact intent ownership (or a proven
/// pre-intent refusal). Independent limits defend against a raw IPC peer that
/// ignores the gateway's own custody caps.
#[derive(Default)]
struct ServerInputClaims {
    entries: HashMap<RequestUid, ServerLegacyClaim>,
    per_connection: HashMap<u64, ServerConnectionClaims>,
    bytes: usize,
    capacity_warned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ServerClaimAdmission {
    New,
    Duplicate,
    Full { warn: bool },
    Invalid,
    Conflict,
}

impl ServerInputClaims {
    fn admit(&mut self, request: GatewayRequest) -> ServerClaimAdmission {
        if !request.has_valid_uid() {
            return ServerClaimAdmission::Invalid;
        }
        let Ok(bytes) = request.input.logical_bytes() else {
            return ServerClaimAdmission::Invalid;
        };
        if let Some(existing) = self.entries.get(&request.uid) {
            return if existing.request == request {
                ServerClaimAdmission::Duplicate
            } else {
                ServerClaimAdmission::Conflict
            };
        }

        let conn = request.input.conn();
        let account = self.per_connection.get(&conn).copied().unwrap_or_default();
        let full = self.entries.len() >= MAX_IN_FLIGHT_GATEWAY_INPUTS
            || self.bytes.saturating_add(bytes) > MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES
            || account.count >= MAX_IN_FLIGHT_GATEWAY_INPUTS_PER_CONNECTION
            || account.bytes.saturating_add(bytes)
                > MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES_PER_CONNECTION;
        if full {
            let warn = !self.capacity_warned;
            self.capacity_warned = true;
            return ServerClaimAdmission::Full { warn };
        }

        self.entries
            .insert(request.uid, ServerLegacyClaim { request, bytes });
        self.bytes += bytes;
        let account = self.per_connection.entry(conn).or_default();
        account.count += 1;
        account.bytes += bytes;
        self.capacity_warned = false;
        ServerClaimAdmission::New
    }

    fn release(&mut self, uid: RequestUid) -> Option<GatewayRequest> {
        let held = self.entries.remove(&uid)?;
        let conn = held.request.input.conn();
        self.bytes = self.bytes.saturating_sub(held.bytes);
        if let Some(account) = self.per_connection.get_mut(&conn) {
            account.count = account.count.saturating_sub(1);
            account.bytes = account.bytes.saturating_sub(held.bytes);
            if account.count == 0 {
                self.per_connection.remove(&conn);
            }
        }
        if self.entries.len() < MAX_IN_FLIGHT_GATEWAY_INPUTS
            && self.bytes < MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES
        {
            self.capacity_warned = false;
        }
        Some(held.request)
    }
}

fn request_disposition(
    sink: &SocketSink,
    request: RequestUid,
    disposition: GatewayRequestDisposition,
) {
    sink.send(ToGateway::RequestDisposition {
        request,
        disposition,
    });
}

fn release_input_claim(
    claims: &Arc<Mutex<ServerInputClaims>>,
    request: RequestUid,
) -> Option<GatewayRequest> {
    claims
        .lock()
        .expect("server input claims poisoned")
        .release(request)
}

async fn handle_authoritative_input(
    request: GatewayRequest,
    routes: Arc<OfferRouteTable>,
    world: WorldHandle,
    store: Arc<dyn ObjectStore>,
    sink: SocketSink,
    claims: Arc<Mutex<ServerInputClaims>>,
) {
    let uid = request.uid;
    let (conn, actor) = match &request.input {
        GatewayInput::Action(input) => (input.conn, input.actor),
        _ => unreachable!("only action requests enter the coordinator handler"),
    };
    let envelope = match ActionCoordinator::gateway_ingress(&request, &routes) {
        Ok(envelope) => envelope,
        Err(detail) => {
            release_input_claim(&claims, uid);
            request_disposition(
                &sink,
                uid,
                GatewayRequestDisposition::Rejected { retryable: false },
            );
            tracing::warn!(%uid, %detail, "invalid structured coordinator request");
            return;
        }
    };
    let status = match ActionCoordinator::status_id(envelope.idempotency()) {
        Ok(status) => status,
        Err(error) => {
            release_input_claim(&claims, uid);
            request_disposition(
                &sink,
                uid,
                GatewayRequestDisposition::Rejected { retryable: false },
            );
            tracing::error!(%uid, error = %error, "coordinator status identity failed");
            return;
        }
    };
    let Some(authoritative_store) = store.authoritative_store() else {
        tracing::error!(%uid, "configured store lacks authoritative request capability; preserving custody");
        return;
    };
    let mut preflight_delay = Duration::from_millis(100);
    let preflight = loop {
        match ActionCoordinator::preflight(authoritative_store, envelope.clone()).await {
            Ok(preflight) => break preflight,
            Err(error) if error.is_transient() => {
                // Busy means "proved no intent" and would be a lie here: this
                // may be a replay whose exact intent already exists. Retain
                // bounded custody and retry the read-only lookup in place.
                tracing::warn!(%uid, error = %error, "coordinator preflight transient; retaining custody");
                tokio::time::sleep(preflight_delay).await;
                preflight_delay = (preflight_delay * 2).min(Duration::from_secs(5));
            }
            Err(error) => {
                // A deterministic/ambiguous store error cannot prove that no
                // intent exists. Preserve both gateway and server custody and
                // publish no lying rejection.
                tracing::error!(%uid, error = %error, "coordinator preflight could not prove request state");
                return;
            }
        }
    };

    let permit = match preflight {
        ActionPreflight::Vacant(permit) => permit,
        ActionPreflight::Pending { token, .. } => {
            release_input_claim(&claims, uid);
            request_disposition(
                &sink,
                uid,
                GatewayRequestDisposition::Owned {
                    token,
                    status: status.clone(),
                },
            );
            return;
        }
        ActionPreflight::Finalized(receipt) => {
            release_input_claim(&claims, uid);
            match world.project_known_action_receipt(actor, status.clone(), &receipt) {
                Ok(projected) => request_disposition(
                    &sink,
                    uid,
                    GatewayRequestDisposition::Finished {
                        recipient: actor,
                        receipt: projected,
                    },
                ),
                Err(error) => {
                    tracing::error!(%uid, error = %error, "finalized retry projection failed");
                    request_disposition(
                        &sink,
                        uid,
                        GatewayRequestDisposition::Owned {
                            token: receipt.token(),
                            status: status.clone(),
                        },
                    );
                }
            }
            return;
        }
        ActionPreflight::Retired { .. } | ActionPreflight::Conflict => {
            release_input_claim(&claims, uid);
            request_disposition(
                &sink,
                uid,
                GatewayRequestDisposition::Rejected { retryable: false },
            );
            return;
        }
        ActionPreflight::Quarantined(receipt) => {
            // Quarantine is terminal evidence, but not successful completion
            // and not proof of "no intent". Hold custody for the incident.
            tracing::error!(
                %uid,
                token = %receipt.token(),
                evidence = %receipt.evidence_hash(),
                "coordinator request is quarantined; preserving gateway custody"
            );
            return;
        }
    };

    // Exact pending/finalized retries above are already authenticated by the
    // durable binding that created them. Only a genuinely vacant request may
    // enter planning, and that requires the current transport association.
    // This ordering lets a lost ownership acknowledgement converge even if
    // the original connection disappeared before gateway replay.
    match world.connection_ready(conn, actor).await {
        Ok(true) => {}
        Ok(false) => {
            release_input_claim(&claims, uid);
            request_disposition(
                &sink,
                uid,
                GatewayRequestDisposition::Rejected { retryable: false },
            );
            tracing::warn!(%uid, conn, %actor, "coordinator request failed transport authentication");
            return;
        }
        Err(WorldReadError::Busy) => {
            release_input_claim(&claims, uid);
            request_disposition(&sink, uid, GatewayRequestDisposition::Busy);
            return;
        }
        Err(WorldReadError::Unavailable) => {
            release_input_claim(&claims, uid);
            request_disposition(
                &sink,
                uid,
                GatewayRequestDisposition::Rejected { retryable: true },
            );
            return;
        }
    }

    let submission = match &request.input {
        GatewayInput::Action(input) => OfferSubmission {
            offer: input.offered_action,
            subject: input.subject.clone(),
            targets: input.targets.clone(),
            parameters: input.parameters.clone(),
        },
        _ => unreachable!("validated coordinator request changed variants"),
    };

    let watch = match world.authoritative_action(permit, submission).await {
        Ok(watch) => watch,
        Err(world_task::AuthoritativeActionStartError::Busy) => {
            release_input_claim(&claims, uid);
            request_disposition(&sink, uid, GatewayRequestDisposition::Busy);
            return;
        }
        Err(world_task::AuthoritativeActionStartError::Unavailable) => {
            release_input_claim(&claims, uid);
            request_disposition(
                &sink,
                uid,
                GatewayRequestDisposition::Rejected { retryable: true },
            );
            return;
        }
        Err(world_task::AuthoritativeActionStartError::Rejected(detail)) => {
            release_input_claim(&claims, uid);
            request_disposition(
                &sink,
                uid,
                GatewayRequestDisposition::Rejected { retryable: false },
            );
            tracing::warn!(%uid, %detail, "coordinator rejected action before durable ownership");
            return;
        }
    };

    let owned = match watch.owned.await {
        Ok(Ok(staged)) => staged,
        Ok(Err(AuthoritativeCommitFailure::Refused(detail))) => {
            release_input_claim(&claims, uid);
            request_disposition(&sink, uid, GatewayRequestDisposition::Busy);
            tracing::warn!(%uid, %detail, "coordinator action refused before intent reservation");
            return;
        }
        Ok(Err(AuthoritativeCommitFailure::Integrity(detail))) => {
            tracing::error!(%uid, %detail, "coordinator ownership became ambiguous; preserving custody");
            return;
        }
        Err(_) => {
            tracing::error!(%uid, "world dropped coordinator ownership result; preserving custody");
            return;
        }
    };
    release_input_claim(&claims, uid);
    request_disposition(
        &sink,
        uid,
        GatewayRequestDisposition::Owned {
            token: owned.token,
            status: status.clone(),
        },
    );

    match watch.finished.await {
        Ok(Ok(finished)) if finished == owned => {
            match authoritative_store
                .authoritative_status(finished.token, finished.bundle_hash)
                .await
            {
                Ok(BundleStatus::FinalizedExact(receipt)) => {
                    match world.project_known_action_receipt(actor, status.clone(), &receipt) {
                        Ok(receipt) => request_disposition(
                            &sink,
                            uid,
                            GatewayRequestDisposition::Finished {
                                recipient: actor,
                                receipt,
                            },
                        ),
                        Err(error) => {
                            tracing::error!(%uid, error = %error, "terminal receipt projection failed");
                        }
                    }
                }
                Ok(other) => {
                    tracing::error!(%uid, ?other, "terminal action lacks a finalized receipt")
                }
                Err(error) => {
                    tracing::error!(%uid, error = %error, "terminal action receipt read failed")
                }
            }
        }
        Ok(Ok(finished)) => tracing::error!(
            %uid,
            owned_token = %owned.token,
            finished_token = %finished.token,
            "coordinator terminal identity differed from owned intent"
        ),
        Ok(Err(failure)) => tracing::error!(
            %uid,
            error = %failure.detail(),
            "owned coordinator request ended in an integrity incident"
        ),
        Err(_) => tracing::error!(%uid, "world dropped coordinator terminal result"),
    }
}

async fn reader_loop(
    r: mush_ipc::BoxRead,
    world: WorldHandle,
    store: Arc<dyn ObjectStore>,
    offer_routes: Arc<OfferRouteTable>,
    reply_sink: SocketSink,
    metrics: &Metrics,
) -> std::io::Result<()> {
    let mut reader = FrameReader::new(r);
    let service_requests = ServiceRequestLimiter::new();
    let input_claims = Arc::new(Mutex::new(ServerInputClaims::default()));
    while let Some(msg) = reader.read::<ToServer>().await? {
        match msg {
            ToServer::Inbound(wire) => {
                if matches!(
                    wire,
                    mush_ipc::WireInbound::Cmd { .. } | mush_ipc::WireInbound::Query { .. }
                ) {
                    tracing::error!("raw player input bypassed the version-3 custody envelope");
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "player input requires GatewayRequest custody",
                    ));
                }
                world.inbound(Inbound::from(wire)).await;
            }
            ToServer::Input(request) => {
                let uid = request.uid;
                if !request.has_valid_uid() || request.input.logical_bytes().is_err() {
                    tracing::error!(request = %uid, "invalid gateway request envelope");
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid gateway request envelope",
                    ));
                }
                match &request.input {
                    GatewayInput::Query { .. } => {
                        let accepted = world.inbound(request.input.into()).await;
                        request_disposition(
                            &reply_sink,
                            uid,
                            if accepted {
                                GatewayRequestDisposition::EphemeralFinished
                            } else {
                                GatewayRequestDisposition::Rejected { retryable: true }
                            },
                        );
                    }
                    GatewayInput::Cmd { .. } => {
                        let admission = input_claims
                            .lock()
                            .expect("server input claims poisoned")
                            .admit(request);
                        match admission {
                            ServerClaimAdmission::New | ServerClaimAdmission::Duplicate => {
                                request_disposition(
                                    &reply_sink,
                                    uid,
                                    GatewayRequestDisposition::LegacyClaimed,
                                );
                            }
                            ServerClaimAdmission::Full { warn } => {
                                if warn {
                                    tracing::warn!(
                                        request = %uid,
                                        held = input_claims
                                            .lock()
                                            .expect("server input claims poisoned")
                                            .entries
                                            .len(),
                                        bytes = input_claims
                                            .lock()
                                            .expect("server input claims poisoned")
                                            .bytes,
                                        "server legacy-claim custody full; refusing request"
                                    );
                                }
                                request_disposition(
                                    &reply_sink,
                                    uid,
                                    GatewayRequestDisposition::Busy,
                                );
                            }
                            ServerClaimAdmission::Invalid | ServerClaimAdmission::Conflict => {
                                tracing::error!(
                                    request = %uid,
                                    "conflicting or invalid gateway request identity"
                                );
                                return Err(std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    "conflicting or invalid gateway request identity",
                                ));
                            }
                        }
                    }
                    GatewayInput::Action(_) => {
                        let admission = input_claims
                            .lock()
                            .expect("server input claims poisoned")
                            .admit(request.clone());
                        match admission {
                            ServerClaimAdmission::New => {
                                let world = world.clone();
                                let store = Arc::clone(&store);
                                let routes = Arc::clone(&offer_routes);
                                let sink = reply_sink.clone();
                                let claims = Arc::clone(&input_claims);
                                tokio::spawn(async move {
                                    handle_authoritative_input(
                                        request, routes, world, store, sink, claims,
                                    )
                                    .await;
                                });
                            }
                            ServerClaimAdmission::Duplicate => {
                                tracing::debug!(
                                    request = %uid,
                                    "duplicate coordinator input joined server pre-ownership custody"
                                );
                            }
                            ServerClaimAdmission::Full { warn } => {
                                if warn {
                                    tracing::warn!(
                                        request = %uid,
                                        "server coordinator custody full; refusing request"
                                    );
                                }
                                request_disposition(
                                    &reply_sink,
                                    uid,
                                    GatewayRequestDisposition::Busy,
                                );
                            }
                            ServerClaimAdmission::Invalid | ServerClaimAdmission::Conflict => {
                                tracing::error!(
                                    request = %uid,
                                    "conflicting or invalid coordinator request identity"
                                );
                                return Err(std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    "conflicting or invalid gateway request identity",
                                ));
                            }
                        }
                    }
                }
            }
            ToServer::GatewayReleased { request } => {
                let Some(claim) = input_claims
                    .lock()
                    .expect("server input claims poisoned")
                    .release(request)
                else {
                    tracing::debug!(
                        request = %request,
                        "duplicate or stale legacy release ignored"
                    );
                    continue;
                };
                // Remove the server claim before enqueueing. From this point a
                // crash can lose the command, but neither this boundary nor a
                // gateway replay can execute it twice.
                if !world.inbound(claim.input.into()).await {
                    tracing::warn!(
                        request = %request,
                        "world closed after legacy release; command was not executed"
                    );
                }
            }
            ToServer::Control(ctrl) => {
                world.inbound(Inbound::Control(ctrl)).await;
            }
            ToServer::Request { id, req } => {
                match service_requests.try_admit() {
                    Ok(permit) => {
                        let world = world.clone();
                        // The `Reply` is must-deliver: `SocketSink::send` tries the
                        // bounded `to_gw` and, on a full channel, tears the boundary
                        // down rather than dropping it (a dropped Reply would hang the
                        // caller's oneshot — I1).
                        let sink = reply_sink.clone();
                        tokio::spawn(async move {
                            // Hold the owned permit through response enqueue; task
                            // count and correlated server custody are therefore both
                            // bounded by the same hard protocol ceiling.
                            let _permit = permit;
                            let resp = handle_request(&world, req).await;
                            sink.send(ToGateway::Reply { id, resp });
                        });
                    }
                    Err(resp) => {
                        // The raw peer exceeded bounded admission. It still gets
                        // exactly one correlated, must-deliver reply; no waiter is
                        // stranded and no detached handler is spawned.
                        reply_sink.send(ToGateway::Reply { id, resp });
                    }
                }
            }
            // M16c re-attach: the gateway replays its full held-online roster
            // after (re)connecting to a freshly booted server. Rebuild each
            // connection's world-side state as a SILENT ConnUp and push each
            // session a fresh `look` (D2) — the reloaded world (objects +
            // durable module state from Redis) is intact; only the RAM-only
            // connection map needs restoring.
            ToServer::ReAnnounce(recs) => {
                if recs.len() > MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS {
                    metrics.ipc_reannounce_protocol_violation();
                    tracing::error!(
                        records = recs.len(),
                        cap = MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS,
                        "peer sent an oversized ReAnnounce; tearing down the boundary before applying any record"
                    );
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "ReAnnounce has {} records; protocol cap is {}",
                            recs.len(),
                            MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS
                        ),
                    ));
                }
                let recs = recs
                    .into_iter()
                    .map(|r| world_task::ReAttachRec {
                        conn: r.conn,
                        actor: r.actor,
                        caps_structured: r.caps_structured,
                    })
                    .collect();
                if !world.reattach(recs).await {
                    tracing::error!(
                        "world refused ReAnnounce; tearing down boundary so the gateway can replay its retained roster"
                    );
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "world unavailable or refused ReAnnounce",
                    ));
                }
            }
            // Per-connection session bookkeeping — the forked server keeps no
            // session mirror (the gateway owns it), so accept and ignore.
            ToServer::Attach { .. } | ToServer::Detach { .. } => {}
        }
    }
    Ok(())
}

/// Answer one [`ServiceReq`] by calling the world handle. Each maps to an
/// existing ack-gated / oneshot world-task round trip; the await happens here
/// (off the world loop), never inside `run`.
async fn handle_request(world: &WorldHandle, req: ServiceReq) -> ServiceResp {
    match req {
        ServiceReq::CreatePlayer { name } => {
            create_player_response(world.create_player(&name).await)
        }
        ServiceReq::NameFree { name } => match world.name_free(&name).await {
            Ok(free) => ServiceResp::NameFree(free),
            Err(WorldReadError::Busy) => ServiceResp::Busy,
            Err(WorldReadError::Unavailable) => ServiceResp::Unavailable,
        },
        ServiceReq::InInputContext { actor } => match world.in_input_context(actor).await {
            Ok(active) => ServiceResp::InInputContext(active),
            Err(WorldReadError::Busy) => ServiceResp::Busy,
            Err(WorldReadError::Unavailable) => ServiceResp::Unavailable,
        },
        ServiceReq::ConnectionReady { conn, actor } => {
            match world.connection_ready(conn, actor).await {
                Ok(ready) => ServiceResp::ConnectionReady(ready),
                Err(WorldReadError::Busy) => ServiceResp::Busy,
                Err(WorldReadError::Unavailable) => ServiceResp::Unavailable,
            }
        }
        ServiceReq::LegalActions {
            conn,
            actor,
            subject,
        } => match world.legal_actions(conn, actor, subject).await {
            Ok(offers) => ServiceResp::LegalActions(Ok(offers)),
            Err(LegalActionReadError::Busy) => ServiceResp::Busy,
            Err(LegalActionReadError::Unavailable) => ServiceResp::Unavailable,
            Err(LegalActionReadError::InvalidSubject) => {
                ServiceResp::LegalActions(Err(LegalActionsError::InvalidSubject))
            }
        },
        ServiceReq::ActionStatus {
            conn,
            actor,
            status,
        } => match world.action_status(conn, actor, &status).await {
            Ok(status) => ServiceResp::ActionStatus(status),
            Err(WorldReadError::Busy) => ServiceResp::Busy,
            Err(WorldReadError::Unavailable) => ServiceResp::Unavailable,
        },
        ServiceReq::Barrier => barrier_response(world.barrier_result().await),
    }
}

/// Map the world task's barrier completion bit without ever promoting failure
/// into an acknowledgement. Shared by split and in-process links so the two
/// deployments cannot drift on this durability/ordering boundary.
fn barrier_response(result: Result<(), WorldReadError>) -> ServiceResp {
    match result {
        Ok(()) => ServiceResp::Barrier,
        Err(WorldReadError::Busy) => ServiceResp::Busy,
        Err(WorldReadError::Unavailable) => ServiceResp::Unavailable,
    }
}

/// Map player creation without collapsing retryable CommitPending overload
/// into a false store failure. The protocol already has a request-level Busy
/// disposition, so no wire-enum extension/version bump is needed.
fn create_player_response(result: Result<mush_core::ObjId, CreatePlayerErr>) -> ServiceResp {
    match result {
        Ok(id) => ServiceResp::CreatePlayer(Ok(id)),
        Err(CreatePlayerErr::Busy) => ServiceResp::Busy,
        Err(CreatePlayerErr::NameTaken) => {
            ServiceResp::CreatePlayer(Err(mush_ipc::CreatePlayerError::NameTaken))
        }
        Err(CreatePlayerErr::World(detail)) => {
            ServiceResp::CreatePlayer(Err(mush_ipc::CreatePlayerError::World(detail)))
        }
        Err(CreatePlayerErr::WorldGone) => {
            ServiceResp::CreatePlayer(Err(mush_ipc::CreatePlayerError::WorldGone))
        }
        Err(CreatePlayerErr::StoreUnavailable) => {
            ServiceResp::CreatePlayer(Err(mush_ipc::CreatePlayerError::StoreUnavailable))
        }
    }
}

#[cfg(test)]
mod service_request_tests {
    use super::{
        CreatePlayerErr, GatewayInput, GatewayRequest, MAX_IN_FLIGHT_GATEWAY_INPUTS_PER_CONNECTION,
        MAX_IN_FLIGHT_SERVICE_REQUESTS, ServerClaimAdmission, ServerInputClaims,
        ServiceRequestLimiter, ServiceResp, WorldReadError, barrier_response,
        create_player_response,
    };
    use mush_core::ObjId;

    #[test]
    fn failed_world_barrier_is_never_promoted_to_an_ack() {
        assert_eq!(barrier_response(Ok(())), ServiceResp::Barrier);
        assert_eq!(
            barrier_response(Err(WorldReadError::Busy)),
            ServiceResp::Busy
        );
        assert_eq!(
            barrier_response(Err(WorldReadError::Unavailable)),
            ServiceResp::Unavailable
        );
    }

    #[test]
    fn commit_pending_player_create_maps_to_request_busy() {
        assert_eq!(
            create_player_response(Err(CreatePlayerErr::Busy)),
            ServiceResp::Busy
        );
    }

    /// The server independently defends against a nonconforming/raw peer: at
    /// most the fixed number of handler permits can exist, the next admission
    /// gets a typed Busy response, and finishing one handler reopens exactly a
    /// slot instead of permanently wedging service traffic.
    #[test]
    fn detached_service_handlers_have_a_hard_admission_ceiling() {
        let limiter = ServiceRequestLimiter::new();
        let mut permits = Vec::with_capacity(MAX_IN_FLIGHT_SERVICE_REQUESTS);
        for _ in 0..MAX_IN_FLIGHT_SERVICE_REQUESTS {
            permits.push(limiter.try_admit().expect("slot below the ceiling"));
        }
        assert_eq!(limiter.slots.available_permits(), 0);
        assert!(matches!(limiter.try_admit(), Err(ServiceResp::Busy)));

        drop(permits.pop());
        assert!(
            limiter.try_admit().is_ok(),
            "a completed request must release one handler slot"
        );
    }

    fn legacy_request(sequence: u64, conn: u64) -> GatewayRequest {
        GatewayRequest::new(
            19,
            sequence,
            GatewayInput::Cmd {
                conn,
                actor: ObjId(7),
                line: format!("say {sequence}"),
            },
        )
    }

    #[test]
    fn legacy_claim_is_exact_bounded_and_released_before_reuse() {
        let mut claims = ServerInputClaims::default();
        let first = legacy_request(1, 9);
        assert_eq!(claims.admit(first.clone()), ServerClaimAdmission::New);
        assert_eq!(
            claims.admit(first.clone()),
            ServerClaimAdmission::Duplicate,
            "an exact replay must re-emit the claim without taking more custody"
        );

        let mut conflicting = first.clone();
        conflicting.input = GatewayInput::Cmd {
            conn: 9,
            actor: ObjId(7),
            line: "different payload".to_string(),
        };
        assert_eq!(
            claims.admit(conflicting),
            ServerClaimAdmission::Conflict,
            "one UID may never identify two payloads"
        );

        assert_eq!(claims.release(first.uid), Some(first));
        assert!(claims.entries.is_empty());
        assert!(claims.per_connection.is_empty());
        assert_eq!(claims.bytes, 0);
    }

    #[test]
    fn raw_peer_cannot_exceed_per_connection_legacy_claim_cap() {
        let mut claims = ServerInputClaims::default();
        for sequence in 0..MAX_IN_FLIGHT_GATEWAY_INPUTS_PER_CONNECTION as u64 {
            assert_eq!(
                claims.admit(legacy_request(sequence, 9)),
                ServerClaimAdmission::New
            );
        }
        assert_eq!(
            claims.admit(legacy_request(10_000, 9)),
            ServerClaimAdmission::Full { warn: true }
        );
        assert_eq!(
            claims.admit(legacy_request(10_001, 9)),
            ServerClaimAdmission::Full { warn: false },
            "continued overload is edge-logged rather than log-amplified"
        );

        let released = claims.release(legacy_request(0, 9).uid).unwrap();
        assert_eq!(released.input.conn(), 9);
        assert_eq!(
            claims.admit(legacy_request(10_002, 9)),
            ServerClaimAdmission::New,
            "release must restore exactly one slot"
        );
    }
}

#[cfg(test)]
mod writer_tests {
    //! Deterministic proofs of the two M18 server->gateway teardown paths in
    //! [`writer_loop`] (section B teardown-notify + section C write timeout),
    //! isolated from a live world/gateway. The re-attach *survival* that rides
    //! these teardowns is proven by the gateway `restart_survival` suite (any
    //! boundary EOF — which a teardown produces by dropping the writer's socket
    //! half — heals via supervise -> relaunch -> ReAnnounce -> re-attach).

    use std::io;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::Duration;

    use tokio::io::AsyncWrite;

    use super::{Arc, BoxWrite, Metrics, Notify, ToGateway, mpsc, writer_loop};

    /// An [`AsyncWrite`] whose writes NEVER complete — a connected peer that is
    /// not draining. Every `write_frame` against it pends until the writer's
    /// timeout elapses.
    struct WedgedWriter;

    impl AsyncWrite for WedgedWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Pending
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Pending
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Pending
        }
    }

    /// A `to_gw` overflow notifies `teardown`; the writer's drain `select!` takes
    /// that arm and returns (dropping its socket half → boundary EOF). The peer
    /// here drains fine, so no write times out — only the teardown ends it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn teardown_notify_returns_the_writer() {
        let (a, b) = tokio::io::duplex(64 * 1024);
        // Drain the peer end so the Hello + any frame never blocks.
        let drain = tokio::spawn(async move {
            let mut b = b;
            let mut buf = [0u8; 4096];
            loop {
                use tokio::io::AsyncReadExt;
                if b.read(&mut buf).await.unwrap_or(0) == 0 {
                    break;
                }
            }
        });
        let (_tx, rx) = mpsc::channel::<ToGateway>(8);
        let metrics = Arc::new(Metrics::new());
        let teardown = Arc::new(Notify::new());
        let w: BoxWrite = Box::new(a);
        let join = tokio::spawn(writer_loop(
            w,
            rx,
            vec![],
            vec![],
            Arc::clone(&metrics),
            Arc::clone(&teardown),
            Duration::from_secs(30),
        ));
        // Simulate the SocketSink hitting a full channel.
        teardown.notify_one();
        tokio::time::timeout(Duration::from_secs(5), join)
            .await
            .expect("writer must return on a teardown notify")
            .expect("writer join");
        // No frame ever timed out — the teardown, not a wedged write, ended it.
        assert_eq!(metrics.snapshot().ipc_write_timeouts_total, 0);
        drain.abort();
    }

    /// A wedged peer (writes never complete) trips the per-frame write timeout:
    /// the writer counts it (`ipc_write_timeouts_total`), warns, and returns —
    /// tearing the boundary down rather than blocking forever. The Hello write
    /// is the first to hit the wedged socket, so the timeout fires there.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wedged_peer_trips_the_write_timeout() {
        let (_tx, rx) = mpsc::channel::<ToGateway>(8);
        let metrics = Arc::new(Metrics::new());
        let teardown = Arc::new(Notify::new());
        let w: BoxWrite = Box::new(WedgedWriter);
        let join = tokio::spawn(writer_loop(
            w,
            rx,
            vec![],
            vec![],
            Arc::clone(&metrics),
            Arc::clone(&teardown),
            Duration::from_millis(100),
        ));
        tokio::time::timeout(Duration::from_secs(5), join)
            .await
            .expect("writer must return after the write timeout")
            .expect("writer join");
        assert_eq!(
            metrics.snapshot().ipc_write_timeouts_total,
            1,
            "the wedged write must be counted"
        );
    }
}

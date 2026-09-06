//! mushd — the rustmush server binary.
//!
//! M2: boots the persistent store (Redis authoritative, fail-fast when
//! unreachable), runs app-level migrations, imports a leftover M1
//! `accounts.json`, loads (or seeds) the world, then starts the
//! single-writer world task with its write-through store-writer, the
//! session layer, the store-backed accounts, and the SSH bridge. Runs until
//! Ctrl-C, SIGTERM (Unix), or an in-game `@shutdown`, then shuts down in
//! order: announce → world task ends → writer
//! drains → `store.drain()` → eligible BGSAVE → exit. The final snapshot is
//! skipped if the writer did not drain or persistence integrity is latched.
//! `--repl` preserves the M0
//! synchronous stdin REPL for offline poking (fully in-memory, no store).
//!
//! Usage: `mushd [--repl] [--config <path>]` (config defaults to
//! `mushd.toml` in the working directory; a missing file means defaults).

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use mush_config::offer_keys::DurableOfferKeyStore;
use mush_config::resolution_keys::DurableResolutionKeyStore;
use mush_core::{ControlMsg, Engine, Inbound};
use mush_llm::LlmService;
use mush_net_web::{Metrics, WebConfig};
use mush_server::config_ext::{
    economy_config_with, llm_config, onboarding_ruleset, shadowrun_cadences,
    shadowrun_economy_config, tacsim_config, weather_config, wod_economy_config, worldgen_config,
};
use mush_session::{SessionConfig, SessionManager};
use mush_store::{
    InMemoryStore, LedgerArchive, ObjectStore, RedisStore, core_migrations, run_migrations,
};
use serde::Deserialize;

use mushd::accounts::{Accounts, import_accounts_json};
use mushd::config::{Config, StoreBackend};
use mushd::ipguard::IpGuard;
use mushd::modules;
use mushd::persist::{load_or_seed_world, reconcile_authoritative_for_writable_boot};
use mushd::quarantine::QuarantineSpool;
use mushd::ssh_bridge::{SshBridge, start_listener};
use mushd::staff::{StaffQueue, register_server_commands_with_cap};
use mushd::web_bridge::WebBridge;
use mushd::world_task::{self, ServerDeps};
use mushd::{repl, unix_now_s};

/// How long the shutdown sequence waits for the writer to drain before
/// giving up. It only takes this long when the store is down; queued
/// batches that could not be flushed are then lost (loudly logged) — the
/// same bounded window a `kill -9` during the outage would lose.
const WRITER_DRAIN_TIMEOUT: Duration = Duration::from_secs(15);

/// Per-step bound on the shutdown tail's store calls (drain barrier,
/// BGSAVE, close). With the store down these would otherwise pend until a
/// reconnect that may never come, wedging shutdown until systemd's
/// SIGKILL. The store client's own command timeout usually fires first;
/// this is the belt to that suspender.
const SHUTDOWN_STORE_TIMEOUT: Duration = Duration::from_secs(10);

/// An ordinary shutdown snapshot is safe only after every queued write has a
/// terminal result and no component observed an integrity breach.
const fn shutdown_bgsave_allowed(writer_drained: bool, integrity_latched: bool) -> bool {
    writer_drained && !integrity_latched
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("Usage: mushd [--repl] [--config <path>]");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--repl") {
        repl::run();
        return ExitCode::SUCCESS;
    }

    // Config is loaded first because the `[log]` section drives the
    // subscriber. Config::load's own tracing calls (before the subscriber is
    // installed) are therefore uncaptured, but its errors go to stderr, so a
    // bad config still reports cleanly.
    let config_path = args
        .iter()
        .position(|a| a == "--config")
        .and_then(|i| args.get(i + 1))
        .map_or_else(|| PathBuf::from("mushd.toml"), PathBuf::from);
    let config = match Config::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("mushd: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Validate the integrity store before logging is allowed to create or
    // append any startup file. Retain this exact instance (and its exclusive
    // lifetime lock) through recovery, ordinary boot, and the writer.
    let quarantine_spool = match QuarantineSpool::open(config.server.data_dir.join("quarantine")) {
        Ok(spool) => spool,
        Err(error) => {
            eprintln!("mushd: quarantine spool preflight failed: {error}");
            return ExitCode::FAILURE;
        }
    };

    // Structured logging per `[log]` (level/format + optional daily-rotating
    // file output). The guard keeps the non-blocking file appender's worker
    // alive for the whole process; it drops at the end of `main`.
    let _log_guards = mushd::logging::init(&config.log, &config.server.data_dir);

    // Dev convenience: load `.env` from the working directory (or an ancestor)
    // into the process environment so LLM API keys resolve without exporting
    // them by hand. Production sets real env vars and ships no `.env`, so a
    // missing file is normal and silently ignored. `dotenvy::dotenv()` does NOT
    // override variables already set (real env wins), and never returns or logs
    // a value. Called here — before the tokio runtime is built — so the
    // (edition-2024-unsafe) `set_var` it performs internally runs
    // single-threaded. Keys are read later by name in `LlmService::from_config`.
    match dotenvy::dotenv() {
        Ok(path) => tracing::info!(path = %path.display(), "loaded .env"),
        Err(e) if e.not_found() => {}
        Err(e) => tracing::warn!(error = %e, "ignoring unreadable .env"),
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    match runtime.block_on(serve(config, quarantine_spool)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mushd: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Open the configured store backend, failing fast with an operator-facing
/// error when Redis is unreachable (PLAN: never run without persistence).
/// With `[store.archive].enabled`, the ledger archive tier (J5) is opened
/// under the data dir and attached, so `log:` reads fall through to it; the
/// archive is returned beside the store for the nightly pass to write.
async fn open_store(
    config: &Config,
) -> Result<(Arc<dyn ObjectStore>, Option<Arc<LedgerArchive>>), String> {
    let archive = if config.store.archive.enabled {
        let root = config.server.data_dir.join(&config.store.archive.dir);
        let archive = LedgerArchive::open(&root)
            .map_err(|e| format!("opening the ledger archive at {}: {e}", root.display()))?;
        let stats = archive.stats();
        tracing::info!(
            root = %root.display(),
            chunks = stats.chunks,
            purged = stats.purged,
            compressed_mb = stats.compressed_bytes / (1024 * 1024),
            "ledger archive tier attached"
        );
        Some(Arc::new(archive))
    } else {
        None
    };
    let store: Arc<dyn ObjectStore> = match config.store_backend().map_err(|e| e.to_string())? {
        StoreBackend::Redis => {
            let url = &config.store.redis_url;
            tracing::info!(%url, "connecting to Redis");
            let store = RedisStore::connect_read_only(url).await.map_err(|e| {
                format!(
                    "cannot reach Redis at {url}: {e}\n\
                     mushd requires its store at boot. Start Redis (or point \
                     [store].redis_url at it), or set [store].store = \"memory\" \
                     for a throwaway world."
                )
            })?;
            match &archive {
                Some(archive) => Arc::new(store.with_archive(Arc::clone(archive))),
                None => Arc::new(store),
            }
        }
        StoreBackend::Memory => {
            tracing::warn!("[store].store = \"memory\": nothing will survive a restart");
            match &archive {
                Some(archive) => Arc::new(InMemoryStore::new().with_archive(Arc::clone(archive))),
                None => Arc::new(InMemoryStore::new()),
            }
        }
    };
    Ok((store, archive))
}

async fn serve(config: Config, mut quarantine_spool: QuarantineSpool) -> Result<(), String> {
    let listen = config.ssh_listen().map_err(|e| e.to_string())?;
    let data_dir = config.server.data_dir.clone();
    // The pre-opened spool still owns the single-deployment lock. Compare any
    // valid active records through the store's read-only connection before
    // migrations, account import, world seeding, or module boot can write.
    mush_server::prepare_quarantine_for_writable_boot(&config, &mut quarantine_spool, None).await?;
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
    tracing::info!(
        name = %config.server.name,
        data_dir = %data_dir.display(),
        %listen,
        "mushd {} starting",
        env!("CARGO_PKG_VERSION"),
    );

    // The one shared metrics registry (PLAN "Observability"): the transports
    // bump connection counters, the world task publishes the world-owned
    // gauges + LLM/NPC aggregates each sample, the store writer stamps
    // write-queue depth and last-ack age, and the web ingress serves it back
    // over /healthz and /metrics.
    let metrics = Arc::new(Metrics::new());

    // Store: connect (fail fast), migrate, one-time accounts.json import,
    // then load or seed the world — all before anything serves.
    let (store, ledger_archive) = open_store(&config).await?;
    let authoritative = store.authoritative_store().ok_or_else(|| {
        "configured store does not implement K3 exact-token durability; refusing writable boot"
            .to_owned()
    })?;
    let recovery = reconcile_authoritative_for_writable_boot(authoritative).await?;
    tracing::info!(
        indexed = recovery.indexed,
        retained = recovery.retained,
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
    let version = run_migrations(&*store, &core_migrations())
        .await
        .map_err(|e| format!("store migrations failed: {e}"))?;
    tracing::info!(schema = version, "store schema current");
    let imported = import_accounts_json(&*store, &data_dir).await?;
    if imported > 0 {
        tracing::info!(imported, "M1 accounts.json imported");
    }
    // Command table: core + engine + server staff commands (@siteban/@dump/
    // @boot/@ps/@stats push typed ops onto the queue the world task
    // drains), plus the markdown help topics.
    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands_with_cap(&mut registry, &staff, config.game.max_characters as usize)
        .map_err(|e| format!("server command registration: {e}"))?;
    match mushd::help::load_help_topics(&mut registry, &config.server.help_dir) {
        Ok(n) => tracing::info!(topics = n, "help topics loaded"),
        Err(e) => tracing::warn!(error = %e, dir = %config.server.help_dir.display(),
            "help topics failed to load; continuing without them"),
    }
    // M7 LLM service: built from [llm] config, resolving each provider's API
    // key from its named env var (never inlined in config, never logged). A
    // keyless clone still boots — `enabled` defaults false, and a provider
    // whose key env-var is unset is skipped (routes to it fail fast). The
    // worldgen module and the world task's `Effect::ModuleAsync` driver share
    // this one handle.
    let llm_cfg = llm_config(&config.llm)?;
    let llm = Arc::new(LlmService::from_config(&llm_cfg).map_err(|e| format!("llm config: {e}"))?);
    tracing::info!(
        enabled = llm_cfg.enabled,
        providers = llm_cfg.providers.len(),
        routes = llm_cfg.routes.len(),
        "llm service built"
    );

    // Game modules: the built-ins (including sim + tactics), then
    // the M7 worldgen module, then the config-selected rulesets
    // ([rulesets].enabled — M6: "wod"). Everything after the built-ins
    // registers cleanly against the one registry; rulesets go last so their
    // approval payload handlers see a loaded jobs module. Then run each
    // namespace's store migrations and hand each module its persisted boot
    // data — all before the world task starts serving.
    let onboarding = onboarding_ruleset(&config.onboarding);
    // E5c: `[wod.economy]`, validated here (an unknown mode is a hard boot error).
    // `None` when no WoD ruleset is enabled — that is what gates the housing title
    // issuer (08d §2).
    let wod_economy = wod_economy_config(&config.wod)?;
    let wod_selected = config
        .rulesets
        .enabled
        .iter()
        .any(|r| r.trim().eq_ignore_ascii_case("wod"));
    // E6a: `[shadowrun.economy]`, validated here (an unknown lifestyle tier is a hard
    // boot error). `None` when no Shadowrun ruleset is enabled — that is what gates
    // the run board's economy observer (08f §2's `issuers` pattern).
    let sr_economy = shadowrun_economy_config(&config.shadowrun)?;
    let sr_selected = config
        .rulesets
        .enabled
        .iter()
        .any(|r| r.trim().eq_ignore_ascii_case("shadowrun"));
    // SP1 / D-LBK-7: a Shadowrun server needs `sr.month` declared or no lifestyle
    // obligation can exist. The host contributes the stanza unless the operator
    // declared one of that name themselves.
    let contributed: Vec<(String, mush_server::config_ext::DeclaredCadence)> = if sr_selected {
        shadowrun_cadences(&sr_economy)
    } else {
        Vec::new()
    };
    let contributed: Vec<(&str, mush_server::config_ext::DeclaredCadence)> = contributed
        .iter()
        .map(|(n, d)| (n.as_str(), d.clone()))
        .collect();
    let mut mods = modules::server_modules_with(
        weather_config(&config.weather),
        economy_config_with(&config.economy, &contributed)?,
        &onboarding,
        wod_selected.then_some(&wod_economy),
        sr_selected.then_some(&sr_economy),
    );
    mods.push(modules::worldgen_module_with(
        Arc::clone(&llm),
        worldgen_config(&config.worldgen),
    ));
    // M8: the NPC agent module, grounded by the enabled ruleset's sheet +
    // on-loop abilities through the generic ruleset grounding adapter (WoD,
    // Shadowrun, …); dialogue-only when no ruleset is enabled.
    let npc_grounding = modules::npc_agent_adapter(&config.rulesets.enabled);
    mods.push(modules::npc_module_with(npc_grounding, &config.npc));
    let rulesets = modules::ruleset_modules(
        &config.rulesets.enabled,
        &onboarding,
        &wod_economy,
        &sr_economy,
    )
    .map_err(|e| format!("ruleset selection: {e}"))?;
    if !rulesets.is_empty() {
        tracing::info!(rulesets = ?config.rulesets.enabled, "rulesets enabled");
    }
    mods.extend(rulesets);
    // E7 cut 1: the TacSim layer, **after** the character ruleset (doc 10 §20, 10b
    // §3.1). It is not a `[rulesets]` id — `[tacsim].enabled` switches it, independently
    // of which single character ruleset is loaded — and it composes last so its boot
    // can read that ruleset's facts. `tacsim_config` reads every configured pack file
    // here, host-side, so an unreadable, oversized or digest-mismatched pack fails the
    // boot naming the file before any module runs (invariant (iv)).
    let tacsim = tacsim_config(&config.tacsim)?;
    if tacsim.enabled {
        tracing::info!(
            packs = tacsim.packs.len(),
            seeds = tacsim.seeds.len(),
            "tacsim enabled"
        );
    }
    mods.push(modules::tacsim_module(tacsim));
    // Compose and freeze every mechanics registry before persisted world data
    // is loaded. This makes the C1 boot order structural instead of relying on
    // individual modules to avoid early interpretation.
    let composition =
        Engine::compose_modules(registry, mods).map_err(|e| format!("module registration: {e}"))?;
    let mut world = load_or_seed_world(&*store).await?;
    // Hydrate the versioned IC timeline before module state and ticks are
    // materialized into the engine. [game] seeds an empty store only;
    // persisted epochs remain authoritative across configuration changes.
    let clock = config.game.game_clock().map_err(|e| e.to_string())?;
    mush_server::timeline::hydrate_world_timeline(&mut world, &*store, clock).await?;
    tracing::info!(
        generation = world.world_timeline().current().generation,
        ic_rate = world.world_timeline().current().rate,
        "world timeline installed",
    );
    let mut engine = Engine::from_composition(world, composition);
    engine.set_offer_keys(offer_keys);
    engine.set_resolution_keys(resolution_keys);
    modules::boot_modules(&mut engine, &*store).await?;
    // J5: the namespaces the nightly archive pass walks — every composed module's.
    let archive_namespaces: Vec<String> = engine
        .modules()
        .iter()
        .map(|m| m.name().to_string())
        .collect();

    // The web hello manifest, collected while the registry is still in
    // hand: every module-registered panel feed plus the engine-side "who",
    // and one feature id per module (comsys advertises as "channels").
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

    // Session layer.
    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: Duration::from_secs(config.game.linkdead_grace_s),
        ..SessionConfig::default()
    }));

    // Abuse protection, seeded with the persisted sitebans.
    let guard = Arc::new(IpGuard::new());
    let bans = store
        .get_bans()
        .await
        .map_err(|e| format!("loading sitebans: {e}"))?;
    let ban_count = bans.len();
    guard.load_sitebans(bans.into_iter().filter_map(|b| {
        b.ip.parse()
            .map(|ip| (ip, b.until_unix))
            .map_err(|e| tracing::error!(ip = b.ip, error = %e, "unparseable persisted ban"))
            .ok()
    }));
    if ban_count > 0 {
        tracing::info!(sitebans = ban_count, "persisted sitebans loaded");
    }

    // Gateway-side control hooks (programme D3): the Discord bridge registers
    // into this when it starts, below; the in-process sink routes the server's
    // `discord_control` through it.
    let controls = Arc::new(mushd::controls::GatewayControls::default());

    // The single-writer world task + its store writer.
    let handles = world_task::spawn_persist_with_decision_workers(
        engine,
        Arc::clone(&sessions),
        ServerDeps {
            store: Arc::clone(&store),
            guard: Arc::clone(&guard),
            staff,
            llm: Some(Arc::clone(&llm)),
            metrics: Arc::clone(&metrics),
            // E0 quarantine spool: a separate durable failure domain under the
            // data dir for confirmed-poison ledger seals.
            quarantine_spool,
        },
        llm_cfg.decision_workers,
        Arc::clone(&controls),
    )?;
    let persistence_integrity = handles.persistence_integrity.clone();
    let world = handles.handle;
    let mut world_join = handles.world_join;
    let writer_join = handles.writer_join;
    let outbox_join = handles.outbox_join;

    // J5: the ledger archive pass, once a UTC day inside its window, beside
    // the store writer and never on the world task.
    if let Some(archive) = ledger_archive {
        let cfg = &config.store.archive;
        let (window_start_min, window_end_min) =
            mush_server::archive::ArchiveConfig::parse_window(&cfg.window)?;
        let job = mush_server::archive::ArchiveConfig {
            window_start_min,
            window_end_min,
            retain_hours: cfg.retain_hours,
            high_watermark_bytes: cfg.high_watermark_mb.saturating_mul(1024 * 1024),
            low_watermark_bytes: cfg.low_watermark_mb.saturating_mul(1024 * 1024),
            segment_chunks: cfg.segment_chunks,
            max_purge_per_pass: cfg.max_purge_per_pass,
            namespaces: archive_namespaces,
        };
        tracing::info!(
            window = %cfg.window,
            retain_hours = cfg.retain_hours,
            high_watermark_mb = cfg.high_watermark_mb,
            low_watermark_mb = cfg.low_watermark_mb,
            "ledger archive pass scheduled"
        );
        let _archive_job = mush_server::archive::spawn_archive_job(
            Arc::clone(&store),
            archive,
            job,
            Arc::clone(&metrics),
        );
    }

    // Store-backed auth + the SSH hooks bridge.
    let accounts = Arc::new(Accounts::new(
        Arc::clone(&store),
        world.clone(),
        config.game.max_characters as usize,
    ));
    let bridge = Arc::new(
        SshBridge::new(
            Arc::clone(&accounts),
            Arc::clone(&sessions),
            world.clone(),
            Arc::clone(&guard),
        )
        .with_metrics(Arc::clone(&metrics)),
    );
    let listener = start_listener(listen, &data_dir, banner(&config.server.name), bridge)
        .await
        .map_err(|e| format!("SSH listener on {listen}: {e}"))?;

    // The web ingress (M4): same accounts, same session manager, same
    // world handle, same guard — plus the token store and the panel
    // manifest. Served by mush-net-web on its own task; stopped gracefully
    // in the shutdown tail.
    let mut web_config =
        WebConfig::deserialize(config.web.clone()).map_err(|e| format!("[web] config: {e}"))?;
    web_config.server_name = format!(
        "{} (rustmush {})",
        config.server.name,
        env!("CARGO_PKG_VERSION")
    );
    web_config.features = features;
    web_config.panels = panels.clone();
    let web_listen = web_config.listen;
    let web_bridge: Arc<dyn mush_net_web::WebHooks> = Arc::new(
        WebBridge::new(
            Arc::clone(&accounts),
            Arc::clone(&sessions),
            world.clone(),
            Arc::clone(&guard),
            Arc::clone(&store),
            panels,
        )
        .with_metrics(Arc::clone(&metrics)),
    );
    let (web_stop, web_stop_rx) = tokio::sync::oneshot::channel::<()>();
    let web_join = tokio::spawn(async move {
        // A web bind/serve failure is loud but non-fatal: SSH players keep
        // playing while the operator fixes the [web] config.
        if let Err(e) = mush_net_web::serve_with_shutdown(web_bridge, web_config, async move {
            let _ = web_stop_rx.await;
        })
        .await
        {
            tracing::error!(error = %e, "web server failed");
        }
    });

    // The Discord bot (D1/D2): same accounts, session manager, world handle,
    // guard and store as the SSH and web paths; off unless `[discord].enabled`,
    // and loud but non-fatal when it cannot start.
    let discord = mushd::discord_bridge::spawn_bot(
        &config,
        Arc::clone(&accounts),
        Arc::clone(&sessions),
        Arc::new(world.clone()) as Arc<dyn mush_ipc::GatewayLink>,
        Arc::clone(&guard),
        Arc::clone(&store) as Arc<dyn ObjectStore>,
        &controls,
    );

    tracing::info!(ssh = %listener.addr(), web = %web_listen, "up; press Ctrl-C to shut down");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Ctrl-C: shutting down");
            // Stop accepting first (PLAN shutdown order), then announce +
            // stop the loop; the world task delivers the shutdown notice
            // to every connection before exiting.
            listener.stop_accepting();
            world.inbound(Inbound::Control(ControlMsg::Shutdown)).await;
            let _ = world_join.await;
        }
        // systemd stop / `kill <pid>` on the production box (PLAN gap
        // closures: the shutdown sequence "also SIGTERM"). Never resolves
        // on Windows.
        () = terminate_signal() => {
            tracing::info!("SIGTERM: shutting down");
            listener.stop_accepting();
            world.inbound(Inbound::Control(ControlMsg::Shutdown)).await;
            let _ = world_join.await;
        }
        // The world task can also end itself (in-game `@shutdown`): it has
        // already stepped ControlMsg::Shutdown and announced.
        _ = &mut world_join => {
            tracing::info!("world task ended on its own");
            listener.stop_accepting();
        }
    }
    // Stop the web ingress (graceful: in-flight requests finish; WS conns
    // saw the shutdown announce before the world task exited).
    let _ = web_stop.send(());
    if tokio::time::timeout(Duration::from_secs(5), web_join)
        .await
        .is_err()
    {
        tracing::warn!("web server did not stop in time; continuing shutdown");
    }
    discord.stop().await;
    // Shutdown order (PLAN gap closures): the world task has stopped
    // mutating; its dropped job sender lets the writer drain every pending
    // batch. Only a cleanly drained writer with no integrity breach may be
    // followed by the ordinary final BGSAVE.
    drop(world);
    if tokio::time::timeout(WRITER_DRAIN_TIMEOUT, outbox_join)
        .await
        .is_err()
    {
        tracing::error!("durable outbox worker did not stop in time");
    }
    let writer_drained = match tokio::time::timeout(WRITER_DRAIN_TIMEOUT, writer_join).await {
        Ok(Ok(())) => true,
        Ok(Err(e)) => {
            tracing::error!(error = %e, "store writer task failed during shutdown");
            false
        }
        Err(_) => {
            tracing::error!(
                "store writer did not drain in time (store down?); exiting anyway — \
                 any batches still queued behind the outage are lost"
            );
            false
        }
    };
    // Each tail step is individually bounded so a dead store can never
    // wedge shutdown (systemd would otherwise escalate to SIGKILL).
    match tokio::time::timeout(SHUTDOWN_STORE_TIMEOUT, store.drain()).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::error!(error = %e, "store drain failed"),
        Err(_) => tracing::error!("store drain timed out (store down?)"),
    }
    let integrity_latched = persistence_integrity.is_latched();
    if shutdown_bgsave_allowed(writer_drained, integrity_latched) {
        match tokio::time::timeout(SHUTDOWN_STORE_TIMEOUT, store.bgsave()).await {
            Ok(Ok(())) => tracing::info!("shutdown BGSAVE triggered"),
            Ok(Err(e)) => tracing::error!(error = %e, "shutdown BGSAVE failed"),
            Err(_) => tracing::error!("shutdown BGSAVE timed out (store down?)"),
        }
    } else if !writer_drained {
        tracing::error!(
            "shutdown BGSAVE skipped: store writer did not drain cleanly; snapshotting could \
             capture incomplete durable state"
        );
    } else {
        debug_assert!(integrity_latched);
        tracing::error!(
            "shutdown BGSAVE skipped: persistence integrity is latched; ordinary snapshots \
             are forbidden until exact recovery"
        );
    }
    if tokio::time::timeout(SHUTDOWN_STORE_TIMEOUT, store.close())
        .await
        .is_err()
    {
        tracing::error!("store close timed out (store down?)");
    }
    // Give transports a beat to flush the shutdown notice, then exit;
    // dropping the runtime tears the listener and connections down.
    tokio::time::sleep(Duration::from_millis(200)).await;
    tracing::info!("mushd exiting at {}", unix_now_s());
    Ok(())
}

/// Resolves when SIGTERM arrives (Unix only — systemd's stop signal);
/// pends forever elsewhere, so the `select!` arm simply never fires on
/// Windows.
async fn terminate_signal() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
                return;
            }
            Err(e) => tracing::error!(error = %e, "cannot install SIGTERM handler"),
        }
    }
    std::future::pending::<()>().await;
}

/// The classic welcome-screen banner (the transport appends the command
/// help below it).
fn banner(name: &str) -> String {
    let version = env!("CARGO_PKG_VERSION");
    format!(
        "------------------------------------------------------------------\n\
         \x20  {name}\n\
         \x20  a rustmush server, v{version}\n\
         \n\
         \x20  Connect to an existing account, or create a new one:\n\
         ------------------------------------------------------------------"
    )
}

#[cfg(test)]
mod tests {
    use super::shutdown_bgsave_allowed;

    #[test]
    fn shutdown_bgsave_requires_clean_drain_and_clear_integrity() {
        assert!(shutdown_bgsave_allowed(true, false));
        assert!(!shutdown_bgsave_allowed(false, false));
        assert!(!shutdown_bgsave_allowed(true, true));
        assert!(!shutdown_bgsave_allowed(false, true));
    }
}

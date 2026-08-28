//! mushd — the rustmush server binary.
//!
//! M2: boots the persistent store (Redis authoritative, fail-fast when
//! unreachable), runs app-level migrations, imports a leftover M1
//! `accounts.json`, loads (or seeds) the world, then starts the
//! single-writer world task with its write-through store-writer, the
//! session layer, the store-backed accounts, and the SSH bridge. Runs until
//! Ctrl-C, SIGTERM (Unix), or an in-game `@shutdown`, then shuts down in
//! order: announce → world task ends → writer
//! drains → `store.drain()` → BGSAVE → exit. `--repl` preserves the M0
//! synchronous stdin REPL for offline poking (fully in-memory, no store).
//!
//! Usage: `mushd [--repl] [--config <path>]` (config defaults to
//! `mushd.toml` in the working directory; a missing file means defaults).

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use mush_core::{ControlMsg, Engine, Inbound};
use mush_session::{SessionConfig, SessionManager};
use mush_store::{InMemoryStore, ObjectStore, RedisStore, core_migrations, run_migrations};
use tracing_subscriber::EnvFilter;

use mushd::accounts::{Accounts, import_accounts_json};
use mushd::config::{Config, StoreBackend};
use mushd::ipguard::IpGuard;
use mushd::modules;
use mushd::persist::load_or_seed_world;
use mushd::ssh_bridge::{SshBridge, start_listener};
use mushd::staff::{StaffQueue, register_server_commands};
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

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

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

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    match runtime.block_on(serve(config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mushd: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Open the configured store backend, failing fast with an operator-facing
/// error when Redis is unreachable (PLAN: never run without persistence).
async fn open_store(config: &Config) -> Result<Arc<dyn ObjectStore>, String> {
    match config.store_backend().map_err(|e| e.to_string())? {
        StoreBackend::Redis => {
            let url = &config.store.redis_url;
            tracing::info!(%url, "connecting to Redis");
            let store = RedisStore::connect(url).await.map_err(|e| {
                format!(
                    "cannot reach Redis at {url}: {e}\n\
                     mushd requires its store at boot. Start Redis (or point \
                     [store].redis_url at it), or set [store].store = \"memory\" \
                     for a throwaway world."
                )
            })?;
            Ok(Arc::new(store))
        }
        StoreBackend::Memory => {
            tracing::warn!("[store].store = \"memory\": nothing will survive a restart");
            Ok(Arc::new(InMemoryStore::new()))
        }
    }
}

async fn serve(config: Config) -> Result<(), String> {
    let listen = config.ssh_listen().map_err(|e| e.to_string())?;
    let data_dir = config.server.data_dir.clone();
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| format!("cannot create data dir {}: {e}", data_dir.display()))?;
    tracing::info!(
        name = %config.server.name,
        data_dir = %data_dir.display(),
        %listen,
        "mushd {} starting",
        env!("CARGO_PKG_VERSION"),
    );

    // Store: connect (fail fast), migrate, one-time accounts.json import,
    // then load or seed the world — all before anything serves.
    let store = open_store(&config).await?;
    let version = run_migrations(&*store, &core_migrations())
        .await
        .map_err(|e| format!("store migrations failed: {e}"))?;
    tracing::info!(schema = version, "store schema current");
    let imported = import_accounts_json(&*store, &data_dir).await?;
    if imported > 0 {
        tracing::info!(imported, "M1 accounts.json imported");
    }
    let mut world = load_or_seed_world(&*store).await?;
    // Install the IC game clock from [game] before the world starts serving.
    // ic_ratio / ic_epoch are read once here (hot-reload comes later).
    let clock = config.game.game_clock().map_err(|e| e.to_string())?;
    world.set_game_clock(clock);
    tracing::info!(
        ic_ratio = clock.ic_ratio,
        ic_epoch = %config.game.ic_epoch,
        "game clock installed",
    );

    // Command table: core + engine + server staff commands (@siteban/@dump/
    // @boot/@ps/@stats push typed ops onto the queue the world task
    // drains), plus the markdown help topics.
    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff)
        .map_err(|e| format!("server command registration: {e}"))?;
    match mushd::help::load_help_topics(&mut registry, &config.server.help_dir) {
        Ok(n) => tracing::info!(topics = n, "help topics loaded"),
        Err(e) => tracing::warn!(error = %e, dir = %config.server.help_dir.display(),
            "help topics failed to load; continuing without them"),
    }
    // Game modules: the built-ins (comsys, mail, jobs, weather, script) plus
    // the config-selected rulesets ([rulesets].enabled — M6: "wod"), which
    // register AFTER the built-ins so their commands/panels/job-queues wire
    // cleanly and their approval payload handlers see a loaded jobs module.
    // Then run each namespace's store migrations and hand each module its
    // persisted boot data — all before the world task starts serving.
    let mut mods = modules::server_modules();
    let rulesets = modules::ruleset_modules(&config.rulesets.enabled)
        .map_err(|e| format!("ruleset selection: {e}"))?;
    if !rulesets.is_empty() {
        tracing::info!(rulesets = ?config.rulesets.enabled, "rulesets enabled");
    }
    mods.extend(rulesets);
    let mut engine = Engine::with_modules(world, registry, mods)
        .map_err(|e| format!("module registration: {e}"))?;
    modules::boot_modules(&mut engine, &*store).await?;

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

    // The single-writer world task + its store writer.
    let handles = world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        ServerDeps {
            store: Arc::clone(&store),
            guard: Arc::clone(&guard),
            staff,
        },
    );
    let world = handles.handle;
    let mut world_join = handles.world_join;
    let writer_join = handles.writer_join;

    // Store-backed auth + the SSH hooks bridge.
    let accounts = Arc::new(Accounts::new(
        Arc::clone(&store),
        world.clone(),
        config.game.max_characters as usize,
    ));
    let bridge = Arc::new(SshBridge::new(
        Arc::clone(&accounts),
        Arc::clone(&sessions),
        world.clone(),
        Arc::clone(&guard),
    ));
    let listener = start_listener(listen, &data_dir, banner(&config.server.name), bridge)
        .await
        .map_err(|e| format!("SSH listener on {listen}: {e}"))?;

    // The web ingress (M4): same accounts, same session manager, same
    // world handle, same guard — plus the token store and the panel
    // manifest. Served by mush-net-web on its own task; stopped gracefully
    // in the shutdown tail.
    let mut web_config = config.web.clone();
    web_config.server_name = format!(
        "{} (rustmush {})",
        config.server.name,
        env!("CARGO_PKG_VERSION")
    );
    web_config.features = features;
    web_config.panels = panels.clone();
    let web_listen = web_config.listen;
    let web_bridge: Arc<dyn mush_net_web::WebHooks> = Arc::new(WebBridge::new(
        Arc::clone(&accounts),
        Arc::clone(&sessions),
        world.clone(),
        Arc::clone(&guard),
        Arc::clone(&store),
        panels,
    ));
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
    // Shutdown order (PLAN gap closures): the world task has stopped
    // mutating; its dropped job sender lets the writer drain every pending
    // batch, then the drain barrier + BGSAVE make the store current and
    // snapshotted before we exit.
    drop(world);
    if tokio::time::timeout(WRITER_DRAIN_TIMEOUT, writer_join)
        .await
        .is_err()
    {
        tracing::error!(
            "store writer did not drain in time (store down?); exiting anyway — \
             any batches still queued behind the outage are lost"
        );
    }
    // Each tail step is individually bounded so a dead store can never
    // wedge shutdown (systemd would otherwise escalate to SIGKILL).
    match tokio::time::timeout(SHUTDOWN_STORE_TIMEOUT, store.drain()).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::error!(error = %e, "store drain failed"),
        Err(_) => tracing::error!("store drain timed out (store down?)"),
    }
    match tokio::time::timeout(SHUTDOWN_STORE_TIMEOUT, store.bgsave()).await {
        Ok(Ok(())) => tracing::info!("shutdown BGSAVE triggered"),
        Ok(Err(e)) => tracing::error!(error = %e, "shutdown BGSAVE failed"),
        Err(_) => tracing::error!("shutdown BGSAVE timed out (store down?)"),
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

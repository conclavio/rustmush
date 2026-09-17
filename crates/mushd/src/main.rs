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
//! Usage: `mushd [--repl] [--check-config] [--config <path>]`.
//! `--check-config` loads the config, reports the refusals named by
//! `mush_server::boot::check_config` (itself `mush_config::preflight::check`
//! plus the world half's own) — that function is the list of record, not a
//! claim that a boot raises nothing else — and exits without opening the store
//! or the quarantine lifetime lock.
//! The config defaults to
//! `mushd.toml` in the working directory, and a missing *implicit* file means
//! defaults; an explicit `--config` that names a file which is not there is a
//! refusal, as is any argument the shared parser does not recognise (WS22-S1).

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use mush_config::cli::CliSpec;
use mush_config::exit::{ExitClass, ExitFailure};
use mush_core::{ControlMsg, Inbound};
use mush_net_web::{Metrics, WebConfig};
use mush_session::{SessionConfig, SessionManager};
use mush_store::ObjectStore;
use serde::Deserialize;

use mushd::accounts::Accounts;
use mushd::boot;
use mushd::config::Config;
use mushd::ipguard::IpGuard;
use mushd::quarantine::QuarantineSpool;
use mushd::ssh_bridge::{SshBridge, start_listener};
use mushd::web_bridge::WebBridge;
use mushd::world_task::{self, ServerDeps, WorldOutcome};
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

/// An ordinary shutdown snapshot is safe only after the world task itself
/// reached a clean exit, every queued write has a terminal result, no
/// component observed an integrity breach, and the world's own join handle
/// yielded an ordinary ending.
///
/// The world-exit term is the one that closes the panic hole: a world task that
/// unwound mid-command still drops its job sender, so the writer drains
/// perfectly and the latch stays clear — and the old two-term gate would then
/// promote whatever RAM had reached the store to the on-disk truth. A world
/// that did not finish is not a world worth snapshotting.
///
/// The world-exit term is unclean for BOTH endings that are not an ordinary
/// one: a world that faulted, and a world that never joined at all within its
/// bound. The second knows even less than the first — nothing here can say
/// whether its RAM finished reaching the store — so it refuses the snapshot for
/// the same reason and with less evidence.
///
/// The fault term is deliberately *not* folded into it, because the two facts
/// are separately derived and fail in different directions. `world_exited_cleanly`
/// is a flag the world loop publishes as its own last statement; `world_faulted`
/// is what the join handle yielded — a caught [`WorldOutcome::Faulted`] or a real
/// task panic across an `.await`, where no statement of the loop ever ran. Either
/// one alone refuses the snapshot, so neither may later be dropped as redundant.
const fn shutdown_bgsave_allowed(
    world_exited_cleanly: bool,
    writer_drained: bool,
    integrity_latched: bool,
    world_faulted: bool,
) -> bool {
    world_exited_cleanly && writer_drained && !integrity_latched && !world_faulted
}

/// Classify how the world task ended: `None` is an ordinary exit, `Some(detail)`
/// is a fault this process must not survive quietly.
///
/// `@shutdown`, Ctrl-C and SIGTERM all end the loop through one of its own
/// breaks and land on `Clean`. Anything else means the world stopped without
/// finishing: [`WorldOutcome::Faulted`] is the fault the world task's own
/// dispatch boundary caught and froze on, and a `JoinError` is the net for the
/// panics that boundary cannot cover — one across an `.await`, or a task
/// cancelled out from under this function.
///
/// Before this classification every one of those endings was `let _ = …`, so a
/// panicked world was indistinguishable from an in-game `@shutdown`: `mushd`
/// logged "world task ended on its own" at INFO and returned
/// [`ExitCode::SUCCESS`], leaving a supervisor with nothing to restart and the
/// test harness with no fault signal at all.
fn world_fault_detail(result: Result<WorldOutcome, tokio::task::JoinError>) -> Option<String> {
    match result {
        Ok(WorldOutcome::Clean) => None,
        Ok(WorldOutcome::Faulted(fault)) => {
            Some(format!("caught at {}: {}", fault.site, fault.detail.trim()))
        }
        Err(e) if e.is_panic() => Some(format!("panicked outside its dispatch boundary: {e}")),
        Err(e) => Some(format!("ended without a value: {e}")),
    }
}

/// How the world task ended, from the joiner's side.
///
/// [`world_fault_detail`] can only speak for a world that actually *joined*.
/// The third ending is the one it cannot see: a world that never resolves at
/// all. That used to be indistinguishable from a healthy slow one because the
/// join was `world_join.await` with no bound — a wedged world hung `mushd`
/// until systemd escalated to `SIGKILL`, which skips the writer drain, the
/// store close and the whole shutdown tail, and leaves an operator an exit with
/// no code at all to explain it.
///
/// The distinction is worth a variant (and a distinct exit code) because the
/// operator responses diverge: a [`Failed`](Self::Failed) world is a bug to
/// page on, a [`Timeout`](Self::Timeout) is usually a box under load, and
/// restarting the second one harder makes it worse.
///
/// WS22-S5 moves this (and [`await_world`]) into `mush_server::shutdown` so
/// both halves share one copy.
#[derive(Debug)]
enum WorldEnd {
    /// The world walked out of its own state machine: `@shutdown`, Ctrl-C,
    /// SIGTERM.
    Clean,
    /// The world stopped without finishing, carrying the detail
    /// [`world_fault_detail`] extracted.
    Failed(String),
    /// The world did not join within its bound. Nothing is known about how it
    /// ended, which is itself the finding.
    Timeout,
}

impl WorldEnd {
    /// Classify a join result that actually arrived.
    fn classify(result: Result<WorldOutcome, tokio::task::JoinError>) -> Self {
        match world_fault_detail(result) {
            None => Self::Clean,
            Some(detail) => Self::Failed(detail),
        }
    }

    /// The fault text, for the ending that has one.
    fn fault_detail(&self) -> Option<&str> {
        match self {
            Self::Failed(detail) => Some(detail),
            Self::Clean | Self::Timeout => None,
        }
    }

    /// Whether the world reached an ordinary ending. Everything else — fault or
    /// silence — refuses the shutdown snapshot.
    const fn is_clean(&self) -> bool {
        matches!(self, Self::Clean)
    }

    /// The shared exit vocabulary's verdict on this ending.
    const fn exit_class(&self) -> ExitClass {
        match self {
            Self::Clean => ExitClass::Ok,
            Self::Failed(_) => ExitClass::Failure,
            Self::Timeout => ExitClass::WorldJoinTimeout,
        }
    }

    /// Spend the ending as this process's exit, once the whole shutdown tail
    /// has run. `None` is a clean exit.
    fn into_failure(self) -> Option<ExitFailure> {
        let class = self.exit_class();
        match self {
            Self::Clean => None,
            Self::Failed(detail) => Some(ExitFailure::new(
                class,
                format!("world task faulted: {detail}"),
            )),
            Self::Timeout => Some(ExitFailure::new(
                class,
                format!(
                    "world task did not join within {}s",
                    world_task::WORLD_JOIN_TIMEOUT.as_secs()
                ),
            )),
        }
    }
}

/// Join the world task, but never for longer than `bound`.
///
/// The bound is [`world_task::WORLD_JOIN_TIMEOUT`] at every call site: strictly
/// longer than the world's own shutdown grace, so the *designed* slow path — a
/// shutdown waiting out a pending exact commit — is never misread as a hung
/// world.
async fn await_world(
    join: &mut tokio::task::JoinHandle<WorldOutcome>,
    bound: Duration,
) -> WorldEnd {
    match tokio::time::timeout(bound, join).await {
        Ok(result) => WorldEnd::classify(result),
        Err(_) => WorldEnd::Timeout,
    }
}

/// This binary's half of the one command line the three binaries share
/// (WS22-S1). `--repl` is `mushd`'s alone; `--ipc` belongs to the server half,
/// so naming it here is an error rather than a token quietly dropped.
const CLI: CliSpec = CliSpec {
    program: "mushd",
    usage: "Usage: mushd [--repl] [--check-config] [--config <path>]",
    ipc: false,
    repl: true,
    check_config: true,
};

fn main() -> ExitCode {
    let cli = match mush_config::cli::parse_env(&CLI) {
        Ok(cli) => cli,
        Err(e) => {
            eprintln!("{}: {e}", CLI.program);
            eprintln!("{}", CLI.usage);
            return ExitClass::Failure.exit_code();
        }
    };
    if cli.help {
        println!("{}", CLI.usage);
        return ExitClass::Ok.exit_code();
    }
    // `--check-config` outranks `--repl` (W1-B8-FIX). This branch used to
    // return before the preflight below, so `mushd --repl --check-config`
    // started the REPL and dropped the check on the floor — precisely the
    // silent-token failure the shared parser exists to abolish, reappearing one
    // level up as a silently-ignored FLAG. The check needs the loaded config,
    // so it is still answered below; all that happens here is that the REPL no
    // longer gets to return first. `--repl` alone is untouched.
    if cli.repl && !cli.check_config {
        repl::run();
        return ExitClass::Ok.exit_code();
    }

    // Config is loaded first because the `[log]` section drives the
    // subscriber. Config::load's own tracing calls (before the subscriber is
    // installed) are therefore uncaptured, but its errors go to stderr, so a
    // bad config still reports cleanly. An explicit `--config` that names a
    // file which is not there stops here instead of booting the defaults.
    let config = match cli.load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}: {e}", CLI.program);
            return ExitClass::Failure.exit_code();
        }
    };

    // `--check-config` (WS04-S3): the refusals named by
    // `mush_server::boot::check_config`, which is the list of record for the
    // all-in-one. `preflight::check_ipc` is deliberately NOT run here — this
    // binary never reads `[ipc]`, so a boundary refusal would name a failure it
    // cannot have (W1-B7-FIX). Nothing on the list opens the store, a socket,
    // or the quarantine spool's exclusive lifetime lock. It sits here — after
    // the config loads, before ANYTHING is opened — precisely so it is safe to
    // run beside a live instance.
    if cli.check_config {
        return mush_config::preflight::verdict(
            CLI.program,
            &cli.config,
            &mush_server::boot::check_config(&config),
        )
        .exit_code();
    }

    // Validate the integrity store before logging is allowed to create or
    // append any startup file. Retain this exact instance (and its exclusive
    // lifetime lock) through recovery, ordinary boot, and the writer.
    let quarantine_spool = match QuarantineSpool::open(config.server.data_dir.join("quarantine")) {
        Ok(spool) => spool,
        Err(error) => {
            eprintln!("mushd: quarantine spool preflight failed: {error}");
            return ExitClass::Failure.exit_code();
        }
    };

    // Structured logging per `[log]` (level/format + optional daily-rotating
    // file output). The guard keeps the non-blocking file appender's worker
    // alive for the whole process; it drops at the end of `main`. The role is
    // the all-in-one's own (`mushd.log`, `half=mushd`) — the split halves pass
    // `gateway`/`server` so they never share one file.
    let _log_guards = mushd::logging::init(
        &config.log,
        &config.server.data_dir,
        mushd::logging::DEFAULT_ROLE,
    );

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
    // The one exit table both halves speak (`mush_config::exit`): 0 clean, 1 a
    // fatal error including a world fault, 3 a world that never joined. An
    // operator's supervisor reads the number; the text explains it.
    match runtime.block_on(serve(config, quarantine_spool)) {
        Ok(()) => ExitClass::Ok.exit_code(),
        Err(failure) => {
            eprintln!("mushd: {failure}");
            failure.class.exit_code()
        }
    }
}

async fn serve(config: Config, mut quarantine_spool: QuarantineSpool) -> Result<(), ExitFailure> {
    let listen = config.ssh_listen().map_err(|e| e.to_string())?;
    let data_dir = config.server.data_dir.clone();
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

    // Everything the deployed all-in-one composes, in one callable root the
    // integration suite boots too (WS04-S0): the quarantine barrier, the store,
    // the three durable key rings, the module roster, the world, and every
    // module's boot pass. WS04-S3 reduced that root to a delegation, so what
    // boots here is `mush_server::boot` itself — the same code the split half
    // runs, rather than a second transcription of it.
    let (
        mush_server::boot::OpenedStore {
            store,
            archive: ledger_archive,
        },
        booted,
    ) = boot::boot_all_in_one(&config, &mut quarantine_spool).await?;

    // The web hello manifest: every module-registered panel feed plus the
    // engine-side "who"/"legal.actions", and one feature id per module (comsys
    // advertises as "channels").
    let (features, panels) = mush_server::boot::build_hello_manifest(&booted.engine);

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
        booted.engine,
        Arc::clone(&sessions),
        ServerDeps {
            store: Arc::clone(&store),
            guard: Arc::clone(&guard),
            staff: booted.staff,
            llm: Some(Arc::clone(&booted.llm)),
            metrics: Arc::clone(&metrics),
            // E0 quarantine spool: a separate durable failure domain under the
            // data dir for confirmed-poison ledger seals.
            quarantine_spool,
            // Programme D5: the on-call Discord DM adapter, when the bot is
            // configured (a REST-only client of its own, beside the gateway's).
            outbox_adapters: mushd::discord_dm::adapters_from_config(&config),
        },
        booted.decision_workers,
        Arc::clone(&controls),
    )?;
    let persistence_integrity = handles.persistence_integrity.clone();
    // Set by the world loop's own last statement; false after a panic, a
    // channel-closed exit, or a shutdown that ran out its bounded grace.
    let world_exit = handles.world_exit.clone();
    let world = handles.handle;
    let mut world_join = handles.world_join;
    let writer_join = handles.writer_join;
    let outbox_join = handles.outbox_join;
    // 16b cut T.2 (T1): the arena tasks, stopped and joined in the tail below.
    let arenas = handles.arenas;

    // J5: the ledger archive pass, once a UTC day inside its window, beside
    // the store writer and never on the world task. WS04-S1: assembled once, in
    // `mush_server::boot`, so the split half schedules the same pass off the
    // same `[store.archive]` reading.
    // The handle is BOUND, not dropped: a detached pass is a task this function
    // owns and cannot stop, still copying and purging inside a runtime the
    // shutdown tail is about to tear down (mushd-5). `stop_ledger_archive` in
    // that tail is the other end of it.
    let archive_job = mush_server::boot::spawn_ledger_archive(
        &config,
        &store,
        ledger_archive,
        booted.archive_namespaces,
        &metrics,
    )?;

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
    // How the world task ended. Both arms below bind the join result rather than
    // dropping it, because either can carry a fault: a world can panic while
    // the operator's Ctrl-C shutdown is already in flight just as easily as it
    // can die on its own. The signal arm's join is BOUNDED, so a world that
    // never ends at all is a third ending with a code of its own rather than a
    // process that hangs until systemd kills it.
    let world_end: WorldEnd = tokio::select! {
        // Ctrl-C at the console, or the systemd stop / `kill <pid>` SIGTERM of
        // the production box (never on Windows). One arm, because the two ran
        // the identical sequence and a copy that drifts is a shutdown that
        // half-works; `mush_config::signals` is the same future the forked
        // halves wait on (WS22-S2).
        signal = mush_config::signals::shutdown_signal() => {
            tracing::info!(%signal, "shutting down on signal");
            // Stop accepting first (PLAN shutdown order), then announce +
            // stop the loop; the world task delivers the shutdown notice
            // to every connection before exiting.
            listener.stop_accepting();
            world.inbound(Inbound::Control(ControlMsg::Shutdown)).await;
            // BOUNDED (WS22-S3). An unbounded join here is how a wedged world
            // turned `systemctl stop` into systemd's own SIGKILL — which skips
            // every step of the tail below.
            await_world(&mut world_join, world_task::WORLD_JOIN_TIMEOUT).await
        }
        // The world task can also end itself — an in-game `@shutdown` (it has
        // already stepped ControlMsg::Shutdown and announced), or a fault that
        // stopped the loop mid-command. The two are not the same event and no
        // longer log the same way.
        joined = &mut world_join => {
            let end = WorldEnd::classify(joined);
            if end.is_clean() {
                tracing::info!("world task ended on its own");
            }
            listener.stop_accepting();
            end
        }
    };
    // Said once, before the shutdown tail, so the ending is on the record ahead
    // of every decision the tail makes about it. The timeout gets its own fixed
    // marker: it is the ending with no detail of its own, so this line IS the
    // whole diagnosis.
    match &world_end {
        WorldEnd::Clean => {}
        WorldEnd::Failed(detail) => tracing::error!(
            %detail,
            "world task faulted; shutting down and exiting on failure"
        ),
        WorldEnd::Timeout => tracing::error!(
            bound_s = world_task::WORLD_JOIN_TIMEOUT.as_secs(),
            "world task did not join within its bound; shutting down without it"
        ),
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
    // 16b cut T.2 (T1): the arena tasks, after the world has stopped (so nothing
    // is starting another match) and before the store writer's drain (an arena
    // task holds no store handle, so it is not something the writer waits for).
    // Each join is individually bounded, like every other step of this tail.
    mush_server::world_task::arena_shutdown(arenas).await;
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
    // The J5 pass is the fourth task this function owns, and it is joined like
    // the other three — before the store drain, because a live pass is still
    // writing to the store this is about to drain and close. It has no shutdown
    // seam of its own (it sleeps in a loop for the life of the process), so the
    // join is a cancel-then-await; both halves of a pass are idempotent, so a
    // cancelled one costs the next night's pass a re-copy and nothing else.
    mush_server::boot::stop_ledger_archive(archive_job).await;
    // Each tail step is individually bounded so a dead store can never
    // wedge shutdown (systemd would otherwise escalate to SIGKILL).
    match tokio::time::timeout(SHUTDOWN_STORE_TIMEOUT, store.drain()).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::error!(error = %e, "store drain failed"),
        Err(_) => tracing::error!("store drain timed out (store down?)"),
    }
    let integrity_latched = persistence_integrity.is_latched();
    let world_exited_cleanly = world_exit.is_clean();
    if shutdown_bgsave_allowed(
        world_exited_cleanly,
        writer_drained,
        integrity_latched,
        !world_end.is_clean(),
    ) {
        match tokio::time::timeout(SHUTDOWN_STORE_TIMEOUT, store.bgsave()).await {
            Ok(Ok(())) => tracing::info!("shutdown BGSAVE triggered"),
            Ok(Err(e)) => tracing::error!(error = %e, "shutdown BGSAVE failed"),
            Err(_) => tracing::error!("shutdown BGSAVE timed out (store down?)"),
        }
    } else if let Some(detail) = world_end.fault_detail() {
        // First, because it is the most specific: it names WHY the world did
        // not finish. The clean-exit branch below would otherwise swallow it.
        tracing::error!(
            %detail,
            "shutdown BGSAVE skipped: the world task faulted, so its RAM is a torn \
             mid-command state and a snapshot would canonize it"
        );
    } else if matches!(world_end, WorldEnd::Timeout) {
        // A world that never joined may still be mutating RAM the store has not
        // seen. Snapshotting under a live writer is the same torn-state hazard
        // as the fault above, minus the explanation.
        tracing::error!(
            "shutdown BGSAVE skipped: the world task never joined, so nothing here knows \
             whether its RAM finished reaching the store"
        );
    } else if !world_exited_cleanly {
        tracing::error!(
            "shutdown BGSAVE skipped: the world task did not reach a clean exit; its RAM never \
             finished reaching the store and a snapshot would canonize a partial world"
        );
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
    // Only NOW, with the writer drained, the store drained and closed and the
    // transports flushed, is the failure raised. An exit code is never worth
    // bypassing a drain, and it is never raised by `process::exit`/`abort` from
    // inside a catch handler. `main` spends the class on this process's exit
    // code and prints the detail to stderr.
    if let Some(failure) = world_end.into_failure() {
        return Err(failure);
    }
    tracing::info!("mushd exiting at {}", unix_now_s());
    Ok(())
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
    use super::{
        ExitClass, WorldEnd, WorldOutcome, await_world, shutdown_bgsave_allowed,
        world_fault_detail, world_task,
    };

    /// The full truth table of the four-term gate. Exactly one row may
    /// snapshot; every row with an unclean world exit must refuse, including
    /// the one that otherwise looks perfect (drained writer, clear latch) —
    /// that row is the panicked world task, and it is the whole point of the
    /// world-exit term.
    #[test]
    fn shutdown_bgsave_requires_clean_drain_and_clear_integrity() {
        assert!(shutdown_bgsave_allowed(true, true, false, false));
        assert!(!shutdown_bgsave_allowed(true, true, true, false));
        assert!(!shutdown_bgsave_allowed(true, false, false, false));
        assert!(!shutdown_bgsave_allowed(true, false, true, false));
        // Every unclean-world-exit row refuses, whatever the others say.
        assert!(!shutdown_bgsave_allowed(false, true, false, false));
        assert!(!shutdown_bgsave_allowed(false, true, true, false));
        assert!(!shutdown_bgsave_allowed(false, false, false, false));
        assert!(!shutdown_bgsave_allowed(false, false, true, false));
    }

    /// The two rows this step exists for, kept apart from the table above so
    /// neither can be quietly folded into the other later.
    ///
    /// - The UNCAUGHT class: a panic across an `.await`, or one outside the
    ///   world loop's guarded sites. The join handle carries the fault and
    ///   nothing else does — the writer drained perfectly, no latch was set —
    ///   and this is exactly how a torn mid-command world used to get
    ///   snapshotted as the on-disk truth.
    /// - The CAUGHT class: the world loop's own dispatch boundary caught the
    ///   panic, froze persistence and stopped, so the latch is already set. The
    ///   row is here to pin the interaction: both gates must refuse
    ///   independently, so neither is later removed as redundant.
    #[test]
    fn a_faulted_world_refuses_the_snapshot_latch_or_no_latch() {
        // Uncaught: drained, unlatched — and still refused.
        assert!(!shutdown_bgsave_allowed(true, true, false, true));
        assert!(!shutdown_bgsave_allowed(false, true, false, true));
        // Caught: the integrity freeze already latched.
        assert!(!shutdown_bgsave_allowed(true, true, true, true));
        assert!(!shutdown_bgsave_allowed(false, true, true, true));
    }

    /// An ordinary ending is not a fault. This is the row that keeps a false
    /// positive from restart-looping a live box: only a `Faulted` outcome or a
    /// real `JoinError` sets the flag.
    #[test]
    fn a_clean_world_outcome_is_never_a_fault() {
        assert_eq!(world_fault_detail(Ok(WorldOutcome::Clean)), None);
    }

    /// The caught class: the world loop's own boundary reports where and what.
    #[test]
    fn a_caught_fault_carries_its_site_and_payload() {
        let detail = world_fault_detail(Ok(WorldOutcome::Faulted(mushd::world_task::WorldFault {
            site: "engine.step",
            detail: "the sky fell".to_owned(),
        })))
        .expect("a caught fault is a fault");
        assert!(detail.contains("engine.step"), "{detail}");
        assert!(detail.contains("the sky fell"), "{detail}");
    }

    /// The uncaught class, against a REAL `JoinError` rather than a stand-in:
    /// a task that panics across the boundary the guarded sites cannot cover,
    /// and a task cancelled out from under the join.
    #[tokio::test]
    async fn a_real_join_error_is_a_fault_panicked_or_cancelled() {
        let panicked = tokio::spawn(async { panic!("across an await") })
            .await
            .expect_err("the task panicked");
        let detail = world_fault_detail(Err(panicked)).expect("a panic is a fault");
        assert!(detail.contains("outside its dispatch boundary"), "{detail}");

        let handle = tokio::spawn(std::future::pending::<()>());
        handle.abort();
        let cancelled = handle.await.expect_err("the task was cancelled");
        let detail = world_fault_detail(Err(cancelled)).expect("a cancellation is a fault");
        assert!(detail.contains("without a value"), "{detail}");
    }

    // -----------------------------------------------------------------------
    // WS22-S3: the bounded join, and the three endings it can report
    // -----------------------------------------------------------------------

    /// A real `JoinError` from a real panic, through the real bound. The panic
    /// is the ending the join handle is the ONLY witness to (the world loop's
    /// own dispatch boundary cannot catch one raised across an `.await`), so a
    /// classifier that lost it would hand `main` a clean exit for a dead world.
    #[tokio::test]
    async fn a_panicked_world_task_is_a_failure() {
        let mut join = tokio::spawn(async { panic!("the world fell over") });
        let end = await_world(&mut join, world_task::WORLD_JOIN_TIMEOUT).await;
        let detail = end.fault_detail().expect("a panic is a fault").to_owned();
        assert!(detail.contains("outside its dispatch boundary"), "{detail}");
        assert!(!end.is_clean());
        assert_eq!(end.exit_class(), ExitClass::Failure);
    }

    /// The other `JoinError`: a task cancelled out from under the joiner. It
    /// carries no payload at all, which is exactly why it must not be mistaken
    /// for an ordinary ending.
    #[tokio::test]
    async fn a_cancelled_world_task_is_a_failure() {
        let mut join = tokio::spawn(std::future::pending::<WorldOutcome>());
        join.abort();
        let end = await_world(&mut join, world_task::WORLD_JOIN_TIMEOUT).await;
        assert!(!end.is_clean());
        assert_eq!(end.exit_class(), ExitClass::Failure);
    }

    /// The contrast: a world that returned `Clean` inside the bound is an
    /// ordinary exit, worth zero. A false positive here restart-loops a live
    /// box.
    #[tokio::test]
    async fn a_world_that_finished_cleanly_is_clean() {
        let mut join = tokio::spawn(async { WorldOutcome::Clean });
        let end = await_world(&mut join, world_task::WORLD_JOIN_TIMEOUT).await;
        assert!(end.is_clean());
        assert_eq!(end.fault_detail(), None);
        assert_eq!(end.exit_class(), ExitClass::Ok);
    }

    /// The failure this step closes: a world task that never resolves.
    ///
    /// Before the bound this was `world_join.await`, so this test could not be
    /// written — it would hang forever, which is precisely what `mushd` did on
    /// a wedged world until systemd escalated to `SIGKILL` and skipped the
    /// whole shutdown tail. The clock is paused, so the assertion on the
    /// elapsed time is exact rather than a range: the join waits its bound and
    /// not a second more or less.
    #[tokio::test(start_paused = true)]
    async fn a_world_that_never_joins_times_out_at_the_bound() {
        let mut join = tokio::spawn(std::future::pending::<WorldOutcome>());
        let start = tokio::time::Instant::now();
        let end = await_world(&mut join, world_task::WORLD_JOIN_TIMEOUT).await;
        assert_eq!(start.elapsed(), world_task::WORLD_JOIN_TIMEOUT);
        assert!(matches!(end, WorldEnd::Timeout), "{end:?}");
        // Not a fault: there is no fault detail to report, and the exit code is
        // its own, so a supervisor can tell a slow stop from a dead world.
        assert_eq!(end.fault_detail(), None);
        assert!(!end.is_clean());
        assert_eq!(end.exit_class(), ExitClass::WorldJoinTimeout);
        join.abort();
    }

    /// The joiner's bound must outlast the world's own shutdown grace,
    /// otherwise the designed slow path — a shutdown that waits out a pending
    /// exact commit — reports as a hung world every single time.
    #[test]
    fn the_join_bound_outlasts_the_worlds_own_grace() {
        assert!(world_task::WORLD_JOIN_TIMEOUT > world_task::WORLD_SHUTDOWN_GRACE);
        assert_eq!(
            world_task::WORLD_JOIN_TIMEOUT,
            world_task::WORLD_SHUTDOWN_GRACE + world_task::WORLD_JOIN_SLACK
        );
    }

    /// This binary's half of the one exit table (`mush_config::exit`). The
    /// numbers are what a supervisor and a unit file encode out of tree, so
    /// they are asserted here as literals, not derived.
    #[test]
    fn the_exit_codes_are_the_shared_vocabulary() {
        assert_eq!(WorldEnd::Clean.exit_class().code(), 0);
        assert_eq!(WorldEnd::Failed("boom".to_owned()).exit_class().code(), 1);
        assert_eq!(WorldEnd::Timeout.exit_class().code(), 3);
        // ...and the reserved one is still reserved (WS22-S18 produces it).
        assert_eq!(ExitClass::QuarantineLockHeld.code(), 2);
    }

    /// The text `main` prints to stderr, for each ending that has any. The
    /// fault wording is asserted end to end by `tests/world_fault.rs`.
    #[test]
    fn each_failing_ending_explains_itself() {
        assert!(WorldEnd::Clean.into_failure().is_none());
        let faulted = WorldEnd::Failed("engine.step: the sky fell".to_owned())
            .into_failure()
            .expect("a fault is a failure");
        assert_eq!(faulted.class, ExitClass::Failure);
        assert!(faulted.detail.contains("world task faulted"), "{faulted}");
        let slow = WorldEnd::Timeout
            .into_failure()
            .expect("a timeout is a failure");
        assert_eq!(slow.class, ExitClass::WorldJoinTimeout);
        assert!(slow.detail.contains("did not join"), "{slow}");
    }
}

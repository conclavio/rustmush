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

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(clippy::unwrap_used, clippy::panic_in_result_fn)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use std::time::Duration;

use mush_config::config::{Config, StoreBackend};
use mush_config::exit::{ExitClass, ExitFailure};
use mush_config::ipguard::IpGuard;
use mush_config::metrics::Metrics;
use mush_core::{ControlMsg, Inbound, OfferRouteTable, OfferSubmission};
use mush_ipc::{
    BoxWrite, EmissionGate, FrameReader, GatewayInput, GatewayRequest, GatewayRequestDisposition,
    IPC_MIN_PROTO, IPC_PRE_HELLO_VERSION, IPC_VERSION, IPC_VERSION_SKEW_MARKER, IpcEndpoint,
    LegalActionsError, LinkVersion, MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES,
    MAX_IN_FLIGHT_GATEWAY_INPUT_BYTES_PER_CONNECTION, MAX_IN_FLIGHT_GATEWAY_INPUTS,
    MAX_IN_FLIGHT_GATEWAY_INPUTS_PER_CONNECTION, MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS,
    MAX_IN_FLIGHT_SERVICE_REQUESTS, ProtoWindow, RequestUid, ServiceReq, ServiceResp, ToGateway,
    ToServer,
};
use mush_store::{BundleStatus, InMemoryStore, ObjectStore, RedisStore};
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};

// The deny above has no test exemption, so a test or fixture helper that
// returns `Result` checks with `check!`/`check_eq!`/`check_ne!` — its asserts
// as early `Err` returns. Declared before every other module because
// `macro_rules!` is textually scoped. The module file gates itself to test
// builds; this file names no test cfg this early, because its source
// censuses read the production half as the text before the first one.
#[cfg_attr(test, macro_use)]
mod test_checks;

pub mod accounts_identity;
pub mod action_coordinator;
pub mod archive;
pub mod boot;
pub mod bootstrap;
pub mod config_ext;
/// E7c10 B3: the server-side, audit-only seat attribution log.
// E4c: the economy -> canon evidence bridge (08c §6.1 item 3).
pub mod decision_dispatch;
pub mod decision_llm;
pub mod discord_dm;
pub mod durable_outbox;
pub mod gateway_link;
pub mod gateway_sink;
pub mod help;
pub mod modules;
// The composition adapters live in `mush-composition` and `tacsim::composition`;
// the paths the integration tests use keep resolving.
pub use mush_composition::{
    campaign_formations, econ_alerts, econ_canon, econ_inbox, industry_recipes, salvage_source,
    seat_audit, sim_routes,
};
pub mod persist;
mod process_fault;
pub mod quarantine;
pub mod retained;
pub mod staff;
pub mod store_read;
pub mod timeline;
pub mod world_task;

use crate::action_coordinator::{
    ActionCoordinator, ActionPreflight, MAX_PREFLIGHT_ATTEMPTS, PREFLIGHT_RETRY_WINDOW,
};
use crate::gateway_sink::SocketSink;
use crate::persist::AuthoritativeCommitFailure;
use crate::persist::WorldExitStatus;
use crate::quarantine::QuarantineSpool;
use crate::world_task::{
    CreatePlayerErr, LegalActionReadError, ServerDeps, WorldHandle, WorldOutcome, WorldReadError,
};

pub use gateway_sink::{GatewaySink, SocketSink as ServerSocketSink};

/// WS01-S8's world-turn fault arming, for a supervised proof whose server
/// halves are threads in the test process. Feature-gated; no shipped build
/// carries it.
#[cfg(feature = "fault-injection")]
pub use process_fault::arm_world_turn_panic;

/// A human-readable server identity string, sent in [`ToGateway::Hello`].
fn server_id() -> String {
    format!("mush-server {}", env!("CARGO_PKG_VERSION"))
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
    quarantine_spool: QuarantineSpool,
) -> Result<(), String> {
    serve_classified(config, bound, store, quarantine_spool)
        .await
        .map_err(|failure| failure.detail)
}

/// [`serve_with_quarantine_spool`], keeping the exit CLASS of the failure.
///
/// The binary's `main` uses this one: a `String` says the process failed and
/// nothing else, and the two endings an operator most needs to tell apart — a
/// world that died and a world that would not stop — both used to arrive as
/// [`ExitCode::FAILURE`](std::process::ExitCode). The class travels with the
/// text so the site that KNOWS which one it was is the site that says so. Every
/// library caller keeps the `String` form above.
///
/// # Errors
///
/// The same failures as [`serve_with_quarantine_spool`], each carrying the
/// [`ExitClass`] it should exit with.
pub async fn serve_classified(
    config: Config,
    bound: Option<oneshot::Sender<IpcEndpoint>>,
    store: Option<Arc<dyn ObjectStore>>,
    mut quarantine_spool: QuarantineSpool,
) -> Result<(), ExitFailure> {
    let endpoint = config.ipc_endpoint().map_err(|e| e.to_string())?;
    prepare_quarantine_for_writable_boot(&config, &mut quarantine_spool, store.as_deref()).await?;
    let (store, ledger_archive) = match store {
        // A caller who brought their own store owns its tiering: this branch
        // never reaches `open_store`, so `[store.archive]` composes nothing and
        // schedules nothing. That is defensible, but it is also the exact "knob
        // wired into one path only" shape WS04 exists to abolish, so it is said
        // out loud rather than discovered by an operator whose ledger never
        // stopped growing. (See this module's own doc comment.)
        Some(s) => {
            if config.store.archive.enabled {
                tracing::warn!(
                    "[store.archive].enabled is ignored on this boot: the caller supplied \
                     the object store, so no archive tier is attached and no nightly pass \
                     is scheduled"
                );
            }
            (s, None)
        }
        // WS04-S1: the archive tier is attached to the store here, so the READ
        // fall-through is live on this half. WS04-S2: the handle rides along to
        // the nightly WRITE pass below, which needs the booted engine's
        // namespace list and so cannot be scheduled until the engine exists.
        None => {
            let opened = boot::open_store(&config).await?;
            (opened.store, opened.archive)
        }
    };
    let booted = boot::build_engine(&config, &store, &quarantine_spool).await?;
    let offer_routes = Arc::new(
        booted
            .engine
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
    let (features, panels) = boot::build_hello_manifest(&booted.engine);

    // The server -> gateway boundary: every `ToGateway` the world loop (and the
    // request handlers) produce is queued here and drained to the socket by the
    // writer task. BOUNDED (M18, `[ipc].to_gw_cap`): every `to_gw` frame is
    // must-deliver, so it is NEVER per-message dropped — on overflow the whole
    // boundary is torn down (the gateway's re-attach re-renders every
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
    // The one metrics registry of this process (PLAN "Observability"): the
    // `SocketSink`, the writer and reader loops, the archive pass and the world
    // loop all stamp it, and `@stats` reads it back. It used to be the throwaway
    // registry the in-memory deps made, shared by accident; now it is made here
    // and shared on purpose (a parity test counts this line: one registry).
    let metrics = Arc::new(Metrics::new());
    // 16b cut AC.1 (16c B-82): the world task's dependencies through the ONE
    // production constructor `mushd::main` calls too — the booted staff queue
    // (matching the registry handlers), the booted LLM service (what the effect
    // dispatcher runs `Effect::ModuleAsync` against), the unified quarantine spool
    // (a separate failure domain from the store, under the configured data dir:
    // confirmed poison seals and complete K3 bundles are fsynced here before the
    // writer enters its integrity halt), and programme D5's on-call Discord DM
    // adapter when the bot is configured.
    let deps = ServerDeps::production(
        &booted,
        Arc::clone(&store),
        Arc::clone(&guard),
        Arc::clone(&metrics),
        quarantine_spool,
        crate::discord_dm::adapters_from_config(&config),
    );
    let boot::BootedEngine {
        engine,
        decision_workers,
        archive_namespaces,
        ..
    } = booted;

    // `teardown` is the one-shot the [`SocketSink`] notifies on overflow and the
    // writer waits on.
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
    // J5: the nightly ledger archive pass, once a UTC day inside its window,
    // beside the store writer and never on the world task — the WRITE/purge half
    // of the tier `open_store` attached above. Its handle is HELD, and joined in
    // the shutdown tail below: a detached pass is a task this function returns
    // out from under, mid-copy, with nobody waiting.
    //
    // Scheduled BEFORE the world task spawns, because a malformed
    // `[store.archive].window` refuses the boot: raising that after the world
    // was serving would abandon a live world task to the runtime drop.
    let archive_job = boot::spawn_ledger_archive(
        &config,
        &store,
        ledger_archive,
        archive_namespaces,
        &metrics,
    )?;
    // H2a: the production decision composition. The provider registry is built
    // over the same LLM service the modules were composed with, and the worker
    // limit (`[llm].decision_workers`) comes from the same reading of `[llm]`
    // that built it — this function no longer re-parses the stanza to find it.
    // The provider registry and the effect dispatcher are fed from ONE field on
    // purpose (16c B-82 was these two fed from two places): the same read
    // `mushd::world_task::spawn_persist_with_decision_workers` makes.
    let deps_llm = deps.llm.clone();
    let handles = world_task::spawn_persist_with_gateway_production(
        engine,
        deps,
        deps_llm,
        decision_workers,
        move |_weak_inbound| {
            // The forked server never polices (it owns no connection buffers), so it
            // ignores the loop's inbound weak sender and just serializes to the boundary.
            Arc::new(sink)
        },
    )?;
    let world = handles.handle;
    let world_join = handles.world_join;
    let store_writer_join = handles.writer_join;
    let outbox_join = handles.outbox_join;
    // 16b cut T.2 (T1): the arena tasks, stopped and joined in the tail below.
    let arenas = handles.arenas;
    // The reader's shutdown-provenance source. The world loop sets this as the
    // LAST statement of its own function — before its receiver drops — so a
    // reader that finds the channel closed can already tell an ordinary exit
    // from a fault without waiting on the join handle it does not own.
    let world_exit = handles.world_exit.clone();

    let listener = mush_ipc::listen(&endpoint)
        .await
        .map_err(|e| format!("ipc listen on {}: {e}", endpoint.describe()))?;
    let actual = listener
        .tcp_addr()
        .map_or_else(|| endpoint.clone(), IpcEndpoint::Tcp);
    tracing::info!(ipc = %listener.describe(), "mush-server up; awaiting gateway");
    // Doc 33 GG.1: the server is its own `Type=notify` unit. READY=1 goes out
    // the moment the socket is bound — the gateway can dial from here on —
    // and never before it (29a §7 H2/H3: a boot that dies first must be a
    // failed start, not a started unit). A no-op outside systemd.
    mush_drain::notify::ready();
    if let Some(tx) = bound {
        let _ = tx.send(actual);
    }

    // One gateway connection per boundary (M16b; M16c adds restart/re-attach).
    let (reader_half, writer_half) = listener
        .accept()
        .await
        .map_err(|e| format!("ipc accept: {e}"))?;
    tracing::info!("gateway connected");

    // The protocol version this link's gateway announces (doc 33 §4.1): the
    // reader records it from the first frame, the writer's emission gate reads
    // it on every frame, so a frame the gateway cannot decode is never sent.
    let link_version = LinkVersion::unknown();

    // Writer: greet (with the panel manifest), then drain the world's output to
    // the socket. Exits when every `to_gw_tx` (held by the world loop + request
    // handlers) is dropped, OR early on a per-frame write timeout / a `teardown`
    // notification (a `to_gw` overflow) — either drops the writer's socket half,
    // EOFing the boundary: this process exits, its unit restarts it, and the
    // gateway re-attaches every held session (M18; doc 33 GG.1).
    let ipc_writer_join = tokio::spawn(writer_loop(
        writer_half,
        to_gw_rx,
        EmissionGate::new(link_version.clone()),
        features,
        panels,
        Arc::clone(&metrics),
        Arc::clone(&teardown),
        write_timeout,
    ));

    // Reader: pump the gateway's `ToServer` traffic into the world. Runs until
    // the gateway closes the boundary (clean EOF), a protocol fault, or the
    // world stops accepting — a closed world channel is permanent, so the
    // reader refuses to keep answering frames it can never deliver.
    //
    // BOXED, therefore pinned: the world's own completion is raced against it
    // below, and a `select!` that moved the reader between polls could drop a
    // half-read frame mid-await. Owning the box also lets this function DROP
    // the reader — and with it the `WorldHandle` clone and the `to_gw` sender
    // inside `reply_sink` — before the joins below wait for those very channels
    // to close.
    let mut reader = Box::pin(reader_loop(
        reader_half,
        world.clone(),
        ingress_store,
        offer_routes,
        reply_sink,
        &metrics,
        world_exit,
        link_version,
    ));
    let mut world_join = world_join;

    // The world half of the fork is only useful while the world task is alive.
    // Before this race the reader was awaited to completion first, so a world
    // that died under a still-connected gateway left this process up with its
    // IPC listener bound, dropping every frame forever and giving systemd
    // nothing to restart (D2). Whichever side ends first ends service.
    //
    // The third arm is the unit's own stop (doc 33 GG.1): `systemctl stop` /
    // `restart` is a SIGTERM to THIS process now, where it used to be an EOF
    // from a gateway closing the boundary. Without the arm the signal's default
    // action killed the server mid-write; with it a stop is the same orderly
    // drain the EOF path runs. `mushd` spends the same future the same way.
    let end = tokio::select! {
        reader_result = &mut reader => BoundaryEnd::Reader(reader_result),
        joined = &mut world_join => BoundaryEnd::World(joined),
        signal = mush_config::signals::shutdown_signal() => BoundaryEnd::Signal(signal),
    };
    let world_result = match end {
        BoundaryEnd::Reader(reader_result) => {
            match reader_result {
                Ok(()) => tracing::info!("gateway disconnected (clean EOF)"),
                // A reader that ended because the world is gone already logged
                // that at its own provenance severity; repeating it as a warn
                // would make every ordinary `@shutdown` look like a fault.
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                    tracing::info!(error = %e, "ipc reader ended with the world already gone");
                }
                Err(e) => tracing::warn!(error = %e, "ipc reader ended on error"),
            }
            // Boundary gone: shut the world down cleanly, then let the writer
            // flush its Bye and exit (it stops when the world drops its
            // `to_gw_tx` clones).
            stop_world(&world, &mut world_join).await
        }
        BoundaryEnd::Signal(signal) => {
            tracing::info!(%signal, "shutting down on signal");
            // The same drain as a closed boundary. The reader is dropped
            // below, which EOFs the gateway into its gap; the gateway holds
            // every connection and re-attaches when the unit is back.
            stop_world(&world, &mut world_join).await
        }
        BoundaryEnd::World(joined) => {
            // The world stopped under a live gateway. It is already gone, so
            // this path sends NO second `ControlMsg::Shutdown`: the ordinary
            // sequence above is the only one that ever issues it, and a second
            // teardown here would race it.
            tracing::info!("world task ended while the gateway was still connected");
            Some(joined)
        }
    };
    // Release the reader's captures before waiting on the channels they hold.
    drop(reader);
    drop(to_gw_tx);
    drop(world);
    // 16b cut T.2 (T1): the arena tasks, after the world has stopped (so nothing
    // is starting another match) and before the store writer's drain (an arena
    // task holds no store handle, so it is not something the writer waits for).
    // Each join is individually bounded, like every other step of this tail.
    world_task::arena_shutdown(arenas).await;
    let _ = outbox_join.await;
    // Keep custody of the actual persistence writer (distinct from the IPC
    // writer above) and wait for it to drain before this runtime can return.
    // Detaching it here would let process teardown discard a batch after the
    // world believed shutdown sequencing had transferred custody.
    let _ = store_writer_join.await;
    let _ = ipc_writer_join.await;
    // The J5 pass is the fourth task this function owns, and it is joined like
    // the other three. It has no shutdown seam of its own (it sleeps in a loop
    // for the life of the process), so the join is a cancel-then-await; both
    // halves of a pass are idempotent, so a cancelled one costs the next night's
    // pass a re-copy and nothing else.
    boot::stop_ledger_archive(archive_job).await;
    // Only now, with the outbox, the store writer, the IPC writer and the
    // archive pass all joined, is the failure raised: an exit code is never
    // worth bypassing a
    // drain, and it is never raised by `process::exit`/`abort` from inside a
    // catch handler or a driver.
    let Some(world_result) = world_result else {
        // The tail above still ran, and each of its joins is the unit's stop
        // timeout's problem to bound (WS22-S6); what this process owes an
        // operator is the distinction, and it is a code of its own so an
        // operator can tell a slow stop from a dead world without parsing
        // logs.
        return Err(ExitFailure::new(
            ExitClass::WorldJoinTimeout,
            format!(
                "world task did not join within {}s",
                world_task::WORLD_JOIN_TIMEOUT.as_secs()
            ),
        ));
    };
    world_exit_result(world_result).inspect_err(|detail| {
        tracing::error!(%detail, "mush-server exiting on a world fault");
    })?;
    tracing::info!("mush-server exiting");
    Ok(())
}

/// Which half of the boundary ended first.
enum BoundaryEnd {
    /// The gateway-facing reader returned (clean EOF, protocol fault, or a
    /// world it can no longer reach).
    Reader(std::io::Result<()>),
    /// The world task's join handle resolved while the reader was still live.
    World(Result<WorldOutcome, tokio::task::JoinError>),
    /// The operator (or the unit) asked this process to stop (doc 33 GG.1).
    Signal(mush_config::signals::ShutdownSignal),
}

/// Stop the world and wait for it, bounded.
///
/// BOUNDED (WS22-S3): a world that will not stop must not hold this process
/// open in silence. The bound is strictly longer than the world's own
/// shutdown grace, so the designed slow path — waiting out a pending exact
/// commit — is never read as a hung world. `None` is the timeout, which the
/// caller turns into [`ExitClass::WorldJoinTimeout`].
async fn stop_world(
    world: &WorldHandle,
    world_join: &mut tokio::task::JoinHandle<WorldOutcome>,
) -> Option<Result<WorldOutcome, tokio::task::JoinError>> {
    world.inbound(Inbound::Control(ControlMsg::Shutdown)).await;
    match tokio::time::timeout(world_task::WORLD_JOIN_TIMEOUT, world_join).await {
        Ok(joined) => Some(joined),
        Err(_) => {
            tracing::error!(
                bound_s = world_task::WORLD_JOIN_TIMEOUT.as_secs(),
                "world task did not join within its bound; shutting down without it"
            );
            None
        }
    }
}

/// Turn the world task's ending into this process's exit.
///
/// A world that walked out of its own state machine is an ordinary exit —
/// `@shutdown` and the boundary's own `ControlMsg::Shutdown` both land here.
/// Anything else is a fault the process must NOT survive quietly: systemd
/// heals the forked half by restarting the unit (D2; `Restart=on-failure`),
/// and it only knows to do that if this process stops and says so. `main` maps the `Err` to
/// [`ExitCode::FAILURE`](std::process::ExitCode).
fn world_exit_result(result: Result<WorldOutcome, tokio::task::JoinError>) -> Result<(), String> {
    match result {
        Ok(WorldOutcome::Clean) => Ok(()),
        Ok(WorldOutcome::Faulted(fault)) => Err(format!(
            "world task faulted at {}: {}",
            fault.site, fault.detail
        )),
        // The dispatch boundary cannot cover a panic across an `.await`, so the
        // task's own `JoinError` is the net for those — and for a world task
        // that was cancelled out from under this function.
        Err(e) if e.is_panic() => Err(format!(
            "world task panicked outside its dispatch boundary: {e}"
        )),
        Err(e) => Err(format!("world task ended without a value: {e}")),
    }
}

/// The socket writer: send [`ToGateway::Hello`] (carrying the panel manifest
/// and this build's protocol window), then forward each queued message through
/// the link's [`EmissionGate`] (doc 33 §4.1: a frame the gateway's announced
/// version cannot decode is withheld or downgraded, never sent). On the channel
/// closing (world gone) it sends a final [`ToGateway::Bye`] and returns.
///
/// The greeting and the health tick bypass the gate: both predate the window,
/// and the greeting goes out before the gateway has announced anything.
///
/// Two M18 teardown paths return early, each dropping `w` (the boundary EOFs;
/// this process exits, its unit restarts it, and the gateway heals the held
/// sessions via ReAnnounce → re-attach):
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
// GG.0 added the emission gate as the loop's eighth argument; the loop is the
// one place these all meet, so the count is the shape, not a smell.
#[allow(clippy::too_many_arguments)]
async fn writer_loop(
    mut w: BoxWrite,
    mut rx: mpsc::Receiver<ToGateway>,
    gate: EmissionGate,
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
            min_proto: IPC_MIN_PROTO,
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
    let mut health_tick = tokio::time::interval(Duration::from_secs(1));
    health_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = health_tick.tick() => {
                let sample = ToGateway::WorldHealth {
                    world_loop_unixtime: metrics.world_loop_last_iter_unix(),
                    loop_lag_ms: metrics.world_loop_lag_ms(),
                    store_ok: metrics.store_ok(),
                    integrity_ok: metrics.integrity_ok(),
                };
                if !write_or_teardown(&mut w, &sample, &metrics, write_timeout,
                    "health write failed; boundary down").await {
                    return;
                }
            }
            msg = rx.recv() => match msg {
                Some(msg) => {
                    let introduced_at = msg.introduced_at();
                    let Some(msg) = gate.admit(msg) else {
                        tracing::debug!(
                            introduced_at,
                            link_version = gate.link().effective(),
                            "frame withheld by the emission gate: the gateway on this link \
                             predates it"
                        );
                        continue;
                    };
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
            // the boundary — the gateway's re-attach re-renders every session.
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

/// How many times an exact gateway replay may re-arm one retained claim
/// before the server gives up on it.
///
/// A retained claim is the honest answer to a store or world state that could
/// neither prove nor deny the request's durable fate; the gateway's own replay
/// (its custody copy survives, because retention publishes no disposition) is
/// what re-runs the read-only preflight. This bound is what keeps that from
/// being an unbounded loop: after it, the server surrenders custody and hands
/// the request back with `Rejected { retryable: true }`.
const MAX_CLAIM_REARMS: u8 = 3;

/// How long one claim may sit in [`ClaimState::Retained`] before the reaper
/// frees its custody outright.
///
/// Retention costs the gateway nothing — it publishes no disposition, so the
/// gateway keeps its own replay copy — but it does cost the *server* a custody
/// slot, and only [`ServerInputClaims::dispose`] returns one. Fill a
/// connection's `MAX_IN_FLIGHT_GATEWAY_INPUTS_PER_CONNECTION` slots with
/// retentions and that connection answers `Full` for the life of the boundary;
/// this TTL is the floor under that.
///
/// The number is derived from the gateway's replay cadence, because a replay
/// that re-arms a claim has to win the race against the reaper: a reaped claim
/// the gateway replays returns as a *fresh* `New` admission with a fresh
/// re-arm budget, so a reaper that fired early would be a loop rather than a
/// bound. The gateway re-sends a held request only when it replays its whole
/// input custody onto a fresh boundary, and the slowest heal it can take is
/// roughly `EXIT_WAIT` (5 s) + `BACKOFF_CAP` (5 s) + `RECONNECT_TIMEOUT`
/// (10 s) + `RESYNC_TIMEOUT` (10 s) ≈ 30 s — the same order as the gateway's
/// own `CRASH_WINDOW`. This is **10×** that, so the reaper is the slow path.
const RETAINED_CLAIM_TTL: Duration = Duration::from_secs(300);

/// How often the retained-claim reaper wakes to scan for expired retentions.
///
/// A tenth of [`RETAINED_CLAIM_TTL`], so a reaped claim's real age is within
/// 10% of the TTL. Each pass is one bounded scan of a table the admission cap
/// already holds at `MAX_IN_FLIGHT_GATEWAY_INPUTS`.
const RETAINED_CLAIM_REAP_INTERVAL: Duration = Duration::from_secs(30);

/// The two custody states one server claim can be in.
///
/// `Active` means some handler owns this uid's single disposition. `Retained`
/// means no handler does: the request's durable fate is unproven, both sides
/// still hold it, and the next exact replay re-arms it. The instant is
/// [`tokio::time::Instant`] — `std::time::Instant` does not move under
/// `tokio::time::pause`, so a test could not reach the age this records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClaimState {
    Active {
        rearms: u8,
    },
    Retained {
        since: tokio::time::Instant,
        reason: &'static str,
        rearms: u8,
    },
}

impl ClaimState {
    /// How many times this claim has already been re-armed — carried across
    /// the flip back to `Active`, because it is the *claim's* budget and not
    /// one retention's.
    const fn rearms(self) -> u8 {
        match self {
            ClaimState::Active { rearms } | ClaimState::Retained { rearms, .. } => rearms,
        }
    }
}

#[derive(Clone, Debug)]
struct ServerLegacyClaim {
    request: GatewayRequest,
    bytes: usize,
    state: ClaimState,
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
    /// An exact replay of a *retained* claim: custody was already held, the
    /// claim is now `Active` again under the same lock that read it, and the
    /// caller owes it one fresh handler. Flipping the state here — rather than
    /// letting the caller do it after the guard drops — is what stops two
    /// concurrent replays from spawning two handlers for one uid.
    Rearm,
    /// A retained claim that has spent its re-arm budget. `admit` has already
    /// disposed of it; the caller owes the gateway a terminal answer.
    Exhausted,
    Full {
        warn: bool,
    },
    Invalid,
    Conflict,
}

/// What a peer-driven `GatewayReleased` did to one claim.
///
/// The claim's kind and its custody state travel together so the reader loop
/// can branch on the outcome without re-matching a payload the table has
/// already classified under the lock.
#[derive(Clone, Debug, PartialEq)]
enum ServerClaimRelease {
    /// No claim under that uid: a duplicate or stale release.
    Unknown,
    /// A legacy command claim. Server custody is surrendered and the request
    /// is the caller's to enqueue exactly once.
    Legacy(GatewayRequest),
    /// A coordinator action claim, still in coordinator custody. The peer does
    /// not own an action's fate: the claim is left in the table for
    /// `handle_authoritative_input` to dispose, and it is never enqueued as
    /// `Inbound`.
    CoordinatorHeld,
}

impl ServerInputClaims {
    fn admit(&mut self, request: GatewayRequest) -> ServerClaimAdmission {
        if !request.has_valid_uid() {
            return ServerClaimAdmission::Invalid;
        }
        let Ok(bytes) = request.input.logical_bytes() else {
            return ServerClaimAdmission::Invalid;
        };
        if let Some(existing) = self.entries.get_mut(&request.uid) {
            if existing.request != request {
                return ServerClaimAdmission::Conflict;
            }
            match existing.state {
                // A handler owns this uid's disposition already.
                ClaimState::Active { .. } => return ServerClaimAdmission::Duplicate,
                ClaimState::Retained {
                    since,
                    reason,
                    rearms,
                } if rearms < MAX_CLAIM_REARMS => {
                    existing.state = ClaimState::Active { rearms: rearms + 1 };
                    tracing::warn!(
                        request = %request.uid,
                        reason,
                        retained_for_ms = since.elapsed().as_millis(),
                        rearms = rearms + 1,
                        "re-arming a retained coordinator claim on an exact replay"
                    );
                    return ServerClaimAdmission::Rearm;
                }
                // Out of re-arms: stop holding a request nobody can resolve.
                ClaimState::Retained { reason, .. } => {
                    tracing::error!(
                        request = %request.uid,
                        reason,
                        max = MAX_CLAIM_REARMS,
                        "retained claim exhausted its re-arm budget; surrendering custody"
                    );
                }
            }
            self.dispose(request.uid);
            return ServerClaimAdmission::Exhausted;
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

        self.entries.insert(
            request.uid,
            ServerLegacyClaim {
                request,
                bytes,
                state: ClaimState::Active { rearms: 0 },
            },
        );
        self.bytes += bytes;
        let account = self.per_connection.entry(conn).or_default();
        account.count += 1;
        account.bytes += bytes;
        self.capacity_warned = false;
        ServerClaimAdmission::New
    }

    /// Park one claim in [`ClaimState::Retained`]: this handler is finished
    /// but the request's durable fate is *unproven*, so custody is kept rather
    /// than answered with a disposition that would be a lie in one direction
    /// or the other.
    ///
    /// Nothing is published, which is precisely what leaves the gateway's own
    /// replay copy in place: its next replay of this uid re-arms the claim
    /// (see [`ServerInputClaims::admit`]) and re-runs the read-only preflight.
    /// Returns `false` if the claim is already gone — a drifted caller, not a
    /// state to invent.
    fn retain(&mut self, uid: RequestUid, reason: &'static str) -> bool {
        let Some(claim) = self.entries.get_mut(&uid) else {
            return false;
        };
        claim.state = ClaimState::Retained {
            since: tokio::time::Instant::now(),
            reason,
            rearms: claim.state.rearms(),
        };
        true
    }

    /// Free every claim that has been [`ClaimState::Retained`] for longer than
    /// `ttl`, reporting the uids so one pass can be logged as one line.
    ///
    /// A reaped claim is answered with the same silence retention itself keeps:
    /// no disposition at all. That is what makes the reap safe — the gateway
    /// still holds its replay copy, so nothing is lost, and its next replay is
    /// admitted as an ordinary `New` claim. What the reap buys back is the
    /// custody slot, through [`ServerInputClaims::dispose`]'s saturating
    /// accounting, so a connection that filled its budget with retentions is
    /// not `Full` forever.
    ///
    /// `Active` claims are never touched: a handler still owns each one's
    /// single disposition, and an entry taken out from under it would let a
    /// replay of that uid spawn a second handler.
    fn reap_retained(&mut self, ttl: Duration) -> Vec<RequestUid> {
        let expired: Vec<RequestUid> = self
            .entries
            .iter()
            .filter_map(|(uid, claim)| match claim.state {
                ClaimState::Retained { since, .. } if since.elapsed() >= ttl => Some(*uid),
                _ => None,
            })
            .collect();
        for uid in &expired {
            let _ = self.dispose(*uid);
        }
        expired
    }

    /// Free one connection's retained claims because that connection detached.
    ///
    /// A detached connection has no session left to answer, and its retentions
    /// are waiting on a replay that will now never come from it — but the
    /// custody they hold is charged to a `per_connection` account that a
    /// reconnect reuses. `Active` claims survive for the same reason the reap
    /// leaves them alone.
    ///
    /// This is a full scan of `entries`, bounded by the admission cap
    /// `MAX_IN_FLIGHT_GATEWAY_INPUTS`, because `per_connection` keys **counts
    /// only**: there is no uid index on this table and WS03-S15 deliberately
    /// adds none, so anything restructuring the table later should know it is
    /// building that index from scratch.
    fn release_connection_retained(&mut self, conn: u64) -> Vec<RequestUid> {
        let detached: Vec<RequestUid> = self
            .entries
            .iter()
            .filter_map(|(uid, claim)| {
                let retained = matches!(claim.state, ClaimState::Retained { .. });
                (retained && claim.request.input.conn() == conn).then_some(*uid)
            })
            .collect();
        for uid in &detached {
            let _ = self.dispose(*uid);
        }
        detached
    }

    /// Surrender custody of one claim unconditionally and reconcile the
    /// accounting. This is the *owner's* disposal path: the coordinator (and
    /// the legacy release below) already decided this request's fate.
    fn dispose(&mut self, uid: RequestUid) -> Option<GatewayRequest> {
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

    /// The peer-driven release: the gateway says it has dropped its replay
    /// copy of one request.
    ///
    /// A legacy command is the only kind the gateway may release this way, and
    /// releasing it is what authorizes execution. A coordinator action is
    /// released by its disposition instead, so a release naming one is refused
    /// twice over: custody stays in the table for the in-flight handler, and
    /// the payload never becomes an `Inbound`.
    fn release(&mut self, uid: RequestUid) -> ServerClaimRelease {
        let Some(held) = self.entries.get(&uid) else {
            return ServerClaimRelease::Unknown;
        };
        if matches!(held.request.input, GatewayInput::Action(_)) {
            return ServerClaimRelease::CoordinatorHeld;
        }
        match self.dispose(uid) {
            Some(request) => ServerClaimRelease::Legacy(request),
            None => ServerClaimRelease::Unknown,
        }
    }

    /// One consistent custody reading: held claims and held bytes taken under
    /// the same guard, so a log line can never pair a count from one instant
    /// with a byte total from another.
    fn capacity_snapshot(&self) -> (usize, usize) {
        (self.entries.len(), self.bytes)
    }
}

/// The crate's single mutex-poison policy for the IPC claim table.
///
/// A panic anywhere in a claim path used to poison this mutex and then abort
/// the reader loop for *every* connection on the next lock. Claim mutations
/// are short, non-reentrant, and already reconciled by [`ServerInputClaims::release`]'s
/// saturating arithmetic, so a poisoned table is recovered rather than
/// escalated — the same policy the writer mailbox already chose in
/// [`crate::world_task`].
pub(crate) fn claims_guard(claims: &Mutex<ServerInputClaims>) -> MutexGuard<'_, ServerInputClaims> {
    claims.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Admit one request and read the resulting custody in a single lock
/// acquisition, so the overload log reports a snapshot that actually existed.
pub(crate) fn admit_with_custody_snapshot(
    claims: &Mutex<ServerInputClaims>,
    request: GatewayRequest,
) -> (ServerClaimAdmission, usize, usize) {
    let mut claims = claims_guard(claims);
    let admission = claims.admit(request);
    let (held, bytes) = claims.capacity_snapshot();
    (admission, held, bytes)
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
    claims_guard(claims).dispose(request)
}

/// Keep custody of one coordinator claim without publishing a disposition,
/// because the request's durable fate could be neither proved nor denied.
///
/// This is the *only* honest answer for an ambiguous outcome: `Busy` and
/// `Rejected` both assert "no intent exists", which a store that never
/// answered cannot support, and `Owned` asserts one does. Retention asserts
/// nothing and costs the gateway nothing — it still holds its replay copy —
/// so its next replay re-arms the claim and re-runs the read-only lookup.
fn retain_input_claim(
    claims: &Arc<Mutex<ServerInputClaims>>,
    request: RequestUid,
    reason: &'static str,
) {
    if claims_guard(claims).retain(request, reason) {
        tracing::warn!(
            %request,
            reason,
            "coordinator custody retained; awaiting an exact gateway replay"
        );
    } else {
        // Only a drifted caller reaches this: the claim was disposed of by
        // some other path while its handler still believed it owned it.
        tracing::error!(
            %request,
            reason,
            "retention named a claim the custody table no longer holds"
        );
    }
}

/// The retained-claim reaper's handle, which aborts the task when it drops.
///
/// The reaper is spawned inside [`reader_loop`] and must not outlive it. That
/// loop has several early `return Err(..)` exits — the raw-input guard, the
/// invalid/conflicting-identity arms, every `world_is_gone` — and `serve` can
/// also drop the reader future outright, so a task cancelled only at the tail
/// of the function would survive most of the ways the boundary actually ends,
/// holding an `Arc` to a claim table nobody else can reach.
///
/// Two independent mechanisms end it, because an abort is scheduling and the
/// stop signal is not: dropping `_stop` closes the reaper's oneshot, which the
/// sleep is selected against, so a reaper parked for the whole interval wakes
/// and returns at once instead of sleeping through the teardown; and the abort
/// covers a reaper caught mid-pass. The claim table is held only by a `Weak`,
/// so even a reaper that somehow outlived both finds nothing to reap.
struct RetainedClaimReaper {
    handle: tokio::task::JoinHandle<()>,
    _stop: tokio::sync::oneshot::Sender<()>,
}

impl Drop for RetainedClaimReaper {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Spawn the boundary's retained-claim reaper.
///
/// Retention (WS03-S4) is the honest answer to an unprovable durable fate, but
/// it is only bounded while replays keep arriving. Within one long-lived
/// boundary no replay ever comes — the gateway re-sends its custody pool on
/// re-attach, and a re-attach means a *new* boundary with an empty table — so
/// without this pass a retained claim holds its slot until the boundary ends.
///
/// The reap publishes nothing: see [`ServerInputClaims::reap_retained`].
fn spawn_retained_claim_reaper(
    claims: &Arc<Mutex<ServerInputClaims>>,
    metrics: &Arc<Metrics>,
) -> RetainedClaimReaper {
    // WEAK on purpose: the reader loop owns the table's lifetime, and a reaper
    // that kept it alive would keep a dead boundary's custody alive with it.
    let claims = Arc::downgrade(claims);
    let metrics = Arc::clone(metrics);
    let (stop, mut stopped) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                () = tokio::time::sleep(RETAINED_CLAIM_REAP_INTERVAL) => {}
                // The reader loop dropped the guard: stop now rather than
                // finish a sleep the boundary has no use for.
                _ = &mut stopped => return,
            }
            let Some(claims) = claims.upgrade() else {
                return;
            };
            // The guard is a statement temporary: the claim table's mutex is
            // never held across the awaits above or below it.
            let reaped = claims_guard(&claims).reap_retained(RETAINED_CLAIM_TTL);
            drop(claims);
            if !reaped.is_empty() {
                metrics.ipc_retained_claims_reaped(reaped.len() as u64);
                tracing::warn!(
                    reaped = reaped.len(),
                    ttl_s = RETAINED_CLAIM_TTL.as_secs(),
                    "freed retained coordinator claims that aged out; the gateway keeps its replay copies"
                );
                for request in reaped {
                    tracing::debug!(%request, "retained claim reaped");
                }
            }
        }
    });
    RetainedClaimReaper {
        handle,
        _stop: stop,
    }
}

async fn handle_authoritative_input(
    request: GatewayRequest,
    routes: Arc<OfferRouteTable>,
    world: WorldHandle,
    store: Arc<dyn ObjectStore>,
    sink: SocketSink,
    claims: Arc<Mutex<ServerInputClaims>>,
    metrics: Arc<Metrics>,
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
        // A configuration fault, not an ambiguity: this store will never grow
        // the capability, so retention would hold the claim while every replay
        // failed identically. Answer terminally instead.
        release_input_claim(&claims, uid);
        request_disposition(
            &sink,
            uid,
            GatewayRequestDisposition::Rejected { retryable: false },
        );
        tracing::error!(%uid, "configured store lacks authoritative request capability");
        return;
    };
    let preflight_started = tokio::time::Instant::now();
    let mut preflight_delay = Duration::from_millis(100);
    let mut preflight_attempts = 0u32;
    let preflight = loop {
        preflight_attempts += 1;
        match ActionCoordinator::preflight(authoritative_store, envelope.clone()).await {
            Ok(preflight) => break preflight,
            Err(error) if error.is_transient() => {
                // Busy means "proved no intent" and would be a lie here: this
                // may be a replay whose exact intent already exists. Retry the
                // read-only lookup in place, but only for a bounded window —
                // this loop is the fast path for a store hiccup, not the
                // durable one. The durable one is retention plus the gateway's
                // own replay, which survives even a server restart.
                let spent = preflight_started.elapsed();
                if preflight_attempts >= MAX_PREFLIGHT_ATTEMPTS
                    || spent.saturating_add(preflight_delay) > PREFLIGHT_RETRY_WINDOW
                {
                    tracing::error!(
                        %uid,
                        error = %error,
                        attempts = preflight_attempts,
                        spent_ms = spent.as_millis(),
                        "coordinator preflight stayed transient to the bound"
                    );
                    // Retention is silent to the gateway on purpose, so this
                    // counter is the operator's only view of a store that is
                    // answering neither yes nor no.
                    metrics.coordinator_preflight_deferred();
                    retain_input_claim(&claims, uid, "preflight transient to the bound");
                    return;
                }
                tracing::warn!(%uid, error = %error, attempt = preflight_attempts, "coordinator preflight transient; retrying in place");
                tokio::time::sleep(preflight_delay).await;
                preflight_delay = (preflight_delay * 2).min(Duration::from_secs(5));
            }
            Err(error) => {
                // A deterministic/ambiguous store error cannot prove that no
                // intent exists. Preserve both gateway and server custody and
                // publish no lying rejection.
                tracing::error!(%uid, error = %error, "coordinator preflight could not prove request state");
                retain_input_claim(&claims, uid, "preflight could not prove request state");
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
            // Quarantine is terminal evidence: the request consumed its
            // idempotency binding and must never be planned again. It is not
            // completion, so `Finished` would be a lie — but the intent IS
            // durably owned, and `Owned` says exactly that and no more. The
            // status lookup is the honest follow-up, and it is terminal for
            // the gateway (which drops its replay copy on any disposition),
            // so no new wire variant is needed to say it.
            tracing::error!(
                %uid,
                token = %receipt.token(),
                evidence = %receipt.evidence_hash(),
                "coordinator request is quarantined; answering from the durable record"
            );
            release_input_claim(&claims, uid);
            request_disposition(
                &sink,
                uid,
                GatewayRequestDisposition::Owned {
                    token: receipt.token(),
                    status: status.clone(),
                },
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
            tracing::error!(%uid, %detail, "coordinator ownership became ambiguous; retaining custody");
            retain_input_claim(&claims, uid, "ownership became ambiguous");
            return;
        }
        Err(_) => {
            tracing::error!(%uid, "world dropped coordinator ownership result; retaining custody");
            retain_input_claim(&claims, uid, "world dropped the ownership result");
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

/// The error every closed-world exit raises, logged at a severity that depends
/// on where the shutdown came from.
///
/// A `false` from [`WorldHandle::inbound`] means the world task's receiver is
/// gone. Nothing re-opens it: the world is single-writer and never restarts in
/// place, so the condition is PERMANENT and the reader must end the boundary
/// rather than keep accepting frames it can never deliver.
///
/// Severity is provenance, not damage. An operator `@shutdown` — typed in game
/// or arriving as `ToServer::Control` — is an ordinary exit and logs at INFO;
/// a world that vanished with no shutdown on record is a fault and logs at
/// ERROR. `shutdown_seen` covers the frame this reader forwarded itself;
/// `world_exit` covers the in-game path, and is set by the world loop's own
/// last statement before its receiver drops.
fn world_is_gone(
    site: &'static str,
    shutdown_seen: bool,
    world_exit: &WorldExitStatus,
) -> std::io::Error {
    if shutdown_seen || world_exit.is_clean() {
        tracing::info!(site, "world task has shut down; ending the boundary");
    } else {
        tracing::error!(
            site,
            "world task is gone with no shutdown on record; ending the boundary so the unit restarts this process"
        );
    }
    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "world task is gone")
}

/// Pump the gateway's `ToServer` traffic into the world.
///
/// The first frame decides the link's protocol version (doc 33 §4.1): a
/// [`ToServer::Hello`] inside this server's window is admitted and recorded in
/// `link` for the writer's emission gate; one outside it is refused with
/// [`IPC_VERSION_SKEW_MARKER`]; any other first frame is a pre-window gateway
/// ([`IPC_PRE_HELLO_VERSION`]), admitted while that version is in the window.
/// See [`admit_gateway`].
///
/// **Exposed only so `tests/world_task/dead_world_reader.rs` can drive it against a world
/// task it has already stopped.** Nothing outside this crate composes a
/// boundary; [`serve`] is the entry point.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)] // GG.0 added the link-version cell; see `writer_loop`
pub async fn reader_loop(
    r: mush_ipc::BoxRead,
    world: WorldHandle,
    store: Arc<dyn ObjectStore>,
    offer_routes: Arc<OfferRouteTable>,
    reply_sink: SocketSink,
    metrics: &Arc<Metrics>,
    world_exit: WorldExitStatus,
    link: LinkVersion,
) -> std::io::Result<()> {
    let mut reader = FrameReader::new(r);
    let service_requests = ServiceRequestLimiter::new();
    let input_claims = Arc::new(Mutex::new(ServerInputClaims::default()));
    // Held in this function's own state, so EVERY way out of it — the early
    // `return Err(..)` guards below, a clean EOF, or `serve` dropping the
    // reader future — drops the guard and ends the reaper with the boundary.
    let _retained_claim_reaper = spawn_retained_claim_reaper(&input_claims, metrics);
    // Set the moment this reader forwards a `ControlMsg::Shutdown`, so the
    // closed channel that follows reads as the exit it asked for.
    let mut shutdown_seen = false;
    // The first frame names the gateway's version, or its shape does.
    let mut first_frame = true;
    while let Some(msg) = reader.read::<ToServer>().await? {
        if first_frame {
            first_frame = false;
            if let ToServer::Hello { proto, min_proto } = &msg {
                admit_gateway(&link, *proto, Some(*min_proto))?;
                continue;
            }
            admit_gateway(&link, IPC_PRE_HELLO_VERSION, None)?;
        }
        match msg {
            ToServer::Hello { proto, min_proto } => {
                tracing::error!(
                    proto,
                    min_proto,
                    "a Hello after the first frame: the gateway's version was already settled"
                );
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Hello is a link's first frame and never repeats",
                ));
            }
            ToServer::Inbound(wire) => {
                if matches!(
                    wire,
                    mush_ipc::WireInbound::Cmd { .. }
                        | mush_ipc::WireInbound::Query { .. }
                        | mush_ipc::WireInbound::Invoke { .. }
                ) {
                    tracing::error!("raw player input bypassed the version-3 custody envelope");
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "player input requires GatewayRequest custody",
                    ));
                }
                if !world.inbound(Inbound::from(wire)).await {
                    return Err(world_is_gone("inbound", shutdown_seen, &world_exit));
                }
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
                    GatewayInput::Query {
                        conn,
                        actor,
                        id,
                        name,
                        args,
                    } => {
                        // Built from the arm's own fields: the kind is already
                        // decided here, so there is no second, fallible
                        // classification of the payload on this path.
                        let query = Inbound::Query {
                            conn: *conn,
                            actor: *actor,
                            id: *id,
                            name: name.clone(),
                            args: args.clone(),
                        };
                        if !world.inbound(query).await {
                            // A closed world channel is permanent, so this is
                            // the one disposition the gateway must NOT retry:
                            // answering `retryable: true` invited a replay into
                            // a world that will never take it.
                            request_disposition(
                                &reply_sink,
                                uid,
                                GatewayRequestDisposition::Rejected { retryable: false },
                            );
                            return Err(world_is_gone("query", shutdown_seen, &world_exit));
                        }
                        request_disposition(
                            &reply_sink,
                            uid,
                            GatewayRequestDisposition::EphemeralFinished,
                        );
                    }
                    GatewayInput::Cmd { .. } | GatewayInput::Invoke { .. } => {
                        let (admission, held, bytes) =
                            admit_with_custody_snapshot(&input_claims, request);
                        match admission {
                            ServerClaimAdmission::New | ServerClaimAdmission::Duplicate => {
                                request_disposition(
                                    &reply_sink,
                                    uid,
                                    GatewayRequestDisposition::LegacyClaimed,
                                );
                            }
                            ServerClaimAdmission::Rearm | ServerClaimAdmission::Exhausted => {
                                // Unreachable by construction: only the
                                // coordinator retains a claim, and it retains
                                // only its own actions. Fail closed rather
                                // than re-authorize an execution this arm
                                // never classified.
                                tracing::error!(
                                    request = %uid,
                                    "legacy claim reported a coordinator-only custody state"
                                );
                                request_disposition(
                                    &reply_sink,
                                    uid,
                                    GatewayRequestDisposition::Rejected { retryable: false },
                                );
                            }
                            ServerClaimAdmission::Full { warn } => {
                                if warn {
                                    tracing::warn!(
                                        request = %uid,
                                        held,
                                        bytes,
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
                        let admission = claims_guard(&input_claims).admit(request.clone());
                        match admission {
                            // `Rearm` is a claim the table just flipped back to
                            // `Active` under its own lock, so exactly one
                            // handler exists for it here too.
                            ServerClaimAdmission::New | ServerClaimAdmission::Rearm => {
                                let world = world.clone();
                                let store = Arc::clone(&store);
                                let routes = Arc::clone(&offer_routes);
                                let sink = reply_sink.clone();
                                let claims = Arc::clone(&input_claims);
                                let metrics = Arc::clone(metrics);
                                tokio::spawn(async move {
                                    handle_authoritative_input(
                                        request, routes, world, store, sink, claims, metrics,
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
                            ServerClaimAdmission::Exhausted => {
                                // The re-arm budget is spent and `admit` has
                                // already surrendered custody. Hand the
                                // request back through the vocabulary the
                                // gateway already releases its replay copy on,
                                // so no uid can be retained forever.
                                tracing::error!(
                                    request = %uid,
                                    "retained coordinator claim exhausted its re-arms; returning it to the gateway"
                                );
                                request_disposition(
                                    &reply_sink,
                                    uid,
                                    GatewayRequestDisposition::Rejected { retryable: true },
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
                // Take the outcome out of the guard before any await: the claim
                // table's mutex is never held across a suspension point.
                let released = claims_guard(&input_claims).release(request);
                let claim = match released {
                    ServerClaimRelease::Legacy(claim) => claim,
                    ServerClaimRelease::Unknown => {
                        tracing::debug!(
                            request = %request,
                            "duplicate or stale legacy release ignored"
                        );
                        continue;
                    }
                    ServerClaimRelease::CoordinatorHeld => {
                        // A conforming gateway releases only what the server
                        // answered `LegacyClaimed`; naming a coordinator action
                        // here is a raw peer's frame. The in-flight handler
                        // still owns this uid's custody and its single
                        // disposition, so publish nothing and take nothing.
                        tracing::error!(
                            request = %request,
                            "peer released a coordinator action; custody stays with the coordinator"
                        );
                        continue;
                    }
                };
                let Ok(inbound) = Inbound::try_from(claim.input) else {
                    // Unreachable by construction — `release` surrenders custody
                    // only for a claim it classified as legacy — so fail closed
                    // on a drifted classification rather than execute it.
                    tracing::error!(
                        request = %request,
                        "released claim is not world input; dropped without execution"
                    );
                    continue;
                };
                // Remove the server claim before enqueueing. From this point a
                // crash can lose the command, but neither this boundary nor a
                // gateway replay can execute it twice.
                if !world.inbound(inbound).await {
                    tracing::warn!(
                        request = %request,
                        "world closed after legacy release; command was not executed"
                    );
                    return Err(world_is_gone(
                        "gateway_released",
                        shutdown_seen,
                        &world_exit,
                    ));
                }
            }
            ToServer::Control(ctrl) => {
                // Record the intent BEFORE the send: an operator shutdown that
                // finds the world already gone is still an ordinary exit, not a
                // fault to relaunch over.
                shutdown_seen |= matches!(ctrl, ControlMsg::Shutdown);
                if !world.inbound(Inbound::Control(ctrl)).await {
                    return Err(world_is_gone("control", shutdown_seen, &world_exit));
                }
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
                        output_brackets: r.output_brackets,
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
            // session mirror (the gateway owns it), so an attach is accepted
            // and ignored.
            ToServer::Attach { .. } => {}
            // A detach is not: this connection's RETAINED claims are waiting on
            // a replay it can no longer send, and their custody is charged to a
            // `per_connection` account a reconnect reuses. Free them here
            // rather than leave them to the TTL. Active claims stay — a handler
            // still owns each one's single disposition — and nothing is
            // published, exactly as at the reaper.
            ToServer::Detach { conn, .. } => {
                let released = claims_guard(&input_claims).release_connection_retained(conn);
                if !released.is_empty() {
                    tracing::warn!(
                        conn,
                        released = released.len(),
                        "freed retained coordinator claims for a detached connection"
                    );
                }
            }
        }
    }
    Ok(())
}

/// Admit or refuse the gateway on a link from what its first frame said (doc
/// 33 §4.1): `announced` is the `proto` of its [`ToServer::Hello`], or
/// [`IPC_PRE_HELLO_VERSION`] for a link whose first frame was anything else
/// (`min_proto` is then `None`). An announced version inside this server's
/// window ([`ProtoWindow::OURS`]) is recorded in `link`, where the writer's
/// emission gate reads it; one outside it is refused, logged with
/// [`IPC_VERSION_SKEW_MARKER`], and the error carries the marker too, so the
/// reader's exit line names it as well.
fn admit_gateway(
    link: &LinkVersion,
    announced: u32,
    min_proto: Option<u32>,
) -> std::io::Result<()> {
    let ours = ProtoWindow::OURS;
    if !ours.contains(announced) {
        tracing::error!(
            marker = IPC_VERSION_SKEW_MARKER,
            gateway_proto = announced,
            gateway_min_proto = min_proto,
            server_window = %ours,
            "the gateway on this link speaks an ipc version outside this server's window; \
             refusing the link (the server upgrades first and serves one version back, \
             doc 33 §4.1)"
        );
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "{IPC_VERSION_SKEW_MARKER}: gateway speaks ipc v{announced}, this server serves \
                 {ours}"
            ),
        ));
    }
    link.record(announced);
    match min_proto {
        Some(min_proto) => tracing::info!(
            gateway_proto = announced,
            gateway_min_proto = min_proto,
            server_window = %ours,
            "gateway hello; link version recorded"
        ),
        None => tracing::info!(
            gateway_proto = announced,
            server_window = %ours,
            "gateway sent no Hello: a pre-window peer (doc 33 §4.1 bootstrap rule); link \
             version recorded"
        ),
    }
    Ok(())
}

/// Answer one [`ServiceReq`] by calling the world handle. Each maps to an
/// existing ack-gated / oneshot world-task round trip; the await happens here
/// (off the world loop), never inside `run`.
///
/// This is the ONE dispatch table both deployments answer from: the forked
/// server's boundary reader calls it directly, and the all-in-one binary
/// reaches it through the in-process [`GatewayLink`](mush_ipc::GatewayLink)
/// impl in [`crate::gateway_link`]. The exhaustive match therefore lives in a
/// single place, so a new variant is a compile error once rather than a
/// silently unanswered request on one of the two links.
///
/// **Exposed only so `tests/split_boundary/gateway_link_parity.rs` can drive it beside the
/// in-process link and prove the two agree** — the same reason
/// [`reader_loop`] is reachable from `tests/world_task/dead_world_reader.rs`. Nothing
/// outside this crate answers a `ServiceReq`; the boundary reader does.
#[doc(hidden)]
pub async fn handle_request(world: &WorldHandle, req: ServiceReq) -> ServiceResp {
    match req {
        ServiceReq::CreatePlayer { name, account } => {
            create_player_response(world.create_player(&name, account).await)
        }
        ServiceReq::UnlinkPlayer { character } => match world.unlink_player(character).await {
            Ok(forgotten) => ServiceResp::UnlinkPlayer(forgotten),
            Err(WorldReadError::Busy) => ServiceResp::Busy,
            Err(WorldReadError::Unavailable) => ServiceResp::Unavailable,
        },
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
        ServiceReq::ReplyBarrier { .. } => barrier_response(world.reply_barrier_result().await),
        ServiceReq::DiscordInteraction(interaction) => {
            match world.discord_interaction(interaction).await {
                Ok(result) => ServiceResp::DiscordInteraction(result),
                Err(WorldReadError::Busy) => ServiceResp::Busy,
                Err(WorldReadError::Unavailable) => ServiceResp::Unavailable,
            }
        }
        ServiceReq::RelaySpeak(req) => match world.relay_speak(req).await {
            Ok(result) => ServiceResp::RelaySpeak(result),
            Err(WorldReadError::Busy) => ServiceResp::Busy,
            Err(WorldReadError::Unavailable) => ServiceResp::Unavailable,
        },
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
mod world_exit_tests {
    //! What `serve` does with the world task's ending. `main` maps the `Err`
    //! to `ExitCode::FAILURE`, and that exit code is the only thing systemd
    //! can act on — so a fault that classified as `Ok` would leave
    //! the forked half looking healthy while its world was gone.

    use super::{WorldOutcome, world_exit_result};
    use crate::world_task::WorldFault;

    #[test]
    fn a_worlds_own_break_is_an_ordinary_exit() {
        assert_eq!(world_exit_result(Ok(WorldOutcome::Clean)), Ok(()));
    }

    #[test]
    fn a_caught_fault_names_its_site_and_fails_the_process() {
        let result = world_exit_result(Ok(WorldOutcome::Faulted(WorldFault {
            site: "engine.step",
            detail: "module unwrapped a None".to_owned(),
        })));
        let detail = result.expect_err("a faulted world must not exit success");
        assert!(
            detail.contains("engine.step") && detail.contains("module unwrapped a None"),
            "the exit reason carries the fault's site and payload: {detail}"
        );
    }

    #[tokio::test]
    async fn a_panic_across_an_await_fails_the_process_too() {
        // The dispatch boundary cannot cover a panic across an `.await`, so
        // this arrives as a real `JoinError` rather than a `WorldOutcome`.
        let joined = tokio::spawn(async { panic!("a driver panicked mid-await") })
            .await
            .map(|(): ()| WorldOutcome::Clean);
        assert!(
            joined.as_ref().is_err_and(tokio::task::JoinError::is_panic),
            "the rig really panicked the task"
        );
        assert!(
            world_exit_result(joined).is_err(),
            "an uncaught world-task panic is a failure exit"
        );
    }
}

#[cfg(test)]
mod service_request_tests {
    use super::{
        Arc, CreatePlayerErr, GatewayInput, GatewayRequest, MAX_CLAIM_REARMS,
        MAX_IN_FLIGHT_GATEWAY_INPUTS_PER_CONNECTION, MAX_IN_FLIGHT_SERVICE_REQUESTS, Metrics,
        Mutex, Notify, RETAINED_CLAIM_REAP_INTERVAL, RETAINED_CLAIM_TTL, ServerClaimAdmission,
        ServerClaimRelease, ServerInputClaims, ServiceRequestLimiter, ServiceResp, SocketSink,
        ToGateway, WorldReadError, admit_with_custody_snapshot, barrier_response, claims_guard,
        create_player_response, release_input_claim, spawn_retained_claim_reaper,
    };
    use mush_core::{ActionOfferId, CanonicalHash, Inbound, ObjId, PerceivedRef};
    use mush_ipc::GatewayAction;
    use std::sync::atomic::{AtomicBool, Ordering};

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

        assert_eq!(claims.release(first.uid), ServerClaimRelease::Legacy(first));
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

        let ServerClaimRelease::Legacy(released) = claims.release(legacy_request(0, 9).uid) else {
            panic!("a legacy claim releases to its request");
        };
        assert_eq!(released.input.conn(), 9);
        assert_eq!(
            claims.admit(legacy_request(10_002, 9)),
            ServerClaimAdmission::New,
            "release must restore exactly one slot"
        );
    }

    /// Every claim below costs the same custody, so `bytes == held * UNIT` is
    /// an invariant of the table that only a torn two-lock read can break.
    const UNIFORM_CLAIM_BYTES: usize = 16 + "say 00000000".len();

    fn uniform_request(sequence: u64, conn: u64) -> GatewayRequest {
        GatewayRequest::new(
            19,
            sequence,
            GatewayInput::Cmd {
                conn,
                actor: ObjId(7),
                line: format!("say {sequence:08}"),
            },
        )
    }

    /// A panic inside any claim path poisons this mutex. The six IPC reader
    /// call sites used to `.expect` on it, so one poisoned lock tore the reader
    /// loop down for *every* connection. The claim table is short-lived,
    /// non-reentrant state reconciled by `release`'s saturating arithmetic, so
    /// the policy is recovery — the retained claims stay usable.
    ///
    /// The panic HOOK is deliberately left alone (W1-B8-FIX, W1-B7-FIX).
    /// Silencing it around the `catch_unwind` reads as tidier, but the hook is
    /// process-global while `cargo test` runs a binary's tests on parallel
    /// threads: two poisonings that overlap interleave their take/set/set-back
    /// (A takes the real hook and installs the silencer, B takes the SILENCER
    /// and installs its own, A restores the real one, B restores the silencer)
    /// and the binary finishes the run with a hook that prints nothing — so
    /// every later panic, in tests that have nothing to do with poisoning,
    /// fails with no message. The cost of not swapping is one deliberate panic
    /// line, which libtest captures and only prints for a test that actually
    /// failed; the benefit is that this test leaves no process-global state
    /// behind at all.
    #[test]
    fn a_poisoned_claim_table_keeps_serving_every_connection() {
        let claims = Arc::new(Mutex::new(ServerInputClaims::default()));
        let first = uniform_request(1, 9);
        assert_eq!(
            claims_guard(&claims).admit(first.clone()),
            ServerClaimAdmission::New
        );

        let poisoner = Arc::clone(&claims);
        let panicked = std::panic::catch_unwind(move || {
            let _held = poisoner.lock().expect("the table starts unpoisoned");
            panic!("a claim path panicked mid-update (expected: this test poisons the table)");
        });
        assert!(panicked.is_err());
        assert!(
            claims.lock().is_err(),
            "the mutex must really be poisoned for this proof to mean anything"
        );

        // All three entry points still see the state the panicking writer left.
        let second = uniform_request(2, 9);
        assert_eq!(
            claims_guard(&claims).admit(second.clone()),
            ServerClaimAdmission::New
        );
        let (admission, held, bytes) = admit_with_custody_snapshot(&claims, first.clone());
        assert_eq!(
            admission,
            ServerClaimAdmission::Duplicate,
            "the retained claim survives the poisoning, so a replay is still exact"
        );
        assert_eq!((held, bytes), (2, 2 * UNIFORM_CLAIM_BYTES));

        assert_eq!(release_input_claim(&claims, first.uid), Some(first));
        assert_eq!(release_input_claim(&claims, second.uid), Some(second));
        assert_eq!(claims_guard(&claims).capacity_snapshot(), (0, 0));
    }

    /// The overload log used to re-lock twice for `held` and `bytes`, pairing a
    /// count from one instant with a byte total from another. One guard now
    /// covers admit + snapshot, so the pair always describes a table that
    /// actually existed.
    #[test]
    fn custody_overload_snapshot_is_taken_under_one_guard() {
        let claims = Arc::new(Mutex::new(ServerInputClaims::default()));
        for sequence in 0..MAX_IN_FLIGHT_GATEWAY_INPUTS_PER_CONNECTION as u64 {
            assert_eq!(
                claims_guard(&claims).admit(uniform_request(sequence, 9)),
                ServerClaimAdmission::New
            );
        }

        let stop = Arc::new(AtomicBool::new(false));
        let churn = {
            let claims = Arc::clone(&claims);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let mut sequence = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    let request = uniform_request(1_000 + (sequence % 16), 2);
                    let uid = request.uid;
                    claims_guard(&claims).admit(request);
                    claims_guard(&claims).release(uid);
                    sequence = sequence.wrapping_add(1);
                }
            })
        };

        for sequence in 0..20_000u64 {
            let (admission, held, bytes) =
                admit_with_custody_snapshot(&claims, uniform_request(50_000 + sequence, 9));
            assert!(
                matches!(admission, ServerClaimAdmission::Full { .. }),
                "the connection is at its cap for the whole run"
            );
            assert_eq!(
                bytes,
                held * UNIFORM_CLAIM_BYTES,
                "held and bytes were read from two different instants"
            );
        }

        stop.store(true, Ordering::Relaxed);
        churn.join().expect("churn thread");
    }

    fn action_request(sequence: u64, conn: u64) -> GatewayRequest {
        GatewayRequest::new(
            19,
            sequence,
            GatewayInput::Action(Box::new(GatewayAction {
                conn,
                actor: ObjId(7),
                offered_action: ActionOfferId(CanonicalHash::from_bytes([3; 32])),
                subject: PerceivedRef::SelfActor,
                targets: Vec::new(),
                parameters: serde_json::json!({}),
            })),
        )
    }

    /// A raw peer could admit an authoritative action and then send
    /// `GatewayReleased` for the same uid. The release path used to convert the
    /// released payload with an infallible `From` whose action arm was an
    /// `unreachable!`, so that frame *pair* panicked the IPC reader loop — in
    /// split mode, an abort into a unit restart with every session
    /// dropped. The peer may not take a coordinator claim at all: custody stays
    /// in the table for the in-flight handler to dispose.
    #[test]
    fn a_peer_release_can_neither_enqueue_nor_evict_a_coordinator_claim() {
        let mut claims = ServerInputClaims::default();
        let action = action_request(1, 9);
        assert_eq!(claims.admit(action.clone()), ServerClaimAdmission::New);
        let (held, bytes) = claims.capacity_snapshot();

        assert_eq!(
            claims.release(action.uid),
            ServerClaimRelease::CoordinatorHeld,
            "an action release must report coordinator custody, not hand back a payload"
        );
        assert_eq!(
            claims.capacity_snapshot(),
            (held, bytes),
            "the claim and its custody accounting must survive the peer's release"
        );
        // Still exactly the claim the coordinator is working on: a replay of the
        // same uid is a duplicate, not a fresh admission that would run twice.
        assert_eq!(
            claims.admit(action.clone()),
            ServerClaimAdmission::Duplicate,
            "the refused release must not have freed the uid"
        );

        // Only the coordinator's own disposal clears it.
        assert_eq!(claims.dispose(action.uid), Some(action.clone()));
        assert_eq!(claims.capacity_snapshot(), (0, 0));
        assert_eq!(claims.release(action.uid), ServerClaimRelease::Unknown);

        // And the payload has no world-input form to be enqueued as, either.
        assert!(Inbound::try_from(action.input).is_err());
    }

    /// Five returns out of `handle_authoritative_input` used to leak the claim:
    /// no release, no disposition, and — because `admit` answered `Duplicate`
    /// for the uid forever after — no way for a gateway replay to restart it
    /// either. That silently wedged one connection's coordinator custody until
    /// it reconnected.
    ///
    /// Retention is the state those returns now take, and an exact replay is
    /// what re-arms it. The flip to `Active` happens under the same guard that
    /// read `Retained`, so a gateway replaying twice cannot spawn two handlers
    /// for one uid: the second replay is an ordinary `Duplicate`.
    #[test]
    fn an_identical_replay_of_a_retained_claim_rearms_once_and_then_is_active() {
        let mut claims = ServerInputClaims::default();
        let action = action_request(1, 9);
        assert_eq!(claims.admit(action.clone()), ServerClaimAdmission::New);
        assert_eq!(
            claims.admit(action.clone()),
            ServerClaimAdmission::Duplicate,
            "a replay while a handler owns the uid must not spawn a second one"
        );
        let custody = claims.capacity_snapshot();

        assert!(claims.retain(action.uid, "test retention"));
        assert_eq!(
            claims.admit(action.clone()),
            ServerClaimAdmission::Rearm,
            "a retained claim must be re-armable, not silently swallowed as a duplicate"
        );
        assert_eq!(
            claims.admit(action.clone()),
            ServerClaimAdmission::Duplicate,
            "the re-arm already flipped the claim Active; a second replay owes no handler"
        );
        assert_eq!(
            claims.capacity_snapshot(),
            custody,
            "re-arming takes no extra custody: it is the same claim"
        );
    }

    /// The other half of the bound: retention may not become a way for one uid
    /// to occupy custody forever. After `MAX_CLAIM_REARMS`, `admit` surrenders
    /// the claim itself and tells the caller to answer terminally — the caller
    /// publishes `Rejected { retryable: true }`, which is the vocabulary the
    /// gateway already drops its own replay copy on.
    #[test]
    fn a_retained_claim_stops_rearming_at_the_bound() {
        let mut claims = ServerInputClaims::default();
        let action = action_request(2, 9);
        assert_eq!(claims.admit(action.clone()), ServerClaimAdmission::New);

        for expected in 1..=MAX_CLAIM_REARMS {
            assert!(claims.retain(action.uid, "test retention"));
            assert_eq!(
                claims.admit(action.clone()),
                ServerClaimAdmission::Rearm,
                "re-arm {expected} is inside the budget"
            );
        }

        assert!(claims.retain(action.uid, "test retention"));
        assert_eq!(
            claims.admit(action.clone()),
            ServerClaimAdmission::Exhausted,
            "the budget is spent: the claim must be surrendered, not retained again"
        );
        assert_eq!(
            claims.capacity_snapshot(),
            (0, 0),
            "an exhausted claim releases its custody and its per-connection budget"
        );
        assert_eq!(
            claims.release(action.uid),
            ServerClaimRelease::Unknown,
            "nothing is left in the table to release"
        );
    }

    /// The re-arm budget is the *claim's*, not one retention's: flipping back
    /// to `Active` may not reset it, or an alternating retain/replay pair would
    /// loop forever.
    #[test]
    fn the_rearm_budget_survives_the_flip_back_to_active() {
        let mut claims = ServerInputClaims::default();
        let action = action_request(3, 9);
        assert_eq!(claims.admit(action.clone()), ServerClaimAdmission::New);
        for _ in 0..MAX_CLAIM_REARMS {
            assert!(claims.retain(action.uid, "test retention"));
            assert_eq!(claims.admit(action.clone()), ServerClaimAdmission::Rearm);
            // Between re-arms the claim is Active, exactly as a live handler
            // leaves it. That must not refund the budget.
            assert_eq!(
                claims.admit(action.clone()),
                ServerClaimAdmission::Duplicate
            );
        }
        assert!(claims.retain(action.uid, "test retention"));
        assert_eq!(claims.admit(action), ServerClaimAdmission::Exhausted);
    }

    /// The wedge WS03-S4's retention bought and WS03-S15 pays for. A retained
    /// claim keeps its custody slot and only `dispose` gives one back, so a
    /// connection whose per-connection budget filled with retentions answered
    /// `Full` for the life of the boundary: within one boundary no gateway
    /// replay ever arrives to re-arm them (the gateway replays its custody pool
    /// onto a *fresh* boundary, whose table is empty).
    ///
    /// The reaper is the floor under that. Time is paused, so the retentions
    /// really do reach `RETAINED_CLAIM_TTL` instead of the test waiting five
    /// minutes — `ClaimState::Retained` records a `tokio::time::Instant` for
    /// exactly this reason.
    #[tokio::test(start_paused = true)]
    async fn retained_claims_do_not_wedge_a_connection_forever() {
        let claims = Arc::new(Mutex::new(ServerInputClaims::default()));
        let metrics = Arc::new(Metrics::new());
        {
            let mut table = claims_guard(&claims);
            for sequence in 0..MAX_IN_FLIGHT_GATEWAY_INPUTS_PER_CONNECTION as u64 {
                let request = action_request(sequence, 9);
                assert_eq!(table.admit(request.clone()), ServerClaimAdmission::New);
                assert!(table.retain(request.uid, "test retention"));
            }
            assert_eq!(
                table.admit(action_request(10_000, 9)),
                ServerClaimAdmission::Full { warn: true },
                "the connection is wedged Full by its own retentions"
            );
        }

        let reaper = spawn_retained_claim_reaper(&claims, &metrics);

        // A pass that runs before the TTL frees nothing: the reaper must be the
        // SLOW path, because a claim it reaps early is one the gateway replays
        // as a fresh admission with a fresh re-arm budget.
        tokio::time::advance(RETAINED_CLAIM_REAP_INTERVAL).await;
        tokio::task::yield_now().await;
        assert_eq!(
            claims_guard(&claims).entries.len(),
            MAX_IN_FLIGHT_GATEWAY_INPUTS_PER_CONNECTION,
            "a retention younger than the TTL is not reaped"
        );

        // Past the TTL, one pass frees every one of them.
        for _ in 0..(2 + RETAINED_CLAIM_TTL.as_secs() / RETAINED_CLAIM_REAP_INTERVAL.as_secs()) {
            let empty = claims_guard(&claims).entries.is_empty();
            if empty {
                break;
            }
            tokio::time::advance(RETAINED_CLAIM_REAP_INTERVAL).await;
            tokio::task::yield_now().await;
        }
        assert!(
            claims_guard(&claims).entries.is_empty(),
            "every retention older than the TTL must be freed"
        );
        assert_eq!(
            metrics.snapshot().ipc_retained_claims_reaped_total,
            MAX_IN_FLIGHT_GATEWAY_INPUTS_PER_CONNECTION as u64,
            "a silent reap still has to be countable"
        );
        assert_eq!(
            claims_guard(&claims).admit(action_request(10_000, 9)),
            ServerClaimAdmission::New,
            "the freed slots are real capacity: the wedged request now admits"
        );
        drop(reaper);
    }

    /// A reaped claim is answered with silence, which is the whole reason the
    /// reap is safe: the gateway still holds its replay copy, so the request
    /// can come back as a fresh `New` admission. Publishing `Busy` or
    /// `Rejected` here would make the gateway drop that copy on a request whose
    /// durable fate was never proven either way.
    ///
    /// The reaper is handed no sink by construction; this pins the observable
    /// consequence — the boundary's whole reply path stays empty across a reap.
    #[tokio::test(start_paused = true)]
    async fn the_reaper_publishes_no_disposition() {
        let (tx, mut replies) = super::mpsc::channel::<ToGateway>(8);
        let metrics = Arc::new(Metrics::new());
        // The real reply path every disposition travels, kept alive for the
        // whole test so an empty receiver means "nothing was sent", never "the
        // sink was gone".
        let _sink = SocketSink::new(tx, Arc::clone(&metrics), Arc::new(Notify::new()), 8, 90);

        let claims = Arc::new(Mutex::new(ServerInputClaims::default()));
        let request = action_request(1, 9);
        {
            let mut table = claims_guard(&claims);
            assert_eq!(table.admit(request.clone()), ServerClaimAdmission::New);
            assert!(table.retain(request.uid, "test retention"));
        }

        let reaper = spawn_retained_claim_reaper(&claims, &metrics);
        for _ in 0..(2 + RETAINED_CLAIM_TTL.as_secs() / RETAINED_CLAIM_REAP_INTERVAL.as_secs()) {
            let empty = claims_guard(&claims).entries.is_empty();
            if empty {
                break;
            }
            tokio::time::advance(RETAINED_CLAIM_REAP_INTERVAL).await;
            tokio::task::yield_now().await;
        }
        assert!(
            claims_guard(&claims).entries.is_empty(),
            "the claim is gone"
        );
        assert!(
            replies.try_recv().is_err(),
            "a reap that published anything would take the gateway's replay copy with it, and \
             that copy is the only thing left that can retry the request"
        );
        drop(reaper);
    }

    /// `ToServer::Detach` used to be an empty arm. A detached connection can
    /// never send the replay its retentions are waiting for, so they go with
    /// it — but only *its* retentions: an `Active` claim on the same connection
    /// still has a live handler owning its single disposition, and another
    /// connection's retention is none of this frame's business.
    #[test]
    fn detach_releases_that_connections_retained_claims_only() {
        let mut claims = ServerInputClaims::default();
        let active = action_request(1, 9);
        let retained = action_request(2, 9);
        let elsewhere = action_request(3, 11);
        for request in [&active, &retained, &elsewhere] {
            assert_eq!(claims.admit(request.clone()), ServerClaimAdmission::New);
        }
        assert!(claims.retain(retained.uid, "test retention"));
        assert!(claims.retain(elsewhere.uid, "test retention"));

        assert_eq!(
            claims.release_connection_retained(9),
            vec![retained.uid],
            "exactly the detached connection's retained claim"
        );
        assert_eq!(
            claims.admit(active.clone()),
            ServerClaimAdmission::Duplicate,
            "the in-flight handler on that connection keeps its claim"
        );
        assert_eq!(
            claims.admit(elsewhere.clone()),
            ServerClaimAdmission::Rearm,
            "another connection's retention is untouched and still re-armable"
        );
        assert_eq!(
            claims.admit(retained.clone()),
            ServerClaimAdmission::New,
            "the released uid is free again, custody accounting and all"
        );

        // A connection holding nothing retained loses nothing to a detach.
        assert!(claims.release_connection_retained(9).is_empty());
        assert_eq!(claims.entries.len(), 3);
    }

    /// Retention names a claim the table holds. A caller that retains a uid
    /// already disposed of gets a `false` it can log, and invents no entry —
    /// custody accounting stays exact.
    #[test]
    fn retaining_a_disposed_claim_creates_nothing() {
        let mut claims = ServerInputClaims::default();
        let action = action_request(4, 9);
        assert_eq!(claims.admit(action.clone()), ServerClaimAdmission::New);
        assert_eq!(claims.dispose(action.uid), Some(action.clone()));
        assert!(!claims.retain(action.uid, "test retention"));
        assert_eq!(claims.capacity_snapshot(), (0, 0));
    }
}

#[cfg(test)]
mod writer_tests {
    //! Deterministic proofs of the two M18 server->gateway teardown paths in
    //! [`writer_loop`] (section B teardown-notify + section C write timeout),
    //! isolated from a live world/gateway. The re-attach *survival* that rides
    //! these teardowns is proven by the gateway `restart_survival` suite (any
    //! boundary EOF — which a teardown produces by dropping the writer's socket
    //! half — heals via the unit restart -> the gateway's reconnect ->
    //! ReAnnounce -> re-attach).

    use std::io;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::Duration;

    use tokio::io::AsyncWrite;

    use super::{
        Arc, BoxWrite, EmissionGate, IPC_MIN_PROTO, IPC_PRE_HELLO_VERSION, IPC_VERSION,
        IPC_VERSION_SKEW_MARKER, LinkVersion, Metrics, Notify, ToGateway, admit_gateway, mpsc,
        writer_loop,
    };

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
            EmissionGate::new(LinkVersion::unknown()),
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
            EmissionGate::new(LinkVersion::unknown()),
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

    /// The emission gate in the real writer (doc 33 §4.1), with the test hook
    /// declaring `SetFloodExempt` a version-13 frame: a 12 link never receives
    /// it while every other frame still flows, and once the shared link cell
    /// records 13 the same writer lets it through. Nothing is gated for real
    /// yet; this is the mechanism the next bump will lean on.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_writer_withholds_a_frame_the_link_predates() {
        use mush_ipc::{FrameReader, write_frame};

        fn flood_exempt_is_new(msg: &ToGateway) -> u32 {
            match msg {
                ToGateway::SetFloodExempt { .. } => 13,
                other => other.introduced_at(),
            }
        }
        let (a, b) = tokio::io::duplex(64 * 1024);
        let (tx, rx) = mpsc::channel::<ToGateway>(8);
        let metrics = Arc::new(Metrics::new());
        let teardown = Arc::new(Notify::new());
        let link = LinkVersion::announced(12);
        let w: BoxWrite = Box::new(a);
        let join = tokio::spawn(writer_loop(
            w,
            rx,
            EmissionGate::with_introduction_table(link.clone(), flood_exempt_is_new),
            vec![],
            vec![],
            Arc::clone(&metrics),
            Arc::clone(&teardown),
            Duration::from_secs(30),
        ));
        let mut frames = FrameReader::new(b);
        let gated = ToGateway::SetFloodExempt {
            char: 7,
            exempt: true,
        };
        let marker = ToGateway::Detach {
            conn: 7,
            cause: mush_ipc::DetachCause::Booted,
        };
        tx.send(gated.clone()).await.expect("queue the gated frame");
        tx.send(marker.clone())
            .await
            .expect("queue the marker frame");
        // Everything up to the marker, skipping the health ticks: the gated
        // frame must not be among it.
        let mut seen = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let frame = frames
                    .read::<ToGateway>()
                    .await
                    .expect("read ok")
                    .expect("a frame");
                let done = frame == marker;
                seen.push(frame);
                if done {
                    break;
                }
            }
        })
        .await
        .expect("the marker frame arrives");
        assert!(
            matches!(seen.first(), Some(ToGateway::Hello { .. })),
            "the greeting is first: {seen:?}"
        );
        assert!(
            !seen.contains(&gated),
            "a 12 link must not receive a 13 frame: {seen:?}"
        );
        // The link announces 13 (the shared cell, as the reader would record
        // it) and the same writer now passes the frame.
        link.record(13);
        tx.send(gated.clone()).await.expect("queue the frame again");
        let passed = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let frame = frames
                    .read::<ToGateway>()
                    .await
                    .expect("read ok")
                    .expect("a frame");
                if frame == gated {
                    break;
                }
                assert!(
                    matches!(frame, ToGateway::WorldHealth { .. }),
                    "only health ticks may precede it: {frame:?}"
                );
            }
        })
        .await;
        assert!(passed.is_ok(), "a 13 link receives the 13 frame");
        drop(tx);
        // A clean close: the writer says Bye and returns.
        let mut w_closed = false;
        while let Ok(Some(frame)) = frames.read::<ToGateway>().await {
            if frame == ToGateway::Bye {
                w_closed = true;
            }
        }
        assert!(w_closed, "the writer closes with Bye once the channel ends");
        tokio::time::timeout(Duration::from_secs(5), join)
            .await
            .expect("writer returns")
            .expect("writer join");
        // Silence the unused-import lint for `write_frame` under every cfg.
        let _ = write_frame::<tokio::io::DuplexStream, ToGateway>;
    }

    /// The link decision (doc 33 §4.1): a Hello inside this server's window is
    /// recorded, a first frame that is not a Hello is a pre-window peer and is
    /// recorded as such while that version is served, and a Hello outside the
    /// window is refused with the `IPC-VERSION-SKEW` marker in the error and
    /// the link left unannounced.
    #[test]
    fn admit_gateway_pins_the_window_and_the_marker() {
        let link = LinkVersion::unknown();
        admit_gateway(&link, IPC_VERSION, Some(IPC_MIN_PROTO)).expect("our own version");
        assert_eq!(link.get(), Some(IPC_VERSION));

        let link = LinkVersion::unknown();
        admit_gateway(&link, IPC_MIN_PROTO, Some(IPC_MIN_PROTO - 1))
            .expect("one step back, inside the window");
        assert_eq!(link.get(), Some(IPC_MIN_PROTO));

        let link = LinkVersion::unknown();
        admit_gateway(&link, IPC_PRE_HELLO_VERSION, None).expect("the bootstrap rule");
        assert_eq!(link.get(), Some(IPC_PRE_HELLO_VERSION));

        for outside in [IPC_VERSION + 1, IPC_MIN_PROTO - 1] {
            let link = LinkVersion::unknown();
            let err = admit_gateway(&link, outside, Some(outside - 1))
                .expect_err("outside the window is refused");
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
            assert!(
                err.to_string().contains(IPC_VERSION_SKEW_MARKER),
                "the marker names the refusal: {err}"
            );
            assert_eq!(link.get(), None, "a refused link is never announced");
        }
    }
}

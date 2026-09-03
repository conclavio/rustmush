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

use std::sync::Arc;

use std::time::Duration;

use mush_config::accounts::import_accounts_json;
use mush_config::config::{Config, StoreBackend};
use mush_config::ipguard::IpGuard;
use mush_config::metrics::Metrics;
use mush_core::{ControlMsg, Engine, Inbound};
use mush_ipc::{
    BoxWrite, FrameReader, IPC_VERSION, IpcEndpoint, ServiceReq, ServiceResp, ToGateway, ToServer,
};
use mush_llm::LlmService;
use mush_store::{InMemoryStore, ObjectStore, RedisStore, core_migrations, run_migrations};
use tokio::sync::Notify;
use tokio::sync::mpsc;
use tokio::sync::oneshot;

pub mod accounts;
pub mod config_ext;
pub mod gateway_sink;
pub mod help;
pub mod modules;
pub mod persist;
pub mod retained;
pub mod staff;
pub mod world_task;

use crate::gateway_sink::SocketSink;
use crate::persist::load_or_seed_world;
use crate::staff::{StaffQueue, register_server_commands_with_cap};
use crate::world_task::{ServerDeps, WorldHandle};

pub use gateway_sink::{GatewaySink, SocketSink as ServerSocketSink};

/// A human-readable server identity string, sent in [`ToGateway::Hello`].
fn server_id() -> String {
    format!("mush-server {}", env!("CARGO_PKG_VERSION"))
}

/// Open the configured store backend (fail fast when Redis is unreachable).
async fn open_store(config: &Config) -> Result<Arc<dyn ObjectStore>, String> {
    match config.store_backend().map_err(|e| e.to_string())? {
        StoreBackend::Redis => {
            let url = &config.store.redis_url;
            tracing::info!(%url, "connecting to Redis");
            let store = RedisStore::connect(url)
                .await
                .map_err(|e| format!("cannot reach Redis at {url}: {e}"))?;
            Ok(Arc::new(store))
        }
        StoreBackend::Memory => {
            tracing::warn!("[store].store = \"memory\": nothing will survive a restart");
            Ok(Arc::new(InMemoryStore::new()))
        }
    }
}

/// Boot the full engine (world load/seed + modules + rulesets), exactly the
/// roster the all-in-one binary builds. Returns the engine plus the
/// [`StaffQueue`] its `@siteban`/`@boot`/... handlers push into — the SAME
/// queue the world loop must drain (`ServerDeps::staff`), matching `main.rs`.
async fn build_engine(
    config: &Config,
    store: &Arc<dyn ObjectStore>,
) -> Result<(Engine, Arc<StaffQueue>), String> {
    let version = run_migrations(&**store, &core_migrations())
        .await
        .map_err(|e| format!("store migrations failed: {e}"))?;
    tracing::info!(schema = version, "store schema current");
    let imported = import_accounts_json(&**store, &config.server.data_dir).await?;
    if imported > 0 {
        tracing::info!(imported, "M1 accounts.json imported");
    }
    let mut world = load_or_seed_world(&**store).await?;
    let clock = config.game.game_clock().map_err(|e| e.to_string())?;
    world.set_game_clock(clock);

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
    let mut mods = modules::server_modules_with(crate::config_ext::weather_config(&config.weather));
    mods.push(modules::worldgen_module_with(
        Arc::clone(&llm),
        crate::config_ext::worldgen_config(&config.worldgen),
    ));
    let npc_grounding = modules::npc_agent_adapter(&config.rulesets.enabled);
    mods.push(modules::npc_module(npc_grounding));
    let rulesets = modules::ruleset_modules(
        &config.rulesets.enabled,
        &crate::config_ext::onboarding_ruleset(&config.onboarding),
    )
    .map_err(|e| format!("ruleset selection: {e}"))?;
    mods.extend(rulesets);
    let mut engine = Engine::with_modules(world, registry, mods)
        .map_err(|e| format!("module registration: {e}"))?;
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
pub async fn serve(config: Config, bound: Option<oneshot::Sender<IpcEndpoint>>) -> Result<(), String> {
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
    let endpoint = config.ipc_endpoint().map_err(|e| e.to_string())?;
    let store = match store {
        Some(s) => s,
        None => open_store(&config).await?,
    };
    let (engine, staff) = build_engine(&config, &store).await?;

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
    // E0 quarantine spool: a separate failure domain from the store, under the
    // configured data dir. A confirmed poison seal is fsynced here before being
    // dropped from re-emit, so it is preserved for staff repair.
    deps.quarantine_dir = Some(config.server.data_dir.join("quarantine"));

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
    let handles = world_task::spawn_persist_with_gateway(engine, deps, move |_weak_inbound| {
        // The forked server never polices (it owns no connection buffers), so it
        // ignores the loop's inbound weak sender and just serializes to the boundary.
        Arc::new(sink)
    });
    let world = handles.handle;
    let world_join = handles.world_join;

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
    let writer_join = tokio::spawn(writer_loop(
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
    let reader_result = reader_loop(reader_half, world.clone(), reply_sink).await;
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
    let _ = writer_join.await;
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
    let _ = tokio::time::timeout(write_timeout, mush_ipc::write_frame(&mut w, &ToGateway::Bye)).await;
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
async fn reader_loop(
    r: mush_ipc::BoxRead,
    world: WorldHandle,
    reply_sink: SocketSink,
) -> std::io::Result<()> {
    let mut reader = FrameReader::new(r);
    while let Some(msg) = reader.read::<ToServer>().await? {
        match msg {
            ToServer::Inbound(wire) => {
                world.inbound(Inbound::from(wire)).await;
            }
            ToServer::Control(ctrl) => {
                world.inbound(Inbound::Control(ctrl)).await;
            }
            ToServer::Request { id, req } => {
                let world = world.clone();
                // The `Reply` is must-deliver: `SocketSink::send` tries the
                // bounded `to_gw` and, on a full channel, tears the boundary
                // down rather than dropping it (a dropped Reply would hang the
                // caller's oneshot — I1).
                let sink = reply_sink.clone();
                tokio::spawn(async move {
                    let resp = handle_request(&world, req).await;
                    sink.send(ToGateway::Reply { id, resp });
                });
            }
            // M16c re-attach: the gateway replays its full held-online roster
            // after (re)connecting to a freshly booted server. Rebuild each
            // connection's world-side state as a SILENT ConnUp and push each
            // session a fresh `look` (D2) — the reloaded world (objects +
            // durable module state from Redis) is intact; only the RAM-only
            // connection map needs restoring.
            ToServer::ReAnnounce(recs) => {
                let recs = recs
                    .into_iter()
                    .map(|r| world_task::ReAttachRec {
                        conn: r.conn,
                        actor: r.actor,
                        caps_structured: r.caps_structured,
                    })
                    .collect();
                world.reattach(recs).await;
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
            ServiceResp::CreatePlayer(world.create_player(&name).await.map_err(Into::into))
        }
        ServiceReq::NameFree { name } => ServiceResp::NameFree(world.name_free(&name).await),
        ServiceReq::InInputContext { actor } => {
            ServiceResp::InInputContext(world.in_input_context(actor).await)
        }
        ServiceReq::Barrier => {
            world.barrier().await;
            ServiceResp::Barrier
        }
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
        fn poll_write(self: Pin<&mut Self>, _: &mut Context<'_>, _: &[u8]) -> Poll<io::Result<usize>> {
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

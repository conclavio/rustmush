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

use mush_config::accounts::import_accounts_json;
use mush_config::config::{Config, StoreBackend};
use mush_config::ipguard::IpGuard;
use mush_core::{ControlMsg, Engine, Inbound};
use mush_ipc::{
    BoxWrite, FrameReader, IPC_VERSION, IpcEndpoint, ServiceReq, ServiceResp, ToGateway, ToServer,
};
use mush_llm::LlmService;
use mush_store::{InMemoryStore, ObjectStore, RedisStore, core_migrations, run_migrations};
use tokio::sync::mpsc;
use tokio::sync::oneshot;

pub mod accounts;
pub mod config_ext;
pub mod gateway_sink;
pub mod help;
pub mod modules;
pub mod persist;
pub mod staff;
pub mod world_task;

use crate::gateway_sink::SocketSink;
use crate::persist::load_or_seed_world;
use crate::staff::{StaffQueue, register_server_commands};
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
    register_server_commands(&mut registry, &staff)
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
    // writer task. Unbounded so the world loop's `transmit` never awaits (D4);
    // real backpressure is the socket's, applied in the writer. The forked
    // server holds no `SessionManager` — the gateway process owns it — so the
    // world loop's session control (`@boot`, `@newpassword`) goes out as
    // fire-and-forget `ToGateway` frames through the [`SocketSink`].
    let (to_gw_tx, to_gw_rx) = mpsc::unbounded_channel::<ToGateway>();

    let guard = Arc::new(IpGuard::new());
    let mut deps = ServerDeps::in_memory();
    deps.store = Arc::clone(&store);
    deps.guard = Arc::clone(&guard);
    // The world loop must drain the SAME staff queue the registry handlers push
    // into (matching `mushd::main`), or `@boot`/`@siteban`/... would be enqueued
    // to an orphan queue and never execute.
    deps.staff = staff;

    let socket_tx = to_gw_tx.clone();
    let handles = world_task::spawn_persist_with_gateway(engine, deps, move |_weak_inbound| {
        // The forked server never polices (it owns no connection buffers), so it
        // ignores the loop's inbound weak sender and just serializes to the boundary.
        Arc::new(SocketSink::new(socket_tx))
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
    // handlers) is dropped.
    let writer_join = tokio::spawn(writer_loop(writer_half, to_gw_rx, features, panels));

    // Reader: pump the gateway's `ToServer` traffic into the world. Runs until
    // the gateway closes the boundary (clean EOF) or a protocol fault.
    let reader_result = reader_loop(reader_half, world.clone(), to_gw_tx.clone()).await;
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
async fn writer_loop(
    mut w: BoxWrite,
    mut rx: mpsc::UnboundedReceiver<ToGateway>,
    features: Vec<String>,
    panels: Vec<String>,
) {
    if let Err(e) = mush_ipc::write_frame(
        &mut w,
        &ToGateway::Hello {
            proto: IPC_VERSION,
            server: server_id(),
            features,
            panels,
        },
    )
    .await
    {
        tracing::warn!(error = %e, "failed to send Hello; boundary down");
        return;
    }
    while let Some(msg) = rx.recv().await {
        if let Err(e) = mush_ipc::write_frame(&mut w, &msg).await {
            tracing::warn!(error = %e, "ipc write failed; boundary down");
            return;
        }
    }
    let _ = mush_ipc::write_frame(&mut w, &ToGateway::Bye).await;
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
    to_gw_tx: mpsc::UnboundedSender<ToGateway>,
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
                let tx = to_gw_tx.clone();
                tokio::spawn(async move {
                    let resp = handle_request(&world, req).await;
                    let _ = tx.send(ToGateway::Reply { id, resp });
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

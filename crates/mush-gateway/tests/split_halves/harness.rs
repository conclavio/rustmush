//! The split rig this binary's modules share (doc 33 GG.1): a stand-in for the
//! server UNIT.
//!
//! Until GG.1 the gateway supervised the server — spawned it, respawned it,
//! gave up on it — and every module here handed the gateway a `ServerLauncher`
//! and drove restarts through its `RestartTrigger`. The server is its own
//! systemd unit now: the gateway only DIALS it, at the one endpoint in
//! `[ipc].listen`, and holds every connection while the unit is away. So the
//! rig is the unit: [`ServerUnit`] boots real `mush-server` halves on their
//! own thread + runtime (the module roster is not `Send`, so the boot future
//! cannot be `tokio::spawn`ed — the real binary `block_on`s it the same way),
//! each at the SAME IPC endpoint — the first boot binds `127.0.0.1:0` and the
//! unit remembers the port — over one shared store, so the world survives
//! every restart exactly as it would on Redis.
//!
//! [`stop`](ServerUnit::stop) makes the running server exit on its own (its
//! runtime drops with every socket; the gateway sees EOF and enters its gap),
//! which is a crash, a `@shutdown`, or `systemctl stop`, and
//! [`start`](ServerUnit::start) is the unit bringing it back. Dropping the
//! handle stops nothing: a module that never restarts its server may let the
//! unit go and the server runs on until the test process ends. A gateway is
//! pointed at the unit with [`aim`](ServerUnit::aim) and started after the
//! unit's first boot (the units say `After=`), or before it, for the proof
//! that a boot with no server is a gap from the start.
//!
//! [`GatewayUnit`] (doc 33 GG.2) is the same shape for the other half: a real
//! `mush-gateway` on its own thread + runtime, so a module can run two of them
//! against one server unit and stop one the way a crash or `systemctl stop`
//! does.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mush_config::config::Config;
use mush_config::generation::Generation;
use mush_drain::lifecycle::{DrainConfig, Trigger};
use mush_ipc::IpcEndpoint;
use mush_store::ObjectStore;
use tokio::sync::{mpsc, oneshot};

/// A boot that has not bound inside this is a failed boot, not a slow one.
const BIND_TIMEOUT: Duration = Duration::from_secs(60);
/// A server asked to stop that has not exited inside this has hung.
const EXIT_TIMEOUT: Duration = Duration::from_secs(60);

/// One running server half.
struct Running {
    /// Fire to make the server exit on its own: the thread's `select!` ends,
    /// the runtime drops, every socket closes.
    crash: oneshot::Sender<()>,
    /// Resolves once the thread has dropped its runtime.
    exited: oneshot::Receiver<()>,
}

/// The stand-in for the server unit. See the module docs.
pub struct ServerUnit {
    store: Arc<dyn ObjectStore>,
    make_config: Box<dyn Fn() -> Config + Send + Sync>,
    /// The endpoint every boot binds. `None` until the first boot (or until
    /// [`at`](ServerUnit::at) fixes one before any boot).
    endpoint: Mutex<Option<IpcEndpoint>>,
    running: Mutex<Option<Running>>,
    boots: AtomicUsize,
}

impl ServerUnit {
    /// A unit over `store`, whose boots read their config from `make_config`
    /// (called once per boot: a fresh scratch `data_dir` each time is the
    /// usual shape). The config's `[ipc].listen` is used by the first boot
    /// only; every boot after it binds whatever the first one bound.
    pub fn new(
        store: Arc<dyn ObjectStore>,
        make_config: impl Fn() -> Config + Send + Sync + 'static,
    ) -> ServerUnit {
        ServerUnit {
            store,
            make_config: Box::new(make_config),
            endpoint: Mutex::new(None),
            running: Mutex::new(None),
            boots: AtomicUsize::new(0),
        }
    }

    /// Fix the endpoint before any boot, so a gateway can be aimed at a unit
    /// that has not started yet (the boot-into-the-gap proof).
    pub fn at(self, endpoint: IpcEndpoint) -> ServerUnit {
        *lock(&self.endpoint) = Some(endpoint);
        self
    }

    /// A loopback endpoint nothing is listening on: bound once to learn a free
    /// port, then released. For [`at`](ServerUnit::at).
    pub fn reserve_endpoint() -> IpcEndpoint {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve a port");
        let addr = listener.local_addr().expect("the reserved address");
        drop(listener);
        IpcEndpoint::Tcp(addr)
    }

    /// The endpoint the unit binds. Panics before the first boot unless
    /// [`at`](ServerUnit::at) fixed one.
    pub fn endpoint(&self) -> IpcEndpoint {
        lock(&self.endpoint)
            .clone()
            .expect("the unit has an endpoint once it has booted (or was fixed with `at`)")
    }

    /// Point a gateway config at this unit: the gateway dials `[ipc].listen`.
    pub fn aim(&self, cfg: &mut Config) {
        cfg.ipc.listen = self.endpoint().describe();
    }

    /// How many servers this unit has booted.
    pub fn boots(&self) -> usize {
        self.boots.load(Ordering::SeqCst)
    }

    /// Boot a server at the unit's endpoint and wait until it has bound. A
    /// server already running is stopped first (a unit runs one process).
    pub async fn start(&self) -> IpcEndpoint {
        if lock(&self.running).is_some() {
            self.stop().await;
        }
        let mut cfg = (self.make_config)();
        if let Some(endpoint) = lock(&self.endpoint).clone() {
            cfg.ipc.listen = endpoint.describe();
        }
        let store = Arc::clone(&self.store);
        let (ipc_tx, ipc_rx) = oneshot::channel();
        let (crash_tx, crash_rx) = oneshot::channel::<()>();
        let (exit_tx, exit_rx) = oneshot::channel();
        self.boots.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            rt.block_on(async move {
                tokio::select! {
                    _ = mush_server::serve_with_store(cfg, Some(ipc_tx), Some(store)) => {}
                    // The forced exit (a crash, a stop): return from `block_on`
                    // so the runtime and its IPC sockets drop and the gateway
                    // sees a boundary EOF the gateway never initiated. Only an
                    // explicit `stop` fires it; a dropped handle (`Err`) leaves
                    // the server running.
                    () = async {
                        if crash_rx.await.is_err() {
                            std::future::pending::<()>().await;
                        }
                    } => {}
                }
            });
            // The listener and every connection are gone only once the
            // runtime is, so `exited` means the port is free to rebind.
            drop(rt);
            let _ = exit_tx.send(());
        });
        let endpoint = tokio::time::timeout(BIND_TIMEOUT, ipc_rx)
            .await
            .expect("the server bound its IPC endpoint in time")
            .expect("the server dropped its ipc oneshot (a failed boot)");
        *lock(&self.endpoint) = Some(endpoint.clone());
        *lock(&self.running) = Some(Running {
            crash: crash_tx,
            exited: exit_rx,
        });
        endpoint
    }

    /// Make the running server exit on its own and wait until it has. A unit
    /// with no running server (one that already exited on its own, say) is
    /// a no-op.
    pub async fn stop(&self) {
        let Some(running) = lock(&self.running).take() else {
            return;
        };
        let _ = running.crash.send(());
        let _ = tokio::time::timeout(EXIT_TIMEOUT, running.exited)
            .await
            .expect("the stopped server exited in time");
    }

    /// Wait for the running server to exit ON ITS OWN (a faulted world, an
    /// in-game `@shutdown`), without asking it to.
    pub async fn wait_exit(&self) {
        let Some(running) = lock(&self.running).take() else {
            return;
        };
        let Running { crash, exited } = running;
        let _ = tokio::time::timeout(EXIT_TIMEOUT, exited)
            .await
            .expect("the server exited on its own in time");
        drop(crash);
    }

    /// `stop` then `start`: `systemctl restart`.
    pub async fn restart(&self) -> IpcEndpoint {
        self.stop().await;
        self.start().await
    }
}

/// How a story starts a gateway beyond [`GatewayUnit::start`] (doc 33 GG.4):
/// its generation; where its drain marker, Discord lock and follow handover
/// live (the unit's `StateDirectory`; `None` is the data dir); how fast its
/// drain runs (`None` is the doorway's 24 h); whether it serves the web; and
/// whether its trusted ingress may serve from a directory that is not setgid
/// (the rig's scratch dir — production never passes it).
#[derive(Default)]
pub struct GatewaySpec {
    pub generation: Option<Generation>,
    pub state_dir: Option<PathBuf>,
    pub drain: Option<DrainConfig>,
    pub web: bool,
    pub trusted_allow_own_group: bool,
}

/// One running gateway half.
struct RunningGateway {
    /// Fire to make the gateway exit at once: the thread's `select!` ends,
    /// the runtime drops, every socket closes.
    crash: oneshot::Sender<()>,
    /// Resolves with `serve`'s result once the thread has dropped its runtime.
    exited: oneshot::Receiver<Result<(), String>>,
}

/// One running gateway half, as a thread with its own runtime: a stand-in for
/// `<inst>mush-gateway.service` (doc 33 GG.2), so a module can run TWO of them
/// against one [`ServerUnit`] — the draining gateway and its successor during a
/// swap, or a crashed gateway and the one systemd brings back.
///
/// [`stop`](GatewayUnit::stop) makes the gateway exit the way a crash or
/// `systemctl stop` does: the runtime drops with every socket, the server sees
/// the link's EOF and detaches that link's connections only. Dropping the
/// handle stops nothing, as with the server unit.
///
/// Doc 33 GG.4: a gateway started with a generation ([`GatewayUnit::start_with`])
/// also takes the swap script's signals in-process — [`drain`](GatewayUnit::drain)
/// is `SIGUSR1`, [`finish`](GatewayUnit::finish) is `SIGTERM` — and
/// [`wait_exit`](GatewayUnit::wait_exit) is the generation ending on its own
/// (empty, or at its deadline) with `serve`'s result.
// The generation half of the API is used by the `cfg(unix)` stories only.
#[cfg_attr(not(unix), allow(dead_code))]
pub struct GatewayUnit {
    ssh: SocketAddr,
    web: Option<SocketAddr>,
    trigger: mpsc::UnboundedSender<Trigger>,
    running: Mutex<Option<RunningGateway>>,
}

#[cfg_attr(not(unix), allow(dead_code))]
impl GatewayUnit {
    /// Start a gateway over `cfg` (already [aimed](ServerUnit::aim) at its
    /// unit) and `store`, of `generation` (`None` is what the binary gets with
    /// no `--generation`), and wait until its SSH listener is bound.
    pub async fn start(
        cfg: Config,
        store: Arc<dyn ObjectStore>,
        generation: Option<Generation>,
    ) -> GatewayUnit {
        GatewayUnit::start_with(
            cfg,
            store,
            GatewaySpec {
                generation,
                ..GatewaySpec::default()
            },
        )
        .await
    }

    /// [`start`](GatewayUnit::start) with the full [`GatewaySpec`]; a gateway
    /// that ends before binding is a failed start.
    pub async fn start_with(
        cfg: Config,
        store: Arc<dyn ObjectStore>,
        spec: GatewaySpec,
    ) -> GatewayUnit {
        match GatewayUnit::try_start_with(cfg, store, spec).await {
            Ok(unit) => unit,
            Err(ended) => panic!("the gateway ended before it bound its SSH listener: {ended:?}"),
        }
    }

    /// [`start_with`](GatewayUnit::start_with), where a gateway that ends
    /// before it binds is `Err` with `serve`'s result: a generation that
    /// found its drain marker returns `Ok(())` at once (doc 33 §4.5), a
    /// failed boot an `Err`.
    pub async fn try_start_with(
        cfg: Config,
        store: Arc<dyn ObjectStore>,
        spec: GatewaySpec,
    ) -> Result<GatewayUnit, Result<(), String>> {
        let (ssh_tx, ssh_rx) = oneshot::channel();
        let (web_tx, web_rx) = if spec.web {
            let (tx, rx) = oneshot::channel();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        let (crash_tx, crash_rx) = oneshot::channel::<()>();
        let (exit_tx, exit_rx) = oneshot::channel();
        let (trigger_tx, trigger_rx) = mpsc::unbounded_channel();
        std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            else {
                let _ = exit_tx.send(Err("the gateway's runtime could not be built".to_string()));
                return;
            };
            let result = rt.block_on(async move {
                tokio::select! {
                    result = mush_gateway::serve(
                        cfg,
                        mush_gateway::GatewayOptions {
                            store: Some(store),
                            ssh_ready: Some(ssh_tx),
                            web_ready: web_tx,
                            siteban_refresh: None,
                            give_up_after: None,
                            generation: spec.generation,
                            state_dir: spec.state_dir,
                            drain: spec.drain,
                            drain_trigger: Some(trigger_rx),
                            trusted_allow_own_group: spec.trusted_allow_own_group,
                        },
                    ) => result,
                    () = async {
                        if crash_rx.await.is_err() {
                            std::future::pending::<()>().await;
                        }
                    } => Err("stopped by the rig".to_string()),
                }
            });
            drop(rt);
            let _ = exit_tx.send(result);
        });
        let mut exited = exit_rx;
        let ssh = match tokio::time::timeout(BIND_TIMEOUT, ssh_rx)
            .await
            .expect("the gateway bound its SSH listener in time")
        {
            Ok(addr) => addr,
            Err(_dropped) => {
                let ended = tokio::time::timeout(EXIT_TIMEOUT, &mut exited)
                    .await
                    .expect("a gateway that did not bind exited in time")
                    .unwrap_or_else(|_| {
                        Err("the gateway's thread ended without a result".to_string())
                    });
                return Err(ended);
            }
        };
        let web = match web_rx {
            Some(rx) => Some(
                tokio::time::timeout(BIND_TIMEOUT, rx)
                    .await
                    .expect("the gateway bound its web listener in time")
                    .expect("the gateway dropped its web oneshot (a failed web bind)"),
            ),
            None => None,
        };
        Ok(GatewayUnit {
            ssh,
            web,
            trigger: trigger_tx,
            running: Mutex::new(Some(RunningGateway {
                crash: crash_tx,
                exited,
            })),
        })
    }

    /// Where this gateway's SSH listener is bound.
    pub fn ssh_addr(&self) -> SocketAddr {
        self.ssh
    }

    /// Where this gateway's web listener is bound (started with
    /// `GatewaySpec::web`).
    pub fn web_addr(&self) -> SocketAddr {
        self.web.expect("a gateway started with the web listener")
    }

    /// The swap script's `SIGUSR1`: drain (doc 33 §4.5).
    pub fn drain(&self) {
        let _ = self.trigger.send(Trigger::Drain);
    }

    /// `SIGTERM`: finish (doc 33 §4.5).
    pub fn finish(&self) {
        let _ = self.trigger.send(Trigger::Finish);
    }

    /// Wait, at most `within`, for the gateway to end on its own (a drained or
    /// finished generation that is empty or at its deadline), and return
    /// `serve`'s result: `Ok(())` is the exit 0 the unit sees.
    pub async fn wait_exit(&self, within: Duration) -> Result<(), String> {
        let Some(RunningGateway { crash, exited }) = lock(&self.running).take() else {
            return Err("the gateway was already stopped".to_string());
        };
        let result = tokio::time::timeout(within, exited)
            .await
            .expect("the gateway exited on its own in time")
            .unwrap_or_else(|_| Err("the gateway's thread ended without a result".to_string()));
        drop(crash);
        result
    }

    /// Make the gateway exit (every socket closes, the server sees this
    /// link's EOF) and wait until it has.
    pub async fn stop(&self) {
        let Some(running) = lock(&self.running).take() else {
            return;
        };
        let _ = running.crash.send(());
        let _ = tokio::time::timeout(EXIT_TIMEOUT, running.exited)
            .await
            .expect("the stopped gateway exited in time");
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

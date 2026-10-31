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

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mush_config::config::Config;
use mush_ipc::IpcEndpoint;
use mush_store::ObjectStore;
use tokio::sync::oneshot;

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

/// One running gateway half, as a thread with its own runtime: a stand-in for
/// `<inst>mush-gateway.service` (doc 33 GG.2), so a module can run TWO of them
/// against one [`ServerUnit`] — the draining gateway and its successor during a
/// swap, or a crashed gateway and the one systemd brings back.
///
/// [`stop`](GatewayUnit::stop) makes the gateway exit the way a crash or
/// `systemctl stop` does: the runtime drops with every socket, the server sees
/// the link's EOF and detaches that link's connections only. Dropping the
/// handle stops nothing, as with the server unit.
pub struct GatewayUnit {
    ssh: std::net::SocketAddr,
    running: Mutex<Option<Running>>,
}

impl GatewayUnit {
    /// Start a gateway over `cfg` (already [aimed](ServerUnit::aim) at its
    /// unit) and `store`, of `generation` (`None` is what the binary gets with
    /// no `--generation`), and wait until its SSH listener is bound.
    pub async fn start(
        cfg: Config,
        store: Arc<dyn ObjectStore>,
        generation: Option<mush_config::generation::Generation>,
    ) -> GatewayUnit {
        let (ssh_tx, ssh_rx) = oneshot::channel();
        let (crash_tx, crash_rx) = oneshot::channel::<()>();
        let (exit_tx, exit_rx) = oneshot::channel();
        std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            rt.block_on(async move {
                tokio::select! {
                    _ = mush_gateway::serve(
                        cfg,
                        mush_gateway::GatewayOptions {
                            store: Some(store),
                            ssh_ready: Some(ssh_tx),
                            web_ready: None,
                            siteban_refresh: None,
                            give_up_after: None,
                            generation,
                        },
                    ) => {}
                    () = async {
                        if crash_rx.await.is_err() {
                            std::future::pending::<()>().await;
                        }
                    } => {}
                }
            });
            drop(rt);
            let _ = exit_tx.send(());
        });
        let ssh = tokio::time::timeout(BIND_TIMEOUT, ssh_rx)
            .await
            .expect("the gateway bound its SSH listener in time")
            .expect("the gateway dropped its ssh oneshot (a failed boot)");
        GatewayUnit {
            ssh,
            running: Mutex::new(Some(Running {
                crash: crash_tx,
                exited: exit_rx,
            })),
        }
    }

    /// Where this gateway's SSH listener is bound.
    pub fn ssh_addr(&self) -> std::net::SocketAddr {
        self.ssh
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

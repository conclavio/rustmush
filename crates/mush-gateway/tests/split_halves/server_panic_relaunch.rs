//! WS01-S8: a forked server whose world dies stops being a server, and the
//! supervisor heals it.
//!
//! The proof splits in two, because it has to. [`LaunchedServer`] carries only
//! `exited: BoxFuture<()>` and the production `SpawnLauncher` logs the child's
//! status inside its own task, so NO supervisor-level assertion can see an exit
//! code (widening that API is WS22's).
//!
//! - **Process half.** A real `mush-server` in a real child process, with one
//!   world turn armed to panic. The child must EXIT, and exit non-zero, so a
//!   supervisor — or systemd — knows to relaunch it. Its contrast case is an
//!   in-game `@shutdown`, which must exit *zero*: the same mechanism, a
//!   different verdict, and the reason the reader gates its log severity on
//!   shutdown provenance rather than shouting on every closed channel.
//! - **Supervisor half.** The gateway supervising a server whose first boot
//!   faults the same way. The connection is never dropped: the player is told
//!   the server is restarting, is reconnected into the reloaded world, and
//!   keeps playing.
//! - **Skew half (WS22-S16).** The contrast to a crash: a server half built at
//!   another commit, which answers the handshake with a different
//!   `IPC_VERSION`. That is an operator error, not a dying world, so it must
//!   NOT spend the crash-loop budget — before this step five mismatched
//!   handshakes abandoned supervision for good, leaving every held session
//!   alive and permanently input-refused.
//!
//! Before this step the server did neither. `serve` awaited its gateway reader
//! to completion and only then looked at the world's join handle, so a world
//! that died under a live, well-behaved gateway left the process up with its
//! listener bound, accepting frames and dropping every one of them — for ever,
//! with nothing for the supervisor to see.
//!
//! [`LaunchedServer`]: mush_gateway::LaunchedServer

use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use mush_config::accounts::hash_password;
use mush_config::config::Config;
use mush_core::GOD;
use mush_gateway::{
    GatewayOptions, IPC_VERSION_SKEW_MARKER, LaunchedServer, RestartTrigger, ServerLauncher,
    ServerSource,
};
use mush_ipc::{
    FrameReader, GatewayInput, GatewayRequest, GatewayRequestDisposition, IPC_VERSION, IpcEndpoint,
    ToGateway, ToServer, WireInbound, write_frame,
};
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use russh::ChannelMsg;
use tokio::sync::oneshot;

const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const GM_PASSWORD: &str = "gm-secret-1";

/// The command line whose world turn is armed to panic. It is not a registered
/// command: the arming sits at the world task's dispatch boundary, ahead of
/// `engine.step`, so what the line *means* never matters.
const FAULT_LINE: &str = "@ws01s8-fault";

/// Set by the parent on the child it re-execs as a server, and by nothing else.
const WORKER_ENV: &str = "RUSTMUSH_WS01_S8_SERVER_WORKER";
const WORKER_IPC_ENV: &str = "RUSTMUSH_WS01_S8_SERVER_IPC";
const WORKER_DATA_ENV: &str = "RUSTMUSH_WS01_S8_SERVER_DATA";
/// The name libtest filters the worker below by.
const WORKER_TEST: &str = "server_panic_relaunch::ws01_s8_server_worker";

fn scratch_dir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("mush-ws01s8-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn test_config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ssh.listen = "127.0.0.1:0".to_string();
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    cfg
}

// ---------------------------------------------------------------------------
// The server worker: a real `mush-server` in its own process
// ---------------------------------------------------------------------------

/// The child half. Inert unless the parent re-execs this binary with
/// [`WORKER_ENV`] set, so an ordinary `cargo test` run walks straight past it.
///
/// It mirrors `mush-server`'s own `main` exactly where it matters: `serve`'s
/// `Ok` is exit 0 and its `Err` is a failure exit. That mapping is the whole
/// contract this proof leans on, so it is spelled out here rather than
/// inherited from a helper.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ws01_s8_server_worker() {
    if std::env::var_os(WORKER_ENV).is_none() {
        return;
    }
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir =
        PathBuf::from(std::env::var_os(WORKER_DATA_ENV).expect("worker data dir"));
    cfg.ssh.listen = "127.0.0.1:0".to_string();
    cfg.ipc.listen = std::env::var(WORKER_IPC_ENV).expect("worker ipc endpoint");

    let code = match mush_server::serve(cfg, None).await {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("mush-server: {error}");
            1
        }
    };
    std::process::exit(code);
}

/// Reserve a localhost port and hand back the endpoint string. The listener is
/// dropped immediately; the child rebinds it.
fn free_ipc_endpoint() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve a port");
    let addr = listener.local_addr().expect("reserved address");
    drop(listener);
    addr.to_string()
}

struct Worker {
    child: tokio::process::Child,
    marker: PathBuf,
}

/// Spawn this test binary as a real server process. `arm` names the command
/// line whose world turn must panic (`None` leaves the world healthy).
fn spawn_worker(tag: &str, endpoint: &str, arm: Option<&str>) -> Worker {
    let root = scratch_dir(tag);
    let marker = root.join("world-fault-marker");
    let exe = std::env::current_exe().expect("the test binary knows its own path");
    let mut command = tokio::process::Command::new(exe);
    command
        .arg(WORKER_TEST)
        .arg("--exact")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(WORKER_ENV, "1")
        .env(WORKER_IPC_ENV, endpoint)
        .env(WORKER_DATA_ENV, &root)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    if let Some(line) = arm {
        command
            .env("RUSTMUSH_WS01_S8_PANIC_CMD", line)
            .env("RUSTMUSH_WS01_S8_PANIC_MARKER", &marker);
    }
    Worker {
        child: command.spawn().expect("spawn the server worker"),
        marker,
    }
}

/// A minimal gateway: connect the boundary, greet, attach God, and run one
/// command line through the legacy custody handshake.
struct FakeGateway {
    frames: FrameReader<mush_ipc::BoxRead>,
    writer: mush_ipc::BoxWrite,
}

impl FakeGateway {
    /// Connect once the child has bound, then take its `Hello`.
    async fn attach(endpoint: &str) -> FakeGateway {
        let endpoint = IpcEndpoint::Tcp(endpoint.parse::<SocketAddr>().expect("ipc address"));
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        let (reader, mut writer) = loop {
            match mush_ipc::connect(&endpoint).await {
                Ok(pair) => break pair,
                Err(error) => {
                    assert!(
                        tokio::time::Instant::now() < deadline,
                        "the server worker never bound {}: {error}",
                        endpoint.describe()
                    );
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        };
        let mut frames = FrameReader::new(reader);
        let hello = tokio::time::timeout(STEP_TIMEOUT, frames.read::<ToGateway>())
            .await
            .expect("hello within timeout")
            .expect("hello read ok")
            .expect("a Hello frame");
        assert!(
            matches!(hello, ToGateway::Hello { .. }),
            "first frame is Hello"
        );
        write_frame(
            &mut writer,
            &ToServer::Inbound(WireInbound::ConnUp {
                conn: 1,
                actor: GOD,
                caps_structured: false,
                from_linkdead: false,
            }),
        )
        .await
        .expect("attach God");
        FakeGateway { frames, writer }
    }

    /// Submit one command through the legacy claim/release handshake, so the
    /// world really executes it.
    async fn play(&mut self, line: &str) {
        let request = GatewayRequest::new(
            1,
            1,
            GatewayInput::Cmd {
                conn: 1,
                actor: GOD,
                line: line.to_string(),
            },
        );
        write_frame(&mut self.writer, &ToServer::Input(request.clone()))
            .await
            .expect("submit the command");
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        loop {
            let frame = tokio::time::timeout_at(deadline, self.frames.read::<ToGateway>())
                .await
                .expect("a claim answer within timeout")
                .expect("boundary read ok")
                .expect("the boundary did not EOF before answering the claim");
            if let ToGateway::RequestDisposition {
                request: uid,
                disposition,
            } = frame
                && uid == request.uid
            {
                assert_eq!(
                    disposition,
                    GatewayRequestDisposition::LegacyClaimed,
                    "the server takes custody before it will execute"
                );
                break;
            }
        }
        write_frame(
            &mut self.writer,
            &ToServer::GatewayReleased {
                request: request.uid,
            },
        )
        .await
        .expect("release the replay copy");
    }

    /// Keep draining the boundary until it EOFs, so the server's writer is
    /// never the thing that blocks its own exit — and hold the write half open
    /// throughout, so nothing this side does can be mistaken for a gateway
    /// that disconnected.
    fn drain(self) -> tokio::task::JoinHandle<()> {
        let FakeGateway { mut frames, writer } = self;
        tokio::spawn(async move {
            let _writer = writer;
            while matches!(frames.read::<ToGateway>().await, Ok(Some(_))) {}
        })
    }
}

// ---------------------------------------------------------------------------
// Process half
// ---------------------------------------------------------------------------

/// The failure this step closes, at process scope: the world faults under a
/// live, polite gateway that never disconnects. The server must NOT sit there
/// as a bound listener in front of a dead world — it must exit, and exit
/// non-zero, which is the only thing a supervisor (or systemd) can act on.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_faulted_world_exits_the_forked_server_non_zero() {
    let endpoint = free_ipc_endpoint();
    let mut worker = spawn_worker("fault", &endpoint, Some(FAULT_LINE));

    let mut gateway = FakeGateway::attach(&endpoint).await;
    gateway.play(FAULT_LINE).await;
    let drained = gateway.drain();

    let status = tokio::time::timeout(STEP_TIMEOUT, worker.child.wait())
        .await
        .expect(
            "the server process must EXIT once its world faults; before WS01-S8 it stayed up \
             forever with a bound listener and a dead world behind it",
        )
        .expect("waiting on the server worker");
    assert!(
        !status.success(),
        "a faulted world is a failure exit ({status}); a supervisor that sees success has no \
         reason to relaunch"
    );
    assert!(
        worker.marker.exists(),
        "the world turn really was armed and really did panic"
    );
    let _ = drained.await;
}

/// The contrast, and the reason the reader gates its log severity on
/// provenance: an in-game `@shutdown` reaches the SAME mechanism — the world
/// ends under a live gateway and `serve` returns — but it is an ordinary exit,
/// so the process exits zero and nothing is a fault.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_in_game_shutdown_exits_the_forked_server_cleanly() {
    let endpoint = free_ipc_endpoint();
    let mut worker = spawn_worker("shutdown", &endpoint, None);

    let mut gateway = FakeGateway::attach(&endpoint).await;
    gateway.play("@shutdown").await;
    let drained = gateway.drain();

    let status = tokio::time::timeout(STEP_TIMEOUT, worker.child.wait())
        .await
        .expect("an in-game @shutdown must end the server process")
        .expect("waiting on the server worker");
    assert!(
        status.success(),
        "an operator shutdown is not a fault; exiting {status} would have the supervisor \
         relaunch a server the operator just stopped"
    );
    assert!(
        !worker.marker.exists(),
        "nothing was armed on this run; the world exited through its own front door"
    );
    let _ = drained.await;
}

// ---------------------------------------------------------------------------
// Supervisor half
// ---------------------------------------------------------------------------

/// Boot one real server on its own thread + runtime (as the binary does — the
/// module roster is not `Send`), sharing `store` so the world survives the
/// relaunch. `exited` fires when that server's `serve` returns.
async fn launch_thread_server(store: Arc<dyn ObjectStore>) -> Result<LaunchedServer, String> {
    let cfg = test_config("server");
    let (ipc_tx, ipc_rx) = oneshot::channel();
    let (exit_tx, exit_rx) = oneshot::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let _ = rt.block_on(mush_server::serve_with_store(
            cfg,
            Some(ipc_tx),
            Some(store),
        ));
        let _ = exit_tx.send(());
    });
    let endpoint = tokio::time::timeout(Duration::from_secs(30), ipc_rx)
        .await
        .map_err(|_| "server ipc bind timed out".to_string())?
        .map_err(|_| "server dropped its ipc oneshot".to_string())?;
    Ok(LaunchedServer {
        endpoint,
        exited: Box::pin(async move {
            let _ = exit_rx.await;
        }),
    })
}

/// A launcher whose FIRST boot has one world turn armed to panic; every boot
/// after that is a healthy server. Arming is one-shot and in-process, so the
/// relaunched half — which shares this process's environment — boots clean.
struct FaultOnceLauncher {
    store: Arc<dyn ObjectStore>,
    launches: AtomicUsize,
}

impl ServerLauncher for FaultOnceLauncher {
    fn launch(&self) -> BoxFuture<'static, Result<LaunchedServer, String>> {
        if self.launches.fetch_add(1, Ordering::SeqCst) == 0 {
            mush_server::arm_world_turn_panic(FAULT_LINE);
        }
        let store = Arc::clone(&self.store);
        Box::pin(launch_thread_server(store))
    }
}

/// The gateway's own KV store (accounts), seeded with the God wizard.
async fn seeded_gw_store() -> Arc<dyn ObjectStore> {
    let gw_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    gw_store
        .create_account(AccountRec {
            id: None,
            name: "boss".to_string(),
            argon2_hash: hash_password(GM_PASSWORD).unwrap(),
            created: 0,
            characters: vec![CharacterRef {
                id: GOD,
                name: "God".to_string(),
            }],
            ssh_keys: vec![],
            discord: None,
        })
        .await
        .unwrap();
    gw_store
}

/// Start the gateway (on its own thread/runtime, as the binary does)
/// supervising `launcher`, and return the bound SSH address plus the restart
/// trigger. The trigger is a live handle on the supervisor and must be HELD for
/// the life of the test: dropping the last one is a shutdown signal, and the
/// next connect is refused with "the game is shutting down".
async fn start_split(launcher: Box<dyn ServerLauncher>) -> (SocketAddr, RestartTrigger) {
    let gw_store = seeded_gw_store().await;
    let (ssh_tx, ssh_rx) = oneshot::channel();
    let (restart_tx, restart_rx) = oneshot::channel::<RestartTrigger>();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(mush_gateway::serve(
            test_config("gateway"),
            GatewayOptions {
                source: ServerSource::Restartable(launcher),
                store: Some(gw_store),
                ssh_ready: Some(ssh_tx),
                web_ready: None,
                restart_ready: Some(restart_tx),
                siteban_refresh: None,
            },
        ))
        .unwrap();
    });
    let ssh_addr = tokio::time::timeout(Duration::from_secs(30), ssh_rx)
        .await
        .expect("gateway bound SSH")
        .expect("ssh addr");
    let trigger = tokio::time::timeout(Duration::from_secs(30), restart_rx)
        .await
        .expect("supervisor handed back a restart trigger")
        .expect("restart trigger");
    (ssh_addr, trigger)
}

struct AcceptAll;

impl russh::client::Handler for AcceptAll {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for f in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&f) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

struct Term {
    channel: russh::Channel<russh::client::Msg>,
    seen: String,
}

impl Term {
    fn mark(&self) -> usize {
        self.seen.len()
    }

    async fn expect_after(&mut self, mark: usize, needle: &str) {
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        while !strip_ansi(&self.seen[mark..]).contains(needle) {
            let msg = tokio::time::timeout_at(deadline, self.channel.wait())
                .await
                .unwrap_or_else(|_| {
                    panic!("timeout waiting for {needle:?}; seen: {:?}", self.seen)
                });
            match msg {
                Some(ChannelMsg::Data { data }) => {
                    self.seen.push_str(&String::from_utf8_lossy(&data));
                }
                Some(_) => {}
                None => panic!(
                    "channel closed waiting for {needle:?}; seen: {:?}",
                    self.seen
                ),
            }
        }
    }

    async fn expect(&mut self, needle: &str) {
        self.expect_after(0, needle).await;
    }

    async fn send_line(&mut self, line: &str) {
        let bytes = format!("{line}\r").into_bytes();
        self.channel.data(&bytes[..]).await.expect("send line");
    }
}

async fn play_client(addr: SocketAddr) -> (russh::client::Handle<AcceptAll>, Term) {
    let config = Arc::new(russh::client::Config::default());
    let mut handle = russh::client::connect(config, addr, AcceptAll)
        .await
        .expect("connect");
    let auth = handle.authenticate_none("play").await.expect("auth-none");
    assert!(auth.success(), "play must be accepted with no auth");
    let channel = handle.channel_open_session().await.expect("open channel");
    channel
        .request_pty(true, "xterm-256color", 80, 24, 0, 0, &[])
        .await
        .expect("pty");
    channel.request_shell(true).await.expect("shell");
    (
        handle,
        Term {
            channel,
            seen: String::new(),
        },
    )
}

/// The heal (D2), end to end: a world panics under a supervised server, and
/// the player never notices more than a blip. The server exits on its own, the
/// supervisor takes its crash arm, the held session re-attaches into the
/// reloaded world, and play continues.
///
/// Before this step there was no exit to take an arm on: the faulted server
/// held its boundary open, so the supervisor saw a healthy connection to a
/// world that could never answer again, and the session was dead for good.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_faulted_world_is_relaunched_and_the_session_keeps_playing() {
    // Shared across every (re)boot, so the world survives the relaunch exactly
    // as it would on Redis.
    let server_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let (ssh_addr, _trigger) = start_split(Box::new(FaultOnceLauncher {
        store: Arc::clone(&server_store),
        launches: AtomicUsize::new(0),
    }))
    .await;

    let (_g, mut god) = play_client(ssh_addr).await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God (account boss, 1 connection).")
        .await;
    god.expect("Limbo #0").await;

    // Build something that must survive the fault.
    let before_dig = god.mark();
    god.send_line("@dig Vault = east;e, west;w").await;
    god.expect_after(before_dig, "Vault created with room number")
        .await;

    // Kill the world from inside a command turn — a real caught panic at the
    // dispatch boundary, not a simulated one.
    let fault_mark = god.mark();
    god.send_line(FAULT_LINE).await;

    // The connection survives: restarting -> Reconnected -> a fresh look.
    god.expect_after(fault_mark, "restarting").await;
    god.expect_after(fault_mark, "Reconnected").await;
    god.expect_after(fault_mark, "Limbo #0").await;

    // World intact and play continues on the relaunched half.
    let after_restart = god.mark();
    god.send_line("east").await;
    god.expect_after(after_restart, "Vault").await;

    drop(god);
}

// ---------------------------------------------------------------------------
// Skew half (WS22-S16)
// ---------------------------------------------------------------------------

/// Skewed boots served before the operator "redeploys" the matching build. One
/// more than the gateway's private `MAX_CONSECUTIVE_FAILURES` (5): the count at
/// which the crash-loop guard used to give up and stop supervising.
const SKEWED_BOOTS: usize = 6;

/// A stand-in for a server half built at another commit: it binds the boundary
/// and greets the gateway properly, but announces the wrong [`IPC_VERSION`].
/// Everything else about it is healthy — that is exactly the point, and why
/// counting it as a crash was wrong.
async fn launch_version_skewed_server() -> Result<LaunchedServer, String> {
    let listener = mush_ipc::listen(&IpcEndpoint::parse("127.0.0.1:0")?)
        .await
        .map_err(|e| format!("binding the skewed server: {e}"))?;
    let addr = listener
        .tcp_addr()
        .ok_or_else(|| "the skewed server needs a tcp endpoint".to_string())?;
    let (exit_tx, exit_rx) = oneshot::channel();
    tokio::spawn(async move {
        if let Ok((reader, mut writer)) = listener.accept().await {
            let _ = write_frame(
                &mut writer,
                &ToGateway::Hello {
                    proto: IPC_VERSION + 1,
                    server: "mush-server (built at another commit)".to_string(),
                    features: vec![],
                    panels: vec![],
                },
            )
            .await;
            // Stay up until the gateway hangs up on the mismatch: a skewed
            // server is a live process, not a crashed one, and the supervisor
            // confirms its exit before it launches the next.
            let mut frames = FrameReader::new(reader);
            while matches!(frames.read::<ToServer>().await, Ok(Some(_))) {}
        }
        drop(listener);
        let _ = exit_tx.send(());
    });
    Ok(LaunchedServer {
        endpoint: IpcEndpoint::Tcp(addr),
        exited: Box::pin(async move {
            let _ = exit_rx.await;
        }),
    })
}

/// The botched deploy, then the fix. Boot 0 is the gateway's own startup (a
/// healthy server, so a player can get in and be held); the next
/// [`SKEWED_BOOTS`] answer with the wrong protocol version; the one after that
/// is the operator redeploying the matching build.
struct SkewedThenRedeployedLauncher {
    store: Arc<dyn ObjectStore>,
    launches: Arc<AtomicUsize>,
}

impl ServerLauncher for SkewedThenRedeployedLauncher {
    fn launch(&self) -> BoxFuture<'static, Result<LaunchedServer, String>> {
        let n = self.launches.fetch_add(1, Ordering::SeqCst);
        if (1..=SKEWED_BOOTS).contains(&n) {
            return Box::pin(launch_version_skewed_server());
        }
        let store = Arc::clone(&self.store);
        Box::pin(launch_thread_server(store))
    }
}

/// A `tracing` writer that keeps the log in memory, so the proof can assert on
/// the operator-facing marker itself rather than on a paraphrase of it.
#[derive(Clone)]
struct CapturedLog(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("log buffer poisoned")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLog {
    type Writer = CapturedLog;

    fn make_writer(&'a self) -> CapturedLog {
        self.clone()
    }
}

/// The failure WS22-S16 closes: a deploy that rebuilds the server half against
/// a bumped `IPC_VERSION` while the running gateway still speaks the old one.
/// Every relaunch handshakes and is refused, and the supervisor used to count
/// each refusal as a crash — five of them and it broadcast "the server is down"
/// and RETURNED, so the boundary stayed in its gap for ever: every SSH session
/// still connected, none of them able to type, and `SIGHUP` powerless because
/// the returned supervisor had dropped the restart receiver.
///
/// The mismatch must instead be its own verdict: logged with a fixed marker an
/// operator can grep, told to players in words that name the action, kept OFF
/// the crash-loop budget, and retried for as long as it takes — so redeploying
/// the matching server build heals the game with no gateway restart and no
/// dropped connection.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_version_skewed_server_is_not_a_crash_and_heals_on_redeploy() {
    let log = Arc::new(Mutex::new(Vec::new()));
    assert!(
        tracing_subscriber::fmt()
            .with_writer(CapturedLog(Arc::clone(&log)))
            .with_max_level(tracing::Level::ERROR)
            .with_ansi(false)
            .try_init()
            .is_ok(),
        "this proof owns the process-wide subscriber; nothing else in this test binary \
         may install one"
    );

    let server_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let launches = Arc::new(AtomicUsize::new(0));
    let (ssh_addr, trigger) = start_split(Box::new(SkewedThenRedeployedLauncher {
        store: Arc::clone(&server_store),
        launches: Arc::clone(&launches),
    }))
    .await;

    let (_g, mut god) = play_client(ssh_addr).await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God (account boss, 1 connection).")
        .await;
    god.expect("Limbo #0").await;

    // The deploy: the operator restarts the server half, and every replacement
    // for the next six attempts is the wrong build.
    let before_restart = god.mark();
    assert!(
        trigger.restart().await,
        "the supervisor must accept a deliberate restart"
    );
    god.expect_after(before_restart, "restarting").await;
    // The gap names the operator action instead of "could not be restarted",
    // and promises the session survives it.
    god.expect_after(before_restart, "different protocol versions")
        .await;
    // ... and it does: the seventh launch is the corrected binary, and the held
    // session re-attaches with no gateway restart. Reaching this line at all is
    // the proof the guard never tripped — a tripped guard returns from
    // `supervise`, and no later launch is ever made.
    god.expect_after(before_restart, "Reconnected").await;
    god.expect_after(before_restart, "Limbo #0").await;

    let after_heal = god.mark();
    god.send_line("@dig Vault = east;e, west;w").await;
    god.expect_after(after_heal, "Vault created with room number")
        .await;

    assert_eq!(
        launches.load(Ordering::SeqCst),
        SKEWED_BOOTS + 2,
        "the supervisor must keep relaunching past MAX_CONSECUTIVE_FAILURES skewed \
         handshakes (boot 0 + {SKEWED_BOOTS} skewed + the redeploy); a smaller count means \
         the crash-loop guard tripped on an operator error and abandoned the boundary"
    );
    let captured = String::from_utf8_lossy(&log.lock().expect("log buffer poisoned")).to_string();
    assert!(
        captured.contains(IPC_VERSION_SKEW_MARKER),
        "the mismatch must be logged with the fixed marker an operator greps for; captured: \
         {captured}"
    );

    drop(god);
    drop(trigger);
}

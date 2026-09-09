//! WS22-S17: an abandoned supervisor is fail-stop and OBSERVABLE, not one chat
//! line.
//!
//! Every path that ends supervision — the crash-loop guard tripping, or an old
//! instance whose exit cannot be established — used to do exactly one thing:
//! broadcast "the server is down" to the sessions it was holding. The process
//! then stayed up and stayed healthy. `systemctl status` said `active
//! (running)`, `Restart=on-failure` never fired because nothing had failed,
//! `/healthz` answered 200 with `ok: true`, and every player who was still
//! connected typed into a boundary that refused them for ever. That is the
//! worst state the split can be in, and nothing anywhere said so.
//!
//! So an abandonment now does four things, and this file proves all four
//! against a real gateway supervising a real in-process server:
//!
//! - it latches the `mush_supervision_abandoned` gauge, which `/healthz`
//!   reports as `supervision_ok: false`;
//! - it logs ONE ERROR carrying [`SUPERVISION_ABANDONED_MARKER`], the fixed
//!   text an alert rule greps for;
//! - it still tells the players, exactly once — the notice must not move into
//!   the retry loop;
//! - and, unless an operator turned the key off, it exits non-zero after a
//!   short grace window, so a supervisor restarts the whole gateway: connections
//!   that refuse every input are worth less than a working game.
//!
//! **Why one test function with three phases.** The marker assertion is a
//! COUNT, and `tracing`'s installed subscriber is process-wide: two test
//! functions abandoning supervision concurrently would write into the same
//! buffer and each would see the other's marker. The phases therefore run in
//! sequence inside one function, so "exactly once" means exactly once.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use mush_config::accounts::hash_password;
use mush_config::config::Config;
use mush_core::GOD;
use mush_gateway::{
    GatewayOptions, LaunchedServer, RestartTrigger, SUPERVISION_ABANDONED_MARKER, ServerLauncher,
    ServerSource,
};
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use russh::ChannelMsg;
use serde_json::Value;
use tokio::sync::oneshot;

const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const GM_PASSWORD: &str = "gm-secret-1";
/// The give-up line the gateway broadcasts (`server_down_notice`).
const DOWN_NOTICE: &str = "could not be restarted";
/// Longer than the gateway's own `ABANDONED_EXIT_GRACE` (3 s), with room for a
/// loaded CI box: the window in which the exit must have happened, and after
/// which "still resident" is a fact rather than a race.
const PAST_THE_GRACE: Duration = Duration::from_secs(12);

// ---------------------------------------------------------------------------
// Rig
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("mush-ws22s17-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn server_config() -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir("server");
    cfg.ssh.listen = "127.0.0.1:0".to_string();
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    cfg.rulesets.enabled = vec![];
    cfg
}

/// The gateway config, with the WEB ingress bound ephemerally so the proof can
/// read the SAME metrics registry the supervisor writes through `/healthz` —
/// the endpoint an operator's monitoring actually scrapes.
fn gateway_config(exit_on_abandon: bool) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir("gateway");
    cfg.ssh.listen = "127.0.0.1:0".to_string();
    cfg.rulesets.enabled = vec![];
    cfg.ipc.exit_on_abandoned_supervision = exit_on_abandon;
    let mut web = toml::value::Table::new();
    web.insert(
        "listen".to_string(),
        toml::Value::String("127.0.0.1:0".to_string()),
    );
    web.insert("auth_burst".to_string(), toml::Value::Integer(0));
    cfg.web = toml::Value::Table(web);
    cfg
}

/// Boot one real `mush-server` on its own thread + runtime over an ephemeral
/// IPC endpoint, sharing `store` so the world survives a restart.
async fn launch_thread_server(store: Arc<dyn ObjectStore>) -> Result<LaunchedServer, String> {
    let cfg = server_config();
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

/// A launcher that always brings a real server up: the healthy contrast case.
struct HealthyLauncher {
    store: Arc<dyn ObjectStore>,
}

impl ServerLauncher for HealthyLauncher {
    fn launch(&self) -> BoxFuture<'static, Result<LaunchedServer, String>> {
        Box::pin(launch_thread_server(Arc::clone(&self.store)))
    }
}

/// A launcher whose FIRST boot is a real server and whose every relaunch fails:
/// the crash-loop shape (R3). Once the gateway is playing, one deliberate
/// restart is all it takes to exhaust the guard.
struct NeverComesBackLauncher {
    store: Arc<dyn ObjectStore>,
    launches: AtomicUsize,
}

impl ServerLauncher for NeverComesBackLauncher {
    fn launch(&self) -> BoxFuture<'static, Result<LaunchedServer, String>> {
        let n = self.launches.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            Box::pin(launch_thread_server(Arc::clone(&self.store)))
        } else {
            Box::pin(async move { Err("simulated relaunch failure".to_string()) })
        }
    }
}

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

/// A running gateway, plus the channel its `serve` result comes back on. The
/// result is the whole point of phase 3: a gateway that abandons supervision
/// must RETURN, and return an error, because `main` turns that into a non-zero
/// process exit.
struct Gateway {
    ssh: SocketAddr,
    web: SocketAddr,
    trigger: RestartTrigger,
    exit: oneshot::Receiver<Result<(), String>>,
}

async fn start_gateway(launcher: Box<dyn ServerLauncher>, exit_on_abandon: bool) -> Gateway {
    let gw_store = seeded_gw_store().await;
    let (ssh_tx, ssh_rx) = oneshot::channel();
    let (web_tx, web_rx) = oneshot::channel();
    let (restart_tx, restart_rx) = oneshot::channel::<RestartTrigger>();
    let (exit_tx, exit_rx) = oneshot::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(mush_gateway::serve(
            gateway_config(exit_on_abandon),
            GatewayOptions {
                source: ServerSource::Restartable(launcher),
                store: Some(gw_store),
                ssh_ready: Some(ssh_tx),
                web_ready: Some(web_tx),
                restart_ready: Some(restart_tx),
                siteban_refresh: None,
            },
        ));
        let _ = exit_tx.send(result);
    });
    let ssh = tokio::time::timeout(Duration::from_secs(30), ssh_rx)
        .await
        .expect("gateway bound SSH")
        .expect("ssh addr");
    let web = tokio::time::timeout(Duration::from_secs(30), web_rx)
        .await
        .expect("gateway bound WEB")
        .expect("web addr");
    let trigger = tokio::time::timeout(Duration::from_secs(30), restart_rx)
        .await
        .expect("supervisor handed back a restart trigger")
        .expect("restart trigger");
    Gateway {
        ssh,
        web,
        trigger,
        exit: exit_rx,
    }
}

/// The health JSON an operator's monitoring scrapes.
async fn healthz(addr: SocketAddr) -> Value {
    reqwest::Client::new()
        .get(format!("http://{addr}/healthz"))
        .send()
        .await
        .expect("healthz reachable")
        .json()
        .await
        .expect("healthz json")
}

/// A `tracing` writer that keeps the log in memory, so the proof asserts on the
/// operator-facing marker itself rather than on a paraphrase of it.
#[derive(Clone)]
struct CapturedLog(Arc<Mutex<Vec<u8>>>);

impl CapturedLog {
    fn markers(&self) -> usize {
        String::from_utf8_lossy(&self.0.lock().expect("log buffer poisoned"))
            .matches(SUPERVISION_ABANDONED_MARKER)
            .count()
    }
}

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

// ---------------------------------------------------------------------------
// A real SSH player
// ---------------------------------------------------------------------------

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

    /// Keep reading for `dur`, so a line broadcast a SECOND time lands in
    /// `seen` and [`Term::count`] can catch it. A channel that closes (the
    /// gateway exited) ends the pump early — that is not a failure here.
    async fn pump(&mut self, dur: Duration) {
        let deadline = tokio::time::Instant::now() + dur;
        while let Ok(msg) = tokio::time::timeout_at(deadline, self.channel.wait()).await {
            match msg {
                Some(ChannelMsg::Data { data }) => {
                    self.seen.push_str(&String::from_utf8_lossy(&data));
                }
                Some(_) => {}
                None => break,
            }
        }
    }

    fn count(&self, needle: &str) -> usize {
        strip_ansi(&self.seen).matches(needle).count()
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

/// Log in as God and land in Limbo.
async fn connect_god(addr: SocketAddr) -> (russh::client::Handle<AcceptAll>, Term) {
    let (handle, mut god) = play_client(addr).await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God (account boss, 1 connection).")
        .await;
    god.expect("Limbo #0").await;
    (handle, god)
}

// ---------------------------------------------------------------------------
// The proof
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_abandoned_supervisor_is_visible_then_fatal() {
    let log = CapturedLog(Arc::new(Mutex::new(Vec::new())));
    assert!(
        tracing_subscriber::fmt()
            .with_writer(log.clone())
            .with_max_level(tracing::Level::ERROR)
            .with_ansi(false)
            .try_init()
            .is_ok(),
        "this proof owns the process-wide subscriber; nothing else in this test binary \
         may install one"
    );

    // --- Phase 1: a HEALTHY supervisor never touches any of it. ------------
    // Including across a real restart, which is the case a latch set too
    // eagerly would ruin: a gateway that healed itself must not leave a gauge
    // behind saying it gave up.
    let healthy_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let healthy = start_gateway(
        Box::new(HealthyLauncher {
            store: Arc::clone(&healthy_store),
        }),
        true,
    )
    .await;
    let (_h, mut player) = connect_god(healthy.ssh).await;
    assert_eq!(healthz(healthy.web).await["supervision_ok"], true);

    let mark = player.mark();
    assert!(healthy.trigger.restart().await, "restart must be accepted");
    player.expect_after(mark, "Reconnected").await;
    assert_eq!(
        healthz(healthy.web).await["supervision_ok"],
        true,
        "a supervisor that healed the restart has not abandoned anything"
    );
    assert_eq!(
        log.markers(),
        0,
        "a healthy restart must not log the abandonment marker"
    );
    assert_eq!(player.count(DOWN_NOTICE), 0);
    drop(player);
    drop(healthy);

    // --- Phase 2: abandoned with the exit key OFF (the dev-box corpse). ----
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let mut resident = start_gateway(
        Box::new(NeverComesBackLauncher {
            store: Arc::clone(&store),
            launches: AtomicUsize::new(0),
        }),
        false,
    )
    .await;
    let (_r, mut player) = connect_god(resident.ssh).await;

    let mark = player.mark();
    assert!(resident.trigger.restart().await, "restart must be accepted");
    player.expect_after(mark, "restarting").await;
    // The crash-loop guard exhausts itself and the supervisor gives up.
    player.expect_after(mark, DOWN_NOTICE).await;

    assert_eq!(
        healthz(resident.web).await["supervision_ok"],
        false,
        "the gauge /healthz reads is what turns an invisible fail-stop into an alert"
    );
    assert_eq!(
        log.markers(),
        1,
        "the give-up marker is logged ONCE, not once per retry"
    );

    // Keep reading well past the grace window: with the key off the gateway
    // stays resident, the connection is never dropped, and the notice is not
    // repeated.
    player.pump(PAST_THE_GRACE).await;
    assert_eq!(
        player.count(DOWN_NOTICE),
        1,
        "the down notice is broadcast exactly once, not per cycle"
    );
    assert!(
        resident.exit.try_recv().is_err(),
        "with [ipc].exit_on_abandoned_supervision off, the gateway must stay up"
    );
    assert_eq!(healthz(resident.web).await["supervision_ok"], false);
    drop(player);
    drop(resident);

    // --- Phase 3: abandoned with the exit key ON (production). ------------
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let fatal = start_gateway(
        Box::new(NeverComesBackLauncher {
            store: Arc::clone(&store),
            launches: AtomicUsize::new(0),
        }),
        true,
    )
    .await;
    let (_f, mut player) = connect_god(fatal.ssh).await;

    let mark = player.mark();
    assert!(fatal.trigger.restart().await, "restart must be accepted");
    // The players are told BEFORE the process goes: the grace window exists
    // for exactly this line.
    player.expect_after(mark, DOWN_NOTICE).await;
    let noticed = tokio::time::Instant::now();

    let ended = tokio::time::timeout(PAST_THE_GRACE, fatal.exit)
        .await
        .expect("an abandoned gateway must exit within the grace window")
        .expect("the gateway thread must report its serve result");
    assert!(
        noticed.elapsed() >= Duration::from_secs(2),
        "the exit must wait out a grace window, not race the notice it just sent"
    );
    let error = ended.expect_err("an abandoned gateway must exit NON-ZERO, not cleanly");
    assert!(
        error.contains(SUPERVISION_ABANDONED_MARKER),
        "the exit reason must name the abandonment: {error}"
    );
    assert!(
        error.contains("crash-loop"),
        "the exit reason must name WHICH abandonment: {error}"
    );
    assert_eq!(
        log.markers(),
        2,
        "phase 3 adds exactly one more marker; the exit path must not re-log it"
    );
}

// ---------------------------------------------------------------------------
// The fix-up proof: an unheld restart trigger is not a shutdown
// ---------------------------------------------------------------------------

/// Start a gateway the way `mush-gateway`'s own `main` does — with NO
/// `restart_ready` channel, so nothing outside `serve` is handed the restart
/// trigger. Returns the SSH address and the channel `serve`'s result comes back
/// on.
async fn start_gateway_without_a_trigger(
    launcher: Box<dyn ServerLauncher>,
) -> (SocketAddr, oneshot::Receiver<Result<(), String>>) {
    let gw_store = seeded_gw_store().await;
    let (ssh_tx, ssh_rx) = oneshot::channel();
    let (exit_tx, exit_rx) = oneshot::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(mush_gateway::serve(
            gateway_config(true),
            GatewayOptions {
                source: ServerSource::Restartable(launcher),
                store: Some(gw_store),
                ssh_ready: Some(ssh_tx),
                web_ready: None,
                restart_ready: None,
                siteban_refresh: None,
            },
        ));
        let _ = exit_tx.send(result);
    });
    let ssh = tokio::time::timeout(Duration::from_secs(30), ssh_rx)
        .await
        .expect("gateway bound SSH")
        .expect("ssh addr");
    (ssh, exit_rx)
}

/// "The restart trigger was dropped" is NOT "the gateway is shutting down".
///
/// `serve` builds the trigger, hands one clone to `restart_ready` — which
/// `mush-gateway`'s own `main` sets to `None` — and gives the other to the
/// `SIGHUP` handler, which is a no-op that drops it immediately on non-Unix and
/// drops it on Unix too when `signal()` fails. With every sender gone the
/// supervisor's `restart_rx.recv()` returns `None` at once and supervision ends
/// on a perfectly healthy server. A gateway that now WATCHES its supervisor
/// must not read that as a shutdown, or it boots, stops accepting and exits 0
/// within milliseconds — on the very platform where `SIGHUP` does not exist.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_gateway_nobody_handed_the_restart_trigger_keeps_serving() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let (ssh, mut exit) = start_gateway_without_a_trigger(Box::new(HealthyLauncher {
        store: Arc::clone(&store),
    }))
    .await;

    // Far longer than the supervisor needs to notice an empty channel.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        exit.try_recv().is_err(),
        "a healthy gateway must not exit merely because nothing outside `serve` held \
         its restart trigger"
    );

    // And it is still a GAME, not just a live process: a player who arrives
    // after that window logs in and the world answers.
    let (_h, mut player) = connect_god(ssh).await;
    let mark = player.mark();
    player.send_line("look").await;
    player.expect_after(mark, "Limbo").await;
    assert!(
        exit.try_recv().is_err(),
        "the gateway must still be serving after a player has played"
    );
}

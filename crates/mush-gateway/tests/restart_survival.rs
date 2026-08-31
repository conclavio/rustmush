//! The M16c PROOF: a real server restart WITHOUT dropping player connections.
//!
//! The gateway supervises the server (launching it, here, as a sibling thread
//! over an ephemeral localhost TCP boundary — the same shape as a child
//! process, and deterministic on Windows). A real SSH client connects and plays;
//! then the server is RESTARTED through the gateway's programmatic
//! [`RestartTrigger`] (the cross-platform stand-in for the Unix `SIGHUP` deploy
//! signal). The proof asserts:
//!
//! - the SSH connection SURVIVES — it is never disconnected;
//! - the player is told "restarting" then "reconnected" and is pushed a fresh
//!   room render from the reloaded world (D2/D3);
//! - world state is INTACT across the restart — a room the player `@dig`ged
//!   before the restart is still reachable afterwards (R1: the server reloads
//!   its world from the shared store, exactly as it would from Redis);
//! - the player can keep playing — a command works after the restart;
//! - multi-connect survives — a second connection on the same character rides
//!   the restart too.
//!
//! The server here is launched with a SHARED in-memory store carried across
//! every (re)boot, so world objects + durable module state survive a restart
//! without needing a live Redis on the dev box (the Redis path is byte-identical
//! — the server just reloads the world on boot either way).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use mush_core::GOD;
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use russh::ChannelMsg;
use tokio::sync::oneshot;

use mush_config::accounts::hash_password;
use mush_config::config::Config;
use mush_gateway::{GatewayOptions, LaunchedServer, RestartTrigger, ServerLauncher, ServerSource};

const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const GM_PASSWORD: &str = "gm-secret-1";

fn scratch_dir(tag: &str) -> PathBuf {
    // A UNIQUE dir per call (not just per PID+tag): every server launch — across
    // concurrent tests AND every restart — gets its own data_dir, so the
    // remove_dir_all below never races a sibling launch still using the path.
    // (A shared dir was an intermittent, load-dependent fast failure.)
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("mush-restart-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A test config: everything in-memory, all listeners ephemeral, no rulesets
/// (a lean world so the assertions match the minimal-world room names).
fn test_config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ssh.listen = "127.0.0.1:0".to_string();
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    cfg.rulesets.enabled = vec![];
    cfg
}

/// The restart-survival [`ServerLauncher`]: each `launch` boots a fresh
/// `mush-server` on its OWN thread + runtime (as the real binary does — the
/// engine's module roster is not `Send`, so the boot future cannot be
/// `tokio::spawn`ed) over an ephemeral IPC endpoint, reusing ONE shared store so
/// the world survives every restart. `exited` fires when that server's `serve`
/// returns.
/// Boot one real `mush-server` on its own thread + runtime over an ephemeral IPC
/// endpoint, sharing `store` so the world survives across restarts. Shared by
/// the healthy [`ThreadLauncher`] and the [`FlakyLauncher`] (whose FIRST boot is
/// real before it starts failing).
async fn launch_thread_server(store: Arc<dyn ObjectStore>) -> Result<LaunchedServer, String> {
    let cfg = test_config("server");
    let (ipc_tx, ipc_rx) = oneshot::channel();
    let (exit_tx, exit_rx) = oneshot::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let _ = rt.block_on(mush_server::serve_with_store(cfg, Some(ipc_tx), Some(store)));
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

struct ThreadLauncher {
    store: Arc<dyn ObjectStore>,
}

impl ServerLauncher for ThreadLauncher {
    fn launch(&self) -> BoxFuture<'static, Result<LaunchedServer, String>> {
        let store = Arc::clone(&self.store);
        Box::pin(launch_thread_server(store))
    }
}

/// Like [`launch_thread_server`], but the booted server can be forced to drop its
/// IPC boundary ON ITS OWN — firing `crash` returns from `block_on`, dropping the
/// runtime and every socket, so the gateway's boundary reader hits EOF. That is
/// the crash / auto-restart path (D1a), reached WITHOUT the `RestartTrigger`.
async fn launch_thread_server_abortable(
    store: Arc<dyn ObjectStore>,
    crash: oneshot::Receiver<()>,
) -> Result<LaunchedServer, String> {
    let cfg = test_config("server");
    let (ipc_tx, ipc_rx) = oneshot::channel();
    let (exit_tx, exit_rx) = oneshot::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            tokio::select! {
                _ = mush_server::serve_with_store(cfg, Some(ipc_tx), Some(store)) => {}
                // Forced self-exit (a crash stand-in): return from block_on so the
                // runtime + IPC sockets drop and the gateway sees a boundary EOF,
                // WITHOUT the gateway ever initiating the close.
                _ = crash => {}
            }
        });
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

/// A launcher whose FIRST boot is a real server that can be crashed on demand
/// (its crash sender is published into `crash` for the test to fire); every boot
/// after that is a normal healthy server. Drives the AUTO-restart (D1a) path: the
/// first server self-exits, the supervisor respawns a healthy one and re-attaches.
struct CrashOnceLauncher {
    store: Arc<dyn ObjectStore>,
    launches: AtomicUsize,
    crash: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

impl ServerLauncher for CrashOnceLauncher {
    fn launch(&self) -> BoxFuture<'static, Result<LaunchedServer, String>> {
        let n = self.launches.fetch_add(1, Ordering::SeqCst);
        let store = Arc::clone(&self.store);
        if n == 0 {
            let (crash_tx, crash_rx) = oneshot::channel();
            *self.crash.lock().unwrap() = Some(crash_tx);
            Box::pin(launch_thread_server_abortable(store, crash_rx))
        } else {
            Box::pin(launch_thread_server(store))
        }
    }
}

/// A launcher whose FIRST boot is a real server, and every relaunch after that
/// FAILS — the crash-loop shape (R3). Once the gateway is up and playing, a
/// restart finds no server to respawn, so the supervisor exhausts the crash-loop
/// guard and tells players the server is down (without dropping them).
struct FlakyLauncher {
    store: Arc<dyn ObjectStore>,
    launches: AtomicUsize,
}

impl ServerLauncher for FlakyLauncher {
    fn launch(&self) -> BoxFuture<'static, Result<LaunchedServer, String>> {
        let n = self.launches.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            Box::pin(launch_thread_server(Arc::clone(&self.store)))
        } else {
            Box::pin(async move { Err("simulated relaunch failure".to_string()) })
        }
    }
}

/// The gateway's own KV store (accounts), seeded with the God wizard.
async fn seeded_gw_store() -> Arc<dyn ObjectStore> {
    let gw_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    gw_store
        .create_account(AccountRec {
            name: "boss".to_string(),
            argon2_hash: hash_password(GM_PASSWORD).unwrap(),
            created: 0,
            characters: vec![CharacterRef {
                id: GOD,
                name: "God".to_string(),
            }],
            ssh_keys: vec![],
        })
        .await
        .unwrap();
    gw_store
}

/// Start the gateway (on its own thread/runtime, as the binary does) supervising
/// `launcher`, and return the bound SSH address plus the programmatic restart
/// trigger. The gateway thread handle is detached — `serve` blocks until Ctrl-C,
/// which never comes in-test; dropping at test end tears the process down.
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
                .unwrap_or_else(|_| panic!("timeout waiting for {needle:?}; seen: {:?}", self.seen));
            match msg {
                Some(ChannelMsg::Data { data }) => {
                    self.seen.push_str(&String::from_utf8_lossy(&data));
                }
                Some(_) => {}
                None => panic!("channel closed waiting for {needle:?}; seen: {:?}", self.seen),
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

async fn play_client(addr: std::net::SocketAddr) -> (russh::client::Handle<AcceptAll>, Term) {
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

/// The whole point (R1/R2/R3/R5): connect, play, RESTART the server, and prove
/// the connections survive with world state intact and play continuing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_restart_keeps_connections_and_world() {
    // The gateway's own KV store (accounts), seeded with the God wizard.
    let gw_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    gw_store
        .create_account(AccountRec {
            name: "boss".to_string(),
            argon2_hash: hash_password(GM_PASSWORD).unwrap(),
            created: 0,
            characters: vec![CharacterRef {
                id: GOD,
                name: "God".to_string(),
            }],
            ssh_keys: vec![],
        })
        .await
        .unwrap();

    // The SERVER's world store — SHARED across every restart, so the world
    // survives (stands in for Redis on the dev box).
    let server_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let launcher = ThreadLauncher {
        store: Arc::clone(&server_store),
    };

    // The gateway runs on its own thread/runtime (as the binary does), owning
    // the supervisor that launches + restarts the server.
    let (ssh_tx, ssh_rx) = oneshot::channel();
    let (restart_tx, restart_rx) = oneshot::channel::<RestartTrigger>();
    let gateway_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(mush_gateway::serve(
            test_config("gateway"),
            GatewayOptions {
                source: ServerSource::Restartable(Box::new(launcher)),
                store: Some(gw_store),
                ssh_ready: Some(ssh_tx),
                web_ready: None,
                restart_ready: Some(restart_tx),
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

    // --- God connects on TWO connections (multi-connect) and plays. ---
    let (_g1, mut god1) = play_client(ssh_addr).await;
    god1.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god1.expect("Connected as God (account boss, 1 connection).").await;
    god1.expect("Limbo(#0)").await;

    let (_g2, mut god2) = play_client(ssh_addr).await;
    god2.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god2.expect("Connected as God (account boss, 2 connections).").await;
    god2.expect("Limbo(#0)").await;

    // A mutating command that must survive the restart: dig a room with an exit.
    let before_dig = god1.mark();
    god1.send_line("@dig Vault = east;e, west;w").await;
    god1.expect_after(before_dig, "Vault created with room number").await;

    // Speech round-trips across the boundary before the restart.
    let before_say = god2.mark();
    god1.send_line("say pre-restart").await;
    god2.expect_after(before_say, "God says, \"pre-restart\"").await;

    // --- RESTART the server (the programmatic SIGHUP stand-in). ---
    let restart_mark1 = god1.mark();
    let restart_mark2 = god2.mark();
    assert!(trigger.restart().await, "restart must be accepted");

    // Both connections are told the game is restarting, then reconnected, and
    // are pushed a fresh room render from the reloaded world — WITHOUT ever
    // being disconnected (R1/R3). The gateway generates the restarting/
    // reconnected lines (server is down); the look comes from the new server.
    god1.expect_after(restart_mark1, "restarting").await;
    god1.expect_after(restart_mark1, "Reconnected").await;
    god1.expect_after(restart_mark1, "Limbo(#0)").await;
    god2.expect_after(restart_mark2, "restarting").await;
    god2.expect_after(restart_mark2, "Reconnected").await;
    god2.expect_after(restart_mark2, "Limbo(#0)").await;

    // --- World state INTACT (R1): the room @dug before the restart is still
    //     there. Walk east into the Vault, then back. ---
    let after_restart = god1.mark();
    god1.send_line("east").await;
    god1.expect_after(after_restart, "Vault").await;

    // --- Play CONTINUES after the restart: speech still round-trips, and the
    //     second connection (still in Limbo) hears it — both connections are
    //     live and the world loop is running. ---
    let after_move = god2.mark();
    god1.send_line("west").await; // back to Limbo with god2
    god1.send_line("say post-restart").await;
    god2.expect_after(after_move, "God says, \"post-restart\"").await;

    drop(god1);
    drop(god2);
    let _ = gateway_thread;
}

/// R1/R5, mid-scene: a restart while the player is standing in a room they built
/// and walked into (NOT Limbo) resumes them THERE — the re-attach look renders
/// each session's ACTUAL current room from the reloaded world, not a hardcoded
/// start room. Then play continues from that room.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restart_mid_scene_resumes_in_current_room() {
    let server_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let (ssh_addr, trigger) = start_split(Box::new(ThreadLauncher {
        store: Arc::clone(&server_store),
    }))
    .await;

    let (_g, mut god) = play_client(ssh_addr).await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God (account boss, 1 connection).").await;
    god.expect("Limbo(#0)").await;

    // Build a room, then WALK INTO it — the player is now mid-scene in Vault.
    let before_dig = god.mark();
    god.send_line("@dig Vault = east;e, west;w").await;
    god.expect_after(before_dig, "Vault created with room number").await;
    let before_move = god.mark();
    god.send_line("east").await;
    god.expect_after(before_move, "Vault").await;

    // RESTART while standing in Vault.
    let restart_mark = god.mark();
    assert!(trigger.restart().await, "restart must be accepted");

    // The connection survives, and the re-attach look renders VAULT (the current
    // room), not Limbo — proving per-session current-room render on re-attach.
    god.expect_after(restart_mark, "restarting").await;
    god.expect_after(restart_mark, "Reconnected").await;
    god.expect_after(restart_mark, "Vault").await;

    // Play continues from Vault: walk back west to Limbo.
    let after_restart = god.mark();
    god.send_line("west").await;
    god.expect_after(after_restart, "Limbo(#0)").await;

    drop(god);
}

/// R3, the crash-loop guard: when the server keeps failing to relaunch, the
/// supervisor does NOT respawn forever — it exhausts the guard and tells the
/// still-connected player the server is down, WITHOUT dropping the connection or
/// hanging. Input during the gap is refused with a clear notice, not lost.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crash_loop_guard_gives_up_without_dropping_players() {
    let server_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let (ssh_addr, trigger) = start_split(Box::new(FlakyLauncher {
        store: Arc::clone(&server_store),
        launches: AtomicUsize::new(0),
    }))
    .await;

    let (_g, mut god) = play_client(ssh_addr).await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God (account boss, 1 connection).").await;
    god.expect("Limbo(#0)").await;

    // Trigger a restart the server can never come back from (every relaunch fails).
    let mark = god.mark();
    assert!(trigger.restart().await, "restart must be accepted");

    // The player is told it is restarting, then — once the crash-loop guard is
    // exhausted — that the server is down. The connection is never dropped: it is
    // still delivering system lines here.
    god.expect_after(mark, "restarting").await;
    god.expect_after(mark, "could not be restarted").await;

    drop(god);
}

/// R1/D1a, the AUTO-restart (crash) path: when the server exits ON ITS OWN — NOT
/// through the deliberate `RestartTrigger` — the supervisor must respawn it and
/// re-attach held sessions WITHOUT dropping the connection. This exercises the
/// crash ARM of the supervisor's select, where the boundary reader has ALREADY
/// terminated before the gap code runs: the gap must not re-await that completed
/// down-handle (doing so panics the supervisor — "JoinHandle polled after
/// completion" — and strands every player in the gap forever). Here the first
/// server drops its IPC boundary on its own; the deliberate trigger is untouched.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_self_exit_auto_restarts_without_dropping_players() {
    let server_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let crash: Arc<Mutex<Option<oneshot::Sender<()>>>> = Arc::new(Mutex::new(None));
    let (ssh_addr, _trigger) = start_split(Box::new(CrashOnceLauncher {
        store: Arc::clone(&server_store),
        launches: AtomicUsize::new(0),
        crash: Arc::clone(&crash),
    }))
    .await;

    let (_g, mut god) = play_client(ssh_addr).await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God (account boss, 1 connection).").await;
    god.expect("Limbo(#0)").await;

    // Build something that must survive the auto-restart.
    let before_dig = god.mark();
    god.send_line("@dig Vault = east;e, west;w").await;
    god.expect_after(before_dig, "Vault created with room number").await;

    // Make the server EXIT ON ITS OWN (the crash / auto-restart path, D1a) by
    // firing the first server's crash signal — NOT the RestartTrigger. The
    // gateway's boundary reader hits EOF and the supervisor takes its crash arm.
    let crash_mark = god.mark();
    let crash_tx = crash
        .lock()
        .unwrap()
        .take()
        .expect("first boot published its crash sender");
    let _ = crash_tx.send(());

    // The connection survives the self-exit: restarting → Reconnected → fresh
    // look, never dropped (R1). Before the fix the supervisor panicked re-polling
    // the completed down-handle and this hung until the step timeout.
    god.expect_after(crash_mark, "restarting").await;
    god.expect_after(crash_mark, "Reconnected").await;
    god.expect_after(crash_mark, "Limbo(#0)").await;

    // World intact + play continues after an AUTO restart.
    let after_restart = god.mark();
    god.send_line("east").await;
    god.expect_after(after_restart, "Vault").await;

    drop(god);
}

/// R3, gap-safe with a peer drop: one connection disconnecting AROUND the restart
/// (during the gap) must not break the surviving connection — the other client
/// still resumes with a fresh render and keeps playing. The server re-attaches
/// only the session that is still online.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn client_disconnect_during_gap_leaves_survivor_intact() {
    let server_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let (ssh_addr, trigger) = start_split(Box::new(ThreadLauncher {
        store: Arc::clone(&server_store),
    }))
    .await;

    // Two connections on the same character (multi-connect).
    let (_s1, mut god1) = play_client(ssh_addr).await;
    god1.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god1.expect("Connected as God (account boss, 1 connection).").await;
    god1.expect("Limbo(#0)").await;

    let (s2, mut god2) = play_client(ssh_addr).await;
    god2.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god2.expect("Connected as God (account boss, 2 connections).").await;
    god2.expect("Limbo(#0)").await;

    // Something to prove the world is intact afterward.
    let before_dig = god1.mark();
    god1.send_line("@dig Vault = east;e, west;w").await;
    god1.expect_after(before_dig, "Vault created with room number").await;

    // RESTART, then immediately drop the SECOND connection — it vanishes during
    // the restart gap, before the re-attach completes.
    let restart_mark = god1.mark();
    assert!(trigger.restart().await, "restart must be accepted");
    drop(god2);
    drop(s2);

    // The SURVIVING connection rides the restart: restarting → Reconnected →
    // fresh look, never dropped.
    god1.expect_after(restart_mark, "restarting").await;
    god1.expect_after(restart_mark, "Reconnected").await;
    god1.expect_after(restart_mark, "Limbo(#0)").await;

    // And it keeps playing — the world (the room it dug) is intact, and a command
    // still works after a peer dropped mid-restart.
    let after_restart = god1.mark();
    god1.send_line("east").await;
    god1.expect_after(after_restart, "Vault").await;

    drop(god1);
}

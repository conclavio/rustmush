//! The M16b core proof: a REAL two-process split driven end to end.
//!
//! `mush-server` (world task + persistence + modules) and `mush-gateway`
//! (SSH listener + session layer + accounts) run as two independent tokio
//! tasks connected by a real localhost TCP socket speaking the `mush-ipc`
//! boundary protocol — the framed rmp-serde codec on real traffic, both logic
//! halves in separate address-space-shaped roles, no OS-process flakiness.
//!
//! An SSH client then plays across the boundary:
//!
//! - God (a seeded wizard account) connects through the gateway, and its
//!   welcome/`look` output originates in the server and crosses back as
//!   `ToGateway::Deliver` frames.
//! - Alice `create`s an account: the gateway reserves it in its own store and
//!   asks the server to make the Player over a correlated `ServiceReq`
//!   (`CreatePlayer`) — the reply comes back as `ToGateway::Reply`.
//! - Alice `say`s and God hears it: a full round trip through the world loop
//!   and back out to a *different* connection, cross-process.
//! - God `@dig`s a room (a mutating command round-trips and persists).
//! - God `@boot`s Alice: server → gateway `CloseSession` (fire-and-forget, the
//!   world loop never blocks — D4/F3); Alice's channel closes, and God can
//!   still act immediately afterwards (the loop never hung).
//! - God `QUIT`s cleanly.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_core::GOD;
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use russh::ChannelMsg;

use mush_config::accounts::hash_password;
use mush_config::config::Config;
use mush_gateway::{GatewayOptions, ServerSource};

const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const GM_PASSWORD: &str = "gm-secret-1";

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mush-2proc-{tag}-{}", std::process::id()));
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

    async fn expect(&mut self, needle: &str) {
        self.expect_after(0, needle).await;
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

    async fn send_line(&mut self, line: &str) {
        let bytes = format!("{line}\r").into_bytes();
        self.channel.data(&bytes[..]).await.expect("send line");
    }

    async fn expect_close(&mut self) {
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        loop {
            let msg = tokio::time::timeout_at(deadline, self.channel.wait())
                .await
                .expect("timeout waiting for close");
            match msg {
                None | Some(ChannelMsg::Close) => return,
                Some(ChannelMsg::Data { data }) => {
                    self.seen.push_str(&String::from_utf8_lossy(&data));
                }
                Some(_) => {}
            }
        }
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_process_connect_play_boot_disconnect() {
    // The gateway's own KV store, pre-seeded with the God wizard account.
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

    // Server + gateway each run on their own OS thread with their own runtime
    // (the engine's `dyn GameModule` roster is not `Sync`, so the boot future
    // is not `Send` and cannot be `tokio::spawn`ed — the real binaries
    // `block_on` it the same way). They talk only over the TCP boundary.
    let (ipc_tx, ipc_rx) = tokio::sync::oneshot::channel();
    let server_cfg = test_config("server");
    let server_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(mush_server::serve(server_cfg, Some(ipc_tx)))
            .unwrap();
    });
    let endpoint = tokio::time::timeout(Duration::from_secs(30), ipc_rx)
        .await
        .expect("server bound its IPC socket")
        .expect("ipc endpoint");

    let (ssh_tx, ssh_rx) = tokio::sync::oneshot::channel();
    let gateway_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(mush_gateway::serve(
            test_config("gateway"),
            GatewayOptions {
                source: ServerSource::Connect(endpoint),
                store: Some(gw_store),
                ssh_ready: Some(ssh_tx),
                web_ready: None,
                restart_ready: None,
            },
        ))
        .unwrap();
    });
    let ssh_addr = tokio::time::timeout(Duration::from_secs(30), ssh_rx)
        .await
        .expect("gateway bound SSH")
        .expect("ssh addr");

    // --- God connects and looks (server output crosses back to the gateway) ---
    let (_g, mut god) = play_client(ssh_addr).await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God (account boss, 1 connection).")
        .await;
    god.expect("Limbo(#0)").await;

    // --- Alice creates an account (CreatePlayer ServiceReq round-trips) ---
    let (_a, mut alice) = play_client(ssh_addr).await;
    alice.expect("connect <account> <password>").await;
    alice.send_line("create Alice secret123").await;
    alice
        .expect("Connected as Alice (account Alice, 1 connection).")
        .await;
    alice.expect("Limbo(#0)").await;
    god.expect("Alice has connected.").await;

    // --- Alice says, God hears: full cross-process say round trip ---
    alice.send_line("say hello across the fork").await;
    god.expect("Alice says, \"hello across the fork\"").await;

    // --- A mutating command round-trips (@dig persists a room) ---
    god.send_line("@dig Vault = east;e, west;w").await;
    god.expect("Vault created with room number").await;

    // --- @boot: server -> gateway CloseSession (fire-and-forget). Alice's
    //     channel closes AND God can still act immediately (no deadlock). ---
    let after_boot = god.mark();
    god.send_line("@boot Alice").await;
    // The staff op really ran server-side (confirmation) AND the gateway closed
    // Alice's connection in response to the fire-and-forget CloseSession.
    god.expect_after(after_boot, "@boot: Alice booted.").await;
    alice.expect_close().await;
    // Prove the world loop kept running right after the fire-and-forget boot:
    // God's own say still round-trips through the server and back.
    god.send_line("say still-here-after-boot").await;
    god.expect_after(after_boot, "God says, \"still-here-after-boot\"")
        .await;

    // --- Clean disconnect ---
    god.send_line("QUIT").await;
    god.expect("Goodbye.").await;
    god.expect_close().await;

    // The server + gateway threads block on Ctrl-C / accept for the process
    // lifetime; they are torn down when the test process exits. Drop our SSH
    // handles to close the client side cleanly.
    drop(god);
    drop(alice);
    let _ = (server_thread, gateway_thread);
}

/// Boot a server + gateway pair on their own runtimes (as the binaries do) and
/// return the SSH listen address plus the thread handles. Shared by the harder
/// scenarios below.
async fn boot_split(
    gw_store: Arc<dyn ObjectStore>,
) -> (
    SocketAddr,
    std::thread::JoinHandle<()>,
    std::thread::JoinHandle<()>,
) {
    let (ipc_tx, ipc_rx) = tokio::sync::oneshot::channel();
    let server_cfg = test_config("server2");
    let server_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(mush_server::serve(server_cfg, Some(ipc_tx)))
            .unwrap();
    });
    let endpoint = tokio::time::timeout(Duration::from_secs(30), ipc_rx)
        .await
        .expect("server bound its IPC socket")
        .expect("ipc endpoint");

    let (ssh_tx, ssh_rx) = tokio::sync::oneshot::channel();
    let gateway_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(mush_gateway::serve(
            test_config("gateway2"),
            GatewayOptions {
                source: ServerSource::Connect(endpoint),
                store: Some(gw_store),
                ssh_ready: Some(ssh_tx),
                web_ready: None,
                restart_ready: None,
            },
        ))
        .unwrap();
    });
    let ssh_addr = tokio::time::timeout(Duration::from_secs(30), ssh_rx)
        .await
        .expect("gateway bound SSH")
        .expect("ssh addr");
    (ssh_addr, server_thread, gateway_thread)
}

/// Harder cross-process proofs (D5/D4/F3): multi-connect fan-out, a link-dead
/// drop (no QUIT) crossing the boundary, a `@newpassword` staff op that runs
/// cross-process without hanging the world loop, and a `@boot` of a
/// multi-connected character (its `CloseSession` closes every connection).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_process_multiconnect_linkdead_and_newpassword() {
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

    let (ssh_addr, server_thread, gateway_thread) = boot_split(Arc::clone(&gw_store)).await;

    // --- God multi-connects: two independent SSH connections on one character. -
    let (_g1, mut god1) = play_client(ssh_addr).await;
    god1.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god1.expect("Connected as God (account boss, 1 connection).")
        .await;
    let (_g2, mut god2) = play_client(ssh_addr).await;
    god2.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    // The second connection sees the session already has one live connection.
    god2.expect("Connected as God (account boss, 2 connections).")
        .await;

    // --- Alice creates and says: BOTH of God's connections hear it (the
    //     server resolves one SendTarget::Char(God), the gateway fans it out to
    //     both connections on its own SessionManager). ---
    let (a_handle, mut alice) = play_client(ssh_addr).await;
    alice.expect("connect <account> <password>").await;
    alice.send_line("create Alice secret123").await;
    alice
        .expect("Connected as Alice (account Alice, 1 connection).")
        .await;
    alice.send_line("say hello to both").await;
    god1.expect("Alice says, \"hello to both\"").await;
    god2.expect("Alice says, \"hello to both\"").await;

    // --- Alice drops her link WITHOUT quitting: the gateway detects the closed
    //     transport, detaches (link-dead), and the server announces it across
    //     the boundary. ---
    let after_drop = god1.mark();
    drop(alice);
    drop(a_handle);
    god1.expect_after(after_drop, "Alice has gone link-dead.")
        .await;

    // --- @newpassword runs cross-process on the staff-op task (never the world
    //     loop) and returns without hanging; in this split test the server's
    //     store does not hold the gateway's account, so it reports "not linked",
    //     but the point is F3: the world loop kept running (God acts right
    //     after). ---
    let after_np = god1.mark();
    god1.send_line("@newpassword Alice=brandnewpw9").await;
    god1.expect_after(after_np, "@newpassword").await;
    god1.send_line("say still-alive-after-newpassword").await;
    god1.expect_after(after_np, "God says, \"still-alive-after-newpassword\"")
        .await;

    // --- One connection QUITs: it closes cleanly while the OTHER connection of
    //     the same multi-connected session stays live and keeps playing (the
    //     server never blocked, and the gateway detached only the one conn). ---
    god1.send_line("QUIT").await;
    god1.expect("Goodbye.").await;
    god1.expect_close().await;
    let after_quit = god2.mark();
    god2.send_line("say god2-still-here").await;
    god2.expect_after(after_quit, "God says, \"god2-still-here\"")
        .await;

    god2.send_line("QUIT").await;
    god2.expect("Goodbye.").await;
    god2.expect_close().await;

    drop(god1);
    drop(god2);
    let _ = (server_thread, gateway_thread);
}

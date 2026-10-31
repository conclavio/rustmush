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
//! - Alice makes a `@macro` and types it, and God hears the expansion: the
//!   kernel's macro alias stage runs in the server half (16b cut AD.1).
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
use mush_gateway::GatewayOptions;

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
        let mut gw_cfg = test_config("gateway");
        gw_cfg.ipc.listen = endpoint.describe();
        rt.block_on(mush_gateway::serve(
            gw_cfg,
            GatewayOptions {
                store: Some(gw_store),
                ssh_ready: Some(ssh_tx),
                web_ready: None,
                siteban_refresh: None,
                give_up_after: None,
                generation: None,
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
    god.expect("Limbo #0").await;

    // --- Alice creates an account (CreatePlayer ServiceReq round-trips) ---
    let (_a, mut alice) = play_client(ssh_addr).await;
    alice.expect("connect <account> <password>").await;
    alice.send_line("create Alice secret123").await;
    alice
        .expect("Connected as Alice (account Alice, 1 connection).")
        .await;
    alice.expect("Limbo #0").await;
    god.expect("Alice has connected.").await;

    // --- Alice says, God hears: full cross-process say round trip ---
    alice.send_line("say hello across the fork").await;
    god.expect("Alice says, \"hello across the fork\"").await;

    // --- A macro expands in the server half (16b cut AD.1): the gateway never
    //     saw a verb called `hey`, so God hearing the expansion proves the
    //     kernel's alias stage runs across the IPC boundary ---
    alice.send_line("@macro hey=say hey there, $1").await;
    alice.expect("Macro hey set: say hey there, $1").await;
    alice.send_line("hey God").await;
    god.expect("Alice says, \"hey there, God\"").await;

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
        let mut gw_cfg = test_config("gateway2");
        gw_cfg.ipc.listen = endpoint.describe();
        rt.block_on(mush_gateway::serve(
            gw_cfg,
            GatewayOptions {
                store: Some(gw_store),
                ssh_ready: Some(ssh_tx),
                web_ready: None,
                siteban_refresh: None,
                give_up_after: None,
                generation: None,
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

// ---------------------------------------------------------------------------
// The siteban seam (WS02-S7): `@siteban` runs in the SERVER half and mirrors
// the ban into the SERVER's `IpGuard` — which, in a fork, admits nothing. The
// guard the SSH listener actually consults lives in the GATEWAY, and used to be
// seeded once at boot: a staff ban persisted correctly and never took effect
// until the gateway was restarted. The gateway now re-reads `ban:*` from the
// shared store on a fixed interval, so a set, a clear and an expiry all
// propagate across the fork with no new boundary frame and no restart.
// ---------------------------------------------------------------------------

/// The refresh interval used while a staff confirmation is in flight. It is
/// deliberately not the shortest interval the mechanism allows: the ban ejects
/// the staff member who issued it (everyone here speaks from loopback), so the
/// confirmation needs room to reach the terminal before the socket closes.
/// Production is `SITEBAN_REFRESH` (10s); this is the same mechanism on a
/// shorter clock, not a different one.
const BAN_REFRESH: Duration = Duration::from_secs(1);

/// The refresh interval for the expiry proof, where nothing is in flight and a
/// tight loop just means a shorter test.
const EXPIRY_REFRESH: Duration = Duration::from_millis(250);

/// Boot a server + gateway pair sharing ONE store, exactly as the split
/// deployment shares one Redis. That sharing is the whole mechanism: the server
/// half writes `ban:{ip}`, the gateway half reads it back.
async fn boot_split_sharing_store(
    tag: &str,
    store: Arc<dyn ObjectStore>,
    refresh: Duration,
) -> (
    SocketAddr,
    std::thread::JoinHandle<()>,
    std::thread::JoinHandle<()>,
) {
    let (ipc_tx, ipc_rx) = tokio::sync::oneshot::channel();
    let server_cfg = test_config(&format!("{tag}-server"));
    let server_store = Arc::clone(&store);
    let server_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(mush_server::serve_with_store(
            server_cfg,
            Some(ipc_tx),
            Some(server_store),
        ))
        .unwrap();
    });
    let endpoint = tokio::time::timeout(Duration::from_secs(30), ipc_rx)
        .await
        .expect("server bound its IPC socket")
        .expect("ipc endpoint");

    // Seed the wizard account AFTER the server has seeded its world into the
    // same store, so the two never race over a fresh one.
    store
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

    let (ssh_tx, ssh_rx) = tokio::sync::oneshot::channel();
    let mut gw_cfg = test_config(&format!("{tag}-gateway"));
    gw_cfg.ipc.listen = endpoint.describe();
    let gateway_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(mush_gateway::serve(
            gw_cfg,
            GatewayOptions {
                store: Some(store),
                ssh_ready: Some(ssh_tx),
                web_ready: None,
                siteban_refresh: Some(refresh),
                give_up_after: None,
                generation: None,
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

/// Try the anonymous front door. `false` when the gateway refuses the address —
/// a live siteban rejects `auth_none` before a welcome screen ever exists.
async fn front_door_admits(addr: SocketAddr) -> bool {
    let config = Arc::new(russh::client::Config::default());
    match russh::client::connect(config, addr, AcceptAll).await {
        Ok(mut handle) => matches!(handle.authenticate_none("play").await, Ok(a) if a.success()),
        Err(_) => false,
    }
}

/// Poll the front door until it reports `want`, or fail the test.
async fn wait_front_door(addr: SocketAddr, want: bool, poll: Duration, what: &str) {
    let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
    loop {
        if front_door_admits(addr).await == want {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timeout waiting for the gateway to {what}"
        );
        tokio::time::sleep(poll).await;
    }
}

/// The core defect: a staff `@siteban` issued in the server half must reach the
/// gateway's admission guard without a gateway restart.
///
/// Pre-fix, the marked assertions below fail — the gateway seeded its mirror
/// once at boot, so a brand-new connection from the banned address sailed
/// through the front door and the already-connected player kept playing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn split_siteban_reaches_the_gateway_without_a_restart() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let (ssh_addr, server_thread, gateway_thread) =
        boot_split_sharing_store("siteban", Arc::clone(&store), BAN_REFRESH).await;

    let (_g, mut god) = play_client(ssh_addr).await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God (account boss, 1 connection).")
        .await;

    // Every client in this rig speaks from loopback, so staff ban themselves
    // along with everyone else — which is exactly what makes the eject limb
    // observable on God's own channel.
    god.send_line("@siteban 127.0.0.1=1h split-seam proof")
        .await;
    // The gateway may eject this very connection as soon as it reads the
    // committed ban, before its confirmation crosses IPC/SSH. Do not require
    // output to win that race. The non-self-ban persistence test checks the
    // confirmation; this test proves committed state, ejection, and admission.
    god.expect_close().await;
    let bans = store.get_bans().await.unwrap();
    assert_eq!(bans.len(), 1, "ban persisted");
    assert_eq!(bans[0].ip, "127.0.0.1");
    assert_eq!(bans[0].reason, "split-seam proof");
    assert!(
        bans[0]
            .until_unix
            .is_some_and(|at| at > mush_config::unix_now_s())
    );

    // 2. A NEW connection attempt is refused. THIS is the pre-fix failure: the
    //    gateway's boot-seeded mirror knew nothing about the ban and admitted
    //    the address.
    wait_front_door(
        ssh_addr,
        false,
        BAN_REFRESH,
        "refuse the newly banned address",
    )
    .await;

    // 3. Lifting re-admits on the next refresh. `@siteban/clear` writes exactly
    //    this store record and nothing else; it is issued through the store
    //    here because the ban just locked every staff member out of the game.
    store.clear_ban("127.0.0.1").await.unwrap();
    wait_front_door(ssh_addr, true, BAN_REFRESH, "re-admit the lifted address").await;

    // ...and the front door is a real front door again.
    let (_g2, mut god2) = play_client(ssh_addr).await;
    god2.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god2.expect("Connected as God (account boss, 1 connection).")
        .await;

    drop(god);
    drop(god2);
    let _ = (server_thread, gateway_thread);
}

/// A ban that lapses by its own `until_unix` stops blocking with no command run
/// anywhere and no restart: the refresh replaces the mirror wholesale, and
/// `load_sitebans` drops what has already expired on the way in.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn split_siteban_expiry_stops_blocking_without_any_command() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let (ssh_addr, server_thread, gateway_thread) =
        boot_split_sharing_store("expiry", Arc::clone(&store), EXPIRY_REFRESH).await;

    // Timed from AFTER boot, so a slow boot cannot expire the ban before the
    // test has looked at it once.
    store
        .set_ban("127.0.0.1", Some(mush_config::unix_now_s() + 4), "lapses")
        .await
        .unwrap();
    wait_front_door(
        ssh_addr,
        false,
        EXPIRY_REFRESH,
        "refuse the timed ban's address",
    )
    .await;

    // No `@siteban/clear`, no restart: just the clock.
    wait_front_door(ssh_addr, true, EXPIRY_REFRESH, "forget the expired ban").await;

    let _ = (server_thread, gateway_thread);
}

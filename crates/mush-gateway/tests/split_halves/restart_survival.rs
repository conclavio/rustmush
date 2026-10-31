//! The M16c PROOF: a real server restart WITHOUT dropping player connections.
//!
//! The server is its own unit (doc 33 GG.1), stood in for here by the
//! [`ServerUnit`] rig: a real `mush-server` on a sibling thread over a
//! localhost TCP boundary the gateway DIALS — the same shape as the unit's
//! socket, and deterministic on Windows. A real SSH client connects and
//! plays; then the server is RESTARTED from outside the gateway, as
//! `systemctl restart <inst>mush-server` does it: the running server exits,
//! a fresh one binds the same endpoint. The proof asserts:
//!
//! - the SSH connection SURVIVES — it is never disconnected;
//! - the player is told "restarting", is pushed a fresh room render from the
//!   reloaded world, and only sees "reconnected" after the replay plus an exact
//!   readiness barrier have crossed (D2/D3);
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
use std::time::Duration;

use mush_core::GOD;
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use russh::ChannelMsg;
use tokio::sync::oneshot;

use mush_config::accounts::hash_password;
use mush_config::config::Config;
use mush_gateway::GatewayOptions;

use super::harness::ServerUnit;

const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const GM_PASSWORD: &str = "gm-secret-1";
/// The world's TRANSIENT authoritative refusal: a commit is in flight.
const COMMIT_PENDING: &str = "please retry in a moment";
/// How many times a mutating line is re-sent on that refusal.
const COMMIT_RETRIES: usize = 8;

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
    cfg
}

/// The server unit over `store` (`harness.rs`): every boot reads a fresh lean
/// config and binds the endpoint the first boot bound, reusing ONE shared
/// store so the world survives every restart.
fn unit(store: Arc<dyn ObjectStore>) -> ServerUnit {
    ServerUnit::new(store, || test_config("server"))
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

/// Start the gateway (on its own thread/runtime, as the binary does) aimed at
/// `unit`, and return the bound SSH address. The gateway thread handle is
/// detached — `serve` blocks until Ctrl-C, which never comes in-test; dropping
/// at test end tears the process down.
async fn start_split(unit: &ServerUnit) -> SocketAddr {
    let gw_store = seeded_gw_store().await;
    let (ssh_tx, ssh_rx) = oneshot::channel();
    let mut cfg = test_config("gateway");
    unit.aim(&mut cfg);
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(mush_gateway::serve(
            cfg,
            GatewayOptions {
                store: Some(gw_store),
                ssh_ready: Some(ssh_tx),
                web_ready: None,
                siteban_refresh: None,
                give_up_after: None,
            },
        ))
        .unwrap();
    });
    tokio::time::timeout(Duration::from_secs(30), ssh_rx)
        .await
        .expect("gateway bound SSH")
        .expect("ssh addr")
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

    /// Run one line and wait for `needle`, **retrying while the world answers
    /// the transient authoritative refusal**.
    ///
    /// A mutating verb rides the authoritative lane, and that lane answers
    /// `mush_server`'s `RefusalReason::CommitPending` — "the world is committing
    /// another action; please retry in a moment" — whenever a commit is in
    /// flight. It is a real answer a real client retries, not a failure, and
    /// under a loaded `cargo test --workspace` it lands often enough to make a
    /// one-shot `send_line` + `expect_after` racy.
    async fn say(&mut self, line: &str, needle: &str) {
        for _ in 0..COMMIT_RETRIES {
            let mark = self.mark();
            self.send_line(line).await;
            if self
                .expect_one_of_after(mark, &[needle, COMMIT_PENDING])
                .await
                == 0
            {
                return;
            }
        }
        panic!("`{line}` was refused as commit-pending {COMMIT_RETRIES} times running");
    }

    /// Wait for whichever of `needles` arrives first, answering its index.
    async fn expect_one_of_after(&mut self, mark: usize, needles: &[&str]) -> usize {
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        loop {
            let seen = strip_ansi(&self.seen[mark..]);
            if let Some(index) = needles.iter().position(|needle| seen.contains(needle)) {
                return index;
            }
            let msg = tokio::time::timeout_at(deadline, self.channel.wait())
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "timeout waiting for any of {needles:?}; seen: {:?}",
                        self.seen
                    )
                });
            match msg {
                Some(ChannelMsg::Data { data }) => {
                    self.seen.push_str(&String::from_utf8_lossy(&data));
                }
                Some(_) => {}
                None => panic!(
                    "channel closed waiting for any of {needles:?}; seen: {:?}",
                    self.seen
                ),
            }
        }
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

    // The SERVER's world store — SHARED across every restart, so the world
    // survives (stands in for Redis on the dev box). The unit boots first
    // (`After=`), then the gateway on its own thread/runtime (as the binary
    // does) dials it.
    let server_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let unit = unit(server_store);
    unit.start().await;
    let (ssh_tx, ssh_rx) = oneshot::channel();
    let mut gw_cfg = test_config("gateway");
    unit.aim(&mut gw_cfg);
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(mush_gateway::serve(
            gw_cfg,
            GatewayOptions {
                store: Some(gw_store),
                ssh_ready: Some(ssh_tx),
                web_ready: None,
                siteban_refresh: None,
                give_up_after: None,
            },
        ))
        .unwrap();
    });
    let ssh_addr = tokio::time::timeout(Duration::from_secs(30), ssh_rx)
        .await
        .expect("gateway bound SSH")
        .expect("ssh addr");

    // --- God connects on TWO connections (multi-connect) and plays. ---
    let (_g1, mut god1) = play_client(ssh_addr).await;
    god1.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god1.expect("Connected as God (account boss, 1 connection).")
        .await;
    god1.expect("Limbo #0").await;

    let (_g2, mut god2) = play_client(ssh_addr).await;
    god2.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god2.expect("Connected as God (account boss, 2 connections).")
        .await;
    god2.expect("Limbo #0").await;

    // A mutating command that must survive the restart: dig a room with an exit.
    god1.say(
        "@dig Vault = east;e, west;w",
        "Vault created with room number",
    )
    .await;

    // Speech round-trips across the boundary before the restart.
    let before_say = god2.mark();
    god1.send_line("say pre-restart").await;
    god2.expect_after(before_say, "God says, \"pre-restart\"")
        .await;

    // --- RESTART the server (`systemctl restart`: it exits, a fresh one binds). ---
    let restart_mark1 = god1.mark();
    let restart_mark2 = god2.mark();
    unit.restart().await;

    // Both connections are told the game is restarting, are pushed a fresh
    // room render from the reloaded world, then are told reconnected only after
    // the replay + exact readiness barrier — WITHOUT ever being disconnected
    // (R1/R3). The gateway generates the system lines; the look comes from the
    // new server. We search from one mark because transport scheduling need not
    // impose a presentation order between the replay render and final notice.
    god1.expect_after(restart_mark1, "restarting").await;
    god1.expect_after(restart_mark1, "Reconnected").await;
    god1.expect_after(restart_mark1, "Limbo #0").await;
    god2.expect_after(restart_mark2, "restarting").await;
    god2.expect_after(restart_mark2, "Reconnected").await;
    god2.expect_after(restart_mark2, "Limbo #0").await;

    // --- World state INTACT (R1): the room @dug before the restart is still
    //     there. Walk east into the Vault, then back. ---
    let after_restart = god1.mark();
    god1.send_line("east").await;
    god1.expect_after(after_restart, "Vault").await;

    // --- Play CONTINUES after the restart: speech still round-trips, and the
    //     second connection (still in Limbo) hears it — both connections are
    //     live and the world loop is running. ---
    let after_move = god2.mark();
    let before_west = god1.mark();
    god1.send_line("west").await; // back to Limbo with god2
    // Command output is acknowledgement-gated. Wait for west's room render
    // before issuing another action; firing `say` immediately would correctly
    // receive typed CommitPending Busy while west's receipt is outstanding.
    god1.expect_after(before_west, "Limbo #0").await;
    god1.send_line("say post-restart").await;
    god2.expect_after(after_move, "God says, \"post-restart\"")
        .await;

    drop(god1);
    drop(god2);
}

/// R1/R5, mid-scene: a restart while the player is standing in a room they built
/// and walked into (NOT Limbo) resumes them THERE — the re-attach look renders
/// each session's ACTUAL current room from the reloaded world, not a hardcoded
/// start room. Then play continues from that room.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restart_mid_scene_resumes_in_current_room() {
    let server_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let unit = unit(server_store);
    unit.start().await;
    let ssh_addr = start_split(&unit).await;

    let (_g, mut god) = play_client(ssh_addr).await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God (account boss, 1 connection).")
        .await;
    god.expect("Limbo #0").await;

    // Build a room, then WALK INTO it — the player is now mid-scene in Vault.
    god.say(
        "@dig Vault = east;e, west;w",
        "Vault created with room number",
    )
    .await;
    let before_move = god.mark();
    god.send_line("east").await;
    god.expect_after(before_move, "Vault").await;

    // RESTART while standing in Vault.
    let restart_mark = god.mark();
    unit.restart().await;

    // The connection survives, and the re-attach look renders VAULT (the current
    // room), not Limbo — proving per-session current-room render on re-attach.
    god.expect_after(restart_mark, "restarting").await;
    god.expect_after(restart_mark, "Reconnected").await;
    god.expect_after(restart_mark, "Vault").await;

    // Play continues from Vault: walk back west to Limbo.
    let after_restart = god.mark();
    god.send_line("west").await;
    god.expect_after(after_restart, "Limbo #0").await;

    drop(god);
}

/// A server unit that stays away (doc 33 GG.1): the gateway holds the player
/// through the whole gap, refuses input with a clear notice rather than losing
/// it, never drops or hangs the connection, and heals on its own the moment
/// the unit is back. (Past the start-limit bound the notice changes to the
/// give-up line; that is `server_unit.rs`'s proof.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_server_that_stays_away_keeps_the_player_held_and_input_refused() {
    let server_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let unit = unit(server_store);
    unit.start().await;
    let ssh_addr = start_split(&unit).await;

    let (_g, mut god) = play_client(ssh_addr).await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God (account boss, 1 connection).")
        .await;
    god.expect("Limbo #0").await;

    // The unit goes down and nothing brings it back for a while.
    let mark = god.mark();
    unit.stop().await;
    god.expect_after(mark, "restarting").await;

    // Input during the gap is refused with a clear notice, not lost.
    let input_mark = god.mark();
    god.send_line("say anyone there").await;
    god.expect_after(input_mark, "your input was not sent")
        .await;
    // Still away: still connected, still refused, not dropped.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let input_mark = god.mark();
    god.send_line("say still here").await;
    god.expect_after(input_mark, "The server is restarting; hold on")
        .await;

    // The unit is back: the gateway heals on its own, nobody is asked.
    let heal_mark = god.mark();
    unit.start().await;
    god.expect_after(heal_mark, "Reconnected").await;
    god.expect_after(heal_mark, "Limbo #0").await;
    let after = god.mark();
    god.send_line("say back").await;
    god.expect_after(after, "God says, \"back\"").await;

    drop(god);
}

/// R1/D1a, the crash path: when the server exits ON ITS OWN — a crash, not a
/// deploy — the keeper must enter the gap from the boundary reader's end and
/// re-attach held sessions WITHOUT dropping the connection once the unit has
/// brought the server back. This exercises the arm of the keeper's select
/// where the boundary reader has ALREADY terminated before the gap code runs:
/// the gap must not re-await that completed down-handle (doing so panics the
/// keeper — "JoinHandle polled after completion" — and strands every player
/// in the gap forever).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_self_exit_auto_restarts_without_dropping_players() {
    let server_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let unit = unit(server_store);
    unit.start().await;
    let ssh_addr = start_split(&unit).await;

    let (_g, mut god) = play_client(ssh_addr).await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God (account boss, 1 connection).")
        .await;
    god.expect("Limbo #0").await;

    // Build something that must survive the auto-restart.
    god.say(
        "@dig Vault = east;e, west;w",
        "Vault created with room number",
    )
    .await;

    // Make the server EXIT ON ITS OWN (the crash path, D1a): the gateway's
    // boundary reader hits EOF and the keeper takes its down arm. Then the
    // unit's `Restart=on-failure` brings a fresh server up at the same
    // endpoint.
    let crash_mark = god.mark();
    unit.stop().await;
    unit.start().await;

    // The connection survives the self-exit: restarting → Reconnected → fresh
    // look, never dropped (R1). Before the fix the keeper panicked re-polling
    // the completed down-handle and this hung until the step timeout.
    god.expect_after(crash_mark, "restarting").await;
    god.expect_after(crash_mark, "Reconnected").await;
    god.expect_after(crash_mark, "Limbo #0").await;

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
    let unit = unit(server_store);
    unit.start().await;
    let ssh_addr = start_split(&unit).await;

    // Two connections on the same character (multi-connect).
    let (_s1, mut god1) = play_client(ssh_addr).await;
    god1.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god1.expect("Connected as God (account boss, 1 connection).")
        .await;
    god1.expect("Limbo #0").await;

    let (s2, mut god2) = play_client(ssh_addr).await;
    god2.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god2.expect("Connected as God (account boss, 2 connections).")
        .await;
    god2.expect("Limbo #0").await;

    // Something to prove the world is intact afterward.
    god1.say(
        "@dig Vault = east;e, west;w",
        "Vault created with room number",
    )
    .await;

    // RESTART, then immediately drop the SECOND connection — it vanishes during
    // the restart gap, before the re-attach completes.
    let restart_mark = god1.mark();
    unit.restart().await;
    drop(god2);
    drop(s2);

    // The SURVIVING connection rides the restart: restarting → Reconnected →
    // fresh look, never dropped.
    god1.expect_after(restart_mark, "restarting").await;
    god1.expect_after(restart_mark, "Reconnected").await;
    god1.expect_after(restart_mark, "Limbo #0").await;

    // And it keeps playing — the world (the room it dug) is intact, and a command
    // still works after a peer dropped mid-restart.
    let after_restart = god1.mark();
    god1.send_line("east").await;
    god1.expect_after(after_restart, "Vault").await;

    drop(god1);
}

/// Retained delimiters belong to one gateway connection, survive a server restart,
/// and are restored before the first post-restart setting command is evaluated.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reply_settings_survive_restart_and_remain_connection_local() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let unit = unit(store);
    unit.start().await;
    let addr = start_split(&unit).await;
    let (_a, mut a) = play_client(addr).await;
    a.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    a.expect("Connected as God").await;
    a.expect("Limbo #0").await;
    let (_b, mut b) = play_client(addr).await;
    b.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    b.expect("Connected as God").await;
    b.expect("Limbo #0").await;
    let mark = a.mark();
    a.send_line("OUTPUTSUFFIX __END__").await;
    a.expect_after(mark, "\n__END__\r\n").await;
    let mark = a.mark();
    a.send_line("OUTPUTPREFIX __BEGIN__").await;
    a.expect_after(mark, "\n__END__\r\n").await;

    let mark = a.mark();
    a.send_line("think before-restart").await;
    a.expect_after(mark, "\n__END__\r\n").await;
    let text = strip_ansi(&a.seen[mark..]);
    assert!(
        text.find("__BEGIN__").unwrap() < text.rfind("before-restart").unwrap(),
        "{text}"
    );
    assert!(
        text.rfind("before-restart").unwrap() < text.find("__END__").unwrap(),
        "{text}"
    );
    let peer = b.mark();
    b.send_line("think peer-unframed").await;
    b.expect_after(peer, "\npeer-unframed\r\n").await;
    let text = strip_ansi(&b.seen[peer..]);
    assert!(
        !text.contains("__BEGIN__") && !text.contains("__END__"),
        "{text}"
    );

    let mark = a.mark();
    unit.restart().await;
    a.expect_after(mark, "Reconnected").await;
    let mark = a.mark();
    a.send_line("think after-restart").await;
    a.expect_after(mark, "\n__END__\r\n").await;
    let text = strip_ansi(&a.seen[mark..]);
    assert!(
        text.find("__BEGIN__").unwrap() < text.rfind("after-restart").unwrap(),
        "{text}"
    );
    assert!(
        text.rfind("after-restart").unwrap() < text.find("__END__").unwrap(),
        "{text}"
    );
    // A suffix-only update after restart must keep the restored prefix.
    let mark = a.mark();
    a.send_line("OUTPUTSUFFIX __NEW_END__").await;
    a.expect_after(mark, "\n__NEW_END__\r\n").await;
    assert!(strip_ansi(&a.seen[mark..]).contains("__BEGIN__"));
    let mark = a.mark();
    a.send_line("think restored-prefix").await;
    a.expect_after(mark, "\n__NEW_END__\r\n").await;
    assert!(strip_ansi(&a.seen[mark..]).contains("__BEGIN__"));
}

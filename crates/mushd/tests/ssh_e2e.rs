//! The M1 exit criterion as one scripted end-to-end journey over real SSH.
//!
//! Boots the whole server (world task, session layer, store-backed
//! accounts, IP guard, SSH listener) on an ephemeral port with a scratch
//! data dir —
//! production wiring except for port and paths — then drives three russh
//! clients through the classic session:
//!
//! - God (a seeded account linked to `#1`) connects via the welcome
//!   screen's `connect`, runs `@slowtest` (typed async re-entry, with a
//!   `think` interleaved to prove the loop never blocked) and `@dig`s a
//!   room with exits both ways.
//! - Alice and Bob `create` accounts in-band (argon2 + world round trip),
//!   land in Limbo together, `say` to each other, and Bob walks east and
//!   back through the dug exits while Alice watches the announcements.
//! - `WHO` lists all three with well-formed idle/conn columns and a conn
//!   time that has really advanced (>= the 3 s the slowtest step took).
//! - Alice QUITs cleanly (farewell + close + room announcement); Bob drops
//!   his transport without QUIT (link-dead announcement), reconnects
//!   within the grace ("has reconnected."), and can still talk.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_core::GOD;
use mush_session::{SessionConfig, SessionManager};
use russh::ChannelMsg;

use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use mushd::accounts::{Accounts, hash_password};
use mushd::ipguard::IpGuard;
use mushd::ssh_bridge::{SshBridge, start_listener};
use mushd::world_task;

const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const GM_PASSWORD: &str = "gm-secret-1";

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mushd-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Boot the full production wiring on an ephemeral port. The `boss` account
/// is seeded and linked to God (`#1`) so a staff connection can drive
/// `@dig` and `@slowtest` over the same SSH front door as everyone else.
async fn boot_server(dir: &std::path::Path) -> SocketAddr {
    std::fs::create_dir_all(dir).unwrap();
    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: Duration::from_secs(120),
        ..SessionConfig::default()
    }));
    let (world, _join) = world_task::spawn(world_task::boot_engine(), Arc::clone(&sessions));
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    store
        .create_account(AccountRec {
            name: "boss".to_string(),
            argon2_hash: hash_password(GM_PASSWORD).unwrap(),
            created: 0,
            characters: vec![CharacterRef {
                id: GOD,
                name: "God".to_string(),
            }],
        })
        .await
        .unwrap();
    let accounts = Arc::new(Accounts::new(store, world.clone(), 3));
    let bridge = Arc::new(SshBridge::new(
        accounts,
        Arc::clone(&sessions),
        world,
        Arc::new(IpGuard::new()),
    ));
    start_listener(
        "127.0.0.1:0".parse().unwrap(),
        dir,
        "Welcome to the e2e MUSH!".to_string(),
        bridge,
    )
    .await
    .expect("listener")
    .addr()
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

/// Drop ANSI CSI sequences so assertions match what a player reads.
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

/// One player's terminal: an SSH channel plus everything it has printed.
struct Term {
    channel: russh::Channel<russh::client::Msg>,
    seen: String,
}

impl Term {
    /// Byte offset of "now" in the raw stream, for [`Term::expect_after`]
    /// when the needle may already occur earlier in the scrollback.
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

/// `ssh play@addr` with a PTY + shell, landing on the welcome screen.
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

/// Parse a rendered WHO duration token (`0s`, `12m`, `3h`, `2d`) to seconds.
fn duration_secs(tok: &str) -> Option<u64> {
    let (num, unit) = tok.split_at(tok.len().checked_sub(1)?);
    let n: u64 = num.parse().ok()?;
    match unit {
        "s" => Some(n),
        "m" => Some(n * 60),
        "h" => Some(n * 3600),
        "d" => Some(n * 86_400),
        _ => None,
    }
}

/// Find `name`'s WHO row in the rendered block and return (idle, conn) in
/// seconds.
fn who_row(block: &str, name: &str) -> (u64, u64) {
    for line in block.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.first() == Some(&name) {
            let idle = duration_secs(toks.get(1).copied().unwrap_or(""))
                .unwrap_or_else(|| panic!("bad idle column in WHO row {line:?}"));
            let conn = duration_secs(toks.get(2).copied().unwrap_or(""))
                .unwrap_or_else(|| panic!("bad conn column in WHO row {line:?}"));
            return (idle, conn);
        }
    }
    panic!("no WHO row for {name:?} in:\n{block}");
}

#[tokio::test(flavor = "multi_thread")]
async fn m1_exit_criterion_full_journey() {
    let dir = scratch_dir("journey");
    let addr = boot_server(&dir).await;

    // --- God connects through the same front door as everyone else. ------
    let (god_handle, mut god) = play_client(addr).await;
    god.expect("Welcome to the e2e MUSH!").await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God (account boss, 1 connection).")
        .await;
    god.expect("Limbo(#0)").await;

    // --- Alice and Bob create accounts and land in the shared room. ------
    let (alice_handle, mut alice) = play_client(addr).await;
    alice.expect("connect <account> <password>").await;
    alice.send_line("create Alice secret123").await;
    alice.expect("Account created.").await;
    alice
        .expect("Connected as Alice (account Alice, 1 connection).")
        .await;
    alice.expect("Limbo(#0)").await;
    god.expect("Alice has connected.").await;

    let (_bob_handle, mut bob) = play_client(addr).await;
    bob.send_line("create Bob hunter42").await;
    bob.expect("Connected as Bob (account Bob, 1 connection).")
        .await;
    bob.expect("Limbo(#0)").await;
    alice.expect("Bob has connected.").await;

    // --- say fans out room-wide (speaker included, third person). --------
    alice.send_line("say Hello from the other side").await;
    bob.expect("Alice says, \"Hello from the other side\"")
        .await;
    alice
        .expect("Alice says, \"Hello from the other side\"")
        .await;

    // --- @slowtest: the typed async re-entry seam, over real SSH. --------
    // The completion lands ~3s later; a think sent right behind it must
    // come back FIRST — the world loop never blocked on the fake call.
    god.send_line("@slowtest").await;
    god.expect("Thinking...").await;
    god.send_line("think interleaved-42").await;
    god.expect("interleaved-42").await;
    god.expect("SLOWTEST: fake async call").await;
    let stripped = strip_ansi(&god.seen);
    let think_at = stripped.find("interleaved-42").expect("think output");
    let slow_at = stripped.find("SLOWTEST:").expect("slowtest output");
    assert!(
        think_at < slow_at,
        "the interleaved think must land before the slowtest completion"
    );

    // --- God digs a room with exits both ways; Bob walks them. -----------
    god.send_line("@dig Vault = east;e, west;w").await;
    god.expect("Vault created with room number").await;
    god.expect("Opened exit east(").await;
    god.expect("Opened exit west(").await;

    bob.send_line("east").await;
    bob.expect("Vault(#").await;
    alice.expect("Bob goes east.").await;
    let bob_back = bob.mark();
    bob.send_line("west").await;
    bob.expect_after(bob_back, "Limbo(#0)").await;
    alice.expect("Bob has arrived.").await;

    // --- WHO: all three, with real idle/conn columns. --------------------
    let who_mark = bob.mark();
    bob.send_line("WHO").await;
    bob.expect_after(who_mark, "3 players connected.").await;
    let block = strip_ansi(&bob.seen[who_mark..]);
    let (_alice_idle, alice_conn) = who_row(&block, "Alice");
    let (bob_idle, _bob_conn) = who_row(&block, "Bob");
    who_row(&block, "God");
    assert!(
        alice_conn >= 3,
        "Alice connected before the 3s slowtest; her conn time must show it (got {alice_conn}s)"
    );
    assert!(bob_idle <= 60, "Bob just typed; idle must be fresh");

    // --- Alice QUITs cleanly. ---------------------------------------------
    alice.send_line("QUIT").await;
    alice.expect("Goodbye.").await;
    alice.expect_close().await;
    bob.expect("Alice has disconnected.").await;
    let _ = alice_handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;

    // --- Bob drops his link without QUIT: link-dead, then reconnect. ------
    let god_linkdead = god.mark();
    let _ = _bob_handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
    drop(bob);
    drop(_bob_handle);
    god.expect_after(god_linkdead, "Bob has gone link-dead.")
        .await;

    // Reconnect within the grace: announced back, session alive again.
    let god_rejoin = god.mark();
    let (bob2_handle, mut bob2) = play_client(addr).await;
    bob2.send_line("connect Bob hunter42").await;
    bob2.expect("Connected as Bob (account Bob, 1 connection).")
        .await;
    bob2.expect("Limbo(#0)").await;
    god.expect_after(god_rejoin, "Bob has reconnected.").await;

    // The revived session still routes both ways.
    bob2.send_line("say back online").await;
    god.expect_after(god_rejoin, "Bob says, \"back online\"")
        .await;

    let _ = god_handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
    let _ = bob2_handle
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
    let _ = std::fs::remove_dir_all(&dir);
}

//! **The front door, through the split** (16b cut F.4; doc 16 §1, §3.1's *the
//! output path differs*; doc 10 R3).
//!
//! `crates/mushd/tests/sim_lenses/front_door.rs` is the in-process half. This is the shape
//! that is deployed: the SSH front door, the session layer and the account store
//! live on the **gateway**, the world and its account oracle live on the
//! **server**, and a character is created by a `mush-ipc`
//! `ServiceReq::CreatePlayer` crossing between them.
//!
//! That request is what cut F.4 changed. It used to carry a name and nothing
//! else, so the server made a Player its own account oracle never heard of, and
//! the gateway wrote the account document afterwards — a document only the *next*
//! boot would read. A player who registered on playtest night could not claim a
//! simulator slot at all. It now carries the account id, the server inserts
//! `character -> account` in the world-task turn that creates the Player, and the
//! claim works before the reply has crossed back.
//!
//! So this lens boots both halves against **one store with no player accounts**,
//! and two people register at the welcome screen, walk in, and claim a slot each.
//! Nothing is seeded and nothing is restarted.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_store::{InMemoryStore, ObjectStore};
use russh::ChannelMsg;
use tokio::sync::oneshot;

use mush_config::config::Config;
use mush_gateway::GatewayOptions;

use super::harness::ServerUnit;

const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const PASSWORD: &str = "front-door-1";

/// The world's transient authoritative refusal: a commit is in flight.
const COMMIT_PENDING: &str = "please retry in a moment";
/// How many times a line is re-sent on a transient before the test gives up.
const RETRY_BUDGET: usize = 40;

/// **The create's answer while the server half is not answering yet.**
///
/// This rig starts the two halves independently and then waits on ONE signal:
/// the gateway's `ssh_ready` oneshot, which fires when the SSH listener binds.
/// Nothing in it waits for the server half to admit anything, and the server's
/// own oneshot says only that its IPC socket is bound. A `create` that lands in
/// that window is refused before the account document is written (the name
/// pre-check is the first thing that crosses, `Accounts::create`), so the whole
/// registration can simply be re-sent. A rig that sent it once would sit out its
/// `STEP_TIMEOUT` instead. This is a rig fix, not a product change: nothing the
/// lens asserts about the front door is weakened.
const WORLD_NOT_UP: &str = "character creation failed: the world is shutting down";
/// How long the first create waits for the server half, in 200 ms steps.
const ADMISSION_TRIES: usize = 25;
/// The same window's answer to the login `create` runs for itself
/// (`ConnectErr::WorldGone`).
const CONNECT_NOT_UP: &str = "Connection failed: the game is shutting down";

// ---------------------------------------------------------------------------
// The split rig — `arena_story_split.rs`', with an EMPTY shared store
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("mush-front-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch dir");
    dir
}

/// Both halves' configuration. The TacSim layer is **on**, which is what puts the
/// simulator centre and its roster in the world — and, since cut F.4, what the
/// shipped dev configs do too.
fn test_config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ssh.listen = "127.0.0.1:0".to_string();
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    cfg.tacsim.enabled = true;
    cfg
}

/// The server unit over `store` (`harness.rs`): a real server on its own
/// thread + runtime, which the gateway dials. Nothing here restarts it; the
/// unit handle may be dropped (the server runs on until the process ends).
fn unit(store: Arc<dyn ObjectStore>) -> ServerUnit {
    ServerUnit::new(store, || test_config("server"))
}

async fn start_split() -> SocketAddr {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let unit = unit(Arc::clone(&store));
    unit.start().await;
    let (ssh_tx, ssh_rx) = oneshot::channel();
    let gw_store = Arc::clone(&store);
    let mut cfg = test_config("gateway");
    unit.aim(&mut cfg);
    std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        let _ = rt.block_on(mush_gateway::serve(
            cfg,
            GatewayOptions {
                store: Some(gw_store),
                ssh_ready: Some(ssh_tx),
                web_ready: None,
                siteban_refresh: None,
                give_up_after: None,
                generation: None,
            },
        ));
    });
    tokio::time::timeout(Duration::from_secs(60), ssh_rx)
        .await
        .expect("gateway bound SSH")
        .expect("ssh addr")
}

// ---------------------------------------------------------------------------
// The terminal — `arena_story_split.rs`'s, trimmed to what this lens types
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
        self.expect_one_of_after(mark, &[needle]).await;
    }

    async fn send_line(&mut self, line: &str) {
        let bytes = format!("{line}\r").into_bytes();
        self.channel.data(&bytes[..]).await.expect("send line");
    }

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

    /// Run one line and wait for `needle`, retrying the world's one transient.
    async fn say(&mut self, line: &str, needle: &str) {
        assert!(
            !line.contains(needle),
            "`{needle}` appears in `{line}`'s own echo; pick a needle from the ANSWER"
        );
        for _ in 0..RETRY_BUDGET {
            let mark = self.mark();
            self.send_line(line).await;
            match self
                .expect_one_of_after(mark, &[needle, COMMIT_PENDING])
                .await
            {
                0 => return,
                _ => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        }
        panic!("`{line}` was refused as transient {RETRY_BUDGET} times running");
    }

    async fn ask(&mut self, line: &str, needle: &str) -> String {
        let mark = self.mark();
        self.say(line, needle).await;
        strip_ansi(&self.seen[mark..])
    }

    /// Walk Limbo → the Welcome Hall → the Simulator Lobby by the exits' own
    /// aliases (`simulator`, never `sim`: cut F.4 took that word off the exit and
    /// left it to the verb).
    async fn walk_to_the_lobby(&mut self) {
        self.say("ooc", "Welcome Hall").await;
        // 16b cut D.5 (16c `B-11`): `l` looks, in the room every pilot crosses
        // on the way in. The hall's own name coming back is the proof.
        self.say("l", "Welcome Hall").await;
        self.say("simulator", "Simulator Lobby").await;
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

/// Make an account and its first character at the welcome screen — the create
/// that crosses the boundary.
async fn register(addr: SocketAddr, name: &str) -> (russh::client::Handle<AcceptAll>, Term) {
    let (handle, mut term) = play_client(addr).await;
    let connected = format!("Connected as {name}");
    // See `WORLD_NOT_UP`: the rig has no barrier against the server half still
    // booting, so the registration waits it out rather than failing the lens.
    let mut made = None;
    for _ in 0..ADMISSION_TRIES {
        let mark = term.mark();
        term.send_line(&format!("create {name} {PASSWORD}")).await;
        if term
            .expect_one_of_after(mark, &["Account created.", WORLD_NOT_UP])
            .await
            == 0
        {
            made = Some(mark);
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let mark = made.unwrap_or_else(|| panic!("the server half never answered `create {name}`"));
    // `create` logs the new character in itself. Only if the world came up
    // between the two halves of that one command does the rig type the login.
    if term
        .expect_one_of_after(mark, &[&connected, CONNECT_NOT_UP])
        .await
        == 0
    {
        return (handle, term);
    }
    for _ in 0..ADMISSION_TRIES {
        let mark = term.mark();
        term.send_line(&format!("connect {name} {PASSWORD}")).await;
        if term
            .expect_one_of_after(mark, &[&connected, CONNECT_NOT_UP])
            .await
            == 0
        {
            return (handle, term);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("the server half never admitted `{name}`");
}

// ---------------------------------------------------------------------------
// The lens
// ---------------------------------------------------------------------------

/// **Two accounts made after boot each claim a slot, across the boundary.**
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_accounts_made_after_boot_claim_a_slot_each_through_the_gateway() {
    let addr = start_split().await;

    // The gateway reserves each account document and asks the server to make the
    // Player; the account id rides on that request.
    let (_r, mut red) = register(addr, "Rowan").await;
    let (_b, mut blue) = register(addr, "Bly").await;

    red.walk_to_the_lobby().await;
    blue.walk_to_the_lobby().await;

    let watching = blue.mark();
    red.say("SIM TEAM red", "takes red slot 1").await;
    blue.expect_after(watching, "Rowan takes red slot 1").await;
    blue.say("SIM TEAM blue", "takes blue slot 1").await;

    let status = red.ask("SIM STATUS", "blue:").await;
    assert!(status.contains("selection"), "{status}");
    assert!(status.contains("Rowan"), "{status}");
    assert!(status.contains("Bly"), "{status}");

    for (who, seen) in [
        ("Rowan", strip_ansi(&red.seen)),
        ("Bly", strip_ansi(&blue.seen)),
    ] {
        assert!(
            !seen.contains("not mapped yet"),
            "{who} was told its account was not mapped:\n{seen}"
        );
        assert!(
            !seen.contains("no account on record"),
            "{who} was told it has no account on record:\n{seen}"
        );
    }
}

//! Doc 33 GG.1: the server as its own unit, seen from the gateway.
//!
//! The gateway no longer starts, stops or restarts the server; it dials the
//! unit's endpoint and holds every connection while the unit is away. Two
//! things that follow are proved here, over a real SSH client against the
//! [`ServerUnit`] rig:
//!
//! - **A boot with no server is a gap from the start.** A gateway started
//!   before its server unit (a box coming up out of order, a server unit that
//!   failed its first start) binds its listeners and serves the restarting
//!   refusal to whoever connects; when the unit comes up the gateway heals on
//!   its own and the same client gets in. Before GG.1 that gateway did not
//!   boot at all.
//! - **The give-up notice.** A server unit that stays down past its
//!   start-limit bound (Q-33-5: five starts in sixty seconds, so a gap past a
//!   minute means systemd has stopped restarting it) is announced to the held
//!   players in place of "restarting; hold on" — the server is down and staff
//!   have been paged — with input refused in the same words, `/healthz`
//!   reporting `server_ok: false`, and a new login refused with the same
//!   reason. The dialing never stops: the unit coming back heals everything,
//!   and the player who was held through it all plays on. The bound is
//!   shortened here through `GatewayOptions::give_up_after`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_config::accounts::hash_password;
use mush_config::config::Config;
use mush_core::GOD;
use mush_gateway::GatewayOptions;
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use russh::ChannelMsg;
use tokio::sync::oneshot;

use super::harness::ServerUnit;

const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const GM_PASSWORD: &str = "gm-secret-1";
/// The shortened give-up bound: long enough that "restarting" is read first,
/// short enough that the proof is not a minute of waiting.
const GIVE_UP_AFTER: Duration = Duration::from_millis(1_500);
/// The refusal a connect gets while the gateway holds for the unit.
const RESTARTING_REFUSAL: &str = "the server is restarting; hold on and try again in a moment";
/// ... and once the gateway has stopped expecting the unit back on its own.
const DOWN_REFUSAL: &str = "the server is down and staff have been paged";

fn scratch_dir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("mush-unit-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch dir");
    dir
}

/// Everything in memory, every listener ephemeral, no simulator.
fn test_config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ssh.listen = "127.0.0.1:0".to_string();
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    cfg
}

/// The gateway's config: the SSH listener plus a web listener, for `/healthz`.
fn gateway_config(unit: &ServerUnit) -> Config {
    let mut cfg = test_config("gateway");
    let mut web = toml::Table::new();
    web.insert(
        "listen".to_string(),
        toml::Value::String("127.0.0.1:0".to_string()),
    );
    cfg.web = toml::Value::Table(web);
    unit.aim(&mut cfg);
    cfg
}

/// The gateway's own KV store (accounts), seeded with the God wizard.
async fn seeded_gw_store() -> Arc<dyn ObjectStore> {
    let gw_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    gw_store
        .create_account(AccountRec {
            id: None,
            name: "boss".to_string(),
            argon2_hash: hash_password(GM_PASSWORD).expect("a hash"),
            created: 0,
            characters: vec![CharacterRef {
                id: GOD,
                name: "God".to_string(),
            }],
            ssh_keys: vec![],
            discord: None,
        })
        .await
        .expect("the staff account");
    gw_store
}

/// A unit over its own world store, started by the caller.
fn unit() -> ServerUnit {
    let server_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    ServerUnit::new(server_store, || test_config("server"))
}

/// Start the gateway (on its own thread/runtime, as the binary does) aimed at
/// `unit`, with `give_up_after` as the keeper's bound; return the bound SSH
/// and web addresses. The thread is detached: `serve` returns only on a
/// shutdown signal, which never comes in-test.
async fn start_gateway(
    unit: &ServerUnit,
    give_up_after: Option<Duration>,
) -> (SocketAddr, SocketAddr) {
    let gw_store = seeded_gw_store().await;
    let (ssh_tx, ssh_rx) = oneshot::channel();
    let (web_tx, web_rx) = oneshot::channel();
    let cfg = gateway_config(unit);
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
                web_ready: Some(web_tx),
                siteban_refresh: None,
                give_up_after,
                generation: None,
                ..Default::default()
            },
        ));
    });
    let ssh_addr = tokio::time::timeout(Duration::from_secs(60), ssh_rx)
        .await
        .expect("gateway bound SSH")
        .expect("ssh addr");
    let web_addr = tokio::time::timeout(Duration::from_secs(60), web_rx)
        .await
        .expect("gateway bound WEB")
        .expect("web addr");
    (ssh_addr, web_addr)
}

/// `GET /healthz`, as the operator's probe reads it.
async fn healthz(web: SocketAddr) -> serde_json::Value {
    let resp = reqwest::Client::new()
        .get(format!("http://{web}/healthz"))
        .send()
        .await
        .expect("/healthz answers");
    resp.json().await.expect("/healthz is JSON")
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
        assert_eq!(
            self.expect_one_of_after(mark, &[needle]).await,
            0,
            "one needle"
        );
    }

    async fn send_line(&mut self, line: &str) {
        let bytes = format!("{line}\r").into_bytes();
        self.channel.data(&bytes[..]).await.expect("send line");
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

    /// `connect` as the staff account; answer what the gateway said: `Ok(())`
    /// for a login, `Err(what it said)` for a `Connection failed:` line or
    /// the login rate limit.
    async fn try_connect(&mut self) -> Result<(), String> {
        let mark = self.mark();
        self.send_line(&format!("connect boss {GM_PASSWORD}")).await;
        match self
            .expect_one_of_after(
                mark,
                &[
                    "Connected as God",
                    "Connection failed:",
                    "You're doing that too fast.",
                ],
            )
            .await
        {
            0 => {
                self.expect_after(mark, "Limbo #0").await;
                Ok(())
            }
            _ => Err(strip_ansi(&self.seen[mark..])),
        }
    }

    /// `connect` until the gateway lets the staff account in, through the
    /// gap's refusals and the login rate limit. Panics past `STEP_TIMEOUT`.
    async fn connect_when_admitted(&mut self) {
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        loop {
            if self.try_connect().await.is_ok() {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the gateway never admitted the login; seen: {:?}",
                self.seen
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
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
        .request_pty(true, "xterm-256color", 120, 40, 0, 0, &[])
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

/// A gateway started before its server unit boots into the gap, refuses the
/// login with the restarting line, and heals on its own when the unit comes
/// up — the same client, still connected, gets in.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_gateway_started_before_its_server_serves_the_gap_then_heals() {
    let unit = unit().at(ServerUnit::reserve_endpoint());
    let (ssh_addr, web_addr) = start_gateway(&unit, None).await;
    assert_eq!(
        unit.boots(),
        0,
        "the unit has not booted; the gateway is up anyway"
    );

    let (_g, mut god) = play_client(ssh_addr).await;
    god.expect_after(0, "connect <account> <password>").await;
    let refused = god
        .try_connect()
        .await
        .expect_err("no server: the login is refused, not admitted");
    assert!(
        refused.contains(RESTARTING_REFUSAL),
        "the gap's own refusal names a restart, not a shutdown; seen: {refused:?}"
    );
    let health = healthz(web_addr).await;
    assert_eq!(
        health["server_ok"], true,
        "a gap inside the start-limit bound is not a server that is down: {health}"
    );

    // The unit comes up. Nothing is asked of the gateway or the client.
    unit.start().await;
    god.connect_when_admitted().await;

    // ... and the world is the one the unit booted.
    let mark = god.mark();
    god.send_line("say the unit came up").await;
    god.expect_after(mark, "God says, \"the unit came up\"")
        .await;

    drop(god);
}

/// A server unit that stays down past the gateway's bound is announced to the
/// held player as down with staff paged, in place of "restarting"; input and
/// new logins are refused in the same words and `/healthz` says `server_ok:
/// false`; the unit coming back heals all of it and the held player plays on.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_server_down_past_the_bound_is_announced_and_the_heal_clears_it() {
    let unit = unit();
    unit.start().await;
    let (ssh_addr, web_addr) = start_gateway(&unit, Some(GIVE_UP_AFTER)).await;

    let (_g, mut god) = play_client(ssh_addr).await;
    god.connect_when_admitted().await;
    assert_eq!(healthz(web_addr).await["server_ok"], true);

    // The unit goes down and stays down.
    let down_mark = god.mark();
    unit.stop().await;
    god.expect_after(down_mark, "restarting").await;
    // Past the bound: the give-up notice, in the gap's own voice.
    god.expect_after(down_mark, "staff have been paged").await;
    let seen = strip_ansi(&god.seen[down_mark..]);
    assert!(
        seen.find("restarting") < seen.find("staff have been paged"),
        "restarting is said first, the give-up after the bound; seen: {seen:?}"
    );

    // Input is refused, not dropped, and says why in the same words.
    let input_mark = god.mark();
    god.send_line("say anyone there").await;
    god.expect_after(
        input_mark,
        "The server is down and staff have been paged; your input was not sent.",
    )
    .await;

    // The operator's probe and a new login agree.
    let health = healthz(web_addr).await;
    assert_eq!(health["server_ok"], false, "{health}");
    assert_eq!(health["ok"], false, "{health}");
    let (_l, mut late) = play_client(ssh_addr).await;
    late.expect_after(0, "connect <account> <password>").await;
    let refused = late
        .try_connect()
        .await
        .expect_err("a login while the server is down is refused");
    assert!(
        refused.contains(DOWN_REFUSAL),
        "the refusal names the give-up, not a restart; seen: {refused:?}"
    );

    // The unit comes back (an operator's `systemctl start`): the held player
    // is reconnected into the reloaded world and the probe clears.
    let heal_mark = god.mark();
    unit.start().await;
    god.expect_after(heal_mark, "Reconnected").await;
    god.expect_after(heal_mark, "Limbo #0").await;
    assert_eq!(healthz(web_addr).await["server_ok"], true);
    let mark = god.mark();
    god.send_line("say back again").await;
    god.expect_after(mark, "God says, \"back again\"").await;
    // The refused late login gets in now, too.
    late.connect_when_admitted().await;

    // The unit's second boot is the one that healed it: nothing else did.
    assert_eq!(unit.boots(), 2);
    drop(god);
    drop(late);
}

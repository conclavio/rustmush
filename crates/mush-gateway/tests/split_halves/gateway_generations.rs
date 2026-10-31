//! Doc 33 §4.4–§4.7, cut GG.4: the two-generation drain, in one process.
//!
//! The swap (§4.7) as the gateway sees it: generation `a` serves; `b` is
//! started on the SAME public ports (`SO_REUSEPORT`, bound through
//! `mush_drain::listen`) and its own trusted socket beside `a`'s; the swap
//! script re-points the trusted symlink at `b` and sends `a` `SIGUSR1`. Proved
//! here against the [`ServerUnit`] rig with two [`GatewayUnit`]s, real SSH
//! clients (public and through the doorway's preamble on the trusted socket)
//! and a real web socket, with a drain of seconds instead of hours:
//!
//! - **The drain holds every session and warns each, in order, through its
//!   own transport.** God on the public port, Alice through the trusted
//!   socket and Alice's web tab, all on `a`, see the two scaled warnings as
//!   `Sys` lines in that order; nobody is cut before the deadline.
//! - **New connections land on `b`.** From the moment `a` drains its
//!   listeners are closed: a new public SSH login and a new trusted session
//!   through the re-pointed symlink are `b`'s and play on after `a` exits.
//!   Under `net.ipv4.tcp_migrate_req = 1` a burst of connections opened as
//!   `a` closes all get an SSH banner (the kernel moved the queued ones);
//!   on a runner where the sysctl is off or unreadable that check is
//!   skipped with a line saying so.
//! - **The deadline closes `a` its transports' own ways.** SSH is told the
//!   reason and its channel ends; web gets a 1012 close frame with the
//!   reason, reconnects by itself with its resume point through the same
//!   port, lands on `b` and is given a fresh attach (`resumed { replayed:
//!   false }`, Q-33-4). `a` exits 0 and leaves its marker.
//! - **A restarted `a` that finds its marker reports ready and exits 0**
//!   (what `Restart=on-failure` would otherwise bring back beside `b`).
//! - **An empty generation exits as soon as it is drained**, marker left.
//! - **A finish (`SIGTERM`) is one warning, the grace, the close**, and
//!   leaves no marker.
//!
//! The refused third generation and `--force` are the swap script's
//! (`scripts/tests/gateway-swap.sh`); `--check-config`'s refusal of a
//! symlinked trusted socket is `mush_config::preflight`'s unit test and
//! `gateway_generations_unit`'s process test.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use mush_config::config::Config;
use mush_config::generation::Generation;
use mush_drain::Warning;
use mush_drain::lifecycle::DrainConfig;
use mush_gateway::drain::CLOSE_REASON;
use mush_proto::{ClientCommand, Envelope, OutputEvent};
use mush_sshdoorway_proto::{DoorwayMsg, IngressMsg, read_msg, write_msg};
use mush_store::ObjectStore;
use mush_testkit::text_of;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tokio::net::{TcpStream, UnixStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use super::harness::{GatewaySpec, GatewayUnit, ServerUnit};
use super::two_gateways::{
    AcceptAll, GM_PASSWORD, STEP_TIMEOUT, Term, booted_unit, gateway_config, god_on, play_client,
    seeded_gw_store,
};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// The scaled drain: `deadline` seconds, a warning at each of `remaining`
/// seconds before it, a one-second finish grace, and a message whose label
/// the stories look for.
fn short_drain(deadline: u64, remaining: &[u64]) -> DrainConfig {
    DrainConfig {
        deadline: Duration::from_secs(deadline),
        warnings: remaining
            .iter()
            .map(|s| Warning {
                remaining: Duration::from_secs(*s),
                label: format!("{s} seconds"),
            })
            .collect(),
        message: "DRAIN: this gateway is restarting; reconnect within {remaining}.".to_string(),
        grace: Duration::from_secs(1),
    }
}

fn scratch_dir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // Short: a Unix socket path is bounded (108 bytes), and the trusted
    // sockets live here.
    let dir = std::env::temp_dir().join(format!("mush-gg-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch dir");
    dir
}

/// One server unit, the gateways' shared account store, and the directory the
/// generations' state and trusted sockets live in.
struct Rig {
    unit: ServerUnit,
    store: Arc<dyn ObjectStore>,
    dir: PathBuf,
}

impl Rig {
    async fn new(tag: &str) -> Rig {
        let dir = scratch_dir(tag);
        // systemd makes the unit's `StateDirectory`; the rig makes its own.
        std::fs::create_dir_all(dir.join("state")).expect("a state dir");
        Rig {
            unit: booted_unit().await,
            store: seeded_gw_store().await,
            dir,
        }
    }

    /// The configured `[ssh].trusted_socket`: the symlink the swap script
    /// re-points (doc 33 §4.4). The doorway dials this name.
    fn link(&self) -> PathBuf {
        self.dir.join("gw.sock")
    }

    /// The socket generation `g` binds beside the link.
    fn socket_of(&self, g: Generation) -> PathBuf {
        g.socket_path(&self.link())
    }

    /// The unit's `StateDirectory`, shared by both generations.
    fn state_dir(&self) -> PathBuf {
        self.dir.join("state")
    }

    fn marker(&self, g: Generation) -> PathBuf {
        self.state_dir().join(format!("draining-{}", g.letter()))
    }

    /// A gateway config with the web listener and the trusted socket. With
    /// `like`, the SAME public ports as that gateway's (the reuseport share
    /// of a swap); without, ephemeral ones.
    fn config(&self, like: Option<&GatewayUnit>) -> Config {
        let mut cfg = gateway_config(&self.unit);
        cfg.ssh.trusted_socket = self.link().display().to_string();
        let (ssh, web) = match like {
            Some(unit) => (unit.ssh_addr().to_string(), unit.web_addr().to_string()),
            None => ("127.0.0.1:0".to_string(), "127.0.0.1:0".to_string()),
        };
        cfg.ssh.listen = ssh;
        let mut table = toml::value::Table::new();
        table.insert("listen".to_string(), toml::Value::String(web));
        table.insert("auth_burst".to_string(), toml::Value::Integer(0));
        cfg.web = toml::Value::Table(table);
        cfg
    }

    fn spec(&self, g: Generation, drain: DrainConfig) -> GatewaySpec {
        GatewaySpec {
            generation: Some(g),
            state_dir: Some(self.state_dir()),
            drain: Some(drain),
            web: true,
            trusted_allow_own_group: true,
        }
    }

    async fn start(
        &self,
        g: Generation,
        like: Option<&GatewayUnit>,
        drain: DrainConfig,
    ) -> GatewayUnit {
        GatewayUnit::start_with(
            self.config(like),
            Arc::clone(&self.store),
            self.spec(g, drain),
        )
        .await
    }

    /// The swap script's re-point (doc 33 §4.4): a fresh temporary link,
    /// renamed over the configured name, so there is no moment without one.
    fn point_link_at(&self, g: Generation) {
        let tmp = self.dir.join("gw.sock.swap");
        let _ = std::fs::remove_file(&tmp);
        std::os::unix::fs::symlink(self.socket_of(g), &tmp).expect("a temporary link");
        std::fs::rename(&tmp, self.link()).expect("the link re-pointed");
    }
}

/// `/proc/sys/net/ipv4/tcp_migrate_req` is 1: the kernel migrates the
/// connections queued on a closing reuseport listener to its siblings.
fn migrate_req_on() -> bool {
    std::fs::read_to_string("/proc/sys/net/ipv4/tcp_migrate_req").is_ok_and(|s| s.trim() == "1")
}

// ---------------------------------------------------------------------------
// Clients
// ---------------------------------------------------------------------------

/// A player through the doorway: dial the configured link (which follows the
/// symlink at connect time), run the doorway's preamble (`Hello`, `Begin`),
/// then SSH over the same stream.
async fn trusted_client(link: &Path) -> (russh::client::Handle<AcceptAll>, Term) {
    let mut stream = UnixStream::connect(link)
        .await
        .expect("dial the trusted link");
    write_msg(
        &mut stream,
        &DoorwayMsg::Hello {
            peer: "198.51.100.7:40000".parse().expect("an address"),
            local: "203.0.113.1:22".parse().expect("an address"),
            conn: 1,
        },
    )
    .await
    .expect("hello");
    let open: IngressMsg = read_msg(&mut stream).await.expect("the answer to hello");
    assert_eq!(open, IngressMsg::Open);
    write_msg(
        &mut stream,
        &DoorwayMsg::Begin {
            user: "play".to_string(),
            attested_key: None,
        },
    )
    .await
    .expect("begin");
    let ready: IngressMsg = read_msg(&mut stream).await.expect("the answer to begin");
    assert_eq!(ready, IngressMsg::Ready);
    let config = Arc::new(russh::client::Config::default());
    let mut handle = russh::client::connect_stream(config, stream, AcceptAll)
        .await
        .expect("ssh over the trusted link");
    let auth = handle.authenticate_none("play").await.expect("auth-none");
    assert!(auth.success(), "play must be accepted with no auth");
    let channel = handle.channel_open_session().await.expect("open channel");
    channel
        .request_pty(true, "xterm-256color", 120, 40, 0, 0, &[])
        .await
        .expect("pty");
    channel.request_shell(true).await.expect("shell");
    (handle, Term::new(channel))
}

/// `create <name> <password>` at the prompt, then Limbo.
async fn create_at(term: &mut Term, name: &str, password: &str) {
    term.expect("connect <account> <password>").await;
    term.send_line(&format!("create {name} {password}")).await;
    term.expect(&format!(
        "Connected as {name} (account {name}, 1 connection)."
    ))
    .await;
    term.expect("Limbo #0").await;
}

/// `connect boss <password>` at the prompt: God, on whichever generation
/// answered.
async fn god_at(term: &mut Term) {
    term.expect("connect <account> <password>").await;
    term.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    term.expect("Connected as God").await;
    term.expect("Limbo #0").await;
}

/// `n` connections opened at once; how many were answered with an SSH banner.
/// With the drain closing one of two reuseport listeners underneath them,
/// every one is, under `tcp_migrate_req = 1`.
async fn banner_burst(addr: SocketAddr, n: usize) -> usize {
    let mut tasks = Vec::with_capacity(n);
    for _ in 0..n {
        tasks.push(tokio::spawn(async move {
            let Ok(Ok(mut stream)) =
                tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(addr)).await
            else {
                return false;
            };
            let mut buf = [0u8; 64];
            match tokio::time::timeout(Duration::from_secs(20), stream.read(&mut buf)).await {
                Ok(Ok(n)) if n > 0 => buf.starts_with(b"SSH-"),
                _ => false,
            }
        }));
    }
    let mut answered = 0;
    for task in tasks {
        if task.await.unwrap_or(false) {
            answered += 1;
        }
    }
    answered
}

async fn post_json(url: &str, body: Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(url)
        .json(&body)
        .send()
        .await
        .expect("post")
}

/// Log in over the REST surface; `(token, the first character's key)`.
async fn login(web: SocketAddr, account: &str, password: &str) -> (String, u64) {
    let url = format!("http://{web}/api/auth/login");
    let resp = post_json(&url, json!({"account": account, "password": password})).await;
    assert_eq!(resp.status(), 200, "login");
    let body: Value = resp.json().await.expect("json");
    (
        body["token"].as_str().expect("a token").to_string(),
        body["characters"][0]["key"].as_u64().expect("a key"),
    )
}

/// Open a WS with the token in the subprotocol (the SPA's path), optionally
/// resuming from `resume` (the SPA's reconnect).
async fn connect_ws(web: SocketAddr, token: &str, char_key: u64, resume: Option<u64>) -> Ws {
    let mut url = format!("ws://{web}/ws?char={char_key}");
    if let Some(seq) = resume {
        url.push_str(&format!("&resume={seq}"));
    }
    let mut req = url.into_client_request().expect("a request");
    req.headers_mut().insert(
        "sec-websocket-protocol",
        HeaderValue::from_str(&format!("rustmush.v1.{token}")).expect("a header"),
    );
    let (ws, _resp) = connect_async(req).await.expect("the web socket opens");
    ws
}

enum Frame {
    Env(Envelope),
    Closed(Option<CloseFrame>),
}

/// The next envelope or the close; answers the application-level ping.
async fn next_frame(ws: &mut Ws) -> Frame {
    loop {
        let msg = tokio::time::timeout(STEP_TIMEOUT, ws.next())
            .await
            .expect("timed out waiting for a frame")
            .expect("stream ended")
            .expect("ws error");
        match msg {
            Message::Text(text) => {
                let env: Envelope = serde_json::from_str(text.as_str()).expect("an envelope");
                if let OutputEvent::Ping { nonce } = &env.event {
                    let pong = ClientCommand::Pong { nonce: *nonce };
                    ws.send(Message::Text(
                        serde_json::to_string(&pong).expect("json").into(),
                    ))
                    .await
                    .expect("pong");
                    continue;
                }
                return Frame::Env(env);
            }
            Message::Close(frame) => return Frame::Closed(frame),
            _ => {}
        }
    }
}

/// Envelopes until one's text contains `needle` (a close first is a failure).
async fn expect_text(ws: &mut Ws, needle: &str) -> Envelope {
    let mut seen = Vec::new();
    loop {
        match next_frame(ws).await {
            Frame::Env(env) => {
                let text = text_of(&env.event);
                if text.contains(needle) {
                    return env;
                }
                seen.push(text);
                assert!(
                    seen.len() < 200,
                    "no envelope containing {needle:?} in {seen:?}"
                );
            }
            Frame::Closed(frame) => panic!("closed while waiting for {needle:?}: {frame:?}"),
        }
    }
}

/// Envelopes until the close frame.
async fn expect_close(ws: &mut Ws) -> CloseFrame {
    let mut seen = Vec::new();
    loop {
        match next_frame(ws).await {
            Frame::Env(env) => {
                seen.push(text_of(&env.event));
                assert!(seen.len() < 200, "no close in {seen:?}");
            }
            Frame::Closed(frame) => return frame.expect("a close frame with a code and reason"),
        }
    }
}

/// Envelopes until `resumed`; whether the server replayed.
async fn expect_resumed(ws: &mut Ws) -> bool {
    let mut seen = Vec::new();
    loop {
        match next_frame(ws).await {
            Frame::Env(env) => {
                if let OutputEvent::Resumed { replayed } = env.event {
                    return replayed;
                }
                seen.push(text_of(&env.event));
                assert!(seen.len() < 200, "no resumed in {seen:?}");
            }
            Frame::Closed(frame) => panic!("closed while waiting for resumed: {frame:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// The swap
// ---------------------------------------------------------------------------

/// Generation `a` drains while `b` serves: every held session is warned in
/// order on its own transport and closed at the deadline its transport's
/// way; new connections are `b`'s; the web tab reconnects onto `b`; `a` exits
/// 0 with its marker and, restarted, refuses to serve.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_drains_while_b_serves_and_every_session_is_warned_then_moved() {
    let rig = Rig::new("swap").await;
    // Warnings at 6 s and 3 s remaining of an 8 s drain: 2 s and 5 s in.
    let drain = short_drain(8, &[6, 3]);

    // --- a serves alone: God on the public port, Alice through the trusted
    // link (pointed at a, as the swap script left it), Alice's web tab. ---
    let a = rig.start(Generation::A, None, drain.clone()).await;
    assert!(
        rig.socket_of(Generation::A).exists(),
        "a bound its own trusted socket"
    );
    assert!(!rig.link().exists(), "no gateway binds the configured name");
    rig.point_link_at(Generation::A);
    let (_g, mut god) = god_on(a.ssh_addr()).await;
    let (_t, mut alice) = trusted_client(&rig.link()).await;
    create_at(&mut alice, "Alice", "secret123").await;
    god.expect("Alice has connected.").await;
    let (token, key) = login(a.web_addr(), "Alice", "secret123").await;
    let mut tab = connect_ws(a.web_addr(), &token, key, None).await;
    let mut last_seq = expect_text(&mut tab, "Limbo").await.seq;

    // --- b joins on the same ports (reuseport) with its own trusted socket;
    // the swap script re-points the link. ---
    let b = rig.start(Generation::B, Some(&a), drain).await;
    assert_eq!(b.ssh_addr(), a.ssh_addr(), "one SSH port, two generations");
    assert_eq!(b.web_addr(), a.web_addr(), "one web port, two generations");
    assert!(
        rig.socket_of(Generation::B).exists(),
        "b bound its own trusted socket"
    );
    rig.point_link_at(Generation::B);

    // --- SIGUSR1 to a. ---
    let god_mark = god.mark();
    let alice_mark = alice.mark();
    a.drain();
    let burst = if migrate_req_on() {
        Some(tokio::spawn(banner_burst(a.ssh_addr(), 16)))
    } else {
        eprintln!(
            "skipping the queued-connection migration check: net.ipv4.tcp_migrate_req is not 1 on this runner"
        );
        None
    };

    // --- The warnings, in order, on each transport. ---
    god.expect_after(god_mark, "reconnect within 6 seconds")
        .await;
    god.expect_after(god_mark, "reconnect within 3 seconds")
        .await;
    let seen = god.since(god_mark);
    assert!(
        seen.find("6 seconds") < seen.find("3 seconds"),
        "public SSH saw the warnings in order: {seen:?}"
    );
    alice
        .expect_after(alice_mark, "reconnect within 6 seconds")
        .await;
    alice
        .expect_after(alice_mark, "reconnect within 3 seconds")
        .await;
    let seen = alice.since(alice_mark);
    assert!(
        seen.find("6 seconds") < seen.find("3 seconds"),
        "trusted SSH saw the warnings in order: {seen:?}"
    );
    last_seq = expect_text(&mut tab, "reconnect within 6 seconds")
        .await
        .seq
        .max(last_seq);
    last_seq = expect_text(&mut tab, "reconnect within 3 seconds")
        .await
        .seq
        .max(last_seq);

    // --- Meanwhile new connections are b's: a new public login, and a new
    // trusted session through the re-pointed link. ---
    let (_bob_handle, mut bob) = play_client(a.ssh_addr()).await;
    create_at(&mut bob, "Bob", "secret123").await;
    let (_g2, mut god_b) = trusted_client(&rig.link()).await;
    god_at(&mut god_b).await;
    if let Some(burst) = burst {
        let answered = burst.await.expect("the burst task");
        assert_eq!(
            answered, 16,
            "every connection opened as a closed its listener was answered"
        );
        eprintln!(
            "queued-connection migration: {answered}/16 connections answered across the close"
        );
    }

    // --- The deadline: a closes its sessions their transports' own ways. ---
    god.expect_after(god_mark, CLOSE_REASON).await;
    god.expect_close().await;
    alice.expect_after(alice_mark, CLOSE_REASON).await;
    alice.expect_close().await;
    let frame = expect_close(&mut tab).await;
    assert_eq!(u16::from(frame.code), 1012, "{frame:?}");
    assert_eq!(frame.reason.to_string(), CLOSE_REASON);
    a.wait_exit(Duration::from_secs(60))
        .await
        .expect("a exits 0 at the deadline");
    assert!(rig.marker(Generation::A).exists(), "a left its marker");
    assert!(!rig.marker(Generation::B).exists());

    // --- The web tab reconnects by itself with its resume point, through
    // the same port, onto b, which gives it a fresh attach (Q-33-4). ---
    let mut tab = connect_ws(a.web_addr(), &token, key, Some(last_seq)).await;
    assert!(
        !expect_resumed(&mut tab).await,
        "b had nothing to replay: a fresh attach"
    );
    expect_text(&mut tab, "Limbo").await;

    // --- b plays on with everyone who landed on it. ---
    bob.say_and_hear("still here on b").await;
    god_b.say_and_hear("the doorway follows the link").await;
    god_b.expect("Bob says, \"still here on b\"").await;

    // --- a, restarted by its unit, finds its marker: ready, exit 0. ---
    match GatewayUnit::try_start_with(
        rig.config(Some(&b)),
        Arc::clone(&rig.store),
        rig.spec(Generation::A, short_drain(8, &[])),
    )
    .await
    {
        Err(Ok(())) => {}
        Err(Err(e)) => panic!("a restarted a with its marker failed instead of exiting 0: {e}"),
        Ok(_) => panic!("a restarted a with its marker served beside b"),
    }
    assert!(
        rig.marker(Generation::A).exists(),
        "the marker is the swap script's to clear"
    );

    b.stop().await;
}

/// A generation with no session exits the moment it is drained, marker left.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_empty_generation_exits_as_soon_as_it_is_drained() {
    let rig = Rig::new("empty").await;
    let a = rig
        .start(Generation::A, None, short_drain(600, &[300]))
        .await;
    a.drain();
    a.wait_exit(Duration::from_secs(30))
        .await
        .expect("an empty generation exits 0 at once");
    assert!(rig.marker(Generation::A).exists());
}

/// `SIGTERM` finishes: one warning, the grace, the close; no marker, so the
/// unit's restart brings the generation back into service.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_finish_warns_once_closes_after_the_grace_and_leaves_no_marker() {
    let rig = Rig::new("finish").await;
    let a = rig
        .start(Generation::A, None, short_drain(600, &[300]))
        .await;
    let (_g, mut god) = god_on(a.ssh_addr()).await;
    let mark = god.mark();
    a.finish();
    god.expect_after(mark, "reconnect within 1s").await;
    god.expect_after(mark, CLOSE_REASON).await;
    god.expect_close().await;
    a.wait_exit(Duration::from_secs(30))
        .await
        .expect("a finished generation exits 0");
    assert!(
        !rig.marker(Generation::A).exists(),
        "a finish leaves no marker"
    );
}

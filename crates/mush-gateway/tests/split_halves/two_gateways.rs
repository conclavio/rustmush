//! Doc 33 §4.3, cut GG.2: one server, two gateways.
//!
//! Until GG.2 the server accepted ONE gateway and exited when its link ended;
//! a gateway restart bounced the server through its unit. The server now runs
//! an accept loop: every gateway is a link of its own, with its own reader,
//! writer, `Hello`, negotiated version and metrics, and the connection map is
//! keyed by (link, connection). Proved here over real SSH clients against the
//! [`ServerUnit`] rig and two [`GatewayUnit`]s dialing it:
//!
//! - **Two gateways of two generations share one world.** Alice on gateway
//!   `a` and God on gateway `b` hear each other (output addressed to a
//!   character reaches every link); `@stats` counts two links. Each gateway
//!   allocates SSH connection ids from 1, and generation `b`'s bit is what
//!   keeps the two id spaces apart (`--generation`, Q-33-1).
//! - **A link's EOF detaches that link's connections only.** Gateway `a`
//!   stops the way a crash or `systemctl stop` does; God on `b` sees Alice go
//!   link-dead and then, after the server's own grace, disconnect; God plays
//!   on; the server never exits; a new gateway `a` dials in and Alice logs
//!   back in through it: the operator-visible change of this cut, a gateway
//!   restart no longer bounces the server.
//! - **Output addressed to a connection reaches its owning link only**, and a
//!   second link's `ReAnnounce` and requests leave the first link untouched:
//!   a hand-driven link `b` re-announces a held session of God's beside the
//!   real gateway `a` God is playing on, is answered its own barrier and its
//!   own room, sees God's `say` (a character) and never God's `look` (a
//!   connection), and its EOF costs God nothing.
//! - **A server-side close crosses links.** God on `b` `@boot`s Alice on `a`:
//!   the server's `CloseSession` reaches gateway `a`, Alice's channel closes,
//!   and she logs back in there.
//! - **A duplicate connection id across links is refused loudly.** Two
//!   gateways started WITHOUT `--generation` hand the server the same id; the
//!   second announcement is refused (`IPC-DUPLICATE-CONN-ID` in the server
//!   log, the newer connection closed on its own link) and the owner keeps
//!   playing on the first.
//!
//! The SSH client helpers here (`Term`, `play_client`, `god_on`, ...) are
//! `pub(super)`: the two-generation story (`gateway_generations`, doc 33
//! GG.4) drives the same clients against the same rig.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mush_config::accounts::hash_password;
use mush_config::config::Config;
use mush_config::generation::Generation;
use mush_core::GOD;
use mush_ipc::{
    AttachRec, DetachCause, FrameReader, GatewayInput, GatewayRequest, IPC_MIN_PROTO, IPC_VERSION,
    SendTarget, ServiceReq, ServiceResp, ToGateway, ToServer, write_frame,
};
use mush_proto::SmolStr;
use mush_server::IPC_DUPLICATE_CONN_ID_MARKER;
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use russh::ChannelMsg;

use super::harness::{GatewayUnit, ServerUnit};

pub(super) const STEP_TIMEOUT: Duration = Duration::from_secs(60);
pub(super) const GM_PASSWORD: &str = "gm-secret-1";
/// The server's own link-dead grace (`[game].linkdead_grace_s`): short, so the
/// server-side reaper for a dead link's actors is seen inside the proof.
const SERVER_LINKDEAD_GRACE_S: u64 = 2;

pub(super) fn scratch_dir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("mush-2gw-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch dir");
    dir
}

/// Everything in memory, every listener ephemeral, no simulator.
pub(super) fn test_config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ssh.listen = "127.0.0.1:0".to_string();
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    cfg
}

/// A gateway's config, aimed at `unit`: SSH only.
pub(super) fn gateway_config(unit: &ServerUnit) -> Config {
    let mut cfg = test_config("gateway");
    unit.aim(&mut cfg);
    cfg
}

/// The gateways' shared KV store (accounts), seeded with the God wizard. Both
/// gateways of a swap read the same Redis; here they share the one map.
pub(super) async fn seeded_gw_store() -> Arc<dyn ObjectStore> {
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

/// A booted unit over its own world store, with the short server-side grace.
pub(super) async fn booted_unit() -> ServerUnit {
    let server_store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let unit = ServerUnit::new(server_store, || {
        let mut cfg = test_config("server");
        cfg.game.linkdead_grace_s = SERVER_LINKDEAD_GRACE_S;
        cfg
    });
    unit.start().await;
    unit
}

async fn gateway(
    unit: &ServerUnit,
    store: &Arc<dyn ObjectStore>,
    generation: Option<Generation>,
) -> GatewayUnit {
    GatewayUnit::start(gateway_config(unit), Arc::clone(store), generation).await
}

pub(super) struct AcceptAll;

impl russh::client::Handler for AcceptAll {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

pub(super) fn strip_ansi(s: &str) -> String {
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

pub(super) struct Term {
    channel: russh::Channel<russh::client::Msg>,
    seen: String,
}

impl Term {
    /// A terminal over an opened channel (a shell with a pty requested).
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(super) fn new(channel: russh::Channel<russh::client::Msg>) -> Term {
        Term {
            channel,
            seen: String::new(),
        }
    }

    pub(super) fn mark(&self) -> usize {
        self.seen.len()
    }

    /// Everything seen since `mark`, ANSI stripped.
    pub(super) fn since(&self, mark: usize) -> String {
        strip_ansi(&self.seen[mark..])
    }

    pub(super) async fn expect(&mut self, needle: &str) {
        self.expect_after(0, needle).await;
    }

    pub(super) async fn expect_after(&mut self, mark: usize, needle: &str) {
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

    pub(super) async fn send_line(&mut self, line: &str) {
        let bytes = format!("{line}\r").into_bytes();
        self.channel.data(&bytes[..]).await.expect("send line");
    }

    /// `say` something and wait for the echo: the player is live, and every
    /// frame the server addressed to this connection before the say has
    /// arrived (one FIFO per link).
    pub(super) async fn say_and_hear(&mut self, text: &str) -> usize {
        let mark = self.mark();
        self.send_line(&format!("say {text}")).await;
        self.expect_after(mark, &format!(" says, \"{text}\"")).await;
        mark
    }

    pub(super) async fn expect_close(&mut self) {
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

pub(super) async fn play_client(addr: SocketAddr) -> (russh::client::Handle<AcceptAll>, Term) {
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

/// God logs in on `addr` and is shown Limbo.
pub(super) async fn god_on(addr: SocketAddr) -> (russh::client::Handle<AcceptAll>, Term) {
    let (handle, mut god) = play_client(addr).await;
    god.expect("connect <account> <password>").await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God (account boss, 1 connection).")
        .await;
    god.expect("Limbo #0").await;
    (handle, god)
}

/// Alice creates her account on `addr` and is shown Limbo.
pub(super) async fn alice_creates_on(addr: SocketAddr) -> (russh::client::Handle<AcceptAll>, Term) {
    let (handle, mut alice) = play_client(addr).await;
    alice.expect("connect <account> <password>").await;
    alice.send_line("create Alice secret123").await;
    alice
        .expect("Connected as Alice (account Alice, 1 connection).")
        .await;
    alice.expect("Limbo #0").await;
    (handle, alice)
}

/// Alice logs back in on `addr` (her account exists in the shared store).
pub(super) async fn alice_returns_on(addr: SocketAddr) -> (russh::client::Handle<AcceptAll>, Term) {
    let (handle, mut alice) = play_client(addr).await;
    alice.expect("connect <account> <password>").await;
    alice.send_line("connect Alice secret123").await;
    alice
        .expect("Connected as Alice (account Alice, 1 connection).")
        .await;
    alice.expect("Limbo #0").await;
    (handle, alice)
}

/// `@stats` as God, answering the IPC boundary line.
async fn ipc_boundary_line(god: &mut Term) -> String {
    let mark = god.mark();
    god.send_line("@stats").await;
    god.expect_after(mark, "IPC boundary:").await;
    // The line ends at the high-water readout; wait for that too so the
    // link count is read off a whole line.
    god.expect_after(mark, "to_gw ").await;
    god.since(mark)
        .lines()
        .find(|l| l.contains("IPC boundary:"))
        .map(str::to_owned)
        .expect("the boundary line")
}

// ---------------------------------------------------------------------------
// Two real gateways, two generations
// ---------------------------------------------------------------------------

/// Two gateways of two generations share one world; one of them stopping
/// detaches its connections only, and a new one dials in while the server runs
/// on.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_gateways_share_one_world_and_one_leaving_costs_only_its_own_connections() {
    let unit = booted_unit().await;
    let store = seeded_gw_store().await;
    let gw_a = gateway(&unit, &store, Some(Generation::A)).await;
    let gw_b = gateway(&unit, &store, Some(Generation::B)).await;

    // --- God on b, Alice on a: both SSH ingresses hand out connection 1; the
    // generation bit keeps them apart (the duplicate story below is what
    // happens without it). ---
    let (_g, mut god) = god_on(gw_b.ssh_addr()).await;
    let line = ipc_boundary_line(&mut god).await;
    assert!(
        line.contains("2 gateway links;"),
        "two links attached: {line}"
    );
    let (_a, mut alice) = alice_creates_on(gw_a.ssh_addr()).await;
    god.expect("Alice has connected.").await;

    // --- Output addressed to a character reaches every link. ---
    alice.send_line("say hello from gateway a").await;
    god.expect("Alice says, \"hello from gateway a\"").await;
    god.send_line("say hello from gateway b").await;
    alice.expect("God says, \"hello from gateway b\"").await;

    // --- Gateway a crashes (or is `systemctl stop`ped): its link's EOF
    // detaches Alice only. God sees her go link-dead, the server's own reaper
    // disconnects her after the grace (her gateway's reaper died with it),
    // and God plays on through b. The server never exits. ---
    let mark = god.mark();
    gw_a.stop().await;
    drop(alice);
    god.expect_after(mark, "Alice has gone link-dead.").await;
    god.expect_after(mark, "Alice has disconnected.").await;
    god.say_and_hear("still here on gateway b").await;
    let line = ipc_boundary_line(&mut god).await;
    assert!(line.contains("1 gateway link;"), "one link left: {line}");
    assert_eq!(unit.boots(), 1, "the server unit never restarted");

    // --- The unit brings gateway a back: it dials the running server, and
    // Alice logs back in through it. Her old connection id is free again. ---
    let gw_a2 = gateway(&unit, &store, Some(Generation::A)).await;
    let mark = god.mark();
    let (_a2, mut alice) = alice_returns_on(gw_a2.ssh_addr()).await;
    god.expect_after(mark, "Alice has connected.").await;
    alice.send_line("say back through the new gateway").await;
    god.expect_after(mark, "Alice says, \"back through the new gateway\"")
        .await;
    let line = ipc_boundary_line(&mut god).await;
    assert!(line.contains("2 gateway links;"), "two links again: {line}");
    assert_eq!(
        unit.boots(),
        1,
        "a gateway restart no longer bounces the server"
    );
}

/// A server-side close crosses links: God on gateway b `@boot`s Alice on
/// gateway a; her channel there closes and she logs back in through it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_boot_from_one_gateway_closes_the_connection_on_the_other() {
    let unit = booted_unit().await;
    let store = seeded_gw_store().await;
    let gw_a = gateway(&unit, &store, Some(Generation::A)).await;
    let gw_b = gateway(&unit, &store, Some(Generation::B)).await;
    let (_g, mut god) = god_on(gw_b.ssh_addr()).await;
    let (_a, mut alice) = alice_creates_on(gw_a.ssh_addr()).await;
    god.expect("Alice has connected.").await;

    let mark = god.mark();
    god.send_line("@boot Alice").await;
    god.expect_after(mark, "@boot: Alice booted.").await;
    alice.expect_close().await;
    drop(alice);

    let mark = god.mark();
    let (_a2, mut alice) = alice_returns_on(gw_a.ssh_addr()).await;
    god.expect_after(mark, "Alice has connected.").await;
    alice.send_line("say booted and back").await;
    god.expect_after(mark, "Alice says, \"booted and back\"")
        .await;
}

// ---------------------------------------------------------------------------
// A hand-driven second link beside a real gateway
// ---------------------------------------------------------------------------

/// A hand-driven gateway's end of one link (the `ipc_window` shape).
struct Link {
    frames: FrameReader<mush_ipc::BoxRead>,
    writer: mush_ipc::BoxWrite,
}

impl Link {
    /// Connect, read the server's greeting, announce the current version.
    async fn connect(endpoint: &mush_ipc::IpcEndpoint) -> Link {
        let (reader, writer) = mush_ipc::connect(endpoint).await.expect("connect");
        let mut link = Link {
            frames: FrameReader::new(reader),
            writer,
        };
        link.read_until("the server's Hello", |f| {
            matches!(f, ToGateway::Hello { .. })
        })
        .await;
        link.send(&ToServer::Hello {
            proto: IPC_VERSION,
            min_proto: IPC_MIN_PROTO,
        })
        .await;
        link
    }

    async fn send(&mut self, msg: &ToServer) {
        write_frame(&mut self.writer, msg)
            .await
            .expect("the link accepts the frame");
    }

    /// Read frames until `pred` accepts one; everything read, the accepted
    /// frame last, comes back.
    async fn read_until(
        &mut self,
        what: &str,
        pred: impl Fn(&ToGateway) -> bool,
    ) -> Vec<ToGateway> {
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        let mut seen = Vec::new();
        loop {
            let frame = tokio::time::timeout_at(deadline, self.frames.read::<ToGateway>())
                .await
                .unwrap_or_else(|_| panic!("timed out waiting for {what}; seen: {seen:?}"))
                .expect("the link reads")
                .unwrap_or_else(|| {
                    panic!("the server closed the link before {what}; seen: {seen:?}")
                });
            let done = pred(&frame);
            seen.push(frame);
            if done {
                return seen;
            }
        }
    }
}

/// The connection id the hand-driven link announces: generation b's bit on an
/// SSH-range id, as a real `--generation b` gateway would allocate.
const B_CONN: u64 = Generation::CONN_ID_BIT | 7;

fn held_session(conn: u64) -> AttachRec {
    AttachRec {
        output_brackets: Default::default(),
        conn,
        actor: GOD,
        account: Some("boss".to_owned()),
        char_name: "God".to_owned(),
        caps_structured: false,
    }
}

fn mentions(frame: &ToGateway, text: &str) -> bool {
    matches!(frame, ToGateway::Deliver { .. }) && format!("{frame:?}").contains(text)
}

/// A delivery addressed to a connection other than `own`: output that belongs
/// to another link and must never be seen on this one.
fn addressed_to_another_connection(frame: &ToGateway, own: u64) -> bool {
    matches!(
        frame,
        ToGateway::Deliver {
            target: SendTarget::Conn(c),
            ..
        } if *c != own
    )
}

/// Output addressed to a connection reaches its owning link only; a second
/// link's `ReAnnounce`, requests and input leave the first link untouched; the
/// second link's EOF costs the first nothing.
///
/// What is addressed to a connection: the attach look (`Reply { id: 0, Room }`
/// the gateway asks for at login), a query's reply, `SetOutputBrackets`, a
/// prompt. A typed command's output goes to the actor, which is every
/// connection of the character on every link.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_link_sees_its_own_connections_output_and_every_characters() {
    let unit = booted_unit().await;
    let store = seeded_gw_store().await;
    let gw_a = gateway(&unit, &store, None).await;

    // --- Link b dials the server and re-announces a session of God's it
    // holds, as a gateway does after a reconnect; it is answered its own
    // barrier and shown the room (God's first connection: the re-attach
    // looks), and its brackets are restored on its own connection. ---
    let mut b = Link::connect(&unit.endpoint()).await;
    b.send(&ToServer::ReAnnounce(vec![held_session(B_CONN)]))
        .await;
    b.send(&ToServer::Request {
        id: 1,
        req: ServiceReq::Barrier,
    })
    .await;
    let up_to_barrier = b
        .read_until("the barrier reply", |f| {
            matches!(
                f,
                ToGateway::Reply {
                    id: 1,
                    resp: ServiceResp::Barrier
                }
            )
        })
        .await;
    assert!(
        up_to_barrier.iter().any(|f| mentions(f, "Limbo")),
        "the re-attached session is shown its room; seen: {up_to_barrier:?}"
    );
    assert!(
        up_to_barrier.iter().any(|f| matches!(
            f,
            ToGateway::SetOutputBrackets { conn, .. } if *conn == B_CONN
        )),
        "its brackets are restored on its own connection; seen: {up_to_barrier:?}"
    );

    // --- God logs in on gateway a beside link b's held session: gateway a
    // correlates its own barrier and readiness requests under its own ids
    // (link b's id 1 is already answered and never confused with a's), and
    // God's attach look on a is addressed to a's connection: it never reaches
    // link b, while God's `say` (a character's) does. ---
    let (_g, mut god) = god_on(gw_a.ssh_addr()).await;
    god.say_and_hear("over to link b").await;
    let up_to_say = b
        .read_until("the say on link b", |f| mentions(f, "over to link b"))
        .await;
    assert!(
        !up_to_say
            .iter()
            .any(|f| addressed_to_another_connection(f, B_CONN)),
        "nothing addressed to gateway a's connection reaches link b; seen: {up_to_say:?}"
    );

    // --- A query from link b is answered to b's connection only: the room
    // comes back on b, and God on a, whose next echo bounds the window (one
    // FIFO per link), never sees it. ---
    let mark = god.mark();
    let query = GatewayRequest::new(
        0x77,
        1,
        GatewayInput::Query {
            conn: B_CONN,
            actor: GOD,
            id: 5,
            name: SmolStr::new_static("room"),
            args: serde_json::json!({}),
        },
    );
    b.send(&ToServer::Input(query.clone())).await;
    b.send(&ToServer::GatewayReleased { request: query.uid })
        .await;
    let up_to_reply = b
        .read_until("the query reply on link b", |f| {
            matches!(
                f,
                ToGateway::Deliver {
                    target: SendTarget::Conn(c),
                    ..
                } if *c == B_CONN
            ) && mentions(f, "Reply { id: 5")
        })
        .await;
    assert!(
        up_to_reply.last().is_some_and(|f| mentions(f, "Limbo")),
        "the reply is the room; seen: {up_to_reply:?}"
    );
    god.say_and_hear("did the room stay on b").await;
    assert!(
        !god.since(mark).contains("Limbo"),
        "link b's room never reaches God on gateway a; seen: {:?}",
        god.since(mark)
    );

    // --- Input from link b runs as God under custody: the disposition comes
    // back to b, and the echo (a character's) reaches both b and God on a. ---
    let request = GatewayRequest::new(
        0x77,
        2,
        GatewayInput::Cmd {
            conn: B_CONN,
            actor: GOD,
            line: "say from link b".to_owned(),
        },
    );
    let mark = god.mark();
    b.send(&ToServer::Input(request.clone())).await;
    b.send(&ToServer::GatewayReleased {
        request: request.uid,
    })
    .await;
    let up_to_echo = b
        .read_until("the echo on link b", |f| mentions(f, "from link b"))
        .await;
    assert!(
        up_to_echo.iter().any(|f| matches!(
            f,
            ToGateway::RequestDisposition { request: uid, .. } if *uid == request.uid
        )),
        "the custody protocol answers link b's request on link b; seen: {up_to_echo:?}"
    );
    god.expect_after(mark, "God says, \"from link b\"").await;

    // --- Link b ends: its connection is detached, God (still on a) plays on,
    // and the server counts one link again. ---
    drop(b);
    god.say_and_hear("link b is gone and I am not").await;
    let line = ipc_boundary_line(&mut god).await;
    assert!(line.contains("1 gateway link;"), "one link left: {line}");
}

// ---------------------------------------------------------------------------
// The duplicate id
// ---------------------------------------------------------------------------

/// A `tracing` writer that keeps what the halves log, for the marker.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("the capture buffer")).into_owned()
    }
}

impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("the capture buffer")
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Capture {
        self.clone()
    }
}

/// Two gateways started without `--generation` allocate the same ids. The
/// server refuses the second announcement of an id loudly: the marker in its
/// log, the newer connection closed on its own link, the owner untouched.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_duplicate_connection_id_across_links_is_refused_loudly() {
    // The one subscriber of this process, before either half starts (nextest
    // runs one test per process).
    let capture = Capture::default();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .with_writer(capture.clone())
            .finish(),
    )
    .expect("the one subscriber of this binary");

    let unit = booted_unit().await;
    let store = seeded_gw_store().await;
    let gw_a = gateway(&unit, &store, None).await;
    let gw_b = gateway(&unit, &store, None).await;

    // God on a is connection 1 on link a.
    let (_g, mut god) = god_on(gw_a.ssh_addr()).await;

    // Alice on b is connection 1 on link b: the same id on a second live
    // link. Her announcement is refused and her connection closed; God's is
    // the owner and stays.
    let (_a, mut alice) = play_client(gw_b.ssh_addr()).await;
    alice.expect("connect <account> <password>").await;
    alice.send_line("create Alice secret123").await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !capture.text().contains(IPC_DUPLICATE_CONN_ID_MARKER) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the server never logged the duplicate; log:\n{}",
            capture.text()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // The server refused the id before the world saw it, so gateway b's own
    // readiness probe comes back "not bound" and the login is refused in
    // those words; the connection is back at its prompt and can only be
    // refused again until the generations are fixed.
    alice
        .expect_after(
            0,
            "Connection failed: the server did not admit this connection",
        )
        .await;
    drop(alice);
    let log = capture.text();
    let line = log
        .lines()
        .find(|l| l.contains(IPC_DUPLICATE_CONN_ID_MARKER))
        .expect("the marker line");
    assert!(line.contains("conn=1"), "the id is named: {line}");
    assert!(
        line.contains("generation=a"),
        "its generation is named: {line}"
    );
    assert!(
        line.contains("owner_link="),
        "the owning link is named beside the refused one: {line}"
    );

    // A connection already playing when its id is found to be a duplicate (a
    // gateway re-announcing what it holds) is closed by the server: the
    // newer link is told to detach it, as a boot.
    let mut c = Link::connect(&unit.endpoint()).await;
    c.send(&ToServer::ReAnnounce(vec![held_session(1)])).await;
    c.send(&ToServer::Request {
        id: 1,
        req: ServiceReq::Barrier,
    })
    .await;
    let up_to_barrier = c
        .read_until("the barrier reply on link c", |f| {
            matches!(
                f,
                ToGateway::Reply {
                    id: 1,
                    resp: ServiceResp::Barrier
                }
            )
        })
        .await;
    assert!(
        up_to_barrier.iter().any(|f| matches!(
            f,
            ToGateway::Detach {
                conn: 1,
                cause: DetachCause::Booted
            }
        )),
        "the re-announced duplicate is detached on its own link; seen: {up_to_barrier:?}"
    );
    assert_eq!(
        capture
            .text()
            .lines()
            .filter(|l| l.contains(IPC_DUPLICATE_CONN_ID_MARKER))
            .count(),
        2,
        "each refusal is a marker line"
    );

    // The owner plays on, on its own link, and every link is still attached:
    // a refused connection is not a refused gateway.
    god.say_and_hear("the owner keeps the id").await;
    let boundary = ipc_boundary_line(&mut god).await;
    assert!(
        boundary.contains("3 gateway links;"),
        "every link stays attached: {boundary}"
    );
    drop(c);
}

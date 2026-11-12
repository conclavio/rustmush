//! Doc 33 §4.1, cut GG.0: the one-step compatibility window at the server's end
//! of the boundary, against a REAL booted `mush-server` over a real localhost
//! socket — the two-halves shape this group's other IPC proofs use (two halves,
//! one socket, no OS-process flakiness).
//!
//! Written against the window, not against version numbers, since the bump to
//! 14 (doc 44 unit MW.24), the first inside the window:
//!
//! - A gateway **one step behind** (13 under a 14 server) announces itself and
//!   is served: the session it holds is re-attached and shown its room
//!   (output), its barrier and readiness probes are answered (requests), and
//!   its command under custody runs (input). This is the window's promise,
//!   and the case a deploy meets every time the server goes first.
//! - A gateway **at the server's version** announces itself and is served the
//!   same.
//! - A gateway **with no `ToServer::Hello`** is a version-12 one (no build
//!   before 13 had a Hello). The bootstrap rule served it while 12 was inside
//!   the window, which was version 13 only; now it is refused like any link
//!   below the window.
//! - A gateway **outside the window** (one step AHEAD, the step the
//!   one-direction window never covers, because the server upgrades first) is
//!   refused: the server
//!   ends the link without answering anything the gateway sent on it. The
//!   `IPC-VERSION-SKEW` marker on that refusal is pinned by `mush_server`'s own
//!   unit test of `admit_gateway`; here the proof is the link ending unanswered.
//!   (Since GG.2 a link's end is that link's alone: the server runs on, and the
//!   refused link sees at most the writer's `Bye` before the close, which
//!   answers nothing the gateway sent.)

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_config::config::Config;
use mush_core::GOD;
use mush_ipc::{
    AttachRec, FrameReader, GatewayInput, GatewayRequest, IPC_MIN_PROTO, IPC_VERSION, SendTarget,
    ServiceReq, ServiceResp, ToGateway, ToServer, write_frame,
};
use mush_store::{InMemoryStore, ObjectStore};

/// One step of the proof must complete within this.
const STEP: Duration = Duration::from_secs(60);

fn scratch_dir(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("rustmush-ipc-window-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// A lean, all-in-memory, ephemeral-listener config.
fn test_config(tag: &str) -> Config {
    let mut config = Config::default();
    config.store.store = "memory".to_owned();
    config.server.data_dir = scratch_dir(tag);
    config.ssh.listen = "127.0.0.1:0".to_owned();
    config.ipc.listen = "127.0.0.1:0".to_owned();
    config
}

/// Boot the real server half on its own runtime thread; its bound IPC endpoint
/// comes back once the socket is listening.
async fn boot_server(tag: &'static str) -> mush_ipc::IpcEndpoint {
    let (bound, bound_rx) = tokio::sync::oneshot::channel();
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let config = test_config(tag);
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("server runtime");
        runtime
            .block_on(mush_server::serve_with_store(
                config,
                Some(bound),
                Some(store),
            ))
            .expect("the server half serves and exits cleanly");
    });
    tokio::time::timeout(STEP, bound_rx)
        .await
        .expect("server bind timed out")
        .expect("server returned its bound endpoint")
}

/// A hand-driven gateway's end of one link.
struct Link {
    frames: FrameReader<mush_ipc::BoxRead>,
    writer: mush_ipc::BoxWrite,
}

impl Link {
    /// Connect and read the server's greeting; returns the link and the window
    /// the server announced as `(proto, min_proto)`.
    async fn connect(endpoint: &mush_ipc::IpcEndpoint) -> (Link, (u32, u32)) {
        let (reader, writer) = mush_ipc::connect(endpoint).await.expect("connect");
        let mut link = Link {
            frames: FrameReader::new(reader),
            writer,
        };
        let hello = link
            .read_until("the server's Hello", |f| {
                matches!(f, ToGateway::Hello { .. })
            })
            .await;
        let Some(ToGateway::Hello {
            proto, min_proto, ..
        }) = hello.last()
        else {
            unreachable!("read_until returned on a Hello");
        };
        let window = (*proto, *min_proto);
        (link, window)
    }

    async fn send(&mut self, msg: &ToServer) {
        write_frame(&mut self.writer, msg)
            .await
            .expect("the link accepts the frame");
    }

    /// Read frames until `pred` accepts one; everything read, the accepted
    /// frame last, comes back. A closed link or [`STEP`] elapsing fails the
    /// proof with what was seen.
    async fn read_until(
        &mut self,
        what: &str,
        pred: impl Fn(&ToGateway) -> bool,
    ) -> Vec<ToGateway> {
        let deadline = tokio::time::Instant::now() + STEP;
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

    /// Read until the server ends the link (a clean EOF or a reset), within
    /// [`STEP`]; everything read comes back.
    async fn read_to_close(&mut self) -> Vec<ToGateway> {
        let deadline = tokio::time::Instant::now() + STEP;
        let mut seen = Vec::new();
        loop {
            match tokio::time::timeout_at(deadline, self.frames.read::<ToGateway>()).await {
                Ok(Ok(Some(frame))) => seen.push(frame),
                Ok(Ok(None)) | Ok(Err(_)) => return seen,
                Err(_) => panic!("the server kept the link open; seen: {seen:?}"),
            }
        }
    }
}

/// The record a gateway re-announces for a session it holds: God, playing on
/// `conn`, as the gateway's `build_attach_recs` would describe it.
fn held_session(conn: u64) -> AttachRec {
    AttachRec {
        output_brackets: Default::default(),
        conn,
        actor: GOD,
        account: Some("boss".to_owned()),
        char_name: String::new(),
        caps_structured: false,
    }
}

fn mentions(frame: &ToGateway, text: &str) -> bool {
    matches!(frame, ToGateway::Deliver { .. }) && format!("{frame:?}").contains(text)
}

fn delivered_to(frame: &ToGateway, conn: u64) -> bool {
    matches!(
        frame,
        ToGateway::Deliver {
            target: SendTarget::Conn(c),
            ..
        } if *c == conn
    ) || matches!(
        frame,
        ToGateway::Deliver {
            target: SendTarget::Char(c),
            ..
        } if *c == u64::from(GOD.0)
    )
}

/// Everything a gateway needs from the server over one link, in the order the
/// real one does it after a (re)connect: the held session re-announced and
/// shown its room (output), the readiness barrier and the connection probe
/// answered (requests), and a command under replay custody run (input).
async fn play_through(link: &mut Link, conn: u64) {
    link.send(&ToServer::ReAnnounce(vec![held_session(conn)]))
        .await;
    link.send(&ToServer::Request {
        id: 1,
        req: ServiceReq::Barrier,
    })
    .await;
    let up_to_barrier = link
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
        up_to_barrier
            .iter()
            .any(|f| delivered_to(f, conn) && mentions(f, "Limbo")),
        "the re-attached session is shown its room before the barrier answers; seen: \
         {up_to_barrier:?}"
    );

    link.send(&ToServer::Request {
        id: 2,
        req: ServiceReq::ConnectionReady { conn, actor: GOD },
    })
    .await;
    link.read_until("the readiness reply", |f| {
        matches!(
            f,
            ToGateway::Reply {
                id: 2,
                resp: ServiceResp::ConnectionReady(true)
            }
        )
    })
    .await;

    let request = GatewayRequest::new(
        0x77,
        1,
        GatewayInput::Cmd {
            conn,
            actor: GOD,
            line: "say the window holds".to_owned(),
        },
    );
    link.send(&ToServer::Input(request.clone())).await;
    link.send(&ToServer::GatewayReleased {
        request: request.uid,
    })
    .await;
    let up_to_echo = link
        .read_until("the say echo", |f| {
            delivered_to(f, conn) && mentions(f, "the window holds")
        })
        .await;
    assert!(
        up_to_echo.iter().any(|f| matches!(
            f,
            ToGateway::RequestDisposition { request: uid, .. } if *uid == request.uid
        )),
        "the custody protocol answers the request with a disposition; seen: {up_to_echo:?}"
    );
}

/// Whether the server answered anything a gateway sent on a link: a reply, a
/// disposition, or output for a connection.
fn answered(seen: &[ToGateway]) -> bool {
    seen.iter().any(|f| {
        matches!(
            f,
            ToGateway::Reply { .. }
                | ToGateway::Deliver {
                    target: SendTarget::Conn(_) | SendTarget::Char(_),
                    ..
                }
                | ToGateway::RequestDisposition { .. }
        )
    })
}

/// A gateway one step behind the server: the window's promise. It announces
/// its own version (every build from 13 on does) and is served in full.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_gateway_one_step_behind_is_served() {
    let endpoint = boot_server("behind").await;
    let (mut link, window) = Link::connect(&endpoint).await;
    assert_eq!(
        window,
        (IPC_VERSION, IPC_MIN_PROTO),
        "the server announces the one-step window it serves"
    );
    link.send(&ToServer::Hello {
        proto: IPC_MIN_PROTO,
        min_proto: IPC_MIN_PROTO - 1,
    })
    .await;
    play_through(&mut link, 41).await;
}

/// A gateway from before the window: no `ToServer::Hello`, its first frame is
/// its roster. The server reads that as a 12 peer, and 12 left the window at
/// the bump to 14: the link is refused and nothing sent on it is answered.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_gateway_with_no_hello_is_below_the_window_and_refused() {
    // The bootstrap rule has run out: checked where the constants are.
    const { assert!(mush_ipc::IPC_PRE_HELLO_VERSION < IPC_MIN_PROTO) };
    let endpoint = boot_server("nohello").await;
    let (mut link, _window) = Link::connect(&endpoint).await;
    // Either write may already fail (the server has closed its end).
    let _ = write_frame(
        &mut link.writer,
        &ToServer::ReAnnounce(vec![held_session(43)]),
    )
    .await;
    let _ = write_frame(
        &mut link.writer,
        &ToServer::Request {
            id: 9,
            req: ServiceReq::Barrier,
        },
    )
    .await;
    let seen = link.read_to_close().await;
    assert!(
        !answered(&seen),
        "a refused link is answered nothing; seen: {seen:?}"
    );
}

/// A gateway at the server's own version announces itself first and is served
/// in full.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_gateway_at_the_servers_version_is_served() {
    let endpoint = boot_server("current").await;
    let (mut link, _window) = Link::connect(&endpoint).await;
    link.send(&ToServer::Hello {
        proto: IPC_VERSION,
        min_proto: IPC_MIN_PROTO,
    })
    .await;
    play_through(&mut link, 42).await;
}

/// A gateway one step AHEAD of the server is the case the one-direction window
/// never covers (the server upgrades first): the server refuses the link and
/// answers nothing on it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_gateway_outside_the_window_is_refused() {
    let endpoint = boot_server("ahead").await;
    let (mut link, _window) = Link::connect(&endpoint).await;
    link.send(&ToServer::Hello {
        proto: IPC_VERSION + 1,
        min_proto: IPC_VERSION,
    })
    .await;
    // Whatever follows the refused Hello is never answered. The write itself
    // may already fail (the server has closed its end), which is fine.
    let _ = write_frame(
        &mut link.writer,
        &ToServer::Request {
            id: 9,
            req: ServiceReq::Barrier,
        },
    )
    .await;
    let seen = link.read_to_close().await;
    assert!(
        !answered(&seen),
        "a refused link is answered nothing (no reply, no disposition, nothing to a connection); seen: {seen:?}"
    );
}

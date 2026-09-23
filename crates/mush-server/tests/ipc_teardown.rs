//! M18 end-to-end proof of the server->gateway boundary teardown (section B/C)
//! against a REAL booted `mush-server`.
//!
//! A minimal fake gateway completes the boundary handshake, attaches a
//! connection, then STOPS DRAINING the server->gateway direction while sending
//! paced input that makes the world produce output. The server's
//! socket send buffer fills, the writer's per-frame timeout ([`ipc].write_timeout_ms`)
//! elapses on the blocked write, and the writer tears the boundary down by
//! dropping its socket half — which the fake gateway observes as a clean EOF on
//! its read side.
//!
//! This is the wedged-peer path (C) that a healthy client can never reach (the
//! everyday slow client is policed by the gateway's `SessionManager`); the
//! bounded-channel overflow (B) and the metric/counter wiring are proven
//! deterministically by the in-crate unit tests, and the *survival* that rides
//! this teardown (supervise -> relaunch -> ReAnnounce -> re-attach) by the
//! gateway `restart_survival` suite — a teardown is just a boundary EOF.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_config::config::Config;
use mush_core::GOD;
use mush_ipc::{
    FrameReader, GatewayInput, GatewayRequest, ToGateway, ToServer, WireInbound, write_frame,
};
use tokio::sync::Notify;
use tracing_subscriber::layer::SubscriberExt;

const WRITE_TIMEOUT_MESSAGE: &str = "ipc write timed out; peer wedged, tearing down boundary";

/// Observe the real writer's timeout before draining can relieve its pressure.
/// This integration binary has one test, so its subscriber covers the booted
/// server's runtime threads without modifying production instrumentation.
struct WriteTimeoutObserver(Arc<Notify>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for WriteTimeoutObserver {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if event.metadata().target() != "mush_server" {
            return;
        }
        let mut visitor = TimeoutMessage(false);
        event.record(&mut visitor);
        if visitor.0 {
            self.0.notify_one();
        }
    }
}

struct TimeoutMessage(bool);

impl tracing::field::Visit for TimeoutMessage {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" && format!("{value:?}") == WRITE_TIMEOUT_MESSAGE {
            self.0 = true;
        }
    }
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mush-ipcteardown-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A lean, all-in-memory, ephemeral-listener config with a SHORT boundary write
/// timeout so a wedged peer is torn down promptly.
fn test_config(tag: &str) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ssh.listen = "127.0.0.1:0".to_string();
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    // A wedged peer past this is torn down; keep it short but generous enough
    // that a legitimate *buffered* write never trips it on a loaded box.
    cfg.ipc.write_timeout_ms = 1000;
    cfg
}

/// Boot a real `mush-server` on its own thread + runtime (its module roster is
/// not `Send`, so the boot future cannot be `tokio::spawn`ed — the binary
/// `block_on`s it the same way) and return the bound IPC endpoint.
async fn boot_server() -> mush_ipc::IpcEndpoint {
    let (ipc_tx, ipc_rx) = tokio::sync::oneshot::channel();
    let cfg = test_config("server");
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        // `None` store opens the configured (memory) backend.
        let _ = rt.block_on(mush_server::serve_with_store(cfg, Some(ipc_tx), None));
    });
    tokio::time::timeout(Duration::from_secs(30), ipc_rx)
        .await
        .expect("server bound its IPC socket")
        .expect("ipc endpoint")
}

/// A wedged gateway — one that connects, attaches a connection, then stops
/// reading the boundary while the world keeps producing — must be torn down by
/// the server's write timeout, WITHOUT the server hanging. The fake gateway
/// sees a clean EOF once the writer drops its socket half.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wedged_gateway_is_torn_down_by_write_timeout() {
    let timed_out = Arc::new(Notify::new());
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry().with(WriteTimeoutObserver(Arc::clone(&timed_out))),
    )
    .expect("install the test's write-timeout observer");
    let endpoint = boot_server().await;
    #[cfg(unix)]
    let mush_ipc::IpcEndpoint::Tcp(addr) = endpoint else {
        panic!("the fixture binds a TCP boundary");
    };
    #[cfg(not(unix))]
    let mush_ipc::IpcEndpoint::Tcp(addr) = endpoint;
    let socket = tokio::net::TcpSocket::new_v4().expect("create fake gateway socket");
    // Fix the peer's receive window before connect; Linux loopback autotuning
    // can otherwise absorb the old finite flood without blocking any write.
    socket
        .set_recv_buffer_size(4096)
        .expect("bound the fake gateway's receive buffer");
    let stream = socket.connect(addr).await.expect("connect boundary");
    stream.set_nodelay(true).expect("set TCP_NODELAY");
    let (reader, mut writer) = stream.into_split();
    let mut frames = FrameReader::new(reader);

    // Handshake: the server greets with Hello first.
    let hello = tokio::time::timeout(Duration::from_secs(10), frames.read::<ToGateway>())
        .await
        .expect("hello within timeout")
        .expect("hello read ok")
        .expect("a Hello frame");
    assert!(
        matches!(hello, ToGateway::Hello { .. }),
        "first frame is Hello"
    );

    // Attach a connection playing God, then stop reading. A paced stream of
    // large replies fills the kernel send buffer without racing to overflow
    // the separate bounded frame queue (which is a different teardown path).
    write_frame(
        &mut writer,
        &ToServer::Inbound(WireInbound::ConnUp {
            conn: 1,
            actor: GOD,
            caps_structured: false,
            from_linkdead: false,
        }),
    )
    .await
    .expect("send ConnUp");
    let teardown_deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    tokio::time::timeout_at(teardown_deadline, async {
        let timeout_notification = timed_out.notified();
        tokio::pin!(timeout_notification);
        tokio::select! {
            biased;
            () = &mut timeout_notification => {}
            () = async {
                let line = format!("say {}", "w".repeat(6000));
                let mut pace = tokio::time::interval(Duration::from_millis(10));
                pace.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                for sequence in 0.. {
                    // Cadence bounds the outstanding frame count while the
                    // blocked writer's real one-second timeout elapses.
                    pace.tick().await;
                    let request = GatewayRequest::new(
                        7,
                        sequence,
                        GatewayInput::Cmd {
                            conn: 1,
                            actor: GOD,
                            line: line.clone(),
                        },
                    );
                    if write_frame(&mut writer, &ToServer::Input(request.clone()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    if write_frame(
                        &mut writer,
                        &ToServer::GatewayReleased { request: request.uid },
                    )
                    .await
                    .is_err()
                    {
                        return;
                    }
                }
            } => {
                // A closed peer can end the producer as the timeout event is
                // published. A transport error alone is never success: await
                // the same notification without consuming its permit twice.
                timeout_notification.await;
            }
        }
    })
    .await
    .expect("a non-draining peer must trigger the real boundary write timeout");

    // Only after the observed timeout, drain the writer's buffered frames to
    // EOF (`Ok(None)`) — the writer tore the boundary down by dropping its half.
    // Bound the drain so a regression (writer never tears down) fails fast rather
    // than hanging.
    let mut saw_deliver = false;
    let torn_down = tokio::time::timeout_at(teardown_deadline, async {
        loop {
            match frames.read::<ToGateway>().await {
                Ok(Some(ToGateway::Deliver { .. })) => saw_deliver = true,
                Ok(Some(_)) => {}
                // Clean EOF: the server dropped its writer half (the teardown).
                Ok(None) => break true,
                // A reset/broken pipe is also a torn-down boundary.
                Err(_) => break true,
            }
        }
    })
    .await
    .expect("the wedged boundary must be torn down, not hang");

    assert!(torn_down, "the server tore the boundary down");
    assert!(
        saw_deliver,
        "the world produced output before the boundary was torn down"
    );

    // Closing our side lets the server's reader EOF and the process exit cleanly.
    drop(writer);
    drop(frames);
}

//! M18 end-to-end proof of the server->gateway boundary teardown (section B/C)
//! against a REAL booted `mush-server`.
//!
//! A minimal fake gateway completes the boundary handshake, attaches a
//! connection, then STOPS DRAINING the server->gateway direction while flooding
//! input that makes the world produce a large burst of output. The server's
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
use std::time::Duration;

use mush_config::config::Config;
use mush_core::GOD;
use mush_ipc::{
    FrameReader, GatewayInput, GatewayRequest, ToGateway, ToServer, WireInbound, write_frame,
};

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
    cfg.rulesets.enabled = vec![];
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
    let endpoint = boot_server().await;
    let (reader, mut writer) = mush_ipc::connect(&endpoint)
        .await
        .expect("connect boundary");
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

    // Attach a connection playing God, then FLOOD input the world will answer
    // with a large burst of output. We deliberately DO NOT read any of it, so
    // the server's socket send buffer fills and the writer blocks on a write.
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
    for sequence in 0..4000u64 {
        let request = GatewayRequest::new(
            7,
            sequence,
            GatewayInput::Cmd {
                conn: 1,
                actor: GOD,
                line: "look".to_string(),
            },
        );
        write_frame(&mut writer, &ToServer::Input(request.clone()))
            .await
            .expect("send Cmd");
        write_frame(
            &mut writer,
            &ToServer::GatewayReleased {
                request: request.uid,
            },
        )
        .await
        .expect("release Cmd");
    }

    // Give the writer's timeout (1s) time to elapse on the blocked write while we
    // keep NOT reading, so draining does not unblock it.
    tokio::time::sleep(Duration::from_millis(2500)).await;

    // Now drain: we read whatever the writer buffered before it wedged, then hit
    // EOF (`Ok(None)`) — the writer tore the boundary down by dropping its half.
    // Bound the drain so a regression (writer never tears down) fails fast rather
    // than hanging.
    let mut saw_deliver = false;
    let torn_down = tokio::time::timeout(Duration::from_secs(20), async {
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

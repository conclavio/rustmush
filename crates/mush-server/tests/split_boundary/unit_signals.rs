//! Doc 33 GG.1: the server is its own systemd unit, so `systemctl stop` and
//! `systemctl restart` reach it as `SIGTERM` directly — until GG.1 the signal
//! went to the gateway, which closed the boundary, and the server only ever
//! saw EOF. A real `mush-server` process with a gateway attached must answer
//! the signal the way it answers that EOF: drain the world, exit zero, say
//! so. Before the signal arm the default action killed the process mid-write,
//! and the unit read that as a failure.
//!
//! Unix only: Windows has no `SIGTERM` for a console process spawned this
//! way, and the unit does not run there.

#![cfg(unix)]

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mush_ipc::{
    FrameReader, IPC_MIN_PROTO, IPC_VERSION, IpcEndpoint, ToGateway, ToServer, write_frame,
};
use mush_testkit::proc::ChildProcess;

/// A cold binary on a loaded CI box boots inside this; a hung drain fails
/// rather than hangs.
const TIMEOUT: Duration = Duration::from_secs(60);

const BIN: &str = env!("CARGO_BIN_EXE_mush-server");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mush-unit-signals-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// A loopback endpoint nothing is listening on.
fn reserve_endpoint() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve a port");
    let addr = listener.local_addr().expect("the reserved address");
    drop(listener);
    addr.to_string()
}

/// Everything in memory, every listener ephemeral, the simulator off.
fn throwaway_config(dir: &Path) -> PathBuf {
    let data_dir = dir.join("data");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    let path = dir.join("throwaway.toml");
    let escaped = data_dir.display().to_string().replace('\\', "/");
    let text = [
        "[server]".to_string(),
        format!("data_dir = \"{escaped}\""),
        "[store]".to_string(),
        "store = \"memory\"".to_string(),
        "[ssh]".to_string(),
        "listen = \"127.0.0.1:0\"".to_string(),
        "[web]".to_string(),
        "listen = \"127.0.0.1:0\"".to_string(),
    ]
    .join("\n");
    std::fs::write(&path, text).expect("write config");
    path
}

/// Attach a minimal gateway: connect, take the server's `Hello`, announce
/// this side's version, and keep the link open — the reader drained on the
/// runtime's workers, the writer held — so nothing this side does can be
/// mistaken for a gateway that hung up.
async fn attach_gateway(endpoint: &str) -> tokio::task::JoinHandle<()> {
    let endpoint = IpcEndpoint::parse(endpoint).expect("a tcp endpoint");
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    let (reader, mut writer) = loop {
        match mush_ipc::connect(&endpoint).await {
            Ok(pair) => break pair,
            Err(error) => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "the server never accepted on {}: {error}",
                    endpoint.describe()
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    };
    let mut frames = FrameReader::new(reader);
    let hello = tokio::time::timeout(TIMEOUT, frames.read::<ToGateway>())
        .await
        .expect("a Hello within the timeout")
        .expect("the boundary reads")
        .expect("a Hello frame before any EOF");
    assert!(
        matches!(hello, ToGateway::Hello { .. }),
        "first frame is Hello"
    );
    write_frame(
        &mut writer,
        &ToServer::Hello {
            proto: IPC_VERSION,
            min_proto: IPC_MIN_PROTO,
        },
    )
    .await
    .expect("announce the gateway");
    tokio::spawn(async move {
        let _writer = writer;
        while matches!(frames.read::<ToGateway>().await, Ok(Some(_))) {}
    })
}

/// `SIGTERM` with a gateway attached: the world is drained, the exit is zero
/// (a `Restart=on-failure` unit stays stopped), and the log says which signal.
#[test]
fn sigterm_with_a_gateway_attached_drains_the_world_and_exits_zero() {
    let dir = scratch("sigterm");
    let config = throwaway_config(&dir);
    let endpoint = reserve_endpoint();
    let mut child = ChildProcess::builder(BIN)
        .args([
            "--config",
            &config.display().to_string(),
            "--ipc",
            &endpoint,
        ])
        .spawn()
        .expect("spawn mush-server");
    child
        .await_log("awaiting gateway", TIMEOUT)
        .expect("the server bound its endpoint");

    // The gateway side lives on this runtime's workers for the rest of the
    // test; the link stays open until the server closes it.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()
        .expect("a runtime");
    let drain = rt.block_on(attach_gateway(&endpoint));
    child
        .await_log("gateway connected", TIMEOUT)
        .expect("the server saw the gateway");

    child.terminate();
    let (status, log) = child
        .wait_with_log(TIMEOUT)
        .expect("the server exited after SIGTERM");
    assert!(
        status.success(),
        "SIGTERM is a clean stop, exit zero; got {status}; output:\n{log}"
    );
    assert!(
        log.contains("shutting down on signal") && log.contains("SIGTERM"),
        "the server names the signal it stopped on; output:\n{log}"
    );
    // The server closed the link on its way out, so the drain is done.
    rt.block_on(async {
        let _ = tokio::time::timeout(TIMEOUT, drain).await;
    });
    let _ = std::fs::remove_dir_all(dir);
}

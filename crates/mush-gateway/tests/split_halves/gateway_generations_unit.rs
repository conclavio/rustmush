//! Doc 33 §4.5, cut GG.4: the real `mush-gateway` binary under its unit's
//! protocol. The in-process story (`gateway_generations`) drives the
//! lifecycle through its trigger; this module sends the signals systemd and
//! the swap script send, to the process, and reads what systemd would read:
//!
//! - `--generation a` says `READY=1` on `$NOTIFY_SOCKET` once it serves;
//! - `SIGUSR1` drains: with no session the process exits 0 at once and leaves
//!   `draining-a` in `$STATE_DIRECTORY`;
//! - started again with that marker it says `READY=1` and exits 0 (a
//!   `Type=notify` process that exited before READY would be a protocol
//!   failure `Restart=on-failure` restarts, doc 29a §7 M4), marker intact;
//! - with the marker cleared (the swap script's job) it serves again, and
//!   `SIGTERM` finishes it: exit 0, no marker;
//! - `--check-config` refuses a symlinked `[ssh].trusted_socket` without
//!   `--generation` and accepts it with one (doc 33 §4.4).
//!
//! No server is up: the gateway serves its boot gap, which is enough for
//! READY (its listeners are bound).

use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mush_testkit::proc::ChildProcess;

const BIN: &str = env!("CARGO_BIN_EXE_mush-gateway");
const TIMEOUT: Duration = Duration::from_secs(60);

fn scratch(tag: &str) -> PathBuf {
    // Short: the notify socket is a Unix socket path (108 bytes at most).
    let dir = std::env::temp_dir().join(format!("mush-ggu-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// A port nothing listens on: bound, read, released. The gateway's dial is
/// refused at once and it serves the gap.
fn refused_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    l.local_addr().expect("an address").port()
}

/// Everything in memory, every listener ephemeral, the server unreachable.
fn config(dir: &Path, trusted_socket: Option<&Path>) -> PathBuf {
    let data_dir = dir.join("data");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    let path = dir.join("gateway.toml");
    let mut text = vec![
        "[server]".to_string(),
        format!("data_dir = \"{}\"", data_dir.display()),
        "[store]".to_string(),
        "store = \"memory\"".to_string(),
        "[ssh]".to_string(),
        "listen = \"127.0.0.1:0\"".to_string(),
    ];
    if let Some(socket) = trusted_socket {
        text.push(format!("trusted_socket = \"{}\"", socket.display()));
    }
    text.extend([
        "[web]".to_string(),
        "listen = \"127.0.0.1:0\"".to_string(),
        "[ipc]".to_string(),
        format!("listen = \"127.0.0.1:{}\"", refused_port()),
    ]);
    std::fs::write(&path, text.join("\n")).expect("write config");
    path
}

/// systemd's end of `$NOTIFY_SOCKET`.
struct NotifySocket {
    socket: UnixDatagram,
    path: PathBuf,
}

impl NotifySocket {
    fn bind(dir: &Path) -> NotifySocket {
        let path = dir.join("notify.sock");
        let socket = UnixDatagram::bind(&path).expect("the notify socket");
        socket
            .set_read_timeout(Some(TIMEOUT))
            .expect("a read timeout");
        NotifySocket { socket, path }
    }

    /// Datagrams until one carries `needle`.
    fn expect(&self, needle: &str) {
        let mut seen = Vec::new();
        loop {
            let mut buf = [0u8; 512];
            let n = self
                .socket
                .recv(&mut buf)
                .unwrap_or_else(|e| panic!("no datagram with {needle:?} ({e}); seen {seen:?}"));
            let text = String::from_utf8_lossy(&buf[..n]).into_owned();
            if text.contains(needle) {
                return;
            }
            seen.push(text);
        }
    }
}

fn spawn_generation(
    config: &Path,
    state_dir: &Path,
    notify: &NotifySocket,
    generation: &str,
) -> ChildProcess {
    ChildProcess::builder(BIN)
        .args([
            "--config",
            &config.display().to_string(),
            "--generation",
            generation,
        ])
        .env("NOTIFY_SOCKET", &notify.path)
        .env("STATE_DIRECTORY", state_dir)
        .spawn()
        .expect("spawn mush-gateway")
}

/// `kill -<sig> <pid>`: what systemd and the swap script send.
fn signal(child: &ChildProcess, sig: &str) {
    let status = std::process::Command::new("kill")
        .args([sig, &child.pid().to_string()])
        .status()
        .expect("kill runs");
    assert!(status.success(), "kill {sig} {}", child.pid());
}

fn exit_zero(child: &mut ChildProcess, what: &str) {
    let status = child
        .wait_exit(TIMEOUT)
        .unwrap_or_else(|e| panic!("{what}: {e}\n{}", child.log_snapshot()));
    assert!(
        status.success(),
        "{what}: exited {status}\n{}",
        child.log_snapshot()
    );
}

/// READY, a drain that leaves the marker, a restart the marker refuses, a
/// finish that leaves none.
#[test]
fn a_generation_reports_ready_drains_on_usr1_refuses_by_its_marker_and_finishes_on_term() {
    let dir = scratch("lifecycle");
    let state_dir = dir.join("state");
    std::fs::create_dir_all(&state_dir).expect("state dir");
    let notify = NotifySocket::bind(&dir);
    let config = config(&dir, None);
    let marker = state_dir.join("draining-a");

    // Serving: READY=1 once the listeners are bound and the gap is serving.
    let mut child = spawn_generation(&config, &state_dir, &notify, "a");
    notify.expect("READY=1");
    child
        .await_log("gateway generation ready", TIMEOUT)
        .unwrap_or_else(|e| panic!("{e}\n{}", child.log_snapshot()));
    assert!(!marker.exists());

    // SIGUSR1: a drain; nobody is connected, so the generation exits at once
    // and leaves its marker.
    signal(&child, "-USR1");
    exit_zero(&mut child, "a drained empty generation");
    assert!(marker.exists(), "the drain left its marker");

    // Restart=on-failure (or an operator) brings it back with the marker:
    // READY=1, exit 0, nothing bound, the marker intact.
    let mut again = spawn_generation(&config, &state_dir, &notify, "a");
    notify.expect("READY=1");
    exit_zero(&mut again, "a generation that found its marker");
    assert!(
        again.log_snapshot().contains("not serving"),
        "{}",
        again.log_snapshot()
    );
    assert!(marker.exists(), "the marker is the swap script's to clear");

    // The swap script cleared it: serving again; SIGTERM finishes (one
    // warning, the grace, the close — nobody to warn here), exit 0, no marker.
    std::fs::remove_file(&marker).expect("the swap script clears the marker");
    let mut third = spawn_generation(&config, &state_dir, &notify, "a");
    notify.expect("READY=1");
    third
        .await_log("gateway generation ready", TIMEOUT)
        .unwrap_or_else(|e| panic!("{e}\n{}", third.log_snapshot()));
    third.terminate();
    exit_zero(&mut third, "a finished generation");
    assert!(!marker.exists(), "a finish leaves no marker");
}

/// Doc 33 §4.4: the configured trusted socket is the generations' symlink;
/// a gateway without `--generation` must not bind it.
#[test]
fn check_config_refuses_a_symlinked_trusted_socket_without_a_generation() {
    let dir = scratch("link");
    let link = dir.join("gw.sock");
    std::os::unix::fs::symlink(dir.join("gw.a.sock"), &link).expect("a symlink");
    let config = config(&dir, Some(&link));
    let config = config.display().to_string();

    let mut refused = ChildProcess::builder(BIN)
        .args(["--check-config", "--config", &config])
        .spawn()
        .expect("spawn");
    let (status, log) = refused.wait_with_log(TIMEOUT).expect("exits");
    assert!(
        !status.success(),
        "a symlinked trusted socket was accepted:\n{log}"
    );
    assert!(log.contains("gw.sock") && log.contains("symlink"), "{log}");

    let mut accepted = ChildProcess::builder(BIN)
        .args(["--check-config", "--config", &config, "--generation", "a"])
        .spawn()
        .expect("spawn");
    let (status, log) = accepted.wait_with_log(TIMEOUT).expect("exits");
    assert!(
        status.success(),
        "a generation was refused its link:\n{log}"
    );
    assert!(log.contains("is usable"), "{log}");
}

//! WS22-S2: `systemctl stop` must end `mushd` through its own drain, not
//! through the kernel.
//!
//! The regression pin for the shared shutdown future. A binary that installs
//! no SIGTERM handler still "stops" when systemd sends one — the default
//! disposition kills it — and every visible symptom of that is *absence*: no
//! shutdown announce to the connected players, no writer drain, no final
//! BGSAVE, and a unit that logs `status=143/n/a` instead of a clean exit. So
//! the discriminating assertion is not "the process ended" but "the process
//! ended with an exit CODE": a signal-terminated process has none at all
//! (`ExitStatus::code()` is `None` on Unix), and its drain tail never ran.
//!
//! Unix only — Windows has no SIGTERM for a console application, and
//! `ChildProcess::terminate` falls back to `TerminateProcess` there, which is
//! the very unclean kill this test exists to rule out.

use std::path::{Path, PathBuf};
use std::time::Duration;

use mush_testkit::proc::ChildProcess;

/// A cold binary on a loaded CI box has to get all the way to its listeners.
const BOOT_TIMEOUT: Duration = Duration::from_secs(60);
/// The whole drain: announce, world join, writer drain, store close, flush.
const EXIT_TIMEOUT: Duration = Duration::from_secs(30);

const BIN: &str = env!("CARGO_BIN_EXE_mushd");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mush-sigterm-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// A config that boots with no Redis and no ruleset: everything in memory,
/// every listener ephemeral. The point of the test is the signal, not the
/// world.
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

#[test]
fn sigterm_ends_mushd_through_its_own_shutdown() {
    let dir = scratch("mushd");
    let config = throwaway_config(&dir);
    let mut child = ChildProcess::builder(BIN)
        .args(["--config", &config.display().to_string()])
        .spawn()
        .expect("spawn mushd");

    // Wait for the listeners: a SIGTERM delivered before the select! is
    // reached would prove nothing about the handler.
    let up = child.await_log("up; press Ctrl-C to shut down", BOOT_TIMEOUT);
    assert!(up.is_ok(), "mushd never came up: {up:?}");

    child.terminate();
    let exited = child.wait_with_log(EXIT_TIMEOUT);
    let (status, log) = match exited {
        Ok(pair) => pair,
        Err(e) => panic!("mushd did not exit after SIGTERM: {e}"),
    };

    assert_eq!(
        status.code(),
        Some(0),
        "mushd exited {status} on SIGTERM — with no code it was killed by the \
         signal and never drained; output:\n{log}"
    );
    // The tail, not just the exit: the last line `main` writes before Ok(()).
    assert!(
        log.contains("mushd exiting at"),
        "mushd exited 0 but its shutdown tail left no trace; output:\n{log}"
    );
}

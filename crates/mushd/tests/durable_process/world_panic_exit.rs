//! A panic must land in the LOG FILE, and the two halves of the split must
//! not share one (WS01-S13).
//!
//! Two gaps, proved here against real OS processes rather than in-process
//! layer primitives, because both failures are process-shaped:
//!
//! - The default panic hook writes to stderr. A gateway-spawned `mush-server`
//!   child configures no stdio, so it inherits the parent's — the panic lands
//!   in the journal and *never* in the file `logging::init` writes under
//!   `data_dir`. The hook installed by `init` must put it there.
//! - Both halves read the same config with the same `data_dir`. Before the
//!   role discriminator they opened two independently buffered appenders on
//!   one path, uncoordinated at rotation.
//!
//! `logging::init` installs a *process-global* subscriber and can only run
//! once per process, so the rig re-executes this very test binary twice — once
//! as the `gateway` half, once as the `server` half — pointed at one shared
//! `data_dir`. That is the real thing: two processes, one directory.

use std::path::{Path, PathBuf};
use std::process::Command;

use mushd::config::LogCfg;
use mushd::logging;

/// Env vars the parent uses to tell a child which half it is and where to log.
const ROLE_ENV: &str = "MUSHD_TEST_LOG_ROLE";
const DIR_ENV: &str = "MUSHD_TEST_LOG_DIR";
/// The child-half test's name, as libtest filters it.
const CHILD_TEST: &str = "world_panic_exit::the_half_child";
/// The panic the `server` half raises; asserted in its log file by the parent.
const PANIC_PAYLOAD: &str = "the-world-loop-fell-over";

/// One half of the split, run as a child process by
/// [`split_halves_write_distinct_log_files`]. Boots real file logging under
/// the role it was handed and — for the `server` half — panics, so the parent
/// can look for the panic in the file afterwards.
#[test]
#[ignore = "the child half of split_halves_write_distinct_log_files"]
fn the_half_child() {
    let role = std::env::var(ROLE_ENV).expect("the parent sets the role");
    let data_dir = PathBuf::from(std::env::var(DIR_ENV).expect("the parent sets the data dir"));

    let log = LogCfg {
        // This fixture asserts INFO records; an operator's RUST_LOG must not
        // silently filter its sentinel out of both real log files.
        level: "info".into(),
        file: true,
        ..LogCfg::default()
    };
    let guards = logging::init(&log, &data_dir, &role);
    tracing::info!(half = %role, "child half up");

    if role == "server" {
        // The shape the hook exists for: a panic that unwinds past the code
        // that raised it. `catch_unwind` here only keeps the child's exit code
        // clean — the hook has already run by the time it catches.
        let caught = std::panic::catch_unwind(|| panic!("{PANIC_PAYLOAD}"));
        assert!(caught.is_err(), "the panic must have unwound");
    }

    // Flush and join the appender's worker before the process exits.
    drop(guards);
}

fn scratch_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mushd-panic-log-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run this test binary again as `role`, returning the child's stderr.
fn run_half(role: &str, data_dir: &Path) -> String {
    let exe = std::env::current_exe().expect("the test binary's own path");
    let out = Command::new(exe)
        .args([CHILD_TEST, "--exact", "--ignored", "--nocapture"])
        .env(ROLE_ENV, role)
        .env(DIR_ENV, data_dir)
        .output()
        .unwrap_or_else(|e| panic!("spawning the {role} half: {e}"));
    assert!(
        out.status.success(),
        "the {role} half exited {:?}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Read the one rotated file whose name starts with `<role>.log`.
fn read_half_log(data_dir: &Path, role: &str) -> String {
    let logs = data_dir.join("logs");
    let prefix = logging::log_file_name(role);
    let mut hits: Vec<PathBuf> = std::fs::read_dir(&logs)
        .unwrap_or_else(|e| panic!("reading {}: {e}", logs.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix))
        })
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "exactly one {prefix} file in {}",
        logs.display()
    );
    std::fs::read_to_string(hits.pop().unwrap()).unwrap()
}

/// The whole contract in one rig: two processes, one `data_dir`, two distinct
/// files, each stamped with its own half — and the `server` half's panic
/// present in *its* log file (not only on the stderr both halves inherit).
#[test]
fn split_halves_write_distinct_log_files() {
    let data_dir = scratch_dir();

    let gateway_stderr = run_half("gateway", &data_dir);
    let server_stderr = run_half("server", &data_dir);

    // Two halves, two files — not one shared `mushd.log`.
    let names: Vec<String> = std::fs::read_dir(data_dir.join("logs"))
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    assert_eq!(names.len(), 2, "one file per half, got {names:?}");
    assert_ne!(
        logging::log_file_name("gateway"),
        logging::log_file_name("server"),
        "the two halves must not share an appender path"
    );
    assert!(
        names.iter().any(|n| n.starts_with("gateway.log")),
        "a gateway.log, got {names:?}"
    );
    assert!(
        names.iter().any(|n| n.starts_with("server.log")),
        "a server.log, got {names:?}"
    );

    let gateway_log = read_half_log(&data_dir, "gateway");
    let server_log = read_half_log(&data_dir, "server");

    // The role reaches every event as a field, and neither half's lines leak
    // into the other's file.
    assert!(
        gateway_log.contains("half=gateway"),
        "gateway log: {gateway_log}"
    );
    assert!(
        !gateway_log.contains("half=server"),
        "gateway log: {gateway_log}"
    );
    assert!(
        server_log.contains("half=server"),
        "server log: {server_log}"
    );
    assert!(
        !server_log.contains("half=gateway"),
        "server log: {server_log}"
    );

    // The failure the step names: the panic is in the FILE.
    let panic_lines: Vec<&str> = server_log
        .lines()
        .filter(|l| l.contains(logging::PANIC_MARKER))
        .collect();
    assert_eq!(
        panic_lines.len(),
        1,
        "exactly one panic event in the server half's log file: {server_log}"
    );
    let line = panic_lines[0];
    assert!(line.contains("ERROR"), "at ERROR: {line}");
    assert!(line.contains(PANIC_PAYLOAD), "the payload text: {line}");
    assert!(
        line.contains("world_panic_exit.rs:"),
        "the panic location: {line}"
    );
    assert!(line.contains("backtrace="), "a backtrace: {line}");
    assert!(
        !gateway_log.contains(logging::PANIC_MARKER),
        "the panic belongs to the half that raised it: {gateway_log}"
    );

    // The hook chains: the default stderr message still appeared, which is the
    // only place it used to appear at all.
    assert!(
        server_stderr.contains(PANIC_PAYLOAD),
        "the previous hook still runs: {server_stderr}"
    );
    assert!(
        !gateway_stderr.contains(PANIC_PAYLOAD),
        "gateway stderr: {gateway_stderr}"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
}

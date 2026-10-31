//! WS22-S1: `mush-gateway` refuses a command line it does not understand.
//!
//! Before the shared parser in `mush_config::cli`, every one of these lines
//! booted: the hand-rolled scanner looked for `--config`, took the token after
//! it if there was one, and dropped everything else on the floor. A mistyped
//! flag started the server on the DEFAULT config — a different data dir, a
//! different store — and said nothing about it. These are process tests
//! because that is where the failure lived: the exit status and what reaches
//! stderr, not a return value.
//!
//! Each case asserts BOTH halves. A non-zero exit alone proves nothing here —
//! on a runner with no Redis the old binary already exited non-zero, for an
//! entirely different reason — so the discriminating assertion is that the
//! offending token or path is named in the output.

use std::path::{Path, PathBuf};
use std::time::Duration;

use mush_testkit::proc::ChildProcess;

/// Generous enough for a cold binary on a loaded CI box, short enough that a
/// regression (a line that boots instead of refusing) fails rather than hangs.
const TIMEOUT: Duration = Duration::from_secs(30);

const BIN: &str = env!("CARGO_BIN_EXE_mush-gateway");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mush-cliargs-mush-gateway-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// A config that WOULD boot: everything in memory, every listener ephemeral,
/// with the simulator disabled. Its job is to make a regression loud — revert the parser and
/// these lines start a server that runs until the timeout kills it, instead of
/// exiting for the wrong reason and accidentally passing.
fn throwaway_config(dir: &Path) -> PathBuf {
    let data_dir = dir.join("data");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    let path = dir.join("throwaway.toml");
    // TOML basic strings take backslash escapes; a Windows path must not carry
    // any into the file.
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

/// Run `mush-gateway` with `args` and return its exit status plus everything it
/// printed.
fn run(args: &[&str]) -> (std::process::ExitStatus, String) {
    let mut child = ChildProcess::builder(BIN)
        .args(args)
        .spawn()
        .expect("spawn mush-gateway");
    child.wait_with_log(TIMEOUT).expect("mush-gateway exited")
}

/// Assert the line was refused, loudly, naming `needle`.
fn assert_refused(args: &[&str], needle: &str) {
    let (status, log) = run(args);
    assert!(
        !status.success(),
        "mush-gateway {args:?} exited {status} — it should have refused; output:\n{log}"
    );
    assert!(
        log.contains(needle),
        "mush-gateway {args:?} did not name {needle:?} in its output:\n{log}"
    );
}

/// The headline case: one transposed letter used to boot the defaults.
#[test]
fn a_mistyped_flag_is_refused_and_named() {
    let dir = scratch("mistyped");
    let config = throwaway_config(&dir);
    assert_refused(
        &[
            "--config",
            &config.display().to_string(),
            "--confg",
            "prod.toml",
        ],
        "--confg",
    );
}

#[test]
fn config_without_a_value_is_refused() {
    assert_refused(&["--config"], "--config");
}

#[test]
fn a_repeated_config_is_refused() {
    let dir = scratch("repeated");
    let config = throwaway_config(&dir).display().to_string();
    assert_refused(&["--config", &config, "--config", &config], "--config");
}

/// The path an operator mistyped has to appear in the refusal, or the refusal
/// does not help them.
#[test]
fn an_explicit_config_that_is_not_there_is_refused() {
    let dir = scratch("absent");
    let missing = dir.join("nope.toml");
    assert_refused(&["--config", &missing.display().to_string()], "nope.toml");
}

/// Neither `--ipc` nor `--repl` is the gateway's: it reads `[ipc].listen` from
/// the config and dials it (doc 33 GG.1), and it has no REPL.
#[test]
fn flags_from_the_other_binaries_are_refused() {
    assert_refused(&["--ipc", "127.0.0.1:6791"], "--ipc");
    assert_refused(&["--repl"], "--repl");
}

/// `--generation` (doc 33 §4.3) takes `a` or `b`, once. Anything else would
/// be a gateway of no generation that LOOKED like one, and two of those hand
/// the server the same connection ids.
#[test]
fn generation_takes_a_or_b_once() {
    let dir = scratch("generation");
    let config = throwaway_config(&dir).display().to_string();
    assert_refused(&["--generation", "c", "--config", &config], "--generation");
    assert_refused(&["--config", &config, "--generation"], "--generation");
    assert_refused(
        &[
            "--generation",
            "a",
            "--generation",
            "b",
            "--config",
            &config,
        ],
        "--generation",
    );
}

/// `--help` still works, and still exits zero, and names the generation flag.
#[test]
fn help_exits_zero_with_the_usage() {
    let (status, log) = run(&["--help"]);
    assert!(status.success(), "--help exited {status}; output:\n{log}");
    assert!(log.contains("Usage: mush-gateway"), "output:\n{log}");
    assert!(log.contains("--generation a|b"), "output:\n{log}");
}

/// Doc 33 §4.7: `--ipc-version` prints the boundary version this binary
/// speaks and exits 0 without reading a config or binding anything, so
/// `deploy.sh` can compare a new server binary's against the running one's.
#[test]
fn ipc_version_prints_the_boundary_version_and_boots_nothing() {
    let (status, log) = run(&["--ipc-version"]);
    assert!(
        status.success(),
        "mush-gateway --ipc-version exited {status}; output:\n{log}"
    );
    assert_eq!(
        log.trim(),
        mush_ipc::IPC_VERSION.to_string(),
        "output:\n{log}"
    );
    // Alone: it is an operation, not a modifier of one.
    assert_refused(&["--ipc-version", "--check-config"], "--ipc-version");
}

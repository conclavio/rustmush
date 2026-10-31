//! WS22-S1: `mushd` refuses a command line it does not understand.
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

const BIN: &str = env!("CARGO_BIN_EXE_mushd");

#[test]
fn bootstrap_refuses_a_throwaway_store_without_booting() {
    let dir = scratch("bootstrap-memory");
    let config = throwaway_config(&dir);
    assert_refused(
        &[
            "--config",
            config.to_str().unwrap(),
            "--bootstrap-staff",
            "keeper",
        ],
        "requires the Redis store",
    );
    assert!(!dir.join("data/quarantine").exists());
    let _ = std::fs::remove_dir_all(dir);
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mush-cliargs-mushd-{tag}-{}", std::process::id()));
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

/// Run `mushd` with `args` and return its exit status plus everything it
/// printed.
fn run(args: &[&str]) -> (std::process::ExitStatus, String) {
    let mut child = ChildProcess::builder(BIN)
        .args(args)
        .spawn()
        .expect("spawn mushd");
    child.wait_with_log(TIMEOUT).expect("mushd exited")
}

/// Assert the line was refused, loudly, naming `needle`.
fn assert_refused(args: &[&str], needle: &str) {
    let (status, log) = run(args);
    assert!(
        !status.success(),
        "mushd {args:?} exited {status} — it should have refused; output:\n{log}"
    );
    assert!(
        log.contains(needle),
        "mushd {args:?} did not name {needle:?} in its output:\n{log}"
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

/// `--ipc` belongs to the server half. `mushd` accepting it silently would be
/// a flag that looks honoured and is not.
#[test]
fn a_flag_from_another_binary_is_refused() {
    assert_refused(&["--ipc", "127.0.0.1:6791"], "--ipc");
    // `--generation` is the gateway's too (doc 33 §4.3): `mushd` has no second gateway.
    assert_refused(&["--generation", "b"], "--generation");
}

/// A config whose `[store].receipt_policy` is a typo — the boot-fatal value
/// `--check-config` exists to catch before a supervisor does.
fn typod_receipt_policy_config(dir: &Path) -> PathBuf {
    let path = throwaway_config(dir);
    let text = std::fs::read_to_string(&path).expect("read config");
    // Inside `[store]`, not appended to the tail — a stray key under the last
    // stanza would be ignored and the test would pass for no reason.
    let spiked = text.replace(
        "store = \"memory\"",
        "store = \"memory\"\nreceipt_policy = \"hostpowerloss\"",
    );
    assert_ne!(spiked, text, "the [store] stanza moved: {text}");
    std::fs::write(&path, spiked).expect("rewrite config");
    path
}

/// `--check-config` outranks `--repl` (W1-B8-FIX).
///
/// The two flags used to be answered in declaration order, and `--repl`
/// returned first: `mushd --repl --check-config` started the REPL and dropped
/// the preflight silently — a flag that looks honoured and is not, which is the
/// exact failure the shared parser exists to abolish, one level up. So the
/// check has to run, name the bad key, and exit non-zero even when `--repl`
/// is on the same line. (The child's stdin is `/dev/null`, so a regression
/// would exit 0 at the REPL's first EOF rather than hang — both assertions
/// below still catch it.)
#[test]
fn check_config_beats_repl() {
    let dir = scratch("repl-check");
    let config = typod_receipt_policy_config(&dir);
    let (status, log) = run(&[
        "--repl",
        "--check-config",
        "--config",
        &config.display().to_string(),
    ]);
    assert!(
        !status.success(),
        "mushd --repl --check-config exited {status} — the preflight must refuse this config; \
         output:\n{log}"
    );
    assert!(
        log.contains("hostpowerloss"),
        "the preflight finding must name the offending value:\n{log}"
    );
    assert!(
        !log.contains("offline REPL"),
        "the REPL must not have started:\n{log}"
    );
}

/// ...and `--repl` on its own is untouched: with stdin at EOF the REPL prints
/// its banner and exits zero, so the precedence above is a rule about the
/// COMBINATION, not a `--repl` that stopped working.
#[test]
fn repl_alone_still_runs() {
    let dir = scratch("repl-alone");
    let config = throwaway_config(&dir);
    let (status, log) = run(&["--repl", "--config", &config.display().to_string()]);
    assert!(
        status.success(),
        "mushd --repl exited {status}; output:\n{log}"
    );
    assert!(log.contains("offline REPL"), "output:\n{log}");
}

/// `--help` still works, and still exits zero.
#[test]
fn help_exits_zero_with_the_usage() {
    let (status, log) = run(&["--help"]);
    assert!(status.success(), "--help exited {status}; output:\n{log}");
    assert!(log.contains("Usage: mushd"), "output:\n{log}");
}

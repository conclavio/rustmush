//! Feature-gated fault-injection hooks: the K8 crash matrix's process
//! terminations, and WS01-S8's armed world-turn panic.
//!
//! The hooks are inert unless both environment variables name an exact cut
//! and a one-shot marker path. A marker is created with `create_new`, so a
//! supervised replacement process can inherit the same environment without
//! crashing a second time.
//!
//! WS01-S11D: the gate is the `fault-injection` cargo feature, **not**
//! `cfg(debug_assertions)`. The two are the same thing only for as long as
//! nobody sets `debug-assertions = true` in `[profile.release]` — and the
//! moment somebody does, an `RUSTMUSH_K8_PROCESS_CUT=...` in the production
//! environment becomes an abort switch sited at the four durability cuts and
//! the quarantine publication chosen precisely to maximise the damage. The
//! feature is off by default and no shipped binary turns it on; the two kill
//! proofs (`crates/mush-gateway/tests/split_halves/authoritative_process_kill.rs`) enable
//! it through a `[dev-dependencies]` edge, so the seam exists exactly where
//! it is proven and nowhere else.

#[cfg(feature = "fault-injection")]
use std::io::Write as _;
#[cfg(feature = "fault-injection")]
use std::path::PathBuf;
#[cfg(feature = "fault-injection")]
use std::time::Duration;

#[cfg(feature = "fault-injection")]
const CUT_ENV: &str = "RUSTMUSH_K8_PROCESS_CUT";
#[cfg(feature = "fault-injection")]
const MARKER_ENV: &str = "RUSTMUSH_K8_PROCESS_CUT_MARKER";
#[cfg(feature = "fault-injection")]
const RELEASE_ENV: &str = "RUSTMUSH_K8_PROCESS_CUT_RELEASE";
/// WS01-S8: the exact command line whose world turn must panic.
#[cfg(feature = "fault-injection")]
const PANIC_CMD_ENV: &str = "RUSTMUSH_WS01_S8_PANIC_CMD";
/// WS01-S8: the one-shot marker for that panic, so a relaunched replacement
/// inheriting the same environment boots clean.
#[cfg(feature = "fault-injection")]
const PANIC_MARKER_ENV: &str = "RUSTMUSH_WS01_S8_PANIC_MARKER";

#[cfg(feature = "fault-injection")]
enum ArmedCut {
    Immediate,
    ReleaseGated(PathBuf),
}

#[cfg(feature = "fault-injection")]
fn arm(point: &'static str) -> Option<ArmedCut> {
    if std::env::var(CUT_ENV).ok().as_deref() != Some(point) {
        return None;
    }
    let marker = std::env::var_os(MARKER_ENV).map(PathBuf::from)?;
    let Ok(mut file) = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
    else {
        return None;
    };
    let _ = writeln!(file, "point={point} pid={}", std::process::id());
    let _ = file.sync_all();
    tracing::warn!(point, marker = %marker.display(), "K8 process cut armed");
    Some(match std::env::var_os(RELEASE_ENV) {
        Some(release) => ArmedCut::ReleaseGated(PathBuf::from(release)),
        None => ArmedCut::Immediate,
    })
}

/// Terminate this fault-injection server process at one exact, externally
/// armed cut.
///
/// When a release path is configured the hook first publishes the marker and
/// waits. That lets the parent prove an externally visible boundary (such as
/// the IPC `Owned` frame) before releasing the crash while the persistence
/// task remains stopped before its next operation.
pub(crate) async fn terminate_at(point: &'static str) {
    #[cfg(feature = "fault-injection")]
    {
        match arm(point) {
            Some(ArmedCut::Immediate) => {}
            Some(ArmedCut::ReleaseGated(release)) => {
                while !release.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
            None => return,
        }
        std::process::abort();
    }

    #[cfg(not(feature = "fault-injection"))]
    let _ = point;
}

/// Synchronous companion for filesystem code that cannot await.
pub(crate) fn terminate_sync_at(point: &'static str) {
    #[cfg(feature = "fault-injection")]
    {
        match arm(point) {
            Some(ArmedCut::Immediate) => {}
            Some(ArmedCut::ReleaseGated(release)) => {
                while !release.exists() {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            None => return,
        }
        std::process::abort();
    }

    #[cfg(not(feature = "fault-injection"))]
    let _ = point;
}

/// The in-process arming slot for [`panic_world_turn_if_armed`]. One-shot: the
/// turn that matches takes the line out, so a supervised relaunch inside the
/// same test process boots clean.
#[cfg(feature = "fault-injection")]
static ARMED_WORLD_TURN: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Arm the next world turn on `line` to panic, in THIS process.
///
/// The companion to the environment route below, for a supervised proof that
/// runs its server halves as threads rather than children: a test process
/// cannot set its own environment without racing every other thread reading it.
/// Feature-gated, so it does not exist in a shipped build.
#[cfg(feature = "fault-injection")]
pub fn arm_world_turn_panic(line: &str) {
    let mut armed = ARMED_WORLD_TURN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *armed = Some(line.trim().to_owned());
}

/// Whether `line` is the armed turn — and, if it is, DISARM before returning.
///
/// Two routes, both one-shot. The in-process slot serves a supervised proof
/// whose servers are threads; the environment plus a `create_new` marker serves
/// a real child process, whose replacement inherits the same environment and
/// must boot clean.
#[cfg(feature = "fault-injection")]
fn take_armed_world_turn(line: &str) -> bool {
    {
        let mut armed = ARMED_WORLD_TURN
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if armed.as_deref() == Some(line) {
            *armed = None;
            return true;
        }
    }
    if std::env::var(PANIC_CMD_ENV).ok().as_deref() != Some(line) {
        return false;
    }
    let Some(marker) = std::env::var_os(PANIC_MARKER_ENV).map(PathBuf::from) else {
        return false;
    };
    let Ok(mut file) = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
    else {
        return false;
    };
    let _ = writeln!(file, "cmd={line} pid={}", std::process::id());
    let _ = file.sync_all();
    true
}

/// Panic ONE world turn, from inside the world task's synchronous dispatch
/// boundary.
///
/// WS01-S8's claim is that a forked server whose world dies stops being a
/// server: it joins its outbox, its store writer and its IPC writer, and then
/// exits non-zero so the supervisor relaunches it (D2). Proving that end to end
/// needs a *real* `mush-server` process whose world *really* faults — nothing
/// simulated, because the whole question is what the process does afterwards.
///
/// Arming is the same shape as the process cuts above and inert for the same
/// reasons: the `fault-injection` feature (off by default, on in no shipped
/// binary), plus an environment naming the exact command line, plus a one-shot
/// marker created with `create_new` so the replacement process — which inherits
/// its parent's environment — runs clean.
///
/// The call site sits INSIDE the guarded dispatch, so this is a caught fault
/// (`WorldOutcome::Faulted`), not a task panic.
pub(crate) fn panic_world_turn_if_armed(inbound: &mush_core::Inbound) {
    #[cfg(feature = "fault-injection")]
    {
        let mush_core::Inbound::Cmd { line, .. } = inbound else {
            return;
        };
        let line = line.trim();
        if !take_armed_world_turn(line) {
            return;
        }
        panic!("WS01-S8 armed world-turn fault on {line:?}");
    }

    #[cfg(not(feature = "fault-injection"))]
    let _ = inbound;
}

#[cfg(test)]
mod tests {
    /// The name of the worker below, as libtest filters it.
    const WORKER: &str = "process_fault::tests::armed_worker";
    /// Set by the parent test on the child it re-execs, and by nothing else.
    const WORKER_ENV: &str = "RUSTMUSH_WS01_S11D_WORKER";

    /// One scratch directory for this test process.
    fn scratch() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "mush-ws01-s11d-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// The child half. Inert unless the parent re-execs this binary with
    /// `WORKER_ENV` set, so an ordinary `cargo test` run walks past it.
    ///
    /// When the parent does arm it, the whole crash-injection environment is
    /// set to a real cut (`persist.rs:1155`), a writable marker path and an
    /// already-published release path: nothing but the absence of the
    /// `fault-injection` feature stops the abort.
    #[tokio::test]
    async fn armed_worker() {
        if std::env::var_os(WORKER_ENV).is_none() {
            return;
        }
        super::terminate_sync_at("after_intent_before_ownership");
        super::terminate_at("after_intent_before_ownership").await;
    }

    /// The bypass this step exists to close, and its mirror.
    ///
    /// A fully armed environment reaches a child process that is this binary.
    /// Without the `fault-injection` feature the child must run both hooks to
    /// completion, exit cleanly, and never even publish the marker — `arm()`
    /// does not exist in that build. With the feature on, the same child must
    /// die, which is what proves the seam moved off `cfg(debug_assertions)`
    /// rather than disappearing. (The full crash-and-recover matrix is
    /// `crates/mush-gateway/tests/split_halves/authoritative_process_kill.rs`; it cannot
    /// live here, because it ends in `abort()`.)
    #[test]
    fn a_fully_armed_environment_reaches_the_hooks_only_with_the_feature() {
        let dir = scratch();
        let marker = dir.join("marker");
        let release = dir.join("release");
        std::fs::write(&release, b"go").unwrap();

        let exe = std::env::current_exe().expect("the test binary knows its own path");
        let output = std::process::Command::new(exe)
            .arg(WORKER)
            .arg("--exact")
            .arg("--test-threads=1")
            .env(WORKER_ENV, "1")
            .env("RUSTMUSH_K8_PROCESS_CUT", "after_intent_before_ownership")
            .env("RUSTMUSH_K8_PROCESS_CUT_MARKER", &marker)
            .env("RUSTMUSH_K8_PROCESS_CUT_RELEASE", &release)
            .output()
            .expect("re-exec the test binary");

        let armed = cfg!(feature = "fault-injection");
        assert_eq!(
            output.status.success(),
            !armed,
            "a child armed with RUSTMUSH_K8_PROCESS_CUT exited {:?}; with the \
             `fault-injection` feature {} the hooks must {}\n--- stderr ---\n{}",
            output.status,
            if armed { "on" } else { "off" },
            if armed { "abort" } else { "be a no-op" },
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            marker.exists(),
            armed,
            "the one-shot marker {} published; `arm()` {} in a build with the \
             `fault-injection` feature {}",
            if marker.exists() { "was" } else { "was not" },
            if marker.exists() {
                "ran"
            } else {
                "did not run"
            },
            if armed { "on" } else { "off" }
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

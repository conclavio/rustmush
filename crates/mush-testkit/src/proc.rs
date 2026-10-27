//! A shared child-process harness for the binaries' own tests (WS22-S1).
//!
//! `mushd`, `mush-server` and `mush-gateway` each need to prove things about
//! the *process*: that a mistyped flag exits non-zero and says which token was
//! wrong, that a signal reaches the shutdown path, that an exit code carries
//! the classification `docs/ops.md` promises. None of that is provable in
//! process — it needs the real binary, its stdout and stderr, and a hard
//! timeout so a wedged child fails the test instead of hanging the suite.
//!
//! `crates/mushd/tests/support/mod.rs`'s `ChildServer` had all of this for
//! `mushd` alone, tied to tokio's clock and to `mushd`'s own readiness lines.
//! This is the same shape, made shareable: deliberately **synchronous** and
//! std-only, because `mush-testkit` links no runtime and a plain `#[test]` is
//! the right home for "spawn it, read its stderr, check its exit status".
//!
//! ```ignore
//! // In `crates/mushd/tests/durable_process/cli_args.rs`, where CARGO_BIN_EXE_mushd exists:
//! let mut child = ChildProcess::builder(env!("CARGO_BIN_EXE_mushd"))
//!     .arg("--confg")
//!     .arg("prod.toml")
//!     .spawn()
//!     .expect("spawn");
//! let (status, log) = child.wait_with_log(Duration::from_secs(30)).expect("exit");
//! assert!(!status.success());
//! assert!(log.contains("--confg"));
//! ```

use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How often a wait loop re-checks. Small enough that a fast failure is still
/// fast, large enough not to spin a core.
const POLL: Duration = Duration::from_millis(25);

/// Build a [`ChildProcess`]: program, arguments, environment, working
/// directory.
pub struct ChildProcessBuilder {
    program: OsString,
    args: Vec<OsString>,
    envs: Vec<(OsString, Option<OsString>)>,
    current_dir: Option<PathBuf>,
}

impl ChildProcessBuilder {
    /// Add one argument.
    #[must_use]
    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    /// Add several arguments.
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|a| a.as_ref().to_os_string()));
        self
    }

    /// Set an environment variable for the child.
    #[must_use]
    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.envs.push((
            key.as_ref().to_os_string(),
            Some(value.as_ref().to_os_string()),
        ));
        self
    }

    /// Remove an environment variable the test runner may have set.
    #[must_use]
    pub fn env_remove(mut self, key: impl AsRef<OsStr>) -> Self {
        self.envs.push((key.as_ref().to_os_string(), None));
        self
    }

    /// Run the child in `dir`. Load-bearing for anything that resolves a
    /// relative path (the implicit `mushd.toml`, a relative `data_dir`).
    #[must_use]
    pub fn current_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.current_dir = Some(dir.into());
        self
    }

    /// Spawn it, with stdout and stderr piped into one captured log.
    ///
    /// # Errors
    ///
    /// The spawn failure, verbatim — usually a binary that was not built.
    pub fn spawn(self) -> std::io::Result<ChildProcess> {
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            // The harness waits on INFO-level readiness markers. Never inherit
            // a developer or CI `RUST_LOG=warn` that suppresses them and turns
            // a healthy child into a timeout.
            .env("RUST_LOG", "info")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in &self.envs {
            match value {
                Some(value) => command.env(key, value),
                None => command.env_remove(key),
            };
        }
        if let Some(dir) = &self.current_dir {
            command.current_dir(dir);
        }
        let mut child = command.spawn()?;

        let log = Arc::new(Mutex::new(String::new()));
        for stream in [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let log = Arc::clone(&log);
            std::thread::spawn(move || {
                let reader = BufReader::new(stream);
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    let Ok(mut log) = log.lock() else { break };
                    log.push_str(&line);
                    log.push('\n');
                }
            });
        }
        Ok(ChildProcess {
            label: self.program.to_string_lossy().into_owned(),
            child,
            log,
        })
    }
}

/// A running binary with its output captured line by line.
///
/// Dropping it kills the child, so a failed assertion never leaves a stray
/// server holding a port.
pub struct ChildProcess {
    label: String,
    child: Child,
    log: Arc<Mutex<String>>,
}

impl ChildProcess {
    /// Start describing a child process to spawn.
    #[must_use]
    pub fn builder(program: impl AsRef<OsStr>) -> ChildProcessBuilder {
        ChildProcessBuilder {
            program: program.as_ref().to_os_string(),
            args: Vec::new(),
            envs: Vec::new(),
            current_dir: None,
        }
    }

    /// Everything the child has written to stdout or stderr so far, with ANSI
    /// escapes removed.
    #[must_use]
    pub fn log_snapshot(&self) -> String {
        let text = match self.log.lock() {
            Ok(log) => log.clone(),
            // A reader thread that panicked mid-push is a test-harness fault,
            // not a fact about the child: read what is there and let the
            // caller's assertion speak.
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        strip_ansi(&text)
    }

    /// Block until `needle` appears in the child's log.
    ///
    /// # Errors
    ///
    /// A message naming the needle and quoting the whole log, on timeout or if
    /// the child exits first without ever printing it.
    pub fn await_log(&mut self, needle: &str, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.log_snapshot().contains(needle) {
                return Ok(());
            }
            let exited = self.child.try_wait().ok().flatten();
            if let Some(status) = exited {
                // One last look: the writer threads may still be draining.
                std::thread::sleep(POLL);
                if self.log_snapshot().contains(needle) {
                    return Ok(());
                }
                return Err(format!(
                    "{} exited {status} before printing {needle:?}; log:\n{}",
                    self.label,
                    self.log_snapshot()
                ));
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "timeout after {timeout:?} waiting for {needle:?} from {}; log:\n{}",
                    self.label,
                    self.log_snapshot()
                ));
            }
            std::thread::sleep(POLL);
        }
    }

    /// Block until the child exits on its own.
    ///
    /// # Errors
    ///
    /// A message quoting the log, if it is still running at the deadline.
    pub fn wait_exit(&mut self, timeout: Duration) -> Result<ExitStatus, String> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return Ok(status),
                Ok(None) => {}
                Err(e) => return Err(format!("{}: try_wait: {e}", self.label)),
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "{} did not exit within {timeout:?}; log:\n{}",
                    self.label,
                    self.log_snapshot()
                ));
            }
            std::thread::sleep(POLL);
        }
    }

    /// [`ChildProcess::wait_exit`] plus the log, after giving the reader
    /// threads a moment to drain the pipes the exit just closed.
    ///
    /// # Errors
    ///
    /// As [`ChildProcess::wait_exit`].
    pub fn wait_with_log(&mut self, timeout: Duration) -> Result<(ExitStatus, String), String> {
        let status = self.wait_exit(timeout)?;
        std::thread::sleep(POLL * 4);
        Ok((status, self.log_snapshot()))
    }

    /// Ask the child to stop the way an operator or an init system would:
    /// `SIGTERM` on Unix, and — Windows having no such signal for a console
    /// application spawned this way — `TerminateProcess` there.
    ///
    /// The Unix side shells out to `kill(1)` rather than take a `libc`
    /// dependency for one call in a test harness.
    pub fn terminate(&mut self) {
        #[cfg(unix)]
        {
            let pid = self.child.id().to_string();
            let _ = Command::new("kill")
                .arg("-TERM")
                .arg(&pid)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        #[cfg(not(unix))]
        {
            self.kill_hard();
        }
    }

    /// `kill -9` equivalent: no shutdown hook, no drain. The process stops.
    pub fn kill_hard(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// The child's OS process id.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for ChildProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Drop CSI escape sequences so an assertion matches the text a human reads.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            for f in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&f) {
                    break;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::strip_ansi;

    #[test]
    fn ansi_is_stripped() {
        assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m text"), "red text");
    }
}

//! The one terminate/interrupt future the three binaries share (WS22-S2).
//!
//! `mushd` grew a private `terminate_signal()` and a `select!` with two arms —
//! one for `tokio::signal::ctrl_c()`, one for SIGTERM — whose bodies were
//! byte-identical: stop accepting, `Control(Shutdown)`, join the world. The
//! other two binaries had nothing: `mush-server` installed no handler at all,
//! and `mush-gateway` awaited a bare `ctrl_c()`, so `systemctl stop` on the
//! split reached a server half that only died because the OS killed it. One
//! future, [`shutdown_signal`], replaces all of that: it resolves once, saying
//! WHICH signal arrived, and every binary spends it on the same drain.
//!
//! Two rules are load-bearing and easy to get backwards:
//!
//! - **A handler that cannot be installed is not fatal.** A restricted
//!   container that refuses the `sigaction` still has to boot; the arm logs
//!   once and becomes [`std::future::pending`], which is exactly what `mushd`
//!   did before. A process that cannot hear SIGTERM is a process an operator
//!   has to `kill -9` — it is not a process that refuses to start.
//! - **A supervised child ignores SIGINT, and only because the marker says
//!   so.** Until doc 33 GG.1 `mush-gateway` spawned its `mush-server` child
//!   with no `setsid`, so the child shared the gateway's process group and a
//!   terminal Ctrl-C was delivered to BOTH halves; a child started with
//!   [`SUPERVISED_CHILD_ENV`] set keeps waiting when SIGINT arrives, so the
//!   parent stays the only initiator. Since GG.1 the server is its own unit
//!   and nothing in the tree sets the marker; it stays for any embedding that
//!   owns a child. The test is the env marker and nothing else: keying it
//!   off "do I have a parent" would make a standalone `mush-server` under
//!   Ctrl-C unstoppable.
//!
//! Ignoring SIGINT still means *installing* its handler. An installed handler
//! is what neutralises the keystroke; skipping the install would leave SIGINT
//! at its default disposition and the supervised child would die on the very
//! Ctrl-C the gateway is supposed to own.
//!
//! Nothing here may panic — `mush-config` denies `clippy::panic`/`unwrap_used`
//! at its crate root and is a ratchet crate in the panic-surface budget — so
//! every failure path logs and degrades instead.

use std::future::pending;

/// The env var a supervisor sets on the child process it owns.
///
/// Was set by `mush-gateway` when it spawned `mush-server` (WS22-S7); since
/// doc 33 GG.1 nothing in the tree sets it. Read here by
/// [`supervised_child`]. Any non-empty value other than `0` means yes.
pub const SUPERVISED_CHILD_ENV: &str = "RUSTMUSH_SUPERVISED_CHILD";

/// Which signal ended the wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownSignal {
    /// SIGINT — Ctrl-C at the console.
    Interrupt,
    /// SIGTERM — `systemctl stop`, `kill <pid>`. Unix only.
    Terminate,
}

impl ShutdownSignal {
    /// The name an operator reading the log will recognise.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Interrupt => "Ctrl-C",
            Self::Terminate => "SIGTERM",
        }
    }
}

impl std::fmt::Display for ShutdownSignal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Whether this process was started by a supervisor that owns its shutdown.
#[must_use]
pub fn supervised_child() -> bool {
    marker_says_supervised(std::env::var(SUPERVISED_CHILD_ENV).ok().as_deref())
}

/// The marker's reading, split out so the rule is testable without mutating
/// the process environment (`unsafe` in edition 2024, and denied workspace
/// wide).
fn marker_says_supervised(value: Option<&str>) -> bool {
    match value {
        Some(v) => {
            let v = v.trim();
            !v.is_empty() && v != "0"
        }
        None => false,
    }
}

/// Resolve when the operator (or the init system) asks this process to stop.
///
/// Never resolves on its own: a boot that hangs here is a boot nobody
/// interrupted, and the one thing this future must never do is turn a start-up
/// into a shutdown.
pub async fn shutdown_signal() -> ShutdownSignal {
    shutdown_signal_with(supervised_child()).await
}

#[cfg(unix)]
async fn shutdown_signal_with(ignore_interrupt: bool) -> ShutdownSignal {
    use tokio::signal::unix::{Signal, SignalKind, signal};

    fn install(kind: SignalKind, name: &str) -> Option<Signal> {
        match signal(kind) {
            Ok(sig) => Some(sig),
            Err(e) => {
                // Non-fatal by design: the process boots, it just cannot be
                // stopped by this signal. Said once, loudly, at start-up.
                tracing::error!(
                    error = %e, signal = name,
                    "cannot install signal handler; this process will not stop on it"
                );
                None
            }
        }
    }

    // BOTH are installed even when SIGINT is to be ignored — see the module
    // docs: the installed handler is what makes the keystroke harmless.
    let mut terminate = install(SignalKind::terminate(), "SIGTERM");
    let mut interrupt = install(SignalKind::interrupt(), "SIGINT");
    let mut noted = false;
    loop {
        tokio::select! {
            () = next(&mut terminate) => return ShutdownSignal::Terminate,
            () = next(&mut interrupt) => {
                if !ignore_interrupt {
                    return ShutdownSignal::Interrupt;
                }
                if !noted {
                    noted = true;
                    tracing::info!(
                        "Ctrl-C ignored: supervised child, the supervisor initiates shutdown"
                    );
                }
            }
        }
    }
}

/// One delivery of `sig`, or a wait that never ends when there is no handler
/// (or the stream closed under us — pending beats spinning).
#[cfg(unix)]
async fn next(sig: &mut Option<tokio::signal::unix::Signal>) {
    match sig {
        Some(s) => {
            if s.recv().await.is_none() {
                pending::<()>().await;
            }
        }
        None => pending::<()>().await,
    }
}

/// Windows has no SIGTERM for a console application, so Ctrl-C is the whole
/// vocabulary; [`ShutdownSignal::Terminate`] simply never occurs there.
#[cfg(not(unix))]
async fn shutdown_signal_with(ignore_interrupt: bool) -> ShutdownSignal {
    let mut noted = false;
    loop {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                if !ignore_interrupt {
                    return ShutdownSignal::Interrupt;
                }
                if !noted {
                    noted = true;
                    tracing::info!(
                        "Ctrl-C ignored: supervised child, the supervisor initiates shutdown"
                    );
                }
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "cannot install Ctrl-C handler; this process will not stop on it"
                );
                pending::<()>().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// How long "still pending" is worth waiting for. Long enough that a
    /// spurious resolution has to be genuinely rare to slip through, short
    /// enough to keep the suite quick.
    const PENDS: Duration = Duration::from_millis(250);

    #[test]
    fn the_marker_is_read_strictly() {
        assert!(!marker_says_supervised(None));
        assert!(!marker_says_supervised(Some("")));
        assert!(!marker_says_supervised(Some("  ")));
        assert!(!marker_says_supervised(Some("0")));
        assert!(marker_says_supervised(Some("1")));
        assert!(marker_says_supervised(Some("yes")));
    }

    /// The two failure modes worth a process's life, in one sequential test —
    /// deliberately ONE test function, because it raises real signals at this
    /// very process and a second signal-touching test running beside it could
    /// have its future resolved by the delivery this one made.
    ///
    /// 1. With nothing delivered the future must still be pending. A future
    ///    that resolves spuriously turns a boot into a shutdown.
    /// 2. With the supervised marker set, a delivered SIGINT must NOT resolve
    ///    it (the gateway's Ctrl-C reaches the child through the shared
    ///    process group) while SIGTERM still must — a child that ignored both
    ///    would be unkillable by its supervisor.
    #[tokio::test]
    async fn the_future_pends_until_a_signal_it_honours_arrives() {
        assert!(
            tokio::time::timeout(PENDS, shutdown_signal_with(false))
                .await
                .is_err(),
            "the shutdown future resolved with no signal delivered"
        );

        #[cfg(unix)]
        {
            let supervised = shutdown_signal_with(true);
            tokio::pin!(supervised);
            // Poll it once FIRST so both handlers are installed before
            // anything is raised: an un-neutralised SIGTERM would kill this
            // test process rather than fail an assertion.
            assert!(
                tokio::time::timeout(PENDS, &mut supervised).await.is_err(),
                "the supervised shutdown future resolved with no signal delivered"
            );

            raise("INT");
            assert!(
                tokio::time::timeout(Duration::from_millis(500), &mut supervised)
                    .await
                    .is_err(),
                "SIGINT resolved a supervised child's shutdown future; the gateway is \
                 supposed to be the only initiator"
            );

            raise("TERM");
            let got = tokio::time::timeout(Duration::from_secs(5), &mut supervised).await;
            assert_eq!(
                got,
                Ok(ShutdownSignal::Terminate),
                "a supervised child must still stop on SIGTERM"
            );
        }
    }

    /// Send `sig` to this very process. Shells out to `kill(1)` rather than
    /// take a `libc` dependency for one call (`mush-testkit` does the same).
    #[cfg(unix)]
    fn raise(sig: &str) {
        let status = std::process::Command::new("kill")
            .arg(format!("-{sig}"))
            .arg(std::process::id().to_string())
            .status()
            .unwrap_or_else(|e| panic!("running kill -{sig}: {e}"));
        assert!(status.success(), "kill -{sig} exited {status}");
    }
}

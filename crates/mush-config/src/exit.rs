//! The one process-exit vocabulary the three binaries share (WS22-S3).
//!
//! Before this module every binary spoke exactly two codes:
//! [`ExitCode::SUCCESS`] and [`ExitCode::FAILURE`]. A supervisor watching
//! `mushd` or `mush-server` could therefore tell "it stopped badly" from "it
//! stopped well" and nothing else — and the two endings an operator most needs
//! to tell apart are exactly the two that looked identical: a world task that
//! **died** (panicked or faulted mid-command, its RAM torn, no snapshot taken)
//! and a world task that is merely **slow to stop** (still finishing an exact
//! commit when the join bound expired). The first is a bug to page on; the
//! second is a box under load, and restarting it harder makes it worse.
//!
//! So the vocabulary is small, fixed, and shared:
//!
//! | code | class | meaning |
//! |---|---|---|
//! | 0 | [`ExitClass::Ok`] | the shutdown sequence ran to the end |
//! | 1 | [`ExitClass::Failure`] | a fatal error, **including a world fault or panic** |
//! | 2 | [`ExitClass::QuarantineLockHeld`] | another process holds the quarantine lifetime lock (reserved, WS22-S18) |
//! | 3 | [`ExitClass::WorldJoinTimeout`] | the world task did not join within its bound |
//!
//! Three rules keep it worth having:
//!
//! - **One table, both halves.** `mushd` and `mush-server` map onto the same
//!   numbers, because the split and the all-in-one are the same game and an
//!   operator should not have to remember which binary produced a 3.
//! - **1 stays the catch-all.** A world panic is a `1` and so is a bad config,
//!   because `1` is what every convention already reads as "it failed"; the log
//!   says which. The codes above it exist only where the *response* differs.
//! - **Reserved is reserved.** [`ExitClass::QuarantineLockHeld`] has no producer
//!   yet (WS22-S18 gives it one). It sits in the table anyway, because the cost
//!   of leaving a hole is zero and the cost of finding one later — after an
//!   operator's unit file already treats `2` as something else — is a silent
//!   misclassification.
//!
//! Every class is non-zero except [`ExitClass::Ok`], so a systemd
//! `Restart=on-failure` fires for all of them.

use std::process::ExitCode;

/// How this process ended, in the one vocabulary both halves speak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitClass {
    /// The shutdown sequence ran to the end.
    Ok,
    /// A fatal error: a failed boot, a refused command line, or a world task
    /// that faulted or panicked. The log line says which.
    Failure,
    /// Another live process holds the quarantine spool's exclusive lifetime
    /// lock, so this one must not touch the integrity store.
    ///
    /// Reserved by WS22-S3, produced by WS22-S18. Distinct because the
    /// operator response is distinct: not "look at the logs", but "you already
    /// have one of these running".
    QuarantineLockHeld,
    /// The world task did not join within [its bound](crate::exit), so the
    /// process is stopping without knowing how the world ended.
    ///
    /// Deliberately NOT [`ExitClass::Failure`]: a slow stop and a dead world
    /// are different incidents with the same non-zero exit, and only the code
    /// tells them apart at the supervisor.
    WorldJoinTimeout,
}

impl ExitClass {
    /// The numeric code this class exits with.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Ok => 0,
            Self::Failure => 1,
            Self::QuarantineLockHeld => 2,
            Self::WorldJoinTimeout => 3,
        }
    }

    /// The [`ExitCode`] a `main` returns for this class.
    ///
    /// `ExitCode::from(0)`/`from(1)` are the same values as
    /// `ExitCode::SUCCESS`/`FAILURE`; going through the table keeps one place
    /// to read.
    #[must_use]
    pub fn exit_code(self) -> ExitCode {
        ExitCode::from(self.code())
    }

    /// The short name an operator reading the log or a runbook will recognise.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Failure => "failure",
            Self::QuarantineLockHeld => "quarantine-lock-held",
            Self::WorldJoinTimeout => "world-join-timeout",
        }
    }

    /// Every class, so a test can walk the whole table without a `match` that
    /// a new variant would silently widen.
    pub const ALL: [Self; 4] = [
        Self::Ok,
        Self::Failure,
        Self::QuarantineLockHeld,
        Self::WorldJoinTimeout,
    ];
}

impl std::fmt::Display for ExitClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// A fatal ending: WHY it happened, in words for the operator, plus WHICH code
/// it exits with.
///
/// The two travel together because they always did — a binary that returned a
/// bare `String` from `serve` had to pick its exit code at the `match` in
/// `main`, which is the one place that has already lost the reason. Carrying
/// the class with the text means the site that knows a world join expired is
/// the site that says so.
///
/// [`From<String>`] yields [`ExitClass::Failure`], so the ordinary `?` on the
/// `Result<_, String>` boot paths keeps meaning "this failed" and only the
/// endings that need their own code have to name one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitFailure {
    /// The code this ending exits with.
    pub class: ExitClass,
    /// The operator-facing reason, printed to stderr by `main`.
    pub detail: String,
}

impl ExitFailure {
    /// A failure in a named class.
    #[must_use]
    pub fn new(class: ExitClass, detail: impl Into<String>) -> Self {
        Self {
            class,
            detail: detail.into(),
        }
    }

    /// The ordinary catch-all: [`ExitClass::Failure`].
    #[must_use]
    pub fn failure(detail: impl Into<String>) -> Self {
        Self::new(ExitClass::Failure, detail)
    }
}

impl From<String> for ExitFailure {
    fn from(detail: String) -> Self {
        Self::failure(detail)
    }
}

impl std::fmt::Display for ExitFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}

impl std::error::Error for ExitFailure {}

#[cfg(test)]
mod tests {
    use super::{ExitClass, ExitFailure};

    /// The table itself. Written out longhand rather than derived from the
    /// `match`, because the whole value of a shared vocabulary is that the
    /// numbers do not move: an operator's unit file, a runbook and a
    /// supervisor's classifier all encode them out of tree.
    #[test]
    fn the_codes_are_the_documented_ones() {
        assert_eq!(ExitClass::Ok.code(), 0);
        assert_eq!(ExitClass::Failure.code(), 1);
        assert_eq!(ExitClass::QuarantineLockHeld.code(), 2);
        assert_eq!(ExitClass::WorldJoinTimeout.code(), 3);
    }

    /// The failure mode a shared vocabulary exists to prevent: two classes
    /// collapsing onto one number, so a supervisor cannot tell a dead world
    /// from a slow one. Walks `ALL`, so adding a variant without a code fails
    /// here rather than in production.
    #[test]
    fn every_class_has_its_own_code() {
        let mut seen: Vec<u8> = ExitClass::ALL.iter().map(|c| c.code()).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(
            seen.len(),
            before,
            "two exit classes share a code: {:?}",
            ExitClass::ALL.map(|c| (c.label(), c.code()))
        );
    }

    /// Only a clean end is zero. Every other class has to trip a
    /// `Restart=on-failure`.
    #[test]
    fn only_a_clean_end_is_zero() {
        for class in ExitClass::ALL {
            assert_eq!(
                class.code() == 0,
                class == ExitClass::Ok,
                "{class} is on the wrong side of zero"
            );
        }
    }

    /// The conversion the boot paths ride on: a plain error string keeps
    /// meaning "it failed", so only the endings that earned a distinct code
    /// have to say one. If this ever silently produced something else, every
    /// `?` in a `serve` body would start lying about how the process ended.
    #[test]
    fn a_bare_error_string_is_the_catch_all_class() {
        let failure = ExitFailure::from("store unreachable".to_owned());
        assert_eq!(failure.class, ExitClass::Failure);
        assert_eq!(failure.to_string(), "store unreachable");
        assert_eq!(
            ExitFailure::new(ExitClass::WorldJoinTimeout, "slow")
                .class
                .code(),
            3
        );
    }
}

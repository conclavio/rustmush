//! Test-only checks for functions that return `Result`.
//!
//! This crate's root denies `clippy::panic_in_result_fn` (WS01-S12 / 17d
//! M6.1), and that lint has no test exemption: an `assert!` in a
//! `#[test] fn … -> Result`, or in a fixture helper that returns `Result`, is
//! an error. These macros are the asserts' replacements — the same condition
//! and the same message, with an early `return Err(..)` where the assert
//! panicked:
//!
//! - `check!(cond)` / `check!(cond, "fmt", args..)` for `assert!`;
//! - `check_eq!(left, right[, "fmt", args..])` for `assert_eq!`;
//! - `check_ne!(left, right[, "fmt", args..])` for `assert_ne!`.
//!
//! They are declared first in `lib.rs` because `macro_rules!` is textually
//! scoped, and they neither panic nor unwrap. The message is built out of
//! line, in [`failed`] and its siblings, so a check costs its caller's stack
//! frame no more than the assert it replaced (`clippy::large_stack_frames`
//! measures test functions too). A `macro_rules!` does not cross crates; the
//! same module lives in `mush-industry`.

use std::error::Error;
use std::fmt;

/// The error a failed `check!`, `check_eq!` or `check_ne!` returns.
pub(crate) struct CheckFailed(String);

/// The bare message, like a failed assert's: a test that returns this error
/// prints it through `Debug`, and quoting it would fold a multi-line
/// `check_eq!` report onto one line.
impl fmt::Debug for CheckFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for CheckFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for CheckFailed {}

/// A few tests here return the error of the code under test rather than
/// `Box<dyn Error>`; a failed check reaches them as that error's message.
impl From<CheckFailed> for mush_core::ModError {
    fn from(failed: CheckFailed) -> Self {
        Self::msg(failed.0)
    }
}

impl From<CheckFailed> for crate::RollError {
    fn from(failed: CheckFailed) -> Self {
        crate::invalid("check", failed.0)
    }
}

/// A failed `check!(cond)`.
pub(crate) fn failed<E: From<CheckFailed>>(message: &str) -> E {
    E::from(CheckFailed(message.to_owned()))
}

/// A failed `check!(cond, "fmt", args..)`.
pub(crate) fn failed_with<E: From<CheckFailed>>(message: fmt::Arguments<'_>) -> E {
    E::from(CheckFailed(message.to_string()))
}

/// A failed `check_eq!`/`check_ne!` without a message; `op` is `==` or `!=`.
pub(crate) fn failed_cmp<E, L, R>(op: &str, left: &L, right: &R) -> E
where
    E: From<CheckFailed>,
    L: fmt::Debug + ?Sized,
    R: fmt::Debug + ?Sized,
{
    E::from(CheckFailed(format!(
        "assertion `left {op} right` failed\n  left: {left:?}\n right: {right:?}"
    )))
}

/// A failed `check_eq!`/`check_ne!` with a message; `op` is `==` or `!=`.
pub(crate) fn failed_cmp_with<E, L, R>(
    op: &str,
    left: &L,
    right: &R,
    message: fmt::Arguments<'_>,
) -> E
where
    E: From<CheckFailed>,
    L: fmt::Debug + ?Sized,
    R: fmt::Debug + ?Sized,
{
    E::from(CheckFailed(format!(
        "assertion `left {op} right` failed: {message}\n  left: {left:?}\n right: {right:?}"
    )))
}

/// `assert!` for a function that returns `Result`.
macro_rules! check {
    ($cond:expr $(,)?) => {
        if !$cond {
            return Err($crate::test_checks::failed(concat!(
                "assertion failed: ",
                stringify!($cond)
            )));
        }
    };
    ($cond:expr, $($arg:tt)+) => {
        if !$cond {
            return Err($crate::test_checks::failed_with(format_args!($($arg)+)));
        }
    };
}

/// `assert_eq!` for a function that returns `Result`.
///
/// The operands are matched, not `let`-bound, as `assert_eq!` does: a
/// temporary either side borrows (`f()?.as_ref()`) lives to the comparison.
macro_rules! check_eq {
    ($left:expr, $right:expr $(,)?) => {
        match (&$left, &$right) {
            (left_val, right_val) => {
                if !(*left_val == *right_val) {
                    return Err($crate::test_checks::failed_cmp("==", left_val, right_val));
                }
            }
        }
    };
    ($left:expr, $right:expr, $($arg:tt)+) => {
        match (&$left, &$right) {
            (left_val, right_val) => {
                if !(*left_val == *right_val) {
                    return Err($crate::test_checks::failed_cmp_with(
                        "==",
                        left_val,
                        right_val,
                        format_args!($($arg)+),
                    ));
                }
            }
        }
    };
}

/// `assert_ne!` for a function that returns `Result`.
macro_rules! check_ne {
    ($left:expr, $right:expr $(,)?) => {
        match (&$left, &$right) {
            (left_val, right_val) => {
                if *left_val == *right_val {
                    return Err($crate::test_checks::failed_cmp("!=", left_val, right_val));
                }
            }
        }
    };
    ($left:expr, $right:expr, $($arg:tt)+) => {
        match (&$left, &$right) {
            (left_val, right_val) => {
                if *left_val == *right_val {
                    return Err($crate::test_checks::failed_cmp_with(
                        "!=",
                        left_val,
                        right_val,
                        format_args!($($arg)+),
                    ));
                }
            }
        }
    };
}

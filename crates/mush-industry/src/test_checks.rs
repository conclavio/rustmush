//! Test-only checks for functions that return `Result`.
//!
//! This crate's root denies `clippy::panic_in_result_fn` (WS01-S12), and that
//! lint has no test exemption: an `assert!` in a
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
//! scoped, and they neither panic nor unwrap. The same module lives in
//! `mush-economy` and `mush-roll`; a `macro_rules!` does not cross crates.

use std::error::Error;
use std::fmt;

/// The error a failed `check!`, `check_eq!` or `check_ne!` returns.
pub(crate) struct CheckFailed(String);

impl CheckFailed {
    pub(crate) fn new(message: String) -> Self {
        Self(message)
    }
}

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

/// `assert!` for a function that returns `Result`.
macro_rules! check {
    ($cond:expr $(,)?) => {
        if !$cond {
            return Err($crate::test_checks::CheckFailed::new(
                concat!("assertion failed: ", stringify!($cond)).to_owned(),
            )
            .into());
        }
    };
    ($cond:expr, $($arg:tt)+) => {
        if !$cond {
            return Err($crate::test_checks::CheckFailed::new(format!($($arg)+)).into());
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
                    return Err($crate::test_checks::CheckFailed::new(format!(
                        "assertion `left == right` failed\n  left: {:?}\n right: {:?}",
                        left_val, right_val
                    ))
                    .into());
                }
            }
        }
    };
    ($left:expr, $right:expr, $($arg:tt)+) => {
        match (&$left, &$right) {
            (left_val, right_val) => {
                if !(*left_val == *right_val) {
                    return Err($crate::test_checks::CheckFailed::new(format!(
                        "assertion `left == right` failed: {}\n  left: {:?}\n right: {:?}",
                        format_args!($($arg)+),
                        left_val,
                        right_val
                    ))
                    .into());
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
                    return Err($crate::test_checks::CheckFailed::new(format!(
                        "assertion `left != right` failed\n  left: {:?}\n right: {:?}",
                        left_val, right_val
                    ))
                    .into());
                }
            }
        }
    };
    ($left:expr, $right:expr, $($arg:tt)+) => {
        match (&$left, &$right) {
            (left_val, right_val) => {
                if *left_val == *right_val {
                    return Err($crate::test_checks::CheckFailed::new(format!(
                        "assertion `left != right` failed: {}\n  left: {:?}\n right: {:?}",
                        format_args!($($arg)+),
                        left_val,
                        right_val
                    ))
                    .into());
                }
            }
        }
    };
}

//! Test-only checks for functions that return `Result`.
//!
//! This crate's root denies `clippy::panic_in_result_fn` (WS01-S12), and that
//! lint has no test exemption: an `assert!` in a `#[test] fn … -> Result`, or in
//! a fixture helper that returns `Result`, is an error. These macros are the
//! asserts' replacements — the same condition and the same message, with an
//! early `return Err(..)` where the assert panicked:
//!
//! - `check!(cond)` / `check!(cond, "fmt", args..)` for `assert!`;
//! - `check_eq!(left, right[, "fmt", args..])` for `assert_eq!`;
//! - `check_ne!(left, right[, "fmt", args..])` for `assert_ne!`.
//!
//! They are declared first in `lib.rs` because `macro_rules!` is textually
//! scoped, and they neither panic nor unwrap.
//!
//! A failed check is a [`CheckFailed`], so a failure can be told from an
//! ordinary refusal: a test that expects a helper to refuse asks [`refused`],
//! which hands a failed check inside that helper back as the test failure it is
//! instead of counting it as the refusal.

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

/// `Ok(true)` when `result` is a refusal, `Ok(false)` when it succeeded, and
/// `Err` — the failure itself — when the call failed one of these checks.
///
/// `check!(refused(call)?)` keeps what `assert!(call.is_err())` meant while
/// `call`'s own fixture checks were asserts: a broken fixture fails the test,
/// it is never the refusal the test is looking for.
pub(crate) fn refused<T>(result: Result<T, Box<dyn Error>>) -> Result<bool, Box<dyn Error>> {
    match result {
        Ok(_) => Ok(false),
        Err(error) if error.is::<CheckFailed>() => Err(error),
        Err(_) => Ok(true),
    }
}

/// `assert!` for a function that returns `Result<_, Box<dyn Error>>`.
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

/// `assert_eq!` for a function that returns `Result<_, Box<dyn Error>>`.
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

/// `assert_ne!` for a function that returns `Result<_, Box<dyn Error>>`.
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

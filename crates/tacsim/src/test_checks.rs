#![cfg(test)]

//! Test-only checks for functions that return `Result`.
//!
//! This crate's root denies `clippy::panic_in_result_fn` (WS01-S12), and
//! that lint has no test exemption: an `assert!` in a
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
//! scoped, and they neither panic nor unwrap. The failure is built out of
//! line, in [`failed`] and its siblings, so a check costs its caller's stack
//! frame no more than the assert it replaced (`clippy::large_stack_frames`
//! measures test functions too). A `macro_rules!` does not cross crates, so each
//! crate that needs these keeps this module as its own copy: `mush-economy`,
//! `mush-industry`, `mush-market`, `mush-roll` and `tacsim`.
//!
//! A failed check is a [`CheckFailed`]. A test that expects a helper to refuse
//! asks [`refused`] (or [`accepted`], or [`is_refusal`] where it cannot return
//! `Err`), which hands a failed check inside that helper back as the test
//! failure it is instead of counting it as the refusal.

use std::any::Any;
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

/// One test-local `RevisionAdapter` returns the Roll error rather than
/// `Box<dyn Error>`; a failed check reaches it as that error's message.
impl From<CheckFailed> for mush_roll::RollError {
    fn from(failed: CheckFailed) -> Self {
        Self::Invalid {
            field: "check".into(),
            message: failed.0,
        }
    }
}

/// The failed check `error` carries, through whichever box it travelled in.
fn failed_check<E: 'static>(error: &E) -> Option<&CheckFailed> {
    let error: &dyn Any = error;
    if let Some(boxed) = error.downcast_ref::<Box<dyn Error>>() {
        return (**boxed).downcast_ref::<CheckFailed>();
    }
    if let Some(boxed) = error.downcast_ref::<Box<dyn Error + Send + Sync>>() {
        return (**boxed).downcast_ref::<CheckFailed>();
    }
    error.downcast_ref::<CheckFailed>()
}

/// `Ok(Some(value))` when `result` succeeded, `Ok(None)` when it refused, and
/// `Err` — the failure itself — when the call failed one of these checks.
pub(crate) fn accepted<T, E: 'static>(result: Result<T, E>) -> Result<Option<T>, Box<dyn Error>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) => match failed_check(&error) {
            Some(failed) => Err(Box::new(CheckFailed(failed.0.clone()))),
            None => Ok(None),
        },
    }
}

/// `Ok(true)` when `result` is a refusal, `Ok(false)` when it succeeded, and
/// `Err` — the failure itself — when the call failed one of these checks.
///
/// `check!(refused(call)?)` keeps what `assert!(call.is_err())` meant while
/// `call`'s own fixture checks were asserts: a broken fixture fails the test,
/// it is never the refusal the test is looking for.
pub(crate) fn refused<T, E: 'static>(result: Result<T, E>) -> Result<bool, Box<dyn Error>> {
    Ok(accepted(result)?.is_none())
}

/// [`refused`] for a test body that cannot return `Err` (a `()` test, a
/// closure, an async block): a failed check panics there, as its assert did.
pub(crate) fn is_refusal<T, E: 'static>(result: Result<T, E>) -> bool {
    match accepted(result) {
        Ok(value) => value.is_none(),
        Err(failed) => panic!("{failed:?}"),
    }
}

/// A failed `check!(cond)`. It returns the caller's whole `Err`, so the
/// expansion writes straight into the caller's return slot.
pub(crate) fn failed<T, E: From<CheckFailed>>(message: &str) -> Result<T, E> {
    Err(E::from(CheckFailed(message.to_owned())))
}

/// A failed `check!(cond, "fmt", args..)`.
pub(crate) fn failed_with<T, E: From<CheckFailed>>(message: fmt::Arguments<'_>) -> Result<T, E> {
    Err(E::from(CheckFailed(message.to_string())))
}

/// A failed `check_eq!`/`check_ne!` without a message; `op` is `==` or `!=`.
pub(crate) fn failed_cmp<T, E, L, R>(op: &str, left: &L, right: &R) -> Result<T, E>
where
    E: From<CheckFailed>,
    L: fmt::Debug + ?Sized,
    R: fmt::Debug + ?Sized,
{
    // `{:>6}` right-aligns the two labels as `assert_eq!` does, without a run of
    // spaces inside a literal (the crates' `vf_z1a` scans read this file).
    Err(E::from(CheckFailed(format!(
        "assertion `left {op} right` failed\n{:>6}: {left:?}\n{:>6}: {right:?}",
        "left", "right"
    ))))
}

/// A failed `check_eq!`/`check_ne!` with a message; `op` is `==` or `!=`.
pub(crate) fn failed_cmp_with<T, E, L, R>(
    op: &str,
    left: &L,
    right: &R,
    message: fmt::Arguments<'_>,
) -> Result<T, E>
where
    E: From<CheckFailed>,
    L: fmt::Debug + ?Sized,
    R: fmt::Debug + ?Sized,
{
    Err(E::from(CheckFailed(format!(
        "assertion `left {op} right` failed: {message}\n{:>6}: {left:?}\n{:>6}: {right:?}",
        "left", "right"
    ))))
}

/// `assert!` for a function that returns `Result`.
macro_rules! check {
    ($cond:expr $(,)?) => {
        if !$cond {
            return $crate::test_checks::failed(concat!("assertion failed: ", stringify!($cond)));
        }
    };
    ($cond:expr, $($arg:tt)+) => {
        if !$cond {
            return $crate::test_checks::failed_with(format_args!($($arg)+));
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
                    return $crate::test_checks::failed_cmp("==", left_val, right_val);
                }
            }
        }
    };
    ($left:expr, $right:expr, $($arg:tt)+) => {
        match (&$left, &$right) {
            (left_val, right_val) => {
                if !(*left_val == *right_val) {
                    return $crate::test_checks::failed_cmp_with(
                        "==",
                        left_val,
                        right_val,
                        format_args!($($arg)+),
                    );
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
                    return $crate::test_checks::failed_cmp("!=", left_val, right_val);
                }
            }
        }
    };
    ($left:expr, $right:expr, $($arg:tt)+) => {
        match (&$left, &$right) {
            (left_val, right_val) => {
                if *left_val == *right_val {
                    return $crate::test_checks::failed_cmp_with(
                        "!=",
                        left_val,
                        right_val,
                        format_args!($($arg)+),
                    );
                }
            }
        }
    };
}

//! Checks for tests and fixture helpers that return `Result` (doc 46 cut CT.2).
//!
//! A crate whose root denies `clippy::panic_in_result_fn` (WS01-S12) cannot
//! `assert!` in a `#[test] fn … -> Result`, or in a fixture helper that returns
//! `Result`: the lint has no test exemption. These macros are the asserts'
//! replacements, with the same condition and the same message, and an early
//! `return Err(..)` where the assert panicked:
//!
//! - [`check!(cond)`](crate::check) / `check!(cond, "fmt", args..)` for `assert!`;
//! - [`check_eq!(left, right[, "fmt", args..])`](crate::check_eq) for `assert_eq!`;
//! - [`check_ne!(left, right[, "fmt", args..])`](crate::check_ne) for `assert_ne!`.
//!
//! They neither panic nor unwrap. The failure is built out of line, in
//! [`failed`] and its siblings, so a check costs its caller's stack frame no
//! more than the assert it replaced (`clippy::large_stack_frames` measures test
//! functions too).
//!
//! This is the one implementation. `mush-economy`, `mush-roll` and
//! `mush-market` each kept a copy of it as `src/test_checks.rs` until CT.2;
//! that file is now three forwarding macros, there only because a
//! `macro_rules!` is textually scoped and the crate's tests name the macros
//! bare. `mush-industry`, `mush-inventory`, `mush-proto`, `mush-server`,
//! `mush-sim`, `mush-tactics` and `tacsim` still carry their own copies.
//!
//! A failed check is a [`CheckFailed`], so a failure can be told from an
//! ordinary refusal: a test that expects a helper to refuse asks [`refused`],
//! which hands a failed check inside that helper back as the test failure it is
//! instead of counting it as the refusal.
//!
//! ```
//! use mush_testkit::{check, check_eq};
//!
//! fn two() -> Result<(), Box<dyn std::error::Error>> {
//!     check!(1 + 1 == 2);
//!     check_eq!(1 + 1, 3, "arithmetic");
//!     Ok(())
//! }
//! let report = format!("{:?}", two().unwrap_err());
//! assert!(report.starts_with("assertion `left == right` failed: arithmetic"));
//! ```

use std::error::Error;
use std::fmt;

/// The error a failed `check!`, `check_eq!` or `check_ne!` returns.
pub struct CheckFailed(String);

impl CheckFailed {
    /// The report, for a crate that turns a failed check into its own error
    /// type (`impl From<CheckFailed> for RollError`).
    #[must_use]
    pub fn into_message(self) -> String {
        self.0
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

/// A few tests return the error of the code under test rather than
/// `Box<dyn Error>`; a failed check reaches them as that error's message.
impl From<CheckFailed> for mush_core::ModError {
    fn from(failed: CheckFailed) -> Self {
        Self::msg(failed.0)
    }
}

/// `Ok(true)` when `result` is a refusal, `Ok(false)` when it succeeded, and
/// `Err` (the failure itself) when the call failed one of these checks.
///
/// `check!(refused(call)?)` keeps what `assert!(call.is_err())` meant while
/// `call`'s own fixture checks were asserts: a broken fixture fails the test,
/// it is never the refusal the test is looking for.
///
/// # Errors
///
/// The failed check, when `result` is one.
pub fn refused<T>(result: Result<T, Box<dyn Error>>) -> Result<bool, Box<dyn Error>> {
    match result {
        Ok(_) => Ok(false),
        Err(error) if error.is::<CheckFailed>() => Err(error),
        Err(_) => Ok(true),
    }
}

/// A failed `check!(cond)`. It returns the caller's whole `Err`, so the
/// expansion writes straight into the caller's return slot.
///
/// # Errors
///
/// Always: this is the failure.
pub fn failed<T, E: From<CheckFailed>>(message: &str) -> Result<T, E> {
    Err(E::from(CheckFailed(message.to_owned())))
}

/// A failed `check!(cond, "fmt", args..)`.
///
/// # Errors
///
/// Always: this is the failure.
pub fn failed_with<T, E: From<CheckFailed>>(message: fmt::Arguments<'_>) -> Result<T, E> {
    Err(E::from(CheckFailed(message.to_string())))
}

/// A failed `check_eq!`/`check_ne!` without a message; `op` is `==` or `!=`.
///
/// # Errors
///
/// Always: this is the failure.
pub fn failed_cmp<T, E, L, R>(op: &str, left: &L, right: &R) -> Result<T, E>
where
    E: From<CheckFailed>,
    L: fmt::Debug + ?Sized,
    R: fmt::Debug + ?Sized,
{
    // `{:>6}` right-aligns the two labels as `assert_eq!` does, without a run of
    // spaces inside a literal.
    Err(E::from(CheckFailed(format!(
        "assertion `left {op} right` failed\n{:>6}: {left:?}\n{:>6}: {right:?}",
        "left", "right"
    ))))
}

/// A failed `check_eq!`/`check_ne!` with a message; `op` is `==` or `!=`.
///
/// # Errors
///
/// Always: this is the failure.
pub fn failed_cmp_with<T, E, L, R>(
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
#[macro_export]
macro_rules! check {
    ($cond:expr $(,)?) => {
        if !$cond {
            return $crate::checks::failed(concat!("assertion failed: ", stringify!($cond)));
        }
    };
    ($cond:expr, $($arg:tt)+) => {
        if !$cond {
            return $crate::checks::failed_with(format_args!($($arg)+));
        }
    };
}

/// `assert_eq!` for a function that returns `Result`.
///
/// The operands are matched, not `let`-bound, as `assert_eq!` does: a
/// temporary either side borrows (`f()?.as_ref()`) lives to the comparison.
#[macro_export]
macro_rules! check_eq {
    ($left:expr, $right:expr $(,)?) => {
        match (&$left, &$right) {
            (left_val, right_val) => {
                if !(*left_val == *right_val) {
                    return $crate::checks::failed_cmp("==", left_val, right_val);
                }
            }
        }
    };
    ($left:expr, $right:expr, $($arg:tt)+) => {
        match (&$left, &$right) {
            (left_val, right_val) => {
                if !(*left_val == *right_val) {
                    return $crate::checks::failed_cmp_with(
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
#[macro_export]
macro_rules! check_ne {
    ($left:expr, $right:expr $(,)?) => {
        match (&$left, &$right) {
            (left_val, right_val) => {
                if *left_val == *right_val {
                    return $crate::checks::failed_cmp("!=", left_val, right_val);
                }
            }
        }
    };
    ($left:expr, $right:expr, $($arg:tt)+) => {
        match (&$left, &$right) {
            (left_val, right_val) => {
                if *left_val == *right_val {
                    return $crate::checks::failed_cmp_with(
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

#[cfg(test)]
mod tests {
    use super::{CheckFailed, refused};
    use std::error::Error;

    type TestResult<T = ()> = Result<T, Box<dyn Error>>;

    fn holds(n: u8) -> TestResult {
        check!(n + 1 == 2);
        check!(true, "never printed {}", 1);
        check_eq!(2, 2);
        check_eq!(2, 2, "never printed");
        check_ne!(2, 3);
        check_ne!(2, 3, "never printed");
        Ok(())
    }

    fn plain(n: u8) -> TestResult {
        check!(n + 1 == 3);
        Ok(())
    }

    fn worded(n: u8) -> TestResult {
        check!(n == 0, "n was {n}");
        Ok(())
    }

    fn equal(left: u8, right: u8) -> TestResult {
        check_eq!(left, right);
        Ok(())
    }

    fn equal_worded(left: u8, right: u8) -> TestResult {
        check_eq!(left, right, "the {}", "pair");
        Ok(())
    }

    fn unequal(left: u8, right: u8) -> TestResult {
        check_ne!(left, right);
        Ok(())
    }

    fn unequal_worded(left: u8, right: u8) -> TestResult {
        check_ne!(left, right, "the {}", "pair");
        Ok(())
    }

    fn report(result: TestResult) -> String {
        result.map_or_else(|error| format!("{error:?}"), |()| "passed".to_owned())
    }

    #[test]
    fn a_check_that_holds_returns_nothing() {
        assert_eq!(report(holds(1)), "passed");
    }

    #[test]
    fn a_failed_check_reports_what_the_assert_reported() {
        assert_eq!(report(plain(1)), "assertion failed: n + 1 == 3");
        assert_eq!(report(worded(4)), "n was 4");
        assert_eq!(
            report(equal(1, 2)),
            "assertion `left == right` failed\n  left: 1\n right: 2"
        );
        assert_eq!(
            report(equal_worded(1, 2)),
            "assertion `left == right` failed: the pair\n  left: 1\n right: 2"
        );
        assert_eq!(
            report(unequal(2, 2)),
            "assertion `left != right` failed\n  left: 2\n right: 2"
        );
        assert_eq!(
            report(unequal_worded(2, 2)),
            "assertion `left != right` failed: the pair\n  left: 2\n right: 2"
        );
    }

    #[test]
    fn refused_tells_a_refusal_from_a_failed_check() {
        let refusal: TestResult = Err("the owner refused".into());
        assert!(matches!(refused(refusal), Ok(true)));
        assert!(matches!(refused(holds(1)), Ok(false)));
        let failure = refused(plain(1)).expect_err("a failed check is the failure");
        assert!(failure.is::<CheckFailed>());
    }

    #[test]
    fn a_failed_check_reaches_a_module_error_as_its_message() {
        fn module() -> Result<(), mush_core::ModError> {
            check_eq!(1, 2, "module");
            Ok(())
        }
        let error = module().expect_err("the check fails");
        assert!(error.to_string().contains("failed: module"));
    }
}

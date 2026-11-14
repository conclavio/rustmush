#![cfg(test)]

//! Test-only checks for functions that return `Result`.
//!
//! This crate's root denies `clippy::panic_in_result_fn` (WS01-S12 / 17d
//! M6.1), and that lint has no test exemption: an `assert!` in a
//! `#[test] fn … -> Result`, or in a fixture helper that returns `Result`, is
//! an error. `check!`, `check_eq!` and `check_ne!` are the asserts'
//! replacements.
//!
//! They are `mush_testkit::checks` (doc 46 cut CT.2); this crate kept its own
//! copy until then. What is left here forwards to that one implementation:
//! a `macro_rules!` is textually scoped, this module is declared first in
//! `lib.rs`, and the crate's tests name the macros bare. The one thing that
//! stays is the conversion into this crate's own error.

use mush_testkit::checks::CheckFailed;

/// A few tests here return [`crate::RollError`]; a failed check reaches them
/// as that error's message.
impl From<CheckFailed> for crate::RollError {
    fn from(failed: CheckFailed) -> Self {
        crate::invalid("check", failed.into_message())
    }
}

/// `assert!` for a function that returns `Result`.
macro_rules! check {
    ($($check:tt)*) => {
        ::mush_testkit::check!($($check)*)
    };
}

/// `assert_eq!` for a function that returns `Result`.
macro_rules! check_eq {
    ($($check:tt)*) => {
        ::mush_testkit::check_eq!($($check)*)
    };
}

/// `assert_ne!` for a function that returns `Result`.
macro_rules! check_ne {
    ($($check:tt)*) => {
        ::mush_testkit::check_ne!($($check)*)
    };
}

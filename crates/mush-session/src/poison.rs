//! The session layer's single lock-poison policy (WS01-S10B).
//!
//! This crate runs inside the gateway half, which D1 makes the half that must
//! survive a server restart. Its locks guard per-connection capabilities, the
//! linkdead observer slot and the per-connection flood buckets — small,
//! non-reentrant state that the next line of input re-derives. Before this
//! module, `.expect("... poisoned")` on them meant one panicking task made
//! every *other* connection's next `check_input` panic too, which is exactly
//! the fan-out the fork exists to prevent.
//!
//! So a poisoned lock here is recovered, not escalated — the same judgement
//! `mush-server`'s `claims_guard` made for the IPC claim table, and the same
//! one `mush_gateway::poison` makes one crate up. Each half is transcribed
//! rather than shared: the model is `pub(crate)` and cannot cross a crate
//! boundary.
//!
//! The first recovery in a process logs once, naming the lock; later ones are
//! silent. The flood buckets are taken once per input line, so a line per
//! acquisition would answer a panic with an unbounded log flood, and the panic
//! itself is already reported by the hook `mush_config::logging::init`
//! installs.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Take `lock`, recovering it if a panicking task poisoned it.
pub(crate) fn locked<'a, T>(lock: &'a Mutex<T>, what: &'static str) -> MutexGuard<'a, T> {
    lock.lock().unwrap_or_else(|poisoned| {
        report_poison(what);
        poisoned.into_inner()
    })
}

/// [`locked`] for a shared read of an [`RwLock`].
pub(crate) fn read_locked<'a, T>(
    lock: &'a RwLock<T>,
    what: &'static str,
) -> RwLockReadGuard<'a, T> {
    lock.read().unwrap_or_else(|poisoned| {
        report_poison(what);
        poisoned.into_inner()
    })
}

/// [`locked`] for an exclusive write to an [`RwLock`].
pub(crate) fn write_locked<'a, T>(
    lock: &'a RwLock<T>,
    what: &'static str,
) -> RwLockWriteGuard<'a, T> {
    lock.write().unwrap_or_else(|poisoned| {
        report_poison(what);
        poisoned.into_inner()
    })
}

/// Set by the first recovered poisoning; never cleared.
static POISON_REPORTED: AtomicBool = AtomicBool::new(false);

/// Whether this process still owes an operator the recovery line.
///
/// Split out from [`report_poison`] so the once-per-process decision is
/// testable against a flag the test owns, rather than against a global another
/// test may already have tripped.
fn first_poison_report(reported: &AtomicBool) -> bool {
    !reported.swap(true, Ordering::Relaxed)
}

fn report_poison(what: &'static str) {
    if first_poison_report(&POISON_REPORTED) {
        tracing::error!(
            lock = what,
            "recovered a poisoned session lock: a task panicked while holding it; the session \
             layer is continuing with the state that panic left, and further recoveries are not \
             logged"
        );
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Poison a mutex the way a real panic does: unwind out of a live guard.
    ///
    /// `AssertUnwindSafe` is the point rather than a wart — the whole claim
    /// under test is that the session layer keeps serving over state a panic
    /// left half-written.
    pub(crate) fn poison_mutex<T>(lock: &Mutex<T>) {
        let unwound = unwinding(|| {
            if let Ok(_guard) = lock.lock() {
                panic!("a task panicked while holding the lock (expected: this test poisons it)");
            }
        });
        assert!(unwound, "the closure must really have panicked");
        assert!(
            lock.lock().is_err(),
            "the lock must really be poisoned for a recovery proof to mean anything"
        );
    }

    /// [`poison_mutex`] for an [`RwLock`], poisoned through its write side.
    pub(crate) fn poison_rwlock<T>(lock: &RwLock<T>) {
        let unwound = unwinding(|| {
            if let Ok(_guard) = lock.write() {
                panic!("a task panicked while holding the lock (expected: this test poisons it)");
            }
        });
        assert!(unwound, "the closure must really have panicked");
        assert!(
            lock.write().is_err(),
            "the lock must really be poisoned for a recovery proof to mean anything"
        );
    }

    /// Run `f`, catching its unwind; `true` if it panicked.
    ///
    /// The panic HOOK is deliberately left alone (W1-B8-FIX). Silencing it
    /// around the `catch_unwind` reads as tidier, but the hook is
    /// process-global while `cargo test` runs a binary's tests on parallel
    /// threads: two poisonings that overlap interleave their
    /// take/set/set-back (A takes the real hook and installs the silencer, B
    /// takes the SILENCER and installs its own, A restores the real one, B
    /// restores the silencer) and the binary finishes the run with a hook that
    /// prints nothing — so every later panic, in tests that have nothing to do
    /// with poisoning, fails with no message. The cost of not swapping is one
    /// deliberate panic line per call, which libtest captures and only prints
    /// for a test that actually failed; the benefit is that this helper leaves
    /// no process-global state behind at all.
    fn unwinding(f: impl FnOnce()) -> bool {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err()
    }

    /// The whole point: the state the panicking task left is still readable and
    /// still writable, so the connections that share the lock keep running.
    #[test]
    fn a_poisoned_mutex_is_recovered_with_its_state_intact() {
        let table: Mutex<Vec<u32>> = Mutex::new(vec![7]);
        poison_mutex(&table);

        assert_eq!(locked(&table, "test table").as_slice(), &[7]);
        locked(&table, "test table").push(9);
        assert_eq!(locked(&table, "test table").as_slice(), &[7, 9]);
    }

    /// Both sides of the `RwLock` recover: a reader that only observes must not
    /// be the thing that takes the process down.
    #[test]
    fn a_poisoned_rwlock_is_recovered_on_both_sides() {
        let caps: RwLock<u32> = RwLock::new(7);
        poison_rwlock(&caps);

        assert_eq!(*read_locked(&caps, "test caps"), 7);
        *write_locked(&caps, "test caps") = 9;
        assert_eq!(*read_locked(&caps, "test caps"), 9);
    }

    /// The recovery line is one per process: the flood buckets are taken once
    /// per input line, so a line per acquisition would be a log flood.
    #[test]
    fn the_recovery_line_is_logged_once_per_process() {
        let reported = AtomicBool::new(false);
        assert!(first_poison_report(&reported));
        assert!(!first_poison_report(&reported));
        assert!(!first_poison_report(&reported));
    }
}

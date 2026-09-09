//! The gateway half's single mutex-poison policy (WS01-S10B).
//!
//! D1 makes this the half that must survive a server restart, so a panic on a
//! gateway task is already the worst thing that can happen here: it unwinds a
//! task that owns the SSH channels the fork exists to preserve. Before this
//! module, `.expect("... poisoned")` guaranteed that one such panic became
//! *every other* task's panic on its next acquisition — the lifecycle custody
//! table, the pending-request table, the manifest cache, the Discord follow
//! table and the two rate limiters are each touched by several tasks, so a
//! single poisoning tore down connections that had nothing to do with it.
//!
//! Every lock this crate holds guards short, non-reentrant bookkeeping that is
//! reconciled by its own saturating arithmetic and re-derived from the next
//! frame, so a poisoned one is **recovered**, not escalated. That is the same
//! judgement `mush-server`'s `claims_guard` made for the IPC claim table; it is
//! transcribed rather than shared because that helper is `pub(crate)` and
//! cannot cross a crate boundary.
//!
//! Recovery is not silent, and it is not loud either: the first poisoning this
//! process recovers logs once, naming the lock. Once — not once per site and
//! not once per acquisition, because the hottest converted sites are taken per
//! frame, and answering a panic with an unbounded log flood would trade one
//! failure mode for another. The panic itself is already reported by the hook
//! `mush_config::logging::init` installs; this line only records that the
//! gateway chose to continue with the state that panic left.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

/// Take `lock`, recovering it if a panicking task poisoned it.
///
/// `what` names the lock for the one-per-process recovery line.
pub(crate) fn locked<'a, T>(lock: &'a Mutex<T>, what: &'static str) -> MutexGuard<'a, T> {
    lock.lock().unwrap_or_else(|poisoned| {
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
            "recovered a poisoned gateway lock: a task panicked while holding it; the gateway \
             is continuing with the state that panic left, and further recoveries are not logged"
        );
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Poison `lock` the way a real panic does: unwind out of a live guard.
    ///
    /// `AssertUnwindSafe` is the point rather than a wart — the whole claim
    /// under test is that the gateway keeps running over state a panic left
    /// half-written.
    pub(crate) fn poison<T>(lock: &Mutex<T>) {
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Ok(_guard) = lock.lock() {
                panic!("a task panicked while holding the lock");
            }
        }));
        std::panic::set_hook(hook);
        assert!(unwound.is_err(), "the closure must really have panicked");
        assert!(
            lock.lock().is_err(),
            "the lock must really be poisoned for a recovery proof to mean anything"
        );
    }

    /// The whole point: the state the panicking task left is still readable and
    /// still writable, so the tasks that share the lock keep running.
    #[test]
    fn a_poisoned_lock_is_recovered_with_its_state_intact() {
        let table: Mutex<Vec<u32>> = Mutex::new(vec![7]);
        poison(&table);

        assert_eq!(locked(&table, "test table").as_slice(), &[7]);
        locked(&table, "test table").push(9);
        assert_eq!(locked(&table, "test table").as_slice(), &[7, 9]);
    }

    /// The recovery line is one per process. The hot sites are taken per frame,
    /// so a line per acquisition would answer a panic with a log flood.
    #[test]
    fn the_recovery_line_is_logged_once_per_process() {
        let reported = AtomicBool::new(false);
        assert!(first_poison_report(&reported));
        assert!(!first_poison_report(&reported));
        assert!(!first_poison_report(&reported));
    }
}

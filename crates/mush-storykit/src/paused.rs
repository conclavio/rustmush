//! The paused clock's barrier (doc 51 §2.6 and §3.3, unit FL.6).
//!
//! An in-process rig on the memory store, with no network client, runs on
//! tokio's paused clock (`#[tokio::test(start_paused = true)]`) and hands the
//! world `mush_config::TimeSource::Tokio`, the server's test clock (this crate
//! does not link it), so the world's unix time and its sleeps move together.
//! Such a rig needs no sleep "to let the world get somewhere": the runtime
//! itself knows when nothing is left to run.
//!
//! On a paused clock tokio moves time forward only when every task on the
//! runtime is parked, waiting on a timer or a channel. So a sleep of the
//! shortest span tokio's timer keeps returns only after every task that could
//! run has run to its next wait. That is [`idle`]: the barrier for "the world
//! has taken everything ready for it", and for an absence (nothing ran)
//! asserted after it.

use std::time::Duration;

/// The span [`idle`] sleeps: tokio's timer resolution, one millisecond. On a
/// paused clock it is virtual time, and the least step the clock can take.
pub const IDLE_STEP: Duration = Duration::from_millis(1);

/// **Every other task on this runtime has parked.** On a paused clock
/// (`start_paused = true`, which is the current-thread runtime) this returns
/// once every task that was ready has run up to its next wait, and the clock
/// has moved [`IDLE_STEP`]. A task that a timer due inside that step wakes
/// may or may not have run by then: it wakes at the same instant as `idle`.
///
/// Use it in place of a sleep, or of a count of `yield_now`, that was there
/// to let a spawned task or the world loop take what was sent to it. An
/// absence is asserted after it: if the work had been going to run, it would
/// have run before the clock moved.
///
/// On a clock that is not paused it is a one-millisecond sleep and proves
/// nothing, so call it only from a `start_paused` test. It does not wait on
/// I/O the runtime does not own (a socket served by another runtime, a child
/// process, Redis): on a paused clock that I/O does not hold the clock back,
/// which is why doc 43c §3.3 keeps such rigs off the paused clock.
pub async fn idle() {
    tokio::time::sleep(IDLE_STEP).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test(start_paused = true)]
    async fn a_task_that_yields_many_times_has_finished_by_idle() {
        let steps = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&steps);
        let task = tokio::spawn(async move {
            for _ in 0..10_000 {
                counter.fetch_add(1, Ordering::SeqCst);
                tokio::task::yield_now().await;
            }
        });
        idle().await;
        assert_eq!(steps.load(Ordering::SeqCst), 10_000);
        assert!(task.is_finished());
    }

    #[tokio::test(start_paused = true)]
    async fn a_task_waiting_on_a_channel_is_parked_and_idle_returns() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(async move { rx.await.is_ok() });
        idle().await;
        assert!(!task.is_finished(), "the task waits on its channel");
        tx.send(()).unwrap();
        idle().await;
        assert!(task.is_finished());
    }

    #[tokio::test(start_paused = true)]
    async fn the_clock_moves_one_step_and_a_later_timer_does_not_fire() {
        let fired = Arc::new(AtomicUsize::new(0));
        let later = Arc::clone(&fired);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            later.fetch_add(1, Ordering::SeqCst);
        });
        let start = tokio::time::Instant::now();
        idle().await;
        assert_eq!(fired.load(Ordering::SeqCst), 0);
        assert_eq!(start.elapsed(), IDLE_STEP);
    }
}

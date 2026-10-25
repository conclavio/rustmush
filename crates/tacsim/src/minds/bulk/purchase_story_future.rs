//! Test-only heap boundaries for the genuine purchase story's nested phases.
//!
//! Each phase is still polled inline by its original caller, with unchanged
//! borrows, result propagation and task cleanup. No task or runtime is added.
use super::TestResult;
use std::{future::Future, mem::size_of, pin::Pin};

// Bound construction temporaries as well as the retained future. This is a
// fixture bound, not a source/worker execution limit or a larger thread stack.
// Match clippy.toml's future-size-threshold even for explicitly boxed phases.
const MAX_PHASE_BYTES: usize = 16 * 1024;

pub(super) fn story_phase<F: Future>(
    phase: &'static str,
    make: impl FnOnce() -> F,
) -> TestResult<Pin<Box<F>>> {
    // Check every instantiated phase while building the real test binary,
    // including phases an earlier failing assertion would never reach.
    const {
        assert!(
            size_of::<F>() <= MAX_PHASE_BYTES,
            "story future exceeds the configured 16 KiB bound"
        );
    }
    checked_story_phase(phase, make)
}

fn checked_story_phase<F: Future>(
    phase: &'static str,
    make: impl FnOnce() -> F,
) -> TestResult<Pin<Box<F>>> {
    let bytes = size_of::<F>();
    eprintln!(
        "M7_STORY_FUTURE phase={phase} bytes={bytes} boxed_bytes={} max_bytes={MAX_PHASE_BYTES}",
        size_of::<Pin<Box<F>>>()
    );
    if bytes > MAX_PHASE_BYTES {
        return Err("purchase story phase exceeds bounded future frame".into());
    }
    Ok(allocate_phase(make))
}

// Keep construction in a separate frame, entered only after the type-size
// check. In particular, an oversized F must not reserve its construction
// temporary in story_phase's own frame before the guard can run.
#[inline(never)]
fn allocate_phase<F: Future>(make: impl FnOnce() -> F) -> Pin<Box<F>> {
    Box::pin(make())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::Cell,
        task::{Context, Poll, Waker},
    };

    struct Oversized {
        _bytes: [u8; MAX_PHASE_BYTES + 1],
    }
    impl Future for Oversized {
        type Output = ();
        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Ready(())
        }
    }
    struct OnDrop<'a>(&'a Cell<usize>);
    impl Drop for OnDrop<'_> {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn oversized_story_future_is_refused_before_factory_entry() {
        let constructed = Cell::new(false);
        let result = checked_story_phase("test.oversized", || -> Oversized {
            constructed.set(true);
            panic!("oversized factory must not be entered")
        });
        assert!(result.is_err());
        assert!(!constructed.get());
    }

    #[test]
    fn story_heap_boundary_preserves_borrow_poll_result_and_cancellation() -> TestResult {
        let polls = Cell::new(0);
        let drops = Cell::new(0);
        let witness = OnDrop(&drops);
        let mut future = story_phase("test.result", || async {
            let _witness = witness;
            polls.set(polls.get() + 1);
            Err::<(), _>("original phase refusal")
        })?;
        check_eq!(polls.get(), 0, "construction does not poll the phase");
        let mut context = Context::from_waker(Waker::noop());
        check_eq!(
            future.as_mut().poll(&mut context),
            Poll::Ready(Err("original phase refusal"))
        );
        check_eq!(polls.get(), 1);
        drop(future);
        check_eq!(drops.get(), 1);

        let witness = OnDrop(&drops);
        let future = story_phase("test.cancel", || async {
            let _witness = witness;
            polls.set(polls.get() + 1);
            std::future::pending::<()>().await;
        })?;
        drop(future);
        check_eq!(polls.get(), 1, "dropping an unpolled phase does not run it");
        check_eq!(
            drops.get(),
            2,
            "captured cleanup is still dropped exactly once"
        );
        Ok(())
    }
}

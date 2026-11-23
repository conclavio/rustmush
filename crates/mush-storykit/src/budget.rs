//! One limit for a whole story (doc 51 §3.7).
//!
//! A test runner terminates a test that runs too long, and a terminated test
//! prints nothing: no assertion, no transcript. A story whose single waits are
//! each longer than that terminate can therefore stall for weeks without
//! saying where. One such story did (`docs/flaky-tests.md`, the captain and
//! its escort): six CI runs ended in a bare `TIMEOUT [1200 s]`.
//!
//! A [`StoryBudget`] is the story's own, shorter limit. Every wait in the
//! story draws its bound from what is left of it, so the story's last wait
//! fails, with its own message, before the runner's terminate.

use std::future::Future;
use std::time::Duration;

use tokio::time::Instant;

use crate::wait::wait_until;

/// **When nextest terminates a test**: twenty minutes, on the `ci` and
/// `ci-bot` profiles of `.config/nextest.toml` (`slow-timeout`: a 60-s period,
/// `terminate-after = 20`). The `wait_budget` gate in `mush-contract-tests`
/// holds this constant to that file.
pub const NEXTEST_TERMINATE: Duration = Duration::from_secs(20 * 60);

/// The room a story leaves under [`NEXTEST_TERMINATE`] for its failure to
/// print and its rig to stop.
pub const TERMINATE_MARGIN: Duration = Duration::from_secs(2 * 60);

/// How long past the budget's end [`StoryBudget::hold`] lets the story run, so
/// that a wait which drew its bound from the budget fails first, with its own
/// message, and `hold` is only the backstop.
const HOLD_GRACE: Duration = Duration::from_secs(30);

/// **A story's whole limit.** Start one at the top of the story, then give
/// every wait [`StoryBudget::limit`] of the span it would otherwise have
/// taken, or wait through [`StoryBudget::wait_until`].
///
/// ```
/// # use mush_storykit::{STEP_TIMEOUT, StoryBudget};
/// # tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(async {
/// let budget = StoryBudget::new(10);
/// budget
///     .hold(async {
///         budget.wait_until(STEP_TIMEOUT, "the listener to bind", async || true).await;
///     })
///     .await;
/// # });
/// ```
///
/// It reads tokio's clock, so a paused test drives it.
#[derive(Debug, Clone, Copy)]
pub struct StoryBudget {
    total: Duration,
    ends: Instant,
}

impl StoryBudget {
    /// A budget of `minutes`, counted from now.
    ///
    /// # Panics
    ///
    /// When `minutes` is zero, or longer than [`NEXTEST_TERMINATE`] less
    /// [`TERMINATE_MARGIN`] (18 minutes): past that the runner would end the
    /// story before the budget could.
    pub fn new(minutes: u64) -> Self {
        let total = Duration::from_secs(minutes.saturating_mul(60));
        let most = NEXTEST_TERMINATE.saturating_sub(TERMINATE_MARGIN);
        assert!(
            minutes > 0 && total <= most,
            "a story budget of {minutes} min does not fit: nextest terminates a test at \
             {NEXTEST_TERMINATE:?}, and a budget may be at most {most:?} so that its own failure \
             prints first"
        );
        Self {
            total,
            ends: Instant::now() + total,
        }
    }

    /// The whole budget, as it was started.
    pub fn total(&self) -> Duration {
        self.total
    }

    /// What is left of it; zero once it is spent.
    pub fn left(&self) -> Duration {
        self.ends.saturating_duration_since(Instant::now())
    }

    /// The bound a wait may take: `want`, or what is left of the budget when
    /// that is less.
    pub fn limit(&self, want: Duration) -> Duration {
        want.min(self.left())
    }

    /// [`wait_until`], bound by [`StoryBudget::limit`] of `want`.
    ///
    /// # Panics
    ///
    /// When the bound passes before `check` returns `true`. The message says
    /// whether the wait's own span ran out or the story's budget did.
    pub async fn wait_until(&self, want: Duration, label: &str, check: impl AsyncFnMut() -> bool) {
        let bound = self.limit(want);
        if bound < want {
            let total = self.total;
            let label = format!(
                "{label} (the wait asked for {want:?}; the story's {total:?} budget had \
                 {bound:?} left)"
            );
            wait_until(bound, &label, check).await;
        } else {
            wait_until(bound, label, check).await;
        }
    }

    /// Run the story's body inside the budget, as the outermost limit: a wait
    /// that did not draw from the budget is still ended by it. The limit falls
    /// thirty seconds after the budget's end, inside [`TERMINATE_MARGIN`], so a
    /// wait that did draw from the budget fails first and names what it
    /// waited for.
    ///
    /// # Panics
    ///
    /// When the story is still running thirty seconds after its budget ended.
    pub async fn hold<T>(&self, story: impl Future<Output = T>) -> T {
        let total = self.total;
        tokio::time::timeout_at(self.ends + HOLD_GRACE, story)
            .await
            .unwrap_or_else(|_| panic!("the story did not finish within its {total:?} budget"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wait::STEP_TIMEOUT;

    #[test]
    fn the_longest_budget_is_the_terminate_less_its_margin() {
        assert_eq!(StoryBudget::new(18).total(), Duration::from_secs(18 * 60));
    }

    #[test]
    #[should_panic(expected = "a story budget of 19 min does not fit")]
    fn a_budget_the_runner_would_end_first_is_refused() {
        let _ = StoryBudget::new(19);
    }

    #[test]
    #[should_panic(expected = "a story budget of 0 min does not fit")]
    fn an_empty_budget_is_refused() {
        let _ = StoryBudget::new(0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_wait_draws_its_bound_from_what_is_left() {
        let budget = StoryBudget::new(2);
        assert_eq!(budget.limit(STEP_TIMEOUT), STEP_TIMEOUT);
        tokio::time::advance(Duration::from_secs(90)).await;
        assert_eq!(budget.left(), Duration::from_secs(30));
        assert_eq!(budget.limit(STEP_TIMEOUT), Duration::from_secs(30));
        tokio::time::advance(Duration::from_secs(90)).await;
        assert_eq!(budget.left(), Duration::ZERO);
        assert_eq!(budget.limit(STEP_TIMEOUT), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    #[should_panic(expected = "the reply did not happen within 60s")]
    async fn a_wait_inside_a_full_budget_fails_as_a_plain_wait_does() {
        let budget = StoryBudget::new(5);
        budget
            .wait_until(STEP_TIMEOUT, "the reply", async || false)
            .await;
    }

    #[tokio::test(start_paused = true)]
    #[should_panic(
        expected = "the reply (the wait asked for 60s; the story's 120s budget had 30s left) did not happen within 30s"
    )]
    async fn a_wait_the_budget_cut_short_says_so() {
        let budget = StoryBudget::new(2);
        tokio::time::advance(Duration::from_secs(90)).await;
        budget
            .wait_until(STEP_TIMEOUT, "the reply", async || false)
            .await;
    }

    #[test]
    fn the_backstop_falls_inside_the_margin() {
        assert!(HOLD_GRACE < TERMINATE_MARGIN);
    }

    #[tokio::test(start_paused = true)]
    async fn a_story_inside_its_budget_returns_its_value() {
        let budget = StoryBudget::new(1);
        let got = budget
            .hold(async {
                tokio::time::sleep(Duration::from_secs(60) + HOLD_GRACE / 2).await;
                7
            })
            .await;
        assert_eq!(got, 7);
    }

    #[tokio::test(start_paused = true)]
    #[should_panic(expected = "the story did not finish within its 60s budget")]
    async fn a_story_past_its_budget_fails_with_the_budget_named() {
        let budget = StoryBudget::new(1);
        budget
            .hold(tokio::time::sleep(Duration::from_secs(61) + HOLD_GRACE))
            .await;
    }
}

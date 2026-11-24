//! **A story that samples chance states its odds** (doc 51 §3.2, rule D;
//! ruling Q-51-3).
//!
//! Most stories should not sample chance at all: a pinned seed
//! ([`crate::story_seed`]) and a chosen roll ([`crate::Figures::seed_where`])
//! make an outcome certain. Some cannot: a story in which a machine or the bot
//! moves a unit has dice the seed fixes and a geometry it does not. Such a
//! story says how often bad luck alone would fail it, in code, and the kit
//! refuses odds worse than the tier's bar. A 60-s match that left four rounds
//! at one chance in two failed one run in eight for days before anyone did
//! the arithmetic (`docs/flaky-tests.md`, row 2); [`Odds`] does it at the top
//! of the test.

/// **The pull-request tier's bar** (ruling Q-51-3): bad luck alone may fail a
/// story at most once in 1,000,000 runs. A story that cannot meet it runs on
/// the dispatch tier and reports an interval.
pub const PULL_REQUEST_BAR: f64 = 1e-6;

/// **The odds of a chance-sampling assertion**: `trials` independent tries,
/// each succeeding with probability `p_each`, of which the story needs `need`.
///
/// ```
/// # use mush_storykit::{Odds, PULL_REQUEST_BAR};
/// // Twenty rounds at the rules' ceiling of 95 in 100, one hit needed.
/// Odds { trials: 20, need: 1, p_each: 0.95 }.assert_false_failure_below(PULL_REQUEST_BAR);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Odds {
    /// How many tries the story makes.
    pub trials: u32,
    /// How many of them must succeed for the story to pass.
    pub need: u32,
    /// The chance one try succeeds, `0.0..=1.0`. Take it from the rules
    /// ([`crate::Figures::chance_bounds`], or the cockpit's own percentage),
    /// and take the least it can be.
    pub p_each: f64,
}

impl Odds {
    /// **The chance bad luck alone fails the story**: fewer than `need`
    /// successes in `trials` tries, the binomial's lower tail.
    ///
    /// # Panics
    ///
    /// When `p_each` is not a probability.
    #[must_use]
    pub fn false_failure(&self) -> f64 {
        assert!(
            (0.0..=1.0).contains(&self.p_each),
            "p_each is a probability; {} is not",
            self.p_each
        );
        if self.need == 0 {
            return 0.0;
        }
        if self.need > self.trials {
            return 1.0;
        }
        if self.p_each <= 0.0 {
            return 1.0;
        }
        if self.p_each >= 1.0 {
            return 0.0;
        }
        // Sum P(k successes) for k < need, each term in logs so that a long
        // run of tries cannot underflow the first term and zero the rest.
        let (ln_p, ln_q) = (self.p_each.ln(), (1.0 - self.p_each).ln());
        let trials = f64::from(self.trials);
        let mut ln_choose = 0.0_f64;
        let mut tail = 0.0_f64;
        for k in 0..self.need {
            let k = f64::from(k);
            tail += (ln_choose + k * ln_p + (trials - k) * ln_q).exp();
            ln_choose += ((trials - k) / (k + 1.0)).ln();
        }
        tail.min(1.0)
    }

    /// **Fail now, with the arithmetic, if bad luck alone would fail the story
    /// more often than `bar`.** Call it before the story spends its tries.
    ///
    /// # Panics
    ///
    /// When [`Odds::false_failure`] is over `bar`. The message gives the
    /// odds and what would meet the bar.
    pub fn assert_false_failure_below(&self, bar: f64) {
        let chance = self.false_failure();
        if chance <= bar {
            return;
        }
        let would = (self.need..=self.need.saturating_add(10_000))
            .find(|&trials| Odds { trials, ..*self }.false_failure() <= bar);
        let fix = match would {
            Some(trials) => format!("{trials} tries would meet it"),
            None => "no number of tries within reach meets it".to_owned(),
        };
        panic!(
            "this story is left to chance: {} of {} tries at {:.3} each fail by bad luck alone \
             {:.3e} of the time (about 1 run in {:.0}), over the bar of {bar:e}. {fix}; or make \
             the outcome certain with a pinned seed (doc 51 rule D).",
            self.need,
            self.trials,
            self.p_each,
            chance,
            1.0 / chance,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(got: f64, want: f64) {
        assert!(
            (got - want).abs() <= want.abs() * 1e-9 + 1e-15,
            "{got} is not {want}"
        );
    }

    #[test]
    fn the_tail_is_the_binomials() {
        // One hit needed: every try missing.
        close(
            Odds {
                trials: 4,
                need: 1,
                p_each: 0.5,
            }
            .false_failure(),
            0.0625,
        );
        // Two needed of three at one in two: none (1/8) or one (3/8).
        close(
            Odds {
                trials: 3,
                need: 2,
                p_each: 0.5,
            }
            .false_failure(),
            0.5,
        );
        close(
            Odds {
                trials: 3,
                need: 1,
                p_each: 0.95,
            }
            .false_failure(),
            0.05_f64.powi(3),
        );
    }

    #[test]
    fn the_edges_are_certain() {
        let odds = |trials, need, p_each| {
            Odds {
                trials,
                need,
                p_each,
            }
            .false_failure()
        };
        close(odds(5, 0, 0.1), 0.0);
        close(odds(2, 3, 0.9), 1.0);
        close(odds(5, 1, 0.0), 1.0);
        close(odds(5, 5, 1.0), 0.0);
    }

    /// A long run does not underflow: half of 5,000 fair tries or fewer is
    /// about one run in two, not zero.
    #[test]
    fn a_long_run_keeps_its_tail() {
        let tail = Odds {
            trials: 5_000,
            need: 2_500,
            p_each: 0.5,
        }
        .false_failure();
        assert!((0.48..0.5).contains(&tail), "{tail}");
    }

    /// Row 2 of `docs/flaky-tests.md`: four rounds at about four in ten, one
    /// hit needed. The story failed one run in eight; this is the message it
    /// would have failed with on the day the cycle moved.
    #[test]
    #[should_panic(expected = "this story is left to chance: 1 of 4 tries at 0.400 each")]
    fn four_rounds_at_four_in_ten_are_refused() {
        Odds {
            trials: 4,
            need: 1,
            p_each: 0.4,
        }
        .assert_false_failure_below(PULL_REQUEST_BAR);
    }

    #[test]
    fn the_refusal_says_what_would_meet_the_bar() {
        let odds = Odds {
            trials: 4,
            need: 1,
            p_each: 0.4,
        };
        let message =
            std::panic::catch_unwind(|| odds.assert_false_failure_below(PULL_REQUEST_BAR))
                .unwrap_err();
        let message = message.downcast_ref::<String>().unwrap();
        // 0.6^28 is over the bar and 0.6^27 is not under it: 28 tries.
        assert!(message.contains("28 tries would meet it"), "{message}");
        assert!(message.contains("about 1 run in 8"), "{message}");
        Odds { trials: 28, ..odds }.assert_false_failure_below(PULL_REQUEST_BAR);
    }
}

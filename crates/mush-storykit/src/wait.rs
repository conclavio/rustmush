//! The one wait loop and the one step limit (doc 51 §3.7).

use std::time::Duration;

/// **The load factor** (doc 51 §3.1): what a limit is multiplied by so that a
/// slow or busy runner is answered in one place.
///
/// A limit is a failure bound, never a pace: a story that waits for an event
/// returns when the event comes, so a larger factor costs time only when
/// something is already wrong.
pub const LOAD_FACTOR: u32 = 2;

/// One step's limit on an unloaded box: a login, a line and its reply, a
/// process start. CI boxes stall on argon2 and on process spawn.
pub const STEP_UNLOADED: Duration = Duration::from_secs(30);

/// **The one limit a step gets**: [`STEP_UNLOADED`] under [`LOAD_FACTOR`],
/// which is 60 s. A bound on an event is this constant, or a small multiple of
/// it, and not a number chosen at the call.
pub const STEP_TIMEOUT: Duration = loaded(STEP_UNLOADED);

/// How often [`wait_until`] asks again.
pub const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// `span` under [`LOAD_FACTOR`]: the limit to give a wait whose unloaded
/// length is `span`.
pub const fn loaded(span: Duration) -> Duration {
    span.saturating_mul(LOAD_FACTOR)
}

/// Wait for an observable condition, with one deadline covering the checks as
/// well as the polling interval. A slow reply inside `check` cannot multiply
/// the bound.
///
/// # Panics
///
/// When `bound` passes before `check` returns `true`. The message names
/// `label` and the bound.
pub async fn wait_until(bound: Duration, label: &str, mut check: impl AsyncFnMut() -> bool) {
    tokio::time::timeout(bound, async {
        loop {
            if check().await {
                return;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{label} did not happen within {bound:?}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_step_limit_is_the_unloaded_step_under_the_load_factor() {
        assert_eq!(STEP_TIMEOUT, Duration::from_secs(60));
        assert_eq!(STEP_TIMEOUT, STEP_UNLOADED * LOAD_FACTOR);
        assert_eq!(loaded(Duration::from_secs(21)), Duration::from_secs(42));
        assert_eq!(loaded(Duration::MAX), Duration::MAX);
    }

    #[tokio::test(start_paused = true)]
    async fn a_wait_returns_as_soon_as_its_condition_holds() {
        let started = tokio::time::Instant::now();
        let mut asked = 0u32;
        wait_until(STEP_TIMEOUT, "the third ask", async || {
            asked += 1;
            asked == 3
        })
        .await;
        assert_eq!(asked, 3);
        assert_eq!(started.elapsed(), POLL_INTERVAL * 2);
    }

    #[tokio::test(start_paused = true)]
    #[should_panic(expected = "the door to open did not happen within 60s")]
    async fn a_wait_that_never_holds_fails_with_its_label_and_bound() {
        wait_until(STEP_TIMEOUT, "the door to open", async || false).await;
    }

    /// The deadline covers the check itself: a check that never answers
    /// cannot hold the wait open.
    #[tokio::test(start_paused = true)]
    #[should_panic(expected = "a reply did not happen within 5s")]
    async fn a_check_that_never_answers_is_inside_the_bound() {
        wait_until(Duration::from_secs(5), "a reply", async || {
            std::future::pending::<bool>().await
        })
        .await;
    }
}

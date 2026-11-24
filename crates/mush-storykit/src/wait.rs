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

/// **Ask until the answer is good, and hand it back.** [`wait_until`] for a
/// wait that reads something: `ask` answers `Ok` with what the story wanted,
/// or `Err` with what it read instead, and is asked again every
/// [`POLL_INTERVAL`] under one deadline.
///
/// A wait that fails should say what it was looking at. The failure prints
/// the last `Err`, so a story that waited for a row of a readout fails with
/// the readout and not with a bare bound.
///
/// # Panics
///
/// When `bound` passes before `ask` answers `Ok`. The message names `label`,
/// the bound and the last thing read.
pub async fn wait_for<T>(
    bound: Duration,
    label: &str,
    mut ask: impl AsyncFnMut() -> Result<T, String>,
) -> T {
    let mut last = String::new();
    let got = tokio::time::timeout(bound, async {
        loop {
            match ask().await {
                Ok(value) => return value,
                Err(read) => last = read,
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
    .await;
    match got {
        Ok(value) => value,
        Err(_) => panic!("{label} did not happen within {bound:?}; it last read:\n{last}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_wait_for_hands_back_the_first_good_answer() {
        let mut asked = 0u32;
        let got = wait_for(STEP_TIMEOUT, "the third ask", async || {
            asked += 1;
            if asked == 3 {
                Ok(asked * 10)
            } else {
                Err(format!("ask {asked}"))
            }
        })
        .await;
        assert_eq!(got, 30);
    }

    #[tokio::test(start_paused = true)]
    #[should_panic(
        expected = "a row did not happen within 5s; it last read:\nNo match is running."
    )]
    async fn a_wait_for_that_never_answers_fails_with_what_it_last_read() {
        wait_for::<()>(Duration::from_secs(5), "a row", async || {
            Err("No match is running.".to_owned())
        })
        .await;
    }

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

//! The world's clock provider (doc 43c §2, unit TS.10b).
//!
//! One handle per process, made at composition and carried as a parameter: in
//! `mush_server::world_task::ServerDeps` into the world loop, and from there to
//! the arenas the world hosts. There is no global, so two rigs in one process
//! never see each other's clock, and every converted call site can be found by
//! grep.
//!
//! - [`TimeSource::System`] is production. Unix time is the OS clock and a sleep
//!   is tokio's, exactly as before this type existed.
//! - [`TimeSource::Tokio`] is for tests. Unix time is derived from tokio's
//!   clock, so a `start_paused` runtime moves the world's unix time and its
//!   sleeps together. Only [`TimeSource::tokio_at`], which a test calls, makes
//!   one; no config key selects it.
//!
//! The provider is for the reads that *gate* behaviour: the world's `now`, the
//! timer due times, the shutdown grace. Reads that *measure* (loop lag, pulse
//! costs, uptime displays) stay on `std`, so a paused test still reports true
//! wall-clock latency.

use std::time::Duration;

/// Where the world reads the time (doc 43c §2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimeSource {
    /// Production: unix time is the OS clock, sleeps are tokio's (real time).
    #[default]
    System,
    /// Tests: unix time is `epoch_unix_ms` plus the tokio time elapsed since
    /// `base`, so a paused runtime's auto-advance moves both together.
    Tokio {
        /// The unix milliseconds this source reads at `base`.
        epoch_unix_ms: i64,
        /// The tokio instant that `epoch_unix_ms` names.
        base: tokio::time::Instant,
    },
}

impl TimeSource {
    /// A test clock that reads `epoch_unix_ms` now and advances with tokio's
    /// clock from here.
    #[must_use]
    pub fn tokio_at(epoch_unix_ms: i64) -> Self {
        Self::Tokio {
            epoch_unix_ms,
            base: tokio::time::Instant::now(),
        }
    }

    /// Whether this is the test arm.
    #[must_use]
    pub const fn is_tokio(&self) -> bool {
        matches!(self, Self::Tokio { .. })
    }

    /// The current time as unix milliseconds.
    #[must_use]
    pub fn unix_ms(&self) -> i64 {
        match self {
            Self::System => crate::unix_now_ms(),
            Self::Tokio {
                epoch_unix_ms,
                base,
            } => {
                let elapsed = i64::try_from(base.elapsed().as_millis()).unwrap_or(i64::MAX);
                epoch_unix_ms.saturating_add(elapsed)
            }
        }
    }

    /// The current time as unix seconds.
    #[must_use]
    pub fn unix_s(&self) -> i64 {
        match self {
            Self::System => crate::unix_now_s(),
            Self::Tokio { .. } => self.unix_ms().div_euclid(1000),
        }
    }

    /// A tokio instant for a gating deadline. Both arms read tokio's clock,
    /// which on a running runtime is the monotonic OS clock.
    #[must_use]
    pub fn instant(&self) -> tokio::time::Instant {
        tokio::time::Instant::now()
    }

    /// Sleep until `due`, in unix milliseconds. An already-past deadline
    /// returns at once.
    ///
    /// On [`TimeSource::System`] a wall-clock deadline is armed against a
    /// monotonic sleep: a backward NTP step delays it by the step size (the
    /// world loop re-arms each iteration), a forward step fires it early. On
    /// [`TimeSource::Tokio`] the deadline is exact tokio time.
    pub async fn sleep_until_unix_ms(&self, due: i64) {
        match self {
            Self::System => {
                let delta = due.saturating_sub(crate::unix_now_ms()).max(0);
                tokio::time::sleep(Duration::from_millis(u64::try_from(delta).unwrap_or(0))).await;
            }
            Self::Tokio {
                epoch_unix_ms,
                base,
            } => {
                let offset = u64::try_from(due.saturating_sub(*epoch_unix_ms)).unwrap_or(0);
                tokio::time::sleep_until(*base + Duration::from_millis(offset)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TimeSource;
    use std::time::Duration;

    #[test]
    fn the_default_is_the_system_clock() {
        assert_eq!(TimeSource::default(), TimeSource::System);
        let before = crate::unix_now_ms();
        let read = TimeSource::System.unix_ms();
        assert!(read >= before && read - before < 60_000);
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn the_tokio_arm_moves_with_the_paused_clock() {
        let clock = TimeSource::tokio_at(1_000_000);
        assert!(clock.is_tokio());
        assert_eq!(clock.unix_ms(), 1_000_000);
        assert_eq!(clock.unix_s(), 1_000);
        tokio::time::sleep(Duration::from_secs(90)).await;
        assert_eq!(clock.unix_ms(), 1_090_000);
        clock.sleep_until_unix_ms(1_200_500).await;
        assert_eq!(clock.unix_ms(), 1_200_500);
        // A past deadline returns at once.
        clock.sleep_until_unix_ms(5).await;
        assert_eq!(clock.unix_ms(), 1_200_500);
    }
}

//! The in-character (IC) game clock.
//!
//! Pure arithmetic over a caller-supplied wall-clock timestamp — core logic
//! never calls `SystemTime`. The IC calendar is a fictional 365-day year
//! (no leap days) counted from the IC epoch, which is all the weather and
//! ruleset systems need: stable seasons and times of day, configurable
//! speed ([`GameClock::ic_ratio`]) and phase ([`GameClock::epoch_offset`]).

use serde::{Deserialize, Serialize};

/// Seconds per IC day.
pub const SECS_PER_DAY: i64 = 86_400;

/// Days per IC year (fictional calendar; no leap days).
pub const DAYS_PER_YEAR: i64 = 365;

/// Converts real (OOC) unix time into IC time and derives season and
/// time-of-day from it.
///
/// `ic = epoch_offset + now * ic_ratio` — an `ic_ratio` of 4 makes a full
/// IC day pass in 6 real hours; `epoch_offset` (IC seconds) shifts the
/// calendar so a game can open in whatever season it likes.
///
/// ```
/// use mush_core::clock::{GameClock, Season};
/// let clock = GameClock { ic_ratio: 1, epoch_offset: 200 * 86_400 };
/// assert_eq!(clock.season(0), Season::Summer); // IC day 200
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameClock {
    /// How many IC seconds pass per real second (0 is treated as 1).
    pub ic_ratio: u32,
    /// IC seconds added after scaling; sets the IC calendar's phase.
    pub epoch_offset: i64,
}

impl Default for GameClock {
    fn default() -> Self {
        GameClock {
            ic_ratio: 1,
            epoch_offset: 0,
        }
    }
}

/// The four seasons, by IC day-of-year (meteorological-style quarters).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Season {
    /// IC days 334..365 and 0..59.
    Winter,
    /// IC days 59..151.
    Spring,
    /// IC days 151..243.
    Summer,
    /// IC days 243..334.
    Autumn,
}

impl std::fmt::Display for Season {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Season::Winter => "winter",
            Season::Spring => "spring",
            Season::Summer => "summer",
            Season::Autumn => "autumn",
        })
    }
}

/// Coarse IC time-of-day buckets, by IC hour.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TimeOfDay {
    /// 00:00–05:59.
    Night,
    /// 06:00–07:59.
    Dawn,
    /// 08:00–11:59.
    Morning,
    /// 12:00–17:59.
    Afternoon,
    /// 18:00–19:59.
    Dusk,
    /// 20:00–23:59.
    Evening,
}

impl std::fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            TimeOfDay::Night => "night",
            TimeOfDay::Dawn => "dawn",
            TimeOfDay::Morning => "morning",
            TimeOfDay::Afternoon => "afternoon",
            TimeOfDay::Dusk => "dusk",
            TimeOfDay::Evening => "evening",
        })
    }
}

impl GameClock {
    /// IC unix-style timestamp for a real (OOC) unix timestamp.
    #[must_use]
    pub fn ic_unix(&self, now_unix: i64) -> i64 {
        let ratio = i64::from(self.ic_ratio.max(1));
        self.epoch_offset
            .saturating_add(now_unix.saturating_mul(ratio))
    }

    /// IC hour of day, `0..24`.
    #[must_use]
    pub fn ic_hour(&self, now_unix: i64) -> u32 {
        let secs = self.ic_unix(now_unix).rem_euclid(SECS_PER_DAY);
        (secs / 3_600) as u32
    }

    /// IC day of year, `0..365`.
    #[must_use]
    pub fn ic_day_of_year(&self, now_unix: i64) -> u32 {
        let days = self.ic_unix(now_unix).div_euclid(SECS_PER_DAY);
        days.rem_euclid(DAYS_PER_YEAR) as u32
    }

    /// IC year number (year 0 begins at IC timestamp 0; negative before).
    #[must_use]
    pub fn ic_year(&self, now_unix: i64) -> i64 {
        self.ic_unix(now_unix)
            .div_euclid(SECS_PER_DAY * DAYS_PER_YEAR)
    }

    /// Season for the given real timestamp.
    #[must_use]
    pub fn season(&self, now_unix: i64) -> Season {
        match self.ic_day_of_year(now_unix) {
            59..=150 => Season::Spring,
            151..=242 => Season::Summer,
            243..=333 => Season::Autumn,
            _ => Season::Winter,
        }
    }

    /// Time-of-day bucket for the given real timestamp.
    #[must_use]
    pub fn time_of_day(&self, now_unix: i64) -> TimeOfDay {
        match self.ic_hour(now_unix) {
            0..=5 => TimeOfDay::Night,
            6..=7 => TimeOfDay::Dawn,
            8..=11 => TimeOfDay::Morning,
            12..=17 => TimeOfDay::Afternoon,
            18..=19 => TimeOfDay::Dusk,
            _ => TimeOfDay::Evening,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_clock_maps_day_and_hour() {
        let c = GameClock::default();
        // Day 200, 13:00.
        let now = 200 * SECS_PER_DAY + 13 * 3_600;
        assert_eq!(c.ic_unix(now), now);
        assert_eq!(c.ic_day_of_year(now), 200);
        assert_eq!(c.ic_hour(now), 13);
        assert_eq!(c.season(now), Season::Summer);
        assert_eq!(c.time_of_day(now), TimeOfDay::Afternoon);
        assert_eq!(c.ic_year(now), 0);
    }

    #[test]
    fn season_boundaries() {
        let c = GameClock::default();
        let at_day = |d: i64| d * SECS_PER_DAY;
        assert_eq!(c.season(at_day(0)), Season::Winter);
        assert_eq!(c.season(at_day(58)), Season::Winter);
        assert_eq!(c.season(at_day(59)), Season::Spring);
        assert_eq!(c.season(at_day(150)), Season::Spring);
        assert_eq!(c.season(at_day(151)), Season::Summer);
        assert_eq!(c.season(at_day(242)), Season::Summer);
        assert_eq!(c.season(at_day(243)), Season::Autumn);
        assert_eq!(c.season(at_day(333)), Season::Autumn);
        assert_eq!(c.season(at_day(334)), Season::Winter);
        assert_eq!(c.season(at_day(364)), Season::Winter);
        // Wraps into the next year.
        assert_eq!(c.season(at_day(365 + 100)), Season::Spring);
    }

    #[test]
    fn time_of_day_buckets() {
        let c = GameClock::default();
        let at_hour = |h: i64| h * 3_600;
        assert_eq!(c.time_of_day(at_hour(0)), TimeOfDay::Night);
        assert_eq!(c.time_of_day(at_hour(5)), TimeOfDay::Night);
        assert_eq!(c.time_of_day(at_hour(6)), TimeOfDay::Dawn);
        assert_eq!(c.time_of_day(at_hour(8)), TimeOfDay::Morning);
        assert_eq!(c.time_of_day(at_hour(12)), TimeOfDay::Afternoon);
        assert_eq!(c.time_of_day(at_hour(18)), TimeOfDay::Dusk);
        assert_eq!(c.time_of_day(at_hour(20)), TimeOfDay::Evening);
        assert_eq!(c.time_of_day(at_hour(23)), TimeOfDay::Evening);
    }

    #[test]
    fn ratio_speeds_up_ic_time() {
        // 24x: one real hour = one IC day.
        let c = GameClock {
            ic_ratio: 24,
            epoch_offset: 0,
        };
        assert_eq!(c.ic_day_of_year(SECS_PER_DAY / 24), 1);
        assert_eq!(c.ic_hour(3_600 / 24), 1);
        // Ratio 0 behaves as 1 (documented safeguard).
        let c0 = GameClock {
            ic_ratio: 0,
            epoch_offset: 0,
        };
        assert_eq!(c0.ic_unix(42), 42);
    }

    #[test]
    fn offset_shifts_calendar_and_negatives_wrap() {
        let c = GameClock {
            ic_ratio: 1,
            epoch_offset: 250 * SECS_PER_DAY,
        };
        assert_eq!(c.ic_day_of_year(0), 250);
        assert_eq!(c.season(0), Season::Autumn);

        // Before the epoch: hours and days still wrap into range.
        let c = GameClock::default();
        assert_eq!(c.ic_hour(-3_600), 23);
        assert_eq!(c.ic_day_of_year(-SECS_PER_DAY), 364);
        assert_eq!(c.ic_year(-1), -1);
    }
}

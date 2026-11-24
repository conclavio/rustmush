//! **The instant a rig boots at** (doc 51 §3.6, rule T, Q-51-9).
//!
//! The IC calendar is a function of the real clock: an IC day, a month and
//! every schedule that hangs from them turn at fixed real instants. A rig that
//! boots its engine at the wall clock therefore runs each story at whatever
//! point of the IC day the test happened to start, and a story that is wrong
//! in one stretch of that day fails only in the runs that start there. Two
//! rows of `docs/flaky-tests.md` were this and nothing else: an IC day turned
//! inside one step of a commerce story, in about one run in thirty.
//!
//! So a rig does not read the wall clock. It boots at [`STORY_EPOCH`], and a
//! story about anything scheduled on the game clock runs once at each
//! [`Phase`] of [`Phase::NAMED`], through [`phases!`](crate::phases):
//!
//! ```ignore
//! use mush_storykit::{Calendar, Phase, phases};
//!
//! /// The rig's own boot, at the phase the story is run at.
//! fn boot(phase: Phase) -> i64 {
//!     // The rate and the IC instant at unix 0, from the config the rig boots with.
//!     phase.boot_unix(Calendar::new(24, 0))
//! }
//!
//! fn a_listing_is_posted_again_each_day(phase: Phase) {
//!     let booted = boot(phase);
//!     assert!(booted >= mush_storykit::STORY_EPOCH);
//! }
//! phases!(#[test] a_listing_is_posted_again_each_day);
//! ```
//!
//! That is three tests, `a_listing_is_posted_again_each_day::at_the_epoch`,
//! `::before_a_days_turn` and `::mid_month`.
//!
//! **The soak draws its own.** `STORY_PHASE=random` gives every boot a phase
//! drawn from a whole schedule year, and each boot prints the phase it took, so
//! a failure names the phase to pin: `STORY_PHASE=+<seconds>` runs every story
//! there, and a story keeps the phase for good by naming it in its `phases!`.

use std::hash::{BuildHasher, Hasher};

/// One IC hour, in IC seconds.
pub const IC_HOUR: i64 = 3_600;

/// One IC day, in IC seconds. The game counts days from IC second zero.
pub const IC_DAY: i64 = 86_400;

/// **One schedule month**, in IC seconds: thirty IC days, counted from IC
/// second zero. It is the month the game's monthly schedules turn on (the
/// Assembly's vote, a stockpile's monthly budget), which is not the civil
/// calendar's month of 28 to 31 days. mush-server's `commerce_composition`
/// suite holds this figure to tacsim's own.
pub const IC_MONTH: i64 = 30 * IC_DAY;

/// How long before an IC day's turn [`Phase::DayTurn`] boots, in IC seconds.
pub const BEFORE_DAY_TURN: i64 = 50;

/// How long before a schedule month's turn [`Phase::MidMonth`] boots, in IC
/// days.
pub const BEFORE_MONTH_TURN_DAYS: i64 = 15;

/// **The instant a stock rig boots at**, in unix seconds.
///
/// It is one fixed second, so a story sees the same IC calendar on every run.
/// It is chosen to sit on no IC hour, day or schedule month boundary under the
/// two calendars the rigs in the tree use, so that a rig does not start with
/// every schedule aligned:
///
/// | Calendar | IC time of day | Day of the schedule month |
/// |---|---|---|
/// | the servers' (`[game]` at its defaults, 24 IC seconds a second) | 09:30:48 | 7 of 0 to 29 |
/// | the engine test world's (one IC second a second, from zero) | 17:23:47 | 10 of 0 to 29 |
///
/// The tests below hold both rows. The instant is in 2030, later than any
/// run's wall clock for years. Nothing in a rig depends on that: a rig's
/// clock is the one it sets, and the suite passes the same booted in the past.
pub const STORY_EPOCH: i64 = 1_900_862_627;

/// **The names of the three tests [`phases!`](crate::phases) makes of a
/// story**, in the order of [`Phase::NAMED`]. The known-flaky gate in
/// `mush-contract-tests` and `scripts/flake_report.py` read a test named
/// `<story>::<one of these>` as that story at a phase.
pub const PHASE_TESTS: [&str; 3] = ["at_the_epoch", "before_a_days_turn", "mid_month"];

/// The environment variable that overrides the phase of every boot in a run.
pub const PHASE_VAR: &str = "STORY_PHASE";

/// **How a rig's IC clock follows the real one**: `ic = ic_at_unix_zero +
/// rate * unix`, the rule of `mush_core::GameClock`. A rig builds it from the
/// config it boots its engine with (`Calendar::new(clock.ic_ratio,
/// clock.epoch_offset)`), so it cannot disagree with the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Calendar {
    rate: i64,
    ic_at_unix_zero: i64,
}

impl Calendar {
    /// A calendar of `rate` IC seconds a real second that reads
    /// `ic_at_unix_zero` at unix second zero. A rate of zero is read as one,
    /// as the engine reads it.
    #[must_use]
    pub fn new(rate: u32, ic_at_unix_zero: i64) -> Calendar {
        Calendar {
            rate: i64::from(rate.max(1)),
            ic_at_unix_zero,
        }
    }

    /// The IC second at real second `unix`.
    #[must_use]
    pub fn ic_at(self, unix: i64) -> i64 {
        self.ic_at_unix_zero
            .saturating_add(unix.saturating_mul(self.rate))
    }

    /// The last real second whose IC time is at or before `ic`.
    #[must_use]
    pub fn at_or_before(self, ic: i64) -> i64 {
        ic.saturating_sub(self.ic_at_unix_zero)
            .div_euclid(self.rate)
    }

    /// The first real second whose IC time is at or after `ic`.
    #[must_use]
    pub fn at_or_after(self, ic: i64) -> i64 {
        let unix = self.at_or_before(ic);
        if self.ic_at(unix) < ic {
            unix.saturating_add(1)
        } else {
            unix
        }
    }

    /// The IC seconds since `period` last turned, at real second `unix`. Zero
    /// is the turn itself.
    #[must_use]
    pub fn since_turn(self, unix: i64, period: i64) -> i64 {
        self.ic_at(unix).rem_euclid(period.max(1))
    }

    /// The IC seconds until `period` next turns, at real second `unix`:
    /// `1..=period`.
    #[must_use]
    pub fn until_turn(self, unix: i64, period: i64) -> i64 {
        period.max(1).saturating_sub(self.since_turn(unix, period))
    }

    /// The real seconds twelve schedule months take: the span a drawn phase
    /// is drawn from, which holds every time of day and every day of a month.
    #[must_use]
    pub fn real_year(self) -> i64 {
        (12 * IC_MONTH).div_euclid(self.rate).max(1)
    }

    /// Where real second `unix` falls in this calendar, for a rig's boot line.
    #[must_use]
    pub fn place(self, unix: i64) -> String {
        let of_day = self.since_turn(unix, IC_DAY);
        format!(
            "IC {:02}:{:02}:{:02} on day {} of the schedule month's 0 to 29",
            of_day.div_euclid(IC_HOUR),
            of_day.rem_euclid(IC_HOUR).div_euclid(60),
            of_day.rem_euclid(60),
            self.since_turn(unix, IC_MONTH).div_euclid(IC_DAY),
        )
    }
}

/// **A named instant of the IC calendar to boot a rig at.**
///
/// The first three are [`Phase::NAMED`], the phases a schedule-bound story
/// runs at on every pull request. [`Phase::Past`] is a phase the soak drew or
/// a story pinned after the soak found it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// [`STORY_EPOCH`] itself: on no boundary.
    Epoch,
    /// The last real second at least [`BEFORE_DAY_TURN`] IC seconds before the
    /// first IC day's turn after the epoch: the day turns in the story's first
    /// step, and every later turn falls on a whole day from the boot.
    DayTurn,
    /// The epoch's time of day, in the first IC day at or after the epoch that
    /// begins [`BEFORE_MONTH_TURN_DAYS`] days before a schedule month's turn.
    MidMonth,
    /// This many real seconds past [`STORY_EPOCH`].
    Past(i64),
}

impl Phase {
    /// The three phases [`phases!`](crate::phases) runs a story at.
    pub const NAMED: [Phase; 3] = [Phase::Epoch, Phase::DayTurn, Phase::MidMonth];

    /// The phase's word, as [`PHASE_VAR`] reads it.
    #[must_use]
    pub fn word(self) -> String {
        match self {
            Phase::Epoch => "epoch".to_owned(),
            Phase::DayTurn => "day-turn".to_owned(),
            Phase::MidMonth => "mid-month".to_owned(),
            Phase::Past(seconds) => format!("+{seconds}"),
        }
    }

    /// The real second this phase is under `calendar`, whatever the
    /// environment says. It is never before [`STORY_EPOCH`].
    #[must_use]
    pub fn instant(self, calendar: Calendar) -> i64 {
        match self {
            Phase::Epoch => STORY_EPOCH,
            Phase::DayTurn => {
                let mut turn = calendar
                    .ic_at(STORY_EPOCH)
                    .saturating_add(calendar.until_turn(STORY_EPOCH, IC_DAY));
                let mut unix = calendar.at_or_before(turn.saturating_sub(BEFORE_DAY_TURN));
                // An epoch already inside the last seconds of its day takes the next turn.
                while unix < STORY_EPOCH {
                    turn = turn.saturating_add(IC_DAY);
                    unix = calendar.at_or_before(turn.saturating_sub(BEFORE_DAY_TURN));
                }
                unix
            }
            Phase::MidMonth => {
                let day = calendar
                    .since_turn(STORY_EPOCH, IC_MONTH)
                    .div_euclid(IC_DAY);
                let wanted = IC_MONTH.div_euclid(IC_DAY) - BEFORE_MONTH_TURN_DAYS;
                let days_on = (wanted - day).rem_euclid(IC_MONTH.div_euclid(IC_DAY));
                calendar.at_or_after(
                    calendar
                        .ic_at(STORY_EPOCH)
                        .saturating_add(days_on.saturating_mul(IC_DAY)),
                )
            }
            Phase::Past(seconds) => STORY_EPOCH.saturating_add(seconds.max(0)),
        }
    }

    /// **The real second a rig boots at for this phase**: [`Phase::instant`],
    /// unless [`PHASE_VAR`] asks for another.
    ///
    /// - unset or empty: this phase;
    /// - `random`: a phase drawn from a whole schedule year past the epoch,
    ///   afresh at every call;
    /// - `+<seconds>`: that many real seconds past the epoch, for every boot;
    /// - `epoch`, `day-turn` or `mid-month`: that named phase, for every boot.
    ///
    /// It prints one line naming the phase it took and where that falls in the
    /// calendar. A test runner shows the line with a failure, so a failure
    /// under a drawn phase names the phase to pin.
    ///
    /// A story that boots more than one engine at the same instant calls this
    /// once and hands the second on.
    ///
    /// # Panics
    ///
    /// When [`PHASE_VAR`] holds none of the forms above. A misspelled phase
    /// must not quietly run the story's own.
    #[must_use]
    pub fn boot_unix(self, calendar: Calendar) -> i64 {
        let asked = std::env::var(PHASE_VAR).ok();
        let taken = self.under(asked.as_deref(), calendar, || {
            std::collections::hash_map::RandomState::new()
                .build_hasher()
                .finish()
        });
        let unix = taken.instant(calendar);
        let why = match asked.as_deref().map(str::trim) {
            None | Some("") => String::new(),
            Some(asked) => format!(
                " ({PHASE_VAR}={asked} in place of the story's {}; pin it with {PHASE_VAR}={})",
                self.word(),
                taken.word()
            ),
        };
        println!(
            "story phase {}: boot at unix {unix}, {}{why}",
            taken.word(),
            calendar.place(unix)
        );
        unix
    }

    /// **Whether [`PHASE_VAR`] asks for a phase of its own**, so that a boot
    /// takes that one and not the story's. A story that checks what its own
    /// pinned phase puts where (that the day turned inside the step the phase
    /// was pinned for) checks it only when this is false.
    #[must_use]
    pub fn overridden() -> bool {
        std::env::var(PHASE_VAR).is_ok_and(|asked| !asked.trim().is_empty())
    }

    /// The phase to take when the environment asked for `asked`; `draw` is
    /// the source of a drawn phase.
    fn under(self, asked: Option<&str>, calendar: Calendar, draw: impl FnOnce() -> u64) -> Phase {
        let asked = asked.map(str::trim).unwrap_or_default();
        let taken = match asked {
            "" => Some(self),
            "random" => {
                let year = u64::try_from(calendar.real_year()).unwrap_or(1).max(1);
                Some(Phase::Past(
                    i64::try_from(draw().rem_euclid(year)).unwrap_or(0),
                ))
            }
            "epoch" => Some(Phase::Epoch),
            "day-turn" => Some(Phase::DayTurn),
            "mid-month" => Some(Phase::MidMonth),
            past => past
                .strip_prefix('+')
                .and_then(|seconds| seconds.parse::<i64>().ok())
                .filter(|seconds| *seconds >= 0)
                .map(Phase::Past),
        };
        assert!(
            taken.is_some(),
            "{PHASE_VAR}={asked:?} is not a phase: leave it unset, or give random, epoch, \
             day-turn, mid-month or +<real seconds past the epoch>"
        );
        taken.unwrap_or(self)
    }
}

/// **Run one story at each named phase** (doc 51 §3.6).
///
/// The story is a function of one [`Phase`], written as any other function.
/// `phases!` takes the test attribute to run it under and the story's name,
/// and makes a module of that name holding three tests, `at_the_epoch`,
/// `before_a_days_turn` and `mid_month`, each of which calls the story with
/// its phase. A test's name is therefore `<story>::<phase>`, so a filter on
/// the story's name still selects all of it.
///
/// ```ignore
/// use mush_storykit::{Phase, phases};
///
/// fn the_clerk_pays_once_a_day(phase: Phase) {
///     assert!(Phase::NAMED.contains(&phase));
/// }
/// phases!(#[test] the_clerk_pays_once_a_day);
/// ```
///
/// An async story says so, and takes whatever runtime attribute it needs:
/// `phases!(#[tokio::test] async the_queue_runs_the_return);`.
///
/// **A pinned phase** is a fourth test, named for what the phase puts where.
/// It is how a phase the soak found stays covered:
///
/// ```ignore
/// phases!(#[test] the_clerk_pays_once_a_day, the_day_turns_at_the_counter = Phase::Past(1_322));
/// ```
#[macro_export]
macro_rules! phases {
    ($(#[$test:meta])+ async $story:ident $(, $pinned:ident = $phase:expr)* $(,)?) => {
        mod $story {
            // A pinned phase is written in the story's own scope.
            #[allow(unused_imports)]
            use super::*;
            $crate::phases!(
                @each [$(#[$test])+] async $story;
                at_the_epoch = $crate::Phase::Epoch,
                before_a_days_turn = $crate::Phase::DayTurn,
                mid_month = $crate::Phase::MidMonth,
                $($pinned = $phase,)*
            );
        }
    };
    ($(#[$test:meta])+ $story:ident $(, $pinned:ident = $phase:expr)* $(,)?) => {
        mod $story {
            // A pinned phase is written in the story's own scope.
            #[allow(unused_imports)]
            use super::*;
            $crate::phases!(
                @each [$(#[$test])+] sync $story;
                at_the_epoch = $crate::Phase::Epoch,
                before_a_days_turn = $crate::Phase::DayTurn,
                mid_month = $crate::Phase::MidMonth,
                $($pinned = $phase,)*
            );
        }
    };
    (@each [$($test:tt)+] $kind:ident $story:ident;) => {};
    (@each [$($test:tt)+] async $story:ident; $name:ident = $phase:expr, $($rest:tt)*) => {
        $($test)+
        async fn $name() {
            super::$story($phase).await;
        }
        $crate::phases!(@each [$($test)+] async $story; $($rest)*);
    };
    (@each [$($test:tt)+] sync $story:ident; $name:ident = $phase:expr, $($rest:tt)*) => {
        $($test)+
        fn $name() {
            super::$story($phase);
        }
        $crate::phases!(@each [$($test)+] sync $story; $($rest)*);
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The servers' calendar: `[game]` at its defaults (`ic_epoch` 2020-01-01
    /// at unix zero, on the 365-day calendar) under the rigs' `ic_ratio = 24`.
    fn servers() -> Calendar {
        Calendar::new(24, 2020 * 365 * IC_DAY)
    }

    /// The engine test world's calendar: one IC second a second, from zero.
    fn test_world() -> Calendar {
        Calendar::new(1, 0)
    }

    #[test]
    fn a_calendar_reads_ic_time_and_finds_the_second_for_an_instant() {
        let calendar = Calendar::new(24, 1_000);
        assert_eq!(calendar.ic_at(0), 1_000);
        assert_eq!(calendar.ic_at(10), 1_240);
        // IC 1,240 is second 10 exactly; 1,250 falls inside second 10.
        assert_eq!(calendar.at_or_before(1_240), 10);
        assert_eq!(calendar.at_or_after(1_240), 10);
        assert_eq!(calendar.at_or_before(1_250), 10);
        assert_eq!(calendar.at_or_after(1_250), 11);
        assert_eq!(calendar.at_or_before(999), -1);
        // A rate of zero is one, as the engine reads it.
        assert_eq!(Calendar::new(0, 0).ic_at(7), 7);
    }

    #[test]
    fn a_turn_is_counted_from_and_to() {
        let calendar = Calendar::new(1, 0);
        assert_eq!(calendar.since_turn(IC_DAY, IC_DAY), 0);
        assert_eq!(calendar.until_turn(IC_DAY, IC_DAY), IC_DAY);
        assert_eq!(calendar.since_turn(IC_DAY + 5, IC_DAY), 5);
        assert_eq!(calendar.until_turn(IC_DAY + 5, IC_DAY), IC_DAY - 5);
        assert_eq!(calendar.since_turn(-1, IC_DAY), IC_DAY - 1);
    }

    /// The epoch's whole point: it is well inside its hour, its day and its
    /// month, under both calendars the rigs use.
    #[test]
    fn the_epoch_sits_on_no_hour_day_or_month_boundary() {
        for (calendar, of_day, day) in [
            (servers(), 9 * IC_HOUR + 30 * 60 + 48, 7),
            (test_world(), 17 * IC_HOUR + 23 * 60 + 47, 10),
        ] {
            assert_eq!(calendar.since_turn(STORY_EPOCH, IC_DAY), of_day);
            assert_eq!(
                calendar
                    .since_turn(STORY_EPOCH, IC_MONTH)
                    .div_euclid(IC_DAY),
                day
            );
            for (period, clear) in [
                (IC_HOUR, 20 * 60),
                (IC_DAY, 6 * IC_HOUR),
                (IC_MONTH, 5 * IC_DAY),
            ] {
                assert!(
                    calendar.since_turn(STORY_EPOCH, period) >= clear
                        && calendar.until_turn(STORY_EPOCH, period) >= clear,
                    "{calendar:?}: the epoch is within {clear} IC seconds of a turn of {period}"
                );
            }
        }
        assert_eq!(
            servers().place(STORY_EPOCH),
            "IC 09:30:48 on day 7 of the schedule month's 0 to 29"
        );
    }

    #[test]
    fn the_day_turn_phase_boots_just_before_a_days_turn() {
        for calendar in [
            servers(),
            test_world(),
            Calendar::new(3_600, 0),
            Calendar::new(7, 13),
        ] {
            let unix = Phase::DayTurn.instant(calendar);
            assert!(unix >= STORY_EPOCH);
            let until = calendar.until_turn(unix, IC_DAY);
            assert!(
                until >= BEFORE_DAY_TURN,
                "{calendar:?}: {until} IC seconds before the turn"
            );
            // The next second is too close to the turn, or past it: this is the last such second.
            let next = calendar.until_turn(unix + 1, IC_DAY);
            assert!(
                next < BEFORE_DAY_TURN || next > until,
                "{calendar:?}: {next}"
            );
            // The first turn after the epoch, unless the epoch stood inside its last seconds.
            assert!(calendar.ic_at(unix) - calendar.ic_at(STORY_EPOCH) < 2 * IC_DAY);
        }
        // Under the servers' calendar a second is 24 IC seconds: three seconds before the turn.
        assert_eq!(
            servers().until_turn(Phase::DayTurn.instant(servers()), IC_DAY),
            72
        );
    }

    #[test]
    fn the_mid_month_phase_keeps_the_time_of_day_fifteen_days_before_a_months_turn() {
        for calendar in [servers(), test_world()] {
            let unix = Phase::MidMonth.instant(calendar);
            assert!(unix >= STORY_EPOCH);
            assert_eq!(
                calendar.since_turn(unix, IC_MONTH).div_euclid(IC_DAY),
                30 - BEFORE_MONTH_TURN_DAYS
            );
            assert_eq!(
                calendar.since_turn(unix, IC_DAY),
                calendar.since_turn(STORY_EPOCH, IC_DAY)
            );
            assert!(calendar.ic_at(unix) - calendar.ic_at(STORY_EPOCH) < IC_MONTH);
        }
    }

    #[test]
    fn the_three_named_phases_are_three_instants() {
        let calendar = servers();
        let [epoch, day_turn, mid_month] = Phase::NAMED.map(|phase| phase.instant(calendar));
        assert_eq!(epoch, STORY_EPOCH);
        assert!(epoch < day_turn && day_turn < mid_month);
        assert_eq!(Phase::Past(90).instant(calendar), STORY_EPOCH + 90);
        assert_eq!(Phase::Past(-5).instant(calendar), STORY_EPOCH);
    }

    #[test]
    fn the_environment_overrides_every_boot_or_none() {
        let calendar = servers();
        let never = || -> u64 { 0 };
        assert_eq!(Phase::DayTurn.under(None, calendar, never), Phase::DayTurn);
        assert_eq!(
            Phase::DayTurn.under(Some(" "), calendar, never),
            Phase::DayTurn
        );
        assert_eq!(
            Phase::Epoch.under(Some("day-turn"), calendar, never),
            Phase::DayTurn
        );
        assert_eq!(
            Phase::Epoch.under(Some("mid-month"), calendar, never),
            Phase::MidMonth
        );
        assert_eq!(
            Phase::MidMonth.under(Some("epoch"), calendar, never),
            Phase::Epoch
        );
        assert_eq!(
            Phase::Epoch.under(Some("+1322"), calendar, never),
            Phase::Past(1_322)
        );
        // Every word a phase prints is a word the variable reads back.
        for phase in [
            Phase::Epoch,
            Phase::DayTurn,
            Phase::MidMonth,
            Phase::Past(77),
        ] {
            assert_eq!(
                Phase::Epoch.under(Some(&phase.word()), calendar, never),
                phase
            );
        }
    }

    #[test]
    fn a_drawn_phase_falls_inside_one_schedule_year() {
        let calendar = servers();
        // Twelve months of thirty IC days at 24 to 1 are fifteen real days.
        assert_eq!(calendar.real_year(), 15 * 86_400);
        for drawn in [0, 1, 1_295_999, 1_296_000, u64::MAX] {
            let taken = Phase::Epoch.under(Some("random"), calendar, || drawn);
            assert!(matches!(taken, Phase::Past(_)), "{taken:?}");
            let seconds = taken.instant(calendar) - STORY_EPOCH;
            assert!((0..calendar.real_year()).contains(&seconds), "{seconds}");
        }
        assert_eq!(
            Phase::Epoch.under(Some("random"), calendar, || 1_296_007),
            Phase::Past(7)
        );
    }

    #[test]
    #[should_panic(expected = "STORY_PHASE=\"midmonth\" is not a phase")]
    fn a_misspelled_phase_fails_the_boot() {
        let _ = Phase::Epoch.under(Some("midmonth"), servers(), || 0);
    }

    #[test]
    #[should_panic(expected = "is not a phase")]
    fn a_span_before_the_epoch_fails_the_boot() {
        let _ = Phase::Epoch.under(Some("+-4"), servers(), || 0);
    }

    /// A story as a suite writes one: a function of its phase. `phases!` makes a module of
    /// its name holding one test a phase, and one more for the pinned phase.
    fn a_sync_story(phase: Phase) {
        assert!(Phase::NAMED.contains(&phase) || phase == Phase::Past(1_322));
    }
    crate::phases!(
        #[test]
        a_sync_story,
        the_day_turns_at_the_counter = Phase::Past(1_322)
    );

    async fn an_async_story(phase: Phase) {
        tokio::task::yield_now().await;
        assert!(Phase::NAMED.contains(&phase));
    }
    crate::phases!(
        #[tokio::test]
        async an_async_story
    );
}

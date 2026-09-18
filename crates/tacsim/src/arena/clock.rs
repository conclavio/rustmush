//! **The arena's clock** (16b cut A.1; doc 16 §3.1 "Tempo", "Rate independence"
//! and "Clock and lag"; D-16-2).
//!
//! Three jobs, and deliberately no more:
//!
//! 1. **The adaptive tempo.** [`Clock::evaluate`] walks doc 16 §3.1's table: any
//!    engaged signal enters [`Tempo::Engaged`] and it is left `engaged_hold_ms`
//!    after the last one; any alert signal enters [`Tempo::Alert`] and it is left
//!    `alert_hold_ms` after the last one; otherwise [`Tempo::Quiet`]. The two
//!    holds are what make it hysteretic: a signal that stops for less than its
//!    hold never changes the tempo, so a firefight does not flap.
//! 2. **The lag clamp.** [`Clock::advance`] turns a wall reading into a `dt` in
//!    arena milliseconds, clamped to `max_catch_up_steps × period`. Past that the
//!    idle wall time is **skipped** and `lagged` is recorded; `dt` is never
//!    expanded to hide lag.
//! 3. **The substep length.** [`Clock::substep_ms`] and [`substeps`] cut a pulse
//!    into the substeps that make the tempo decide only how often the arena
//!    steps and never an outcome. No unit advances more than one cell in a
//!    substep, which is the invariant doc 16 §3.4 step 3 leans on.
//!
//! **The tempo decides nothing.** That is the whole point of D-16-2, and it is
//! why this file holds no state a resolution could read: nothing in it is an
//! input to motion, perception or damage.

use crate::config::{ArenaConfig, TempoConfig};
use serde::{Deserialize, Serialize};

/// The floor on a substep, milliseconds (doc 16 §3.1 "Rate independence").
///
/// It bounds the work one pulse can be subdivided into, and it is a floor rather
/// than advice because a zero-length substep cannot terminate.
pub const MIN_SUBSTEP_MS: u32 = 50;

/// The adaptive pulse rate (doc 16 §3.1's tempo table, D-16-2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tempo {
    /// No contact in range, no queued fire intent, no controller approaching.
    #[default]
    Quiet,
    /// Something is held, ordered or approaching.
    Alert,
    /// Shots, scheduled impacts or hostiles in weapons range. **The harness's
    /// 25 ms p99 applies here.**
    Engaged,
}

impl Tempo {
    /// Every tempo, from slowest to fastest.
    pub const ALL: [Tempo; 3] = [Tempo::Quiet, Tempo::Alert, Tempo::Engaged];

    /// This tempo's period in milliseconds, from `[tacsim.arena.tempo]`.
    #[must_use]
    pub const fn period_ms(&self, cfg: &TempoConfig) -> u32 {
        match self {
            Tempo::Quiet => cfg.quiet_ms,
            Tempo::Alert => cfg.alert_ms,
            Tempo::Engaged => cfg.engaged_ms,
        }
    }

    /// The wire word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Tempo::Quiet => "quiet",
            Tempo::Alert => "alert",
            Tempo::Engaged => "engaged",
        }
    }

    /// The byte [`crate::arena::hash::state_hash`] folds in.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Tempo::Quiet => 0,
            Tempo::Alert => 1,
            Tempo::Engaged => 2,
        }
    }
}

/// **What each cut tells the clock about this pulse** (doc 16 §3.1's tempo
/// table, read as six independent observations).
///
/// Every cut supplies its own and no cut reads another's: A.2's `step` passes
/// what motion and the intent queues know and leaves the rest `false` until cut
/// S mints contacts and cut W.1 fires shots. That is why this is a struct of flags
/// rather than a computed tempo — a cut that has not landed contributes nothing
/// instead of contributing a wrong answer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TempoSignals {
    /// Some unit holds a contact (classified or not) inside
    /// `tempo.alert_range_cm` (cut S).
    pub contact_within_alert_range: bool,
    /// A fire intent is queued (16b cut W.1 sets it; the queue is cut A's).
    pub fire_intent_queued: bool,
    /// A controller has entered `Approach` (cut N).
    pub ai_approaching: bool,
    /// A shot was fired this pulse (16b cut W.1).
    pub shot_fired: bool,
    /// An impact is scheduled (16b cut W.1: every accepted shot schedules one,
    /// so this and `shot_fired` move together).
    pub impact_scheduled: bool,
    /// Hostile units are within weapons range of each other. **Still `false`**:
    /// 16b §8's cut W row names the other two `Engaged` triggers, and a
    /// hostile-in-range sweep is a per-pulse cost nobody has bounded yet.
    pub hostile_in_weapons_range: bool,
}

impl TempoSignals {
    /// Nothing is happening — the `Quiet` row of doc 16 §3.1's table.
    #[must_use]
    pub const fn quiet() -> TempoSignals {
        TempoSignals {
            contact_within_alert_range: false,
            fire_intent_queued: false,
            ai_approaching: false,
            shot_fired: false,
            impact_scheduled: false,
            hostile_in_weapons_range: false,
        }
    }

    /// Whether any of the three `Engaged` triggers fired.
    #[must_use]
    pub const fn engaged(&self) -> bool {
        self.shot_fired || self.impact_scheduled || self.hostile_in_weapons_range
    }

    /// Whether any of the three `Alert` triggers fired.
    #[must_use]
    pub const fn alert(&self) -> bool {
        self.contact_within_alert_range || self.fire_intent_queued || self.ai_approaching
    }
}

/// **The arena's clock**: the tempo it is in, when each trigger was last seen,
/// and the wall reading the last `dt` was measured from.
///
/// The fields are private because they are a state machine, not a record: the
/// only legal transitions are the ones [`Clock::evaluate`] makes, and the only
/// legal way arena time moves is [`Clock::advance`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Clock {
    tempo: Tempo,
    tempo_since_ms: i64,
    last_alert_signal_ms: Option<i64>,
    last_engaged_signal_ms: Option<i64>,
    last_wall_ms: Option<i64>,
    lagged: bool,
    /// Set only by [`Clock::pin_tempo_for_tests`]; while it is set,
    /// [`Clock::evaluate`] records signals and changes nothing.
    pinned: Option<Tempo>,
}

impl Clock {
    /// A clock in [`Tempo::Quiet`] that has never seen a signal or a wall
    /// reading.
    #[must_use]
    pub fn new() -> Clock {
        Clock::default()
    }

    /// The tempo the arena is in.
    #[must_use]
    pub const fn tempo(&self) -> Tempo {
        self.tempo
    }

    /// The arena millisecond the current tempo was entered at.
    #[must_use]
    pub const fn tempo_since_ms(&self) -> i64 {
        self.tempo_since_ms
    }

    /// Whether the last [`Clock::advance`] clamped.
    #[must_use]
    pub const fn lagged(&self) -> bool {
        self.lagged
    }

    /// **Hold the tempo at `tempo`** — the one lever a determinism test needs
    /// (16b A7; doc 16 §3.1 "Rate independence").
    ///
    /// The property being tested is that *the tempo decides only how often `step`
    /// runs and never an outcome*, so the test has to drive one arena at three
    /// pulse cadences with the tempo **held**: an arena left to evaluate its own
    /// tempo would pick a different one at a different cadence, and the comparison
    /// would be measuring the tempo rather than the outcome. Calling it again with
    /// another tempo is how a test flips the tempo mid-run at a point it chooses.
    ///
    /// While it is pinned, [`Clock::evaluate`] still records the signals it is
    /// given — so a later unpinned clock is not left with a blank history — and
    /// answers `None`, so no `tempo` ledger event is journalled for a pinned
    /// change. It is `#[doc(hidden)]` and named for what it is: nothing in the
    /// shipped lane calls it, and cut T's task must not.
    #[doc(hidden)]
    pub fn pin_tempo_for_tests(&mut self, tempo: Tempo) {
        self.tempo = tempo;
        self.pinned = Some(tempo);
    }

    /// **Doc 16 §3.1's tempo table, exactly.**
    ///
    /// Records whichever triggers `signals` reports at `now_ms`, then picks the
    /// tempo: `Engaged` while an engaged signal is younger than
    /// `engaged_hold_ms`, else `Alert` while an alert signal is younger than
    /// `alert_hold_ms`, else `Quiet`. "Younger than" is strict, so a hold of
    /// 10,000 ms is left at exactly 10,000 ms after the last signal.
    ///
    /// The two holds are independent, as the table's two "leave when" cells are:
    /// a match that only ever reports engaged signals falls straight from
    /// `Engaged` to `Quiet` when they stop, because nothing ever raised `Alert`.
    ///
    /// Answers `Some((from, to))` when the tempo changed, which cut A.2 turns
    /// into the one `tempo` ledger event doc 16 §3.1 asks for, and `None`
    /// otherwise.
    pub fn evaluate(
        &mut self,
        now_ms: i64,
        signals: &TempoSignals,
        cfg: &TempoConfig,
    ) -> Option<(Tempo, Tempo)> {
        if signals.engaged() {
            self.last_engaged_signal_ms = Some(now_ms);
        }
        if signals.alert() {
            self.last_alert_signal_ms = Some(now_ms);
        }
        let held = |last: Option<i64>, hold_ms: u32| -> bool {
            last.is_some_and(|at| now_ms.saturating_sub(at) < i64::from(hold_ms))
        };
        // A pinned clock keeps the tempo a test chose: the signals above are still
        // recorded, and nothing here moves.
        if let Some(pinned) = self.pinned {
            self.tempo = pinned;
            return None;
        }
        let target = if held(self.last_engaged_signal_ms, cfg.engaged_hold_ms) {
            Tempo::Engaged
        } else if held(self.last_alert_signal_ms, cfg.alert_hold_ms) {
            Tempo::Alert
        } else {
            Tempo::Quiet
        };
        if target == self.tempo {
            return None;
        }
        let from = self.tempo;
        self.tempo = target;
        self.tempo_since_ms = now_ms;
        Some((from, target))
    }

    /// **Turn a wall reading into a `dt` in arena milliseconds** (doc 16 §3.1
    /// "Clock and lag").
    ///
    /// The first call establishes the wall reference and yields `dt = 0`: arena
    /// time starts at zero and the opening pulse integrates nothing, which keeps
    /// the first pulse identical at every tempo.
    ///
    /// Afterwards `dt` is the wall interval clamped to
    /// `max_catch_up_steps × period` for the **current** tempo. Past the clamp
    /// the excess wall time is discarded rather than carried, and the answer's
    /// second half is `true`: cut A.2 turns that into one `lagged` ledger event.
    /// `dt` is never larger than the wall interval — the arena does not
    /// manufacture time — and a wall clock that went backwards yields `0`.
    ///
    /// A configured `max_catch_up_steps` of zero is read as one: a zero clamp
    /// would stop arena time outright, which is not a lag policy.
    pub fn advance(&mut self, wall_now_ms: i64, cfg: &ArenaConfig) -> (u32, bool) {
        let period = self.tempo.period_ms(&cfg.tempo);
        let steps = cfg.max_catch_up_steps.max(1);
        let cap = period.saturating_mul(steps);
        let Some(previous) = self.last_wall_ms else {
            self.last_wall_ms = Some(wall_now_ms);
            self.lagged = false;
            return (0, false);
        };
        self.last_wall_ms = Some(wall_now_ms);
        let elapsed = wall_now_ms.saturating_sub(previous).max(0);
        let dt = u32::try_from(elapsed).unwrap_or(u32::MAX).min(cap);
        let lagged = elapsed > i64::from(cap);
        self.lagged = lagged;
        (dt, lagged)
    }

    /// **The substep length** (doc 16 §3.1 "Rate independence"):
    /// `min(substep_max_ms, cell_cm × 1000 / max_speed_cms)`, floored at
    /// [`MIN_SUBSTEP_MS`].
    ///
    /// The quotient is the milliseconds the fastest unit in the match needs to
    /// cross one cell, so no unit ever advances more than one cell in a substep
    /// and step 4 of doc 16 §3.4 has at most one boundary to test.
    ///
    /// When no unit can move the length is `substep_max_ms` — there is no cell
    /// crossing to bound — still floored at [`MIN_SUBSTEP_MS`], because a
    /// configured `substep_max_ms` below the floor would otherwise subdivide a
    /// pulse without limit.
    #[must_use]
    pub fn substep_ms(cfg: &TempoConfig, cell_cm: u16, max_speed_cms: u32) -> u32 {
        let base = match u32::from(cell_cm)
            .saturating_mul(1_000)
            .checked_div(max_speed_cms)
        {
            Some(crossing) => cfg.substep_max_ms.min(crossing),
            // Nothing can move: there is no cell crossing to bound.
            None => cfg.substep_max_ms,
        };
        base.max(MIN_SUBSTEP_MS)
    }
}

/// The substep length, as a free function — the same answer as
/// [`Clock::substep_ms`], which is where it is documented.
///
/// Both spellings exist because the substep length is a property of the
/// configuration and the match, not of a clock instance: a caller that has the
/// tempo table and the fastest unit should not have to reach for a `Clock` to ask.
#[must_use]
pub fn substep_ms(cfg: &TempoConfig, cell_cm: u16, max_speed_cms: u32) -> u32 {
    Clock::substep_ms(cfg, cell_cm, max_speed_cms)
}

/// **The substeps one pulse is cut into**: every length is `substep_ms` except a
/// shorter last one, and they sum to exactly `dt_ms`.
///
/// So `substeps(1_000, 250)` is `[250, 250, 250, 250]` and `substeps(1_100, 250)`
/// is `[250, 250, 250, 250, 100]`. A `dt_ms` of zero is no substeps at all — the
/// opening pulse, and any pulse the clock clamped to nothing.
///
/// The remainder goes **last** rather than being spread, because the substep
/// boundaries are where due events are applied (doc 16 §3.1 phase 3c): an equal
/// prefix means the boundary timestamps of a pulse are the same whatever its
/// length, and only the final partial boundary moves.
///
/// `dt_ms` is expected to be the clamped value [`Clock::advance`] returned, which
/// at the shipped constants is at most `4 × 5,000 = 20,000` ms and so at most 400
/// substeps.
#[must_use]
pub fn substeps(dt_ms: u32, substep_ms: u32) -> Vec<u32> {
    if dt_ms == 0 {
        return Vec::new();
    }
    if substep_ms == 0 {
        // Not reachable through `substep_ms`, which floors at MIN_SUBSTEP_MS;
        // a zero length here is one whole substep rather than a division trap.
        return vec![dt_ms];
    }
    let whole = dt_ms / substep_ms;
    let remainder = dt_ms % substep_ms;
    let mut out: Vec<u32> = Vec::with_capacity(
        usize::try_from(whole.saturating_add(1))
            .unwrap_or(1)
            .min(1_024),
    );
    for _ in 0..whole {
        out.push(substep_ms);
    }
    if remainder != 0 {
        out.push(remainder);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engaged_signal() -> TempoSignals {
        TempoSignals {
            shot_fired: true,
            ..TempoSignals::quiet()
        }
    }

    fn alert_signal() -> TempoSignals {
        TempoSignals {
            contact_within_alert_range: true,
            ..TempoSignals::quiet()
        }
    }

    #[test]
    fn the_periods_are_the_configured_ones() {
        let cfg = TempoConfig::default();
        assert_eq!(Tempo::Quiet.period_ms(&cfg), 5_000);
        assert_eq!(Tempo::Alert.period_ms(&cfg), 1_000);
        assert_eq!(Tempo::Engaged.period_ms(&cfg), 500);
        assert_eq!(
            Tempo::ALL.map(|tempo| tempo.as_str()),
            ["quiet", "alert", "engaged"]
        );
    }

    #[test]
    fn a_fresh_clock_is_quiet_and_stays_quiet_without_a_signal() {
        let cfg = TempoConfig::default();
        let mut clock = Clock::new();
        assert_eq!(clock.tempo(), Tempo::Quiet);
        assert!(clock.evaluate(0, &TempoSignals::quiet(), &cfg).is_none());
        assert!(
            clock
                .evaluate(60_000, &TempoSignals::quiet(), &cfg)
                .is_none()
        );
        assert_eq!(clock.tempo(), Tempo::Quiet);
    }

    #[test]
    fn an_alert_signal_enters_alert_and_the_hold_leaves_it() {
        let cfg = TempoConfig::default();
        let mut clock = Clock::new();
        assert_eq!(
            clock.evaluate(1_000, &alert_signal(), &cfg),
            Some((Tempo::Quiet, Tempo::Alert))
        );
        assert_eq!(clock.tempo_since_ms(), 1_000);
        // One millisecond before the hold expires, still Alert.
        assert!(
            clock
                .evaluate(1_000 + 29_999, &TempoSignals::quiet(), &cfg)
                .is_none()
        );
        assert_eq!(clock.tempo(), Tempo::Alert);
        // At exactly the hold, back to Quiet.
        assert_eq!(
            clock.evaluate(1_000 + 30_000, &TempoSignals::quiet(), &cfg),
            Some((Tempo::Alert, Tempo::Quiet))
        );
    }

    #[test]
    fn an_engaged_signal_enters_engaged_and_the_hold_leaves_it() {
        let cfg = TempoConfig::default();
        let mut clock = Clock::new();
        assert_eq!(
            clock.evaluate(0, &engaged_signal(), &cfg),
            Some((Tempo::Quiet, Tempo::Engaged))
        );
        assert!(
            clock
                .evaluate(9_999, &TempoSignals::quiet(), &cfg)
                .is_none()
        );
        // The engaged hold expires; nothing ever raised Alert, so it falls to
        // Quiet in one step, which is what the table's two independent "leave
        // when" cells say.
        assert_eq!(
            clock.evaluate(10_000, &TempoSignals::quiet(), &cfg),
            Some((Tempo::Engaged, Tempo::Quiet))
        );
    }

    #[test]
    fn engaged_outranks_alert_and_falls_back_to_it_when_the_alert_hold_is_younger() {
        let cfg = TempoConfig::default();
        let mut clock = Clock::new();
        // Alert first, then a shot: Engaged wins while its hold is alive.
        assert_eq!(
            clock.evaluate(0, &alert_signal(), &cfg),
            Some((Tempo::Quiet, Tempo::Alert))
        );
        assert_eq!(
            clock.evaluate(5_000, &engaged_signal(), &cfg),
            Some((Tempo::Alert, Tempo::Engaged))
        );
        // The engaged hold expires at 15,000; the alert signal at 0 is still
        // inside its 30,000 hold, so the clock steps down to Alert, not Quiet.
        assert_eq!(
            clock.evaluate(15_000, &TempoSignals::quiet(), &cfg),
            Some((Tempo::Engaged, Tempo::Alert))
        );
        // And the alert hold expires at 30,000.
        assert_eq!(
            clock.evaluate(30_000, &TempoSignals::quiet(), &cfg),
            Some((Tempo::Alert, Tempo::Quiet))
        );
    }

    #[test]
    fn hysteresis_a_signal_that_stops_for_less_than_the_hold_never_leaves_the_tempo() {
        let cfg = TempoConfig::default();
        let mut clock = Clock::new();
        clock.evaluate(0, &engaged_signal(), &cfg);
        assert_eq!(clock.tempo(), Tempo::Engaged);
        let mut entered = 0_usize;
        // Sixty pulses of intermittent fire: a shot every 9,000 ms, which is
        // under the 10,000 ms hold, and silence in between.
        for pulse in 1..=60_i64 {
            let now = pulse * 3_000;
            let signals = if now % 9_000 == 0 {
                engaged_signal()
            } else {
                TempoSignals::quiet()
            };
            if clock.evaluate(now, &signals, &cfg).is_some() {
                entered += 1;
            }
        }
        assert_eq!(entered, 0, "the tempo never flapped");
        assert_eq!(clock.tempo(), Tempo::Engaged);
        assert_eq!(clock.tempo_since_ms(), 0);
    }

    #[test]
    fn advance_starts_from_the_first_reading_clamps_and_flags_lag() {
        let cfg = ArenaConfig::default();
        let mut clock = Clock::new();
        // The first reading is the reference, not an interval.
        assert_eq!(clock.advance(1_000_000, &cfg), (0, false));
        assert!(!clock.lagged());
        // Quiet: 5,000 ms period, 4 catch-up steps, so the clamp is 20,000 ms.
        assert_eq!(clock.advance(1_005_000, &cfg), (5_000, false));
        assert_eq!(clock.advance(1_025_000, &cfg), (20_000, false));
        // One millisecond past the clamp is lag, and dt is the clamp, never more.
        assert_eq!(clock.advance(1_045_001, &cfg), (20_000, true));
        assert!(clock.lagged());
        // The skipped wall time is discarded, not carried: the next interval is
        // measured from the reading just taken.
        assert_eq!(clock.advance(1_046_001, &cfg), (1_000, false));
        assert!(!clock.lagged());
        // A wall clock that went backwards yields nothing rather than a negative.
        assert_eq!(clock.advance(1_000_000, &cfg), (0, false));
    }

    #[test]
    fn the_clamp_follows_the_tempo_and_never_expands_dt() {
        let cfg = ArenaConfig::default();
        let mut clock = Clock::new();
        clock.evaluate(0, &engaged_signal(), &cfg.tempo);
        assert_eq!(clock.tempo(), Tempo::Engaged);
        assert_eq!(clock.advance(0, &cfg), (0, false));
        // Engaged: 500 ms period, so the clamp is 2,000 ms.
        assert_eq!(clock.advance(2_000, &cfg), (2_000, false));
        assert_eq!(clock.advance(9_000, &cfg), (2_000, true));
        // And a short interval is never rounded up to a period.
        assert_eq!(clock.advance(9_007, &cfg), (7, false));
    }

    #[test]
    fn a_zero_catch_up_configuration_is_read_as_one_period() {
        let cfg = ArenaConfig {
            max_catch_up_steps: 0,
            ..ArenaConfig::default()
        };
        let mut clock = Clock::new();
        assert_eq!(clock.advance(0, &cfg), (0, false));
        assert_eq!(clock.advance(100_000, &cfg), (5_000, true));
    }

    #[test]
    fn the_substep_length_is_the_cell_crossing_time_capped_and_floored() {
        let cfg = TempoConfig::default();
        // 1,000 cm cells at 1,000 cm/s is 1,000 ms to cross, so the 250 ms cap
        // wins.
        assert_eq!(Clock::substep_ms(&cfg, 1_000, 1_000), 250);
        // 1,000 cm cells at 5,000 cm/s is 200 ms, under the cap.
        assert_eq!(Clock::substep_ms(&cfg, 1_000, 5_000), 200);
        // Fast enough to cross a cell in under the floor: the floor wins.
        assert_eq!(Clock::substep_ms(&cfg, 1_000, 100_000), MIN_SUBSTEP_MS);
        // Nothing can move: the cap, with no crossing to bound.
        assert_eq!(Clock::substep_ms(&cfg, 1_000, 0), 250);
        // A configured cap under the floor is still floored.
        let tight = TempoConfig {
            substep_max_ms: 10,
            ..TempoConfig::default()
        };
        assert_eq!(Clock::substep_ms(&tight, 1_000, 1_000), MIN_SUBSTEP_MS);
        assert_eq!(Clock::substep_ms(&tight, 1_000, 0), MIN_SUBSTEP_MS);
    }

    #[test]
    fn the_substeps_sum_to_dt_and_none_exceeds_the_length() {
        assert_eq!(substeps(0, 250), Vec::<u32>::new());
        assert_eq!(substeps(1_000, 250), vec![250, 250, 250, 250]);
        assert_eq!(substeps(1_100, 250), vec![250, 250, 250, 250, 100]);
        assert_eq!(substeps(100, 250), vec![100]);
        assert_eq!(substeps(250, 250), vec![250]);
        // A zero length is one whole substep, never a division trap.
        assert_eq!(substeps(700, 0), vec![700]);
        // The two invariants A.2's step leans on, over every tempo's clamp.
        for dt in [1_u32, 7, 50, 137, 500, 1_000, 2_000, 4_000, 20_000] {
            for length in [MIN_SUBSTEP_MS, 100, 200, 250] {
                let cut = substeps(dt, length);
                assert_eq!(cut.iter().copied().sum::<u32>(), dt, "dt {dt} by {length}");
                assert!(
                    cut.iter().all(|piece| *piece <= length && *piece > 0),
                    "dt {dt} by {length} produced {cut:?}"
                );
                // Every piece but the last is exactly the length.
                if let Some((_, prefix)) = cut.split_last() {
                    assert!(prefix.iter().all(|piece| *piece == length));
                }
            }
        }
    }
}

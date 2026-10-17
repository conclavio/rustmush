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
//!
//! # The staff floor (16b cut I.3; 16c N10)
//!
//! `@arena/tempo` sets a **floor under the wake interval**: while it stands the
//! task wakes once per `max(period, floor)` of **wall** time
//! ([`Clock::wake_period_ms`]), and each wake still integrates at most
//! `max_catch_up_steps × period` of the **adaptive** tempo's own period — the
//! clamp in [`Clock::advance`] is untouched. A floor longer than that clamp is
//! slow motion (a 5,000 ms floor at `Engaged` moves 2,000 ms of arena time a
//! wake); a floor at or under a tempo's clamp only coarsens its wakes. It is
//! never a pause: every on-time wake moves arena time, and the rate independence
//! of doc 16 §3.1 is what makes a slowed match the same match (the tempo never
//! decides an outcome, so neither does a floor under it).
//!
//! Three rules keep it that way, and each has its reason:
//!
//! 1. **Lag is lateness past the floor.** A floored wake is late on purpose, so
//!    [`Clock::advance`] flags `elapsed − floor_then > cap`, where `floor_then`
//!    is the floor in force when the interval **began** (the floor at the
//!    previous wake, written on every reading including the first). A slowed
//!    match journals no `lagged`, and neither does the first pulse after a lift,
//!    whose interval began under the floor.
//! 2. **Every floored pulse ends on the substep lattice.** Under a floor the arena
//!    is behind the wall on purpose, and `Arena::next_due_ms`'s lattice rule — a
//!    wake target that is an absolute lattice point in arena time — cannot pull a
//!    wake back onto the lattice, because the wake is a wall interval. So
//!    `Arena::step` quantises the pulse's `dt` down to the lattice
//!    (`floor_to(now + dt, substep) − now`, possibly zero) for as long as a floor
//!    stands or stood at the previous wake ([`Clock::floored_now_or_then`]), and
//!    the timer's overshoot is discarded exactly as the clamp discards its excess.
//!    Without that, the pull-in pulse on a set and every wake below the clamp
//!    would add the OS overshoot to arena time and never return to the lattice,
//!    and due events would be applied against positions that depend on the wall
//!    moment staff typed the verb (D-16-2; 16b cut U.1). A quantised pulse of
//!    no time past the opening still runs the phase-4 drain, so a beam it fires
//!    lands at the millisecond it was fired, as in a pulse that ran a substep
//!    (`Arena::step`'s drain).
//! 3. **A floor is a multiple of `tempo.engaged_ms`**, from `engaged_ms` to
//!    [`MAX_TEMPO_FLOOR_MS`] (the verb refuses anything else), and `engaged_ms`
//!    is a multiple of the substep for every shipped configuration (16b cut M.2,
//!    16c Q24) — so every on-time floored wake moves exactly `min(cap, floor)` of
//!    arena time and the verb's figures are exactly true. What no configuration
//!    may do is make the substep LONGER than `engaged_ms`:
//!    `ArenaConfig::validate` refuses `substep_max_ms > engaged_ms`, since a
//!    wake that moves less than one substep would be cut back to nothing by
//!    rule 2 on every wake, and a floor would then be the pause it must never
//!    be. With that, every on-time floored wake moves at least one substep.
//!
//! The floor is a **view** of the wake cadence: `state_hash` folds none of it.
//! Its one ledger line, `tempo_floor`, journalled once per change
//! (`Clock::take_floor_change`), moves the ledger's sequence like every line.

use crate::config::{ArenaConfig, TempoConfig};
use serde::{Deserialize, Serialize};

/// The floor on a substep, milliseconds (doc 16 §3.1 "Rate independence").
///
/// It bounds the work one pulse can be subdivided into, and it is a floor rather
/// than advice because a zero-length substep cannot terminate.
pub const MIN_SUBSTEP_MS: u32 = 50;

/// **The longest tempo floor staff may set**, milliseconds (16b cut I.3; 16c
/// N10): ten seconds, because pilots' orders wait for the next wake under a
/// floor, and a longer wait is a pause by another name.
pub const MAX_TEMPO_FLOOR_MS: u32 = 10_000;

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
    /// A controller has entered `Approach` (**16b cut N.1** set it, off the
    /// reflex controller's rows until 17b cut M4.6 deleted them; a mind's
    /// cached doctrine state since 17b cut M4.3, `Arena::any_mind_approaching`,
    /// on a hull sealed on Standard Doctrine).
    pub ai_approaching: bool,
    /// A shot was fired this pulse (16b cut W.1).
    pub shot_fired: bool,
    /// An impact is scheduled (16b cut W.1: every accepted shot schedules one,
    /// so this and `shot_fired` move together).
    pub impact_scheduled: bool,
    /// Hostile units are within weapons range of each other.
    ///
    /// Cut W.1 left it `false` because a field-wide sweep was a per-pulse cost
    /// nobody had bounded. **16b cut N.1 sets it** off
    /// `crate::arena::state::Arena::hostile_in_weapons_range`, which is not a
    /// sweep: it reads the classified contacts each platform already holds against
    /// that platform's own `fight_inner_cm` (16b cut E.1: the band it would
    /// actually fight in, and not the longest gun it carries), so the cost is
    /// bounded by the contact store the sensor pass bounds and never by the size
    /// of the field.
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
    /// **The staff floor under the wake interval** (16b cut I.3), milliseconds,
    /// or `None`. Written by [`Clock::set_floor`] and nothing else.
    floor_ms: Option<u32>,
    /// **The floor that was in force at the previous wake** (16b cut I.3): what
    /// [`Clock::advance`] measures the interval it closes against, written by it
    /// on every reading, the first included — so a floor set before the first
    /// pulse is the floor the second pulse is measured against, and the first
    /// pulse after a lift is still measured against the lifted floor.
    floor_at_last_wake: Option<u32>,
    /// **Whether the floor changed since the last `tempo_floor` line** (16b cut
    /// I.3), taken by [`Clock::take_floor_change`].
    floor_changed: bool,
}

/// **What a snapshot carries of the clock** (16b cut AF.1; doc 16 §3.1 "Clock
/// and lag"): the tempo, when it was entered, and when each trigger was last
/// seen — the four fields [`Clock::evaluate`] decides the next tempo from, all
/// in arena milliseconds. Built by [`Clock::to_state`], read back by
/// [`Clock::from_state`], whose head says why nothing else is in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockState {
    tempo: Tempo,
    tempo_since_ms: i64,
    last_alert_signal_ms: Option<i64>,
    last_engaged_signal_ms: Option<i64>,
}

/// Complete synthetic replay timebase. Live restart still uses ClockState and
/// reanchors to its new process; replay restores the recorded wall/floor state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayClock {
    state: ClockState,
    last_wall_ms: Option<i64>,
    lagged: bool,
    pinned: Option<Tempo>,
    floor_ms: Option<u32>,
    floor_at_last_wake: Option<u32>,
    floor_changed: bool,
}

impl Clock {
    /// The captured manual-clock anchor; private forks preserve the whole clock.
    pub(crate) fn replay_wall_ms(&self) -> Option<i64> {
        self.last_wall_ms
    }

    /// Capture every clock field. A new field must explicitly choose its replay
    /// semantics here; an ordinary snapshot's wall reset is deliberately separate.
    #[must_use]
    pub fn replay_state(&self) -> ReplayClock {
        let Self {
            tempo,
            tempo_since_ms,
            last_alert_signal_ms,
            last_engaged_signal_ms,
            last_wall_ms,
            lagged,
            pinned,
            floor_ms,
            floor_at_last_wake,
            floor_changed,
        } = *self;
        ReplayClock {
            state: ClockState {
                tempo,
                tempo_since_ms,
                last_alert_signal_ms,
                last_engaged_signal_ms,
            },
            last_wall_ms,
            lagged,
            pinned,
            floor_ms,
            floor_at_last_wake,
            floor_changed,
        }
    }

    /// Restore a private replay's explicit timebase without a fresh wall anchor.
    #[must_use]
    pub fn from_replay(state: ReplayClock) -> Self {
        let ReplayClock {
            state,
            last_wall_ms,
            lagged,
            pinned,
            floor_ms,
            floor_at_last_wake,
            floor_changed,
        } = state;
        Self {
            tempo: state.tempo,
            tempo_since_ms: state.tempo_since_ms,
            last_alert_signal_ms: state.last_alert_signal_ms,
            last_engaged_signal_ms: state.last_engaged_signal_ms,
            last_wall_ms,
            lagged,
            pinned,
            floor_ms,
            floor_at_last_wake,
            floor_changed,
        }
    }

    /// A clock in [`Tempo::Quiet`] that has never seen a signal or a wall
    /// reading.
    #[must_use]
    pub fn new() -> Clock {
        Clock::default()
    }

    /// **The clock as a snapshot carries it** (16b cut AF.1): the four fields
    /// of [`ClockState`], all arena time.
    #[must_use]
    pub const fn to_state(&self) -> ClockState {
        ClockState {
            tempo: self.tempo,
            tempo_since_ms: self.tempo_since_ms,
            last_alert_signal_ms: self.last_alert_signal_ms,
            last_engaged_signal_ms: self.last_engaged_signal_ms,
        }
    }

    /// **A clock back from a snapshot** (16b cut AF.1): the tempo and its two
    /// hold clocks as they stood, and nothing else.
    ///
    /// What is **not** carried, and why:
    ///
    /// * `last_wall_ms` — the old process's wall reading, which means nothing to
    ///   this one; the resume anchors a fresh one ([`Clock::reanchor`]), so arena
    ///   time does not advance across the outage.
    /// * `lagged` — a reading of the last [`Clock::advance`], not state: the
    ///   first resumed pulse says its own.
    /// * `pinned` — a test lever ([`Clock::pin_tempo_for_tests`]); a test that
    ///   pins pins again, as its driver does every pulse.
    /// * the three floor fields — the staff tempo floor is a lever set for a
    ///   process that is gone (16c `Q-AF1` at its default): a resumed match runs
    ///   unfloored, `@arena/status` shows none, and no `tempo_floor` line is
    ///   journalled for the lift, because nobody lifted it.
    #[must_use]
    pub fn from_state(state: ClockState) -> Clock {
        // Every field by name, so a field added to the clock is a compile error
        // here until somebody decides whether a resume carries it.
        let ClockState {
            tempo,
            tempo_since_ms,
            last_alert_signal_ms,
            last_engaged_signal_ms,
        } = state;
        Clock {
            tempo,
            tempo_since_ms,
            last_alert_signal_ms,
            last_engaged_signal_ms,
            last_wall_ms: None,
            lagged: false,
            pinned: None,
            floor_ms: None,
            floor_at_last_wake: None,
            floor_changed: false,
        }
    }

    /// **Anchor the clock on this process's wall** (16b cut AF.1; doc 16 §3.1
    /// "Clock and lag"): the next [`Clock::advance`] measures its `dt` from
    /// `wall_now_ms`, reads as not lagged, and runs under no staff floor.
    ///
    /// Arena time does not advance across whatever came before — a restart's
    /// outage is not time the match lived through — so no `lagged` line is
    /// journalled for it; what moves arena time is only what this process's
    /// wall does from here.
    pub fn reanchor(&mut self, wall_now_ms: i64) {
        self.last_wall_ms = Some(wall_now_ms);
        self.lagged = false;
        self.floor_ms = None;
        self.floor_at_last_wake = None;
        self.floor_changed = false;
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

    /// **Set or lift the staff floor** (16b cut I.3; this module's head),
    /// milliseconds; `None` lifts it. The arena task applies it on arrival of an
    /// `ArenaMsg::TempoFloor`: it is not an input to `step`.
    ///
    /// A change is remembered for one `tempo_floor` line only when the value
    /// differs, so setting the floor that already stands journals nothing. The
    /// bounds are the verb's (`engaged_ms` to [`MAX_TEMPO_FLOOR_MS`] in steps of
    /// `engaged_ms`); the clock takes what it is given.
    pub fn set_floor(&mut self, floor_ms: Option<u32>) {
        if self.floor_ms != floor_ms {
            self.floor_ms = floor_ms;
            self.floor_changed = true;
        }
    }

    /// The staff floor standing now, milliseconds (16b cut I.3).
    #[must_use]
    pub const fn floor_ms(&self) -> Option<u32> {
        self.floor_ms
    }

    /// **The floor's change, once** (16b cut I.3): `Some(new value)` the first
    /// time it is asked after [`Clock::set_floor`] changed the floor, and `None`
    /// until the next change — what pulse phase 5 turns into one `tempo_floor`
    /// line.
    pub(crate) fn take_floor_change(&mut self) -> Option<Option<u32>> {
        if !self.floor_changed {
            return None;
        }
        self.floor_changed = false;
        Some(self.floor_ms)
    }

    /// **Whether this pulse is quantised to the substep lattice** (16b cut I.3):
    /// a floor stands now, or stood at the previous wake. `Arena::step` reads it
    /// **before** [`Clock::advance`] overwrites the previous wake's floor, so the
    /// first pulse after a lift — whose interval began under the floor and whose
    /// `dt` carries a floored wake's overshoot — is quantised too.
    #[must_use]
    pub const fn floored_now_or_then(&self) -> bool {
        self.floor_ms.is_some() || self.floor_at_last_wake.is_some()
    }

    /// **The wall interval the task wakes on** (16b cut I.3):
    /// `max(period, floor)` for the current tempo, the period alone with no
    /// floor. Under a floor `Arena::next_due_ms` answers the wall reading plus
    /// this; the catch-up clamp in [`Clock::advance`] stays on the period.
    #[must_use]
    pub fn wake_period_ms(&self, cfg: &TempoConfig) -> u32 {
        self.tempo.period_ms(cfg).max(self.floor_ms.unwrap_or(0))
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
    ///
    /// **16b cut I.3**: the clamp stays on the adaptive tempo's own period, and
    /// lag is lateness past the staff floor that was in force when the interval
    /// **began** — `elapsed − floor_at_last_wake > cap` — so a slowed match is
    /// not lagging, and neither is the first pulse after a lift. The floor in
    /// force now becomes the previous wake's floor on every reading, the first
    /// one included, so a floor set before the opening pulse is what the second
    /// pulse is measured against. Quantising a floored pulse's `dt` to the
    /// substep lattice is `Arena::step`'s (this module's head says why).
    pub fn advance(&mut self, wall_now_ms: i64, cfg: &ArenaConfig) -> (u32, bool) {
        let period = self.tempo.period_ms(&cfg.tempo);
        let steps = cfg.max_catch_up_steps.max(1);
        let cap = period.saturating_mul(steps);
        let floor_then = self.floor_at_last_wake;
        self.floor_at_last_wake = self.floor_ms;
        let Some(previous) = self.last_wall_ms else {
            self.last_wall_ms = Some(wall_now_ms);
            self.lagged = false;
            return (0, false);
        };
        self.last_wall_ms = Some(wall_now_ms);
        let elapsed = wall_now_ms.saturating_sub(previous).max(0);
        let dt = u32::try_from(elapsed).unwrap_or(u32::MAX).min(cap);
        let lagged = elapsed.saturating_sub(i64::from(floor_then.unwrap_or(0))) > i64::from(cap);
        self.lagged = lagged;
        (dt, lagged)
    }

    /// **The substep length** (doc 16 §3.1 "Rate independence"):
    /// `min(substep_max_ms, (cell_cm - 1) × 1000 / max_speed_cms)`, floored at
    /// [`MIN_SUBSTEP_MS`].
    ///
    /// The quotient is the milliseconds the fastest unit in the match needs to
    /// cross one cell **less a centimetre**, so **one substep's travel on either
    /// axis — the carry it banked last substep included, because that carry is
    /// inside the same division — is at most `cell_cm - 1`**. No unit therefore
    /// advances more than one cell in a substep and step 4 of doc 16 §3.4 has at
    /// most one boundary to test.
    ///
    /// **16b cut Q.6 took that centimetre back, for margin and not for a repair.**
    /// The bound was `cell_cm × 1000 / max_speed_cms`, under which travel could be
    /// exactly `cell_cm` — and *that is still one cell*, because
    /// [`crate::arena::state::ArenaTerrain::cell_at`] divides: adding exactly
    /// `cell_cm` to a coordinate raises its cell index by exactly one, from a pose
    /// one centimetre short of the edge as much as from anywhere else. Nor could
    /// the carry [`crate::arena::state::MotionCarry`] added push it past that: one
    /// axis of a substep is `div_euclid(speed × dt × fixed + carry, D)` with
    /// `carry < D` and `|fixed| ≤ SIN_ONE`, so under the old bound the quotient was
    /// at most `cell_cm` and under this one it is at most `cell_cm - 1`. The
    /// tightening therefore buys a **strict** margin: with the remainder now inside
    /// that quotient, the old bound was tight with nothing at all left over, and
    /// every later change to the travel arithmetic would have had to re-derive that
    /// it was still exactly tight. The stock tank (1,500 cm/s on 1,000 cm cells) is
    /// bound by the 250 ms cap either way, so no shipped partition moves.
    ///
    /// When no unit can move the length is `substep_max_ms` — there is no cell
    /// crossing to bound — still floored at [`MIN_SUBSTEP_MS`], because a
    /// configured `substep_max_ms` below the floor would otherwise subdivide a
    /// pulse without limit. The floor is the one thing that can still beat the
    /// bound (a unit faster than 19,980 cm/s on 10 m cells), and since **16b cut
    /// M.4** (16c `B-57`) a match whose ceiling does is refused at the door
    /// ([`crate::arena::state::ArenaError::SubstepBound`]); motion's own one-cell
    /// guard stays behind it as belt and braces.
    ///
    /// **`max_speed_cms` is the terrain factor's ceiling** (16b cut M.4, 16c
    /// `T-15`): [`crate::arena::state::Arena::max_speed_cms`], the largest
    /// `max_speed × 100 / min(100, min_cost[row])` over the units, each on its
    /// own cost row (16b cut AE.1, 16c `B-58`), because on a cell cheaper than
    /// 100 motion's terrain cap sits above the authored speed. No unit reaches
    /// it (the order is clamped to the authored speed in `apply_intent`), so the
    /// scale is a conservative margin that keeps the bound true of the cap
    /// itself. On the shipped ground the tracked and legged rows' cheapest
    /// passable price is 100 and the wheeled row's is the road's 80, so a
    /// 3,000 cm/s scout car lifts the ceiling to 3,750 — and the substep stays
    /// 250 ms (`999,000 / 3,750` is 266).
    #[must_use]
    pub fn substep_ms(cfg: &TempoConfig, cell_cm: u16, max_speed_cms: u32) -> u32 {
        let base = match u32::from(cell_cm)
            .saturating_sub(1)
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

    // -----------------------------------------------------------------------
    // 16b cut I.3 — the staff floor
    // -----------------------------------------------------------------------

    /// **16b cut I.3**: the wake interval is `max(period, floor)` for the
    /// current tempo, and the period alone without a floor.
    #[test]
    fn i3_the_wake_period_is_the_period_or_the_floor_whichever_is_longer() {
        let cfg = TempoConfig::default();
        let mut clock = Clock::new();
        assert_eq!(clock.floor_ms(), None);
        assert_eq!(clock.wake_period_ms(&cfg), 5_000, "Quiet, no floor");
        clock.pin_tempo_for_tests(Tempo::Engaged);
        assert_eq!(clock.wake_period_ms(&cfg), 500, "Engaged, no floor");
        clock.set_floor(Some(5_000));
        assert_eq!(clock.floor_ms(), Some(5_000));
        assert_eq!(clock.wake_period_ms(&cfg), 5_000, "Engaged under 5,000");
        clock.pin_tempo_for_tests(Tempo::Quiet);
        clock.set_floor(Some(1_000));
        assert_eq!(
            clock.wake_period_ms(&cfg),
            5_000,
            "a floor under the period coarsens nothing"
        );
        clock.set_floor(Some(MAX_TEMPO_FLOOR_MS));
        assert_eq!(clock.wake_period_ms(&cfg), 10_000);
        clock.set_floor(None);
        assert_eq!(clock.wake_period_ms(&cfg), 5_000);
    }

    /// **16b cut I.3**: lag is lateness past the floor in force when the
    /// interval began — `floor + cap` is not lag and `floor + cap + 1` is;
    /// without a floor `cap + 1` is, as it always was.
    #[test]
    fn i3_lag_is_measured_past_the_floor_the_interval_began_under() {
        let cfg = ArenaConfig::default();
        let mut clock = Clock::new();
        clock.pin_tempo_for_tests(Tempo::Engaged);
        clock.set_floor(Some(5_000));
        assert_eq!(clock.advance(0, &cfg), (0, false));
        // Engaged: a 2,000 ms clamp. 5,000 + 2,000 is not lag.
        assert_eq!(clock.advance(7_000, &cfg), (2_000, false));
        // 5,000 + 2,000 + 1 is.
        assert_eq!(clock.advance(14_001, &cfg), (2_000, true));
        assert!(clock.lagged());
        // An on-time floored wake is 5,000 ms and never lag.
        assert_eq!(clock.advance(19_001, &cfg), (2_000, false));
        // Without a floor, the clamp alone, as before this cut: the lift is
        // measured against the floor it began under, and the next interval is
        // not.
        clock.set_floor(None);
        assert_eq!(
            clock.advance(26_001, &cfg),
            (2_000, false),
            "the first interval after a lift began under the floor"
        );
        assert_eq!(clock.advance(28_002, &cfg), (2_000, true), "2,001 is lag");
        assert_eq!(clock.advance(30_002, &cfg), (2_000, false));
    }

    /// **16b cut I.3**: a floor set before the first reading is what the second
    /// interval is measured against — the first reading's early return records
    /// it too.
    #[test]
    fn i3_a_floor_set_before_the_first_reading_measures_the_second_interval() {
        let cfg = ArenaConfig::default();
        let mut clock = Clock::new();
        clock.pin_tempo_for_tests(Tempo::Engaged);
        clock.set_floor(Some(5_000));
        assert_eq!(clock.advance(1_000_000, &cfg), (0, false));
        assert_eq!(
            clock.advance(1_005_000, &cfg),
            (2_000, false),
            "a slowed match is not lagging"
        );
        assert!(!clock.lagged());
    }

    /// **16b cut I.3**: `set_floor` is idempotent, and `take_floor_change`
    /// answers once per change.
    #[test]
    fn i3_a_floor_change_is_taken_once_and_a_repeat_is_no_change() {
        let mut clock = Clock::new();
        assert_eq!(clock.take_floor_change(), None);
        clock.set_floor(None);
        assert_eq!(clock.take_floor_change(), None, "no floor to no floor");
        clock.set_floor(Some(5_000));
        clock.set_floor(Some(5_000));
        assert_eq!(clock.take_floor_change(), Some(Some(5_000)));
        assert_eq!(clock.take_floor_change(), None, "once");
        clock.set_floor(Some(5_000));
        assert_eq!(clock.take_floor_change(), None, "the same floor again");
        // Two changes before a take are one line with the newest value.
        clock.set_floor(Some(1_000));
        clock.set_floor(Some(2_000));
        assert_eq!(clock.take_floor_change(), Some(Some(2_000)));
        clock.set_floor(None);
        assert_eq!(clock.take_floor_change(), Some(None), "the lift");
        assert_eq!(clock.take_floor_change(), None);
    }

    /// **16b cut I.3**: a pulse is quantised while a floor stands and on the
    /// one pulse after a lift, and not after that.
    #[test]
    fn i3_the_quantisation_runs_under_a_floor_and_one_pulse_past_it() {
        let cfg = ArenaConfig::default();
        let mut clock = Clock::new();
        assert!(!clock.floored_now_or_then());
        clock.set_floor(Some(1_000));
        assert!(clock.floored_now_or_then());
        clock.advance(0, &cfg);
        clock.set_floor(None);
        assert!(
            clock.floored_now_or_then(),
            "the interval after a lift began under the floor"
        );
        clock.advance(1_000, &cfg);
        assert!(!clock.floored_now_or_then());
    }

    #[test]
    fn the_substep_length_is_the_cell_crossing_time_capped_and_floored() {
        let cfg = TempoConfig::default();
        // 1,000 cm cells at 1,000 cm/s is 1,000 ms to cross, so the 250 ms cap
        // wins.
        assert_eq!(Clock::substep_ms(&cfg, 1_000, 1_000), 250);
        // 1,000 cm cells at 5,000 cm/s crosses the cell in 200 ms; the bound is
        // one centimetre short of that cell, so it is 199 — and 199 ms of travel
        // at 5,000 cm/s is 995 cm. The motion carry (16b cut Q.6) is added
        // *inside* the same division, so it buys no extra centimetre on top: the
        // whole substep's travel is at most `cell_cm - 1`, 999 cm.
        assert_eq!(Clock::substep_ms(&cfg, 1_000, 5_000), 199);
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

    /// **16b cut AF.1**: a clock back from its state keeps the tempo and the two
    /// hold clocks and nothing of the old wall, and `reanchor` makes the next
    /// `advance` measure from the new anchor — not lagged, however long the
    /// outage was, and under no floor, whatever the old process had set.
    #[test]
    fn af1_a_reanchored_clock_measures_from_the_anchor_and_carries_no_floor() {
        let cfg = ArenaConfig::default();
        let mut old = Clock::new();
        let _ = old.evaluate(3_000, &alert_signal(), &cfg.tempo);
        let _ = old.evaluate(4_000, &engaged_signal(), &cfg.tempo);
        old.set_floor(Some(5_000));
        let _ = old.advance(10_000, &cfg);
        let _ = old.advance(15_000, &cfg);
        let state = old.to_state();

        let mut resumed = Clock::from_state(state);
        assert_eq!(resumed.to_state(), state, "the four fields come back");
        assert_eq!(resumed.tempo(), old.tempo());
        assert_eq!(resumed.tempo_since_ms(), old.tempo_since_ms());
        assert_eq!(resumed.floor_ms(), None, "the staff floor is not carried");
        assert!(!resumed.floored_now_or_then());
        // A floor set on the resumed clock before the anchor is lifted by it too,
        // and journals no line: nobody lifted it.
        resumed.set_floor(Some(5_000));
        resumed.reanchor(9_000_000);
        assert_eq!(resumed.floor_ms(), None);
        assert_eq!(resumed.take_floor_change(), None);
        assert!(!resumed.lagged());
        // The next reading is measured from the anchor: 500 ms of this process's
        // wall is 500 ms of arena time, and the hours of outage before it are
        // none of it.
        let (dt, lagged) = resumed.advance(9_000_500, &cfg);
        assert_eq!((dt, lagged), (500, false));
        assert!(!resumed.lagged());
    }
}

#[cfg(test)]
mod replay_tests {
    use super::*;

    #[test]
    fn replay_keeps_all_clock_fields_and_live_restart_still_resets_them() {
        let original = Clock {
            tempo: Tempo::Alert,
            tempo_since_ms: 12_000,
            last_alert_signal_ms: Some(15_000),
            last_engaged_signal_ms: Some(9_000),
            last_wall_ms: Some(21_003),
            lagged: true,
            pinned: Some(Tempo::Quiet),
            floor_ms: Some(5_000),
            floor_at_last_wake: Some(10_000),
            floor_changed: true,
        };
        let document = serde_json::to_vec(&original.replay_state()).expect("encode");
        let recorded = serde_json::from_slice(&document).expect("decode");
        assert_eq!(Clock::from_replay(recorded), original);
        let restarted = Clock::from_state(original.to_state());
        assert_eq!(restarted.last_wall_ms, None);
        assert_eq!(restarted.floor_ms, None);
        assert_ne!(restarted, original);
    }
}

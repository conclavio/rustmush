//! Clocks, deadlines, schedules, and the bounded catch-up arithmetic (E4a, §3.1).
//!
//! Everything here is **pure**: no world, no wall clock of its own. The two live
//! instants ([`Clocks::real`], [`Clocks::ic`]) are handed in by the caller —
//! [`crate::ValidateEnv`] carries them for the engine, and the E4b1 sweep reads them
//! from `World::now()` / `World::game_now_unix()`.
//!
//! Three rules make the arithmetic safe (D-E4-2, D-E4-3, review points C8/C9):
//!
//! - **A deadline names its clock.** [`EconomicDeadline`] and [`EconomicSchedule`]
//!   carry the [`EconomicClock`] their `at`/`next_due` is expressed on, so two
//!   stamps are only ever compared on the same clock — doc 09 §2.5's rule, and the
//!   reason a grace deadline can never be checked against wall time by accident.
//! - **A clock that moved backwards yields zero.** [`due_periods`] returns `0` for
//!   `now < next_due` rather than an underflowed or negative count, so an epoch
//!   change or a rewound IC clock charges nothing.
//! - **Catch-up is bounded at creation.** [`EconomicSchedule::new`] clamps
//!   `maximum_catch_up` to [`MAX_CATCH_UP_CAP`] whatever the caller asked for, so no
//!   stored record can ever authorize an unbounded backlog sweep.
//!
//! Catch-up is always computed from `next_due` and the clock, **never** from a tick
//! count (D-E4-3): the core scheduler is in-RAM and re-armed from scratch at boot,
//! so tick counts do not survive a restart and downtime would silently vanish.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::ids::CadenceId;
use crate::snapshot::EconomySnapshot;

/// The largest catch-up any schedule may declare (§9). A schedule asking for more is
/// clamped at construction — the stored record can never authorize an unbounded
/// backlog (review point C8).
pub const MAX_CATCH_UP_CAP: u8 = 8;
/// The default catch-up for a schedule whose creator did not choose one (§9).
pub const DEFAULT_CATCH_UP: u8 = 3;
/// The economy sweep's wall-clock period (§9, owner-confirmed). E4b1 arms the tick
/// with it; E4a only needs it to reason about what a "no sweep ran" gap looks like.
pub const SWEEP_PERIOD_SECS: i64 = 30;
/// How large a gap between two recorded sweeps means **no sweep ran** across it —
/// the [`crate::classify_forgiveness`] evidence threshold when the gap does *not*
/// span a boot.
///
/// **Two** sweep periods (E4b1 verify fold, S3). Four was too generous: a 60–120 s
/// outage — a restart, a stop-the-world GC, a hypervisor pause — produced a gap in
/// which no sweep ran and yet classified as `Overload`, the one cause that escalates
/// on repeat, so two ordinary restarts inside the escalation window paged a staffer
/// about a starved post budget that never existed. The boot stamp
/// (`meta.last_boot_real`) is the primary evidence now; this threshold only has to
/// catch the outages no boot marker explains, and one missed fire plus slack is
/// enough for that.
pub const SWEEP_GAP_DOWNTIME_SECS: i64 = 2 * SWEEP_PERIOD_SECS;

/// Which clock a deadline or schedule is expressed on (D-E4-2).
///
/// The pre-K4 adapter shape over doc 09 §2.5's `ClockRef`: `RealUtc ≙ Utc`,
/// `WorldIc ≙ WorldIc { generation }`, `Cadence(id) ≙ Registered { owner: economy,
/// id, generation }`, and `Manual` is a cadence whose source is an explicit act.
/// `Registered { provider }` stays deferred — no clock-provider seam exists and no
/// consumer needs one before E7 (§1.1).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EconomicClock {
    /// Wall-clock unix seconds.
    #[default]
    RealUtc,
    /// The world's in-character clock (`World::game_now_unix`).
    WorldIc,
    /// A position advanced only by an explicit act, carried by the record itself.
    Manual,
    /// An economy-owned [`CadenceClock`]; "now" is its `position` (a period index).
    Cadence(CadenceId),
}

/// An absolute instant **on a named clock**. Never a bare `i64`: comparing a grace
/// deadline against the wrong clock is the bug this type exists to make unwritable.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EconomicDeadline {
    /// The clock `at` is expressed on.
    pub clock: EconomicClock,
    /// The instant, on that clock.
    pub at: i64,
}

impl EconomicDeadline {
    /// A deadline on the real clock.
    #[must_use]
    pub const fn real(at: i64) -> EconomicDeadline {
        EconomicDeadline { clock: EconomicClock::RealUtc, at }
    }
}

/// A recurring (or one-shot) due-date, on one clock, with bounded catch-up.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EconomicSchedule {
    /// The clock `next_due` is expressed on.
    pub clock: EconomicClock,
    /// The next due instant, absolute on `clock`.
    pub next_due: i64,
    /// The period length on `clock`; `None` is a one-shot.
    pub cadence: Option<i64>,
    /// How many periods one sweep may settle, ≤ [`MAX_CATCH_UP_CAP`] (clamped by
    /// [`EconomicSchedule::new`] whatever the caller asked for).
    pub maximum_catch_up: u8,
    /// The [`EconomicClock::Manual`] position this record carries — advanced only by
    /// an explicit act, ignored on every other clock.
    pub manual_now: i64,
}

impl EconomicSchedule {
    /// Build a schedule, **clamping** `maximum_catch_up` to [`MAX_CATCH_UP_CAP`] and
    /// to at least 1 (a zero catch-up would silently never settle anything).
    #[must_use]
    pub fn new(
        clock: EconomicClock,
        next_due: i64,
        cadence: Option<i64>,
        maximum_catch_up: u8,
    ) -> EconomicSchedule {
        EconomicSchedule {
            clock,
            next_due,
            cadence,
            maximum_catch_up: maximum_catch_up.clamp(1, MAX_CATCH_UP_CAP),
            manual_now: 0,
        }
    }
    /// A recurring schedule on the real clock at the default catch-up.
    #[must_use]
    pub fn every(next_due: i64, cadence: i64) -> EconomicSchedule {
        EconomicSchedule::new(EconomicClock::RealUtc, next_due, Some(cadence), DEFAULT_CATCH_UP)
    }
}

/// How many periods of `s` are due at `now` **on `s`'s own clock**.
///
/// `0` when `now < next_due` — a clock that moved backwards (an epoch change, a
/// rewound IC clock, a re-pointed cadence) never yields a garbage count (C9). A
/// one-shot (`cadence: None`) is at most one period. A non-positive cadence, which
/// no constructor should produce but a corrupt doc could, is treated as a one-shot
/// rather than divided by.
#[must_use]
pub fn due_periods(s: &EconomicSchedule, now: i64) -> u32 {
    if now < s.next_due {
        return 0;
    }
    let Some(cadence) = s.cadence else { return 1 };
    if cadence <= 0 {
        return 1;
    }
    // i128 so `now - next_due` cannot overflow at the i64 extremes.
    let elapsed = i128::from(now) - i128::from(s.next_due);
    let n = elapsed / i128::from(cadence) + 1;
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The schedule after settling (or forgiving) `n` periods.
///
/// A one-shot advanced by ≥1 period is **done**: its `next_due` becomes `i64::MAX`,
/// so [`due_periods`] answers `0` forever after. A saturating add is deliberate — a
/// schedule pushed past `i64::MAX` is likewise done, never wrapped into the past.
#[must_use]
pub fn advance(s: &EconomicSchedule, n: u32) -> EconomicSchedule {
    let mut out = *s;
    if n == 0 {
        return out;
    }
    match s.cadence {
        None => out.next_due = i64::MAX,
        Some(cadence) if cadence > 0 => {
            let step = i128::from(cadence) * i128::from(n);
            let next = i128::from(s.next_due).saturating_add(step);
            out.next_due = i64::try_from(next).unwrap_or(i64::MAX);
        }
        Some(_) => out.next_due = i64::MAX,
    }
    out
}

/// Re-anchor `s` to the first due instant **strictly after** `now`, without
/// counting how many periods that skipped.
///
/// The escape hatch for a schedule whose period index can no longer be advanced —
/// `periods_settled + due` would overflow `u32` (E4a verify fold, finding 13). Every
/// pending period is forgiven wholesale and the schedule starts again from `now`;
/// nothing is charged and no key index moves, which is the only way to keep the
/// deterministic `obligation/{id}/{period}` keys unique. A one-shot (or a corrupt
/// non-positive cadence) is simply done.
#[must_use]
pub fn reanchor(s: &EconomicSchedule, now: i64) -> EconomicSchedule {
    let mut out = *s;
    let Some(cadence) = s.cadence.filter(|c| *c > 0) else {
        out.next_due = i64::MAX;
        return out;
    };
    if now < s.next_due {
        return out;
    }
    // i128 throughout: `now - next_due` spans the i64 range at the extremes.
    let elapsed = i128::from(now) - i128::from(s.next_due);
    let periods = elapsed / i128::from(cadence) + 1;
    let next = i128::from(s.next_due).saturating_add(periods.saturating_mul(i128::from(cadence)));
    out.next_due = i64::try_from(next).unwrap_or(i64::MAX);
    out
}

// ---- cadence clocks (D-E4-2) ----------------------------------------------

/// What a [`CadenceClock`] advances from.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CadenceSource {
    /// Wall-clock seconds.
    RealUtc,
    /// The world's IC clock.
    WorldIc,
    /// Only an explicit act (`@econ/cadence advance <id>`).
    Manual,
}

/// A named campaign/domain clock the economy owns (E5's owner answer 7).
///
/// Staff/config-defined, advanced by the sweep from its `source` with bounded
/// catch-up (the excess is recorded as forgiven periods), or by an explicit act when
/// `source` is [`CadenceSource::Manual`]. A schedule on a cadence expresses its
/// `next_due` as a **period index**, so housing, careers, rent and upkeep all run on
/// the campaign's own beat — never on an IC:OOC ratio or a literal calendar.
///
/// The `cadence:{id}` container doc is E4b1's; this is the record and its pure
/// advance math.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CadenceClock {
    /// Identity (never reused).
    pub id: CadenceId,
    /// Display/config name.
    pub name: SmolStr,
    /// What it advances from.
    pub source: CadenceSource,
    /// The period length in `source` units (> 0; a non-positive period never
    /// advances rather than dividing by zero).
    pub period: i64,
    /// How many periods one advance may cover; the excess is forgiven.
    pub catch_up_cap: u8,
    /// The current period index — what [`EconomicClock::Cadence`] resolves to.
    pub position: u64,
    /// The `source` instant `position` was last advanced from (aligned to a period
    /// boundary), so elapsed periods are computed from the record, not from ticks.
    pub advanced_at: i64,
    /// Staleness stamp.
    pub revision: u64,
}

/// How many whole periods have elapsed on `c` at `source_now`.
///
/// `0` when the source moved backwards or the period is non-positive — the same
/// fail-quiet rule as [`due_periods`].
#[must_use]
pub fn cadence_elapsed(c: &CadenceClock, source_now: i64) -> u64 {
    if c.period <= 0 || source_now <= c.advanced_at {
        return 0;
    }
    let elapsed = i128::from(source_now) - i128::from(c.advanced_at);
    u64::try_from(elapsed / i128::from(c.period)).unwrap_or(u64::MAX)
}

/// Advance `c` to `source_now` under its own catch-up cap.
///
/// Returns the advanced record and how many periods were **forgiven** (elapsed
/// beyond the cap). `advanced_at` moves to the last whole period boundary either
/// way, so a forgiven period is genuinely skipped rather than replayed at the next
/// advance — doc 08 §2 forbids charging a downtime backlog.
///
/// A [`CadenceSource::Manual`] cadence is returned **unchanged**: its position is a
/// human's to move ([`advance_cadence_manual`]), and reading a wall or IC instant as
/// its source would let the sweep advance a clock nobody asked to advance (E4a
/// verify fold, finding 12).
#[must_use]
pub fn advance_cadence(c: &CadenceClock, source_now: i64) -> (CadenceClock, u64) {
    if c.source == CadenceSource::Manual {
        return (c.clone(), 0);
    }
    let elapsed = cadence_elapsed(c, source_now);
    if elapsed == 0 {
        return (c.clone(), 0);
    }
    // Clamped at BOTH ends, exactly like `EconomicSchedule::new`: a stored
    // `catch_up_cap` of 0 would take no periods at all while `advanced_at` still
    // moved, silently swallowing every period forever.
    let cap = u64::from(c.catch_up_cap.clamp(1, MAX_CATCH_UP_CAP));
    let taken = elapsed.min(cap);
    let mut out = c.clone();
    out.position = out.position.saturating_add(taken);
    let moved = i128::from(c.advanced_at) + i128::from(c.period) * i128::from(elapsed);
    out.advanced_at = i64::try_from(moved).unwrap_or(i64::MAX);
    out.revision += 1;
    (out, elapsed - taken)
}

/// Advance a [`CadenceSource::Manual`] cadence by `n` periods (an explicit act; no
/// catch-up cap applies, because a human asked for exactly this many).
#[must_use]
pub fn advance_cadence_manual(c: &CadenceClock, n: u64) -> CadenceClock {
    let mut out = c.clone();
    out.position = out.position.saturating_add(n);
    out.revision += 1;
    out
}

// ---- the resolver ---------------------------------------------------------

/// The two live instants, resolved onto whichever clock a record names.
///
/// `Manual` comes from the record itself (a schedule's `manual_now`), and a
/// [`EconomicClock::Cadence`]'s position comes from the snapshot. A cadence whose
/// record is missing resolves to `None`, and every consumer treats that as **not
/// yet** — a missing clock can never make a deadline pass.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Clocks {
    /// Wall-clock unix seconds.
    pub real: i64,
    /// The world's IC clock, unix seconds.
    pub ic: i64,
}

impl Clocks {
    /// Build a resolver.
    #[must_use]
    pub const fn new(real: i64, ic: i64) -> Clocks {
        Clocks { real, ic }
    }
    /// "Now" on `clock`, with `manual_now` supplied by the record that named it.
    /// `None` for a cadence with no record in the snapshot.
    #[must_use]
    pub fn now_on(
        &self,
        clock: EconomicClock,
        snapshot: &EconomySnapshot,
        manual_now: i64,
    ) -> Option<i64> {
        match clock {
            EconomicClock::RealUtc => Some(self.real),
            EconomicClock::WorldIc => Some(self.ic),
            EconomicClock::Manual => Some(manual_now),
            EconomicClock::Cadence(id) => snapshot
                .cadence(id)
                .map(|c| i64::try_from(c.position).unwrap_or(i64::MAX)),
        }
    }
    /// Whether `deadline` has passed. A cadence with no record has **not** passed
    /// (fail-closed: an `Expire` on an unresolvable clock is refused).
    #[must_use]
    pub fn passed(
        &self,
        deadline: &EconomicDeadline,
        snapshot: &EconomySnapshot,
        manual_now: i64,
    ) -> bool {
        self.now_on(deadline.clock, snapshot, manual_now)
            .is_some_and(|now| now >= deadline.at)
    }
    /// How many periods of `s` are due, resolved on `s`'s own clock. `0` when the
    /// clock cannot be resolved.
    #[must_use]
    pub fn due_periods(&self, s: &EconomicSchedule, snapshot: &EconomySnapshot) -> u32 {
        self.now_on(s.clock, snapshot, s.manual_now)
            .map_or(0, |now| due_periods(s, now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maximum_catch_up_is_clamped_at_construction() {
        assert_eq!(EconomicSchedule::new(EconomicClock::RealUtc, 0, Some(10), 200).maximum_catch_up, MAX_CATCH_UP_CAP);
        assert_eq!(EconomicSchedule::new(EconomicClock::RealUtc, 0, Some(10), 0).maximum_catch_up, 1);
    }

    #[test]
    fn a_backwards_clock_yields_zero_periods() {
        let s = EconomicSchedule::every(1_000, 100);
        assert_eq!(due_periods(&s, 999), 0);
        assert_eq!(due_periods(&s, i64::MIN), 0);
        assert_eq!(due_periods(&s, 1_000), 1);
        assert_eq!(due_periods(&s, 1_250), 3);
    }

    #[test]
    fn a_one_shot_is_at_most_one_period_and_never_recurs() {
        let s = EconomicSchedule::new(EconomicClock::RealUtc, 100, None, 3);
        assert_eq!(due_periods(&s, 10_000), 1);
        let done = advance(&s, 1);
        assert_eq!(done.next_due, i64::MAX);
        assert_eq!(due_periods(&done, i64::MAX - 1), 0);
    }

    #[test]
    fn cadence_advance_forgives_the_excess_and_never_replays_it() {
        let c = CadenceClock {
            id: CadenceId(1),
            name: "season".into(),
            source: CadenceSource::RealUtc,
            period: 100,
            catch_up_cap: 3,
            position: 5,
            advanced_at: 1_000,
            revision: 1,
        };
        let (next, forgiven) = advance_cadence(&c, 1_000 + 100 * 10);
        assert_eq!(forgiven, 7);
        assert_eq!(next.position, 8);
        assert_eq!(next.advanced_at, 2_000);
        // The forgiven periods are gone: advancing again at the same instant is a no-op.
        let (again, forgiven2) = advance_cadence(&next, 2_000);
        assert_eq!(forgiven2, 0);
        assert_eq!(again.position, 8);
    }
}

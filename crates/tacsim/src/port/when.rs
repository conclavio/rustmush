//! **When a queued departure may leave** (doc 18 cut IC11; doc 18 §4.4 "The
//! queued-orders model": "a daily UTC time or `+Nh`", "the real-time … departure
//! clock (UTC)"; lead rulings L8 and L9 of the IC11 brief).
//!
//! Pure, and total: every function answers for every input, with checked or
//! saturating arithmetic and no indexing. The sailing plan stores **real UTC
//! instants** only ([`NotBefore`]); every IC figure is derived live through the
//! world's current timeline generation (`ic_at_utc`, `plan_instant`), so a rate
//! change relabels nothing (doc 18 §4.4 "IC-stamped conditions relabel on a rate
//! generation like journeys": there is no IC stamp to relabel).
//!
//! - `parse_at` reads `at=`'s value: `now`, `HH:MM` (the next such UTC minute),
//!   `daily HH:MM`, `+Nh`, `+Nm`, at most [`MAX_AHEAD_SECS`] ahead;
//! - `parse_until` reads `until=`'s value: `+Nh` or `+Nm`, at most
//!   [`MAX_WINDOW_SECS`];
//! - `resolve_due` is the one rule for when a depart is due (lead ruling L9): a
//!   daily departure resolves at the depart's **reach** — the real second the order
//!   arm first found it runnable — so it is never "missed": reached inside its
//!   window it leaves at once, after it, it waits for the next day's hour;
//! - `render_due` is the OOC-facing render (`+ship/orders`, `+ship/plan`): the UTC
//!   hour, the day word, the IC civil date and the real time until it.

use mush_core::World;
use serde::{Deserialize, Serialize};

use crate::port::families::strict_decimal_u32;
use crate::port::voyage::{real_span, utc_day_word, utc_hhmm};

/// `HH:MM` and `+Nh`/`+Nm` reach at most a week ahead (owner item O6).
pub const MAX_AHEAD_SECS: i64 = 7 * 86_400;
/// `until=` is at most 24 hours (owner item O6).
pub const MAX_WINDOW_SECS: u32 = 86_400;

/// Seconds in one UTC day.
const DAY: i64 = 86_400;

/// `at=`'s refusal.
pub const BAD_TIME: &str =
    "Departure times read now, HH:MM, daily HH:MM, +Nh or +Nm, in UTC, up to a week ahead.";
/// `until=`'s refusal.
pub const BAD_UNTIL: &str = "until= reads +Nh or +Nm, up to 24 hours.";

/// **When a departure may leave** — the `not_before` wire of a queued depart
/// (doc 18 §4.4; the IC11 brief's item 2): externally tagged, snake case
/// (`"now"`, `{"utc": 1760000000}`, `{"utc_daily": "19:30"}`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum NotBefore {
    /// As soon as it is reached.
    Now,
    /// A fixed real instant (unix seconds, UTC), fixed at filing.
    Utc(i64),
    /// Every day at this UTC wall time, `HH:MM` (resolved at the reach, L9).
    UtcDaily(String),
}

/// **`HH:MM` as minutes past midnight** (`"19:30"` → 1170): exactly two digits,
/// a colon and two digits, hours `00..=23` and minutes `00..=59`.
#[must_use]
pub(crate) fn hhmm_minutes(text: &str) -> Option<u32> {
    let (hours, minutes) = text.split_once(':')?;
    let two = |part: &str| part.len() == 2 && part.bytes().all(|b| b.is_ascii_digit());
    if !two(hours) || !two(minutes) {
        return None;
    }
    let (h, m) = (hours.parse::<u32>().ok()?, minutes.parse::<u32>().ok()?);
    (h <= 23 && m <= 59).then(|| h.saturating_mul(60).saturating_add(m))
}

/// `+N<unit>` as seconds, `N` a strict decimal of at least 1 and the unit `h` or
/// `m`; `None` for anything else or past `i64`.
fn plus_span(text: &str) -> Option<i64> {
    let rest = text.strip_prefix('+')?;
    let (digits, per) = match rest.strip_suffix('h') {
        Some(n) => (n, 3_600_i64),
        None => (rest.strip_suffix('m')?, 60_i64),
    };
    let n = strict_decimal_u32(digits).filter(|n| *n >= 1)?;
    i64::from(n).checked_mul(per)
}

/// The latest instant `<= at` whose UTC wall time is `minutes` past midnight, at
/// second 0.
fn latest_at_or_before(at: i64, minutes: u32) -> i64 {
    let day_start = at.div_euclid(DAY).saturating_mul(DAY);
    let candidate = day_start.saturating_add(i64::from(minutes).saturating_mul(60));
    if candidate <= at {
        candidate
    } else {
        candidate.saturating_sub(DAY)
    }
}

/// **`at=`'s value** (the IC11 brief's item 1): `now`; `HH:MM` → the first instant
/// `>= now_utc` at that UTC minute (today or tomorrow); `daily` with `next` an
/// `HH:MM` → [`NotBefore::UtcDaily`], consuming `next`; `+Nh`/`+Nm` → `now_utc`
/// plus the span. Case-insensitive.
///
/// # Errors
///
/// [`BAD_TIME`] for anything else, or a reach past [`MAX_AHEAD_SECS`].
pub(crate) fn parse_at(
    word: &str,
    next: Option<&str>,
    now_utc: i64,
) -> Result<(NotBefore, bool), &'static str> {
    let word = word.to_ascii_lowercase();
    if word == "now" {
        return Ok((NotBefore::Now, false));
    }
    if word == "daily" {
        let hhmm = next
            .filter(|text| hhmm_minutes(text).is_some())
            .ok_or(BAD_TIME)?;
        return Ok((NotBefore::UtcDaily(hhmm.to_owned()), true));
    }
    if let Some(minutes) = hhmm_minutes(&word) {
        let latest = latest_at_or_before(now_utc, minutes);
        let at = if latest >= now_utc {
            latest
        } else {
            latest.saturating_add(DAY)
        };
        return Ok((NotBefore::Utc(at), false));
    }
    let span = plus_span(&word)
        .filter(|span| *span <= MAX_AHEAD_SECS)
        .ok_or(BAD_TIME)?;
    let at = now_utc.checked_add(span).ok_or(BAD_TIME)?;
    Ok((NotBefore::Utc(at), false))
}

/// **`until=`'s value**: `+Nh` or `+Nm`, at most [`MAX_WINDOW_SECS`], as seconds.
///
/// # Errors
///
/// [`BAD_UNTIL`] for anything else.
pub(crate) fn parse_until(word: &str) -> Result<u32, &'static str> {
    plus_span(&word.to_ascii_lowercase())
        .and_then(|span| u32::try_from(span).ok())
        .filter(|span| *span <= MAX_WINDOW_SECS)
        .ok_or(BAD_UNTIL)
}

/// **When a depart is due** (lead ruling L9; reviewer F8): `Now` → the reach;
/// `Utc(t)` → `t`; `UtcDaily(hhmm)` → with `c` the latest instant `<= reached_utc`
/// at that UTC minute, `c` when `reached_utc <= c + window` (window 0 when `None`),
/// else `c + 1 day`. An unparseable daily time (never written: `Orders::check`
/// refuses it) resolves at the reach.
#[must_use]
pub(crate) fn resolve_due(at: &NotBefore, window_s: Option<u32>, reached_utc: i64) -> i64 {
    match at {
        NotBefore::Now => reached_utc,
        NotBefore::Utc(t) => *t,
        NotBefore::UtcDaily(hhmm) => {
            let Some(minutes) = hhmm_minutes(hhmm) else {
                return reached_utc;
            };
            let c = latest_at_or_before(reached_utc, minutes);
            let window = i64::from(window_s.unwrap_or(0));
            if reached_utc <= c.saturating_add(window) {
                c
            } else {
                c.saturating_add(DAY)
            }
        }
    }
}

/// **The IC instant of real second `utc`** through the world's current timeline
/// generation (a continuous IC axis, `journey.rs`'s relabel note; holds at the
/// frozen IC timestamp while the world clock is frozen).
#[must_use]
pub(crate) fn ic_at_utc(w: &World, utc: i64) -> i64 {
    w.world_timeline().stamp_at(utc).at
}

/// **The IC second at real second `real` under the epoch then in force** (doc 18 cut
/// IC13, the dormancy clock): the last epoch whose `real_anchor <= real`, else the
/// first, evaluated with `WorldTimelineEpoch::ic_at`. Unlike [`ic_at_utc`], which
/// evaluates only the current generation, this is correct for an instant before a
/// rate change and across a rate-0 epoch. Pure.
#[must_use]
pub(crate) fn ic_at_real(w: &World, real: i64) -> i64 {
    ic_at_real_in(w.world_timeline(), real)
}

/// [`ic_at_real`] over a timeline.
fn ic_at_real_in(timeline: &mush_core::WorldTimeline, real: i64) -> i64 {
    let epochs = timeline.epochs();
    epochs
        .iter()
        .rev()
        .find(|epoch| epoch.real_anchor <= real)
        .or_else(|| epochs.first())
        .map_or(0, |epoch| epoch.ic_at(real))
}

/// **The IC instant a passage due at `due_utc` is planned at**: never in the past
/// (`max(world IC now, ic_at_utc(due))`; D-18-41: "the plan at the depart order's
/// `not_before`, never now").
#[must_use]
pub(crate) fn plan_instant(w: &World, due_utc: i64) -> i64 {
    w.world_ic_now().at.max(ic_at_utc(w, due_utc))
}

/// **A due instant as the OOC renders print it** (doc 18 §4.4 "real-time ETA and
/// departure clock"): `"{HH:MM} UTC {day word} (IC {civil date}; in {real span})"`,
/// `IC clock stopped` for the IC clause at world rate 0 or while frozen, and `"now"`
/// for a due at or before the present.
#[must_use]
pub(crate) fn render_due(w: &World, due_utc: i64) -> String {
    let now = w.now();
    if due_utc <= now {
        return "now".to_owned();
    }
    let ic = if w.world_timeline().is_frozen() || w.world_timeline().current().rate == 0 {
        "IC clock stopped".to_owned()
    } else {
        format!(
            "IC {}",
            mush_core::CivilDate::from_ic_seconds(plan_instant(w, due_utc))
        )
    };
    format!(
        "{} UTC {} ({ic}; in {})",
        utc_hhmm(due_utc),
        utc_day_word(now, due_utc),
        real_span(due_utc.saturating_sub(now))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-10-17 00:00:00 UTC.
    const MIDNIGHT: i64 = 20_743 * 86_400;

    fn at(h: i64, m: i64) -> i64 {
        MIDNIGHT + h * 3_600 + m * 60
    }

    #[test]
    fn ic11_the_time_parser_reads_every_form() {
        let now = at(19, 0);
        assert_eq!(parse_at("now", None, now), Ok((NotBefore::Now, false)));
        assert_eq!(parse_at("NOW", None, now), Ok((NotBefore::Now, false)));
        // Today's minute when it is still ahead (or now), tomorrow's once it passed.
        assert_eq!(
            parse_at("19:30", None, now),
            Ok((NotBefore::Utc(at(19, 30)), false))
        );
        assert_eq!(
            parse_at("19:00", None, now),
            Ok((NotBefore::Utc(now), false))
        );
        assert_eq!(
            parse_at("18:59", None, now),
            Ok((NotBefore::Utc(at(18, 59) + 86_400), false))
        );
        assert_eq!(
            parse_at("19:01", None, now + 1),
            Ok((NotBefore::Utc(at(19, 1)), false))
        );
        // `daily` consumes its next token.
        assert_eq!(
            parse_at("daily", Some("19:30"), now),
            Ok((NotBefore::UtcDaily("19:30".to_owned()), true))
        );
        assert_eq!(parse_at("daily", None, now), Err(BAD_TIME));
        assert_eq!(parse_at("daily", Some("7:30"), now), Err(BAD_TIME));
        assert_eq!(
            parse_at("+90m", None, now),
            Ok((NotBefore::Utc(now + 5_400), false))
        );
        assert_eq!(
            parse_at("+2h", None, now),
            Ok((NotBefore::Utc(now + 7_200), false))
        );
        assert_eq!(
            parse_at("+168h", None, now),
            Ok((NotBefore::Utc(now + MAX_AHEAD_SECS), false))
        );
        for bad in [
            "+169h", "25:00", "7:5", "+0h", "-2h", "+02h", "+h", "+2d", "19:60", "1930", "",
        ] {
            assert_eq!(parse_at(bad, None, now), Err(BAD_TIME), "{bad:?}");
        }
        assert_eq!(hhmm_minutes("19:30"), Some(1_170));
        assert_eq!(hhmm_minutes("00:00"), Some(0));
        assert_eq!(hhmm_minutes("23:59"), Some(1_439));
        assert_eq!(hhmm_minutes("24:00"), None);
        assert_eq!(parse_until("+2h"), Ok(7_200));
        assert_eq!(parse_until("+90M"), Ok(5_400));
        assert_eq!(parse_until("+24h"), Ok(86_400));
        for bad in ["+25h", "+0m", "2h", "now", "+1441m"] {
            assert_eq!(parse_until(bad), Err(BAD_UNTIL), "{bad:?}");
        }
    }

    #[test]
    fn ic11_daily_resolves_at_the_reach_and_rolls_past_its_window() {
        let daily = NotBefore::UtcDaily("19:30".to_owned());
        // Reached before the hour: today's hour.
        assert_eq!(resolve_due(&daily, Some(7_200), at(19, 0)), at(19, 30));
        // Inside the window: that hour, at or before the reach, so due now.
        let inside = resolve_due(&daily, Some(7_200), at(20, 15));
        assert_eq!(inside, at(19, 30));
        assert!(inside <= at(20, 15));
        // Past the window: tomorrow's hour.
        assert_eq!(
            resolve_due(&daily, Some(7_200), at(21, 31)),
            at(19, 30) + 86_400
        );
        // No window and one minute late: tomorrow; exactly on the minute: now.
        assert_eq!(resolve_due(&daily, None, at(19, 31)), at(19, 30) + 86_400);
        assert_eq!(resolve_due(&daily, None, at(19, 30)), at(19, 30));
        // `Now` and `Utc` pass through.
        assert_eq!(resolve_due(&NotBefore::Now, None, at(3, 0)), at(3, 0));
        assert_eq!(
            resolve_due(&NotBefore::Utc(at(5, 0)), Some(60), at(3, 0)),
            at(5, 0)
        );
    }

    /// Reviewer F8: two consecutive daily departs resolve at their own reach.
    #[test]
    fn ic11_two_consecutive_daily_departs_resolve_at_their_own_reach() {
        let daily = NotBefore::UtcDaily("19:30".to_owned());
        let window = Some(7_200);
        // A is reached at 19:00 and sails at 19:30.
        assert_eq!(resolve_due(&daily, window, at(19, 0)), at(19, 30));
        // B becomes the cursor at 19:35 when A's launch is verified; had it been
        // resolved then, it would be due at once (inside its window).
        let at_cursor = resolve_due(&daily, window, at(19, 35));
        assert_eq!(at_cursor, at(19, 30));
        // But B is reached only when the hull docks, ~20 h later at 15:35 the next
        // day: it is due at 19:30 of THAT day, so she does not sail on arrival.
        let reached = at(15, 35) + 86_400;
        let at_reach = resolve_due(&daily, window, reached);
        assert_eq!(at_reach, at(19, 30) + 86_400);
        assert!(at_reach > reached, "she waits for the hour");
        assert_ne!(at_cursor, at_reach);
    }

    fn world_at(now: i64, rate: u32) -> World {
        let mut w = World::new_minimal();
        w.set_now(now);
        w.set_world_timeline(
            mush_core::WorldTimeline::from_epochs(vec![mush_core::WorldTimelineEpoch {
                real_anchor: now - 100,
                ic_anchor: 1_000_000,
                rate,
                generation: 1,
            }])
            .expect("one epoch"),
        );
        w
    }

    #[test]
    fn ic11_utc_maps_to_ic_through_the_current_generation() {
        let now = at(19, 0);
        let w = world_at(now, 3_600);
        assert_eq!(ic_at_utc(&w, now), 1_000_000 + 100 * 3_600);
        assert_eq!(ic_at_utc(&w, now + 10), 1_000_000 + 110 * 3_600);
        assert_eq!(w.world_ic_now().at, ic_at_utc(&w, now));
        // A due in the future plans at its own IC instant; a past one clamps to now.
        assert_eq!(plan_instant(&w, now + 60), 1_000_000 + 160 * 3_600);
        assert_eq!(plan_instant(&w, now - 60), w.world_ic_now().at);
        let line = render_due(&w, now + 1_800);
        assert!(line.starts_with("19:30 UTC today (IC "), "{line}");
        assert!(line.ends_with("; in 30 min)"), "{line}");
        assert_eq!(render_due(&w, now), "now");
        assert_eq!(render_due(&w, now - 5), "now");
        // At rate 0 the IC clock stands still: every instant maps to the anchor.
        let stopped = world_at(now, 0);
        assert_eq!(ic_at_utc(&stopped, now + 3_600), 1_000_000);
        assert_eq!(plan_instant(&stopped, now + 3_600), 1_000_000);
        assert_eq!(
            render_due(&stopped, now + 3_600),
            "20:00 UTC today (IC clock stopped; in 1 h 0 min)"
        );
    }

    /// Doc 18 cut IC13: the dormancy cutoff is read under the epoch in force at that
    /// real instant — 24, then 3600, then a stopped clock.
    #[test]
    fn ic13_ic_at_real_reads_the_epoch_in_force() {
        use mush_core::clock::WorldTimelineEpoch;
        let first = WorldTimelineEpoch {
            real_anchor: 1_000,
            ic_anchor: 50_000,
            rate: 24,
            generation: 1,
        };
        let second = WorldTimelineEpoch {
            real_anchor: 2_000,
            ic_anchor: first.ic_at(2_000),
            rate: 3_600,
            generation: 2,
        };
        let third = WorldTimelineEpoch {
            real_anchor: 3_000,
            ic_anchor: second.ic_at(3_000),
            rate: 0,
            generation: 3,
        };
        let timeline =
            mush_core::WorldTimeline::from_epochs(vec![first, second, third]).expect("contiguous");
        // Inside each epoch.
        assert_eq!(ic_at_real_in(&timeline, 1_500), 50_000 + 500 * 24);
        assert_eq!(
            ic_at_real_in(&timeline, 2_500),
            50_000 + 1_000 * 24 + 500 * 3_600
        );
        let stopped = 50_000 + 1_000 * 24 + 1_000 * 3_600;
        assert_eq!(ic_at_real_in(&timeline, 3_500), stopped);
        assert_eq!(ic_at_real_in(&timeline, 9_999_999), stopped);
        // On each anchor (continuous, so either neighbour agrees).
        assert_eq!(ic_at_real_in(&timeline, 1_000), 50_000);
        assert_eq!(ic_at_real_in(&timeline, 2_000), 50_000 + 1_000 * 24);
        assert_eq!(ic_at_real_in(&timeline, 3_000), stopped);
        // Before the first: the first epoch, extrapolated backwards.
        assert_eq!(ic_at_real_in(&timeline, 900), 50_000 - 100 * 24);
        // The current-epoch reading `ic_at_utc` would give is wrong before the last
        // rate change; this one is not.
        assert_ne!(
            timeline.current().ic_at(1_500),
            ic_at_real_in(&timeline, 1_500)
        );
    }
}

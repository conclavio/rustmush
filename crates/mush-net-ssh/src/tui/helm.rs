//! **The camera on a leash** (doc 21 cut G.4, D-21-14): the pixel view's
//! turn keys steer the tank, and the picture shows the tank's heading as the
//! sim will have it, never one the sim has not granted.
//!
//! A turn is a **helm order** — a `HEADING <deg>` line, the verb and the
//! authority a pilot types — so a seat without the helm, or a tank without
//! power, simply does not turn. The sim resolves orders at its pulses (500,
//! 1000 or 5000 ms by tempo), so between frames the view **predicts** the
//! heading: from the last frame's, toward the order it sent, at the tank's
//! turn rate, starting no earlier than the order went out and never past it.
//! The next frame is the truth and replaces the prediction.
//!
//! A held turn orders a heading one pulse's turn ahead of the predicted one
//! and extends it as the tank closes on it; letting go orders the predicted
//! heading, so the tank stops about where the picture showed it. The lead is
//! kept to a pulse's worth so a release lands before the tank has turned much
//! past what the pilot saw.
//!
//! **The turn rate is measured, not read**: no feed carries it (it is the
//! unit definition's `turn_rate_mdeg_s`, 30°/s on the stock tank), so it
//! starts at that and is re-measured from consecutive frames — only across
//! a pulse in which the tank was still short of the order, since a turn that
//! finished mid-pulse would read slow.
//!
//! **The throttle and the lock cycle** (doc 21 cut G.12b) sit beside the
//! helm: [`Throttle`] steps `SPEED` orders the same way, from a prediction the
//! frame corrects, and [`lock_order`] / [`lock_next`] are the `t` / `T` walk
//! over the frame's contacts.

use std::time::{Duration, Instant};

use serde_json::Value;

/// The rate assumed until a turn is measured: the stock tank's, 30°/s.
pub const DEFAULT_TURN_RATE: i64 = 30_000;
/// The measured rate is kept inside this band (millidegrees a second).
const RATE_BAND: (i64, i64) = (5_000, 180_000);
/// The least time between two orders while a turn is held, and between two
/// `SPEED` orders while the speed keys are pressed (G.12b).
pub const ORDER_GAP: Duration = Duration::from_millis(250);
/// A held turn's lead is a pulse of turning, kept inside this band.
const LEAD_BAND: (i64, i64) = (20_000, 120_000);
/// A frame within this of the order has reached it (millidegrees).
const REACHED: i64 = 1_000;

/// One full turn, millidegrees.
const FULL: i64 = 360_000;

/// `to - from` on the shortest arc, `-180_000..=180_000`.
#[must_use]
pub fn arc(from: i64, to: i64) -> i64 {
    let d = (to - from).rem_euclid(FULL);
    if d > FULL / 2 { d - FULL } else { d }
}

/// The pulse length a tempo word stands for, at the shipped
/// `[tacsim.arena.tempo]` defaults.
fn tempo_ms(word: &str) -> Option<i64> {
    Some(match word {
        "quiet" => 5_000,
        "alert" => 1_000,
        "engaged" => 500,
        _ => return None,
    })
}

/// The last frame the helm saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seen {
    /// The frame's arena time.
    t_ms: i64,
    /// Its heading.
    heading: i64,
    /// When it arrived here.
    at: Instant,
}

/// The tank's helm as the pixel view drives it.
#[derive(Clone, Debug)]
pub struct Helm {
    /// `-1` turning left, `0` not, `1` turning right.
    turning: i8,
    /// The heading last ordered, and when.
    ordered: Option<(i64, Instant)>,
    seen: Option<Seen>,
    rate: i64,
    tempo_ms: i64,
    /// `HEADING` lines waiting to be sent.
    orders: Vec<String>,
}

impl Default for Helm {
    fn default() -> Self {
        Self::new()
    }
}

impl Helm {
    /// A helm that has seen no frame and ordered nothing.
    #[must_use]
    pub fn new() -> Helm {
        Helm {
            turning: 0,
            ordered: None,
            seen: None,
            rate: DEFAULT_TURN_RATE,
            tempo_ms: 1_000,
            orders: Vec::new(),
        }
    }

    /// Which way the pilot is turning the tank: `-1`, `0`, `1`.
    #[must_use]
    pub fn turning(&self) -> i8 {
        self.turning
    }

    /// The heading last ordered and not yet reached, millidegrees.
    #[must_use]
    pub fn ordered(&self) -> Option<i64> {
        self.ordered.map(|(o, _)| o)
    }

    /// The turn rate in use, millidegrees a second.
    #[must_use]
    pub fn rate(&self) -> i64 {
        self.rate
    }

    /// Read the held cockpit frame: a new one (by its arena time) replaces
    /// the prediction's base, and across a pulse the tank spent turning
    /// toward an order it had not reached it measures the turn rate.
    pub fn observe(&mut self, cockpit: &Value, now: Instant) {
        let Some(t_ms) = cockpit.get("t_ms").and_then(Value::as_i64) else {
            return;
        };
        let heading = cockpit
            .get("heading_mdeg")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .rem_euclid(FULL);
        if let Some(ms) = cockpit
            .get("tempo")
            .and_then(Value::as_str)
            .and_then(tempo_ms)
        {
            self.tempo_ms = ms;
        }
        if let Some(prev) = self.seen {
            if prev.t_ms == t_ms {
                return;
            }
            let turned = arc(prev.heading, heading).abs();
            let dt = t_ms - prev.t_ms;
            if let Some((o, _)) = self.ordered
                && dt > 0
                && turned > 0
                && arc(heading, o).abs() > REACHED
            {
                let measured = turned * 1_000 / dt;
                self.rate = measured.clamp(RATE_BAND.0, RATE_BAND.1);
            }
        }
        self.seen = Some(Seen {
            t_ms,
            heading,
            at: now,
        });
        if self.turning == 0
            && self
                .ordered
                .is_some_and(|(o, _)| arc(heading, o).abs() <= REACHED)
        {
            self.ordered = None;
        }
    }

    /// The heading the tank has at `now` as far as the view can tell: the
    /// last frame's, carried toward the order at the turn rate from when the
    /// order went out (or the frame arrived, if later), for at most a pulse
    /// and a half, never past the order. An order no frame has answered a
    /// second past its pulse is taken as not taken (no helm, no power), and
    /// the frame's heading stands. `None` before any frame.
    #[must_use]
    pub fn predicted(&self, now: Instant) -> Option<i64> {
        let seen = self.seen?;
        let Some((o, at)) = self.ordered else {
            return Some(seen.heading);
        };
        let pulse = Duration::from_millis(u64::try_from(self.tempo_ms).unwrap_or(1_000));
        if seen.at < at && now.saturating_duration_since(at) > pulse + Duration::from_secs(1) {
            return Some(seen.heading);
        }
        let from = seen.at.max(at);
        let elapsed = now
            .saturating_duration_since(from)
            .min(pulse + pulse / 2)
            .as_millis();
        let step = self.rate * i64::try_from(elapsed).unwrap_or(0) / 1_000;
        let d = arc(seen.heading, o);
        Some((seen.heading + d.signum() * d.abs().min(step)).rem_euclid(FULL))
    }

    /// How far ahead a held turn orders: a pulse and a half of turning.
    fn lead(&self) -> i64 {
        (self.rate * (self.tempo_ms + self.tempo_ms / 2) / 1_000).clamp(LEAD_BAND.0, LEAD_BAND.1)
    }

    fn order(&mut self, heading: i64, now: Instant) {
        let deg = (heading.rem_euclid(FULL) + 500) / 1_000 % 360;
        self.orders.push(format!("HEADING {deg}"));
        self.ordered = Some((deg * 1_000, now));
    }

    /// Start turning the tank `dir` (`-1` left, `1` right). Nothing before a
    /// frame has been seen: there is no heading to turn from.
    pub fn start(&mut self, dir: i8, now: Instant) {
        let Some(p) = self.predicted(now) else {
            return;
        };
        self.turning = dir.signum();
        self.order(p + i64::from(self.turning) * self.lead(), now);
    }

    /// Stop turning: order the heading the picture shows, unless the tank
    /// is already there.
    pub fn stop(&mut self, now: Instant) {
        self.turning = 0;
        if let (Some(p), Some(o)) = (self.predicted(now), self.ordered())
            && arc(p, o).abs() > REACHED
        {
            self.order(p, now);
        }
    }

    /// While a turn is held, extend the order before the tank reaches it.
    pub fn tick(&mut self, now: Instant) {
        if self.turning == 0 {
            return;
        }
        let (Some(p), Some((o, at))) = (self.predicted(now), self.ordered) else {
            return;
        };
        if arc(p, o).abs() < self.lead() / 2 && now.saturating_duration_since(at) >= ORDER_GAP {
            self.order(p + i64::from(self.turning) * self.lead(), now);
        }
    }

    /// The `HEADING` lines to send, oldest first.
    pub fn take_orders(&mut self) -> Vec<String> {
        std::mem::take(&mut self.orders)
    }
}

// ---- the throttle (doc 21 cut G.12b) ----------------------------------------

/// The top speed assumed when the frame names none (`max_speed_kph`): the
/// stock tank's 60 kph.
pub const DEFAULT_MAX_KPH: i64 = 60;
/// The reverse ceiling assumed when the frame names none (`reverse_kph`).
pub const DEFAULT_REVERSE_KPH: i64 = 20;
/// The least speed step, kph.
pub const MIN_STEP_KPH: i64 = 5;
/// A frame within this of the ordered speed has reached it (kph: the frame's
/// `speed_kph` is rounded).
const SPEED_REACHED_KPH: i64 = 1;

/// **The throttle** (doc 21 §7.3, cut G.12b): `w` and `s` step the ordered
/// speed by a tenth of the hull's top speed (at least [`MIN_STEP_KPH`]),
/// through zero into reverse and clamped at either ceiling, each step a
/// `SPEED <kph>` line — the verb and the authority a pilot types, so a hull
/// that cannot make way simply does not.
///
/// A step is taken from the **predicted** speed: the last order while the
/// frame has not reached it, else the frame's `speed_kph`, which is the truth
/// and replaces the prediction once it comes within a kph of the order. Steps
/// faster than [`ORDER_GAP`] coalesce — the last value wins when the gap
/// opens — so a key held down sends a few orders, not one a repeat.
#[derive(Clone, Debug)]
pub struct Throttle {
    /// The speed last ordered and not yet reached, kph.
    ordered: Option<i64>,
    /// A step taken inside the gap, waiting to go out.
    pending: Option<i64>,
    /// When the last order went out.
    sent_at: Option<Instant>,
    /// The frame's speed, kph.
    seen: Option<i64>,
    /// The ceiling ahead, kph.
    max: i64,
    /// The ceiling astern, kph (positive).
    reverse: i64,
    /// `SPEED` lines waiting to be sent.
    orders: Vec<String>,
}

impl Default for Throttle {
    fn default() -> Self {
        Self::new()
    }
}

impl Throttle {
    /// A throttle that has seen no frame, at the default ceilings.
    #[must_use]
    pub fn new() -> Throttle {
        Throttle {
            ordered: None,
            pending: None,
            sent_at: None,
            seen: None,
            max: DEFAULT_MAX_KPH,
            reverse: DEFAULT_REVERSE_KPH,
            orders: Vec::new(),
        }
    }

    /// Read the held cockpit frame: its speed, and its ceilings where it
    /// names them (G.12a's `max_speed_kph` and `reverse_kph`). An order the
    /// frame has reached is closed; so is any order on a hull that is a wreck
    /// or immobile, which will never reach it.
    pub fn observe(&mut self, cockpit: &Value) {
        if let Some(max) = cockpit.get("max_speed_kph").and_then(Value::as_i64) {
            self.max = max.max(0);
        }
        if let Some(rev) = cockpit.get("reverse_kph").and_then(Value::as_i64) {
            self.reverse = rev.abs();
        }
        let Some(kph) = cockpit.get("speed_kph").and_then(Value::as_i64) else {
            return;
        };
        self.seen = Some(kph);
        let stuck = ["wreck", "immobile"]
            .iter()
            .any(|k| cockpit.get(*k).and_then(Value::as_bool) == Some(true));
        if stuck
            || self
                .ordered
                .is_some_and(|o| (o - kph).abs() <= SPEED_REACHED_KPH)
        {
            self.ordered = None;
        }
    }

    /// One step, kph: a tenth of the top speed, at least [`MIN_STEP_KPH`].
    #[must_use]
    pub fn step(&self) -> i64 {
        ((self.max + 5) / 10).max(MIN_STEP_KPH)
    }

    /// The speed the hull is making as far as the view can tell: the newest
    /// order not yet reached, else the frame's.
    #[must_use]
    pub fn predicted(&self) -> i64 {
        self.pending
            .or(self.ordered)
            .unwrap_or_else(|| self.seen.unwrap_or(0))
    }

    /// The frame's speed, kph.
    #[must_use]
    pub fn seen(&self) -> Option<i64> {
        self.seen
    }

    /// The speed ordered (or about to be) and not yet reached, kph.
    #[must_use]
    pub fn ordered(&self) -> Option<i64> {
        self.pending.or(self.ordered)
    }

    /// `w` (`dir` 1) or `s` (`-1`): one step from the predicted speed,
    /// clamped at the ceilings.
    pub fn bump(&mut self, dir: i8, now: Instant) {
        let want = (self.predicted() + i64::from(dir.signum()) * self.step())
            .clamp(-self.reverse, self.max);
        self.pending = Some(want);
        self.tick(now);
    }

    /// `x`: all stop, ordered at once whatever the gap.
    pub fn stop(&mut self, now: Instant) {
        self.pending = Some(0);
        self.flush(now);
    }

    /// Send a step still waiting on the gap now: the view is hiding, and its
    /// clock, which would have sent it, stops with it.
    pub fn flush(&mut self, now: Instant) {
        self.sent_at = None;
        self.tick(now);
    }

    /// Send a coalesced step once the gap since the last order has opened.
    pub fn tick(&mut self, now: Instant) {
        let Some(want) = self.pending else {
            return;
        };
        if self
            .sent_at
            .is_some_and(|at| now.saturating_duration_since(at) < ORDER_GAP)
        {
            return;
        }
        self.pending = None;
        self.sent_at = Some(now);
        self.ordered = Some(want);
        self.orders.push(format!("SPEED {want}"));
    }

    /// The `SPEED` lines to send, oldest first.
    pub fn take_orders(&mut self) -> Vec<String> {
        std::mem::take(&mut self.orders)
    }
}

// ---- the lock cycle (doc 21 cut G.12b) ---------------------------------------

/// **The lock cycle's order** (doc 21 §7.3): the frame's contacts by
/// designator, hostiles first (classified to a team and not a friend), then
/// the unidentified (`class` `unknown`), then the rest (classified friends),
/// nearest first within each, the designator breaking a tie.
#[must_use]
pub fn lock_order(cockpit: &Value) -> Vec<String> {
    let mut rows: Vec<(u8, i64, String)> = cockpit
        .get("contacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let name = c.get("designator").and_then(Value::as_str)?.to_string();
            let unknown = c
                .get("class")
                .and_then(Value::as_str)
                .is_none_or(|k| k == "unknown");
            let friendly = c.get("friendly").and_then(Value::as_bool) == Some(true);
            let rank = match (unknown, friendly) {
                (false, false) => 0,
                (true, _) => 1,
                (false, true) => 2,
            };
            let range = c.get("range_m").and_then(Value::as_i64).unwrap_or(i64::MAX);
            Some((rank, range, name))
        })
        .collect();
    rows.sort();
    rows.into_iter().map(|(_, _, name)| name).collect()
}

/// The contact `t` (`dir` 1) or `T` (`-1`) locks: the one after (before)
/// `current` in `order`, wrapping; the first when nothing is locked or the
/// lock is not in the order; nothing with no contacts.
#[must_use]
pub fn lock_next(order: &[String], current: Option<&str>, dir: i8) -> Option<String> {
    let n = order.len();
    if n == 0 {
        return None;
    }
    let at = current.and_then(|c| order.iter().position(|d| d == c));
    let i = match at {
        None => 0,
        Some(i) if dir >= 0 => (i + 1) % n,
        Some(i) => (i + n - 1) % n,
    };
    order.get(i).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(t_ms: i64, heading_deg: i64, tempo: &str) -> Value {
        serde_json::json!({ "t_ms": t_ms, "heading_mdeg": heading_deg * 1_000, "tempo": tempo })
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn arcs_are_short() {
        assert_eq!(arc(350_000, 10_000), 20_000);
        assert_eq!(arc(10_000, 350_000), -20_000);
        assert_eq!(arc(0, 180_000), 180_000);
        assert_eq!(arc(90_000, 90_000), 0);
    }

    /// No frame, no heading to turn from: nothing is ordered.
    #[test]
    fn a_helm_without_a_frame_orders_nothing() {
        let t0 = Instant::now();
        let mut h = Helm::new();
        h.start(1, t0);
        assert!(h.take_orders().is_empty());
        assert_eq!(h.predicted(t0), None);
    }

    /// A held turn orders a pulse and a half ahead, the picture turns at the
    /// rate from when the order went out, and letting go orders where the
    /// picture is.
    #[test]
    fn a_turn_leads_by_a_pulse_and_a_release_orders_the_shown_heading() {
        let t0 = Instant::now();
        let mut h = Helm::new();
        h.observe(&frame(10_000, 90, "alert"), t0);
        assert_eq!(h.predicted(t0), Some(90_000));
        h.start(1, t0 + ms(200));
        // 30°/s for 1.5 s of 1000 ms pulses: 45° ahead.
        assert_eq!(h.take_orders(), ["HEADING 135"]);
        assert_eq!(
            h.predicted(t0 + ms(200)),
            Some(90_000),
            "moves from the order"
        );
        assert_eq!(h.predicted(t0 + ms(700)), Some(105_000));
        // Never past a pulse and a half without a frame (and never past
        // the order).
        assert_eq!(h.predicted(t0 + ms(1_700)), Some(135_000));
        h.stop(t0 + ms(700));
        assert_eq!(h.take_orders(), ["HEADING 105"]);
        assert_eq!(h.turning(), 0);
        // A frame at the ordered heading closes the order.
        h.observe(&frame(11_000, 105, "alert"), t0 + ms(1_000));
        assert_eq!(h.ordered(), None);
        assert_eq!(h.predicted(t0 + ms(3_000)), Some(105_000));
    }

    /// Frames across a pulse the tank spent turning measure the rate; one in
    /// which it reached the order does not.
    #[test]
    fn the_turn_rate_is_measured_while_short_of_the_order() {
        let t0 = Instant::now();
        let mut h = Helm::new();
        h.observe(&frame(1_000, 0, "engaged"), t0);
        h.start(-1, t0);
        // A pulse and a half at 500 ms pulses: 22.5° ahead.
        assert_eq!(h.take_orders(), ["HEADING 338"]);
        h.observe(&frame(1_500, 338, "engaged"), t0 + ms(500));
        assert_eq!(h.rate(), DEFAULT_TURN_RATE, "reached it: no measure");
        // Ordered on to 316; 18° in the next 500 ms pulse, still short of
        // it: a faster chassis than assumed.
        h.start(-1, t0 + ms(600));
        assert_eq!(h.take_orders(), ["HEADING 316"]);
        h.observe(&frame(2_000, 320, "engaged"), t0 + ms(1_000));
        assert_eq!(h.rate(), 36_000);
    }

    /// An order the sim never answers (no helm, no power) stops being drawn
    /// a second past its pulse.
    #[test]
    fn an_unanswered_order_is_not_drawn_past_its_pulse() {
        let t0 = Instant::now();
        let mut h = Helm::new();
        h.observe(&frame(1_000, 90, "engaged"), t0);
        h.start(1, t0 + ms(10));
        let _ = h.take_orders();
        assert!(h.predicted(t0 + ms(400)).is_some_and(|p| p > 90_000));
        assert_eq!(h.predicted(t0 + ms(1_600)), Some(90_000));
    }

    /// While held, the order is extended before the tank reaches it, no
    /// sooner than the order gap.
    #[test]
    fn a_held_turn_is_extended_before_it_runs_out() {
        let t0 = Instant::now();
        let mut h = Helm::new();
        h.observe(&frame(1_000, 0, "alert"), t0);
        h.start(1, t0);
        assert_eq!(h.take_orders(), ["HEADING 45"]);
        h.tick(t0 + ms(100));
        assert!(h.take_orders().is_empty(), "far from the order");
        // A frame at 30°: 15° short, under half the 45° lead.
        h.observe(&frame(2_000, 30, "alert"), t0 + ms(1_000));
        h.tick(t0 + ms(1_000));
        assert_eq!(h.take_orders(), ["HEADING 75"]);
        h.tick(t0 + ms(1_100));
        assert!(h.take_orders().is_empty(), "the gap holds");
    }

    // ---- G.12b ---------------------------------------------------------------

    fn speed(kph: i64) -> Value {
        serde_json::json!({ "speed_kph": kph })
    }

    /// `w` from rest orders a tenth of the top speed; steps coalesce inside
    /// the gap and the last wins; the frame replaces the prediction once it
    /// reaches the order.
    #[test]
    fn the_throttle_steps_a_tenth_and_coalesces() {
        let t0 = Instant::now();
        let mut t = Throttle::new();
        t.observe(&speed(0));
        assert_eq!(t.step(), 6, "a tenth of the default 60");
        t.bump(1, t0);
        assert_eq!(t.take_orders(), ["SPEED 6"]);
        assert_eq!((t.predicted(), t.ordered()), (6, Some(6)));
        // Two more inside the gap: one order, the last value, when it opens.
        t.bump(1, t0 + ms(50));
        t.bump(1, t0 + ms(100));
        assert!(t.take_orders().is_empty(), "the gap holds");
        assert_eq!(t.predicted(), 18, "but the prediction moves");
        t.tick(t0 + ORDER_GAP);
        assert_eq!(t.take_orders(), ["SPEED 18"]);
        // The frame on its way: still the order; reached: the frame's.
        t.observe(&speed(10));
        assert_eq!(t.predicted(), 18);
        t.observe(&speed(17));
        assert_eq!((t.predicted(), t.ordered()), (17, None));
        // All stop goes at once.
        t.stop(t0 + ORDER_GAP + ms(10));
        assert_eq!(t.take_orders(), ["SPEED 0"]);
        // A step inside the gap goes out on a flush (the view hiding).
        t.bump(1, t0 + ORDER_GAP + ms(20));
        assert!(t.take_orders().is_empty());
        t.flush(t0 + ORDER_GAP + ms(30));
        assert_eq!(t.take_orders(), ["SPEED 6"]);
    }

    /// The frame's ceilings set the step and the clamps; through zero into
    /// reverse, held at the reverse ceiling.
    #[test]
    fn the_throttle_clamps_at_the_frames_ceilings() {
        let t0 = Instant::now();
        let mut t = Throttle::new();
        t.observe(&serde_json::json!({ "speed_kph": 40, "max_speed_kph": 43, "reverse_kph": 12 }));
        assert_eq!(t.step(), 5, "a tenth of 43 is under the floor");
        t.bump(1, t0);
        assert_eq!(t.take_orders(), ["SPEED 43"], "clamped at the top");
        let mut later = t0;
        for want in ["SPEED 38", "SPEED 33"] {
            later += ORDER_GAP;
            t.bump(-1, later);
            assert_eq!(t.take_orders(), [want]);
        }
        let mut r = Throttle::new();
        r.observe(&serde_json::json!({ "speed_kph": 0, "max_speed_kph": 43, "reverse_kph": 12 }));
        r.bump(-1, t0);
        r.bump(-1, t0 + ORDER_GAP);
        r.bump(-1, t0 + ORDER_GAP * 2);
        assert_eq!(r.take_orders(), ["SPEED -5", "SPEED -10", "SPEED -12"]);
        // A wreck never reaches its order: the frame's speed stands.
        r.observe(&serde_json::json!({ "speed_kph": 0, "wreck": true }));
        assert_eq!((r.predicted(), r.ordered()), (0, None));
    }

    /// Hostiles first, then the unidentified, then friends, nearest first;
    /// the cycle wraps both ways and starts at the first.
    #[test]
    fn the_lock_cycle_walks_hostiles_then_unknowns_then_friends() {
        let frame: Value = serde_json::from_str(include_str!(
            "../../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"
        ))
        .expect("fixture");
        // BA blue hostile 450 m; X3 unknown 200 m; X4 unknown 600 m; RB friend.
        let order = lock_order(&frame);
        assert_eq!(order, ["BA", "X3", "X4", "RB"]);
        assert_eq!(lock_next(&order, Some("BA"), 1).as_deref(), Some("X3"));
        assert_eq!(lock_next(&order, Some("RB"), 1).as_deref(), Some("BA"));
        assert_eq!(lock_next(&order, Some("BA"), -1).as_deref(), Some("RB"));
        assert_eq!(lock_next(&order, None, -1).as_deref(), Some("BA"));
        assert_eq!(lock_next(&order, Some("ZZ"), 1).as_deref(), Some("BA"));
        assert_eq!(lock_next(&[], None, 1), None);
    }
}

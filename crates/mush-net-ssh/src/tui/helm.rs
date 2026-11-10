//! **The camera on a leash** (doc 21 cut G.4, D-21-14): the pixel view's
//! turn keys steer the tank, and the picture turns as the tank will.
//!
//! A turn is a **helm order** — a `HEADING <deg>` line, the verb and the
//! authority a pilot types — so a seat without the helm, or a tank without
//! power, simply does not turn. The keys' orders carry the verbs' `QUIET`
//! word (doc 21 §7.3, 2026-10-27): the arena takes them without the
//! `Steering to …` / `Making … kph.` acknowledgement a typed order gets,
//! since the HUD shows the ordered heading and speed and a held key would
//! otherwise fill the comms; refusals are still spoken.
//!
//! The sim resolves orders at its pulses (500, 1000 or 5000 ms by tempo), and
//! an order that arrives late in a pulse is still given the whole pulse of
//! turning: a frame's heading moves in steps of up to a pulse's turn. The
//! picture must not step with it (the owner, 2026-11-09: "it jerks and moves
//! in wide swaths"). So the helm keeps **one continuous shown heading**
//! ([`Helm::predicted`]), carried on from where it stands at every event and
//! never restarted from a frame:
//!
//! - **held**, it swings from where it stood at the press: a slow first
//!   [`TAP_MS`] so a tap is one degree, then the unit's turn rate. The press
//!   orders that one degree, and every [`ORDER_GAP`] after it the helm orders
//!   the heading the picture will show one gap on, so at each pulse the sim
//!   lands on the picture or a few degrees ahead of it, and the frames of a
//!   turn in hand are not drawn;
//! - **released**, the helm orders the heading the picture shows and the
//!   picture holds it. The sim brings the tank there at its next pulse, back
//!   the few degrees of the lead if it had run ahead;
//! - **otherwise** (a typed order, a machine at the helm) it follows the
//!   frames at the turn rate, at most a pulse behind, in place of stepping.
//!
//! **The leash**: the picture never runs further ahead of the last frame than
//! a pulse and a half of turning (and never so far that an order would be the
//! long way round), a hull the frame says cannot turn is not drawn turning,
//! and a turn the frames do not bear out — no helm, an order refused — is
//! dropped a second past its pulse, the picture easing back to the frames.
//!
//! **The turn rate** is the frame's `turn_rate_mdeg_s` (the unit definition's,
//! 20°/s to 90°/s across the shipped ground hulls). A frame without one — a
//! replay, a fixture — leaves the stock tank's 30°/s, re-measured from
//! consecutive frames across a pulse the tank spent short of its order.
//!
//! **The throttle and the lock cycle** (doc 21 cut G.12b) sit beside the
//! helm: [`Throttle`] steps `SPEED` orders the same way, from a prediction the
//! frame corrects, and [`lock_order`] / [`lock_next`] are the `t` / `T` walk
//! over the frame's contacts.

use std::time::{Duration, Instant};

use serde_json::Value;

/// The rate assumed until a frame carries one or a turn is measured: the
/// stock tank's, 30°/s.
pub const DEFAULT_TURN_RATE: i64 = 30_000;
/// The turn rate is kept inside this band (millidegrees a second).
const RATE_BAND: (i64, i64) = (1_000, 360_000);
/// The time between two orders while a turn is held, and the least between
/// two `SPEED` orders while the speed keys are pressed (G.12b).
pub const ORDER_GAP: Duration = Duration::from_millis(250);
/// A held turn's order runs ahead of the picture by what the picture will
/// swing in one [`ORDER_GAP`], kept inside this band (millidegrees).
const LEAD_BAND: (i64, i64) = (1_000, 25_000);
/// The furthest the picture runs ahead of the last frame (millidegrees): a
/// pulse and a half of turning inside this band. With [`LEAD_BAND`]'s top it
/// keeps an order under a half turn from the tank, which turns on the short
/// arc.
const LEASH_BAND: (i64, i64) = (20_000, 150_000);
/// A frame within this of the order has reached it (millidegrees).
const REACHED: i64 = 1_000;
/// The least a press turns the tank, and what a tap turns it: one degree.
const TAP: i64 = 1_000;
/// A press this short is a tap: the swing's slow start, milliseconds.
pub const TAP_MS: i64 = 200;
/// No prediction runs longer than this without an event, milliseconds.
const RUN_CAP_MS: i64 = 600_000;

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

/// Whole milliseconds from `from` to `to`, zero when `to` is the earlier.
fn ms_between(from: Instant, to: Instant) -> i64 {
    i64::try_from(to.saturating_duration_since(from).as_millis())
        .unwrap_or(RUN_CAP_MS)
        .min(RUN_CAP_MS)
}

/// The last frame the helm saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seen {
    /// The frame's arena time.
    t_ms: i64,
    /// Its heading.
    heading: i64,
    /// Its ground speed, centimetres per second.
    speed_cms: i64,
    /// Whether the hull can physically make way (powered, unblocked, mobile).
    can_move: bool,
    /// Whether the hull can turn at all (powered, mobile, not a wreck): the
    /// arena refuses a `HEADING` otherwise.
    can_turn: bool,
    /// When it arrived here.
    at: Instant,
}

/// The tank's helm as the pixel view drives it.
#[derive(Clone, Debug)]
pub struct Helm {
    /// `-1` turning left, `0` not, `1` turning right.
    turning: i8,
    /// The heading last ordered, and when it was first ordered.
    ordered: Option<(i64, Instant)>,
    /// The order before the standing one, within the turn in hand: one the
    /// arena has certainly had by the time a frame arrives.
    prior: Option<i64>,
    seen: Option<Seen>,
    /// The picture's heading at an instant. Every prediction runs on from
    /// here, and every event (a frame, a press, a release, a dropped turn)
    /// moves it to where the picture stood, so the picture never jumps.
    shown: Option<(i64, Instant)>,
    /// The picture's heading when the turn in hand began, and when.
    turn_from: Option<(i64, Instant)>,
    /// True once consecutive arena pulses (`prev.t_ms < t_ms`) have been observed.
    live_stream: bool,
    rate: i64,
    /// True once a frame has carried the unit's turn rate: nothing is
    /// measured after.
    rate_read: bool,
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
            prior: None,
            seen: None,
            shown: None,
            turn_from: None,
            live_stream: false,
            rate: DEFAULT_TURN_RATE,
            rate_read: false,
            tempo_ms: 1_000,
            orders: Vec::new(),
        }
    }

    /// Which way the pilot is turning the tank: `-1`, `0`, `1`.
    #[must_use]
    pub fn turning(&self) -> i8 {
        self.turning
    }

    /// The heading last ordered and not yet reached, millidegrees. While a
    /// turn is held it is the running lead, a few degrees ahead of the
    /// picture, and not a heading the pilot chose.
    #[must_use]
    pub fn ordered(&self) -> Option<i64> {
        self.ordered.map(|(o, _)| o)
    }

    /// The turn rate in use, millidegrees a second.
    #[must_use]
    pub fn rate(&self) -> i64 {
        self.rate
    }

    /// Read the held cockpit frame. A new one (by its arena time) is the
    /// truth about the tank, and the picture carries on from where it stood:
    /// a turn in hand keeps swinging, a released one keeps its heading, and
    /// anything else is followed at the turn rate. The frame also gives the
    /// turn rate (or lets it be measured), and a held turn it does not bear
    /// out is dropped.
    pub fn observe(&mut self, cockpit: &Value, now: Instant) {
        let Some(t_ms) = cockpit.get("t_ms").and_then(Value::as_i64) else {
            return;
        };
        let prev = self.seen;
        if prev.is_some_and(|p| p.t_ms == t_ms) {
            return;
        }
        let heading = cockpit
            .get("heading_mdeg")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .rem_euclid(FULL);
        let speed_cms = cockpit
            .get("speed_cms")
            .and_then(Value::as_i64)
            .unwrap_or_else(|| {
                cockpit
                    .get("speed_kph")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
                    * 1_000
                    / 36
            });
        let can_turn = cockpit.get("wreck").and_then(Value::as_bool) != Some(true)
            && cockpit.get("immobile").and_then(Value::as_bool) != Some(true)
            && !matches!(
                cockpit.get("power").and_then(Value::as_str),
                Some("off" | "starting")
            );
        let can_move = can_turn && cockpit.get("blocked").and_then(Value::as_bool) != Some(true);
        // Where the picture stands, by the old frame, rate and tempo: the
        // new ones change where it goes from here, never where it is.
        let stands = self.predicted(now);
        if let Some(ms) = cockpit
            .get("tempo")
            .and_then(Value::as_str)
            .and_then(tempo_ms)
        {
            self.tempo_ms = ms;
        }
        if let Some(rate) = cockpit
            .get("turn_rate_mdeg_s")
            .and_then(Value::as_i64)
            .filter(|rate| *rate > 0)
        {
            self.rate = rate.clamp(RATE_BAND.0, RATE_BAND.1);
            self.rate_read = true;
        }
        let mut carried = None;
        if let Some(prev) = prev {
            let dt = t_ms - prev.t_ms;
            self.live_stream = dt > 0 && dt <= 10_000;
            if dt > 0 {
                carried = stands;
                self.judge(prev, heading, dt, now);
            }
        }
        // The first frame, and the first after the arena's clock went back
        // (another match, a replay), is shown as it is.
        self.shown = Some((carried.unwrap_or(heading), now));
        self.seen = Some(Seen {
            t_ms,
            heading,
            speed_cms,
            can_move,
            can_turn,
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

    /// What the frame after `prev` says of the helm's own order: `dt` arena
    /// milliseconds on, the tank is at `heading`.
    fn judge(&mut self, prev: Seen, heading: i64, dt: i64, now: Instant) {
        if self.turning == 0 {
            // A released order the tank is no longer closing on, a second
            // past its pulse, was not taken.
            if let Some((o, at)) = self.ordered
                && now.saturating_duration_since(at) > self.grace()
                && arc(heading, o).abs() >= arc(prev.heading, o).abs()
            {
                self.ordered = None;
            }
            return;
        }
        let dir = i64::from(self.turning);
        // Only a pulse the turn was in hand for from end to end, and that
        // left the tank short of an order the arena certainly had, says
        // anything: the sim spent the whole of it turning, or would not turn.
        let whole = self.turn_from.is_some_and(|(_, at)| at <= prev.at);
        let short = self.prior.is_some_and(|o| dir * arc(heading, o) > REACHED);
        if !(whole && short) {
            return;
        }
        let turned = dir * arc(prev.heading, heading);
        if turned <= 0 {
            // Ordered ahead for a whole pulse and no nearer: refused, or a
            // hull that cannot turn that way.
            self.turning = 0;
            self.turn_from = None;
            self.ordered = None;
            self.prior = None;
        } else if !self.rate_read && dt <= self.tempo_ms + self.tempo_ms / 2 {
            self.rate = (turned * 1_000 / dt).clamp(RATE_BAND.0, RATE_BAND.1);
        }
    }

    /// The heading the picture shows at `now`: continuous in time, and
    /// carried across every frame, press and release from where it stood.
    /// A held turn swings on at the turn rate, no further ahead of the last
    /// frame than the leash; a released one closes on the heading ordered at
    /// the release and holds it; with no order of its own the picture follows
    /// the last frame's heading, at the turn rate or in a pulse, whichever is
    /// sooner. `None` before any frame.
    #[must_use]
    pub fn predicted(&self, now: Instant) -> Option<i64> {
        let seen = self.seen?;
        let (v0, t0) = self.shown.unwrap_or((seen.heading, seen.at));
        let ms = ms_between(t0, now);
        let v = if self.turning != 0
            && let Some((_, from)) = self.turn_from
        {
            let dir = i64::from(self.turning);
            let swung = if seen.can_turn {
                self.swing(ms_between(from, now)) - self.swing(ms_between(from, t0))
            } else {
                0
            };
            let room = (self.leash() - dir * arc(seen.heading, v0)).max(0);
            v0 + dir * swung.min(room)
        } else if let Some((o, _)) = self.ordered {
            let run = if seen.can_turn {
                self.rate * ms / 1_000
            } else {
                0
            };
            let d = arc(v0, o);
            v0 + d.signum() * d.abs().min(run)
        } else {
            let d = arc(v0, seen.heading);
            let in_a_pulse = d.abs() * ms / self.tempo_ms.max(1);
            v0 + d.signum() * d.abs().min((self.rate * ms / 1_000).max(in_a_pulse))
        };
        Some(v.rem_euclid(FULL))
    }

    /// Dead-reckoned `(dx_cm, dy_cm)` position offset from the last observed
    /// keyframe's `(x_cm, y_cm)` at `now`, integrated along the predicted hull
    /// heading for at most a pulse and a half while a live pulse stream or helm
    /// order is active.
    #[must_use]
    pub fn predicted_offset_cm(&self, now: Instant) -> (i64, i64) {
        let Some(seen) = self.seen else {
            return (0, 0);
        };
        if !seen.can_move
            || seen.speed_cms == 0
            || (!self.live_stream && self.ordered.is_none() && self.turning == 0)
        {
            return (0, 0);
        }
        let pulse = self.pulse();
        let elapsed_ms = i64::try_from(
            now.saturating_duration_since(seen.at)
                .min(pulse + pulse / 2)
                .as_millis(),
        )
        .unwrap_or(0);
        if elapsed_ms <= 0 {
            return (0, 0);
        }
        let hdg_mdeg = self.predicted(now).unwrap_or(seen.heading);
        let rad = ((hdg_mdeg as f64) / 1_000.0).to_radians();
        let dist_cm = (seen.speed_cms as f64) * (elapsed_ms as f64) / 1_000.0;
        let dx_cm = (dist_cm * rad.sin()).round() as i64;
        let dy_cm = (dist_cm * rad.cos()).round() as i64;
        (dx_cm, dy_cm)
    }

    /// The sim's pulse at the frame's tempo.
    fn pulse(&self) -> Duration {
        Duration::from_millis(u64::try_from(self.tempo_ms).unwrap_or(1_000))
    }

    /// How long an order may go unanswered: its pulse and a second.
    fn grace(&self) -> Duration {
        self.pulse() + Duration::from_secs(1)
    }

    /// How far a turn held for `ms` has swung the picture: [`TAP`] over the
    /// first [`TAP_MS`] (or the turn rate, if that is slower), then the turn
    /// rate. The slow start is what makes a tap one degree and no more.
    fn swing(&self, ms: i64) -> i64 {
        let slow = ms.clamp(0, TAP_MS);
        let fast = (ms - slow).max(0);
        (TAP * slow / TAP_MS).min(self.rate * slow / 1_000) + self.rate * fast / 1_000
    }

    /// How far ahead of the picture a turn held for `ms` orders: what the
    /// picture swings in the next [`ORDER_GAP`], when the next order goes. A
    /// few degrees (7.5° on the stock tank), so a pulse that resolves between
    /// two orders leaves the tank that little ahead of the picture, and the
    /// release brings it back no further.
    fn lead(&self, ms: i64) -> i64 {
        let gap = i64::try_from(ORDER_GAP.as_millis()).unwrap_or(250);
        (self.swing(ms + gap) - self.swing(ms)).clamp(LEAD_BAND.0, LEAD_BAND.1)
    }

    /// The furthest the picture runs ahead of the last frame: a pulse and a
    /// half of turning, inside [`LEASH_BAND`].
    fn leash(&self) -> i64 {
        (self.rate * (self.tempo_ms + self.tempo_ms / 2) / 1_000).clamp(LEASH_BAND.0, LEASH_BAND.1)
    }

    /// Order `heading`, to the whole degree — unless that degree is the
    /// order already standing.
    fn order(&mut self, heading: i64, now: Instant) {
        let deg = (heading.rem_euclid(FULL) + 500) / 1_000 % 360;
        if self.ordered() == Some(deg * 1_000) {
            return;
        }
        self.orders.push(format!("HEADING {deg} QUIET"));
        self.ordered = Some((deg * 1_000, now));
    }

    /// Start turning the tank `dir` (`-1` left, `1` right), from the heading
    /// the picture shows, and order the tap's one degree: let go at once and
    /// that order stands. Nothing before a frame has been seen: there is no
    /// heading to turn from.
    pub fn start(&mut self, dir: i8, now: Instant) {
        let dir = dir.signum();
        let Some(p) = self.predicted(now) else {
            return;
        };
        if dir == 0 {
            return;
        }
        self.shown = Some((p, now));
        self.turn_from = Some((p, now));
        self.turning = dir;
        self.order(p + i64::from(dir) * TAP, now);
        self.prior = None;
    }

    /// Stop turning: order the heading the picture shows, which the picture
    /// then holds. A press let go inside its first degree is a tap, and
    /// turns the tank that one degree.
    pub fn stop(&mut self, now: Instant) {
        if self.turning == 0 {
            return;
        }
        let dir = i64::from(self.turning);
        let stands = self.predicted(now);
        self.turning = 0;
        self.prior = None;
        let from = self.turn_from.take();
        let Some(p) = stands else {
            return;
        };
        self.shown = Some((p, now));
        let from = from.map_or(p, |(heading, _)| heading);
        let target = if dir * arc(from, p) < TAP {
            from + dir * TAP
        } else {
            p
        };
        self.order(target, now);
    }

    /// While a turn is held, order on ahead of the picture every
    /// [`ORDER_GAP`]. A turn or a released order no frame has spoken to for
    /// a second past its pulse was not taken: it is dropped, and the picture
    /// goes back to the frames.
    pub fn tick(&mut self, now: Instant) {
        let Some(seen) = self.seen else {
            return;
        };
        let since = |at: Instant| now.saturating_duration_since(at);
        let grace = self.grace();
        let silent = since(seen.at) > grace;
        if self.turning == 0 {
            if silent && self.ordered.is_some_and(|(_, at)| since(at) > grace) {
                self.shown = self.predicted(now).map(|p| (p, now));
                self.ordered = None;
            }
            return;
        }
        let (Some(p), Some((_, from))) = (self.predicted(now), self.turn_from) else {
            return;
        };
        if silent && since(from) > grace {
            self.shown = Some((p, now));
            self.turning = 0;
            self.turn_from = None;
            self.ordered = None;
            self.prior = None;
            return;
        }
        if self.ordered.is_none_or(|(_, at)| since(at) >= ORDER_GAP) {
            let lead = self.lead(ms_between(from, now));
            let was = self.ordered();
            self.order(p + i64::from(self.turning) * lead, now);
            if self.ordered() != was {
                self.prior = was;
            }
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
        self.orders.push(format!("SPEED {want} QUIET"));
    }

    /// The `SPEED` lines to send, oldest first.
    pub fn take_orders(&mut self) -> Vec<String> {
        std::mem::take(&mut self.orders)
    }
}

// ---- the lock cycle (doc 21 cut G.12b) ---------------------------------------

/// **The lock cycle's order** (doc 21 §7.3): the frame's contacts by
/// designator, by the viewer's `relation` (doc 32 cut CF.0): hostiles first,
/// then the unidentified (`unknown`, or a row without the word), then
/// neutrals, then own and friends, nearest first within each, the designator
/// breaking a tie.
#[must_use]
pub fn lock_order(cockpit: &Value) -> Vec<String> {
    let mut rows: Vec<(u8, i64, String)> = cockpit
        .get("contacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let name = c.get("designator").and_then(Value::as_str)?.to_string();
            // Doc 32 cut CF.0 (Q-CF-12): ranked by the viewer's relation.
            let rank = match c.get("relation").and_then(Value::as_str) {
                Some("hostile") => 0,
                Some("neutral") => 2,
                Some("own" | "friend") => 3,
                _ => 1,
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

    /// A frame that carries the unit's turn rate, as the arena's do.
    fn rated(t_ms: i64, heading_deg: i64, tempo: &str, rate: i64) -> Value {
        serde_json::json!({
            "t_ms": t_ms,
            "heading_mdeg": heading_deg * 1_000,
            "tempo": tempo,
            "turn_rate_mdeg_s": rate,
        })
    }

    /// The arena as the helm meets it: the standing order is resolved at the
    /// pulse with the whole pulse of turning, whenever in the pulse it came,
    /// on the short arc; and a frame goes out when the heading moved. Its
    /// frames carry no turn rate, as a replay's do not.
    struct Arena {
        heading: i64,
        order: Option<i64>,
        rate: i64,
        tempo: &'static str,
        t_ms: i64,
    }

    impl Arena {
        fn new(heading_deg: i64, rate: i64, tempo: &'static str) -> Arena {
            Arena {
                heading: heading_deg * 1_000,
                order: None,
                rate,
                tempo,
                t_ms: 1_000,
            }
        }

        fn pulse_ms(&self) -> u64 {
            tempo_ms(self.tempo)
                .and_then(|ms| u64::try_from(ms).ok())
                .unwrap_or(1_000)
        }

        fn frame(&self) -> Value {
            serde_json::json!({
                "t_ms": self.t_ms,
                "heading_mdeg": self.heading,
                "tempo": self.tempo,
            })
        }

        /// One pulse: the frame it sends, if the heading moved.
        fn pulse(&mut self) -> Option<Value> {
            let pulse = tempo_ms(self.tempo).unwrap_or(1_000);
            self.t_ms += pulse;
            let before = self.heading;
            if let Some(order) = self.order {
                let d = arc(self.heading, order);
                let step = d.abs().min(self.rate * pulse / 1_000);
                self.heading = (self.heading + d.signum() * step).rem_euclid(FULL);
            }
            (self.heading != before).then(|| self.frame())
        }

        /// Take the helm's lines as the arena's command queue does.
        fn take(&mut self, lines: Vec<String>) {
            for line in lines {
                let deg = line
                    .strip_prefix("HEADING ")
                    .and_then(|rest| rest.strip_suffix(" QUIET"))
                    .and_then(|deg| deg.parse::<i64>().ok());
                assert!(deg.is_some(), "not a quiet heading order: {line}");
                self.order = deg.map(|deg| deg * 1_000);
            }
        }
    }

    /// Paint every 50 ms for `total` ms against the arena, the turn key down
    /// at `press` and up at `release`: the heading the picture showed at each
    /// paint, the first at 0 ms.
    fn drive(
        arena: &mut Arena,
        h: &mut Helm,
        t0: Instant,
        press: u64,
        release: u64,
        total: u64,
    ) -> Vec<i64> {
        let mut shown = Vec::new();
        for at in (0..=total).step_by(50) {
            let now = t0 + ms(at);
            if at > 0
                && at.is_multiple_of(arena.pulse_ms())
                && let Some(frame) = arena.pulse()
            {
                h.observe(&frame, now);
            }
            if at == press {
                h.start(1, now);
            }
            if at == release {
                h.stop(now);
            }
            h.tick(now);
            arena.take(h.take_orders());
            shown.push(h.predicted(now).unwrap_or(-1));
        }
        shown
    }

    /// A held turn swings the picture from where it stood at the press — a
    /// slow first fifth of a second, then the turn rate — and letting go
    /// orders the heading the picture shows, which it then holds.
    #[test]
    fn a_turn_swings_from_the_press_and_a_release_orders_the_shown_heading() {
        let t0 = Instant::now();
        let mut h = Helm::new();
        h.observe(&frame(10_000, 90, "alert"), t0);
        assert_eq!(h.predicted(t0), Some(90_000));
        h.start(1, t0 + ms(200));
        // The press orders the tap's degree, not a pulse and a half of turn.
        assert_eq!(h.take_orders(), ["HEADING 91 QUIET"]);
        assert_eq!(
            h.predicted(t0 + ms(200)),
            Some(90_000),
            "moves from the press"
        );
        assert_eq!(h.predicted(t0 + ms(400)), Some(91_000), "the slow start");
        assert_eq!(h.predicted(t0 + ms(700)), Some(100_000), "then 30°/s");
        // The first order of the hold: where the picture will be a gap on.
        h.tick(t0 + ms(450));
        assert_eq!(h.take_orders(), ["HEADING 100 QUIET"]);
        h.stop(t0 + ms(800));
        assert_eq!(h.take_orders(), ["HEADING 103 QUIET"]);
        assert_eq!(h.turning(), 0);
        // A frame short of the order does not pull the picture back to it…
        h.observe(&frame(11_000, 100, "alert"), t0 + ms(1_000));
        assert_eq!(h.predicted(t0 + ms(1_000)), Some(103_000));
        assert_eq!(h.predicted(t0 + ms(1_900)), Some(103_000));
        assert_eq!(h.ordered(), Some(103_000));
        // …and the frame at the ordered heading closes the order.
        h.observe(&frame(12_000, 103, "alert"), t0 + ms(2_000));
        assert_eq!(h.ordered(), None);
        assert_eq!(h.predicted(t0 + ms(5_000)), Some(103_000));
    }

    /// A tap is one degree and one line, however short the press, so a pilot
    /// walks the last degrees to a heading a tap at a time.
    #[test]
    fn a_tap_turns_the_tank_one_degree() {
        let t0 = Instant::now();
        let mut h = Helm::new();
        h.observe(&frame(1_000, 87, "alert"), t0);
        h.start(1, t0 + ms(1_000));
        h.stop(t0 + ms(1_030));
        assert_eq!(h.take_orders(), ["HEADING 88 QUIET"], "one line a tap");
        assert_eq!(h.predicted(t0 + ms(1_200)), Some(88_000));
        // A slower tap is the same degree, and two taps are two degrees.
        h.start(1, t0 + ms(2_000));
        h.stop(t0 + ms(2_180));
        assert_eq!(h.take_orders(), ["HEADING 89 QUIET"]);
        assert_eq!(h.predicted(t0 + ms(2_400)), Some(89_000));
        // Left, across north.
        let mut h = Helm::new();
        h.observe(&frame(1_000, 0, "quiet"), t0);
        h.start(-1, t0);
        h.stop(t0 + ms(90));
        assert_eq!(h.take_orders(), ["HEADING 359 QUIET"]);
        assert_eq!(h.predicted(t0 + ms(400)), Some(359_000));
    }

    /// While held, the helm orders every gap, and each order is the heading
    /// the picture will show one gap on: the lead is a few degrees, not a
    /// pulse and a half of turn.
    #[test]
    fn a_held_turn_orders_where_the_picture_will_be_a_gap_on() {
        let t0 = Instant::now();
        let mut h = Helm::new();
        h.observe(&frame(1_000, 90, "alert"), t0);
        h.start(1, t0);
        assert_eq!(h.take_orders(), ["HEADING 91 QUIET"]);
        let mut sent = 0;
        for at in (50..=1_000).step_by(50) {
            let now = t0 + ms(at);
            h.tick(now);
            let orders = h.take_orders();
            if at.is_multiple_of(250) {
                let ahead = h.predicted(now + ORDER_GAP).unwrap_or(-1);
                let deg = (ahead + 500) / 1_000;
                assert_eq!(orders, [format!("HEADING {deg} QUIET")], "at {at} ms");
                assert!(arc(h.predicted(now).unwrap_or(-1), deg * 1_000) <= 8_000);
                sent += 1;
            } else {
                assert!(orders.is_empty(), "the gap holds at {at} ms: {orders:?}");
            }
        }
        assert_eq!(sent, 4);
    }

    /// The owner's report (2026-11-09): the picture "jerks and moves in wide
    /// swaths". Against an arena that resolves orders a pulse at a time, at
    /// every tempo and wherever in the pulse the key goes down, a three-second
    /// hold is one swing: never back, never faster than the turn rate, still
    /// from the release on, and the tank comes to the heading the picture
    /// stopped at.
    #[test]
    fn a_held_turn_is_one_smooth_swing_whatever_the_pulse() {
        for (tempo, press) in [
            ("engaged", 150),
            ("alert", 700),
            ("alert", 950),
            ("quiet", 700),
            ("quiet", 4_800),
        ] {
            let release = press + 3_000;
            let t0 = Instant::now();
            let mut arena = Arena::new(90, 30_000, tempo);
            let mut h = Helm::new();
            h.observe(&arena.frame(), t0);
            let shown = drive(&mut arena, &mut h, t0, press, release, release + 12_000);
            // One degree in the slow start, then 2.8 s at 30°/s.
            let target = 175_000;
            for (at, pair) in (50_u64..).step_by(50).zip(shown.windows(2)) {
                let step = arc(pair[0], pair[1]);
                assert!(
                    (0..=1_500).contains(&step),
                    "{tempo}/{press}: {step} mdeg in the paint at {at} ms"
                );
                if at <= press {
                    assert_eq!(pair[1], 90_000, "{tempo}/{press}: before the press");
                }
                if at >= release {
                    assert_eq!(pair[1], target, "{tempo}/{press}: still at {at} ms");
                }
            }
            assert_eq!(
                arena.heading, target,
                "{tempo}/{press}: the tank came to it"
            );
            assert_eq!(h.ordered(), None, "{tempo}/{press}");
            assert_eq!(h.rate(), DEFAULT_TURN_RATE, "{tempo}/{press}: no measure");
        }
    }

    /// The frame's `turn_rate_mdeg_s` is the unit's and the picture swings at
    /// it; nothing a frame shows afterwards measures over it.
    #[test]
    fn the_frames_turn_rate_is_the_units() {
        let t0 = Instant::now();
        let mut h = Helm::new();
        h.observe(&rated(1_000, 0, "alert", 60_000), t0);
        assert_eq!(h.rate(), 60_000);
        h.start(1, t0);
        // One degree in the slow start, then half a second at 60°/s.
        assert_eq!(h.predicted(t0 + ms(700)), Some(31_000));
        for at in (50..=950).step_by(50) {
            h.tick(t0 + ms(at));
        }
        // A whole pulse of turning that left the tank ten degrees on and
        // short of its order: a measure, were the rate not the frame's.
        h.observe(&rated(2_000, 10, "alert", 60_000), t0 + ms(1_000));
        assert_eq!(h.turning(), 1);
        assert_eq!(h.rate(), 60_000);
    }

    /// A frame without a rate (a replay, a fixture) leaves the stock 30°/s,
    /// and a hull that turns slower is measured across the first whole pulse
    /// it spends short of its order: the picture waits for it from there, and
    /// the tank still comes to the heading the picture stopped at.
    #[test]
    fn a_slower_hull_is_measured_when_the_frame_carries_no_rate() {
        let t0 = Instant::now();
        let mut arena = Arena::new(90, 20_000, "alert");
        let mut h = Helm::new();
        h.observe(&arena.frame(), t0);
        let shown = drive(&mut arena, &mut h, t0, 100, 2_500, 8_000);
        assert_eq!(h.rate(), 20_000);
        for pair in shown.windows(2) {
            let step = arc(pair[0], pair[1]);
            assert!((0..=1_500).contains(&step), "{pair:?}");
        }
        assert_eq!(arena.heading, 152_000);
        assert_eq!(shown.last().copied(), Some(arena.heading));
    }

    /// A turn the frames do not bear out — a seat without the helm, an order
    /// refused — is dropped, and the picture eases back to the frames.
    #[test]
    fn a_turn_the_frames_do_not_bear_out_is_dropped() {
        // Frames come, and the tank is no nearer its order a whole pulse on.
        let t0 = Instant::now();
        let mut h = Helm::new();
        h.observe(&frame(1_000, 90, "alert"), t0);
        h.start(1, t0 + ms(100));
        for at in (150..=2_000).step_by(50) {
            let now = t0 + ms(at);
            if at == 1_000 {
                h.observe(&frame(2_000, 90, "alert"), now);
                assert_eq!(h.turning(), 1, "the press came inside that pulse");
            }
            if at == 2_000 {
                h.observe(&frame(3_000, 90, "alert"), now);
            }
            h.tick(now);
        }
        assert_eq!(h.turning(), 0);
        assert_eq!(h.ordered(), None);
        let _ = h.take_orders();
        // The picture had run to the leash, a pulse and a half of turning,
        // and goes back to the frame inside a pulse: no snap.
        assert_eq!(h.predicted(t0 + ms(2_000)), Some(135_000));
        assert_eq!(h.predicted(t0 + ms(2_500)), Some(112_500));
        assert_eq!(h.predicted(t0 + ms(3_000)), Some(90_000));
        h.tick(t0 + ms(3_000));
        assert!(h.take_orders().is_empty(), "a dropped turn orders nothing");

        // No frame at all: dropped a second past the pulse.
        let mut h = Helm::new();
        h.observe(&frame(1_000, 90, "alert"), t0);
        h.start(1, t0 + ms(100));
        for at in (150..=2_050).step_by(50) {
            h.tick(t0 + ms(at));
        }
        assert_eq!(h.turning(), 1);
        h.tick(t0 + ms(2_150));
        assert_eq!(h.turning(), 0);
        assert_eq!(h.ordered(), None);
        assert_eq!(h.predicted(t0 + ms(2_150)), Some(135_000));
        assert_eq!(h.predicted(t0 + ms(3_150)), Some(90_000));
    }

    /// A hull the frame says cannot turn — cold, starting, immobile, a wreck
    /// — is not drawn turning. The order still goes: the arena's refusal is
    /// what tells the pilot why.
    #[test]
    fn a_hull_that_cannot_turn_is_not_drawn_turning() {
        for (key, value) in [
            ("power", Value::from("off")),
            ("power", Value::from("starting")),
            ("immobile", Value::from(true)),
            ("wreck", Value::from(true)),
        ] {
            let t0 = Instant::now();
            let mut h = Helm::new();
            let mut cockpit = frame(1_000, 90, "alert");
            cockpit[key] = value;
            h.observe(&cockpit, t0);
            h.start(1, t0);
            assert_eq!(h.take_orders(), ["HEADING 91 QUIET"], "{key}");
            assert_eq!(h.predicted(t0 + ms(1_000)), Some(90_000), "{key}");
            h.stop(t0 + ms(1_000));
            assert_eq!(h.predicted(t0 + ms(1_500)), Some(90_000), "{key}");
        }
    }

    /// A turn that is not the helm's own — a typed `HEADING`, a machine at
    /// the helm — is followed at the turn rate, or inside a pulse when the
    /// hull is faster than the helm knows, and never stepped to.
    #[test]
    fn a_turn_that_is_not_the_helms_is_followed_and_not_stepped() {
        let t0 = Instant::now();
        let mut h = Helm::new();
        h.observe(&frame(1_000, 90, "alert"), t0);
        h.observe(&frame(2_000, 120, "alert"), t0 + ms(1_000));
        assert_eq!(h.predicted(t0 + ms(1_000)), Some(90_000), "no step");
        assert_eq!(h.predicted(t0 + ms(1_500)), Some(105_000));
        assert_eq!(h.predicted(t0 + ms(2_000)), Some(120_000));
        assert_eq!(h.predicted(t0 + ms(9_000)), Some(120_000));
        h.tick(t0 + ms(2_000));
        assert!(h.take_orders().is_empty());
        h.observe(&frame(3_000, 210, "alert"), t0 + ms(2_000));
        assert_eq!(h.predicted(t0 + ms(2_500)), Some(165_000));
        assert_eq!(h.predicted(t0 + ms(3_000)), Some(210_000));
        // The arena's clock going back is another picture: shown as it is.
        h.observe(&frame(500, 10, "alert"), t0 + ms(3_100));
        assert_eq!(h.predicted(t0 + ms(3_100)), Some(10_000));
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
        assert_eq!(t.take_orders(), ["SPEED 6 QUIET"]);
        assert_eq!((t.predicted(), t.ordered()), (6, Some(6)));
        // Two more inside the gap: one order, the last value, when it opens.
        t.bump(1, t0 + ms(50));
        t.bump(1, t0 + ms(100));
        assert!(t.take_orders().is_empty(), "the gap holds");
        assert_eq!(t.predicted(), 18, "but the prediction moves");
        t.tick(t0 + ORDER_GAP);
        assert_eq!(t.take_orders(), ["SPEED 18 QUIET"]);
        // The frame on its way: still the order; reached: the frame's.
        t.observe(&speed(10));
        assert_eq!(t.predicted(), 18);
        t.observe(&speed(17));
        assert_eq!((t.predicted(), t.ordered()), (17, None));
        // All stop goes at once.
        t.stop(t0 + ORDER_GAP + ms(10));
        assert_eq!(t.take_orders(), ["SPEED 0 QUIET"]);
        // A step inside the gap goes out on a flush (the view hiding).
        t.bump(1, t0 + ORDER_GAP + ms(20));
        assert!(t.take_orders().is_empty());
        t.flush(t0 + ORDER_GAP + ms(30));
        assert_eq!(t.take_orders(), ["SPEED 6 QUIET"]);
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
        assert_eq!(t.take_orders(), ["SPEED 43 QUIET"], "clamped at the top");
        let mut later = t0;
        for want in ["SPEED 38 QUIET", "SPEED 33 QUIET"] {
            later += ORDER_GAP;
            t.bump(-1, later);
            assert_eq!(t.take_orders(), [want]);
        }
        let mut r = Throttle::new();
        r.observe(&serde_json::json!({ "speed_kph": 0, "max_speed_kph": 43, "reverse_kph": 12 }));
        r.bump(-1, t0);
        r.bump(-1, t0 + ORDER_GAP);
        r.bump(-1, t0 + ORDER_GAP * 2);
        assert_eq!(
            r.take_orders(),
            ["SPEED -5 QUIET", "SPEED -10 QUIET", "SPEED -12 QUIET"]
        );
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

    #[test]
    fn helm_dead_reckons_position_along_predicted_heading_between_pulses() {
        let t0 = Instant::now();
        let mut h = Helm::new();
        // A lone static fixture frame does not dead-reckon until a second pulse or helm order.
        h.observe(
            &serde_json::json!({
                "t_ms": 1_000,
                "heading_mdeg": 90_000,
                "speed_cms": 1_000,
                "tempo": "alert"
            }),
            t0,
        );
        assert_eq!(h.predicted_offset_cm(t0 + ms(500)), (0, 0));

        // Second pulse activates live 20 Hz dead-reckoning: heading 90° (East) at 10 m/s
        // for 500 ms moves +500 cm East, 0 cm North.
        h.observe(
            &serde_json::json!({
                "t_ms": 2_000,
                "heading_mdeg": 90_000,
                "speed_cms": 1_000,
                "tempo": "alert"
            }),
            t0 + ms(1_000),
        );
        assert_eq!(h.predicted_offset_cm(t0 + ms(1_500)), (500, 0));
    }
}

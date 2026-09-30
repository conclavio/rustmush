//! The pilot (cut AJ.2): a deterministic tactician over the [`Picture`].
//!
//! [`Pilot::decide`] reads the picture and a [`Doctrine`] and says what to type
//! next — a heading, a speed, a lock, the guns to fire, whether to ask where The
//! Crossroads lies. No I/O and no clock: `now_ms` is the runner's session clock,
//! and the lane decides which of the orders go out this tick and tells the pilot
//! ([`Pilot::sent_heading`], [`Pilot::sent_speed`], [`Pilot::sent_lock`]).
//!
//! **Written from the player help alone.** Nothing of the machines' own
//! controller is read, imported or mirrored. Every number below is a sentence
//! of `help simulator` ("Fighting the machines") or `help cockpit` ("Driving",
//! "Seeing", "Shooting"):
//!
//! * a machine "closes: to about seven tenths of what its own sight and its
//!   gun's inner band allow (420 m for the stock tank), where it stops, turns
//!   onto you and shoots" — so the pilot does not stand and trade at that range;
//! * "on grass a stopped tank can be seen out to 80 per cent of that (480 m)
//!   and a moving one out to all of it" (600 m) — so the orbit's default radius,
//!   **450 m**, sits just outside the stand-off and inside the stopped-tank
//!   sight: the pilot keeps a stopped machine in view while making it turn;
//! * "a side is twice the target, 7 m of flank against 3.5 m of bow", and "one
//!   caught from behind spends its thinnest" plate — so the orbit circles toward
//!   the target's **rear** when the picture holds a track on it;
//! * a machine "drives at where it last saw you", and a contact "is drawn where
//!   it was last seen" — so without a track the orbit **reverses** every 20 s,
//!   which makes the machine's plot of the pilot go stale;
//! * "`SPEED` says both when you ask for more than it has" — so the first speed
//!   order is `999`, which reads the tank's own ceilings; until they are known
//!   "full ahead" is 40 kph, under the stock tank's 41 on grass;
//! * "Three seconds of holding a return is enough to tell it apart" whatever is
//!   locked — so an unidentified contact is never locked, and never fired on;
//! * the `% to hit` is printed only for the lock and the band word is the
//!   scope's ("It is **not** a judgement of your gun") — so the target is the
//!   nearest hostile with a fresh plot, and a held lock is kept unless another
//!   is at least 100 m nearer;
//! * "Past a gun's last band the chance is 0%: the round is still fired and
//!   spent" — so a gun whose last read said 0% is not fired;
//! * "Ground a tank cannot enter stops it, and the cockpit says so once per
//!   stop" — so a `Held:` notice turns the tank 45° and holds that for 3 s;
//! * "A machine that loses you … crosses to the other side's entry zone to
//!   look", and the proving ground's roads meet at The Crossroads (B) — so with
//!   no identified hostile held for 5 s (an unidentified contact does not
//!   count) the pilot drives for `B`, past it 800 m, and back.
//!
//! The pilot's own rule above any doctrine: hurt to half its hull's internals,
//! or with a struck facet at a quarter of its plate, it withdraws for 10 s and
//! then orbits 150 m wider (at most 600 m, the stock sight) for the rest of the
//! match.
use serde::Serialize;

use super::{
    Side,
    picture::{Contact, EventKind, Phase, Picture, Power, bearing_deg, distance_m},
};

/// "Full ahead" before the tank's own ceilings are read.
pub const UNKNOWN_FULL_KPH: i32 = 40;
/// The speed that asks the tank for its ceilings.
pub const CEILING_ASK_KPH: i32 = 999;
/// A plot older than this is not a fresh one.
pub const FRESH_PLOT_MS: u64 = 3_000;
/// …and a row read longer ago than this is not a current reading of it.
pub const FRESH_READ_MS: u64 = 2_000;
/// A held lock is kept unless another hostile is at least this much nearer.
pub const RETARGET_NEARER_M: u32 = 100;
/// The least time between two `LOCK` orders.
pub const LOCK_EVERY_MS: u64 = 1_500;
/// A `ready` read this soon after an ack may be the pre-shot frame.
pub const READY_AFTER_ACK_MS: u64 = 1_500;
/// The orbit's radial correction: at most this many degrees off the tangent…
pub const ORBIT_CORRECTION_DEG: f64 = 60.0;
/// …reached this many metres off the band.
pub const ORBIT_CORRECTION_SPAN_M: f64 = 150.0;
/// `Close` drives straight in to this range, then orbits there.
pub const CLOSE_M: f64 = 200.0;
/// `StandOff` holds within this of its band.
pub const STAND_OFF_SLACK_M: f64 = 50.0;
/// The orbit reverses after this long without a track to steer by.
pub const FLIP_EVERY_MS: u64 = 20_000;
/// A track is advanced by its plot's age, at most this far.
pub const TRACK_LEAD_MAX_MS: u64 = 3_000;
/// Falling back: the withdrawal's length…
pub const WITHDRAW_MS: u64 = 10_000;
/// …the band's widening after it…
pub const FALL_BACK_WIDEN_M: u32 = 150;
/// …and the widest band it widens to (the stock sight).
pub const FALL_BACK_MAX_M: u32 = 600;
/// A `Held:` notice turns the tank this far…
pub const HELD_TURN_DEG: f64 = 45.0;
/// …and holds the new heading this long.
pub const HELD_FOR_MS: u64 = 3_000;
/// With no hostile held this long the pilot searches.
pub const SEARCH_AFTER_MS: u64 = 5_000;
/// Within this of The Crossroads the search drives on past it…
pub const SEARCH_ARRIVE_M: f64 = 100.0;
/// …this far, then turns back.
pub const SEARCH_BEYOND_M: f64 = 800.0;
/// A heading is re-ordered only when it differs by this much…
pub const HEADING_STEP_DEG: u32 = 8;
/// …or by any amount after this long.
pub const HEADING_REFRESH_MS: u64 = 4_000;
/// The spot the search drives for (`help simulator`, "The ground").
pub const SEARCH_SPOT: char = 'B';

// The doctrine the pilot flies moved to the commander's module (cut AJ.3),
// which parses and clamps the model's answer into it; these names stay here.
pub use super::commander::{Doctrine, Sense, Stance};

/// What to type this tick; everything optional.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Orders {
    pub heading: Option<u32>,
    pub speed: Option<i32>,
    pub lock: Option<String>,
    /// Full weapon ids, main gun first.
    pub fire: Vec<String>,
    /// Ask `BEARING B` (the lane paces it).
    pub ask_spot_b: bool,
}

/// The search's leg.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum SearchLeg {
    /// Driving for The Crossroads.
    ToB,
    /// Past it, on a fixed heading, from `origin`.
    Beyond {
        origin: (f64, f64),
        heading_deg: f64,
    },
}

/// The pilot's own state. One per match.
#[derive(Clone, Debug, Serialize)]
pub struct Pilot {
    /// `+1` counter-clockwise round the target, `-1` clockwise.
    pub sense: i8,
    /// When the current stretch of orbiting began.
    pub orbit_since_ms: Option<u64>,
    /// When the sense last reversed (or the orbit began).
    pub last_flip_ms: u64,
    /// Withdrawing until then (falling back).
    pub withdraw_until_ms: Option<u64>,
    /// The pilot has fallen back; its band is widened for the match.
    pub fallen_back: bool,
    /// Holding `held_heading` until then after a `Held:` notice.
    pub held_until_ms: u64,
    pub held_heading: Option<u32>,
    /// The newest `Held:` event already turned for.
    pub held_seen_ms: u64,
    pub search: SearchLeg,
    /// The own position at `Power online.` (the first `STATUS` after it).
    pub start_pos: Option<(f64, f64)>,
    /// Where The Crossroads is, once a `STATUS` has named it nearest
    /// ([`super::picture::Spot::plotted_m`]). The search steers by it, which is
    /// the latest `BEARING B` answer made current: the answer is up to 5 s old,
    /// the plotted spot is where the ground is.
    pub spot_b_pos: Option<(f64, f64)>,
    /// When power came online, as the pilot first saw it.
    pub power_ms: Option<u64>,
    /// The last time a hostile contact was held.
    pub last_hostile_ms: Option<u64>,
    /// The last heading ordered, and when.
    pub last_heading_order: Option<(u32, u64)>,
    pub last_speed_order: Option<i32>,
    pub last_lock_order_ms: Option<u64>,
    /// `SPEED 999` has gone out (or the ceilings were known).
    pub ceilings_asked: bool,
}

impl Default for Pilot {
    fn default() -> Self {
        Pilot {
            sense: 1,
            orbit_since_ms: None,
            last_flip_ms: 0,
            withdraw_until_ms: None,
            fallen_back: false,
            held_until_ms: 0,
            held_heading: None,
            held_seen_ms: 0,
            search: SearchLeg::ToB,
            start_pos: None,
            spot_b_pos: None,
            power_ms: None,
            last_hostile_ms: None,
            last_heading_order: None,
            last_speed_order: None,
            last_lock_order_ms: None,
            ceilings_asked: false,
        }
    }
}

/// Degrees into `0..360`.
fn norm(deg: f64) -> f64 {
    deg.rem_euclid(360.0)
}

/// A heading as `HEADING` takes it.
fn whole(deg: f64) -> u32 {
    // `norm` is in 0..360, so the rounded value is 0..=360 and fits.
    (norm(deg).round() as u32) % 360
}

/// The smaller angle between two whole headings.
fn angle_between(a: u32, b: u32) -> u32 {
    let d = a.abs_diff(b) % 360;
    d.min(360 - d)
}

/// `pos` moved `metres` along `bearing`.
fn along(pos: (f64, f64), bearing: f64, metres: f64) -> (f64, f64) {
    let r = bearing.to_radians();
    (pos.0 + metres * r.sin(), pos.1 + metres * r.cos())
}

/// Whether a weapon is believed ready now.
fn believed_ready(weapon: &super::picture::Weapon, now_ms: u64) -> bool {
    let read_ready = weapon.ready
        && weapon.read_ms > 0
        && weapon
            .fired_ms
            .is_none_or(|fired| weapon.read_ms > fired.saturating_add(READY_AFTER_ACK_MS));
    let due = weapon.next_ready_ms.is_some_and(|at| now_ms >= at);
    let cycled = match (weapon.cycle_ms, weapon.fired_ms) {
        (Some(cycle), Some(fired)) => now_ms >= fired.saturating_add(cycle),
        _ => false,
    };
    read_ready || due || cycled
}

impl Pilot {
    pub fn new() -> Pilot {
        Pilot::default()
    }

    /// **Decide.** What to type now, given the picture and the doctrine.
    pub fn decide(&mut self, picture: &Picture, doctrine: &Doctrine, now_ms: u64) -> Orders {
        let mut orders = Orders::default();
        // Rule 1: nothing unless flying a powered tank in a running match, and
        // nothing ever once it is destroyed.
        if picture.own.destroyed
            || !matches!(picture.phase, Phase::Running { .. })
            || !picture.aboard
            || picture.own.power != Power::Online
        {
            return orders;
        }
        let Some(side) = picture.side else {
            return orders;
        };
        let other = side.other();
        // The doctrine's own sense, when it names one, is `s` everywhere — the
        // orbit and a `Held:` turn alike; `Auto` keeps the pilot's running one.
        match doctrine.sense {
            Sense::Cw => self.sense = -1,
            Sense::Ccw => self.sense = 1,
            Sense::Auto => {}
        }
        self.observe(picture, other, now_ms);

        // Rule 3: the target.
        let target = self.target(picture, doctrine, other);

        // Rule 4: the lock.
        if let Some(target) = &target
            && picture.lock.as_deref() != Some(target.as_str())
            && self
                .last_lock_order_ms
                .is_none_or(|at| now_ms >= at.saturating_add(LOCK_EVERY_MS))
        {
            orders.lock = Some(target.clone());
        }

        // Rule 5: fire.
        if let Some(target) = &target {
            orders.fire = fireable(picture, target, other, now_ms);
        }

        // Rules 2 and 6: the speed ceilings, then movement.
        if picture.own.immobile || picture.own.disabled {
            return orders;
        }
        if picture.own.max_ahead_kph.is_some() {
            self.ceilings_asked = true;
        }
        let full = picture.own.max_ahead_kph.unwrap_or(UNKNOWN_FULL_KPH);
        let motion = self.motion(picture, doctrine, target.as_deref(), full, now_ms);
        let (heading, mut speed, ask) = motion;
        orders.ask_spot_b = ask;
        if !self.ceilings_asked {
            speed = Some(CEILING_ASK_KPH);
        }
        if let Some(deg) = heading {
            let deg = whole(deg);
            let due = match self.last_heading_order {
                None => true,
                Some((last, at)) => {
                    let diff = angle_between(deg, last);
                    diff >= HEADING_STEP_DEG
                        || (diff > 0 && now_ms >= at.saturating_add(HEADING_REFRESH_MS))
                }
            };
            if due {
                orders.heading = Some(deg);
            }
        }
        if let Some(kph) = speed
            && self.last_speed_order != Some(kph)
        {
            orders.speed = Some(kph);
        }
        orders
    }

    /// The lane typed `HEADING deg`.
    pub fn sent_heading(&mut self, deg: u32, now_ms: u64) {
        self.last_heading_order = Some((deg, now_ms));
    }

    /// The lane typed `SPEED kph`.
    pub fn sent_speed(&mut self, kph: i32) {
        self.last_speed_order = Some(kph);
        if kph == CEILING_ASK_KPH {
            self.ceilings_asked = true;
        }
    }

    /// The lane typed a `LOCK`.
    pub fn sent_lock(&mut self, now_ms: u64) {
        self.last_lock_order_ms = Some(now_ms);
    }

    /// A heading order went unanswered: order it again when re-decided.
    pub fn forget_heading(&mut self) {
        self.last_heading_order = None;
    }

    /// A speed order went unanswered: order it again when re-decided (the
    /// ceilings too, if that is what was asked).
    pub fn forget_speed(&mut self) {
        if self.last_speed_order == Some(CEILING_ASK_KPH) {
            self.ceilings_asked = false;
        }
        self.last_speed_order = None;
    }

    // -- internals -------------------------------------------------------------

    /// Fold in what the picture has learned since the last decision: when power
    /// came up, where the pilot started, where The Crossroads is, whether a
    /// hostile is held, a `Held:` notice, and the fall-back rule.
    fn observe(&mut self, picture: &Picture, other: Side, now_ms: u64) {
        let power_ms = *self.power_ms.get_or_insert(now_ms);
        if self.last_hostile_ms.is_none() {
            self.last_hostile_ms = Some(power_ms);
        }
        if self.held_seen_ms < power_ms {
            self.held_seen_ms = power_ms;
        }
        if self.start_pos.is_none()
            && let Some(pos) = picture.own.pos_m
            && picture.own.pos_read_ms >= power_ms
        {
            self.start_pos = Some(pos);
        }
        // The Crossroads' own position, as the picture plotted it from a
        // `STATUS` block that named it nearest (range and bearing from the
        // block's own position; a `BEARING B` answer never moves it).
        if let Some((x, y)) = picture
            .spots
            .get(&SEARCH_SPOT)
            .and_then(|spot| spot.plotted_m)
        {
            self.spot_b_pos = Some((x as f64, y as f64));
        }
        if picture.contacts.values().any(|c| c.side == Some(other)) {
            self.last_hostile_ms = Some(now_ms);
        }
        if let Some(held) = picture
            .events
            .iter()
            .filter(|event| event.kind == EventKind::Held && event.at_ms > self.held_seen_ms)
            .map(|event| event.at_ms)
            .max()
        {
            self.held_seen_ms = held;
            self.held_heading = Some(whole(
                f64::from(picture.own.heading_deg) + HELD_TURN_DEG * f64::from(self.sense),
            ));
            self.held_until_ms = now_ms.saturating_add(HELD_FOR_MS);
        }
        if !self.fallen_back {
            let hull_half = picture
                .own
                .sections
                .get("hull")
                .and_then(|section| section.internals)
                .is_some_and(|(left, max)| max > 0 && left.saturating_mul(2) <= max);
            let facet_quarter = picture
                .own
                .sections
                .values()
                .flat_map(|section| section.facets.values())
                .any(|(left, max)| *max > 0 && left.saturating_mul(4) <= *max);
            if hull_half || facet_quarter {
                self.fallen_back = true;
                self.withdraw_until_ms = Some(now_ms.saturating_add(WITHDRAW_MS));
            }
        }
    }

    /// Rule 3: the doctrine's target when it is a held hostile; else the nearest
    /// fresh hostile, keeping the lock's contact unless another is at least
    /// 100 m nearer. Never an unidentified contact, never a friend.
    fn target(&self, picture: &Picture, doctrine: &Doctrine, other: Side) -> Option<String> {
        if let Some(wanted) = &doctrine.target
            && picture
                .contacts
                .get(wanted)
                .is_some_and(|c| c.side == Some(other))
        {
            return Some(wanted.clone());
        }
        let best = picture
            .contacts
            .values()
            .filter(|c| c.side == Some(other))
            .min_by_key(|c| (c.age_ms > FRESH_PLOT_MS, c.range_m.unwrap_or(u32::MAX)))?;
        if let Some(lock) = &picture.lock
            && !picture.lock_friendly
            && let Some(held) = picture.contacts.get(lock)
            && held.side == Some(other)
        {
            let held_range = held.range_m.unwrap_or(u32::MAX);
            let best_range = best.range_m.unwrap_or(u32::MAX);
            if best_range.saturating_add(RETARGET_NEARER_M) > held_range {
                return Some(lock.clone());
            }
        }
        Some(best.designator.clone())
    }

    /// Rule 6: `(heading, speed, ask the bearing to B)`.
    fn motion(
        &mut self,
        picture: &Picture,
        doctrine: &Doctrine,
        target: Option<&str>,
        full: i32,
        now_ms: u64,
    ) -> (Option<f64>, Option<i32>, bool) {
        let Some(own) = picture.own_pos_at(now_ms) else {
            return (None, None, false);
        };
        // A `Held:` notice outranks everything for its three seconds.
        if now_ms < self.held_until_ms
            && let Some(held) = self.held_heading
        {
            return (Some(f64::from(held)), Some(full), false);
        }
        let withdrawing = self.withdraw_until_ms.is_some_and(|until| now_ms < until);
        let (stance, band) = if withdrawing {
            (Stance::Withdraw, f64::from(doctrine.band_m))
        } else if self.fallen_back {
            (
                Stance::Orbit,
                f64::from(
                    doctrine
                        .band_m
                        .saturating_add(FALL_BACK_WIDEN_M)
                        .min(FALL_BACK_MAX_M),
                ),
            )
        } else {
            (doctrine.stance, f64::from(doctrine.band_m))
        };
        let contact = target.and_then(|t| picture.contacts.get(t));
        let aim = contact.and_then(|c| target_pos(picture, c, now_ms));
        let (Some(contact), Some(aim)) = (contact, aim) else {
            self.orbit_since_ms = None;
            if withdrawing {
                return (None, Some(full), false);
            }
            let quiet_since = self.last_hostile_ms.unwrap_or(now_ms);
            if target.is_none() && now_ms >= quiet_since.saturating_add(SEARCH_AFTER_MS) {
                return self.search(picture, own, full);
            }
            return (None, None, false);
        };
        // A hostile is held: the next search starts over.
        self.search = SearchLeg::ToB;
        let d = distance_m(own, aim);
        let beta = bearing_deg(own, aim);
        match stance {
            Stance::Orbit => (
                Some(self.orbit(doctrine, contact, aim, own, d, beta, band, now_ms)),
                Some(full),
                false,
            ),
            Stance::Close if d <= CLOSE_M => (
                Some(self.orbit(doctrine, contact, aim, own, d, beta, CLOSE_M, now_ms)),
                Some(full),
                false,
            ),
            Stance::Close => {
                self.orbit_since_ms = None;
                (Some(beta), Some(full), false)
            }
            Stance::StandOff => {
                self.orbit_since_ms = None;
                if d > band + STAND_OFF_SLACK_M {
                    (Some(beta), Some(full), false)
                } else if d < band - STAND_OFF_SLACK_M {
                    (Some(beta + 180.0), Some(full), false)
                } else {
                    (Some(beta), Some(0), false)
                }
            }
            Stance::Withdraw => {
                self.orbit_since_ms = None;
                (Some(beta + 180.0), Some(full), false)
            }
        }
    }

    /// The orbit's heading: the tangent at the band, bent up to 60° toward the
    /// target when outside it and away when inside, reached 150 m off the band.
    #[allow(clippy::too_many_arguments)]
    fn orbit(
        &mut self,
        doctrine: &Doctrine,
        contact: &Contact,
        aim: (f64, f64),
        own: (f64, f64),
        d: f64,
        beta: f64,
        band: f64,
        now_ms: u64,
    ) -> f64 {
        if self.orbit_since_ms.is_none() {
            self.orbit_since_ms = Some(now_ms);
            self.last_flip_ms = now_ms;
        }
        let s = match doctrine.sense {
            Sense::Cw => {
                self.sense = -1;
                -1.0
            }
            Sense::Ccw => {
                self.sense = 1;
                1.0
            }
            Sense::Auto => match &contact.track {
                // Toward the rear: clockwise when the rear lies clockwise of the
                // pilot as the target sees it. No timed flip while a track holds.
                Some(track) => {
                    let theta = norm(track.course_deg + 180.0 - bearing_deg(aim, own));
                    self.sense = if theta > 0.0 && theta < 180.0 { -1 } else { 1 };
                    self.last_flip_ms = now_ms;
                    f64::from(self.sense)
                }
                None => {
                    if now_ms >= self.last_flip_ms.saturating_add(FLIP_EVERY_MS) {
                        self.sense = -self.sense;
                        self.last_flip_ms = now_ms;
                    }
                    f64::from(self.sense)
                }
            },
        };
        let offset = ((d - band) / ORBIT_CORRECTION_SPAN_M).clamp(-1.0, 1.0) * ORBIT_CORRECTION_DEG;
        beta + s * (90.0 - offset)
    }

    /// The search: for The Crossroads, past it 800 m along the start's line
    /// through it, then back.
    fn search(
        &mut self,
        picture: &Picture,
        own: (f64, f64),
        full: i32,
    ) -> (Option<f64>, Option<i32>, bool) {
        if let SearchLeg::Beyond {
            origin,
            heading_deg,
        } = self.search
        {
            if distance_m(origin, own) < SEARCH_BEYOND_M {
                return (Some(heading_deg), Some(full), true);
            }
            self.search = SearchLeg::ToB;
        }
        if let Some(spot) = self.spot_b_pos
            && distance_m(own, spot) <= SEARCH_ARRIVE_M
        {
            let heading = match self.start_pos {
                Some(start) if distance_m(start, spot) > 1.0 => bearing_deg(start, spot),
                _ => f64::from(picture.own.heading_deg),
            };
            self.search = SearchLeg::Beyond {
                origin: own,
                heading_deg: heading,
            };
            return (Some(heading), Some(full), true);
        }
        let heading = match self.spot_b_pos {
            Some(spot) => Some(bearing_deg(own, spot)),
            None => picture
                .spots
                .get(&SEARCH_SPOT)
                .and_then(|spot| spot.bearing_deg)
                .map(f64::from),
        };
        (heading, Some(full), true)
    }
}

/// Where the target is now, as best the picture says: its last plot (a fix, or
/// the row's range and bearing from where the pilot was), advanced along a
/// published track by the plot's age, at most 3 s.
pub fn target_pos(picture: &Picture, contact: &Contact, now_ms: u64) -> Option<(f64, f64)> {
    let plot = match contact.fixes.back() {
        Some(fix) => fix.pos_m,
        None => along(
            picture.own_pos_at(contact.read_ms)?,
            f64::from(contact.bearing_deg),
            f64::from(contact.range_m?),
        ),
    };
    Some(match &contact.track {
        Some(track) => {
            let age_ms = contact
                .age_ms
                .saturating_add(now_ms.saturating_sub(contact.read_ms))
                .min(TRACK_LEAD_MAX_MS);
            along(
                plot,
                track.course_deg,
                track.speed_mps * age_ms as f64 / 1_000.0,
            )
        }
        None => plot,
    })
}

/// Rule 5: the guns to fire at `target`, main gun first.
fn fireable(picture: &Picture, target: &str, other: Side, now_ms: u64) -> Vec<String> {
    if picture.lock.as_deref() != Some(target) || picture.lock_friendly {
        return Vec::new();
    }
    let Some(contact) = picture.contacts.get(target) else {
        return Vec::new();
    };
    if contact.side != Some(other)
        || contact.age_ms > FRESH_PLOT_MS
        || now_ms.saturating_sub(contact.read_ms) > FRESH_READ_MS
    {
        return Vec::new();
    }
    let main = picture
        .weapons
        .iter()
        .position(|w| w.id == "cannon")
        .unwrap_or(0);
    let order = std::iter::once(main).chain((0..picture.weapons.len()).filter(|i| *i != main));
    order
        .filter_map(|index| picture.weapons.get(index))
        .filter(|w| w.rounds > 0 && w.percent != Some(0) && believed_ready(w, now_ms))
        .map(|w| w.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::arena::{
        classify,
        picture::{Event, Fix, Section, Track, Weapon},
        settings::Settings,
    };

    const NOW: u64 = 100_000;

    /// A red tank at the origin, powered and aboard in a running match, heading
    /// north, its ceilings read (54 ahead), `SPEED 999` already out.
    fn flying() -> (Picture, Pilot) {
        let mut picture = Picture {
            phase: Phase::Running { match_id: 1 },
            following: Some(1),
            side: Some(Side::Red),
            aboard: true,
            ..Picture::default()
        };
        picture.own.power = Power::Online;
        picture.own.pos_m = Some((0.0, 0.0));
        picture.own.pos_read_ms = NOW;
        picture.own.max_ahead_kph = Some(54);
        picture.own.max_astern_kph = Some(27);
        let mut pilot = Pilot::new();
        pilot.power_ms = Some(NOW - 10_000);
        pilot.ceilings_asked = true;
        (picture, pilot)
    }

    /// A contact at `bearing`/`range` from the origin, read now, fresh.
    fn contact(designator: &str, side: Option<Side>, bearing: u32, range: u32) -> Contact {
        let pos = along((0.0, 0.0), f64::from(bearing), f64::from(range));
        Contact {
            designator: designator.into(),
            was: None,
            side,
            bearing_deg: bearing,
            rel_deg: bearing,
            band: "far".into(),
            range_m: Some(range),
            age_ms: 0,
            read_ms: NOW,
            fixes: VecDeque::from([Fix {
                at_ms: NOW - 500,
                pos_m: pos,
                sigma_m: 60.0,
                from_lock: false,
            }]),
            track: None,
        }
    }

    fn hold(picture: &mut Picture, contact: Contact) {
        picture.contacts.insert(contact.designator.clone(), contact);
    }

    fn weapon(id: &str, ready: bool) -> Weapon {
        Weapon {
            id: id.into(),
            ready,
            rounds: 10,
            rounds_max: 40,
            read_ms: NOW - 100,
            ..Weapon::default()
        }
    }

    fn doctrine() -> Doctrine {
        Doctrine::pilot_default(&Settings::default())
    }

    fn decide(picture: &Picture, pilot: &mut Pilot) -> Orders {
        pilot.decide(picture, &doctrine(), NOW)
    }

    fn heading(orders: &Orders) -> u32 {
        orders.heading.expect("a heading")
    }

    #[test]
    fn orbit_takes_the_tangent_at_the_band_and_bends_outside_and_inside_it() {
        for (sense, tangent) in [(Sense::Ccw, 90), (Sense::Cw, 270)] {
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
            let doctrine = Doctrine {
                sense,
                ..doctrine()
            };
            let orders = pilot.decide(&picture, &doctrine, NOW);
            assert_eq!(heading(&orders), tangent, "{sense:?} at the band");
            assert_eq!(orders.speed, Some(54), "full ahead");
            // Far: 30° off the bearing, toward it.
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 750));
            let far = heading(&pilot.decide(&picture, &doctrine, NOW));
            assert_eq!(far, if sense == Sense::Ccw { 30 } else { 330 });
            // Near: 150° off it, opening.
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 150));
            let near = heading(&pilot.decide(&picture, &doctrine, NOW));
            assert_eq!(near, if sense == Sense::Ccw { 150 } else { 210 });
            // Half the span off the band (R + 75): half the correction, 30°.
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 525));
            let half = heading(&pilot.decide(&picture, &doctrine, NOW));
            assert_eq!(half, if sense == Sense::Ccw { 60 } else { 300 });
        }
    }

    #[test]
    fn close_stand_off_and_withdraw_headings() {
        let run = |stance: Stance, range: u32| {
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 90, range));
            let doctrine = Doctrine {
                stance,
                sense: Sense::Ccw,
                ..doctrine()
            };
            pilot.decide(&picture, &doctrine, NOW)
        };
        assert_eq!(run(Stance::Close, 600).heading, Some(90));
        assert_eq!(run(Stance::Close, 600).speed, Some(54));
        // Inside 200 m it orbits at 200: at 200 exactly, the tangent.
        assert_eq!(run(Stance::Close, 200).heading, Some(180));
        assert_eq!(run(Stance::StandOff, 700).heading, Some(90));
        assert_eq!(run(Stance::StandOff, 200).heading, Some(270));
        let inside = run(Stance::StandOff, 460);
        assert_eq!((inside.heading, inside.speed), (Some(90), Some(0)));
        let away = run(Stance::Withdraw, 400);
        assert_eq!((away.heading, away.speed), (Some(270), Some(54)));
    }

    #[test]
    fn auto_sense_circles_toward_a_tracked_targets_rear_with_no_timed_flip() {
        // The target is north of the pilot, coursing east (090): its rear points
        // west. Seen from the target, the pilot is due south (180); the rear
        // (270) lies clockwise of it, so the pilot circles clockwise.
        let (mut picture, mut pilot) = flying();
        let mut ba = contact("BA", Some(Side::Blue), 0, 450);
        ba.track = Some(Track {
            course_deg: 90.0,
            speed_mps: 8.0,
        });
        hold(&mut picture, ba);
        let orders = decide(&picture, &mut pilot);
        assert_eq!(pilot.sense, -1, "clockwise, toward the rear");
        // The track is advanced by the plot's age: with age 0 read now the aim is
        // the plot, so the heading is the tangent clockwise, 270 give or take the
        // radial correction.
        let h = heading(&orders);
        assert!(angle_between(h, 270) <= 5, "heading {h}");
        // Coursing west instead: the rear is east, counter-clockwise of south.
        let (mut picture, mut pilot) = flying();
        let mut ba = contact("BA", Some(Side::Blue), 0, 450);
        ba.track = Some(Track {
            course_deg: 270.0,
            speed_mps: 8.0,
        });
        hold(&mut picture, ba);
        decide(&picture, &mut pilot);
        assert_eq!(pilot.sense, 1, "counter-clockwise, toward the rear");
        // No timed flip while the track holds, however long.
        for step in 1..=5u64 {
            pilot.decide(&picture, &doctrine(), NOW + step * FLIP_EVERY_MS);
            assert_eq!(pilot.sense, 1);
        }
    }

    #[test]
    fn with_no_track_the_orbit_reverses_every_twenty_seconds() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        let mut senses = Vec::new();
        for at in [0, 19_999, 20_000, 39_999, 40_000] {
            let mut picture = picture.clone();
            if let Some(c) = picture.contacts.get_mut("BA") {
                c.read_ms = NOW + at;
            }
            picture.own.pos_read_ms = NOW + at;
            pilot.decide(&picture, &doctrine(), NOW + at);
            senses.push(pilot.sense);
        }
        assert_eq!(senses, [1, 1, -1, -1, 1]);
    }

    #[test]
    fn falling_back_on_half_the_hull_withdraws_then_widens_the_band() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        picture.own.sections.insert(
            "hull".into(),
            Section {
                condition: "damaged".into(),
                internals: Some((30, 60)),
                facets: Default::default(),
            },
        );
        let doctrine = Doctrine {
            sense: Sense::Ccw,
            ..doctrine()
        };
        let orders = pilot.decide(&picture, &doctrine, NOW);
        assert!(pilot.fallen_back);
        assert_eq!(heading(&orders), 180, "withdrawing: straight away");
        // Ten seconds on it orbits at min(450 + 150, 600) = 600: at 450 that is
        // inside the band, so the heading opens past the tangent.
        let mut later = picture.clone();
        let at = NOW + WITHDRAW_MS;
        later.own.pos_read_ms = at;
        if let Some(c) = later.contacts.get_mut("BA") {
            c.read_ms = at;
        }
        let orders = pilot.decide(&later, &doctrine, at);
        assert_eq!(heading(&orders), 150, "the band is 600: 150 m inside it");
        // A band of 500 falls back to the 600 m ceiling, not 650: with the
        // target at 600 m that is the tangent (090), where a 650 m band would
        // still close 20° (110).
        let mut at_600 = later.clone();
        hold(&mut at_600, contact("BA", Some(Side::Blue), 0, 600));
        if let Some(c) = at_600.contacts.get_mut("BA") {
            c.read_ms = at;
        }
        let wide = Doctrine {
            band_m: 500,
            ..doctrine.clone()
        };
        let orders = pilot.decide(&at_600, &wide, at);
        assert_eq!(heading(&orders), 90, "the band is 600, not 650");
    }

    #[test]
    fn falling_back_on_a_quarter_facet_from_a_hit_line() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        picture.apply(
            NOW,
            &classify("Hit on the hull's front facet: 100 of 400 armour left."),
        );
        let orders = decide(&picture, &mut pilot);
        assert!(pilot.fallen_back);
        assert_eq!(pilot.withdraw_until_ms, Some(NOW + WITHDRAW_MS));
        assert_eq!(heading(&orders), 180);
        // 101 of 400 is not a quarter.
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        picture.apply(
            NOW,
            &classify("Hit on the hull's front facet: 101 of 400 armour left."),
        );
        decide(&picture, &mut pilot);
        assert!(!pilot.fallen_back);
    }

    #[test]
    fn a_held_notice_turns_forty_five_degrees_by_sense_for_three_seconds() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        picture.own.heading_deg = 100;
        picture.events.push_back(Event {
            at_ms: NOW - 10,
            kind: EventKind::Held,
            text: "Held: something is in the way.".into(),
        });
        let doctrine = Doctrine {
            sense: Sense::Cw,
            ..doctrine()
        };
        // A clockwise doctrine: s = -1, so the turn is -45, whatever the
        // pilot's running sense was (+1 until an orbit sets it).
        assert_eq!(pilot.sense, 1);
        let orders = pilot.decide(&picture, &doctrine, NOW);
        assert_eq!(heading(&orders), 55);
        pilot.sent_heading(55, NOW);
        let orders = pilot.decide(&picture, &doctrine, NOW + HELD_FOR_MS - 1);
        assert_eq!(orders.heading, None, "held on 055");
        let orders = pilot.decide(&picture, &doctrine, NOW + HELD_FOR_MS);
        assert_eq!(heading(&orders), 270, "back on the clockwise tangent");
        // Counter-clockwise: +45.
        let held = |sense: Sense, running: i8| {
            let (mut picture, mut pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
            picture.own.heading_deg = 100;
            pilot.sense = running;
            picture.events.push_back(Event {
                at_ms: NOW - 10,
                kind: EventKind::Held,
                text: "Held: something is in the way.".into(),
            });
            let doctrine = Doctrine {
                sense,
                ..Doctrine::pilot_default(&Settings::default())
            };
            heading(&pilot.decide(&picture, &doctrine, NOW))
        };
        assert_eq!(held(Sense::Ccw, -1), 145);
        assert_eq!(held(Sense::Cw, 1), 55);
        // `Auto`: the pilot's running sense.
        assert_eq!(held(Sense::Auto, 1), 145);
        assert_eq!(held(Sense::Auto, -1), 55);
    }

    #[test]
    fn an_immobile_tank_orders_no_motion_and_a_destroyed_one_nothing() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        picture.weapons = vec![weapon("cannon", true)];
        picture.own.immobile = true;
        let orders = decide(&picture, &mut pilot);
        assert_eq!((orders.heading, orders.speed), (None, None));
        assert_eq!(orders.lock.as_deref(), Some("BA"), "it still locks");
        picture.own.immobile = false;
        picture.own.disabled = true;
        let orders = decide(&picture, &mut pilot);
        assert_eq!((orders.heading, orders.speed), (None, None));
        picture.own.destroyed = true;
        assert_eq!(decide(&picture, &mut pilot), Orders::default());
    }

    #[test]
    fn nothing_unless_aboard_powered_and_running() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        let mut off = picture.clone();
        off.aboard = false;
        assert_eq!(decide(&off, &mut pilot), Orders::default());
        let mut dark = picture.clone();
        dark.own.power = Power::Starting;
        assert_eq!(decide(&dark, &mut pilot), Orders::default());
        let mut seated = picture.clone();
        seated.phase = Phase::Seated { match_id: 1 };
        assert_eq!(decide(&seated, &mut pilot), Orders::default());
    }

    #[test]
    fn never_a_friend_or_an_unknown_locked_or_fired_on() {
        let (mut picture, mut pilot) = flying();
        picture.weapons = vec![weapon("cannon", true)];
        hold(&mut picture, contact("X1", None, 0, 300));
        hold(&mut picture, contact("RB", Some(Side::Red), 90, 200));
        let orders = decide(&picture, &mut pilot);
        assert_eq!(orders.lock, None);
        assert!(orders.fire.is_empty());
        // Even locked, a friend or an unknown is never fired on.
        for (lock, friendly) in [("X1", false), ("RB", true)] {
            picture.lock = Some(lock.into());
            picture.lock_friendly = friendly;
            let orders = decide(&picture, &mut pilot);
            assert!(orders.fire.is_empty(), "{lock}");
            assert_eq!(orders.lock, None);
        }
    }

    #[test]
    fn the_lock_keeps_its_target_until_another_is_a_hundred_metres_nearer() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        hold(&mut picture, contact("BB", Some(Side::Blue), 90, 360));
        picture.lock = Some("BA".into());
        assert_eq!(decide(&picture, &mut pilot).lock, None, "90 m nearer: kept");
        hold(&mut picture, contact("BB", Some(Side::Blue), 90, 350));
        assert_eq!(
            decide(&picture, &mut pilot).lock.as_deref(),
            Some("BB"),
            "100 m nearer"
        );
        // With no lock the nearest fresh plot wins, a stale one last.
        let (mut picture, mut pilot) = flying();
        let mut stale = contact("BA", Some(Side::Blue), 0, 100);
        stale.age_ms = 4_000;
        hold(&mut picture, stale);
        hold(&mut picture, contact("BB", Some(Side::Blue), 90, 500));
        assert_eq!(decide(&picture, &mut pilot).lock.as_deref(), Some("BB"));
        // A lock order goes at most once every 1.5 s.
        pilot.sent_lock(NOW);
        assert_eq!(pilot.decide(&picture, &doctrine(), NOW + 1_499).lock, None);
        assert_eq!(
            pilot
                .decide(&picture, &doctrine(), NOW + 1_500)
                .lock
                .as_deref(),
            Some("BB")
        );
    }

    #[test]
    fn fire_is_gated_on_readiness_rounds_freshness_and_chance() {
        let armed = || {
            let (mut picture, pilot) = flying();
            hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
            picture.lock = Some("BA".into());
            picture.weapons = vec![weapon("machine_gun", true), weapon("cannon", true)];
            (picture, pilot)
        };
        let (picture, mut pilot) = armed();
        assert_eq!(
            decide(&picture, &mut pilot).fire,
            ["cannon", "machine_gun"],
            "the main gun first"
        );
        // Not ready.
        let (mut picture, mut pilot) = armed();
        picture.weapons[1].ready = false;
        assert_eq!(decide(&picture, &mut pilot).fire, ["machine_gun"]);
        // A `ready` read inside 1.5 s of the ack is not believed.
        let (mut picture, mut pilot) = armed();
        picture.weapons[1].fired_ms = Some(NOW - 1_000);
        picture.weapons[1].read_ms = NOW - 100;
        assert_eq!(decide(&picture, &mut pilot).fire, ["machine_gun"]);
        picture.weapons[1].fired_ms = Some(NOW - 2_000);
        assert_eq!(decide(&picture, &mut pilot).fire, ["cannon", "machine_gun"]);
        // A cycle not yet elapsed; then elapsed.
        let (mut picture, mut pilot) = armed();
        picture.weapons[1].ready = false;
        picture.weapons[1].fired_ms = Some(NOW - 4_000);
        picture.weapons[1].cycle_ms = Some(6_000);
        assert_eq!(decide(&picture, &mut pilot).fire, ["machine_gun"]);
        picture.weapons[1].cycle_ms = Some(4_000);
        assert_eq!(decide(&picture, &mut pilot).fire, ["cannon", "machine_gun"]);
        // `next_ready_ms` from a cycling row.
        let (mut picture, mut pilot) = armed();
        picture.weapons[1].ready = false;
        picture.weapons[1].next_ready_ms = Some(NOW + 1);
        assert_eq!(decide(&picture, &mut pilot).fire, ["machine_gun"]);
        picture.weapons[1].next_ready_ms = Some(NOW);
        assert_eq!(decide(&picture, &mut pilot).fire, ["cannon", "machine_gun"]);
        // No rounds; 0%.
        let (mut picture, mut pilot) = armed();
        picture.weapons[1].rounds = 0;
        picture.weapons[0].percent = Some(0);
        assert!(decide(&picture, &mut pilot).fire.is_empty());
        // A stale lock: the plot too old, or the row read too long ago.
        let (mut picture, mut pilot) = armed();
        if let Some(c) = picture.contacts.get_mut("BA") {
            c.age_ms = 3_001;
        }
        assert!(decide(&picture, &mut pilot).fire.is_empty());
        let (mut picture, mut pilot) = armed();
        if let Some(c) = picture.contacts.get_mut("BA") {
            c.read_ms = NOW - 2_001;
        }
        assert!(decide(&picture, &mut pilot).fire.is_empty());
        // Locked on something else: nothing.
        let (mut picture, mut pilot) = armed();
        hold(&mut picture, contact("BB", Some(Side::Blue), 90, 460));
        picture.lock = Some("BB".into());
        assert_eq!(decide(&picture, &mut pilot).fire, ["cannon", "machine_gun"]);
        picture.lock = None;
        assert!(decide(&picture, &mut pilot).fire.is_empty());
        // No cannon: the first weapon is the main gun.
        let (mut picture, mut pilot) = armed();
        picture.weapons = vec![weapon("laser", true), weapon("machine_gun", true)];
        assert_eq!(decide(&picture, &mut pilot).fire, ["laser", "machine_gun"]);
    }

    #[test]
    fn the_search_drives_for_b_past_it_eight_hundred_metres_and_back() {
        let (mut picture, mut pilot) = flying();
        picture.spots.insert(
            'B',
            crate::arena::picture::Spot {
                name: "The Crossroads".into(),
                range_m: Some(1_000),
                bearing_deg: Some(90),
                read_ms: NOW,
                plotted_m: Some((1_000, 0)),
            },
        );
        // Nothing held since power came up ten seconds ago: searching.
        let orders = decide(&picture, &mut pilot);
        assert!(orders.ask_spot_b);
        assert_eq!(heading(&orders), 90);
        assert_eq!(pilot.spot_b_pos.map(|p| p.0.round()), Some(1_000.0));
        assert_eq!(pilot.start_pos, Some((0.0, 0.0)));
        // Within 100 m of B: on along start→B (090) for 800 m.
        let mut at_b = picture.clone();
        at_b.own.pos_m = Some((950.0, 0.0));
        at_b.own.heading_deg = 90;
        // A `BEARING B` answer read in the same tick as the position changes the
        // spot's bearing, not where the spot lies.
        if let Some(spot) = at_b.spots.get_mut(&'B') {
            spot.bearing_deg = Some(95);
            spot.read_ms = NOW + 1;
        }
        at_b.own.pos_read_ms = NOW + 1;
        pilot.sent_heading(90, NOW);
        let orders = pilot.decide(&at_b, &doctrine(), NOW + 1);
        assert!(matches!(pilot.search, SearchLeg::Beyond { .. }));
        assert!(orders.ask_spot_b);
        assert_eq!(orders.heading, None, "already on 090");
        let mut on = at_b.clone();
        on.own.pos_m = Some((1_700.0, 0.0));
        let orders = pilot.decide(&on, &doctrine(), NOW + 2);
        assert_eq!(orders.heading, None, "still going: 750 m out");
        let mut out = at_b.clone();
        out.own.pos_m = Some((1_760.0, 0.0));
        let orders = pilot.decide(&out, &doctrine(), NOW + 3);
        assert_eq!(pilot.search, SearchLeg::ToB);
        assert_eq!(heading(&orders), 270, "back for B");
        // A hostile held: no search, and not before five quiet seconds.
        let (mut picture, mut pilot) = flying();
        pilot.power_ms = Some(NOW - 4_999);
        assert!(!decide(&picture, &mut pilot).ask_spot_b);
        hold(&mut picture, contact("X1", None, 0, 500));
        let (_, mut fresh) = flying();
        assert!(
            fresh.decide(&picture, &doctrine(), NOW).ask_spot_b,
            "an unknown is not a hostile"
        );
    }

    #[test]
    fn the_crossroads_is_plotted_from_a_status_block_never_from_a_bearing_answer() {
        let (mut picture, mut pilot) = flying();
        let read = |picture: &mut Picture, at: u64, lines: &[&str]| {
            for line in lines {
                picture.apply(at, &classify(line));
            }
        };
        // At (1005, 975) The Crossroads is nearest, 290 m at 090: (1295, 975).
        read(
            &mut picture,
            NOW,
            &[
                "Callsign RA.",
                "Red Tank 1: power online, 0 kph, heading 090°",
                "  at 1005 m east, 975 m north; scope clear",
                "  nearest spot: The Crossroads (B), 290 m at 090°",
                "No contacts.",
            ],
        );
        decide(&picture, &mut pilot);
        assert_eq!(pilot.spot_b_pos, Some((1_295.0, 975.0)));
        // Twenty seconds on, at (828, 839) with The Ford nearest, a `STATUS` and
        // a `BEARING B` answer land together: the bearing is fresh, the spot's
        // range is the old block's. B stays where it is.
        let later = NOW + 20_000;
        read(
            &mut picture,
            later,
            &[
                "Callsign RA.",
                "Red Tank 1: power online, 0 kph, heading 090°",
                "  at 828 m east, 839 m north; scope clear",
                "  nearest spot: The Ford (A), 300 m at 200°",
                "Bearing to The Crossroads (B): 070° (rel 340°).",
            ],
        );
        assert_eq!(picture.spots[&'B'].bearing_deg, Some(70));
        assert_eq!(picture.spots[&'B'].range_m, Some(290));
        let orders = pilot.decide(&picture, &doctrine(), later);
        assert_eq!(pilot.spot_b_pos, Some((1_295.0, 975.0)));
        // bearing (828, 839) -> (1295, 975) is 073.8.
        assert_eq!(orders.heading, Some(74));
        // A `RANGE B` answer does not move it either.
        read(
            &mut picture,
            later + 10,
            &["Range to The Crossroads (B): 480 m."],
        );
        pilot.decide(&picture, &doctrine(), later + 10);
        assert_eq!(pilot.spot_b_pos, Some((1_295.0, 975.0)));
    }

    #[test]
    fn a_heading_goes_again_only_eight_degrees_off_or_after_four_seconds() {
        let (mut picture, mut pilot) = flying();
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        let doctrine = Doctrine {
            sense: Sense::Ccw,
            ..doctrine()
        };
        assert_eq!(pilot.decide(&picture, &doctrine, NOW).heading, Some(90));
        pilot.sent_heading(90, NOW);
        pilot.sent_speed(54);
        // 7° off (the target 7° further round): not yet.
        hold(&mut picture, contact("BA", Some(Side::Blue), 7, 450));
        let orders = pilot.decide(&picture, &doctrine, NOW + 100);
        assert_eq!((orders.heading, orders.speed), (None, None));
        assert_eq!(
            pilot.decide(&picture, &doctrine, NOW + 4_000).heading,
            Some(97),
            "any amount after 4 s"
        );
        hold(&mut picture, contact("BA", Some(Side::Blue), 8, 450));
        assert_eq!(
            pilot.decide(&picture, &doctrine, NOW + 100).heading,
            Some(98)
        );
        // An unanswered order is forgotten and goes again.
        pilot.forget_heading();
        assert_eq!(
            pilot.decide(&picture, &doctrine, NOW + 101).heading,
            Some(98)
        );
    }

    #[test]
    fn the_first_speed_order_is_nine_nine_nine() {
        let (mut picture, mut pilot) = flying();
        picture.own.max_ahead_kph = None;
        pilot.ceilings_asked = false;
        hold(&mut picture, contact("BA", Some(Side::Blue), 0, 450));
        let orders = decide(&picture, &mut pilot);
        assert_eq!(orders.speed, Some(CEILING_ASK_KPH));
        pilot.sent_speed(CEILING_ASK_KPH);
        // Asked, not yet read: full ahead is 40.
        assert_eq!(decide(&picture, &mut pilot).speed, Some(UNKNOWN_FULL_KPH));
        // Unanswered: asked again.
        pilot.forget_speed();
        assert_eq!(decide(&picture, &mut pilot).speed, Some(CEILING_ASK_KPH));
        // Read: 54.
        picture.apply(
            NOW,
            &classify("That unit makes 54 kph ahead and 27 astern; SPEED -27 to 54."),
        );
        assert_eq!(decide(&picture, &mut pilot).speed, Some(54));
    }
}

//! **S7 (doc 10 T17) — the campaign ephemeris: the constant sky** (as amended
//! by doc 18 R9 and doc 10 §22's T17 amendment; doc 18 §4.8, D-18-48).
//!
//! The solar system is not a coordinate map. It is a constellation of places,
//! each body a circular, coplanar orbit — a heliocentric distance and a longitude
//! at an epoch — and the distance between two of them is the law of cosines over
//! those numbers. A fixture body is placed at the configured campaign epoch; a
//! lane body (doc 18 cut IC2) carries its own `epoch_ic`, the real date whose
//! true longitude and distance it was authored from, and is held there.
//!
//! `rate = 0` is the default and holds every body at its authored phase (the
//! constant sky): Titan is the same distance from Saturn on the day a character
//! moves there as it is a year later, and a route's pinned chord is the chord on
//! any IC day. The knob exists so a campaign that wants a moving sky can have one
//! *coherently*: every distance is derived from the same body positions at the
//! same instant, so every pair sharing a body changes together and no pair can
//! contradict another. That is the E lens's property, and it holds by
//! construction rather than by bookkeeping. Every production distance asks
//! [`crate::sky::Sky`] (D-18-43); the methods here are its circular model and
//! the fixtures' own readings.
//!
//! Every number is an integer of fixed scale — micro-AU, milli-days,
//! milli-degrees — except the last step, the law of cosines, which is evaluated
//! in `f64` and rounded to a kilometre. A kilometre on a hundred million is
//! nothing a duration reads, and the same target computes the same kilometre
//! every load (the D lens is per platform; the pin is re-derived, never stored).

use std::collections::BTreeMap;

use mush_core::CanonicalId;

use crate::config::EphemerisConfig;
use crate::pack::{BodyDef, PackEntry, RulesPack};

/// One astronomical unit, kilometres (IAU 2012, rounded).
pub const AU_KM: u64 = 149_597_871;

/// The farthest orbit a pack may author: 100 000 AU, the Oort cloud's edge.
pub const MAX_ORBIT_UAU: u64 = 100_000_000_000;

/// The longest period a pack may author: a billion days.
pub const MAX_PERIOD_MDAYS: u64 = 1_000_000_000_000;

/// A full turn, milli-degrees.
const MDEG_TURN: i128 = 360_000;

/// One body of the sky: a circular, coplanar orbit and where on it the body
/// stood at its epoch — its own `epoch_ic` when it has one, else the campaign
/// epoch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Body {
    /// Orbit radius, micro-AU.
    pub orbit_uau: u64,
    /// Orbital period, milli-days.
    pub period_mdays: u64,
    /// Heliocentric longitude at the epoch, milli-degrees in `0..360_000`.
    pub phase_mdeg: i64,
    /// **Doc 18 cut IC2 (§4.8, D-18-48)** — the IC instant the phase is at, or
    /// `None` for the configured campaign epoch.
    pub epoch_ic: Option<i64>,
}

impl From<&BodyDef> for Body {
    fn from(def: &BodyDef) -> Body {
        Body {
            orbit_uau: def.orbit_uau,
            period_mdays: def.period_mdays,
            phase_mdeg: def.phase_mdeg,
            epoch_ic: def.epoch_ic,
        }
    }
}

/// The sky of one pack: its bodies, the epoch and the rate.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ephemeris {
    epoch: i64,
    rate_permille: u32,
    bodies: BTreeMap<CanonicalId, Body>,
}

impl Ephemeris {
    /// An empty sky under a configuration.
    #[must_use]
    pub fn new(cfg: &EphemerisConfig) -> Ephemeris {
        Ephemeris {
            epoch: cfg.epoch,
            rate_permille: cfg.rate,
            bodies: BTreeMap::new(),
        }
    }

    /// The same sky with one more body.
    #[must_use]
    pub fn with_body(mut self, id: CanonicalId, body: Body) -> Ephemeris {
        self.bodies.insert(id, body);
        self
    }

    /// Every `body` entry of a pack, under the configuration.
    #[must_use]
    pub fn from_pack(pack: &RulesPack, cfg: &EphemerisConfig) -> Ephemeris {
        let mut sky = Ephemeris::new(cfg);
        for (id, entry) in pack.entries() {
            if let PackEntry::Body(def) = entry {
                sky.bodies.insert(id.clone(), Body::from(def));
            }
        }
        sky
    }

    /// The configured campaign epoch, IC seconds on the game calendar: the
    /// instant for a body without its own `epoch_ic`.
    #[must_use]
    pub fn epoch(&self) -> i64 {
        self.epoch
    }

    /// Permille of the real sky's motion per IC second; 0 holds every body at
    /// its authored phase (the constant sky).
    #[must_use]
    pub fn rate_permille(&self) -> u32 {
        self.rate_permille
    }

    /// Whether no body is known.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bodies.is_empty()
    }

    /// One body.
    #[must_use]
    pub fn body(&self, id: &CanonicalId) -> Option<&Body> {
        self.bodies.get(id)
    }

    /// The bodies, in id order.
    pub fn bodies(&self) -> impl Iterator<Item = (&CanonicalId, &Body)> {
        self.bodies.iter()
    }

    /// A body's heliocentric longitude at IC time `t`, milli-degrees in
    /// `0..360_000`. At rate 0 this is the authored phase whatever `t` is;
    /// otherwise the phase advances by `rate` of the real sky's motion, measured
    /// from the body's own `epoch_ic` (doc 18 cut IC2, D-18-48) or, without
    /// one, from the configured campaign epoch.
    #[must_use]
    pub fn phase_mdeg_at(&self, id: &CanonicalId, t: i64) -> Option<i64> {
        let body = self.body(id)?;
        let base = i128::from(body.phase_mdeg);
        if self.rate_permille == 0 {
            return Some(normalize(base));
        }
        // The period in seconds, times a thousand: milli-days × 86 400 s/day.
        let period_ms = i128::from(body.period_mdays).checked_mul(86_400)?;
        if period_ms == 0 {
            return None;
        }
        let elapsed = i128::from(t).checked_sub(i128::from(body.epoch_ic.unwrap_or(self.epoch)))?;
        let advance = elapsed
            .checked_mul(i128::from(self.rate_permille))?
            .checked_mul(MDEG_TURN)?
            / period_ms;
        Some(normalize(base.checked_add(advance)?))
    }

    /// The distance between two bodies at IC time `t`, kilometres — the law of
    /// cosines over their orbit radii and the difference of their longitudes.
    #[must_use]
    pub fn distance_km_at(&self, a: &CanonicalId, b: &CanonicalId, t: i64) -> Option<u64> {
        let (body_a, body_b) = (self.body(a)?, self.body(b)?);
        let delta = self
            .phase_mdeg_at(a, t)?
            .checked_sub(self.phase_mdeg_at(b, t)?)?;
        Some(separation_km(body_a.orbit_uau, body_b.orbit_uau, delta))
    }

    /// The pinned distance: the distance at the configured epoch, which at rate
    /// 0 is the distance on any IC day. Fixtures and tests only (D-18-43):
    /// production pins through [`crate::sky::Sky::distance_km`].
    #[must_use]
    pub fn pinned_km(&self, a: &CanonicalId, b: &CanonicalId) -> Option<u64> {
        self.distance_km_at(a, b, self.epoch)
    }
}

/// Milli-degrees into `0..360_000`.
fn normalize(mdeg: i128) -> i64 {
    let wrapped = ((mdeg % MDEG_TURN) + MDEG_TURN) % MDEG_TURN;
    // In range by construction; the fallback is unreachable.
    i64::try_from(wrapped).unwrap_or(0)
}

/// The separation of two circular, coplanar orbits at a longitude difference,
/// kilometres: `√(r₁² + r₂² − 2·r₁·r₂·cos Δθ)`, rounded.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn separation_km(orbit_a_uau: u64, orbit_b_uau: u64, delta_mdeg: i64) -> u64 {
    let au = AU_KM as f64;
    let r1 = orbit_a_uau as f64 * au / 1_000_000.0;
    let r2 = orbit_b_uau as f64 * au / 1_000_000.0;
    let theta = (delta_mdeg as f64 / 1_000.0).to_radians();
    let squared = r1 * r1 + r2 * r2 - 2.0 * r1 * r2 * theta.cos();
    squared.max(0.0).sqrt().round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cid(s: &str) -> CanonicalId {
        CanonicalId::new(s).expect("a canonical id")
    }

    fn sky(rate: u32) -> Ephemeris {
        Ephemeris::new(&EphemerisConfig {
            epoch: 1_900_000_000,
            rate,
        })
        .with_body(
            cid("earth"),
            Body {
                orbit_uau: 1_000_000,
                period_mdays: 365_250,
                phase_mdeg: 300_000,
                epoch_ic: None,
            },
        )
        .with_body(
            cid("mars"),
            Body {
                orbit_uau: 1_524_000,
                period_mdays: 686_980,
                phase_mdeg: 130_000,
                epoch_ic: None,
            },
        )
        .with_body(
            cid("ceres"),
            Body {
                orbit_uau: 2_766_000,
                period_mdays: 1_681_600,
                phase_mdeg: 140_000,
                epoch_ic: None,
            },
        )
    }

    #[test]
    fn a_frozen_sky_holds_its_phases_on_any_day() {
        let sky = sky(0);
        for t in [i64::MIN / 4, -1, 0, 1_900_000_000, i64::MAX / 4] {
            assert_eq!(sky.phase_mdeg_at(&cid("mars"), t), Some(130_000));
        }
    }

    #[test]
    fn a_moving_sky_advances_a_body_one_turn_per_period() {
        let sky = sky(1_000);
        let one_earth_year = 1_900_000_000 + 365_250 * 86_400 / 1_000;
        assert_eq!(
            sky.phase_mdeg_at(&cid("earth"), one_earth_year),
            Some(300_000),
            "a full turn brings Earth back to its phase"
        );
        let quarter = 1_900_000_000 + 365_250 * 86_400 / 4_000;
        assert_eq!(sky.phase_mdeg_at(&cid("earth"), quarter), Some(30_000));
    }

    #[test]
    fn two_places_on_one_orbit_and_phase_are_no_distance_apart() {
        assert_eq!(separation_km(1_524_000, 1_524_000, 0), 0);
        assert_eq!(separation_km(1_524_000, 1_524_000, 360_000), 0);
    }

    #[test]
    fn opposite_phases_sum_the_radii() {
        let km = separation_km(1_000_000, 1_524_000, 180_000);
        assert_eq!(km, (AU_KM as f64 * 2.524).round() as u64);
    }
}

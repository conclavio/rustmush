//! **The sky seam** (doc 18 cut IC2; doc 18 §4.8 "The sky", D-18-43, D-18-48).
//!
//! The one query every production distance asks. A route's pinned chord
//! ([`crate::store`]'s `materialize_route`), `@corridor/show`'s per-leg figures
//! and — from cut IC9 — the passage planner read [`Sky::distance_km`], never the
//! circular model's own methods, so the astronomy engine (post-slice cut P-A)
//! swaps the model behind this enum (`Kepler`, `Tabulated`) without a second
//! rounding path or a caller that knew which sky it had (D-18-43).
//!
//! In the slice the sky is [`Sky::Circular`]: [`Ephemeris`] over a pack's `body`
//! entries, each a true heliocentric distance and longitude at its own
//! `epoch_ic` (D-18-48; a fixture body at the configured campaign epoch), held
//! there at rate 0 (the constant sky, doc 10 §22's T17 as amended by R9). The
//! atlas drawing stays an [`Ephemeris`] projection (D-18-49): it is a picture,
//! not a distance.
//!
//! Dates render on the game's own calendar (`mush_core::CivilDate`: 365-day
//! years, year 0 at IC 0; lead ruling R3), so the date `@corridor/show` prints
//! beside a figure is the date `time` shows a player.

use mush_core::{CanonicalId, CivilDate, DefinitionDigest};
use serde_json::json;

use crate::config::EphemerisConfig;
use crate::ephemeris::{AU_KM, Ephemeris, separation_km};
use crate::pack::RulesPack;

/// The domain a sky's revision digest is taken under.
pub const SKY_DIGEST_DOMAIN: &str = "tacsim.sky.v1";

/// A heliocentric position, kilometres, in the ecliptic frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PosKm {
    /// Towards the vernal equinox.
    pub x: i64,
    /// Ninety degrees on, in the ecliptic.
    pub y: i64,
    /// Out of the ecliptic; zero while the sky is circular and coplanar.
    pub z: i64,
}

/// Why a sky could not state its revision.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SkyError {
    /// The canonical encoder refused the sky's value (lead ruling R9: a
    /// `Result`, never a panic or a sentinel digest).
    #[error("the sky does not encode: {0}")]
    Encoding(String),
}

/// **The sky** (doc 18 §4.8). P-A adds `Kepler` and `Tabulated`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Sky {
    /// Circular, coplanar orbits from a pack's bodies.
    Circular(Ephemeris),
}

impl Sky {
    /// The sky of one pack's `body` entries under the configuration.
    #[must_use]
    pub fn from_pack(pack: &RulesPack, cfg: &EphemerisConfig) -> Sky {
        Sky::Circular(Ephemeris::from_pack(pack, cfg))
    }

    /// The model's word (`circular`).
    #[must_use]
    pub fn model(&self) -> &'static str {
        match self {
            Sky::Circular(_) => "circular",
        }
    }

    /// The configured campaign epoch, IC seconds.
    #[must_use]
    pub fn config_epoch(&self) -> i64 {
        match self {
            Sky::Circular(sky) => sky.epoch(),
        }
    }

    /// The instant a body's authored numbers are at: its own `epoch_ic`, else
    /// the configured epoch; `None` for a body the sky does not know.
    #[must_use]
    pub fn epoch_of(&self, body: &CanonicalId) -> Option<i64> {
        match self {
            Sky::Circular(sky) => sky.body(body).map(|b| b.epoch_ic.unwrap_or(sky.epoch())),
        }
    }

    /// A body's position at IC time `t_ic` — `r = orbit`, `θ = phase at t`,
    /// rounded to the kilometre, `z = 0` while circular. No production caller
    /// in the slice (D-18-43; the chart is P-A's).
    #[must_use]
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    pub fn position_km(&self, body: &CanonicalId, t_ic: i64) -> Option<PosKm> {
        match self {
            Sky::Circular(sky) => {
                let b = sky.body(body)?;
                let phase = sky.phase_mdeg_at(body, t_ic)?;
                let r = b.orbit_uau as f64 * AU_KM as f64 / 1_000_000.0;
                let theta = (phase as f64 / 1_000.0).to_radians();
                Some(PosKm {
                    x: (r * theta.cos()).round() as i64,
                    y: (r * theta.sin()).round() as i64,
                    z: 0,
                })
            }
        }
    }

    /// **The distance from `a` at `ta` to `b` at `tb`**, kilometres. For the
    /// circular sky this is [`separation_km`] over the two orbits and the
    /// difference of the two phases — the function
    /// [`Ephemeris::distance_km_at`] calls, so `ta == tb` is bit-identical to it
    /// and there is no second rounding path.
    #[must_use]
    pub fn distance_km(&self, a: &CanonicalId, ta: i64, b: &CanonicalId, tb: i64) -> Option<u64> {
        match self {
            Sky::Circular(sky) => {
                let (body_a, body_b) = (sky.body(a)?, sky.body(b)?);
                let delta = sky
                    .phase_mdeg_at(a, ta)?
                    .checked_sub(sky.phase_mdeg_at(b, tb)?)?;
                Some(separation_km(body_a.orbit_uau, body_b.orbit_uau, delta))
            }
        }
    }

    /// **The sky's revision**: the canonical digest, under
    /// [`SKY_DIGEST_DOMAIN`], of the model, the configured epoch and rate, and
    /// every body's four numbers (`epoch_ic` null when absent). A passage plan
    /// records it (cut IC9), so a plan made under one sky is never read as made
    /// under another.
    ///
    /// # Errors
    ///
    /// [`SkyError::Encoding`] when the canonical encoder refuses the value.
    pub fn revision(&self) -> Result<DefinitionDigest, SkyError> {
        match self {
            Sky::Circular(sky) => {
                let bodies: serde_json::Map<String, serde_json::Value> = sky
                    .bodies()
                    .map(|(id, body)| {
                        (
                            id.to_string(),
                            json!({
                                "orbit_uau": body.orbit_uau,
                                "period_mdays": body.period_mdays,
                                "phase_mdeg": body.phase_mdeg,
                                "epoch_ic": body.epoch_ic,
                            }),
                        )
                    })
                    .collect();
                let value = json!({
                    "model": "circular",
                    "epoch": sky.epoch(),
                    "rate": sky.rate_permille(),
                    "bodies": bodies,
                });
                DefinitionDigest::for_canonical_value(SKY_DIGEST_DOMAIN, &value)
                    .map_err(|error| SkyError::Encoding(error.to_string()))
            }
        }
    }
}

/// **An IC instant's date on the game calendar**, `YYYY-MM-DD`
/// (`mush_core::CivilDate`; lead ruling R3).
#[must_use]
pub fn epoch_date(ic_s: i64) -> String {
    let date = CivilDate::from_ic_seconds(ic_s);
    format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ephemeris::Body;

    fn cid(s: &str) -> CanonicalId {
        CanonicalId::new(s).expect("a canonical id")
    }

    fn sky(rate: u32, epoch_ic: Option<i64>) -> Sky {
        sky_with(rate, epoch_ic, epoch_ic)
    }

    fn sky_with(rate: u32, ceres_epoch: Option<i64>, mars_epoch: Option<i64>) -> Sky {
        Sky::Circular(
            Ephemeris::new(&EphemerisConfig { epoch: 0, rate })
                .with_body(
                    cid("ceres"),
                    Body {
                        orbit_uau: 2_866_568,
                        period_mdays: 1_681_861,
                        phase_mdeg: 281_654,
                        epoch_ic: ceres_epoch,
                    },
                )
                .with_body(
                    cid("mars"),
                    Body {
                        orbit_uau: 1_445_443,
                        period_mdays: 687_034,
                        phase_mdeg: 276_240,
                        epoch_ic: mars_epoch,
                    },
                ),
        )
    }

    #[test]
    fn a_circular_position_lies_in_the_ecliptic() {
        let sky = sky(1_000, Some(74_152_540_800));
        for t in [0, 74_152_540_800, 80_000_000_000] {
            let at = sky.position_km(&cid("ceres"), t).expect("a known body");
            assert_eq!(at.z, 0);
        }
        assert_eq!(sky.position_km(&cid("pluto"), 0), None);
    }

    #[test]
    fn the_epoch_date_is_the_game_calendars() {
        assert_eq!(epoch_date(0), "0000-01-01");
        let ic = CivilDate::parse("2351-05-13")
            .expect("a calendar date")
            .to_ic_seconds();
        assert_eq!(ic, 74_152_540_800);
        assert_eq!(epoch_date(ic), "2351-05-13");
    }

    #[test]
    fn the_revision_is_stable_and_moves_with_the_rate_and_an_epoch() {
        let first = sky(0, Some(74_152_540_800)).revision().expect("encodes");
        let again = sky(0, Some(74_152_540_800)).revision().expect("encodes");
        assert_eq!(first, again, "two builds of one sky, one revision");
        let rated = sky(1_000, Some(74_152_540_800))
            .revision()
            .expect("encodes");
        assert_ne!(first, rated, "the rate is in the revision");
        let moved = sky_with(0, Some(74_152_540_800), Some(74_152_627_200))
            .revision()
            .expect("encodes");
        assert_ne!(first, moved, "one body's epoch_ic is in the revision");
        let absent = sky(0, None).revision().expect("encodes");
        assert_ne!(first, absent, "an absent epoch_ic is a different sky");
    }
}

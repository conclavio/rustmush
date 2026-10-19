//! **The sky seam** (doc 18 cut IC2; doc 18 §4.8 "The sky", D-18-43, D-18-48;
//! doc 19 cut A.1, D-19-1 … D-19-5).
//!
//! The one query every production distance asks. A route's pinned chord
//! ([`crate::store`]'s `materialize_route`), `@corridor/show`'s per-leg figures
//! and — from cut IC9 — the passage planner read [`Sky::distance_km`], never a
//! model's own methods, so the model behind this enum changes without a second
//! rounding path or a caller that knew which sky it had (D-18-43).
//!
//! Two models stand behind it:
//!
//! - [`Sky::Circular`] — the slice's: [`Ephemeris`] over a pack's `body`
//!   entries, each a true heliocentric distance and longitude at its own
//!   `epoch_ic` (D-18-48; a fixture body at the configured campaign epoch),
//!   advanced on a circular, coplanar orbit at the configured rate. Every pack
//!   without an element table is this sky, byte for byte as before.
//! - [`Sky::Kepler`] — the astronomy engine's (doc 19): chosen whenever any body
//!   of the pack carries an element table ([`crate::kepler`]). A body with a
//!   table moves on its real orbit in three dimensions; a body without one keeps
//!   the circular model's integer arithmetic to the bit, and a pair of circular
//!   bodies keeps [`separation_km`]. The rate keeps its meaning: `0` holds every
//!   body at its epoch position (now with latitude), `1000` is real motion.
//!
//! The atlas drawing stays an [`Ephemeris`] projection (D-18-49): it is a
//! picture, not a distance. Dates render on the game's own calendar
//! (`mush_core::CivilDate`: 365-day years, year 0 at IC 0; lead ruling R3).

use std::collections::BTreeMap;

use mush_core::{CanonicalId, CivilDate, DefinitionDigest};
use serde_json::json;

use crate::config::EphemerisConfig;
use crate::ephemeris::{AU_KM, Body, Ephemeris, separation_km};
use crate::kepler::{self, ElementRow};
use crate::pack::{PackEntry, RulesPack};

/// The domain a sky's revision digest is taken under.
pub const SKY_DIGEST_DOMAIN: &str = "tacsim.sky.v1";
/// The domain one body's element table is digested under, for the revision.
pub const SKY_TABLE_DIGEST_DOMAIN: &str = "tacsim.sky.table.v1";
/// The speed of light, metres per second, for the light lag (doc 19 §7).
pub const LIGHT_M_PER_S: u64 = 299_792_458;

/// A heliocentric position, kilometres, in the ecliptic frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PosKm {
    /// Towards the vernal equinox.
    pub x: i64,
    /// Ninety degrees on, in the ecliptic.
    pub y: i64,
    /// Out of the ecliptic; zero for a circular body.
    pub z: i64,
}

impl PosKm {
    /// The distance to another position, kilometres: the integer square root of
    /// the sum of squares (no rounding path but the one at the position).
    #[must_use]
    pub fn distance_km(self, other: PosKm) -> u64 {
        let d = |a: i64, b: i64| -> u128 {
            let gap = i128::from(a) - i128::from(b);
            gap.unsigned_abs().pow(2)
        };
        let squared = d(self.x, other.x) + d(self.y, other.y) + d(self.z, other.z);
        u64::try_from(squared.isqrt()).unwrap_or(u64::MAX)
    }
}

/// Why a sky could not state its revision.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SkyError {
    /// The canonical encoder refused the sky's value (lead ruling R9: a
    /// `Result`, never a panic or a sentinel digest).
    #[error("the sky does not encode: {0}")]
    Encoding(String),
}

/// **The Kepler sky** (doc 19 D-19-1): the circular model for every body, and an
/// element table for those that have one. The tables' digests are taken once,
/// here, so [`Sky::revision`] hashes a summary rather than every row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeplerSky {
    circular: Ephemeris,
    tables: BTreeMap<CanonicalId, Vec<ElementRow>>,
    table_digests: BTreeMap<CanonicalId, Result<DefinitionDigest, String>>,
}

impl KeplerSky {
    /// From the circular model and the tables of the bodies that have one.
    #[must_use]
    pub fn new(circular: Ephemeris, tables: BTreeMap<CanonicalId, Vec<ElementRow>>) -> KeplerSky {
        let table_digests = tables
            .iter()
            .map(|(id, rows)| {
                let value = json!(
                    rows.iter()
                        .map(|row| {
                            json!([
                                row.epoch_ic,
                                row.a_nau,
                                row.e_nano,
                                row.i_udeg,
                                row.node_udeg,
                                row.peri_udeg,
                                row.ma_udeg,
                                row.n_ndeg_day
                            ])
                        })
                        .collect::<Vec<_>>()
                );
                let digest = DefinitionDigest::for_canonical_value(SKY_TABLE_DIGEST_DOMAIN, &value)
                    .map_err(|error| error.to_string());
                (id.clone(), digest)
            })
            .collect();
        KeplerSky {
            circular,
            tables,
            table_digests,
        }
    }

    /// The circular model under the tables.
    #[must_use]
    pub fn circular(&self) -> &Ephemeris {
        &self.circular
    }

    /// A body's element table, or `None` for a circular body.
    #[must_use]
    pub fn table(&self, body: &CanonicalId) -> Option<&[ElementRow]> {
        self.tables.get(body).map(Vec::as_slice)
    }

    /// The bodies that move on tables, in id order.
    pub fn keplerian_bodies(&self) -> impl Iterator<Item = &CanonicalId> {
        self.tables.keys()
    }

    /// The instant a Keplerian body's table is read at for IC time `t`: its
    /// own epoch plus the elapsed interval scaled by the rate (`0` holds the
    /// body at its epoch, `1000` is real motion).
    fn effective_ic(&self, body: &Body, t: i64) -> i64 {
        let epoch = body.epoch_ic.unwrap_or_else(|| self.circular.epoch());
        match self.circular.rate_permille() {
            0 => epoch,
            1_000 => t,
            rate => {
                let elapsed = i128::from(t) - i128::from(epoch);
                let scaled = elapsed * i128::from(rate) / 1_000;
                i64::try_from(i128::from(epoch) + scaled).unwrap_or(epoch)
            }
        }
    }

    /// A body's position at IC time `t`, kilometres in `f64`, or `None` for a
    /// body the sky does not know.
    #[must_use]
    pub fn position_f64(&self, body: &CanonicalId, t: i64) -> Option<[f64; 3]> {
        let def = self.circular.body(body)?;
        match self.tables.get(body) {
            Some(rows) => {
                let at = self.effective_ic(def, t);
                kepler::table_position(rows, kepler::julian_day(at))
            }
            None => {
                let at = circular_position(&self.circular, body, t)?;
                #[allow(clippy::cast_precision_loss)]
                Some([at.x as f64, at.y as f64, at.z as f64])
            }
        }
    }
}

/// A circular body's position at IC time `t` — `r = orbit`, `θ = phase at t`,
/// rounded to the kilometre, `z = 0`.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn circular_position(sky: &Ephemeris, body: &CanonicalId, t_ic: i64) -> Option<PosKm> {
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

/// A position in `f64` kilometres rounded to the kilometre.
#[allow(clippy::cast_possible_truncation)]
fn rounded(position: [f64; 3]) -> PosKm {
    PosKm {
        x: position[0].round() as i64,
        y: position[1].round() as i64,
        z: position[2].round() as i64,
    }
}

/// **The sky** (doc 18 §4.8; doc 19 §3).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Sky {
    /// Circular, coplanar orbits from a pack's bodies (the slice's model).
    Circular(Ephemeris),
    /// Real orbits from element tables, circular bodies beside them (doc 19).
    Kepler(KeplerSky),
}

impl Sky {
    /// The sky of one pack's `body` entries under the configuration:
    /// [`Sky::Kepler`] when any body carries an element table, else
    /// [`Sky::Circular`] (D-19-1).
    #[must_use]
    pub fn from_pack(pack: &RulesPack, cfg: &EphemerisConfig) -> Sky {
        let circular = Ephemeris::from_pack(pack, cfg);
        let tables: BTreeMap<CanonicalId, Vec<ElementRow>> = pack
            .entries()
            .iter()
            .filter_map(|(id, entry)| match entry {
                PackEntry::Body(def) if def.is_keplerian() => {
                    Some((id.clone(), def.elements.clone()))
                }
                _ => None,
            })
            .collect();
        if tables.is_empty() {
            Sky::Circular(circular)
        } else {
            Sky::Kepler(KeplerSky::new(circular, tables))
        }
    }

    /// The model's word (`circular` or `kepler`).
    #[must_use]
    pub fn model(&self) -> &'static str {
        match self {
            Sky::Circular(_) => "circular",
            Sky::Kepler(_) => "kepler",
        }
    }

    /// The circular model every sky carries (the Kepler sky's circular bodies
    /// and the four authored numbers of every body).
    #[must_use]
    pub fn ephemeris(&self) -> &Ephemeris {
        match self {
            Sky::Circular(sky) => sky,
            Sky::Kepler(sky) => sky.circular(),
        }
    }

    /// The configured campaign epoch, IC seconds.
    #[must_use]
    pub fn config_epoch(&self) -> i64 {
        self.ephemeris().epoch()
    }

    /// The instant a body's authored numbers are at: its own `epoch_ic`, else
    /// the configured epoch; `None` for a body the sky does not know.
    #[must_use]
    pub fn epoch_of(&self, body: &CanonicalId) -> Option<i64> {
        let sky = self.ephemeris();
        sky.body(body).map(|b| b.epoch_ic.unwrap_or(sky.epoch()))
    }

    /// Whether `body` moves on an element table in this sky.
    #[must_use]
    pub fn is_keplerian(&self, body: &CanonicalId) -> bool {
        match self {
            Sky::Circular(_) => false,
            Sky::Kepler(sky) => sky.table(body).is_some(),
        }
    }

    /// **A body's orbital period, days, at IC time `t_ic`** (doc 19 cut A.4, for
    /// the synodic scan): on a table, `360 / n` of the row nearest `t_ic`; on the
    /// circular model, the authored `period_mdays`; `None` for a body the sky
    /// does not know.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn period_days(&self, body: &CanonicalId, t_ic: i64) -> Option<f64> {
        if let Sky::Kepler(sky) = self
            && let Some(rows) = sky.table(body)
        {
            let nearest = rows.iter().min_by_key(|row| row.epoch_ic.abs_diff(t_ic))?;
            if nearest.n_ndeg_day <= 0 {
                return None;
            }
            return Some(360.0 * 1e9 / nearest.n_ndeg_day as f64);
        }
        self.ephemeris()
            .body(body)
            .map(|b| b.period_mdays as f64 / 1_000.0)
    }

    /// A body's position at IC time `t_ic`, rounded to the kilometre: on the
    /// circular model `r = orbit`, `θ = phase at t`, `z = 0`; on a table, its
    /// Kepler propagation (D-19-4).
    #[must_use]
    pub fn position_km(&self, body: &CanonicalId, t_ic: i64) -> Option<PosKm> {
        match self {
            Sky::Circular(sky) => circular_position(sky, body, t_ic),
            Sky::Kepler(sky) => {
                if sky.table(body).is_some() {
                    sky.position_f64(body, t_ic).map(rounded)
                } else {
                    circular_position(sky.circular(), body, t_ic)
                }
            }
        }
    }

    /// A body's heliocentric distance (km), ecliptic longitude and latitude
    /// (degrees) at IC time `t_ic`, for the chart.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn heliocentric(&self, body: &CanonicalId, t_ic: i64) -> Option<(f64, f64, f64)> {
        let at = self.position_km(body, t_ic)?;
        Some(kepler::polar([at.x as f64, at.y as f64, at.z as f64]))
    }

    /// **The distance from `a` at `ta` to `b` at `tb`**, kilometres. For a pair
    /// of circular bodies this is [`separation_km`] over the two orbits and the
    /// difference of the two phases — the function
    /// [`Ephemeris::distance_km_at`] calls, so `ta == tb` is bit-identical to it
    /// and there is no second rounding path. When either body moves on a table
    /// it is the integer distance between the two rounded positions
    /// ([`PosKm::distance_km`]).
    #[must_use]
    pub fn distance_km(&self, a: &CanonicalId, ta: i64, b: &CanonicalId, tb: i64) -> Option<u64> {
        match self {
            Sky::Circular(sky) => circular_distance(sky, a, ta, b, tb),
            Sky::Kepler(sky) => {
                if sky.table(a).is_none() && sky.table(b).is_none() {
                    return circular_distance(sky.circular(), a, ta, b, tb);
                }
                let (pa, pb) = (self.position_km(a, ta)?, self.position_km(b, tb)?);
                Some(pa.distance_km(pb))
            }
        }
    }

    /// **The one-way light lag from `a` at `ta` to `b` at `tb`**, whole seconds
    /// rounded up (doc 19 §7).
    #[must_use]
    pub fn light_lag_s(&self, a: &CanonicalId, ta: i64, b: &CanonicalId, tb: i64) -> Option<u64> {
        let km = self.distance_km(a, ta, b, tb)?;
        Some((km * 1_000).div_ceil(LIGHT_M_PER_S))
    }

    /// **The sky's revision**: the canonical digest, under
    /// [`SKY_DIGEST_DOMAIN`], of the model, the configured epoch and rate, every
    /// body's four numbers (`epoch_ic` null when absent) and, for a body on a
    /// table, the table's own digest. A passage plan records it (cut IC9), so a
    /// plan made under one sky is never read as made under another; the
    /// circular sky's value is byte for byte the slice's.
    ///
    /// # Errors
    ///
    /// [`SkyError::Encoding`] when the canonical encoder refuses the value.
    pub fn revision(&self) -> Result<DefinitionDigest, SkyError> {
        let sky = self.ephemeris();
        let mut bodies: serde_json::Map<String, serde_json::Value> = sky
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
        if let Sky::Kepler(kepler) = self {
            for (id, digest) in &kepler.table_digests {
                let digest = digest
                    .as_ref()
                    .map_err(|error| SkyError::Encoding(error.clone()))?;
                if let Some(serde_json::Value::Object(entry)) = bodies.get_mut(id.as_str()) {
                    entry.insert("elements".to_owned(), json!(digest.to_hex()));
                }
            }
        }
        let value = json!({
            "model": self.model(),
            "epoch": sky.epoch(),
            "rate": sky.rate_permille(),
            "bodies": bodies,
        });
        DefinitionDigest::for_canonical_value(SKY_DIGEST_DOMAIN, &value)
            .map_err(|error| SkyError::Encoding(error.to_string()))
    }
}

/// The circular model's distance: [`separation_km`] over the two orbits and the
/// phase difference, exactly as [`Ephemeris::distance_km_at`] takes it.
fn circular_distance(
    sky: &Ephemeris,
    a: &CanonicalId,
    ta: i64,
    b: &CanonicalId,
    tb: i64,
) -> Option<u64> {
    let (body_a, body_b) = (sky.body(a)?, sky.body(b)?);
    let delta = sky
        .phase_mdeg_at(a, ta)?
        .checked_sub(sky.phase_mdeg_at(b, tb)?)?;
    Some(separation_km(body_a.orbit_uau, body_b.orbit_uau, delta))
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

    fn circular(rate: u32, ceres_epoch: Option<i64>, mars_epoch: Option<i64>) -> Ephemeris {
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
            )
    }

    fn sky_with(rate: u32, ceres_epoch: Option<i64>, mars_epoch: Option<i64>) -> Sky {
        Sky::Circular(circular(rate, ceres_epoch, mars_epoch))
    }

    /// Ceres's osculating row at the lane's epoch (Horizons, 2351-05-13).
    fn ceres_row() -> ElementRow {
        ElementRow::from_authored(&[
            74_152_540_800.0,
            2.767_755_414,
            0.075_862_114,
            10.550_296,
            75.280_015,
            84.918_514,
            114.238_750,
            0.214_048_659,
        ])
        .expect("a row")
    }

    fn kepler(rate: u32) -> Sky {
        let mut tables = BTreeMap::new();
        tables.insert(cid("ceres"), vec![ceres_row()]);
        Sky::Kepler(KeplerSky::new(
            circular(rate, Some(74_152_540_800), Some(74_152_540_800)),
            tables,
        ))
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

    #[test]
    fn a_kepler_body_stands_where_the_circular_one_does_at_the_epoch() {
        let sky = kepler(1_000);
        let e = 74_152_540_800;
        let ceres = cid("ceres");
        assert!(sky.is_keplerian(&ceres));
        assert!(!sky.is_keplerian(&cid("mars")));
        assert_eq!(sky.model(), "kepler");
        let (r, lon, lat) = sky.heliocentric(&ceres, e).expect("a position");
        #[allow(clippy::cast_precision_loss)]
        let authored_km = 2.866_568 * AU_KM as f64;
        assert!(
            (r - authored_km).abs() < 300.0,
            "r {r} against {authored_km}"
        );
        assert!((lon - 281.654).abs() < 2e-3, "λ {lon}");
        assert!((lat - -4.73).abs() < 2e-3, "β {lat}");
        // The rounded position and the f64 one agree to the kilometre.
        let at = sky.position_km(&ceres, e).expect("a position");
        assert_ne!(at.z, 0, "a Keplerian body has latitude");
        let km = sky
            .distance_km(&ceres, e, &cid("mars"), e)
            .expect("a distance");
        let mars = sky.position_km(&cid("mars"), e).expect("a position");
        assert_eq!(km, at.distance_km(mars));
    }

    #[test]
    fn the_rate_holds_or_moves_a_kepler_body() {
        let e = 74_152_540_800;
        let year = 365 * 86_400;
        let held = kepler(0);
        let ceres = cid("ceres");
        assert_eq!(
            held.position_km(&ceres, e),
            held.position_km(&ceres, e + year),
            "rate 0 holds a Keplerian body at its epoch"
        );
        let moving = kepler(1_000);
        assert_ne!(
            moving.position_km(&ceres, e),
            moving.position_km(&ceres, e + year),
            "rate 1000 moves it"
        );
        // Half rate: a year on reads the half-year sky.
        let half = {
            let mut tables = BTreeMap::new();
            tables.insert(ceres.clone(), vec![ceres_row()]);
            Sky::Kepler(KeplerSky::new(circular(500, Some(e), Some(e)), tables))
        };
        assert_eq!(
            half.position_km(&ceres, e + year),
            moving.position_km(&ceres, e + year / 2)
        );
    }

    #[test]
    fn a_circular_pair_in_a_kepler_sky_keeps_the_slices_arithmetic() {
        let e = 74_152_540_800;
        let mut tables = BTreeMap::new();
        tables.insert(cid("pallas"), vec![ceres_row()]);
        let plain = circular(1_000, Some(e), Some(e));
        let with_table = Sky::Kepler(KeplerSky::new(
            plain.clone().with_body(
                cid("pallas"),
                Body {
                    orbit_uau: 2_866_568,
                    period_mdays: 1_681_861,
                    phase_mdeg: 281_654,
                    epoch_ic: Some(e),
                },
            ),
            tables,
        ));
        let (ceres, mars) = (cid("ceres"), cid("mars"));
        for t in [e - 100 * 86_400, e, e + 400 * 86_400] {
            assert_eq!(
                with_table.distance_km(&ceres, t, &mars, t),
                Sky::Circular(plain.clone()).distance_km(&ceres, t, &mars, t)
            );
            assert_eq!(
                with_table.distance_km(&ceres, t, &mars, t),
                plain.distance_km_at(&ceres, &mars, t)
            );
        }
    }

    #[test]
    fn the_kepler_revision_covers_the_table() {
        let e = 74_152_540_800;
        let one = kepler(0).revision().expect("encodes");
        let same = kepler(0).revision().expect("encodes");
        assert_eq!(one, same);
        let plain = sky(0, Some(e)).revision().expect("encodes");
        assert_ne!(one, plain, "a table is a different sky");
        let mut other_row = ceres_row().authored();
        other_row[6] += 1.0;
        let mut tables = BTreeMap::new();
        tables.insert(
            cid("ceres"),
            vec![ElementRow::from_authored(&other_row).expect("a row")],
        );
        let moved = Sky::Kepler(KeplerSky::new(circular(0, Some(e), Some(e)), tables))
            .revision()
            .expect("encodes");
        assert_ne!(one, moved, "one row's mean anomaly is in the revision");
    }

    #[test]
    fn the_integer_distance_is_the_euclidean_one() {
        let a = PosKm { x: 3, y: 4, z: 12 };
        let o = PosKm { x: 0, y: 0, z: 0 };
        assert_eq!(a.distance_km(o), 13);
        assert_eq!(o.distance_km(a), 13);
        let far = PosKm {
            x: i64::MAX / 4,
            y: -(i64::MAX / 4),
            z: 0,
        };
        assert!(far.distance_km(o) > 0);
        let light = kepler(0)
            .light_lag_s(&cid("ceres"), 0, &cid("mars"), 0)
            .expect("a lag");
        assert!(
            light > 60,
            "minutes of light between Ceres and Mars: {light}"
        );
    }
}

//! **The Kepler propagation behind the moving sky** (doc 19 §3, D-19-2 … D-19-4;
//! cut A.1).
//!
//! A body's motion is authored as an **element table**: osculating Keplerian
//! elements (heliocentric ecliptic J2000) at a sorted list of epochs — in the
//! lane, January 1 and July 1 of every year of the campaign window plus the
//! lane's own epoch, straight from JPL Horizons (`content/sky/fetch_elements.py`).
//! At an instant between two rows each row's two-body propagation is taken and
//! the two are blended linearly in time; at a row the position is the row's,
//! exact; outside the table the nearest end row propagates alone. Measured
//! against Horizons every 91 days over 2340–2400 the blend holds every lane body
//! within 0.07 ‰ of its heliocentric distance (Earth within 1.1 ‰: the Moon's
//! wobble), where any single fitted element set drifts by one to five percent —
//! Jupiter's periodic pull on an asteroid is not a secular rate.
//!
//! **The calendar** (D-19-3). The game clock is `mush_core::CivilDate` (365-day
//! years, year 0 at IC 0) and a real date maps to it **by its label**: the sky a
//! player sees on the game date 2351-05-13 is the real sky of 2351-05-13. So an
//! instant is propagated over real days — the Julian Day of its label
//! ([`julian_day`]) — and every row's epoch is read the same way, which is why
//! the Python's and the Rust's numbers agree to the kilometre.
//!
//! Every field is an integer of fixed scale (nano-AU, nano for the
//! eccentricity, micro-degrees, nano-degrees per day), so a row is `Eq`, a
//! pack digest is over integers, and the authored decimals round-trip exactly.
//! The propagation itself is `f64` with a fixed Newton count and rounds to the
//! kilometre at the end; a committed passage is frozen (T1), so a platform's
//! last-place difference in `sin` is harmless (D-19-4).

use mush_core::CivilDate;

use crate::ephemeris::AU_KM;

/// How many authored numbers a row has: `[epoch_ic, a_au, e, i_deg, node_deg,
/// peri_deg, mean_anomaly_deg, mean_motion_deg_day]`.
pub const ROW_FIELDS: usize = 8;
/// The most rows a body may author (a semi-annual table spans 256 years).
pub const MAX_ELEMENT_ROWS: usize = 512;
/// The fewest rows a table may have: one (a single osculating set, propagated
/// alone).
pub const MIN_ELEMENT_ROWS: usize = 1;
/// The largest semi-major axis a row may author, nano-AU (1,000 AU).
pub const MAX_A_NAU: u64 = 1_000_000_000_000;
/// The largest eccentricity a row may author, in nano: `0.99` — a bound orbit
/// the solver converges on in its fixed step count.
pub const MAX_E_NANO: u64 = 990_000_000;
/// The slowest mean motion a row may author, nano-degrees per day: a thousand-
/// year period.
pub const MIN_N_NDEG_DAY: i64 = 360_000_000_000 / 365_250;
/// The Newton steps Kepler's equation is given, every time (deterministic).
pub const NEWTON_STEPS: usize = 16;

const SECS_PER_DAY: i64 = 86_400;
const NANO: f64 = 1e9;
const MICRO: f64 = 1e6;
const UDEG_TURN: i64 = 360_000_000;

/// **One row of an element table**: osculating elements at `epoch_ic`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ElementRow {
    /// The IC second (game calendar) the elements are osculating at.
    pub epoch_ic: i64,
    /// Semi-major axis, nano-AU; `1..=MAX_A_NAU`.
    pub a_nau: u64,
    /// Eccentricity × 10⁹; `0..=MAX_E_NANO`.
    pub e_nano: u64,
    /// Inclination to the ecliptic, micro-degrees; `0..=180_000_000`.
    pub i_udeg: i64,
    /// Longitude of the ascending node Ω, micro-degrees in `0..360_000_000`.
    pub node_udeg: i64,
    /// Argument of perihelion ω, micro-degrees in `0..360_000_000`.
    pub peri_udeg: i64,
    /// Mean anomaly at `epoch_ic`, micro-degrees in `0..360_000_000`.
    pub ma_udeg: i64,
    /// Mean motion, nano-degrees per day; `MIN_N_NDEG_DAY..`.
    pub n_ndeg_day: i64,
}

impl ElementRow {
    /// From the authored numbers, in [`ROW_FIELDS`] order.
    ///
    /// # Errors
    ///
    /// A message naming the field, for a non-finite value or one past its bound.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub fn from_authored(row: &[f64; ROW_FIELDS]) -> Result<ElementRow, String> {
        let [epoch, a, e, i, node, peri, ma, n] = *row;
        for (name, value) in [
            ("epoch_ic", epoch),
            ("a_au", a),
            ("e", e),
            ("i_deg", i),
            ("node_deg", node),
            ("peri_deg", peri),
            ("mean_anomaly_deg", ma),
            ("mean_motion_deg_day", n),
        ] {
            if !value.is_finite() {
                return Err(format!("an element row's {name} is a number"));
            }
        }
        if epoch.fract() != 0.0 || epoch.abs() > crate::pack::MAX_EPOCH_IC as f64 {
            return Err("an element row's epoch_ic is a whole IC second in range".to_owned());
        }
        let a_nau = (a * NANO).round();
        if a_nau < 1.0 || a_nau > MAX_A_NAU as f64 {
            return Err("an element row's a_au is out of range".to_owned());
        }
        let e_nano = (e * NANO).round();
        if e_nano < 0.0 || e_nano > MAX_E_NANO as f64 {
            return Err("an element row's e is 0 to 0.99".to_owned());
        }
        let i_udeg = (i * MICRO).round();
        if !(0.0..=180_000_000.0).contains(&i_udeg) {
            return Err("an element row's i_deg is 0 to 180".to_owned());
        }
        let n_ndeg = (n * NANO).round();
        if n_ndeg < MIN_N_NDEG_DAY as f64 || n_ndeg > i64::MAX as f64 / 4.0 {
            return Err("an element row's mean_motion_deg_day is out of range".to_owned());
        }
        let angle = |value: f64| -> i64 { ((value * MICRO).round() as i64).rem_euclid(UDEG_TURN) };
        Ok(ElementRow {
            epoch_ic: epoch as i64,
            a_nau: a_nau as u64,
            e_nano: e_nano as u64,
            i_udeg: i_udeg as i64,
            node_udeg: angle(node),
            peri_udeg: angle(peri),
            ma_udeg: angle(ma),
            n_ndeg_day: n_ndeg as i64,
        })
    }

    /// The authored numbers, in [`ROW_FIELDS`] order (the exact decimals the
    /// integers came from).
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn authored(&self) -> [f64; ROW_FIELDS] {
        [
            self.epoch_ic as f64,
            self.a_nau as f64 / NANO,
            self.e_nano as f64 / NANO,
            self.i_udeg as f64 / MICRO,
            self.node_udeg as f64 / MICRO,
            self.peri_udeg as f64 / MICRO,
            self.ma_udeg as f64 / MICRO,
            self.n_ndeg_day as f64 / NANO,
        ]
    }

    /// The row's Julian Day: its epoch's calendar label (D-19-3).
    #[must_use]
    pub fn julian_day(&self) -> f64 {
        julian_day(self.epoch_ic)
    }
}

/// **The Julian Day of an IC instant's calendar label** (D-19-3): the game
/// calendar's `YYYY-MM-DD` read as the Gregorian date of the same name (the
/// standard formula; the game has no February 29, so every label is a date),
/// plus the fraction of the day.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn julian_day(ic: i64) -> f64 {
    let date = CivilDate::from_ic_seconds(ic);
    let (y, m, d) = (date.year, i64::from(date.month), i64::from(date.day));
    let a = (14 - m).div_euclid(12);
    let yy = y + 4800 - a;
    let mm = m + 12 * a - 3;
    let jdn = d + (153 * mm + 2).div_euclid(5) + 365 * yy + yy.div_euclid(4) - yy.div_euclid(100)
        + yy.div_euclid(400)
        - 32045;
    let day_fraction = ic.rem_euclid(SECS_PER_DAY) as f64 / SECS_PER_DAY as f64;
    jdn as f64 - 0.5 + day_fraction
}

/// **One row's two-body position at `jd`**, heliocentric ecliptic J2000,
/// kilometres (`f64`): the mean anomaly advanced by the row's mean motion,
/// Kepler's equation by [`NEWTON_STEPS`] Newton steps from `E = M` (or `π` past
/// `e = 0.8`), the orbital-plane position rotated by ω, i, Ω.
#[must_use]
#[allow(clippy::cast_precision_loss, clippy::many_single_char_names)]
pub fn propagate(row: &ElementRow, jd: f64) -> [f64; 3] {
    let dt_days = jd - row.julian_day();
    let a = row.a_nau as f64 / NANO;
    let e = (row.e_nano as f64 / NANO).clamp(0.0, 0.99);
    let inc = (row.i_udeg as f64 / MICRO).to_radians();
    let node = (row.node_udeg as f64 / MICRO).to_radians();
    let peri = (row.peri_udeg as f64 / MICRO).to_radians();
    let n = row.n_ndeg_day as f64 / NANO;
    let m = (row.ma_udeg as f64 / MICRO + n * dt_days)
        .rem_euclid(360.0)
        .to_radians();
    let mut big_e = if e < 0.8 { m } else { std::f64::consts::PI };
    for _ in 0..NEWTON_STEPS {
        big_e -= (big_e - e * big_e.sin() - m) / (1.0 - e * big_e.cos());
    }
    let xp = a * (big_e.cos() - e);
    let yp = a * (1.0 - e * e).sqrt() * big_e.sin();
    let (cw, sw) = (peri.cos(), peri.sin());
    let (co, so) = (node.cos(), node.sin());
    let (ci, si) = (inc.cos(), inc.sin());
    let x = (co * cw - so * sw * ci) * xp + (-co * sw - so * cw * ci) * yp;
    let y = (so * cw + co * sw * ci) * xp + (-so * sw + co * cw * ci) * yp;
    let z = (sw * si) * xp + (cw * si) * yp;
    let au = AU_KM as f64;
    [x * au, y * au, z * au]
}

/// **A table's position at `jd`**, kilometres (`f64`): between two rows the
/// two propagations blended linearly in time; at or past an end, that row's
/// propagation alone. `None` for an empty table. Rows must be sorted by epoch
/// (the pack validator's rule).
#[must_use]
pub fn table_position(rows: &[ElementRow], jd: f64) -> Option<[f64; 3]> {
    let (first, last) = (rows.first()?, rows.last()?);
    if jd <= first.julian_day() {
        return Some(propagate(first, jd));
    }
    if jd >= last.julian_day() {
        return Some(propagate(last, jd));
    }
    // The last row at or before `jd` (the first row is, by the checks above).
    let index = rows.partition_point(|row| row.julian_day() <= jd);
    let lower = rows.get(index.checked_sub(1)?)?;
    let upper = rows.get(index)?;
    let (jd0, jd1) = (lower.julian_day(), upper.julian_day());
    if jd1 <= jd0 {
        return Some(propagate(lower, jd));
    }
    let u = (jd - jd0) / (jd1 - jd0);
    let p0 = propagate(lower, jd);
    let p1 = propagate(upper, jd);
    Some([
        (1.0 - u) * p0[0] + u * p1[0],
        (1.0 - u) * p0[1] + u * p1[1],
        (1.0 - u) * p0[2] + u * p1[2],
    ])
}

/// Heliocentric distance, longitude and latitude (km, degrees in `0..360`,
/// degrees in `-90..=90`) of a position.
#[must_use]
pub fn polar(position: [f64; 3]) -> (f64, f64, f64) {
    let [x, y, z] = position;
    let r = (x * x + y * y + z * z).sqrt();
    let longitude = y.atan2(x).to_degrees().rem_euclid(360.0);
    let latitude = if r > 0.0 {
        (z / r).clamp(-1.0, 1.0).asin().to_degrees()
    } else {
        0.0
    };
    (r, longitude, latitude)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ceres at the lane's epoch, from Horizons ELEMENTS (2351-05-13).
    fn ceres() -> ElementRow {
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

    #[test]
    fn the_julian_day_is_the_labels() {
        // 2351-05-13 00:00 on the game calendar (74,152,540,800) is JD 2579876.5
        // on the real one (the slice's Horizons script, `ELEMENTS_EPOCH_JD`).
        assert!((julian_day(74_152_540_800) - 2_579_876.5).abs() < 1e-9);
        // 2000-01-01 12:00 is J2000.0 = JD 2451545.0.
        let j2000 = CivilDate {
            year: 2000,
            month: 1,
            day: 1,
            hour: 12,
            minute: 0,
            second: 0,
        }
        .to_ic_seconds();
        assert!((julian_day(j2000) - 2_451_545.0).abs() < 1e-9);
        // Half a day on is half a Julian Day on.
        assert!((julian_day(74_152_540_800 + 43_200) - 2_579_877.0).abs() < 1e-9);
    }

    #[test]
    fn a_row_round_trips_its_authored_decimals() {
        let row = ceres();
        let again = ElementRow::from_authored(&row.authored()).expect("a row");
        assert_eq!(row, again);
        assert_eq!(row.a_nau, 2_767_755_414);
        assert_eq!(row.e_nano, 75_862_114);
        assert_eq!(row.n_ndeg_day, 214_048_659);
    }

    #[test]
    fn the_wire_bounds_hold() {
        let mut bad = ceres().authored();
        bad[2] = 0.995;
        assert!(ElementRow::from_authored(&bad).is_err(), "e past 0.99");
        let mut bad = ceres().authored();
        bad[3] = -1.0;
        assert!(
            ElementRow::from_authored(&bad).is_err(),
            "a negative inclination"
        );
        let mut bad = ceres().authored();
        bad[7] = 0.0;
        assert!(ElementRow::from_authored(&bad).is_err(), "no motion");
        let mut bad = ceres().authored();
        bad[0] = 0.5;
        assert!(
            ElementRow::from_authored(&bad).is_err(),
            "a fractional epoch"
        );
        let mut wraps = ceres().authored();
        wraps[4] = 435.280_015;
        assert_eq!(
            ElementRow::from_authored(&wraps).expect("a row").node_udeg,
            ceres().node_udeg,
            "an angle is taken modulo a turn"
        );
    }

    #[test]
    fn ceres_at_its_epoch_is_where_the_slice_put_it() {
        // Lane v3 authored Ceres at 2.866568 AU and 281.654° on 2351-05-13 from
        // Horizons VECTORS (doc 18 cut IC2); the osculating elements of the same
        // instant must put it there.
        let (r, lon, lat) = polar(propagate(&ceres(), ceres().julian_day()));
        assert!((r / AU_KM as f64 - 2.866_568).abs() < 2e-6, "r {r}");
        assert!((lon - 281.654).abs() < 2e-3, "λ {lon}");
        assert!((lat - -4.73).abs() < 2e-3, "β {lat}");
    }

    #[test]
    fn a_table_is_exact_at_its_rows_and_blends_between_them() {
        let first = ceres();
        let mut second = first.authored();
        second[0] += 182.0 * 86_400.0;
        second[6] = (second[6] + 0.214_048_659 * 182.0) % 360.0;
        let second = ElementRow::from_authored(&second).expect("a row");
        let rows = [first, second];
        for row in &rows {
            let alone = propagate(row, row.julian_day());
            let table = table_position(&rows, row.julian_day()).expect("a position");
            assert_eq!(alone, table, "exact at a row");
        }
        let mid = (first.julian_day() + second.julian_day()) / 2.0;
        let blended = table_position(&rows, mid).expect("a position");
        let (p0, p1) = (propagate(&first, mid), propagate(&second, mid));
        for k in 0..3 {
            assert!(((p0[k] + p1[k]) / 2.0 - blended[k]).abs() < 1e-3);
        }
        assert!(table_position(&[], mid).is_none());
        // Past the last row the last row propagates alone.
        let late = second.julian_day() + 400.0;
        assert_eq!(table_position(&rows, late), Some(propagate(&second, late)));
    }
}

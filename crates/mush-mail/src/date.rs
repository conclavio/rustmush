//! Tiny hand-rolled UTC date formatting (no chrono dependency).
//!
//! **Timezone policy:** all mail timestamps are stored and rendered in
//! UTC. The terminal shows UTC (suffixed where the full form is used); the
//! web client receives the raw Unix timestamp and localizes it
//! client-side (M4).
//!
//! The civil-date math is the standard days-from-civil inverse
//! (Howard Hinnant's `civil_from_days`), exact over the whole `i64` Unix
//! range we care about, proleptic Gregorian.

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Broken-down UTC time: `(year, month 1-12, day 1-31, hour, minute)`.
#[must_use]
pub fn civil_from_unix(ts: i64) -> (i64, u32, u32, u32, u32) {
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = yoe + era * 400 + i64::from(m <= 2);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    (
        y,
        m as u32,
        d as u32,
        (secs / 3600) as u32,
        (secs % 3600 / 60) as u32,
    )
}

/// The classic mail-listing date column: `"Aug 27"` (UTC).
#[must_use]
pub fn format_date(ts: i64) -> String {
    let (_, m, d, _, _) = civil_from_unix(ts);
    format!("{} {d:02}", MONTHS[(m - 1) as usize])
}

/// The full header form: `"Aug 27 2026 14:03 UTC"`.
#[must_use]
pub fn format_datetime(ts: i64) -> String {
    let (y, m, d, hh, mm) = civil_from_unix(ts);
    format!(
        "{} {d:02} {y} {hh:02}:{mm:02} UTC",
        MONTHS[(m - 1) as usize]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_timestamps() {
        // The Unix epoch.
        assert_eq!(format_datetime(0), "Jan 01 1970 00:00 UTC");
        assert_eq!(format_date(0), "Jan 01");
        // 2001-09-09 01:46:40 UTC — the billennium.
        assert_eq!(format_datetime(1_000_000_000), "Sep 09 2001 01:46 UTC");
        assert_eq!(format_date(1_000_000_000), "Sep 09");
        // 2023-11-14 22:13:20 UTC.
        assert_eq!(format_datetime(1_700_000_000), "Nov 14 2023 22:13 UTC");
        // Leap day 2024: 2024-02-29 00:00:00 UTC.
        assert_eq!(format_datetime(1_709_164_800), "Feb 29 2024 00:00 UTC");
        // Pre-epoch: one second before 1970.
        assert_eq!(format_datetime(-1), "Dec 31 1969 23:59 UTC");
    }

    #[test]
    fn civil_parts() {
        // 2000-03-01 00:00:00 UTC (day after the century leap day).
        assert_eq!(civil_from_unix(951_868_800), (2000, 3, 1, 0, 0));
        // End of a day.
        assert_eq!(civil_from_unix(86_399), (1970, 1, 1, 23, 59));
    }
}

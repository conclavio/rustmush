//! The pure weather state machine: transition rolls, temperature, and line
//! selection. Everything here is a pure function of a [`ClimateDef`], the
//! current state/season/time-of-day, and a [`WxRng`] — no world access, no
//! I/O — so it is exhaustively testable and deterministic under a seeded RNG
//! (the design's requirement for reproducible transitions).

use mush_core::{Season, TimeOfDay};
use smol_str::SmolStr;

use crate::cell::WxRng;
use crate::climate::{ClimateDef, WxStateDef, season_key, tod_key};

/// Roll one transition for a cell currently in `from`, under `season` /
/// `tod`, using `climate`'s matrix modified by its time-of-day table. The
/// result may equal `from` (a self-loop / "no change"). Falls back to `from`
/// when the matrix has no row for the current state.
#[must_use]
pub fn roll_transition(
    climate: &ClimateDef,
    from: &SmolStr,
    season: Season,
    tod: TimeOfDay,
    rng: &mut WxRng,
) -> SmolStr {
    let Some(matrix) = climate.seasonal.get(season_key(season)) else {
        return from.clone();
    };
    let Some(row) = matrix.get(from) else {
        return from.clone();
    };
    let tod_mods = climate.tod.get(tod_key(tod));

    let mut weighted: Vec<(&SmolStr, u64)> = Vec::with_capacity(row.len());
    let mut total: u64 = 0;
    for (to, base) in row {
        let mut weight = u64::from(*base);
        if let Some(mods) = tod_mods
            && let Some(pct) = mods.get(to)
        {
            weight = weight.saturating_mul(u64::from(*pct)) / 100;
        }
        if weight > 0 {
            weighted.push((to, weight));
            total += weight;
        }
    }
    if total == 0 {
        return from.clone();
    }
    let mut pick = rng.next_u64() % total;
    for (to, weight) in weighted {
        if pick < weight {
            return to.clone();
        }
        pick -= weight;
    }
    from.clone() // unreachable: pick < total
}

/// Roll a temperature within a state's band, biased by season (colder in
/// winter, warmer in summer) with a small deterministic jitter.
#[must_use]
pub fn roll_temp(def: &WxStateDef, season: Season, rng: &mut WxRng) -> i16 {
    let (lo, hi) = def.temp_band;
    if hi <= lo {
        return lo;
    }
    let span = i32::from(hi - lo);
    let frac_pct = match season {
        Season::Winter => 18,
        Season::Spring => 50,
        Season::Summer => 82,
        Season::Autumn => 45,
    };
    let center = i32::from(lo) + span * frac_pct / 100;
    let jitter = (span / 5).max(1);
    #[allow(clippy::cast_possible_wrap)]
    let offset = rng.below((2 * jitter + 1) as u32) as i32 - jitter;
    (center + offset).clamp(i32::from(lo), i32::from(hi)) as i16
}

/// Pick the line shown when a zone transitions `from → to`: a specific
/// `"<from>-><to>"` pool wins, else the destination's generic pool, else a
/// plain generated fallback (so a sparse climate file never produces silence
/// on a real state change).
#[must_use]
pub fn pick_transition_line(climate: &ClimateDef, from: &str, to: &str, rng: &mut WxRng) -> String {
    let pair = SmolStr::new(format!("{from}->{to}"));
    if let Some(line) = climate
        .lines
        .transition_pair
        .get(&pair)
        .and_then(|pool| pick(pool, rng))
    {
        return line;
    }
    if let Some(line) = climate
        .lines
        .transition
        .get(to)
        .and_then(|pool| pick(pool, rng))
    {
        return line;
    }
    format!("The weather shifts to {to}.")
}

/// Pick one ambient line for a zone sitting in `state`: the merged pool of
/// the state's ambient lines plus the generic season and time-of-day mood
/// pools. `None` when every relevant pool is empty (the caller then emits
/// nothing for this room this tick).
#[must_use]
pub fn pick_ambient_line(
    climate: &ClimateDef,
    state: &str,
    season: Season,
    tod: TimeOfDay,
    rng: &mut WxRng,
) -> Option<String> {
    let mut pool: Vec<&String> = Vec::new();
    if let Some(v) = climate.lines.ambient.get(state) {
        pool.extend(v);
    }
    if let Some(v) = climate.lines.ambient_season.get(season_key(season)) {
        pool.extend(v);
    }
    if let Some(v) = climate.lines.ambient_tod.get(tod_key(tod)) {
        pool.extend(v);
    }
    if pool.is_empty() {
        return None;
    }
    let idx = rng.below(pool.len() as u32) as usize;
    Some(pool[idx].clone())
}

/// Pick a random line from a pool, or `None` if empty.
fn pick(pool: &[String], rng: &mut WxRng) -> Option<String> {
    if pool.is_empty() {
        return None;
    }
    let idx = rng.below(pool.len() as u32) as usize;
    Some(pool[idx].clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::climate::ClimateSet;

    fn temperate() -> ClimateDef {
        ClimateSet::builtin()
            .unwrap()
            .get("temperate-coastal")
            .unwrap()
            .clone()
    }

    #[test]
    fn transition_is_deterministic_under_a_seed() {
        let c = temperate();
        let run = |seed| {
            let mut rng = WxRng::seeded(seed);
            let mut state = SmolStr::new("clear");
            let mut trail = Vec::new();
            for _ in 0..20 {
                state = roll_transition(&c, &state, Season::Autumn, TimeOfDay::Morning, &mut rng);
                trail.push(state.clone());
            }
            trail
        };
        assert_eq!(run(1234), run(1234), "same seed → same trail");
        assert_ne!(run(1234), run(9999), "different seed → different trail");
    }

    #[test]
    fn transition_only_yields_known_states() {
        let c = temperate();
        let mut rng = WxRng::seeded(5);
        let mut state = SmolStr::new("clear");
        for _ in 0..500 {
            state = roll_transition(&c, &state, Season::Winter, TimeOfDay::Night, &mut rng);
            assert!(c.state(&state).is_some(), "rolled unknown state {state}");
        }
    }

    #[test]
    fn single_target_row_is_forced_regardless_of_rng() {
        // A matrix row with exactly one target must always pick it.
        let toml = r#"
id = "forced"
[[states]]
id = "clear"
visibility = 100
precip = 0
wind = 10
temp_band = [0, 20]
[[states]]
id = "rain"
visibility = 60
precip = 60
wind = 40
temp_band = [2, 18]
[seasonal.winter]
clear = { rain = 100 }
rain = { rain = 100 }
[seasonal.spring]
clear = { rain = 100 }
rain = { rain = 100 }
[seasonal.summer]
clear = { rain = 100 }
rain = { rain = 100 }
[seasonal.autumn]
clear = { rain = 100 }
rain = { rain = 100 }
"#;
        let c = ClimateDef::parse("forced", toml).unwrap();
        for seed in 0..50 {
            let mut rng = WxRng::seeded(seed);
            let got = roll_transition(
                &c,
                &SmolStr::new("clear"),
                Season::Summer,
                TimeOfDay::Afternoon,
                &mut rng,
            );
            assert_eq!(got, "rain");
        }
    }

    #[test]
    fn temp_stays_within_band() {
        let c = temperate();
        let def = c.state("rain").unwrap();
        let mut rng = WxRng::seeded(11);
        for _ in 0..200 {
            let t = roll_temp(def, Season::Winter, &mut rng);
            assert!(
                t >= def.temp_band.0 && t <= def.temp_band.1,
                "temp {t} out of band"
            );
        }
    }

    #[test]
    fn winter_is_colder_than_summer_on_average() {
        let c = temperate();
        let def = c.state("clear").unwrap();
        let mut rng = WxRng::seeded(3);
        let avg = |season, rng: &mut WxRng| {
            let sum: i64 = (0..200)
                .map(|_| i64::from(roll_temp(def, season, rng)))
                .sum();
            sum / 200
        };
        let winter = avg(Season::Winter, &mut rng);
        let summer = avg(Season::Summer, &mut rng);
        assert!(winter < summer, "winter {winter} !< summer {summer}");
    }

    #[test]
    fn transition_line_prefers_specific_pair() {
        let c = temperate();
        let mut rng = WxRng::seeded(1);
        // overcast->rain has a specific pair line in the data file.
        let line = pick_transition_line(&c, "overcast", "rain", &mut rng);
        assert_eq!(line, "The drizzle thickens into steady rain.");
    }

    #[test]
    fn ambient_line_present_for_known_state() {
        let c = temperate();
        let mut rng = WxRng::seeded(2);
        let line = pick_ambient_line(&c, "rain", Season::Autumn, TimeOfDay::Night, &mut rng);
        assert!(line.is_some());
    }
}

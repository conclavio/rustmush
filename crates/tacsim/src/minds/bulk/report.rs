//! Public statistical contract. This game module is intentionally outside
//! arena/: observations do not affect resolution or state hashes.
use super::{BulkError, invalid};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Source-free frozen scenario identity. External acceptance compares these
/// values before comparing intervals; seeds are independent samples unless equal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Context {
    pub scenario: super::model::Scenario,
    pub scenario_content_sha256: String,
    pub ground: String,
    pub ground_sha256: String,
    pub config_sha256: String,
    pub chassis: String,
    pub startup_ms: u32,
    pub max_duration_ms: u32,
    pub native_substep_ms: u32,
    pub candidate: Selection,
    pub opponent: Selection,
    pub pair_seeds: Vec<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Selection {
    pub class: String,
    pub profile: String,
    pub source_sha256: String,
}
impl Context {
    pub fn validate(&self) -> Result<(), BulkError> {
        let digests = [
            &self.scenario_content_sha256,
            &self.ground_sha256,
            &self.config_sha256,
            &self.candidate.source_sha256,
            &self.opponent.source_sha256,
        ];
        let labels = [
            &self.ground,
            &self.chassis,
            &self.candidate.class,
            &self.candidate.profile,
            &self.opponent.class,
            &self.opponent.profile,
        ];
        if digests
            .iter()
            .any(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
            || labels
                .iter()
                .any(|s| s.is_empty() || s.len() > 128 || s.chars().any(char::is_control))
            || !(5..=50).contains(&self.pair_seeds.len())
            || self.max_duration_ms == 0
            || self.native_substep_ms == 0
        {
            return Err(invalid("report frozen context"));
        }
        Ok(())
    }
    pub fn lines(&self) -> Vec<String> {
        vec![
            format!(
                "Frozen {:?}: {}; chassis {}; startup {} ms; limit {} ms; native {} ms.",
                self.scenario,
                self.ground,
                self.chassis,
                self.startup_ms,
                self.max_duration_ms,
                self.native_substep_ms
            ),
            format!(
                "Candidate class {} profile {}; opponent class {} profile {}.",
                self.candidate.class,
                self.candidate.profile,
                self.opponent.class,
                self.opponent.profile
            ),
            format!(
                "Scenario SHA256 {}; ground SHA256 {}.",
                self.scenario_content_sha256, self.ground_sha256
            ),
            format!("Config SHA256 {}.", self.config_sha256),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Interval {
    pub low: f64,
    pub high: f64,
}
/// Wilson score interval, z=1.959963984540054 (95%, two-sided).
/// Wins are successes; losses, draws and timeouts are all in the denominator.
/// An empty sample has no interval, rather than false certainty at zero.
pub(crate) fn wilson_95(wins: u32, total: u32) -> Option<Interval> {
    if total == 0 || wins > total {
        return None;
    }
    let z = 1.959_963_984_540_054_f64;
    let n = f64::from(total);
    let p = f64::from(wins) / n;
    let z2 = z * z;
    let denominator = 1.0 + z2 / n;
    let center = (p + z2 / (2.0 * n)) / denominator;
    let half = z * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt() / denominator;
    Some(Interval {
        low: (center - half).max(0.0),
        high: (center + half).min(1.0),
    })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Win,
    Loss,
    Draw,
    Timeout,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Game {
    pub seed: u64,
    pub swapped: bool,
    pub outcome: Outcome,
    pub elapsed_ms: u32,
    pub units: u8,
    pub final_hash: u64,
    pub ledger_digest: [u8; 32],
    pub section_losses: BTreeMap<String, u32>,
    pub weapon_outcomes: BTreeMap<String, u32>,
    pub odds_buckets: [u32; 10],
    pub facet_outcomes: BTreeMap<String, u32>,
    pub rounds: u32,
    pub rounds_left: u32,
    /// First section-destruction event affected the candidate side. None means
    /// neither side lost a section; ordered ledger events resolve simultaneity.
    pub lost_first_section: Option<bool>,
    pub rules_fired: u32,
    pub rules_held: u32,
    pub faults: u32,
    /// Exact bounded histogram; class budget is at most 4500 inclusive.
    pub operations: Vec<u32>,
    /// Native ROM is deliberately unmetered; no false zero-op observation.
    pub unmeasured_thinks: u32,
    /// Accepted-shot odds not observed by telemetry, never invented buckets.
    pub odds_unobserved: u32,
}
impl Game {
    fn validate(&self) -> Result<(), BulkError> {
        if !(2..=12).contains(&self.units)
            || self.operations.len() != 4501
            || [
                &self.section_losses,
                &self.weapon_outcomes,
                &self.facet_outcomes,
            ]
            .into_iter()
            .any(|map| {
                map.len() > 128
                    || map
                        .keys()
                        .any(|s| s.is_empty() || s.len() > 128 || s.chars().any(char::is_control))
            })
        {
            return Err(invalid("game report dimensions"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Report {
    pub games: Vec<Game>,
}
impl Report {
    pub fn lines(&self) -> Vec<String> {
        let [wins, losses, draws, timeouts] = self.counts();
        let mut lines = vec![format!(
            "Won {wins}; lost {losses}; drew {draws}; timed out {timeouts}."
        )];
        if let Some(i) = self.interval() {
            lines.push(format!(
                "Win rate {wins} of {}: 95% Wilson {:.1} to {:.1}%.",
                self.games.len(),
                i.low * 100.0,
                i.high * 100.0
            ));
        }
        let (first, won) = self.first_section_counts();
        lines.push(format!(
            "Lost the first section in {first}; won {won} of those."
        ));
        let sum = |f: fn(&Game) -> u32| self.games.iter().map(|g| u64::from(f(g))).sum::<u64>();
        lines.push(format!(
            "Rounds fired {}; candidate rounds left {}; rules fired {}, held {}; faults {}.",
            sum(|g| g.rounds),
            sum(|g| g.rounds_left),
            sum(|g| g.rules_fired),
            sum(|g| g.rules_held),
            sum(|g| g.faults)
        ));
        lines.push(format!(
            "Ops p50 {}, p99 {}; unmeasured native thinks {}.",
            self.operations_percentile(50)
                .map_or_else(|| "unmeasured".into(), |n| n.to_string()),
            self.operations_percentile(99)
                .map_or_else(|| "unmeasured".into(), |n| n.to_string()),
            sum(|g| g.unmeasured_thinks)
        ));
        let mut odds = [0_u64; 10];
        for game in &self.games {
            for (n, count) in odds.iter_mut().zip(game.odds_buckets) {
                *n += u64::from(count);
            }
        }
        lines.push(format!(
            "Accepted-shot odds buckets 0–9%, 10–19%, …, 90–100%: {}.",
            odds.iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
        for (label, select) in [
            (
                "Sections",
                (|g: &Game| &g.section_losses) as fn(&Game) -> &BTreeMap<String, u32>,
            ),
            ("Weapons", |g: &Game| &g.weapon_outcomes),
            ("Facets", |g: &Game| &g.facet_outcomes),
        ] {
            let mut values = BTreeMap::<String, u64>::new();
            for g in &self.games {
                for (key, n) in select(g) {
                    *values.entry(key.clone()).or_default() += u64::from(*n);
                }
            }
            for (key, n) in values.iter().take(45) {
                lines.push(format!("{label}: {key} {n}"));
            }
            if values.len() > 45 {
                lines.push(format!(
                    "{label}: {} further categories retained in the complete artifact.",
                    values.len() - 45
                ));
            }
        }
        for pair in self.games.chunks(2) {
            if let Some(first) = pair.first() {
                lines.push(format!(
                    "Seed {}: normal {:?}; swapped {:?}.",
                    first.seed,
                    first.outcome,
                    pair.get(1).map(|g| g.outcome)
                ));
            }
        }
        lines
    }
    pub fn validate(&self) -> Result<(), BulkError> {
        if self.games.len() > 100 || !self.games.len().is_multiple_of(2) {
            return Err(invalid("report paired game count"));
        }
        let mut seeds = std::collections::BTreeSet::new();
        for [normal, swapped] in self.games.as_chunks::<2>().0 {
            normal.validate()?;
            swapped.validate()?;
            if normal.seed != swapped.seed
                || normal.swapped
                || !swapped.swapped
                || !seeds.insert(normal.seed)
            {
                return Err(invalid("report seed pairing"));
            }
        }
        Ok(())
    }
    pub fn counts(&self) -> [u32; 4] {
        let mut totals = [0; 4];
        let [wins, losses, draws, timeouts] = &mut totals;
        for g in &self.games {
            match g.outcome {
                Outcome::Win => *wins += 1,
                Outcome::Loss => *losses += 1,
                Outcome::Draw => *draws += 1,
                Outcome::Timeout => *timeouts += 1,
            }
        }
        totals
    }
    pub fn interval(&self) -> Option<Interval> {
        wilson_95(self.counts()[0], self.games.len() as u32)
    }
    pub fn first_section_counts(&self) -> (u32, u32) {
        self.games
            .iter()
            .filter(|g| g.lost_first_section == Some(true))
            .fold((0, 0), |(lost, won), g| {
                (lost + 1, won + u32::from(g.outcome == Outcome::Win))
            })
    }
    pub fn operations_percentile(&self, percent: u32) -> Option<u32> {
        if !(1..=100).contains(&percent) {
            return None;
        }
        let mut counts = vec![0_u64; 4501];
        for game in &self.games {
            for (slot, n) in counts.iter_mut().zip(&game.operations) {
                *slot = slot.saturating_add(u64::from(*n))
            }
        }
        let total = counts.iter().copied().sum::<u64>();
        if total == 0 {
            return None;
        }
        let rank = total.saturating_mul(u64::from(percent)).div_ceil(100);
        let mut seen = 0_u64;
        counts.into_iter().enumerate().find_map(|(ops, n)| {
            seen = seen.saturating_add(n);
            (seen >= rank).then_some(ops as u32)
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn published_wilson_fixtures() {
        assert_eq!(wilson_95(0, 0), None);
        assert_eq!(wilson_95(11, 10), None);
        for (wins, total, low, high) in [
            (0, 10, 0.0, 0.2775327998628892),
            (5, 10, 0.236593090512564, 0.763406909487436),
            (10, 10, 0.7224672001371107, 1.0),
        ] {
            let actual = wilson_95(wins, total).expect("interval");
            assert!((actual.low - low).abs() < 1e-12);
            assert!((actual.high - high).abs() < 1e-12);
        }
    }
}

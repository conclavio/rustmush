//! Audit log (design 03 §4.3). Every call — success, refusal, error, budget
//! refusal, or kill-switch — leaves one attributed row.
//!
//! **Persistence seam (M7):** rows live in a bounded in-memory ring and are
//! mirrored to `tracing`. The durable path is an `XADD` to a
//! `mod:{ns}:llm_audit` stream via `ModuleOp::StreamAppend` (owned by the
//! worldgen module / `mushd`); mush-llm keeps no store handle. Full prompt
//! text is deliberately **not** stored (it contains player speech) — only a
//! SHA-256 and a truncated summary.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use mush_core::ObjId;
use sha2::{Digest, Sha256};

/// How a call turned out.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Completed normally.
    Ok,
    /// The model refused.
    Refused,
    /// A provider/transport error.
    Error,
    /// Refused pre-call by a budget ceiling.
    Budget,
    /// Refused pre-call by a kill switch.
    Disabled,
}

impl Outcome {
    /// The wire label.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Refused => "refused",
            Outcome::Error => "error",
            Outcome::Budget => "budget",
            Outcome::Disabled => "disabled",
        }
    }
}

/// One audit row.
#[derive(Clone, Debug, PartialEq)]
pub struct AuditRecord {
    /// Wall-clock unix seconds.
    pub ts: u64,
    /// Feature route name.
    pub feature: String,
    /// Provider id.
    pub provider: String,
    /// Model id (as routed).
    pub model: String,
    /// Triggering actor (builder/player), if any.
    pub actor: Option<ObjId>,
    /// NPC, if this was an agent turn.
    pub npc: Option<ObjId>,
    /// SHA-256 of the full assembled prompt (hex).
    pub prompt_sha256: String,
    /// First ≤200 chars of the last user turn.
    pub prompt_summary: String,
    /// First ≤200 chars of the response text.
    pub response_summary: String,
    /// Input tokens.
    pub input_tokens: u64,
    /// Output tokens.
    pub output_tokens: u64,
    /// Cost, USD (authoritative or estimated), if known.
    pub cost_usd: Option<f64>,
    /// End-to-end latency, ms.
    pub latency_ms: u64,
    /// Result.
    pub outcome: Outcome,
}

/// Cumulative LLM counters for one `(feature, provider)` pair since boot, as
/// aggregated by [`AuditSink`]. Unlike the bounded [`AuditSink`] ring (which
/// only keeps the most recent rows), these are monotonic totals suitable for
/// a `/metrics` exposition — they never contain prompt text, keys, or any
/// per-player attribution (only the feature route and provider id).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FeatureTotals {
    /// Feature route name.
    pub feature: String,
    /// Provider id.
    pub provider: String,
    /// Total calls recorded (every outcome).
    pub calls: u64,
    /// Calls the model refused.
    pub refusals: u64,
    /// Calls refused pre-flight by a budget ceiling.
    pub budget_trips: u64,
    /// Calls refused pre-flight by a kill switch.
    pub disabled: u64,
    /// Provider/transport errors.
    pub errors: u64,
    /// Input tokens consumed.
    pub input_tokens: u64,
    /// Output tokens consumed.
    pub output_tokens: u64,
    /// Cost consumed, USD.
    pub cost_usd: f64,
}

/// One UTC-day, one `(feature, provider)` aggregate slice — the LLM cost
/// dashboard's day-by-day series (M10c). Built by scanning the retained
/// ring, so it is **best-effort**: a very busy server may have evicted rows
/// older than the ring's capacity before the window closes (the documented
/// path to an unbounded history is a persisted daily rollup — see the
/// module docs). Aggregates only — no prompt text, no key, no per-player
/// data beyond the feature/provider dimensions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DaySlice {
    /// UTC calendar day, `"YYYY-MM-DD"`.
    pub day: String,
    /// Feature route name.
    pub feature: String,
    /// Provider id.
    pub provider: String,
    /// Calls of any outcome.
    pub calls: u64,
    /// Calls the model refused.
    pub refusals: u64,
    /// Calls refused pre-flight by a budget ceiling.
    pub budget_trips: u64,
    /// Calls refused pre-flight by a kill switch.
    pub disabled: u64,
    /// Provider/transport errors.
    pub errors: u64,
    /// Input tokens consumed.
    pub input_tokens: u64,
    /// Output tokens consumed.
    pub output_tokens: u64,
    /// Cost consumed, USD.
    pub cost_usd: f64,
}

/// The bounded in-memory audit ring plus a `tracing` mirror and a small
/// cumulative aggregate (by feature route + provider) for the metrics surface.
pub struct AuditSink {
    ring: Mutex<VecDeque<AuditRecord>>,
    /// Monotonic per-`(feature, provider)` totals since boot. Separate from
    /// the ring so `/metrics` counters survive the ring's eviction.
    totals: Mutex<BTreeMap<(String, String), FeatureTotals>>,
    capacity: usize,
}

impl std::fmt::Debug for AuditSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let g = self.ring.lock().expect("audit lock");
        write!(f, "AuditSink({}/{} rows)", g.len(), self.capacity)
    }
}

impl Default for AuditSink {
    fn default() -> AuditSink {
        AuditSink::new(1024)
    }
}

impl AuditSink {
    /// A sink retaining the most recent `capacity` rows.
    #[must_use]
    pub fn new(capacity: usize) -> AuditSink {
        AuditSink {
            ring: Mutex::new(VecDeque::with_capacity(capacity.min(1024))),
            totals: Mutex::new(BTreeMap::new()),
            capacity: capacity.max(1),
        }
    }

    /// Record one row (and emit a scrubbed `tracing` event — never the prompt
    /// text, never a key).
    pub fn record(&self, rec: AuditRecord) {
        tracing::info!(
            target: "mush_llm::audit",
            feature = %rec.feature,
            provider = %rec.provider,
            model = %rec.model,
            actor = ?rec.actor,
            npc = ?rec.npc,
            input_tokens = rec.input_tokens,
            output_tokens = rec.output_tokens,
            cost_usd = ?rec.cost_usd,
            latency_ms = rec.latency_ms,
            outcome = rec.outcome.as_str(),
            prompt_sha256 = %rec.prompt_sha256,
            "llm call"
        );
        // Fold into the cumulative aggregate before the ring eviction can lose
        // the row (the aggregate keeps monotonic counters for `/metrics`).
        {
            let mut t = self.totals.lock().expect("audit totals lock");
            let entry = t
                .entry((rec.feature.clone(), rec.provider.clone()))
                .or_insert_with(|| FeatureTotals {
                    feature: rec.feature.clone(),
                    provider: rec.provider.clone(),
                    ..FeatureTotals::default()
                });
            entry.calls += 1;
            match rec.outcome {
                Outcome::Ok => {}
                Outcome::Refused => entry.refusals += 1,
                Outcome::Error => entry.errors += 1,
                Outcome::Budget => entry.budget_trips += 1,
                Outcome::Disabled => entry.disabled += 1,
            }
            entry.input_tokens = entry.input_tokens.saturating_add(rec.input_tokens);
            entry.output_tokens = entry.output_tokens.saturating_add(rec.output_tokens);
            if let Some(c) = rec.cost_usd {
                entry.cost_usd += c;
            }
        }
        let mut g = self.ring.lock().expect("audit lock");
        if g.len() == self.capacity {
            g.pop_front();
        }
        g.push_back(rec);
    }

    /// The cumulative per-`(feature, provider)` totals since boot, sorted by
    /// key. Monotonic — safe as `/metrics` counters (the ring's eviction does
    /// not touch them). Contains no prompt text, key, or per-player data.
    #[must_use]
    pub fn totals(&self) -> Vec<FeatureTotals> {
        self.totals
            .lock()
            .expect("audit totals lock")
            .values()
            .cloned()
            .collect()
    }

    /// The most recent `n` rows, newest last.
    #[must_use]
    pub fn recent(&self, n: usize) -> Vec<AuditRecord> {
        let g = self.ring.lock().expect("audit lock");
        let skip = g.len().saturating_sub(n);
        g.iter().skip(skip).cloned().collect()
    }

    /// Total rows currently retained.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ring.lock().expect("audit lock").len()
    }

    /// Whether the ring is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ring.lock().expect("audit lock").is_empty()
    }

    /// Aggregate the retained ring into per-day, per-`(feature, provider)`
    /// buckets covering the last `days` UTC days (today inclusive), sorted
    /// oldest day first. See [`DaySlice`] for the best-effort caveat.
    #[must_use]
    pub fn daily_series(&self, days: u64) -> Vec<DaySlice> {
        let days = days.max(1);
        let today = now_unix() / 86_400;
        let cutoff = today.saturating_sub(days - 1);
        let mut buckets: BTreeMap<(u64, String, String), DaySlice> = BTreeMap::new();
        {
            let g = self.ring.lock().expect("audit lock");
            for r in g.iter() {
                let day = r.ts / 86_400;
                if day < cutoff {
                    continue;
                }
                let slice = buckets
                    .entry((day, r.feature.clone(), r.provider.clone()))
                    .or_insert_with(|| DaySlice {
                        day: iso_date(day),
                        feature: r.feature.clone(),
                        provider: r.provider.clone(),
                        ..DaySlice::default()
                    });
                slice.calls += 1;
                match r.outcome {
                    Outcome::Ok => {}
                    Outcome::Refused => slice.refusals += 1,
                    Outcome::Error => slice.errors += 1,
                    Outcome::Budget => slice.budget_trips += 1,
                    Outcome::Disabled => slice.disabled += 1,
                }
                slice.input_tokens = slice.input_tokens.saturating_add(r.input_tokens);
                slice.output_tokens = slice.output_tokens.saturating_add(r.output_tokens);
                if let Some(c) = r.cost_usd {
                    slice.cost_usd += c;
                }
            }
        }
        let mut rows: Vec<DaySlice> = buckets.into_values().collect();
        rows.sort_by(|a, b| {
            a.day
                .cmp(&b.day)
                .then_with(|| a.feature.cmp(&b.feature))
                .then_with(|| a.provider.cmp(&b.provider))
        });
        rows
    }
}

/// UTC calendar date (`"YYYY-MM-DD"`) for a day number (days since the Unix
/// epoch) — the days-from-civil inverse (Howard Hinnant's
/// `civil_from_days`), exact and dependency-free. Mirrors `mush-mail`'s
/// `civil_from_unix` (same math, from seconds); duplicated in miniature here
/// rather than adding a cross-crate dependency for ~10 lines of arithmetic.
fn iso_date(day: u64) -> String {
    #[allow(clippy::cast_possible_wrap)]
    let z = day as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Hex SHA-256 of a prompt (stable, non-reversible attribution key).
#[must_use]
pub fn prompt_sha256(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    let digest = h.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Truncate to ≤200 chars on a char boundary, appending `…` when clipped.
#[must_use]
pub fn summarize(s: &str) -> String {
    let s = s.trim();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= 200 {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}

/// Current wall-clock unix seconds.
#[must_use]
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_truncates_on_char_boundary() {
        let long = "é".repeat(500);
        let s = summarize(&long);
        assert_eq!(s.chars().count(), 201); // 200 + the ellipsis
        assert!(s.ends_with('…'));
        assert_eq!(summarize("  short  "), "short");
    }

    #[test]
    fn sha256_is_stable_and_hex() {
        let a = prompt_sha256("hello world");
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(a, prompt_sha256("hello world"));
        assert_ne!(a, prompt_sha256("hello world!"));
    }

    #[test]
    fn ring_bounds_capacity() {
        let sink = AuditSink::new(2);
        for i in 0..5 {
            sink.record(AuditRecord {
                ts: i,
                feature: "f".into(),
                provider: "p".into(),
                model: "m".into(),
                actor: None,
                npc: None,
                prompt_sha256: String::new(),
                prompt_summary: String::new(),
                response_summary: String::new(),
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: None,
                latency_ms: 0,
                outcome: Outcome::Ok,
            });
        }
        assert_eq!(sink.len(), 2);
        let recent = sink.recent(10);
        assert_eq!(recent.first().unwrap().ts, 3);
        assert_eq!(recent.last().unwrap().ts, 4);
    }

    fn rec(
        feature: &str,
        provider: &str,
        outcome: Outcome,
        inp: u64,
        out: u64,
        cost: Option<f64>,
    ) -> AuditRecord {
        AuditRecord {
            ts: 0,
            feature: feature.into(),
            provider: provider.into(),
            model: "m".into(),
            actor: None,
            npc: None,
            prompt_sha256: String::new(),
            prompt_summary: String::new(),
            response_summary: String::new(),
            input_tokens: inp,
            output_tokens: out,
            cost_usd: cost,
            latency_ms: 0,
            outcome,
        }
    }

    #[test]
    fn totals_accumulate_by_feature_provider_and_survive_ring_eviction() {
        // Tiny ring (1 row) so the aggregate must not depend on retained rows.
        let sink = AuditSink::new(1);
        sink.record(rec(
            "worldgen_room",
            "openrouter",
            Outcome::Ok,
            100,
            50,
            Some(0.01),
        ));
        sink.record(rec(
            "worldgen_room",
            "openrouter",
            Outcome::Refused,
            10,
            0,
            None,
        ));
        sink.record(rec(
            "worldgen_room",
            "openrouter",
            Outcome::Budget,
            0,
            0,
            None,
        ));
        sink.record(rec(
            "npc_dialogue_major",
            "anthropic",
            Outcome::Ok,
            5,
            5,
            Some(0.002),
        ));
        // Ring only holds the last row, but totals hold everything.
        assert_eq!(sink.len(), 1);
        let totals = sink.totals();
        assert_eq!(totals.len(), 2);
        let wg = totals
            .iter()
            .find(|t| t.feature == "worldgen_room")
            .unwrap();
        assert_eq!(wg.calls, 3);
        assert_eq!(wg.refusals, 1);
        assert_eq!(wg.budget_trips, 1);
        assert_eq!(wg.input_tokens, 110);
        assert_eq!(wg.output_tokens, 50);
        assert!((wg.cost_usd - 0.01).abs() < 1e-9);
        let npc = totals
            .iter()
            .find(|t| t.feature == "npc_dialogue_major")
            .unwrap();
        assert_eq!(npc.calls, 1);
        assert_eq!(npc.provider, "anthropic");
    }

    #[test]
    fn iso_date_matches_known_civil_dates() {
        // Day 0 == the Unix epoch, 1970-01-01 (cross-checked against
        // mush-mail's civil_from_unix(0)).
        assert_eq!(iso_date(0), "1970-01-01");
        // 2024-02-29 (leap day) is day 1_709_164_800 / 86_400.
        assert_eq!(iso_date(1_709_164_800 / 86_400), "2024-02-29");
        // A day after the leap day rolls the month.
        assert_eq!(iso_date(1_709_164_800 / 86_400 + 1), "2024-03-01");
    }

    fn rec_at(ts: u64, feature: &str, provider: &str, outcome: Outcome, cost: f64) -> AuditRecord {
        AuditRecord {
            ts,
            feature: feature.into(),
            provider: provider.into(),
            model: "m".into(),
            actor: None,
            npc: None,
            prompt_sha256: String::new(),
            prompt_summary: String::new(),
            response_summary: String::new(),
            input_tokens: 10,
            output_tokens: 5,
            cost_usd: Some(cost),
            latency_ms: 0,
            outcome,
        }
    }

    #[test]
    fn daily_series_buckets_by_utc_day_and_drops_older_rows() {
        let sink = AuditSink::new(1024);
        let today = now_unix() / 86_400;
        let yesterday_ts = (today - 1) * 86_400;
        let today_ts = today * 86_400;
        let ancient_ts = (today.saturating_sub(30)) * 86_400; // outside a 7-day window

        sink.record(rec_at(
            yesterday_ts,
            "worldgen_room",
            "openrouter",
            Outcome::Ok,
            0.01,
        ));
        sink.record(rec_at(
            today_ts,
            "worldgen_room",
            "openrouter",
            Outcome::Ok,
            0.02,
        ));
        sink.record(rec_at(
            today_ts,
            "worldgen_room",
            "openrouter",
            Outcome::Refused,
            0.0,
        ));
        sink.record(rec_at(
            today_ts,
            "npc_dialogue_major",
            "anthropic",
            Outcome::Budget,
            0.0,
        ));
        sink.record(rec_at(
            ancient_ts,
            "worldgen_room",
            "openrouter",
            Outcome::Ok,
            9.0,
        ));

        let series = sink.daily_series(7);
        // The 30-day-old row falls outside the window entirely.
        assert!(series.iter().all(|s| (s.cost_usd - 9.0).abs() > 1e-9));
        // Oldest day first.
        assert!(series[0].day <= series.last().unwrap().day);

        let today_wg = series
            .iter()
            .find(|s| s.feature == "worldgen_room" && s.day == iso_date(today))
            .expect("today's worldgen_room bucket");
        assert_eq!(today_wg.calls, 2);
        assert_eq!(today_wg.refusals, 1);
        assert!((today_wg.cost_usd - 0.02).abs() < 1e-9);

        let yesterday_wg = series
            .iter()
            .find(|s| s.feature == "worldgen_room" && s.day == iso_date(today - 1))
            .expect("yesterday's worldgen_room bucket");
        assert_eq!(yesterday_wg.calls, 1);

        let today_npc = series
            .iter()
            .find(|s| s.feature == "npc_dialogue_major")
            .expect("today's npc bucket");
        assert_eq!(today_npc.budget_trips, 1);
        assert_eq!(today_npc.provider, "anthropic");
    }
}

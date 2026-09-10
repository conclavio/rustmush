//! Explicit, deterministic comparisons of sanitized experiment analyses. Missing
//! evidence is inconclusive; thresholds never imply statistical significance.
use std::{cmp::Ordering, collections::BTreeSet, fmt::Write, path::Path};

use serde::{Deserialize, Serialize};

use crate::{
    Result, analysis,
    experiment::{load_json, revision},
};

const MAX_POLICY_BYTES: usize = 128_000;
const MAX_RUBRICS: usize = 256;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub version: u32,
    pub checks: Vec<Check>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub id: String,
    pub metric: Metric,
    #[serde(default)]
    pub baseline_variant: Option<String>,
    #[serde(default)]
    pub candidate_variant: Option<String>,
    #[serde(default)]
    pub probe_id: Option<String>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default = "one")]
    pub min_samples: u64,
    #[serde(default)]
    pub minimum: Option<String>,
    #[serde(default)]
    pub maximum: Option<String>,
    #[serde(default)]
    pub max_drop: Option<String>,
    #[serde(default)]
    pub max_increase: Option<String>,
}

fn one() -> u64 {
    1
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    RunPassRate,
    ProbePassRate,
    ProbeValidResponseRate,
    ElapsedMsMean,
    ModelRequestsMean,
    ProviderCostMean,
}

impl Metric {
    fn probe(self) -> bool {
        matches!(self, Self::ProbePassRate | Self::ProbeValidResponseRate)
    }
}

impl Policy {
    pub fn load(path: &Path) -> Result<Self> {
        let policy: Self = load_json(path, MAX_POLICY_BYTES)?;
        policy.validate()?;
        Ok(policy)
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != 1 || self.checks.is_empty() || self.checks.len() > 64 {
            return Err("regression policy needs version 1 and 1..64 checks".into());
        }
        let mut ids = BTreeSet::new();
        for check in &self.checks {
            if !crate::organization::identifier(&check.id) || !ids.insert(&check.id) {
                return Err("regression check IDs must be unique bounded identifiers".into());
            }
            if !(1..=1_000_000).contains(&check.min_samples) {
                return Err("regression min_samples must be in 1..1000000".into());
            }
            for hash in [&check.baseline_variant, &check.candidate_variant]
                .into_iter()
                .flatten()
            {
                if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("regression variant selectors must be SHA256 strings".into());
                }
            }
            if let Some(id) = &check.probe_id
                && (!check.metric.probe() || !crate::organization::identifier(id))
            {
                return Err(
                    "regression probe_id needs a probe metric and bounded identifier".into(),
                );
            }
            match (&check.currency, check.metric) {
                (Some(currency), Metric::ProviderCostMean)
                    if currency.len() == 3 && currency.bytes().all(|c| c.is_ascii_uppercase()) => {}
                (None, Metric::ProviderCostMean) => {
                    return Err("provider_cost_mean requires an explicit currency".into());
                }
                (Some(_), _) => {
                    return Err(
                        "currency is valid only as three uppercase letters for provider_cost_mean"
                            .into(),
                    );
                }
                _ => {}
            }
            let thresholds = [
                &check.minimum,
                &check.maximum,
                &check.max_drop,
                &check.max_increase,
            ];
            if thresholds.iter().all(|n| n.is_none()) {
                return Err("regression checks require at least one threshold".into());
            }
            for value in thresholds.into_iter().flatten() {
                let whole = value.split('.').next().unwrap_or("");
                if whole.len() > 20 || Rational::decimal(value).is_none() {
                    return Err("regression thresholds need nonnegative decimal strings with at most 20 whole and 18 fractional digits".into());
                }
            }
            if let (Some(minimum), Some(maximum)) = (&check.minimum, &check.maximum) {
                let minimum = Rational::decimal(minimum).unwrap();
                let maximum = Rational::decimal(maximum).unwrap();
                if minimum.compare(maximum) == Some(Ordering::Greater) {
                    return Err("regression minimum exceeds maximum".into());
                }
            }
        }
        if serde_json::to_vec(self)?.len() > MAX_POLICY_BYTES {
            return Err("regression policy exceeds 128 KB".into());
        }
        Ok(())
    }
}

#[derive(Serialize)]
pub struct Report {
    pub version: u32,
    pub success: bool,
    pub policy: Policy,
    pub policy_revision: String,
    pub baseline_revision: String,
    pub candidate_revision: String,
    pub checks: Vec<CheckResult>,
    pub limitations: Vec<&'static str>,
}

#[derive(Serialize)]
pub struct CheckResult {
    pub id: String,
    pub metric: Metric,
    pub status: Status,
    pub reasons: Vec<&'static str>,
    pub baseline: Sample,
    pub candidate: Sample,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Fail,
    Inconclusive,
}

#[derive(Serialize)]
pub struct Scope {
    pub kind: &'static str,
    pub variant: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Simulation {
    Simulated,
    Nonsimulated,
    Mixed,
    Unknown,
}

/// `total`/`denominator` describe the observed samples, including partial
/// coverage. `value` is absent unless this side has complete usable coverage.
#[derive(Serialize)]
pub struct Sample {
    pub scope: Scope,
    pub run_count: usize,
    pub samples: u128,
    pub missing: u128,
    pub simulation: Simulation,
    pub total: Option<String>,
    pub denominator: String,
    pub value: Option<Fraction>,
    pub probe_rubrics: Vec<Rubric>,
    pub probe_rubrics_omitted: usize,
    pub reasons: Vec<&'static str>,
}

#[derive(Serialize)]
pub struct Fraction {
    pub numerator: String,
    pub denominator: String,
}

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Rubric {
    pub probe_id: String,
    pub definition_revision: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Rational {
    numerator: u128,
    denominator: u128,
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Rational {
    const ZERO: Self = Self {
        numerator: 0,
        denominator: 1,
    };

    fn new(numerator: u128, denominator: u128) -> Option<Self> {
        if denominator == 0 {
            return None;
        }
        let divisor = gcd(numerator, denominator);
        Some(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }

    fn integer(value: u128) -> Self {
        Self {
            numerator: value,
            denominator: 1,
        }
    }

    fn decimal(text: &str) -> Option<Self> {
        if text.is_empty() || text.len() > 59 {
            return None;
        }
        let (whole, fraction) = match text.split_once('.') {
            Some((whole, fraction)) if !fraction.is_empty() => (whole, fraction),
            Some(_) => return None,
            None => (text, ""),
        };
        if whole.is_empty()
            || fraction.len() > 18
            || !whole
                .bytes()
                .chain(fraction.bytes())
                .all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let numerator = format!("{whole}{fraction}").parse::<u128>().ok()?;
        Self::new(numerator, 10_u128.checked_pow(fraction.len() as u32)?)
    }

    fn add(self, other: Self) -> Option<Self> {
        let divisor = gcd(self.denominator, other.denominator);
        let left_scale = other.denominator / divisor;
        let right_scale = self.denominator / divisor;
        let numerator = self
            .numerator
            .checked_mul(left_scale)?
            .checked_add(other.numerator.checked_mul(right_scale)?)?;
        Self::new(numerator, self.denominator.checked_mul(left_scale)?)
    }

    fn subtract(self, other: Self) -> Option<Self> {
        let divisor = gcd(self.denominator, other.denominator);
        let left_scale = other.denominator / divisor;
        let right_scale = self.denominator / divisor;
        let numerator = self
            .numerator
            .checked_mul(left_scale)?
            .checked_sub(other.numerator.checked_mul(right_scale)?)?;
        Self::new(numerator, self.denominator.checked_mul(left_scale)?)
    }

    fn divide(self, denominator: u128) -> Option<Self> {
        if denominator == 0 {
            return None;
        }
        let divisor = gcd(self.numerator, denominator);
        Self::new(
            self.numerator / divisor,
            self.denominator.checked_mul(denominator / divisor)?,
        )
    }

    fn compare(self, other: Self) -> Option<Ordering> {
        let divisor = gcd(self.denominator, other.denominator);
        let left = self.numerator.checked_mul(other.denominator / divisor)?;
        let right = other.numerator.checked_mul(self.denominator / divisor)?;
        Some(left.cmp(&right))
    }

    fn fraction(self) -> Fraction {
        Fraction {
            numerator: self.numerator.to_string(),
            denominator: self.denominator.to_string(),
        }
    }

    // Only sums of input decimals are formatted as decimals. Calculated means
    // remain fractions and therefore never need rounding or truncation.
    fn decimal_string(self) -> Option<String> {
        let mut text = (self.numerator / self.denominator).to_string();
        let mut remainder = self.numerator % self.denominator;
        if remainder != 0 {
            text.push('.');
            for _ in 0..18 {
                let scaled = remainder.checked_mul(10)?;
                text.push(char::from(
                    b'0' + u8::try_from(scaled / self.denominator).ok()?,
                ));
                remainder = scaled % self.denominator;
                if remainder == 0 {
                    break;
                }
            }
        }
        (remainder == 0).then_some(text)
    }
}

struct Collected {
    sample: Sample,
    value: Option<Rational>,
}

fn reason(reasons: &mut Vec<&'static str>, item: &'static str) {
    if !reasons.contains(&item) {
        reasons.push(item);
    }
}

fn collect(check: &Check, report: &analysis::Report, variant: Option<&str>) -> Collected {
    let runs: Vec<_> = report
        .runs
        .iter()
        .filter(|run| variant.is_none_or(|selector| run.variant_id.eq_ignore_ascii_case(selector)))
        .collect();
    let mut sample = Sample {
        scope: Scope {
            kind: if variant.is_some() {
                "variant"
            } else {
                "aggregate"
            },
            variant: variant.map(str::to_owned),
        },
        run_count: runs.len(),
        samples: 0,
        missing: 0,
        simulation: Simulation::Unknown,
        total: None,
        denominator: "0".into(),
        value: None,
        probe_rubrics: vec![],
        probe_rubrics_omitted: 0,
        reasons: vec![],
    };
    if runs.is_empty() {
        reason(&mut sample.reasons, "scope_empty");
    }
    let mut simulated = false;
    let mut nonsimulated = false;
    let mut unknown = false;
    let mut rubrics = BTreeSet::new();
    let mut total = Some(Rational::ZERO);
    for run in runs {
        match run.interactive_debug {
            Some(false) => {}
            Some(true) => reason(&mut sample.reasons, "interactive_debug_run"),
            None => reason(&mut sample.reasons, "debug_coverage_missing"),
        }
        if run.actors.is_empty() || run.missing_metrics.iter().any(|metric| metric == "cast") {
            unknown = true;
            reason(&mut sample.reasons, "roster_unknown");
        }
        for actor in &run.actors {
            match actor.simulated_model {
                Some(true) => simulated = true,
                Some(false) => nonsimulated = true,
                None => unknown = true,
            }
        }
        let mut include = |value: Option<Rational>| {
            if let Some(value) = value {
                sample.samples += 1;
                total = total.and_then(|total| total.add(value));
            } else {
                sample.missing += 1;
                reason(&mut sample.reasons, "metric_missing");
            }
        };
        match check.metric {
            Metric::RunPassRate => include(run.success.map(|n| Rational::integer(u128::from(n)))),
            Metric::ElapsedMsMean => include(run.elapsed_ms.map(Rational::integer)),
            Metric::ModelRequestsMean => {
                for actor in &run.actors {
                    include(actor.model_requests.map(Rational::integer));
                }
            }
            Metric::ProviderCostMean => {
                for actor in &run.actors {
                    let cost = actor
                        .session_costs
                        .as_ref()
                        .and_then(|costs| {
                            costs.complete_provider_cost(check.currency.as_deref().unwrap())
                        })
                        .filter(|(requests, _)| actor.model_requests == Some(*requests as u128))
                        .and_then(|(_, amount)| Rational::decimal(&amount.to_string()));
                    include(cost);
                }
            }
            Metric::ProbePassRate | Metric::ProbeValidResponseRate => {
                match run.probes_omitted {
                    Some(0) => {}
                    Some(_) => reason(&mut sample.reasons, "probe_omitted"),
                    None => reason(&mut sample.reasons, "probe_coverage_missing"),
                }
                if let Some(probes) = &run.probes {
                    let mut sequences = BTreeSet::new();
                    for probe in probes {
                        match probe.seq {
                            Some(seq) if seq > 0 => {
                                if !sequences.insert((&probe.actor, seq)) {
                                    reason(&mut sample.reasons, "probe_sequence_duplicate");
                                }
                            }
                            _ => reason(&mut sample.reasons, "probe_sequence_missing"),
                        }
                        let Some(grade) = &probe.grade else {
                            sample.missing += 1;
                            reason(&mut sample.reasons, "probe_ungraded");
                            continue;
                        };
                        if check
                            .probe_id
                            .as_ref()
                            .is_some_and(|id| grade.probe_id != *id)
                        {
                            continue;
                        }
                        rubrics.insert(Rubric {
                            probe_id: grade.probe_id.clone(),
                            definition_revision: grade.definition_revision.to_ascii_lowercase(),
                        });
                        let passed = if check.metric == Metric::ProbePassRate {
                            grade.success
                        } else {
                            grade.response_valid
                        };
                        sample.samples += 1;
                        total = total
                            .and_then(|total| total.add(Rational::integer(u128::from(passed))));
                    }
                } else {
                    sample.missing += 1;
                    reason(&mut sample.reasons, "probe_coverage_missing");
                }
            }
        }
    }
    sample.simulation = if unknown || (!simulated && !nonsimulated) {
        Simulation::Unknown
    } else if simulated && nonsimulated {
        Simulation::Mixed
    } else if simulated {
        Simulation::Simulated
    } else {
        Simulation::Nonsimulated
    };
    match sample.simulation {
        Simulation::Unknown => reason(&mut sample.reasons, "simulation_unknown"),
        Simulation::Mixed => reason(&mut sample.reasons, "simulation_mixed"),
        _ => {}
    }
    if sample.samples < u128::from(check.min_samples) {
        reason(&mut sample.reasons, "insufficient_samples");
    }
    if rubrics.len() > MAX_RUBRICS {
        sample.probe_rubrics_omitted = rubrics.len() - MAX_RUBRICS;
        reason(&mut sample.reasons, "rubric_capacity");
    }
    sample.probe_rubrics = rubrics.into_iter().take(MAX_RUBRICS).collect();
    sample.denominator = sample.samples.to_string();
    sample.total = total.and_then(Rational::decimal_string);
    let value = total.and_then(|n| n.divide(sample.samples));
    if total.is_none() || sample.total.is_none() || (sample.samples != 0 && value.is_none()) {
        reason(&mut sample.reasons, "arithmetic_overflow");
    }
    let value = if sample.reasons.is_empty() {
        value
    } else {
        None
    };
    sample.value = value.map(Rational::fraction);
    Collected { sample, value }
}

pub fn evaluate(
    policy: &Policy,
    baseline: &analysis::Report,
    candidate: &analysis::Report,
) -> Result<Report> {
    policy.validate()?;
    if baseline.version != 1 || candidate.version != 1 {
        return Err("regression comparison requires version-1 analyses".into());
    }
    let mut checks = Vec::with_capacity(policy.checks.len());
    for check in &policy.checks {
        let baseline = collect(check, baseline, check.baseline_variant.as_deref());
        let candidate = collect(check, candidate, check.candidate_variant.as_deref());
        let mut reasons = Vec::new();
        if !baseline.sample.reasons.is_empty() {
            reason(&mut reasons, "baseline_incomplete");
        }
        if !candidate.sample.reasons.is_empty() {
            reason(&mut reasons, "candidate_incomplete");
        }
        if baseline.sample.simulation != candidate.sample.simulation {
            reason(&mut reasons, "simulation_mismatch");
        }
        if check.metric.probe() && baseline.sample.probe_rubrics != candidate.sample.probe_rubrics {
            reason(&mut reasons, "rubric_mismatch");
        }
        let mut status = Status::Inconclusive;
        if reasons.is_empty() {
            if let (Some(baseline), Some(candidate)) = (baseline.value, candidate.value) {
                let mut overflow = false;
                let mut test =
                    |value: Option<Rational>, threshold: &Option<String>, minimum: bool, failed| {
                        if let Some(threshold) = threshold {
                            let outcome = value.and_then(|value| {
                                value.compare(Rational::decimal(threshold).unwrap())
                            });
                            match outcome {
                                None => overflow = true,
                                Some(order)
                                    if (minimum && order == Ordering::Less)
                                        || (!minimum && order == Ordering::Greater) =>
                                {
                                    reason(&mut reasons, failed)
                                }
                                _ => {}
                            }
                        }
                    };
                test(Some(candidate), &check.minimum, true, "below_minimum");
                test(Some(candidate), &check.maximum, false, "above_maximum");
                let direction = baseline.compare(candidate);
                let drop = match direction {
                    Some(Ordering::Greater) => baseline.subtract(candidate),
                    Some(_) => Some(Rational::ZERO),
                    None => None,
                };
                let increase = match direction {
                    Some(Ordering::Less) => candidate.subtract(baseline),
                    Some(_) => Some(Rational::ZERO),
                    None => None,
                };
                test(drop, &check.max_drop, false, "drop_exceeded");
                test(increase, &check.max_increase, false, "increase_exceeded");
                if overflow {
                    reason(&mut reasons, "arithmetic_overflow");
                } else {
                    status = if reasons.is_empty() {
                        Status::Pass
                    } else {
                        Status::Fail
                    };
                }
            } else {
                reason(&mut reasons, "insufficient_samples");
            }
        }
        checks.push(CheckResult {
            id: check.id.clone(),
            metric: check.metric,
            status,
            reasons,
            baseline: baseline.sample,
            candidate: candidate.sample,
        });
    }
    Ok(Report {
        version: 1,
        success: checks.iter().all(|check| check.status == Status::Pass),
        policy: policy.clone(),
        policy_revision: revision(&serde_json::to_vec(policy)?),
        baseline_revision: revision(&serde_json::to_vec(baseline)?),
        candidate_revision: revision(&serde_json::to_vec(candidate)?),
        checks,
        limitations: vec![
            "Thresholds are deterministic comparisons of recorded samples, not statistical significance tests or proof of equivalent scenarios.",
            "Aggregate scopes include every recorded variant. Means use runs for elapsed time and actor sessions for requests and provider charges; probe rates use recorded graded evaluations.",
            "Simulation labels describe recorded local response-tape use. Nonsimulated does not verify a live provider or production conditions.",
            "Interactive phase-debugged runs and legacy runs without an explicit debug marker are inconclusive; rerun unattended for acceptance checks.",
            "Revisions identify the sanitized compared content, not authenticity. No referenced configuration, prompt, memory, answer or trace file is opened.",
        ],
    })
}

fn xml(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(256)
        .fold(String::new(), |mut out, c| {
            out.push_str(match c {
                '&' => "&amp;",
                '<' => "&lt;",
                '>' => "&gt;",
                '"' => "&quot;",
                '\'' => "&apos;",
                _ => {
                    out.push(c);
                    return out;
                }
            });
            out
        })
}

pub fn junit(report: &Report) -> String {
    let failed = report
        .checks
        .iter()
        .filter(|c| c.status == Status::Fail)
        .count();
    let errors = report
        .checks
        .iter()
        .filter(|c| c.status == Status::Inconclusive)
        .count();
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuite name=\"bot-regression\" tests=\"{}\" failures=\"{failed}\" errors=\"{errors}\">\n",
        report.checks.len()
    );
    for check in &report.checks {
        let _ = writeln!(
            out,
            "  <testcase classname=\"bot-regression\" name=\"{}\">",
            xml(&check.id)
        );
        let tag = match check.status {
            Status::Pass => None,
            Status::Fail => Some("failure"),
            Status::Inconclusive => Some("error"),
        };
        if let Some(tag) = tag {
            let _ = writeln!(
                out,
                "    <{tag} message=\"{}\" />",
                xml(&check.reasons.join(","))
            );
        }
        out.push_str("  </testcase>\n");
    }
    out.push_str("</testsuite>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn policy(metric: &str) -> Policy {
        let mut check = json!({"id":"comparison","metric":metric,"minimum":"0"});
        if metric == "provider_cost_mean" {
            check["currency"] = "USD".into();
        }
        serde_json::from_value(json!({"version":1,"checks":[check]})).unwrap()
    }

    fn run(index: u64, passed: bool, amount: &str) -> Value {
        let mut costs = serde_json::to_value(crate::costs::Breakdown::default()).unwrap();
        costs["totals"]["requests"] = 2.into();
        costs["totals"]["responses"] = 2.into();
        costs["totals"]["known_cost_requests"] = 2.into();
        costs["totals"]["provider_cost_by_currency"] = json!({"USD":amount});
        json!({"index":index,"cast":[{"name":"scout","model":"model/example"}],
            "result":{"success":passed,"elapsed_ms":13,"debug":{"enabled":false,"held_ms":0},
                "actors":{"scout":{"report":{"simulated_model":false,
                    "state":{"model_calls":1,"commands_sent":3},"costs":{"session":costs}}}},
                "probes":[],"probes_omitted":0}})
    }

    fn analysis(runs: Vec<Value>) -> analysis::Report {
        analysis::analyze(&json!({"version":1,"runs":runs})).unwrap()
    }

    fn probe(passed: bool, valid: bool, rubric: &str) -> Value {
        json!({"actor":"scout","seq":if passed { 1 } else { 2 },"grade":{"version":1,"probe_id":"recall",
            "definition_revision":revision(rubric.as_bytes()),"request_revision":revision(b"capture"),
            "success":passed,"response_valid":valid,"checks":[{"id":"answer","passed":passed}]}})
    }

    fn status(
        policy: &Policy,
        baseline: &analysis::Report,
        candidate: &analysis::Report,
    ) -> Status {
        evaluate(policy, baseline, candidate).unwrap().checks[0].status
    }

    #[test]
    fn policy_rejects_ambiguous_and_unbounded_thresholds_and_selectors() {
        let initial = serde_json::to_value(policy("run_pass_rate")).unwrap();
        for (key, invalid) in [
            ("metric", json!("tokens")),
            ("id", json!("with spaces")),
            ("min_samples", json!(0)),
            ("min_samples", json!(1_000_001)),
            ("minimum", json!(0.5)),
            ("minimum", json!("-0.1")),
            ("minimum", json!("1e-3")),
            ("minimum", json!("1.")),
            ("minimum", json!(".1")),
            ("minimum", json!("0.0000000000000000001")),
            ("minimum", json!("100000000000000000000")),
            ("minimum", Value::Null),
            ("probe_id", json!("recall")),
            ("currency", json!("USD")),
            ("baseline_variant", json!("missing")),
            ("maximum", json!("-1")),
            ("unknown", json!(true)),
        ] {
            let mut value = initial.clone();
            value["checks"][0][key] = invalid;
            let parsed = serde_json::from_value::<Policy>(value);
            assert!(
                parsed.is_err() || parsed.unwrap().validate().is_err(),
                "{key}"
            );
        }
        let mut duplicate = policy("run_pass_rate");
        duplicate.checks.push(duplicate.checks[0].clone());
        assert!(duplicate.validate().is_err());
        let mut inverted = policy("run_pass_rate");
        inverted.checks[0].minimum = Some("1".into());
        inverted.checks[0].maximum = Some("0.9".into());
        assert!(inverted.validate().is_err());
        let mut costs = policy("provider_cost_mean");
        for currency in [None, Some("usd".into()), Some("USDD".into())] {
            costs.checks[0].currency = currency;
            assert!(costs.validate().is_err());
        }
    }

    #[test]
    fn exact_rates_use_both_cohorts_and_do_not_inherit_run_failure() {
        let baseline = analysis(vec![
            run(1, true, "0"),
            run(2, false, "0"),
            run(3, true, "0"),
        ]);
        let candidate = analysis(vec![run(1, true, "0"), run(2, false, "0")]);
        let mut policy = policy("run_pass_rate");
        policy.checks[0].min_samples = 2;
        policy.checks[0].minimum = Some("0.5".into());
        policy.checks[0].max_drop = Some("0.166666666666666666".into());
        let report = evaluate(&policy, &baseline, &candidate).unwrap();
        assert_eq!(report.checks[0].status, Status::Fail);
        assert_eq!(report.checks[0].reasons, ["drop_exceeded"]);
        assert_eq!(report.checks[0].baseline.samples, 3);
        assert_eq!(report.checks[0].baseline.total.as_deref(), Some("2"));
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(
            value["checks"][0]["baseline"]["value"],
            json!({"numerator":"2","denominator":"3"})
        );
        policy.checks[0].max_drop = Some("0.166666666666666667".into());
        assert_eq!(status(&policy, &baseline, &candidate), Status::Pass);
        policy.checks[0].min_samples = 3;
        assert_eq!(status(&policy, &baseline, &candidate), Status::Inconclusive);
        policy.checks[0].min_samples = 1;
        policy.checks[0].minimum = Some("0".into());
        policy.checks[0].max_drop = None;
        let failed = analysis(vec![run(1, false, "0")]);
        assert!(!failed.success);
        assert!(evaluate(&policy, &failed, &failed).unwrap().success);
    }

    #[test]
    fn decimal_cost_comparisons_preserve_the_eighteenth_place() {
        let baseline = analysis(vec![run(1, true, "0.100000000000000001")]);
        let candidate = analysis(vec![run(1, true, "0.100000000000000002")]);
        let mut policy = policy("provider_cost_mean");
        policy.checks[0].max_increase = Some("0.000000000000000001".into());
        assert_eq!(status(&policy, &baseline, &candidate), Status::Pass);
        policy.checks[0].max_increase = Some("0".into());
        assert_eq!(status(&policy, &baseline, &candidate), Status::Fail);
        let report = evaluate(&policy, &baseline, &candidate).unwrap();
        assert_eq!(
            report.checks[0].candidate.total.as_deref(),
            Some("0.100000000000000002")
        );
        assert_eq!(report.checks[0].candidate.denominator, "1");
        assert_eq!(
            report.checks[0].candidate.value.as_ref().unwrap().numerator,
            "50000000000000001"
        );
        assert_eq!(
            report.checks[0]
                .candidate
                .value
                .as_ref()
                .unwrap()
                .denominator,
            "500000000000000000"
        );
    }

    #[test]
    fn means_count_actor_sessions_and_ledger_requests_across_goal_resets() {
        let report = analysis(vec![run(1, true, "0.1"), run(2, true, "0.2")]);
        let model = evaluate(&policy("model_requests_mean"), &report, &report).unwrap();
        assert_eq!(model.checks[0].candidate.samples, 2);
        assert_eq!(model.checks[0].candidate.total.as_deref(), Some("4"));
        assert_eq!(
            model.checks[0].candidate.value.as_ref().unwrap().numerator,
            "2"
        );
        let costs = evaluate(&policy("provider_cost_mean"), &report, &report).unwrap();
        assert_eq!(costs.checks[0].candidate.total.as_deref(), Some("0.3"));
        assert_eq!(costs.checks[0].candidate.denominator, "2");
        assert_eq!(
            costs.checks[0].candidate.value.as_ref().unwrap().numerator,
            "3"
        );
        assert_eq!(
            costs.checks[0]
                .candidate
                .value
                .as_ref()
                .unwrap()
                .denominator,
            "20"
        );
        let elapsed = evaluate(&policy("elapsed_ms_mean"), &report, &report).unwrap();
        assert_eq!(elapsed.checks[0].candidate.total.as_deref(), Some("26"));
        assert_eq!(elapsed.checks[0].candidate.samples, 2);
    }

    #[test]
    fn missing_currency_or_receipts_never_become_a_zero_cost() {
        let baseline = analysis(vec![run(1, true, "0.1")]);
        let policy = policy("provider_cost_mean");
        for (pointer, value) in [
            ("/result/actors/scout/report/costs/session", Value::Null),
            (
                "/result/actors/scout/report/costs/session/totals/known_cost_requests",
                json!(1),
            ),
            (
                "/result/actors/scout/report/costs/session/totals/unknown_cost_requests",
                json!(1),
            ),
            (
                "/result/actors/scout/report/costs/session/totals/provider_cost_by_currency",
                json!({}),
            ),
            (
                "/result/actors/scout/report/costs/session/totals/provider_cost_by_currency",
                json!({"EUR":"0.1"}),
            ),
            (
                "/result/actors/scout/report/costs/session/totals/provider_cost_by_currency",
                json!({"USD":"0.1","EUR":"0"}),
            ),
            (
                "/result/actors/scout/report/costs/session/totals/pending",
                json!(1),
            ),
        ] {
            let mut entry = run(1, true, "0.1");
            *entry.pointer_mut(pointer).unwrap() = value;
            let candidate = analysis(vec![entry]);
            let compared = evaluate(&policy, &baseline, &candidate).unwrap();
            assert_eq!(compared.checks[0].status, Status::Inconclusive, "{pointer}");
            assert!(compared.checks[0].candidate.value.is_none());
        }
        let mut zero = run(1, true, "0");
        zero["result"]["actors"]["scout"]["report"]["costs"]["session"] =
            serde_json::to_value(crate::costs::Breakdown::default()).unwrap();
        let zero = analysis(vec![zero]);
        assert_eq!(status(&policy, &zero, &zero), Status::Pass);
    }

    #[test]
    fn probes_require_complete_matching_rubrics_including_under_id_filters() {
        let mut entry = run(1, false, "0");
        entry["result"]["probes"] = json!([probe(true, true, "a"), probe(false, false, "a")]);
        let baseline = analysis(vec![entry.clone()]);
        for metric in ["probe_pass_rate", "probe_valid_response_rate"] {
            let mut policy = policy(metric);
            policy.checks[0].probe_id = Some("recall".into());
            let report = evaluate(&policy, &baseline, &baseline).unwrap();
            assert!(report.success);
            assert_eq!(report.checks[0].baseline.samples, 2);
            assert_eq!(report.checks[0].baseline.total.as_deref(), Some("1"));
            for (pointer, value) in [
                ("/result/probes", Value::Null),
                ("/result/probes", json!([])),
                ("/result/probes/1/grade", Value::Null),
                ("/result/probes/1/seq", Value::Null),
                ("/result/probes/1/seq", json!(0)),
                ("/result/probes/1/seq", json!(1)),
                ("/result/probes_omitted", Value::Null),
                ("/result/probes_omitted", json!(1)),
            ] {
                let mut candidate = entry.clone();
                *candidate.pointer_mut(pointer).unwrap() = value;
                let candidate = analysis(vec![candidate]);
                assert_eq!(
                    status(&policy, &baseline, &candidate),
                    Status::Inconclusive,
                    "{pointer}"
                );
            }
            let mut changed = entry.clone();
            changed["result"]["probes"] = json!([probe(true, true, "b")]);
            let changed = analysis(vec![changed]);
            let report = evaluate(&policy, &baseline, &changed).unwrap();
            assert_eq!(report.checks[0].status, Status::Inconclusive);
            assert!(report.checks[0].reasons.contains(&"rubric_mismatch"));
        }
    }

    #[test]
    fn interactive_and_unclassified_runs_cannot_pass_regression_gates() {
        let baseline = analysis(vec![run(1, true, "0")]);
        for marker in [json!({"enabled":true,"held_ms":0}), Value::Null] {
            let mut candidate = run(1, true, "0");
            candidate["result"]["debug"] = marker;
            let candidate = analysis(vec![candidate]);
            for metric in [
                "run_pass_rate",
                "model_requests_mean",
                "provider_cost_mean",
                "elapsed_ms_mean",
            ] {
                let report = evaluate(&policy(metric), &baseline, &candidate).unwrap();
                assert_eq!(report.checks[0].status, Status::Inconclusive);
                assert!(report.checks[0].candidate.value.is_none());
            }
        }
    }

    #[test]
    fn matrix_scopes_are_explicit_and_simulation_must_be_comparable() {
        let plain = run(1, true, "0");
        let mut tape = run(2, true, "0");
        tape["cast"][0]["model"] = "model/tape".into();
        tape["result"]["actors"]["scout"]["report"]["simulated_model"] = true.into();
        let mixed = analysis(vec![plain.clone(), tape.clone()]);
        let mut policy = policy("run_pass_rate");
        assert_eq!(status(&policy, &mixed, &mixed), Status::Inconclusive);
        policy.checks[0].baseline_variant = Some(mixed.runs[1].variant_id.clone());
        policy.checks[0].candidate_variant = policy.checks[0].baseline_variant.clone();
        let scoped = evaluate(&policy, &mixed, &mixed).unwrap();
        assert!(scoped.success);
        assert_eq!(scoped.checks[0].baseline.simulation, Simulation::Simulated);
        assert_eq!(scoped.checks[0].baseline.scope.kind, "variant");
        assert_eq!(scoped.checks[0].baseline.run_count, 1);
        policy.checks[0].candidate_variant = Some(mixed.runs[0].variant_id.clone());
        let compared = evaluate(&policy, &mixed, &mixed).unwrap();
        assert!(compared.checks[0].reasons.contains(&"simulation_mismatch"));
        policy.checks[0].candidate_variant = Some("f".repeat(64));
        let compared = evaluate(&policy, &mixed, &mixed).unwrap();
        assert!(
            compared.checks[0]
                .candidate
                .reasons
                .contains(&"scope_empty")
        );
        let mut missing = plain;
        missing["result"]["actors"]["scout"]["report"]["simulated_model"] = Value::Null;
        let unknown = analysis(vec![missing]);
        policy.checks[0].baseline_variant = None;
        policy.checks[0].candidate_variant = None;
        assert_eq!(status(&policy, &unknown, &unknown), Status::Inconclusive);
    }

    #[test]
    fn checked_capacity_and_missing_measurements_are_inconclusive() {
        let mut baseline = analysis(vec![run(1, true, "0"), run(2, true, "0")]);
        baseline.runs[0].elapsed_ms = Some(u128::MAX);
        baseline.runs[1].elapsed_ms = Some(1);
        let policy = policy("elapsed_ms_mean");
        let report = evaluate(&policy, &baseline, &baseline).unwrap();
        assert_eq!(report.checks[0].status, Status::Inconclusive);
        assert!(
            report.checks[0]
                .baseline
                .reasons
                .contains(&"arithmetic_overflow")
        );
        assert!(report.checks[0].baseline.total.is_none());
        baseline.runs[0].elapsed_ms = Some(0);
        baseline.runs[1].elapsed_ms = None;
        let report = evaluate(&policy, &baseline, &baseline).unwrap();
        assert_eq!(report.checks[0].status, Status::Inconclusive);
        assert_eq!(report.checks[0].baseline.missing, 1);
        assert_eq!(report.checks[0].baseline.total.as_deref(), Some("0"));
        assert!(report.checks[0].baseline.value.is_none());
        assert_eq!(
            Rational::decimal("0.1")
                .unwrap()
                .add(Rational::decimal("0.2").unwrap())
                .unwrap()
                .decimal_string()
                .as_deref(),
            Some("0.3")
        );
    }

    #[test]
    fn fingerprints_and_ci_outcomes_are_separate_from_source_success() {
        let baseline = analysis(vec![run(1, false, "0")]);
        let mut candidate = analysis(vec![run(1, false, "0")]);
        let mut policy = policy("run_pass_rate");
        policy.checks[0].minimum = Some("1".into());
        let report = evaluate(&policy, &baseline, &candidate).unwrap();
        assert_eq!(report.baseline_revision, report.candidate_revision);
        assert_eq!(report.policy_revision.len(), 64);
        assert!(junit(&report).contains("failures=\"1\" errors=\"0\""));
        assert!(junit(&report).contains("<failure "));
        candidate.runs[0].success = None;
        let mut unknown = evaluate(&policy, &baseline, &candidate).unwrap();
        assert_ne!(unknown.baseline_revision, unknown.candidate_revision);
        assert!(junit(&unknown).contains("failures=\"0\" errors=\"1\""));
        assert!(junit(&unknown).contains("<error "));
        unknown.checks[0].id = "a<&\"'\u{1}>".into();
        let xml = junit(&unknown);
        assert!(xml.contains("a&lt;&amp;&quot;&apos;&gt;"));
        assert!(!xml.contains('\u{1}'));
        assert!(xml.len() < 1000);
    }
}

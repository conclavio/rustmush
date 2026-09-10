//! Offline, bounded experiment summaries. This module reads only summary.json;
//! it never follows artifact paths or opens configurations, traces or memories.
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
    path::Path,
};

use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    Result,
    costs::Breakdown,
    experiment::{load_json, revision},
};

const MAX_SUMMARY_BYTES: usize = 64_000_000;
const MAX_RUNS: usize = 4096;
const MAX_FAILURES: usize = 128;
const MAX_PROBES: usize = 4096;

/// Only the rubric identity, sealed request identity and boolean outcomes are
/// exported. Freeform grading details and answers stay in the original artifact.
#[derive(Serialize)]
pub struct ProbeCheck {
    pub id: String,
    pub passed: bool,
}

#[derive(Serialize)]
pub struct ProbeGrade {
    pub probe_id: String,
    pub definition_revision: String,
    pub request_revision: String,
    pub success: bool,
    pub response_valid: bool,
    pub checks: Vec<ProbeCheck>,
}

#[derive(Serialize)]
pub struct ProbeEvaluation {
    pub actor: String,
    pub seq: Option<u64>,
    pub grade: Option<ProbeGrade>,
}

#[derive(Default, Serialize)]
pub struct ProbeCounts {
    pub evaluations: usize,
    pub graded: usize,
    pub ungraded: usize,
    pub passed: usize,
    pub failed: usize,
    pub valid_responses: usize,
    pub invalid_responses: usize,
    pub checks_passed: usize,
    pub checks_failed: usize,
}
impl ProbeCounts {
    fn include(&mut self, grade: Option<&ProbeGrade>) {
        self.evaluations += 1;
        if let Some(grade) = grade {
            self.graded += 1;
            self.passed += usize::from(grade.success);
            self.failed += usize::from(!grade.success);
            self.valid_responses += usize::from(grade.response_valid);
            self.invalid_responses += usize::from(!grade.response_valid);
            for check in &grade.checks {
                self.checks_passed += usize::from(check.passed);
                self.checks_failed += usize::from(!check.passed);
            }
        } else {
            self.ungraded += 1;
        }
    }
}

#[derive(Serialize)]
pub struct ProbeSummary {
    pub probe_id: String,
    pub definition_revision: String,
    pub totals: ProbeCounts,
    pub by_actor: BTreeMap<String, ProbeCounts>,
}

#[derive(Default, Serialize)]
pub struct ProbeAggregate {
    pub runs_reported: usize,
    pub runs_missing: usize,
    pub omitted_evaluations: Metric,
    pub totals: ProbeCounts,
    pub by_actor: BTreeMap<String, ProbeCounts>,
    pub by_probe: Vec<ProbeSummary>,
}
impl ProbeAggregate {
    fn include(&mut self, probes: Option<&[ProbeEvaluation]>, omitted: Option<u128>) -> Result<()> {
        self.omitted_evaluations.include(omitted)?;
        let Some(probes) = probes else {
            self.runs_missing += 1;
            return Ok(());
        };
        self.runs_reported += 1;
        for probe in probes {
            let grade = probe.grade.as_ref();
            self.totals.include(grade);
            self.by_actor
                .entry(probe.actor.clone())
                .or_default()
                .include(grade);
            if let Some(grade) = grade {
                let index = match self.by_probe.binary_search_by(|item| {
                    (&item.probe_id, &item.definition_revision)
                        .cmp(&(&grade.probe_id, &grade.definition_revision))
                }) {
                    Ok(index) => index,
                    Err(index) => {
                        self.by_probe.insert(
                            index,
                            ProbeSummary {
                                probe_id: grade.probe_id.clone(),
                                definition_revision: grade.definition_revision.clone(),
                                totals: ProbeCounts::default(),
                                by_actor: BTreeMap::new(),
                            },
                        );
                        index
                    }
                };
                let group = &mut self.by_probe[index];
                group.totals.include(Some(grade));
                group
                    .by_actor
                    .entry(probe.actor.clone())
                    .or_default()
                    .include(Some(grade));
            }
        }
        Ok(())
    }
}

/// Counts include coverage. A missing value is never silently treated as zero.
#[derive(Default, Serialize)]
pub struct Metric {
    pub known_total: Option<u128>,
    pub minimum: Option<u128>,
    pub maximum: Option<u128>,
    pub reported: usize,
    pub missing: usize,
}
impl Metric {
    fn include(&mut self, value: Option<u128>) -> Result<()> {
        if let Some(value) = value {
            self.known_total = Some(
                self.known_total
                    .unwrap_or(0)
                    .checked_add(value)
                    .ok_or("analysis metric overflow")?,
            );
            self.minimum = Some(self.minimum.map_or(value, |n| n.min(value)));
            self.maximum = Some(self.maximum.map_or(value, |n| n.max(value)));
            self.reported += 1;
        } else {
            self.missing += 1;
        }
        Ok(())
    }
}

#[derive(Default, Serialize)]
pub struct Aggregate {
    pub runs: usize,
    pub passed: usize,
    pub failed: usize,
    pub unknown: usize,
    pub elapsed_ms: Metric,
    pub active_elapsed_ms: Metric,
    pub debug_held_ms: Metric,
    pub debugged_runs: usize,
    pub unattended_runs: usize,
    pub unknown_debug_runs: usize,
    /// Actor-session counters, not the current goal's resettable run budget.
    pub model_requests: Metric,
    pub commands_sent: Metric,
    /// Separately retained because /goal resets this runner counter.
    pub current_goal_model_calls: Metric,
    pub session_costs: Option<Breakdown>,
    pub cost_sessions_reported: usize,
    pub cost_sessions_missing: usize,
    pub simulated_actor_sessions: usize,
    pub nonsimulated_actor_sessions: usize,
    pub unknown_simulation_actor_sessions: usize,
    pub probes: ProbeAggregate,
}
impl Aggregate {
    fn include(&mut self, run: &Run) -> Result<()> {
        self.runs += 1;
        match run.success {
            Some(true) => self.passed += 1,
            Some(false) => self.failed += 1,
            None => self.unknown += 1,
        }
        self.elapsed_ms.include(run.elapsed_ms)?;
        self.active_elapsed_ms.include(run.active_elapsed_ms)?;
        self.debug_held_ms.include(run.debug_held_ms)?;
        match run.interactive_debug {
            Some(true) => self.debugged_runs += 1,
            Some(false) => self.unattended_runs += 1,
            None => self.unknown_debug_runs += 1,
        }
        self.probes
            .include(run.probes.as_deref(), run.probes_omitted)?;
        for actor in &run.actors {
            match actor.simulated_model {
                Some(true) => self.simulated_actor_sessions += 1,
                Some(false) => self.nonsimulated_actor_sessions += 1,
                None => self.unknown_simulation_actor_sessions += 1,
            }
            self.model_requests.include(actor.model_requests)?;
            self.commands_sent.include(actor.commands_sent)?;
            self.current_goal_model_calls
                .include(actor.current_goal_model_calls)?;
            if let Some(costs) = &actor.session_costs {
                self.session_costs.get_or_insert_default().merge(costs)?;
                self.cost_sessions_reported += 1;
            } else {
                self.cost_sessions_missing += 1;
            }
        }
        Ok(())
    }
}

#[derive(Serialize)]
pub struct ActorVariant {
    pub name: String,
    pub model: Option<String>,
    pub prompts: Option<String>,
    pub context: Option<String>,
    pub character: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<String>,
    /// Hashes the entire authored actor spec, including config and script
    /// selectors, without copying goals, roles or local controls into exports.
    pub specification_revision: String,
}

#[derive(Serialize)]
pub struct Variant {
    pub id: String,
    pub actors: Vec<ActorVariant>,
    pub run_indexes: Vec<u64>,
    pub metrics: Aggregate,
}

#[derive(Serialize)]
pub struct ActorRun {
    pub name: String,
    pub status: Option<String>,
    pub actual_model: Option<String>,
    pub model_profile: Option<String>,
    pub simulated_model: Option<bool>,
    pub commands_sent: Option<u128>,
    pub model_requests: Option<u128>,
    pub current_goal_model_calls: Option<u128>,
    pub session_costs: Option<Breakdown>,
}

/// Diagnostics intentionally omit matcher literals, observed text, control
/// contents, provider error bodies, prompts and member briefings.
#[derive(Serialize)]
pub struct Failure {
    pub scope: String,
    pub category: String,
    pub assertion_index: Option<usize>,
    pub actor: Option<String>,
    pub event_kind: Option<String>,
}

#[derive(Serialize)]
pub struct Run {
    pub index: u64,
    pub repetition: Option<u64>,
    pub variant_id: String,
    pub success: Option<bool>,
    pub elapsed_ms: Option<u128>,
    pub active_elapsed_ms: Option<u128>,
    pub interactive_debug: Option<bool>,
    pub debug_held_ms: Option<u128>,
    pub assertions_success: Option<bool>,
    pub actors: Vec<ActorRun>,
    /// None means the old/missing report did not record probe coverage; an
    /// explicit empty array means this run recorded no probe evaluations.
    pub probes: Option<Vec<ProbeEvaluation>>,
    pub probes_omitted: Option<u128>,
    pub failures: Vec<Failure>,
    pub omitted_failures: usize,
    pub missing_metrics: Vec<String>,
}

#[derive(Serialize)]
pub struct Report {
    pub version: u32,
    pub name: String,
    pub success: bool,
    pub metrics: Aggregate,
    pub variants: Vec<Variant>,
    pub runs: Vec<Run>,
    pub limitations: Vec<String>,
}

/// Accept either an artifact directory or its summary.json. No sibling or
/// referenced path is read, so moving an artifact directory does not break this.
pub fn load(path: &Path) -> Result<Report> {
    let path = if path.is_dir() {
        path.join("summary.json")
    } else {
        path.to_path_buf()
    };
    analyze(&load_json::<Value>(&path, MAX_SUMMARY_BYTES)?)
}

fn number(value: &Value) -> Option<u128> {
    value.as_number()?.to_string().parse().ok()
}

fn bounded(value: &str, max: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect()
}

fn label(value: &Value, max: usize) -> Option<String> {
    value.as_str().map(|s| bounded(s, max))
}

fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .map(|(k, v)| (k.clone(), canonical(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        _ => value.clone(),
    }
}

fn variant(cast: &Value, index: u64) -> Result<(String, Vec<ActorVariant>)> {
    let Some(cast) = cast.as_array() else {
        // Unknown casts must not be grouped as if they were equal experiments.
        return Ok((revision(format!("unknown-cast-{index}").as_bytes()), vec![]));
    };
    if cast.is_empty() || cast.len() > 16 {
        return Err("analysis requires 1..16 actors in each recorded cast".into());
    }
    let mut actors = BTreeMap::new();
    for actor in cast {
        let name = actor["name"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or("analysis cast actor needs a bounded name")?;
        if actors.contains_key(name) {
            return Err("analysis cast actor names must be unique".into());
        }
        let selector = |key: &str| -> Result<Option<String>> {
            match &actor[key] {
                Value::Null => Ok(None),
                Value::String(s) if s.len() <= 4096 => Ok(Some(s.clone())),
                _ => Err(format!("analysis cast {key} selector must be a string or null").into()),
            }
        };
        actors.insert(
            name.to_owned(),
            ActorVariant {
                name: name.into(),
                model: selector("model")?,
                prompts: selector("prompts")?,
                context: selector("context")?,
                character: selector("character")?,
                budget: selector("budget")?,
                specification_revision: revision(&serde_json::to_vec(&canonical(actor))?),
            },
        );
    }
    let actors: Vec<_> = actors.into_values().collect();
    let id = revision(&serde_json::to_vec(&actors)?);
    Ok((id, actors))
}

fn record_failure(
    run: &mut Run,
    scope: &str,
    category: &str,
    index: Option<usize>,
    matcher: &Value,
) {
    if run.failures.len() == MAX_FAILURES {
        run.omitted_failures += 1;
        return;
    }
    run.failures.push(Failure {
        scope: bounded(scope, 200),
        category: category.into(),
        assertion_index: index,
        actor: label(&matcher["actor"], 256),
        event_kind: label(&matcher["kind"], 100),
    });
}

fn assertions(run: &mut Run, scope: &str, value: &Value) {
    let before = run.failures.len() + run.omitted_failures;
    for (key, category) in [
        ("missing_eventually", "missing_eventually"),
        ("remaining_ordered", "remaining_ordered"),
    ] {
        if let Some(items) = value[key].as_array() {
            for (index, matcher) in items.iter().enumerate() {
                record_failure(run, scope, category, Some(index + 1), matcher);
            }
        }
    }
    if !value["forbidden_failure"].is_null() {
        record_failure(
            run,
            scope,
            "forbidden_observed",
            None,
            &value["forbidden_failure"]["matcher"],
        );
    }
    if value["success"] == false && before == run.failures.len() + run.omitted_failures {
        record_failure(
            run,
            scope,
            "assertion_failed_without_details",
            None,
            &Value::Null,
        );
    }
}

fn error_category(value: &Value) -> &'static str {
    let text = value.as_str().unwrap_or("");
    if text == "harness interrupted" {
        "interrupted"
    } else if text.contains("timed out") || text.contains("deadline") {
        "timeout"
    } else if text.contains("forbidden") {
        "forbidden_observed"
    } else if text.contains("expected status") {
        "unexpected_actor_status"
    } else if text.contains("exited before") {
        "early_actor_exit"
    } else {
        "harness_error"
    }
}

fn probe_grade(value: &Value) -> Result<ProbeGrade> {
    if value["version"] != 1 || !value.is_object() {
        return Err("analysis supports complete version-1 probe grades".into());
    }
    let id = |value: &Value| -> Result<String> {
        value
            .as_str()
            .filter(|s| crate::organization::identifier(s))
            .map(str::to_owned)
            .ok_or_else(|| "analysis probe/check IDs must be bounded identifiers".into())
    };
    let hash = |key: &str| -> Result<String> {
        value[key]
            .as_str()
            .filter(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))
            .map(str::to_owned)
            .ok_or_else(|| "analysis probe revisions must be SHA256 strings".into())
    };
    let success = value["success"]
        .as_bool()
        .ok_or("analysis probe grade needs a boolean success")?;
    let response_valid = value["response_valid"]
        .as_bool()
        .ok_or("analysis probe grade needs response_valid")?;
    let source = value["checks"]
        .as_array()
        .filter(|c| !c.is_empty() && c.len() <= 64)
        .ok_or("analysis probe grade needs 1..64 checks")?;
    let mut seen = BTreeSet::new();
    let mut checks = Vec::with_capacity(source.len());
    for check in source {
        let id = id(&check["id"])?;
        if !seen.insert(id.clone()) {
            return Err("analysis probe grade repeats a check ID".into());
        }
        checks.push(ProbeCheck {
            id,
            passed: check["passed"]
                .as_bool()
                .ok_or("analysis probe check needs a boolean outcome")?,
        });
    }
    if success != (response_valid && checks.iter().all(|c| c.passed)) {
        return Err("analysis probe success disagrees with its response validity or checks".into());
    }
    Ok(ProbeGrade {
        probe_id: id(&value["probe_id"])?,
        definition_revision: hash("definition_revision")?,
        request_revision: hash("request_revision")?,
        success,
        response_valid,
        checks,
    })
}

fn probes(run: &mut Run, result: &Value) -> Result<()> {
    if result["probes_omitted"].is_null() {
        run.missing_metrics.push("probes_omitted".into());
    } else {
        run.probes_omitted = Some(
            number(&result["probes_omitted"])
                .ok_or("analysis probes_omitted must be a nonnegative integer")?,
        );
    }
    if result["probes"].is_null() {
        run.missing_metrics.push("probes".into());
        return Ok(());
    }
    let source = result["probes"]
        .as_array()
        .filter(|p| p.len() <= MAX_PROBES)
        .ok_or("analysis probes must be an array of at most 4096 evaluations")?;
    let mut evaluations = Vec::with_capacity(source.len());
    for (index, entry) in source.iter().enumerate() {
        let actor = entry["actor"]
            .as_str()
            .filter(|s| crate::organization::identifier(s))
            .ok_or("analysis probe evaluation needs a bounded actor identifier")?;
        if !run.actors.is_empty() && !run.actors.iter().any(|a| a.name == actor) {
            return Err("analysis probe evaluation names an unknown actor".into());
        }
        let seq = if entry["seq"].is_null() {
            run.missing_metrics
                .push(format!("probes.{}.seq", index + 1));
            None
        } else {
            Some(
                entry["seq"]
                    .as_u64()
                    .ok_or("analysis probe sequence must be a nonnegative integer")?,
            )
        };
        let grade = if entry["grade"].is_null() {
            run.missing_metrics
                .push(format!("probes.{}.grade", index + 1));
            None
        } else {
            let grade = probe_grade(&entry["grade"])?;
            let scope = format!(
                "probe:{}:{}:evaluation:{}",
                grade.probe_id,
                &grade.definition_revision[..12],
                index + 1
            );
            if !grade.success {
                record_failure(
                    run,
                    &scope,
                    "probe_failed",
                    None,
                    &json!({"actor":actor,"kind":"probe-grade"}),
                );
            }
            if !grade.response_valid {
                record_failure(
                    run,
                    &scope,
                    "probe_response_invalid",
                    None,
                    &json!({"actor":actor,"kind":"probe-grade"}),
                );
            }
            for (index, check) in grade.checks.iter().enumerate().filter(|(_, c)| !c.passed) {
                record_failure(
                    run,
                    &scope,
                    "probe_check_failed",
                    Some(index + 1),
                    &json!({"actor":actor,"kind":format!("probe-check:{}",check.id)}),
                );
            }
            Some(grade)
        };
        evaluations.push(ProbeEvaluation {
            actor: actor.into(),
            seq,
            grade,
        });
    }
    run.probes = Some(evaluations);
    Ok(())
}

/// Analyze one version-1 harness summary. The outcome recorded by the harness
/// remains authoritative; this does not rerun assertions against transcripts.
pub fn analyze(summary: &Value) -> Result<Report> {
    if serde_json::to_vec(summary)?.len() > MAX_SUMMARY_BYTES {
        return Err("analysis summary exceeds 64 MB".into());
    }
    if summary["version"] != 1 {
        return Err("analysis supports harness summary version 1".into());
    }
    let source = summary["runs"]
        .as_array()
        .filter(|r| !r.is_empty() && r.len() <= MAX_RUNS)
        .ok_or("analysis requires 1..4096 recorded runs")?;
    let mut report = Report { version: 1, name: label(&summary["name"], 256)
        .unwrap_or_else(|| "Harness experiment".into()), success: true,
        metrics: Aggregate::default(), variants: vec![], runs: vec![], limitations: vec![
            "Offline analysis uses only summary.json; referenced configs, traces and actor files are never opened.".into(),
            "Variants compare recorded actor specifications and path selectors, not file contents or live model settings. Null selectors mean inherited or unspecified.".into(),
            "Model requests come from session accounting; current_goal_model_calls is separate because /goal resets that counter. Runtime model switching is reflected in cost breakdowns.".into(),
            "Simulation is reported only when the actor report explicitly records it; a mock model name or response tape selector alone is not proof of provider charges or their absence.".into(),
            "Known cost totals cover reported actor sessions only; missing sessions and unknown_cost_requests are not zero spend. Group and lifetime totals are not added.".into(),
            "Token totals retain ledger semantics: absent provider token fields are not reconstructed. Timing is reported run wall time and has no inferred zero samples.".into(),
            "Failure diagnostics omit matching literals and event bodies. Assertion indexes within missing/remaining lists are local to those lists.".into(),
            "Probe counts cover recorded evaluations only. Missing probe arrays, null grades and omitted evaluations retain separate coverage; omissions cannot be assigned a rubric or outcome. Rubrics group by both ID and definition revision.".into(),
        ] };
    let mut variants = BTreeMap::<String, Variant>::new();
    let mut indexes = BTreeSet::new();
    for entry in source {
        let index = entry["index"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or("analysis run needs a positive index")?;
        if !indexes.insert(index) {
            return Err("analysis run indexes must be unique".into());
        }
        let (mut variant_id, cast) = variant(&entry["cast"], index)?;
        let result = &entry["result"];
        if let Some(conditions) = result.pointer("/conversation/conditions") {
            variant_id = revision(&serde_json::to_vec(&serde_json::json!({
                "cast":variant_id,"conversation":canonical(conditions)
            }))?);
        }
        let interactive_debug = if result["debug"].is_null() {
            None
        } else {
            Some(
                result["debug"]["enabled"]
                    .as_bool()
                    .ok_or("analysis debug marker needs a boolean enabled field")?,
            )
        };
        let duration = |value: &Value| -> Result<Option<u128>> {
            if value.is_null() {
                Ok(None)
            } else {
                Ok(Some(number(value).ok_or(
                    "analysis debug durations must be nonnegative integers",
                )?))
            }
        };
        let debug_held_ms = duration(&result["debug"]["held_ms"])?;
        let active_elapsed_ms = duration(&result["active_elapsed_ms"])?;
        let elapsed_ms = number(&result["elapsed_ms"]);
        if (interactive_debug == Some(false) && debug_held_ms.is_some_and(|ms| ms != 0))
            || debug_held_ms
                .zip(elapsed_ms)
                .is_some_and(|(held, wall)| held > wall)
            || active_elapsed_ms
                .zip(elapsed_ms)
                .is_some_and(|(active, wall)| active > wall)
        {
            return Err("analysis debug timing contradicts its run metadata".into());
        }
        let mut run = Run {
            index,
            repetition: entry["repetition"].as_u64(),
            variant_id: variant_id.clone(),
            success: result["success"].as_bool(),
            elapsed_ms,
            active_elapsed_ms,
            interactive_debug,
            debug_held_ms,
            assertions_success: result["assertions"]["success"].as_bool(),
            actors: vec![],
            probes: None,
            probes_omitted: None,
            failures: vec![],
            omitted_failures: 0,
            missing_metrics: vec![],
        };
        if cast.is_empty() {
            run.missing_metrics.push("cast".into());
        }
        if run.success.is_none() {
            run.missing_metrics.push("success".into());
        }
        if run.elapsed_ms.is_none() {
            run.missing_metrics.push("elapsed_ms".into());
        }
        if run.interactive_debug.is_none() {
            run.missing_metrics.push("interactive_debug".into());
        }
        if run.debug_held_ms.is_none() {
            run.missing_metrics.push("debug_held_ms".into());
        }
        if run.active_elapsed_ms.is_none() {
            run.missing_metrics.push("active_elapsed_ms".into());
        }
        if run.assertions_success.is_none() {
            run.missing_metrics.push("assertions.success".into());
        }
        let mut actor_names: BTreeSet<String> = cast.iter().map(|a| a.name.clone()).collect();
        if let Some(actors) = result["actors"].as_object() {
            if actors.len() > 16 || actors.keys().any(|s| s.len() > 256) {
                return Err("analysis actor report names exceed bounds".into());
            }
            actor_names.extend(actors.keys().cloned());
        } else {
            run.missing_metrics.push("actors".into());
        }
        for name in actor_names {
            let actor = &result["actors"][&name]["report"];
            let mut missing = |field: &str| {
                run.missing_metrics
                    .push(format!("actors.{}.{}", bounded(&name, 256), field))
            };
            let status = label(&actor["state"]["status"], 100);
            if status.is_none() {
                missing("status");
            }
            let actual_model = label(&actor["model"]["settings"]["name"], 512);
            let model_profile = label(&actor["model"]["profile"], 100);
            let simulated_model = actor["simulated_model"].as_bool();
            if simulated_model.is_none() {
                missing("simulated_model");
            }
            let commands_sent = number(&actor["state"]["commands_sent"]);
            if commands_sent.is_none() {
                missing("commands_sent");
            }
            let current_goal_model_calls = number(&actor["state"]["model_calls"]);
            if current_goal_model_calls.is_none() {
                missing("current_goal_model_calls");
            }
            let session = &actor["costs"]["session"];
            let costs = if session.is_null() {
                None
            } else {
                // Invalid old/partial accounting is absent data, not a fabricated
                // default ledger. Full well-formed sessions still aggregate.
                serde_json::from_value::<Breakdown>(session.clone()).ok()
            };
            if costs.is_none() {
                missing("session_costs");
            }
            let model_requests = costs
                .as_ref()
                .and_then(|_| number(&session["totals"]["requests"]));
            if model_requests.is_none() {
                missing("model_requests");
            }
            run.actors.push(ActorRun {
                name,
                status,
                actual_model,
                model_profile,
                simulated_model,
                commands_sent,
                model_requests,
                current_goal_model_calls,
                session_costs: costs,
            });
        }
        if !result["error"].is_null() {
            record_failure(
                &mut run,
                "run",
                error_category(&result["error"]),
                None,
                &Value::Null,
            );
        }
        probes(&mut run, result)?;
        assertions(&mut run, "run", &result["assertions"]);
        if let Some(steps) = result["steps"].as_array() {
            if steps.len() > 4096 {
                return Err("analysis step count exceeds limit".into());
            }
            for (index, step) in steps.iter().enumerate() {
                assertions(
                    &mut run,
                    &format!("step:{}", index + 1),
                    &step["assertions"],
                );
            }
        }
        if !result["pending_step"].is_null() {
            let scope = format!(
                "step:{}",
                result["steps"].as_array().map_or(1, |s| s.len() + 1)
            );
            record_failure(&mut run, &scope, "phase_incomplete", None, &Value::Null);
            assertions(&mut run, &scope, &result["pending_step"]["assertions"]);
            if let Some(waiting) = result["pending_step"]["waiting_for"].as_array() {
                for actor in waiting {
                    record_failure(
                        &mut run,
                        &scope,
                        "missing_control_acknowledgment",
                        None,
                        &json!({"actor":actor}),
                    );
                }
            }
        }
        if run.success == Some(false) && run.failures.is_empty() {
            record_failure(
                &mut run,
                "run",
                "unsuccessful_without_details",
                None,
                &Value::Null,
            );
        }
        if run.success == Some(true) && !run.failures.is_empty() {
            return Err(format!(
                "analysis run {index} claims success but records a failure or incomplete phase"
            )
            .into());
        }
        report.success &= run.success == Some(true);
        report.metrics.include(&run)?;
        let group = variants
            .entry(variant_id.clone())
            .or_insert_with(|| Variant {
                id: variant_id,
                actors: cast,
                run_indexes: vec![],
                metrics: Aggregate::default(),
            });
        group.run_indexes.push(index);
        group.metrics.include(&run)?;
        report.runs.push(run);
    }
    if report.metrics.unknown == 0
        && let Some(recorded) = summary["success"].as_bool()
        && recorded != report.success
    {
        return Err("analysis summary success disagrees with its recorded run outcomes".into());
    }
    report.variants = variants.into_values().collect();
    Ok(report)
}

fn xml(value: &str) -> String {
    let mut result = String::new();
    for c in value.chars() {
        match c {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&apos;"),
            '\t' | '\n' | '\r' => result.push(c),
            c if c >= ' ' && c != '\u{fffe}' && c != '\u{ffff}' => result.push(c),
            _ => result.push('\u{fffd}'),
        }
    }
    result
}

fn seconds(ms: u128) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

/// One test case per recorded run. Unknown outcomes become CI errors, not
/// passing cases. Missing durations have no `time` attribute.
pub fn junit(report: &Report) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        out,
        "<testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"{}\">",
        xml(&report.name),
        report.metrics.runs,
        report.metrics.failed,
        report.metrics.unknown
    );
    for run in &report.runs {
        let _ = write!(
            out,
            "  <testcase name=\"run-{}\" classname=\"variant.{}\"",
            run.index,
            xml(&run.variant_id)
        );
        if let Some(ms) = run.elapsed_ms {
            let _ = write!(out, " time=\"{}\"", seconds(ms));
        }
        if run.success == Some(true) {
            out.push_str(" />\n");
            continue;
        }
        out.push_str(">\n");
        let (tag, message) = if run.success.is_none() {
            ("error", "Run outcome is missing")
        } else {
            ("failure", "Harness run did not pass")
        };
        let mut details = String::new();
        for failure in &run.failures {
            let _ = writeln!(
                details,
                "{}: {}{}{}{}",
                failure.scope,
                failure.category,
                failure
                    .assertion_index
                    .map_or_else(String::new, |n| format!(" (item {n})")),
                failure
                    .actor
                    .as_ref()
                    .map_or_else(String::new, |a| format!(" actor={a}")),
                failure
                    .event_kind
                    .as_ref()
                    .map_or_else(String::new, |k| format!(" event={k}"))
            );
        }
        if run.omitted_failures > 0 {
            let _ = writeln!(
                details,
                "{} additional diagnostics omitted",
                run.omitted_failures
            );
        }
        if !run.missing_metrics.is_empty() {
            let _ = writeln!(
                details,
                "Missing metrics: {}",
                run.missing_metrics.join(", ")
            );
        }
        // The JSON report has all bounded diagnostics; CI receives at most 8k
        // characters per case and never includes raw artifact contents.
        let truncated = details.chars().count() > 8000;
        let mut details: String = details.chars().take(8000).collect();
        if truncated {
            details.push_str("\n[diagnostics bounded]");
        }
        let _ = writeln!(
            out,
            "    <{tag} message=\"{message}\">{}</{tag}>\n  </testcase>",
            xml(&details)
        );
    }
    out.push_str("</testsuite>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conversation_policy_content_and_floor_mode_distinguish_variants() {
        let mut a = run(1, true, "0");
        a["result"]["conversation"]["conditions"] =
            serde_json::json!({"speaking_floor":null,"actors":{"scout":{"reply_min_ms":700}}});
        let mut b = a.clone();
        b["index"] = 2.into();
        b["result"]["conversation"]["conditions"]["speaking_floor"] =
            serde_json::json!({"speaking_window_ms":1800});
        let mut c = a.clone();
        c["index"] = 3.into();
        c["result"]["conversation"]["conditions"]["actors"]["scout"]["reply_min_ms"] = 900.into();
        let mut d = a.clone();
        d["index"] = 4.into();
        let result = analyze(&serde_json::json!({"version":1,"runs":[a,b,c,d]})).unwrap();
        assert_eq!(result.variants.len(), 3);
        assert_eq!(result.runs[0].variant_id, result.runs[3].variant_id);
    }

    fn session(amount: &str, requests: u64) -> Value {
        let mut value = serde_json::to_value(Breakdown::default()).unwrap();
        value["totals"]["requests"] = requests.into();
        value["totals"]["known_cost_requests"] = requests.into();
        value["totals"]["provider_cost_by_currency"] = json!({"USD":amount});
        value
    }
    fn run(index: u64, success: bool, amount: &str) -> Value {
        json!({"index":index,"repetition":index,"cast":[{"name":"scout","model":"test/model","prompts":"prompt.json","context":null,"character":"scout.json","goal":"PRIVATE GOAL"}],
            "result":{"success":success,"elapsed_ms":1250,"actors":{"scout":{"report":{
                "state":{"status":"paused","commands_sent":3,"model_calls":1},"costs":{"session":session(amount,4),"all_sessions":session("999",90)}}}}}})
    }

    #[test]
    fn debug_runs_preserve_hold_timing_and_missing_classification() {
        let mut unattended = run(1, true, "0");
        unattended["result"]["debug"] = json!({"enabled":false,"held_ms":0});
        unattended["result"]["active_elapsed_ms"] = 1250.into();
        let mut interactive = run(2, true, "0");
        interactive["result"]["debug"] = json!({"enabled":true,"held_ms":100});
        interactive["result"]["active_elapsed_ms"] = 1150.into();
        let legacy = run(3, true, "0");
        let report =
            analyze(&json!({"version":1,"runs":[unattended.clone(), interactive, legacy]}))
                .unwrap();
        assert_eq!(report.metrics.debugged_runs, 1);
        assert_eq!(report.metrics.unattended_runs, 1);
        assert_eq!(report.metrics.unknown_debug_runs, 1);
        assert_eq!(report.metrics.debug_held_ms.known_total, Some(100));
        assert_eq!(report.metrics.debug_held_ms.missing, 1);
        assert_eq!(report.metrics.active_elapsed_ms.known_total, Some(2400));
        assert_eq!(report.runs[2].interactive_debug, None);
        for metadata in [
            json!({"enabled":false,"held_ms":1}),
            json!({"enabled":true,"held_ms":1300}),
            json!({"enabled":"false"}),
        ] {
            let mut malformed = unattended.clone();
            malformed["result"]["debug"] = metadata;
            assert!(analyze(&json!({"version":1,"runs":[malformed]})).is_err());
        }
    }

    #[test]
    fn repeated_variants_merge_exact_session_costs_and_keep_coverage() {
        let first = run(1, true, "0.1");
        let mut second = run(2, false, "0.2");
        second["result"]["pending_step"] = json!({"waiting_for":["scout"],"assertions":{
            "success":false,"missing_eventually":[{"actor":"scout","kind":"observation","contains":"PRIVATE BRIEF"}]}});
        let third = json!({"index":3,"cast":first["cast"],"result":{"success":false,"error":"server error PRIVATE"}});
        let report =
            analyze(&json!({"version":1,"name":"Trial","runs":[first,second,third]})).unwrap();
        assert_eq!(report.variants.len(), 1);
        assert_eq!((report.metrics.passed, report.metrics.failed), (1, 2));
        assert_eq!(report.metrics.model_requests.known_total, Some(8));
        assert_eq!(report.metrics.current_goal_model_calls.known_total, Some(2));
        assert_eq!(report.metrics.commands_sent.known_total, Some(6));
        assert_eq!(report.metrics.elapsed_ms.missing, 1);
        assert_eq!(report.metrics.cost_sessions_missing, 1);
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(
            value["metrics"]["session_costs"]["totals"]["provider_cost_by_currency"]["USD"],
            "0.3"
        );
        assert_eq!(report.runs[1].failures.len(), 3);
        assert!(!value.to_string().contains("PRIVATE"));
        assert!(!junit(&report).contains("PRIVATE"));
    }

    #[test]
    fn unknown_data_is_null_and_unknown_outcome_is_a_ci_error() {
        let report =
            analyze(&json!({"version":1,"name":"A & <\"B\">\u{0001}","runs":[{"index":1}]}))
                .unwrap();
        assert!(!report.success);
        assert_eq!(report.metrics.unknown, 1);
        assert!(report.metrics.elapsed_ms.known_total.is_none());
        assert!(report.metrics.session_costs.is_none());
        assert!(report.runs[0].missing_metrics.contains(&"actors".into()));
        let output = junit(&report);
        assert!(output.contains("name=\"A &amp; &lt;&quot;B&quot;&gt;\""));
        assert!(output.contains("errors=\"1\""));
        assert!(output.contains("<error "));
        assert!(!output.contains(" time="));
        assert!(!output.contains('\u{1}'));
    }

    #[test]
    fn actor_order_does_not_split_variants_but_other_authored_inputs_do() {
        let mut first = run(1, true, "0");
        let mut second = run(2, true, "0");
        let actor = json!({"name":"medic","model":"test/model","goal":"heal"});
        first["cast"].as_array_mut().unwrap().push(actor.clone());
        second["cast"].as_array_mut().unwrap().insert(0, actor);
        let mut third = second.clone();
        third["index"] = 3.into();
        third["cast"][0]["goal"] = "scout".into();
        let report = analyze(&json!({"version":1,"runs":[first,second,third]})).unwrap();
        assert_eq!(report.variants.len(), 2);
        assert_eq!(report.runs[0].variant_id, report.runs[1].variant_id);
        assert_ne!(report.runs[1].variant_id, report.runs[2].variant_id);
        assert_eq!(report.metrics.cost_sessions_missing, 3);
    }

    #[test]
    fn partial_cost_data_is_missing_and_failure_exports_are_bounded() {
        let mut entry = run(1, false, "0.1");
        entry["result"]["actors"]["scout"]["report"]["costs"]["session"] =
            json!({"totals":{"requests":100}});
        entry["result"]["assertions"] = json!({"success":false,
            "missing_eventually":(0..200).map(|_| json!({"actor":"a<&\"","kind":"observation","equals":"PRIVATE"})).collect::<Vec<_>>()});
        let report = analyze(&json!({"version":1,"runs":[entry]})).unwrap();
        assert!(report.metrics.session_costs.is_none());
        assert!(report.metrics.model_requests.known_total.is_none());
        assert_eq!(report.runs[0].failures.len(), MAX_FAILURES);
        assert_eq!(report.runs[0].omitted_failures, 72);
        let output = junit(&report);
        assert!(output.contains("actor=a&lt;&amp;&quot;"));
        assert!(!output.contains("PRIVATE"));
        assert!(output.len() < 20_000);
    }

    #[test]
    fn simulation_requires_evidence_and_conflicting_success_is_rejected() {
        let mut entry = run(1, true, "0.1");
        let actor = &mut entry["result"]["actors"]["scout"]["report"];
        actor["simulated_model"] = true.into();
        actor["model"] = json!({"profile":"fast","settings":{"name":"model/actual"}});
        let report = analyze(&json!({"version":1,"runs":[entry.clone()]})).unwrap();
        assert_eq!(report.metrics.simulated_actor_sessions, 1);
        assert_eq!(
            report.runs[0].actors[0].actual_model.as_deref(),
            Some("model/actual")
        );
        assert!(
            report.runs[0]
                .missing_metrics
                .contains(&"assertions.success".into())
        );
        assert!(analyze(&json!({"version":1,"success":false,"runs":[entry.clone()]})).is_err());
        entry["result"]["assertions"] = json!({"success":false});
        assert!(analyze(&json!({"version":1,"runs":[entry]})).is_err());
    }

    fn grade(passed: bool, rubric: &str) -> Value {
        json!({"version":1,"probe_id":"recall_marco","definition_revision":revision(rubric.as_bytes()),
            "request_revision":revision(b"same captured request may be evaluated more than once"),
            "success":passed,"response_valid":true,"checks":[
                {"id":"answer","passed":passed,"reason":"PRIVATE_EXPECTED_ANSWER","expected":"PRIVATE_EXPECTED_ANSWER"},
                {"id":"evidence","passed":true,"reason":"PRIVATE_QUOTE","evidence_ids":[71]}],
            "response":"PRIVATE_RESPONSE","invalid_citations":[{"quote":"PRIVATE_QUOTE"}]})
    }

    #[test]
    fn probes_aggregate_each_evaluation_and_keep_rubric_and_variant_boundaries() {
        let mut first = run(1, true, "0");
        let mut second = run(2, false, "0");
        let mut third = run(3, true, "0");
        let legacy = run(4, true, "0");
        let mut empty = run(5, true, "0");
        let mut ungraded = run(6, false, "0");
        first["result"]["probes"] = json!([
            {"actor":"scout","seq":10,"grade":grade(true,"rubric-a")},
            {"actor":"scout","seq":20,"grade":grade(true,"rubric-a")}]);
        second["result"]["probes"] =
            json!([{ "actor":"scout","seq":10,"grade":grade(false,"rubric-a")}]);
        third["cast"][0]["model"] = "another/model".into();
        third["result"]["probes"] =
            json!([{ "actor":"scout","seq":10,"grade":grade(true,"rubric-b")}]);
        empty["result"]["probes"] = json!([]);
        ungraded["result"]["probes"] =
            json!([{ "actor":"scout","seq":10,"grade":null,"outcome":"probe-error"}]);
        for entry in [
            &mut first,
            &mut second,
            &mut third,
            &mut empty,
            &mut ungraded,
        ] {
            entry["result"]["probes_omitted"] = 0.into();
        }
        ungraded["result"]["probes_omitted"] = 2.into();
        let report =
            analyze(&json!({"version":1,"runs":[first,second,third,legacy,empty,ungraded]}))
                .unwrap();
        let totals = &report.metrics.probes;
        assert_eq!((totals.runs_reported, totals.runs_missing), (5, 1));
        assert_eq!(
            (
                totals.totals.evaluations,
                totals.totals.graded,
                totals.totals.ungraded
            ),
            (5, 4, 1)
        );
        assert_eq!((totals.totals.passed, totals.totals.failed), (3, 1));
        assert_eq!(
            (totals.totals.checks_passed, totals.totals.checks_failed),
            (7, 1)
        );
        assert_eq!(totals.omitted_evaluations.known_total, Some(2));
        assert_eq!(totals.omitted_evaluations.missing, 1);
        assert_eq!(totals.by_actor["scout"].evaluations, 5);
        assert_eq!(totals.by_probe.len(), 2);
        let rubric_a = totals
            .by_probe
            .iter()
            .find(|g| g.definition_revision == revision(b"rubric-a"))
            .unwrap();
        assert_eq!(
            (
                rubric_a.totals.evaluations,
                rubric_a.totals.passed,
                rubric_a.totals.failed
            ),
            (3, 2, 1)
        );
        assert_eq!(rubric_a.by_actor["scout"].checks_failed, 1);
        assert_eq!(report.variants.len(), 2);
        let changed = report
            .variants
            .iter()
            .find(|v| v.run_indexes == [3])
            .unwrap();
        assert_eq!(changed.metrics.probes.totals.evaluations, 1);
        assert_eq!(changed.metrics.probes.by_probe.len(), 1);
        assert!(report.runs[3].probes.is_none());
        assert!(report.runs[4].probes.as_ref().unwrap().is_empty());
        assert!(report.runs[5].probes.as_ref().unwrap()[0].grade.is_none());
        let exported = serde_json::to_value(&report).unwrap().to_string();
        assert!(!exported.contains("PRIVATE_"));
        let xml = junit(&report);
        assert!(!xml.contains("PRIVATE_"));
        assert!(xml.contains("probe_failed"));
        assert!(xml.contains("probe-check:answer"));
    }

    #[test]
    fn failed_or_malformed_probe_grades_cannot_claim_success() {
        let mut entry = run(1, true, "0");
        entry["result"]["probes"] =
            json!([{ "actor":"scout","seq":10,"grade":grade(false,"rubric-a")}]);
        assert!(analyze(&json!({"version":1,"runs":[entry.clone()]})).is_err());
        entry["result"]["success"] = false.into();
        let mut valid = entry.clone();
        let grade = &mut valid["result"]["probes"][0]["grade"];
        grade["checks"][0]["passed"] = true.into();
        grade["response_valid"] = false.into();
        let report = analyze(&json!({"version":1,"runs":[valid]})).unwrap();
        assert_eq!(report.metrics.probes.totals.invalid_responses, 1);
        assert_eq!(report.metrics.probes.totals.checks_failed, 0);
        assert!(junit(&report).contains("probe_response_invalid"));
        for (pointer, value) in [
            ("/result/probes/0/grade/success", json!(true)),
            ("/result/probes/0/grade/response_valid", json!(null)),
            (
                "/result/probes/0/grade/definition_revision",
                json!("invalid"),
            ),
            ("/result/probes/0/grade/checks", json!([])),
            ("/result/probes/0/grade/checks/1/id", json!("answer")),
            ("/result/probes/0/grade/checks/0/passed", json!("false")),
            ("/result/probes/0/actor", json!("unknown_actor")),
            ("/result/probes/0/seq", json!("10")),
            ("/result/probes", json!({})),
            ("/result/probes/0/grade", json!({})),
        ] {
            let mut malformed = entry.clone();
            *malformed.pointer_mut(pointer).unwrap() = value;
            assert!(
                analyze(&json!({"version":1,"runs":[malformed]})).is_err(),
                "accepted malformed {pointer}"
            );
        }
        entry["result"]["probes_omitted"] = (-1).into();
        assert!(analyze(&json!({"version":1,"runs":[entry]})).is_err());
    }

    #[test]
    fn null_probe_reports_and_ungraded_attempts_remain_unknown() {
        let mut legacy = run(1, true, "0");
        legacy["result"]["probes"] = Value::Null;
        let mut empty = run(2, true, "0");
        empty["result"]["probes"] = json!([]);
        empty["result"]["probes_omitted"] = 0.into();
        let mut ungraded = run(3, false, "0");
        ungraded["result"]["probes"] =
            json!([{ "actor":"scout","grade":null,"outcome":"probe-cancelled"}]);
        let report = analyze(&json!({"version":1,"runs":[legacy,empty,ungraded]})).unwrap();
        let probes = &report.metrics.probes;
        assert_eq!((probes.runs_reported, probes.runs_missing), (2, 1));
        assert_eq!(probes.totals.evaluations, 1);
        assert_eq!(probes.totals.ungraded, 1);
        assert_eq!(probes.totals.graded, 0);
        assert_eq!(probes.totals.passed, 0);
        assert_eq!(probes.totals.failed, 0);
        assert!(probes.by_probe.is_empty());
        assert_eq!(probes.omitted_evaluations.reported, 1);
        assert_eq!(probes.omitted_evaluations.missing, 2);
        assert!(
            report.runs[2]
                .missing_metrics
                .contains(&"probes.1.seq".into())
        );
        assert!(
            report.runs[2]
                .missing_metrics
                .contains(&"probes.1.grade".into())
        );
    }
}

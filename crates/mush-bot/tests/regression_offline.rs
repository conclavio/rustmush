//! Regression gates compare recorded artifacts without loading runtime settings.
use std::{
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU32, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use mush_bot::{analysis, costs::Breakdown, experiment::revision};
use serde_json::{Value, json};

struct Fixture {
    directory: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let directory = std::env::temp_dir().join(format!(
            "mush-bot-regression-offline-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir(&directory).unwrap();
        let fixture = Self { directory };
        fixture.write(".env", "MUSH_BOT_BAD_DOTENV=\"unterminated");
        fixture.write(
            "mush-bot.toml",
            "[intentionally invalid runtime configuration",
        );
        fixture.json(
            "policy.json",
            &policy(vec![
                json!({"id":"quality","metric":"run_pass_rate","minimum":"0.5","max_drop":"0"}),
            ]),
        );
        fixture.json("baseline.json", &summary(&[true, true], "0.1", 1000));
        fixture.json("candidate.json", &summary(&[true, true], "0.1", 1000));
        fixture
    }

    fn write(&self, name: &str, contents: &str) {
        std::fs::write(self.directory.join(name), contents).unwrap();
    }

    fn json(&self, name: &str, value: &Value) {
        self.write(name, &serde_json::to_string_pretty(value).unwrap());
    }

    fn read(&self, name: &str) -> Value {
        serde_json::from_slice(&std::fs::read(self.directory.join(name)).unwrap()).unwrap()
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mush-bot"))
            .args(args)
            .current_dir(&self.directory)
            .env_remove("MUSH_BOT_REGRESSION_TEST_SSH_SECRET")
            .env_remove("MUSH_BOT_REGRESSION_TEST_MODEL_SECRET")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn compare(&self, expected_exit: i32) -> Value {
        let output = self.run(&[
            "--compare-runs",
            "policy.json",
            "baseline.json",
            "candidate.json",
        ]);
        assert_eq!(
            output.status.code(),
            Some(expected_exit),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let (Ok(directory), Ok(temporary)) = (
            self.directory.canonicalize(),
            std::env::temp_dir().canonicalize(),
        ) && directory.parent() == Some(temporary.as_path())
            && directory
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("mush-bot-regression-offline-"))
        {
            let _ = std::fs::remove_dir_all(directory);
        }
    }
}

fn policy(checks: Vec<Value>) -> Value {
    json!({"version":1,"checks":checks})
}

fn session(amount: &str) -> Value {
    let mut value = json!(Breakdown::default());
    value["totals"]["requests"] = 2.into();
    value["totals"]["responses"] = 2.into();
    value["totals"]["known_cost_requests"] = 2.into();
    value["totals"]["provider_cost_by_currency"] = json!({"USD":amount});
    value
}

fn summary(outcomes: &[bool], amount: &str, elapsed_ms: u64) -> Value {
    json!({"version":1,"name":"Offline regression fixture","runs":outcomes.iter().enumerate().map(|(index, success)| {
        json!({"index":index+1,"repetition":index+1,"cast":[{
            "name":"scout","config":"referenced-config-must-not-be-opened.toml","model":"baseline-profile","goal":"PRIVATE_GOAL_MARKER"
        }],"result":{"success":success,"elapsed_ms":elapsed_ms,"debug":{"enabled":false,"held_ms":0},"assertions":{"success":success},
            "probes_omitted":0,"probes":[{"actor":"scout","seq":10,"grade":{
                "version":1,"probe_id":"receipt","definition_revision":revision(b"stable rubric"),
                "request_revision":revision(b"saved request"),"success":success,"response_valid":true,
                "checks":[{"id":"amount","passed":success,"reason":"PRIVATE_EXPECTED_ANSWER_MARKER"}],
                "response":"PRIVATE_MODEL_ANSWER_MARKER"
            }}],"actors":{"scout":{"report":{
                "simulated_model":false,"model":{"profile":"baseline-profile","settings":{"name":"recorded/model"}},
                "state":{"status":"paused","commands_sent":3,"model_calls":2},
                "costs":{"session":session(amount),"all_sessions":session("999999")}
            }}}}})
    }).collect::<Vec<_>>()})
}

fn status<'a>(report: &'a Value, id: &str) -> &'a str {
    report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap()["status"]
        .as_str()
        .unwrap()
}

#[test]
fn regression_policy_and_improved_runs_export_offline_json_and_junit() {
    let fixture = Fixture::new();
    fixture.json("baseline.json", &summary(&[true, false], "0.1", 1000));
    let mut candidate = summary(&[true, true], "0.09", 900);
    for run in candidate["runs"].as_array_mut().unwrap() {
        run["cast"][0]["model"] = "candidate-profile".into();
    }
    fixture.json("candidate.json", &candidate);
    let baseline_variant = analysis::analyze(&fixture.read("baseline.json"))
        .unwrap()
        .variants[0]
        .id
        .clone();
    let candidate_variant = analysis::analyze(&candidate).unwrap().variants[0]
        .id
        .clone();
    fixture.json("policy.json", &policy(vec![
        json!({"id":"run_quality","metric":"run_pass_rate","minimum":"0.75","max_drop":"0","min_samples":2,
            "baseline_variant":baseline_variant,"candidate_variant":candidate_variant}),
        json!({"id":"probe_quality","metric":"probe_pass_rate","probe_id":"receipt","minimum":"0.75","max_drop":"0","min_samples":2}),
        json!({"id":"response_validity","metric":"probe_valid_response_rate","probe_id":"receipt","minimum":"1"}),
        json!({"id":"latency","metric":"elapsed_ms_mean","maximum":"1000","max_increase":"0"}),
        json!({"id":"requests","metric":"model_requests_mean","minimum":"2","maximum":"2"}),
        json!({"id":"spend","metric":"provider_cost_mean","currency":"USD","minimum":"0.09","maximum":"0.1","max_increase":"0"}),
    ]));
    let checked = fixture.run(&["--check-regression", "policy.json"]);
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let normalized: Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(normalized["version"], 1);
    assert_eq!(normalized["checks"].as_array().unwrap().len(), 6);
    assert_eq!(normalized["checks"][5]["maximum"], "0.1");
    let original = std::fs::read(fixture.directory.join("baseline.json")).unwrap();
    std::fs::create_dir(fixture.directory.join("candidate-directory")).unwrap();
    fixture.json("candidate-directory/summary.json", &candidate);
    let output = fixture.run(&[
        "--compare-runs",
        "policy.json",
        "baseline.json",
        "candidate-directory",
        "--report",
        "regression.json",
        "--junit",
        "regression.xml",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = fixture.read("regression.json");
    assert_eq!(report["success"], true);
    assert_eq!(report["policy"], normalized);
    let normalized_policy: mush_bot::regression::Policy =
        serde_json::from_value(normalized).unwrap();
    assert_eq!(
        report["policy_revision"],
        revision(&serde_json::to_vec(&normalized_policy).unwrap())
    );
    for id in [
        "run_quality",
        "probe_quality",
        "response_validity",
        "latency",
        "requests",
        "spend",
    ] {
        assert_eq!(status(&report, id), "pass");
    }
    let xml = std::fs::read_to_string(fixture.directory.join("regression.xml")).unwrap();
    assert!(xml.contains("<testcase"));
    assert!(!xml.contains("<failure") && !xml.contains("<error"));
    for private in [
        "PRIVATE_GOAL_MARKER",
        "PRIVATE_EXPECTED_ANSWER_MARKER",
        "PRIVATE_MODEL_ANSWER_MARKER",
    ] {
        assert!(!report.to_string().contains(private));
        assert!(!xml.contains(private));
    }
    assert_eq!(
        std::fs::read(fixture.directory.join("baseline.json")).unwrap(),
        original
    );
    assert!(!fixture.directory.join(".mush-bot").exists());
}

#[test]
fn quality_and_cost_regressions_fail_the_cli_and_junit() {
    let fixture = Fixture::new();
    fixture.json("candidate.json", &summary(&[true, false], "0.11", 1000));
    fixture.json(
        "policy.json",
        &policy(vec![
            json!({"id":"quality","metric":"run_pass_rate","max_drop":"0"}),
            json!({"id":"cost","metric":"provider_cost_mean","currency":"USD","max_increase":"0"}),
        ]),
    );
    let report = fixture.compare(1);
    assert_eq!(report["success"], false);
    assert_eq!(status(&report, "quality"), "fail");
    assert_eq!(status(&report, "cost"), "fail");
    let output = fixture.run(&[
        "--compare-runs",
        "policy.json",
        "baseline.json",
        "candidate.json",
        "--junit",
        "failed.xml",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        std::fs::read_to_string(fixture.directory.join("failed.xml"))
            .unwrap()
            .contains("<failure")
    );
}

#[test]
fn missing_unknown_and_insufficient_samples_are_inconclusive() {
    let fixture = Fixture::new();
    let base = summary(&[true, true], "0.1", 1000);
    let cases = [
        (
            json!({"id":"metric","metric":"elapsed_ms_mean","maximum":"2000"}),
            "/runs/1/result/elapsed_ms",
            Value::Null,
        ),
        (
            json!({"id":"metric","metric":"provider_cost_mean","currency":"USD","maximum":"1"}),
            "/runs/1/result/actors/scout/report/costs/session",
            Value::Null,
        ),
        (
            json!({"id":"metric","metric":"probe_pass_rate","probe_id":"receipt","minimum":"0"}),
            "/runs/1/result/probes",
            Value::Null,
        ),
        (
            json!({"id":"metric","metric":"run_pass_rate","minimum":"0"}),
            "/runs/1/result/success",
            Value::Null,
        ),
        (
            json!({"id":"metric","metric":"run_pass_rate","minimum":"0"}),
            "/runs/1/result/actors/scout/report/simulated_model",
            Value::Null,
        ),
        (
            json!({"id":"metric","metric":"run_pass_rate","minimum":"0"}),
            "/runs/1/result/actors/scout/report/simulated_model",
            true.into(),
        ),
    ];
    for (check, pointer, missing) in cases {
        let mut candidate = base.clone();
        *candidate.pointer_mut(pointer).unwrap() = missing;
        fixture.json("candidate.json", &candidate);
        fixture.json("policy.json", &policy(vec![check]));
        assert_eq!(
            status(&fixture.compare(1), "metric"),
            "inconclusive",
            "missing {pointer}"
        );
    }
    let mut candidate = base.clone();
    let totals = &mut candidate["runs"][1]["result"]["actors"]["scout"]["report"]["costs"]["session"]
        ["totals"];
    totals["known_cost_requests"] = 1.into();
    totals["unknown_cost_requests"] = 1.into();
    fixture.json("candidate.json", &candidate);
    fixture.json(
        "policy.json",
        &policy(vec![
            json!({"id":"metric","metric":"provider_cost_mean","currency":"USD","maximum":"100"}),
        ]),
    );
    assert_eq!(status(&fixture.compare(1), "metric"), "inconclusive");
    fixture.json("candidate.json", &summary(&[true], "0.1", 1000));
    fixture.json(
        "policy.json",
        &policy(vec![
            json!({"id":"metric","metric":"run_pass_rate","minimum":"0","min_samples":2}),
        ]),
    );
    assert_eq!(status(&fixture.compare(1), "metric"), "inconclusive");
}

#[test]
fn cost_limits_compare_the_last_decimal_digit_without_float_rounding() {
    let fixture = Fixture::new();
    fixture.json(
        "baseline.json",
        &summary(&[true], "0.100000000000000001", 1000),
    );
    fixture.json(
        "candidate.json",
        &summary(&[true], "0.100000000000000002", 1000),
    );
    fixture.json("policy.json", &policy(vec![json!({"id":"cost","metric":"provider_cost_mean","currency":"USD","max_increase":"0.000000000000000001"})]));
    assert_eq!(status(&fixture.compare(0), "cost"), "pass");
    fixture.json(
        "candidate.json",
        &summary(&[true], "0.100000000000000003", 1000),
    );
    assert_eq!(status(&fixture.compare(1), "cost"), "fail");
}

#[test]
fn probe_rubric_drift_and_ungraded_evidence_do_not_produce_a_quality_verdict() {
    let fixture = Fixture::new();
    fixture.json(
        "policy.json",
        &policy(vec![
            json!({"id":"quality","metric":"probe_pass_rate","probe_id":"receipt","minimum":"0"}),
        ]),
    );
    let mut candidate = fixture.read("candidate.json");
    for run in candidate["runs"].as_array_mut().unwrap() {
        run["result"]["probes"][0]["grade"]["definition_revision"] =
            revision(b"changed rubric").into();
    }
    fixture.json("candidate.json", &candidate);
    let report = fixture.compare(1);
    assert_eq!(status(&report, "quality"), "inconclusive");
    assert!(
        report["checks"][0]["reasons"]
            .to_string()
            .contains("rubric")
    );
    let mut candidate = summary(&[true, false], "0.1", 1000);
    candidate["runs"][1]["result"]["probes"][0]["grade"] = Value::Null;
    fixture.json("candidate.json", &candidate);
    assert_eq!(status(&fixture.compare(1), "quality"), "inconclusive");
}

#[test]
fn explicit_gate_thresholds_can_pass_even_when_source_runs_failed() {
    let fixture = Fixture::new();
    let failed = summary(&[false, false], "0.1", 1000);
    assert!(!analysis::analyze(&failed).unwrap().success);
    fixture.json("baseline.json", &failed);
    fixture.json("candidate.json", &failed);
    fixture.json("policy.json", &policy(vec![json!({"id":"permitted_failure_rate","metric":"run_pass_rate","minimum":"0","maximum":"0","max_drop":"0"})]));
    let report = fixture.compare(0);
    assert_eq!(report["success"], true);
    assert_eq!(status(&report, "permitted_failure_rate"), "pass");
}

#[test]
fn regression_modes_reject_conflicts_and_preserve_existing_outputs() {
    let fixture = Fixture::new();
    let conflicts: &[&[&str]] = &[
        &[
            "--check-regression",
            "policy.json",
            "--compare-runs",
            "policy.json",
            "baseline.json",
            "candidate.json",
        ],
        &[
            "--compare-runs",
            "policy.json",
            "baseline.json",
            "candidate.json",
            "--goal",
            "Act",
        ],
        &[
            "--compare-runs",
            "policy.json",
            "baseline.json",
            "candidate.json",
            "--tui",
        ],
        &[
            "--compare-runs",
            "policy.json",
            "baseline.json",
            "candidate.json",
            "--include-content",
        ],
        &[
            "--compare-runs",
            "policy.json",
            "baseline.json",
            "candidate.json",
            "--check-probe",
            "missing.json",
        ],
        &["--check-regression", "policy.json", "--headless"],
        &["--compare-runs", "policy.json", "baseline.json"],
    ];
    for args in conflicts {
        let output = fixture.run(args);
        assert_eq!(
            output.status.code(),
            Some(1),
            "accepted conflicting arguments {args:?}"
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains("could not load .env"));
    }
    fixture.write("existing.json", "KEEP JSON EXACTLY");
    fixture.write("existing.xml", "KEEP XML EXACTLY");
    for args in [
        vec![
            "--compare-runs",
            "policy.json",
            "baseline.json",
            "candidate.json",
            "--report",
            "existing.json",
            "--junit",
            "must-not-create.xml",
        ],
        vec![
            "--compare-runs",
            "policy.json",
            "baseline.json",
            "candidate.json",
            "--report",
            "must-not-create.json",
            "--junit",
            "existing.xml",
        ],
    ] {
        let output = fixture.run(&args);
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("already exists"));
    }
    assert_eq!(
        std::fs::read_to_string(fixture.directory.join("existing.json")).unwrap(),
        "KEEP JSON EXACTLY"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.directory.join("existing.xml")).unwrap(),
        "KEEP XML EXACTLY"
    );
    assert!(!fixture.directory.join("must-not-create.xml").exists());
    assert!(!fixture.directory.join("must-not-create.json").exists());
}

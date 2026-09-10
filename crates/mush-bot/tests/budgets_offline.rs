//! Budget policies are JSON artifacts that can be checked without a game,
//! model, runtime configuration, credentials, or dotenv.
use std::{
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU32, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

struct Fixture {
    directory: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let directory = std::env::temp_dir().join(format!(
            "mush-bot-budget-offline-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir(&directory).unwrap();
        let fixture = Self { directory };
        fixture.write(".env", "MUSH_BOT_INVALID_DOTENV=\"unterminated");
        fixture.write("mush-bot.toml", "[invalid runtime configuration");
        fixture.write("trace.jsonl", "{\"kind\":\"ready\",\"data\":null}\n");
        fixture.json("budget.json", json!({"version":1,"max_requests":2}));
        fixture
    }

    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.directory.join(name), text).unwrap();
    }

    fn json(&self, name: &str, value: Value) {
        self.write(name, &serde_json::to_string_pretty(&value).unwrap());
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mush-bot"))
            .args(args)
            .current_dir(&self.directory)
            .env_remove("MUSH_BOT_BUDGET_TEST_MODEL_SECRET")
            .env_remove("MUSH_BOT_BUDGET_TEST_SSH_SECRET")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn success(&self, args: &[&str]) -> Output {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn failure(&self, args: &[&str]) -> Output {
        let output = self.run(args);
        let error = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{args:?}: {error}");
        assert!(!error.contains("could not load .env"), "{args:?}: {error}");
        output
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
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("mush-bot-budget-offline-"))
        {
            let _ = std::fs::remove_dir_all(directory);
        }
    }
}

#[test]
fn check_budget_normalizes_exact_json_without_loading_runtime_or_environment() {
    let fixture = Fixture::new();
    fixture.json(
        "budget.json",
        json!({
            "version":1,"max_requests":0,
            "max_cost_by_currency":{"USD":"0.250000000000000001","EUR":"2.5000"},
            "unknown_cost":"allow"
        }),
    );
    let output = fixture.success(&[
        "--config",
        "missing-runtime.toml",
        "--check-budget",
        "budget.json",
    ]);
    let normalized: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        normalized,
        json!({
            "version":1,"max_requests":0,
            "max_cost_by_currency":{"USD":"0.250000000000000001","EUR":"2.5"},
            "unknown_cost":"allow"
        })
    );
    assert!(!fixture.directory.join(".mush-bot").exists());

    fixture.json("defaults.json", json!({"version":1}));
    fixture.success(&[
        "--check-budget",
        "defaults.json",
        "--report",
        "normalized.json",
    ]);
    let defaults: Value =
        serde_json::from_slice(&std::fs::read(fixture.directory.join("normalized.json")).unwrap())
            .unwrap();
    assert_eq!(
        defaults,
        json!({
            "version":1,"max_requests":null,"max_cost_by_currency":{},"unknown_cost":"block"
        })
    );
    let repeated = fixture.success(&["--check-budget", "normalized.json"]);
    assert_eq!(
        serde_json::from_slice::<Value>(&repeated.stdout).unwrap(),
        defaults
    );
}

#[test]
fn invalid_budget_policies_fail_before_creating_reports() {
    let fixture = Fixture::new();
    for invalid in [
        json!({"version":2}),
        json!({"version":1,"max_requests":-1}),
        json!({"version":1,"max_requests":1.5}),
        json!({"version":1,"max_requests":1000001}),
        json!({"version":1,"max_requests":"2"}),
        json!({"version":1,"max_cost_by_currency":{"USD":0.25}}),
        json!({"version":1,"max_cost_by_currency":{"usd":"0.25"}}),
        json!({"version":1,"max_cost_by_currency":{"US":"0.25"}}),
        json!({"version":1,"max_cost_by_currency":{"USD":"-1"}}),
        json!({"version":1,"max_cost_by_currency":{"USD":"0.0000000000000000001"}}),
        json!({"version":1,"max_cost_by_currency":{"USD":"NaN"}}),
        json!({"version":1,"unknown_cost":"ignore"}),
        json!({"version":1,"unknown_cost":null}),
        json!({"version":1,"unexpected":"private-policy-value"}),
    ] {
        fixture.json("invalid.json", invalid.clone());
        let output = fixture.failure(&[
            "--check-budget",
            "invalid.json",
            "--report",
            "invalid-report.json",
        ]);
        assert!(output.stdout.is_empty(), "{invalid}");
        assert!(
            !fixture.directory.join("invalid-report.json").exists(),
            "{invalid}"
        );
    }
    for invalid in [
        "{\"version\":1,\"max_requests\":2,\"max_requests\":3}",
        "{\"version\":1,\"max_requests\":",
    ] {
        fixture.write("invalid.json", invalid);
        fixture.failure(&[
            "--check-budget",
            "invalid.json",
            "--report",
            "invalid-report.json",
        ]);
        assert!(!fixture.directory.join("invalid-report.json").exists());
    }
}

#[test]
fn budget_artifacts_require_json_and_refuse_to_overwrite_input_or_output() {
    let fixture = Fixture::new();
    fixture.json("budget.txt", json!({"version":1}));
    fixture.failure(&["--check-budget", "budget.txt"]);
    fixture.failure(&["--check-budget", "budget.json", "--report", "report.txt"]);
    assert!(!fixture.directory.join("report.txt").exists());

    let original = std::fs::read(fixture.directory.join("budget.json")).unwrap();
    fixture.failure(&["--check-budget", "budget.json", "--report", "budget.json"]);
    assert_eq!(
        std::fs::read(fixture.directory.join("budget.json")).unwrap(),
        original
    );

    fixture.write("existing.json", "KEEP_EXISTING_BUDGET_REPORT");
    let failure = fixture.failure(&["--check-budget", "budget.json", "--report", "existing.json"]);
    assert!(String::from_utf8_lossy(&failure.stderr).contains("already exists"));
    assert_eq!(
        std::fs::read_to_string(fixture.directory.join("existing.json")).unwrap(),
        "KEEP_EXISTING_BUDGET_REPORT"
    );
}

#[test]
fn budget_flags_cannot_fall_through_to_other_offline_or_live_modes() {
    let fixture = Fixture::new();
    for args in [
        vec!["--check-budget", "budget.json", "--budget", "budget.json"],
        vec!["--check-budget", "budget.json", "--goal", "Find a rose"],
        vec!["--check-budget", "budget.json", "--tui"],
        vec!["--check-budget", "budget.json", "--json-events"],
        vec!["--check-budget", "budget.json", "--paused"],
        vec!["--check-budget", "budget.json", "--model", "careful"],
        vec!["--check-budget", "budget.json", "--replay", "capture.json"],
        vec!["--check-budget", "budget.json", "--check-prompts"],
        vec![
            "--check-budget",
            "budget.json",
            "--harness",
            "cast.json",
            "--plan-harness",
        ],
        vec![
            "--check-budget",
            "budget.json",
            "--inspect-trace",
            "trace.jsonl",
        ],
        vec!["--check-budget", "budget.json", "--include-content"],
        vec!["--check-budget", "budget.json", "--junit", "result.xml"],
        vec![
            "--check-budget",
            "budget.json",
            "--check-budget",
            "budget.json",
        ],
        vec!["--budget", "budget.json", "--inspect-trace", "trace.jsonl"],
        vec![
            "--budget",
            "budget.json",
            "--check-character",
            "--character",
            "character.json",
        ],
        vec!["--budget", "budget.json", "--budget", "budget.json"],
        vec!["--budget"],
        vec!["--check-budget"],
    ] {
        fixture.failure(&args);
    }
    assert!(!fixture.directory.join(".mush-bot").exists());
    assert!(!fixture.directory.join("result.xml").exists());
}

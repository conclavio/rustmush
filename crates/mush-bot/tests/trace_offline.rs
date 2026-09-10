//! Trace inspection reads only the explicit JSONL and optional query artifact.
use std::{
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU32, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use mush_bot::experiment::revision;
use serde_json::{Value, json};

struct Fixture {
    directory: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let directory = std::env::temp_dir().join(format!(
            "mush-bot-trace-offline-{}-{}-{}",
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
        fixture.write(
            "mush-bot.toml",
            "[intentionally invalid runtime configuration",
        );
        fixture.write("referenced-private.json", "PRIVATE_FILE_MUST_NOT_BE_OPENED");
        fixture.jsonl("standalone.jsonl", &standalone());
        fixture.jsonl("joined.jsonl", &joined());
        fixture
    }

    fn write(&self, name: &str, value: &str) {
        std::fs::write(self.directory.join(name), value).unwrap();
    }

    fn json(&self, name: &str, value: &Value) {
        self.write(name, &serde_json::to_string_pretty(value).unwrap());
    }

    fn jsonl(&self, name: &str, values: &[Value]) {
        let mut text = values
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        text.push('\n');
        self.write(name, &text);
    }

    fn read(&self, name: &str) -> Value {
        serde_json::from_slice(&std::fs::read(self.directory.join(name)).unwrap()).unwrap()
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mush-bot"))
            .args(args)
            .current_dir(&self.directory)
            .env_remove("MUSH_BOT_TRACE_TEST_SSH_SECRET")
            .env_remove("MUSH_BOT_TRACE_TEST_MODEL_SECRET")
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

    fn entries(&self) -> Vec<PathBuf> {
        let mut entries = std::fs::read_dir(&self.directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        entries
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
                .is_some_and(|name| name.starts_with("mush-bot-trace-offline-"))
        {
            let _ = std::fs::remove_dir_all(directory);
        }
    }
}

fn event(seq: u64, timestamp_ms: u64, kind: &str, data: Value) -> Value {
    json!({"seq":seq,"timestamp_ms":timestamp_ms,"request_id":null,"command_sequence":null,"kind":kind,"data":data})
}

fn standalone() -> Vec<Value> {
    let mut input = event(
        2,
        1010,
        "model-input",
        json!({"payload":{"messages":[{"role":"system","content":"PRIVATE_SYSTEM_PROMPT"}]}}),
    );
    input["request_id"] = 11.into();
    let mut response = event(
        3,
        1020,
        "model-reply",
        json!({"content":"PRIVATE_MODEL_RESPONSE"}),
    );
    response["request_id"] = 11.into();
    let mut sent = event(4, 1030, "send", json!("say PRIVATE_COMMAND"));
    sent["command_sequence"] = 4.into();
    vec![
        event(
            1,
            1000,
            "ready",
            json!({"state":{"status":"paused","notes":"PRIVATE_STATE_NOTES"}}),
        ),
        input,
        response,
        sent,
        event(
            5,
            1040,
            "observation",
            json!("Marco says, \"PRIVATE_OBSERVATION\".\n"),
        ),
        event(
            6,
            1050,
            "future_event_type",
            json!({"path":"referenced-private.json","value":"PRIVATE_FUTURE_DATA"}),
        ),
    ]
}

fn joined() -> Vec<Value> {
    vec![
        json!({"elapsed_ms":0,"actor":"_harness","event":{"kind":"phase-start","data":{"id":"phase-1","control":"PRIVATE_HARNESS_CONTROL"}}}),
        json!({"elapsed_ms":10,"actor":"scout","event":event(1,2000,"observation",json!("needle PRIVATE_SCOUT_FIRST"))}),
        json!({"elapsed_ms":20,"actor":"medic","event":event(1,2100,"observation",json!("needle PRIVATE_MEDIC"))}),
        json!({"elapsed_ms":30,"actor":"scout","event":event(2,3000,"send",json!("needle PRIVATE_SCOUT_COMMAND"))}),
        json!({"elapsed_ms":40,"actor":"scout","event":event(3,3010,"observation",json!("Needle PRIVATE_SCOUT_SECOND"))}),
        json!({"elapsed_ms":50,"actor":"scout","event":event(4,3020,"observation",json!("Different PRIVATE_SCOUT_LAST"))}),
    ]
}

#[test]
fn standalone_trace_metadata_is_offline_and_omits_payloads_by_default() {
    let fixture = Fixture::new();
    let entries = fixture.entries();
    let original = std::fs::read(fixture.directory.join("standalone.jsonl")).unwrap();
    let output = fixture.success(&[
        "--inspect-trace",
        "standalone.jsonl",
        "--config",
        "missing-runtime.toml",
    ]);
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["version"], 1);
    assert_eq!(report["source"]["format"], "standalone");
    assert_eq!(report["source"]["sha256"], revision(&original));
    assert_eq!(
        report["totals"],
        json!({"events":6,"matched":6,"returned":6})
    );
    assert_eq!(report["include_content"], false);
    let rows = report["entries"].as_array().unwrap();
    assert_eq!(rows[0]["line"], 1);
    assert_eq!(rows[0]["timestamp_ms"], 1000);
    assert_eq!(rows[0]["seq"], 1);
    assert!(rows[0]["actor"].is_null() && rows[0]["elapsed_ms"].is_null());
    assert_eq!(rows[1]["request_id"], 11);
    assert_eq!(rows[3]["command_sequence"], 4);
    assert_eq!(rows[5]["kind"], "future_event_type");
    assert!(
        rows.iter()
            .all(|row| row.get("raw").is_none() && row.get("data").is_none())
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("PRIVATE_"));
    assert_eq!(fixture.entries(), entries);
    assert_eq!(
        std::fs::read(fixture.directory.join("standalone.jsonl")).unwrap(),
        original
    );
}

#[test]
fn opt_in_content_exports_exact_standalone_and_joined_records() {
    let fixture = Fixture::new();
    for (source, output_name, expected, format) in [
        (
            "standalone.jsonl",
            "standalone-report.json",
            standalone(),
            "standalone",
        ),
        ("joined.jsonl", "joined-report.json", joined(), "joined"),
    ] {
        fixture.success(&[
            "--inspect-trace",
            source,
            "--include-content",
            "--report",
            output_name,
        ]);
        let report = fixture.read(output_name);
        assert_eq!(report["include_content"], true);
        assert_eq!(report["source"]["format"], format);
        let raw = report["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["raw"].clone())
            .collect::<Vec<_>>();
        assert_eq!(raw, expected);
        assert!(
            !report
                .to_string()
                .contains("PRIVATE_FILE_MUST_NOT_BE_OPENED")
        );
    }
    let report = fixture.read("joined-report.json");
    assert_eq!(report["entries"][0]["actor"], "_harness");
    assert_eq!(report["entries"][0]["elapsed_ms"], 0);
    assert!(report["entries"][0]["seq"].is_null());
    assert_eq!(report["entries"][1]["timestamp_ms"], 2000);
    assert_eq!(report["entries"][1]["elapsed_ms"], 10);
}

#[test]
fn joined_trace_filters_and_pagination_use_actor_kind_and_elapsed_time() {
    let fixture = Fixture::new();
    let mut query = json!({"version":1,"actors":["scout"],"kinds":["observation"],"contains":"NEEDLE","from_ms":10,"to_ms":40,"offset":0,"limit":1});
    fixture.json("query.json", &query);
    let output = fixture.success(&[
        "--inspect-trace",
        "joined.jsonl",
        "--trace-query",
        "query.json",
    ]);
    let first: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        first["totals"],
        json!({"events":6,"matched":2,"returned":1})
    );
    assert_eq!(first["entries"][0]["line"], 2);
    assert_eq!(first["entries"][0]["elapsed_ms"], 10);
    assert_eq!(first["pagination"]["next_offset"], 1);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("PRIVATE_"));
    query["offset"] = 1.into();
    fixture.json("query.json", &query);
    let output = fixture.success(&[
        "--inspect-trace",
        "joined.jsonl",
        "--trace-query",
        "query.json",
    ]);
    let second: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        second["totals"],
        json!({"events":6,"matched":2,"returned":1})
    );
    assert_eq!(second["entries"][0]["line"], 5);
    assert_eq!(second["entries"][0]["elapsed_ms"], 40);
    assert!(second["pagination"]["next_offset"].is_null());
    query["offset"] = 99.into();
    fixture.json("query.json", &query);
    let output = fixture.success(&[
        "--inspect-trace",
        "joined.jsonl",
        "--trace-query",
        "query.json",
    ]);
    let empty: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        empty["totals"],
        json!({"events":6,"matched":2,"returned":0})
    );
    assert!(empty["entries"].as_array().unwrap().is_empty());
    assert!(empty["pagination"]["next_offset"].is_null());
}

#[test]
fn malformed_tail_reports_its_exact_line_without_publishing_a_partial_result() {
    let fixture = Fixture::new();
    let rows = standalone();
    fixture.write(
        "broken.jsonl",
        &format!(
            "{}\n{}\n{{\"kind\":\"observation\",\"data\":\"PRIVATE_BROKEN_TAIL\"",
            rows[0], rows[1]
        ),
    );
    fixture.json("one-row.json", &json!({"version":1,"limit":1}));
    let output = fixture.run(&[
        "--inspect-trace",
        "broken.jsonl",
        "--trace-query",
        "one-row.json",
        "--report",
        "partial.json",
    ]);
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("broken.jsonl"));
    assert!(error.contains("line 3: invalid JSON"), "{error}");
    assert!(!error.contains("PRIVATE_BROKEN_TAIL"));
    assert!(!fixture.directory.join("partial.json").exists());
    assert!(output.stdout.is_empty());
}

#[test]
fn invalid_queries_fail_before_writing_reports() {
    let fixture = Fixture::new();
    for query in [
        json!({"version":2}),
        json!({"version":1,"from_ms":40,"to_ms":10}),
        json!({"version":1,"limit":0}),
        json!({"version":1,"offset":-1}),
        json!({"version":1,"unexpected":true}),
        json!({"version":1,"from_ms":"later"}),
    ] {
        fixture.json("invalid-query.json", &query);
        let output = fixture.run(&[
            "--inspect-trace",
            "joined.jsonl",
            "--trace-query",
            "invalid-query.json",
            "--report",
            "invalid-report.json",
        ]);
        assert_eq!(output.status.code(), Some(1), "accepted {query}");
        assert!(!fixture.directory.join("invalid-report.json").exists());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("could not load .env"));
    }
}

#[test]
fn trace_modes_reject_conflicts_and_noninteractive_tui_without_loading_runtime() {
    let fixture = Fixture::new();
    fixture.json("query.json", &json!({"version":1}));
    let conflicts: &[&[&str]] = &[
        &["--trace-query", "query.json"],
        &["--inspect-trace", "standalone.jsonl", "--goal", "Act"],
        &["--inspect-trace", "standalone.jsonl", "--json-events"],
        &[
            "--inspect-trace",
            "standalone.jsonl",
            "--check-prompts",
            "--prompts",
            "missing.json",
        ],
        &[
            "--inspect-trace",
            "standalone.jsonl",
            "--replay",
            "missing.json",
        ],
        &[
            "--inspect-trace",
            "standalone.jsonl",
            "--tui",
            "--report",
            "conflict.json",
        ],
        &[
            "--inspect-trace",
            "standalone.jsonl",
            "--tui",
            "--include-content",
        ],
        &[
            "--inspect-trace",
            "standalone.jsonl",
            "--junit",
            "conflict.xml",
        ],
        &["--inspect-trace"],
    ];
    for args in conflicts {
        let output = fixture.run(args);
        assert_eq!(
            output.status.code(),
            Some(1),
            "accepted incompatible arguments {args:?}"
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains("could not load .env"));
    }
    assert!(!fixture.directory.join("conflict.json").exists());
    assert!(!fixture.directory.join("conflict.xml").exists());
    let output = fixture.run(&[
        "--inspect-trace",
        "standalone.jsonl",
        "--tui",
        "--trace-query",
        "query.json",
    ]);
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    assert!(error.contains("terminal"), "{error}");
    assert!(!error.contains(".env"));
}

#[test]
fn trace_artifacts_require_explicit_extensions_and_cannot_overwrite_existing_files() {
    let fixture = Fixture::new();
    fixture.write("existing.json", "KEEP EXACTLY");
    let output = fixture.run(&[
        "--inspect-trace",
        "standalone.jsonl",
        "--report",
        "existing.json",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("already exists"));
    assert_eq!(
        std::fs::read_to_string(fixture.directory.join("existing.json")).unwrap(),
        "KEEP EXACTLY"
    );
    fixture.write("trace.txt", &format!("{}\n", standalone()[0]));
    fixture.json("query.txt", &json!({"version":1}));
    for args in [
        vec![
            "--inspect-trace",
            "trace.txt",
            "--report",
            "bad-source.json",
        ],
        vec![
            "--inspect-trace",
            "standalone.jsonl",
            "--trace-query",
            "query.txt",
            "--report",
            "bad-query.json",
        ],
        vec![
            "--inspect-trace",
            "standalone.jsonl",
            "--report",
            "report.txt",
        ],
        vec!["--inspect-trace", ".", "--report", "directory.json"],
    ] {
        let output = fixture.run(&args);
        assert_eq!(output.status.code(), Some(1));
    }
    for path in [
        "bad-source.json",
        "bad-query.json",
        "report.txt",
        "directory.json",
    ] {
        assert!(!fixture.directory.join(path).exists());
    }
}

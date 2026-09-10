//! Structured grading consumes saved evidence, without loading runtime settings.
use std::{
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU32, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use mush_bot::{
    composition::Settings,
    experiment::revision,
    memory::{Episode, Event, Record, Source},
    model::Capture,
    probe::Spec,
};
use serde_json::{Value, json};

const QUOTE: &str = "Flower Shop receipt: 7 copper coins paid for one rose.";

struct Fixture {
    directory: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let directory = std::env::temp_dir().join(format!(
            "mush-bot-probes-offline-{}-{}-{}",
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
            "[this is intentionally invalid runtime configuration",
        );
        let spec: Spec = serde_json::from_value(json!({
            "version":1,"id":"receipt","question":"What did you pay, and at which shop?",
            "fields":[
                {"name":"amount","type":"number","description":"The number of copper coins paid."},
                {"name":"shop","type":"string","description":"The shop's name."}
            ],
            "checks":[
                {"id":"amount_paid","field":"amount","equals":7,"evidence":{"source":"observed","contains":"7 copper"}},
                {"id":"purchase_place","field":"shop","equals":"Flower Shop","evidence":{"source":"observed","contains":"Flower Shop"}}
            ]
        })).unwrap();
        fixture.json("probe.json", &json!(spec));
        fixture.json("capture-v1.json", &json!(capture(&spec, false)));
        fixture.json("capture-v2.json", &json!(capture(&spec, true)));
        fixture.json(
            "answer.json",
            &json!({"answers":{"amount":7,"shop":"Flower Shop"},"evidence":[
                {"field":"amount","record_id":41,"quote":QUOTE},
                {"field":"shop","record_id":41,"quote":QUOTE}
            ]}),
        );
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
            .env_remove("MUSH_BOT_PROBE_TEST_SSH_SECRET")
            .env_remove("MUSH_BOT_PROBE_TEST_MODEL_SECRET")
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
                .is_some_and(|n| n.starts_with("mush-bot-probes-offline-"))
        {
            let _ = std::fs::remove_dir_all(directory);
        }
    }
}

fn capture(spec: &Spec, composed: bool) -> Capture {
    let record = Record {
        id: 41,
        at_ms: 1000,
        session: 1,
        event: Event::Episode(Episode {
            source: Source::Observed,
            text: QUOTE.into(),
            place: None,
            after_command: None,
            speaker_labels: vec![],
        }),
    };
    let mut foreign = record.clone();
    foreign.id = 99;
    let context = json!({
        "runner_goal":"Recall the shopping trip",
        "character_profile":{"name":"Rose"},
        "remembered_experience":{"records":[record]},
        "reference_documents":[{"text":json!({"records":[foreign]}).to_string()}]
    });
    let system = "Answer from the evidence supplied in this saved request.";
    let question = spec.question_prompt().unwrap();
    let (messages, composition) = if composed {
        let (messages, report) = Settings::default()
            .prepare(system.into(), &context, Some(&question))
            .unwrap();
        (messages, Some(report))
    } else {
        (
            vec![
                json!({"role":"system","content":system}),
                json!({"role":"user","content":context.to_string()}),
                json!({"role":"user","content":question}),
            ],
            None,
        )
    };
    let payload =
        json!({"model":"offline/no-requests","max_tokens":256,"stream":false,"messages":messages});
    let capture = Capture {
        version: if composed { 2 } else { 1 },
        question: true,
        prompt_revision: revision(system.as_bytes()),
        request_revision: revision(&serde_json::to_vec(&payload).unwrap()),
        payload,
        composition_revision: composition
            .as_ref()
            .map(|report| revision(&serde_json::to_vec(report).unwrap())),
        composition,
    };
    capture.validate().unwrap();
    capture
}

#[test]
fn offline_grading_rejects_duplicate_answer_keys_without_normalizing_them() {
    let fixture = Fixture::new();
    let evidence = fixture.read("answer.json")["evidence"].clone();
    fixture.write(
        "duplicate-answer.json",
        &format!(
            r#"{{"answers":{{"amount":99,"amount":7,"shop":"Flower Shop"}},"evidence":{evidence}}}"#
        ),
    );
    let output = fixture.run(&[
        "--grade-probe",
        "probe.json",
        "capture-v2.json",
        "duplicate-answer.json",
    ]);
    assert_eq!(output.status.code(), Some(1));
    let grade: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(grade["success"], false);
    assert_eq!(grade["response_valid"], false);
}

#[test]
fn check_and_grade_probe_work_offline_with_legacy_and_composed_captures() {
    let fixture = Fixture::new();
    let output = fixture.success(&["--check-probe", "probe.json"]);
    let normalized: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(normalized, fixture.read("probe.json"));
    for (capture, report) in [
        ("capture-v1.json", "grade-v1.json"),
        ("capture-v2.json", "grade-v2.json"),
    ] {
        let original = std::fs::read(fixture.directory.join(capture)).unwrap();
        fixture.success(&[
            "--grade-probe",
            "probe.json",
            capture,
            "answer.json",
            "--report",
            report,
        ]);
        let grade = fixture.read(report);
        assert_eq!(grade["version"], 1);
        assert_eq!(grade["success"], true);
        assert_eq!(grade["response_valid"], true);
        assert_eq!(
            grade["request_revision"],
            fixture.read(capture)["request_revision"]
        );
        assert_eq!(grade["checks"][0]["evidence_ids"], json!([41]));
        assert_eq!(grade["checks"][1]["evidence_ids"], json!([41]));
        assert_eq!(
            std::fs::read(fixture.directory.join(capture)).unwrap(),
            original
        );
    }
    assert!(!fixture.directory.join(".mush-bot").exists());
}

#[test]
fn wrong_answers_return_a_failed_grade_without_copying_answer_or_expected_values() {
    let fixture = Fixture::new();
    let mut answer = fixture.read("answer.json");
    answer["answers"] = json!({"amount":99,"shop":"WRONG_MODEL_VALUE_MARKER"});
    fixture.json("wrong.json", &answer);
    let output = fixture.run(&[
        "--grade-probe",
        "probe.json",
        "capture-v2.json",
        "wrong.json",
    ]);
    assert_eq!(output.status.code(), Some(1));
    let grade: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(grade["success"], false);
    assert_eq!(grade["response_valid"], true);
    assert_eq!(grade["checks"][0]["reason"], "answer_mismatch");
    assert_eq!(grade["checks"][1]["reason"], "answer_mismatch");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(!text.contains("WRONG_MODEL_VALUE_MARKER"));
    assert!(!text.contains("Flower Shop"));
    assert!(!text.contains("7 copper"));
    assert!(grade["checks"][0].get("equals").is_none());
}

#[test]
fn foreign_or_invented_quotes_cannot_satisfy_the_grading_cli() {
    let fixture = Fixture::new();
    for (file, capture, change, reason) in [
        (
            "wrong-quote.json",
            "capture-v1.json",
            json!({"quote":"INVENTED_QUOTE_MARKER"}),
            "quote_not_in_captured_episode",
        ),
        (
            "foreign.json",
            "capture-v2.json",
            json!({"record_id":99}),
            "episode_not_in_prepared_capture",
        ),
    ] {
        let mut answer = fixture.read("answer.json");
        for (key, value) in change.as_object().unwrap() {
            answer["evidence"][0][key] = value.clone();
        }
        fixture.json(file, &answer);
        let output = fixture.run(&["--grade-probe", "probe.json", capture, file]);
        assert_eq!(output.status.code(), Some(1));
        let grade: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(grade["success"], false);
        assert_eq!(grade["response_valid"], false);
        assert_eq!(grade["invalid_citations"][0]["reason"], reason);
        assert!(!String::from_utf8_lossy(&output.stdout).contains("INVENTED_QUOTE_MARKER"));
    }
}

#[test]
fn revised_rubrics_can_regrade_the_same_capture_without_changing_its_prompt() {
    let fixture = Fixture::new();
    let output = fixture.success(&[
        "--grade-probe",
        "probe.json",
        "capture-v2.json",
        "answer.json",
    ]);
    let original_grade: Value = serde_json::from_slice(&output.stdout).unwrap();
    let original_capture = std::fs::read(fixture.directory.join("capture-v2.json")).unwrap();
    let mut spec = fixture.read("probe.json");
    spec["checks"][1]["equals"] = json!("EDITED_EXPECTED_SHOP_MARKER");
    fixture.json("revised.json", &spec);
    let output = fixture.run(&[
        "--grade-probe",
        "revised.json",
        "capture-v2.json",
        "answer.json",
        "--report",
        "regraded.json",
    ]);
    assert_eq!(output.status.code(), Some(1));
    let grade = fixture.read("regraded.json");
    assert_eq!(grade["success"], false);
    assert_eq!(grade["response_valid"], true);
    assert_eq!(
        grade["request_revision"],
        original_grade["request_revision"]
    );
    assert_ne!(
        grade["definition_revision"],
        original_grade["definition_revision"]
    );
    assert_eq!(grade["checks"][1]["reason"], "answer_mismatch");
    assert!(!grade.to_string().contains("EDITED_EXPECTED_SHOP_MARKER"));
    assert_eq!(
        std::fs::read(fixture.directory.join("capture-v2.json")).unwrap(),
        original_capture
    );
}

#[test]
fn probe_cli_rejects_mode_conflicts_and_never_overwrites_an_existing_report() {
    let fixture = Fixture::new();
    let conflicts: &[&[&str]] = &[
        &[
            "--grade-probe",
            "probe.json",
            "capture-v2.json",
            "answer.json",
            "--headless",
        ],
        &[
            "--grade-probe",
            "probe.json",
            "capture-v2.json",
            "answer.json",
            "--check-probe",
            "probe.json",
        ],
        &[
            "--check-probe",
            "probe.json",
            "--check-prompts",
            "--prompts",
            "missing.json",
        ],
        &[
            "--grade-probe",
            "probe.json",
            "capture-v2.json",
            "answer.json",
            "--junit",
            "grade.xml",
        ],
        &[
            "--grade-probe",
            "probe.json",
            "capture-v2.json",
            "answer.json",
            "--include-content",
        ],
        &["--grade-probe", "probe.json", "capture-v2.json"],
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
    fixture.write("existing.json", "KEEP EXACTLY");
    let output = fixture.run(&[
        "--grade-probe",
        "probe.json",
        "capture-v2.json",
        "answer.json",
        "--report",
        "existing.json",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("already exists"));
    assert_eq!(
        std::fs::read_to_string(fixture.directory.join("existing.json")).unwrap(),
        "KEEP EXACTLY"
    );
}

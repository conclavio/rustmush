//! CLI inspection works without dotenv, credentials, HTTP, SSH, or a game fixture.
use std::{
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU32, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use mush_bot::{composition::Settings, experiment::revision, model::Capture};
use serde_json::{Value, json};

const SSH_SECRET: &str = "MUSH_BOT_OFFLINE_TEST_MISSING_SSH_SECRET";
const MODEL_SECRET: &str = "MUSH_BOT_OFFLINE_TEST_MISSING_MODEL_SECRET";

#[test]
fn conversation_validation_is_offline_strict_and_does_not_create_runtime_artifacts() {
    let fixture = Fixture::new();
    fixture.json(
        "conversation.json",
        &json!({"version":1,"mode":"spectator"}),
    );
    let entries = fixture.entries();
    let output = fixture.success(&[
        "--config",
        "missing.toml",
        "--check-conversation",
        "conversation.json",
    ]);
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["mode"], "spectator");
    assert_eq!(entries, fixture.entries());
    fixture.json(
        "conversation.json",
        &json!({"version":1,"max_utterances":0}),
    );
    assert!(
        !fixture
            .run(&["--check-conversation", "conversation.json"])
            .status
            .success()
    );
    fixture.json(
        "conversation.json",
        &json!({"version":1,"unknown_setting":true}),
    );
    assert!(
        !fixture
            .run(&["--check-conversation", "conversation.json"])
            .status
            .success()
    );
}

struct Fixture {
    directory: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let directory = std::env::temp_dir().join(format!(
            "mush-bot-offline-{}-{}-{}",
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
            "bot.toml",
            &format!(
                "model_profiles = 'models.json'\n[ssh]\nhost = '127.0.0.1'\nport = 1\nusername = 'offline'\npassword_env = '{SSH_SECRET}'\ntimeout_secs = 1\n"
            ),
        );
        let model = |name| {
            json!({"name":name,"endpoint":"http://127.0.0.1:1/v1/chat/completions",
            "api_key_env":MODEL_SECRET,"timeout_secs":1})
        };
        fixture.json(
            "models.json",
            &json!({"version":1,"default":"a","profiles":{
            "a":model("offline/a"),"b":model("offline/b")}}),
        );
        fixture.json("plain.json", &json!({"version":1,"name":"plain"}));
        fixture.json(
            "wary.json",
            &json!({"version":1,"name":"wary","supplement":"Ask for evidence."}),
        );
        fixture.json("cast.json", &json!({"version":1,"name":"Offline pack plan",
            "actors":[
                {"name":"scout","config":"bot.toml","login":"PRIVATE_SCOUT_LOGIN","expected_status":"paused"},
                {"name":"medic","config":"bot.toml","expected_status":"paused"}
            ],
            "groups":[{"id":"pack","name":"Lantern Pack","kind":"garou pack","mission":"Search the forest",
                "shared_knowledge":["OFFLINE_SHARED_STARTING_KNOWLEDGE"],
                "members":[
                    {"actor":"medic","role":"healer","briefing":["OFFLINE_PRIVATE_MEDIC_BRIEFING"]},
                    {"actor":"scout","role":"pathfinder","briefing":["OFFLINE_PRIVATE_SCOUT_BRIEFING"]}
                ]}],
            "steps":[
                {"name":"Enter together","group":"pack","dispatch":"serial","control":"/send north",
                    "expect":[{"kind":"command-result","pointer":"/command","equals":"north"}],
                    "reject":[{"kind":"blocked"}]},
                {"name":"Pause observers","actors":["scout","medic"],"control":"/pause"}
            ],
            "matrix":{"actor":"scout","models":["a","b"],"prompts":["plain.json","wary.json"],"repeats":2},
            "server":{"program":"mush-bot-offline-test-program-that-does-not-exist","ready_output":"READY","timeout_ms":100}
        }));
        fixture.json(
            "left.json",
            &json!(capture("OFFLINE_OLD_MEMORY", "offline/a")),
        );
        fixture.json(
            "right.json",
            &json!(capture("OFFLINE_NEW_MEMORY", "offline/b")),
        );
        fixture
    }

    fn write(&self, name: &str, contents: &str) {
        std::fs::write(self.directory.join(name), contents).unwrap();
    }

    fn json(&self, name: &str, value: &Value) {
        self.write(name, &serde_json::to_string_pretty(value).unwrap());
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mush-bot"))
            .args(args)
            .current_dir(&self.directory)
            .env_remove(SSH_SECRET)
            .env_remove(MODEL_SECRET)
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

    fn read(&self, name: &str) -> Value {
        serde_json::from_slice(&std::fs::read(self.directory.join(name)).unwrap()).unwrap()
    }

    fn entries(&self) -> Vec<PathBuf> {
        let mut paths = std::fs::read_dir(&self.directory)
            .unwrap()
            .map(|p| p.unwrap().path())
            .collect::<Vec<_>>();
        paths.sort();
        paths
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Cleanup is restricted to this fixture's verified direct child of the
        // system temporary directory; never remove a computed unchecked path.
        if let (Ok(directory), Ok(temporary)) = (
            self.directory.canonicalize(),
            std::env::temp_dir().canonicalize(),
        ) && directory.parent() == Some(temporary.as_path())
            && directory
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("mush-bot-offline-"))
        {
            let _ = std::fs::remove_dir_all(directory);
        }
    }
}

fn capture(memory: &str, model: &str) -> Capture {
    let (messages, composition) = Settings::default()
        .prepare(
            "Offline test system instructions".into(),
            &json!({"runner_goal":"Inspect the cast","memory":memory}),
            None,
        )
        .unwrap();
    let payload = json!({"model":model,"max_tokens":128,"stream":false,"messages":messages});
    let capture = Capture {
        version: 2,
        question: false,
        prompt_revision: revision(b"Offline test system instructions"),
        request_revision: revision(&serde_json::to_vec(&payload).unwrap()),
        payload,
        composition_revision: Some(revision(&serde_json::to_vec(&composition).unwrap())),
        composition: Some(composition),
    };
    capture.validate().unwrap();
    capture
}

#[test]
fn comparison_cli_is_offline_and_content_is_opt_in() {
    let fixture = Fixture::new();
    let entries = fixture.entries();
    let output = fixture.success(&[
        "--compare-captures",
        "left.json",
        "right.json",
        "--config",
        "does-not-exist.toml",
    ]);
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["version"], 1);
    assert_eq!(report["request_equal"], false);
    assert_eq!(report["messages_equal"], false);
    assert_eq!(report["settings_equal"], false);
    assert_eq!(report["includes_content"], false);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("OFFLINE_OLD_MEMORY"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("OFFLINE_NEW_MEMORY"));
    assert_eq!(fixture.entries(), entries);

    fixture.success(&[
        "--compare-captures",
        "left.json",
        "right.json",
        "--include-content",
        "--report",
        "comparison.json",
    ]);
    let detailed = fixture.read("comparison.json");
    assert_eq!(detailed["includes_content"], true);
    assert!(detailed.to_string().contains("OFFLINE_OLD_MEMORY"));
    assert!(detailed.to_string().contains("OFFLINE_NEW_MEMORY"));
    assert_eq!(detailed["messages"][0]["before"]["component_id"], "memory");
    let index = detailed["messages"][0]["index"].as_u64().unwrap() as usize;
    assert_eq!(
        detailed["messages"][0]["before"]["message"],
        fixture.read("left.json")["payload"]["messages"][index]
    );
}

#[test]
fn harness_plan_expands_matrix_and_scoped_serial_phases_without_private_briefings() {
    let fixture = Fixture::new();
    let entries = fixture.entries();
    let output = fixture.success(&["--harness", "cast.json", "--plan-harness"]);
    let plan: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(plan["version"], 1);
    assert_eq!(plan["run_count"], 8);
    assert_eq!(plan["actor_sessions"], 16);
    assert_eq!(plan["owns_server"], true);
    assert_eq!(plan["phases"][0]["targets"], json!(["medic", "scout"]));
    assert_eq!(plan["phases"][0]["dispatch"], "serial");
    assert_eq!(plan["phases"][0]["requires_acknowledgments"], true);
    assert_eq!(
        plan["phases"][0]["assertions"]["eventually"][0]["actor"],
        "medic"
    );
    assert_eq!(
        plan["phases"][0]["assertions"]["eventually"][1]["actor"],
        "scout"
    );
    assert_eq!(
        plan["phases"][0]["assertions"]["forbidden"][0]["actor"],
        "medic"
    );
    assert_eq!(
        plan["phases"][0]["assertions"]["forbidden"][1]["actor"],
        "scout"
    );
    assert_eq!(plan["phases"][1]["targets"], json!(["scout", "medic"]));
    assert_eq!(plan["phases"][1]["dispatch"], "parallel");
    let variants: Vec<_> = plan["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| {
            let scout = &run["actors"][0];
            (
                scout["model"].as_str().unwrap().to_owned(),
                Path::new(scout["prompts"].as_str().unwrap())
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned(),
                run["repetition"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        variants,
        vec![
            ("a".into(), "plain.json".into(), 1),
            ("a".into(), "plain.json".into(), 2),
            ("a".into(), "wary.json".into(), 1),
            ("a".into(), "wary.json".into(), 2),
            ("b".into(), "plain.json".into(), 1),
            ("b".into(), "plain.json".into(), 2),
            ("b".into(), "wary.json".into(), 1),
            ("b".into(), "wary.json".into(), 2),
        ]
    );
    let text = String::from_utf8_lossy(&output.stdout);
    for private in [
        "OFFLINE_PRIVATE",
        "PRIVATE_SCOUT_LOGIN",
        SSH_SECRET,
        MODEL_SECRET,
        "OFFLINE_SHARED_STARTING_KNOWLEDGE",
    ] {
        assert!(!text.contains(private));
    }
    assert_eq!(fixture.entries(), entries);
    fixture.success(&[
        "--harness",
        "cast.json",
        "--plan-harness",
        "--report",
        "plan.json",
    ]);
    assert_eq!(fixture.read("plan.json"), plan);

    // Existing validation remains a normalized manifest, not an expanded plan.
    let output = fixture.success(&["--harness", "cast.json", "--check-harness"]);
    let normalized: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(normalized.get("runs").is_none());
    assert_eq!(normalized["matrix"]["repeats"], 2);
    assert_eq!(
        normalized["groups"][0]["members"][0]["briefing"][0],
        "OFFLINE_PRIVATE_MEDIC_BRIEFING"
    );
}

#[test]
fn offline_modes_reject_conflicts_and_preserve_existing_artifacts() {
    let fixture = Fixture::new();
    let conflicts: &[&[&str]] = &[
        &[
            "--compare-captures",
            "left.json",
            "right.json",
            "--harness",
            "cast.json",
            "--plan-harness",
        ],
        &["--harness", "cast.json", "--plan-harness", "--goal", "Act"],
        &["--compare-captures", "left.json", "right.json", "--tui"],
        &["--include-content"],
        &["--plan-harness"],
        &[
            "--harness",
            "cast.json",
            "--plan-harness",
            "--junit",
            "report.xml",
        ],
        &[
            "--compare-captures",
            "left.json",
            "right.json",
            "--model",
            "a",
        ],
        &[
            "--harness",
            "cast.json",
            "--check-harness",
            "--plan-harness",
        ],
        &["--compare-captures", "left.json"],
    ];
    let entries = fixture.entries();
    for args in conflicts {
        let output = fixture.run(args);
        assert!(
            !output.status.success(),
            "accepted conflicting arguments {args:?}"
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains("could not load .env"));
    }
    assert_eq!(fixture.entries(), entries);
    fixture.write("existing.json", "KEEP THIS EXACT CONTENT");
    for args in [
        vec![
            "--compare-captures",
            "left.json",
            "right.json",
            "--report",
            "existing.json",
        ],
        vec![
            "--harness",
            "cast.json",
            "--plan-harness",
            "--report",
            "existing.json",
        ],
    ] {
        let output = fixture.run(&args);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("already exists"));
        assert_eq!(
            std::fs::read_to_string(fixture.directory.join("existing.json")).unwrap(),
            "KEEP THIS EXACT CONTENT"
        );
    }
    let output = fixture.run(&[
        "--harness",
        "cast.json",
        "--plan-harness",
        "--report",
        "wrong.txt",
    ]);
    assert!(!output.status.success());
    assert!(!fixture.directory.join("wrong.txt").exists());
}

#[test]
fn phase_debug_settings_validate_offline_and_require_an_interactive_cast() {
    let fixture = Fixture::new();
    fixture.json(
        "gates.json",
        &json!({"version":1,"start_held":true,"before_phases":[2],"max_hold_ms":5000}),
    );
    let checked = fixture.run(&["--check-harness-debug", "gates.json"]);
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let settings: Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(settings["before_phases"], json!([2]));
    for args in [
        vec!["--harness-debug", "gates.json"],
        vec![
            "--harness",
            "cast.json",
            "--harness-debug",
            "gates.json",
            "--output",
            "must-not-create",
        ],
        vec![
            "--harness",
            "cast.json",
            "--harness-debug",
            "gates.json",
            "--tui",
            "--plan-harness",
        ],
        vec!["--check-harness-debug", "gates.json", "--goal", "Act"],
    ] {
        let result = fixture.run(&args);
        assert!(!result.status.success(), "accepted {args:?}");
        assert!(!String::from_utf8_lossy(&result.stderr).contains("could not load .env"));
    }
    assert!(!fixture.directory.join("must-not-create").exists());
}

#[test]
fn tampered_capture_cannot_produce_an_offline_comparison_report() {
    let fixture = Fixture::new();
    let mut altered = fixture.read("right.json");
    altered["payload"]["model"] = json!("unsealed-change");
    fixture.json("right.json", &altered);
    let output = fixture.run(&[
        "--compare-captures",
        "left.json",
        "right.json",
        "--report",
        "tampered.json",
    ]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("invalid or modified decision capture")
    );
    assert!(!fixture.directory.join("tampered.json").exists());
}

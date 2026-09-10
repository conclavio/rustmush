//! The debugger runs in an owned helper process so bot children inherit fixture
//! credentials without changing the test runner's process-global environment.
use super::*;
use mush_bot::cast_debug::{Action, Command as DebugCommand, Controller, Settings, Snapshot};
use tokio::sync::{mpsc, watch};

const PREFIX: &str = "CAST_DEBUG ";

fn emit(kind: &str, data: impl serde::Serialize) {
    println!("{PREFIX}{}", json!({"kind":kind,"data":data}));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cast_debug_helper() {
    if std::env::var("MUSH_BOT_CAST_DEBUG_HELPER").as_deref() != Ok("1") {
        return;
    }
    let manifest = mush_bot::harness::Manifest::load(Path::new(
        &std::env::var("MUSH_BOT_CAST_DEBUG_MANIFEST").unwrap(),
    ))
    .unwrap();
    let directory = PathBuf::from(std::env::var("MUSH_BOT_CAST_DEBUG_OUTPUT").unwrap());
    let settings = Settings::load(Path::new(
        &std::env::var("MUSH_BOT_CAST_DEBUG_SETTINGS").unwrap(),
    ))
    .unwrap();
    let (commands, incoming) = mpsc::channel(32);
    let (state, mut snapshots) = watch::channel(Snapshot::default());
    let (cancel, cancelled) = watch::channel(false);
    let input_cancel = cancel.clone();
    let (events, mut observations) = mpsc::channel::<Value>(256);
    let input = tokio::spawn(async move {
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let value: Value = serde_json::from_str(&line).unwrap();
            if value["cancel"] == true {
                let _ = input_cancel.send(true);
                continue;
            }
            let action = match value["action"].as_str().unwrap() {
                "hold" => Action::Hold,
                "continue" => Action::Continue,
                "step" => Action::Step,
                _ => panic!("invalid helper control"),
            };
            if commands
                .send(DebugCommand {
                    run: value["run"].as_u64().unwrap() as usize,
                    revision: value["revision"].as_u64().unwrap(),
                    action,
                })
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let states = tokio::spawn(async move {
        while snapshots.changed().await.is_ok() {
            emit("state", snapshots.borrow_and_update().clone());
        }
    });
    let observations = tokio::spawn(async move {
        while let Some(value) = observations.recv().await {
            let event = if value["event"].is_object() {
                &value["event"]
            } else {
                &value
            };
            if [
                "ready",
                "send",
                "control",
                "command-result",
                "model-request",
                "cast-debug",
            ]
            .iter()
            .any(|kind| event["kind"] == *kind)
            {
                let data = if event["kind"] == "ready" {
                    Value::Null
                } else {
                    event["data"].clone()
                };
                emit(
                    "event",
                    json!({"actor":value["actor"],"kind":event["kind"],"data":data}),
                );
            }
        }
    });
    let controller = Controller {
        settings,
        commands: incoming,
        state,
        bot_program: Some(env!("CARGO_BIN_EXE_mush-bot").into()),
    };
    let result = mush_bot::harness::run_debugged(
        manifest,
        &directory,
        Some(mush_bot::harness::Observer {
            events,
            cancel: cancelled,
        }),
        controller,
    )
    .await;
    emit(
        "result",
        match result {
            Ok(result) => result,
            Err(error) => json!({"helper_error":error.to_string()}),
        },
    );
    drop(cancel);
    input.abort();
    states.abort();
    observations.abort();
}

struct DebuggedCast {
    child: tokio::process::Child,
    input: Option<tokio::process::ChildStdin>,
    lines: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    output: PathBuf,
    seen: Vec<Value>,
}

impl DebuggedCast {
    fn spawn(fixture: &Fixture, manifest: Value, settings: Value, name: &str) -> Self {
        let manifest_path = fixture.dir.join(format!("{name}-cast.json"));
        let settings_path = fixture.dir.join(format!("{name}-debug.json"));
        let output = fixture.dir.join(format!("{name}-results"));
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        std::fs::write(&settings_path, serde_json::to_vec(&settings).unwrap()).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("cast_debug::cast_debug_helper")
            .arg("--exact")
            .arg("--nocapture")
            .current_dir(&fixture.dir)
            .env("MUSH_BOT_CAST_DEBUG_HELPER", "1")
            .env("MUSH_BOT_CAST_DEBUG_MANIFEST", &manifest_path)
            .env("MUSH_BOT_CAST_DEBUG_OUTPUT", &output)
            .env("MUSH_BOT_CAST_DEBUG_SETTINGS", settings_path)
            .env("MUSH_BOT_E2E_PASSWORD", PASSWORD)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        Self {
            input: child.stdin.take(),
            lines: BufReader::new(child.stdout.take().unwrap()).lines(),
            child,
            output,
            seen: vec![],
        }
    }

    async fn until(&mut self, condition: impl Fn(&Value) -> bool) -> Value {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let line = self
                    .lines
                    .next_line()
                    .await
                    .unwrap()
                    .expect("debug helper exited before expected event");
                let Some(value) = line.strip_prefix(PREFIX) else {
                    continue;
                };
                let value: Value = serde_json::from_str(value).unwrap();
                self.seen.push(value.clone());
                if condition(&value) {
                    return value;
                }
                assert_ne!(
                    value["kind"], "result",
                    "helper finished too soon: {}",
                    value["data"]
                );
            }
        })
        .await
        .expect("debugger event deadline")
    }

    async fn held(&mut self, run: u64, next_phase: u64) -> Value {
        self.until(|event| {
            event["kind"] == "state"
                && event["data"]["mode"] == "held"
                && event["data"]["run"] == run
                && event["data"]["next_phase"] == next_phase
        })
        .await["data"]
            .clone()
    }

    async fn send(&mut self, value: Value) {
        self.input
            .as_mut()
            .unwrap()
            .write_all(format!("{value}\n").as_bytes())
            .await
            .unwrap();
    }

    async fn action(&mut self, snapshot: &Value, action: &str) {
        self.send(json!({"run":snapshot["run"],"revision":snapshot["revision"],"action":action}))
            .await;
    }

    fn joined(&self, run: usize) -> Vec<Value> {
        std::fs::read_to_string(self.output.join(format!("run-{run:04}/joined.jsonl")))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    async fn finish(mut self) -> Value {
        let result = self.until(|event| event["kind"] == "result").await["data"].clone();
        // Tokio's stdin reader may still own a blocking OS read. EOF lets the
        // helper runtime finish after it has already produced the run report.
        self.input.take();
        assert!(
            tokio::time::timeout(Duration::from_secs(5), self.child.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
        assert!(result.get("helper_error").is_none(), "{result}");
        result
    }
}

fn held_settings(max_hold_ms: u64) -> Value {
    json!({"version":1,"start_held":true,"before_phases":[],"max_hold_ms":max_hold_ms})
}

fn simple_cast(config: &Path, timeout_ms: u64) -> Value {
    json!({"name":"Held single actor","timeout_ms":timeout_ms,
        "actors":[{"name":"scout","config":config,"expected_status":"paused"}],
        "steps":[{"actor":"scout","control":"/send look","expect":[
            {"kind":"command-result","pointer":"/command","equals":"look","text_pointer":"/output","contains":"Market Square"}]}]})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cast_step_holds_after_one_serial_group_phase_and_rejects_stale_controls_across_runs() {
    let fixture = Fixture::boot_world(false, None, true).await;
    let model = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"choices":[{"message":{"content":"PHASE_ACK"}}]})),
        )
        .expect(4)
        .mount(&model)
        .await;
    let config = fixture.config(Some(&model), 10);
    let manifest = json!({"name":"Step a whole party phase","timeout_ms":10000,
        "actors":[
            {"name":"scout","config":config,"start":"/note ACTOR_START_SENTINEL","expected_status":"paused"},
            {"name":"medic","config":config,"login":"marco+Marco","expected_status":"paused"}
        ],
        "groups":[{"id":"party","name":"Lantern Company","kind":"party","mission":"Compare observations",
            "members":[{"actor":"scout","role":"pathfinder"},{"actor":"medic","role":"healer"}]}],
        "steps":[
            {"name":"Question each member","group":"party","dispatch":"serial","control":"/ask PHASE_MODEL_SENTINEL","expect":[{"kind":"answer","contains":"PHASE_ACK"}]},
            {"name":"Look together","group":"party","dispatch":"serial","control":"/send look","expect":[{"kind":"command-result","pointer":"/command","equals":"look","text_pointer":"/output","contains":"Market Square"}]}
        ],"matrix":{"actor":"scout","repeats":2}});
    let mut cast = DebuggedCast::spawn(&fixture, manifest, held_settings(15000), "step");
    let first = cast.held(1, 1).await;
    assert!(model.received_requests().await.unwrap().is_empty());
    let joined = cast.joined(1);
    assert!(!joined.iter().any(|e| e["event"]["kind"] == "phase-start"));
    assert!(!joined.iter().any(|e| {
        e["event"]["kind"] == "control"
            && e["event"]["data"]
                .as_str()
                .is_some_and(|s| s.contains("ACTOR_START_SENTINEL"))
    }));
    cast.action(&first, "step").await;
    cast.action(&first, "step").await; // A queued duplicate cannot release the next boundary.
    let second = cast.held(1, 2).await;
    assert_ne!(first["revision"], second["revision"]);
    assert_eq!(model.received_requests().await.unwrap().len(), 2);
    let joined = cast.joined(1);
    assert!(
        joined.iter().any(|e| e["event"]["kind"] == "control"
            && e["event"]["data"] == "/note ACTOR_START_SENTINEL")
    );
    assert!(
        !joined
            .iter()
            .any(|e| e["event"]["kind"] == "phase-start" && e["event"]["data"]["id"] == "phase-2")
    );
    let scout_answer = joined
        .iter()
        .position(|e| e["actor"] == "scout" && e["event"]["kind"] == "answer")
        .unwrap();
    let medic_start = joined
        .iter()
        .position(|e| {
            e["actor"] == "medic"
                && e["event"]["kind"] == "control-started"
                && e["event"]["data"]["id"] == "phase-1"
        })
        .unwrap();
    assert!(
        scout_answer < medic_start,
        "one Step must honor each serial member's outcome"
    );
    cast.action(&first, "step").await;
    cast.until(|e| {
        e["kind"] == "event"
            && e["data"]["kind"] == "cast-debug"
            && e["data"]["data"]["reason"] == "stale_revision"
            && e["data"]["data"]["state"]["mode"] == "held"
    })
    .await;
    cast.action(&second, "continue").await;
    cast.action(&second, "continue").await;
    let next_run = cast.held(2, 1).await;
    assert_eq!(model.received_requests().await.unwrap().len(), 2);
    cast.action(&second, "step").await;
    cast.until(|e| {
        e["kind"] == "event"
            && e["data"]["kind"] == "cast-debug"
            && e["data"]["data"]["reason"] == "stale_run"
            && e["data"]["data"]["state"]["run"] == 2
    })
    .await;
    cast.action(&next_run, "step").await;
    let last_phase = cast.held(2, 2).await;
    assert_eq!(model.received_requests().await.unwrap().len(), 4);
    cast.action(&last_phase, "continue").await;
    let summary = cast.finish().await;
    assert_eq!(summary["success"], true);
    assert_eq!(summary["runs"].as_array().unwrap().len(), 2);
    assert!(
        summary["runs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|run| run["result"]["steps"].as_array().unwrap().len() == 2)
    );
    assert!(
        summary["runs"][0]["result"]["debug"]["rejected_commands"]
            .as_u64()
            .unwrap()
            >= 2
    );
    assert!(
        summary["runs"][1]["result"]["debug"]["rejected_commands"]
            .as_u64()
            .unwrap()
            >= 1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn held_cast_still_observes_world_speech_and_enforces_global_forbidden_assertions() {
    let fixture = Fixture::boot_world(false, None, true).await;
    let mut speaker = PlayerConsole::speaker(&fixture).await;
    let config = fixture.config(None, 10);
    let mut manifest = simple_cast(&config, 10000);
    manifest["assertions"] = json!({"forbidden":[{"actor":"scout","kind":"observation","contains":"HELD_WORLD_FORBIDDEN"}]});
    let mut cast = DebuggedCast::spawn(&fixture, manifest, held_settings(15000), "forbidden");
    cast.held(1, 1).await;
    let directory = cast.output.clone();
    speaker.say("HELD_WORLD_FORBIDDEN").await;
    let summary = cast.finish().await;
    assert_eq!(summary["success"], false);
    let run = &summary["runs"][0]["result"];
    assert_eq!(run["assertions"]["success"], false);
    assert_eq!(
        run["assertions"]["forbidden_failure"]["evidence"]["actor"],
        "scout"
    );
    assert!(run["steps"].as_array().unwrap().is_empty());
    let joined = std::fs::read_to_string(directory.join("run-0001/joined.jsonl")).unwrap();
    assert!(joined.contains("HELD_WORLD_FORBIDDEN"));
    assert!(directory.join("run-0001/scout/report.json").exists());
    speaker.quit().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controller_disconnect_and_explicit_cancel_persist_owned_bot_reports() {
    let fixture = Fixture::boot().await;
    let config = fixture.config(None, 10);
    for (name, explicit_cancel) in [("controller-eof", false), ("cancel", true)] {
        let mut cast = DebuggedCast::spawn(
            &fixture,
            simple_cast(&config, 10000),
            held_settings(15000),
            name,
        );
        cast.held(1, 1).await;
        let directory = cast.output.clone();
        if explicit_cancel {
            cast.send(json!({"cancel":true})).await;
        } else {
            cast.input.take();
        }
        let summary = cast.finish().await;
        assert_eq!(summary["success"], false);
        let run = &summary["runs"][0]["result"];
        assert!(run["steps"].as_array().unwrap().is_empty());
        assert_eq!(run["actors"]["scout"]["exit_code"], 0);
        assert_eq!(
            run["actors"]["scout"]["report"]["state"]["status"],
            "paused"
        );
        assert!(directory.join("run-0001/scout/report.json").exists());
        assert!(directory.join("run-0001/joined.jsonl").exists());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hold_deadline_is_bounded_and_held_time_is_excluded_from_active_run_budget() {
    let fixture = Fixture::boot().await;
    let config = fixture.config(None, 10);
    let mut cast = DebuggedCast::spawn(
        &fixture,
        simple_cast(&config, 3000),
        held_settings(6000),
        "active-budget",
    );
    cast.held(1, 1).await;
    let held = cast
        .until(|e| {
            e["kind"] == "state"
                && e["data"]["mode"] == "held"
                && e["data"]["held_ms"].as_u64().is_some_and(|ms| ms >= 3200)
        })
        .await["data"]
        .clone();
    cast.action(&held, "continue").await;
    let summary = cast.finish().await;
    assert_eq!(summary["success"], true, "{summary}");
    let run = &summary["runs"][0]["result"];
    assert!(run["debug"]["held_ms"].as_u64().unwrap() >= 3200);
    assert!(run["elapsed_ms"].as_u64().unwrap() >= 3200);
    assert!(run["active_elapsed_ms"].as_u64().unwrap() < 3000);
    let mut cast = DebuggedCast::spawn(
        &fixture,
        simple_cast(&config, 5000),
        held_settings(150),
        "hold-deadline",
    );
    cast.held(1, 1).await;
    let summary = cast.finish().await;
    assert_eq!(summary["success"], false);
    let run = &summary["runs"][0]["result"];
    assert!(
        run["error"]
            .as_str()
            .unwrap()
            .to_ascii_lowercase()
            .contains("hold")
    );
    assert!(run["steps"].as_array().unwrap().is_empty());
    assert!(run["debug"]["held_ms"].as_u64().unwrap() >= 150);
    assert_eq!(run["actors"]["scout"]["exit_code"], 0);
}

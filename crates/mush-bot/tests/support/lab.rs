use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn organized_cast_keeps_briefings_private_coordinates_travel_and_totals_session_costs() {
    let fixture = Fixture::boot_world(false, None, true).await;
    let model = MockServer::start().await;
    Mock::given(method("POST")).respond_with(|request: &Request| {
        let payload: Value = request.body_json().unwrap();
        let parts: Vec<Value> = payload["messages"].as_array().unwrap().iter()
            .filter_map(|m| serde_json::from_str(m["content"].as_str().unwrap()).ok()).collect();
        let affiliations = &parts.iter().find(|p| p["component"] == "group_affiliations").unwrap()["value"];
        let body = payload.to_string();
        let role = if affiliations["actor"] == "scout" {
            assert!(body.contains("SUSPECT_PATRON"));
            assert!(!body.contains("PRIVATE_DEBT"));
            "pathfinder"
        } else {
            assert!(body.contains("PRIVATE_DEBT"));
            assert!(!body.contains("SUSPECT_PATRON"));
            "medic"
        };
        assert!(!body.contains("HIDDEN_LORE"));
        ResponseTemplate::new(200).set_body_json(json!({"model":"test/provider","choices":[{"message":{"content":format!("Ready: {role}.")}}],
            "usage":{"cost":0.125,"prompt_tokens":20,"completion_tokens":5}}))
    }).expect(2).mount(&model).await;
    let config = fixture.config(Some(&model), 10);
    std::fs::write(
        &config,
        std::fs::read_to_string(&config).unwrap().replace(
            "name = \"fixture-model\"",
            "name = \"fixture-model\"\ncost_currency = \"USD\"",
        ),
    )
    .unwrap();
    let inherited_budget = write_json(&fixture, "inherited-budget.json", json!({"max_requests":1}));
    let scout_budget = write_json(&fixture, "scout-budget.json", json!({"max_requests":2}));
    std::fs::write(
        &config,
        format!(
            "budget = '{}'\n{}",
            inherited_budget.display(),
            std::fs::read_to_string(&config).unwrap()
        ),
    )
    .unwrap();
    let prompts = write_json(
        &fixture,
        "party-prompts.json",
        json!({"name":"party","composition":{"lore":[
            {"id":"secret","text":"HIDDEN_LORE","keywords":[]}
        ]}}),
    );
    let manifest = write_json(
        &fixture,
        "party.json",
        json!({"name":"Party expedition","timeout_ms":15000,
        "actors":[
            {"name":"scout","config":config,"prompts":prompts,"budget":scout_budget,"expected_status":"paused"},
            {"name":"medic","config":config,"prompts":prompts,"login":"marco+Marco","expected_status":"paused"}
        ],
        "groups":[
            {"id":"party","name":"Lantern Company","kind":"adventuring party","mission":"Explore the flower shop and regroup.",
                "shared_knowledge":["Our rendezvous is Market Square."],"members":[
                    {"actor":"scout","role":"pathfinder","briefing":["SUSPECT_PATRON"]},
                    {"actor":"medic","role":"medic","briefing":["PRIVATE_DEBT"]}]},
            {"id":"guild","name":"Cartographers","kind":"guild","mission":"Draw honest maps.","members":[{"actor":"scout","role":"surveyor"}]}
        ],
        "steps":[
            {"name":"Briefing","group":"party","control":"/ask What is your role?","expect":[{"kind":"answer","contains":"Ready:"}]},
            {"name":"Explore together","group":"party","dispatch":"serial","control":"/send east","expect":[{"kind":"command-result","text_pointer":"/output","contains":"Flower Shop #"}]},
            {"name":"Regroup","group":"party","dispatch":"serial","control":"/send west","expect":[{"kind":"command-result","text_pointer":"/output","contains":"Market Square #"}]},
            {"name":"Hold","actors":["scout","medic"],"control":"/pause"}
        ]}),
    );
    let directory = fixture.dir.join("party-results");
    let result = output(
        fixture
            .command(&config)
            .arg("--harness")
            .arg(&manifest)
            .arg("--output")
            .arg(&directory),
    )
    .await;
    let summary: Value =
        serde_json::from_slice(&std::fs::read(directory.join("summary.json")).unwrap()).unwrap();
    assert!(
        result.status.success(),
        "{}\n{}\n{}",
        summary["runs"][0]["result"]["error"],
        summary["runs"][0]["result"]["pending_step"],
        json!(
            ["scout", "medic"]
                .into_iter()
                .map(|actor| {
                    let text = std::fs::read_to_string(
                        directory.join(format!("run-0001/{actor}/trace.jsonl")),
                    )
                    .unwrap();
                    let events: Vec<Value> = text
                        .lines()
                        .map(|s| serde_json::from_str::<Value>(s).unwrap())
                        .filter(|e| {
                            [
                                "control-started",
                                "control-applied",
                                "command-result",
                                "send",
                            ]
                            .contains(&e["kind"].as_str().unwrap_or(""))
                        })
                        .collect();
                    json!({"actor":actor,"events":events})
                })
                .collect::<Vec<_>>()
        )
    );
    let run = &summary["runs"][0]["result"];
    assert_eq!(run["steps"].as_array().unwrap().len(), 4);
    for step in run["steps"].as_array().unwrap() {
        assert_eq!(step["acknowledged"].as_array().unwrap().len(), 2);
        assert!(step["waiting_for"].as_array().unwrap().is_empty());
    }
    assert_eq!(
        run["costs"]["cast"]["totals"]["provider_cost_by_currency"]["USD"],
        "0.25"
    );
    assert_eq!(
        run["costs"]["by_group"]["party"]["totals"]["provider_cost_by_currency"]["USD"],
        "0.25"
    );
    assert_eq!(
        run["costs"]["by_group"]["guild"]["totals"]["provider_cost_by_currency"]["USD"],
        "0.125"
    );
    assert_eq!(
        run["costs"]["cast"]["by_prompt_type"]["runner_question"]["requests"],
        2
    );
    for (actor, remaining) in [("scout", 1), ("medic", 0)] {
        let report: Value = serde_json::from_slice(
            &std::fs::read(directory.join(format!("run-0001/{actor}/report.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(report["budget"]["status"]["usage"]["requests"], 1);
        assert_eq!(report["budget"]["status"]["remaining_requests"], remaining);
    }
    let scout =
        std::fs::read_to_string(directory.join("run-0001/scout/affiliations.json")).unwrap();
    assert!(scout.contains("SUSPECT_PATRON"));
    assert!(!scout.contains("PRIVATE_DEBT"));
    let trace = std::fs::read_to_string(directory.join("run-0001/joined.jsonl")).unwrap();
    assert!(trace.contains("phase-complete"));
    let joined: Vec<Value> = trace
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    // Inspect exactly what the public SSH cast recorded, with no connections or
    // filesystem references resolved from inside the trace.
    let inspected = mush_bot::trace::load(&directory.join("run-0001/joined.jsonl")).unwrap();
    assert_eq!(inspected.events.len(), joined.len());
    let requests = inspected
        .matching(&mush_bot::trace::Query {
            actors: vec!["scout".into()],
            kinds: vec!["model-input".into()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(requests.len(), 1);
    let related = inspected.related(requests[0]);
    assert!(
        related
            .iter()
            .any(|i| inspected.events[*i].kind == "model-reply")
    );
    assert!(
        related
            .iter()
            .all(|i| inspected.events[*i].actor.as_deref() == Some("scout"))
    );
    for (phase, command) in [("phase-2", "east"), ("phase-3", "west")] {
        let scout_result = joined
            .iter()
            .position(|e| {
                e["actor"] == "scout"
                    && e["event"]["kind"] == "command-result"
                    && e["event"]["data"]["command"] == command
            })
            .unwrap();
        let medic_start = joined
            .iter()
            .position(|e| {
                e["actor"] == "medic"
                    && e["event"]["kind"] == "control-started"
                    && e["event"]["data"]["id"] == phase
            })
            .unwrap();
        assert!(
            scout_result < medic_start,
            "serial dispatch waits for the earlier member's outcome"
        );
    }
    let events: Vec<Value> = std::fs::read_to_string(directory.join("run-0001/scout/trace.jsonl"))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let standalone = mush_bot::trace::load(&directory.join("run-0001/scout/trace.jsonl")).unwrap();
    assert_eq!(standalone.events.len(), events.len());
    assert!(
        standalone
            .events
            .iter()
            .all(|e| e.actor.is_none() && e.elapsed_ms.is_none())
    );
    let capture: mush_bot::model::Capture = serde_json::from_value(
        events.iter().find(|e| e["kind"] == "model-input").unwrap()["data"].clone(),
    )
    .unwrap();
    capture.validate().unwrap();
    assert_eq!(capture.version, 2);
    assert!(
        capture
            .composition
            .as_ref()
            .unwrap()
            .blocks
            .iter()
            .any(|b| b.id == "secret" && b.reason == "knowledge_not_granted")
    );
    // Analyze the actual compact harness artifact without consulting config,
    // private per-actor files, dotenv, SSH or the model again.
    std::fs::write(fixture.dir.join(".env"), "DO_NOT_LOAD='unfinished\n").unwrap();
    let analysis_path = fixture.dir.join("party-analysis.json");
    let junit_path = fixture.dir.join("party-tests.xml");
    let result = output(
        fixture
            .command(&config)
            .arg("--analyze-harness")
            .arg(&directory)
            .arg("--report")
            .arg(&analysis_path)
            .arg("--junit")
            .arg(&junit_path),
    )
    .await;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let analysis: Value = serde_json::from_slice(&std::fs::read(&analysis_path).unwrap()).unwrap();
    assert_eq!(analysis["metrics"]["passed"], 1);
    assert_eq!(analysis["metrics"]["model_requests"]["known_total"], 2);
    assert_eq!(
        analysis["metrics"]["session_costs"]["totals"]["provider_cost_by_currency"]["USD"],
        "0.25"
    );
    let exported = std::fs::read_to_string(&analysis_path).unwrap();
    assert!(!exported.contains("SUSPECT_PATRON") && !exported.contains("PRIVATE_DEBT"));
    assert!(
        std::fs::read_to_string(&junit_path)
            .unwrap()
            .contains("<testcase")
    );
    assert_eq!(model.received_requests().await.unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn context_profiles_switch_private_recall_policy_without_rewriting_memory() {
    let fixture = Fixture::boot().await;
    let model = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|request: &Request| {
            let body: Value = request.body_json().unwrap();
            if body["messages"].as_array().unwrap().len() == 2 {
                reply(json!({"action":"recall","query":"rose silver","summary":"Inspect remembered evidence."}))
            } else {
                ResponseTemplate::new(200).set_body_json(json!({
                    "choices":[{"message":{"content":"I inspected the supplied evidence."}}]
                }))
            }
        })
        .expect(4)
        .mount(&model)
        .await;
    let config = fixture.config(Some(&model), 10);
    let report = fixture.dir.join("recall-policy-report.json");
    let trace = fixture.dir.join("recall-policy-trace.jsonl");
    let mut bot = PlayerConsole::spawn(
        fixture
            .command(&config)
            .arg("--paused")
            .arg("--json-events")
            .arg("--report")
            .arg(&report)
            .arg("--transcript")
            .arg(&trace),
    );
    bot.until(r#""kind":"ready""#).await;
    for note in [
        "The florist's rose cost 7 silver.",
        "The mayor enjoys chess.",
    ] {
        bot.send(&format!("/remember {note}")).await;
        bot.until(r#""kind":"remembered""#).await;
    }
    for policy in ["lexical", "recency"] {
        bot.send(&format!(
            "/context set {}",
            json!({"recall":{"policy":policy,"max_records":1,"recent_episodes":0}})
        ))
        .await;
        bot.until("Context settings changed").await;
        bot.send("/ask rose silver").await;
        bot.until(r#""kind":"answer""#).await;
    }
    bot.send(r#"/context set {"retrieval":false,"recall":{"policy":"recency"}}"#)
        .await;
    bot.until("Context settings changed").await;
    bot.send("/ask rose silver").await;
    bot.until(r#""kind":"answer""#).await;
    bot.send(r#"/context set {"recall":{"policy":"recency","max_records":1,"recent_episodes":0}}"#)
        .await;
    bot.until("Context settings changed").await;
    bot.send("/step").await;
    bot.until(r#""kind":"recall""#).await;
    bot.quit().await;
    let requests = model.received_requests().await.unwrap();
    let views: Vec<Value> = requests
        .iter()
        .map(|r| {
            let payload: Value = r.body_json().unwrap();
            serde_json::from_str(payload["messages"][1]["content"].as_str().unwrap()).unwrap()
        })
        .collect();
    assert_eq!(
        views[0]["remembered_experience"]["selection"]["settings"]["policy"],
        "lexical"
    );
    assert!(
        views[0]["remembered_experience"]["records"]
            .to_string()
            .contains("7 silver")
    );
    assert!(
        !views[0]["remembered_experience"]["records"]
            .to_string()
            .contains("chess")
    );
    assert_eq!(
        views[1]["remembered_experience"]["selection"]["settings"]["policy"],
        "recency"
    );
    assert!(
        views[1]["remembered_experience"]["records"]
            .to_string()
            .contains("chess")
    );
    assert!(
        !views[1]["remembered_experience"]["records"]
            .to_string()
            .contains("7 silver")
    );
    assert!(views[2]["remembered_experience"].is_null());
    let events: Vec<Value> = std::fs::read_to_string(trace)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let recall: Value = serde_json::from_str(
        events.iter().find(|e| e["kind"] == "recall").unwrap()["data"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        recall["selection"]["settings"],
        views[3]["context_settings"]["recall"]
    );
    assert!(
        recall["records"].to_string().contains("chess"),
        "model-triggered recall: {recall}"
    );
    assert!(!recall["records"].to_string().contains("7 silver"));
    let report: Value = serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
    let journal =
        std::fs::read_to_string(report["memory_store"]["file"].as_str().unwrap()).unwrap();
    assert!(journal.contains("7 silver") && journal.contains("chess"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prompt_preview_is_free_and_composed_capture_replays_the_exact_request_without_ssh() {
    let fixture = Fixture::boot().await;
    let model = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(reply(decision("done", None, "Inspected the square.")))
        .expect(2)
        .mount(&model)
        .await;
    let config = fixture.config(Some(&model), 10);
    let prompts = write_json(
        &fixture,
        "composed.json",
        json!({"name":"composed","composition":{}}),
    );
    let capture_path = fixture.dir.join("composed-capture.json");
    let trace = fixture.dir.join("preview.jsonl");
    let mut bot = PlayerConsole::spawn(
        fixture
            .command(&config)
            .arg("--paused")
            .arg("--prompts")
            .arg(&prompts)
            .arg("--transcript")
            .arg(&trace),
    );
    bot.until("Exits    east").await;
    bot.send(r#"/character-state set {"mood":"wary","remaining_turns":1}"#)
        .await;
    bot.until("[bot/character-state]").await;
    bot.send("/inspect preview").await;
    bot.until("[bot/prompt-inspection]").await;
    // A state request orders this assertion after the preview completed.
    bot.send("/state").await;
    bot.until("[bot/state]").await;
    assert!(model.received_requests().await.unwrap().is_empty());
    bot.send("/step model").await;
    bot.until("[bot/breakpoint]").await;
    bot.send(&format!("/capture {}", capture_path.display()))
        .await;
    bot.until("Exact model request saved").await;
    bot.send("/commit").await;
    bot.until("[bot/done]").await;
    let oversized = json!({"name":"budget-block","composition":{"max_input_bytes":4096,
        "components":[{"id":"required","text":"x".repeat(7000),"required":true}]}});
    bot.send(&format!("/prompt set {oversized}")).await;
    bot.until("Prompt profile changed").await;
    bot.send("/step").await;
    bot.until("Prompt preparation failed").await;
    bot.send("/state").await;
    bot.until("[bot/state]").await;
    assert_eq!(
        model.received_requests().await.unwrap().len(),
        1,
        "failed preparation leaves local controls available and makes no provider request"
    );
    bot.quit().await;
    let offline = fixture.dir.join("replay-offline.toml");
    std::fs::write(
        &offline,
        std::fs::read_to_string(&config)
            .unwrap()
            .replace(&format!("port = {}", fixture.addr.port()), "port = 1"),
    )
    .unwrap();
    let report_path = fixture.dir.join("composed-replay.json");
    let result = output(
        fixture
            .command(&offline)
            .arg("--replay")
            .arg(&capture_path)
            .arg("--report")
            .arg(&report_path),
    )
    .await;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let requests = model.received_requests().await.unwrap();
    assert_eq!(
        requests[0].body_json::<Value>().unwrap(),
        requests[1].body_json::<Value>().unwrap()
    );
    let report: Value = serde_json::from_slice(&std::fs::read(report_path).unwrap()).unwrap();
    assert_eq!(report["commands_sent"], 0);
    assert_eq!(report["capture"]["version"], 2);
    let events: Vec<Value> = std::fs::read_to_string(&trace)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let held = events
        .iter()
        .position(|e| e["kind"] == "staged-decision")
        .unwrap();
    let expired = events
        .iter()
        .position(|e| e["kind"] == "character-state-expired")
        .unwrap();
    assert!(
        held < expired,
        "previews and staged decisions do not consume a temporary condition"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn harness_accepts_clean_listener_shutdown_and_expected_headless_budget_exit() {
    let fixture = Fixture::boot().await;
    let config = fixture.config(None, 1);
    let tape = write_json(
        &fixture,
        "unused-tape.json",
        json!({"version":1,"replies":[{"content":"unused"}]}),
    );
    let manifest = write_json(
        &fixture,
        "listener.json",
        json!({"name":"Listener shutdown","timeout_ms":10000,
            "actors":[{"name":"rose","config":config,"model_tape":tape,"goal":"","start":"/listen","expected_status":"listening"}],
            "assertions":{"eventually":[{"kind":"snapshot","pointer":"/state/status","equals":"listening"}]}
        }),
    );
    let dir = fixture.dir.join("listener-results");
    let result = output(
        fixture
            .command(&config)
            .arg("--harness")
            .arg(manifest)
            .arg("--output")
            .arg(&dir),
    )
    .await;
    let report: Value =
        serde_json::from_slice(&std::fs::read(dir.join("summary.json")).unwrap()).unwrap();
    assert!(result.status.success(), "{}", report);
    assert_eq!(
        report["runs"][0]["result"]["actors"]["rose"]["status_before_shutdown"],
        "listening"
    );
    assert_eq!(
        report["runs"][0]["result"]["actors"]["rose"]["report"]["state"]["status"],
        "paused"
    );
    let script = fixture.scenario(json!([
        {"command":"help inventory","expect":["List everything you are carrying"]},
        {"command":"look","expect":["Market Square"]}
    ]));
    let manifest = write_json(
        &fixture,
        "budget.json",
        json!({"name":"Expected script limit","timeout_ms":10000,
            "actors":[{"name":"rose","config":config,"script":script,"expected_status":"blocked"}],
            "assertions":{"eventually":[{"kind":"blocked","contains":"Scenario step limit reached"}]}
        }),
    );
    let dir = fixture.dir.join("budget-results");
    let result = output(
        fixture
            .command(&config)
            .arg("--harness")
            .arg(manifest)
            .arg("--output")
            .arg(&dir),
    )
    .await;
    let report: Value =
        serde_json::from_slice(&std::fs::read(dir.join("summary.json")).unwrap()).unwrap();
    assert!(result.status.success(), "{}", report);
    assert_eq!(
        report["runs"][0]["result"]["actors"]["rose"]["exit_code"],
        2
    );
}

fn write_json(fixture: &Fixture, name: &str, value: Value) -> PathBuf {
    let path = fixture.dir.join(name);
    std::fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    path
}

#[test]
fn harness_owned_server_helper() {
    if std::env::var("MUSH_BOT_OWNED_HELPER").as_deref() != Ok("1") {
        return;
    }
    use std::io::Write;
    let mut lock = std::fs::File::create("owned-helper.lock").unwrap();
    lock.try_lock().unwrap();
    writeln!(lock, "{}", std::process::id()).unwrap();
    println!("OWNED-HELPER-READY");
    // This process belongs to the harness. The parent verifies that its lock is
    // released when the run finishes, proving owned helper cleanup.
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn harness_waits_for_owned_server_readiness_and_releases_it_after_the_run() {
    let fixture = Fixture::boot().await;
    let config = fixture.config(None, 10);
    let script = fixture.scenario(
        json!([{"command":"help inventory","expect":["List everything you are carrying"]}]),
    );
    let manifest = write_json(
        &fixture,
        "owned-server.json",
        json!({"name":"Owned helper lifecycle","timeout_ms":10000,
            "server":{"program":std::env::current_exe().unwrap(),"args":["--exact","lab::harness_owned_server_helper","--nocapture"],"working_directory":fixture.dir,"ready_output":"OWNED-HELPER-READY","timeout_ms":5000},
        "actors":[{"name":"rose","config":config,"script":script}],
            "assertions":{"eventually":[{"actor":"rose","kind":"ready","pointer":"/pending_command","equals":null}]}
        }),
    );
    let directory = fixture.dir.join("owned-results");
    let result = output(
        fixture
            .command(&config)
            .env("MUSH_BOT_OWNED_HELPER", "1")
            .arg("--harness")
            .arg(manifest)
            .arg("--output")
            .arg(&directory),
    )
    .await;
    let report: Value =
        serde_json::from_slice(&std::fs::read(directory.join("summary.json")).unwrap()).unwrap();
    assert!(
        result.status.success(),
        "{}",
        serde_json::to_string_pretty(&report).unwrap()
    );
    let file = std::fs::File::open(fixture.dir.join("owned-helper.lock")).unwrap();
    file.try_lock()
        .expect("the owned helper must have exited and released its file lock");
    assert!(
        std::fs::read_to_string(directory.join("run-0001/server-stdout.txt"))
            .unwrap()
            .contains("OWNED-HELPER-READY")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prompt_context_edits_stage_decisions_and_exact_capture_replays_without_ssh() {
    let fixture = Fixture::boot().await;
    let model = MockServer::start().await;
    let phase = Arc::new(AtomicU32::new(0));
    let next = phase.clone();
    Mock::given(method("POST")).respond_with(move |request:&Request| {
        let payload:Value=request.body_json().unwrap();
        let context=request_context(request);
        assert!(payload["messages"][0]["content"].as_str().unwrap().ends_with("Prefer precise amounts."));
        assert_eq!(context["terminal_history"],"");
        assert!(context["remembered_experience"].is_null());
        assert!(context["route_lookup"].is_null());
        assert!(context["reference_documents"].is_null());
        let content=match next.fetch_add(1,Ordering::SeqCst) {
            0 => json!({"action":"command","command":"look","summary":"Inspect the square.","memory":"Keep a rose map."}).to_string(),
            1 => {assert_eq!(context["memory"],"Keep a rose map."); "I remember the rose map.".into()},
            2 => json!({"action":"command","command":"say replay-must-not-run","summary":"Replay only."}).to_string(),
            _=>panic!("extra request"),
        };
        ResponseTemplate::new(200).set_body_json(json!({"choices":[{"message":{"content":content}}],"usage":{"cost":0.01}}))
    }).expect(3).mount(&model).await;
    let config = fixture.config(Some(&model), 10);
    let report = fixture.dir.join("edit-report.json");
    let trace = fixture.dir.join("edit-trace.jsonl");
    let capture = fixture.dir.join("capture.json");
    let mut bot = PlayerConsole::spawn(
        fixture
            .command(&config)
            .arg("--paused")
            .arg("--report")
            .arg(&report)
            .arg("--transcript")
            .arg(&trace),
    );
    bot.until("[bot/send] look").await;
    bot.until("Exits    east").await;
    bot.send(r#"/prompt set {"name":"precise","supplement":"Prefer precise amounts."}"#)
        .await;
    bot.until("Prompt profile changed").await;
    bot.send(r#"/context set {"history_bytes":0,"speech_limit":0,"retrieval":false,"routes":false,"references":false}"#).await;
    bot.until("Context settings changed").await;
    bot.send("/step model").await;
    bot.until("[bot/breakpoint]").await;
    bot.send(&format!("/capture {}", capture.display())).await;
    bot.until("Exact model request saved").await;
    bot.send("/commit").await;
    bot.until("[bot/send] look").await;
    bot.until("Exits    east").await;
    bot.send(r#"/prompt set {"version":2}"#).await;
    bot.until("[bot/control-error]").await;
    bot.send("/ask What do you remember?").await;
    bot.until("[bot/answer] I remember the rose map.").await;
    bot.quit().await;
    let report: Value = serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
    assert_eq!(report["state"]["commands_sent"], 3); // Bootstrap + one committed decision.
    assert_eq!(report["prompts"]["name"], "precise");
    let events: Vec<Value> = std::fs::read_to_string(trace)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let held = events
        .iter()
        .position(|e| e["kind"] == "staged-decision")
        .unwrap();
    let committed = events.iter().position(|e| e["kind"] == "decision").unwrap();
    assert!(held < committed);
    let input = events.iter().find(|e| e["kind"] == "model-input").unwrap();
    let reply = events.iter().find(|e| e["kind"] == "model-reply").unwrap();
    assert_eq!(input["request_id"], reply["request_id"]);
    assert!(events.iter().any(|e| {
        e["kind"] == "command-result"
            && e["data"]["output"]
                .as_str()
                .unwrap_or("")
                .contains("Market Square")
    }));
    let offline = fixture.dir.join("offline.toml");
    std::fs::write(
        &offline,
        std::fs::read_to_string(&config)
            .unwrap()
            .replace(&format!("port = {}", fixture.addr.port()), "port = 1"),
    )
    .unwrap();
    let replay = fixture.dir.join("replay.json");
    let result = output(
        fixture
            .command(&offline)
            .arg("--replay")
            .arg(&capture)
            .arg("--report")
            .arg(&replay),
    )
    .await;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let replay: Value = serde_json::from_slice(&std::fs::read(replay).unwrap()).unwrap();
    assert_eq!(replay["commands_sent"], 0);
    assert_eq!(replay["decision"]["command"], "say replay-must-not-run");
    let requests = model.received_requests().await.unwrap();
    assert_eq!(
        requests[0].body_json::<Value>().unwrap(),
        requests[2].body_json::<Value>().unwrap()
    );
    assert_eq!(phase.load(Ordering::SeqCst), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn held_decision_rejects_new_observations_and_watch_cancels_slow_tape() {
    let fixture = Fixture::boot_world(false, None, true).await;
    let mut marco = PlayerConsole::speaker(&fixture).await;
    let config = fixture.config(None, 10);
    let tape = write_json(
        &fixture,
        "watch-tape.json",
        json!({"version":1,"replies":[
            {"content":decision("command",Some("east"),"Stale travel.").to_string()},
            {"delay_ms":3000,"content":decision("command",Some("say stale-watch"),"Stale reply.").to_string()}
        ]}),
    );
    let report = fixture.dir.join("watch-report.json");
    let trace = fixture.dir.join("watch-trace.jsonl");
    let mut bot = PlayerConsole::spawn(
        fixture
            .command(&config)
            .arg("--paused")
            .arg("--model-tape")
            .arg(tape)
            .arg("--report")
            .arg(&report)
            .arg("--transcript")
            .arg(&trace),
    );
    bot.until("[bot/send] look").await;
    bot.until("Exits    east").await;
    bot.send("/step model").await;
    bot.until("[bot/breakpoint]").await;
    marco.say("A new fact just arrived.").await;
    bot.until("A new fact just arrived.").await;
    bot.send("/commit").await;
    bot.until("game output changed after the decision").await;
    bot.send(r#"/debug set {"observations":["urgent"]}"#).await;
    bot.until("Breakpoints and watches updated").await;
    bot.send("/resume").await;
    bot.until("[bot/model]").await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    marco.say("urgent: please wait").await;
    bot.until("[bot/watch]").await;
    bot.quit().await;
    marco.quit().await;
    let report: Value = serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
    assert_eq!(report["state"]["commands_sent"], 2);
    assert_eq!(report["state"]["memory"], "");
    let events: Vec<Value> = std::fs::read_to_string(trace)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert!(!events.iter().any(|e| e["kind"] == "decision"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cast_matrix_coordinates_real_speech_interruptions_with_isolated_memory_and_joined_evidence()
 {
    let fixture = Fixture::boot_world(false, None, true).await;
    let config = fixture.config(None, 10);
    let tape = write_json(
        &fixture,
        "cast-tape.json",
        json!({"version":1,"replies":[
            {"delay_ms":3000,"content":decision("command",Some("say stale-cast"),"Outdated decision.").to_string()},
            {"content":decision("command",Some("say Hello, visitor."),"Answer the greeting.").to_string()},
            {"content":decision("wait",None,"Allow the conversation to settle.").to_string().replace("\"wait_ms\":0","\"wait_ms\":1000")}
        ]}),
    );
    let a = write_json(
        &fixture,
        "a.json",
        json!({"name":"a","supplement":"Be warm."}),
    );
    let b = write_json(
        &fixture,
        "b.json",
        json!({"name":"b","supplement":"Be concise."}),
    );
    let harness = write_json(
        &fixture,
        "cast.json",
        json!({"name":"Conversation interruption","timeout_ms":15000,
            "actors":[
                {"name":"rose","config":config,"model_tape":tape,"start":"/resume","expected_status":"paused"},
            {"name":"marco","config":config,"login":"marco+Marco","goal":"","expected_status":"paused"}
            ],
            "steps":[
                {"expect":[{"actor":"rose","kind":"model","contains":"Choosing next action"}]},
                {"actor":"marco","control":"/send say Rosebot, are you there?","expect":[{"actor":"rose","kind":"send","contains":"say Hello, visitor."}]},
                {"actor":"rose","control":"/pause","expect":[{"actor":"rose","kind":"paused"}]}
            ],
            "assertions":{
                "eventually":[{"actor":"marco","kind":"observation","contains":"Hello, visitor."}],
                "ordered":[{"actor":"rose","kind":"model-input"},{"actor":"rose","kind":"attention"},{"actor":"rose","kind":"send","contains":"Hello, visitor."}],
                "forbidden":[{"actor":"rose","kind":"send","contains":"stale-cast"}]
            },
            "matrix":{"actor":"rose","prompts":[a,b],"repeats":2}
        }),
    );
    let directory = fixture.dir.join("cast-results");
    let result = output(
        fixture
            .command(&config)
            .arg("--harness")
            .arg(harness)
            .arg("--output")
            .arg(&directory),
    )
    .await;
    let summary: Value =
        serde_json::from_slice(&std::fs::read(directory.join("summary.json")).unwrap()).unwrap();
    assert!(
        result.status.success(),
        "{}\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr),
        serde_json::to_string_pretty(&summary).unwrap()
    );
    assert_eq!(summary["runs"].as_array().unwrap().len(), 4);
    for (i, run) in summary["runs"].as_array().unwrap().iter().enumerate() {
        assert_eq!(run["result"]["success"], true);
        assert_eq!(
            run["result"]["assertions"]["ordered_evidence"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        let report = &run["result"]["actors"]["rose"]["report"];
        assert_eq!(report["costs"]["session"]["totals"]["cancelled"], 1);
        assert!(
            report["memory_store"]["file"]
                .as_str()
                .unwrap()
                .contains(&format!("run-{:04}", i + 1))
        );
        assert!(
            directory
                .join(format!("run-{:04}/joined.jsonl", i + 1))
                .exists()
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn harness_rejects_self_reported_success_without_world_evidence_and_handles_disconnect_fault()
{
    let fixture = Fixture::boot().await;
    let config = fixture.config(None, 10);
    let tape = write_json(
        &fixture,
        "claim-tape.json",
        json!({"version":1,"replies":[{"content":decision("done",None,"I reached the Flower Shop.").to_string()}]}),
    );
    let manifest = write_json(
        &fixture,
        "unproven.json",
        json!({"name":"Unproven success","timeout_ms":3000,
            "actors":[{"name":"rose","config":config,"model_tape":tape,"start":"/resume"}],
            "assertions":{"eventually":[{"kind":"command-result","text_pointer":"/output","contains":"Flower Shop #"}]}
        }),
    );
    let dir = fixture.dir.join("unproven-results");
    let result = output(
        fixture
            .command(&config)
            .arg("--harness")
            .arg(manifest)
            .arg("--output")
            .arg(&dir),
    )
    .await;
    assert!(!result.status.success());
    let report: Value =
        serde_json::from_slice(&std::fs::read(dir.join("summary.json")).unwrap()).unwrap();
    assert_eq!(report["runs"][0]["result"]["assertions"]["success"], false);
    let disconnect = write_json(
        &fixture,
        "disconnect.json",
        json!({"name":"Connection loss","timeout_ms":10000,
            "actors":[{"name":"rose","config":config,"test_controls":true,"expected_status":"error"}],
            "steps":[{"actor":"rose","control":"/fault disconnect","expect":[{"actor":"rose","kind":"error","contains":"SSH session closed"}]}],
            "assertions":{"forbidden":[{"kind":"model-input"}]}
        }),
    );
    let dir = fixture.dir.join("disconnect-results");
    let result = output(
        fixture
            .command(&config)
            .arg("--harness")
            .arg(disconnect)
            .arg("--output")
            .arg(&dir),
    )
    .await;
    let report: Value =
        serde_json::from_slice(&std::fs::read(dir.join("summary.json")).unwrap()).unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stderr),
        serde_json::to_string_pretty(&report).unwrap()
    );
}

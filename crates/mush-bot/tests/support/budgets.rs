use super::*;

fn write_json(fixture: &Fixture, name: &str, value: Value) -> PathBuf {
    let path = fixture.dir.join(name);
    std::fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    path
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn currency_config(fixture: &Fixture, model: &MockServer) -> PathBuf {
    let config = fixture.config(Some(model), 10);
    let text = std::fs::read_to_string(&config).unwrap().replace(
        "name = \"fixture-model\"",
        "name = \"fixture-model\"\ncost_currency = \"USD\"",
    );
    std::fs::write(&config, text).unwrap();
    config
}

fn charged_reply(content: &str, cost: Option<&str>) -> ResponseTemplate {
    let mut usage = json!({"prompt_tokens":10,"completion_tokens":5});
    if let Some(cost) = cost {
        usage["cost"] = serde_json::from_str(cost).unwrap();
    }
    ResponseTemplate::new(200).set_body_json(json!({
        "model":"fixture-model","choices":[{"message":{"content":content}}],"usage":usage
    }))
}

async fn event(bot: &mut PlayerConsole, kind: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut seen = Vec::new();
        while let Some(line) = bot.lines.next_line().await.unwrap() {
            let row: Value = serde_json::from_str(&line).expect("JSON event output");
            if row["kind"] == kind {
                return row;
            }
            seen.push(row);
        }
        panic!("bot closed before {kind}: {seen:?}");
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {kind}"))
}

async fn text_event(bot: &mut PlayerConsole, kind: &str) -> Value {
    let row = event(bot, kind).await;
    serde_json::from_str(row["data"].as_str().unwrap()).unwrap()
}

async fn budget(bot: &mut PlayerConsole) -> Value {
    bot.send("/budget show").await;
    text_event(bot, "budget").await
}

async fn set_budget(bot: &mut PlayerConsole, settings: Value) -> Value {
    bot.send(&format!("/budget set {settings}")).await;
    let changed = event(bot, "budget-settings").await["data"].clone();
    event(bot, "paused").await;
    changed
}

async fn denied(bot: &mut PlayerConsole, reason: &str) -> Value {
    let blocked = event(bot, "budget-blocked").await["data"].clone();
    assert_eq!(blocked["status"]["allowed"], false);
    assert!(
        blocked["status"]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["kind"] == reason),
        "{blocked}"
    );
    event(bot, "blocked").await;
    blocked
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_request_limit_survives_goal_and_model_changes_and_covers_questions_and_probes() {
    const MEMORY: &str = "I promised to bring a map to the flower shop.";
    let fixture = Fixture::boot().await;
    let model = MockServer::start().await;
    let calls = Arc::new(AtomicU32::new(0));
    let next = calls.clone();
    Mock::given(method("POST"))
        .respond_with(move |request: &Request| {
            let body: Value = request.body_json().unwrap();
            let turn = next.fetch_add(1, Ordering::SeqCst);
            if turn > 0 {
                assert_eq!(request_context(request)["memory"], MEMORY);
                assert_eq!(request_context(request)["runner_goal"], "Inspect the market only.");
                assert_eq!(body["model"], "fixture-careful");
            }
            match turn {
                0 => {
                    assert_eq!(body["model"], "fixture-fast");
                    charged_reply(&json!({"action":"wait","summary":"Keep the map promise.","memory":MEMORY,"wait_ms":100}).to_string(), Some("0.1"))
                }
                1 => charged_reply("The map promise remains in memory.", Some("0.1")),
                2 => charged_reply(r#"{"answers":{"understood":true},"evidence":[]}"#, Some("0.1")),
                n => panic!("denied requests must not reach the provider: {n}"),
            }
        })
        .expect(3)
        .mount(&model)
        .await;
    write_json(
        &fixture,
        "models.json",
        json!({"version":1,"default":"fast","profiles":{
            "fast":{"endpoint":format!("{}/v1/chat/completions",model.uri()),"name":"fixture-fast","cost_currency":"USD"},
            "careful":{"endpoint":format!("{}/v1/chat/completions",model.uri()),"name":"fixture-careful","cost_currency":"USD"}
        }}),
    );
    let config = fixture.config(None, 10);
    std::fs::write(
        &config,
        format!(
            "model_profiles = \"models.json\"\n{}",
            std::fs::read_to_string(&config).unwrap()
        ),
    )
    .unwrap();
    let policy = write_json(
        &fixture,
        "request-budget.json",
        json!({"version":1,"max_requests":1}),
    );
    let probe = write_json(
        &fixture,
        "understood-probe.json",
        json!({
            "version":1,"id":"budget_probe","question":"Confirm you understood the runner's question.",
            "fields":[{"name":"understood","type":"boolean","description":"Whether the question was understood."}],
            "checks":[{"id":"understood","field":"understood","equals":true}]
        }),
    );
    let report = fixture.dir.join("request-budget-report.json");
    let trace = fixture.dir.join("request-budget-trace.jsonl");
    let mut bot = PlayerConsole::spawn(
        fixture
            .command(&config)
            .arg("--budget")
            .arg(&policy)
            .arg("--paused")
            .arg("--json-events")
            .arg("--report")
            .arg(&report)
            .arg("--transcript")
            .arg(&trace),
    );
    event(&mut bot, "ready").await;
    bot.send("/step").await;
    event(&mut bot, "summary").await;
    bot.send("/goal Inspect the market only.").await;
    event(&mut bot, "paused").await;
    bot.send("/model use careful").await;
    event(&mut bot, "model-selected").await;
    event(&mut bot, "paused").await;
    bot.send("/ask What do you remember?").await;
    let blocked = denied(&mut bot, "request_limit_reached").await;
    assert_eq!(blocked["status"]["usage"]["requests"], 1);
    assert_eq!(blocked["status"]["remaining_requests"], 0);
    bot.send("/state").await;
    let state = text_event(&mut bot, "state").await;
    assert_eq!(state["state"]["status"], "blocked");
    assert_eq!(
        state["state"]["model_calls"], 0,
        "denial must not increment the reset goal counter"
    );
    assert_eq!(state["state"]["memory"], MEMORY);

    let changed = set_budget(&mut bot, json!({"version":1,"max_requests":3})).await;
    assert_ne!(changed["revision"], blocked["revision"]);
    let raised = budget(&mut bot).await;
    assert_eq!(raised["status"]["usage"]["requests"], 1);
    assert_eq!(raised["status"]["remaining_requests"], 2);
    bot.send("/ask What do you remember?").await;
    assert_eq!(
        event(&mut bot, "answer").await["data"],
        "The map promise remains in memory."
    );
    bot.send(&format!("/probe {}", probe.display())).await;
    assert_eq!(
        event(&mut bot, "probe-result").await["data"]["success"],
        true
    );
    bot.send("/ask Can another question bypass the session cap?")
        .await;
    denied(&mut bot, "request_limit_reached").await;
    let saved = fixture.dir.join("saved-budget.json");
    bot.send(&format!("/budget save {}", saved.display())).await;
    event(&mut bot, "saved").await;
    assert_eq!(read_json(&saved)["max_requests"], 3);
    bot.quit().await;

    let report = read_json(&report);
    assert_eq!(report["state"]["memory"], MEMORY);
    assert_eq!(report["state"]["model_calls"], 2);
    assert_eq!(report["budget"]["status"]["usage"]["requests"], 3);
    assert_eq!(report["budget"]["status"]["allowed"], false);
    assert_eq!(report["budget"]["revision"], changed["revision"]);
    assert_eq!(report["costs"]["session"]["totals"]["requests"], 3);
    for purpose in ["goal_decision", "runner_question", "structured_probe"] {
        assert_eq!(
            report["costs"]["session"]["by_prompt_type"][purpose]["requests"],
            1
        );
    }
    let events: Vec<Value> = std::fs::read_to_string(trace)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        events
            .iter()
            .filter(|event| event["kind"] == "model-request")
            .count(),
        3
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["kind"] == "budget-blocked")
            .count(),
        2
    );
    assert_eq!(model.received_requests().await.unwrap().len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cost_gate_uses_exact_receipts_and_requires_an_explicit_unknown_cost_override() {
    let fixture = Fixture::boot().await;
    let model = MockServer::start().await;
    let next = Arc::new(AtomicU32::new(0));
    Mock::given(method("POST"))
        .respond_with(
            move |_: &Request| match next.fetch_add(1, Ordering::SeqCst) {
                0 => charged_reply("First receipt.", Some("0.1")),
                1 => charged_reply("Second receipt.", Some("0.2")),
                2 => charged_reply("This provider omitted its charge.", None),
                3 => charged_reply(
                    "The runner explicitly allowed an unknown charge.",
                    Some("0.05"),
                ),
                n => panic!("cost-denied request reached provider: {n}"),
            },
        )
        .expect(4)
        .mount(&model)
        .await;
    let config = currency_config(&fixture, &model);
    let policy = write_json(
        &fixture,
        "cost-budget.json",
        json!({"version":1,"max_cost_by_currency":{"USD":"0.3"}}),
    );
    // Config-relative policy loading exercises the same entry point used by
    // harness actors without a CLI policy override.
    std::fs::write(
        &config,
        format!(
            "budget = \"cost-budget.json\"\n{}",
            std::fs::read_to_string(&config).unwrap()
        ),
    )
    .unwrap();
    assert!(policy.exists());
    let report = fixture.dir.join("cost-budget-report.json");
    let mut bot = PlayerConsole::spawn(
        fixture
            .command(&config)
            .arg("--paused")
            .arg("--json-events")
            .arg("--report")
            .arg(&report),
    );
    event(&mut bot, "ready").await;
    bot.send("/ask First question.").await;
    event(&mut bot, "answer").await;
    let below = budget(&mut bot).await;
    assert_eq!(below["status"]["allowed"], true);
    assert_eq!(below["status"]["remaining_cost_by_currency"]["USD"], "0.2");
    bot.send("/ask Second question.").await;
    event(&mut bot, "answer").await;
    bot.send("/ask Third question must wait.").await;
    let exact = denied(&mut bot, "cost_limit_reached").await;
    assert_eq!(exact["status"]["usage"]["requests"], 2);
    assert_eq!(
        exact["status"]["usage"]["provider_cost_by_currency"]["USD"],
        "0.3"
    );
    assert_eq!(exact["status"]["remaining_cost_by_currency"]["USD"], "0");

    let raised = write_json(
        &fixture,
        "raised-cost-budget.json",
        json!({"version":1,"max_cost_by_currency":{"USD":"1"}}),
    );
    bot.send(&format!("/budget load {}", raised.display()))
        .await;
    event(&mut bot, "budget-settings").await;
    event(&mut bot, "paused").await;
    bot.send("/ask Now receive an unknown charge.").await;
    event(&mut bot, "answer").await;
    bot.send("/ask An omitted charge must not count as free.")
        .await;
    let unknown = denied(&mut bot, "unknown_cost_requests").await;
    assert_eq!(unknown["status"]["usage"]["requests"], 3);
    assert_eq!(unknown["status"]["usage"]["unknown_cost_requests"], 1);
    assert_eq!(
        unknown["status"]["remaining_cost_by_currency"]["USD"],
        "0.7"
    );
    set_budget(
        &mut bot,
        json!({"version":1,"max_cost_by_currency":{"USD":"1"},"unknown_cost":"allow"}),
    )
    .await;
    bot.send("/ask Continue with the known accounting gap.")
        .await;
    event(&mut bot, "answer").await;
    bot.quit().await;

    let report = read_json(&report);
    assert_eq!(report["budget"]["settings"]["unknown_cost"], "allow");
    assert_eq!(report["budget"]["status"]["allowed"], true);
    assert_eq!(report["budget"]["status"]["usage"]["requests"], 4);
    assert_eq!(
        report["budget"]["status"]["usage"]["unknown_cost_requests"],
        1
    );
    assert_eq!(
        report["budget"]["status"]["remaining_cost_by_currency"]["USD"],
        "0.65"
    );
    assert_eq!(
        report["costs"]["session"]["totals"]["provider_cost_by_currency"]["USD"],
        "0.35"
    );
    assert_eq!(model.received_requests().await.unwrap().len(), 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn denied_and_cancelled_calls_preserve_controls_and_invalid_replies_still_consume_requests() {
    let fixture = Fixture::boot().await;
    let model = MockServer::start().await;
    let pending = Arc::new(Notify::new());
    let notify = pending.clone();
    let next = Arc::new(AtomicU32::new(0));
    Mock::given(method("POST"))
        .respond_with(
            move |_: &Request| match next.fetch_add(1, Ordering::SeqCst) {
                0 => {
                    notify.notify_one();
                    charged_reply("STALE_BUDGET_CANCELLED_ANSWER", Some("5"))
                        .set_delay(Duration::from_secs(2))
                }
                1 => charged_reply("This is not a valid model decision.", Some("0.1")),
                2 => charged_reply(
                    "Questions still work after a failed decision.",
                    Some("0.05"),
                ),
                n => panic!("request {n} should have been blocked"),
            },
        )
        .expect(3)
        .mount(&model)
        .await;
    let config = currency_config(&fixture, &model);
    let policy = write_json(
        &fixture,
        "zero-budget.json",
        json!({"version":1,"max_requests":0}),
    );
    let report = fixture.dir.join("cancel-budget-report.json");
    let trace = fixture.dir.join("cancel-budget-trace.jsonl");
    let mut bot = PlayerConsole::spawn(
        fixture
            .command(&config)
            .arg("--budget")
            .arg(&policy)
            .arg("--paused")
            .arg("--json-events")
            .arg("--report")
            .arg(&report)
            .arg("--transcript")
            .arg(&trace),
    );
    event(&mut bot, "ready").await;
    bot.send("/ask The zero limit blocks this question.").await;
    assert_eq!(
        denied(&mut bot, "request_limit_reached").await["status"]["usage"]["requests"],
        0
    );
    bot.send("/send east").await;
    let command = event(&mut bot, "command-result").await;
    assert_eq!(command["data"]["command"], "east");
    assert!(
        command["data"]["output"]
            .as_str()
            .unwrap()
            .contains("Flower Shop")
    );
    assert_eq!(budget(&mut bot).await["status"]["usage"]["requests"], 0);

    set_budget(&mut bot, json!({"version":1,"max_requests":1})).await;
    bot.send("/ask Hold this request open.").await;
    tokio::time::timeout(Duration::from_secs(10), pending.notified())
        .await
        .unwrap();
    bot.send("/pause").await;
    event(&mut bot, "model-cancelled").await;
    event(&mut bot, "paused").await;
    bot.send("/resume").await;
    let cancelled = denied(&mut bot, "request_limit_reached").await;
    assert_eq!(cancelled["status"]["usage"]["requests"], 1);
    // Cancelled before a receipt: unpriced, but not an unknown cost that a
    // spending policy would block on (the runner cancels by itself on speech).
    assert_eq!(cancelled["status"]["usage"]["unknown_cost_requests"], 0);
    set_budget(&mut bot, json!({"version":1,"max_requests":3})).await;
    bot.send("/step").await;
    let invalid = event(&mut bot, "blocked").await;
    assert!(
        invalid["data"]
            .as_str()
            .unwrap()
            .contains("Invalid model decision")
    );
    assert_eq!(budget(&mut bot).await["status"]["usage"]["requests"], 2);
    bot.send("/ask Recover after the invalid decision.").await;
    event(&mut bot, "answer").await;
    bot.send("/ask There are no session requests left.").await;
    denied(&mut bot, "request_limit_reached").await;
    bot.quit().await;

    let report = read_json(&report);
    let totals = &report["costs"]["session"]["totals"];
    assert_eq!(totals["requests"], 3);
    assert_eq!(totals["cancelled"], 1);
    assert_eq!(totals["responses"], 2);
    assert_eq!(totals["unknown_cost_requests"], 0);
    assert_eq!(totals["cancelled_unpriced_requests"], 1);
    assert_eq!(totals["provider_cost_by_currency"]["USD"], "0.15");
    assert_eq!(report["budget"]["status"]["usage"]["requests"], 3);
    assert_eq!(report["state"]["model_calls"], 3);
    let trace = std::fs::read_to_string(trace).unwrap();
    assert!(!trace.contains("STALE_BUDGET_CANCELLED_ANSWER"));
    assert_eq!(model.received_requests().await.unwrap().len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zero_request_limit_does_not_prevent_command_scripts() {
    let fixture = Fixture::boot().await;
    let model = MockServer::start().await;
    let config = currency_config(&fixture, &model);
    let policy = write_json(
        &fixture,
        "script-budget.json",
        json!({"version":1,"max_requests":0}),
    );
    let scenario = fixture.scenario(json!([
        {"command":"east","expect":["Flower Shop"]},
        {"command":"get rose","expect":["You take rose."]},
        {"command":"inventory","expect":["You are carrying:","rose"]}
    ]));
    let report = fixture.dir.join("script-budget-report.json");
    let result = output(
        fixture
            .command(&config)
            .arg("--budget")
            .arg(&policy)
            .arg("--script")
            .arg(&scenario)
            .arg("--report")
            .arg(&report),
    )
    .await;
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let report = read_json(&report);
    assert_eq!(report["success"], true);
    assert_eq!(report["budget"]["status"]["allowed"], false);
    assert_eq!(report["budget"]["status"]["usage"]["requests"], 0);
    assert_eq!(report["costs"]["session"]["totals"]["requests"], 0);
    assert!(model.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replay_checks_session_limits_before_dispatch_and_records_the_final_receipt() {
    // No world or SSH listener is started: even the successful replay must
    // work with an unavailable SSH endpoint and an unset login secret.
    let dir = std::env::temp_dir().join(format!(
        "mush-bot-budget-replay-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join(".env"), "").unwrap();
    let fixture = Fixture {
        dir,
        addr: "127.0.0.1:1".parse().unwrap(),
        pin: "unused-replay-pin".into(),
    };
    let model = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(charged_reply(
            &json!({
                "action":"command","command":"say REPLAY_COMMAND_MUST_NEVER_BE_SENT",
                "summary":"This command is only a replay result."
            })
            .to_string(),
            Some("0.25"),
        ))
        .expect(1)
        .mount(&model)
        .await;
    let settings: mush_bot::config::Model = serde_json::from_value(json!({
        "endpoint":format!("{}/v1/chat/completions", model.uri()),
        "name":"fixture-model","cost_currency":"USD","timeout_secs":10
    }))
    .unwrap();
    let capture = mush_bot::model::Brain::new(&settings).unwrap().prepare(
        json!({"runner_goal":"Inspect a saved request only.","memory":"Keep the replay command local."}),
        None,
        &mush_bot::experiment::Prompts::default(),
    ).unwrap();
    capture.validate().unwrap();
    let capture_path = write_json(
        &fixture,
        "replay-capture.json",
        serde_json::to_value(&capture).unwrap(),
    );
    let original = std::fs::read(&capture_path).unwrap();
    let config = currency_config(&fixture, &model);
    let blocked_policy = write_json(
        &fixture,
        "blocked-replay-budget.json",
        json!({"version":1,"max_requests":0}),
    );
    let blocked_path = fixture.dir.join("blocked-replay-report.json");
    let blocked = output(
        fixture
            .command(&config)
            .env_remove("MUSH_BOT_E2E_PASSWORD")
            .arg("--replay")
            .arg(&capture_path)
            .arg("--budget")
            .arg(&blocked_policy)
            .arg("--report")
            .arg(&blocked_path),
    )
    .await;
    assert_eq!(
        blocked.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&blocked.stderr)
    );
    let blocked = read_json(&blocked_path);
    assert_eq!(blocked["success"], false);
    assert_eq!(blocked["request"], Value::Null);
    assert_eq!(blocked["content"], Value::Null);
    assert_eq!(blocked["commands_sent"], 0);
    assert_eq!(blocked["costs"]["session"]["totals"]["requests"], 0);
    assert_eq!(blocked["budget"]["status"]["usage"]["requests"], 0);
    assert_eq!(
        blocked["budget"]["status"]["reasons"],
        json!([{"kind":"request_limit_reached","limit":0}])
    );
    assert!(model.received_requests().await.unwrap().is_empty());

    let allowed_policy = write_json(
        &fixture,
        "allowed-replay-budget.json",
        json!({
            "version":1,"max_requests":1,"max_cost_by_currency":{"USD":"0.25"}
        }),
    );
    let allowed_path = fixture.dir.join("allowed-replay-report.json");
    let allowed = output(
        fixture
            .command(&config)
            .env_remove("MUSH_BOT_E2E_PASSWORD")
            .arg("--replay")
            .arg(&capture_path)
            .arg("--budget")
            .arg(&allowed_policy)
            .arg("--report")
            .arg(&allowed_path),
    )
    .await;
    assert!(
        allowed.status.success(),
        "{}",
        String::from_utf8_lossy(&allowed.stderr)
    );
    let allowed = read_json(&allowed_path);
    assert_eq!(allowed["success"], true);
    assert_eq!(allowed["commands_sent"], 0);
    assert_eq!(
        allowed["decision"]["command"],
        "say REPLAY_COMMAND_MUST_NEVER_BE_SENT"
    );
    assert_eq!(allowed["request"]["prompt_type"], "decision_replay");
    assert_eq!(allowed["budget_check"]["allowed"], true);
    assert_eq!(allowed["budget_check"]["usage"]["requests"], 0);
    assert_eq!(allowed["budget"]["status"]["allowed"], false);
    assert_eq!(allowed["budget"]["status"]["usage"]["requests"], 1);
    assert_eq!(
        allowed["budget"]["status"]["usage"]["unknown_cost_requests"],
        0
    );
    assert_eq!(
        allowed["budget"]["status"]["remaining_cost_by_currency"]["USD"],
        "0"
    );
    assert_eq!(
        allowed["costs"]["session"]["by_prompt_type"]["decision_replay"]["provider_cost_by_currency"]
            ["USD"],
        "0.25"
    );
    assert_eq!(
        allowed["budget"]["status"]["reasons"],
        json!([
            {"kind":"request_limit_reached","limit":1},
            {"kind":"cost_limit_reached","currency":"USD","limit":"0.25","spent":"0.25"}
        ])
    );
    assert_eq!(std::fs::read(&capture_path).unwrap(), original);
    let requests = model.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body_json::<Value>().unwrap(), capture.payload);
}

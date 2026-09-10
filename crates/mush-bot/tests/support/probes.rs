use super::*;

const PRICE_QUOTE: &str = "The florist quoted seven silver for a rose; I did not buy it.";
const PRIVATE_CHECK: &str = "PRIVATE_CHECK_SENTINEL_amount";

fn write_json(fixture: &Fixture, name: &str, value: Value) -> PathBuf {
    let path = fixture.dir.join(name);
    std::fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    path
}

fn price_probe(id: &str, question: &str) -> Value {
    json!({"version":1,"id":id,"question":question,"fields":[
        {"name":"quoted_amount","type":"number","description":"The quoted number of coins for a rose."},
        {"name":"currency","type":"string","description":"The currency of that quote."},
        {"name":"purchase_confirmed","type":"boolean","description":"Whether the supplied evidence establishes a completed purchase; null when unestablished."}
    ],"checks":[
        {"id":PRIVATE_CHECK,"field":"quoted_amount","equals":7,"evidence":{"source":"observed","contains":"quoted seven silver"}},
        {"id":"quoted_currency","field":"currency","equals":"silver","evidence":{"source":"observed","contains":"seven silver"}},
        {"id":"no_confirmed_purchase","field":"purchase_confirmed","equals":null}
    ]})
}

fn with_currency(fixture: &Fixture, model: &MockServer) -> PathBuf {
    let config = fixture.config(Some(model), 10);
    let text = std::fs::read_to_string(&config).unwrap().replace(
        "name = \"fixture-model\"",
        "name = \"fixture-model\"\ncost_currency = \"USD\"",
    );
    std::fs::write(&config, text).unwrap();
    config
}

fn context_value(payload: &Value, field: &str) -> Value {
    for message in payload["messages"].as_array().unwrap() {
        let Ok(value) = serde_json::from_str::<Value>(message["content"].as_str().unwrap()) else {
            continue;
        };
        if value["component"] == field {
            return value["value"].clone();
        }
        if let Some(value) = value.get(field) {
            return value.clone();
        }
    }
    panic!("prepared request omitted {field}");
}

fn quoted_episode(payload: &Value) -> u64 {
    context_value(payload, "remembered_experience")["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| {
            record["event"]["type"] == "episode"
                && record["event"]["data"]["source"] == "observed"
                && record["event"]["data"]["text"]
                    .as_str()
                    .is_some_and(|text| text.contains(PRICE_QUOTE))
        })
        .expect("each actor must recall the speech it actually observed")["id"]
        .as_u64()
        .unwrap()
}

fn price_answer(record_id: u64, fabricated: bool) -> Value {
    json!({"answers":{"quoted_amount":7,"currency":"silver","purchase_confirmed":null},"evidence":[
        {"field":"quoted_amount","record_id":record_id,"quote":if fabricated {"FABRICATED_PURCHASE_QUOTE"} else {PRICE_QUOTE}},
        {"field":"currency","record_id":record_id,"quote":PRICE_QUOTE}
    ]})
}

fn priced_response(content: String) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "model":"fixture-model","choices":[{"message":{"content":content}}],
        "usage":{"cost":0.125,"prompt_tokens":40,"completion_tokens":12}
    }))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serial_group_probes_grade_private_evidence_and_stop_on_a_fabricated_quote() {
    let fixture = Fixture::boot_world(false, None, true).await;
    let model = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|request: &Request| {
            let payload: Value = request.body_json().unwrap();
            assert!(!payload.to_string().contains(PRIVATE_CHECK));
            let record_id = quoted_episode(&payload);
            let question = payload["messages"].as_array().unwrap().last().unwrap()["content"]
                .as_str()
                .unwrap();
            let fabricated = question.contains("SECOND_PROBE_FABRICATED_QUOTE");
            priced_response(price_answer(record_id, fabricated).to_string())
        })
        .expect(3)
        .mount(&model)
        .await;
    let config = with_currency(&fixture, &model);
    let prompts = write_json(
        &fixture,
        "composed-probes.json",
        json!({"name":"evidence","composition":{}}),
    );
    let good = write_json(
        &fixture,
        "quoted-price.json",
        price_probe(
            "quoted_price",
            "Recall the florist's quoted rose price and currency. Is a purchase confirmed?",
        ),
    );
    let fabricated = write_json(
        &fixture,
        "fabricated-price.json",
        price_probe(
            "fabricated_quote",
            "SECOND_PROBE_FABRICATED_QUOTE: Recall the quoted rose price and currency. Is a purchase confirmed?",
        ),
    );
    let say = format!("say {PRICE_QUOTE}");
    let manifest = write_json(
        &fixture,
        "probe-party.json",
        json!({"name":"Independent group testimony checks","timeout_ms":15000,
        "actors":[
            {"name":"scout","config":config,"prompts":prompts,"expected_status":"paused"},
            {"name":"medic","config":config,"prompts":prompts,"login":"marco+Marco","expected_status":"paused"}
        ],
        "groups":[{"id":"party","name":"Lantern Company","kind":"adventuring party","mission":"Compare what we actually heard.",
            "members":[{"actor":"scout","role":"pathfinder"},{"actor":"medic","role":"healer"}]}],
        "steps":[
            {"name":"Observe quoted price","actor":"medic","control":format!("/send {say}"),"expect":[
                {"actor":"scout","kind":"observation","contains":PRICE_QUOTE},
                {"actor":"medic","kind":"command-result","pointer":"/command","equals":say,"text_pointer":"/output","contains":PRICE_QUOTE}
            ]},
            {"name":"Check both memories","group":"party","dispatch":"serial","probe":good},
            {"name":"Reject fabricated provenance","group":"party","dispatch":"serial","probe":fabricated}
        ]}),
    );
    let directory = fixture.dir.join("probe-party-results");
    let result = output(
        fixture
            .command(&config)
            .arg("--harness")
            .arg(&manifest)
            .arg("--output")
            .arg(&directory),
    )
    .await;
    assert_eq!(result.status.code(), Some(1));
    let summary: Value =
        serde_json::from_slice(&std::fs::read(directory.join("summary.json")).unwrap()).unwrap();
    let run = &summary["runs"][0]["result"];
    assert_eq!(run["success"], false);
    assert_eq!(
        run["steps"].as_array().unwrap().len(),
        2,
        "{}",
        run["error"]
    );
    let probes = run["probes"].as_array().unwrap();
    assert_eq!(probes.len(), 3, "{}", run["error"]);
    assert_eq!(
        probes
            .iter()
            .map(|p| p["actor"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["scout", "medic", "scout"]
    );
    assert_eq!(
        probes
            .iter()
            .map(|p| p["grade"]["success"].as_bool().unwrap())
            .collect::<Vec<_>>(),
        [true, true, false]
    );
    assert_eq!(
        probes[2]["grade"]["invalid_citations"][0]["reason"],
        "quote_not_in_captured_episode"
    );
    assert_eq!(run["pending_step"]["queued"], json!(["medic"]));
    assert_eq!(
        run["costs"]["cast"]["totals"]["provider_cost_by_currency"]["USD"],
        "0.375"
    );
    let costs = &run["costs"]["cast"]["by_prompt_type"]["structured_probe"];
    assert_eq!(costs["requests"], 3);
    assert_eq!(costs["responses"], 3);
    assert_eq!(costs["provider_cost_by_currency"]["USD"], "0.375");

    let joined: Vec<Value> = std::fs::read_to_string(directory.join("run-0001/joined.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for actor in ["scout", "medic"] {
        let events: Vec<&Value> = joined.iter().filter(|e| e["actor"] == actor).collect();
        assert!(
            events.iter().any(|e| e["event"]["kind"] == "result"),
            "failure cleanup must retain {actor}'s final report"
        );
        assert_eq!(
            events.last().unwrap()["event"]["kind"],
            "process-eof",
            "failure cleanup must drain {actor}'s stdout even after process exit"
        );
    }
    assert!(!joined.iter().any(|e| e["actor"] == "medic"
        && e["event"]["kind"] == "control-started"
        && e["event"]["data"]["id"] == "phase-3"));
    for (index, probe) in probes.iter().enumerate() {
        let actor = probe["actor"].as_str().unwrap();
        let capture: mush_bot::model::Capture = serde_json::from_value(
            joined
                .iter()
                .find(|event| {
                    event["actor"] == actor
                        && event["event"]["kind"] == "model-input"
                        && event["event"]["data"]["request_revision"]
                            == probe["grade"]["request_revision"]
                })
                .unwrap()["event"]["data"]
                .clone(),
        )
        .unwrap();
        capture.validate().unwrap();
        assert!(!capture.payload.to_string().contains(PRIVATE_CHECK));
        if index < 2 {
            assert_eq!(
                probe["grade"]["checks"][0]["evidence_ids"],
                json!([quoted_episode(&capture.payload)])
            );
        }
        assert!(joined.iter().any(|event| event["actor"] == actor
            && event["event"]["seq"] == probe["seq"]
            && event["event"]["kind"] == "probe-result"));
    }
    for (number, id) in [(2, "quoted_price"), (3, "fabricated_quote")] {
        let spec =
            mush_bot::probe::Spec::load(&directory.join(format!("run-0001/probe-{number}.json")))
                .unwrap();
        assert_eq!(spec.id, id);
        let revision = mush_bot::experiment::revision(&serde_json::to_vec(&spec).unwrap());
        assert!(
            probes
                .iter()
                .filter(|p| p["grade"]["probe_id"] == id)
                .all(|p| p["grade"]["definition_revision"] == revision)
        );
    }
    let analysis = serde_json::to_value(mush_bot::analysis::load(&directory).unwrap()).unwrap();
    assert_eq!(analysis["metrics"]["probes"]["totals"]["evaluations"], 3);
    assert_eq!(analysis["metrics"]["probes"]["totals"]["passed"], 2);
    assert_eq!(analysis["metrics"]["probes"]["totals"]["failed"], 1);
    assert_eq!(
        analysis["metrics"]["probes"]["totals"]["invalid_responses"],
        1
    );
    assert_eq!(
        analysis["metrics"]["session_costs"]["by_prompt_type"]["structured_probe"]["provider_cost_by_currency"]
            ["USD"],
        "0.375"
    );
    assert_eq!(model.received_requests().await.unwrap().len(), 3);
    // Exercise regression gates against a real failing cast artifact, including
    // exact actor-session charges and the grader's private-rubric boundary.
    let analysis = mush_bot::analysis::load(&directory).unwrap();
    let policy: mush_bot::regression::Policy = serde_json::from_value(json!({
        "version":1,"checks":[
            {"id":"observed-grade-rate","metric":"probe_pass_rate","minimum":"0.666666666666666666","max_drop":"0","min_samples":3},
            {"id":"exact-session-charge","metric":"provider_cost_mean","currency":"USD","maximum":"0.1875","max_increase":"0","min_samples":2},
            {"id":"session-requests","metric":"model_requests_mean","maximum":"1.5","min_samples":2}
        ]
    })).unwrap();
    let comparison = mush_bot::regression::evaluate(&policy, &analysis, &analysis).unwrap();
    assert!(
        comparison.success,
        "{}",
        serde_json::to_string(&comparison).unwrap()
    );
    assert!(
        !serde_json::to_string(&comparison)
            .unwrap()
            .contains(PRIVATE_CHECK)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pausing_a_probe_cancels_its_grade_and_keeps_unreceived_cost_unknown() {
    let fixture = Fixture::boot().await;
    let model = MockServer::start().await;
    let pending = Arc::new(Notify::new());
    let notify = pending.clone();
    let calls = Arc::new(AtomicU32::new(0));
    let next = calls.clone();
    Mock::given(method("POST"))
        .respond_with(move |request: &Request| {
            let payload: Value = request.body_json().unwrap();
            assert!(!payload.to_string().contains(PRIVATE_CHECK));
            match next.fetch_add(1, Ordering::SeqCst) {
                0 => {
                    notify.notify_one();
                    priced_response(json!({"answers":{"quoted_amount":7,"currency":"silver","purchase_confirmed":null},"evidence":[],"extra":"STALE_PROBE_REPLY"}).to_string())
                        .set_delay(Duration::from_secs(2))
                }
                1 => priced_response("Runner control still works.".into()).set_delay(Duration::from_millis(2500)),
                n => panic!("unexpected probe cancellation request {n}"),
            }
        })
        .expect(2)
        .mount(&model)
        .await;
    let config = with_currency(&fixture, &model);
    let probe = write_json(
        &fixture,
        "slow-probe.json",
        price_probe("slow_probe", "Recall the quoted rose price and currency."),
    );
    let report = fixture.dir.join("cancelled-probe-report.json");
    let trace = fixture.dir.join("cancelled-probe-trace.jsonl");
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
    bot.send(&format!("/probe {}", probe.display())).await;
    tokio::time::timeout(Duration::from_secs(10), pending.notified())
        .await
        .unwrap();
    bot.send("/pause").await;
    bot.until(r#""kind":"probe-cancelled""#).await;
    bot.send("/ask Is the runner still in control?").await;
    let answer = bot.until(r#""kind":"answer""#).await;
    assert!(answer.contains("Runner control still works."));
    assert!(!answer.contains(r#""kind":"probe-result""#));
    bot.quit().await;
    let report: Value = serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
    assert!(report["probe_results"].as_array().unwrap().is_empty());
    assert_eq!(report["state"]["status"], "paused");
    assert_eq!(report["state"]["model_calls"], 2);
    let costs = &report["costs"]["session"];
    assert_eq!(costs["totals"]["requests"], 2);
    assert_eq!(costs["totals"]["cancelled"], 1);
    assert_eq!(costs["totals"]["responses"], 1);
    assert_eq!(costs["totals"]["unknown_cost_requests"], 1);
    assert_eq!(costs["totals"]["known_cost_requests"], 1);
    assert_eq!(costs["totals"]["provider_cost_by_currency"]["USD"], "0.125");
    assert_eq!(costs["by_prompt_type"]["structured_probe"]["cancelled"], 1);
    assert_eq!(
        costs["by_prompt_type"]["structured_probe"]["unknown_cost_requests"],
        1
    );
    assert_eq!(costs["by_prompt_type"]["runner_question"]["responses"], 1);
    let text = std::fs::read_to_string(trace).unwrap();
    assert!(!text.contains("STALE_PROBE_REPLY"));
    let events: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        events
            .iter()
            .filter(|e| e["kind"] == "probe-cancelled")
            .count(),
        1
    );
    assert!(
        !events
            .iter()
            .any(|e| e["kind"] == "probe-result" || e["kind"] == "decision")
    );
    assert_eq!(model.received_requests().await.unwrap().len(), 2);
}

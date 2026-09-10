use super::*;

fn write(fixture: &Fixture, name: &str, value: Value) -> PathBuf {
    let path = fixture.dir.join(name);
    std::fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    path
}
fn events(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}
fn policy(fixture: &Fixture, extra: Value) -> PathBuf {
    let mut value =
        json!({"debounce_ms":50,"reply_min_ms":500,"reply_jitter_ms":0,"request_cooldown_ms":50});
    value
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    write(fixture, "conversation.json", value)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn harness_revokes_expired_leases_and_stops_if_the_owner_cannot_acknowledge() {
    for acknowledges in [true, false] {
        let fixture = Fixture::boot_world(false, None, true).await;
        let config = fixture.config(None, 20);
        let tape = write(
            &fixture,
            "tape.json",
            json!({"version":1,"replies":(0..20).map(|_| json!({
            "content":decision("command",Some("say Ready."),"Ready").to_string()
        })).collect::<Vec<_>>()}),
        );
        let policy = policy(&fixture, json!({"reply_min_ms":50}));
        let manifest = write(
            &fixture,
            "expiry-cast.json",
            json!({"name":"Expired floor","timeout_ms":15_000,
            "speaking_floor":{"lease_ms":800,"revoke_timeout_ms":1000,"speaking_window_ms":100},
            "actors":[
                {"name":"owner","config":config,"conversation":policy,"model_tape":tape,"test_controls":true,"expected_status":"paused"},
                {"name":"waiter","config":config,"login":"marco+Marco","conversation":policy,"model_tape":tape,"expected_status":"paused"}
            ],"steps":[
                {"actor":"owner","control":if acknowledges {"/fault floor-grants"} else {"/fault floor-controls"}},
                {"actor":"owner","control":"/resume","expect":[{"actor":"owner","kind":"speech-intent"}]},
                {"actor":"waiter","control":"/resume","expect":[{"actor":"waiter","kind":"speech-dispatched"}]},
                {"actors":["owner","waiter"],"control":"/pause"}
            ],"assertions":{"forbidden":[{"actor":"owner","kind":"speech-dispatched"}]}}),
        );
        let directory = fixture.dir.join("expiry-results");
        let output = output(
            fixture
                .command(&config)
                .arg("--harness")
                .arg(&manifest)
                .arg("--output")
                .arg(&directory),
        )
        .await;
        let summary: Value =
            serde_json::from_slice(&std::fs::read(directory.join("summary.json")).unwrap())
                .unwrap();
        assert_eq!(
            output.status.success(),
            acknowledges,
            "{}\n{summary}",
            String::from_utf8_lossy(&output.stderr)
        );
        let trace = events(&directory.join("run-0001/joined.jsonl"));
        assert!(
            trace.iter().any(|e| e["event"]["kind"] == "speaking-floor"
                && e["event"]["data"]["action"] == "revoke")
        );
        if !acknowledges {
            assert!(
                summary["runs"][0]["result"]["error"]
                    .as_str()
                    .unwrap()
                    .contains("did not acknowledge")
            );
            assert!(
                !trace
                    .iter()
                    .any(|e| e["event"]["kind"] == "speech-dispatched")
            );
            assert!(!trace.iter().any(|e| e["event"]["kind"] == "speaking-floor"
                && e["event"]["data"]["action"] == "grant"
                && e["event"]["data"]["actor"] == "waiter"));
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn managed_cast_disconnect_stops_grants_and_preserves_failure_artifacts() {
    let fixture = Fixture::boot_world(false, None, true).await;
    let config = fixture.config(None, 10);
    let tape = write(
        &fixture,
        "disconnect-tape.json",
        json!({"version":1,"replies":[{"content":decision("command",Some("say Should not send."),"Ready").to_string()}]}),
    );
    let policy = policy(&fixture, json!({"reply_min_ms":50}));
    let manifest = write(
        &fixture,
        "disconnect-cast.json",
        json!({"name":"Floor disconnect","timeout_ms":10_000,"speaking_floor":{},
        "actors":[
            {"name":"owner","config":config,"conversation":policy,"model_tape":tape,"test_controls":true,"expected_status":"paused"},
            {"name":"witness","config":config,"login":"marco+Marco","expected_status":"paused"}
        ],"steps":[
            {"actor":"owner","control":"/fault floor-grants"},
            {"actor":"owner","control":"/resume","expect":[{"actor":"owner","kind":"speech-intent"}]},
            {"actor":"owner","control":"/fault disconnect","expect":[{"actor":"owner","kind":"process-eof"}]}
        ]}),
    );
    let directory = fixture.dir.join("disconnect-results");
    let result = output(
        fixture
            .command(&config)
            .arg("--harness")
            .arg(&manifest)
            .arg("--output")
            .arg(&directory),
    )
    .await;
    assert!(!result.status.success());
    let trace = events(&directory.join("run-0001/joined.jsonl"));
    assert!(
        trace
            .iter()
            .any(|e| e["event"]["kind"] == "process-eof" && e["actor"] == "owner")
    );
    assert!(
        !trace
            .iter()
            .any(|e| e["event"]["kind"] == "speech-dispatched")
    );
}
async fn event(bot: &mut PlayerConsole, kind: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut recent = Vec::new();
        while let Some(line) = bot.lines.next_line().await.unwrap() {
            let row: Value = serde_json::from_str(&line).unwrap();
            if row["kind"] == kind {
                return row;
            }
            assert!(
                !["error", "blocked", "control-error"]
                    .iter()
                    .any(|k| row["kind"] == *k),
                "{kind}: {row}"
            );
            recent.push(row);
        }
        panic!("closed before {kind}: {recent:?}");
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {kind}"))
}
async fn state(bot: &mut PlayerConsole) -> Value {
    bot.send("/state").await;
    serde_json::from_str(event(bot, "state").await["data"].as_str().unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn uncued_correction_cancels_held_reply_before_memory_or_send_and_preserves_cost() {
    let fixture = Fixture::boot_world(false, None, true).await;
    let mut pc = PlayerConsole::speaker(&fixture).await;
    let model = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|request: &Request| {
            let context = request_context(request);
            let corrected = context["recent_speech"]
                .to_string()
                .contains("a lily instead");
            assert!(
                !context["memory"]
                    .as_str()
                    .unwrap_or("")
                    .contains("STALE_ROSE_NOTES")
            );
            let command = if corrected {
                "say A lily, understood."
            } else {
                "say A rose, understood."
            };
            let mut d = decision("command", Some(command), "Answer the current request.");
            d["memory"] = json!(if corrected {
                "Remember the lily request."
            } else {
                "STALE_ROSE_NOTES"
            });
            let mut response = reply(d);
            response = response.insert_header("x-fixture", "conversation");
            response
        })
        .expect(2)
        .mount(&model)
        .await;
    let config = conversation_config(&fixture, &model);
    let policy = policy(&fixture, json!({"reply_min_ms":900}));
    let trace = fixture.dir.join("speech.jsonl");
    let mut bot = PlayerConsole::spawn(
        fixture
            .command(&config)
            .args(["--goal", "", "--json-events"])
            .arg("--conversation")
            .arg(&policy)
            .arg("--transcript")
            .arg(&trace),
    );
    event(&mut bot, "ready").await;
    pc.say("Rosebot, please bring a rose?").await;
    event(&mut bot, "speech-prepared").await;
    pc.say("Actually, a lily instead.").await;
    event(&mut bot, "speech-cancelled").await;
    let sent = event(&mut bot, "speech-dispatched").await;
    let snapshot = state(&mut bot).await;
    assert_eq!(snapshot["state"]["memory"], "Remember the lily request.");
    assert_eq!(snapshot["state"]["model_calls"], 2);
    assert_eq!(snapshot["conversation"]["state"]["cancelled_replies"], 1);
    bot.quit().await;
    pc.quit().await;
    let rows = events(&trace);
    assert!(
        !rows
            .iter()
            .any(|e| e["kind"] == "send" && e["data"] == "say A rose, understood.")
    );
    let prepared = rows
        .iter()
        .rev()
        .find(|e| e["kind"] == "speech-prepared")
        .unwrap();
    assert!(sent["elapsed_ms"].as_u64().unwrap() >= prepared["elapsed_ms"].as_u64().unwrap() + 850);
    assert_eq!(
        rows.iter().filter(|e| e["kind"] == "cost-receipt").count(),
        2
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_social_work_is_cancelled_by_uncued_speech_without_request_storm() {
    let fixture = Fixture::boot_world(false, None, true).await;
    let mut pc = PlayerConsole::speaker(&fixture).await;
    let model = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            reply(decision("command", Some("say stale"), "stale"))
                .set_delay(Duration::from_secs(3)),
        )
        .expect(1)
        .mount(&model)
        .await;
    let config = conversation_config(&fixture, &model);
    let policy = policy(&fixture, json!({"max_social_requests":1}));
    let mut bot = PlayerConsole::spawn(
        fixture
            .command(&config)
            .args(["--goal", "", "--json-events"])
            .arg("--conversation")
            .arg(&policy),
    );
    event(&mut bot, "ready").await;
    pc.say("Rosebot, can you answer?").await;
    event(&mut bot, "model-request").await;
    // Request event precedes HTTP dispatch; wait for the actual stub request before correcting.
    tokio::time::timeout(Duration::from_secs(3), async {
        while model.received_requests().await.unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    pc.say("Never mind the earlier question.").await;
    event(&mut bot, "model-cancelled").await;
    tokio::time::sleep(Duration::from_millis(350)).await;
    let snapshot = state(&mut bot).await;
    assert_eq!(snapshot["state"]["model_calls"], 1);
    assert_eq!(
        snapshot["conversation"]["state"]["reason"],
        "social request window exhausted; waiting"
    );
    assert_eq!(snapshot["model_in_flight"], false);
    bot.quit().await;
    pc.quit().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_floor_intent_cannot_be_revived_by_late_grant() {
    let fixture = Fixture::boot_world(false, None, true).await;
    let mut pc = PlayerConsole::speaker(&fixture).await;
    let model = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(reply(decision("command", Some("say Ready."), "Ready")))
        .mount(&model)
        .await;
    let config = conversation_config(&fixture, &model);
    let policy = policy(&fixture, json!({"reply_min_ms":50}));
    let trace = fixture.dir.join("managed.jsonl");
    let mut bot = PlayerConsole::spawn(
        fixture
            .command(&config)
            .args(["--goal", "", "--json-events", "--managed-floor"])
            .arg("--conversation")
            .arg(&policy)
            .arg("--transcript")
            .arg(&trace),
    );
    event(&mut bot, "ready").await;
    pc.say("Rosebot, ready?").await;
    let old = event(&mut bot, "speech-intent").await["data"]["id"]
        .as_u64()
        .unwrap();
    pc.say("One more thing before answering.").await;
    event(&mut bot, "speech-cancelled").await;
    bot.send("/pause").await;
    event(&mut bot, "paused").await;
    bot.send(&format!("/floor grant {old}")).await;
    let ack = event(&mut bot, "speech-complete").await;
    assert_eq!(ack["data"]["id"], old);
    assert_eq!(ack["data"]["outcome"], "already_cancelled");
    assert!(state(&mut bot).await["conversation"]["intent"].is_null());
    bot.quit().await;
    pc.quit().await;
    assert!(
        !events(&trace)
            .iter()
            .any(|e| e["kind"] == "send" && e["data"] == "say Ready.")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_managed_bots_and_pc_take_turns_with_private_context_and_cited_scene_artifacts() {
    let fixture = Fixture::boot_players(false, None, true, &["Ada", "Jun"]).await;
    let model = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|request: &Request| {
            let context = request_context(request);
            let name = context["character_profile"]["name"].as_str().unwrap();
            let body = context.to_string();
            for other in ["Rosebot", "Ada", "Jun"] {
                assert_eq!(body.contains(&format!("PRIVATE_{other}")), other == name);
            }
            reply(decision(
                "command",
                Some(&format!("say Ready from {name}.")),
                "Join the greeting.",
            ))
        })
        .mount(&model)
        .await;
    let config = fixture.config(Some(&model), 20);
    let policy = policy(&fixture, json!({"reply_min_ms":120,"reply_jitter_ms":100}));
    let actors: Vec<_> = ["Rosebot", "Ada", "Jun"].iter().map(|name| {
        let character = write(&fixture, &format!("{name}.json"), json!({"name":name,"background":format!("PRIVATE_{name}")}));
        json!({"name":name,"config":config,"login":format!("{}+{name}",name.to_lowercase()),"character":character,
            "goal":"","conversation":policy,"start":"/listen","expected_status":"listening"})
    }).chain(std::iter::once(json!({"name":"pc","config":config,"login":"marco+Marco","goal":"","expected_status":"paused"}))).collect();
    let recap = fixture.dir.join("scene-recap.json");
    let bookmark = fixture.dir.join("scene-bookmark.json");
    let manifest = write(
        &fixture,
        "cast.json",
        json!({"name":"Conversation foundation","actors":actors,"timeout_ms":30_000,
        "speaking_floor":{"speaking_window_ms":350},
        "steps":[
            {"actor":"pc","control":"/send say What a lovely evening.","expect":[{"actor":"pc","kind":"command-result"}]},
            {"actor":"pc","control":"/send say Rosebot, Ada, Jun, are you ready?","expect":[
                {"actor":"Rosebot","kind":"speech-dispatched"},{"actor":"Ada","kind":"speech-dispatched"},{"actor":"Jun","kind":"speech-dispatched"}]},
            {"actor":"pc","control":"/send look","expect":[{"actor":"pc","kind":"command-result","contains":"Market Square"}]},
            {"actor":"pc","control":format!("/recap {}",recap.display()),"expect":[{"actor":"pc","kind":"scene-recap"}]},
            {"actor":"pc","control":format!("/bookmark {}",bookmark.display()),"expect":[{"actor":"pc","kind":"scene-bookmark"}]}
        ]}),
    );
    let directory = fixture.dir.join("cast-result");
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
        String::from_utf8_lossy(&result.stderr),
        summary,
        directory.display()
    );
    let rows = events(&directory.join("run-0001/joined.jsonl"));
    let sends: Vec<_> = rows
        .iter()
        .filter(|r| r["event"]["kind"] == "speech-dispatched")
        .collect();
    assert_eq!(sends.len(), 3);
    for pair in sends.windows(2) {
        assert!(
            pair[1]["elapsed_ms"].as_u64().unwrap()
                >= pair[0]["elapsed_ms"].as_u64().unwrap() + 350
        );
    }
    let hello = rows
        .iter()
        .position(|r| {
            r["event"]["kind"] == "send"
                && r["event"]["data"]
                    .as_str()
                    .is_some_and(|s| s.contains("are you ready"))
        })
        .unwrap();
    assert!(
        !rows[..hello]
            .iter()
            .any(|r| r["event"]["kind"] == "model-request")
    );
    for row in rows
        .iter()
        .filter(|r| r["event"]["kind"] == "speech-intent")
    {
        assert_eq!(
            row["event"]["data"].as_object().unwrap().len(),
            2,
            "floor metadata has no reply text or private state"
        );
    }
    let recap: mush_bot::scene::Recap =
        serde_json::from_slice(&std::fs::read(&recap).unwrap()).unwrap();
    assert!(recap.incomplete);
    assert_eq!(recap.provider_requests, 0);
    let quotes = recap
        .entries
        .iter()
        .map(|e| e.quote.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for name in ["Rosebot", "Ada", "Jun"] {
        assert!(quotes.contains(&format!("Ready from {name}")), "{quotes}");
    }
    assert!(!quotes.contains("PRIVATE_"));
    let b = mush_bot::scene::Bookmark::load(&bookmark).unwrap();
    assert!(!b.world_restorable);
    assert_eq!(b.modules["runner"].data["state"]["status"], "paused");
    let checked = output(
        fixture
            .command(&config)
            .arg("--check-bookmark")
            .arg(&bookmark),
    )
    .await;
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spectator_observes_addresses_without_a_model_and_recap_excludes_authored_priors() {
    let fixture = Fixture::boot_world(false, None, true).await;
    let mut pc = PlayerConsole::speaker(&fixture).await;
    let model = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(reply(decision("done", None, "Unused")))
        .expect(0)
        .mount(&model)
        .await;
    let config = conversation_config(&fixture, &model);
    let policy = policy(&fixture, json!({"mode":"spectator"}));
    let mut bot = PlayerConsole::spawn(
        fixture
            .command(&config)
            .args(["--goal", "", "--json-events"])
            .arg("--conversation")
            .arg(&policy),
    );
    event(&mut bot, "ready").await;
    bot.send("/remember OFFSCREEN_PRIVATE_PRIOR").await;
    event(&mut bot, "remembered").await;
    bot.send("/listen").await;
    pc.say("Rosebot, our rehearsal is at seven.").await;
    tokio::time::sleep(Duration::from_millis(350)).await;
    let observing = state(&mut bot).await;
    assert_eq!(observing["state"]["status"], "listening");
    assert_eq!(observing["state"]["model_calls"], 0);
    // /send look is an ordinary operator action and waits for fresh output before the recap.
    bot.send("/send look").await;
    event(&mut bot, "command-result").await;
    let recap_path = fixture.dir.join("observer.json");
    bot.send(&format!("/recap {}", recap_path.display())).await;
    event(&mut bot, "scene-recap").await;
    let recap = std::fs::read_to_string(recap_path).unwrap();
    assert!(recap.contains("rehearsal is at seven"));
    assert!(!recap.contains("OFFSCREEN_PRIVATE_PRIOR"));
    assert_eq!(state(&mut bot).await["state"]["model_calls"], 0);
    bot.quit().await;
    pc.quit().await;
}

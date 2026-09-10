use mush_bot::{
    campaign::{self, Checkpoint, Manifest},
    experiment::{load_json, save_json},
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[path = "support/campaign_fixture.rs"]
mod fixture;

#[test]
fn campaign_fixture_process() {
    if let Ok(path) = std::env::var("MUSH_CAMPAIGN_START") {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(fixture::serve(path.into()))
            .unwrap();
    }
}
fn temp() -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let path = std::env::temp_dir().join(format!(
        "mush-campaign-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir(&path).unwrap();
    path
}
fn step(actor: &str, control: &str, kind: Option<&str>) -> Value {
    json!({"actor":actor,"control":control,"wait":kind.map(|k| json!({"kind":k}))})
}
fn setup(dir: &std::path::Path) -> Manifest {
    save_json(&dir.join("tape.json"), &json!({"version":1,"replies":[
        {"content":"{\"action\":\"command\",\"command\":\"say STALE UNSENT REPLY\",\"summary\":\"Staged old response\",\"memory\":\"STALE NOTES\"}"},
        {"content":"{\"action\":\"command\",\"command\":\"pose plays a melodic counterpoint.\",\"summary\":\"Demonstrate a counterpoint\",\"memory\":\"I demonstrated a melodic counterpoint.\"}"},
        {"content":"{\"action\":\"done\",\"summary\":\"The demonstration is complete\"}"}
    ]})).unwrap();
    save_json(
        &dir.join("conversation.json"),
        &json!({"reply_min_ms":0,"reply_jitter_ms":0,"debounce_ms":0}),
    )
    .unwrap();
    std::fs::write(dir.join("actor.toml"), "model_tape = 'tape.json'\nconversation = 'conversation.json'\n[agent]\nsettle_ms = 50\nobservation_ms = 500\nbootstrap = ['help', 'look']\n").unwrap();
    let value = json!({"version":1,"id":"band-campaign","fixture_program":std::env::current_exe().unwrap(),
    "fixture_args":["--exact","campaign_fixture_process","--nocapture"],"max_requests":100,
    "actors":[{"name":"Bassist","config":dir.join("actor.toml"),"request_allowance":4},{"name":"Lead","config":dir.join("actor.toml"),"request_allowance":4},{"name":"Drummer","config":dir.join("actor.toml"),"request_allowance":4}],
    "branches":[
        {"name":"pivotal","steps":[step("Lead","/send say Stick to roots and fifths for now.",Some("command-result")),
            step("Bassist","/goal Find room for melodic bass in this band",None),
            step("Bassist","/remember I have played bass for eight years",None),
            step("Bassist","/step model",Some("staged-decision"))]},
        {"name":"private","parent":"pivotal","interventions":[step("Bassist","/note PRIVATE CHILD INSTRUCTION",None)],"steps":[
            step("Bassist","/send north",Some("command-result")),
            step("Bassist","/send page Lead=PRIVATE DISCUSSION about my counterpoint",Some("command-result"))]},
        {"name":"public","parent":"pivotal","steps":[step("Bassist","/send say PUBLIC CONFRONTATION: I can contribute more than roots.",Some("command-result"))]},
        {"name":"demonstration","parent":"pivotal","steps":[step("Lead","/send say Show us your counterpoint.",Some("command-result")),step("Bassist","/step",Some("command-result"))]},
        {"name":"parent-again","parent":"pivotal","steps":[step("Bassist","/send look",Some("command-result"))]}
    ]});
    let manifest: Manifest = serde_json::from_value(value).unwrap();
    manifest.validate().unwrap();
    manifest
}

#[tokio::test]
async fn three_playable_continuations_keep_shared_history_private_knowledge_and_spend() {
    let dir = temp();
    let manifest = setup(&dir);
    let output = dir.join("run");
    let summary = campaign::run(
        &manifest,
        &output,
        std::path::Path::new(env!("CARGO_BIN_EXE_mush-bot")),
    )
    .await
    .unwrap_or_else(|e| panic!("{e}; evidence: {}", output.display()));
    assert_eq!(summary.branches.len(), 5);
    let parent_path = output.join("pivotal/checkpoint.json");
    let original_bytes = std::fs::read(&parent_path).unwrap();
    let parent = Checkpoint::load(&parent_path).unwrap();
    let private = Checkpoint::load(&output.join("private/checkpoint.json")).unwrap();
    let public = Checkpoint::load(&output.join("public/checkpoint.json")).unwrap();
    let demo = Checkpoint::load(&output.join("demonstration/checkpoint.json")).unwrap();
    let again = Checkpoint::load(&output.join("parent-again/checkpoint.json")).unwrap();
    for checkpoint in [&parent, &private, &public, &demo, &again] {
        for actor in checkpoint.bots.values() {
            let knowledge = serde_json::to_string(&actor.memory).unwrap();
            for protocol in ["__campaign_fence", "CAMPAIGN FROZEN"] {
                assert!(!knowledge.contains(protocol));
                assert!(!actor.state.recent.contains(protocol));
            }
        }
    }
    assert_eq!(parent.bots["Bassist"].usage.requests, 1);
    assert_eq!(private.bots["Bassist"].usage.requests, 1);
    assert_eq!(demo.bots["Bassist"].usage.requests, 2);
    assert_eq!(again.bots["Bassist"].usage.requests, 1);
    assert_eq!(
        again.bots["Bassist"].state.goal,
        parent.bots["Bassist"].state.goal
    );
    assert!(
        private.bots["Bassist"]
            .state
            .instructions
            .contains("PRIVATE CHILD")
    );
    assert!(
        !again.bots["Bassist"]
            .state
            .instructions
            .contains("PRIVATE CHILD")
    );
    assert!(
        serde_json::to_string(&private.bots["Lead"].memory)
            .unwrap()
            .contains("PRIVATE DISCUSSION")
    );
    assert!(
        !serde_json::to_string(&private.bots["Drummer"].memory)
            .unwrap()
            .contains("PRIVATE DISCUSSION")
    );
    assert!(
        serde_json::to_string(&public.bots["Drummer"].memory)
            .unwrap()
            .contains("PUBLIC CONFRONTATION")
    );
    for text in [
        "PRIVATE DISCUSSION",
        "PUBLIC CONFRONTATION",
        "plays a melodic counterpoint",
        "STALE NOTES",
    ] {
        assert!(
            !serde_json::to_string(&again.bots["Bassist"].memory)
                .unwrap()
                .contains(text),
            "leaked {text}"
        );
    }
    assert_eq!(again.world.data, parent.world.data);
    // A pre-wave-3 save omitted newly defaulted fields. Its original immutable
    // parent hash must survive loading, without allowing unbound new state.
    let mut legacy = serde_json::to_value(&parent).unwrap();
    for bot in legacy["bots"].as_object_mut().unwrap().values_mut() {
        bot["version"] = json!(1);
        for field in ["mission", "playtest", "playtest_feedback"] {
            bot.as_object_mut().unwrap().remove(field);
        }
        bot.as_object_mut().unwrap().remove("learning");
        bot.as_object_mut().unwrap().remove("activity");
        bot["context"]
            .as_object_mut()
            .unwrap()
            .remove("learning_bytes");
        // This prefix has no activity execution: only the new final contribution.
        let records = bot["memory"]["records"].as_array_mut().unwrap();
        assert_eq!(records.last().unwrap()["event"]["type"], "activity");
        records.pop();
        bot.as_object_mut().unwrap().remove("social");
        bot["context"]
            .as_object_mut()
            .unwrap()
            .remove("social_bytes");
    }
    legacy["id"] = json!("");
    legacy["id"] = json!(mush_bot::experiment::revision(
        &serde_json::to_vec(&legacy).unwrap()
    ));
    let legacy_path = dir.join("legacy-v1.json");
    save_json(&legacy_path, &legacy).unwrap();
    let mut loaded = Checkpoint::load(&legacy_path).unwrap();
    assert_eq!(loaded.id, legacy["id"]);
    loaded.bots.get_mut("Bassist").unwrap().context.social_bytes = 0;
    assert!(loaded.validate().is_err());
    assert_ne!(private.world.data, parent.world.data);
    assert_eq!(std::fs::read(&parent_path).unwrap(), original_bytes);
    let trace = std::fs::read_to_string(output.join("demonstration/Bassist/trace.jsonl")).unwrap();
    let sent = trace
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|e| e["kind"] == "send")
        .collect::<Vec<_>>();
    assert!(
        !serde_json::to_string(&sent)
            .unwrap()
            .contains("STALE UNSENT REPLY")
    );
    assert!(
        serde_json::to_string(&sent)
            .unwrap()
            .contains("counterpoint")
    );
    let inspection = campaign::inspect(&output.join("summary.json")).unwrap();
    assert_eq!(inspection["mode"], "restored_world_branch");
    assert_eq!(inspection["provider_spend_usd"], "0");
    campaign::select(
        &output.join("summary.json"),
        "demonstration",
        &dir.join("selected.json"),
    )
    .unwrap();
    let selected: Value = load_json(&dir.join("selected.json"), 16000).unwrap();
    assert_eq!(selected["selected"]["id"], demo.id);
    // Continue the chosen world using the original append-only campaign budget.
    // Source files are no longer the saved character's authority after restore.
    std::fs::write(dir.join("actor.toml"), "model_tape = 'deleted-tape.json'\ncharacter = 'deleted-character.json'\nknowledge = ['deleted-wiki.md']\n").unwrap();
    let mut next = manifest.clone();
    next.from_checkpoint = Some(output.join("demonstration/checkpoint.json"));
    next.branches = vec![
        serde_json::from_value(
            json!({"name":"encore","steps":[step("Bassist","/send look",Some("command-result"))]}),
        )
        .unwrap(),
    ];
    let continued = campaign::run(
        &next,
        &dir.join("continued"),
        std::path::Path::new(env!("CARGO_BIN_EXE_mush-bot")),
    )
    .await
    .unwrap();
    assert_eq!(continued.ledger, summary.ledger);
    assert_eq!(continued.branches[0].reserved_requests, 72);
    let encore = Checkpoint::load(&continued.branches[0].checkpoint.path).unwrap();
    assert!(encore.bots["Bassist"].state.memory.contains("demonstrated"));
    // Restoring is not authority to raise/replenish the campaign cap.
    next.max_requests = 200;
    assert!(
        campaign::run(
            &next,
            &dir.join("illegal-cap"),
            std::path::Path::new(env!("CARGO_BIN_EXE_mush-bot"))
        )
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("cap/ledger mismatch")
    );
    let mut broken = serde_json::to_value(&parent).unwrap();
    broken["world"]["capabilities"] = json!("database-only");
    save_json(&dir.join("unsupported.json"), &broken).unwrap();
    assert!(Checkpoint::load(&dir.join("unsupported.json")).is_err());
    let mut broken = serde_json::to_value(&parent).unwrap();
    broken["bots"]["Bassist"]["state"]["goal"] = json!("tampered");
    save_json(&dir.join("tampered.json"), &broken).unwrap();
    assert!(Checkpoint::load(&dir.join("tampered.json")).is_err());
    std::fs::rename(&summary.ledger, dir.join("ledger-preserved.jsonl")).unwrap();
    next.max_requests = 100;
    assert!(
        campaign::run(
            &next,
            &dir.join("missing-ledger"),
            std::path::Path::new(env!("CARGO_BIN_EXE_mush-bot"))
        )
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("ledger is missing")
    );
    assert!(!summary.ledger.exists());
}

#[tokio::test]
async fn quiesce_cancels_inflight_work_and_branch_turn_resets_cannot_bypass_campaign_quota() {
    let dir = temp();
    let mut manifest = setup(&dir);
    let mut tape: Value = load_json(&dir.join("tape.json"), 10000).unwrap();
    tape["replies"][0]["delay_ms"] = json!(10_000);
    std::fs::write(dir.join("tape.json"), serde_json::to_vec(&tape).unwrap()).unwrap();
    manifest.actors.truncate(1);
    manifest.actors[0].request_allowance = 1;
    manifest.max_requests = 2;
    manifest.branches = serde_json::from_value(json!([
        {"name":"pending","steps":[step("Bassist","/goal Play a counterpoint",None),step("Bassist","/step",Some("model-request"))]},
        {"name":"after","parent":"pending","steps":[step("Bassist","/step",Some("command-result")),
            step("Bassist","/goal A brand new goal",None),step("Bassist","/turns 10",None),step("Bassist","/step",Some("blocked"))]}
    ])).unwrap();
    let output = dir.join("run");
    campaign::run(
        &manifest,
        &output,
        std::path::Path::new(env!("CARGO_BIN_EXE_mush-bot")),
    )
    .await
    .unwrap_or_else(|e| panic!("{e}; {}", output.display()));
    let before = Checkpoint::load(&output.join("pending/checkpoint.json")).unwrap();
    let after = Checkpoint::load(&output.join("after/checkpoint.json")).unwrap();
    assert_eq!(before.bots["Bassist"].usage.requests, 1);
    assert_eq!(after.bots["Bassist"].usage.requests, 2);
    assert_eq!(
        before.bots["Bassist"].tape.as_ref().unwrap().replies.len(),
        2
    );
    assert!(
        after.bots["Bassist"]
            .state
            .outcome
            .contains("Campaign reserved request allowance exhausted")
    );
    assert!(after.bots["Bassist"].state.memory.contains("counterpoint"));
    let trace = std::fs::read_to_string(output.join("pending/Bassist/trace.jsonl")).unwrap();
    assert!(trace.contains("model-cancelled"));
    assert!(
        !serde_json::to_string(&after.bots["Bassist"].memory)
            .unwrap()
            .contains("STALE NOTES")
    );
    let mut next = manifest;
    next.from_checkpoint = Some(output.join("pending/checkpoint.json"));
    next.branches = vec![serde_json::from_value(json!({"name":"exhausted"})).unwrap()];
    assert!(
        campaign::run(
            &next,
            &dir.join("over-cap"),
            std::path::Path::new(env!("CARGO_BIN_EXE_mush-bot"))
        )
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("cap would be exceeded")
    );
}

#[tokio::test]
async fn inherited_session_budget_survives_restore_and_goal_change() {
    let dir = temp();
    let mut manifest = setup(&dir);
    save_json(&dir.join("budget.json"), &json!({"max_requests":1})).unwrap();
    let mut config = std::fs::read_to_string(dir.join("actor.toml")).unwrap();
    config.insert_str(0, "budget = 'budget.json'\n");
    std::fs::write(dir.join("actor.toml"), config).unwrap();
    manifest.actors.truncate(1);
    manifest.branches = serde_json::from_value(json!([
        {"name":"one","steps":[step("Bassist","/goal Discuss the band",None),step("Bassist","/step model",Some("staged-decision"))]},
        {"name":"two","parent":"one","steps":[step("Bassist","/goal Different goal",None),step("Bassist","/step",Some("budget-blocked"))]}
    ])).unwrap();
    let output = dir.join("run");
    campaign::run(
        &manifest,
        &output,
        std::path::Path::new(env!("CARGO_BIN_EXE_mush-bot")),
    )
    .await
    .unwrap_or_else(|e| panic!("{e}; {}", output.display()));
    let two = Checkpoint::load(&output.join("two/checkpoint.json")).unwrap();
    assert_eq!(two.bots["Bassist"].usage.requests, 1);
    assert_eq!(two.bots["Bassist"].budget.max_requests, Some(1));
    assert!(two.bots["Bassist"].state.outcome.contains("request"));
}

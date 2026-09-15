use mush_bot::{
    activity,
    campaign::{self, Checkpoint, Manifest},
    config::{Memory, Ssh},
    experiment::save_json,
    learning,
    memory::{Book, Event, Source},
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[path = "support/campaign_fixture.rs"]
mod fixture;
#[test]
fn life_fixture_process() {
    if let Ok(path) = std::env::var("MUSH_CAMPAIGN_START") {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(fixture::serve(path.into()))
            .unwrap();
    }
}
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn book() -> Book {
    Book::open(
        &Memory {
            persistent: false,
            ..Default::default()
        },
        &Ssh {
            username: "Bot".into(),
            ..Default::default()
        },
    )
    .unwrap()
}
fn observe(book: &mut Book, text: &str) -> u64 {
    book.observe(text).unwrap();
    book.flush().unwrap();
    book.records()
        .iter()
        .rev()
        .find(|r| matches!(&r.event,Event::Episode(e) if e.source==Source::Observed))
        .unwrap()
        .id
}
fn room(book: &mut Book, name: &str, id: u64) {
    // The runner marks the server's own room display at connect; a bare
    // header at any other time is somebody else's text and never moves the
    // character, so the fixture's opening room is announced the same way.
    book.expect_arrival();
    observe(
        book,
        &format!("{name}(#{id})\nObvious exits:\nnorth south east west"),
    );
}
fn trip(book: &mut Book, command: &str, name: &str, id: u64) {
    book.sent(command).unwrap();
    room(book, name, id);
    book.finish_command().unwrap();
}
fn temp() -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "bot-life-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&p).unwrap();
    p
}

#[test]
fn map_invalidates_only_observed_failures_and_reestablishes_new_travel() {
    let mut b = book();
    room(&mut b, "Home", 0);
    trip(&mut b, "north", "Shop", 1);
    trip(&mut b, "south", "Home", 0);
    b.sent("north").unwrap();
    observe(&mut b, "Guide says, \"There is no exit that way.\"");
    b.finish_command().unwrap();
    assert_eq!(b.route("Shop")["status"], "remembered_route");
    b.sent("north").unwrap();
    observe(&mut b, "The exit is blocked.");
    b.finish_command().unwrap();
    assert_eq!(b.route("Shop")["status"], "no_remembered_route");
    assert!(
        b.map()
            .edges
            .iter()
            .any(|e| e.status == mush_bot::navigation::Status::Invalidated)
    );
    trip(&mut b, "north", "New Shop", 1);
    trip(&mut b, "south", "Home", 0);
    assert_eq!(b.route("New Shop")["status"], "remembered_route");
    let source = observe(&mut b, "Guide says, \"East may lead to a garden.\"");
    let report = json!({"from":{"dbref":"#0","name":"Home"},"command":"east","to":{"dbref":"#2","name":"Garden"},"status":"reported","reason":"Guide reported it","evidence":[{"episode":source,"quote":"East may lead to a garden."}]});
    assert!(
        b.annotate_navigation(Some(json!([report])))
            .unwrap()
            .is_empty()
    );
    assert_ne!(b.route("Garden")["status"], "remembered_route");
}

#[test]
fn activity_is_quiet_coalesces_windows_and_never_promotes_echo_or_speech_to_work() {
    let mut b = book();
    room(&mut b, "Lounge", 1);
    let plan = activity::Plan::load(&root().join("bots/life/dog-walker.json")).unwrap();
    let mut s = activity::State::default();
    s.install(plan.clone(), b.records()).unwrap();
    s.enabled = true;
    let command = s.choose(&b).unwrap();
    assert_eq!(command, "walk-dogs");
    b.sent(&command).unwrap();
    s.dispatched(b.records().last().unwrap().id);
    observe(
        &mut b,
        "Walker says, \"Work receipt: the dogs have returned from their walk.\"\n                    Work receipt: the dogs have returned from their walk.",
    );
    b.finish_command().unwrap();
    assert!(s.reconcile(b.records(), true));
    assert!(s.suspended);
    assert!(s.completed.is_empty());
    assert!(s.choose(&b).is_none());
    s.suspended = false;
    s.advance(100).unwrap();
    let command = s.choose(&b).unwrap();
    b.sent(&command).unwrap();
    s.dispatched(b.records().last().unwrap().id);
    observe(
        &mut b,
        "Work receipt: the dogs have returned from their walk.",
    );
    b.finish_command().unwrap();
    assert!(s.reconcile(b.records(), true));
    assert_eq!(s.completed["work"].cycle, 0);
    assert_eq!(s.actions, 2);
    assert_eq!(s.unsolicited_speech, 0);
    let decisions = s.decisions;
    for _ in 0..100 {
        s.advance(100).unwrap();
        assert!(s.choose(&b).is_none());
    }
    assert_eq!(s.decisions, decisions);
    s.advance(plan.period_ms * 9).unwrap();
    let command = s.choose(&b).unwrap();
    assert_eq!(command, "walk-dogs");
    assert_eq!(s.active.as_ref().unwrap().cycle, 9);
    let old_actions = s.actions;
    s.cancel_unsent();
    s.enabled = false;
    assert!(s.choose(&b).is_none());
    assert_eq!(s.actions, old_actions);
    let mut updated = plan;
    updated.procedures[0].steps[0].command = "different-work".into();
    assert!(s.install(updated.clone(), b.records()).is_err());
    updated.procedures[0].revision = 2;
    s.install(updated, b.records()).unwrap();
    assert_eq!(s.actions, old_actions);
    assert!(s.active.is_none());
    assert_eq!(s.history.len(), 1);
}

fn lesson(
    id: &str,
    participants: (&str, &str),
    phase: &str,
    revision: u32,
    content: &str,
    episode: u64,
    quote: &str,
) -> Value {
    let (teacher, student) = participants;
    json!({"type":"lesson","id":id,"teacher":{"name":teacher,"kind":if teacher=="Bot"{"external_bot"}else{"pc"}},"student":{"name":student,"kind":if student=="Bot"{"external_bot"}else{"pc"}},"topic":"guitar counterpoint","phase":phase,"revision":revision,"content":content,"evidence":[{"episode":episode,"quote":quote}]})
}
#[test]
fn corrections_control_transfer_and_growth_cannot_take_over_pcs() {
    let mut b = book();
    room(&mut b, "Lesson", 0);
    for (phase, rev, actor, text) in [
        ("assess", 1, "PC", "Let's hear the pattern."),
        ("demonstrate", 1, "PC", "Keep every note ringing."),
        ("practice", 1, "Bot", "I try letting every note ring."),
        (
            "feedback",
            2,
            "PC",
            "Correction: mute the open string before changing position.",
        ),
        (
            "practice",
            2,
            "Bot",
            "I mute the open string before moving.",
        ),
        (
            "feedback",
            2,
            "PC",
            "Good. Keep muting before position changes.",
        ),
    ] {
        let quote = format!("{actor} says, \"{text}\"");
        let ep = observe(&mut b, &quote);
        let update = lesson("guitar", ("PC", "Bot"), phase, rev, text, ep, &quote);
        assert!(
            b.annotate_learning(Some(json!([update])), "Bot")
                .unwrap()
                .is_empty()
        );
    }
    let quote = "Bot plays a new counterpoint, muting the open string before changing position.";
    let ep = observe(&mut b, quote);
    assert!(
        !b.annotate_learning(
            Some(json!([lesson(
                "guitar",
                ("PC", "Bot"),
                "apply",
                2,
                "Transfer",
                ep,
                quote
            )])),
            "Bot"
        )
        .unwrap()
        .is_empty()
    );
    trip(&mut b, "north", "Stage", 1);
    let ep = observe(&mut b, quote);
    assert!(
        b.annotate_learning(
            Some(json!([lesson(
                "guitar",
                ("PC", "Bot"),
                "apply",
                2,
                "Transfer",
                ep,
                quote
            )])),
            "Bot"
        )
        .unwrap()
        .is_empty()
    );
    let state = b.learning("Bot").unwrap();
    assert_eq!(state.lessons["guitar"].history.len(), 7);
    assert_eq!(state.lessons["guitar"].revision, 2);
    assert!(
        state
            .context("guitar", 16000)
            .to_string()
            .contains("mute the open string")
    );
    b.learning_policy(
        learning::Policy {
            mode: learning::Mode::Automatic,
            allowed_keys: vec!["confidence.guitar".into()],
            max_changes: 1,
            ..Default::default()
        },
        "Bot",
    )
    .unwrap();
    let growth = |name: &str, kind: &str, value: i16| json!({"type":"growth","subject":{"name":name,"kind":kind},"key":"confidence.guitar","value":value,"rationale":"New exercise demonstrated","evidence":[{"episode":ep,"quote":quote}]});
    assert!(
        b.annotate_learning(
            Some(json!([
                growth("PC", "pc", 10),
                growth("Bot", "external_bot", 10),
                growth("Bot", "external_bot", 20)
            ])),
            "Bot"
        )
        .unwrap()
        .is_empty()
    );
    let state = b.learning("Bot").unwrap();
    assert_eq!(state.overlay["confidence.guitar"], 10);
    assert_eq!(
        b.learning("account+Bot").unwrap().overlay["confidence.guitar"],
        10
    );
    assert_eq!(state.proposals[0].status, "suggested");
    assert_eq!(state.proposals[1].status, "applied");
    assert_eq!(state.proposals[2].status, "suggested");
    b.sent("pose invents a lesson.").unwrap();
    let sent = b.records().last().unwrap().id;
    let update = lesson(
        "fake",
        ("Bot", "PC"),
        "assess",
        1,
        "Made up",
        sent,
        "pose invents a lesson.",
    );
    assert!(
        !b.annotate_learning(Some(json!([update])), "Bot")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn interrupted_receipts_reconcile_but_unsettled_reconnects_never_replay() {
    let mut b = book();
    room(&mut b, "Lounge", 1);
    let mut s = activity::State::default();
    s.install(
        activity::Plan::load(&root().join("bots/life/dog-walker.json")).unwrap(),
        b.records(),
    )
    .unwrap();
    s.enabled = true;
    let command = s.choose(&b).unwrap();
    b.sent(&command).unwrap();
    s.dispatched(b.records().last().unwrap().id);
    let mut reconnect = s.clone();
    reconnect.reconnect();
    assert!(reconnect.suspended);
    assert!(reconnect.pending.is_none());
    assert!(reconnect.choose(&b).is_none());
    assert_eq!(reconnect.actions, 1);
    s.enabled = false;
    observe(
        &mut b,
        "Work receipt: the dogs have returned from their walk.",
    );
    b.finish_command().unwrap();
    assert!(s.reconcile(b.records(), true));
    assert_eq!(s.completed.len(), 1);
    b.save_activity(&s).unwrap();
    let saved = b.activity().unwrap();
    assert_eq!(saved.actions, 1);
    assert_eq!(
        saved.completed["work"].receipts,
        s.completed["work"].receipts
    );
    let mut data = serde_json::to_value(b.checkpoint().unwrap()).unwrap();
    data["records"].as_array_mut().unwrap().last_mut().unwrap()["event"]["data"]["version"] =
        json!(999);
    let mut restored = book();
    restored
        .restore(serde_json::from_value(data).unwrap())
        .unwrap();
    assert!(restored.activity().is_err());
}

#[test]
fn activity_budgets_and_prerequisites_do_not_reset_on_reload() {
    let mut b = book();
    room(&mut b, "Lounge", 1);
    let mut plan = activity::Plan::load(&root().join("bots/life/dog-walker.json")).unwrap();
    plan.limits.max_actions = 0;
    let mut s = activity::State::default();
    s.install(plan.clone(), b.records()).unwrap();
    s.enabled = true;
    assert!(s.choose(&b).is_none());
    assert!(s.suspended);
    assert_eq!(s.actions, 0);
    let mut cycle = plan.clone();
    cycle.routines[0].requires = vec!["study".into()];
    assert!(cycle.validate().is_err());
    plan.limits.max_actions = 5;
    plan.procedures[0].steps[0].command = "say Daily announcement".into();
    plan.procedures[0].steps[0].expect = vec!["Bot says, \"Daily announcement\"".into()];
    plan.procedures[0].revision = 2;
    s.install(plan.clone(), b.records()).unwrap();
    s.enabled = true;
    assert!(s.choose(&b).is_none());
    assert!(s.reason.contains("speech allowance"));
    plan.limits.max_unsolicited_speech = 1;
    s.install(plan, b.records()).unwrap();
    s.enabled = true;
    s.advance(100).unwrap();
    let command = s.choose(&b).unwrap();
    b.sent(&command).unwrap();
    s.dispatched(b.records().last().unwrap().id);
    assert_eq!(s.unsolicited_speech, 1);
    assert_eq!(s.actions, 1);
    let mut reopened = s.clone();
    reopened.reconnect();
    assert_eq!(reopened.unsolicited_speech, 1);
    assert_eq!(reopened.actions, 1);
}

#[test]
fn coalesced_lesson_observations_preserve_order_and_frozen_policy_keeps_suggestions_separate() {
    let mut b = book();
    room(&mut b, "Lesson", 0);
    let first = "PC says, \"Show me the pattern.\"";
    let second = "PC says, \"Mute the open string before a position change.\"";
    let ep = observe(&mut b, &format!("{first}\n{second}"));
    let updates = json!([
        lesson("one", ("PC", "Bot"), "assess", 1, "Assess", ep, first),
        lesson(
            "one",
            ("PC", "Bot"),
            "demonstrate",
            1,
            "Mute first",
            ep,
            second
        )
    ]);
    assert!(
        b.annotate_learning(Some(updates), "Bot")
            .unwrap()
            .is_empty()
    );
    // The same evidence cannot be recycled for a later phase.
    assert!(
        !b.annotate_learning(
            Some(json!([lesson(
                "one",
                ("PC", "Bot"),
                "practice",
                1,
                "Recycled",
                ep,
                first
            )])),
            "Bot"
        )
        .unwrap()
        .is_empty()
    );
    let change = json!({"type":"growth","subject":{"name":"Bot","kind":"external_bot"},"key":"confidence.guitar","value":10,"rationale":"An encouraging lesson","evidence":[{"episode":ep,"quote":second}]});
    assert!(
        b.annotate_learning(Some(json!([change.clone()])), "Bot")
            .unwrap()
            .is_empty()
    );
    assert_eq!(b.learning("Bot").unwrap().proposals[0].status, "frozen");
    b.learning_policy(
        learning::Policy {
            mode: learning::Mode::Suggest,
            ..Default::default()
        },
        "Bot",
    )
    .unwrap();
    assert!(
        b.annotate_learning(Some(json!([change])), "Bot")
            .unwrap()
            .is_empty()
    );
    let state = b.learning("Bot").unwrap();
    assert_eq!(state.proposals[1].status, "suggested");
    assert!(state.overlay.is_empty());
    let decision=mush_bot::model::Decision::parse(r#"{"action":"command","command":"look","summary":"Observe","learning":"invalid auxiliary shape"}"#).unwrap();
    assert_eq!(decision.command.as_deref(), Some("look"));
    assert!(
        !b.annotate_learning(decision.learning, "Bot")
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn quiet_work_study_cycle_uses_ssh_and_restores_receipts_without_replaying_work() {
    let dir = temp();
    let mut manifest = Manifest::load(&root().join("bots/life/campaign.json")).unwrap();
    manifest.fixture_program = std::env::current_exe().unwrap();
    manifest.fixture_args = vec![
        "--exact".into(),
        "life_fixture_process".into(),
        "--nocapture".into(),
    ];
    let output = dir.join("run");
    let result = campaign::run(
        &manifest,
        &output,
        Path::new(env!("CARGO_BIN_EXE_mush-bot")),
    )
    .await
    .unwrap_or_else(|e| panic!("{e}: {}", output.display()));
    assert_eq!(result.provider_spend_usd, "0");
    let day = Checkpoint::load(&output.join("day/checkpoint.json")).unwrap();
    let restored = Checkpoint::load(&output.join("restored/checkpoint.json")).unwrap();
    let state = day.bots["Walker"].activity.as_ref().unwrap();
    assert_eq!(state.completed.len(), 2);
    assert_eq!(state.actions, 4);
    assert_eq!(state.unsolicited_speech, 0);
    assert_eq!(
        restored.bots["Walker"].activity.as_ref().unwrap().actions,
        4
    );
    assert_eq!(
        day.bots["Walker"].usage.requests, 1,
        "only the direct engagement should call the model"
    );
    assert_eq!(day.bots["PC"].usage.requests, 0);
    let trace = std::fs::read_to_string(output.join("day/Walker/trace.jsonl")).unwrap();
    assert!(!trace.contains("memory-feedback"));
    assert!(!trace.contains("control-error"));
    let map = mush_bot::navigation::Map::project(&day.bots["Walker"].memory.records);
    assert!(
        map.edges
            .iter()
            .any(|e| e.command == "north" && e.status == mush_bot::navigation::Status::Invalidated)
    );
    let mut bad = serde_json::to_value(&day.bots["Walker"]).unwrap();
    bad.as_object_mut().unwrap().remove("activity");
    assert!(
        serde_json::from_value::<mush_bot::checkpoint::Snapshot>(bad)
            .unwrap()
            .validate()
            .is_err()
    );
}

fn find_episode(save: &Checkpoint, actor: &str, quote: &str) -> u64 {
    save.bots[actor].memory.records.iter().find(|r|matches!(&r.event,Event::Episode(e) if e.source==Source::Observed&&e.text.contains(quote))).unwrap().id
}
fn campaign_step(actor: &str, control: String, kind: &str) -> Value {
    json!({"actor":actor,"control":control,"wait":{"kind":kind}})
}
fn transcript(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
#[tokio::test]
async fn pc_and_bot_teach_both_directions_and_corrected_knowledge_reaches_later_model_actions() {
    let dir = temp();
    let mut manifest = Manifest::load(&root().join("bots/life/guitar.json")).unwrap();
    manifest.fixture_program = std::env::current_exe().unwrap();
    manifest.fixture_args = vec![
        "--exact".into(),
        "life_fixture_process".into(),
        "--nocapture".into(),
    ];
    let output = dir.join("lessons");
    campaign::run(
        &manifest,
        &output,
        Path::new(env!("CARGO_BIN_EXE_mush-bot")),
    )
    .await
    .unwrap_or_else(|e| panic!("{e}: {}", output.display()));
    let prefix = Checkpoint::load(&output.join("bot-teaches/checkpoint.json")).unwrap();
    let mut annotations = vec![];
    for (phase, rev, actor, text) in [
        ("assess", 1, "PC", "says, \"Show me your counterpoint.\""),
        (
            "demonstrate",
            1,
            "PC",
            "says, \"Let the open string ring as you change position.\"",
        ),
        (
            "practice",
            1,
            "Bot",
            "tries changing position with the open string ringing.",
        ),
        (
            "feedback",
            2,
            "PC",
            "says, \"Correction: mute the open string before changing position.\"",
        ),
        (
            "practice",
            2,
            "Bot",
            "mutes the open string before changing position.",
        ),
        (
            "feedback",
            2,
            "PC",
            "says, \"Good. Your position change is clean now.\"",
        ),
    ] {
        let quote = format!("{actor} {text}");
        annotations.push(lesson(
            "pc-lesson",
            ("PC", "Bot"),
            phase,
            rev,
            text,
            find_episode(&prefix, "Bot", &quote),
            &quote,
        ));
    }
    let mut reverse = vec![];
    for (phase, actor, text) in [
        (
            "assess",
            "Bot",
            "says, \"Play your first counterpoint for me.\"",
        ),
        (
            "demonstrate",
            "Bot",
            "says, \"Mute the open string before changing position.\"",
        ),
        (
            "practice",
            "PC",
            "mutes the open string and changes position.",
        ),
        (
            "feedback",
            "Bot",
            "says, \"Good. The counterpoint is clearer.\"",
        ),
    ] {
        let quote = format!("{actor} {text}");
        reverse.push(lesson(
            "bot-lesson",
            ("Bot", "PC"),
            phase,
            1,
            text,
            find_episode(&prefix, "Bot", &quote),
            &quote,
        ));
    }
    let tape = dir.join("learned.json");
    save_json(&tape,&json!({"version":1,"replies":[
        {"content":json!({"action":"done","summary":"Record observed corrected lesson.","learning":annotations}).to_string()},
        {"content":json!({"action":"done","summary":"Keep a PC lesson record without changing the PC.","learning":reverse}).to_string()},
        {"content":json!({"action":"command","command":"pose plays a new counterpoint, muting the open string before changing position.","summary":"Apply the corrected technique in a different room."}).to_string()},
        {"content":json!({"action":"command","command":"say I learned to mute the open string before changing position. Try it in your new melody.","summary":"Teach the corrected technique back to the player."}).to_string()}
    ]})).unwrap();
    let mut next = manifest.clone();
    next.from_checkpoint = Some(output.join("bot-teaches/checkpoint.json"));
    next.branches=serde_json::from_value(json!([{"name":"applied","steps":[
        {"actor":"Bot","control":"/goal Learn and teach the corrected guitar counterpoint"},
        campaign_step("Bot",format!("/tape {}",tape.display()),"tape-changed"),
        campaign_step("Bot","/step".into(),"decision"),campaign_step("Bot","/step".into(),"decision"),
        campaign_step("Bot","/send north".into(),"command-result"),campaign_step("PC","/send north".into(),"command-result"),
        campaign_step("Bot","/step".into(),"command-result"),campaign_step("Bot","/step".into(),"command-result")]}])).unwrap();
    let applied_dir = dir.join("application");
    campaign::run(
        &next,
        &applied_dir,
        Path::new(env!("CARGO_BIN_EXE_mush-bot")),
    )
    .await
    .unwrap_or_else(|e| panic!("{e}: {}", applied_dir.display()));
    let applied = Checkpoint::load(&applied_dir.join("applied/checkpoint.json")).unwrap();
    let state = applied.bots["Bot"].learning.as_ref().unwrap();
    assert_eq!(state.lessons.len(), 2);
    assert_eq!(state.lessons["pc-lesson"].revision, 2);
    assert!(
        state.lessons["pc-lesson"]
            .current_instruction
            .contains("Correction: mute")
    );
    assert!(
        applied.bots["PC"]
            .learning
            .as_ref()
            .unwrap()
            .lessons
            .is_empty()
    );
    assert!(
        applied.bots["PC"]
            .learning
            .as_ref()
            .unwrap()
            .overlay
            .is_empty()
    );
    let trace = transcript(&applied_dir.join("applied/Bot/trace.jsonl"));
    assert!(
        !trace
            .iter()
            .any(|e| e["kind"] == "memory-feedback" || e["kind"] == "control-error")
    );
    let requests: Vec<_> = trace
        .iter()
        .filter(|e| e["kind"] == "model-input")
        .collect();
    assert_eq!(requests.len(), 4);
    // A recall query ensures the relevant corrected lesson is supplied even when
    // the current user goal is empty and incidental room output has no topic words.
    assert!(
        requests[2].to_string().contains("Correction: mute"),
        "{}",
        requests[2]
    );
    let quote = "Bot plays a new counterpoint, muting the open string before changing position.";
    let transfer = lesson(
        "pc-lesson",
        ("PC", "Bot"),
        "apply",
        2,
        "Applied in another room",
        find_episode(&applied, "Bot", quote),
        quote,
    );
    let final_tape = dir.join("transfer.json");
    save_json(&final_tape,&json!({"version":1,"replies":[{"content":json!({"action":"done","summary":"Record transfer evidence.","learning":[transfer]}).to_string()}]})).unwrap();
    next.from_checkpoint = Some(applied_dir.join("applied/checkpoint.json"));
    next.branches=serde_json::from_value(json!([{"name":"transfer","steps":[campaign_step("Bot",format!("/tape {}",final_tape.display()),"tape-changed"),campaign_step("Bot","/step".into(),"decision")]}])).unwrap();
    let final_dir = dir.join("transfer");
    campaign::run(&next, &final_dir, Path::new(env!("CARGO_BIN_EXE_mush-bot")))
        .await
        .unwrap();
    let saved = Checkpoint::load(&final_dir.join("transfer/checkpoint.json")).unwrap();
    assert_eq!(
        saved.bots["Bot"].learning.as_ref().unwrap().lessons["pc-lesson"]
            .application_places
            .len(),
        1
    );
    assert_eq!(
        serde_json::to_value(&saved.bots["PC"].character).unwrap(),
        serde_json::to_value(&prefix.bots["PC"].character).unwrap()
    );
    // No fixture skill system or hidden NPC state is touched by lesson prose.
    assert!(!saved.world.data.to_string().contains("guitar_skill"));
}

//! **The arena commander's call, offline** (cut AJ.3): a commander built from a
//! TOML config's `[arena.commander_model]` against a `wiremock` HTTP stub, and
//! against a local model tape. No game server, no model API: every answer here
//! is the stub's or the tape's. (The commander flying over a real world is
//! cut AJ.4's.)
//!
//! What these hold: the request carries exactly the model's `reasoning` and
//! `provider` objects, `"stream": false` and the commander's own system prompt;
//! the answer reads as a `Doctrine`; the cost ledger files the call under
//! `arena_commander` with the stub's `usage.cost`; and a tape shared by the
//! commander's client and the goal loop's is consumed in admission order, each
//! call answered by the entry it reserved however late it runs. (That the
//! runner itself builds the commander's client on the session's tape is
//! `commander::brain`'s, tested beside it; the runner reserving at admission is
//! proved end to end by cut AJ.4.)
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use mush_bot::{
    arena::{
        Doctrine, Picture, classify,
        commander::{self, COMMANDER_POLICY, Sense, Stance},
    },
    config::Config,
    costs::{Ledger, PromptType},
    model::Brain,
    tape::{Entry, Source, Tape},
};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

/// A fresh directory for one test's files.
fn directory() -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let directory = std::env::temp_dir().join(format!(
        "mush-bot-arena-commander-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).expect("directory");
    directory
}

/// A config whose `[arena.commander_model]` points at `endpoint`, routed the
/// way the programme lead measured: reasoning off, pinned to one provider.
fn config(endpoint: &str) -> Config {
    let directory = directory();
    let path = directory.join("bot.toml");
    std::fs::write(
        &path,
        format!(
            "[ssh]\nusername = \"pilot+Pilot\"\n\
             [memory]\npersistent = false\n\
             [costs]\npersistent = false\n\
             [arena]\ncommander_ms = 1000\n\
             [arena.commander_model]\n\
             endpoint = \"{endpoint}\"\n\
             name = \"deepseek/deepseek-v4.1-flash\"\n\
             max_tokens = 200\n\
             cost_currency = \"USD\"\n\
             reasoning = {{ enabled = false }}\n\
             provider = {{ order = [\"Together\"], allow_fallbacks = false }}\n"
        ),
    )
    .expect("config");
    Config::load(&path).expect("a valid config")
}

/// A picture a seated pilot could hold: its own tank and one hostile.
fn picture() -> Picture {
    let mut picture = Picture::default();
    for line in [
        "Callsign RA.",
        "Red Tank 1: power online, 30 kph, heading 090°",
        "  at 60 m east, 970 m north; scope holds a hostile",
        "  hull  intact  internals 60/60",
        "    front 400/400  left 250/250  right 250/250  rear 150/150",
    ] {
        picture.apply(1_000, &classify(line));
    }
    picture.settle(1_200);
    picture.apply(1_200, &classify("Power online."));
    picture.apply(
        1_300,
        &classify("BA  blue  bearing 000° (rel 270°)  far  450 m  1 s"),
    );
    picture.settle(1_500);
    picture
}

const DOCTRINE: &str =
    r#"{"target":"BA","stance":"stand_off","band_m":500,"sense":"ccw","note":"face it"}"#;

#[tokio::test]
async fn a_commander_call_carries_its_routing_and_is_filed_under_arena_commander() {
    let stub = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "gen-commander", "model": "deepseek/deepseek-v4.1-flash", "provider": "Together",
            "choices": [{"message": {"content": format!("```json\n{DOCTRINE}\n```")},
                "finish_reason": "stop"}],
            "usage": {"cost": 0.00042, "prompt_tokens": 400, "completion_tokens": 37,
                "total_tokens": 437}
        })))
        .expect(1)
        .mount(&stub)
        .await;
    let config = config(&format!("{}/v1/chat/completions", stub.uri()));
    // No `[model]`: the commander is the table's, recorded as `arena_commander`.
    let resolved = commander::resolve(&config.arena, None).expect("a commander");
    assert!(resolved.from_table);
    assert_eq!(resolved.selection.profile, "arena_commander");
    assert_eq!(
        commander::start_line(Some(&resolved)),
        "Commander: deepseek/deepseek-v4.1-flash from [arena.commander_model]."
    );
    let model = resolved.selection.settings.clone();
    let brain = Brain::with_tape(&model, None).expect("a client");
    let view = picture().commander_view(
        4_000,
        &Doctrine::pilot_default(&mush_bot::arena::settings::Settings::default()),
    );
    let capture = commander::capture(&model, &view).expect("a request");
    let costs = Arc::new(Mutex::new(
        Ledger::open(&config.costs, &config.ssh, &config.memory.namespace).expect("a ledger"),
    ));
    let request = commander::admit(&costs, resolved.selection.clone(), &capture).expect("admitted");
    let reply = commander::call(brain, capture.clone(), costs.clone(), request.id)
        .await
        .expect("a reply");
    assert_eq!(reply.error, None);

    // The request: exactly these keys, the two objects verbatim, the policy.
    let requests = stub.received_requests().await.expect("recorded");
    assert_eq!(requests.len(), 1);
    let body: Value = requests[0].body_json().expect("json");
    let keys: BTreeSet<&str> = body
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        BTreeSet::from([
            "max_tokens",
            "messages",
            "model",
            "provider",
            "reasoning",
            "stream"
        ])
    );
    assert_eq!(body["reasoning"], json!({"enabled": false}));
    assert_eq!(
        body["provider"],
        json!({"order": ["Together"], "allow_fallbacks": false})
    );
    assert_eq!(body["stream"], false);
    assert_eq!(body["model"], "deepseek/deepseek-v4.1-flash");
    assert_eq!(body["max_tokens"], 200);
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][0]["content"], COMMANDER_POLICY);
    let sent: Value = serde_json::from_str(body["messages"][1]["content"].as_str().expect("text"))
        .expect("the picture as JSON");
    assert_eq!(sent, view);
    assert_eq!(sent["contacts"][0]["d"], "BA");
    assert_eq!(
        body, capture.payload,
        "the recorded request is the one sent"
    );

    // The answer is a doctrine.
    let doctrine = Doctrine::parse(&reply.content).expect("a doctrine");
    assert_eq!(doctrine.target.as_deref(), Some("BA"));
    assert_eq!(doctrine.stance, Stance::StandOff);
    assert_eq!(doctrine.band_m, 500);
    assert_eq!(doctrine.sense, Sense::Ccw);

    // The ledger: one `arena_commander` request, priced by the stub.
    let report = serde_json::to_value(costs.lock().expect("ledger").report().expect("report"))
        .expect("json");
    let commander = &report["session"]["by_prompt_type"]["arena_commander"];
    assert_eq!(commander["requests"], 1, "{report}");
    assert_eq!(commander["responses"], 1);
    assert_eq!(commander["provider_cost_by_currency"]["USD"], "0.00042");
    assert_eq!(report["session"]["totals"]["requests"], 1);
    assert!(report["session"]["by_prompt_type"]["goal_decision"].is_null());
    assert_eq!(
        report["session"]["by_profile"]["arena_commander"]["requests"],
        1
    );
    // The request's recorded model carries the routing, so its revision does.
    let stored = serde_json::to_value(&request).expect("json");
    assert_eq!(stored["prompt_type"], "arena_commander");
    assert_eq!(
        stored["model"]["settings"]["reasoning"],
        json!({"enabled": false})
    );
}

#[tokio::test]
async fn an_error_status_is_an_error_reply_and_still_filed() {
    let stub = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "error": {"message": "rate limited"}, "usage": {"cost": 0}
        })))
        .expect(1)
        .mount(&stub)
        .await;
    let config = config(&format!("{}/v1/chat/completions", stub.uri()));
    let resolved = commander::resolve(&config.arena, None).expect("a commander");
    let model = resolved.selection.settings.clone();
    let brain = Brain::with_tape(&model, None).expect("a client");
    let view = picture().commander_view(4_000, &Doctrine::pilot_default(&config.arena));
    let capture = commander::capture(&model, &view).expect("a request");
    let costs = Arc::new(Mutex::new(
        Ledger::open(&config.costs, &config.ssh, &config.memory.namespace).expect("a ledger"),
    ));
    let request = commander::admit(&costs, resolved.selection, &capture).expect("admitted");
    let reply = commander::call(brain, capture, costs.clone(), request.id)
        .await
        .expect("a reply");
    let error = reply.error.expect("an error");
    assert!(error.contains("429"), "{error}");
    let report = serde_json::to_value(costs.lock().expect("ledger").report().expect("report"))
        .expect("json");
    assert_eq!(
        report["session"]["by_prompt_type"]["arena_commander"]["errors"],
        1
    );
}

fn entry(content: &str) -> Entry {
    Entry {
        delay_ms: 0,
        content: Some(content.into()),
        error: None,
    }
}

#[tokio::test]
async fn a_tape_shared_with_the_goal_loop_is_consumed_in_admission_order() {
    // The session's one tape, shared by the goal loop's client (the session
    // model, `Brain::prepare`) and the commander's (`commander::brain`, as the
    // runner builds it), each entry reserved at admission.
    let tape = Arc::new(Tape::new(Source {
        version: 1,
        replies: vec![
            entry(r#"{"target":null,"stance":"orbit","band_m":450,"sense":"auto"}"#),
            entry(r#"{"action":"wait","summary":"The goal loop's turn.","wait_ms":1000}"#),
            entry(r#"{"target":"BA","stance":"close","band_m":2000,"sense":"cw"}"#),
        ],
    }));
    let config = config("http://127.0.0.1:1/v1/chat/completions");
    let resolved = commander::resolve(&config.arena, None).expect("a commander");
    let model = resolved.selection.settings.clone();
    let commander_brain = commander::brain(&resolved, Some(tape.clone())).expect("a client");
    let session_model: mush_bot::config::Model = serde_json::from_value(json!({
        "endpoint": "http://127.0.0.1:1/v1/chat/completions", "name": "session-model"}))
    .expect("a session model");
    let goal_brain = Brain::with_tape(&session_model, Some(tape.clone())).expect("a client");
    let costs = Arc::new(Mutex::new(
        Ledger::open(&config.costs, &config.ssh, &config.memory.namespace).expect("a ledger"),
    ));
    let view = picture().commander_view(4_000, &Doctrine::pilot_default(&config.arena));

    // Admitted in this order: commander, goal loop, commander.
    let mut first = commander_brain.clone();
    let capture = commander::capture(&model, &view).expect("a request");
    let request = commander::admit(&costs, resolved.selection.clone(), &capture).expect("admitted");
    first.reserve_tape().expect("reserved");
    let first = tokio::spawn(commander::call(first, capture, costs.clone(), request.id));

    let mut goal = goal_brain.clone();
    let goal_capture = goal
        .prepare(
            json!({"goal": "fight in the simulator"}),
            None,
            &mush_bot::experiment::Prompts::default(),
        )
        .expect("a goal request");
    let goal_request = costs
        .lock()
        .expect("ledger")
        .start(
            mush_bot::models::Selection {
                profile: "configured".into(),
                settings: session_model.clone(),
            },
            PromptType::GoalDecision,
            goal_capture.prompt_revision.clone(),
        )
        .expect("admitted");
    goal.reserve_tape().expect("reserved");

    let mut third = commander_brain.clone();
    let capture = commander::capture(&model, &view).expect("a request");
    let request = commander::admit(&costs, resolved.selection.clone(), &capture).expect("admitted");
    third.reserve_tape().expect("reserved");
    // Every entry is claimed at admission, before any call has run.
    assert!(tape.snapshot().expect("snapshot").replies.is_empty());

    // Awaited in reverse: each still gets the entry it reserved.
    let third = commander::call(third, capture, costs.clone(), request.id)
        .await
        .expect("a reply");
    let doctrine = Doctrine::parse(&third.content).expect("a doctrine");
    assert_eq!(doctrine.stance, Stance::Close);
    assert_eq!(doctrine.band_m, 900, "clamped");
    let reply = goal.dispatch(&goal_capture).await.expect("a reply");
    costs
        .lock()
        .expect("ledger")
        .finish(
            goal_request.id,
            mush_bot::costs::Outcome::Response,
            Some(reply.receipt.clone()),
            0,
        )
        .expect("finished");
    assert!(reply.content.contains("The goal loop's turn."));
    mush_bot::model::Decision::parse(&reply.content).expect("a goal decision");
    let first = first.await.expect("joined").expect("a reply");
    assert_eq!(
        Doctrine::parse(&first.content).expect("a doctrine").stance,
        Stance::Orbit
    );

    // The ledger files each under its own purpose.
    let report = serde_json::to_value(costs.lock().expect("ledger").report().expect("report"))
        .expect("json");
    let by_type = &report["session"]["by_prompt_type"];
    assert_eq!(by_type["arena_commander"]["requests"], 2, "{report}");
    assert_eq!(by_type["goal_decision"]["requests"], 1, "{report}");
}

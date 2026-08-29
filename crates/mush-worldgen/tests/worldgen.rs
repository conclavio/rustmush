//! Integration tests for the worldgen module: themes, the `@gen` draft
//! pipeline, the async defer/re-entry path, and persistence — all against a
//! network-free [`MockProvider`]. No real LLM, no sockets.

use std::sync::Arc;

use mush_core::{
    Audience, Effect, Effects, GOD, Inbound, LIMBO, ModuleOp, ModuleWrite, ObjId, ObjKind, Outbox,
    Powers, WorldCommand,
};
use mush_llm::{
    BudgetConfig, BudgetLimit, CompletionResponse, ContentBlock, LlmProvider, LlmService,
    MockProvider, ProviderCaps, RouteConfig, ScopeLimits, StopReason, Usage,
};
use mush_proto::OutputEvent;
use mush_testkit::TestWorld;
use mush_worldgen::{GenOutcome, GenSnapshot, StoredDraft, WorldgenModule, run_worldgen_async};
use serde_json::json;

// ----------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------

fn route() -> RouteConfig {
    RouteConfig {
        model: "test/mock".into(),
        max_tokens: 2048,
        timeout_s: 60,
        reasoning: None,
        reasoning_max_tokens: None,
    }
}

fn service(mock: &Arc<MockProvider>) -> Arc<LlmService> {
    let concrete = Arc::clone(mock);
    let p: Arc<dyn LlmProvider> = concrete;
    Arc::new(
        LlmService::builder()
            .route("worldgen_room", route())
            .provider("openrouter", p)
            .global_enabled(true)
            .build(),
    )
}

fn service_with_budget(mock: &Arc<MockProvider>, budget: BudgetConfig) -> Arc<LlmService> {
    let concrete = Arc::clone(mock);
    let p: Arc<dyn LlmProvider> = concrete;
    Arc::new(
        LlmService::builder()
            .route("worldgen_room", route())
            .provider("openrouter", p)
            .budgets(budget)
            .global_enabled(true)
            .build(),
    )
}

fn tool_resp(input: serde_json::Value) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::ToolUse {
            id: "t1".into(),
            name: "submit_room".into(),
            input,
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage {
            input_tokens: 120,
            output_tokens: 60,
            ..Usage::default()
        },
        model: "test/mock".into(),
    }
}

fn text_resp(t: &str) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::text(t)],
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
        model: "test/mock".into(),
    }
}

/// A world booted with the worldgen module, plus a fresh Hall (`#2`) the
/// builder (GOD, who holds every power) stands in.
fn gen_world() -> (TestWorld, Arc<MockProvider>, Arc<LlmService>, ObjId) {
    let mock = Arc::new(MockProvider::new("openrouter"));
    let llm = service(&mock);
    let mut tw = TestWorld::with_modules(vec![Box::new(WorldgenModule::new(Arc::clone(&llm)))]);
    let hall = tw
        .world_mut()
        .create(ObjKind::Room, "Hall", GOD, None)
        .unwrap();
    tw.world_mut().move_to(GOD, hall).unwrap();
    (tw, mock, llm, hall)
}

fn step_cmd(
    tw: &mut TestWorld,
    actor: ObjId,
    line: &str,
) -> (Vec<(Audience, OutputEvent)>, Vec<Effect>) {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut().step(
        Inbound::Cmd {
            conn: 0,
            actor,
            line: line.to_string(),
        },
        &mut out,
        &mut fx,
    );
    (out.take(), fx.drain())
}

/// The single `Effect::ModuleAsync` a `@gen` deferred: `(module, correlation, payload)`.
fn module_async(effects: &[Effect]) -> (String, u64, String) {
    for e in effects {
        if let Effect::ModuleAsync {
            module,
            correlation,
            payload,
            ..
        } = e
        {
            return (module.to_string(), *correlation, payload.clone());
        }
    }
    panic!("expected one Effect::ModuleAsync, got {effects:?}");
}

fn apply_done(
    tw: &mut TestWorld,
    module: &str,
    actor: ObjId,
    correlation: u64,
    payload: String,
) -> Vec<(Audience, OutputEvent)> {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut().step(
        Inbound::Apply(WorldCommand::ModuleAsyncDone {
            module: module.into(),
            actor,
            correlation,
            payload,
        }),
        &mut out,
        &mut fx,
    );
    out.take()
}

/// The `Sys` lines addressed to `actor`.
fn sys_to(events: &[(Audience, OutputEvent)], actor: ObjId) -> Vec<String> {
    events
        .iter()
        .filter_map(|(aud, e)| match (aud, e) {
            (Audience::Actor(a), OutputEvent::Sys(t)) if *a == actor => Some(t.to_plain_text()),
            _ => None,
        })
        .collect()
}

fn contains(lines: &[String], needle: &str) -> bool {
    lines.iter().any(|l| l.contains(needle))
}

/// Drive one full `@gen` cycle (dispatch → run off-loop → re-enter) and
/// return the completion's events.
async fn gen_cycle(
    tw: &mut TestWorld,
    llm: &LlmService,
    builder: ObjId,
    line: &str,
) -> Vec<(Audience, OutputEvent)> {
    let (_open, effects) = step_cmd(tw, builder, line);
    let (module, correlation, payload) = module_async(&effects);
    let result = run_worldgen_async(llm, &payload).await;
    apply_done(tw, &module, builder, correlation, result)
}

// ----------------------------------------------------------------------
// Themes
// ----------------------------------------------------------------------

#[test]
fn theme_set_show_and_exemplar() {
    let mut tw = TestWorld::with_modules(vec![Box::new(WorldgenModule::new(service(&Arc::new(
        MockProvider::new("openrouter"),
    ))))]);
    let hall = tw
        .world_mut()
        .create(ObjKind::Room, "Hall", GOD, None)
        .unwrap();
    tw.world_mut().move_to(GOD, hall).unwrap();

    // Set several fields, including a list field.
    step_cmd(&mut tw, GOD, "@theme here=style_guide:smoky jazz-age noir");
    step_cmd(&mut tw, GOD, "@theme here=vocabulary:gaslight, brass, rain");
    step_cmd(&mut tw, GOD, "@theme here=desc_length:long");
    let theme = mush_worldgen::resolve_theme(tw.world(), hall);
    assert_eq!(theme.style_guide, "smoky jazz-age noir");
    assert_eq!(theme.vocabulary, ["gaslight", "brass", "rain"]);
    assert_eq!(theme.desc_length, mush_worldgen::DescLength::Long);

    // /show renders the resolved theme.
    let (events, _) = step_cmd(&mut tw, GOD, "@theme/show here");
    let lines = sys_to(&events, GOD);
    assert!(contains(&lines, "smoky jazz-age noir"), "show: {lines:?}");
    assert!(contains(&lines, "gaslight"), "show: {lines:?}");

    // /exemplar copies a room's accepted desc into the theme.
    let alley = tw
        .world_mut()
        .create(ObjKind::Room, "Alley", GOD, None)
        .unwrap();
    tw.world_mut()
        .set_attr(
            alley,
            "desc",
            mush_core::Attr::text("A narrow brick alley, slick with rain.", GOD),
        )
        .unwrap();
    step_cmd(&mut tw, GOD, &format!("@theme/exemplar here=#{}", alley.0));
    let theme = mush_worldgen::theme_on(tw.world(), hall).unwrap();
    assert_eq!(theme.exemplars.len(), 1);
    assert!(theme.exemplars[0].contains("narrow brick alley"));
}

#[test]
fn theme_resolution_is_hierarchical() {
    let mut tw = TestWorld::with_modules(vec![Box::new(WorldgenModule::new(service(&Arc::new(
        MockProvider::new("openrouter"),
    ))))]);
    // Zone master, and a room in the zone.
    let downtown = tw
        .world_mut()
        .create(ObjKind::Thing, "Downtown", GOD, Some(LIMBO))
        .unwrap();
    tw.world_mut()
        .set_flag(downtown, mush_core::Flags::ZONE_MASTER, true)
        .unwrap();
    let hall = tw
        .world_mut()
        .create(ObjKind::Room, "Hall", GOD, None)
        .unwrap();
    tw.world_mut().set_zone(hall, Some(downtown)).unwrap();
    let bare = tw
        .world_mut()
        .create(ObjKind::Room, "Bare", GOD, None)
        .unwrap();
    tw.world_mut().set_zone(bare, Some(downtown)).unwrap();

    // Game default on #0, zone default on Downtown, room override on Hall.
    step_cmd(&mut tw, GOD, "@theme #0=vocabulary:fog");
    step_cmd(&mut tw, GOD, "@theme #0=desc_length:long");
    step_cmd(
        &mut tw,
        GOD,
        &format!("@theme #{}=style_guide:zone style", downtown.0),
    );
    step_cmd(
        &mut tw,
        GOD,
        &format!("@theme #{}=style_guide:room style", hall.0),
    );

    // Hall: nearest style wins; vocabulary unions in the game default; length
    // inherits from the game default (Hall/zone leave it at the default).
    let ht = mush_worldgen::resolve_theme(tw.world(), hall);
    assert_eq!(ht.style_guide, "room style");
    assert!(ht.vocabulary.iter().any(|v| v == "fog"), "{ht:?}");
    assert_eq!(ht.desc_length, mush_worldgen::DescLength::Long);

    // Bare has no room override: it falls through to the zone style.
    let bt = mush_worldgen::resolve_theme(tw.world(), bare);
    assert_eq!(bt.style_guide, "zone style");
}

// ----------------------------------------------------------------------
// @gen: prompt assembly, draft (not committed), the defer/re-entry path
// ----------------------------------------------------------------------

#[tokio::test]
async fn gen_prompt_carries_theme_and_neighbors() {
    let (mut tw, mock, llm, hall) = gen_world();
    // A theme on the room and a described neighbor to the north.
    step_cmd(&mut tw, GOD, "@theme here=style_guide:smoky jazz-age noir");
    let alley = tw
        .world_mut()
        .create(ObjKind::Room, "Alley", GOD, None)
        .unwrap();
    tw.world_mut()
        .set_attr(
            alley,
            "desc",
            mush_core::Attr::text("A narrow brick alley.", GOD),
        )
        .unwrap();
    let north = tw
        .world_mut()
        .create(ObjKind::Exit, "north", GOD, Some(hall))
        .unwrap();
    tw.world_mut().link_exit(north, Some(alley)).unwrap();

    mock.push_response(tool_resp(json!({ "desc": "A cramped pawn shop." })));
    let (_open, effects) = step_cmd(
        &mut tw,
        GOD,
        "@gen here=A cramped pawn shop, dusty counters.",
    );
    let (_m, _c, payload) = module_async(&effects);
    let _ = run_worldgen_async(&llm, &payload).await;

    let req = mock.last_request().expect("provider was called");
    let user = req
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        user.contains("smoky jazz-age noir"),
        "theme missing:\n{user}"
    );
    assert!(
        user.contains("A narrow brick alley."),
        "neighbor missing:\n{user}"
    );
    assert!(
        user.contains("A cramped pawn shop"),
        "prose missing:\n{user}"
    );
    // The forced-tool structured-output request carries the schema tool.
    assert!(req.tools.iter().any(|t| t.name == "submit_room"));
}

#[tokio::test]
async fn gen_defers_and_draft_appears_via_reentry_without_committing() {
    let (mut tw, mock, llm, hall) = gen_world();
    mock.push_response(tool_resp(
        json!({ "desc": "A grand hall of veined marble.", "short_desc": "a marble hall" }),
    ));

    // The @gen returns immediately (world never blocks): "Generating..." with
    // NO draft and NO committed desc yet.
    let (open, effects) = step_cmd(&mut tw, GOD, "@gen here=A grand hall of marble.");
    assert!(
        contains(&sys_to(&open, GOD), "Generating"),
        "{:?}",
        sys_to(&open, GOD)
    );
    assert!(tw.world().attr(hall, "desc").is_none());
    let (module, correlation, payload) = module_async(&effects);

    // Off-loop work takes "time"; the world proceeds meanwhile.
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let result = run_worldgen_async(&llm, &payload).await;

    // Re-entry stores the draft and previews it — still not committed.
    let done = apply_done(&mut tw, &module, GOD, correlation, result);
    let lines = sys_to(&done, GOD);
    assert!(contains(&lines, "Draft ready"), "{lines:?}");
    assert!(contains(&lines, "grand hall of veined marble"), "{lines:?}");
    assert!(
        tw.world().attr(hall, "desc").is_none(),
        "must not auto-commit"
    );

    // A side-by-side preview panel frame reached the builder.
    assert!(done.iter().any(|(_, e)| matches!(
        e,
        OutputEvent::Panel { feed, .. } if feed == "worldgen.draft"
    )));

    // The draft was persisted as a module op (draft:{room}:{builder}).
    let ops = tw.take_module_ops();
    assert!(
        ops.iter().any(|op| op.ns == "worldgen"
            && matches!(&op.op, ModuleWrite::JsonSet { key, .. } if key.as_str() == format!("draft:{}:{}", hall.0, GOD.0))),
        "no draft persist op: {ops:?}"
    );
}

#[tokio::test]
async fn malformed_output_repairs_then_succeeds() {
    let (mut tw, mock, llm, _hall) = gen_world();
    // First: unparseable; repair: a valid tool call.
    mock.push_response(text_resp("here you go: {not valid json"));
    mock.push_response(tool_resp(json!({ "desc": "A recovered hall." })));

    let done = gen_cycle(&mut tw, &llm, GOD, "@gen here=A hall.").await;
    assert!(
        contains(&sys_to(&done, GOD), "Draft ready"),
        "{:?}",
        sys_to(&done, GOD)
    );
    assert_eq!(mock.call_count(), 2, "one repair retry expected");
}

#[tokio::test]
async fn malformed_output_twice_is_an_error() {
    let (mut _tw, mock, llm, _hall) = gen_world();
    mock.push_response(text_resp("garbage one"));
    mock.push_response(text_resp("garbage two"));

    // Drive just the async half to inspect the outcome directly.
    let snapshot = GenSnapshot {
        kind: mush_worldgen::GenKind::Room,
        room: ObjId(2),
        builder: GOD,
        theme: Default::default(),
        facts: mush_worldgen::RoomFacts {
            name: "Hall".into(),
            exits: vec![],
        },
        neighbors: vec![],
        prose: "A hall.".into(),
        guidance: None,
        prior_draft: None,
    };
    let payload = serde_json::to_string(&snapshot).unwrap();
    let result = run_worldgen_async(&llm, &payload).await;
    let outcome: GenOutcome = serde_json::from_str(&result).unwrap();
    assert!(
        matches!(
            outcome,
            GenOutcome::Err {
                retryable: true,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(mock.call_count(), 2);
}

#[test]
fn gen_rejects_a_room_you_are_not_in() {
    // `@gen <remote-room>=<prose>` would key a draft the review subcommands
    // (which resolve `here`) could never reach — reject it before any defer.
    let (mut tw, _mock, _llm, hall) = gen_world();
    let annex = tw
        .world_mut()
        .create(ObjKind::Room, "Annex", GOD, None)
        .unwrap();

    let (events, effects) = step_cmd(&mut tw, GOD, &format!("@gen #{}=A spooky annex.", annex.0));
    assert!(
        contains(&sys_to(&events, GOD), "room you're in"),
        "{:?}",
        sys_to(&events, GOD)
    );
    assert!(
        effects.is_empty(),
        "no LLM work may be deferred for a room the builder is not in"
    );

    // The current room, addressed by dbref, is still accepted (it defers).
    let (_e, effects) = step_cmd(&mut tw, GOD, &format!("@gen #{}=A grand hall.", hall.0));
    assert!(
        !effects.is_empty(),
        "the current room by dbref is a valid @gen target"
    );
}

#[tokio::test]
async fn text_fallback_takes_the_last_valid_json_object() {
    // A provider WITHOUT forced tool choice → the model answers in prose. A
    // reasoning preamble may carry a throwaway JSON object before the real
    // answer; the last valid draft must win, not the first `{`.
    let mock = Arc::new(MockProvider::new("openrouter").with_caps(ProviderCaps {
        tool_calling: true,
        forced_tool_choice: false,
        native_structured_output: false,
        sampling_params: true,
        streaming_usage: true,
        reasoning: true,
    }));
    let llm = service(&mock);
    let mut tw = TestWorld::with_modules(vec![Box::new(WorldgenModule::new(Arc::clone(&llm)))]);
    let hall = tw
        .world_mut()
        .create(ObjKind::Room, "Hall", GOD, None)
        .unwrap();
    tw.world_mut().move_to(GOD, hall).unwrap();

    mock.push_response(text_resp(
        "Reasoning: {\"desc\": \"a throwaway preamble object\"}. \
         Final answer:\n{\"desc\": \"A quiet study lined with books.\"}",
    ));

    let done = gen_cycle(&mut tw, &llm, GOD, "@gen here=A study.").await;
    let lines = sys_to(&done, GOD);
    assert!(contains(&lines, "Draft ready"), "{lines:?}");
    assert!(
        contains(&lines, "A quiet study lined with books"),
        "{lines:?}"
    );
    assert!(
        !contains(&lines, "throwaway preamble"),
        "took the preamble object instead of the real answer: {lines:?}"
    );
    // A single call: the text fallback parsed cleanly, no repair.
    assert_eq!(mock.call_count(), 1);
}

// ----------------------------------------------------------------------
// Draft review: accept / retry / discard
// ----------------------------------------------------------------------

#[tokio::test]
async fn accept_writes_desc_through_the_attr_path() {
    let (mut tw, mock, llm, hall) = gen_world();
    mock.push_response(tool_resp(json!({
        "desc": "A committed hall.",
        "short_desc": "the hall",
        "details": [{ "keyword": "counter", "desc": "A dusty glass counter." }]
    })));
    gen_cycle(&mut tw, &llm, GOD, "@gen here=A hall.").await;

    let (events, _) = step_cmd(&mut tw, GOD, "@gen/accept");
    assert!(
        contains(&sys_to(&events, GOD), "Accepted"),
        "{:?}",
        sys_to(&events, GOD)
    );

    // Written through the ordinary attribute path — and it persists on the obj.
    assert_eq!(
        tw.world().attr(hall, "desc").and_then(|a| a.as_text()),
        Some("A committed hall.")
    );
    assert_eq!(
        tw.world()
            .attr(hall, "short_desc")
            .and_then(|a| a.as_text()),
        Some("the hall")
    );
    assert_eq!(
        tw.world()
            .attr(hall, "detail:counter")
            .and_then(|a| a.as_text()),
        Some("A dusty glass counter.")
    );

    // Accept cleared the draft (persist Del queued) and there is nothing left.
    let (events, _) = step_cmd(&mut tw, GOD, "@gen/accept");
    assert!(
        contains(&sys_to(&events, GOD), "No draft"),
        "{:?}",
        sys_to(&events, GOD)
    );
}

#[tokio::test]
async fn retry_includes_the_prior_draft_and_guidance() {
    let (mut tw, mock, llm, _hall) = gen_world();
    mock.push_response(tool_resp(
        json!({ "desc": "First draft, bright and airy." }),
    ));
    gen_cycle(&mut tw, &llm, GOD, "@gen here=A hall, make it bright.").await;

    // Retry: the deferred snapshot must carry the prior draft + the new guidance.
    let (_events, effects) = step_cmd(&mut tw, GOD, "@gen/retry make it darker and colder");
    let (_m, _c, payload) = module_async(&effects);
    let snap: GenSnapshot = serde_json::from_str(&payload).unwrap();
    assert_eq!(
        snap.prior_draft.as_ref().map(|d| d.desc.as_str()),
        Some("First draft, bright and airy.")
    );
    assert_eq!(snap.guidance.as_deref(), Some("make it darker and colder"));
    // The original brief is reused verbatim.
    assert!(snap.prose.contains("make it bright"));

    // And the reassembled prompt tells the model to avoid the prior draft.
    let _ = run_worldgen_async(&llm, &payload).await;
    let req = mock.last_request().unwrap();
    let user = req
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(user.contains("AVOID THIS"), "{user}");
    assert!(user.contains("First draft, bright and airy."), "{user}");
    assert!(user.contains("darker and colder"), "{user}");
}

#[tokio::test]
async fn discard_removes_the_draft() {
    let (mut tw, mock, llm, hall) = gen_world();
    mock.push_response(tool_resp(json!({ "desc": "Throwaway." })));
    gen_cycle(&mut tw, &llm, GOD, "@gen here=A hall.").await;

    let (events, _) = step_cmd(&mut tw, GOD, "@gen/discard");
    assert!(
        contains(&sys_to(&events, GOD), "discarded"),
        "{:?}",
        sys_to(&events, GOD)
    );
    // A Del op was queued and the draft is gone.
    let ops = tw.take_module_ops();
    assert!(ops.iter().any(|op| matches!(
        &op.op,
        ModuleWrite::Del { key } if key.as_str() == format!("draft:{}:{}", hall.0, GOD.0)
    )));
    let (events, _) = step_cmd(&mut tw, GOD, "@gen/accept");
    assert!(contains(&sys_to(&events, GOD), "No draft"));
}

// ----------------------------------------------------------------------
// Gate, budget, persistence
// ----------------------------------------------------------------------

#[test]
fn builder_llm_gate_refuses_a_non_builder() {
    let (mut tw, _mock, _llm, hall) = gen_world();
    let nobody = tw.spawn_player("Nobody"); // plain player, no BUILDER_LLM
    tw.world_mut().move_to(nobody, hall).unwrap();

    let (events, effects) = step_cmd(&mut tw, nobody, "@gen here=A hall.");
    assert!(contains(&sys_to(&events, nobody), "Permission denied."));
    assert!(
        effects.is_empty(),
        "no LLM work may be deferred for a non-builder"
    );

    // Granting the power AND control of the room lets them through (the
    // deferred effect appears). BUILDER_LLM is the LLM-spend gate; controlling
    // the room is the ordinary edit permission.
    tw.world_mut().get_mut(nobody).unwrap().powers = Powers::BUILDER_LLM;
    tw.world_mut().get_mut(hall).unwrap().owner = nobody;
    let (_events, effects) = step_cmd(&mut tw, nobody, "@gen here=A hall.");
    assert!(
        !effects.is_empty(),
        "builder-power actor who controls the room may @gen"
    );
}

#[tokio::test]
async fn per_builder_budget_is_enforced() {
    let mock = Arc::new(MockProvider::new("openrouter"));
    // A tiny per-actor daily ceiling: the pre-flight check trips immediately.
    let budget = BudgetConfig {
        per_actor_default: Some(BudgetLimit::tokens(10)),
        ..BudgetConfig::default()
    };
    let llm = service_with_budget(&mock, budget);
    let mut tw = TestWorld::with_modules(vec![Box::new(WorldgenModule::new(Arc::clone(&llm)))]);
    let hall = tw
        .world_mut()
        .create(ObjKind::Room, "Hall", GOD, None)
        .unwrap();
    tw.world_mut().move_to(GOD, hall).unwrap();
    mock.push_response(tool_resp(json!({ "desc": "unreachable" })));

    let done = gen_cycle(&mut tw, &llm, GOD, "@gen here=A hall.").await;
    assert!(
        contains(&sys_to(&done, GOD), "budget"),
        "{:?}",
        sys_to(&done, GOD)
    );
    assert_eq!(
        mock.call_count(),
        0,
        "budget trips before any provider call"
    );
}

#[tokio::test]
async fn draft_persists_across_reboot() {
    // First boot: generate a draft, capture its persisted document.
    let (mut tw, mock, llm, hall) = gen_world();
    assert_eq!(hall, ObjId(2)); // deterministic id, recreated identically below
    mock.push_response(tool_resp(json!({ "desc": "A hall that survives reboot." })));
    gen_cycle(&mut tw, &llm, GOD, "@gen here=A hall.").await;
    let ops = tw.take_module_ops();
    let (key, value) = ops
        .iter()
        .find_map(|op| match &op.op {
            ModuleWrite::JsonSet { key, value } if key.starts_with("draft:") => {
                Some((key.clone(), value.clone()))
            }
            _ => None,
        })
        .expect("a draft was persisted");
    // Sanity: the persisted doc is a StoredDraft.
    let _: StoredDraft = serde_json::from_value(value.clone()).unwrap();

    // Second boot: fresh module + identically-shaped world, hand it the doc.
    let mock2 = Arc::new(MockProvider::new("openrouter"));
    let llm2 = service(&mock2);
    let mut tw2 = TestWorld::with_modules(vec![Box::new(WorldgenModule::new(llm2))]);
    let hall2 = tw2
        .world_mut()
        .create(ObjKind::Room, "Hall", GOD, None)
        .unwrap();
    assert_eq!(hall2, hall);
    tw2.world_mut().move_to(GOD, hall2).unwrap();

    let mut boot = std::collections::HashMap::new();
    boot.insert(
        smol_str::SmolStr::new("worldgen"),
        mush_core::ModuleBootData {
            kv: vec![(key, value)],
            streams: std::collections::HashMap::new(),
        },
    );
    tw2.modules_loaded(boot);

    // The reloaded draft is live: accepting it writes the drafted desc.
    let (events, _) = step_cmd(&mut tw2, GOD, "@gen/accept");
    assert!(
        contains(&sys_to(&events, GOD), "Accepted"),
        "{:?}",
        sys_to(&events, GOD)
    );
    assert_eq!(
        tw2.world().attr(hall2, "desc").and_then(|a| a.as_text()),
        Some("A hall that survives reboot.")
    );
}

#[test]
fn llm_staff_commands_reach_the_shared_service() {
    let mock = Arc::new(MockProvider::new("openrouter"));
    let llm = service(&mock);
    let mut tw = TestWorld::with_modules(vec![Box::new(WorldgenModule::new(Arc::clone(&llm)))]);

    // /budget renders the ledger view.
    let (events, _) = step_cmd(&mut tw, GOD, "@llm/budget");
    assert!(
        contains(&sys_to(&events, GOD), "budget"),
        "{:?}",
        sys_to(&events, GOD)
    );

    // /disable + /enable drive the SAME kill switch the test holds a handle to.
    assert!(!llm.kill_switch().global_disabled());
    step_cmd(&mut tw, GOD, "@llm/disable");
    assert!(llm.kill_switch().global_disabled());
    step_cmd(&mut tw, GOD, "@llm/enable");
    assert!(!llm.kill_switch().global_disabled());
}

// ----------------------------------------------------------------------
// The LLM cost dashboard (M10c): the "llm.dashboard" query handler +
// `@llm/dashboard`.
// ----------------------------------------------------------------------

#[test]
fn llm_dashboard_query_handler_is_staff_gated_and_carries_no_prompt_text() {
    let (mut tw, _mock, llm, hall) = gen_world();
    let nobody = tw.spawn_player("Nobody"); // plain player, no staff role
    tw.world_mut().move_to(nobody, hall).unwrap();

    // Give the shared service some spend to aggregate: a global-scoped
    // settle (so a "global" budget row is guaranteed) plus a full audit row
    // carrying real "prompt-shaped" text the payload must never surface.
    llm.ledger().settle(
        &ScopeLimits {
            global: Some(BudgetLimit::tokens(1_000_000)),
            ..Default::default()
        },
        &Usage {
            input_tokens: 42,
            output_tokens: 8,
            ..Usage::default()
        },
    );
    llm.audit_log().record(mush_llm::AuditRecord {
        ts: 1_700_000_000,
        feature: "worldgen_room".into(),
        provider: "openrouter".into(),
        model: "z-ai/glm-5.3".into(),
        actor: Some(GOD),
        npc: None,
        prompt_sha256: "deadbeef".into(),
        prompt_summary: "A SUPER SECRET builder prompt".into(),
        response_summary: "A SUPER SECRET generated room description".into(),
        input_tokens: 42,
        output_tokens: 8,
        cost_usd: Some(0.01),
        latency_ms: 900,
        outcome: mush_llm::Outcome::Ok,
    });

    let (owner, handler) = tw
        .registry()
        .query_handler("llm.dashboard")
        .expect("llm.dashboard query handler registered");
    assert_eq!(owner.as_str(), "worldgen");

    // A mortal is refused, never sees the aggregate.
    let refusal = handler
        .query(tw.world(), nobody, &json!({}))
        .expect_err("a plain player must be refused the LLM dashboard");
    assert!(!refusal.to_plain_text().is_empty());

    // GOD (outranks Royalty) gets the full aggregate.
    let ok = handler
        .query(tw.world(), GOD, &json!({}))
        .expect("staff gets the dashboard");
    let OutputEvent::Panel { feed, data, .. } = ok else {
        panic!("expected a Panel frame, got {ok:?}");
    };
    assert_eq!(feed.as_str(), "llm.dashboard");

    let global = data["budgets"]
        .as_array()
        .expect("budgets array")
        .iter()
        .find(|b| b["scope"] == json!("global"))
        .expect("a global budget row");
    assert_eq!(global["input_tokens"], json!(42));
    assert_eq!(global["output_tokens"], json!(8));

    let recent = data["recent_calls"].as_array().expect("recent_calls array");
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0]["feature"], json!("worldgen_room"));
    assert_eq!(recent[0]["outcome"], json!("ok"));
    // The recorded actor is resolved to a display name (the ObjRef wire
    // shape), not a bare id.
    assert_eq!(recent[0]["actor"]["name"], json!("God"));

    // NO prompt/response text anywhere in the payload, at any depth.
    let rendered = data.to_string();
    assert!(!rendered.contains("SECRET"));
    assert!(!rendered.contains("prompt_summary"));
    assert!(!rendered.contains("response_summary"));
    assert!(!rendered.contains("deadbeef")); // prompt_sha256 excluded too
}

#[test]
fn llm_dashboard_terminal_command_renders_budgets_and_recent_calls() {
    let mock = Arc::new(MockProvider::new("openrouter"));
    let budget = BudgetConfig {
        per_feature: [("worldgen_room".to_string(), BudgetLimit::tokens(1_000))]
            .into_iter()
            .collect(),
        ..BudgetConfig::default()
    };
    let llm = service_with_budget(&mock, budget);
    let mut tw = TestWorld::with_modules(vec![Box::new(WorldgenModule::new(Arc::clone(&llm)))]);

    llm.ledger().settle(
        &ScopeLimits {
            feature: Some(("worldgen_room".to_string(), BudgetLimit::tokens(1_000))),
            actor: Some((GOD, BudgetLimit::tokens(1_000))),
            ..Default::default()
        },
        &Usage {
            input_tokens: 30,
            output_tokens: 20,
            cost_usd: Some(0.02),
            ..Usage::default()
        },
    );

    let (events, _) = step_cmd(&mut tw, GOD, "@llm/dashboard");
    let lines = sys_to(&events, GOD);
    assert!(
        contains(&lines, "kill switch"),
        "expected a kill-switch line: {lines:?}"
    );
    assert!(
        contains(&lines, "feature:worldgen_room"),
        "expected a per-feature budget row: {lines:?}"
    );
    assert!(
        contains(&lines, "Top builders by spend today"),
        "expected a top-builders section: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("God") && l.contains("30 in") && l.contains("20 out")),
        "expected God's spend row (30 in + 20 out tokens): {lines:?}"
    );

    // A mortal is refused the whole command (min_role: Wizard).
    let nobody = tw.spawn_player("Nobody");
    let (events, _) = step_cmd(&mut tw, nobody, "@llm/dashboard");
    assert!(contains(&sys_to(&events, nobody), "Permission denied."));
}

// ----------------------------------------------------------------------
// Batch @gen/building: the brief pass + the room-pass pipeline (design 03 §5.3)
// ----------------------------------------------------------------------

/// A service with BOTH the brief and room routes on the same mock provider.
fn batch_service(mock: &Arc<MockProvider>) -> Arc<LlmService> {
    let concrete = Arc::clone(mock);
    let p: Arc<dyn LlmProvider> = concrete;
    Arc::new(
        LlmService::builder()
            .route("worldgen_room", route())
            .route("worldgen_brief", route())
            .provider("openrouter", p)
            .global_enabled(true)
            .build(),
    )
}

fn batch_service_with_budget(mock: &Arc<MockProvider>, budget: BudgetConfig) -> Arc<LlmService> {
    let concrete = Arc::clone(mock);
    let p: Arc<dyn LlmProvider> = concrete;
    Arc::new(
        LlmService::builder()
            .route("worldgen_room", route())
            .route("worldgen_brief", route())
            .provider("openrouter", p)
            .budgets(budget)
            .global_enabled(true)
            .build(),
    )
}

/// A forced-tool response carrying a building brief.
fn brief_tool_resp(input: serde_json::Value) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::ToolUse {
            id: "b1".into(),
            name: "submit_building_brief".into(),
            input,
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage {
            input_tokens: 300,
            output_tokens: 200,
            ..Usage::default()
        },
        model: "test/mock".into(),
    }
}

/// A room tool response with explicit token usage (for budget arithmetic).
fn room_resp_usage(input: serde_json::Value, in_tok: u64, out_tok: u64) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::ToolUse {
            id: "r1".into(),
            name: "submit_room".into(),
            input,
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage {
            input_tokens: in_tok,
            output_tokens: out_tok,
            ..Usage::default()
        },
        model: "test/mock".into(),
    }
}

/// The concatenated user-turn text of a captured request.
fn user_text(req: &mush_llm::CompletionRequest) -> String {
    req.messages
        .iter()
        .flat_map(|m| &m.content)
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Dig a one-way exit `from --name--> to`.
fn dig_exit(tw: &mut TestWorld, name: &str, from: ObjId, to: ObjId) {
    let e = tw
        .world_mut()
        .create(ObjKind::Exit, name, GOD, Some(from))
        .unwrap();
    tw.world_mut().link_exit(e, Some(to)).unwrap();
}

/// A 3-room dug building (Hall entrance <-> Parlor <-> Study, two-way exits),
/// a room-level theme on the Hall, GOD standing in the Hall. Unzoned rooms —
/// the walk falls back to bounded hop count.
fn building_world() -> (TestWorld, Arc<MockProvider>, Arc<LlmService>, [ObjId; 3]) {
    let mock = Arc::new(MockProvider::new("openrouter"));
    let llm = batch_service(&mock);
    let mut tw = TestWorld::with_modules(vec![Box::new(WorldgenModule::new(Arc::clone(&llm)))]);
    let hall = tw
        .world_mut()
        .create(ObjKind::Room, "Hall", GOD, None)
        .unwrap();
    let parlor = tw
        .world_mut()
        .create(ObjKind::Room, "Parlor", GOD, None)
        .unwrap();
    let study = tw
        .world_mut()
        .create(ObjKind::Room, "Study", GOD, None)
        .unwrap();
    dig_exit(&mut tw, "north", hall, parlor);
    dig_exit(&mut tw, "south", parlor, hall);
    dig_exit(&mut tw, "north", parlor, study);
    dig_exit(&mut tw, "south", study, parlor);
    tw.world_mut().move_to(GOD, hall).unwrap();
    step_cmd(&mut tw, GOD, "@theme here=style_guide:smoky jazz-age noir");
    (tw, mock, llm, [hall, parlor, study])
}

/// Apply a completion, returning BOTH the events and any effects the
/// completion deferred (the room-pass pipeline re-defers the next room).
fn apply_done_fx(
    tw: &mut TestWorld,
    module: &str,
    actor: ObjId,
    correlation: u64,
    payload: String,
) -> (Vec<(Audience, OutputEvent)>, Vec<Effect>) {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut().step(
        Inbound::Apply(WorldCommand::ModuleAsyncDone {
            module: module.into(),
            actor,
            correlation,
            payload,
        }),
        &mut out,
        &mut fx,
    );
    (out.take(), fx.drain())
}

/// Drive the completion-pipelined room pass: run each deferred room off-loop
/// and re-enter, following the chain of next-room effects to exhaustion.
async fn drive_rooms(
    tw: &mut TestWorld,
    llm: &LlmService,
    builder: ObjId,
    mut effects: Vec<Effect>,
) -> Vec<(Audience, OutputEvent)> {
    let mut all = Vec::new();
    loop {
        let next = effects.iter().find_map(|e| match e {
            Effect::ModuleAsync {
                module,
                correlation,
                payload,
                ..
            } => Some((module.to_string(), *correlation, payload.clone())),
            _ => None,
        });
        let Some((module, correlation, payload)) = next else {
            break;
        };
        let result = run_worldgen_async(llm, &payload).await;
        let (events, fx) = apply_done_fx(tw, &module, builder, correlation, result);
        all.extend(events);
        effects = fx;
    }
    all
}

/// The brief the mock returns for the 3-room building.
fn mansion_brief() -> serde_json::Value {
    json!({
        "concept": "A jazz-age mansion gone to seed",
        "palette": "brass, velvet, gaslight, rain on glass",
        "rooms": [
            { "index": 1, "name": "Hall",   "concept": "grand marble foyer" },
            { "index": 2, "name": "Parlor", "concept": "smoky sitting room" },
            { "index": 3, "name": "Study",  "concept": "book-lined study" }
        ]
    })
}

/// Run `@gen/building here=...`, the brief pass, and its re-entry. Returns the
/// brief-completion events.
async fn run_brief(
    tw: &mut TestWorld,
    llm: &LlmService,
    line: &str,
) -> Vec<(Audience, OutputEvent)> {
    let (_open, effects) = step_cmd(tw, GOD, line);
    let (module, correlation, payload) = module_async(&effects);
    let result = run_worldgen_async(llm, &payload).await;
    apply_done(tw, &module, GOD, correlation, result)
}

#[tokio::test]
async fn building_brief_walks_the_graph_and_carries_the_theme() {
    let (mut tw, mock, llm, [hall, _parlor, _study]) = building_world();
    mock.push_response(brief_tool_resp(mansion_brief()));

    let done = run_brief(
        &mut tw,
        &llm,
        "@gen/building here=A grand jazz-age mansion.",
    )
    .await;
    assert!(
        contains(&sys_to(&done, GOD), "Brief ready"),
        "{:?}",
        sys_to(&done, GOD)
    );

    // The brief prompt walked the room graph and carried the theme.
    let brief_req = &mock.calls()[0];
    let user = user_text(brief_req);
    assert!(
        user.contains("smoky jazz-age noir"),
        "theme missing:\n{user}"
    );
    assert!(user.contains("BUILDING MAP"), "no graph walk:\n{user}");
    for name in ["Hall", "Parlor", "Study"] {
        assert!(user.contains(name), "room {name} missing:\n{user}");
    }
    assert!(user.contains("-> Parlor"), "graph edge missing:\n{user}");
    assert!(
        brief_req
            .tools
            .iter()
            .any(|t| t.name == "submit_building_brief"),
        "brief tool not offered"
    );

    // Nothing is committed by the brief (it is itself a reviewable draft).
    assert!(tw.world().attr(hall, "desc").is_none());
    // A brief panel frame reached the builder.
    assert!(done.iter().any(|(_, e)| matches!(
        e,
        OutputEvent::Panel { feed, .. } if feed == "worldgen.draft"
    )));
    // The batch persisted a building:{entrance}:{builder} document.
    let ops = tw.take_module_ops();
    assert!(
        ops.iter().any(|op| op.ns == "worldgen"
            && matches!(&op.op, ModuleWrite::JsonSet { key, .. }
                if key.as_str() == format!("building:{}:{}", hall.0, GOD.0))),
        "no batch persist op: {ops:?}"
    );
}

#[tokio::test]
async fn building_room_pass_seeds_brief_and_drafted_neighbors() {
    let (mut tw, mock, llm, [hall, parlor, study]) = building_world();
    mock.push_response(brief_tool_resp(mansion_brief()));
    // Rooms drafted in BFS order: Hall, Parlor, Study.
    mock.push_response(tool_resp(json!({
        "desc": "A grand hall of veined marble, brass fixtures aglow."
    })));
    mock.push_response(tool_resp(
        json!({ "desc": "A smoky parlor of worn velvet." }),
    ));
    mock.push_response(tool_resp(
        json!({ "desc": "A book-lined study, rain on the glass." }),
    ));

    run_brief(
        &mut tw,
        &llm,
        "@gen/building here=A grand jazz-age mansion.",
    )
    .await;

    // Accept the brief -> the room-pass pipeline begins.
    let (acc, effects) = step_cmd(&mut tw, GOD, "@gen/accept");
    assert!(
        contains(&sys_to(&acc, GOD), "Generating"),
        "{:?}",
        sys_to(&acc, GOD)
    );
    let events = drive_rooms(&mut tw, &llm, GOD, effects).await;

    // brief + one call per room.
    assert_eq!(mock.call_count(), 4, "brief + 3 rooms");
    let calls = mock.calls();
    // EVERY room prompt (calls 1..=3, after the brief) carried the shared
    // brief palette.
    for (i, call) in calls.iter().enumerate().skip(1) {
        let u = user_text(call);
        assert!(
            u.contains("brass, velvet, gaslight"),
            "room #{i} prompt missing brief palette:\n{u}"
        );
    }
    // A later room's prompt carried an already-DRAFTED neighbor (Parlor, the
    // 2nd room, has an exit back to the just-drafted Hall).
    let parlor_prompt = user_text(&calls[2]);
    assert!(
        parlor_prompt.contains("A grand hall of veined marble"),
        "parlor prompt missing its drafted neighbor:\n{parlor_prompt}"
    );

    // Progress streamed, and the batch completed.
    let lines = sys_to(&events, GOD);
    assert!(contains(&lines, "Drafted"), "no progress: {lines:?}");
    assert!(contains(&lines, "complete"), "no completion: {lines:?}");
    // A worldgen.draft panel frame per room draft.
    let panels = events
        .iter()
        .filter(|(_, e)| matches!(e, OutputEvent::Panel { feed, .. } if feed == "worldgen.draft"))
        .count();
    assert_eq!(panels, 3, "expected one draft panel per room");

    // NOTHING was auto-committed — every room is still an unreviewed draft.
    for r in [hall, parlor, study] {
        assert!(
            tw.world().attr(r, "desc").is_none(),
            "room {r} auto-committed"
        );
    }
}

#[tokio::test]
async fn building_review_steps_and_accept_writes_descs() {
    let (mut tw, mock, llm, [hall, _parlor, _study]) = building_world();
    mock.push_response(brief_tool_resp(mansion_brief()));
    mock.push_response(tool_resp(
        json!({ "desc": "A grand hall of veined marble." }),
    ));
    mock.push_response(tool_resp(
        json!({ "desc": "A smoky parlor of worn velvet." }),
    ));
    mock.push_response(tool_resp(json!({ "desc": "A book-lined study." })));

    run_brief(
        &mut tw,
        &llm,
        "@gen/building here=A grand jazz-age mansion.",
    )
    .await;
    let (_acc, effects) = step_cmd(&mut tw, GOD, "@gen/accept");
    drive_rooms(&mut tw, &llm, GOD, effects).await;

    // @gen/review lists the batch + status.
    let (rev, _) = step_cmd(&mut tw, GOD, "@gen/review here");
    let lines = sys_to(&rev, GOD);
    assert!(contains(&lines, "3/3 rooms drafted"), "{lines:?}");
    assert!(contains(&lines, "drafted] Hall"), "{lines:?}");

    // @gen/review sets the cursor to the first drafted room (Hall); accept
    // writes its desc through the ordinary attribute path.
    let (accepted, _) = step_cmd(&mut tw, GOD, "@gen/accept");
    assert!(
        contains(&sys_to(&accepted, GOD), "Accepted"),
        "{:?}",
        sys_to(&accepted, GOD)
    );
    assert_eq!(
        tw.world().attr(hall, "desc").and_then(|a| a.as_text()),
        Some("A grand hall of veined marble.")
    );

    // @gen/accept/all writes every remaining drafted room.
    let (all, _) = step_cmd(&mut tw, GOD, "@gen/accept/all");
    assert!(
        contains(&sys_to(&all, GOD), "Accepted"),
        "{:?}",
        sys_to(&all, GOD)
    );
    assert_eq!(
        tw.world().attr(_parlor, "desc").and_then(|a| a.as_text()),
        Some("A smoky parlor of worn velvet.")
    );
    assert_eq!(
        tw.world().attr(_study, "desc").and_then(|a| a.as_text()),
        Some("A book-lined study.")
    );
}

#[test]
fn building_refuses_a_room_with_no_interior_exits() {
    let mock = Arc::new(MockProvider::new("openrouter"));
    let llm = batch_service(&mock);
    let mut tw = TestWorld::with_modules(vec![Box::new(WorldgenModule::new(llm))]);
    let hall = tw
        .world_mut()
        .create(ObjKind::Room, "Hall", GOD, None)
        .unwrap();
    tw.world_mut().move_to(GOD, hall).unwrap();

    let (events, effects) = step_cmd(&mut tw, GOD, "@gen/building here=A lone room.");
    assert!(
        contains(&sys_to(&events, GOD), "no interior exits"),
        "{:?}",
        sys_to(&events, GOD)
    );
    assert!(
        effects.is_empty(),
        "no brief may be deferred for a room with no interior"
    );
}

#[test]
fn building_refuses_a_non_builder() {
    let (mut tw, _mock, _llm, [hall, ..]) = building_world();
    let nobody = tw.spawn_player("Nobody"); // no BUILDER_LLM power
    tw.world_mut().move_to(nobody, hall).unwrap();

    let (events, effects) = step_cmd(&mut tw, nobody, "@gen/building here=A mansion.");
    assert!(contains(&sys_to(&events, nobody), "Permission denied."));
    assert!(
        effects.is_empty(),
        "no LLM work may be deferred for a non-builder"
    );
}

#[tokio::test]
async fn building_budget_trip_mid_batch_degrades_remaining() {
    let mock = Arc::new(MockProvider::new("openrouter"));
    // A per-FEATURE ceiling on the room route only, sized so one room settles
    // (2000 tokens) and the next room's pre-check trips. The brief route is
    // uncapped, so the brief itself always succeeds.
    let budget = BudgetConfig {
        per_feature: [("worldgen_room".to_string(), BudgetLimit::tokens(3_000))]
            .into_iter()
            .collect(),
        ..BudgetConfig::default()
    };
    let llm = batch_service_with_budget(&mock, budget);
    let mut tw = TestWorld::with_modules(vec![Box::new(WorldgenModule::new(Arc::clone(&llm)))]);
    let hall = tw
        .world_mut()
        .create(ObjKind::Room, "Hall", GOD, None)
        .unwrap();
    let parlor = tw
        .world_mut()
        .create(ObjKind::Room, "Parlor", GOD, None)
        .unwrap();
    let study = tw
        .world_mut()
        .create(ObjKind::Room, "Study", GOD, None)
        .unwrap();
    dig_exit(&mut tw, "north", hall, parlor);
    dig_exit(&mut tw, "south", parlor, hall);
    dig_exit(&mut tw, "north", parlor, study);
    dig_exit(&mut tw, "south", study, parlor);
    tw.world_mut().move_to(GOD, hall).unwrap();

    mock.push_response(brief_tool_resp(mansion_brief()));
    // Only the first room ever reaches the provider; its big settle exhausts
    // the room-feature budget so the second room trips pre-call.
    mock.push_response(room_resp_usage(
        json!({ "desc": "A grand hall of veined marble." }),
        1_000,
        1_000,
    ));

    run_brief(&mut tw, &llm, "@gen/building here=A grand mansion.").await;
    let (_acc, effects) = step_cmd(&mut tw, GOD, "@gen/accept");
    let events = drive_rooms(&mut tw, &llm, GOD, effects).await;

    // The budget tripped mid-batch: the builder was told, and the batch stopped.
    assert!(
        contains(&sys_to(&events, GOD), "Budget reached"),
        "{:?}",
        sys_to(&events, GOD)
    );
    // Only the brief + the first room reached the provider.
    assert_eq!(mock.call_count(), 2, "brief + one room before the trip");

    // The remaining rooms are marked needs-attention; NOTHING auto-committed.
    let (rev, _) = step_cmd(&mut tw, GOD, "@gen/review here");
    let lines = sys_to(&rev, GOD);
    assert!(contains(&lines, "needs attention"), "{lines:?}");
    for r in [hall, parlor, study] {
        assert!(
            tw.world().attr(r, "desc").is_none(),
            "budget-halted batch must not auto-commit {r}"
        );
    }
}

#[tokio::test]
async fn building_batch_persists_across_reboot() {
    // Boot 1: run a building to drafts, capture the persisted documents.
    let (mut tw, mock, llm, [hall, parlor, study]) = building_world();
    assert_eq!([hall.0, parlor.0, study.0], [2, 3, 4]); // deterministic ids
    mock.push_response(brief_tool_resp(mansion_brief()));
    mock.push_response(tool_resp(
        json!({ "desc": "A grand hall of veined marble." }),
    ));
    mock.push_response(tool_resp(
        json!({ "desc": "A smoky parlor of worn velvet." }),
    ));
    mock.push_response(tool_resp(json!({ "desc": "A book-lined study." })));
    run_brief(
        &mut tw,
        &llm,
        "@gen/building here=A grand jazz-age mansion.",
    )
    .await;
    let (_acc, effects) = step_cmd(&mut tw, GOD, "@gen/accept");
    drive_rooms(&mut tw, &llm, GOD, effects).await;

    // Reduce every persisted op to the final kv state of the worldgen namespace.
    let kv = final_kv(tw.take_module_ops());
    assert!(
        kv.iter()
            .any(|(k, _)| k.as_str() == format!("building:{}:{}", hall.0, GOD.0)),
        "the batch document was not persisted: {:?}",
        kv.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>()
    );

    // Boot 2: an identically-shaped world + a fresh module, handed the docs.
    let mock2 = Arc::new(MockProvider::new("openrouter"));
    let llm2 = batch_service(&mock2);
    let mut tw2 = TestWorld::with_modules(vec![Box::new(WorldgenModule::new(llm2))]);
    for name in ["Hall", "Parlor", "Study"] {
        tw2.world_mut()
            .create(ObjKind::Room, name, GOD, None)
            .unwrap();
    }
    dig_exit(&mut tw2, "north", hall, parlor);
    dig_exit(&mut tw2, "south", parlor, hall);
    dig_exit(&mut tw2, "north", parlor, study);
    dig_exit(&mut tw2, "south", study, parlor);
    tw2.world_mut().move_to(GOD, hall).unwrap();

    let mut boot = std::collections::HashMap::new();
    boot.insert(
        smol_str::SmolStr::new("worldgen"),
        mush_core::ModuleBootData {
            kv,
            streams: std::collections::HashMap::new(),
        },
    );
    tw2.modules_loaded(boot);

    // The reloaded batch is live: review lists it, and accepting a room draft
    // writes its desc.
    let (rev, _) = step_cmd(&mut tw2, GOD, "@gen/review here");
    assert!(
        contains(&sys_to(&rev, GOD), "rooms drafted"),
        "reloaded batch not listed: {:?}",
        sys_to(&rev, GOD)
    );
    let (accepted, _) = step_cmd(&mut tw2, GOD, "@gen/accept");
    assert!(
        contains(&sys_to(&accepted, GOD), "Accepted"),
        "{:?}",
        sys_to(&accepted, GOD)
    );
    assert_eq!(
        tw2.world().attr(hall, "desc").and_then(|a| a.as_text()),
        Some("A grand hall of veined marble.")
    );
}

/// Reduce a stream of module ops to the final kv state (last JsonSet wins;
/// Del removes) — the store's view after a boot's writes.
fn final_kv(ops: Vec<ModuleOp>) -> Vec<(smol_str::SmolStr, serde_json::Value)> {
    use std::collections::BTreeMap;
    let mut kv: BTreeMap<smol_str::SmolStr, serde_json::Value> = BTreeMap::new();
    for op in ops {
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.insert(key, value);
            }
            ModuleWrite::Del { key } => {
                kv.remove(&key);
            }
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv.into_iter().collect()
}

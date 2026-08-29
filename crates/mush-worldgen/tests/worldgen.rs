//! Integration tests for the worldgen module: themes, the `@gen` draft
//! pipeline, the async defer/re-entry path, and persistence — all against a
//! network-free [`MockProvider`]. No real LLM, no sockets.

use std::sync::Arc;

use mush_core::{
    Audience, Effect, Effects, GOD, Inbound, LIMBO, ModuleWrite, ObjId, ObjKind, Outbox, Powers,
    WorldCommand,
};
use mush_llm::{
    BudgetConfig, BudgetLimit, CompletionResponse, ContentBlock, LlmProvider, LlmService,
    MockProvider, ProviderCaps, RouteConfig, StopReason, Usage,
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

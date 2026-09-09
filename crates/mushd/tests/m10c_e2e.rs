//! M10c integration end-to-end: the three M10 depth features
//! (batch `@gen/building`, the weather LLM flavor top-up, and the staff LLM
//! cost dashboard) exercised together over the SAME module stack `main.rs`
//! boots, with a network-free [`MockProvider`] injected as the `LlmService`
//! (scripted responses; NO network, NO key). Nothing here makes a real LLM
//! call.
//!
//! These drive the engine deterministically (dispatch a command, run the
//! deferred `Effect::ModuleAsync` off-loop against the mock, re-enter the
//! `WorldCommand::ModuleAsyncDone` completion) — the exact seam `mushd`'s
//! world task wires in `spawn_effects`, minus the tokio timing. That keeps the
//! batch pipeline (sequential room re-entry) and the weather tick (seeded RNG)
//! deterministic.
//!
//! Inventory:
//! - `m10c_full_journey_building_and_dashboard` — a builder digs a zoned
//!   3-room building, `@gen/building` produces a reviewed brief then a room
//!   draft per room (all async), `@gen/review` + `@gen/accept/all` writes the
//!   descs, and `look` shows them; then `@llm/dashboard` renders staff
//!   aggregates (a mortal is refused), the `llm.dashboard` query (the web
//!   panel's keyframe path) returns aggregates with **no prompt text** (a
//!   mortal is refused), and the budget ledger + audit log recorded the mock
//!   calls. A non-builder's `@gen/building` is refused.
//! - `weather_flavor_generate_approve_and_tick_draws_approved` —
//!   `@weather/generate` caches PENDING flavor, `@weather/lines/approve` moves
//!   one into the live pool, and a subsequent tick draws the approved line
//!   while a pending line is NEVER shown.
//! - `m10c_persistence_twin_in_memory` / `..._redis_gated` — a building's
//!   drafts and an approved flavor line survive a reboot from the store alone.
//! - `m10c_reboot_mid_room_pass_downgrades_and_resumes` — a batch interrupted
//!   mid-room-pass (a room left `Generating`) is not wedged by a reboot: the
//!   orphan is downgraded to `Pending` and `@gen/review` re-arms the pass.
//! - `m10c_dashboard_role_gate_parity` — the LLM dashboard shares ONE floor
//!   (Wizard) across the web query and the terminal command: Wizard+ is served
//!   both, a Royalty/Builder is refused both.

mod support;

use std::sync::Arc;
use std::time::Duration;

use mush_core::{
    Audience, Effect, Effects, Engine, GOD, Inbound, ModuleWrite, ObjId, ObjKind, Outbox, World,
    WorldCommand,
};
use mush_llm::{
    BudgetConfig, BudgetLimit, CompletionResponse, ContentBlock, LlmProvider, LlmService,
    MockProvider, RouteConfig, StopReason, Usage,
};
use mush_proto::{OutputEvent, SmolStr};
use mush_store::{InMemoryStore, ObjectStore, WriteBatch};
use mush_testkit::TestWorld;
use mush_weather::{
    ClimateDef, ClimateSet, FlavorBucket, FlavorKey, WeatherConfig, WeatherModule, WeatherState,
    WxRng, run_weather_async,
};
use serde_json::json;

// A sentinel planted in a prompt (the building prose), used to prove the LLM
// dashboard/query carry aggregates ONLY — never any prompt text.
const PROMPT_SENTINEL: &str = "SECRETPROSE9137";

// ----------------------------------------------------------------------
// LLM harness (MockProvider — no network, no key)
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

/// A service backed by `mock` on the worldgen (room + brief) and weather
/// (ambience) routes, with layered budgets so the cost dashboard has caps to
/// render. `enabled = false` engages the global kill switch (the shape a
/// keyless clone boots in).
fn m10c_service(mock: &Arc<MockProvider>, enabled: bool) -> Arc<LlmService> {
    let concrete = Arc::clone(mock);
    let p: Arc<dyn LlmProvider> = concrete;
    let budget = BudgetConfig {
        global: Some(BudgetLimit {
            tokens: Some(5_000_000),
            cost_usd: Some(5.0),
        }),
        per_actor_default: Some(BudgetLimit::tokens(1_000_000)),
        per_feature: [("worldgen_room".to_string(), BudgetLimit::tokens(2_000_000))]
            .into_iter()
            .collect(),
        ..BudgetConfig::default()
    };
    Arc::new(
        LlmService::builder()
            .route("worldgen_room", route())
            .route("worldgen_brief", route())
            .route("ambience", route())
            .provider("openrouter", p)
            .budgets(budget)
            .global_enabled(enabled)
            .build(),
    )
}

fn tool_resp(name: &str, input: serde_json::Value) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::ToolUse {
            id: "t1".into(),
            name: name.into(),
            input,
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage {
            input_tokens: 200,
            output_tokens: 120,
            ..Usage::default()
        },
        model: "test/mock".into(),
    }
}

fn room_resp(desc: &str) -> CompletionResponse {
    tool_resp("submit_room", json!({ "desc": desc }))
}

fn brief_resp(input: serde_json::Value) -> CompletionResponse {
    tool_resp("submit_building_brief", input)
}

fn flavor_resp(transition: &[&str], ambient: &[&str]) -> CompletionResponse {
    tool_resp(
        "submit_flavor",
        json!({ "transition": transition, "ambient": ambient }),
    )
}

// ----------------------------------------------------------------------
// Engine driving (dispatch → run off-loop → re-enter), routed by module NS —
// exactly what `mushd`'s `spawn_effects` does, minus tokio.
// ----------------------------------------------------------------------

fn step(
    engine: &mut Engine,
    actor: ObjId,
    line: &str,
) -> (Vec<(Audience, OutputEvent)>, Vec<Effect>) {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine.step(
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

fn apply(
    engine: &mut Engine,
    module: &str,
    actor: ObjId,
    correlation: u64,
    payload: String,
) -> (Vec<(Audience, OutputEvent)>, Vec<Effect>) {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine.step(
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

/// Run the module's off-loop async work for a payload, routed by namespace —
/// the same dispatch `mushd::world_task::spawn_effects` performs.
async fn run_async(module: &str, payload: &str, llm: &LlmService) -> String {
    if module == mush_worldgen::NS {
        mush_worldgen::run_worldgen_async(llm, payload).await
    } else if module == mush_weather::NS {
        run_weather_async(llm, payload).await
    } else {
        panic!("unexpected module async: {module}")
    }
}

/// Dispatch `line`, then drive EVERY deferred `Effect::ModuleAsync` it (and
/// any completion it re-defers — the batch room pipeline) produces to
/// exhaustion, collecting all emitted events. Deterministic (no tokio).
async fn run_cmd(
    engine: &mut Engine,
    llm: &LlmService,
    actor: ObjId,
    line: &str,
) -> Vec<(Audience, OutputEvent)> {
    let (mut all, mut effects) = step(engine, actor, line);
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
        let result = run_async(&module, &payload, llm).await;
        let (events, fx) = apply(engine, &module, actor, correlation, result);
        all.extend(events);
        effects = fx;
    }
    all
}

/// The first deferred `Effect::ModuleAsync` in `effects` as
/// `(module, correlation, payload)` — used to drive the batch pipeline one
/// room at a time (so a test can interrupt it mid-pass).
fn one_module_async(effects: &[Effect]) -> Option<(String, u64, String)> {
    effects.iter().find_map(|e| match e {
        Effect::ModuleAsync {
            module,
            correlation,
            payload,
            ..
        } => Some((module.to_string(), *correlation, payload.clone())),
        _ => None,
    })
}

/// The `Sys` lines addressed to `actor` in `events`.
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

/// The first object named `name` (case-insensitive), for pulling ids out of
/// `@create`/`@dig` without parsing output.
fn find_obj(w: &World, name: &str) -> ObjId {
    w.iter()
        .find(|o| o.name.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| panic!("no object named {name:?}"))
        .id
}

/// Dispatch a `llm.dashboard` query as `actor` (conn 7) — the web bridge's
/// `PanelSubscribe{feed:"llm.dashboard"}` routing target — and return the
/// wrapped reply event.
fn dashboard_query(engine: &mut Engine, actor: ObjId) -> OutputEvent {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine.step(
        Inbound::Query {
            conn: 7,
            actor,
            id: 42,
            name: SmolStr::new("llm.dashboard"),
            args: json!({}),
        },
        &mut out,
        &mut fx,
    );
    for (aud, ev) in out.take() {
        if let (Audience::Conn(7), OutputEvent::Reply { id: 42, event }) = (aud, ev) {
            return *event;
        }
    }
    panic!("no correlated llm.dashboard reply");
}

// ----------------------------------------------------------------------
// Full journey: batch @gen/building + the LLM cost dashboard
// ----------------------------------------------------------------------

/// The brief the mock returns for the 3-room building (concepts by index).
fn speakeasy_brief() -> serde_json::Value {
    json!({
        "concept": "A prohibition-era speakeasy gone to seed",
        "palette": "brass rails, gin, rain on tin, gaslight",
        "rooms": [
            { "index": 1, "name": "Hall",   "concept": "smoke-stained entry" },
            { "index": 2, "name": "Parlor", "concept": "velvet gambling parlor" },
            { "index": 3, "name": "Study",  "concept": "ledger-cluttered back office" }
        ]
    })
}

#[tokio::test]
async fn m10c_full_journey_building_and_dashboard() {
    const HALL_DESC: &str = "A smoke-stained entry hall, brass rails green with age.";
    const PARLOR_DESC: &str = "A velvet parlor where a faro table sags under dust.";
    const STUDY_DESC: &str = "A back office, ledgers swollen with damp under a bare bulb.";

    let mock = Arc::new(MockProvider::new("openrouter"));
    let llm = m10c_service(&mock, true);
    // The real production module stack: comsys/mail/jobs/weather/script +
    // worldgen (which owns @llm + the llm.dashboard query handler).
    let mut mods = mushd::modules::server_modules();
    mods.push(mushd::modules::worldgen_module(Arc::clone(&llm)));
    let mut tw = TestWorld::with_modules(mods);

    // --- Dig a zoned 3-room building off Limbo (Hall entrance). ----------
    tw.run(GOD, "@create Speakeasy");
    let zone = find_obj(tw.world(), "Speakeasy");
    tw.run(GOD, &format!("@set #{}=ZONE_MASTER", zone.0));
    tw.run(GOD, "@dig Hall=hall,out");
    let hall = find_obj(tw.world(), "Hall");
    tw.run(GOD, &format!("@zone #{}=#{}", hall.0, zone.0));
    tw.run(GOD, "hall"); // move through the exit into the Hall
    assert_eq!(tw.world().get(GOD).and_then(|o| o.location), Some(hall));
    tw.run(GOD, "@dig Parlor=north,south");
    let parlor = find_obj(tw.world(), "Parlor");
    tw.run(GOD, &format!("@zone #{}=#{}", parlor.0, zone.0));
    tw.run(GOD, "@dig Study=up,down");
    let study = find_obj(tw.world(), "Study");
    tw.run(GOD, &format!("@zone #{}=#{}", study.0, zone.0));

    // --- @gen/building: reviewed brief, then a room draft per room. -------
    mock.push_response(brief_resp(speakeasy_brief()));
    mock.push_response(room_resp(HALL_DESC));
    mock.push_response(room_resp(PARLOR_DESC));
    mock.push_response(room_resp(STUDY_DESC));

    let prose = format!("A crumbling {PROMPT_SENTINEL} behind a barbershop; brass, gin and rain.");
    let brief_events = run_cmd(
        tw.engine_mut(),
        &llm,
        GOD,
        &format!("@gen/building here={prose}"),
    )
    .await;
    assert!(
        contains(&sys_to(&brief_events, GOD), "Brief ready"),
        "brief pass must surface a reviewable brief: {:?}",
        sys_to(&brief_events, GOD)
    );
    // Nothing is committed by the brief.
    for r in [hall, parlor, study] {
        assert!(
            tw.world().attr(r, "desc").is_none(),
            "brief must not commit"
        );
    }

    // Accept the brief -> the completion-pipelined room pass runs all rooms.
    let room_events = run_cmd(tw.engine_mut(), &llm, GOD, "@gen/accept").await;
    assert!(
        contains(&sys_to(&room_events, GOD), "Drafted"),
        "room pass must stream progress: {:?}",
        sys_to(&room_events, GOD)
    );
    // brief + one call per room.
    assert_eq!(mock.call_count(), 4, "brief + 3 rooms");
    // Still nothing committed (drafts await review).
    for r in [hall, parlor, study] {
        assert!(
            tw.world().attr(r, "desc").is_none(),
            "room pass must not auto-commit {r}"
        );
    }

    // --- @gen/review + accept: write every drafted room's desc. ----------
    let review = tw.run(GOD, "@gen/review here");
    assert!(
        !sys_to(&review, GOD).is_empty(),
        "@gen/review must list the batch"
    );
    let accepted = tw.run(GOD, "@gen/accept/all");
    assert!(
        contains(&sys_to(&accepted, GOD), "Accepted") || contains(&sys_to(&accepted, GOD), "wrote"),
        "@gen/accept/all must confirm: {:?}",
        sys_to(&accepted, GOD)
    );
    // Every room now carries a committed desc (written through the attr path).
    for r in [hall, parlor, study] {
        assert!(
            tw.world()
                .attr(r, "desc")
                .and_then(|a| a.as_text())
                .is_some_and(|d| !d.trim().is_empty()),
            "room {r} must have a desc after @gen/accept/all"
        );
    }
    // --- look shows the committed desc (in the Hall, GOD's room). --------
    let hall_desc = tw
        .world()
        .attr(hall, "desc")
        .and_then(|a| a.as_text())
        .unwrap()
        .to_string();
    let look = tw.run(GOD, "look");
    assert!(
        tw.seen_by(&look, GOD).join("\n").contains(&hall_desc),
        "look must render the committed desc"
    );

    // --- The budget ledger + audit log recorded the mock calls. ----------
    assert!(
        llm.ledger().global().total_tokens() > 0,
        "the budget ledger must have recorded the mock calls"
    );
    assert!(
        !llm.audit_log().recent(20).is_empty(),
        "the audit log must have recorded the mock calls"
    );

    // --- @llm/dashboard (staff terminal) renders; a mortal is refused. ---
    let dash = tw.run(GOD, "@llm/dashboard");
    let dash_lines = sys_to(&dash, GOD);
    assert!(
        contains(&dash_lines, "dashboard") || contains(&dash_lines, "kill switch"),
        "@llm/dashboard must render for staff: {dash_lines:?}"
    );
    let mortal = tw.spawn_player("Mortal");
    let refused = tw.run(mortal, "@llm/dashboard");
    assert!(
        contains(&sys_to(&refused, mortal), "Permission denied"),
        "a mortal must be refused @llm/dashboard: {:?}",
        sys_to(&refused, mortal)
    );

    // --- The llm.dashboard web query returns aggregates with NO prompt
    //     text (the panel keyframe path); a mortal query is refused. -------
    let ev = dashboard_query(tw.engine_mut(), GOD);
    let OutputEvent::Panel { feed, data, .. } = &ev else {
        panic!("staff llm.dashboard query must reply with a Panel keyframe, got {ev:?}");
    };
    assert_eq!(feed, "llm.dashboard");
    assert!(
        data.get("budgets")
            .and_then(|b| b.as_array())
            .is_some_and(|b| !b.is_empty()),
        "the dashboard must carry budget rows: {data}"
    );
    assert!(
        data.get("recent_calls")
            .and_then(|c| c.as_array())
            .is_some_and(|c| !c.is_empty()),
        "the dashboard must carry recent calls: {data}"
    );
    // GOD is the top actor by spend (all calls were attributed to the builder).
    let top_actors = serde_json::to_string(&data["top_actors"]).unwrap();
    assert!(
        top_actors.contains(&format!("#{}", GOD.0)),
        "top actors must include the builder: {top_actors}"
    );
    // The whole payload is aggregates only — the prompt sentinel never leaks.
    let whole = serde_json::to_string(data).unwrap();
    assert!(
        !whole.contains(PROMPT_SENTINEL),
        "the dashboard must carry NO prompt text; it leaked {PROMPT_SENTINEL}:\n{whole}"
    );

    let mortal_ev = dashboard_query(tw.engine_mut(), mortal);
    let OutputEvent::Sys(t) = &mortal_ev else {
        panic!("a mortal llm.dashboard query must be refused with Sys, got {mortal_ev:?}");
    };
    assert!(
        t.to_plain_text().to_lowercase().contains("staff"),
        "the refusal must say staff-only: {}",
        t.to_plain_text()
    );

    // --- A non-builder's @gen/building is refused (no LLM work deferred). -
    tw.world_mut().move_to(mortal, hall).unwrap();
    let (nb_events, nb_effects) = step(tw.engine_mut(), mortal, "@gen/building here=let me build.");
    assert!(
        contains(&sys_to(&nb_events, mortal), "Permission denied"),
        "a non-builder must be refused: {:?}",
        sys_to(&nb_events, mortal)
    );
    assert!(
        nb_effects.is_empty(),
        "no LLM work may be deferred for a non-builder"
    );
}

// ----------------------------------------------------------------------
// Weather LLM flavor: generate -> approve -> a tick draws the approved line
// ----------------------------------------------------------------------

/// A barren climate (clear/rain, EMPTY hand-written line pools) so the
/// approved overlay is the sole tick candidate — the deterministic way to
/// prove "the tick draws the approved line, never a pending one".
const BARREN_TOML: &str = r#"
id = "barren"
name = "Barren"
[[states]]
id = "clear"
visibility = 100
precip = 0
wind = 10
temp_band = [0, 20]
[[states]]
id = "rain"
visibility = 60
precip = 60
wind = 40
temp_band = [2, 18]
[seasonal.winter]
clear = { rain = 100 }
rain = { rain = 100 }
[seasonal.spring]
clear = { rain = 100 }
rain = { rain = 100 }
[seasonal.summer]
clear = { rain = 100 }
rain = { rain = 100 }
[seasonal.autumn]
clear = { rain = 100 }
rain = { rain = 100 }
"#;

fn weather_to_room(events: &[(Audience, OutputEvent)], room: ObjId) -> Vec<String> {
    events
        .iter()
        .filter_map(|(aud, ev)| match (aud, ev) {
            (Audience::Room { room: r, .. }, OutputEvent::Weather { text, .. }) if *r == room => {
                Some(text.to_plain_text())
            }
            _ => None,
        })
        .collect()
}

fn seed_rng(tw: &mut TestWorld, s: u64) {
    tw.world_mut()
        .module_state_mut::<WeatherState>()
        .unwrap()
        .rng = WxRng::seeded(s);
}

#[tokio::test]
async fn weather_flavor_generate_approve_and_tick_draws_approved() {
    const APPROVED: &str = "Rain sheets off the tin awnings in the dark.";

    let barren = ClimateDef::parse("barren", BARREN_TOML).unwrap();
    let module = WeatherModule::with_climates(
        ClimateSet::from_defs([barren]),
        WeatherConfig {
            tick: Duration::from_secs(60),
            ambient_permille: 1000,
            ..WeatherConfig::default()
        },
    );
    let mut tw = TestWorld::with_modules(vec![Box::new(module)]);

    // A zone + one OUTDOORS room on the barren climate; pin to rain so no
    // transition steals the tick. S2b: the cell keys on the starter lot's OOC
    // Area, and the audience stands at its outdoor bus stop.
    let room = tw
        .world()
        .iter()
        .find(|o| o.kind == ObjKind::Room && o.name == "Bus Stop (OOC)")
        .expect("the starter lot is applied")
        .id;
    tw.run(GOD, "@weather/climate OOC Area=barren");
    tw.run(GOD, "@weather OOC Area=rain");

    // A player in the room so the tick has an audience.
    let player = tw.spawn_player("Rae");
    tw.world_mut().move_to(player, room).unwrap();

    // --- @weather/generate: one ambient variant lands PENDING. -----------
    let mock = Arc::new(MockProvider::new("openrouter"));
    mock.push_response(flavor_resp(&[], &[APPROVED]));
    let llm = m10c_service(&mock, true);
    let gen_out = run_cmd(tw.engine_mut(), &llm, GOD, "@weather/generate barren rain").await;
    assert!(
        contains(&sys_to(&gen_out, GOD), "PENDING"),
        "generate must report pending lines: {:?}",
        sys_to(&gen_out, GOD)
    );
    // Not live yet.
    assert!(
        tw.world()
            .module_state::<WeatherState>()
            .unwrap()
            .flavor
            .approved_texts("barren", "rain", FlavorBucket::Ambient)
            .is_empty(),
        "a freshly generated line must be pending, not live"
    );

    // A tick before approval shows NOTHING (barren pool + not-approved overlay).
    seed_rng(&mut tw, 7);
    let before = tw.fire_due(60_000);
    assert!(
        weather_to_room(&before, room).is_empty(),
        "a pending line must never be shown: {:?}",
        weather_to_room(&before, room)
    );

    // --- Approve it -> it enters the live pool. --------------------------
    let key = FlavorKey::new("barren", "rain", FlavorBucket::Ambient);
    let pending_id = tw
        .world()
        .module_state::<WeatherState>()
        .unwrap()
        .flavor
        .pool(&key)
        .iter()
        .find(|l| !l.approved)
        .map(|l| l.id)
        .expect("a pending line");
    tw.run(GOD, &format!("@weather/lines/approve {pending_id}"));
    assert_eq!(
        tw.world()
            .module_state::<WeatherState>()
            .unwrap()
            .flavor
            .approved_texts("barren", "rain", FlavorBucket::Ambient),
        vec![APPROVED.to_string()],
        "approve must move the line into the live pool"
    );

    // --- A subsequent tick now draws the approved line. ------------------
    seed_rng(&mut tw, 7);
    let after = tw.fire_due(120_000);
    assert_eq!(
        weather_to_room(&after, room),
        vec![APPROVED.to_string()],
        "after approval the tick draws the approved line"
    );
}

// ----------------------------------------------------------------------
// Persistence twin: a building's drafts + an approved flavor line survive a
// reboot from the store alone (module-op write-through + boot reload).
// ----------------------------------------------------------------------

/// A worldgen + weather engine (core commands + the two modules), the boot
/// shape `mushd::modules::boot_modules` reloads against.
fn wg_weather_engine(llm: Arc<LlmService>) -> Engine {
    let registry = mushd::world_task::base_registry();
    let mut world = World::new_minimal();
    world.set_now(0);
    Engine::with_modules(
        world,
        registry,
        vec![
            mushd::modules::worldgen_module(Arc::clone(&llm)),
            Box::new(WeatherModule::new()),
        ],
    )
    .unwrap()
}

/// Dig the 3-room building topology directly (Hall entrance <-> Parlor <->
/// Study), returning the ids. Unzoned — the walk falls back to hop count.
fn dig_building(e: &mut Engine) -> [ObjId; 3] {
    let hall = e.world.create(ObjKind::Room, "Hall", GOD, None).unwrap();
    let parlor = e.world.create(ObjKind::Room, "Parlor", GOD, None).unwrap();
    let study = e.world.create(ObjKind::Room, "Study", GOD, None).unwrap();
    for (name, from, to) in [
        ("north", hall, parlor),
        ("south", parlor, hall),
        ("north", parlor, study),
        ("south", study, parlor),
    ] {
        let ex = e
            .world
            .create(ObjKind::Exit, name, GOD, Some(from))
            .unwrap();
        e.world.link_exit(ex, Some(to)).unwrap();
    }
    e.world.move_to(GOD, hall).unwrap();
    [hall, parlor, study]
}

/// The full twin over `store`: boot 1 produces a building batch (brief + room
/// drafts) and an approved weather flavor line, writes both through the store;
/// boot 2 reloads from the store alone and asserts the batch is live and the
/// flavor line is approved again.
async fn m10c_round_trip(store: Arc<dyn ObjectStore>) {
    // --- Boot 1: generate a building batch + approve a flavor line. -------
    let mock = Arc::new(MockProvider::new("openrouter"));
    let llm = m10c_service(&mock, true);
    let mut e1 = wg_weather_engine(Arc::clone(&llm));
    let [hall, parlor, study] = dig_building(&mut e1);

    mock.push_response(brief_resp(speakeasy_brief()));
    mock.push_response(room_resp("A grand hall of veined marble."));
    mock.push_response(room_resp("A smoky parlor of worn velvet."));
    mock.push_response(room_resp("A book-lined study."));
    // Brief pass.
    run_cmd(
        &mut e1,
        &llm,
        GOD,
        "@gen/building here=A grand jazz-age mansion.",
    )
    .await;
    // Room pass.
    run_cmd(&mut e1, &llm, GOD, "@gen/accept").await;

    // A weather flavor line, approved.
    mock.push_response(flavor_resp(
        &["The rain hardens."],
        &["Rain ticks on the awnings."],
    ));
    run_cmd(
        &mut e1,
        &llm,
        GOD,
        "@weather/generate temperate-coastal rain",
    )
    .await;
    let key = FlavorKey::new("temperate-coastal", "rain", FlavorBucket::Ambient);
    let pending_id = e1
        .world
        .module_state::<WeatherState>()
        .unwrap()
        .flavor
        .pool(&key)
        .iter()
        .find(|l| !l.approved)
        .map(|l| l.id)
        .expect("a pending ambient line");
    step(
        &mut e1,
        GOD,
        &format!("@weather/lines/approve {pending_id}"),
    );

    // Write every queued module op (worldgen drafts + weather flavor) through
    // the store, durably (as the world task's flush_dirty does).
    let ops = e1.take_module_ops();
    assert!(
        ops.iter().any(|op| matches!(&op.op,
            ModuleWrite::JsonSet { key, .. } if key.starts_with("building:"))),
        "the building batch must persist"
    );
    assert!(
        ops.iter().any(|op| matches!(&op.op,
            ModuleWrite::JsonSet { key, .. } if key.starts_with("flavor:"))),
        "the approved flavor pool must persist"
    );
    store
        .write(WriteBatch {
            serial: 1,
            durable: true,
            module_ops: ops,
            ..WriteBatch::default()
        })
        .await
        .unwrap();
    drop(e1); // the "old" process is gone; only the store survives.

    // --- Boot 2: reload from the store alone. ----------------------------
    let mock2 = Arc::new(MockProvider::new("openrouter"));
    let llm2 = m10c_service(&mock2, true);
    let mut e2 = wg_weather_engine(llm2);
    // Recreate the building topology identically (same deterministic ids the
    // drafts name); flavor is climate-keyed and needs no world objects.
    let [hall2, parlor2, study2] = dig_building(&mut e2);
    assert_eq!(
        [hall2, parlor2, study2],
        [hall, parlor, study],
        "ids must match"
    );
    // The mushd boot path: migrations, load each namespace, on_world_loaded.
    mushd::modules::boot_modules(&mut e2, &*store)
        .await
        .unwrap();

    // The reloaded building batch is live: review lists it, and accepting
    // writes a reloaded room draft's desc (no regeneration).
    let (rev, _) = step(&mut e2, GOD, "@gen/review here");
    assert!(
        !sys_to(&rev, GOD).is_empty(),
        "the reloaded batch must list: {:?}",
        sys_to(&rev, GOD)
    );
    step(&mut e2, GOD, "@gen/accept/all");
    for r in [hall2, parlor2, study2] {
        assert!(
            e2.world
                .attr(r, "desc")
                .and_then(|a| a.as_text())
                .is_some_and(|d| !d.trim().is_empty()),
            "a reloaded room draft must accept-write its desc for {r}"
        );
    }
    assert_eq!(
        mock2.call_count(),
        0,
        "reboot + accept must not call the provider"
    );

    // The approved flavor line is live again after the reboot.
    let live = e2
        .world
        .module_state::<WeatherState>()
        .unwrap()
        .flavor
        .approved_texts("temperate-coastal", "rain", FlavorBucket::Ambient);
    assert_eq!(
        live,
        vec!["Rain ticks on the awnings.".to_string()],
        "the approved flavor line must survive the reboot"
    );
}

#[tokio::test]
async fn m10c_persistence_twin_in_memory() {
    m10c_round_trip(Arc::new(InMemoryStore::new())).await;
}

/// The same twin over a live Redis — the reboot-from-Redis-alone surrogate.
/// Gated on `REDIS_URL`; runs in db 6 (dbs 7–15 are claimed by the other
/// gated tests, which run in parallel).
#[tokio::test(flavor = "multi_thread")]
async fn m10c_persistence_twin_redis_gated() {
    let Some(url) = std::env::var("REDIS_URL").ok().filter(|s| !s.is_empty()) else {
        eprintln!(
            "skipping m10c_persistence_twin_redis_gated: REDIS_URL not set \
             (no live Redis on this machine)"
        );
        return;
    };
    let url = support::redis_url_with_db(&url, 6);
    let store = mush_store::RedisStore::connect(&url)
        .await
        .expect("connect to REDIS_URL");
    store.wipe_namespaces().await.expect("wipe test namespaces");
    m10c_round_trip(Arc::new(store)).await;
}

// ----------------------------------------------------------------------
// Reboot DURING the room pass: an interrupted batch must not wedge. A room
// left `Generating` when the process dies is downgraded to `Pending` on
// reload, and the next `@gen/review` re-arms the stalled pass.
// ----------------------------------------------------------------------

#[tokio::test]
async fn m10c_reboot_mid_room_pass_downgrades_and_resumes() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());

    // --- Boot 1: start the room pass, interrupt after the FIRST room. -----
    let mock = Arc::new(MockProvider::new("openrouter"));
    let llm = m10c_service(&mock, true);
    let mut e1 = wg_weather_engine(Arc::clone(&llm));
    let [hall, parlor, study] = dig_building(&mut e1);

    // Only the brief + the first room are queued; the pass is interrupted
    // before rooms 2 and 3 are ever generated.
    mock.push_response(brief_resp(speakeasy_brief()));
    mock.push_response(room_resp("A grand hall of veined marble."));

    // Brief pass -> reviewable brief.
    run_cmd(
        &mut e1,
        &llm,
        GOD,
        "@gen/building here=A grand jazz-age mansion.",
    )
    .await;

    // Accept the brief -> defers room 1 (Hall).
    let (_out, effects) = step(&mut e1, GOD, "@gen/accept");
    // Run EXACTLY room 1's async, re-enter -> Hall drafts and room 2 (Parlor)
    // is deferred as `Generating`, which we deliberately never run.
    let (module, correlation, payload) = one_module_async(&effects).expect("room 1 deferred");
    let result = run_async(&module, &payload, &llm).await;
    apply(&mut e1, &module, GOD, correlation, result);

    // At the interruption: Hall drafted, Parlor `Generating` (in flight, lost),
    // Study pending; batch status Generating.
    {
        let st = e1
            .world
            .module_state::<mush_worldgen::WorldgenState>()
            .unwrap();
        let batch = st.brief(hall, GOD).expect("batch present");
        assert_eq!(batch.status, mush_worldgen::BatchStatus::Generating);
        assert_eq!(
            batch.room(parlor).map(|r| r.status),
            Some(mush_worldgen::RoomState::Generating),
            "room 2 is in flight at the interruption"
        );
    }

    // Persist everything queued so far and drop the "old" process.
    let ops = e1.take_module_ops();
    store
        .write(WriteBatch {
            serial: 1,
            durable: true,
            module_ops: ops,
            ..WriteBatch::default()
        })
        .await
        .unwrap();
    drop(e1);

    // --- Boot 2: reload from the store alone. -----------------------------
    let mock2 = Arc::new(MockProvider::new("openrouter"));
    let llm2 = m10c_service(&mock2, true);
    let mut e2 = wg_weather_engine(Arc::clone(&llm2));
    let [hall2, parlor2, study2] = dig_building(&mut e2);
    assert_eq!([hall2, parlor2, study2], [hall, parlor, study], "ids match");
    mushd::modules::boot_modules(&mut e2, &*store)
        .await
        .unwrap();

    // The orphaned `Generating` room was downgraded to `Pending`; the batch is
    // still `Generating` (stalled), with NO room stuck mid-generation.
    {
        let st = e2
            .world
            .module_state::<mush_worldgen::WorldgenState>()
            .unwrap();
        let batch = st.brief(hall, GOD).expect("batch reloaded");
        assert_eq!(
            batch.status,
            mush_worldgen::BatchStatus::Generating,
            "the stalled batch stays generating until re-armed"
        );
        assert!(
            batch
                .rooms
                .iter()
                .all(|r| r.status != mush_worldgen::RoomState::Generating),
            "no room may remain `Generating` after a reboot: {:?}",
            batch
                .rooms
                .iter()
                .map(|r| (r.id, r.status))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            batch.room(parlor).map(|r| r.status),
            Some(mush_worldgen::RoomState::Pending),
            "the lost in-flight room is downgraded to pending"
        );
    }

    // `@gen/review` re-arms the stalled pass; queue the two remaining rooms.
    mock2.push_response(room_resp("A smoky parlor of worn velvet."));
    mock2.push_response(room_resp("A book-lined study."));
    run_cmd(&mut e2, &llm2, GOD, "@gen/review here").await;

    // All three rooms drafted now; accept writes every desc (Hall's survived).
    run_cmd(&mut e2, &llm2, GOD, "@gen/accept/all").await;
    for r in [hall2, parlor2, study2] {
        assert!(
            e2.world
                .attr(r, "desc")
                .and_then(|a| a.as_text())
                .is_some_and(|d| !d.trim().is_empty()),
            "every room must have a committed desc after resume + accept: {r}"
        );
    }
    // Only the two un-drafted rooms were regenerated on resume; Hall's draft
    // survived the reboot and was NOT re-called.
    assert_eq!(
        mock2.call_count(),
        2,
        "resume regenerates only the 2 rooms without a surviving draft"
    );
}

// ----------------------------------------------------------------------
// Role-gate parity: the LLM dashboard shares ONE floor (Wizard) across both
// surfaces. Wizard+ is served the web query AND the terminal command; a
// Royalty (or Builder) is refused BOTH — never served one and refused the
// other.
// ----------------------------------------------------------------------

#[tokio::test]
async fn m10c_dashboard_role_gate_parity() {
    use mush_core::Flags;

    let mock = Arc::new(MockProvider::new("openrouter"));
    let llm = m10c_service(&mock, true);
    let mut mods = mushd::modules::server_modules();
    mods.push(mushd::modules::worldgen_module(Arc::clone(&llm)));
    let mut tw = TestWorld::with_modules(mods);

    // GOD is Wizard+. Add an exactly-Royalty and an exactly-Builder player.
    let royalty = tw.spawn_player("Roya");
    tw.world_mut()
        .set_flag(royalty, Flags::ROYALTY, true)
        .unwrap();
    let builder = tw.spawn_player("Buildy");
    tw.world_mut()
        .set_flag(builder, Flags::BUILDER, true)
        .unwrap();

    // --- Web query (the llm.dashboard keyframe path). ---------------------
    // Wizard+ (GOD) is served a Panel keyframe.
    let god_ev = dashboard_query(tw.engine_mut(), GOD);
    assert!(
        matches!(god_ev, OutputEvent::Panel { .. }),
        "Wizard+ must be served the web dashboard, got {god_ev:?}"
    );
    // Royalty and Builder are refused with a Sys line (NOT served a keyframe).
    for (who, label) in [(royalty, "Royalty"), (builder, "Builder")] {
        let ev = dashboard_query(tw.engine_mut(), who);
        let OutputEvent::Sys(t) = &ev else {
            panic!("{label} must be refused the web dashboard, got {ev:?}");
        };
        assert!(
            t.to_plain_text().to_lowercase().contains("staff"),
            "{label} web refusal must say staff-only: {}",
            t.to_plain_text()
        );
    }

    // --- Terminal @llm/dashboard (same aggregates, same floor). -----------
    let god_dash = sys_to(&tw.run(GOD, "@llm/dashboard"), GOD);
    assert!(
        contains(&god_dash, "dashboard") || contains(&god_dash, "kill switch"),
        "Wizard+ must be served the terminal dashboard: {god_dash:?}"
    );
    for (who, label) in [(royalty, "Royalty"), (builder, "Builder")] {
        let refused = sys_to(&tw.run(who, "@llm/dashboard"), who);
        assert!(
            contains(&refused, "Permission denied"),
            "{label} must be refused the terminal dashboard: {refused:?}"
        );
    }
}

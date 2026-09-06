//! End-to-end tests for the M10 weather LLM flavor top-up (MockProvider —
//! NO network, NO key). Drives the real production path: `@weather/generate`
//! defers an `Effect::ModuleAsync`, the off-loop `run_weather_async` runs
//! against a MockProvider-backed `LlmService`, and the result re-enters as
//! `WorldCommand::ModuleAsyncDone` — exactly as `mushd`'s world task wires it.
//!
//! Covered: generate produces PENDING lines cached via module ops (and the
//! prompt carries the climate style + zone theme); `@weather/lines` lists
//! hand-written vs pending vs approved distinctly; approve moves a line into
//! the live pool; tick-time selection only draws hand-written + approved (a
//! pending line is NEVER shown); the disabled-LLM path is inert; the pool
//! persists across a reboot; and a non-staff caller is refused.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use mush_core::{
    Attr, Audience, Effect, Effects, Flags, GOD, Inbound, LIMBO, ObjId, ObjKind, Outbox,
    WorldCommand,
};
use mush_llm::{
    CompletionResponse, ContentBlock, LlmProvider, LlmService, MockProvider, RouteConfig,
    StopReason, Usage,
};
use mush_proto::OutputEvent;
use mush_store::{InMemoryStore, ObjectStore, WriteBatch};
use mush_testkit::TestWorld;
use mush_weather::{
    ClimateDef, ClimateSet, FlavorBucket, FlavorKey, MAX_FLAVOR_LINES_PER_POOL, WeatherConfig,
    WeatherModule, WeatherState, WxRng, run_weather_async,
};
use serde_json::json;
use smol_str::SmolStr;

const CLIMATE: &str = "temperate-coastal";

// ----------------------------------------------------------------------
// LLM harness (MockProvider — no network, no key)
// ----------------------------------------------------------------------

fn ambience_route() -> RouteConfig {
    RouteConfig {
        model: "deepseek/deepseek-v4-flash-0731".into(),
        max_tokens: 400,
        timeout_s: 30,
        reasoning: Some("off".into()),
        reasoning_max_tokens: None,
    }
}

/// A service backed by `mock` on the `ambience` route, enabled or (kill-switch
/// engaged) disabled.
fn service(mock: &Arc<MockProvider>, enabled: bool) -> Arc<LlmService> {
    let concrete = Arc::clone(mock);
    let p: Arc<dyn LlmProvider> = concrete;
    Arc::new(
        LlmService::builder()
            .route("ambience", ambience_route())
            .provider("openrouter", p)
            .global_enabled(enabled)
            .build(),
    )
}

/// A canned `submit_flavor` tool call carrying transition + ambient arrays.
fn flavor_resp(transition: &[&str], ambient: &[&str]) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::ToolUse {
            id: "t1".into(),
            name: "submit_flavor".into(),
            input: json!({ "transition": transition, "ambient": ambient }),
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage {
            input_tokens: 60,
            output_tokens: 40,
            ..Usage::default()
        },
        model: "test/mock".into(),
    }
}

/// The concatenated user-turn text a request carried (what the model saw).
fn prompt_text(req: &mush_llm::CompletionRequest) -> String {
    let mut s = String::new();
    for m in &req.messages {
        for b in &m.content {
            if let ContentBlock::Text { text } = b {
                s.push_str(text);
                s.push('\n');
            }
        }
    }
    s
}

// ----------------------------------------------------------------------
// Engine driving helpers (capture deferred effects; inject completions)
// ----------------------------------------------------------------------

/// Dispatch one line and return `(events, deferred effects)`.
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

/// Re-enter one finished module-async as `WorldCommand::ModuleAsyncDone`.
fn inject_done(
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

/// The full `@weather/generate` round trip: dispatch, run every deferred
/// generator off-loop against `llm`, and re-enter each completion. Returns the
/// completion output events.
async fn generate(
    tw: &mut TestWorld,
    llm: &LlmService,
    actor: ObjId,
    line: &str,
) -> Vec<(Audience, OutputEvent)> {
    let (_, effects) = step_cmd(tw, actor, line);
    let mut all = Vec::new();
    for e in effects {
        if let Effect::ModuleAsync {
            module,
            actor: a,
            correlation,
            payload,
        } = e
        {
            let result = run_weather_async(llm, &payload).await;
            all.extend(inject_done(tw, module.as_str(), a, correlation, result));
        }
    }
    all
}

// ----------------------------------------------------------------------
// Fixtures
// ----------------------------------------------------------------------

/// A world with the weather module over the shipped climates.
fn world() -> TestWorld {
    let module = WeatherModule::with_config(WeatherConfig {
        tick: Duration::from_secs(60),
        ambient_permille: 1000,
        ..WeatherConfig::default()
    });
    TestWorld::with_modules(vec![Box::new(module)])
}

/// Create a zone + one OUTDOORS room, assign `CLIMATE`, return `(zone, room)`.
fn zone_with_climate(tw: &mut TestWorld) -> (ObjId, ObjId) {
    let zone = tw
        .world_mut()
        .create(ObjKind::Thing, "Downtown", GOD, Some(LIMBO))
        .unwrap();
    let room = tw
        .world_mut()
        .create(ObjKind::Room, "Plaza", GOD, None)
        .unwrap();
    tw.world_mut().set_zone(room, Some(zone)).unwrap();
    tw.world_mut()
        .set_flag(room, Flags::OUTDOORS, true)
        .unwrap();
    tw.run(GOD, &format!("@weather/climate #{}={CLIMATE}", zone.0));
    (zone, room)
}

fn flavor_ops(tw: &mut TestWorld) -> Vec<(String, serde_json::Value)> {
    tw.take_module_ops()
        .into_iter()
        .filter_map(|op| match op.op {
            mush_core::ModuleWrite::JsonSet { key, value } if key.starts_with("flavor:") => {
                Some((key.to_string(), value))
            }
            _ => None,
        })
        .collect()
}

// ----------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------

#[tokio::test]
async fn generate_caches_pending_lines_and_prompt_carries_style_and_theme() {
    let mut tw = world();
    let (zone, _room) = zone_with_climate(&mut tw);
    // A worldgen theme on the zone — weather reads it for setting-consistency.
    tw.world_mut()
        .set_attr(
            zone,
            "worldgen.theme",
            Attr::json(
                json!({
                    "style_guide": "rain-slick neon streets of a drowned city",
                    "vocabulary": ["neon", "puddle"],
                    "banned": ["sunshine"]
                }),
                GOD,
            ),
        )
        .unwrap();
    tw.take_module_ops(); // drop the assign/theme cell ops

    let mock = Arc::new(MockProvider::new("openrouter"));
    mock.push_response(flavor_resp(
        &[
            "Neon smears as the first drops fall.",
            "The drizzle thickens.",
        ],
        &[
            "Rain ticks on the tin awnings.",
            "Puddles mirror the signs.",
        ],
    ));
    let llm = service(&mock, true);

    let out = generate(
        &mut tw,
        &llm,
        GOD,
        &format!("@weather/generate {CLIMATE} rain"),
    )
    .await;

    // The completion notified the requester with counts.
    assert!(
        tw.seen_by(&out, GOD).iter().any(|l| l.contains("PENDING")),
        "completion should report pending lines: {:?}",
        tw.seen_by(&out, GOD)
    );

    // The prompt carried the climate style AND the zone theme.
    let req = mock.last_request().expect("one call");
    let prompt = prompt_text(&req);
    assert!(
        prompt.contains("drifting sea fog"),
        "prompt missing climate style: {prompt}"
    );
    assert!(
        prompt.contains("rain-slick neon streets"),
        "prompt missing zone theme style: {prompt}"
    );
    assert!(
        prompt.contains("neon") && prompt.contains("sunshine"),
        "theme vocab/banned: {prompt}"
    );

    // The pending lines were cached via module ops under the documented keys.
    let ops = flavor_ops(&mut tw);
    let keys: Vec<&str> = ops.iter().map(|(k, _)| k.as_str()).collect();
    assert!(
        keys.contains(&"flavor:temperate-coastal:rain:transition")
            && keys.contains(&"flavor:temperate-coastal:rain:ambient"),
        "expected both bucket docs, got {keys:?}"
    );
    // And they are stored PENDING (approved=false).
    let st = tw.world().module_state::<WeatherState>().unwrap();
    let key = FlavorKey::new(CLIMATE, "rain", FlavorBucket::Ambient);
    assert_eq!(st.flavor.pool(&key).len(), 2);
    assert!(
        st.flavor.pool(&key).iter().all(|l| !l.approved),
        "generated lines must start pending"
    );
    // No approved line is live yet.
    assert!(
        st.flavor
            .approved_texts(CLIMATE, "rain", FlavorBucket::Ambient)
            .is_empty()
    );
}

#[tokio::test]
async fn duplicate_completion_reports_actual_additions_and_avoids_rewrite() {
    let mut tw = world();
    zone_with_climate(&mut tw);
    let mock = Arc::new(MockProvider::new("openrouter"));
    let response = flavor_resp(&["Same shift."], &["Same ambience."]);
    mock.push_response(response.clone());
    mock.push_response(response);
    let llm = service(&mock, true);

    generate(
        &mut tw,
        &llm,
        GOD,
        &format!("@weather/generate {CLIMATE} rain"),
    )
    .await;
    tw.take_module_ops();

    let out = generate(
        &mut tw,
        &llm,
        GOD,
        &format!("@weather/generate {CLIMATE} rain"),
    )
    .await;
    let seen = tw.seen_by(&out, GOD).join("\n");
    assert!(
        seen.contains("0 transition + 0 ambient"),
        "completion must report accepted rather than offered lines: {seen}"
    );
    assert!(
        flavor_ops(&mut tw).is_empty(),
        "an all-duplicate completion must not rewrite either pool"
    );
}

#[test]
fn generation_skips_targets_whose_two_pools_are_full() {
    let mut tw = world();
    zone_with_climate(&mut tw);
    let weather = tw.world_mut().module_state_mut::<WeatherState>().unwrap();
    for bucket in FlavorBucket::ALL {
        assert_eq!(
            weather.flavor.add_pending(
                CLIMATE,
                "rain",
                bucket,
                (0..MAX_FLAVOR_LINES_PER_POOL).map(|n| format!("{bucket:?} line {n}")),
            ),
            MAX_FLAVOR_LINES_PER_POOL
        );
    }

    let (out, effects) = step_cmd(&mut tw, GOD, &format!("@weather/generate {CLIMATE} rain"));
    assert!(effects.is_empty(), "a full target must spend no LLM work");
    assert!(
        tw.seen_by(&out, GOD)
            .iter()
            .any(|line| line.contains("1 full bucket(s) were skipped")),
        "staff should be told why no request fired: {:?}",
        tw.seen_by(&out, GOD)
    );
}

#[tokio::test]
async fn lines_lists_builtin_pending_and_approved_distinctly() {
    let mut tw = world();
    zone_with_climate(&mut tw);
    let mock = Arc::new(MockProvider::new("openrouter"));
    mock.push_response(flavor_resp(
        &["Generated shift line."],
        &["Generated ambient line."],
    ));
    let llm = service(&mock, true);
    generate(
        &mut tw,
        &llm,
        GOD,
        &format!("@weather/generate {CLIMATE} rain"),
    )
    .await;

    // Before approval: builtin + PENDING appear; nothing is approved.
    let out = tw.run(GOD, &format!("@weather/lines {CLIMATE} rain"));
    let seen = tw.seen_by(&out, GOD).join("\n");
    assert!(
        seen.contains("[builtin]"),
        "listing shows hand-written: {seen}"
    );
    assert!(seen.contains("PENDING"), "listing shows pending: {seen}");
    assert!(!seen.contains("[approved"), "nothing approved yet: {seen}");
    // A known hand-written ambient line is present (from the shipped data).
    assert!(
        seen.contains("Rain patters steadily against every surface."),
        "builtin ambient line missing: {seen}"
    );

    // Approve the pending ambient line by its id, then it lists as approved.
    let id = pending_id(&tw, CLIMATE, "rain", FlavorBucket::Ambient);
    tw.run(GOD, &format!("@weather/lines/approve {id}"));
    let out = tw.run(GOD, &format!("@weather/lines {CLIMATE} rain"));
    let seen = tw.seen_by(&out, GOD).join("\n");
    assert!(
        seen.contains(&format!("[approved #{id}")) || seen.contains("[approved"),
        "listing shows approved after approve: {seen}"
    );
}

#[tokio::test]
async fn approve_moves_a_line_into_the_live_pool() {
    let mut tw = world();
    zone_with_climate(&mut tw);
    let mock = Arc::new(MockProvider::new("openrouter"));
    mock.push_response(flavor_resp(&[], &["Approved-only ambient line."]));
    let llm = service(&mock, true);
    generate(
        &mut tw,
        &llm,
        GOD,
        &format!("@weather/generate {CLIMATE} rain"),
    )
    .await;

    // Pending: not live.
    assert!(
        tw.world()
            .module_state::<WeatherState>()
            .unwrap()
            .flavor
            .approved_texts(CLIMATE, "rain", FlavorBucket::Ambient)
            .is_empty()
    );
    let id = pending_id(&tw, CLIMATE, "rain", FlavorBucket::Ambient);
    let out = tw.run(GOD, &format!("@weather/lines/approve {id}"));
    assert!(
        tw.seen_by(&out, GOD)
            .iter()
            .any(|l| l.contains("now in the live pool")),
        "approve confirms: {:?}",
        tw.seen_by(&out, GOD)
    );
    // Now live.
    assert_eq!(
        tw.world()
            .module_state::<WeatherState>()
            .unwrap()
            .flavor
            .approved_texts(CLIMATE, "rain", FlavorBucket::Ambient),
        vec!["Approved-only ambient line.".to_string()]
    );
    // The approval persisted (the pool doc was rewritten with approved=true).
    let ops = flavor_ops(&mut tw);
    assert!(
        ops.iter()
            .any(|(k, _)| k == "flavor:temperate-coastal:rain:ambient"),
        "approve should persist the pool"
    );
}

/// Tick-time selection draws ONLY hand-written + approved lines: a pending
/// line is never shown; an approved one is. Uses a barren custom climate whose
/// hand-written pools are empty, so the approved overlay is the sole candidate.
#[tokio::test]
async fn tick_selection_shows_approved_never_pending() {
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
    let zone = tw
        .world_mut()
        .create(ObjKind::Thing, "Downtown", GOD, Some(LIMBO))
        .unwrap();
    let room = tw
        .world_mut()
        .create(ObjKind::Room, "Plaza", GOD, None)
        .unwrap();
    tw.world_mut().set_zone(room, Some(zone)).unwrap();
    tw.world_mut()
        .set_flag(room, Flags::OUTDOORS, true)
        .unwrap();
    tw.run(GOD, &format!("@weather/climate #{}=barren", zone.0));
    tw.run(GOD, &format!("@weather #{}=rain", zone.0)); // pin: no transition steals the tick

    // Inject one PENDING ambient line directly (as a generate would).
    tw.world_mut()
        .module_state_mut::<WeatherState>()
        .unwrap()
        .flavor
        .add_pending(
            "barren",
            "rain",
            FlavorBucket::Ambient,
            ["PENDING RAIN LINE".to_string()],
        );

    let player = tw.spawn_player("Rae");
    tw.world_mut().move_to(player, room).unwrap();

    // Pending: the tick shows NOTHING (barren pools + not-approved overlay).
    seed(&mut tw, 7);
    let out = tw.fire_due(60_000);
    assert!(
        weather_to_room(&out, room).is_empty(),
        "a pending line must never be shown: {:?}",
        weather_to_room(&out, room)
    );

    // Approve it, then the tick can only draw that one approved line.
    let id = pending_id(&tw, "barren", "rain", FlavorBucket::Ambient);
    tw.run(GOD, &format!("@weather/lines/approve {id}"));
    seed(&mut tw, 7);
    let out = tw.fire_due(120_000);
    let lines = weather_to_room(&out, room);
    assert_eq!(
        lines,
        vec!["PENDING RAIN LINE".to_string()],
        "after approval the tick shows the approved line"
    );
}

#[tokio::test]
async fn disabled_llm_path_stores_nothing_and_notifies() {
    let mut tw = world();
    zone_with_climate(&mut tw);
    tw.take_module_ops();
    let mock = Arc::new(MockProvider::new("openrouter"));
    // Kill switch engaged: llm.call short-circuits to Disabled before the mock.
    let llm = service(&mock, false);

    let out = generate(
        &mut tw,
        &llm,
        GOD,
        &format!("@weather/generate {CLIMATE} rain"),
    )
    .await;
    assert!(
        tw.seen_by(&out, GOD)
            .iter()
            .any(|l| l.contains("LLM features are disabled")),
        "disabled path notifies: {:?}",
        tw.seen_by(&out, GOD)
    );
    assert_eq!(
        mock.call_count(),
        0,
        "disabled: the provider is never called"
    );
    // Nothing cached, nothing live — everything unchanged.
    assert!(
        flavor_ops(&mut tw).is_empty(),
        "disabled: no flavor persisted"
    );
    assert!(
        tw.world()
            .module_state::<WeatherState>()
            .unwrap()
            .flavor
            .is_empty(),
        "disabled: flavor store stays empty"
    );
}

#[tokio::test]
async fn flavor_pool_survives_reboot() {
    let mut tw = world();
    zone_with_climate(&mut tw);
    tw.take_module_ops();
    let mock = Arc::new(MockProvider::new("openrouter"));
    mock.push_response(flavor_resp(&["Shift A.", "Shift B."], &["Ambient A."]));
    let llm = service(&mock, true);
    generate(
        &mut tw,
        &llm,
        GOD,
        &format!("@weather/generate {CLIMATE} rain"),
    )
    .await;
    // Approve one transition line; leave the rest pending.
    let approve_id = pending_id(&tw, CLIMATE, "rain", FlavorBucket::Transition);
    tw.run(GOD, &format!("@weather/lines/approve {approve_id}"));

    // Collect every weather op and write it through a real store.
    let ops = tw.take_module_ops();
    assert!(!ops.is_empty());
    let store = InMemoryStore::new();
    futures::executor::block_on(async {
        store
            .write(WriteBatch {
                serial: 1,
                module_ops: ops,
                ..WriteBatch::default()
            })
            .await
            .unwrap();
    });
    let kv =
        futures::executor::block_on(async { store.module_json_list("weather").await.unwrap() });

    // Fresh boot from the store.
    let mut tw2 = world();
    let boot: HashMap<SmolStr, mush_core::ModuleBootData> = HashMap::from([(
        SmolStr::new("weather"),
        mush_core::ModuleBootData {
            kv: kv.into_iter().map(|(k, v)| (SmolStr::new(k), v)).collect(),
            streams: HashMap::new(),
            ..Default::default()
        },
    )]);
    tw2.modules_loaded(boot);

    let st = tw2.world().module_state::<WeatherState>().unwrap();
    // The one approved transition line ("Shift A.", the lowest-id pending one)
    // is live again after reboot.
    let live = st
        .flavor
        .approved_texts(CLIMATE, "rain", FlavorBucket::Transition);
    assert_eq!(
        live,
        vec!["Shift A.".to_string()],
        "approved line survives reboot"
    );
    // The ambient pool (still pending) survived too, and is still pending.
    let amb = FlavorKey::new(CLIMATE, "rain", FlavorBucket::Ambient);
    assert_eq!(st.flavor.pool(&amb).len(), 1);
    assert!(st.flavor.pool(&amb).iter().all(|l| !l.approved));
}

#[test]
fn non_staff_generate_is_refused() {
    let mut tw = world();
    zone_with_climate(&mut tw);
    let player = tw.spawn_player("Mook"); // a plain Player
    let out = tw.run(player, &format!("@weather/generate {CLIMATE} rain"));
    assert!(
        tw.seen_by(&out, player)
            .iter()
            .any(|l| l.contains("Permission denied")),
        "a non-staff caller is refused: {:?}",
        tw.seen_by(&out, player)
    );
}

#[test]
fn royalty_without_builder_llm_is_refused() {
    let mut tw = world();
    zone_with_climate(&mut tw);
    let staff = tw.spawn_player("Reeve");
    tw.world_mut()
        .set_flag(staff, Flags::ROYALTY, true)
        .unwrap();
    let out = tw.run(staff, &format!("@weather/generate {CLIMATE} rain"));
    assert!(
        tw.seen_by(&out, staff)
            .iter()
            .any(|l| l.contains("BUILDER_LLM")),
        "Royalty without BUILDER_LLM is told why: {:?}",
        tw.seen_by(&out, staff)
    );
}

// ----------------------------------------------------------------------
// small helpers
// ----------------------------------------------------------------------

/// The id of the (first) pending line in a bucket.
fn pending_id(tw: &TestWorld, climate: &str, state: &str, bucket: FlavorBucket) -> u64 {
    let key = FlavorKey::new(climate, state, bucket);
    tw.world()
        .module_state::<WeatherState>()
        .unwrap()
        .flavor
        .pool(&key)
        .iter()
        .find(|l| !l.approved)
        .map(|l| l.id)
        .expect("a pending line")
}

fn seed(tw: &mut TestWorld, s: u64) {
    tw.world_mut()
        .module_state_mut::<WeatherState>()
        .unwrap()
        .rng = WxRng::seeded(s);
}

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

/// A barren climate: clear/rain, empty line pools — the approved overlay is the
/// only ambient/transition candidate.
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

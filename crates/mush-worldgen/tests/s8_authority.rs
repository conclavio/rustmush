//! S8.4 (14a #4): staff authority is checked when a proposal is applied, not
//! when it was prepared. The review's reproduction is in `review_plan.rs` and
//! passes unchanged; these are the cases around it.

use std::sync::Arc;

use mush_atlas::{AtlasConfig, AtlasModule};
use mush_content::{Appliers, ContentModule, Pack, parse_pack};
use mush_core::{
    Audience, Effect, Effects, GameModule, Inbound, ObjId, ObjKind, Outbox, WorldCommand,
};
use mush_llm::{
    CompletionResponse, ContentBlock, LlmProvider, LlmService, MockProvider, RouteConfig,
    StopReason, Usage,
};
use mush_proto::OutputEvent;
use mush_testkit::TestWorld;
use mush_worldgen::{WorldgenConfig, WorldgenModule, run_worldgen_async};
use serde_json::{Value, json};

// ----------------------------------------------------------------------
// Fixtures
// ----------------------------------------------------------------------

fn signed(mut doc: Value) -> Pack {
    doc["manifest"]["digest"] = Value::String("0".repeat(64));
    let unsigned = serde_json::to_string(&doc).expect("json");
    let digest = parse_pack(&unsigned)
        .expect("parses")
        .computed_digest()
        .expect("digests")
        .to_hex();
    doc["manifest"]["digest"] = Value::String(digest);
    parse_pack(&serde_json::to_string(&doc).expect("json")).expect("parses")
}

fn region() -> Pack {
    signed(json!({
        "manifest": { "id": "region.town", "version": 1, "kind": "region", "digest": "", "requires": [] },
        "entries": {
            "region.town": { "kind": "region", "def": {
                "name": "The Town", "kind": "surface", "body": "earth",
                "extent": [[0, 0], [400, 400]], "datum_m": 0.0,
                "background": { "surface": "paved", "mix": [["grass", 8]], "relief_cm": 0,
                                "relief_period_cm": 800, "seed": 1 },
                "ground_resolution_cm": 1000,
                "theme": { "name": "Plain", "style_guide": "plain and warm" } } },
            "locality.town": { "kind": "locality", "def": {
                "region": "region.town", "name": "Town", "kind": "district",
                "footprint": [[0, 0], [400, 0], [400, 400], [0, 400]] } },
            "connection.road": { "kind": "connection", "def": {
                "region": "region.town", "name": "The Road", "kind": "journey",
                "from": { "neighborhood": "neighborhood.block", "point": [30, 30, 0] },
                "to": { "neighborhood": "neighborhood.far", "point": [300, 300, 0] },
                "direction": "both", "modes": ["foot"], "corridor": [[30, 30, 0], [300, 300, 0]] } }
        }
    }))
}

fn block() -> Pack {
    signed(json!({
        "manifest": { "id": "neighborhood.block", "version": 1, "kind": "neighborhood", "digest": "",
                      "requires": [{ "id": "region.town", "version": 1 }] },
        "entries": {
            "neighborhood": { "kind": "neighborhood", "def": {
                "region": "region.town", "locality": "locality.town", "name": "The Block",
                "setting": "suburban", "footprint": [[0, 0], [200, 0], [200, 200], [0, 200]],
                "room_budget": { "target": 3, "maximum": 12 },
                "hydration": { "enabled": true, "max_player_sites": 4 } } },
            "room.green": { "kind": "room", "def": { "name": "The Green", "surface": "ground",
                "footprint": { "rect": [[10, 10], [60, 60]] }, "desc": "A wide green." } },
            "exit.limbo_green": { "kind": "exit", "def": { "from": "limbo", "to": "room.green",
                "names": ["Out <O>;o;out", "Limbo <L>;limbo"] } }
        }
    }))
}

fn route() -> RouteConfig {
    RouteConfig {
        model: "test/mock".into(),
        max_tokens: 4096,
        timeout_s: 60,
        reasoning: None,
        reasoning_max_tokens: None,
    }
}

fn service(mock: &Arc<MockProvider>) -> Arc<LlmService> {
    Arc::new(
        LlmService::builder()
            .provider("openrouter", Arc::clone(mock) as Arc<dyn LlmProvider>)
            .route("worldgen_plan", route())
            .global_enabled(true)
            .build(),
    )
}

fn config() -> WorldgenConfig {
    WorldgenConfig {
        plan_cost_cap_usd: 5.0,
        plan_cost_target_usd: 2.0,
        plan_max_rounds: 6,
        ..WorldgenConfig::default()
    }
}

/// The town with the atlas, the content module, jobs, canon, the NPC host and the
/// worldgen module on `llm`; the block is on the shelf too.
fn town(llm: &Arc<LlmService>) -> TestWorld {
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
    let modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(mush_npc::NpcModule::new()),
        Box::new(mush_canon::CanonModule::new()),
        Box::new(mush_jobs::JobsModule::new()),
        Box::new(
            AtlasModule::with_config(AtlasConfig {
                hydration_cap: 2,
                hydration_strikes_to_zero: 3,
                ..AtlasConfig::default()
            })
            .with_shelf(vec![block()]),
        ),
        Box::new(
            ContentModule::new()
                .with_appliers(appliers)
                .with_boot_packs(vec![region(), block()]),
        ),
        Box::new(WorldgenModule::with_config(Arc::clone(llm), config())),
    ];
    TestWorld::with_modules(modules)
}

fn rig() -> (TestWorld, Arc<MockProvider>, Arc<LlmService>) {
    let mock = Arc::new(MockProvider::new("openrouter"));
    let llm = service(&mock);
    let tw = town(&llm);
    (tw, mock, llm)
}

fn room(tw: &TestWorld, name: &str) -> Option<ObjId> {
    tw.world()
        .iter()
        .find(|o| {
            o.kind == ObjKind::Room && o.name == name && !o.flags.contains(mush_core::Flags::GOING)
        })
        .map(|o| o.id)
}

fn step(
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

fn sys_to(events: &[(Audience, OutputEvent)], actor: ObjId) -> String {
    events
        .iter()
        .filter_map(|(aud, e)| match (aud, e) {
            (Audience::Actor(a), OutputEvent::Sys(t)) if *a == actor => Some(t.to_plain_text()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The one `Effect::ModuleAsync` a command deferred.
fn module_async(effects: &[Effect]) -> (u64, String) {
    let mut found = effects.iter().filter_map(|e| match e {
        Effect::ModuleAsync {
            correlation,
            payload,
            ..
        } => Some((*correlation, payload.clone())),
        _ => None,
    });
    let one = found.next().expect("one deferred planning run");
    assert!(found.next().is_none(), "exactly one run was deferred");
    one
}

fn apply_done(
    tw: &mut TestWorld,
    actor: ObjId,
    correlation: u64,
    payload: String,
) -> Vec<(Audience, OutputEvent)> {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut().step(
        Inbound::Apply(WorldCommand::ModuleAsyncDone {
            module: "worldgen".into(),
            actor,
            correlation,
            payload,
        }),
        &mut out,
        &mut fx,
    );
    out.take()
}

/// Run the deferred planning run off-loop and re-enter it.
async fn finish(
    tw: &mut TestWorld,
    llm: &LlmService,
    actor: ObjId,
    effects: &[Effect],
) -> Vec<(Audience, OutputEvent)> {
    let (correlation, payload) = module_async(effects);
    let outcome = run_worldgen_async(llm, &payload).await;
    apply_done(tw, actor, correlation, outcome)
}

// ----------------------------------------------------------------------
// Scripted replies
// ----------------------------------------------------------------------

fn tool_use(id: &str, name: &str, input: Value) -> ContentBlock {
    ContentBlock::ToolUse {
        id: id.into(),
        name: name.into(),
        input,
    }
}

fn reply(blocks: Vec<ContentBlock>, cost: f64) -> CompletionResponse {
    CompletionResponse {
        content: blocks,
        stop_reason: StopReason::ToolUse,
        usage: Usage {
            input_tokens: 500,
            output_tokens: 200,
            cost_usd: Some(cost),
            ..Usage::default()
        },
        model: "test/mock".into(),
    }
}

/// Propose these entries and validate, in one turn.
fn propose_and_validate(entries: Value, cost: f64) -> CompletionResponse {
    reply(
        vec![
            tool_use(
                "p1",
                "pack.propose",
                json!({ "pack": { "entries": entries } }),
            ),
            tool_use("v1", "pack.validate", json!({})),
        ],
        cost,
    )
}

/// A site of `kind` over `rect`, one room on it, a walk from the Green.
fn site_entries(name: &str, kind: &str, rect: [[f64; 2]; 2]) -> Value {
    let [[x0, y0], [x1, y1]] = rect;
    json!({
        "site.plot": { "kind": "site", "def": { "name": name, "site_kind": kind,
            "footprint": [[x0, y0], [x1, y0], [x1, y1], [x0, y1]] } },
        "room.plot": { "kind": "room", "def": { "name": name, "surface": "ground",
            "site": "site.plot", "footprint": { "rect": [[x0, y0], [x1, y1]] },
            "desc": "A quiet place with a bench under a tree." } },
        "exit.green_plot": { "kind": "exit", "def": { "from": "neighborhood.block.room.green",
            "to": "room.plot", "names": [format!("{name} <P>;plot"), "Green <G>;g"],
            "portal": { "distance_m": 40 } } }
    })
}

fn proposal(tw: &TestWorld, id: u64) -> mush_worldgen::ProposalRec {
    tw.world()
        .module_state::<mush_worldgen::WorldgenState>()
        .expect("worldgen")
        .proposals
        .get(&id)
        .cloned()
        .expect("the proposal")
}

/// A wizard proposes the bakery and the run leaves it ready.
async fn ready_bakery(tw: &mut TestWorld, mock: &MockProvider, llm: &LlmService) -> ObjId {
    let author = tw.spawn_player("Builder");
    tw.world_mut()
        .get_mut(author)
        .unwrap()
        .flags
        .insert(mush_core::Flags::WIZARD);
    let (open, effects) = step(
        tw,
        author,
        "@atlas/propose The Block=A bakery on the corner.",
    );
    assert!(
        sys_to(&open, author).contains("Proposal 1 is being planned"),
        "{}",
        sys_to(&open, author)
    );
    mock.push_response(propose_and_validate(
        site_entries("The Bakery", "site.shop", [[100.0, 10.0], [140.0, 50.0]]),
        0.50,
    ));
    let done = finish(tw, llm, author, &effects).await;
    assert!(
        sys_to(&done, author).contains("Proposal 1 is ready"),
        "{}",
        sys_to(&done, author)
    );
    assert!(proposal(tw, 1).staff);
    author
}

#[tokio::test]
async fn an_author_who_keeps_staff_applies() {
    let (mut tw, mock, llm) = rig();
    let author = ready_bakery(&mut tw, &mock, &llm).await;
    let (went, _) = step(&mut tw, author, "+build/go 1");
    assert!(
        sys_to(&went, author).contains("Applied"),
        "{}",
        sys_to(&went, author)
    );
    assert!(room(&tw, "The Bakery").is_some());
}

#[tokio::test]
async fn a_repromoted_author_goes_again() {
    let (mut tw, mock, llm) = rig();
    let author = ready_bakery(&mut tw, &mock, &llm).await;
    tw.world_mut()
        .get_mut(author)
        .unwrap()
        .flags
        .remove(mush_core::Flags::WIZARD);
    let (went, _) = step(&mut tw, author, "+build/go 1");
    let out = sys_to(&went, author);
    assert!(out.contains("staff authority you no longer hold"), "{out}");
    assert!(room(&tw, "The Bakery").is_none());
    // The record is provenance, not a grant — and it is still ready.
    let rec = proposal(&tw, 1);
    assert!(rec.staff);
    assert_eq!(rec.status, mush_worldgen::ProposalStatus::Ready);
    tw.world_mut()
        .get_mut(author)
        .unwrap()
        .flags
        .insert(mush_core::Flags::WIZARD);
    let (went, _) = step(&mut tw, author, "+build/go 1");
    assert!(
        sys_to(&went, author).contains("Applied"),
        "{}",
        sys_to(&went, author)
    );
    assert!(room(&tw, "The Bakery").is_some());
}

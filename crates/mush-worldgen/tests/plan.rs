//! The building agent (doc 14 §4; S3, 14-spatial-implementation-plan §3.4) against a
//! scripted `MockProvider`: a brief becomes a validating pack and `+build/go` builds
//! it; a pack that never validates leaves a resumable proposal; the cost cap stops a
//! run and nothing is applied; the target is told and never enforced; no tool
//! mutates the world; a stranger cannot go; staff apply live with a record ticket;
//! questions pause the run; reconceptualizing keeps the access point; a secret in a
//! persona is refused; boot makes no provider call; a reboot keeps the book.

use std::collections::HashMap;
use std::sync::Arc;

use mush_atlas::{AtlasConfig, AtlasModule, AtlasState};
use mush_content::{Appliers, ContentModule, Pack, parse_pack};
use mush_core::{
    Audience, Effect, Effects, GOD, GameModule, Inbound, ModuleBootData, ModuleWrite, ObjId,
    ObjKind, Outbox, WorldCommand,
};
use mush_llm::{
    CompletionResponse, ContentBlock, LlmProvider, LlmService, MockProvider, RouteConfig,
    StopReason, Usage,
};
use mush_proto::OutputEvent;
use mush_testkit::TestWorld;
use mush_worldgen::{ProposalStatus, WorldgenConfig, WorldgenModule, run_worldgen_async};
use serde_json::{Value, json};
use smol_str::SmolStr;

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
                "region": "region.town", "name": "The Road", "kind": "road",
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
                "names": ["Out <O>;o;out", "Limbo <L>;l;limbo"] } }
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

fn live_objects(tw: &TestWorld) -> usize {
    tw.world()
        .iter()
        .filter(|o| !o.flags.contains(mush_core::Flags::GOING))
        .count()
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

fn read(what: &str) -> CompletionResponse {
    reply(
        vec![tool_use("r1", "atlas.read", json!({ "what": what }))],
        0.05,
    )
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

fn ask(questions: &[&str]) -> CompletionResponse {
    reply(
        vec![tool_use("a1", "ask", json!({ "questions": questions }))],
        0.05,
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
            "to": "room.plot", "names": [format!("{name} <P>;p"), "Green <G>;g"],
            "portal": { "distance_m": 40 } } }
    })
}

/// The footprint the atlas reserved for the player's request, as `[[x0,y0],[x1,y1]]`.
fn reserved_rect(tw: &TestWorld) -> [[f64; 2]; 2] {
    let atlas = tw.world().module_state::<AtlasState>().expect("atlas");
    let rec = atlas
        .hydration
        .requests
        .values()
        .next()
        .expect("a request was reserved");
    let bbox = rec.footprint.to_cm().expect("cm").bbox();
    let m = mush_atlas::geom::metres;
    [
        [m(bbox.min.x), m(bbox.min.y)],
        [m(bbox.max.x), m(bbox.max.y)],
    ]
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

fn green_player(tw: &mut TestWorld, name: &str) -> ObjId {
    let who = tw.spawn_player(name);
    let green = room(tw, "The Green").expect("the green");
    tw.world_mut().move_to(who, green).expect("moves");
    who
}

// ----------------------------------------------------------------------
// The loop and the player path
// ----------------------------------------------------------------------

#[tokio::test]
async fn a_players_brief_becomes_a_validating_pack_and_go_activates_it_pending_review() {
    let (mut tw, mock, llm) = rig();
    // P9: boot made no provider call.
    assert_eq!(mock.call_count(), 0);
    let rae = green_player(&mut tw, "Rae");
    let before = live_objects(&tw);

    let (open, effects) = step(&mut tw, rae, "+build/place park=A quiet park with a bench.");
    let said = sys_to(&open, rae);
    assert!(
        said.contains("reserved") && said.contains("proposal 1 is being planned"),
        "{said}"
    );
    assert_eq!(proposal(&tw, 1).status, ProposalStatus::Planning);
    let rect = reserved_rect(&tw);

    // Round 1 reads the neighborhood; round 2 proposes and validates clean.
    mock.push_response(read("neighborhood"));
    mock.push_response(propose_and_validate(
        site_entries("Rae's Park", "site.park", rect),
        0.40,
    ));
    let done = finish(&mut tw, &llm, rae, &effects).await;
    let told = sys_to(&done, rae);
    assert!(told.contains("Proposal 1 is ready"), "{told}");
    assert!(told.contains("+build/go 1"), "{told}");
    assert_eq!(mock.call_count(), 2);
    let rec = proposal(&tw, 1);
    assert_eq!(rec.status, ProposalStatus::Ready);
    assert_eq!(rec.rounds, 2);
    assert!((rec.cost_usd - 0.45).abs() < 1e-9, "{}", rec.cost_usd);
    assert!(rec.pack.is_some());
    // A: nothing touched the world during the run.
    assert_eq!(live_objects(&tw), before, "no tool mutates the world");
    // The first request learned a proposal stands.
    {
        let atlas = tw.world().module_state::<AtlasState>().expect("atlas");
        let req = atlas.hydration.requests.values().next().expect("request");
        assert!(
            matches!(
                req.status,
                mush_atlas::RequestStatus::Proposed { proposal: 1 }
            ),
            "{:?}",
            req.status
        );
    }

    // The proposal reads: the pack, the entries, the next step.
    let (shown, _) = step(&mut tw, rae, "+build/show 1");
    let shown = sys_to(&shown, rae);
    assert!(shown.contains("ready"), "{shown}");
    assert!(
        shown.contains("site.hyd_") && shown.contains("1 exit, 1 room, 1 site"),
        "{shown}"
    );
    assert!(shown.contains("next: +build/go 1"), "{shown}");

    // Go: the site stands pending review, with a ticket and a canon event.
    let (went, _) = step(&mut tw, rae, "+build/go 1");
    let went = sys_to(&went, rae);
    assert!(
        went.contains("stands pending review") && went.contains("job"),
        "{went}"
    );
    assert!(room(&tw, "Rae's Park").is_some());
    assert!(matches!(
        proposal(&tw, 1).status,
        ProposalStatus::Applied { .. }
    ));
    let atlas = tw.world().module_state::<AtlasState>().expect("atlas");
    let site = atlas
        .records
        .sites
        .values()
        .find(|s| s.name == "Rae's Park")
        .expect("the site");
    assert!(matches!(
        site.origin,
        mush_atlas::record::SiteOrigin::Hydrated { .. }
    ));
    assert_eq!(site.review, mush_atlas::record::ReviewState::PendingReview);
    let job = mush_jobs::get_job(tw.world(), 1).expect("the BUILD ticket");
    assert_eq!(job.queue.as_str(), "BUILD");
    assert!(
        job.payload
            .as_ref()
            .is_some_and(|p| p.handler.as_str() == "build.review")
    );
    // Status shows both.
    let (status, _) = step(&mut tw, rae, "+build/status");
    let status = sys_to(&status, rae);
    assert!(status.contains("1 — applied — a site.park"), "{status}");
    assert!(status.contains("1 of 2 request slots in use"), "{status}");
    // Asking again for the same park converges on the request; the proposal is
    // applied, so a new one starts for the same reserved ground? No: the request
    // is not outstanding once it stands; the second ask is a new site elsewhere.
    let (again, effects) = step(&mut tw, rae, "+build/place park=Another park.");
    let again = sys_to(&again, rae);
    assert!(again.contains("proposal 2 is being planned"), "{again}");
    assert!(module_async(&effects).0 > 0);
}

#[tokio::test]
async fn a_pack_that_never_validates_leaves_a_resumable_proposal_with_findings() {
    let (mut tw, mock, llm) = rig();
    let rae = green_player(&mut tw, "Rae");
    let (_, effects) = step(&mut tw, rae, "+build/place park=A park.");
    // Every reply proposes a park past the neighborhood's edge, then validates.
    mock.set_default(Ok(propose_and_validate(
        site_entries("Far Park", "site.park", [[190.0, 190.0], [230.0, 230.0]]),
        0.10,
    )));
    let done = finish(&mut tw, &llm, rae, &effects).await;
    let told = sys_to(&done, rae);
    assert!(told.contains("Proposal 1 needs work"), "{told}");
    assert_eq!(mock.call_count(), 6, "the round bound");
    let rec = proposal(&tw, 1);
    assert!(
        matches!(rec.status, ProposalStatus::NeedsWork { .. }),
        "{:?}",
        rec.status
    );
    assert!(rec.pack.is_some(), "the last pack is kept");
    assert!(
        rec.findings.iter().any(|f| f.code == "geometry.outside"),
        "{:?}",
        rec.findings
    );
    let (shown, _) = step(&mut tw, rae, "+build/show 1");
    let shown = sys_to(&shown, rae);
    assert!(shown.contains("geometry.outside"), "{shown}");
    assert!(shown.contains("next: +build/feedback 1=<text>"), "{shown}");
    // Go is refused; feedback runs it again with the prior pack in the prompt.
    let (went, _) = step(&mut tw, rae, "+build/go 1");
    assert!(
        sys_to(&went, rae).contains("needs work"),
        "{}",
        sys_to(&went, rae)
    );
    let rect = reserved_rect(&tw);
    let (fed, effects) = step(
        &mut tw,
        rae,
        "+build/feedback 1=Keep it inside the block, on the reserved ground.",
    );
    assert!(
        sys_to(&fed, rae).contains("planned again"),
        "{}",
        sys_to(&fed, rae)
    );
    let stale_payload = {
        // An outcome computed for the revision before the feedback is dropped.
        let (correlation, payload) = module_async(&effects);
        let mut old: Value = serde_json::from_str(&payload).expect("json");
        old["revision"] = json!(1);
        (correlation, serde_json::to_string(&old).expect("json"))
    };
    mock.push_response(propose_and_validate(
        site_entries("Rae's Park", "site.park", rect),
        0.30,
    ));
    let done = finish(&mut tw, &llm, rae, &effects).await;
    assert!(
        sys_to(&done, rae).contains("Proposal 1 is ready"),
        "{}",
        sys_to(&done, rae)
    );
    let rec = proposal(&tw, 1);
    assert_eq!(rec.status, ProposalStatus::Ready);
    assert_eq!(rec.feedback.len(), 1);
    let last = mock.last_request().expect("a request");
    let opening = last
        .messages
        .first()
        .map(|m| format!("{:?}", m.content))
        .unwrap_or_default();
    assert!(
        opening.contains("Keep it inside") && opening.contains("previous proposal"),
        "{opening}"
    );
    // The stale outcome: run it and re-enter; the proposal stays ready.
    let outcome = run_worldgen_async(&llm, &stale_payload.1).await;
    apply_done(&mut tw, rae, stale_payload.0, outcome);
    assert_eq!(
        proposal(&tw, 1).status,
        ProposalStatus::Ready,
        "a stale outcome is dropped"
    );
}

#[tokio::test]
async fn the_cost_cap_stops_a_run_and_nothing_is_applied() {
    let (mut tw, mock, llm) = rig();
    let rae = green_player(&mut tw, "Rae");
    let before = live_objects(&tw);
    let (_, effects) = step(&mut tw, rae, "+build/place park=A park.");
    // Every call costs two dollars and only reads: 2, 4, then the cap of 5 refuses the fourth.
    mock.set_default(Ok(reply(
        vec![tool_use("r", "atlas.read", json!({ "what": "rooms" }))],
        2.0,
    )));
    let done = finish(&mut tw, &llm, rae, &effects).await;
    let told = sys_to(&done, rae);
    assert!(told.contains("stopped at its cost cap"), "{told}");
    assert_eq!(mock.call_count(), 3);
    let rec = proposal(&tw, 1);
    assert_eq!(rec.status, ProposalStatus::Capped);
    assert!((rec.cost_usd - 6.0).abs() < 1e-9);
    assert_eq!(live_objects(&tw), before);
    let (went, _) = step(&mut tw, rae, "+build/go 1");
    assert!(
        sys_to(&went, rae).contains("at its cost cap"),
        "{}",
        sys_to(&went, rae)
    );
    let (fed, effects) = step(&mut tw, rae, "+build/feedback 1=try again");
    assert!(
        sys_to(&fed, rae).contains("spent its cap"),
        "{}",
        sys_to(&fed, rae)
    );
    assert!(
        effects
            .iter()
            .all(|e| !matches!(e, Effect::ModuleAsync { .. })),
        "no run was deferred"
    );
}

#[tokio::test]
async fn the_target_is_told_to_the_model_and_never_enforced() {
    let (mut tw, mock, llm) = rig();
    let rae = green_player(&mut tw, "Rae");
    let (_, effects) = step(&mut tw, rae, "+build/place park=A park.");
    let rect = reserved_rect(&tw);
    // Over the target of $2 in one call, under the cap of $5: it completes.
    mock.push_response(propose_and_validate(
        site_entries("Rae's Park", "site.park", rect),
        3.0,
    ));
    finish(&mut tw, &llm, rae, &effects).await;
    assert_eq!(proposal(&tw, 1).status, ProposalStatus::Ready);
    let system = mock
        .last_request()
        .and_then(|r| r.system)
        .expect("a system prompt");
    assert!(
        system.contains("under $2.00") && system.contains("stops at $5.00"),
        "{system}"
    );
    assert!(system.contains("after 6 calls"), "{system}");
    assert!(
        system.contains("plain and warm"),
        "the theme is in the prompt: {system}"
    );
}

// ----------------------------------------------------------------------
// Authority
// ----------------------------------------------------------------------

#[tokio::test]
async fn a_staff_proposal_applies_live_with_a_record_ticket_and_a_stranger_cannot_go() {
    let (mut tw, mock, llm) = rig();
    let rae = green_player(&mut tw, "Rae");
    let (open, effects) = step(
        &mut tw,
        GOD,
        "@atlas/propose The Block=A bakery on the corner.",
    );
    assert!(
        sys_to(&open, GOD).contains("Proposal 1 is being planned"),
        "{}",
        sys_to(&open, GOD)
    );
    mock.push_response(propose_and_validate(
        site_entries("The Bakery", "site.shop", [[100.0, 10.0], [140.0, 50.0]]),
        0.50,
    ));
    let done = finish(&mut tw, &llm, GOD, &effects).await;
    assert!(
        sys_to(&done, GOD).contains("Proposal 1 is ready"),
        "{}",
        sys_to(&done, GOD)
    );
    let rec = proposal(&tw, 1);
    assert!(rec.staff);
    assert!(
        rec.pack.as_ref().is_some_and(|p| p
            .manifest()
            .id()
            .as_str()
            .starts_with("site.p1_a_bakery"))
    );
    // Rae is not the author and not staff.
    let (went, _) = step(&mut tw, rae, "+build/go 1");
    assert!(
        sys_to(&went, rae).contains("not yours"),
        "{}",
        sys_to(&went, rae)
    );
    let (shown, _) = step(&mut tw, rae, "+build/show 1");
    assert!(
        sys_to(&shown, rae).contains("not yours"),
        "{}",
        sys_to(&shown, rae)
    );
    assert!(room(&tw, "The Bakery").is_none());
    // Staff go: live, with a record ticket that carries no payload.
    let (went, _) = step(&mut tw, GOD, "+build/go 1");
    let went = sys_to(&went, GOD);
    assert!(
        went.contains("Applied site.p1_a_bakery") && went.contains("3 created"),
        "{went}"
    );
    assert!(went.contains("ticket 1 filed"), "{went}");
    assert!(room(&tw, "The Bakery").is_some());
    let job = mush_jobs::get_job(tw.world(), 1).expect("the record ticket");
    assert!(job.payload.is_none());
    assert!(
        job.title.starts_with("Applied site.p1_a_bakery"),
        "{}",
        job.title
    );
    let atlas = tw.world().module_state::<AtlasState>().expect("atlas");
    let site = atlas
        .records
        .sites
        .values()
        .find(|s| s.name == "The Bakery")
        .expect("site");
    assert_eq!(site.origin, mush_atlas::record::SiteOrigin::Authored);
    // Going twice is refused.
    let (again, _) = step(&mut tw, GOD, "+build/go 1");
    assert!(
        sys_to(&again, GOD).contains("applied"),
        "{}",
        sys_to(&again, GOD)
    );
}

#[tokio::test]
async fn questions_pause_the_run_and_answers_resume_it() {
    let (mut tw, mock, llm) = rig();
    let rae = green_player(&mut tw, "Rae");
    let (_, effects) = step(&mut tw, rae, "+build/place park=A park.");
    let rect = reserved_rect(&tw);
    mock.push_response(ask(&["A pond or a fountain?", "Benches, how many?"]));
    let done = finish(&mut tw, &llm, rae, &effects).await;
    let told = sys_to(&done, rae);
    assert!(
        told.contains("Proposal 1 asks") && told.contains("1. A pond or a fountain?"),
        "{told}"
    );
    assert_eq!(mock.call_count(), 1, "the run paused at the question");
    assert_eq!(proposal(&tw, 1).status, ProposalStatus::NeedsAnswers);
    // One answer keeps it waiting; the second runs it again with both answers.
    let (one, effects) = step(&mut tw, rae, "+build/answer 1/1=A pond.");
    assert!(
        sys_to(&one, rae).contains("1 question(s) still open"),
        "{}",
        sys_to(&one, rae)
    );
    assert!(
        effects
            .iter()
            .all(|e| !matches!(e, Effect::ModuleAsync { .. }))
    );
    let (two, effects) = step(&mut tw, rae, "+build/answer 1/2=Three.");
    assert!(
        sys_to(&two, rae).contains("planned again"),
        "{}",
        sys_to(&two, rae)
    );
    mock.push_response(propose_and_validate(
        site_entries("Pond Park", "site.park", rect),
        0.30,
    ));
    finish(&mut tw, &llm, rae, &effects).await;
    assert_eq!(proposal(&tw, 1).status, ProposalStatus::Ready);
    let last = mock.last_request().expect("a request");
    let opening = last
        .messages
        .first()
        .map(|m| format!("{:?}", m.content))
        .unwrap_or_default();
    assert!(
        opening.contains("A pond.") && opening.contains("Three."),
        "{opening}"
    );
    // A question that does not exist is refused.
    let (bad, _) = step(&mut tw, rae, "+build/answer 1/9=no");
    assert!(
        sys_to(&bad, rae).contains("no question 9"),
        "{}",
        sys_to(&bad, rae)
    );
}

// ----------------------------------------------------------------------
// Reconceptualize, secrets, boot
// ----------------------------------------------------------------------

#[tokio::test]
async fn reconceptualizing_a_pack_keeps_the_access_point_and_diffs_against_the_latch() {
    let (mut tw, mock, llm) = rig();
    let (open, effects) = step(&mut tw, GOD, "@atlas/propose --from neighborhood.block");
    assert!(
        sys_to(&open, GOD).contains("Proposal 1 is being planned"),
        "{}",
        sys_to(&open, GOD)
    );
    // The model keeps the neighborhood, the Green and the way in, redescribes the
    // Green and adds a Pond.
    let mut entries = serde_json::to_value(block()).expect("json")["entries"].clone();
    entries["room.green"]["def"]["desc"] = json!("A wide green, freshly mown.");
    entries["room.pond"] = json!({ "kind": "room", "def": { "name": "The Pond", "surface": "ground",
        "footprint": { "rect": [[60, 10], [100, 60]] } } });
    entries["exit.green_pond"] = json!({ "kind": "exit", "def": { "from": "room.green", "to": "room.pond",
        "names": ["Pond <P>;p", "Green <G>;g"] } });
    mock.push_response(propose_and_validate(entries, 0.60));
    let done = finish(&mut tw, &llm, GOD, &effects).await;
    assert!(
        sys_to(&done, GOD).contains("Proposal 1 is ready"),
        "{}",
        sys_to(&done, GOD)
    );
    let rec = proposal(&tw, 1);
    let pack = rec.pack.as_ref().expect("pack");
    assert_eq!(pack.manifest().id().as_str(), "neighborhood.block");
    assert_eq!(pack.manifest().version(), 2);
    let (shown, _) = step(&mut tw, GOD, "+build/show 1");
    let shown = sys_to(&shown, GOD);
    assert!(
        shown.contains("against neighborhood.block@1: 2 create, 1 update, 0 retire, 2 unchanged"),
        "{shown}"
    );
    // The prompt carried the current pack to keep the access point.
    let last = mock.last_request().expect("a request");
    let opening = last
        .messages
        .first()
        .map(|m| format!("{:?}", m.content))
        .unwrap_or_default();
    assert!(
        opening.contains("Reconceptualize pack neighborhood.block@1"),
        "{opening}"
    );
    assert!(opening.contains("exit.limbo_green"), "{opening}");
    // Applying it: the way in from Limbo is unchanged, the Green is updated, the Pond is new.
    let (went, _) = step(&mut tw, GOD, "+build/go 1");
    let went = sys_to(&went, GOD);
    assert!(
        went.contains("Applied neighborhood.block@2") && went.contains("2 created, 1 updated"),
        "{went}"
    );
    assert!(room(&tw, "The Pond").is_some());
}

#[tokio::test]
async fn a_secret_in_a_persona_is_refused_by_the_validator() {
    let (mut tw, mock, llm) = rig();
    let before = live_objects(&tw);
    let (_, effects) = step(
        &mut tw,
        GOD,
        "@atlas/propose The Block=A cottage with a secret.",
    );
    let mut entries = site_entries("The Cottage", "site.house", [[100.0, 10.0], [140.0, 50.0]]);
    entries["secret.will"] =
        json!({ "kind": "secret", "def": { "truth": "The will names the gardener." } });
    entries["npc.aunt"] = json!({ "kind": "npc", "def": { "name": "Aunt May", "room": "room.plot",
        "profile": { "persona": "Kindly; she knows the will names the gardener." } } });
    mock.set_default(Ok(propose_and_validate(entries, 0.20)));
    let done = finish(&mut tw, &llm, GOD, &effects).await;
    assert!(
        sys_to(&done, GOD).contains("needs work"),
        "{}",
        sys_to(&done, GOD)
    );
    let rec = proposal(&tw, 1);
    assert!(
        rec.findings
            .iter()
            .any(|f| f.code == "secrets.leak" && f.blocks()),
        "{:?}",
        rec.findings
    );
    assert_eq!(live_objects(&tw), before);
    // The tool result told the model what to fix.
    let last = mock.last_request().expect("a request");
    let results = format!("{:?}", last.messages);
    assert!(results.contains("secrets.leak"), "{results}");
}

#[tokio::test]
async fn proposals_and_their_counter_survive_a_reboot() {
    let (mut tw, mock, llm) = rig();
    let rae = green_player(&mut tw, "Rae");
    let (_, effects) = step(&mut tw, rae, "+build/place park=A park.");
    let rect = reserved_rect(&tw);
    mock.push_response(propose_and_validate(
        site_entries("Rae's Park", "site.park", rect),
        0.40,
    ));
    finish(&mut tw, &llm, rae, &effects).await;
    // A second proposal left mid-run.
    let (_, _effects) = step(&mut tw, rae, "+build/place garden=A garden.");
    assert_eq!(proposal(&tw, 2).status, ProposalStatus::Planning);

    let ops = tw.take_module_ops();
    let mut boot: HashMap<SmolStr, ModuleBootData> = HashMap::new();
    for op in ops {
        let data = boot.entry(op.ns.clone()).or_default();
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                data.kv.retain(|(k, _)| *k != key);
                data.kv.push((key, value));
            }
            ModuleWrite::Del { key } => data.kv.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    tw.modules_loaded(boot);
    let first = proposal(&tw, 1);
    assert_eq!(first.status, ProposalStatus::Ready);
    assert!(first.pack.is_some());
    assert!(matches!(
        proposal(&tw, 2).status,
        ProposalStatus::Failed {
            retryable: true,
            ..
        }
    ));
    let st = tw
        .world()
        .module_state::<mush_worldgen::WorldgenState>()
        .expect("worldgen");
    assert_eq!(st.next_proposal, 3);
    // The book still works: a third proposal takes the next id.
    let (open, _) = step(&mut tw, rae, "+build/feedback 2=try again");
    assert!(
        sys_to(&open, rae).contains("planned again"),
        "{}",
        sys_to(&open, rae)
    );
}

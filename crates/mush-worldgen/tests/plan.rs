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
                "region": "region.town", "name": "The Road", "kind": "journey",
                "from": { "neighborhood": "neighborhood.block", "point": [30, 30, 0] },
                "to": { "neighborhood": "neighborhood.far", "point": [300, 300, 0] },
                "direction": "both", "modes": ["foot"], "corridor": [[30, 30, 0], [300, 300, 0]] } }
        }
    }))
}

fn block() -> Pack {
    block_with(true, 4)
}

fn block_with(enabled: bool, max_player_sites: u32) -> Pack {
    signed(json!({
        "manifest": { "id": "neighborhood.block", "version": 1, "kind": "neighborhood", "digest": "",
                      "requires": [{ "id": "region.town", "version": 1 }] },
        "entries": {
            "neighborhood": { "kind": "neighborhood", "def": {
                "region": "region.town", "locality": "locality.town", "name": "The Block",
                "setting": "suburban", "footprint": [[0, 0], [200, 0], [200, 200], [0, 200]],
                "room_budget": { "target": 3, "maximum": 12 },
                "hydration": { "enabled": enabled, "max_player_sites": max_player_sites } } },
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
fn town_with_block(llm: &Arc<LlmService>, nb_pack: Pack) -> TestWorld {
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
            .with_shelf(vec![nb_pack.clone()]),
        ),
        Box::new(
            ContentModule::new()
                .with_appliers(appliers)
                .with_boot_packs(vec![region(), nb_pack]),
        ),
        Box::new(WorldgenModule::with_config(Arc::clone(llm), config())),
    ];
    TestWorld::with_modules(modules)
}

fn rig() -> (TestWorld, Arc<MockProvider>, Arc<LlmService>) {
    rig_with_block(block())
}

fn rig_with_block(nb_pack: Pack) -> (TestWorld, Arc<MockProvider>, Arc<LlmService>) {
    let mock = Arc::new(MockProvider::new("openrouter"));
    let llm = service(&mock);
    let tw = town_with_block(&llm, nb_pack);
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

/// WS03-A1: nothing this route wrote was DROPPED for want of a declared
/// cross-namespace delegation. A refused write still mutates RAM, so the
/// session looks right and only a reboot shows the loss — `+build` reaches the
/// content latch, both crates' appliers and the jobs module's ticket under
/// worldgen's identity, and this is what says all four landed.
fn no_refused_writes(tw: &TestWorld, after: &str) {
    let refused: Vec<String> = tw
        .world()
        .refused_namespace_writes()
        .into_iter()
        .map(|(module, ns, _)| format!("{module} -> {ns}"))
        .collect();
    assert!(
        refused.is_empty(),
        "{after} made cross-namespace write(s) that did not land: {refused:?}. Declare the \
         pair with Registrar::persist_delegation and pin its row in module-ns-owners.toml."
    );
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
            "to": "room.plot", "names": [format!("{name} <P>;plot"), "Green <G>;g"],
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
    // `PolygonCm::bbox()` is max-exclusive (+1 cm); subtract 1 cm so the rect
    // matches `rec.footprint`'s actual upper corner.
    [
        [m(bbox.min.x), m(bbox.min.y)],
        [m(bbox.max.x - 1), m(bbox.max.y - 1)],
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
    no_refused_writes(&tw, "a player's +build/go");
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
    // Rae is not the author and not staff (uniform refusal hides proposal existence).
    let (went, _) = step(&mut tw, rae, "+build/go 1");
    assert!(
        sys_to(&went, rae).contains("No proposal 1."),
        "{}",
        sys_to(&went, rae)
    );
    let (shown, _) = step(&mut tw, rae, "+build/show 1");
    assert!(
        sys_to(&shown, rae).contains("No proposal 1."),
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
    // Going twice is refused — and that refusal is durable only if the latch
    // write actually landed, so check nothing on the route was dropped.
    let (again, _) = step(&mut tw, GOD, "+build/go 1");
    assert!(
        sys_to(&again, GOD).contains("applied"),
        "{}",
        sys_to(&again, GOD)
    );
    no_refused_writes(&tw, "a staff +build/go");
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
        "names": ["Pond <P>;pond", "Green <G>;g"] } });
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

// ---------------------------------------------------------------------------
// S6 (doc 14 §4.6): the set designer's and the front office's interfaces —
// the proposal and the ticket are the asker's (A, $), a status is a read (M)
// ---------------------------------------------------------------------------

fn invoke_interface(
    tw: &mut TestWorld,
    provider: &str,
    asker: ObjId,
    ability: &str,
    input: Value,
) -> Result<mush_core::InterfaceReply, mush_core::InterfaceError> {
    let engine = tw.engine_mut();
    let provider = engine
        .registry
        .interface_provider(provider)
        .unwrap_or_else(|| panic!("no provider {provider}"));
    let mut out = mush_core::Outbox::new();
    let mut ctx = mush_core::Ctx {
        w: &mut engine.world,
        actor: asker,
        switches: mush_core::SwitchSet::default(),
        args: mush_core::ParsedArgs::default(),
        out: &mut out,
        reg: &engine.registry,
        module: Some(SmolStr::new("npc")),
        typed_state: mush_core::ModuleActionState::unavailable(),
    };
    provider.invoke(
        &mut ctx,
        mush_core::AbilityCall { npc: GOD, asker },
        ability,
        &input,
    )
}

#[test]
fn the_set_designer_drafts_in_the_askers_name_and_never_applies() {
    let (mut tw, _mock, _llm) = rig();
    let alice = green_player(&mut tw, "Alice");
    let bob = green_player(&mut tw, "Bob");
    // Nothing on the desk yet: a read, and nothing written.
    let _ = tw.engine_mut().take_module_ops();
    let reply =
        invoke_interface(&mut tw, "build", alice, "build.status", json!({})).expect("status");
    assert!(
        reply.text.contains("No proposals of yours"),
        "{}",
        reply.text
    );
    assert!(
        tw.engine_mut().take_module_ops().is_empty(),
        "a status is a read"
    );
    // A brief becomes a proposal owned by the asker.
    let reply = invoke_interface(
        &mut tw,
        "build",
        alice,
        "build.propose",
        json!({ "brief": "A noodle bar on the corner, open late, with a back room." }),
    )
    .expect("a proposal");
    assert!(reply.text.contains("is being planned"), "{}", reply.text);
    let reply =
        invoke_interface(&mut tw, "build", alice, "build.status", json!({})).expect("status");
    assert!(
        reply.text.contains("1 proposal(s)") && reply.text.contains("noodle bar"),
        "{}",
        reply.text
    );
    // Bob asks the same NPC: Alice's proposal is not his to see.
    let reply = invoke_interface(&mut tw, "build", bob, "build.status", json!({})).expect("status");
    assert!(
        reply.text.contains("No proposals of yours"),
        "{}",
        reply.text
    );
    // Answering a question on a proposal that is not yours is refused; an
    // empty brief is invalid; the NPC has no `build.apply`.
    let err = invoke_interface(
        &mut tw,
        "build",
        bob,
        "build.answer",
        json!({ "id": 1, "n": 1, "text": "yes" }),
    )
    .expect_err("not Bob's");
    assert!(
        matches!(err, mush_core::InterfaceError::Refused(_)),
        "{err:?}"
    );
    assert!(matches!(
        invoke_interface(
            &mut tw,
            "build",
            alice,
            "build.propose",
            json!({ "brief": "   " })
        ),
        Err(mush_core::InterfaceError::Invalid(_))
    ));
    assert!(matches!(
        invoke_interface(&mut tw, "build", alice, "build.apply", json!({ "id": 1 })),
        Err(mush_core::InterfaceError::Unknown(_))
    ));
    // The proposal record is Alice's: +build/status prints it for her alone.
    let events = tw.run(alice, "+build/status");
    let text: String = events
        .iter()
        .map(|(_, e)| mush_testkit::text_of(e))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("noodle bar"), "{text}");
}

#[test]
fn the_front_office_files_and_shows_only_the_askers_own_tickets() {
    let (mut tw, _mock, _llm) = rig();
    let alice = tw.spawn_player("Alice");
    let bob = tw.spawn_player("Bob");
    let reply = invoke_interface(
        &mut tw,
        "request",
        alice,
        "request.file",
        json!({ "title": "A key to the writers' room", "text": "I left my notes inside." }),
    )
    .expect("filed");
    assert!(reply.text.contains("Filed as request #"), "{}", reply.text);
    let reply =
        invoke_interface(&mut tw, "request", alice, "request.status", json!({})).expect("status");
    assert!(reply.text.contains("writers' room"), "{}", reply.text);
    // Bob cannot be shown Alice's ticket through the same NPC.
    let reply =
        invoke_interface(&mut tw, "request", bob, "request.status", json!({})).expect("status");
    assert!(reply.text.contains("no requests on file"), "{}", reply.text);
    // A status is a read; a filing is the one write, the asker's.
    let _ = tw.engine_mut().take_module_ops();
    invoke_interface(&mut tw, "request", alice, "request.status", json!({})).expect("status");
    assert!(tw.engine_mut().take_module_ops().is_empty());
    assert!(matches!(
        invoke_interface(&mut tw, "request", alice, "request.file", json!({})),
        Err(mush_core::InterfaceError::Invalid(_))
    ));
    // The ticket is in the queue as Alice's, for staff to work.
    let filed =
        invoke_interface(&mut tw, "request", alice, "request.status", json!({})).expect("status");
    let id: u64 = filed
        .text
        .trim_start_matches('#')
        .split(' ')
        .next()
        .and_then(|n| n.parse().ok())
        .expect("the ticket number");
    let job = mush_jobs::get_job(tw.world(), id).expect("the job");
    assert_eq!(job.opener, alice);
    assert!(job.tags.iter().any(|t| t == "interface"));
}

/// STRAG-M9 (worldgen): withdrawing a proposal cancels queued/in-flight LLM
/// `ModuleAsync` jobs (0 LLM calls when withdrawn before start), prunes
/// withdrawn proposals past `MAX_RETAINED_TERMINAL_PROPOSALS` with disk
/// deletion and monotonic IDs across reboot, and enforces per-player rate limits.
#[tokio::test]
async fn strag_m9_withdraw_cancels_inflight_llm_prunes_proposals_and_enforces_rate_limits() {
    let (mut tw, mock, llm) = rig();
    let rae = green_player(&mut tw, "Rae");
    let green = tw.world().get(rae).and_then(|o| o.location).expect("green");

    // 1. Place a build request -> deferred ModuleAsync is queued; withdraw BEFORE
    //    the LLM job runs -> `run_plan` sees the cancellation token and makes 0 LLM calls!
    let (_events, effects) = step(&mut tw, rae, "+build/place park=A bench under an oak tree.");
    let async_effects: Vec<Effect> = effects
        .into_iter()
        .filter(|e| matches!(e, Effect::ModuleAsync { .. }))
        .collect();
    assert_eq!(async_effects.len(), 1);
    let rev_before = proposal(&tw, 1).revision;

    let (wd_events, _) = step(&mut tw, rae, "+build/withdraw 1");
    assert!(sys_to(&wd_events, rae).contains("withdrawn"));
    assert!(
        proposal(&tw, 1).revision > rev_before,
        "withdraw must bump proposal revision to invalidate stale outcomes"
    );

    // Now drive the deferred ModuleAsync payload: it must exit immediately with 0 LLM calls.
    mock.push_response(read("neighborhood"));
    let _ = finish(&mut tw, &llm, rae, &async_effects).await;
    assert_eq!(
        mock.call_count(),
        0,
        "cancelled off-loop planning job must make zero LLM calls"
    );
    assert_eq!(proposal(&tw, 1).status, ProposalStatus::Withdrawn);

    // 2. Staff churn past MAX_RETAINED_TERMINAL_PROPOSALS prunes old withdrawn proposals
    //    on RAM and disk while keeping monotonic IDs.
    tw.world_mut().move_to(GOD, green).expect("god to green");
    let total = mush_worldgen::MAX_RETAINED_TERMINAL_PROPOSALS + 20;
    let mut last_id = 1u64;
    for i in 0..total {
        let (_, _) = step(
            &mut tw,
            GOD,
            &format!("+build/place park_{i}=Brief number {i}."),
        );
        let newest = tw
            .world()
            .module_state::<mush_worldgen::WorldgenState>()
            .expect("wg")
            .proposals
            .keys()
            .next_back()
            .copied()
            .expect("proposal");
        last_id = newest;
        let _ = step(&mut tw, GOD, &format!("+build/withdraw {newest}"));
    }
    let retained = tw
        .world()
        .module_state::<mush_worldgen::WorldgenState>()
        .expect("wg")
        .proposals
        .len();
    assert!(
        retained <= mush_worldgen::MAX_RETAINED_TERMINAL_PROPOSALS,
        "expected <= {}, got {retained}",
        mush_worldgen::MAX_RETAINED_TERMINAL_PROPOSALS
    );
    let ops = tw.take_module_ops();
    let del_count = ops
        .iter()
        .filter(|op| {
            op.ns.as_str() == "worldgen"
                && matches!(&op.op, ModuleWrite::Del { key } if key.starts_with("proposal:"))
        })
        .count();
    assert!(
        del_count >= 20,
        "pruned proposals must emit ModuleWrite::Del"
    );

    // 3. Boot hydration with > MAX_PROPOSALS withdrawn docs + 1 open doc preserves the open
    //    doc, prunes excess withdrawn docs, and keeps next_proposal strictly monotonic.
    let sample = tw
        .world()
        .module_state::<mush_worldgen::WorldgenState>()
        .expect("wg")
        .proposals
        .values()
        .next()
        .expect("sample")
        .clone();
    let mut synthetic_kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    for i in 1..=(u64::try_from(mush_worldgen::MAX_PROPOSALS + 10).unwrap_or(1034)) {
        let mut p = sample.clone();
        p.id = i;
        p.updated = i64::try_from(i).unwrap_or(0);
        p.status = ProposalStatus::Withdrawn;
        synthetic_kv.push((
            mush_worldgen::proposal_key(i),
            serde_json::to_value(&p).expect("json"),
        ));
    }
    let open_id = u64::try_from(mush_worldgen::MAX_PROPOSALS + 11).unwrap_or(1035);
    let mut open_p = sample.clone();
    open_p.id = open_id;
    open_p.status = ProposalStatus::Ready;
    synthetic_kv.push((
        mush_worldgen::proposal_key(open_id),
        serde_json::to_value(&open_p).expect("json"),
    ));
    let (hydrated, next_id, dropped) = mush_worldgen::proposal::hydrate_boot(&synthetic_kv);
    assert!(
        hydrated.contains_key(&open_id),
        "open proposal must survive boot pruning"
    );
    assert_eq!(
        hydrated.len(),
        mush_worldgen::MAX_RETAINED_TERMINAL_PROPOSALS + 1
    );
    assert!(
        next_id > open_id,
        "next_proposal must stay monotonic: {next_id}"
    );
    assert_eq!(
        dropped.len(),
        mush_worldgen::MAX_PROPOSALS + 10 - mush_worldgen::MAX_RETAINED_TERMINAL_PROPOSALS
    );
    let _ = last_id;

    // 4. Non-staff player withdrawal cooldown & hourly withdrawal cap block churn.
    //    Rae already withdrew 1 proposal above; a 2nd withdraw triggers cooldown on the 3rd place.
    let _ = step(&mut tw, rae, "+build/place bench=Second bench.");
    let p2 = tw
        .world()
        .module_state::<mush_worldgen::WorldgenState>()
        .expect("wg")
        .proposals
        .values()
        .filter(|p| p.author == rae && p.status.open())
        .map(|p| p.id)
        .max()
        .expect("p2");
    let _ = step(&mut tw, rae, &format!("+build/withdraw {p2}"));
    let (cooled_ev, _) = step(&mut tw, rae, "+build/place bench3=Third bench immediately.");
    assert!(
        sys_to(&cooled_ev, rae).contains("after withdrawing"),
        "expected withdrawal cooldown refusal: {}",
        sys_to(&cooled_ev, rae)
    );
}

/// **P2-MOD-28**: `BuildInterface::invoke("build.propose")` routes non-staff
/// players through `proposal::place` / `mush_atlas::hydrate::request` (enforcing
/// neighborhood presence, `nb.hydration.enabled`, strike suspension `cap == 0`,
/// slot caps, and footprints), `proposal::create` enforces
/// `MAX_OPEN_PROPOSALS_PER_PLAYER`, and stale/idle open proposals (`Failed`,
/// `Capped`, `Ready`, etc.) are pruned so global `MAX_PROPOSALS` can never be
/// permanently exhausted.
#[test]
fn p2_mod_28_build_propose_enforces_hydration_gates_open_cap_and_stale_pruning() {
    let (mut tw, _mock, _llm) = rig_with_block(block_with(true, 32));

    // 1. Non-staff player outside any neighborhood (LIMBO) cannot bypass into
    //    `ProposeScope::World` via `build.propose`.
    let drifter = tw.spawn_player("Drifter");
    let err = invoke_interface(
        &mut tw,
        "build",
        drifter,
        "build.propose",
        json!({ "brief": "A rogue continent." }),
    )
    .expect_err("off-neighborhood non-staff build.propose must be refused");
    assert!(
        matches!(err, mush_core::InterfaceError::Refused(ref msg) if msg.contains("no neighborhood")),
        "{err:?}"
    );
    assert!(
        tw.world()
            .module_state::<mush_worldgen::WorldgenState>()
            .expect("wg")
            .proposals
            .is_empty(),
        "refused build.propose must create zero proposals"
    );

    // 2. Strike-suspended player (`cap == 0`) in an enabled neighborhood is refused.
    let suspended = green_player(&mut tw, "Suspended");
    let _ = step(&mut tw, GOD, &format!("@atlas/cap #{}=0", suspended.0));
    let err = invoke_interface(
        &mut tw,
        "build",
        suspended,
        "build.propose",
        json!({ "brief": "A banned tavern." }),
    )
    .expect_err("strike-suspended player must be refused by build.propose");
    assert!(
        matches!(err, mush_core::InterfaceError::Refused(ref msg) if msg.contains("allows no requests")),
        "{err:?}"
    );

    // 3. Per-player hydration slot cap (`hydration_cap == 2`) is enforced through `build.propose`.
    let builder = green_player(&mut tw, "Builder");
    for i in 0..2 {
        let reply = invoke_interface(
            &mut tw,
            "build",
            builder,
            "build.propose",
            json!({ "brief": format!("Shop number {i} on the square."), "kind": format!("shop_{i}") }),
        )
        .expect("within slot cap");
        assert!(reply.text.contains("is being planned"), "{}", reply.text);
    }
    let over_slots = invoke_interface(
        &mut tw,
        "build",
        builder,
        "build.propose",
        json!({ "brief": "Third shop past hydration slot cap.", "kind": "shop_2" }),
    )
    .expect_err("3rd concurrent build.propose must hit hydration slot cap");
    assert!(
        matches!(over_slots, mush_core::InterfaceError::Refused(ref msg) if msg.contains("standing allows 2")),
        "{over_slots:?}"
    );

    // 4. Neighborhood with `hydration.enabled == false` refuses `build.propose`.
    {
        let (mut locked_tw, _m, _l) = rig_with_block(block_with(false, 0));
        let newcomer = green_player(&mut locked_tw, "Newcomer");
        let disabled_err = invoke_interface(
            &mut locked_tw,
            "build",
            newcomer,
            "build.propose",
            json!({ "brief": "A stall in a locked district." }),
        )
        .expect_err("disabled neighborhood must refuse build.propose");
        assert!(
            matches!(disabled_err, mush_core::InterfaceError::Refused(ref msg) if msg.contains("does not take player-built sites")),
            "{disabled_err:?}"
        );
    }

    // 5. Per-player open-proposal cap (`MAX_OPEN_PROPOSALS_PER_PLAYER`) blocks a
    //    player even if their hydration slot cap is raised higher.
    let capped = green_player(&mut tw, "CappedPlayer");
    let _ = step(&mut tw, GOD, &format!("@atlas/cap #{}=10", capped.0));
    for i in 0..mush_worldgen::MAX_OPEN_PROPOSALS_PER_PLAYER {
        let reply = invoke_interface(
            &mut tw,
            "build",
            capped,
            "build.propose",
            json!({ "brief": format!("Bench {i}."), "kind": format!("bench_{i}") }),
        )
        .expect("within MAX_OPEN_PROPOSALS_PER_PLAYER");
        assert!(reply.text.contains("is being planned"), "{}", reply.text);
    }
    let over_open = invoke_interface(
        &mut tw,
        "build",
        capped,
        "build.propose",
        json!({ "brief": "One too many open proposals.", "kind": "bench_extra" }),
    )
    .expect_err("exceeding MAX_OPEN_PROPOSALS_PER_PLAYER must refuse");
    assert!(
        matches!(over_open, mush_core::InterfaceError::Refused(ref msg) if msg.contains("open proposals")),
        "{over_open:?}"
    );

    // 6. Global MAX_PROPOSALS saturated with open Failed/Capped/Ready proposals
    //    never permanently blocks new staff or player proposals: oldest/lowest-tier
    //    idle open proposals are pruned with disk deletion and monotonic ids.
    let template = tw
        .world()
        .module_state::<mush_worldgen::WorldgenState>()
        .expect("wg")
        .proposals
        .values()
        .next()
        .expect("sample proposal")
        .clone();
    let now = tw.world().now();
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
    let wg_boot = boot.entry(SmolStr::new_static("worldgen")).or_default();
    wg_boot.kv.retain(|(k, _)| !k.starts_with("proposal:"));
    for i in 1..=(u64::try_from(mush_worldgen::MAX_PROPOSALS).unwrap_or(1024)) {
        let mut p = template.clone();
        p.id = i;
        p.author = ObjId(500 + u32::try_from(i % 200).unwrap_or(0));
        p.staff = false;
        p.source = mush_worldgen::ProposalSource::Prose {
            text: format!("Abandoned {i}"),
        };
        p.updated = now.saturating_sub(i64::try_from(i).unwrap_or(0));
        p.status = match i % 3 {
            0 => ProposalStatus::Failed {
                message: "model error".to_string(),
                retryable: false,
            },
            1 => ProposalStatus::Capped,
            _ => ProposalStatus::Ready,
        };
        wg_boot.kv.push((
            mush_worldgen::proposal_key(i),
            serde_json::to_value(&p).expect("json"),
        ));
    }
    tw.modules_loaded(boot);
    assert_eq!(
        tw.world()
            .module_state::<mush_worldgen::WorldgenState>()
            .expect("wg")
            .proposals
            .len(),
        mush_worldgen::MAX_PROPOSALS,
        "reboot must load all MAX_PROPOSALS open records"
    );
    let _ = tw.take_module_ops();
    let (staff_ev, _) = step(&mut tw, GOD, "@atlas/propose The Block=Fresh staff plaza.");
    let staff_text = sys_to(&staff_ev, GOD);
    assert!(
        staff_text.contains("is being planned"),
        "full book of Failed/Capped/Ready proposals must prune and admit new work: {staff_text}"
    );
    let ops = tw.take_module_ops();
    assert!(
        ops.iter().any(|op| {
            op.ns.as_str() == "worldgen"
                && matches!(&op.op, ModuleWrite::Del { key } if key.starts_with("proposal:"))
        }),
        "capacity eviction must emit ModuleWrite::Del for pruned open proposal"
    );
    assert!(
        tw.world()
            .module_state::<mush_worldgen::WorldgenState>()
            .expect("wg")
            .proposals
            .len()
            <= mush_worldgen::MAX_PROPOSALS
    );

    // 7. Advancing past STALE_PROPOSAL_TTL_SECS automatically prunes stale open proposals.
    let stale_now = now + mush_worldgen::STALE_PROPOSAL_TTL_SECS + 10;
    tw.set_now(stale_now);
    let _ = tw.take_module_ops();
    let (after_stale, _) = step(&mut tw, GOD, "@atlas/propose The Block=After TTL.");
    assert!(
        sys_to(&after_stale, GOD).contains("is being planned"),
        "{}",
        sys_to(&after_stale, GOD)
    );
    let remaining = tw
        .world()
        .module_state::<mush_worldgen::WorldgenState>()
        .expect("wg")
        .proposals
        .len();
    assert_eq!(
        remaining, 1,
        "all stale open proposals past STALE_PROPOSAL_TTL_SECS must be pruned"
    );
}

/// P2-MOD-87:
/// - `+build/show <id>` returns a uniform refusal indistinguishable from a
///   nonexistent proposal for non-owned proposals (no existence oracle).
/// - `proposal::answer` enforces open-state, `retryable: false`, and `MAX_FEEDBACK`
///   without mutating closed/non-retryable proposals or bypassing the feedback cap.
#[tokio::test]
async fn p2_mod_87_proposal_answer_guards_and_build_show_uniform_refusal() {
    let (mut tw, mock, llm) = rig();
    let alice = green_player(&mut tw, "Alice");
    let bob = green_player(&mut tw, "Bob");
    tw.set_now(1_000);

    // Alice creates proposal 1 and gets 1 question back (`NeedsAnswers`).
    let (_, effects) = step(&mut tw, alice, "+build/place park=A quiet garden.");
    mock.push_response(ask(&["What kind of trees?"]));
    finish(&mut tw, &llm, alice, &effects).await;
    assert_eq!(proposal(&tw, 1).status, ProposalStatus::NeedsAnswers);

    // 1. `+build/show <id>` existence oracle: Bob probing Alice's proposal 1 gets
    //    the exact same shape as probing nonexistent proposal 9999.
    let (other_ev, _) = step(&mut tw, bob, "+build/show 1");
    let (missing_ev, _) = step(&mut tw, bob, "+build/show 9999");
    assert_eq!(
        sys_to(&other_ev, bob).replace('1', "N"),
        sys_to(&missing_ev, bob).replace("9999", "N"),
        "+build/show must not distinguish another player's proposal from missing"
    );
    assert!(
        !sys_to(&other_ev, bob).contains("is not yours"),
        "existence oracle text leaked: {}",
        sys_to(&other_ev, bob)
    );

    // 2. Answering Q1 triggers planning, then the plan fails permanently (`retryable: false`).
    let (_, fx) = step(&mut tw, alice, "+build/answer 1/1=Oaks.");
    assert!(fx.iter().any(|e| matches!(e, Effect::ModuleAsync { .. })));
    mock.set_default(Err(mush_llm::LlmError::Refused {
        category: Some("policy".into()),
    }));
    finish(&mut tw, &llm, alice, &fx).await;
    assert!(
        matches!(
            proposal(&tw, 1).status,
            ProposalStatus::Failed {
                retryable: false,
                ..
            }
        ),
        "expected permanent failure, got {:?}",
        proposal(&tw, 1).status
    );

    // `proposal::answer` MUST refuse a `retryable: false` proposal without mutating its answer!
    let (fail_ev, fail_fx) = step(&mut tw, alice, "+build/answer 1/1=Pines.");
    assert!(
        fail_fx
            .iter()
            .all(|e| !matches!(e, Effect::ModuleAsync { .. })),
        "retryable:false must not restart planning"
    );
    assert!(
        sys_to(&fail_ev, alice).contains("failed permanently"),
        "{}",
        sys_to(&fail_ev, alice)
    );
    assert_eq!(
        proposal(&tw, 1).questions[0].answer.as_deref(),
        Some("Oaks."),
        "refused answer must not mutate failed proposal"
    );

    // 3. Withdrawn (closed) proposal cannot be mutated via `proposal::answer`.
    let (wd_ev, _) = step(&mut tw, alice, "+build/withdraw 1");
    assert!(
        sys_to(&wd_ev, alice).contains("withdrawn"),
        "{}",
        sys_to(&wd_ev, alice)
    );
    let (closed_ev, closed_fx) = step(&mut tw, alice, "+build/answer 1/1=Birches.");
    assert!(
        closed_fx
            .iter()
            .all(|e| !matches!(e, Effect::ModuleAsync { .. }))
    );
    assert!(
        sys_to(&closed_ev, alice).contains("withdrawn"),
        "{}",
        sys_to(&closed_ev, alice)
    );
    assert_eq!(
        proposal(&tw, 1).questions[0].answer.as_deref(),
        Some("Oaks."),
        "closed proposal must not be mutated by answer"
    );

    // 4. Re-answering an already-answered question on a NeedsWork proposal
    //    cannot bypass `MAX_FEEDBACK` to run unlimited planning rounds.
    let (_, fx2) = step(&mut tw, alice, "+build/place park=Another garden.");
    mock.push_response(ask(&["Color?"]));
    finish(&mut tw, &llm, alice, &fx2).await;
    assert_eq!(proposal(&tw, 2).status, ProposalStatus::NeedsAnswers);

    // Drive each planning run into NeedsWork through the real async completion path.
    mock.set_default(Ok(propose_and_validate(
        site_entries("Far Park", "site.park", [[190.0, 190.0], [230.0, 230.0]]),
        0.01,
    )));

    // Initial answer consumes 0 feedback rounds and moves to NeedsWork.
    let (_, fx_init) = step(&mut tw, alice, "+build/answer 2/1=Green.");
    assert!(
        fx_init
            .iter()
            .any(|e| matches!(e, Effect::ModuleAsync { .. }))
    );
    finish(&mut tw, &llm, alice, &fx_init).await;
    assert!(matches!(
        proposal(&tw, 2).status,
        ProposalStatus::NeedsWork { .. }
    ));

    // Re-answer Q1 `MAX_FEEDBACK` times (transitioning back to NeedsWork each time).
    for round in 0..mush_worldgen::proposal::MAX_FEEDBACK {
        let (_, rfx) = step(&mut tw, alice, &format!("+build/answer 2/1=Green v{round}"));
        assert!(
            rfx.iter().any(|e| matches!(e, Effect::ModuleAsync { .. })),
            "re-answer {round} within MAX_FEEDBACK allowed"
        );
        finish(&mut tw, &llm, alice, &rfx).await;
        assert!(matches!(
            proposal(&tw, 2).status,
            ProposalStatus::NeedsWork { .. }
        ));
    }
    // The next re-answer MUST hit MAX_FEEDBACK and refuse!
    let (cap_ev, cap_fx) = step(&mut tw, alice, "+build/answer 2/1=One too many.");
    assert!(
        cap_fx
            .iter()
            .all(|e| !matches!(e, Effect::ModuleAsync { .. })),
        "re-answer past MAX_FEEDBACK must not trigger run"
    );
    assert!(
        sys_to(&cap_ev, alice).contains("rounds of feedback"),
        "expected MAX_FEEDBACK refusal: {}",
        sys_to(&cap_ev, alice)
    );
}

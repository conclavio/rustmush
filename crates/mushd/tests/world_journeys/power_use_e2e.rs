//! M14 perception-director informant tie-in, end to end through the real mushd
//! module wiring (built-ins + worldgen + the `npc` module grounded by the
//! ruleset adapter + the seeded `toy` ruleset), ungated and fully in-memory —
//! the same harness shape as `intel_e2e` / `perception_e2e`, driven by hand so
//! the per-observer projection is deterministic.
//!
//! Proves the M14 informant loop, retiring M12's power-use-reporting deferral: a
//! character using a *subtle* power (`+toysubtle`, delivered only on a gated
//! sense channel) is PERCEIVED-OR-NOT per onlooker instead of broadcast.
//!
//! - a sharp-eared bartender is a director winner: it files an
//!   `IntelKind::PowerUse` record its principal pulls with `+intel` (V5);
//! - a dull bystander in the room perceives NOTHING — no witnessed line ever
//!   reaches it (V1, no leak);
//! - a dull bartender files NOTHING — it never perceived the act, so its
//!   principal's `+intel` is empty (V1 + V5).
//!
//! 16b cut R-b: the WoD ruleset this was written against is gone, so the subtle
//! power is the toy's `+toysubtle <target>=<text>`, whose channel is an
//! Awareness **threshold** rather than a Discipline gate plus a roll. The
//! "a low pool is not what blinds it — the channel gate is" nuance the WoD
//! version carried has no toy equivalent and is gone with it; what remains is
//! the load-bearing per-observer projection.

use std::sync::Arc;

use mush_core::{Audience, ConnId, Effects, Engine, GOD, Inbound, ObjId, ObjKind, Outbox, World};
use mush_llm::{BudgetConfig, LlmProvider, LlmService, MockProvider, RouteConfig};
use mush_proto::OutputEvent;
use mush_store::InMemoryStore;
use mush_testkit::toy_ruleset::ToyRuleset;

use mushd::modules;
use mushd::world_task::base_registry;

use mush_npc::AgentGrounding;

/// Fixed RNG seed for the toy module → deterministic dice.
const SEED: u64 = 0xB0A7;

// ----------------------------------------------------------------------
// Harness (a trimmed copy of intel_e2e's Rig).
// ----------------------------------------------------------------------

struct Rig {
    engine: Engine,
    bar: ObjId,
}

impl Rig {
    async fn boot(llm: &Arc<LlmService>) -> Rig {
        let store = InMemoryStore::new();
        let registry = base_registry();

        let mut mods = modules::server_modules();
        mods.push(modules::worldgen_module(Arc::clone(llm)));
        let grounding: Arc<dyn AgentGrounding> =
            Arc::new(modules::RulesetAgent::new(ToyRuleset::with_seed(SEED)));
        mods.push(modules::npc_module(Some(grounding)));
        mods.push(Box::new(ToyRuleset::with_seed(SEED)));

        let mut engine =
            Engine::with_modules(World::new_minimal(), registry, mods).expect("module wiring");
        modules::boot_modules(&mut engine, &store)
            .await
            .expect("module boot");
        engine.set_now(1_000);

        let bar = engine
            .world
            .create(ObjKind::Room, "The Tap Room", GOD, None)
            .expect("bar");
        engine.world.move_to(GOD, bar).expect("move god");
        Rig { engine, bar }
    }

    fn set_now(&mut self, secs: i64) {
        self.engine.set_now(secs);
    }

    fn run(&mut self, actor: ObjId, line: &str) -> Vec<(Audience, OutputEvent)> {
        let mut out = Outbox::new();
        let mut fx = Effects::new();
        self.engine.step(
            Inbound::Cmd {
                conn: ConnId::from(0u64),
                actor,
                line: line.to_string(),
            },
            &mut out,
            &mut fx,
        );
        out.take()
    }

    /// Run a command as `actor` and return the plain-text Sys lines it produced.
    fn actor_sys(&mut self, actor: ObjId, line: &str) -> String {
        let mut s = String::new();
        for (_, ev) in self.run(actor, line) {
            if let OutputEvent::Sys(rt) = ev {
                s.push_str(&rt.to_plain_text());
                s.push('\n');
            }
        }
        s
    }

    fn staff(&mut self, line: &str) -> String {
        self.actor_sys(GOD, line)
    }

    fn player(&mut self, name: &str) -> ObjId {
        let id = self
            .engine
            .world
            .create(ObjKind::Player, name, GOD, Some(self.bar))
            .expect("player");
        self.engine.world.set_connected(id, true);
        id
    }
}

fn service() -> Arc<LlmService> {
    let mock = Arc::new(MockProvider::new("openrouter"));
    let route = |max: u32| RouteConfig {
        model: "mock-model".into(),
        max_tokens: max,
        timeout_s: 30,
        reasoning: None,
        reasoning_max_tokens: None,
    };
    let svc = LlmService::builder()
        .provider("openrouter", Arc::clone(&mock) as Arc<dyn LlmProvider>)
        .route("npc_dialogue_major", route(1024))
        .route("npc_dialogue_minor", route(1024))
        .route("npc_dialogue_extra", route(1024))
        .route("npc_summarize", route(768))
        .budgets(BudgetConfig::default())
        .global_enabled(true)
        .build();
    Arc::new(svc)
}

/// Create + configure a bartender NPC `npc_name` with a `template` sheet,
/// reporting to `principal` (the character it files intel for).
fn make_informant_bartender(rig: &mut Rig, npc_name: &str, template: &str, principal: &str) {
    let out = rig.staff(&format!("@agent/create {npc_name}"));
    assert!(
        out.contains(&format!("Created NPC agent {npc_name}")),
        "{out}"
    );
    rig.staff(&format!("@agent/set {npc_name}/persona=a watchful barkeep"));
    rig.staff(&format!("@agent/set {npc_name}/tier=major"));
    rig.staff(&format!(
        "@agent/set {npc_name}/fallback=The bartender says nothing."
    ));
    let out = rig.staff(&format!("@agent/sheet {npc_name}={template}"));
    assert!(out.contains("sheet"), "{out}");
    let out = rig.staff(&format!("@agent/set {npc_name}/principal={principal}"));
    assert!(
        out.contains(&format!("Set principal on {npc_name}")),
        "{out}"
    );
}

/// Give an existing player a toy sheet, so they can work a subtle power at all
/// (`+toysubtle` needs no rating — the gate is on the ONLOOKER's Awareness).
fn make_subtle_actor(rig: &mut Rig, name: &str) {
    rig.staff(&format!("@sheet/set {name}/template=alpha"));
}

/// Everything an `Audience::Actor(who)` event delivered to `who`, as plain text
/// (Sys lines + Narrative bodies) — used to prove a non-perceiver received
/// nothing about a subtle power.
fn text_to(events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    let mut s = String::new();
    for (aud, ev) in events {
        if !matches!(aud, Audience::Actor(a) if *a == who) {
            continue;
        }
        match ev {
            OutputEvent::Sys(rt) => {
                s.push_str(&rt.to_plain_text());
                s.push('\n');
            }
            OutputEvent::Narrative { text, .. } => {
                s.push_str(&text.to_plain_text());
                s.push('\n');
            }
            _ => {}
        }
    }
    s
}

// ----------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------

/// V5 (positive) + V1: a sharp-eared bartender is a director winner when a
/// character works a subtle power — it perceives the act and files a `poweruse`
/// record its principal pulls with `+intel`. A dull bystander sharing the room
/// perceives NOTHING (no witnessed line ever reaches it).
#[tokio::test]
async fn sharp_witnesses_subtle_power_and_files_intel_bystander_sees_nothing() {
    let svc = service();
    let mut rig = Rig::boot(&svc).await;

    let marcus = rig.player("Marcus"); // the principal
    make_informant_bartender(&mut rig, "Ghoulkeep", "alpha", "Marcus");
    // Overwhelming senses: Perception + Awareness 5 — well clear of the gated
    // channel's Awareness 3 threshold.
    rig.staff("@sheet/set Ghoulkeep/perception=5");
    rig.staff("@sheet/set Ghoulkeep/awareness=5");

    // A dull bystander: a plain player with no sheet at all, which reads
    // Awareness 0 — below the channel threshold, so the director skips them.
    let bob = rig.player("Bob");

    let vlad = rig.player("Vlad");
    make_subtle_actor(&mut rig, "Vlad");

    // Vlad works the subtle power on Marcus. The director projects it
    // per-observer: the sharp bartender perceives it (winner) and files intel;
    // the raw detail never leaves the actor's own stream.
    let events = rig.run(vlad, "+toysubtle Marcus=bend to my will");

    // V1: the dull bystander received nothing about the act — no witnessed
    // line, no raw detail, true silence.
    let to_bob = text_to(&events, bob);
    assert!(
        !to_bob.contains("bends another's will"),
        "bystander must not get the witnessed line: {to_bob:?}"
    );
    assert!(
        !to_bob.to_lowercase().contains("subtle"),
        "bystander must not get the raw power detail: {to_bob:?}"
    );

    // V5: Marcus debriefs his asset — a `poweruse` record about Vlad.
    let out = rig.actor_sys(marcus, "+intel");
    assert!(out.contains("Your intel log"), "{out}");
    assert!(out.contains("Vlad"), "the power user is reported: {out}");
    assert!(
        out.contains("[poweruse]"),
        "filed as a witnessed power use: {out}"
    );
    assert!(
        out.contains("used a subtle power on Marcus"),
        "the act's own detail is rendered: {out}"
    );
    assert!(
        out.contains("witnessed directly by Ghoulkeep"),
        "provenance recorded (V5): {out}"
    );

    // Filtering to Vlad shows the same record; an unknown subject is clean.
    let by_vlad = rig.actor_sys(marcus, "+intel Vlad");
    assert!(by_vlad.contains("Intel about Vlad"), "{by_vlad}");
    assert!(by_vlad.contains("[poweruse]"), "{by_vlad}");
}

/// V1 + V5 (negative, the load-bearing property): a bartender whose Awareness
/// falls short of the gated channel is NOT a director winner for a subtle power
/// — it perceives nothing, so it files nothing, and its principal's `+intel` is
/// empty.
#[tokio::test]
async fn dull_bartender_perceives_no_subtle_power_and_files_nothing() {
    let svc = service();
    let mut rig = Rig::boot(&svc).await;

    let lucy = rig.player("Lucy"); // the principal
    make_informant_bartender(&mut rig, "Barback", "alpha", "Lucy");
    // Sharp ordinary senses, but Awareness 2 — one short of the channel's 3.
    rig.staff("@sheet/set Barback/perception=5");
    rig.staff("@sheet/set Barback/awareness=2");

    let vlad = rig.player("Vlad");
    make_subtle_actor(&mut rig, "Vlad");

    // Vlad works the subtle power: the under-threshold bartender is skipped by
    // the director, perceives nothing, and files nothing (V1 — reports only
    // what it perceives).
    rig.run(vlad, "+toysubtle Lucy=bend to my will");
    rig.set_now(1_010);

    let out = rig.actor_sys(lucy, "+intel");
    assert!(
        out.contains("no intel on file"),
        "a bartender that perceived no subtle power files nothing (V1): {out}"
    );
}

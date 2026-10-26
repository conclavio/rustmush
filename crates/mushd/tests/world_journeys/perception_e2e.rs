//! M11 perception vertical, end to end through the real mushd module wiring
//! (built-ins + worldgen + the `npc` module grounded by the ruleset adapter +
//! the seeded `toy` ruleset), ungated and fully in-memory — the same harness
//! shape as `npc_e2e`, driven by hand so the off-loop NPC turn is
//! deterministic.
//!
//! Proves the NPC-as-observer half of the vertical (design D5): a bartender NPC
//! reacts to a hider's speech only when it can *perceive* the hider. A sharp
//! bartender pierces the concealment and takes its turn; a dull one is met with
//! true silence — no stimulus, no turn, no tokens.
//!
//! 16b cut R-b: the WoD ruleset this was written against is gone, so the
//! concealment fixture is the toy's `+toyhide` flag and its Awareness-vs-Stealth
//! `HookPoint::Perceive` gate. The platform property is unchanged; only the
//! splat's dice are.

use std::sync::Arc;

use mush_core::module::TickHandler;
use mush_core::{
    ConnId, Ctx, Effect, Effects, Engine, GOD, Inbound, ObjId, ObjKind, Outbox, ParsedArgs,
    SwitchSet, World,
};
use mush_llm::{BudgetConfig, LlmProvider, LlmService, MockProvider, RouteConfig};
use mush_proto::OutputEvent;
use mush_store::InMemoryStore;
use mush_testkit::toy_ruleset::ToyRuleset;
use smol_str::SmolStr;

use mushd::modules;
use mushd::world_task::base_registry;

use mush_npc::AgentGrounding;

/// Fixed RNG seed for the toy module → deterministic dice.
const SEED: u64 = 0xB0A7;

// ----------------------------------------------------------------------
// Harness (a trimmed copy of npc_e2e's Rig — the parts this vertical needs).
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

    fn run(&mut self, actor: ObjId, line: &str) -> Vec<(mush_core::Audience, OutputEvent)> {
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

    fn staff(&mut self, line: &str) -> String {
        let events = self.run(GOD, line);
        let mut s = String::new();
        for (_, ev) in events {
            if let OutputEvent::Sys(rt) = ev {
                s.push_str(&rt.to_plain_text());
                s.push('\n');
            }
        }
        s
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

    fn npc_id(&self, name: &str) -> ObjId {
        self.engine.world.resolve_player(name).expect("npc exists")
    }

    /// Fire the npc coalescing tick and return the single deferred npc turn, if
    /// any (actor, correlation, serialized snapshot).
    fn one_turn(&mut self) -> Option<(ObjId, u64, String)> {
        let handler: Arc<dyn TickHandler> = self
            .engine
            .registry
            .module_ticks()
            .iter()
            .find(|t| t.module == "npc")
            .expect("npc tick")
            .handler
            .clone();
        let mut out = Outbox::new();
        {
            let mut ctx = Ctx {
                w: &mut self.engine.world,
                actor: GOD,
                switches: SwitchSet::default(),
                args: ParsedArgs::default(),
                out: &mut out,
                reg: &self.engine.registry,
                module: Some(SmolStr::new("npc")),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            handler.tick(&mut ctx, "agent", None);
        }
        out.take_deferred().into_iter().find_map(|d| match d {
            mush_core::Deferred::Effect(Effect::ModuleAsync {
                module,
                actor,
                correlation,
                payload,
            }) if module == "npc" => Some((actor, correlation, payload)),
            _ => None,
        })
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

/// Create + configure a bartender NPC with a `template` sheet, returning its id.
fn make_bartender(rig: &mut Rig, template: &str) -> ObjId {
    let out = rig.staff("@agent/create Bartender");
    assert!(out.contains("Created NPC agent Bartender"), "{out}");
    rig.staff("@agent/set Bartender/persona=a watchful barkeep");
    rig.staff("@agent/set Bartender/tier=major");
    rig.staff("@agent/set Bartender/fallback=The bartender says nothing.");
    let out = rig.staff(&format!("@agent/sheet Bartender={template}"));
    assert!(out.contains("sheet"), "{out}");
    rig.npc_id("Bartender")
}

/// Give an existing player a toy sheet with `stealth` dots, so `+toyhide on`
/// conceals them from anyone whose Awareness falls short.
fn make_hider(rig: &mut Rig, name: &str, stealth: i32) {
    rig.staff(&format!("@sheet/set {name}/template=alpha"));
    rig.staff(&format!("@sheet/set {name}/stealth={stealth}"));
}

// ----------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------

#[tokio::test]
async fn a_dull_bartender_never_notices_a_concealed_speaker() {
    let svc = service();
    let mut rig = Rig::boot(&svc).await;
    let _bar = make_bartender(&mut rig, "alpha"); // Awareness 0

    let vlad = rig.player("Vlad");
    make_hider(&mut rig, "Vlad", 3);

    // Control: while in plain sight, the bartender wakes to Vlad's speech.
    rig.run(vlad, "say Evening.");
    rig.set_now(1_003);
    assert!(
        rig.one_turn().is_some(),
        "a visible speaker wakes the bartender (baseline)"
    );

    // Now Vlad conceals himself and speaks: the dull bartender neither hears
    // nor wakes.
    rig.run(vlad, "+toyhide on");
    rig.run(vlad, "say You never saw me.");
    rig.set_now(1_010);
    assert!(
        rig.one_turn().is_none(),
        "a dull bartender is met with true silence — no stimulus, no turn"
    );
}

#[tokio::test]
async fn a_sharp_bartender_pierces_the_concealment() {
    let svc = service();
    let mut rig = Rig::boot(&svc).await;
    let bar = make_bartender(&mut rig, "alpha");
    // Overwhelming senses: Perception + Awareness 5.
    rig.staff("@sheet/set Bartender/perception=5");
    rig.staff("@sheet/set Bartender/awareness=5");

    let vlad = rig.player("Vlad");
    make_hider(&mut rig, "Vlad", 2);

    // Vlad conceals himself and speaks: Awareness 5 clears Stealth 2, so the
    // bartender still takes its turn against the speaker it senses.
    rig.run(vlad, "+toyhide on");
    rig.run(vlad, "say Do you sense me?");
    rig.set_now(1_003);
    let turn = rig.one_turn();
    assert!(
        matches!(turn, Some((npc, _, _)) if npc == bar),
        "the sharp-eared bartender perceives the concealed speaker and wakes"
    );
}

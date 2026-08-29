//! M11 perception vertical, end to end through the real mushd module wiring
//! (built-ins + worldgen + the `npc` module grounded by the WoD adapter + the
//! seeded WoD ruleset), ungated and fully in-memory — the same harness shape as
//! `npc_e2e`, driven by hand so the off-loop NPC turn is deterministic.
//!
//! Proves the NPC-as-observer half of the vertical (design D5): a bartender NPC
//! reacts to a vampire's speech only when it can *perceive* the vampire. A
//! ghoul bartender with Auspex pierces an Obfuscate veil and takes its turn; a
//! mundane bartender with no Auspex is met with true silence — no stimulus, no
//! turn, no tokens.

use std::sync::Arc;

use mush_core::module::TickHandler;
use mush_core::{
    ConnId, Ctx, Effect, Effects, Engine, GOD, Inbound, ObjId, ObjKind, Outbox, ParsedArgs,
    SwitchSet, World,
};
use mush_llm::{
    BudgetConfig, LlmProvider, LlmService, MockProvider, RouteConfig,
};
use mush_proto::OutputEvent;
use mush_store::InMemoryStore;
use smol_str::SmolStr;

use mushd::modules;
use mushd::world_task::base_registry;

use mush_npc::AgentGrounding;

/// Fixed RNG seed for the WoD module → deterministic dice.
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
            modules::npc_agent_adapter(&["wod".to_string()]).expect("wod grounding adapter");
        mods.push(modules::npc_module(Some(grounding)));
        mods.push(Box::new(ruleset_wod::WodRuleset::with_seed(SEED)));

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

/// Make an existing player a vampire with `obfuscate` dots of Obfuscate.
fn make_vampire(rig: &mut Rig, name: &str, obfuscate: i32) {
    rig.staff(&format!("@sheet/set {name}/template=vampire"));
    rig.staff(&format!(
        "@sheet/set {name}/disciplines:Obfuscate={obfuscate}"
    ));
}

// ----------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------

#[tokio::test]
async fn a_mundane_bartender_never_notices_a_cloaked_vampire() {
    let svc = service();
    let mut rig = Rig::boot(&svc).await;
    let _bar = make_bartender(&mut rig, "mortal"); // no Auspex

    let vlad = rig.player("Vlad");
    make_vampire(&mut rig, "Vlad", 3);

    // Control: while in plain sight, the bartender wakes to Vlad's speech.
    rig.run(vlad, "say Evening.");
    rig.set_now(1_003);
    assert!(
        rig.one_turn().is_some(),
        "a visible speaker wakes the bartender (baseline)"
    );

    // Now Vlad cloaks and speaks: the mundane bartender neither hears nor wakes.
    rig.run(vlad, "+obfuscate on");
    rig.run(vlad, "say You never saw me.");
    rig.set_now(1_010);
    assert!(
        rig.one_turn().is_none(),
        "a mundane bartender is met with true silence — no stimulus, no turn"
    );
}

#[tokio::test]
async fn a_ghoul_bartender_with_auspex_pierces_the_veil() {
    let svc = service();
    let mut rig = Rig::boot(&svc).await;
    let bar = make_bartender(&mut rig, "ghoul");
    // Overwhelming senses: Perception + Awareness 5 and Auspex 5.
    rig.staff("@sheet/set Bartender/perception=5");
    rig.staff("@sheet/set Bartender/awareness=5");
    rig.staff("@sheet/set Bartender/disciplines:Auspex=5");

    let vlad = rig.player("Vlad");
    make_vampire(&mut rig, "Vlad", 2);

    // Vlad cloaks (the bartender rolls once, now) and speaks: Auspex pierces the
    // veil, so the bartender still takes its turn against the speaker it senses.
    rig.run(vlad, "+obfuscate on");
    rig.run(vlad, "say Do you sense me, ghoul?");
    rig.set_now(1_003);
    let turn = rig.one_turn();
    assert!(
        matches!(turn, Some((npc, _, _)) if npc == bar),
        "the Auspex ghoul bartender perceives the cloaked vampire and wakes"
    );
}

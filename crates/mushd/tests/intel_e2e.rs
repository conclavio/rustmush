//! M12 informant loop, end to end through the real mushd module wiring
//! (built-ins + worldgen + the `npc` module grounded by the WoD adapter + the
//! seeded WoD ruleset), ungated and fully in-memory — the same harness shape as
//! `perception_e2e`, driven by hand so the perception roll is deterministic.
//!
//! Proves the load-bearing correctness property (P1): an informant can report
//! ONLY what its NPC actually PERCEIVED. A ghoul bartender with Auspex, set to
//! report to a domitor, PIERCES a cloaked vampire's veil and files a `Concealed`
//! intel record — which the domitor pulls with `+intel`. A mundane bartender,
//! met with true silence, pierces nothing and files nothing, so its domitor's
//! `+intel` is empty.

use std::sync::Arc;

use mush_core::{
    ConnId, Effects, Engine, GOD, Inbound, ObjId, ObjKind, Outbox, World,
};
use mush_llm::{BudgetConfig, LlmProvider, LlmService, MockProvider, RouteConfig};
use mush_proto::OutputEvent;
use mush_store::InMemoryStore;

use mushd::modules;
use mushd::world_task::base_registry;

use mush_npc::AgentGrounding;

/// Fixed RNG seed for the WoD module → deterministic perception dice.
const SEED: u64 = 0xB0A7;

// ----------------------------------------------------------------------
// Harness (a trimmed copy of perception_e2e's Rig).
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

/// Create + configure a bartender NPC with a `template` sheet, reporting to
/// `principal` (the domitor it files intel for).
fn make_informant_bartender(rig: &mut Rig, template: &str, principal: &str) {
    let out = rig.staff("@agent/create Bartender");
    assert!(out.contains("Created NPC agent Bartender"), "{out}");
    rig.staff("@agent/set Bartender/persona=a watchful barkeep");
    rig.staff("@agent/set Bartender/tier=major");
    rig.staff("@agent/set Bartender/fallback=The bartender says nothing.");
    let out = rig.staff(&format!("@agent/sheet Bartender={template}"));
    assert!(out.contains("sheet"), "{out}");
    let out = rig.staff(&format!("@agent/set Bartender/principal={principal}"));
    assert!(out.contains("Set principal on Bartender"), "{out}");
}

/// Make an existing player a vampire with `obfuscate` dots of Obfuscate.
fn make_vampire(rig: &mut Rig, name: &str, obfuscate: i32) {
    rig.staff(&format!("@sheet/set {name}/template=vampire"));
    rig.staff(&format!("@sheet/set {name}/disciplines:Obfuscate={obfuscate}"));
}

// ----------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------

/// P1 (positive): a ghoul bartender with overwhelming senses pierces a cloaked
/// vampire's veil and FILES the pierced sighting; its domitor pulls it via
/// `+intel` and sees a `concealed` record about the vampire.
#[tokio::test]
async fn ghoul_informant_pierces_and_files_intel_the_domitor_pulls() {
    let svc = service();
    let mut rig = Rig::boot(&svc).await;
    let marcus = rig.player("Marcus"); // the domitor
    make_informant_bartender(&mut rig, "ghoul", "Marcus");
    // Overwhelming senses: Perception + Awareness 5 and Auspex 5.
    rig.staff("@sheet/set Bartender/perception=5");
    rig.staff("@sheet/set Bartender/awareness=5");
    rig.staff("@sheet/set Bartender/disciplines:Auspex=5");

    let vlad = rig.player("Vlad");
    make_vampire(&mut rig, "Vlad", 2);

    // Vlad cloaks (the bartender rolls once, now) and speaks: Auspex pierces the
    // veil, so the informant perceives him and files the notable pierce.
    rig.run(vlad, "+obfuscate on");
    rig.run(vlad, "say Do you sense me, ghoul?");

    // Marcus debriefs his asset: a concealed-pierce record about Vlad.
    let out = rig.actor_sys(marcus, "+intel");
    assert!(out.contains("Your intel log"), "{out}");
    assert!(out.contains("Vlad"), "the pierced vampire is reported: {out}");
    assert!(out.contains("[concealed]"), "filed as a pierced concealment: {out}");
    assert!(
        out.contains("trying to stay hidden"),
        "the notable detail is rendered: {out}"
    );
    assert!(
        out.contains("witnessed directly by Bartender"),
        "provenance recorded (P5): {out}"
    );

    // Filtering to Vlad shows the same record; an unknown subject is clean.
    let by_vlad = rig.actor_sys(marcus, "+intel Vlad");
    assert!(by_vlad.contains("Intel about Vlad"), "{by_vlad}");
    let none = rig.actor_sys(marcus, "+intel Nobody");
    assert!(none.contains("No intel on file about Nobody"), "{none}");
}

/// P1 (negative, the load-bearing property): a mundane bartender is met with
/// true silence by the cloaked vampire — it perceives nothing, so it files
/// nothing, and its domitor's `+intel` is empty.
#[tokio::test]
async fn mundane_informant_perceives_nothing_and_files_nothing() {
    let svc = service();
    let mut rig = Rig::boot(&svc).await;
    let marcus = rig.player("Marcus");
    make_informant_bartender(&mut rig, "mortal", "Marcus"); // no Auspex

    let vlad = rig.player("Vlad");
    make_vampire(&mut rig, "Vlad", 3);

    // Vlad cloaks and speaks: the mundane bartender neither hears nor sees him,
    // so nothing is ever filed (P1 — an informant reports only what it perceives).
    rig.run(vlad, "+obfuscate on");
    rig.run(vlad, "say You never saw me.");
    rig.set_now(1_010);

    let out = rig.actor_sys(marcus, "+intel");
    assert!(
        out.contains("no intel on file"),
        "a bartender that pierced nothing files nothing (P1): {out}"
    );
}

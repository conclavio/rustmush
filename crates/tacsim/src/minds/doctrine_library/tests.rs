use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use mush_core::module::{ModError, Registrar};
use mush_core::{
    Audience, Effects, Engine, GOD, GameModule, Inbound, LIMBO, ObjId, ObjKind, Outbox, Registry,
    World,
};
use mush_proto::OutputEvent;
use mush_roll::{RollError, RollModule, RollStore, WorkingCopy};

use crate::config::ArenaConfig;
use crate::minds::workshop::commands::WorkshopCommands;
use crate::minds::workshop::executor::{CheckPictures, PictureReport, WorkshopExecutor};

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct NoPictures(Arc<AtomicUsize>);
impl CheckPictures for NoPictures {
    fn run(&self, _: &WorkingCopy) -> Result<Vec<PictureReport>, RollError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(RollError::Missing)
    }
}
struct Workshop(Arc<WorkshopCommands>);
impl GameModule for Workshop {
    fn name(&self) -> &'static str {
        crate::store::NS
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        // Match the real Tacsim module; @mind also owns authoring commands.
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        self.0.register(reg)
    }
}

fn rig(cfg: ReflexConfig) -> Result<(Engine, ObjId, Arc<AtomicUsize>), Box<dyn std::error::Error>> {
    let calls = Arc::new(AtomicUsize::new(0));
    let commands = Arc::new(WorkshopCommands::new(
        WorkshopExecutor::new(Arc::new(NoPictures(calls.clone())))?,
        ArenaConfig {
            reflex: cfg,
            ..ArenaConfig::default()
        },
        0,
    ));
    let mut world = World::new_minimal();
    let actor = world.create(ObjKind::Player, "Reader", GOD, Some(LIMBO))?;
    let mut engine = Engine::with_modules(
        world,
        Registry::new(),
        vec![
            Box::new(RollModule::with_writers([crate::store::NS])),
            Box::new(Workshop(commands)),
        ],
    )?;
    engine.step(
        Inbound::ConnUp {
            conn: 7,
            actor,
            caps_structured: true,
            from_linkdead: false,
        },
        &mut Outbox::new(),
        &mut Effects::new(),
    );
    engine.world.take_dirty();
    check!(engine.take_module_ops().is_empty());
    Ok((engine, actor, calls))
}

fn read(engine: &mut Engine, actor: ObjId, command: &str) -> String {
    let mut out = Outbox::new();
    let mut effects = Effects::new();
    engine.step(
        Inbound::Cmd {
            conn: 7,
            actor,
            line: command.to_owned(),
        },
        &mut out,
        &mut effects,
    );
    assert!(
        effects.is_empty(),
        "teaching read must not queue worker/store work"
    );
    assert!(
        engine.take_module_ops().is_empty(),
        "teaching read must not publish"
    );
    let mut lines = Vec::new();
    for (audience, event) in out.events() {
        if let OutputEvent::Sys(text) = event {
            assert_eq!(*audience, Audience::Actor(actor));
            lines.push(text.to_plain_text());
        }
    }
    assert!(!lines.is_empty(), "actual command must produce a reply");
    lines.join("\n")
}

#[test]
fn every_documented_preset_is_a_bounded_actual_card_page() -> TestResult {
    let cfg = ReflexConfig::default();
    let index = page("", &cfg)?;
    let documented = [
        "Line",
        "Sentry",
        "Sniper",
        "Skirmisher",
        "Hunter",
        "Rearguard",
    ];
    check_eq!(PRESETS, documented);
    for name in documented {
        check!(index.contains(&format!("  {name}:")));
        let text = page(&format!(" {} ", name.to_ascii_lowercase()), &cfg)?;
        check!(text.contains(&format!("Standard Doctrine / {name}")));
        check!(text.len() <= MAX_PAGE_BYTES);
        for line in preset(name, &cfg)
            .ok_or("missing stock preset")?
            .read_back(&cfg, true)
        {
            check!(text.contains(&line.replace("[M2]", "[inert]")));
        }
        check!(!text.contains("[M2]"));
        check!(text.contains("unarmed hull"));
    }
    check!(page("Skirmisher", &cfg)?.contains("does not make it orbit"));
    check!(page("Sentry", &cfg)?.contains("holds no arc"));
    Ok(())
}

#[test]
fn actual_engine_uses_current_configuration_without_business_or_world_mutation() -> TestResult {
    let cfg = ReflexConfig {
        engage_band_permille: 850,
        withdraw_permille: 410,
        stand_min_permille: 205,
        rally_ms: 7_000,
        search_ms: 27_000,
        hunt_after_ms: 31_000,
        weight_range: 70,
        weight_facet: 55,
        weight_health: 25,
        ..ReflexConfig::default()
    };
    let (mut engine, actor, calls) = rig(cfg)?;
    check!(
        engine
            .world
            .module_state::<mush_sim::AccountOracle>()
            .is_none()
    );
    check!(
        engine
            .world
            .module_state::<crate::store::TacsimStore>()
            .is_none()
    );
    let before = serde_json::to_vec(&engine.world.iter().collect::<Vec<_>>())?;
    let allocator = engine.world.next_id();
    let typed = (
        engine.action_state().revision(),
        engine.action_state().len(),
    );
    let prepared = engine
        .world
        .module_state::<RollStore>()
        .ok_or("roll")?
        .prepare_create(
            actor,
            "unpublished",
            crate::minds::starter::standard_draft()?,
        )?;
    let line = read(&mut engine, actor, "@mind/doctrine LiNe");
    for expected in [
        "band 85%",
        "break_off 41%",
        "fight_home 20.5%",
        "rally 7 s",
        "search 27 s",
        "hunt_after 31 s",
        "(70/55/25)",
    ] {
        check!(
            line.contains(expected),
            "configured term {expected} must be visible"
        );
    }
    let sniper = read(&mut engine, actor, "@mind/doctrine Sniper");
    check!(sniper.contains("band 100%"));
    check!(sniper.contains("reserve 30%"));
    check!(sniper.contains("break_off 41%"));
    check!(sniper.contains("rally 7 s"));
    let index = read(&mut engine, actor, "@mind/doctrine");
    check!(index.contains("Six free public pages"));
    for preset in PRESETS {
        check!(
            read(&mut engine, actor, &format!("@mind/doctrine {preset}"))
                .contains(&format!("Standard Doctrine / {preset}"))
        );
    }
    check_eq!(
        serde_json::to_vec(&engine.world.iter().collect::<Vec<_>>())?,
        before
    );
    check_eq!(engine.world.next_id(), allocator);
    check!(engine.world.take_dirty().is_empty());
    check_eq!(
        (
            engine.action_state().revision(),
            engine.action_state().len()
        ),
        typed
    );
    let roll = engine.world.module_state::<RollStore>().ok_or("roll")?;
    roll.check_prepared(&prepared)?;
    check!(roll.names(actor).is_empty());
    check_eq!(calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn selectors_are_closed_and_never_fall_back_to_authored_or_filed_content() -> TestResult {
    let cfg = ReflexConfig::default();
    for bad in [
        "Standard",
        "Private",
        "#42",
        "paid:1",
        "Line@2",
        "Line/secret",
        "Line Sniper",
        "Line\n",
        "Line\0",
        "Líne",
    ] {
        check!(page(bad, &cfg).is_err());
    }
    check!(page(&" ".repeat(MAX_SELECTOR_BYTES + 1), &cfg).is_err());
    let (mut engine, actor, calls) = rig(cfg)?;
    for selector in [
        "Private",
        "#42",
        "paid:1",
        "Line@2",
        "Line Sniper",
        &"a".repeat(MAX_SELECTOR_BYTES + 1),
    ] {
        let text = read(&mut engine, actor, &format!("@mind/doctrine {selector}"));
        check!(text.contains(SELECT));
        check!(!text.contains("Current configured card"));
    }
    let text = read(&mut engine, actor, "@mind/doctrine/source Line");
    check!(text.contains("@mind/doctrine"));
    check!(!text.contains("Current configured card"));
    check_eq!(calls.load(Ordering::SeqCst), 0);
    check!(
        engine
            .world
            .module_state::<RollStore>()
            .ok_or("roll")?
            .names(actor)
            .is_empty()
    );
    Ok(())
}

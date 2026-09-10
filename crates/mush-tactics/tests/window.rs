//! S4: `+btest` opens a real window over the lot (plan §3.5), composited by the
//! atlas off-loop; the pilot stands on it once the map is drawn, and the window
//! closes with the battle.

use mush_atlas::{AtlasModule, AtlasState, starter};
use mush_content::{Appliers, ContentModule};
use mush_core::{
    Audience, Effect, Effects, GameModule, Inbound, ObjId, ObjKind, Outbox, WorldCommand,
};
use mush_proto::OutputEvent;
use mush_sim::SimulationModule;
use mush_tactics::TacticsStub;
use mush_testkit::TestWorld;

fn lot() -> TestWorld {
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
    let content = ContentModule::new()
        .with_appliers(appliers)
        .with_boot_packs(starter::packs().expect("the shipped packs load"));
    let modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(TacticsStub::new()),
        Box::new(mush_npc::NpcModule::new()),
        Box::new(SimulationModule::new()),
        Box::new(AtlasModule::new().with_shelf(starter::packs().expect("shelf"))),
        Box::new(content),
    ];
    TestWorld::with_modules(modules)
}

fn room_named(tw: &TestWorld, name: &str) -> ObjId {
    tw.world()
        .iter()
        .find(|o| o.kind == ObjKind::Room && o.name == name)
        .map(|o| o.id)
        .unwrap_or_else(|| panic!("no room named {name}"))
}

fn text(tw: &TestWorld, events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    tw.seen_by(events, who).join("\n")
}

fn finish(tw: &mut TestWorld, actor: ObjId, effects: &[Effect]) -> Vec<(Audience, OutputEvent)> {
    let (correlation, payload) = effects
        .iter()
        .find_map(|e| match e {
            Effect::ModuleAsync {
                module,
                correlation,
                payload,
                ..
            } if module.as_str() == mush_atlas::NS => Some((*correlation, payload.clone())),
            _ => None,
        })
        .expect("the atlas deferred a composite");
    let outcome = mush_atlas::run_async(&payload);
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    tw.engine_mut().step(
        Inbound::Apply(WorldCommand::ModuleAsyncDone {
            module: mush_atlas::NS.into(),
            actor,
            correlation,
            payload: outcome,
        }),
        &mut out,
        &mut fx,
    );
    out.take()
}

#[test]
fn btest_opens_a_window_over_the_pilots_room_and_closes_it_with_the_battle() {
    let mut tw = lot();
    let pilot = tw.spawn_player("Natasha");
    let coffee = room_named(&tw, "Coffee Shop");
    tw.world_mut()
        .move_to(pilot, coffee)
        .expect("to the coffee shop");
    let (events, effects) = tw.run_with_effects(pilot, "+btest/start");
    let t = text(&tw, &events, pilot);
    assert!(t.contains("BTEST: battle"), "{t}");
    assert!(!t.contains("no map here"), "{t}");
    let window = tw
        .world()
        .module_state::<AtlasState>()
        .and_then(|st| st.windows.open.values().next().cloned())
        .expect("a window opened");
    assert!(window.rooms.contains(&coffee));
    assert!(window.composite.is_none(), "not drawn yet");
    let done = finish(&mut tw, pilot, &effects);
    let t = text(&tw, &done, pilot);
    assert!(t.contains("The map is drawn"), "{t}");
    let (_, pose) = mush_sim::pose_of(tw.world(), pilot).expect("the pilot stands on the map");
    assert!(pose.x > 312.0 && pose.x < 334.0, "{pose:?}");
    assert_eq!(tw.world().get(pilot).and_then(|o| o.location), Some(coffee));
    // The battle ends: the window closes, the pilot stays where they stand.
    let events = tw.run(pilot, "+btest/stop");
    let t = text(&tw, &events, pilot);
    assert!(t.contains("BTEST: battle") && t.contains("ended"), "{t}");
    assert!(
        tw.world()
            .module_state::<AtlasState>()
            .is_some_and(|st| st.windows.open.is_empty()),
        "the window closed with the battle"
    );
    assert!(mush_sim::pose_of(tw.world(), pilot).is_none());
    assert_eq!(tw.world().get(pilot).and_then(|o| o.location), Some(coffee));
}

#[test]
fn btest_without_geometry_is_a_battle_without_a_map() {
    let mut tw = lot();
    let pilot = tw.spawn_player("Natasha"); // in Limbo, which has no geometry
    let events = tw.run(pilot, "+btest/start");
    let t = text(&tw, &events, pilot);
    assert!(t.contains("BTEST: battle"), "{t}");
    assert!(t.contains("no map here"), "{t}");
    assert!(
        tw.world()
            .module_state::<AtlasState>()
            .is_some_and(|st| st.windows.open.is_empty())
    );
}

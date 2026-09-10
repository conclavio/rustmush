//! S7 review regression: NPC occupants belong in ground engagements.
//! This asserts the expected behavior and fails at d713d17.
//! See docs/design/14a-spatial-s1-s7-code-review.md.

use mush_atlas::{AtlasModule, starter};
use mush_content::{Appliers, ContentModule, parse_pack};
use mush_core::{
    Audience, Effect, Effects, GOD, GameModule, Inbound, ObjId, ObjKind, Outbox, WorldCommand,
};
use mush_proto::OutputEvent;
use mush_sim::SimulationModule;
use mush_tactics::TacticsModule;
use mush_tactics::ground::GROUND_DRIVER;
use mush_testkit::TestWorld;

fn modules(extra_packs: Vec<String>) -> Vec<Box<dyn GameModule>> {
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
    let mut packs = starter::packs().expect("the shipped packs load");
    for p in extra_packs {
        packs.push(parse_pack(&p).expect("an extra pack parses"));
    }
    let content = ContentModule::new()
        .with_appliers(appliers)
        .with_boot_packs(packs);
    vec![
        Box::new(TacticsModule::new()),
        Box::new(mush_npc::NpcModule::new()),
        Box::new(SimulationModule::new()),
        Box::new(AtlasModule::new().with_shelf(starter::packs().expect("shelf"))),
        Box::new(content),
    ]
}

fn lot() -> TestWorld {
    TestWorld::with_modules(modules(Vec::new()))
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

/// Run the composite the atlas deferred, so the window has its cells.
fn finish(tw: &mut TestWorld, actor: ObjId, effects: &[Effect]) {
    let Some((correlation, payload)) = effects.iter().find_map(|e| match e {
        Effect::ModuleAsync {
            module,
            correlation,
            payload,
            ..
        } if module.as_str() == mush_atlas::NS => Some((*correlation, payload.clone())),
        _ => None,
    }) else {
        panic!("the atlas deferred no composite");
    };
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
}

#[test]
fn review_ground_window_must_include_npc_occupants() {
    let mut tw = lot();
    let backlot = room_named(&tw, "Backlot");
    let pilot = tw.spawn_player("Pilot");
    tw.world_mut().move_to(pilot, backlot).unwrap();
    let npc = tw
        .world_mut()
        .create(ObjKind::Npc, "Opponent", GOD, Some(backlot))
        .unwrap();
    tw.run(GOD, &format!("@behavior/add #{}=sim.domain", backlot.0));
    let (events, effects) = tw.run_with_effects(
        GOD,
        &format!("@domain/create #{}={GROUND_DRIVER}", backlot.0),
    );
    assert!(text(&tw, &events, GOD).contains("Created domain"));
    finish(&mut tw, GOD, &effects);
    assert!(
        mush_sim::pose_of(tw.world(), pilot).is_some(),
        "player placed"
    );
    assert!(
        mush_sim::pose_of(tw.world(), npc).is_some(),
        "NPC in the same room was omitted from the engagement"
    );
}

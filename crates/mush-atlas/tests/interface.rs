//! S6 (doc 14 §4.6): the receptionist's and the bus-watcher's interfaces over
//! the lot, invoked as a player and as staff — the A lens (authority is the
//! asker's), the M lens (a read writes nothing), and the answers themselves.

use mush_atlas::{AtlasModule, AtlasState, starter};
use mush_content::{Appliers, ContentModule};
use mush_core::{
    AbilityCall, Ctx, Flags, GOD, GameModule, InterfaceError, InterfaceReply, ObjId, ObjKind,
    Outbox, ParsedArgs, SwitchSet,
};
use mush_sim::SimulationModule;
use mush_testkit::TestWorld;
use smol_str::SmolStr;

fn modules() -> Vec<Box<dyn GameModule>> {
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
    let content = ContentModule::new()
        .with_appliers(appliers)
        .with_boot_packs(starter::packs().expect("the shipped packs load"));
    vec![
        Box::new(mush_npc::NpcModule::new()),
        Box::new(SimulationModule::new()),
        Box::new(AtlasModule::new().with_shelf(starter::packs().expect("shelf"))),
        Box::new(content),
    ]
}

fn lot() -> TestWorld {
    TestWorld::with_modules(modules())
}

fn named(tw: &TestWorld, kind: ObjKind, name: &str) -> ObjId {
    tw.world()
        .iter()
        .find(|o| o.kind == kind && o.name == name)
        .map(|o| o.id)
        .unwrap_or_else(|| panic!("no {kind:?} named {name}"))
}

/// Invoke one interface ability the way the NPC runtime does: in a command
/// context whose actor is the asker.
fn invoke(
    tw: &mut TestWorld,
    provider: &str,
    npc: ObjId,
    asker: ObjId,
    ability: &str,
    input: serde_json::Value,
) -> Result<InterfaceReply, InterfaceError> {
    let engine = tw.engine_mut();
    let provider = engine
        .registry
        .interface_provider(provider)
        .unwrap_or_else(|| panic!("no provider {provider}"));
    let mut out = Outbox::new();
    let mut ctx = Ctx {
        w: &mut engine.world,
        actor: asker,
        switches: SwitchSet::default(),
        args: ParsedArgs::default(),
        out: &mut out,
        reg: &engine.registry,
        module: Some(SmolStr::new("npc")),
        typed_state: mush_core::ModuleActionState::unavailable(),
    };
    provider.invoke(&mut ctx, AbilityCall { npc, asker }, ability, &input)
}

#[test]
fn the_packs_name_the_providers_the_server_composes() {
    let tw = lot();
    let susie = named(&tw, ObjKind::Npc, "Susie");
    let profile = tw
        .world()
        .attr(susie, mush_npc::PROFILE_ATTR)
        .and_then(|a| a.as_json())
        .map(|v| serde_json::from_value::<mush_npc::AgentProfile>(v.clone()).expect("profile"))
        .expect("Susie's profile");
    assert_eq!(profile.interfaces, vec![SmolStr::new("atlas")]);
    for id in ["atlas", "travel", "help"] {
        assert!(
            tw.registry().interface_provider(id).is_some(),
            "{id} is composed"
        );
    }
    // Every id the packs name is one the full server composes.
    let composed = [
        "atlas", "weather", "travel", "market", "build", "rules", "request", "help",
    ];
    for o in tw.world().iter().filter(|o| o.kind == ObjKind::Npc) {
        if let Some(p) = tw
            .world()
            .attr(o.id, mush_npc::PROFILE_ATTR)
            .and_then(|a| a.as_json())
            .and_then(|v| serde_json::from_value::<mush_npc::AgentProfile>(v.clone()).ok())
        {
            for id in &p.interfaces {
                assert!(composed.contains(&id.as_str()), "{}: {id}", o.name);
            }
        }
    }
    let state = tw.world().module_state::<AtlasState>().expect("atlas");
    assert!(!state.records.neighborhoods.is_empty());
}

#[test]
fn the_receptionist_describes_the_lot_with_the_askers_eyes() {
    let mut tw = lot();
    let susie = named(&tw, ObjKind::Npc, "Susie");
    let lobby = named(&tw, ObjKind::Room, "Lobby");
    let rae = tw.spawn_player("Rae");
    tw.world_mut()
        .move_to(rae, lobby)
        .expect("Rae to the lobby");
    tw.world_mut()
        .move_to(GOD, lobby)
        .expect("staff to the lobby");
    // A player: the restricted Production Office is not in the answer.
    let reply = invoke(
        &mut tw,
        "atlas",
        susie,
        rae,
        "atlas.describe",
        serde_json::json!({}),
    )
    .expect("an answer");
    assert!(reply.text.contains("Coffee Shop"), "{}", reply.text);
    assert!(
        !reply.text.contains("Production Office"),
        "hidden from a player:\n{}",
        reply.text
    );
    // Staff, from the same lore and the same NPC: it is there.
    let reply = invoke(
        &mut tw,
        "atlas",
        susie,
        GOD,
        "atlas.describe",
        serde_json::json!({}),
    )
    .expect("an answer");
    assert!(
        reply.text.contains("Production Office"),
        "shown to staff:\n{}",
        reply.text
    );
    // A place the player may not name is refused in +atlas's words.
    let err = invoke(
        &mut tw,
        "atlas",
        susie,
        rae,
        "atlas.describe",
        serde_json::json!({ "place": "Atlantis" }),
    )
    .expect_err("no such place");
    assert!(matches!(err, InterfaceError::Refused(m) if m.contains("Atlantis")));
    // An unknown ability is unknown.
    assert!(matches!(
        invoke(
            &mut tw,
            "atlas",
            susie,
            rae,
            "atlas.burn",
            serde_json::json!({})
        ),
        Err(InterfaceError::Unknown(_))
    ));
}

#[test]
fn who_nearby_is_the_room_as_the_asker_sees_it() {
    let mut tw = lot();
    let susie = named(&tw, ObjKind::Npc, "Susie");
    let lobby = named(&tw, ObjKind::Room, "Lobby");
    let rae = tw.spawn_player("Rae");
    let kit = tw.spawn_player("Kit");
    let ghost = tw.spawn_player("Ghost");
    for p in [rae, kit, ghost] {
        tw.world_mut().move_to(p, lobby).expect("to the lobby");
    }
    tw.world_mut()
        .set_flag(ghost, Flags::DARK, true)
        .expect("dark");
    let reply = invoke(
        &mut tw,
        "atlas",
        susie,
        rae,
        "who.nearby",
        serde_json::json!({}),
    )
    .expect("an answer");
    assert!(reply.text.contains("Kit"), "{}", reply.text);
    assert!(
        reply.text.contains("Susie"),
        "the NPC herself is present: {}",
        reply.text
    );
    assert!(
        !reply.text.contains("Ghost"),
        "a DARK player is absent: {}",
        reply.text
    );
    assert!(
        !reply.text.contains("Rae"),
        "the asker is not listed to themself: {}",
        reply.text
    );
}

#[test]
fn the_bus_watcher_knows_where_the_bus_is_and_the_askers_own_journey() {
    let mut tw = lot();
    let regina = named(&tw, ObjKind::Npc, "Regina");
    let forecourt = named(&tw, ObjKind::Room, "Forecourt & Bus Stop");
    let rae = tw.spawn_player("Rae");
    tw.world_mut()
        .move_to(rae, forecourt)
        .expect("Rae to the forecourt");
    // Before the first tick nothing has left its stop; the answer says so.
    let reply = invoke(
        &mut tw,
        "travel",
        regina,
        rae,
        "travel.next",
        serde_json::json!({}),
    )
    .expect("an answer");
    assert!(
        reply.text.contains("Nothing is on the road"),
        "{}",
        reply.text
    );
    // The bus departs on its schedule.
    for t in [30i64, 60, 120, 240] {
        tw.set_now(t);
        let _ = tw.fire_due(t * 1000);
    }
    let reply = invoke(
        &mut tw,
        "travel",
        regina,
        rae,
        "travel.next",
        serde_json::json!({}),
    )
    .expect("an answer");
    assert!(reply.text.contains("Studio Tour Bus"), "{}", reply.text);
    let reply = invoke(
        &mut tw,
        "travel",
        regina,
        rae,
        "travel.status",
        serde_json::json!({}),
    )
    .expect("an answer");
    assert_eq!(reply.text, "You are not travelling.");
    let reply = invoke(
        &mut tw,
        "travel",
        regina,
        rae,
        "travel.next",
        serde_json::json!({ "carrier": "zeppelin" }),
    )
    .expect("an answer");
    assert!(reply.text.contains("Nothing called"), "{}", reply.text);
}

#[test]
fn a_read_writes_nothing() {
    let mut tw = lot();
    let susie = named(&tw, ObjKind::Npc, "Susie");
    let regina = named(&tw, ObjKind::Npc, "Regina");
    let lobby = named(&tw, ObjKind::Room, "Lobby");
    let rae = tw.spawn_player("Rae");
    tw.world_mut()
        .move_to(rae, lobby)
        .expect("Rae to the lobby");
    let _ = tw.engine_mut().take_module_ops();
    let before = tw.world().iter().count();
    invoke(
        &mut tw,
        "atlas",
        susie,
        rae,
        "atlas.describe",
        serde_json::json!({}),
    )
    .expect("describe");
    invoke(
        &mut tw,
        "atlas",
        susie,
        rae,
        "who.nearby",
        serde_json::json!({}),
    )
    .expect("who");
    invoke(
        &mut tw,
        "travel",
        regina,
        rae,
        "travel.next",
        serde_json::json!({}),
    )
    .expect("next");
    invoke(
        &mut tw,
        "travel",
        regina,
        rae,
        "travel.status",
        serde_json::json!({}),
    )
    .expect("status");
    let ops = tw.engine_mut().take_module_ops();
    assert!(ops.is_empty(), "a read persisted nothing: {ops:?}");
    assert_eq!(tw.world().iter().count(), before, "a read made nothing");
}

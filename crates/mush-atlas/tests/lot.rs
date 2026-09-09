//! The starter area, applied: the lot boots whole from three static packs and zero
//! model calls, reads back through every record's wire, refuses to be torn down
//! while someone is standing in it, and draws within eighty columns. Plan §3.1's
//! *Latch*, *Boot* and *Views* tests, with the *Geometry* refusals in `geometry.rs`.

use std::collections::HashMap;

use mush_atlas::{AtlasModule, AtlasState, SurfaceRef, starter};
use mush_content::{Appliers, ContentModule, ContentState};
use mush_core::{
    Audience, CanonicalId, Effects, Engine, GOD, GameModule, LIMBO, ModuleBootData, ModuleWrite,
    ObjKind, Outbox,
};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, boot_world, text_of};
use smol_str::SmolStr;

/// The production shape of the three modules the lot needs: the NPC host, the
/// atlas, and the content module last with the starter packs as boot packs.
fn lot_modules(starter_area: bool) -> Vec<Box<dyn GameModule>> {
    let mut appliers = Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
    let mut content = ContentModule::new().with_appliers(appliers);
    if starter_area {
        content = content.with_boot_packs(starter::packs().expect("the shipped packs load"));
    }
    vec![
        Box::new(mush_npc::NpcModule::new()),
        Box::new(AtlasModule::new().with_shelf(starter::packs().expect("shelf"))),
        Box::new(content),
    ]
}

fn lot() -> TestWorld {
    TestWorld::with_modules(lot_modules(true))
}

fn texts(tw: &TestWorld, events: &[(Audience, OutputEvent)], who: mush_core::ObjId) -> String {
    tw.seen_by(events, who).join("\n")
}

/// Live objects: a destroyed object stays a `GOING` husk until the world task's
/// sweep, so a count that means "what is there" excludes them.
fn live(tw: &TestWorld) -> impl Iterator<Item = &mush_core::Obj> {
    tw.world()
        .iter()
        .filter(|o| !o.flags.contains(mush_core::Flags::GOING))
}

fn rooms(tw: &TestWorld) -> usize {
    live(tw).filter(|o| o.kind == ObjKind::Room).count()
}

fn objects(tw: &TestWorld) -> usize {
    live(tw).count()
}

fn room_named(tw: &TestWorld, name: &str) -> mush_core::ObjId {
    tw.world()
        .iter()
        .find(|o| o.kind == ObjKind::Room && o.name == name)
        .unwrap_or_else(|| panic!("no room named {name}"))
        .id
}

fn npc_named<'a>(tw: &'a TestWorld, name: &str) -> &'a mush_core::Obj {
    tw.world()
        .iter()
        .find(|o| o.kind == ObjKind::Npc && o.name == name)
        .unwrap_or_else(|| panic!("no NPC named {name}"))
}

// ---- boot --------------------------------------------------------------------

#[test]
fn boot_world_stays_minimal_and_the_starter_makes_the_lot() {
    let (world, _) = boot_world();
    assert_eq!(world.iter().count(), 2, "Limbo and God, nothing else");
    let bare = TestWorld::with_modules(lot_modules(false));
    assert_eq!(
        objects(&bare),
        2,
        "the content module with no boot packs grows nothing"
    );

    let tw = lot();
    assert_eq!(
        rooms(&tw),
        starter::LOT_ROOMS + 2,
        "fourteen lot rooms, the bus interior, and Limbo"
    );
    let limbo = tw.world().get(LIMBO).expect("limbo");
    let names: Vec<&str> = limbo
        .exits
        .iter()
        .filter_map(|e| tw.world().get(*e).map(|o| o.name.as_str()))
        .collect();
    assert_eq!(names, ["OOC Area <O>", "Staff <S>"], "the two exits on #0");
    // The cast, at their posts.
    for (who, room) in [
        ("Susie", "Lobby"),
        ("Ron Duvall", "Lobby"),
        ("Phil", "Breakroom"),
        ("Gretchen", "Souvenir Shop"),
        ("Karen Smith", "Backlot"),
        ("Regina", "Forecourt & Bus Stop"),
        ("Cady Heron", "Coffee Shop"),
        ("Janis Ian", "Writers' Room"),
        ("Aaron Samuels", "Studio Tour Bus"),
        ("Sharon Norbury", "Welcome Hall"),
    ] {
        let npc = npc_named(&tw, who);
        assert_eq!(
            npc.location,
            Some(room_named(&tw, room)),
            "{who} stands in {room}"
        );
        assert!(
            npc.attrs.get(mush_npc::PROFILE_ATTR).is_some(),
            "{who} carries a profile"
        );
    }
    let content = tw.world().module_state::<ContentState>().expect("content");
    for id in [
        starter::REGION_PACK,
        starter::OOC_AREA_PACK,
        starter::STAFF_LOBBY_PACK,
    ] {
        let latch = content
            .latches
            .applied(&CanonicalId::new(id).expect("id"))
            .unwrap_or_else(|| panic!("{id} is latched"));
        assert_eq!(latch.anomaly(), None, "{id} applied whole");
        assert!(!latch.is_retired());
    }
    let atlas = tw.world().module_state::<AtlasState>().expect("atlas");
    assert_eq!(atlas.records.regions.len(), 1);
    assert_eq!(atlas.records.neighborhoods.len(), 2);
    assert_eq!(atlas.records.roomgeo.len(), starter::LOT_ROOMS + 1);
    assert!(
        atlas.records.tiles.values().any(|t| matches!(
            &t.surface,
            SurfaceRef::Floor { structure, index: 1 } if structure.entry.as_str() == "structure.lobby"
        )),
        "the mezzanine has its own tile"
    );
    assert!(
        atlas
            .records
            .tiles
            .values()
            .any(|t| t.owner == mush_atlas::record::TileOwner::Region),
        "the region has its coarse ground tile"
    );
    assert_eq!(atlas.records.secrets.len(), 1);
    assert_eq!(atlas.records.connections.len(), 1);
}

/// P9: a first boot defers no effect — nothing leaves the world task, so nothing
/// can reach a provider. Driven on the `Engine` directly so the effects are visible.
#[test]
fn first_boot_defers_no_effect() {
    let (world, registry) = boot_world();
    let mut engine = Engine::with_modules(world, registry, lot_modules(true)).expect("composes");
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine
        .modules_loaded(HashMap::new(), &mut out, &mut fx)
        .expect("boots");
    assert!(fx.is_empty(), "boot deferred effects: {fx:?}");
    assert_eq!(
        engine
            .world
            .iter()
            .filter(|o| o.kind == ObjKind::Room && !o.flags.contains(mush_core::Flags::GOING))
            .count(),
        starter::LOT_ROOMS + 2
    );
}

// ---- the latch ---------------------------------------------------------------

#[test]
fn a_reboot_from_the_persisted_documents_applies_nothing_and_keeps_every_record() {
    let mut tw = lot();
    let before = objects(&tw);
    let ops = tw.take_module_ops();
    assert!(
        ops.iter().any(|op| op.ns == "content"),
        "latches were written"
    );
    assert!(
        ops.iter().any(|op| op.ns == "atlas"),
        "records were written"
    );
    assert!(
        ops.iter().any(|op| op.ns == "npc"),
        "the shared lore pool was written under the NPC module"
    );
    // Fold the write batch into per-namespace boot data, as the store would.
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
    let record_count = tw
        .world()
        .module_state::<AtlasState>()
        .expect("atlas")
        .records
        .len();
    tw.modules_loaded(boot);
    assert_eq!(objects(&tw), before, "a reboot applies nothing (P8)");
    let atlas = tw.world().module_state::<AtlasState>().expect("atlas");
    assert_eq!(
        atlas.records.len(),
        record_count,
        "every record came back through its wire"
    );
    let ops = tw.take_module_ops();
    assert!(
        !ops.iter()
            .any(|op| op.ns == "atlas" && matches!(op.op, ModuleWrite::Del { .. })),
        "no record was dropped at boot"
    );
}

#[test]
fn plan_is_empty_after_apply_and_apply_refuses_a_noop() {
    let mut tw = lot();
    let events = tw.run(GOD, "@atlas/pack plan neighborhood.staff_lobby");
    let said = texts(&tw, &events, GOD);
    assert!(said.contains("0 create, 0 update, 0 retire"), "{said}");
    assert!(said.contains("Nothing to do."), "{said}");
    let before = objects(&tw);
    let events = tw.run(
        GOD,
        "@atlas/pack apply neighborhood.staff_lobby 0000000000000000",
    );
    let said = texts(&tw, &events, GOD);
    assert!(said.contains("not the plan"), "{said}");
    assert_eq!(objects(&tw), before);
}

#[test]
fn retire_is_refused_while_occupied_and_then_removes_exactly_the_pack() {
    let mut tw = lot();
    let alice = tw.spawn_player("Alice");
    tw.run(alice, "s");
    assert_eq!(
        tw.world().get(alice).and_then(|o| o.location),
        Some(room_named(&tw, "Forecourt & Bus Stop"))
    );
    let before = objects(&tw);
    let events = tw.run(GOD, "@atlas/pack retire neighborhood.staff_lobby");
    let said = texts(&tw, &events, GOD);
    assert!(said.contains("occupied by Alice"), "{said}");
    assert_eq!(objects(&tw), before, "a refused retire touches nothing");

    // The OOC area cannot go while the lobby's bus exits lead into its bus.
    let events = tw.run(GOD, "@atlas/pack retire neighborhood.ooc_area");
    let said = texts(&tw, &events, GOD);
    assert!(said.contains("leads into"), "{said}");

    tw.run(alice, "x");
    assert_eq!(tw.world().get(alice).and_then(|o| o.location), Some(LIMBO));
    let events = tw.run(
        GOD,
        "@atlas/pack retire neighborhood.staff_lobby the lot is being rebuilt",
    );
    let said = texts(&tw, &events, GOD);
    assert!(said.contains("Retired neighborhood.staff_lobby"), "{said}");
    assert_eq!(
        rooms(&tw),
        starter::LOT_ROOMS + 2 - 8,
        "the lobby's eight rooms are gone"
    );
    let limbo = tw.world().get(LIMBO).expect("limbo");
    assert_eq!(
        limbo.exits.len(),
        1,
        "the <S> exit went with its pack; <O> stayed"
    );
    let atlas = tw.world().module_state::<AtlasState>().expect("atlas");
    assert!(
        atlas
            .records
            .keys_of_pack(&CanonicalId::new(starter::STAFF_LOBBY_PACK).expect("id"))
            .is_empty(),
        "no record of the pack remains"
    );
    assert_eq!(atlas.records.neighborhoods.len(), 1);
    assert!(atlas.records.regions.len() == 1, "the region stayed");
    let content = tw.world().module_state::<ContentState>().expect("content");
    assert!(
        content
            .latches
            .applied(&CanonicalId::new(starter::STAFF_LOBBY_PACK).expect("id"))
            .is_some_and(|l| l.is_retired()),
        "the latch is kept, marked retired"
    );
    // A retired pack plans as a fresh apply and comes back whole.
    let events = tw.run(GOD, "@atlas/pack plan neighborhood.staff_lobby");
    let said = texts(&tw, &events, GOD);
    let fingerprint = said
        .lines()
        .find_map(|l| l.trim().strip_prefix("fingerprint "))
        .and_then(|rest| rest.split_whitespace().next())
        .expect("a fingerprint")
        .to_string();
    let events = tw.run(GOD, &format!("@atlas/pack apply {fingerprint}"));
    let said = texts(&tw, &events, GOD);
    assert!(said.contains("Applied neighborhood.staff_lobby"), "{said}");
    assert!(!said.contains("STOPPED"), "{said}");
    assert_eq!(rooms(&tw), starter::LOT_ROOMS + 2);
}

#[test]
fn the_staff_lock_holds_and_the_map_hides_the_office_from_a_player() {
    let mut tw = lot();
    let alice = tw.spawn_player("Alice");
    tw.run(alice, "s");
    tw.run(alice, "l");
    assert_eq!(
        tw.world().get(alice).and_then(|o| o.location),
        Some(room_named(&tw, "Lobby"))
    );
    let events = tw.run(alice, "up");
    let said = texts(&tw, &events, alice);
    assert!(
        said.contains("can't go that way")
            || said.contains("STAFF ONLY")
            || said.contains("Permission"),
        "the stair is locked to staff: {said}"
    );
    assert_eq!(
        tw.world().get(alice).and_then(|o| o.location),
        Some(room_named(&tw, "Lobby"))
    );
    let events = tw.run(alice, "+atlas");
    let said = texts(&tw, &events, alice);
    assert!(said.contains("Lobby"), "{said}");
    assert!(said.contains("Susie"), "the receptionist is listed: {said}");
    assert!(
        !said.contains("Production Office"),
        "absent, not blanked: {said}"
    );
    assert!(said.contains("(you)"), "{said}");
    for line in said.lines() {
        assert!(line.chars().count() <= 80, "over 80 columns: {line:?}");
    }
    // Staff see it, marked.
    tw.run(GOD, "s");
    tw.run(GOD, "l");
    let events = tw.run(GOD, "+atlas");
    let said = texts(&tw, &events, GOD);
    assert!(said.contains("Production Office [restricted]"), "{said}");
    assert!(
        said.contains("Mezzanine"),
        "the floor is drawn with its name: {said}"
    );
    // God walks up; Alice may not.
    tw.run(GOD, "up");
    assert_eq!(
        tw.world().get(GOD).and_then(|o| o.location),
        Some(room_named(&tw, "Production Office"))
    );
}

#[test]
fn the_views_answer_from_limbo_by_name_and_for_the_region() {
    let mut tw = lot();
    let alice = tw.spawn_player("Alice");
    let events = tw.run(alice, "+atlas");
    let said = texts(&tw, &events, alice);
    assert!(said.contains("Limbo is on no map"), "{said}");
    tw.run(alice, "o");
    let events = tw.run(alice, "+atlas");
    let said = texts(&tw, &events, alice);
    assert!(said.contains("OOC Area"), "{said}");
    assert!(said.contains("Welcome Hall"), "{said}");
    assert!(said.contains("Sharon Norbury"), "{said}");
    assert!(
        said.contains("Studio Tour"),
        "the way out is listed: {said}"
    );
    let events = tw.run(alice, "+atlas staff lobby");
    let said = texts(&tw, &events, alice);
    assert!(said.contains("Staff Lobby"), "{said}");
    assert!(!said.contains("Production Office"), "{said}");
    let events = tw.run(alice, "+atlas/region");
    let said = texts(&tw, &events, alice);
    assert!(said.contains("The Lot"), "{said}");
    assert!(said.contains("(you are here)"), "{said}");
    assert!(said.contains("Security Gate"), "{said}");
    for line in said.lines() {
        assert!(line.chars().count() <= 80, "over 80 columns: {line:?}");
    }
}

#[test]
fn pack_list_and_show_are_staff_only_and_show_keeps_the_secret_off_the_map() {
    let mut tw = lot();
    let alice = tw.spawn_player("Alice");
    let events = tw.run(alice, "@atlas/pack list");
    let said = texts(&tw, &events, alice);
    assert!(said.contains("Permission denied"), "{said}");
    let events = tw.run(GOD, "@atlas/pack list");
    let said = texts(&tw, &events, GOD);
    assert!(said.contains("region.limbo@1"), "{said}");
    assert!(said.contains("applied v1"), "{said}");
    let events = tw.run(GOD, "@atlas/pack show neighborhood.staff_lobby");
    let said = texts(&tw, &events, GOD);
    assert!(said.contains("secret secret.late"), "{said}");
    assert!(said.contains("requires: region.limbo@1"), "{said}");
    // The secret is nowhere a player can reach: not on the map, not on any NPC.
    tw.run(alice, "s");
    let events = tw.run(alice, "+atlas");
    let said = texts(&tw, &events, alice);
    assert!(!said.contains("long way"), "{said}");
    for npc in tw.world().iter().filter(|o| o.kind == ObjKind::Npc) {
        let knowledge = npc
            .attrs
            .get(mush_npc::KNOWLEDGE_ATTR)
            .and_then(mush_core::Attr::as_json)
            .map(ToString::to_string)
            .unwrap_or_default();
        assert!(
            !knowledge.contains("long way"),
            "{}'s knowledge carries the secret",
            npc.name
        );
    }
}

#[test]
fn every_event_a_boot_emits_is_plain_text() {
    // Nothing here is a real assertion about rendering; it keeps `text_of` honest
    // for the Sys events the views emit.
    let mut tw = lot();
    let events = tw.run(GOD, "@atlas/pack list");
    for (_, event) in &events {
        let _ = text_of(event);
    }
}

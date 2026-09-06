//! End-to-end faction registry tests through the real `Engine`: staff
//! administration, the visibility projections (covert members, hidden
//! factions, covert holdings), edge acyclicity, the permission gate, comsys
//! channel binding, and the full persistence round trip (drain module ops →
//! `InMemoryStore` → fresh boot → identical state).

use std::collections::HashMap;

use mush_comsys::ComsysModule;
use mush_core::engine::{Effects, Engine, Inbound};
use mush_core::{
    Flags, GOD, LIMBO, ModuleBootData, ModuleOp, ModuleWrite, ObjId, ObjKind, Outbox, Powers,
    Registry, World, register_core_commands, register_engine_commands,
};
use mush_faction::{FactionModule, FactionState, NS, Visibility};
use mush_proto::OutputEvent;
use mush_store::{InMemoryStore, ObjectStore, WriteBatch};
use smol_str::SmolStr;

// ----------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------

/// Engine with core + engine commands, the faction module, and comsys (for
/// channel-binding tests); players Nick, Ann, and Bob in Limbo; first boot.
fn boot() -> (Engine, ObjId, ObjId, ObjId) {
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let nick = world
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .unwrap();
    let ann = world
        .create(ObjKind::Player, "Ann", GOD, Some(LIMBO))
        .unwrap();
    let bob = world
        .create(ObjKind::Player, "Bob", GOD, Some(LIMBO))
        .unwrap();
    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    register_engine_commands(&mut registry).unwrap();
    let mut engine = Engine::with_modules(
        world,
        registry,
        vec![
            Box::new(ComsysModule::new()),
            Box::new(FactionModule::new()),
        ],
    )
    .unwrap();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(HashMap::new(), &mut out, &mut fx)
        .unwrap();
    (engine, nick, ann, bob)
}

fn setup() -> (Engine, ObjId, ObjId, ObjId) {
    let (mut engine, nick, ann, bob) = boot();
    let _ = engine.take_module_ops();
    (engine, nick, ann, bob)
}

fn run(engine: &mut Engine, actor: ObjId, line: &str) -> Outbox {
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.step(
        Inbound::Cmd {
            conn: 1,
            actor,
            line: line.into(),
        },
        &mut out,
        &mut fx,
    );
    out
}

fn state(engine: &Engine) -> &FactionState {
    engine
        .world
        .module_state::<FactionState>()
        .expect("faction state")
}

fn sys_texts(out: &Outbox) -> Vec<String> {
    out.events()
        .iter()
        .filter_map(|(_, e)| match e {
            OutputEvent::Sys(t) => Some(t.to_plain_text()),
            _ => None,
        })
        .collect()
}

fn joined(out: &Outbox) -> String {
    sys_texts(out).join("\n")
}

// ----------------------------------------------------------------------
// Administration + visibility projection
// ----------------------------------------------------------------------

#[test]
fn create_add_and_visibility_projection() {
    let (mut engine, nick, ann, bob) = setup();
    let out = run(&mut engine, GOD, "@faction/create Camarilla");
    assert!(joined(&out).contains("Created faction Camarilla"));
    run(
        &mut engine,
        GOD,
        "@faction/add Camarilla=Nick/level:3/title:Prince",
    );
    run(&mut engine, GOD, "@faction/add Camarilla=Ann/vis:covert");

    // Truth: both memberships exist.
    let st = state(&engine);
    let cam = st.faction_by_name("Camarilla").unwrap().id;
    assert!(st.is_member(nick, cam));
    assert!(st.is_member(ann, cam));
    assert!(st.is_officer(nick, cam), "level 3 >= default officer 2");

    // Projection: an outsider sees the public member, never the covert one.
    let out = run(&mut engine, bob, "+faction Camarilla");
    let text = joined(&out);
    assert!(text.contains("Nick"), "{text}");
    assert!(text.contains("Prince"), "{text}");
    assert!(!text.contains("Ann"), "covert member leaked: {text}");
    assert!(
        !text.contains("level"),
        "levels are officer/staff knowledge: {text}"
    );
    // The +factions count only counts visible members.
    let out = run(&mut engine, bob, "+factions");
    assert!(joined(&out).contains("1 known member"), "{}", joined(&out));

    // The covert member always sees themselves.
    let out = run(&mut engine, ann, "+faction Camarilla");
    let text = joined(&out);
    assert!(text.contains("Your membership"), "{text}");
    assert!(text.contains("covert — only you and staff know"), "{text}");

    // Staff truth dump shows everything.
    let out = run(&mut engine, GOD, "@faction/show Camarilla");
    let text = joined(&out);
    assert!(text.contains("Ann"), "{text}");
    assert!(text.contains("covert"), "{text}");
}

#[test]
fn hidden_faction_is_indistinguishable_from_missing() {
    let (mut engine, _nick, ann, bob) = setup();
    run(&mut engine, GOD, "@faction/create Black Hand");
    run(&mut engine, GOD, "@faction/set Black Hand=hidden:yes");
    run(&mut engine, GOD, "@faction/add Black Hand=Ann/vis:covert");

    let out = run(&mut engine, bob, "+factions");
    assert!(
        !joined(&out).contains("Black Hand"),
        "hidden faction listed to outsider: {}",
        joined(&out)
    );
    let out = run(&mut engine, bob, "+faction Black Hand");
    assert!(
        joined(&out).contains("No such faction."),
        "{}",
        joined(&out)
    );
    // The member sees it.
    let out = run(&mut engine, ann, "+factions");
    assert!(joined(&out).contains("Black Hand"), "{}", joined(&out));
}

#[test]
fn structural_edges_are_acyclic_and_front_edges_stay_covert() {
    let (mut engine, _nick, _ann, bob) = setup();
    run(&mut engine, GOD, "@faction/create Court of Chicago");
    run(&mut engine, GOD, "@faction/create Camarilla");
    let out = run(
        &mut engine,
        GOD,
        "@faction/edge Court of Chicago=chapter/Camarilla",
    );
    assert!(
        joined(&out).contains("chapter of Camarilla"),
        "{}",
        joined(&out)
    );
    // Closing the loop in the same kind is refused.
    let out = run(
        &mut engine,
        GOD,
        "@faction/edge Camarilla=chapter/Court of Chicago",
    );
    assert!(
        joined(&out).contains("acyclic per kind"),
        "{}",
        joined(&out)
    );
    // A different kind between the same nodes is fine.
    let out = run(
        &mut engine,
        GOD,
        "@faction/edge Camarilla=contracted/Court of Chicago",
    );
    assert!(joined(&out).contains("contracted to"), "{}", joined(&out));

    // FrontFor defaults covert: players never see it, staff do.
    run(&mut engine, GOD, "@faction/create Setites");
    run(&mut engine, GOD, "@faction/create Temple of Set");
    run(
        &mut engine,
        GOD,
        "@faction/edge Temple of Set=front/Setites",
    );
    let out = run(&mut engine, bob, "+faction Temple of Set");
    assert!(
        !joined(&out).to_lowercase().contains("front for"),
        "covert front edge leaked: {}",
        joined(&out)
    );
    let out = run(&mut engine, GOD, "@faction/show Temple of Set");
    assert!(joined(&out).contains("front for"), "{}", joined(&out));
}

#[test]
fn admin_gate_royalty_or_faction_admin_power() {
    let (mut engine, nick, _ann, _bob) = setup();
    let out = run(&mut engine, nick, "@faction/create Anarchs");
    assert!(
        joined(&out).contains("Permission denied."),
        "{}",
        joined(&out)
    );
    assert!(state(&engine).faction_by_name("Anarchs").is_none());
    // The granular power admits without a staff role.
    engine.world.get_mut(nick).unwrap().powers = Powers::FACTION_ADMIN;
    let out = run(&mut engine, nick, "@faction/create Anarchs");
    assert!(
        joined(&out).contains("Created faction Anarchs"),
        "{}",
        joined(&out)
    );
}

// ----------------------------------------------------------------------
// Channel binding
// ----------------------------------------------------------------------

fn on_channel(engine: &Engine, who: ObjId, chan: mush_comsys::ChannelId) -> bool {
    engine
        .world
        .module_state::<mush_comsys::ComsysState>()
        .unwrap()
        .members
        .contains_key(&(who, chan))
}

#[test]
fn channel_binding_joins_only_public_members() {
    let (mut engine, nick, ann, bob) = setup();
    run(&mut engine, GOD, "@channel/create Elysium");
    run(&mut engine, GOD, "@faction/create Camarilla");
    run(&mut engine, GOD, "@faction/add Camarilla=Nick");
    run(&mut engine, GOD, "@faction/add Camarilla=Ann/vis:covert");
    run(&mut engine, GOD, "@faction/channel Camarilla=Elysium");

    let chan = mush_comsys::api::channel_id_by_name(&engine.world, "Elysium").unwrap();
    assert!(on_channel(&engine, nick, chan), "public member auto-joined");
    assert!(
        !on_channel(&engine, ann, chan),
        "covert member must NOT ride the channel roster (it would leak)"
    );

    // A comsys roster is world-readable (@channel/who under the default open
    // see lock), so even MembersKnow must not be auto-joined.
    run(&mut engine, GOD, "@faction/add Camarilla=Bob/vis:members");
    assert!(
        !on_channel(&engine, bob, chan),
        "MembersKnow on a world-readable roster would leak the membership"
    );

    // Raising to Public joins; dropping below Public leaves; removal leaves.
    run(&mut engine, GOD, "@faction/mod Camarilla=Bob/vis:public");
    assert!(on_channel(&engine, bob, chan));
    run(&mut engine, GOD, "@faction/remove Camarilla=Bob");
    assert!(!on_channel(&engine, bob, chan));
    run(&mut engine, GOD, "@faction/mod Camarilla=Nick/vis:covert");
    assert!(
        !on_channel(&engine, nick, chan),
        "channel membership follows public membership"
    );

    // Binding an open-locked channel warns staff about the world-open roster.
    let out = run(&mut engine, GOD, "@faction/channel Camarilla=Elysium");
    assert!(
        joined(&out).contains("currently open"),
        "open see/join locks deserve a warning: {}",
        joined(&out)
    );
}

#[test]
fn manual_channel_joins_are_never_the_modules_to_remove() {
    let (mut engine, nick, ann, _bob) = setup();
    run(&mut engine, GOD, "@channel/create Elysium");
    run(&mut engine, GOD, "@faction/create Camarilla");
    run(&mut engine, GOD, "@faction/add Camarilla=Nick");
    run(&mut engine, GOD, "@faction/add Camarilla=Ann/vis:covert");
    run(&mut engine, GOD, "@faction/channel Camarilla=Elysium");
    let chan = mush_comsys::api::channel_id_by_name(&engine.world, "Elysium").unwrap();

    // The covert member joins by hand (the channel is open) — self-disclosure
    // is hers to make, and the module must never undo it.
    run(&mut engine, ann, "addcom ely=Elysium");
    assert!(on_channel(&engine, ann, chan));
    run(&mut engine, GOD, "@faction/mod Camarilla=Ann/title:Scourge");
    assert!(
        on_channel(&engine, ann, chan),
        "a title-only edit must not kick a manual channel member"
    );
    run(&mut engine, GOD, "@faction/remove Camarilla=Ann");
    assert!(
        on_channel(&engine, ann, chan),
        "removing a covert member must not touch a join the module never made"
    );

    // The public member deliberately leaves; unrelated edits must not
    // silently re-join them.
    run(&mut engine, nick, "delcom camarilla");
    assert!(!on_channel(&engine, nick, chan));
    run(
        &mut engine,
        GOD,
        "@faction/mod Camarilla=Nick/title:Sheriff",
    );
    assert!(
        !on_channel(&engine, nick, chan),
        "a cosmetic edit must not revert a deliberate delcom"
    );
}

#[test]
fn destroy_leaves_the_bound_channel() {
    let (mut engine, nick, _ann, _bob) = setup();
    run(&mut engine, GOD, "@channel/create Elysium");
    run(&mut engine, GOD, "@faction/create Camarilla");
    run(&mut engine, GOD, "@faction/add Camarilla=Nick");
    run(&mut engine, GOD, "@faction/channel Camarilla=Elysium");
    let chan = mush_comsys::api::channel_id_by_name(&engine.world, "Elysium").unwrap();
    assert!(on_channel(&engine, nick, chan));
    run(&mut engine, GOD, "@faction/destroy Camarilla");
    assert!(
        !on_channel(&engine, nick, chan),
        "destroy mirrors @faction/remove: module-joined members leave the channel"
    );
}

#[test]
fn channel_inspect_without_equals_does_not_unbind() {
    let (mut engine, _nick, _ann, _bob) = setup();
    run(&mut engine, GOD, "@channel/create Elysium");
    run(&mut engine, GOD, "@faction/create Camarilla");
    run(&mut engine, GOD, "@faction/channel Camarilla=Elysium");
    let out = run(&mut engine, GOD, "@faction/channel Camarilla");
    assert!(
        joined(&out).contains("is bound to channel Elysium"),
        "{}",
        joined(&out)
    );
    assert!(
        state(&engine)
            .faction_by_name("Camarilla")
            .unwrap()
            .channel
            .is_some(),
        "inspection must never unbind"
    );
    // The explicit '=' form unbinds.
    run(&mut engine, GOD, "@faction/channel Camarilla=");
    assert!(
        state(&engine)
            .faction_by_name("Camarilla")
            .unwrap()
            .channel
            .is_none()
    );
}

#[test]
fn aliases_are_validated_like_names() {
    let (mut engine, _nick, _ann, _bob) = setup();
    run(&mut engine, GOD, "@faction/create Camarilla");
    run(&mut engine, GOD, "@faction/create Undertow");
    // A collision with another faction's name would silently re-target every
    // name-resolving command (including @faction/destroy).
    let out = run(&mut engine, GOD, "@faction/set Camarilla=aliases:Undertow");
    assert!(joined(&out).contains("collides"), "{}", joined(&out));
    assert!(
        state(&engine)
            .faction_by_name("Camarilla")
            .unwrap()
            .aliases
            .is_empty()
    );
    // Illegal charset (the '/'-separated command grammar) is refused too.
    let out = run(&mut engine, GOD, "@faction/set Camarilla=aliases:Cam/Ivory");
    assert!(
        joined(&out).contains("not a usable name"),
        "{}",
        joined(&out)
    );
    // A legal alias resolves; destroy still hits the right faction.
    run(
        &mut engine,
        GOD,
        "@faction/set Camarilla=aliases:The Ivory Tower",
    );
    let out = run(&mut engine, GOD, "@faction/destroy Undertow");
    assert!(
        joined(&out).contains("Destroyed faction Undertow"),
        "{}",
        joined(&out)
    );
    assert!(state(&engine).faction_by_name("The Ivory Tower").is_some());
}

#[test]
fn roster_order_hides_rank_from_non_officers() {
    let (mut engine, nick, _ann, _bob) = setup();
    run(&mut engine, GOD, "@faction/create Camarilla");
    // Bob outranks Ann; alphabetical and rank order disagree.
    run(&mut engine, GOD, "@faction/add Camarilla=Bob/level:5");
    run(&mut engine, GOD, "@faction/add Camarilla=Ann/level:1");
    // An outsider gets the alphabetical roster (rank order is itself
    // officer/staff knowledge — a sorted list would leak it label-free).
    let out = run(&mut engine, nick, "+faction Camarilla");
    let text = joined(&out);
    let (ann_at, bob_at) = (text.find("Ann").unwrap(), text.find("Bob").unwrap());
    assert!(ann_at < bob_at, "outsiders see alphabetical order: {text}");
    // Staff (and officers) see the rank-sorted roster with levels.
    let out = run(&mut engine, GOD, "+faction Camarilla");
    let text = joined(&out);
    let (ann_at, bob_at) = (text.find("Ann").unwrap(), text.find("Bob").unwrap());
    assert!(bob_at < ann_at, "officers/staff see rank order: {text}");
    assert!(text.contains("[level 5]"), "{text}");
}

// ----------------------------------------------------------------------
// Territory
// ----------------------------------------------------------------------

#[test]
fn territory_control_contests_and_covert_holdings() {
    let (mut engine, nick, _ann, bob) = setup();
    run(&mut engine, GOD, "@region/create The Waterfront");
    run(&mut engine, GOD, "@faction/create Undertow");
    run(&mut engine, GOD, "@faction/create Camarilla");
    run(&mut engine, GOD, "@faction/add Undertow=Nick/level:3");
    run(
        &mut engine,
        GOD,
        "@faction/control The Waterfront=Undertow/defacto",
    );
    run(
        &mut engine,
        GOD,
        "@faction/control The Waterfront=Camarilla/dejure",
    );
    run(
        &mut engine,
        GOD,
        "@faction/contest The Waterfront=Camarilla/dejure/Undertow",
    );

    // The de jure / de facto split is public knowledge here.
    let out = run(&mut engine, bob, "+territory");
    let text = joined(&out);
    assert!(text.contains("Undertow (de facto)"), "{text}");
    assert!(
        text.contains("Camarilla (de jure, contested by Undertow)"),
        "{text}"
    );

    // A covert hold: outsiders never see it; the holder's officers do.
    run(
        &mut engine,
        GOD,
        "@faction/control The Waterfront=Undertow/covert/vis:covert",
    );
    let out = run(&mut engine, bob, "+territory");
    assert!(
        !joined(&out).contains("Undertow (covert)"),
        "covert hold leaked to outsider: {}",
        joined(&out)
    );
    let out = run(&mut engine, nick, "+territory");
    assert!(
        joined(&out).contains("Undertow (covert)"),
        "the holder's officer knows their own holdings: {}",
        joined(&out)
    );

    // Zone coverage requires a zone master object.
    let zmo = engine
        .world
        .create(ObjKind::Thing, "Docks Zone", GOD, Some(LIMBO))
        .unwrap();
    let out = run(&mut engine, GOD, "@region/zone The Waterfront=Docks Zone");
    assert!(
        joined(&out).contains("not a zone master object"),
        "{}",
        joined(&out)
    );
    engine
        .world
        .get_mut(zmo)
        .unwrap()
        .flags
        .insert(Flags::ZONE_MASTER);
    let out = run(&mut engine, GOD, "@region/zone The Waterfront=Docks Zone");
    assert!(joined(&out).contains("now covers zone"), "{}", joined(&out));
}

// ----------------------------------------------------------------------
// Persistence round trip
// ----------------------------------------------------------------------

/// Boot a second engine over an identical world from the boot data read back
/// out of the store.
fn reboot_from_store(store: &InMemoryStore) -> Engine {
    let kv = futures::executor::block_on(async { store.module_json_list(NS).await.unwrap() });
    let boot_data = ModuleBootData {
        kv: kv.into_iter().map(|(k, v)| (SmolStr::new(k), v)).collect(),
        streams: HashMap::new(),
        ..Default::default()
    };
    let mut world = World::new_minimal();
    world.set_now(1_000);
    world
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .unwrap();
    world
        .create(ObjKind::Player, "Ann", GOD, Some(LIMBO))
        .unwrap();
    world
        .create(ObjKind::Player, "Bob", GOD, Some(LIMBO))
        .unwrap();
    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    register_engine_commands(&mut registry).unwrap();
    let mut engine = Engine::with_modules(
        world,
        registry,
        vec![
            Box::new(ComsysModule::new()),
            Box::new(FactionModule::new()),
        ],
    )
    .unwrap();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(
            HashMap::from([(SmolStr::new(NS), boot_data)]),
            &mut out,
            &mut fx,
        )
        .unwrap();
    engine
}

#[test]
fn persistence_round_trip_rebuilds_identical_state() {
    let (mut engine, _nick, _ann, _bob) = boot();
    let mut ops: Vec<ModuleOp> = engine.take_module_ops();
    for (actor, line) in [
        (GOD, "@faction/create Camarilla"),
        (GOD, "@faction/desc Camarilla=The Ivory Tower."),
        (GOD, "@faction/set Camarilla=officer:3"),
        (GOD, "@faction/set Camarilla=intel:2"),
        (GOD, "@faction/create Undertow"),
        (GOD, "@faction/set Undertow=hidden:yes"),
        (GOD, "@faction/add Camarilla=Nick/level:3/title:Prince"),
        (GOD, "@faction/add Camarilla=Ann/vis:covert"),
        (GOD, "@faction/add Undertow=Bob/vis:members"),
        (GOD, "@faction/edge Undertow=subordinate/Camarilla"),
        (
            GOD,
            "@faction/resource Camarilla=influence.police/3/precinct captains",
        ),
        (GOD, "@region/create The Waterfront"),
        (GOD, "@region/note The Waterfront=Docks and warehouses."),
        (
            GOD,
            "@faction/control The Waterfront=Undertow/defacto/strength:2",
        ),
        (
            GOD,
            "@faction/contest The Waterfront=Undertow/defacto/Camarilla",
        ),
    ] {
        let out = run(&mut engine, actor, line);
        let text = joined(&out);
        assert!(
            !text.contains("Huh?") && !text.contains("Usage:") && !text.contains("No such"),
            "{line} failed: {text}"
        );
        ops.extend(engine.take_module_ops());
    }
    let store = InMemoryStore::new();
    futures::executor::block_on(async {
        store
            .write(WriteBatch {
                serial: 1,
                module_ops: ops,
                ..WriteBatch::default()
            })
            .await
            .unwrap();
    });
    let engine2 = reboot_from_store(&store);
    assert_eq!(
        state(&engine),
        state(&engine2),
        "factions, members, edges, resources, regions, control, and the id \
         allocators all round-trip"
    );
    // Sanity on the rebuilt side.
    let st2 = state(&engine2);
    let cam = st2.faction_by_name("Camarilla").unwrap();
    assert_eq!(cam.officer_level, 3);
    assert_eq!(cam.intel_level, 2);
    assert_eq!(cam.resources.get("influence.police"), Some(&3));
    assert!(st2.faction_by_name("Undertow").unwrap().hidden);
    let region = st2.region_by_name("The Waterfront").unwrap();
    let recs = st2.control.get(&region.id).unwrap();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].strength, 2);
    assert_eq!(recs[0].contested_by.len(), 1);
    assert_eq!(st2.next_faction_id, 2);
    assert_eq!(st2.next_region_id, 1);
}

#[test]
fn boot_never_rewinds_allocators_below_loaded_ids() {
    // An older/partially restored meta doc must not let alloc hand out a
    // live id (silently replacing an existing faction).
    let (mut engine, _nick, _ann, _bob) = boot();
    let mut ops: Vec<ModuleOp> = engine.take_module_ops();
    run(&mut engine, GOD, "@faction/create Camarilla");
    ops.extend(engine.take_module_ops());
    run(&mut engine, GOD, "@faction/create Undertow");
    ops.extend(engine.take_module_ops());
    // Simulate the stale restore: overwrite meta with rewound counters.
    ops.push(ModuleOp {
        ns: SmolStr::new(NS),
        op: ModuleWrite::JsonSet {
            key: SmolStr::new("meta"),
            value: serde_json::json!({ "next_faction_id": 0, "next_region_id": 0 }),
        },
    });
    let store = InMemoryStore::new();
    futures::executor::block_on(async {
        store
            .write(WriteBatch {
                serial: 1,
                module_ops: ops,
                ..WriteBatch::default()
            })
            .await
            .unwrap();
    });
    let mut engine2 = reboot_from_store(&store);
    assert_eq!(
        state(&engine2).next_faction_id,
        2,
        "allocator resumes past the loaded ids, not at the stale meta value"
    );
    let out = run(&mut engine2, GOD, "@faction/create Anarchs");
    assert!(joined(&out).contains("(id 2)"), "{}", joined(&out));
    assert!(state(&engine2).faction_by_name("Camarilla").is_some());
    assert!(state(&engine2).faction_by_name("Undertow").is_some());
}

#[test]
fn destroy_cascades_members_edges_and_control() {
    let (mut engine, nick, _ann, _bob) = setup();
    run(&mut engine, GOD, "@faction/create Camarilla");
    run(&mut engine, GOD, "@faction/create Undertow");
    run(&mut engine, GOD, "@faction/add Undertow=Nick");
    run(
        &mut engine,
        GOD,
        "@faction/edge Undertow=subordinate/Camarilla",
    );
    run(&mut engine, GOD, "@region/create The Waterfront");
    run(
        &mut engine,
        GOD,
        "@faction/control The Waterfront=Undertow/defacto",
    );
    run(
        &mut engine,
        GOD,
        "@faction/contest The Waterfront=Undertow/defacto/Camarilla",
    );

    run(&mut engine, GOD, "@faction/destroy Camarilla");
    let st = state(&engine);
    assert!(st.faction_by_name("Camarilla").is_none());
    let undertow = st.faction_by_name("Undertow").unwrap();
    assert!(
        undertow.edges.is_empty(),
        "edges pointing at the destroyed faction are pruned"
    );
    let region = st.region_by_name("The Waterfront").unwrap();
    assert!(
        st.control.get(&region.id).unwrap()[0]
            .contested_by
            .is_empty(),
        "contested_by entries naming the destroyed faction are pruned"
    );

    run(&mut engine, GOD, "@faction/destroy Undertow");
    let st = state(&engine);
    assert!(!st.is_member(nick, mush_faction::FactionId(1)));
    assert!(
        st.control.is_empty(),
        "control records of the destroyed holder are gone"
    );
}

#[test]
fn membership_visibility_matrix_members_and_officers() {
    let (mut engine, nick, ann, bob) = setup();
    run(&mut engine, GOD, "@faction/create Setites");
    run(&mut engine, GOD, "@faction/add Setites=Nick/level:2"); // officer
    run(&mut engine, GOD, "@faction/add Setites=Ann/vis:members");
    run(&mut engine, GOD, "@faction/add Setites=Bob/vis:officers");
    {
        // Sanity: three memberships with the intended visibilities.
        let st = state(&engine);
        let f = st.faction_by_name("Setites").unwrap().id;
        assert_eq!(st.members_of(f).count(), 3);
        assert!(
            st.members_of(f)
                .any(|m| m.visibility == Visibility::OfficersKnow)
        );
    }
    // Ann (plain member) sees the MembersKnow member (herself) and Nick
    // (public), but not the OfficersKnow member Bob.
    let out = run(&mut engine, ann, "+faction Setites");
    let text = joined(&out);
    assert!(text.contains("Nick"), "{text}");
    assert!(text.contains("Ann"), "{text}");
    assert!(
        !text.contains("Bob"),
        "OfficersKnow leaked to member: {text}"
    );
    // Nick (officer) sees all three.
    let out = run(&mut engine, nick, "+faction Setites");
    let text = joined(&out);
    assert!(
        text.contains("Bob"),
        "officer must see OfficersKnow: {text}"
    );
    // Bob sees himself even though his record is OfficersKnow and he isn't
    // an officer.
    let out = run(&mut engine, bob, "+faction Setites");
    assert!(joined(&out).contains("Your membership"), "{}", joined(&out));
}

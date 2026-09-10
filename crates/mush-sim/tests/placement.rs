//! Phase B cut 1 acceptance: the placement graph, its projections, gates,
//! recovery, and reboot behaviour, driven through the real `Engine` via
//! `TestWorld` with the `SimulationModule` installed.

use std::collections::HashMap;

use mush_core::{
    BehaviorBinding, BehaviorId, Effects, GOD, LIMBO, ModuleBootData, ModuleWrite, ObjId, ObjKind,
    Outbox, WorldEvent,
};
use mush_proto::OutputEvent;
use mush_sim::{
    DomainId, DomainStatus, SimConfig, SimulationModule, SpatialParent, SpatialState,
    verify_indexes, verify_projections,
};
use mush_store::{InMemoryStore, ObjectStore, WriteBatch};
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![Box::new(SimulationModule::new())])
}

fn texts(events: &[(mush_core::Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}

fn bind(tw: &mut TestWorld, obj: ObjId, behavior: &str) {
    tw.world_mut()
        .attach_behavior(
            obj,
            BehaviorId::new(behavior).unwrap(),
            BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .unwrap();
}

fn loc(tw: &TestWorld, id: ObjId) -> Option<ObjId> {
    tw.world().get(id).and_then(|o| o.location)
}

fn state(tw: &TestWorld) -> &SpatialState {
    tw.world()
        .module_state::<SpatialState>()
        .expect("sim state")
}

/// The shared fixture. Creation order is fixed so a second `TestWorld`
/// allocates the same ids (reboot tests).
struct Fx {
    field: ObjId,
    hangar: ObjId,
    mech: ObjId,
    mech2: ObjId,
    carrier: ObjId,
}

fn fixture(tw: &mut TestWorld) -> Fx {
    let field = tw
        .world_mut()
        .create(ObjKind::Room, "Field", GOD, None)
        .unwrap();
    let hangar = tw
        .world_mut()
        .create(ObjKind::Room, "Hangar", GOD, None)
        .unwrap();
    let mech = tw
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, Some(LIMBO))
        .unwrap();
    let mech2 = tw
        .world_mut()
        .create(ObjKind::Thing, "Locust", GOD, Some(LIMBO))
        .unwrap();
    let carrier = tw
        .world_mut()
        .create(ObjKind::Thing, "Leopard", GOD, Some(LIMBO))
        .unwrap();
    bind(tw, field, "sim.domain");
    bind(tw, hangar, "sim.interior");
    bind(tw, mech, "sim.participant");
    bind(tw, mech2, "sim.participant");
    bind(tw, carrier, "sim.participant");
    Fx {
        field,
        hangar,
        mech,
        mech2,
        carrier,
    }
}

/// `match_object` resolves bare names only in the actor's room/inventory, so
/// staff address remote rooms and units by dbref. The fixture ids are
/// deterministic (Limbo #0, GOD #1, then the fixture in creation order), so
/// the tests may write names and have them substituted here.
fn sub(line: &str) -> String {
    let mut s = line.to_string();
    for (name, id) in [
        ("Field", 2),
        ("Hangar", 3),
        ("Atlas", 4),
        ("Locust", 5),
        ("Leopard", 6),
    ] {
        let mut out = String::new();
        let mut rest = s.as_str();
        while let Some(i) = rest.find(name) {
            let before = rest[..i]
                .chars()
                .last()
                .is_none_or(|c| !c.is_alphanumeric() && c != '_');
            let after = rest[i + name.len()..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric() && c != '_');
            out.push_str(&rest[..i]);
            if before && after {
                out.push_str(&format!("#{id}"));
            } else {
                out.push_str(name);
            }
            rest = &rest[i + name.len()..];
        }
        out.push_str(rest);
        s = out;
    }
    s
}

fn say(tw: &mut TestWorld, line: &str) -> String {
    texts(&tw.run(GOD, &sub(line)))
}

/// Any error-tagged span — every `CmdError`, `SpatialError`, and gate refusal
/// renders as one, our own `sim`-tagged info lines never do.
fn has_error(events: &[(mush_core::Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
}

/// Run a command that must be accepted: fails loudly on any refusal.
fn ok(tw: &mut TestWorld, line: &str) -> String {
    let events = tw.run(GOD, &sub(line));
    let t = texts(&events);
    assert!(!has_error(&events), "{line:?} was refused: {t}");
    t
}

fn check_invariants(tw: &TestWorld) {
    let st = state(tw);
    verify_indexes(st).unwrap();
    let bad = verify_projections(st, tw.world());
    assert!(bad.is_empty(), "projection drift: {bad:?}");
}

fn fold_kv(tw: &mut TestWorld) -> Vec<(SmolStr, serde_json::Value)> {
    let mut kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    for op in tw.take_module_ops() {
        assert_eq!(op.ns, "sim");
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.retain(|(k, _)| *k != key);
                kv.push((key, value));
            }
            ModuleWrite::Del { key } => kv.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv
}

fn reboot_with(kv: Vec<(SmolStr, serde_json::Value)>) -> (TestWorld, Fx) {
    let mut tw2 = boot();
    let fx = fixture(&mut tw2);
    tw2.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        ModuleBootData {
            kv,
            streams: HashMap::new(),
            ..Default::default()
        },
    )]));
    (tw2, fx)
}

fn legacy_passive(data: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "schema": "sim.passive", "version": 1, "data": data })
}

fn module_doc_mut<'a>(
    kv: &'a mut [(SmolStr, serde_json::Value)],
    key: &str,
) -> &'a mut serde_json::Value {
    &mut kv
        .iter_mut()
        .find(|(candidate, _)| candidate == key)
        .unwrap_or_else(|| panic!("missing module doc {key}"))
        .1
}

// ----------------------------------------------------------------------

#[test]
fn domain_create_and_direct_placement_projects_into_presentation_room() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    let t = ok(&mut tw, "@domain/create Field=sim.passive");
    assert!(t.contains("Created domain #0"), "{t}");
    let t = ok(&mut tw, "@spatial/place/domain Atlas=0");
    assert!(t.contains("Placed Atlas direct in domain #0"), "{t}");
    assert_eq!(
        loc(&tw, fx.mech),
        Some(fx.field),
        "projected into the presentation room"
    );
    let st = state(&tw);
    assert!(matches!(
        st.parent_of(fx.mech),
        SpatialParent::Direct {
            domain: DomainId(0),
            ..
        }
    ));
    assert_eq!(st.derived.domain_direct[&DomainId(0)].len(), 1);
    assert_eq!(st.derived.projected[&DomainId(0)].len(), 1);
    assert_eq!(mush_sim::effective_domain(st, fx.mech), Some(DomainId(0)));
    check_invariants(&tw);
    // Persisted: domain, meta, placement, and a recovery-log chunk.
    let batch = tw.drain_write_batch();
    let keys: Vec<String> = batch
        .module_ops
        .iter()
        .filter_map(|o| match &o.op {
            ModuleWrite::JsonSet { key, .. } => Some(key.to_string()),
            _ => None,
        })
        .collect();
    assert!(keys.iter().any(|k| k == "domain:0"), "{keys:?}");
    assert!(keys.iter().any(|k| k == "meta"), "{keys:?}");
    assert!(
        keys.iter()
            .any(|k| k == &format!("placement:{}", fx.mech.0)),
        "{keys:?}"
    );
    assert!(batch.appended_to("sim", "recovery"), "audit appended");
}

#[test]
fn placement_needs_participant_and_domain_needs_host_behavior() {
    let mut tw = boot();
    let _fx = fixture(&mut tw);
    let plain = tw
        .world_mut()
        .create(ObjKind::Thing, "Rock", GOD, Some(LIMBO))
        .unwrap();
    ok(&mut tw, "@domain/create Field=sim.passive");
    let t = say(&mut tw, "@spatial/place/domain Rock=0");
    assert!(
        t.contains("must carry the `sim.participant` behavior"),
        "{t}"
    );
    assert!(!state(&tw).placements.contains_key(&plain));
    let t = say(&mut tw, "@domain/create Hangar=sim.passive");
    assert!(t.contains("must carry `sim.domain`"), "{t}");
    let t = say(&mut tw, "@domain/create Field=sim.passive");
    assert!(t.contains("already has a domain"), "{t}");
}

#[test]
fn interior_space_projection_and_slot_capacity() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/space Hangar=fixed:0");
    ok(&mut tw, "@spatial/slot Hangar=bay,1");
    ok(&mut tw, "@spatial/place/interior Atlas=Hangar,bay");
    assert_eq!(loc(&tw, fx.mech), Some(fx.hangar));
    let st = state(&tw);
    assert_eq!(
        mush_sim::effective_domain(st, fx.mech),
        Some(DomainId(0)),
        "fixed anchor gives locality"
    );
    assert_eq!(
        mush_sim::presence(st, fx.mech),
        Some(mush_sim::Presence::Suppressed)
    );
    assert!(st.derived.local_objects[&DomainId(0)].contains(&fx.mech));
    assert!(
        !st.derived
            .projected
            .get(&DomainId(0))
            .is_some_and(|s| s.contains(&fx.mech))
    );
    // Capacity 1: the second mech is refused, nothing changes.
    let t = say(&mut tw, "@spatial/place/interior Locust=Hangar,bay");
    assert!(t.contains("is full (1 max)"), "{t}");
    assert_eq!(loc(&tw, fx.mech2), Some(LIMBO));
    assert!(!state(&tw).placements.contains_key(&fx.mech2));
    // Re-placing the same mech into the same slot is not double-counted.
    ok(&mut tw, "@spatial/place/interior Atlas=Hangar,bay");
    // Unknown slot refused.
    let t = say(&mut tw, "@spatial/place/interior Locust=Hangar,cargo");
    assert!(t.contains("has no compartment `cargo`"), "{t}");
    check_invariants(&tw);
}

#[test]
fn attached_projection_follows_exposure_and_the_host() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/mount Leopard=deck,1,exposed");
    ok(&mut tw, "@spatial/mount Leopard=hold,4");
    ok(&mut tw, "@spatial/place/domain Leopard=0");
    ok(&mut tw, "@spatial/place/attached Atlas=Leopard,deck");
    ok(&mut tw, "@spatial/place/attached Locust=Leopard,hold");
    assert_eq!(loc(&tw, fx.carrier), Some(fx.field));
    assert_eq!(
        loc(&tw, fx.mech),
        Some(fx.field),
        "exposed: alongside the host"
    );
    assert_eq!(
        loc(&tw, fx.mech2),
        Some(fx.carrier),
        "suppressed: inside the host"
    );
    let st = state(&tw);
    assert_eq!(
        mush_sim::presence(st, fx.mech),
        Some(mush_sim::Presence::DerivedExposed)
    );
    assert_eq!(mush_sim::effective_domain(st, fx.mech2), Some(DomainId(0)));
    assert_eq!(
        st.derived.projected[&DomainId(0)].len(),
        2,
        "carrier + exposed mech"
    );
    assert_eq!(st.children_of(fx.carrier).len(), 2);
    // Deck capacity 1.
    let t = say(&mut tw, "@spatial/place/attached Locust=Leopard,deck");
    assert!(t.contains("is full (1 max)"), "{t}");
    assert_eq!(loc(&tw, fx.mech2), Some(fx.carrier));
    // Re-presenting the domain re-projects the carrier AND its exposed rider.
    let hall = tw
        .world_mut()
        .create(ObjKind::Room, "Hall", GOD, None)
        .unwrap();
    ok(&mut tw, &format!("@domain/present 0=#{}", hall.0));
    assert_eq!(loc(&tw, fx.carrier), Some(hall));
    check_invariants(&tw);
    // Unmount refused while occupied; unplace refused while dependents remain.
    let t = say(&mut tw, "@spatial/unmount Leopard=hold");
    assert!(t.contains("Detach what is mounted there first"), "{t}");
    let t = say(&mut tw, "@spatial/unplace Leopard");
    assert!(t.contains("still has 2 dependent(s)"), "{t}");
}

#[test]
fn cycles_are_refused_without_mutation() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/mount Leopard=deck");
    ok(&mut tw, "@spatial/mount Atlas=rack");
    ok(&mut tw, "@spatial/place/domain Leopard=0");
    ok(&mut tw, "@spatial/place/attached Atlas=Leopard,deck");
    let before = state(&tw).clone();
    let t = say(&mut tw, "@spatial/place/attached Leopard=Atlas,rack");
    assert!(t.contains("its own ancestor"), "{t}");
    assert_eq!(state(&tw), &before, "refused transitions mutate nothing");
    assert_eq!(loc(&tw, fx.carrier), Some(fx.field));
    // Self-attachment too.
    let t = say(&mut tw, "@spatial/place/attached Atlas=Atlas,rack");
    assert!(t.contains("its own ancestor"), "{t}");
    // A riding space CAN be re-anchored to another object not under it: Atlas
    // is attached to Leopard, not below the hangar.
    ok(&mut tw, "@spatial/space Hangar=moving:Leopard");
    let t = ok(&mut tw, "@spatial/space Hangar=moving:Atlas");
    assert!(t.contains("re-anchored"), "{t}");
    assert!(
        matches!(state(&tw).spaces[&fx.hangar].anchor, mush_sim::SpaceAnchor::MovingObject { host } if host == fx.mech)
    );
    // But not to something that IS under it: put Locust inside the hangar, then
    // try to make the hangar ride Locust.
    ok(&mut tw, "@spatial/place/interior Locust=Hangar");
    let t = say(&mut tw, "@spatial/space Hangar=moving:Locust");
    assert!(t.contains("its own ancestor"), "{t}");
    check_invariants(&tw);
}

#[test]
fn anchor_and_placement_of_a_space_must_agree() {
    let mut tw = boot();
    let _fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    // A hangar fixed in domain 0 cannot be placed attached to a carrier.
    ok(&mut tw, "@spatial/space Hangar=fixed:0");
    bind(&mut tw, _fx.hangar, "sim.participant");
    ok(&mut tw, "@spatial/mount Leopard=deck");
    ok(&mut tw, "@spatial/place/domain Leopard=0");
    let t = say(&mut tw, "@spatial/place/attached Hangar=Leopard,deck");
    assert!(
        t.contains("is fixed in domain #0 but placed elsewhere"),
        "{t}"
    );
    // Re-anchor it to ride the carrier, then the attachment is fine.
    ok(&mut tw, "@spatial/space Hangar=moving:Leopard");
    ok(&mut tw, "@spatial/place/attached Hangar=Leopard,deck");
    ok(&mut tw, "@spatial/place/interior Atlas=Hangar");
    let st = state(&tw);
    assert_eq!(
        mush_sim::effective_domain(st, _fx.mech),
        Some(DomainId(0)),
        "through the moving anchor"
    );
    // And it cannot now be re-anchored fixed while attached elsewhere.
    let t = say(&mut tw, "@spatial/space Hangar=fixed:0");
    assert!(t.contains("but placed elsewhere"), "{t}");
    check_invariants(&tw);
}

#[test]
fn movement_and_carry_gates_veto_placed_and_quarantined_objects() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    let bob = tw.spawn_player("Bob");
    bind(&mut tw, bob, "sim.participant");
    let hall = tw
        .world_mut()
        .create(ObjKind::Room, "Hall", GOD, None)
        .unwrap();
    let ex = tw
        .world_mut()
        .create(ObjKind::Exit, "north", GOD, Some(fx.field))
        .unwrap();
    tw.world_mut().link_exit(ex, Some(hall)).unwrap();
    ok(&mut tw, &format!("@spatial/place/domain #{}=0", bob.0));
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    assert_eq!(loc(&tw, bob), Some(fx.field));
    // Bob cannot walk out.
    let t = texts(&tw.run(bob, "north"));
    assert!(t.contains("moves only through the simulation"), "{t}");
    assert_eq!(loc(&tw, bob), Some(fx.field));
    // GOD cannot pick the mech up, even as a wizard.
    tw.world_mut().move_to(GOD, fx.field).unwrap();
    let t = say(&mut tw, "get Atlas");
    assert!(t.contains("moves only through the simulation"), "{t}");
    assert_eq!(loc(&tw, fx.mech), Some(fx.field));
    // Interior occupants are held too (no player egress in cut 1).
    ok(&mut tw, "@spatial/space Hangar=fixed:0");
    ok(
        &mut tw,
        &format!("@spatial/place/interior #{}=Hangar", bob.0),
    );
    assert_eq!(loc(&tw, bob), Some(fx.hangar));
    let ex2 = tw
        .world_mut()
        .create(ObjKind::Exit, "out", GOD, Some(fx.hangar))
        .unwrap();
    tw.world_mut().link_exit(ex2, Some(hall)).unwrap();
    let t = texts(&tw.run(bob, "out"));
    assert!(t.contains("leaves only through the simulation"), "{t}");
    assert_eq!(loc(&tw, bob), Some(fx.hangar), "interior occupant is held");
    // But an ordinary visitor with no placement comes and goes freely.
    let ann = tw.spawn_player("Ann");
    tw.world_mut().move_to(ann, fx.hangar).unwrap();
    let t = texts(&tw.run(ann, "out"));
    assert!(!t.contains("only through"), "{t}");
    assert_eq!(
        loc(&tw, ann),
        Some(hall),
        "a non-placed visitor is not gated"
    );
    // Unplacing returns the mech to the recovery room and frees it.
    ok(&mut tw, "@spatial/unplace Atlas=done testing");
    assert_eq!(loc(&tw, fx.mech), Some(LIMBO));
    tw.world_mut().move_to(GOD, LIMBO).unwrap();
    let t = say(&mut tw, "get Atlas");
    assert!(!t.contains("moves only through"), "{t}");
    assert_eq!(loc(&tw, fx.mech), Some(GOD));
}

#[test]
fn unplace_keep_leaves_the_object_where_it_is() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    ok(&mut tw, "@spatial/unplace/keep Atlas");
    assert_eq!(loc(&tw, fx.mech), Some(fx.field));
    let st = state(&tw);
    let p = &st.placements[&fx.mech];
    assert!(matches!(p.parent, SpatialParent::Unplaced));
    let u = p.unplaced.as_ref().unwrap();
    assert!(!u.quarantined);
    assert_eq!(u.by, Some(GOD));
    assert!(matches!(u.previous, Some(SpatialParent::Direct { .. })));
    // It is a plain object again: not in any domain index.
    assert!(
        st.derived
            .domain_direct
            .get(&DomainId(0))
            .is_none_or(|s| s.is_empty())
    );
    check_invariants(&tw);
}

#[test]
fn structural_destroy_gate_and_detach_gate() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/space Hangar=fixed:0");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    // Host, presentation room (same object here), and space refuse @destroy.
    // (The binding-level Refuse fires first; the structural gate is the
    // backstop for binding-less objects, exercised on `Stage` below.)
    let t = say(&mut tw, "@destroy Field");
    assert!(
        t.contains("hosts domain #0") || t.contains("refuses destruction"),
        "{t}"
    );
    let t = say(&mut tw, "@destroy Hangar");
    assert!(
        t.contains("interior space") || t.contains("refuses destruction"),
        "{t}"
    );
    assert!(tw.world().get(fx.field).is_some() && tw.world().get(fx.hangar).is_some());
    // A Thing-hosted domain protects its (binding-less) presentation room.
    let hq = tw
        .world_mut()
        .create(ObjKind::Thing, "HQ", GOD, Some(LIMBO))
        .unwrap();
    let stage = tw
        .world_mut()
        .create(ObjKind::Room, "Stage", GOD, None)
        .unwrap();
    bind(&mut tw, hq, "sim.domain");
    ok(
        &mut tw,
        &format!("@domain/create HQ=sim.passive,#{}", stage.0),
    );
    let t = say(&mut tw, &format!("@destroy #{}", stage.0));
    assert!(t.contains("presentation room of domain(s) #1"), "{t}");
    // Bindings cannot be stripped while referenced.
    let t = say(&mut tw, "@behavior/remove Field=sim.domain");
    assert!(t.contains("still hosts domain #0"), "{t}");
    let t = say(&mut tw, "@behavior/remove Atlas=sim.participant");
    assert!(t.contains("still placed"), "{t}");
    let t = say(&mut tw, "@behavior/remove Hangar=sim.interior");
    assert!(t.contains("still an interior space"), "{t}");
    // Retire needs an unreferenced domain.
    let t = say(&mut tw, "@domain/retire 0");
    assert!(t.contains("still referenced by 2"), "{t}");
    ok(&mut tw, "@spatial/unplace Atlas");
    ok(&mut tw, "@spatial/space Hangar=unanchored");
    ok(&mut tw, "@domain/retire 0");
    assert_eq!(
        state(&tw).domains[&DomainId(0)].status,
        DomainStatus::Retired
    );
    ok(&mut tw, "@behavior/remove Field=sim.domain");
    ok(&mut tw, "@behavior/remove Atlas=sim.participant");
    ok(&mut tw, "@spatial/unspace Hangar");
    ok(&mut tw, "@behavior/remove Hangar=sim.interior");
    // A retired domain accepts nothing and its id is never reused.
    bind(&mut tw, fx.field, "sim.domain");
    let t = ok(&mut tw, "@domain/create Field=sim.passive");
    assert!(t.contains("Created domain #2"), "{t}");
    check_invariants(&tw);
}

#[test]
fn programmatic_destroy_of_a_host_quarantines_dependents() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/mount Leopard=deck,2,exposed");
    ok(&mut tw, "@spatial/place/domain Leopard=0");
    ok(&mut tw, "@spatial/place/attached Atlas=Leopard,deck");
    ok(&mut tw, "@spatial/place/attached Locust=Leopard,deck");
    // Even an exhausted revision must not prevent destruction quarantine: the
    // record stays terminal at MAX and cannot later be recovered/re-placed.
    tw.world_mut()
        .module_state_mut::<SpatialState>()
        .expect("sim state")
        .placements
        .get_mut(&fx.mech2)
        .expect("Locust placement")
        .revision = u64::MAX;
    let _ = tw.take_module_ops();
    // Destroy the carrier through the world (bypassing @destroy's gate) and
    // fire the event the command layer would have.
    tw.world_mut().destroy(fx.carrier).unwrap();
    let mut out = Outbox::new();
    let mut fx_ = Effects::new();
    tw.engine_mut().emit_world_event(
        &WorldEvent::ObjDestroyed { obj: fx.carrier },
        &mut out,
        &mut fx_,
    );
    let st = state(&tw);
    assert!(
        !st.placements.contains_key(&fx.carrier),
        "husk's record retired"
    );
    assert!(!st.mounts.contains_key(&fx.carrier));
    assert!(st.is_quarantined(fx.mech) && st.is_quarantined(fx.mech2));
    assert_eq!(st.placements[&fx.mech2].revision, u64::MAX);
    assert_eq!(loc(&tw, fx.mech), Some(LIMBO));
    assert_eq!(loc(&tw, fx.mech2), Some(LIMBO));
    check_invariants(&tw);
    // Quarantined objects are held until staff recover them.
    tw.world_mut().move_to(GOD, LIMBO).unwrap();
    let t = say(&mut tw, "get Atlas");
    assert!(t.contains("is quarantined"), "{t}");
    let t = ok(&mut tw, "@spatial/quarantine");
    assert!(t.contains("Atlas") && t.contains("was destroyed"), "{t}");
    ok(&mut tw, "@spatial/recover Atlas");
    let t = say(&mut tw, "get Atlas");
    assert!(!t.contains("quarantined"), "{t}");
    // Persisted as deletes + quarantined placements.
    let ops = tw.take_module_ops();
    assert!(ops.iter().any(
        |o| matches!(&o.op, ModuleWrite::Del { key } if key == &format!("mounts:{}", fx.carrier.0))
    ));
    assert!(ops.iter().any(|o| matches!(&o.op, ModuleWrite::Del { key } if key == &format!("placement:{}", fx.carrier.0))));
}

#[test]
fn destroying_a_domain_host_programmatically_retires_it_and_quarantines_direct() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    let hq = tw
        .world_mut()
        .create(ObjKind::Thing, "HQ", GOD, Some(LIMBO))
        .unwrap();
    bind(&mut tw, hq, "sim.domain");
    ok(&mut tw, "@domain/create HQ=sim.passive,Field");
    ok(&mut tw, "@spatial/space Hangar=fixed:0");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    ok(&mut tw, "@spatial/place/interior Locust=Hangar");
    tw.world_mut().destroy(hq).unwrap();
    let mut out = Outbox::new();
    let mut fx_ = Effects::new();
    tw.engine_mut()
        .emit_world_event(&WorldEvent::ObjDestroyed { obj: hq }, &mut out, &mut fx_);
    let st = state(&tw);
    assert_eq!(st.domains[&DomainId(0)].status, DomainStatus::Retired);
    assert!(st.is_quarantined(fx.mech), "direct participant quarantined");
    assert!(!st.is_quarantined(fx.mech2), "interior occupant stays put");
    assert!(matches!(
        st.parent_of(fx.mech2),
        SpatialParent::Interior { .. }
    ));
    assert!(
        matches!(
            st.spaces[&fx.hangar].anchor,
            mush_sim::SpaceAnchor::Unanchored
        ),
        "fixed space unanchored"
    );
    assert_eq!(loc(&tw, fx.mech2), Some(fx.hangar));
    check_invariants(&tw);
}

#[test]
fn reboot_round_trip_reproduces_state_and_projections() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/space Hangar=fixed:0");
    ok(&mut tw, "@spatial/slot Hangar=bay,2");
    ok(&mut tw, "@spatial/mount Leopard=deck,1,exposed");
    ok(&mut tw, "@spatial/place/domain Leopard=0");
    ok(&mut tw, "@spatial/place/attached Atlas=Leopard,deck");
    ok(&mut tw, "@spatial/place/interior Locust=Hangar,bay");
    ok(&mut tw, "@spatial/unplace/keep Locust=parked");
    let kv = fold_kv(&mut tw);
    let (mut tw2, fx2) = reboot_with(kv);
    assert_eq!(
        state(&tw2),
        state(&tw),
        "records and indexes survive a reboot"
    );
    assert_eq!(loc(&tw2, fx2.carrier), Some(fx2.field));
    assert_eq!(loc(&tw2, fx2.mech), Some(fx2.field));
    assert_eq!(state(&tw2).next_domain_id, 1);
    check_invariants(&tw2);
    // The recovery ledger restored (SpatialState::eq ignores it, so check it).
    assert!(
        !state(&tw2).recovery.open_entries("recovery").is_empty(),
        "recovery ledger restored"
    );
    // The reboot is a no-op on the store when nothing was wrong.
    assert!(
        tw2.take_module_ops().is_empty(),
        "clean boot persists nothing"
    );
    // The allocator resumes: a brand-new host gets #1, never a reused id.
    let mars = tw2
        .world_mut()
        .create(ObjKind::Room, "Mars", GOD, None)
        .unwrap();
    bind(&mut tw2, mars, "sim.domain");
    let t = ok(&mut tw2, &format!("@domain/create #{}=sim.passive", mars.0));
    assert!(t.contains("Created domain #1"), "{t}");
    // And a room carrying only sim.interior is still refused as a host.
    let t = say(&mut tw2, "@domain/create Hangar=sim.passive");
    assert!(t.contains("must carry `sim.domain`"), "{t}");
    let _ = fx;
}

#[test]
fn legacy_nested_passive_values_load_exactly_without_an_eager_rewrite() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/space Hangar=fixed:0");
    ok(&mut tw, "@spatial/mount Leopard=deck,1,exposed");
    ok(&mut tw, r#"@spatial/place/domain Leopard=0,{"x":1}"#);
    ok(
        &mut tw,
        r#"@spatial/place/attached Atlas=Leopard,deck,{"dx":2}"#,
    );
    ok(&mut tw, r#"@spatial/place/domain Locust=0,{"x":3}"#);
    ok(&mut tw, "@spatial/unplace/keep Locust=parked");
    let mut kv = fold_kv(&mut tw);

    // Commands already write the shared v2 shape.
    let domain_doc = module_doc_mut(&mut kv, "domain:0");
    assert!(
        domain_doc["driver"].is_object(),
        "new domain driver is an exact definition reference: {domain_doc}"
    );
    assert!(
        domain_doc.get("schema_version").is_none(),
        "new domains do not emit the legacy split version"
    );
    domain_doc["driver"] = serde_json::json!("sim.passive");
    domain_doc["schema_version"] = serde_json::json!(1);

    let carrier_key = format!("placement:{}", fx.carrier.0);
    let carrier_doc = module_doc_mut(&mut kv, &carrier_key);
    let authored = &carrier_doc["parent"]["pose"]["posture"];
    assert!(
        authored["schema"].is_object(),
        "new rules value is exact-reference v2: {authored}"
    );
    assert!(
        authored.get("version").is_none(),
        "new value has no legacy top-level version"
    );
    carrier_doc["parent"]["pose"] = legacy_passive(serde_json::json!({ "x": 1 }));

    let attached_key = format!("placement:{}", fx.mech.0);
    module_doc_mut(&mut kv, &attached_key)["parent"]["local_pose"] =
        legacy_passive(serde_json::json!({ "dx": 2 }));

    let unplaced_key = format!("placement:{}", fx.mech2.0);
    module_doc_mut(&mut kv, &unplaced_key)["unplaced"]["previous"]["pose"] =
        legacy_passive(serde_json::json!({ "x": 3 }));

    let space_key = format!("space:{}", fx.hangar.0);
    let space_doc = module_doc_mut(&mut kv, &space_key);
    space_doc["anchor"]["anchor_pose"] = legacy_passive(serde_json::json!({ "x": 4 }));
    space_doc["egresses"]["bay"] = serde_json::json!({
        "label": "Bay doors",
        "to_pose": legacy_passive(serde_json::json!({ "x": 5 }))
    });

    let (mut tw2, fx2) = reboot_with(kv);
    let exact = mush_sim::passive_definition_ref();
    let st = state(&tw2);
    assert_eq!(
        st.domains[&DomainId(0)].driver,
        exact,
        "the contextual legacy domain tuple maps to the exact built-in definition"
    );
    let SpatialParent::Direct {
        pose: carrier_pose, ..
    } = st.parent_of(fx2.carrier)
    else {
        panic!("carrier did not load direct");
    };
    assert_eq!(carrier_pose.posture.schema(), &exact);
    assert_eq!(carrier_pose.posture.data(), &serde_json::json!({ "x": 1 }));
    let SpatialParent::Attached {
        local_pose: Some(local_pose),
        ..
    } = st.parent_of(fx2.mech)
    else {
        panic!("attachment did not load");
    };
    assert_eq!(local_pose.schema(), &exact);
    assert_eq!(local_pose.data(), &serde_json::json!({ "dx": 2 }));
    let previous = st.placements[&fx2.mech2]
        .unplaced
        .as_ref()
        .and_then(|unplaced| unplaced.previous.as_ref())
        .expect("kept previous parent");
    let SpatialParent::Direct {
        pose: previous_pose,
        ..
    } = previous
    else {
        panic!("previous direct placement did not load");
    };
    assert_eq!(previous_pose.posture.schema(), &exact);
    assert_eq!(previous_pose.posture.data(), &serde_json::json!({ "x": 3 }));
    let mush_sim::SpaceAnchor::FixedDomain {
        anchor_pose: Some(anchor_pose),
        ..
    } = &st.spaces[&fx2.hangar].anchor
    else {
        panic!("fixed anchor pose did not load");
    };
    assert_eq!(anchor_pose.schema(), &exact);
    assert_eq!(anchor_pose.data(), &serde_json::json!({ "x": 4 }));
    let egress_pose = st.spaces[&fx2.hangar].egresses["bay"]
        .to_pose
        .as_ref()
        .expect("egress pose loaded");
    assert_eq!(egress_pose.schema(), &exact);
    assert_eq!(egress_pose.data(), &serde_json::json!({ "x": 5 }));
    assert!(
        tw2.take_module_ops().is_empty(),
        "legacy compatibility reads do not rewrite authoritative docs"
    );
}

#[test]
fn unavailable_exact_rules_references_are_held_without_rewriting_raw_documents() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    ok(&mut tw, "@spatial/place/domain Locust=0");
    let mut kv = fold_kv(&mut tw);

    let atlas_key = format!("placement:{}", fx.mech.0);
    module_doc_mut(&mut kv, &atlas_key)["parent"]["pose"]["posture"]["schema"]["key"]["id"] =
        serde_json::json!("missing");

    let locust_key = format!("placement:{}", fx.mech2.0);
    let digest =
        module_doc_mut(&mut kv, &locust_key)["parent"]["pose"]["posture"]["schema"]["digest"]
            .as_str()
            .expect("definition digest")
            .to_string();
    let replacement = if digest.starts_with('0') { '1' } else { '0' };
    module_doc_mut(&mut kv, &locust_key)["parent"]["pose"]["posture"]["schema"]["digest"] =
        serde_json::json!(format!("{replacement}{}", &digest[1..]));

    let original_atlas = module_doc_mut(&mut kv, &atlas_key).clone();
    let original_locust = module_doc_mut(&mut kv, &locust_key).clone();
    let (mut tw2, fx2) = reboot_with(kv);
    assert!(
        state(&tw2).is_quarantined(fx2.mech),
        "a missing exact schema cannot activate"
    );
    assert!(
        state(&tw2).is_quarantined(fx2.mech2),
        "a known locator with a drifted digest cannot activate"
    );
    assert!(!state(&tw2).placements.contains_key(&fx2.mech));
    assert!(!state(&tw2).placements.contains_key(&fx2.mech2));
    assert_eq!(
        state(&tw2)
            .held_placement(fx2.mech)
            .map(|held| held.key.as_str()),
        Some(atlas_key.as_str())
    );
    assert_eq!(
        state(&tw2)
            .held_placement(fx2.mech2)
            .map(|held| held.key.as_str()),
        Some(locust_key.as_str())
    );
    let ops = tw2.take_module_ops();
    assert!(
        ops.is_empty(),
        "a boot containing only held exact-reference failures must emit no persistence: {ops:?}"
    );
    // The boot input values are the exact values a store retains when no op
    // targets their keys. Keep explicit copies so this test catches a future
    // accidental in-place repair before the outbox boundary as well.
    assert_eq!(
        original_atlas["parent"]["pose"]["posture"]["schema"]["key"]["id"],
        "missing"
    );
    assert_ne!(
        original_locust["parent"]["pose"]["posture"]["schema"]["digest"],
        serde_json::to_value(mush_sim::passive_definition_ref()).unwrap()["digest"]
    );
}

#[test]
#[should_panic(expected = "bad domain doc")]
fn unavailable_exact_domain_driver_fails_container_boot() {
    let mut tw = boot();
    let _fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    let mut kv = fold_kv(&mut tw);
    let domain = module_doc_mut(&mut kv, "domain:0");
    let digest = domain["driver"]["digest"]
        .as_str()
        .expect("definition digest")
        .to_string();
    let replacement = if digest.starts_with('0') { '1' } else { '0' };
    domain["driver"]["digest"] = serde_json::json!(format!("{replacement}{}", &digest[1..]));
    let _ = reboot_with(kv);
}

#[test]
fn legacy_dotted_schema_is_not_inferred_outside_the_owned_mapping() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    let mut kv = fold_kv(&mut tw);
    let key = format!("placement:{}", fx.mech.0);
    module_doc_mut(&mut kv, &key)["parent"]["pose"] = serde_json::json!({
        "schema": "some_other.pose",
        "version": 1,
        "data": null
    });
    let (tw2, fx2) = reboot_with(kv);
    assert!(
        state(&tw2).is_quarantined(fx2.mech),
        "unknown legacy schema is not owner-inferred"
    );
}

#[test]
fn boot_quarantines_dangling_placements_and_reconciles_drift() {
    let mut tw = boot();
    let _fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    ok(&mut tw, "@spatial/place/domain Locust=0");
    let mut kv = fold_kv(&mut tw);
    // Simulate a lost domain doc: both placements dangle.
    kv.retain(|(k, _)| k != "domain:0");
    let mut tw2 = boot();
    let fx2 = fixture(&mut tw2);
    // And simulate location drift on a record that will be quarantined.
    tw2.world_mut().move_to(fx2.mech, fx2.hangar).unwrap();
    tw2.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        ModuleBootData {
            kv,
            streams: HashMap::new(),
            ..Default::default()
        },
    )]));
    let st = state(&tw2);
    assert!(st.is_quarantined(fx2.mech) && st.is_quarantined(fx2.mech2));
    let u = st.placements[&fx2.mech].unplaced.as_ref().unwrap();
    assert!(u.reason.contains("domain #0 missing"), "{}", u.reason);
    assert!(
        matches!(u.previous, Some(SpatialParent::Direct { .. })),
        "recovery keeps the old parent"
    );
    assert_eq!(
        loc(&tw2, fx2.mech),
        Some(LIMBO),
        "quarantine projects to the recovery room"
    );
    check_invariants(&tw2);
    // The quarantine is persisted at boot.
    let ops = tw2.take_module_ops();
    assert!(ops.iter().any(|o| matches!(&o.op, ModuleWrite::JsonSet { key, value } if key == &format!("placement:{}", fx2.mech.0) && value["unplaced"]["quarantined"] == true)));

    // A valid placement whose object drifted is moved back, not quarantined.
    let mut tw = boot();
    let _fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    let kv = fold_kv(&mut tw);
    let mut tw3 = boot();
    let fx3 = fixture(&mut tw3);
    tw3.world_mut().move_to(fx3.mech, fx3.hangar).unwrap();
    tw3.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        ModuleBootData {
            kv,
            streams: HashMap::new(),
            ..Default::default()
        },
    )]));
    assert!(!state(&tw3).is_quarantined(fx3.mech));
    assert_eq!(
        loc(&tw3, fx3.mech),
        Some(fx3.field),
        "projection reconciled"
    );
    check_invariants(&tw3);
}

#[test]
#[should_panic(expected = "bad domain doc")]
fn corrupt_container_doc_fails_the_boot() {
    let mut tw = boot();
    let _fx = fixture(&mut tw);
    tw.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        ModuleBootData {
            kv: vec![(
                SmolStr::new("domain:0"),
                serde_json::json!({"garbage": true}),
            )],
            streams: HashMap::new(),
            ..Default::default()
        },
    )]));
}

#[test]
fn non_admins_are_refused() {
    let mut tw = boot();
    let _fx = fixture(&mut tw);
    let bob = tw.spawn_player("Bob");
    let t = texts(&tw.run(bob, "@domain"));
    assert!(t.contains("Permission denied"), "{t}");
    let t = texts(&tw.run(bob, "@spatial Atlas"));
    assert!(t.contains("Permission denied"), "{t}");
}

#[test]
fn show_tree_validate_and_log_report() {
    let mut tw = boot();
    let _fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/mount Leopard=deck,1,exposed");
    ok(&mut tw, "@spatial/place/domain Leopard=0");
    ok(&mut tw, "@spatial/place/attached Atlas=Leopard,deck");
    let t = ok(&mut tw, "@spatial Atlas");
    assert!(
        t.contains("attached to Leopard at deck")
            && t.contains("effective domain #0")
            && t.contains("DerivedExposed"),
        "{t}"
    );
    let t = ok(&mut tw, "@spatial/tree Leopard");
    assert!(t.contains("Leopard") && t.contains("  Atlas"), "{t}");
    let t = ok(&mut tw, "@spatial/validate");
    assert!(
        t.contains("Indexes: consistent")
            && t.contains("Projections: consistent")
            && t.contains("0 quarantined"),
        "{t}"
    );
    let t = ok(&mut tw, "@spatial/log");
    assert!(t.contains("\"op\":\"place\""), "{t}");
    let t = ok(&mut tw, "@domain");
    assert!(
        t.contains("#0 Active") && t.contains("2 local, 2 projected"),
        "{t}"
    );
    let t = ok(&mut tw, "@domain/status Field");
    assert!(t.contains("1 direct, 2 local, 2 projected"), "{t}");
    ok(&mut tw, "@domain/pause 0");
    let t = say(&mut tw, "@spatial/place/domain Locust=0");
    assert!(t.contains("is Paused and accepts no placements"), "{t}");
    ok(&mut tw, "@domain/resume 0");
    ok(&mut tw, "@spatial/place/domain Locust=0");
}

/// A small deterministic random-operation loop: every accepted or refused
/// operation must leave the indexes consistent, every projection honoured,
/// and the ancestor relation acyclic and bounded.
#[test]
fn random_operations_preserve_invariants() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    let extra: Vec<ObjId> = (0..4)
        .map(|i| {
            let id = tw
                .world_mut()
                .create(ObjKind::Thing, &format!("Unit{i}"), GOD, Some(LIMBO))
                .unwrap();
            bind(&mut tw, id, "sim.participant");
            id
        })
        .collect();
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/space Hangar=fixed:0");
    ok(&mut tw, "@spatial/slot Hangar=bay,2");
    for &u in [fx.mech, fx.mech2, fx.carrier].iter().chain(extra.iter()) {
        ok(&mut tw, &format!("@spatial/mount #{}=rack,2,exposed", u.0));
        ok(&mut tw, &format!("@spatial/mount #{}=hold,1", u.0));
    }
    let units: Vec<ObjId> = [fx.mech, fx.mech2, fx.carrier]
        .into_iter()
        .chain(extra)
        .collect();
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    let mut accepted = 0;
    for _ in 0..120 {
        let a = units[next(units.len())];
        let b = units[next(units.len())];
        let line = match next(6) {
            0 => format!("@spatial/place/domain #{}=0", a.0),
            1 => format!("@spatial/place/interior #{}=Hangar,bay", a.0),
            2 => format!("@spatial/place/attached #{}=#{},rack", a.0, b.0),
            3 => format!("@spatial/place/attached #{}=#{},hold", a.0, b.0),
            4 => format!("@spatial/unplace #{}", a.0),
            _ => format!("@spatial/unplace/keep #{}", a.0),
        };
        let before = state(&tw).clone();
        let events = tw.run(GOD, &line);
        let t = texts(&events);
        if t.contains("Placed") || t.contains("Unplaced") {
            accepted += 1;
        } else {
            // A refused transition must mutate nothing.
            assert!(
                has_error(&events),
                "neither accepted nor refused: {line} -> {t}"
            );
            assert_eq!(
                state(&tw),
                &before,
                "refused op mutated state: {line} -> {t}"
            );
        }
        check_invariants(&tw);
        let st = state(&tw);
        for &u in &units {
            assert_ok_ancestors(st, u);
            // No mount is ever over capacity (cap: rack 2, hold 1).
            assert!(st.attachments(u, &mush_sim::MountId::new("rack").unwrap()) <= 2);
            assert!(st.attachments(u, &mush_sim::MountId::new("hold").unwrap()) <= 1);
        }
    }
    assert!(
        accepted > 10,
        "the loop exercised real transitions ({accepted})"
    );
}

fn assert_ok_ancestors(st: &SpatialState, u: ObjId) {
    mush_sim::graph::ancestors(st, u).unwrap_or_else(|e| panic!("{u}: {e}"));
}

#[test]
fn depth_is_bounded_by_height_not_node_count() {
    let mut tw = boot();
    let _fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    // A deep attachment chain: each unit mounts the next.
    let chain: Vec<ObjId> = (0..9)
        .map(|i| {
            let id = tw
                .world_mut()
                .create(ObjKind::Thing, &format!("Chain{i}"), GOD, Some(LIMBO))
                .unwrap();
            bind(&mut tw, id, "sim.participant");
            id
        })
        .collect();
    for &u in &chain {
        ok(&mut tw, &format!("@spatial/mount #{}=m", u.0));
    }
    ok(&mut tw, &format!("@spatial/place/domain #{}=0", chain[0].0));
    for k in 1..8 {
        ok(
            &mut tw,
            &format!(
                "@spatial/place/attached #{}=#{},m",
                chain[k].0,
                chain[k - 1].0
            ),
        );
    }
    // The 9th link would exceed the maximum ancestor depth.
    let t = say(
        &mut tw,
        &format!("@spatial/place/attached #{}=#{},m", chain[8].0, chain[7].0),
    );
    assert!(t.contains("exceed the maximum spatial depth"), "{t}");
    assert!(!state(&tw).is_placed(chain[8]));
    check_invariants(&tw);

    // Wide-but-shallow: a carrier with six riders placed at depth 2 is fine.
    // The old node-count depth check would have refused this as TooDeep.
    let a = tw
        .world_mut()
        .create(ObjKind::Thing, "WideA", GOD, Some(LIMBO))
        .unwrap();
    let b = tw
        .world_mut()
        .create(ObjKind::Thing, "WideB", GOD, Some(LIMBO))
        .unwrap();
    let carrier = tw
        .world_mut()
        .create(ObjKind::Thing, "Wide", GOD, Some(LIMBO))
        .unwrap();
    for u in [a, b, carrier] {
        bind(&mut tw, u, "sim.participant");
    }
    ok(&mut tw, &format!("@spatial/mount #{}=m", a.0));
    ok(&mut tw, &format!("@spatial/mount #{}=m", b.0));
    ok(
        &mut tw,
        &format!("@spatial/mount #{}=bay,6,exposed", carrier.0),
    );
    ok(&mut tw, &format!("@spatial/place/domain #{}=0", a.0));
    ok(
        &mut tw,
        &format!("@spatial/place/attached #{}=#{},m", b.0, a.0),
    );
    for i in 0..6 {
        let id = tw
            .world_mut()
            .create(ObjKind::Thing, &format!("Rider{i}"), GOD, Some(LIMBO))
            .unwrap();
        bind(&mut tw, id, "sim.participant");
        ok(
            &mut tw,
            &format!("@spatial/place/attached #{}=#{},bay", id.0, carrier.0),
        );
    }
    // b sits at ancestor depth 2; the loaded carrier (node count 7) still fits.
    ok(
        &mut tw,
        &format!("@spatial/place/attached #{}=#{},m", carrier.0, b.0),
    );
    assert_eq!(state(&tw).children_of(carrier).len(), 6);
    check_invariants(&tw);
}

#[test]
fn unspace_and_unmount_happy_paths() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/space Hangar=fixed:0");
    ok(&mut tw, "@spatial/mount Leopard=deck,1");
    let _ = tw.take_module_ops();
    // Unspace an empty space: the record is gone and the doc DEL'd.
    ok(&mut tw, "@spatial/unspace Hangar");
    assert!(!state(&tw).spaces.contains_key(&fx.hangar));
    let ops = tw.take_module_ops();
    assert!(ops.iter().any(
        |o| matches!(&o.op, ModuleWrite::Del { key } if key == &format!("space:{}", fx.hangar.0))
    ));
    // Unmount the last mount: the mounts doc is DEL'd.
    ok(&mut tw, "@spatial/unmount Leopard=deck");
    assert!(!state(&tw).mounts.contains_key(&fx.carrier));
    let ops = tw.take_module_ops();
    assert!(ops.iter().any(
        |o| matches!(&o.op, ModuleWrite::Del { key } if key == &format!("mounts:{}", fx.carrier.0))
    ));
    check_invariants(&tw);
}

#[test]
fn store_round_trip_through_in_memory_store() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/space Hangar=fixed:0");
    ok(&mut tw, "@spatial/mount Leopard=deck,1,exposed");
    ok(&mut tw, "@spatial/place/domain Leopard=0");
    ok(&mut tw, "@spatial/place/attached Atlas=Leopard,deck");
    ok(&mut tw, "@spatial/place/interior Locust=Hangar");
    let ops = tw.take_module_ops();
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
    let kv = futures::executor::block_on(async { store.module_json_list("sim").await.unwrap() });
    let boot_data = ModuleBootData {
        kv: kv.into_iter().map(|(k, v)| (SmolStr::new(k), v)).collect(),
        streams: HashMap::new(),
        ..Default::default()
    };
    let mut tw2 = boot();
    let fx2 = fixture(&mut tw2);
    tw2.modules_loaded(HashMap::from([(SmolStr::new("sim"), boot_data)]));
    assert_eq!(
        state(&tw2),
        state(&tw),
        "state survives a real store round trip"
    );
    assert_eq!(loc(&tw2, fx2.mech), Some(fx2.field));
    assert_eq!(loc(&tw2, fx2.carrier), Some(fx2.field));
    check_invariants(&tw2);
    let _ = fx;
}

#[test]
fn quarantine_uses_the_configured_recovery_room() {
    // The recovery room is the first object created, so its id is fixed.
    let recov = ObjId(2);
    let mut tw =
        TestWorld::with_modules(vec![Box::new(SimulationModule::with_config(SimConfig {
            recovery_room: recov,
        }))]);
    let recovery = tw
        .world_mut()
        .create(ObjKind::Room, "Recovery", GOD, None)
        .unwrap();
    assert_eq!(recovery, recov, "deterministic id");
    let field = tw
        .world_mut()
        .create(ObjKind::Room, "Field", GOD, None)
        .unwrap();
    let hq = tw
        .world_mut()
        .create(ObjKind::Thing, "HQ", GOD, Some(LIMBO))
        .unwrap();
    let mech = tw
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, Some(LIMBO))
        .unwrap();
    bind(&mut tw, hq, "sim.domain");
    bind(&mut tw, mech, "sim.participant");
    tw.run(
        GOD,
        &format!("@domain/create #{}=sim.passive,#{}", hq.0, field.0),
    );
    tw.run(GOD, &format!("@spatial/place/domain #{}=0", mech.0));
    assert_eq!(loc(&tw, mech), Some(field));
    // Destroying the domain host quarantines the mech into the custom room.
    tw.world_mut().destroy(hq).unwrap();
    let mut out = Outbox::new();
    let mut fxx = Effects::new();
    tw.engine_mut()
        .emit_world_event(&WorldEvent::ObjDestroyed { obj: hq }, &mut out, &mut fxx);
    assert!(state(&tw).is_quarantined(mech));
    assert_eq!(
        loc(&tw, mech),
        Some(recovery),
        "quarantine used the configured room"
    );
}

#[test]
fn boot_holds_unparseable_placement_docs_without_clobbering_them() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    let mut kv = fold_kv(&mut tw);
    // Corrupt Atlas's placement doc (Atlas exists in the reboot fixture).
    let atlas_key = format!("placement:{}", fx.mech.0);
    for (k, v) in kv.iter_mut() {
        if k.as_str() == atlas_key {
            *v = serde_json::json!({ "broken": true });
        }
    }
    // A garbage doc for a non-existent object id.
    kv.push((
        SmolStr::new("placement:9999"),
        serde_json::json!("nonsense"),
    ));
    let atlas_raw = kv
        .iter()
        .find(|(key, _)| key == &atlas_key)
        .map(|(_, value)| value.clone())
        .expect("corrupt Atlas doc remains in boot input");
    let phantom_raw = kv
        .iter()
        .find(|(key, _)| key == "placement:9999")
        .map(|(_, value)| value.clone())
        .expect("phantom doc remains in boot input");
    let (mut tw2, fx2) = reboot_with(kv);
    // Atlas: held by canonical key, mechanically inactive, and projected into
    // the recovery room without manufacturing a replacement PlacementRecord.
    assert!(state(&tw2).is_quarantined(fx2.mech));
    assert!(
        state(&tw2)
            .held_placement(fx2.mech)
            .expect("held placement metadata")
            .reason
            .contains("missing field")
    );
    assert!(!state(&tw2).placements.contains_key(&fx2.mech));
    assert_eq!(loc(&tw2, fx2.mech), Some(LIMBO));
    let shown = ok(&mut tw2, "@spatial Atlas");
    assert!(
        shown.contains("HELD") && shown.contains(&atlas_key),
        "{shown}"
    );
    let refused = say(&mut tw2, "@spatial/recover Atlas");
    assert!(refused.contains("repair or migrate"), "{refused}");
    let refused = say(&mut tw2, "@spatial/place/domain Atlas=0");
    assert!(refused.contains("held unreadable placement"), "{refused}");
    // A corrupt doc for a missing object is held too: boot cannot infer that
    // deleting the only evidence is safe.
    assert!(!state(&tw2).placements.contains_key(&ObjId(9999)));
    assert!(state(&tw2).held_placement(ObjId(9999)).is_some());
    let ops = tw2.take_module_ops();
    assert!(
        ops.is_empty(),
        "a boot containing only unreadable placements must emit no persistence: {ops:?}"
    );
    assert_eq!(atlas_raw, serde_json::json!({ "broken": true }));
    assert_eq!(phantom_raw, serde_json::json!("nonsense"));
    check_invariants(&tw2);
}

#[test]
fn refused_new_space_leaves_no_phantom_record() {
    // A refused @spatial/space on a NEW room must not leave a record behind.
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@domain/retire 0");
    let before = state(&tw).clone();
    let _ = tw.take_module_ops();
    let t = say(&mut tw, "@spatial/space Hangar=fixed:0");
    assert!(t.contains("Retired"), "{t}");
    assert!(
        !state(&tw).spaces.contains_key(&fx.hangar),
        "no phantom space record"
    );
    assert_eq!(state(&tw), &before, "refused define mutated nothing");
    assert!(
        tw.take_module_ops().is_empty(),
        "refused define persisted nothing"
    );
}

#[test]
fn non_ascii_anchor_input_is_rejected_not_panicking() {
    let mut tw = boot();
    let _fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    // A multi-byte char straddling the "fixed:"/"moving:" prefix boundary must
    // produce a normal error, never a char-boundary panic on the world thread.
    for bad in ["Montré", "café", "movinég:x", "fixé:0"] {
        let t = say(&mut tw, &format!("@spatial/space Hangar={bad}"));
        assert!(t.contains("Anchor:"), "{bad:?} -> {t}");
    }
    assert!(!state(&tw).spaces.contains_key(&_fx.hangar));
}

#[test]
fn subtree_and_depth_walk_includes_domain_directs_not_just_placement_children() {
    // A Thing that BOTH hosts its own domain AND is itself a placed participant
    // — the case where `children` (placement-only) under-counts descendants.
    let mut tw = boot();
    let fx = fixture(&mut tw);
    let hq = tw
        .world_mut()
        .create(ObjKind::Thing, "HQ", GOD, Some(LIMBO))
        .unwrap();
    let stage = tw
        .world_mut()
        .create(ObjKind::Room, "Stage", GOD, None)
        .unwrap();
    bind(&mut tw, hq, "sim.domain");
    bind(&mut tw, hq, "sim.participant");
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(
        &mut tw,
        &format!("@domain/create #{}=sim.passive,#{}", hq.0, stage.0),
    );
    ok(&mut tw, "@spatial/place/domain Atlas=1"); // mech Direct in HQ's own domain
    ok(&mut tw, &format!("@spatial/place/domain #{}=0", hq.0)); // HQ Direct in the outer domain
    let st = state(&tw);
    // The complete ancestry-inverse sees the mech as a descendant of HQ...
    assert!(
        st.descendant_edges(hq).contains(&fx.mech),
        "domain directs are descendants of the host"
    );
    assert!(st.subtree(hq).contains(&fx.mech), "subtree is complete");
    assert!(
        st.subtree_height(hq) >= 1,
        "height counts the hosted domain's directs"
    );
    // ...but `children_of` stays the placement-only dependents index.
    assert!(!st.children_of(hq).contains(&fx.mech));
    check_invariants(&tw);
}

#[test]
fn exhausted_domain_and_placement_revisions_refuse_without_mutation() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    {
        let state = tw
            .world_mut()
            .module_state_mut::<SpatialState>()
            .expect("sim state");
        state.domains.get_mut(&DomainId(0)).unwrap().revision = u64::MAX;
        state.placements.get_mut(&fx.mech).unwrap().revision = u64::MAX;
    }
    let before = state(&tw).clone();
    let before_location = loc(&tw, fx.mech);
    let _ = tw.take_module_ops();

    let status = say(&mut tw, "@domain/pause 0");
    assert!(status.contains("exhausted its revision"), "{status}");
    let unplace = say(&mut tw, "@spatial/unplace Atlas");
    assert!(unplace.contains("exhausted its placement"), "{unplace}");

    assert_eq!(state(&tw), &before, "refused revisions mutated sim state");
    assert_eq!(loc(&tw, fx.mech), before_location);
    assert!(
        tw.take_module_ops().is_empty(),
        "refused revisions emitted persistence"
    );
}

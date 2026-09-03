//! Phase B cut 2 acceptance: stations → command subjects, embark/egress/
//! attach/detach/carrier-transfer, the player `egress`, the occupied-station
//! destroy veto, and reboot republication — driven through the real Engine via
//! `TestWorld` with the `SimulationModule` plus a toy unit behavior.

use std::collections::HashMap;
use std::sync::Arc;

use mush_core::{
    ArgShape, Audience, BehaviorBinding, BehaviorCommand, BehaviorId, BehaviorSpec, BehaviorTarget,
    CmdError, CommandReach, CommandSpec, Ctx, Effects, Flags, GOD, GameModule, LIMBO, ModError,
    ModuleBootData, ModuleWrite, ObjId, ObjKind, Outbox, Registrar, WorldEvent,
};
use mush_proto::OutputEvent;
use mush_sim::{Presence, SimulationModule, SpatialParent, SpatialState};
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

// ---- a toy unit behavior: one Operated-only command, the assignment link is
// the only path to it (the ruleset-tacsim contract) ----------------------

struct ToyUnitModule;

fn fire(ctx: &mut Ctx<'_>, t: &BehaviorTarget) -> Result<(), CmdError> {
    let actor = ctx.actor;
    ctx.out
        .sys_to_actor(actor, "toy", format!("BANG from {}", t.subject));
    Ok(())
}

impl GameModule for ToyUnitModule {
    fn name(&self) -> &'static str {
        "toy"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let mut spec = BehaviorSpec::new(BehaviorId::new("toy.unit").unwrap());
        spec.allowed_kinds = vec![ObjKind::Thing, ObjKind::Player, ObjKind::Npc];
        spec.commands = vec![
            BehaviorCommand::new(
                CommandSpec {
                    name: "fire".into(),
                    shape: ArgShape::None,
                    help: "fire".into(),
                    ..CommandSpec::default()
                },
                Arc::new(fire),
            )
            .with_reach(vec![CommandReach::Operated]),
        ];
        reg.behavior(spec)?;
        Ok(())
    }
}

// ---- harness -----------------------------------------------------------

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(SimulationModule::new()),
        Box::new(ToyUnitModule),
    ])
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_error(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
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

fn subjects(tw: &TestWorld, actor: ObjId) -> Vec<ObjId> {
    tw.world()
        .command_subjects_of(actor)
        .iter()
        .map(|l| l.subject)
        .collect()
}

/// Deterministic fixture: Field(#2 domain host), Bay(#3 sim.interior),
/// Leopard(#4 carrier), Atlas(#5 mech, toy.unit), Elemental(#6 BA), Grunt(#7).
struct Fx {
    field: ObjId,
    bay: ObjId,
    carrier: ObjId,
    mech: ObjId,
    ba: ObjId,
    grunt: ObjId,
    pilot: ObjId,
}

fn fixture(tw: &mut TestWorld) -> Fx {
    let field = tw
        .world_mut()
        .create(ObjKind::Room, "Field", GOD, None)
        .unwrap();
    let bay = tw
        .world_mut()
        .create(ObjKind::Room, "Bay", GOD, None)
        .unwrap();
    let carrier = tw
        .world_mut()
        .create(ObjKind::Thing, "Leopard", GOD, Some(LIMBO))
        .unwrap();
    let mech = tw
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, Some(LIMBO))
        .unwrap();
    let ba = tw
        .world_mut()
        .create(ObjKind::Thing, "Elemental", GOD, Some(LIMBO))
        .unwrap();
    let grunt = tw
        .world_mut()
        .create(ObjKind::Thing, "Grunt", GOD, Some(LIMBO))
        .unwrap();
    bind(tw, field, "sim.domain");
    bind(tw, bay, "sim.interior");
    for u in [carrier, mech, ba, grunt] {
        bind(tw, u, "sim.participant");
    }
    bind(tw, mech, "toy.unit");
    // A durable pilot Player (#8) so assignments survive a reboot.
    let pilot = tw
        .world_mut()
        .create(ObjKind::Player, "Ann", GOD, Some(LIMBO))
        .unwrap();
    Fx {
        field,
        bay,
        carrier,
        mech,
        ba,
        grunt,
        pilot,
    }
}

fn sub(line: &str) -> String {
    let mut s = line.to_string();
    for (name, id) in [
        ("Field", 2),
        ("Bay", 3),
        ("Leopard", 4),
        ("Atlas", 5),
        ("Elemental", 6),
        ("Grunt", 7),
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

fn say_as(tw: &mut TestWorld, who: ObjId, line: &str) -> String {
    texts(&tw.run(who, &sub(line)))
}
fn say(tw: &mut TestWorld, line: &str) -> String {
    say_as(tw, GOD, line)
}
fn ok(tw: &mut TestWorld, line: &str) -> String {
    let events = tw.run(GOD, &sub(line));
    let t = texts(&events);
    assert!(!has_error(&events), "{line:?} refused: {t}");
    t
}

fn fold_kv(tw: &mut TestWorld) -> Vec<(SmolStr, serde_json::Value)> {
    let mut kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    for op in tw.take_module_ops() {
        if op.ns != "sim" {
            continue;
        }
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
        },
    )]));
    (tw2, fx)
}

/// Domain #0 on Field, the carrier placed direct, the bay riding it, and the
/// mech embarked inside the bay. Returns nothing extra — ids are on `Fx`.
fn dropship_setup(tw: &mut TestWorld) {
    ok(tw, "@domain/create Field=sim.passive");
    ok(tw, "@spatial/place/domain Leopard=0");
    ok(tw, "@spatial/space Bay=moving:Leopard");
    ok(tw, "@spatial/place/interior Atlas=Bay");
}

// ----------------------------------------------------------------------

#[test]
fn pilot_station_grants_only_the_pilot_unit_commands() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    let ann = fx.pilot;
    let bob = tw.spawn_player("Bob");
    // No one can fire the mech until assigned.
    assert!(texts(&tw.run(ann, "fire")).contains("Huh?"), "no link yet");
    ok(&mut tw, "@spatial/station Atlas=pilot,pilot");
    ok(&mut tw, &format!("@spatial/assign Atlas=pilot,#{}", ann.0));
    assert_eq!(
        subjects(&tw, ann),
        vec![fx.mech],
        "pilot holds the unit as a command subject"
    );
    assert!(
        texts(&tw.run(ann, "fire")).contains(&format!("BANG from {}", fx.mech)),
        "pilot fires"
    );
    // Bob (no station) cannot.
    assert!(
        texts(&tw.run(bob, "fire")).contains("Huh?"),
        "non-pilot cannot fire"
    );
    // Unassigning revokes it.
    ok(&mut tw, "@spatial/unassign Atlas=pilot");
    assert!(subjects(&tw, ann).is_empty());
    assert!(texts(&tw.run(ann, "fire")).contains("Huh?"), "revoked");
}

#[test]
fn embark_into_a_bay_suppresses_presence() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    dropship_setup(&mut tw);
    assert_eq!(
        loc(&tw, fx.carrier),
        Some(fx.field),
        "carrier presents into the domain room"
    );
    assert_eq!(
        loc(&tw, fx.mech),
        Some(fx.bay),
        "projects into the bay room"
    );
    let st = state(&tw);
    assert_eq!(mush_sim::presence(st, fx.mech), Some(Presence::Suppressed));
    assert_eq!(
        mush_sim::effective_domain(st, fx.mech),
        Some(mush_sim::DomainId(0)),
        "locality via the moving anchor"
    );
    assert!(
        !st.derived
            .projected
            .get(&mush_sim::DomainId(0))
            .is_some_and(|s| s.contains(&fx.mech)),
        "no independent projection"
    );
    assert!(st.derived.local_objects[&mush_sim::DomainId(0)].contains(&fx.mech));
}

#[test]
fn carrier_transfer_carries_the_bay_occupant_then_egress() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    dropship_setup(&mut tw);
    let field2 = tw
        .world_mut()
        .create(ObjKind::Room, "Field2", GOD, None)
        .unwrap();
    bind(&mut tw, field2, "sim.domain");
    ok(
        &mut tw,
        &format!("@domain/create #{}=sim.passive", field2.0),
    );
    // Transfer the carrier to domain #1: the mech's placement record is
    // untouched but its effective domain follows.
    ok(&mut tw, "@spatial/place/domain Leopard=1");
    let st = state(&tw);
    assert!(
        matches!(st.parent_of(fx.mech), SpatialParent::Interior { .. }),
        "record unchanged"
    );
    assert_eq!(
        mush_sim::effective_domain(st, fx.mech),
        Some(mush_sim::DomainId(1)),
        "domain followed"
    );
    assert_eq!(
        loc(&tw, fx.mech),
        Some(fx.bay),
        "still physically in the bay"
    );
    // Egress the mech onto the new domain (staff).
    ok(&mut tw, "@spatial/egress Atlas");
    let st = state(&tw);
    assert!(matches!(
        st.parent_of(fx.mech),
        SpatialParent::Direct {
            domain: mush_sim::DomainId(1),
            ..
        }
    ));
    assert_eq!(
        loc(&tw, fx.mech),
        Some(field2),
        "landed in the new domain's presentation room"
    );
    assert_eq!(mush_sim::presence(st, fx.mech), Some(Presence::Independent));
}

#[test]
fn attach_battle_armor_is_exposed() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    ok(&mut tw, "@spatial/mount Atlas=ba,2,exposed");
    ok(&mut tw, "@spatial/place/attached Elemental=Atlas,ba");
    let st = state(&tw);
    assert_eq!(
        mush_sim::presence(st, fx.ba),
        Some(Presence::DerivedExposed)
    );
    assert_eq!(
        loc(&tw, fx.ba),
        Some(fx.field),
        "exposed: alongside the host"
    );
    assert!(st.derived.projected[&mush_sim::DomainId(0)].contains(&fx.ba));
    // Detach: the BA becomes the host's sibling, Direct in the domain.
    ok(&mut tw, "@spatial/detach Elemental");
    let st = state(&tw);
    assert!(matches!(
        st.parent_of(fx.ba),
        SpatialParent::Direct {
            domain: mush_sim::DomainId(0),
            ..
        }
    ));
    assert_eq!(st.children_of(fx.mech).len(), 0, "no longer attached");
}

#[test]
fn infantry_cargo_has_locality_but_no_controls() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    dropship_setup(&mut tw);
    ok(&mut tw, "@spatial/slot Bay=troop,4");
    ok(&mut tw, "@spatial/place/interior Grunt=Bay,troop");
    let st = state(&tw);
    assert_eq!(
        mush_sim::effective_domain(st, fx.grunt),
        Some(mush_sim::DomainId(0))
    );
    assert_eq!(mush_sim::presence(st, fx.grunt), Some(Presence::Suppressed));
    // Cargo has no station, hence no command subject and no unit commands.
    assert!(subjects(&tw, fx.grunt).is_empty());
    // A grunt bound to toy.unit still cannot fire without a link.
    bind(&mut tw, fx.grunt, "toy.unit");
    assert!(texts(&tw.run(fx.grunt, "fire")).contains("Huh?"));
}

#[test]
fn player_egress_leaves_the_space_and_bystanders_cannot() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    dropship_setup(&mut tw);
    let ann = fx.pilot;
    bind(&mut tw, ann, "sim.participant");
    ok(&mut tw, &format!("@spatial/place/interior #{}=Bay", ann.0));
    assert_eq!(loc(&tw, ann), Some(fx.bay));
    // A bystander standing in the bay who is NOT placed cannot egress.
    let bob = tw.spawn_player("Bob");
    tw.world_mut().move_to(bob, fx.bay).unwrap();
    assert!(texts(&tw.run(bob, "egress")).contains("not inside anything"));
    // Ann egresses onto the carrier's domain.
    let t = texts(&tw.run(ann, "egress"));
    assert!(t.contains("You egress"), "{t}");
    let st = state(&tw);
    assert!(matches!(
        st.parent_of(ann),
        SpatialParent::Direct {
            domain: mush_sim::DomainId(0),
            ..
        }
    ));
    assert_eq!(loc(&tw, ann), Some(fx.field));
    // `disembark` is an alias.
    ok(&mut tw, &format!("@spatial/place/interior #{}=Bay", ann.0));
    assert!(texts(&tw.run(ann, "disembark")).contains("You egress"));
}

#[test]
fn occupied_station_refuses_destroy() {
    let mut tw = boot();
    let _fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    let ann = _fx.pilot;
    ok(&mut tw, "@spatial/station Atlas=pilot,pilot");
    ok(&mut tw, &format!("@spatial/assign Atlas=pilot,#{}", ann.0));
    let t = say(&mut tw, "@destroy Atlas");
    assert!(t.contains("crewed station(s)"), "{t}");
    ok(&mut tw, "@spatial/unassign Atlas=pilot");
    ok(&mut tw, "@spatial/unstation Atlas=pilot");
    // With the station gone, destruction is allowed.
    let t = say(&mut tw, "@destroy Atlas");
    assert!(!t.contains("crewed"), "{t}");
    assert!(
        tw.world()
            .get(_fx.mech)
            .is_none_or(|o| o.flags.contains(Flags::GOING)),
        "destroyed"
    );
}

#[test]
fn destroying_a_crewed_unit_programmatically_drops_records_and_links() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    let ann = fx.pilot;
    ok(&mut tw, "@spatial/station Atlas=pilot,pilot");
    ok(&mut tw, &format!("@spatial/assign Atlas=pilot,#{}", ann.0));
    assert_eq!(subjects(&tw, ann), vec![fx.mech]);
    let _ = tw.take_module_ops();
    // Combat kill: World::destroy bypasses the veto by design.
    tw.world_mut().destroy(fx.mech).unwrap();
    let mut out = Outbox::new();
    let mut fxx = Effects::new();
    tw.engine_mut().emit_world_event(
        &WorldEvent::ObjDestroyed { obj: fx.mech },
        &mut out,
        &mut fxx,
    );
    let st = state(&tw);
    assert!(
        !st.stations.contains_key(&fx.mech) && !st.assignments.contains_key(&fx.mech),
        "records gone"
    );
    assert!(subjects(&tw, ann).is_empty(), "link scrubbed");
    // Persisted deletes.
    let ops = tw.take_module_ops();
    assert!(ops.iter().any(
        |o| matches!(&o.op, ModuleWrite::Del { key } if key == &format!("stations:{}", fx.mech.0))
    ));
    assert!(ops.iter().any(
        |o| matches!(&o.op, ModuleWrite::Del { key } if key == &format!("assignment:{}", fx.mech.0))
    ));
}

#[test]
fn unplacing_a_unit_revokes_its_crew_links() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    let ann = fx.pilot;
    ok(&mut tw, "@spatial/station Atlas=pilot,pilot");
    ok(&mut tw, &format!("@spatial/assign Atlas=pilot,#{}", ann.0));
    assert_eq!(subjects(&tw, ann), vec![fx.mech]);
    // Unplaced unit grants no control.
    ok(&mut tw, "@spatial/unplace Atlas");
    assert!(
        subjects(&tw, ann).is_empty(),
        "unplaced unit does not grant control"
    );
    assert!(
        state(&tw).assignments.contains_key(&fx.mech),
        "assignment survived the unplace"
    );
    // Re-placing restores the crew link.
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    assert_eq!(
        subjects(&tw, ann),
        vec![fx.mech],
        "re-placing restores control"
    );
}

#[test]
fn reboot_preserves_stations_assignments_and_republishes_links() {
    let mut tw = boot();
    let _fx = fixture(&mut tw);
    dropship_setup(&mut tw);
    let ann = _fx.pilot;
    ok(&mut tw, "@spatial/station Atlas=pilot,pilot");
    ok(&mut tw, &format!("@spatial/assign Atlas=pilot,#{}", ann.0));
    let kv = fold_kv(&mut tw);
    let (mut tw2, fx2) = reboot_with(kv);
    // The pilot is part of the fixture, so she exists BEFORE modules_loaded and
    // the boot republish reattaches her link with no re-assignment.
    assert_eq!(state(&tw2), state(&tw), "records + indexes survive reboot");
    assert!(
        state(&tw2).stations.contains_key(&fx2.mech)
            && state(&tw2).assignments.contains_key(&fx2.mech)
    );
    assert_eq!(
        subjects(&tw2, fx2.pilot),
        vec![fx2.mech],
        "control link republished at boot"
    );
    assert!(
        texts(&tw2.run(fx2.pilot, "fire")).contains("BANG"),
        "pilot fires after reboot"
    );
}

#[test]
fn boot_drops_assignments_whose_actor_or_host_is_gone() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    let ann = fx.pilot;
    ok(&mut tw, "@spatial/station Atlas=pilot,pilot");
    ok(&mut tw, &format!("@spatial/assign Atlas=pilot,#{}", ann.0));
    let mut kv = fold_kv(&mut tw);
    // Point the assignment at a non-existent actor id.
    for (k, v) in kv.iter_mut() {
        if k.as_str() == format!("assignment:{}", fx.mech.0)
            && let Some(seats) = v.get_mut("seats").and_then(|s| s.as_object_mut())
        {
            for seat in seats.values_mut() {
                seat["actor"] = serde_json::json!(9999);
            }
        }
    }
    let (tw2, fx2) = reboot_with(kv);
    // The dangling seat is dropped; no assignment survives.
    assert!(
        !state(&tw2).assignments.contains_key(&fx2.mech),
        "dangling assignment dropped at boot"
    );
}

#[test]
fn control_links_are_per_unit_at_highest_role_and_survive_partial_unassign() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    let mech2 = tw
        .world_mut()
        .create(ObjKind::Thing, "Locust", GOD, Some(LIMBO))
        .unwrap();
    bind(&mut tw, mech2, "sim.participant");
    bind(&mut tw, mech2, "toy.unit");
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    ok(&mut tw, &format!("@spatial/place/domain #{}=0", mech2.0));
    let ann = fx.pilot;
    // Two stations on Atlas: the actor's single link takes the higher role.
    ok(&mut tw, "@spatial/station Atlas=gun,gunner");
    ok(&mut tw, "@spatial/station Atlas=cockpit,pilot");
    ok(&mut tw, &format!("@spatial/assign Atlas=gun,#{}", ann.0));
    ok(
        &mut tw,
        &format!("@spatial/assign Atlas=cockpit,#{}", ann.0),
    );
    assert_eq!(
        subjects(&tw, ann),
        vec![fx.mech],
        "one link per unit even with two stations"
    );
    assert_eq!(
        tw.world().command_subjects_of(ann)[0].relation.as_str(),
        "pilot",
        "highest role wins"
    );
    // Also crew mech2; Ann holds links to both units.
    ok(
        &mut tw,
        &format!("@spatial/station #{}=cockpit,pilot", mech2.0),
    );
    ok(
        &mut tw,
        &format!("@spatial/assign #{}=cockpit,#{}", mech2.0, ann.0),
    );
    let mut subs = subjects(&tw, ann);
    subs.sort();
    let mut want = vec![fx.mech, mech2];
    want.sort();
    assert_eq!(subs, want, "links to both units");
    // Unassigning one Atlas station keeps the pilot link to Atlas AND mech2.
    ok(&mut tw, "@spatial/unassign Atlas=gun");
    let mut subs = subjects(&tw, ann);
    subs.sort();
    assert_eq!(subs, want, "partial unassign keeps the survivors");
    // Redefining the surviving station's role updates the link priority in place.
    ok(&mut tw, "@spatial/station Atlas=cockpit,gunner");
    assert_eq!(
        tw.world()
            .command_subjects_of(ann)
            .iter()
            .find(|l| l.subject == fx.mech)
            .unwrap()
            .relation
            .as_str(),
        "gunner",
        "role redefine refreshes the link"
    );
    // Fully unassign Atlas: only the mech2 link remains.
    ok(&mut tw, "@spatial/unassign Atlas=cockpit");
    assert_eq!(subjects(&tw, ann), vec![mech2]);
}

#[test]
fn destroying_a_presentation_room_quarantines_crewed_directs_and_revokes_control() {
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
    ok(
        &mut tw,
        &format!("@domain/create #{}=sim.passive,#{}", hq.0, stage.0),
    );
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    let ann = fx.pilot;
    ok(&mut tw, "@spatial/station Atlas=pilot,pilot");
    ok(&mut tw, &format!("@spatial/assign Atlas=pilot,#{}", ann.0));
    assert_eq!(subjects(&tw, ann), vec![fx.mech]);
    // Programmatic destroy of the SEPARATE presentation room (bypasses the veto).
    tw.world_mut().destroy(stage).unwrap();
    let mut out = Outbox::new();
    let mut fxx = Effects::new();
    tw.engine_mut()
        .emit_world_event(&WorldEvent::ObjDestroyed { obj: stage }, &mut out, &mut fxx);
    assert!(
        state(&tw).is_quarantined(fx.mech),
        "direct quarantined when its presentation room dies"
    );
    assert!(
        subjects(&tw, ann).is_empty(),
        "a quarantined unit grants no control"
    );
}

#[test]
fn egress_refuses_unanchored_and_reports_unknown_egress() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    // An unanchored interior: egress has nowhere to go.
    ok(&mut tw, "@spatial/space Bay=unanchored");
    ok(&mut tw, "@spatial/place/interior Atlas=Bay");
    let t = say(&mut tw, "@spatial/egress Atlas");
    assert!(t.contains("unanchored"), "{t}");
    // Anchor the bay to the domain; an unknown egress id is reported.
    ok(&mut tw, "@spatial/space Bay=fixed:0");
    let t = say(&mut tw, "@spatial/egress Atlas=doors");
    assert!(t.contains("No egress `doors`"), "{t}");
    // No-arg egress lands the unit Direct in the domain.
    ok(&mut tw, "@spatial/egress Atlas");
    assert!(matches!(
        state(&tw).parent_of(fx.mech),
        SpatialParent::Direct {
            domain: mush_sim::DomainId(0),
            ..
        }
    ));
}

#[test]
fn boot_drops_assignment_whose_station_is_gone() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    ok(&mut tw, "@spatial/station Atlas=pilot,pilot");
    ok(
        &mut tw,
        &format!("@spatial/assign Atlas=pilot,#{}", fx.pilot.0),
    );
    let mut kv = fold_kv(&mut tw);
    // The station definition vanishes but the assignment doc remains.
    kv.retain(|(k, _)| k.as_str() != format!("stations:{}", fx.mech.0));
    let (tw2, fx2) = reboot_with(kv);
    assert!(
        !state(&tw2).assignments.contains_key(&fx2.mech),
        "a seat for a gone station is dropped at boot"
    );
    assert!(
        subjects(&tw2, fx2.pilot).is_empty(),
        "no stale control link"
    );
}

#[test]
#[should_panic(expected = "claims host")]
fn boot_fails_on_stations_key_host_mismatch() {
    let mut tw = boot();
    let _fx = fixture(&mut tw);
    tw.modules_loaded(HashMap::from([(
        SmolStr::new("sim"),
        ModuleBootData {
            kv: vec![(
                SmolStr::new("stations:5"),
                serde_json::json!({ "host": 6, "stations": {} }),
            )],
            streams: HashMap::new(),
        },
    )]));
}

// ---------------------------------------------------------------------
// F3: control channels, authority union, and the control-epoch lease
// ---------------------------------------------------------------------

fn channels_of(tw: &TestWorld, actor: ObjId, subject: ObjId) -> Vec<String> {
    tw.world()
        .command_subjects_of(actor)
        .iter()
        .find(|l| l.subject == subject)
        .map(|l| l.channels.iter().map(|c| c.as_str().to_string()).collect())
        .unwrap_or_default()
}

#[test]
fn a_station_grants_channels_and_priority_defaults_by_role() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    let ann = fx.pilot;
    // pilot grants movement (priority defaults to 2); gunner grants weapons (1).
    ok(
        &mut tw,
        "@spatial/station Atlas=cockpit,pilot,channels:movement",
    );
    ok(
        &mut tw,
        "@spatial/station Atlas=gun,gunner,channels:weapons",
    );
    ok(
        &mut tw,
        &format!("@spatial/assign Atlas=cockpit,#{}", ann.0),
    );
    ok(&mut tw, &format!("@spatial/assign Atlas=gun,#{}", ann.0));
    // The actor's single link UNIONS both seats' channels; relation = the
    // max-priority seat (pilot), priority = 2.
    let mut chans = channels_of(&tw, ann, fx.mech);
    chans.sort();
    assert_eq!(
        chans,
        vec!["movement".to_string(), "weapons".to_string()],
        "channels union across seats"
    );
    let link = tw.world().command_subjects_of(ann)[0].clone();
    assert_eq!(
        link.relation.as_str(),
        "pilot",
        "relation = max-priority seat"
    );
    assert_eq!(link.priority, 2, "pilot's default priority");
}

#[test]
fn explicit_station_priority_overrides_the_role_default() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    ok(&mut tw, "@spatial/station Atlas=cockpit,gunner,priority:9");
    ok(
        &mut tw,
        &format!("@spatial/assign Atlas=cockpit,#{}", fx.pilot.0),
    );
    assert_eq!(
        tw.world().command_subjects_of(fx.pilot)[0].priority,
        9,
        "explicit priority wins over role default"
    );
}

#[test]
fn control_epoch_bumps_on_assign_unassign_and_survives_reboot() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    ok(
        &mut tw,
        "@spatial/station Atlas=cockpit,pilot,channels:movement",
    );
    let g0 = state(&tw).control_generation(fx.mech);
    ok(
        &mut tw,
        &format!("@spatial/assign Atlas=cockpit,#{}", fx.pilot.0),
    );
    let g1 = state(&tw).control_generation(fx.mech);
    assert!(g1 > g0, "assign bumps the control epoch ({g0} -> {g1})");
    ok(&mut tw, "@spatial/unassign Atlas=cockpit");
    let g2 = state(&tw).control_generation(fx.mech);
    assert!(g2 > g1, "unassign bumps the control epoch ({g1} -> {g2})");
    // Monotonic across reboot.
    let kv = fold_kv(&mut tw);
    let (tw2, fx2) = reboot_with(kv);
    assert_eq!(
        state(&tw2).control_generation(fx2.mech),
        g2,
        "epoch persists across reboot"
    );
}

#[test]
fn the_authority_inspector_shows_granted_channels() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    ok(
        &mut tw,
        "@spatial/station Atlas=cockpit,pilot,channels:movement",
    );
    ok(
        &mut tw,
        &format!("@spatial/assign Atlas=cockpit,#{}", fx.pilot.0),
    );
    let t = say(&mut tw, &format!("@spatial/authority #{}", fx.pilot.0));
    assert!(
        t.contains("movement") && t.contains("pilot"),
        "inspector shows channels + relation: {t}"
    );
}

#[test]
fn a_legacy_m29_station_doc_without_channels_or_priority_loads() {
    // A pre-F3 stations doc has role/space/policy but NO channels/priority; the
    // #[serde(default)] fields must let it load (empty channels, priority 0).
    let mut tw = boot();
    let fx = fixture(&mut tw);
    let _ = tw.take_module_ops();
    let key = SmolStr::new(format!("stations:{}", fx.mech.0));
    let doc = serde_json::json!({
        "host": fx.mech.0,
        "stations": {
            "cockpit": { "role": "pilot", "space": null, "policy": { "schema": "sim.passive", "version": 1, "data": null } }
        }
    });
    let (mut tw2, fx2) = reboot_with(vec![(key, doc)]);
    let st = state(&tw2);
    let hs = st
        .stations
        .get(&fx2.mech)
        .expect("legacy station doc loaded");
    let spec = hs
        .stations
        .get(&mush_sim::StationId::new("cockpit").unwrap())
        .unwrap();
    assert!(
        spec.channels.is_empty(),
        "legacy station grants no channels"
    );
    assert_eq!(spec.priority, 0, "legacy station defaults to priority 0");
    let exact = mush_sim::passive_definition_ref();
    assert_eq!(
        spec.policy.schema(),
        &exact,
        "legacy policy maps to the built-in exact definition"
    );
    assert!(spec.policy.data().is_null());
    let encoded = serde_json::to_value(&spec.policy).unwrap();
    assert!(
        encoded["schema"].is_object(),
        "shared v2 schema is an exact reference: {encoded}"
    );
    assert_eq!(encoded["schema"]["key"]["owner"], serde_json::json!("sim"));
    assert_eq!(
        encoded["schema"]["key"]["kind"],
        serde_json::json!("schema")
    );
    assert_eq!(encoded["schema"]["key"]["id"], serde_json::json!("passive"));
    assert_eq!(encoded["schema"]["version"], serde_json::json!(1));
    assert!(
        encoded.get("version").is_none(),
        "legacy top-level version is not re-emitted"
    );
    let definitions = tw2
        .registry()
        .definitions()
        .expect("composition froze definitions");
    assert_eq!(definitions.get_exact(&exact).unwrap().reference(), &exact);
    assert!(
        tw2.take_module_ops().is_empty(),
        "a clean legacy read is not eagerly rewritten"
    );
}

#[test]
fn the_control_epoch_bumps_at_every_control_change_site() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    // A plain placed participant (no station) grows NO cgen doc and stays at 0.
    let _ = tw.take_module_ops();
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    assert_eq!(
        state(&tw).control_generation(fx.mech),
        0,
        "a plain placed object has no epoch"
    );
    assert!(
        !tw.take_module_ops()
            .iter()
            .any(|o| matches!(&o.op, ModuleWrite::JsonSet { key, .. } if key.starts_with("cgen:"))),
        "no cgen doc for a non-host object"
    );
    // Defining, redefining, unplacing, and replacing a control host each bump.
    ok(
        &mut tw,
        "@spatial/station Atlas=cockpit,pilot,channels:movement",
    );
    let a = state(&tw).control_generation(fx.mech);
    assert!(a > 0, "station define bumps");
    ok(&mut tw, "@spatial/station Atlas=cockpit,pilot,priority:5");
    let b = state(&tw).control_generation(fx.mech);
    assert!(b > a, "station redefine bumps");
    ok(&mut tw, "@spatial/unplace Atlas");
    let c = state(&tw).control_generation(fx.mech);
    assert!(c > b, "unplace of a control host bumps");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    assert!(
        state(&tw).control_generation(fx.mech) > c,
        "re-place of a control host bumps"
    );
}

#[test]
fn destroying_a_crew_actor_bumps_its_unit_epoch() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    ok(
        &mut tw,
        "@spatial/station Atlas=cockpit,pilot,channels:movement",
    );
    ok(
        &mut tw,
        &format!("@spatial/assign Atlas=cockpit,#{}", fx.pilot.0),
    );
    let g = state(&tw).control_generation(fx.mech);
    // Destroying the pilot vacates its seat: the unit's control changed -> bump.
    tw.world_mut().destroy(fx.pilot).unwrap();
    let mut out = Outbox::new();
    let mut fxx = Effects::new();
    tw.engine_mut().emit_world_event(
        &WorldEvent::ObjDestroyed { obj: fx.pilot },
        &mut out,
        &mut fxx,
    );
    assert!(
        state(&tw).control_generation(fx.mech) > g,
        "destroying a crew actor bumps its unit's epoch"
    );
}

#[test]
fn exhausted_control_generation_refuses_structural_change_without_mutation() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(&mut tw, "@domain/create Field=sim.passive");
    ok(&mut tw, "@spatial/place/domain Atlas=0");
    ok(
        &mut tw,
        "@spatial/station Atlas=cockpit,pilot,channels:movement",
    );
    ok(
        &mut tw,
        &format!("@spatial/assign Atlas=cockpit,#{}", fx.pilot.0),
    );
    tw.world_mut()
        .module_state_mut::<SpatialState>()
        .expect("sim state")
        .control_gen
        .insert(fx.mech, u64::MAX);
    let before = state(&tw).clone();
    let before_location = loc(&tw, fx.mech);
    let before_subjects = subjects(&tw, fx.pilot);
    let _ = tw.take_module_ops();

    let refused = say(&mut tw, "@spatial/unplace Atlas");
    assert!(refused.contains("exhausted its control"), "{refused}");
    assert_eq!(state(&tw), &before, "refusal mutated sim state");
    assert_eq!(loc(&tw, fx.mech), before_location);
    assert_eq!(subjects(&tw, fx.pilot), before_subjects);
    assert!(
        tw.take_module_ops().is_empty(),
        "refusal emitted persistence"
    );
}

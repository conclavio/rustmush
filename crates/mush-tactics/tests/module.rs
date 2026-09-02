//! M33 acceptance: the tactics module attaches M31/M32 to real objects, persists
//! them, survives reboot, and READS them (@unit/show + the tactics.unit panel
//! consuming the M32 damage Journal). Driven through the real Engine via
//! `TestWorld`; the example catalog stands in for the not-yet-built ruleset.

use std::collections::HashMap;

use mush_core::{Audience, BehaviorBinding, BehaviorId, Effects, GOD, ObjId, ObjKind, Outbox, ModuleBootData, ModuleWrite, WorldEvent};
use mush_proto::{OutputEvent, PanelFrameKind};
use mush_testkit::{TestWorld, text_of};
use mush_tactics::{
    CapabilityId, CapabilityInit, CapabilityKey, CellId, CellSpec, ConditionId, DefinitionRef, FacetId,
    FacetSpec, LocomotionId, MediumId, MobilityMode, MobilityModeId, MobilityProfile, MobilityVocabulary,
    ModeRules, RulesValue, SectionId, SectionSpec, TacticalUnit, TacticsModule, UnitCatalog,
    UnitCatalogBuilder, UnitSpec, UnitStore,
};
use smol_str::SmolStr;

const FEED: &str = "tactics.unit";

fn example_spec() -> UnitSpec {
    let reg = MobilityVocabulary::base().freeze();
    let profile = MobilityProfile::new(
        vec![MobilityMode::new(
            MobilityModeId::new("ground.biped").unwrap(),
            [MediumId::new("ground").unwrap()],
            LocomotionId::new("legged").unwrap(),
            ModeRules::empty("tacsim.kin"),
        )],
        MobilityModeId::new("ground.biped").unwrap(),
        &reg,
    )
    .unwrap();
    UnitSpec {
        definition: DefinitionRef::new("tacsim.mech.locust", 1).unwrap(),
        sections: vec![SectionSpec {
            id: SectionId::new("center_torso").unwrap(),
            condition: ConditionId::new("ok").unwrap(),
            facets: vec![FacetSpec {
                id: FacetId::new("rear").unwrap(),
                cells: vec![CellSpec { id: CellId::new("armor").unwrap(), min: 0, max: 6, init: 6 }],
            }],
            slots: vec![],
            cells: vec![CellSpec { id: CellId::new("structure").unwrap(), min: 0, max: 8, init: 8 }],
        }],
        unit_cells: vec![CellSpec { id: CellId::new("heat").unwrap(), min: 0, max: 30, init: 0 }],
        components: vec![],
        capabilities: vec![CapabilityInit {
            key: CapabilityKey { id: CapabilityId::new("tactics.mobility").unwrap(), source: None },
            rules: RulesValue::empty("x"),
        }],
        mobility: Some(profile),
        rules: RulesValue::empty("x"),
    }
}

fn example_catalog() -> UnitCatalog {
    let reg = MobilityVocabulary::base().freeze();
    let mut cb = UnitCatalogBuilder::new();
    cb.add(example_spec(), &reg).unwrap();
    cb.freeze()
}

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![Box::new(TacticsModule::with_catalog(example_catalog()))])
}
fn unit_beh() -> BehaviorId {
    BehaviorId::new("tactics.unit").unwrap()
}
fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events.iter().map(|(_, e)| text_of(e)).collect::<Vec<_>>().join("\n")
}
fn has_error(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error"))))
}
fn frames(events: &[(Audience, OutputEvent)], kind: PanelFrameKind) -> Vec<serde_json::Value> {
    events
        .iter()
        .filter_map(|(_, e)| match e {
            OutputEvent::Panel { feed, kind: k, data } if feed == FEED && *k == kind => Some(data.clone()),
            _ => None,
        })
        .collect()
}
fn units(tw: &TestWorld) -> &UnitStore {
    tw.world().module_state::<UnitStore>().expect("unit store")
}
fn cur(tw: &TestWorld, obj: ObjId) -> i64 {
    let u = units(tw).units.get(&obj).unwrap();
    u.cell(&mush_tactics::CellAddr::Facet {
        section: SectionId::new("center_torso").unwrap(),
        facet: FacetId::new("rear").unwrap(),
        cell: CellId::new("armor").unwrap(),
    })
    .unwrap()
    .current
}
/// A JSON UnitMutationPlan that reduces rear armor by `d`.
fn armor_plan(d: i64) -> String {
    format!(
        r#"{{"ops":[{{"AdjustCell":{{"addr":{{"Facet":{{"section":"center_torso","facet":"rear","cell":"armor"}}}},"delta":{d}}}}}]}}"#
    )
}
fn fold_kv(tw: &mut TestWorld) -> Vec<(SmolStr, serde_json::Value)> {
    let mut kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    for op in tw.take_module_ops() {
        if op.ns != "tactics" {
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
fn reboot(kv: Vec<(SmolStr, serde_json::Value)>) -> (TestWorld, ObjId) {
    let mut tw = boot();
    let mech = tw.world_mut().create(ObjKind::Thing, "Atlas", GOD, None).unwrap();
    tw.modules_loaded(HashMap::from([(SmolStr::new("tactics"), ModuleBootData { kv, streams: HashMap::new() })]));
    (tw, mech)
}
fn make_unit(tw: &mut TestWorld) -> ObjId {
    let mech = tw.world_mut().create(ObjKind::Thing, "Atlas", GOD, None).unwrap();
    let ev = tw.run(GOD, &format!("@unit/create #{}=tacsim.mech.locust", mech.0));
    assert!(!has_error(&ev), "create refused: {}", texts(&ev));
    mech
}

// ----------------------------------------------------------------------

#[test]
fn create_marks_persists_and_keyframes_a_unit() {
    let mut tw = boot();
    let mech = tw.world_mut().create(ObjKind::Thing, "Atlas", GOD, None).unwrap();
    let ev = tw.run(GOD, &format!("@unit/create #{}=tacsim.mech.locust", mech.0));
    assert!(units(&tw).units.contains_key(&mech), "unit in the store");
    assert!(tw.world().has_behavior(mech, &unit_beh()), "tactics.unit binding attached");
    assert_eq!(cur(&tw, mech), 6, "rear armor at init");
    assert_eq!(frames(&ev, PanelFrameKind::Keyframe).len(), 1, "one keyframe on create");
    // The doc persisted under ns tactics.
    assert!(tw.take_module_ops().iter().any(|o| o.ns == "tactics" && matches!(&o.op, ModuleWrite::JsonSet { key, .. } if key == &format!("unit:{}", mech.0))));
}

#[test]
fn create_is_refused_for_bad_targets_and_unknown_schemas() {
    let mut tw = boot();
    let room = tw.world_mut().create(ObjKind::Room, "Field", GOD, None).unwrap();
    assert!(has_error(&tw.run(GOD, &format!("@unit/create #{}=tacsim.mech.locust", room.0))), "a room cannot be a unit");
    let mech = make_unit(&mut tw);
    assert!(has_error(&tw.run(GOD, &format!("@unit/create #{}=tacsim.mech.locust", mech.0))), "already a unit");
    let m2 = tw.world_mut().create(ObjKind::Thing, "Other", GOD, None).unwrap();
    assert!(has_error(&tw.run(GOD, &format!("@unit/create #{}=nope.nope", m2.0))), "unknown schema");
}

#[test]
fn damage_applies_persists_and_deltas_from_the_journal() {
    let mut tw = boot();
    let mech = make_unit(&mut tw);
    tw.take_module_ops();
    let ev = tw.run(GOD, &format!("@unit/damage #{}={}", mech.0, armor_plan(-4)));
    assert!(!has_error(&ev), "damage refused: {}", texts(&ev));
    assert_eq!(cur(&tw, mech), 2, "armor reduced in the store (single source of truth)");
    assert_eq!(units(&tw).units.get(&mech).unwrap().revision(), 1, "revision bumped");
    let deltas = frames(&ev, PanelFrameKind::Delta);
    assert_eq!(deltas.len(), 1, "one delta per plan");
    assert_eq!(deltas[0]["from_revision"], serde_json::json!(0), "delta = serialized journal");
    assert_eq!(deltas[0]["to_revision"], serde_json::json!(1));
    // Persisted the whole updated doc.
    assert!(tw.take_module_ops().iter().any(|o| matches!(&o.op, ModuleWrite::JsonSet { key, .. } if key == &format!("unit:{}", mech.0))));
}

#[test]
fn damage_refuses_grant_capability_and_out_of_bounds() {
    let mut tw = boot();
    let mech = make_unit(&mut tw);
    // GrantCapability is rejected via the staff debug path (safe subset).
    let grant = r#"{"ops":[{"GrantCapability":{"key":{"id":"tactics.x","source":null},"rules":{"schema":"x","version":1,"data":{}}}}]}"#;
    assert!(has_error(&tw.run(GOD, &format!("@unit/damage #{}={}", mech.0, grant))), "GrantCapability refused");
    // Out of bounds refuses the whole plan (M32 refuse-not-clamp); armor untouched.
    assert!(has_error(&tw.run(GOD, &format!("@unit/damage #{}={}", mech.0, armor_plan(-999)))));
    assert_eq!(cur(&tw, mech), 6, "refused plan left the unit untouched");
}

#[test]
fn a_unit_survives_reboot_with_its_damage() {
    let mut tw = boot();
    let mech = make_unit(&mut tw);
    tw.run(GOD, &format!("@unit/damage #{}={}", mech.0, armor_plan(-4)));
    let kv = fold_kv(&mut tw);
    let (tw2, mech2) = reboot(kv);
    assert_eq!(mech, mech2, "deterministic id");
    assert!(units(&tw2).units.contains_key(&mech2), "unit restored");
    assert_eq!(cur(&tw2, mech2), 2, "damage persisted across reboot");
    assert!(tw2.world().has_behavior(mech2, &unit_beh()), "binding present after boot");
}

#[test]
fn destroying_the_object_drops_the_unit_and_doc() {
    let mut tw = boot();
    let mech = make_unit(&mut tw);
    tw.take_module_ops();
    tw.world_mut().destroy(mech).unwrap();
    let mut out = Outbox::new();
    let mut fxx = Effects::new();
    tw.engine_mut().emit_world_event(&WorldEvent::ObjDestroyed { obj: mech }, &mut out, &mut fxx);
    assert!(!units(&tw).units.contains_key(&mech), "unit dropped");
    assert!(tw.take_module_ops().iter().any(|o| matches!(&o.op, ModuleWrite::Del { key } if key == &format!("unit:{}", mech.0))), "doc deleted");
}

#[test]
fn detach_gate_blocks_stripping_a_live_unit_but_remove_is_the_key() {
    let mut tw = boot();
    let mech = make_unit(&mut tw);
    // The DetachGate refuses @behavior/remove while the unit is live.
    assert!(has_error(&tw.run(GOD, &format!("@behavior/remove #{}=tactics.unit", mech.0))), "gate refuses strip");
    assert!(tw.world().has_behavior(mech, &unit_beh()));
    // @unit/remove is the sanctioned teardown: drops the unit + detaches.
    let ev = tw.run(GOD, &format!("@unit/remove #{}", mech.0));
    assert!(!has_error(&ev), "remove refused: {}", texts(&ev));
    assert!(!units(&tw).units.contains_key(&mech));
    assert!(!tw.world().has_behavior(mech, &unit_beh()), "binding detached by remove");
}

#[test]
fn boot_detaches_a_binding_with_no_doc() {
    let mut tw = boot();
    let ghost = tw.world_mut().create(ObjKind::Thing, "Ghost", GOD, None).unwrap();
    tw.world_mut()
        .attach_behavior(ghost, unit_beh(), BehaviorBinding { schema_version: 1, attached_by: GOD, attached_at: 0 })
        .unwrap();
    // Reboot with NO unit docs: the stale marker is detached.
    tw.modules_loaded(HashMap::from([(SmolStr::new("tactics"), ModuleBootData { kv: vec![], streams: HashMap::new() })]));
    assert!(!tw.world().has_behavior(ghost, &unit_beh()), "binding with no doc detached at boot");
}

#[test]
fn boot_reattaches_a_binding_for_a_doc_that_lost_it() {
    let mut tw = boot();
    let mech = make_unit(&mut tw);
    let kv = fold_kv(&mut tw);
    // Reboot recreates the object but does NOT re-add the binding; boot must.
    let (tw2, mech2) = reboot(kv);
    assert_eq!(mech, mech2);
    assert!(tw2.world().has_behavior(mech2, &unit_beh()), "boot re-attached the marker for a live doc");
    assert!(units(&tw2).units.contains_key(&mech2));
}

#[test]
fn catalog_builder_validates_mobility_against_the_registry() {
    // A spec whose mode uses an undeclared locomotion is refused at freeze time.
    let reg = MobilityVocabulary::base().freeze();
    let mut spec = example_spec();
    let bad = MobilityProfile::new(
        vec![MobilityMode::new(
            MobilityModeId::new("ground.mech").unwrap(),
            [MediumId::new("ground").unwrap()],
            LocomotionId::new("battlemech").unwrap(), // not in the base vocabulary
            ModeRules::empty("x"),
        )],
        MobilityModeId::new("ground.mech").unwrap(),
        &{
            // build the bad profile against a registry that DOES declare it,
            // so the profile itself is valid; the CATALOG registry does not.
            let mut v = MobilityVocabulary::base();
            v.declare_locomotion(LocomotionId::new("battlemech").unwrap());
            v.freeze()
        },
    )
    .unwrap();
    spec.mobility = Some(bad);
    let mut cb = UnitCatalogBuilder::new();
    assert!(cb.add(spec, &reg).is_err(), "catalog freeze rejects an undeclared-atom mode");
}

#[test]
fn show_dumps_the_unit_structure() {
    let mut tw = boot();
    let mech = make_unit(&mut tw);
    let t = texts(&tw.run(GOD, &format!("@unit/show #{}", mech.0)));
    assert!(t.contains("tacsim.mech.locust") && t.contains("center_torso") && t.contains("ground.biped"), "{t}");
}

// ---- verify-fold: corrupt-doc boot branches, staff gating, negative destroy ----

fn del_keys(tw: &mut TestWorld) -> Vec<String> {
    tw.take_module_ops()
        .iter()
        .filter_map(|o| match &o.op {
            ModuleWrite::Del { key } if o.ns == "tactics" => Some(key.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn boot_deletes_the_raw_key_of_a_mismatched_doc_not_the_embedded_id() {
    let mut tw = boot();
    let mech = tw.world_mut().create(ObjKind::Thing, "Atlas", GOD, None).unwrap();
    let doc = serde_json::to_value(TacticalUnit::from_spec(&example_spec(), mech).unwrap()).unwrap();
    let good = SmolStr::new(format!("unit:{}", mech.0));
    let wrong = SmolStr::new(format!("unit:{}", mech.0 + 1000)); // key id != embedded object id
    tw.modules_loaded(HashMap::from([(
        SmolStr::new("tactics"),
        ModuleBootData { kv: vec![(good.clone(), doc.clone()), (wrong.clone(), doc)], streams: HashMap::new() },
    )]));
    // The healthy unit loaded; ONLY the mis-keyed doc is deleted (never the good one).
    assert!(units(&tw).units.contains_key(&mech), "healthy unit survived");
    let dels = del_keys(&mut tw);
    assert!(dels.contains(&wrong.to_string()), "the mis-keyed doc is deleted");
    assert!(!dels.contains(&good.to_string()), "the healthy sibling doc is NOT deleted");
}

#[test]
fn boot_deletes_an_unparseable_doc_by_its_raw_key() {
    let mut tw = boot();
    // A non-numeric suffix + garbage value: the raw key must be deleted, not unit:0.
    tw.modules_loaded(HashMap::from([(
        SmolStr::new("tactics"),
        ModuleBootData { kv: vec![(SmolStr::new("unit:abc"), serde_json::json!("not a unit"))], streams: HashMap::new() },
    )]));
    let dels = del_keys(&mut tw);
    assert!(dels.contains(&"unit:abc".to_string()), "the raw corrupt key is deleted: {dels:?}");
    assert!(!dels.contains(&"unit:0".to_string()), "does not spuriously delete unit:0");
}

#[test]
fn unit_commands_are_staff_only() {
    let mut tw = boot();
    let mech = make_unit(&mut tw);
    let joe = tw.spawn_player("Joe"); // ordinary player: no SIM_ADMIN, not Royalty
    assert!(has_error(&tw.run(joe, &format!("@unit/show #{}", mech.0))), "show refused for non-staff");
    assert!(has_error(&tw.run(joe, &format!("@unit/damage #{}={}", mech.0, armor_plan(-1)))), "damage refused");
    assert!(has_error(&tw.run(joe, &format!("@unit/remove #{}", mech.0))), "remove refused");
    assert!(has_error(&tw.run(joe, &format!("@unit/create #{}=tacsim.mech.locust", joe.0))), "create refused");
    assert!(units(&tw).units.contains_key(&mech), "the unit is untouched by a non-staff actor");
}

#[test]
fn destroying_an_unrelated_object_leaves_units_untouched() {
    let mut tw = boot();
    let mech = make_unit(&mut tw);
    let rock = tw.world_mut().create(ObjKind::Thing, "Rock", GOD, None).unwrap();
    tw.take_module_ops();
    tw.world_mut().destroy(rock).unwrap();
    let mut out = Outbox::new();
    let mut fxx = Effects::new();
    tw.engine_mut().emit_world_event(&WorldEvent::ObjDestroyed { obj: rock }, &mut out, &mut fxx);
    assert!(units(&tw).units.contains_key(&mech), "the unit survives an unrelated destroy");
    assert!(del_keys(&mut tw).is_empty(), "no unit doc deleted for an unrelated object");
}

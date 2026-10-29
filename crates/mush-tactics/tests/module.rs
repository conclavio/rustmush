//! K1 acceptance for the live tactics module: exact definition composition,
//! v2-only authoring, explicit v1 dry-run migration, lifecycle gating, and a
//! boot path that never edits authoritative records or behavior bindings.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use mush_core::module::{GameModule, ModError, Registrar};
use mush_core::{
    Audience, BehaviorBinding, BehaviorId, DefinitionDescriptor, DefinitionDigest, DefinitionId,
    DefinitionKey, DefinitionKind, DefinitionRef, DefinitionRegistryBuilder,
    DefinitionValidationContext, Effects, GOD, MechanicalState, ModuleBootData, ModuleWrite, ObjId,
    ObjKind, Outbox, OwnerId, RecoveryReason, WorldEvent,
};
use mush_proto::{OutputEvent, PanelFrameKind};
use mush_tactics::{
    CapabilityId, CapabilityInit, CapabilityKey, CellId, CellSpec, ComponentId, ComponentSpec,
    ConditionId, FacetId, FacetSpec, InstallPoint, InstallSlotId, LocomotionId,
    MAX_UNIT_BINDING_REPAIRS, MAX_UNIT_CELLS, MAX_UNIT_SECTIONS, MediumId, MobilityMode,
    MobilityModeId, MobilityProfile, MobilityVocabulary, ModeRules, RulesValue, SectionId,
    SectionSpec, TacticalUnit, TacticsModule, UNIT_BINDING_VERSION, UNIT_RECORD_VERSION,
    UnitCatalog, UnitCatalogBuilder, UnitSpec, UnitSpecBody, UnitStore,
};
use mush_testkit::{TestWorld, text_of};
use serde_json::{Value, json};
use smol_str::SmolStr;

const FEED: &str = "tactics.unit";

#[path = "module/movement_custody.rs"]
mod movement_custody;

fn register_ref(
    builder: &mut DefinitionRegistryBuilder,
    owner: &str,
    kind: &str,
    id: &str,
    payload: Value,
) -> DefinitionRef {
    builder
        .register(
            OwnerId::new(owner).unwrap(),
            DefinitionDescriptor::new(
                DefinitionKind::new(kind).unwrap(),
                DefinitionId::new(id).unwrap(),
                1,
                payload,
            )
            .unwrap(),
        )
        .unwrap()
}

fn example_spec() -> UnitSpec {
    example_spec_for("tacsim", "tacsim.mech.locust")
}

fn example_spec_for(owner: &str, unit_id: &str) -> UnitSpec {
    let mut definitions = DefinitionRegistryBuilder::new();
    let opaque = register_ref(
        &mut definitions,
        owner,
        "schema",
        &format!("{owner}.x"),
        json!({}),
    );
    let kinematics = register_ref(
        &mut definitions,
        owner,
        "schema",
        &format!("{owner}.kin"),
        json!({}),
    );
    let vocabulary = MobilityVocabulary::base().freeze();
    let profile = MobilityProfile::new(
        vec![MobilityMode::new(
            MobilityModeId::new("ground.biped").unwrap(),
            [MediumId::new("ground").unwrap()],
            LocomotionId::new("legged").unwrap(),
            ModeRules::new(kinematics, json!({})).unwrap(),
        )],
        MobilityModeId::new("ground.biped").unwrap(),
        &vocabulary,
    )
    .unwrap();
    let body = UnitSpecBody {
        sections: vec![SectionSpec {
            id: SectionId::new("center_torso").unwrap(),
            condition: ConditionId::new("ok").unwrap(),
            facets: vec![FacetSpec {
                id: FacetId::new("rear").unwrap(),
                cells: vec![CellSpec {
                    id: CellId::new("armor").unwrap(),
                    min: 0,
                    max: 6,
                    init: 6,
                }],
            }],
            slots: vec![],
            cells: vec![CellSpec {
                id: CellId::new("structure").unwrap(),
                min: 0,
                max: 8,
                init: 8,
            }],
        }],
        unit_cells: vec![CellSpec {
            id: CellId::new("heat").unwrap(),
            min: 0,
            max: 30,
            init: 0,
        }],
        components: vec![],
        capabilities: vec![CapabilityInit {
            key: CapabilityKey {
                id: CapabilityId::new("tactics.mobility").unwrap(),
                source: None,
            },
            rules: RulesValue::new(opaque.clone(), json!({})).unwrap(),
        }],
        mobility: Some(profile),
        rules: RulesValue::new(opaque, json!({})).unwrap(),
    };
    let descriptor = body
        .descriptor(
            DefinitionKind::new("unit").unwrap(),
            DefinitionId::new(unit_id).unwrap(),
            1,
        )
        .unwrap();
    let definition = definitions
        .register(OwnerId::new(owner).unwrap(), descriptor)
        .unwrap();
    body.with_definition(definition)
}

/// A second schema whose one component is detachable, ownable kit — the only
/// thing `InstalledComponent.asset` was ever for, and the subject of the E3c
/// scrub.
fn kitted_spec() -> UnitSpec {
    let mut definitions = DefinitionRegistryBuilder::new();
    let opaque = register_ref(&mut definitions, "tacsim", "schema", "tacsim.x", json!({}));
    let gauss = register_ref(
        &mut definitions,
        "tacsim",
        "component",
        "tacsim.weapon.gauss",
        json!({}),
    );
    let body = UnitSpecBody {
        sections: vec![SectionSpec {
            id: SectionId::new("center_torso").unwrap(),
            condition: ConditionId::new("ok").unwrap(),
            facets: vec![],
            slots: vec![],
            cells: vec![CellSpec {
                id: CellId::new("structure").unwrap(),
                min: 0,
                max: 8,
                init: 8,
            }],
        }],
        unit_cells: vec![],
        components: vec![ComponentSpec {
            id: ComponentId::new("gauss").unwrap(),
            definition: gauss,
            install: InstallPoint::Unit,
            condition: ConditionId::new("ok").unwrap(),
            cells: vec![],
            asset: None,
        }],
        capabilities: vec![],
        mobility: None,
        rules: RulesValue::new(opaque, json!({})).unwrap(),
    };
    let descriptor = body
        .descriptor(
            DefinitionKind::new("unit").unwrap(),
            DefinitionId::new("tacsim.mech.atlas").unwrap(),
            1,
        )
        .unwrap();
    let definition = definitions
        .register(OwnerId::new("tacsim").unwrap(), descriptor)
        .unwrap();
    body.with_definition(definition)
}

fn example_catalog() -> UnitCatalog {
    let vocabulary = MobilityVocabulary::base().freeze();
    let spec = example_spec();
    let kit = kitted_spec();
    let mut catalog = UnitCatalogBuilder::new();
    catalog.add(spec.clone(), &vocabulary).unwrap();
    catalog.add(kit, &vocabulary).unwrap();
    catalog
        .legacy_v1_alias(
            DefinitionId::new("tacsim.mech.locust").unwrap(),
            1,
            spec.definition,
        )
        .unwrap();
    catalog.freeze()
}

#[derive(Debug)]
struct TestRulesModule {
    owner: &'static str,
    specs: Vec<UnitSpec>,
}

impl GameModule for TestRulesModule {
    fn name(&self) -> &'static str {
        self.owner
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let mut definitions = BTreeMap::new();
        for spec in &self.specs {
            for reference in spec.body().definition_references() {
                let descriptor = DefinitionDescriptor::new(
                    reference.key().kind.clone(),
                    reference.key().id.clone(),
                    reference.version(),
                    json!({}),
                )
                .map_err(|error| ModError::msg(error.to_string()))?;
                definitions.entry(reference).or_insert(descriptor);
            }
            definitions.insert(
                spec.definition.clone(),
                spec.descriptor()
                    .map_err(|error| ModError::msg(error.to_string()))?
                    .as_current(),
            );
        }

        let mut expected_by_kind: BTreeMap<DefinitionKind, BTreeMap<DefinitionRef, _>> =
            BTreeMap::new();
        for (reference, descriptor) in &definitions {
            let mut references = descriptor.references().to_vec();
            references.sort();
            expected_by_kind
                .entry(reference.key().kind.clone())
                .or_default()
                .insert(
                    reference.clone(),
                    (
                        descriptor.payload().clone(),
                        references,
                        descriptor.is_current(),
                    ),
                );
        }
        for (kind, expected) in expected_by_kind {
            reg.definition_validator(kind, move |context: &DefinitionValidationContext<'_>| {
                let entry = context.definition();
                let Some((payload, references, current)) = expected.get(entry.reference()) else {
                    return Err("unknown test rules definition".into());
                };
                if entry.payload() != payload
                    || entry.references() != references
                    || entry.is_current() != *current
                {
                    return Err("test rules definition failed typed validation".into());
                }
                Ok(())
            })?;
        }

        for (reference, descriptor) in definitions {
            let registered = reg.definition(descriptor)?;
            if registered != reference {
                return Err(ModError::msg("test definition identity drift"));
            }
        }
        Ok(())
    }
}

fn boot_with_provider(include_provider: bool) -> TestWorld {
    let spec = example_spec();
    let kit = kitted_spec();
    let catalog = example_catalog();
    let mut modules: Vec<Box<dyn GameModule>> = Vec::new();
    if include_provider {
        modules.push(Box::new(TestRulesModule {
            owner: "tacsim",
            specs: vec![spec, kit],
        }));
    }
    modules.push(Box::new(TacticsModule::with_catalog(catalog)));
    TestWorld::with_modules(modules)
}

fn boot() -> TestWorld {
    boot_with_provider(true)
}

fn unit_behavior() -> BehaviorId {
    BehaviorId::new("tactics.unit").unwrap()
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, event)| text_of(event))
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_error(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, event)| {
        matches!(event, OutputEvent::Sys(text) if text.spans().iter().any(|span| span.tag.as_deref() == Some("error")))
    })
}

fn frames(events: &[(Audience, OutputEvent)], kind: PanelFrameKind) -> Vec<Value> {
    events
        .iter()
        .filter_map(|(_, event)| match event {
            OutputEvent::Panel {
                feed,
                kind: actual,
                data,
            } if feed == FEED && *actual == kind => Some(data.clone()),
            _ => None,
        })
        .collect()
}

fn store(world: &TestWorld) -> &UnitStore {
    world
        .world()
        .module_state::<UnitStore>()
        .expect("unit store")
}

fn armor(world: &TestWorld, object: ObjId) -> i64 {
    store(world)
        .units
        .get(&object)
        .unwrap()
        .cell(&mush_tactics::CellAddr::Facet {
            section: SectionId::new("center_torso").unwrap(),
            facet: FacetId::new("rear").unwrap(),
            cell: CellId::new("armor").unwrap(),
        })
        .unwrap()
        .current
}

fn armor_plan(delta: i64) -> String {
    format!(
        r#"{{"ops":[{{"AdjustCell":{{"addr":{{"Facet":{{"section":"center_torso","facet":"rear","cell":"armor"}}}},"delta":{delta}}}}}]}}"#
    )
}

fn fold_kv(world: &mut TestWorld) -> Vec<(SmolStr, Value)> {
    let mut kv = Vec::new();
    for operation in world.take_module_ops() {
        if operation.ns != "tactics" {
            continue;
        }
        match operation.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.retain(|(existing, _)| *existing != key);
                kv.push((key, value));
            }
            ModuleWrite::Del { key } => kv.retain(|(existing, _)| *existing != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv
}

fn make_unit(world: &mut TestWorld) -> ObjId {
    let object = world
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, None)
        .unwrap();
    let output = world.run(
        GOD,
        &format!("@unit/create #{}=tacsim.mech.locust", object.0),
    );
    assert!(!has_error(&output), "create refused: {}", texts(&output));
    object
}

fn load_tactics(world: &mut TestWorld, kv: Vec<(SmolStr, Value)>) {
    world.modules_loaded(HashMap::from([(
        SmolStr::new("tactics"),
        ModuleBootData {
            kv,
            streams: HashMap::new(),
            ..Default::default()
        },
    )]));
}

fn attach_current_binding(world: &mut TestWorld, object: ObjId) {
    world
        .world_mut()
        .attach_behavior(
            object,
            unit_behavior(),
            BehaviorBinding {
                schema_version: UNIT_BINDING_VERSION,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .unwrap();
}

fn current_record_ref(world: &TestWorld) -> DefinitionRef {
    let key = DefinitionKey::new(
        OwnerId::new("tactics").unwrap(),
        DefinitionKind::new("record").unwrap(),
        DefinitionId::new("unit").unwrap(),
    );
    world
        .registry()
        .definitions()
        .unwrap()
        .current(&key)
        .expect("current tactics unit record schema")
        .clone()
}

fn current_document(world: &TestWorld, unit: &TacticalUnit) -> Value {
    json!({
        "record": "mush-tactics.unit",
        "record_version": UNIT_RECORD_VERSION,
        "record_schema": current_record_ref(world),
        "unit": unit,
    })
}

fn legacy_ref(reference: &Value) -> Value {
    json!({
        "id": reference["key"]["id"].clone(),
        "version": reference["version"].clone(),
    })
}

fn legacy_rules(value: &mut Value) {
    let schema = legacy_ref(&value["schema"]);
    let data = value["data"].take();
    *value = json!({
        "schema": schema["id"].clone(),
        "version": schema["version"].clone(),
        "data": data,
    });
}

fn legacy_document(unit: &TacticalUnit) -> Value {
    let mut value = serde_json::to_value(unit).unwrap();
    value["schema"] = legacy_ref(&value["schema"]);
    legacy_rules(&mut value["rules"]);
    for capability in value["capabilities"].as_array_mut().unwrap() {
        legacy_rules(&mut capability[1]);
    }
    if let Some(mobility) = value["mobility"].as_object_mut() {
        for mode in mobility["modes"].as_array_mut().unwrap() {
            legacy_rules(&mut mode["rules"]);
        }
    }
    for component in value["components"].as_object_mut().unwrap().values_mut() {
        component["definition"] = legacy_ref(&component["definition"]);
    }
    value
}

#[test]
fn create_authors_only_v2_with_exact_refs_and_current_binding() {
    let mut world = boot();
    let object = world
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, None)
        .unwrap();
    let output = world.run(
        GOD,
        &format!("@unit/create #{}=tacsim.mech.locust", object.0),
    );
    assert!(!has_error(&output), "{}", texts(&output));
    assert_eq!(armor(&world, object), 6);
    assert_eq!(frames(&output, PanelFrameKind::Keyframe).len(), 1);
    assert_eq!(store(&world).records.len(), 1);
    assert_eq!(store(&world).records[0].state, MechanicalState::Active);
    assert_eq!(store(&world).boot_report.as_ref().unwrap().active, 1);
    assert_eq!(
        world
            .world()
            .get(object)
            .unwrap()
            .behaviors
            .get(&unit_behavior())
            .unwrap()
            .schema_version,
        UNIT_BINDING_VERSION
    );
    let operations = world.take_module_ops();
    let value = operations
        .iter()
        .find_map(|operation| match &operation.op {
            ModuleWrite::JsonSet { key, value }
                if operation.ns == "tactics" && key == &format!("unit:{}", object.0) =>
            {
                Some(value)
            }
            _ => None,
        })
        .expect("v2 unit write");
    assert_eq!(value["record"], "mush-tactics.unit");
    assert_eq!(value["record_version"], UNIT_RECORD_VERSION);
    assert!(value["unit"]["schema"]["digest"].is_string());
}

#[test]
fn generic_behavior_attach_uses_the_registered_binding_schema() {
    let mut world = boot();
    let object = world
        .world_mut()
        .create(ObjKind::Thing, "Marker only", GOD, None)
        .unwrap();
    let output = world.run(GOD, &format!("@behavior/add #{}=tactics.unit", object.0));
    assert!(!has_error(&output), "{}", texts(&output));
    assert_eq!(
        world.world().get(object).unwrap().behaviors[&unit_behavior()].schema_version,
        UNIT_BINDING_VERSION
    );
}

#[test]
fn damage_is_atomic_persisted_and_projected() {
    let mut world = boot();
    let object = make_unit(&mut world);
    world.take_module_ops();
    let output = world.run(
        GOD,
        &format!("@unit/damage #{}={}", object.0, armor_plan(-4)),
    );
    assert!(!has_error(&output), "{}", texts(&output));
    assert_eq!(armor(&world, object), 2);
    assert_eq!(store(&world).units[&object].revision(), 1);
    assert_eq!(
        store(&world).records[0].unit.as_ref().unwrap().revision(),
        1
    );
    assert_eq!(frames(&output, PanelFrameKind::Delta)[0]["to_revision"], 1);
    assert!(world.take_module_ops().iter().any(|operation| {
        matches!(&operation.op, ModuleWrite::JsonSet { value, .. } if value["record_version"] == UNIT_RECORD_VERSION)
    }));
    assert!(has_error(&world.run(
        GOD,
        &format!("@unit/damage #{}={}", object.0, armor_plan(-999))
    )));
    assert_eq!(armor(&world, object), 2, "refusal leaves state unchanged");
}

#[test]
fn current_v2_reboots_active_only_with_exact_binding_and_definitions() {
    let mut first = boot();
    let object = make_unit(&mut first);
    first.run(
        GOD,
        &format!("@unit/damage #{}={}", object.0, armor_plan(-4)),
    );
    let kv = fold_kv(&mut first);

    let mut second = boot();
    let rebooted = second
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, None)
        .unwrap();
    assert_eq!(rebooted, object);
    attach_current_binding(&mut second, rebooted);
    second.take_module_ops();
    load_tactics(&mut second, kv);

    assert_eq!(armor(&second, rebooted), 2);
    assert_eq!(store(&second).boot_report.as_ref().unwrap().active, 1);
    assert!(
        second.take_module_ops().is_empty(),
        "boot authored no writes"
    );
}

#[test]
fn removing_a_boot_loaded_unit_reconciles_lifecycle_and_allows_recreate() {
    let mut first = boot();
    let object = make_unit(&mut first);
    let kv = fold_kv(&mut first);

    let mut second = boot();
    let rebooted = second
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, None)
        .unwrap();
    assert_eq!(rebooted, object);
    attach_current_binding(&mut second, object);
    load_tactics(&mut second, kv);
    assert_eq!(store(&second).records.len(), 1);

    let removed = second.run(GOD, &format!("@unit/remove #{}", object.0));
    assert!(!has_error(&removed), "{}", texts(&removed));
    assert!(!store(&second).units.contains_key(&object));
    assert!(
        store(&second)
            .records
            .iter()
            .all(|record| record.object != Some(object))
    );
    assert_eq!(store(&second).boot_report.as_ref().unwrap().active, 0);

    let recreated = second.run(
        GOD,
        &format!("@unit/create #{}=tacsim.mech.locust", object.0),
    );
    assert!(!has_error(&recreated), "{}", texts(&recreated));
    assert_eq!(store(&second).records.len(), 1);
    assert_eq!(store(&second).boot_report.as_ref().unwrap().active, 1);
}

#[test]
fn legacy_v1_has_exact_dry_run_but_stays_migrating_and_unwritten() {
    let mut world = boot();
    let object = world
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, None)
        .unwrap();
    world
        .world_mut()
        .attach_behavior(
            object,
            unit_behavior(),
            BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .unwrap();
    let unit = TacticalUnit::from_spec(&example_spec(), object).unwrap();
    world.take_module_ops();
    load_tactics(
        &mut world,
        vec![(
            SmolStr::new(format!("unit:{}", object.0)),
            legacy_document(&unit),
        )],
    );

    assert!(!store(&world).units.contains_key(&object));
    let record = &store(&world).records[0];
    assert!(matches!(record.state, MechanicalState::Migrating { .. }));
    let plan = record.migration_plan.as_ref().expect("exact dry-run plan");
    assert_eq!(plan.step_count, 1);
    assert_eq!(plan.from.key().kind.as_str(), "record");
    assert_eq!(plan.from.version(), 1);
    assert_eq!(plan.to.key().kind.as_str(), "record");
    assert_eq!(plan.to.version(), UNIT_RECORD_VERSION);
    assert_ne!(plan.before_digest, plan.after_digest);
    // Reaching Migrating also proves boot decoded the migration output through
    // the strict UnitDocumentV2 envelope and compared its body to this unit;
    // a bare TacticalUnit output is quarantined by decode_pending_record.
    assert_eq!(record.unit.as_ref(), Some(&unit));
    assert_eq!(store(&world).binding_repairs.len(), 1);
    assert!(
        world.take_module_ops().is_empty(),
        "legacy bytes were not rewritten"
    );
}

#[test]
fn missing_runtime_definition_freezes_and_preserves_current_record() {
    let spec = example_spec();
    let object = ObjId(2);
    let unit = TacticalUnit::from_spec(&spec, object).unwrap();
    let mut world = boot_with_provider(false);
    let actual = world
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, None)
        .unwrap();
    assert_eq!(actual, object);
    attach_current_binding(&mut world, object);
    world.take_module_ops();
    let document = current_document(&world, &unit);
    load_tactics(
        &mut world,
        vec![(SmolStr::new(format!("unit:{}", object.0)), document)],
    );

    assert!(!store(&world).units.contains_key(&object));
    assert!(matches!(
        store(&world).records[0].state,
        MechanicalState::Frozen { .. }
    ));
    assert_eq!(store(&world).boot_report.as_ref().unwrap().frozen, 1);
    assert!(world.take_module_ops().is_empty());
}

#[test]
fn current_record_with_missing_binding_freezes_without_reattaching() {
    let mut world = boot();
    let object = world
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, None)
        .unwrap();
    let unit = TacticalUnit::from_spec(&example_spec(), object).unwrap();
    let document = current_document(&world, &unit);
    load_tactics(
        &mut world,
        vec![(SmolStr::new(format!("unit:{}", object.0)), document)],
    );
    assert!(!world.world().has_behavior(object, &unit_behavior()));
    assert!(matches!(
        store(&world).records[0].state,
        MechanicalState::Frozen { .. }
    ));
    assert_eq!(store(&world).binding_repairs[0].found_schema, None);
    assert!(world.take_module_ops().is_empty());
}

#[test]
fn stale_binding_is_reported_and_never_detached_at_boot() {
    let mut world = boot();
    let object = world
        .world_mut()
        .create(ObjKind::Thing, "Ghost", GOD, None)
        .unwrap();
    attach_current_binding(&mut world, object);
    load_tactics(&mut world, vec![]);
    assert!(world.world().has_behavior(object, &unit_behavior()));
    assert_eq!(store(&world).binding_repairs.len(), 1);
    assert_eq!(store(&world).binding_repairs[0].record_key, None);
    assert!(world.take_module_ops().is_empty());
}

#[test]
fn binding_repair_diagnostics_are_bounded_with_an_exact_total() {
    let mut world = boot();
    let total = MAX_UNIT_BINDING_REPAIRS + 9;
    for index in 0..total {
        let name = format!("Ghost {index}");
        let object = world
            .world_mut()
            .create(ObjKind::Thing, &name, GOD, None)
            .unwrap();
        attach_current_binding(&mut world, object);
    }
    load_tactics(&mut world, vec![]);
    assert_eq!(store(&world).binding_repair_count, total as u64);
    assert_eq!(
        store(&world).binding_repairs.len(),
        MAX_UNIT_BINDING_REPAIRS
    );
    assert!(
        store(&world)
            .boot_report
            .as_ref()
            .unwrap()
            .details_truncated
    );
}

#[test]
fn incident_ids_do_not_change_when_an_unrelated_earlier_record_is_added() {
    fn incident_for_key(world: &TestWorld, key: &str) -> mush_core::IncidentId {
        let record = store(world)
            .records
            .iter()
            .find(|record| record.key == key)
            .unwrap();
        match record.state {
            MechanicalState::Quarantined { incident } | MechanicalState::Frozen { incident } => {
                incident
            }
            _ => panic!("expected held record"),
        }
    }

    let mut first = boot();
    load_tactics(
        &mut first,
        vec![(SmolStr::new("unit:abc"), json!({"bad": true}))],
    );
    let expected = incident_for_key(&first, "unit:abc");

    let mut second = boot();
    load_tactics(
        &mut second,
        vec![
            (SmolStr::new("unit:aaa"), json!({"unrelated": true})),
            (SmolStr::new("unit:abc"), json!({"bad": true})),
        ],
    );
    assert_eq!(incident_for_key(&second, "unit:abc"), expected);
}

#[test]
fn malformed_unknown_noncanonical_and_miskeyed_records_are_quarantined_not_deleted() {
    let mut world = boot();
    let object = world
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, None)
        .unwrap();
    attach_current_binding(&mut world, object);
    let unit = TacticalUnit::from_spec(&example_spec(), object).unwrap();
    let mut unknown = current_document(&world, &unit);
    unknown["record_version"] = json!(99);
    let records = vec![
        (SmolStr::new("unit:abc"), json!("garbage")),
        (
            SmolStr::new(format!("unit:0{}", object.0)),
            current_document(&world, &unit),
        ),
        (
            SmolStr::new(format!("unit:{}", object.0 + 1)),
            current_document(&world, &unit),
        ),
        (SmolStr::new(format!("unit:{}", object.0)), unknown),
    ];
    world.take_module_ops();
    load_tactics(&mut world, records);
    assert_eq!(store(&world).records.len(), 4);
    assert!(
        store(&world)
            .records
            .iter()
            .all(|record| { matches!(record.state, MechanicalState::Quarantined { .. }) })
    );
    assert!(store(&world).units.is_empty());
    assert!(
        world.take_module_ops().is_empty(),
        "no bad raw key was deleted"
    );
}

#[test]
fn miskeyed_record_keeps_both_object_claims_and_cannot_be_overwritten() {
    let mut world = boot();
    let key_object = world
        .world_mut()
        .create(ObjKind::Thing, "Key shell", GOD, None)
        .unwrap();
    let embedded_object = world
        .world_mut()
        .create(ObjKind::Thing, "Embedded shell", GOD, None)
        .unwrap();
    attach_current_binding(&mut world, key_object);
    let unit = TacticalUnit::from_spec(&example_spec(), embedded_object).unwrap();
    let key = SmolStr::new(format!("unit:{}", key_object.0));
    let document = current_document(&world, &unit);
    load_tactics(&mut world, vec![(key.clone(), document)]);

    let record = &store(&world).records[0];
    assert_eq!(record.key, key);
    assert_eq!(record.key_object, Some(key_object));
    assert_eq!(record.object, Some(embedded_object));
    assert!(matches!(record.state, MechanicalState::Quarantined { .. }));

    world.take_module_ops();
    assert!(has_error(&world.run(
        GOD,
        &format!("@unit/create #{}=tacsim.mech.locust", key_object.0)
    )));
    assert!(has_error(&world.run(
        GOD,
        &format!("@behavior/remove #{}=tactics.unit", key_object.0)
    )));
    assert!(world.take_module_ops().is_empty());
    assert_eq!(store(&world).records.len(), 1);
    assert_eq!(store(&world).records[0].key, key);
}

#[test]
fn current_record_requires_the_exact_registered_envelope_schema() {
    let mut world = boot();
    let object = world
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, None)
        .unwrap();
    attach_current_binding(&mut world, object);
    let unit = TacticalUnit::from_spec(&example_spec(), object).unwrap();
    let mut document = current_document(&world, &unit);
    let wrong_schema = document["unit"]["schema"].clone();
    document["record_schema"] = wrong_schema;
    // The body is deliberately malformed as well: rejection must be driven by
    // the unauthenticated envelope identity before typed TacticalUnit decode.
    document["unit"] = json!("not a tactical unit");

    load_tactics(
        &mut world,
        vec![(SmolStr::new(format!("unit:{}", object.0)), document)],
    );

    assert!(!store(&world).units.contains_key(&object));
    assert!(matches!(
        store(&world).records[0].state,
        MechanicalState::Quarantined { .. }
    ));
    assert!(world.take_module_ops().is_empty());
}

#[test]
fn exact_record_schema_digest_conflict_freezes_tactics_and_all_intact_records() {
    let mut world = boot();
    let conflicted = world
        .world_mut()
        .create(ObjKind::Thing, "Conflicted", GOD, None)
        .unwrap();
    let dependent = world
        .world_mut()
        .create(ObjKind::Thing, "Dependent", GOD, None)
        .unwrap();
    for object in [conflicted, dependent] {
        attach_current_binding(&mut world, object);
    }
    let conflicted_unit = TacticalUnit::from_spec(&example_spec(), conflicted).unwrap();
    let dependent_unit = TacticalUnit::from_spec(&example_spec(), dependent).unwrap();
    let mut conflicted_document = current_document(&world, &conflicted_unit);
    let dependent_document = current_document(&world, &dependent_unit);
    let digest = conflicted_document["record_schema"]["digest"]
        .as_str()
        .unwrap()
        .to_string();
    let replacement = if digest.starts_with('0') { '1' } else { '0' };
    conflicted_document["record_schema"]["digest"] =
        json!(format!("{replacement}{}", &digest[1..]));
    load_tactics(
        &mut world,
        vec![
            (
                SmolStr::new(format!("unit:{}", conflicted.0)),
                conflicted_document,
            ),
            (
                SmolStr::new(format!("unit:{}", dependent.0)),
                dependent_document,
            ),
        ],
    );

    assert!(matches!(
        world
            .registry()
            .require_mechanics_owner_ready(&OwnerId::new("tactics").unwrap()),
        Err(mush_core::MechanicalOwnerReadinessError::Frozen { .. })
    ));
    assert!(
        world
            .registry()
            .require_mechanics_owner_ready(&OwnerId::new("tacsim").unwrap())
            .is_ok()
    );
    assert!(store(&world).units.is_empty());
    assert!(
        store(&world)
            .records
            .iter()
            .all(|record| matches!(record.state, MechanicalState::Frozen { .. }))
    );
}

#[test]
fn a_unit_envelope_under_a_non_unit_key_is_held_not_silently_filtered() {
    let mut world = boot();
    let object = world
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, None)
        .unwrap();
    attach_current_binding(&mut world, object);
    let unit = TacticalUnit::from_spec(&example_spec(), object).unwrap();
    world.take_module_ops();
    let document = current_document(&world, &unit);
    load_tactics(&mut world, vec![(SmolStr::new("misfiled:atlas"), document)]);

    assert_eq!(store(&world).records.len(), 1);
    assert_eq!(store(&world).records[0].key, "misfiled:atlas");
    assert_eq!(store(&world).records[0].object, Some(object));
    assert!(store(&world).records[0].unit.is_some());
    assert!(matches!(
        store(&world).records[0].state,
        MechanicalState::Quarantined { .. }
    ));
    assert!(store(&world).units.is_empty());
    assert!(
        store(&world).binding_repairs.is_empty(),
        "the decoded subject prevents a spurious binding-without-record repair"
    );
    assert!(world.take_module_ops().is_empty());
}

#[test]
fn dependency_digest_drift_freezes_owner_fans_out_and_leaves_other_owner_active() {
    let tacsim = example_spec();
    let auxsim = example_spec_for("auxsim", "auxsim.vehicle.truck");
    let vocabulary = MobilityVocabulary::base().freeze();
    let mut catalog = UnitCatalogBuilder::new();
    catalog.add(tacsim.clone(), &vocabulary).unwrap();
    catalog.add(auxsim.clone(), &vocabulary).unwrap();
    let mut world = TestWorld::with_modules(vec![
        Box::new(TestRulesModule {
            owner: "tacsim",
            specs: vec![tacsim.clone()],
        }),
        Box::new(TestRulesModule {
            owner: "auxsim",
            specs: vec![auxsim.clone()],
        }),
        Box::new(TacticsModule::with_catalog(catalog.freeze())),
    ]);

    let conflicted = world
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, None)
        .unwrap();
    let same_owner = world
        .world_mut()
        .create(ObjKind::Thing, "Locust", GOD, None)
        .unwrap();
    let unrelated = world
        .world_mut()
        .create(ObjKind::Thing, "Truck", GOD, None)
        .unwrap();
    for object in [conflicted, same_owner, unrelated] {
        attach_current_binding(&mut world, object);
    }
    let conflicted_unit = TacticalUnit::from_spec(&tacsim, conflicted).unwrap();
    let same_owner_unit = TacticalUnit::from_spec(&tacsim, same_owner).unwrap();
    let unrelated_unit = TacticalUnit::from_spec(&auxsim, unrelated).unwrap();
    let mut conflicted_document = current_document(&world, &conflicted_unit);
    let digest = conflicted_document["unit"]["rules"]["schema"]["digest"]
        .as_str()
        .unwrap()
        .to_string();
    let replacement = if digest.starts_with('0') { '1' } else { '0' };
    conflicted_document["unit"]["rules"]["schema"]["digest"] =
        json!(format!("{replacement}{}", &digest[1..]));
    // This lexically earlier exact reference is deliberately unavailable. Boot
    // must still scan the complete dependency set and latch the later digest
    // conflict instead of letting the missing reference mask an owner breach.
    conflicted_document["unit"]["capabilities"][0][1]["schema"]["key"]["id"] = json!("tacsim.a");
    let same_owner_document = current_document(&world, &same_owner_unit);
    let unrelated_document = current_document(&world, &unrelated_unit);
    load_tactics(
        &mut world,
        vec![
            (
                SmolStr::new(format!("unit:{}", conflicted.0)),
                conflicted_document,
            ),
            (
                SmolStr::new(format!("unit:{}", same_owner.0)),
                same_owner_document,
            ),
            (
                SmolStr::new(format!("unit:{}", unrelated.0)),
                unrelated_document,
            ),
        ],
    );
    for object in [conflicted, same_owner] {
        assert!(matches!(
            store(&world)
                .records
                .iter()
                .find(|record| record.object == Some(object))
                .unwrap()
                .state,
            MechanicalState::Frozen { .. }
        ));
        assert!(!store(&world).units.contains_key(&object));
    }
    assert_eq!(
        store(&world)
            .records
            .iter()
            .find(|record| record.object == Some(unrelated))
            .unwrap()
            .state,
        MechanicalState::Active
    );
    assert!(store(&world).units.contains_key(&unrelated));
    let tacsim_owner = OwnerId::new("tacsim").unwrap();
    let auxsim_owner = OwnerId::new("auxsim").unwrap();
    assert!(matches!(
        world
            .registry()
            .require_mechanics_owner_ready(&tacsim_owner),
        Err(mush_core::MechanicalOwnerReadinessError::Frozen { .. })
    ));
    assert!(
        world
            .registry()
            .require_mechanics_owner_ready(&auxsim_owner)
            .is_ok()
    );

    let damage = world.run(
        GOD,
        &format!("@unit/damage #{}={}", unrelated.0, armor_plan(-1)),
    );
    assert!(!has_error(&damage), "{}", texts(&damage));
    assert_eq!(armor(&world, unrelated), 5);
}

#[test]
fn parsable_orphan_cannot_mask_an_exact_dependency_digest_conflict() {
    let mut world = boot();
    let dependent = world
        .world_mut()
        .create(ObjKind::Thing, "Dependent", GOD, None)
        .unwrap();
    attach_current_binding(&mut world, dependent);
    let orphan = ObjId(900_000);
    let orphan_unit = TacticalUnit::from_spec(&example_spec(), orphan).unwrap();
    let dependent_unit = TacticalUnit::from_spec(&example_spec(), dependent).unwrap();
    let mut orphan_document = current_document(&world, &orphan_unit);
    let dependent_document = current_document(&world, &dependent_unit);
    let digest = orphan_document["unit"]["rules"]["schema"]["digest"]
        .as_str()
        .unwrap()
        .to_string();
    let replacement = if digest.starts_with('0') { '1' } else { '0' };
    orphan_document["unit"]["rules"]["schema"]["digest"] =
        json!(format!("{replacement}{}", &digest[1..]));

    load_tactics(
        &mut world,
        vec![
            (SmolStr::new(format!("unit:{}", orphan.0)), orphan_document),
            (
                SmolStr::new(format!("unit:{}", dependent.0)),
                dependent_document,
            ),
        ],
    );

    assert!(matches!(
        world
            .registry()
            .require_mechanics_owner_ready(&OwnerId::new("tacsim").unwrap()),
        Err(mush_core::MechanicalOwnerReadinessError::Frozen { .. })
    ));
    assert!(matches!(
        store(&world)
            .records
            .iter()
            .find(|record| record.key_object == Some(orphan))
            .unwrap()
            .state,
        MechanicalState::Quarantined { .. }
    ));
    assert!(matches!(
        store(&world)
            .records
            .iter()
            .find(|record| record.object == Some(dependent))
            .unwrap()
            .state,
        MechanicalState::Frozen { .. }
    ));
    assert!(!store(&world).units.contains_key(&dependent));
}

#[test]
fn coherent_v2_shape_drift_is_quarantined_against_the_exact_unit_spec() {
    let mut world = boot();
    let object = world
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, None)
        .unwrap();
    attach_current_binding(&mut world, object);
    let unit = TacticalUnit::from_spec(&example_spec(), object).unwrap();
    let mut document = current_document(&world, &unit);
    let heat = document["unit"]["cells"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|pair| pair[0].get("Unit").is_some())
        .expect("unit heat cell");
    heat[1]["max"] = json!(31);

    // The forged record remains internally coherent and carries unchanged,
    // globally valid exact references; only definition reconciliation catches
    // that it invented a different immutable unit shape.
    serde_json::from_value::<TacticalUnit>(document["unit"].clone()).unwrap();
    world.take_module_ops();
    load_tactics(
        &mut world,
        vec![(SmolStr::new(format!("unit:{}", object.0)), document)],
    );
    assert!(matches!(
        store(&world).records[0].state,
        MechanicalState::Quarantined { .. }
    ));
    assert!(store(&world).units.is_empty());
    assert!(world.take_module_ops().is_empty());
}

#[test]
fn duplicate_subjects_quarantine_every_claim_deterministically() {
    let mut world = boot();
    let object = world
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, None)
        .unwrap();
    attach_current_binding(&mut world, object);
    let unit = TacticalUnit::from_spec(&example_spec(), object).unwrap();
    let document = current_document(&world, &unit);
    load_tactics(
        &mut world,
        vec![
            (SmolStr::new(format!("unit:{}", object.0)), document.clone()),
            (SmolStr::new(format!("unit:{}", object.0)), document),
        ],
    );
    assert_eq!(store(&world).records.len(), 2);
    assert!(
        store(&world)
            .records
            .iter()
            .all(|record| { matches!(record.state, MechanicalState::Quarantined { .. }) })
    );
    assert!(store(&world).units.is_empty());
}

#[test]
fn invalid_object_kind_is_quarantined_with_same_create_boot_eligibility() {
    let mut world = boot();
    let room = world
        .world_mut()
        .create(ObjKind::Room, "Hangar", GOD, None)
        .unwrap();
    let unit = TacticalUnit::from_spec(&example_spec(), room).unwrap();
    let document = current_document(&world, &unit);
    load_tactics(
        &mut world,
        vec![(SmolStr::new(format!("unit:{}", room.0)), document)],
    );
    assert!(matches!(
        store(&world).records[0].state,
        MechanicalState::Quarantined { .. }
    ));
    assert!(has_error(&world.run(
        GOD,
        &format!("@unit/create #{}=tacsim.mech.locust", room.0)
    )));
}

#[test]
fn detach_gate_and_commands_cannot_mutate_held_records() {
    let mut world = boot();
    let object = world
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, None)
        .unwrap();
    attach_current_binding(&mut world, object);
    let unit = TacticalUnit::from_spec(&example_spec(), object).unwrap();
    let mut unknown = current_document(&world, &unit);
    unknown["record_version"] = json!(99);
    load_tactics(
        &mut world,
        vec![(SmolStr::new(format!("unit:{}", object.0)), unknown)],
    );
    assert!(has_error(&world.run(
        GOD,
        &format!("@behavior/remove #{}=tactics.unit", object.0)
    )));
    assert!(has_error(
        &world.run(GOD, &format!("@unit/remove #{}", object.0))
    ));
    assert!(has_error(&world.run(
        GOD,
        &format!("@unit/damage #{}={}", object.0, armor_plan(-1))
    )));
    assert!(has_error(
        &world.run(GOD, &format!("@destroy #{}", object.0))
    ));
    assert!(
        world
            .world()
            .get(object)
            .is_some_and(|object| !object.flags.contains(mush_core::Flags::GOING))
    );
    assert!(world.world().has_behavior(object, &unit_behavior()));
}

#[test]
fn destroying_active_unit_deletes_it_but_unrelated_destroy_does_not() {
    let mut world = boot();
    let object = make_unit(&mut world);
    let rock = world
        .world_mut()
        .create(ObjKind::Thing, "Rock", GOD, None)
        .unwrap();
    world.take_module_ops();
    world.world_mut().destroy(rock).unwrap();
    let mut out = Outbox::new();
    let mut effects = Effects::new();
    world.engine_mut().emit_world_event(
        &WorldEvent::ObjDestroyed { obj: rock },
        &mut out,
        &mut effects,
    );
    assert!(store(&world).units.contains_key(&object));
    assert!(world.take_module_ops().is_empty());

    world.world_mut().destroy(object).unwrap();
    world.engine_mut().emit_world_event(
        &WorldEvent::ObjDestroyed { obj: object },
        &mut out,
        &mut effects,
    );
    assert!(!store(&world).units.contains_key(&object));
    assert!(
        store(&world)
            .records
            .iter()
            .all(|record| record.object != Some(object))
    );
    assert_eq!(store(&world).boot_report.as_ref().unwrap().active, 0);
    assert!(world.take_module_ops().iter().any(|operation| {
        matches!(&operation.op, ModuleWrite::Del { key } if key == &format!("unit:{}", object.0))
    }));
}

#[test]
fn destroy_command_cannot_discard_a_unit_while_a_dependency_owner_is_frozen() {
    let mut world = boot();
    let object = make_unit(&mut world);
    world.take_module_ops();
    world
        .registry()
        .freeze_mechanics_owner(
            &OwnerId::new("tacsim").unwrap(),
            mush_core::IncidentId(879),
            RecoveryReason::IntegrityViolation {
                code: mush_core::CanonicalId::new("test_pre_destroy_owner_freeze").unwrap(),
            },
        )
        .unwrap();

    let output = world.run(GOD, &format!("@destroy #{}", object.0));
    assert!(has_error(&output), "{}", texts(&output));
    assert!(
        world
            .world()
            .get(object)
            .is_some_and(|object| !object.flags.contains(mush_core::Flags::GOING))
    );
    assert!(store(&world).units.contains_key(&object));
    assert!(world.take_module_ops().is_empty());
}

#[test]
fn destroy_after_owner_freeze_preserves_record_and_only_removes_object_shell() {
    let mut world = boot();
    let object = make_unit(&mut world);
    world.take_module_ops();
    let owner = OwnerId::new("tacsim").unwrap();
    world
        .registry()
        .freeze_mechanics_owner(
            &owner,
            mush_core::IncidentId(880),
            RecoveryReason::IntegrityViolation {
                code: mush_core::CanonicalId::new("test_runtime_owner_freeze").unwrap(),
            },
        )
        .unwrap();

    world.world_mut().destroy(object).unwrap();
    let mut out = Outbox::new();
    let mut effects = Effects::new();
    world.engine_mut().emit_world_event(
        &WorldEvent::ObjDestroyed { obj: object },
        &mut out,
        &mut effects,
    );

    assert!(
        world
            .world()
            .get(object)
            .is_some_and(|object| object.flags.contains(mush_core::Flags::GOING)),
        "the post-destroy observer cannot veto the shell's prior GOING transition"
    );
    assert!(!store(&world).units.contains_key(&object));
    let record = store(&world)
        .records
        .iter()
        .find(|record| record.object == Some(object))
        .expect("authoritative module record remains in custody");
    assert_eq!(
        record.state,
        MechanicalState::Frozen {
            incident: mush_core::IncidentId(880)
        }
    );
    let report = store(&world).boot_report.as_ref().unwrap();
    assert_eq!(report.active, 0);
    assert_eq!(report.frozen, 1);
    assert!(world.take_module_ops().iter().all(|operation| {
        !matches!(&operation.op, ModuleWrite::Del { key } if key == &format!("unit:{}", object.0))
    }));
}

#[test]
fn recovery_command_exposes_bounded_summary_without_raw_payload() {
    let mut world = boot();
    load_tactics(
        &mut world,
        vec![(SmolStr::new("unit:abc"), json!({"secret": "do-not-print"}))],
    );
    let output = texts(&world.run(GOD, "@unit/recovery"));
    assert!(output.contains("1 quarantined") && output.contains("incident"));
    assert!(!output.contains("do-not-print"));
}

#[test]
fn unit_commands_are_staff_only() {
    let mut world = boot();
    let object = make_unit(&mut world);
    let player = world.spawn_player("Joe");
    assert!(has_error(
        &world.run(player, &format!("@unit/show #{}", object.0))
    ));
    assert!(has_error(&world.run(player, "@unit/recovery")));
    assert!(store(&world).units.contains_key(&object));
}

// ---- E3c: the component-asset scrub (doc 08b §5.2) --------------------

fn kitted_component() -> ComponentId {
    ComponentId::new("gauss").unwrap()
}
fn asset_of(tw: &TestWorld, unit: ObjId) -> Option<ObjId> {
    store(tw)
        .units
        .get(&unit)
        .unwrap()
        .component(&kitted_component())
        .unwrap()
        .asset
}
fn revision_of(tw: &TestWorld, unit: ObjId) -> u64 {
    store(tw).units.get(&unit).unwrap().revision()
}
/// A `tacsim.mech.atlas` unit whose `gauss` component points at a live object.
fn kitted_unit(tw: &mut TestWorld) -> (ObjId, ObjId) {
    let mech = tw
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, None)
        .unwrap();
    let rifle = tw
        .world_mut()
        .create(ObjKind::Thing, "Gauss Rifle", GOD, None)
        .unwrap();
    let ev = tw.run(GOD, &format!("@unit/create #{}=tacsim.mech.atlas", mech.0));
    assert!(!has_error(&ev), "create refused: {}", texts(&ev));
    let ev = tw.run(GOD, &format!(r#"@unit/damage #{}={{"ops":[{{"SetComponentAsset":{{"component":"gauss","asset":{}}}}}]}}"#, mech.0, rifle.0));
    assert!(!has_error(&ev), "SetComponentAsset refused: {}", texts(&ev));
    assert_eq!(asset_of(tw, mech), Some(rifle), "the pointer did not take");
    (mech, rifle)
}

#[test]
fn set_component_asset_refuses_an_unknown_component_and_bumps_the_revision() {
    let mut tw = boot();
    let (mech, rifle) = kitted_unit(&mut tw);
    assert_eq!(
        revision_of(&tw, mech),
        1,
        "the successful op bumped the revision once"
    );

    // An unresolved referent is refused exactly like every other one, and the
    // refused plan leaves the unit untouched (M32 refuse-not-clamp).
    let bad = r#"{"ops":[{"SetComponentAsset":{"component":"nope","asset":null}}]}"#;
    assert!(
        has_error(&tw.run(GOD, &format!("@unit/damage #{}={}", mech.0, bad))),
        "unknown component refused"
    );
    assert_eq!(
        asset_of(&tw, mech),
        Some(rifle),
        "a refused plan changed the pointer"
    );
    assert_eq!(
        revision_of(&tw, mech),
        1,
        "a refused plan bumped the revision"
    );

    // Clearing it explicitly is legal and bumps again.
    let clear = r#"{"ops":[{"SetComponentAsset":{"component":"gauss","asset":null}}]}"#;
    let ev = tw.run(GOD, &format!("@unit/damage #{}={}", mech.0, clear));
    assert!(!has_error(&ev), "clearing refused: {}", texts(&ev));
    assert_eq!(asset_of(&tw, mech), None);
    assert_eq!(revision_of(&tw, mech), 2);
}

#[test]
fn destroying_the_asset_object_scrubs_the_component_pointer() {
    let mut tw = boot();
    let (mech, rifle) = kitted_unit(&mut tw);
    tw.take_module_ops();

    tw.world_mut().destroy(rifle).unwrap();
    let mut out = Outbox::new();
    let mut fxx = Effects::new();
    tw.engine_mut()
        .emit_world_event(&WorldEvent::ObjDestroyed { obj: rifle }, &mut out, &mut fxx);

    assert_eq!(
        asset_of(&tw, mech),
        None,
        "the dangling asset pointer survived its referent"
    );
    assert_eq!(
        revision_of(&tw, mech),
        2,
        "the scrub is a real revision bump, not a silent doc edit"
    );
    assert!(
        tw.take_module_ops().iter().any(|o| matches!(&o.op, ModuleWrite::JsonSet { key, .. } if key == &format!("unit:{}", mech.0))),
        "the repaired unit doc was not persisted"
    );
}

#[test]
fn boot_scrubs_a_pointer_to_a_missing_object_without_the_event() {
    // The backstop half: only `@destroy` emits `ObjDestroyed`, so a
    // programmatic destruction during downtime is caught at boot instead.
    let mut tw = boot();
    let (mech, rifle) = kitted_unit(&mut tw);
    let kv = fold_kv(&mut tw);
    tw.world_mut().destroy(rifle).unwrap(); // no event emitted

    tw.modules_loaded(HashMap::from([(
        SmolStr::new("tactics"),
        ModuleBootData {
            kv,
            streams: HashMap::new(),
            ..Default::default()
        },
    )]));
    assert_eq!(
        asset_of(&tw, mech),
        None,
        "boot left a pointer to a GOING object"
    );
    assert_eq!(
        revision_of(&tw, mech),
        2,
        "the boot scrub bumps the revision too"
    );
    assert!(
        tw.take_module_ops().iter().any(|o| matches!(&o.op, ModuleWrite::JsonSet { key, .. } if key == &format!("unit:{}", mech.0))),
        "the repaired doc was not written back"
    );
}

#[test]
fn a_live_asset_pointer_survives_boot_and_unrelated_destruction() {
    let mut tw = boot();
    let (mech, rifle) = kitted_unit(&mut tw);
    let other = tw
        .world_mut()
        .create(ObjKind::Thing, "Rock", GOD, None)
        .unwrap();
    tw.world_mut().destroy(other).unwrap();
    let mut out = Outbox::new();
    let mut fxx = Effects::new();
    tw.engine_mut()
        .emit_world_event(&WorldEvent::ObjDestroyed { obj: other }, &mut out, &mut fxx);
    assert_eq!(
        asset_of(&tw, mech),
        Some(rifle),
        "an unrelated destroy scrubbed a live pointer"
    );
    assert_eq!(
        revision_of(&tw, mech),
        1,
        "an unrelated destroy bumped a revision"
    );

    let kv = fold_kv(&mut tw);
    tw.modules_loaded(HashMap::from([(
        SmolStr::new("tactics"),
        ModuleBootData {
            kv,
            streams: HashMap::new(),
            ..Default::default()
        },
    )]));
    assert_eq!(
        asset_of(&tw, mech),
        Some(rifle),
        "boot scrubbed a pointer to a LIVE object"
    );
    assert_eq!(revision_of(&tw, mech), 1, "a clean boot bumped a revision");
}

#[test]
fn exact_reference_constructor_remains_owner_kind_version_and_digest_bound() {
    let reference = example_spec().definition;
    let rebuilt = DefinitionRef::new(
        DefinitionKey::new(
            reference.key().owner.clone(),
            reference.key().kind.clone(),
            reference.key().id.clone(),
        ),
        reference.version(),
        DefinitionDigest::from_hex(&reference.digest().to_hex()).unwrap(),
    )
    .unwrap();
    assert_eq!(rebuilt, reference);
}

// ---- E7c8: B2 — `unit.status` is a PROJECTION, never a document ----

/// Publish a station grant for `viewer` on `unit`, the way `mush-sim`'s
/// `links::refresh_actor_links` does. This test crate has no `mush-sim` edge, so
/// the grant is published directly through `World` — which is the point: the
/// projection reads K5a's grant, not any simulation type.
fn grant_station(world: &mut TestWorld, viewer: ObjId, unit: ObjId) {
    world
        .world_mut()
        .publish_command_subject(mush_core::CommandSubjectLink {
            actor: viewer,
            subject: unit,
            owner: SmolStr::new("sim"),
            station: SmolStr::new("gunnery"),
            relation: SmolStr::new("gunner"),
            priority: 10,
            channels: Default::default(),
            epoch: 1,
        });
}

/// **`unit.status` is a projection, not a document.** Composing it from the
/// stores and composing it from a `TacticalUnit` value that was **never
/// persisted** produce byte-identical results — which is only possible because
/// nothing stores a status anywhere.
#[test]
fn e7c8_b2_unit_status_is_byte_identical_from_the_store_and_from_a_never_persisted_fixture() {
    let mut tw = boot();
    let viewer = tw
        .world_mut()
        .create(ObjKind::Player, "Gunner", GOD, None)
        .unwrap();
    let unit = make_unit(&mut tw);
    grant_station(&mut tw, viewer, unit);

    let from_store = mush_tactics::status::unit_status(tw.world(), viewer, unit);
    let fixture = tw
        .world()
        .module_state::<UnitStore>()
        .and_then(|store| store.units.get(&unit).cloned())
        .expect("a live unit");
    let from_fixture = mush_tactics::status::unit_status_of(tw.world(), viewer, &fixture);
    assert_eq!(from_store, from_fixture);
    assert_eq!(from_store.render(), from_fixture.render());
    assert!(from_store.authorized);
    assert!(from_store.sections > 0, "a real unit has sections");

    // And the composition WROTE nothing: no `tactics` document, of any key.
    let ops: Vec<_> = tw
        .take_module_ops()
        .into_iter()
        .filter(|op| op.ns == "tactics")
        .collect();
    let _ = mush_tactics::status::unit_status(tw.world(), viewer, unit);
    let after: Vec<_> = tw
        .take_module_ops()
        .into_iter()
        .filter(|op| op.ns == "tactics")
        .collect();
    assert!(
        after.is_empty(),
        "composing a status wrote {} document(s); it is a projection",
        after.len()
    );
    let _ = ops;
}

/// **What a viewer may not learn.** A viewer with no station on the unit, a
/// viewer with a station on an object that is not a unit, and a viewer asking
/// about an object that does not exist all receive the **byte-identical**
/// rendering — the status never confirms that a unit is there (§4 rule 4).
#[test]
fn e7c8_b2_unstationed_not_a_unit_and_nonexistent_all_render_identically() {
    let mut tw = boot();
    let viewer = tw
        .world_mut()
        .create(ObjKind::Player, "Gunner", GOD, None)
        .unwrap();
    let unit = make_unit(&mut tw);
    let husk = tw
        .world_mut()
        .create(ObjKind::Thing, "Crate", GOD, None)
        .unwrap();

    // (a) stationed nowhere, asking about a real unit.
    let unstationed = mush_tactics::status::unit_status(tw.world(), viewer, unit);
    // (b) stationed on an object that carries no unit record.
    grant_station(&mut tw, viewer, husk);
    let not_a_unit = mush_tactics::status::unit_status(tw.world(), viewer, husk);
    // (c) asking about an object that does not exist at all.
    let nonexistent = mush_tactics::status::unit_status(tw.world(), viewer, ObjId(9_999));

    assert_eq!(unstationed, not_a_unit);
    assert_eq!(not_a_unit, nonexistent);
    assert_eq!(unstationed.render(), nonexistent.render());
    assert!(!unstationed.authorized);
    assert_eq!(unstationed, mush_tactics::status::UnitStatus::unknown());

    // The stationed viewer of the REAL unit does see it — so the identity above
    // is a fog property, not a broken read.
    grant_station(&mut tw, viewer, unit);
    let seen = mush_tactics::status::unit_status(tw.world(), viewer, unit);
    assert!(seen.authorized);
    assert_ne!(seen.render(), unstationed.render());
}

/// One station's contact picture never leaks through another's status line: the
/// contact count is the **viewer's own** ledger rows, so two viewers of the same
/// unit see their own counts.
#[test]
fn e7c8_b2_the_contact_count_is_the_viewers_own_never_the_units() {
    let mut tw = boot();
    let one = tw
        .world_mut()
        .create(ObjKind::Player, "Alpha", GOD, None)
        .unwrap();
    let two = tw
        .world_mut()
        .create(ObjKind::Player, "Bravo", GOD, None)
        .unwrap();
    let unit = make_unit(&mut tw);
    grant_station(&mut tw, one, unit);
    grant_station(&mut tw, two, unit);

    let mut ledger = mush_core::ContactLedgerState::default();
    ledger.install(
        vec![mush_core::ContactLedgerRow {
            holder: mush_core::PrincipalId::Object { id: one },
            contact: mush_core::ContactId::from_bytes([7_u8; 16]),
            generation: 1,
            revision: 1,
            active: true,
            observed_at: mush_core::ClockStamp {
                clock: mush_core::ClockRef::WorldIc { generation: 1 },
                at: 5,
            },
            phantom: None,
        }],
        mush_core::ClockStamp {
            clock: mush_core::ClockRef::WorldIc { generation: 1 },
            at: 5,
        },
    );
    if let Some(slot) = tw
        .world_mut()
        .module_state_mut::<mush_core::ContactLedgerState>()
    {
        *slot = ledger;
    }

    let alpha = mush_tactics::status::unit_status(tw.world(), one, unit);
    let bravo = mush_tactics::status::unit_status(tw.world(), two, unit);
    assert_eq!(alpha.contacts, 1);
    assert_eq!(bravo.contacts, 0, "one holder's picture leaked to another");
    // Everything else about the same unit is identical for both viewers.
    assert_eq!(alpha.revision, bravo.revision);
    assert_eq!(alpha.sections, bravo.sections);
    assert_eq!(alpha.components, bravo.components);
}

// ---- E7 cut 8 independent verify fold ----

/// **A12 (LOW).** `unit.status` authorizes on a **`sim`** grant, not on any
/// grant at all. `viewer_is_stationed` used to be
/// `grant.subject == unit` with no owner filter — contrary to its own doc — so
/// any future module publishing a `CommandSubjectLink` on that object would have
/// silently conferred read access to the unit's true revision, section and
/// component counts and its `Obj.location`. A foreign-owner grant now renders
/// **byte-identically to no grant at all**.
#[test]
fn vf_a12_a_foreign_owner_grant_renders_unit_status_byte_identically_to_no_grant_at_all() {
    let mut tw = boot();
    let viewer = tw
        .world_mut()
        .create(ObjKind::Player, "Gunner", GOD, None)
        .unwrap();
    let unit = make_unit(&mut tw);
    let nothing = mush_tactics::status::unit_status(tw.world(), viewer, unit);
    // A grant published by SOME OTHER owner on the same object.
    tw.world_mut()
        .publish_command_subject(mush_core::CommandSubjectLink {
            actor: viewer,
            subject: unit,
            owner: SmolStr::new("industry"),
            station: SmolStr::new("gunnery"),
            relation: SmolStr::new("gunner"),
            priority: 10,
            channels: Default::default(),
            epoch: 1,
        });
    let foreign = mush_tactics::status::unit_status(tw.world(), viewer, unit);
    assert_eq!(
        foreign, nothing,
        "a foreign owner's link must confer no station at all"
    );
    assert_eq!(foreign.render(), nothing.render());
    assert!(!mush_tactics::status::viewer_is_stationed(
        tw.world(),
        viewer,
        unit
    ));
    // And the `sim` grant still does authorize, so the fence is a filter and
    // not a break.
    grant_station(&mut tw, viewer, unit);
    assert!(mush_tactics::status::viewer_is_stationed(
        tw.world(),
        viewer,
        unit
    ));
    let seen = mush_tactics::status::unit_status(tw.world(), viewer, unit);
    assert!(seen.authorized);
    assert_ne!(seen.render(), nothing.render());
}

/// **A12, the caller half.** The projection has a real surface now: `@unit/status`
/// runs it for an ordinary (non-staff) viewer, and an object the viewer holds no
/// `sim` station on reads byte-identically to one that never existed.
#[test]
fn vf_a12b_the_unit_status_verb_exists_and_hides_existence_for_an_ordinary_viewer() {
    let mut tw = boot();
    let viewer = tw
        .world_mut()
        .create(ObjKind::Player, "Gunner", GOD, None)
        .unwrap();
    let unit = make_unit(&mut tw);
    let unstationed = tw.run(viewer, &format!("@unit/status #{}", unit.0));
    let nonexistent = tw.run(viewer, "@unit/status #999999");
    let text = |events: &[(mush_core::Audience, mush_proto::OutputEvent)]| {
        events
            .iter()
            .map(|(_, event)| mush_testkit::text_of(event))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(
        text(&unstationed),
        text(&nonexistent),
        "an unauthorized read and a nonexistent one must be byte-identical"
    );
    grant_station(&mut tw, viewer, unit);
    let stationed = tw.run(viewer, &format!("@unit/status #{}", unit.0));
    assert_ne!(
        text(&stationed),
        text(&nonexistent),
        "a stationed viewer sees the unit, so the identity above is a fog rule"
    );
}

// ---- Interface plan I8: the unit view ----------------------------------

/// A kitted, mobile, capability-bearing unit whose rule bodies carry MARKERS,
/// so a projection that leaks any of them is caught by a text scan.
fn i8_spec() -> UnitSpec {
    let mut definitions = DefinitionRegistryBuilder::new();
    let opaque = register_ref(
        &mut definitions,
        "tacsim",
        "schema",
        "tacsim.i8x",
        json!({}),
    );
    let kinematics = register_ref(
        &mut definitions,
        "tacsim",
        "schema",
        "tacsim.i8kin",
        json!({}),
    );
    let gauss = register_ref(
        &mut definitions,
        "tacsim",
        "component",
        "tacsim.weapon.i8gauss",
        json!({}),
    );
    let vocabulary = MobilityVocabulary::base().freeze();
    let biped = MobilityModeId::new("ground.biped").unwrap();
    let profile = MobilityProfile::new(
        vec![
            MobilityMode::new(
                biped.clone(),
                [MediumId::new("ground").unwrap()],
                LocomotionId::new("legged").unwrap(),
                ModeRules::new(kinematics.clone(), json!({"kin": "kin-marker"})).unwrap(),
            ),
            MobilityMode::new(
                MobilityModeId::new("ground.quad").unwrap(),
                [MediumId::new("ground").unwrap()],
                LocomotionId::new("legged").unwrap(),
                ModeRules::new(kinematics, json!({"kin": "kin-marker-2"})).unwrap(),
            ),
        ],
        biped,
        &vocabulary,
    )
    .unwrap();
    let body = UnitSpecBody {
        sections: vec![SectionSpec {
            id: SectionId::new("center_torso").unwrap(),
            condition: ConditionId::new("ok").unwrap(),
            facets: vec![FacetSpec {
                id: FacetId::new("rear").unwrap(),
                cells: vec![CellSpec {
                    id: CellId::new("armor").unwrap(),
                    min: 0,
                    max: 6,
                    init: 6,
                }],
            }],
            slots: vec![InstallSlotId::new("torso_hardpoint").unwrap()],
            cells: vec![CellSpec {
                id: CellId::new("structure").unwrap(),
                min: 0,
                max: 8,
                init: 8,
            }],
        }],
        unit_cells: vec![CellSpec {
            id: CellId::new("heat").unwrap(),
            min: 0,
            max: 30,
            init: 20,
        }],
        components: vec![ComponentSpec {
            id: ComponentId::new("gauss").unwrap(),
            definition: gauss,
            install: InstallPoint::Slot {
                section: SectionId::new("center_torso").unwrap(),
                slot: InstallSlotId::new("torso_hardpoint").unwrap(),
            },
            condition: ConditionId::new("ok").unwrap(),
            cells: vec![CellSpec {
                id: CellId::new("ammo").unwrap(),
                min: 0,
                max: 12,
                init: 9,
            }],
            asset: None,
        }],
        capabilities: vec![CapabilityInit {
            key: CapabilityKey {
                id: CapabilityId::new("tactics.mobility").unwrap(),
                source: None,
            },
            rules: RulesValue::new(opaque.clone(), json!({"cap": "cap-rules-marker"})).unwrap(),
        }],
        mobility: Some(profile),
        rules: RulesValue::new(opaque, json!({"secret": "rules-body-marker"})).unwrap(),
    };
    let descriptor = body
        .descriptor(
            DefinitionKind::new("unit").unwrap(),
            DefinitionId::new("tacsim.mech.i8").unwrap(),
            1,
        )
        .unwrap();
    let definition = definitions
        .register(OwnerId::new("tacsim").unwrap(), descriptor)
        .unwrap();
    body.with_definition(definition)
}

fn boot_i8() -> TestWorld {
    let vocabulary = MobilityVocabulary::base().freeze();
    let specs = vec![example_spec(), kitted_spec(), i8_spec()];
    let mut catalog = UnitCatalogBuilder::new();
    for spec in &specs {
        catalog.add(spec.clone(), &vocabulary).unwrap();
    }
    let modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(TestRulesModule {
            owner: "tacsim",
            specs,
        }),
        Box::new(TacticsModule::with_catalog(catalog.freeze())),
    ];
    TestWorld::with_modules(modules)
}

fn i8_player(tw: &mut TestWorld, name: &str) -> ObjId {
    tw.world_mut()
        .create(ObjKind::Player, name, GOD, None)
        .unwrap()
}

fn i8_unit_owned_by(tw: &mut TestWorld, owner: ObjId) -> ObjId {
    let object = tw
        .world_mut()
        .create(ObjKind::Thing, "Halberd", owner, None)
        .unwrap();
    let output = tw.run(GOD, &format!("@unit/create #{}=tacsim.mech.i8", object.0));
    assert!(!has_error(&output), "create refused: {}", texts(&output));
    object
}

fn i8_unit(tw: &mut TestWorld) -> ObjId {
    i8_unit_owned_by(tw, GOD)
}

fn i8_grant(tw: &mut TestWorld, viewer: ObjId, unit: ObjId, priority: u16) {
    tw.world_mut()
        .publish_command_subject(mush_core::CommandSubjectLink {
            actor: viewer,
            subject: unit,
            owner: SmolStr::new("sim"),
            station: SmolStr::new(format!("station-{priority}")),
            relation: SmolStr::new("gunner"),
            priority,
            channels: Default::default(),
            epoch: 1,
        });
}

fn i8_query(
    tw: &TestWorld,
    feed: &str,
    actor: ObjId,
    params: Value,
) -> Result<OutputEvent, mush_proto::RichText> {
    let (_, handler) = tw
        .registry()
        .query_handler(feed)
        .unwrap_or_else(|| panic!("{feed} has a registered query handler"));
    handler.query(tw.world(), actor, &params)
}

fn i8_keyframe(result: Result<OutputEvent, mush_proto::RichText>, feed: &str) -> Value {
    match result {
        Ok(OutputEvent::Panel {
            feed: f,
            kind: PanelFrameKind::Keyframe,
            data,
        }) if f == feed => data,
        other => panic!("expected a {feed} keyframe, got {other:?}"),
    }
}

fn i8_frames_to(
    events: &[(Audience, OutputEvent)],
    feed: &str,
    kind: PanelFrameKind,
    who: ObjId,
) -> Vec<Value> {
    events
        .iter()
        .filter_map(|(aud, event)| match event {
            OutputEvent::Panel {
                feed: f,
                kind: k,
                data,
            } if f == feed && *k == kind && *aud == Audience::Actor(who) => Some(data.clone()),
            _ => None,
        })
        .collect()
}

fn i8_assert_clean(surface: &str, text: &str, forbidden: &[&str]) {
    for fragment in forbidden {
        assert!(
            !text.contains(fragment),
            "{surface} leaks {fragment:?}: {text}"
        );
    }
}

const I8_FORBIDDEN: &[&str] = &[
    "rules-body-marker",
    "kin-marker",
    "cap-rules-marker",
    "\"rules\"",
    "offer",
    "commit",
    "incident",
];

fn i8_damage(tw: &mut TestWorld, unit: ObjId, ops: &str) -> Vec<(Audience, OutputEvent)> {
    let ev = tw.run(
        GOD,
        &format!(r#"@unit/damage #{}={{"ops":[{ops}]}}"#, unit.0),
    );
    assert!(!has_error(&ev), "{}", texts(&ev));
    ev
}

fn i8_frame_of(tw: &TestWorld, feed: &str, who: ObjId, unit: ObjId) -> Value {
    i8_keyframe(
        i8_query(tw, feed, who, json!({ "object": format!("#{}", unit.0) })),
        feed,
    )
}

#[test]
fn i8_p1_the_keyframe_carries_no_rules_no_definition_body_and_no_foreign_object_id() {
    let mut tw = boot_i8();
    let unit = i8_unit(&mut tw);
    let rifle = tw
        .world_mut()
        .create(ObjKind::Thing, "Rifle", GOD, None)
        .unwrap();
    i8_damage(
        &mut tw,
        unit,
        &format!(
            r#"{{"SetComponentAsset":{{"component":"gauss","asset":{}}}}}"#,
            rifle.0
        ),
    );
    let frame = i8_frame_of(&tw, "tactics.unit", GOD, unit);
    let text = frame.to_string();
    i8_assert_clean("tactics.unit keyframe", &text, I8_FORBIDDEN);
    assert!(
        !text.contains(&format!("#{}", rifle.0)),
        "the asset dbref leaked: {text}"
    );
    assert_eq!(frame["components"][0]["asset"], json!(true));
    assert_eq!(frame["components"][0]["id"], json!("gauss"));
    assert_eq!(
        frame["components"][0]["install"],
        json!("slot/center_torso/torso_hardpoint")
    );
    assert!(
        frame["schema"]
            .as_str()
            .unwrap()
            .starts_with("tacsim:unit:tacsim.mech.i8@1#")
    );
    assert_eq!(
        frame["capabilities"],
        json!([{ "id": "tactics.mobility", "source": null }])
    );
    assert_eq!(frame["mobility"]["mode"], json!("ground.biped"));
    assert_eq!(frame["mobility"]["switching"], Value::Null);
    assert_eq!(frame["mobility"]["modes"][0]["locomotion"], json!("legged"));
    assert!(frame["mobility"]["modes"][0].get("rules").is_none());
    // The old `project` serialized the whole UnitRepr; the fixture's markers
    // would have been on the wire.
    let raw = serde_json::to_value(store(&tw).units.get(&unit).unwrap())
        .unwrap()
        .to_string();
    assert!(
        raw.contains("rules-body-marker"),
        "the fixture really carries the marker"
    );
}

#[test]
fn i8_p2_the_delta_projection_carries_no_asset_object_id_and_keeps_to_revision_at_the_root() {
    let mut tw = boot_i8();
    let unit = i8_unit(&mut tw);
    let rifle = tw
        .world_mut()
        .create(ObjKind::Thing, "Rifle", GOD, None)
        .unwrap();
    let ev = i8_damage(
        &mut tw,
        unit,
        &format!(
            r#"{{"SetComponentAsset":{{"component":"gauss","asset":{}}}}},{{"AdjustCell":{{"addr":{{"Facet":{{"section":"center_torso","facet":"rear","cell":"armor"}}}},"delta":-5}}}},{{"SetSectionCondition":{{"section":"center_torso","condition":"damaged"}}}}"#,
            rifle.0
        ),
    );
    let deltas = frames(&ev, PanelFrameKind::Delta);
    assert_eq!(deltas.len(), 1);
    let delta = &deltas[0];
    assert_eq!(delta["to_revision"], json!(1));
    assert_eq!(delta["from_revision"], json!(0));
    assert_eq!(delta["object"]["id"], json!(format!("#{}", unit.0)));
    let entries = delta["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(
        entries[0],
        json!({ "kind": "asset", "component": "gauss", "attached": true })
    );
    assert_eq!(
        entries[1],
        json!({ "kind": "cell", "addr": "facet/center_torso/rear/armor", "from": 6, "to": 1 })
    );
    assert_eq!(
        entries[2],
        json!({ "kind": "section", "section": "center_torso", "from": "ok", "to": "damaged" })
    );
    let text = delta.to_string();
    assert!(!text.contains(&format!("#{}", rifle.0)));
    assert!(!text.contains(&format!("\"asset\":{}", rifle.0)));
    i8_assert_clean("tactics.unit delta", &text, I8_FORBIDDEN);
}

#[test]
fn i8_p3_named_refusals_are_byte_identical_for_stranger_nonexistent_non_unit_malformed_and_staff() {
    let mut tw = boot_i8();
    let unit = i8_unit(&mut tw);
    let stranger = i8_player(&mut tw, "Stranger");
    let brick = tw
        .world_mut()
        .create(ObjKind::Thing, "Brick", GOD, None)
        .unwrap();
    let refusal = mush_tactics::ViewRefusal.render();
    assert_eq!(refusal, "You have no station on that unit.");
    let cases: Vec<(ObjId, Value)> = vec![
        (stranger, json!({ "object": format!("#{}", unit.0) })),
        (stranger, json!({ "object": "#999" })),
        (stranger, json!({ "object": format!("#{}", brick.0) })),
        (stranger, json!({ "object": "#x" })),
        (stranger, json!({ "object": 7 })),
        (GOD, json!({ "object": "#999" })),
        (GOD, json!({ "object": format!("#{}", brick.0) })),
    ];
    for feed in ["tactics.unit", "tactics.vitals"] {
        let mut seen = Vec::new();
        for (who, params) in &cases {
            let err = i8_query(&tw, feed, *who, params.clone())
                .expect_err("a named failure is a refusal");
            seen.push(format!("{err:?}"));
        }
        assert!(
            seen.iter().all(|s| s == &seen[0]),
            "{feed} refusals differ: {seen:#?}"
        );
        assert!(seen[0].contains(&refusal));
    }
}

#[test]
fn i8_p4_a_held_record_answers_state_frozen_with_the_record_key_and_no_incident() {
    let spec = example_spec();
    let object = ObjId(2);
    let unit = TacticalUnit::from_spec(&spec, object).unwrap();
    let mut tw = boot_with_provider(false);
    let actual = tw
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, None)
        .unwrap();
    assert_eq!(actual, object);
    attach_current_binding(&mut tw, object);
    tw.take_module_ops();
    let document = current_document(&tw, &unit);
    load_tactics(
        &mut tw,
        vec![(SmolStr::new(format!("unit:{}", object.0)), document)],
    );
    assert!(matches!(
        store(&tw).records[0].state,
        MechanicalState::Frozen { .. }
    ));
    let frame = i8_keyframe(
        i8_query(&tw, "tactics.unit", GOD, json!({ "object": "#2" })),
        "tactics.unit",
    );
    assert_eq!(frame["state"], json!("frozen"));
    assert_eq!(frame["record"], json!("unit:2"));
    assert_eq!(frame["sections"], json!([]));
    assert_eq!(frame["cells"], json!([]));
    assert_eq!(frame["components"], json!([]));
    assert_eq!(frame["mobility"], Value::Null);
    assert_eq!(frame["counts"]["cells"], json!(0));
    assert_eq!(frame["truncated"], json!(false));
    assert_eq!(frame["revision"], json!(unit.revision()));
    assert_eq!(frame["schema"], json!(unit.schema().to_string()));
    i8_assert_clean(
        "held keyframe",
        &frame.to_string(),
        &["incident", "Migrating", "\"rules\""],
    );
    let shown = texts(&tw.run(GOD, "@unit/show #2"));
    assert!(
        shown.contains("tactical record unit:2 is frozen; mechanics are disabled"),
        "{shown}"
    );
    assert!(!shown.contains("incident"), "{shown}");
    // An active frame carries the same key with `null`.
    let mut live = boot_i8();
    let live_unit = i8_unit(&mut live);
    let frame = i8_frame_of(&live, "tactics.unit", GOD, live_unit);
    assert_eq!(frame["record"], Value::Null);
    assert_eq!(frame["state"], json!("active"));
}

#[test]
fn i8_c1_the_dto_and_at_unit_show_name_the_same_sections_cells_components_and_capabilities() {
    let mut tw = boot_i8();
    let unit = i8_unit(&mut tw);
    let frame = i8_frame_of(&tw, "tactics.unit", GOD, unit);
    let shown = texts(&tw.run(GOD, &format!("@unit/show #{}", unit.0)));
    let lines: Vec<&str> = shown.lines().collect();
    let mut expected: Vec<String> = Vec::new();
    for s in frame["sections"].as_array().unwrap() {
        expected.push(format!(
            "  section {}: {}",
            s["id"].as_str().unwrap(),
            s["condition"].as_str().unwrap()
        ));
    }
    for c in frame["cells"].as_array().unwrap() {
        expected.push(format!(
            "  cell {}: {}/{}",
            c["addr"].as_str().unwrap(),
            c["current"],
            c["max"]
        ));
    }
    for c in frame["components"].as_array().unwrap() {
        expected.push(format!(
            "  component {}: {} ({})",
            c["id"].as_str().unwrap(),
            c["condition"].as_str().unwrap(),
            c["definition"].as_str().unwrap()
        ));
    }
    for k in frame["capabilities"].as_array().unwrap() {
        expected.push(format!(
            "  capability {} (source {})",
            k["id"].as_str().unwrap(),
            k["source"].as_str().unwrap_or("none")
        ));
    }
    assert_eq!(expected.len(), 1 + 4 + 1 + 1);
    for line in &expected {
        assert!(
            lines.contains(&line.as_str()),
            "missing {line:?} in:\n{shown}"
        );
    }
    // ... and nothing the DTO does not name.
    let named = lines.iter().filter(|l| l.starts_with("  ")).count();
    assert_eq!(named, expected.len());
    assert!(lines[0].contains(&format!("schema {}", frame["schema"].as_str().unwrap())));
    assert!(lines[0].contains("mobility ground.biped"));
}

#[test]
fn i8_c2_a_params_less_subscribe_with_no_unit_answers_a_null_keyframe_not_a_sys() {
    let mut tw = boot_i8();
    let viewer = i8_player(&mut tw, "Nobody");
    for feed in ["tactics.unit", "tactics.vitals"] {
        let data = i8_keyframe(i8_query(&tw, feed, viewer, json!({})), feed);
        assert_eq!(data, Value::Null, "{feed}");
        let data = i8_keyframe(i8_query(&tw, feed, viewer, json!({ "object": null })), feed);
        assert_eq!(data, Value::Null, "{feed}");
    }
}

#[test]
fn i8_c3_the_default_subject_is_the_viewers_own_object_then_its_highest_priority_station() {
    let mut tw = boot_i8();
    let low = i8_unit(&mut tw);
    let high = i8_unit(&mut tw);
    let pilot = i8_player(&mut tw, "Pilot");
    i8_grant(&mut tw, pilot, low, 5);
    i8_grant(&mut tw, pilot, high, 10);
    let frame = i8_keyframe(
        i8_query(&tw, "tactics.unit", pilot, json!({})),
        "tactics.unit",
    );
    assert_eq!(frame["object"]["id"], json!(format!("#{}", high.0)));
    let vitals = i8_keyframe(
        i8_query(&tw, "tactics.vitals", pilot, json!({})),
        "tactics.vitals",
    );
    assert_eq!(vitals["subject"]["id"], json!(format!("#{}", high.0)));
    // A station on a nonexistent or non-unit subject is skipped.
    let ghost = i8_player(&mut tw, "Ghost");
    i8_grant(&mut tw, ghost, ObjId(999), 50);
    i8_grant(&mut tw, ghost, low, 1);
    let frame = i8_keyframe(
        i8_query(&tw, "tactics.unit", ghost, json!({})),
        "tactics.unit",
    );
    assert_eq!(frame["object"]["id"], json!(format!("#{}", low.0)));
    // A player who IS a unit reads themself first, whatever they hold.
    let output = tw.run(GOD, &format!("@unit/create #{}=tacsim.mech.i8", pilot.0));
    assert!(!has_error(&output), "{}", texts(&output));
    let frame = i8_keyframe(
        i8_query(&tw, "tactics.unit", pilot, json!({})),
        "tactics.unit",
    );
    assert_eq!(frame["object"]["id"], json!(format!("#{}", pilot.0)));
}

#[test]
fn i8_c4_stationed_non_controller_owner_and_royalty_read_and_a_stranger_does_not() {
    let mut tw = boot_i8();
    let owner = i8_player(&mut tw, "Owner");
    let unit = i8_unit_owned_by(&mut tw, owner);
    let gunner = i8_player(&mut tw, "Gunner");
    let stranger = i8_player(&mut tw, "Stranger");
    i8_grant(&mut tw, gunner, unit, 10);
    for who in [gunner, owner, GOD] {
        let frame = i8_frame_of(&tw, "tactics.unit", who, unit);
        assert_eq!(
            frame["object"]["id"],
            json!(format!("#{}", unit.0)),
            "#{}",
            who.0
        );
    }
    let params = json!({ "object": format!("#{}", unit.0) });
    assert!(i8_query(&tw, "tactics.unit", stranger, params.clone()).is_err());
    assert!(i8_query(&tw, "tactics.vitals", stranger, params).is_err());
    // The owner holds no station and is not a unit: the unnamed default is
    // null for them — authority is not the default subject.
    assert_eq!(
        i8_keyframe(
            i8_query(&tw, "tactics.unit", owner, json!({})),
            "tactics.unit"
        ),
        Value::Null
    );
}

#[test]
fn i8_c5_the_two_queries_write_nothing_and_stamp_no_idle() {
    let mut tw = boot_i8();
    let unit = i8_unit(&mut tw);
    let pilot = i8_player(&mut tw, "Pilot");
    i8_grant(&mut tw, pilot, unit, 10);
    tw.connect(pilot);
    tw.take_module_ops();
    let before = tw.world().session_stat(pilot).expect("connected");
    for feed in ["tactics.unit", "tactics.vitals"] {
        i8_keyframe(i8_query(&tw, feed, pilot, json!({})), feed);
        i8_frame_of(&tw, feed, pilot, unit);
    }
    assert_eq!(tw.world().session_stat(pilot), Some(before));
    assert!(tw.take_module_ops().is_empty(), "a query queued a write");
}

#[test]
fn i8_c6_view_rs_opens_no_grant_read() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/view.rs");
    let text = std::fs::read_to_string(path).expect("mush-tactics/src/view.rs");
    for needle in [
        "command_subjects_of(",
        ".channel_grant(",
        "channel_grant_is_current(",
    ] {
        assert!(
            !text.contains(needle),
            "view.rs reads a grant directly ({needle}); go through status.rs"
        );
    }
    assert!(text.contains("viewer_is_stationed("));
    assert!(
        !text.contains("to_value(unit"),
        "the view must be accessor-built"
    );
}

fn i8_big_spec(sections: usize, cells_per_section: usize, unit_cells: usize) -> UnitSpec {
    let mut definitions = DefinitionRegistryBuilder::new();
    let opaque = register_ref(
        &mut definitions,
        "tacsim",
        "schema",
        "tacsim.big",
        json!({}),
    );
    let body = UnitSpecBody {
        sections: (0..sections)
            .map(|i| SectionSpec {
                id: SectionId::new(format!("sec_{i:03}")).unwrap(),
                condition: ConditionId::new("ok").unwrap(),
                facets: vec![],
                slots: vec![],
                cells: (0..cells_per_section)
                    .map(|j| CellSpec {
                        id: CellId::new(format!("cell_{j:02}")).unwrap(),
                        min: 0,
                        max: 10,
                        init: 10,
                    })
                    .collect(),
            })
            .collect(),
        unit_cells: (0..unit_cells)
            .map(|j| CellSpec {
                id: CellId::new(format!("pool_{j}")).unwrap(),
                min: 0,
                max: 30,
                init: 15,
            })
            .collect(),
        components: vec![],
        capabilities: vec![],
        mobility: None,
        rules: RulesValue::new(opaque, json!({})).unwrap(),
    };
    let descriptor = body
        .descriptor(
            DefinitionKind::new("unit").unwrap(),
            DefinitionId::new("tacsim.mech.big").unwrap(),
            1,
        )
        .unwrap();
    let definition = definitions
        .register(OwnerId::new("tacsim").unwrap(), descriptor)
        .unwrap();
    body.with_definition(definition)
}

#[test]
fn i8_b1_the_keyframe_at_max_unit_cells_stays_under_the_view_budget_and_the_record_bound() {
    let unit_cells = 2;
    let spec = i8_big_spec(130, 63, unit_cells);
    let mut tw = boot_i8();
    let object = tw
        .world_mut()
        .create(ObjKind::Thing, "Leviathan", GOD, None)
        .unwrap();
    let unit = TacticalUnit::from_spec(&spec, object).unwrap();
    assert_eq!(unit.cells().count(), 130 * 63 + unit_cells);
    let view = mush_tactics::UnitView::of(
        tw.world(),
        object,
        &mush_tactics::UnitSource::Active(&unit),
        0,
    );
    let frame = view.to_frame();
    let bytes = frame.to_string().len();
    assert!(
        bytes <= mush_tactics::MAX_UNIT_VIEW_BYTES + 512,
        "{bytes} bytes over the view budget"
    );
    assert!(
        mush_core::canonical_json_bytes_with_limit(&frame, mush_tactics::MAX_UNIT_RECORD_BYTES)
            .is_ok()
    );
    assert_eq!(frame["truncated"], json!(true));
    assert_eq!(frame["counts"]["cells"], json!(130 * 63 + unit_cells));
    assert_eq!(frame["counts"]["sections"], json!(130));
    assert_eq!(
        frame["sections"].as_array().unwrap().len(),
        130,
        "sections are never cut"
    );
    let cells = frame["cells"].as_array().unwrap();
    assert!(cells.len() < 130 * 63 + unit_cells);
    // b3: the unit-level pools survive the cut, first.
    assert_eq!(cells[0]["addr"], json!("unit/pool_0"));
    assert_eq!(cells[1]["addr"], json!("unit/pool_1"));
    assert!(cells[2]["addr"].as_str().unwrap().starts_with("section/"));
    let vitals = view.vitals_value();
    assert_eq!(vitals["vitals"].as_array().unwrap().len(), unit_cells);
}

#[test]
fn i8_b2_truncated_is_false_and_counts_match_for_the_ordinary_unit() {
    let mut tw = boot_i8();
    let unit = i8_unit(&mut tw);
    let frame = i8_frame_of(&tw, "tactics.unit", GOD, unit);
    assert_eq!(frame["truncated"], json!(false));
    assert_eq!(
        frame["counts"],
        json!({ "sections": 1, "cells": 4, "components": 1, "capabilities": 1 })
    );
    let keys: Vec<&str> = frame
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec![
            "capabilities",
            "cells",
            "components",
            "counts",
            "mobility",
            "object",
            "record",
            "revision",
            "schema",
            "sections",
            "state",
            "truncated",
        ]
    );
    let cells: Vec<&str> = frame["cells"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["addr"].as_str().unwrap())
        .collect();
    assert_eq!(
        cells,
        vec![
            "unit/heat",
            "section/center_torso/structure",
            "facet/center_torso/rear/armor",
            "component/gauss/ammo",
        ]
    );
    assert_eq!(
        frame["sections"][0]["facets"],
        json!([{ "id": "rear", "cells": ["armor"] }])
    );
    assert_eq!(frame["sections"][0]["slots"], json!(["torso_hardpoint"]));
    assert_eq!(frame["sections"][0]["state"], json!("ok"));
    assert_eq!(frame["components"][0]["state"], json!("ok"));
}

#[test]
fn i8_v1_tactics_vitals_projects_only_unit_level_cells_with_the_unit_revision() {
    let mut tw = boot_i8();
    let unit = i8_unit(&mut tw);
    let ev = i8_damage(
        &mut tw,
        unit,
        r#"{"SetCell":{"addr":{"Unit":{"cell":"heat"}},"value":3}}"#,
    );
    let frame = i8_frame_of(&tw, "tactics.vitals", GOD, unit);
    assert_eq!(frame["subject"]["id"], json!(format!("#{}", unit.0)));
    assert_eq!(frame["revision"], json!(1));
    assert_eq!(
        frame["vitals"],
        json!([{
            "id": "tactics.heat", "label": "heat", "role": "pool",
            "value": { "kind": "bounded", "current": 3, "min": 0, "max": 30 },
            "state": "crit"
        }])
    );
    // ... and the same frame rode the damage output to the staffer.
    let pushed = i8_frames_to(&ev, "tactics.vitals", PanelFrameKind::Keyframe, GOD);
    assert_eq!(pushed, vec![frame]);
}

#[test]
fn i8_v2_a_cell_id_over_the_name_cap_is_skipped_and_the_frame_still_ships() {
    let mut definitions = DefinitionRegistryBuilder::new();
    let opaque = register_ref(
        &mut definitions,
        "tacsim",
        "schema",
        "tacsim.long",
        json!({}),
    );
    let long = "l".repeat(60);
    let body = UnitSpecBody {
        sections: vec![SectionSpec {
            id: SectionId::new("hull").unwrap(),
            condition: ConditionId::new("ok").unwrap(),
            facets: vec![],
            slots: vec![],
            cells: vec![],
        }],
        unit_cells: vec![
            CellSpec {
                id: CellId::new(&long).unwrap(),
                min: 0,
                max: 5,
                init: 5,
            },
            CellSpec {
                id: CellId::new("heat").unwrap(),
                min: 0,
                max: 30,
                init: 20,
            },
        ],
        components: vec![],
        capabilities: vec![],
        mobility: None,
        rules: RulesValue::new(opaque, json!({})).unwrap(),
    };
    let descriptor = body
        .descriptor(
            DefinitionKind::new("unit").unwrap(),
            DefinitionId::new("tacsim.mech.long").unwrap(),
            1,
        )
        .unwrap();
    let definition = definitions
        .register(OwnerId::new("tacsim").unwrap(), descriptor)
        .unwrap();
    let spec = body.with_definition(definition);
    let mut tw = boot_i8();
    let object = tw
        .world_mut()
        .create(ObjKind::Thing, "Longboat", GOD, None)
        .unwrap();
    let unit = TacticalUnit::from_spec(&spec, object).unwrap();
    let view = mush_tactics::UnitView::of(
        tw.world(),
        object,
        &mush_tactics::UnitSource::Active(&unit),
        0,
    );
    let vitals = view.vitals_value();
    let list = vitals["vitals"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"], json!("tactics.heat"));
    // The keyframe still carries the long cell — only the gauge is skipped.
    assert_eq!(view.to_frame()["counts"]["cells"], json!(2));
}

#[test]
fn i8_v3_dto_cell_state_and_vital_state_agree_with_pool_state() {
    use mush_proto::VitalState;
    let table = [
        (0, VitalState::Down),
        (3, VitalState::Crit),
        (7, VitalState::Crit),
        (8, VitalState::Warn),
        (15, VitalState::Warn),
        (16, VitalState::Ok),
        (30, VitalState::Ok),
    ];
    for (current, expected) in table {
        assert_eq!(
            mush_ruleset::pool_state(current, 0, 30),
            expected,
            "{current}/30"
        );
    }
    assert_eq!(
        mush_ruleset::pool_state(5, 5, 5),
        VitalState::Ok,
        "an empty span is ok"
    );
    let mut tw = boot_i8();
    let unit = i8_unit(&mut tw);
    let frame = i8_frame_of(&tw, "tactics.unit", GOD, unit);
    let vitals = i8_frame_of(&tw, "tactics.vitals", GOD, unit);
    for cell in frame["cells"].as_array().unwrap() {
        let expected = mush_ruleset::pool_state(
            cell["current"].as_i64().unwrap(),
            cell["min"].as_i64().unwrap(),
            cell["max"].as_i64().unwrap(),
        );
        assert_eq!(
            cell["state"],
            serde_json::to_value(expected).unwrap(),
            "{cell}"
        );
    }
    assert_eq!(vitals["vitals"][0]["state"], frame["cells"][0]["state"]);
    // A section's state is the worst of its own and its facets' cells; a
    // component's the worst of its own.
    i8_damage(
        &mut tw,
        unit,
        r#"{"SetCell":{"addr":{"Facet":{"section":"center_torso","facet":"rear","cell":"armor"}},"value":1}}"#,
    );
    let frame = i8_frame_of(&tw, "tactics.unit", GOD, unit);
    assert_eq!(frame["sections"][0]["state"], json!("crit"));
    assert_eq!(frame["components"][0]["state"], json!("ok"));
}

#[test]
fn i8_s1_a_switching_profile_projects_from_to_ready_at_and_the_text_says_so() {
    let spec = i8_spec();
    let quad = MobilityModeId::new("ground.quad").unwrap();
    let mut body = spec.body();
    let mut profile = body.mobility.clone().unwrap();
    profile.begin_switch(&quad, 100, 10).unwrap();
    body.mobility = Some(profile);
    let spec = body.with_definition(spec.definition.clone());
    let mut tw = boot_i8();
    let object = tw
        .world_mut()
        .create(ObjKind::Thing, "Walker", GOD, None)
        .unwrap();
    let unit = TacticalUnit::from_spec(&spec, object).unwrap();
    let mid = mush_tactics::UnitView::of(
        tw.world(),
        object,
        &mush_tactics::UnitSource::Active(&unit),
        105,
    );
    let frame = mid.to_frame();
    assert_eq!(frame["mobility"]["mode"], Value::Null);
    assert_eq!(
        frame["mobility"]["switching"],
        json!({ "from": "ground.biped", "to": "ground.quad", "ready_at": 110 })
    );
    let line = &mid.render_lines("Walker")[0];
    assert!(
        line.contains("mobility ground.biped -> ground.quad (ready 110)"),
        "{line}"
    );
    let due = mush_tactics::UnitView::of(
        tw.world(),
        object,
        &mush_tactics::UnitSource::Active(&unit),
        120,
    );
    assert_eq!(due.to_frame()["mobility"]["mode"], json!("ground.quad"));
    // A due-but-unsettled switch reports the mode only (verify fold): a
    // frame never carries both halves.
    assert_eq!(due.to_frame()["mobility"]["switching"], Value::Null);
    assert!(due.render_lines("Walker")[0].contains("mobility ground.quad"));
}

#[test]
fn i8_s2_the_create_keyframe_is_exactly_one_frame_to_the_actor_and_carries_no_rules() {
    let mut tw = boot_i8();
    let object = tw
        .world_mut()
        .create(ObjKind::Thing, "Halberd", GOD, None)
        .unwrap();
    let ev = tw.run(GOD, &format!("@unit/create #{}=tacsim.mech.i8", object.0));
    assert!(!has_error(&ev), "{}", texts(&ev));
    let all: Vec<&(Audience, OutputEvent)> = ev
        .iter()
        .filter(|(_, e)| matches!(e, OutputEvent::Panel { feed, .. } if feed == "tactics.unit"))
        .collect();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].0, Audience::Actor(GOD));
    let frames = i8_frames_to(&ev, "tactics.unit", PanelFrameKind::Keyframe, GOD);
    assert_eq!(frames.len(), 1);
    i8_assert_clean("create keyframe", &frames[0].to_string(), I8_FORBIDDEN);
    assert_eq!(frames[0]["revision"], json!(0));
    // Verify fold: the revision-0 vitals frame reaches the actor too (and any
    // stationed viewer), so a recreate resets every client's watermark.
    let vitals = i8_frames_to(&ev, "tactics.vitals", PanelFrameKind::Keyframe, GOD);
    assert_eq!(vitals.len(), 1);
    assert_eq!(vitals[0]["revision"], json!(0));
    assert_eq!(vitals[0]["vitals"][0]["id"], json!("tactics.heat"));
}

#[test]
fn i8_s3_the_damage_delta_leads_with_the_staffers_frame() {
    let mut tw = boot_i8();
    let unit = i8_unit(&mut tw);
    let pilot = i8_player(&mut tw, "Pilot");
    i8_grant(&mut tw, pilot, unit, 10);
    tw.connect(pilot);
    let ev = i8_damage(
        &mut tw,
        unit,
        r#"{"AdjustCell":{"addr":{"Facet":{"section":"center_torso","facet":"rear","cell":"armor"}},"delta":-1}}"#,
    );
    let first = ev
        .iter()
        .find(|(_, e)| {
            matches!(e, OutputEvent::Panel { feed, kind: PanelFrameKind::Delta, .. } if feed == "tactics.unit")
        })
        .expect("a delta");
    assert_eq!(first.0, Audience::Actor(GOD));
    assert_eq!(
        frames(&ev, PanelFrameKind::Delta)[0]["to_revision"],
        json!(1)
    );
}

#[test]
fn i8_f1_a_damage_delta_and_a_vitals_keyframe_reach_each_connected_stationed_viewer_once_and_the_unit_keyframe_is_not_fanned()
 {
    let mut tw = boot_i8();
    let unit = i8_unit(&mut tw);
    let ann = i8_player(&mut tw, "Ann");
    let bob = i8_player(&mut tw, "Bob");
    let offline = i8_player(&mut tw, "Offline");
    let bystander = i8_player(&mut tw, "Bystander");
    for who in [ann, bob, offline] {
        i8_grant(&mut tw, who, unit, 10);
    }
    for who in [ann, bob, bystander] {
        tw.connect(who);
    }
    let ev = i8_damage(
        &mut tw,
        unit,
        r#"{"AdjustCell":{"addr":{"Unit":{"cell":"heat"}},"delta":5}}"#,
    );
    for who in [GOD, ann, bob] {
        assert_eq!(
            i8_frames_to(&ev, "tactics.unit", PanelFrameKind::Delta, who).len(),
            1,
            "#{}",
            who.0
        );
        let vitals = i8_frames_to(&ev, "tactics.vitals", PanelFrameKind::Keyframe, who);
        assert_eq!(vitals.len(), 1, "#{}", who.0);
        assert_eq!(vitals[0]["vitals"][0]["value"]["current"], json!(25));
        assert_eq!(vitals[0]["revision"], json!(1));
        assert!(i8_frames_to(&ev, "tactics.unit", PanelFrameKind::Keyframe, who).is_empty());
    }
    for who in [offline, bystander] {
        assert!(
            i8_frames_to(&ev, "tactics.unit", PanelFrameKind::Delta, who).is_empty(),
            "#{}",
            who.0
        );
        assert!(
            i8_frames_to(&ev, "tactics.vitals", PanelFrameKind::Keyframe, who).is_empty(),
            "#{}",
            who.0
        );
    }
    assert_eq!(frames(&ev, PanelFrameKind::Delta).len(), 3);
    assert!(ev.iter().all(|(aud, _)| matches!(aud, Audience::Actor(_))));
}

#[test]
fn i8_f2_the_after_action_audience_caps_at_max_unit_audience() {
    let mut tw = boot_i8();
    let unit = i8_unit(&mut tw);
    let n = mush_tactics::MAX_UNIT_AUDIENCE + 3;
    for i in 0..n {
        let who = i8_player(&mut tw, &format!("Crew{i}"));
        i8_grant(&mut tw, who, unit, 10);
        tw.connect(who);
    }
    let ev = i8_damage(
        &mut tw,
        unit,
        r#"{"AdjustCell":{"addr":{"Unit":{"cell":"heat"}},"delta":1}}"#,
    );
    assert_eq!(
        frames(&ev, PanelFrameKind::Delta).len(),
        1 + mush_tactics::MAX_UNIT_AUDIENCE
    );
}

#[test]
fn i8_r1_unit_show_is_still_staff_only() {
    let mut tw = boot_i8();
    let unit = i8_unit(&mut tw);
    let pilot = i8_player(&mut tw, "Pilot");
    i8_grant(&mut tw, pilot, unit, 10);
    let ev = tw.run(pilot, &format!("@unit/show #{}", unit.0));
    assert!(has_error(&ev), "{}", texts(&ev));
    assert!(texts(&ev).contains("Permission denied"));
    // ... while the projection is theirs.
    i8_frame_of(&tw, "tactics.unit", pilot, unit);
}

#[test]
fn i8_p5_every_surface_of_the_view_is_clean() {
    let mut tw = boot_i8();
    let unit = i8_unit(&mut tw);
    let keyframe = i8_frame_of(&tw, "tactics.unit", GOD, unit);
    let vitals = i8_frame_of(&tw, "tactics.vitals", GOD, unit);
    let shown = texts(&tw.run(GOD, &format!("@unit/show #{}", unit.0)));
    i8_assert_clean("tactics.unit keyframe", &keyframe.to_string(), I8_FORBIDDEN);
    i8_assert_clean("tactics.vitals keyframe", &vitals.to_string(), I8_FORBIDDEN);
    i8_assert_clean("@unit/show", &shown, I8_FORBIDDEN);
}

// ---- Interface plan I8: the verify fold (vf_) ---------------------------

/// A spec whose weight is all in the ALWAYS-EMITTED lane: one section, no
/// section cells, `unit_cells` unit-level pools with `id_len`-byte ids.
fn i8_wide_spec(unit_cells: usize, id_len: usize) -> UnitSpec {
    let mut definitions = DefinitionRegistryBuilder::new();
    let opaque = register_ref(
        &mut definitions,
        "tacsim",
        "schema",
        "tacsim.wide",
        json!({}),
    );
    let body = UnitSpecBody {
        sections: vec![SectionSpec {
            id: SectionId::new("hull").unwrap(),
            condition: ConditionId::new("ok").unwrap(),
            facets: vec![],
            slots: vec![],
            cells: vec![],
        }],
        unit_cells: (0..unit_cells)
            .map(|j| CellSpec {
                id: CellId::new(format!("{j:0>4}{:p<width$}", "", width = id_len - 4)).unwrap(),
                min: 0,
                max: 10,
                init: 10,
            })
            .collect(),
        components: vec![],
        capabilities: vec![],
        mobility: None,
        rules: RulesValue::new(opaque, json!({})).unwrap(),
    };
    let descriptor = body
        .descriptor(
            DefinitionKind::new("unit").unwrap(),
            DefinitionId::new("tacsim.mech.wide").unwrap(),
            1,
        )
        .unwrap();
    let definition = definitions
        .register(OwnerId::new("tacsim").unwrap(), descriptor)
        .unwrap();
    body.with_definition(definition)
}

/// A spec at the facet and slot caps with long ids: 256 sections, four
/// facets each (= `MAX_UNIT_FACETS`), eight slots each (= `MAX_UNIT_SLOTS`),
/// one cell per facet.
fn i8_wide_sections_spec() -> UnitSpec {
    let mut definitions = DefinitionRegistryBuilder::new();
    let opaque = register_ref(
        &mut definitions,
        "tacsim",
        "schema",
        "tacsim.facets",
        json!({}),
    );
    let long = |prefix: &str, i: usize| format!("{prefix}{i:0>60}");
    let body = UnitSpecBody {
        sections: (0..MAX_UNIT_SECTIONS)
            .map(|i| SectionSpec {
                id: SectionId::new(long("s", i)).unwrap(),
                condition: ConditionId::new("ok").unwrap(),
                facets: (0..4)
                    .map(|f| FacetSpec {
                        id: FacetId::new(long("f", f)).unwrap(),
                        cells: vec![CellSpec {
                            id: CellId::new(long("c", f)).unwrap(),
                            min: 0,
                            max: 10,
                            init: 10,
                        }],
                    })
                    .collect(),
                slots: (0..8)
                    .map(|k| InstallSlotId::new(long("k", k)).unwrap())
                    .collect(),
                cells: vec![],
            })
            .collect(),
        unit_cells: vec![],
        components: vec![],
        capabilities: vec![],
        mobility: None,
        rules: RulesValue::new(opaque, json!({})).unwrap(),
    };
    let descriptor = body
        .descriptor(
            DefinitionKind::new("unit").unwrap(),
            DefinitionId::new("tacsim.mech.facets").unwrap(),
            1,
        )
        .unwrap();
    let definition = definitions
        .register(OwnerId::new("tacsim").unwrap(), descriptor)
        .unwrap();
    body.with_definition(definition)
}

#[test]
fn vf_b4_the_always_emitted_lane_is_admitted_against_the_hard_bound() {
    let spec = i8_wide_spec(MAX_UNIT_CELLS, 64);
    let mut tw = boot_i8();
    let object = tw
        .world_mut()
        .create(ObjKind::Thing, "Behemoth", GOD, None)
        .unwrap();
    let unit = TacticalUnit::from_spec(&spec, object).unwrap();
    // The persisted shape fits the record bound — the frame's per-cell cost
    // is what does not — so this is the divergence the fold closes.
    let persisted = serde_json::to_value(&unit).unwrap();
    assert!(
        mush_core::canonical_json_bytes_with_limit(&persisted, mush_tactics::MAX_UNIT_RECORD_BYTES)
            .is_ok(),
        "the unit itself persists under the record bound"
    );
    let view = mush_tactics::UnitView::of(
        tw.world(),
        object,
        &mush_tactics::UnitSource::Active(&unit),
        0,
    );
    let frame = view.to_frame();
    let bytes = frame.to_string().len();
    assert!(
        bytes <= mush_tactics::MAX_UNIT_RECORD_BYTES,
        "{bytes} bytes: the frame crossed the hard bound"
    );
    assert!(
        mush_core::canonical_json_bytes_with_limit(&frame, mush_tactics::MAX_UNIT_RECORD_BYTES)
            .is_ok()
    );
    assert_eq!(frame["truncated"], json!(true));
    assert_eq!(frame["counts"]["cells"], json!(MAX_UNIT_CELLS));
    assert!(frame["cells"].as_array().unwrap().len() < MAX_UNIT_CELLS);
    assert!(!frame["cells"].as_array().unwrap().is_empty());
    // Sixty-four-byte ids exceed the forty-byte vitals budget, so this unit
    // has no gauges — the frame still shipped, which is the point.
    assert!(view.vitals_value()["vitals"].as_array().unwrap().is_empty());
}

#[test]
fn vf_b5_a_facet_and_slot_heavy_unit_stays_under_the_hard_bound() {
    let spec = i8_wide_sections_spec();
    let mut tw = boot_i8();
    let object = tw
        .world_mut()
        .create(ObjKind::Thing, "Fortress", GOD, None)
        .unwrap();
    let unit = TacticalUnit::from_spec(&spec, object).unwrap();
    assert_eq!(unit.sections().count(), MAX_UNIT_SECTIONS);
    let frame = mush_tactics::UnitView::of(
        tw.world(),
        object,
        &mush_tactics::UnitSource::Active(&unit),
        0,
    )
    .to_frame();
    let bytes = frame.to_string().len();
    assert!(
        bytes <= mush_tactics::MAX_UNIT_RECORD_BYTES,
        "{bytes} bytes over the hard bound"
    );
    assert_eq!(frame["counts"]["sections"], json!(MAX_UNIT_SECTIONS));
    // Over the soft budget, the frame says so even though the always-emitted
    // lane is what carried it there.
    if bytes > mush_tactics::MAX_UNIT_VIEW_BYTES {
        assert_eq!(frame["truncated"], json!(true));
    }
}

#[test]
fn vf_p6_a_mis_keyed_record_never_names_the_other_object() {
    let mut tw = boot_i8();
    let key_object = tw
        .world_mut()
        .create(ObjKind::Thing, "Key shell", GOD, None)
        .unwrap();
    let embedded = i8_player(&mut tw, "Embedded");
    attach_current_binding(&mut tw, key_object);
    let unit = TacticalUnit::from_spec(&example_spec(), embedded).unwrap();
    let key = SmolStr::new(format!("unit:{}", key_object.0));
    let document = current_document(&tw, &unit);
    load_tactics(&mut tw, vec![(key, document)]);
    let record = &store(&tw).records[0];
    assert_eq!(record.key_object, Some(key_object));
    assert_eq!(record.object, Some(embedded));

    // Named by its KEY object: the frame carries the key object's own record
    // name, no schema and revision 0 — nothing of the embedded unit.
    let by_key = i8_keyframe(
        i8_query(
            &tw,
            "tactics.unit",
            GOD,
            json!({ "object": format!("#{}", key_object.0) }),
        ),
        "tactics.unit",
    );
    assert_eq!(by_key["object"]["id"], json!(format!("#{}", key_object.0)));
    assert_eq!(by_key["record"], json!(format!("unit:{}", key_object.0)));
    assert_eq!(by_key["schema"], json!(""));
    assert_eq!(by_key["revision"], json!(0));
    let text = by_key.to_string();
    assert!(!text.contains(&format!("#{}", embedded.0)), "{text}");
    assert!(!text.contains(&format!("unit:{}", embedded.0)), "{text}");

    // Named by its EMBEDDED object — and as that object's own unnamed
    // default — the frame carries that object's record name and the decoded
    // unit's schema, never the key object's dbref.
    for params in [json!({ "object": format!("#{}", embedded.0) }), json!({})] {
        let frame = i8_keyframe(
            i8_query(&tw, "tactics.unit", embedded, params),
            "tactics.unit",
        );
        assert_eq!(frame["object"]["id"], json!(format!("#{}", embedded.0)));
        assert_eq!(frame["record"], json!(format!("unit:{}", embedded.0)));
        assert_eq!(frame["schema"], json!(unit.schema().to_string()));
        assert_eq!(frame["state"], json!("quarantined"));
        let text = frame.to_string();
        assert!(!text.contains(&format!("#{}", key_object.0)), "{text}");
        assert!(!text.contains(&format!("unit:{}", key_object.0)), "{text}");
    }
}

#[test]
fn vf_r2_an_active_station_outranks_a_held_own_record_for_the_default_subject() {
    let mut tw = boot_i8();
    let pilot = i8_player(&mut tw, "Pilot");
    // The pilot's own object carries a document with no binding: Frozen.
    let own = TacticalUnit::from_spec(&example_spec(), pilot).unwrap();
    let document = current_document(&tw, &own);
    load_tactics(
        &mut tw,
        vec![(SmolStr::new(format!("unit:{}", pilot.0)), document)],
    );
    assert!(matches!(
        store(&tw).records[0].state,
        MechanicalState::Frozen { .. }
    ));
    let held = i8_keyframe(
        i8_query(&tw, "tactics.unit", pilot, json!({})),
        "tactics.unit",
    );
    assert_eq!(held["object"]["id"], json!(format!("#{}", pilot.0)));
    assert_eq!(held["state"], json!("frozen"));
    // A live seat outranks the held record of one's own.
    let unit = i8_unit(&mut tw);
    i8_grant(&mut tw, pilot, unit, 10);
    let live = i8_keyframe(
        i8_query(&tw, "tactics.unit", pilot, json!({})),
        "tactics.unit",
    );
    assert_eq!(live["object"]["id"], json!(format!("#{}", unit.0)));
    assert_eq!(live["state"], json!("active"));
}

#[test]
fn vf_f3_remove_then_create_refreshes_the_stationed_pilot_and_resets_the_revision() {
    let mut tw = boot_i8();
    let unit = i8_unit(&mut tw);
    let pilot = i8_player(&mut tw, "Pilot");
    i8_grant(&mut tw, pilot, unit, 10);
    tw.connect(pilot);
    i8_damage(
        &mut tw,
        unit,
        r#"{"AdjustCell":{"addr":{"Unit":{"cell":"heat"}},"delta":1}}"#,
    );
    let ev = tw.run(GOD, &format!("@unit/remove #{}", unit.0));
    assert!(!has_error(&ev), "{}", texts(&ev));
    // The pilot's default subject is gone: null on both feeds, once each.
    assert_eq!(
        i8_frames_to(&ev, "tactics.unit", PanelFrameKind::Keyframe, pilot),
        vec![Value::Null]
    );
    assert_eq!(
        i8_frames_to(&ev, "tactics.vitals", PanelFrameKind::Keyframe, pilot),
        vec![Value::Null]
    );
    // The staffer, unstationed, gets nothing on the feeds (their HUD was not
    // showing this unit).
    assert!(i8_frames_to(&ev, "tactics.unit", PanelFrameKind::Keyframe, GOD).is_empty());
    let ev = tw.run(GOD, &format!("@unit/create #{}=tacsim.mech.i8", unit.0));
    assert!(!has_error(&ev), "{}", texts(&ev));
    for who in [GOD, pilot] {
        let frames = i8_frames_to(&ev, "tactics.unit", PanelFrameKind::Keyframe, who);
        assert_eq!(frames.len(), 1, "#{}", who.0);
        assert_eq!(frames[0]["revision"], json!(0));
        let vitals = i8_frames_to(&ev, "tactics.vitals", PanelFrameKind::Keyframe, who);
        assert_eq!(vitals.len(), 1, "#{}", who.0);
        assert_eq!(vitals[0]["revision"], json!(0));
    }
}

#[test]
fn vf_f4_a_destroyed_asset_refreshes_the_repaired_unit_for_its_stationed_viewers() {
    let mut tw = boot_i8();
    let unit = i8_unit(&mut tw);
    let rifle = tw
        .world_mut()
        .create(ObjKind::Thing, "Rifle", GOD, None)
        .unwrap();
    i8_damage(
        &mut tw,
        unit,
        &format!(
            r#"{{"SetComponentAsset":{{"component":"gauss","asset":{}}}}}"#,
            rifle.0
        ),
    );
    let before = i8_frame_of(&tw, "tactics.unit", GOD, unit);
    assert_eq!(before["components"][0]["asset"], json!(true));
    let pilot = i8_player(&mut tw, "Pilot");
    i8_grant(&mut tw, pilot, unit, 10);
    tw.connect(pilot);
    tw.world_mut().destroy(rifle).unwrap();
    let mut out = Outbox::new();
    let mut effects = Effects::new();
    tw.engine_mut().emit_world_event(
        &WorldEvent::ObjDestroyed { obj: rifle },
        &mut out,
        &mut effects,
    );
    let ev = out.take();
    let frames = i8_frames_to(&ev, "tactics.unit", PanelFrameKind::Keyframe, pilot);
    assert_eq!(frames.len(), 1, "{}", texts(&ev));
    assert_eq!(frames[0]["components"][0]["asset"], json!(false));
    assert_eq!(
        frames[0]["revision"],
        json!(before["revision"].as_u64().unwrap() + 1)
    );
    assert_eq!(
        i8_frames_to(&ev, "tactics.vitals", PanelFrameKind::Keyframe, pilot).len(),
        1
    );
    let text = frames[0].to_string();
    assert!(!text.contains(&format!("#{}", rifle.0)), "{text}");
}

#[test]
fn vf_f5_a_unit_that_is_its_own_viewer_receives_its_after_action() {
    let mut tw = boot_i8();
    let pilot = i8_player(&mut tw, "Pilot");
    let output = tw.run(GOD, &format!("@unit/create #{}=tacsim.mech.i8", pilot.0));
    assert!(!has_error(&output), "{}", texts(&output));
    tw.connect(pilot);
    let ev = i8_damage(
        &mut tw,
        pilot,
        r#"{"AdjustCell":{"addr":{"Unit":{"cell":"heat"}},"delta":1}}"#,
    );
    assert_eq!(
        i8_frames_to(&ev, "tactics.unit", PanelFrameKind::Delta, pilot).len(),
        1
    );
    assert_eq!(
        i8_frames_to(&ev, "tactics.vitals", PanelFrameKind::Keyframe, pilot).len(),
        1
    );
    // Not connected: nothing.
    let drone = tw
        .world_mut()
        .create(ObjKind::Npc, "Drone", GOD, None)
        .unwrap();
    let output = tw.run(GOD, &format!("@unit/create #{}=tacsim.mech.i8", drone.0));
    assert!(!has_error(&output), "{}", texts(&output));
    let ev = i8_damage(
        &mut tw,
        drone,
        r#"{"AdjustCell":{"addr":{"Unit":{"cell":"heat"}},"delta":1}}"#,
    );
    assert!(i8_frames_to(&ev, "tactics.unit", PanelFrameKind::Delta, drone).is_empty());
}

#[test]
fn vf_f6_the_vitals_keyframe_follows_the_viewers_default_subject_only() {
    let mut tw = boot_i8();
    let low = i8_unit(&mut tw);
    let high = i8_unit(&mut tw);
    let pilot = i8_player(&mut tw, "Pilot");
    i8_grant(&mut tw, pilot, low, 5);
    i8_grant(&mut tw, pilot, high, 10);
    tw.connect(pilot);
    let ev = i8_damage(
        &mut tw,
        low,
        r#"{"AdjustCell":{"addr":{"Unit":{"cell":"heat"}},"delta":1}}"#,
    );
    assert_eq!(
        i8_frames_to(&ev, "tactics.unit", PanelFrameKind::Delta, pilot).len(),
        1,
        "the delta reaches every stationed viewer"
    );
    assert!(
        i8_frames_to(&ev, "tactics.vitals", PanelFrameKind::Keyframe, pilot).is_empty(),
        "the lower seat never repoints the gauge strip"
    );
    let ev = i8_damage(
        &mut tw,
        high,
        r#"{"AdjustCell":{"addr":{"Unit":{"cell":"heat"}},"delta":1}}"#,
    );
    assert_eq!(
        i8_frames_to(&ev, "tactics.unit", PanelFrameKind::Delta, pilot).len(),
        1
    );
    assert_eq!(
        i8_frames_to(&ev, "tactics.vitals", PanelFrameKind::Keyframe, pilot).len(),
        1
    );
}

#[test]
fn vf_v4_the_effective_cell_id_budget_is_forty_bytes() {
    let forty = "a".repeat(40);
    let forty_one = "b".repeat(41);
    let mut definitions = DefinitionRegistryBuilder::new();
    let opaque = register_ref(
        &mut definitions,
        "tacsim",
        "schema",
        "tacsim.forty",
        json!({}),
    );
    let body = UnitSpecBody {
        sections: vec![SectionSpec {
            id: SectionId::new("hull").unwrap(),
            condition: ConditionId::new("ok").unwrap(),
            facets: vec![],
            slots: vec![],
            cells: vec![],
        }],
        unit_cells: vec![
            CellSpec {
                id: CellId::new(&forty).unwrap(),
                min: 0,
                max: 5,
                init: 5,
            },
            CellSpec {
                id: CellId::new(&forty_one).unwrap(),
                min: 0,
                max: 5,
                init: 5,
            },
        ],
        components: vec![],
        capabilities: vec![],
        mobility: None,
        rules: RulesValue::new(opaque, json!({})).unwrap(),
    };
    let descriptor = body
        .descriptor(
            DefinitionKind::new("unit").unwrap(),
            DefinitionId::new("tacsim.mech.forty").unwrap(),
            1,
        )
        .unwrap();
    let definition = definitions
        .register(OwnerId::new("tacsim").unwrap(), descriptor)
        .unwrap();
    let spec = body.with_definition(definition);
    let mut tw = boot_i8();
    let object = tw
        .world_mut()
        .create(ObjKind::Thing, "Forty", GOD, None)
        .unwrap();
    let unit = TacticalUnit::from_spec(&spec, object).unwrap();
    let vitals = mush_tactics::UnitView::of(
        tw.world(),
        object,
        &mush_tactics::UnitSource::Active(&unit),
        0,
    )
    .vitals_value();
    let ids: Vec<&str> = vitals["vitals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec![format!("tactics.{forty}").as_str()]);
}

#[test]
fn vf_c8_the_grant_helpers_are_called_only_from_view_module_and_status() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let allowed = ["status.rs", "view.rs", "module.rs"].map(std::path::Path::new);
    let mut directories = vec![src.clone()];
    let mut files = Vec::new();
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(directory).expect("source directory") {
            let entry = entry.expect("source entry");
            let path = entry.path();
            let kind = entry.file_type().expect("source entry type");
            if kind.is_dir() {
                directories.push(path);
            } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    for path in files {
        // Match root-relative paths: a nested module.rs is not an allowed consumer.
        let relative = path.strip_prefix(&src).expect("source-relative path");
        let name = relative.display();
        let text = std::fs::read_to_string(&path).expect("Rust source");
        for needle in [
            "viewer_is_stationed(",
            "primary_stationed_unit(",
            "stationed_viewers(",
        ] {
            let calls = text
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .filter(|l| l.contains(needle) && !l.contains("pub fn "))
                .count();
            assert!(
                calls == 0 || allowed.contains(&relative),
                "{name} calls {needle} — a new authority consumer outside the census"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 16b cut Q.8 — `refit_unit`, the second instantiate route
// ---------------------------------------------------------------------------

/// The same composition as [`boot`], plus `mush-sim`: a refit has to be shown
/// NOT to reach a placement, a station or a billet, and those live on the
/// simulation's own state slot.
fn boot_with_sim() -> TestWorld {
    let spec = example_spec();
    let kit = kitted_spec();
    let catalog = example_catalog();
    TestWorld::with_modules(vec![
        Box::new(TestRulesModule {
            owner: "tacsim",
            specs: vec![spec, kit],
        }),
        Box::new(mush_sim::SimulationModule::new()),
        Box::new(TacticsModule::with_catalog(catalog)),
    ])
}

/// Run one closure against a real `Ctx` under the **tactics** module's own
/// identity, and hand back what it answered beside every op it queued.
///
/// The identity matters, and it is **not** production's: `persist_unit_document`
/// goes through `Ctx::persist`, whose fast path is one namespace equality and
/// which consults the delegation table only on a mismatch — so under the
/// crate's own `tactics` identity the `tacsim -> tactics` delegation is never
/// read at all. What these tests prove is therefore the shape of the write (one
/// op, one namespace, one key) and not that the crossing is permitted; the
/// crossing has its own lens in the game crate, over the real composition where
/// the delegation is really declared:
/// `crates/tacsim/tests/match_lifecycle.rs`'s
/// `q8_a_refit_under_the_tacsim_identity_queues_the_tactics_document`.
fn with_tactics_ctx<T>(
    world: &mut TestWorld,
    f: impl FnOnce(&mut mush_core::Ctx<'_>) -> T,
) -> (T, Vec<mush_core::ModuleOp>) {
    let mut out = Outbox::new();
    let engine = world.engine_mut();
    let mut ctx = mush_core::Ctx {
        w: &mut engine.world,
        actor: GOD,
        switches: mush_core::SwitchSet::default(),
        args: mush_core::ParsedArgs::default(),
        out: &mut out,
        reg: &engine.registry,
        module: Some(SmolStr::new("tactics")),
        typed_state: mush_core::ModuleActionState::unavailable(),
    };
    let answer = f(&mut ctx);
    let ops = out
        .take_deferred()
        .into_iter()
        .filter_map(|deferred| match deferred {
            mush_core::Deferred::Persist(op) => Some(op),
            _ => None,
        })
        .collect();
    (answer, ops)
}

/// The spec of the OTHER shipped schema — `tacsim.mech.atlas`, which has a
/// component the locust does not and no facet the locust does, so a refit
/// between them is a real change of structure that no `UnitMutationPlan` could
/// express.
fn atlas_spec() -> UnitSpec {
    let wanted = DefinitionId::new("tacsim.mech.atlas").unwrap();
    example_catalog()
        .specs()
        .find(|spec| spec.definition.key().id == wanted)
        .cloned()
        .expect("the kitted schema is in the catalog")
}

/// `mush-sim`'s whole record set, for a before/after comparison.
fn spatial(world: &TestWorld) -> mush_sim::SpatialState {
    world
        .world()
        .module_state::<mush_sim::SpatialState>()
        .expect("the simulation slot")
        .clone()
}

/// Place `unit` in a passive domain of its own and give it one control station,
/// so the refit has a placement, a station topology and an `assignment:<unit>`
/// record to leave alone.
fn station_and_place(world: &mut TestWorld, unit: ObjId) {
    let room = world
        .world_mut()
        .create(ObjKind::Room, "Hangar", GOD, None)
        .unwrap();
    with_tactics_ctx(world, |ctx| {
        let domain = mush_sim::open_passive_domain(ctx, room, room).expect("a passive domain");
        mush_sim::place(
            ctx,
            unit,
            domain,
            mush_sim::Pose {
                x_cm: 1_200,
                y_cm: 3_400,
                z_cm: 0,
                heading_mdeg: 90_000,
                posture: mush_sim::passive_rules_value(json!({})).expect("a posture"),
            },
        )
        .expect("the unit stands somewhere");
        let mut channels = BTreeSet::new();
        channels.insert(mush_core::ChannelId::new("drive").unwrap());
        mush_sim::declare_station(
            ctx,
            unit,
            &mush_sim::StationId::new("pilot").unwrap(),
            "pilot",
            &channels,
            None,
            2,
        )
        .expect("the unit has a cockpit");
    });
}

#[test]
fn q8_refit_refuses_an_object_with_no_unit_and_writes_nothing() {
    let mut world = boot();
    let bare = world
        .world_mut()
        .create(ObjKind::Thing, "Crate", GOD, None)
        .unwrap();
    let spec = example_spec();
    let (answer, ops) = with_tactics_ctx(&mut world, |ctx| {
        mush_tactics::module::refit_unit(ctx, bare, &spec)
    });
    assert_eq!(
        answer.unwrap_err(),
        format!("#{} holds no unit to refit", bare.0)
    );
    assert!(ops.is_empty(), "a refusal wrote {ops:?}");
    assert!(!store(&world).units.contains_key(&bare));
}

#[test]
fn q8_refit_refuses_a_deployed_unit_and_writes_nothing() {
    // With `mush-sim`, because the custody mark lives on ITS state slot and the
    // refusal is a read of that slot.
    let mut world = boot_with_sim();
    let unit = make_unit(&mut world);
    let before = store(&world).units.get(&unit).cloned().expect("the unit");
    with_tactics_ctx(&mut world, |ctx| {
        mush_tactics::custody::check_out(
            ctx,
            unit,
            mush_sim::DeployedMark::new(7, "tacsim.arena.proving_ground"),
        )
        .expect("the arena takes it");
    });
    let atlas = atlas_spec();
    let (answer, ops) = with_tactics_ctx(&mut world, |ctx| {
        mush_tactics::module::refit_unit(ctx, unit, &atlas)
    });
    assert_eq!(
        answer.unwrap_err(),
        format!(
            "#{} is deployed; a deployed unit cannot be refitted",
            unit.0
        )
    );
    assert!(ops.is_empty(), "a refusal wrote {ops:?}");
    assert_eq!(
        store(&world)
            .units
            .get(&unit)
            .cloned()
            .expect("still a unit"),
        before,
        "the arena is the single writer while it holds the unit"
    );
}

#[test]
fn q8_refit_refuses_a_spec_the_registry_does_not_hold_and_writes_nothing() {
    let mut world = boot();
    let unit = make_unit(&mut world);
    let before = store(&world).units.get(&unit).cloned().expect("the unit");
    // A schema of an owner this world never composed: every one of its
    // references is absent from the registry, which is what `create_unit`'s
    // exact-digest walk exists to catch and what `refit_unit` repeats.
    let stranger = example_spec_for("shadowrun", "shadowrun.mech.ghost");
    let (answer, ops) = with_tactics_ctx(&mut world, |ctx| {
        mush_tactics::module::refit_unit(ctx, unit, &stranger)
    });
    let why = answer.unwrap_err();
    assert!(
        why.contains("not available") || why.contains("unavailable"),
        "the refusal names the missing definition: {why}"
    );
    assert!(ops.is_empty(), "a refusal wrote {ops:?}");
    assert_eq!(
        store(&world)
            .units
            .get(&unit)
            .cloned()
            .expect("still a unit"),
        before
    );
}

#[test]
fn q8_a_refit_changes_the_type_and_leaves_the_world_around_it_alone() {
    let mut world = boot_with_sim();
    let unit = make_unit(&mut world);
    station_and_place(&mut world, unit);
    let _ = fold_kv(&mut world);

    let binding_before = world
        .world()
        .get(unit)
        .and_then(|object| object.behaviors.get(&unit_behavior()).cloned())
        .expect("`@unit/create` attached the unit behaviour");
    let spatial_before = spatial(&world);
    let locust = store(&world)
        .units
        .get(&unit)
        .cloned()
        .expect("the unit")
        .schema()
        .clone();

    let atlas = atlas_spec();
    let (answer, ops) = with_tactics_ctx(&mut world, |ctx| {
        mush_tactics::module::refit_unit(ctx, unit, &atlas)
    });
    let refitted = answer.expect("the refit lands");
    assert_eq!(*refitted.schema(), atlas.definition);
    assert_ne!(*refitted.schema(), locust, "it really is another schema");

    // **One write, in one namespace, on one key.** That is the whole proof that
    // the placement, the stations and the billets are untouched: `refit_unit`
    // never reaches `mush-sim` at all.
    assert_eq!(ops.len(), 1, "a refit is one document: {ops:?}");
    assert_eq!(ops[0].ns, "tactics");
    let expected_key = format!("unit:{}", unit.0);
    assert!(
        matches!(&ops[0].op, ModuleWrite::JsonSet { key, .. } if key.as_str() == expected_key),
        "the one op is this unit's record: {:?}",
        ops[0].op
    );
    assert_eq!(
        spatial(&world),
        spatial_before,
        "a refit moved a placement, a station or a billet"
    );
    assert_eq!(
        world
            .world()
            .get(unit)
            .and_then(|object| object.behaviors.get(&unit_behavior()).cloned()),
        Some(binding_before),
        "a refit restamped the behaviour binding"
    );

    // The RAM row is the new type, and it is still the ONE active record.
    assert_eq!(
        *store(&world)
            .units
            .get(&unit)
            .expect("still a unit")
            .schema(),
        atlas.definition
    );
    assert_eq!(
        store(&world)
            .records
            .iter()
            .filter(|record| record.object == Some(unit) && record.state == MechanicalState::Active)
            .count(),
        1,
        "the lifecycle row was replaced, not duplicated"
    );
}

#[test]
fn q8_a_refitted_document_decodes_as_the_new_type_on_the_next_boot() {
    let mut world = boot();
    let unit = make_unit(&mut world);
    let _ = fold_kv(&mut world);
    let atlas = atlas_spec();
    let (answer, ops) = with_tactics_ctx(&mut world, |ctx| {
        mush_tactics::module::refit_unit(ctx, unit, &atlas)
    });
    answer.expect("the refit lands");
    let kv: Vec<(SmolStr, Value)> = ops
        .into_iter()
        .filter_map(|op| match op.op {
            ModuleWrite::JsonSet { key, value } => Some((key, value)),
            _ => None,
        })
        .collect();
    assert_eq!(kv.len(), 1);

    let mut next = boot();
    let reborn = next
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, None)
        .unwrap();
    assert_eq!(reborn, unit, "the object id assumption this boot rests on");
    attach_current_binding(&mut next, reborn);
    load_tactics(&mut next, kv);
    let held = store(&next)
        .units
        .get(&reborn)
        .cloned()
        .expect("the refitted record boots active");
    assert_eq!(*held.schema(), atlas.definition);
    assert!(
        held.component(&ComponentId::new("gauss").unwrap())
            .is_some(),
        "the new type's own component came back with it"
    );
}

// ---------------------------------------------------------------------------
// 16b cut AF.3 — the boot keeps its markers, and the game layer claims them
// ---------------------------------------------------------------------------

/// **A game layer's claim, and nothing else** (16b cut AF.3): composed after
/// the tactics module, it takes the markers that module's boot kept
/// (`custody::take_boot_deployed`) in its own `on_world_loaded` — the claim
/// window — and does nothing with them, which is what a game layer that
/// resumes the match they name leaves behind: the markers on the records and
/// in the custody index.
struct Claimer;

impl GameModule for Claimer {
    fn name(&self) -> &'static str {
        "claimer"
    }

    fn register(&mut self, _reg: &mut Registrar<'_>) -> Result<(), ModError> {
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut mush_core::module::ModuleCtx<'_>,
        _data: ModuleBootData,
    ) -> Result<(), ModError> {
        let _ = mush_tactics::custody::take_boot_deployed(ctx.w);
        Ok(())
    }
}

/// [`boot_with_sim`], with or without a [`Claimer`] after the tactics module.
fn af3_boot(claim: bool) -> TestWorld {
    let mut modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(TestRulesModule {
            owner: "tacsim",
            specs: vec![example_spec(), kitted_spec()],
        }),
        Box::new(mush_sim::SimulationModule::new()),
        Box::new(TacticsModule::with_catalog(example_catalog())),
    ];
    if claim {
        modules.push(Box::new(Claimer));
    }
    TestWorld::with_modules(modules)
}

/// Check `unit` out to match 7 under the tactics module's identity, and
/// answer the store as it then stands (the tactics namespace): the check-out's
/// own write rides its `Ctx`'s outbox, not the engine's op queue.
fn af3_check_out(world: &mut TestWorld, unit: ObjId) -> Vec<(SmolStr, Value)> {
    let mut kv = fold_kv(world);
    let ((), ops) = with_tactics_ctx(world, |ctx| {
        mush_tactics::custody::check_out(
            ctx,
            unit,
            mush_sim::DeployedMark::new(7, "tacsim.arena.proving_ground"),
        )
        .expect("the arena takes it");
    });
    for op in ops {
        if let ModuleWrite::JsonSet { key, value } = op.op {
            kv.retain(|(existing, _)| *existing != key);
            kv.push((key, value));
        }
    }
    kv
}

/// A restart of the server half over `kv`: the custody index is a new
/// process's empty RAM, and the tactics module boots from its documents.
fn af3_reboot(world: &mut TestWorld, kv: Vec<(SmolStr, Value)>) {
    *world
        .world_mut()
        .module_state_mut::<mush_sim::deployed::DeployedUnits>()
        .expect("the custody index") = mush_sim::deployed::DeployedUnits::new();
    load_tactics(world, kv);
}

/// The one `unit:<obj>` document a list of ops wrote, if it wrote one.
fn af3_written(ops: &[mush_core::ModuleOp], unit: ObjId) -> Option<Value> {
    let key = format!("unit:{}", unit.0);
    ops.iter().rev().find_map(|op| match &op.op {
        ModuleWrite::JsonSet { key: at, value } if *at == key => Some(value.clone()),
        _ => None,
    })
}

/// **The boot keeps a `deployed` marker for the game layer to claim, and
/// restores what nobody claimed** (16b cut AF.3; the brief's test 13). The
/// tactics boot no longer restores a marked unit in `on_world_loaded`:
///
/// - **claimed** (a game layer composed after it takes the list): the marker
///   survives the boot on the record and in `mush_sim::deployed`, and the unit
///   is still refused as deployed;
/// - **unclaimed** (no game layer): it is restored at this module's
///   `on_all_loaded` — the check-out snapshot in the record, the marker gone
///   from RAM and the custody index, and the marker-less document persisted —
///   and the unit takes a plan again;
/// - **scrubbed** (claimed, with a component pointing at an object that went
///   away during the downtime): the boot's E3c scrub rewrites the document, and
///   the rewrite still carries the marker, so a second restart still finds it;
/// - **scrubbed mid-run** (the object `@destroy`ed while the unit is out on
///   loan): the runtime scrub's rewrite carries the marker as well.
#[test]
fn af3_the_boot_keeps_the_markers_for_the_claim_and_restores_the_unclaimed() {
    // Claimed.
    let mut world = af3_boot(true);
    let unit = make_unit(&mut world);
    let kv = af3_check_out(&mut world, unit);
    af3_reboot(&mut world, kv);
    assert!(
        mush_tactics::custody::marker_of(world.world(), unit).is_some(),
        "the claimed marker survives the boot"
    );
    assert!(mush_sim::deployed::is_deployed(world.world(), unit));
    let output = world.run(GOD, &format!("@unit/damage #{}={}", unit.0, armor_plan(-1)));
    assert!(
        has_error(&output),
        "a claimed unit took a plan: {}",
        texts(&output)
    );
    assert!(texts(&output).contains("deployed"), "{}", texts(&output));
    assert!(
        af3_written(&world.take_module_ops(), unit).is_none(),
        "a claimed marker is not rewritten"
    );

    // Unclaimed.
    let mut world = af3_boot(false);
    let unit = make_unit(&mut world);
    let snapshot = store(&world).units.get(&unit).cloned().expect("the unit");
    let kv = af3_check_out(&mut world, unit);
    af3_reboot(&mut world, kv);
    assert!(
        mush_tactics::custody::marker_of(world.world(), unit).is_none(),
        "nobody claimed it, so on_all_loaded restored it"
    );
    assert!(!mush_sim::deployed::is_deployed(world.world(), unit));
    assert_eq!(store(&world).units.get(&unit), Some(&snapshot));
    let document =
        af3_written(&world.take_module_ops(), unit).expect("the marker-less document is persisted");
    assert!(document.get("deployed").is_none(), "{document}");
    let output = world.run(GOD, &format!("@unit/damage #{}={}", unit.0, armor_plan(-1)));
    assert!(!has_error(&output), "{}", texts(&output));

    // Scrubbed, and still marked on disk.
    let mut world = af3_boot(true);
    let (mech, rifle) = kitted_unit(&mut world);
    let kv = af3_check_out(&mut world, mech);
    world.world_mut().destroy(rifle).unwrap(); // during the downtime
    af3_reboot(&mut world, kv);
    assert_eq!(
        asset_of(&world, mech),
        None,
        "the boot scrubbed the pointer"
    );
    let document =
        af3_written(&world.take_module_ops(), mech).expect("the scrub rewrote the document");
    assert!(
        document.get("deployed").is_some(),
        "the rewrite dropped the marker: {document}"
    );
    assert!(mush_tactics::custody::marker_of(world.world(), mech).is_some());

    // Scrubbed mid-run, by `@destroy`'s own event (the runtime scrub in
    // `on_obj_destroyed`): its rewrite carries the marker too, so a restart
    // after it still finds the unit on loan.
    let mut world = af3_boot(true);
    let (mech, rifle) = kitted_unit(&mut world);
    let _ = af3_check_out(&mut world, mech);
    world.take_module_ops();
    world.world_mut().destroy(rifle).unwrap();
    let mut out = Outbox::new();
    let mut fxx = Effects::new();
    world.engine_mut().emit_world_event(
        &WorldEvent::ObjDestroyed { obj: rifle },
        &mut out,
        &mut fxx,
    );
    assert_eq!(
        asset_of(&world, mech),
        None,
        "the runtime scrub repaired the pointer"
    );
    let document = af3_written(&world.take_module_ops(), mech)
        .expect("the runtime scrub rewrote the document");
    assert!(
        document.get("deployed").is_some(),
        "the runtime rewrite dropped the marker: {document}"
    );
    assert!(mush_tactics::custody::marker_of(world.world(), mech).is_some());
    assert!(mush_sim::deployed::is_deployed(world.world(), mech));
}

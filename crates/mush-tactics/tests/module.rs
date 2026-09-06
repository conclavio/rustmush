//! K1 acceptance for the live tactics module: exact definition composition,
//! v2-only authoring, explicit v1 dry-run migration, lifecycle gating, and a
//! boot path that never edits authoritative records or behavior bindings.

use std::collections::{BTreeMap, HashMap};

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
    ConditionId, FacetId, FacetSpec, InstallPoint, LocomotionId, MAX_UNIT_BINDING_REPAIRS,
    MediumId, MobilityMode, MobilityModeId, MobilityProfile, MobilityVocabulary, ModeRules,
    RulesValue, SectionId, SectionSpec, TacticalUnit, TacticsModule, UNIT_BINDING_VERSION,
    UNIT_RECORD_VERSION, UnitCatalog, UnitCatalogBuilder, UnitSpec, UnitSpecBody, UnitStore,
};
use mush_testkit::{TestWorld, text_of};
use serde_json::{Value, json};
use smol_str::SmolStr;

const FEED: &str = "tactics.unit";

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

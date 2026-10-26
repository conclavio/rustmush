//! Pure fitting prerequisites over the actual registered stock definitions.
//! These are structural tests, not economic/custody/paid-installation evidence.

use mush_core::{DefinitionKind, DefinitionRegistry, GameModule, ModError, ObjId, Registrar};
use mush_tactics::unit::replacement::{ComponentState, registered_spec, replace_component};
use mush_tactics::{
    CapabilityId, CapabilityKey, CellAddr, CellId, ComponentId, ConditionId, TacticalUnit,
    UnitMutationOp, UnitMutationPlan,
};
use mush_testkit::TestWorld;
use tacsim::arena::core::CoreClass;
use tacsim::definitions::{
    Catalog,
    core_replacement::{GovernedLineCores, LINE_I, LINE_II, LINE_III},
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Definitions(Catalog);
impl GameModule for Definitions {
    fn name(&self) -> &'static str {
        tacsim::NS
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        for kind in tacsim::definitions::KINDS {
            let descriptors = self
                .0
                .descriptors
                .iter()
                .filter(|row| row.kind().as_str() == kind)
                .cloned()
                .collect();
            reg.definition_validator(
                DefinitionKind::new(kind).map_err(|error| ModError::msg(error.to_string()))?,
                tacsim::definitions::exact_validator(descriptors),
            )?;
        }
        for descriptor in &self.0.descriptors {
            reg.definition(descriptor.clone())?;
        }
        Ok(())
    }
}

fn fixture() -> Result<(GovernedLineCores, TestWorld), Box<dyn std::error::Error>> {
    let family = GovernedLineCores::build(Catalog::build_fielded(&[], tacsim::NS)?)?;
    let world = TestWorld::with_modules(vec![Box::new(Definitions(family.catalog().clone()))]);
    Ok((family, world))
}

fn registry(world: &TestWorld) -> Result<&DefinitionRegistry, Box<dyn std::error::Error>> {
    world
        .registry()
        .definitions()
        .ok_or_else(|| "actual registry missing".into())
}

fn unit(
    family: &GovernedLineCores,
    class: CoreClass,
    object: ObjId,
) -> Result<TacticalUnit, Box<dyn std::error::Error>> {
    let reference = family.definition(class).ok_or("class missing")?;
    let spec = family
        .catalog()
        .units
        .get_exact(reference)
        .ok_or("spec missing")?;
    Ok(TacticalUnit::from_spec(spec, object)?)
}

fn change(
    unit: &TacticalUnit,
    ops: Vec<UnitMutationOp>,
) -> Result<TacticalUnit, Box<dyn std::error::Error>> {
    Ok(mush_tactics::unit::apply(
        unit,
        &UnitMutationPlan::new(ops, Some(unit.revision()), None)?,
        19,
    )?
    .0)
}

fn incoming_unit(
    family: &GovernedLineCores,
    class: CoreClass,
    asset: ObjId,
) -> Result<TacticalUnit, Box<dyn std::error::Error>> {
    change(
        &unit(family, class, ObjId(200))?,
        vec![UnitMutationOp::SetComponentAsset {
            component: ComponentId::new("core")?,
            asset: Some(asset),
        }],
    )
}

fn captured(unit: &TacticalUnit) -> Result<ComponentState, Box<dyn std::error::Error>> {
    Ok(ComponentState::capture(unit, &ComponentId::new("core")?)?)
}

#[test]
fn governed_family_registers_exact_roots_without_reminting_unrelated_components() -> TestResult {
    let stock = Catalog::build_fielded(&[], tacsim::NS)?;
    let (family, world) = fixture()?;
    assert_eq!(family.catalog().units.len(), stock.units.len() + 2);
    for old in stock.units.specs() {
        assert_eq!(family.catalog().units.get_exact(&old.definition), Some(old));
    }
    let base = stock.units.get(LINE_II).ok_or("line missing")?;
    for (class, name) in [
        (CoreClass::I, LINE_I),
        (CoreClass::II, LINE_II),
        (CoreClass::III, LINE_III),
    ] {
        let reference = family.definition(class).ok_or("root missing")?;
        assert_eq!(reference.key().id.as_str(), name);
        let entry = registry(&world)?.get_exact(reference)?;
        let spec = family
            .catalog()
            .units
            .get_exact(reference)
            .ok_or("spec missing")?;
        assert_eq!(entry.payload(), spec.descriptor()?.payload());
        assert_eq!(spec.sections, base.sections);
        assert_eq!(spec.mobility, base.mobility);
        for component in base
            .components
            .iter()
            .filter(|row| row.id.as_str() != "core")
        {
            assert_eq!(
                spec.components.iter().find(|row| row.id == component.id),
                Some(component)
            );
        }
        let mut actual_rules = spec.rules.data().clone();
        let mut original_rules = base.rules.data().clone();
        // The embedded declaration is the sole opaque-rules change.
        for rules in [&mut actual_rules, &mut original_rules] {
            let rows = rules
                .get_mut("components")
                .and_then(serde_json::Value::as_array_mut)
                .ok_or("declarations missing")?;
            rows.retain(|row| row.get("id").and_then(serde_json::Value::as_str) != Some("core"));
        }
        assert_eq!(actual_rules, original_rules);
    }
    assert!(family.definition(CoreClass::Zero).is_none());
    Ok(())
}

#[test]
fn every_finite_transition_preserves_damage_ammo_capabilities_and_current_mobility() -> TestResult {
    let (family, world) = fixture()?;
    let core = ComponentId::new("core")?;
    for from in [CoreClass::I, CoreClass::II, CoreClass::III] {
        for to in [CoreClass::I, CoreClass::II, CoreClass::III] {
            let original = unit(&family, from, ObjId(101))?;
            let mut ops = Vec::new();
            for (address, cell) in original.cells() {
                if matches!(address, CellAddr::Component { component, .. } if component == &core) {
                    continue;
                }
                let value = if cell.current > cell.min {
                    cell.current - 1
                } else {
                    (cell.current + 1).min(cell.max)
                };
                ops.push(UnitMutationOp::SetCell {
                    addr: address.clone(),
                    value,
                });
            }
            assert!(
                ops.len() > 10,
                "real armor/internals/ammunition/condition cells exercised"
            );
            ops.extend([
                UnitMutationOp::SetSectionCondition {
                    section: mush_tactics::SectionId::new("left_track")?,
                    condition: ConditionId::new("damaged")?,
                },
                UnitMutationOp::SetComponentCondition {
                    component: ComponentId::new("cannon")?,
                    condition: ConditionId::new("damaged")?,
                },
                UnitMutationOp::SetComponentAsset {
                    component: ComponentId::new("cannon")?,
                    asset: Some(ObjId(303)),
                },
                UnitMutationOp::SetCell {
                    addr: CellAddr::Component {
                        component: core.clone(),
                        cell: CellId::new("core_class")?,
                    },
                    value: 0,
                },
                UnitMutationOp::SetComponentCondition {
                    component: core.clone(),
                    condition: ConditionId::new("destroyed")?,
                },
                UnitMutationOp::SetComponentAsset {
                    component: core.clone(),
                    asset: Some(ObjId(301)),
                },
                UnitMutationOp::RevokeCapability {
                    key: CapabilityKey {
                        id: CapabilityId::new("tacsim.core")?,
                        source: Some(core.clone()),
                    },
                },
                UnitMutationOp::RevokeCapability {
                    key: CapabilityKey {
                        id: CapabilityId::new("tacsim.power")?,
                        source: Some(ComponentId::new("power")?),
                    },
                },
            ]);
            let mut wire = serde_json::to_value(change(&original, ops)?)?;
            let mode = original
                .mobility()
                .and_then(|profile| profile.modes().first())
                .ok_or("mode missing")?;
            *wire
                .pointer_mut("/mobility/mode")
                .ok_or("mode state missing")? =
                serde_json::to_value(mush_tactics::mobility::ModeState::Switching {
                    from: mode.id.clone(),
                    to: mode.id.clone(),
                    ready_at: 500,
                })?;
            *wire
                .pointer_mut("/mobility/modes/0/rules/data/max_speed_cms")
                .ok_or("speed missing")? = serde_json::json!(17);
            let current: TacticalUnit = serde_json::from_value(wire)?;
            let pre = serde_json::to_vec(&current)?;
            let incoming = captured(&incoming_unit(&family, to, ObjId(302))?)?;
            let displaced = captured(&current)?;
            let (post, journal) = family.replace(
                registry(&world)?,
                &current,
                &incoming,
                current.revision(),
                900,
            )?;
            assert_eq!(
                serde_json::to_vec(&current)?,
                pre,
                "preimage remains untouched"
            );
            assert_eq!(post.object(), current.object());
            assert_eq!(post.revision(), current.revision() + 1);
            assert_eq!(
                post.sections().collect::<Vec<_>>(),
                current.sections().collect::<Vec<_>>()
            );
            assert_eq!(
                post.mobility(),
                current.mobility(),
                "even a due switch is not settled by fitting"
            );
            for (address, value) in current.cells() {
                if !matches!(address, CellAddr::Component { component, .. } if component == &core) {
                    assert_eq!(post.cell(address), Some(value));
                }
            }
            for (id, value) in current.components().filter(|(id, _)| *id != &core) {
                assert_eq!(post.component(id), Some(value));
            }
            let before_caps: Vec<_> = current
                .capabilities()
                .filter(|key| key.source.as_ref() != Some(&core))
                .map(|key| (key, current.capability(key)))
                .collect();
            let after_caps: Vec<_> = post
                .capabilities()
                .filter(|key| key.source.as_ref() != Some(&core))
                .map(|key| (key, post.capability(key)))
                .collect();
            assert_eq!(after_caps, before_caps);
            assert_eq!(journal.displaced, displaced);
            assert_eq!(journal.inserted, incoming);
            assert_eq!(journal.from_revision, current.revision());
            assert_eq!(journal.to_revision, post.revision());
            assert_eq!(journal.now, 900);
            assert_eq!(
                post.schema(),
                family.definition(to).ok_or("target missing")?
            );
            let restored: TacticalUnit = serde_json::from_slice(&serde_json::to_vec(&post)?)?;
            registered_spec(registry(&world)?, post.schema())?.validate_instance(&restored)?;
            if from != to {
                assert!(
                    registered_spec(registry(&world)?, current.schema())?
                        .validate_instance(&restored)
                        .is_err()
                );
            }
        }
    }
    Ok(())
}

#[test]
fn stale_revision_overflow_and_modified_opaque_rules_refuse_without_mutation() -> TestResult {
    let (family, world) = fixture()?;
    let current = unit(&family, CoreClass::II, ObjId(101))?;
    let incoming = captured(&incoming_unit(&family, CoreClass::I, ObjId(302))?)?;
    assert!(
        family
            .replace(
                registry(&world)?,
                &current,
                &incoming,
                current.revision() + 1,
                0
            )
            .is_err()
    );
    for (path, value) in [
        ("/revision", serde_json::json!(u64::MAX)),
        ("/rules/data/sensor_height_cm", serde_json::json!(123)),
    ] {
        let mut wire = serde_json::to_value(&current)?;
        *wire.pointer_mut(path).ok_or("fixture path missing")? = value;
        let altered: TacticalUnit = serde_json::from_value(wire)?;
        let before = serde_json::to_vec(&altered)?;
        assert!(
            family
                .replace(
                    registry(&world)?,
                    &altered,
                    &incoming,
                    altered.revision(),
                    0
                )
                .is_err()
        );
        assert_eq!(serde_json::to_vec(&altered)?, before);
    }
    Ok(())
}

#[test]
fn incoming_damage_class_governance_missing_asset_and_duplicate_asset_refuse() -> TestResult {
    let (family, world) = fixture()?;
    let current = unit(&family, CoreClass::II, ObjId(101))?;
    let healthy = incoming_unit(&family, CoreClass::III, ObjId(302))?;
    let core = ComponentId::new("core")?;
    let key = CapabilityKey {
        id: CapabilityId::new("tacsim.core")?,
        source: Some(core.clone()),
    };
    let schema = healthy
        .capability(&key)
        .ok_or("core capability missing")?
        .schema()
        .clone();
    for op in [
        UnitMutationOp::SetCell {
            addr: CellAddr::Component {
                component: core.clone(),
                cell: CellId::new("core_class")?,
            },
            value: 2,
        },
        UnitMutationOp::SetComponentCondition {
            component: core.clone(),
            condition: ConditionId::new("damaged")?,
        },
        UnitMutationOp::SetComponentAsset {
            component: core.clone(),
            asset: None,
        },
        UnitMutationOp::GrantCapability {
            key: key.clone(),
            rules: mush_core::RulesValue::new(
                schema,
                tacsim::definitions::core::profile_payload(CoreClass::III, false),
            )?,
        },
    ] {
        let bad = captured(&change(&healthy, vec![op])?)?;
        assert!(
            family
                .replace(registry(&world)?, &current, &bad, current.revision(), 0)
                .is_err()
        );
    }
    let already_installed = change(
        &current,
        vec![UnitMutationOp::SetComponentAsset {
            component: ComponentId::new("cannon")?,
            asset: Some(ObjId(302)),
        }],
    )?;
    assert!(
        family
            .replace(
                registry(&world)?,
                &already_installed,
                &captured(&healthy)?,
                already_installed.revision(),
                0
            )
            .is_err()
    );
    assert!(
        family
            .replace(
                registry(&world)?,
                &current,
                &captured(&incoming_unit(&family, CoreClass::III, current.object())?)?,
                current.revision(),
                0
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn unsupported_chassis_missing_core_mount_and_moved_mount_refuse() -> TestResult {
    let (family, world) = fixture()?;
    let incoming = captured(&incoming_unit(&family, CoreClass::I, ObjId(302))?)?;
    let mut refused = 0;
    for spec in
        family.catalog().units.specs().filter(|spec| {
            ![LINE_I, LINE_II, LINE_III].contains(&spec.definition.key().id.as_str())
        })
    {
        let other = TacticalUnit::from_spec(spec, ObjId(101))?;
        assert!(
            family
                .replace(registry(&world)?, &other, &incoming, other.revision(), 0)
                .is_err()
        );
        refused += 1;
    }
    assert!(refused > 0, "actual distinct fielded chassis exercised");
    let current = unit(&family, CoreClass::II, ObjId(101))?;
    for remove in [false, true] {
        let mut wire = serde_json::to_value(&current)?;
        if remove {
            // Removing just the installed row is intrinsically invalid because
            // cells/capabilities still reference it; serde must already refuse.
            wire.get_mut("components")
                .and_then(serde_json::Value::as_object_mut)
                .ok_or("components missing")?
                .remove("core");
            assert!(serde_json::from_value::<TacticalUnit>(wire).is_err());
        } else {
            *wire
                .pointer_mut("/components/core/install")
                .ok_or("mount missing")? = serde_json::to_value(
                mush_tactics::InstallPoint::Section(mush_tactics::SectionId::new("hull")?),
            )?;
            let moved: TacticalUnit = serde_json::from_value(wire)?;
            assert!(
                family
                    .replace(registry(&world)?, &moved, &incoming, moved.revision(), 0)
                    .is_err()
            );
        }
    }
    Ok(())
}

#[test]
fn same_payload_different_unrelated_component_reference_is_not_equivalence() -> TestResult {
    let (family, world) = fixture()?;
    let mut definitions = mush_core::DefinitionRegistryBuilder::new();
    let owner = mush_core::OwnerId::new(tacsim::NS)?;
    for descriptor in &family.catalog().descriptors {
        definitions.register(owner.clone(), descriptor.clone())?;
    }
    let mut body = registered_spec(
        registry(&world)?,
        family.definition(CoreClass::II).ok_or("root missing")?,
    )?
    .body();
    let cannon = body
        .components
        .iter_mut()
        .find(|row| row.id.as_str() == "cannon")
        .ok_or("cannon missing")?;
    let entry = registry(&world)?.get_exact(&cannon.definition)?;
    let descriptor = mush_core::DefinitionDescriptor::new(
        DefinitionKind::new("component")?,
        mush_core::DefinitionId::new("tacsim.test.same_cannon")?,
        1,
        entry.payload().clone(),
    )?
    .with_references(entry.references().to_vec());
    cannon.definition = definitions.register(owner.clone(), descriptor)?;
    let descriptor = body.descriptor(
        DefinitionKind::new("unit")?,
        mush_core::DefinitionId::new("tacsim.unit.same_shape")?,
        1,
    )?;
    let reference = definitions.register(owner, descriptor)?;
    let registry = definitions.freeze()?;
    let forged = TacticalUnit::from_spec(&body.with_definition(reference), ObjId(101))?;
    let incoming = captured(&incoming_unit(&family, CoreClass::I, ObjId(302))?)?;
    assert!(
        replace_component(
            &registry,
            &forged,
            family.definition(CoreClass::I).ok_or("root missing")?,
            &ComponentId::new("core")?,
            &incoming,
            forged.revision(),
            0
        )
        .is_err()
    );
    assert!(
        family
            .replace(&registry, &forged, &incoming, forged.revision(), 0)
            .is_err()
    );
    Ok(())
}

#[test]
fn replacement_is_not_an_ordinary_world_mutation_verb() {
    assert!(
        serde_json::from_value::<UnitMutationOp>(
            serde_json::json!({"InstallComponent":{"component":"core"}})
        )
        .is_err()
    );
}

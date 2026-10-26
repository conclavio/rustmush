//! The finite governed I/II/III line-tank family and its pure fitting adapter.
//! Explicit composition only: this does not alter the ordinary fielded catalog,
//! register an action, grant economic authority, or write a World unit.

use mush_core::{
    CanonicalId, DefinitionRef, DefinitionRegistry, DefinitionRegistryBuilder, RulesValue,
};
use mush_tactics::unit::replacement::{
    ComponentState, ReplacementJournal, registered_spec, replace_component,
};
use mush_tactics::{ComponentId, MobilityVocabulary, TacticalUnit, UnitCatalogBuilder};

use super::{Catalog, DefinitionError, Minter, UnitBinding, core, unit};
use crate::arena::core::CoreClass;

/// The existing Class II root is retained byte for byte.
pub const LINE_II: &str = "tacsim.unit.line_tank";
/// Explicit Class I fitted root; it retains the existing non-core components.
pub const LINE_I: &str = "tacsim.unit.line_tank_core_i";
/// Explicit Class III fitted root; it retains the existing non-core components.
pub const LINE_III: &str = "tacsim.unit.line_tank_core_iii";

fn bad(detail: impl Into<String>) -> DefinitionError {
    DefinitionError::value("governed line core replacement", detail)
}

/// Immutable definition bundle produced from the exact shipped line chassis.
/// Consumers register `catalog().descriptors` using the existing strict owner
/// validators and install its `units` catalog together. No other chassis is
/// admitted by this finite adapter, including the laser/thermal/ECM variants.
#[derive(Clone, Debug)]
pub struct GovernedLineCores {
    catalog: Catalog,
    members: [(CoreClass, DefinitionRef); 3],
}

impl GovernedLineCores {
    /// Extend a catalog with two explicitly registered shapes, preserving all
    /// original definitions, aliases and unchanged component references. An
    /// operator override of this finite chassis/core family is refused rather
    /// than inferred equivalent by name or scalar values.
    pub fn build(mut catalog: Catalog) -> Result<Self, DefinitionError> {
        let shipped = Catalog::build_game(&[], crate::NS)?;
        let base = catalog
            .units
            .get(LINE_II)
            .ok_or_else(|| bad("line tank missing"))?
            .clone();
        if Some(&base) != shipped.units.get(LINE_II) {
            return Err(bad("line tank is not the exact shipped shape"));
        }
        let mut definitions = DefinitionRegistryBuilder::new();
        let owner = base.definition.key().owner.clone();
        for descriptor in &catalog.descriptors {
            definitions
                .register(owner.clone(), descriptor.clone())
                .map_err(|error| bad(error.to_string()))?;
        }
        let component = ComponentId::new("core").map_err(|error| bad(error.to_string()))?;
        let core_schema = base
            .capabilities
            .iter()
            .find(|row| row.key.source.as_ref() == Some(&component))
            .ok_or_else(|| bad("line core capability missing"))?
            .rules
            .schema()
            .clone();
        let base_id = CanonicalId::new(LINE_II).map_err(|error| bad(error.to_string()))?;
        let authored = catalog
            .specs
            .get(&base_id)
            .ok_or_else(|| bad("authored line missing"))?
            .clone();
        let mut added = Vec::new();
        for (class, root, core_name) in [
            (CoreClass::I, LINE_I, "tacsim.core.doctrine"),
            (CoreClass::III, LINE_III, "tacsim.core.command"),
        ] {
            let core_id = CanonicalId::new(core_name).map_err(|error| bad(error.to_string()))?;
            let core_ref = catalog
                .references
                .get(&core_id)
                .ok_or_else(|| bad("core reference missing"))?
                .clone();
            if catalog.cores.get(&core_id) != shipped.cores.get(&core_id)
                || catalog.references.get(&core_id) != shipped.references.get(&core_id)
                || catalog
                    .cores
                    .get(&core_id)
                    .is_none_or(|row| row.class != class || !row.governed)
            {
                return Err(bad(
                    "incoming core definition is not the exact governed stock class",
                ));
            }
            let component_name = format!("{root}.core");
            let component_descriptor = unit::descriptor(
                "component",
                &component_name,
                1,
                serde_json::json!({"role":"core", "core":core_name}),
                vec![core_ref],
            )
            .map_err(bad)?
            .as_current();
            let component_ref = definitions
                .register(owner.clone(), component_descriptor.clone())
                .map_err(|error| bad(error.to_string()))?;
            let mut body = base.body();
            let row = body
                .components
                .iter_mut()
                .find(|row| row.id == component)
                .ok_or_else(|| bad("line core mount missing"))?;
            row.definition = component_ref.clone();
            row.cells = vec![core::core_cell(class).ok_or_else(|| bad("core class missing"))?];
            let capability = body
                .capabilities
                .iter_mut()
                .find(|row| row.key.source.as_ref() == Some(&component))
                .ok_or_else(|| bad("line core capability missing"))?;
            *capability = core::core_capability(&component, class, true, core_schema.clone())
                .ok_or_else(|| bad("core capability invalid"))?;
            let mut authored_variant = authored.clone();
            authored_variant
                .components
                .iter_mut()
                .find(|row| row.id.as_str() == "core")
                .ok_or_else(|| bad("authored core missing"))?
                .role = unit::ComponentRole::Core { core: core_id };
            let mut rules = body.rules.data().clone();
            let declarations = rules
                .get_mut("components")
                .ok_or_else(|| bad("core declarations missing"))?;
            *declarations = serde_json::to_value(&authored_variant.components)
                .map_err(|error| bad(error.to_string()))?;
            body.rules = RulesValue::new(body.rules.schema().clone(), rules)
                .map_err(|error| bad(error.to_string()))?;
            let root_id = CanonicalId::new(root).map_err(|error| bad(error.to_string()))?;
            let descriptor = body
                .descriptor(
                    base.definition.key().kind.clone(),
                    mush_core::DefinitionId::new(root).map_err(|error| bad(error.to_string()))?,
                    1,
                )
                .map_err(|error| bad(error.to_string()))?
                .as_current();
            let reference = unit::register(
                &mut Minter {
                    scratch: &mut definitions,
                    owner: &owner,
                },
                descriptor.clone(),
                root,
            )?;
            let spec = body.with_definition(reference.clone());
            catalog
                .descriptors
                .extend([component_descriptor.clone(), descriptor.clone()]);
            catalog.references.insert(
                CanonicalId::new(&component_name).map_err(|error| bad(error.to_string()))?,
                component_ref,
            );
            catalog
                .references
                .insert(root_id.clone(), reference.clone());
            catalog.specs.insert(root_id.clone(), authored_variant);
            catalog.bindings.insert(
                root_id,
                UnitBinding {
                    spec: spec.clone(),
                    descriptors: vec![component_descriptor, descriptor],
                },
            );
            added.push((class, spec));
        }
        // Validate the complete dependency closure, not only locally minted refs.
        definitions
            .freeze()
            .map_err(|error| bad(error.to_string()))?;
        let mobility = MobilityVocabulary::base().freeze();
        let mut units = UnitCatalogBuilder::new();
        for spec in catalog
            .units
            .specs()
            .chain(added.iter().map(|(_, spec)| spec))
        {
            units
                .add(spec.clone(), &mobility)
                .map_err(|error| bad(error.to_string()))?;
        }
        for (id, version, target) in catalog.units.legacy_v1_aliases() {
            units
                .legacy_v1_alias(id.clone(), version, target.clone())
                .map_err(|error| bad(error.to_string()))?;
        }
        catalog.units = units.freeze();
        let lookup = |name| {
            catalog
                .units
                .get(name)
                .map(|spec| spec.definition.clone())
                .ok_or_else(|| bad("family root missing"))
        };
        let members = [
            (CoreClass::I, lookup(LINE_I)?),
            (CoreClass::II, lookup(LINE_II)?),
            (CoreClass::III, lookup(LINE_III)?),
        ];
        Ok(Self { catalog, members })
    }

    /// Exact descriptors, bindings and specs for trusted composition.
    #[must_use]
    pub const fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Resolve a finite family member; Class 0 is not an empty physical mount.
    pub fn definition(&self, class: CoreClass) -> Option<&DefinitionRef> {
        self.members
            .iter()
            .find(|(found, _)| *found == class)
            .map(|(_, reference)| reference)
    }

    /// Prepare a healthy governed core replacement. The incoming state must
    /// carry a real asset pointer, but that pointer alone proves no ownership,
    /// custody, availability or standing. Those checks belong to the later
    /// closed composed hardware action. The displaced state is never repaired.
    pub fn replace(
        &self,
        registry: &DefinitionRegistry,
        current: &TacticalUnit,
        incoming: &ComponentState,
        expected_revision: u64,
        now: i64,
    ) -> Result<(TacticalUnit, ReplacementJournal), DefinitionError> {
        let core = ComponentId::new("core").map_err(|error| bad(error.to_string()))?;
        let (fitted, current_ref) = self
            .members
            .iter()
            .find(|(_, reference)| reference == current.schema())
            .ok_or_else(|| bad("unsupported chassis or exact root"))?;
        let before =
            registered_spec(registry, current_ref).map_err(|error| bad(error.to_string()))?;
        if self.catalog.units.get_exact(current_ref) != Some(&before) {
            return Err(bad("registered current body differs"));
        }
        before
            .validate_instance(current)
            .map_err(|error| bad(error.to_string()))?;
        let baseline = TacticalUnit::from_spec(&before, current.object())
            .map_err(|error| bad(error.to_string()))?;
        let expected_old =
            ComponentState::capture(&baseline, &core).map_err(|error| bad(error.to_string()))?;
        let old =
            ComponentState::capture(current, &core).map_err(|error| bad(error.to_string()))?;
        if old
            .cells()
            .values()
            .any(|cell| cell.current > i64::from(fitted.ordinal()))
            || old
                .capabilities()
                .iter()
                .any(|(key, value)| expected_old.capabilities().get(key) != Some(value))
        {
            return Err(bad("current core fitting metadata differs"));
        }
        let target = self
            .members
            .iter()
            .find_map(|(_, reference)| {
                let spec = self.catalog.units.get_exact(reference)?;
                spec.components
                    .iter()
                    .find(|row| row.id == core && row.definition == incoming.installed().definition)
                    .map(|_| reference)
            })
            .ok_or_else(|| bad("unsupported incoming core definition"))?;
        let after = registered_spec(registry, target).map_err(|error| bad(error.to_string()))?;
        if self.catalog.units.get_exact(target) != Some(&after) {
            return Err(bad("registered target body differs"));
        }
        let baseline = TacticalUnit::from_spec(&after, current.object())
            .map_err(|error| bad(error.to_string()))?;
        let healthy =
            ComponentState::capture(&baseline, &core).map_err(|error| bad(error.to_string()))?;
        if incoming.installed().condition != healthy.installed().condition
            || incoming.installed().install != healthy.installed().install
            || incoming.installed().asset.is_none()
            || incoming.cells() != healthy.cells()
            || incoming.capabilities() != healthy.capabilities()
        {
            return Err(bad(
                "incoming core is not intact, governed and at its fitted class",
            ));
        }
        replace_component(
            registry,
            current,
            target,
            &core,
            incoming,
            expected_revision,
            now,
        )
        .map_err(|error| bad(error.to_string()))
    }
}

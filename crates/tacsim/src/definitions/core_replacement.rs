//! The finite I/II/III fitting families and their pure fitting adapter: the
//! governed line tanks (`GovernedLineCores`) and, since doc 17h cut M8b, the
//! surface rig (`SurfaceRigCores`, the terrain vehicle's cored roots). Each
//! family carries a governed root per class and, since M8b (doc 17 §10,
//! D-UM-42), an **ungoverned twin** of each, which only the ungoverning recipe
//! (`core_ungovern`) fits.
//! Explicit composition only: this does not alter the ordinary fielded catalog,
//! register an action, grant economic authority, or write a World unit.

use mush_core::{
    CanonicalId, DefinitionRef, DefinitionRegistry, DefinitionRegistryBuilder, RulesValue,
};
use mush_tactics::unit::replacement::{
    ComponentState, ReplacementJournal, registered_spec, replace_component,
};
use mush_tactics::{
    ComponentId, ComponentSpec, ConditionId, InstallPoint, MobilityVocabulary, SectionId,
    TacticalUnit, UnitCatalogBuilder,
};

use super::{Catalog, DefinitionError, Minter, UnitBinding, core, unit};
use crate::arena::core::CoreClass;

/// The existing Class II root is retained byte for byte.
pub const LINE_II: &str = "tacsim.unit.line_tank";
/// Explicit Class I fitted root; it retains the existing non-core components.
pub const LINE_I: &str = "tacsim.unit.line_tank_core_i";
/// Explicit Class III fitted root; it retains the existing non-core components.
pub const LINE_III: &str = "tacsim.unit.line_tank_core_iii";
/// The line tank's ungoverned Class I twin (doc 17h cut M8b C2).
pub const LINE_I_UNGOVERNED: &str = "tacsim.unit.line_tank_core_i_ungoverned";
/// The line tank's ungoverned Class II twin (doc 17h cut M8b C2).
pub const LINE_II_UNGOVERNED: &str = "tacsim.unit.line_tank_core_ii_ungoverned";
/// The line tank's ungoverned Class III twin (doc 17h cut M8b C2).
pub const LINE_III_UNGOVERNED: &str = "tacsim.unit.line_tank_core_iii_ungoverned";

/// The terrain vehicle's Class I fitted root (doc 17h cut M8b C1): the shipped
/// `tacsim.unit.tv_prospector` with a core in its chassis and every other
/// component retained. The shipped rig itself stays coreless (Class 0, the ROM):
/// the unit schema admits no empty core mount, so only these roots are cored.
pub const RIG_I: &str = "tacsim.unit.tv_prospector_core_i";
/// The terrain vehicle's Class II fitted root (doc 17h cut M8b C1).
pub const RIG_II: &str = "tacsim.unit.tv_prospector_core_ii";
/// The terrain vehicle's Class III fitted root (doc 17h cut M8b C1).
pub const RIG_III: &str = "tacsim.unit.tv_prospector_core_iii";
/// The terrain vehicle's ungoverned Class I twin (doc 17h cut M8b C2).
pub const RIG_I_UNGOVERNED: &str = "tacsim.unit.tv_prospector_core_i_ungoverned";
/// The terrain vehicle's ungoverned Class II twin (doc 17h cut M8b C2).
pub const RIG_II_UNGOVERNED: &str = "tacsim.unit.tv_prospector_core_ii_ungoverned";
/// The terrain vehicle's ungoverned Class III twin (doc 17h cut M8b C2).
pub const RIG_III_UNGOVERNED: &str = "tacsim.unit.tv_prospector_core_iii_ungoverned";

/// **The chassis a fitted root is** (doc 17h cut M8b G1): a rig root (governed or twin) is
/// the shipped terrain vehicle, a line root the shipped line tank; any other id is itself. A
/// fitted root differs from its base only in the core mount, so every surface read (interior,
/// logistics, drive rates) is the base's.
#[must_use]
pub fn surface_chassis(id: &str) -> &str {
    if [
        RIG_I,
        RIG_II,
        RIG_III,
        RIG_I_UNGOVERNED,
        RIG_II_UNGOVERNED,
        RIG_III_UNGOVERNED,
    ]
    .contains(&id)
    {
        crate::definitions::content::TV_PROSPECTOR
    } else if [
        LINE_I,
        LINE_III,
        LINE_I_UNGOVERNED,
        LINE_II_UNGOVERNED,
        LINE_III_UNGOVERNED,
    ]
    .contains(&id)
    {
        LINE_II
    } else {
        id
    }
}

/// The shipped governed core of each class.
const GOVERNED_CORES: [(CoreClass, &str); 3] = [
    (CoreClass::I, "tacsim.core.doctrine"),
    (CoreClass::II, "tacsim.core.line"),
    (CoreClass::III, "tacsim.core.command"),
];
/// The shipped ungoverned twin of each class (doc 17h cut M8b C2,
/// `content/cores/cores.json`): the same class, the governor's bit off.
const UNGOVERNED_CORES: [(CoreClass, &str); 3] = [
    (CoreClass::I, "tacsim.core.doctrine_ungoverned"),
    (CoreClass::II, "tacsim.core.line_ungoverned"),
    (CoreClass::III, "tacsim.core.command_ungoverned"),
];

/// **Which recipe fits a core** (doc 17h cut M8b C2): the governed
/// `core_refit` fits a governed root only; the ungoverning `core_ungovern`
/// fits an ungoverned twin only. Neither ever takes a hull to Class 0.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FittingRecipe {
    /// `core_refit`: the chartered fitting, governed roots.
    Refit,
    /// `core_ungovern`: the ungoverning refit, ungoverned twins.
    Ungovern,
}
impl FittingRecipe {
    /// Whether this recipe's targets carry the governor.
    #[must_use]
    pub const fn governed(self) -> bool {
        matches!(self, Self::Refit)
    }
}

fn bad(detail: impl Into<String>) -> DefinitionError {
    DefinitionError::value("governed line core replacement", detail)
}

/// Where a family's core goes on its chassis.
#[derive(Clone, Copy)]
enum Mount {
    /// The base chassis carries a core component; the roots swap it.
    Replace,
    /// The base chassis carries none; the roots add one in this section.
    Add(&'static str),
}

/// One family's chassis and its six roots, `(class, governed, name)`. A
/// `None` name is the base chassis itself, retained.
struct Chassis {
    base: &'static str,
    mount: Mount,
    roots: [(CoreClass, bool, Option<&'static str>); 6],
}

const LINE: Chassis = Chassis {
    base: LINE_II,
    mount: Mount::Replace,
    roots: [
        (CoreClass::I, true, Some(LINE_I)),
        (CoreClass::II, true, None),
        (CoreClass::III, true, Some(LINE_III)),
        (CoreClass::I, false, Some(LINE_I_UNGOVERNED)),
        (CoreClass::II, false, Some(LINE_II_UNGOVERNED)),
        (CoreClass::III, false, Some(LINE_III_UNGOVERNED)),
    ],
};

const RIG: Chassis = Chassis {
    base: crate::definitions::content::TV_PROSPECTOR,
    mount: Mount::Add("chassis"),
    roots: [
        (CoreClass::I, true, Some(RIG_I)),
        (CoreClass::II, true, Some(RIG_II)),
        (CoreClass::III, true, Some(RIG_III)),
        (CoreClass::I, false, Some(RIG_I_UNGOVERNED)),
        (CoreClass::II, false, Some(RIG_II_UNGOVERNED)),
        (CoreClass::III, false, Some(RIG_III_UNGOVERNED)),
    ],
};

/// One fitted root: its class, whether its core is governed, its reference.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Member {
    class: CoreClass,
    governed: bool,
    definition: DefinitionRef,
}

/// **One finite fitting family** (the shared body of [`GovernedLineCores`]
/// and [`SurfaceRigCores`]): the catalog with the family's roots added, and
/// the roots by class and governor.
#[derive(Clone, Debug)]
pub struct CoreFamily {
    catalog: Catalog,
    members: Vec<Member>,
}

/// Immutable definition bundle produced from the exact shipped line chassis.
/// Consumers register `catalog().descriptors` using the existing strict owner
/// validators and install its `units` catalog together. No other chassis is
/// admitted by this finite adapter, including the laser/thermal/ECM variants.
#[derive(Clone, Debug)]
pub struct GovernedLineCores(CoreFamily);

/// **The surface rig's fitting family** (doc 17h cut M8b C1): the exact
/// shipped terrain vehicle with a core added in its chassis, one root per
/// class and an ungoverned twin of each. Built on a catalog that already
/// carries the line family, so its catalog is the whole fitted set.
#[derive(Clone, Debug)]
pub struct SurfaceRigCores(CoreFamily);

impl GovernedLineCores {
    /// Extend a catalog with the explicitly registered shapes, preserving all
    /// original definitions, aliases and unchanged component references. An
    /// operator override of this finite chassis/core family is refused rather
    /// than inferred equivalent by name or scalar values.
    pub fn build(catalog: Catalog) -> Result<Self, DefinitionError> {
        let shipped = Catalog::build_game(&[], crate::NS)?;
        CoreFamily::build(catalog, &shipped, &LINE).map(Self)
    }
}

impl SurfaceRigCores {
    /// Extend a catalog with the rig's cored roots (doc 17h cut M8b C1). The
    /// base terrain vehicle must be the exact shipped (fielded) shape.
    pub fn build(catalog: Catalog) -> Result<Self, DefinitionError> {
        let shipped = Catalog::build_fielded(&[], crate::NS)?;
        CoreFamily::build(catalog, &shipped, &RIG).map(Self)
    }
}

impl std::ops::Deref for GovernedLineCores {
    type Target = CoreFamily;
    fn deref(&self) -> &CoreFamily {
        &self.0
    }
}
impl std::ops::Deref for SurfaceRigCores {
    type Target = CoreFamily;
    fn deref(&self) -> &CoreFamily {
        &self.0
    }
}

impl CoreFamily {
    #[allow(clippy::too_many_lines)]
    fn build(
        mut catalog: Catalog,
        shipped: &Catalog,
        chassis: &Chassis,
    ) -> Result<Self, DefinitionError> {
        let base = catalog
            .units
            .get(chassis.base)
            .ok_or_else(|| bad("family chassis missing"))?
            .clone();
        if Some(&base) != shipped.units.get(chassis.base) {
            return Err(bad("family chassis is not the exact shipped shape"));
        }
        let mut definitions = DefinitionRegistryBuilder::new();
        let owner = base.definition.key().owner.clone();
        for descriptor in &catalog.descriptors {
            definitions
                .register(owner.clone(), descriptor.clone())
                .map_err(|error| bad(error.to_string()))?;
        }
        let component = ComponentId::new("core").map_err(|error| bad(error.to_string()))?;
        // The `tacsim.core.profile` schema: off the base's own core, or (a
        // coreless base) off the shipped line tank's, the one minted schema.
        let schema_source = match chassis.mount {
            Mount::Replace => &base,
            Mount::Add(_) => catalog
                .units
                .get(LINE_II)
                .ok_or_else(|| bad("line tank missing"))?,
        };
        let core_schema = schema_source
            .capabilities
            .iter()
            .find(|row| row.key.source.as_ref() == Some(&component))
            .ok_or_else(|| bad("line core capability missing"))?
            .rules
            .schema()
            .clone();
        let base_id = CanonicalId::new(chassis.base).map_err(|error| bad(error.to_string()))?;
        let authored = catalog
            .specs
            .get(&base_id)
            .ok_or_else(|| bad("authored chassis missing"))?
            .clone();
        let mut added = Vec::new();
        let mut named = Vec::new();
        for (class, governed, root) in chassis.roots {
            let Some(root) = root else {
                named.push((class, governed, chassis.base));
                continue;
            };
            named.push((class, governed, root));
            let cores = if governed {
                GOVERNED_CORES
            } else {
                UNGOVERNED_CORES
            };
            let core_name = cores
                .iter()
                .find(|(found, _)| *found == class)
                .map(|(_, name)| *name)
                .ok_or_else(|| bad("core class missing"))?;
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
                    .is_none_or(|row| row.class != class || row.governed != governed)
            {
                return Err(bad(if governed {
                    "incoming core definition is not the exact governed stock class"
                } else {
                    "incoming core definition is not the exact ungoverned stock class"
                }));
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
            let cell = core::core_cell(class).ok_or_else(|| bad("core class missing"))?;
            let capability =
                core::core_capability(&component, class, governed, core_schema.clone())
                    .ok_or_else(|| bad("core capability invalid"))?;
            let mut authored_variant = authored.clone();
            match chassis.mount {
                Mount::Replace => {
                    let row = body
                        .components
                        .iter_mut()
                        .find(|row| row.id == component)
                        .ok_or_else(|| bad("line core mount missing"))?;
                    row.definition = component_ref.clone();
                    row.cells = vec![cell];
                    let found = body
                        .capabilities
                        .iter_mut()
                        .find(|row| row.key.source.as_ref() == Some(&component))
                        .ok_or_else(|| bad("line core capability missing"))?;
                    *found = capability;
                    authored_variant
                        .components
                        .iter_mut()
                        .find(|row| row.id.as_str() == "core")
                        .ok_or_else(|| bad("authored core missing"))?
                        .role = unit::ComponentRole::Core { core: core_id };
                }
                Mount::Add(section) => {
                    if body.components.iter().any(|row| row.id == component)
                        || authored_variant
                            .components
                            .iter()
                            .any(|row| row.id.as_str() == "core")
                    {
                        return Err(bad("chassis already carries a core"));
                    }
                    body.components.push(ComponentSpec {
                        id: component.clone(),
                        definition: component_ref.clone(),
                        install: InstallPoint::Section(
                            SectionId::new(section).map_err(|error| bad(error.to_string()))?,
                        ),
                        condition: ConditionId::new(unit::INTACT)
                            .map_err(|error| bad(error.to_string()))?,
                        cells: vec![cell],
                        asset: None,
                    });
                    body.capabilities.push(capability);
                    authored_variant.components.push(unit::ComponentDef {
                        id: CanonicalId::new("core").map_err(|error| bad(error.to_string()))?,
                        section: CanonicalId::new(section)
                            .map_err(|error| bad(error.to_string()))?,
                        role: unit::ComponentRole::Core { core: core_id },
                    });
                }
            }
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
            added.push(spec);
        }
        // Validate the complete dependency closure, not only locally minted refs.
        definitions
            .freeze()
            .map_err(|error| bad(error.to_string()))?;
        let mobility = MobilityVocabulary::base().freeze();
        let mut units = UnitCatalogBuilder::new();
        for spec in catalog.units.specs().chain(added.iter()) {
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
        let members = named
            .into_iter()
            .map(|(class, governed, name)| {
                catalog
                    .units
                    .get(name)
                    .map(|spec| Member {
                        class,
                        governed,
                        definition: spec.definition.clone(),
                    })
                    .ok_or_else(|| bad("family root missing"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { catalog, members })
    }

    /// Exact descriptors, bindings and specs for trusted composition.
    #[must_use]
    pub const fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Resolve a finite family member's governed root; Class 0 is not an empty
    /// physical mount.
    pub fn definition(&self, class: CoreClass) -> Option<&DefinitionRef> {
        self.root(class, true)
    }

    /// The class's ungoverned twin (doc 17h cut M8b C2), fitted only by the
    /// ungoverning recipe.
    pub fn twin(&self, class: CoreClass) -> Option<&DefinitionRef> {
        self.root(class, false)
    }

    /// The root of `class` with the governor on or off.
    pub fn root(&self, class: CoreClass, governed: bool) -> Option<&DefinitionRef> {
        self.members
            .iter()
            .find(|row| row.class == class && row.governed == governed)
            .map(|row| &row.definition)
    }

    /// Whether `reference` is one of this family's roots, governed or not.
    #[must_use]
    pub fn contains(&self, reference: &DefinitionRef) -> bool {
        self.members.iter().any(|row| &row.definition == reference)
    }

    /// Prepare a healthy governed core replacement (the `core_refit` recipe).
    /// The incoming state must carry a real asset pointer, but that pointer
    /// alone proves no ownership, custody, availability or standing. Those
    /// checks belong to the later closed composed hardware action. The
    /// displaced state is never repaired.
    pub fn replace(
        &self,
        registry: &DefinitionRegistry,
        current: &TacticalUnit,
        incoming: &ComponentState,
        expected_revision: u64,
        now: i64,
    ) -> Result<(TacticalUnit, ReplacementJournal), DefinitionError> {
        self.replace_with(
            FittingRecipe::Refit,
            registry,
            current,
            incoming,
            expected_revision,
            now,
        )
    }

    /// Prepare a healthy replacement under `recipe` (doc 17h cut M8b C2): the
    /// governed recipe fits a governed root only and refuses an ungoverned
    /// payload; the ungoverning recipe fits an ungoverned twin only.
    pub fn replace_with(
        &self,
        recipe: FittingRecipe,
        registry: &DefinitionRegistry,
        current: &TacticalUnit,
        incoming: &ComponentState,
        expected_revision: u64,
        now: i64,
    ) -> Result<(TacticalUnit, ReplacementJournal), DefinitionError> {
        let core = ComponentId::new("core").map_err(|error| bad(error.to_string()))?;
        let fitted = self
            .members
            .iter()
            .find(|row| &row.definition == current.schema())
            .ok_or_else(|| bad("unsupported chassis or exact root"))?;
        let current_ref = &fitted.definition;
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
            .any(|cell| cell.current > i64::from(fitted.class.ordinal()))
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
            .filter(|row| row.governed == recipe.governed())
            .find_map(|row| {
                let spec = self.catalog.units.get_exact(&row.definition)?;
                spec.components
                    .iter()
                    .find(|component| {
                        component.id == core
                            && component.definition == incoming.installed().definition
                    })
                    .map(|_| &row.definition)
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

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::ObjId;

    fn instance(catalog: &Catalog, reference: &DefinitionRef) -> TacticalUnit {
        let spec = catalog.units.get_exact(reference).expect("the root's spec");
        TacticalUnit::from_spec(spec, ObjId(4)).expect("it instantiates")
    }

    /// **Every pod core is governed** (D-UM-41; doc 17h cut M8b C4, the gate's test 6): every
    /// fielded chassis — what the arena's pods and its fill are dealt from — that carries a
    /// core carries a governed one; the three ungoverned cores are content no fielded chassis
    /// fits. So nothing in play lifts the floor.
    #[test]
    fn m8b_every_pod_core_is_governed() {
        let fielded = Catalog::build_fielded(&[], crate::NS).expect("the fielded content");
        let mut cored = 0;
        for (id, binding) in &fielded.bindings {
            let unit = TacticalUnit::from_spec(&binding.spec, ObjId(4)).expect("instantiates");
            if let Some(hardware) = crate::arena::core::read_core(&unit) {
                assert!(hardware.governed, "{id} fits an ungoverned core");
                cored += 1;
            }
        }
        assert!(cored > 0, "the fielded pods carry cores");
        // G11: the lobby fields only what the store's fielded catalog specs (`fieldable_ids`
        // reads `store.definitions().specs`), and no twin root is there; every fielded hull's
        // seal is governed.
        assert!(
            fielded
                .specs
                .keys()
                .all(|id| !id.as_str().ends_with("_ungoverned")),
            "no ungoverned root is fieldable"
        );
        let minds = crate::config::MindsConfig::default();
        let reflex = crate::config::ReflexConfig::default();
        for binding in fielded.bindings.values() {
            let unit = TacticalUnit::from_spec(&binding.spec, ObjId(4)).expect("instantiates");
            let seal = crate::match_::seal::seal_rom_for(
                ObjId(4),
                &unit,
                &[],
                crate::arena::mind::SortieId::ic(ObjId(4), 1),
                &crate::match_::seal::IcSealConfig {
                    minds: &minds,
                    reflex: &reflex,
                    principal: None,
                    impounded: false,
                },
            )
            .expect("seals");
            assert!(!seal.lifts_floor());
        }
        for (class, name) in UNGOVERNED_CORES {
            let core = fielded
                .cores
                .get(&CanonicalId::new(name).expect("an id"))
                .expect("the twin core ships");
            assert_eq!((core.class, core.governed), (class, false));
        }
    }

    /// **The rig's family** (doc 17h cut M8b C1): the shipped terrain vehicle stays coreless
    /// (Class 0, the ROM); each of its six roots carries a core of its class and governor in
    /// the chassis section, every other component, the sections and the mobility retained.
    #[test]
    fn m8b_the_rig_family_cores_the_terrain_vehicle_and_leaves_the_shipped_one_coreless() {
        let fielded = Catalog::build_fielded(&[], crate::NS).expect("the fielded content");
        let line = GovernedLineCores::build(fielded.clone()).expect("the line family");
        let rig = SurfaceRigCores::build(line.catalog().clone()).expect("the rig family");
        let base = fielded
            .units
            .get(crate::definitions::content::TV_PROSPECTOR)
            .expect("the shipped rig");
        let shipped = TacticalUnit::from_spec(base, ObjId(4)).expect("instantiates");
        assert_eq!(crate::arena::core::read_core(&shipped), None);
        assert_eq!(
            rig.catalog()
                .units
                .get(crate::definitions::content::TV_PROSPECTOR),
            Some(base),
            "the shipped rig is byte-identical"
        );
        for class in [CoreClass::I, CoreClass::II, CoreClass::III] {
            for governed in [true, false] {
                let reference = rig.root(class, governed).expect("a root");
                assert!(rig.contains(reference) && !line.contains(reference));
                let spec = rig.catalog().units.get_exact(reference).expect("its spec");
                assert_eq!(spec.sections, base.sections);
                assert_eq!(spec.mobility, base.mobility);
                for component in &base.components {
                    assert_eq!(
                        spec.components.iter().find(|row| row.id == component.id),
                        Some(component)
                    );
                }
                let hardware = crate::arena::core::read_core(&instance(rig.catalog(), reference))
                    .expect("a core");
                assert_eq!((hardware.class, hardware.governed), (class, governed));
            }
        }
        assert_eq!(rig.root(CoreClass::Zero, true), None);
        assert_eq!(
            rig.catalog().units.len(),
            fielded.units.len() + 5 + 6,
            "the line family's five roots and the rig's six"
        );
    }
}

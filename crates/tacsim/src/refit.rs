//! **The refit family** (doc 31 cut UD.4a, D-31-14 and D-31-15 as amended on
//! 2026-10-30): which registered units a hull may be refitted to in a bay, and
//! the refit itself.
//!
//! A refit is a change of registered design: a unit's rules are its
//! definition, so fitting a laser where a cannon was makes the hull the laser
//! variant. [`mush_tactics::unit::replacement::refit_component`] is the pure
//! transition; this module is TacSim's adapter over it. Two rulings shape it:
//!
//! - **Mount-stable, not id-stable** (the owner: *"match by mount is the right
//!   path"*). What comes out and what goes in share a mount, the same install
//!   point; each keeps its own variant's component id, so `cannon` comes out
//!   of the turret and `laser` goes in, as the shipped line tank and its laser
//!   variant already have them — nothing is renamed, and a mech with two guns
//!   needs no slot names.
//! - **The family is found, not declared.** A hull's family is every unit in
//!   the catalogue one admitted transition reaches from it ([`family`]): at
//!   most one component out, at most one in, on one mount, everything else
//!   equal. A published design joins a family by being built that way, and
//!   the bay offers exactly what the transition admits — never a free edit.
//!
//! The section cells a refit may change are the ones TacSim's binder derives
//! from what is fitted: the heat pair a heat-bearing weapon adds to the hull
//! ([`DERIVED_CELLS`]).

use mush_core::{DefinitionRegistry, DefinitionRegistryBuilder, ObjId};
use mush_tactics::unit::replacement::{ComponentState, RefitJournal, refit_component};
use mush_tactics::{CellId, ComponentId, TacticalUnit, UnitError};

use crate::definitions::Catalog;
use crate::definitions::unit::{HEAT_CELL, HEAT_SINK_CELL};

/// The section cells a refit may add, drop or re-bound: the heat pair.
pub const DERIVED_CELLS: [&str; 2] = [HEAT_CELL, HEAT_SINK_CELL];

fn derived() -> Vec<CellId> {
    DERIVED_CELLS
        .iter()
        .filter_map(|c| CellId::new(c).ok())
        .collect()
}

/// The catalogue's descriptors as a frozen registry, which the transition
/// reads the two exact roots from.
///
/// # Errors
///
/// The registry's, for a descriptor that will not register.
pub fn registry_of(catalog: &Catalog) -> Result<DefinitionRegistry, String> {
    let owner = catalog
        .bindings
        .values()
        .next()
        .map(|binding| binding.spec.definition.key().owner.clone())
        .ok_or("the catalogue binds no unit")?;
    let mut builder = DefinitionRegistryBuilder::new();
    for descriptor in &catalog.descriptors {
        let owner = owner.clone();
        builder
            .register(owner, descriptor.clone())
            .map_err(|e| e.to_string())?;
    }
    builder.freeze().map_err(|e| e.to_string())
}

/// One way a hull may be refitted: the unit it becomes, what comes out and
/// what goes in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefitOption {
    /// The unit the hull becomes.
    pub target: mush_core::CanonicalId,
    /// The component taken out, if any.
    pub remove: Option<ComponentId>,
    /// The component fitted, if any.
    pub install: Option<ComponentId>,
}

/// The components `from` has and `to` lacks, and the reverse, by id.
fn diff(
    from: &mush_tactics::UnitSpec,
    to: &mush_tactics::UnitSpec,
) -> (Vec<ComponentId>, Vec<ComponentId>) {
    let ids = |s: &mush_tactics::UnitSpec| -> Vec<ComponentId> {
        s.components.iter().map(|c| c.id.clone()).collect()
    };
    let (a, b) = (ids(from), ids(to));
    let out = a.iter().filter(|id| !b.contains(id)).cloned().collect();
    let into = b.iter().filter(|id| !a.contains(id)).cloned().collect();
    (out, into)
}

/// **The refit family of `unit`**: every unit in `catalog` one admitted
/// transition reaches from it, with what comes out and what goes in, in the
/// catalogue's order. Tried on a copy; nothing is changed.
#[must_use]
pub fn family(
    catalog: &Catalog,
    registry: &DefinitionRegistry,
    unit: &TacticalUnit,
) -> Vec<RefitOption> {
    let Some(from) = catalog.units.get_exact(unit.schema()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (id, binding) in &catalog.bindings {
        let to = &binding.spec;
        if to.definition == from.definition {
            continue;
        }
        let (removed, added) = diff(from, to);
        if removed.len() > 1 || added.len() > 1 || (removed.is_empty() && added.is_empty()) {
            continue;
        }
        let remove = removed.first().cloned();
        let install = added.first().cloned();
        let incoming = match &install {
            Some(c) => match fresh_component(to, unit.object(), c) {
                Ok(state) => Some(state),
                Err(_) => continue,
            },
            None => None,
        };
        let tried = refit_component(
            registry,
            unit,
            &to.definition,
            remove.as_ref(),
            install.as_ref().zip(incoming.as_ref()),
            &derived(),
            unit.revision(),
            0,
        );
        if tried.is_ok() {
            out.push(RefitOption {
                target: id.clone(),
                remove,
                install,
            });
        }
    }
    out
}

/// A new component `component` of `spec`, as it is fitted fresh (a new part
/// from stores: intact, its cells at their starting values).
///
/// # Errors
///
/// The unit's, for a spec that will not instantiate or has no such component.
pub fn fresh_component(
    spec: &mush_tactics::UnitSpec,
    object: ObjId,
    component: &ComponentId,
) -> Result<ComponentState, UnitError> {
    let baseline = TacticalUnit::from_spec(spec, object)?;
    ComponentState::capture(&baseline, component)
}

/// **Refit `unit` to `option`** with `incoming` as the part fitted (a fresh
/// one when `None`): the transition, with TacSim's derived cells.
///
/// # Errors
///
/// An unknown target, or whatever the transition refuses.
pub fn refit(
    catalog: &Catalog,
    registry: &DefinitionRegistry,
    unit: &TacticalUnit,
    option: &RefitOption,
    incoming: Option<ComponentState>,
    now: i64,
) -> Result<(TacticalUnit, RefitJournal), UnitError> {
    let to = &catalog
        .bindings
        .get(&option.target)
        .ok_or_else(|| UnitError::DefinitionDescriptor(format!("no unit {}", option.target)))?
        .spec;
    let incoming = match (&option.install, incoming) {
        (Some(c), None) => Some(fresh_component(to, unit.object(), c)?),
        (_, given) => given,
    };
    refit_component(
        registry,
        unit,
        &to.definition,
        option.remove.as_ref(),
        option.install.as_ref().zip(incoming.as_ref()),
        &derived(),
        unit.revision(),
        now,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fielded() -> (Catalog, DefinitionRegistry) {
        let catalog = Catalog::build_fielded(&[], crate::NS).expect("the fielded content");
        let registry = registry_of(&catalog).expect("it registers");
        (catalog, registry)
    }

    fn hull(catalog: &Catalog, id: &str) -> TacticalUnit {
        let id = mush_core::CanonicalId::new(id).expect("an id");
        let spec = &catalog.bindings.get(&id).expect("a shipped unit").spec;
        TacticalUnit::from_spec(spec, ObjId(77)).expect("it instantiates")
    }

    fn option<'a>(options: &'a [RefitOption], target: &str) -> Option<&'a RefitOption> {
        options.iter().find(|o| o.target.as_str() == target)
    }

    fn has_heat(unit: &TacticalUnit) -> bool {
        unit.cells().any(|(k, _)| {
            matches!(k, mush_tactics::CellAddr::Section { cell, .. } if cell.as_str() == HEAT_CELL)
        })
    }

    fn fitted(unit: &TacticalUnit, id: &str) -> bool {
        unit.components().any(|(c, _)| c.as_str() == id)
    }

    /// **The line tank's family, mount by mount** (the owner's ruling): the
    /// cannon comes out of the turret and the laser goes in under its own id;
    /// a pod goes on and comes off; and each way back is in the other's family.
    #[test]
    fn ud4_the_line_tank_family_is_found_by_mount() {
        let (catalog, registry) = fielded();
        let tank = hull(&catalog, "tacsim.unit.line_tank");
        let options = family(&catalog, &registry, &tank);
        let laser = option(&options, "tacsim.unit.line_tank_laser").expect("the laser variant");
        assert_eq!(
            laser.remove.as_ref().map(ComponentId::as_str),
            Some("cannon")
        );
        assert_eq!(
            laser.install.as_ref().map(ComponentId::as_str),
            Some("laser")
        );
        let ecm = option(&options, "tacsim.unit.line_tank_ecm").expect("the ECM variant");
        assert!(
            ecm.remove.is_none() && ecm.install.is_some(),
            "a pod goes on"
        );
        assert!(
            option(&options, "tacsim.unit.biped_mk1").is_none(),
            "a walker is no refit of a tank"
        );

        let (lasered, journal) =
            refit(&catalog, &registry, &tank, laser, None, 1_000).expect("refitted");
        assert_eq!(lasered.revision(), tank.revision() + 1);
        assert_eq!(
            journal.removed.as_ref().map(|(c, _)| c.as_str()),
            Some("cannon")
        );
        assert!(fitted(&lasered, "laser") && !fitted(&lasered, "cannon"));
        assert!(
            has_heat(&lasered),
            "the laser variant's hull carries the heat pair"
        );
        let back = family(&catalog, &registry, &lasered);
        let cannon = option(&back, "tacsim.unit.line_tank").expect("and back");
        let (again, _) =
            refit(&catalog, &registry, &lasered, cannon, None, 2_000).expect("refitted back");
        assert!(!has_heat(&again), "the heat pair goes with the laser");

        let (podded, _) = refit(&catalog, &registry, &tank, ecm, None, 3_000).expect("pod on");
        let off = option(
            &family(&catalog, &registry, &podded),
            "tacsim.unit.line_tank",
        )
        .cloned()
        .expect("pod off");
        assert!(off.install.is_none() && off.remove.is_some());
        refit(&catalog, &registry, &podded, &off, None, 4_000).expect("the pod comes off");
    }

    /// The transition keeps what it does not touch: damage on the hull's
    /// armour survives a refit of its turret weapon.
    #[test]
    fn ud4_a_refit_keeps_the_hulls_damage() {
        let (catalog, registry) = fielded();
        let tank = hull(&catalog, "tacsim.unit.line_tank");
        let (armour, cell) = tank
            .cells()
            .find(|(k, _)| matches!(k, mush_tactics::CellAddr::Facet { .. }))
            .map(|(k, c)| (k.clone(), *c))
            .expect("a facet");
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::SetCell {
                addr: armour.clone(),
                value: cell.min,
            }],
            None,
            None,
        )
        .expect("a plan");
        let (damaged, _) = mush_tactics::apply(&tank, &plan, 0).expect("damaged");
        let options = family(&catalog, &registry, &damaged);
        let laser = option(&options, "tacsim.unit.line_tank_laser").expect("the laser variant");
        let (lasered, _) = refit(&catalog, &registry, &damaged, laser, None, 0).expect("refitted");
        assert_eq!(lasered.cell(&armour).map(|c| c.current), Some(cell.min));
    }
}

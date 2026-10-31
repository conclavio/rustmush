//! **The refit bay's TacSim content** (doc 31 cut UD.4b): the bay recipe,
//! the part catalogue over the shipped units and the bay refit's
//! composition, read from the launch deployment's bay coordinates.
//!
//! The bay refits a hull within the fielded units (never the governed or
//! twin core roots, which only the core refit fits), one part out and one in
//! on one mount ([`crate::refit`]), with the heat pair as the derivable
//! section cells. A part is stocked as `tacsim.part.<component>` — the
//! component's id within its unit, so a cannon taken off one variant fits
//! another — and a core is no part at all: the catalogue names none, so no
//! bay offer can move a core.
use super::RefitBay;
use crate::definitions::unit::INTACT;
use mush_core::{
    CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, ModError,
};
use mush_economy::bay_refit::{BayPaymentPolicy, BaySite};
use mush_industry::bay_refit::{BAY_CAPABILITY, BayPartCatalog, BayRefitConfig};
use mush_inventory::{EndpointRef, Grade};
use mush_tactics::unit::replacement::ComponentState;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
/// The bay recipe's id; the kernel pins it.
pub const RECIPE: &str = "bay_refit";
/// The grade an intact part goes back to stores at, and a damaged one.
const INTACT_GRADE: u8 = 9;
const WORN_GRADE: u8 = 3;

/// The bay recipe: the work a refit takes and the facility word it runs at.
pub(crate) fn recipe_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("recipe").map_err(bad)?,
        DefinitionId::new(RECIPE).map_err(bad)?,
        1,
        serde_json::json!({"format":1,"label":"Bay component refit","work_units":super::BAY_WORK_UNITS,"facility_capability":BAY_CAPABILITY}),
    )
    .map_err(bad)
}

/// **TacSim's part catalogue**: a component's part is its id within its unit;
/// a core is no part.
pub(crate) struct ShippedParts;
impl BayPartCatalog for ShippedParts {
    fn commodity(&self, component: &DefinitionRef) -> Option<CanonicalId> {
        let id = component.key().id.as_str();
        let part = id.rsplit('.').next()?;
        if component.key().owner.as_str() != crate::NS || part == "core" || part == id {
            return None;
        }
        CanonicalId::new(format!("tacsim.part.{part}")).ok()
    }
    fn grade(&self, removed: &ComponentState) -> Option<Grade> {
        Grade::new(if removed.installed().condition.as_str() == INTACT {
            INTACT_GRADE
        } else {
            WORN_GRADE
        })
        .ok()
    }
}

/// The bay refit for `site`, refitting within `fielded`.
pub(crate) fn config(
    site: &RefitBay,
    issuer: mush_org::SystemPrincipalId,
    instrument: mush_economy::InstrumentId,
    recipe: DefinitionRef,
    fielded: BTreeSet<DefinitionRef>,
    economy_owner: DefinitionRef,
) -> Result<BayRefitConfig, ModError> {
    let payment = BayPaymentPolicy {
        issuer,
        instrument,
        recipe,
        bays: vec![BaySite {
            facility: site.facility,
            provider: site.provider.clone(),
        }],
        max_fee_minor: site.max_fee_minor.unwrap_or(super::BAY_MAX_FEE_MINOR),
    };
    let derived = crate::refit::DERIVED_CELLS
        .iter()
        .map(|c| mush_tactics::CellId::new(c).map_err(bad))
        .collect::<Result<Vec<_>, _>>()?;
    BayRefitConfig::new(
        payment,
        super::BAY_WORK_UNITS,
        BTreeMap::from([(site.facility, EndpointRef(site.stores))]),
        fielded,
        derived,
        Arc::new(ShippedParts),
        economy_owner,
    )
    .map_err(bad)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{DefinitionRegistryBuilder, OwnerId};

    fn component(owner: &str, id: &str) -> DefinitionRef {
        DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(owner).expect("an owner"),
                DefinitionDescriptor::new(
                    DefinitionKind::new("component").expect("a kind"),
                    DefinitionId::new(id).expect("an id"),
                    1,
                    serde_json::json!({"fixture": id}),
                )
                .expect("a descriptor"),
            )
            .expect("registers")
    }

    /// A part is its component's id within its unit, so the cannon off one
    /// variant is the cannon of another; a core is no part.
    #[test]
    fn ud4b_a_part_is_its_mount_component_and_a_core_is_none() {
        let parts = ShippedParts;
        let of = |id| {
            parts
                .commodity(&component(crate::NS, id))
                .map(|c| c.to_string())
        };
        assert_eq!(
            of("tacsim.unit.line_tank.cannon").as_deref(),
            Some("tacsim.part.cannon")
        );
        assert_eq!(
            of("tacsim.unit.line_tank_laser.cannon"),
            of("tacsim.unit.line_tank.cannon")
        );
        assert_eq!(of("tacsim.unit.line_tank.core"), None);
        assert_eq!(parts.commodity(&component("other", "x.cannon")), None);
    }
}

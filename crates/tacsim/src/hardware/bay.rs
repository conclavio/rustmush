//! **The refit bay's TacSim content** (doc 31 cut UD.4b): the bay recipe,
//! the part catalogue over the shipped units and the bay refit's
//! composition, read from the launch deployment's bay coordinates.
//!
//! The bay refits a hull within the fielded units (never the governed or
//! twin core roots, which only the core refit fits), one part out and one in
//! on one mount ([`crate::refit`]), with the heat pair as the derivable
//! section cells. A part is stocked as `tacsim.part.<kind>.<row>` — the
//! hardware row its slot names (doc 39 cut IN.1, Q-39-3, Q-39-35), so a cannon
//! taken off a line tank fits a heavy — and a core is no part at all: the
//! catalogue names none, so no bay offer can move a core.
use super::RefitBay;
use crate::definitions::PartRow;
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

/// **TacSim's part catalogue** (doc 39 cut IN.1, Q-39-3): a component's part is the hardware
/// row its slot names, from the catalogue's part index; a core is no part.
pub(crate) struct ShippedParts {
    parts: Arc<BTreeMap<CanonicalId, PartRow>>,
}
impl ShippedParts {
    /// Over `parts`, the catalogue's index ([`crate::definitions::Catalog::parts`]).
    pub(crate) fn new(parts: BTreeMap<CanonicalId, PartRow>) -> Self {
        Self {
            parts: Arc::new(parts),
        }
    }
}
impl BayPartCatalog for ShippedParts {
    fn commodity(&self, component: &DefinitionRef) -> Option<CanonicalId> {
        part_row(&self.parts, component).map(|row| row.commodity.clone())
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

/// **The part `component` is in `parts`**: TacSim's own components only.
fn part_row<'a>(
    parts: &'a BTreeMap<CanonicalId, PartRow>,
    component: &DefinitionRef,
) -> Option<&'a PartRow> {
    if component.key().owner.as_str() != crate::NS {
        return None;
    }
    parts.get(&CanonicalId::new(component.key().id.as_str()).ok()?)
}

/// **The part a component stocks as** (doc 36 cut GC.8b; doc 39 cut IN.1): the catalogue's own
/// answer, so a part the bay strips or scraps is the same commodity a refit takes out, and the
/// hardware row it fits.
pub(crate) fn part_of(w: &mush_core::World, component: &DefinitionRef) -> Option<PartRow> {
    let store = w.module_state::<crate::store::TacsimStore>()?;
    part_row(&store.definitions().parts, component).cloned()
}

/// The grade a part goes to stores at: intact, or worn (damaged or jammed).
pub(crate) const fn part_grade(intact: bool) -> u8 {
    if intact { INTACT_GRADE } else { WORN_GRADE }
}

/// The bay refit for `site`, refitting within `fielded`.
pub(crate) fn config(
    site: &RefitBay,
    issuer: mush_org::SystemPrincipalId,
    instrument: mush_economy::InstrumentId,
    recipe: DefinitionRef,
    fielded: BTreeSet<DefinitionRef>,
    parts: BTreeMap<CanonicalId, PartRow>,
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
        // Doc 36 cut GC.7 (Q-36-31): the bay quotes from the shipped workshop tariff.
        crate::definitions::workshop::workshop_tariff().map_err(bad)?,
        BTreeMap::from([(site.facility, EndpointRef(site.stores))]),
        fielded,
        derived,
        Arc::new(ShippedParts::new(parts)),
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

    /// **Doc 39 cut IN.1** (Q-39-3): a part is the hardware row its slot names, so the cannon
    /// off a line tank is the heavy's; a core is no part, nor another owner's component.
    #[test]
    fn in1_a_part_is_its_slots_hardware_row_and_a_core_is_none() {
        let catalog = crate::definitions::Catalog::build_fielded(
            &crate::TacsimRuleset::default()
                .loaded_packs()
                .expect("the shipped packs load"),
            crate::NS,
        )
        .expect("the shipped catalogue builds");
        let parts = ShippedParts::new(catalog.parts.clone());
        let of = |id| {
            parts
                .commodity(&component(crate::NS, id))
                .map(|c| c.to_string())
        };
        assert_eq!(
            of("tacsim.unit.line_tank.cannon").as_deref(),
            Some("tacsim.part.weapon.cannon_105")
        );
        assert_eq!(
            of("tacsim.unit.heavy_mk1.cannon"),
            of("tacsim.unit.line_tank.cannon")
        );
        assert_eq!(
            of("tacsim.unit.dray.power").as_deref(),
            Some("tacsim.part.plant.hauler_mk1")
        );
        assert_eq!(
            of("tacsim.unit.tv_escort.power").as_deref(),
            Some("tacsim.part.plant.vehicle_mk1")
        );
        assert_eq!(
            of("tacsim.unit.line_tank_ecm.ecm_pod").as_deref(),
            Some("tacsim.part.sensor.ecm_mk1")
        );
        assert_eq!(of("tacsim.unit.line_tank.core"), None);
        assert_eq!(of("tacsim.unit.corvette.power"), None, "an unpriced plant");
        assert_eq!(parts.commodity(&component("other", "x.cannon")), None);
    }
}

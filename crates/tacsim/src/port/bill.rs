//! **The design's bill** (doc 39 cut IN.9, §4.7; Q-39-14): what the yard takes in to build one
//! hull, derived from the design at the catalogue build, never authored beside it.
//!
//! The partition and the bulk lines are IN.8's (`crate::definitions::mass::MassBudget`, §4.8):
//! the alloys and fittings each the hull's share plus its share of the 30 ‰ loss, rounded up, and
//! the days one per 20 t, at least two. The bill adds one part a slot and the core's class. A
//! design the yard does not partition (its parts and core leave no remainder, or its class is not
//! built here) has no bill. The figures are provisional (the owner's economy rule of 2026-10-24).
//!
//! The hull recipe pack (`content/industry/hulls-v1.json`) carries each buildable design's bulk
//! lines and work units; its test pins every row to `design_bill`, so the pack can never drift
//! from the design. The parts are the order's own lines, delivered as lots.

use std::collections::BTreeMap;

use mush_core::CanonicalId;

use crate::definitions::mass::{KG_PER_T, MassBudget};
use crate::definitions::unit::ComponentRole;
use crate::definitions::{Catalog, GoodsTable};

/// The work units a day (the industry clock's slot-hours).
pub const WORK_UNITS_A_DAY: u64 = 24;

/// **One design's bill** (§4.7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesignBill {
    /// One part good a slot, in component order.
    pub parts: Vec<CanonicalId>,
    /// The core definition, if the design carries one.
    pub core: Option<CanonicalId>,
    /// Structural alloys, tonnes.
    pub alloys_t: u64,
    /// Industrial parts (fittings), tonnes.
    pub fittings_t: u64,
    /// The yard days.
    pub days: u64,
    /// IN.8's partition it was cut from.
    pub budget: MassBudget,
}

impl DesignBill {
    /// The work units the hull recipe runs.
    #[must_use]
    pub const fn work_units(&self) -> u64 {
        self.days.saturating_mul(WORK_UNITS_A_DAY)
    }

    /// The part lines: each part good and how many the design takes.
    #[must_use]
    pub fn part_lines(&self) -> BTreeMap<CanonicalId, u64> {
        let mut lines = BTreeMap::new();
        for part in &self.parts {
            *lines.entry(part.clone()).or_insert(0) += 1;
        }
        lines
    }

    /// The waste the bill declares: inputs less the hull (the loss and the rounding), kg.
    #[must_use]
    pub fn waste_kg(&self) -> u64 {
        let inputs = self
            .alloys_t
            .saturating_add(self.fittings_t)
            .saturating_mul(KG_PER_T)
            .saturating_add(self.budget.parts_kg)
            .saturating_add(self.budget.core_kg);
        inputs.saturating_sub(self.budget.dry_mass_kg)
    }
}

/// The part good a design's slot stocks as (the catalogue's part index, `{unit}.{slot}`).
fn part_good(catalog: &Catalog, design: &CanonicalId, slot: &CanonicalId) -> Option<CanonicalId> {
    let key = CanonicalId::new(format!("{design}.{slot}")).ok()?;
    catalog.parts.get(&key).map(|row| row.commodity.clone())
}

/// **The bill of `design`** (§4.7), from its mass budget.
///
/// # Errors
///
/// A design the catalogue does not hold, or one the yard does not partition.
pub fn design_bill(
    catalog: &Catalog,
    _goods: &GoodsTable,
    design: &CanonicalId,
) -> Result<DesignBill, String> {
    let spec = catalog
        .specs
        .get(design)
        .ok_or_else(|| format!("{design} is not in the catalogue"))?;
    let budget = catalog
        .budget_of(design.as_str())
        .cloned()
        .ok_or_else(|| format!("{design} has no mass budget, so the yard cannot bill it"))?;
    let (alloys_t, fittings_t) = budget.bill_tonnes();
    let mut parts = Vec::new();
    let mut core = None;
    for component in &spec.components {
        match &component.role {
            ComponentRole::Core { core: id } => core = Some(id.clone()),
            role if role.part_row().is_some() => {
                if let Some(good) = part_good(catalog, design, &component.id) {
                    parts.push(good);
                }
            }
            _ => {}
        }
    }
    Ok(DesignBill {
        parts,
        core,
        alloys_t,
        fittings_t,
        days: budget.build_days(),
        budget,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bill(design: &str) -> DesignBill {
        let catalog = Catalog::build_fielded(&[], crate::NS).expect("the fielded content");
        let goods = crate::definitions::goods::goods_table().expect("the goods table");
        design_bill(&catalog, &goods, &CanonicalId::new(design).expect("an id")).expect("a bill")
    }

    #[test]
    fn in9_the_escort_and_the_dray_p_bills_come_from_their_designs() {
        let escort = bill("tacsim.unit.tv_escort");
        assert_eq!(
            (escort.alloys_t, escort.fittings_t, escort.days),
            (3, 1, 2),
            "an 8 t hull: 2,565 kg of structure and 855 of fittings, the two-day floor"
        );
        assert_eq!(
            escort.parts.len(),
            4,
            "a cannon, a coaxial gun, a sight and a plant"
        );
        let dray = bill("tacsim.unit.dray_p");
        assert_eq!(
            (dray.alloys_t, dray.fittings_t, dray.days),
            (163, 55, 12),
            "the Dray-P: 157,500 and 52,500 kg and the loss, twelve days"
        );
        // The shipped hull pack is the bills, row for row: bulk lines, kit, days.
        let pack: serde_json::Value =
            serde_json::from_str(crate::definitions::content::INDUSTRY_HULLS_JSON)
                .expect("the hull pack parses");
        let entries = pack["entries"].as_object().expect("entries");
        assert!(!entries.is_empty());
        for (id, entry) in entries {
            let design = format!("tacsim.unit.{id}");
            let b = bill(&design);
            let def = &entry["def"];
            let units = |good: &str| {
                def["inputs"]
                    .as_array()
                    .and_then(|lines| lines.iter().find(|l| l["commodity"] == good))
                    .and_then(|l| l["units"].as_u64())
            };
            assert_eq!(
                units("structural_alloys"),
                Some(b.alloys_t),
                "{id}'s alloys"
            );
            assert_eq!(
                units("industrial_parts"),
                Some(b.fittings_t),
                "{id}'s fittings"
            );
            assert_eq!(
                def["work_units"].as_u64(),
                Some(b.work_units()),
                "{id}'s days"
            );
            assert_eq!(
                def["outputs"][0]["commodity"].as_str(),
                Some(format!("tacsim.hull_kit.{id}").as_str()),
                "{id}'s kit"
            );
            assert_eq!(def["requires"][0].as_str(), Some("tacsim.yard"));
        }
    }
}

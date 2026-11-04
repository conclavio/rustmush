//! **The design's bill** (doc 39 cut IN.9, §4.7 and §4.8; Q-39-14): what the yard takes in to
//! build one hull, derived from the design at the catalogue build, never authored beside it.
//!
//! The partition (§4.8): `dry_mass = parts + core + structure + fittings`, the parts one
//! catalogue part a slot, the core by its class (150 / 250 / 500 kg for I / II / III), the
//! remainder split by the class's `structure_share`, and the 30 ‰ assembly loss **on top** of
//! the hull, split the same way. The bill (§4.7): one part a slot, the core's class, `alloys_t =
//! ceil((structure + loss × share) / 1000)`, `fittings_t = ceil((fittings + loss × (1 − share)) /
//! 1000)`, `days = max(2, ceil(dry_mass_t / 20))`. A design whose parts and core leave no
//! remainder is refused. The figures are provisional (the owner's economy rule of 2026-10-24).
//!
//! The hull recipe pack (`content/industry/hulls-v1.json`) carries each buildable design's bulk
//! lines and work units; its test pins every row to `design_bill`, so the pack can never drift
//! from the design. The parts are the order's own lines, delivered as lots.

use std::collections::BTreeMap;

use mush_core::CanonicalId;

use crate::definitions::unit::ComponentRole;
use crate::definitions::{Catalog, GoodsTable, UnitClass, UnitSpec};

/// The assembly loss, per mille of the dry mass (§4.8).
pub const LOSS_PERMILLE: u64 = 30;
/// The fewest days a hull takes (§4.7).
pub const MIN_DAYS: u64 = 2;
/// Tonnes of dry mass a yard day builds (§4.7).
pub const TONNES_A_DAY: u64 = 20;
/// The work units a day (the industry clock's slot-hours).
pub const WORK_UNITS_A_DAY: u64 = 24;

/// **`structure_share` by class**, per mille (§4.8; provisional): armour and vehicle 750, walker
/// 700, hauler 750, drone 650. Capital and station hulls are not partitioned (not built here).
#[must_use]
pub const fn structure_share_permille(class: UnitClass) -> Option<u64> {
    match class {
        UnitClass::Armour | UnitClass::Vehicle | UnitClass::Hauler => Some(750),
        UnitClass::Walker => Some(700),
        UnitClass::Drone => Some(650),
        _ => None,
    }
}

/// The core's mass by its class, kg (§4.8; provisional).
#[must_use]
pub const fn core_kg(class: crate::arena::core::CoreClass) -> u64 {
    use crate::arena::core::CoreClass;
    match class {
        CoreClass::Zero => 0,
        CoreClass::I => 150,
        CoreClass::II => 250,
        CoreClass::III => 500,
    }
}

/// **One design's partition** (§4.8), kilograms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Partition {
    /// The finished hull.
    pub dry_mass_kg: u64,
    /// One catalogue part a slot.
    pub parts_kg: u64,
    /// The core.
    pub core_kg: u64,
    /// The structure: the remainder's share.
    pub structure_kg: u64,
    /// The fittings: the rest of the remainder.
    pub fittings_kg: u64,
    /// The assembly loss on top.
    pub loss_kg: u64,
    /// The class's structure share, per mille.
    pub share_permille: u64,
}

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
    /// The partition it was cut from.
    pub partition: Partition,
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
            .saturating_mul(1_000)
            .saturating_add(self.partition.parts_kg)
            .saturating_add(self.partition.core_kg);
        inputs.saturating_sub(self.partition.dry_mass_kg)
    }
}

fn div_ceil(numerator: u64, denominator: u64) -> u64 {
    numerator.div_ceil(denominator.max(1))
}

/// **The partition of `spec`** (§4.8).
///
/// # Errors
///
/// A sentence for a design with no dry mass, a class that is not built here, a slot's part
/// with no mass, or parts and a core that leave no remainder.
pub fn partition(
    catalog: &Catalog,
    goods: &GoodsTable,
    design: &CanonicalId,
    spec: &UnitSpec,
) -> Result<Partition, String> {
    let dry_mass_kg = spec
        .logistics
        .as_ref()
        .map(|logistics| u64::from(logistics.dry_mass_t).saturating_mul(1_000))
        .ok_or_else(|| format!("{design} carries no logistics, so no dry mass"))?;
    let class = spec
        .class
        .ok_or_else(|| format!("{design} declares no class"))?;
    let share_permille = structure_share_permille(class)
        .ok_or_else(|| format!("{design}'s class is not built at a yard"))?;
    let mut parts_kg = 0u64;
    let mut core = 0u64;
    for component in &spec.components {
        match &component.role {
            ComponentRole::Core { core: id } => {
                let def = catalog
                    .cores
                    .get(id)
                    .ok_or_else(|| format!("{design}: core {id} is not in the catalogue"))?;
                core = core.saturating_add(core_kg(def.class));
            }
            role if role.part_row().is_some() => {
                let good = part_good(catalog, design, &component.id)
                    .ok_or_else(|| format!("{design}: slot {} has no part", component.id))?;
                let kg = goods
                    .good(&good)
                    .and_then(crate::definitions::GoodDef::mass_kg_per_unit)
                    .ok_or_else(|| format!("{design}: part {good} has no mass"))?;
                parts_kg = parts_kg.saturating_add(kg);
            }
            _ => {}
        }
    }
    let remainder = dry_mass_kg
        .checked_sub(parts_kg.saturating_add(core))
        .filter(|kg| *kg > 0)
        .ok_or_else(|| format!("{design}: its parts and core leave no remainder"))?;
    let structure_kg = remainder.saturating_mul(share_permille) / 1_000;
    Ok(Partition {
        dry_mass_kg,
        parts_kg,
        core_kg: core,
        structure_kg,
        fittings_kg: remainder.saturating_sub(structure_kg),
        loss_kg: dry_mass_kg.saturating_mul(LOSS_PERMILLE) / 1_000,
        share_permille,
    })
}

/// The part good a design's slot stocks as (the catalogue's part index, `{unit}.{slot}`).
fn part_good(catalog: &Catalog, design: &CanonicalId, slot: &CanonicalId) -> Option<CanonicalId> {
    let key = CanonicalId::new(format!("{design}.{slot}")).ok()?;
    catalog.parts.get(&key).map(|row| row.commodity.clone())
}

/// **The bill of `design`** (§4.7).
///
/// # Errors
///
/// [`partition`]'s sentences, or a design the catalogue does not hold.
pub fn design_bill(
    catalog: &Catalog,
    goods: &GoodsTable,
    design: &CanonicalId,
) -> Result<DesignBill, String> {
    let spec = catalog
        .specs
        .get(design)
        .ok_or_else(|| format!("{design} is not in the catalogue"))?;
    let partition = partition(catalog, goods, design, spec)?;
    let share = partition.share_permille;
    // In grams-per-mille: kg × 1000 for the structure, the loss's share added at per mille.
    let alloys_t = div_ceil(
        partition
            .structure_kg
            .saturating_mul(1_000)
            .saturating_add(partition.loss_kg.saturating_mul(share)),
        1_000_000,
    );
    let fittings_t = div_ceil(
        partition
            .fittings_kg
            .saturating_mul(1_000)
            .saturating_add(partition.loss_kg.saturating_mul(1_000 - share)),
        1_000_000,
    );
    let days = div_ceil(partition.dry_mass_kg, TONNES_A_DAY.saturating_mul(1_000)).max(MIN_DAYS);
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
        days,
        partition,
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
        for design in [
            "tacsim.unit.tv_escort",
            "tacsim.unit.tv_prospector",
            "tacsim.unit.tv_tender",
            "tacsim.unit.dray",
            "tacsim.unit.dray_p",
        ] {
            let b = bill(design);
            eprintln!(
                "{design}: parts {:?} core {:?} alloys {} t fittings {} t days {} waste {} kg {:?}",
                b.parts,
                b.core,
                b.alloys_t,
                b.fittings_t,
                b.days,
                b.waste_kg(),
                b.partition
            );
        }
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
        let escort = bill("tacsim.unit.tv_escort");
        assert_eq!(escort.days, 2, "an 8 t hull is the two-day floor");
        assert_eq!(
            escort.parts.len(),
            4,
            "a cannon, a coaxial gun, a sight and a plant"
        );
    }
}

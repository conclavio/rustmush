//! **The design's bill** (doc 39 cut IN.9, §4.7; Q-39-14): what the yard takes in to build one
//! hull, derived from the design at the catalogue build, never authored beside it.
//!
//! The partition and the bulk lines are IN.8's (`crate::definitions::mass::MassBudget`, §4.8):
//! the alloys and fittings each the hull's share plus its share of the 30 ‰ loss, rounded up, and
//! the days one per 20 t, at least two. The bill adds one part a slot and, for a design that
//! carries a stock core, **one core module** (doc 48 unit PL.7, Q-48-48): the part good its core
//! stocks as, of the core's mass, delivered and consumed at the commission like any part. The
//! hull's core stays the governed core its design carries, outside the monthly die quota: no die
//! is drawn and no core title is born (the owner's call). A design the yard does not partition
//! (its parts and core leave no remainder, or its class is not built here) has no bill. The
//! figures are provisional (the owner's economy rule of 2026-10-24).
//!
//! The hull recipe pack (`content/industry/hulls-v2.json`) carries each buildable design's bulk
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
    /// **The core module** (doc 48 unit PL.7): the part good the stock core stocks as, billed
    /// once beside the parts ([`core_module_of`]); `Some` exactly when [`Self::core`] is.
    pub core_module: Option<CanonicalId>,
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

    /// The part lines: each part good and how many the design takes, the core module among them
    /// (doc 48 unit PL.7). Each is delivered as lots and consumed at the commission.
    #[must_use]
    pub fn part_lines(&self) -> BTreeMap<CanonicalId, u64> {
        let mut lines = BTreeMap::new();
        for part in self.parts.iter().chain(self.core_module.as_ref()) {
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

/// The prefix of a core definition: `tacsim.core.<name>`.
pub const CORE_PREFIX: &str = "tacsim.core.";
/// The prefix of a core module good: `tacsim.part.core.<name>` (doc 48 unit PL.7).
pub const CORE_MODULE_PREFIX: &str = "tacsim.part.core.";

/// **The core module good of `core`** (doc 48 unit PL.7, Q-48-48): the core `tacsim.core.<name>`
/// stocks as the part good `tacsim.part.core.<name>`, as a slot's part `tacsim.<rest>` stocks as
/// `tacsim.part.<rest>`. `None` for an id outside the core prefix.
#[must_use]
pub fn core_module_of(core: &CanonicalId) -> Option<CanonicalId> {
    let name = core.as_str().strip_prefix(CORE_PREFIX)?;
    CanonicalId::new(format!("{CORE_MODULE_PREFIX}{name}")).ok()
}

/// **The core a core module good stocks** (doc 48 unit PL.7): `tacsim.part.core.<name>` is the
/// core `tacsim.core.<name>`, by its definition's exact reference, when the catalogue holds it.
#[must_use]
pub fn core_module_row(catalog: &Catalog, good: &CanonicalId) -> Option<mush_core::DefinitionRef> {
    let name = good.as_str().strip_prefix(CORE_MODULE_PREFIX)?;
    let core = CanonicalId::new(format!("{CORE_PREFIX}{name}")).ok()?;
    catalog.cores.contains_key(&core).then_some(())?;
    catalog.references.get(&core).cloned()
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
/// A design the catalogue does not hold, one the yard does not partition, or one whose stock core
/// has no core module good in `goods` (doc 48 unit PL.7: a stock core is never free).
pub fn design_bill(
    catalog: &Catalog,
    goods: &GoodsTable,
    design: &CanonicalId,
) -> Result<DesignBill, String> {
    let spec = catalog
        .specs
        .get(design)
        .ok_or_else(|| format!("{design} is not in the catalog"))?;
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
    let core_module = match &core {
        Some(core) => Some(
            core_module_of(core)
                .filter(|module| goods.good(module).is_some())
                .ok_or_else(|| {
                    format!("{design}'s core {core} has no module good, so the yard cannot bill it")
                })?,
        ),
        None => None,
    };
    Ok(DesignBill {
        parts,
        core,
        core_module,
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
        // Doc 41 cut RS.6a (Q-41-15): Tycho's two designs are billed like any other.
        let crawler = bill("tacsim.unit.tycho_crawler");
        assert_eq!(
            (crawler.alloys_t, crawler.fittings_t, crawler.days),
            (100, 34, 7),
            "the 130 t crawler: a week in the yard"
        );
        assert_eq!(crawler.parts.len(), 2, "a plant and a sight");
        let lifter = bill("tacsim.unit.tycho_lifter");
        assert_eq!(
            (lifter.alloys_t, lifter.fittings_t, lifter.days),
            (441, 147, 30),
            "the 600 t lifter: a month in the yard"
        );
        // Doc 47 unit CO.10b: the wrecker is built as the tender is, from its 12 t design.
        let wrecker = bill(crate::definitions::content::TV_WRECKER);
        assert_eq!(
            (wrecker.alloys_t, wrecker.fittings_t, wrecker.days),
            (9, 3, 2),
            "a 12 t rig: two yard days"
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
        // Doc 48 unit PL.7: the bill's lines fit an order's, and its lots a commission's.
        for id in entries.keys() {
            let b = bill(&format!("tacsim.unit.{id}"));
            assert!(
                b.part_lines().len() + 2 <= crate::port::works::MAX_BILL_LINES,
                "{id}'s bill fits an order"
            );
            assert!(
                b.parts.len() + usize::from(b.core_module.is_some())
                    <= crate::port::commission::MAX_COMMISSION_PARTS,
                "{id}'s lots fit a commission"
            );
        }
        // CO.10b: the wrecker has its row and its kit good, as the tender has.
        assert!(entries.contains_key("tv_wrecker"), "the wrecker's hull row");
        let goods = crate::definitions::goods::goods_table().expect("the goods table");
        let kit = CanonicalId::new("tacsim.hull_kit.tv_wrecker").expect("an id");
        assert!(
            goods.goods.iter().any(|g| g.id == kit),
            "the wrecker's hull kit is a good"
        );
    }

    /// **Doc 48 unit PL.7: a stock core bills its core module** (Q-48-48). The escort and every
    /// line tank bill the line core's module (`tacsim.part.core.line`) once beside their parts;
    /// the scout bills the doctrine core's and the heavy the command core's. A coreless design
    /// (the Dray-P, the prospector) bills none. Each module good is a part of its core's mass and
    /// price, so the bill's matter and the build's reference still balance.
    #[test]
    fn pl7_a_stock_cored_design_bills_its_core_module() {
        let goods = crate::definitions::goods::goods_table().expect("the goods table");
        let line = CanonicalId::new("tacsim.part.core.line").expect("an id");
        for design in [
            "tacsim.unit.tv_escort",
            "tacsim.unit.line_tank",
            "tacsim.unit.line_tank_ecm",
            "tacsim.unit.line_tank_eccm",
            "tacsim.unit.line_tank_laser",
            "tacsim.unit.line_tank_thermal",
            "tacsim.unit.biped_mk1",
            "tacsim.unit.biped_mk2",
        ] {
            let b = bill(design);
            assert_eq!(b.core_module.as_ref(), Some(&line), "{design}'s module");
            assert_eq!(
                b.part_lines().get(&line),
                Some(&1),
                "{design} bills it once"
            );
            let lots: u64 = b.part_lines().values().sum();
            assert_eq!(lots, b.parts.len() as u64 + 1, "{design}'s lots");
        }
        for (design, module) in [
            ("tacsim.unit.scout_mk1", "tacsim.part.core.doctrine"),
            ("tacsim.unit.heavy_mk1", "tacsim.part.core.command"),
        ] {
            assert_eq!(
                bill(design).core_module.map(|m| m.as_str().to_owned()),
                Some(module.to_owned()),
                "{design}'s module"
            );
        }
        for design in ["tacsim.unit.dray_p", "tacsim.unit.tv_prospector"] {
            let b = bill(design);
            assert_eq!((b.core.as_ref(), b.core_module.as_ref()), (None, None));
        }
        let catalog = Catalog::build_fielded(&[], crate::NS).expect("the fielded content");
        for (core, class) in [
            ("tacsim.core.doctrine", crate::arena::core::CoreClass::I),
            ("tacsim.core.line", crate::arena::core::CoreClass::II),
            ("tacsim.core.command", crate::arena::core::CoreClass::III),
        ] {
            let core = CanonicalId::new(core).expect("an id");
            assert_eq!(catalog.cores.get(&core).map(|def| def.class), Some(class));
            let module = core_module_of(&core).expect("a module id");
            let good = goods.good(&module).expect("the module is a good");
            assert!(crate::port_tick::is_part(&module), "{module} is a part");
            assert_eq!(
                good.mass_kg_per_unit(),
                Some(crate::definitions::mass::core_kg(class)),
                "{module} weighs its core"
            );
            assert_eq!(
                good.reference_major.saturating_mul(100),
                crate::definitions::mass::core_minor(class),
                "{module} is priced at its core"
            );
        }
        // A goods table without the module refuses the bill: a stock core is never free.
        let mut bare = goods.clone();
        bare.goods.retain(|g| g.id != line);
        assert!(
            design_bill(
                &catalog,
                &bare,
                &CanonicalId::new("tacsim.unit.tv_escort").expect("an id")
            )
            .is_err()
        );
    }
}

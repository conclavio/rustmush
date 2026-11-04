//! **The mass budget** (doc 39 cut IN.8; §4.8, Q-39-15, Q-39-26): each design's one complete
//! partition, computed at the catalogue build.
//!
//! ```text
//! dry_mass_kg  = parts_kg + core_kg + structure_kg + fittings_kg
//! remainder_kg = dry_mass_kg − parts_kg − core_kg            (no budget when ≤ 0)
//! structure_kg = remainder_kg × structure_share(class)
//! fittings_kg  = remainder_kg − structure_kg
//! ```
//!
//! The per-point figures divide the structure and the fittings over the design's **structural
//! points** — every facet's armour and every section's internals, never a heat cell — so an
//! escort's armour patch of 100 points draws 86 kg of alloys and 29 kg of fittings and a Dray's
//! draws 8.65 t and 2.88 t. The scrap yield is the structure's kilograms at
//! [`SCRAP_RECOVERY_PERMILLE`]. The bill (§4.7) adds the assembly loss on top of the hull, rounds
//! each bulk line up to whole tonnes and prices the build; IN.8 reads it only as the reference a
//! scrap's fifth bounds (§4.6) until IN.9 builds hulls from it. Every figure here is provisional
//! under the owner's economy rule of 2026-10-24.
//!
//! These figures replace the three constants doc 39 §4.8 retires: the workshop's 100 kg of scrap
//! a structural point and 1,000 kg a section, and the tariff's 100 points to a tonne of fittings.

use std::collections::BTreeMap;

use mush_core::CanonicalId;

use crate::arena::core::CoreClass;
use crate::definitions::{CoreDef, DefinitionError, GoodsTable, UnitClass, UnitSpec};

/// Kilograms a tonne.
pub const KG_PER_T: u64 = 1_000;

/// **The assembly loss** (§4.8), per mille of the dry mass: waste on top of the hull, split by
/// the structure share.
pub const LOSS_PERMILLE: u64 = 30;

/// **What a scrap recovers of the structure** (§4.6's `r`), per mille.
pub const SCRAP_RECOVERY_PERMILLE: u64 = 600;

/// **A day of the yard's build, work units** ([`mush_industry::cost::WORK_UNITS_PER_DAY`]).
const WORK_UNITS_PER_DAY: u64 = mush_industry::cost::WORK_UNITS_PER_DAY;

/// **The yard's work unit, minor units** (§4.4's yard row: 68.75 cr), until IN.9 prices the
/// build from the yard facility's own cost row.
pub const YARD_WORK_UNIT_MINOR: u64 = 6_875;

/// **The yard's margin on the build**, per mille (§4.4: 25 %).
pub const YARD_MARGIN_PERMILLE: u64 = 250;

/// The core's kilograms by class (§4.8: 150 / 250 / 500; none for the drive ROM).
#[must_use]
pub const fn core_kg(class: CoreClass) -> u64 {
    match class {
        CoreClass::Zero => 0,
        CoreClass::I => 150,
        CoreClass::II => 250,
        CoreClass::III => 500,
    }
}

/// The core's reference, minor units: the line tank's basis at 3 / 8 / 25 %
/// ([`mush_economy::core_integration::CoreClass::finished_minor`]).
#[must_use]
pub fn core_minor(class: CoreClass) -> u64 {
    use mush_economy::core_integration::CoreClass as Made;
    let made = match class {
        CoreClass::Zero => return 0,
        CoreClass::I => Made::I,
        CoreClass::II => Made::II,
        CoreClass::III => Made::III,
    };
    u64::try_from(made.finished_minor()).unwrap_or(0)
}

/// **The structure's share of the remainder**, per mille, by class (§4.8); `None` for a class
/// the yard does not partition (a capital hull and a station, Q-39-20).
#[must_use]
pub const fn structure_share_permille(class: UnitClass) -> Option<u64> {
    match class {
        UnitClass::Armour | UnitClass::Vehicle | UnitClass::Hauler => Some(750),
        UnitClass::Walker => Some(700),
        UnitClass::Drone => Some(650),
        UnitClass::Station | UnitClass::Capital => None,
    }
}

/// **One design's partition** (§4.8), kilograms throughout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MassBudget {
    /// The finished hull.
    pub dry_mass_kg: u64,
    /// One part a slot, at the catalogue's masses.
    pub parts_kg: u64,
    /// The core.
    pub core_kg: u64,
    /// The structure: `structural_alloys`.
    pub structure_kg: u64,
    /// The fittings: `industrial_parts`.
    pub fittings_kg: u64,
    /// The structural points the structure and fittings spread over (armour and internals).
    pub points: u64,
    /// The structure share, per mille.
    pub share_permille: u64,
    /// The parts' references, minor units: one part a slot.
    pub parts_minor: u64,
    /// The core's reference, minor units.
    pub core_minor: u64,
}

impl MassBudget {
    /// **The alloys `points` of repair draw**, kilograms, rounded up: a repair never draws less
    /// matter than the points hold.
    #[must_use]
    pub fn structure_kg_for(&self, points: u64) -> u64 {
        share_up(points, self.structure_kg, self.points)
    }

    /// **The fittings `points` of repair draw**, kilograms, rounded up.
    #[must_use]
    pub fn fittings_kg_for(&self, points: u64) -> u64 {
        share_up(points, self.fittings_kg, self.points)
    }

    /// **The alloys a scrap of `points` surviving structural points yields**, kilograms, rounded
    /// down (§4.6's `mass_yield_kg`): the structure they hold at [`SCRAP_RECOVERY_PERMILLE`].
    #[must_use]
    pub fn scrap_yield_kg(&self, points: u64) -> u64 {
        let held = u128::from(points.min(self.points)) * u128::from(self.structure_kg)
            / u128::from(self.points.max(1));
        u64::try_from(held * u128::from(SCRAP_RECOVERY_PERMILLE) / 1_000).unwrap_or(u64::MAX)
    }

    /// **The remains' yield**, kilograms (§4.6 on a site): the whole structure at
    /// [`SCRAP_RECOVERY_PERMILLE`]; remains keep no points to count.
    #[must_use]
    pub fn remains_yield_kg(&self) -> u64 {
        self.scrap_yield_kg(self.points)
    }

    /// The assembly loss, kilograms.
    #[must_use]
    pub const fn loss_kg(&self) -> u64 {
        self.dry_mass_kg.saturating_mul(LOSS_PERMILLE) / 1_000
    }

    /// **The bill's two bulk lines, whole tonnes** (§4.7): each the hull's share plus its share
    /// of the loss, rounded up — `(alloys_t, fittings_t)`.
    #[must_use]
    pub const fn bill_tonnes(&self) -> (u64, u64) {
        let loss = self.loss_kg();
        let loss_structure = loss.saturating_mul(self.share_permille) / 1_000;
        let loss_fittings = loss.saturating_sub(loss_structure);
        (
            self.structure_kg
                .saturating_add(loss_structure)
                .div_ceil(KG_PER_T),
            self.fittings_kg
                .saturating_add(loss_fittings)
                .div_ceil(KG_PER_T),
        )
    }

    /// The build's IC days (§4.7): one per 20 t, at least two.
    #[must_use]
    pub const fn build_days(&self) -> u64 {
        let days = self.dry_mass_kg.div_ceil(20 * KG_PER_T);
        if days < 2 { 2 } else { days }
    }

    /// **The design's bill, minor units** (§4.4, §4.7): the parts, the core and the two bulk
    /// lines at the references the caller reads (`alloys_minor_t`, `fittings_minor_t` a tonne),
    /// and the build and its margin at the yard's provisional row. The escort's is 75,495 cr at
    /// 450 and 520; a scrap's fifth reads it (§4.6).
    #[must_use]
    pub fn bill_minor(&self, alloys_minor_t: u64, fittings_minor_t: u64) -> u64 {
        let (alloys_t, fittings_t) = self.bill_tonnes();
        let build = self
            .build_days()
            .saturating_mul(WORK_UNITS_PER_DAY)
            .saturating_mul(YARD_WORK_UNIT_MINOR);
        let margin = build.saturating_mul(YARD_MARGIN_PERMILLE) / 1_000;
        self.parts_minor
            .saturating_add(self.core_minor)
            .saturating_add(alloys_t.saturating_mul(alloys_minor_t))
            .saturating_add(fittings_t.saturating_mul(fittings_minor_t))
            .saturating_add(build)
            .saturating_add(margin)
    }

    /// The figure a hundred points draw, kilograms a point to the hundredth, for the
    /// structure and the fittings: `(86, 29)` reads 0.86 and 0.29 kg for an escort.
    #[must_use]
    pub fn per_point_centi(&self) -> (u64, u64) {
        (
            share_up(100, self.structure_kg, self.points),
            share_up(100, self.fittings_kg, self.points),
        )
    }
}

/// `points × of / over`, rounded up.
fn share_up(points: u64, of: u64, over: u64) -> u64 {
    let over = u128::from(over.max(1));
    u64::try_from((u128::from(points) * u128::from(of)).div_ceil(over)).unwrap_or(u64::MAX)
}

/// **A design's structural points**: every facet's armour and every section's internals —
/// never a heat cell (doc 39 review 1 #2.4).
#[must_use]
pub fn structural_points(spec: &UnitSpec) -> u64 {
    spec.sections.iter().fold(0u64, |sum, section| {
        section.facets.iter().fold(
            sum.saturating_add(u64::from(section.internals)),
            |sum, facet| sum.saturating_add(u64::from(facet.armor)),
        )
    })
}

/// **Partition `spec`** (§4.8): its dry mass (the authored `logistics.dry_mass_t`, else the
/// field-ops measure, [`crate::field_ops::dry_mass_kg_for_spec`]), one part a slot at the goods
/// table's masses and references, its core, and the remainder split by its class. `None` for a
/// class the yard does not partition, a design with no structural points, and a design whose
/// parts and core leave no remainder (a simulator unit whose derived mass is lighter than its
/// fittings): such a hull draws nothing for repair and yields nothing to scrap.
///
/// # Errors
///
/// A core the catalogue does not carry.
pub fn partition(
    unit: &CanonicalId,
    spec: &UnitSpec,
    goods: &GoodsTable,
    cores: &BTreeMap<CanonicalId, CoreDef>,
    parts: &BTreeMap<CanonicalId, crate::definitions::PartRow>,
) -> Result<Option<MassBudget>, DefinitionError> {
    let Some(share_permille) = structure_share_permille(spec.class_of()) else {
        return Ok(None);
    };
    let points = structural_points(spec);
    if points == 0 {
        return Ok(None);
    }
    let dry_mass_kg = crate::field_ops::dry_mass_kg_for_spec(spec);
    let (mut parts_kg, mut parts_minor) = (0u64, 0u64);
    let mut core = CoreClass::Zero;
    for component in &spec.components {
        if let crate::definitions::unit::ComponentRole::Core { core: id } = &component.role {
            core = cores
                .get(id)
                .map(|def| def.class)
                .ok_or_else(|| DefinitionError::reference(unit.as_str(), "core", id.as_str()))?;
            continue;
        }
        let Ok(key) = CanonicalId::new(format!("{unit}.{}", component.id)) else {
            continue;
        };
        let Some(good) = parts.get(&key).and_then(|row| goods.good(&row.commodity)) else {
            continue;
        };
        parts_kg = parts_kg.saturating_add(good.mass_kg_per_unit().unwrap_or(0));
        parts_minor = parts_minor.saturating_add(good.reference_major.saturating_mul(100));
    }
    let core_kg = core_kg(core);
    let Some(remainder) = dry_mass_kg
        .checked_sub(parts_kg)
        .and_then(|left| left.checked_sub(core_kg))
        .filter(|left| *left > 0)
    else {
        return Ok(None);
    };
    let structure_kg = remainder.saturating_mul(share_permille) / 1_000;
    Ok(Some(MassBudget {
        dry_mass_kg,
        parts_kg,
        core_kg,
        structure_kg,
        fittings_kg: remainder.saturating_sub(structure_kg),
        points,
        share_permille,
        parts_minor,
        core_minor: core_minor(core),
    }))
}

// ---- the world's reads -------------------------------------------------------------------

/// **`hull`'s design and its budget** (doc 39 cut IN.8): the catalogue entry its unit's schema
/// names, when the yard partitions it.
#[must_use]
pub(crate) fn hull_budget(
    w: &mush_core::World,
    hull: mush_core::ObjId,
) -> Option<(CanonicalId, MassBudget)> {
    let unit = mush_tactics::module::unit_of(w, hull)?;
    let store = w.module_state::<crate::store::TacsimStore>()?;
    let wanted = unit.schema().key().id.as_str();
    store
        .definitions()
        .budgets
        .iter()
        .find(|(id, _)| id.as_str() == wanted)
        .map(|(id, budget)| (id.clone(), budget.clone()))
}

/// **A bulk good's reference, minor units a ledger unit** (§4.6, §4.10): the live ask of the
/// listing the lane's programme names for it (`structural_alloys` at Kasei, `industrial_parts`
/// at the Yards), else the goods table's baseline; 0 when neither reads.
#[must_use]
pub(crate) fn good_reference_minor(w: &mush_core::World, good: &str) -> u64 {
    let Ok(good) = CanonicalId::new(good) else {
        return 0;
    };
    let named = crate::port::voyage::lane_ports(w)
        .into_iter()
        .find_map(|(def, _)| def.programme.as_ref()?.reference.get(&good).cloned());
    crate::port::proceeds::reference_of(w, named.as_ref(), &good, good.as_str())
        .ok()
        .and_then(|(price, _)| u64::try_from(price.minor()).ok())
        .unwrap_or(0)
}

/// **A design's reference price, minor units** (§4.6, Q-39-17): the programme's buyout for a
/// programme hull (the Dray, and the refits it takes), else the design's bill at the live
/// references.
#[must_use]
pub(crate) fn reference_price_minor(
    w: &mush_core::World,
    design: &CanonicalId,
    budget: &MassBudget,
) -> u64 {
    let scale = crate::port::apply::credit_scale(w);
    let buyout = crate::port::voyage::lane_ports(w)
        .into_iter()
        .find_map(|(def, _)| {
            let programme = def.programme.as_ref()?;
            (&programme.unit == design || programme.refits.values().any(|unit| unit == design))
                .then_some(programme.buyout_major)
        });
    if let Some(minor) = buyout
        .and_then(|major| crate::port::apply::minor_of(major, scale))
        .and_then(|minor| u64::try_from(minor).ok())
    {
        return minor;
    }
    budget.bill_minor(
        good_reference_minor(w, crate::port::bay_work::ALLOYS_GOOD),
        good_reference_minor(w, crate::port::bay_work::PARTS_GOOD),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn escort() -> MassBudget {
        MassBudget {
            dry_mass_kg: 8_000,
            parts_kg: 4_330,
            core_kg: 250,
            structure_kg: 2_565,
            fittings_kg: 855,
            points: 2_990,
            share_permille: 750,
            parts_minor: 6_150_000,
            core_minor: 800_000,
        }
    }

    /// **Doc 39 §4.4's escort, line by line** (cut IN.8): 3 t of alloys and 1 t of fittings,
    /// two days, 75,495 cr at 450 and 520; a patch of 100 points draws 86 and 29 kg; a whole
    /// scrap yields 1,539 kg.
    #[test]
    fn in8_the_escorts_bill_draw_and_yield_are_the_documents() {
        let escort = escort();
        assert_eq!(escort.loss_kg(), 240);
        assert_eq!(escort.bill_tonnes(), (3, 1));
        assert_eq!(escort.build_days(), 2);
        assert_eq!(escort.bill_minor(45_000, 52_000), 7_549_500);
        assert_eq!(
            (escort.structure_kg_for(100), escort.fittings_kg_for(100)),
            (86, 29)
        );
        assert_eq!(escort.per_point_centi(), (86, 29));
        assert_eq!(escort.remains_yield_kg(), 1_539);
        assert_eq!(escort.scrap_yield_kg(0), 0);
        assert_eq!(escort.scrap_yield_kg(9_999), 1_539, "never past the design");
    }

    /// **The Dray** (§4.4, §4.8): 163 t and 55 t, 14 days, 86.5 and 28.8 kg a point.
    #[test]
    fn in8_the_drays_partition_is_the_documents() {
        let dray = MassBudget {
            dry_mass_kg: 240_000,
            parts_kg: 30_000,
            core_kg: 0,
            structure_kg: 157_500,
            fittings_kg: 52_500,
            points: 1_820,
            share_permille: 750,
            parts_minor: 3_000_000,
            core_minor: 0,
        };
        assert_eq!(dray.bill_tonnes(), (163, 55));
        assert_eq!(dray.build_days(), 12);
        assert_eq!(dray.structure_kg_for(100), 8_654);
        assert_eq!(dray.fittings_kg_for(100), 2_885);
        assert_eq!(dray.remains_yield_kg(), 94_500);
    }
}

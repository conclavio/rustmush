//! **The carrier's loss: one consequence table** (doc 36 cut GC.3; Q-36-8, Q-36-47; doc 32 F-3).
//!
//! What a carrier's damage costs it, in one place, so a hull lost on the ground and one lost in
//! space answer to the same rules (Q-36-47: a carrier is the same hull in a ground field and in
//! a space bubble). The table is keyed on the state of a hull's sections **by the role each
//! plays** ([`CarrierPart`]), read off the authored consequences, never off a section's name: the
//! Dray's `drive` and the Halcyon's `power` are both the part whose loss stops the hull moving.
//! Space maps its own `ShipSectionId` onto [`CarrierPart`] in its own tree, as Q-36-47's
//! follow-up; nothing here reads space.
//!
//! **The table** ([`TABLE`], Q-36-8 as ruled 2026-11-02):
//!
//! - **a destroyed drive refuses lift**: the hull neither lifts from a pad nor sails from a dock
//!   until the drive is rebuilt;
//! - **a destroyed hull wrecks the carrier**: it refuses lift too, its crew are stranded on the
//!   site and leave by extraction (Q-CF-39), and every rig in its bay becomes a wreck where the
//!   carrier stands;
//! - **a carrier wreck needs a tug of at least half its tonnage** ([`tug_floor_kg`]), since a
//!   unit tows up to twice its own (FO-13). No shipped design is that heavy, so in practice a
//!   carrier wreck is not towed yet.
//!
//! **Who reads it.** `crate::port::voyage`'s departure gate (live `+ship/lift`, `+ship/undock` and
//! a queued departure alike), the raider's barge lift (`crate::field::raiders`), the crew check-in
//! at demotion (`crate::field::crew`), the demotion's bay pass (`crate::field::close`) and the tow
//! gate (`crate::field::carrier::refuse_tow`). A section is down when its condition is destroyed,
//! a wreck or irrecoverable, or its internals stand at their floor.

use std::collections::BTreeSet;

use mush_core::{ObjId, World};
use mush_tactics::field_ops::SectionCondition;
use mush_tactics::{
    CapabilityKey, CellAddr, CellId, ConditionId, SectionId, TacticalUnit, UnitMutationOp,
    UnitMutationPlan,
};

use crate::arena::damage::{
    HULL_SECTION, MOBILITY_CAPABILITY, POWER_CAPABILITY, WEAPON_CAPABILITY_PREFIX, WRECK_CONDITION,
};
use crate::definitions::UnitSpec;
use crate::definitions::unit::{Consequence, INTERNALS_CELL, SectionDef};

/// **A part of a carrier the table reads**, by the role its section plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CarrierPart {
    /// The section whose loss destroys the unit (`destroy_unit`): the Dray's and the Halcyon's
    /// `hull`.
    Hull,
    /// The section whose loss stops it moving (`disable_motion`): the Dray's `drive`, the
    /// Halcyon's `power`.
    Drive,
}

impl CarrierPart {
    /// The part `section` plays, from its authored consequences: `destroy_unit` makes it the
    /// hull, else `disable_motion` the drive.
    #[must_use]
    pub fn of_section(section: &SectionDef) -> Option<CarrierPart> {
        if section
            .consequences
            .iter()
            .any(|consequence| matches!(consequence, Consequence::DestroyUnit))
        {
            Some(CarrierPart::Hull)
        } else if section
            .consequences
            .iter()
            .any(|consequence| matches!(consequence, Consequence::DisableMotion))
        {
            Some(CarrierPart::Drive)
        } else {
            None
        }
    }

    /// The word a line uses: `hull`, `drive`.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            CarrierPart::Hull => "hull",
            CarrierPart::Drive => "drive",
        }
    }
}

/// **What a part's loss does** (Q-36-8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CarrierLoss {
    /// It neither lifts from a pad nor sails from a dock.
    RefusesLift,
    /// It is a wreck where it stands: a salvage object, towed only by a tug of at least half its
    /// tonnage ([`tug_floor_kg`]).
    Wrecked,
    /// Everyone aboard is stranded on the site and leaves by extraction (Q-CF-39).
    CrewStranded,
    /// Every rig in its bay is a wreck too, where the carrier stands.
    BayRigsWrecked,
}

/// **The table** (Q-36-8): each part, and what its loss does.
pub const TABLE: [(CarrierPart, &[CarrierLoss]); 2] = [
    (CarrierPart::Drive, &[CarrierLoss::RefusesLift]),
    (
        CarrierPart::Hull,
        &[
            CarrierLoss::RefusesLift,
            CarrierLoss::Wrecked,
            CarrierLoss::CrewStranded,
            CarrierLoss::BayRigsWrecked,
        ],
    ),
];

/// **A carrier wreck's tug floor** (Q-36-8 as ruled): a tug of at least this share of the
/// wreck's tonnage, per mille.
pub const TUG_SHARE_PERMILLE: u64 = 500;

/// A destroyed drive's refusal: `{name}`.
pub const DRIVE_DOWN: &str =
    "The {name}'s drive is destroyed: it cannot lift until the drive is rebuilt.";
/// A wrecked carrier's refusal: `{name}`.
pub const HULL_DOWN: &str = "The {name} is a wreck: it will never lift again. Anyone aboard is \
     stranded and can ask +sortie/extract for a way out.";

/// **The parts of one carrier that are down.**
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CarrierDamage {
    down: BTreeSet<CarrierPart>,
}

impl CarrierDamage {
    /// **Read `cells` against its spec**: a part is down when any section playing it is.
    #[must_use]
    pub fn read(spec: &UnitSpec, cells: &TacticalUnit) -> CarrierDamage {
        let down = spec
            .sections
            .iter()
            .filter_map(|section| {
                let part = CarrierPart::of_section(section)?;
                section_down(cells, section.id.as_str()).then_some(part)
            })
            .collect();
        CarrierDamage { down }
    }

    /// The same, from the parts named down (a test's or space's reading).
    #[must_use]
    pub fn of_parts(parts: impl IntoIterator<Item = CarrierPart>) -> CarrierDamage {
        CarrierDamage {
            down: parts.into_iter().collect(),
        }
    }

    /// Whether `part` is down.
    #[must_use]
    pub fn is_down(&self, part: CarrierPart) -> bool {
        self.down.contains(&part)
    }

    /// **What the parts down cost**, per [`TABLE`].
    #[must_use]
    pub fn losses(&self) -> BTreeSet<CarrierLoss> {
        TABLE
            .iter()
            .filter(|(part, _)| self.is_down(*part))
            .flat_map(|(_, losses)| losses.iter().copied())
            .collect()
    }

    /// Whether the parts down cost `loss`.
    #[must_use]
    pub fn has(&self, loss: CarrierLoss) -> bool {
        self.losses().contains(&loss)
    }

    /// **The refusal a lift meets**, with `{name}` unfilled: a wreck's before a drive's.
    #[must_use]
    pub fn lift_refusal(&self) -> Option<&'static str> {
        if self.has(CarrierLoss::Wrecked) {
            Some(HULL_DOWN)
        } else if self.has(CarrierLoss::RefusesLift) {
            Some(DRIVE_DOWN)
        } else {
            None
        }
    }
}

/// **Whether `section` of `cells` is down**: destroyed, a wreck or irrecoverable, or its
/// internals at their floor.
#[must_use]
pub fn section_down(cells: &TacticalUnit, section: &str) -> bool {
    let Ok(id) = SectionId::new(section) else {
        return false;
    };
    let condition = cells
        .section(&id)
        .map(|state| SectionCondition::classify_condition_id(&state.condition));
    if matches!(
        condition,
        Some(
            SectionCondition::Destroyed | SectionCondition::Wreck | SectionCondition::Irrecoverable
        )
    ) {
        return true;
    }
    CellId::new(INTERNALS_CELL).is_ok_and(|cell| {
        cells
            .cell(&CellAddr::Section { section: id, cell })
            .is_some_and(|internals| internals.current <= internals.min)
    })
}

/// **The authored spec behind `cells`**, by its schema's definition id.
#[must_use]
pub fn spec_of<'a>(w: &'a World, cells: &TacticalUnit) -> Option<&'a UnitSpec> {
    let wanted = cells.schema().key().id.as_str();
    w.module_state::<crate::TacsimStore>()?
        .definitions()
        .specs
        .iter()
        .find(|(id, _)| id.as_str() == wanted)
        .map(|(_, spec)| spec)
}

/// **`hull`'s damage now**, from its own unit record; `None` when it has no record or no spec.
#[must_use]
pub fn damage_of(w: &World, hull: ObjId) -> Option<CarrierDamage> {
    let cells = mush_tactics::module::unit_of(w, hull)?;
    Some(CarrierDamage::read(spec_of(w, cells)?, cells))
}

/// **`unit`'s dry mass, kilograms** (FO-13's measure, `crate::field_ops::dry_mass_kg_for_spec`).
#[must_use]
pub fn mass_kg(w: &World, unit: ObjId) -> Option<u64> {
    let cells = mush_tactics::module::unit_of(w, unit)?;
    spec_of(w, cells).map(crate::field_ops::dry_mass_kg_for_spec)
}

/// **The least tug that may tow a carrier wreck of `carrier_kg`** (Q-36-8): half its tonnage,
/// rounded up.
#[must_use]
pub fn tug_floor_kg(carrier_kg: u64) -> u64 {
    let floor = (u128::from(carrier_kg) * u128::from(TUG_SHARE_PERMILLE)).div_ceil(1_000);
    u64::try_from(floor).unwrap_or(u64::MAX)
}

/// **The refusal a carrier's lift meets for its damage**, named: `None` when it may lift.
#[must_use]
pub fn lift_refusal(w: &World, hull: ObjId) -> Option<String> {
    let line = damage_of(w, hull)?.lift_refusal()?;
    Some(line.replace("{name}", &crate::port::coop::name_of(w, hull)))
}

/// **The plan that makes `cells` a wreck where it stands** (a wrecked carrier's bay rigs,
/// Q-36-8): the arena's own wreck mark — the [`WRECK_CONDITION`] on the hull, or on the section
/// whose loss destroys the unit when it has no hull — and the three revocations doc 16 §3.3
/// names. `None` for a unit that is a wreck already, or has no section to carry the mark.
#[must_use]
pub fn wreck_plan(spec: Option<&UnitSpec>, cells: &TacticalUnit) -> Option<UnitMutationPlan> {
    if crate::arena::damage::is_wreck(cells) {
        return None;
    }
    let body = SectionId::new(HULL_SECTION)
        .ok()
        .filter(|hull| cells.section(hull).is_some())
        .or_else(|| {
            spec?
                .sections
                .iter()
                .find(|section| CarrierPart::of_section(section) == Some(CarrierPart::Hull))
                .and_then(|section| SectionId::new(section.id.as_str()).ok())
                .filter(|section| cells.section(section).is_some())
        })?;
    let mut ops = vec![UnitMutationOp::SetSectionCondition {
        section: body,
        condition: ConditionId::new(WRECK_CONDITION).ok()?,
    }];
    let revoked: Vec<CapabilityKey> = cells
        .capabilities()
        .filter(|key| {
            let id = key.id.as_str();
            id == POWER_CAPABILITY
                || id == MOBILITY_CAPABILITY
                || id.starts_with(WEAPON_CAPABILITY_PREFIX)
        })
        .cloned()
        .collect();
    ops.extend(
        revoked
            .into_iter()
            .map(|key| UnitMutationOp::RevokeCapability { key }),
    );
    UnitMutationPlan::new(
        ops,
        Some(cells.revision()),
        Some(smol_str::SmolStr::new_static("carrier:wrecked-in-bay")),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The table as ruled** (Q-36-8): a drive refuses lift and nothing else; a hull wrecks,
    /// strands and takes the bay with it; a wreck's lift refusal is the wreck's.
    #[test]
    fn gc3_the_table_is_the_ruling() {
        let drive = CarrierDamage::of_parts([CarrierPart::Drive]);
        assert_eq!(drive.losses(), BTreeSet::from([CarrierLoss::RefusesLift]));
        assert_eq!(drive.lift_refusal(), Some(DRIVE_DOWN));
        let hull = CarrierDamage::of_parts([CarrierPart::Hull]);
        for loss in [
            CarrierLoss::RefusesLift,
            CarrierLoss::Wrecked,
            CarrierLoss::CrewStranded,
            CarrierLoss::BayRigsWrecked,
        ] {
            assert!(hull.has(loss), "{loss:?}");
        }
        let both = CarrierDamage::of_parts([CarrierPart::Drive, CarrierPart::Hull]);
        assert_eq!(both.lift_refusal(), Some(HULL_DOWN));
        assert_eq!(CarrierDamage::default().lift_refusal(), None);
        assert!(CarrierDamage::default().losses().is_empty());
    }

    /// **A carrier wreck's tug** (Q-36-8 as ruled): at least half its tonnage, rounded up.
    #[test]
    fn gc3_a_carrier_wreck_needs_a_tug_of_half_its_tonnage() {
        assert_eq!(tug_floor_kg(240_000), 120_000);
        assert_eq!(tug_floor_kg(3), 2);
        assert_eq!(tug_floor_kg(0), 0);
        assert_eq!(tug_floor_kg(u64::MAX), u64::MAX.div_ceil(2));
    }
}

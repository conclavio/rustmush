//! `H1-A` — Field-unit component survival, operational state derivation, epoch
//! disposition, and crash-safe versioned authority handoff (design doc 27 §4.1,
//! §4.4, `FO-1`, `FO-2`, `FO-10`, `Q-H7`–`Q-H9`).
//!
//! ## Core invariants
//!
//! 1. **Component survival is localized to destroyed host sections or explicit
//!    component hits**: defeating a unit via cockpit/head destruction, mobility
//!    kill, or power loss does not blanket-destroy components in surviving
//!    sections.
//! 2. **Distinct operational states**: `Operational`, `Immobile`, `Unarmed`,
//!    `Disabled`, `Wreck`, and `Irrecoverable` are derived deterministically
//!    from the unit's structural sections, installed components, and cells.
//! 3. **Recoverable defeats keep the hull epoch open (`Q-H9`)**: `Immobile`,
//!    `Unarmed`, `Disabled`, and `Wreck` units remain physical field objects
//!    whose hull epoch stays open across disablement, abandonment, and towing
//!    until carrier/hangar recovery closes it as `RecoveredToBase`. Only
//!    `Irrecoverable` permits closing as `Destroyed`.
//! 4. **IC custody mutation vocabulary compliance**: `ic_condition_plan` emits
//!    only `SetCell`, `SetSectionCondition`, and `SetComponentCondition`
//!    (never `RevokeCapability` or `GrantCapability`), satisfying
//!    `ic_custody::close_unit` while preserving surviving component state.
//! 5. **Versioned authority handoff without OOC rollback (`§4.4`, `FO-10`)**:
//!    `FieldCustodyLedger` fences authority transitions with a monotonic
//!    `authority_generation` and refuses OOC snapshot rollback or revision
//!    regression on IC units.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use mush_core::{DefinitionRef, ObjId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::unit::{
    CellAddr, CellId, ComponentId, ConditionId, InstallPoint, SectionId, TacticalUnit, UnitError,
    UnitMutationOp, UnitMutationPlan,
};

/// Maximum tracked units in one persisted [`FieldCustodyLedger`].
pub const MAX_FIELD_CUSTODY_UNITS: usize = 1_024;

/// Canonical condition vocabulary for an installed component (`FO-1`, `Q-H8`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentCondition {
    /// Fully functional component.
    Intact,
    /// Degraded/partially damaged component that remains operational or repairable.
    Damaged,
    /// Temporarily or mechanically jammed/offline component that remains salvageable.
    Jammed,
    /// Ruined component (either directly destroyed or hosted in a destroyed section).
    Destroyed,
}

impl ComponentCondition {
    /// Canonical tactical condition string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Intact => "intact",
            Self::Damaged => "damaged",
            Self::Jammed => "jammed",
            Self::Destroyed => "destroyed",
        }
    }

    /// Convert to a validated [`ConditionId`].
    ///
    /// # Errors
    /// Returns [`UnitError`] only if canonical id construction fails.
    pub fn to_condition_id(self) -> Result<ConditionId, UnitError> {
        ConditionId::new(self.as_str())
    }

    /// Parse an exact canonical [`ConditionId`] into a [`ComponentCondition`].
    #[must_use]
    pub fn from_condition_id(id: &ConditionId) -> Option<Self> {
        match id.as_str() {
            "intact" => Some(Self::Intact),
            "damaged" => Some(Self::Damaged),
            "jammed" => Some(Self::Jammed),
            "destroyed" => Some(Self::Destroyed),
            _ => None,
        }
    }

    /// Classify any [`ConditionId`] into the closest [`ComponentCondition`].
    #[must_use]
    pub fn classify_condition_id(id: &ConditionId) -> Self {
        match id.as_str() {
            "intact" | "ready" | "ok" | "nominal" => Self::Intact,
            "damaged" | "degraded" | "worn" => Self::Damaged,
            "jammed" | "offline" | "disabled" | "fault" => Self::Jammed,
            "destroyed" | "wreck" | "ruined" | "irrecoverable" | "dead" => Self::Destroyed,
            _ => Self::Intact,
        }
    }

    /// Whether the component can actively operate in the field.
    #[must_use]
    pub const fn is_operational(self) -> bool {
        matches!(self, Self::Intact | Self::Damaged)
    }

    /// Whether the component survives as a salvageable/repairable part (`FO-2`).
    #[must_use]
    pub const fn is_salvageable(self) -> bool {
        !matches!(self, Self::Destroyed)
    }
}

/// Canonical condition vocabulary for a structural section (`FO-1`, `Q-H7`, `Q-H8`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionCondition {
    /// Undamaged or structurally sound section.
    Intact,
    /// Structurally damaged section whose hosted components still survive unless directly hit.
    Damaged,
    /// Section structure breached/destroyed; components installed in this section are lost.
    Destroyed,
    /// Primary body/hull mission-killed into a field wreck without obliterating surviving sections.
    Wreck,
    /// Catastrophic structural loss leaving no recoverable section.
    Irrecoverable,
}

impl SectionCondition {
    /// Canonical tactical condition string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Intact => "intact",
            Self::Damaged => "damaged",
            Self::Destroyed => "destroyed",
            Self::Wreck => "wreck",
            Self::Irrecoverable => "irrecoverable",
        }
    }

    /// Convert to a validated [`ConditionId`].
    ///
    /// # Errors
    /// Returns [`UnitError`] only if canonical id construction fails.
    pub fn to_condition_id(self) -> Result<ConditionId, UnitError> {
        ConditionId::new(self.as_str())
    }

    /// Parse an exact canonical [`ConditionId`] into a [`SectionCondition`].
    #[must_use]
    pub fn from_condition_id(id: &ConditionId) -> Option<Self> {
        match id.as_str() {
            "intact" => Some(Self::Intact),
            "damaged" => Some(Self::Damaged),
            "destroyed" => Some(Self::Destroyed),
            "wreck" => Some(Self::Wreck),
            "irrecoverable" => Some(Self::Irrecoverable),
            _ => None,
        }
    }

    /// Classify any [`ConditionId`] into the closest [`SectionCondition`].
    #[must_use]
    pub fn classify_condition_id(id: &ConditionId) -> Self {
        match id.as_str() {
            "intact" | "ready" | "ok" | "nominal" => Self::Intact,
            "damaged" | "degraded" | "breached" => Self::Damaged,
            "destroyed" | "ruined" | "severed" => Self::Destroyed,
            "wreck" | "disabled" => Self::Wreck,
            "irrecoverable" | "vaporized" => Self::Irrecoverable,
            _ => Self::Intact,
        }
    }

    /// Whether this section condition destroys components installed inside it.
    #[must_use]
    pub const fn destroys_hosted_components(self) -> bool {
        matches!(self, Self::Destroyed | Self::Irrecoverable)
    }

    /// Whether the section remains operational for locomotion or control roles.
    #[must_use]
    pub const fn is_functional(self) -> bool {
        matches!(self, Self::Intact | Self::Damaged)
    }
}

/// Distinct operational states for a field unit (`FO-1`, `Q-H7`, `Q-H9`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationalState {
    /// Active control/power, mobile, and (if an armed hull) has at least one working weapon.
    Operational,
    /// Retains power and control, but locomotion is lost (e.g. tracks or legs destroyed).
    Immobile,
    /// Retains power, control, and mobility, but all weapons on an armed hull are inoperable.
    Unarmed,
    /// Inoperable under its own control/power (e.g. cockpit/head loss on a biped while torso
    /// and limbs survive, power plant offline, or both immobile and unarmed), yet structurally
    /// recoverable with surviving components preserved.
    Disabled,
    /// Primary structural/mission defeat leaving an uncrewed/inoperable field wreck that can
    /// still be towed and stripped/repaired for surviving sections and components.
    Wreck,
    /// Catastrophic total loss leaving no towable hull (`IcCloseKind::Destroyed`).
    Irrecoverable,
}

impl OperationalState {
    /// Canonical snake_case label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Operational => "operational",
            Self::Immobile => "immobile",
            Self::Unarmed => "unarmed",
            Self::Disabled => "disabled",
            Self::Wreck => "wreck",
            Self::Irrecoverable => "irrecoverable",
        }
    }

    /// Whether the unit is fully operational.
    #[must_use]
    pub const fn is_operational(self) -> bool {
        matches!(self, Self::Operational)
    }

    /// Whether the unit can be attached and towed by a recovery tug (`FO-1`, `FO-13`, `Q-S5`).
    #[must_use]
    pub const fn is_towable(self) -> bool {
        matches!(
            self,
            Self::Immobile | Self::Unarmed | Self::Disabled | Self::Wreck
        )
    }

    /// Whether the unit is defeated for combat resolution purposes (`Disabled`, `Wreck`, `Irrecoverable`).
    #[must_use]
    pub const fn is_combat_defeated(self) -> bool {
        matches!(self, Self::Disabled | Self::Wreck | Self::Irrecoverable)
    }

    /// Whether the unit retains a physical hull with potentially salvageable components (`FO-2`).
    #[must_use]
    pub const fn retains_salvageable_hull(self) -> bool {
        !matches!(self, Self::Irrecoverable)
    }

    /// Hull epoch disposition while the unit is on the field (`Q-H9`).
    #[must_use]
    pub const fn epoch_disposition(self) -> FieldEpochDisposition {
        match self {
            Self::Irrecoverable => FieldEpochDisposition::CloseDestroyed,
            Self::Operational | Self::Immobile | Self::Unarmed | Self::Disabled | Self::Wreck => {
                FieldEpochDisposition::RemainOpen
            }
        }
    }
}

impl fmt::Display for OperationalState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether a field unit's hull epoch remains open on the field or closes as destroyed (`Q-H9`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldEpochDisposition {
    /// Recoverable unit (`Operational`, `Immobile`, `Unarmed`, `Disabled`, `Wreck`):
    /// hull epoch remains open across disablement, abandonment, and towing until base/carrier
    /// recovery closes it.
    RemainOpen,
    /// Catastrophic total loss (`Irrecoverable`): hull epoch may close as `Destroyed`.
    CloseDestroyed,
}

/// Neutral representation of how an IC hull sortie/epoch closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldCloseOutcome {
    /// Unit returned to base under its own power.
    ReturnedToBase {
        /// Destination hangar/carrier base object.
        base: ObjId,
    },
    /// Unit powered down inside base custody.
    PoweredDownInBase {
        /// Host hangar/carrier base object.
        base: ObjId,
    },
    /// Disabled/immobile/wrecked unit recovered to a base or carrier bay (`Q-H9`, `FO-3`).
    RecoveredToBase {
        /// Destination hangar/carrier base object.
        base: ObjId,
    },
    /// Catastrophic total destruction (`Irrecoverable` only).
    Destroyed,
}

/// Validate that a proposed [`FieldCloseOutcome`] is legal for the unit's [`OperationalState`] (`Q-H9`).
///
/// # Errors
/// Returns [`ConditionError`] if a recoverable unit is closed as `Destroyed`, if an
/// `Irrecoverable` unit is closed as returned/recovered, or if `base` is invalid.
pub fn validate_close_outcome(
    unit: ObjId,
    state: OperationalState,
    outcome: &FieldCloseOutcome,
) -> Result<(), ConditionError> {
    if unit.0 == 0 {
        return Err(ConditionError::InvalidObjectId("unit"));
    }
    match *outcome {
        FieldCloseOutcome::Destroyed => {
            if state != OperationalState::Irrecoverable {
                return Err(ConditionError::RecoverableUnitCannotCloseDestroyed { state });
            }
            Ok(())
        }
        FieldCloseOutcome::ReturnedToBase { base } => {
            if base.0 == 0 || base == unit {
                return Err(ConditionError::InvalidBaseObject { unit, base });
            }
            if !matches!(
                state,
                OperationalState::Operational | OperationalState::Unarmed
            ) {
                return Err(ConditionError::CannotReturnUnderOwnPower { state });
            }
            Ok(())
        }
        FieldCloseOutcome::PoweredDownInBase { base }
        | FieldCloseOutcome::RecoveredToBase { base } => {
            if base.0 == 0 || base == unit {
                return Err(ConditionError::InvalidBaseObject { unit, base });
            }
            if state == OperationalState::Irrecoverable {
                return Err(ConditionError::IrrecoverableUnitCannotBeRecovered);
            }
            Ok(())
        }
    }
}

/// Ruleset-supplied anatomy roles for a [`TacticalUnit`] so `mush-tactics` can derive
/// [`FieldUnitAssessment`] without hardcoding ruleset section or component names.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitAnatomyProfile {
    /// Primary structural core sections (e.g. `torso` or `hull`).
    pub core_sections: BTreeSet<SectionId>,
    /// Command/control sections (e.g. `head`). Loss of any control section disables
    /// self-controlled operation (`Disabled`) without destroying torso/limb components.
    pub control_sections: BTreeSet<SectionId>,
    /// Required single-point locomotion sections (e.g. `engine` on a tank). Loss of any
    /// section in this set disables movement (`Immobile`).
    #[serde(default)]
    pub required_locomotion_sections: BTreeSet<SectionId>,
    /// Redundant locomotion sections (e.g. `left_track` + `right_track`, or `left_leg` + `right_leg`).
    pub locomotion_sections: BTreeSet<SectionId>,
    /// Minimum functional locomotion sections required to move under own power.
    pub min_locomotion_sections: usize,
    /// Installed components that provide primary unit power.
    pub power_components: BTreeSet<ComponentId>,
    /// Installed components that provide weapons.
    pub weapon_components: BTreeSet<ComponentId>,
    /// Installed components that provide sensors/ECM/ECCM.
    pub sensor_components: BTreeSet<ComponentId>,
    /// Optional section-scoped cell id tracking internal structure points (e.g. `"internals"`).
    pub internals_cell: Option<CellId>,
    /// Optional component-scoped cell id tracking component condition points (e.g. `"condition"`).
    pub component_health_cell: Option<CellId>,
    /// Optional component-scoped cell id tracking weapon ammunition (e.g. `"ammo"`).
    pub weapon_ammo_cell: Option<CellId>,
}

/// Derived operational and component-survival assessment of a [`TacticalUnit`] (`H1-A`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldUnitAssessment {
    /// Unit object id.
    pub unit: ObjId,
    /// Unit revision at assessment time.
    pub unit_revision: u64,
    /// Primary derived operational state.
    pub state: OperationalState,
    /// Hull epoch disposition implied by `state`.
    pub epoch_disposition: FieldEpochDisposition,
    /// Whether the unit retains power.
    pub has_power: bool,
    /// Whether the unit retains command/cockpit control and power.
    pub has_control: bool,
    /// Whether the unit can move under its own power.
    pub can_move: bool,
    /// Whether the unit has at least one operational weapon with ammo (if ammo-tracked).
    pub can_fire: bool,
    /// Whether the unit is a towable field casualty.
    pub is_towable: bool,
    /// Effective condition of each structural section.
    pub section_conditions: BTreeMap<SectionId, SectionCondition>,
    /// Effective condition of each installed component.
    pub component_conditions: BTreeMap<ComponentId, ComponentCondition>,
    /// Components that remain `Intact`.
    pub intact_components: Vec<ComponentId>,
    /// Components that are `Damaged` or `Jammed` (still salvageable/repairable).
    pub degraded_components: Vec<ComponentId>,
    /// Components that are `Destroyed`.
    pub destroyed_components: Vec<ComponentId>,
    /// Weapon components that remain operational and able to fire.
    pub operational_weapons: Vec<ComponentId>,
}

impl FieldUnitAssessment {
    /// All surviving (salvageable) components (`Intact`, `Damaged`, or `Jammed`).
    #[must_use]
    pub fn surviving_components(&self) -> Vec<ComponentId> {
        self.component_conditions
            .iter()
            .filter(|(_, cond)| cond.is_salvageable())
            .map(|(id, _)| id.clone())
            .collect()
    }
}

/// Determine the section in which an [`InstallPoint`] resides, if section-scoped.
#[must_use]
pub fn install_point_section(install: &InstallPoint) -> Option<&SectionId> {
    match install {
        InstallPoint::Unit => None,
        InstallPoint::Section(section) | InstallPoint::Slot { section, .. } => Some(section),
    }
}

/// Derive the effective [`SectionCondition`] of one section on `unit`.
#[must_use]
pub fn effective_section_condition(
    unit: &TacticalUnit,
    section_id: &SectionId,
    anatomy: &UnitAnatomyProfile,
) -> Option<SectionCondition> {
    let section = unit.section(section_id)?;
    let stored = SectionCondition::classify_condition_id(&section.condition);
    if matches!(
        stored,
        SectionCondition::Irrecoverable | SectionCondition::Destroyed
    ) {
        return Some(stored);
    }
    if let Some(internals_id) = &anatomy.internals_cell {
        let addr = CellAddr::Section {
            section: section_id.clone(),
            cell: internals_id.clone(),
        };
        if let Some(cell) = unit.cell(&addr) {
            if cell.current <= cell.min {
                return Some(SectionCondition::Destroyed);
            }
            if cell.current < cell.max && stored == SectionCondition::Intact {
                return Some(SectionCondition::Damaged);
            }
        }
    }
    Some(stored)
}

/// Derive the effective [`ComponentCondition`] of one installed component on `unit`.
#[must_use]
pub fn effective_component_condition(
    unit: &TacticalUnit,
    component_id: &ComponentId,
    section_conditions: &BTreeMap<SectionId, SectionCondition>,
    anatomy: &UnitAnatomyProfile,
) -> Option<ComponentCondition> {
    let comp = unit.component(component_id)?;
    let stored = ComponentCondition::classify_condition_id(&comp.condition);
    if stored == ComponentCondition::Destroyed {
        return Some(ComponentCondition::Destroyed);
    }
    if let Some(host_section) = install_point_section(&comp.install)
        && let Some(sec_cond) = section_conditions.get(host_section)
        && sec_cond.destroys_hosted_components()
    {
        return Some(ComponentCondition::Destroyed);
    }
    if let Some(health_id) = &anatomy.component_health_cell {
        let addr = CellAddr::Component {
            component: component_id.clone(),
            cell: health_id.clone(),
        };
        if let Some(cell) = unit.cell(&addr) {
            if cell.current <= cell.min {
                return Some(ComponentCondition::Destroyed);
            }
            if cell.current < cell.max && stored == ComponentCondition::Intact {
                return Some(ComponentCondition::Damaged);
            }
        }
    }
    Some(stored)
}

/// Pure assessment of a [`TacticalUnit`] given its [`UnitAnatomyProfile`] (`H1-A`).
#[must_use]
pub fn assess_tactical_unit(
    unit: &TacticalUnit,
    anatomy: &UnitAnatomyProfile,
) -> FieldUnitAssessment {
    let mut section_conditions = BTreeMap::new();
    for (sec_id, _) in unit.sections() {
        if let Some(cond) = effective_section_condition(unit, sec_id, anatomy) {
            section_conditions.insert(sec_id.clone(), cond);
        }
    }

    let mut component_conditions = BTreeMap::new();
    let mut intact_components = Vec::new();
    let mut degraded_components = Vec::new();
    let mut destroyed_components = Vec::new();

    for (comp_id, _) in unit.components() {
        if let Some(cond) =
            effective_component_condition(unit, comp_id, &section_conditions, anatomy)
        {
            match cond {
                ComponentCondition::Intact => intact_components.push(comp_id.clone()),
                ComponentCondition::Damaged | ComponentCondition::Jammed => {
                    degraded_components.push(comp_id.clone());
                }
                ComponentCondition::Destroyed => destroyed_components.push(comp_id.clone()),
            }
            component_conditions.insert(comp_id.clone(), cond);
        }
    }

    let any_irrecoverable = section_conditions
        .values()
        .any(|c| *c == SectionCondition::Irrecoverable);
    let all_sections_destroyed = !section_conditions.is_empty()
        && section_conditions.values().all(|c| {
            matches!(
                c,
                SectionCondition::Destroyed | SectionCondition::Irrecoverable
            )
        });

    let core_wrecked_or_destroyed = !anatomy.core_sections.is_empty()
        && anatomy.core_sections.iter().any(|sec_id| {
            section_conditions.get(sec_id).is_some_and(|c| {
                matches!(
                    c,
                    SectionCondition::Destroyed
                        | SectionCondition::Wreck
                        | SectionCondition::Irrecoverable
                )
            })
        });

    let any_section_wreck = section_conditions
        .values()
        .any(|c| *c == SectionCondition::Wreck);

    let has_power = if anatomy.power_components.is_empty() {
        !all_sections_destroyed && !any_irrecoverable
    } else {
        anatomy.power_components.iter().any(|comp_id| {
            component_conditions
                .get(comp_id)
                .is_some_and(|c| c.is_operational())
        })
    };

    let control_intact = anatomy.control_sections.iter().all(|sec_id| {
        section_conditions
            .get(sec_id)
            .is_some_and(|c| c.is_functional())
    });

    let has_control =
        has_power && control_intact && !core_wrecked_or_destroyed && !any_irrecoverable;

    let required_locomotion_ok = anatomy.required_locomotion_sections.iter().all(|sec_id| {
        section_conditions
            .get(sec_id)
            .is_some_and(|c| c.is_functional())
    });

    let functional_locomotion_count = anatomy
        .locomotion_sections
        .iter()
        .filter(|sec_id| {
            section_conditions
                .get(*sec_id)
                .is_some_and(|c| c.is_functional())
        })
        .count();

    let locomotion_ok = required_locomotion_ok
        && (if anatomy.min_locomotion_sections == 0 {
            true
        } else {
            functional_locomotion_count >= anatomy.min_locomotion_sections
        });

    let has_mobility_profile = unit.mobility().is_some();
    let can_move = has_control && has_mobility_profile && locomotion_ok;

    let mut operational_weapons = Vec::new();
    for weapon_id in &anatomy.weapon_components {
        let comp_ok = component_conditions
            .get(weapon_id)
            .is_some_and(|c| c.is_operational());
        if !comp_ok {
            continue;
        }
        let ammo_ok = if let Some(ammo_cell_id) = &anatomy.weapon_ammo_cell {
            let addr = CellAddr::Component {
                component: weapon_id.clone(),
                cell: ammo_cell_id.clone(),
            };
            unit.cell(&addr).is_none_or(|cell| cell.current > cell.min)
        } else {
            true
        };
        if ammo_ok {
            operational_weapons.push(weapon_id.clone());
        }
    }

    let is_armed_hull = !anatomy.weapon_components.is_empty();
    let can_fire = has_control && !operational_weapons.is_empty();

    let state = if any_irrecoverable || all_sections_destroyed {
        OperationalState::Irrecoverable
    } else if core_wrecked_or_destroyed || any_section_wreck {
        OperationalState::Wreck
    } else if !has_control || (!can_move && is_armed_hull && !can_fire) {
        OperationalState::Disabled
    } else if !can_move {
        OperationalState::Immobile
    } else if is_armed_hull && !can_fire {
        OperationalState::Unarmed
    } else {
        OperationalState::Operational
    };

    let epoch_disposition = state.epoch_disposition();
    let is_towable = state.is_towable();

    FieldUnitAssessment {
        unit: unit.object(),
        unit_revision: unit.revision(),
        state,
        epoch_disposition,
        has_power,
        has_control,
        can_move,
        can_fire,
        is_towable,
        section_conditions,
        component_conditions,
        intact_components,
        degraded_components,
        destroyed_components,
        operational_weapons,
    }
}

/// Construct an IC-custody-legal [`UnitMutationPlan`] that transforms `before` into
/// `after`'s cell, section-condition, and component-condition state (`H1-A`, `Q-H8`).
///
/// # Errors
/// Returns [`ConditionError`] if `before` and `after` do not refer to the same unit
/// or schema, or if plan construction fails.
pub fn ic_condition_plan(
    before: &TacticalUnit,
    after: &TacticalUnit,
    anatomy: Option<&UnitAnatomyProfile>,
    cause: Option<SmolStr>,
) -> Result<Option<UnitMutationPlan>, ConditionError> {
    if before.object() != after.object() {
        return Err(ConditionError::UnitMismatch {
            before: before.object(),
            after: after.object(),
        });
    }
    if before.schema() != after.schema() {
        return Err(ConditionError::SchemaMismatch);
    }

    let mut ops = Vec::new();

    for (addr, before_cell) in before.cells() {
        let after_cell = after
            .cell(addr)
            .ok_or_else(|| ConditionError::MissingCell(addr.clone()))?;
        if before_cell.current != after_cell.current {
            ops.push(UnitMutationOp::SetCell {
                addr: addr.clone(),
                value: after_cell.current,
            });
        }
    }

    let mut target_section_conditions = BTreeMap::new();
    for (sec_id, before_sec) in before.sections() {
        let after_sec = after
            .section(sec_id)
            .ok_or_else(|| ConditionError::MissingSection(sec_id.clone()))?;
        let target_cond = if let Some(profile) = anatomy {
            effective_section_condition_from_cells(after, sec_id, &after_sec.condition, profile)?
        } else {
            after_sec.condition.clone()
        };
        if let Some(classified) = SectionCondition::from_condition_id(&target_cond) {
            target_section_conditions.insert(sec_id.clone(), classified);
        } else {
            target_section_conditions.insert(
                sec_id.clone(),
                SectionCondition::classify_condition_id(&target_cond),
            );
        }
        if before_sec.condition != target_cond {
            ops.push(UnitMutationOp::SetSectionCondition {
                section: sec_id.clone(),
                condition: target_cond,
            });
        }
    }

    for (comp_id, before_comp) in before.components() {
        let after_comp = after
            .component(comp_id)
            .ok_or_else(|| ConditionError::MissingComponent(comp_id.clone()))?;
        let target_cond = if let Some(profile) = anatomy {
            let eff =
                effective_component_condition(after, comp_id, &target_section_conditions, profile)
                    .unwrap_or(ComponentCondition::Intact);
            eff.to_condition_id().map_err(ConditionError::Unit)?
        } else {
            after_comp.condition.clone()
        };
        if before_comp.condition != target_cond {
            ops.push(UnitMutationOp::SetComponentCondition {
                component: comp_id.clone(),
                condition: target_cond,
            });
        }
    }

    if ops.is_empty() {
        return Ok(None);
    }

    let plan =
        UnitMutationPlan::new(ops, Some(before.revision()), cause).map_err(ConditionError::Unit)?;
    Ok(Some(plan))
}

fn effective_section_condition_from_cells(
    unit: &TacticalUnit,
    section_id: &SectionId,
    raw_condition: &ConditionId,
    anatomy: &UnitAnatomyProfile,
) -> Result<ConditionId, ConditionError> {
    let raw = SectionCondition::classify_condition_id(raw_condition);
    if raw == SectionCondition::Irrecoverable {
        return SectionCondition::Irrecoverable
            .to_condition_id()
            .map_err(ConditionError::Unit);
    }
    if let Some(internals_id) = &anatomy.internals_cell {
        let addr = CellAddr::Section {
            section: section_id.clone(),
            cell: internals_id.clone(),
        };
        if let Some(cell) = unit.cell(&addr) {
            if cell.current <= cell.min {
                let destroyed_cond = if anatomy.core_sections.contains(section_id)
                    && raw == SectionCondition::Wreck
                {
                    SectionCondition::Wreck
                } else {
                    SectionCondition::Destroyed
                };
                return destroyed_cond
                    .to_condition_id()
                    .map_err(ConditionError::Unit);
            }
            if cell.current < cell.max {
                return SectionCondition::Damaged
                    .to_condition_id()
                    .map_err(ConditionError::Unit);
            }
            if raw == SectionCondition::Wreck {
                return SectionCondition::Intact
                    .to_condition_id()
                    .map_err(ConditionError::Unit);
            }
        }
    }
    Ok(raw_condition.clone())
}

/// Authority domain currently holding an IC field unit (`§4.4`, `FO-10`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldAuthorityKind {
    /// Stored in a home or forward hangar bay.
    Hangar {
        /// Base/hangar object id.
        base: ObjId,
    },
    /// Stowed in a mobile carrier bay (`Q-S8`).
    CarrierBay {
        /// Carrier unit/vessel object id.
        carrier: ObjId,
        /// Bay slot identifier.
        slot: SmolStr,
    },
    /// Present on the persistent field surface at a concrete site pose.
    FieldSurface {
        /// Surface site/region object id.
        site: ObjId,
        /// X coordinate in centimeters.
        x_cm: i64,
        /// Y coordinate in centimeters.
        y_cm: i64,
    },
    /// Checked out to an active IC combat instance at a field site.
    CombatInstance {
        /// Surface site/region object id.
        site: ObjId,
        /// Combat instance identifier.
        instance_id: u64,
    },
}

impl FieldAuthorityKind {
    fn validate(&self, unit: ObjId) -> Result<(), ConditionError> {
        match self {
            Self::Hangar { base } => {
                if base.0 == 0 || *base == unit {
                    return Err(ConditionError::InvalidBaseObject { unit, base: *base });
                }
            }
            Self::CarrierBay { carrier, slot } => {
                if carrier.0 == 0 || *carrier == unit {
                    return Err(ConditionError::InvalidBaseObject {
                        unit,
                        base: *carrier,
                    });
                }
                if slot.trim().is_empty() {
                    return Err(ConditionError::EmptySlotId);
                }
            }
            Self::FieldSurface { site, .. } => {
                if site.0 == 0 || *site == unit {
                    return Err(ConditionError::InvalidSiteObject { unit, site: *site });
                }
            }
            Self::CombatInstance { site, instance_id } => {
                if site.0 == 0 || *site == unit {
                    return Err(ConditionError::InvalidSiteObject { unit, site: *site });
                }
                if *instance_id == 0 {
                    return Err(ConditionError::ZeroCombatInstanceId);
                }
            }
        }
        Ok(())
    }
}

/// Versioned IC authority handoff record for one field unit (`§4.4`, `FO-1`, `FO-10`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "FieldCustodyRecordWire", into = "FieldCustodyRecordWire")]
pub struct FieldCustodyRecord {
    unit: ObjId,
    schema: DefinitionRef,
    authority_generation: u64,
    authority: FieldAuthorityKind,
    operational_state: OperationalState,
    epoch_disposition: FieldEpochDisposition,
    unit_revision: u64,
    sortie_id: Option<u64>,
    cargo_mass_kg: u64,
    cargo_volume_l: u64,
    updated_at_ic: i64,
}

impl FieldCustodyRecord {
    /// Unit object id.
    #[must_use]
    pub const fn unit(&self) -> ObjId {
        self.unit
    }

    /// Bound unit definition reference.
    #[must_use]
    pub const fn schema(&self) -> &DefinitionRef {
        &self.schema
    }

    /// Monotonic authority handoff generation (`>= 1`).
    #[must_use]
    pub const fn authority_generation(&self) -> u64 {
        self.authority_generation
    }

    /// Current authority holding the unit.
    #[must_use]
    pub const fn authority(&self) -> &FieldAuthorityKind {
        &self.authority
    }

    /// Current derived operational state.
    #[must_use]
    pub const fn operational_state(&self) -> OperationalState {
        self.operational_state
    }

    /// Current hull epoch disposition.
    #[must_use]
    pub const fn epoch_disposition(&self) -> FieldEpochDisposition {
        self.epoch_disposition
    }

    /// Committed `TacticalUnit::revision()` at the latest handoff.
    #[must_use]
    pub const fn unit_revision(&self) -> u64 {
        self.unit_revision
    }

    /// Active IC sortie identifier, if open.
    #[must_use]
    pub const fn sortie_id(&self) -> Option<u64> {
        self.sortie_id
    }

    /// Retained onboard cargo mass in kilograms (`FO-3`, `Q-S8`).
    #[must_use]
    pub const fn cargo_mass_kg(&self) -> u64 {
        self.cargo_mass_kg
    }

    /// Retained onboard cargo volume in liters (`FO-3`, `Q-S8`).
    #[must_use]
    pub const fn cargo_volume_l(&self) -> u64 {
        self.cargo_volume_l
    }

    /// IC timestamp of the latest authority handoff.
    #[must_use]
    pub const fn updated_at_ic(&self) -> i64 {
        self.updated_at_ic
    }

    fn validate(&self) -> Result<(), ConditionError> {
        if self.unit.0 == 0 {
            return Err(ConditionError::InvalidObjectId("unit"));
        }
        if self.authority_generation == 0 {
            return Err(ConditionError::ZeroAuthorityGeneration);
        }
        if self.sortie_id == Some(0) {
            return Err(ConditionError::ZeroSortieId);
        }
        if self.operational_state.epoch_disposition() != self.epoch_disposition {
            return Err(ConditionError::EpochDispositionMismatch {
                state: self.operational_state,
                disposition: self.epoch_disposition,
            });
        }
        self.authority.validate(self.unit)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldCustodyRecordWire {
    unit: ObjId,
    schema: DefinitionRef,
    authority_generation: u64,
    authority: FieldAuthorityKind,
    operational_state: OperationalState,
    epoch_disposition: FieldEpochDisposition,
    unit_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sortie_id: Option<u64>,
    cargo_mass_kg: u64,
    cargo_volume_l: u64,
    updated_at_ic: i64,
}

impl TryFrom<FieldCustodyRecordWire> for FieldCustodyRecord {
    type Error = ConditionError;

    fn try_from(wire: FieldCustodyRecordWire) -> Result<Self, Self::Error> {
        let record = Self {
            unit: wire.unit,
            schema: wire.schema,
            authority_generation: wire.authority_generation,
            authority: wire.authority,
            operational_state: wire.operational_state,
            epoch_disposition: wire.epoch_disposition,
            unit_revision: wire.unit_revision,
            sortie_id: wire.sortie_id,
            cargo_mass_kg: wire.cargo_mass_kg,
            cargo_volume_l: wire.cargo_volume_l,
            updated_at_ic: wire.updated_at_ic,
        };
        record.validate()?;
        Ok(record)
    }
}

impl From<FieldCustodyRecord> for FieldCustodyRecordWire {
    fn from(record: FieldCustodyRecord) -> Self {
        Self {
            unit: record.unit,
            schema: record.schema,
            authority_generation: record.authority_generation,
            authority: record.authority,
            operational_state: record.operational_state,
            epoch_disposition: record.epoch_disposition,
            unit_revision: record.unit_revision,
            sortie_id: record.sortie_id,
            cargo_mass_kg: record.cargo_mass_kg,
            cargo_volume_l: record.cargo_volume_l,
            updated_at_ic: record.updated_at_ic,
        }
    }
}

/// Parameters for an atomic IC authority handoff (`FieldCustodyLedger::handoff`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorityHandoffRequest {
    /// Target unit object id.
    pub unit: ObjId,
    /// Expected current authority generation (optimistic concurrency fence).
    pub expected_generation: u64,
    /// Next authority domain.
    pub next_authority: FieldAuthorityKind,
    /// Post-handoff operational state.
    pub operational_state: OperationalState,
    /// Post-handoff `TacticalUnit::revision()`.
    pub unit_revision: u64,
    /// Active sortie id (or `None` when closed in hangar/carrier bay).
    pub sortie_id: Option<u64>,
    /// Retained cargo mass in kilograms.
    pub cargo_mass_kg: u64,
    /// Retained cargo volume in liters.
    pub cargo_volume_l: u64,
    /// IC timestamp of the handoff.
    pub at_ic: i64,
}

/// Durable versioned ledger tracking IC field-unit custody and fencing against OOC
/// snapshot rollback (`§4.4`, `FO-10`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "FieldCustodyLedgerWire", into = "FieldCustodyLedgerWire")]
pub struct FieldCustodyLedger {
    records: BTreeMap<ObjId, FieldCustodyRecord>,
}

impl FieldCustodyLedger {
    /// Create an empty ledger.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            records: BTreeMap::new(),
        }
    }

    /// Number of tracked units.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the ledger is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Look up a unit's custody record.
    #[must_use]
    pub fn get(&self, unit: ObjId) -> Option<&FieldCustodyRecord> {
        self.records.get(&unit)
    }

    /// Iterate over all custody records in deterministic `ObjId` order.
    pub fn records(&self) -> impl Iterator<Item = (&ObjId, &FieldCustodyRecord)> {
        self.records.iter()
    }

    /// Register a unit in initial IC custody (`authority_generation = 1`).
    ///
    /// # Errors
    /// Returns [`ConditionError`] if the unit is already registered, the ledger is full,
    /// or the initial record fails validation.
    pub fn register_unit(
        &mut self,
        unit: &TacticalUnit,
        authority: FieldAuthorityKind,
        operational_state: OperationalState,
        sortie_id: Option<u64>,
        at_ic: i64,
    ) -> Result<&FieldCustodyRecord, ConditionError> {
        let obj = unit.object();
        if self.records.contains_key(&obj) {
            return Err(ConditionError::UnitAlreadyRegistered(obj));
        }
        if self.records.len() >= MAX_FIELD_CUSTODY_UNITS {
            return Err(ConditionError::CustodyLedgerFull);
        }
        let record = FieldCustodyRecord {
            unit: obj,
            schema: unit.schema().clone(),
            authority_generation: 1,
            authority,
            operational_state,
            epoch_disposition: operational_state.epoch_disposition(),
            unit_revision: unit.revision(),
            sortie_id,
            cargo_mass_kg: 0,
            cargo_volume_l: 0,
            updated_at_ic: at_ic,
        };
        record.validate()?;
        self.records.insert(obj, record);
        self.records
            .get(&obj)
            .ok_or(ConditionError::UnregisteredUnit(obj))
    }

    /// Atomically hand off authority for `req.unit` (`§4.4`).
    ///
    /// # Errors
    /// Returns [`ConditionError`] on stale generation, revision regression, or invalid target.
    pub fn handoff(
        &mut self,
        req: AuthorityHandoffRequest,
    ) -> Result<&FieldCustodyRecord, ConditionError> {
        let current = self
            .records
            .get(&req.unit)
            .ok_or(ConditionError::UnregisteredUnit(req.unit))?;

        if current.authority_generation != req.expected_generation {
            return Err(ConditionError::AuthorityGenerationMismatch {
                unit: req.unit,
                expected: req.expected_generation,
                actual: current.authority_generation,
            });
        }
        if req.unit_revision < current.unit_revision {
            return Err(ConditionError::UnitRevisionRegression {
                unit: req.unit,
                previous: current.unit_revision,
                proposed: req.unit_revision,
            });
        }
        if req.at_ic < current.updated_at_ic {
            return Err(ConditionError::TimestampRegression {
                previous: current.updated_at_ic,
                proposed: req.at_ic,
            });
        }
        if (current.operational_state == OperationalState::Irrecoverable
            || req.operational_state == OperationalState::Irrecoverable)
            && matches!(
                req.next_authority,
                FieldAuthorityKind::CombatInstance { .. }
            )
        {
            return Err(ConditionError::IrrecoverableUnitCannotEnterCombat(req.unit));
        }

        let next_gen = current
            .authority_generation
            .checked_add(1)
            .ok_or(ConditionError::AuthorityGenerationOverflow)?;

        let updated = FieldCustodyRecord {
            unit: req.unit,
            schema: current.schema.clone(),
            authority_generation: next_gen,
            authority: req.next_authority,
            operational_state: req.operational_state,
            epoch_disposition: req.operational_state.epoch_disposition(),
            unit_revision: req.unit_revision,
            sortie_id: req.sortie_id,
            cargo_mass_kg: req.cargo_mass_kg,
            cargo_volume_l: req.cargo_volume_l,
            updated_at_ic: req.at_ic,
        };
        updated.validate()?;
        self.records.insert(req.unit, updated);
        self.records
            .get(&req.unit)
            .ok_or(ConditionError::UnregisteredUnit(req.unit))
    }

    /// Guard that refuses OOC snapshot restoration (`custody::restore_at_boot`) on any unit
    /// tracked in IC field custody (`§4.4`, `FO-10`).
    ///
    /// # Errors
    /// Returns [`ConditionError::OocSnapshotRollbackForbidden`] if `unit` is under IC custody.
    pub fn refuse_ooc_snapshot_restore(&self, unit: ObjId) -> Result<(), ConditionError> {
        if let Some(rec) = self.records.get(&unit) {
            return Err(ConditionError::OocSnapshotRollbackForbidden {
                unit,
                authority_generation: rec.authority_generation,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldCustodyLedgerWire {
    records: Vec<FieldCustodyRecord>,
}

impl TryFrom<FieldCustodyLedgerWire> for FieldCustodyLedger {
    type Error = ConditionError;

    fn try_from(wire: FieldCustodyLedgerWire) -> Result<Self, Self::Error> {
        if wire.records.len() > MAX_FIELD_CUSTODY_UNITS {
            return Err(ConditionError::CustodyLedgerFull);
        }
        let mut records = BTreeMap::new();
        for rec in wire.records {
            let unit = rec.unit();
            if records.insert(unit, rec).is_some() {
                return Err(ConditionError::DuplicateCustodyRecord(unit));
            }
        }
        Ok(Self { records })
    }
}

impl From<FieldCustodyLedger> for FieldCustodyLedgerWire {
    fn from(ledger: FieldCustodyLedger) -> Self {
        Self {
            records: ledger.records.into_values().collect(),
        }
    }
}

/// Errors returned by `H1-A` condition assessment, IC mutation planning, and authority handoff.
#[derive(Clone, Debug, PartialEq)]
pub enum ConditionError {
    /// Underlying [`UnitError`] when building a [`ConditionId`] or [`UnitMutationPlan`].
    Unit(UnitError),
    /// Object id was `#0`.
    InvalidObjectId(&'static str),
    /// Base or carrier object id was `#0` or equal to the unit itself.
    InvalidBaseObject {
        /// Unit object id.
        unit: ObjId,
        /// Invalid base/carrier object id.
        base: ObjId,
    },
    /// Surface site object id was `#0` or equal to the unit itself.
    InvalidSiteObject {
        /// Unit object id.
        unit: ObjId,
        /// Invalid site object id.
        site: ObjId,
    },
    /// Carrier slot id was empty.
    EmptySlotId,
    /// Combat instance id was `0`.
    ZeroCombatInstanceId,
    /// Authority generation was `0`.
    ZeroAuthorityGeneration,
    /// Sortie id was `Some(0)`.
    ZeroSortieId,
    /// Authority generation overflowed `u64::MAX`.
    AuthorityGenerationOverflow,
    /// Recoverable unit (`Operational`, `Immobile`, `Unarmed`, `Disabled`, `Wreck`) cannot
    /// close its hull epoch as `Destroyed` (`Q-H9`).
    RecoverableUnitCannotCloseDestroyed {
        /// The unit's recoverable state.
        state: OperationalState,
    },
    /// Unit in `state` cannot close as `ReturnedToBase` under its own power.
    CannotReturnUnderOwnPower {
        /// The unit's non-mobile or defeated state.
        state: OperationalState,
    },
    /// `Irrecoverable` unit cannot be recovered to a base or carrier bay.
    IrrecoverableUnitCannotBeRecovered,
    /// `before` and `after` units have different `ObjId`s.
    UnitMismatch {
        /// `before.object()`.
        before: ObjId,
        /// `after.object()`.
        after: ObjId,
    },
    /// `before` and `after` units have different `DefinitionRef` schemas.
    SchemaMismatch,
    /// Cell present on `before` is missing on `after`.
    MissingCell(CellAddr),
    /// Section present on `before` is missing on `after`.
    MissingSection(SectionId),
    /// Component present on `before` is missing on `after`.
    MissingComponent(ComponentId),
    /// Stored epoch disposition does not match `operational_state.epoch_disposition()`.
    EpochDispositionMismatch {
        /// Operational state.
        state: OperationalState,
        /// Mismatched disposition.
        disposition: FieldEpochDisposition,
    },
    /// Unit is already registered in [`FieldCustodyLedger`].
    UnitAlreadyRegistered(ObjId),
    /// Unit is not registered in [`FieldCustodyLedger`].
    UnregisteredUnit(ObjId),
    /// Duplicate unit record in serialized [`FieldCustodyLedger`].
    DuplicateCustodyRecord(ObjId),
    /// [`FieldCustodyLedger`] reached [`MAX_FIELD_CUSTODY_UNITS`].
    CustodyLedgerFull,
    /// Optimistic authority generation fence failed.
    AuthorityGenerationMismatch {
        /// Unit object id.
        unit: ObjId,
        /// Expected generation.
        expected: u64,
        /// Actual generation.
        actual: u64,
    },
    /// Proposed handoff would regress `TacticalUnit::revision()` (snapshot rollback attempt).
    UnitRevisionRegression {
        /// Unit object id.
        unit: ObjId,
        /// Previously committed revision.
        previous: u64,
        /// Proposed lower revision.
        proposed: u64,
    },
    /// Proposed handoff timestamp regressed.
    TimestampRegression {
        /// Previous timestamp.
        previous: i64,
        /// Proposed earlier timestamp.
        proposed: i64,
    },
    /// An `Irrecoverable` unit cannot be checked out to an active combat instance.
    IrrecoverableUnitCannotEnterCombat(ObjId),
    /// OOC arena snapshot rollback is forbidden on an IC-custodied unit (`§4.4`, `FO-10`).
    OocSnapshotRollbackForbidden {
        /// Unit object id.
        unit: ObjId,
        /// Active IC authority generation.
        authority_generation: u64,
    },
}

impl fmt::Display for ConditionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit(e) => write!(f, "unit error: {e}"),
            Self::InvalidObjectId(field) => write!(f, "invalid zero object id for {field}"),
            Self::InvalidBaseObject { unit, base } => {
                write!(f, "invalid base/carrier object {base:?} for unit {unit:?}")
            }
            Self::InvalidSiteObject { unit, site } => {
                write!(f, "invalid site object {site:?} for unit {unit:?}")
            }
            Self::EmptySlotId => f.write_str("carrier bay slot id must not be empty"),
            Self::ZeroCombatInstanceId => f.write_str("combat instance id must be non-zero"),
            Self::ZeroAuthorityGeneration => f.write_str("authority generation must be non-zero"),
            Self::ZeroSortieId => f.write_str("sortie id must be non-zero when present"),
            Self::AuthorityGenerationOverflow => f.write_str("authority generation overflow"),
            Self::RecoverableUnitCannotCloseDestroyed { state } => {
                write!(
                    f,
                    "recoverable unit in state '{state}' cannot close its hull epoch as Destroyed"
                )
            }
            Self::CannotReturnUnderOwnPower { state } => {
                write!(
                    f,
                    "unit in state '{state}' cannot close as ReturnedToBase under its own power"
                )
            }
            Self::IrrecoverableUnitCannotBeRecovered => {
                f.write_str("irrecoverable unit cannot be recovered to base")
            }
            Self::UnitMismatch { before, after } => {
                write!(f, "unit mismatch: before {before:?} vs after {after:?}")
            }
            Self::SchemaMismatch => f.write_str("unit schema mismatch between before and after"),
            Self::MissingCell(addr) => write!(f, "missing cell in after unit: {addr:?}"),
            Self::MissingSection(sec) => write!(f, "missing section in after unit: {sec}"),
            Self::MissingComponent(comp) => write!(f, "missing component in after unit: {comp}"),
            Self::EpochDispositionMismatch { state, disposition } => {
                write!(
                    f,
                    "epoch disposition {disposition:?} does not match operational state {state}"
                )
            }
            Self::UnitAlreadyRegistered(u) => write!(f, "unit {u:?} is already registered"),
            Self::UnregisteredUnit(u) => write!(f, "unit {u:?} is not registered in field custody"),
            Self::DuplicateCustodyRecord(u) => write!(f, "duplicate custody record for {u:?}"),
            Self::CustodyLedgerFull => f.write_str("field custody ledger is full"),
            Self::AuthorityGenerationMismatch {
                unit,
                expected,
                actual,
            } => write!(
                f,
                "authority generation mismatch for {unit:?}: expected {expected}, actual {actual}"
            ),
            Self::UnitRevisionRegression {
                unit,
                previous,
                proposed,
            } => write!(
                f,
                "unit revision regression for {unit:?}: previous {previous}, proposed {proposed}"
            ),
            Self::TimestampRegression { previous, proposed } => {
                write!(
                    f,
                    "timestamp regression: previous {previous}, proposed {proposed}"
                )
            }
            Self::IrrecoverableUnitCannotEnterCombat(u) => {
                write!(f, "irrecoverable unit {u:?} cannot enter combat")
            }
            Self::OocSnapshotRollbackForbidden {
                unit,
                authority_generation,
            } => write!(
                f,
                "OOC snapshot rollback forbidden for IC unit {unit:?} at generation {authority_generation}"
            ),
        }
    }
}

impl std::error::Error for ConditionError {}

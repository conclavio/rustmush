//! Doc 27 (`H1-A` / `H2-A`) — `tacsim` integration for field-unit component
//! survival, operational state assessment, IC combat-return mutation plans,
//! hull-epoch close validation, and universal towing across stock unit schemas.

use std::collections::BTreeSet;
use std::fmt;

use mush_core::ObjId;
use mush_roll::commerce::ic_claims::IcCloseKind;
pub use mush_tactics::field_ops::{
    AttachTowRequest, AuthorityHandoffRequest, BayCapacitySpec, ComponentCondition, ConditionError,
    DEFAULT_MAX_ATTACH_DISTANCE_CM, FieldAuthorityKind, FieldCloseOutcome, FieldCustodyLedger,
    FieldCustodyRecord, FieldEpochDisposition, FieldInspectionReport, FieldPose,
    FieldUnitAssessment, LoadIntoBayRequest, LoadedCasualtyOutcome, MAX_FIELD_CUSTODY_UNITS,
    MAX_RECOVERY_OPERATIONS, MAX_TOW_MASS_RATIO_PERMILLE, OperationalState, RecoveryBook,
    RecoveryError, RecoveryOpId, RecoveryOperation, RecoveryStage, SectionCondition, TowEvaluation,
    TowRigProfile, TugInspectionContext, UnitAnatomyProfile, UnitMassProfile, assess_tactical_unit,
    effective_component_condition, effective_section_condition, ic_condition_plan,
    inspect_field_unit, scaled_tow_speed_cms, tow_mass_ratio_permille, tow_speed_factor_permille,
    validate_close_outcome,
};
use mush_tactics::{
    CellAddr, CellId, ComponentId, SectionId, TacticalUnit, UnitError, UnitMutationPlan, apply,
};
use smol_str::SmolStr;

use crate::arena::damage::{HULL_SECTION, POWER_CAPABILITY, UnitReturn, WEAPON_CAPABILITY_PREFIX};
use crate::definitions::unit::{
    AMMO_CELL, ComponentRole, CONDITION_CELL, Consequence, INTERNALS_CELL, UnitSpec,
};

/// Default dry-mass conversion factor (`kg` per maximum armor + internals point)
/// for combat units that do not declare a spacecraft `logistics.dry_mass_t` block.
pub const STRUCTURAL_POINT_MASS_KG: u64 = 10;

/// Fallback hull width (`cm`) when a unit spec declares no silhouettes.
pub const DEFAULT_UNIT_WIDTH_CM: u32 = 300;

/// Fallback hull height (`cm`) when a unit spec declares no silhouettes.
pub const DEFAULT_UNIT_HEIGHT_CM: u32 = 250;

/// Derive the [`UnitAnatomyProfile`] for an authored `tacsim` [`UnitSpec`] (`H1-A`).
///
/// # Errors
/// Returns [`FieldOpsError::Unit`] if any section, component, or cell id fails canonical
/// tactical-id validation.
pub fn anatomy_for_unit_spec(spec: &UnitSpec) -> Result<UnitAnatomyProfile, FieldOpsError> {
    let primary_body_id = spec
        .sections
        .iter()
        .find(|s| s.id.as_str() == HULL_SECTION)
        .or_else(|| spec.sections.first())
        .map(|s| s.id.as_str());

    let mut core_sections = BTreeSet::new();
    let mut control_sections = BTreeSet::new();
    let mut required_locomotion_sections = BTreeSet::new();
    let mut locomotion_sections = BTreeSet::new();

    for section in &spec.sections {
        let sec_id = SectionId::new(section.id.as_str()).map_err(FieldOpsError::Unit)?;
        let is_primary = primary_body_id == Some(section.id.as_str());

        for consequence in &section.consequences {
            match consequence {
                Consequence::DestroyUnit => {
                    if is_primary {
                        core_sections.insert(sec_id.clone());
                    } else {
                        control_sections.insert(sec_id.clone());
                    }
                }
                Consequence::DisableMotion => {
                    required_locomotion_sections.insert(sec_id.clone());
                }
                Consequence::DisableMotionWhenAllOf { sections } => {
                    for member in sections {
                        let member_id =
                            SectionId::new(member.as_str()).map_err(FieldOpsError::Unit)?;
                        locomotion_sections.insert(member_id);
                    }
                }
                Consequence::Slow { .. }
                | Consequence::NoTurnToward { .. }
                | Consequence::RemoveWeapons { .. }
                | Consequence::DegradeCore
                | Consequence::DestroyCore => {}
            }
        }
    }

    if core_sections.is_empty()
        && let Some(primary) = primary_body_id
    {
        core_sections.insert(SectionId::new(primary).map_err(FieldOpsError::Unit)?);
    }

    let min_locomotion_sections = usize::from(!locomotion_sections.is_empty());

    let mut power_components = BTreeSet::new();
    let mut weapon_components = BTreeSet::new();
    let mut sensor_components = BTreeSet::new();

    for component in &spec.components {
        let comp_id = ComponentId::new(component.id.as_str()).map_err(FieldOpsError::Unit)?;
        match &component.role {
            ComponentRole::Power { .. } => {
                power_components.insert(comp_id);
            }
            ComponentRole::Weapon { .. } => {
                weapon_components.insert(comp_id);
            }
            ComponentRole::Sensor { .. }
            | ComponentRole::Ecm { .. }
            | ComponentRole::Eccm { .. } => {
                sensor_components.insert(comp_id);
            }
            ComponentRole::Relay { .. } | ComponentRole::Core { .. } => {}
        }
    }

    Ok(UnitAnatomyProfile {
        core_sections,
        control_sections,
        required_locomotion_sections,
        locomotion_sections,
        min_locomotion_sections,
        power_components,
        weapon_components,
        sensor_components,
        internals_cell: Some(CellId::new(INTERNALS_CELL).map_err(FieldOpsError::Unit)?),
        component_health_cell: Some(CellId::new(CONDITION_CELL).map_err(FieldOpsError::Unit)?),
        weapon_ammo_cell: Some(CellId::new(AMMO_CELL).map_err(FieldOpsError::Unit)?),
    })
}

/// Derive a fallback [`UnitAnatomyProfile`] directly from a bound [`TacticalUnit`]
/// when the caller does not hold the authored [`UnitSpec`].
///
/// # Errors
/// Returns [`FieldOpsError::Unit`] if canonical cell ids fail construction.
pub fn anatomy_for_tactical_unit(unit: &TacticalUnit) -> Result<UnitAnatomyProfile, FieldOpsError> {
    let internals_cell = CellId::new(INTERNALS_CELL).map_err(FieldOpsError::Unit)?;
    let component_health_cell = CellId::new(CONDITION_CELL).map_err(FieldOpsError::Unit)?;
    let weapon_ammo_cell = CellId::new(AMMO_CELL).map_err(FieldOpsError::Unit)?;

    let mut core_sections = BTreeSet::new();
    let mut control_sections = BTreeSet::new();
    let mut required_locomotion_sections = BTreeSet::new();
    let mut locomotion_sections = BTreeSet::new();

    for (sec_id, _) in unit.sections() {
        match sec_id.as_str() {
            "hull" | "torso" | "chassis" | "body" => {
                core_sections.insert(sec_id.clone());
            }
            "head" | "cockpit" | "bridge" => {
                control_sections.insert(sec_id.clone());
            }
            "engine" | "drive" => {
                required_locomotion_sections.insert(sec_id.clone());
            }
            "left_track" | "right_track" | "tracks" | "left_leg" | "right_leg" | "wheels" => {
                locomotion_sections.insert(sec_id.clone());
            }
            _ => {}
        }
    }
    if core_sections.is_empty()
        && let Some((first_sec, _)) = unit.sections().next()
    {
        core_sections.insert(first_sec.clone());
    }

    let min_locomotion_sections = usize::from(!locomotion_sections.is_empty());

    let mut power_components = BTreeSet::new();
    let mut weapon_components = BTreeSet::new();
    let mut sensor_components = BTreeSet::new();

    for key in unit.capabilities() {
        if key.id.as_str() == POWER_CAPABILITY
            && let Some(src) = &key.source
        {
            power_components.insert(src.clone());
        } else if key.id.as_str().starts_with(WEAPON_CAPABILITY_PREFIX)
            && let Some(src) = &key.source
        {
            weapon_components.insert(src.clone());
        }
    }

    for (comp_id, _) in unit.components() {
        if comp_id.as_str() == "power" || comp_id.as_str() == "power_plant" {
            power_components.insert(comp_id.clone());
        }
        let has_ammo = unit
            .cell(&CellAddr::Component {
                component: comp_id.clone(),
                cell: weapon_ammo_cell.clone(),
            })
            .is_some();
        if has_ammo {
            weapon_components.insert(comp_id.clone());
        } else if unit
            .cell(&CellAddr::Component {
                component: comp_id.clone(),
                cell: component_health_cell.clone(),
            })
            .is_some()
            && !power_components.contains(comp_id)
        {
            sensor_components.insert(comp_id.clone());
        }
    }

    Ok(UnitAnatomyProfile {
        core_sections,
        control_sections,
        required_locomotion_sections,
        locomotion_sections,
        min_locomotion_sections,
        power_components,
        weapon_components,
        sensor_components,
        internals_cell: Some(internals_cell),
        component_health_cell: Some(component_health_cell),
        weapon_ammo_cell: Some(weapon_ammo_cell),
    })
}

/// Assess a `tacsim` [`TacticalUnit`] into its [`FieldUnitAssessment`] (`H1-A`).
///
/// # Errors
/// Returns [`FieldOpsError`] if anatomy profile construction fails.
pub fn assess_tacsim_unit(
    unit: &TacticalUnit,
    spec: Option<&UnitSpec>,
) -> Result<FieldUnitAssessment, FieldOpsError> {
    let anatomy = match spec {
        Some(s) => anatomy_for_unit_spec(s)?,
        None => anatomy_for_tactical_unit(unit)?,
    };
    Ok(assess_tactical_unit(unit, &anatomy))
}

/// Build an IC-custody-legal [`UnitMutationPlan`] between `before` and `after` for a
/// `tacsim` unit (`H1-A`, `Q-H8`).
///
/// # Errors
/// Returns [`FieldOpsError`] if anatomy derivation or plan construction fails.
pub fn tacsim_ic_condition_plan(
    before: &TacticalUnit,
    after: &TacticalUnit,
    spec: Option<&UnitSpec>,
    cause: Option<SmolStr>,
) -> Result<Option<UnitMutationPlan>, FieldOpsError> {
    let anatomy = match spec {
        Some(s) => anatomy_for_unit_spec(s)?,
        None => anatomy_for_tactical_unit(before)?,
    };
    ic_condition_plan(before, after, Some(&anatomy), cause).map_err(FieldOpsError::Condition)
}

/// Apply post-combat `after_cells` onto the pre-engagement `before` [`TacticalUnit`]
/// using the IC-custody-legal condition vocabulary (`H1-A`, `FO-1`, `FO-2`, `Q-H8`).
///
/// # Errors
/// Returns [`FieldOpsError`] if `before.object() != after_cells.object()` or plan application fails.
pub fn apply_ic_condition_handoff(
    before: &TacticalUnit,
    after_cells: &TacticalUnit,
    spec: Option<&UnitSpec>,
    at_ic: i64,
) -> Result<(TacticalUnit, FieldUnitAssessment, Option<UnitMutationPlan>), FieldOpsError> {
    let anatomy = match spec {
        Some(s) => anatomy_for_unit_spec(s)?,
        None => anatomy_for_tactical_unit(before)?,
    };
    let plan_opt = ic_condition_plan(
        before,
        after_cells,
        Some(&anatomy),
        Some(SmolStr::new("ic_combat_return")),
    )
    .map_err(FieldOpsError::Condition)?;

    let post_unit = match &plan_opt {
        Some(plan) => {
            apply(before, plan, at_ic)
                .map_err(FieldOpsError::Unit)?
                .0
        }
        None => before.clone(),
    };
    let assessment = assess_tactical_unit(&post_unit, &anatomy);
    Ok((post_unit, assessment, plan_opt))
}

/// Apply an arena [`UnitReturn`] onto the pre-engagement `before` [`TacticalUnit`] using
/// the IC-custody-legal condition vocabulary (`H1-A`, `FO-1`, `FO-2`, `Q-H8`).
///
/// # Errors
/// Returns [`FieldOpsError`] if `before.object() != unit_return.unit` or plan application fails.
pub fn apply_ic_combat_return(
    before: &TacticalUnit,
    unit_return: &UnitReturn,
    spec: Option<&UnitSpec>,
    at_ic: i64,
) -> Result<(TacticalUnit, FieldUnitAssessment, Option<UnitMutationPlan>), FieldOpsError> {
    apply_ic_condition_handoff(before, &unit_return.cells, spec, at_ic)
}

/// Convert a neutral [`FieldCloseOutcome`] into `mush-roll`'s [`IcCloseKind`].
#[must_use]
pub const fn to_ic_close_kind(outcome: FieldCloseOutcome) -> IcCloseKind {
    match outcome {
        FieldCloseOutcome::ReturnedToBase { base } => IcCloseKind::ReturnedToBase { base },
        FieldCloseOutcome::PoweredDownInBase { base } => IcCloseKind::PoweredDownInBase { base },
        FieldCloseOutcome::RecoveredToBase { base } => IcCloseKind::RecoveredToBase { base },
        FieldCloseOutcome::Destroyed => IcCloseKind::Destroyed,
    }
}

/// Convert `mush-roll`'s [`IcCloseKind`] into a neutral [`FieldCloseOutcome`].
#[must_use]
pub const fn from_ic_close_kind(kind: IcCloseKind) -> FieldCloseOutcome {
    match kind {
        IcCloseKind::ReturnedToBase { base } => FieldCloseOutcome::ReturnedToBase { base },
        IcCloseKind::PoweredDownInBase { base } => FieldCloseOutcome::PoweredDownInBase { base },
        IcCloseKind::RecoveredToBase { base } => FieldCloseOutcome::RecoveredToBase { base },
        IcCloseKind::Destroyed => FieldCloseOutcome::Destroyed,
    }
}

/// Validate that an [`IcCloseKind`] is legal for `unit`'s derived [`OperationalState`] (`Q-H9`).
///
/// # Errors
/// Returns [`FieldOpsError::Condition`] if the close kind violates `Q-H9`.
pub fn validate_ic_close_for_unit(
    unit: ObjId,
    state: OperationalState,
    close_kind: IcCloseKind,
) -> Result<(), FieldOpsError> {
    let outcome = from_ic_close_kind(close_kind);
    validate_close_outcome(unit, state, &outcome).map_err(FieldOpsError::Condition)
}

/// Compute the deterministic dry mass in kilograms for a `tacsim` [`UnitSpec`] (`FO-13`, `Q-S6`).
#[must_use]
pub fn dry_mass_kg_for_spec(spec: &UnitSpec) -> u64 {
    if let Some(logistics) = &spec.logistics {
        return u64::from(logistics.dry_mass_t).saturating_mul(1_000).max(1);
    }
    let mut points: u64 = 0;
    for section in &spec.sections {
        points = points.saturating_add(u64::from(section.internals));
        for facet in &section.facets {
            points = points.saturating_add(u64::from(facet.armor));
        }
    }
    points.saturating_mul(STRUCTURAL_POINT_MASS_KG).max(1)
}

/// Extract the bow width (`width_cm`) and maximum horizontal silhouette height (`height_cm`)
/// for a `tacsim` [`UnitSpec`] (`Q-S8`).
#[must_use]
pub fn envelope_cm_for_spec(spec: &UnitSpec) -> (u32, u32) {
    let width_cm = spec
        .silhouettes
        .iter()
        .find(|s| s.band.as_str() == "front")
        .or_else(|| spec.silhouettes.first())
        .map_or(DEFAULT_UNIT_WIDTH_CM, |s| s.width_cm.max(1));

    let height_cm = spec
        .silhouettes
        .iter()
        .filter(|s| s.band.as_str() != "top")
        .map(|s| s.height_cm)
        .max()
        .or_else(|| spec.silhouettes.first().map(|s| s.height_cm))
        .unwrap_or(DEFAULT_UNIT_HEIGHT_CM)
        .max(1);

    (width_cm, height_cm)
}

/// Build a [`UnitMassProfile`] for a `tacsim` [`UnitSpec`] with retained `cargo_mass_kg`
/// and `cargo_volume_l` (`FO-3`, `FO-13`, `Q-S8`).
///
/// # Errors
/// Returns [`FieldOpsError::Recovery`] if mass addition overflows.
pub fn unit_mass_profile_for_spec(
    spec: &UnitSpec,
    cargo_mass_kg: u64,
    cargo_volume_l: u64,
) -> Result<UnitMassProfile, FieldOpsError> {
    let dry_mass_kg = dry_mass_kg_for_spec(spec);
    let (width_cm, height_cm) = envelope_cm_for_spec(spec);
    UnitMassProfile::new(
        dry_mass_kg,
        cargo_mass_kg,
        cargo_volume_l,
        width_cm,
        height_cm,
    )
    .map_err(FieldOpsError::Recovery)
}

/// Read the uncoupled forward base speed (`max_speed_cms`) of a `tacsim` [`UnitSpec`].
#[must_use]
pub fn base_speed_cms_for_spec(spec: &UnitSpec) -> u32 {
    spec.mobility
        .modes()
        .first()
        .map_or(0, |mode| mode.rules.max_speed_cms)
}

/// Errors returned by `tacsim::field_ops`.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldOpsError {
    /// Underlying [`UnitError`].
    Unit(UnitError),
    /// Underlying [`ConditionError`].
    Condition(ConditionError),
    /// Underlying [`RecoveryError`].
    Recovery(RecoveryError),
}

impl fmt::Display for FieldOpsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit(e) => write!(f, "tactical unit error: {e}"),
            Self::Condition(e) => write!(f, "field condition error: {e}"),
            Self::Recovery(e) => write!(f, "field recovery error: {e}"),
        }
    }
}

impl std::error::Error for FieldOpsError {}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use mush_core::{CanonicalId, ObjId};
    use mush_roll::commerce::ic_claims::IcCloseKind;
    use mush_tactics::{
        CapabilityId, CapabilityKey, CellAddr, CellId, ComponentId, ConditionId, SectionId,
        TacticalUnit, UnitMutationOp, UnitMutationPlan, apply,
    };
    use smol_str::SmolStr;

    use super::*;
    use crate::definitions::Catalog;

    type TestResult = Result<(), Box<dyn Error>>;

    fn load_stock_catalog() -> Result<Catalog, Box<dyn Error>> {
        Ok(Catalog::build_fielded(&[], "tacsim")?)
    }

    fn instantiate_stock_unit(
        catalog: &Catalog,
        unit_entry_id: &str,
        obj: ObjId,
    ) -> Result<(TacticalUnit, UnitSpec), Box<dyn Error>> {
        let key = CanonicalId::new(unit_entry_id)?;
        let spec = catalog
            .specs
            .get(&key)
            .ok_or_else(|| format!("missing unit spec {unit_entry_id}"))?
            .clone();
        let binding = catalog
            .bindings
            .get(&key)
            .ok_or_else(|| format!("missing unit binding {unit_entry_id}"))?;
        let unit = TacticalUnit::from_spec(&binding.spec, obj)?;
        Ok((unit, spec))
    }

    #[test]
    fn stock_biped_mk1_headshot_preserves_torso_and_arm_weapons_for_recovery() -> TestResult {
        let catalog = load_stock_catalog()?;
        let (before, spec) = instantiate_stock_unit(&catalog, "tacsim.unit.biped_mk1", ObjId(301))?;

        let fresh = assess_tacsim_unit(&before, Some(&spec))?;
        check_eq!(fresh.state, OperationalState::Operational);
        check!(fresh.can_move);
        check!(fresh.can_fire);
        check_eq!(fresh.intact_components.len(), 5);

        let head = SectionId::new("head")?;
        let torso = SectionId::new("torso")?;
        let internals = CellId::new("internals")?;
        let cannon = ComponentId::new("cannon")?;
        let machine_gun = ComponentId::new("machine_gun")?;
        let power = ComponentId::new("power")?;
        let core = ComponentId::new("core")?;
        let visual_sensor = ComponentId::new("visual_sensor")?;
        let ammo = CellId::new("ammo")?;

        let cannon_ammo_before = before
            .cell(&CellAddr::Component {
                component: cannon.clone(),
                cell: ammo.clone(),
            })
            .ok_or("missing cannon ammo")?
            .current;

        let arena_ops = vec![
            UnitMutationOp::SetCell {
                addr: CellAddr::Section {
                    section: head.clone(),
                    cell: internals,
                },
                value: 0,
            },
            UnitMutationOp::SetSectionCondition {
                section: head.clone(),
                condition: ConditionId::new("destroyed")?,
            },
            UnitMutationOp::SetCell {
                addr: CellAddr::Component {
                    component: cannon.clone(),
                    cell: ammo.clone(),
                },
                value: cannon_ammo_before - 2,
            },
            UnitMutationOp::SetSectionCondition {
                section: torso.clone(),
                condition: ConditionId::new("wreck")?,
            },
            UnitMutationOp::RevokeCapability {
                key: CapabilityKey {
                    id: CapabilityId::new("tacsim.power")?,
                    source: Some(power.clone()),
                },
            },
            UnitMutationOp::RevokeCapability {
                key: CapabilityKey {
                    id: CapabilityId::new("tactics.mobility")?,
                    source: None,
                },
            },
        ];
        let arena_plan = UnitMutationPlan::new(arena_ops, Some(before.revision()), None)?;
        let (arena_cells, _) = apply(&before, &arena_plan, 1_000)?;

        let (ic_unit, assessment, ic_plan_opt) =
            apply_ic_condition_handoff(&before, &arena_cells, Some(&spec), 1_010)?;
        check!(ic_plan_opt.is_some());

        check_eq!(assessment.state, OperationalState::Disabled);
        check!(assessment.is_towable);
        check_eq!(
            assessment.epoch_disposition,
            FieldEpochDisposition::RemainOpen
        );

        check_eq!(
            assessment.section_conditions.get(&head),
            Some(&SectionCondition::Destroyed)
        );
        check_eq!(
            assessment.component_conditions.get(&visual_sensor),
            Some(&ComponentCondition::Destroyed)
        );

        check_eq!(
            assessment.section_conditions.get(&torso),
            Some(&SectionCondition::Intact)
        );
        for surviving in [&cannon, &machine_gun, &power, &core] {
            check_eq!(
                assessment.component_conditions.get(surviving),
                Some(&ComponentCondition::Intact),
                "expected {surviving} to survive intact on headshot biped_mk1"
            );
        }

        let cannon_ammo_after = ic_unit
            .cell(&CellAddr::Component {
                component: cannon,
                cell: ammo,
            })
            .ok_or("missing cannon ammo after")?
            .current;
        check_eq!(cannon_ammo_after, cannon_ammo_before - 2);

        check!(
            validate_ic_close_for_unit(
                ic_unit.object(),
                assessment.state,
                IcCloseKind::Destroyed
            )
            .is_err()
        );
        check!(
            validate_ic_close_for_unit(
                ic_unit.object(),
                assessment.state,
                IcCloseKind::RecoveredToBase { base: ObjId(999) }
            )
            .is_ok()
        );

        Ok(())
    }

    #[test]
    fn stock_line_tank_distinguishes_engine_immobile_track_redundancy_and_turret_unarmed()
    -> TestResult {
        let catalog = load_stock_catalog()?;
        let (before, spec) = instantiate_stock_unit(&catalog, "tacsim.unit.line_tank", ObjId(302))?;

        let internals = CellId::new("internals")?;
        let engine = SectionId::new("engine")?;
        let left_track = SectionId::new("left_track")?;
        let right_track = SectionId::new("right_track")?;
        let turret = SectionId::new("turret")?;
        let cannon = ComponentId::new("cannon")?;
        let machine_gun = ComponentId::new("machine_gun")?;
        let power = ComponentId::new("power")?;

        let engine_kill = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: engine.clone(),
                        cell: internals.clone(),
                    },
                    value: 0,
                },
                UnitMutationOp::SetSectionCondition {
                    section: engine,
                    condition: ConditionId::new("destroyed")?,
                },
            ],
            Some(before.revision()),
            None,
        )?;
        let (engine_dead_tank, _) = apply(&before, &engine_kill, 100)?;
        let engine_assess = assess_tacsim_unit(&engine_dead_tank, Some(&spec))?;
        check_eq!(engine_assess.state, OperationalState::Immobile);
        check!(!engine_assess.can_move);
        check!(engine_assess.can_fire);
        check!(engine_assess.is_towable);
        check_eq!(
            engine_assess.component_conditions.get(&cannon),
            Some(&ComponentCondition::Intact)
        );
        check_eq!(
            engine_assess.component_conditions.get(&machine_gun),
            Some(&ComponentCondition::Intact)
        );

        let one_track_kill = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: left_track.clone(),
                        cell: internals.clone(),
                    },
                    value: 0,
                },
                UnitMutationOp::SetSectionCondition {
                    section: left_track,
                    condition: ConditionId::new("destroyed")?,
                },
            ],
            Some(before.revision()),
            None,
        )?;
        let (one_track_tank, _) = apply(&before, &one_track_kill, 200)?;
        let one_track_assess = assess_tacsim_unit(&one_track_tank, Some(&spec))?;
        check_eq!(one_track_assess.state, OperationalState::Operational);
        check!(one_track_assess.can_move);

        let second_track_kill = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: right_track.clone(),
                        cell: internals.clone(),
                    },
                    value: 0,
                },
                UnitMutationOp::SetSectionCondition {
                    section: right_track,
                    condition: ConditionId::new("destroyed")?,
                },
            ],
            Some(one_track_tank.revision()),
            None,
        )?;
        let (both_tracks_tank, _) = apply(&one_track_tank, &second_track_kill, 210)?;
        let both_tracks_assess = assess_tacsim_unit(&both_tracks_tank, Some(&spec))?;
        check_eq!(both_tracks_assess.state, OperationalState::Immobile);
        check!(!both_tracks_assess.can_move);
        check!(both_tracks_assess.can_fire);

        let turret_kill = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: turret.clone(),
                        cell: internals,
                    },
                    value: 0,
                },
                UnitMutationOp::SetSectionCondition {
                    section: turret,
                    condition: ConditionId::new("destroyed")?,
                },
            ],
            Some(before.revision()),
            None,
        )?;
        let (turretless_tank, _) = apply(&before, &turret_kill, 300)?;
        let turret_assess = assess_tacsim_unit(&turretless_tank, Some(&spec))?;
        check_eq!(turret_assess.state, OperationalState::Unarmed);
        check!(turret_assess.can_move);
        check!(!turret_assess.can_fire);
        check!(turret_assess.is_towable);
        check_eq!(
            turret_assess.component_conditions.get(&power),
            Some(&ComponentCondition::Intact)
        );
        check_eq!(
            turret_assess.component_conditions.get(&cannon),
            Some(&ComponentCondition::Destroyed)
        );

        Ok(())
    }

    #[test]
    fn stock_units_universal_towing_tonnage_cap_and_carrier_bay_recovery() -> TestResult {
        let catalog = load_stock_catalog()?;
        let (tank_unit, tank_spec) =
            instantiate_stock_unit(&catalog, "tacsim.unit.line_tank", ObjId(401))?;
        let (biped_before, biped_spec) =
            instantiate_stock_unit(&catalog, "tacsim.unit.biped_mk1", ObjId(402))?;
        let (_prospector_unit, prospector_spec) =
            instantiate_stock_unit(&catalog, "tacsim.unit.tv_prospector", ObjId(403))?;

        let tank_mass = unit_mass_profile_for_spec(&tank_spec, 0, 0)?;
        let biped_mass = unit_mass_profile_for_spec(&biped_spec, 0, 0)?;
        let prospector_empty_mass = unit_mass_profile_for_spec(&prospector_spec, 0, 0)?;
        let prospector_heavy_ore_mass = unit_mass_profile_for_spec(&prospector_spec, 8_000, 8_800)?;

        check_eq!(tank_mass.total_mass_kg(), 29_900);
        check_eq!(biped_mass.total_mass_kg(), 50_900);
        check_eq!(prospector_empty_mass.total_mass_kg(), 6_000);
        check_eq!(prospector_heavy_ore_mass.total_mass_kg(), 14_000);

        check!(
            scaled_tow_speed_cms(
                base_speed_cms_for_spec(&prospector_spec),
                prospector_empty_mass.total_mass_kg(),
                prospector_empty_mass.total_mass_kg(),
                TowRigProfile::Standard,
            )
            .is_ok()
        );
        check!(
            scaled_tow_speed_cms(
                base_speed_cms_for_spec(&prospector_spec),
                prospector_empty_mass.total_mass_kg(),
                prospector_heavy_ore_mass.total_mass_kg(),
                TowRigProfile::Standard,
            )
            .is_err()
        );

        let head = SectionId::new("head")?;
        let internals = CellId::new("internals")?;
        let headshot_plan = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: head.clone(),
                        cell: internals,
                    },
                    value: 0,
                },
                UnitMutationOp::SetSectionCondition {
                    section: head,
                    condition: ConditionId::new("destroyed")?,
                },
            ],
            Some(biped_before.revision()),
            None,
        )?;
        let (disabled_biped, _) = apply(&biped_before, &headshot_plan, 500)?;
        let biped_assess = assess_tacsim_unit(&disabled_biped, Some(&biped_spec))?;
        let tank_assess = assess_tacsim_unit(&tank_unit, Some(&tank_spec))?;
        check_eq!(biped_assess.state, OperationalState::Disabled);

        let site = ObjId(750);
        let pose = FieldPose {
            site,
            x_cm: 0,
            y_cm: 0,
        };
        let mut book = RecoveryBook::new();
        let op = book.attach(AttachTowRequest {
            tug: tank_unit.object(),
            tug_state: tank_assess.state,
            tug_can_move: tank_assess.can_move,
            tug_base_speed_cms: base_speed_cms_for_spec(&tank_spec),
            tug_mass_kg: tank_mass.total_mass_kg(),
            rig_profile: TowRigProfile::Standard,
            tug_pose: pose,
            casualty_assessment: biped_assess,
            casualty_mass: biped_mass,
            casualty_pose: pose,
            max_attach_distance_cm: DEFAULT_MAX_ATTACH_DISTANCE_CM,
            at_ic: 1_000,
        })?;
        let op_id = op.id();
        check_eq!(op.coupled_speed_cms(), 510);

        let bay_pose = FieldPose {
            site,
            x_cm: 2_000,
            y_cm: 0,
        };
        book.advance_tow(op_id, 1, bay_pose, 5, 1_005)?;

        let small_vehicle_bay = BayCapacitySpec {
            max_units: 2,
            current_units: 0,
            max_mass_kg: 100_000,
            current_mass_kg: 0,
            max_volume_l: 20_000,
            current_volume_l: 0,
            max_width_cm: 500,
            max_height_cm: 500,
        };
        let dim_err = book.load_into_bay(LoadIntoBayRequest {
            id: op_id,
            expected_revision: 2,
            carrier: ObjId(900),
            slot: SmolStr::new("vehicle_bay_small"),
            carrier_pose: bay_pose,
            max_load_distance_cm: DEFAULT_MAX_ATTACH_DISTANCE_CM,
            bay_capacity: &small_vehicle_bay,
            casualty_unit: &disabled_biped,
            at_ic: 1_008,
        });
        check!(matches!(
            dim_err,
            Err(RecoveryError::BayDimensionExceeded {
                unit_width_cm: 520,
                unit_height_cm: 1_100,
                max_width_cm: 500,
                max_height_cm: 500,
            })
        ));

        let carrier_bay = BayCapacitySpec {
            max_units: 2,
            current_units: 0,
            max_mass_kg: 100_000,
            current_mass_kg: 0,
            max_volume_l: 20_000,
            current_volume_l: 0,
            max_width_cm: 800,
            max_height_cm: 1_500,
        };
        let loaded = book.load_into_bay(LoadIntoBayRequest {
            id: op_id,
            expected_revision: 2,
            carrier: ObjId(900),
            slot: SmolStr::new("mech_bay_alpha"),
            carrier_pose: bay_pose,
            max_load_distance_cm: DEFAULT_MAX_ATTACH_DISTANCE_CM,
            bay_capacity: &carrier_bay,
            casualty_unit: &disabled_biped,
            at_ic: 1_010,
        })?;
        check_eq!(loaded.preserved_state, OperationalState::Disabled);
        check_eq!(loaded.preserved_unit_revision, disabled_biped.revision());
        check_eq!(
            to_ic_close_kind(loaded.close_outcome),
            IcCloseKind::RecoveredToBase { base: ObjId(900) }
        );

        Ok(())
    }
}

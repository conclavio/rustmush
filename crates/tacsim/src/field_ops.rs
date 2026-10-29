//! Doc 27 (`H1-A`–`H1-C`, `H2-A`–`H2-C`, `H3`) — `tacsim` integration for field-unit
//! component survival, operational state assessment, IC combat-return mutation plans,
//! hull-epoch close validation, field combat promotion/demotion & crew survival,
//! universal towing, recovery entitlement & 48-hour salvage claims, and workshop
//! component stripping, scrapping, and restoration across stock unit schemas.

use std::collections::BTreeSet;
use std::fmt;

use mush_core::ObjId;
use mush_roll::commerce::ic_claims::IcCloseKind;
pub use mush_tactics::field_ops::{
    AtRiskRepairEntry, AttachTowRequest, AuthoritativeRecoveryRecord, AuthorityHandoffRequest,
    BayCapacitySpec, COMBAT_WALL_CLOCK_RATE, CONTEST_WINDOW_AVAILABLE_REAL_SECS,
    CarrierDepartureOutcome, ComponentCondition, ComponentProvenance, ConditionError,
    ContestGround, ContestSuccessReason, CrewSurvivalBook, CustodyAction, CustodyTitleStatus,
    DEFAULT_FALLBACK_SAFE_PORT, DEFAULT_MAX_ATTACH_DISTANCE_CM, DemotionOutcome,
    DemotionSettlementGate, EngagementError, FALLBACK_SECTION_SCRAP_KG, FieldAuthorityKind,
    FieldCloseOutcome, FieldCustodyLedger, FieldCustodyRecord, FieldEpochDisposition,
    FieldInspectionReport, FieldObservationBook, FieldPose, FieldPromotionTrigger,
    FieldSiteEngagementBook, FieldUnitAssessment, GroundTenure, IC_SURFACE_CLOCK_RATE,
    LinkLossPosture, LoadIntoBayRequest, LoadedCasualtyOutcome, MAX_AUTHORITATIVE_RECOVERY_RECORDS,
    MAX_CLAIM_ENTRIES, MAX_CREW_RECORDS, MAX_FIELD_CUSTODY_UNITS, MAX_OBSERVATIONS_PER_ACTOR,
    MAX_RECOVERY_OPERATIONS, MAX_SALVAGE_CLAIMS, MAX_SITE_UNITS, MAX_STRIPPED_COMPONENTS,
    MAX_TOW_MASS_RATIO_PERMILLE, MAX_WORKSHOP_ORDERS, MotionClockHandoff, ObservationSource,
    ObservedCombatFact, ObserverCombatEntry, OccupantSurvivalState, OperationalState,
    RecoveryAuthorityContext, RecoveryBook, RecoveryEntitlement, RecoveryError, RecoveryOpId,
    RecoveryOperation, RecoveryStage, ReplacementPartSpec, SCRAP_KG_PER_STRUCTURAL_POINT,
    SafePortCandidate, SalvageClaim, SalvageClaimBook, SalvageClaimId, SalvageClaimState,
    SalvageContest, SalvageError, SalvageEvidenceKind, SalvageEvidenceRecord, SalvageSubject,
    ScrapYieldReport, SectionCondition, SiteUnitPresence, StripAuthorization,
    StrippedComponentRecord, TowEvaluation, TowRigProfile, TugInspectionContext,
    UnitAnatomyProfile, UnitMassProfile, WithdrawalDestination, WorkshopBook, WorkshopError,
    WorkshopOperationKind, WorkshopWorkOrderRecord, assess_tactical_unit,
    check_custody_action_allowed, effective_component_condition, effective_section_condition,
    evaluate_recovery_entitlement, ic_condition_plan, inspect_field_unit,
    resolve_link_loss_posture, scaled_tow_speed_cms, select_extraction_port,
    tow_mass_ratio_permille, tow_speed_factor_permille, validate_close_outcome,
};
use mush_tactics::{
    CellAddr, CellId, ComponentId, SectionId, TacticalUnit, UnitError, UnitMutationPlan, apply,
};
use smol_str::SmolStr;

use crate::arena::damage::{HULL_SECTION, POWER_CAPABILITY, UnitReturn, WEAPON_CAPABILITY_PREFIX};
use crate::definitions::unit::{
    AMMO_CELL, CONDITION_CELL, ComponentRole, Consequence, INTERNALS_CELL, UnitSpec,
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
        Some(plan) => apply(before, plan, at_ic).map_err(FieldOpsError::Unit)?.0,
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
    /// Underlying [`EngagementError`].
    Engagement(EngagementError),
    /// Underlying [`RecoveryError`].
    Recovery(RecoveryError),
    /// Underlying [`SalvageError`].
    Salvage(SalvageError),
    /// Underlying [`WorkshopError`].
    Workshop(WorkshopError),
}

impl fmt::Display for FieldOpsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit(e) => write!(f, "tactical unit error: {e}"),
            Self::Condition(e) => write!(f, "field condition error: {e}"),
            Self::Engagement(e) => write!(f, "field engagement error: {e}"),
            Self::Recovery(e) => write!(f, "field recovery error: {e}"),
            Self::Salvage(e) => write!(f, "field salvage error: {e}"),
            Self::Workshop(e) => write!(f, "field workshop error: {e}"),
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
            validate_ic_close_for_unit(ic_unit.object(), assessment.state, IcCloseKind::Destroyed)
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

    #[test]
    fn h1b_h1c_sortie_engagement_clock_handoff_crew_extraction_and_persistent_casualty()
    -> TestResult {
        let catalog = load_stock_catalog()?;
        let (biped_before, biped_spec) =
            instantiate_stock_unit(&catalog, "tacsim.unit.biped_mk1", ObjId(501))?;
        let (tank_enemy, tank_spec) =
            instantiate_stock_unit(&catalog, "tacsim.unit.line_tank", ObjId(502))?;
        let (tank_reinforce, _) =
            instantiate_stock_unit(&catalog, "tacsim.unit.line_tank", ObjId(503))?;

        let biped_anatomy = anatomy_for_unit_spec(&biped_spec)?;
        let tank_anatomy = anatomy_for_unit_spec(&tank_spec)?;

        let site = ObjId(770);
        let carrier = ObjId(910);
        let pilot = ObjId(810);

        let mut ledger = FieldCustodyLedger::new();
        let mut site_book = FieldSiteEngagementBook::new(site)?;
        let mut crew_book = CrewSurvivalBook::new();
        let mut obs_book = FieldObservationBook::new();

        crew_book.board_unit(pilot, biped_before.object())?;

        let pose_biped = FieldPose {
            site,
            x_cm: 1_000,
            y_cm: 1_000,
        };
        let pose_enemy = FieldPose {
            site,
            x_cm: 3_000,
            y_cm: 1_000,
        };

        site_book.deploy_surface_unit(
            &mut ledger,
            &biped_before,
            &biped_anatomy,
            SmolStr::new("expedition"),
            pose_biped,
            Some(carrier),
            1_000,
        )?;
        site_book.deploy_surface_unit(
            &mut ledger,
            &tank_enemy,
            &tank_anatomy,
            SmolStr::new("raiders"),
            pose_enemy,
            None,
            1_000,
        )?;

        // Surface drive on the rate-24 IC domain clock before combat promotion (`Q-R3`).
        let engaged_pose_biped = FieldPose {
            site,
            x_cm: 1_800,
            y_cm: 1_000,
        };
        let moved_cm =
            site_book.record_surface_motion(biped_before.object(), engaged_pose_biped, None)?;
        check_eq!(moved_cm, 800);

        // Promote site to wall-clock combat (`1:1`) on explicit hostile action (`Q-H1`, `Q-H2`, `Q-R3`).
        let promo_handoffs = site_book.promote_to_engagement(
            &mut ledger,
            99,
            FieldPromotionTrigger::ExplicitHostileAction {
                attacker: tank_enemy.object(),
                target: biped_before.object(),
            },
            1,
            1_020,
        )?;
        check_eq!(site_book.current_clock_rate(), COMBAT_WALL_CLOCK_RATE);
        check_eq!(promo_handoffs.len(), 2);

        // Reinforce engagement with a friendly line_tank (`Q-H3`).
        site_book.reinforce_engagement(
            &mut ledger,
            &tank_reinforce,
            &tank_anatomy,
            SmolStr::new("expedition"),
            FieldPose {
                site,
                x_cm: 500,
                y_cm: 1_000,
            },
            Some(carrier),
            1_025,
        )?;

        // Enemy tank inflicts a headshot on `biped_mk1` (`Q-H7`, `FO-1`).
        let head = SectionId::new("head")?;
        let internals = CellId::new("internals")?;
        let cannon = ComponentId::new("cannon")?;
        let ammo = CellId::new("ammo")?;
        let headshot = UnitMutationPlan::new(
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
                UnitMutationOp::SetCell {
                    addr: CellAddr::Component {
                        component: cannon.clone(),
                        cell: ammo,
                    },
                    value: 6,
                },
            ],
            Some(biped_before.revision()),
            None,
        )?;
        let (_disabled_biped, biped_assess) = site_book.apply_field_damage(
            &mut ledger,
            &biped_before,
            &headshot,
            &biped_anatomy,
            1_030,
        )?;
        check_eq!(biped_assess.state, OperationalState::Disabled);
        check!(biped_assess.intact_components.contains(&cannon));

        // Pilot survives with zero permadeath (`FO-12`, `Q-H12`) and requests guaranteed NPC extraction
        // after an initial friendly rescue window expires (`Q-F4`, `P-5`).
        crew_book.record_unit_defeat_survival(
            pilot,
            biped_before.object(),
            biped_assess.state,
            engaged_pose_biped,
            true,
            true,
            1_030,
        )?;
        crew_book.begin_friendly_rescue(
            pilot,
            tank_reinforce.object(),
            engaged_pose_biped,
            200,
            1_040,
        )?;
        let port_candidates = vec![SafePortCandidate {
            port_id: SmolStr::new("stickney"),
            eligible_and_known: true,
            transit_duration_ic: 400,
        }];
        check!(
            crew_book
                .request_npc_extraction(pilot, &port_candidates, 600, 1_100)
                .is_err()
        );
        crew_book.request_npc_extraction(pilot, &port_candidates, 600, 1_250)?;
        let extracted = crew_book.complete_npc_extraction(pilot, 1_650)?;
        check_eq!(
            *extracted,
            OccupantSurvivalState::RecoveredAtPort {
                port: SmolStr::new("stickney"),
                left_unit_in_field: biped_before.object(),
                arrived_at_ic: 1_650,
            }
        );

        // Observer-bounded post-battle knowledge (`Q-H6`): only `tank_reinforce` inspected `biped_mk1`.
        obs_book.record_observation(
            tank_reinforce.object(),
            ObservationSource::FieldInspection,
            ObservedCombatFact::SurvivingComponentsRevealed {
                unit: biped_before.object(),
                surviving_components: biped_assess.surviving_components(),
            },
            1_035,
        )?;
        check!(obs_book.knows_surviving_components(tank_reinforce.object(), biped_before.object()));
        check!(!obs_book.knows_surviving_components(tank_enemy.object(), biped_before.object()));

        // Enemy withdraws, friendly tank stows in carrier bay, engagement settles and demotes (`Q-H4`, `Q-S10`).
        site_book.withdraw_unit(
            &mut ledger,
            tank_enemy.object(),
            false,
            WithdrawalDestination::PerimeterSurface {
                pose: FieldPose {
                    site: ObjId(771),
                    x_cm: 0,
                    y_cm: 0,
                },
            },
            1_040,
        )?;
        site_book.withdraw_unit(
            &mut ledger,
            tank_reinforce.object(),
            false,
            WithdrawalDestination::CarrierBay {
                carrier,
                slot: SmolStr::new("bay_alpha"),
            },
            1_042,
        )?;
        site_book.set_settlement_gate(DemotionSettlementGate::default());
        let demoted = site_book.demote_engagement(&mut ledger, 1_045)?;
        check_eq!(
            demoted.persistent_field_casualties,
            vec![biped_before.object()]
        );

        // Carrier departure requires explicit leave-behind confirmation for `biped_mk1` (`Q-H10`).
        check!(site_book.depart_carrier(carrier, false, 1_050).is_err());
        let departed = site_book.depart_carrier(carrier, true, 1_052)?;
        check_eq!(departed.stowed_units, vec![tank_reinforce.object()]);
        check_eq!(departed.left_behind_units, vec![biped_before.object()]);

        Ok(())
    }

    #[test]
    fn h2b_h2c_end_to_end_lawful_and_unlawful_recovery_sorties_and_48h_salvage_contest()
    -> TestResult {
        let catalog = load_stock_catalog()?;
        let (tank_unit, tank_spec) =
            instantiate_stock_unit(&catalog, "tacsim.unit.line_tank", ObjId(601))?;
        let (biped_before, biped_spec) =
            instantiate_stock_unit(&catalog, "tacsim.unit.biped_mk1", ObjId(602))?;

        let head = SectionId::new("head")?;
        let internals = CellId::new("internals")?;
        let headshot = UnitMutationPlan::new(
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
        let (disabled_biped, _) = apply(&biped_before, &headshot, 500)?;
        let biped_assess = assess_tacsim_unit(&disabled_biped, Some(&biped_spec))?;
        let tank_assess = assess_tacsim_unit(&tank_unit, Some(&tank_spec))?;
        let tank_mass = unit_mass_profile_for_spec(&tank_spec, 0, 0)?;
        let biped_mass = unit_mass_profile_for_spec(&biped_spec, 0, 0)?;

        let site = ObjId(780);
        let pose = FieldPose {
            site,
            x_cm: 0,
            y_cm: 0,
        };
        let bay_pose = FieldPose {
            site,
            x_cm: 1_500,
            y_cm: 0,
        };
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

        // 1. Lawful landholder recovery on owned ground: rejects `BarePriorTitle` (`Q-R1`) and
        // transfers clean title after 48 available real hours (`Q-S1`, `Q-S2`, `Q-S13`–`Q-S15`).
        let mut rec_book = RecoveryBook::new();
        let mut salv_book = SalvageClaimBook::new();
        let unit_owner = ObjId(10);
        let landholder = ObjId(20);
        let carrier = ObjId(920);

        let op = rec_book.attach(AttachTowRequest {
            tug: tank_unit.object(),
            tug_state: tank_assess.state,
            tug_can_move: tank_assess.can_move,
            tug_base_speed_cms: base_speed_cms_for_spec(&tank_spec),
            tug_mass_kg: tank_mass.total_mass_kg(),
            rig_profile: TowRigProfile::Standard,
            tug_pose: pose,
            casualty_assessment: biped_assess.clone(),
            casualty_mass: biped_mass,
            casualty_pose: pose,
            max_attach_distance_cm: DEFAULT_MAX_ATTACH_DISTANCE_CM,
            at_ic: 1_000,
        })?;
        let op_id = op.id();

        let lawful_ctx = RecoveryAuthorityContext {
            recoverer: landholder,
            unit_title_holder: unit_owner,
            has_owner_authorization: false,
            has_landholder_authorization: true,
            ground_tenure_at_attach: GroundTenure::Owned {
                landholder,
                claim_id: Some(SmolStr::new("deed.frontier_780")),
            },
        };
        salv_book.record_physical_attach(
            op_id,
            disabled_biped.object(),
            tank_unit.object(),
            &lawful_ctx,
            1_000,
        )?;
        rec_book.advance_tow(op_id, 1, bay_pose, 4, 1_004)?;
        rec_book.load_into_bay(LoadIntoBayRequest {
            id: op_id,
            expected_revision: 2,
            carrier,
            slot: SmolStr::new("mech_bay_1"),
            carrier_pose: bay_pose,
            max_load_distance_cm: DEFAULT_MAX_ATTACH_DISTANCE_CM,
            bay_capacity: &carrier_bay,
            casualty_unit: &disabled_biped,
            at_ic: 1_010,
        })?;
        salv_book.record_bay_load(op_id, carrier, 1_010)?;

        let status = salv_book.resolve_hull_recovery_custody(op_id, 50_000)?;
        let claim_id = match status {
            CustodyTitleStatus::PendingSalvageClaim { claim_id, .. } => claim_id,
            CustodyTitleStatus::ResolvedTitle { .. } => {
                return Err("expected PendingSalvageClaim".into());
            }
        };

        let bare_contest = salv_book.submit_contest(
            claim_id,
            unit_owner,
            ContestGround::BarePriorTitle,
            55_000,
        )?;
        check_eq!(bare_contest.valid_to_vacate, None);

        let (settled_status, _) =
            salv_book.adjudicate_claim(claim_id, 50_000 + CONTEST_WINDOW_AVAILABLE_REAL_SECS)?;
        check_eq!(
            settled_status,
            CustodyTitleStatus::ResolvedTitle { owner: landholder }
        );

        // 2. Unlawful third-party recovery on owned ground: vacated when proven with acquired evidence (`FO-4`, `Q-S4`).
        let pirate = ObjId(30);
        let op2 = rec_book.attach(AttachTowRequest {
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
            at_ic: 2_000,
        })?;
        let op2_id = op2.id();
        let unlawful_ctx = RecoveryAuthorityContext {
            recoverer: pirate,
            unit_title_holder: landholder,
            has_owner_authorization: false,
            has_landholder_authorization: false,
            ground_tenure_at_attach: GroundTenure::Owned {
                landholder,
                claim_id: Some(SmolStr::new("deed.frontier_780")),
            },
        };
        salv_book.record_physical_attach(
            op2_id,
            disabled_biped.object(),
            tank_unit.object(),
            &unlawful_ctx,
            2_000,
        )?;
        rec_book.load_into_bay(LoadIntoBayRequest {
            id: op2_id,
            expected_revision: 1,
            carrier: ObjId(999),
            slot: SmolStr::new("pirate_bay"),
            carrier_pose: pose,
            max_load_distance_cm: DEFAULT_MAX_ATTACH_DISTANCE_CM,
            bay_capacity: &carrier_bay,
            casualty_unit: &disabled_biped,
            at_ic: 2_010,
        })?;
        salv_book.record_bay_load(op2_id, ObjId(999), 2_010)?;

        // Duplicate active hull claim for the same unit is refused unless we test on a fresh SalvageClaimBook.
        let mut salv_book_unlawful = SalvageClaimBook::new();
        salv_book_unlawful.record_physical_attach(
            op2_id,
            disabled_biped.object(),
            tank_unit.object(),
            &unlawful_ctx,
            2_000,
        )?;
        salv_book_unlawful.record_bay_load(op2_id, ObjId(999), 2_010)?;
        let status2 = salv_book_unlawful.resolve_hull_recovery_custody(op2_id, 60_000)?;
        let claim2_id = match status2 {
            CustodyTitleStatus::PendingSalvageClaim { claim_id, .. } => claim_id,
            CustodyTitleStatus::ResolvedTitle { .. } => {
                return Err("expected PendingSalvageClaim".into());
            }
        };
        let evidence = SalvageEvidenceRecord {
            operation_id: op2_id,
            casualty: disabled_biped.object(),
            holder: landholder,
            kind: SalvageEvidenceKind::DirectWitness {
                witness: landholder,
                observed_at_ic: 2_002,
            },
        };
        salv_book_unlawful.submit_contest(
            claim2_id,
            landholder,
            ContestGround::UnlawfulRecovery { evidence },
            61_000,
        )?;
        let (vacated_status, _) = salv_book_unlawful.adjudicate_claim(claim2_id, 61_100)?;
        check_eq!(
            vacated_status,
            CustodyTitleStatus::ResolvedTitle { owner: landholder }
        );

        Ok(())
    }

    #[test]
    fn h3_stock_biped_component_stripping_workshop_restoration_and_scrapping() -> TestResult {
        let catalog = load_stock_catalog()?;
        let (donor_before, biped_spec) =
            instantiate_stock_unit(&catalog, "tacsim.unit.biped_mk1", ObjId(701))?;
        let (recipient_before, _) =
            instantiate_stock_unit(&catalog, "tacsim.unit.biped_mk1", ObjId(702))?;
        let anatomy = anatomy_for_unit_spec(&biped_spec)?;

        let head = SectionId::new("head")?;
        let internals = CellId::new("internals")?;
        let cannon = ComponentId::new("cannon")?;
        let machine_gun = ComponentId::new("machine_gun")?;
        let visual_sensor = ComponentId::new("visual_sensor")?;
        let ammo = CellId::new("ammo")?;

        let cannon_ammo_init = donor_before
            .cell(&CellAddr::Component {
                component: cannon.clone(),
                cell: ammo.clone(),
            })
            .ok_or("missing cannon ammo")?
            .current;

        // Donor suffers a headshot after spending 3 rounds of cannon ammo (`Disabled`).
        let donor_hit = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: head.clone(),
                        cell: internals.clone(),
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
                    value: cannon_ammo_init - 3,
                },
            ],
            Some(donor_before.revision()),
            None,
        )?;
        let (donor_disabled, _) = apply(&donor_before, &donor_hit, 100)?;

        let owner = ObjId(10);
        let custody = CustodyTitleStatus::ResolvedTitle { owner };
        let mut workshop = WorkshopBook::new();

        // 1. Strip the surviving `cannon` from `donor_disabled` (`Q-S11`, `Q-F5`, `Q-F6`).
        let (donor_stripped, _, stripped_cannon) = workshop.strip_component(
            &donor_disabled,
            &anatomy,
            &cannon,
            owner,
            &StripAuthorization::BayOrWorkshop {
                custody: custody.clone(),
                has_explicit_owner_permission: false,
            },
            None,
            110,
        )?;
        check_eq!(
            stripped_cannon.provenance.parent_hull,
            donor_disabled.object()
        );
        check_eq!(
            stripped_cannon.cells.get(&ammo).map(|c| c.current),
            Some(cannon_ammo_init - 3)
        );

        // 2. Strip the surviving `visual_sensor` from `recipient_before` before `recipient` loses its cannon,
        // and use that sensor to restore `donor_stripped`'s reconstructed head!
        let (recipient_no_sensor, _, stripped_sensor) = workshop.strip_component(
            &recipient_before,
            &anatomy,
            &visual_sensor,
            owner,
            &StripAuthorization::BayOrWorkshop {
                custody: custody.clone(),
                has_explicit_owner_permission: false,
            },
            None,
            115,
        )?;

        // Recipient also has both weapons destroyed -> `Unarmed`.
        let recipient_disarm = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetComponentCondition {
                    component: cannon.clone(),
                    condition: ConditionId::new("destroyed")?,
                },
                UnitMutationOp::SetComponentCondition {
                    component: machine_gun,
                    condition: ConditionId::new("destroyed")?,
                },
            ],
            Some(recipient_no_sensor.revision()),
            None,
        )?;
        let (recipient_unarmed, _) = apply(&recipient_no_sensor, &recipient_disarm, 120)?;
        let unarmed_assess = assess_tacsim_unit(&recipient_unarmed, Some(&biped_spec))?;
        check_eq!(unarmed_assess.state, OperationalState::Unarmed);

        // Install the salvaged `cannon` from `donor` into `recipient_unarmed` -> `Operational` (`FO-2`, `Q-S12`)!
        let cannon_replacement = ReplacementPartSpec::from_stripped_record(&stripped_cannon);
        let (recipient_rearmed, rearmed_assess, _) = workshop.replace_component_in_slot(
            &recipient_unarmed,
            &anatomy,
            &cannon,
            &cannon_replacement,
            owner,
            &custody,
            false,
            1_200,
            180,
            None,
            130,
        )?;
        check_eq!(rearmed_assess.state, OperationalState::Operational);
        check!(rearmed_assess.can_fire);
        check_eq!(
            recipient_rearmed
                .cell(&CellAddr::Component {
                    component: cannon,
                    cell: ammo,
                })
                .map(|c| c.current),
            Some(cannon_ammo_init - 3)
        );

        // 3. Reconstruct `donor_stripped`'s destroyed `head` and install `stripped_sensor` (`FO-2`, `Q-S12`).
        let (donor_head_rebuilt, _, _) = workshop.reconstruct_section(
            &donor_stripped,
            &anatomy,
            &head,
            owner,
            &custody,
            false,
            2_500,
            300,
            None,
            140,
        )?;
        let sensor_replacement = ReplacementPartSpec::from_stripped_record(&stripped_sensor);
        let (donor_restored, donor_restored_assess, _) = workshop.replace_component_in_slot(
            &donor_head_rebuilt,
            &anatomy,
            &visual_sensor,
            &sensor_replacement,
            owner,
            &custody,
            false,
            900,
            120,
            None,
            150,
        )?;
        // Donorstill has its `machine_gun`, `power`, and `core` intact in `torso`, so with `head`
        // reconstructed and `visual_sensor` replaced, `donor_restored` is back to `Operational`!
        check_eq!(donor_restored_assess.state, OperationalState::Operational);
        check!(donor_restored_assess.can_move);
        check!(donor_restored_assess.can_fire);

        // 4. Scrap `donor_restored` under resolved title (`Q-S11`, `Q-F5`).
        let (_scrapped_unit, scrap_report) =
            workshop.scrap_hull(&donor_restored, &anatomy, owner, &custody, false, 200)?;
        check_eq!(
            scrap_report.final_assessment.state,
            OperationalState::Irrecoverable
        );
        check!(scrap_report.structural_scrap_kg > 0);

        Ok(())
    }
}

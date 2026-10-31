//! Doc 27 (`H1-A`–`H1-C`, `H2-A`–`H2-C`, `H3`, `W-clock`) — `tacsim` integration for
//! field-unit component survival, operational state assessment, IC combat-return mutation
//! plans, hull-epoch close validation, field combat promotion/demotion & crew survival,
//! sortie roster/manifest/crew-pool/expense orchestration, universal & live coupled towing,
//! surface block-tenure & mining-pause bridges, recovery entitlement & 48-hour salvage
//! claims, black-box forensics & contraband component provenance, field engineering &
//! resupply tenders, the `W-clock` consumers (post-outage unit quarantine, salvage deadline
//! extension; the clock itself is the kernel's), and workshop component
//! stripping, scrapping, rearming, cost quoting, and restoration across stock unit schemas.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use mush_core::ObjId;
use mush_org::PrincipalId;
use mush_roll::commerce::ic_claims::IcCloseKind;
pub use mush_tactics::field_ops::{
    ActiveFieldSortie, AdjustedSalvageClaimDeadline, AtRiskRepairEntry, AttachTowRequest,
    AuthoritativeRecoveryRecord, AuthorityHandoffRequest, BayCapacitySpec, BlackBoxFlightEntry,
    BlackBoxForensicReport, COMBAT_WALL_CLOCK_RATE, CONTEST_WINDOW_AVAILABLE_REAL_SECS,
    CarrierDepartureOutcome, CarrierPersonnelManifest, ComponentCondition, ComponentProvenance,
    ConditionError, ContestGround, ContestSuccessReason, ContrabandReason, CrewMemberKind,
    CrewSurvivalBook, CustodyAction, CustodyTitleStatus, DEFAULT_FALLBACK_SAFE_PORT,
    DEFAULT_MAX_ATTACH_DISTANCE_CM, DEFAULT_MAX_TENDER_SERVICE_DISTANCE_CM,
    DEFAULT_SPARE_PARTS_KG_PER_STRUCTURAL_POINT, DEFAULT_UNJAM_SPARE_PARTS_KG, DemotionOutcome,
    DemotionSettlementGate, EmbarkationOrigin, EmbarkedPersonRecord, EngagementError,
    FALLBACK_SECTION_SCRAP_KG, FieldAuthorityKind, FieldCloseOutcome, FieldCustodyLedger,
    FieldCustodyRecord, FieldEngineeringActionKind, FieldEngineeringBook, FieldEngineeringError,
    FieldEngineeringLogEntry, FieldEpochDisposition, FieldInspectionReport, FieldObservationBook,
    FieldPose, FieldPromotionTrigger, FieldSiteEngagementBook, FieldTenderState,
    FieldUnitAssessment, GroundTenure, IC_DAY_SECONDS, IC_SURFACE_CLOCK_RATE, LinkLossPosture,
    LoadIntoBayRequest, LoadedCasualtyOutcome, MAX_AUTHORITATIVE_RECOVERY_RECORDS,
    MAX_BLACK_BOX_ENTRIES_PER_HULL, MAX_CLAIM_ENTRIES, MAX_CREW_POOL_CONTRACTS, MAX_CREW_RECORDS,
    MAX_FIELD_CUSTODY_UNITS, MAX_FIELD_ENGINEERING_LOGS, MAX_FIELD_TENDERS, MAX_MANIFEST_PERSONNEL,
    MAX_OBSERVATIONS_PER_ACTOR, MAX_QUARANTINED_UNITS, MAX_RECOVERY_OPERATIONS, MAX_SALVAGE_CLAIMS,
    MAX_SITE_UNITS, MAX_STRIPPED_COMPONENTS, MAX_TOW_MASS_RATIO_PERMILLE, MAX_WORKSHOP_ORDERS,
    MissionLeaderRoleGrant, MotionClockHandoff, NpcCrewContract, NpcMissionScope,
    ObservationSource, ObservedCombatFact, ObserverCombatEntry, OccupantSurvivalState,
    OperationalState, OutageQuarantineBook, PortCrewPool, ProvenanceLegalityStatus,
    QuarantineError, QuarantinedUnitRecord, RecoveryAuthorityContext, RecoveryBook,
    RecoveryEntitlement, RecoveryError, RecoveryOpId, RecoveryOperation, RecoveryStage,
    ReplacementPartSpec, SCRAP_KG_PER_STRUCTURAL_POINT, SafePortCandidate, SalvageClaim,
    SalvageClaimBook, SalvageClaimId, SalvageClaimState, SalvageContest, SalvageError,
    SalvageEvidenceKind, SalvageEvidenceRecord, SalvageSubject, ScrapYieldReport, SectionCondition,
    SiteUnitPresence, SortieError, SortieLaunchBlockReason, SortieSlotLaunchState,
    StripAuthorization, StrippedComponentRecord, TowEvaluation, TowRigProfile,
    TugInspectionContext, UnitAnatomyProfile, UnitExpenseCategory, UnitExpensePolicy,
    UnitMassProfile, WithdrawalDestination, WorkshopBook, WorkshopCostQuote, WorkshopError,
    WorkshopFacilityTariff, WorkshopOperationKind, WorkshopWorkOrderRecord, assess_tactical_unit,
    check_custody_action_allowed, effective_component_condition, effective_section_condition,
    evaluate_recovery_entitlement, ic_condition_plan, inspect_field_unit, is_occupant_fit_for_duty,
    resolve_link_loss_posture, scaled_tow_speed_cms, select_extraction_port,
    sync_detachment_availability_from_field, tow_mass_ratio_permille, tow_speed_factor_permille,
    validate_close_outcome,
};
use mush_tactics::{
    CellAddr, CellId, ComponentId, SectionId, TacticalUnit, UnitError, UnitMutationPlan, apply,
};
use smol_str::SmolStr;

use crate::arena::damage::{HULL_SECTION, POWER_CAPABILITY, UnitReturn, WEAPON_CAPABILITY_PREFIX};
use crate::arena::ledger::EventKind;
use crate::arena::perception::Notice;
use crate::arena::state::Arena;
use crate::definitions::unit::{
    AMMO_CELL, CONDITION_CELL, ComponentRole, Consequence, INTERNALS_CELL, UnitSpec,
};
use crate::surface::address::{block_letter, pocket_of_cell};
use crate::surface::black_box::BlackBoxLine;
use crate::surface::claims::BlockTenure;
use crate::surface::drive::{DriveRates, cu_of_cost, secs_of_cost};

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

/// Convert a frontier site [`FieldPose`] (`x_cm`, `y_cm`) into its `10 m` surface cell
/// and resolve the `4×4` frontier block letter (`'A'..='P'`) (`FO-4`, `Q-S1`, `Q-S3`).
#[must_use]
pub fn block_letter_at_site_pose(pose: FieldPose) -> Option<char> {
    let cell_x = pose.x_cm.div_euclid(1_000);
    let cell_y = pose.y_cm.div_euclid(1_000);
    let pocket = pocket_of_cell((cell_x, cell_y))?;
    block_letter(pocket.0, pocket.1)
}

/// Resolve the [`GroundTenure`] at a casualty's [`FieldPose`] from the frontier site's
/// [`BlockTenure`] table (`FO-4`, `Q-S1`, `Q-S3`).
///
/// When a block is held by an organization principal rather than an object principal,
/// `org_landholder_fallback` (or the block's `place` object id if `None`) is used as
/// the `ObjId` landholder handle.
#[must_use]
pub fn ground_tenure_at_site_pose(
    pose: FieldPose,
    block_tenures: &BTreeMap<char, BlockTenure>,
    now_ic: i64,
    org_landholder_fallback: Option<ObjId>,
) -> GroundTenure {
    let Some(letter) = block_letter_at_site_pose(pose) else {
        return GroundTenure::Unclaimed;
    };
    let Some(tenure) = block_tenures.get(&letter) else {
        return GroundTenure::Unclaimed;
    };
    let Some(claim) = &tenure.claim else {
        return GroundTenure::Unclaimed;
    };
    if !claim.live(now_ic) {
        return GroundTenure::Unclaimed;
    }
    let landholder = if let PrincipalId::Object(obj) = claim.holder {
        obj
    } else {
        org_landholder_fallback.unwrap_or(tenure.place)
    };
    GroundTenure::Owned {
        landholder,
        claim_id: Some(SmolStr::new(format!(
            "block.{letter}.epoch.{}",
            claim.epoch
        ))),
    }
}

/// Check whether `unit` is eligible to run an unengaged frontier surface mining pass
/// on `site_book` (`Q-H1`).
///
/// Surface mining pauses/refuses while `site_book.is_engaged()` is true or when `unit`
/// is not an `Operational` unit on the site.
#[must_use]
pub fn can_run_surface_mining_for_unit(site_book: &FieldSiteEngagementBook, unit: ObjId) -> bool {
    if site_book.is_engaged() {
        return false;
    }
    let Some(presence) = site_book.unit_presence(unit) else {
        return false;
    };
    presence.assessment.state == OperationalState::Operational
}

/// Summary of reconciling an [`Arena`] combat step or match return into IC field-operations
/// books (`H1-A`, `H1-B`, `H1-C`, `Q-H6`, `Q-H8`, `Q-H12`).
#[derive(Clone, Debug, PartialEq)]
pub struct ArenaFieldReconciliationReport {
    /// Post-reconciliation [`TacticalUnit`] and [`FieldUnitAssessment`] per unit on the site.
    pub updated_units: BTreeMap<ObjId, (TacticalUnit, FieldUnitAssessment)>,
    /// Number of observer-bounded [`ObservedCombatFact`] records appended from `notices`.
    pub observations_recorded: usize,
}

/// Reconcile [`Arena::returned_units`] and combat [`Notice`]s into a [`FieldSiteEngagementBook`],
/// [`FieldCustodyLedger`], [`CrewSurvivalBook`], and observer-bounded [`FieldObservationBook`]
/// (`H1-A`, `H1-B`, `H1-C`, `Q-H6`, `Q-H8`, `Q-H12`).
///
/// # Errors
/// Returns [`FieldOpsError`] if anatomy derivation, IC condition-plan construction,
/// field-damage application, crew survival recording, or observation logging fails.
#[allow(clippy::too_many_arguments)]
pub fn reconcile_arena_to_field_engagement(
    arena: &Arena,
    notices: &[Notice],
    pre_combat_units: &BTreeMap<ObjId, (TacticalUnit, Option<UnitSpec>)>,
    unit_pilots: &BTreeMap<ObjId, ObjId>,
    site_book: &mut FieldSiteEngagementBook,
    ledger: &mut FieldCustodyLedger,
    crew_book: &mut CrewSurvivalBook,
    obs_book: &mut FieldObservationBook,
    at_ic: i64,
) -> Result<ArenaFieldReconciliationReport, FieldOpsError> {
    let site = site_book.site();
    let mut updated_units = BTreeMap::new();

    for unit_return in arena.returned_units() {
        let Some((before, spec_opt)) = pre_combat_units.get(&unit_return.unit) else {
            continue;
        };
        let anatomy = match spec_opt {
            Some(spec) => anatomy_for_unit_spec(spec)?,
            None => anatomy_for_tactical_unit(before)?,
        };
        let field_pose = FieldPose {
            site,
            x_cm: unit_return.pose.x_cm,
            y_cm: unit_return.pose.y_cm,
        };
        if let Some(presence) = site_book.unit_presence(unit_return.unit)
            && presence.pose != field_pose
            && presence.assessment.can_move
        {
            let _ = site_book
                .record_surface_motion(unit_return.unit, field_pose, None)
                .map_err(FieldOpsError::Engagement)?;
        }

        let plan_opt = ic_condition_plan(
            before,
            &unit_return.cells,
            Some(&anatomy),
            Some(SmolStr::new("arena_combat_return")),
        )
        .map_err(FieldOpsError::Condition)?;

        let (post_unit, assessment) = match plan_opt {
            Some(plan) => site_book
                .apply_field_damage(ledger, before, &plan, &anatomy, at_ic)
                .map_err(FieldOpsError::Engagement)?,
            None => {
                let assess = assess_tactical_unit(before, &anatomy);
                (before.clone(), assess)
            }
        };

        if assessment.state != OperationalState::Operational
            && let Some(&pilot) = unit_pilots.get(&unit_return.unit)
            && matches!(
                crew_book.state(pilot),
                Some(OccupantSurvivalState::AboardUnit { .. })
            )
        {
            crew_book
                .record_unit_defeat_survival(
                    pilot,
                    unit_return.unit,
                    assessment.state,
                    field_pose,
                    true,
                    false,
                    at_ic,
                )
                .map_err(FieldOpsError::Engagement)?;
        }

        updated_units.insert(unit_return.unit, (post_unit, assessment));
    }

    let mut observations_recorded: usize = 0;
    for (_ts, event, _details) in notices {
        let (target_unit, attacker_opt, fact_opt) = match event {
            EventKind::Hit {
                unit: attacker,
                target,
                section,
                ..
            } => {
                let sec_id = SectionId::new(section.as_str()).map_err(FieldOpsError::Unit)?;
                (
                    *target,
                    Some(*attacker),
                    Some(ObservedCombatFact::SectionDamaged {
                        unit: *target,
                        section: sec_id,
                    }),
                )
            }
            EventKind::SectionDestroyed {
                unit: target,
                section,
                by,
                ..
            } => {
                let sec_id = SectionId::new(section.as_str()).map_err(FieldOpsError::Unit)?;
                (
                    *target,
                    *by,
                    Some(ObservedCombatFact::SectionDamaged {
                        unit: *target,
                        section: sec_id,
                    }),
                )
            }
            EventKind::UnitDisabled { unit: target } => {
                let state = updated_units
                    .get(target)
                    .map_or(OperationalState::Disabled, |(_, assess)| assess.state);
                (
                    *target,
                    None,
                    Some(ObservedCombatFact::UnitOperationalStateObserved {
                        unit: *target,
                        state,
                    }),
                )
            }
            EventKind::UnitDestroyed { unit: target, by } => {
                let state = updated_units
                    .get(target)
                    .map_or(OperationalState::Wreck, |(_, assess)| assess.state);
                (
                    *target,
                    *by,
                    Some(ObservedCombatFact::UnitOperationalStateObserved {
                        unit: *target,
                        state,
                    }),
                )
            }
            _ => (ObjId(0), None, None),
        };

        let Some(fact) = fact_opt else {
            continue;
        };

        let mut direct_observers = BTreeSet::new();
        direct_observers.insert(target_unit);
        if let Some(attacker) = attacker_opt {
            direct_observers.insert(attacker);
        }
        for &candidate in pre_combat_units.keys() {
            if arena
                .contacts
                .of(candidate)
                .any(|(_, c)| c.target == target_unit)
            {
                direct_observers.insert(candidate);
            }
        }

        for observer_unit in direct_observers {
            obs_book
                .record_observation(
                    observer_unit,
                    ObservationSource::DirectLineOfSight,
                    fact.clone(),
                    at_ic,
                )
                .map_err(FieldOpsError::Engagement)?;
            observations_recorded = observations_recorded.saturating_add(1);
        }
    }

    Ok(ArenaFieldReconciliationReport {
        updated_units,
        observations_recorded,
    })
}

/// Derive coupled surface [`DriveRates`] for an active [`RecoveryOperation`] (`H2-A`,
/// `Q-S7`).
///
/// - Scales `max_speed_cms` to `op.coupled_speed_cms().max(1)`.
/// - Scales `cu_per_nominal_cell` proportionally to combined mass
///   `⌈base_cu * (tug_mass_kg + casualty_mass_kg) / tug_mass_kg⌉` so towing a heavy
///   casualty consumes proportionally more battery charge per nominal cell.
#[must_use]
pub fn coupled_drive_rates_for_tow(
    base_tug_rates: &DriveRates,
    op: &RecoveryOperation,
) -> DriveRates {
    let tug_mass = op.tug_mass_kg().max(1);
    let combined_mass = tug_mass.saturating_add(op.casualty_mass().total_mass_kg());
    let scaled_cu = u64::from(base_tug_rates.cu_per_nominal_cell)
        .saturating_mul(combined_mass)
        .div_ceil(tug_mass);
    DriveRates {
        max_speed_cms: op.coupled_speed_cms().max(1),
        cu_per_nominal_cell: u32::try_from(scaled_cu).unwrap_or(u32::MAX),
    }
}

/// Advance an active coupled tow along one surface drive leg of atlas path `leg_cost`
/// (hundredths of a nominal cell), updating both the [`RecoveryOperation`] and the
/// [`FieldSiteEngagementBook`] poses while computing coupled leg duration (`secs`) and
/// battery draw (`cu`) via [`coupled_drive_rates_for_tow`] (`H1-B`, `H2-A`, `Q-S7`).
///
/// # Errors
/// Returns [`FieldOpsError`] if the tow operation is unknown, not attached, or fails
/// coupled motion validation.
#[allow(clippy::too_many_arguments)]
pub fn step_coupled_surface_tow_drive_leg(
    site_book: &mut FieldSiteEngagementBook,
    ledger: &mut FieldCustodyLedger,
    recovery_book: &mut RecoveryBook,
    op_id: RecoveryOpId,
    expected_generation: u64,
    base_tug_rates: &DriveRates,
    leg_cost: u64,
    destination_pose: FieldPose,
    now_ic: i64,
) -> Result<(RecoveryOperation, u32, u32), FieldOpsError> {
    let op_snapshot = recovery_book
        .get(op_id)
        .ok_or(FieldOpsError::Recovery(RecoveryError::UnknownOperation(
            op_id,
        )))?
        .clone();
    let coupled_rates = coupled_drive_rates_for_tow(base_tug_rates, &op_snapshot);
    let leg_secs = secs_of_cost(leg_cost, &coupled_rates).unwrap_or(1).max(1);
    let leg_cu = cu_of_cost(leg_cost, &coupled_rates).unwrap_or(0);
    let arrival_ic = now_ic.saturating_add(i64::from(leg_secs));
    site_book
        .step_coupled_tow_motion(
            ledger,
            recovery_book,
            op_id,
            expected_generation,
            destination_pose,
            leg_secs,
            arrival_ic,
        )
        .map_err(FieldOpsError::Engagement)?;
    let updated_op = recovery_book
        .get(op_id)
        .ok_or(FieldOpsError::Recovery(RecoveryError::UnknownOperation(
            op_id,
        )))?
        .clone();
    Ok((updated_op, leg_secs, leg_cu))
}

/// Record a surface [`BlackBoxLine`] (`MindSealed` or `SortieClosed`) into a hull's
/// tamper-evident black-box forensic log inside [`SalvageClaimBook`] (`FO-4`, `Q-S4`).
///
/// # Errors
/// Returns [`FieldOpsError::Salvage`] if the hull's black-box entry cap is exceeded.
pub fn record_surface_black_box_line_to_forensics(
    salvage_book: &mut SalvageClaimBook,
    line: &BlackBoxLine,
) -> Result<(), FieldOpsError> {
    match line {
        BlackBoxLine::MindSealed {
            unit,
            sortie,
            at_ic,
            site,
            mind,
            governed,
            ..
        } => {
            salvage_book
                .append_black_box_entry(
                    *unit,
                    BlackBoxFlightEntry::EpochOpened {
                        sortie_id: SmolStr::new(format!("{sortie:?}")),
                        host_site: SmolStr::new(site.as_str()),
                        operator: None,
                        mind_label: mind
                            .clone()
                            .unwrap_or_else(|| SmolStr::new_static("unsealed")),
                        governed: governed.unwrap_or(false),
                        at_ic: *at_ic,
                    },
                )
                .map_err(FieldOpsError::Salvage)?;
        }
        BlackBoxLine::SortieClosed {
            unit, at_ic, close, ..
        } => {
            let base = match close {
                IcCloseKind::ReturnedToBase { base }
                | IcCloseKind::PoweredDownInBase { base }
                | IcCloseKind::RecoveredToBase { base } => *base,
                IcCloseKind::Destroyed => ObjId(0),
            };
            salvage_book
                .append_black_box_entry(
                    *unit,
                    BlackBoxFlightEntry::EpochClosedInBay {
                        operation_id: None,
                        base,
                        at_ic: *at_ic,
                    },
                )
                .map_err(FieldOpsError::Salvage)?;
        }
    }
    Ok(())
}

/// Record arena combat [`Notice`] events (`Hit`, `SectionDestroyed`) into each affected
/// unit's black-box flight recorder inside [`SalvageClaimBook`] (`FO-4`, `Q-H6`, `Q-H9`,
/// `Q-S4`).
///
/// # Errors
/// Returns [`FieldOpsError`] if any section identifier in `notices` is invalid or if
/// the black-box entry cap is exceeded.
pub fn record_arena_notices_to_black_box(
    salvage_book: &mut SalvageClaimBook,
    notices: &[Notice],
    updated_states: &BTreeMap<ObjId, OperationalState>,
    at_ic: i64,
) -> Result<usize, FieldOpsError> {
    let mut recorded: usize = 0;
    for (_ts, event, _details) in notices {
        match event {
            EventKind::Hit {
                unit: attacker,
                target,
                section,
                ..
            } => {
                let sec_id = SectionId::new(section.as_str()).map_err(FieldOpsError::Unit)?;
                let state = updated_states
                    .get(target)
                    .copied()
                    .unwrap_or(OperationalState::Operational);
                salvage_book
                    .append_black_box_entry(
                        *target,
                        BlackBoxFlightEntry::CombatDamageRecorded {
                            attacker: Some(*attacker),
                            section: sec_id,
                            resulting_state: state,
                            at_ic,
                        },
                    )
                    .map_err(FieldOpsError::Salvage)?;
                recorded = recorded.saturating_add(1);
            }
            EventKind::SectionDestroyed {
                unit: target,
                section,
                by,
                ..
            } => {
                let sec_id = SectionId::new(section.as_str()).map_err(FieldOpsError::Unit)?;
                let state = updated_states
                    .get(target)
                    .copied()
                    .unwrap_or(OperationalState::Disabled);
                salvage_book
                    .append_black_box_entry(
                        *target,
                        BlackBoxFlightEntry::CombatDamageRecorded {
                            attacker: *by,
                            section: sec_id,
                            resulting_state: state,
                            at_ic,
                        },
                    )
                    .map_err(FieldOpsError::Salvage)?;
                recorded = recorded.saturating_add(1);
            }
            _ => {}
        }
    }
    Ok(recorded)
}

/// Unjam a jammed component on a `tacsim` [`TacticalUnit`] in the field using a
/// registered [`FieldTenderState`], deriving anatomy from `spec` (or fallback unit
/// inspection) and checking active combat / tow state on `site_book` and
/// `recovery_book` (`H1-A`, `H2-A`, `H3`).
///
/// # Errors
/// Returns [`FieldOpsError`] if anatomy derivation or field unjam validation fails.
#[allow(clippy::too_many_arguments)]
pub fn field_unjam_tacsim_unit(
    eng_book: &mut FieldEngineeringBook,
    site_book: &FieldSiteEngagementBook,
    recovery_book: &RecoveryBook,
    tender_unit: ObjId,
    target_unit: &mut TacticalUnit,
    spec: Option<&UnitSpec>,
    component_id: &ComponentId,
    restored_health_cell: Option<&CellId>,
    spare_parts_cost_kg: u64,
    elapsed_secs: i64,
    completed_at_ic: i64,
) -> Result<FieldEngineeringLogEntry, FieldOpsError> {
    let anatomy = match spec {
        Some(s) => anatomy_for_unit_spec(s)?,
        None => anatomy_for_tactical_unit(target_unit)?,
    };
    let target_id = target_unit.object();
    let target_pose = site_book.unit_presence(target_id).map_or(
        FieldPose {
            site: site_book.site(),
            x_cm: 0,
            y_cm: 0,
        },
        |u| u.pose,
    );
    let target_in_active_combat = site_book.is_engaged();
    let target_coupled_to_tow = recovery_book.active_operation_for_tug(target_id).is_some()
        || recovery_book
            .active_operation_for_casualty(target_id)
            .is_some();
    eng_book
        .field_unjam_component(
            tender_unit,
            target_unit,
            target_pose,
            &anatomy,
            component_id,
            restored_health_cell,
            spare_parts_cost_kg,
            target_in_active_combat,
            target_coupled_to_tow,
            elapsed_secs,
            completed_at_ic,
        )
        .map_err(FieldOpsError::Engineering)
}

/// Stabilize a damaged (non-destroyed) section on a `tacsim` [`TacticalUnit`] in the
/// field using a registered [`FieldTenderState`] (`H1-A`, `H2-A`, `H3`).
///
/// # Errors
/// Returns [`FieldOpsError`] if the section is destroyed (requiring a workshop facility)
/// or any field engineering check fails.
#[allow(clippy::too_many_arguments)]
pub fn field_stabilize_tacsim_unit_section(
    eng_book: &mut FieldEngineeringBook,
    site_book: &FieldSiteEngagementBook,
    recovery_book: &RecoveryBook,
    tender_unit: ObjId,
    target_unit: &mut TacticalUnit,
    spec: Option<&UnitSpec>,
    section_id: &SectionId,
    requested_points: i64,
    spare_parts_kg_per_point: u64,
    elapsed_secs: i64,
    completed_at_ic: i64,
) -> Result<FieldEngineeringLogEntry, FieldOpsError> {
    let anatomy = match spec {
        Some(s) => anatomy_for_unit_spec(s)?,
        None => anatomy_for_tactical_unit(target_unit)?,
    };
    let target_id = target_unit.object();
    let target_pose = site_book.unit_presence(target_id).map_or(
        FieldPose {
            site: site_book.site(),
            x_cm: 0,
            y_cm: 0,
        },
        |u| u.pose,
    );
    let target_in_active_combat = site_book.is_engaged();
    let target_coupled_to_tow = recovery_book.active_operation_for_tug(target_id).is_some()
        || recovery_book
            .active_operation_for_casualty(target_id)
            .is_some();
    eng_book
        .field_stabilize_damaged_section(
            tender_unit,
            target_unit,
            target_pose,
            &anatomy,
            section_id,
            requested_points,
            spare_parts_kg_per_point,
            target_in_active_combat,
            target_coupled_to_tow,
            elapsed_secs,
            completed_at_ic,
        )
        .map_err(FieldOpsError::Engineering)
}

/// Rearm ammunition and/or transfer charge (`CU`) to a `tacsim` [`TacticalUnit`] in the
/// field from a registered [`FieldTenderState`] (`H1-A`, `H2-A`, `H3`).
///
/// # Errors
/// Returns [`FieldOpsError`] if the target is an unserviceable wreck or any field
/// engineering check fails.
#[allow(clippy::too_many_arguments)]
pub fn field_resupply_tacsim_unit(
    eng_book: &mut FieldEngineeringBook,
    site_book: &FieldSiteEngagementBook,
    recovery_book: &RecoveryBook,
    tender_unit: ObjId,
    target_unit: &mut TacticalUnit,
    spec: Option<&UnitSpec>,
    weapon_rearm: Option<(&ComponentId, i64)>,
    requested_charge_cu: u32,
    credit_cost_per_round: u64,
    credit_cost_per_cu: u64,
    expense_policy: Option<&UnitExpensePolicy>,
    elapsed_secs: i64,
    completed_at_ic: i64,
) -> Result<FieldEngineeringLogEntry, FieldOpsError> {
    let anatomy = match spec {
        Some(s) => anatomy_for_unit_spec(s)?,
        None => anatomy_for_tactical_unit(target_unit)?,
    };
    let ammo_cell = CellId::new(AMMO_CELL).map_err(FieldOpsError::Unit)?;
    let weapon_rearm_with_cell = weapon_rearm.map(|(comp, rounds)| (comp, &ammo_cell, rounds));
    let target_id = target_unit.object();
    let target_pose = site_book.unit_presence(target_id).map_or(
        FieldPose {
            site: site_book.site(),
            x_cm: 0,
            y_cm: 0,
        },
        |u| u.pose,
    );
    let target_in_active_combat = site_book.is_engaged();
    let target_coupled_to_tow = recovery_book.active_operation_for_tug(target_id).is_some()
        || recovery_book
            .active_operation_for_casualty(target_id)
            .is_some();
    eng_book
        .field_resupply_ammo_and_charge(
            tender_unit,
            target_unit,
            target_pose,
            &anatomy,
            weapon_rearm_with_cell,
            requested_charge_cu,
            credit_cost_per_round,
            credit_cost_per_cu,
            expense_policy,
            target_in_active_combat,
            target_coupled_to_tow,
            elapsed_secs,
            completed_at_ic,
        )
        .map_err(FieldOpsError::Engineering)
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
    /// Underlying [`SortieError`].
    Sortie(SortieError),
    /// Underlying [`FieldEngineeringError`].
    Engineering(FieldEngineeringError),
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
            Self::Sortie(e) => write!(f, "field sortie error: {e}"),
            Self::Engineering(e) => write!(f, "field engineering error: {e}"),
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

    #[test]
    fn surface_block_tenure_mining_pause_and_workshop_cost_quote_and_rearm() -> TestResult {
        use mush_org::PrincipalId;

        use crate::surface::claims::{BlockClaim, BlockTenure};

        let catalog = load_stock_catalog()?;
        let (biped_before, biped_spec) =
            instantiate_stock_unit(&catalog, "tacsim.unit.biped_mk1", ObjId(801))?;
        let anatomy = anatomy_for_unit_spec(&biped_spec)?;

        // 1. Surface block letter & live vs lapsed `BlockTenure` -> `GroundTenure` (`FO-4`, `Q-S1`, `Q-S3`).
        let site = ObjId(790);
        // `x_cm = 5_000, y_cm = 5_000` -> cell `(5, 5)` -> south-west block `(0, 0)` = `'M'`.
        let pose_block_m = FieldPose {
            site,
            x_cm: 5_000,
            y_cm: 5_000,
        };
        check_eq!(block_letter_at_site_pose(pose_block_m), Some('M'));

        let landholder = ObjId(44);
        let mut tenure_m =
            BlockTenure::new(CanonicalId::new("tacsim.port.site_790")?, 'M', ObjId(990));
        tenure_m.claim = Some(BlockClaim {
            holder: PrincipalId::Object(landholder),
            account: None,
            epoch: 2,
            opened_ic: None,
            registered_ic: 1_000,
            term_start_ic: 1_000,
            term_end_ic: 10_000,
            worked_at_term_start: 0,
            discovered_ic: None,
            declared: None,
            fee: None,
            refund: None,
            lapsed_ic: None,
        });
        let mut tenures = BTreeMap::new();
        tenures.insert('M', tenure_m);
        check_eq!(
            ground_tenure_at_site_pose(pose_block_m, &tenures, 5_000, None),
            GroundTenure::Owned {
                landholder,
                claim_id: Some(SmolStr::new("block.M.epoch.2")),
            }
        );
        // After `term_end_ic`, the lapsed block resolves to `GroundTenure::Unclaimed`.
        check_eq!(
            ground_tenure_at_site_pose(pose_block_m, &tenures, 15_000, None),
            GroundTenure::Unclaimed
        );

        // 2. Surface mining pause during active site combat (`Q-H1`).
        let mut ledger = FieldCustodyLedger::new();
        let mut site_book = FieldSiteEngagementBook::new(site)?;
        site_book.deploy_surface_unit(
            &mut ledger,
            &biped_before,
            &anatomy,
            SmolStr::new("miners"),
            pose_block_m,
            None,
            1_000,
        )?;
        check!(can_run_surface_mining_for_unit(
            &site_book,
            biped_before.object()
        ));

        site_book.promote_to_engagement(
            &mut ledger,
            101,
            FieldPromotionTrigger::MutualEngagementOrder {
                initiator: biped_before.object(),
                opponent: ObjId(899),
            },
            1,
            1_010,
        )?;
        check!(!can_run_surface_mining_for_unit(
            &site_book,
            biped_before.object()
        ));

        // 3. Workshop cost quote and component rearming (`H3`, `Q-S12`, `Q-A4`).
        let cannon = ComponentId::new("cannon")?;
        let machine_gun = ComponentId::new("machine_gun")?;
        let ammo = CellId::new("ammo")?;
        let disarm_plan = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetCell {
                    addr: CellAddr::Component {
                        component: cannon.clone(),
                        cell: ammo.clone(),
                    },
                    value: 0,
                },
                UnitMutationOp::SetCell {
                    addr: CellAddr::Component {
                        component: machine_gun,
                        cell: ammo.clone(),
                    },
                    value: 0,
                },
            ],
            Some(biped_before.revision()),
            None,
        )?;
        let (depleted_biped, _) = apply(&biped_before, &disarm_plan, 1_100)?;
        let depleted_assess = assess_tacsim_unit(&depleted_biped, Some(&biped_spec))?;
        check_eq!(depleted_assess.state, OperationalState::Unarmed);

        let owner = ObjId(10);
        let custody = CustodyTitleStatus::ResolvedTitle { owner };
        let mut workshop = WorkshopBook::new();
        let tariff = WorkshopFacilityTariff::default();
        let quote = workshop.quote_full_restoration(&depleted_biped, &anatomy, &tariff)?;
        check!(!quote.components_to_rearm.is_empty());
        check!(quote.total_resource_cost > 0);

        let (_rearmed_biped, rearmed_assess, order) = workshop.rearm_component(
            &depleted_biped,
            &anatomy,
            &cannon,
            &ammo,
            10,
            owner,
            &custody,
            false,
            150,
            60,
            None,
            1_200,
        )?;
        check_eq!(
            order.kind,
            WorkshopOperationKind::RearmComponent {
                component: cannon,
                cell: ammo,
                rounds_added: 10,
            }
        );
        check_eq!(rearmed_assess.state, OperationalState::Operational);
        check!(rearmed_assess.can_fire);

        Ok(())
    }

    #[test]
    fn sortie_roster_crew_pool_manifest_and_live_arena_reconciliation() -> TestResult {
        use mush_tactics::detachment::{
            Controller as DetController, DetachmentId, DetachmentRoster, LeadershipPolicy, Member,
            Slot,
        };

        use crate::arena::state::PartyId;
        use crate::arena::test_support::{arena_of, line_tank, pose as arena_pose};

        let site = ObjId(795);
        let carrier = ObjId(950);
        let home_port = ObjId(960);
        let organizer = ObjId(50);
        let pilot_red = ObjId(60);
        let npc_blue = ObjId(70);
        let red_tank_id = ObjId(810);
        let blue_tank_id = ObjId(811);

        let catalog = load_stock_catalog()?;
        let (red_before, tank_spec) =
            instantiate_stock_unit(&catalog, "tacsim.unit.line_tank", red_tank_id)?;
        let (blue_before, _) =
            instantiate_stock_unit(&catalog, "tacsim.unit.line_tank", blue_tank_id)?;
        let red_anatomy = anatomy_for_unit_spec(&tank_spec)?;
        let blue_anatomy = anatomy_for_unit_spec(&tank_spec)?;

        let mut red_custody = line_tank(red_tank_id, PartyId::RED, arena_pose(1_500, 1_500));
        red_custody.unit = red_before.clone();
        let mut blue_custody = line_tank(blue_tank_id, PartyId::BLUE, arena_pose(2_500, 1_500));
        blue_custody.unit = blue_before.clone();

        // 1. Assemble detachment roster, manifest, crew pool, and active sortie (`FO-6`–`FO-9`, `FO-15`, `FO-16`, `P-1`).
        let s0 = Slot::new(1)?;
        let s1 = Slot::new(2)?;
        let members = vec![
            Member {
                slot: s0,
                unit: red_tank_id,
                controller: DetController::Player {
                    character: pilot_red,
                },
            },
            Member {
                slot: s1,
                unit: blue_tank_id,
                controller: DetController::Npc {
                    character: npc_blue,
                },
            },
        ];
        let mut roster = DetachmentRoster::new(
            DetachmentId(55),
            1,
            members,
            Some(npc_blue),
            LeadershipPolicy::default(),
        )?;

        let mut manifest = CarrierPersonnelManifest::new(carrier)?;
        manifest.embark_person(
            pilot_red,
            CrewMemberKind::Player,
            EmbarkationOrigin::LastPortDeparture { port: home_port },
            1_000,
        )?;
        manifest.embark_person(
            npc_blue,
            CrewMemberKind::HumanNpc,
            EmbarkationOrigin::LastPortDeparture { port: home_port },
            1_000,
        )?;

        let mut crew_pool = PortCrewPool::new();
        crew_pool.register_available_npc(
            npc_blue,
            home_port,
            200,
            NpcMissionScope::DetachmentCommander,
        )?;
        crew_pool.hire_npc(npc_blue, organizer, home_port, 1_000)?;

        let mut sortie = ActiveFieldSortie::stage_from_roster(
            1,
            organizer,
            Some(carrier),
            &roster,
            &crew_pool,
            1_000,
        )?;
        sortie.set_unit_expense_policy(
            red_tank_id,
            UnitExpensePolicy {
                ammo_payer: organizer,
                recovery_payer: pilot_red,
                repair_payer: organizer,
            },
        )?;
        check_eq!(
            sortie
                .expense_policies()
                .get(&red_tank_id)
                .map(|p| p.payer_for(UnitExpenseCategory::AmmunitionAndConsumables)),
            Some(organizer)
        );
        check_eq!(
            sortie
                .expense_policies()
                .get(&red_tank_id)
                .map(|p| p.payer_for(UnitExpenseCategory::FieldRecoveryAndTowing)),
            Some(pilot_red)
        );

        let mut ledger = FieldCustodyLedger::new();
        let mut site_book = FieldSiteEngagementBook::new(site)?;
        let mut crew_book = CrewSurvivalBook::new();
        let mut obs_book = FieldObservationBook::new();

        crew_book.board_unit(pilot_red, red_tank_id)?;
        crew_book.board_unit(npc_blue, blue_tank_id)?;

        let s0_launch = sortie.launch_slot(
            s0,
            &roster,
            Some(&mut manifest),
            &crew_pool,
            &crew_book,
            1_010,
        )?;
        check_eq!(
            s0_launch,
            SortieSlotLaunchState::Launched {
                launched_at_ic: 1_010
            }
        );
        let s1_launch = sortie.launch_slot(
            s1,
            &roster,
            Some(&mut manifest),
            &crew_pool,
            &crew_book,
            1_010,
        )?;
        check_eq!(
            s1_launch,
            SortieSlotLaunchState::Launched {
                launched_at_ic: 1_010
            }
        );

        site_book.deploy_surface_unit(
            &mut ledger,
            &red_before,
            &red_anatomy,
            SmolStr::new("red"),
            FieldPose {
                site,
                x_cm: 1_500,
                y_cm: 1_500,
            },
            Some(carrier),
            1_010,
        )?;
        site_book.deploy_surface_unit(
            &mut ledger,
            &blue_before,
            &blue_anatomy,
            SmolStr::new("blue"),
            FieldPose {
                site,
                x_cm: 2_500,
                y_cm: 1_500,
            },
            Some(carrier),
            1_010,
        )?;
        site_book.promote_to_engagement(
            &mut ledger,
            500,
            FieldPromotionTrigger::ExplicitHostileAction {
                attacker: blue_tank_id,
                target: red_tank_id,
            },
            1,
            1_020,
        )?;

        // 2. Run a live `Arena` where `red_tank_id` suffers hull destruction (`Wreck`), and reconcile
        // into `site_book`, `ledger`, `crew_book`, `obs_book`, and `sortie` leadership (`Q-H6`, `Q-H8`, `P-1`).
        let mut arena = arena_of(vec![red_custody, blue_custody]);
        let hull = SectionId::new("hull")?;
        let internals = CellId::new("internals")?;
        let wreck_plan = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: hull.clone(),
                        cell: internals,
                    },
                    value: 0,
                },
                UnitMutationOp::SetSectionCondition {
                    section: hull,
                    condition: ConditionId::new("wreck")?,
                },
            ],
            Some(red_before.revision()),
            None,
        )?;
        if let Some(red_state) = arena.unit_mut(red_tank_id) {
            let (wrecked_unit, _) = apply(&red_state.unit, &wreck_plan, 1_025)?;
            red_state.unit = wrecked_unit;
        }
        let mut notices = Vec::new();
        arena.destroy_unit(red_tank_id, 1_025, Some(blue_tank_id), &mut notices);

        let mut pre_combat = BTreeMap::new();
        pre_combat.insert(red_tank_id, (red_before, None));
        pre_combat.insert(blue_tank_id, (blue_before, None));
        let mut unit_pilots = BTreeMap::new();
        unit_pilots.insert(red_tank_id, pilot_red);
        unit_pilots.insert(blue_tank_id, npc_blue);

        let report = reconcile_arena_to_field_engagement(
            &arena,
            &notices,
            &pre_combat,
            &unit_pilots,
            &mut site_book,
            &mut ledger,
            &mut crew_book,
            &mut obs_book,
            1_030,
        )?;
        let (_, red_post_assess) = report
            .updated_units
            .get(&red_tank_id)
            .ok_or("missing red_tank_id in reconciliation report")?;
        check_eq!(red_post_assess.state, OperationalState::Wreck);
        check!(red_post_assess.is_towable);
        // Turret cannon and visual_sensor survive intact on a hull-wrecked tank (`FO-1`)!
        check!(
            red_post_assess
                .intact_components
                .contains(&ComponentId::new("cannon")?)
        );
        check!(report.observations_recorded > 0);
        check!(!obs_book.digest_for_observer(blue_tank_id).is_empty());

        // Pilot survived (`FO-12`) and detachment leadership automatically succeeds to `s1` (`npc_blue`, `P-1`, `Q-F2`).
        let connections = BTreeMap::new();
        let mut command_authorized = BTreeSet::new();
        command_authorized.insert(s1);
        let change = sortie.reconcile_leadership_from_field(
            &mut roster,
            &site_book,
            &crew_book,
            &connections,
            &command_authorized,
            1_035,
            1_035,
        )?;
        check!(change.is_some());
        check_eq!(
            roster.leader().map(|l| (l.slot, l.character)),
            Some((s1, npc_blue))
        );
        check_eq!(
            sortie.leader_grant(),
            Some(&MissionLeaderRoleGrant {
                sortie_id: 1,
                leader: npc_blue,
                leader_kind: CrewMemberKind::HumanNpc,
                command_slot: s1,
                term: roster.term(),
                can_direct_tactical_units: true,
                can_authorize_field_recovery: true,
                grants_persistent_org_authority: false,
                granted_at_ic: 1_035,
            })
        );

        Ok(())
    }

    #[test]
    fn stock_line_tank_tows_wrecked_biped_mk1_with_coupled_drive_rates_and_mid_tow_interdiction()
    -> TestResult {
        let catalog = load_stock_catalog()?;
        let (tug_unit, tug_spec) =
            instantiate_stock_unit(&catalog, "tacsim.unit.line_tank", ObjId(910))?;
        let (casualty_before, casualty_spec) =
            instantiate_stock_unit(&catalog, "tacsim.unit.biped_mk1", ObjId(911))?;
        let (attacker_unit, attacker_spec) =
            instantiate_stock_unit(&catalog, "tacsim.unit.line_tank", ObjId(912))?;
        let site = ObjId(950);

        // Wreck `biped_mk1` torso so it is a towable wreck.
        let torso = SectionId::new("torso")?;
        let internals = CellId::new("internals")?;
        let wreck_plan = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: torso.clone(),
                        cell: internals,
                    },
                    value: 0,
                },
                UnitMutationOp::SetSectionCondition {
                    section: torso,
                    condition: ConditionId::new("wreck")?,
                },
            ],
            Some(casualty_before.revision()),
            None,
        )?;
        let (casualty_wreck, _) = apply(&casualty_before, &wreck_plan, 1_000)?;
        let tug_anatomy = anatomy_for_unit_spec(&tug_spec)?;
        let casualty_anatomy = anatomy_for_unit_spec(&casualty_spec)?;
        let attacker_anatomy = anatomy_for_unit_spec(&attacker_spec)?;
        let casualty_assess = assess_tactical_unit(&casualty_wreck, &casualty_anatomy);
        check_eq!(casualty_assess.state, OperationalState::Wreck);

        let start_pose = FieldPose {
            site,
            x_cm: 100,
            y_cm: 100,
        };
        let mut ledger = FieldCustodyLedger::new();
        let mut site_book = FieldSiteEngagementBook::new(site)?;
        site_book.deploy_surface_unit(
            &mut ledger,
            &tug_unit,
            &tug_anatomy,
            SmolStr::new("expedition"),
            start_pose,
            None,
            1_000,
        )?;
        site_book.deploy_surface_unit(
            &mut ledger,
            &casualty_wreck,
            &casualty_anatomy,
            SmolStr::new("expedition"),
            start_pose,
            None,
            1_000,
        )?;
        site_book.deploy_surface_unit(
            &mut ledger,
            &attacker_unit,
            &attacker_anatomy,
            SmolStr::new("raiders"),
            FieldPose {
                site,
                x_cm: 1_500,
                y_cm: 400,
            },
            None,
            1_000,
        )?;

        let tug_mass = unit_mass_profile_for_spec(&tug_spec, 0, 0)?;
        let casualty_mass = unit_mass_profile_for_spec(&casualty_spec, 0, 0)?;
        let tug_speed = base_speed_cms_for_spec(&tug_spec);

        let mut recovery_book = RecoveryBook::new();
        let op_id = site_book.attach_site_tow(
            &mut recovery_book,
            AttachTowRequest {
                tug: tug_unit.object(),
                tug_state: OperationalState::Operational,
                tug_can_move: true,
                tug_base_speed_cms: tug_speed,
                tug_mass_kg: tug_mass.total_mass_kg(),
                rig_profile: TowRigProfile::Standard,
                tug_pose: start_pose,
                casualty_assessment: casualty_assess,
                casualty_mass,
                casualty_pose: start_pose,
                max_attach_distance_cm: DEFAULT_MAX_ATTACH_DISTANCE_CM,
                at_ic: 1_010,
            },
        )?;
        let op = recovery_book.get(op_id).ok_or("expected tow op")?;
        let base_rates = DriveRates {
            max_speed_cms: tug_speed,
            cu_per_nominal_cell: 20,
        };
        let coupled_rates = coupled_drive_rates_for_tow(&base_rates, op);
        check!(coupled_rates.max_speed_cms < base_rates.max_speed_cms);
        check!(coupled_rates.cu_per_nominal_cell > base_rates.cu_per_nominal_cell);

        // Advance one coupled surface drive leg (cost = 150 = 1 regolith cell).
        let waypoint = FieldPose {
            site,
            x_cm: 350,
            y_cm: 100,
        };
        let (after_leg, leg_secs, leg_cu) = step_coupled_surface_tow_drive_leg(
            &mut site_book,
            &mut ledger,
            &mut recovery_book,
            op_id,
            1,
            &base_rates,
            150,
            waypoint,
            1_010,
        )?;
        check!(leg_secs > 0);
        check!(leg_cu > 0);
        check_eq!(after_leg.stage().active_pose(), Some(waypoint));
        check_eq!(
            site_book.unit_presence(tug_unit.object()).map(|u| u.pose),
            Some(waypoint)
        );
        check_eq!(
            site_book
                .unit_presence(casualty_wreck.object())
                .map(|u| u.pose),
            Some(waypoint)
        );

        // Mid-tow combat interdiction promotes the site to active combat at `waypoint`, and
        // emergency drop detaches the casualty in place (`H1-B` + `H2-A`).
        let handoffs = site_book.interdict_active_tow(
            &mut ledger,
            &recovery_book,
            op_id,
            attacker_unit.object(),
            77,
            1,
            1_050,
        )?;
        check!(site_book.is_engaged());
        check!(!handoffs.is_empty());
        let drop_pose = site_book.emergency_drop_tow(&mut recovery_book, op_id, 2, 1_052)?;
        check_eq!(drop_pose, waypoint);
        check_eq!(
            recovery_book.active_operation_for_tug(tug_unit.object()),
            None
        );

        Ok(())
    }

    #[test]
    fn stock_arena_black_box_forensics_and_contraband_component_workshop_refusal() -> TestResult {
        let catalog = load_stock_catalog()?;
        let (victim_before, victim_spec) =
            instantiate_stock_unit(&catalog, "tacsim.unit.line_tank", ObjId(920))?;
        let (raider_unit, _) =
            instantiate_stock_unit(&catalog, "tacsim.unit.line_tank", ObjId(921))?;
        let (recipient_before, recipient_spec) =
            instantiate_stock_unit(&catalog, "tacsim.unit.line_tank", ObjId(922))?;

        let bay = ObjId(961);
        let owner_id = ObjId(10);
        let raider_id = ObjId(20);
        let op_id = RecoveryOpId::new(88)?;

        let mut salvage_book = SalvageClaimBook::new();
        let mut workshop = WorkshopBook::new();

        // Record combat hit in live `Arena` notices onto `victim_before`'s black box (`FO-4`, `Q-S4`).
        let notices: Vec<Notice> = vec![(
            1_100,
            EventKind::SectionDestroyed {
                unit: victim_before.object(),
                section: SmolStr::new_static("hull"),
                by: Some(raider_unit.object()),
                consequences: Vec::new(),
            },
            Vec::new(),
        )];
        let mut states = BTreeMap::new();
        states.insert(victim_before.object(), OperationalState::Wreck);
        let recorded =
            record_arena_notices_to_black_box(&mut salvage_book, &notices, &states, 1_100)?;
        check_eq!(recorded, 1);

        // Wreck `victim_before`'s hull section while keeping `cannon` intact.
        let hull = SectionId::new("hull")?;
        let internals = CellId::new("internals")?;
        let cannon = ComponentId::new("cannon")?;
        let wreck_plan = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: hull.clone(),
                        cell: internals,
                    },
                    value: 0,
                },
                UnitMutationOp::SetSectionCondition {
                    section: hull,
                    condition: ConditionId::new("wreck")?,
                },
            ],
            Some(victim_before.revision()),
            None,
        )?;
        let (victim_wreck, _) = apply(&victim_before, &wreck_plan, 1_100)?;
        let victim_anatomy = anatomy_for_unit_spec(&victim_spec)?;
        let recipient_anatomy = anatomy_for_unit_spec(&recipient_spec)?;

        let unlawful_auth_ctx = RecoveryAuthorityContext {
            recoverer: raider_id,
            unit_title_holder: owner_id,
            has_owner_authorization: false,
            has_landholder_authorization: false,
            ground_tenure_at_attach: GroundTenure::Owned {
                landholder: owner_id,
                claim_id: Some(SmolStr::new("deed.beta")),
            },
        };
        salvage_book.record_physical_attach(
            op_id,
            victim_wreck.object(),
            raider_unit.object(),
            &unlawful_auth_ctx,
            1_110,
        )?;

        // Raider unlawfully hot-strips `cannon` on `owner_id`'s land (`FO-4`, `Q-F6`).
        let (_, _, stripped_cannon) = workshop.strip_component(
            &victim_wreck,
            &victim_anatomy,
            &cannon,
            raider_id,
            &StripAuthorization::UnlawfulFieldHotStrip {
                authority_ctx: unlawful_auth_ctx,
            },
            Some(&mut salvage_book),
            1_120,
        )?;
        check_eq!(
            stripped_cannon.provenance.legality,
            ProvenanceLegalityStatus::ContrabandHotPart {
                rightful_owner: owner_id,
                reason: ContrabandReason::UnlawfulFieldStripOnOwnedGround,
            }
        );

        // Load hull into bay and extract black-box forensics (`FO-4`, `Q-S4`, `P-3`).
        salvage_book.record_bay_load(op_id, bay, 1_150)?;
        let forensics = salvage_book.extract_black_box_forensics(op_id, owner_id, true, 1_200)?;
        check!(forensics.reveals_unlawful_recovery);
        check_eq!(forensics.unlawful_stripped_components, vec![cannon.clone()]);

        // Prepare `recipient_before` with a destroyed `cannon` so we can attempt installation.
        let break_cannon_plan = UnitMutationPlan::new(
            vec![UnitMutationOp::SetComponentCondition {
                component: cannon.clone(),
                condition: ConditionId::new("destroyed")?,
            }],
            Some(recipient_before.revision()),
            None,
        )?;
        let (recipient_broken, _) = apply(&recipient_before, &break_cannon_plan, 1_220)?;

        // Legitimate workshop refuses to install the contraband cannon (`Q-F6`).
        let replacement_spec = ReplacementPartSpec::from_stripped_record(&stripped_cannon);
        let install_refused = workshop.replace_component_in_slot(
            &recipient_broken,
            &recipient_anatomy,
            &cannon,
            &replacement_spec,
            raider_id,
            &CustodyTitleStatus::ResolvedTitle { owner: raider_id },
            false,
            600,
            120,
            Some(&mut salvage_book),
            1_250,
        );
        check_eq!(
            install_refused,
            Err(WorkshopError::ContrabandComponentRefused {
                record_id: stripped_cannon.record_id,
                parent_hull: victim_wreck.object(),
                rightful_owner: owner_id,
                reason: ContrabandReason::UnlawfulFieldStripOnOwnedGround,
            })
        );

        // Once rightful owner clears contraband provenance, installation succeeds.
        workshop.clear_contraband_provenance(stripped_cannon.record_id, owner_id)?;
        let (restored_recipient, _, _) = workshop.replace_component_in_slot(
            &recipient_broken,
            &recipient_anatomy,
            &cannon,
            &replacement_spec,
            owner_id,
            &CustodyTitleStatus::ResolvedTitle { owner: owner_id },
            true,
            600,
            120,
            Some(&mut salvage_book),
            1_260,
        )?;
        let recipient_assess = assess_tacsim_unit(&restored_recipient, Some(&recipient_spec))?;
        check_eq!(
            recipient_assess.component_conditions.get(&cannon),
            Some(&ComponentCondition::Intact)
        );

        Ok(())
    }

    #[test]
    fn stock_tv_prospector_tender_services_damaged_line_tank_and_enforces_workshop_boundary()
    -> TestResult {
        let catalog = load_stock_catalog()?;
        let (tender_unit, _) =
            instantiate_stock_unit(&catalog, "tacsim.unit.tv_prospector", ObjId(930))?;
        let (mut tank_unit, tank_spec) =
            instantiate_stock_unit(&catalog, "tacsim.unit.line_tank", ObjId(931))?;
        let site = ObjId(970);
        let pose = FieldPose {
            site,
            x_cm: 400,
            y_cm: 400,
        };

        let mut ledger = FieldCustodyLedger::new();
        let mut site_book = FieldSiteEngagementBook::new(site)?;
        let tank_anatomy = anatomy_for_unit_spec(&tank_spec)?;
        site_book.deploy_surface_unit(
            &mut ledger,
            &tank_unit,
            &tank_anatomy,
            SmolStr::new("expedition"),
            pose,
            None,
            1_000,
        )?;
        let recovery_book = RecoveryBook::new();
        let mut eng_book = FieldEngineeringBook::new();
        eng_book.register_tender(FieldTenderState {
            tender_unit: tender_unit.object(),
            site_id: site,
            pose,
            operational_state: OperationalState::Operational,
            spare_parts_kg: 400,
            ammo_supply_rounds: 50,
            charge_reserve_cu: 150,
            max_service_distance_cm: DEFAULT_MAX_TENDER_SERVICE_DISTANCE_CM,
        })?;

        // Jam `cannon`, damage `turret`, deplete `cannon` ammo, and destroy `left_track`.
        let cannon = ComponentId::new("cannon")?;
        let turret = SectionId::new("turret")?;
        let left_track = SectionId::new("left_track")?;
        let internals = CellId::new("internals")?;
        let ammo = CellId::new("ammo")?;

        let damage_plan = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetComponentCondition {
                    component: cannon.clone(),
                    condition: ConditionId::new("jammed")?,
                },
                UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: turret.clone(),
                        cell: internals.clone(),
                    },
                    value: 4,
                },
                UnitMutationOp::SetSectionCondition {
                    section: turret.clone(),
                    condition: ConditionId::new("damaged")?,
                },
                UnitMutationOp::SetCell {
                    addr: CellAddr::Component {
                        component: cannon.clone(),
                        cell: ammo,
                    },
                    value: 2,
                },
                UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: left_track.clone(),
                        cell: internals,
                    },
                    value: 0,
                },
                UnitMutationOp::SetSectionCondition {
                    section: left_track.clone(),
                    condition: ConditionId::new("destroyed")?,
                },
            ],
            Some(tank_unit.revision()),
            None,
        )?;
        let (damaged_tank, _) = apply(&tank_unit, &damage_plan, 1_010)?;
        tank_unit = damaged_tank;

        // 1. Field unjam `cannon`.
        let unjam = field_unjam_tacsim_unit(
            &mut eng_book,
            &site_book,
            &recovery_book,
            tender_unit.object(),
            &mut tank_unit,
            Some(&tank_spec),
            &cannon,
            None,
            DEFAULT_UNJAM_SPARE_PARTS_KG,
            30,
            1_040,
        )?;
        check_eq!(
            unjam.action,
            FieldEngineeringActionKind::UnjamComponent {
                component_id: cannon.clone(),
                restored_condition: ComponentCondition::Intact,
                spare_parts_consumed_kg: DEFAULT_UNJAM_SPARE_PARTS_KG,
            }
        );

        // 2. Field stabilize damaged `turret` (+4 points).
        let patch = field_stabilize_tacsim_unit_section(
            &mut eng_book,
            &site_book,
            &recovery_book,
            tender_unit.object(),
            &mut tank_unit,
            Some(&tank_spec),
            &turret,
            4,
            DEFAULT_SPARE_PARTS_KG_PER_STRUCTURAL_POINT,
            45,
            1_085,
        )?;
        check_eq!(
            patch.action,
            FieldEngineeringActionKind::StabilizeDamagedSection {
                section_id: turret,
                points_restored: 4,
                spare_parts_consumed_kg: 100,
            }
        );

        // 3. Refuses to reconstruct destroyed `left_track` in the field (`H3` workshop boundary).
        let track_res = field_stabilize_tacsim_unit_section(
            &mut eng_book,
            &site_book,
            &recovery_book,
            tender_unit.object(),
            &mut tank_unit,
            Some(&tank_spec),
            &left_track,
            10,
            DEFAULT_SPARE_PARTS_KG_PER_STRUCTURAL_POINT,
            60,
            1_145,
        );
        check_eq!(
            track_res,
            Err(FieldOpsError::Engineering(
                FieldEngineeringError::RequiresWorkshopFacility {
                    unit: tank_unit.object(),
                    reason: SmolStr::new_static(
                        "destroyed section cannot be reconstructed in the field; workshop facility required",
                    ),
                }
            ))
        );

        // 4. Field resupply `cannon` ammo and transfer 25 CU charge.
        let resupply = field_resupply_tacsim_unit(
            &mut eng_book,
            &site_book,
            &recovery_book,
            tender_unit.object(),
            &mut tank_unit,
            Some(&tank_spec),
            Some((&cannon, 6)),
            25,
            10,
            2,
            None,
            20,
            1_165,
        )?;
        check_eq!(
            resupply.action,
            FieldEngineeringActionKind::FieldResupply {
                component_id: Some(cannon),
                rounds_transferred: 6,
                charge_transferred_cu: 25,
                billed_credits: 110,
                billed_payer: None,
            }
        );

        Ok(())
    }
}

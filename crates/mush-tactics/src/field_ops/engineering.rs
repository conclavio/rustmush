//! Field Engineering, Mobile Recovery Rigs & Resupply Tenders (`H1-A`, `H2-A`, `H3`
//! Field vs. Workshop Boundary).
//!
//! Enforces the architectural boundary between field triage/resupply and full hangar/bay
//! workshop overhaul:
//! - **Allowed in the field** (outside active combat, within tender service range):
//!   - Unjamming a [`ComponentCondition::Jammed`] weapon or subsystem in-place.
//!   - Stabilizing/patching structural points on a [`SectionCondition::Damaged`] or partially
//!     depleted [`SectionCondition::Intact`] section using carried spare parts.
//!   - Transferring field ammunition rounds and power charge (`CU`) from a tender or resupply rig,
//!     with optional [`UnitExpensePolicy`] attribution ([`UnitExpenseCategory::AmmunitionAndConsumables`]).
//! - **Refused in the field (`RequiresWorkshopFacility`)**:
//!   - Reconstructing a [`SectionCondition::Destroyed`] section (`H3`).
//!   - Repairing or replacing a [`ComponentCondition::Destroyed`] or `"stripped"` component (`H3`).
//!   - Servicing an [`OperationalState::Irrecoverable`] wreck.

use std::collections::BTreeMap;
use std::fmt;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::field_ops::condition::{
    ComponentCondition, FieldUnitAssessment, OperationalState, SectionCondition,
    UnitAnatomyProfile, assess_tactical_unit, effective_component_condition,
    effective_section_condition, install_point_section,
};
use crate::field_ops::recovery::FieldPose;
use crate::field_ops::sortie::{UnitExpenseCategory, UnitExpensePolicy};
use crate::unit::{
    CellAddr, CellId, ComponentId, ConditionId, SectionId, TacticalUnit, UnitError, UnitMutationOp,
    UnitMutationPlan, apply,
};

/// Default maximum distance in centimeters (`1,000 cm` = `10 m`) for field tender service.
pub const DEFAULT_MAX_TENDER_SERVICE_DISTANCE_CM: u64 = 1_000;

/// Default spare parts mass in kilograms consumed per structural point patched in the field.
pub const DEFAULT_SPARE_PARTS_KG_PER_STRUCTURAL_POINT: u64 = 25;

/// Default spare parts mass in kilograms consumed to clear a field weapon/component jam.
pub const DEFAULT_UNJAM_SPARE_PARTS_KG: u64 = 10;

/// Maximum mobile tenders tracked in one [`FieldEngineeringBook`].
pub const MAX_FIELD_TENDERS: usize = 256;

/// Maximum field engineering log entries retained in one [`FieldEngineeringBook`].
pub const MAX_FIELD_ENGINEERING_LOGS: usize = 2_048;

/// Live state and consumable reserves of a mobile recovery rig or field resupply tender.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldTenderState {
    /// Tender unit object id (`!= ObjId(0)`).
    pub tender_unit: ObjId,
    /// Site where the tender is currently operating (`!= ObjId(0)`).
    pub site_id: ObjId,
    /// Current surface pose of the tender.
    pub pose: FieldPose,
    /// Current operational state of the tender (`Operational` or `Unarmed` required to service).
    pub operational_state: OperationalState,
    /// Remaining field spare parts mass in kilograms.
    pub spare_parts_kg: u64,
    /// Remaining field ammunition reserve in rounds.
    pub ammo_supply_rounds: i64,
    /// Remaining transferable power charge reserve in charge units (`CU`).
    pub charge_reserve_cu: u32,
    /// Maximum service reach in centimeters (`>= 1`).
    pub max_service_distance_cm: u64,
}

/// Kind of field engineering or resupply action executed by a tender.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldEngineeringActionKind {
    /// Cleared a [`ComponentCondition::Jammed`] state in the field.
    UnjamComponent {
        /// Unjammed component id.
        component_id: ComponentId,
        /// Restored condition (`Intact` or `Damaged`).
        restored_condition: ComponentCondition,
        /// Spare parts mass consumed in kilograms.
        spare_parts_consumed_kg: u64,
    },
    /// Patched structural points on a non-destroyed section in the field.
    StabilizeDamagedSection {
        /// Patched section id.
        section_id: SectionId,
        /// Structural points restored.
        points_restored: i64,
        /// Spare parts mass consumed in kilograms.
        spare_parts_consumed_kg: u64,
    },
    /// Transferred field ammunition rounds and/or power charge (`CU`).
    FieldResupply {
        /// Rearmed component id, if ammunition was transferred.
        component_id: Option<ComponentId>,
        /// Ammunition rounds transferred.
        rounds_transferred: i64,
        /// Power charge (`CU`) transferred.
        charge_transferred_cu: u32,
        /// Consumable cost in credits, if billed.
        billed_credits: u64,
        /// Payer principal resolved from [`UnitExpensePolicy`], if provided.
        billed_payer: Option<ObjId>,
    },
}

/// Audit log entry for a completed field engineering or resupply action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldEngineeringLogEntry {
    /// Monotonic log entry id (`>= 1`).
    pub entry_id: u64,
    /// Tender unit that performed the service.
    pub tender_unit: ObjId,
    /// Serviced target unit.
    pub target_unit: ObjId,
    /// Site where the service took place.
    pub site_id: ObjId,
    /// Action details.
    pub action: FieldEngineeringActionKind,
    /// IC duration required to complete the field work (`>= 0`).
    pub work_duration_ic: i64,
    /// IC timestamp when the action completed.
    pub completed_at_ic: i64,
    /// Resulting post-service assessment of `target_unit`.
    pub post_assessment: FieldUnitAssessment,
}

/// Ledger of mobile field engineering tenders and completed field triage/resupply actions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldEngineeringBook {
    next_entry_id: u64,
    tenders: BTreeMap<ObjId, FieldTenderState>,
    logs: BTreeMap<u64, FieldEngineeringLogEntry>,
}

impl FieldEngineeringBook {
    /// Create an empty [`FieldEngineeringBook`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// All registered field tenders keyed by tender unit [`ObjId`].
    #[must_use]
    pub const fn tenders(&self) -> &BTreeMap<ObjId, FieldTenderState> {
        &self.tenders
    }

    /// Look up a registered field tender by unit [`ObjId`].
    #[must_use]
    pub fn tender(&self, tender_unit: ObjId) -> Option<&FieldTenderState> {
        self.tenders.get(&tender_unit)
    }

    /// All recorded field engineering log entries keyed by `entry_id`.
    #[must_use]
    pub const fn logs(&self) -> &BTreeMap<u64, FieldEngineeringLogEntry> {
        &self.logs
    }

    /// Register or update a mobile field tender or recovery/resupply rig.
    ///
    /// # Errors
    /// Returns [`FieldEngineeringError`] if IDs are zero, `max_service_distance_cm == 0`,
    /// `ammo_supply_rounds < 0`, or the tender table is at capacity.
    pub fn register_tender(
        &mut self,
        tender: FieldTenderState,
    ) -> Result<&FieldTenderState, FieldEngineeringError> {
        if tender.tender_unit.0 == 0 {
            return Err(FieldEngineeringError::InvalidObjectId("tender_unit"));
        }
        if tender.site_id.0 == 0 {
            return Err(FieldEngineeringError::InvalidObjectId("site_id"));
        }
        if tender.max_service_distance_cm == 0 {
            return Err(FieldEngineeringError::InvalidServiceDistance(0));
        }
        if tender.ammo_supply_rounds < 0 {
            return Err(FieldEngineeringError::InvalidSupplyAmount(
                "ammo_supply_rounds must be >= 0",
            ));
        }
        if !self.tenders.contains_key(&tender.tender_unit)
            && self.tenders.len() >= MAX_FIELD_TENDERS
        {
            return Err(FieldEngineeringError::TenderCapacityExceeded {
                max: MAX_FIELD_TENDERS,
            });
        }

        let unit_id = tender.tender_unit;
        self.tenders.insert(unit_id, tender);
        self.tenders
            .get(&unit_id)
            .ok_or(FieldEngineeringError::UnknownTender(unit_id))
    }

    /// Update a registered tender's pose and operational state on its site.
    ///
    /// # Errors
    /// Returns [`FieldEngineeringError::UnknownTender`] if `tender_unit` is not registered.
    pub fn update_tender_pose_and_state(
        &mut self,
        tender_unit: ObjId,
        pose: FieldPose,
        operational_state: OperationalState,
    ) -> Result<&FieldTenderState, FieldEngineeringError> {
        let tender = self
            .tenders
            .get_mut(&tender_unit)
            .ok_or(FieldEngineeringError::UnknownTender(tender_unit))?;
        tender.pose = pose;
        tender.operational_state = operational_state;
        Ok(tender)
    }

    /// Unjam a [`ComponentCondition::Jammed`] component in the field without a workshop bay.
    ///
    /// Restores the component's condition to `intact` (or `damaged` if `health_cell_id` is
    /// present and currently below its maximum value), consuming `spare_parts_cost_kg` from the
    /// tender's `spare_parts_kg` reserve.
    ///
    /// Refuses with [`FieldEngineeringError::RequiresWorkshopFacility`] if the target hull is
    /// [`OperationalState::Irrecoverable`], the component is [`ComponentCondition::Destroyed`]
    /// or `"stripped"`, or its host section is [`SectionCondition::Destroyed`].
    ///
    /// # Errors
    /// Returns [`FieldEngineeringError`] if the site is in active combat, the world clock is
    /// frozen, the tender is unable to service or out of range, spare parts are insufficient,
    /// or the component is not jammed / requires a workshop facility.
    #[allow(clippy::too_many_arguments)]
    pub fn field_unjam_component(
        &mut self,
        tender_unit: ObjId,
        target_unit: &mut TacticalUnit,
        target_pose: FieldPose,
        anatomy: &UnitAnatomyProfile,
        component_id: &ComponentId,
        health_cell_id: Option<&CellId>,
        spare_parts_cost_kg: u64,
        site_in_combat: bool,
        world_clock_frozen: bool,
        work_duration_ic: i64,
        completed_at_ic: i64,
    ) -> Result<FieldEngineeringLogEntry, FieldEngineeringError> {
        if self.logs.len() >= MAX_FIELD_ENGINEERING_LOGS {
            return Err(FieldEngineeringError::LogCapacityExceeded {
                max: MAX_FIELD_ENGINEERING_LOGS,
            });
        }
        validate_field_environment(site_in_combat, world_clock_frozen, work_duration_ic)?;
        validate_tender_reach(self.tenders.get(&tender_unit), tender_unit, target_pose)?;

        let pre_assessment = assess_tactical_unit(target_unit, anatomy);
        if pre_assessment.state == OperationalState::Irrecoverable {
            return Err(FieldEngineeringError::RequiresWorkshopFacility {
                unit: target_unit.object(),
                reason: SmolStr::new_static(
                    "irrecoverable wreck cannot be serviced by a field tender",
                ),
            });
        }

        let comp = target_unit.component(component_id).ok_or_else(|| {
            FieldEngineeringError::UnknownComponent {
                unit: target_unit.object(),
                component: component_id.clone(),
            }
        })?;
        if comp.condition.as_str() == "stripped" {
            return Err(FieldEngineeringError::RequiresWorkshopFacility {
                unit: target_unit.object(),
                reason: SmolStr::new_static(
                    "stripped component slot requires workshop replacement",
                ),
            });
        }
        if let Some(section_id) = install_point_section(&comp.install)
            && effective_section_condition(target_unit, section_id, anatomy)
                == Some(SectionCondition::Destroyed)
        {
            return Err(FieldEngineeringError::RequiresWorkshopFacility {
                unit: target_unit.object(),
                reason: SmolStr::new_static(
                    "component in destroyed section requires workshop section reconstruction",
                ),
            });
        }

        let current_cond = effective_component_condition(
            target_unit,
            component_id,
            &pre_assessment.section_conditions,
            anatomy,
        )
        .unwrap_or(ComponentCondition::Intact);
        match current_cond {
            ComponentCondition::Destroyed => {
                return Err(FieldEngineeringError::RequiresWorkshopFacility {
                    unit: target_unit.object(),
                    reason: SmolStr::new_static(
                        "destroyed component requires workshop repair or replacement",
                    ),
                });
            }
            ComponentCondition::Intact | ComponentCondition::Damaged => {
                return Err(FieldEngineeringError::ComponentNotJammed {
                    unit: target_unit.object(),
                    component: component_id.clone(),
                    condition: current_cond,
                });
            }
            ComponentCondition::Jammed => {}
        }

        let tender = self
            .tenders
            .get(&tender_unit)
            .ok_or(FieldEngineeringError::UnknownTender(tender_unit))?;
        if tender.spare_parts_kg < spare_parts_cost_kg {
            return Err(FieldEngineeringError::InsufficientSpareParts {
                tender: tender_unit,
                available_kg: tender.spare_parts_kg,
                required_kg: spare_parts_cost_kg,
            });
        }

        let h_cell_opt = health_cell_id.or(anatomy.component_health_cell.as_ref());
        let is_partially_damaged = h_cell_opt.is_some_and(|h_id| {
            target_unit
                .cell(&CellAddr::Component {
                    component: component_id.clone(),
                    cell: h_id.clone(),
                })
                .is_some_and(|cell| cell.current < cell.max)
        });
        let (target_cond_str, restored_condition) = if is_partially_damaged {
            ("damaged", ComponentCondition::Damaged)
        } else {
            ("intact", ComponentCondition::Intact)
        };
        let cond_id = ConditionId::new(target_cond_str).map_err(FieldEngineeringError::Unit)?;

        let plan = UnitMutationPlan::new(
            vec![UnitMutationOp::SetComponentCondition {
                component: component_id.clone(),
                condition: cond_id,
            }],
            Some(target_unit.revision()),
            Some(SmolStr::new_static("field_engineering.unjam")),
        )
        .map_err(FieldEngineeringError::Unit)?;
        let (updated_unit, _) =
            apply(target_unit, &plan, completed_at_ic).map_err(FieldEngineeringError::Unit)?;
        *target_unit = updated_unit;

        let tender_mut = self
            .tenders
            .get_mut(&tender_unit)
            .ok_or(FieldEngineeringError::UnknownTender(tender_unit))?;
        tender_mut.spare_parts_kg = tender_mut
            .spare_parts_kg
            .saturating_sub(spare_parts_cost_kg);
        let site_id = tender_mut.site_id;

        let post_assessment = assess_tactical_unit(target_unit, anatomy);
        let entry = self.record_log(
            tender_unit,
            target_unit.object(),
            site_id,
            FieldEngineeringActionKind::UnjamComponent {
                component_id: component_id.clone(),
                restored_condition,
                spare_parts_consumed_kg: spare_parts_cost_kg,
            },
            work_duration_ic,
            completed_at_ic,
            post_assessment,
        );
        Ok(entry)
    }

    /// Stabilize/patch structural points on a [`SectionCondition::Damaged`] or partially depleted
    /// [`SectionCondition::Intact`] section in the field using spare parts from a tender.
    ///
    /// Enforces the `H3` workshop boundary: [`SectionCondition::Destroyed`] sections and
    /// [`OperationalState::Irrecoverable`] wrecks cannot be reconstructed in the field and fail
    /// with [`FieldEngineeringError::RequiresWorkshopFacility`].
    ///
    /// # Errors
    /// Returns [`FieldEngineeringError`] if the site is in active combat, the world clock is
    /// frozen, the tender is out of range or lacks spare parts, `requested_points <= 0`, or the
    /// section is destroyed / full.
    #[allow(clippy::too_many_arguments)]
    pub fn field_stabilize_damaged_section(
        &mut self,
        tender_unit: ObjId,
        target_unit: &mut TacticalUnit,
        target_pose: FieldPose,
        anatomy: &UnitAnatomyProfile,
        section_id: &SectionId,
        requested_points: i64,
        spare_parts_kg_per_point: u64,
        site_in_combat: bool,
        world_clock_frozen: bool,
        work_duration_ic: i64,
        completed_at_ic: i64,
    ) -> Result<FieldEngineeringLogEntry, FieldEngineeringError> {
        if self.logs.len() >= MAX_FIELD_ENGINEERING_LOGS {
            return Err(FieldEngineeringError::LogCapacityExceeded {
                max: MAX_FIELD_ENGINEERING_LOGS,
            });
        }
        if requested_points <= 0 {
            return Err(FieldEngineeringError::InvalidSupplyAmount(
                "requested_points must be >= 1",
            ));
        }
        validate_field_environment(site_in_combat, world_clock_frozen, work_duration_ic)?;
        validate_tender_reach(self.tenders.get(&tender_unit), tender_unit, target_pose)?;

        let pre_assessment = assess_tactical_unit(target_unit, anatomy);
        if pre_assessment.state == OperationalState::Irrecoverable {
            return Err(FieldEngineeringError::RequiresWorkshopFacility {
                unit: target_unit.object(),
                reason: SmolStr::new_static(
                    "irrecoverable wreck cannot be patched by a field tender",
                ),
            });
        }

        if target_unit.section(section_id).is_none() {
            return Err(FieldEngineeringError::UnknownSection {
                unit: target_unit.object(),
                section: section_id.clone(),
            });
        }
        let eff_sec = effective_section_condition(target_unit, section_id, anatomy)
            .unwrap_or(SectionCondition::Intact);
        if eff_sec == SectionCondition::Destroyed {
            return Err(FieldEngineeringError::RequiresWorkshopFacility {
                unit: target_unit.object(),
                reason: SmolStr::new_static(
                    "destroyed section cannot be reconstructed in the field; workshop facility required",
                ),
            });
        }

        let internals_id =
            anatomy
                .internals_cell
                .as_ref()
                .ok_or(FieldEngineeringError::InvalidSupplyAmount(
                    "anatomy profile has no internals_cell configured",
                ))?;
        let cell_addr = CellAddr::Section {
            section: section_id.clone(),
            cell: internals_id.clone(),
        };
        let cell =
            target_unit
                .cell(&cell_addr)
                .ok_or_else(|| FieldEngineeringError::MissingCell {
                    unit: target_unit.object(),
                    cell: internals_id.clone(),
                })?;
        let missing_points = cell.max.saturating_sub(cell.current);
        if missing_points <= 0 && eff_sec == SectionCondition::Intact {
            return Err(FieldEngineeringError::SectionAlreadyIntact {
                unit: target_unit.object(),
                section: section_id.clone(),
            });
        }

        let points_restored = requested_points.min(missing_points.max(1));
        let new_val = cell.current.saturating_add(points_restored).min(cell.max);
        let new_cond_str = if new_val >= cell.max {
            "intact"
        } else {
            "damaged"
        };
        let points_u64 = u64::try_from(points_restored).unwrap_or(0);
        let required_spare_parts_kg = points_u64.saturating_mul(spare_parts_kg_per_point);

        let tender = self
            .tenders
            .get(&tender_unit)
            .ok_or(FieldEngineeringError::UnknownTender(tender_unit))?;
        if tender.spare_parts_kg < required_spare_parts_kg {
            return Err(FieldEngineeringError::InsufficientSpareParts {
                tender: tender_unit,
                available_kg: tender.spare_parts_kg,
                required_kg: required_spare_parts_kg,
            });
        }

        let cond_id = ConditionId::new(new_cond_str).map_err(FieldEngineeringError::Unit)?;
        let plan = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetCell {
                    addr: cell_addr,
                    value: new_val,
                },
                UnitMutationOp::SetSectionCondition {
                    section: section_id.clone(),
                    condition: cond_id,
                },
            ],
            Some(target_unit.revision()),
            Some(SmolStr::new_static("field_engineering.stabilize_section")),
        )
        .map_err(FieldEngineeringError::Unit)?;
        let (updated_unit, _) =
            apply(target_unit, &plan, completed_at_ic).map_err(FieldEngineeringError::Unit)?;
        *target_unit = updated_unit;

        let tender_mut = self
            .tenders
            .get_mut(&tender_unit)
            .ok_or(FieldEngineeringError::UnknownTender(tender_unit))?;
        tender_mut.spare_parts_kg = tender_mut
            .spare_parts_kg
            .saturating_sub(required_spare_parts_kg);
        let site_id = tender_mut.site_id;

        let post_assessment = assess_tactical_unit(target_unit, anatomy);
        let entry = self.record_log(
            tender_unit,
            target_unit.object(),
            site_id,
            FieldEngineeringActionKind::StabilizeDamagedSection {
                section_id: section_id.clone(),
                points_restored,
                spare_parts_consumed_kg: required_spare_parts_kg,
            },
            work_duration_ic,
            completed_at_ic,
            post_assessment,
        );
        Ok(entry)
    }

    /// Transfer field ammunition rounds and/or power charge (`CU`) from a tender to `target_unit`.
    ///
    /// Resolves the billed payer from `expense_policy` under
    /// [`UnitExpenseCategory::AmmunitionAndConsumables`] (`Q-A4`) when provided.
    ///
    /// # Errors
    /// Returns [`FieldEngineeringError`] if the site is in active combat, the world clock is
    /// frozen, the tender is out of range or lacks ammo/charge reserves, or the target weapon is
    /// destroyed/stripped or housed in a destroyed section.
    #[allow(clippy::too_many_arguments)]
    pub fn field_resupply_ammo_and_charge(
        &mut self,
        tender_unit: ObjId,
        target_unit: &mut TacticalUnit,
        target_pose: FieldPose,
        anatomy: &UnitAnatomyProfile,
        rearm_target: Option<(&ComponentId, &CellId, i64)>,
        requested_charge_cu: u32,
        credit_cost_per_round: u64,
        credit_cost_per_cu: u64,
        expense_policy: Option<&UnitExpensePolicy>,
        site_in_combat: bool,
        world_clock_frozen: bool,
        work_duration_ic: i64,
        completed_at_ic: i64,
    ) -> Result<FieldEngineeringLogEntry, FieldEngineeringError> {
        if self.logs.len() >= MAX_FIELD_ENGINEERING_LOGS {
            return Err(FieldEngineeringError::LogCapacityExceeded {
                max: MAX_FIELD_ENGINEERING_LOGS,
            });
        }
        if rearm_target.is_none() && requested_charge_cu == 0 {
            return Err(FieldEngineeringError::InvalidSupplyAmount(
                "must request ammo rounds or charge CU > 0",
            ));
        }
        validate_field_environment(site_in_combat, world_clock_frozen, work_duration_ic)?;
        validate_tender_reach(self.tenders.get(&tender_unit), tender_unit, target_pose)?;

        let pre_assessment = assess_tactical_unit(target_unit, anatomy);
        if pre_assessment.state == OperationalState::Irrecoverable {
            return Err(FieldEngineeringError::RequiresWorkshopFacility {
                unit: target_unit.object(),
                reason: SmolStr::new_static(
                    "irrecoverable wreck cannot be resupplied in the field",
                ),
            });
        }

        let mut rounds_to_transfer = 0_i64;
        let mut target_comp_cell: Option<(ComponentId, CellId, i64)> = None;

        if let Some((component_id, ammo_cell_id, requested_rounds)) = rearm_target {
            if requested_rounds <= 0 {
                return Err(FieldEngineeringError::InvalidSupplyAmount(
                    "requested_rounds must be >= 1",
                ));
            }
            let comp = target_unit.component(component_id).ok_or_else(|| {
                FieldEngineeringError::UnknownComponent {
                    unit: target_unit.object(),
                    component: component_id.clone(),
                }
            })?;
            let eff_comp = effective_component_condition(
                target_unit,
                component_id,
                &pre_assessment.section_conditions,
                anatomy,
            );
            if comp.condition.as_str() == "stripped"
                || eff_comp == Some(ComponentCondition::Destroyed)
            {
                return Err(FieldEngineeringError::RequiresWorkshopFacility {
                    unit: target_unit.object(),
                    reason: SmolStr::new_static(
                        "destroyed or stripped component cannot be rearmed in the field",
                    ),
                });
            }
            if let Some(sec_id) = install_point_section(&comp.install)
                && effective_section_condition(target_unit, sec_id, anatomy)
                    == Some(SectionCondition::Destroyed)
            {
                return Err(FieldEngineeringError::RequiresWorkshopFacility {
                    unit: target_unit.object(),
                    reason: SmolStr::new_static(
                        "component in destroyed section cannot be rearmed in the field",
                    ),
                });
            }
            let cell_addr = CellAddr::Component {
                component: component_id.clone(),
                cell: ammo_cell_id.clone(),
            };
            let cell =
                target_unit
                    .cell(&cell_addr)
                    .ok_or_else(|| FieldEngineeringError::MissingCell {
                        unit: target_unit.object(),
                        cell: ammo_cell_id.clone(),
                    })?;
            let capacity_left = cell.max.saturating_sub(cell.current);
            rounds_to_transfer = requested_rounds.min(capacity_left).max(0);
            let new_cell_val = cell.current.saturating_add(rounds_to_transfer);
            target_comp_cell = Some((component_id.clone(), ammo_cell_id.clone(), new_cell_val));
        }

        let tender = self
            .tenders
            .get(&tender_unit)
            .ok_or(FieldEngineeringError::UnknownTender(tender_unit))?;
        if tender.ammo_supply_rounds < rounds_to_transfer {
            return Err(FieldEngineeringError::InsufficientAmmoSupply {
                tender: tender_unit,
                available_rounds: tender.ammo_supply_rounds,
                required_rounds: rounds_to_transfer,
            });
        }
        if tender.charge_reserve_cu < requested_charge_cu {
            return Err(FieldEngineeringError::InsufficientChargeReserve {
                tender: tender_unit,
                available_cu: tender.charge_reserve_cu,
                required_cu: requested_charge_cu,
            });
        }

        if let Some((ref comp_id, ref cell_id, new_val)) = target_comp_cell
            && rounds_to_transfer > 0
        {
            let plan = UnitMutationPlan::new(
                vec![UnitMutationOp::SetCell {
                    addr: CellAddr::Component {
                        component: comp_id.clone(),
                        cell: cell_id.clone(),
                    },
                    value: new_val,
                }],
                Some(target_unit.revision()),
                Some(SmolStr::new_static("field_engineering.resupply")),
            )
            .map_err(FieldEngineeringError::Unit)?;
            let (updated_unit, _) =
                apply(target_unit, &plan, completed_at_ic).map_err(FieldEngineeringError::Unit)?;
            *target_unit = updated_unit;
        }

        let tender_mut = self
            .tenders
            .get_mut(&tender_unit)
            .ok_or(FieldEngineeringError::UnknownTender(tender_unit))?;
        tender_mut.ammo_supply_rounds = tender_mut
            .ammo_supply_rounds
            .saturating_sub(rounds_to_transfer);
        tender_mut.charge_reserve_cu = tender_mut
            .charge_reserve_cu
            .saturating_sub(requested_charge_cu);
        let site_id = tender_mut.site_id;

        let rounds_u64 = u64::try_from(rounds_to_transfer).unwrap_or(0);
        let ammo_cost = rounds_u64.saturating_mul(credit_cost_per_round);
        let charge_cost = u64::from(requested_charge_cu).saturating_mul(credit_cost_per_cu);
        let billed_credits = ammo_cost.saturating_add(charge_cost);
        let billed_payer = expense_policy
            .map(|policy| policy.payer_for(UnitExpenseCategory::AmmunitionAndConsumables));

        let post_assessment = assess_tactical_unit(target_unit, anatomy);
        let entry = self.record_log(
            tender_unit,
            target_unit.object(),
            site_id,
            FieldEngineeringActionKind::FieldResupply {
                component_id: target_comp_cell.map(|(cid, _, _)| cid),
                rounds_transferred: rounds_to_transfer,
                charge_transferred_cu: requested_charge_cu,
                billed_credits,
                billed_payer,
            },
            work_duration_ic,
            completed_at_ic,
            post_assessment,
        );
        Ok(entry)
    }

    #[allow(clippy::too_many_arguments)]
    fn record_log(
        &mut self,
        tender_unit: ObjId,
        target_unit: ObjId,
        site_id: ObjId,
        action: FieldEngineeringActionKind,
        work_duration_ic: i64,
        completed_at_ic: i64,
        post_assessment: FieldUnitAssessment,
    ) -> FieldEngineeringLogEntry {
        self.next_entry_id = self.next_entry_id.saturating_add(1);
        let entry_id = self.next_entry_id;
        let entry = FieldEngineeringLogEntry {
            entry_id,
            tender_unit,
            target_unit,
            site_id,
            action,
            work_duration_ic,
            completed_at_ic,
            post_assessment,
        };
        self.logs.insert(entry_id, entry.clone());
        entry
    }
}

fn validate_field_environment(
    site_in_combat: bool,
    world_clock_frozen: bool,
    work_duration_ic: i64,
) -> Result<(), FieldEngineeringError> {
    if world_clock_frozen {
        return Err(FieldEngineeringError::WorldClockFrozen);
    }
    if site_in_combat {
        return Err(FieldEngineeringError::SiteInActiveCombat);
    }
    if work_duration_ic < 0 {
        return Err(FieldEngineeringError::InvalidWorkDuration(work_duration_ic));
    }
    Ok(())
}

fn validate_tender_reach(
    tender: Option<&FieldTenderState>,
    tender_unit: ObjId,
    target_pose: FieldPose,
) -> Result<(), FieldEngineeringError> {
    let tender = tender.ok_or(FieldEngineeringError::UnknownTender(tender_unit))?;
    if !matches!(
        tender.operational_state,
        OperationalState::Operational | OperationalState::Unarmed
    ) {
        return Err(FieldEngineeringError::TenderInoperable {
            tender: tender_unit,
            state: tender.operational_state,
        });
    }
    let distance_cm = tender.pose.distance_cm_to(&target_pose).ok_or(
        FieldEngineeringError::TargetOutOfServiceRange {
            tender: tender_unit,
            distance_cm: u64::MAX,
            max_distance_cm: tender.max_service_distance_cm,
        },
    )?;
    if distance_cm > tender.max_service_distance_cm {
        return Err(FieldEngineeringError::TargetOutOfServiceRange {
            tender: tender_unit,
            distance_cm,
            max_distance_cm: tender.max_service_distance_cm,
        });
    }
    Ok(())
}

/// Errors produced by field engineering and resupply tender operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldEngineeringError {
    /// Required object id was zero (`ObjId(0)`).
    InvalidObjectId(&'static str),
    /// Service distance must be `>= 1` cm.
    InvalidServiceDistance(u64),
    /// Invalid supply quantity or request parameter.
    InvalidSupplyAmount(&'static str),
    /// Work duration in IC seconds was negative.
    InvalidWorkDuration(i64),
    /// Tender table capacity exceeded.
    TenderCapacityExceeded {
        /// Maximum tenders supported.
        max: usize,
    },
    /// Engineering log capacity exceeded.
    LogCapacityExceeded {
        /// Maximum log entries supported.
        max: usize,
    },
    /// Referenced tender is not registered.
    UnknownTender(ObjId),
    /// Field engineering is blocked while the world clock is frozen (`FO-11`).
    WorldClockFrozen,
    /// Field engineering and tender resupply require the site to be out of active combat.
    SiteInActiveCombat,
    /// Tender is immobilized, disabled, or irrecoverable and cannot service other units.
    TenderInoperable {
        /// Tender unit id.
        tender: ObjId,
        /// Current operational state.
        state: OperationalState,
    },
    /// Target unit is outside the tender's service distance.
    TargetOutOfServiceRange {
        /// Tender unit id.
        tender: ObjId,
        /// Measured distance in centimeters.
        distance_cm: u64,
        /// Maximum allowed service distance in centimeters.
        max_distance_cm: u64,
    },
    /// Operation crosses the field-vs-workshop boundary and requires a hangar/carrier bay (`H3`).
    RequiresWorkshopFacility {
        /// Target unit id.
        unit: ObjId,
        /// Explanation of why a workshop is required.
        reason: SmolStr,
    },
    /// Referenced section was not found on the unit.
    UnknownSection {
        /// Unit id.
        unit: ObjId,
        /// Section id.
        section: SectionId,
    },
    /// Referenced component was not found on the unit.
    UnknownComponent {
        /// Unit id.
        unit: ObjId,
        /// Component id.
        component: ComponentId,
    },
    /// Referenced scalar cell was not found.
    MissingCell {
        /// Unit id.
        unit: ObjId,
        /// Cell id.
        cell: CellId,
    },
    /// Component is not jammed (`Intact` or `Damaged`), so unjamming is a no-op.
    ComponentNotJammed {
        /// Unit id.
        unit: ObjId,
        /// Component id.
        component: ComponentId,
        /// Current component condition.
        condition: ComponentCondition,
    },
    /// Section is already intact and at maximum structural points.
    SectionAlreadyIntact {
        /// Unit id.
        unit: ObjId,
        /// Section id.
        section: SectionId,
    },
    /// Tender lacks sufficient spare parts mass (`kg`).
    InsufficientSpareParts {
        /// Tender unit id.
        tender: ObjId,
        /// Available spare parts in kilograms.
        available_kg: u64,
        /// Required spare parts in kilograms.
        required_kg: u64,
    },
    /// Tender lacks sufficient ammunition rounds.
    InsufficientAmmoSupply {
        /// Tender unit id.
        tender: ObjId,
        /// Available rounds.
        available_rounds: i64,
        /// Required rounds.
        required_rounds: i64,
    },
    /// Tender lacks sufficient power charge reserve (`CU`).
    InsufficientChargeReserve {
        /// Tender unit id.
        tender: ObjId,
        /// Available charge units.
        available_cu: u32,
        /// Required charge units.
        required_cu: u32,
    },
    /// Underlying [`TacticalUnit`] mutation error.
    Unit(UnitError),
}

impl fmt::Display for FieldEngineeringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidObjectId(field) => write!(f, "{field} must not be ObjId(0)"),
            Self::InvalidServiceDistance(dist) => {
                write!(
                    f,
                    "tender max_service_distance_cm must be >= 1 (got {dist})"
                )
            }
            Self::InvalidSupplyAmount(msg) => write!(f, "invalid field engineering supply: {msg}"),
            Self::InvalidWorkDuration(secs) => {
                write!(f, "work_duration_ic must be >= 0 (got {secs})")
            }
            Self::TenderCapacityExceeded { max } => {
                write!(f, "field tender capacity exceeded (max {max})")
            }
            Self::LogCapacityExceeded { max } => {
                write!(f, "field engineering log capacity exceeded (max {max})")
            }
            Self::UnknownTender(unit) => write!(f, "unknown field tender {unit}"),
            Self::WorldClockFrozen => {
                write!(
                    f,
                    "field engineering is blocked while the world clock is frozen"
                )
            }
            Self::SiteInActiveCombat => {
                write!(
                    f,
                    "field engineering requires the site to be out of active combat"
                )
            }
            Self::TenderInoperable { tender, state } => {
                write!(f, "field tender {tender} is inoperable ({state:?})")
            }
            Self::TargetOutOfServiceRange {
                tender,
                distance_cm,
                max_distance_cm,
            } => write!(
                f,
                "target is {distance_cm}cm from tender {tender} (max {max_distance_cm}cm)"
            ),
            Self::RequiresWorkshopFacility { unit, reason } => {
                write!(f, "unit {unit} cannot be serviced in the field ({reason})")
            }
            Self::UnknownSection { unit, section } => {
                write!(f, "unknown section {section} on unit {unit}")
            }
            Self::UnknownComponent { unit, component } => {
                write!(f, "unknown component {component} on unit {unit}")
            }
            Self::MissingCell { unit, cell } => {
                write!(f, "missing scalar cell {cell} on unit {unit}")
            }
            Self::ComponentNotJammed {
                unit,
                component,
                condition,
            } => write!(
                f,
                "component {component} on unit {unit} is not jammed ({condition:?})"
            ),
            Self::SectionAlreadyIntact { unit, section } => {
                write!(f, "section {section} on unit {unit} is already intact")
            }
            Self::InsufficientSpareParts {
                tender,
                available_kg,
                required_kg,
            } => write!(
                f,
                "tender {tender} has {available_kg}kg spare parts, needs {required_kg}kg"
            ),
            Self::InsufficientAmmoSupply {
                tender,
                available_rounds,
                required_rounds,
            } => write!(
                f,
                "tender {tender} has {available_rounds} ammo rounds, needs {required_rounds}"
            ),
            Self::InsufficientChargeReserve {
                tender,
                available_cu,
                required_cu,
            } => write!(
                f,
                "tender {tender} has {available_cu} CU charge reserve, needs {required_cu} CU"
            ),
            Self::Unit(err) => write!(f, "unit mutation error: {err}"),
        }
    }
}

impl std::error::Error for FieldEngineeringError {}

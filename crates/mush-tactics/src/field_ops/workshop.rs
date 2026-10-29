//! `H3` — Component stripping with parent-hull provenance, hull scrapping, and
//! workshop repair/reconstruction/component replacement (design doc 27 §4.1, §5,
//! §6 `H3`, `FO-2`, `FO-14`, `Q-S11`, `Q-S12`, `Q-S14`, `Q-F5`, `Q-F6`).

use std::collections::BTreeMap;
use std::fmt;

use mush_core::{DefinitionRef, ObjId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::field_ops::condition::{
    ComponentCondition, FieldUnitAssessment, OperationalState, SectionCondition,
    UnitAnatomyProfile, assess_tactical_unit, effective_component_condition,
    effective_section_condition, install_point_section,
};
use crate::field_ops::salvage::{
    BlackBoxFlightEntry, CustodyAction, CustodyTitleStatus, RecoveryAuthorityContext,
    SalvageClaimBook, SalvageClaimId, SalvageError, check_custody_action_allowed,
    evaluate_recovery_entitlement,
};
use crate::unit::{
    CellAddr, CellId, ComponentId, ConditionId, InstallPoint, ScalarCell, SectionId, TacticalUnit,
    UnitError, UnitMutationOp, UnitMutationPlan, apply,
};

/// Scrap mass in kilograms yielded per remaining structural point during hull scrapping (`Q-S11`).
pub const SCRAP_KG_PER_STRUCTURAL_POINT: u64 = 100;

/// Fallback scrap mass in kilograms per non-destroyed section when no internals cell is configured.
pub const FALLBACK_SECTION_SCRAP_KG: u64 = 1_000;

/// Maximum detached component records retained in one [`WorkshopBook`].
pub const MAX_STRIPPED_COMPONENTS: usize = 2_048;

/// Maximum workshop work-order records retained in one [`WorkshopBook`].
pub const MAX_WORKSHOP_ORDERS: usize = 2_048;

/// Reason a stripped component is flagged as contraband (`FO-4`, `Q-S4`, `Q-F5`, `Q-F6`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContrabandReason {
    /// Detached in the field on someone else's owned ground without owner or landholder authority (`FO-4`, `Q-S4`, `Q-F6`).
    UnlawfulFieldStripOnOwnedGround,
    /// Salvage claim covering the parent hull or component was contested and vacated (`Q-S13`, `Q-F5`, `Q-F6`).
    VacatedSalvageClaim {
        /// Vacated salvage claim id.
        claim_id: SalvageClaimId,
    },
}

/// Legal title/provenance status of a detached component (`FO-4`, `Q-S4`, `Q-F5`, `Q-F6`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProvenanceLegalityStatus {
    /// Lawfully detached under resolved title, explicit owner permission, or lawful field salvage.
    #[default]
    CleanTitle,
    /// Unlawfully detached or derived from a vacated salvage claim; refused by lawful workshops
    /// until cleared by `rightful_owner` (`FO-4`, `Q-S4`, `Q-F6`).
    ContrabandHotPart {
        /// Rightful legal title holder of the parent hull at the time of stripping.
        rightful_owner: ObjId,
        /// Reason the part carries a contraband flag.
        reason: ContrabandReason,
    },
}

/// Provenance recorded on a component stripped from a hull in a bay or in the field (`Q-S11`, `Q-F6`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentProvenance {
    /// Hull object id from which the component was detached.
    pub parent_hull: ObjId,
    /// Unit definition of the parent hull.
    pub parent_schema: DefinitionRef,
    /// Host section on the parent hull, if section-installed.
    pub host_section: Option<SectionId>,
    /// Actor who performed the strip operation.
    pub stripped_by: ObjId,
    /// IC timestamp when the component was stripped.
    pub stripped_at_ic: i64,
    /// Whether the component was stripped in the field (`true`) or in a bay/workshop (`false`).
    pub field_stripped: bool,
    /// Legal title/contraband provenance status (`FO-4`, `Q-S4`, `Q-F6`).
    #[serde(default)]
    pub legality: ProvenanceLegalityStatus,
}

/// Durable record of a surviving component detached from a hull (`FO-2`, `Q-S11`, `Q-F6`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrippedComponentRecord {
    /// Monotonic detached-component record id (`>= 1`).
    pub record_id: u64,
    /// Component slot/instance id on the parent hull.
    pub component_id: ComponentId,
    /// Exact component definition reference.
    pub definition: DefinitionRef,
    /// Original install point on the parent hull.
    pub install: InstallPoint,
    /// Preserved condition (`Intact`, `Damaged`, or `Jammed`).
    pub condition: ComponentCondition,
    /// Preserved component-local scalar cells (such as remaining ammo or health points).
    pub cells: BTreeMap<CellId, ScalarCell>,
    /// Economic asset pointer transferred from the hull slot, if any.
    pub asset: Option<ObjId>,
    /// Parent-hull provenance (`Q-S11`, `Q-F6`).
    pub provenance: ComponentProvenance,
    /// Whether this detached part has already been consumed/installed into another hull.
    pub consumed_into_hull: Option<ObjId>,
}

/// Authorization context for stripping a component (`Q-S11`, `Q-F5`, `Q-F6`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StripAuthorization {
    /// Stripping in a hangar or carrier bay (`Q-F5`): requires resolved title or explicit
    /// existing-title-owner permission (refused while salvage title is pending).
    BayOrWorkshop {
        /// Current custody/title status of the hull.
        custody: CustodyTitleStatus,
        /// Whether the existing legal owner explicitly authorized destructive stripping.
        has_explicit_owner_permission: bool,
    },
    /// Stripping in the field (`Q-F6`): requires lawful field recovery/stripping authority
    /// (`LawfulSelfOrAgent`, `LawfulLandholderOrAgent`, or `LawfulOpenGround`).
    Field {
        /// Field recovery authority context at the hull's site position.
        authority_ctx: RecoveryAuthorityContext,
    },
    /// Physical field stripping regardless of ground entitlement (`FO-4`, `Q-S4`, `Q-F6`):
    /// if `authority_ctx` is `UnlawfulOnOwnedGround`, the physical strip succeeds without a
    /// public theft broadcast, records a `FieldComponentStripped { lawful: false }` entry on
    /// the hull's black box, and flags the detached part as [`ProvenanceLegalityStatus::ContrabandHotPart`].
    UnlawfulFieldHotStrip {
        /// Field recovery authority context at the hull's site position.
        authority_ctx: RecoveryAuthorityContext,
    },
}

/// Deterministic scrap yield produced when dismantling a hull (`Q-S11`, `Q-F5`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScrapYieldReport {
    /// Scrapped hull object id.
    pub hull: ObjId,
    /// Actor who executed the scrap operation.
    pub scrapped_by: ObjId,
    /// Recoverable structural scrap mass in kilograms.
    pub structural_scrap_kg: u64,
    /// Number of remaining unstripped salvageable components dismantled with the hull.
    pub dismantled_salvageable_components: usize,
    /// Resulting assessment (`OperationalState::Irrecoverable`).
    pub final_assessment: FieldUnitAssessment,
    /// IC timestamp when scrapping completed.
    pub scrapped_at_ic: i64,
}

/// Replacement part specification used to replace a `Destroyed` or `"stripped"` component
/// in a workshop (`FO-2`, `Q-S12`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplacementPartSpec {
    /// Optional [`StrippedComponentRecord::record_id`] if sourced from a salvaged part.
    pub source_stripped_record_id: Option<u64>,
    /// Component definition reference (must match the target slot's definition).
    pub definition: DefinitionRef,
    /// Condition of the replacement part (`Intact` or `Damaged`).
    pub condition: ComponentCondition,
    /// Initial/preserved values for component-scoped scalar cells (e.g. ammo or health).
    pub cell_values: BTreeMap<CellId, i64>,
    /// Optional economic asset object id for the replacement component.
    pub asset: Option<ObjId>,
}

impl ReplacementPartSpec {
    /// Build a [`ReplacementPartSpec`] from a [`StrippedComponentRecord`] (`FO-2`, `Q-S11`, `Q-S12`).
    #[must_use]
    pub fn from_stripped_record(record: &StrippedComponentRecord) -> Self {
        let cell_values = record
            .cells
            .iter()
            .map(|(k, v)| (k.clone(), v.current))
            .collect();
        Self {
            source_stripped_record_id: Some(record.record_id),
            definition: record.definition.clone(),
            condition: record.condition,
            cell_values,
            asset: record.asset,
        }
    }
}

/// Kind of workshop restoration work order executed on a hull (`FO-2`, `Q-S12`, `Q-S14`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkshopOperationKind {
    /// Repaired a `Damaged` structural section in place (`Q-S12`).
    RepairSection {
        /// Repaired section id.
        section: SectionId,
    },
    /// Reconstructed a `Destroyed` or `Wreck` structural section (`FO-2`, `Q-S12`).
    ReconstructSection {
        /// Reconstructed section id.
        section: SectionId,
    },
    /// Repaired an extant `Damaged` or `Jammed` component in place (`Q-S12`).
    RepairComponent {
        /// Repaired component id.
        component: ComponentId,
    },
    /// Replaced a `Destroyed` (`slagged`) or `"stripped"` component with a compatible
    /// replacement part (`FO-2`, `Q-S12`).
    ReplaceComponent {
        /// Replaced component slot/instance id.
        component: ComponentId,
        /// Optional salvaged component record consumed by this replacement.
        source_stripped_record_id: Option<u64>,
    },
    /// Rearmed or replenished a component-scoped consumable cell (such as weapon ammunition)
    /// without altering other sections or components (`Q-S12`, `Q-A4`).
    RearmComponent {
        /// Target component slot/instance id.
        component: ComponentId,
        /// Target consumable cell id (e.g. `"ammo"`).
        cell: CellId,
        /// Number of rounds/units added.
        rounds_added: i64,
    },
}

/// Deterministic tariff schedule for a bay or settlement workshop (`Q-S12`, `Q-A4`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkshopFacilityTariff {
    /// Resource cost per structural point repaired on a `Damaged` section.
    pub cost_per_structural_point_repair: u64,
    /// Work time in IC seconds per structural point repaired on a `Damaged` section.
    pub duration_ic_per_structural_point_repair: i64,
    /// Base resource cost to reconstruct a `Destroyed` or `Wreck` section shell.
    pub base_cost_reconstruct_section: u64,
    /// Work time in IC seconds to reconstruct a `Destroyed` or `Wreck` section shell.
    pub base_duration_ic_reconstruct_section: i64,
    /// Resource cost to repair an extant `Damaged` or `Jammed` component in place.
    pub cost_repair_component: u64,
    /// Work time in IC seconds to repair an extant `Damaged` or `Jammed` component.
    pub duration_ic_repair_component: i64,
    /// Labor/facility resource cost to install a replacement component into a slot.
    pub labor_cost_replace_component: u64,
    /// Work time in IC seconds to install a replacement component into a slot.
    pub duration_ic_replace_component: i64,
    /// Resource cost per round/unit of ammunition replenished.
    pub cost_per_ammo_round: u64,
    /// Work time in IC seconds per rearm operation.
    pub duration_ic_rearm_component: i64,
}

impl Default for WorkshopFacilityTariff {
    fn default() -> Self {
        Self {
            cost_per_structural_point_repair: 25,
            duration_ic_per_structural_point_repair: 10,
            base_cost_reconstruct_section: 1_500,
            base_duration_ic_reconstruct_section: 240,
            cost_repair_component: 400,
            duration_ic_repair_component: 90,
            labor_cost_replace_component: 600,
            duration_ic_replace_component: 120,
            cost_per_ammo_round: 10,
            duration_ic_rearm_component: 30,
        }
    }
}

/// Deterministic cost and duration quote to restore a unit's damaged/destroyed sections,
/// components, and depleted ammunition (`Q-S12`, `Q-A4`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkshopCostQuote {
    /// Target unit object id.
    pub unit: ObjId,
    /// Sections that can be repaired in place (`Damaged`).
    pub sections_to_repair: Vec<SectionId>,
    /// Sections that require structural shell reconstruction (`Destroyed` or `Wreck`).
    pub sections_to_reconstruct: Vec<SectionId>,
    /// Extant components that can be repaired in place (`Damaged` or `Jammed`).
    pub components_to_repair: Vec<ComponentId>,
    /// Component slots that are `Destroyed` or `"stripped"` and require a replacement part.
    pub components_to_replace: Vec<ComponentId>,
    /// Weapon components with depleted ammo and the missing round count for each.
    pub components_to_rearm: BTreeMap<ComponentId, i64>,
    /// Total estimated resource/credit cost (excluding replacement component purchase price).
    pub total_resource_cost: u64,
    /// Total estimated workshop duration in IC seconds.
    pub total_work_duration_ic: i64,
}

/// Completed workshop work-order record (`FO-2`, `Q-S12`, `Q-S14`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkshopWorkOrderRecord {
    /// Monotonic work-order id (`>= 1`).
    pub order_id: u64,
    /// Target unit object id.
    pub unit: ObjId,
    /// Actor executing the work order.
    pub actor: ObjId,
    /// Operation performed.
    pub kind: WorkshopOperationKind,
    /// Resource/credit cost expended.
    pub resource_cost: u64,
    /// Work time expended in IC seconds.
    pub work_duration_ic: i64,
    /// Whether this repair was performed at the custodian's own risk under a pending
    /// salvage claim (`Q-S14`).
    pub performed_at_risk_under_pending_claim: bool,
    /// Operational state before the work order.
    pub state_before: OperationalState,
    /// Operational state after the work order.
    pub state_after: OperationalState,
    /// IC timestamp when the work order completed.
    pub completed_at_ic: i64,
}

/// Ledger managing stripped components, hull scrapping, and workshop repair/replacement
/// work orders (`H3`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkshopBook {
    next_stripped_id: u64,
    next_order_id: u64,
    stripped_components: BTreeMap<u64, StrippedComponentRecord>,
    work_orders: BTreeMap<u64, WorkshopWorkOrderRecord>,
}

impl WorkshopBook {
    /// Construct an empty [`WorkshopBook`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_stripped_id: 1,
            next_order_id: 1,
            stripped_components: BTreeMap::new(),
            work_orders: BTreeMap::new(),
        }
    }

    /// Look up a [`StrippedComponentRecord`] by id.
    #[must_use]
    pub fn stripped_component(&self, record_id: u64) -> Option<&StrippedComponentRecord> {
        self.stripped_components.get(&record_id)
    }

    /// All recorded [`StrippedComponentRecord`]s.
    #[must_use]
    pub const fn stripped_components(&self) -> &BTreeMap<u64, StrippedComponentRecord> {
        &self.stripped_components
    }

    /// All recorded [`WorkshopWorkOrderRecord`]s.
    #[must_use]
    pub const fn work_orders(&self) -> &BTreeMap<u64, WorkshopWorkOrderRecord> {
        &self.work_orders
    }

    /// Strip a surviving (`Intact`, `Damaged`, or `Jammed`) component from `unit` (`Q-S11`, `Q-F5`, `Q-F6`).
    ///
    /// - Enforces `Q-F5` (refuses destructive stripping under pending salvage custody without
    ///   explicit owner permission) and `Q-F6` (requires lawful field-stripping authority in the field).
    /// - Refuses if the component is already `"stripped"` (`Q-F6`) or `Destroyed` (`Q-S11`).
    /// - Mints a [`StrippedComponentRecord`] preserving exact component identity, condition,
    ///   scalar cells (e.g. remaining ammo), asset pointer, and parent-hull provenance.
    /// - Mutates `unit` so the slot is marked `"stripped"`, its cells are zeroed to `min`,
    ///   its asset pointer is cleared, and (if `salvage_book` is supplied) records the
    ///   stripped component on `salvage_book` so the hull cannot double-claim it (`Q-F6`).
    ///
    /// # Errors
    /// Returns [`WorkshopError`] if authorization fails, the component is missing, already
    /// stripped, or destroyed, or unit mutation fails.
    #[allow(clippy::too_many_arguments)]
    pub fn strip_component(
        &mut self,
        unit: &TacticalUnit,
        anatomy: &UnitAnatomyProfile,
        component_id: &ComponentId,
        actor: ObjId,
        authorization: &StripAuthorization,
        salvage_book: Option<&mut SalvageClaimBook>,
        at_ic: i64,
    ) -> Result<(TacticalUnit, FieldUnitAssessment, StrippedComponentRecord), WorkshopError> {
        if actor.0 == 0 {
            return Err(WorkshopError::InvalidObjectId("actor"));
        }
        if self.stripped_components.len() >= MAX_STRIPPED_COMPONENTS {
            return Err(WorkshopError::MaxStrippedComponentsReached {
                max: MAX_STRIPPED_COMPONENTS,
            });
        }

        let (field_stripped, legality) = match authorization {
            StripAuthorization::BayOrWorkshop {
                custody,
                has_explicit_owner_permission,
            } => {
                check_custody_action_allowed(
                    custody,
                    CustodyAction::StripComponent,
                    *has_explicit_owner_permission,
                )
                .map_err(WorkshopError::Salvage)?;
                (false, ProvenanceLegalityStatus::CleanTitle)
            }
            StripAuthorization::Field { authority_ctx } => {
                let entitlement =
                    evaluate_recovery_entitlement(authority_ctx).map_err(WorkshopError::Salvage)?;
                if !entitlement.is_lawful() {
                    return Err(WorkshopError::UnlawfulFieldStrip {
                        hull: unit.object(),
                        component: component_id.clone(),
                    });
                }
                (true, ProvenanceLegalityStatus::CleanTitle)
            }
            StripAuthorization::UnlawfulFieldHotStrip { authority_ctx } => {
                let entitlement =
                    evaluate_recovery_entitlement(authority_ctx).map_err(WorkshopError::Salvage)?;
                let status = if entitlement.is_lawful() {
                    ProvenanceLegalityStatus::CleanTitle
                } else {
                    ProvenanceLegalityStatus::ContrabandHotPart {
                        rightful_owner: authority_ctx.unit_title_holder,
                        reason: ContrabandReason::UnlawfulFieldStripOnOwnedGround,
                    }
                };
                (true, status)
            }
        };

        let installed = unit
            .component(component_id)
            .ok_or_else(|| WorkshopError::UnknownComponent(component_id.clone()))?
            .clone();

        if installed.condition.as_str() == "stripped" {
            return Err(WorkshopError::ComponentAlreadyStripped {
                hull: unit.object(),
                component: component_id.clone(),
            });
        }

        let pre_assessment = assess_tactical_unit(unit, anatomy);
        let eff_cond = pre_assessment
            .component_conditions
            .get(component_id)
            .copied()
            .unwrap_or(ComponentCondition::Destroyed);

        if !eff_cond.is_salvageable() {
            return Err(WorkshopError::CannotStripDestroyedComponent {
                hull: unit.object(),
                component: component_id.clone(),
            });
        }

        let mut preserved_cells = BTreeMap::new();
        let mut ops = Vec::new();
        for (addr, cell) in unit.cells() {
            if let CellAddr::Component { component, cell: c } = addr
                && component == component_id
            {
                preserved_cells.insert(c.clone(), *cell);
                if cell.current != cell.min {
                    ops.push(UnitMutationOp::SetCell {
                        addr: addr.clone(),
                        value: cell.min,
                    });
                }
            }
        }

        if installed.asset.is_some() {
            ops.push(UnitMutationOp::SetComponentAsset {
                component: component_id.clone(),
                asset: None,
            });
        }

        ops.push(UnitMutationOp::SetComponentCondition {
            component: component_id.clone(),
            condition: ConditionId::new("stripped").map_err(WorkshopError::Unit)?,
        });

        let plan = UnitMutationPlan::new(
            ops,
            Some(unit.revision()),
            Some(SmolStr::new("field_ops.strip_component")),
        )
        .map_err(WorkshopError::Unit)?;
        let (updated_unit, _) = apply(unit, &plan, at_ic).map_err(WorkshopError::Unit)?;
        let post_assessment = assess_tactical_unit(&updated_unit, anatomy);

        let record_id = self.next_stripped_id;
        self.next_stripped_id = self
            .next_stripped_id
            .checked_add(1)
            .ok_or(WorkshopError::IdOverflow)?;

        let host_section = install_point_section(&installed.install).cloned();
        let is_lawful_strip = matches!(legality, ProvenanceLegalityStatus::CleanTitle);
        let record = StrippedComponentRecord {
            record_id,
            component_id: component_id.clone(),
            definition: installed.definition,
            install: installed.install,
            condition: eff_cond,
            cells: preserved_cells,
            asset: installed.asset,
            provenance: ComponentProvenance {
                parent_hull: unit.object(),
                parent_schema: unit.schema().clone(),
                host_section,
                stripped_by: actor,
                stripped_at_ic: at_ic,
                field_stripped,
                legality,
            },
            consumed_into_hull: None,
        };
        self.stripped_components.insert(record_id, record.clone());

        if let Some(book) = salvage_book {
            book.mark_component_stripped(unit.object(), component_id.clone());
            if field_stripped {
                let _ = book.append_black_box_entry(
                    unit.object(),
                    BlackBoxFlightEntry::FieldComponentStripped {
                        component: component_id.clone(),
                        stripped_by: actor,
                        lawful: is_lawful_strip,
                        at_ic,
                    },
                );
            }
        }

        Ok((updated_unit, post_assessment, record))
    }

    /// Flag a detached component record as [`ProvenanceLegalityStatus::ContrabandHotPart`]
    /// when a salvage claim covering the parent hull or component is contested and vacated
    /// (`Q-S13`, `Q-F5`, `Q-F6`).
    ///
    /// # Errors
    /// Returns [`WorkshopError`] if `record_id` is unknown or `rightful_owner.0 == 0`.
    pub fn mark_stripped_component_contraband_on_vacated_claim(
        &mut self,
        record_id: u64,
        claim_id: SalvageClaimId,
        rightful_owner: ObjId,
    ) -> Result<&StrippedComponentRecord, WorkshopError> {
        if rightful_owner.0 == 0 {
            return Err(WorkshopError::InvalidObjectId("rightful_owner"));
        }
        let rec = self
            .stripped_components
            .get_mut(&record_id)
            .ok_or(WorkshopError::UnknownStrippedComponentRecord(record_id))?;
        rec.provenance.legality = ProvenanceLegalityStatus::ContrabandHotPart {
            rightful_owner,
            reason: ContrabandReason::VacatedSalvageClaim { claim_id },
        };
        Ok(rec)
    }

    /// Clear a [`ProvenanceLegalityStatus::ContrabandHotPart`] flag back to
    /// [`ProvenanceLegalityStatus::CleanTitle`] when authorized by `rightful_owner` (`FO-4`, `Q-S4`).
    ///
    /// # Errors
    /// Returns [`WorkshopError`] if `record_id` is unknown or `authorizing_owner != rightful_owner`.
    pub fn clear_contraband_provenance(
        &mut self,
        record_id: u64,
        authorizing_owner: ObjId,
    ) -> Result<&StrippedComponentRecord, WorkshopError> {
        if authorizing_owner.0 == 0 {
            return Err(WorkshopError::InvalidObjectId("authorizing_owner"));
        }
        let rec = self
            .stripped_components
            .get_mut(&record_id)
            .ok_or(WorkshopError::UnknownStrippedComponentRecord(record_id))?;
        if let ProvenanceLegalityStatus::ContrabandHotPart { rightful_owner, .. } =
            &rec.provenance.legality
            && *rightful_owner != authorizing_owner
        {
            return Err(WorkshopError::UnauthorizedContrabandClearance {
                record_id,
                rightful_owner: *rightful_owner,
                actor: authorizing_owner,
            });
        }
        rec.provenance.legality = ProvenanceLegalityStatus::CleanTitle;
        Ok(rec)
    }

    /// Destructively scrap a hull into structural and component scrap yields (`Q-S11`, `Q-F5`).
    ///
    /// Refused while salvage title is pending (`CustodyTitleStatus::PendingSalvageClaim`)
    /// unless `has_explicit_owner_permission` is true (`Q-F5`). Transitions the hull to
    /// [`OperationalState::Irrecoverable`].
    ///
    /// # Errors
    /// Returns [`WorkshopError`] if authorization fails, the hull is already `Irrecoverable`,
    /// or unit mutation fails.
    pub fn scrap_hull(
        &mut self,
        unit: &TacticalUnit,
        anatomy: &UnitAnatomyProfile,
        actor: ObjId,
        custody: &CustodyTitleStatus,
        has_explicit_owner_permission: bool,
        at_ic: i64,
    ) -> Result<(TacticalUnit, ScrapYieldReport), WorkshopError> {
        if actor.0 == 0 {
            return Err(WorkshopError::InvalidObjectId("actor"));
        }
        check_custody_action_allowed(
            custody,
            CustodyAction::ScrapHull,
            has_explicit_owner_permission,
        )
        .map_err(WorkshopError::Salvage)?;

        let pre_assessment = assess_tactical_unit(unit, anatomy);
        if !pre_assessment.state.retains_salvageable_hull() {
            return Err(WorkshopError::HullAlreadyIrrecoverable(unit.object()));
        }

        let mut structural_points: u64 = 0;
        let mut ops = Vec::new();
        for (addr, cell) in unit.cells() {
            if matches!(addr, CellAddr::Section { .. } | CellAddr::Facet { .. })
                && cell.current > cell.min
            {
                let remaining = u64::try_from(cell.current.saturating_sub(cell.min)).unwrap_or(0);
                structural_points = structural_points.saturating_add(remaining);
            }
            if cell.current != cell.min {
                ops.push(UnitMutationOp::SetCell {
                    addr: addr.clone(),
                    value: cell.min,
                });
            }
        }

        let structural_scrap_kg = if structural_points > 0 {
            structural_points.saturating_mul(SCRAP_KG_PER_STRUCTURAL_POINT)
        } else {
            let surviving_sections = pre_assessment
                .section_conditions
                .values()
                .filter(|c| {
                    !matches!(
                        c,
                        SectionCondition::Destroyed | SectionCondition::Irrecoverable
                    )
                })
                .count() as u64;
            surviving_sections
                .saturating_mul(FALLBACK_SECTION_SCRAP_KG)
                .max(FALLBACK_SECTION_SCRAP_KG)
        };

        let dismantled_salvageable_components = pre_assessment.surviving_components().len();

        let irrecoverable_cond = SectionCondition::Irrecoverable
            .to_condition_id()
            .map_err(WorkshopError::Unit)?;
        for (sec_id, sec_state) in unit.sections() {
            if sec_state.condition != irrecoverable_cond {
                ops.push(UnitMutationOp::SetSectionCondition {
                    section: sec_id.clone(),
                    condition: irrecoverable_cond.clone(),
                });
            }
        }

        let destroyed_comp_cond = ComponentCondition::Destroyed
            .to_condition_id()
            .map_err(WorkshopError::Unit)?;
        for (comp_id, comp_state) in unit.components() {
            if comp_state.asset.is_some() {
                ops.push(UnitMutationOp::SetComponentAsset {
                    component: comp_id.clone(),
                    asset: None,
                });
            }
            if comp_state.condition != destroyed_comp_cond
                && comp_state.condition.as_str() != "stripped"
            {
                ops.push(UnitMutationOp::SetComponentCondition {
                    component: comp_id.clone(),
                    condition: destroyed_comp_cond.clone(),
                });
            }
        }

        let plan = UnitMutationPlan::new(
            ops,
            Some(unit.revision()),
            Some(SmolStr::new("field_ops.scrap_hull")),
        )
        .map_err(WorkshopError::Unit)?;
        let (scrapped_unit, _) = apply(unit, &plan, at_ic).map_err(WorkshopError::Unit)?;
        let final_assessment = assess_tactical_unit(&scrapped_unit, anatomy);

        Ok((
            scrapped_unit,
            ScrapYieldReport {
                hull: unit.object(),
                scrapped_by: actor,
                structural_scrap_kg,
                dismantled_salvageable_components,
                final_assessment,
                scrapped_at_ic: at_ic,
            },
        ))
    }

    /// Repair a `Damaged` structural section in place without resetting untouched sections,
    /// components, or ammo (`Q-S12`, `Q-S14`).
    ///
    /// Note: a `Destroyed` or `Wreck` section requires [`Self::reconstruct_section`].
    ///
    /// # Errors
    /// Returns [`WorkshopError`] if the section is unknown, `Destroyed`/`Wreck`/`Irrecoverable`,
    /// or work parameters are invalid.
    #[allow(clippy::too_many_arguments)]
    pub fn repair_section(
        &mut self,
        unit: &TacticalUnit,
        anatomy: &UnitAnatomyProfile,
        section_id: &SectionId,
        actor: ObjId,
        custody: &CustodyTitleStatus,
        has_explicit_owner_permission: bool,
        resource_cost: u64,
        work_duration_ic: i64,
        salvage_book: Option<&mut SalvageClaimBook>,
        at_ic: i64,
    ) -> Result<(TacticalUnit, FieldUnitAssessment, WorkshopWorkOrderRecord), WorkshopError> {
        self.validate_work_inputs(
            actor,
            custody,
            has_explicit_owner_permission,
            work_duration_ic,
        )?;
        let _ = unit
            .section(section_id)
            .ok_or_else(|| WorkshopError::UnknownSection(section_id.clone()))?;

        let pre_assessment = assess_tactical_unit(unit, anatomy);
        let eff_sec = effective_section_condition(unit, section_id, anatomy)
            .unwrap_or(SectionCondition::Destroyed);

        if matches!(
            eff_sec,
            SectionCondition::Destroyed | SectionCondition::Wreck | SectionCondition::Irrecoverable
        ) {
            return Err(WorkshopError::SectionRequiresReconstruction {
                hull: unit.object(),
                section: section_id.clone(),
                condition: eff_sec,
            });
        }

        let mut ops = Vec::new();
        for (addr, cell) in unit.cells() {
            let belongs_to_sec = match addr {
                CellAddr::Section { section, .. } | CellAddr::Facet { section, .. } => {
                    section == section_id
                }
                _ => false,
            };
            if belongs_to_sec && cell.current < cell.max {
                ops.push(UnitMutationOp::SetCell {
                    addr: addr.clone(),
                    value: cell.max,
                });
            }
        }

        let intact_cond = SectionCondition::Intact
            .to_condition_id()
            .map_err(WorkshopError::Unit)?;
        if unit
            .section(section_id)
            .is_some_and(|s| s.condition != intact_cond)
        {
            ops.push(UnitMutationOp::SetSectionCondition {
                section: section_id.clone(),
                condition: intact_cond,
            });
        }

        let updated_unit = if ops.is_empty() {
            unit.clone()
        } else {
            let plan = UnitMutationPlan::new(
                ops,
                Some(unit.revision()),
                Some(SmolStr::new("field_ops.repair_section")),
            )
            .map_err(WorkshopError::Unit)?;
            apply(unit, &plan, at_ic).map_err(WorkshopError::Unit)?.0
        };
        let post_assessment = assess_tactical_unit(&updated_unit, anatomy);

        let record = self.record_work_order(
            unit.object(),
            actor,
            WorkshopOperationKind::RepairSection {
                section: section_id.clone(),
            },
            custody,
            resource_cost,
            work_duration_ic,
            pre_assessment.state,
            post_assessment.state,
            salvage_book,
            at_ic,
        )?;

        Ok((updated_unit, post_assessment, record))
    }

    /// Reconstruct a `Destroyed` or `Wreck` structural section (`FO-2`, `Q-S12`, `Q-S14`).
    ///
    /// Restores the section's structural and facet cells to `max` and sets the section
    /// condition to `Intact`. Components that were hosted in the destroyed section were
    /// slagged when the section was destroyed and remain explicitly `"destroyed"` (or
    /// `"stripped"`) until replaced via [`Self::replace_component_in_slot`] (`FO-2`, `Q-S12`).
    ///
    /// # Errors
    /// Returns [`WorkshopError`] if the section is unknown, the hull is `Irrecoverable`,
    /// or work parameters are invalid.
    #[allow(clippy::too_many_arguments)]
    pub fn reconstruct_section(
        &mut self,
        unit: &TacticalUnit,
        anatomy: &UnitAnatomyProfile,
        section_id: &SectionId,
        actor: ObjId,
        custody: &CustodyTitleStatus,
        has_explicit_owner_permission: bool,
        resource_cost: u64,
        work_duration_ic: i64,
        salvage_book: Option<&mut SalvageClaimBook>,
        at_ic: i64,
    ) -> Result<(TacticalUnit, FieldUnitAssessment, WorkshopWorkOrderRecord), WorkshopError> {
        self.validate_work_inputs(
            actor,
            custody,
            has_explicit_owner_permission,
            work_duration_ic,
        )?;
        let _ = unit
            .section(section_id)
            .ok_or_else(|| WorkshopError::UnknownSection(section_id.clone()))?;

        let pre_assessment = assess_tactical_unit(unit, anatomy);
        if !pre_assessment.state.retains_salvageable_hull() {
            return Err(WorkshopError::HullAlreadyIrrecoverable(unit.object()));
        }

        let eff_sec = effective_section_condition(unit, section_id, anatomy)
            .unwrap_or(SectionCondition::Destroyed);
        let section_was_destroying = eff_sec.destroys_hosted_components();

        let mut ops = Vec::new();
        let destroyed_comp_cond = ComponentCondition::Destroyed
            .to_condition_id()
            .map_err(WorkshopError::Unit)?;

        // Any component hosted in a destroyed section was slagged when the section was lost;
        // persist its `destroyed` condition explicitly before restoring the section structure
        // so reconstructing the section shell does not silently resurrect slagged components (`FO-2`, `Q-S12`).
        if section_was_destroying {
            for (comp_id, comp_state) in unit.components() {
                if install_point_section(&comp_state.install) == Some(section_id)
                    && comp_state.condition.as_str() != "stripped"
                    && comp_state.condition != destroyed_comp_cond
                {
                    ops.push(UnitMutationOp::SetComponentCondition {
                        component: comp_id.clone(),
                        condition: destroyed_comp_cond.clone(),
                    });
                }
            }
        }

        for (addr, cell) in unit.cells() {
            let belongs_to_sec = match addr {
                CellAddr::Section { section, .. } | CellAddr::Facet { section, .. } => {
                    section == section_id
                }
                _ => false,
            };
            if belongs_to_sec && cell.current < cell.max {
                ops.push(UnitMutationOp::SetCell {
                    addr: addr.clone(),
                    value: cell.max,
                });
            }
        }

        let intact_cond = SectionCondition::Intact
            .to_condition_id()
            .map_err(WorkshopError::Unit)?;
        if unit
            .section(section_id)
            .is_some_and(|s| s.condition != intact_cond)
        {
            ops.push(UnitMutationOp::SetSectionCondition {
                section: section_id.clone(),
                condition: intact_cond,
            });
        }

        let updated_unit = if ops.is_empty() {
            unit.clone()
        } else {
            let plan = UnitMutationPlan::new(
                ops,
                Some(unit.revision()),
                Some(SmolStr::new("field_ops.reconstruct_section")),
            )
            .map_err(WorkshopError::Unit)?;
            apply(unit, &plan, at_ic).map_err(WorkshopError::Unit)?.0
        };
        let post_assessment = assess_tactical_unit(&updated_unit, anatomy);

        let record = self.record_work_order(
            unit.object(),
            actor,
            WorkshopOperationKind::ReconstructSection {
                section: section_id.clone(),
            },
            custody,
            resource_cost,
            work_duration_ic,
            pre_assessment.state,
            post_assessment.state,
            salvage_book,
            at_ic,
        )?;

        Ok((updated_unit, post_assessment, record))
    }

    /// Repair an extant `Damaged` or `Jammed` component in place (`Q-S12`, `Q-S14`).
    ///
    /// Refuses with [`WorkshopError::ComponentRequiresReplacement`] if the component is
    /// `Destroyed` (`slagged`) or `"stripped"` (`FO-2`, `Q-S12`).
    ///
    /// # Errors
    /// Returns [`WorkshopError`] if the component is unknown, its host section is destroyed,
    /// or the component is `Destroyed` / `"stripped"`.
    #[allow(clippy::too_many_arguments)]
    pub fn repair_component(
        &mut self,
        unit: &TacticalUnit,
        anatomy: &UnitAnatomyProfile,
        component_id: &ComponentId,
        actor: ObjId,
        custody: &CustodyTitleStatus,
        has_explicit_owner_permission: bool,
        resource_cost: u64,
        work_duration_ic: i64,
        salvage_book: Option<&mut SalvageClaimBook>,
        at_ic: i64,
    ) -> Result<(TacticalUnit, FieldUnitAssessment, WorkshopWorkOrderRecord), WorkshopError> {
        self.validate_work_inputs(
            actor,
            custody,
            has_explicit_owner_permission,
            work_duration_ic,
        )?;
        let installed = unit
            .component(component_id)
            .ok_or_else(|| WorkshopError::UnknownComponent(component_id.clone()))?;

        let pre_assessment = assess_tactical_unit(unit, anatomy);
        if let Some(host_sec) = install_point_section(&installed.install)
            && let Some(sec_cond) = pre_assessment.section_conditions.get(host_sec)
            && !sec_cond.is_functional()
        {
            return Err(WorkshopError::HostSectionNotFunctional {
                hull: unit.object(),
                section: host_sec.clone(),
                condition: *sec_cond,
            });
        }

        let eff_comp = effective_component_condition(
            unit,
            component_id,
            &pre_assessment.section_conditions,
            anatomy,
        )
        .unwrap_or(ComponentCondition::Destroyed);

        if !eff_comp.is_salvageable() || installed.condition.as_str() == "stripped" {
            return Err(WorkshopError::ComponentRequiresReplacement {
                hull: unit.object(),
                component: component_id.clone(),
            });
        }

        let mut ops = Vec::new();
        if let Some(health_cell) = &anatomy.component_health_cell {
            let addr = CellAddr::Component {
                component: component_id.clone(),
                cell: health_cell.clone(),
            };
            if let Some(cell) = unit.cell(&addr)
                && cell.current < cell.max
            {
                ops.push(UnitMutationOp::SetCell {
                    addr,
                    value: cell.max,
                });
            }
        }

        let intact_cond = ComponentCondition::Intact
            .to_condition_id()
            .map_err(WorkshopError::Unit)?;
        if installed.condition != intact_cond {
            ops.push(UnitMutationOp::SetComponentCondition {
                component: component_id.clone(),
                condition: intact_cond,
            });
        }

        let updated_unit = if ops.is_empty() {
            unit.clone()
        } else {
            let plan = UnitMutationPlan::new(
                ops,
                Some(unit.revision()),
                Some(SmolStr::new("field_ops.repair_component")),
            )
            .map_err(WorkshopError::Unit)?;
            apply(unit, &plan, at_ic).map_err(WorkshopError::Unit)?.0
        };
        let post_assessment = assess_tactical_unit(&updated_unit, anatomy);

        let record = self.record_work_order(
            unit.object(),
            actor,
            WorkshopOperationKind::RepairComponent {
                component: component_id.clone(),
            },
            custody,
            resource_cost,
            work_duration_ic,
            pre_assessment.state,
            post_assessment.state,
            salvage_book,
            at_ic,
        )?;

        Ok((updated_unit, post_assessment, record))
    }

    /// Replace a `Destroyed` (`slagged`) or `"stripped"` component on `unit` with a
    /// compatible [`ReplacementPartSpec`] (`FO-2`, `Q-S12`, `Q-S14`).
    ///
    /// - Requires the host section (if any) to be functional (`Intact` or `Damaged`); if
    ///   the host section was `Destroyed`, call [`Self::reconstruct_section`] first.
    /// - Validates that `replacement.definition` matches the slot's expected definition
    ///   and that `replacement.condition` is salvageable (`Intact`, `Damaged`, or `Jammed`).
    /// - If `replacement.source_stripped_record_id` names a [`StrippedComponentRecord`] in
    ///   this book, verifies it has not already been consumed and marks it consumed into `unit`.
    /// - Recomputes [`FieldUnitAssessment`] (e.g., restoring a headshot `Disabled` biped
    ///   back to `Operational` while preserving untouched sections and ammo, `Q-S12`).
    ///
    /// # Errors
    /// Returns [`WorkshopError`] if the slot is not `Destroyed`/`"stripped"`, the host
    /// section is not functional, the replacement definition does not match, or the
    /// stripped part was already consumed.
    #[allow(clippy::too_many_arguments)]
    pub fn replace_component_in_slot(
        &mut self,
        unit: &TacticalUnit,
        anatomy: &UnitAnatomyProfile,
        component_id: &ComponentId,
        replacement: &ReplacementPartSpec,
        actor: ObjId,
        custody: &CustodyTitleStatus,
        has_explicit_owner_permission: bool,
        resource_cost: u64,
        work_duration_ic: i64,
        salvage_book: Option<&mut SalvageClaimBook>,
        at_ic: i64,
    ) -> Result<(TacticalUnit, FieldUnitAssessment, WorkshopWorkOrderRecord), WorkshopError> {
        self.validate_work_inputs(
            actor,
            custody,
            has_explicit_owner_permission,
            work_duration_ic,
        )?;

        if !replacement.condition.is_salvageable() {
            return Err(WorkshopError::InvalidReplacementPartCondition(
                replacement.condition,
            ));
        }

        if let Some(rec_id) = replacement.source_stripped_record_id {
            let rec = self
                .stripped_components
                .get(&rec_id)
                .ok_or(WorkshopError::UnknownStrippedComponentRecord(rec_id))?;
            if let Some(other_hull) = rec.consumed_into_hull {
                return Err(WorkshopError::StrippedComponentAlreadyConsumed {
                    record_id: rec_id,
                    consumed_into_hull: other_hull,
                });
            }
            if let ProvenanceLegalityStatus::ContrabandHotPart {
                rightful_owner,
                reason,
            } = &rec.provenance.legality
            {
                return Err(WorkshopError::ContrabandComponentRefused {
                    record_id: rec_id,
                    parent_hull: rec.provenance.parent_hull,
                    rightful_owner: *rightful_owner,
                    reason: reason.clone(),
                });
            }
        }

        let installed = unit
            .component(component_id)
            .ok_or_else(|| WorkshopError::UnknownComponent(component_id.clone()))?
            .clone();

        if replacement.definition != installed.definition {
            return Err(WorkshopError::IncompatibleReplacementDefinition {
                component: component_id.clone(),
                expected: Box::new(installed.definition),
                actual: Box::new(replacement.definition.clone()),
            });
        }

        let pre_assessment = assess_tactical_unit(unit, anatomy);
        if let Some(host_sec) = install_point_section(&installed.install)
            && let Some(sec_cond) = pre_assessment.section_conditions.get(host_sec)
            && !sec_cond.is_functional()
        {
            return Err(WorkshopError::HostSectionNotFunctional {
                hull: unit.object(),
                section: host_sec.clone(),
                condition: *sec_cond,
            });
        }

        let eff_comp = effective_component_condition(
            unit,
            component_id,
            &pre_assessment.section_conditions,
            anatomy,
        )
        .unwrap_or(ComponentCondition::Destroyed);

        if eff_comp != ComponentCondition::Destroyed && installed.condition.as_str() != "stripped" {
            return Err(WorkshopError::SlotAlreadyHasIntactOrRepairableComponent {
                hull: unit.object(),
                component: component_id.clone(),
                condition: eff_comp,
            });
        }

        let mut ops = Vec::new();
        for (addr, cell) in unit.cells() {
            if let CellAddr::Component { component, cell: c } = addr
                && component == component_id
            {
                let target_val = replacement
                    .cell_values
                    .get(c)
                    .copied()
                    .unwrap_or(cell.max)
                    .clamp(cell.min, cell.max);
                if cell.current != target_val {
                    ops.push(UnitMutationOp::SetCell {
                        addr: addr.clone(),
                        value: target_val,
                    });
                }
            }
        }

        if installed.asset != replacement.asset {
            ops.push(UnitMutationOp::SetComponentAsset {
                component: component_id.clone(),
                asset: replacement.asset,
            });
        }

        let next_cond = replacement
            .condition
            .to_condition_id()
            .map_err(WorkshopError::Unit)?;
        if installed.condition != next_cond {
            ops.push(UnitMutationOp::SetComponentCondition {
                component: component_id.clone(),
                condition: next_cond,
            });
        }

        let plan = UnitMutationPlan::new(
            ops,
            Some(unit.revision()),
            Some(SmolStr::new("field_ops.replace_component")),
        )
        .map_err(WorkshopError::Unit)?;
        let (updated_unit, _) = apply(unit, &plan, at_ic).map_err(WorkshopError::Unit)?;
        let post_assessment = assess_tactical_unit(&updated_unit, anatomy);

        if let Some(rec_id) = replacement.source_stripped_record_id
            && let Some(rec) = self.stripped_components.get_mut(&rec_id)
        {
            rec.consumed_into_hull = Some(unit.object());
        }

        let record = self.record_work_order(
            unit.object(),
            actor,
            WorkshopOperationKind::ReplaceComponent {
                component: component_id.clone(),
                source_stripped_record_id: replacement.source_stripped_record_id,
            },
            custody,
            resource_cost,
            work_duration_ic,
            pre_assessment.state,
            post_assessment.state,
            salvage_book,
            at_ic,
        )?;

        Ok((updated_unit, post_assessment, record))
    }

    /// Rearm or replenish a component-scoped consumable cell (such as weapon ammunition) on a
    /// surviving component without touching other sections or components (`Q-S12`, `Q-A4`).
    ///
    /// Restoring ammunition on an `Unarmed` unit whose weapon component is otherwise functional
    /// transitions its [`FieldUnitAssessment`] back to [`OperationalState::Operational`] (or
    /// `Impaired` / `Immobilized` / `Disabled` according to its structural state).
    ///
    /// # Errors
    /// Returns [`WorkshopError`] if the component or cell is unknown, the host section is not
    /// functional, the component is `Destroyed` or `"stripped"`, or `rounds_to_add <= 0`.
    #[allow(clippy::too_many_arguments)]
    pub fn rearm_component(
        &mut self,
        unit: &TacticalUnit,
        anatomy: &UnitAnatomyProfile,
        component_id: &ComponentId,
        ammo_cell_id: &CellId,
        rounds_to_add: i64,
        actor: ObjId,
        custody: &CustodyTitleStatus,
        has_explicit_owner_permission: bool,
        resource_cost: u64,
        work_duration_ic: i64,
        salvage_book: Option<&mut SalvageClaimBook>,
        at_ic: i64,
    ) -> Result<(TacticalUnit, FieldUnitAssessment, WorkshopWorkOrderRecord), WorkshopError> {
        self.validate_work_inputs(
            actor,
            custody,
            has_explicit_owner_permission,
            work_duration_ic,
        )?;
        if rounds_to_add <= 0 {
            return Err(WorkshopError::InvalidRearmAmount(rounds_to_add));
        }

        let installed = unit
            .component(component_id)
            .ok_or_else(|| WorkshopError::UnknownComponent(component_id.clone()))?;

        let pre_assessment = assess_tactical_unit(unit, anatomy);
        if let Some(host_sec) = install_point_section(&installed.install)
            && let Some(sec_cond) = pre_assessment.section_conditions.get(host_sec)
            && !sec_cond.is_functional()
        {
            return Err(WorkshopError::HostSectionNotFunctional {
                hull: unit.object(),
                section: host_sec.clone(),
                condition: *sec_cond,
            });
        }

        let eff_comp = effective_component_condition(
            unit,
            component_id,
            &pre_assessment.section_conditions,
            anatomy,
        )
        .unwrap_or(ComponentCondition::Destroyed);

        if !eff_comp.is_salvageable() || installed.condition.as_str() == "stripped" {
            return Err(WorkshopError::ComponentRequiresReplacement {
                hull: unit.object(),
                component: component_id.clone(),
            });
        }

        let addr = CellAddr::Component {
            component: component_id.clone(),
            cell: ammo_cell_id.clone(),
        };
        let cell = unit
            .cell(&addr)
            .ok_or_else(|| WorkshopError::UnknownComponentCell {
                component: component_id.clone(),
                cell: ammo_cell_id.clone(),
            })?;

        let target_val = cell
            .current
            .saturating_add(rounds_to_add)
            .clamp(cell.min, cell.max);
        let actual_added = target_val.saturating_sub(cell.current);
        let updated_unit = if actual_added == 0 {
            unit.clone()
        } else {
            let plan = UnitMutationPlan::new(
                vec![UnitMutationOp::SetCell {
                    addr,
                    value: target_val,
                }],
                Some(unit.revision()),
                Some(SmolStr::new("field_ops.rearm_component")),
            )
            .map_err(WorkshopError::Unit)?;
            apply(unit, &plan, at_ic).map_err(WorkshopError::Unit)?.0
        };
        let post_assessment = assess_tactical_unit(&updated_unit, anatomy);

        let record = self.record_work_order(
            unit.object(),
            actor,
            WorkshopOperationKind::RearmComponent {
                component: component_id.clone(),
                cell: ammo_cell_id.clone(),
                rounds_added: actual_added,
            },
            custody,
            resource_cost,
            work_duration_ic,
            pre_assessment.state,
            post_assessment.state,
            salvage_book,
            at_ic,
        )?;

        Ok((updated_unit, post_assessment, record))
    }

    /// Compute a deterministic [`WorkshopCostQuote`] for restoring `unit`'s damaged/destroyed
    /// sections, components, and depleted weapon ammunition under `tariff` (`Q-S12`, `Q-A4`).
    ///
    /// # Errors
    /// Returns [`WorkshopError::HullAlreadyIrrecoverable`] if the hull is `Irrecoverable`.
    pub fn quote_full_restoration(
        &self,
        unit: &TacticalUnit,
        anatomy: &UnitAnatomyProfile,
        tariff: &WorkshopFacilityTariff,
    ) -> Result<WorkshopCostQuote, WorkshopError> {
        let assessment = assess_tactical_unit(unit, anatomy);
        if !assessment.state.retains_salvageable_hull() {
            return Err(WorkshopError::HullAlreadyIrrecoverable(unit.object()));
        }

        let mut sections_to_repair = Vec::new();
        let mut sections_to_reconstruct = Vec::new();
        let mut total_resource_cost: u64 = 0;
        let mut total_work_duration_ic: i64 = 0;

        for (sec_id, cond) in &assessment.section_conditions {
            match cond {
                SectionCondition::Intact => {
                    let missing_pts = missing_section_points(unit, sec_id);
                    if missing_pts > 0 {
                        sections_to_repair.push(sec_id.clone());
                        total_resource_cost = total_resource_cost.saturating_add(
                            missing_pts.saturating_mul(tariff.cost_per_structural_point_repair),
                        );
                        total_work_duration_ic = total_work_duration_ic.saturating_add(
                            i64::try_from(missing_pts)
                                .unwrap_or(i64::MAX)
                                .saturating_mul(tariff.duration_ic_per_structural_point_repair)
                                .max(1),
                        );
                    }
                }
                SectionCondition::Damaged => {
                    sections_to_repair.push(sec_id.clone());
                    let missing_pts = missing_section_points(unit, sec_id).max(1);
                    total_resource_cost = total_resource_cost.saturating_add(
                        missing_pts.saturating_mul(tariff.cost_per_structural_point_repair),
                    );
                    total_work_duration_ic = total_work_duration_ic.saturating_add(
                        i64::try_from(missing_pts)
                            .unwrap_or(i64::MAX)
                            .saturating_mul(tariff.duration_ic_per_structural_point_repair)
                            .max(1),
                    );
                }
                SectionCondition::Destroyed | SectionCondition::Wreck => {
                    sections_to_reconstruct.push(sec_id.clone());
                    let missing_pts = missing_section_points(unit, sec_id);
                    let pt_cost =
                        missing_pts.saturating_mul(tariff.cost_per_structural_point_repair);
                    total_resource_cost = total_resource_cost.saturating_add(
                        tariff.base_cost_reconstruct_section.saturating_add(pt_cost),
                    );
                    total_work_duration_ic = total_work_duration_ic
                        .saturating_add(tariff.base_duration_ic_reconstruct_section.max(1));
                }
                SectionCondition::Irrecoverable => {}
            }
        }

        let mut components_to_repair = Vec::new();
        let mut components_to_replace = Vec::new();
        let mut components_to_rearm = BTreeMap::new();

        for (comp_id, cond) in &assessment.component_conditions {
            let is_stripped = unit
                .component(comp_id)
                .is_some_and(|c| c.condition.as_str() == "stripped");
            if is_stripped || *cond == ComponentCondition::Destroyed {
                components_to_replace.push(comp_id.clone());
                total_resource_cost =
                    total_resource_cost.saturating_add(tariff.labor_cost_replace_component);
                total_work_duration_ic = total_work_duration_ic
                    .saturating_add(tariff.duration_ic_replace_component.max(1));
            } else if matches!(
                cond,
                ComponentCondition::Damaged | ComponentCondition::Jammed
            ) {
                components_to_repair.push(comp_id.clone());
                total_resource_cost =
                    total_resource_cost.saturating_add(tariff.cost_repair_component);
                total_work_duration_ic = total_work_duration_ic
                    .saturating_add(tariff.duration_ic_repair_component.max(1));
            }
        }

        if let Some(ammo_cell) = &anatomy.weapon_ammo_cell {
            for weapon_id in &anatomy.weapon_components {
                if components_to_replace.contains(weapon_id) {
                    continue;
                }
                let addr = CellAddr::Component {
                    component: weapon_id.clone(),
                    cell: ammo_cell.clone(),
                };
                if let Some(cell) = unit.cell(&addr)
                    && cell.current < cell.max
                {
                    let missing_rounds = cell.max.saturating_sub(cell.current);
                    components_to_rearm.insert(weapon_id.clone(), missing_rounds);
                    let rounds_u64 = u64::try_from(missing_rounds).unwrap_or(0);
                    total_resource_cost = total_resource_cost
                        .saturating_add(rounds_u64.saturating_mul(tariff.cost_per_ammo_round));
                    total_work_duration_ic = total_work_duration_ic
                        .saturating_add(tariff.duration_ic_rearm_component.max(1));
                }
            }
        }

        Ok(WorkshopCostQuote {
            unit: unit.object(),
            sections_to_repair,
            sections_to_reconstruct,
            components_to_repair,
            components_to_replace,
            components_to_rearm,
            total_resource_cost,
            total_work_duration_ic,
        })
    }

    fn validate_work_inputs(
        &self,
        actor: ObjId,
        custody: &CustodyTitleStatus,
        has_explicit_owner_permission: bool,
        work_duration_ic: i64,
    ) -> Result<(), WorkshopError> {
        if actor.0 == 0 {
            return Err(WorkshopError::InvalidObjectId("actor"));
        }
        if work_duration_ic <= 0 {
            return Err(WorkshopError::InvalidWorkDuration(work_duration_ic));
        }
        if self.work_orders.len() >= MAX_WORKSHOP_ORDERS {
            return Err(WorkshopError::MaxWorkOrdersReached {
                max: MAX_WORKSHOP_ORDERS,
            });
        }
        check_custody_action_allowed(
            custody,
            CustodyAction::DocumentedRepairAtRisk,
            has_explicit_owner_permission,
        )
        .map_err(WorkshopError::Salvage)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn record_work_order(
        &mut self,
        unit: ObjId,
        actor: ObjId,
        kind: WorkshopOperationKind,
        custody: &CustodyTitleStatus,
        resource_cost: u64,
        work_duration_ic: i64,
        state_before: OperationalState,
        state_after: OperationalState,
        salvage_book: Option<&mut SalvageClaimBook>,
        completed_at_ic: i64,
    ) -> Result<WorkshopWorkOrderRecord, WorkshopError> {
        let order_id = self.next_order_id;
        self.next_order_id = self
            .next_order_id
            .checked_add(1)
            .ok_or(WorkshopError::IdOverflow)?;

        let performed_at_risk_under_pending_claim =
            if let CustodyTitleStatus::PendingSalvageClaim { claim_id, .. } = custody {
                if let Some(book) = salvage_book {
                    let desc = SmolStr::new(format!("workshop_order_{order_id}"));
                    book.record_repair_at_risk(*claim_id, desc, resource_cost, completed_at_ic)
                        .map_err(WorkshopError::Salvage)?;
                }
                true
            } else {
                false
            };

        let record = WorkshopWorkOrderRecord {
            order_id,
            unit,
            actor,
            kind,
            resource_cost,
            work_duration_ic,
            performed_at_risk_under_pending_claim,
            state_before,
            state_after,
            completed_at_ic,
        };
        self.work_orders.insert(order_id, record.clone());
        Ok(record)
    }
}

fn missing_section_points(unit: &TacticalUnit, section_id: &SectionId) -> u64 {
    let mut missing: u64 = 0;
    for (addr, cell) in unit.cells() {
        let belongs_to_sec = match addr {
            CellAddr::Section { section, .. } | CellAddr::Facet { section, .. } => {
                section == section_id
            }
            _ => false,
        };
        if belongs_to_sec && cell.current < cell.max {
            let diff = u64::try_from(cell.max.saturating_sub(cell.current)).unwrap_or(0);
            missing = missing.saturating_add(diff);
        }
    }
    missing
}

/// Errors produced by `H3` component stripping, hull scrapping, and workshop restoration.
#[derive(Clone, Debug, PartialEq)]
pub enum WorkshopError {
    /// Object id was `ObjId(0)`.
    InvalidObjectId(&'static str),
    /// Monotonic id counter overflowed `u64`.
    IdOverflow,
    /// Work duration was `<= 0`.
    InvalidWorkDuration(i64),
    /// Rearm amount was `<= 0`.
    InvalidRearmAmount(i64),
    /// Unknown structural section on the unit.
    UnknownSection(SectionId),
    /// Unknown component on the unit.
    UnknownComponent(ComponentId),
    /// Unknown cell on the target component.
    UnknownComponentCell {
        /// Component id.
        component: ComponentId,
        /// Cell id.
        cell: CellId,
    },
    /// Field stripping on owned ground lacked owner or landholder authority (`Q-F6`).
    UnlawfulFieldStrip {
        /// Hull object id.
        hull: ObjId,
        /// Component id.
        component: ComponentId,
    },
    /// Component was already stripped from this hull (`Q-F6`).
    ComponentAlreadyStripped {
        /// Hull object id.
        hull: ObjId,
        /// Component id.
        component: ComponentId,
    },
    /// Cannot strip a `Destroyed` (`slagged`) component (`Q-S11`, `FO-2`).
    CannotStripDestroyedComponent {
        /// Hull object id.
        hull: ObjId,
        /// Component id.
        component: ComponentId,
    },
    /// Hull is already `Irrecoverable` and cannot be scrapped or reconstructed.
    HullAlreadyIrrecoverable(ObjId),
    /// Section is `Destroyed`, `Wreck`, or `Irrecoverable` and requires [`WorkshopBook::reconstruct_section`]
    /// rather than in-place `repair_section` (`Q-S12`).
    SectionRequiresReconstruction {
        /// Hull object id.
        hull: ObjId,
        /// Section id.
        section: SectionId,
        /// Current section condition.
        condition: SectionCondition,
    },
    /// Host section is `Destroyed`, `Wreck`, or `Irrecoverable`; reconstruct the section
    /// before repairing or replacing a component inside it (`Q-S12`).
    HostSectionNotFunctional {
        /// Hull object id.
        hull: ObjId,
        /// Host section id.
        section: SectionId,
        /// Current section condition.
        condition: SectionCondition,
    },
    /// Component is `Destroyed` (`slagged`) or `"stripped"` and requires [`WorkshopBook::replace_component_in_slot`]
    /// with a replacement part rather than in-place `repair_component` (`FO-2`, `Q-S12`).
    ComponentRequiresReplacement {
        /// Hull object id.
        hull: ObjId,
        /// Component id.
        component: ComponentId,
    },
    /// Replacement part condition was `Destroyed`.
    InvalidReplacementPartCondition(ComponentCondition),
    /// Unknown [`StrippedComponentRecord`] id.
    UnknownStrippedComponentRecord(u64),
    /// [`StrippedComponentRecord`] was already consumed into another hull.
    StrippedComponentAlreadyConsumed {
        /// Stripped component record id.
        record_id: u64,
        /// Hull into which it was already installed.
        consumed_into_hull: ObjId,
    },
    /// Replacement part definition did not match the slot's expected definition.
    IncompatibleReplacementDefinition {
        /// Target component slot id.
        component: ComponentId,
        /// Expected definition reference.
        expected: Box<DefinitionRef>,
        /// Supplied definition reference.
        actual: Box<DefinitionRef>,
    },
    /// Slot still holds a surviving (`Intact`, `Damaged`, or `Jammed`) component; repair or
    /// strip it first before installing a replacement part.
    SlotAlreadyHasIntactOrRepairableComponent {
        /// Hull object id.
        hull: ObjId,
        /// Component id.
        component: ComponentId,
        /// Current effective condition.
        condition: ComponentCondition,
    },
    /// Maximum stripped component records reached.
    MaxStrippedComponentsReached {
        /// Maximum records.
        max: usize,
    },
    /// Maximum workshop work orders reached.
    MaxWorkOrdersReached {
        /// Maximum orders.
        max: usize,
    },
    /// Lawful workshop refused to install a contraband hot component (`FO-4`, `Q-S4`, `Q-F6`).
    ContrabandComponentRefused {
        /// Stripped component record id.
        record_id: u64,
        /// Parent hull from which the part was stripped.
        parent_hull: ObjId,
        /// Rightful legal title holder.
        rightful_owner: ObjId,
        /// Contraband reason.
        reason: ContrabandReason,
    },
    /// Actor attempting to clear contraband provenance is not the rightful owner.
    UnauthorizedContrabandClearance {
        /// Stripped component record id.
        record_id: u64,
        /// Rightful owner of the part.
        rightful_owner: ObjId,
        /// Actor who attempted clearance.
        actor: ObjId,
    },
    /// Underlying [`SalvageError`].
    Salvage(SalvageError),
    /// Underlying [`UnitError`].
    Unit(UnitError),
}

impl fmt::Display for WorkshopError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidObjectId(field) => write!(f, "{field} object id must be non-zero"),
            Self::IdOverflow => f.write_str("workshop id counter overflow"),
            Self::InvalidWorkDuration(d) => write!(f, "work duration must be positive, got {d}"),
            Self::InvalidRearmAmount(n) => write!(f, "rearm amount must be positive, got {n}"),
            Self::UnknownSection(s) => write!(f, "unknown section {s}"),
            Self::UnknownComponent(c) => write!(f, "unknown component {c}"),
            Self::UnknownComponentCell { component, cell } => {
                write!(f, "unknown cell {cell} on component {component}")
            }
            Self::UnlawfulFieldStrip { hull, component } => write!(
                f,
                "unlawful field strip of {component} from {hull} on owned ground is prohibited"
            ),
            Self::ComponentAlreadyStripped { hull, component } => {
                write!(f, "component {component} on {hull} is already stripped")
            }
            Self::CannotStripDestroyedComponent { hull, component } => write!(
                f,
                "cannot strip destroyed component {component} from {hull}"
            ),
            Self::HullAlreadyIrrecoverable(u) => write!(f, "hull {u} is already irrecoverable"),
            Self::SectionRequiresReconstruction {
                hull,
                section,
                condition,
            } => write!(
                f,
                "section {section} on {hull} is in condition {} and requires reconstruction",
                condition.as_str()
            ),
            Self::HostSectionNotFunctional {
                hull,
                section,
                condition,
            } => write!(
                f,
                "host section {section} on {hull} is in condition {}; reconstruct it before component work",
                condition.as_str()
            ),
            Self::ComponentRequiresReplacement { hull, component } => write!(
                f,
                "component {component} on {hull} is destroyed/stripped and requires a replacement part"
            ),
            Self::InvalidReplacementPartCondition(c) => {
                write!(f, "replacement part condition {} is not usable", c.as_str())
            }
            Self::UnknownStrippedComponentRecord(id) => {
                write!(f, "unknown stripped component record {id}")
            }
            Self::StrippedComponentAlreadyConsumed {
                record_id,
                consumed_into_hull,
            } => write!(
                f,
                "stripped component record {record_id} was already installed into {consumed_into_hull}"
            ),
            Self::IncompatibleReplacementDefinition {
                component,
                expected,
                actual,
            } => write!(
                f,
                "incompatible replacement definition for {component}: expected {expected:?}, got {actual:?}"
            ),
            Self::SlotAlreadyHasIntactOrRepairableComponent {
                hull,
                component,
                condition,
            } => write!(
                f,
                "component slot {component} on {hull} still holds a {} component; repair or strip it first",
                condition.as_str()
            ),
            Self::MaxStrippedComponentsReached { max } => {
                write!(f, "stripped component limit ({max}) reached")
            }
            Self::MaxWorkOrdersReached { max } => {
                write!(f, "workshop work-order limit ({max}) reached")
            }
            Self::ContrabandComponentRefused {
                record_id,
                parent_hull,
                rightful_owner,
                reason,
            } => write!(
                f,
                "workshop refused contraband component {record_id} from {parent_hull} (rightful owner {rightful_owner}, reason {reason:?})"
            ),
            Self::UnauthorizedContrabandClearance {
                record_id,
                rightful_owner,
                actor,
            } => write!(
                f,
                "actor {actor} is not authorized to clear contraband component {record_id} owned by {rightful_owner}"
            ),
            Self::Salvage(e) => write!(f, "salvage authority error: {e}"),
            Self::Unit(e) => write!(f, "tactical unit error: {e}"),
        }
    }
}

impl std::error::Error for WorkshopError {}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::error::Error;

    use mush_core::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, ObjId,
        OwnerId,
    };

    use super::*;
    use crate::field_ops::recovery::RecoveryOpId;
    use crate::field_ops::salvage::{GroundTenure, SalvageClaimId};
    use crate::mobility::{
        LocomotionId, MediumId, MobilityMode, MobilityModeId, MobilityProfile, MobilityVocabulary,
        ModeRules,
    };
    use crate::unit::{
        CellSpec, ComponentSpec, FacetId, FacetSpec, RulesValue, SectionSpec, UnitSpec,
        UnitSpecBody,
    };

    type TestResult = Result<(), Box<dyn Error>>;

    struct WorkshopFixture {
        unit: TacticalUnit,
        anatomy: UnitAnatomyProfile,
        cockpit_def: DefinitionRef,
    }

    fn build_workshop_test_biped(obj: ObjId) -> Result<WorkshopFixture, Box<dyn Error>> {
        let mut reg = DefinitionRegistryBuilder::new();
        let owner = OwnerId::new("test")?;
        let rules_ref = reg.register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("schema")?,
                DefinitionId::new("unit_rules")?,
                1,
                serde_json::json!({}),
            )?,
        )?;
        let mode_rules_ref = reg.register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("schema")?,
                DefinitionId::new("walk_rules")?,
                1,
                serde_json::json!({}),
            )?,
        )?;
        let cockpit_def = reg.register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("component")?,
                DefinitionId::new("cockpit")?,
                1,
                serde_json::json!({}),
            )?,
        )?;
        let reactor_def = reg.register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("component")?,
                DefinitionId::new("reactor")?,
                1,
                serde_json::json!({}),
            )?,
        )?;
        let autocannon_def = reg.register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("component")?,
                DefinitionId::new("autocannon")?,
                1,
                serde_json::json!({}),
            )?,
        )?;

        let head = SectionId::new("head")?;
        let torso = SectionId::new("torso")?;
        let left_leg = SectionId::new("left_leg")?;
        let right_leg = SectionId::new("right_leg")?;
        let internals = CellId::new("internals")?;
        let comp_hp = CellId::new("condition")?;
        let ammo = CellId::new("ammo")?;
        let intact = ConditionId::new("intact")?;

        let cockpit = ComponentId::new("cockpit")?;
        let reactor = ComponentId::new("reactor")?;
        let autocannon = ComponentId::new("autocannon")?;

        let make_sec = |id: SectionId, hp: i64| -> Result<SectionSpec, Box<dyn Error>> {
            Ok(SectionSpec {
                id,
                condition: intact.clone(),
                facets: vec![FacetSpec {
                    id: FacetId::new("front")?,
                    cells: vec![],
                }],
                slots: vec![],
                cells: vec![CellSpec {
                    id: internals.clone(),
                    min: 0,
                    max: hp,
                    init: hp,
                }],
            })
        };

        let mode_id = MobilityModeId::new("walk")?;
        let mob_reg = MobilityVocabulary::base().freeze();
        let mobility = Some(MobilityProfile::new(
            vec![MobilityMode::new(
                mode_id.clone(),
                [MediumId::new("ground")?],
                LocomotionId::new("biped")?,
                ModeRules::new(mode_rules_ref, serde_json::json!({}))?,
            )],
            mode_id,
            &mob_reg,
        )?);

        let body = UnitSpecBody {
            sections: vec![
                make_sec(head.clone(), 10)?,
                make_sec(torso.clone(), 30)?,
                make_sec(left_leg.clone(), 15)?,
                make_sec(right_leg.clone(), 15)?,
            ],
            unit_cells: vec![],
            components: vec![
                ComponentSpec {
                    id: cockpit,
                    definition: cockpit_def.clone(),
                    install: InstallPoint::Section(head.clone()),
                    condition: intact.clone(),
                    cells: vec![CellSpec {
                        id: comp_hp.clone(),
                        min: 0,
                        max: 10,
                        init: 10,
                    }],
                    asset: Some(ObjId(801)),
                },
                ComponentSpec {
                    id: reactor.clone(),
                    definition: reactor_def,
                    install: InstallPoint::Section(torso.clone()),
                    condition: intact.clone(),
                    cells: vec![CellSpec {
                        id: comp_hp.clone(),
                        min: 0,
                        max: 20,
                        init: 20,
                    }],
                    asset: None,
                },
                ComponentSpec {
                    id: autocannon.clone(),
                    definition: autocannon_def,
                    install: InstallPoint::Section(torso.clone()),
                    condition: intact,
                    cells: vec![
                        CellSpec {
                            id: comp_hp.clone(),
                            min: 0,
                            max: 15,
                            init: 15,
                        },
                        CellSpec {
                            id: ammo.clone(),
                            min: 0,
                            max: 20,
                            init: 20,
                        },
                    ],
                    asset: Some(ObjId(802)),
                },
            ],
            capabilities: vec![],
            mobility,
            rules: RulesValue::new(rules_ref, serde_json::json!({}))?,
        };
        let unit_desc =
            body.descriptor(DefinitionKind::new("unit")?, DefinitionId::new("biped")?, 1)?;
        let unit_ref = reg.register(owner, unit_desc)?;
        let spec: UnitSpec = body.with_definition(unit_ref);

        let unit = TacticalUnit::from_spec(&spec, obj)?;
        let anatomy = UnitAnatomyProfile {
            core_sections: BTreeSet::from([torso]),
            control_sections: BTreeSet::from([head]),
            required_locomotion_sections: BTreeSet::new(),
            locomotion_sections: BTreeSet::from([left_leg, right_leg]),
            min_locomotion_sections: 2,
            power_components: BTreeSet::from([reactor]),
            weapon_components: BTreeSet::from([autocannon]),
            sensor_components: BTreeSet::new(),
            internals_cell: Some(internals),
            component_health_cell: Some(comp_hp),
            weapon_ammo_cell: Some(ammo),
        };
        Ok(WorkshopFixture {
            unit,
            anatomy,
            cockpit_def,
        })
    }

    #[test]
    fn strip_surviving_component_and_reinstall_into_second_hull_with_provenance() -> TestResult {
        let owner = ObjId(10);
        let donor_fix = build_workshop_test_biped(ObjId(101))?;
        let recipient_fix = build_workshop_test_biped(ObjId(102))?;
        let donor_before = donor_fix.unit;
        let anatomy = donor_fix.anatomy;
        let recipient_before = recipient_fix.unit;

        let head = SectionId::new("head")?;
        let internals = CellId::new("internals")?;
        let autocannon = ComponentId::new("autocannon")?;
        let cockpit = ComponentId::new("cockpit")?;
        let ammo = CellId::new("ammo")?;
        let comp_hp = CellId::new("condition")?;

        let donor_damage = UnitMutationPlan::new(
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
                        component: autocannon.clone(),
                        cell: ammo.clone(),
                    },
                    value: 14,
                },
            ],
            Some(donor_before.revision()),
            None,
        )?;
        let (donor_disabled, _) = apply(&donor_before, &donor_damage, 100)?;

        let mut workshop = WorkshopBook::new();
        let mut salvage_book = SalvageClaimBook::new();

        let pending_custody = CustodyTitleStatus::PendingSalvageClaim {
            claim_id: SalvageClaimId::new(1)?,
            custodian: ObjId(99),
            prior_owner: owner,
            deadline_real_s: 200_000,
        };
        check!(
            workshop
                .strip_component(
                    &donor_disabled,
                    &anatomy,
                    &autocannon,
                    ObjId(99),
                    &StripAuthorization::BayOrWorkshop {
                        custody: pending_custody.clone(),
                        has_explicit_owner_permission: false,
                    },
                    Some(&mut salvage_book),
                    110,
                )
                .is_err()
        );

        let resolved_custody = CustodyTitleStatus::ResolvedTitle { owner };
        check!(matches!(
            workshop.strip_component(
                &donor_disabled,
                &anatomy,
                &cockpit,
                owner,
                &StripAuthorization::BayOrWorkshop {
                    custody: resolved_custody.clone(),
                    has_explicit_owner_permission: false,
                },
                Some(&mut salvage_book),
                115,
            ),
            Err(WorkshopError::CannotStripDestroyedComponent { .. })
        ));

        let (donor_after_strip, donor_post_assess, stripped_rec) = workshop.strip_component(
            &donor_disabled,
            &anatomy,
            &autocannon,
            owner,
            &StripAuthorization::BayOrWorkshop {
                custody: resolved_custody.clone(),
                has_explicit_owner_permission: false,
            },
            Some(&mut salvage_book),
            120,
        )?;

        check_eq!(stripped_rec.provenance.parent_hull, donor_disabled.object());
        check_eq!(stripped_rec.condition, ComponentCondition::Intact);
        check_eq!(stripped_rec.cells.get(&ammo).map(|c| c.current), Some(14));
        check_eq!(stripped_rec.asset, Some(ObjId(802)));
        check!(!donor_post_assess.can_fire);

        check!(matches!(
            workshop.strip_component(
                &donor_after_strip,
                &anatomy,
                &autocannon,
                owner,
                &StripAuthorization::BayOrWorkshop {
                    custody: resolved_custody.clone(),
                    has_explicit_owner_permission: false,
                },
                Some(&mut salvage_book),
                125,
            ),
            Err(WorkshopError::ComponentAlreadyStripped { .. })
        ));

        let recipient_damage = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetCell {
                    addr: CellAddr::Component {
                        component: autocannon.clone(),
                        cell: comp_hp,
                    },
                    value: 0,
                },
                UnitMutationOp::SetComponentCondition {
                    component: autocannon.clone(),
                    condition: ConditionId::new("destroyed")?,
                },
                UnitMutationOp::SetComponentAsset {
                    component: autocannon.clone(),
                    asset: None,
                },
            ],
            Some(recipient_before.revision()),
            None,
        )?;
        let (recipient_unarmed, _) = apply(&recipient_before, &recipient_damage, 200)?;
        let recipient_assess = assess_tactical_unit(&recipient_unarmed, &anatomy);
        check_eq!(recipient_assess.state, OperationalState::Unarmed);

        check!(matches!(
            workshop.repair_component(
                &recipient_unarmed,
                &anatomy,
                &autocannon,
                owner,
                &resolved_custody,
                false,
                500,
                60,
                None,
                210,
            ),
            Err(WorkshopError::ComponentRequiresReplacement { .. })
        ));

        let replacement_spec = ReplacementPartSpec::from_stripped_record(&stripped_rec);
        let (recipient_restored, restored_assess, order) = workshop.replace_component_in_slot(
            &recipient_unarmed,
            &anatomy,
            &autocannon,
            &replacement_spec,
            owner,
            &resolved_custody,
            false,
            800,
            120,
            None,
            220,
        )?;
        check_eq!(order.state_before, OperationalState::Unarmed);
        check_eq!(order.state_after, OperationalState::Operational);
        check_eq!(restored_assess.state, OperationalState::Operational);
        check!(restored_assess.can_fire);
        check_eq!(
            recipient_restored
                .cell(&CellAddr::Component {
                    component: autocannon,
                    cell: ammo,
                })
                .map(|c| c.current),
            Some(14)
        );

        check!(
            workshop
                .scrap_hull(
                    &donor_after_strip,
                    &anatomy,
                    ObjId(99),
                    &pending_custody,
                    false,
                    300
                )
                .is_err()
        );
        let (_scrapped_donor, yield_report) = workshop.scrap_hull(
            &donor_after_strip,
            &anatomy,
            owner,
            &resolved_custody,
            false,
            310,
        )?;
        check_eq!(
            yield_report.final_assessment.state,
            OperationalState::Irrecoverable
        );
        check_eq!(yield_report.structural_scrap_kg, 6_000);
        check_eq!(yield_report.dismantled_salvageable_components, 1);

        Ok(())
    }

    #[test]
    fn reconstruct_destroyed_head_and_replace_cockpit_restores_disabled_biped_to_operational()
    -> TestResult {
        let owner = ObjId(10);
        let fix = build_workshop_test_biped(ObjId(105))?;
        let before = fix.unit;
        let anatomy = fix.anatomy;
        let head = SectionId::new("head")?;
        let internals = CellId::new("internals")?;
        let cockpit = ComponentId::new("cockpit")?;
        let autocannon = ComponentId::new("autocannon")?;
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
                    section: head.clone(),
                    condition: ConditionId::new("destroyed")?,
                },
                UnitMutationOp::SetCell {
                    addr: CellAddr::Component {
                        component: autocannon.clone(),
                        cell: ammo.clone(),
                    },
                    value: 17,
                },
            ],
            Some(before.revision()),
            None,
        )?;
        let (disabled_unit, _) = apply(&before, &headshot, 100)?;
        let disabled_assess = assess_tactical_unit(&disabled_unit, &anatomy);
        check_eq!(disabled_assess.state, OperationalState::Disabled);

        let mut workshop = WorkshopBook::new();
        let custody = CustodyTitleStatus::ResolvedTitle { owner };

        let new_cockpit = ReplacementPartSpec {
            source_stripped_record_id: None,
            definition: fix.cockpit_def,
            condition: ComponentCondition::Intact,
            cell_values: BTreeMap::new(),
            asset: Some(ObjId(899)),
        };
        check!(matches!(
            workshop.replace_component_in_slot(
                &disabled_unit,
                &anatomy,
                &cockpit,
                &new_cockpit,
                owner,
                &custody,
                false,
                1_000,
                180,
                None,
                110,
            ),
            Err(WorkshopError::HostSectionNotFunctional { .. })
        ));

        let (head_rebuilt, head_assess, _) = workshop.reconstruct_section(
            &disabled_unit,
            &anatomy,
            &head,
            owner,
            &custody,
            false,
            2_000,
            240,
            None,
            120,
        )?;
        check_eq!(
            head_assess.section_conditions.get(&head),
            Some(&SectionCondition::Intact)
        );
        check_eq!(
            head_assess.component_conditions.get(&cockpit),
            Some(&ComponentCondition::Destroyed)
        );

        let (fully_restored, final_assess, _) = workshop.replace_component_in_slot(
            &head_rebuilt,
            &anatomy,
            &cockpit,
            &new_cockpit,
            owner,
            &custody,
            false,
            1_500,
            180,
            None,
            130,
        )?;
        check_eq!(final_assess.state, OperationalState::Operational);
        check_eq!(
            final_assess.component_conditions.get(&cockpit),
            Some(&ComponentCondition::Intact)
        );
        check_eq!(
            fully_restored
                .cell(&CellAddr::Component {
                    component: autocannon,
                    cell: ammo,
                })
                .map(|c| c.current),
            Some(17)
        );

        Ok(())
    }

    #[test]
    fn field_strip_records_part_salvage_claim_and_prevents_chassis_double_claim() -> TestResult {
        let unit_owner = ObjId(10);
        let salvager = ObjId(20);
        let fix = build_workshop_test_biped(ObjId(109))?;
        let unit = fix.unit;
        let anatomy = fix.anatomy;
        let autocannon = ComponentId::new("autocannon")?;

        let mut workshop = WorkshopBook::new();
        let mut salvage_book = SalvageClaimBook::new();

        let field_ctx = RecoveryAuthorityContext {
            recoverer: salvager,
            unit_title_holder: unit_owner,
            has_owner_authorization: false,
            has_landholder_authorization: false,
            ground_tenure_at_attach: GroundTenure::Unclaimed,
        };

        let (_unit_after, _assess, stripped_rec) = workshop.strip_component(
            &unit,
            &anatomy,
            &autocannon,
            salvager,
            &StripAuthorization::Field {
                authority_ctx: field_ctx.clone(),
            },
            Some(&mut salvage_book),
            500,
        )?;

        let op_id = RecoveryOpId::new(77)?;
        let part_status = salvage_book.file_part_salvage_claim(
            stripped_rec.record_id,
            unit.object(),
            autocannon.clone(),
            op_id,
            &field_ctx,
            1_000,
        )?;
        check!(matches!(
            part_status,
            CustodyTitleStatus::PendingSalvageClaim { .. }
        ));

        salvage_book.record_physical_attach(op_id, unit.object(), ObjId(201), &field_ctx, 510)?;
        salvage_book.record_bay_load(op_id, ObjId(901), 550)?;
        let hull_status = salvage_book.resolve_hull_recovery_custody(op_id, 1_100)?;
        if let CustodyTitleStatus::PendingSalvageClaim { claim_id, .. } = hull_status {
            let hull_claim = salvage_book
                .claim(claim_id)
                .ok_or("missing hull salvage claim")?;
            check!(hull_claim.excluded_components.contains(&autocannon));
        } else {
            return Err("expected PendingSalvageClaim for hull".into());
        }

        Ok(())
    }

    #[test]
    fn quote_restoration_and_rearm_depleted_weapon_restores_unarmed_unit() -> TestResult {
        let owner = ObjId(10);
        let fix = build_workshop_test_biped(ObjId(112))?;
        let before = fix.unit;
        let anatomy = fix.anatomy;
        let autocannon = ComponentId::new("autocannon")?;
        let ammo = CellId::new("ammo")?;

        let deplete = UnitMutationPlan::new(
            vec![UnitMutationOp::SetCell {
                addr: CellAddr::Component {
                    component: autocannon.clone(),
                    cell: ammo.clone(),
                },
                value: 0,
            }],
            Some(before.revision()),
            None,
        )?;
        let (unarmed_unit, _) = apply(&before, &deplete, 100)?;
        let unarmed_assess = assess_tactical_unit(&unarmed_unit, &anatomy);
        check_eq!(unarmed_assess.state, OperationalState::Unarmed);

        let mut workshop = WorkshopBook::new();
        let tariff = WorkshopFacilityTariff::default();
        let quote = workshop.quote_full_restoration(&unarmed_unit, &anatomy, &tariff)?;
        check_eq!(quote.components_to_rearm.get(&autocannon), Some(&20));
        check_eq!(quote.total_resource_cost, 200);
        check_eq!(quote.total_work_duration_ic, 30);

        let custody = CustodyTitleStatus::ResolvedTitle { owner };
        let (rearmed_unit, rearmed_assess, order) = workshop.rearm_component(
            &unarmed_unit,
            &anatomy,
            &autocannon,
            &ammo,
            20,
            owner,
            &custody,
            false,
            quote.total_resource_cost,
            quote.total_work_duration_ic,
            None,
            130,
        )?;
        check_eq!(order.state_before, OperationalState::Unarmed);
        check_eq!(order.state_after, OperationalState::Operational);
        check_eq!(rearmed_assess.state, OperationalState::Operational);
        check_eq!(
            rearmed_unit
                .cell(&CellAddr::Component {
                    component: autocannon,
                    cell: ammo,
                })
                .map(|c| c.current),
            Some(20)
        );

        Ok(())
    }
}

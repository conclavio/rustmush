//! IC Field Operations (`H1-A`–`H1-C`, `H2-A`–`H2-C`, `H3`):
//! - [`condition`] (`H1-A`): section-localized component survival, operational states,
//!   hull-epoch close gating, and versioned IC authority handoff.
//! - [`engagement`] (`H1-B` & `H1-C`): field combat entry, motion-clock handoff,
//!   reinforcement, withdrawal, demotion settlement, carrier leave-behind confirmation,
//!   link-loss posture, crew survival / guaranteed NPC extraction, and observer-bounded
//!   post-battle records.
//! - [`recovery`] (`H2-A`): universal tow coupling (`<= 2x` tug mass cap), tonnage
//!   speed scaling, field inspection, exclusive attachment, coupled movement, and
//!   carrier bay loading.
//! - [`salvage`] (`H2-B` & `H2-C`): ground tenure at attach, lawful/unlawful recovery
//!   entitlement, private recovery records, evidence-gated unlawful salvage actions,
//!   post-close black-box access rules, public salvage title claims, and the
//!   48-available-real-hour contest workflow.
//! - [`workshop`] (`H3`): component stripping with parent-hull provenance, hull
//!   scrapping, in-place repair (including pending-custody repair-at-risk), section
//!   reconstruction, destroyed/stripped component replacement, rearming, and cost quoting.
//! - [`sortie`] (`FO-6`–`FO-9`, `FO-15`, `FO-16`, `Q-D3`–`Q-D6`, `Q-A4`, `Q-F1`, `Q-F2`,
//!   `Q-R2`, `Q-R5`, `P-1`): detachment sortie staging, partial-launch persistence,
//!   carrier personnel embarkation manifests, NPC crew contracts/wages, unit expense
//!   policies, bounded mission-leader succession, and field-to-roster availability sync.
//! - [`quarantine`] (`W-clock` consumer, `Q-H11`, `P-4`): post-outage audit quarantine of
//!   field units whose persisted state disagrees with their site books, stamped from the
//!   kernel's `WorldClockResumed` event. The clock itself — the durable checkpoint, the
//!   outage evaluation, the countdown and its broadcasts — is the kernel's
//!   (`mush_core::clock::WorldTimeline`); the salvage contest windows pause and extend from
//!   the same events (`salvage`, `Q-F3`).
//! - [`engineering`] (`H1-A`, `H2-A`, `H3` Field vs. Workshop Boundary): mobile field
//!   engineering tenders, in-field weapon unjamming, damaged-section stabilization, field
//!   ammo/charge resupply, and workshop-boundary enforcement.

pub mod condition;
pub mod engagement;
pub mod engineering;
pub mod quarantine;
pub mod recovery;
pub mod salvage;
pub mod sortie;
pub mod workshop;

pub use condition::{
    AuthorityHandoffRequest, ComponentCondition, ConditionError, FieldAuthorityKind,
    FieldCloseOutcome, FieldCustodyLedger, FieldCustodyRecord, FieldEpochDisposition,
    FieldUnitAssessment, MAX_FIELD_CUSTODY_UNITS, OperationalState, SectionCondition,
    UnitAnatomyProfile, assess_tactical_unit, effective_component_condition,
    effective_section_condition, ic_condition_plan, install_point_section, validate_close_outcome,
};
pub use engagement::{
    COMBAT_WALL_CLOCK_RATE, CarrierDepartureOutcome, CrewSurvivalBook, DEFAULT_FALLBACK_SAFE_PORT,
    DemotionOutcome, DemotionSettlementGate, EngagementError, FieldObservationBook,
    FieldPromotionTrigger, FieldSiteEngagementBook, IC_SURFACE_CLOCK_RATE, LinkLossPosture,
    MAX_CREW_RECORDS, MAX_OBSERVATIONS_PER_ACTOR, MAX_SITE_UNITS, MotionClockHandoff,
    ObservationSource, ObservedCombatFact, ObserverCombatEntry, OccupantSurvivalState,
    SafePortCandidate, SiteUnitPresence, WithdrawalDestination, resolve_link_loss_posture,
    select_extraction_port,
};
pub use engineering::{
    DEFAULT_MAX_TENDER_SERVICE_DISTANCE_CM, DEFAULT_SPARE_PARTS_KG_PER_STRUCTURAL_POINT,
    DEFAULT_UNJAM_SPARE_PARTS_KG, FieldEngineeringActionKind, FieldEngineeringBook,
    FieldEngineeringError, FieldEngineeringLogEntry, FieldTenderState, MAX_FIELD_ENGINEERING_LOGS,
    MAX_FIELD_TENDERS,
};
pub use quarantine::{
    MAX_QUARANTINED_UNITS, OutageQuarantineBook, QuarantineError, QuarantinedUnitRecord,
};
pub use recovery::{
    AttachTowRequest, BayCapacitySpec, DEFAULT_MAX_ATTACH_DISTANCE_CM, FieldInspectionReport,
    FieldPose, LoadIntoBayRequest, LoadedCasualtyOutcome, MAX_RECOVERY_OPERATIONS,
    MAX_TOW_MASS_RATIO_PERMILLE, RecoveryBook, RecoveryError, RecoveryOpId, RecoveryOperation,
    RecoveryStage, TowEvaluation, TowRigProfile, TugInspectionContext, UnitMassProfile,
    inspect_field_unit, scaled_tow_speed_cms, tow_mass_ratio_permille, tow_speed_factor_permille,
};
pub use salvage::{
    AdjustedSalvageClaimDeadline, AtRiskRepairEntry, AuthoritativeRecoveryRecord,
    BlackBoxFlightEntry, BlackBoxForensicReport, CONTEST_WINDOW_AVAILABLE_REAL_SECS, ContestGround,
    ContestSuccessReason, CustodyAction, CustodyTitleStatus, GroundTenure,
    MAX_AUTHORITATIVE_RECOVERY_RECORDS, MAX_BLACK_BOX_ENTRIES_PER_HULL, MAX_CLAIM_ENTRIES,
    MAX_SALVAGE_CLAIMS, RecoveryAuthorityContext, RecoveryEntitlement, SalvageClaim,
    SalvageClaimBook, SalvageClaimId, SalvageClaimState, SalvageContest, SalvageError,
    SalvageEvidenceKind, SalvageEvidenceRecord, SalvageSubject, check_custody_action_allowed,
    evaluate_recovery_entitlement,
};
pub use sortie::{
    ActiveFieldSortie, CarrierPersonnelManifest, CrewMemberKind, EmbarkationOrigin,
    EmbarkedPersonRecord, IC_DAY_SECONDS, MAX_CREW_POOL_CONTRACTS, MAX_MANIFEST_PERSONNEL,
    MissionLeaderRoleGrant, NpcCrewContract, NpcMissionScope, PortCrewPool, SortieError,
    SortieLaunchBlockReason, SortieSlotLaunchState, UnitExpenseCategory, UnitExpensePolicy,
    is_occupant_fit_for_duty, sync_detachment_availability_from_field,
};
pub use workshop::{
    ComponentProvenance, ContrabandReason, FALLBACK_SECTION_SCRAP_KG, MAX_STRIPPED_COMPONENTS,
    MAX_WORKSHOP_ORDERS, ProvenanceLegalityStatus, ReplacementPartSpec,
    SCRAP_KG_PER_STRUCTURAL_POINT, ScrapYieldReport, StripAuthorization, StrippedComponentRecord,
    WorkshopBook, WorkshopCostQuote, WorkshopError, WorkshopFacilityTariff, WorkshopOperationKind,
    WorkshopWorkOrderRecord,
};

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::error::Error;

    use mush_core::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, ObjId,
        OwnerId, WorldEvent,
    };
    use smol_str::SmolStr;

    use super::*;
    use crate::mobility::{
        LocomotionId, MediumId, MobilityMode, MobilityModeId, MobilityProfile, MobilityVocabulary,
        ModeRules,
    };
    use crate::unit::{
        CapabilityId, CapabilityInit, CapabilityKey, CellAddr, CellId, CellSpec, ComponentId,
        ComponentSpec, ConditionId, InstallPoint, InstallSlotId, RulesValue, SectionId,
        SectionSpec, TacticalUnit, UnitMutationOp, UnitMutationPlan, UnitSpec, apply,
    };

    type TestResult = Result<(), Box<dyn Error>>;

    struct TestFixture {
        unit: TacticalUnit,
        anatomy: UnitAnatomyProfile,
    }

    fn build_biped_fixture(object: ObjId) -> Result<TestFixture, Box<dyn Error>> {
        let mut reg = DefinitionRegistryBuilder::new();
        let owner = OwnerId::new("tacsim")?;
        let rules_ref = reg.register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("schema")?,
                DefinitionId::new("biped_rules")?,
                1,
                serde_json::json!({"class": "biped"}),
            )?,
        )?;
        let mode_rules_ref = reg.register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("schema")?,
                DefinitionId::new("stride_rules")?,
                1,
                serde_json::json!({"max_speed_cms": 1200}),
            )?,
        )?;
        let comp_def = reg.register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("component")?,
                DefinitionId::new("std_part")?,
                1,
                serde_json::json!({"tier": 1}),
            )?,
        )?;
        let cap_rules_ref = reg.register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("schema")?,
                DefinitionId::new("cap_rules")?,
                1,
                serde_json::json!({"ok": true}),
            )?,
        )?;

        let intact = ConditionId::new("intact")?;
        let internals_id = CellId::new("internals")?;
        let cond_cell_id = CellId::new("condition")?;
        let ammo_cell_id = CellId::new("ammo")?;

        let head = SectionId::new("head")?;
        let torso = SectionId::new("torso")?;
        let left_arm = SectionId::new("left_arm")?;
        let right_arm = SectionId::new("right_arm")?;
        let left_leg = SectionId::new("left_leg")?;
        let right_leg = SectionId::new("right_leg")?;

        let sections = vec![
            SectionSpec {
                id: head.clone(),
                condition: intact.clone(),
                facets: Vec::new(),
                slots: vec![InstallSlotId::new("cockpit")?],
                cells: vec![CellSpec {
                    id: internals_id.clone(),
                    min: 0,
                    max: 8,
                    init: 8,
                }],
            },
            SectionSpec {
                id: torso.clone(),
                condition: intact.clone(),
                facets: Vec::new(),
                slots: vec![
                    InstallSlotId::new("core")?,
                    InstallSlotId::new("shoulder")?,
                    InstallSlotId::new("mast")?,
                ],
                cells: vec![CellSpec {
                    id: internals_id.clone(),
                    min: 0,
                    max: 24,
                    init: 24,
                }],
            },
            SectionSpec {
                id: left_arm.clone(),
                condition: intact.clone(),
                facets: Vec::new(),
                slots: vec![InstallSlotId::new("hardpoint")?],
                cells: vec![CellSpec {
                    id: internals_id.clone(),
                    min: 0,
                    max: 12,
                    init: 12,
                }],
            },
            SectionSpec {
                id: right_arm.clone(),
                condition: intact.clone(),
                facets: Vec::new(),
                slots: vec![InstallSlotId::new("hardpoint")?],
                cells: vec![CellSpec {
                    id: internals_id.clone(),
                    min: 0,
                    max: 12,
                    init: 12,
                }],
            },
            SectionSpec {
                id: left_leg.clone(),
                condition: intact.clone(),
                facets: Vec::new(),
                slots: Vec::new(),
                cells: vec![CellSpec {
                    id: internals_id.clone(),
                    min: 0,
                    max: 14,
                    init: 14,
                }],
            },
            SectionSpec {
                id: right_leg.clone(),
                condition: intact.clone(),
                facets: Vec::new(),
                slots: Vec::new(),
                cells: vec![CellSpec {
                    id: internals_id.clone(),
                    min: 0,
                    max: 14,
                    init: 14,
                }],
            },
        ];

        let power_plant = ComponentId::new("power_plant")?;
        let mast_array = ComponentId::new("mast_array")?;
        let carbine = ComponentId::new("carbine")?;
        let needler = ComponentId::new("needler")?;
        let missile_rack = ComponentId::new("missile_rack")?;

        let components = vec![
            ComponentSpec {
                id: power_plant.clone(),
                definition: comp_def.clone(),
                install: InstallPoint::Slot {
                    section: torso.clone(),
                    slot: InstallSlotId::new("core")?,
                },
                condition: intact.clone(),
                cells: vec![CellSpec {
                    id: cond_cell_id.clone(),
                    min: 0,
                    max: 10,
                    init: 10,
                }],
                asset: None,
            },
            ComponentSpec {
                id: mast_array.clone(),
                definition: comp_def.clone(),
                install: InstallPoint::Slot {
                    section: torso.clone(),
                    slot: InstallSlotId::new("mast")?,
                },
                condition: intact.clone(),
                cells: vec![CellSpec {
                    id: cond_cell_id.clone(),
                    min: 0,
                    max: 10,
                    init: 10,
                }],
                asset: None,
            },
            ComponentSpec {
                id: carbine.clone(),
                definition: comp_def.clone(),
                install: InstallPoint::Slot {
                    section: right_arm.clone(),
                    slot: InstallSlotId::new("hardpoint")?,
                },
                condition: intact.clone(),
                cells: vec![
                    CellSpec {
                        id: cond_cell_id.clone(),
                        min: 0,
                        max: 10,
                        init: 10,
                    },
                    CellSpec {
                        id: ammo_cell_id.clone(),
                        min: 0,
                        max: 30,
                        init: 30,
                    },
                ],
                asset: None,
            },
            ComponentSpec {
                id: needler.clone(),
                definition: comp_def.clone(),
                install: InstallPoint::Slot {
                    section: left_arm.clone(),
                    slot: InstallSlotId::new("hardpoint")?,
                },
                condition: intact.clone(),
                cells: vec![
                    CellSpec {
                        id: cond_cell_id.clone(),
                        min: 0,
                        max: 10,
                        init: 10,
                    },
                    CellSpec {
                        id: ammo_cell_id.clone(),
                        min: 0,
                        max: 40,
                        init: 40,
                    },
                ],
                asset: None,
            },
            ComponentSpec {
                id: missile_rack.clone(),
                definition: comp_def,
                install: InstallPoint::Slot {
                    section: torso.clone(),
                    slot: InstallSlotId::new("shoulder")?,
                },
                condition: intact,
                cells: vec![
                    CellSpec {
                        id: cond_cell_id.clone(),
                        min: 0,
                        max: 10,
                        init: 10,
                    },
                    CellSpec {
                        id: ammo_cell_id.clone(),
                        min: 0,
                        max: 8,
                        init: 8,
                    },
                ],
                asset: None,
            },
        ];

        let capabilities = vec![
            CapabilityInit {
                key: CapabilityKey {
                    id: CapabilityId::new("tacsim.power")?,
                    source: Some(power_plant.clone()),
                },
                rules: RulesValue::new(cap_rules_ref.clone(), serde_json::json!({}))?,
            },
            CapabilityInit {
                key: CapabilityKey {
                    id: CapabilityId::new("tactics.mobility")?,
                    source: None,
                },
                rules: RulesValue::new(cap_rules_ref.clone(), serde_json::json!({}))?,
            },
            CapabilityInit {
                key: CapabilityKey {
                    id: CapabilityId::new("tacsim.weapon.carbine")?,
                    source: Some(carbine.clone()),
                },
                rules: RulesValue::new(cap_rules_ref, serde_json::json!({}))?,
            },
        ];

        let mode_id = MobilityModeId::new("stride")?;
        let mob_reg = MobilityVocabulary::base().freeze();
        let mobility = Some(MobilityProfile::new(
            vec![MobilityMode::new(
                mode_id.clone(),
                [MediumId::new("ground")?],
                LocomotionId::new("biped")?,
                ModeRules::new(mode_rules_ref, serde_json::json!({"max_speed_cms": 1200}))?,
            )],
            mode_id,
            &mob_reg,
        )?);

        let body = crate::unit::UnitSpecBody {
            sections,
            unit_cells: Vec::new(),
            components,
            capabilities,
            mobility,
            rules: RulesValue::new(rules_ref, serde_json::json!({"class": "biped"}))?,
        };
        let unit_desc = body.descriptor(
            DefinitionKind::new("unit")?,
            DefinitionId::new("biped_mk1")?,
            1,
        )?;
        let unit_ref = reg.register(owner, unit_desc)?;
        let spec: UnitSpec = body.with_definition(unit_ref);
        let unit = TacticalUnit::from_spec(&spec, object)?;

        let anatomy = UnitAnatomyProfile {
            core_sections: BTreeSet::from([torso]),
            control_sections: BTreeSet::from([head]),
            required_locomotion_sections: BTreeSet::new(),
            locomotion_sections: BTreeSet::from([left_leg, right_leg]),
            min_locomotion_sections: 2,
            power_components: BTreeSet::from([power_plant]),
            weapon_components: BTreeSet::from([carbine, needler, missile_rack]),
            sensor_components: BTreeSet::from([mast_array]),
            internals_cell: Some(internals_id),
            component_health_cell: Some(cond_cell_id),
            weapon_ammo_cell: Some(ammo_cell_id),
        };

        Ok(TestFixture { unit, anatomy })
    }

    #[test]
    fn h1a_headshot_biped_retains_surviving_sections_and_weapons() -> TestResult {
        let fixture = build_biped_fixture(ObjId(101))?;
        let before = &fixture.unit;

        let initial = assess_tactical_unit(before, &fixture.anatomy);
        check_eq!(initial.state, OperationalState::Operational);
        check!(initial.can_move);
        check!(initial.can_fire);
        check!(!initial.is_towable);
        check_eq!(initial.epoch_disposition, FieldEpochDisposition::RemainOpen);
        check_eq!(initial.intact_components.len(), 5);

        let head = SectionId::new("head")?;
        let torso = SectionId::new("torso")?;
        let internals = CellId::new("internals")?;
        let carbine = ComponentId::new("carbine")?;
        let ammo = CellId::new("ammo")?;

        let arena_ops = vec![
            UnitMutationOp::SetCell {
                addr: CellAddr::Section {
                    section: head.clone(),
                    cell: internals,
                },
                value: 0,
            },
            UnitMutationOp::SetCell {
                addr: CellAddr::Component {
                    component: carbine.clone(),
                    cell: ammo.clone(),
                },
                value: 24,
            },
            UnitMutationOp::SetSectionCondition {
                section: torso.clone(),
                condition: ConditionId::new("wreck")?,
            },
            UnitMutationOp::RevokeCapability {
                key: CapabilityKey {
                    id: CapabilityId::new("tactics.mobility")?,
                    source: None,
                },
            },
        ];
        let arena_plan = UnitMutationPlan::new(arena_ops, Some(before.revision()), None)?;
        let (arena_after, _) = apply(before, &arena_plan, 1_000)?;

        let ic_plan_opt = ic_condition_plan(
            before,
            &arena_after,
            Some(&fixture.anatomy),
            Some(SmolStr::new("ic_headshot_handoff")),
        )?;
        let ic_plan = ic_plan_opt.ok_or("expected non-empty IC condition plan")?;

        for op in ic_plan.ops() {
            let legal = matches!(
                op,
                UnitMutationOp::SetCell { .. }
                    | UnitMutationOp::AdjustCell { .. }
                    | UnitMutationOp::SetSectionCondition { .. }
                    | UnitMutationOp::SetComponentCondition { .. }
            );
            check!(legal, "illegal IC op in plan: {op:?}");
        }

        let (ic_after, _) = apply(before, &ic_plan, 1_005)?;
        let assessment = assess_tactical_unit(&ic_after, &fixture.anatomy);

        check_eq!(assessment.state, OperationalState::Disabled);
        check!(!assessment.has_control);
        check!(assessment.has_power);
        check!(!assessment.can_move);
        check!(!assessment.can_fire);
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
            assessment.section_conditions.get(&torso),
            Some(&SectionCondition::Intact)
        );
        check_eq!(assessment.intact_components.len(), 5);
        check!(assessment.destroyed_components.is_empty());

        let carbine_ammo = ic_after
            .cell(&CellAddr::Component {
                component: carbine,
                cell: ammo,
            })
            .ok_or("missing carbine ammo cell")?;
        check_eq!(carbine_ammo.current, 24);

        check!(
            validate_close_outcome(
                ic_after.object(),
                assessment.state,
                &FieldCloseOutcome::Destroyed
            )
            .is_err()
        );
        check!(
            validate_close_outcome(
                ic_after.object(),
                assessment.state,
                &FieldCloseOutcome::RecoveredToBase { base: ObjId(900) }
            )
            .is_ok()
        );
        Ok(())
    }

    #[test]
    fn h1a_distinct_immobile_unarmed_wreck_and_irrecoverable_states() -> TestResult {
        let fixture = build_biped_fixture(ObjId(102))?;
        let base = &fixture.unit;
        let internals = CellId::new("internals")?;
        let left_leg = SectionId::new("left_leg")?;
        let right_leg = SectionId::new("right_leg")?;
        let left_arm = SectionId::new("left_arm")?;
        let right_arm = SectionId::new("right_arm")?;
        let torso = SectionId::new("torso")?;
        let head = SectionId::new("head")?;
        let missile_rack = ComponentId::new("missile_rack")?;
        let cond_cell = CellId::new("condition")?;

        let leg_plan = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: left_leg.clone(),
                        cell: internals.clone(),
                    },
                    value: 0,
                },
                UnitMutationOp::SetSectionCondition {
                    section: left_leg.clone(),
                    condition: ConditionId::new("destroyed")?,
                },
            ],
            Some(base.revision()),
            None,
        )?;
        let (legging_unit, _) = apply(base, &leg_plan, 100)?;
        let leg_assess = assess_tactical_unit(&legging_unit, &fixture.anatomy);
        check_eq!(leg_assess.state, OperationalState::Immobile);
        check!(!leg_assess.can_move);
        check!(leg_assess.can_fire);
        check!(leg_assess.is_towable);
        check_eq!(leg_assess.operational_weapons.len(), 3);
        check_eq!(
            leg_assess.epoch_disposition,
            FieldEpochDisposition::RemainOpen
        );

        let disarm_plan = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetSectionCondition {
                    section: left_arm.clone(),
                    condition: ConditionId::new("destroyed")?,
                },
                UnitMutationOp::SetSectionCondition {
                    section: right_arm.clone(),
                    condition: ConditionId::new("destroyed")?,
                },
                UnitMutationOp::SetCell {
                    addr: CellAddr::Component {
                        component: missile_rack.clone(),
                        cell: cond_cell,
                    },
                    value: 0,
                },
            ],
            Some(base.revision()),
            None,
        )?;
        let (unarmed_unit, _) = apply(base, &disarm_plan, 200)?;
        let unarmed_assess = assess_tactical_unit(&unarmed_unit, &fixture.anatomy);
        check_eq!(unarmed_assess.state, OperationalState::Unarmed);
        check!(unarmed_assess.can_move);
        check!(!unarmed_assess.can_fire);
        check!(unarmed_assess.is_towable);
        check_eq!(unarmed_assess.destroyed_components.len(), 3);
        check_eq!(unarmed_assess.intact_components.len(), 2);

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
                    section: torso.clone(),
                    condition: ConditionId::new("wreck")?,
                },
            ],
            Some(base.revision()),
            None,
        )?;
        let (wreck_unit, _) = apply(base, &wreck_plan, 300)?;
        let wreck_assess = assess_tactical_unit(&wreck_unit, &fixture.anatomy);
        check_eq!(wreck_assess.state, OperationalState::Wreck);
        check!(wreck_assess.is_towable);
        check_eq!(
            wreck_assess.epoch_disposition,
            FieldEpochDisposition::RemainOpen
        );
        check!(
            wreck_assess
                .intact_components
                .contains(&ComponentId::new("carbine")?)
        );
        check!(
            wreck_assess
                .intact_components
                .contains(&ComponentId::new("needler")?)
        );

        let irrec_plan = UnitMutationPlan::new(
            vec![UnitMutationOp::SetSectionCondition {
                section: torso,
                condition: ConditionId::new("irrecoverable")?,
            }],
            Some(base.revision()),
            None,
        )?;
        let (irrec_unit, _) = apply(base, &irrec_plan, 400)?;
        let irrec_assess = assess_tactical_unit(&irrec_unit, &fixture.anatomy);
        check_eq!(irrec_assess.state, OperationalState::Irrecoverable);
        check!(!irrec_assess.is_towable);
        check_eq!(
            irrec_assess.epoch_disposition,
            FieldEpochDisposition::CloseDestroyed
        );
        check!(
            validate_close_outcome(
                irrec_unit.object(),
                irrec_assess.state,
                &FieldCloseOutcome::Destroyed
            )
            .is_ok()
        );
        check!(
            validate_close_outcome(
                irrec_unit.object(),
                irrec_assess.state,
                &FieldCloseOutcome::RecoveredToBase { base: ObjId(900) }
            )
            .is_err()
        );
        let _ = (right_leg, head);
        Ok(())
    }

    #[test]
    fn h1a_custody_ledger_fences_generations_and_blocks_ooc_rollback() -> TestResult {
        let fixture = build_biped_fixture(ObjId(103))?;
        let unit = &fixture.unit;
        let mut ledger = FieldCustodyLedger::new();

        let rec = ledger.register_unit(
            unit,
            FieldAuthorityKind::Hangar { base: ObjId(500) },
            OperationalState::Operational,
            None,
            1_000,
        )?;
        check_eq!(rec.authority_generation(), 1);

        check_eq!(
            ledger.refuse_ooc_snapshot_restore(unit.object()),
            Err(ConditionError::OocSnapshotRollbackForbidden {
                unit: unit.object(),
                authority_generation: 1,
            })
        );

        let rec = ledger.handoff(AuthorityHandoffRequest {
            unit: unit.object(),
            expected_generation: 1,
            next_authority: FieldAuthorityKind::FieldSurface {
                site: ObjId(600),
                x_cm: 100,
                y_cm: 200,
            },
            operational_state: OperationalState::Operational,
            unit_revision: 0,
            sortie_id: Some(77),
            cargo_mass_kg: 1_200,
            cargo_volume_l: 400,
            at_ic: 1_010,
        })?;
        check_eq!(rec.authority_generation(), 2);

        check!(
            ledger
                .handoff(AuthorityHandoffRequest {
                    unit: unit.object(),
                    expected_generation: 1,
                    next_authority: FieldAuthorityKind::CombatInstance {
                        site: ObjId(600),
                        instance_id: 9,
                    },
                    operational_state: OperationalState::Operational,
                    unit_revision: 0,
                    sortie_id: Some(77),
                    cargo_mass_kg: 1_200,
                    cargo_volume_l: 400,
                    at_ic: 1_020,
                })
                .is_err()
        );

        let rec = ledger.handoff(AuthorityHandoffRequest {
            unit: unit.object(),
            expected_generation: 2,
            next_authority: FieldAuthorityKind::FieldSurface {
                site: ObjId(600),
                x_cm: 150,
                y_cm: 250,
            },
            operational_state: OperationalState::Disabled,
            unit_revision: 3,
            sortie_id: Some(77),
            cargo_mass_kg: 1_200,
            cargo_volume_l: 400,
            at_ic: 1_030,
        })?;
        check_eq!(rec.authority_generation(), 3);
        check_eq!(rec.unit_revision(), 3);

        check_eq!(
            ledger.handoff(AuthorityHandoffRequest {
                unit: unit.object(),
                expected_generation: 3,
                next_authority: FieldAuthorityKind::Hangar { base: ObjId(500) },
                operational_state: OperationalState::Operational,
                unit_revision: 0,
                sortie_id: None,
                cargo_mass_kg: 1_200,
                cargo_volume_l: 400,
                at_ic: 1_040,
            }),
            Err(ConditionError::UnitRevisionRegression {
                unit: unit.object(),
                previous: 3,
                proposed: 0,
            })
        );

        let json = serde_json::to_string(&ledger)?;
        let decoded: FieldCustodyLedger = serde_json::from_str(&json)?;
        check_eq!(decoded, ledger);
        Ok(())
    }

    #[test]
    fn h2a_universal_2x_tonnage_cap_and_speed_scaling() -> TestResult {
        let tug_mass_kg = 20_000;
        let base_speed_cms = 1_200;

        let speed_half =
            scaled_tow_speed_cms(base_speed_cms, tug_mass_kg, 10_000, TowRigProfile::Standard)?;
        check_eq!(speed_half, 840);

        let speed_equal =
            scaled_tow_speed_cms(base_speed_cms, tug_mass_kg, 20_000, TowRigProfile::Standard)?;
        check_eq!(speed_equal, 660);

        let speed_double =
            scaled_tow_speed_cms(base_speed_cms, tug_mass_kg, 40_000, TowRigProfile::Standard)?;
        check_eq!(speed_double, 300);

        check!(speed_half > speed_equal);
        check!(speed_equal > speed_double);

        let opt_double = scaled_tow_speed_cms(
            base_speed_cms,
            tug_mass_kg,
            40_000,
            TowRigProfile::RecoveryOptimized,
        )?;
        check_eq!(opt_double, 600);
        check!(opt_double > speed_double);

        check_eq!(
            scaled_tow_speed_cms(base_speed_cms, tug_mass_kg, 40_001, TowRigProfile::Standard,),
            Err(RecoveryError::ExceedsTonnageCap {
                tug_mass_kg: 20_000,
                casualty_mass_kg: 40_001,
                max_casualty_mass_kg: 40_000,
            })
        );
        check_eq!(
            scaled_tow_speed_cms(
                base_speed_cms,
                tug_mass_kg,
                40_001,
                TowRigProfile::RecoveryOptimized,
            ),
            Err(RecoveryError::ExceedsTonnageCap {
                tug_mass_kg: 20_000,
                casualty_mass_kg: 40_001,
                max_casualty_mass_kg: 40_000,
            })
        );
        Ok(())
    }

    #[test]
    fn h2a_exclusive_attachment_coupled_movement_and_unhealed_bay_load() -> TestResult {
        let fixture = build_biped_fixture(ObjId(202))?;
        let head = SectionId::new("head")?;
        let internals = CellId::new("internals")?;

        let kill_head = UnitMutationPlan::new(
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
            Some(fixture.unit.revision()),
            None,
        )?;
        let (casualty_unit, _) = apply(&fixture.unit, &kill_head, 500)?;
        check_eq!(casualty_unit.revision(), 1);
        let casualty_assessment = assess_tactical_unit(&casualty_unit, &fixture.anatomy);
        check_eq!(casualty_assessment.state, OperationalState::Disabled);

        let casualty_mass = UnitMassProfile::new(35_000, 3_000, 1_200, 320, 480)?;
        check_eq!(casualty_mass.total_mass_kg(), 38_000);

        let site = ObjId(700);
        let casualty_pose = FieldPose {
            site,
            x_cm: 1_000,
            y_cm: 1_000,
        };
        let tug_a = ObjId(201);
        let tug_b = ObjId(203);
        let tug_pose = FieldPose {
            site,
            x_cm: 1_300,
            y_cm: 1_400,
        };

        let mut book = RecoveryBook::new();

        let report = inspect_field_unit(
            casualty_assessment.clone(),
            casualty_mass,
            casualty_pose,
            Some(TugInspectionContext {
                tug: tug_a,
                tug_state: OperationalState::Operational,
                tug_can_move: true,
                tug_base_speed_cms: 1_000,
                tug_mass_kg: 20_000,
                rig_profile: TowRigProfile::Standard,
                tug_pose,
                max_attach_distance_cm: DEFAULT_MAX_ATTACH_DISTANCE_CM,
            }),
            Some(&book),
        );
        let eval = report
            .tow_evaluation
            .as_ref()
            .ok_or("expected tow evaluation")?;
        check!(eval.eligible);
        check_eq!(eval.mass_ratio_permille, Some(1_900));
        check_eq!(eval.coupled_speed_cms, Some(280));

        let op = book.attach(AttachTowRequest {
            tug: tug_a,
            tug_state: OperationalState::Operational,
            tug_can_move: true,
            tug_base_speed_cms: 1_000,
            tug_mass_kg: 20_000,
            rig_profile: TowRigProfile::Standard,
            tug_pose,
            casualty_assessment: casualty_assessment.clone(),
            casualty_mass,
            casualty_pose,
            max_attach_distance_cm: DEFAULT_MAX_ATTACH_DISTANCE_CM,
            at_ic: 1_000,
        })?;
        let op_id = op.id();
        check_eq!(op.coupled_speed_cms(), 280);

        let competing = book.attach(AttachTowRequest {
            tug: tug_b,
            tug_state: OperationalState::Operational,
            tug_can_move: true,
            tug_base_speed_cms: 1_000,
            tug_mass_kg: 25_000,
            rig_profile: TowRigProfile::Standard,
            tug_pose,
            casualty_assessment,
            casualty_mass,
            casualty_pose,
            max_attach_distance_cm: DEFAULT_MAX_ATTACH_DISTANCE_CM,
            at_ic: 1_001,
        });
        check_eq!(
            competing,
            Err(RecoveryError::CasualtyAlreadyAttached {
                casualty: ObjId(202),
                existing_op: op_id,
                controlling_tug: tug_a,
            })
        );

        let carrier_pose = FieldPose {
            site,
            x_cm: 3_300,
            y_cm: 1_400,
        };
        let op = book.advance_tow(op_id, 1, carrier_pose, 10, 1_010)?;
        check_eq!(op.revision(), 2);
        check_eq!(op.stage().distance_towed_cm(), 2_000);

        check!(
            book.advance_tow(
                op_id,
                2,
                FieldPose {
                    site,
                    x_cm: 4_300,
                    y_cm: 1_400,
                },
                1,
                1_011,
            )
            .is_err()
        );

        let small_bay = BayCapacitySpec {
            max_units: 2,
            current_units: 0,
            max_mass_kg: 36_000,
            current_mass_kg: 0,
            max_volume_l: 5_000,
            current_volume_l: 0,
            max_width_cm: 400,
            max_height_cm: 500,
        };
        check_eq!(
            book.load_into_bay(LoadIntoBayRequest {
                id: op_id,
                expected_revision: 2,
                carrier: ObjId(800),
                slot: SmolStr::new("recovery_bay_1"),
                carrier_pose,
                max_load_distance_cm: DEFAULT_MAX_ATTACH_DISTANCE_CM,
                bay_capacity: &small_bay,
                casualty_unit: &casualty_unit,
                at_ic: 1_020,
            }),
            Err(RecoveryError::BayMassExceeded {
                required_mass_kg: 38_000,
                max_mass_kg: 36_000,
            })
        );

        let large_bay = BayCapacitySpec {
            max_mass_kg: 50_000,
            ..small_bay
        };
        let loaded = book.load_into_bay(LoadIntoBayRequest {
            id: op_id,
            expected_revision: 2,
            carrier: ObjId(800),
            slot: SmolStr::new("recovery_bay_1"),
            carrier_pose,
            max_load_distance_cm: DEFAULT_MAX_ATTACH_DISTANCE_CM,
            bay_capacity: &large_bay,
            casualty_unit: &casualty_unit,
            at_ic: 1_020,
        })?;
        check_eq!(loaded.preserved_state, OperationalState::Disabled);
        check_eq!(loaded.preserved_unit_revision, 1);
        check_eq!(loaded.preserved_cargo_mass_kg, 3_000);
        check_eq!(loaded.preserved_cargo_volume_l, 1_200);
        check_eq!(
            loaded.close_outcome,
            FieldCloseOutcome::RecoveredToBase { base: ObjId(800) }
        );

        let encoded = serde_json::to_string(&book)?;
        let decoded: RecoveryBook = serde_json::from_str(&encoded)?;
        check_eq!(decoded, book);
        Ok(())
    }

    #[test]
    fn w_clock_events_quarantine_mismatched_units_and_extend_salvage_claim_deadlines() -> TestResult
    {
        let mut quarantine = OutageQuarantineBook::new();
        let mut salvage_book = SalvageClaimBook::new();
        let mut custody_ledger = FieldCustodyLedger::new();
        let mut site_book = FieldSiteEngagementBook::new(ObjId(900))?;

        let fixture = build_biped_fixture(ObjId(201))?;
        let mut fixture_mismatch = build_biped_fixture(ObjId(202))?;
        let pose_201 = FieldPose {
            site: ObjId(900),
            x_cm: 100,
            y_cm: 100,
        };
        let pose_202 = FieldPose {
            site: ObjId(900),
            x_cm: 200,
            y_cm: 100,
        };
        site_book.deploy_surface_unit(
            &mut custody_ledger,
            &fixture.unit,
            &fixture.anatomy,
            SmolStr::new("expedition"),
            pose_201,
            None,
            1_000,
        )?;
        site_book.deploy_surface_unit(
            &mut custody_ledger,
            &fixture_mismatch.unit,
            &fixture_mismatch.anatomy,
            SmolStr::new("expedition"),
            pose_202,
            None,
            1_000,
        )?;

        // File a pending salvage claim before the outage (`Q-F3`).
        let authority = RecoveryAuthorityContext {
            recoverer: ObjId(50),
            unit_title_holder: ObjId(60),
            has_owner_authorization: false,
            has_landholder_authorization: false,
            ground_tenure_at_attach: GroundTenure::Unclaimed,
        };
        let op_id = RecoveryOpId::new(1)?;
        salvage_book.record_physical_attach(op_id, ObjId(201), ObjId(300), &authority, 50_000)?;
        salvage_book.record_bay_load(op_id, ObjId(800), 50_010)?;
        let custody_status = salvage_book.resolve_hull_recovery_custody(op_id, 10_020)?;
        let (claim_id, initial_deadline) = match custody_status {
            CustodyTitleStatus::PendingSalvageClaim {
                claim_id,
                deadline_real_s,
                ..
            } => (claim_id, deadline_real_s),
            other => return Err(format!("expected pending salvage claim, got {other:?}").into()),
        };

        // The kernel froze the world at its pre-outage checkpoint (real 10_020, ic 50_000) and
        // the boot re-emits `WorldClockFrozen`: the contest window pauses there (`Q-F3`).
        let frozen = WorldEvent::WorldClockFrozen {
            real_unix: 10_020,
            ic_unix: 50_000,
        };
        check!(salvage_book.apply_world_clock_event(&frozen).is_empty());
        check_eq!(
            salvage_book.claim(claim_id).map(|c| c.paused_at_real_s),
            Some(Some(10_020))
        );
        check!(matches!(
            salvage_book.adjudicate_claim(claim_id, initial_deadline + 1),
            Err(SalvageError::ClaimPausedForOutage(id)) if id == claim_id
        ));
        // A freeze audits nothing: quarantine is decided when the world comes back (`Q-H11`).
        check!(
            quarantine
                .audit_site_units_on_world_clock_resumed(
                    &frozen,
                    &site_book,
                    &custody_ledger,
                    [&fixture.unit, &fixture_mismatch.unit],
                )?
                .is_empty()
        );

        // Mutate fixture_mismatch's unit revision outside site_book/custody_ledger to simulate an
        // incompatible post-outage persisted revision (`Q-H11`).
        let bump_plan = UnitMutationPlan::new(
            vec![UnitMutationOp::SetCell {
                addr: CellAddr::Section {
                    section: SectionId::new("head")?,
                    cell: CellId::new("internals")?,
                },
                value: 7,
            }],
            Some(fixture_mismatch.unit.revision()),
            None,
        )?;
        let (bumped_202, _) = apply(&fixture_mismatch.unit, &bump_plan, 50_000)?;
        fixture_mismatch.unit = bumped_202;

        // The world task resumes the clock 900 s later (a 600 s outage plus the 300 s countdown)
        // and emits `WorldClockResumed` with the frozen IC second as the resumed one.
        let resumed = WorldEvent::WorldClockResumed {
            frozen_real_unix: 10_020,
            resumed_real_unix: 10_920,
            unavailable_secs: 900,
            ic_unix: 50_000,
        };

        // Audit on resume: ObjId(201) is clean; ObjId(202) has a revision mismatch and is
        // quarantined, stamped from the event (`Q-H11`, `P-4`).
        let quarantined = quarantine.audit_site_units_on_world_clock_resumed(
            &resumed,
            &site_book,
            &custody_ledger,
            [&fixture.unit, &fixture_mismatch.unit],
        )?;
        check_eq!(quarantined.len(), 1);
        let q_rec = quarantined
            .first()
            .ok_or("expected ObjId(202) to be quarantined")?;
        check_eq!(q_rec.unit, ObjId(202));
        check_eq!(q_rec.site, ObjId(900));
        check_eq!(q_rec.reason, SmolStr::new("custody_revision_mismatch"));
        check_eq!(q_rec.preserved_revision, fixture_mismatch.unit.revision());
        check_eq!(q_rec.quarantined_at_real_s, 10_920);
        check_eq!(q_rec.quarantined_at_ic_s, 50_000);
        check_eq!(quarantine.ensure_unit_not_quarantined(ObjId(201)), Ok(()));
        check!(matches!(
            quarantine.ensure_unit_not_quarantined(ObjId(202)),
            Err(QuarantineError::UnitQuarantined {
                unit: ObjId(202),
                ..
            })
        ));
        check_eq!(quarantine.quarantined_units().len(), 1);

        // The same event extends the paused contest window by the 900 s the world was away.
        let adjusted = salvage_book.apply_world_clock_event(&resumed);
        check_eq!(adjusted.len(), 1);
        let adj = adjusted.first().ok_or("expected one adjusted claim")?;
        check_eq!(adj.claim_id, claim_id);
        check_eq!(adj.previous_deadline_real_s, initial_deadline);
        check_eq!(adj.adjusted_deadline_real_s, initial_deadline + 900);
        check_eq!(adj.total_paused_real_s, 900);
        check_eq!(
            salvage_book.claim(claim_id).map(|c| c.paused_at_real_s),
            Some(None)
        );

        // A claim the freeze event never reached is still extended by the unavailable time: the
        // resume event pauses it at `frozen_real_unix` first (the economy's rule for a window it
        // finds open at the freeze).
        let op_2 = RecoveryOpId::new(2)?;
        salvage_book.record_physical_attach(op_2, ObjId(202), ObjId(300), &authority, 50_000)?;
        salvage_book.record_bay_load(op_2, ObjId(800), 50_010)?;
        let (claim_2, deadline_2) = match salvage_book
            .resolve_hull_recovery_custody(op_2, 10_920)?
        {
            CustodyTitleStatus::PendingSalvageClaim {
                claim_id,
                deadline_real_s,
                ..
            } => (claim_id, deadline_real_s),
            other => return Err(format!("expected pending salvage claim, got {other:?}").into()),
        };
        let resumed_again = WorldEvent::WorldClockResumed {
            frozen_real_unix: 11_000,
            resumed_real_unix: 11_300,
            unavailable_secs: 300,
            ic_unix: 50_000,
        };
        let adjusted_again = salvage_book.apply_world_clock_event(&resumed_again);
        check_eq!(adjusted_again.len(), 2);
        check_eq!(
            salvage_book.claim(claim_id).map(|c| c.deadline_real_s),
            Some(initial_deadline + 900 + 300)
        );
        check_eq!(
            salvage_book.claim(claim_2).map(|c| c.deadline_real_s),
            Some(deadline_2 + 300)
        );

        // Quarantine holds until staff releases it (`Q-H11`).
        check!(quarantine.release_quarantined_unit(ObjId(202)).is_some());
        quarantine.ensure_unit_not_quarantined(ObjId(202))?;

        let encoded = serde_json::to_string(&quarantine)?;
        let decoded: OutageQuarantineBook = serde_json::from_str(&encoded)?;
        check_eq!(decoded, quarantine);
        Ok(())
    }

    #[test]
    fn live_coupled_surface_tow_motion_and_mid_tow_combat_interdiction() -> TestResult {
        let site = ObjId(910);
        let tug_id = ObjId(301);
        let cas_id = ObjId(302);
        let attacker_id = ObjId(303);

        let tug_fixture = build_biped_fixture(tug_id)?;
        let mut cas_fixture = build_biped_fixture(cas_id)?;
        let attacker_fixture = build_biped_fixture(attacker_id)?;

        // Immobilize cas_fixture (destroy both legs).
        let internals = CellId::new("internals")?;
        let destroyed = ConditionId::new("destroyed")?;
        let left_leg = SectionId::new("left_leg")?;
        let right_leg = SectionId::new("right_leg")?;
        let plan = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: left_leg.clone(),
                        cell: internals.clone(),
                    },
                    value: 0,
                },
                UnitMutationOp::SetSectionCondition {
                    section: left_leg.clone(),
                    condition: destroyed.clone(),
                },
                UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: right_leg.clone(),
                        cell: internals.clone(),
                    },
                    value: 0,
                },
                UnitMutationOp::SetSectionCondition {
                    section: right_leg.clone(),
                    condition: destroyed.clone(),
                },
            ],
            Some(cas_fixture.unit.revision()),
            None,
        )?;
        let (immobile_cas, _) = apply(&cas_fixture.unit, &plan, 1_000)?;
        cas_fixture.unit = immobile_cas;
        let cas_assessment = assess_tactical_unit(&cas_fixture.unit, &cas_fixture.anatomy);
        check_eq!(cas_assessment.state, OperationalState::Immobile);

        let mut ledger = FieldCustodyLedger::new();
        let mut site_book = FieldSiteEngagementBook::new(site)?;
        let mut recovery_book = RecoveryBook::new();
        let initial_pose = FieldPose {
            site,
            x_cm: 500,
            y_cm: 500,
        };
        site_book.deploy_surface_unit(
            &mut ledger,
            &tug_fixture.unit,
            &tug_fixture.anatomy,
            SmolStr::new("expedition"),
            initial_pose,
            None,
            1_000,
        )?;
        site_book.deploy_surface_unit(
            &mut ledger,
            &cas_fixture.unit,
            &cas_fixture.anatomy,
            SmolStr::new("expedition"),
            initial_pose,
            None,
            1_000,
        )?;
        site_book.deploy_surface_unit(
            &mut ledger,
            &attacker_fixture.unit,
            &attacker_fixture.anatomy,
            SmolStr::new("raiders"),
            FieldPose {
                site,
                x_cm: 1_200,
                y_cm: 500,
            },
            None,
            1_000,
        )?;

        let op_id = site_book.attach_site_tow(
            &mut recovery_book,
            AttachTowRequest {
                tug: tug_id,
                tug_state: OperationalState::Operational,
                tug_can_move: true,
                tug_base_speed_cms: 1_000,
                tug_mass_kg: 20_000,
                rig_profile: TowRigProfile::Standard,
                tug_pose: initial_pose,
                casualty_assessment: cas_assessment,
                casualty_mass: UnitMassProfile::new(20_000, 0, 0, 300, 400)?,
                casualty_pose: initial_pose,
                max_attach_distance_cm: DEFAULT_MAX_ATTACH_DISTANCE_CM,
                at_ic: 1_005,
            },
        )?;

        // Independent surface motion for either coupled unit is refused (`Q-S7`).
        let next_pose = FieldPose {
            site,
            x_cm: 900,
            y_cm: 500,
        };
        check_eq!(
            site_book.record_surface_motion(tug_id, next_pose, None),
            Err(EngagementError::UnitCoupledToActiveTow {
                unit: tug_id,
                op_id,
            })
        );
        check_eq!(
            site_book.record_surface_motion(cas_id, next_pose, None),
            Err(EngagementError::UnitCoupledToActiveTow {
                unit: cas_id,
                op_id,
            })
        );

        // Coupled tow motion updates both tug and casualty atomically (`Q-S7`).
        site_book.step_coupled_tow_motion(
            &mut ledger,
            &mut recovery_book,
            op_id,
            1,
            next_pose,
            5,
            1_010,
        )?;
        check_eq!(
            site_book.unit_presence(tug_id).ok_or("missing tug")?.pose,
            next_pose
        );
        check_eq!(
            site_book.unit_presence(cas_id).ok_or("missing cas")?.pose,
            next_pose
        );

        // Hostile interdiction mid-tow promotes the site to active combat at the coupled pose (`H1-B`, `H2-A`).
        let handoffs = site_book.interdict_active_tow(
            &mut ledger,
            &recovery_book,
            op_id,
            attacker_id,
            42,
            1,
            1_012,
        )?;
        check!(site_book.is_engaged());
        let tug_handoff = handoffs
            .iter()
            .find(|h| h.unit == tug_id)
            .ok_or("missing tug handoff")?;
        check_eq!(tug_handoff.handoff_pose, next_pose);
        check_eq!(tug_handoff.active_recovery_op, Some(op_id));

        // Emergency drop detaches the casualty at its current pose so the tug can maneuver at full speed.
        let drop_pose = site_book.emergency_drop_tow(&mut recovery_book, op_id, 2, 1_013)?;
        check_eq!(drop_pose, next_pose);
        check_eq!(recovery_book.active_operation_for_tug(tug_id), None);

        // Settle and demote combat, re-attach tow, promote to combat, and verify auto-detach when tug is immobilized (`H2-A`, `Q-H8`).
        site_book.set_settlement_gate(DemotionSettlementGate::default());
        site_book.demote_engagement(&mut ledger, 1_400)?;
        let cas_assessment2 = assess_tactical_unit(&cas_fixture.unit, &cas_fixture.anatomy);
        let op2_id = site_book.attach_site_tow(
            &mut recovery_book,
            AttachTowRequest {
                tug: tug_id,
                tug_state: OperationalState::Operational,
                tug_can_move: true,
                tug_base_speed_cms: 1_000,
                tug_mass_kg: 20_000,
                rig_profile: TowRigProfile::Standard,
                tug_pose: next_pose,
                casualty_assessment: cas_assessment2,
                casualty_mass: UnitMassProfile::new(20_000, 0, 0, 300, 400)?,
                casualty_pose: next_pose,
                max_attach_distance_cm: DEFAULT_MAX_ATTACH_DISTANCE_CM,
                at_ic: 1_405,
            },
        )?;
        site_book.promote_to_engagement(
            &mut ledger,
            43,
            FieldPromotionTrigger::ExplicitHostileAction {
                attacker: attacker_id,
                target: tug_id,
            },
            1,
            1_406,
        )?;
        // Destroy tug's legs while towing: apply_field_damage_with_recovery_sync auto-detaches the tow!
        let tug_leg_plan = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: left_leg.clone(),
                        cell: internals.clone(),
                    },
                    value: 0,
                },
                UnitMutationOp::SetSectionCondition {
                    section: left_leg,
                    condition: destroyed.clone(),
                },
                UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: right_leg.clone(),
                        cell: internals,
                    },
                    value: 0,
                },
                UnitMutationOp::SetSectionCondition {
                    section: right_leg,
                    condition: destroyed,
                },
            ],
            Some(tug_fixture.unit.revision()),
            None,
        )?;
        let (_, _, auto_detached) = site_book.apply_field_damage_with_recovery_sync(
            &mut ledger,
            &mut recovery_book,
            &tug_fixture.unit,
            &tug_leg_plan,
            &tug_fixture.anatomy,
            1_407,
        )?;
        check_eq!(auto_detached, Some(op2_id));
        check_eq!(recovery_book.active_operation_for_tug(tug_id), None);

        Ok(())
    }

    #[test]
    fn black_box_forensics_and_contraband_component_provenance() -> TestResult {
        let hull_id = ObjId(401);
        let tug_id = ObjId(402);
        let owner_id = ObjId(40);
        let thief_id = ObjId(41);
        let base_id = ObjId(850);
        let op_id = RecoveryOpId::new(77)?;

        let fixture = build_biped_fixture(hull_id)?;
        let mut recipient_fixture = build_biped_fixture(ObjId(403))?;
        let mut salvage_book = SalvageClaimBook::new();
        let mut workshop_book = WorkshopBook::new();

        // Append combat damage and unlawful tow attach on owner's ground (`FO-4`, `Q-S4`, `P-3`).
        salvage_book.append_black_box_entry(
            hull_id,
            BlackBoxFlightEntry::EpochOpened {
                sortie_id: SmolStr::new("sortie.920"),
                host_site: SmolStr::new("site.920"),
                operator: Some(owner_id),
                mind_label: SmolStr::new("Standard Doctrine"),
                governed: true,
                at_ic: 2_000,
            },
        )?;
        salvage_book.append_black_box_entry(
            hull_id,
            BlackBoxFlightEntry::CombatDamageRecorded {
                attacker: Some(thief_id),
                section: SectionId::new("head")?,
                resulting_state: OperationalState::Disabled,
                at_ic: 2_010,
            },
        )?;
        let unlawful_auth_ctx = RecoveryAuthorityContext {
            recoverer: thief_id,
            unit_title_holder: owner_id,
            has_owner_authorization: false,
            has_landholder_authorization: false,
            ground_tenure_at_attach: GroundTenure::Owned {
                landholder: owner_id,
                claim_id: Some(SmolStr::new("deed.alpha")),
            },
        };
        salvage_book.record_physical_attach(op_id, hull_id, tug_id, &unlawful_auth_ctx, 2_020)?;

        // Thief performs an unlawful hot field strip of `carbine` on owner's ground (`FO-4`, `Q-F6`).
        let carbine = ComponentId::new("carbine")?;
        let (_, _, stripped) = workshop_book.strip_component(
            &fixture.unit,
            &fixture.anatomy,
            &carbine,
            thief_id,
            &StripAuthorization::UnlawfulFieldHotStrip {
                authority_ctx: unlawful_auth_ctx,
            },
            Some(&mut salvage_book),
            2_025,
        )?;
        let stripped_rec_id = stripped.record_id;
        check_eq!(
            stripped.provenance.legality,
            ProvenanceLegalityStatus::ContrabandHotPart {
                rightful_owner: owner_id,
                reason: ContrabandReason::UnlawfulFieldStripOnOwnedGround,
            }
        );

        // Before bay load closes the epoch, black-box forensic extraction is refused (`P-3`, `Q-H9`).
        check_eq!(
            salvage_book.extract_black_box_forensics(op_id, owner_id, true, 2_030),
            Err(SalvageError::CasualtyNotYetLoadedInBay(hull_id))
        );

        // Close epoch in bay and extract black-box forensics (`P-3`, `Q-S4`).
        salvage_book.record_bay_load(op_id, base_id, 2_040)?;
        let forensics = salvage_book.extract_black_box_forensics(op_id, owner_id, true, 2_050)?;
        check!(forensics.reveals_unlawful_recovery);
        check_eq!(
            forensics.unlawful_stripped_components,
            vec![carbine.clone()]
        );
        check_eq!(
            forensics.minted_evidence.kind,
            SalvageEvidenceKind::RecoveredBlackBox {
                hull: hull_id,
                read_at_ic: 2_050,
            }
        );

        // Destroy `carbine` on recipient_fixture so we can attempt to install the stripped part.
        let destroyed = ConditionId::new("destroyed")?;
        let cond_cell = CellId::new("condition")?;
        let destroy_plan = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetCell {
                    addr: CellAddr::Component {
                        component: carbine.clone(),
                        cell: cond_cell,
                    },
                    value: 0,
                },
                UnitMutationOp::SetComponentCondition {
                    component: carbine.clone(),
                    condition: destroyed,
                },
            ],
            Some(recipient_fixture.unit.revision()),
            None,
        )?;
        let (damaged_recipient, _) = apply(&recipient_fixture.unit, &destroy_plan, 2_055)?;
        recipient_fixture.unit = damaged_recipient;

        // Lawful workshop refuses to install the contraband hot part (`FO-4`, `Q-F6`).
        let replacement_spec = ReplacementPartSpec::from_stripped_record(&stripped);
        let install_res = workshop_book.replace_component_in_slot(
            &recipient_fixture.unit,
            &recipient_fixture.anatomy,
            &carbine,
            &replacement_spec,
            thief_id,
            &CustodyTitleStatus::ResolvedTitle { owner: thief_id },
            false,
            600,
            120,
            Some(&mut salvage_book),
            2_060,
        );
        check_eq!(
            install_res,
            Err(WorkshopError::ContrabandComponentRefused {
                record_id: stripped_rec_id,
                parent_hull: hull_id,
                rightful_owner: owner_id,
                reason: ContrabandReason::UnlawfulFieldStripOnOwnedGround,
            })
        );

        // Rightful owner clears contraband provenance after recovery, allowing lawful installation.
        workshop_book.clear_contraband_provenance(stripped_rec_id, owner_id)?;
        workshop_book.replace_component_in_slot(
            &recipient_fixture.unit,
            &recipient_fixture.anatomy,
            &carbine,
            &replacement_spec,
            owner_id,
            &CustodyTitleStatus::ResolvedTitle { owner: owner_id },
            true,
            600,
            120,
            Some(&mut salvage_book),
            2_070,
        )?;

        Ok(())
    }

    #[test]
    fn field_engineering_tender_unjam_stabilize_resupply_and_workshop_boundary() -> TestResult {
        let site = ObjId(930);
        let tender_id = ObjId(501);
        let target_id = ObjId(502);
        let organizer = ObjId(70);
        let ammo_sponsor = ObjId(71);

        let mut fixture = build_biped_fixture(target_id)?;
        let pose = FieldPose {
            site,
            x_cm: 200,
            y_cm: 200,
        };

        let mut eng_book = FieldEngineeringBook::new();
        eng_book.register_tender(FieldTenderState {
            tender_unit: tender_id,
            site_id: site,
            pose,
            operational_state: OperationalState::Operational,
            spare_parts_kg: 500,
            ammo_supply_rounds: 100,
            charge_reserve_cu: 200,
            max_service_distance_cm: DEFAULT_MAX_TENDER_SERVICE_DISTANCE_CM,
        })?;

        // 1. Jam `carbine`, damage `left_arm`, deplete `needler` ammo, and destroy `left_leg`.
        let carbine = ComponentId::new("carbine")?;
        let needler = ComponentId::new("needler")?;
        let left_arm = SectionId::new("left_arm")?;
        let left_leg = SectionId::new("left_leg")?;
        let internals = CellId::new("internals")?;
        let ammo_cell = CellId::new("ammo")?;

        let plan = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetComponentCondition {
                    component: carbine.clone(),
                    condition: ConditionId::new("jammed")?,
                },
                UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: left_arm.clone(),
                        cell: internals.clone(),
                    },
                    value: 6,
                },
                UnitMutationOp::SetSectionCondition {
                    section: left_arm.clone(),
                    condition: ConditionId::new("damaged")?,
                },
                UnitMutationOp::SetCell {
                    addr: CellAddr::Component {
                        component: needler.clone(),
                        cell: ammo_cell.clone(),
                    },
                    value: 2,
                },
                UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: left_leg.clone(),
                        cell: internals,
                    },
                    value: 0,
                },
                UnitMutationOp::SetSectionCondition {
                    section: left_leg.clone(),
                    condition: ConditionId::new("destroyed")?,
                },
            ],
            Some(fixture.unit.revision()),
            None,
        )?;
        let (damaged_unit, _) = apply(&fixture.unit, &plan, 3_000)?;
        fixture.unit = damaged_unit;

        // Field tender unjams `carbine` in the field.
        let unjam_entry = eng_book.field_unjam_component(
            tender_id,
            &mut fixture.unit,
            pose,
            &fixture.anatomy,
            &carbine,
            None,
            DEFAULT_UNJAM_SPARE_PARTS_KG,
            false,
            false,
            30,
            3_030,
        )?;
        check_eq!(
            unjam_entry.action,
            FieldEngineeringActionKind::UnjamComponent {
                component_id: carbine,
                restored_condition: ComponentCondition::Intact,
                spare_parts_consumed_kg: DEFAULT_UNJAM_SPARE_PARTS_KG,
            }
        );

        // Field tender stabilizes damaged `left_arm` (+6 structural points).
        let patch_entry = eng_book.field_stabilize_damaged_section(
            tender_id,
            &mut fixture.unit,
            pose,
            &fixture.anatomy,
            &left_arm,
            6,
            DEFAULT_SPARE_PARTS_KG_PER_STRUCTURAL_POINT,
            false,
            false,
            60,
            3_090,
        )?;
        check_eq!(
            patch_entry.action,
            FieldEngineeringActionKind::StabilizeDamagedSection {
                section_id: left_arm,
                points_restored: 6,
                spare_parts_consumed_kg: 150,
            }
        );

        // Field tender refuses to reconstruct destroyed `left_leg` (`H3` workshop boundary).
        let destroyed_res = eng_book.field_stabilize_damaged_section(
            tender_id,
            &mut fixture.unit,
            pose,
            &fixture.anatomy,
            &left_leg,
            12,
            DEFAULT_SPARE_PARTS_KG_PER_STRUCTURAL_POINT,
            false,
            false,
            60,
            3_150,
        );
        check_eq!(
            destroyed_res,
            Err(FieldEngineeringError::RequiresWorkshopFacility {
                unit: target_id,
                reason: SmolStr::new_static(
                    "destroyed section cannot be reconstructed in the field; workshop facility required",
                ),
            })
        );

        // Field tender rearms `needler` and transfers 40 CU charge, billing `ammo_sponsor` via `UnitExpensePolicy`.
        let policy = UnitExpensePolicy {
            ammo_payer: ammo_sponsor,
            recovery_payer: organizer,
            repair_payer: organizer,
        };
        let resupply_entry = eng_book.field_resupply_ammo_and_charge(
            tender_id,
            &mut fixture.unit,
            pose,
            &fixture.anatomy,
            Some((&needler, &ammo_cell, 20)),
            40,
            5,
            2,
            Some(&policy),
            false,
            false,
            20,
            3_170,
        )?;
        // `needler` had 2/30 ammo so 20 rounds transferred (20 * 5 = 100) + 40 CU * 2 = 80 => 180 credits.
        check_eq!(
            resupply_entry.action,
            FieldEngineeringActionKind::FieldResupply {
                component_id: Some(needler),
                rounds_transferred: 20,
                charge_transferred_cu: 40,
                billed_credits: 180,
                billed_payer: Some(ammo_sponsor),
            }
        );

        let tender_after = eng_book.tender(tender_id).ok_or("expected tender")?;
        check_eq!(tender_after.spare_parts_kg, 500 - 10 - 150);
        check_eq!(tender_after.ammo_supply_rounds, 80);
        check_eq!(tender_after.charge_reserve_cu, 160);

        Ok(())
    }
}

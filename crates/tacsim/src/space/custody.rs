//! **Kernel Custody, `TacticalUnit` Damage Persistence & `doc 27a` Salvage Registry Bridge**
//! (Doc 35 cut `SF.10`; `docs/design/35-ship-free-flight-combat-and-mining.md` `D-35-1`,
//! `D-35-12`, `D-35-13`, `D-35-14`).
//!
//! Bridges the Tier-1/Tier-2/Tier-3 space flight, combat, and 6-DOF docking simulation
//! directly into `mush-tactics` and `tacsim::field_ops`:
//! 1. **Bidirectional `TacticalUnit` $\leftrightarrow$ `SpaceShipState` custody (`D-35-1`)**:
//!    - [`space_ship_from_tactical_unit`] reads a bound [`TacticalUnit`]'s live `hull`,
//!      `hold`, and `drive` section `internals` (`CellAddr::Section`) and `front`, `left`,
//!      `right`, `rear`, and `top` facet `armor` (`CellAddr::Facet`) into a [`SpaceShipState`].
//!    - [`build_space_damage_mutation_plan`] translates post-combat and post-docking
//!      [`SpaceShipState`] section/facet cells back into an IC-custody-legal
//!      [`UnitMutationPlan`] and [`FieldUnitAssessment`] (`Operational`, `Immobile`,
//!      `Disabled`, `Wreck`) via [`crate::field_ops::apply_ic_condition_handoff`].
//! 2. **Tamper-Evident Black-Box Forensics (`FO-4`, `Q-H6`, `Q-S4`)**:
//!    - [`record_space_combat_to_black_box`] records [`SpaceCombatEvent`] hits (`TorpedoHit`,
//!      `LaserHit`, `RailgunHit`) and open-epoch disablement (`ShipDisabledDerelict`) as
//!      [`BlackBoxFlightEntry::CombatDamageRecorded`] entries inside [`SalvageClaimBook`].
//! 3. **48-Hour Registry Salvage Claims (`doc 27a`, `D-35-12`, `D-35-14`)**:
//!    - [`register_derelict_salvage_in_book`] records a clamped derelict [`SalvageTowRecord`]
//!      in [`SalvageClaimBook`] (`record_physical_attach`, `record_bay_load`, and
//!      `resolve_hull_recovery_custody`), opening the 48-hour (`172,800 s`) Registry contest
//!      window when recovered by a non-owner or resolving title immediately on self-recovery.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, ObjId};
use mush_tactics::{
    CellAddr, CellId, ConditionId, FacetId, SectionId, TacticalUnit, UnitError, UnitMutationOp,
    UnitMutationPlan, apply,
};

use crate::definitions::Catalog;
use crate::definitions::unit::{ARMOR_CELL, INTERNALS_CELL, UnitSpec};
use crate::field_ops::{
    BlackBoxFlightEntry, CustodyTitleStatus, FieldOpsError, FieldUnitAssessment, GroundTenure,
    OperationalState, RecoveryAuthorityContext, RecoveryOpId, SalvageClaimBook,
    apply_ic_condition_handoff,
};
use crate::port::ship::SHIP_DAMAGE;
use crate::space::bubble::{
    DERELICT_WINDOW_S, FacetBand, HullStatus, ShipSectionId, ShipSectionState, SpaceCombatEvent,
    SpacePartyId, SpaceShipState,
};
use crate::space::docking::SalvageTowRecord;

/// All canonical [`FacetBand`]s on a spacecraft hull (`dray.json`, `D-35-1`).
pub const ALL_FACET_BANDS: [FacetBand; 5] = [
    FacetBand::Front,
    FacetBand::Left,
    FacetBand::Right,
    FacetBand::Rear,
    FacetBand::Top,
];

/// All canonical [`ShipSectionId`]s on a spacecraft hull (`dray.json`, `D-35-1`).
pub const ALL_SHIP_SECTIONS: [ShipSectionId; 3] = [
    ShipSectionId::Hull,
    ShipSectionId::Hold,
    ShipSectionId::Drive,
];

/// Instantiate a stock spacecraft [`TacticalUnit`] and its authored [`UnitSpec`] from
/// the compiled-in fielded catalog (e.g. `"tacsim.unit.dray"` or `"tacsim.unit.dray_p"`).
///
/// # Errors
/// Returns [`FieldOpsError`] if the catalog or unit entry cannot be bound.
pub fn instantiate_stock_space_unit(
    obj: ObjId,
    unit_entry_id: &str,
) -> Result<(TacticalUnit, UnitSpec), FieldOpsError> {
    let catalog = Catalog::build_fielded(&[], "tacsim")
        .map_err(|_| FieldOpsError::Unit(UnitError::EmptyId))?;
    let key =
        CanonicalId::new(unit_entry_id).map_err(|_| FieldOpsError::Unit(UnitError::EmptyId))?;
    let spec = catalog
        .specs
        .get(&key)
        .ok_or(FieldOpsError::Unit(UnitError::EmptyId))?
        .clone();
    let binding = catalog
        .bindings
        .get(&key)
        .ok_or(FieldOpsError::Unit(UnitError::EmptyId))?;
    let unit = TacticalUnit::from_spec(&binding.spec, obj).map_err(FieldOpsError::Unit)?;
    Ok((unit, spec))
}

/// Construct a [`SpaceShipState`] directly from a bound [`TacticalUnit`]'s live section
/// `internals` and facet `armor` cells (`D-35-1`).
///
/// # Errors
/// Returns [`FieldOpsError::Unit`] if canonical section, facet, or cell identifiers fail
/// validation.
pub fn space_ship_from_tactical_unit(
    unit: &TacticalUnit,
    ship_id: impl Into<String>,
    callsign: impl Into<String>,
    party: SpacePartyId,
    pos_m: [i64; 3],
    vel_mmps: [i64; 3],
) -> Result<SpaceShipState, FieldOpsError> {
    let mut ship = SpaceShipState::new_dray(ship_id, callsign, party, pos_m, vel_mmps);
    ship.hull_class = unit.schema().key().id.as_str().to_owned();

    let internals_cell = CellId::new(INTERNALS_CELL).map_err(FieldOpsError::Unit)?;
    let armor_cell = CellId::new(ARMOR_CELL).map_err(FieldOpsError::Unit)?;

    let mut sections = BTreeMap::new();
    for sec_id in ALL_SHIP_SECTIONS {
        let tac_sec_id = SectionId::new(sec_id.id_str()).map_err(FieldOpsError::Unit)?;
        let internals_addr = CellAddr::Section {
            section: tac_sec_id.clone(),
            cell: internals_cell.clone(),
        };
        let (internals, max_internals) = if let Some(cell) = unit.cell(&internals_addr) {
            (
                u32::try_from(cell.current.max(0)).unwrap_or(0),
                u32::try_from(cell.max.max(0)).unwrap_or(0),
            )
        } else if let Some(fallback) = ship.sections.get(&sec_id) {
            (fallback.internals, fallback.max_internals)
        } else {
            (0, 0)
        };

        let mut facets = BTreeMap::new();
        for band in ALL_FACET_BANDS {
            let tac_facet_id = FacetId::new(band.id_str()).map_err(FieldOpsError::Unit)?;
            let facet_addr = CellAddr::Facet {
                section: tac_sec_id.clone(),
                facet: tac_facet_id,
                cell: armor_cell.clone(),
            };
            if let Some(cell) = unit.cell(&facet_addr) {
                let armor = u32::try_from(cell.current.max(0)).unwrap_or(0);
                facets.insert(band, armor);
            }
        }

        sections.insert(
            sec_id,
            ShipSectionState {
                id: sec_id,
                facets,
                internals,
                max_internals,
            },
        );
    }

    ship.sections = sections;

    let drive_zero = ship
        .sections
        .get(&ShipSectionId::Drive)
        .is_some_and(|s| s.internals == 0);
    let hull_zero = ship
        .sections
        .get(&ShipSectionId::Hull)
        .is_some_and(|s| s.internals == 0);
    if drive_zero || hull_zero {
        ship.status = HullStatus::DisabledDerelict {
            epoch_open: true,
            life_support_remaining_s: DERELICT_WINDOW_S,
            salvage_contest_window_s: DERELICT_WINDOW_S,
        };
    }

    Ok(ship)
}

/// Build and apply an IC-custody-legal [`UnitMutationPlan`] that persists a
/// [`SpaceShipState`]'s combat or docking damage onto `before`, returning the updated
/// [`TacticalUnit`], its [`FieldUnitAssessment`] (`Operational`, `Immobile`, `Disabled`,
/// `Wreck`), and the [`UnitMutationPlan`] (`D-35-1`, `D-35-12`).
///
/// # Errors
/// Returns [`FieldOpsError`] if cell/condition mutation or IC condition handoff fails.
pub fn build_space_damage_mutation_plan(
    before: &TacticalUnit,
    ship: &SpaceShipState,
    spec: Option<&UnitSpec>,
    at_ic: i64,
) -> Result<(TacticalUnit, FieldUnitAssessment, Option<UnitMutationPlan>), FieldOpsError> {
    let internals_cell = CellId::new(INTERNALS_CELL).map_err(FieldOpsError::Unit)?;
    let armor_cell = CellId::new(ARMOR_CELL).map_err(FieldOpsError::Unit)?;
    let destroyed_cond = ConditionId::new("destroyed").map_err(FieldOpsError::Unit)?;
    let damaged_cond = ConditionId::new("damaged").map_err(FieldOpsError::Unit)?;
    let intact_cond = ConditionId::new("intact").map_err(FieldOpsError::Unit)?;

    let mut ops = Vec::new();

    for (sec_id, sec_state) in &ship.sections {
        let tac_sec_id = SectionId::new(sec_id.id_str()).map_err(FieldOpsError::Unit)?;

        // 1. Section internals cell
        let internals_addr = CellAddr::Section {
            section: tac_sec_id.clone(),
            cell: internals_cell.clone(),
        };
        if let Some(before_cell) = before.cell(&internals_addr) {
            let target_val = i64::from(sec_state.internals).clamp(before_cell.min, before_cell.max);
            if before_cell.current != target_val {
                ops.push(UnitMutationOp::SetCell {
                    addr: internals_addr,
                    value: target_val,
                });
            }
        }

        // 2. Section condition (`intact`, `damaged`, `destroyed`)
        if let Some(before_sec) = before.section(&tac_sec_id) {
            let target_cond = if sec_state.internals == 0 {
                &destroyed_cond
            } else if sec_state.internals < sec_state.max_internals {
                &damaged_cond
            } else {
                &intact_cond
            };
            if &before_sec.condition != target_cond {
                ops.push(UnitMutationOp::SetSectionCondition {
                    section: tac_sec_id.clone(),
                    condition: target_cond.clone(),
                });
            }
        }

        // 3. Facet armor cells
        for (band, &armor_val) in &sec_state.facets {
            let tac_facet_id = FacetId::new(band.id_str()).map_err(FieldOpsError::Unit)?;
            let facet_addr = CellAddr::Facet {
                section: tac_sec_id.clone(),
                facet: tac_facet_id,
                cell: armor_cell.clone(),
            };
            if let Some(before_cell) = before.cell(&facet_addr) {
                let target_val = i64::from(armor_val).clamp(before_cell.min, before_cell.max);
                if before_cell.current != target_val {
                    ops.push(UnitMutationOp::SetCell {
                        addr: facet_addr,
                        value: target_val,
                    });
                }
            }
        }
    }

    let after_cells = if ops.is_empty() {
        before.clone()
    } else {
        let scratch_plan = UnitMutationPlan::new(ops, Some(before.revision()), None)
            .map_err(FieldOpsError::Unit)?;
        apply(before, &scratch_plan, at_ic)
            .map_err(FieldOpsError::Unit)?
            .0
    };

    apply_ic_condition_handoff(before, &after_cells, spec, at_ic)
}

/// Format `+ship`'s canonical `Damage: {sections}.` line directly from a [`TacticalUnit`]
/// (`doc 32` cut `CF.3`, `D-35-1`), matching [`crate::port::ship::damage_line`].
#[must_use]
pub fn format_tactical_unit_damage_line(unit: &TacticalUnit) -> Option<String> {
    let hurt: Vec<String> = unit
        .cells()
        .filter_map(|(addr, cell)| match addr {
            CellAddr::Section { section, cell: id }
                if id.as_str() == INTERNALS_CELL && cell.current < cell.max =>
            {
                Some(if cell.current <= cell.min {
                    format!("{} destroyed", section.as_str())
                } else {
                    format!(
                        "{} {}/{}",
                        section.as_str(),
                        cell.current.saturating_sub(cell.min),
                        cell.max.saturating_sub(cell.min)
                    )
                })
            }
            _ => None,
        })
        .collect();
    (!hurt.is_empty()).then(|| SHIP_DAMAGE.replace("{sections}", &hurt.join(", ")))
}

/// Record [`SpaceCombatEvent`] hits (`TorpedoHit`, `LaserHit`, `RailgunHit`) and
/// open-epoch disablement (`ShipDisabledDerelict`) into `target_hull`'s tamper-evident
/// black-box flight recorder inside [`SalvageClaimBook`] (`FO-4`, `Q-H6`, `Q-S4`, `D-35-12`).
///
/// # Errors
/// Returns [`FieldOpsError`] if section id construction fails or the hull's black-box
/// entry cap is exceeded.
pub fn record_space_combat_to_black_box(
    salvage_book: &mut SalvageClaimBook,
    target_hull: ObjId,
    target_ship_id: &str,
    attacker_hull: Option<ObjId>,
    events: &[SpaceCombatEvent],
    resulting_state: OperationalState,
    at_ic: i64,
) -> Result<usize, FieldOpsError> {
    let mut recorded = 0usize;
    for ev in events {
        match ev {
            SpaceCombatEvent::TorpedoHit {
                target_id, section, ..
            }
            | SpaceCombatEvent::LaserHit {
                target_id, section, ..
            }
            | SpaceCombatEvent::RailgunHit {
                target_id, section, ..
            } if target_id == target_ship_id => {
                let sec_id = SectionId::new(section.id_str()).map_err(FieldOpsError::Unit)?;
                salvage_book
                    .append_black_box_entry(
                        target_hull,
                        BlackBoxFlightEntry::CombatDamageRecorded {
                            attacker: attacker_hull,
                            section: sec_id,
                            resulting_state,
                            at_ic,
                        },
                    )
                    .map_err(FieldOpsError::Salvage)?;
                recorded = recorded.saturating_add(1);
            }
            SpaceCombatEvent::ShipDisabledDerelict { ship_id, .. } if ship_id == target_ship_id => {
                let sec_id =
                    SectionId::new(ShipSectionId::Drive.id_str()).map_err(FieldOpsError::Unit)?;
                salvage_book
                    .append_black_box_entry(
                        target_hull,
                        BlackBoxFlightEntry::CombatDamageRecorded {
                            attacker: attacker_hull,
                            section: sec_id,
                            resulting_state,
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

/// Register a clamped derelict [`SalvageTowRecord`] inside `doc 27a`'s authoritative
/// [`SalvageClaimBook`], recording the physical tow attachment (`TowAttached`), carrier/port
/// berth recovery (`EpochClosedInBay`), and resolving custody into either an immediate
/// self-recovery title or a 48-hour (`172,800 s`) [`CustodyTitleStatus::PendingSalvageClaim`]
/// (`D-35-12`, `D-35-14`, `Q-S1`, `Q-S2`, `Q-S14`, `Q-S15`).
///
/// # Errors
/// Returns [`FieldOpsError::Salvage`] if object identifiers are invalid or a duplicate
/// active claim already exists for `derelict_obj`.
#[allow(clippy::too_many_arguments)]
pub fn register_derelict_salvage_in_book(
    salvage_book: &mut SalvageClaimBook,
    operation_id: RecoveryOpId,
    _tow: &SalvageTowRecord,
    derelict_obj: ObjId,
    tug_obj: ObjId,
    recoverer_obj: ObjId,
    prior_owner_obj: ObjId,
    port_base_obj: ObjId,
    attached_at_ic: i64,
    filed_at_real_s: i64,
) -> Result<CustodyTitleStatus, FieldOpsError> {
    let authority_ctx = RecoveryAuthorityContext {
        recoverer: recoverer_obj,
        unit_title_holder: prior_owner_obj,
        has_owner_authorization: recoverer_obj == prior_owner_obj,
        has_landholder_authorization: false,
        ground_tenure_at_attach: GroundTenure::Unclaimed,
    };

    salvage_book
        .record_physical_attach(
            operation_id,
            derelict_obj,
            tug_obj,
            &authority_ctx,
            attached_at_ic,
        )
        .map_err(FieldOpsError::Salvage)?;

    salvage_book
        .record_bay_load(operation_id, port_base_obj, attached_at_ic)
        .map_err(FieldOpsError::Salvage)?;

    salvage_book
        .resolve_hull_recovery_custody(operation_id, filed_at_real_s)
        .map_err(FieldOpsError::Salvage)
}

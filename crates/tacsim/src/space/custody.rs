//! **Kernel Custody, `TacticalUnit` Damage Persistence & `doc 27a` Salvage Registry Bridge**
//! (Doc 35 cut `SF.R2`; `docs/design/35-ship-free-flight-combat-and-mining.md` `D-35-1`,
//! `D-35-12`, `D-35-13`, `D-35-14`).
//!
//! Bridges the Tier-1/Tier-2/Tier-3 space flight, combat, and 6-DOF docking simulation
//! directly into `mush-tactics` and `tacsim::field_ops`:
//! 1. **Canonical `UnitSpec` + `TacticalUnit` $\leftrightarrow$ `SpaceShipState` custody (`D-35-1`)**:
//!    - [`check_out_space_unit`] checks out a live hull [`TacticalUnit`] via
//!      [`mush_tactics::custody::check_out`] (`DeployedMark::field`) and populates
//!      [`SpaceShipState`] from the unit's live section `internals`, facet `armor`,
//!      `heat` cell, component conditions, and canonical [`UnitSpec`] logistics
//!      (`dray.json`, `dray_p.json`, `halcyon.json`).
//!    - [`check_in_space_unit`] translates post-combat and post-docking [`SpaceShipState`]
//!      section/facet/component/heat state into an IC-custody-legal [`UnitMutationPlan`]
//!      and [`FieldUnitAssessment`] (`Operational`, `Immobile`, `Disabled`, `Wreck`) via
//!      [`build_space_damage_mutation_plan`] and commits it back to `UnitStore` through
//!      [`mush_tactics::custody::check_in`].
//! 2. **Tamper-Evident Black-Box Forensics (`FO-4`, `Q-H6`, `Q-S4`)**:
//!    - [`record_space_combat_to_black_box`] records [`SpaceCombatEvent`] hits (`TorpedoHit`,
//!      `LaserHit`, `RailgunHit`) and open-epoch disablement (`ShipDisabledDerelict`) as
//!      [`BlackBoxFlightEntry::CombatDamageRecorded`] entries inside [`SalvageClaimBook`].
//! 3. **48-Hour Registry Salvage Claims (`doc 27a`, `D-35-12`, `D-35-14`)**:
//!    - [`register_derelict_salvage_in_book`] records a clamped derelict recovery
//!      in [`SalvageClaimBook`] (`record_physical_attach`, `record_bay_load`, and
//!      `resolve_hull_recovery_custody`), opening the 48 real-hour (`172,800 s`) Registry
//!      contest window when recovered by a non-owner or resolving title immediately on
//!      self-recovery.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use mush_core::{CanonicalId, Ctx, ObjId};
use mush_sim::deployed::DeployedMark;
use mush_tactics::{
    CellAddr, CellId, ComponentId, ConditionId, FacetId, SectionId, TacticalUnit, UnitError,
    UnitMutationOp, UnitMutationPlan, apply,
};
use smol_str::SmolStr;

use crate::definitions::Catalog;
use crate::definitions::logistics::DriveLaw;
use crate::definitions::unit::{
    ARMOR_CELL, Consequence, HEAT_CELL, HEAT_SINK_CELL, INTERNALS_CELL, UnitSpec,
};
use crate::field_ops::{
    BlackBoxFlightEntry, ComponentCondition, CustodyTitleStatus, FieldOpsError,
    FieldUnitAssessment, GroundTenure, OperationalState, RecoveryAuthorityContext, RecoveryOpId,
    SalvageClaimBook, apply_ic_condition_handoff,
};
use crate::port::ship::SHIP_DAMAGE;
use crate::space::bubble::{
    DERELICT_WINDOW_S, FacetBand, HullStatus, ShipSectionId, ShipSectionState, SpaceCombatEvent,
    SpacePartyId, SpaceSectionConsequence, SpaceShipState, SubsystemCrit,
};
use crate::space::docking::DockingRcsAuthority;
use crate::space::weapons::{LaserMount, PdcMount, RailgunMount, TorpedoTube};

/// Space-only hull parameters that do not exist on [`UnitSpec`] (`D-35-1`, `35b` §4 & §8 `SF.R15`):
/// 6-DOF RCS thruster authority, weapon/point-defense mounts, and `mush-pixel` 3D model archetype.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpaceUnitOverlay {
    pub unit_id: &'static str,
    pub rcs_authority: DockingRcsAuthority,
    pub torpedo_tubes: Vec<TorpedoTube>,
    pub lasers: Vec<LaserMount>,
    pub railguns: Vec<RailgunMount>,
    pub pdcs: Vec<PdcMount>,
    pub decoys_remaining: u8,
    pub pixel_model: &'static str,
}

/// Look up the canonical [`SpaceUnitOverlay`] for `unit_entry_id` (`D-35-1`, `SF.R15`).
#[must_use]
pub fn space_unit_overlay_for_id(unit_entry_id: &str) -> SpaceUnitOverlay {
    let base_id = crate::definitions::core_replacement::surface_chassis(unit_entry_id);
    match base_id {
        "tacsim.unit.corvette" => SpaceUnitOverlay {
            unit_id: "tacsim.unit.corvette",
            rcs_authority: DockingRcsAuthority {
                trans_dv_mmps_per_frame: 40,
                closure_dv_mmps_per_frame: 35,
                attitude_rate_mdeg_s_per_frame: 240,
                roll_rate_mdeg_s_per_frame: 320,
                damp_trans_mmps_per_frame: 20,
                damp_attitude_mdeg_s_per_frame: 120,
                damp_roll_mdeg_s_per_frame: 160,
                kg_per_pulse_batch: 1,
            },
            torpedo_tubes: vec![TorpedoTube::standard_twin("torp_bow", 8)],
            lasers: vec![
                LaserMount::standard_turret("laser_1"),
                LaserMount::standard_turret("laser_2"),
            ],
            railguns: vec![RailgunMount::standard_spinal("rail_spinal", 24)],
            pdcs: vec![
                PdcMount::standard_rotary("pdc_port", 3_000),
                PdcMount::standard_rotary("pdc_stbd", 3_000),
            ],
            decoys_remaining: 6,
            pixel_model: "corvette",
        },
        "tacsim.unit.raider_heavy" => SpaceUnitOverlay {
            unit_id: "tacsim.unit.raider_heavy",
            rcs_authority: DockingRcsAuthority {
                trans_dv_mmps_per_frame: 35,
                closure_dv_mmps_per_frame: 30,
                attitude_rate_mdeg_s_per_frame: 210,
                roll_rate_mdeg_s_per_frame: 280,
                damp_trans_mmps_per_frame: 18,
                damp_attitude_mdeg_s_per_frame: 105,
                damp_roll_mdeg_s_per_frame: 140,
                kg_per_pulse_batch: 1,
            },
            torpedo_tubes: vec![TorpedoTube::standard_twin("torp_bow", 8)],
            lasers: vec![
                LaserMount::standard_turret("laser_1"),
                LaserMount::standard_turret("laser_2"),
            ],
            railguns: vec![RailgunMount::standard_spinal("rail_spinal", 24)],
            pdcs: vec![
                PdcMount::standard_rotary("pdc_port", 3_000),
                PdcMount::standard_rotary("pdc_stbd", 3_000),
            ],
            decoys_remaining: 6,
            pixel_model: "raider",
        },
        "tacsim.unit.halcyon" => SpaceUnitOverlay {
            unit_id: "tacsim.unit.halcyon",
            rcs_authority: DockingRcsAuthority {
                trans_dv_mmps_per_frame: 18,
                closure_dv_mmps_per_frame: 15,
                attitude_rate_mdeg_s_per_frame: 100,
                roll_rate_mdeg_s_per_frame: 140,
                damp_trans_mmps_per_frame: 9,
                damp_attitude_mdeg_s_per_frame: 50,
                damp_roll_mdeg_s_per_frame: 70,
                kg_per_pulse_batch: 2,
            },
            torpedo_tubes: vec![TorpedoTube::standard_twin("torp_bow", 16)],
            lasers: vec![
                LaserMount::standard_turret("laser_dorsal"),
                LaserMount::standard_turret("laser_ventral"),
            ],
            railguns: vec![RailgunMount::standard_spinal("rail_spinal", 32)],
            pdcs: vec![
                PdcMount::standard_rotary("pdc_port", 4_000),
                PdcMount::standard_rotary("pdc_stbd", 4_000),
                PdcMount::standard_rotary("pdc_dorsal", 4_000),
                PdcMount::standard_rotary("pdc_ventral", 4_000),
            ],
            decoys_remaining: 8,
            pixel_model: "halcyon",
        },
        "tacsim.unit.dray_p" => SpaceUnitOverlay {
            unit_id: "tacsim.unit.dray_p",
            rcs_authority: DockingRcsAuthority::default(),
            torpedo_tubes: Vec::new(),
            lasers: vec![
                LaserMount::standard_turret("laser_dorsal"),
                LaserMount::standard_turret("laser_ventral"),
            ],
            railguns: Vec::new(),
            pdcs: vec![
                PdcMount::standard_rotary("pdc_dorsal", 2_000),
                PdcMount::standard_rotary("pdc_ventral", 2_000),
            ],
            decoys_remaining: 4,
            pixel_model: "freighter",
        },
        _ => SpaceUnitOverlay {
            unit_id: "tacsim.unit.dray",
            rcs_authority: DockingRcsAuthority::default(),
            torpedo_tubes: Vec::new(),
            lasers: vec![
                LaserMount::standard_turret("laser_dorsal"),
                LaserMount::standard_turret("laser_ventral"),
            ],
            railguns: Vec::new(),
            pdcs: vec![
                PdcMount::standard_rotary("pdc_dorsal", 2_000),
                PdcMount::standard_rotary("pdc_ventral", 2_000),
            ],
            decoys_remaining: 4,
            pixel_model: "freighter",
        },
    }
}

/// Apply [`SpaceUnitOverlay`] RCS authority and weapon/PDC mounts to `ship` (`D-35-1`, `SF.R15`).
pub fn apply_space_unit_overlay(ship: &mut SpaceShipState, overlay: &SpaceUnitOverlay) {
    ship.rcs_authority = overlay.rcs_authority;
    ship.torpedo_tubes = overlay.torpedo_tubes.clone();
    ship.lasers = overlay.lasers.clone();
    ship.railguns = overlay.railguns.clone();
    ship.pdcs = overlay.pdcs.clone();
    ship.decoys_remaining = overlay.decoys_remaining;
}

fn parse_ship_section_id(word: &str) -> Option<ShipSectionId> {
    match word {
        "hull" => Some(ShipSectionId::Hull),
        "hold" => Some(ShipSectionId::Hold),
        "drive" | "power" => Some(ShipSectionId::Drive),
        _ => None,
    }
}

fn parse_facet_band(word: &str) -> Option<FacetBand> {
    match word {
        "front" => Some(FacetBand::Front),
        "left" => Some(FacetBand::Left),
        "right" => Some(FacetBand::Right),
        "rear" => Some(FacetBand::Rear),
        "top" => Some(FacetBand::Top),
        _ => None,
    }
}

fn map_consequence_word(word: &Consequence) -> Option<SpaceSectionConsequence> {
    match word {
        Consequence::DestroyUnit => Some(SpaceSectionConsequence::DestroyUnit),
        Consequence::DisableMotion => Some(SpaceSectionConsequence::DisableMotion),
        Consequence::Slow { percent } => Some(SpaceSectionConsequence::Slow {
            percent: u32::from(*percent),
        }),
        _ => None,
    }
}

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

fn fielded_space_catalog() -> Option<&'static Catalog> {
    static CATALOG: OnceLock<Option<Catalog>> = OnceLock::new();
    CATALOG
        .get_or_init(|| Catalog::build_fielded(&[], "tacsim").ok())
        .as_ref()
}

/// Look up the authored [`UnitSpec`] for `unit_entry_id` (normalizing fitted core chassis
/// ids via [`crate::definitions::core_replacement::surface_chassis`]).
#[must_use]
pub fn unit_spec_for_id(unit_entry_id: &str) -> Option<&'static UnitSpec> {
    let base_id = crate::definitions::core_replacement::surface_chassis(unit_entry_id);
    let key = CanonicalId::new(base_id).ok()?;
    fielded_space_catalog()?.specs.get(&key)
}

/// Resolve the [`SectionId`] on `unit` corresponding to `sec_id` (`D-35-1`).
///
/// Maps [`ShipSectionId::Drive`] to `"drive"` on `dray.json` / `dray_p.json` and falls
/// back to `"power"` on `halcyon.json` (whose propulsion/motion-disabling section is `"power"`).
#[must_use]
pub fn resolve_tactical_section_id(
    unit: &TacticalUnit,
    sec_id: ShipSectionId,
) -> Option<SectionId> {
    let primary = SectionId::new(sec_id.id_str()).ok()?;
    if unit.section(&primary).is_some() {
        return Some(primary);
    }
    if sec_id == ShipSectionId::Drive {
        let power_sec = SectionId::new("power").ok()?;
        if unit.section(&power_sec).is_some() {
            return Some(power_sec);
        }
    }
    None
}

/// Apply canonical [`UnitSpec`] logistics, drive parameters, propellant capacity,
/// authored section `struck_through` / `consequences`, facet band weights, heat dissipation,
/// and silhouette hit radius to `ship` (`D-35-1`, `35a` §1 row 2 & §6 row `SF.16/19`, `SF.R15`).
pub fn apply_unit_spec_to_space_ship(ship: &mut SpaceShipState, spec: &UnitSpec) {
    if spec.heat_dissipation_per_s > 0 {
        ship.cooling_rate_permille_s = u16::try_from(spec.heat_dissipation_per_s).unwrap_or(30);
    }
    if !spec.facets.is_empty() {
        let mut weights_map = BTreeMap::new();
        for fb in &spec.facets {
            let Some(band) = parse_facet_band(fb.band.as_str()) else {
                continue;
            };
            let mut band_weights = Vec::new();
            for entry in &fb.sections {
                if let Some(sec_id) = parse_ship_section_id(entry.section.as_str())
                    && entry.weight > 0
                {
                    band_weights.push((sec_id, u32::from(entry.weight)));
                }
            }
            if !band_weights.is_empty() {
                weights_map.insert(band, band_weights);
            }
        }
        if !weights_map.is_empty() {
            ship.facet_weights = weights_map;
        }
    }
    for sec_spec in &spec.sections {
        let Some(sec_id) = parse_ship_section_id(sec_spec.id.as_str()) else {
            continue;
        };
        if let Some(sec_state) = ship.sections.get_mut(&sec_id) {
            sec_state.struck_through = sec_spec.struck_through.as_ref().and_then(|st| {
                Some((
                    parse_ship_section_id(st.section.as_str())?,
                    parse_facet_band(st.facet.as_str())?,
                ))
            });
            sec_state.consequences = sec_spec
                .consequences
                .iter()
                .filter_map(map_consequence_word)
                .collect();
        }
    }
    if let Some(logistics) = &spec.logistics {
        ship.dry_mass_t = logistics.dry_mass_t;
        ship.payload_ceiling_kg = logistics.payload_ceiling_kg;
        if let Ok(prop_key) = CanonicalId::new("propellant")
            && let Ok(mass_key) = CanonicalId::new("si.mass_kg")
            && let Some(prop_stow) = logistics.stowage.get(&prop_key)
            && let Some(&mass_kg) = prop_stow.limits.get(&mass_key)
        {
            let cap_kg = u32::try_from(mass_kg).unwrap_or(u32::MAX);
            ship.propellant_cap_kg = cap_kg;
            ship.propellant_kg = cap_kg;
        }
        if let Some(drive) = &logistics.drive {
            match drive.law {
                DriveLaw::Flip {
                    accel_mg,
                    exhaust_kms,
                } => {
                    ship.rated_accel_mg = accel_mg;
                    ship.exhaust_kms = exhaust_kms;
                    let hull_drive = crate::port::passage::HullDrive {
                        class: drive.class.clone(),
                        law: drive.law,
                        payload_ceiling_kg: logistics.payload_ceiling_kg,
                        throttle_permille: crate::port::passage::MAX_THROTTLE_PERMILLE,
                    };
                    let dry_kg = u64::from(logistics.dry_mass_t).saturating_mul(1_000);
                    let payload_kg = u64::from(ship.propellant_kg);
                    if let Some(accel) =
                        crate::port::passage::flip_accel_mps2(&hull_drive, dry_kg, payload_kg)
                    {
                        ship.max_accel_mps2 = (accel.ceil() as i64).max(1);
                    }
                }
                DriveLaw::Coast { .. } => {}
            }
        }
    }
    if let Some(min_dim_cm) = spec
        .silhouettes
        .iter()
        .map(|s| s.width_cm.min(s.height_cm))
        .min()
    {
        let radius_m = i64::from(min_dim_cm / 300).max(5);
        ship.hit_radius_m = radius_m;
    }
}

/// Instantiate a stock spacecraft [`TacticalUnit`] and its authored [`UnitSpec`] from
/// the compiled-in fielded catalog (e.g. `"tacsim.unit.dray"`, `"tacsim.unit.dray_p"`,
/// or `"tacsim.unit.halcyon"`).
///
/// # Errors
/// Returns [`FieldOpsError`] if the catalog or unit entry cannot be bound.
pub fn instantiate_stock_space_unit(
    obj: ObjId,
    unit_entry_id: &str,
) -> Result<(TacticalUnit, UnitSpec), FieldOpsError> {
    let catalog = fielded_space_catalog().ok_or(FieldOpsError::Unit(UnitError::EmptyId))?;
    let base_id = crate::definitions::core_replacement::surface_chassis(unit_entry_id);
    let key = CanonicalId::new(base_id).map_err(|_| FieldOpsError::Unit(UnitError::EmptyId))?;
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

/// Create and register a stock spacecraft [`TacticalUnit`] in `ctx.w`'s `UnitStore` from the
/// compiled-in fielded catalog (`"tacsim.unit.corvette"`, `"tacsim.unit.dray"`, etc.) (`SF.R12`, `D-35-1`).
///
/// # Errors
/// Returns an error string if the catalog binding cannot be resolved or `create_unit` fails.
pub fn create_stock_space_unit_in_world(
    ctx: &mut Ctx<'_>,
    obj: ObjId,
    unit_entry_id: &str,
) -> Result<TacticalUnit, String> {
    let catalog = fielded_space_catalog().ok_or_else(|| "space catalog unavailable".to_owned())?;
    let base_id = crate::definitions::core_replacement::surface_chassis(unit_entry_id);
    let key = CanonicalId::new(base_id).map_err(|e| e.to_string())?;
    let binding = catalog
        .bindings
        .get(&key)
        .ok_or_else(|| format!("no binding for {base_id}"))?;
    mush_tactics::module::create_unit(ctx, obj, &binding.spec)
}

/// Construct a [`SpaceShipState`] directly from a bound [`TacticalUnit`]'s live section
/// `internals`, facet `armor`, `heat` cell, component conditions, and canonical [`UnitSpec`]
/// (`D-35-1`).
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
    let spec = unit_spec_for_id(unit.schema().key().id.as_str());
    space_ship_from_tactical_unit_with_spec(unit, spec, ship_id, callsign, party, pos_m, vel_mmps)
}

/// Construct a [`SpaceShipState`] from `unit` and an explicit optional [`UnitSpec`] (`D-35-1`).
///
/// # Errors
/// Returns [`FieldOpsError::Unit`] if canonical section, facet, or cell identifiers fail
/// validation.
pub fn space_ship_from_tactical_unit_with_spec(
    unit: &TacticalUnit,
    spec: Option<&UnitSpec>,
    ship_id: impl Into<String>,
    callsign: impl Into<String>,
    party: SpacePartyId,
    pos_m: [i64; 3],
    vel_mmps: [i64; 3],
) -> Result<SpaceShipState, FieldOpsError> {
    let base_id =
        crate::definitions::core_replacement::surface_chassis(unit.schema().key().id.as_str());
    let mut ship = if base_id == "tacsim.unit.corvette" {
        SpaceShipState::new_corvette(ship_id, callsign, party, pos_m, vel_mmps)
    } else {
        SpaceShipState::new_dray(ship_id, callsign, party, pos_m, vel_mmps)
    };
    ship.hull_class = base_id.to_owned();
    apply_space_unit_overlay(&mut ship, &space_unit_overlay_for_id(base_id));

    let resolved_spec = spec.or_else(|| unit_spec_for_id(base_id));
    if let Some(s) = resolved_spec {
        apply_unit_spec_to_space_ship(&mut ship, s);
        if matches!(base_id, "tacsim.unit.corvette" | "tacsim.unit.raider_heavy") {
            ship.max_accel_mps2 = 60;
            ship.delta_v_mps = 9_500;
        }
    }

    let internals_cell = CellId::new(INTERNALS_CELL).map_err(FieldOpsError::Unit)?;
    let armor_cell = CellId::new(ARMOR_CELL).map_err(FieldOpsError::Unit)?;

    let mut sections = BTreeMap::new();
    for sec_id in ALL_SHIP_SECTIONS {
        let Some(tac_sec_id) = resolve_tactical_section_id(unit, sec_id) else {
            continue;
        };
        let internals_addr = CellAddr::Section {
            section: tac_sec_id.clone(),
            cell: internals_cell.clone(),
        };
        let (internals, max_internals) = if let Some(cell) = unit.cell(&internals_addr) {
            (
                u32::try_from(cell.current.max(0)).unwrap_or(0),
                u32::try_from(cell.max.max(0)).unwrap_or(0),
            )
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

        let (struck_through, consequences) = ship
            .sections
            .get(&sec_id)
            .map(|s| (s.struck_through, s.consequences.clone()))
            .unwrap_or_default();

        sections.insert(
            sec_id,
            ShipSectionState {
                id: sec_id,
                facets,
                internals,
                max_internals,
                struck_through,
                consequences,
            },
        );
    }

    if !sections.is_empty() {
        ship.sections = sections;
        if let Some(s) = resolved_spec {
            for sec_spec in &s.sections {
                let Some(sec_id) = parse_ship_section_id(sec_spec.id.as_str()) else {
                    continue;
                };
                if let Some(sec_state) = ship.sections.get_mut(&sec_id) {
                    sec_state.struck_through = sec_spec.struck_through.as_ref().and_then(|st| {
                        Some((
                            parse_ship_section_id(st.section.as_str())?,
                            parse_facet_band(st.facet.as_str())?,
                        ))
                    });
                    sec_state.consequences = sec_spec
                        .consequences
                        .iter()
                        .filter_map(map_consequence_word)
                        .collect();
                }
            }
        }
    }

    // Read hull `heat` and `heat_sink` cells (`D-35-1`, `SF.R15`).
    if let Ok(hull_sec) = SectionId::new(ShipSectionId::Hull.id_str()) {
        if let Ok(heat_cell) = CellId::new(HEAT_CELL)
            && let Some(cell) = unit.cell(&CellAddr::Section {
                section: hull_sec.clone(),
                cell: heat_cell,
            })
        {
            ship.heat_permille = u16::try_from(cell.current.clamp(0, 1_000)).unwrap_or(0);
        }
        if let Ok(sink_cell) = CellId::new(HEAT_SINK_CELL)
            && let Some(cell) = unit.cell(&CellAddr::Section {
                section: hull_sec,
                cell: sink_cell,
            })
            && cell.current > 0
        {
            ship.cooling_rate_permille_s = u16::try_from(cell.current.min(1_000)).unwrap_or(30);
        }
    }

    // Reflect degraded `power` component condition as a drive/radiator subsystem crit.
    if let Ok(power_id) = ComponentId::new("power")
        && let Some(comp) = unit.component(&power_id)
    {
        let cond = ComponentCondition::classify_condition_id(&comp.condition);
        if matches!(
            cond,
            ComponentCondition::Damaged | ComponentCondition::Jammed
        ) && !ship.crits.contains(&SubsystemCrit::DriveDegraded)
        {
            ship.crits.push(SubsystemCrit::DriveDegraded);
        }
    }

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
/// [`SpaceShipState`]'s combat or docking damage (section `internals`, facet `armor`,
/// `heat` cell, section conditions, and component conditions) onto `before`, returning
/// the updated [`TacticalUnit`], its [`FieldUnitAssessment`] (`Operational`, `Immobile`,
/// `Disabled`, `Wreck`), and the [`UnitMutationPlan`] (`D-35-1`, `D-35-12`).
///
/// # Errors
/// Returns [`FieldOpsError`] if cell/condition mutation or IC condition handoff fails.
pub fn build_space_damage_mutation_plan(
    before: &TacticalUnit,
    ship: &SpaceShipState,
    spec: Option<&UnitSpec>,
    at_ic: i64,
) -> Result<(TacticalUnit, FieldUnitAssessment, Option<UnitMutationPlan>), FieldOpsError> {
    let resolved_spec = spec.or_else(|| unit_spec_for_id(before.schema().key().id.as_str()));
    let internals_cell = CellId::new(INTERNALS_CELL).map_err(FieldOpsError::Unit)?;
    let armor_cell = CellId::new(ARMOR_CELL).map_err(FieldOpsError::Unit)?;
    let destroyed_cond = ConditionId::new("destroyed").map_err(FieldOpsError::Unit)?;
    let wreck_cond = ConditionId::new("wreck").map_err(FieldOpsError::Unit)?;
    let damaged_cond = ConditionId::new("damaged").map_err(FieldOpsError::Unit)?;
    let intact_cond = ConditionId::new("intact").map_err(FieldOpsError::Unit)?;

    let all_ship_sections_zero = !ship.sections.is_empty()
        && ship.sections.values().all(|s| s.internals == 0)
        && matches!(
            ship.status,
            HullStatus::DisabledDerelict {
                epoch_open: true,
                ..
            }
        );

    let mut ops = Vec::new();

    for (sec_id, sec_state) in &ship.sections {
        let Some(tac_sec_id) = resolve_tactical_section_id(before, *sec_id) else {
            continue;
        };

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

        // 2. Section condition (`intact`, `damaged`, `destroyed`, or `wreck` on open-epoch hull zero)
        if let Some(before_sec) = before.section(&tac_sec_id) {
            let target_cond = if sec_state.internals == 0 {
                if *sec_id == ShipSectionId::Hull && all_ship_sections_zero {
                    &wreck_cond
                } else {
                    &destroyed_cond
                }
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

    // 4. Optional hull `heat` cell (`D-35-1`).
    if let Ok(hull_sec) = SectionId::new(ShipSectionId::Hull.id_str())
        && let Ok(heat_cell) = CellId::new(HEAT_CELL)
    {
        let heat_addr = CellAddr::Section {
            section: hull_sec,
            cell: heat_cell,
        };
        if let Some(before_heat) = before.cell(&heat_addr) {
            let target_heat = i64::from(ship.heat_permille).clamp(before_heat.min, before_heat.max);
            if before_heat.current != target_heat {
                ops.push(UnitMutationOp::SetCell {
                    addr: heat_addr,
                    value: target_heat,
                });
            }
        }
    }

    // 5. Subsystem crit -> surviving `power` component condition (`D-35-1`).
    let has_power_crit = ship.crits.contains(&SubsystemCrit::DriveDegraded)
        || ship.crits.contains(&SubsystemCrit::RadiatorDamaged);
    if has_power_crit
        && let Ok(power_id) = ComponentId::new("power")
        && let Some(before_comp) = before.component(&power_id)
        && before_comp.condition == intact_cond
    {
        ops.push(UnitMutationOp::SetComponentCondition {
            component: power_id,
            condition: damaged_cond,
        });
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

    apply_ic_condition_handoff(before, &after_cells, resolved_spec, at_ic)
}

/// Check out a live hull [`TacticalUnit`] into a space bubble via
/// [`mush_tactics::custody::check_out`] (`DeployedMark::field`) and construct its
/// canonical [`SpaceShipState`] (`SF.R2`, `D-35-1`).
///
/// # Errors
/// Returns an error string if the hull has no bound [`TacticalUnit`], is already
/// deployed under another mark, or fails `SpaceShipState` construction.
#[allow(clippy::too_many_arguments)]
pub fn check_out_space_unit(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    site_key: &str,
    ordinal: u32,
    ship_id: impl Into<String>,
    callsign: impl Into<String>,
    party: SpacePartyId,
    pos_m: [i64; 3],
    vel_mmps: [i64; 3],
) -> Result<SpaceShipState, String> {
    let mark = DeployedMark::field(site_key, ordinal.max(1));
    let marker = mush_tactics::custody::check_out(ctx, hull, mark)
        .map_err(|e| format!("space check_out failed for #{}: {e}", hull.0))?;
    space_ship_from_tactical_unit(&marker.snapshot, ship_id, callsign, party, pos_m, vel_mmps)
        .map_err(|e| format!("space ship construction failed for #{}: {e}", hull.0))
}

/// Check in a hull [`TacticalUnit`] from a space bubble via
/// [`mush_tactics::custody::check_in`], persisting all section `internals`, facet `armor`,
/// `heat`, section-condition, and component-condition mutations onto `UnitStore`
/// (`SF.R2`, `D-35-1`, `D-35-12`, `Q-CF-35`).
///
/// # Errors
/// Returns an error string if the hull has no bound [`TacticalUnit`] or if the
/// IC mutation plan cannot be constructed or applied.
pub fn check_in_space_unit(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    ship: &SpaceShipState,
    now_ic_s: i64,
) -> Result<FieldUnitAssessment, String> {
    let before = mush_tactics::module::unit_of(ctx.w, hull)
        .cloned()
        .ok_or_else(|| format!("hull #{} has no bound TacticalUnit", hull.0))?;
    let (_, assessment, plan_opt) = build_space_damage_mutation_plan(&before, ship, None, now_ic_s)
        .map_err(|e| format!("space check_in plan failed for #{}: {e}", hull.0))?;

    // Per `Q-CF-35` (`field/close.rs:247`): IC check-in never passes `None` (which would
    // restore the OOC pre-checkout snapshot); when no cells changed, pass an empty IC plan.
    let empty_plan = UnitMutationPlan::new(
        Vec::new(),
        Some(before.revision()),
        Some(SmolStr::new_static("space:checkin")),
    )
    .map_err(|e| format!("empty space check_in plan failed for #{}: {e}", hull.0))?;
    let plan_ref = plan_opt.as_ref().unwrap_or(&empty_plan);

    if mush_tactics::custody::marker_of(ctx.w, hull).is_some() {
        mush_tactics::custody::check_in(ctx, hull, Some(plan_ref))
            .map_err(|e| format!("space check_in failed for #{}: {e}", hull.0))?;
    } else {
        mush_sim::deployed::clear(ctx.w, hull);
    }

    Ok(assessment)
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

/// Register a clamped derelict recovery inside `doc 27a`'s authoritative
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

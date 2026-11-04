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
//! 3. **The space custody (doc 41 cut RS.8, Q-36-46)**: a towed derelict's
//!    [`SalvageCustody`] row lives in [`SpaceCustodyBook`], one durable
//!    `tacsim:space:custody:{domain}` document a recovery domain (a space site has no
//!    `SiteRecord`); `+salvage` files, contests and settles it through the ground's own
//!    `crate::surface::claim` paths, on the one `claim_window_s` window.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use mush_core::{CanonicalId, Ctx, ObjId};
use mush_sim::deployed::DeployedMark;
use mush_tactics::{
    CellAddr, CellId, ComponentId, ConditionId, FacetId, SectionId, TacticalUnit, UnitError,
    UnitMutationOp, UnitMutationPlan, apply,
};
use smol_str::SmolStr;

use crate::arena::party::PartyId;
use crate::definitions::Catalog;
use crate::definitions::logistics::DriveLaw;
use crate::definitions::unit::{
    ARMOR_CELL, Consequence, HEAT_CELL, HEAT_SINK_CELL, INTERNALS_CELL, UnitSpec,
};
use crate::field_ops::{
    BlackBoxFlightEntry, ComponentCondition, FieldOpsError, FieldUnitAssessment, GroundTenure,
    OperationalState, SalvageClaimBook, apply_ic_condition_handoff,
};
use crate::port::ship::SHIP_DAMAGE;
use crate::space::bubble::{
    DERELICT_LIFE_SUPPORT_S, FacetBand, HullStatus, ShipSectionId, ShipSectionState,
    SpaceCombatEvent, SpaceSectionConsequence, SpaceShipState, SubsystemCrit,
};
use crate::space::docking::DockingRcsAuthority;
use crate::space::weapons::{LaserMode, LaserMount, PdcMount, RailgunMount, TorpedoTube};
use crate::surface::salvage::SalvageCustody;

/// Capability id used to persist space-only subsystem crits (`PdcOffline`, `SensorDegraded`,
/// `RadiatorDamaged`, `DriveDegraded`), per-PDC `operational` flags, and `slow_pct` across
/// `check_in_space_unit` / `check_out_space_unit` (`D-35-1`, `SF.R20`).
pub const SPACE_SUBSYSTEMS_CAPABILITY: &str = "tacsim.space.subsystems";

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

/// Look up the canonical [`SpaceUnitOverlay`] for `unit_entry_id` (`D-35-1`, `SF.R15`, `SF.R22`).
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
                LaserMount {
                    max_range_m: 160_000,
                    base_damage: 180,
                    ..LaserMount::standard_turret("laser_1")
                },
                LaserMount {
                    mode: LaserMode::Defense,
                    defense_range_m: 45_000,
                    ..LaserMount::standard_turret("laser_2")
                },
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
            torpedo_tubes: vec![TorpedoTube {
                tubes: 2,
                max_dv_mps: 12_000,
                accel_mps2: 250,
                ..TorpedoTube::standard_twin("torp_bow", 8)
            }],
            lasers: vec![
                LaserMount::standard_turret("laser_1"),
                LaserMount::standard_turret("laser_2"),
            ],
            railguns: vec![RailgunMount {
                max_range_m: 250_000,
                muzzle_vel_mps: 12_000,
                ..RailgunMount::standard_spinal("rail_spinal", 24)
            }],
            pdcs: vec![
                PdcMount::standard_rotary("pdc_port", 3_000),
                PdcMount::standard_rotary("pdc_stbd", 3_000),
            ],
            decoys_remaining: 6,
            pixel_model: "raider",
        },
        "tacsim.unit.corvette_interdictor" => SpaceUnitOverlay {
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
                LaserMount {
                    mode: LaserMode::Defense,
                    ..LaserMount::standard_turret("laser_2")
                },
            ],
            railguns: vec![RailgunMount::standard_spinal("rail_spinal", 24)],
            pdcs: vec![
                PdcMount::standard_rotary("pdc_port", 3_000),
                PdcMount::standard_rotary("pdc_stbd", 3_000),
            ],
            decoys_remaining: 6,
            pixel_model: "corvette",
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
            rcs_authority: DockingRcsAuthority {
                trans_dv_mmps_per_frame: 32,
                closure_dv_mmps_per_frame: 28,
                attitude_rate_mdeg_s_per_frame: 195,
                roll_rate_mdeg_s_per_frame: 260,
                damp_trans_mmps_per_frame: 16,
                damp_attitude_mdeg_s_per_frame: 95,
                damp_roll_mdeg_s_per_frame: 130,
                kg_per_pulse_batch: 1,
            },
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
    ship.pixel_model = overlay.pixel_model.to_owned();
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
                    ship.base_rated_accel_mg = accel_mg;
                    ship.rated_accel_mg = accel_mg;
                    ship.exhaust_kms = exhaust_kms;
                    ship.recompute_drive_kinematics();
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
    party: PartyId,
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
    party: PartyId,
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

    // Reapply authored `Slow` consequences for any destroyed sections (`D-35-1`, `SF.R20`).
    let mut next_slow_pct = ship.slow_pct;
    let mut any_slow = false;
    for sec in ship.sections.values() {
        if sec.internals == 0 {
            for cons in &sec.consequences {
                if let SpaceSectionConsequence::Slow { percent } = *cons {
                    let clamped = u8::try_from(percent.clamp(1, 100)).unwrap_or(100);
                    next_slow_pct =
                        crate::arena::damage::apply_slow_consequence(next_slow_pct, clamped);
                    any_slow = true;
                }
            }
        }
    }
    if any_slow {
        ship.slow_pct = next_slow_pct;
        if !ship.crits.contains(&SubsystemCrit::DriveDegraded) {
            ship.crits.push(SubsystemCrit::DriveDegraded);
        }
    }

    // Read hull `heat` and `heat_sink` cells (`D-35-1`, `SF.R15`, `SF.R20`).
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
            if cell.current < cell.max && !ship.crits.contains(&SubsystemCrit::RadiatorDamaged) {
                ship.crits.push(SubsystemCrit::RadiatorDamaged);
            }
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

    // Restore persisted space subsystem crits (`PdcOffline`, `SensorDegraded`, `RadiatorDamaged`,
    // `DriveDegraded`), stripped PDC mounts, and `slow_pct` (`D-35-1`, `SF.R20`).
    if let Ok(cap_id) = mush_tactics::CapabilityId::new(SPACE_SUBSYSTEMS_CAPABILITY)
        && let Some(cap) = unit.capability(&mush_tactics::CapabilityKey {
            id: cap_id,
            source: None,
        })
    {
        let data = cap.data();
        if let Some(crits_val) = data.get("crits")
            && let Ok(saved_crits) = serde_json::from_value::<Vec<SubsystemCrit>>(crits_val.clone())
        {
            for crit in saved_crits {
                if !ship.crits.contains(&crit) {
                    ship.crits.push(crit);
                }
            }
        }
        if let Some(stripped_val) = data.get("stripped_pdcs")
            && let Ok(stripped_ids) = serde_json::from_value::<Vec<String>>(stripped_val.clone())
        {
            for pdc in &mut ship.pdcs {
                if stripped_ids.iter().any(|id| id == &pdc.id) {
                    pdc.operational = false;
                }
            }
        }
        if ship.crits.contains(&SubsystemCrit::PdcOffline)
            && ship.pdcs.iter().all(|p| p.operational)
            && let Some(first_pdc) = ship.pdcs.first_mut()
        {
            first_pdc.operational = false;
        }
        if let Some(slow_val) = data.get("slow_pct").and_then(serde_json::Value::as_u64)
            && let Ok(saved_slow) = u8::try_from(slow_val.clamp(1, 100))
        {
            ship.slow_pct = crate::arena::damage::apply_slow_consequence(ship.slow_pct, saved_slow);
        }
    }

    ship.recompute_drive_accel();

    if ship
        .carrier_damage()
        .has(crate::carrier_damage::CarrierLoss::RefusesLift)
    {
        ship.status = HullStatus::DisabledDerelict {
            epoch_open: true,
            life_support_remaining_s: DERELICT_LIFE_SUPPORT_S,
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
        && ship
            .carrier_damage()
            .has(crate::carrier_damage::CarrierLoss::Wrecked)
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

    // 4. Optional hull `heat` and `heat_sink` cells (`D-35-1`, `SF.R20`).
    if let Ok(hull_sec) = SectionId::new(ShipSectionId::Hull.id_str()) {
        if let Ok(heat_cell) = CellId::new(HEAT_CELL) {
            let heat_addr = CellAddr::Section {
                section: hull_sec.clone(),
                cell: heat_cell,
            };
            if let Some(before_heat) = before.cell(&heat_addr) {
                let target_heat =
                    i64::from(ship.heat_permille).clamp(before_heat.min, before_heat.max);
                if before_heat.current != target_heat {
                    ops.push(UnitMutationOp::SetCell {
                        addr: heat_addr,
                        value: target_heat,
                    });
                }
            }
        }
        if ship.crits.contains(&SubsystemCrit::RadiatorDamaged)
            && let Ok(sink_cell) = CellId::new(HEAT_SINK_CELL)
        {
            let sink_addr = CellAddr::Section {
                section: hull_sec,
                cell: sink_cell,
            };
            if let Some(before_sink) = before.cell(&sink_addr) {
                let target_sink =
                    (before_sink.max / 2).clamp(before_sink.min.max(1), before_sink.max);
                if before_sink.current != target_sink {
                    ops.push(UnitMutationOp::SetCell {
                        addr: sink_addr,
                        value: target_sink,
                    });
                }
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

    let (mut post_unit, assessment, mut plan_opt) =
        apply_ic_condition_handoff(before, &after_cells, resolved_spec, at_ic)?;

    // 6. Persist space-only subsystem crits (`PdcOffline`, `SensorDegraded`, `RadiatorDamaged`,
    //    `DriveDegraded`), stripped PDC mounts, and `slow_pct` across check-in (`D-35-1`, `SF.R20`).
    let stripped_pdcs: Vec<String> = ship
        .pdcs
        .iter()
        .filter(|p| !p.operational)
        .map(|p| p.id.clone())
        .collect();
    let has_subsystem_state =
        !ship.crits.is_empty() || !stripped_pdcs.is_empty() || ship.slow_pct < 100;
    let subsys_key = mush_tactics::CapabilityKey {
        id: mush_tactics::CapabilityId::new(SPACE_SUBSYSTEMS_CAPABILITY)
            .map_err(FieldOpsError::Unit)?,
        source: None,
    };
    let cap_op = if has_subsystem_state {
        let cap_schema = before
            .capabilities()
            .next()
            .and_then(|k| before.capability(k))
            .map(|rv| rv.schema().clone())
            .unwrap_or_else(|| before.schema().clone());
        let rules = mush_core::RulesValue::new(
            cap_schema,
            serde_json::json!({
                "crits": ship.crits,
                "stripped_pdcs": stripped_pdcs,
                "slow_pct": ship.slow_pct,
            }),
        )
        .map_err(|_| FieldOpsError::Unit(mush_tactics::UnitError::EmptyId))?;
        if before.capability(&subsys_key) != Some(&rules) {
            Some(UnitMutationOp::GrantCapability {
                key: subsys_key,
                rules,
            })
        } else {
            None
        }
    } else if before.capability(&subsys_key).is_some() {
        Some(UnitMutationOp::RevokeCapability { key: subsys_key })
    } else {
        None
    };

    if let Some(op) = cap_op {
        let mut combined_ops = plan_opt
            .as_ref()
            .map_or_else(Vec::new, |p| p.ops().to_vec());
        combined_ops.push(op);
        let new_plan = UnitMutationPlan::new(
            combined_ops,
            Some(before.revision()),
            Some(SmolStr::new_static("ic_combat_return")),
        )
        .map_err(FieldOpsError::Unit)?;
        post_unit = apply(before, &new_plan, at_ic)
            .map_err(FieldOpsError::Unit)?
            .0;
        plan_opt = Some(new_plan);
    }

    Ok((post_unit, assessment, plan_opt))
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
    party: PartyId,
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
    // Doc 42 cut CR.6 (Q-42-24): the crew aboard check in with the hull.
    check_in_space_crew(
        ctx,
        hull,
        matches!(ship.status, HullStatus::DisabledDerelict { .. }),
    );

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

/// Most custody rows one recovery domain's document holds, as a ground site's record does.
pub const MAX_SPACE_CUSTODY: usize = 32;

/// **The space custody** (doc 41 cut RS.8, Q-36-46): the [`SalvageCustody`] rows of towed
/// derelicts, grouped by the domain each was recovered in, each group one durable
/// `tacsim:space:custody:{domain}` document. A hull stands in at most one row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpaceCustodyBook {
    by_domain: BTreeMap<mush_sim::DomainId, Vec<SalvageCustody>>,
}

impl SpaceCustodyBook {
    /// The row naming `hull`.
    #[must_use]
    pub fn get(&self, hull: &ObjId) -> Option<&SalvageCustody> {
        self.rows().find(|row| row.hull == *hull)
    }

    /// The row naming `hull`, mutably.
    pub fn get_mut(&mut self, hull: &ObjId) -> Option<&mut SalvageCustody> {
        self.by_domain
            .values_mut()
            .flat_map(|rows| rows.iter_mut())
            .find(|row| row.hull == *hull)
    }

    /// Whether a row names `hull`.
    #[must_use]
    pub fn contains_key(&self, hull: &ObjId) -> bool {
        self.get(hull).is_some()
    }

    /// The domain whose document holds `hull`'s row.
    #[must_use]
    pub fn domain_of(&self, hull: ObjId) -> Option<mush_sim::DomainId> {
        self.by_domain
            .iter()
            .find(|(_, rows)| rows.iter().any(|row| row.hull == hull))
            .map(|(domain, _)| *domain)
    }

    /// Put `row` in `domain`'s document, replacing any row naming its hull. `false`, and nothing
    /// written, when the document is full ([`MAX_SPACE_CUSTODY`]).
    pub fn insert(&mut self, domain: mush_sim::DomainId, row: SalvageCustody) -> bool {
        let _ = self.remove(&row.hull);
        let rows = self.by_domain.entry(domain).or_default();
        if rows.len() >= MAX_SPACE_CUSTODY {
            if rows.is_empty() {
                self.by_domain.remove(&domain);
            }
            return false;
        }
        rows.push(row);
        true
    }

    /// Take `hull`'s row out; an emptied document goes.
    pub fn remove(&mut self, hull: &ObjId) -> Option<SalvageCustody> {
        let domain = self.domain_of(*hull)?;
        let rows = self.by_domain.get_mut(&domain)?;
        let at = rows.iter().position(|row| row.hull == *hull)?;
        let row = rows.remove(at);
        if rows.is_empty() {
            self.by_domain.remove(&domain);
        }
        Some(row)
    }

    /// Every row, by domain then filing order.
    pub fn rows(&self) -> impl Iterator<Item = &SalvageCustody> {
        self.by_domain.values().flatten()
    }

    /// The documents: each domain and its rows.
    #[must_use]
    pub fn documents(&self) -> &BTreeMap<mush_sim::DomainId, Vec<SalvageCustody>> {
        &self.by_domain
    }

    /// Restore one domain's document as read from the store.
    pub fn restore(&mut self, domain: mush_sim::DomainId, rows: Vec<SalvageCustody>) {
        if rows.is_empty() {
            self.by_domain.remove(&domain);
        } else {
            self.by_domain.insert(domain, rows);
        }
    }
}

/// **The pin a clamp starts** (doc 41 cut RS.8, Q-36-46; doc 35b §3, doc 35c §6.3): the
/// derelict's title as the economy holds it and everyone who answers for it, open space's
/// `GroundTenure::Unclaimed`, and the witnesses the bubble's presence rows name: the helm of
/// every other carrier in the tug's bubble. A derelict the economy has no title for is titled
/// first to whoever really held her (her helm, her raider leader, her last pilot, her owner);
/// one nobody held stays untitled, and her claim cannot be filed.
pub fn pin_at_clamp(
    ctx: &mut Ctx<'_>,
    derelict: ObjId,
    tug: ObjId,
    pinned_ic: i64,
) -> crate::surface::claim::TenurePin {
    let titled = ctx
        .w
        .module_state::<mush_economy::EconomyStore>()
        .is_some_and(|store| store.snapshot().asset(derelict).is_some());
    if !titled && let Some(holder) = super::tick::resolve_hull_prior_owner(ctx.w, derelict, tug) {
        ensure_space_hull_titled(ctx, derelict, holder);
    }
    let (title, answerers) = crate::surface::claim::title_of(ctx.w, derelict);
    let mut witnesses: Vec<crate::surface::claim::Witness> = Vec::new();
    if let Some(active) = ctx
        .w
        .module_state::<super::tick::SpaceBubbleBook>()
        .and_then(|book| book.active_for_hull(tug))
    {
        for (carrier, _, _, _) in active.all_carrier_hulls() {
            if carrier == tug || carrier == derelict {
                continue;
            }
            if let Some(holder) = crate::port::coop::helm_holder(ctx.w, carrier)
                && holder != ctx.actor
                && !witnesses.iter().any(|w| w.holder == holder)
            {
                witnesses.push(crate::surface::claim::Witness {
                    holder,
                    unit: carrier,
                    kind: crate::surface::claim::WitnessKind::Sensor,
                });
            }
        }
    }
    witnesses.truncate(crate::surface::claim::MAX_WITNESSES);
    crate::surface::claim::TenurePin {
        casualty: derelict,
        tug,
        recoverer: ctx.actor,
        answering: answerers.first().copied(),
        answerers: if matches!(title, Some(mush_org::PrincipalId::Object(_))) {
            Vec::new()
        } else {
            answerers
        },
        title,
        owner_grant: None,
        landholder_authorized: false,
        tenure: GroundTenure::Unclaimed,
        witnesses,
        pinned_ic,
    }
}

// ---- the crew's space seam (doc 42 cut CR.6, Q-42-24) ---------------------------------------

/// **The site a space domain's crew records name** (doc 42 cut CR.6, Q-42-24): the lane entry
/// whose map hosts the domain (a waypoint such as `kirkwood_depot`), else `space_domain_<n>`.
#[must_use]
pub fn space_site(w: &mush_core::World, domain: mush_sim::DomainId) -> Option<CanonicalId> {
    let host = w
        .module_state::<mush_sim::SpatialState>()
        .and_then(|state| state.domains.get(&domain))
        .map(|record| record.host);
    if let Some(host) = host {
        for (_, map) in crate::port::voyage::lane_ports(w) {
            if let Some(map) = map
                && map.host == Some(host.0)
            {
                return Some(map.entry.clone());
            }
        }
    }
    CanonicalId::new(format!("space_domain_{}", domain.0)).ok()
}

/// The domain `hull` flew in last: its bubble (a settled carrier keeps its ship row there), its
/// derelict record, the bubble it just left, or where it is placed.
fn space_domain_of(w: &mush_core::World, hull: ObjId) -> Option<mush_sim::DomainId> {
    if let Some(book) = w.module_state::<super::tick::SpaceBubbleBook>() {
        if let Some(active) = book.active_for_hull(hull) {
            return Some(active.domain);
        }
        if let Some((&domain, _)) = book
            .by_domain
            .iter()
            .find(|(_, active)| active.ship_hulls.values().any(|&h| h == hull))
        {
            return Some(domain);
        }
        if let Some(derelict) = book.derelicts.get(&hull) {
            return Some(derelict.domain);
        }
        // A carrier leaving a bubble checks in after the bubble's record is filed.
        if let Some(completed) = book.completed.get(&hull) {
            return Some(completed.domain);
        }
    }
    mush_sim::pose_of(w, hull).map(|(domain, _)| domain)
}

/// **The crew of `hull` checked in from space** (doc 42 cut CR.6, Q-42-24): every character
/// aboard gets the ground's record (`crate::field::crew::check_in`, with its rules: a record in
/// custody is left alone, a check-in never heals, a fallen hull strands its souls). A
/// `DisabledDerelict` strands them too: her crew are `Stranded` in her on the domain's site and
/// may ask `+sortie/extract`, as on a ground site.
pub fn check_in_space_crew(ctx: &mut Ctx<'_>, hull: ObjId, disabled: bool) {
    let souls = crate::field::carrier::souls_aboard(ctx.w, hull);
    if souls.is_empty() {
        return;
    }
    let Some(site) = space_domain_of(ctx.w, hull).and_then(|domain| space_site(ctx.w, domain))
    else {
        return;
    };
    for who in souls {
        crate::field::crew::check_in(ctx, &site, hull, Some(who));
        if !disabled {
            continue;
        }
        let Some(record) = crate::field::crew::crew_of(ctx.w, who).cloned() else {
            continue;
        };
        if record.location != (crate::field::crew::CrewLocation::Carrier { hull }) {
            continue;
        }
        crate::field::crew::put(
            ctx,
            crate::field::crew::CrewRecord {
                location: crate::field::crew::CrewLocation::Stranded {
                    site: site.clone(),
                    unit: hull,
                },
                ..record
            },
        );
        let line = crate::field::crew::STRANDED
            .replace("{unit}", &hull.0.to_string())
            .replace("{site}", &crate::surface::act::site_name(ctx.w, &site));
        ctx.out.sys_to_actor(who, "tacsim.field", line);
    }
}

/// **Whether `tug` hard-docking `derelict` is a friend's** (doc 42 cut CR.6; Q-42-24's
/// D-35-12): the ground's Friend rule (`crate::field::parties::friends`, as a ground load reads
/// it): one owner behind both, or an organization they openly share. The bubble's own relation
/// is not read: SF.R18 marks every co-participating carrier Friend for fire control, which says
/// nothing of who may take whom.
fn hard_dock_friendly(ctx: &Ctx<'_>, site: &CanonicalId, tug: ObjId, derelict: ObjId) -> bool {
    let entrants = crate::field::parties::entrants(ctx, site, &[derelict, tug]);
    let [a, b] = entrants.as_slice() else {
        return false;
    };
    if a.key == b.key {
        return true;
    }
    let Ok((parties, _)) = crate::field::parties::letter(&entrants) else {
        return false;
    };
    let letter = |key| parties.iter().find(|p| p.key == key).map(|p| p.party);
    let (Some(x), Some(y)) = (letter(a.key), letter(b.key)) else {
        return false;
    };
    crate::field::parties::friends(&entrants, &parties)
        .into_iter()
        .any(|pair| pair == (x, y) || pair == (y, x))
}

/// **A hard-dock on a derelict with crew** (doc 42 cut CR.6, Q-42-24; Q-42-1 (iii)): the
/// derelict's crew are checked in first (stranded in her), then a non-friend's clamp captures
/// them as a ground load does (`crate::field::crew::captured_loaded`: the seat vacated, the
/// body stepped into the tug's bay room, `Captured { carrier: tug, captor }`, both told, the
/// standing paid), and a friend's takes them aboard the tug
/// (`crate::field::crew::aboard_loaded`). The captives, in record order.
pub fn capture_at_hard_dock(ctx: &mut Ctx<'_>, tug: ObjId, derelict: ObjId) -> Vec<ObjId> {
    check_in_space_crew(ctx, derelict, true);
    // The record is the truth, as a ground load reads it: everyone it holds in the derelict.
    // (A clamp is rare; the characters are scanned through the public reader.)
    let crewed = ctx.w.iter().any(|object| {
        object.kind == mush_core::ObjKind::Player
            && crate::field::crew::crew_of(ctx.w, object.id)
                .is_some_and(|record| crate::field::crew::held_in(record, derelict))
    });
    if !crewed {
        return Vec::new();
    }
    let Some(room) = crate::port::ship::hull_view(ctx.w, tug).and_then(|view| {
        crate::port::vehicle::bay_rooms(ctx.w, &view)
            .into_iter()
            .next()
            .or_else(|| view.airlock())
    }) else {
        return Vec::new();
    };
    let Some(site) = space_domain_of(ctx.w, derelict).and_then(|domain| space_site(ctx.w, domain))
    else {
        return Vec::new();
    };
    if hard_dock_friendly(ctx, &site, tug, derelict) {
        crate::field::crew::aboard_loaded(ctx, derelict, tug, Some(room));
        return Vec::new();
    }
    crate::field::crew::captured_loaded(
        ctx,
        &crate::field::crew::LoadCapture {
            site: &site,
            ordinal: 0,
            casualty: derelict,
            carrier: tug,
            room,
            captor: ctx.actor,
        },
    )
}

/// Real UTC seconds now (`w.now()`), clamped to at least `1` (`SF.R19`, `35b` §6.8 #14).
#[must_use]
pub fn real_now_secs(w: &mush_core::World) -> i64 {
    w.now().max(1)
}

/// Configured salvage contest window in real seconds (`[tacsim.recovery].claim_window_s`),
/// clamped to `1..=MAX_SALVAGE_CLAIM_WINDOW_SECS` (`SF.R19`, `35b` §6.8 #15).
#[must_use]
pub fn salvage_claim_window_secs(w: &mush_core::World) -> i64 {
    w.module_state::<crate::store::TacsimStore>()
        .map_or(mush_economy::SALVAGE_CLAIM_WINDOW_SECS, |store| {
            store.config().recovery.claim_window_s
        })
        .clamp(1, mush_economy::MAX_SALVAGE_CLAIM_WINDOW_SECS)
}

/// Ensure `hull` carries an active asset title in [`mush_economy::EconomyStore`] owned by
/// `owner` (`SF.R19`, `Q-36-46`).
pub fn ensure_space_hull_titled(ctx: &mut Ctx<'_>, hull: ObjId, owner: ObjId) {
    let titled = ctx
        .w
        .module_state::<mush_economy::EconomyStore>()
        .is_some_and(|store| store.snapshot().asset(hull).is_some());
    if titled {
        return;
    }
    if let Some(issuer) = mush_org::SystemPrincipalId::new(crate::port::hull::REGISTRY_ISSUER) {
        let _ = mush_economy::issue_title(
            ctx,
            hull,
            mush_org::PrincipalId::Object(owner),
            None,
            issuer,
            None,
        );
    }
}

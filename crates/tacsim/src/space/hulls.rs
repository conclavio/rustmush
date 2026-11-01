//! **Canonical Space Hull Specifications & Slot-Stable Refit Bay Packages (`SF.16`)**
//! (`docs/design/35-ship-free-flight-combat-and-mining.md`, `docs/design/31-unit-designer-refit.md`).
//!
//! Loads the authored [`SpaceHullCatalog`] from `crates/tacsim/content/space/hulls.json`:
//! - **Five canonical space hulls** (`tacsim.unit.dray`, `tacsim.unit.dray_p`,
//!   `tacsim.unit.corvette`, `tacsim.unit.raider`, `tacsim.unit.halcyon`), each mapped
//!   to its `SF.14` baked 3D `mush-pixel` model (`freighter`, `corvette`, `raider`, `shipyard`)
//!   and slot-stable `refit_family` (`dray_hull`, `frigate_hull`, `carrier_hull`).
//! - **Five slot-stable Refit Bay work-order packages** (`refit.dray.prospector`,
//!   `refit.dray.expedition`, `refit.frigate.picket_screen`, `refit.frigate.strike_raider`,
//!   `refit.carrier.deep_space`) enforcing `doc 31` family compatibility (`D-31-1`..`D-31-16`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::space::bubble::{
    EmconPosture, FacetBand, HullStatus, ShipSectionId, ShipSectionState, SpacePartyId,
    SpaceShipState,
};
use crate::space::weapons::{LaserMount, PdcMount, RailgunMount, TorpedoTube};

const HULLS_JSON: &str = include_str!("../../content/space/hulls.json");

/// Schema header for `content/space/hulls.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceHullCatalogSchema {
    pub id: String,
    pub version: u32,
}

/// Authored specification for a spacecraft hull class (`SF.16`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceHullSpec {
    pub id: String,
    pub short_id: String,
    pub label: String,
    /// Slot-stable refit family (`dray_hull`, `frigate_hull`, `carrier_hull`, `doc 31`).
    pub refit_family: String,
    /// Corresponding `SF.14` baked 3D `mush-pixel` sprite sheet (`freighter`, `corvette`, `raider`, `shipyard`).
    pub pixel_model: String,
    pub dry_mass_t: u32,
    pub max_accel_mps2: i64,
    pub propellant_kg: u32,
    pub delta_v_mps: i64,
    pub return_reserve_dv_mps: i64,
    pub cooling_rate_permille_s: u16,
    pub hit_radius_m: i64,
    pub decoys: u8,
    /// `[front, left, right, rear, top]` hull facet armour values.
    pub hull_armor: [u32; 5],
    pub hull_internals: u32,
    pub hold_internals: u32,
    pub drive_internals: u32,
    pub torpedo_tubes: u8,
    pub torpedo_magazine: u16,
    pub laser_turrets: u8,
    pub railgun_rounds: u16,
    pub pdc_mounts: u8,
    pub pdc_ammo: u32,
}

impl SpaceHullSpec {
    /// Instantiate a live [`SpaceShipState`] from this authored hull specification.
    #[must_use]
    pub fn instantiate_ship(
        &self,
        id: impl Into<String>,
        callsign: impl Into<String>,
        party: SpacePartyId,
        pos_m: [i64; 3],
        vel_mmps: [i64; 3],
    ) -> SpaceShipState {
        let mut sections = BTreeMap::new();
        sections.insert(
            ShipSectionId::Hull,
            ShipSectionState {
                id: ShipSectionId::Hull,
                facets: BTreeMap::from([
                    (FacetBand::Front, self.hull_armor[0]),
                    (FacetBand::Left, self.hull_armor[1]),
                    (FacetBand::Right, self.hull_armor[2]),
                    (FacetBand::Rear, self.hull_armor[3]),
                    (FacetBand::Top, self.hull_armor[4]),
                ]),
                internals: self.hull_internals,
                max_internals: self.hull_internals,
            },
        );
        sections.insert(
            ShipSectionId::Hold,
            ShipSectionState {
                id: ShipSectionId::Hold,
                facets: BTreeMap::from([
                    (FacetBand::Left, (self.hull_armor[1] * 3) / 4),
                    (FacetBand::Right, (self.hull_armor[2] * 3) / 4),
                    (FacetBand::Top, (self.hull_armor[4] * 5) / 6),
                ]),
                internals: self.hold_internals,
                max_internals: self.hold_internals,
            },
        );
        sections.insert(
            ShipSectionId::Drive,
            ShipSectionState {
                id: ShipSectionId::Drive,
                facets: BTreeMap::new(),
                internals: self.drive_internals,
                max_internals: self.drive_internals,
            },
        );

        let torpedo_tubes = if self.torpedo_tubes > 0 {
            let mut tube = TorpedoTube::standard_twin("torp_bow", self.torpedo_magazine);
            tube.tubes = self.torpedo_tubes;
            vec![tube]
        } else {
            Vec::new()
        };

        let lasers = (0..self.laser_turrets)
            .map(|idx| LaserMount::standard_turret(format!("laser_{}", idx + 1)))
            .collect();

        let railguns = if self.railgun_rounds > 0 {
            vec![RailgunMount::standard_spinal(
                "rail_spinal",
                self.railgun_rounds,
            )]
        } else {
            Vec::new()
        };

        let pdcs = (0..self.pdc_mounts)
            .map(|idx| PdcMount::standard_rotary(format!("pdc_{}", idx + 1), self.pdc_ammo))
            .collect();

        SpaceShipState {
            id: id.into(),
            callsign: callsign.into(),
            hull_class: self.id.clone(),
            party,
            pos_m,
            vel_mmps,
            thrust_mps2: [0, 0, 0],
            max_accel_mps2: self.max_accel_mps2,
            attitude_deg: (0, 0, 0),
            propellant_kg: self.propellant_kg,
            delta_v_mps: self.delta_v_mps,
            return_reserve_dv_mps: self.return_reserve_dv_mps,
            heat_permille: 80,
            cooling_rate_permille_s: self.cooling_rate_permille_s,
            pdc_auto: true,
            crew_linkdead: false,
            emcon: EmconPosture::Standard,
            locked_target_id: None,
            hit_radius_m: self.hit_radius_m,
            sections,
            crits: Vec::new(),
            status: HullStatus::Operational,
            torpedo_tubes,
            lasers,
            railguns,
            pdcs,
            decoys_remaining: self.decoys,
            active_decoy_ms: 0,
        }
    }
}

/// Authored slot-stable Refit Bay work-order package (`doc 31`, `SF.16`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceRefitPackage {
    pub id: String,
    pub label: String,
    /// Required `refit_family` (`dray_hull`, `frigate_hull`, `carrier_hull`).
    pub refit_family: String,
    /// Resulting canonical hull ID after refit.
    pub target_hull_id: String,
    /// Quoted workshop cost in credits (`cr`).
    pub cost_cr: u64,
    /// Quoted workshop turnaround time in IC hours.
    pub work_hours: u32,
    pub add_delta_v_mps: i64,
    pub add_propellant_kg: u32,
    pub add_laser_turrets: u8,
    pub add_torpedo_magazine: u16,
    pub add_railgun_rounds: u16,
    pub add_pdc_ammo: u32,
    pub add_decoys: u8,
    pub add_cooling_permille_s: u16,
}

/// Complete catalog of authored space hulls and refit packages (`content/space/hulls.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceHullCatalog {
    pub schema: SpaceHullCatalogSchema,
    pub hulls: Vec<SpaceHullSpec>,
    pub refit_packages: Vec<SpaceRefitPackage>,
}

/// Receipt emitted when a [`SpaceRefitPackage`] is applied to a [`SpaceShipState`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceRefitReceipt {
    pub ship_id: String,
    pub package_id: String,
    pub refit_family: String,
    pub previous_hull_class: String,
    pub new_hull_class: String,
    pub cost_cr: u64,
    pub work_hours: u32,
    pub new_delta_v_mps: i64,
    pub new_propellant_kg: u32,
}

/// Refusal reasons when validating or applying a [`SpaceRefitPackage`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SpaceRefitError {
    #[error("unknown space hull class '{0}'")]
    UnknownHullClass(String),
    #[error("unknown space refit package '{0}'")]
    UnknownPackage(String),
    #[error(
        "refit family mismatch: hull '{hull_class}' belongs to family '{hull_family}', but package '{package_id}' requires '{required_family}'"
    )]
    FamilyMismatch {
        hull_class: String,
        hull_family: String,
        package_id: String,
        required_family: String,
    },
    #[error("cannot refit a disabled derelict hull without first restoring hull/drive structure")]
    HullDisabled,
}

impl SpaceHullCatalog {
    /// Load the compiled-in [`SpaceHullCatalog`] from `crates/tacsim/content/space/hulls.json`.
    #[must_use]
    pub fn embedded() -> Self {
        serde_json::from_str(HULLS_JSON).unwrap_or(Self {
            schema: SpaceHullCatalogSchema {
                id: "tacsim.space.hulls".to_owned(),
                version: 1,
            },
            hulls: Vec::new(),
            refit_packages: Vec::new(),
        })
    }

    /// Look up a [`SpaceHullSpec`] by canonical ID (`tacsim.unit.corvette`) or short ID (`corvette`).
    #[must_use]
    pub fn find_hull(&self, id_or_short: &str) -> Option<&SpaceHullSpec> {
        let needle = id_or_short.trim();
        self.hulls
            .iter()
            .find(|h| h.id.eq_ignore_ascii_case(needle) || h.short_id.eq_ignore_ascii_case(needle))
    }

    /// Look up a [`SpaceRefitPackage`] by package ID (`refit.dray.prospector`).
    #[must_use]
    pub fn find_package(&self, package_id: &str) -> Option<&SpaceRefitPackage> {
        let needle = package_id.trim();
        self.refit_packages
            .iter()
            .find(|p| p.id.eq_ignore_ascii_case(needle))
    }

    /// Apply a slot-stable [`SpaceRefitPackage`] to `ship` (`doc 31`, `SF.16`).
    ///
    /// # Errors
    /// Returns [`SpaceRefitError`] if `ship` is disabled, `package_id` is unknown, or
    /// `ship`'s `refit_family` does not match the package's `refit_family`.
    pub fn apply_refit(
        &self,
        ship: &mut SpaceShipState,
        package_id: &str,
    ) -> Result<SpaceRefitReceipt, SpaceRefitError> {
        if ship.status != HullStatus::Operational {
            return Err(SpaceRefitError::HullDisabled);
        }
        let pkg = self
            .find_package(package_id)
            .ok_or_else(|| SpaceRefitError::UnknownPackage(package_id.to_owned()))?;
        let hull_spec = self
            .find_hull(&ship.hull_class)
            .ok_or_else(|| SpaceRefitError::UnknownHullClass(ship.hull_class.clone()))?;

        if hull_spec.refit_family != pkg.refit_family {
            return Err(SpaceRefitError::FamilyMismatch {
                hull_class: ship.hull_class.clone(),
                hull_family: hull_spec.refit_family.clone(),
                package_id: pkg.id.clone(),
                required_family: pkg.refit_family.clone(),
            });
        }

        let prev_class = ship.hull_class.clone();
        ship.hull_class = pkg.target_hull_id.clone();
        ship.delta_v_mps = ship.delta_v_mps.saturating_add(pkg.add_delta_v_mps);
        ship.propellant_kg = ship.propellant_kg.saturating_add(pkg.add_propellant_kg);
        ship.cooling_rate_permille_s = ship
            .cooling_rate_permille_s
            .saturating_add(pkg.add_cooling_permille_s);
        ship.decoys_remaining = ship.decoys_remaining.saturating_add(pkg.add_decoys);

        for _ in 0..pkg.add_laser_turrets {
            let next_idx = ship.lasers.len() + 1;
            ship.lasers.push(LaserMount::standard_turret(format!(
                "laser_refit_{next_idx}"
            )));
        }
        if pkg.add_torpedo_magazine > 0 {
            if let Some(tube) = ship.torpedo_tubes.first_mut() {
                tube.magazine_remaining = tube
                    .magazine_remaining
                    .saturating_add(pkg.add_torpedo_magazine);
            } else {
                ship.torpedo_tubes.push(TorpedoTube::standard_twin(
                    "torp_refit",
                    pkg.add_torpedo_magazine,
                ));
            }
        }
        if pkg.add_railgun_rounds > 0 {
            if let Some(rg) = ship.railguns.first_mut() {
                rg.rounds_remaining = rg.rounds_remaining.saturating_add(pkg.add_railgun_rounds);
            } else {
                ship.railguns.push(RailgunMount::standard_spinal(
                    "rail_refit",
                    pkg.add_railgun_rounds,
                ));
            }
        }
        if pkg.add_pdc_ammo > 0 {
            for pdc in &mut ship.pdcs {
                pdc.ammo_remaining = pdc.ammo_remaining.saturating_add(pkg.add_pdc_ammo);
            }
        }

        Ok(SpaceRefitReceipt {
            ship_id: ship.id.clone(),
            package_id: pkg.id.clone(),
            refit_family: pkg.refit_family.clone(),
            previous_hull_class: prev_class,
            new_hull_class: ship.hull_class.clone(),
            cost_cr: pkg.cost_cr,
            work_hours: pkg.work_hours,
            new_delta_v_mps: ship.delta_v_mps,
            new_propellant_kg: ship.propellant_kg,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_hull_catalog_loads_all_5_hulls_and_enforces_slot_stable_refit_families() {
        let cat = SpaceHullCatalog::embedded();
        assert_eq!(cat.hulls.len(), 5);
        assert_eq!(cat.refit_packages.len(), 5);

        let dray_spec = cat.find_hull("dray");
        assert!(dray_spec.is_some());
        let corvette_spec = cat.find_hull("corvette");
        assert!(corvette_spec.is_some());
        let raider_spec = cat.find_hull("raider");
        assert!(raider_spec.is_some());
        let halcyon_spec = cat.find_hull("halcyon");
        assert!(halcyon_spec.is_some());

        let Some(dray_spec) = dray_spec else {
            return;
        };
        let mut dray =
            dray_spec.instantiate_ship("ship.1", "UNS-DRAY", SpacePartyId(1), [0, 0, 0], [0, 0, 0]);
        assert_eq!(dray.hull_class, "tacsim.unit.dray");
        assert_eq!(dray.lasers.len(), 1);

        // Refit Dray -> Dray-P Prospector (`dray_hull` family succeeds)
        let receipt = cat.apply_refit(&mut dray, "refit.dray.prospector");
        assert!(receipt.is_ok());
        assert_eq!(dray.hull_class, "tacsim.unit.dray_p");
        assert_eq!(dray.lasers.len(), 2);

        // Attempting to apply a frigate refit (`frigate_hull`) to a Dray (`dray_hull`) is refused!
        let bad = cat.apply_refit(&mut dray, "refit.frigate.strike_raider");
        assert!(matches!(bad, Err(SpaceRefitError::FamilyMismatch { .. })));

        // Instantiate a Raider (`frigate_hull`) and apply the strike refit
        let Some(raider_spec) = raider_spec else {
            return;
        };
        let mut raider = raider_spec.instantiate_ship(
            "ship.2",
            "CORS-RAIDER",
            SpacePartyId(2),
            [0, 0, 0],
            [0, 0, 0],
        );
        let ok_raider = cat.apply_refit(&mut raider, "refit.frigate.strike_raider");
        assert!(ok_raider.is_ok());
        assert_eq!(
            raider.torpedo_tubes.first().map(|t| t.magazine_remaining),
            Some(16)
        );
        assert_eq!(
            raider.railguns.first().map(|r| r.rounds_remaining),
            Some(32)
        );
    }
}

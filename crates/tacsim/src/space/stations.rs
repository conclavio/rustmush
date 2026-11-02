//! **Multicrew Bridge Stations, Linkdead Ballistic Safety, Light-Lag IR Sensing & Damage Control**
//! (Doc 35 cut `SF.15`; `docs/design/35-ship-free-flight-combat-and-mining.md` `D-35-7`, `D-35-9`, `D-35-10`).
//!
//! Implements:
//! 1. **Multicrew Bridge Seats ([`BridgeSeat`], [`ShipCrewRoster`], `D-35-9`)**:
//!    - Canonical stations: `command`, `helm`, `tactical`, `ew`, `engineering`, `flight_ops`.
//!    - Solo pilots occupy `command` by default and operate all channels directly; multi-player
//!      crews bind individual seats on the same hull (`+helm/station <ship_id> <seat>`) and
//!      share real-time telemetry across `space.holotank`, `space.cockpit`, and `space.docking`.
//! 2. **Linkdead Ballistic-Drift Rule (`D-35-7`, `Q-H5`, `P-2`)**:
//!    - When all seated crew on a ship go linkdead ([`ShipCrewRoster::all_crew_linkdead`]),
//!      main drive thrust immediately cuts to `[0, 0, 0]` (ballistic drift) while automatic
//!      point-defense (`pdc_auto`) remains armed for the reclaim grace window.
//! 3. **Light-Lag & Thermal/IR Contact Projection ([`compute_ir_signature_kw`], [`project_light_lag_contact`], `D-35-10`)**:
//!    - Computes a target's thermal/IR bloom from passive hull floor (`EmconPosture`),
//!      radiator heat (`heat_permille`), and main torch drive thrust (`thrust_mps2`).
//!    - Projects light-lagged apparent position $\mathbf{r}_{\text{app}} = \mathbf{r} - \mathbf{v} \cdot (d / c)$
//!      and inverse-square signal-to-noise ratio (`snr_db`) against observer sensor aperture.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::ObjId;
use serde::{Deserialize, Serialize};

use crate::space::bubble::{EmconPosture, HullStatus, SpaceShipState, SubsystemCrit};
use crate::space::weapons::isqrt_i128;

/// Speed of light in vacuum in metres per second ($c = 299{,}792{,}458\text{ m/s}$, `D-35-10`).
pub const SPEED_OF_LIGHT_MPS: i64 = 299_792_458;

/// Canonical multicrew bridge station roles (`doc 10 §8`, `D-35-9`).
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum BridgeSeat {
    /// Solo captain or bridge commander; authorized to operate all stations.
    #[default]
    Command,
    /// Attitude, main torch burns, velocity matching, decoupling, and autopilot (`+helm`).
    Helm,
    /// Point-defense, offensive/defensive lasers, torpedoes, and railguns (`+tac`).
    Tactical,
    /// Emissions control (`EMCON`), active radar/IR decoys, and light-lag sensor scans (`+tac/emcon`, `+tac/decoy`, `+tac/scan`, `+mine/ping`).
    Ew,
    /// Open-cycle coolant blowdown, subsystem crit repair, and drop-tank rigging (`+helm/coolant`, `+helm/repair`, `+helm/droptanks`).
    Engineering,
    /// Collector drone sorties, docking collar operations, and salvage tow rigging (`+mine/drones`, `+helm/dock`, `+helm/tow`).
    FlightOps,
}

impl BridgeSeat {
    /// Canonical lowercase identifier string.
    #[must_use]
    pub fn id_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Helm => "helm",
            Self::Tactical => "tactical",
            Self::Ew => "ew",
            Self::Engineering => "engineering",
            Self::FlightOps => "flight_ops",
        }
    }

    /// Parse a station name from user input.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "command" | "cmd" | "captain" | "cap" => Some(Self::Command),
            "helm" | "pilot" | "nav" => Some(Self::Helm),
            "tactical" | "tac" | "weapons" | "gunner" => Some(Self::Tactical),
            "ew" | "sensors" | "comms" | "ops" => Some(Self::Ew),
            "engineering" | "eng" | "damage" | "dc" => Some(Self::Engineering),
            "flight_ops" | "flight" | "drones" | "deck" => Some(Self::FlightOps),
            _ => None,
        }
    }

    /// Parse an [`EmconPosture`] from user input (`+tac/emcon`).
    #[must_use]
    pub fn parse_emcon(raw: &str) -> Option<EmconPosture> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "ghost" | "dark" | "0" => Some(EmconPosture::Ghost),
            "passive" | "quiet" | "1" => Some(EmconPosture::Passive),
            "std" | "standard" | "normal" | "2" => Some(EmconPosture::Standard),
            "combat" | "active" | "full" | "3" => Some(EmconPosture::ActiveCombat),
            _ => None,
        }
    }

    /// Parse a [`SubsystemCrit`] target for `+helm/repair`.
    #[must_use]
    pub fn parse_crit(raw: &str) -> Option<SubsystemCrit> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "radiator" | "rad" | "cooling" => Some(SubsystemCrit::RadiatorDamaged),
            "drive" | "torch" | "engine" => Some(SubsystemCrit::DriveDegraded),
            "pdc" | "ciws" | "turret" => Some(SubsystemCrit::PdcOffline),
            "sensor" | "sensors" | "radar" => Some(SubsystemCrit::SensorDegraded),
            _ => None,
        }
    }

    /// Check whether this bridge seat is authorized to execute `verb` (`+helm`, `+tac`, `+mine`)
    /// with optional `switch` (`D-35-9`).
    ///
    /// Read-only status queries (`switch == None`) and `+helm/station` / `+helm/linkdead`
    /// are always permitted for any seated crew member.
    #[must_use]
    pub fn permits(self, verb: &str, switch: Option<&str>) -> bool {
        if self == Self::Command {
            return true;
        }
        let Some(sw) = switch else {
            return true;
        };
        if sw == "station" || sw == "linkdead" {
            return true;
        }
        match (verb, sw) {
            ("+helm", "coolant" | "repair" | "droptanks") => {
                matches!(self, Self::Engineering | Self::Helm)
            }
            ("+helm", "dock" | "tow") => matches!(self, Self::FlightOps | Self::Helm),
            ("+helm", _) => self == Self::Helm,
            ("+tac", "emcon" | "decoy" | "scan") => matches!(self, Self::Ew | Self::Tactical),
            ("+tac", _) => self == Self::Tactical,
            ("+mine", "ping") => matches!(self, Self::Ew | Self::FlightOps),
            ("+mine", "drones") => self == Self::FlightOps,
            ("+mine", _) => matches!(self, Self::FlightOps | Self::Helm),
            _ => false,
        }
    }
}

/// Multicrew seat assignments and linkdead state for a single spacecraft (`D-35-7`, `D-35-9`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShipCrewRoster {
    /// Seated crew actors mapped to their assigned [`BridgeSeat`].
    pub seats: BTreeMap<ObjId, BridgeSeat>,
    /// Subset of seated crew actors currently marked linkdead (`D-35-7`).
    pub linkdead: BTreeSet<ObjId>,
}

impl ShipCrewRoster {
    /// Assign `actor` to `seat` on this ship (clearing any prior linkdead flag for `actor`).
    pub fn assign_seat(&mut self, actor: ObjId, seat: BridgeSeat) {
        self.seats.insert(actor, seat);
        self.linkdead.remove(&actor);
    }

    /// Query `actor`'s assigned [`BridgeSeat`], defaulting to [`BridgeSeat::Command`] for
    /// solo pilots not yet explicitly assigned a specialist seat (`D-35-9`).
    #[must_use]
    pub fn seat_for(&self, actor: ObjId) -> BridgeSeat {
        self.seats
            .get(&actor)
            .copied()
            .unwrap_or(BridgeSeat::Command)
    }

    /// Mark `actor` as linkdead or reconnected (`D-35-7`), returning `true` if **all** seated
    /// crew on the hull are now linkdead.
    pub fn set_linkdead(&mut self, actor: ObjId, is_linkdead: bool) -> bool {
        self.seats.entry(actor).or_insert(BridgeSeat::Command);
        if is_linkdead {
            self.linkdead.insert(actor);
        } else {
            self.linkdead.remove(&actor);
        }
        self.all_crew_linkdead()
    }

    /// Returns `true` when the ship has seated crew and every seated crew member is linkdead (`D-35-7`).
    #[must_use]
    pub fn all_crew_linkdead(&self) -> bool {
        !self.seats.is_empty() && self.seats.keys().all(|k| self.linkdead.contains(k))
    }

    /// Apply the `D-35-7` linkdead ballistic-drift safety rule to `ship`:
    /// if all crew are linkdead, main thrust cuts to `[0, 0, 0]` while `pdc_auto` remains armed!
    pub fn sync_ship_linkdead_state(&self, ship: &mut SpaceShipState) {
        let all_ld = self.all_crew_linkdead();
        ship.crew_linkdead = all_ld;
        if all_ld {
            ship.thrust_mps2 = [0, 0, 0];
            // Point defense stays armed during linkdead reclaim grace window (`D-35-7`).
            ship.pdc_auto = true;
        }
    }
}

/// Light-lagged thermal/IR and radar contact projection (`D-35-10`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LightLagContact {
    pub target_id: String,
    pub callsign: String,
    /// True instantaneous 3D distance in metres.
    pub true_distance_m: i64,
    /// Speed-of-light propagation delay $\Delta t = \lfloor d / c \rfloor$ in milliseconds.
    pub light_lag_ms: u64,
    /// Apparent position $\mathbf{r}_{\text{app}} = \mathbf{r} - \mathbf{v} \cdot \Delta t$ at light emission time.
    pub apparent_pos_m: [i64; 3],
    /// Target thermal/IR + radar output in kilowatts (`kW`).
    pub ir_signature_kw: u64,
    /// Observer signal-to-noise ratio in decibels (`dB`); contact is resolved when `snr_db >= 0`.
    pub snr_db: i32,
    /// Whether the observer's sensors resolve the contact above the noise floor.
    pub detected: bool,
}

/// Compute a spacecraft's thermal/IR + active emissions signature in kilowatts (`kW`, `D-35-10`).
///
/// Combines:
/// - Passive hull & avionics floor governed by [`EmconPosture`] (`250 kW` in `Ghost` up to `2,400 kW` in `ActiveCombat`)
/// - Radiator thermal plume proportional to `heat_permille` (doubled if `RadiatorDamaged` forces open-loop venting)
/// - Actinic fusion torch drive bloom (`45,000 kW` per $\text{m/s}^2$ of main thrust acceleration)
#[must_use]
pub fn compute_ir_signature_kw(ship: &SpaceShipState) -> u64 {
    if ship.status != HullStatus::Operational {
        // Cold drifting derelict hull floor
        return 120;
    }
    let emcon_floor_kw: u64 = match ship.emcon {
        EmconPosture::Ghost => 250,
        EmconPosture::Passive => 600,
        EmconPosture::Standard => 1_200,
        EmconPosture::ActiveCombat => 2_400,
    };

    let rad_mult: u64 = if ship.crits.contains(&SubsystemCrit::RadiatorDamaged) {
        30
    } else if ship.emcon == EmconPosture::Ghost {
        4
    } else {
        15
    };
    let radiator_kw = u64::from(ship.heat_permille).saturating_mul(rad_mult);

    let ax = ship.thrust_mps2[0] as i128;
    let ay = ship.thrust_mps2[1] as i128;
    let az = ship.thrust_mps2[2] as i128;
    let accel_mag = if ship.crew_linkdead {
        0
    } else {
        isqrt_i128(ax * ax + ay * ay + az * az) as u64
    };
    let torch_bloom_kw = accel_mag.saturating_mul(45_000);

    let decoy_bloom_kw = if ship.active_decoy_ms > 0 { 180_000 } else { 0 };

    emcon_floor_kw
        .saturating_add(radiator_kw)
        .saturating_add(torch_bloom_kw)
        .saturating_add(decoy_bloom_kw)
}

/// Project a target spacecraft through light-lag (`SPEED_OF_LIGHT_MPS`) and inverse-square
/// thermal/radar detection relative to `observer` (`D-35-10`).
#[must_use]
pub fn project_light_lag_contact(
    observer: &SpaceShipState,
    target: &SpaceShipState,
) -> LightLagContact {
    let dx = (target.pos_m[0] - observer.pos_m[0]) as i128;
    let dy = (target.pos_m[1] - observer.pos_m[1]) as i128;
    let dz = (target.pos_m[2] - observer.pos_m[2]) as i128;
    let dist_m = isqrt_i128(dx * dx + dy * dy + dz * dz).max(1) as i64;

    // Light-lag in milliseconds: dt_ms = (dist_m * 1000) / c
    let light_lag_ms =
        ((dist_m as u128).saturating_mul(1_000) / (SPEED_OF_LIGHT_MPS as u128)) as u64;

    // Apparent position when the light left the target: r_app = r - v * dt
    let dt_i128 = i128::from(light_lag_ms);
    let apparent_pos_m = [
        target.pos_m[0].saturating_sub(((target.vel_mmps[0] as i128 * dt_i128) / 1_000_000) as i64),
        target.pos_m[1].saturating_sub(((target.vel_mmps[1] as i128 * dt_i128) / 1_000_000) as i64),
        target.pos_m[2].saturating_sub(((target.vel_mmps[2] as i128 * dt_i128) / 1_000_000) as i64),
    ];

    let ir_signature_kw = compute_ir_signature_kw(target);

    // Observer aperture gain: ActiveCombat boosts gain, SensorDegraded halves aperture
    let aperture_gain: i128 = match observer.emcon {
        EmconPosture::ActiveCombat => 150,
        EmconPosture::Standard => 100,
        EmconPosture::Passive => 85,
        EmconPosture::Ghost => 65,
    };
    let crit_factor: i128 = if observer.crits.contains(&SubsystemCrit::SensorDegraded) {
        40
    } else {
        100
    };

    let dist_km = (dist_m / 1_000).max(1) as i128;
    // Flux metric: (ir_kw * aperture * crit_factor * 500_000) / (dist_km^2)
    let numerator = (ir_signature_kw as i128)
        .saturating_mul(aperture_gain)
        .saturating_mul(crit_factor)
        .saturating_mul(500_000);
    let denominator = dist_km.saturating_mul(dist_km).max(1);
    let flux_ratio = numerator / denominator;

    // Convert flux_ratio around detection threshold (flux_ratio >= 8 => >= 0 dB)
    let snr_db = if flux_ratio <= 0 {
        -20
    } else {
        let log2_approx = (127 - flux_ratio.leading_zeros() as i32) - 3;
        log2_approx * 3
    };
    let detected = snr_db >= 0 || dist_m <= 25_000;

    LightLagContact {
        target_id: target.id.clone(),
        callsign: target.callsign.clone(),
        true_distance_m: dist_m,
        light_lag_ms,
        apparent_pos_m,
        ir_signature_kw,
        snr_db,
        detected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::space::bubble::{LocalSpaceBubble, SpaceCombatEvent, SpacePartyId, SpaceRelation};

    #[test]
    fn multicrew_seats_enforce_station_permissions_and_linkdead_ballistic_cut() {
        let mut roster = ShipCrewRoster::default();
        let helm_actor = ObjId(101);
        let tac_actor = ObjId(102);
        let eng_actor = ObjId(103);
        let ew_actor = ObjId(104);

        roster.assign_seat(helm_actor, BridgeSeat::Helm);
        roster.assign_seat(tac_actor, BridgeSeat::Tactical);
        roster.assign_seat(eng_actor, BridgeSeat::Engineering);
        roster.assign_seat(ew_actor, BridgeSeat::Ew);

        assert!(roster.seat_for(helm_actor).permits("+helm", Some("burn")));
        assert!(!roster.seat_for(helm_actor).permits("+tac", Some("torp")));
        assert!(roster.seat_for(tac_actor).permits("+tac", Some("torp")));
        assert!(roster.seat_for(eng_actor).permits("+helm", Some("coolant")));
        assert!(roster.seat_for(eng_actor).permits("+helm", Some("repair")));
        assert!(roster.seat_for(ew_actor).permits("+tac", Some("emcon")));
        assert!(roster.seat_for(ew_actor).permits("+tac", Some("decoy")));

        let mut ship = SpaceShipState::new_corvette(
            "halcyon",
            "UNS-HALCYON",
            SpacePartyId(1),
            [0, 0, 0],
            [1_000_000, 0, 0],
        );
        ship.thrust_mps2 = [40, 0, 0];
        ship.pdc_auto = false;

        // Three of four crew go linkdead -> ship still has one live crew member
        assert!(!roster.set_linkdead(helm_actor, true));
        assert!(!roster.set_linkdead(tac_actor, true));
        assert!(!roster.set_linkdead(eng_actor, true));
        roster.sync_ship_linkdead_state(&mut ship);
        assert!(!ship.crew_linkdead);
        assert_eq!(ship.thrust_mps2, [40, 0, 0]);

        // Final crew member goes linkdead -> D-35-7 cuts thrust to ballistic drift and arms PDCs
        assert!(roster.set_linkdead(ew_actor, true));
        roster.sync_ship_linkdead_state(&mut ship);
        assert!(ship.crew_linkdead);
        assert_eq!(ship.thrust_mps2, [0, 0, 0]);
        assert!(ship.pdc_auto);

        // Helm reconnects -> linkdead clears
        assert!(!roster.set_linkdead(helm_actor, false));
        roster.sync_ship_linkdead_state(&mut ship);
        assert!(!ship.crew_linkdead);
    }

    #[test]
    fn decoy_spoofs_incoming_torpedo_and_engineering_flushes_heat_and_repairs_crits() {
        let mut bubble = LocalSpaceBubble::new("b.ew", "kirkwood_gap_3_1", "EW TEST", 1_000);
        let shooter = SpaceShipState::new_corvette(
            "raider",
            "CORS-1",
            SpacePartyId(2),
            [25_000, 0, 0],
            [-200_000, 0, 0],
        );
        let mut defender =
            SpaceShipState::new_dray("dray", "DRAY-1", SpacePartyId(1), [0, 0, 0], [0, 0, 0]);
        // Disable PDCs and lasers so only the EW decoy can save the Dray!
        defender.pdc_auto = false;
        defender.lasers.clear();
        defender.heat_permille = 700;
        defender.crits.push(SubsystemCrit::RadiatorDamaged);
        defender.crits.push(SubsystemCrit::PdcOffline);
        if let Some(pdc) = defender.pdcs.first_mut() {
            pdc.operational = false;
        }
        bubble.insert_ship(shooter);
        bubble.insert_ship(defender);
        bubble.set_party_relation(SpacePartyId(1), SpacePartyId(2), SpaceRelation::Hostile);

        // Launch 1 torpedo from 25 km
        let launched = bubble.fire_torpedo_salvo("raider", "dray", 1);
        assert_eq!(launched.len(), 1);

        // Deploy ECM decoy canister
        let rem = bubble.launch_decoy("dray");
        assert_eq!(rem, Ok(3));

        // Step 1 second -> torpedo is inside 35 km decoy envelope and gets spoofed!
        let report = bubble.step_wall_ms(1_000);
        assert!(
            report
                .events
                .iter()
                .any(|e| matches!(e, SpaceCombatEvent::TorpedoSpoofedByDecoy { defender_id, .. } if defender_id == "dray")),
            "expected TorpedoSpoofedByDecoy, got {:?}",
            report.events
        );
        assert!(bubble.torpedoes.is_empty());

        // Engineering flushes coolant and repairs radiator & PDC crits
        let blowdown = bubble.blowdown_coolant("dray");
        assert!(matches!(
            blowdown,
            Ok(SpaceCombatEvent::CoolantBlowdown { new_heat_permille, .. }) if new_heat_permille < 300
        ));
        assert!(
            bubble
                .repair_subsystem_crit("dray", SubsystemCrit::RadiatorDamaged)
                .is_ok()
        );
        assert!(
            bubble
                .repair_subsystem_crit("dray", SubsystemCrit::PdcOffline)
                .is_ok()
        );
        let dray = bubble.ships.get("dray");
        assert!(dray.is_some_and(|s| s.crits.is_empty() && s.pdcs.iter().all(|p| p.operational)));
    }

    #[test]
    fn light_lag_and_ir_bloom_reflect_emcon_and_torch_burns() {
        let observer =
            SpaceShipState::new_corvette("obs", "PICKET-1", SpacePartyId(1), [0, 0, 0], [0, 0, 0]);
        // Target 600,000 km away (~2.0 seconds of light lag) moving at +10,000 m/s along Y
        let mut target = SpaceShipState::new_dray(
            "tgt",
            "GHOST-RUNNER",
            SpacePartyId(2),
            [600_000_000, 20_000, 0],
            [0, 10_000_000, 0],
        );
        target.emcon = EmconPosture::Ghost;
        target.heat_permille = 20;

        let ghost_contact = project_light_lag_contact(&observer, &target);
        assert!((1_990..=2_010).contains(&ghost_contact.light_lag_ms));
        // Apparent Y position lags behind true Y (20,000 m) by ~20,000 m -> near 0 m!
        assert!(ghost_contact.apparent_pos_m[1] < 500);
        assert!(
            !ghost_contact.detected,
            "cold ghost target at 600,000 km should be below SNR threshold"
        );

        // Target lights its main fusion torch at 25 m/s^2 -> massive IR bloom!
        target.thrust_mps2 = [25, 0, 0];
        let burning_contact = project_light_lag_contact(&observer, &target);
        assert!(burning_contact.ir_signature_kw > ghost_contact.ir_signature_kw * 500);
        assert!(
            burning_contact.detected,
            "torch bloom should be detected even across light-seconds"
        );
    }
}

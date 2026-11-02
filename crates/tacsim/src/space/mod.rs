//! **Pure Space Kinematics, Weapons, 5-Facet Armor & `TacticalUnit` Custody Library**
//! (`docs/design/35-ship-free-flight-combat-and-mining.md` Draft 0.3,
//! `docs/design/35a-space-flight-review.md` §8.2–8.3).
//!
//! Exports the deterministic 1:1 wall-clock [`LocalSpaceBubble`] kinematics/combat
//! stepper, [`BubbleSnapshot`] and [`BubbleInputTape`] replay primitives, weapon
//! mounts and in-flight ordnance ([`weapons`]), pure 6-DOF docking gate predicates
//! ([`DockingPose6Dof`]), and bidirectional `TacticalUnit` $\leftrightarrow$
//! [`SpaceShipState`] cell/condition mapping ([`custody`]).

use serde::{Deserialize, Serialize};

pub mod bubble;
pub mod commands;
pub mod custody;
pub mod docking;
pub mod mining;
pub mod picket;
pub mod tick;
pub mod weapons;

pub use bubble::{
    BubbleId, BubbleInputCommand, BubbleInputEntry, BubbleInputTape, BubbleSnapshot,
    BubbleStepReport, DERELICT_WINDOW_S, EmconPosture, FacetBand, HullStatus, LocalSpaceBubble,
    MAX_SUBSTEP_MS, ShipSectionId, ShipSectionState, SpaceCombatEvent, SpacePartyId, SpaceRelation,
    SpaceShipState, SubsystemCrit,
};
pub use commands::{
    BridgeRole, HELM_HELP, HELM_SWITCHES, HELM_VERB, HOLOTANK_FEED, MINE_HELP, MINE_SWITCHES,
    MINE_VERB, NO_BRIDGE_STATION, NO_SUCH_CONTACT, NOT_ABOARD_SHIP, SPACE_COCKPIT_FEED,
    SPACE_DOCKING_FEED, SpaceContactObservation, TAC_HELP, TAC_SWITCHES, TAC_VERB,
    emcon_level_for_posture, emcon_posture_for_level, evaluate_space_contact_detection,
    heading_order_vector, helm_cmd, hull_aboard_for_actor, is_hull_captain, mine_cmd,
    observe_bubble_contacts, parse_emcon_token, seat_role_for_actor, tac_cmd,
};
pub use custody::{
    ALL_FACET_BANDS, ALL_SHIP_SECTIONS, apply_unit_spec_to_space_ship,
    build_space_damage_mutation_plan, check_in_space_unit, check_out_space_unit,
    format_tactical_unit_damage_line, instantiate_stock_space_unit,
    record_space_combat_to_black_box, register_derelict_salvage_in_book,
    resolve_tactical_section_id, space_ship_from_tactical_unit,
    space_ship_from_tactical_unit_with_spec, unit_spec_for_id,
};
pub use docking::{
    COLLAR_SCRAPE_FRONT_ARMOR_DAMAGE, DOCK_GATE_MAX_ANG_RATE_MDEG_S, DOCK_GATE_MAX_LAT_VEL_MMPS,
    DOCK_GATE_RANGE_Z_MM, DOCKING_FPS, DockingChallenge, DockingControlInput, DockingGateStatus,
    DockingInputTape, DockingOutcome, DockingRcsAuthority, DockingReplayReport, DockingSession,
    DockingStepperState, DockingTapeError, DockingTapeSpan, DockingTargetKind, SalvageTowRecord,
    evaluate_docking_gates, parse_dock_text_order, replay_docking_tape, solve_docking_tape,
    solve_hard_bounce_tape,
};
pub use mining::{
    ActiveClusterMiningSession, ClusterRockSpec, ClusterSiteState, ClusterTenureOrder, ClusterZone,
    DRAY_P_HOLD_CAPACITY_KG, DRAY_P_HOLD_CAPACITY_T, DRONE_SORTIE_IC_SECS, DRONE_SORTIE_YIELD_T,
    DroneSortieState, FRONTIER_LASER_BLOOM_HEAT, LASER_CYCLE_IC_SECS, LASER_CYCLE_YIELD_T,
    LaserExtractionCycle, MAX_MINING_MATCH_RANGE_M, MAX_VELOCITY_MATCH_SPEED_MMPS,
    OrbitalClusterId, PendingSpaceMineDraw, SpectralClass, build_updated_pocket_cursor,
    check_velocity_match_gate, ore_specific_volume_l_per_t,
};
pub use picket::{
    BELT_RAIDER_PARTY, BELT_RAIDER_RHAI, CUSTOMS_CORVETTE_PARTY, CUSTOMS_FINE_CONTRACT_NAME,
    CUSTOMS_FINE_MAJOR_PER_TONNE, CUSTOMS_FINE_MIN_MAJOR, HEAVE_TO_REL_VEL_MMPS,
    KIRKWOOD_PICKET_RHAI, KirkwoodPicketState, LiveCustomsManifest, PICKET_RUN_BOUNDARY_M,
    PicketInspectionOutcome, PicketInspectionStatus, RAIDER_AMBUSH_RANGE_M,
    assess_customs_fine_and_advance_front, build_kirkwood_picket_ships, customs_fine_spec,
    distance_3d_m, emit_light_lagged_picket_hail, inspect_hull_manifest,
    record_ran_the_picket_incident, relative_speed_3d_mmps, run_space_rhai_think,
    should_open_kirkwood_picket, step_kirkwood_picket,
};
pub use tick::{
    ActiveInterdictionBubble, CARRIER_PARTY, COMBAT_STEP_WALL_MS, CRUISE_STEP_WALL_MS,
    CompletedInterdictionBubble, FreeFlightGuidanceMode, HullPropellantLedger,
    INTERDICTION_STANDOFF_M, INTERDICTOR_PARTY, ORDNANCE_STEP_WALL_MS, SOLAR_MU_KM3_S2,
    SPACE_STATE_FORMAT, SPACE_STATE_KEY, SPACE_TICK, SPACE_TICK_PERIOD, SpaceBubbleBook,
    SpaceDerelictRecord, SpaceFreeFlightRecord, SpacePersistedState, apply_bubble_command,
    boot_read, clear_interdiction_bubble, persist_space_state,
};
pub use weapons::{
    DEFAULT_TORPEDO_MAX_FLIGHT_MS, InFlightSlug, InFlightTorpedo, LaserMode, LaserMount, PdcMount,
    RailgunMount, TorpedoTube, compute_railgun_lead,
};

/// Maximum standoff range (metres) to initiate a 6-DOF docking approach (`<= 500 m`).
pub const MAX_DOCKING_INIT_RANGE_M: i64 = 500;

/// Maximum relative speed (m/s) to initiate a 6-DOF docking approach (`<= 5 m/s`).
pub const MAX_DOCKING_INIT_REL_SPEED_MPS: i64 = 5;

/// Lateral alignment gate (`|x| <= 250 mm`, `|y| <= 250 mm`; `D-35-14`).
pub const DOCK_GATE_LATERAL_MM: i64 = 250;

/// Attitude alignment gate (`|pitch| <= 2,000 mdeg`, `|yaw| <= 2,000 mdeg`, i.e. `<= 2.0 deg`).
pub const DOCK_GATE_ATTITUDE_MDEG: i32 = 2_000;

/// Roll alignment gate (`|roll_err| <= 3,000 mdeg`, i.e. `<= 3.0 deg`).
pub const DOCK_GATE_ROLL_MDEG: i32 = 3_000;

/// Minimum forward closure speed at collar contact (`>= 100 mm/s`, i.e. `0.10 m/s`; `D-35-14`).
pub const DOCK_GATE_CLOSURE_MIN_MMPS: i64 = 100;

/// Maximum safe forward closure speed at collar contact (`<= 800 mm/s`, i.e. `0.80 m/s`).
pub const DOCK_GATE_CLOSURE_MAX_MMPS: i64 = 800;

/// Six-degree-of-freedom relative pose and rates between the ship's docking collar
/// and the target collar (`doc 35` §8.1, `Q-35-5`, `D-35-14`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockingPose6Dof {
    /// Lateral horizontal offset from target collar centreline in millimetres (`0` = centred).
    pub lateral_x_mm: i64,
    /// Lateral vertical offset from target collar centreline in millimetres (`0` = centred).
    pub lateral_y_mm: i64,
    /// Forward range to target collar contact plane in millimetres (`0` = contact).
    pub range_z_mm: i64,
    /// Lateral X velocity in mm/s.
    pub vel_x_mmps: i64,
    /// Lateral Y velocity in mm/s.
    pub vel_y_mmps: i64,
    /// Closure rate toward collar in mm/s (positive = closing, reducing `range_z_mm`).
    pub closure_mmps: i64,
    /// Pitch error relative to collar normal in millidegrees.
    pub pitch_err_mdeg: i32,
    /// Yaw error relative to collar normal in millidegrees.
    pub yaw_err_mdeg: i32,
    /// Roll error relative to target collar rotation in millidegrees.
    pub roll_err_mdeg: i32,
    /// Pitch angular rate in millidegrees/s.
    pub pitch_rate_mdeg_s: i32,
    /// Yaw angular rate in millidegrees/s.
    pub yaw_rate_mdeg_s: i32,
    /// Roll angular rate in millidegrees/s.
    pub roll_rate_mdeg_s: i32,
}

impl DockingPose6Dof {
    /// Return whether lateral translation is inside the alignment gate (`<= 500 mm`).
    #[must_use]
    pub const fn lateral_aligned(&self) -> bool {
        self.lateral_x_mm.abs() <= DOCK_GATE_LATERAL_MM
            && self.lateral_y_mm.abs() <= DOCK_GATE_LATERAL_MM
    }

    /// Return whether pitch and yaw attitude errors are inside the gate (`<= 2.0 deg`).
    #[must_use]
    pub const fn attitude_aligned(&self) -> bool {
        self.pitch_err_mdeg.abs() <= DOCK_GATE_ATTITUDE_MDEG
            && self.yaw_err_mdeg.abs() <= DOCK_GATE_ATTITUDE_MDEG
    }

    /// Return whether roll alignment error is inside the gate (`<= 3.0 deg`).
    #[must_use]
    pub const fn roll_aligned(&self) -> bool {
        self.roll_err_mdeg.abs() <= DOCK_GATE_ROLL_MDEG
    }

    /// Return whether closure speed is inside the safe capture band (`50..=800 mm/s`).
    #[must_use]
    pub const fn closure_safe(&self) -> bool {
        self.closure_mmps >= DOCK_GATE_CLOSURE_MIN_MMPS
            && self.closure_mmps <= DOCK_GATE_CLOSURE_MAX_MMPS
    }

    /// Return whether all 5 pose/rate gates are simultaneously green (`doc 35` §8.1).
    #[must_use]
    pub const fn pose_aligned(&self) -> bool {
        self.lateral_aligned()
            && self.attitude_aligned()
            && self.roll_aligned()
            && self.closure_safe()
            && self.vel_x_mmps.abs() <= 300
            && self.vel_y_mmps.abs() <= 300
    }

    /// Return whether the collar is at contact range (`range_z_mm <= 500 mm`) with all
    /// alignment gates green.
    #[must_use]
    pub const fn ready_to_clamp(&self) -> bool {
        self.range_z_mm <= 500 && self.pose_aligned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::ObjId;

    #[test]
    fn moving_shooter_railgun_lead_hits_steady_target_and_misses_on_jink() {
        let mut bubble = LocalSpaceBubble::new(
            "bubble.railgun_lead",
            "site.test",
            "Railgun Lead Test",
            1_000,
        );
        bubble.set_party_relation(SpacePartyId(1), SpacePartyId(2), SpaceRelation::Hostile);

        // Both shooter and target have significant 3D world velocities (`35a` §5.1).
        let mut shooter = SpaceShipState::new_corvette(
            "ship.shooter",
            "Corvette",
            SpacePartyId(1),
            [1_500, -400, 250],
            [5_000_000, -2_000_000, 1_000_000], // +5 km/s X, -2 km/s Y, +1 km/s Z
        );
        shooter.railguns = vec![RailgunMount::standard_spinal("rail.1", 8)];
        bubble.insert_ship(shooter);

        let target = SpaceShipState::new_dray(
            "ship.target",
            "Target",
            SpacePartyId(2),
            [75_500, 1_600, -750],
            [-1_000_000, 3_000_000, -500_000], // -1 km/s X, +3 km/s Y, -0.5 km/s Z
        );
        bubble.insert_ship(target);

        let fired = bubble.fire_railgun("ship.shooter", "ship.target");
        assert_eq!(
            fired.len(),
            1,
            "moving shooter must compute a valid intercept lead"
        );

        // Branch A: target holds velocity -> slug hits!
        let mut steady_bubble = bubble.clone();
        let steady_report = steady_bubble.step_wall_ms_at(8_000, 1_192);
        assert!(
            steady_report.events.iter().any(|e| matches!(
                e,
                SpaceCombatEvent::RailgunHit { target_id, .. } if target_id == "ship.target"
            )),
            "railgun fired from moving shooter must hit steady-velocity target: {:?}",
            steady_report.events
        );

        // Branch B: target executes lateral jink burn -> slug misses (`RailgunDodgedByJink`).
        let mut jink_bubble = bubble;
        assert!(
            jink_bubble
                .command_burn("ship.target", [0, 25, 0], true)
                .is_ok()
        );
        let jink_report = jink_bubble.step_wall_ms_at(8_000, 1_192);
        assert!(
            jink_report.events.iter().any(|e| matches!(
                e,
                SpaceCombatEvent::RailgunDodgedByJink { target_id, .. } if target_id == "ship.target"
            )),
            "lateral jink burn during time-of-flight must dodge the unguided slug: {:?}",
            jink_report.events
        );
    }

    #[test]
    fn sub_metre_position_remainder_accumulates_slow_velocity_across_250ms_substeps() {
        let mut bubble =
            LocalSpaceBubble::new("bubble.submetre", "site.test", "Sub-Metre Remainder", 2_000);
        // Dray-P moving at 0.49 m/s (490 mm/s) after 1 s of 0.49 m/s^2 acceleration.
        // Without `pos_rem_um`, `(490 * 250) / 1_000_000 == 0` on every 250 ms substep (`35a` §5.2).
        let dray = SpaceShipState::new_dray(
            "ship.dray",
            "Dray-P",
            SpacePartyId(1),
            [0, 0, 0],
            [490, 0, 0],
        );
        bubble.insert_ship(dray);

        // Step 10 seconds in 250 ms increments: `490 mm/s * 10 s = 4.9 m` (`pos_m[0] == 4`, `pos_rem_um[0] == 900_000`).
        for step in 1..=40 {
            bubble.step_wall_ms_at(250, 2_000 + step * 6);
        }
        let ship = bubble.ships.get("ship.dray").expect("dray present");
        assert_eq!(ship.pos_m[0], 4);
        assert_eq!(ship.pos_rem_um[0], 900_000);
    }

    #[test]
    fn torpedo_3d_proportional_navigation_and_ballistic_coasting_after_dv_exhaustion() {
        let mut bubble =
            LocalSpaceBubble::new("bubble.torp_pn", "site.test", "Torpedo PN & Coast", 3_000);
        bubble.set_party_relation(SpacePartyId(1), SpacePartyId(2), SpaceRelation::Hostile);

        let attacker = SpaceShipState::new_corvette(
            "ship.attacker",
            "Attacker",
            SpacePartyId(1),
            [0, 0, 0],
            [0, 0, 0],
        );
        let mut target = SpaceShipState::new_dray(
            "ship.target",
            "Target",
            SpacePartyId(2),
            [12_000, 1_500, 0],
            [0, 180_000, 0], // moving laterally at +180 m/s Y
        );
        target.pdcs.clear();
        target.lasers.clear();
        bubble.insert_ship(attacker);
        bubble.insert_ship(target);

        // Give the torpedo a small delta-v budget (`400 m/s`) so it burns out mid-flight,
        // emits `TorpedoExhaustedDeltaV` once, and coasts ballistically to impact (`35a` §5.9).
        assert_eq!(
            bubble
                .fire_torpedo_salvo("ship.attacker", "ship.target", 1)
                .len(),
            1
        );
        assert_eq!(
            bubble.torpedoes[0].max_flight_ms,
            DEFAULT_TORPEDO_MAX_FLIGHT_MS
        );
        bubble.torpedoes[0].remaining_dv_mmps = 400_000;
        bubble.torpedoes[0].vel_mmps = [1_200_000, 330_000, 0];

        let report = bubble.step_wall_ms_at(12_000, 3_288);
        let exhausted_count = report
            .events
            .iter()
            .filter(|e| matches!(e, SpaceCombatEvent::TorpedoExhaustedDeltaV { .. }))
            .count();
        assert_eq!(
            exhausted_count, 1,
            "torpedo must emit TorpedoExhaustedDeltaV exactly once when fuel reaches 0: {:?}",
            report.events
        );
        assert!(
            report.events.iter().any(|e| matches!(
                e,
                SpaceCombatEvent::TorpedoHit { target_id, .. } if target_id == "ship.target"
            )),
            "torpedo must coast ballistically after fuel exhaustion and still impact on-course target: {:?}",
            report.events
        );

        // Long-range band check (doc 35 §6.2: 10–500 km under the 90 s flight cap):
        // At 450 km (450,000 m), a standard torpedo (250 m/s^2, 12 km/s delta-v) boosts
        // for 48 s (288 km) and coasts at 12 km/s for ~13.5 s (162 km), impacting around 61.5 s.
        let mut long_bubble =
            LocalSpaceBubble::new("bubble.long_torp", "site.test", "Long-Range Torpedo", 5_000);
        long_bubble.set_party_relation(SpacePartyId(1), SpacePartyId(2), SpaceRelation::Hostile);
        long_bubble.insert_ship(SpaceShipState::new_corvette(
            "ship.attacker",
            "Attacker",
            SpacePartyId(1),
            [0, 0, 0],
            [0, 0, 0],
        ));
        let mut far_target = SpaceShipState::new_dray(
            "ship.far_target",
            "Far Target",
            SpacePartyId(2),
            [450_000, 0, 0],
            [0, 0, 0],
        );
        far_target.pdcs.clear();
        far_target.lasers.clear();
        long_bubble.insert_ship(far_target);
        assert_eq!(
            long_bubble
                .fire_torpedo_salvo("ship.attacker", "ship.far_target", 1)
                .len(),
            1
        );
        let long_report = long_bubble.step_wall_ms_at(70_000, 6_680);
        assert!(
            long_report.events.iter().any(|e| matches!(
                e,
                SpaceCombatEvent::TorpedoHit { target_id, .. } if target_id == "ship.far_target"
            )),
            "450 km torpedo must boost and coast to impact within the 90 s flight cap: {:?}",
            long_report.events
        );
    }

    #[test]
    fn pdc_burst_cooldown_accumulates_hits_allows_salvo_saturation_and_decoy_checks_target_lock() {
        let mut bubble =
            LocalSpaceBubble::new("bubble.pdc_decoy", "site.test", "PDC & Decoy", 4_000);
        bubble.set_party_relation(SpacePartyId(1), SpacePartyId(2), SpaceRelation::Hostile);

        let attacker = SpaceShipState::new_corvette(
            "ship.attacker",
            "Attacker",
            SpacePartyId(1),
            [0, 0, 0],
            [1_500_000, 0, 0], // closing at 1.5 km/s
        );
        let mut defender = SpaceShipState::new_dray(
            "ship.defender",
            "Defender",
            SpacePartyId(2),
            [2_500, 0, 0],
            [0, 0, 0],
        );
        // Single PDC mount (`cooldown_ms = 500`, `damage_per_burst = 50`), lasers off:
        // Two bursts (at 250 ms and 750 ms) accumulate 100 damage to destroy the first
        // torpedo (`integrity: 100 -> 50 -> 0`), while the second torpedo in the salvo
        // penetrates before the single PDC can cycle two more bursts (`35a` §5.8).
        defender.pdcs = vec![PdcMount::standard_rotary("pdc.1", 2_000)];
        defender.lasers.clear();

        // Friendly escort deploys a decoy nearby, but the torpedoes are locked onto `ship.defender`.
        // The escort's decoy must NOT spoof torpedoes locked onto `ship.defender` (`35a` §5.8).
        let mut escort = SpaceShipState::new_dray(
            "ship.escort",
            "Escort",
            SpacePartyId(2),
            [2_400, 50, 0],
            [0, 0, 0],
        );
        escort.pdcs.clear();
        escort.lasers.clear();

        bubble.insert_ship(attacker);
        bubble.insert_ship(defender);
        bubble.insert_ship(escort);

        assert!(bubble.launch_decoy("ship.escort").is_ok());
        assert_eq!(
            bubble
                .fire_torpedo_salvo("ship.attacker", "ship.defender", 2)
                .len(),
            2
        );

        // First 250 ms substep: PDC fires burst 1 (`50` damage) at torpedo 1 (`integrity: 100 -> 50`),
        // so neither torpedo is destroyed on the first burst (`deterministic hit accumulation`).
        let first_step = bubble.step_wall_ms_at(250, 4_006);
        assert!(
            !first_step
                .events
                .iter()
                .any(|e| matches!(e, SpaceCombatEvent::TorpedoInterceptedByPdc { .. })),
            "single 50-damage PDC burst must not one-shot a 100-integrity torpedo"
        );
        assert_eq!(bubble.torpedoes.len(), 2);
        assert_eq!(bubble.torpedoes[0].integrity, 50);
        assert_eq!(bubble.torpedoes[1].integrity, 100);

        let report = bubble.step_wall_ms_at(2_000, 4_054);
        let pdc_intercepts = report
            .events
            .iter()
            .filter(|e| matches!(e, SpaceCombatEvent::TorpedoInterceptedByPdc { .. }))
            .count();
        let torpedo_hits = report
            .events
            .iter()
            .filter(|e| matches!(e, SpaceCombatEvent::TorpedoHit { target_id, .. } if target_id == "ship.defender"))
            .count();
        let decoy_spoofs = report
            .events
            .iter()
            .filter(|e| matches!(e, SpaceCombatEvent::TorpedoSpoofedByDecoy { .. }))
            .count();

        assert_eq!(
            decoy_spoofs, 0,
            "escort decoy must not spoof torpedoes locked onto ship.defender"
        );
        assert_eq!(
            pdc_intercepts, 1,
            "single PDC on 500 ms cooldown accumulates 2 bursts (100 damage) to intercept 1 torpedo"
        );
        assert_eq!(
            torpedo_hits, 1,
            "second torpedo in simultaneous salvo must penetrate the single PDC's cooldown window"
        );
    }

    #[test]
    fn step_wall_ms_at_tracks_fractional_seconds_enforces_monotonic_ic_stamp_and_clamps_i64_max() {
        let mut bubble =
            LocalSpaceBubble::new("bubble.clock", "site.test", "Clock & Overflow", 10_000);
        let mut ship = SpaceShipState::new_dray(
            "ship.derelict",
            "Derelict",
            SpacePartyId(1),
            [0, 0, 0],
            [0, 0, 0],
        );
        // Verify `i64::MAX` thrust components do not overflow in `command_burn` (`35a` §5.14).
        bubble.insert_ship(ship.clone());
        assert!(
            bubble
                .command_burn("ship.derelict", [i64::MAX, i64::MIN, i64::MAX], true)
                .is_ok()
        );
        let clamped_thrust = bubble.ships["ship.derelict"].thrust_mps2;
        let clamped_mag_sq = i128::from(clamped_thrust[0]) * i128::from(clamped_thrust[0])
            + i128::from(clamped_thrust[1]) * i128::from(clamped_thrust[1])
            + i128::from(clamped_thrust[2]) * i128::from(clamped_thrust[2]);
        let max_mag_sq = i128::from(ship.max_accel_mps2) * i128::from(ship.max_accel_mps2);
        assert!(clamped_mag_sq <= max_mag_sq);

        // Mark ship derelict and step by non-1000ms-divisor increments (`300 ms`, `400 ms`, `300 ms`).
        ship.status = HullStatus::DisabledDerelict {
            epoch_open: true,
            life_support_remaining_s: 1_000,
            salvage_contest_window_s: 2_000,
        };
        bubble.insert_ship(ship);

        bubble.step_wall_ms_at(300, 10_007);
        bubble.step_wall_ms_at(400, 10_017);
        bubble.step_wall_ms_at(300, 10_024);
        assert_eq!(bubble.wall_elapsed_ms, 1_000);
        assert_eq!(bubble.ic_timestamp_s, 10_024);
        match bubble.ships["ship.derelict"].status {
            HullStatus::DisabledDerelict {
                life_support_remaining_s,
                salvage_contest_window_s,
                ..
            } => {
                assert_eq!(life_support_remaining_s, 999);
                assert_eq!(salvage_contest_window_s, 1_999);
            }
            ref other => panic!("expected DisabledDerelict, got {other:?}"),
        }

        // Verify `step_wall_ms_at` stamps the caller's authoritative W-clock (`35a` §5.3)
        // and enforces a monotonic guard when handed an earlier IC timestamp.
        bubble.step_wall_ms_at(500, 74_152_600_000);
        assert_eq!(bubble.ic_timestamp_s, 74_152_600_000);
        bubble.step_wall_ms_at(250, 74_152_500_000);
        assert_eq!(
            bubble.ic_timestamp_s, 74_152_600_000,
            "monotonic guard must prevent a rewound IC instant from moving ic_timestamp_s backward"
        );
        bubble.step_wall_ms_at(0, 74_152_400_000);
        assert_eq!(bubble.ic_timestamp_s, 74_152_600_000);
    }

    #[test]
    fn bubble_input_tape_and_snapshot_replay_deterministically() {
        let mut bubble = LocalSpaceBubble::new("bubble.replay", "site.test", "Tape Replay", 50_000);
        bubble.set_party_relation(SpacePartyId(1), SpacePartyId(2), SpaceRelation::Hostile);
        bubble.insert_ship(SpaceShipState::new_corvette(
            "ship.alpha",
            "Alpha",
            SpacePartyId(1),
            [0, 0, 0],
            [100_000, 0, 0],
        ));
        bubble.insert_ship(SpaceShipState::new_corvette(
            "ship.beta",
            "Beta",
            SpacePartyId(2),
            [2_500, 200, 0],
            [-150_000, 0, 0],
        ));

        let mut tape = BubbleInputTape::new(&bubble);
        let mut live_events = Vec::new();

        live_events.extend(tape.record_and_apply(
            &mut bubble,
            BubbleInputCommand::FireTorpedoSalvo {
                shooter_id: "ship.alpha".to_owned(),
                target_id: "ship.beta".to_owned(),
                count: 2,
            },
        ));
        live_events.extend(tape.record_and_apply(
            &mut bubble,
            BubbleInputCommand::StepWall {
                dt_wall_ms: 1_000,
                ic_now_s: 50_001,
            },
        ));

        // Capture a mid-battle snapshot after the first 2 tape entries (`start_entry_idx == 2`).
        let mid_snapshot = bubble.snapshot();
        let mid_idx = tape.entries.len();

        live_events.extend(tape.record_and_apply(
            &mut bubble,
            BubbleInputCommand::FireOffensiveLasers {
                shooter_id: "ship.alpha".to_owned(),
                target_id: "ship.beta".to_owned(),
            },
        ));
        live_events.extend(tape.record_and_apply(
            &mut bubble,
            BubbleInputCommand::StepWall {
                dt_wall_ms: 1_500,
                ic_now_s: 50_003,
            },
        ));

        let final_hash = bubble.state_hash();

        // Full replay from initial snapshot must produce identical state hash and events.
        let (replayed_full, full_events) = tape.replay();
        assert_eq!(replayed_full.state_hash(), final_hash);
        assert_eq!(full_events, live_events);

        // Mid-battle resume from `mid_snapshot` must also reach the exact same final state hash.
        let (replayed_mid, _mid_events) = tape.replay_from_snapshot(mid_snapshot, mid_idx);
        assert_eq!(replayed_mid.state_hash(), final_hash);
    }

    #[test]
    fn tactical_unit_custody_roundtrips_section_and_facet_damage() {
        let hull_obj = ObjId(4201);
        let (unit_before, unit_spec) =
            instantiate_stock_space_unit(hull_obj, "tacsim.unit.dray").expect("stock dray unit");
        let mut ship = space_ship_from_tactical_unit(
            &unit_before,
            "ship.dray_custody",
            "CSS Custody",
            SpacePartyId(1),
            [0, 0, 0],
            [0, 0, 0],
        )
        .expect("ship from tactical unit");

        // Canonical `dray.json` logistics (`D-35-1`, `35a` §1 row 2 & §6 row `SF.16/19`):
        assert_eq!(ship.dry_mass_t, 240);
        assert_eq!(ship.payload_ceiling_kg, 181_000);
        assert_eq!(ship.propellant_cap_kg, 80_000);
        assert_eq!(ship.rated_accel_mg, 50);
        assert!((ship.rated_accel_mps2() - 0.490_332_5).abs() < 1e-4);
        assert_eq!(ship.exhaust_kms, 6_000);

        // Inflict combat damage to rear drive armor and drive internals, plus a DriveDegraded crit.
        if let Some(drive_sec) = ship.sections.get_mut(&ShipSectionId::Drive) {
            drive_sec.facets.insert(FacetBand::Rear, 0);
            drive_sec.internals = 12;
        }
        ship.crits.push(SubsystemCrit::DriveDegraded);

        let (unit_after, assessment, plan) =
            build_space_damage_mutation_plan(&unit_before, &ship, Some(&unit_spec), 60_000)
                .expect("damage mutation plan");
        assert!(plan.is_some());
        assert_eq!(
            assessment.state,
            crate::field_ops::OperationalState::Operational
        );
        let dmg_line = format_tactical_unit_damage_line(&unit_after).expect("damage line");
        assert!(
            dmg_line.contains("drive 12/"),
            "unexpected damage line: {dmg_line}"
        );

        // Also verify `dray_p.json` and `halcyon.json` canonical `UnitSpec` mapping (`D-35-1`).
        let (dray_p_unit, _) = instantiate_stock_space_unit(ObjId(4202), "tacsim.unit.dray_p")
            .expect("stock dray_p unit");
        let dray_p_ship = space_ship_from_tactical_unit(
            &dray_p_unit,
            "ship.dray_p",
            "CSS Prospector",
            SpacePartyId(1),
            [0, 0, 0],
            [0, 0, 0],
        )
        .expect("dray_p ship");
        assert_eq!(dray_p_ship.dry_mass_t, 240);
        assert_eq!(dray_p_ship.propellant_cap_kg, 80_000);
        assert_eq!(dray_p_ship.rated_accel_mg, 50);

        let (halcyon_unit, halcyon_spec) =
            instantiate_stock_space_unit(ObjId(4203), "tacsim.unit.halcyon")
                .expect("stock halcyon unit");
        let mut halcyon_ship = space_ship_from_tactical_unit(
            &halcyon_unit,
            "ship.halcyon",
            "CNS Halcyon",
            SpacePartyId(1),
            [0, 0, 0],
            [0, 0, 0],
        )
        .expect("halcyon ship");
        assert_eq!(halcyon_ship.dry_mass_t, 4_000);
        assert_eq!(halcyon_ship.payload_ceiling_kg, 1_920_000);
        assert_eq!(halcyon_ship.propellant_cap_kg, 1_800_000);
        assert_eq!(halcyon_ship.rated_accel_mg, 50);
        assert_eq!(
            halcyon_ship.sections[&ShipSectionId::Hull].internals,
            12_000
        );
        assert_eq!(
            halcyon_ship.sections[&ShipSectionId::Drive].internals,
            3_000
        );

        // Disable Halcyon's `power` section (`ShipSectionId::Drive` -> `"power"` on `halcyon.json`)
        halcyon_ship
            .sections
            .get_mut(&ShipSectionId::Drive)
            .expect("halcyon drive/power")
            .internals = 0;
        halcyon_ship.status = HullStatus::DisabledDerelict {
            epoch_open: true,
            life_support_remaining_s: DERELICT_WINDOW_S,
            salvage_contest_window_s: DERELICT_WINDOW_S,
        };
        let (halcyon_after, halcyon_assessment, halcyon_plan) = build_space_damage_mutation_plan(
            &halcyon_unit,
            &halcyon_ship,
            Some(&halcyon_spec),
            61_000,
        )
        .expect("halcyon damage plan");
        assert!(halcyon_plan.is_some());
        assert_eq!(
            halcyon_assessment.state,
            crate::field_ops::OperationalState::Disabled
        );
        assert_eq!(
            halcyon_assessment.epoch_disposition,
            crate::field_ops::FieldEpochDisposition::RemainOpen
        );
        let halcyon_dmg = format_tactical_unit_damage_line(&halcyon_after).expect("halcyon damage");
        assert!(
            halcyon_dmg.contains("power destroyed"),
            "unexpected halcyon damage line: {halcyon_dmg}"
        );
    }

    #[test]
    fn docking_input_tape_replays_60hz_gates_rejects_uncommanded_and_detects_collar_scrape() {
        let challenge = DockingChallenge::standard_port("dock.4201.1");

        // 1. Starting pose must be outside all 5 alignment gates so an uncommanded tape never auto-wins (`35a` §1, §4).
        let start_gates = evaluate_docking_gates(&challenge.start_pose);
        assert_eq!(start_gates.green_count(), 0);
        let default_tape = DockingInputTape::from_frames(&[DockingControlInput::default(); 200]);
        let default_report =
            replay_docking_tape(&challenge, &default_tape).expect("replay default tape");
        assert!(
            matches!(default_report.outcome, DockingOutcome::OutOfGate { .. }),
            "uncommanded default tape must be rejected as OutOfGate, got {:?}",
            default_report.outcome
        );

        // 2. Closed-loop pilot solver produces a bounded RLE tape (`<= 3,600` frames) that clamps with all 5 gates green.
        let valid_tape = solve_docking_tape(&challenge);
        assert!(valid_tape.total_frames() <= super::docking::MAX_DOCKING_TAPE_FRAMES);
        let rle = valid_tape.encode_rle();
        let decoded = DockingInputTape::decode_rle(&rle).expect("decode RLE");
        assert_eq!(decoded, valid_tape);

        let valid_report = replay_docking_tape(&challenge, &decoded).expect("replay valid tape");
        assert_eq!(valid_report.outcome, DockingOutcome::Clamped);
        assert!(valid_report.final_gates.all_green());
        assert!(valid_report.rcs_spent_kg >= 1);

        // 3. Hard closure impact (`> 800 mm/s` into `range_z_mm == 0`) triggers CollarScrapeBounce + Front armor damage.
        let bounce_tape = solve_hard_bounce_tape(&challenge);
        let bounce_report =
            replay_docking_tape(&challenge, &bounce_tape).expect("replay hard bounce tape");
        assert!(
            matches!(
                bounce_report.outcome,
                DockingOutcome::CollarScrapeBounce {
                    impact_closure_mmps,
                    front_armor_damage,
                } if impact_closure_mmps > DOCK_GATE_CLOSURE_MAX_MMPS && front_armor_damage >= COLLAR_SCRAPE_FRONT_ARMOR_DAMAGE
            ),
            "expected CollarScrapeBounce, got {:?}",
            bounce_report.outcome
        );
    }

    #[test]
    fn space_persisted_state_round_trips_json_with_bubble_relations_standing_orders_and_salvage_book()
     {
        let mut book = SpaceBubbleBook::default();
        let hull = mush_core::ObjId(4201);
        let interdictor = mush_core::ObjId(9901);
        let domain = mush_sim::DomainId(7);

        let mut bubble = LocalSpaceBubble::new(
            "bubble.4201.1",
            "space.domain.7",
            "Kirkwood Gap Picket",
            74_152_600_000,
        );
        bubble.insert_ship(SpaceShipState::new_dray(
            "ship.4201",
            "Salt Moth",
            SpacePartyId(1),
            [0, 0, 0],
            [0, 0, 0],
        ));
        bubble.insert_ship(SpaceShipState::new_corvette(
            "ship.9901",
            "Belt Raider",
            SpacePartyId(3),
            [18_000, 0, 0],
            [-250_000, 0, 0],
        ));
        bubble.set_party_relation(SpacePartyId(1), SpacePartyId(3), SpaceRelation::Hostile);

        let mut tape = BubbleInputTape::new(&bubble);
        let _ = tape.record_and_apply(
            &mut bubble,
            BubbleInputCommand::StepWall {
                dt_wall_ms: 1_000,
                ic_now_s: 74_152_600_024,
            },
        );
        let snap = bubble.snapshot();
        book.by_hull.insert(
            hull,
            ActiveInterdictionBubble {
                hull,
                interdictor_hull: interdictor,
                domain,
                carrier_ship_id: "ship.4201".to_owned(),
                interdictor_ship_id: "ship.9901".to_owned(),
                leg_cursor: 1,
                opened_ic_s: 74_152_600_000,
                steps: 1,
                cleared: false,
                bubble,
                tape,
                snapshots: vec![snap],
                picket: Some(KirkwoodPicketState::new(
                    mush_core::ObjId(9801),
                    "ship.9801".to_owned(),
                    interdictor,
                    "ship.9901".to_owned(),
                )),
            },
        );

        let _ = book
            .record_standing_order(
                hull,
                mush_sim::tick::OrderChannel::Thrust,
                mush_sim::tick::OrderValue::Scalar { value: 500 },
                mush_core::ObjId(101),
                domain,
                1,
                10,
            )
            .expect("standing order");

        let mut salvage = crate::field_ops::SalvageClaimBook::new();
        let op_id = crate::field_ops::RecoveryOpId::new(1).expect("op_id");
        let title_status = register_derelict_salvage_in_book(
            &mut salvage,
            op_id,
            interdictor,
            hull,
            hull,
            interdictor,
            mush_core::ObjId(500),
            74_152_600_024,
            1_000,
        )
        .expect("salvage claim");
        book.derelicts.insert(
            interdictor,
            SpaceDerelictRecord {
                hull: interdictor,
                domain,
                disabled_at_ic_s: 74_152_600_024,
                epoch_open: true,
                life_support_remaining_s: DERELICT_WINDOW_S,
                salvage_contest_window_s: DERELICT_WINDOW_S,
                assessment: None,
                salvage_title_status: Some(title_status),
            },
        );

        let state = SpacePersistedState {
            format: SPACE_STATE_FORMAT,
            book,
            salvage,
        };
        let encoded = serde_json::to_value(&state).expect("encode SpacePersistedState");
        let decoded: SpacePersistedState =
            serde_json::from_value(encoded).expect("decode SpacePersistedState");
        assert_eq!(decoded, state);
    }
}

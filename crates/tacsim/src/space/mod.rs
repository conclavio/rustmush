//! **Space bubbles, 1:1 wall-time weapon triad, and the Kirkwood Customs Picket**
//! (Doc 35 cut `SF.3`; `docs/design/35-ship-free-flight-combat-and-mining.md` §5–§6, §9).
//!
//! This module implements Tier-2 local-space domains ([`LocalSpaceBubble`]),
//! the 1:1 wall-time weapon triad ([`TorpedoTube`], [`LaserMount`], [`RailgunMount`],
//! [`PdcMount`]), and the multi-party Kirkwood Customs Picket encounter
//! ([`KirkwoodPicketEncounter`]), reusing `dray.json`'s canonical sections
//! (`hull`, `hold`, `drive`) and facet bands (`front`, `left`, `right`, `rear`, `top`)
//! while keeping zero file overlap with `arena/*`, `field/*`, `surface/*`, and
//! `detachment/*` (`D-35-1`, `D-35-2`).

pub mod bubble;
pub mod docking;
pub mod mining;
pub mod picket;
pub mod weapons;

pub use bubble::{
    BubbleId, BubbleStepReport, DERELICT_WINDOW_S, EmconPosture, FacetBand, HullStatus,
    LocalSpaceBubble, MAX_SUBSTEP_MS, ShipSectionId, ShipSectionState, SpaceCombatEvent,
    SpacePartyId, SpaceRelation, SpaceShipState, SubsystemCrit,
};
pub use docking::{
    DOCK_GATE_ATTITUDE_MDEG, DOCK_GATE_CLOSURE_MAX_MMPS, DOCK_GATE_CLOSURE_MIN_MMPS,
    DOCK_GATE_LATERAL_MM, DOCK_GATE_ROLL_MDEG, DockingControlInput, DockingError, DockingOutcome,
    DockingPose6Dof, DockingSession, DockingSessionState, DockingTargetKind,
    MAX_DOCKING_INIT_RANGE_M, MAX_DOCKING_INIT_REL_SPEED_MPS, SALVAGE_CONTEST_WINDOW_IC_SECS,
    SalvageTowRecord,
};
pub use mining::{
    ASTEROID_CONTEST_WINDOW_IC_SECS, AsteroidClaimState, AsteroidClusterField, AsteroidOrePool,
    AsteroidPocket, AsteroidSpectralClass, ClusterJurisdiction, CollectorDrone, DroneDoctrinePhase,
    ExcursionId, ExtractionReport, MAX_DRONES_PER_DRAY_P, MAX_MINING_RANGE_M,
    MAX_MINING_REL_VEL_MPS, OreGoodId, OreHoldState, OreMiningError, PROSPECTING_LICENCE_FEE_CR,
    ProspectingLicence,
};
pub use picket::{
    CUSTOMS_SHIP_ID, HAULER_SHIP_ID, HEAVE_TO_MAX_REL_MPS, HaulerManifest, KirkwoodPicketEncounter,
    PARTY_CUSTOMS, PARTY_HAULER, PARTY_RAIDER, PicketOutcome, PicketPhase, RAIDER_SHIP_ID,
};
pub use weapons::{
    InFlightSlug, InFlightTorpedo, LaserMode, LaserMount, PdcMount, RailgunMount, TorpedoTube,
    compute_railgun_lead,
};

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    type TestResult = Result<(), Box<dyn Error>>;

    #[test]
    fn torpedo_inherits_launcher_velocity_and_is_intercepted_by_auto_pdc_or_defense_laser()
    -> TestResult {
        let mut bubble = LocalSpaceBubble::new(
            "bubble.test_torp",
            "kirkwood_gap_3_1",
            "TORPEDO TEST",
            1_000,
        );

        let shooter = SpaceShipState::new_corvette(
            "raider",
            "KESTREL",
            SpacePartyId(2),
            [80_000, 0, 0],
            [-120_000, 15_000, -5_000],
        );
        let mut defender = SpaceShipState::new_corvette(
            "defender",
            "VIGILANT",
            SpacePartyId(1),
            [0, 0, 0],
            [0, 0, 0],
        );
        defender.pdc_auto = true;
        bubble.insert_ship(shooter);
        bubble.insert_ship(defender);
        bubble.set_party_relation(SpacePartyId(1), SpacePartyId(2), SpaceRelation::Hostile);

        // Toggle defender's second laser into Defense mode (`D-35-8`)
        check!(bubble.set_laser_mode("defender", "laser_2", LaserMode::Defense));

        let launch_events = bubble.fire_torpedo_salvo("raider", "defender", 2);
        check_eq!(launch_events.len(), 2);
        check_eq!(bubble.torpedoes.len(), 2);

        // Verify exact inheritance of launcher's 3D velocity vector (`D-35-8`)
        for torp in &bubble.torpedoes {
            check_eq!(torp.vel_mmps, [-120_000, 15_000, -5_000]);
        }

        // Advance 1:1 wall time in 1 s steps up to 35 s; verify both torpedoes are intercepted
        // by the defender's defensive laser and automatic PDC screen before hitting the hull!
        let mut pdc_intercepts = 0usize;
        let mut laser_intercepts = 0usize;
        let mut hits = 0usize;
        for _ in 0..35 {
            let rep = bubble.step_wall_ms(1_000);
            check_eq!(rep.substeps_executed, 4);
            for ev in rep.events {
                match ev {
                    SpaceCombatEvent::TorpedoInterceptedByPdc { .. } => pdc_intercepts += 1,
                    SpaceCombatEvent::TorpedoInterceptedByLaser { .. } => laser_intercepts += 1,
                    SpaceCombatEvent::TorpedoHit { .. } => hits += 1,
                    _ => {}
                }
            }
            if bubble.torpedoes.is_empty() {
                break;
            }
        }

        check_eq!(
            hits,
            0,
            "auto-PDC and defensive laser should screen a 2-torp salvo"
        );
        check!(
            pdc_intercepts + laser_intercepts == 2,
            "expected both torpedoes to be intercepted"
        );
        Ok(())
    }

    #[test]
    fn railgun_cannot_be_intercepted_by_pdc_and_is_dodged_by_lateral_jink_burn() -> TestResult {
        // Case A: Non-maneuvering target gets hit despite active PDCs and defensive lasers
        let mut bubble_hit =
            LocalSpaceBubble::new("bubble.rg_hit", "pallas_inclined_belt", "RG HIT", 2_000);
        let shooter = SpaceShipState::new_corvette(
            "shooter",
            "CORVETTE",
            SpacePartyId(1),
            [100_000, 0, 0],
            [0, 0, 0],
        );
        let mut target = SpaceShipState::new_corvette(
            "target",
            "TARGET",
            SpacePartyId(2),
            [0, 0, 0],
            [0, 50_000, 0],
        );
        target.pdc_auto = true;
        bubble_hit.insert_ship(shooter.clone());
        bubble_hit.insert_ship(target.clone());
        bubble_hit.set_party_relation(SpacePartyId(1), SpacePartyId(2), SpaceRelation::Hostile);
        check!(bubble_hit.set_laser_mode("target", "laser_1", LaserMode::Defense));

        let fired = bubble_hit.fire_railgun("shooter", "target");
        check_eq!(fired.len(), 1);
        let report_hit = bubble_hit.step_wall_ms(6_000);
        let hit_count = report_hit
            .events
            .iter()
            .filter(|e| matches!(e, SpaceCombatEvent::RailgunHit { .. }))
            .count();
        check_eq!(
            hit_count,
            1,
            "railgun slug must bypass PDCs/lasers and hit a target that does not jink"
        );

        // Case B: Target executes a lateral +Z jink burn (`20 m/s^2`) during the 5 s time-of-flight
        let mut bubble_jink =
            LocalSpaceBubble::new("bubble.rg_jink", "pallas_inclined_belt", "RG JINK", 2_000);
        bubble_jink.insert_ship(shooter);
        bubble_jink.insert_ship(target);
        bubble_jink.set_party_relation(SpacePartyId(1), SpacePartyId(2), SpaceRelation::Hostile);

        let fired_jink = bubble_jink.fire_railgun("shooter", "target");
        check_eq!(fired_jink.len(), 1);

        // Immediately command a lateral jink burn (`[0, 0, 20] m/s^2`) on the target:
        // Over 5 s, z displacement = 0.5 * 20 * 25 = 250 m > 60 m hit_radius_m!
        check!(
            bubble_jink
                .command_burn("target", [0, 0, 20], false)
                .is_ok()
        );

        let report_jink = bubble_jink.step_wall_ms(6_000);
        let dodged = report_jink
            .events
            .iter()
            .any(|e| matches!(e, SpaceCombatEvent::RailgunDodgedByJink { .. }));
        check!(
            dodged,
            "lateral jinking burn during time-of-flight must dodge the railgun slug"
        );
        Ok(())
    }

    #[test]
    fn attitude_roll_rotates_armor_facets_and_reserve_guard_protects_return_dv() -> TestResult {
        let mut bubble = LocalSpaceBubble::new("bubble.facets", "ceres_halo", "FACET TEST", 3_000);
        let attacker = SpaceShipState::new_corvette(
            "attacker",
            "ATTACKER",
            SpacePartyId(1),
            [0, 40_000, 0],
            [0, 0, 0],
        );
        let mut dray =
            SpaceShipState::new_dray("dray", "DRAY HAULER", SpacePartyId(2), [0, 0, 0], [0, 0, 0]);
        // Facing +X (`yaw = 0`), attacker at +Y (`[0, 40_000, 0]`) is on the Right (Starboard) facet
        dray.attitude_deg = (0, 0, 0);
        check_eq!(
            dray.select_facet_for_attack([0, 40_000, 0]),
            FacetBand::Right
        );

        // Roll hull +90 deg so the Top (Dorsal) facet rotates into the +Y attack bearing
        dray.attitude_deg = (0, 0, -90);
        check_eq!(dray.select_facet_for_attack([0, 40_000, 0]), FacetBand::Top);

        // Roll hull 180 deg so the Left (Port) facet faces the +Y attack bearing
        dray.attitude_deg = (0, 0, 180);
        check_eq!(
            dray.select_facet_for_attack([0, 40_000, 0]),
            FacetBand::Left
        );

        // Test RETURN_RESERVE guard (`D-35-6`): when delta_v_mps <= return_reserve_dv_mps,
        // unforced burn returns `Err(TankReserveWarning)`, while `force = true` succeeds.
        dray.delta_v_mps = dray.return_reserve_dv_mps;
        bubble.insert_ship(attacker);
        bubble.insert_ship(dray);

        let unforced = bubble.command_burn("dray", [10, 0, 0], false);
        check!(matches!(
            unforced,
            Err(SpaceCombatEvent::TankReserveWarning { .. })
        ));
        let forced = bubble.command_burn("dray", [10, 0, 0], true);
        check!(forced.is_ok());

        // Test linkdead ballistic safety rule (`D-35-7`):
        if let Some(ship) = bubble.ships.get_mut("dray") {
            ship.crew_linkdead = true;
        }
        let _ = bubble.step_wall_ms(500);
        let ship_after = bubble.ships.get("dray").ok_or("missing dray")?;
        check_eq!(
            ship_after.thrust_mps2,
            [0, 0, 0],
            "linkdead ship must cut main drive to zero while keeping PDCs armed"
        );
        check!(ship_after.pdc_auto);
        Ok(())
    }

    #[test]
    fn story_1_kirkwood_customs_picket_heave_to_ambush_jink_and_derelict() -> TestResult {
        // 1. Player Dray-P promotes into the Kirkwood Customs Picket with papered strategic_ore
        let mut encounter = KirkwoodPicketEncounter::promote_interception(
            50_000,
            HaulerManifest {
                strategic_ore_kg: 18_000,
                metal_ore_kg: 12_000,
                valid_registry_claim: true,
                contraband_aboard: false,
            },
        );
        check!(encounter.journey_promoted_paused);
        check!(matches!(
            encounter.phase,
            PicketPhase::HailedToHeaveTo { .. }
        ));

        // 2. Player commands a retro-burn (`[-20, 0, 0] m/s^2`) for 1.5 s to heave-to (< 10 m/s)
        check!(
            encounter
                .bubble
                .command_burn(HAULER_SHIP_ID, [-20, 0, 0], false)
                .is_ok()
        );
        let _ = encounter.step_wall_ms(1_500);
        check!(
            encounter
                .bubble
                .command_burn(HAULER_SHIP_ID, [0, 0, 0], false)
                .is_ok()
        );
        let rel_speed = encounter
            .relative_speed_to_customs_mps()
            .ok_or("missing ships")?;
        check!(
            rel_speed <= HEAVE_TO_MAX_REL_MPS,
            "hauler should be heaved-to within 10 m/s, got {rel_speed}"
        );
        check!(matches!(
            encounter.phase,
            PicketPhase::HeavedToInspecting { .. }
        ));

        // 3. Raider Kestrel lights its torch and launches a 2-torpedo salvo + 120 km railgun shot!
        let ambush_events = encounter.trigger_raider_ambush();
        check_eq!(ambush_events.len(), 3);
        check_eq!(encounter.phase, PicketPhase::RaiderAmbushActive);

        // Player executes a lateral +Z jink burn (`[0, 0, 15] m/s^2`) to dodge the railgun slug
        // while staying inside the customs corvette's 15 km automatic PDC screen!
        check!(
            encounter
                .bubble
                .command_burn(HAULER_SHIP_ID, [0, 0, 15], false)
                .is_ok()
        );

        let mut railgun_dodged = false;
        for _ in 0..30 {
            let rep = encounter.step_wall_ms(1_000);
            // Cut jink burn after 3 s so hauler stays close to the customs corvette screen
            if encounter.bubble.wall_elapsed_ms >= 4_500 {
                let _ = encounter
                    .bubble
                    .command_burn(HAULER_SHIP_ID, [0, 0, 0], false);
            }
            for ev in rep.events {
                if matches!(ev, SpaceCombatEvent::RailgunDodgedByJink { .. }) {
                    railgun_dodged = true;
                }
            }
            if encounter.bubble.torpedoes.is_empty() && encounter.bubble.railgun_slugs.is_empty() {
                break;
            }
        }

        check!(
            railgun_dodged,
            "player hauler should dodge the 120 km railgun shot by jinking"
        );
        check!(
            encounter.customs_screen_intercepts > 0,
            "customs corvette PDC screen should assist in intercepting the raider's torpedo salvo"
        );

        // 4. Customs corvette and player disable the raider's drive (`struck_through: rear -> drive`),
        //    leaving the raider as an open-epoch derelict (`D-35-12`) and resolving the picket!
        if let Some(raider) = encounter.bubble.ships.get_mut(RAIDER_SHIP_ID) {
            // Raider turns away to flee, exposing its rear drive facet to customs fire
            raider.attitude_deg = (0, 0, 0);
            raider.pos_m = [40_000, 0, 0];
            let _ = raider.apply_damage(FacetBand::Rear, 350);
        }
        let _ = encounter.step_wall_ms(1_000);

        let raider_state = encounter
            .bubble
            .ships
            .get(RAIDER_SHIP_ID)
            .ok_or("missing raider")?;
        check!(matches!(
            raider_state.status,
            HullStatus::DisabledDerelict {
                epoch_open: true,
                ..
            }
        ));
        check_eq!(
            encounter.phase,
            PicketPhase::Resolved(PicketOutcome::ClearedToProceed {
                raider_disabled_derelict: true,
                customs_screen_assisted: true,
            })
        );
        check!(!encounter.journey_promoted_paused);

        // 5. Verify `holotank_feed` and `cockpit_feed` JSON projections
        let holo = encounter.bubble.holotank_feed(HAULER_SHIP_ID);
        check_eq!(
            holo.get("title").and_then(serde_json::Value::as_str),
            Some("KIRKWOOD CUSTOMS PICKET")
        );
        let cockpit = encounter.bubble.cockpit_feed(HAULER_SHIP_ID);
        check_eq!(
            cockpit.get("callsign").and_then(serde_json::Value::as_str),
            Some("DRAY-P AURORA")
        );
        Ok(())
    }

    #[test]
    fn sf4_asteroid_mining_stow_ore_velocity_match_and_4_drone_cap() -> TestResult {
        let mut field = AsteroidClusterField::new_canonical_cluster("pallas_inclined_belt", 10_000);

        // 1. Plain Dray bulk hauler (`accepts_stow_ore = false`) is refused raw ore extraction!
        let plain_dray = SpaceShipState::new_dray(
            "plain_dray",
            "PLAIN DRAY",
            SpacePartyId(1),
            [1_000, -400, 150],
            [2_000, -1_000, 500],
        );
        field.insert_ship_with_hold(plain_dray, OreHoldState::new_plain_dray_bulk_only());
        check_eq!(
            field.extract_with_mining_laser("plain_dray", "rock.c_101", 1_000),
            Err(OreMiningError::HullLacksStowOre)
        );

        // 2. Dray-P prospector enters far from M-309 with high relative velocity (`30 m/s`)
        let dray_p = SpaceShipState::new_dray(
            "dray_p",
            "DRAY-P PROSPECTOR",
            SpacePartyId(1),
            [0, 0, 0],
            [30_000, 0, 0],
        );
        field.insert_ship_with_hold(dray_p, OreHoldState::new_dray_p());

        // Ping M-309 (free, no licence needed)
        let (spec, pools) = field.ping_rock("dray_p", "rock.m_309")?;
        check_eq!(spec, AsteroidSpectralClass::MType);
        check_eq!(pools.len(), 2);
        check_eq!(pools[0].0.as_str(), "strategic_ore");
        check_eq!(pools[1].0.as_str(), "metal_ore");

        // Attempting to mine before approaching within 2,000 m fails with `OutOfRange`
        check!(matches!(
            field.extract_with_mining_laser("dray_p", "rock.m_309", 5_000),
            Err(OreMiningError::OutOfRange { .. })
        ));

        // Move within 1,000 m of M-309 but keep 30 m/s mismatch -> fails with `VelocityMismatch`
        if let Some(ship) = field.bubble.ships.get_mut("dray_p") {
            ship.pos_m = [4_000, 3_200, 1_400];
        }
        check!(matches!(
            field.extract_with_mining_laser("dray_p", "rock.m_309", 5_000),
            Err(OreMiningError::VelocityMismatch { .. })
        ));

        // 3. Execute `+helm/match rock.m_309` (`|dv| <= 5 m/s` within `2,000 m`)
        let (dist_m, rel_mps) = field.match_velocity_with_rock("dray_p", "rock.m_309")?;
        check!(dist_m <= MAX_MINING_RANGE_M);
        check!(rel_mps <= MAX_MINING_REL_VEL_MPS);

        // 4. Launching 5 drones exceeds the 4-drone `Dray-P` bay cap (`doc 20 §17`)
        check_eq!(
            field.launch_collector_drones("dray_p", "rock.m_309", 5),
            Err(OreMiningError::DroneBayCapExceeded {
                requested: 5,
                max_drones: 4,
            })
        );

        // Launching 4 collector drones under a sealed `ExcursionId` succeeds and loads `strategic_ore`
        let (exc_id, drones) = field.launch_collector_drones("dray_p", "rock.m_309", 4)?;
        check_eq!(exc_id, ExcursionId(1));
        check_eq!(drones.len(), 4);
        let drone_rep = field.complete_drone_sortie_cycle("dray_p")?;
        check_eq!(drone_rep.good, OreGoodId::StrategicOre);
        check_eq!(drone_rep.extracted_kg, 10_000);
        // Mining M-309 in Pallas without Freeport paper flags a Pallas Syndicate claim-jump!
        check!(drone_rep.claim_jump_flagged);

        // 5. Overheating the ship mining laser on C-101 (`CType` ice/carbon) triggers outgassing kick
        let _ = field.match_velocity_with_rock("dray_p", "rock.c_101")?;
        if let Some(ship) = field.bubble.ships.get_mut("dray_p") {
            ship.heat_permille = 700;
        }
        let laser_rep = field.extract_with_mining_laser("dray_p", "rock.c_101", 4_000)?;
        check_eq!(laser_rep.good, OreGoodId::IceOre);
        check_ne!(
            laser_rep.outgassing_dv_kick_mps,
            [0, 0, 0],
            "overheated laser on C-type ice rock must trigger volatile outgassing delta-v kick"
        );
        Ok(())
    }

    #[test]
    fn sf4_freeport_registry_licence_claim_contest_and_core_commons_gate() -> TestResult {
        let mut core_field = AsteroidClusterField::new_canonical_cluster("ceres_halo", 100_000);

        let miner_a =
            SpaceShipState::new_dray("miner_a", "MINER A", SpacePartyId(1), [0, 0, 0], [0, 0, 0]);
        let miner_b =
            SpaceShipState::new_dray("miner_b", "MINER B", SpacePartyId(2), [0, 0, 0], [0, 0, 0]);
        core_field.insert_ship_with_hold(miner_a, OreHoldState::new_dray_p());
        core_field.insert_ship_with_hold(miner_b, OreHoldState::new_dray_p());

        let _ = core_field.match_velocity_with_rock("miner_a", "rock.s_204")?;
        let _ = core_field.match_velocity_with_rock("miner_b", "rock.s_204")?;

        // In `ceres_halo` (`CoreCommons`), mining without a Freeport Registry licence is refused
        check_eq!(
            core_field.extract_with_mining_laser("miner_a", "rock.s_204", 2_000),
            Err(OreMiningError::UnlicensedInCoreZone)
        );

        // `miner_b` pings the rock first at `t = 100_000` (hand-made discovery evidence)
        let _ = core_field.ping_rock("miner_b", "rock.s_204")?;
        core_field.bubble.ic_timestamp_s += 600;

        // `miner_a` buys a 2,500 cr / 168 IC day prospecting licence, pings at `t = 100_600`, and stakes
        let lic_a = core_field.issue_prospecting_licence("miner_a");
        check_eq!(lic_a.fee_paid_cr, PROSPECTING_LICENCE_FEE_CR);
        let _ = core_field.ping_rock("miner_a", "rock.s_204")?;
        core_field.stake_claim("miner_a", "rock.s_204")?;

        // `miner_b` cannot mine `miner_a`'s staked rock in `CoreCommons`
        let _ = core_field.issue_prospecting_licence("miner_b");
        check_eq!(
            core_field.extract_with_mining_laser("miner_b", "rock.s_204", 1_000),
            Err(OreMiningError::ClaimHeldByOther {
                holder_id: "miner_a".to_owned(),
            })
        );

        // Within the 48-hour contest window, `miner_b` (who has earlier hand-made ping evidence at 100_000)
        // successfully files a Freeport Registry contest!
        core_field.contest_claim("miner_b", "rock.s_204")?;
        let rock = core_field.rocks.get("rock.s_204").ok_or("missing rock")?;
        check!(matches!(
            &rock.claim,
            AsteroidClaimState::FreeportStaked {
                contested_by: Some(challenger),
                ..
            } if challenger == "miner_b"
        ));
        Ok(())
    }

    #[test]
    fn manual_6dof_docking_at_port_clamps_within_gates_and_high_speed_scrapes_front_facet()
    -> TestResult {
        // 1. Opt-in port berth approach starting misaligned (`dx = +400 mm`, `dy = -320 mm`,
        // `pitch = -2500 mdeg`, `roll = +3600 mdeg`, `dz = 2400 mm`) while paused from world tick.
        let mut session = DockingSession::start_port_approach(
            "dock.port_1",
            "dray_1",
            SpacePartyId(1),
            400,
            "ceres_port",
            "CERES-BERTH-04",
            DockingPose6Dof {
                dx_mm: 400,
                dy_mm: -320,
                dz_mm: 2_400,
                pitch_mdeg: -2_500,
                yaw_mdeg: 1_200,
                roll_mdeg: 3_600,
                vx_mmps: 0,
                vy_mmps: 0,
                vz_mmps: 0,
                vpitch_mdeg_s: 0,
                vyaw_mdeg_s: 0,
                vroll_mdeg_s: 0,
            },
        );
        check!(session.paused_from_world_tick);
        check!(!session.target.is_mandatory());
        check!(!session.pose.ready_to_clamp());

        // Trim lateral, pitch, and roll into the green gate using WASD + Arrows + Q/E
        for _ in 0..24 {
            let _ = session.step_frame_60fps(DockingControlInput {
                trans_x: -1,
                trans_y: 1,
                pitch_trim: 1,
                roll_trim: -1,
                ..DockingControlInput::default()
            });
        }
        for _ in 0..45 {
            let _ = session.step_frame_60fps(DockingControlInput::default());
        }
        check!(
            session.pose.pose_aligned(),
            "pose should now be inside ±250 mm, ±2 deg, ±3 deg gates: {:?}",
            session.pose
        );

        // Pulse `R` (`closure_pulse = +1`) to establish a ~400 mm/s closure rate (`0.40 m/s`)
        for _ in 0..20 {
            let _ = session.step_frame_60fps(DockingControlInput {
                closure_pulse: 1,
                ..DockingControlInput::default()
            });
        }
        check!(session.pose.ready_to_clamp());

        let feed = session.docking_feed();
        check_eq!(
            feed.get("status").and_then(serde_json::Value::as_str),
            Some("ALIGNED-READY")
        );

        // Coast to collar contact (`dz_mm <= 0`) at 60 fps
        for _ in 0..450 {
            let st = session.step_frame_60fps(DockingControlInput::default());
            if matches!(st, DockingSessionState::Completed(_)) {
                break;
            }
        }
        check!(!session.paused_from_world_tick);
        check!(matches!(
            session.state,
            DockingSessionState::Completed(DockingOutcome::Clamped {
                saved_delta_v_mps,
                saved_approach_ic_secs: 900,
                ..
            }) if saved_delta_v_mps > 0
        ));

        // 2. High-speed impact (`vz = 1,250 mm/s > 800 mm/s`) causes CollarScrapeBounce and damages Front facet
        let mut bubble = LocalSpaceBubble::new("bubble.dock", "ceres_halo", "DOCK TEST", 5_000);
        let dray =
            SpaceShipState::new_dray("dray_1", "DRAY", SpacePartyId(1), [0, 0, 0], [0, 0, 0]);
        let initial_front = *dray
            .sections
            .get(&ShipSectionId::Hull)
            .ok_or("missing hull")?
            .facets
            .get(&FacetBand::Front)
            .ok_or("missing front")?;
        bubble.insert_ship(dray);

        let mut fast_session = DockingSession::start_port_approach(
            "dock.port_fast",
            "dray_1",
            SpacePartyId(1),
            400,
            "ceres_port",
            "CERES-BERTH-04",
            DockingPose6Dof {
                dx_mm: 0,
                dy_mm: 0,
                dz_mm: 20,
                pitch_mdeg: 0,
                yaw_mdeg: 0,
                roll_mdeg: 0,
                vx_mmps: 0,
                vy_mmps: 0,
                vz_mmps: 1_260,
                vpitch_mdeg_s: 0,
                vyaw_mdeg_s: 0,
                vroll_mdeg_s: 0,
            },
        );
        let st = fast_session.step_frame_60fps(DockingControlInput::default());
        check!(matches!(
            st,
            DockingSessionState::Completed(DockingOutcome::CollarScrapeBounce {
                collar_damaged: true,
                ..
            })
        ));
        let _ = fast_session.apply_outcome_to_bubble(&mut bubble);
        let after_front = *bubble
            .ships
            .get("dray_1")
            .ok_or("missing ship")?
            .sections
            .get(&ShipSectionId::Hull)
            .ok_or("missing hull")?
            .facets
            .get(&FacetBand::Front)
            .ok_or("missing front")?;
        check!(
            after_front < initial_front,
            "collar scrape should ablate Front facet armor ({after_front} < {initial_front})"
        );
        Ok(())
    }

    #[test]
    fn mandatory_derelict_docking_enforces_range_clamps_tumbling_hull_and_attaches_2x_salvage_tow()
    -> TestResult {
        let mut bubble = LocalSpaceBubble::new(
            "bubble.salvage",
            "pallas_inclined_belt",
            "PALLAS SALVAGE",
            80_000,
        );
        let hauler = SpaceShipState::new_dray(
            "salvager",
            "SALVAGER",
            SpacePartyId(1),
            [0, 0, 0],
            [0, 0, 0],
        );
        let mut derelict = SpaceShipState::new_corvette(
            "raider_wreck",
            "KESTREL WRECK",
            SpacePartyId(2),
            [1_200, 0, 0],
            [0, 0, 0],
        );
        derelict.status = HullStatus::DisabledDerelict {
            epoch_open: true,
            life_support_remaining_s: DERELICT_WINDOW_S,
            salvage_contest_window_s: DERELICT_WINDOW_S,
        };
        bubble.insert_ship(hauler);
        bubble.insert_ship(derelict);

        // At 1,200 m (> 500 m limit), initiating derelict docking is refused
        let too_far = DockingSession::start_derelict_docking(
            &bubble,
            "dock.derelict_far",
            "salvager",
            "raider_wreck",
            400,
            600,
            DockingPose6Dof {
                dx_mm: 0,
                dy_mm: 0,
                dz_mm: 1_000,
                pitch_mdeg: 0,
                yaw_mdeg: 0,
                roll_mdeg: 0,
                vx_mmps: 0,
                vy_mmps: 0,
                vz_mmps: 420,
                vpitch_mdeg_s: 0,
                vyaw_mdeg_s: 0,
                vroll_mdeg_s: 0,
            },
        );
        check!(matches!(too_far, Err(DockingError::OutOfRange { .. })));

        // Move salvager within 250 m (`<= 500 m`) and start mandatory 6-DOF derelict docking
        if let Some(ship) = bubble.ships.get_mut("salvager") {
            ship.pos_m = [950, 0, 0];
        }
        let mut session = DockingSession::start_derelict_docking(
            &bubble,
            "dock.derelict_1",
            "salvager",
            "raider_wreck",
            400,
            600,
            DockingPose6Dof {
                dx_mm: 40,
                dy_mm: -30,
                dz_mm: 900,
                pitch_mdeg: 0,
                yaw_mdeg: 0,
                roll_mdeg: 0,
                vx_mmps: 0,
                vy_mmps: 0,
                vz_mmps: 450,
                vpitch_mdeg_s: 0,
                vyaw_mdeg_s: 0,
                vroll_mdeg_s: 0,
            },
        )?;
        check!(session.target.is_mandatory());

        for _ in 0..200 {
            let st = session.step_frame_60fps(DockingControlInput::default());
            if matches!(st, DockingSessionState::Completed(_)) {
                break;
            }
        }
        check!(matches!(
            session.state,
            DockingSessionState::Completed(DockingOutcome::Clamped { .. })
        ));

        // Attach salvage tow: 600 t derelict <= 2 * 400 t (800 t cap) succeeds and opens 48 h contest!
        let tow = session.attach_salvage_tow(80_100)?;
        check_eq!(tow.prior_owner_party, Some(SpacePartyId(2)));
        check!(tow.contest_open);
        check_eq!(
            tow.contest_expires_ic_secs,
            80_100 + SALVAGE_CONTEST_WINDOW_IC_SECS
        );

        // Verify `doc 27a` 2x tonnage tow cap: a 900 t hull exceeds 2 * 400 t = 800 t
        session.target = DockingTargetKind::DisabledDerelict {
            derelict_ship_id: "heavy_wreck".to_owned(),
            derelict_tonnage_t: 900,
            prior_owner_party: Some(SpacePartyId(2)),
        };
        session.state = DockingSessionState::Completed(DockingOutcome::Clamped {
            target: session.target.clone(),
            latch_closure_mmps: 450,
            saved_delta_v_mps: 0,
            saved_approach_ic_secs: 0,
        });
        check!(matches!(
            session.attach_salvage_tow(80_100),
            Err(DockingError::TowTonnageExceeded {
                towing_ship_tonnage_t: 400,
                derelict_tonnage_t: 900,
                max_towable_tonnage_t: 800,
            })
        ));
        Ok(())
    }
}

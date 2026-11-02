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

pub mod autopilot_bot;
pub mod bubble;
pub mod commands;
pub mod custody;
pub mod docking;
pub mod flight;
pub mod mining;
pub mod picket;
pub mod stations;
pub mod weapons;

pub use autopilot_bot::{
    CockpitSnapshot, DockingSnapshot, HolotankContact, HolotankSnapshot, HolotankTorpedo,
    SpaceMissionGoal, SpacePilotDoctrine, compute_6dof_rcs_control,
};
pub use bubble::{
    BubbleId, BubbleStepReport, DERELICT_WINDOW_S, EmconPosture, FacetBand, HullStatus,
    LocalSpaceBubble, MAX_SUBSTEP_MS, ShipSectionId, ShipSectionState, SpaceCombatEvent,
    SpacePartyId, SpaceRelation, SpaceShipState, SubsystemCrit,
};
pub use commands::{
    ActiveSpaceContext, HELM_HELP, HELM_SWITCHES, HELM_VERB, MINE_HELP, MINE_SWITCHES, MINE_VERB,
    SPACE_COCKPIT_FEED, SPACE_DOCKING_FEED, SPACE_HOLOTANK_FEED, SpaceBook, TAC_HELP, TAC_SWITCHES,
    TAC_VERB, register,
};
pub use custody::{
    ALL_FACET_BANDS, ALL_SHIP_SECTIONS, build_space_damage_mutation_plan,
    format_tactical_unit_damage_line, instantiate_stock_space_unit,
    record_space_combat_to_black_box, register_derelict_salvage_in_book,
    space_ship_from_tactical_unit,
};
pub use docking::{
    DOCK_GATE_ATTITUDE_MDEG, DOCK_GATE_CLOSURE_MAX_MMPS, DOCK_GATE_CLOSURE_MIN_MMPS,
    DOCK_GATE_LATERAL_MM, DOCK_GATE_ROLL_MDEG, DockingControlInput, DockingError, DockingOutcome,
    DockingPose6Dof, DockingSession, DockingSessionState, DockingTargetKind,
    MAX_DOCKING_INIT_RANGE_M, MAX_DOCKING_INIT_REL_SPEED_MPS, SALVAGE_CONTEST_WINDOW_IC_SECS,
    SalvageTowRecord,
};
pub use flight::{
    DecoupledLegTicket, FlightError, FlightGuidanceMode, FromPointPassagePlan,
    RecoupledJourneyTicket, SpaceFlightRecord,
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
pub use stations::{
    BridgeSeat, LightLagContact, SPEED_OF_LIGHT_MPS, ShipCrewRoster, compute_ir_signature_kw,
    project_light_lag_contact,
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

    fn test_kepler_sky() -> Result<crate::sky::Sky, Box<dyn Error>> {
        use std::collections::BTreeMap;

        use mush_core::CanonicalId;

        use crate::config::EphemerisConfig;
        use crate::ephemeris::{Body, Ephemeris};
        use crate::kepler::ElementRow;
        use crate::sky::{KeplerSky, Sky};

        let epoch = 74_152_540_800_i64;
        let ceres = CanonicalId::new("ceres")?;
        let mars = CanonicalId::new("mars")?;
        let jupiter = CanonicalId::new("jupiter")?;
        let saturn = CanonicalId::new("saturn")?;

        let circular = Ephemeris::new(&EphemerisConfig {
            epoch: 0,
            rate: 1_000,
        })
        .with_body(
            ceres.clone(),
            Body {
                orbit_uau: 2_866_568,
                period_mdays: 1_681_861,
                phase_mdeg: 281_654,
                epoch_ic: Some(epoch),
                parent: None,
            },
        )
        .with_body(
            mars,
            Body {
                orbit_uau: 1_445_443,
                period_mdays: 687_034,
                phase_mdeg: 276_240,
                epoch_ic: Some(epoch),
                parent: None,
            },
        )
        .with_body(
            jupiter,
            Body {
                orbit_uau: 5_204_400,
                period_mdays: 4_332_589,
                phase_mdeg: 112_000,
                epoch_ic: Some(epoch),
                parent: None,
            },
        )
        .with_body(
            saturn,
            Body {
                orbit_uau: 9_582_600,
                period_mdays: 10_759_220,
                phase_mdeg: 45_000,
                epoch_ic: Some(epoch),
                parent: None,
            },
        );

        let ceres_row = ElementRow::from_authored(&[
            74_152_540_800.0,
            2.767_755_414,
            0.075_862_114,
            10.550_296,
            75.280_015,
            84.918_514,
            114.238_750,
            0.214_048_659,
        ])?;
        let mut tables = BTreeMap::new();
        tables.insert(ceres, vec![ceres_row]);
        Ok(Sky::Kepler(KeplerSky::new(circular, tables)))
    }

    #[test]
    fn sf6_mid_leg_decouple_live_propellant_debit_reserve_guard_and_autopilot_recouple()
    -> TestResult {
        use mush_core::CanonicalId;

        let sky = test_kepler_sky()?;
        let epoch = 74_152_540_800_i64;
        let ceres = CanonicalId::new("ceres")?;
        let mars = CanonicalId::new("mars")?;

        // 1. Ship decouples halfway through a 40,000 s Ceres -> Mars passage (`+helm/decouple`)
        let mut flight = SpaceFlightRecord::decouple_from_leg(
            &sky,
            "ship.dray_7",
            "UNS-WANDERER",
            "dray",
            SpacePartyId(1),
            &ceres,
            &mars,
            epoch,
            epoch + 40_000,
            epoch + 20_000,
            32_000,
            20_000,
        )?;
        let ticket = flight
            .decoupled_ticket
            .as_ref()
            .ok_or("missing decoupled ticket")?;
        check!(ticket.ticket_voided);
        check_eq!(ticket.refunded_propellant_kg, 16_000);
        check_eq!(flight.propellant_kg, 36_000);
        check_eq!(flight.delta_v_mps, 18_000);

        // Interpolated position at midpoint is strictly between Ceres and Mars
        let p_ceres = sky.position_km(&ceres, epoch).ok_or("missing ceres")?;
        let p_mars = sky
            .position_km(&mars, epoch + 40_000)
            .ok_or("missing mars")?;
        let ship_km = flight.pos_km();
        check_ne!(ship_km, p_ceres);
        check_ne!(ship_km, p_mars);

        // 2. Commanding a burn that dips below `return_reserve_dv_mps` (6,000 m/s) raises
        // `FlightError::TankReserveWarning` unless `force = true` (`D-35-6`).
        let unforced = flight.command_burn_dv([1_000, 0, 0], 20, 700, false);
        check!(matches!(
            unforced,
            Err(FlightError::TankReserveWarning { .. })
        ));

        // A smaller 2,000 m/s burn (`20 m/s^2 * 100 s`) leaves 16,000 m/s >= 6,000 m/s reserve
        flight.command_burn_dv([1_000, 0, 0], 20, 100, false)?;
        let prop_before = flight.propellant_kg;
        flight.step_ic_seconds(&sky, 100);
        check_eq!(flight.delta_v_mps, 16_000);
        check_eq!(flight.propellant_kg, prop_before - 4_000);

        // 3. Re-coupling to autopilot (`+helm/autopilot ceres`) is refused if engaged in combat,
        // and succeeds once unengaged (`D-35-5`).
        check_eq!(
            flight.recouple_autopilot(&sky, &ceres, true),
            Err(FlightError::EngagedInCombat)
        );
        let recoupled = flight.recouple_autopilot(&sky, &ceres, false)?;
        check_eq!(recoupled.dest_body, ceres);
        check!(recoupled.distance_km > 0);
        check!(recoupled.arrive_ic > recoupled.depart_ic);
        Ok(())
    }

    #[test]
    fn sf6_outer_system_neptune_triton_expedition_light_lag_and_canopy_projection() -> TestResult {
        use mush_core::CanonicalId;

        let sky = test_kepler_sky()?;
        let epoch = 74_152_540_800_i64;
        let ceres = CanonicalId::new("ceres")?;
        let neptune = CanonicalId::new("neptune")?;
        let triton = CanonicalId::new("triton")?;

        let mut expedition = SpaceFlightRecord::undock_from_body(
            &sky,
            "ship.halcyon",
            "UNS-HALCYON",
            "halcyon",
            SpacePartyId(1),
            &ceres,
            epoch,
        )?;

        // Without long-range drop tanks, an outer-system expedition to Neptune (30 AU)
        // triggers the `TankReserveWarning` / `InsufficientDeltaV` guard!
        check!(
            expedition
                .fly_expedition_to_body(&sky, &neptune, 85_000, false)
                .is_err()
        );

        // Fit long-range expedition drop tanks (Story 2 step 4) and burn out to Neptune & Triton!
        expedition.fit_long_range_drop_tanks();
        check!(expedition.drop_tanks_fitted);
        let plan = expedition.fly_expedition_to_body(&sky, &neptune, 85_000, false)?;
        check!(plan.distance_km > 3_500_000_000);

        // At 85,000 km standoff from Neptune, distance to Neptune is 85,000 km,
        // while light lag back to inner-system Ceres is > 10,000 seconds (> 2.7 hours)!
        let dist_neptune = expedition
            .distance_to_body_km(&sky, &neptune)
            .ok_or("missing neptune")?;
        check_eq!(dist_neptune, 85_000);
        let lag_to_ceres_s = expedition
            .light_lag_to_body_s(&sky, &ceres)
            .ok_or("missing ceres lag")?;
        check!(
            lag_to_ceres_s > 10_000,
            "expected multi-hour light lag from Neptune to Ceres, got {lag_to_ceres_s} s"
        );

        // Project `space.cockpit` feed at Neptune & Triton and verify both outer bodies appear
        let canopy = expedition.canopy_feed(&sky, &[neptune, triton]);
        let bodies = canopy
            .get("bodies")
            .and_then(serde_json::Value::as_array)
            .ok_or("missing bodies array")?;
        check_eq!(bodies.len(), 2);
        Ok(())
    }

    #[test]
    fn story_2_pallas_claim_jump_derelict_salvage_and_neptune_expedition_end_to_end() -> TestResult
    {
        use mush_core::CanonicalId;

        // Step 1: Crew flies a Dray-P into `pallas_inclined_belt`, pings M-type rock `rock.m_301`,
        // matches 3D velocity (`|dv| <= 5 m/s` within 2 km), deploys 4 collector drones (`ExcursionId`),
        // and extracts `strategic_ore` + `metal_ore` with the ship mining laser into `stow.ore`.
        let mut field =
            AsteroidClusterField::new_canonical_cluster("pallas_inclined_belt", 200_000);
        let dray_p = SpaceShipState::new_dray(
            "player_dray_p",
            "UNS-PROSPECTOR",
            SpacePartyId(1),
            [0, 0, 0],
            [0, 0, 0],
        );
        field.insert_ship_with_hold(dray_p, OreHoldState::new_dray_p());

        let (spec, pools) = field.ping_rock("player_dray_p", "rock.m_309")?;
        check_eq!(spec, AsteroidSpectralClass::MType);
        check_eq!(pools.len(), 2);
        let (dist_m, rel_mps) = field.match_velocity_with_rock("player_dray_p", "rock.m_309")?;
        check!(dist_m <= MAX_MINING_RANGE_M);
        check!(rel_mps <= MAX_MINING_REL_VEL_MPS);

        let (excursion, drones) =
            field.launch_collector_drones("player_dray_p", "rock.m_309", 4)?;
        check_eq!(excursion, ExcursionId(1));
        check_eq!(drones.len(), 4);
        let drone_rep = field.complete_drone_sortie_cycle("player_dray_p")?;
        check_eq!(drone_rep.good, OreGoodId::StrategicOre);
        check_eq!(drone_rep.extracted_kg, 10_000);

        let laser_rep = field.extract_with_mining_laser("player_dray_p", "rock.m_309", 4_000)?;
        check_eq!(laser_rep.good, OreGoodId::StrategicOre);
        check_eq!(laser_rep.extracted_kg, 4_000);
        check!(
            laser_rep.ir_bloom_permille >= 600,
            "mining laser should spike thermal/IR bloom across the cluster"
        );
        check!(
            laser_rep.claim_jump_flagged,
            "mining Syndicate-papered rock without Registry stake logs claim-jump event"
        );

        // Step 2: Thermal bloom attracts a rival claim-jumper; after a laser/railgun duel on 1:1
        // wall time, the attacker's drive is disabled and its hull remains as an open-epoch derelict.
        let mut jumper = SpaceShipState::new_corvette(
            "claim_jumper",
            "SYNDICATE-CUTTER",
            SpacePartyId(2),
            [35_000, 0, 0],
            [0, 0, 0],
        );
        // Set jumper drive internals low so a laser volley disables its rear drive into a derelict
        jumper.attitude_deg = (0, 0, 0);
        if let Some(drive) = jumper.sections.get_mut(&ShipSectionId::Drive) {
            drive.facets.insert(FacetBand::Rear, 0);
            drive.internals = 10;
        }
        field.bubble.insert_ship(jumper);
        field
            .bubble
            .set_party_relation(SpacePartyId(1), SpacePartyId(2), SpaceRelation::Hostile);

        // Player refits/arms a defensive laser on the Dray-P (or escort) to disable the claim-jumper's drive
        if let Some(ship) = field.bubble.ships.get_mut("player_dray_p") {
            ship.pos_m = [34_750, 0, 0];
            ship.vel_mmps = [0, 0, 0];
            ship.lasers
                .push(LaserMount::standard_turret("mining_combat_laser"));
        }
        let laser_events = field
            .bubble
            .fire_offensive_lasers("player_dray_p", "claim_jumper");
        check!(!laser_events.is_empty());
        let jumper_after = field
            .bubble
            .ships
            .get("claim_jumper")
            .ok_or("missing claim_jumper")?;
        check!(matches!(
            jumper_after.status,
            HullStatus::DisabledDerelict {
                epoch_open: true,
                ..
            }
        ));

        // Step 3: Within 500 m (250 m standoff), pilot triggers mandatory out-of-time-loop 6-DOF
        // hard-docking (`docking.rs`), aligns within gates at < 0.80 m/s, clamps, and attaches
        // a salvage tow line under `doc 27a` (2x tonnage limit, 48 h contest window).
        let mut dock_session = DockingSession::start_derelict_docking(
            &field.bubble,
            "dock.story2",
            "player_dray_p",
            "claim_jumper",
            400,
            550,
            DockingPose6Dof {
                dx_mm: 90,
                dy_mm: -70,
                dz_mm: 800,
                pitch_mdeg: 300,
                yaw_mdeg: -250,
                roll_mdeg: 400,
                vx_mmps: 0,
                vy_mmps: 0,
                vz_mmps: 450,
                vpitch_mdeg_s: 0,
                vyaw_mdeg_s: 0,
                vroll_mdeg_s: 0,
            },
        )?;
        check!(dock_session.paused_from_world_tick);
        check!(dock_session.target.is_mandatory());
        for _ in 0..160 {
            let st = dock_session.step_frame_60fps(DockingControlInput::default());
            if matches!(st, DockingSessionState::Completed(_)) {
                break;
            }
        }
        check!(matches!(
            dock_session.state,
            DockingSessionState::Completed(DockingOutcome::Clamped { .. })
        ));
        let now_ic_u64 = field.bubble.ic_timestamp_s.max(0) as u64;
        let tow = dock_session.attach_salvage_tow(now_ic_u64)?;
        check!(tow.contest_open);
        check_eq!(
            tow.contest_expires_ic_secs,
            now_ic_u64 + SALVAGE_CONTEST_WINDOW_IC_SECS
        );

        // Step 4: With the payout, captain fits long-range drop tanks, decouples into Tier-1
        // System Free Flight (`+helm/decouple`), burns out past Saturn and Uranus to Neptune & Triton,
        // and projects the First-Person Space Bridge Canopy (`space.cockpit`).
        let sky = test_kepler_sky()?;
        let epoch = 74_152_540_800_i64;
        let ceres = CanonicalId::new("ceres")?;
        let mars = CanonicalId::new("mars")?;
        let saturn = CanonicalId::new("saturn")?;
        let uranus = CanonicalId::new("uranus")?;
        let neptune = CanonicalId::new("neptune")?;
        let triton = CanonicalId::new("triton")?;

        let mut free_flight = SpaceFlightRecord::decouple_from_leg(
            &sky,
            "player_dray_p",
            "UNS-PROSPECTOR",
            "dray_p",
            SpacePartyId(1),
            &ceres,
            &mars,
            epoch,
            epoch + 30_000,
            epoch + 10_000,
            24_000,
            24_000,
        )?;
        free_flight.fit_long_range_drop_tanks();
        let _saturn_flyby = free_flight.fly_expedition_to_body(&sky, &saturn, 120_000, true)?;
        let _uranus_flyby = free_flight.fly_expedition_to_body(&sky, &uranus, 110_000, true)?;
        let neptune_flyby = free_flight.fly_expedition_to_body(&sky, &neptune, 90_000, true)?;
        check!(neptune_flyby.distance_km > 1_000_000_000);

        let canopy_feed = free_flight.canopy_feed(&sky, &[neptune, triton]);
        let bodies = canopy_feed
            .get("bodies")
            .and_then(serde_json::Value::as_array)
            .ok_or("missing bodies")?;
        check_eq!(bodies.len(), 2);
        Ok(())
    }

    #[test]
    fn sf9_live_mush_verbs_helm_tac_and_mine_drive_stories_1_and_2_and_emit_space_feeds()
    -> TestResult {
        use crate::composition as comp;
        let cfg = crate::TacsimConfig {
            enabled: true,
            ..crate::TacsimConfig::default()
        };
        let onboarding = mush_core::OnboardingConfig::default();
        let game = crate::GameEconomyConfig::default();
        let (mut mods, inventory) = comp::domain_modules_with_economy(
            mush_economy::EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &game,
        );
        mods.push(comp::npc_module(comp::npc_agent_adapter()));
        mods.extend(comp::sheet_modules(&onboarding, &game));
        mods.push(comp::tacsim_module_wired(cfg, true, inventory));
        let mut tw = mush_testkit::TestWorld::with_modules(mods);
        let pilot = mush_core::GOD;

        // 0. Story 1 via live MUSH verbs: promote into `kirkwood_picket`, brake (`+helm/burn`),
        //    step the 1:1 picket bubble (`+tac/step`), roll hull attitude (`+helm/roll`), and
        //    fire defensive ordnance (`+tac/laser`, `+tac/torp`, `+tac/rail`).
        let picket_events = tw.run(pilot, "+helm/enter kirkwood_picket");
        check!(
            picket_events.iter().any(|(_, ev)| {
                matches!(ev, mush_proto::OutputEvent::Panel { feed, .. } if feed == SPACE_HOLOTANK_FEED)
            }),
            "expected space.holotank keyframe on +helm/enter kirkwood_picket"
        );
        let _ = tw.run(pilot, "+helm/burn -15 0 0");
        let _ = tw.run(pilot, "+tac/step 1000");
        let _ = tw.run(pilot, "+helm/roll 0 0 90");
        let _ = tw.run(pilot, "+tac/laser defense");

        // 1. Story 2 via live MUSH verbs: enter Pallas asteroid cluster via `+helm/enter pallas_inclined_belt`
        let enter_events = tw.run(pilot, "+helm/enter pallas_inclined_belt");
        check!(
            enter_events.iter().any(|(_, ev)| {
                matches!(ev, mush_proto::OutputEvent::Panel { feed, .. } if feed == SPACE_HOLOTANK_FEED)
            }),
            "expected space.holotank keyframe on +helm/enter"
        );

        // 2. Ping M-type rock `rock.m_309`, match 3D velocity (`+helm/match`), launch 4 drones (`+mine/drones`),
        //    and fire mining laser (`+mine/laser`), triggering frontier claim-jump rival spawn!
        let _ = tw.run(pilot, "+mine/ping rock.m_309");
        let _ = tw.run(pilot, "+helm/match rock.m_309");
        let _ = tw.run(pilot, "+mine/drones rock.m_309 4");
        let _ = tw.run(pilot, "+mine/laser rock.m_309 4000");

        // 3. Fire offensive laser at the spawned `claim_jumper` via `+tac/beam claim_jumper` to disable its drive
        let _ = tw.run(pilot, "+tac/pdc auto");
        let _ = tw.run(pilot, "+tac/beam claim_jumper");

        // 4. Hard-dock with the disabled derelict via `+helm/dock derelict claim_jumper`, clamp, and attach tow (`+helm/tow`)
        let dock_events = tw.run(pilot, "+helm/dock derelict claim_jumper");
        check!(
            dock_events.iter().any(|(_, ev)| {
                matches!(ev, mush_proto::OutputEvent::Panel { feed, .. } if feed == SPACE_DOCKING_FEED)
            }),
            "expected space.docking keyframe on +helm/dock derelict"
        );
        let _ = tw.run(pilot, "+helm/dock clamp");
        let _ = tw.run(pilot, "+helm/tow");

        let ship_id = format!("ship.{}", pilot.0);
        {
            let book = tw
                .world()
                .module_state::<SpaceBook>()
                .ok_or("missing SpaceBook")?;
            let tow = book.tows.get(&ship_id).ok_or("missing SalvageTowRecord")?;
            check!(tow.contest_open);
            check_eq!(tow.derelict_ship_id, "claim_jumper");
            let jumper_assess = book
                .unit_assessments
                .get("claim_jumper")
                .ok_or("missing claim_jumper FieldUnitAssessment")?;
            check!(jumper_assess.is_towable);
            let claim_id = crate::field_ops::SalvageClaimId::new(1)?;
            check!(
                book.salvage_book.claim(claim_id).is_some(),
                "expected 48h SalvageClaim registered in SpaceBook.salvage_book by +helm/tow"
            );
        }

        // 5. Decouple into Tier-1 System Free Flight, fit drop tanks, fly expedition to Neptune, and recouple autopilot!
        let _ = tw.run(pilot, "+helm/decouple ceres mars");
        let _ = tw.run(pilot, "+helm/droptanks");
        let nep_events = tw.run(pilot, "+helm/expedition neptune");
        check!(
            nep_events.iter().any(|(_, ev)| {
                matches!(ev, mush_proto::OutputEvent::Panel { feed, .. } if feed == SPACE_COCKPIT_FEED)
            }),
            "expected space.cockpit canopy keyframe on +helm/expedition neptune"
        );
        let _ = tw.run(pilot, "+helm/autopilot ceres");

        Ok(())
    }

    #[test]
    fn sf10_tactical_unit_custody_roundtrip_black_box_forensics_and_salvage_claim_book()
    -> TestResult {
        use crate::field_ops::{
            CONTEST_WINDOW_AVAILABLE_REAL_SECS, ContestGround, CustodyTitleStatus,
            OperationalState, RecoveryOpId, SalvageClaimBook,
        };
        use mush_core::ObjId;

        // 1. Instantiate a stock `tacsim.unit.dray` TacticalUnit (`ObjId(801)`) and convert it
        //    into a `SpaceShipState` via `space_ship_from_tactical_unit` (`D-35-1`).
        let (dray_before, dray_spec) =
            instantiate_stock_space_unit(ObjId(801), "tacsim.unit.dray")?;
        let mut ship = space_ship_from_tactical_unit(
            &dray_before,
            "derelict_dray",
            "UNS-DRAY-801",
            SpacePartyId(2),
            [0, 0, 0],
            [0, 0, 0],
        )?;
        check_eq!(ship.hull_integrity_pct(), 100);
        check_eq!(format_tactical_unit_damage_line(&dray_before), None);

        // 2. Apply space combat damage: scrape `Front` armor (50 dmg) and blow out `Rear` armor
        //    + `Drive` internals (350 dmg -> 60 absorbed by `hull.rear`, 250 dealt to `drive`
        //    internals -> `drive.internals == 0`, transitioning hull to `DisabledDerelict`).
        let _ = ship.apply_damage(FacetBand::Front, 50);
        let (struck_sec, absorbed, dealt) = ship.apply_damage(FacetBand::Rear, 350);
        check_eq!(struck_sec, ShipSectionId::Drive);
        check_eq!(absorbed, 60);
        check_eq!(dealt, 250);
        check!(matches!(
            ship.status,
            HullStatus::DisabledDerelict {
                epoch_open: true,
                ..
            }
        ));

        // 3. Build and apply the IC-custody-legal `UnitMutationPlan` onto `dray_before`
        //    via `build_space_damage_mutation_plan`.
        let (dray_after, assessment, plan_opt) =
            build_space_damage_mutation_plan(&dray_before, &ship, Some(&dray_spec), 2_000)?;
        check!(plan_opt.is_some());
        check_eq!(assessment.state, OperationalState::Immobile);
        check!(!assessment.can_move);
        check!(assessment.is_towable);
        check_eq!(
            format_tactical_unit_damage_line(&dray_after).as_deref(),
            Some("Damage: drive destroyed.")
        );

        // Round-trip `dray_after` back into `SpaceShipState` and verify `DisabledDerelict`
        // and damaged `Front` facet (`120 - 50 = 70`) persist identically.
        let roundtripped = space_ship_from_tactical_unit(
            &dray_after,
            "derelict_dray",
            "UNS-DRAY-801",
            SpacePartyId(2),
            [0, 0, 0],
            [0, 0, 0],
        )?;
        check!(matches!(
            roundtripped.status,
            HullStatus::DisabledDerelict {
                epoch_open: true,
                ..
            }
        ));
        check_eq!(
            roundtripped
                .sections
                .get(&ShipSectionId::Hull)
                .and_then(|s| s.facets.get(&FacetBand::Front))
                .copied(),
            Some(70)
        );

        // 4. Record space combat events into `SalvageClaimBook`'s tamper-evident black box
        //    and register the 48-hour salvage tow claim (`doc 27a`).
        let mut salvage_book = SalvageClaimBook::new();
        let events = vec![
            SpaceCombatEvent::LaserHit {
                shooter_id: "tug_ship".to_owned(),
                target_id: "derelict_dray".to_owned(),
                distance_m: 350,
                facet: FacetBand::Rear,
                section: ShipSectionId::Drive,
                armor_damage: 60,
                internal_damage: 250,
                shooter_heat_permille: 240,
            },
            SpaceCombatEvent::ShipDisabledDerelict {
                ship_id: "derelict_dray".to_owned(),
                epoch_open: true,
                life_support_remaining_s: DERELICT_WINDOW_S,
            },
        ];
        let recorded = record_space_combat_to_black_box(
            &mut salvage_book,
            dray_after.object(),
            "derelict_dray",
            Some(ObjId(802)),
            &events,
            assessment.state,
            2_000,
        )?;
        check_eq!(recorded, 2);

        let tow = SalvageTowRecord {
            towing_ship_id: "tug_ship".to_owned(),
            derelict_ship_id: "derelict_dray".to_owned(),
            derelict_tonnage_t: 400,
            prior_owner_party: Some(SpacePartyId(2)),
            contest_open: true,
            contest_expires_ic_secs: 2_000 + SALVAGE_CONTEST_WINDOW_IC_SECS,
        };
        let op_id = RecoveryOpId::new(1)?;
        let salvager = ObjId(10);
        let prior_owner = ObjId(20);
        let port_base = ObjId(900);
        let custody_status = register_derelict_salvage_in_book(
            &mut salvage_book,
            op_id,
            &tow,
            dray_after.object(),
            ObjId(802),
            salvager,
            prior_owner,
            port_base,
            2_010,
            50_000,
        )?;

        let claim_id = match custody_status {
            CustodyTitleStatus::PendingSalvageClaim { claim_id, .. } => claim_id,
            other => return Err(format!("expected PendingSalvageClaim, got {other:?}").into()),
        };

        // Extract black-box forensics in port custody and verify the recorded combat hits + tow
        let forensics = salvage_book.extract_black_box_forensics(op_id, salvager, true, 2_020)?;
        check_eq!(forensics.hull, dray_after.object());
        check_eq!(forensics.closed_in_base, port_base);
        check!(!forensics.reveals_unlawful_recovery);
        check_eq!(forensics.entries.len(), 4);

        // Bare prior title alone does not vacate a lawful open-space salvage recovery (`doc 27a` `Q-R1`),
        // and after 48 available real hours (`CONTEST_WINDOW_AVAILABLE_REAL_SECS`), title settles to `salvager`!
        let bare = salvage_book.submit_contest(
            claim_id,
            prior_owner,
            ContestGround::BarePriorTitle,
            55_000,
        )?;
        check_eq!(bare.valid_to_vacate, None);
        let (settled, _) =
            salvage_book.adjudicate_claim(claim_id, 50_000 + CONTEST_WINDOW_AVAILABLE_REAL_SECS)?;
        check_eq!(
            settled,
            CustodyTitleStatus::ResolvedTitle { owner: salvager }
        );

        Ok(())
    }

    #[test]
    fn sf11_autonomous_space_pilot_doctrine_completes_stories_1_and_2_from_panel_feeds()
    -> TestResult {
        use crate::composition as comp;
        let cfg = crate::TacsimConfig {
            enabled: true,
            ..crate::TacsimConfig::default()
        };
        let onboarding = mush_core::OnboardingConfig::default();
        let game = crate::GameEconomyConfig::default();
        let (mut mods, inventory) = comp::domain_modules_with_economy(
            mush_economy::EconomyConfig::default(),
            mush_atlas::AtlasConfig::default(),
            &game,
        );
        mods.push(comp::npc_module(comp::npc_agent_adapter()));
        mods.extend(comp::sheet_modules(&onboarding, &game));
        mods.push(comp::tacsim_module_wired(cfg, true, inventory));
        let mut tw = mush_testkit::TestWorld::with_modules(mods);
        let pilot = mush_core::GOD;

        // 1. Story 1: Autonomous `SpacePilotDoctrine` executes Kirkwood 3:1 Customs Picket
        //    and Belt Raider Ambush purely from `space.holotank` and `space.cockpit` keyframes.
        let mut bot = SpacePilotDoctrine::new(SpaceMissionGoal::KirkwoodPicketTransit);
        while !bot.is_goal_complete() {
            let cmds = bot.decide_next_commands();
            check!(!cmds.is_empty(), "expected non-empty bot command batch");
            for cmd in cmds {
                let events = tw.run(pilot, &cmd);
                bot.observe_events(&events);
            }
        }
        check!(bot.keyframes_ingested >= 6);
        let holo1 = bot
            .latest_holotank
            .as_ref()
            .ok_or("missing holotank snapshot after Story 1")?;
        check!(holo1.has_inspecting_picket());
        let cockpit1 = bot
            .latest_cockpit
            .as_ref()
            .ok_or("missing cockpit snapshot after Story 1")?;
        check_eq!(cockpit1.pdc_mode, "AUTO");
        check_eq!(cockpit1.attitude_deg.2, 90);

        // 2. Story 2 (Part A): Switch autonomous pilot to `ClusterProspectAndSalvage`
        //    in `pallas_inclined_belt` targeting M-type rock `rock.m_309`.
        bot.set_goal(SpaceMissionGoal::ClusterProspectAndSalvage {
            cluster_id: "pallas_inclined_belt".to_owned(),
            target_rock_id: "rock.m_309".to_owned(),
        });
        while !bot.is_goal_complete() {
            let cmds = bot.decide_next_commands();
            check!(!cmds.is_empty(), "expected non-empty cluster command batch");
            for cmd in cmds {
                let events = tw.run(pilot, &cmd);
                bot.observe_events(&events);
            }
        }

        // Verify the bot disabled `claim_jumper` into a derelict, clamped in 6-DOF (`space.docking`),
        // and registered the 48-hour salvage tow claim in `SpaceBook.salvage_book`!
        let dock_snap = bot
            .latest_docking
            .as_ref()
            .ok_or("missing docking snapshot after 6-DOF hard-dock")?;
        check_eq!(dock_snap.status, "CLAMPED");
        check_eq!(dock_snap.target_kind, "disabled_derelict");

        // Also verify `compute_6dof_rcs_control` steers a misaligned approach snapshot toward alignment
        let misaligned = DockingSnapshot {
            dx_mm: 220,
            dy_mm: -180,
            dz_mm: 900,
            pitch_mdeg: 1_200,
            yaw_mdeg: -1_100,
            roll_mdeg: 1_500,
            vx_mmps: 0,
            vy_mmps: 0,
            vz_mmps: 50,
            dampeners: false,
            status: "APPROACHING".to_owned(),
            ..DockingSnapshot::default()
        };
        let rcs = compute_6dof_rcs_control(&misaligned);
        check_eq!(rcs.trans_x, -1);
        check_eq!(rcs.trans_y, 1);
        check_eq!(rcs.pitch_trim, -1);
        check_eq!(rcs.yaw_trim, 1);
        check_eq!(rcs.roll_trim, -1);
        check_eq!(rcs.closure_pulse, 1);
        check!(rcs.toggle_dampeners);

        // 3. Story 2 (Part B): Switch autonomous pilot to `OuterSystemExpedition`
        //    to decouple, fit drop tanks, fly out to Neptune & Triton, and recouple autopilot!
        bot.set_goal(SpaceMissionGoal::OuterSystemExpedition {
            from_body: "ceres".to_owned(),
            to_body: "mars".to_owned(),
            expedition_body: "neptune".to_owned(),
        });
        while !bot.is_goal_complete() {
            let cmds = bot.decide_next_commands();
            check!(
                !cmds.is_empty(),
                "expected non-empty expedition command batch"
            );
            for cmd in cmds {
                let events = tw.run(pilot, &cmd);
                bot.observe_events(&events);
            }
        }
        let cockpit2 = bot
            .latest_cockpit
            .as_ref()
            .ok_or("missing canopy snapshot after Neptune expedition")?;
        check!(
            cockpit2.visible_bodies.iter().any(|b| b == "neptune"),
            "expected Neptune visible on space.cockpit canopy"
        );
        check!(
            cockpit2.visible_bodies.iter().any(|b| b == "triton"),
            "expected Triton visible on space.cockpit canopy"
        );

        Ok(())
    }
}

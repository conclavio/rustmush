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
pub mod picket;
pub mod weapons;

pub use bubble::{
    BubbleId, BubbleStepReport, DERELICT_WINDOW_S, EmconPosture, FacetBand, HullStatus,
    LocalSpaceBubble, MAX_SUBSTEP_MS, ShipSectionId, ShipSectionState, SpaceCombatEvent,
    SpacePartyId, SpaceRelation, SpaceShipState, SubsystemCrit,
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
}

//! **Live `CarrierJourney` Interdiction Auto-Promotion & Two-Tier Clock Pulse (`SF.17`)**
//! (`docs/design/35-ship-free-flight-combat-and-mining.md` §5–§6, `D-35-3`, `D-35-5`, `Q-CF-32`, `Q-CF-33`).
//!
//! Coordinates the two-tier space simulation clock and automatic leg interdiction promotion
//! inside [`super::commands::SpaceBook`]:
//! 1. **Automatic `CarrierJourney` Interdiction Promotion ([`InterdictionLegWatch`], `D-35-5`)**:
//!    - Watches a filed transit leg (`from_body -> to_body`, `[depart_ic, arrive_ic]`) with an
//!      interdiction site (`kirkwood_picket`, `kirkwood_gap_3_1`, `pallas_inclined_belt`,
//!      `ceres_halo`, `saturn_b_ring_edge`).
//!    - When the world IC clock `now_ic` reaches or crosses `interdiction_at_ic`, automatically
//!      pauses the filed leg (`promoted = true`) and promotes the hull into a live 1:1 wall-time
//!      [`super::picket::KirkwoodPicketEncounter`] or [`super::mining::AsteroidClusterField`]
//!      inside [`super::commands::SpaceBook`].
//! 2. **Two-Tier Clock Pulse ([`pulse_space_book`], `D-35-3`, `D-35-14`)**:
//!    - Advances Tier-1 [`super::flight::SpaceFlightRecord`]s by `dt_ic_s` on the `WorldTimeline`
//!      IC clock (skipping any ship currently paused in an out-of-time-loop 6-DOF
//!      [`super::docking::DockingSession`], `Q-35-5`).
//!    - Advances Tier-2 [`super::bubble::LocalSpaceBubble`]s, clusters, and customs pickets by
//!      `dt_wall_ms` on **1:1 wall time** (`Q-CF-32`), respecting `outage_frozen` (`Q-CF-33`)
//!      and synchronizing `TacticalUnit` damage & `SalvageClaimBook` forensics (`SF.10`).

use mush_core::ObjId;
use serde::{Deserialize, Serialize};

use crate::sky::Sky;
use crate::space::bubble::{LocalSpaceBubble, SpacePartyId, SpaceShipState};
use crate::space::commands::{ActiveSpaceContext, SpaceBook};
use crate::space::mining::{AsteroidClusterField, OreHoldState};
use crate::space::picket::{HaulerManifest, KirkwoodPicketEncounter};
use crate::space::weapons::LaserMount;

/// A watched `CarrierJourney` transit leg with an interdiction trigger (`D-35-5`, `SF.17`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterdictionLegWatch {
    pub ship_id: String,
    pub pilot: ObjId,
    pub from_body: String,
    pub to_body: String,
    pub depart_ic: i64,
    pub arrive_ic: i64,
    /// IC timestamp along `[depart_ic, arrive_ic]` where interdiction promotes the journey.
    pub interdiction_at_ic: i64,
    /// Site or encounter identifier (`kirkwood_picket`, `pallas_inclined_belt`, etc.).
    pub interdiction_site: String,
    /// Whether the leg has been paused and promoted into a live Tier-2 bubble (`JourneyState::Promoted`).
    pub promoted: bool,
}

impl InterdictionLegWatch {
    /// Create a new watched transit leg that triggers interdiction at the midpoint of `[depart_ic, arrive_ic]`.
    #[must_use]
    pub fn new(
        ship_id: impl Into<String>,
        pilot: ObjId,
        from_body: impl Into<String>,
        to_body: impl Into<String>,
        depart_ic: i64,
        arrive_ic: i64,
        interdiction_site: impl Into<String>,
    ) -> Self {
        let mid_ic = depart_ic.saturating_add((arrive_ic.saturating_sub(depart_ic)) / 2);
        Self {
            ship_id: ship_id.into(),
            pilot,
            from_body: from_body.into(),
            to_body: to_body.into(),
            depart_ic,
            arrive_ic,
            interdiction_at_ic: mid_ic,
            interdiction_site: interdiction_site.into(),
            promoted: false,
        }
    }
}

/// Discrete event emitted during a [`pulse_space_book`] tick (`SF.17`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpacePulseEvent {
    /// A filed transit leg reached `interdiction_at_ic` and promoted into a Tier-2 space bubble.
    LegInterdicted {
        ship_id: String,
        pilot: ObjId,
        site_id: String,
        at_ic: i64,
    },
    /// A Tier-1 free-flying ship advanced by `dt_ic_s` on the IC clock.
    FreeFlightAdvanced {
        ship_id: String,
        dt_ic_s: u64,
        pos_km: [i64; 3],
        remaining_dv_mps: i64,
    },
    /// A Tier-2 1:1 wall-time bubble advanced by `dt_wall_ms`.
    BubbleAdvanced {
        bubble_id: String,
        dt_wall_ms: u32,
        combat_events: usize,
    },
    /// A ship paused in a 6-DOF `DockingSession` was held out of the world time loop (`Q-35-5`).
    DockingSessionPaused { ship_id: String },
}

/// Summary report returned by [`pulse_space_book`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpacePulseReport {
    pub now_ic: i64,
    pub dt_ic_s: u64,
    pub dt_wall_ms: u32,
    pub events: Vec<SpacePulseEvent>,
}

/// Pulse [`SpaceBook`] across a world tick (`now_ic`, `dt_ic_s`) and 1:1 wall-clock step (`dt_wall_ms`, `SF.17`):
/// 1. Promotes any [`InterdictionLegWatch`] whose `interdiction_at_ic <= now_ic`.
/// 2. Advances active Tier-1 `SpaceFlightRecord`s by `dt_ic_s` (skipping ships paused in `book.dockings`).
/// 3. Steps active Tier-2 1:1 wall-time bubbles (`pickets`, `clusters`, `bubbles`) by `dt_wall_ms`
///    and synchronizes `TacticalUnit` damage and `SalvageClaimBook` black-box forensics.
pub fn pulse_space_book(
    book: &mut SpaceBook,
    sky: Option<&Sky>,
    now_ic: i64,
    dt_ic_s: u64,
    dt_wall_ms: u32,
) -> SpacePulseReport {
    let mut events = Vec::new();

    // 1. Check watched legs for automatic interdiction promotion (`D-35-5`)
    let mut to_promote: Vec<(String, ObjId, String)> = Vec::new();
    for watch in book.leg_watches.values_mut() {
        if !watch.promoted && now_ic >= watch.interdiction_at_ic {
            watch.promoted = true;
            to_promote.push((
                watch.ship_id.clone(),
                watch.pilot,
                watch.interdiction_site.clone(),
            ));
        }
    }

    for (ship_id, pilot, site) in to_promote {
        book.actor_ships.insert(pilot, ship_id.clone());
        if site == "kirkwood_picket" {
            let enc = KirkwoodPicketEncounter::promote_interception(
                now_ic,
                HaulerManifest {
                    strategic_ore_kg: 18_000,
                    metal_ore_kg: 12_000,
                    valid_registry_claim: true,
                    contraband_aboard: false,
                },
            );
            book.pickets.insert(ship_id.clone(), enc);
            book.ship_context
                .insert(ship_id.clone(), ActiveSpaceContext::Picket(ship_id.clone()));
            book.sync_bubble_damage_and_forensics(&ship_id, None, &[], now_ic);
        } else if matches!(
            site.as_str(),
            "ceres_halo" | "kirkwood_gap_3_1" | "pallas_inclined_belt" | "saturn_b_ring_edge"
        ) {
            let field = book
                .clusters
                .entry(site.clone())
                .or_insert_with(|| AsteroidClusterField::new_canonical_cluster(&site, now_ic));
            let mut dray_p = SpaceShipState::new_dray(
                &ship_id,
                "UNS-PROSPECTOR",
                SpacePartyId(1),
                [0, 0, 0],
                [0, 0, 0],
            );
            dray_p
                .lasers
                .push(LaserMount::standard_turret("turret_laser_1"));
            field.insert_ship_with_hold(dray_p, OreHoldState::new_dray_p());
            book.ship_context
                .insert(ship_id.clone(), ActiveSpaceContext::Cluster(site.clone()));
            book.sync_bubble_damage_and_forensics(&ship_id, None, &[], now_ic);
        } else {
            let bubble =
                LocalSpaceBubble::new(format!("bubble.{site}"), &site, site.to_uppercase(), now_ic);
            let dray = SpaceShipState::new_dray(
                &ship_id,
                "DRAY HAULER",
                SpacePartyId(1),
                [0, 0, 0],
                [0, 0, 0],
            );
            let entry = book.bubbles.entry(site.clone()).or_insert(bubble);
            entry.insert_ship(dray);
            book.ship_context
                .insert(ship_id.clone(), ActiveSpaceContext::Bubble(site.clone()));
            book.sync_bubble_damage_and_forensics(&ship_id, None, &[], now_ic);
        }
        events.push(SpacePulseEvent::LegInterdicted {
            ship_id,
            pilot,
            site_id: site,
            at_ic: now_ic,
        });
    }

    // 2. Record ships paused in out-of-time-loop 6-DOF docking sessions (`Q-35-5`, `D-35-14`)
    let paused_ships: Vec<String> = book.dockings.keys().cloned().collect();
    for sid in &paused_ships {
        events.push(SpacePulseEvent::DockingSessionPaused {
            ship_id: sid.clone(),
        });
    }

    // 3. Step Tier-1 System Free Flight records on the IC clock (`D-35-3`)
    if dt_ic_s > 0
        && let Some(sky_ref) = sky
    {
        for (sid, flight) in &mut book.flights {
            if paused_ships.contains(sid) {
                continue;
            }
            flight.step_ic_seconds(sky_ref, dt_ic_s);
            let p = flight.pos_km();
            events.push(SpacePulseEvent::FreeFlightAdvanced {
                ship_id: sid.clone(),
                dt_ic_s,
                pos_km: [p.x, p.y, p.z],
                remaining_dv_mps: flight.delta_v_mps,
            });
        }
    }

    // 4. Step Tier-2 1:1 wall-time LocalSpaceBubbles (`D-35-3`, `Q-CF-32`)
    if dt_wall_ms > 0 {
        let picket_keys: Vec<String> = book.pickets.keys().cloned().collect();
        for key in picket_keys {
            if paused_ships.contains(&key) {
                continue;
            }
            if let Some(enc) = book.pickets.get_mut(&key) {
                let rep = enc.step_wall_ms(dt_wall_ms);
                let c_events = rep.events.clone();
                let bid = enc.bubble.id.0.clone();
                book.sync_bubble_damage_and_forensics(&key, None, &c_events, now_ic);
                events.push(SpacePulseEvent::BubbleAdvanced {
                    bubble_id: bid,
                    dt_wall_ms: rep.wall_advanced_ms,
                    combat_events: c_events.len(),
                });
            }
        }

        let cluster_keys: Vec<String> = book.clusters.keys().cloned().collect();
        for cid in cluster_keys {
            if let Some(cluster) = book.clusters.get_mut(&cid) {
                let rep = cluster.bubble.step_wall_ms(dt_wall_ms);
                let c_events = rep.events.clone();
                let bid = cluster.bubble.id.0.clone();
                let ship_ids: Vec<String> = cluster.bubble.ships.keys().cloned().collect();
                for sid in ship_ids {
                    book.sync_bubble_damage_and_forensics(&sid, None, &c_events, now_ic);
                }
                events.push(SpacePulseEvent::BubbleAdvanced {
                    bubble_id: bid,
                    dt_wall_ms: rep.wall_advanced_ms,
                    combat_events: c_events.len(),
                });
            }
        }

        let bubble_keys: Vec<String> = book.bubbles.keys().cloned().collect();
        for bid_key in bubble_keys {
            if let Some(bubble) = book.bubbles.get_mut(&bid_key) {
                let rep = bubble.step_wall_ms(dt_wall_ms);
                let c_events = rep.events.clone();
                let bid = bubble.id.0.clone();
                let ship_ids: Vec<String> = bubble.ships.keys().cloned().collect();
                for sid in ship_ids {
                    book.sync_bubble_damage_and_forensics(&sid, None, &c_events, now_ic);
                }
                events.push(SpacePulseEvent::BubbleAdvanced {
                    bubble_id: bid,
                    dt_wall_ms: rep.wall_advanced_ms,
                    combat_events: c_events.len(),
                });
            }
        }
    }

    SpacePulseReport {
        now_ic,
        dt_ic_s,
        dt_wall_ms,
        events,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::space::docking::{DockingPose6Dof, DockingSession};

    #[test]
    fn interdiction_watch_auto_promotes_at_midpoint_and_docking_pauses_ship_from_pulse() {
        let mut book = SpaceBook::default();
        let pilot = ObjId(42);
        let watch = InterdictionLegWatch::new(
            "ship.42",
            pilot,
            "pallas",
            "ceres",
            1_000,
            2_000,
            "kirkwood_picket",
        );
        assert_eq!(watch.interdiction_at_ic, 1_500);
        book.leg_watches.insert("ship.42".to_owned(), watch);

        // Before midpoint (t = 1,200): leg is not yet interdicted
        let rep_early = pulse_space_book(&mut book, None, 1_200, 60, 0);
        assert!(rep_early.events.is_empty());
        assert!(!book.pickets.contains_key("ship.42"));

        // At midpoint (t = 1,500): leg automatically promotes into KirkwoodPicketEncounter!
        let rep_mid = pulse_space_book(&mut book, None, 1_500, 60, 1_000);
        assert!(
            rep_mid
                .events
                .iter()
                .any(|e| matches!(e, SpacePulseEvent::LegInterdicted { ship_id, site_id, .. } if ship_id == "ship.42" && site_id == "kirkwood_picket"))
        );
        assert!(book.pickets.contains_key("ship.42"));

        // Enter a 6-DOF docking session on ship.42 -> subsequent pulse holds ship.42 paused (`Q-35-5`)
        let dock = DockingSession::start_port_approach(
            "dock.42",
            "ship.42",
            SpacePartyId(1),
            400,
            "ceres_port",
            "BERTH-1",
            DockingPose6Dof {
                dx_mm: 0,
                dy_mm: 0,
                dz_mm: 800,
                pitch_mdeg: 0,
                yaw_mdeg: 0,
                roll_mdeg: 0,
                vx_mmps: 0,
                vy_mmps: 0,
                vz_mmps: 400,
                vpitch_mdeg_s: 0,
                vyaw_mdeg_s: 0,
                vroll_mdeg_s: 0,
            },
        );
        book.dockings.insert("ship.42".to_owned(), dock);
        let rep_paused = pulse_space_book(&mut book, None, 1_600, 60, 1_000);
        assert!(
            rep_paused
                .events
                .iter()
                .any(|e| matches!(e, SpacePulseEvent::DockingSessionPaused { ship_id } if ship_id == "ship.42"))
        );
    }
}

//! **The Kirkwood Customs Picket** (`docs/design/35-ship-free-flight-combat-and-mining.md`
//! §9 Story 1, `D-35-5`, `D-35-11`).
//!
//! When a voyage leg with `LegDef.interdiction` reaches the Kirkwood 3:1 resonance
//! boundary (`PromotionTrigger::Interception`), the carrier promotes into
//! `JourneyState::Promoted` (pausing the leg cursor) and enters the Kirkwood
//! [`LocalSpaceBubble`] alongside:
//! - **Republic Customs Corvette (`"REV VIGILANT"`, Party 2)** — `Neutral`/`Inspecting`,
//!   hailing the hauler to heave-to ($|\Delta \mathbf{v}| \le 10\text{ m/s}$) for a
//!   manifest and Freeport Registry claim check.
//! - **Belt Raider (`"RAIDER KESTREL"`, Party 3)** — `Hostile`, lurking on EMCON Ghost
//!   until it lights its torch to launch a torpedo salvo and spinal railgun attack.

use serde::{Deserialize, Serialize};

use crate::space::bubble::{
    BubbleStepReport, HullStatus, LocalSpaceBubble, SpaceCombatEvent, SpacePartyId, SpaceRelation,
    SpaceShipState,
};
use crate::space::weapons::isqrt_i128;

/// Maximum relative speed in $\text{m/s}$ to satisfy the customs heave-to order (`Story 1`).
pub const HEAVE_TO_MAX_REL_MPS: i64 = 10;

/// Party IDs in a standard Kirkwood Picket encounter.
pub const PARTY_HAULER: SpacePartyId = SpacePartyId(1);
pub const PARTY_CUSTOMS: SpacePartyId = SpacePartyId(2);
pub const PARTY_RAIDER: SpacePartyId = SpacePartyId(3);

/// Canonical ship IDs in the Kirkwood Customs Picket encounter.
pub const HAULER_SHIP_ID: &str = "ship.player_dray_p";
pub const CUSTOMS_SHIP_ID: &str = "ship.customs_vigilant";
pub const RAIDER_SHIP_ID: &str = "ship.raider_kestrel";

/// Cargo / Registry paper state of the arriving hauler (`D-35-11`, `D-35-13`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HaulerManifest {
    /// Kilograms of `strategic_ore` in `stow.ore`.
    pub strategic_ore_kg: u32,
    /// Kilograms of `metal_ore` in `stow.ore`.
    pub metal_ore_kg: u32,
    /// Whether the hauler holds a valid Freeport Registry prospecting licence and claim paper.
    pub valid_registry_claim: bool,
    /// Whether the hold carries prohibited contraband.
    pub contraband_aboard: bool,
}

/// Phase of the Kirkwood Customs Picket encounter (`D-35-11`, Story 1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PicketPhase {
    /// Customs has hailed the arriving ship on radio; waiting for heave-to ($|\Delta \mathbf{v}| \le 10\text{ m/s}$).
    HailedToHeaveTo { radio_notice: String },
    /// Ship has matched velocity within $10\text{ m/s}$; customs is scanning `stow.ore` and Registry paper.
    HeavedToInspecting { scan_progress_ms: u32 },
    /// Belt Raider has lit its torch / fired ordnance; customs corvette and player coordinate defense.
    RaiderAmbushActive,
    /// Encounter has concluded and the ship is cleared to un-promote (`JourneyState`) or dock.
    Resolved(PicketOutcome),
}

/// Final resolution of a Kirkwood Customs Picket encounter (`D-35-11`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PicketOutcome {
    /// Cleared to resume passage (either clean manifest/claim paper or mutually defended against raider).
    ClearedToProceed {
        raider_disabled_derelict: bool,
        customs_screen_assisted: bool,
    },
    /// Fined for un-papered ore or minor violation, advancing the Customs Assertion front.
    Fined {
        fine_credits: u64,
        customs_assertion_delta: i32,
    },
    /// Hauler burned past the picket perimeter without submitting to inspection.
    RanThePicket { customs_assertion_delta: i32 },
}

/// State machine wrapping a [`LocalSpaceBubble`] at the Kirkwood Customs Picket (`Story 1`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KirkwoodPicketEncounter {
    pub bubble: LocalSpaceBubble,
    pub manifest: HaulerManifest,
    pub phase: PicketPhase,
    pub customs_screen_intercepts: u32,
    pub journey_promoted_paused: bool,
}

impl KirkwoodPicketEncounter {
    /// Promote a voyage at the Kirkwood leg boundary (`PromotionTrigger::Interception`)
    /// into a live 3-party [`LocalSpaceBubble`] (`Story 1`).
    #[must_use]
    pub fn promote_interception(ic_timestamp_s: i64, manifest: HaulerManifest) -> Self {
        let mut bubble = LocalSpaceBubble::new(
            "bubble.kirkwood_picket",
            "kirkwood_gap_3_1",
            "KIRKWOOD CUSTOMS PICKET",
            ic_timestamp_s,
        );

        // 1. Player Dray-P arriving at origin with 35 m/s forward drift (`[35_000, 0, 0] mm/s`)
        let mut hauler = SpaceShipState::new_dray(
            HAULER_SHIP_ID,
            "DRAY-P AURORA",
            PARTY_HAULER,
            [0, 0, 0],
            [35_000, 0, 0],
        );
        hauler.pdc_auto = true;
        hauler.locked_target_id = Some(RAIDER_SHIP_ID.to_owned());
        bubble.insert_ship(hauler);

        // 2. Republic Customs Corvette "REV VIGILANT" stationed 8 km ahead/port (`[8_000, 2_000, 500] m`)
        //    so its 15 km automatic PDC envelope covers the hauler when heaved-to!
        let mut customs = SpaceShipState::new_corvette(
            CUSTOMS_SHIP_ID,
            "REV VIGILANT",
            PARTY_CUSTOMS,
            [8_000, 2_000, 500],
            [0, 0, 0],
        );
        customs.pdc_auto = true;
        customs.locked_target_id = Some(RAIDER_SHIP_ID.to_owned());
        bubble.insert_ship(customs);

        // 3. Belt Raider "KESTREL" lurking 120 km out at high elevation (`[120_000, -15_000, 12_000] m`)
        let mut raider = SpaceShipState::new_corvette(
            RAIDER_SHIP_ID,
            "RAIDER KESTREL",
            PARTY_RAIDER,
            [120_000, -15_000, 12_000],
            [-50_000, 0, 0],
        );
        raider.locked_target_id = Some(HAULER_SHIP_ID.to_owned());
        bubble.insert_ship(raider);

        bubble.set_party_relation(PARTY_HAULER, PARTY_CUSTOMS, SpaceRelation::Inspecting);
        bubble.set_party_relation(PARTY_HAULER, PARTY_RAIDER, SpaceRelation::Hostile);
        bubble.set_party_relation(PARTY_CUSTOMS, PARTY_RAIDER, SpaceRelation::Hostile);

        Self {
            bubble,
            manifest,
            phase: PicketPhase::HailedToHeaveTo {
                radio_notice:
                    "REV VIGILANT [PICKET]: Hauler Aurora, heave to within 10 m/s for Registry & stow.ore inspection."
                        .to_owned(),
            },
            customs_screen_intercepts: 0,
            journey_promoted_paused: true,
        }
    }

    /// Relative speed in whole $\text{m/s}$ between the player hauler and the customs corvette.
    #[must_use]
    pub fn relative_speed_to_customs_mps(&self) -> Option<i64> {
        let hauler = self.bubble.ships.get(HAULER_SHIP_ID)?;
        let customs = self.bubble.ships.get(CUSTOMS_SHIP_ID)?;
        let dvx = ((hauler.vel_mmps[0] - customs.vel_mmps[0]) / 1_000) as i128;
        let dvy = ((hauler.vel_mmps[1] - customs.vel_mmps[1]) / 1_000) as i128;
        let dvz = ((hauler.vel_mmps[2] - customs.vel_mmps[2]) / 1_000) as i128;
        Some(isqrt_i128(dvx * dvx + dvy * dvy + dvz * dvz) as i64)
    }

    /// Trigger the Belt Raider's ambush (`Story 1` step 3): lights torch, launches a 2-torpedo
    /// salvo at the player hauler, and fires a $120\text{ km}$ spinal railgun slug.
    pub fn trigger_raider_ambush(&mut self) -> Vec<SpaceCombatEvent> {
        self.phase = PicketPhase::RaiderAmbushActive;
        // Customs corvette immediately upgrades relation with the compliant hauler to Friendly
        self.bubble
            .set_party_relation(PARTY_HAULER, PARTY_CUSTOMS, SpaceRelation::Friendly);

        let mut events = self
            .bubble
            .fire_torpedo_salvo(RAIDER_SHIP_ID, HAULER_SHIP_ID, 2);
        events.extend(self.bubble.fire_railgun(RAIDER_SHIP_ID, HAULER_SHIP_ID));
        events
    }

    /// Advance the Kirkwood encounter by `dt_wall_ms` on 1:1 wall time (`D-35-3`).
    pub fn step_wall_ms(&mut self, dt_wall_ms: u32) -> BubbleStepReport {
        let report = self.bubble.step_wall_ms(dt_wall_ms);

        for ev in &report.events {
            if let SpaceCombatEvent::TorpedoInterceptedByPdc { defender_id, .. }
            | SpaceCombatEvent::TorpedoInterceptedByLaser { defender_id, .. } = ev
                && defender_id == CUSTOMS_SHIP_ID
            {
                self.customs_screen_intercepts = self.customs_screen_intercepts.saturating_add(1);
            }
        }

        // Transition phase based on relative velocity, ambush state, and raider disablement
        match &mut self.phase {
            PicketPhase::HailedToHeaveTo { .. } => {
                if let Some(rel_mps) = self.relative_speed_to_customs_mps()
                    && rel_mps <= HEAVE_TO_MAX_REL_MPS
                {
                    self.phase = PicketPhase::HeavedToInspecting {
                        scan_progress_ms: 0,
                    };
                }
            }
            PicketPhase::HeavedToInspecting { scan_progress_ms } => {
                *scan_progress_ms = scan_progress_ms.saturating_add(dt_wall_ms);
                if *scan_progress_ms >= 3_000 {
                    let outcome = if !self.manifest.valid_registry_claim
                        && (self.manifest.strategic_ore_kg > 0 || self.manifest.metal_ore_kg > 0)
                    {
                        PicketOutcome::Fined {
                            fine_credits: 1_500,
                            customs_assertion_delta: 1,
                        }
                    } else {
                        PicketOutcome::ClearedToProceed {
                            raider_disabled_derelict: false,
                            customs_screen_assisted: self.customs_screen_intercepts > 0,
                        }
                    };
                    self.phase = PicketPhase::Resolved(outcome);
                    self.journey_promoted_paused = false;
                }
            }
            PicketPhase::RaiderAmbushActive => {
                let raider_derelict = self
                    .bubble
                    .ships
                    .get(RAIDER_SHIP_ID)
                    .is_some_and(|s| matches!(s.status, HullStatus::DisabledDerelict { .. }));
                if raider_derelict && self.bubble.torpedoes.is_empty() {
                    self.phase = PicketPhase::Resolved(PicketOutcome::ClearedToProceed {
                        raider_disabled_derelict: true,
                        customs_screen_assisted: self.customs_screen_intercepts > 0,
                    });
                    self.journey_promoted_paused = false;
                }
            }
            PicketPhase::Resolved(_) => {}
        }

        report
    }
}

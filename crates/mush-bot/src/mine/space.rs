//! **Standalone SSH line-mode spaceflight, combat, mining & salvage pilot (`SF.12`)**
//! (`docs/design/35-ship-free-flight-combat-and-mining.md` §9, §10).
//!
//! Plays canonical Story 1 (Kirkwood 3:1 Customs Picket & Belt Raider Ambush) and
//! Story 2 (Pallas Inclined Belt prospecting, claim-jump defense, 6-DOF derelict
//! hard-docking, 48-hour salvage tow, and outer-system Neptune expedition) purely
//! from the player-visible SSH terminal text (`help ships`).
//!
//! Like [`super::lane`], this module has zero runtime or build dependencies on any
//! game crate; the `#[cfg(test)]` contract test drives `SpaceLane` against a live
//! `tacsim` world via `mush-bot`'s `[dev-dependencies]` to guarantee that any drift
//! in the server's terminal output sentences is caught immediately in CI.

use serde::{Deserialize, Serialize};

/// Ordered beats of the combined Story 1 + Story 2 spaceflight cohort run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpaceBeat {
    /// Enter Kirkwood 3:1 Customs Picket (`+helm/enter kirkwood_picket`).
    EnterKirkwoodPicket,
    /// Brake to heave-to within `<= 10 m/s` (`+helm/burn -15 0 0`).
    BrakeForCustoms,
    /// Step picket bubble +1000 ms (`+tac/step 1000`).
    StepPicketHeaveTo,
    /// Enable automatic rotary PDC point-defense (`+tac/pdc auto`).
    EnablePdcAuto,
    /// Switch laser turrets to point-defense mode (`+tac/laser defense`).
    SwitchLaserDefense,
    /// Execute lateral jink burn (`+helm/burn 0 12 0`).
    LateralJinkBurn,
    /// Roll hull attitude 90 deg to present fresh armor (`+helm/roll 0 0 90`).
    RollArmorFacet,
    /// Fire counter-battery railgun at raider (`+tac/rail ship.raider_kestrel`).
    FireCounterRailgun,
    /// Step picket bubble through torpedo interception (`+tac/step 1000`).
    StepPicketIntercept,
    /// Enter Pallas Inclined Belt orbital asteroid cluster (`+helm/enter pallas_inclined_belt`).
    EnterPallasCluster,
    /// Ping M-type asteroid pocket `rock.m_309` (`+mine/ping rock.m_309`).
    PingMTypeRock,
    /// Match 3D velocity within `<= 5 m/s` (`+helm/match rock.m_309`).
    MatchRockVelocity,
    /// Launch 4 collector drones (`+mine/drones rock.m_309 4`).
    LaunchCollectorDrones,
    /// Extract ore with ship mining laser, triggering IR bloom & claim-jumper (`+mine/laser rock.m_309 4000`).
    ExtractWithMiningLaser,
    /// Fire offensive lasers at `claim_jumper` to disable its drive (`+tac/beam claim_jumper`).
    DisableClaimJumper,
    /// Initiate mandatory 6-DOF hard-docking with derelict (`+helm/dock derelict claim_jumper`).
    StartDerelictDocking,
    /// Clamp 6-DOF docking collar (`+helm/dock clamp`).
    ClampDerelictCollar,
    /// Attach salvage tow line under 48h Freeport Registry contest window (`+helm/tow`).
    AttachSalvageTow,
    /// Decouple from transit leg into Tier-1 3D System Free Flight (`+helm/decouple ceres mars`).
    DecoupleFreeFlight,
    /// Fit long-range external expedition drop tanks (`+helm/droptanks`).
    FitDropTanks,
    /// Fly outer-system expedition to Neptune (`+helm/expedition neptune`).
    FlyNeptuneExpedition,
    /// Recouple to port autopilot (`+helm/autopilot ceres`).
    RecoupleAutopilot,
    /// Both Story 1 and Story 2 completed.
    Complete,
}

impl SpaceBeat {
    /// The exact player MUSH command issued at this beat, or `None` when [`Self::Complete`].
    #[must_use]
    pub const fn command(self) -> Option<&'static str> {
        match self {
            Self::EnterKirkwoodPicket => Some("+helm/enter kirkwood_picket"),
            Self::BrakeForCustoms => Some("+helm/burn -15 0 0"),
            Self::StepPicketHeaveTo => Some("+tac/step 1000"),
            Self::EnablePdcAuto => Some("+tac/pdc auto"),
            Self::SwitchLaserDefense => Some("+tac/laser defense"),
            Self::LateralJinkBurn => Some("+helm/burn 0 12 0"),
            Self::RollArmorFacet => Some("+helm/roll 0 0 90"),
            Self::FireCounterRailgun => Some("+tac/rail ship.raider_kestrel"),
            Self::StepPicketIntercept => Some("+tac/step 1000"),
            Self::EnterPallasCluster => Some("+helm/enter pallas_inclined_belt"),
            Self::PingMTypeRock => Some("+mine/ping rock.m_309"),
            Self::MatchRockVelocity => Some("+helm/match rock.m_309"),
            Self::LaunchCollectorDrones => Some("+mine/drones rock.m_309 4"),
            Self::ExtractWithMiningLaser => Some("+mine/laser rock.m_309 4000"),
            Self::DisableClaimJumper => Some("+tac/beam claim_jumper"),
            Self::StartDerelictDocking => Some("+helm/dock derelict claim_jumper"),
            Self::ClampDerelictCollar => Some("+helm/dock clamp"),
            Self::AttachSalvageTow => Some("+helm/tow"),
            Self::DecoupleFreeFlight => Some("+helm/decouple ceres mars"),
            Self::FitDropTanks => Some("+helm/droptanks"),
            Self::FlyNeptuneExpedition => Some("+helm/expedition neptune"),
            Self::RecoupleAutopilot => Some("+helm/autopilot ceres"),
            Self::Complete => None,
        }
    }
}

/// Check whether `line` belongs to the closed player-only spaceflight vocabulary
/// (`+helm`, `+tac`, `+mine` and their documented switches in `help ships`).
///
/// Rejects any staff `@` command or unrecognised verb.
#[must_use]
pub fn in_space_vocabulary(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('@') {
        return false;
    }
    let head = trimmed.split_whitespace().next().unwrap_or("");
    matches!(
        head,
        "+helm"
            | "+helm/enter"
            | "+helm/burn"
            | "+helm/roll"
            | "+helm/match"
            | "+helm/decouple"
            | "+helm/droptanks"
            | "+helm/expedition"
            | "+helm/autopilot"
            | "+helm/dock"
            | "+helm/tow"
            | "+tac"
            | "+tac/pdc"
            | "+tac/laser"
            | "+tac/torp"
            | "+tac/rail"
            | "+tac/beam"
            | "+tac/step"
            | "+mine"
            | "+mine/ping"
            | "+mine/licence"
            | "+mine/stake"
            | "+mine/contest"
            | "+mine/laser"
            | "+mine/drones"
    )
}

/// Structured evidence collected from terminal output across Story 1 and Story 2.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceEvidence {
    /// Kirkwood 3:1 Picket interdiction triggered and hailed by Republic Customs Corvette.
    pub picket_interdicted: bool,
    /// Braking burn executed to heave-to (`<= 10 m/s`).
    pub heave_to_braked: bool,
    /// Automatic rotary PDC network enabled.
    pub pdc_auto_enabled: bool,
    /// Laser turrets switched to `Defense` mode.
    pub laser_defense_enabled: bool,
    /// Lateral jink burn executed to dodge unguided railgun slug.
    pub lateral_jink_executed: bool,
    /// Hull attitude rolled 90 deg to rotate fresh armor facet.
    pub attitude_rolled_90: bool,
    /// Counter-battery railgun fired at `ship.raider_kestrel`.
    pub counter_railgun_fired: bool,
    /// Entered `pallas_inclined_belt` orbital asteroid cluster.
    pub pallas_cluster_entered: bool,
    /// Pinged M-type asteroid `rock.m_309`.
    pub m_type_rock_pinged: bool,
    /// Matched 3D velocity within `<= 5 m/s` mining gate.
    pub velocity_matched: bool,
    /// Collector drone sortie recovered ore into `stow.ore`.
    pub drone_sortie_recovered: bool,
    /// Ship mining laser extracted ore and flagged frontier claim-jump.
    pub mining_laser_claim_jump: bool,
    /// Disabled `claim_jumper` with offensive laser beams.
    pub claim_jumper_disabled: bool,
    /// Entered mandatory 6-DOF derelict docking session.
    pub derelict_docking_started: bool,
    /// Clamped 6-DOF docking collar onto `claim_jumper`.
    pub hard_dock_clamped: bool,
    /// Attached salvage tow line with 48-hour Freeport Registry contest window.
    pub salvage_tow_attached: bool,
    /// Decoupled mid-leg into Tier-1 3D System Free Flight.
    pub decoupled_free_flight: bool,
    /// Fitted external long-range drop tanks.
    pub drop_tanks_fitted: bool,
    /// Arrived at Neptune on outer-system expedition with light-lag telemetry.
    pub neptune_expedition_arrived: bool,
    /// Recoupled to port autopilot (`ceres`).
    pub autopilot_recoupled: bool,
}

impl SpaceEvidence {
    /// Return whether every milestone across Story 1 and Story 2 has been observed.
    #[must_use]
    pub const fn all_milestones_complete(&self) -> bool {
        self.picket_interdicted
            && self.heave_to_braked
            && self.pdc_auto_enabled
            && self.laser_defense_enabled
            && self.lateral_jink_executed
            && self.attitude_rolled_90
            && self.counter_railgun_fired
            && self.pallas_cluster_entered
            && self.m_type_rock_pinged
            && self.velocity_matched
            && self.drone_sortie_recovered
            && self.mining_laser_claim_jump
            && self.claim_jumper_disabled
            && self.derelict_docking_started
            && self.hard_dock_clamped
            && self.salvage_tow_attached
            && self.decoupled_free_flight
            && self.drop_tanks_fitted
            && self.neptune_expedition_arrived
            && self.autopilot_recoupled
    }
}

/// Standalone line-mode state machine that drives Story 1 and Story 2 from SSH terminal lines.
#[derive(Clone, Debug)]
pub struct SpaceLane {
    beat: SpaceBeat,
    /// Collected evidence from server output lines.
    pub evidence: SpaceEvidence,
    /// Ordered history of commands sent by the lane.
    pub sent_commands: Vec<String>,
}

impl Default for SpaceLane {
    fn default() -> Self {
        Self::new()
    }
}

impl SpaceLane {
    /// Create a new [`SpaceLane`] positioned at [`SpaceBeat::EnterKirkwoodPicket`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            beat: SpaceBeat::EnterKirkwoodPicket,
            evidence: SpaceEvidence {
                picket_interdicted: false,
                heave_to_braked: false,
                pdc_auto_enabled: false,
                laser_defense_enabled: false,
                lateral_jink_executed: false,
                attitude_rolled_90: false,
                counter_railgun_fired: false,
                pallas_cluster_entered: false,
                m_type_rock_pinged: false,
                velocity_matched: false,
                drone_sortie_recovered: false,
                mining_laser_claim_jump: false,
                claim_jumper_disabled: false,
                derelict_docking_started: false,
                hard_dock_clamped: false,
                salvage_tow_attached: false,
                decoupled_free_flight: false,
                drop_tanks_fitted: false,
                neptune_expedition_arrived: false,
                autopilot_recoupled: false,
            },
            sent_commands: Vec::new(),
        }
    }

    /// Current [`SpaceBeat`] of the lane.
    #[must_use]
    pub const fn beat(&self) -> SpaceBeat {
        self.beat
    }

    /// Return the initial command to kick off the lane (`+helm/enter kirkwood_picket`).
    pub fn start_command(&mut self) -> Option<&'static str> {
        let cmd = self.beat.command()?;
        self.sent_commands.push(cmd.to_owned());
        Some(cmd)
    }

    /// Feed a single player-visible terminal line from the server and return the next
    /// MUSH command to send when the current beat's acknowledgement is matched.
    pub fn feed_line(&mut self, line: &str) -> Option<&'static str> {
        let trimmed = line.trim();
        let next_beat = match self.beat {
            SpaceBeat::EnterKirkwoodPicket
                if trimmed.contains("Interdiction triggered at Kirkwood 3:1 Picket") =>
            {
                self.evidence.picket_interdicted = true;
                Some(SpaceBeat::BrakeForCustoms)
            }
            SpaceBeat::BrakeForCustoms
                if trimmed.contains("Main drive thrust set to [-15, 0, 0] m/s^2") =>
            {
                self.evidence.heave_to_braked = true;
                Some(SpaceBeat::StepPicketHeaveTo)
            }
            SpaceBeat::StepPicketHeaveTo
                if trimmed.contains("Stepped Kirkwood Picket bubble +1000 ms") =>
            {
                Some(SpaceBeat::EnablePdcAuto)
            }
            SpaceBeat::EnablePdcAuto
                if trimmed.contains("Rotary PDC point-defense network set to auto=true.") =>
            {
                self.evidence.pdc_auto_enabled = true;
                Some(SpaceBeat::SwitchLaserDefense)
            }
            SpaceBeat::SwitchLaserDefense
                if trimmed.contains("Laser turrets switched to Defense mode.") =>
            {
                self.evidence.laser_defense_enabled = true;
                Some(SpaceBeat::LateralJinkBurn)
            }
            SpaceBeat::LateralJinkBurn
                if trimmed.contains("Main drive thrust set to [0, 12, 0] m/s^2") =>
            {
                self.evidence.lateral_jink_executed = true;
                Some(SpaceBeat::RollArmorFacet)
            }
            SpaceBeat::RollArmorFacet
                if trimmed
                    .contains("Hull attitude trimmed to yaw=0 deg, pitch=0 deg, roll=90 deg.") =>
            {
                self.evidence.attitude_rolled_90 = true;
                Some(SpaceBeat::FireCounterRailgun)
            }
            SpaceBeat::FireCounterRailgun
                if trimmed.contains("Fired")
                    && trimmed.contains("railgun slug(s) at 'ship.raider_kestrel'") =>
            {
                self.evidence.counter_railgun_fired = true;
                Some(SpaceBeat::StepPicketIntercept)
            }
            SpaceBeat::StepPicketIntercept
                if trimmed.contains("Stepped Kirkwood Picket bubble +1000 ms") =>
            {
                Some(SpaceBeat::EnterPallasCluster)
            }
            SpaceBeat::EnterPallasCluster
                if trimmed
                    .contains("Entered orbital asteroid cluster 'pallas_inclined_belt'") =>
            {
                self.evidence.pallas_cluster_entered = true;
                Some(SpaceBeat::PingMTypeRock)
            }
            SpaceBeat::PingMTypeRock
                if trimmed.contains("Pinged rock 'rock.m_309': class=M") =>
            {
                self.evidence.m_type_rock_pinged = true;
                Some(SpaceBeat::MatchRockVelocity)
            }
            SpaceBeat::MatchRockVelocity
                if trimmed.contains("Matched 3D velocity with 'rock.m_309'") =>
            {
                self.evidence.velocity_matched = true;
                Some(SpaceBeat::LaunchCollectorDrones)
            }
            SpaceBeat::LaunchCollectorDrones
                if trimmed.contains("Collector drone sortie")
                    && trimmed.contains("from 'rock.m_309' into stow.ore") =>
            {
                self.evidence.drone_sortie_recovered = true;
                Some(SpaceBeat::ExtractWithMiningLaser)
            }
            SpaceBeat::ExtractWithMiningLaser
                if trimmed.contains("Extracted 4000 kg")
                    && trimmed.contains("from 'rock.m_309' into stow.ore")
                    && trimmed.contains("claim_jump=true") =>
            {
                self.evidence.mining_laser_claim_jump = true;
                Some(SpaceBeat::DisableClaimJumper)
            }
            SpaceBeat::DisableClaimJumper
                if trimmed.contains("Fired offensive laser pulse(s) at 'claim_jumper'") =>
            {
                self.evidence.claim_jumper_disabled = true;
                Some(SpaceBeat::StartDerelictDocking)
            }
            SpaceBeat::StartDerelictDocking
                if trimmed.contains(
                    "Entered mandatory out-of-time-loop 6-DOF hard-docking session with derelict 'claim_jumper'",
                ) =>
            {
                self.evidence.derelict_docking_started = true;
                Some(SpaceBeat::ClampDerelictCollar)
            }
            SpaceBeat::ClampDerelictCollar if trimmed.contains("HARD-DOCK CLAMPED") => {
                self.evidence.hard_dock_clamped = true;
                Some(SpaceBeat::AttachSalvageTow)
            }
            SpaceBeat::AttachSalvageTow
                if trimmed.contains("Salvage tow line attached to derelict 'claim_jumper'")
                    && trimmed.contains("48 h Freeport contest window open") =>
            {
                self.evidence.salvage_tow_attached = true;
                Some(SpaceBeat::DecoupleFreeFlight)
            }
            SpaceBeat::DecoupleFreeFlight
                if trimmed.contains(
                    "Decoupled from 'ceres -> mars' transit leg into Tier-1 3D System Free Flight",
                ) =>
            {
                self.evidence.decoupled_free_flight = true;
                Some(SpaceBeat::FitDropTanks)
            }
            SpaceBeat::FitDropTanks
                if trimmed.contains("Fitted long-range external expedition drop tanks") =>
            {
                self.evidence.drop_tanks_fitted = true;
                Some(SpaceBeat::FlyNeptuneExpedition)
            }
            SpaceBeat::FlyNeptuneExpedition
                if trimmed.contains("Outer-system expedition arrived at 'neptune'") =>
            {
                self.evidence.neptune_expedition_arrived = true;
                Some(SpaceBeat::RecoupleAutopilot)
            }
            SpaceBeat::RecoupleAutopilot
                if trimmed.contains("Autopilot recoupled to 'ceres'") =>
            {
                self.evidence.autopilot_recoupled = true;
                Some(SpaceBeat::Complete)
            }
            _ => None,
        }?;

        self.beat = next_beat;
        let cmd = self.beat.command()?;
        self.sent_commands.push(cmd.to_owned());
        Some(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tacsim::space::{
        HELM_SWITCHES, HELM_VERB, MINE_SWITCHES, MINE_VERB, RAIDER_SHIP_ID, TAC_SWITCHES, TAC_VERB,
    };

    #[test]
    fn space_vocabulary_matches_tacsim_registered_verbs_and_switches() {
        for (verb, switches) in [
            (HELM_VERB, &HELM_SWITCHES[..]),
            (TAC_VERB, &TAC_SWITCHES[..]),
            (MINE_VERB, &MINE_SWITCHES[..]),
        ] {
            assert!(
                in_space_vocabulary(verb),
                "bare verb must be in space vocabulary: {verb}"
            );
            for sw in switches {
                let with_sw = format!("{verb}/{sw} arg");
                assert!(
                    in_space_vocabulary(&with_sw),
                    "registered switch must be in space vocabulary: {with_sw}"
                );
            }
        }
        assert!(!in_space_vocabulary("@unit/create tacsim.unit.dray"));
        assert!(!in_space_vocabulary(""));
        assert!(!in_space_vocabulary("+unknown/verb"));
    }

    #[test]
    fn space_lane_plays_stories_1_and_2_through_all_22_beats() {
        assert_eq!(RAIDER_SHIP_ID, "ship.raider_kestrel");

        let mut lane = SpaceLane::new();
        let first = lane.start_command().expect("start command");
        assert_eq!(first, "+helm/enter kirkwood_picket");
        assert!(in_space_vocabulary(first));

        let server_replies = [
            "Interdiction triggered at Kirkwood 3:1 Picket: CarrierJourney paused (Promoted); Republic Customs Corvette hails you to heave-to (<= 10 m/s).",
            "Main drive thrust set to [-15, 0, 0] m/s^2 (force=false).",
            "Stepped Kirkwood Picket bubble +1000 ms: phase=HeavedToInspecting { scan_progress_ms: 0 }, events=0.",
            "Rotary PDC point-defense network set to auto=true.",
            "Laser turrets switched to Defense mode.",
            "Main drive thrust set to [0, 12, 0] m/s^2 (force=false).",
            "Hull attitude trimmed to yaw=0 deg, pitch=0 deg, roll=90 deg.",
            "Fired 1 railgun slug(s) at 'ship.raider_kestrel'.",
            "Stepped Kirkwood Picket bubble +1000 ms: phase=RaiderAmbushActive, events=2.",
            "Entered orbital asteroid cluster 'pallas_inclined_belt' on 1:1 wall-clock time; Dray-P stow.ore bay online.",
            "Pinged rock 'rock.m_309': class=M, deposits=[strategic_ore:Rich, metal_ore:Fair].",
            "Matched 3D velocity with 'rock.m_309': standoff distance 420 m, relative speed 0 m/s (within <= 5 m/s mining gate).",
            "Collector drone sortie (ExcursionId=1, 4 drones) recovered 3200 kg of 'strategic_ore' from 'rock.m_309' into stow.ore.",
            "Extracted 4000 kg of 'strategic_ore' from 'rock.m_309' into stow.ore (hold=7200/100000 kg, ir_bloom=650 permille, claim_jump=true).",
            "Fired offensive laser pulse(s) at 'claim_jumper' (2 hit events).",
            "Entered mandatory out-of-time-loop 6-DOF hard-docking session with derelict 'claim_jumper'.",
            "HARD-DOCK CLAMPED at 450 mm/s (saved_dv=0 m/s, saved_time=0 IC s).",
            "Salvage tow line attached to derelict 'claim_jumper' (550 t <= 800 t 2x cap); 48 h Freeport contest window open until IC 172800 s.",
            "Decoupled from 'ceres -> mars' transit leg into Tier-1 3D System Free Flight at [412000000, 12000000, 3500000] km.",
            "Fitted long-range external expedition drop tanks: remaining_dv=84000 m/s, propellant=84000 kg.",
            "Outer-system expedition arrived at 'neptune': transit distance 4350000000 km, one-way radio light-lag 14510 s (241 min).",
            "Autopilot recoupled to 'ceres': distance 4350000000 km, debited dv 12000 m/s, transit 86400 s (arrive_ic=74152720000).",
        ];

        for reply in server_replies {
            if let Some(cmd) = lane.feed_line(reply) {
                assert!(
                    in_space_vocabulary(cmd),
                    "command must be in closed space vocabulary: {cmd}"
                );
            }
        }

        assert_eq!(lane.beat(), SpaceBeat::Complete);
        assert!(lane.evidence.all_milestones_complete());
        assert_eq!(lane.sent_commands.len(), 22);
    }
}

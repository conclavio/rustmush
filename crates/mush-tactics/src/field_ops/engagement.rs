//! `H1-B` & `H1-C` — IC field combat entry, motion-clock handoff, reinforcement,
//! withdrawal, demotion settlement, carrier leave-behind confirmation, link-loss
//! posture, crew survival / guaranteed NPC extraction, and observer-bounded
//! post-battle records (design doc 27 §4.1, §4.4, §5, §6 `H1-B`/`H1-C`,
//! `FO-1`, `FO-2`, `FO-10`, `FO-12`, `Q-H1`–`Q-H12`, `Q-S9`, `Q-S10`, `Q-F4`,
//! `Q-R3`, `P-2`, `P-5`).

use std::collections::BTreeMap;
use std::fmt;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::field_ops::condition::{
    AuthorityHandoffRequest, ConditionError, FieldAuthorityKind, FieldCustodyLedger,
    FieldUnitAssessment, OperationalState, UnitAnatomyProfile, assess_tactical_unit,
    ic_condition_plan,
};
use crate::field_ops::recovery::{FieldPose, RecoveryOpId};
use crate::unit::{ComponentId, SectionId, TacticalUnit, UnitError, UnitMutationPlan, apply};

/// Unengaged frontier surface IC clock rate (`24` IC seconds per `1` wall-clock second, `Q-R3`).
pub const IC_SURFACE_CLOCK_RATE: u32 = 24;

/// Active tactical combat clock rate (`1` combat second per `1` wall-clock second, `Q-R3`).
pub const COMBAT_WALL_CLOCK_RATE: u32 = 1;

/// Default fallback safe port for guaranteed NPC extraction when no nearer known safe
/// port is eligible (`Q-F4`, Phobos / Stickney).
pub const DEFAULT_FALLBACK_SAFE_PORT: &str = "phobos";

/// Maximum units tracked in one [`FieldSiteEngagementBook`].
pub const MAX_SITE_UNITS: usize = 512;

/// Maximum observations stored per observer in [`FieldObservationBook`].
pub const MAX_OBSERVATIONS_PER_ACTOR: usize = 1_024;

/// Maximum crew survival records in [`CrewSurvivalBook`].
pub const MAX_CREW_RECORDS: usize = 1_024;

/// Trigger that promotes an unengaged frontier site into an active combat engagement (`Q-H1`, `Q-S9`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldPromotionTrigger {
    /// Explicit hostile weapon fire, lock-on, or attack command (`Q-H1`).
    ExplicitHostileAction {
        /// Initiating attacker unit.
        attacker: ObjId,
        /// Targeted unit.
        target: ObjId,
    },
    /// Uninvited/hostile crossing into an armed perimeter (`Q-H1`).
    ArmedPerimeterCrossing {
        /// Intruding unit.
        intruder: ObjId,
        /// Defending unit or perimeter owner.
        defender: ObjId,
    },
    /// Mutual engagement order between opposing commanders/units (`Q-H1`).
    MutualEngagementOrder {
        /// First engaging unit.
        initiator: ObjId,
        /// Opposing engaging unit.
        opponent: ObjId,
    },
    /// Hostile interdiction of an active field recovery/tow operation (`Q-S9`).
    RecoveryInterdiction {
        /// Challenging unit interdicting the recovery.
        challenger: ObjId,
        /// Towing unit being interdicted.
        tug: ObjId,
        /// Towed casualty unit.
        casualty: ObjId,
    },
}

impl FieldPromotionTrigger {
    fn validate(&self) -> Result<(), EngagementError> {
        match *self {
            Self::ExplicitHostileAction { attacker, target }
            | Self::ArmedPerimeterCrossing {
                intruder: attacker,
                defender: target,
            }
            | Self::MutualEngagementOrder {
                initiator: attacker,
                opponent: target,
            } => {
                if attacker.0 == 0 || target.0 == 0 {
                    return Err(EngagementError::InvalidObjectId("promotion_participant"));
                }
                if attacker == target {
                    return Err(EngagementError::SelfEngagement(attacker));
                }
            }
            Self::RecoveryInterdiction {
                challenger,
                tug,
                casualty,
            } => {
                if challenger.0 == 0 || tug.0 == 0 || casualty.0 == 0 {
                    return Err(EngagementError::InvalidObjectId("interdiction_participant"));
                }
                if challenger == tug || challenger == casualty || tug == casualty {
                    return Err(EngagementError::SelfEngagement(challenger));
                }
            }
        }
        Ok(())
    }
}

/// Snapshot of a unit's surface motion or tow progress at the moment a site promotes
/// from the `rate-24` IC surface clock to `1:1` wall-time combat, or demotes back (`Q-H2`, `Q-R3`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionClockHandoff {
    /// Unit object id.
    pub unit: ObjId,
    /// Exact field pose at the clock handoff boundary (no rewind or duplication).
    pub handoff_pose: FieldPose,
    /// Clock rate before handoff (`24` on promotion, `1` on demotion).
    pub from_clock_rate: u32,
    /// Clock rate after handoff (`1` on promotion, `24` on demotion).
    pub to_clock_rate: u32,
    /// Cumulative distance travelled or towed prior to the handoff (`cm`).
    pub preserved_distance_cm: u64,
    /// Active tow operation id if this unit was towing or being towed across the boundary.
    pub active_recovery_op: Option<RecoveryOpId>,
    /// IC timestamp of the handoff.
    pub handoff_at_ic: i64,
}

/// Settlement gate conditions that must clear before an active engagement can demote
/// back to persistent field surface state (`Q-H4`, `Q-S10`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DemotionSettlementGate {
    /// In-flight projectiles, artillery shells, or unresolved damage impacts.
    pub pending_impacts: u32,
    /// Opposing operational armed pairs still actively engaging within weapons/contact range.
    pub active_hostile_contacts: u32,
    /// Units mid-transition across a site boundary, reinforcement drop, or bay ramp.
    pub pending_boundary_handoffs: u32,
}

impl DemotionSettlementGate {
    /// Whether all settlement conditions are clear so the engagement may demote (`Q-H4`).
    #[must_use]
    pub const fn is_settled(&self) -> bool {
        self.pending_impacts == 0
            && self.active_hostile_contacts == 0
            && self.pending_boundary_handoffs == 0
    }
}

/// State of a unit deployed on a frontier site (`H1-B`, `H1-C`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteUnitPresence {
    /// Unit object id.
    pub unit: ObjId,
    /// Faction or force side tag (`"alpha"`, `"bravo"`, etc.).
    pub side: SmolStr,
    /// Current field pose on the site.
    pub pose: FieldPose,
    /// Latest derived operational assessment.
    pub assessment: FieldUnitAssessment,
    /// Cumulative surface/combat distance moved (`cm`) across clock handoffs (`Q-R3`).
    pub cumulative_distance_cm: u64,
    /// Active tow operation if coupled.
    pub active_recovery_op: Option<RecoveryOpId>,
    /// Optional carrier object id to whose sortie manifest this unit belongs (`Q-H10`).
    pub assigned_carrier: Option<ObjId>,
}

/// Withdrawal destination when a unit exits an active field engagement (`Q-H3`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WithdrawalDestination {
    /// Withdraw to an adjacent/perimeter surface pose outside the combat instance.
    PerimeterSurface {
        /// Destination surface pose.
        pose: FieldPose,
    },
    /// Withdraw directly into a carrier bay at the site.
    CarrierBay {
        /// Carrier object id.
        carrier: ObjId,
        /// Bay slot id.
        slot: SmolStr,
    },
    /// Withdraw directly into a base hangar.
    Hangar {
        /// Base object id.
        base: ObjId,
    },
}

/// Outcome of demoting a settled field engagement back to persistent surface state (`Q-H4`, `Q-S10`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DemotionOutcome {
    /// Site object id.
    pub site: ObjId,
    /// Closed combat instance id.
    pub closed_instance_id: u64,
    /// Units remaining on the field in operational condition (`Operational`, `Unarmed`).
    pub operational_surface_units: Vec<ObjId>,
    /// Persistent recoverable casualties remaining on the field (`Immobile`, `Disabled`, `Wreck`)
    /// whose hull epochs remain open (`Q-H9`, `Q-S10`).
    pub persistent_field_casualties: Vec<ObjId>,
    /// Catastrophic losses (`Irrecoverable`) eligible for `FieldCloseOutcome::Destroyed`.
    pub irrecoverable_wrecks: Vec<ObjId>,
    /// Motion-clock handoff records (`1:1` combat -> `rate-24` surface, `Q-R3`).
    pub clock_handoffs: Vec<MotionClockHandoff>,
    /// IC timestamp of demotion.
    pub demoted_at_ic: i64,
}

/// Outcome of a carrier departure check on a frontier site (`Q-H10`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CarrierDepartureOutcome {
    /// Departing carrier object id.
    pub carrier: ObjId,
    /// Site departed from.
    pub site: ObjId,
    /// Sortie units safely stowed aboard the carrier.
    pub stowed_units: Vec<ObjId>,
    /// Sortie units explicitly left behind on the field (`Q-H10`).
    /// These units remain persistent field objects with their legal title intact
    /// and their hull epochs `RemainOpen`.
    pub left_behind_units: Vec<ObjId>,
    /// IC timestamp of departure.
    pub departed_at_ic: i64,
}

/// Manages unengaged surface presence, promotion to active combat, reinforcement,
/// withdrawal, re-attacks on disabled units, and demotion settlement for a frontier site (`H1-B`, `H1-C`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldSiteEngagementBook {
    site: ObjId,
    active_instance_id: Option<u64>,
    last_trigger: Option<FieldPromotionTrigger>,
    settlement_gate: DemotionSettlementGate,
    units: BTreeMap<ObjId, SiteUnitPresence>,
    stowed_in_carrier: BTreeMap<ObjId, (ObjId, SmolStr)>,
    clock_handoffs: Vec<MotionClockHandoff>,
}

impl FieldSiteEngagementBook {
    /// Construct a new unengaged [`FieldSiteEngagementBook`] for `site`.
    ///
    /// # Errors
    /// Returns [`EngagementError::InvalidObjectId`] if `site.0 == 0`.
    pub const fn new(site: ObjId) -> Result<Self, EngagementError> {
        if site.0 == 0 {
            return Err(EngagementError::InvalidObjectId("site"));
        }
        Ok(Self {
            site,
            active_instance_id: None,
            last_trigger: None,
            settlement_gate: DemotionSettlementGate {
                pending_impacts: 0,
                active_hostile_contacts: 0,
                pending_boundary_handoffs: 0,
            },
            units: BTreeMap::new(),
            stowed_in_carrier: BTreeMap::new(),
            clock_handoffs: Vec::new(),
        })
    }

    /// Site object id.
    #[must_use]
    pub const fn site(&self) -> ObjId {
        self.site
    }

    /// Active combat instance id, if the site is currently engaged.
    #[must_use]
    pub const fn active_instance_id(&self) -> Option<u64> {
        self.active_instance_id
    }

    /// Whether the site is currently in an active combat engagement.
    #[must_use]
    pub const fn is_engaged(&self) -> bool {
        self.active_instance_id.is_some()
    }

    /// Active clock rate for units on the site (`1` when engaged, `24` when unengaged, `Q-R3`).
    #[must_use]
    pub const fn current_clock_rate(&self) -> u32 {
        if self.active_instance_id.is_some() {
            COMBAT_WALL_CLOCK_RATE
        } else {
            IC_SURFACE_CLOCK_RATE
        }
    }

    /// Current demotion settlement gate state.
    #[must_use]
    pub const fn settlement_gate(&self) -> &DemotionSettlementGate {
        &self.settlement_gate
    }

    /// Update the demotion settlement gate counters (`Q-H4`).
    pub fn set_settlement_gate(&mut self, gate: DemotionSettlementGate) {
        self.settlement_gate = gate;
    }

    /// Inspect a unit currently on the site surface/engagement.
    #[must_use]
    pub fn unit_presence(&self, unit: ObjId) -> Option<&SiteUnitPresence> {
        self.units.get(&unit)
    }

    /// All units currently on the site surface/engagement.
    #[must_use]
    pub const fn units(&self) -> &BTreeMap<ObjId, SiteUnitPresence> {
        &self.units
    }

    /// Recorded motion-clock handoffs across promotions and demotions (`Q-R3`).
    #[must_use]
    pub fn clock_handoffs(&self) -> &[MotionClockHandoff] {
        &self.clock_handoffs
    }

    /// Deploy or register a unit onto the unengaged site surface and record its
    /// [`FieldAuthorityKind::FieldSurface`] custody in `ledger`.
    ///
    /// # Errors
    /// Returns [`EngagementError`] if the site is already engaged (use [`Self::reinforce_engagement`]
    /// instead), the pose is on a different site, or custody handoff fails.
    #[allow(clippy::too_many_arguments)]
    pub fn deploy_surface_unit(
        &mut self,
        ledger: &mut FieldCustodyLedger,
        unit: &TacticalUnit,
        anatomy: &UnitAnatomyProfile,
        side: SmolStr,
        pose: FieldPose,
        assigned_carrier: Option<ObjId>,
        at_ic: i64,
    ) -> Result<SiteUnitPresence, EngagementError> {
        if self.is_engaged() {
            return Err(EngagementError::SiteAlreadyEngaged {
                site: self.site,
                instance_id: self.active_instance_id.unwrap_or(0),
            });
        }
        if pose.site != self.site {
            return Err(EngagementError::SiteMismatch {
                expected: self.site,
                actual: pose.site,
            });
        }
        if side.trim().is_empty() {
            return Err(EngagementError::EmptySideTag);
        }
        if !self.units.contains_key(&unit.object()) && self.units.len() >= MAX_SITE_UNITS {
            return Err(EngagementError::MaxSiteUnitsReached {
                max: MAX_SITE_UNITS,
            });
        }

        let assessment = assess_tactical_unit(unit, anatomy);
        let surface_auth = FieldAuthorityKind::FieldSurface {
            site: pose.site,
            x_cm: pose.x_cm,
            y_cm: pose.y_cm,
        };
        if let Some(rec) = ledger.get(unit.object()).cloned() {
            ledger
                .handoff(AuthorityHandoffRequest {
                    unit: unit.object(),
                    expected_generation: rec.authority_generation(),
                    next_authority: surface_auth,
                    operational_state: assessment.state,
                    unit_revision: assessment.unit_revision,
                    sortie_id: rec.sortie_id(),
                    cargo_mass_kg: rec.cargo_mass_kg(),
                    cargo_volume_l: rec.cargo_volume_l(),
                    at_ic,
                })
                .map_err(EngagementError::Condition)?;
        } else {
            ledger
                .register_unit(unit, surface_auth, assessment.state, None, at_ic)
                .map_err(EngagementError::Condition)?;
        }

        self.stowed_in_carrier.remove(&unit.object());
        let presence = SiteUnitPresence {
            unit: unit.object(),
            side,
            pose,
            assessment,
            cumulative_distance_cm: 0,
            active_recovery_op: None,
            assigned_carrier,
        };
        self.units.insert(unit.object(), presence.clone());
        Ok(presence)
    }

    /// Advance a unit's surface position (or coupled tow position) on the site,
    /// updating its pose and cumulative distance (`Q-R3`).
    ///
    /// # Errors
    /// Returns [`EngagementError`] if the unit is unknown, immobile without an active
    /// tow, or `next_pose` is on a different site.
    pub fn record_surface_motion(
        &mut self,
        unit: ObjId,
        next_pose: FieldPose,
        active_recovery_op: Option<RecoveryOpId>,
    ) -> Result<u64, EngagementError> {
        if next_pose.site != self.site {
            return Err(EngagementError::SiteMismatch {
                expected: self.site,
                actual: next_pose.site,
            });
        }
        let presence = self
            .units
            .get_mut(&unit)
            .ok_or(EngagementError::UnknownSiteUnit(unit))?;
        if !presence.assessment.can_move && active_recovery_op.is_none() {
            return Err(EngagementError::UnitCannotMoveUnderOwnPower {
                unit,
                state: presence.assessment.state,
            });
        }
        let step_cm = presence.pose.distance_cm_to(&next_pose).unwrap_or(0);
        presence.pose = next_pose;
        presence.cumulative_distance_cm = presence.cumulative_distance_cm.saturating_add(step_cm);
        presence.active_recovery_op = active_recovery_op;
        Ok(presence.cumulative_distance_cm)
    }

    /// Promote the site from unengaged IC surface mode (`rate 24`) to an active combat
    /// engagement (`1:1` wall time) on `trigger` (`Q-H1`, `Q-H2`, `Q-R3`, `Q-S9`).
    ///
    /// Converts every present unit's authority to [`FieldAuthorityKind::CombatInstance`]
    /// at its exact current [`FieldPose`] without rewinding or duplicating distance.
    ///
    /// # Errors
    /// Returns [`EngagementError`] if `instance_id == 0`, the site is already engaged,
    /// `trigger` is invalid, or custody handoff fails.
    pub fn promote_to_engagement(
        &mut self,
        ledger: &mut FieldCustodyLedger,
        instance_id: u64,
        trigger: FieldPromotionTrigger,
        initial_hostile_contacts: u32,
        at_ic: i64,
    ) -> Result<Vec<MotionClockHandoff>, EngagementError> {
        if instance_id == 0 {
            return Err(EngagementError::ZeroInstanceId);
        }
        if let Some(existing) = self.active_instance_id {
            return Err(EngagementError::SiteAlreadyEngaged {
                site: self.site,
                instance_id: existing,
            });
        }
        trigger.validate()?;

        let mut handoffs = Vec::with_capacity(self.units.len());
        for presence in self.units.values() {
            let rec = ledger
                .get(presence.unit)
                .ok_or(EngagementError::UnknownSiteUnit(presence.unit))?;
            let expected_gen = rec.authority_generation();
            let sortie_id = rec.sortie_id();
            let cargo_mass_kg = rec.cargo_mass_kg();
            let cargo_volume_l = rec.cargo_volume_l();

            ledger
                .handoff(AuthorityHandoffRequest {
                    unit: presence.unit,
                    expected_generation: expected_gen,
                    next_authority: FieldAuthorityKind::CombatInstance {
                        site: self.site,
                        instance_id,
                    },
                    operational_state: presence.assessment.state,
                    unit_revision: presence.assessment.unit_revision,
                    sortie_id,
                    cargo_mass_kg,
                    cargo_volume_l,
                    at_ic,
                })
                .map_err(EngagementError::Condition)?;

            handoffs.push(MotionClockHandoff {
                unit: presence.unit,
                handoff_pose: presence.pose,
                from_clock_rate: IC_SURFACE_CLOCK_RATE,
                to_clock_rate: COMBAT_WALL_CLOCK_RATE,
                preserved_distance_cm: presence.cumulative_distance_cm,
                active_recovery_op: presence.active_recovery_op,
                handoff_at_ic: at_ic,
            });
        }

        self.active_instance_id = Some(instance_id);
        self.last_trigger = Some(trigger);
        self.settlement_gate = DemotionSettlementGate {
            pending_impacts: 0,
            active_hostile_contacts: initial_hostile_contacts,
            pending_boundary_handoffs: 0,
        };
        self.clock_handoffs.extend(handoffs.iter().cloned());
        Ok(handoffs)
    }

    /// Admit a reinforcing unit into an active field engagement at `entry_pose` (`Q-H3`).
    ///
    /// # Errors
    /// Returns [`EngagementError`] if the site is not engaged, the unit is already in
    /// the engagement, `entry_pose` is on another site, or custody handoff fails.
    #[allow(clippy::too_many_arguments)]
    pub fn reinforce_engagement(
        &mut self,
        ledger: &mut FieldCustodyLedger,
        unit: &TacticalUnit,
        anatomy: &UnitAnatomyProfile,
        side: SmolStr,
        entry_pose: FieldPose,
        assigned_carrier: Option<ObjId>,
        at_ic: i64,
    ) -> Result<SiteUnitPresence, EngagementError> {
        let instance_id = self
            .active_instance_id
            .ok_or(EngagementError::SiteNotEngaged { site: self.site })?;
        if entry_pose.site != self.site {
            return Err(EngagementError::SiteMismatch {
                expected: self.site,
                actual: entry_pose.site,
            });
        }
        if side.trim().is_empty() {
            return Err(EngagementError::EmptySideTag);
        }
        if self.units.contains_key(&unit.object()) {
            return Err(EngagementError::UnitAlreadyPresent(unit.object()));
        }
        if self.units.len() >= MAX_SITE_UNITS {
            return Err(EngagementError::MaxSiteUnitsReached {
                max: MAX_SITE_UNITS,
            });
        }

        let assessment = assess_tactical_unit(unit, anatomy);
        if !assessment.can_move {
            return Err(EngagementError::UnitCannotMoveUnderOwnPower {
                unit: unit.object(),
                state: assessment.state,
            });
        }

        let combat_auth = FieldAuthorityKind::CombatInstance {
            site: self.site,
            instance_id,
        };
        if let Some(rec) = ledger.get(unit.object()).cloned() {
            ledger
                .handoff(AuthorityHandoffRequest {
                    unit: unit.object(),
                    expected_generation: rec.authority_generation(),
                    next_authority: combat_auth,
                    operational_state: assessment.state,
                    unit_revision: assessment.unit_revision,
                    sortie_id: rec.sortie_id(),
                    cargo_mass_kg: rec.cargo_mass_kg(),
                    cargo_volume_l: rec.cargo_volume_l(),
                    at_ic,
                })
                .map_err(EngagementError::Condition)?;
        } else {
            ledger
                .register_unit(unit, combat_auth, assessment.state, None, at_ic)
                .map_err(EngagementError::Condition)?;
        }

        self.stowed_in_carrier.remove(&unit.object());
        let presence = SiteUnitPresence {
            unit: unit.object(),
            side,
            pose: entry_pose,
            assessment,
            cumulative_distance_cm: 0,
            active_recovery_op: None,
            assigned_carrier,
        };
        self.units.insert(unit.object(), presence.clone());
        Ok(presence)
    }

    /// Withdraw a unit (under its own power or towed) from an active engagement or site
    /// surface to `destination` (`Q-H3`).
    ///
    /// # Errors
    /// Returns [`EngagementError`] if the unit is unknown, cannot move (and is not being
    /// towed out), or custody handoff fails.
    pub fn withdraw_unit(
        &mut self,
        ledger: &mut FieldCustodyLedger,
        unit: ObjId,
        towed_by_mobile_tug: bool,
        destination: WithdrawalDestination,
        at_ic: i64,
    ) -> Result<FieldAuthorityKind, EngagementError> {
        let presence = self
            .units
            .get(&unit)
            .ok_or(EngagementError::UnknownSiteUnit(unit))?
            .clone();

        if !presence.assessment.can_move && !towed_by_mobile_tug {
            return Err(EngagementError::UnitCannotMoveUnderOwnPower {
                unit,
                state: presence.assessment.state,
            });
        }

        let next_authority = match &destination {
            WithdrawalDestination::PerimeterSurface { pose } => FieldAuthorityKind::FieldSurface {
                site: pose.site,
                x_cm: pose.x_cm,
                y_cm: pose.y_cm,
            },
            WithdrawalDestination::CarrierBay { carrier, slot } => FieldAuthorityKind::CarrierBay {
                carrier: *carrier,
                slot: slot.clone(),
            },
            WithdrawalDestination::Hangar { base } => FieldAuthorityKind::Hangar { base: *base },
        };

        let rec = ledger
            .get(unit)
            .ok_or(EngagementError::UnknownSiteUnit(unit))?;
        let expected_gen = rec.authority_generation();
        let sortie_id = rec.sortie_id();
        let cargo_mass_kg = rec.cargo_mass_kg();
        let cargo_volume_l = rec.cargo_volume_l();

        ledger
            .handoff(AuthorityHandoffRequest {
                unit,
                expected_generation: expected_gen,
                next_authority: next_authority.clone(),
                operational_state: presence.assessment.state,
                unit_revision: presence.assessment.unit_revision,
                sortie_id,
                cargo_mass_kg,
                cargo_volume_l,
                at_ic,
            })
            .map_err(EngagementError::Condition)?;

        match destination {
            WithdrawalDestination::PerimeterSurface { pose } => {
                if pose.site == self.site && !self.is_engaged() {
                    if let Some(p) = self.units.get_mut(&unit) {
                        p.pose = pose;
                    }
                } else {
                    self.units.remove(&unit);
                }
            }
            WithdrawalDestination::CarrierBay { carrier, slot } => {
                self.units.remove(&unit);
                self.stowed_in_carrier.insert(unit, (carrier, slot));
            }
            WithdrawalDestination::Hangar { .. } => {
                self.units.remove(&unit);
            }
        }

        Ok(next_authority)
    }

    /// Apply causal combat or post-disablement damage to a unit on the site (`Q-H7`, `Q-H8`).
    ///
    /// Disabled and wrecked units remain physical targets on the field (`Q-H8`); further
    /// hits can ruin surviving components or push a `Wreck` into `Irrecoverable`.
    ///
    /// # Errors
    /// Returns [`EngagementError`] if the unit is not on the site, the mutation plan fails,
    /// or IC condition handoff fails.
    pub fn apply_field_damage(
        &mut self,
        ledger: &mut FieldCustodyLedger,
        unit_before: &TacticalUnit,
        damage_plan: &UnitMutationPlan,
        anatomy: &UnitAnatomyProfile,
        at_ic: i64,
    ) -> Result<(TacticalUnit, FieldUnitAssessment), EngagementError> {
        let unit_id = unit_before.object();
        if !self.units.contains_key(&unit_id) {
            return Err(EngagementError::UnknownSiteUnit(unit_id));
        }

        let (raw_damaged, _) =
            apply(unit_before, damage_plan, at_ic).map_err(EngagementError::Unit)?;
        let ic_plan_opt = ic_condition_plan(
            unit_before,
            &raw_damaged,
            Some(anatomy),
            Some(SmolStr::new("field_ops.combat_damage")),
        )
        .map_err(EngagementError::Condition)?;

        let updated_unit = match ic_plan_opt {
            Some(plan) => {
                apply(unit_before, &plan, at_ic)
                    .map_err(EngagementError::Unit)?
                    .0
            }
            None => unit_before.clone(),
        };
        let assessment = assess_tactical_unit(&updated_unit, anatomy);

        let rec = ledger
            .get(unit_id)
            .ok_or(EngagementError::UnknownSiteUnit(unit_id))?;
        let expected_gen = rec.authority_generation();
        let current_authority = rec.authority().clone();
        let sortie_id = rec.sortie_id();
        let cargo_mass_kg = rec.cargo_mass_kg();
        let cargo_volume_l = rec.cargo_volume_l();

        ledger
            .handoff(AuthorityHandoffRequest {
                unit: unit_id,
                expected_generation: expected_gen,
                next_authority: current_authority,
                operational_state: assessment.state,
                unit_revision: assessment.unit_revision,
                sortie_id,
                cargo_mass_kg,
                cargo_volume_l,
                at_ic,
            })
            .map_err(EngagementError::Condition)?;

        if let Some(presence) = self.units.get_mut(&unit_id) {
            presence.assessment = assessment.clone();
        }

        Ok((updated_unit, assessment))
    }

    /// Demote an active field engagement back to persistent surface state once the
    /// [`DemotionSettlementGate`] has settled (`Q-H4`, `Q-R3`, `Q-S10`).
    ///
    /// Recoverable casualties (`Immobile`, `Unarmed`, `Disabled`, `Wreck`) remain on the
    /// site surface with their hull epochs open (`FieldEpochDisposition::RemainOpen`).
    ///
    /// # Errors
    /// Returns [`EngagementError`] if the site is not engaged, the settlement gate is
    /// still blocked, or custody handoff fails.
    pub fn demote_engagement(
        &mut self,
        ledger: &mut FieldCustodyLedger,
        at_ic: i64,
    ) -> Result<DemotionOutcome, EngagementError> {
        let instance_id = self
            .active_instance_id
            .ok_or(EngagementError::SiteNotEngaged { site: self.site })?;
        if !self.settlement_gate.is_settled() {
            return Err(EngagementError::DemotionBlocked(self.settlement_gate));
        }

        let mut operational_surface_units = Vec::new();
        let mut persistent_field_casualties = Vec::new();
        let mut irrecoverable_wrecks = Vec::new();
        let mut clock_handoffs = Vec::with_capacity(self.units.len());

        for presence in self.units.values() {
            let rec = ledger
                .get(presence.unit)
                .ok_or(EngagementError::UnknownSiteUnit(presence.unit))?;
            let expected_gen = rec.authority_generation();
            let sortie_id = rec.sortie_id();
            let cargo_mass_kg = rec.cargo_mass_kg();
            let cargo_volume_l = rec.cargo_volume_l();

            ledger
                .handoff(AuthorityHandoffRequest {
                    unit: presence.unit,
                    expected_generation: expected_gen,
                    next_authority: FieldAuthorityKind::FieldSurface {
                        site: presence.pose.site,
                        x_cm: presence.pose.x_cm,
                        y_cm: presence.pose.y_cm,
                    },
                    operational_state: presence.assessment.state,
                    unit_revision: presence.assessment.unit_revision,
                    sortie_id,
                    cargo_mass_kg,
                    cargo_volume_l,
                    at_ic,
                })
                .map_err(EngagementError::Condition)?;

            match presence.assessment.state {
                OperationalState::Operational | OperationalState::Unarmed => {
                    operational_surface_units.push(presence.unit);
                }
                OperationalState::Immobile
                | OperationalState::Disabled
                | OperationalState::Wreck => {
                    persistent_field_casualties.push(presence.unit);
                }
                OperationalState::Irrecoverable => {
                    irrecoverable_wrecks.push(presence.unit);
                }
            }

            clock_handoffs.push(MotionClockHandoff {
                unit: presence.unit,
                handoff_pose: presence.pose,
                from_clock_rate: COMBAT_WALL_CLOCK_RATE,
                to_clock_rate: IC_SURFACE_CLOCK_RATE,
                preserved_distance_cm: presence.cumulative_distance_cm,
                active_recovery_op: presence.active_recovery_op,
                handoff_at_ic: at_ic,
            });
        }

        self.active_instance_id = None;
        self.clock_handoffs.extend(clock_handoffs.iter().cloned());

        Ok(DemotionOutcome {
            site: self.site,
            closed_instance_id: instance_id,
            operational_surface_units,
            persistent_field_casualties,
            irrecoverable_wrecks,
            clock_handoffs,
            demoted_at_ic: at_ic,
        })
    }

    /// Validate and execute a carrier departure from the site (`Q-H10`).
    ///
    /// If any units assigned to `carrier` remain outside the carrier bay on the site
    /// (whether operational, immobile, disabled, or wrecked) and `confirm_leave_behind`
    /// is `false`, returns [`EngagementError::LeaveBehindConfirmationRequired`].
    /// When `confirm_leave_behind` is `true`, the left-behind units remain in the field
    /// with their legal title and open hull epochs untouched (`Q-H10`).
    ///
    /// # Errors
    /// Returns [`EngagementError`] if `carrier.0 == 0` or leave-behind confirmation is
    /// required but not provided.
    pub fn depart_carrier(
        &mut self,
        carrier: ObjId,
        confirm_leave_behind: bool,
        at_ic: i64,
    ) -> Result<CarrierDepartureOutcome, EngagementError> {
        if carrier.0 == 0 {
            return Err(EngagementError::InvalidObjectId("carrier"));
        }

        let left_behind_units: Vec<ObjId> = self
            .units
            .values()
            .filter(|p| p.assigned_carrier == Some(carrier))
            .map(|p| p.unit)
            .collect();

        if !left_behind_units.is_empty() && !confirm_leave_behind {
            return Err(EngagementError::LeaveBehindConfirmationRequired {
                carrier,
                unaccounted_units: left_behind_units,
            });
        }

        let stowed_units: Vec<ObjId> = self
            .stowed_in_carrier
            .iter()
            .filter(|(_, (c, _))| *c == carrier)
            .map(|(u, _)| *u)
            .collect();

        for u in &stowed_units {
            self.stowed_in_carrier.remove(u);
        }

        Ok(CarrierDepartureOutcome {
            carrier,
            site: self.site,
            stowed_units,
            left_behind_units,
            departed_at_ic: at_ic,
        })
    }
}

/// Controller posture of a field unit during pilot link loss (`Q-H5`, `P-2`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "posture", rename_all = "snake_case", deny_unknown_fields)]
pub enum LinkLossPosture {
    /// Pilot connection is active or still within `linkdead_grace_ms + reclaim_min_ms`.
    PlayerControlled,
    /// Grace and handover delay expired; sealed autonomous mind takes control (`Q-H5`).
    SealedMindFallback {
        /// Identifier of the sealed mind on the hull.
        mind_id: SmolStr,
    },
    /// Grace and handover delay expired on a hull with no sealed mind (`P-2`):
    /// holds last safe posture (zero velocity, no weapon fire, no new motion) while
    /// remaining physically vulnerable on the field.
    SafeHoldPosture,
}

/// Resolve the deterministic controller posture of a field unit during pilot disconnect (`Q-H5`, `P-2`).
#[must_use]
pub fn resolve_link_loss_posture(
    disconnected_elapsed_ms: u64,
    linkdead_grace_ms: u64,
    reclaim_min_ms: u64,
    sealed_mind: Option<&str>,
) -> LinkLossPosture {
    let handover_threshold_ms = linkdead_grace_ms.saturating_add(reclaim_min_ms);
    if disconnected_elapsed_ms < handover_threshold_ms {
        return LinkLossPosture::PlayerControlled;
    }
    if let Some(mind) = sealed_mind.map(str::trim).filter(|s| !s.is_empty()) {
        LinkLossPosture::SealedMindFallback {
            mind_id: SmolStr::new(mind),
        }
    } else {
        LinkLossPosture::SafeHoldPosture
    }
}

/// Survival and extraction status of a crew member / pilot (`FO-12`, `Q-H12`, `Q-F4`, `P-5`).
///
/// By `FO-12`, there is **zero player permadeath**: cockpit/head destruction or hull
/// loss transitions the occupant to `InjuredAboard` or `EjectedInField`, never death.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum OccupantSurvivalState {
    /// Seated aboard an operational or partially damaged unit.
    AboardUnit {
        /// Host unit object id.
        unit: ObjId,
        /// Whether the occupant sustained non-lethal combat injury (`Q-H12`).
        injured: bool,
    },
    /// Ejected or escaped from a disabled/wrecked/irrecoverable hull at a field pose (`Q-H12`).
    StrandedInField {
        /// Hull the occupant ejected/escaped from.
        from_unit: ObjId,
        /// Survivor's current field pose.
        pose: FieldPose,
        /// Whether the survivor is injured/incapacitated.
        injured: bool,
        /// IC timestamp when the occupant became stranded.
        stranded_at_ic: i64,
    },
    /// Friendly field rescue is physically under way within a bounded window (`Q-F4`, `P-5`).
    FriendlyRescueUnderway {
        /// Hull the occupant came from.
        from_unit: ObjId,
        /// Rescuing friendly unit object id.
        rescuer_unit: ObjId,
        /// Survivor's field pose.
        pose: FieldPose,
        /// Whether the survivor is injured.
        injured: bool,
        /// IC timestamp when friendly rescue began.
        started_at_ic: i64,
        /// IC timestamp when the published friendly rescue bound expires.
        window_expires_at_ic: i64,
    },
    /// Player-requested guaranteed NPC extraction is in flight (`Q-F4`, `P-5`).
    /// Leaves the unit, equipment, and cargo in the field with its hull epoch open.
    NpcExtractionInFlight {
        /// Abandoned hull left in the field.
        left_unit_in_field: ObjId,
        /// Destination safe port id (nearest eligible known safe port or fallback `"phobos"`).
        destination_port: SmolStr,
        /// IC timestamp when extraction was requested.
        requested_at_ic: i64,
        /// IC timestamp when extraction completes (extended by any world outage freeze).
        completes_at_ic: i64,
        /// Real-time UTC second when a world outage freeze paused this extraction, if currently frozen.
        paused_at_real_s: Option<i64>,
    },
    /// Delivered to a safe port (`Q-F4`) or rescued aboard a friendly carrier.
    RecoveredAtPort {
        /// Safe port id where the character arrived.
        port: SmolStr,
        /// Hull left behind in the field (whose epoch remains open until hull recovery).
        left_unit_in_field: ObjId,
        /// IC timestamp of arrival.
        arrived_at_ic: i64,
    },
    /// Captured alive in the field (`Q-H12`).
    Captured {
        /// Capturing unit or faction object id.
        captor: ObjId,
        /// Hull the occupant came from.
        from_unit: ObjId,
        /// IC timestamp of capture.
        captured_at_ic: i64,
    },
}

/// Candidate safe port for guaranteed NPC extraction (`Q-F4`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafePortCandidate {
    /// Port venue id (e.g. `"ceres"`, `"stickney"`, `"phobos"`).
    pub port_id: SmolStr,
    /// Whether the stranded character knows/is eligible to enter this port.
    pub eligible_and_known: bool,
    /// Campaign transit delay in IC seconds from the frontier site.
    pub transit_duration_ic: i64,
}

/// Select the nearest eligible known safe port, or fall back to [`DEFAULT_FALLBACK_SAFE_PORT`] (`Q-F4`).
#[must_use]
pub fn select_extraction_port(
    candidates: &[SafePortCandidate],
    fallback_duration_ic: i64,
) -> (SmolStr, i64) {
    let best = candidates
        .iter()
        .filter(|c| c.eligible_and_known && !c.port_id.trim().is_empty())
        .min_by_key(|c| c.transit_duration_ic.max(1));

    match best {
        Some(candidate) => (
            candidate.port_id.clone(),
            candidate.transit_duration_ic.max(1),
        ),
        None => (
            SmolStr::new(DEFAULT_FALLBACK_SAFE_PORT),
            fallback_duration_ic.max(1),
        ),
    }
}

/// Ledger tracking crew survival, friendly rescue bounds, and player-requested NPC extraction
/// with zero permadeath (`FO-12`, `Q-H12`, `Q-F4`, `P-5`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrewSurvivalBook {
    occupants: BTreeMap<ObjId, OccupantSurvivalState>,
}

impl CrewSurvivalBook {
    /// Construct an empty [`CrewSurvivalBook`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up an occupant's current survival state.
    #[must_use]
    pub fn state(&self, occupant: ObjId) -> Option<&OccupantSurvivalState> {
        self.occupants.get(&occupant)
    }

    /// Register an occupant aboard a deployed unit.
    ///
    /// # Errors
    /// Returns [`EngagementError`] if IDs are zero or capacity is exceeded.
    pub fn board_unit(
        &mut self,
        occupant: ObjId,
        unit: ObjId,
    ) -> Result<&OccupantSurvivalState, EngagementError> {
        if occupant.0 == 0 {
            return Err(EngagementError::InvalidObjectId("occupant"));
        }
        if unit.0 == 0 {
            return Err(EngagementError::InvalidObjectId("unit"));
        }
        if !self.occupants.contains_key(&occupant) && self.occupants.len() >= MAX_CREW_RECORDS {
            return Err(EngagementError::MaxCrewRecordsReached {
                max: MAX_CREW_RECORDS,
            });
        }
        self.occupants.insert(
            occupant,
            OccupantSurvivalState::AboardUnit {
                unit,
                injured: false,
            },
        );
        self.occupants
            .get(&occupant)
            .ok_or(EngagementError::UnknownOccupant(occupant))
    }

    /// Resolve occupant survival when their unit suffers a control-section kill, disabled
    /// state, wreck, or catastrophic loss (`FO-12`, `Q-H12`).
    ///
    /// Never kills the player character: ejects/strands them at `pose` on `Wreck` or
    /// `Irrecoverable` (or when `ejected == true`), or marks them `AboardUnit { injured: true }`
    /// if remaining with a `Disabled` hull (`P-5`).
    ///
    /// # Errors
    /// Returns [`EngagementError`] if IDs are invalid.
    #[allow(clippy::too_many_arguments)]
    pub fn record_unit_defeat_survival(
        &mut self,
        occupant: ObjId,
        unit: ObjId,
        unit_state: OperationalState,
        pose: FieldPose,
        ejected: bool,
        injured: bool,
        at_ic: i64,
    ) -> Result<&OccupantSurvivalState, EngagementError> {
        if occupant.0 == 0 {
            return Err(EngagementError::InvalidObjectId("occupant"));
        }
        if unit.0 == 0 {
            return Err(EngagementError::InvalidObjectId("unit"));
        }
        if !self.occupants.contains_key(&occupant) && self.occupants.len() >= MAX_CREW_RECORDS {
            return Err(EngagementError::MaxCrewRecordsReached {
                max: MAX_CREW_RECORDS,
            });
        }

        let must_exit = ejected
            || matches!(
                unit_state,
                OperationalState::Wreck | OperationalState::Irrecoverable
            );
        let next = if must_exit {
            OccupantSurvivalState::StrandedInField {
                from_unit: unit,
                pose,
                injured,
                stranded_at_ic: at_ic,
            }
        } else {
            OccupantSurvivalState::AboardUnit { unit, injured }
        };
        self.occupants.insert(occupant, next);
        self.occupants
            .get(&occupant)
            .ok_or(EngagementError::UnknownOccupant(occupant))
    }

    /// Begin a physical friendly rescue of a stranded occupant within a bounded IC window (`Q-F4`, `P-5`).
    ///
    /// While this rescue is active and within `window_duration_ic`, NPC extraction is
    /// refused so an in-progress player rescue is not pre-empted.
    ///
    /// # Errors
    /// Returns [`EngagementError`] if the occupant is not stranded/aboard a casualty or
    /// `window_duration_ic <= 0`.
    pub fn begin_friendly_rescue(
        &mut self,
        occupant: ObjId,
        rescuer_unit: ObjId,
        pose: FieldPose,
        window_duration_ic: i64,
        at_ic: i64,
    ) -> Result<&OccupantSurvivalState, EngagementError> {
        if rescuer_unit.0 == 0 {
            return Err(EngagementError::InvalidObjectId("rescuer_unit"));
        }
        if window_duration_ic <= 0 {
            return Err(EngagementError::InvalidDuration(window_duration_ic));
        }
        let current = self
            .occupants
            .get(&occupant)
            .ok_or(EngagementError::UnknownOccupant(occupant))?;

        let (from_unit, injured) = match current {
            OccupantSurvivalState::StrandedInField {
                from_unit, injured, ..
            }
            | OccupantSurvivalState::AboardUnit {
                unit: from_unit,
                injured,
            } => (*from_unit, *injured),
            _ => return Err(EngagementError::OccupantNotStranded(occupant)),
        };

        let expires = at_ic.saturating_add(window_duration_ic);
        self.occupants.insert(
            occupant,
            OccupantSurvivalState::FriendlyRescueUnderway {
                from_unit,
                rescuer_unit,
                pose,
                injured,
                started_at_ic: at_ic,
                window_expires_at_ic: expires,
            },
        );
        self.occupants
            .get(&occupant)
            .ok_or(EngagementError::UnknownOccupant(occupant))
    }

    /// Request guaranteed NPC extraction for a stranded occupant on explicit player command (`Q-F4`, `P-5`).
    ///
    /// - Never runs automatically without the stranded character's request (`P-5`).
    /// - Refuses with [`EngagementError::FriendlyRescueActive`] **only** if a friendly rescue
    ///   is currently in progress and `at_ic < window_expires_at_ic`.
    /// - Leaves the hull, equipment, and cargo in the field with its hull epoch open (`Q-F4`).
    ///
    /// # Errors
    /// Returns [`EngagementError`] if an unexpired friendly rescue is active or the occupant
    /// is not eligible for extraction.
    pub fn request_npc_extraction(
        &mut self,
        occupant: ObjId,
        candidates: &[SafePortCandidate],
        fallback_duration_ic: i64,
        at_ic: i64,
    ) -> Result<&OccupantSurvivalState, EngagementError> {
        let current = self
            .occupants
            .get(&occupant)
            .ok_or(EngagementError::UnknownOccupant(occupant))?;

        let from_unit = match current {
            OccupantSurvivalState::StrandedInField { from_unit, .. }
            | OccupantSurvivalState::AboardUnit {
                unit: from_unit, ..
            } => *from_unit,
            OccupantSurvivalState::FriendlyRescueUnderway {
                from_unit,
                rescuer_unit,
                window_expires_at_ic,
                ..
            } => {
                if at_ic < *window_expires_at_ic {
                    return Err(EngagementError::FriendlyRescueActive {
                        occupant,
                        rescuer_unit: *rescuer_unit,
                        window_expires_at_ic: *window_expires_at_ic,
                    });
                }
                *from_unit
            }
            _ => return Err(EngagementError::OccupantNotStranded(occupant)),
        };

        let (destination_port, duration_ic) =
            select_extraction_port(candidates, fallback_duration_ic);
        let completes_at_ic = at_ic.saturating_add(duration_ic);

        self.occupants.insert(
            occupant,
            OccupantSurvivalState::NpcExtractionInFlight {
                left_unit_in_field: from_unit,
                destination_port,
                requested_at_ic: at_ic,
                completes_at_ic,
                paused_at_real_s: None,
            },
        );
        self.occupants
            .get(&occupant)
            .ok_or(EngagementError::UnknownOccupant(occupant))
    }

    /// Pause all in-flight NPC extractions during a global world outage freeze (`Q-F4`, `Q-H11`).
    pub fn pause_extractions_for_outage(&mut self, freeze_at_real_s: i64) {
        for state in self.occupants.values_mut() {
            if let OccupantSurvivalState::NpcExtractionInFlight {
                paused_at_real_s, ..
            } = state
                && paused_at_real_s.is_none()
            {
                *paused_at_real_s = Some(freeze_at_real_s);
            }
        }
    }

    /// Resume in-flight NPC extractions after a world outage freeze + 5-minute countdown,
    /// extending their completion IC timestamp by the frozen campaign duration (`Q-F4`, `Q-H11`).
    pub fn resume_extractions_after_outage(&mut self, unfreeze_at_real_s: i64, ic_clock_rate: u32) {
        let rate = i64::from(ic_clock_rate.max(1));
        for state in self.occupants.values_mut() {
            if let OccupantSurvivalState::NpcExtractionInFlight {
                completes_at_ic,
                paused_at_real_s,
                ..
            } = state
                && let Some(paused_real) = paused_at_real_s.take()
            {
                let delta_real = unfreeze_at_real_s.saturating_sub(paused_real).max(0);
                let delta_ic = delta_real.saturating_mul(rate);
                *completes_at_ic = completes_at_ic.saturating_add(delta_ic);
            }
        }
    }

    /// Settle a due NPC extraction once `now_ic >= completes_at_ic` (and not outage-paused),
    /// delivering the occupant to `destination_port` while leaving their hull in the field (`Q-F4`).
    ///
    /// # Errors
    /// Returns [`EngagementError`] if the occupant has no in-flight extraction, is paused
    /// for an outage, or `now_ic < completes_at_ic`.
    pub fn complete_npc_extraction(
        &mut self,
        occupant: ObjId,
        now_ic: i64,
    ) -> Result<&OccupantSurvivalState, EngagementError> {
        let current = self
            .occupants
            .get(&occupant)
            .ok_or(EngagementError::UnknownOccupant(occupant))?;

        let (left_unit_in_field, destination_port, completes_at_ic, paused_at_real_s) =
            match current {
                OccupantSurvivalState::NpcExtractionInFlight {
                    left_unit_in_field,
                    destination_port,
                    completes_at_ic,
                    paused_at_real_s,
                    ..
                } => (
                    *left_unit_in_field,
                    destination_port.clone(),
                    *completes_at_ic,
                    *paused_at_real_s,
                ),
                _ => return Err(EngagementError::ExtractionNotInFlight(occupant)),
            };

        if paused_at_real_s.is_some() {
            return Err(EngagementError::ExtractionPausedForOutage(occupant));
        }
        if now_ic < completes_at_ic {
            return Err(EngagementError::ExtractionNotYetDue {
                occupant,
                now_ic,
                completes_at_ic,
            });
        }

        self.occupants.insert(
            occupant,
            OccupantSurvivalState::RecoveredAtPort {
                port: destination_port,
                left_unit_in_field,
                arrived_at_ic: now_ic,
            },
        );
        self.occupants
            .get(&occupant)
            .ok_or(EngagementError::UnknownOccupant(occupant))
    }
}

/// How an observer acquired a post-battle or field combat fact (`Q-H6`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationSource {
    /// Directly witnessed by the observer's own sensors/line-of-sight during combat.
    DirectLineOfSight,
    /// Received via an explicit delivered sensor/comms report from another unit.
    DeliveredReport {
        /// Reporting unit or character.
        sender: ObjId,
    },
    /// Acquired by physically inspecting a field casualty (`inspect_field_unit`).
    FieldInspection,
    /// Acquired by reading a recovered unit's black box (`P-3`, `Q-H6`).
    RecoveredBlackBox {
        /// Hull whose black box was read.
        hull: ObjId,
    },
}

/// Single observer-scoped combat or field fact (`Q-H6`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "fact", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObservedCombatFact {
    /// Target contact sighted at a pose.
    ContactSighted {
        /// Observed unit.
        unit: ObjId,
        /// Observed pose.
        pose: FieldPose,
    },
    /// Section damage observed on a unit.
    SectionDamaged {
        /// Target unit.
        unit: ObjId,
        /// Damaged section.
        section: SectionId,
    },
    /// Unit observed entering a defeated/casualty operational state.
    UnitOperationalStateObserved {
        /// Target unit.
        unit: ObjId,
        /// Observed operational state.
        state: OperationalState,
    },
    /// Surviving components revealed via close field inspection or black-box log (`Q-H6`, `Q-S5`).
    SurvivingComponentsRevealed {
        /// Target unit.
        unit: ObjId,
        /// Surviving salvageable components.
        surviving_components: Vec<ComponentId>,
    },
}

/// Timestamped entry in an actor's observer-bounded combat record (`Q-H6`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserverCombatEntry {
    /// Observer who holds this fact.
    pub observer: ObjId,
    /// How the fact was acquired.
    pub source: ObservationSource,
    /// The observed fact.
    pub fact: ObservedCombatFact,
    /// IC timestamp when recorded.
    pub recorded_at_ic: i64,
}

/// Per-observer combat and post-battle knowledge book (`Q-H6`).
///
/// Ending an IC field engagement publishes **no** public replay or omniscient enemy
/// inventory; each actor sees only their own recorded entries.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldObservationBook {
    by_observer: BTreeMap<ObjId, Vec<ObserverCombatEntry>>,
}

impl FieldObservationBook {
    /// Construct an empty [`FieldObservationBook`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an observed fact for `observer` (`Q-H6`).
    ///
    /// # Errors
    /// Returns [`EngagementError`] if `observer.0 == 0` or the per-observer cap is reached.
    pub fn record_observation(
        &mut self,
        observer: ObjId,
        source: ObservationSource,
        fact: ObservedCombatFact,
        recorded_at_ic: i64,
    ) -> Result<(), EngagementError> {
        if observer.0 == 0 {
            return Err(EngagementError::InvalidObjectId("observer"));
        }
        let list = self.by_observer.entry(observer).or_default();
        if list.len() >= MAX_OBSERVATIONS_PER_ACTOR {
            return Err(EngagementError::MaxObservationsReached {
                observer,
                max: MAX_OBSERVATIONS_PER_ACTOR,
            });
        }
        list.push(ObserverCombatEntry {
            observer,
            source,
            fact,
            recorded_at_ic,
        });
        Ok(())
    }

    /// Share an observer's recorded facts about `subject_unit` with `recipient` via a
    /// delivered report (`Q-H6`).
    ///
    /// # Errors
    /// Returns [`EngagementError`] if IDs are invalid or recipient's observation cap is reached.
    pub fn deliver_report(
        &mut self,
        sender: ObjId,
        recipient: ObjId,
        subject_unit: ObjId,
        delivered_at_ic: i64,
    ) -> Result<usize, EngagementError> {
        if sender.0 == 0 || recipient.0 == 0 {
            return Err(EngagementError::InvalidObjectId("report_participant"));
        }
        let matching_facts: Vec<ObservedCombatFact> = self
            .by_observer
            .get(&sender)
            .map(|entries| {
                entries
                    .iter()
                    .filter(|e| match &e.fact {
                        ObservedCombatFact::ContactSighted { unit, .. }
                        | ObservedCombatFact::SectionDamaged { unit, .. }
                        | ObservedCombatFact::UnitOperationalStateObserved { unit, .. }
                        | ObservedCombatFact::SurvivingComponentsRevealed { unit, .. } => {
                            *unit == subject_unit
                        }
                    })
                    .map(|e| e.fact.clone())
                    .collect()
            })
            .unwrap_or_default();

        let count = matching_facts.len();
        for fact in matching_facts {
            self.record_observation(
                recipient,
                ObservationSource::DeliveredReport { sender },
                fact,
                delivered_at_ic,
            )?;
        }
        Ok(count)
    }

    /// Return the post-battle digest visible to `observer` (`Q-H6`).
    ///
    /// Returns an empty slice for any actor who did not witness the engagement or
    /// receive a report/inspection/black-box log.
    #[must_use]
    pub fn digest_for_observer(&self, observer: ObjId) -> &[ObserverCombatEntry] {
        self.by_observer.get(&observer).map_or(&[], Vec::as_slice)
    }

    /// Whether `observer` has any knowledge of `unit`'s surviving component inventory (`Q-H6`).
    #[must_use]
    pub fn knows_surviving_components(&self, observer: ObjId, unit: ObjId) -> bool {
        self.digest_for_observer(observer).iter().any(|e| {
            matches!(
                &e.fact,
                ObservedCombatFact::SurvivingComponentsRevealed { unit: u, .. } if *u == unit
            )
        })
    }
}

/// Errors produced by `H1-B` / `H1-C` field engagement, crew survival, and observation operations.
#[derive(Clone, Debug, PartialEq)]
pub enum EngagementError {
    /// Object id was `ObjId(0)`.
    InvalidObjectId(&'static str),
    /// Engagement participant targeted itself.
    SelfEngagement(ObjId),
    /// Combat instance id was `0`.
    ZeroInstanceId,
    /// Side tag was empty.
    EmptySideTag,
    /// Site is already in an active combat engagement.
    SiteAlreadyEngaged {
        /// Site object id.
        site: ObjId,
        /// Active combat instance id.
        instance_id: u64,
    },
    /// Site is not currently in an active combat engagement.
    SiteNotEngaged {
        /// Site object id.
        site: ObjId,
    },
    /// Pose site did not match the engagement book's site.
    SiteMismatch {
        /// Expected site.
        expected: ObjId,
        /// Actual pose site.
        actual: ObjId,
    },
    /// Unit is already present on the site.
    UnitAlreadyPresent(ObjId),
    /// Unit is not present on the site.
    UnknownSiteUnit(ObjId),
    /// Unit cannot move under its own power and is not being towed.
    UnitCannotMoveUnderOwnPower {
        /// Unit object id.
        unit: ObjId,
        /// Current operational state.
        state: OperationalState,
    },
    /// Site unit capacity reached.
    MaxSiteUnitsReached {
        /// Maximum allowed units.
        max: usize,
    },
    /// Demotion refused because the settlement gate is not yet clear (`Q-H4`).
    DemotionBlocked(DemotionSettlementGate),
    /// Carrier departure refused until the player/commander explicitly confirms leaving
    /// unaccounted/disabled sortie units behind (`Q-H10`).
    LeaveBehindConfirmationRequired {
        /// Carrier object id.
        carrier: ObjId,
        /// Units belonging to the carrier's sortie that are still out on the site.
        unaccounted_units: Vec<ObjId>,
    },
    /// Unknown crew/occupant id.
    UnknownOccupant(ObjId),
    /// Occupant is not in a stranded/casualty state.
    OccupantNotStranded(ObjId),
    /// Invalid non-positive duration.
    InvalidDuration(i64),
    /// Guaranteed NPC extraction is refused while a friendly rescue is actively under way
    /// within its published window (`Q-F4`, `P-5`).
    FriendlyRescueActive {
        /// Stranded occupant id.
        occupant: ObjId,
        /// Friendly rescuer unit id.
        rescuer_unit: ObjId,
        /// IC timestamp when the friendly rescue window expires.
        window_expires_at_ic: i64,
    },
    /// Occupant does not have an in-flight NPC extraction.
    ExtractionNotInFlight(ObjId),
    /// In-flight NPC extraction is currently paused due to a global outage freeze (`Q-F4`).
    ExtractionPausedForOutage(ObjId),
    /// In-flight NPC extraction has not yet reached its completion timestamp.
    ExtractionNotYetDue {
        /// Occupant id.
        occupant: ObjId,
        /// Current IC timestamp.
        now_ic: i64,
        /// Scheduled completion IC timestamp.
        completes_at_ic: i64,
    },
    /// Crew book capacity reached.
    MaxCrewRecordsReached {
        /// Maximum records.
        max: usize,
    },
    /// Per-observer observation cap reached.
    MaxObservationsReached {
        /// Observer id.
        observer: ObjId,
        /// Maximum entries.
        max: usize,
    },
    /// Underlying [`ConditionError`].
    Condition(ConditionError),
    /// Underlying [`UnitError`].
    Unit(UnitError),
}

impl fmt::Display for EngagementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidObjectId(field) => write!(f, "{field} object id must be non-zero"),
            Self::SelfEngagement(u) => write!(f, "unit {u} cannot engage itself"),
            Self::ZeroInstanceId => f.write_str("combat instance id must be non-zero"),
            Self::EmptySideTag => f.write_str("engagement side tag must not be empty"),
            Self::SiteAlreadyEngaged { site, instance_id } => {
                write!(
                    f,
                    "site {site} is already engaged in combat instance {instance_id}"
                )
            }
            Self::SiteNotEngaged { site } => write!(f, "site {site} is not currently engaged"),
            Self::SiteMismatch { expected, actual } => {
                write!(f, "site mismatch: expected {expected}, got {actual}")
            }
            Self::UnitAlreadyPresent(u) => write!(f, "unit {u} is already present on the site"),
            Self::UnknownSiteUnit(u) => write!(f, "unit {u} is not present on the site"),
            Self::UnitCannotMoveUnderOwnPower { unit, state } => {
                write!(
                    f,
                    "unit {unit} cannot move under own power in state {state}"
                )
            }
            Self::MaxSiteUnitsReached { max } => {
                write!(f, "site unit limit ({max}) reached")
            }
            Self::DemotionBlocked(gate) => write!(
                f,
                "engagement demotion blocked (pending_impacts={}, active_hostile_contacts={}, pending_boundary_handoffs={})",
                gate.pending_impacts, gate.active_hostile_contacts, gate.pending_boundary_handoffs
            ),
            Self::LeaveBehindConfirmationRequired {
                carrier,
                unaccounted_units,
            } => write!(
                f,
                "carrier {carrier} has {} unaccounted field unit(s); explicit leave-behind confirmation required",
                unaccounted_units.len()
            ),
            Self::UnknownOccupant(o) => write!(f, "unknown crew occupant {o}"),
            Self::OccupantNotStranded(o) => write!(f, "occupant {o} is not stranded in the field"),
            Self::InvalidDuration(d) => write!(f, "duration must be positive, got {d}"),
            Self::FriendlyRescueActive {
                occupant,
                rescuer_unit,
                window_expires_at_ic,
            } => write!(
                f,
                "NPC extraction for {occupant} refused while friendly rescue by {rescuer_unit} is active until IC {window_expires_at_ic}"
            ),
            Self::ExtractionNotInFlight(o) => {
                write!(f, "occupant {o} has no NPC extraction in flight")
            }
            Self::ExtractionPausedForOutage(o) => {
                write!(
                    f,
                    "NPC extraction for {occupant} is paused for world outage",
                    occupant = o
                )
            }
            Self::ExtractionNotYetDue {
                occupant,
                now_ic,
                completes_at_ic,
            } => write!(
                f,
                "NPC extraction for {occupant} not yet due (now={now_ic}, completes_at={completes_at_ic})"
            ),
            Self::MaxCrewRecordsReached { max } => {
                write!(f, "crew survival book limit ({max}) reached")
            }
            Self::MaxObservationsReached { observer, max } => {
                write!(f, "observer {observer} reached observation limit ({max})")
            }
            Self::Condition(e) => write!(f, "field condition error: {e}"),
            Self::Unit(e) => write!(f, "tactical unit error: {e}"),
        }
    }
}

impl std::error::Error for EngagementError {}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::error::Error;

    use mush_core::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, ObjId,
        OwnerId,
    };

    use super::*;
    use crate::field_ops::condition::{
        ComponentCondition, FieldCloseOutcome, FieldEpochDisposition, validate_close_outcome,
    };
    use crate::mobility::{
        LocomotionId, MediumId, MobilityMode, MobilityModeId, MobilityProfile, MobilityVocabulary,
        ModeRules,
    };
    use crate::unit::{
        CellAddr, CellId, CellSpec, ComponentSpec, ConditionId, FacetId, FacetSpec, InstallPoint,
        RulesValue, SectionSpec, UnitMutationOp, UnitSpec, UnitSpecBody,
    };

    type TestResult = Result<(), Box<dyn Error>>;

    fn build_test_biped(obj: ObjId) -> Result<(TacticalUnit, UnitAnatomyProfile), Box<dyn Error>> {
        let mut reg = DefinitionRegistryBuilder::new();
        let owner = OwnerId::new("test")?;
        let rules_ref = reg.register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("schema")?,
                DefinitionId::new("unit_rules")?,
                1,
                serde_json::json!({}),
            )?,
        )?;
        let mode_rules_ref = reg.register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("schema")?,
                DefinitionId::new("walk_rules")?,
                1,
                serde_json::json!({}),
            )?,
        )?;
        let cockpit_def = reg.register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("component")?,
                DefinitionId::new("cockpit")?,
                1,
                serde_json::json!({}),
            )?,
        )?;
        let reactor_def = reg.register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("component")?,
                DefinitionId::new("reactor")?,
                1,
                serde_json::json!({}),
            )?,
        )?;
        let autocannon_def = reg.register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("component")?,
                DefinitionId::new("autocannon")?,
                1,
                serde_json::json!({}),
            )?,
        )?;

        let head = SectionId::new("head")?;
        let torso = SectionId::new("torso")?;
        let left_leg = SectionId::new("left_leg")?;
        let right_leg = SectionId::new("right_leg")?;
        let internals = CellId::new("internals")?;
        let intact = ConditionId::new("intact")?;

        let cockpit = ComponentId::new("cockpit")?;
        let reactor = ComponentId::new("reactor")?;
        let autocannon = ComponentId::new("autocannon")?;
        let ammo = CellId::new("ammo")?;

        let make_sec = |id: SectionId, hp: i64| -> Result<SectionSpec, Box<dyn Error>> {
            Ok(SectionSpec {
                id,
                condition: intact.clone(),
                facets: vec![FacetSpec {
                    id: FacetId::new("front")?,
                    cells: vec![],
                }],
                slots: vec![],
                cells: vec![CellSpec {
                    id: internals.clone(),
                    min: 0,
                    max: hp,
                    init: hp,
                }],
            })
        };

        let mode_id = MobilityModeId::new("walk")?;
        let mob_reg = MobilityVocabulary::base().freeze();
        let mobility = Some(MobilityProfile::new(
            vec![MobilityMode::new(
                mode_id.clone(),
                [MediumId::new("ground")?],
                LocomotionId::new("biped")?,
                ModeRules::new(mode_rules_ref, serde_json::json!({}))?,
            )],
            mode_id,
            &mob_reg,
        )?);

        let body = UnitSpecBody {
            sections: vec![
                make_sec(head.clone(), 10)?,
                make_sec(torso.clone(), 30)?,
                make_sec(left_leg.clone(), 15)?,
                make_sec(right_leg.clone(), 15)?,
            ],
            unit_cells: vec![],
            components: vec![
                ComponentSpec {
                    id: cockpit,
                    definition: cockpit_def,
                    install: InstallPoint::Section(head.clone()),
                    condition: intact.clone(),
                    cells: vec![],
                    asset: None,
                },
                ComponentSpec {
                    id: reactor.clone(),
                    definition: reactor_def,
                    install: InstallPoint::Section(torso.clone()),
                    condition: intact.clone(),
                    cells: vec![],
                    asset: None,
                },
                ComponentSpec {
                    id: autocannon.clone(),
                    definition: autocannon_def,
                    install: InstallPoint::Section(torso.clone()),
                    condition: intact,
                    cells: vec![CellSpec {
                        id: ammo.clone(),
                        min: 0,
                        max: 20,
                        init: 20,
                    }],
                    asset: None,
                },
            ],
            capabilities: vec![],
            mobility,
            rules: RulesValue::new(rules_ref, serde_json::json!({}))?,
        };
        let unit_desc =
            body.descriptor(DefinitionKind::new("unit")?, DefinitionId::new("biped")?, 1)?;
        let unit_ref = reg.register(owner, unit_desc)?;
        let spec: UnitSpec = body.with_definition(unit_ref);

        let unit = TacticalUnit::from_spec(&spec, obj)?;
        let anatomy = UnitAnatomyProfile {
            core_sections: BTreeSet::from([torso]),
            control_sections: BTreeSet::from([head]),
            required_locomotion_sections: BTreeSet::new(),
            locomotion_sections: BTreeSet::from([left_leg, right_leg]),
            min_locomotion_sections: 2,
            power_components: BTreeSet::from([reactor]),
            weapon_components: BTreeSet::from([autocannon]),
            sensor_components: BTreeSet::new(),
            internals_cell: Some(internals),
            component_health_cell: None,
            weapon_ammo_cell: Some(ammo),
        };
        Ok((unit, anatomy))
    }

    #[test]
    fn promotion_reinforcement_damage_demotion_and_carrier_leave_behind_workflow() -> TestResult {
        let site = ObjId(500);
        let carrier = ObjId(900);
        let (unit_a, anatomy) = build_test_biped(ObjId(101))?;
        let (unit_b, _) = build_test_biped(ObjId(102))?;
        let (unit_c, _) = build_test_biped(ObjId(103))?;

        let mut ledger = FieldCustodyLedger::new();
        let mut book = FieldSiteEngagementBook::new(site)?;
        check_eq!(book.current_clock_rate(), IC_SURFACE_CLOCK_RATE);

        let pose_a = FieldPose {
            site,
            x_cm: 1_000,
            y_cm: 2_000,
        };
        let pose_b = FieldPose {
            site,
            x_cm: 4_000,
            y_cm: 2_000,
        };
        book.deploy_surface_unit(
            &mut ledger,
            &unit_a,
            &anatomy,
            SmolStr::new("alpha"),
            pose_a,
            Some(carrier),
            100,
        )?;
        book.deploy_surface_unit(
            &mut ledger,
            &unit_b,
            &anatomy,
            SmolStr::new("bravo"),
            pose_b,
            None,
            100,
        )?;

        let moved_pose_a = FieldPose {
            site,
            x_cm: 2_500,
            y_cm: 2_000,
        };
        let dist = book.record_surface_motion(unit_a.object(), moved_pose_a, None)?;
        check_eq!(dist, 1_500);

        let handoffs = book.promote_to_engagement(
            &mut ledger,
            42,
            FieldPromotionTrigger::ExplicitHostileAction {
                attacker: unit_b.object(),
                target: unit_a.object(),
            },
            1,
            120,
        )?;
        check_eq!(book.current_clock_rate(), COMBAT_WALL_CLOCK_RATE);
        check_eq!(handoffs.len(), 2);
        let handoff_a = handoffs
            .iter()
            .find(|h| h.unit == unit_a.object())
            .ok_or("missing handoff_a")?;
        check_eq!(handoff_a.handoff_pose, moved_pose_a);
        check_eq!(handoff_a.from_clock_rate, IC_SURFACE_CLOCK_RATE);
        check_eq!(handoff_a.to_clock_rate, COMBAT_WALL_CLOCK_RATE);
        check_eq!(handoff_a.preserved_distance_cm, 1_500);

        let entry_pose_c = FieldPose {
            site,
            x_cm: 0,
            y_cm: 0,
        };
        book.reinforce_engagement(
            &mut ledger,
            &unit_c,
            &anatomy,
            SmolStr::new("alpha"),
            entry_pose_c,
            Some(carrier),
            125,
        )?;

        let head = SectionId::new("head")?;
        let internals = CellId::new("internals")?;
        let headshot = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: head.clone(),
                        cell: internals.clone(),
                    },
                    value: 0,
                },
                UnitMutationOp::SetSectionCondition {
                    section: head,
                    condition: ConditionId::new("destroyed")?,
                },
            ],
            Some(unit_a.revision()),
            None,
        )?;
        let (disabled_a, assess_a) =
            book.apply_field_damage(&mut ledger, &unit_a, &headshot, &anatomy, 130)?;
        check_eq!(assess_a.state, OperationalState::Disabled);
        check_eq!(
            assess_a.epoch_disposition,
            FieldEpochDisposition::RemainOpen
        );

        book.withdraw_unit(
            &mut ledger,
            unit_c.object(),
            false,
            WithdrawalDestination::CarrierBay {
                carrier,
                slot: SmolStr::new("bay_1"),
            },
            135,
        )?;

        check!(matches!(
            book.demote_engagement(&mut ledger, 140),
            Err(EngagementError::DemotionBlocked(_))
        ));

        book.withdraw_unit(
            &mut ledger,
            unit_b.object(),
            false,
            WithdrawalDestination::PerimeterSurface {
                pose: FieldPose {
                    site: ObjId(501),
                    x_cm: 0,
                    y_cm: 0,
                },
            },
            142,
        )?;
        book.set_settlement_gate(DemotionSettlementGate::default());
        let demoted = book.demote_engagement(&mut ledger, 145)?;
        check_eq!(book.current_clock_rate(), IC_SURFACE_CLOCK_RATE);
        check_eq!(demoted.persistent_field_casualties, vec![unit_a.object()]);
        check!(demoted.operational_surface_units.is_empty());

        let dep_err = book.depart_carrier(carrier, false, 150);
        check!(matches!(
            dep_err,
            Err(EngagementError::LeaveBehindConfirmationRequired {
                carrier: c,
                ref unaccounted_units,
            }) if c == carrier && unaccounted_units == &vec![unit_a.object()]
        ));

        let dep_ok = book.depart_carrier(carrier, true, 152)?;
        check_eq!(dep_ok.stowed_units, vec![unit_c.object()]);
        check_eq!(dep_ok.left_behind_units, vec![unit_a.object()]);
        check!(book.unit_presence(unit_a.object()).is_some());

        let torso = SectionId::new("torso")?;
        let left_leg = SectionId::new("left_leg")?;
        let right_leg = SectionId::new("right_leg")?;
        let finish_off = UnitMutationPlan::new(
            vec![
                UnitMutationOp::SetCell {
                    addr: CellAddr::Section {
                        section: torso.clone(),
                        cell: internals,
                    },
                    value: 0,
                },
                UnitMutationOp::SetSectionCondition {
                    section: torso,
                    condition: ConditionId::new("irrecoverable")?,
                },
                UnitMutationOp::SetSectionCondition {
                    section: left_leg,
                    condition: ConditionId::new("destroyed")?,
                },
                UnitMutationOp::SetSectionCondition {
                    section: right_leg,
                    condition: ConditionId::new("destroyed")?,
                },
            ],
            Some(disabled_a.revision()),
            None,
        )?;
        let (_ruined_a, ruined_assess) =
            book.apply_field_damage(&mut ledger, &disabled_a, &finish_off, &anatomy, 160)?;
        check_eq!(ruined_assess.state, OperationalState::Irrecoverable);
        check_eq!(
            ruined_assess
                .component_conditions
                .get(&ComponentId::new("autocannon")?),
            Some(&ComponentCondition::Destroyed)
        );
        check!(
            validate_close_outcome(
                unit_a.object(),
                ruined_assess.state,
                &FieldCloseOutcome::Destroyed
            )
            .is_ok()
        );

        Ok(())
    }

    #[test]
    fn link_loss_posture_and_crew_survival_npc_extraction_invariants() -> TestResult {
        check_eq!(
            resolve_link_loss_posture(10_000, 30_000, 5_000, Some("mind.guard")),
            LinkLossPosture::PlayerControlled
        );
        check_eq!(
            resolve_link_loss_posture(35_000, 30_000, 5_000, Some("mind.guard")),
            LinkLossPosture::SealedMindFallback {
                mind_id: SmolStr::new("mind.guard")
            }
        );
        check_eq!(
            resolve_link_loss_posture(35_000, 30_000, 5_000, None),
            LinkLossPosture::SafeHoldPosture
        );

        let pilot = ObjId(701);
        let hull = ObjId(101);
        let rescuer = ObjId(102);
        let pose = FieldPose {
            site: ObjId(500),
            x_cm: 500,
            y_cm: 500,
        };
        let mut crew = CrewSurvivalBook::new();
        crew.board_unit(pilot, hull)?;

        let state = crew.record_unit_defeat_survival(
            pilot,
            hull,
            OperationalState::Disabled,
            pose,
            true,
            true,
            1_000,
        )?;
        check!(matches!(
            state,
            OccupantSurvivalState::StrandedInField {
                from_unit,
                injured: true,
                ..
            } if *from_unit == hull
        ));

        crew.begin_friendly_rescue(pilot, rescuer, pose, 300, 1_100)?;

        let candidates = vec![SafePortCandidate {
            port_id: SmolStr::new("stickney"),
            eligible_and_known: true,
            transit_duration_ic: 600,
        }];
        let refused = crew.request_npc_extraction(pilot, &candidates, 900, 1_200);
        check!(matches!(
            refused,
            Err(EngagementError::FriendlyRescueActive {
                occupant,
                rescuer_unit,
                window_expires_at_ic: 1_400,
            }) if occupant == pilot && rescuer_unit == rescuer
        ));

        let in_flight = crew.request_npc_extraction(pilot, &candidates, 900, 1_450)?;
        check_eq!(
            *in_flight,
            OccupantSurvivalState::NpcExtractionInFlight {
                left_unit_in_field: hull,
                destination_port: SmolStr::new("stickney"),
                requested_at_ic: 1_450,
                completes_at_ic: 2_050,
                paused_at_real_s: None,
            }
        );

        crew.pause_extractions_for_outage(5_000);
        check!(matches!(
            crew.complete_npc_extraction(pilot, 3_000),
            Err(EngagementError::ExtractionPausedForOutage(p)) if p == pilot
        ));
        crew.resume_extractions_after_outage(5_100, IC_SURFACE_CLOCK_RATE);
        check!(matches!(
            crew.complete_npc_extraction(pilot, 4_000),
            Err(EngagementError::ExtractionNotYetDue {
                completes_at_ic: 4_450,
                ..
            })
        ));
        let arrived = crew.complete_npc_extraction(pilot, 4_450)?;
        check_eq!(
            *arrived,
            OccupantSurvivalState::RecoveredAtPort {
                port: SmolStr::new("stickney"),
                left_unit_in_field: hull,
                arrived_at_ic: 4_450,
            }
        );

        Ok(())
    }

    #[test]
    fn observer_bounded_post_battle_digest_prevents_omniscient_leaks() -> TestResult {
        let observer_a = ObjId(11);
        let observer_b = ObjId(12);
        let bystander_c = ObjId(13);
        let enemy_unit = ObjId(202);

        let mut obs = FieldObservationBook::new();
        obs.record_observation(
            observer_a,
            ObservationSource::DirectLineOfSight,
            ObservedCombatFact::UnitOperationalStateObserved {
                unit: enemy_unit,
                state: OperationalState::Disabled,
            },
            500,
        )?;
        obs.record_observation(
            observer_a,
            ObservationSource::FieldInspection,
            ObservedCombatFact::SurvivingComponentsRevealed {
                unit: enemy_unit,
                surviving_components: vec![ComponentId::new("autocannon")?],
            },
            510,
        )?;

        check!(obs.knows_surviving_components(observer_a, enemy_unit));
        check!(!obs.knows_surviving_components(observer_b, enemy_unit));
        check!(obs.digest_for_observer(bystander_c).is_empty());

        let delivered = obs.deliver_report(observer_a, observer_b, enemy_unit, 520)?;
        check_eq!(delivered, 2);
        check!(obs.knows_surviving_components(observer_b, enemy_unit));
        check!(obs.digest_for_observer(bystander_c).is_empty());

        Ok(())
    }
}

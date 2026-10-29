//! Detachment sortie lifecycle, carrier personnel embarkation manifest, NPC crew
//! hiring/wage accrual, unit expense attribution, and field-to-roster availability
//! reconciliation (design doc 27 §5, `FO-6`–`FO-9`, `FO-15`, `FO-16`, `Q-D3`–`Q-D6`,
//! `Q-A4`, `Q-F1`, `Q-F2`, `Q-R2`, `Q-R5`, `Q-H11`, `P-1`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};

use crate::detachment::{
    Connection, Controller, DetachmentId, DetachmentRoster, Leader, LeaderKind, LeadershipChange,
    Member, MemberFacts, RosterError, Slot,
};
use crate::field_ops::engagement::{
    CrewSurvivalBook, FieldSiteEngagementBook, OccupantSurvivalState,
};

/// Standard IC day duration in seconds (`86,400` IC seconds) used for daily NPC crew wage
/// accrual (`Q-R5`).
pub const IC_DAY_SECONDS: i64 = 86_400;

/// Maximum personnel entries tracked on a single [`CarrierPersonnelManifest`].
pub const MAX_MANIFEST_PERSONNEL: usize = 512;

/// Maximum NPC crew contracts tracked in a [`PortCrewPool`].
pub const MAX_CREW_POOL_CONTRACTS: usize = 1_024;

/// Whether a crew member or operator is a player character or a persistent human NPC (`FO-15`, `Q-R5`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrewMemberKind {
    /// Player character.
    Player,
    /// Persistent human NPC crew member (`FO-15`, `Q-R5`).
    HumanNpc,
}

impl From<LeaderKind> for CrewMemberKind {
    fn from(kind: LeaderKind) -> Self {
        match kind {
            LeaderKind::Player => Self::Player,
            LeaderKind::Npc => Self::HumanNpc,
        }
    }
}

/// Physical origin of an embarked crew member or passenger on a carrier or dropship
/// (`FO-16`, `Q-R2`).
///
/// Personnel cannot spawn out of thin air mid-flight; every person aboard a carrier must
/// trace to one of these four physical embarkation events.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "origin", rename_all = "snake_case", deny_unknown_fields)]
pub enum EmbarkationOrigin {
    /// Embarked while the carrier was docked at a port (`FO-16`, `Q-R2`).
    LastPortDeparture {
        /// Port object id where the person embarked.
        port: ObjId,
    },
    /// Transferred from another vessel during a ship-to-ship docking or shuttle transfer (`Q-R2`).
    ShipToShipTransfer {
        /// Source vessel object id.
        from_vessel: ObjId,
    },
    /// Rescued from a field site (`Q-R1`, `Q-R2`).
    FieldRescue {
        /// Site object id where the rescue occurred.
        site: ObjId,
    },
    /// Extracted from a recovered hull brought into the carrier's bay (`Q-R1`, `Q-R2`).
    RecoveredHullCrew {
        /// Recovered hull object id.
        hull: ObjId,
    },
}

/// Record of one person (player or human NPC) physically aboard a carrier (`FO-16`, `Q-R2`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbarkedPersonRecord {
    /// Person object id (`!= ObjId(0)`).
    pub person: ObjId,
    /// Player or human NPC classification.
    pub kind: CrewMemberKind,
    /// Physical embarkation provenance (`FO-16`, `Q-R2`).
    pub origin: EmbarkationOrigin,
    /// IC timestamp when the person embarked aboard this carrier.
    pub embarked_at_ic: i64,
    /// Currently assigned sortie unit (`Some(unit)` while launched from the carrier, `None` while aboard).
    pub deployed_in_unit: Option<ObjId>,
}

/// Physical personnel manifest for a carrier or dropship (`FO-16`, `Q-R2`).
///
/// Enforces that replacement pilots, crew, or NPC detachment commanders assigned mid-expedition
/// must already be physically aboard the carrier (and not currently deployed in another unit
/// or incapacitated/stranded/captured).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CarrierPersonnelManifest {
    carrier: ObjId,
    personnel: BTreeMap<ObjId, EmbarkedPersonRecord>,
}

impl CarrierPersonnelManifest {
    /// Create an empty manifest for `carrier`.
    ///
    /// # Errors
    /// Returns [`SortieError::InvalidObjectId`] if `carrier == ObjId(0)`.
    pub fn new(carrier: ObjId) -> Result<Self, SortieError> {
        if carrier.0 == 0 {
            return Err(SortieError::InvalidObjectId("carrier"));
        }
        Ok(Self {
            carrier,
            personnel: BTreeMap::new(),
        })
    }

    /// Carrier vessel object id.
    #[must_use]
    pub const fn carrier(&self) -> ObjId {
        self.carrier
    }

    /// All embarked personnel records keyed by person [`ObjId`].
    #[must_use]
    pub const fn personnel(&self) -> &BTreeMap<ObjId, EmbarkedPersonRecord> {
        &self.personnel
    }

    /// Look up an embarked person by [`ObjId`].
    #[must_use]
    pub fn person(&self, person: ObjId) -> Option<&EmbarkedPersonRecord> {
        self.personnel.get(&person)
    }

    /// Embark a person onto the carrier with explicit physical provenance (`FO-16`, `Q-R2`).
    ///
    /// # Errors
    /// Returns [`SortieError`] if any object id is zero, the manifest is full, or the person
    /// is already embarked and deployed in a field unit.
    pub fn embark_person(
        &mut self,
        person: ObjId,
        kind: CrewMemberKind,
        origin: EmbarkationOrigin,
        at_ic: i64,
    ) -> Result<&EmbarkedPersonRecord, SortieError> {
        if person.0 == 0 {
            return Err(SortieError::InvalidObjectId("person"));
        }
        match &origin {
            EmbarkationOrigin::LastPortDeparture { port } if port.0 == 0 => {
                return Err(SortieError::InvalidObjectId("port"));
            }
            EmbarkationOrigin::ShipToShipTransfer { from_vessel } if from_vessel.0 == 0 => {
                return Err(SortieError::InvalidObjectId("from_vessel"));
            }
            EmbarkationOrigin::FieldRescue { site } if site.0 == 0 => {
                return Err(SortieError::InvalidObjectId("site"));
            }
            EmbarkationOrigin::RecoveredHullCrew { hull } if hull.0 == 0 => {
                return Err(SortieError::InvalidObjectId("hull"));
            }
            _ => {}
        }
        if let Some(existing) = self.personnel.get(&person)
            && let Some(active_unit) = existing.deployed_in_unit
        {
            return Err(SortieError::PersonAlreadyDeployedInUnit {
                person,
                unit: active_unit,
            });
        }
        if !self.personnel.contains_key(&person) && self.personnel.len() >= MAX_MANIFEST_PERSONNEL {
            return Err(SortieError::MaxManifestPersonnelReached {
                max: MAX_MANIFEST_PERSONNEL,
            });
        }

        self.personnel.insert(
            person,
            EmbarkedPersonRecord {
                person,
                kind,
                origin,
                embarked_at_ic: at_ic,
                deployed_in_unit: None,
            },
        );
        self.personnel
            .get(&person)
            .ok_or(SortieError::PersonNotOnCarrierManifest {
                carrier: self.carrier,
                person,
            })
    }

    /// Disembark a person from the carrier (e.g. at port or via ship-to-ship transfer).
    ///
    /// # Errors
    /// Returns [`SortieError`] if the person is not on the manifest or is currently deployed
    /// in a launched unit.
    pub fn disembark_person(&mut self, person: ObjId) -> Result<EmbarkedPersonRecord, SortieError> {
        let existing =
            self.personnel
                .get(&person)
                .ok_or(SortieError::PersonNotOnCarrierManifest {
                    carrier: self.carrier,
                    person,
                })?;
        if let Some(unit) = existing.deployed_in_unit {
            return Err(SortieError::PersonAlreadyDeployedInUnit { person, unit });
        }
        self.personnel
            .remove(&person)
            .ok_or(SortieError::PersonNotOnCarrierManifest {
                carrier: self.carrier,
                person,
            })
    }

    /// Check whether `person` is physically aboard the carrier, not currently deployed in
    /// another unit, and not injured/stranded/captured in `crew_book` (`FO-16`, `Q-R2`).
    #[must_use]
    pub fn is_available_for_seat(&self, person: ObjId, crew_book: &CrewSurvivalBook) -> bool {
        let Some(rec) = self.personnel.get(&person) else {
            return false;
        };
        if rec.deployed_in_unit.is_some() {
            return false;
        }
        is_occupant_fit_for_duty(person, crew_book)
    }

    fn mark_deployed(&mut self, person: ObjId, unit: ObjId) -> Result<(), SortieError> {
        let rec =
            self.personnel
                .get_mut(&person)
                .ok_or(SortieError::PersonNotOnCarrierManifest {
                    carrier: self.carrier,
                    person,
                })?;
        if let Some(other_unit) = rec.deployed_in_unit
            && other_unit != unit
        {
            return Err(SortieError::PersonAlreadyDeployedInUnit {
                person,
                unit: other_unit,
            });
        }
        rec.deployed_in_unit = Some(unit);
        Ok(())
    }

    fn mark_returned_to_carrier(&mut self, person: ObjId) {
        if let Some(rec) = self.personnel.get_mut(&person) {
            rec.deployed_in_unit = None;
        }
    }
}

/// Check whether `occupant` is fit to operate or command a unit according to `crew_book`.
#[must_use]
pub fn is_occupant_fit_for_duty(occupant: ObjId, crew_book: &CrewSurvivalBook) -> bool {
    match crew_book.state(occupant) {
        None
        | Some(
            OccupantSurvivalState::AboardUnit { injured: false, .. }
            | OccupantSurvivalState::RecoveredAtPort { .. },
        ) => true,
        Some(
            OccupantSurvivalState::AboardUnit { injured: true, .. }
            | OccupantSurvivalState::StrandedInField { .. }
            | OccupantSurvivalState::FriendlyRescueUnderway { .. }
            | OccupantSurvivalState::NpcExtractionInFlight { .. }
            | OccupantSurvivalState::Captured { .. },
        ) => false,
    }
}

/// Authorized mission scope for a hired human NPC crew member (`FO-15`, `Q-R5`, `Q-F1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NpcMissionScope {
    /// Standard vehicle crew/operator seat under a player or NPC detachment commander.
    SubordinateOperator,
    /// Authorized to command a detachment sortie (`FO-15`, `Q-F1`, `Q-F2`).
    DetachmentCommander,
}

/// Employment contract for a persistent human NPC crew member hired from a port pool (`Q-R5`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpcCrewContract {
    /// Persistent human NPC object id.
    pub npc: ObjId,
    /// Home port where the NPC was hired.
    pub home_port: ObjId,
    /// Employer / payer principal (`None` while unassigned in the port hiring pool).
    pub employer: Option<ObjId>,
    /// Daily wage in credits per [`IC_DAY_SECONDS`] (`86,400` IC seconds).
    pub daily_wage_credits: u64,
    /// Authorized mission scope (`SubordinateOperator` or `DetachmentCommander`).
    pub mission_scope: NpcMissionScope,
    /// IC timestamp when the current employment contract started.
    pub hired_at_ic: Option<i64>,
    /// IC timestamp through which wages have been accrued.
    pub last_wage_accrual_ic: Option<i64>,
    /// Unpaid accrued wage balance in credits owed by `employer`.
    pub unpaid_accrued_wages: u64,
}

/// Port-based NPC crew hiring pool and active contract ledger (`Q-R5`, `Q-H11`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortCrewPool {
    contracts: BTreeMap<ObjId, NpcCrewContract>,
}

impl PortCrewPool {
    /// Construct an empty [`PortCrewPool`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// All registered NPC crew contracts keyed by NPC [`ObjId`].
    #[must_use]
    pub const fn contracts(&self) -> &BTreeMap<ObjId, NpcCrewContract> {
        &self.contracts
    }

    /// Look up an NPC's contract by [`ObjId`].
    #[must_use]
    pub fn contract(&self, npc: ObjId) -> Option<&NpcCrewContract> {
        self.contracts.get(&npc)
    }

    /// Register an available human NPC in the port hiring pool (`Q-R5`).
    ///
    /// # Errors
    /// Returns [`SortieError`] if `npc` or `home_port` is zero, the pool is full, or `npc`
    /// is already hired by an employer.
    pub fn register_available_npc(
        &mut self,
        npc: ObjId,
        home_port: ObjId,
        daily_wage_credits: u64,
        mission_scope: NpcMissionScope,
    ) -> Result<&NpcCrewContract, SortieError> {
        if npc.0 == 0 {
            return Err(SortieError::InvalidObjectId("npc"));
        }
        if home_port.0 == 0 {
            return Err(SortieError::InvalidObjectId("home_port"));
        }
        if let Some(existing) = self.contracts.get(&npc)
            && let Some(employer) = existing.employer
        {
            return Err(SortieError::NpcAlreadyHired { npc, employer });
        }
        if !self.contracts.contains_key(&npc) && self.contracts.len() >= MAX_CREW_POOL_CONTRACTS {
            return Err(SortieError::MaxCrewContractsReached {
                max: MAX_CREW_POOL_CONTRACTS,
            });
        }
        self.contracts.insert(
            npc,
            NpcCrewContract {
                npc,
                home_port,
                employer: None,
                daily_wage_credits,
                mission_scope,
                hired_at_ic: None,
                last_wage_accrual_ic: None,
                unpaid_accrued_wages: 0,
            },
        );
        self.contracts
            .get(&npc)
            .ok_or(SortieError::UnknownNpcCrew(npc))
    }

    /// Hire an available NPC from `at_port` under `employer` (`Q-R5`).
    ///
    /// # Errors
    /// Returns [`SortieError`] if the NPC is unknown, already hired, not at `at_port`,
    /// or `employer` is zero.
    pub fn hire_npc(
        &mut self,
        npc: ObjId,
        employer: ObjId,
        at_port: ObjId,
        at_ic: i64,
    ) -> Result<&NpcCrewContract, SortieError> {
        if employer.0 == 0 {
            return Err(SortieError::InvalidObjectId("employer"));
        }
        if at_port.0 == 0 {
            return Err(SortieError::InvalidObjectId("at_port"));
        }
        let contract = self
            .contracts
            .get_mut(&npc)
            .ok_or(SortieError::UnknownNpcCrew(npc))?;
        if let Some(current_emp) = contract.employer {
            return Err(SortieError::NpcAlreadyHired {
                npc,
                employer: current_emp,
            });
        }
        if contract.home_port != at_port {
            return Err(SortieError::NpcNotAtPort {
                npc,
                expected_port: contract.home_port,
                actual_port: at_port,
            });
        }
        contract.employer = Some(employer);
        contract.hired_at_ic = Some(at_ic);
        contract.last_wage_accrual_ic = Some(at_ic);
        Ok(contract)
    }

    /// Accrue daily IC wages for all hired NPCs through `now_ic`, subtracting any
    /// `frozen_duration_ic` caused by server outage freezes (`Q-R5`, `Q-H11`).
    ///
    /// Returns the total newly accrued wage amount across all contracts.
    pub fn settle_npc_wages(&mut self, now_ic: i64, frozen_duration_ic: i64) -> u64 {
        let freeze_deduction = frozen_duration_ic.max(0);
        let mut newly_accrued_total: u64 = 0;

        for contract in self.contracts.values_mut() {
            if contract.employer.is_none() {
                continue;
            }
            let Some(last_ic) = contract.last_wage_accrual_ic else {
                contract.last_wage_accrual_ic = Some(now_ic);
                continue;
            };
            let raw_elapsed = now_ic.saturating_sub(last_ic);
            let billable_elapsed = raw_elapsed.saturating_sub(freeze_deduction);
            if billable_elapsed < IC_DAY_SECONDS {
                continue;
            }
            let full_days = billable_elapsed / IC_DAY_SECONDS;
            let days_u64 = u64::try_from(full_days).unwrap_or(0);
            let wage_add = days_u64.saturating_mul(contract.daily_wage_credits);
            contract.unpaid_accrued_wages = contract.unpaid_accrued_wages.saturating_add(wage_add);
            newly_accrued_total = newly_accrued_total.saturating_add(wage_add);

            let consumed_ic = full_days
                .saturating_mul(IC_DAY_SECONDS)
                .saturating_add(freeze_deduction);
            contract.last_wage_accrual_ic = Some(last_ic.saturating_add(consumed_ic));
        }

        newly_accrued_total
    }

    /// Record payment of `amount` against `npc`'s accrued wage balance.
    ///
    /// # Errors
    /// Returns [`SortieError::UnknownNpcCrew`] if `npc` is not registered.
    pub fn pay_accrued_wages(&mut self, npc: ObjId, amount: u64) -> Result<u64, SortieError> {
        let contract = self
            .contracts
            .get_mut(&npc)
            .ok_or(SortieError::UnknownNpcCrew(npc))?;
        let paid = amount.min(contract.unpaid_accrued_wages);
        contract.unpaid_accrued_wages = contract.unpaid_accrued_wages.saturating_sub(paid);
        Ok(paid)
    }

    /// Release a hired NPC back to `return_port`'s hiring pool once unpaid wages are settled (`Q-R5`).
    ///
    /// # Errors
    /// Returns [`SortieError`] if the NPC is unknown, not currently hired, or still has unpaid
    /// accrued wages.
    pub fn release_npc_at_port(
        &mut self,
        npc: ObjId,
        return_port: ObjId,
    ) -> Result<&NpcCrewContract, SortieError> {
        if return_port.0 == 0 {
            return Err(SortieError::InvalidObjectId("return_port"));
        }
        let contract = self
            .contracts
            .get_mut(&npc)
            .ok_or(SortieError::UnknownNpcCrew(npc))?;
        if contract.employer.is_none() {
            return Err(SortieError::NpcNotHired(npc));
        }
        if contract.unpaid_accrued_wages > 0 {
            return Err(SortieError::UnpaidWagesOutstanding {
                npc,
                unpaid_credits: contract.unpaid_accrued_wages,
            });
        }
        contract.home_port = return_port;
        contract.employer = None;
        contract.hired_at_ic = None;
        contract.last_wage_accrual_ic = None;
        Ok(contract)
    }
}

/// Category of sortie or post-sortie unit expense (`Q-A4`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitExpenseCategory {
    /// Ammunition, fuel, and expended tactical consumables.
    AmmunitionAndConsumables,
    /// Field towing, crane extraction, and recovery operations.
    FieldRecoveryAndTowing,
    /// Hangar/workshop structural repair, section reconstruction, and component replacement.
    WorkshopRepairAndReplacement,
}

/// Explicit per-category expense payer policy for a deployed unit (`Q-A4`).
///
/// Defaults all categories to the detachment organizer so multi-owner detachments have
/// unambiguous cost attribution unless the organizer and unit owner configure a split.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitExpensePolicy {
    /// Principal billed for ammunition and consumables.
    pub ammo_payer: ObjId,
    /// Principal billed for field recovery and towing.
    pub recovery_payer: ObjId,
    /// Principal billed for workshop repair and component replacement.
    pub repair_payer: ObjId,
}

impl UnitExpensePolicy {
    /// Default expense policy where `organizer` pays all categories (`Q-A4`).
    #[must_use]
    pub const fn organizer_pays_all(organizer: ObjId) -> Self {
        Self {
            ammo_payer: organizer,
            recovery_payer: organizer,
            repair_payer: organizer,
        }
    }

    /// Resolve the payer principal for `category`.
    #[must_use]
    pub const fn payer_for(&self, category: UnitExpenseCategory) -> ObjId {
        match category {
            UnitExpenseCategory::AmmunitionAndConsumables => self.ammo_payer,
            UnitExpenseCategory::FieldRecoveryAndTowing => self.recovery_payer,
            UnitExpenseCategory::WorkshopRepairAndReplacement => self.repair_payer,
        }
    }
}

/// Bounded mission-leader authority granted to a sortie commander or automatic successor (`Q-F2`).
///
/// Grants tactical command over the detachment's launched units, field recovery initiation,
/// and retreat/extract orders for this sortie only, without granting persistent organization
/// role permissions or asset title transfer authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionLeaderRoleGrant {
    /// Sortie id to which this grant is scoped.
    pub sortie_id: u64,
    /// Person holding bounded mission-leader authority.
    pub leader: ObjId,
    /// Whether `leader` is a player or an embarked human NPC (`FO-15`, `Q-F1`).
    pub leader_kind: CrewMemberKind,
    /// Slot from which `leader` commands.
    pub command_slot: Slot,
    /// Roster leadership term associated with this grant.
    pub term: u64,
    /// May issue tactical movement/fire/retreat orders to sortie units.
    pub can_direct_tactical_units: bool,
    /// May initiate lawful field towing/recovery for sortie units (`Q-F2`).
    pub can_authorize_field_recovery: bool,
    /// Persistent org-role mutation or asset-title transfer is always `false` (`Q-F2`).
    pub grants_persistent_org_authority: bool,
    /// IC timestamp when this mission-leader grant took effect.
    pub granted_at_ic: i64,
}

/// Per-slot launch state within an [`ActiveFieldSortie`] (`Q-D6`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum SortieSlotLaunchState {
    /// Slot is staged on the roster and has not yet physically launched.
    PendingLaunch,
    /// Slot has physically launched into the field (`Q-D6`: remains out in the field even if
    /// sibling slots fail to launch).
    Launched {
        /// IC timestamp when this slot launched.
        launched_at_ic: i64,
    },
    /// Slot failed to launch and remains docked/staged so it can be retried or cancelled
    /// without rolling back already-launched slots (`Q-D6`).
    LaunchBlocked {
        /// Reason the slot could not launch on the last attempt.
        reason: SortieLaunchBlockReason,
    },
    /// Unlaunched slot was explicitly cancelled by the organizer (`Q-D6`).
    CancelledBeforeLaunch {
        /// IC timestamp when the unlaunched slot was cancelled.
        cancelled_at_ic: i64,
    },
    /// Launched unit has returned to bay/hangar custody.
    ReturnedToBay {
        /// IC timestamp when the unit returned.
        returned_at_ic: i64,
    },
}

/// Reason an individual slot could not launch during a sortie launch attempt (`Q-D6`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortieLaunchBlockReason {
    /// Assigned operator is not on the carrier manifest or is already deployed in another unit (`FO-16`).
    OperatorUnavailableOnManifest,
    /// Assigned human NPC operator lacks an active employment contract (`Q-R5`).
    NpcOperatorNotContracted,
    /// Assigned operator is currently injured, stranded, or captured in `CrewSurvivalBook`.
    OperatorUnfitForDuty,
}

/// Active field sortie coordinating a [`DetachmentRoster`], carrier embarkation manifest,
/// partial-launch persistence (`Q-D6`), no-player NPC-led sorties (`FO-15`, `Q-F1`), and
/// bounded mission-leader succession (`Q-F2`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveFieldSortie {
    sortie_id: u64,
    detachment_id: DetachmentId,
    organizer: ObjId,
    carrier: Option<ObjId>,
    slot_states: BTreeMap<Slot, SortieSlotLaunchState>,
    expense_policies: BTreeMap<ObjId, UnitExpensePolicy>,
    leader_grant: Option<MissionLeaderRoleGrant>,
    created_at_ic: i64,
}

impl ActiveFieldSortie {
    /// Stage a new [`ActiveFieldSortie`] from a resolved [`DetachmentRoster`] (`FO-6`–`FO-9`,
    /// `FO-15`, `Q-F1`, `Q-F2`).
    ///
    /// Supports both player-led sorties and organizer-launched no-player sorties commanded
    /// by an embarked human NPC whose contract has [`NpcMissionScope::DetachmentCommander`].
    ///
    /// # Errors
    /// Returns [`SortieError`] if `sortie_id == 0`, `organizer == ObjId(0)`, the roster has
    /// no members, or a nominated NPC commander lacks [`NpcMissionScope::DetachmentCommander`].
    pub fn stage_from_roster(
        sortie_id: u64,
        organizer: ObjId,
        carrier: Option<ObjId>,
        roster: &DetachmentRoster,
        crew_pool: &PortCrewPool,
        created_at_ic: i64,
    ) -> Result<Self, SortieError> {
        if sortie_id == 0 {
            return Err(SortieError::InvalidSortieId);
        }
        if organizer.0 == 0 {
            return Err(SortieError::InvalidObjectId("organizer"));
        }
        if let Some(c) = carrier
            && c.0 == 0
        {
            return Err(SortieError::InvalidObjectId("carrier"));
        }
        if roster.members().is_empty() {
            return Err(SortieError::EmptyRoster);
        }

        // Validate initial leader if roster has already been reconciled, or validate the first
        // player / NPC commander candidate on the roster.
        let initial_leader = if let Some(leader) = roster.leader() {
            if leader.kind == LeaderKind::Npc {
                validate_npc_commander_contract(leader.character, crew_pool)?;
            }
            Some(MissionLeaderRoleGrant {
                sortie_id,
                leader: leader.character,
                leader_kind: CrewMemberKind::from(leader.kind),
                command_slot: leader.slot,
                term: roster.term(),
                can_direct_tactical_units: true,
                can_authorize_field_recovery: true,
                grants_persistent_org_authority: false,
                granted_at_ic: created_at_ic,
            })
        } else {
            resolve_initial_grant_from_members(sortie_id, roster, crew_pool, created_at_ic)?
        };

        let mut slot_states = BTreeMap::new();
        let mut expense_policies = BTreeMap::new();
        for member in roster.members() {
            slot_states.insert(member.slot, SortieSlotLaunchState::PendingLaunch);
            expense_policies
                .entry(member.unit)
                .or_insert_with(|| UnitExpensePolicy::organizer_pays_all(organizer));
        }

        Ok(Self {
            sortie_id,
            detachment_id: roster.id(),
            organizer,
            carrier,
            slot_states,
            expense_policies,
            leader_grant: initial_leader,
            created_at_ic,
        })
    }

    /// Sortie identifier (`>= 1`).
    #[must_use]
    pub const fn sortie_id(&self) -> u64 {
        self.sortie_id
    }

    /// Detachment identifier from the [`DetachmentRoster`].
    #[must_use]
    pub const fn detachment_id(&self) -> DetachmentId {
        self.detachment_id
    }

    /// Organizer principal (`FO-6`).
    #[must_use]
    pub const fn organizer(&self) -> ObjId {
        self.organizer
    }

    /// Carrier vessel from which the sortie launches, if carrier-borne (`FO-16`).
    #[must_use]
    pub const fn carrier(&self) -> Option<ObjId> {
        self.carrier
    }

    /// Current bounded [`MissionLeaderRoleGrant`], if a leader is active (`Q-F2`).
    #[must_use]
    pub const fn leader_grant(&self) -> Option<&MissionLeaderRoleGrant> {
        self.leader_grant.as_ref()
    }

    /// Per-slot launch states (`Q-D6`).
    #[must_use]
    pub const fn slot_states(&self) -> &BTreeMap<Slot, SortieSlotLaunchState> {
        &self.slot_states
    }

    /// Expense policy map keyed by unit [`ObjId`] (`Q-A4`).
    #[must_use]
    pub const fn expense_policies(&self) -> &BTreeMap<ObjId, UnitExpensePolicy> {
        &self.expense_policies
    }

    /// IC timestamp when the sortie was staged.
    #[must_use]
    pub const fn created_at_ic(&self) -> i64 {
        self.created_at_ic
    }

    /// Override the [`UnitExpensePolicy`] for `unit` (`Q-A4`).
    ///
    /// # Errors
    /// Returns [`SortieError::InvalidObjectId`] if `unit` or any payer id is zero.
    pub fn set_unit_expense_policy(
        &mut self,
        unit: ObjId,
        policy: UnitExpensePolicy,
    ) -> Result<(), SortieError> {
        if unit.0 == 0
            || policy.ammo_payer.0 == 0
            || policy.recovery_payer.0 == 0
            || policy.repair_payer.0 == 0
        {
            return Err(SortieError::InvalidObjectId("expense_policy"));
        }
        self.expense_policies.insert(unit, policy);
        Ok(())
    }

    /// Attempt to launch a single slot (`Q-D6`).
    ///
    /// If the slot's operator is physically available on `manifest` (when carrier-borne),
    /// fit for duty in `crew_book`, and contracted in `crew_pool` (when a human NPC),
    /// transitions the slot to [`SortieSlotLaunchState::Launched`] and marks the operator
    /// deployed on `manifest`. Otherwise records [`SortieSlotLaunchState::LaunchBlocked`]
    /// on this slot without rolling back any already-launched sibling slots (`Q-D6`).
    ///
    /// # Errors
    /// Returns [`SortieError`] if `slot` is unknown or already launched/cancelled.
    pub fn launch_slot(
        &mut self,
        slot: Slot,
        roster: &DetachmentRoster,
        manifest: Option<&mut CarrierPersonnelManifest>,
        crew_pool: &PortCrewPool,
        crew_book: &CrewSurvivalBook,
        at_ic: i64,
    ) -> Result<SortieSlotLaunchState, SortieError> {
        let current_state = self
            .slot_states
            .get(&slot)
            .copied()
            .ok_or(SortieError::UnknownSlot(slot))?;

        if matches!(
            current_state,
            SortieSlotLaunchState::Launched { .. }
                | SortieSlotLaunchState::CancelledBeforeLaunch { .. }
                | SortieSlotLaunchState::ReturnedToBay { .. }
        ) {
            return Err(SortieError::InvalidSlotTransition {
                slot,
                current: current_state,
            });
        }

        let member = roster.member(slot).ok_or(SortieError::UnknownSlot(slot))?;

        if let Controller::Npc { character } = member.controller {
            let is_contracted = crew_pool
                .contract(character)
                .is_some_and(|c| c.employer.is_some());
            if !is_contracted {
                let state = SortieSlotLaunchState::LaunchBlocked {
                    reason: SortieLaunchBlockReason::NpcOperatorNotContracted,
                };
                self.slot_states.insert(slot, state);
                return Ok(state);
            }
        }

        if let Some(character) = member.controller.character() {
            if !is_occupant_fit_for_duty(character, crew_book) {
                let state = SortieSlotLaunchState::LaunchBlocked {
                    reason: SortieLaunchBlockReason::OperatorUnfitForDuty,
                };
                self.slot_states.insert(slot, state);
                return Ok(state);
            }
            if let Some(carrier_manifest) = manifest {
                if !carrier_manifest.is_available_for_seat(character, crew_book) {
                    let state = SortieSlotLaunchState::LaunchBlocked {
                        reason: SortieLaunchBlockReason::OperatorUnavailableOnManifest,
                    };
                    self.slot_states.insert(slot, state);
                    return Ok(state);
                }
                carrier_manifest.mark_deployed(character, member.unit)?;
            }
        }

        let launched = SortieSlotLaunchState::Launched {
            launched_at_ic: at_ic,
        };
        self.slot_states.insert(slot, launched);
        self.expense_policies
            .entry(member.unit)
            .or_insert_with(|| UnitExpensePolicy::organizer_pays_all(self.organizer));
        Ok(launched)
    }

    /// Cancel an unlaunched (`PendingLaunch` or `LaunchBlocked`) slot without affecting
    /// already-launched units in the field (`Q-D6`).
    ///
    /// # Errors
    /// Returns [`SortieError`] if the slot is unknown or has already launched.
    pub fn cancel_unlaunched_slot(
        &mut self,
        slot: Slot,
        at_ic: i64,
    ) -> Result<SortieSlotLaunchState, SortieError> {
        let current_state = self
            .slot_states
            .get(&slot)
            .copied()
            .ok_or(SortieError::UnknownSlot(slot))?;

        if !matches!(
            current_state,
            SortieSlotLaunchState::PendingLaunch | SortieSlotLaunchState::LaunchBlocked { .. }
        ) {
            return Err(SortieError::InvalidSlotTransition {
                slot,
                current: current_state,
            });
        }

        let next = SortieSlotLaunchState::CancelledBeforeLaunch {
            cancelled_at_ic: at_ic,
        };
        self.slot_states.insert(slot, next);
        Ok(next)
    }

    /// Record that a launched slot has returned to the carrier or base bay.
    ///
    /// # Errors
    /// Returns [`SortieError`] if the slot is unknown or was not in `Launched` state.
    pub fn record_slot_return_to_bay(
        &mut self,
        slot: Slot,
        roster: &DetachmentRoster,
        manifest: Option<&mut CarrierPersonnelManifest>,
        at_ic: i64,
    ) -> Result<SortieSlotLaunchState, SortieError> {
        let current_state = self
            .slot_states
            .get(&slot)
            .copied()
            .ok_or(SortieError::UnknownSlot(slot))?;

        if !matches!(current_state, SortieSlotLaunchState::Launched { .. }) {
            return Err(SortieError::InvalidSlotTransition {
                slot,
                current: current_state,
            });
        }

        if let Some(carrier_manifest) = manifest
            && let Some(member) = roster.member(slot)
            && let Some(character) = member.controller.character()
        {
            carrier_manifest.mark_returned_to_carrier(character);
        }

        let next = SortieSlotLaunchState::ReturnedToBay {
            returned_at_ic: at_ic,
        };
        self.slot_states.insert(slot, next);
        Ok(next)
    }

    /// Reconcile `roster` against live field state in `site_book` and `crew_book` and update
    /// this sortie's bounded [`MissionLeaderRoleGrant`] on leadership succession (`Q-D4`, `Q-F2`, `P-1`).
    ///
    /// # Errors
    /// Returns [`SortieError::Roster`] if `roster.reconcile` fails.
    #[allow(clippy::too_many_arguments)]
    pub fn reconcile_leadership_from_field(
        &mut self,
        roster: &mut DetachmentRoster,
        site_book: &FieldSiteEngagementBook,
        crew_book: &CrewSurvivalBook,
        connections: &BTreeMap<ObjId, Connection>,
        command_authorized_slots: &BTreeSet<Slot>,
        now_ms: u64,
        at_ic: i64,
    ) -> Result<Option<LeadershipChange>, SortieError> {
        let change = sync_detachment_availability_from_field(
            roster,
            Some(&self.slot_states),
            site_book,
            crew_book,
            connections,
            command_authorized_slots,
            now_ms,
        )?;

        self.leader_grant = roster.leader().map(
            |Leader {
                 slot,
                 character,
                 kind,
             }| {
                MissionLeaderRoleGrant {
                    sortie_id: self.sortie_id,
                    leader: character,
                    leader_kind: CrewMemberKind::from(kind),
                    command_slot: slot,
                    term: roster.term(),
                    can_direct_tactical_units: true,
                    can_authorize_field_recovery: true,
                    grants_persistent_org_authority: false,
                    granted_at_ic: at_ic,
                }
            },
        );

        Ok(change)
    }
}

/// Build authoritative [`MemberFacts`] from a site's [`FieldSiteEngagementBook`],
/// [`CrewSurvivalBook`], optional sortie slot launch states, and session [`Connection`]s,
/// and reconcile `roster` in one step (`Q-D4`, `Q-F2`, `P-1`).
///
/// - `capable`: `true` iff any human operator (`Player` or `Npc`) is fit for duty in `crew_book`.
/// - `controls_unit`: `true` iff the slot is not `LaunchBlocked` or `CancelledBeforeLaunch`
///   and the operator has not abandoned the hull via NPC extraction.
/// - `unit_disabled`: `true` iff `site_book` records the unit in a combat-defeated state
///   (`Disabled`, `Wreck`, or `Irrecoverable`).
/// - `connection`: looked up from `connections` for `Controller::Player`, or `Connection::Connected`
///   for `Controller::Npc` and `Controller::Automated`.
///
/// # Errors
/// Returns [`SortieError::Roster`] if `roster.reconcile` fails.
pub fn sync_detachment_availability_from_field(
    roster: &mut DetachmentRoster,
    slot_launch_states: Option<&BTreeMap<Slot, SortieSlotLaunchState>>,
    site_book: &FieldSiteEngagementBook,
    crew_book: &CrewSurvivalBook,
    connections: &BTreeMap<ObjId, Connection>,
    command_authorized_slots: &BTreeSet<Slot>,
    now_ms: u64,
) -> Result<Option<LeadershipChange>, SortieError> {
    let mut facts = Vec::with_capacity(roster.members().len());

    for &Member {
        slot,
        unit,
        controller,
    } in roster.members()
    {
        let launch_ok = match slot_launch_states.and_then(|m| m.get(&slot)) {
            Some(
                SortieSlotLaunchState::LaunchBlocked { .. }
                | SortieSlotLaunchState::CancelledBeforeLaunch { .. },
            ) => false,
            Some(
                SortieSlotLaunchState::PendingLaunch
                | SortieSlotLaunchState::Launched { .. }
                | SortieSlotLaunchState::ReturnedToBay { .. },
            )
            | None => true,
        };

        let unit_disabled = site_book
            .unit_presence(unit)
            .is_some_and(|p| p.assessment.state.is_combat_defeated());

        let (capable, controls_unit, connection) = match controller {
            Controller::Player { character } => {
                let fit = is_occupant_fit_for_duty(character, crew_book);
                let aboard = match crew_book.state(character) {
                    None | Some(OccupantSurvivalState::AboardUnit { .. }) => true,
                    Some(
                        OccupantSurvivalState::StrandedInField { .. }
                        | OccupantSurvivalState::FriendlyRescueUnderway { .. }
                        | OccupantSurvivalState::NpcExtractionInFlight { .. }
                        | OccupantSurvivalState::RecoveredAtPort { .. }
                        | OccupantSurvivalState::Captured { .. },
                    ) => false,
                };
                let conn = connections
                    .get(&character)
                    .copied()
                    .unwrap_or(Connection::Connected);
                (fit, launch_ok && aboard, conn)
            }
            Controller::Npc { character } => {
                let fit = is_occupant_fit_for_duty(character, crew_book);
                let aboard = match crew_book.state(character) {
                    None | Some(OccupantSurvivalState::AboardUnit { .. }) => true,
                    Some(
                        OccupantSurvivalState::StrandedInField { .. }
                        | OccupantSurvivalState::FriendlyRescueUnderway { .. }
                        | OccupantSurvivalState::NpcExtractionInFlight { .. }
                        | OccupantSurvivalState::RecoveredAtPort { .. }
                        | OccupantSurvivalState::Captured { .. },
                    ) => false,
                };
                (fit, launch_ok && aboard, Connection::Connected)
            }
            Controller::Automated => (true, launch_ok, Connection::Connected),
        };

        facts.push(MemberFacts {
            slot,
            capable,
            controls_unit,
            unit_disabled,
            connection,
            command_authorized: command_authorized_slots.contains(&slot),
        });
    }

    roster
        .reconcile(now_ms, &facts)
        .map_err(SortieError::Roster)
}

fn validate_npc_commander_contract(
    npc: ObjId,
    crew_pool: &PortCrewPool,
) -> Result<(), SortieError> {
    let contract = crew_pool
        .contract(npc)
        .ok_or(SortieError::UnknownNpcCrew(npc))?;
    if contract.employer.is_none() {
        return Err(SortieError::NpcNotHired(npc));
    }
    if contract.mission_scope != NpcMissionScope::DetachmentCommander {
        return Err(SortieError::NpcLacksCommanderScope(npc));
    }
    Ok(())
}

fn resolve_initial_grant_from_members(
    sortie_id: u64,
    roster: &DetachmentRoster,
    crew_pool: &PortCrewPool,
    created_at_ic: i64,
) -> Result<Option<MissionLeaderRoleGrant>, SortieError> {
    for member in roster.members() {
        if let Controller::Player { character } = member.controller {
            return Ok(Some(MissionLeaderRoleGrant {
                sortie_id,
                leader: character,
                leader_kind: CrewMemberKind::Player,
                command_slot: member.slot,
                term: roster.term(),
                can_direct_tactical_units: true,
                can_authorize_field_recovery: true,
                grants_persistent_org_authority: false,
                granted_at_ic: created_at_ic,
            }));
        }
    }
    for member in roster.members() {
        if let Controller::Npc { character } = member.controller
            && let Some(contract) = crew_pool.contract(character)
            && contract.employer.is_some()
            && contract.mission_scope == NpcMissionScope::DetachmentCommander
        {
            return Ok(Some(MissionLeaderRoleGrant {
                sortie_id,
                leader: character,
                leader_kind: CrewMemberKind::HumanNpc,
                command_slot: member.slot,
                term: roster.term(),
                can_direct_tactical_units: true,
                can_authorize_field_recovery: true,
                grants_persistent_org_authority: false,
                granted_at_ic: created_at_ic,
            }));
        }
    }
    Err(SortieError::NoEligibleCommanderInRoster)
}

/// Errors produced by sortie staging, carrier embarkation manifests, and NPC crew contracts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SortieError {
    /// Object id was `ObjId(0)`.
    InvalidObjectId(&'static str),
    /// Sortie id was `0`.
    InvalidSortieId,
    /// Detachment roster had no members.
    EmptyRoster,
    /// Slot does not exist on the detachment roster.
    UnknownSlot(Slot),
    /// Roster contains neither a player controller nor a contracted NPC with
    /// [`NpcMissionScope::DetachmentCommander`] (`FO-15`, `Q-F1`).
    NoEligibleCommanderInRoster,
    /// Person is not on the carrier's physical embarkation manifest (`FO-16`, `Q-R2`).
    PersonNotOnCarrierManifest {
        /// Carrier vessel id.
        carrier: ObjId,
        /// Missing person id.
        person: ObjId,
    },
    /// Person is already deployed in an active unit and cannot be double-assigned or disembarked.
    PersonAlreadyDeployedInUnit {
        /// Person id.
        person: ObjId,
        /// Unit id in which the person is currently deployed.
        unit: ObjId,
    },
    /// Maximum carrier manifest size reached.
    MaxManifestPersonnelReached {
        /// Maximum personnel count.
        max: usize,
    },
    /// Unknown NPC crew member id.
    UnknownNpcCrew(ObjId),
    /// NPC crew member is already hired by an employer.
    NpcAlreadyHired {
        /// NPC object id.
        npc: ObjId,
        /// Current employer id.
        employer: ObjId,
    },
    /// NPC crew member is not currently hired.
    NpcNotHired(ObjId),
    /// NPC crew member is at a different port from the hiring location.
    NpcNotAtPort {
        /// NPC object id.
        npc: ObjId,
        /// Port where the NPC is located.
        expected_port: ObjId,
        /// Port where hiring was attempted.
        actual_port: ObjId,
    },
    /// NPC has unpaid accrued wages that must be settled before release (`Q-R5`).
    UnpaidWagesOutstanding {
        /// NPC object id.
        npc: ObjId,
        /// Unpaid wage balance in credits.
        unpaid_credits: u64,
    },
    /// NPC operator assigned to command lacks [`NpcMissionScope::DetachmentCommander`] (`FO-15`, `Q-F1`).
    NpcLacksCommanderScope(ObjId),
    /// Maximum crew pool contracts reached.
    MaxCrewContractsReached {
        /// Maximum contracts.
        max: usize,
    },
    /// Invalid slot launch state transition.
    InvalidSlotTransition {
        /// Slot.
        slot: Slot,
        /// Current launch state.
        current: SortieSlotLaunchState,
    },
    /// Underlying [`RosterError`].
    Roster(RosterError),
}

impl fmt::Display for SortieError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidObjectId(field) => write!(f, "{field} object id must be non-zero"),
            Self::InvalidSortieId => f.write_str("sortie id must be non-zero"),
            Self::EmptyRoster => f.write_str("detachment roster has no members"),
            Self::UnknownSlot(slot) => write!(f, "unknown detachment slot {}", slot.get()),
            Self::NoEligibleCommanderInRoster => f.write_str(
                "roster has no player controller or contracted NPC with DetachmentCommander scope",
            ),
            Self::PersonNotOnCarrierManifest { carrier, person } => {
                write!(f, "person {person} is not aboard carrier {carrier}")
            }
            Self::PersonAlreadyDeployedInUnit { person, unit } => {
                write!(f, "person {person} is already deployed in unit {unit}")
            }
            Self::MaxManifestPersonnelReached { max } => {
                write!(f, "carrier manifest limit ({max}) reached")
            }
            Self::UnknownNpcCrew(npc) => write!(f, "unknown NPC crew member {npc}"),
            Self::NpcAlreadyHired { npc, employer } => {
                write!(f, "NPC {npc} is already hired by {employer}")
            }
            Self::NpcNotHired(npc) => write!(f, "NPC {npc} is not currently hired"),
            Self::NpcNotAtPort {
                npc,
                expected_port,
                actual_port,
            } => write!(
                f,
                "NPC {npc} is at port {expected_port}, cannot hire at {actual_port}"
            ),
            Self::UnpaidWagesOutstanding {
                npc,
                unpaid_credits,
            } => write!(
                f,
                "NPC {npc} has {unpaid_credits} unpaid wage credits outstanding"
            ),
            Self::NpcLacksCommanderScope(npc) => {
                write!(f, "NPC {npc} lacks DetachmentCommander mission scope")
            }
            Self::MaxCrewContractsReached { max } => {
                write!(f, "crew pool contract limit ({max}) reached")
            }
            Self::InvalidSlotTransition { slot, current } => write!(
                f,
                "invalid sortie transition on slot {} from {current:?}",
                slot.get()
            ),
            Self::Roster(e) => write!(f, "detachment roster error: {e}"),
        }
    }
}

impl std::error::Error for SortieError {}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::error::Error;

    use mush_core::ObjId;

    use super::*;
    use crate::detachment::LeadershipPolicy;
    use crate::field_ops::condition::OperationalState;
    use crate::field_ops::recovery::FieldPose;

    type TestResult = Result<(), Box<dyn Error>>;

    #[test]
    fn no_player_npc_led_sortie_partial_launch_and_wages() -> TestResult {
        let organizer = ObjId(10);
        let port = ObjId(50);
        let carrier = ObjId(60);
        let site = ObjId(70);
        let npc_commander = ObjId(101);
        let npc_wingman = ObjId(102);
        let unembarked_npc = ObjId(103);
        let unit_a = ObjId(201);
        let unit_b = ObjId(202);
        let unit_c = ObjId(203);

        let mut crew_pool = PortCrewPool::new();
        crew_pool.register_available_npc(
            npc_commander,
            port,
            100,
            NpcMissionScope::DetachmentCommander,
        )?;
        crew_pool.register_available_npc(
            npc_wingman,
            port,
            60,
            NpcMissionScope::SubordinateOperator,
        )?;
        crew_pool.register_available_npc(
            unembarked_npc,
            port,
            60,
            NpcMissionScope::SubordinateOperator,
        )?;

        crew_pool.hire_npc(npc_commander, organizer, port, 1_000)?;
        crew_pool.hire_npc(npc_wingman, organizer, port, 1_000)?;
        crew_pool.hire_npc(unembarked_npc, organizer, port, 1_000)?;

        let mut manifest = CarrierPersonnelManifest::new(carrier)?;
        manifest.embark_person(
            npc_commander,
            CrewMemberKind::HumanNpc,
            EmbarkationOrigin::LastPortDeparture { port },
            1_010,
        )?;
        manifest.embark_person(
            npc_wingman,
            CrewMemberKind::HumanNpc,
            EmbarkationOrigin::LastPortDeparture { port },
            1_010,
        )?;

        let slot1 = Slot::new(1)?;
        let slot2 = Slot::new(2)?;
        let slot3 = Slot::new(3)?;

        let mut roster = DetachmentRoster::new(
            DetachmentId(7),
            1,
            vec![
                Member {
                    slot: slot1,
                    unit: unit_a,
                    controller: Controller::Npc {
                        character: npc_commander,
                    },
                },
                Member {
                    slot: slot2,
                    unit: unit_b,
                    controller: Controller::Npc {
                        character: npc_wingman,
                    },
                },
                Member {
                    slot: slot3,
                    unit: unit_c,
                    controller: Controller::Npc {
                        character: unembarked_npc,
                    },
                },
            ],
            Some(npc_commander),
            LeadershipPolicy::default(),
        )?;

        let mut sortie = ActiveFieldSortie::stage_from_roster(
            1,
            organizer,
            Some(carrier),
            &roster,
            &crew_pool,
            1_020,
        )?;
        let grant = sortie
            .leader_grant()
            .ok_or("expected initial leader grant")?;
        check_eq!(grant.leader, npc_commander);
        check_eq!(grant.leader_kind, CrewMemberKind::HumanNpc);
        check!(!grant.grants_persistent_org_authority);

        let mut crew_book = CrewSurvivalBook::new();
        crew_book.board_unit(npc_commander, unit_a)?;
        crew_book.board_unit(npc_wingman, unit_b)?;

        let s1 = sortie.launch_slot(
            slot1,
            &roster,
            Some(&mut manifest),
            &crew_pool,
            &crew_book,
            1_050,
        )?;
        let s2 = sortie.launch_slot(
            slot2,
            &roster,
            Some(&mut manifest),
            &crew_pool,
            &crew_book,
            1_050,
        )?;
        let s3 = sortie.launch_slot(
            slot3,
            &roster,
            Some(&mut manifest),
            &crew_pool,
            &crew_book,
            1_050,
        )?;

        check_eq!(
            s1,
            SortieSlotLaunchState::Launched {
                launched_at_ic: 1_050
            }
        );
        check_eq!(
            s2,
            SortieSlotLaunchState::Launched {
                launched_at_ic: 1_050
            }
        );
        check_eq!(
            s3,
            SortieSlotLaunchState::LaunchBlocked {
                reason: SortieLaunchBlockReason::OperatorUnavailableOnManifest
            }
        );

        // Partial launch persists (`Q-D6`): slot 3 can be cancelled while slots 1 and 2 stay launched.
        let cancelled = sortie.cancel_unlaunched_slot(slot3, 1_060)?;
        check_eq!(
            cancelled,
            SortieSlotLaunchState::CancelledBeforeLaunch {
                cancelled_at_ic: 1_060
            }
        );
        check_eq!(
            sortie.slot_states().get(&slot1),
            Some(&SortieSlotLaunchState::Launched {
                launched_at_ic: 1_050
            })
        );

        let site_book = FieldSiteEngagementBook::new(site)?;
        let auth_slots = BTreeSet::from([slot1, slot2, slot3]);
        let change = sortie.reconcile_leadership_from_field(
            &mut roster,
            &site_book,
            &crew_book,
            &BTreeMap::new(),
            &auth_slots,
            5_000,
            1_070,
        )?;
        check!(change.is_some());
        check_eq!(
            roster.leader(),
            Some(Leader {
                slot: slot1,
                character: npc_commander,
                kind: LeaderKind::Npc,
            })
        );

        // Two IC days plus 10,000s of outage freeze -> only 2 full days billed (`Q-R5`, `Q-H11`).
        let accrued = crew_pool.settle_npc_wages(1_000 + 2 * IC_DAY_SECONDS + 10_000, 10_000);
        check_eq!(accrued, 2 * (100 + 60 + 60));

        Ok(())
    }

    #[test]
    fn player_command_succession_on_combat_incapacitation_updates_bounded_grant() -> TestResult {
        let organizer = ObjId(10);
        let pilot_a = ObjId(101);
        let pilot_b = ObjId(102);
        let unit_a = ObjId(201);
        let unit_b = ObjId(202);
        let site_id = ObjId(500);

        let slot1 = Slot::new(1)?;
        let slot2 = Slot::new(2)?;

        let mut roster = DetachmentRoster::new(
            DetachmentId(8),
            1,
            vec![
                Member {
                    slot: slot1,
                    unit: unit_a,
                    controller: Controller::Player { character: pilot_a },
                },
                Member {
                    slot: slot2,
                    unit: unit_b,
                    controller: Controller::Player { character: pilot_b },
                },
            ],
            None,
            LeadershipPolicy::default(),
        )?;

        let crew_pool = PortCrewPool::new();
        let mut sortie =
            ActiveFieldSortie::stage_from_roster(2, organizer, None, &roster, &crew_pool, 100)?;

        let mut crew_book = CrewSurvivalBook::new();
        crew_book.board_unit(pilot_a, unit_a)?;
        crew_book.board_unit(pilot_b, unit_b)?;

        sortie.launch_slot(slot1, &roster, None, &crew_pool, &crew_book, 110)?;
        sortie.launch_slot(slot2, &roster, None, &crew_pool, &crew_book, 110)?;

        let site_book = FieldSiteEngagementBook::new(site_id)?;
        let auth_slots = BTreeSet::from([slot1, slot2]);

        sortie.reconcile_leadership_from_field(
            &mut roster,
            &site_book,
            &crew_book,
            &BTreeMap::new(),
            &auth_slots,
            1_000,
            120,
        )?;
        check_eq!(sortie.leader_grant().map(|g| g.leader), Some(pilot_a));

        // Pilot A's unit is wrecked and Pilot A is stranded/injured -> Pilot B succeeds automatically (`Q-F2`).
        crew_book.record_unit_defeat_survival(
            pilot_a,
            unit_a,
            OperationalState::Wreck,
            FieldPose {
                site: site_id,
                x_cm: 50,
                y_cm: 50,
            },
            true,
            true,
            130,
        )?;

        let change = sortie.reconcile_leadership_from_field(
            &mut roster,
            &site_book,
            &crew_book,
            &BTreeMap::new(),
            &auth_slots,
            2_000,
            135,
        )?;
        check!(change.is_some());
        let grant = sortie.leader_grant().ok_or("expected successor grant")?;
        check_eq!(grant.leader, pilot_b);
        check_eq!(grant.command_slot, slot2);
        check!(!grant.grants_persistent_org_authority);

        Ok(())
    }
}

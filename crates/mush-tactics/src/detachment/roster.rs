//! Stable numbered membership and automatic command succession.
//!
//! The host supplies authoritative availability and mission authority. In IC it
//! must resolve real characters, current carrier containment, consent, actual stations,
//! and bounded mission grants; constructing this model grants none of them.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod checkpoint;
pub use checkpoint::RosterCheckpoint;

/// An operation's detachment identity, distinct from a hull's seal epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DetachmentId(pub u64);

/// A one-based command slot. Holes are retained; succession never renumbers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct Slot(u16);

impl Slot {
    /// Construct a nonzero slot. The roster separately checks its capacity.
    pub fn new(value: u16) -> Result<Self, RosterError> {
        if value == 0 {
            return Err(RosterError::InvalidSlot);
        }
        Ok(Self(value))
    }

    /// The player-facing slot number.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for Slot {
    type Error = RosterError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Slot> for u16 {
    fn from(value: Slot) -> Self {
        value.get()
    }
}

/// The assigned decision-maker. An automated mind is never a human principal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Controller {
    Player {
        character: ObjId,
    },
    /// A real character already resolved by the host, never a generated persona.
    Npc {
        character: ObjId,
    },
    Automated,
}

impl Controller {
    #[must_use]
    pub const fn character(self) -> Option<ObjId> {
        match self {
            Self::Player { character } | Self::Npc { character } => Some(character),
            Self::Automated => None,
        }
    }
}

/// One command unit. Future infantry groups must supply their own stable unit id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub slot: Slot,
    pub unit: ObjId,
    pub controller: Controller,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaderKind {
    Player,
    Npc,
}

/// Command identity only: no casualty or disconnection reason is projected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Leader {
    pub slot: Slot,
    pub character: ObjId,
    pub kind: LeaderKind,
}

/// Tunable command policy, independent of practice lobby sizes and timers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeadershipPolicy {
    pub slot_count: u16,
    /// Continuous healthy connection required after becoming unavailable.
    pub return_hysteresis_ms: u64,
}

impl Default for LeadershipPolicy {
    fn default() -> Self {
        Self {
            slot_count: 6,
            return_hysteresis_ms: 3_000,
        }
    }
}

/// Session facts use the host's existing linkdead deadline, not a second grace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    Connected,
    Linkdead { grace_until_ms: u64 },
    Disconnected,
}

/// An authoritative snapshot for every roster slot at one monotonic timestamp.
/// NPCs use `Connected` when their actual decision/control channel is available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemberFacts {
    pub slot: Slot,
    pub capable: bool,
    pub controls_unit: bool,
    pub unit_disabled: bool,
    pub connection: Connection,
    /// Current mission-role admission, not ownership or per-recipient authority.
    /// A missing grant refuses commands; it does not skip a higher-priority player.
    pub command_authorized: bool,
}

impl MemberFacts {
    fn healthy(self) -> bool {
        self.capable && self.controls_unit && !self.unit_disabled
    }

    fn available(self, now_ms: u64) -> bool {
        self.healthy()
            && match self.connection {
                Connection::Connected => true,
                Connection::Linkdead { grace_until_ms } => now_ms < grace_until_ms,
                Connection::Disconnected => false,
            }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct LeadershipChange {
    pub previous: Option<Leader>,
    pub current: Option<Leader>,
    pub term: u64,
}

/// A projection for an audience the host already authorized to see this roster.
/// It deliberately omits health, hidden casualty facts and permission internals.
/// Delivery still uses the host's real communication/observation boundaries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RosterProjection {
    pub detachment: DetachmentId,
    pub revision: u64,
    pub term: u64,
    pub members: Vec<Member>,
    pub leader: Option<Leader>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Availability {
    available: bool,
    grace_until_ms: Option<u64>,
    settling: bool,
    connected_since_ms: Option<u64>,
}

/// Immutable resolved membership plus ephemeral command availability.
///
/// No direct deserializer bypasses construction. A bounded checkpoint retains
/// succession and reconnect timers; restoring it requires fresh availability
/// and authorization before command admission. The host owns its durable commit.
/// A deliberate roster edit creates a new revision and requires the DT.2
/// consent/reservation owner to validate it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetachmentRoster {
    id: DetachmentId,
    revision: u64,
    members: Vec<Member>,
    npc_commander: Option<ObjId>,
    policy: LeadershipPolicy,
    term: u64,
    leader: Option<Leader>,
    last_now_ms: Option<u64>,
    next_reconciliation_at_ms: Option<u64>,
    availability: BTreeMap<Slot, Availability>,
    authority: BTreeMap<Slot, bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum RosterError {
    #[error("slot numbers start at one")]
    InvalidSlot,
    #[error("invalid saved leadership state")]
    InvalidCheckpoint,
    #[error("a detachment needs nonzero slot capacity")]
    InvalidCapacity,
    #[error("slot lies outside the configured command capacity")]
    SlotOutsideCapacity,
    #[error("a command slot is assigned twice")]
    DuplicateSlot,
    #[error("a unit is assigned twice")]
    DuplicateUnit,
    #[error("a character is assigned twice")]
    DuplicateCharacter,
    #[error("the nominated commander is not a real NPC member")]
    InvalidNpcCommander,
    #[error("availability contains duplicate slots")]
    DuplicateFacts,
    #[error("availability is missing a roster slot")]
    MissingFacts,
    #[error("availability names a slot outside this roster")]
    UnknownFacts,
    #[error("command time moved backwards")]
    BackwardTime,
    #[error("availability must be refreshed at its command deadline")]
    RefreshRequired,
    #[error("command term exhausted")]
    ExhaustedTerm,
    #[error("roster revision is stale")]
    StaleRevision,
    #[error("command term is stale")]
    StaleTerm,
    #[error("the issuer is not this detachment's current leader")]
    NotLeader,
    #[error("the leader lacks current mission command admission")]
    Unauthorized,
}

impl DetachmentRoster {
    /// Build a resolved roster. Membership alone never authorizes physical use.
    pub fn new(
        id: DetachmentId,
        revision: u64,
        mut members: Vec<Member>,
        npc_commander: Option<ObjId>,
        policy: LeadershipPolicy,
    ) -> Result<Self, RosterError> {
        if policy.slot_count == 0 {
            return Err(RosterError::InvalidCapacity);
        }
        let mut slots = BTreeSet::new();
        let mut units = BTreeSet::new();
        let mut characters = BTreeSet::new();
        for member in &members {
            if member.slot.get() > policy.slot_count {
                return Err(RosterError::SlotOutsideCapacity);
            }
            if !slots.insert(member.slot) {
                return Err(RosterError::DuplicateSlot);
            }
            if !units.insert(member.unit) {
                return Err(RosterError::DuplicateUnit);
            }
            if let Some(character) = member.controller.character()
                && !characters.insert(character)
            {
                return Err(RosterError::DuplicateCharacter);
            }
        }
        if let Some(commander) = npc_commander
            && !members.iter().any(|member| {
                member.controller
                    == Controller::Npc {
                        character: commander,
                    }
            })
        {
            return Err(RosterError::InvalidNpcCommander);
        }
        members.sort_by_key(|member| member.slot);
        Ok(Self {
            id,
            revision,
            members,
            npc_commander,
            policy,
            term: 0,
            leader: None,
            last_now_ms: None,
            next_reconciliation_at_ms: None,
            availability: BTreeMap::new(),
            authority: BTreeMap::new(),
        })
    }

    #[must_use]
    pub const fn id(&self) -> DetachmentId {
        self.id
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn term(&self) -> u64 {
        self.term
    }

    #[must_use]
    pub const fn leader(&self) -> Option<Leader> {
        self.leader
    }

    /// Timestamp of the authoritative availability snapshot. Commands must not
    /// be admitted or delivered at a time preceding the state they inspect.
    #[must_use]
    pub const fn last_reconciled_at_ms(&self) -> Option<u64> {
        self.last_now_ms
    }

    /// Earliest known grace expiry or return-hysteresis boundary. Hosts schedule
    /// a fresh snapshot here; time-aware admission fails closed at this instant.
    #[must_use]
    pub const fn next_reconciliation_at_ms(&self) -> Option<u64> {
        self.next_reconciliation_at_ms
    }

    #[must_use]
    pub fn members(&self) -> &[Member] {
        &self.members
    }

    #[must_use]
    pub fn member(&self, slot: Slot) -> Option<&Member> {
        self.members.iter().find(|member| member.slot == slot)
    }

    #[must_use]
    pub fn projection(&self) -> RosterProjection {
        RosterProjection {
            detachment: self.id,
            revision: self.revision,
            term: self.term,
            members: self.members.clone(),
            leader: self.leader,
        }
    }

    /// Apply a complete host snapshot atomically. During linkdead grace the
    /// current healthy leader remains eligible; expired grace triggers succession.
    /// A returning slot must stay connected through its hysteresis before it can
    /// retake command. An initial healthy roster takes command immediately.
    /// Hosts call this at grace/hysteresis deadlines and before order admission
    /// or delivery, even when no new session event has arrived.
    pub fn reconcile(
        &mut self,
        now_ms: u64,
        facts: &[MemberFacts],
    ) -> Result<Option<LeadershipChange>, RosterError> {
        if self.last_now_ms.is_some_and(|previous| now_ms < previous) {
            return Err(RosterError::BackwardTime);
        }
        let mut by_slot = BTreeMap::new();
        for fact in facts {
            if self.member(fact.slot).is_none() {
                return Err(RosterError::UnknownFacts);
            }
            if by_slot.insert(fact.slot, *fact).is_some() {
                return Err(RosterError::DuplicateFacts);
            }
        }
        if by_slot.len() != self.members.len() {
            return Err(RosterError::MissingFacts);
        }

        let mut availability = BTreeMap::new();
        let mut authority = BTreeMap::new();
        let mut next_reconciliation_at_ms = None;
        let mut player = None;
        let mut npc = None;
        for member in &self.members {
            let Some(fact) = by_slot.get(&member.slot).copied() else {
                return Err(RosterError::MissingFacts);
            };
            authority.insert(member.slot, fact.command_authorized);
            let prior = self.availability.get(&member.slot);
            let available = fact.available(now_ms);
            let connected = fact.healthy() && fact.connection == Connection::Connected;
            let connected_since_ms = if connected {
                Some(
                    prior
                        .and_then(|prior| prior.connected_since_ms)
                        .unwrap_or(now_ms),
                )
            } else {
                None
            };
            // A delayed snapshot cannot erase the grace boundary we already
            // knew. Without an earlier recorded reconnect, its returning player
            // must settle even if the cached leader still names that character.
            let mut settling = prior.is_some_and(|prior| {
                prior.settling
                    || !prior.available
                    || prior
                        .grace_until_ms
                        .is_some_and(|deadline| now_ms >= deadline)
            });
            if settling
                && connected_since_ms.is_some_and(|since| {
                    now_ms.saturating_sub(since) >= self.policy.return_hysteresis_ms
                })
            {
                settling = false;
            }
            if matches!(member.controller, Controller::Player { .. }) && available {
                let deadline = match fact.connection {
                    Connection::Linkdead { grace_until_ms } => Some(grace_until_ms),
                    Connection::Connected if settling => connected_since_ms
                        .and_then(|since| since.checked_add(self.policy.return_hysteresis_ms)),
                    _ => None,
                };
                if let Some(deadline) = deadline {
                    next_reconciliation_at_ms = Some(
                        next_reconciliation_at_ms
                            .map_or(deadline, |current: u64| current.min(deadline)),
                    );
                }
            }
            availability.insert(
                member.slot,
                Availability {
                    available,
                    grace_until_ms: match fact.connection {
                        Connection::Linkdead { grace_until_ms } => Some(grace_until_ms),
                        _ => None,
                    },
                    settling,
                    connected_since_ms,
                },
            );
            match member.controller {
                Controller::Player { character } if player.is_none() && available && !settling => {
                    player = Some(Leader {
                        slot: member.slot,
                        character,
                        kind: LeaderKind::Player,
                    });
                }
                Controller::Npc { character }
                    if self.npc_commander == Some(character)
                        && connected
                        && fact.command_authorized =>
                {
                    npc = Some(Leader {
                        slot: member.slot,
                        character,
                        kind: LeaderKind::Npc,
                    });
                }
                _ => {}
            }
        }
        let next = player.or(npc);
        let change = if self.leader != next {
            Some(LeadershipChange {
                previous: self.leader,
                current: next,
                term: self.term.checked_add(1).ok_or(RosterError::ExhaustedTerm)?,
            })
        } else {
            None
        };
        self.availability = availability;
        self.authority = authority;
        self.last_now_ms = Some(now_ms);
        self.next_reconciliation_at_ms = next_reconciliation_at_ms;
        if let Some(change) = change {
            self.leader = change.current;
            self.term = change.term;
        }
        Ok(change)
    }

    /// Check identity, concurrency fences and all known availability deadlines.
    /// A missed grace/hysteresis refresh never silently extends a command term.
    /// Physical/capability changes still require fresh host facts before admission.
    pub fn validate_command_at(
        &self,
        issuer: ObjId,
        revision: u64,
        term: u64,
        now_ms: u64,
    ) -> Result<(), RosterError> {
        if self.last_now_ms.is_some_and(|last| now_ms < last) {
            return Err(RosterError::BackwardTime);
        }
        if self
            .next_reconciliation_at_ms
            .is_some_and(|deadline| now_ms >= deadline)
        {
            return Err(RosterError::RefreshRequired);
        }
        self.validate_command(issuer, revision, term)
    }

    /// Check identity and concurrency fences. Per-recipient asset grants,
    /// delivery, station controls and payload safety remain separate checks.
    /// The host must first reconcile availability at the admission timestamp.
    fn validate_command(&self, issuer: ObjId, revision: u64, term: u64) -> Result<(), RosterError> {
        if revision != self.revision {
            return Err(RosterError::StaleRevision);
        }
        if term != self.term {
            return Err(RosterError::StaleTerm);
        }
        let leader = self
            .leader
            .filter(|leader| leader.character == issuer)
            .ok_or(RosterError::NotLeader)?;
        if self.authority.get(&leader.slot) != Some(&true) {
            return Err(RosterError::Unauthorized);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn member(slot: u16, controller: Controller) -> Result<Member, RosterError> {
        Ok(Member {
            slot: Slot::new(slot)?,
            unit: ObjId(100 + u32::from(slot)),
            controller,
        })
    }

    fn roster() -> Result<DetachmentRoster, RosterError> {
        DetachmentRoster::new(
            DetachmentId(7),
            12,
            vec![
                member(
                    6,
                    Controller::Player {
                        character: ObjId(60),
                    },
                )?,
                member(
                    2,
                    Controller::Player {
                        character: ObjId(20),
                    },
                )?,
                member(
                    3,
                    Controller::Npc {
                        character: ObjId(30),
                    },
                )?,
                member(1, Controller::Automated)?,
            ],
            Some(ObjId(30)),
            LeadershipPolicy::default(),
        )
    }

    fn facts(roster: &DetachmentRoster) -> Vec<MemberFacts> {
        roster
            .members()
            .iter()
            .map(|member| MemberFacts {
                slot: member.slot,
                capable: true,
                controls_unit: true,
                unit_disabled: false,
                connection: Connection::Connected,
                command_authorized: true,
            })
            .collect()
    }

    fn change(facts: &mut [MemberFacts], slot: u16, update: impl FnOnce(&mut MemberFacts)) {
        if let Some(fact) = facts.iter_mut().find(|fact| fact.slot.get() == slot) {
            update(fact);
        }
    }

    #[test]
    fn slots_do_not_move_on_successor_npc_or_leaderless_transitions() -> TestResult {
        let mut roster = roster()?;
        let original = roster.members().to_vec();
        let mut facts = facts(&roster);
        roster.reconcile(0, &facts)?;
        check_eq!(
            roster.leader().map(|leader| leader.character),
            Some(ObjId(20))
        );
        check_eq!(roster.term(), 1);
        change(&mut facts, 2, |fact| fact.unit_disabled = true);
        roster.reconcile(1, &facts)?;
        check_eq!(
            roster.leader().map(|leader| leader.character),
            Some(ObjId(60))
        );
        change(&mut facts, 6, |fact| fact.capable = false);
        roster.reconcile(2, &facts)?;
        check_eq!(
            roster.leader().map(|leader| leader.kind),
            Some(LeaderKind::Npc)
        );
        change(&mut facts, 3, |fact| fact.command_authorized = false);
        roster.reconcile(3, &facts)?;
        check_eq!(roster.leader(), None);
        check_eq!(roster.term(), 4);
        check_eq!(roster.revision(), 12);
        check_eq!(roster.members(), original);
        Ok(())
    }

    #[test]
    fn grace_expiry_and_flapping_return_do_not_churn_terms() -> TestResult {
        let mut roster = roster()?;
        let mut facts = facts(&roster);
        roster.reconcile(0, &facts)?;
        change(&mut facts, 2, |fact| {
            fact.connection = Connection::Linkdead {
                grace_until_ms: 300_100,
            }
        });
        roster.reconcile(100, &facts)?;
        roster.reconcile(300_099, &facts)?;
        check_eq!(roster.term(), 1);
        roster.reconcile(300_100, &facts)?;
        check_eq!(
            roster.leader().map(|leader| leader.character),
            Some(ObjId(60))
        );
        change(&mut facts, 2, |fact| {
            fact.connection = Connection::Connected
        });
        roster.reconcile(301_000, &facts)?;
        roster.reconcile(303_999, &facts)?;
        check_eq!(roster.term(), 2);
        change(&mut facts, 2, |fact| {
            fact.connection = Connection::Linkdead {
                grace_until_ms: 604_000,
            }
        });
        roster.reconcile(304_000, &facts)?;
        change(&mut facts, 2, |fact| {
            fact.connection = Connection::Connected
        });
        roster.reconcile(305_000, &facts)?;
        roster.reconcile(307_999, &facts)?;
        check_eq!(roster.term(), 2);
        roster.reconcile(308_000, &facts)?;
        check_eq!(
            roster.leader().map(|leader| leader.character),
            Some(ObjId(20))
        );
        check_eq!(roster.term(), 3);
        Ok(())
    }

    #[test]
    fn returning_while_disabled_does_not_start_hysteresis() -> TestResult {
        let mut roster = roster()?;
        let mut facts = facts(&roster);
        roster.reconcile(0, &facts)?;
        change(&mut facts, 2, |fact| {
            fact.unit_disabled = true;
            fact.connection = Connection::Disconnected;
        });
        roster.reconcile(1, &facts)?;
        change(&mut facts, 2, |fact| {
            fact.connection = Connection::Connected
        });
        roster.reconcile(5_000, &facts)?;
        roster.reconcile(10_000, &facts)?;
        check_eq!(roster.term(), 2);
        change(&mut facts, 2, |fact| fact.unit_disabled = false);
        roster.reconcile(10_001, &facts)?;
        roster.reconcile(13_000, &facts)?;
        check_eq!(roster.term(), 2);
        roster.reconcile(13_001, &facts)?;
        check_eq!(roster.term(), 3);
        Ok(())
    }

    #[test]
    fn missing_authority_refuses_commands_without_skipping_player() -> TestResult {
        let mut roster = roster()?;
        let mut facts = facts(&roster);
        change(&mut facts, 2, |fact| fact.command_authorized = false);
        roster.reconcile(0, &facts)?;
        check_eq!(
            roster.leader().map(|leader| leader.character),
            Some(ObjId(20))
        );
        check_eq!(
            roster.validate_command(ObjId(20), 12, 1),
            Err(RosterError::Unauthorized)
        );
        change(&mut facts, 2, |fact| fact.command_authorized = true);
        roster.reconcile(1, &facts)?;
        roster.validate_command(ObjId(20), 12, 1)?;
        check_eq!(
            roster.validate_command(ObjId(60), 12, 1),
            Err(RosterError::NotLeader)
        );
        check_eq!(
            roster.validate_command(ObjId(20), 11, 1),
            Err(RosterError::StaleRevision)
        );
        check_eq!(
            roster.validate_command(ObjId(20), 12, 0),
            Err(RosterError::StaleTerm)
        );
        Ok(())
    }

    #[test]
    fn missed_grace_and_return_deadlines_fail_closed_until_refreshed() -> TestResult {
        let mut roster = roster()?;
        let mut facts = facts(&roster);
        roster.reconcile(0, &facts)?;
        check_eq!(roster.next_reconciliation_at_ms(), None);
        roster.validate_command_at(ObjId(20), 12, 1, 100)?;
        change(&mut facts, 2, |fact| {
            fact.connection = Connection::Linkdead {
                grace_until_ms: 200,
            }
        });
        roster.reconcile(100, &facts)?;
        check_eq!(roster.next_reconciliation_at_ms(), Some(200));
        roster.validate_command_at(ObjId(20), 12, 1, 199)?;
        let before = roster.clone();
        check_eq!(
            roster.validate_command_at(ObjId(20), 12, 1, 200),
            Err(RosterError::RefreshRequired)
        );
        check_eq!(
            roster.validate_command_at(ObjId(20), 12, 1, 999),
            Err(RosterError::RefreshRequired)
        );
        check_eq!(roster, before);
        roster.reconcile(200, &facts)?;
        roster.validate_command_at(ObjId(60), 12, 2, 200)?;
        change(&mut facts, 2, |fact| {
            fact.connection = Connection::Connected
        });
        roster.reconcile(300, &facts)?;
        check_eq!(roster.next_reconciliation_at_ms(), Some(3_300));
        roster.validate_command_at(ObjId(60), 12, 2, 3_299)?;
        check_eq!(
            roster.validate_command_at(ObjId(60), 12, 2, 3_300),
            Err(RosterError::RefreshRequired)
        );
        roster.reconcile(3_300, &facts)?;
        roster.validate_command_at(ObjId(20), 12, 3, 3_300)?;
        check_eq!(roster.next_reconciliation_at_ms(), None);
        check_eq!(
            roster.validate_command_at(ObjId(20), 12, 3, 3_299),
            Err(RosterError::BackwardTime)
        );
        Ok(())
    }

    #[test]
    fn reconnect_snapshot_after_missed_grace_still_waits_for_hysteresis() -> TestResult {
        for returned_at in [100, 200] {
            let mut roster = roster()?;
            let mut facts = facts(&roster);
            roster.reconcile(0, &facts)?;
            change(&mut facts, 2, |fact| {
                fact.connection = Connection::Linkdead {
                    grace_until_ms: 100,
                };
            });
            roster.reconcile(1, &facts)?;
            // No timer callback at expiry: the next snapshot is the reconnect.
            change(&mut facts, 2, |fact| {
                fact.connection = Connection::Connected
            });
            roster.reconcile(returned_at, &facts)?;
            check_eq!(
                roster.leader().map(|leader| leader.character),
                Some(ObjId(60))
            );
            check_eq!(roster.term(), 2);
            check_eq!(
                roster.next_reconciliation_at_ms(),
                Some(returned_at + 3_000)
            );
            roster.reconcile(returned_at + 2_999, &facts)?;
            check_eq!(roster.term(), 2);
            roster.reconcile(returned_at + 3_000, &facts)?;
            check_eq!(
                roster.leader().map(|leader| leader.character),
                Some(ObjId(20))
            );
            check_eq!(roster.term(), 3);
        }
        Ok(())
    }

    #[test]
    fn recorded_reconnect_before_grace_keeps_command_without_hysteresis() -> TestResult {
        let mut roster = roster()?;
        let mut facts = facts(&roster);
        roster.reconcile(0, &facts)?;
        change(&mut facts, 2, |fact| {
            fact.connection = Connection::Linkdead {
                grace_until_ms: 100,
            };
        });
        roster.reconcile(1, &facts)?;
        change(&mut facts, 2, |fact| {
            fact.connection = Connection::Connected
        });
        roster.reconcile(99, &facts)?;
        roster.reconcile(200, &facts)?;
        check_eq!(roster.term(), 1);
        check_eq!(roster.next_reconciliation_at_ms(), None);
        roster.validate_command_at(ObjId(20), 12, 1, 200)?;
        Ok(())
    }

    #[test]
    fn failed_refresh_preserves_the_earliest_known_deadline() -> TestResult {
        let mut roster = roster()?;
        let mut facts = facts(&roster);
        change(&mut facts, 2, |fact| {
            fact.connection = Connection::Linkdead {
                grace_until_ms: 500,
            }
        });
        change(&mut facts, 6, |fact| {
            fact.connection = Connection::Linkdead {
                grace_until_ms: 300,
            }
        });
        roster.reconcile(100, &facts)?;
        check_eq!(roster.next_reconciliation_at_ms(), Some(300));
        check_eq!(roster.reconcile(300, &[]), Err(RosterError::MissingFacts));
        check_eq!(roster.next_reconciliation_at_ms(), Some(300));
        check_eq!(
            roster.validate_command_at(ObjId(20), 12, 1, 300),
            Err(RosterError::RefreshRequired)
        );
        roster.reconcile(300, &facts)?;
        check_eq!(roster.next_reconciliation_at_ms(), Some(500));
        roster.validate_command_at(ObjId(20), 12, 1, 300)?;
        Ok(())
    }

    #[test]
    fn invalid_snapshots_leave_every_field_unchanged() -> TestResult {
        let mut roster = roster()?;
        let facts = facts(&roster);
        roster.reconcile(10, &facts)?;
        let before = roster.clone();
        check_eq!(roster.reconcile(9, &facts), Err(RosterError::BackwardTime));
        check_eq!(roster, before);
        check_eq!(roster.reconcile(11, &[]), Err(RosterError::MissingFacts));
        check_eq!(roster, before);
        let mut duplicate = facts.clone();
        if let Some(first) = facts.first() {
            duplicate.push(*first);
        }
        check_eq!(
            roster.reconcile(11, &duplicate),
            Err(RosterError::DuplicateFacts)
        );
        check_eq!(roster, before);
        let mut unknown = facts;
        if let Some(first) = unknown.first_mut() {
            first.slot = Slot::new(5)?;
        }
        check_eq!(
            roster.reconcile(11, &unknown),
            Err(RosterError::UnknownFacts)
        );
        check_eq!(roster, before);
        Ok(())
    }

    #[test]
    fn malformed_membership_and_zero_wire_slot_are_refused() -> TestResult {
        let create = |members, npc| {
            DetachmentRoster::new(
                DetachmentId(1),
                1,
                members,
                npc,
                LeadershipPolicy::default(),
            )
        };
        let player = member(
            1,
            Controller::Player {
                character: ObjId(1),
            },
        )?;
        check_eq!(
            create(vec![player, player], None),
            Err(RosterError::DuplicateSlot)
        );
        let mut other = member(2, Controller::Automated)?;
        other.unit = player.unit;
        check_eq!(
            create(vec![player, other], None),
            Err(RosterError::DuplicateUnit)
        );
        other.unit = ObjId(102);
        other.controller = Controller::Npc {
            character: ObjId(1),
        };
        check_eq!(
            create(vec![player, other], None),
            Err(RosterError::DuplicateCharacter)
        );
        check_eq!(
            create(vec![player], Some(ObjId(1))),
            Err(RosterError::InvalidNpcCommander)
        );
        check_eq!(
            create(vec![member(7, Controller::Automated)?], None),
            Err(RosterError::SlotOutsideCapacity)
        );
        check!(serde_json::from_str::<Slot>("0").is_err());
        check_eq!(serde_json::from_str::<Slot>("6")?, Slot::new(6)?);
        Ok(())
    }

    #[test]
    fn projection_contains_no_unavailability_reason() -> TestResult {
        let mut roster = roster()?;
        let mut facts = facts(&roster);
        roster.reconcile(0, &facts)?;
        change(&mut facts, 2, |fact| fact.capable = false);
        roster.reconcile(1, &facts)?;
        let projection = serde_json::to_string(&roster.projection())?;
        for hidden in [
            "capable",
            "disabled",
            "connection",
            "command_authorized",
            "hysteresis",
        ] {
            check!(!projection.contains(hidden));
        }
        Ok(())
    }
}

//! A bounded restart record, deliberately excluding cached authorization.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RosterCheckpoint {
    schema: u16,
    id: DetachmentId,
    revision: u64,
    members: Vec<Member>,
    npc_commander: Option<ObjId>,
    policy: LeadershipPolicy,
    term: u64,
    leader: Option<Leader>,
    last_now_ms: Option<u64>,
    availability: BTreeMap<Slot, Availability>,
}

impl RosterCheckpoint {
    #[must_use]
    pub fn matches_membership(&self, roster: &DetachmentRoster) -> bool {
        self.id == roster.id
            && self.revision == roster.revision
            && self.members == roster.members
            && self.npc_commander == roster.npc_commander
            && self.policy == roster.policy
    }

    /// Advancing the observation clock alone does not require a durable write.
    #[must_use]
    pub fn same_succession(&self, other: &Self) -> bool {
        self.schema == other.schema
            && self.id == other.id
            && self.revision == other.revision
            && self.members == other.members
            && self.npc_commander == other.npc_commander
            && self.policy == other.policy
            && self.term == other.term
            && self.leader == other.leader
            && self.availability == other.availability
    }
}

impl DetachmentRoster {
    /// Save succession identity and reconnect timers, never a grant or lease.
    #[must_use]
    pub fn checkpoint(&self) -> RosterCheckpoint {
        RosterCheckpoint {
            schema: 1,
            id: self.id,
            revision: self.revision,
            members: self.members.clone(),
            npc_commander: self.npc_commander,
            policy: self.policy,
            term: self.term,
            leader: self.leader,
            last_now_ms: self.last_now_ms,
            availability: self.availability.clone(),
        }
    }

    /// Restore only validated membership and history. Command admission remains
    /// closed until the host reconciles a complete set of current owner facts.
    pub fn restore(saved: &RosterCheckpoint) -> Result<Self, RosterError> {
        if saved.schema != 1 || saved.members.len() > 6 || saved.policy.slot_count > 6 {
            return Err(RosterError::InvalidCheckpoint);
        }
        let mut roster = Self::new(
            saved.id,
            saved.revision,
            saved.members.clone(),
            saved.npc_commander,
            saved.policy,
        )?;
        let valid_leader = saved.leader.is_none_or(|leader| {
            roster
                .member(leader.slot)
                .is_some_and(|member| match member.controller {
                    Controller::Player { character } => {
                        leader.kind == LeaderKind::Player && character == leader.character
                    }
                    Controller::Npc { character } => {
                        leader.kind == LeaderKind::Npc
                            && character == leader.character
                            && saved.npc_commander == Some(character)
                    }
                    Controller::Automated => false,
                })
        });
        if !valid_leader
            || saved.leader.is_some() && saved.term == 0
            || match saved.last_now_ms {
                None => !saved.availability.is_empty() || saved.term != 0 || saved.leader.is_some(),
                Some(now) => {
                    saved.availability.len() != saved.members.len()
                        || saved.availability.iter().any(|(slot, a)| {
                            roster.member(*slot).is_none()
                                || a.connected_since_ms.is_some_and(|since| since > now)
                                || a.connected_since_ms.is_some() && a.grace_until_ms.is_some()
                        })
                }
            }
        {
            return Err(RosterError::InvalidCheckpoint);
        }
        roster.term = saved.term;
        roster.leader = saved.leader;
        roster.last_now_ms = saved.last_now_ms;
        roster.availability = saved.availability.clone();
        // Even a saved future deadline cannot extend stale authority on restart.
        roster.next_reconciliation_at_ms = saved.last_now_ms;
        Ok(roster)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_retains_term_and_hysteresis_but_requires_live_authority()
    -> Result<(), Box<dyn std::error::Error>> {
        let members = vec![
            Member {
                slot: Slot::new(1)?,
                unit: ObjId(11),
                controller: Controller::Player {
                    character: ObjId(1),
                },
            },
            Member {
                slot: Slot::new(2)?,
                unit: ObjId(12),
                controller: Controller::Player {
                    character: ObjId(2),
                },
            },
        ];
        let mut roster = DetachmentRoster::new(
            DetachmentId(1),
            1,
            members,
            None,
            LeadershipPolicy::default(),
        )?;
        let mut facts = vec![
            MemberFacts {
                slot: Slot::new(1)?,
                capable: true,
                controls_unit: true,
                unit_disabled: false,
                connection: Connection::Connected,
                command_authorized: true,
            },
            MemberFacts {
                slot: Slot::new(2)?,
                capable: true,
                controls_unit: true,
                unit_disabled: false,
                connection: Connection::Connected,
                command_authorized: true,
            },
        ];
        roster.reconcile(0, &facts)?;
        facts[0].connection = Connection::Disconnected;
        roster.reconcile(100, &facts)?;
        facts[0].connection = Connection::Connected;
        roster.reconcile(200, &facts)?;
        let saved: RosterCheckpoint =
            serde_json::from_value(serde_json::to_value(roster.checkpoint())?)?;
        let mut restored = DetachmentRoster::restore(&saved)?;
        check_eq!(restored.projection(), roster.projection());
        check_eq!(
            restored.validate_command_at(ObjId(2), 1, restored.term(), 201),
            Err(RosterError::RefreshRequired)
        );
        restored.reconcile(3199, &facts)?;
        check_eq!(restored.leader().map(|l| l.character), Some(ObjId(2)));
        restored.reconcile(3200, &facts)?;
        check_eq!(restored.leader().map(|l| l.character), Some(ObjId(1)));
        check_eq!(restored.term(), roster.term() + 1);
        facts[0].command_authorized = false;
        restored.reconcile(3201, &facts)?;
        check_eq!(
            restored.validate_command_at(ObjId(1), 1, restored.term(), 3201),
            Err(RosterError::Unauthorized)
        );
        Ok(())
    }

    #[test]
    fn malformed_checkpoint_cannot_bypass_roster_construction()
    -> Result<(), Box<dyn std::error::Error>> {
        let roster = DetachmentRoster::new(
            DetachmentId(1),
            1,
            vec![Member {
                slot: Slot::new(1)?,
                unit: ObjId(11),
                controller: Controller::Player {
                    character: ObjId(1),
                },
            }],
            None,
            LeadershipPolicy::default(),
        )?;
        let mut saved = roster.checkpoint();
        saved.members.push(saved.members[0]);
        check!(DetachmentRoster::restore(&saved).is_err());
        saved.members.pop();
        saved.leader = Some(Leader {
            slot: Slot::new(1)?,
            character: ObjId(2),
            kind: LeaderKind::Player,
        });
        saved.term = 1;
        check!(DetachmentRoster::restore(&saved).is_err());
        Ok(())
    }
}

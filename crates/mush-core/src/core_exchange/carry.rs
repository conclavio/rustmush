//! Borrowed fixed-exchange attribution for real carry hooks. No decoder or
//! public constructor can turn caller parameters into this context.
use super::*;

#[derive(Debug)]
pub struct ExchangeCarryAttempt {
    facts: ExchangeFacts,
    action: DefinitionRef,
    commit: CommitToken,
    participants: [DefinitionRef; 2],
}
impl ExchangeCarryAttempt {
    pub(super) fn capture(
        facts: &ExchangeFacts,
        action: &DefinitionRef,
        commit: CommitToken,
        participants: &[DefinitionRef; 2],
    ) -> Result<Self, ParticipantError> {
        facts.validate()?;
        exact_action(action)?;
        check_participants(participants)?;
        Ok(Self {
            facts: facts.clone(),
            action: action.clone(),
            commit,
            participants: participants.clone(),
        })
    }
    pub(crate) fn require_move(
        &self,
        actor: ObjId,
        object: ObjId,
        destination: ObjId,
    ) -> Result<(), ParticipantError> {
        // Repeated on each real hook evaluation; the captured identity is never
        // inferred from location, a pending row alone, or caller parameters.
        exact_action(&self.action)?;
        check_participants(&self.participants)?;
        self.facts.validate()?;
        let incoming = object == self.facts.incoming() && destination == self.facts.hull();
        let outgoing = self
            .facts
            .displaced()
            .is_some_and(|(core, holder)| object == core && destination == holder);
        if actor != self.facts.requester() || !(incoming || outgoing) {
            return Err(refused());
        }
        Ok(())
    }
    pub const fn order(&self) -> u64 {
        self.facts.order()
    }
    pub const fn requester(&self) -> ObjId {
        self.facts.requester()
    }
    pub const fn hull(&self) -> ObjId {
        self.facts.hull()
    }
    pub const fn unit_revision(&self) -> u64 {
        self.facts.unit_revision()
    }
    pub const fn incoming(&self) -> ObjId {
        self.facts.incoming()
    }
    pub const fn source(&self) -> ObjId {
        self.facts.source()
    }
    pub const fn displaced(&self) -> Option<(ObjId, ObjId)> {
        self.facts.displaced()
    }
    pub const fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub const fn commit(&self) -> CommitToken {
        self.commit
    }
    pub const fn participants(&self) -> &[DefinitionRef; 2] {
        &self.participants
    }
}

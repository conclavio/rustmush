use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum OutputState {
    Reserved {},
    Consumed {
        completion: CommitToken,
        action: DefinitionRef,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OutputRecord {
    pub(crate) format: u16,
    pub(crate) birth: u64,
    pub(crate) purpose: String,
    pub(crate) order: CoreOutputOrder,
    pub(crate) object: ObjId,
    pub(crate) created_at: i64,
    pub(crate) object_digest: DefinitionDigest,
    pub(crate) reservation: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) state: OutputState,
}
impl OutputRecord {
    pub(crate) fn birth_object(&self) -> Obj {
        let mut object = Obj::new(self.object, ObjKind::Thing, NAME, GOD, self.created_at);
        object.location = Some(self.order.container());
        object
    }
    pub(crate) fn validate(&self) -> Result<(), ParticipantError> {
        self.order
            .validate()
            .map_err(|_| ParticipantError::Authority)?;
        if self.format != FORMAT
            || self.birth == 0
            || self.purpose != PURPOSE
            || self.object.0 < 2
            || self.object == self.order.container()
            || self.object == self.order.facility()
            || !fixed_action(&self.action, RESERVE_ACTION)
            || object_digest(&self.birth_object())? != self.object_digest
        {
            return Err(ParticipantError::Authority);
        }
        if let OutputState::Consumed { completion, action } = &self.state
            && (*completion == self.reservation || !fixed_action(action, COMPLETE_ACTION))
        {
            return Err(ParticipantError::Authority);
        }
        bounded(self)?;
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OutputDeployment {
    pub(crate) format: u16,
    pub(crate) initialized_by: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) born: u64,
    pub(crate) consumed: u64,
}
impl OutputDeployment {
    pub(crate) fn validate(&self) -> Result<(), ParticipantError> {
        if self.format != FORMAT
            || !fixed_action(&self.action, INITIALIZE_ACTION)
            || self.consumed > self.born
            || self.born > crate::action_state::MAX_ACTION_STATE_BOOT_RECORDS as u64
        {
            return Err(ParticipantError::Authority);
        }
        bounded(self)?;
        Ok(())
    }
}

/// Read-only immutable reservation facts. Deserialization is intentionally
/// unavailable: only an actual acknowledged row and World image produce this.
#[derive(Debug)]
pub struct ReservedCoreOutput {
    pub(crate) row: OutputRecord,
    pub(crate) preparation: CommitToken,
    pub(crate) prepared_action: DefinitionRef,
}
impl ReservedCoreOutput {
    pub fn order(&self) -> &CoreOutputOrder {
        &self.row.order
    }
    pub const fn object(&self) -> ObjId {
        self.row.object
    }
    pub const fn reservation(&self) -> CommitToken {
        self.row.reservation
    }
    pub const fn object_digest(&self) -> DefinitionDigest {
        self.row.object_digest
    }
    pub fn reservation_action(&self) -> &DefinitionRef {
        &self.row.action
    }
    pub const fn commit(&self) -> CommitToken {
        self.preparation
    }
    pub fn action(&self) -> &DefinitionRef {
        &self.prepared_action
    }
}
/// Actual immutable retained history, emitted by the complete Core audit. The
/// Industry owner must reverse-join every record to its accepted work order and
/// every consumption to its actual certificate/title completion in one commit.
#[derive(Clone, Debug)]
pub struct CoreOutputHistory {
    pub(crate) row: OutputRecord,
}
impl CoreOutputHistory {
    pub fn order(&self) -> &CoreOutputOrder {
        &self.row.order
    }
    pub const fn object(&self) -> ObjId {
        self.row.object
    }
    pub const fn reservation(&self) -> CommitToken {
        self.row.reservation
    }
    pub const fn object_digest(&self) -> DefinitionDigest {
        self.row.object_digest
    }
    pub fn action(&self) -> &DefinitionRef {
        &self.row.action
    }
    pub fn completion(&self) -> Option<(CommitToken, &DefinitionRef)> {
        match &self.row.state {
            OutputState::Reserved {} => None,
            OutputState::Consumed { completion, action } => Some((*completion, action)),
        }
    }
}

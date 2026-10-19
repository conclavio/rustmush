//! Immutable compatible-line publication and its exact sequence index.
use crate::commerce::rating::RatingRef;
use crate::commerce::records::{FiledRecord, InitialFilingOperation, PublicationError, Record};
use crate::commerce::{
    CharterId, FiledRevisionId, LicenceId, LineId, RevisionIdentity, TermGeneration,
};
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, ObjId, mechanics::DefinitionRef};
use mush_economy::registry_filing::UpdateAuditSubject;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "OperationWire")]
pub(crate) struct UpdateOperation {
    pub(crate) format: u16,
    pub(crate) commit: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) actor: ObjId,
    pub(crate) initial: CommitToken,
    pub(crate) charter: CharterId,
    pub(crate) line: LineId,
    pub(crate) licence: LicenceId,
    pub(crate) term: TermGeneration,
    pub(crate) previous: FiledRevisionId,
    pub(crate) filed: FiledRevisionId,
    pub(crate) sequence: u64,
    pub(crate) revision: RevisionIdentity,
    pub(crate) rating: RatingRef,
    pub(crate) schema: DefinitionDigest,
    pub(crate) duplicate: DefinitionDigest,
    pub(crate) at_ic: ClockStamp,
    pub(crate) economic: UpdateAuditSubject,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationWire {
    format: u16,
    commit: CommitToken,
    action: DefinitionRef,
    actor: ObjId,
    initial: CommitToken,
    charter: CharterId,
    line: LineId,
    licence: LicenceId,
    term: TermGeneration,
    previous: FiledRevisionId,
    filed: FiledRevisionId,
    sequence: u64,
    revision: RevisionIdentity,
    rating: RatingRef,
    schema: DefinitionDigest,
    duplicate: DefinitionDigest,
    at_ic: ClockStamp,
    economic: UpdateAuditSubject,
}
impl TryFrom<OperationWire> for UpdateOperation {
    type Error = PublicationError;
    fn try_from(w: OperationWire) -> Result<Self, Self::Error> {
        let row = Self {
            format: w.format,
            commit: w.commit,
            action: w.action,
            actor: w.actor,
            initial: w.initial,
            charter: w.charter,
            line: w.line,
            licence: w.licence,
            term: w.term,
            previous: w.previous,
            filed: w.filed,
            sequence: w.sequence,
            revision: w.revision,
            rating: w.rating,
            schema: w.schema,
            duplicate: w.duplicate,
            at_ic: w.at_ic,
            economic: w.economic,
        };
        row.validate()?;
        Ok(row)
    }
}
impl Record for UpdateOperation {
    const FAMILY: &'static str = "roll_mind_line_update";
    const MAX_BYTES: usize = 8 * 1024;
    fn key(&self) -> String {
        self.commit.to_string()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.economic
            .validate()
            .map_err(|_| PublicationError::Invalid)?;
        super::check_action(&self.action)?;
        crate::commerce::WorldIcSeconds::from_stamp(&self.at_ic)
            .map_err(|_| PublicationError::Invalid)?;
        if self.format != 1
            || self.actor.0 == 0
            || self.commit == self.initial
            || self.previous == self.filed
            || !(2..=crate::MAX_RECORDS as u64 + 1).contains(&self.sequence)
            || self.revision.revision().key.character != self.actor
            || self.economic.commit != self.commit
            || self.economic.actor != self.actor
            || self.economic.charter != self.charter.object()
        {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}
impl UpdateOperation {
    pub(crate) fn check_birth(
        &self,
        birth: &InitialFilingOperation,
    ) -> Result<(), PublicationError> {
        self.validate()?;
        birth.validate()?;
        if birth.format != 1
            || birth.parent.is_some()
            || birth.economic.house.is_some()
            || birth.commit != self.initial
            || birth.requester != self.actor
            || birth.charter != self.charter
            || birth.line != self.line
            || birth.licence != self.licence
            || birth.term != self.term
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    pub(crate) fn check_filed(&self, filed: &FiledRecord) -> Result<(), PublicationError> {
        self.validate()?;
        filed.validate()?;
        let p = filed.content.position();
        if filed.format != 3
            || filed.operation != self.commit
            || filed.content.id() != self.filed
            || filed.content.parent().is_some()
            || p.charter != self.charter
            || p.line != self.line
            || p.sequence.get() != self.sequence
            || filed.content.revision() != self.revision
            || filed.rating != self.rating
            || filed.duplicate != self.duplicate
            || filed.content.filed_at()
                != crate::commerce::WorldIcSeconds::from_stamp(&self.at_ic)
                    .map_err(|_| PublicationError::Invalid)?
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PositionWire")]
pub(crate) struct UpdatePosition {
    pub(crate) format: u16,
    pub(crate) line: LineId,
    pub(crate) sequence: u64,
    pub(crate) filed: FiledRevisionId,
    pub(crate) operation: CommitToken,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PositionWire {
    format: u16,
    line: LineId,
    sequence: u64,
    filed: FiledRevisionId,
    operation: CommitToken,
}
impl TryFrom<PositionWire> for UpdatePosition {
    type Error = PublicationError;
    fn try_from(w: PositionWire) -> Result<Self, Self::Error> {
        let row = Self {
            format: w.format,
            line: w.line,
            sequence: w.sequence,
            filed: w.filed,
            operation: w.operation,
        };
        row.validate()?;
        Ok(row)
    }
}
impl UpdatePosition {
    pub(crate) fn key_for(line: LineId, sequence: u64) -> String {
        format!("line/{:016x}/{sequence:016x}", line.get())
    }
    pub(crate) fn check(&self, op: &UpdateOperation) -> Result<(), PublicationError> {
        self.validate()?;
        if self.line != op.line
            || self.sequence != op.sequence
            || self.filed != op.filed
            || self.operation != op.commit
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}
impl Record for UpdatePosition {
    const FAMILY: &'static str = "roll_mind_line_update_position";
    const MAX_BYTES: usize = 1024;
    fn key(&self) -> String {
        Self::key_for(self.line, self.sequence)
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if self.format != 1 || !(2..=crate::MAX_RECORDS as u64 + 1).contains(&self.sequence) {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}

use super::*;
use crate::core_output::records::{OutputRecord, OutputState};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeliveryRecord {
    pub(crate) format: u16,
    pub(crate) operation: crate::CommitToken,
    pub(crate) action: crate::DefinitionRef,
    pub(crate) output: OutputRecord,
    pub(crate) at_utc: i64,
    pub(crate) delivered_digest: crate::DefinitionDigest,
}
impl DeliveryRecord {
    pub(crate) fn delivered_object(&self) -> Obj {
        let mut object = self.output.birth_object();
        object.location = Some(self.output.order.requester());
        object.modified = self.at_utc;
        object
    }
    pub(crate) fn validate(&self) -> Result<(), ParticipantError> {
        self.output.validate()?;
        let OutputState::Consumed { completion, .. } = self.output.state else {
            return Err(ParticipantError::Authority);
        };
        if self.format != 1
            || !crate::core_output::fixed_action(&self.action, ACTION)
            || self.operation == self.output.reservation
            || self.operation == completion
            || crate::core_output::object_digest(&self.delivered_object())? != self.delivered_digest
        {
            return Err(ParticipantError::Authority);
        }
        bounded(self)?;
        Ok(())
    }
}
/// Actual prepared delivery identity, only minted by the fixed typed service.
/// The Industry owner retains this exact receipt in its own acknowledged index.
#[derive(Debug)]
pub struct DeliveredCoreOutput {
    row: DeliveryRecord,
    completion: crate::CommitToken,
}
impl DeliveredCoreOutput {
    pub(super) fn from_record(row: DeliveryRecord) -> Result<Self, ParticipantError> {
        row.validate()?;
        let OutputState::Consumed { completion, .. } = row.output.state else {
            return Err(ParticipantError::Authority);
        };
        Ok(Self { row, completion })
    }
    pub fn order(&self) -> &crate::core_output::CoreOutputOrder {
        &self.row.output.order
    }
    pub const fn object(&self) -> ObjId {
        self.row.output.object
    }
    pub const fn reservation(&self) -> crate::CommitToken {
        self.row.output.reservation
    }
    pub const fn completion(&self) -> crate::CommitToken {
        self.completion
    }
    pub const fn commit(&self) -> crate::CommitToken {
        self.row.operation
    }
    pub fn action(&self) -> &crate::DefinitionRef {
        &self.row.action
    }
    pub const fn delivered_at(&self) -> i64 {
        self.row.at_utc
    }
    pub const fn object_digest(&self) -> crate::DefinitionDigest {
        self.row.delivered_digest
    }
    pub fn digest(&self) -> Result<crate::DefinitionDigest, ParticipantError> {
        self.row.validate()?;
        crate::DefinitionDigest::for_canonical_value(
            "industry.core_delivery.receipt.v1",
            &bounded(&self.row)?,
        )
        .map_err(|_| ParticipantError::Encoding)
    }
}
/// Same closed identity obtained from a complete current/historical audit.
#[derive(Debug)]
pub struct CoreDeliveryHistory {
    pub(crate) receipt: DeliveredCoreOutput,
}
impl CoreDeliveryHistory {
    pub fn receipt(&self) -> &DeliveredCoreOutput {
        &self.receipt
    }
}

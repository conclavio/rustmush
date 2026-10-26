//! Closed permanent exact-filed cause. Historical witnesses never grant authority.
use super::super::{
    RevisionIdentity,
    rating::RatingRef,
    records::{FiledRecord, InitialFilingOperation, RevisionStatus},
};
use super::*;
use mush_core::{ClockStamp, DefinitionDigest};
use mush_org::SystemPrincipalId;
use serde::Deserializer;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Reason {
    RegistryReview,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "OperationWire")]
pub(crate) struct RecallOperation {
    pub(crate) format: u16,
    pub(crate) operation: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) authority: RegistryRecallPolicy,
    pub(crate) actor: ObjId,
    pub(crate) authorization: authority::Authorization,
    pub(crate) filed: FiledRevisionId,
    pub(crate) revision: RevisionIdentity,
    pub(crate) rating: RatingRef,
    pub(crate) initial_operation: CommitToken,
    pub(crate) initial_issuer: SystemPrincipalId,
    pub(crate) withdrawn_by: Option<CommitToken>,
    pub(crate) reason: Reason,
    pub(crate) at_ic: ClockStamp,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationWire {
    format: u16,
    operation: CommitToken,
    action: DefinitionRef,
    authority: RegistryRecallPolicy,
    actor: ObjId,
    authorization: authority::Authorization,
    filed: FiledRevisionId,
    revision: RevisionIdentity,
    rating: RatingRef,
    initial_operation: CommitToken,
    initial_issuer: SystemPrincipalId,
    #[serde(deserialize_with = "required_option")]
    withdrawn_by: Option<CommitToken>,
    reason: Reason,
    at_ic: ClockStamp,
}
fn required_option<'de, D: Deserializer<'de>>(d: D) -> Result<Option<CommitToken>, D::Error> {
    Option::<CommitToken>::deserialize(d)
}
impl TryFrom<OperationWire> for RecallOperation {
    type Error = PublicationError;
    fn try_from(w: OperationWire) -> Result<Self, Self::Error> {
        let op = Self {
            format: w.format,
            operation: w.operation,
            action: w.action,
            authority: w.authority,
            actor: w.actor,
            authorization: w.authorization,
            filed: w.filed,
            revision: w.revision,
            rating: w.rating,
            initial_operation: w.initial_operation,
            initial_issuer: w.initial_issuer,
            withdrawn_by: w.withdrawn_by,
            reason: w.reason,
            at_ic: w.at_ic,
        };
        op.validate()?;
        Ok(op)
    }
}
impl Record for RecallOperation {
    const FAMILY: &'static str = "roll_registry_revision_recalls";
    const MAX_BYTES: usize = 16384;
    fn key(&self) -> String {
        self.operation.to_hex()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.authority.validate()?;
        self.authorization.validate()?;
        super::super::WorldIcSeconds::from_stamp(&self.at_ic).map_err(bad)?;
        if self.format != 1
            || self.action != self.authority.action().map_err(bad)?
            || self.authority.operators.binary_search(&self.actor).is_err()
            || self.initial_issuer != self.authority.issuer
            || self.operation == self.initial_operation
            || self.withdrawn_by == Some(self.operation)
        {
            return Err(PublicationError::Invalid);
        }
        // Include the actual typed family envelope, not just its nested value.
        let envelope = json!({"family":{"owner":crate::NS,"id":Self::FAMILY},
            "schema":{"id":format!("{}.value.v1",Self::FAMILY),"version":1},
            "key":self.key(),"value":self});
        mush_core::canonical_json_bytes_with_limit(&envelope, Self::MAX_BYTES)
            .map_err(|_| PublicationError::Capacity)?;
        Ok(())
    }
}
impl RecallOperation {
    pub(crate) fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        DefinitionDigest::for_canonical_value(
            "roll.registry_revision_recall.cause.v1",
            &serde_json::to_value(self).map_err(bad)?,
        )
        .map_err(bad)
    }
    pub(crate) fn root(&self) -> Result<RequiredRoot, PublicationError> {
        Ok(RequiredRoot {
            anchor: RootAnchor::RegistryRecall(self.operation, self.digest()?),
            identity: self.revision,
        })
    }
    pub(crate) fn status(&self) -> RevisionStatus {
        RevisionStatus {
            format: 1,
            filed: self.filed,
            withdrawn_by: self.withdrawn_by,
            recalled_by: Some(self.operation),
        }
    }
    pub(crate) fn matches(
        &self,
        initial: &InitialFilingOperation,
        filed: &FiledRecord,
    ) -> Result<(), PublicationError> {
        self.validate()?;
        initial.validate()?;
        filed.validate()?;
        if initial.commit != self.initial_operation
            || initial.economic.issuer != self.initial_issuer
            || filed.content.id() != self.filed
            || filed.content.revision() != self.revision
            || filed.rating != self.rating
            || filed.content.position().charter != initial.charter
            || filed.content.position().line != initial.line
            || (filed.format != 3
                && (filed.operation != initial.commit
                    || initial.filed != self.filed
                    || initial.revision != self.revision))
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}

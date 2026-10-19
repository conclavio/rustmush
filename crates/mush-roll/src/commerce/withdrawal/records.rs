//! Durable cause is strict data; only the private producer can install it.
use super::super::{LineId, RevisionIdentity};
use super::*;
use mush_core::DefinitionDigest;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WithdrawalOperation {
    pub(crate) format: u16,
    pub(crate) operation: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) author: ObjId,
    pub(crate) initial_filing: CommitToken,
    pub(crate) filed: FiledRevisionId,
    pub(crate) charter: CharterId,
    pub(crate) line: LineId,
    pub(crate) revision: RevisionIdentity,
    pub(crate) title_revision: u64,
}
impl Record for WithdrawalOperation {
    const FAMILY: &'static str = "roll_mind_author_withdrawal";
    const MAX_BYTES: usize = 8192;
    fn key(&self) -> String {
        self.operation.to_hex()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if self.format != 1
            || self.action != action_reference().map_err(bad)?
            || self.operation == self.initial_filing
            || self.title_revision == 0
            || self.author != self.revision.revision().key.character
        {
            return Err(PublicationError::Invalid);
        }
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(self).map_err(bad)?,
            Self::MAX_BYTES,
        )
        .map_err(|_| PublicationError::Capacity)?;
        Ok(())
    }
}
impl WithdrawalOperation {
    pub(crate) fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        DefinitionDigest::for_canonical_value(
            "roll.personal_author_withdrawal.cause.v1",
            &serde_json::to_value(self).map_err(bad)?,
        )
        .map_err(bad)
    }
    pub(crate) fn root(&self) -> Result<RequiredRoot, PublicationError> {
        Ok(RequiredRoot {
            anchor: RootAnchor::AuthorWithdrawal(self.operation, self.digest()?),
            identity: self.revision,
        })
    }
    pub(crate) fn status(&self) -> super::super::records::RevisionStatus {
        super::super::records::RevisionStatus {
            format: 1,
            filed: self.filed,
            withdrawn_by: Some(self.operation),
            recalled_by: None,
        }
    }
    pub(crate) fn matches_initial(
        &self,
        initial: &super::super::records::InitialFilingOperation,
        filed: &super::super::records::FiledRecord,
    ) -> Result<(), PublicationError> {
        self.validate()?;
        initial.validate()?;
        filed.validate()?;
        if initial.commit != self.initial_filing
            || (filed.format != 3 && filed.operation != self.initial_filing)
            || initial.requester != self.author
            || initial.economic.house.is_some()
            || initial.economic.issued_owner != mush_org::PrincipalId::Object(self.author)
            || (filed.format != 3
                && (initial.revision != self.revision || initial.filed != self.filed))
            || filed.content.id() != self.filed
            || filed.content.revision() != self.revision
            || initial.charter != self.charter
            || filed.content.position().charter != self.charter
            || initial.line != self.line
            || filed.content.position().line != self.line
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}

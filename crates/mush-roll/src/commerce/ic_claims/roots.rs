//! Actual four-owner publication alone constructs the permanent claim root.
use super::super::{
    records::{PublicationError, Record},
    roots::{RequiredRoot, RootAnchor},
};
use super::{ClaimRecord, IcAdmissionFacts};
use mush_core::{CommitToken, DefinitionRef, participant::OwnerPrepareContext};

#[derive(Clone)]
pub(crate) struct IcFence {
    action: DefinitionRef,
    commit: CommitToken,
    fresh: Option<IcAdmissionFacts>,
}
impl IcFence {
    pub(crate) fn check(&self, ctx: &OwnerPrepareContext<'_>) -> Result<(), PublicationError> {
        if ctx.action() != &self.action || ctx.commit_token() != self.commit {
            return Err(PublicationError::Mismatch);
        }
        if let Some(facts) = &self.fresh {
            facts.check_fresh(ctx)?;
        }
        Ok(())
    }
}
pub(crate) struct IcRoots {
    row: ClaimRecord,
    before: Option<ClaimRecord>,
    fence: IcFence,
}
impl IcRoots {
    pub(super) fn new(
        row: ClaimRecord,
        before: Option<ClaimRecord>,
        fresh: Option<IcAdmissionFacts>,
    ) -> Result<Self, PublicationError> {
        row.validate()?;
        let (action, commit) = match (&before, &row.closed, &fresh) {
            (None, None, Some(f)) if row.open.admission_digest()? == f.binding_digest()? => {
                (row.open.action.clone(), row.open.operation)
            }
            (Some(old), Some(c), None) if old.closed.is_none() && old.open == row.open => {
                (c.action.clone(), c.operation)
            }
            _ => return Err(PublicationError::Mismatch),
        };
        Ok(Self {
            row,
            before,
            fence: IcFence {
                action,
                commit,
                fresh,
            },
        })
    }
    pub(crate) fn commit(&self) -> CommitToken {
        self.fence.commit
    }
    pub(crate) fn fence(&self) -> &IcFence {
        &self.fence
    }
    pub(crate) fn row(&self) -> &ClaimRecord {
        &self.row
    }
    pub(crate) fn before(&self) -> Option<&ClaimRecord> {
        self.before.as_ref()
    }
    pub(crate) fn required(&self) -> Result<RequiredRoot, PublicationError> {
        required(&self.row)
    }
}
pub(crate) fn required(row: &ClaimRecord) -> Result<RequiredRoot, PublicationError> {
    row.validate()?;
    Ok(RequiredRoot {
        anchor: RootAnchor::IcClaim(row.open.operation, row.open.digest()?),
        identity: row.open.revision.clone(),
    })
}

//! Exact before/after retention for actual Registry job transitions.
//!
//! These nonserializable requests are Roll-private. The registered builder must
//! first read the real old row and the exact staged post-image through ActionTxn
//! and bind actual quota/artifact participant outputs. No input accepts roots or
//! a claimed certificate from a client. Retention alone never grants use.

use mush_core::CommitToken;

use super::rating::{RatingError, RatingJob, RatingJobState, RatingReceipt};
use super::roots::{RequiredRoot, RootAnchor};

pub(crate) struct RatingRoots {
    commit: CommitToken,
    before: Vec<RequiredRoot>,
    after: Vec<RequiredRoot>,
    marked: Option<super::marked_rating::AdmissionFence>,
}

fn root(job: &RatingJob, anchor: RootAnchor) -> RequiredRoot {
    RequiredRoot {
        anchor,
        identity: job.subject().revision(),
    }
}

/// Permanent job evidence survives every terminal state. A pending output is
/// a different, real obligation which occupies the future receipt's root slot.
pub(crate) fn required(job: &RatingJob) -> Result<Vec<RequiredRoot>, RatingError> {
    job.validate()?;
    let mut roots = vec![root(job, RootAnchor::RatingJob(job.job()))];
    match job.state() {
        RatingJobState::Queued {} | RatingJobState::Running { .. } => {
            roots.push(root(job, RootAnchor::RatingPending(job.job())));
        }
        RatingJobState::Completed { receipt } => {
            if receipt.job() != job.job() {
                return Err(RatingError::Identity);
            }
            roots.push(root(
                job,
                RootAnchor::Rating {
                    job: job.job(),
                    id: receipt.id(),
                },
            ));
        }
        RatingJobState::Nonconforming { .. }
        | RatingJobState::Failed { .. }
        | RatingJobState::Interrupted {}
        | RatingJobState::Cancelled {} => {}
    }
    Ok(roots)
}

impl RatingRoots {
    /// A paid receipt can advance the actual execution cursor without issuing
    /// a new lease or changing the typed job. The closed producer verifies the
    /// exact before/after execution rows and actual quota receipt first.
    pub(crate) fn settled_progress(
        job: &RatingJob,
        commit: CommitToken,
    ) -> Result<Self, RatingError> {
        if !matches!(job.state(), RatingJobState::Running { .. }) {
            return Err(RatingError::Invalid);
        }
        Ok(Self {
            commit,
            before: required(job)?,
            after: required(job)?,
            marked: None,
        })
    }
    /// Called only by the closed admission builder after checking destination
    /// absence and reading back the exact queued job. Its job is this commit.
    pub(crate) fn admission(job: &RatingJob, commit: CommitToken) -> Result<Self, RatingError> {
        if job.job() != commit || !matches!(job.state(), RatingJobState::Queued {}) {
            return Err(RatingError::Invalid);
        }
        Ok(Self {
            commit,
            before: Vec::new(),
            after: required(job)?,
            marked: None,
        })
    }

    /// The actual owner reader is captured only for new marked admission. The
    /// ordinary stored job/roots remain independent of subsequent rights loss.
    pub(crate) fn admission_marked(
        job: &RatingJob,
        commit: CommitToken,
        action: mush_core::DefinitionRef,
        access: super::marked_rating::MarkedRatingAccess,
    ) -> Result<Self, RatingError> {
        let fence = access.into_fence(action, commit);
        if super::RevisionIdentity::capture(fence.revision()).map_err(|_| RatingError::Identity)?
            != job.subject().revision()
        {
            return Err(RatingError::Identity);
        }
        let mut request = Self::admission(job, commit)?;
        request.marked = Some(fence);
        Ok(request)
    }
    pub(crate) fn marked(&self) -> Option<&super::marked_rating::AdmissionFence> {
        self.marked.as_ref()
    }

    /// Called after the actual paid receipt and staged job have been reconciled.
    /// A measured certificate is required only for exact Completed; ordinary
    /// progress preserves both roots, and abort removes pending alone.
    pub(crate) fn transition(
        before: &RatingJob,
        after: &RatingJob,
        receipt: Option<&RatingReceipt>,
        commit: CommitToken,
    ) -> Result<Self, RatingError> {
        if before.transition(after.state().clone())? != *after {
            return Err(RatingError::Identity);
        }
        match (after.state(), receipt) {
            (RatingJobState::Completed { .. }, Some(receipt)) => {
                receipt.validate()?;
                if !after.matches_receipt(receipt)? {
                    return Err(RatingError::Identity);
                }
            }
            (RatingJobState::Completed { .. }, None) | (_, Some(_)) => {
                return Err(RatingError::Incomplete);
            }
            (_, None) => {}
        }
        Ok(Self {
            commit,
            before: required(before)?,
            after: required(after)?,
            marked: None,
        })
    }

    pub(crate) const fn commit(&self) -> CommitToken {
        self.commit
    }
    pub(crate) fn before(&self) -> &[RequiredRoot] {
        &self.before
    }
    pub(crate) fn after(&self) -> &[RequiredRoot] {
        &self.after
    }
}

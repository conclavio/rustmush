//! Current decision and immutable historical proof share exact source joins.
use super::*;
use crate::commerce::{
    FiledRevisionId,
    rating::{RatingCompatibility, RatingJobState, nonconformance::NonconformanceRef},
    read::CommerceRead,
    records::RevisionStatus,
    roots::{RequiredRoot, RootAnchor},
};
use serde::Serialize;
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RejectionProof {
    pub(crate) filed: FiledRevisionId,
    pub(crate) revision: crate::commerce::RevisionIdentity,
    pub(crate) birth_rating: crate::commerce::rating::RatingRef,
    pub(crate) rejection: NonconformanceRef,
    pub(crate) plan: mush_core::DefinitionDigest,
    pub(crate) compatibility: RatingCompatibility,
    pub(crate) status: RevisionStatus,
    pub(crate) measured_at: mush_core::ClockStamp,
}
pub(crate) fn resolve(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    r: &RatingFamilies,
    filed: FiledRevisionId,
    expected: &RatingCompatibility,
) -> Result<RejectionProof, PublicationError> {
    use crate::commerce::entitlement::history::read;
    let row = read(ctx, &f.filed, filed.key())?;
    let key = super::super::ReconformanceHead::key_for(filed, expected.digest())?;
    let head = read(ctx, &f.reconformance_heads, key.clone())?;
    if head.key() != key || head.filed != filed || head.compatibility != expected.digest() {
        return Err(PublicationError::Mismatch);
    }
    let job = ctx
        .get(&r.jobs, &head.job)
        .map_err(bad)?
        .ok_or(PublicationError::Mismatch)?;
    let execution = ctx
        .get(&r.executions, &head.job)
        .map_err(bad)?
        .ok_or(PublicationError::Mismatch)?;
    execution.matches_job(job).map_err(bad)?;
    let binding = execution
        .admission()
        .reconformance
        .as_ref()
        .ok_or(PublicationError::Mismatch)?;
    let RatingJobState::Nonconforming { receipt } = job.state() else {
        return Err(PublicationError::Unavailable);
    };
    let rejection = r.complete_nonconformance(ctx, receipt).map_err(bad)?;
    if binding.filed() != filed
        || binding.birth_rating() != row.rating
        || job.compatibility() != expected
        || rejection.compatibility() != expected
        || rejection.subject().revision() != row.content.revision()
        || rejection.subject().definition() != row.definition
    {
        return Err(PublicationError::Mismatch);
    }
    ctx.world()
        .module_state::<crate::RollStore>()
        .ok_or(PublicationError::Unavailable)?
        .check_required_root(&RequiredRoot {
            anchor: RootAnchor::Filed(filed),
            identity: row.content.revision(),
        })
        .map_err(bad)?;
    let status = read(ctx, &f.status, filed.key())?;
    if status.filed != filed {
        return Err(PublicationError::Mismatch);
    }
    Ok(RejectionProof {
        filed,
        revision: row.content.revision(),
        birth_rating: row.rating,
        rejection: *receipt,
        plan: rejection.plan(),
        compatibility: expected.clone(),
        status: status.clone(),
        measured_at: rejection.publication().issued.clone(),
    })
}
pub(crate) fn verify(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    r: &RatingFamilies,
    op: &records::MeasuredRecall,
) -> Result<(), PublicationError> {
    op.validate()?;
    let header = ctx
        .get(&r.nonconformance, &op.rejection.job())
        .map_err(bad)?
        .ok_or(PublicationError::Mismatch)?;
    if header.reference().map_err(bad)? != op.rejection {
        return Err(PublicationError::Mismatch);
    }
    if op.manifest.digest() != header.compatibility().digest() {
        return Err(PublicationError::Mismatch);
    }
    let proof = resolve(ctx, f, r, op.filed, header.compatibility())?;
    if proof.revision != op.revision
        || proof.birth_rating != op.birth_rating
        || proof.rejection != op.rejection
        || proof.plan != op.plan
        || proof.status != op.status()
        || (op.at_ic.clock == proof.measured_at.clock && op.at_ic.at < proof.measured_at.at)
    {
        return Err(PublicationError::Mismatch);
    }
    ctx.world()
        .module_state::<crate::RollStore>()
        .ok_or(PublicationError::Unavailable)?
        .check_required_root(&op.root()?)
        .map_err(bad)
}

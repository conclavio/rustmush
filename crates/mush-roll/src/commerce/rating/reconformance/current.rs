//! Current execution certification is separate from immutable filing/term history.
//! Callers supply their actual composed compatibility, never a request digest.
use super::{RatingError, ReconformanceHead};
use crate::commerce::{
    families::CommerceFamilies,
    rating::{
        RatingCompatibility, RatingFamilies, RatingJob, RatingJobState, RatingReceipt, RatingRef,
        execution::RatingExecution,
    },
    read::CommerceRead,
    records::{FiledRecord, Record},
};
use mush_core::action_state::ActionTxn;

fn selected(
    head: &ReconformanceHead,
    filed: &FiledRecord,
    expected: &RatingCompatibility,
    job: &RatingJob,
    execution: &RatingExecution,
) -> Result<RatingRef, RatingError> {
    head.validate().map_err(|_| RatingError::Identity)?;
    filed.validate().map_err(|_| RatingError::Identity)?;
    job.validate()?;
    execution.matches_job(job)?;
    let admission = execution
        .admission()
        .reconformance
        .as_ref()
        .ok_or(RatingError::Identity)?;
    if head.filed != filed.content.id()
        || head.compatibility != expected.digest()
        || head.job != job.job()
        || job.compatibility() != expected
        || admission.filed() != head.filed
        || admission.birth_rating() != filed.rating
        || job.subject().revision() != filed.content.revision()
        || job.subject().definition() != filed.definition
    {
        return Err(RatingError::Identity);
    }
    // A failed or pending current-plan attempt is not a successful certificate.
    // Same-plan successful attempts cannot be superseded at admission.
    match job.state() {
        RatingJobState::Completed { receipt } => Ok(*receipt),
        _ => Err(RatingError::Unavailable),
    }
}

pub(crate) fn receipt<'a>(
    ctx: &'a impl CommerceRead,
    commerce: &CommerceFamilies,
    ratings: &RatingFamilies,
    filed: &FiledRecord,
    expected: &RatingCompatibility,
) -> Result<&'a RatingReceipt, RatingError> {
    let key = ReconformanceHead::key_for(filed.content.id(), expected.digest())
        .map_err(|_| RatingError::Invalid)?;
    let reference = match ctx
        .get(&commerce.reconformance_heads, &key)
        .map_err(|_| RatingError::Unavailable)?
    {
        None => filed.rating,
        Some(head) => {
            if head.key() != key {
                return Err(RatingError::Identity);
            }
            let job = ctx
                .get(&ratings.jobs, &head.job)
                .map_err(|_| RatingError::Unavailable)?
                .ok_or(RatingError::Incomplete)?;
            let execution = ctx
                .get(&ratings.executions, &head.job)
                .map_err(|_| RatingError::Unavailable)?
                .ok_or(RatingError::Incomplete)?;
            selected(head, filed, expected, job, execution)?
        }
    };
    let receipt = ratings.complete_active_receipt(ctx, &reference, expected)?;
    if receipt.subject().revision() != filed.content.revision()
        || receipt.subject().definition() != filed.definition
    {
        return Err(RatingError::Identity);
    }
    Ok(receipt)
}

pub(crate) fn receipt_in_txn(
    txn: &mut ActionTxn<'_>,
    commerce: &CommerceFamilies,
    ratings: &RatingFamilies,
    filed: &FiledRecord,
    expected: &RatingCompatibility,
) -> Result<RatingReceipt, RatingError> {
    let key = ReconformanceHead::key_for(filed.content.id(), expected.digest())
        .map_err(|_| RatingError::Invalid)?;
    let head = txn
        .get(&commerce.reconformance_heads, &key)
        .map_err(|_| RatingError::Unavailable)?
        .cloned();
    let reference = match head {
        None => filed.rating,
        Some(head) => {
            if head.key() != key {
                return Err(RatingError::Identity);
            }
            let job = txn
                .get(&ratings.jobs, &head.job)
                .map_err(|_| RatingError::Unavailable)?
                .cloned()
                .ok_or(RatingError::Incomplete)?;
            let execution = txn
                .get(&ratings.executions, &head.job)
                .map_err(|_| RatingError::Unavailable)?
                .cloned()
                .ok_or(RatingError::Incomplete)?;
            selected(&head, filed, expected, &job, &execution)?
        }
    };
    let receipt = ratings.complete_active_receipt_in_txn(txn, &reference, expected)?;
    if receipt.subject().revision() != filed.content.revision()
        || receipt.subject().definition() != filed.definition
    {
        return Err(RatingError::Identity);
    }
    Ok(receipt)
}

#[cfg(test)]
#[path = "current_tests.rs"]
mod tests;

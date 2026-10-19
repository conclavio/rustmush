//! Exact historical certificate for one filed revision. No current-plan or
//! title/term predicates enter immutable grant and captured-sortie validation.
use crate::commerce::{
    rating::{RatingError, RatingFamilies, RatingJobState, RatingReceipt, RatingRef},
    read::CommerceRead,
    records::FiledRecord,
};

pub(crate) fn certificate<'a>(
    ctx: &'a impl CommerceRead,
    ratings: &RatingFamilies,
    filed: &FiledRecord,
    reference: &RatingRef,
) -> Result<&'a RatingReceipt, RatingError> {
    let receipt = ratings.resolve_receipt_read(ctx, reference)?;
    if receipt.subject().revision() != filed.content.revision()
        || receipt.subject().definition() != filed.definition
    {
        return Err(RatingError::Identity);
    }
    if *reference != filed.rating {
        let job = ctx
            .get(&ratings.jobs, &reference.job())
            .map_err(|_| RatingError::Unavailable)?
            .ok_or(RatingError::Incomplete)?;
        let execution = ctx
            .get(&ratings.executions, &reference.job())
            .map_err(|_| RatingError::Unavailable)?
            .ok_or(RatingError::Incomplete)?;
        execution.matches_job(job)?;
        let binding = execution
            .admission()
            .reconformance
            .as_ref()
            .ok_or(RatingError::Identity)?;
        if binding.filed() != filed.content.id()
            || binding.birth_rating() != filed.rating
            || job.subject().revision() != filed.content.revision()
            || job.subject().definition() != filed.definition
            || job.compatibility() != receipt.compatibility()
            || !matches!(job.state(),RatingJobState::Completed{receipt:actual} if actual==reference)
        {
            return Err(RatingError::Identity);
        }
    }
    Ok(receipt)
}

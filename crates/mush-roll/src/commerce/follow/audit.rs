//! Called only after the mandatory whole-commerce history/source/root audit.
//! Build order is independent of file IDs and input-document order.
use super::*;
use crate::commerce::{
    audit::{read, scan},
    families::CommerceFamilies,
    rating::{RatingFamilies, RatingStanding},
};
use mush_core::ModuleCtx;

fn active(
    ctx: &ModuleCtx<'_>,
    ratings: &RatingFamilies,
    reference: &RatingRef,
) -> Result<bool, PublicationError> {
    let receipt = ratings
        .resolve_receipt(ctx, reference)
        .map_err(|_| PublicationError::Mismatch)?;
    ratings
        .complete_receipt(ctx, receipt)
        .map_err(|_| PublicationError::Mismatch)?;
    let status = ctx
        .action_state(&ratings.status, &reference.job())
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::Missing)?;
    status.validate().map_err(|_| PublicationError::Mismatch)?;
    if status.rating() != reference {
        return Err(PublicationError::Mismatch);
    }
    Ok(matches!(status.standing(), RatingStanding::Active {}))
}

pub(crate) fn rebuild(
    ctx: &ModuleCtx<'_>,
    commerce: &CommerceFamilies,
    ratings: &RatingFamilies,
) -> Result<Index, PublicationError> {
    let mut ordered = BTreeMap::new();
    scan(ctx, &commerce.filed, |filed| {
        let receipt = ratings
            .resolve_receipt(ctx, &filed.rating)
            .map_err(|_| PublicationError::Mismatch)?;
        let mut entry = Entry::filing(filed, receipt)?;
        if !active(ctx, ratings, &filed.rating)? {
            entry
                .certificates
                .insert(receipt.compatibility().digest(), None);
        }
        let status = read(ctx, &commerce.status, filed.key())?;
        if status.filed != entry.filed {
            return Err(PublicationError::Mismatch);
        }
        entry.blocked = status.recalled_by.is_some() || status.withdrawn_by.is_some();
        if ordered
            .insert((entry.line, entry.sequence), entry)
            .is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    })?;
    let mut index = Index::default();
    for (_, entry) in ordered {
        let previous = index.heads.get(&entry.line).map(|head| head.filed);
        let change = Change(Kind::Filing { entry, previous });
        let prepared = index.prepare(&change)?;
        index.apply(prepared);
    }
    let line_count = scan(ctx, &commerce.lines, |line| {
        let head = index.heads.get(&line.id).ok_or(PublicationError::Missing)?;
        let entry = index
            .entries
            .get(&head.filed)
            .ok_or(PublicationError::Missing)?;
        if head.filed != line.head
            || head.sequence.checked_add(1) != Some(line.next_sequence)
            || entry.charter != line.charter
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    })?;
    if line_count != index.heads.len() as u64 {
        return Err(PublicationError::Mismatch);
    }
    scan(ctx, &commerce.reconformance_heads, |head| {
        let filed = read(ctx, &commerce.filed, head.filed.key())?;
        let job = ctx
            .action_state(&ratings.jobs, &head.job)
            .map_err(|_| PublicationError::Unavailable)?
            .ok_or(PublicationError::Missing)?;
        let execution = ctx
            .action_state(&ratings.executions, &head.job)
            .map_err(|_| PublicationError::Unavailable)?
            .ok_or(PublicationError::Missing)?;
        // Only the explicit pending/failed branch may be ineligible. Broken
        // reads, mismatched heads and malformed evidence abort reconstruction.
        match crate::commerce::rating::reconformance::current::selected(
            head,
            filed,
            job.compatibility(),
            job,
            execution,
        ) {
            Ok(_) | Err(crate::commerce::rating::RatingError::Unavailable) => {}
            Err(_) => return Err(PublicationError::Mismatch),
        }
        let receipt = match job.state() {
            crate::commerce::rating::RatingJobState::Completed { receipt } => Some(
                ratings
                    .resolve_receipt(ctx, receipt)
                    .map_err(|_| PublicationError::Mismatch)?,
            ),
            _ => None,
        };
        let mut change = Change::certificate(filed, job, execution, receipt)?;
        if let Kind::Certificate { rating, .. } = &mut change.0 {
            if let Some(reference) = rating {
                if !active(ctx, ratings, &reference.reference)? {
                    *rating = None;
                }
            }
        }
        let prepared = index.prepare(&change)?;
        index.apply(prepared);
        Ok(())
    })?;
    Ok(index)
}

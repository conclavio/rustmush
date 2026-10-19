//! Indexed forward/reverse audit of actual filed re-conformance admissions.
//! Current standing is never required to retain historical measured evidence.
use super::ReconformanceHead;
use crate::commerce::{
    families::CommerceFamilies,
    rating::{RatingFamilies, RatingJobState},
    records::{PublicationError, Record},
};
use mush_core::{CommitToken, ModuleCtx};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn audit(
    ctx: &ModuleCtx<'_>,
    commerce: &CommerceFamilies,
    ratings: &RatingFamilies,
    enabled: bool,
) -> Result<(), PublicationError> {
    crate::commerce::audit::scan(ctx, &commerce.measured_recalls, |_| {
        if enabled {
            Ok(())
        } else {
            Err(PublicationError::Mismatch)
        }
    })?;
    let mut heads = BTreeMap::new();
    crate::commerce::audit::scan(ctx, &commerce.reconformance_heads, |head| {
        if !enabled || heads.insert(head.key(), head.job).is_some() {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    })?;
    let mut links: BTreeMap<String, BTreeMap<CommitToken, Option<CommitToken>>> = BTreeMap::new();
    let mut after = None;
    let mut count = 0usize;
    loop {
        let page = ctx
            .scan_action_state(&ratings.executions, after.as_deref(), 128)
            .map_err(|_| PublicationError::Unavailable)?;
        for entry in page.entries() {
            count = count.checked_add(1).ok_or(PublicationError::Capacity)?;
            if count > crate::MAX_RECORDS {
                return Err(PublicationError::Capacity);
            }
            let execution = entry.value();
            if entry.encoded_key() != execution.job().to_hex() {
                return Err(PublicationError::Mismatch);
            }
            let Some(binding) = execution.admission().reconformance.as_ref() else {
                continue;
            };
            if !enabled {
                return Err(PublicationError::Mismatch);
            }
            let job = ctx
                .action_state(&ratings.jobs, &execution.job())
                .map_err(|_| PublicationError::Unavailable)?
                .ok_or(PublicationError::Missing)?;
            execution
                .matches_job(job)
                .map_err(|_| PublicationError::Mismatch)?;
            let filed = crate::commerce::audit::read(ctx, &commerce.filed, binding.filed().key())?;
            let birth = ratings
                .resolve_receipt(ctx, &filed.rating)
                .map_err(|_| PublicationError::Mismatch)?;
            if binding.birth_rating() != filed.rating
                || job.subject().revision() != filed.content.revision()
                || job.subject().definition() != filed.definition
                || birth.subject().revision() != filed.content.revision()
                || birth.subject().definition() != filed.definition
            {
                return Err(PublicationError::Mismatch);
            }
            if let Some(previous) = binding.previous_attempt() {
                let old = ctx
                    .action_state(&ratings.jobs, &previous)
                    .map_err(|_| PublicationError::Unavailable)?
                    .ok_or(PublicationError::Missing)?;
                let old_execution = ctx
                    .action_state(&ratings.executions, &previous)
                    .map_err(|_| PublicationError::Unavailable)?
                    .ok_or(PublicationError::Missing)?;
                old_execution
                    .matches_job(old)
                    .map_err(|_| PublicationError::Mismatch)?;
                let prior = old_execution
                    .admission()
                    .reconformance
                    .as_ref()
                    .ok_or(PublicationError::Mismatch)?;
                if previous == job.job()
                    || prior.filed() != binding.filed()
                    || prior.birth_rating() != binding.birth_rating()
                    || old.compatibility() != job.compatibility()
                    || old.subject() != job.subject()
                    || !matches!(
                        old.state(),
                        RatingJobState::Failed { .. }
                            | RatingJobState::Cancelled {}
                            | RatingJobState::Interrupted {}
                    )
                {
                    return Err(PublicationError::Mismatch);
                }
            }
            let key = ReconformanceHead::key_for(binding.filed(), job.compatibility().digest())?;
            if links
                .entry(key)
                .or_default()
                .insert(job.job(), binding.previous_attempt())
                .is_some()
            {
                return Err(PublicationError::Mismatch);
            }
        }
        let Some(next) = page.next_after() else {
            break;
        };
        if page.entries().is_empty() || after.as_deref().is_some_and(|old| old >= next.as_str()) {
            return Err(PublicationError::Mismatch);
        }
        after = Some(next.to_string());
    }
    if heads.keys().ne(links.keys()) {
        return Err(PublicationError::Mismatch);
    }
    for (key, chain) in links {
        let mut next = Some(*heads.get(&key).ok_or(PublicationError::Missing)?);
        let mut seen = BTreeSet::new();
        while let Some(job) = next {
            if !seen.insert(job) {
                return Err(PublicationError::Mismatch);
            }
            next = *chain.get(&job).ok_or(PublicationError::Missing)?;
        }
        if seen.len() != chain.len() {
            return Err(PublicationError::Mismatch);
        }
    }
    Ok(())
}

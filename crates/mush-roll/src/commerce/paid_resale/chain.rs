//! Complete bounded structural reconstruction. The caller separately proves
//! every actual native history, offer, accepted term and retained root. There is
//! no current eligibility check here and no insertion-order history fallback.
use super::super::Holder;
use super::records::*;
use crate::commerce::{
    TermGeneration,
    paid::{PaidTermRecord, PersonalPaidLicence},
    records::{PublicationError, Record},
};
use mush_core::CommitToken;
use mush_economy::managed_paid_resale::{PaidBirthRefV1, PaidHolderRefV1};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn verify(
    licence: &PersonalPaidLicence,
    birth: &PaidBirthRefV1,
    birth_holder: Holder,
    head: Option<&PaidResaleHead>,
    operations: &[&PaidResaleRecord],
    holders: &BTreeMap<u64, &PaidHolderRecord>,
    terms: &BTreeMap<TermGeneration, &PaidTermRecord>,
) -> Result<(), PublicationError> {
    licence.validate()?;
    birth.validate().map_err(|_| PublicationError::Invalid)?;
    if operations.len() > MAX_OPERATIONS
        || holders.len() > MAX_OPERATIONS
        || birth.operation != licence.operation()
        || birth_holder.is_nobody()
    {
        return Err(PublicationError::Mismatch);
    }
    let mut by_sequence = BTreeMap::new();
    let mut offers = BTreeSet::new();
    let mut commits = BTreeSet::<CommitToken>::new();
    for &row in operations {
        row.validate()?;
        if row.binding().licence() != licence.id()
            || row.binding().birth() != birth
            || row.binding().charter() != licence.charter()
            || row.binding().line() != licence.line()
            || by_sequence.insert(row.sequence(), row).is_some()
            || !offers.insert(row.offer())
            || !commits.insert(row.operation())
        {
            return Err(PublicationError::Mismatch);
        }
        if let Some(t) = row.state().terminal()
            && t != row.operation()
            && !commits.insert(t)
        {
            return Err(PublicationError::Mismatch);
        }
    }
    let mut current = PaidHolderRefV1 {
        generation: 1,
        operation: birth.operation,
        digest: birth.digest,
    };
    let mut holder = birth_holder;
    let mut last: Option<&PaidResaleRecord> = None;
    let mut transferred = BTreeSet::new();
    let mut prior_term = TermGeneration::new(1).map_err(|_| PublicationError::Invalid)?;
    for (offset, (_, row)) in by_sequence.iter().enumerate() {
        if row.sequence() != (offset as u64) + 1
            || row.previous().cloned() != last.map(|p| p.reference()).transpose()?
            || last.is_some_and(|p| matches!(p.state(), PaidResaleState::Funded {}))
            || row.binding().ownership() != &current
            || Holder::Character(row.economic().consent().terms.seller) != holder
            || row.binding().term().generation() < prior_term
            || row.binding().term().generation() > licence.term()
            || terms.get(&row.binding().term().generation()).copied() != Some(row.binding().term())
        {
            return Err(PublicationError::Mismatch);
        }
        prior_term = row.binding().term().generation();
        match row.state() {
            PaidResaleState::Settled { ownership, .. } => {
                let next = holders
                    .get(&ownership.generation)
                    .ok_or(PublicationError::Missing)?;
                next.matches_record(row)?;
                if &next.ownership()? != ownership || !transferred.insert(next.generation()) {
                    return Err(PublicationError::Mismatch);
                }
                current = *ownership;
                holder = next.holder();
            }
            PaidResaleState::Funded {} => {
                // A pending attempt retains precisely today's instrument;
                // reciprocal renewal admission excludes a term advance.
                if row.binding().term().generation() != licence.term() {
                    return Err(PublicationError::Mismatch);
                }
            }
            PaidResaleState::Refunded { .. } => (),
        }
        last = Some(row);
    }
    if transferred.len() != holders.len()
        || holder != licence.holder()
        || match licence.ownership() {
            None => current.generation != 1,
            Some(r) => r != &current,
        }
    {
        return Err(PublicationError::Mismatch);
    }
    match (last, head) {
        (None, None) => Ok(()),
        (Some(row), Some(head)) => head.matches(row),
        _ => Err(PublicationError::Missing),
    }
}

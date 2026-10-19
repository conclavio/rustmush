//! Bounded immutable term/holder/attempt reconstruction. The caller proves
//! every actual native acceptance and holder row; these are structural joins.
use super::super::{
    TermGeneration,
    entitlement::PaidTermEconomicsV1,
    paid::{PaidTermRecord, PersonalPaidLicence},
    records::{PublicationError, Record},
};
use super::records::{MAX_OPERATIONS, RenewalAttemptV1, RenewalHeadV1, RenewalOperationV1};
use mush_core::CommitToken;
use std::collections::{BTreeMap, BTreeSet};

fn holder(row: &RenewalOperationV1) -> u64 {
    row.binding
        .holder_basis()
        .map_or(1, |b| b.ownership.generation)
}
fn immutable(row: &RenewalOperationV1, licence: &PersonalPaidLicence) -> bool {
    row.binding.licence() == licence.id()
        && row.binding.charter() == licence.charter()
        && row.binding.line() == licence.line()
        && row.binding.birth_operation() == licence.operation()
}
/// The current licence is an actual transaction beforeimage. Old closed heads
/// remain their accepted holder; a different holder needs exact newer evidence.
pub(crate) fn check_draft(
    licence: &PersonalPaidLicence,
    row: &RenewalOperationV1,
    head: Option<&RenewalHeadV1>,
    previous: Option<&RenewalOperationV1>,
) -> Result<(), PublicationError> {
    licence.validate()?;
    row.validate()?;
    if !matches!(row.state, super::RenewalStateV1::Drafted {})
        || !row.binding.matches_birth(licence)
        || row.binding.predecessor != licence.term()
    {
        return Err(PublicationError::Mismatch);
    }
    match (head, previous) {
        (None, None) if licence.term().get() == 1 && row.attempt.previous_operation().is_none() => {
            Ok(())
        }
        (Some(head), Some(previous)) => {
            if head != &RenewalHeadV1::for_operation(previous)?
                || head.pending.is_some()
                || !immutable(previous, licence)
                || head.current != licence.term()
                || previous.operation == row.operation
                || previous.contract == row.contract
                || row.operation
                    == previous
                        .state
                        .terminal_commit()
                        .ok_or(PublicationError::Mismatch)?
            {
                return Err(PublicationError::Mismatch);
            }
            let utc = match &previous.state {
                super::RenewalStateV1::Granted { terminal, .. }
                | super::RenewalStateV1::Refunded { terminal, .. } => terminal.posted_at_utc(),
                super::RenewalStateV1::UnfundedClosed { closure } => closure.posted_at_utc,
                super::RenewalStateV1::UnfundedServiceClosed { closure } => closure.posted_at_utc(),
                _ => return Err(PublicationError::Mismatch),
            };
            if row.created_at_utc < utc {
                return Err(PublicationError::Mismatch);
            }
            match row.attempt {
                RenewalAttemptV1::Automatic {} | RenewalAttemptV1::HolderAutomatic { .. }
                    if previous.state.granted()
                        && row.binding.predecessor > previous.binding.predecessor
                        && holder(row) >= holder(previous) =>
                {
                    Ok(())
                }
                RenewalAttemptV1::HolderAutomatic { .. }
                    if previous.state.retryable()
                        && row.binding.predecessor == previous.binding.predecessor
                        && holder(row) > holder(previous) =>
                {
                    Ok(())
                }
                RenewalAttemptV1::BuyerRetry { previous_operation }
                | RenewalAttemptV1::HolderRetry {
                    previous_operation, ..
                } if previous_operation == previous.operation
                    && previous.state.retryable()
                    && head.current == head.attempted_predecessor
                    && row.binding == previous.binding =>
                {
                    Ok(())
                }
                _ => Err(PublicationError::Mismatch),
            }
        }
        _ => Err(PublicationError::Mismatch),
    }
}

pub(crate) fn verify(
    licence: &PersonalPaidLicence,
    head: Option<&RenewalHeadV1>,
    operations: &[&RenewalOperationV1],
    terms: &BTreeMap<TermGeneration, &PaidTermRecord>,
    economics: &BTreeMap<TermGeneration, &PaidTermEconomicsV1>,
) -> Result<(), PublicationError> {
    licence.validate()?;
    if operations.len() > MAX_OPERATIONS {
        return Err(PublicationError::Capacity);
    }
    let one = TermGeneration::new(1).map_err(|_| PublicationError::Invalid)?;
    let birth = terms.get(&one).ok_or(PublicationError::Missing)?;
    let birth_economics = economics.get(&one).ok_or(PublicationError::Missing)?;
    if birth.licence() != licence.id()
        || birth.operation() != licence.operation()
        || birth_economics.licence() != licence.id()
        || birth_economics.operation() != licence.operation()
        || birth_economics.generation() != one
    {
        return Err(PublicationError::Mismatch);
    }
    let mut automatic = BTreeMap::new();
    let mut successors = BTreeMap::new();
    let mut tokens = BTreeSet::new();
    let mut commits = BTreeSet::from([licence.operation()]);
    let mut contracts = BTreeSet::new();
    for &row in operations {
        row.validate()?;
        if !immutable(row, licence)
            || !tokens.insert(row.operation)
            || !contracts.insert(row.contract)
            || !commits.insert(row.operation)
            || row
                .state
                .funding()
                .is_some_and(|f| !commits.insert(f.commit()))
            || row
                .state
                .terminal_commit()
                .is_some_and(|c| !commits.insert(c))
        {
            return Err(PublicationError::Mismatch);
        }
        if let Some(previous) = row.attempt.previous_operation() {
            if successors.insert(previous, row).is_some() {
                return Err(PublicationError::Mismatch);
            }
        } else if automatic
            .insert((row.binding.predecessor, holder(row)), row)
            .is_some()
        {
            return Err(PublicationError::Mismatch);
        }
    }
    let mut generation = one;
    let mut last: Option<&RenewalOperationV1> = None;
    let mut visits = BTreeSet::<CommitToken>::new();
    let mut grant_count = 0_usize;
    for ((predecessor, holder_generation), mut row) in automatic {
        if predecessor != generation || last.is_some_and(|p| holder_generation < holder(p)) {
            return Err(PublicationError::Mismatch);
        }
        let at_generation = row.binding.accepted_pointer(generation)?;
        let admitted = RenewalOperationV1 {
            state: super::RenewalStateV1::Drafted {},
            ..(*row).clone()
        };
        let prior_head = last.map(RenewalHeadV1::for_operation).transpose()?;
        check_draft(&at_generation, &admitted, prior_head.as_ref(), last)?;
        loop {
            if !visits.insert(row.operation) {
                return Err(PublicationError::Mismatch);
            }
            let predecessor = terms.get(&generation).ok_or(PublicationError::Missing)?;
            let predecessor_economics = economics
                .get(&generation)
                .ok_or(PublicationError::Missing)?;
            row.binding
                .check_predecessor(&at_generation, predecessor, predecessor_economics)?;
            last = Some(row);
            if row.state.granted() {
                let (_, expected_term, expected_economics) = row.granted_rows()?;
                if terms.get(&row.binding.next).copied() != Some(&expected_term)
                    || economics.get(&row.binding.next).copied() != Some(&expected_economics)
                    || successors.contains_key(&row.operation)
                {
                    return Err(PublicationError::Mismatch);
                }
                generation = row.binding.next;
                grant_count = grant_count
                    .checked_add(1)
                    .ok_or(PublicationError::Capacity)?;
                break;
            }
            let Some(next) = successors.remove(&row.operation) else {
                break;
            };
            let old_head = RenewalHeadV1::for_operation(row)?;
            let admitted = RenewalOperationV1 {
                state: super::RenewalStateV1::Drafted {},
                ..(*next).clone()
            };
            check_draft(&at_generation, &admitted, Some(&old_head), Some(row))?;
            row = next;
        }
    }
    if visits.len() != operations.len()
        || !successors.is_empty()
        || terms.len() != grant_count + 1
        || economics.len() != grant_count + 1
        || licence.term() != generation
    {
        return Err(PublicationError::Mismatch);
    }
    if last.is_some_and(|r| {
        holder(r) > licence.ownership().map_or(1, |b| b.generation)
            || (r.state.pending() && !r.binding.matches_birth(licence))
    }) {
        return Err(PublicationError::Mismatch);
    }
    match (last, head) {
        (None, None) => Ok(()),
        (Some(last), Some(head)) if head == &RenewalHeadV1::for_operation(last)? => Ok(()),
        _ => Err(PublicationError::Mismatch),
    }
}

//! Complete immutable attempt/generation reconstruction, independent of token
//! sort order. These structural joins supplement actual E4/Market provenance;
//! they never establish it. No current issuer eligibility is read at boot.

use super::super::{
    TermGeneration,
    entitlement::PaidTermEconomicsV1,
    paid::{PaidTermRecord, PersonalPaidLicence},
    records::{PublicationError, Record},
};
use super::records::{MAX_OPERATIONS, RenewalAttemptV1, RenewalHeadV1, RenewalOperationV1};
use mush_core::CommitToken;
use std::collections::{BTreeMap, BTreeSet};

/// Before publishing Draft, require the current exact head and its actual last
/// operation. A pending/unknown obligation can never be replaced by a retry.
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
    match (head, previous, row.attempt) {
        (None, None, RenewalAttemptV1::Automatic {}) if licence.term().get() == 1 => Ok(()),
        (Some(head), Some(previous), attempt) => {
            if head != &RenewalHeadV1::for_operation(previous)?
                || head.pending.is_some()
                || head.licence != licence.id()
                || head.holder != licence.holder()
                || head.birth_operation != licence.operation()
                || head.current != licence.term()
                || previous.operation == row.operation
                || previous.contract == row.contract
            {
                return Err(PublicationError::Mismatch);
            }
            let at = previous
                .state
                .terminal_commit()
                .ok_or(PublicationError::Mismatch)?;
            if row.operation == at {
                return Err(PublicationError::Mismatch);
            }
            let terminal_utc = match &previous.state {
                super::RenewalStateV1::Granted { terminal, .. }
                | super::RenewalStateV1::Refunded { terminal, .. } => terminal.posted_at_utc(),
                super::RenewalStateV1::UnfundedClosed { closure } => closure.posted_at_utc,
                _ => return Err(PublicationError::Mismatch),
            };
            if row.created_at_utc < terminal_utc {
                return Err(PublicationError::Mismatch);
            }
            match attempt {
                RenewalAttemptV1::Automatic {}
                    if previous.state.granted()
                        && row.binding.predecessor.get() > head.attempted_predecessor.get() =>
                {
                    Ok(())
                }
                RenewalAttemptV1::BuyerRetry { previous_operation }
                    if previous_operation == previous.operation
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

/// Checks every retained attempt and every actual granted term/economics row
/// for one birth. The caller separately validates the birth and all real E4
/// offer/money histories, even when these collections are empty.
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
        if !row.binding.matches_birth(licence)
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
        match row.attempt {
            RenewalAttemptV1::Automatic {} => {
                if automatic.insert(row.binding.predecessor, row).is_some() {
                    return Err(PublicationError::Mismatch);
                }
            }
            RenewalAttemptV1::BuyerRetry { previous_operation } => {
                if successors.insert(previous_operation, row).is_some() {
                    return Err(PublicationError::Mismatch);
                }
            }
        }
    }
    let mut generation = one;
    let mut last = None;
    let mut visits = BTreeSet::<CommitToken>::new();
    let mut grant_count = 0_usize;
    // A grant advances one generation; a retry follows one unique terminal
    // link. Each row is visited at most once and all maps are bounded above.
    while let Some(mut row) = automatic.remove(&generation) {
        let prior = last;
        let old_head = prior.map(RenewalHeadV1::for_operation).transpose()?;
        let admitted = RenewalOperationV1 {
            state: super::RenewalStateV1::Drafted {},
            ..(*row).clone()
        };
        let at_generation = PersonalPaidLicence::original(
            licence.id(),
            licence.charter(),
            licence.line(),
            licence.holder(),
            generation,
            licence.operation(),
        )?;
        check_draft(&at_generation, &admitted, old_head.as_ref(), prior)?;
        loop {
            if !visits.insert(row.operation) {
                return Err(PublicationError::Mismatch);
            }
            let predecessor = terms.get(&generation).ok_or(PublicationError::Missing)?;
            let predecessor_economics = economics
                .get(&generation)
                .ok_or(PublicationError::Missing)?;
            row.binding
                .check_predecessor(licence, predecessor, predecessor_economics)?;
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
        if last.is_some_and(|row| !row.state.granted()) {
            break;
        }
    }
    if visits.len() != operations.len()
        || !automatic.is_empty()
        || !successors.is_empty()
        || terms.len() != grant_count + 1
        || economics.len() != grant_count + 1
        || licence.term() != generation
    {
        return Err(PublicationError::Mismatch);
    }
    match (last, head) {
        (None, None) => Ok(()),
        (Some(last), Some(head)) if head == &RenewalHeadV1::for_operation(last)? => Ok(()),
        _ => Err(PublicationError::Mismatch),
    }
}

//! Birth and current generation are independent retained obligations. Boot
//! proves every link; live reads bind the audited current head and predecessor.
use super::super::{
    entitlement::history::{granted, read},
    families::CommerceFamilies,
    rating::RatingFamilies,
    read::CommerceRead,
    records::{InitialFilingOperation, OwnerLicence, OwnerTerm, PublicationError, Record},
};
use super::records::AttestationOperation;
use crate::RollStore;

fn unavailable<T>(_: T) -> PublicationError {
    PublicationError::Unavailable
}
fn mismatch<T>(_: T) -> PublicationError {
    PublicationError::Mismatch
}

fn check(
    roll: &RollStore,
    licence: &OwnerLicence,
    term: &OwnerTerm,
    birth: &InitialFilingOperation,
    operation: Option<&AttestationOperation>,
    previous: Option<&OwnerTerm>,
) -> Result<(), PublicationError> {
    licence.validate()?;
    term.validate()?;
    birth.validate()?;
    if licence.operation != birth.commit
        || licence.id != birth.licence
        || licence.charter != birth.charter
        || licence.line != birth.line
        || term.licence != licence.id
        || term.generation != licence.term
    {
        return Err(PublicationError::Mismatch);
    }
    check_generation(roll, term, birth, operation, previous)
}

/// Historical causes are checked independently of the mutable current pointer.
/// This proves retained custody only; no current source or issue authority.
fn check_generation(
    roll: &RollStore,
    term: &OwnerTerm,
    birth: &InitialFilingOperation,
    operation: Option<&AttestationOperation>,
    previous: Option<&OwnerTerm>,
) -> Result<(), PublicationError> {
    term.validate()?;
    birth.validate()?;
    if term.licence != birth.licence {
        return Err(PublicationError::Mismatch);
    }
    match (operation, previous) {
        (None, None) if term.generation.get() == 1 && term.operation == birth.commit => {}
        (Some(op), Some(old)) if term.generation.get() > 1 => {
            op.follows(old, birth)?;
            if &op.next != term {
                return Err(PublicationError::Mismatch);
            }
            for root in op.roots()? {
                roll.check_required_root(&root).map_err(mismatch)?;
            }
        }
        _ => return Err(PublicationError::Mismatch),
    }
    Ok(())
}

pub(crate) fn current(
    ctx: &impl CommerceRead,
    roll: &RollStore,
    f: &CommerceFamilies,
    licence: &OwnerLicence,
) -> Result<OwnerTerm, PublicationError> {
    let term = read(
        ctx,
        &f.terms,
        OwnerTerm::encoded_key(licence.id, licence.term),
    )?;
    let birth = read(ctx, &f.operations, licence.operation.to_hex())?;
    let op = if term.generation.get() > 1 {
        Some(read(ctx, &f.owner_attestations, term.operation.to_hex())?)
    } else {
        None
    };
    let previous = op
        .map(|op| {
            read(
                ctx,
                &f.terms,
                OwnerTerm::encoded_key(licence.id, op.predecessor),
            )
        })
        .transpose()?;
    check(roll, licence, term, birth, op, previous)?;
    if let Some(op) = op {
        super::super::charter_transfer::history::require_captured(
            ctx,
            f,
            birth,
            op.charter_ownership.as_ref(),
            op.owner,
        )?;
    }
    Ok(term.clone())
}

/// Actual checked transactional reads, with bounded owned clones so no mutable
/// ActionTxn borrow escapes across another checked read.
pub(crate) fn current_in_txn(
    txn: &mut mush_core::ActionTxn<'_>,
    world: &mush_core::World,
    f: &CommerceFamilies,
    licence: &OwnerLicence,
) -> Result<OwnerTerm, PublicationError> {
    let term = txn
        .get(&f.terms, &OwnerTerm::encoded_key(licence.id, licence.term))
        .map_err(unavailable)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    let birth = txn
        .get(&f.operations, &licence.operation.to_hex())
        .map_err(unavailable)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    let op = if term.generation.get() > 1 {
        Some(
            txn.get(&f.owner_attestations, &term.operation.to_hex())
                .map_err(unavailable)?
                .cloned()
                .ok_or(PublicationError::Missing)?,
        )
    } else {
        None
    };
    let previous = match &op {
        None => None,
        Some(op) => Some(
            txn.get(
                &f.terms,
                &OwnerTerm::encoded_key(licence.id, op.predecessor),
            )
            .map_err(unavailable)?
            .cloned()
            .ok_or(PublicationError::Missing)?,
        ),
    };
    let roll = world
        .module_state::<RollStore>()
        .ok_or(PublicationError::Unavailable)?;
    roll.commerce_ready().map_err(unavailable)?;
    check(roll, licence, &term, &birth, op.as_ref(), previous.as_ref())?;
    if let Some(op) = &op {
        super::super::charter_transfer::history::require_captured_in_txn(
            txn,
            world,
            f,
            &birth,
            op.charter_ownership.as_ref(),
            op.owner,
        )?;
    }
    Ok(term)
}

/// An exact historical generation, including its immutable attestation cause
/// and predecessor/root joins. Whole-family audit still proves the full chain.
pub(crate) fn historical(
    ctx: &impl CommerceRead,
    roll: &RollStore,
    f: &CommerceFamilies,
    birth: &InitialFilingOperation,
    generation: super::super::TermGeneration,
) -> Result<OwnerTerm, PublicationError> {
    let term = read(
        ctx,
        &f.terms,
        OwnerTerm::encoded_key(birth.licence, generation),
    )?;
    let op = if generation.get() > 1 {
        Some(read(ctx, &f.owner_attestations, term.operation.to_hex())?)
    } else {
        None
    };
    let previous = op
        .map(|op| {
            read(
                ctx,
                &f.terms,
                OwnerTerm::encoded_key(birth.licence, op.predecessor),
            )
        })
        .transpose()?;
    if term.generation != generation {
        return Err(PublicationError::Mismatch);
    }
    check_generation(roll, term, birth, op, previous)?;
    if let Some(op) = op {
        super::super::charter_transfer::history::require_captured(
            ctx,
            f,
            birth,
            op.charter_ownership.as_ref(),
            op.owner,
        )?;
    }
    Ok(term.clone())
}

/// Same historical proof inside the actual checked transaction read set.
pub(crate) fn historical_in_txn(
    txn: &mut mush_core::ActionTxn<'_>,
    world: &mush_core::World,
    f: &CommerceFamilies,
    birth: &InitialFilingOperation,
    generation: super::super::TermGeneration,
) -> Result<OwnerTerm, PublicationError> {
    let term = txn
        .get(&f.terms, &OwnerTerm::encoded_key(birth.licence, generation))
        .map_err(unavailable)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    let op = if generation.get() > 1 {
        Some(
            txn.get(&f.owner_attestations, &term.operation.to_hex())
                .map_err(unavailable)?
                .cloned()
                .ok_or(PublicationError::Missing)?,
        )
    } else {
        None
    };
    let previous = match &op {
        Some(op) => Some(
            txn.get(
                &f.terms,
                &OwnerTerm::encoded_key(birth.licence, op.predecessor),
            )
            .map_err(unavailable)?
            .cloned()
            .ok_or(PublicationError::Missing)?,
        ),
        None => None,
    };
    let roll = world
        .module_state::<RollStore>()
        .ok_or(PublicationError::Unavailable)?;
    if term.generation != generation {
        return Err(PublicationError::Mismatch);
    }
    check_generation(roll, &term, birth, op.as_ref(), previous.as_ref())?;
    if let Some(op) = &op {
        super::super::charter_transfer::history::require_captured_in_txn(
            txn,
            world,
            f,
            birth,
            op.charter_ownership.as_ref(),
            op.owner,
        )?;
    }
    Ok(term)
}

pub(crate) fn verify(
    ctx: &impl CommerceRead,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    op: &AttestationOperation,
) -> Result<(), PublicationError> {
    let birth = read(ctx, &f.operations, op.birth_operation.to_hex())?;
    let previous = read(
        ctx,
        &f.terms,
        OwnerTerm::encoded_key(op.licence, op.predecessor),
    )?;
    op.follows(previous, birth)?;
    super::super::charter_transfer::history::require_captured(
        ctx,
        f,
        birth,
        op.charter_ownership.as_ref(),
        op.owner,
    )?;
    let term = read(ctx, &f.terms, op.next.key())?;
    let filed = read(ctx, &f.filed, op.filed.key())?;
    let rating =
        super::super::rating::reconformance::history::certificate(ctx, ratings, filed, &op.rating)
            .map_err(unavailable)?;
    if term != &op.next
        || filed.content.id() != op.filed
        || filed.content.revision() != op.revision
        || filed.content.position().charter != op.charter
        || filed.content.position().line != op.line
        || rating.subject().revision() != op.revision
        || rating.compatibility().digest() != op.manifest.digest()
    {
        return Err(PublicationError::Mismatch);
    }
    if previous.generation.get() > 1 {
        let prior = read(ctx, &f.owner_attestations, previous.operation.to_hex())?;
        if prior.next != *previous
            || prior.birth_operation != op.birth_operation
            || prior.observed_utc > op.observed_utc
            || (prior.observed_ic.clock == op.observed_ic.clock
                && prior.observed_ic.at > op.observed_ic.at)
        {
            return Err(PublicationError::Mismatch);
        }
    }
    if let Some(parent) = op.next.attestation.as_ref().and_then(|e| e.parent.as_ref()) {
        let selected = &parent.selected;
        let licence = read(ctx, &f.paid_licences, selected.licence.key())?;
        let term = read(
            ctx,
            &f.paid_terms,
            format!("{}/{}", selected.licence.key(), selected.term.key()),
        )?;
        let actual = granted(ctx, f, ratings, licence, term)?;
        if actual.holder() != op.owner
            || actual.licence() != selected.licence
            || actual.term() != selected.term
            || actual.charter() != parent.charter
            || actual.line() != parent.line
            || actual.filed() != selected.filed
            || actual.revision() != selected.revision
            || actual.operation() != selected.operation
            || actual.terminal() != selected.terminal
            || term.window() != selected.window
            || term.terms().form() != selected.form
        {
            return Err(PublicationError::Mismatch);
        }
    }
    for root in op.roots()? {
        roll.check_required_root(&root).map_err(mismatch)?;
    }
    Ok(())
}

/// Full forward/reverse census, including empty families and absent heads.
/// Only compact per-licence counters are retained, never cloned row payloads.
pub(crate) fn audit(
    ctx: &mush_core::ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    enabled: bool,
) -> Result<u64, PublicationError> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut chains = BTreeMap::<super::super::LicenceId, (u64, u64, mush_core::CommitToken)>::new();
    let mut generations = BTreeSet::new();
    let count = super::super::audit::scan(ctx, &f.owner_attestations, |op| {
        if !enabled {
            return Err(PublicationError::Unavailable);
        }
        verify(ctx, roll, f, ratings, op)?;
        if !generations.insert((op.licence, op.next.generation)) {
            return Err(PublicationError::Mismatch);
        }
        let row = chains.entry(op.licence).or_insert((0, 1, op.operation));
        row.0 = row.0.checked_add(1).ok_or(PublicationError::Capacity)?;
        if op.next.generation.get() > row.1 {
            row.1 = op.next.generation.get();
            row.2 = op.operation;
        }
        Ok(())
    })?;
    super::super::audit::scan(ctx, &f.licences, |licence| {
        let term = current(ctx, roll, f, licence)?;
        match chains.remove(&licence.id) {
            None if licence.term.get() == 1 => {}
            Some((count, last, operation))
                if enabled
                    && count.checked_add(1) == Some(last)
                    && licence.term.get() == last
                    && term.operation == operation => {}
            _ => return Err(PublicationError::Mismatch),
        }
        Ok(())
    })?;
    if !chains.is_empty() {
        return Err(PublicationError::Mismatch);
    }
    Ok(count)
}

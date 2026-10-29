//! Actual historical qualification evidence. No live issuer or parent rights
//! are used to decide whether an acknowledged replacement happened.
use super::super::{
    dependent_owner::OwnerParentBinding,
    entitlement::history as paid_history,
    families::CommerceFamilies,
    rating::RatingFamilies,
    read::CommerceRead,
    records::{OwnerTerm, PublicationError, Record, RevisionPolicy},
};
use super::records::{QualificationHead, QualificationOperation};
use crate::RollStore;

pub(crate) fn verify(
    ctx: &impl CommerceRead,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    op: &QualificationOperation,
) -> Result<(), PublicationError> {
    op.validate()?;
    let birth = paid_history::read(ctx, &f.operations, op.birth_operation.to_hex())?;
    let binding = paid_history::read(
        ctx,
        &f.owner_parents,
        OwnerParentBinding::key_for(op.owner_licence, birth.requester),
    )?;
    let filed = paid_history::read(ctx, &f.filed, op.filed.key())?;
    let term = paid_history::read(
        ctx,
        &f.terms,
        OwnerTerm::encoded_key(op.owner_licence, op.owner_term),
    )?;
    op.matches_birth(binding, filed, term, birth)?;
    super::super::charter_transfer::history::require_captured(
        ctx,
        f,
        birth,
        op.charter_ownership.as_ref(),
        op.owner,
    )?;
    super::super::dependent_owner::verify_history(
        ctx, roll, f, ratings, binding, filed, term, birth,
    )?;
    let selected = &op.selected_parent;
    let parent = paid_history::read(ctx, &f.filed, selected.filed.key())?;
    let licence = paid_history::read(ctx, &f.paid_licences, selected.licence.key())?;
    let paid_term = paid_history::read(
        ctx,
        &f.paid_terms,
        format!("{}/{}", selected.licence.key(), selected.term.key()),
    )?;
    let proof = paid_history::granted(ctx, f, ratings, licence, paid_term)?;
    require_selected_holder(
        ctx,
        f,
        ratings,
        licence,
        selected.holder_basis.as_ref(),
        proof.holder(),
        op.owner,
        &op.observed_ic,
    )?;
    if proof.operation() != selected.operation
        || proof.terminal() != selected.terminal
        || proof.licence() != selected.licence
        || proof.term() != selected.term
        || proof.charter() != op.parent_charter
        || proof.line() != op.parent_line
        || proof.filed() != selected.filed
        || proof.revision() != selected.revision
        || paid_term.window() != selected.window
        || paid_term.terms().form() != selected.form
        || paid_term.policy()
            != (RevisionPolicy::Pin {
                filed: selected.filed,
                revision: selected.revision,
            })
        || parent.content.id() != selected.filed
        || parent.content.revision() != selected.revision
        || parent.content.position().charter != op.parent_charter
        || parent.content.position().line != op.parent_line
    {
        return Err(PublicationError::Mismatch);
    }
    roll.check_required_root(&op.root()?)
        .map_err(|_| PublicationError::Mismatch)
}

/// Immutable selected-holder attribution, separate from the accepted term's
/// holder and today's licence pointer. Old omitted evidence keeps the exact
/// original term-holder rule; a new capture must resolve the actual settlement.
#[allow(clippy::too_many_arguments)]
pub(crate) fn require_selected_holder(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence: &super::super::paid::PersonalPaidLicence,
    basis: Option<&super::records::ParentHolderBasis>,
    accepted_holder: super::super::Holder,
    owner: mush_core::ObjId,
    observed: &mush_core::ClockStamp,
) -> Result<(), PublicationError> {
    // M8c F1 (the lead's ruling): the selected holder is the dependent's
    // owner as a character; an organisation-held parent never qualifies a
    // personal filing in this cut (`owner_qualification::current`).
    let Some(basis) = basis else {
        return if accepted_holder == super::super::Holder::Character(owner) {
            Ok(())
        } else {
            Err(PublicationError::Mismatch)
        };
    };
    basis.validate()?;
    if super::super::paid_resale::history::historical(ctx, f, ratings, licence, &basis.ownership)?
        != super::super::Holder::Character(owner)
    {
        return Err(PublicationError::Mismatch);
    }
    let holder = paid_history::read(
        ctx,
        &f.paid_holders,
        super::super::paid_resale::PaidHolderRecord::key_for(
            licence.id(),
            basis.ownership.generation,
        ),
    )?;
    if holder.acceptance() != basis.acceptance || holder.ownership()? != basis.ownership {
        return Err(PublicationError::Mismatch);
    }
    let row = paid_history::read(ctx, &f.paid_resales, basis.acceptance.to_hex())?;
    holder.matches_record(row)?;
    let settled = row
        .economic()
        .terminal()
        .map(|t| &t.decision().at_ic)
        .unwrap_or(&row.economic().consent().accepted_at_ic);
    if settled.clock == observed.clock && settled.at > observed.at {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}

/// Source acquisition uses its own retained observation; omission keeps the
/// unchanged legacy accepted-term-holder rule without inventing a timestamp.
pub(crate) fn require_captured_holder(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence: &super::super::paid::PersonalPaidLicence,
    capture: Option<&super::records::ParentHolderCapture>,
    accepted_holder: super::super::Holder,
    owner: mush_core::ObjId,
) -> Result<(), PublicationError> {
    match capture {
        Some(capture) => {
            capture.validate()?;
            require_selected_holder(
                ctx,
                f,
                ratings,
                licence,
                Some(&capture.basis),
                accepted_holder,
                owner,
                &capture.observed_ic,
            )
        }
        // M8c F1: the legacy rule is the owner's own character holder.
        None if accepted_holder == super::super::Holder::Character(owner) => Ok(()),
        None => Err(PublicationError::Mismatch),
    }
}

/// Complete bounded chain inventory. Ordering uses acknowledged ordinal and
/// previous links, never commit-token ordering or unrelated clock generations.
pub(crate) fn audit(
    ctx: &mush_core::ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    enabled: bool,
) -> Result<(), PublicationError> {
    use std::collections::{BTreeMap, BTreeSet};
    // Keep only compact counters/ordinals; never clone the permanent payloads
    // into a second boot inventory. The shared scan has the actual row ceiling.
    let mut chains = BTreeMap::<String, (u64, u64, mush_core::CommitToken)>::new();
    let mut ordinals = BTreeSet::new();
    super::super::audit::scan(ctx, &f.owner_qualification_operations, |op| {
        if !enabled {
            return Err(PublicationError::Unavailable);
        }
        verify(ctx, roll, f, ratings, op)?;
        let prior = op
            .previous
            .map(|commit| {
                paid_history::read(ctx, &f.owner_qualification_operations, commit.to_hex())
            })
            .transpose()?;
        op.follows(prior)?;
        let key = op.head().key();
        if !ordinals.insert((key.clone(), op.ordinal)) {
            return Err(PublicationError::Mismatch);
        }
        let row = chains.entry(key).or_insert((0, 0, op.operation));
        row.0 = row.0.checked_add(1).ok_or(PublicationError::Capacity)?;
        if op.ordinal > row.1 {
            row.1 = op.ordinal;
            row.2 = op.operation;
        }
        Ok(())
    })?;
    let mut seen = BTreeSet::new();
    super::super::audit::scan(ctx, &f.owner_qualification_heads, |head| {
        if !enabled || !seen.insert(head.key()) {
            return Err(PublicationError::Mismatch);
        }
        let (count, ordinal, operation) =
            chains.get(&head.key()).ok_or(PublicationError::Missing)?;
        if count != ordinal || head.ordinal != *ordinal || head.operation != *operation {
            return Err(PublicationError::Mismatch);
        }
        let last = paid_history::read(ctx, &f.owner_qualification_operations, operation.to_hex())?;
        head.matches(last)
    })?;
    if seen.len() != chains.len() {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}

pub(crate) fn selected<'a>(
    ctx: &'a impl CommerceRead,
    f: &CommerceFamilies,
    owner: mush_core::ObjId,
    licence: super::super::LicenceId,
) -> Result<Option<&'a QualificationOperation>, PublicationError> {
    let head = paid_history::optional(
        ctx,
        &f.owner_qualification_heads,
        QualificationHead::key_for(licence, owner),
    )?;
    match head {
        None => Ok(None),
        Some(head) => {
            let operation = paid_history::read(
                ctx,
                &f.owner_qualification_operations,
                head.operation.to_hex(),
            )?;
            head.matches(operation)?;
            if head.owner != owner || head.owner_licence != licence {
                return Err(PublicationError::Mismatch);
            }
            Ok(Some(operation))
        }
    }
}

//! Bidirectional cause/inventory audit; never a current source-rights check.
use super::super::audit::{read, scan};
use super::super::families::CommerceFamilies;
use super::super::parent_selector::DraftParentSelection;
use super::super::records::{PublicationError, Record};
use super::records::{RetiredDraftParent, RetirementOperation};
use crate::RollStore;
use mush_core::ModuleCtx;

pub(crate) fn check(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    selector: &DraftParentSelection,
    tombstone: &RetiredDraftParent,
) -> Result<(), PublicationError> {
    let op = read(ctx, &f.authored_retirements, tombstone.operation.to_hex())?;
    op.check_selection(selector)?;
    if op.tombstone()? != *tombstone {
        return Err(PublicationError::Mismatch);
    }
    roll.check_authored_retirement(op, selector)
        .map_err(|_| PublicationError::Mismatch)
}
pub(crate) fn audit(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    enabled: bool,
) -> Result<u64, PublicationError> {
    let tombstones = scan(ctx, &f.retired_draft_parents, |row| {
        if !enabled {
            return Err(PublicationError::Unavailable);
        }
        let selector = read(ctx, &f.draft_parents, row.key())?;
        check(ctx, roll, f, selector, row)
    })?;
    let operations = scan(ctx, &f.authored_retirements, |op: &RetirementOperation| {
        if !enabled {
            return Err(PublicationError::Unavailable);
        }
        let tombstone = read(
            ctx,
            &f.retired_draft_parents,
            DraftParentSelection::key_for(op.actor, op.mind),
        )?;
        if tombstone.operation != op.operation {
            return Err(PublicationError::Mismatch);
        }
        let selector = read(ctx, &f.draft_parents, tombstone.key())?;
        check(ctx, roll, f, selector, tombstone)
    })?;
    if operations != tombstones {
        return Err(PublicationError::Mismatch);
    }
    Ok(tombstones)
}

//! Complete permanent sequence and fee proof. Current eligibility is never a boot predicate.
use super::records::{UpdateOperation, UpdatePosition};
use crate::RollStore;
use crate::commerce::{
    audit::{read, scan},
    families::CommerceFamilies,
    rating::RatingFamilies,
    read::CommerceRead,
    records::*,
    roots::{RequiredRoot, RootAnchor},
};
use mush_core::{DefinitionRef, ModuleCtx, StateFamilyHandle};

pub(crate) fn required(op: &UpdateOperation) -> [RequiredRoot; 2] {
    [
        RequiredRoot {
            anchor: RootAnchor::Filed(op.filed),
            identity: op.revision,
        },
        RequiredRoot {
            anchor: RootAnchor::UpdateOperation(op.commit),
            identity: op.revision,
        },
    ]
}
fn get<'a, R: Record>(
    ctx: &'a impl CommerceRead,
    h: &StateFamilyHandle<String, R>,
    key: String,
) -> Result<&'a R, PublicationError> {
    let row = ctx
        .get(h, &key)
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::Missing)?;
    row.validate()?;
    if row.key() != key {
        return Err(PublicationError::Mismatch);
    }
    Ok(row)
}
/// Ready owner readers check the exact immutable row and actual journal proof;
/// the mandatory whole-family boot audit has already proved the complete chain.
/// This function grants no current title, source, rating or term authority.
pub(crate) fn birth<'a>(
    ctx: &'a impl CommerceRead,
    f: &CommerceFamilies,
    filed: &FiledRecord,
) -> Result<&'a InitialFilingOperation, PublicationError> {
    filed.validate()?;
    if !matches!(filed.format, 3 | 4) {
        let initial = get(ctx, &f.operations, filed.operation.to_string())?;
        if initial.filed != filed.content.id() || initial.revision != filed.content.revision() {
            return Err(PublicationError::Mismatch);
        }
        return Ok(initial);
    }
    let op = get(ctx, &f.line_updates, filed.operation.to_string())?;
    let initial = get(ctx, &f.operations, op.initial.to_string())?;
    op.check_birth(initial)?;
    super::super::charter_transfer::history::require_captured(
        ctx,
        f,
        initial,
        op.charter_ownership.as_ref(),
        op.actor,
    )?;
    op.check_filed(filed)?;
    let charter = get(ctx, &f.charters, op.charter.key())?;
    let line = get(ctx, &f.lines, op.line.key())?;
    if charter.operation != initial.commit
        || charter.line != op.line
        || line.operation != initial.commit
        || line.charter != op.charter
        || line.schema_digest != op.schema
        || line.format != 2
        || op.sequence >= line.next_sequence
    {
        return Err(PublicationError::Mismatch);
    }
    get(
        ctx,
        &f.update_positions,
        UpdatePosition::key_for(op.line, op.sequence),
    )?
    .check(op)?;
    let economy = ctx
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    economy
        .update_receipt_audit()
        .verify(&op.economic)
        .map_err(|_| PublicationError::Mismatch)?;
    let roll = ctx
        .world()
        .module_state::<RollStore>()
        .ok_or(PublicationError::Unavailable)?;
    let term =
        super::super::owner_attestation::history::historical(ctx, roll, f, initial, op.term)?;
    op.check_owner_term(&term)?;
    for root in required(op) {
        roll.check_required_root(&root)
            .map_err(|_| PublicationError::Mismatch)?;
    }
    Ok(initial)
}

pub(crate) fn audit(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    action: Option<&DefinitionRef>,
) -> Result<u64, PublicationError> {
    audit_with_derived(ctx, roll, f, ratings, action, None)
}
pub(crate) fn audit_with_derived(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    action: Option<&DefinitionRef>,
    derived: Option<&DefinitionRef>,
) -> Result<u64, PublicationError> {
    let economy = ctx
        .state::<mush_economy::EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    let economic = economy.update_receipt_audit();
    economic
        .require_complete()
        .map_err(|_| PublicationError::Unavailable)?;
    let count = scan(ctx, &f.line_updates, |op| {
        if (if op.parent.is_some() { derived } else { action }).is_none() {
            return Err(PublicationError::Unavailable);
        }
        let initial = read(ctx, &f.operations, op.initial.to_string())?;
        op.check_birth(initial)?;
        let filed = read(ctx, &f.filed, op.filed.key())?;
        op.check_filed(filed)?;
        birth(ctx, f, filed)?;
        let status = read(ctx, &f.status, op.filed.key())?;
        super::super::withdrawal::audit::status(ctx, f, status)?;
        let identity = read(ctx, &f.identities, op.duplicate.to_string())?;
        if identity.filed != op.filed
            || identity.revision != op.revision
            || identity.duplicate != op.duplicate
        {
            return Err(PublicationError::Mismatch);
        }
        let reference = op.revision.revision();
        let revision = roll
            .revision(reference.key.character, reference.key)
            .map_err(|_| PublicationError::Unavailable)?;
        if op.parent.is_some() {
            super::derived::historical(ctx, roll, f, ratings, op, initial)?;
        } else {
            revision
                .require_original_access()
                .map_err(|_| PublicationError::Mismatch)?;
        }
        if !op
            .revision
            .matches(revision)
            .map_err(|_| PublicationError::Invalid)?
            || schema_line_digest(revision.envelope())? != op.schema
            || canonical_duplicate_digest(revision)? != op.duplicate
        {
            return Err(PublicationError::Mismatch);
        }
        let rating = ratings
            .resolve_receipt(ctx, &op.rating)
            .map_err(|_| PublicationError::Unavailable)?;
        let historical_action = if op.parent.is_some() {
            super::registration::historical_derived_action(rating.compatibility())
        } else {
            super::registration::historical_action(rating.compatibility())
        }
        .map_err(|_| PublicationError::Invalid)?;
        if historical_action != op.action
            || rating.reference().map_err(|_| PublicationError::Invalid)? != op.rating
            || rating.subject().revision() != op.revision
            || rating.subject().schema() != op.schema
            || rating.subject().definition() != filed.definition
        {
            return Err(PublicationError::Mismatch);
        }
        filed
            .content
            .check_links(&super::super::dependent_owner::ContentView {
                ctx,
                roll,
                families: f,
            })
            .map_err(|_| PublicationError::Mismatch)?;
        Ok(())
    })?;
    let positions = scan(ctx, &f.update_positions, |p| {
        let op = read(ctx, &f.line_updates, p.operation.to_string())?;
        p.check(op)
    })?;
    if positions != count {
        return Err(PublicationError::Mismatch);
    }
    let mut fee_count = 0u64;
    for receipt in economic
        .receipts()
        .map_err(|_| PublicationError::Unavailable)?
    {
        fee_count = fee_count.checked_add(1).ok_or(PublicationError::Capacity)?;
        let op = read(ctx, &f.line_updates, receipt.commit.to_string())?;
        if &op.economic != receipt || (if op.parent.is_some() { derived } else { action }).is_none()
        {
            return Err(PublicationError::Mismatch);
        }
    }
    if fee_count != count {
        return Err(PublicationError::Mismatch);
    }
    let mut linked = 0u64;
    scan(ctx, &f.lines, |line| {
        let initial = read(ctx, &f.operations, line.operation.to_string())?;
        if line.id != initial.line || line.charter != initial.charter {
            return Err(PublicationError::Mismatch);
        }
        let mut previous = initial.filed;
        for sequence in 2..line.next_sequence {
            linked = linked.checked_add(1).ok_or(PublicationError::Capacity)?;
            if linked > count {
                return Err(PublicationError::Mismatch);
            }
            let position = read(
                ctx,
                &f.update_positions,
                UpdatePosition::key_for(line.id, sequence),
            )?;
            let op = read(ctx, &f.line_updates, position.operation.to_string())?;
            position.check(op)?;
            op.check_birth(initial)?;
            let prior_file = read(ctx, &f.filed, previous.key())?;
            if op.previous != previous
                || op.schema != line.schema_digest
                || prior_file.content.filed_at().get() > op.at_ic.at
            {
                return Err(PublicationError::Mismatch);
            }
            previous = op.filed;
        }
        if line.head != previous {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    })?;
    if linked != count {
        return Err(PublicationError::Mismatch);
    }
    Ok(count)
}

/// Exact transaction-side historical routing for the source-free withdrawal producer.
pub(crate) fn birth_in_txn(
    txn: &mut mush_core::ActionTxn<'_>,
    world: &mush_core::World,
    f: &CommerceFamilies,
    filed: &FiledRecord,
) -> Result<InitialFilingOperation, PublicationError> {
    filed.validate()?;
    let initial_key = if matches!(filed.format, 3 | 4) {
        let op = txn
            .get(&f.line_updates, &filed.operation.to_string())
            .map_err(|_| PublicationError::Unavailable)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        op.check_filed(filed)?;
        let position = txn
            .get(
                &f.update_positions,
                &UpdatePosition::key_for(op.line, op.sequence),
            )
            .map_err(|_| PublicationError::Unavailable)?
            .ok_or(PublicationError::Missing)?;
        position.check(&op)?;
        let initial = txn
            .get(&f.operations, &op.initial.to_string())
            .map_err(|_| PublicationError::Unavailable)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        op.check_birth(&initial)?;
        super::super::charter_transfer::history::require_captured_in_txn(
            txn,
            world,
            f,
            &initial,
            op.charter_ownership.as_ref(),
            op.actor,
        )?;
        let term = super::super::owner_attestation::history::historical_in_txn(
            txn, world, f, &initial, op.term,
        )?;
        op.check_owner_term(&term)?;
        world
            .module_state::<mush_economy::EconomyStore>()
            .ok_or(PublicationError::Unavailable)?
            .update_receipt_audit()
            .verify(&op.economic)
            .map_err(|_| PublicationError::Mismatch)?;
        let roll = world
            .module_state::<RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        for required in required(&op) {
            roll.check_required_root(&required)
                .map_err(|_| PublicationError::Mismatch)?;
        }
        op.initial
    } else {
        filed.operation
    };
    let initial = txn
        .get(&f.operations, &initial_key.to_string())
        .map_err(|_| PublicationError::Unavailable)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    initial.validate()?;
    if !matches!(filed.format, 3 | 4)
        && (initial.filed != filed.content.id() || initial.revision != filed.content.revision())
    {
        return Err(PublicationError::Mismatch);
    }
    Ok(initial)
}

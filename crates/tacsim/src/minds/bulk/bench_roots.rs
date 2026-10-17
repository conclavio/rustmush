//! Ephemeral bench source roots retain full immutable revision envelopes. A
//! worker owns the root until it actually exits, including cancelled requests.
use super::{BulkError, invalid};
use mush_core::{AccountId, Ctx};
use mush_roll::{ReferenceKey, ReferenceKind, RevisionRef, RollStore};
const PREFIX: &str = "bench/";
pub(super) fn key(account: AccountId, lease: u64, ticket: u64) -> ReferenceKey {
    ReferenceKey {
        owner: crate::store::NS.into(),
        kind: ReferenceKind::OpenSeal,
        record: format!(
            "{PREFIX}{}/{lease}/{ticket}",
            account.canonical_hash().to_hex()
        ),
    }
}
pub(super) fn change(
    ctx: &mut Ctx<'_>,
    replacement: Option<(ReferenceKey, RevisionRef)>,
    releases: Vec<ReferenceKey>,
) -> Result<(), BulkError> {
    if replacement.is_none() && releases.is_empty() {
        return Ok(());
    }
    if let Some(module) = &ctx.module
        && (module != crate::store::NS
            || ctx.reg.persist_delegation(module, mush_roll::NS).is_none())
    {
        return Err(invalid("bench publication lacks owner authority"));
    }
    if replacement
        .as_ref()
        .is_some_and(|(_, r)| r.key.character != ctx.actor)
    {
        return Err(BulkError::Missing);
    }
    if replacement
        .iter()
        .map(|(k, _)| k)
        .chain(releases.iter())
        .any(|k| {
            k.owner != crate::store::NS
                || k.kind != ReferenceKind::OpenSeal
                || !k.record.starts_with(PREFIX)
        })
    {
        return Err(invalid("bench reference owner"));
    }
    let roll = ctx
        .w
        .module_state_mut::<RollStore>()
        .ok_or(BulkError::Missing)?;
    let existing = roll.reference_keys(crate::store::NS, ReferenceKind::OpenSeal);
    let releases = releases
        .into_iter()
        .filter(|k| existing.contains(k))
        .collect::<Vec<_>>();
    let replacements = replacement
        .into_iter()
        .map(|(k, r)| (k, vec![r]))
        .collect::<Vec<_>>();
    if replacements.is_empty() && releases.is_empty() {
        return Ok(());
    }
    let prepared = roll
        .prepare_reference_batch(crate::store::NS, replacements, releases)
        .map_err(|e| invalid(e.to_string()))?;
    let mut ops = Vec::new();
    roll.publish_with(prepared, |rows| ops.extend_from_slice(rows))
        .map_err(|e| invalid(e.to_string()))?;
    for op in ops {
        ctx.persist(op);
    }
    // Keep an accepted root transition even when a later queue refusal reports
    // an error. Cleanup either publishes its own complete inverse or retains
    // the original root for repair; neither may lose its durable counterpart.
    ctx.out.pin_deferred();
    Ok(())
}
pub(super) fn recover(ctx: &mut Ctx<'_>) -> Result<(), BulkError> {
    // Minimal TacSim compositions have no Roll owner and therefore cannot
    // retain author roots. Author commands still require that owner explicitly.
    let Some(roll) = ctx.w.module_state::<RollStore>() else {
        return Ok(());
    };
    let roots = roll
        .reference_keys(crate::store::NS, ReferenceKind::OpenSeal)
        .into_iter()
        .filter(|key| key.record.starts_with(PREFIX))
        .collect::<Vec<_>>();
    for batch in roots.chunks(128) {
        change(ctx, None, batch.to_vec())?;
    }
    Ok(())
}

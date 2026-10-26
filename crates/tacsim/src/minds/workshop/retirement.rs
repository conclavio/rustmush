//! Metadata-only entry to the existing acknowledged Roll retirement action.
use super::commands::roll;
use mush_core::action::PerceivedRef;
use mush_core::{Ctx, Flags, ObjKind};
use mush_roll::commerce::AuthoredRetirementSelection;
use mush_roll::{EditBase, RollError};

/// None means the validated original path may proceed. A marked refusal never
/// falls back to lifecycle publication. No live paid/source authority is read.
pub(super) fn request(
    ctx: &mut Ctx<'_>,
    name: &str,
    expected: Option<EditBase>,
    conn: Option<u64>,
) -> Result<Option<String>, RollError> {
    let actor = ctx.actor;
    let Some(selection) = AuthoredRetirementSelection::capture(roll(ctx)?, actor, name, expected)?
    else {
        return Ok(None);
    };
    let conn = conn
        .or_else(|| ctx.w.dispatching_conn())
        .ok_or(RollError::Missing)?;
    let binding = ctx
        .w
        .connection_binding(conn)
        .filter(|(id, _)| *id == actor)
        .ok_or(RollError::Missing)?;
    if ctx.module.as_deref() != Some(crate::NS)
        || ctx
            .w
            .get(actor)
            .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
    {
        return Err(RollError::Missing);
    }
    if ctx.authoritative_pending(actor) || ctx.authoritative_queue_full() {
        return Err(RollError::Missing);
    }
    let name = selection.name().to_owned();
    let (action, parameters, key) = selection.into_request()?;
    if !ctx
        .reg
        .definitions()
        .is_some_and(|defs| defs.iter().any(|entry| entry.reference() == &action))
        || ctx.w.connection_binding(conn) != Some(binding)
    {
        return Err(RollError::Missing);
    }
    // The immutable dispatch stamp supplies actor; the actual server retains
    // this request through transient preflight/Unknown. No callback mutates Roll.
    let _observation = ctx
        .submit_authoritative_observed(action, PerceivedRef::SelfActor, vec![], parameters, key)
        .map_err(|_| RollError::Missing)?;
    Ok(Some(name))
}

//! **`@registry/inspect <hull>`** (doc 17h cut M8a, A4): the command body
//! `market_commands::registry` dispatches to. It submits `registry_mind_inspection` with
//! `{"hull": <n>}` and prints only `Inspecting hull #<n>.`; the facts reach the operator as
//! the provider's receipt, rendered from the record it wrote (the fix round's F3, the lead's
//! ruling: the driver prints it in place of `Done.`).
//!
//! **The observer rule.** Only the live Org authorization decides who may inspect: an
//! actor the writer does not offer the action to — anyone who is not a configured operator
//! holding `roll.registry.inspect` on the scope — is told the driver's one sentence
//! (`INSPECT_REFUSED`, the driver's own constant), before any hull is looked at, so nothing
//! about hulls reaches them. A hull the provider's read refuses (one the programme does not
//! know, or one with no unit: `facts::read`, the gate the typed action shares) is
//! `No such hull.` to an operator. A composition without the inspection answers
//! `INSPECTION_OFF`, never through the Registry service's own "unavailable" mapping.

use mush_core::{CmdError, Ctx, ObjId, PerceivedRef};
use serde_json::json;

use super::{InspectionHost, facts};
use crate::minds::market_commands::words::{self, fill};

/// The command body. `usage` is `@registry`'s help, the reply to a malformed number.
pub(crate) fn inspect(ctx: &mut Ctx<'_>, usage: &str) -> Result<(), CmdError> {
    let text = ctx.args.raw().trim();
    let text = text.strip_prefix('#').unwrap_or(text);
    let number = crate::minds::market_commands::author::number(text)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| CmdError::msg(usage))?;
    let actor = ctx.actor;
    let hull = ObjId(number);
    let (action, key) = {
        let w: &mush_core::World = &*ctx.w;
        let host = InspectionHost::of(w).ok_or_else(|| CmdError::msg(words::INSPECTION_OFF))?;
        if !host.writer.offered(w, actor) {
            return Err(CmdError::msg(words::INSPECT_REFUSED));
        }
        let mut reader = facts::CtxRead(ctx);
        if facts::read(w, &mut reader, hull).is_none() {
            return Err(CmdError::msg(words::NO_SUCH_HULL));
        }
        if ctx.authoritative_pending(actor) || ctx.authoritative_queue_full() {
            return Err(CmdError::msg(words::BUSY));
        }
        let at = w.world_ic_now().at;
        let key = crate::minds::market_commands::next_key(
            &host.attempts,
            "inspect",
            &number.to_string(),
            at,
        )?;
        (host.writer.action().clone(), key)
    };
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({"hull": number}),
        key,
    );
    ctx.out.info(
        actor,
        fill(words::INSPECTING, &[("hull", &number.to_string())]),
    );
    Ok(())
}

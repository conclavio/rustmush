//! **`@registry/inspect <hull>`** (doc 17h cut M8a, A4): the command body
//! `market_commands::registry` dispatches to. It prints the hull's facts in words and
//! submits `registry_mind_inspection` with `{"hull": <n>}`, which lands the record.
//!
//! **The observer rule.** Only the live Org authorization decides who may inspect: an
//! actor the writer does not offer the action to — anyone who is not a configured operator
//! holding `roll.registry.inspect` on the scope — is told the driver's one sentence
//! (`INSPECT_REFUSED`, the same sentence the server gives a refused submission), before
//! any hull is looked at, so nothing about hulls reaches them. A hull the programme does
//! not know (`known`: the port maps' hull rows, the yards' hulls, the pool and yard rigs)
//! or that has no unit is `No such hull.` to an operator. A composition without the
//! inspection answers
//! `INSPECTION_OFF`, never through the Registry service's own "unavailable" mapping.

use mush_core::{CmdError, Ctx, ObjId, PerceivedRef};
use serde_json::json;

use super::{InspectionHost, facts};
use crate::minds::market_commands::words::{self, fill};

/// **A hull the programme knows**: [`crate::surface::recovery::world_hulls`] (the port maps'
/// hull rows and the yards' hulls) plus every port map's rig rows — the lease pool's
/// (`map.vehicles.rigs`) and the yard's — the terrain vehicles the hull-row set leaves out
/// (the Roll inspects a rig on the pad as readily as a ship).
fn known(w: &mush_core::World, hull: ObjId) -> bool {
    use crate::port::vehicle::{RigRows, rig_rows};
    crate::surface::recovery::world_hulls(w).contains(&hull)
        || w.module_state::<crate::store::TacsimStore>()
            .is_some_and(|store| {
                store.port_maps().any(|map| {
                    [RigRows::Pool, RigRows::Yard]
                        .into_iter()
                        .flat_map(|which| rig_rows(map, which))
                        .any(|rig| rig.tv == hull.0)
                })
            })
}

/// The command body. `usage` is `@registry`'s help, the reply to a malformed number.
pub(crate) fn inspect(ctx: &mut Ctx<'_>, usage: &str) -> Result<(), CmdError> {
    let text = ctx.args.raw().trim();
    let text = text.strip_prefix('#').unwrap_or(text);
    let number = crate::minds::market_commands::author::number(text)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| CmdError::msg(usage))?;
    let actor = ctx.actor;
    let hull = ObjId(number);
    let (lines, action, key) = {
        let w: &mush_core::World = &*ctx.w;
        let host = InspectionHost::of(w).ok_or_else(|| CmdError::msg(words::INSPECTION_OFF))?;
        if !host.writer.offered(w, actor) {
            return Err(CmdError::msg(words::INSPECT_REFUSED));
        }
        if !known(w, hull) {
            return Err(CmdError::msg(words::NO_SUCH_HULL));
        }
        let mut reader = facts::CtxRead(ctx);
        let found =
            facts::read(w, &mut reader, hull).ok_or_else(|| CmdError::msg(words::NO_SUCH_HULL))?;
        let lines = facts::lines(w, &found);
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
        (lines, host.writer.action().clone(), key)
    };
    for line in lines {
        ctx.out.info(actor, line);
    }
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

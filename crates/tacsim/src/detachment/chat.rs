//! A detachment is a roster-scoped IC channel, not a second radio system.
use super::{
    activation::{self, ActivationHandles, ActiveDetachment},
    planning::DraftRef,
};
use mush_core::{CmdError, Ctx};

pub(crate) fn key(roster: &ActiveDetachment) -> String {
    format!(
        "dt:{}:{}:{}",
        roster.organizer.0, roster.draft.0, roster.revision
    )
}
fn unavailable() -> CmdError {
    CmdError::msg(
        "No available detachment chat. Use +sortie/chat [<organizer>:]<draft>:<activation>[=<message>].",
    )
}
/// DT-8a: the organizer of a roster they hold no slot in.
pub(crate) const NOT_A_MEMBER: &str = "Detachment chat is for the roster's slot holders. You organized this detachment but hold no slot in it, so you are not a member; take a slot in a new revision to join its chat.";
pub(crate) fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.iter().count() != 1
        || ctx.args.raw().len() > mush_comsys::transit::MAX_TEXT_BYTES + 96
    {
        return Err(unavailable());
    }
    let raw = ctx.args.raw().trim().to_owned();
    let (selector, message) = raw
        .split_once('=')
        .map_or((raw.as_str(), None), |(s, m)| (s, Some(m)));
    let h = ctx
        .w
        .module_state::<ActivationHandles>()
        .cloned()
        .ok_or_else(unavailable)?;
    let (reference, revision) = if selector.is_empty() {
        let r = ctx
            .action_state(&h.reservations, &ctx.actor)
            .map_err(|_| unavailable())?
            .filter(|r| r.validate().is_ok())
            .ok_or_else(unavailable)?;
        (
            DraftRef {
                organizer: r.organizer,
                id: r.draft,
            },
            r.revision,
        )
    } else {
        let id = super::sortie::operation_id(selector, ctx.actor)?;
        (id.reference(), id.activation)
    };
    let roster = ctx
        .action_state(&h.records, &reference)
        .map_err(|_| unavailable())?
        .filter(|r| r.active() && r.revision == revision && r.validate().is_ok())
        .cloned()
        .ok_or_else(unavailable)?;
    let member = roster
        .plan
        .slots
        .iter()
        .any(|s| s.controller.character() == Some(ctx.actor));
    if !member && ctx.actor != roster.organizer {
        return Err(unavailable());
    }
    // DT-8a: membership is the slot holders' alone (doc 26a); an organizer outside the
    // slots is told so before any channel setup is requested on their behalf.
    if !member {
        return Err(CmdError::msg(NOT_A_MEMBER));
    }
    let channel = mush_comsys::managed::channel(ctx.w, &key(&roster));
    let membership = channel.and_then(|c| {
        ctx.w
            .module_state::<mush_comsys::ComsysState>()?
            .members
            .get(&(ctx.actor, c))
            .cloned()
    });
    if channel.is_none() || membership.is_none() {
        if message.is_some() {
            return Err(CmdError::msg(
                "Open this detachment chat first with +sortie/chat and its roster reference, then send your message.",
            ));
        }
        activation::submit(
            ctx,
            activation::Operation::Chat {
                reference,
                expected_activation: revision,
            },
        )
        .map_err(CmdError::msg)?;
        ctx.out.info(ctx.actor, "Detachment chat setup requested; after it commits, use +sortie/chat again for your alias.");
        return Ok(());
    }
    let channel = channel.ok_or_else(unavailable)?;
    if let Some(message) = message {
        return mush_comsys::api::speak(ctx, channel, message);
    }
    let membership = membership.ok_or_else(unavailable)?;
    let name = mush_comsys::api::channel_name(ctx.w, channel).ok_or_else(unavailable)?;
    ctx.out.info(ctx.actor, format!("Detachment chat [{name}]: {} <message>; {} :<pose>. Recall received lines with @channel/recall {}. Messages use IC transit; queued does not mean received. Orders and WILCO/NEGATIVE remain their separate commands.", membership.alias,membership.alias,membership.alias));
    Ok(())
}

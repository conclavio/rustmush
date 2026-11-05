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
/// **A captive's gag on the detachment's radio** (doc 38a review F-22a; doc 42 Q-42-2): comsys asks
/// it before every IC line ([`mush_comsys::transit::SpeakGate`]), and on a roster channel (a `dt:`
/// key) it answers the crew record's held gate for chat, so the `dt` alias refuses a captive as
/// `+sortie/chat` does. Membership is untouched: a release restores the radio with no write.
#[derive(Debug)]
pub struct DetachmentChatGate;

impl mush_comsys::transit::SpeakGate for DetachmentChatGate {
    fn refusal(
        &self,
        world: &mush_core::World,
        actor: mush_core::ObjId,
        managed: Option<&str>,
    ) -> Option<String> {
        managed.filter(|key| key.starts_with("dt:"))?;
        crate::field::crew::refusal(world, actor, crate::field::crew::Gate::Chat)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::crew::{CREW_FORMAT, CrewBook, CrewCondition, CrewLocation, CrewRecord};
    use mush_comsys::transit::SpeakGate;
    use mush_core::{ObjId, ObjKind, World};

    fn record(who: ObjId, location: CrewLocation) -> CrewRecord {
        CrewRecord {
            format: CREW_FORMAT,
            character: who,
            location,
            condition: CrewCondition::Fit,
            at_ic: 0,
            organizer: None,
            extraction: None,
            treatment: None,
            resolve_permille: None,
        }
    }

    /// **Doc 38a review F-22a** (Q-42-2): on a roster channel the gate refuses a captive with the
    /// held line and passes them once released; it never gags another channel.
    #[test]
    fn review_72h_a_captive_is_gagged_on_the_detachment_radio_only() {
        let mut w = World::new_minimal();
        let who = w
            .create(
                ObjKind::Player,
                "Pilot",
                mush_core::GOD,
                Some(mush_core::LIMBO),
            )
            .expect("a pilot");
        let carrier = w
            .create(
                ObjKind::Thing,
                "Kestrel",
                mush_core::GOD,
                Some(mush_core::LIMBO),
            )
            .expect("a carrier");
        w.module_state_insert(CrewBook::default());
        let gate = DetachmentChatGate;
        assert_eq!(gate.refusal(&w, who, Some("dt:4:8:1")), None, "no record");
        crate::field::crew::hold_for_tests(
            &mut w,
            record(
                who,
                CrewLocation::Captured {
                    carrier,
                    captor: mush_core::GOD,
                    since_ic: 0,
                },
            ),
        );
        let held = gate.refusal(&w, who, Some("dt:4:8:1"));
        assert!(
            held.as_deref()
                .is_some_and(|line| line.contains("held aboard the Kestrel")),
            "{held:?}"
        );
        assert_eq!(gate.refusal(&w, who, None), None, "a public channel");
        assert_eq!(
            gate.refusal(&w, who, Some("guild:7")),
            None,
            "another roster"
        );
        crate::field::crew::hold_for_tests(
            &mut w,
            record(
                who,
                CrewLocation::Port {
                    port: smol_str::SmolStr::new("ceres"),
                },
            ),
        );
        assert_eq!(gate.refusal(&w, who, Some("dt:4:8:1")), None, "released");
    }
}

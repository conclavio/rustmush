//! Speech: `say`, `pose`, `semipose`, `@emit`, `think`, `whisper`, `page`.
//!
//! Speech emits one semantic [`OutputEvent::Narrative`] to the whole room
//! (actor included); renderers own the "You say" vs. "Nick says"
//! distinction. Room speech honors the room's `@lock/speech`. Private
//! lines (`whisper`, `page`) go to the target's actor audience plus a
//! confirmation to the speaker; they are not gated by the speech lock.
//!
//! `@ignore` shaping (silent gag — see [`crate::commands::admin`]): a page
//! or whisper whose target has muted the sender is **dropped here**, while
//! the sender still receives the ordinary delivery confirmation. Room
//! speech from a muted speaker is instead filtered per-listener where the
//! world task resolves `Audience::Room`.

use std::sync::Arc;

use mush_proto::{NarrativeKind, OutputEvent, RichText};

use crate::cmd::{ArgShape, CmdError, CommandSpec, Ctx, Registry, RegistryError};
use crate::flags::{Flags, Role};
use crate::lock::{LockType, MAX_LOCK_DEPTH, eval_lock};
use crate::matchit::match_object;
use crate::obj::ObjId;
use crate::output::{display_name, obj_ref};
use crate::perm;
use crate::world::World;

/// Emit an attributed narrative line to the actor's whole room, honoring
/// the room's `Speech` lock (`@lock/speech`).
fn room_narrative(ctx: &mut Ctx<'_>, kind: NarrativeKind, text: &str) -> Result<(), CmdError> {
    let w = &*ctx.w;
    let loc = w
        .get(ctx.actor)
        .and_then(|o| o.location)
        .ok_or_else(|| CmdError::msg("You are nowhere."))?;
    if let Some(l) = w.get(loc).and_then(|r| r.locks.get(&LockType::Speech))
        && !eval_lock(w, l, ctx.actor, MAX_LOCK_DEPTH)
    {
        return Err(CmdError::msg("You can't speak here."));
    }
    let ev = OutputEvent::Narrative {
        kind,
        actor: Some(obj_ref(w, ctx.actor)),
        text: RichText::plain(text),
    };
    ctx.out.to_room(loc, Vec::new(), ev);
    // Post-mutation observer notification (comsys logging, NPC stimuli).
    ctx.emit_world_event(&crate::module::WorldEvent::Said {
        who: ctx.actor,
        room: loc,
        kind,
        text: text.to_string(),
    });
    Ok(())
}

fn say_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let text = ctx.args.raw().to_string();
    if text.is_empty() {
        return Err(CmdError::msg("Say what?"));
    }
    room_narrative(ctx, NarrativeKind::Say, &text)
}

fn pose_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let text = ctx.args.raw().to_string();
    if text.is_empty() {
        return Err(CmdError::msg("Pose what?"));
    }
    room_narrative(ctx, NarrativeKind::Pose, &text)
}

fn semipose_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let text = ctx.args.raw().to_string();
    if text.is_empty() {
        return Err(CmdError::msg("Pose what?"));
    }
    room_narrative(ctx, NarrativeKind::SemiPose, &text)
}

/// Whether `who` may `@emit`: Royalty+ implicitly, otherwise the `SPOOF`
/// flag. Everything else a player produces (say/pose/whisper/page/OOC/
/// channel lines) carries the true name, so this is the one verb that can
/// put an unattributed line in front of another player.
pub fn can_emit(w: &World, who: ObjId) -> bool {
    perm::has_role(w, who, Role::Royalty)
        || w.get(who).is_some_and(|o| o.flags.contains(Flags::SPOOF))
}

/// The narrative kind an `@emit` from `who` travels as. Wizard+ narration is
/// plain [`NarrativeKind::Emit`] everywhere; below that it is
/// [`NarrativeKind::Spoof`], which the room fan-out downgrades to `Emit` for
/// every listener who is not entitled to see its source (see
/// [`spoof_for_listener`]).
pub fn emit_kind(w: &World, who: ObjId) -> NarrativeKind {
    if perm::has_role(w, who, Role::Wizard) {
        NarrativeKind::Emit
    } else {
        NarrativeKind::Spoof
    }
}

/// Whether `listener` sees the true sender of a spoofed emit: the `NOSPOOF`
/// preference, or Royalty+.
pub fn sees_spoof_source(w: &World, listener: ObjId) -> bool {
    perm::has_role(w, listener, Role::Royalty)
        || w.get(listener)
            .is_some_and(|o| o.flags.contains(Flags::NOSPOOF))
}

/// The event `listener` receives for `event`: a [`NarrativeKind::Spoof`]
/// line keeps its kind (renderers prefix the true sender) only for a
/// listener who [`sees_spoof_source`]; everyone else gets the bare
/// [`NarrativeKind::Emit`]. Any other event passes through untouched.
pub fn spoof_for_listener(w: &World, listener: ObjId, event: &OutputEvent) -> Option<OutputEvent> {
    match event {
        OutputEvent::Narrative {
            kind: NarrativeKind::Spoof,
            actor,
            text,
        } if !sees_spoof_source(w, listener) => Some(OutputEvent::Narrative {
            kind: NarrativeKind::Emit,
            actor: actor.clone(),
            text: text.clone(),
        }),
        _ => None,
    }
}

fn emit_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let text = ctx.args.raw().to_string();
    if text.is_empty() {
        return Err(CmdError::msg("Emit what?"));
    }
    if !can_emit(ctx.w, ctx.actor) {
        return Err(CmdError::msg(
            "Permission denied. @emit needs the SPOOF flag; say and pose always carry your name.",
        ));
    }
    // The actor rides along for spoof-tracking; the kind decides whether a
    // listener's renderer may show it.
    let kind = emit_kind(ctx.w, ctx.actor);
    room_narrative(ctx, kind, &text)
}

fn think_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let text = ctx.args.raw().to_string();
    if text.is_empty() {
        return Err(CmdError::msg("Think what?"));
    }
    ctx.out.to_actor(
        ctx.actor,
        OutputEvent::Narrative {
            kind: NarrativeKind::Emit,
            actor: None,
            text: RichText::plain(text),
        },
    );
    Ok(())
}

fn whisper_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let target_name = ctx.args.lhs().to_string();
    let msg = ctx
        .args
        .rhs()
        .map(str::to_string)
        .filter(|m| !m.is_empty())
        .ok_or_else(|| CmdError::msg("Whisper what?  (whisper <who>=<message>)"))?;
    let target = match_object(ctx.w, ctx.actor, &target_name)?;
    let my_loc = ctx.w.get(ctx.actor).and_then(|o| o.location);
    let their_loc = ctx.w.get(target).and_then(|o| o.location);
    if my_loc.is_none() || my_loc != their_loc {
        return Err(CmdError::msg("They aren't here."));
    }
    // Silent gag: a muted sender's whisper is dropped, but the sender's
    // confirmation below is identical either way.
    if !crate::commands::admin::is_ignoring(ctx.w, target, ctx.actor) {
        let ev = OutputEvent::Narrative {
            kind: NarrativeKind::Whisper,
            actor: Some(obj_ref(ctx.w, ctx.actor)),
            text: RichText::plain(&msg),
        };
        ctx.out.to_actor(target, ev);
    }
    let who = display_name(ctx.w, target);
    ctx.out
        .info(ctx.actor, format!("You whisper \"{msg}\" to {who}."));
    Ok(())
}

/// Resolve a page target: `me`, `#dbref`, or a (possibly `*`-prefixed)
/// player/NPC name — pages are global, so the player index is authoritative.
/// A `#dbref` must name an actor: pages never target rooms or things.
fn page_target(w: &World, actor: ObjId, raw: &str) -> Result<ObjId, CmdError> {
    let q = raw.trim();
    if q.eq_ignore_ascii_case("me") {
        return Ok(actor);
    }
    if let Some(num) = q.strip_prefix('#') {
        return num
            .parse::<u32>()
            .ok()
            .map(crate::obj::ObjId)
            .filter(|&id| w.get(id).is_some_and(|o| o.kind.is_actor()))
            .ok_or_else(|| CmdError::msg("I don't recognize that player."));
    }
    let name = q.strip_prefix('*').unwrap_or(q);
    w.resolve_player(name)
        .ok_or_else(|| CmdError::msg("I don't recognize that player."))
}

fn page_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let target_name = ctx.args.lhs().to_string();
    let msg = ctx
        .args
        .rhs()
        .map(str::to_string)
        .filter(|m| !m.is_empty())
        .ok_or_else(|| CmdError::msg("Page what?  (page <who>=<message>)"))?;
    let target = page_target(ctx.w, ctx.actor, &target_name)?;
    let t = ctx
        .w
        .get(target)
        .ok_or_else(|| CmdError::msg("I don't recognize that player."))?;
    // HAVEN refuses all pages; the Page lock refines who may get through.
    let refused = t.flags.contains(Flags::HAVEN)
        || t.locks
            .get(&LockType::Page)
            .is_some_and(|l| !eval_lock(ctx.w, l, ctx.actor, MAX_LOCK_DEPTH));
    if refused {
        return Err(CmdError::msg("That player does not wish to be disturbed."));
    }
    // Silent gag: a muted sender's page is dropped, but the sender's
    // confirmation below is identical either way ("delivered" semantics).
    if !crate::commands::admin::is_ignoring(ctx.w, target, ctx.actor) {
        let ev = OutputEvent::Narrative {
            kind: NarrativeKind::Page,
            actor: Some(obj_ref(ctx.w, ctx.actor)),
            text: RichText::plain(&msg),
        };
        ctx.out.to_actor(target, ev);
        // Interface plan I11: a page also lands on the notification surface
        // (the toast/tray, the TUI status count) with the reply ready to
        // type — the affordance that matters when the target is on another
        // screen. Same gate as the line itself: never for an ignored sender.
        let sender = display_name(ctx.w, ctx.actor);
        ctx.notice_info(
            target,
            "page",
            format!("{sender} pages: {msg}"),
            Some(("Reply", &format!("page {sender}="))),
        );
    }
    let who = display_name(ctx.w, target);
    ctx.out
        .info(ctx.actor, format!("You paged {who} with '{msg}'."));
    Ok(())
}

/// Register the speech commands.
pub(super) fn register(reg: &mut Registry) -> Result<(), RegistryError> {
    reg.register(
        CommandSpec {
            name: "say".into(),
            help: "say <message>  --  Speak to everyone in your current room. \
                   The shorthand prefix \" does the same: \"Hello = say Hello. \
                   A room's @lock/speech may refuse you."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(say_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "pose".into(),
            help: "pose <action>  --  Emote: everyone in the room sees your name, a \
                   space, and the action (\"pose grins.\" shows as \"Nick grins.\"). \
                   Prefix shorthand: \":grins.\""
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(pose_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "semipose".into(),
            help: "semipose <action>  --  Like pose but with no space after your name \
                   (\"semipose 's eyes narrow.\" shows as \"Nick's eyes narrow.\"). \
                   Prefix shorthand: \";'s eyes narrow.\""
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(semipose_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "@emit".into(),
            help: "@emit <text>  --  Emit free-form narration to your room with no \
                   name prefix. Needs the SPOOF flag (Royalty and Wizards have it). \
                   Below Wizard the true sender shows as a [Name] prefix to staff and \
                   to anyone set NOSPOOF. Prefix shorthand: \"\\<text>\"."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(emit_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "think".into(),
            help: "think <text>  --  Echo text back to yourself only. Nobody else \
                   sees it; handy for notes-to-self."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(think_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "whisper".into(),
            shape: ArgShape::EqPair,
            help: "whisper <who>=<message>  --  Quietly speak to one person in the \
                   same room. Only they (and you) see it."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(whisper_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "page".into(),
            aliases: vec!["p".into()],
            shape: ArgShape::EqPair,
            help: "page <player>=<message>  --  Send a private message to a player \
                   anywhere on the game. A player set HAVEN, or whose page lock you \
                   fail (@lock/page), cannot be paged."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(page_cmd),
    )?;
    Ok(())
}

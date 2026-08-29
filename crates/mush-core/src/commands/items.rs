//! Item handling: `get`/`take`, `drop`, `give`, `inventory`.
//!
//! Lock semantics (TinyMUX-style): a thing's `Basic` lock gates who may
//! pick it up, its `Drop` lock gates dropping, and a recipient's `Give`
//! lock gates receiving. `STICKY` things return home when dropped.

use std::sync::Arc;

use mush_proto::{OutputEvent, RichText};

use crate::cmd::{ArgShape, CmdError, CommandSpec, Ctx, Registry, RegistryError};
use crate::flags::Flags;
use crate::lock::{LockType, MAX_LOCK_DEPTH, eval_lock};
use crate::matchit::match_object;
use crate::obj::ObjKind;
use crate::output::{display_name, name_span};

use super::{attr_text, narrative_emit};

fn get_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let q = ctx.args.raw().to_string();
    if q.is_empty() {
        return Err(CmdError::msg("Get what?"));
    }
    let target = match_object(ctx.w, ctx.actor, &q)?;
    let o = ctx
        .w
        .get(target)
        .ok_or_else(|| CmdError::msg("I don't see that here."))?;
    if o.location == Some(ctx.actor) {
        return Err(CmdError::msg("You already have that!"));
    }
    match o.kind {
        ObjKind::Thing => {}
        ObjKind::Player | ObjKind::Npc => {
            return Err(CmdError::msg("You can't pick that up!"));
        }
        ObjKind::Exit | ObjKind::Room => {
            return Err(CmdError::msg("You can't take that!"));
        }
    }
    let my_loc = ctx.w.get(ctx.actor).and_then(|a| a.location);
    if o.location != my_loc || my_loc.is_none() {
        return Err(CmdError::msg("I don't see that here."));
    }
    // The Basic lock on a thing is its take lock.
    if let Some(l) = o.locks.get(&LockType::Basic)
        && !eval_lock(ctx.w, l, ctx.actor, MAX_LOCK_DEPTH)
    {
        let fail =
            attr_text(ctx.w, target, "fail").unwrap_or_else(|| "You can't pick that up.".into());
        return Err(CmdError::msg(fail));
    }
    ctx.w.move_to(target, ctx.actor)?;
    let name = display_name(ctx.w, target);
    let succ = attr_text(ctx.w, target, "succ").unwrap_or_else(|| format!("You take {name}."));
    ctx.out.info(ctx.actor, succ);
    if let Some(loc) = my_loc {
        let osucc = attr_text(ctx.w, target, "osucc")
            .map_or_else(|| format!(" takes {name}."), |m| format!(" {m}"));
        let text = RichText::new()
            .span(name_span(ctx.w, ctx.actor))
            .text(osucc);
        ctx.out
            .to_room_from(loc, vec![ctx.actor], ctx.actor, narrative_emit(text));
    }
    Ok(())
}

fn drop_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let q = ctx.args.raw().to_string();
    if q.is_empty() {
        return Err(CmdError::msg("Drop what?"));
    }
    let target = match_object(ctx.w, ctx.actor, &q)?;
    let o = ctx
        .w
        .get(target)
        .ok_or_else(|| CmdError::msg("You don't have that!"))?;
    if o.location != Some(ctx.actor) {
        return Err(CmdError::msg("You don't have that!"));
    }
    if let Some(l) = o.locks.get(&LockType::Drop)
        && !eval_lock(ctx.w, l, ctx.actor, MAX_LOCK_DEPTH)
    {
        return Err(CmdError::msg("You can't drop that."));
    }
    let loc = ctx
        .w
        .get(ctx.actor)
        .and_then(|a| a.location)
        .ok_or_else(|| CmdError::msg("You are nowhere."))?;
    // STICKY things return to their home when dropped.
    let sticky_home = o
        .flags
        .contains(Flags::STICKY)
        .then_some(o.home)
        .flatten()
        .filter(|&h| ctx.w.get(h).is_some());
    let dest = sticky_home.unwrap_or(loc);
    ctx.w.move_to(target, dest)?;
    let name = display_name(ctx.w, target);
    let msg = attr_text(ctx.w, target, "drop").unwrap_or_else(|| format!("You drop {name}."));
    ctx.out.info(ctx.actor, msg);
    let odrop = attr_text(ctx.w, target, "odrop")
        .map_or_else(|| format!(" drops {name}."), |m| format!(" {m}"));
    let text = RichText::new()
        .span(name_span(ctx.w, ctx.actor))
        .text(odrop);
    ctx.out
        .to_room_from(loc, vec![ctx.actor], ctx.actor, narrative_emit(text));
    Ok(())
}

fn give_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let obj_name = ctx.args.lhs().to_string();
    let who_name = ctx
        .args
        .rhs()
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("Give what to whom?  (give <object>=<recipient>)"))?;
    if obj_name.is_empty() {
        return Err(CmdError::msg(
            "Give what to whom?  (give <object>=<recipient>)",
        ));
    }
    let obj = match_object(ctx.w, ctx.actor, &obj_name)?;
    if ctx.w.get(obj).and_then(|o| o.location) != Some(ctx.actor) {
        return Err(CmdError::msg("You don't have that!"));
    }
    let recipient = match_object(ctx.w, ctx.actor, &who_name)?;
    if recipient == ctx.actor {
        return Err(CmdError::msg("You already have that!"));
    }
    let r = ctx
        .w
        .get(recipient)
        .ok_or_else(|| CmdError::msg("They aren't here."))?;
    let my_loc = ctx.w.get(ctx.actor).and_then(|a| a.location);
    if r.location != my_loc || my_loc.is_none() {
        return Err(CmdError::msg("They aren't here."));
    }
    // Recipients: actors always, things only when ENTER_OK (vending seams).
    let receivable =
        r.kind.is_actor() || (r.kind == ObjKind::Thing && r.flags.contains(Flags::ENTER_OK));
    if !receivable {
        return Err(CmdError::msg("You can't give things to that."));
    }
    if let Some(l) = r.locks.get(&LockType::Give)
        && !eval_lock(ctx.w, l, ctx.actor, MAX_LOCK_DEPTH)
    {
        return Err(CmdError::msg("They don't want that."));
    }
    ctx.w.move_to(obj, recipient)?;
    let obj_disp = display_name(ctx.w, obj);
    let who_disp = display_name(ctx.w, recipient);
    ctx.out
        .info(ctx.actor, format!("You give {obj_disp} to {who_disp}."));
    let text = RichText::new()
        .span(name_span(ctx.w, ctx.actor))
        .text(format!(" gives you {obj_disp}."));
    ctx.out.to_actor(recipient, narrative_emit(text));
    if let Some(loc) = my_loc {
        let text = RichText::new()
            .span(name_span(ctx.w, ctx.actor))
            .text(format!(" gives {obj_disp} to "))
            .span(name_span(ctx.w, recipient))
            .text(".");
        ctx.out.to_room_from(
            loc,
            vec![ctx.actor, recipient],
            ctx.actor,
            narrative_emit(text),
        );
    }
    Ok(())
}

fn inventory_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let carried: Vec<_> = ctx
        .w
        .contents_of(ctx.actor)
        .iter()
        .copied()
        .filter(|&c| {
            ctx.w
                .get(c)
                .is_some_and(|o| !o.flags.contains(Flags::GOING))
        })
        .collect();
    if carried.is_empty() {
        ctx.out.info(ctx.actor, "You aren't carrying anything.");
        return Ok(());
    }
    ctx.out.info(ctx.actor, "You are carrying:");
    for c in carried {
        let text = RichText::new().span(name_span(ctx.w, c));
        ctx.out.to_actor(ctx.actor, OutputEvent::Sys(text));
    }
    Ok(())
}

/// Register the item commands.
pub(super) fn register(reg: &mut Registry) -> Result<(), RegistryError> {
    reg.register(
        CommandSpec {
            name: "get".into(),
            aliases: vec!["take".into()],
            help: "get <thing>  --  Pick up a thing in your current room (alias: \
                   take). The thing's Basic lock must admit you; its succ/osucc \
                   attributes override the default take messages. You can't pick \
                   up players, exits, or rooms."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(get_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "drop".into(),
            help: "drop <thing>  --  Put down something you are carrying. A Drop \
                   lock (@lock/drop) can forbid it; a STICKY thing returns to its \
                   home instead of landing in the room."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(drop_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "give".into(),
            shape: ArgShape::EqPair,
            help: "give <thing>=<recipient>  --  Hand something you carry to a \
                   player or NPC in the same room (or an ENTER_OK thing). The \
                   recipient's Give lock (@lock/give) may refuse it."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(give_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "inventory".into(),
            aliases: vec!["i".into(), "inv".into()],
            shape: ArgShape::None,
            help: "inventory  --  List everything you are carrying (aliases: i, inv).".into(),
            ..CommandSpec::default()
        },
        Arc::new(inventory_cmd),
    )?;
    Ok(())
}

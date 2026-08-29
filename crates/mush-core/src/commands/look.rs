//! Looking and examining: `look` (rooms, objects, `detail:` keywords) and
//! `examine` (full attribute dump, gated by [`perm::can_examine`]).

use std::sync::Arc;

use mush_proto::{OutputEvent, RichText};

use crate::attr::{AttrFlags, AttrValue};
use crate::cmd::{CmdError, CommandSpec, Ctx, Registry, RegistryError};
use crate::flags::{Flags, Role};
use crate::matchit::{MatchErr, match_object};
use crate::module::can_perceive;
use crate::obj::{ObjId, ObjKind};
use crate::output::{Outbox, name_span};
use crate::perm;
use crate::world::World;

use super::{attr_text, show_room};

fn look_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let q = ctx.args.raw().to_string();
    let loc = ctx.w.get(ctx.actor).and_then(|o| o.location);
    if q.is_empty() {
        let loc = loc.ok_or_else(|| CmdError::msg("You are nowhere."))?;
        if ctx.w.get(loc).is_none() {
            return Err(CmdError::msg("You are nowhere."));
        }
        show_room(ctx.reg, ctx.w, ctx.actor, loc, ctx.out);
        return Ok(());
    }
    match match_object(ctx.w, ctx.actor, &q) {
        // M11 per-observer projection: a target the looker cannot perceive
        // (a cloaked vampire) resolves to nothing — true silence, so `look
        // <name>` and `look #dbref` cannot probe a concealed subject. The
        // looker always perceives themselves. Free pass with no Perceive hook,
        // so this is identical to today for a game without concealment.
        Ok(target)
            if target == ctx.actor || can_perceive(ctx.reg, ctx.w, ctx.actor, target) =>
        {
            look_at(ctx.reg, ctx.w, ctx.actor, target, ctx.out);
            Ok(())
        }
        // A matched-but-imperceptible target falls through exactly like a name
        // that matched nothing: the `detail:` keyword fallback, then NoMatch.
        Ok(_) | Err(MatchErr::NoMatch) => {
            // `look <keyword>`: room details stored as `detail:<keyword>`
            // attributes on the room (inheritable), then on the looker.
            let attr = format!("detail:{}", q.trim());
            let detail = loc
                .and_then(|l| attr_text(ctx.w, l, &attr))
                .or_else(|| attr_text(ctx.w, ctx.actor, &attr));
            match detail {
                Some(text) => {
                    ctx.out
                        .to_actor(ctx.actor, OutputEvent::Sys(RichText::plain(text)));
                    Ok(())
                }
                None => Err(MatchErr::NoMatch.into()),
            }
        }
        Err(e) => Err(e.into()),
    }
}

/// Show one object to a looker: rooms render as a full [`RoomView`];
/// everything else shows its desc plus (for non-`OPAQUE` **things**)
/// contents. An actor's carry is private — `look <player>` never lists
/// another character's inventory unless the looker controls them
/// (`inventory` is how you see your own).
fn look_at(reg: &Registry, w: &World, looker: ObjId, target: ObjId, out: &mut Outbox) {
    let Some(o) = w.get(target) else { return };
    if o.kind == ObjKind::Room {
        show_room(reg, w, looker, target, out);
        return;
    }
    let desc =
        attr_text(w, target, "desc").unwrap_or_else(|| "You see nothing special.".to_string());
    out.to_actor(
        looker,
        OutputEvent::Sys(
            RichText::new()
                .span(name_span(w, target))
                .text(format!(" -- {desc}")),
        ),
    );
    // Visible contents of things (a shop shelf); actors' carry only shows
    // to whoever controls them.
    let contents_visible = !o.contents.is_empty()
        && !o.flags.contains(Flags::OPAQUE)
        && (o.kind == ObjKind::Thing || perm::controls(w, looker, target));
    if contents_visible {
        let mut text = RichText::plain("Contents:");
        for &c in &o.contents {
            if w.get(c)
                .is_some_and(|co| !co.flags.intersects(Flags::GOING | Flags::DARK))
                // M11 per-observer projection: never list an occupant the
                // looker cannot perceive (free pass with no Perceive hook).
                && can_perceive(reg, w, looker, c)
            {
                text = text.text(" ").span(name_span(w, c));
            }
        }
        out.to_actor(looker, OutputEvent::Sys(text));
    }
}

fn examine_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let q = ctx.args.raw().to_string();
    let target = match_object(ctx.w, ctx.actor, if q.is_empty() { "me" } else { &q })?;
    // M11 per-observer projection: a concealed target the examiner cannot
    // perceive is unresolvable — the same NoMatch a non-existent name gives, so
    // `examine <name>`/`examine #dbref` cannot confirm a cloaked subject's
    // presence (even for a controller with no piercing sense). Self is always
    // perceivable; free pass with no Perceive hook registered.
    if target != ctx.actor && !can_perceive(ctx.reg, ctx.w, ctx.actor, target) {
        return Err(MatchErr::NoMatch.into());
    }
    let w = &*ctx.w;
    let o = w.get(target).ok_or(MatchErr::NoMatch)?;
    let header = RichText::new()
        .span(name_span(w, target))
        .text(format!("({target})"));

    if !perm::can_examine(w, ctx.actor, target) {
        // Classic public view: name and owner only.
        let owner = w.get(o.owner).map_or_else(
            || o.owner.to_string(),
            |ow| format!("{}({})", ow.name, o.owner),
        );
        ctx.out.to_actor(
            ctx.actor,
            OutputEvent::Sys(header.text(format!(" is owned by {owner}."))),
        );
        return Ok(());
    }

    ctx.out.to_actor(ctx.actor, OutputEvent::Sys(header));
    let mut flag_names: Vec<&str> = o.flags.iter_names().map(|(n, _)| n).collect();
    flag_names.extend(o.markers.iter().map(smol_str::SmolStr::as_str));
    ctx.out.info(
        ctx.actor,
        format!(
            "Type: {:?}  Flags: {}",
            o.kind,
            if flag_names.is_empty() {
                "(none)".to_string()
            } else {
                flag_names.join(" ")
            }
        ),
    );
    ctx.out.info(
        ctx.actor,
        format!(
            "Owner: {}  Zone: {}  Parent: {}",
            fmt_ref(w, Some(o.owner)),
            fmt_ref(w, o.zone),
            fmt_ref(w, o.parent)
        ),
    );
    ctx.out.info(
        ctx.actor,
        format!(
            "Location: {}  Home: {}  Destination: {}",
            fmt_ref(w, o.location),
            fmt_ref(w, o.home),
            fmt_ref(w, o.destination)
        ),
    );
    if !o.locks.is_empty() {
        for (lt, expr) in &o.locks {
            ctx.out.info(ctx.actor, format!("Lock/{lt}: {expr}"));
        }
    }
    let examiner_is_wizard = perm::has_role(w, ctx.actor, Role::Wizard);
    let show_hidden = ctx.switches.has("all");
    for (name, attr) in o.attrs.iter() {
        if attr.flags.contains(AttrFlags::WIZARD_ONLY) && !examiner_is_wizard {
            continue;
        }
        // SELF_ONLY: readable only by the object itself and staff.
        if attr.flags.contains(AttrFlags::SELF_ONLY) && ctx.actor != target && !examiner_is_wizard {
            continue;
        }
        // HIDDEN: withheld from the normal dump; examine/all shows it.
        if attr.flags.contains(AttrFlags::HIDDEN) && !show_hidden {
            continue;
        }
        let value = match &attr.value {
            AttrValue::Text(s) => s.clone(),
            AttrValue::Json(v) => v.to_string(),
        };
        ctx.out
            .info(ctx.actor, format!("{name} [{}]: {value}", attr.owner));
    }
    // M11 per-observer projection: `examine` dumps raw contents (offline
    // players included), so it must also drop occupants the examiner cannot
    // perceive, or a concealed subject would leak here. Free pass — hence
    // byte-identical output — with no Perceive hook registered.
    let visible: Vec<ObjId> = o
        .contents
        .iter()
        .copied()
        .filter(|&c| can_perceive(ctx.reg, w, ctx.actor, c))
        .collect();
    if !visible.is_empty() {
        let mut text = RichText::plain("Contents:");
        for c in visible {
            text = text.text(" ").span(name_span(w, c));
        }
        ctx.out.to_actor(ctx.actor, OutputEvent::Sys(text));
    }
    Ok(())
}

fn fmt_ref(w: &World, id: Option<ObjId>) -> String {
    match id {
        None => "*NOTHING*".to_string(),
        Some(id) => w
            .get(id)
            .map_or_else(|| id.to_string(), |o| format!("{}({id})", o.name)),
    }
}

/// Register `look` and `examine`.
pub(super) fn register(reg: &mut Registry) -> Result<(), RegistryError> {
    reg.register(
        CommandSpec {
            name: "look".into(),
            aliases: vec!["l".into()],
            help: "look [<target>]  --  With no argument, show your current room: \
                   description, visible contents, and exits (DARK and OPAQUE are \
                   honored). With a target, show that object's description; \
                   \"look <keyword>\" also finds room details stored in \
                   detail:<keyword> attributes."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(look_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "examine".into(),
            aliases: vec!["ex".into()],
            switches: vec!["all".into()],
            help: "examine[/all] [<target>]  --  Full technical view of an object \
                   you control (or may examine): id, type, flags, owner, zone, \
                   parent, locks, and attributes. Defaults to yourself. HIDDEN \
                   attributes only show with /all; SELF_ONLY attributes show only \
                   to the object itself and staff. On objects you may not examine, \
                   only the owner is shown."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(examine_cmd),
    )?;
    Ok(())
}

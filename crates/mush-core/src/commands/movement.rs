//! Movement: exit traversal (used directly by the dispatch pipeline),
//! `goto`/`go`/`move`, and `home`.
//!
//! Exit lookup order is room exits → the room's zone's exits → global
//! exits on Limbo (the master room), matching treaty ruling 5. Traversal
//! honors the exit's `Basic` lock and the classic move-message attributes:
//! `succ`/`osucc` on success, `fail`/`ofail` on a failed lock, and an
//! arrival line in the destination (`odrop` overrides the default).

use std::sync::Arc;

use mush_proto::{OutputEvent, RichText};

use crate::cmd::{ArgShape, CmdError, CommandSpec, Ctx, Registry, RegistryError};
use crate::flags::Flags;
use crate::lock::{LockType, MAX_LOCK_DEPTH, eval_lock_with};
use crate::module::{WorldEvent, check_move_attempt, emit_world_event};
use crate::obj::{LIMBO, Obj, ObjId};
use crate::object_read::{ObjectRead, PlainRead};
use crate::output::{Outbox, name_span, primary_name};
use crate::world::World;

use super::{attr_text, narrative_emit, show_room};

/// Find an exit matching `name` from `actor`'s position: the room's own
/// exits, then exits attached to the room's Zone Master Object, then
/// global exits on Limbo. Matching is exact (case-insensitive) over
/// `;`-segments and aliases.
#[must_use]
pub fn find_exit(w: &World, actor: ObjId, name: &str) -> Option<ObjId> {
    match find_exit_with(&mut PlainRead::new(w), actor, name) {
        Ok(found) => found,
        Err(never) => match never {},
    }
}

/// The same ordered lookup through an explicit reader. A proof reader records
/// both absent and nonmatching candidates, as well as each holder's exit list.
pub(crate) fn find_exit_with<'w, R: ObjectRead<'w>>(
    reader: &mut R,
    actor: ObjId,
    name: &str,
) -> Result<Option<ObjId>, R::Error> {
    let Some(loc) = reader.object(actor)?.and_then(|object| object.location) else {
        return Ok(None);
    };
    if let Some(exit) = exit_in_with(reader, loc, name)? {
        return Ok(Some(exit));
    }
    if let Some(zone) = reader.object(loc)?.and_then(|room| room.zone)
        && let Some(exit) = exit_in_with(reader, zone, name)?
    {
        return Ok(Some(exit));
    }
    if loc != LIMBO {
        return exit_in_with(reader, LIMBO, name);
    }
    Ok(None)
}

fn exit_in_with<'w, R: ObjectRead<'w>>(
    reader: &mut R,
    holder: ObjId,
    query: &str,
) -> Result<Option<ObjId>, R::Error> {
    let Some(holder) = reader.object(holder)? else {
        return Ok(None);
    };
    for exit in &holder.exits {
        let Some(object) = reader.object(*exit)? else {
            continue;
        };
        if object.flags.contains(Flags::GOING) {
            continue;
        }
        if exit_name_matches_with_work(object, query, |units| reader.charge_work(units))? {
            return Ok(Some(*exit));
        }
    }
    Ok(None)
}

/// Whether exit `o` answers to `q`: exact (case-insensitive) over its
/// `;`-segments and aliases. Shared with `@macro`'s exit note.
pub(super) fn exit_name_matches(o: &Obj, q: &str) -> bool {
    match exit_name_matches_with_work(o, q, |_| Ok::<(), std::convert::Infallible>(())) {
        Ok(matches) => matches,
        Err(never) => match never {},
    }
}

fn exit_name_matches_with_work<E>(
    object: &Obj,
    query: &str,
    mut charge: impl FnMut(usize) -> Result<(), E>,
) -> Result<bool, E> {
    charge(
        object
            .name
            .len()
            .saturating_add(query.len())
            .saturating_add(1),
    )?;
    if object
        .name
        .split(';')
        .map(str::trim)
        .any(|segment| segment.eq_ignore_ascii_case(query))
    {
        return Ok(true);
    }
    for alias in &object.aliases {
        charge(alias.len().saturating_add(1))?;
        if alias.eq_ignore_ascii_case(query) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Move `actor` through `exit`: check the exit's `Basic` lock, consult
/// module `MoveAttempt` hooks (veto-capable), relocate, and — only once
/// the move has actually succeeded — emit the classic move messages, show
/// the destination room (with `LookRendered` hook lines), and notify
/// `Moved` observers. Observers never see a departure narrative for a
/// move that failed (e.g. a destination destroyed after the exit was
/// linked).
///
/// `lens` is the caller's typed-state reach: a dispatched command passes
/// `ctx.typed_state.lens()`, so the `MoveAttempt` veto chain and the arrival
/// render read the same live typed state the command does.
pub fn traverse_exit(
    reg: &Registry,
    w: &mut World,
    actor: ObjId,
    exit: ObjId,
    out: &mut Outbox,
    lens: crate::action_state::TypedStateLens<'_>,
) {
    let Some(e) = w.get(exit) else { return };
    let exit_name = primary_name(&e.name).to_string();
    let dest = e.destination;
    let from = w.get(actor).and_then(|o| o.location);

    if let Some(fail) = exit_lock_refusal(w, actor, exit) {
        out.error(actor, fail);
        if let Some(fr) = from
            && let Some(of) = attr_text(w, exit, "ofail")
        {
            let text = RichText::new()
                .span(name_span(w, actor))
                .text(format!(" {of}"));
            out.to_room_from(fr, vec![actor], actor, narrative_emit(text));
        }
        return;
    }

    let Some(dest) = dest else {
        out.error(actor, "That exit doesn't lead anywhere.");
        return;
    };

    // Module veto (tactics contested ground, wod barriers): the reason is shown
    // to the mover, the room sees nothing.
    if let Err(reason) = check_move_attempt(reg, w, actor, Some(exit), dest, lens) {
        out.to_actor(actor, OutputEvent::Sys(reason));
        return;
    }

    // Move first; announce only what actually happened (MUSH voice, never
    // a raw internal error).
    if w.move_to(actor, dest).is_err() {
        out.error(actor, "You can't go that way.");
        return;
    }
    if let Some(fr) = from {
        let leave = attr_text(w, exit, "osucc")
            .map_or_else(|| format!(" goes {exit_name}."), |m| format!(" {m}"));
        let text = RichText::new().span(name_span(w, actor)).text(leave);
        out.to_room_from(fr, vec![actor], actor, narrative_emit(text));
    }
    if let Some(s) = attr_text(w, exit, "succ") {
        out.info(actor, s);
    }
    let arrive = attr_text(w, exit, "odrop")
        .map_or_else(|| " has arrived.".to_string(), |m| format!(" {m}"));
    let text = RichText::new().span(name_span(w, actor)).text(arrive);
    out.to_room_from(dest, vec![actor], actor, narrative_emit(text));
    show_room(reg, w, actor, dest, out, lens);
    emit_world_event(
        reg,
        w,
        out,
        &WorldEvent::Moved {
            who: actor,
            from,
            to: dest,
        },
        lens,
    );
}

/// The exit's `Basic` lock, refusing `actor`: its `fail` text (`You can't go
/// that way.` when the exit has none). `None` when there is no lock or it
/// passes.
fn exit_lock_refusal(w: &World, actor: ObjId, exit: ObjId) -> Option<String> {
    let allowed = match exit_lock_allows_with(&mut PlainRead::new(w), actor, exit) {
        Ok(allowed) => allowed,
        Err(never) => match never {},
    };
    if allowed {
        return None;
    }
    Some(attr_text(w, exit, "fail").unwrap_or_else(|| "You can't go that way.".to_string()))
}

/// Basic-lock policy shared with inert admission. A missing object or unset
/// lock retains the existing unlocked result; route resolution is separate.
pub(crate) fn exit_lock_allows_with<'w, R: ObjectRead<'w>>(
    reader: &mut R,
    actor: ObjId,
    exit: ObjId,
) -> Result<bool, R::Error> {
    let Some(lock) = reader
        .object(exit)?
        .and_then(|object| object.locks.get(&LockType::Basic))
    else {
        return Ok(true);
    };
    eval_lock_with(reader, lock, actor, MAX_LOCK_DEPTH)
}

/// Why a traversal was refused (S8.1, 14a #1): the exit's own lock, or a
/// module's veto.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraversalRefusal {
    /// The exit's `Basic` lock refused the mover: its `fail` text.
    Locked(String),
    /// A `MoveAttempt` hook vetoed the move: its reason.
    Vetoed(RichText),
}

impl TraversalRefusal {
    /// The refusal as the mover reads it.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            TraversalRefusal::Locked(text) => text.clone(),
            TraversalRefusal::Vetoed(rich) => rich.to_plain_text(),
        }
    }
}

/// Authorize `actor` moving to `dest` through `exit` — or through none, a
/// teleport-shaped move — exactly as [`traverse_exit`] does before it relocates
/// anyone: the exit's `Basic` lock, then the `MoveAttempt` veto chain. This is
/// the one gate: a tactical step across a room boundary (S8.1, 14a #1) asks it
/// through [`Ctx::authorize_traversal`] with the portal's exit, so a locked
/// doorway is locked on the map as it is in prose.
///
/// # Errors
///
/// The lock's `fail` text, or the first hook's reason.
pub fn authorize_traversal(
    reg: &Registry,
    w: &World,
    actor: ObjId,
    exit: Option<ObjId>,
    dest: ObjId,
    lens: crate::action_state::TypedStateLens<'_>,
) -> Result<(), TraversalRefusal> {
    if let Some(exit) = exit
        && let Some(fail) = exit_lock_refusal(w, actor, exit)
    {
        return Err(TraversalRefusal::Locked(fail));
    }
    check_move_attempt(reg, w, actor, exit, dest, lens).map_err(TraversalRefusal::Vetoed)
}

/// Send `actor` home (the `home` keyword / command). Consults the
/// `MoveAttempt` hooks like every other move — a vetoed destination (a
/// locked battle hex, a warded room) is vetoed even when it happens to be
/// someone's home.
/// `lens` is the caller's typed-state reach — see [`traverse_exit`].
pub fn go_home(
    reg: &Registry,
    w: &mut World,
    actor: ObjId,
    out: &mut Outbox,
    lens: crate::action_state::TypedStateLens<'_>,
) {
    let Some(home) = w.get(actor).and_then(|o| o.home) else {
        out.error(actor, "You have no home.");
        return;
    };
    if w.get(home).is_none() {
        out.error(actor, "Your home is gone.");
        return;
    }
    let from = w.get(actor).and_then(|o| o.location);
    if from == Some(home) {
        out.info(actor, "You are already home.");
        return;
    }
    // Module veto (`exit: None` — home is not an exit traversal).
    if let Err(reason) = check_move_attempt(reg, w, actor, None, home, lens) {
        out.to_actor(actor, OutputEvent::Sys(reason));
        return;
    }
    // Move first; announce only on success (never a raw internal error).
    if w.move_to(actor, home).is_err() {
        out.error(actor, "You can't go home right now.");
        return;
    }
    out.info(actor, "There's no place like home...");
    if let Some(fr) = from {
        let text = RichText::new()
            .span(name_span(w, actor))
            .text(" goes home.");
        out.to_room_from(fr, vec![actor], actor, narrative_emit(text));
    }
    let text = RichText::new()
        .span(name_span(w, actor))
        .text(" arrives home.");
    out.to_room_from(home, vec![actor], actor, narrative_emit(text));
    show_room(reg, w, actor, home, out, lens);
    emit_world_event(
        reg,
        w,
        out,
        &WorldEvent::Moved {
            who: actor,
            from,
            to: home,
        },
        lens,
    );
}

fn goto_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let dir = ctx.args.raw().to_string();
    if dir.is_empty() {
        return Err(CmdError::msg("Go where?"));
    }
    if dir.eq_ignore_ascii_case("home") {
        go_home(ctx.reg, ctx.w, ctx.actor, ctx.out, ctx.typed_state.lens());
        return Ok(());
    }
    match find_exit(ctx.w, ctx.actor, &dir) {
        Some(e) => {
            traverse_exit(
                ctx.reg,
                ctx.w,
                ctx.actor,
                e,
                ctx.out,
                ctx.typed_state.lens(),
            );
            Ok(())
        }
        None => Err(CmdError::msg("You can't go that way.")),
    }
}

fn home_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    go_home(ctx.reg, ctx.w, ctx.actor, ctx.out, ctx.typed_state.lens());
    Ok(())
}

/// Register `goto` (aliases `go`, `move`) and `home`.
pub(super) fn register(reg: &mut Registry) -> Result<(), RegistryError> {
    reg.register(
        CommandSpec {
            name: "goto".into(),
            aliases: vec!["go".into(), "move".into()],
            help: "goto <exit>  --  Move through a named exit, exactly as typing the \
                   exit's name would. Exits are searched in your current room, then on \
                   the room's zone object, then among global exits; the exit's Basic \
                   lock must admit you. \"goto home\" is the same as \"home\"."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(goto_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "home".into(),
            shape: ArgShape::None,
            help: "home  --  Return to your home location (set with \"@link me=here\"). \
                   The word HOME is also recognized bare, before exit matching, so an \
                   exit can never shadow it."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(home_cmd),
    )?;
    Ok(())
}

//! The `@`-building suite: `@create`, `@dig`, `@open`, `@link`, `@desc`,
//! `@name`, `@set`, `@lock`/`@unlock`, `@teleport`, `@zone`, `@parent`,
//! `@chown`, `@destroy`, `@find`.
//!
//! Every handler routes authority through [`perm`] (never inline flag
//! math) and reports in classic MUSH voice ("Set.", "Opened.",
//! "Permission denied.").

use std::sync::Arc;

use mush_proto::{OutputEvent, RichText};

use crate::attr::{Attr, AttrFlags, AttrValue};
use crate::cmd::{ArgShape, CmdError, CommandSpec, Ctx, Registry, RegistryError};
use crate::flags::{Flags, Powers, Role};
use crate::lock::{LockType, parse_lock};
use crate::matchit::match_object;
use crate::module::WorldEvent;
use crate::obj::{ObjId, ObjKind};
use crate::output::{name_span, primary_name};
use crate::perm;
use crate::world::World;

use super::narrative_emit;

/// Lock-type names valid as `/switches` on `@lock` and `@unlock`.
///
/// Deliberately limited to the lock types some command actually
/// **enforces** in M0 — basic (exits/pickup), drop, give, page, speech,
/// teleport, zone. `enter`/`use`/`mail` exist in [`LockType`] but are not
/// offered until their commands land (M3+): a lock you can set but nothing
/// checks would be a silent no-op security control.
const LOCK_SWITCHES: &[&str] = &[
    "basic", "drop", "give", "page", "speech", "teleport", "zone",
];

/// Convert a `&str` switch table into the owned form [`CommandSpec`] wants.
fn switch_vec(switches: &[&str]) -> Vec<smol_str::SmolStr> {
    switches
        .iter()
        .copied()
        .map(smol_str::SmolStr::new)
        .collect()
}

fn perm_denied() -> CmdError {
    CmdError::msg("Permission denied.")
}

/// **An exit may not take a word the kernel or the host owns** (16c `B-75`, cut AQ;
/// the owner's ruling of 2026-10-18). Dispatch matches exits before the command
/// table, so an exit named or aliased `l`, `home` or `inv` takes that word from
/// every player in its room. Pack apply has refused such a segment since 16b cut
/// D.5 (`mush-atlas`'s `check_exit_names`); `@dig`, `@open` and `@name` refuse it
/// here, in the same words, through the same seam — [`Registry::kernel_command_named`],
/// which answers for the core command set and the host's own verbs and never for a
/// module's (a module's verb is contextual and an exit may take it, as cut F.4
/// decided for `board`). Every `;`-separated segment of `spec` is checked, the
/// primary name included; blank segments are skipped, as [`open_exit`] skips them.
fn check_exit_words(reg: &Registry, spec: &str) -> Result<(), CmdError> {
    for segment in spec.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(command) = reg.kernel_command_named(segment) {
            return Err(CmdError::msg(format!(
                "An exit may not be called '{segment}': that is the command '{command}', \
                 which works in every room, and the exit would swallow it."
            )));
        }
    }
    Ok(())
}

/// Create an exit named by `spec` (`"north;n;nor"`) from `src`, owned by
/// `owner`, optionally linked to `dest`. Returns the new exit. The caller has
/// run [`check_exit_words`] over `spec` first.
fn open_exit(
    w: &mut World,
    owner: ObjId,
    src: ObjId,
    spec: &str,
    dest: Option<ObjId>,
) -> Result<ObjId, CmdError> {
    let mut segs = spec.split(';').map(str::trim).filter(|s| !s.is_empty());
    let name = segs
        .next()
        .ok_or_else(|| CmdError::msg("Open which exit?"))?
        .to_string();
    let aliases: Vec<String> = segs.map(str::to_string).collect();
    let exit = w.create(ObjKind::Exit, &name, owner, Some(src))?;
    for a in aliases {
        w.add_alias(exit, &a)?;
    }
    if let Some(d) = dest {
        w.link_exit(exit, Some(d))?;
    }
    Ok(exit)
}

fn create_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let name = ctx.args.raw().trim().to_string();
    if name.is_empty() {
        return Err(CmdError::msg("Create what?"));
    }
    let thing = ctx
        .w
        .create(ObjKind::Thing, &name, ctx.actor, Some(ctx.actor))?;
    ctx.out
        .info(ctx.actor, format!("Created: {name}({thing})."));
    ctx.emit_world_event(&WorldEvent::ObjCreated {
        obj: thing,
        kind: ObjKind::Thing,
        creator: ctx.actor,
    });
    Ok(())
}

fn dig_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let name = ctx.args.lhs().to_string();
    if name.is_empty() {
        return Err(CmdError::msg("Dig what?"));
    }
    let exit_specs: Vec<String> = ctx.args.rhs_list().to_vec();
    // Validate every fallible prerequisite before allocating the room. Command
    // handlers run under an output/deferred checkpoint, not a World rollback;
    // returning Err after `create` would otherwise leave a dirty, silently
    // persisted room while reporting only the later permission error.
    let exit_source = if exit_specs.is_empty() {
        None
    } else {
        if exit_specs.iter().any(|spec| {
            !spec
                .split(';')
                .map(str::trim)
                .any(|segment| !segment.is_empty())
        }) {
            return Err(CmdError::msg("Open which exit?"));
        }
        // 16c B-75 (cut AQ): neither exit may take a kernel or host word, and the
        // room is not made for one that would.
        for spec in &exit_specs {
            check_exit_words(ctx.reg, spec)?;
        }
        let here = ctx
            .w
            .get(ctx.actor)
            .and_then(|o| o.location)
            .ok_or_else(|| CmdError::msg("You are nowhere; no exits opened."))?;
        if !perm::controls(ctx.w, ctx.actor, here) {
            return Err(CmdError::msg(
                "Permission denied.  (You don't control this room; no exits opened.)",
            ));
        }
        Some(here)
    };
    let room = ctx.w.create(ObjKind::Room, &name, ctx.actor, None)?;
    ctx.out.info(
        ctx.actor,
        format!("{name} created with room number {room}."),
    );
    ctx.emit_world_event(&WorldEvent::ObjCreated {
        obj: room,
        kind: ObjKind::Room,
        creator: ctx.actor,
    });

    if let Some(here) = exit_source {
        // Exit out: here -> new room.
        let out_exit = open_exit(ctx.w, ctx.actor, here, &exit_specs[0], Some(room))?;
        ctx.out.info(
            ctx.actor,
            format!("Opened exit {}({out_exit}).", primary_name(&exit_specs[0])),
        );
        ctx.emit_world_event(&WorldEvent::ObjCreated {
            obj: out_exit,
            kind: ObjKind::Exit,
            creator: ctx.actor,
        });
        // Exit back: new room -> here.
        if let Some(back) = exit_specs.get(1) {
            let back_exit = open_exit(ctx.w, ctx.actor, room, back, Some(here))?;
            ctx.out.info(
                ctx.actor,
                format!("Opened exit {}({back_exit}).", primary_name(back)),
            );
            ctx.emit_world_event(&WorldEvent::ObjCreated {
                obj: back_exit,
                kind: ObjKind::Exit,
                creator: ctx.actor,
            });
        }
    }
    if ctx.switches.has("teleport") {
        // Same contract as @teleport: consult the MoveAttempt veto and
        // notify Moved observers — @dig/teleport is a move like any other.
        if let Err(reason) = ctx.check_move_attempt(ctx.actor, None, room) {
            ctx.out.to_actor(ctx.actor, OutputEvent::Sys(reason));
            return Ok(());
        }
        let from = ctx.w.get(ctx.actor).and_then(|o| o.location);
        ctx.w.move_to(ctx.actor, room)?;
        ctx.show_room(ctx.actor, room);
        ctx.emit_world_event(&WorldEvent::Moved {
            who: ctx.actor,
            from,
            to: room,
        });
    }
    Ok(())
}

fn open_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let spec = ctx.args.lhs().to_string();
    if spec.is_empty() {
        return Err(CmdError::msg("Open which exit?"));
    }
    let here = ctx
        .w
        .get(ctx.actor)
        .and_then(|o| o.location)
        .ok_or_else(|| CmdError::msg("You are nowhere."))?;
    if !perm::controls(ctx.w, ctx.actor, here) {
        return Err(perm_denied());
    }
    // 16c B-75 (cut AQ): the exit may not take a kernel or host word.
    check_exit_words(ctx.reg, &spec)?;
    let dest = match ctx.args.rhs().map(str::to_string).filter(|s| !s.is_empty()) {
        Some(d) => {
            let dest = match_object(ctx.w, ctx.actor, &d)?;
            if !can_link_to(ctx.w, ctx.actor, dest) {
                return Err(perm_denied());
            }
            Some(dest)
        }
        None => None,
    };
    let exit = open_exit(ctx.w, ctx.actor, here, &spec, dest)?;
    ctx.out.info(ctx.actor, format!("Opened. ({exit})"));
    ctx.emit_world_event(&WorldEvent::ObjCreated {
        obj: exit,
        kind: ObjKind::Exit,
        creator: ctx.actor,
    });
    Ok(())
}

/// Whether `who` may link an exit to `dest`: control it, or it's a room
/// left open to jumps (`JUMP_OK` doubles as link-ok in M0).
fn can_link_to(w: &World, who: ObjId, dest: ObjId) -> bool {
    perm::controls(w, who, dest)
        || w.get(dest)
            .is_some_and(|o| o.flags.contains(Flags::JUMP_OK))
}

fn link_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().to_string();
    let rhs = ctx
        .args
        .rhs()
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("Link to what?  (@link <exit>=<destination>)"))?;
    let target = match_object(ctx.w, ctx.actor, &lhs)?;
    if !perm::can_modify(ctx.w, ctx.actor, target) {
        return Err(perm_denied());
    }
    let kind = ctx.w.get(target).map(|o| o.kind);
    if kind == Some(ObjKind::Exit) {
        let dest = match_object(ctx.w, ctx.actor, &rhs)?;
        if ctx.w.get(dest).is_none_or(|o| o.kind != ObjKind::Room) {
            return Err(CmdError::msg("Exits can only be linked to rooms."));
        }
        if !can_link_to(ctx.w, ctx.actor, dest) {
            return Err(perm_denied());
        }
        ctx.w.link_exit(target, Some(dest))?;
        ctx.out.info(ctx.actor, "Linked.");
    } else {
        // @link me=here — set an object's home. The destination needs the
        // same authority as linking an exit there: control it, or it is
        // left open (JUMP_OK doubles as link-ok in M0).
        let home = match_object(ctx.w, ctx.actor, &rhs)?;
        if !can_link_to(ctx.w, ctx.actor, home) {
            return Err(perm_denied());
        }
        ctx.w.set_home(target, Some(home))?;
        ctx.out.info(ctx.actor, "Home set.");
    }
    Ok(())
}

fn desc_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().to_string();
    let rhs = ctx
        .args
        .rhs()
        .map(str::to_string)
        .ok_or_else(|| CmdError::msg("Describe what?  (@desc <target>=<text>)"))?;
    let target = match_object(ctx.w, ctx.actor, &lhs)?;
    if !perm::can_modify(ctx.w, ctx.actor, target) {
        return Err(perm_denied());
    }
    if rhs.is_empty() {
        ctx.w.remove_attr(target, "desc")?;
        ctx.out.info(ctx.actor, "Description cleared.");
    } else {
        ctx.w.set_attr(target, "desc", Attr::text(rhs, ctx.actor))?;
        ctx.out.info(ctx.actor, "Set.");
    }
    Ok(())
}

fn name_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().to_string();
    if ctx.args.rhs().is_none() {
        return Err(CmdError::msg(
            "Rename to what?  (@name <target>=<new>[;alias...])",
        ));
    }
    let parts: Vec<String> = ctx.args.rhs_list().to_vec();
    let new_name = parts
        .first()
        .cloned()
        .ok_or_else(|| CmdError::msg("Rename to what?  (@name <target>=<new>[;alias...])"))?;
    let target = match_object(ctx.w, ctx.actor, &lhs)?;
    if !perm::can_modify(ctx.w, ctx.actor, target) {
        return Err(perm_denied());
    }
    // 16c B-75 (cut AQ): an exit's new name and every new alias may not be a
    // kernel or host word; refused before anything is renamed.
    if ctx.w.get(target).is_some_and(|o| o.kind == ObjKind::Exit) {
        for part in &parts {
            check_exit_words(ctx.reg, part)?;
        }
    }
    // Validate every new alias BEFORE mutating anything, so a collision
    // cannot leave the object half-renamed (the new name itself is checked
    // by set_name before it mutates).
    if ctx.w.get(target).is_some_and(|o| o.kind.is_actor()) {
        for a in &parts[1..] {
            if ctx.w.resolve_player(a).is_some_and(|other| other != target) {
                return Err(CmdError::msg("That name is already in use."));
            }
        }
    }
    ctx.w.set_name(target, &new_name).map_err(|e| match e {
        crate::world::WorldError::NameInUse(_) => CmdError::msg("That name is already in use."),
        other => other.into(),
    })?;
    // Aliases given with the new name replace the old alias set.
    let old: Vec<String> = ctx
        .w
        .get(target)
        .map(|o| o.aliases.iter().map(ToString::to_string).collect())
        .unwrap_or_default();
    for a in old {
        ctx.w.remove_alias(target, &a)?;
    }
    for a in &parts[1..] {
        ctx.w.add_alias(target, a)?;
    }
    ctx.out.info(ctx.actor, "Name set.");
    Ok(())
}

fn set_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().to_string();
    let rhs = ctx.args.rhs().map(str::to_string).ok_or_else(|| {
        CmdError::msg("Set what?  (@set <target>=[!]<FLAG> or @set <target>/<attr>=<value>)")
    })?;

    if let Some((tname, aname)) = lhs.split_once('/') {
        // Attribute form: @set <target>/<attr>=<value>.
        let aname = aname.trim();
        if aname.is_empty() {
            return Err(CmdError::msg("Set which attribute?"));
        }
        let target = match_object(ctx.w, ctx.actor, tname)?;
        if !perm::can_modify(ctx.w, ctx.actor, target) {
            return Err(perm_denied());
        }
        // Module-owned attribute namespaces (`wod:*`): only the owning
        // module's own commands (ctx.module) and Wizards+ may write them.
        if !ctx
            .reg
            .may_write_attr(ctx.w, ctx.actor, ctx.module.as_deref(), aname)
        {
            let ns = aname.split(':').next().unwrap_or(aname);
            return Err(CmdError::msg(format!(
                "Permission denied.  (The {ns}: attribute namespace is module-owned.)"
            )));
        }
        let existing = ctx
            .w
            .get(target)
            .and_then(|o| o.attrs.get(aname))
            .map(|a| (a.flags, a.owner));
        let existing_flags = existing.map(|(f, _)| f);
        if existing_flags.is_some_and(|f| f.contains(AttrFlags::WIZARD_ONLY))
            && !perm::has_role(ctx.w, ctx.actor, Role::Wizard)
        {
            return Err(perm_denied());
        }
        // A LOCKED attribute may only be changed by whoever set it, or staff.
        if existing.is_some_and(|(f, owner)| {
            f.contains(AttrFlags::LOCKED)
                && owner != ctx.actor
                && !perm::has_role(ctx.w, ctx.actor, Role::Wizard)
        }) {
            return Err(CmdError::msg("That attribute is locked."));
        }
        // `@set/flag <target>/<attr>=[!]<FLAG>`: rewrite one bit on an
        // attribute that already exists, leaving the value alone.
        if ctx.switches.has("flag") {
            let (on, fname) = match rhs.strip_prefix('!') {
                Some(rest) => (false, rest.trim()),
                None => (true, rhs.trim()),
            };
            if fname.is_empty() {
                return Err(CmdError::msg(
                    "Set which attribute flag?  (@set/flag <target>/<attr>=[!]<FLAG>)",
                ));
            }
            let name = fname.to_uppercase();
            let Some(flag) = AttrFlags::from_name(&name) else {
                return Err(CmdError::msg(format!(
                    "I don't understand the attribute flag \"{fname}\".  (Known: {}.)",
                    AttrFlags::all().names().join(" ")
                )));
            };
            // WIZARD_ONLY and SELF_ONLY are the two bits a lock key refuses
            // to read through (`lock::attr_is_readable`). Leaving them to
            // whoever controls the object would hand every player a switch
            // for defeating the attribute locks aimed at them, so both
            // directions are staff-only; HIDDEN, LOCKED and NO_INHERIT stay
            // controller tools.
            if flag.intersects(AttrFlags::WIZARD_ONLY | AttrFlags::SELF_ONLY)
                && !perm::has_role(ctx.w, ctx.actor, Role::Wizard)
            {
                return Err(perm_denied());
            }
            let Some(mut attr) = ctx.w.get(target).and_then(|o| o.attrs.get(aname)).cloned() else {
                return Err(CmdError::msg(format!("{aname} - No such attribute.")));
            };
            attr.flags.set(flag, on);
            ctx.w.set_attr(target, aname, attr)?;
            ctx.out.info(
                ctx.actor,
                format!("{aname} - {name} {}.", if on { "set" } else { "cleared" }),
            );
            ctx.emit_world_event(&crate::module::WorldEvent::AttrSet {
                obj: target,
                name: aname.to_string(),
                setter: ctx.actor,
            });
            return Ok(());
        }
        if rhs.is_empty() {
            ctx.w.remove_attr(target, aname)?;
            ctx.out.info(ctx.actor, format!("{aname} - Cleared."));
        } else {
            let attr = Attr {
                value: AttrValue::Text(rhs),
                owner: ctx.actor,
                flags: existing_flags.unwrap_or_default(),
            };
            ctx.w.set_attr(target, aname, attr)?;
            ctx.out.info(ctx.actor, format!("{aname} - Set."));
        }
        // Post-mutation observer signal (mush-script recompiles on
        // `script:*` changes; harmless for observers that filter it out).
        ctx.emit_world_event(&crate::module::WorldEvent::AttrSet {
            obj: target,
            name: aname.to_string(),
            setter: ctx.actor,
        });
        return Ok(());
    }

    // Flag form: @set <target>=[!]<FLAG>.
    if ctx.switches.has("flag") {
        return Err(CmdError::msg(
            "That is an object flag, not an attribute flag.  \
             (@set/flag <target>/<attr>=[!]<FLAG>)",
        ));
    }
    let target = match_object(ctx.w, ctx.actor, &lhs)?;
    let (on, flag_name) = match rhs.strip_prefix('!') {
        Some(rest) => (false, rest.trim().to_string()),
        None => (true, rhs.trim().to_string()),
    };
    if flag_name.is_empty() {
        return Err(CmdError::msg("Set which flag?"));
    }
    let Some(flag) = Flags::from_name(&flag_name.to_uppercase()) else {
        // Module marker flags ("wod:kindred"): registered names are
        // settable by whoever can modify the target.
        if let Some((canonical, _module)) = ctx.reg.marker_owner(&flag_name) {
            let canonical = canonical.clone();
            if !perm::can_modify(ctx.w, ctx.actor, target) {
                return Err(perm_denied());
            }
            ctx.w.set_marker(target, &canonical, on)?;
            ctx.out
                .info(ctx.actor, if on { "Set." } else { "Cleared." });
            return Ok(());
        }
        return Err(CmdError::msg(format!(
            "I don't understand the flag \"{flag_name}\"."
        )));
    };
    if !perm::can_modify(ctx.w, ctx.actor, target) {
        return Err(perm_denied());
    }
    // Privileged flags.
    if flag == Flags::GOING {
        return Err(CmdError::msg("Use @destroy for that."));
    }
    if flag == Flags::WIZARD && !perm::has_role(ctx.w, ctx.actor, Role::God) {
        return Err(perm_denied());
    }
    // SPOOF is the licence to put unattributed text in front of other
    // players, so it is granted the way BUILDER is. NOSPOOF is an observer
    // preference and stays self-settable.
    if (flag == Flags::ROYALTY || flag == Flags::BUILDER || flag == Flags::SPOOF)
        && !perm::has_role(ctx.w, ctx.actor, Role::Wizard)
    {
        return Err(perm_denied());
    }
    // DARK on a player/NPC is staff-grade invisibility (hides from WHO and
    // room contents), so setting it on an actor needs a wizard; clearing
    // it, and DARK on rooms/things/exits, stays a builder tool.
    if flag == Flags::DARK
        && on
        && ctx.w.get(target).is_some_and(|o| o.kind.is_actor())
        && !perm::has_role(ctx.w, ctx.actor, Role::Wizard)
    {
        return Err(perm_denied());
    }
    ctx.w.set_flag(target, flag, on)?;
    ctx.out
        .info(ctx.actor, if on { "Set." } else { "Cleared." });
    Ok(())
}

/// The lock type selected by an `@lock`/`@unlock` switch (default Basic).
fn lock_type_from_switches(ctx: &Ctx<'_>) -> LockType {
    for s in ctx.switches.iter() {
        if let Ok(lt) = s.parse::<LockType>() {
            return lt;
        }
    }
    LockType::Basic
}

fn lock_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().to_string();
    let key = ctx
        .args
        .rhs()
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("Lock to what?  (@lock[/<type>] <target>=<key>)"))?;
    let target = match_object(ctx.w, ctx.actor, &lhs)?;
    if !perm::can_modify(ctx.w, ctx.actor, target) {
        return Err(perm_denied());
    }
    let lt = lock_type_from_switches(ctx);
    let actor = ctx.actor;
    let w = &*ctx.w;
    let expr = parse_lock(&key, |name| {
        if name.eq_ignore_ascii_case("me") {
            Some(actor)
        } else {
            match_object(w, actor, name)
                .ok()
                .or_else(|| w.resolve_player(name))
        }
    })
    .map_err(|e| CmdError::msg(format!("I don't understand that key.  ({e})")))?;
    if let Some(o) = ctx.w.get_mut(target) {
        o.locks.insert(lt, expr);
    }
    ctx.out.info(ctx.actor, "Locked.");
    Ok(())
}

fn unlock_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.raw().to_string();
    if lhs.is_empty() {
        return Err(CmdError::msg("Unlock what?"));
    }
    let target = match_object(ctx.w, ctx.actor, &lhs)?;
    if !perm::can_modify(ctx.w, ctx.actor, target) {
        return Err(perm_denied());
    }
    let lt = lock_type_from_switches(ctx);
    let removed = ctx
        .w
        .get_mut(target)
        .is_some_and(|o| o.locks.remove(&lt).is_some());
    ctx.out.info(
        ctx.actor,
        if removed {
            "Unlocked."
        } else {
            "It wasn't locked."
        },
    );
    Ok(())
}

fn teleport_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().to_string();
    if lhs.is_empty() {
        return Err(CmdError::msg(
            "Teleport where?  (@teleport [<target>=]<destination>)",
        ));
    }
    let (target, dest_name) = match ctx.args.rhs().map(str::to_string).filter(|s| !s.is_empty()) {
        Some(d) => (match_object(ctx.w, ctx.actor, &lhs)?, d),
        None => (ctx.actor, lhs),
    };
    let raw_dest = match_object(ctx.w, ctx.actor, &dest_name)?;
    // Teleporting "to" an object means its location; to a room, the room.
    let dest = match ctx.w.get(raw_dest).map(|o| (o.kind, o.location)) {
        Some((ObjKind::Room, _)) => raw_dest,
        Some((ObjKind::Exit, _)) => {
            return Err(CmdError::msg("You can't teleport to an exit."));
        }
        Some((_, Some(loc))) => loc,
        _ => return Err(CmdError::msg("You can't go there.")),
    };
    if !perm::can_teleport(ctx.w, ctx.actor, target, dest) {
        return Err(perm_denied());
    }
    // Module veto (MoveAttempt hooks; `exit: None` marks a teleport).
    if let Err(reason) = ctx.check_move_attempt(target, None, dest) {
        ctx.out.to_actor(ctx.actor, OutputEvent::Sys(reason));
        return Ok(());
    }
    let from = ctx.w.get(target).and_then(|o| o.location);
    // Move first (a cycle — teleporting into something you carry — or a
    // vanished destination must NOT leak a raw internal error, and the
    // room must not see a departure that never happened).
    ctx.w
        .move_to(target, dest)
        .map_err(|_| CmdError::msg("You can't go there."))?;
    if let Some(fr) = from {
        let text = RichText::new()
            .span(name_span(ctx.w, target))
            .text(" has left.");
        ctx.out
            .to_room_from(fr, vec![target], target, narrative_emit(text));
    }
    let text = RichText::new()
        .span(name_span(ctx.w, target))
        .text(" has arrived.");
    ctx.out
        .to_room_from(dest, vec![target], target, narrative_emit(text));
    if ctx.w.get(target).is_some_and(|o| o.kind.is_actor()) {
        ctx.show_room(target, dest);
    }
    if target != ctx.actor {
        ctx.out.info(ctx.actor, "Teleported.");
    }
    ctx.emit_world_event(&WorldEvent::Moved {
        who: target,
        from,
        to: dest,
    });
    Ok(())
}

fn zone_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().to_string();
    let rhs = ctx
        .args
        .rhs()
        .map(str::to_string)
        .ok_or_else(|| CmdError::msg("Zone to what?  (@zone <target>=<zmo|none>)"))?;
    let target = match_object(ctx.w, ctx.actor, &lhs)?;
    if !perm::can_modify(ctx.w, ctx.actor, target) {
        return Err(perm_denied());
    }
    if rhs.is_empty() || rhs.eq_ignore_ascii_case("none") {
        ctx.w.set_zone(target, None)?;
        ctx.out.info(ctx.actor, "Zone cleared.");
        return Ok(());
    }
    let zmo = match_object(ctx.w, ctx.actor, &rhs)?;
    if ctx
        .w
        .get(zmo)
        .is_none_or(|o| !o.flags.contains(Flags::ZONE_MASTER))
    {
        return Err(CmdError::msg("That is not a Zone Master Object."));
    }
    // Zoning to a ZMO needs authority over it: control, or pass its Zone lock.
    if !perm::controls(ctx.w, ctx.actor, zmo) {
        let passes = ctx
            .w
            .get(zmo)
            .and_then(|o| o.locks.get(&crate::lock::LockType::Zone))
            .is_some_and(|l| {
                crate::lock::eval_lock(ctx.w, l, ctx.actor, crate::lock::MAX_LOCK_DEPTH)
            });
        if !passes {
            return Err(perm_denied());
        }
    }
    ctx.w.set_zone(target, Some(zmo))?;
    ctx.out.info(ctx.actor, "Zoned.");
    Ok(())
}

fn parent_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().to_string();
    let rhs = ctx
        .args
        .rhs()
        .map(str::to_string)
        .ok_or_else(|| CmdError::msg("Parent to what?  (@parent <target>=<obj|none>)"))?;
    let target = match_object(ctx.w, ctx.actor, &lhs)?;
    if !perm::can_modify(ctx.w, ctx.actor, target) {
        return Err(perm_denied());
    }
    if rhs.is_empty() || rhs.eq_ignore_ascii_case("none") {
        ctx.w.set_parent(target, None)?;
        ctx.out.info(ctx.actor, "Parent cleared.");
        return Ok(());
    }
    let parent = match_object(ctx.w, ctx.actor, &rhs)?;
    if !perm::controls(ctx.w, ctx.actor, parent) {
        return Err(perm_denied());
    }
    ctx.w.set_parent(target, Some(parent))?;
    ctx.out.info(ctx.actor, "Parent set.");
    Ok(())
}

fn chown_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().to_string();
    let rhs = ctx
        .args
        .rhs()
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("Chown to whom?  (@chown <object>=<player>)"))?;
    let target = match_object(ctx.w, ctx.actor, &lhs)?;
    let new_owner = match_object(ctx.w, ctx.actor, &rhs)
        .ok()
        .or_else(|| ctx.w.resolve_player(rhs.trim_start_matches('*')))
        .ok_or(crate::matchit::MatchErr::NoMatch)?;
    if ctx
        .w
        .get(new_owner)
        .is_none_or(|o| o.kind != ObjKind::Player)
    {
        return Err(CmdError::msg("Owners must be players."));
    }
    if ctx.w.get(target).is_some_and(|o| o.kind == ObjKind::Player) {
        return Err(CmdError::msg("Players always own themselves."));
    }
    ctx.w.chown(target, new_owner)?;
    ctx.out.info(ctx.actor, "Owner changed.");
    Ok(())
}

fn destroy_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let q = ctx.args.raw().to_string();
    if q.is_empty() {
        return Err(CmdError::msg("Destroy what?"));
    }
    let target = match_object(ctx.w, ctx.actor, &q)?;
    let overriding = ctx.switches.has("override");
    if overriding && !perm::has_role(ctx.w, ctx.actor, Role::Wizard) {
        return Err(perm_denied());
    }
    if !perm::controls(ctx.w, ctx.actor, target) {
        return Err(perm_denied());
    }
    let o = ctx.w.get(target).ok_or(crate::matchit::MatchErr::NoMatch)?;
    if o.kind == ObjKind::Player && !perm::has_role(ctx.w, ctx.actor, Role::Wizard) {
        return Err(CmdError::msg("Only wizards may destroy players."));
    }
    if o.flags.contains(Flags::SAFE) && !overriding {
        return Err(CmdError::msg(
            "That object is marked SAFE.  Clear the flag, or use @destroy/override (wizards).",
        ));
    }
    // Pre-destroy veto (design D8): behaviors and registered structural
    // dependents refuse here, at the command layer, before `World::destroy`
    // marks the husk GOING — so `World` stays observer-free and a domain host is
    // refused before its exit cascade can begin.
    if let Err(reason) = ctx.reg.check_destroy(ctx.w, ctx.actor, target) {
        return Err(CmdError::msg(reason));
    }
    let destroyed = ctx.w.destroy(target)?;
    ctx.out.info(ctx.actor, "Destroyed.");
    // One ObjDestroyed per husk — the target and any exits taken with it — so
    // module cleanup observers fire for cascaded exits too, not just the target.
    for obj in destroyed {
        ctx.emit_world_event(&WorldEvent::ObjDestroyed { obj });
    }
    Ok(())
}

fn find_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let pattern = ctx.args.raw().trim().to_lowercase();
    let staff = perm::has_role(ctx.w, ctx.actor, Role::Wizard)
        || perm::has_power(ctx.w, ctx.actor, Powers::EXAMINE_ALL);
    let my_owner = ctx.w.get(ctx.actor).map(|o| o.owner);
    let use_glob = pattern.contains('*') || pattern.contains('?');
    let mut count = 0usize;
    let mut lines: Vec<RichText> = Vec::new();
    for o in ctx.w.iter() {
        if o.flags.contains(Flags::GOING) {
            continue;
        }
        if !staff && Some(o.owner) != my_owner {
            continue;
        }
        // **16b cut U.4 (16c `T-67`, Q34)**: nothing the caller cannot
        // perceive is listed, staff included. This is the kernel's own
        // per-observer seam — the one `look` asks — and it is how doc 16 §3.6's
        // world-side exclusion reaches this listing without `mush-core` naming
        // `mush-sim`: `PlacementGate::perceives` answers `Conceal` for every unit
        // an arena holds, for every observer. A listing omits and says nothing
        // about what it omitted, so the count below counts only what it listed
        // and the answer for a held unit is the one an unheld name gets.
        if !ctx.can_perceive(ctx.actor, o.id) {
            continue;
        }
        let hit = pattern.is_empty()
            || if use_glob {
                crate::lock::glob_match(&pattern, &o.name)
            } else {
                o.name.to_lowercase().contains(&pattern)
            };
        if hit {
            count += 1;
            lines.push(
                RichText::new()
                    .span(name_span(ctx.w, o.id))
                    .text(format!("({})", o.id)),
            );
        }
    }
    for l in lines {
        ctx.out.to_actor(ctx.actor, OutputEvent::Sys(l));
    }
    ctx.out
        .info(ctx.actor, format!("*** END OF LIST ***  ({count} found)"));
    Ok(())
}

/// Register the building commands.
#[allow(clippy::too_many_lines)]
pub(super) fn register(reg: &mut Registry) -> Result<(), RegistryError> {
    reg.register(
        CommandSpec {
            name: "@create".into(),
            min_role: Role::Builder,
            help: "@create <name>  --  Create a new thing, owned by you, placed in \
                   your inventory. Requires the BUILDER flag."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(create_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "@dig".into(),
            switches: switch_vec(&["teleport"]),
            shape: ArgShape::EqPairList,
            min_role: Role::Builder,
            help: "@dig <name>[=<exit-out>[,<exit-back>]]  --  Create a new room. \
                   Exit specs may carry ;-separated aliases (\"north;n\"): the first \
                   opens here leading to the new room, the second opens in the new \
                   room leading back. @dig/teleport also moves you there. Requires \
                   BUILDER and control of the current room for the exits."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(dig_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "@open".into(),
            shape: ArgShape::EqPair,
            min_role: Role::Builder,
            help: "@open <exit>[;<alias>...][=<destination>]  --  Open a new exit in \
                   your current room, optionally linked at once. You must control \
                   the room, and the destination must be yours or JUMP_OK."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(open_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "@link".into(),
            shape: ArgShape::EqPair,
            help: "@link <exit>=<room>  or  @link <object>=<home>  --  Link an \
                   unlinked exit to a destination room, or set an object's (or \
                   your own) home: \"@link me=here\". You must control the thing \
                   being linked; the destination (room or home) must be yours or \
                   JUMP_OK."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(link_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "@desc".into(),
            aliases: vec!["@describe".into()],
            shape: ArgShape::EqPair,
            help: "@desc <target>=<text>  --  Set the description shown when someone \
                   looks at the target (\"@desc here=...\" for the room you're in). \
                   An empty text clears it. You must control the target."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(desc_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "@name".into(),
            shape: ArgShape::EqPairSemi,
            help: "@name <target>=<new-name>[;<alias>...]  --  Rename an object you \
                   control. Aliases after the name replace the old alias list; for \
                   players and NPCs, names and aliases must be unused (checked \
                   before anything changes)."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(name_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "@set".into(),
            switches: switch_vec(&["flag"]),
            shape: ArgShape::EqPair,
            help: "@set <target>=[!]<FLAG>  or  @set <target>/<attr>=<value>  or  \
                   @set/flag <target>/<attr>=[!]<ATTRFLAG>  --  Set or clear (!) an \
                   object flag, write an attribute (empty value clears it), or set \
                   one flag on an attribute that already exists. WIZARD needs God; \
                   ROYALTY/BUILDER need a wizard; DARK on a player or NPC needs a \
                   wizard; attributes flagged WIZARD_ONLY need a wizard and LOCKED \
                   ones their setter. Attribute flags: HIDDEN (kept out of a plain \
                   examine), LOCKED (only its setter may rewrite it), NO_INHERIT \
                   (not seen through @parent) -- and WIZARD_ONLY / SELF_ONLY, the \
                   two read guards, which only a wizard may set or clear because \
                   locks refuse to read through them. A flag survives a later \
                   value write. You must control the target."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(set_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "@lock".into(),
            switches: switch_vec(LOCK_SWITCHES),
            shape: ArgShape::EqPair,
            help: "@lock[/<type>] <target>=<key>  --  Lock a target you control. \
                   Types: basic (default; exits/pickup), drop, give, page, speech, \
                   teleport, zone. (enter/use/mail arrive with their commands.) \
                   Keys: me, *player, #123, +#123 (carry), $#123 (owned-by), \
                   flag^WIZARD, attr:glob, @#123/type (indirect), combined with \
                   ! & | and parentheses. A key may nest at most 48 operators \
                   deep (16 levels of parentheses); deeper keys are refused."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(lock_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "@unlock".into(),
            switches: switch_vec(LOCK_SWITCHES),
            help: "@unlock[/<type>] <target>  --  Remove a lock (default type: basic) \
                   from a target you control."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(unlock_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "@teleport".into(),
            aliases: vec!["@tel".into()],
            shape: ArgShape::EqPair,
            help: "@teleport [<target>=]<destination>  --  Move yourself (or a \
                   target you control) to a destination: a room you control or one \
                   set JUMP_OK. The TEL_ANYWHERE power opens all rooms, TEL_ANYTHING \
                   moves other people's objects; a destination @lock/teleport is \
                   honored. Teleporting to an object goes to its location."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(teleport_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "@zone".into(),
            shape: ArgShape::EqPair,
            help: "@zone <target>=<zmo|none>  --  Put an object you control into a \
                   zone (the zone object must be flagged ZONE_MASTER and be yours, \
                   or admit you through its @lock/zone), or clear it with \"none\". \
                   A ZMO's zone lock delegates control over all zone members."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(zone_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "@parent".into(),
            shape: ArgShape::EqPair,
            help: "@parent <target>=<obj|none>  --  Set the attribute-inheritance \
                   parent of an object you control (the parent must also be yours). \
                   Attribute lookups walk up to 10 parents; NO_INHERIT attributes \
                   don't inherit. \"none\" clears."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(parent_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "@chown".into(),
            shape: ArgShape::EqPair,
            power: Some(Powers::CHOWN_ANY),
            help: "@chown <object>=<player>  --  Transfer ownership of an object to \
                   a player. Requires the CHOWN_ANY power (wizards have it \
                   implicitly). Players always own themselves."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(chown_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "@destroy".into(),
            aliases: vec!["@dest".into(), "@nuke".into()],
            switches: switch_vec(&["override"]),
            help: "@destroy <target>  --  Destroy an object you control: it is \
                   marked GOING, detached from the world (contents go home, a \
                   room's exits die with it) and swept later; its id is never \
                   reused. SAFE objects refuse unless a wizard uses \
                   @destroy/override. Destroying players is wizard-only."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(destroy_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "@find".into(),
            help: "@find [<pattern>]  --  List your objects whose names match the \
                   pattern (substring, or a glob when it contains * or ?; empty \
                   lists everything). Wizards and EXAMINE_ALL holders search all \
                   objects."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(find_cmd),
    )?;
    Ok(())
}

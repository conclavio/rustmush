//! The canon command surface: `+chronicle` (player view of recent canon),
//! `@canon` (staff full view), and `@canon/retcon` (staff supersession).

use std::sync::Arc;

use mush_core::output::display_name;
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, ModError, ObjId, Registrar, Role, World, perm,
};

use crate::data::{CanonEvent, CanonId, CanonState};

fn state_ref<'a>(ctx: &'a Ctx<'_>) -> &'a CanonState {
    ctx.w
        .module_state::<CanonState>()
        .expect("canon state slot installed at registration")
}

fn is_staff(w: &World, who: ObjId) -> bool {
    perm::has_role(w, who, Role::Royalty)
}

fn to_actor(ctx: &mut Ctx<'_>, text: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "canon", text.into());
}

/// One chronicle line for `e`, marking a retconned (superseded) entry.
///
/// The raw canon id is shown to **staff only**: ids are a dense global counter,
/// so exposing them to a player whose view is scope-filtered would leak the
/// existence, count, and timing of hidden events through the gaps in the ids
/// they can see. Players get the summary alone.
fn line(w: &World, e: &CanonEvent, superseded: bool, staff: bool) -> String {
    let ents: Vec<String> = e.entities.iter().map(|&o| display_name(w, o)).collect();
    let who = if ents.is_empty() {
        String::new()
    } else {
        format!(" [{}]", ents.join(", "))
    };
    let mark = if superseded { "  (retconned)" } else { "" };
    if staff {
        format!(
            "  #{} <{}>  {}{who}{mark}",
            e.id,
            e.scope.label(),
            e.summary
        )
    } else {
        format!("  {}{who}{mark}", e.summary)
    }
}

/// `+chronicle` — the recent canon a player is entitled to see (Public plus any
/// Restricted event they are an entity of), newest first. The "previously on…"
/// of the world; superseded entries are marked, retcon meta-events hidden.
fn cmd_chronicle(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let staff = is_staff(ctx.w, actor);
    let (visible, superseded) = {
        let st = state_ref(ctx);
        let superseded = st.superseded_in_tail();
        let visible: Vec<CanonEvent> = st
            .recent()
            .into_iter()
            // Hide retcon meta-events from the player chronicle (their effect
            // shows as a "(retconned)" mark on the target); show real events.
            .filter(|e| e.supersedes.is_none())
            .filter(|e| e.visible_to(actor, staff))
            .collect();
        (visible, superseded)
    };
    if visible.is_empty() {
        to_actor(ctx, "The chronicle is empty so far.");
        return Ok(());
    }
    to_actor(ctx, "Previously, in the chronicle:");
    for e in visible.iter().rev() {
        let l = line(ctx.w, e, superseded.contains(&e.id), false);
        to_actor(ctx, l);
    }
    Ok(())
}

/// `@canon` — the full recent ledger for staff: every event (retcons included),
/// with ids and scopes, newest first.
fn cmd_canon(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("retcon") {
        return canon_retcon(ctx);
    }
    if !is_staff(ctx.w, ctx.actor) {
        return Err(CmdError::msg("Permission denied."));
    }
    let (events, superseded) = {
        let st = state_ref(ctx);
        (st.recent(), st.superseded_in_tail())
    };
    if events.is_empty() {
        to_actor(ctx, "The canon ledger is empty.");
        return Ok(());
    }
    to_actor(
        ctx,
        format!(
            "Canon ledger — {} recent event(s), newest first:",
            events.len()
        ),
    );
    for e in events.iter().rev() {
        let l = line(ctx.w, e, superseded.contains(&e.id), true);
        to_actor(ctx, l);
    }
    Ok(())
}

/// `@canon/retcon <id>=<reason>` — append a superseding event for canon `id`.
/// History is not rewritten; the supersession is itself an auditable canon
/// event.
fn canon_retcon(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !is_staff(ctx.w, ctx.actor) {
        return Err(CmdError::msg("Permission denied."));
    }
    let id: u64 = ctx
        .args
        .lhs()
        .trim()
        .trim_start_matches('#')
        .parse()
        .map_err(|_| CmdError::msg("Usage: @canon/retcon <id>=<reason>"))?;
    let reason = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("Give a reason: @canon/retcon <id>=<reason>"))?
        .to_string();
    // Validate the target against the RAM tail (deep-history retcon needs the
    // store-read seam): it must exist, be a real event (not a retcon
    // meta-event), and not already be superseded.
    let target = CanonId(id);
    let (found, is_retcon) = {
        let recent = state_ref(ctx).recent();
        let hit = recent.iter().find(|e| e.id == target);
        (hit.is_some(), hit.is_some_and(|e| e.supersedes.is_some()))
    };
    if !found {
        return Err(CmdError::msg(
            "No such canon event in recent history. (Older events need the archive, not yet built.)",
        ));
    }
    if is_retcon {
        return Err(CmdError::msg(
            "That is a retcon record, not a canon event — retcon the original id instead.",
        ));
    }
    if state_ref(ctx).superseded_in_tail().contains(&target) {
        return Err(CmdError::msg(
            "That canon event has already been retconned.",
        ));
    }
    match crate::retcon(ctx, target, &reason) {
        Some(rid) => {
            to_actor(
                ctx,
                format!("Retconned canon #{id} (recorded as #{rid}): {reason}"),
            );
            Ok(())
        }
        None => Err(CmdError::msg("Canon ledger is unavailable.")),
    }
}

pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "+chronicle".into(),
            shape: ArgShape::None,
            help: "+chronicle  --  The recent canon you know of: the notable things that have \
                   become true in the world, newest first. You see public events and those you \
                   were involved in; retconned entries are marked."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_chronicle),
    )?;
    reg.command(
        CommandSpec {
            name: "@canon".into(),
            shape: ArgShape::EqPair,
            switches: vec!["retcon".into()],
            help: "@canon  --  (staff) The recent canon ledger in full: every durable \
                   consequence with its id, scope, and entities, newest first (retcons \
                   included).\n\
                   @canon/retcon <id>=<reason>  --  (staff) Supersede canon event <id>. History \
                   is never rewritten — this appends an auditable retcon that marks the target \
                   no longer current."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_canon),
    )?;
    Ok(())
}

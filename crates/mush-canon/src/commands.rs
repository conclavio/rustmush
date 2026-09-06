//! The canon command surface: `+chronicle` (player view of recent canon),
//! `@canon` (staff full view), `@canon/retcon` (staff supersession), and the
//! H1 archive (`+chronicle/archive`, `@canon/archive`) over cold pages.

use std::sync::Arc;

use mush_core::output::display_name;
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, ModError, ObjId, Registrar, Role, StoreReadCompletion,
    StoreReadResult, World, archive_page_range, chunk_key, perm, read_cold_range,
};
use smol_str::SmolStr;

use crate::data::{CanonEvent, CanonId, CanonState, LOG, PendingCanonRead};

fn state_ref<'a>(ctx: &'a Ctx<'_>) -> &'a CanonState {
    ctx.w
        .module_state::<CanonState>()
        .expect("canon state slot installed at registration")
}

fn state<'a>(ctx: &'a mut Ctx<'_>) -> &'a mut CanonState {
    ctx.w
        .module_state_mut::<CanonState>()
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
    if ctx.switches.has("archive") {
        return canon_archive(ctx, false);
    }
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
    if ctx.switches.has("archive") {
        return canon_archive(ctx, true);
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
            "No such canon event in recent history. (Older events are readable with \
             @canon/archive; a retcon still needs the target in recent history.)",
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

// ----------------------------------------------------------------------
// H1 — the archive: cold pages of the chronicle through the store-read seam
// ----------------------------------------------------------------------

/// `+chronicle/archive [page]` and `@canon/archive [page]` — request one page
/// of the chronicle's **sealed** chunks (history older than the RAM tail)
/// through the module store-read seam. Page 0 is the newest sealed page; the
/// page renders when the read completes ([`CanonArchiveRead`]).
fn canon_archive(ctx: &mut Ctx<'_>, staff_view: bool) -> Result<(), CmdError> {
    let actor = ctx.actor;
    // D-H-3, with verify fold B1's stale window (see the scenes archive).
    let now = ctx.w.now();
    if let Some(p) = state_ref(ctx).pending_reads.get(&actor)
        && now.saturating_sub(p.requested_at) < mush_core::STORE_READ_STALE_SECS
    {
        to_actor(
            ctx,
            "An archive read is already in flight for you; wait for it to answer.",
        );
        return Ok(());
    }
    let raw = if staff_view {
        ctx.args.lhs().trim().to_string()
    } else {
        ctx.args.raw().trim().to_string()
    };
    let page: u64 = if raw.is_empty() {
        0
    } else {
        raw.parse()
            .map_err(|_| CmdError::msg("Usage: /archive [page]  (page 0 is the newest)"))?
    };
    let open = state_ref(ctx).ledger.open_chunk(LOG);
    if open == 0 {
        to_actor(
            ctx,
            "Nothing of the chronicle is archived yet; it is all recent (+chronicle).",
        );
        return Ok(());
    }
    let Some((lo, hi)) = archive_page_range(open - 1, page) else {
        to_actor(ctx, "No such archive page.");
        return Ok(());
    };
    let keys: Vec<SmolStr> = (lo..=hi).map(|n| chunk_key(LOG, n)).collect();
    let correlation = match ctx.read_store(keys) {
        Ok(c) => c,
        Err(error) => {
            to_actor(
                ctx,
                format!("That archive page cannot be requested: {error}."),
            );
            return Ok(());
        }
    };
    state(ctx).pending_reads.insert(
        actor,
        PendingCanonRead {
            correlation,
            requested_at: now,
            page,
            chunk_lo: lo,
            chunk_hi: hi,
        },
    );
    to_actor(
        ctx,
        format!("Reading archive page {page} of the chronicle (chunks {lo}–{hi}) …"),
    );
    Ok(())
}

/// The canon module's [`StoreReadCompletion`] (H1): render the archive page a
/// reader asked for through the **same** line renderer and scope filter the
/// live `+chronicle`/`@canon` use.
pub(crate) struct CanonArchiveRead;

impl StoreReadCompletion for CanonArchiveRead {
    fn on_read(&self, ctx: &mut Ctx<'_>, correlation: u64, result: StoreReadResult) {
        let actor = ctx.actor;
        let pending = {
            let st = state(ctx);
            match st.pending_reads.get(&actor) {
                Some(p) if p.correlation == correlation => st.pending_reads.remove(&actor),
                Some(p) => {
                    tracing::warn!(
                        %actor,
                        correlation,
                        pending = p.correlation,
                        "canon: stale archive completion dropped"
                    );
                    return;
                }
                None => {
                    tracing::warn!(
                        %actor,
                        correlation,
                        "canon: archive completion with no pending read dropped"
                    );
                    return;
                }
            }
        };
        let Some(pending) = pending else {
            return;
        };
        // Re-validate (treaty ruling 3): the actor may be gone, and staff
        // status is read now, not when the page was requested.
        if ctx.w.get(actor).is_none() {
            return;
        }
        let staff = is_staff(ctx.w, actor);
        let docs = match result {
            StoreReadResult::Failed(detail) => {
                to_actor(
                    ctx,
                    format!("The chronicle archive read did not complete ({detail}); ask again."),
                );
                return;
            }
            StoreReadResult::Oversized { bytes } => {
                to_actor(
                    ctx,
                    format!(
                        "Archive page {} of the chronicle is too large to read at once \
                         ({bytes} bytes).",
                        pending.page
                    ),
                );
                return;
            }
            StoreReadResult::Docs { docs, .. } => docs,
        };
        if docs.is_empty() {
            to_actor(
                ctx,
                format!(
                    "The chronicle has no archived chunks {}–{}.",
                    pending.chunk_lo, pending.chunk_hi
                ),
            );
            return;
        }
        let entries = match read_cold_range(&docs, LOG) {
            Ok(entries) => entries,
            Err(error) => {
                to_actor(
                    ctx,
                    format!(
                        "Archive page {} of the chronicle cannot be read: {error}.",
                        pending.page
                    ),
                );
                return;
            }
        };
        let page: Vec<CanonEvent> = entries
            .iter()
            .filter_map(|e| serde_json::from_value::<CanonEvent>(e.value.clone()).ok())
            .collect();
        // Supersession is resolved over everything readable here: this page,
        // the sealed boot window and the RAM tail — a retcon recorded after
        // the page's events lives in a later chunk. A retcon in a sealed chunk
        // that is on none of those is the documented limit of one page.
        let superseded = {
            let st = state_ref(ctx);
            let mut set = CanonState::superseded_in(&page);
            set.extend(st.superseded_in_tail());
            set.extend(CanonState::superseded_in(&st.sealed_tail));
            set
        };
        let visible: Vec<&CanonEvent> = page
            .iter()
            .filter(|e| staff || e.supersedes.is_none())
            .filter(|e| e.visible_to(actor, staff))
            .collect();
        let header = if staff {
            format!(
                "Canon ledger — archive page {} (chunks {}–{}), {} event(s), newest first:",
                pending.page,
                pending.chunk_lo,
                pending.chunk_hi,
                visible.len()
            )
        } else {
            format!(
                "Previously, in the chronicle (archive page {}):",
                pending.page
            )
        };
        let lines: Vec<String> = visible
            .iter()
            .rev()
            .map(|e| line(ctx.w, e, superseded.contains(&e.id), staff))
            .collect();
        to_actor(ctx, header);
        if lines.is_empty() {
            to_actor(ctx, "  (nothing you may see on that page)");
        }
        for l in lines {
            to_actor(ctx, l);
        }
    }
}

pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "+chronicle".into(),
            shape: ArgShape::Raw,
            switches: vec!["archive".into()],
            help: "+chronicle  --  The recent canon you know of: the notable things that have \
                   become true in the world, newest first. You see public events and those you \
                   were involved in; retconned entries are marked.\n\
                   +chronicle/archive [page]  --  Older canon than the recent view holds, a page \
                   at a time (page 0 is the newest archived page). The page arrives a moment \
                   later."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_chronicle),
    )?;
    reg.command(
        CommandSpec {
            name: "@canon".into(),
            shape: ArgShape::EqPair,
            switches: vec!["retcon".into(), "archive".into()],
            help: "@canon  --  (staff) The recent canon ledger in full: every durable \
                   consequence with its id, scope, and entities, newest first (retcons \
                   included).\n\
                   @canon/archive [page]  --  (staff) The same, over the archived pages older \
                   than the recent ledger (page 0 is the newest archived page).\n\
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

//! The `@privilege` command family: `/grant`, `/revoke`, `/list`, `/audit`
//! (the RAM tail) and `/archive` (cold pages through the H1 store-read seam).

use std::sync::Arc;

use mush_core::output::display_name;
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, ModError, ObjId, Privileges, Registrar, Role,
    StoreReadCompletion, StoreReadResult, World, archive_page_range, chunk_key, read_cold_range,
};
use smol_str::SmolStr;

use crate::data::{
    AuditAction, LOG, MAX_REASON_BYTES, MAX_RECENT_LINES, PendingAuditRead, PrivilegeAction,
    PrivilegeAuditEntry, PrivilegesState, contains_identity, decode_entries, parse_privilege,
    privilege_name,
};

fn state_ref<'a>(ctx: &'a Ctx<'_>) -> &'a PrivilegesState {
    ctx.w
        .module_state::<PrivilegesState>()
        .expect("privileges state slot installed at registration")
}

fn state<'a>(ctx: &'a mut Ctx<'_>) -> &'a mut PrivilegesState {
    ctx.w
        .module_state_mut::<PrivilegesState>()
        .expect("privileges state slot installed at registration")
}

fn to_actor(ctx: &mut Ctx<'_>, text: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "privilege", text.into());
}

/// Resolve a player argument: a `#dbref` naming an actor, or a (possibly
/// `*`-prefixed) player name.
fn resolve_player(w: &World, raw: &str) -> Result<ObjId, CmdError> {
    let q = raw.trim();
    if let Ok(id) = q.parse::<ObjId>() {
        return if w.get(id).is_some_and(|o| o.kind.is_actor()) {
            Ok(id)
        } else {
            Err(CmdError::msg("I don't recognize that player."))
        };
    }
    let name = q.strip_prefix('*').unwrap_or(q);
    w.resolve_player(name)
        .ok_or_else(|| CmdError::msg("I don't recognize that player."))
}

/// Validate a staff-entered reason: required, bounded, and never an identity.
fn checked_reason(raw: &str) -> Result<String, CmdError> {
    let reason = raw.trim();
    if reason.is_empty() {
        return Err(CmdError::msg(
            "A reason is required: @privilege/grant <player>=<privilege>=<reason>.",
        ));
    }
    if reason.len() > MAX_REASON_BYTES {
        return Err(CmdError::msg(format!(
            "That reason is too long ({} bytes; the limit is {MAX_REASON_BYTES}).",
            reason.len()
        )));
    }
    if contains_identity(reason) {
        return Err(CmdError::msg(
            "A reason may not contain an identifier; the audit log never records one.",
        ));
    }
    Ok(reason.to_string())
}

/// Split `<privilege>=<reason>` (the right-hand side of the EqPair).
fn split_privilege_and_reason(rhs: &str) -> Result<(Privileges, String), CmdError> {
    let (name, reason) = rhs.split_once('=').unwrap_or((rhs, ""));
    let privilege = parse_privilege(name).ok_or_else(|| {
        CmdError::msg(format!(
            "Unknown privilege {:?}. Known: {}.",
            name.trim(),
            Privileges::all()
                .iter_names()
                .map(|(n, _)| n)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;
    Ok((privilege, checked_reason(reason)?))
}

/// One audit line. Names are rendered live (an object may be gone by now, in
/// which case its dbref stands).
fn line(w: &World, e: &PrivilegeAuditEntry) -> String {
    let who = display_name(w, e.actor);
    let mut s = format!("  [{}] {who} {}", e.at, e.action.label());
    if let Some(p) = &e.privilege {
        s.push(' ');
        s.push_str(p);
    }
    if let Some(subject) = e.subject {
        s.push_str(" -> ");
        s.push_str(&display_name(w, subject));
    }
    if let Some(reason) = &e.reason {
        s.push_str(&format!(" ({reason})"));
    }
    s
}

fn grant_or_revoke(ctx: &mut Ctx<'_>, grant: bool) -> Result<(), CmdError> {
    let verb = if grant { "grant" } else { "revoke" };
    let target = ctx.args.lhs().trim().to_string();
    let rhs = ctx.args.rhs().unwrap_or("").trim().to_string();
    if target.is_empty() || rhs.is_empty() {
        return Err(CmdError::msg(format!(
            "Usage: @privilege/{verb} <player>=<privilege>=<reason>."
        )));
    }
    let subject = resolve_player(ctx.w, &target)?;
    let (privilege, reason) = split_privilege_and_reason(&rhs)?;
    let name = privilege_name(privilege);
    let now = ctx.w.now();
    let held = ctx
        .w
        .get(subject)
        .is_some_and(|o| o.privileges.contains(privilege));
    if !grant && !held {
        return Err(CmdError::msg(format!(
            "{} does not hold {name}.",
            display_name(ctx.w, subject)
        )));
    }
    // Mutate first (a no-op re-grant still audits: the act was requested).
    if let Some(o) = ctx.w.get_mut(subject) {
        o.privileges.set(privilege, grant);
        o.modified = now;
    }
    crate::audit(
        ctx,
        PrivilegeAction {
            action: if grant {
                AuditAction::Grant
            } else {
                AuditAction::Revoke
            },
            privilege: Some(privilege),
            subject: Some(subject),
            reason: Some(reason.clone()),
            detail: serde_json::json!({ "already_held": held }),
        },
    )
    .map_err(|e| CmdError::msg(format!("The audit could not be recorded: {e}.")))?;
    let subject_name = display_name(ctx.w, subject);
    let actor_name = display_name(ctx.w, ctx.actor);
    if grant {
        to_actor(
            ctx,
            format!(
                "Granted {name} to {subject_name}{}. Recorded.",
                if held { " (already held)" } else { "" }
            ),
        );
        ctx.out.sys_to_actor(
            subject,
            "privilege",
            format!("{actor_name} granted you the {name} privilege: {reason}"),
        );
    } else {
        to_actor(
            ctx,
            format!("Revoked {name} from {subject_name}. Recorded."),
        );
        ctx.out.sys_to_actor(
            subject,
            "privilege",
            format!("{actor_name} revoked your {name} privilege: {reason}"),
        );
    }
    Ok(())
}

fn list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim().to_string();
    if raw.is_empty() {
        let holders: Vec<(String, SmolStr)> = ctx
            .w
            .iter()
            .filter(|o| o.kind.is_actor() && !o.privileges.is_empty())
            .map(|o| (display_name(ctx.w, o.id), privilege_name(o.privileges)))
            .collect();
        if holders.is_empty() {
            to_actor(ctx, "No one holds a privilege.");
            return Ok(());
        }
        to_actor(ctx, "Privilege holders:");
        for (who, what) in holders {
            to_actor(ctx, format!("  {who}: {what}"));
        }
        return Ok(());
    }
    let subject = resolve_player(ctx.w, &raw)?;
    let held = ctx.w.get(subject).map(|o| o.privileges).unwrap_or_default();
    let who = display_name(ctx.w, subject);
    if held.is_empty() {
        to_actor(ctx, format!("{who} holds no privilege."));
    } else {
        to_actor(ctx, format!("{who}: {}", privilege_name(held)));
    }
    Ok(())
}

/// `@privilege/audit [<player>]` — the RAM tail, newest first, filtered to
/// entries where `<player>` acted or was the subject when given.
fn audit_recent(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim().to_string();
    let filter = if raw.is_empty() {
        None
    } else {
        Some(resolve_player(ctx.w, &raw)?)
    };
    let entries = state_ref(ctx).recent();
    let lines: Vec<String> = entries
        .iter()
        .rev()
        .filter(|e| filter.is_none_or(|f| e.actor == f || e.subject == Some(f)))
        .take(MAX_RECENT_LINES)
        .map(|e| line(ctx.w, e))
        .collect();
    let open = state_ref(ctx).audit.open_chunk(LOG);
    to_actor(
        ctx,
        format!(
            "Privilege audit — recent, newest first ({} shown{}):",
            lines.len(),
            if open > 0 {
                format!("; {open} sealed page(s) in the archive, @privilege/archive [page]")
            } else {
                String::new()
            }
        ),
    );
    if lines.is_empty() {
        to_actor(ctx, "  (nothing recorded yet)");
    }
    for l in lines {
        to_actor(ctx, l);
    }
    Ok(())
}

/// `@privilege/archive [<page>]` — request one page of the audit's sealed
/// chunks through the module store-read seam (page 0 is the newest sealed
/// page); it renders when the read completes ([`AuditArchiveRead`]).
fn audit_archive(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
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
    let raw = ctx.args.raw().trim().to_string();
    let page: u64 = if raw.is_empty() {
        0
    } else {
        raw.parse().map_err(|_| {
            CmdError::msg("Usage: @privilege/archive [page]  (page 0 is the newest)")
        })?
    };
    let open = state_ref(ctx).audit.open_chunk(LOG);
    if open == 0 {
        to_actor(
            ctx,
            "Nothing of the privilege audit is archived yet; it is all recent (@privilege/audit).",
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
        PendingAuditRead {
            correlation,
            requested_at: now,
            page,
            chunk_lo: lo,
            chunk_hi: hi,
        },
    );
    to_actor(
        ctx,
        format!("Reading archive page {page} of the privilege audit (chunks {lo}–{hi}) …"),
    );
    Ok(())
}

/// The module's [`StoreReadCompletion`] (H1): render the archive page a
/// reader asked for through the same line renderer as the live view.
pub(crate) struct AuditArchiveRead;

impl StoreReadCompletion for AuditArchiveRead {
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
                        "privileges: stale archive completion dropped"
                    );
                    return;
                }
                None => {
                    tracing::warn!(
                        %actor,
                        correlation,
                        "privileges: archive completion with no pending read dropped"
                    );
                    return;
                }
            }
        };
        let Some(pending) = pending else {
            return;
        };
        // Re-validate (treaty ruling 3): the actor may be gone, and the role is
        // read now, not when the page was requested.
        if ctx.w.get(actor).is_none() || !mush_core::perm::has_role(ctx.w, actor, Role::Wizard) {
            return;
        }
        let docs = match result {
            StoreReadResult::Failed(detail) => {
                to_actor(
                    ctx,
                    format!(
                        "The privilege audit archive read did not complete ({detail}); ask again."
                    ),
                );
                return;
            }
            StoreReadResult::Oversized { bytes } => {
                to_actor(
                    ctx,
                    format!(
                        "Archive page {} of the privilege audit is too large to read at once \
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
                    "The privilege audit has no archived chunks {}–{}.",
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
                        "Archive page {} of the privilege audit cannot be read: {error}.",
                        pending.page
                    ),
                );
                return;
            }
        };
        let page = decode_entries(entries.iter().map(|e| &e.value));
        to_actor(
            ctx,
            format!(
                "Privilege audit — archive page {} (chunks {}–{}), {} entries, newest first:",
                pending.page,
                pending.chunk_lo,
                pending.chunk_hi,
                page.len()
            ),
        );
        if page.is_empty() {
            to_actor(ctx, "  (nothing on that page)");
        }
        let lines: Vec<String> = page.iter().rev().map(|e| line(ctx.w, e)).collect();
        for l in lines {
            to_actor(ctx, l);
        }
    }
}

fn cmd_privilege(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("grant") {
        grant_or_revoke(ctx, true)
    } else if ctx.switches.has("revoke") {
        grant_or_revoke(ctx, false)
    } else if ctx.switches.has("audit") {
        audit_recent(ctx)
    } else if ctx.switches.has("archive") {
        audit_archive(ctx)
    } else {
        // `/list`, or a bare `@privilege`, lists.
        list(ctx)
    }
}

pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "@privilege".into(),
            shape: ArgShape::EqPair,
            switches: vec![
                "grant".into(),
                "revoke".into(),
                "list".into(),
                "audit".into(),
                "archive".into(),
            ],
            min_role: Role::Wizard,
            help: "@privilege/grant <player>=<privilege>=<reason>  --  Grant an explicit \
                   privilege. Unlike a power, no rank implies a privilege: not Wizards, not \
                   God — only a grant made here, which is recorded with its reason in a \
                   durable audit. @privilege/revoke <player>=<privilege>=<reason> takes one \
                   back. @privilege/list [<player>] shows every holder (or one player's). \
                   @privilege/audit [<player>] shows the recent audit — grants, revocations, \
                   permitted uses and refused attempts — newest first; @privilege/archive \
                   [page] reads older history a page at a time (page 0 is the newest). The \
                   audit records that a privilege was used, never what was seen. Privileges: \
                   DISCORD_IDENTITY (read a player's linked Discord identity, @discord/show). \
                   Wizards only."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_privilege),
    )?;
    Ok(())
}

//! The `+request` / `+jobs` / `+job` / `+note` / `+notes` command surface.
//!
//! Role gating, MUSH voice:
//!
//! - Working a queue (assign/approve/deny/status, seeing its full listing)
//!   requires the queue's registered `min_role`; a job's opener and
//!   assignee always see (and may comment on) their own jobs.
//! - Notes staff is Royalty+; players may write non-staff-only notes on
//!   their own character.
//! - The opener gets a Sys notice (tag `"jobs"`) on status changes and
//!   comments while connected.

use std::sync::Arc;

use mush_core::output::display_name;
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, ModError, ModuleOp, ModuleWrite, ObjId, ObjKind,
    Registrar, Registry, Role, World, match_object, perm,
};
use smol_str::SmolStr;

use crate::api::{JobsApi, OpenJob, add_comment, job_doc, note_doc, open_job, queue_spec};
use crate::data::{
    Approval, Job, JobComment, JobStatus, JobsState, NS, Note, REQ_QUEUE, canon_category, fmt_ago,
    note_key, validate_terminal_comment,
};

/// Minimum role treated as "staff" for character notes.
pub const NOTES_STAFF: Role = Role::Royalty;

/// Register the whole command surface on the module's registrar.
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "+request".into(),
            shape: ArgShape::EqPair,
            help: "+request <title>=<text> -- file a request with staff. It becomes a job \
                   in the REQ queue; you'll get a notice when its status changes. See your \
                   open requests with +jobs."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_request),
    )?;
    reg.command(
        CommandSpec {
            name: "+jobs".into(),
            shape: ArgShape::Raw,
            help: "+jobs [<queue>] -- list jobs you can see, optionally in one queue. \
                   Staff see every job in queues their role can work; everyone always \
                   sees jobs they opened or are assigned to."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_jobs),
    )?;
    reg.command(
        CommandSpec {
            name: "+job".into(),
            shape: ArgShape::EqPair,
            switches: vec![
                "comment".into(),
                "assign".into(),
                "approve".into(),
                "deny".into(),
                "status".into(),
            ],
            help: "+job <#> -- show one job with its comment trail.\n\
                   +job/comment <#>=<text> -- add a comment (opener, assignee, or queue staff).\n\
                   +job/assign <#>=<staffer> -- assign a job (queue staff; an Open job \
                   becomes In progress).\n\
                   +job/approve <#>[=<comment>] -- resolve a job; a structured payload's \
                   registered handler is applied first and a handler failure leaves the \
                   job unchanged.\n\
                   +job/deny <#>=<reason> -- deny a job with a reason.\n\
                   +job/status <#>=<open|inprogress|resolved|denied> -- set the status \
                   directly (queue staff; never runs payload handlers -- use /approve \
                   for that; a terminal payload job cannot be reopened).\n\
                   The opener is notified of comments and status changes while connected."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_job),
    )?;
    reg.command(
        CommandSpec {
            name: "+note".into(),
            shape: ArgShape::EqPair,
            switches: vec!["approve".into(), "staff".into()],
            help: "+note <char> -- list a character's notes you may see.\n\
                   +note <char>/<category> -- read one note.\n\
                   +note <char>/<category>=<text> -- write a note (staff on anyone; \
                   players on their own character). Editing clears any approval stamp.\n\
                   +note <char>/<category>= -- delete the note.\n\
                   +note/staff <char>/<category>=<text> -- staff: write a staff-only \
                   note, hidden from the character.\n\
                   +note/approve <char>/<category> -- staff: stamp a note approved."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_note),
    )?;
    reg.command(
        CommandSpec {
            name: "+notes".into(),
            shape: ArgShape::None,
            help: "+notes -- list your own character's notes (staff-only notes stay \
                   hidden unless you are staff)."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_notes),
    )?;
    Ok(())
}

// ----------------------------------------------------------------------
// Shared helpers
// ----------------------------------------------------------------------

/// The role needed to work `queue`. Unregistered queues (a producing
/// module not loaded this boot) fall back to Wizard.
fn queue_min_role(reg: &Registry, queue: &str) -> Role {
    queue_spec(reg, queue).map_or(Role::Wizard, |q| q.min_role)
}

/// Whether `who` can work `queue` (staff operations + full listing).
fn works_queue(w: &World, reg: &Registry, who: ObjId, queue: &str) -> bool {
    perm::has_role(w, who, queue_min_role(reg, queue))
}

/// Whether `who` may see `job`: opener and assignee always; otherwise
/// queue staff.
fn may_view_job(w: &World, reg: &Registry, who: ObjId, job: &Job) -> bool {
    job.opener == who || job.assignee == Some(who) || works_queue(w, reg, who, &job.queue)
}

/// Whether `who` is notes staff.
fn is_notes_staff(w: &World, who: ObjId) -> bool {
    perm::has_role(w, who, NOTES_STAFF)
}

/// Whether `who` may see `note`.
fn may_see_note(w: &World, who: ObjId, note: &Note) -> bool {
    is_notes_staff(w, who) || (note.owner == who && !note.staff_only)
}

/// Parse the job id from the command's lhs.
fn parse_job_id(lhs: &str) -> Result<u64, CmdError> {
    let s = lhs.trim().trim_start_matches('#');
    if s.is_empty() {
        return Err(CmdError::msg("Which job? (+job <#>)"));
    }
    s.parse::<u64>()
        .map_err(|_| CmdError::msg(format!("\"{s}\" is not a job number.")))
}

/// A clone of the job, or "No such job.".
fn job_clone(ctx: &Ctx<'_>, id: u64) -> Result<Job, CmdError> {
    ctx.w
        .module_state::<JobsState>()
        .and_then(|st| st.jobs.get(&id))
        .cloned()
        .ok_or_else(|| CmdError::msg("No such job."))
}

/// Resolve a character (player or NPC) by name, `*name`, `#dbref`, or
/// `me`, globally.
fn resolve_char(w: &World, looker: ObjId, name: &str) -> Result<ObjId, CmdError> {
    let name = name.trim();
    let id = w
        .resolve_player(name)
        .or_else(|| match_object(w, looker, name).ok())
        .ok_or_else(|| CmdError::msg(format!("No such character: {name}.")))?;
    match w.get(id).map(|o| o.kind) {
        Some(ObjKind::Player | ObjKind::Npc) => Ok(id),
        _ => Err(CmdError::msg(format!("{name} is not a character."))),
    }
}

/// Mutate job `id` in place, persist the whole document, and return a
/// clone of the updated job. `updated_at` is stamped automatically.
fn update_job(ctx: &mut Ctx<'_>, id: u64, mutate: impl FnOnce(&mut Job)) -> Result<Job, CmdError> {
    let now = ctx.w.now();
    let st = ctx
        .w
        .module_state_mut::<JobsState>()
        .ok_or_else(|| CmdError::msg("The jobs module is not loaded."))?;
    let job = st
        .jobs
        .get_mut(&id)
        .ok_or_else(|| CmdError::msg("No such job."))?;
    mutate(job);
    job.updated_at = now;
    let doc = job_doc(job);
    let updated = job.clone();
    ctx.persist(ModuleOp {
        ns: NS.into(),
        op: ModuleWrite::JsonSet {
            key: crate::data::job_key(id),
            value: doc,
        },
    });
    Ok(updated)
}

/// Notify the opener of job `id` (tag `"jobs"`) if they are connected and
/// not the acting player.
fn notify_opener(ctx: &mut Ctx<'_>, opener: ObjId, id: u64, msg: String) {
    notify(ctx, opener, id, msg);
}

/// Notify any actor about job `id` (tag `"jobs"`) if connected and not the
/// actor: the classic Sys line for the scrollback, and — interface plan I11,
/// the producer §3.11 promised and look-back I11-C24 found missing — a
/// `Notice` in the `jobs` category whose one action prefills `+job <id>`, so
/// the web tray and the TUI's unseen count carry it and a tap opens the job.
fn notify(ctx: &mut Ctx<'_>, who: ObjId, id: u64, msg: String) {
    if who != ctx.actor && ctx.w.is_connected(who) {
        ctx.out.sys_to_actor(who, "jobs", msg.clone());
        ctx.notice_info(who, "jobs", msg, Some(("Open", &format!("+job {id}"))));
    }
}

// ----------------------------------------------------------------------
// +request / +jobs / +job
// ----------------------------------------------------------------------

fn cmd_request(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let title = ctx.args.lhs().trim().to_string();
    let text = ctx
        .args
        .rhs()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if title.is_empty() || text.is_empty() {
        return Err(CmdError::msg("Usage: +request <title>=<text>"));
    }
    let id = open_job(
        ctx,
        OpenJob {
            queue: REQ_QUEUE.into(),
            title: title.clone(),
            opener: ctx.actor,
            text: Some(text),
            payload: None,
            tags: Vec::new(),
        },
    )
    .map_err(CmdError::msg)?;
    ctx.out.info(
        ctx.actor,
        format!("Request filed as job {id} in queue {REQ_QUEUE}. Track it with +job {id}."),
    );
    // Interface programme D5: staff hear about a new request on their relay
    // sink too (the Discord staff channel, when the operator mapped one). The
    // title is the player's own words, the same staff read in `+jobs`.
    let opener = display_name(ctx.w, ctx.actor);
    let now_ms = ctx.w.now().saturating_mul(1000);
    ctx.out.push(
        mush_core::Audience::Relay(mush_proto::SmolStr::new(
            mush_core::output::STAFF_RELAY_SINK,
        )),
        mush_proto::OutputEvent::Notice {
            id: 0,
            severity: mush_proto::NoticeSeverity::Info,
            category: mush_proto::SmolStr::new("request"),
            text: mush_proto::RichText::plain(format!("+request #{id} from {opener}: {title}")),
            action: Some(mush_proto::NoticeAction {
                label: "Open".into(),
                prefill: format!("+job {id}"),
            }),
            ts: now_ms,
        },
    );
    Ok(())
}

fn cmd_jobs(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let filter = ctx.args.raw().trim().to_string();
    let filter = if filter.is_empty() {
        None
    } else {
        let q = queue_spec(ctx.reg, &filter)
            .ok_or_else(|| CmdError::msg(format!("No such job queue: {filter}.")))?;
        Some(q.name.clone())
    };
    let Some(st) = ctx.w.module_state::<JobsState>() else {
        return Err(CmdError::msg("The jobs module is not loaded."));
    };
    let rows: Vec<String> = st
        .jobs
        .values()
        .filter(|j| filter.as_ref().is_none_or(|f| j.queue == *f))
        .filter(|j| may_view_job(ctx.w, ctx.reg, ctx.actor, j))
        .map(|j| {
            let assignee = j
                .assignee
                .map_or_else(|| "-".to_string(), |a| display_name(ctx.w, a));
            format!(
                "{:>4}  {:<8} {:<12} {:<30} {} / {}",
                j.id,
                j.queue,
                j.status,
                j.title,
                display_name(ctx.w, j.opener),
                assignee
            )
        })
        .collect();
    if rows.is_empty() {
        ctx.out.info(ctx.actor, "No jobs to show.");
        return Ok(());
    }
    ctx.out.info(
        ctx.actor,
        format!(
            "{:>4}  {:<8} {:<12} {:<30} {}",
            "Job", "Queue", "Status", "Title", "Opener / Assignee"
        ),
    );
    for row in rows {
        ctx.out.info(ctx.actor, row);
    }
    Ok(())
}

fn cmd_job(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let switches: Vec<String> = ctx.switches.iter().map(str::to_string).collect();
    if switches.len() > 1 {
        return Err(CmdError::msg("Pick one +job switch."));
    }
    match switches.first().map(String::as_str) {
        None => job_detail(ctx),
        Some("comment") => job_comment(ctx),
        Some("assign") => job_assign(ctx),
        Some("approve") => job_approve(ctx),
        Some("deny") => job_deny(ctx),
        Some("status") => job_status(ctx),
        Some(other) => Err(CmdError::msg(format!("Unknown +job switch /{other}."))),
    }
}

fn job_detail(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_job_id(ctx.args.lhs())?;
    let job = job_clone(ctx, id)?;
    if !may_view_job(ctx.w, ctx.reg, ctx.actor, &job) {
        return Err(CmdError::msg("Permission denied."));
    }
    let now = ctx.w.now();
    let assignee = job
        .assignee
        .map_or_else(|| "(none)".to_string(), |a| display_name(ctx.w, a));
    ctx.out.info(
        ctx.actor,
        format!("Job {} [{}] -- {}", job.id, job.queue, job.title),
    );
    ctx.out.info(
        ctx.actor,
        format!(
            "Status: {}   Opener: {}   Assignee: {}",
            job.status,
            display_name(ctx.w, job.opener),
            assignee
        ),
    );
    ctx.out.info(
        ctx.actor,
        format!(
            "Opened: {} ago   Updated: {} ago",
            fmt_ago(now, job.opened_at),
            fmt_ago(now, job.updated_at)
        ),
    );
    if !job.tags.is_empty() {
        let tags: Vec<&str> = job.tags.iter().map(SmolStr::as_str).collect();
        ctx.out
            .info(ctx.actor, format!("Tags: {}", tags.join(", ")));
    }
    if let Some(p) = &job.payload {
        ctx.out.info(
            ctx.actor,
            format!("Payload: {} (applied on approval)", p.handler),
        );
    }
    if job.comments.is_empty() {
        ctx.out.info(ctx.actor, "No comments.");
    } else {
        ctx.out.info(ctx.actor, "Comments:");
        let lines: Vec<String> = job
            .comments
            .iter()
            .enumerate()
            .map(|(i, c)| {
                format!(
                    "  {}. {} ({} ago): {}",
                    i + 1,
                    display_name(ctx.w, c.who),
                    fmt_ago(now, c.ts),
                    c.text
                )
            })
            .collect();
        for line in lines {
            ctx.out.info(ctx.actor, line);
        }
    }
    Ok(())
}

fn job_comment(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_job_id(ctx.args.lhs())?;
    let text = ctx
        .args
        .rhs()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if text.is_empty() {
        return Err(CmdError::msg("Usage: +job/comment <#>=<text>"));
    }
    let job = job_clone(ctx, id)?;
    if !may_view_job(ctx.w, ctx.reg, ctx.actor, &job) {
        return Err(CmdError::msg("Permission denied."));
    }
    let who = ctx.actor;
    add_comment(ctx, id, who, &text).map_err(CmdError::msg)?;
    ctx.out
        .info(ctx.actor, format!("Comment added to job {id}."));
    let msg = format!(
        "{} commented on job {id} ({}): {text}",
        display_name(ctx.w, ctx.actor),
        job.title
    );
    notify_opener(ctx, job.opener, id, msg);
    Ok(())
}

fn job_assign(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_job_id(ctx.args.lhs())?;
    let target = ctx
        .args
        .rhs()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if target.is_empty() {
        return Err(CmdError::msg("Usage: +job/assign <#>=<staffer>"));
    }
    let job = job_clone(ctx, id)?;
    if !works_queue(ctx.w, ctx.reg, ctx.actor, &job.queue) {
        return Err(CmdError::msg("Permission denied."));
    }
    let staffer = resolve_char(ctx.w, ctx.actor, &target)?;
    if !works_queue(ctx.w, ctx.reg, staffer, &job.queue) {
        return Err(CmdError::msg(format!(
            "{} cannot work queue {}.",
            display_name(ctx.w, staffer),
            job.queue
        )));
    }
    let job = update_job(ctx, id, move |j| {
        j.assignee = Some(staffer);
        if j.status == JobStatus::Open {
            j.status = JobStatus::InProgress;
        }
    })?;
    let staffer_name = display_name(ctx.w, staffer);
    ctx.out
        .info(ctx.actor, format!("Job {id} assigned to {staffer_name}."));
    let msg = format!(
        "Job {id} ({}) has been assigned to {staffer_name}.",
        job.title
    );
    notify_opener(ctx, job.opener, id, msg);
    let msg = format!("You have been assigned job {id} ({}).", job.title);
    notify(ctx, staffer, id, msg);
    Ok(())
}

fn job_approve(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_job_id(ctx.args.lhs())?;
    let comment = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string);
    let job = job_clone(ctx, id)?;
    if !works_queue(ctx.w, ctx.reg, ctx.actor, &job.queue) {
        return Err(CmdError::msg("Permission denied."));
    }
    if job.status.is_terminal() {
        return Err(CmdError::msg(format!(
            "Job {id} is already {}.",
            job.status.to_string().to_lowercase()
        )));
    }
    if let Some(text) = &comment {
        validate_terminal_comment(&job, text).map_err(CmdError::msg)?;
    }
    // Apply the structured payload first: a missing handler or a handler
    // error aborts here, leaving the job untouched. On success the
    // handler's mutations and the status flip below commit atomically
    // (same command, same WriteBatch).
    if let Some(payload) = &job.payload {
        let handler = ctx
            .state::<JobsApi>()
            .and_then(|api| api.handlers.get(&payload.handler).cloned())
            .ok_or_else(|| {
                CmdError::msg(format!(
                    "No payload handler {:?} is registered; job {id} unchanged.",
                    payload.handler
                ))
            })?;
        handler(ctx, &job).map_err(|e| {
            CmdError::msg(format!(
                "Payload handler {:?} failed: {e} Job {id} unchanged.",
                payload.handler
            ))
        })?;
    }
    let (who, ts) = (ctx.actor, ctx.w.now());
    let stamp = comment.clone();
    let applied = job.payload.is_some();
    let job = update_job(ctx, id, move |j| {
        j.status = JobStatus::Resolved;
        // E4c verify fold, E3: the **applied marker**, written in the same command
        // and the same batch as the handler mutations above. It is what the
        // economy's `jobs.resolved` fact answers from: a status flip settles
        // nothing, because a status is not a fact about the past.
        if applied {
            j.payload_applied = Some(ts);
        }
        if let Some(text) = stamp {
            j.comments.push(JobComment { who, ts, text });
        }
    })?;
    // Approval is durability-critical (PLAN: XP approval gates its
    // confirmation on the store ack): "Job N approved." — and the payload
    // handler's applied effects — must not be confirmed before the batch
    // carrying them is acknowledged.
    ctx.gate_on_durable_ack();
    ctx.out.info(ctx.actor, format!("Job {id} approved."));
    let mut msg = format!(
        "Your job {id} ({}) has been approved by {}.",
        job.title,
        display_name(ctx.w, ctx.actor)
    );
    if let Some(text) = comment {
        msg.push_str(&format!(" {text}"));
    }
    notify_opener(ctx, job.opener, id, msg);
    Ok(())
}

fn job_deny(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_job_id(ctx.args.lhs())?;
    let reason = ctx
        .args
        .rhs()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if reason.is_empty() {
        return Err(CmdError::msg("Usage: +job/deny <#>=<reason>"));
    }
    let job = job_clone(ctx, id)?;
    if !works_queue(ctx.w, ctx.reg, ctx.actor, &job.queue) {
        return Err(CmdError::msg("Permission denied."));
    }
    if job.status.is_terminal() {
        return Err(CmdError::msg(format!(
            "Job {id} is already {}.",
            job.status.to_string().to_lowercase()
        )));
    }
    validate_terminal_comment(&job, &reason).map_err(CmdError::msg)?;
    let (who, ts) = (ctx.actor, ctx.w.now());
    let reason_comment = reason.clone();
    let job = update_job(ctx, id, move |j| {
        j.status = JobStatus::Denied;
        j.comments.push(JobComment {
            who,
            ts,
            text: reason_comment,
        });
    })?;
    ctx.out.info(ctx.actor, format!("Job {id} denied."));
    let msg = format!(
        "Your job {id} ({}) has been denied by {}: {reason}",
        job.title,
        display_name(ctx.w, ctx.actor)
    );
    notify_opener(ctx, job.opener, id, msg);
    Ok(())
}

fn job_status(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_job_id(ctx.args.lhs())?;
    let wanted = ctx.args.rhs().map(str::trim).unwrap_or_default();
    let status = JobStatus::parse(wanted)
        .ok_or_else(|| CmdError::msg("Valid statuses: open, inprogress, resolved, denied."))?;
    let job = job_clone(ctx, id)?;
    if !works_queue(ctx.w, ctx.reg, ctx.actor, &job.queue) {
        return Err(CmdError::msg("Permission denied."));
    }
    // A payload job that reached a terminal status may have had its
    // payload APPLIED (approve) — reopening it would let a second
    // /approve run the handler again with no applied-marker to stop it
    // (double XP spend). Payload jobs are one-shot: once terminal, they
    // stay terminal.
    if job.payload.is_some() && job.status.is_terminal() && !status.is_terminal() {
        return Err(CmdError::msg(format!(
            "Job {id} is {} and carries a payload; payload jobs cannot be \
             reopened (file a new one).",
            job.status.to_string().to_lowercase()
        )));
    }
    // E4c verify fold, E3: **a payload job resolves only through `/approve`**, and
    // its `Resolved` is not a status staff can move off afterwards.
    //
    // `Resolved` on a payload job is what the economy's `jobs.resolved` fact is
    // about, so this command was a settlement lever: `+job/status <n>=resolved`
    // released escrow on a payload that had never run, and `Resolved` -> `Denied`
    // (terminal to terminal, so the reopen guard above let it through) took the
    // answer back after money had already moved on it.
    if job.payload.is_some() {
        if status == JobStatus::Resolved {
            return Err(CmdError::msg(format!(
                "Job {id} carries a payload: it is resolved by +job/approve, which runs the \
                 payload, and by nothing else."
            )));
        }
        if job.status == JobStatus::Resolved {
            return Err(CmdError::msg(format!(
                "Job {id} carries a payload and is resolved; that is a fact about what was \
                 applied and cannot be moved."
            )));
        }
    }
    let job = update_job(ctx, id, move |j| {
        j.status = status;
    })?;
    ctx.out
        .info(ctx.actor, format!("Job {id} status set to {status}."));
    let msg = format!(
        "Job {id} ({}) status changed to {status} by {}.",
        job.title,
        display_name(ctx.w, ctx.actor)
    );
    notify_opener(ctx, job.opener, id, msg);
    Ok(())
}

// ----------------------------------------------------------------------
// +note / +notes
// ----------------------------------------------------------------------

fn cmd_note(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let approve = ctx.switches.has("approve");
    let staff_mark = ctx.switches.has("staff");
    if approve && staff_mark {
        return Err(CmdError::msg("Pick one +note switch."));
    }
    let lhs = ctx.args.lhs().to_string();
    let rhs = ctx.args.rhs().map(str::to_string);
    let (char_name, category) = match lhs.split_once('/') {
        Some((c, cat)) => (c.trim().to_string(), Some(canon_category(cat))),
        None => (lhs.trim().to_string(), None),
    };
    if char_name.is_empty() {
        return Err(CmdError::msg("Usage: +note <char>[/<category>][=<text>]"));
    }
    let who = resolve_char(ctx.w, ctx.actor, &char_name)?;
    if let Some(cat) = &category
        && cat.is_empty()
    {
        return Err(CmdError::msg("Which category?"));
    }

    if approve {
        return note_approve(ctx, who, category, rhs);
    }
    if let Some(text) = rhs {
        let Some(cat) = category else {
            return Err(CmdError::msg("Usage: +note <char>/<category>=<text>"));
        };
        return note_write(ctx, who, cat, text.trim().to_string(), staff_mark);
    }
    if staff_mark {
        return Err(CmdError::msg("Usage: +note/staff <char>/<category>=<text>"));
    }
    match category {
        Some(cat) => note_read(ctx, who, &cat),
        None => note_list(ctx, who),
    }
}

fn note_approve(
    ctx: &mut Ctx<'_>,
    who: ObjId,
    category: Option<SmolStr>,
    rhs: Option<String>,
) -> Result<(), CmdError> {
    if !is_notes_staff(ctx.w, ctx.actor) {
        return Err(CmdError::msg("Permission denied."));
    }
    let Some(cat) = category else {
        return Err(CmdError::msg("Usage: +note/approve <char>/<category>"));
    };
    if rhs.is_some() {
        return Err(CmdError::msg("+note/approve takes no text."));
    }
    let (by, at) = (ctx.actor, ctx.w.now());
    let st = ctx
        .w
        .module_state_mut::<JobsState>()
        .ok_or_else(|| CmdError::msg("The jobs module is not loaded."))?;
    let note = st
        .notes
        .get_mut(&(who, cat.clone()))
        .ok_or_else(|| CmdError::msg("No such note."))?;
    if note.approved.is_some() {
        return Err(CmdError::msg("That note is already approved."));
    }
    note.approved = Some(Approval { by, at });
    let doc = note_doc(note);
    ctx.persist(ModuleOp {
        ns: NS.into(),
        op: ModuleWrite::JsonSet {
            key: note_key(who, &cat),
            value: doc,
        },
    });
    ctx.out.info(
        ctx.actor,
        format!("Note '{cat}' on {} approved.", display_name(ctx.w, who)),
    );
    Ok(())
}

fn note_write(
    ctx: &mut Ctx<'_>,
    who: ObjId,
    cat: SmolStr,
    text: String,
    staff_mark: bool,
) -> Result<(), CmdError> {
    let staff = is_notes_staff(ctx.w, ctx.actor);
    if !staff {
        let existing_staff_only = ctx
            .w
            .module_state::<JobsState>()
            .and_then(|st| st.notes.get(&(who, cat.clone())))
            .is_some_and(|n| n.staff_only);
        if staff_mark || who != ctx.actor || existing_staff_only {
            return Err(CmdError::msg("Permission denied."));
        }
    }
    let author = ctx.actor;
    let st = ctx
        .w
        .module_state_mut::<JobsState>()
        .ok_or_else(|| CmdError::msg("The jobs module is not loaded."))?;
    if text.is_empty() {
        // Deletion: `+note <char>/<category>=`.
        if st.notes.remove(&(who, cat.clone())).is_none() {
            return Err(CmdError::msg("No such note."));
        }
        ctx.persist(ModuleOp {
            ns: NS.into(),
            op: ModuleWrite::Del {
                key: note_key(who, &cat),
            },
        });
        ctx.out.info(
            ctx.actor,
            format!("Note '{cat}' on {} deleted.", display_name(ctx.w, who)),
        );
        return Ok(());
    }
    let note = Note {
        owner: who,
        category: cat.clone(),
        text,
        author,
        staff_only: staff_mark,
        approved: None,
    };
    let doc = note_doc(&note);
    st.notes.insert((who, cat.clone()), note);
    ctx.persist(ModuleOp {
        ns: NS.into(),
        op: ModuleWrite::JsonSet {
            key: note_key(who, &cat),
            value: doc,
        },
    });
    ctx.out.info(
        ctx.actor,
        format!("Note '{cat}' set on {}.", display_name(ctx.w, who)),
    );
    Ok(())
}

fn note_read(ctx: &mut Ctx<'_>, who: ObjId, cat: &SmolStr) -> Result<(), CmdError> {
    let note = ctx
        .w
        .module_state::<JobsState>()
        .and_then(|st| st.notes.get(&(who, cat.clone())))
        .filter(|n| may_see_note(ctx.w, ctx.actor, n))
        .cloned()
        .ok_or_else(|| CmdError::msg("No such note."))?;
    ctx.out.info(
        ctx.actor,
        format!(
            "Note '{cat}' on {} (by {}{}{}):",
            display_name(ctx.w, who),
            display_name(ctx.w, note.author),
            if note.staff_only { ", staff-only" } else { "" },
            note.approved.as_ref().map_or_else(String::new, |a| {
                format!(", approved by {}", display_name(ctx.w, a.by))
            })
        ),
    );
    ctx.out.info(ctx.actor, note.text);
    Ok(())
}

fn note_list(ctx: &mut Ctx<'_>, who: ObjId) -> Result<(), CmdError> {
    if !is_notes_staff(ctx.w, ctx.actor) && who != ctx.actor {
        return Err(CmdError::msg("Permission denied."));
    }
    let rows = note_rows(ctx.w, ctx.actor, who);
    if rows.is_empty() {
        ctx.out.info(
            ctx.actor,
            format!("No notes on {}.", display_name(ctx.w, who)),
        );
        return Ok(());
    }
    ctx.out
        .info(ctx.actor, format!("Notes on {}:", display_name(ctx.w, who)));
    for row in rows {
        ctx.out.info(ctx.actor, row);
    }
    Ok(())
}

fn cmd_notes(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let rows = note_rows(ctx.w, ctx.actor, ctx.actor);
    if rows.is_empty() {
        ctx.out.info(ctx.actor, "You have no notes.");
        return Ok(());
    }
    ctx.out.info(ctx.actor, "Your notes:");
    for row in rows {
        ctx.out.info(ctx.actor, row);
    }
    Ok(())
}

/// Render the note-list rows for `who` that `viewer` may see.
fn note_rows(w: &World, viewer: ObjId, who: ObjId) -> Vec<String> {
    let Some(st) = w.module_state::<JobsState>() else {
        return Vec::new();
    };
    st.notes
        .range((who, SmolStr::default())..)
        .take_while(|((owner, _), _)| *owner == who)
        .filter(|(_, n)| may_see_note(w, viewer, n))
        .map(|((_, cat), n)| {
            let mut markers = String::new();
            if n.staff_only {
                markers.push_str(" [staff-only]");
            }
            if n.approved.is_some() {
                markers.push_str(" [approved]");
            } else {
                markers.push_str(" [unapproved]");
            }
            format!("  {cat:<16}{markers}  {}", n.text)
        })
        .collect()
}

//! The `@mail` command surface, message delivery, and the compose input
//! context.
//!
//! One registered command, switch-routed (`@mail/send`, `@mail/quick`, …).
//! Message numbers (`<n>`) are always 1-based positions in the actor's
//! **current folder** listing — the classic MUSH convention (they shift
//! when messages are filed elsewhere; ids in headers are stable).

use mush_core::lock::MAX_LOCK_DEPTH;
use mush_core::output::display_name;
use mush_core::{
    CmdError, ContextOutcome, Ctx, InputContext, LockType, ObjId, Role, World, eval_lock, obj_ref,
    perm,
};
use mush_proto::{InputMode, OutputEvent, PromptSpec, RichText};
use smol_str::SmolStr;

use crate::data::{
    self, ARCHIVE, INBOX, MAX_USER_FOLDERS, MailMsg, MailState, MailStatus, MailboxEntry, SENT,
    TRASH, is_builtin_folder, valid_folder_name,
};
use crate::date::{format_date, format_datetime};

/// Shared view of the mail state slot (installed at registration; its
/// absence is a wiring bug).
fn state<'a>(ctx: &'a Ctx<'_>) -> &'a MailState {
    ctx.state::<MailState>().expect("mail state slot installed")
}

/// Mutable view of the mail state slot.
fn state_mut<'a>(ctx: &'a mut Ctx<'_>) -> &'a mut MailState {
    ctx.state_mut::<MailState>()
        .expect("mail state slot installed")
}

/// The `@mail` handler: routes on switches, defaults to list/read.
pub(crate) fn mail_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("send") {
        return send_cmd(ctx);
    }
    if ctx.switches.has("quick") {
        return quick_cmd(ctx);
    }
    if ctx.switches.has("replyall") {
        return reply_cmd(ctx, true);
    }
    if ctx.switches.has("reply") {
        return reply_cmd(ctx, false);
    }
    if ctx.switches.has("fwd") {
        return fwd_cmd(ctx);
    }
    if ctx.switches.has("file") {
        return file_cmd(ctx);
    }
    if ctx.switches.has("folder") {
        return folder_cmd(ctx);
    }
    if ctx.switches.has("newfolder") {
        return newfolder_cmd(ctx);
    }
    if ctx.switches.has("rmfolder") {
        return rmfolder_cmd(ctx);
    }
    if ctx.switches.has("delete") {
        return delete_cmd(ctx);
    }
    if ctx.switches.has("purge") {
        return purge_cmd(ctx);
    }
    if ctx.switches.has("unread") {
        return unread_cmd(ctx);
    }
    if ctx.switches.has("urgent") {
        return Err(CmdError::msg(
            "MAIL: /urgent modifies /send, /quick, /reply, /replyall, or /fwd.",
        ));
    }
    let arg = ctx.args.raw().to_string();
    if arg.is_empty() {
        list_cmd(ctx)
    } else {
        read_cmd(ctx, &arg)
    }
}

// ----------------------------------------------------------------------
// Shared helpers
// ----------------------------------------------------------------------

/// Comma-separated display names.
fn name_list(w: &World, ids: &[ObjId]) -> String {
    ids.iter()
        .map(|&i| display_name(w, i))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Parse a 1-based message number.
fn parse_index(arg: &str) -> Result<usize, CmdError> {
    arg.trim()
        .parse::<usize>()
        .ok()
        .filter(|n| *n >= 1)
        .ok_or_else(|| CmdError::msg("MAIL: Expected a message number from the current folder."))
}

/// Resolve message number `n` in the actor's current folder to a mail id.
fn resolve_msg(ctx: &Ctx<'_>, n: usize) -> Result<u64, CmdError> {
    let st = state(ctx);
    let folder = st.current_folder(ctx.actor);
    let ids = st.folder_ids(ctx.actor, folder);
    ids.get(n - 1).copied().ok_or_else(|| {
        CmdError::msg(format!(
            "MAIL: No message {n} in folder {}.",
            st.current_folder(ctx.actor)
        ))
    })
}

/// Resolve a recipient list: names (or `*name` / `#dbref`) separated by
/// commas and/or whitespace, deduplicated. Unknown names abort the send.
fn parse_recipients(w: &World, spec: &str) -> Result<Vec<ObjId>, CmdError> {
    let mut out: Vec<ObjId> = Vec::new();
    let mut unknown: Vec<String> = Vec::new();
    for tok in spec
        .split(|c: char| c == ',' || c.is_whitespace())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let name = tok.trim_start_matches('*');
        let found = if let Some(num) = name.strip_prefix('#') {
            num.parse::<u32>().ok().map(ObjId)
        } else {
            w.resolve_player(name)
        };
        match found.filter(|&i| w.get(i).is_some_and(|o| o.kind.is_actor())) {
            Some(i) => {
                if !out.contains(&i) {
                    out.push(i);
                }
            }
            None => unknown.push(tok.to_string()),
        }
    }
    if !unknown.is_empty() {
        return Err(CmdError::msg(format!(
            "MAIL: No such player: {}.",
            unknown.join(", ")
        )));
    }
    if out.is_empty() {
        return Err(CmdError::msg("MAIL: Whom do you want to mail?"));
    }
    Ok(out)
}

// ----------------------------------------------------------------------
// Delivery (send/quick/fwd and compose completion all land here)
// ----------------------------------------------------------------------

/// Deliver one message: fan out to every recipient that passes their
/// `@lock/mail` and quota, store the message once plus one mailbox entry
/// per accepted recipient (and the sender's `sent` copy), queue the
/// write-through ops, and emit live [`OutputEvent::MailNotice`]s.
///
/// Refused recipients are reported to the sender; if nobody accepts,
/// nothing is stored and an error is returned.
pub(crate) fn deliver(
    ctx: &mut Ctx<'_>,
    to: &[ObjId],
    subject: &str,
    body: &str,
    reply_to: Option<u64>,
    urgent: bool,
) -> Result<(), CmdError> {
    let from = ctx.actor;
    let quota = state(ctx).quota();
    let mut accepted: Vec<ObjId> = Vec::new();
    let mut refused: Vec<String> = Vec::new();
    for &r in to {
        let Some(lock_opt) = ctx.w.get(r).map(|o| o.locks.get(&LockType::Mail).cloned()) else {
            refused.push(format!("{r} no longer exists."));
            continue;
        };
        let name = display_name(ctx.w, r);
        if let Some(l) = lock_opt
            && !eval_lock(ctx.w, &l, from, MAX_LOCK_DEPTH)
        {
            refused.push(format!("{name} is refusing your mail."));
            continue;
        }
        // Quota gates delivery; staff mailboxes are exempt.
        if !perm::has_role(ctx.w, r, Role::Wizard) && state(ctx).entry_count(r) >= quota {
            refused.push(format!("{name}'s mailbox is full."));
            continue;
        }
        accepted.push(r);
    }

    if accepted.is_empty() {
        for line in refused {
            ctx.out.abort_info(from, format!("MAIL: {line}"));
        }
        return Err(CmdError::msg("MAIL: Nobody could receive your message."));
    }

    let now = ctx.w.now();
    let mut ops = Vec::new();
    let mut notices: Vec<(ObjId, u32)> = Vec::new();
    let id = {
        let st = state_mut(ctx);
        let id = st.alloc_id();
        let msg = MailMsg {
            id,
            from,
            to: accepted.clone(),
            subject: subject.to_string(),
            body: body.to_string(),
            reply_to,
            sent_at: now,
        };
        ops.push(data::msg_op(&msg));
        st.insert_msg(msg);
        for &r in &accepted {
            let entry = MailboxEntry {
                owner: r,
                msg: id,
                folder: SmolStr::new(INBOX),
                status: if urgent {
                    MailStatus::Urgent
                } else {
                    MailStatus::Unread
                },
                read_at: None,
            };
            ops.push(data::entry_op(&entry));
            st.box_mut(r).entries.insert(id, entry);
            notices.push((r, st.unread_counts(r).0));
        }
        // The sender's copy files under `sent`, pre-read — unless the
        // sender mailed themselves, in which case the inbox entry wins
        // (entries are keyed `(owner, msg)`).
        if st.entry(from, id).is_none() {
            let entry = MailboxEntry {
                owner: from,
                msg: id,
                folder: SmolStr::new(SENT),
                status: MailStatus::Read,
                read_at: Some(now),
            };
            ops.push(data::entry_op(&entry));
            st.box_mut(from).entries.insert(id, entry);
        }
        ops.push(data::next_id_op(st.next_id()));
        id
    };
    for op in ops {
        ctx.persist(op);
    }

    let names = name_list(ctx.w, &accepted);
    ctx.out
        .info(from, format!("MAIL: Message {id} sent to {names}."));
    for line in refused {
        ctx.out.info(from, format!("MAIL: {line}"));
    }
    let from_ref = obj_ref(ctx.w, from);
    for (r, unread) in notices {
        ctx.out.to_actor(
            r,
            OutputEvent::MailNotice {
                from: from_ref.clone(),
                subject: subject.to_string(),
                id,
                unread,
            },
        );
    }
    // Mail send is durability-critical (PLAN): "MAIL: Message N sent."
    // must not reach the sender until the store has acknowledged the
    // batch that carries the message.
    ctx.gate_on_durable_ack();
    Ok(())
}

// ----------------------------------------------------------------------
// Compose input context
// ----------------------------------------------------------------------

/// The multi-line compose mode pushed by `@mail/send` and `@mail/reply`.
///
/// While on top of the input stack it captures every line as body text.
/// `--` alone sends and pops; `~p` previews; `~q` is the stack's own hard
/// escape (this context never sees it). Lines arrive trimmed (dispatch
/// trims all input).
pub(crate) struct Compose {
    /// Validated recipients (re-checked at send time).
    pub(crate) to: Vec<ObjId>,
    /// Subject line.
    pub(crate) subject: String,
    /// Reply chain link, when composing via `/reply`.
    pub(crate) reply_to: Option<u64>,
    /// Whether the message delivers with urgent status.
    pub(crate) urgent: bool,
    /// Body lines collected so far.
    pub(crate) lines: Vec<String>,
}

impl InputContext for Compose {
    fn prompt(&self, _w: &World) -> PromptSpec {
        PromptSpec {
            mode: InputMode::Normal,
            text: Some(RichText::plain("Mail>")),
        }
    }

    fn feed(&mut self, ctx: &mut Ctx<'_>, line: &str) -> ContextOutcome {
        if line == "--" {
            let body = self.lines.join("\n");
            if let Err(e) = deliver(
                ctx,
                &self.to,
                &self.subject,
                &body,
                self.reply_to,
                self.urgent,
            ) {
                ctx.out.error(ctx.actor, e.to_string());
            }
            return ContextOutcome::Pop;
        }
        if line.eq_ignore_ascii_case("~p") {
            let me = ctx.actor;
            let to = name_list(ctx.w, &self.to);
            ctx.out.info(me, "MAIL: --- preview ---");
            ctx.out.info(me, format!("MAIL: To: {to}"));
            ctx.out.info(
                me,
                format!(
                    "MAIL: Subject: {}{}",
                    self.subject,
                    if self.urgent { " (urgent)" } else { "" }
                ),
            );
            for l in &self.lines {
                ctx.out.info(me, l.clone());
            }
            ctx.out.info(
                me,
                format!(
                    "MAIL: --- {} line(s); \"--\" sends, \"~q\" aborts ---",
                    self.lines.len()
                ),
            );
            return ContextOutcome::Consumed;
        }
        self.lines.push(line.to_string());
        ContextOutcome::Consumed
    }
}

/// Announce and push a compose context.
fn start_compose(
    ctx: &mut Ctx<'_>,
    to: Vec<ObjId>,
    subject: String,
    reply_to: Option<u64>,
    urgent: bool,
) -> Result<(), CmdError> {
    let names = name_list(ctx.w, &to);
    ctx.out.info(
        ctx.actor,
        format!(
            "MAIL: Composing to {names} -- subject: {subject}{}",
            if urgent { " (urgent)" } else { "" }
        ),
    );
    ctx.out.info(
        ctx.actor,
        "MAIL: Type your message; \"--\" alone sends, \"~p\" previews, \"~q\" aborts.",
    );
    ctx.push_input_context(Box::new(Compose {
        to,
        subject,
        reply_to,
        urgent,
        lines: Vec::new(),
    }));
    Ok(())
}

// ----------------------------------------------------------------------
// Subcommands
// ----------------------------------------------------------------------

fn send_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let subject = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("MAIL: Usage: @mail/send <players>=<subject>"))?
        .to_string();
    let to = parse_recipients(ctx.w, ctx.args.lhs())?;
    let urgent = ctx.switches.has("urgent");
    start_compose(ctx, to, subject, None, urgent)
}

fn quick_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let usage = || CmdError::msg("MAIL: Usage: @mail/quick <players>=<subject>/<body>");
    let rhs = ctx.args.rhs().map(str::to_string).ok_or_else(usage)?;
    let (subject, body) = rhs.split_once('/').ok_or_else(usage)?;
    let (subject, body) = (subject.trim(), body.trim());
    if subject.is_empty() {
        return Err(usage());
    }
    let to = parse_recipients(ctx.w, ctx.args.lhs())?;
    let urgent = ctx.switches.has("urgent");
    deliver(ctx, &to, subject, body, None, urgent)
}

fn reply_cmd(ctx: &mut Ctx<'_>, all: bool) -> Result<(), CmdError> {
    let n = parse_index(ctx.args.lhs())?;
    let msg_id = resolve_msg(ctx, n)?;
    let (orig_from, orig_to, orig_subject) = {
        let m = state(ctx)
            .message(msg_id)
            .ok_or_else(|| CmdError::msg("MAIL: That message no longer exists."))?;
        (m.from, m.to.clone(), m.subject.clone())
    };
    let me = ctx.actor;
    let mut to: Vec<ObjId> = Vec::new();
    let candidates =
        std::iter::once(orig_from).chain(if all { orig_to.clone() } else { Vec::new() });
    for id in candidates {
        if id != me && !to.contains(&id) {
            to.push(id);
        }
    }
    if to.is_empty() {
        // Replying to your own message: back to its recipients — or to
        // yourself when you were the only recipient.
        for id in orig_to {
            if id != me && !to.contains(&id) {
                to.push(id);
            }
        }
        if to.is_empty() {
            to.push(me);
        }
    }
    let subject = if orig_subject.to_ascii_lowercase().starts_with("re:") {
        orig_subject
    } else {
        format!("Re: {orig_subject}")
    };
    let urgent = ctx.switches.has("urgent");
    start_compose(ctx, to, subject, Some(msg_id), urgent)
}

fn fwd_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let n = parse_index(ctx.args.lhs())?;
    let players = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("MAIL: Usage: @mail/fwd <n>=<players>"))?
        .to_string();
    let msg_id = resolve_msg(ctx, n)?;
    let (orig_from, subject, body) = {
        let m = state(ctx)
            .message(msg_id)
            .ok_or_else(|| CmdError::msg("MAIL: That message no longer exists."))?;
        (m.from, m.subject.clone(), m.body.clone())
    };
    let to = parse_recipients(ctx.w, &players)?;
    let subject = if subject.to_ascii_lowercase().starts_with("fwd:") {
        subject
    } else {
        format!("Fwd: {subject}")
    };
    let body = format!(
        "[Forwarded from {}]\n{body}",
        display_name(ctx.w, orig_from)
    );
    let urgent = ctx.switches.has("urgent");
    deliver(ctx, &to, &subject, &body, None, urgent)
}

fn read_cmd(ctx: &mut Ctx<'_>, arg: &str) -> Result<(), CmdError> {
    let n = parse_index(arg)?;
    let msg_id = resolve_msg(ctx, n)?;
    let me = ctx.actor;
    let (msg, folder, was_urgent) = {
        let st = state(ctx);
        let m = st
            .message(msg_id)
            .ok_or_else(|| CmdError::msg("MAIL: That message no longer exists."))?
            .clone();
        let urgent = st.entry(me, msg_id).map(|e| e.status) == Some(MailStatus::Urgent);
        (m, st.current_folder(me).to_string(), urgent)
    };
    let from_name = display_name(ctx.w, msg.from);
    let to_names = name_list(ctx.w, &msg.to);
    ctx.out
        .info(me, format!("MAIL: Message {n} in {folder} (id {})", msg.id));
    ctx.out.info(me, format!("From:    {from_name}"));
    ctx.out.info(me, format!("To:      {to_names}"));
    ctx.out
        .info(me, format!("Date:    {}", format_datetime(msg.sent_at)));
    ctx.out.info(
        me,
        format!(
            "Subject: {}{}",
            msg.subject,
            if was_urgent { " (urgent)" } else { "" }
        ),
    );
    if let Some(r) = msg.reply_to {
        ctx.out.info(me, format!("In-Reply-To: message id {r}"));
    }
    ctx.out.info(me, "----");
    for line in msg.body.lines() {
        ctx.out.info(me, line);
    }
    ctx.out.info(me, "----");
    // Mark read (write-through only when the status actually changed).
    let now = ctx.w.now();
    let op = {
        let st = state_mut(ctx);
        let e = st.entry_mut(me, msg_id).expect("resolved entry exists");
        if e.status.is_unread() {
            e.status = MailStatus::Read;
            e.read_at = Some(now);
            Some(data::entry_op(e))
        } else {
            None
        }
    };
    if let Some(op) = op {
        ctx.persist(op);
    }
    Ok(())
}

fn list_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let me = ctx.actor;
    let (folder, rows) = {
        let st = state(ctx);
        let folder = st.current_folder(me).to_string();
        let rows: Vec<(usize, char, ObjId, String, String)> = st
            .folder_ids(me, &folder)
            .iter()
            .enumerate()
            .map(|(i, &id)| {
                let e = st.entry(me, id).expect("listed entry exists");
                let m = st.message(id).expect("entry references stored message");
                let flag = match e.status {
                    MailStatus::Urgent => '!',
                    MailStatus::Unread => 'U',
                    MailStatus::Read => '-',
                };
                (
                    i + 1,
                    flag,
                    m.from,
                    m.subject.clone(),
                    format_date(m.sent_at),
                )
            })
            .collect();
        (folder, rows)
    };
    if rows.is_empty() {
        ctx.out.info(me, format!("MAIL: Folder {folder} is empty."));
        return Ok(());
    }
    let count = rows.len();
    let unread = rows.iter().filter(|r| r.1 != '-').count();
    ctx.out.info(me, format!("--- MAIL: {folder} ---"));
    for (n, flag, from, subject, date) in rows {
        let from = display_name(ctx.w, from);
        ctx.out.info(
            me,
            format!("{n:>3} {flag} {from:<14.14} {subject:<30.30} {date}"),
        );
    }
    ctx.out.info(
        me,
        format!(
            "--- {count} message{}, {unread} unread ---",
            if count == 1 { "" } else { "s" }
        ),
    );
    Ok(())
}

fn folder_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let me = ctx.actor;
    let arg = ctx.args.lhs().trim().to_lowercase();
    if arg.is_empty() {
        // Listing: the three visible built-ins plus user folders (trash is
        // hidden; it can still be switched to by name).
        let lines: Vec<String> = {
            let st = state(ctx);
            let current = st.current_folder(me).to_string();
            let mut names: Vec<String> = [INBOX, SENT, ARCHIVE]
                .iter()
                .map(ToString::to_string)
                .collect();
            names.extend(st.user_folders(me).iter().map(ToString::to_string));
            names
                .iter()
                .map(|f| {
                    let ids = st.folder_ids(me, f);
                    let unread = ids
                        .iter()
                        .filter(|&&id| st.entry(me, id).is_some_and(|e| e.status.is_unread()))
                        .count();
                    format!(
                        "MAIL: {} {f} ({} message{}, {unread} unread)",
                        if *f == current { "*" } else { " " },
                        ids.len(),
                        if ids.len() == 1 { "" } else { "s" }
                    )
                })
                .collect()
        };
        for l in lines {
            ctx.out.info(me, l);
        }
        return Ok(());
    }
    if !state(ctx).folder_exists(me, &arg) {
        return Err(CmdError::msg(format!("MAIL: No folder named {arg}.")));
    }
    state_mut(ctx).box_mut(me).current = SmolStr::new(&arg);
    ctx.out
        .info(me, format!("MAIL: Current folder is now {arg}."));
    Ok(())
}

fn newfolder_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let me = ctx.actor;
    let name = ctx.args.lhs().trim().to_lowercase();
    if !valid_folder_name(&name) {
        return Err(CmdError::msg(
            "MAIL: Folder names are 1-16 characters of a-z, 0-9, '-' and '_'.",
        ));
    }
    if is_builtin_folder(&name) {
        return Err(CmdError::msg(format!("MAIL: {name} is a built-in folder.")));
    }
    let op = {
        let st = state_mut(ctx);
        let b = st.box_mut(me);
        if b.folders.iter().any(|f| f.as_str() == name) {
            return Err(CmdError::msg(format!(
                "MAIL: Folder {name} already exists."
            )));
        }
        if b.folders.len() >= MAX_USER_FOLDERS {
            return Err(CmdError::msg(format!(
                "MAIL: You may create at most {MAX_USER_FOLDERS} folders."
            )));
        }
        b.folders.push(SmolStr::new(&name));
        data::folders_op(me, &b.folders)
    };
    ctx.persist(op);
    ctx.out.info(me, format!("MAIL: Folder {name} created."));
    Ok(())
}

fn rmfolder_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let me = ctx.actor;
    let name = ctx.args.lhs().trim().to_lowercase();
    {
        let st = state(ctx);
        if !st.user_folders(me).iter().any(|f| f.as_str() == name) {
            return Err(CmdError::msg(format!(
                "MAIL: No such folder {name}. (Built-in folders cannot be removed.)"
            )));
        }
        if !st.folder_ids(me, &name).is_empty() {
            return Err(CmdError::msg(format!("MAIL: Folder {name} is not empty.")));
        }
    }
    let op = {
        let st = state_mut(ctx);
        let b = st.box_mut(me);
        b.folders.retain(|f| f.as_str() != name);
        if b.current == name {
            b.current = SmolStr::new(INBOX);
        }
        data::folders_op(me, &b.folders)
    };
    ctx.persist(op);
    ctx.out.info(me, format!("MAIL: Folder {name} removed."));
    Ok(())
}

fn file_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let me = ctx.actor;
    let n = parse_index(ctx.args.lhs())?;
    let folder = ctx
        .args
        .rhs()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("MAIL: Usage: @mail/file <n>=<folder>"))?;
    let msg_id = resolve_msg(ctx, n)?;
    if !state(ctx).folder_exists(me, &folder) {
        return Err(CmdError::msg(format!("MAIL: No folder named {folder}.")));
    }
    move_entry(ctx, me, msg_id, &folder)?;
    ctx.out
        .info(me, format!("MAIL: Message {n} filed in {folder}."));
    Ok(())
}

fn delete_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let me = ctx.actor;
    let n = parse_index(ctx.args.lhs())?;
    let msg_id = resolve_msg(ctx, n)?;
    if state(ctx)
        .entry(me, msg_id)
        .is_some_and(|e| e.folder == TRASH)
    {
        return Err(CmdError::msg("MAIL: That message is already in the trash."));
    }
    move_entry(ctx, me, msg_id, TRASH)?;
    ctx.out.info(
        me,
        format!(
            "MAIL: Message {n} moved to trash. \
             (\"@mail/folder trash\" to review, \"@mail/purge\" to empty.)"
        ),
    );
    Ok(())
}

/// `@mail/purge`: empty the trash for good. Entries are deleted outright;
/// a message document whose last referencing entry was just purged is
/// deleted too (other recipients' copies keep theirs alive).
fn purge_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let me = ctx.actor;
    let trashed: Vec<u64> = state(ctx).folder_ids(me, TRASH);
    if trashed.is_empty() {
        ctx.out.info(me, "MAIL: Your trash is already empty.");
        return Ok(());
    }
    let mut ops = Vec::new();
    {
        let st = state_mut(ctx);
        for &id in &trashed {
            st.remove_entry(me, id);
            ops.push(data::del_op(data::box_key(me, id)));
            if !st.msg_referenced(id) && st.remove_msg(id).is_some() {
                ops.push(data::del_op(data::msg_key(id)));
            }
        }
    }
    for op in ops {
        ctx.persist(op);
    }
    let n = trashed.len();
    ctx.out.info(
        me,
        format!(
            "MAIL: {n} message{} purged from trash.",
            if n == 1 { "" } else { "s" }
        ),
    );
    Ok(())
}

fn unread_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let me = ctx.actor;
    let n = parse_index(ctx.args.lhs())?;
    let msg_id = resolve_msg(ctx, n)?;
    let op = {
        let st = state_mut(ctx);
        let e = st
            .entry_mut(me, msg_id)
            .ok_or_else(|| CmdError::msg("MAIL: That message no longer exists."))?;
        if e.status.is_unread() {
            return Err(CmdError::msg(format!(
                "MAIL: Message {n} is already unread."
            )));
        }
        e.status = MailStatus::Unread;
        e.read_at = None;
        data::entry_op(e)
    };
    ctx.persist(op);
    ctx.out
        .info(me, format!("MAIL: Message {n} marked unread."));
    Ok(())
}

/// Re-file one entry and queue its write-through op.
fn move_entry(ctx: &mut Ctx<'_>, owner: ObjId, msg: u64, folder: &str) -> Result<(), CmdError> {
    let op = {
        let st = state_mut(ctx);
        let e = st
            .entry_mut(owner, msg)
            .ok_or_else(|| CmdError::msg("MAIL: That message no longer exists."))?;
        e.folder = SmolStr::new(folder);
        data::entry_op(e)
    };
    ctx.persist(op);
    Ok(())
}

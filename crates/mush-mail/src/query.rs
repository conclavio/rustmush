//! Web `Req` hydration for `@mail` (M4): the `"mail.list"` query handler,
//! registered via
//! [`Registrar::query_handler`](mush_core::Registrar::query_handler) and
//! invoked by the engine's `Inbound::Query` handling.
//!
//! **Reading mail is deliberately NOT a query.** Queries are read-only by
//! contract, and reading a message *mutates*: it marks the entry Read (and
//! stamps `read_at`), which must write through with command atomicity. The
//! web client therefore reads a message by submitting the ordinary
//! `Line("@mail <n>")` command — same parser, same semantics as SSH — and
//! only *lists* through this handler.

use mush_core::{ObjId, QueryHandler, Role, World, perm};
use mush_proto::{OutputEvent, PanelFrameKind, RichText};
use smol_str::SmolStr;

use crate::data::{ARCHIVE, INBOX, MailState, MailStatus, SENT};

fn err(msg: impl Into<String>) -> RichText {
    RichText::tag("error", msg)
}

/// `"mail.list"` — the mail panel keyframe: every folder with its
/// message/unread counts, plus the entries of one folder.
///
/// Params: `{ "folder": <name | null> }` (mirrors
/// [`Query::MailList`](mush_proto::Query::MailList); `null` lists the
/// inbox). The hidden `trash` folder may be requested explicitly (so
/// mis-deletes can be recovered from the panel) but is never included in
/// the folder summaries, matching `@mail/folder`.
///
/// Reply payload: `Panel { feed: "mail", kind: Keyframe, data: {
///   "folder": <listed folder>,
///   "folders": [ { name, count, unread } ],
///   "entries": [ { index, id, from: { id, name }, subject, status,
///                  sent_at, reply_to } ],
///   "quota": { used, max } | null   // null for quota-exempt staff
/// } }`.
///
/// `index` is the 1-based position `@mail <n>` resolves in that folder, so
/// a panel row can submit `Line("@mail <index>")` — but only against the
/// player's **current** folder selection; the panel should switch first
/// (`Line("@mail/folder <name>")`) when listing another folder.
pub struct ListQuery;

impl QueryHandler for ListQuery {
    fn query(
        &self,
        w: &World,
        actor: ObjId,
        params: &serde_json::Value,
    ) -> Result<OutputEvent, RichText> {
        let st = w
            .module_state::<MailState>()
            .ok_or_else(|| err("Mail is not loaded."))?;
        let folder = params
            .get("folder")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map_or_else(|| INBOX.to_string(), str::to_lowercase);
        if !st.folder_exists(actor, &folder) {
            return Err(err(format!("MAIL: No folder named {folder}.")));
        }

        // Folder summaries: the built-ins then the player's own, exactly
        // the `@mail/folder` listing (trash stays hidden).
        let mut names: Vec<String> = [INBOX, SENT, ARCHIVE]
            .iter()
            .map(ToString::to_string)
            .collect();
        names.extend(st.user_folders(actor).iter().map(ToString::to_string));
        let folders: Vec<serde_json::Value> = names
            .iter()
            .map(|f| {
                let ids = st.folder_ids(actor, f);
                let unread = ids
                    .iter()
                    .filter(|&&id| st.entry(actor, id).is_some_and(|e| e.status.is_unread()))
                    .count();
                serde_json::json!({ "name": f, "count": ids.len(), "unread": unread })
            })
            .collect();

        let entries: Vec<serde_json::Value> = st
            .folder_ids(actor, &folder)
            .iter()
            .enumerate()
            .map(|(i, &id)| {
                let e = st.entry(actor, id).expect("listed entry exists");
                let m = st.message(id).expect("entry references stored message");
                let status = match e.status {
                    MailStatus::Urgent => "urgent",
                    MailStatus::Unread => "unread",
                    MailStatus::Read => "read",
                };
                let from = w
                    .get(m.from)
                    .map_or_else(|| "(vanished)".to_string(), |o| o.name.clone());
                serde_json::json!({
                    "index": i + 1,
                    "id": m.id,
                    "from": { "id": m.from.to_string(), "name": from },
                    "subject": m.subject,
                    "status": status,
                    "sent_at": m.sent_at,
                    "reply_to": m.reply_to,
                })
            })
            .collect();

        // Staff are quota-exempt, mirroring the send path.
        let quota = if perm::has_role(w, actor, Role::Wizard) {
            serde_json::Value::Null
        } else {
            serde_json::json!({ "used": st.entry_count(actor), "max": st.quota() })
        };

        Ok(OutputEvent::Panel {
            feed: SmolStr::new_static("mail"),
            kind: PanelFrameKind::Keyframe,
            data: serde_json::json!({
                "folder": folder,
                "folders": folders,
                "entries": entries,
                "quota": quota,
            }),
        })
    }
}

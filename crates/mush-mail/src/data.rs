//! Mail data model, live state, persistence keys, and the boot rebuild.
//!
//! Storage shape (design doc 04 §8.1): each message body is stored **once**
//! as a [`MailMsg`] document; per-recipient state (folder, read status)
//! lives in small [`MailboxEntry`] documents referencing it. Threads are a
//! client-side derivation of the `reply_to` chain — there is no thread
//! entity.
//!
//! Persisted documents (module namespace `"mail"`, written through
//! [`Ctx::persist`](mush_core::Ctx::persist)):
//!
//! | key                 | value                                     |
//! |---------------------|-------------------------------------------|
//! | `msg:{id}`          | [`MailMsg`]                               |
//! | `box:{owner}:{msg}` | [`MailboxEntry`] (`owner` = bare obj num) |
//! | `folders:{owner}`   | the owner's user-created folder names     |
//! | `next_id`           | next unallocated mail id                  |
//! | `config`            | `{ "quota": <entries> }` (read-only here) |
//!
//! Live state is one [`MailState`] slot on the `World`, rebuilt from those
//! documents in `on_world_loaded`. The **current folder** selection is
//! deliberately transient (resets to `inbox` each boot) — it is parser
//! convenience, not mail data.

use std::collections::BTreeMap;

use mush_core::{ModError, ModuleBootData, ModuleOp, ModuleWrite, ObjId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// The module's persistence namespace (and module id).
pub const NS: &str = "mail";

/// Default folder deliveries land in.
pub const INBOX: &str = "inbox";
/// Folder holding the sender's copy of every message they send.
pub const SENT: &str = "sent";
/// Default keep-it folder.
pub const ARCHIVE: &str = "archive";
/// Hidden soft-delete folder: `@mail/delete` files entries here. It is
/// omitted from folder listings but can be switched to (so mis-deletes can
/// be `@mail/file`d back out); `@mail/purge` empties it for good. Trashed
/// entries do not count against the quota.
pub const TRASH: &str = "trash";

/// Default per-player mailbox quota (total entries across all folders),
/// overridable via the persisted `config` document. Staff (Wizard+) are
/// exempt.
pub const DEFAULT_QUOTA: usize = 300;
/// Maximum user-created folders per player.
pub const MAX_USER_FOLDERS: usize = 16;
/// Maximum folder-name length (after lowercasing).
pub const MAX_FOLDER_NAME: usize = 16;
/// Maximum recipients retained in one message and fanned out in one commit.
pub const MAX_MAIL_RECIPIENTS: usize = 32;
/// Maximum UTF-8 bytes in one subject.
pub const MAX_MAIL_SUBJECT_BYTES: usize = 512;
/// Maximum UTF-8 bytes in one message body.
pub const MAX_MAIL_BODY_BYTES: usize = 64 * 1024;
/// Maximum lines in one message body (also bounds compose preview fan-out).
pub const MAX_MAIL_BODY_LINES: usize = 512;

/// The `next_id` kv key.
pub const NEXT_ID_KEY: &str = "next_id";
/// The `config` kv key.
pub const CONFIG_KEY: &str = "config";

/// Whether `name` is one of the built-in folders (including the hidden
/// trash) — these always exist and can never be created or removed.
#[must_use]
pub fn is_builtin_folder(name: &str) -> bool {
    matches!(name, INBOX | SENT | ARCHIVE | TRASH)
}

/// A folder name is 1–16 chars of `[a-z0-9_-]` (callers lowercase first).
#[must_use]
pub fn valid_folder_name(name: &str) -> bool {
    (1..=MAX_FOLDER_NAME).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

// ----------------------------------------------------------------------
// Documents
// ----------------------------------------------------------------------

/// One mail message, stored once regardless of recipient count.
///
/// `sent_at` is a Unix timestamp in **UTC seconds** — the terminal renders
/// it via [`crate::date`] (UTC policy; the web client localizes later).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailMsg {
    /// Monotonic mail id (allocated from the module's `next_id`).
    pub id: u64,
    /// The sender.
    pub from: ObjId,
    /// Every recipient it was actually delivered to.
    pub to: Vec<ObjId>,
    /// Subject line.
    pub subject: String,
    /// Message body (newline-joined compose lines).
    pub body: String,
    /// The message this one replies to, if any (threads derive from this).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<u64>,
    /// Unix send time, UTC seconds.
    pub sent_at: i64,
}

/// Per-recipient read status.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailStatus {
    /// Delivered, not yet read.
    Unread,
    /// Read (reading an urgent message also lands here).
    Read,
    /// Delivered with the urgent flag, not yet read.
    Urgent,
}

impl MailStatus {
    /// Whether the entry still counts as unread.
    #[must_use]
    pub fn is_unread(self) -> bool {
        matches!(self, MailStatus::Unread | MailStatus::Urgent)
    }
}

/// One recipient's handle on one message: which folder it is filed in and
/// whether they have read it. The body lives in the referenced [`MailMsg`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxEntry {
    /// The mailbox owner.
    pub owner: ObjId,
    /// The referenced message id.
    pub msg: u64,
    /// The folder this entry is filed in.
    pub folder: SmolStr,
    /// Read status.
    pub status: MailStatus,
    /// When the owner read it (Unix UTC seconds), if they have.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_at: Option<i64>,
}

// ----------------------------------------------------------------------
// Live state
// ----------------------------------------------------------------------

/// One player's mailbox: entries by message id, their user-created folders,
/// and the (transient) current-folder selection.
#[derive(Clone, Debug)]
pub struct Mailbox {
    /// Entries keyed by message id (`BTreeMap` keeps listings id-ordered).
    pub(crate) entries: BTreeMap<u64, MailboxEntry>,
    /// User-created folder names (creation order; cap
    /// [`MAX_USER_FOLDERS`]).
    pub(crate) folders: Vec<SmolStr>,
    /// The folder `@mail` lists from. Transient — resets to inbox on boot.
    pub(crate) current: SmolStr,
}

impl Default for Mailbox {
    fn default() -> Mailbox {
        Mailbox {
            entries: BTreeMap::new(),
            folders: Vec::new(),
            current: SmolStr::new(INBOX),
        }
    }
}

/// The mail module's whole live state, kept in a typed module-state slot on
/// the `World` and rebuilt from persisted documents at boot.
#[derive(Clone, Debug)]
pub struct MailState {
    msgs: BTreeMap<u64, MailMsg>,
    boxes: BTreeMap<ObjId, Mailbox>,
    next_id: u64,
    quota: usize,
}

impl Default for MailState {
    fn default() -> MailState {
        MailState {
            msgs: BTreeMap::new(),
            boxes: BTreeMap::new(),
            next_id: 1,
            quota: DEFAULT_QUOTA,
        }
    }
}

impl MailState {
    /// The message with this id, if it exists.
    #[must_use]
    pub fn message(&self, id: u64) -> Option<&MailMsg> {
        self.msgs.get(&id)
    }

    /// `owner`'s entry for message `msg`, if any.
    #[must_use]
    pub fn entry(&self, owner: ObjId, msg: u64) -> Option<&MailboxEntry> {
        self.boxes.get(&owner).and_then(|b| b.entries.get(&msg))
    }

    /// Message ids filed in `owner`'s `folder`, ascending (the listing
    /// order `@mail` numbers against).
    #[must_use]
    pub fn folder_ids(&self, owner: ObjId, folder: &str) -> Vec<u64> {
        self.boxes.get(&owner).map_or_else(Vec::new, |b| {
            b.entries
                .values()
                .filter(|e| e.folder == folder)
                .map(|e| e.msg)
                .collect()
        })
    }

    /// `(unread, urgent)` counts for `owner`, excluding the hidden trash.
    #[must_use]
    pub fn unread_counts(&self, owner: ObjId) -> (u32, u32) {
        let mut unread = 0u32;
        let mut urgent = 0u32;
        if let Some(b) = self.boxes.get(&owner) {
            for e in b.entries.values() {
                if e.folder != TRASH && e.status.is_unread() {
                    unread += 1;
                    if e.status == MailStatus::Urgent {
                        urgent += 1;
                    }
                }
            }
        }
        (unread, urgent)
    }

    /// Entries `owner` holds against their quota: every folder **except
    /// the trash**. Trash is excluded so that a player at quota can always
    /// free space themselves (`@mail/delete` then `@mail/purge`) — if
    /// trashed mail still counted, a full mailbox would refuse all inbound
    /// mail forever with no self-service way out.
    #[must_use]
    pub fn entry_count(&self, owner: ObjId) -> usize {
        self.boxes.get(&owner).map_or(0, |b| {
            b.entries.values().filter(|e| e.folder != TRASH).count()
        })
    }

    /// `owner`'s current folder (the one `@mail` lists).
    #[must_use]
    pub fn current_folder(&self, owner: ObjId) -> &str {
        self.boxes.get(&owner).map_or(INBOX, |b| b.current.as_str())
    }

    /// `owner`'s user-created folders, in creation order.
    #[must_use]
    pub fn user_folders(&self, owner: ObjId) -> &[SmolStr] {
        self.boxes.get(&owner).map_or(&[], |b| b.folders.as_slice())
    }

    /// Whether `folder` exists for `owner` (built-ins always do).
    #[must_use]
    pub fn folder_exists(&self, owner: ObjId, folder: &str) -> bool {
        is_builtin_folder(folder)
            || self
                .boxes
                .get(&owner)
                .is_some_and(|b| b.folders.iter().any(|f| f == folder))
    }

    /// The configured per-player quota.
    #[must_use]
    pub fn quota(&self) -> usize {
        self.quota
    }

    /// Override the quota (config application; tests).
    pub fn set_quota(&mut self, quota: usize) {
        self.quota = quota;
    }

    /// The next mail id that would be allocated.
    #[must_use]
    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    /// Allocate the next mail id, or return `None` when the counter cannot
    /// advance without wrapping.
    pub(crate) fn alloc_id(&mut self) -> Option<u64> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1)?;
        Some(id)
    }

    /// `owner`'s mailbox, created on first touch.
    pub(crate) fn box_mut(&mut self, owner: ObjId) -> &mut Mailbox {
        self.boxes.entry(owner).or_default()
    }

    /// Store a message document.
    pub(crate) fn insert_msg(&mut self, msg: MailMsg) {
        self.msgs.insert(msg.id, msg);
    }

    /// Mutable access to one entry.
    pub(crate) fn entry_mut(&mut self, owner: ObjId, msg: u64) -> Option<&mut MailboxEntry> {
        self.boxes
            .get_mut(&owner)
            .and_then(|b| b.entries.get_mut(&msg))
    }

    /// Whether ANY mailbox still holds an entry for message `msg`.
    #[must_use]
    pub fn msg_referenced(&self, msg: u64) -> bool {
        self.boxes.values().any(|b| b.entries.contains_key(&msg))
    }

    /// Remove one entry outright (purge path). Returns it.
    pub(crate) fn remove_entry(&mut self, owner: ObjId, msg: u64) -> Option<MailboxEntry> {
        self.boxes
            .get_mut(&owner)
            .and_then(|b| b.entries.remove(&msg))
    }

    /// Drop a message document (last reference purged).
    pub(crate) fn remove_msg(&mut self, id: u64) -> Option<MailMsg> {
        self.msgs.remove(&id)
    }

    /// Remove `owner`'s whole mailbox (destroyed characters). Returns it.
    pub(crate) fn remove_box(&mut self, owner: ObjId) -> Option<Mailbox> {
        self.boxes.remove(&owner)
    }
}

// ----------------------------------------------------------------------
// Persistence keys + ops
// ----------------------------------------------------------------------

/// The kv key a message document lives under.
#[must_use]
pub fn msg_key(id: u64) -> SmolStr {
    SmolStr::new(format!("msg:{id}"))
}

/// The kv key one owner's entry for one message lives under.
#[must_use]
pub fn box_key(owner: ObjId, msg: u64) -> SmolStr {
    SmolStr::new(format!("box:{}:{}", owner.0, msg))
}

/// The kv key one owner's user-folder list lives under.
#[must_use]
pub fn folders_key(owner: ObjId) -> SmolStr {
    SmolStr::new(format!("folders:{}", owner.0))
}

fn json_set(key: SmolStr, value: serde_json::Value) -> ModuleOp {
    ModuleOp {
        ns: SmolStr::new(NS),
        op: ModuleWrite::JsonSet { key, value },
    }
}

/// The write-through op for a message document.
pub(crate) fn msg_op(msg: &MailMsg) -> ModuleOp {
    json_set(
        msg_key(msg.id),
        serde_json::to_value(msg).expect("mail message serializes"),
    )
}

/// The write-through op for one mailbox entry.
pub(crate) fn entry_op(e: &MailboxEntry) -> ModuleOp {
    json_set(
        box_key(e.owner, e.msg),
        serde_json::to_value(e).expect("mailbox entry serializes"),
    )
}

/// The write-through op for one owner's user-folder list.
pub(crate) fn folders_op(owner: ObjId, folders: &[SmolStr]) -> ModuleOp {
    json_set(
        folders_key(owner),
        serde_json::to_value(folders).expect("folder list serializes"),
    )
}

/// The write-through op for the id counter.
pub(crate) fn next_id_op(next: u64) -> ModuleOp {
    json_set(SmolStr::new(NEXT_ID_KEY), serde_json::Value::from(next))
}

/// A `Del` write-through op for one key in the mail namespace (purge,
/// destroyed-character cleanup).
pub(crate) fn del_op(key: SmolStr) -> ModuleOp {
    ModuleOp {
        ns: SmolStr::new(NS),
        op: ModuleWrite::Del { key },
    }
}

/// Number of logical lines in a stored body. The empty body has zero lines.
#[must_use]
pub(crate) fn body_line_count(body: &str) -> usize {
    if body.is_empty() {
        0
    } else {
        body.as_bytes().iter().filter(|&&b| b == b'\n').count() + 1
    }
}

/// Validate the hard per-message storage/fan-out bounds.
pub(crate) fn validate_message(msg: &MailMsg) -> Result<(), String> {
    if msg.id == 0 {
        return Err("message id zero is reserved".to_string());
    }
    validate_message_content(&msg.to, &msg.subject, &msg.body)
        .map_err(|detail| format!("message {} {detail}", msg.id))
}

/// Validate content before allocating an id or mutating live mail state.
pub(crate) fn validate_message_content(
    to: &[ObjId],
    subject: &str,
    body: &str,
) -> Result<(), String> {
    if to.is_empty() || to.len() > MAX_MAIL_RECIPIENTS {
        return Err(format!(
            "has {} recipients; expected 1..={MAX_MAIL_RECIPIENTS}",
            to.len()
        ));
    }
    if subject.trim().is_empty() || subject.len() > MAX_MAIL_SUBJECT_BYTES {
        return Err(format!(
            "subject is empty or exceeds {MAX_MAIL_SUBJECT_BYTES} bytes"
        ));
    }
    if body.len() > MAX_MAIL_BODY_BYTES || body_line_count(body) > MAX_MAIL_BODY_LINES {
        return Err(format!(
            "body exceeds {MAX_MAIL_BODY_BYTES} bytes or {MAX_MAIL_BODY_LINES} lines"
        ));
    }
    Ok(())
}

// ----------------------------------------------------------------------
// Boot rebuild
// ----------------------------------------------------------------------

/// Rebuild the whole [`MailState`] from the module's persisted documents.
///
/// Malformed documents fail the boot loudly (bad mail data is an
/// operational problem, not something to paper over); the one tolerated
/// inconsistency is a mailbox entry whose message document is missing
/// (e.g. a partially-completed purge) — such orphans are dropped. Unknown
/// keys are ignored for forward compatibility.
pub(crate) fn rebuild(data: &ModuleBootData) -> Result<MailState, ModError> {
    let mut st = MailState::default();
    let mut pending: Vec<MailboxEntry> = Vec::new();
    let mut stored_next: u64 = 1;

    for (key, value) in &data.kv {
        if key == CONFIG_KEY {
            if let Some(q) = value.get("quota").and_then(serde_json::Value::as_u64) {
                st.quota = usize::try_from(q).unwrap_or(usize::MAX);
            }
        } else if key == NEXT_ID_KEY {
            stored_next = value
                .as_u64()
                .ok_or_else(|| ModError::msg("mail: next_id is not a number"))?;
        } else if key.starts_with("msg:") {
            let msg: MailMsg = serde_json::from_value(value.clone())
                .map_err(|e| ModError::msg(format!("mail: bad document {key:?}: {e}")))?;
            validate_message(&msg)
                .map_err(|detail| ModError::msg(format!("mail: bad document {key:?}: {detail}")))?;
            st.msgs.insert(msg.id, msg);
        } else if key.starts_with("box:") {
            let entry: MailboxEntry = serde_json::from_value(value.clone())
                .map_err(|e| ModError::msg(format!("mail: bad document {key:?}: {e}")))?;
            pending.push(entry);
        } else if let Some(rest) = key.strip_prefix("folders:") {
            let owner = rest
                .parse::<u32>()
                .map(ObjId)
                .map_err(|_| ModError::msg(format!("mail: bad folders key {key:?}")))?;
            let folders: Vec<SmolStr> = serde_json::from_value(value.clone())
                .map_err(|e| ModError::msg(format!("mail: bad document {key:?}: {e}")))?;
            st.box_mut(owner).folders = folders;
        }
        // Unknown keys: ignored (additive evolution).
    }

    for e in pending {
        if st.msgs.contains_key(&e.msg) {
            st.box_mut(e.owner).entries.insert(e.msg, e);
        }
        // Orphan entry (message doc gone): dropped, see above.
    }

    let max_msg = st.msgs.keys().next_back().copied().unwrap_or(0);
    let derived_next = max_msg
        .checked_add(1)
        .ok_or_else(|| ModError::msg("mail: message id space is exhausted"))?;
    st.next_id = stored_next.max(derived_next).max(1);
    Ok(st)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn folder_name_rules() {
        assert!(valid_folder_name("projects"));
        assert!(valid_folder_name("a"));
        assert!(valid_folder_name("x-y_9"));
        assert!(!valid_folder_name(""));
        assert!(!valid_folder_name("Projects")); // callers lowercase first
        assert!(!valid_folder_name("has space"));
        assert!(!valid_folder_name("seventeen-letters!"));
        assert!(!valid_folder_name(&"a".repeat(MAX_FOLDER_NAME + 1)));
        assert!(is_builtin_folder(INBOX));
        assert!(is_builtin_folder(TRASH));
        assert!(!is_builtin_folder("projects"));
    }

    #[test]
    fn rebuild_drops_orphans_and_derives_next_id() {
        let msg = MailMsg {
            id: 7,
            from: ObjId(2),
            to: vec![ObjId(3)],
            subject: "hi".into(),
            body: "there".into(),
            reply_to: None,
            sent_at: 100,
        };
        let entry = MailboxEntry {
            owner: ObjId(3),
            msg: 7,
            folder: SmolStr::new(INBOX),
            status: MailStatus::Unread,
            read_at: None,
        };
        let orphan = MailboxEntry {
            owner: ObjId(3),
            msg: 99, // no msg:99 document
            ..entry.clone()
        };
        let data = ModuleBootData {
            kv: vec![
                (box_key(ObjId(3), 7), serde_json::to_value(&entry).unwrap()),
                (
                    box_key(ObjId(3), 99),
                    serde_json::to_value(&orphan).unwrap(),
                ),
                (msg_key(7), serde_json::to_value(&msg).unwrap()),
                (SmolStr::new(NEXT_ID_KEY), json!(3)), // behind max msg id
                (SmolStr::new(CONFIG_KEY), json!({"quota": 5})),
                (SmolStr::new("mystery"), json!("ignored")),
            ],
            streams: std::collections::HashMap::new(),
            ..Default::default()
        };
        let st = rebuild(&data).unwrap();
        assert_eq!(st.message(7), Some(&msg));
        assert_eq!(st.entry(ObjId(3), 7), Some(&entry));
        assert_eq!(st.entry(ObjId(3), 99), None, "orphan dropped");
        assert_eq!(st.next_id(), 8, "next_id derived past max msg id");
        assert_eq!(st.quota(), 5);
    }

    #[test]
    fn rebuild_rejects_malformed_documents() {
        let data = ModuleBootData {
            kv: vec![(msg_key(1), json!({"not": "a message"}))],
            streams: std::collections::HashMap::new(),
            ..Default::default()
        };
        assert!(rebuild(&data).is_err());
        let data = ModuleBootData {
            kv: vec![(SmolStr::new(NEXT_ID_KEY), json!("nan"))],
            streams: std::collections::HashMap::new(),
            ..Default::default()
        };
        assert!(rebuild(&data).is_err());
    }

    #[test]
    fn rebuild_rejects_oversized_content_and_exhausted_ids() {
        let oversized = MailMsg {
            id: 1,
            from: ObjId(2),
            to: vec![ObjId(3)],
            subject: "hi".into(),
            body: "x".repeat(MAX_MAIL_BODY_BYTES + 1),
            reply_to: None,
            sent_at: 100,
        };
        let data = ModuleBootData {
            kv: vec![(msg_key(1), serde_json::to_value(oversized).unwrap())],
            streams: std::collections::HashMap::new(),
            ..Default::default()
        };
        assert!(rebuild(&data).is_err());

        let exhausted = MailMsg {
            id: u64::MAX,
            from: ObjId(2),
            to: vec![ObjId(3)],
            subject: "hi".into(),
            body: "there".into(),
            reply_to: None,
            sent_at: 100,
        };
        let data = ModuleBootData {
            kv: vec![(msg_key(u64::MAX), serde_json::to_value(exhausted).unwrap())],
            streams: std::collections::HashMap::new(),
            ..Default::default()
        };
        assert!(rebuild(&data).is_err());

        let mut state = MailState {
            next_id: u64::MAX - 1,
            ..MailState::default()
        };
        assert_eq!(state.alloc_id(), Some(u64::MAX - 1));
        assert_eq!(state.next_id(), u64::MAX);
        assert_eq!(state.alloc_id(), None);
        assert_eq!(state.next_id(), u64::MAX, "refusal must not wrap");
    }

    #[test]
    fn status_serde_and_unread() {
        assert_eq!(
            serde_json::to_value(MailStatus::Urgent).unwrap(),
            json!("urgent")
        );
        assert!(MailStatus::Unread.is_unread());
        assert!(MailStatus::Urgent.is_unread());
        assert!(!MailStatus::Read.is_unread());
    }
}

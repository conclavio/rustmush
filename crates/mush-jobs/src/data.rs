//! Job and note data shapes, the in-RAM state slot, and persistence keys.
//!
//! Everything here is plain data with serde round-tripping: the persisted
//! documents at `job:{id}`, `note:{char}:{category}`, and `meta` are the
//! serde_json form of these structs, written through [`mush_core::ModuleOp`]
//! and read back at boot via
//! [`GameModule::on_world_loaded`](mush_core::GameModule::on_world_loaded).

use std::collections::BTreeMap;
use std::fmt;

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// The jobs module's persistence namespace (documents live at
/// `mod:jobs:*` in the store).
pub const NS: &str = "jobs";

/// The kv key of the module metadata document (`{"next_id": N}`), which
/// keeps job ids sequential across reboots.
pub const META_KEY: &str = "meta";

/// The core player-request queue, registered by the jobs module itself.
pub const REQ_QUEUE: &str = "REQ";

/// Maximum UTF-8 bytes in one job title.
pub const MAX_JOB_TITLE_BYTES: usize = 512;
/// Maximum UTF-8 bytes in one job comment.
pub const MAX_JOB_COMMENT_BYTES: usize = 16 * 1024;
/// Hard persisted comment cap, including terminal staff audit comments.
pub const MAX_JOB_COMMENTS: usize = 256;
/// Ordinary comment cap. The remainder is reserved for terminal staff actions.
pub const MAX_ORDINARY_JOB_COMMENTS: usize = 252;

/// Where a job stands.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobStatus {
    /// Newly opened, nobody working it.
    Open,
    /// Assigned / being worked.
    InProgress,
    /// Closed successfully (`+job/approve`).
    Resolved,
    /// Closed with a refusal (`+job/deny`).
    Denied,
}

impl JobStatus {
    /// Parse a user-typed status name (`open`, `inprogress`, `in-progress`,
    /// `wip`, `resolved`, `done`, `denied`), case-insensitively.
    #[must_use]
    pub fn parse(s: &str) -> Option<JobStatus> {
        let key: String = s
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        match key.as_str() {
            "open" => Some(JobStatus::Open),
            "inprogress" | "wip" => Some(JobStatus::InProgress),
            "resolved" | "done" => Some(JobStatus::Resolved),
            "denied" => Some(JobStatus::Denied),
            _ => None,
        }
    }

    /// Whether the job is closed (`Resolved` or `Denied`).
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, JobStatus::Resolved | JobStatus::Denied)
    }
}

impl fmt::Display for JobStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            JobStatus::Open => "Open",
            JobStatus::InProgress => "In progress",
            JobStatus::Resolved => "Resolved",
            JobStatus::Denied => "Denied",
        };
        // `pad`, not `write_str`, so width specs in listings work.
        f.pad(s)
    }
}

/// One entry in a job's comment trail.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobComment {
    /// The commenter.
    pub who: ObjId,
    /// When (unix seconds, world clock).
    pub ts: i64,
    /// The comment text.
    pub text: String,
}

/// A structured payload attached to a job: `handler` names a registered
/// [`PayloadHandlerFn`](crate::PayloadHandlerFn) (e.g. the WoD XP applier)
/// which `+job/approve` invokes with `data` before flipping the status.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobPayload {
    /// The registered payload-handler name.
    pub handler: SmolStr,
    /// Handler-defined data (a sheet diff, a chargen submission, …).
    pub data: serde_json::Value,
}

/// One ticket. Persisted whole at `job:{id}` on every mutation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    /// Sequential id (1-based; never reused).
    pub id: u64,
    /// The queue it lives in (canonical registered name, e.g. `"REQ"`).
    pub queue: SmolStr,
    /// One-line title.
    pub title: String,
    /// Who opened it (always allowed to see it).
    pub opener: ObjId,
    /// The staffer working it, if assigned (always allowed to see it).
    #[serde(default)]
    pub assignee: Option<ObjId>,
    /// Where it stands.
    pub status: JobStatus,
    /// Free-form tags (set by modules opening jobs through the API; no
    /// player command edits them).
    #[serde(default)]
    pub tags: Vec<SmolStr>,
    /// The comment trail, oldest first.
    #[serde(default)]
    pub comments: Vec<JobComment>,
    /// Optional structured payload consumed by `+job/approve`.
    #[serde(default)]
    pub payload: Option<JobPayload>,
    /// When the payload handler actually **ran** (unix seconds), for a
    /// payload-bearing job (E4c verify fold, E3).
    ///
    /// `status == Resolved` is a *state*: staff set it with `+job/status`, and before
    /// the fold that flip was indistinguishable — to the economy's fact provider, and
    /// therefore to a contract settling on `jobs.resolved` — from an approval that
    /// had actually applied the payload. It is the applied marker, not the status,
    /// that is a fact about the past: `+job/approve` writes it in the same command,
    /// the same `WriteBatch`, as the handler's own mutations, and nothing ever
    /// clears it.
    #[serde(default)]
    pub payload_applied: Option<i64>,
    /// When it was opened (unix seconds).
    pub opened_at: i64,
    /// Last mutation time (unix seconds).
    pub updated_at: i64,
}

/// An approval stamp on a character note.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    /// The approving staffer.
    pub by: ObjId,
    /// When (unix seconds).
    pub at: i64,
}

/// One character note. A character has at most one note per category;
/// persisted whole at `note:{char}:{category}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// The character the note is about.
    pub owner: ObjId,
    /// Category key (canonically lowercased).
    pub category: SmolStr,
    /// The note body.
    pub text: String,
    /// Who wrote (last edited) it.
    pub author: ObjId,
    /// Hidden from the character themselves; staff-set only.
    #[serde(default)]
    pub staff_only: bool,
    /// The approval stamp, if approved. Editing the note clears it.
    #[serde(default)]
    pub approved: Option<Approval>,
}

/// The in-RAM working state, living in a typed module state slot on the
/// [`World`](mush_core::World). Persisted documents are the source of
/// truth; this is the full read cache rebuilt at boot.
#[derive(Clone, Debug, Default)]
pub(crate) struct JobsState {
    /// All jobs by id.
    pub jobs: BTreeMap<u64, Job>,
    /// The next id to allocate (0 means "never allocated": ids start at 1).
    pub next_id: u64,
    /// All notes by `(character, category)`.
    pub notes: BTreeMap<(ObjId, SmolStr), Note>,
}

impl JobsState {
    /// Allocate the next sequential job id, or return `None` when the counter
    /// cannot advance without wrapping.
    pub fn alloc_id(&mut self) -> Option<u64> {
        if self.next_id == 0 {
            self.next_id = 1;
        }
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1)?;
        Some(id)
    }
}

/// Validate one comment's content independently of trail capacity.
pub(crate) fn validate_comment_text(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("a job comment cannot be empty".to_string());
    }
    if text.len() > MAX_JOB_COMMENT_BYTES {
        return Err(format!(
            "a job comment may contain at most {MAX_JOB_COMMENT_BYTES} UTF-8 bytes"
        ));
    }
    Ok(())
}

/// Validate an ordinary comment append, preserving terminal audit capacity.
pub(crate) fn validate_ordinary_comment(job: &Job, text: &str) -> Result<(), String> {
    validate_comment_text(text)?;
    if job.comments.len() >= MAX_ORDINARY_JOB_COMMENTS {
        return Err(format!(
            "job {} has reached its ordinary comment limit of {MAX_ORDINARY_JOB_COMMENTS}",
            job.id
        ));
    }
    Ok(())
}

/// Validate a terminal approve/deny comment against the absolute trail cap.
pub(crate) fn validate_terminal_comment(job: &Job, text: &str) -> Result<(), String> {
    validate_comment_text(text)?;
    if job.comments.len() >= MAX_JOB_COMMENTS {
        return Err(format!(
            "job {} has reached its hard comment limit of {MAX_JOB_COMMENTS}",
            job.id
        ));
    }
    Ok(())
}

/// Validate the bounded portions of a persisted job document.
pub(crate) fn validate_job(job: &Job) -> Result<(), String> {
    if job.id == 0 {
        return Err("job id zero is reserved".to_string());
    }
    if job.title.trim().is_empty() || job.title.len() > MAX_JOB_TITLE_BYTES {
        return Err(format!(
            "job {} title is empty or exceeds {MAX_JOB_TITLE_BYTES} UTF-8 bytes",
            job.id
        ));
    }
    if job.comments.len() > MAX_JOB_COMMENTS {
        return Err(format!(
            "job {} has {} comments; maximum is {MAX_JOB_COMMENTS}",
            job.id,
            job.comments.len()
        ));
    }
    for comment in &job.comments {
        validate_comment_text(&comment.text)
            .map_err(|detail| format!("job {}: {detail}", job.id))?;
    }
    Ok(())
}

/// The persistence key of a job document.
#[must_use]
pub fn job_key(id: u64) -> SmolStr {
    SmolStr::new(format!("job:{id}"))
}

/// The persistence key of a note document. `category` is stored lowercased.
#[must_use]
pub fn note_key(owner: ObjId, category: &str) -> SmolStr {
    SmolStr::new(format!("note:{}:{}", owner.0, category))
}

/// The metadata document body.
#[must_use]
pub fn meta_value(next_id: u64) -> serde_json::Value {
    serde_json::json!({ "next_id": next_id })
}

/// Canonicalize a note category for keying: trimmed and lowercased.
#[must_use]
pub fn canon_category(category: &str) -> SmolStr {
    SmolStr::new(category.trim().to_lowercase())
}

/// A compact "how long ago" for MUSH output (`42s`, `5m`, `3h`, `2d`).
#[must_use]
pub fn fmt_ago(now: i64, ts: i64) -> String {
    let d = (now - ts).max(0);
    if d < 60 {
        format!("{d}s")
    } else if d < 3600 {
        format!("{}m", d / 60)
    } else if d < 86_400 {
        format!("{}h", d / 3600)
    } else {
        format!("{}d", d / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_parse_and_display() {
        assert_eq!(JobStatus::parse("Open"), Some(JobStatus::Open));
        assert_eq!(JobStatus::parse("in-progress"), Some(JobStatus::InProgress));
        assert_eq!(JobStatus::parse("In Progress"), Some(JobStatus::InProgress));
        assert_eq!(JobStatus::parse("WIP"), Some(JobStatus::InProgress));
        assert_eq!(JobStatus::parse("resolved"), Some(JobStatus::Resolved));
        assert_eq!(JobStatus::parse("done"), Some(JobStatus::Resolved));
        assert_eq!(JobStatus::parse("DENIED"), Some(JobStatus::Denied));
        assert_eq!(JobStatus::parse("bogus"), None);
        assert_eq!(JobStatus::InProgress.to_string(), "In progress");
        assert!(JobStatus::Resolved.is_terminal());
        assert!(JobStatus::Denied.is_terminal());
        assert!(!JobStatus::Open.is_terminal());
        assert!(!JobStatus::InProgress.is_terminal());
    }

    #[test]
    fn id_allocation_is_sequential_from_one() {
        let mut st = JobsState::default();
        assert_eq!(st.alloc_id(), Some(1));
        assert_eq!(st.alloc_id(), Some(2));
        st.next_id = 42;
        assert_eq!(st.alloc_id(), Some(42));
        assert_eq!(st.next_id, 43);
        st.next_id = u64::MAX;
        assert_eq!(st.alloc_id(), None);
        assert_eq!(st.next_id, u64::MAX);
    }

    #[test]
    fn keys_and_helpers() {
        assert_eq!(job_key(7).as_str(), "job:7");
        assert_eq!(
            note_key(ObjId(5), "background").as_str(),
            "note:5:background"
        );
        assert_eq!(canon_category("  BackGround "), "background");
        assert_eq!(meta_value(3), serde_json::json!({"next_id": 3}));
        assert_eq!(fmt_ago(1000, 990), "10s");
        assert_eq!(fmt_ago(1000, 1000 - 300), "5m");
        assert_eq!(fmt_ago(1000 + 7200, 1000), "2h");
        assert_eq!(fmt_ago(200_000, 0), "2d");
        assert_eq!(fmt_ago(0, 50), "0s");
    }

    #[test]
    fn job_and_note_serde_round_trip() {
        let job = Job {
            id: 3,
            queue: "REQ".into(),
            title: "Fix my desc".into(),
            opener: ObjId(5),
            assignee: Some(ObjId(2)),
            status: JobStatus::InProgress,
            tags: vec!["xp".into()],
            comments: vec![JobComment {
                who: ObjId(5),
                ts: 100,
                text: "please".into(),
            }],
            payload: Some(JobPayload {
                handler: "wod.xp".into(),
                data: serde_json::json!({"dots": 1}),
            }),
            payload_applied: None,
            opened_at: 100,
            updated_at: 200,
        };
        let v = serde_json::to_value(&job).unwrap();
        let back: Job = serde_json::from_value(v).unwrap();
        assert_eq!(back, job);

        let note = Note {
            owner: ObjId(5),
            category: "background".into(),
            text: "Grew up in Limbo.".into(),
            author: ObjId(2),
            staff_only: true,
            approved: Some(Approval {
                by: ObjId(1),
                at: 150,
            }),
        };
        let v = serde_json::to_value(&note).unwrap();
        let back: Note = serde_json::from_value(v).unwrap();
        assert_eq!(back, note);

        // Older documents without the defaulted fields still parse.
        let minimal = serde_json::json!({
            "id": 1, "queue": "REQ", "title": "t", "opener": 5,
            "status": "Open", "opened_at": 1, "updated_at": 1
        });
        let j: Job = serde_json::from_value(minimal).unwrap();
        assert_eq!(j.assignee, None);
        assert!(j.tags.is_empty() && j.comments.is_empty() && j.payload.is_none());
    }

    #[test]
    fn persisted_job_bounds_are_validated() {
        let mut job = Job {
            id: 1,
            queue: REQ_QUEUE.into(),
            title: "bounded".into(),
            opener: ObjId(2),
            assignee: None,
            status: JobStatus::Open,
            tags: Vec::new(),
            comments: Vec::new(),
            payload: None,
            payload_applied: None,
            opened_at: 1,
            updated_at: 1,
        };
        assert!(validate_job(&job).is_ok());
        job.title = "x".repeat(MAX_JOB_TITLE_BYTES + 1);
        assert!(validate_job(&job).is_err());
        job.title = "bounded".into();
        job.comments = (0..=MAX_JOB_COMMENTS)
            .map(|n| JobComment {
                who: ObjId(2),
                ts: n as i64,
                text: format!("comment {n}"),
            })
            .collect();
        assert!(validate_job(&job).is_err());
    }
}

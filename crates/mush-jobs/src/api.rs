//! The public, module-facing API: payload handlers and programmatic job
//! opening.
//!
//! # The payload-handler pattern
//!
//! The jobs module owns a registry of **payload handlers** living in a
//! typed module state slot ([`JobsApi`]) on the
//! [`World`](mush_core::World). Because state slots are keyed by `TypeId`
//! and `JobsApi` is exported, any other module can reach the registry
//! through the module API alone — no direct dependency on the jobs
//! module's internals, no second registry:
//!
//! 1. The jobs module declares the slot at registration
//!    (`reg.state(JobsApi::default())`), so it exists on the world from
//!    engine construction.
//! 2. A producing module (e.g. the WoD ruleset in M6) calls
//!    [`register_payload_handler`] — typically from its own
//!    `on_world_loaded`, which receives a full
//!    [`ModuleCtx`](mush_core::ModuleCtx) whose `ctx.w` is the world.
//! 3. The same module opens jobs carrying a [`JobPayload`] via
//!    [`open_job`] (e.g. `+xp/spend` files a job in its `xp` queue with
//!    the sheet diff as `data`).
//! 4. When staff run `+job/approve`, the jobs module looks the handler up
//!    by name and invokes it **before** flipping the status: the handler's
//!    world mutations, its queued persistence ops, and the job's own
//!    status write all ride the same command — one `WriteBatch`, one
//!    atomic commit. A handler error leaves the job untouched and is
//!    reported to the approver — and dispatch **discards every
//!    persistence op the failed command deferred** (the queue is
//!    truncated back on `Err`), so a handler that queued `ctx.persist`
//!    writes before failing never half-commits them.
//!
//! Handlers must still validate-then-apply for **world mutations**: RAM
//! changes made before an `Err` are not rolled back — only the deferred
//! persistence/timer/effect queue is.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use mush_core::{Ctx, ModuleOp, ModuleWrite, ObjId, Registry, World};
use smol_str::SmolStr;

use crate::data::{
    Job, JobComment, JobPayload, JobStatus, JobsState, META_KEY, NS, Note, canon_category, job_key,
    meta_value,
};

/// A registered payload applier. Invoked by `+job/approve` with the full
/// command [`Ctx`] (world, outbox, registry — the approving staffer is
/// `ctx.actor`) and the job being approved. `Err(reason)` aborts the
/// approval: the job keeps its status and the reason is shown to the
/// approver.
pub type PayloadHandlerFn = Arc<dyn Fn(&mut Ctx<'_>, &Job) -> Result<(), String> + Send + Sync>;

/// The exported handle other modules use to plug into job approval — a
/// payload-handler registry living in a module state slot on the world
/// (see the module docs for the pattern).
#[derive(Clone, Default)]
pub struct JobsApi {
    pub(crate) handlers: HashMap<SmolStr, PayloadHandlerFn>,
}

impl fmt::Debug for JobsApi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut names: Vec<&str> = self.handlers.keys().map(SmolStr::as_str).collect();
        names.sort_unstable();
        write!(f, "JobsApi(handlers: {names:?})")
    }
}

/// Register a payload handler under `name`. Call this after the engine is
/// constructed with the jobs module loaded — typically from your module's
/// `on_world_loaded` (`ctx.w` is the world). Duplicate names and a world
/// without the jobs module are errors.
pub fn register_payload_handler(
    w: &mut World,
    name: &str,
    handler: PayloadHandlerFn,
) -> Result<(), String> {
    let api = w
        .module_state_mut::<JobsApi>()
        .ok_or_else(|| "the jobs module is not loaded".to_string())?;
    let key = SmolStr::new(name);
    if api.handlers.contains_key(&key) {
        return Err(format!("payload handler {name:?} is already registered"));
    }
    api.handlers.insert(key, handler);
    Ok(())
}

/// Everything needed to open a job programmatically.
#[derive(Clone, Debug)]
pub struct OpenJob {
    /// The registered queue to file it in (matched case-insensitively).
    pub queue: SmolStr,
    /// One-line title (must be non-empty).
    pub title: String,
    /// The opener (gets Sys notices on status changes and comments).
    pub opener: ObjId,
    /// Optional opening text, recorded as the first comment.
    pub text: Option<String>,
    /// Optional structured payload for `+job/approve` to apply.
    pub payload: Option<JobPayload>,
    /// Free-form tags.
    pub tags: Vec<SmolStr>,
}

/// Open a job: allocate the next sequential id, insert it, and queue the
/// write-through persistence ops (`job:{id}` plus the id-counter `meta`
/// doc) on `ctx`. This is what `+request` uses and what producing modules
/// call from their own commands. Returns the new job id.
pub fn open_job(ctx: &mut Ctx<'_>, spec: OpenJob) -> Result<u64, String> {
    let queue = queue_spec(ctx.reg, &spec.queue)
        .ok_or_else(|| format!("no such job queue: {}", spec.queue))?
        .name
        .clone();
    let title = spec.title.trim().to_string();
    if title.is_empty() {
        return Err("a job needs a title".to_string());
    }
    let now = ctx.w.now();
    let st = ctx
        .w
        .module_state_mut::<JobsState>()
        .ok_or_else(|| "the jobs module is not loaded".to_string())?;
    let id = st.alloc_id();
    let mut comments = Vec::new();
    if let Some(text) = spec
        .text
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        comments.push(JobComment {
            who: spec.opener,
            ts: now,
            text: text.to_string(),
        });
    }
    let job = Job {
        id,
        queue,
        title,
        opener: spec.opener,
        assignee: None,
        status: JobStatus::Open,
        tags: spec.tags,
        comments,
        payload: spec.payload,
        payload_applied: None,
        opened_at: now,
        updated_at: now,
    };
    let doc = job_doc(&job);
    let next_id = st.next_id;
    st.jobs.insert(id, job);
    ctx.persist(ModuleOp {
        ns: NS.into(),
        op: ModuleWrite::JsonSet {
            key: job_key(id),
            value: doc,
        },
    });
    ctx.persist(ModuleOp {
        ns: NS.into(),
        op: ModuleWrite::JsonSet {
            key: META_KEY.into(),
            value: meta_value(next_id),
        },
    });
    Ok(id)
}

/// Read one job (any caller; enforce visibility yourself if it matters).
#[must_use]
pub fn get_job(w: &World, id: u64) -> Option<&Job> {
    w.module_state::<JobsState>()?.jobs.get(&id)
}

/// Append a comment to an existing job programmatically (RAM + write-through),
/// the way a producing module attaches structured output to a ticket — the
/// WoD `+roll /job <n>` switch pins a roll's summary to job `id`. Updates the
/// in-RAM [`JobsState`] and queues the job-doc `JsonSet` on `ctx` so the store
/// and the read cache never diverge. Enforce visibility/permission at the call
/// site; this is the low-level primitive. Returns the job's title on success.
///
/// # Errors
///
/// Errors if the jobs module is not loaded or no job has that id.
pub fn add_comment(ctx: &mut Ctx<'_>, id: u64, who: ObjId, text: &str) -> Result<String, String> {
    let now = ctx.w.now();
    let st = ctx
        .w
        .module_state_mut::<JobsState>()
        .ok_or_else(|| "the jobs module is not loaded".to_string())?;
    let job = st
        .jobs
        .get_mut(&id)
        .ok_or_else(|| format!("no such job: {id}"))?;
    job.comments.push(JobComment {
        who,
        ts: now,
        text: text.to_string(),
    });
    job.updated_at = now;
    let title = job.title.clone();
    let doc = job_doc(job);
    ctx.persist(ModuleOp {
        ns: NS.into(),
        op: ModuleWrite::JsonSet {
            key: job_key(id),
            value: doc,
        },
    });
    Ok(title)
}

/// Read one note by character and category (case-insensitive category).
#[must_use]
pub fn get_note<'w>(w: &'w World, owner: ObjId, category: &str) -> Option<&'w Note> {
    w.module_state::<JobsState>()?
        .notes
        .get(&(owner, canon_category(category)))
}

/// The registered queue spec for `name` (case-insensitive), if any.
#[must_use]
pub(crate) fn queue_spec<'r>(reg: &'r Registry, name: &str) -> Option<&'r mush_core::JobQueueSpec> {
    reg.job_queues()
        .iter()
        .find(|q| q.name.eq_ignore_ascii_case(name))
}

/// A job's persisted document form.
#[must_use]
pub(crate) fn job_doc(job: &Job) -> serde_json::Value {
    serde_json::to_value(job).expect("Job serializes")
}

/// A note's persisted document form.
#[must_use]
pub(crate) fn note_doc(note: &Note) -> serde_json::Value {
    serde_json::to_value(note).expect("Note serializes")
}

/// Close (or reopen) a job **programmatically**, on behalf of the module that opened
/// it — RAM + write-through, exactly like [`add_comment`].
///
/// The producing-module counterpart to [`open_job`]. `+job/approve` and `+job/deny`
/// are how a *person* closes a ticket; this is how the module that raised one closes
/// it when the thing it was raised about resolved itself. E5's housing is the first
/// caller: a story window opens a `wod-story` job asking an ST to decide an eviction,
/// and when the tenant's support comes back the question is simply gone — leaving the
/// job open would put an ST in front of a decision about a home that is fine.
///
/// It deliberately does **not** run payload handlers: a payload is applied by
/// `+job/approve` under a staffer's authority, and a module closing its own ticket is
/// not that. Pass `JobStatus::Resolved` for "answered", `JobStatus::Denied` for
/// "withdrawn"; `note` (when non-empty) is recorded as a closing comment attributed
/// to `by`.
///
/// # Errors
///
/// Errors if the jobs module is not loaded, no job has that id, or the job is
/// already in that status (so a retrying tick writes nothing).
pub fn set_job_status(
    ctx: &mut Ctx<'_>,
    id: u64,
    by: ObjId,
    status: JobStatus,
    note: &str,
) -> Result<(), String> {
    let now = ctx.w.now();
    let st = ctx
        .w
        .module_state_mut::<JobsState>()
        .ok_or_else(|| "the jobs module is not loaded".to_string())?;
    let job = st
        .jobs
        .get_mut(&id)
        .ok_or_else(|| format!("no such job: {id}"))?;
    if job.status == status {
        return Err(format!("job {id} is already {status}"));
    }
    let note = note.trim();
    if !note.is_empty() {
        job.comments.push(JobComment {
            who: by,
            ts: now,
            text: note.to_string(),
        });
    }
    job.status = status;
    job.updated_at = now;
    let doc = job_doc(job);
    ctx.persist(ModuleOp {
        ns: NS.into(),
        op: ModuleWrite::JsonSet {
            key: job_key(id),
            value: doc,
        },
    });
    Ok(())
}

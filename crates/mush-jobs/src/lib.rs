//! # mush-jobs — ticket queues and character notes
//!
//! Job-lite staff infrastructure (design doc 04 §9), built entirely on the
//! merged module API: sequential-numbered tickets in named queues with a
//! comment trail and an optional **structured payload** applied on
//! approval, plus per-character notes with staff visibility and approval
//! stamps.
//!
//! ## Shape
//!
//! - [`JobsModule`] is a [`GameModule`]: it registers the `+request`,
//!   `+jobs`, `+job`, `+note`, and `+notes` commands, two typed state
//!   slots, and the core `REQ` queue (player requests, workable by
//!   Royalty+).
//! - Other modules declare their own queues through the registrar
//!   (`reg.job_queue(JobQueueSpec { .. })`) — a queue's `min_role` gates
//!   who can work it and see its full listing; a job's opener and
//!   assignee always see their own.
//! - Persistence is write-through via [`ModuleOp`](mush_core::ModuleOp)s
//!   in the `jobs` namespace: `job:{id}` and `note:{char}:{category}`
//!   JSON documents plus a `meta` doc carrying the id counter, so job
//!   numbering stays sequential across reboots. Boot data arrives through
//!   [`GameModule::on_world_loaded`].
//!
//! ## Payload handlers (the approval hook)
//!
//! A job may carry a [`JobPayload`] — a registered handler name plus
//! arbitrary JSON. `+job/approve` invokes the handler **before** flipping
//! the status; a handler error (or a missing handler) leaves the job
//! untouched and is reported to the approver. Producing modules register
//! handlers via [`register_payload_handler`] and open payload-carrying
//! jobs via [`open_job`]; the exported registry handle ([`JobsApi`])
//! lives in a module state slot on the world — see [`api`] for the full
//! pattern write-up. The WoD XP-spend flow consumes this in M6.

mod api;
mod commands;
mod data;
// E4c: the economy fact provider (08c §6.1 item 1) — jobs answers `resolved` for
// payload-bearing jobs only.
mod facts;

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::module::{WorldEvent, WorldEventKind};
use mush_core::{
    Ctx, GameModule, JobQueueSpec, ModError, ModuleBootData, ModuleCtx, ModuleOp, ModuleWrite,
    Registrar, Role,
};

pub use api::{
    JobsApi, OpenJob, PayloadHandlerFn, add_comment, get_job, get_note, open_job,
    register_payload_handler, set_job_status,
};
pub use commands::NOTES_STAFF;
pub use facts::JobsFactProvider;
pub use data::{
    Approval, Job, JobComment, JobPayload, JobStatus, META_KEY, NS, Note, REQ_QUEUE, fmt_ago,
    job_key, meta_value, note_key,
};

use data::JobsState;

/// `ObjDestroyed` observer: a destroyed character's notes are removed and
/// their persisted documents deleted. Jobs they opened stay — the ticket
/// trail is the staff's historical record, not the character's data.
fn on_obj_destroyed(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::ObjDestroyed { obj } = ev else {
        return;
    };
    let owner = *obj;
    let removed: Vec<smol_str::SmolStr> = {
        let Some(st) = ctx.state_mut::<JobsState>() else {
            return;
        };
        let cats: Vec<smol_str::SmolStr> = st
            .notes
            .keys()
            .filter(|(o, _)| *o == owner)
            .map(|(_, c)| c.clone())
            .collect();
        for c in &cats {
            st.notes.remove(&(owner, c.clone()));
        }
        cats
    };
    for cat in removed {
        ctx.persist(ModuleOp {
            ns: NS.into(),
            op: ModuleWrite::Del {
                key: note_key(owner, &cat),
            },
        });
    }
}

/// The jobs module. Hand it to
/// [`Engine::with_modules`](mush_core::engine::Engine::with_modules).
#[derive(Debug, Default)]
pub struct JobsModule;

impl JobsModule {
    /// A fresh jobs module.
    #[must_use]
    pub fn new() -> JobsModule {
        JobsModule
    }
}

impl GameModule for JobsModule {
    fn name(&self) -> &'static str {
        "jobs"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(JobsState::default());
        reg.state(JobsApi::default());
        reg.subscribe(WorldEventKind::ObjDestroyed, Arc::new(on_obj_destroyed));
        reg.job_queue(JobQueueSpec {
            name: REQ_QUEUE.into(),
            title: "Player requests".into(),
            min_role: Role::Royalty,
        })?;
        commands::register(reg)
    }

    /// Boot rebuild. **Tolerant by design**: an unknown key (a rolled-back
    /// build meeting a newer build's doc) is logged and ignored, and an
    /// unparseable job/note doc is quarantined — logged and skipped, left
    /// untouched in the store — rather than refusing to boot the whole
    /// server. Nothing here is destructive, so skipping is safe; strict
    /// verification belongs in staff tooling, not the boot path. The id
    /// counter self-heals past every *loaded* job either way.
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        boot: ModuleBootData,
    ) -> Result<(), ModError> {
        let mut jobs = BTreeMap::new();
        let mut notes = BTreeMap::new();
        let mut next_id: u64 = 0;
        for (key, value) in boot.kv {
            if key.as_str() == META_KEY {
                match value.get("next_id").and_then(serde_json::Value::as_u64) {
                    Some(n) => next_id = next_id.max(n),
                    None => {
                        tracing::error!(doc = %value, "jobs: bad meta doc ignored (id counter self-heals)");
                    }
                }
            } else if key.starts_with("job:") {
                match serde_json::from_value::<Job>(value) {
                    Ok(job) => {
                        // Self-healing even if the meta doc is missing or
                        // behind: the counter clears every loaded job id.
                        next_id = next_id.max(job.id + 1);
                        jobs.insert(job.id, job);
                    }
                    Err(e) => {
                        tracing::error!(%key, error = %e, "jobs: unparseable job doc quarantined (skipped, left in store)");
                    }
                }
            } else if key.starts_with("note:") {
                match serde_json::from_value::<Note>(value) {
                    Ok(note) => {
                        notes.insert((note.owner, note.category.clone()), note);
                    }
                    Err(e) => {
                        tracing::error!(%key, error = %e, "jobs: unparseable note doc quarantined (skipped, left in store)");
                    }
                }
            } else {
                tracing::warn!(%key, "jobs: unknown persisted key ignored (newer build's data?)");
            }
        }
        let st = ctx
            .state_mut::<JobsState>()
            .ok_or_else(|| ModError::msg("jobs: state slot missing"))?;
        st.jobs = jobs;
        st.notes = notes;
        st.next_id = next_id;
        Ok(())
    }
}

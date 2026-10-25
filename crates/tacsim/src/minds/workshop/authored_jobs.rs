//! Marked author jobs retain evidence, never a cached grant. Only an actual
//! connection completion can take the worker result and freshly authorize use.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};

use mush_core::{
    ArgShape, CmdError, CommandSpec, ConnectionCompletion, ConnectionCompletionHandle, Ctx,
    ModError, ObjId, Registrar,
};
use mush_proto::{
    InvokeResult, MindDiagnostic, MindEditBase, MindPictureReport, MindResponse, OutputEvent,
    RichText,
};
use mush_roll::commerce::authored_access::AuthoredDraftAccess;
use mush_roll::{EditBase, RollError};

use super::commands::{WorkshopCommands, roll, sources};
use super::executor::{JobId, JobKind, MAX_JOBS, Principal};
use super::invalid;

/// Only service custody is stored here, never eligibility or source authority.
#[derive(Clone)]
pub(super) struct WorkshopService(pub(super) Weak<WorkshopCommands>);
#[derive(Clone)]
struct CompletionHandle(ConnectionCompletionHandle);
#[derive(Clone, Copy)]
pub(super) enum Delivery {
    Line,
    Remote { request: u64 },
}
struct Entry {
    principal: Principal,
    admission: AuthoredDraftAccess,
    ticket: ConnectionCompletion,
    delivery: Delivery,
    lease: u64,
}
#[derive(Default)]
pub(super) struct Jobs(Mutex<BTreeMap<JobId, Arc<Entry>>>);

/// Called only by the explicit authored-access composition. Default workshop
/// registration has no completion command and cannot submit marked jobs.
pub(super) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    reg.command(
        CommandSpec {
            name: "@mind-author-complete".into(),
            shape: ArgShape::Raw,
            ..CommandSpec::default()
        },
        Arc::new(|ctx: &mut Ctx<'_>| {
            let service = ctx
                .w
                .module_state::<WorkshopService>()
                .and_then(|s| s.0.upgrade())
                .ok_or_else(|| CmdError::msg("Authoring completion unavailable."))?;
            service.complete_authored(ctx);
            Ok(())
        }),
    )?;
    let handle = reg.connection_completion("@mind-author-complete", &[])?;
    reg.state(CompletionHandle(handle));
    Ok(())
}

impl WorkshopCommands {
    pub(super) fn submit_authored(
        &self,
        ctx: &Ctx<'_>,
        principal: Principal,
        name: &str,
        base: EditBase,
        kind: JobKind,
        delivery: Delivery,
    ) -> Result<JobId, RollError> {
        let mind = roll(ctx)?.mind(principal.actor(), name)?;
        if mind.require_original_access().is_ok() {
            return Err(RollError::Missing);
        }
        let admission = super::authored_access::current_job(ctx, mind.id(), kind.authored_kind())?;
        if admission.actor() != principal.actor() || admission.mind() != mind || mind.base() != base
        {
            return Err(RollError::StaleEdit);
        }
        let handle = ctx
            .w
            .module_state::<CompletionHandle>()
            .ok_or(RollError::Missing)?;
        let mut jobs = self.authored.0.lock().map_err(|_| RollError::Missing)?;
        if jobs.len() >= MAX_JOBS {
            return Err(invalid("workshop", "authoring queue is full"));
        }
        let id = self
            .executor
            .submit_authored(principal, &admission, base, kind)?;
        let ticket = match ctx.capture_connection_completion(&handle.0, &id.number().to_string()) {
            Ok(ticket) => ticket,
            Err(_) => {
                let _ = self.executor.cancel(principal, id);
                return Err(RollError::Missing);
            }
        };
        let Some((actor, lease)) = ctx.w.connection_binding(ticket.connection()) else {
            let _ = self.executor.cancel(principal, id);
            return Err(RollError::Missing);
        };
        if actor != principal.actor() {
            let _ = self.executor.cancel(principal, id);
            return Err(RollError::Missing);
        }
        jobs.insert(
            id,
            Arc::new(Entry {
                principal,
                admission,
                ticket,
                delivery,
                lease,
            }),
        );
        Ok(id)
    }

    pub(super) fn authored_queued(&self, ctx: &mut Ctx<'_>, id: JobId) {
        if let Some(row) = self
            .authored
            .0
            .lock()
            .ok()
            .and_then(|jobs| jobs.get(&id).cloned())
            && row.ticket.is_live(ctx.w)
        {
            ctx.out.to_conn(
                row.ticket.connection(),
                OutputEvent::Sys(RichText::plain(format!(
                    "Authoring job {} queued.",
                    id.number()
                ))),
            );
        }
    }
    pub(super) fn is_authored(&self, id: JobId) -> bool {
        self.authored
            .0
            .lock()
            .map_or(true, |jobs| jobs.contains_key(&id))
    }
    pub(super) fn cancel_authored_character(&self, actor: ObjId) {
        if let Ok(mut jobs) = self.authored.0.lock() {
            jobs.retain(|_, row| row.principal.actor() != actor);
        }
    }
    fn remove_authored(&self, id: JobId) {
        if let Ok(mut jobs) = self.authored.0.lock() {
            jobs.remove(&id);
        }
    }

    /// Readiness only: the result stays in the executor's counted account/global
    /// slot until actual handler entry. Scheduling does not consume its receipt.
    pub(super) fn poll_authored(&self, ctx: &mut Ctx<'_>) {
        let jobs = match self.authored.0.lock() {
            Ok(jobs) => jobs
                .iter()
                .map(|(id, row)| (*id, Arc::clone(row)))
                .collect::<Vec<_>>(),
            Err(_) => return,
        };
        let ready = self.executor.ready();
        for (id, row) in jobs {
            if !row.ticket.is_live(ctx.w) || row.ticket.has_entered() {
                let _ = self.executor.cancel(row.principal, id);
                self.remove_authored(id);
                // Generic terminal UX needs no source/parent read, but still
                // cannot go to a replacement physical connection or actor.
                if ctx.w.connection_binding(row.ticket.connection())
                    == Some((row.principal.actor(), row.lease))
                {
                    deliver(ctx, &row, unavailable());
                }
                continue;
            }
            if ready
                .iter()
                .any(|(actor, job)| *actor == row.principal.actor() && *job == id)
            {
                let _ = ctx.schedule_connection_completion(&row.ticket);
            }
        }
    }

    fn fresh_authored(&self, ctx: &Ctx<'_>, row: &Entry) -> Result<AuthoredDraftAccess, RollError> {
        if !row.ticket.is_current(ctx) || self.principal(ctx)? != row.principal {
            return Err(RollError::Missing);
        }
        let kind = row.admission.job_kind().ok_or(RollError::Missing)?;
        let current = super::authored_access::current_job(ctx, row.admission.mind().id(), kind)?;
        if !row.admission.same_job_subject(&current) || !row.ticket.is_current(ctx) {
            return Err(RollError::Missing);
        }
        Ok(current)
    }

    fn complete_authored(&self, ctx: &mut Ctx<'_>) {
        let argument = ctx.args.raw();
        let Some((id, row)) = self.authored.0.lock().ok().and_then(|jobs| {
            jobs.iter()
                .find(|(id, _)| id.number().to_string() == argument)
                .map(|(id, row)| (*id, Arc::clone(row)))
        }) else {
            return;
        };
        // A manual command must not cancel, consume or otherwise affect a job.
        if !row.ticket.is_current(ctx) {
            return;
        }
        self.remove_authored(id);
        let result = (|| {
            let current = self.fresh_authored(ctx, &row)?;
            let completion = self
                .executor
                .take(row.principal, id)?
                .ok_or(RollError::Missing)?;
            completion.check_authored(row.principal, &current)?;
            let response = if let Some(diagnostic) = completion.diagnostic {
                MindResponse::Checked {
                    job: id.number(),
                    reports: Vec::new(),
                    diagnostics: vec![MindDiagnostic {
                        field: diagnostic.field,
                        message: diagnostic.message,
                        line: diagnostic.line,
                        column: diagnostic.column,
                    }],
                }
            } else {
                match completion.kind {
                    JobKind::Check => MindResponse::Checked {
                        job: id.number(),
                        reports: completion
                            .reports
                            .into_iter()
                            .map(|r| MindPictureReport {
                                picture: super::structured::picture_name(r.picture).into(),
                                armed: r.armed,
                                operations: r.operations,
                                fees: r.fees,
                                intents: r.intents,
                                fault: r.fault,
                                station_refusals: r.station_refusals,
                                class_violations: r.class_violations,
                            })
                            .collect(),
                        diagnostics: Vec::new(),
                    },
                    JobKind::Save { notes, version } => {
                        let receipt = completion
                            .validated
                            .ok_or_else(|| invalid("save", "validation receipt missing"))?;
                        let prepared = receipt.prepare_authored_save(
                            row.principal,
                            &current,
                            roll(ctx)?,
                            sources(ctx)?,
                            &notes,
                            version.as_deref(),
                        )?;
                        let reference = prepared.revision();
                        let response = MindResponse::Saved {
                            job: id.number(),
                            revision: reference.key.number,
                            digest: reference.digest.to_hex(),
                            base: MindEditBase {
                                edit: row
                                    .admission
                                    .mind()
                                    .base()
                                    .edit
                                    .checked_add(1)
                                    .ok_or(RollError::Exhausted("edit"))?,
                                revision: Some(reference.key.number),
                            },
                        };
                        self.fresh_authored(ctx, &row)?;
                        // Exclusive World, no await or callback from fresh read
                        // to both-owner preflight/install; last fallible action.
                        crate::store::publish_mind_revision(ctx, prepared)?;
                        return Ok(response);
                    }
                }
            };
            self.fresh_authored(ctx, &row)?;
            Ok::<_, RollError>(response)
        })();
        if result.is_err() {
            let _ = self.executor.cancel(row.principal, id);
        }
        // Refusals deliberately contain no worker diagnostic, name or source.
        let response = result.unwrap_or_else(|_| unavailable());
        reply(ctx, &row, response);
    }

    /// Source-free status/cancel stays bound to the original physical request.
    /// Returning None means this is not a marked job; it is not an ACL fallback.
    pub(super) fn authored_status(
        &self,
        ctx: &mut Ctx<'_>,
        principal: Principal,
        conn: u64,
        request: u64,
        number: u64,
        cancel: bool,
    ) -> Option<Result<MindResponse, RollError>> {
        let found = match self.authored.0.lock() {
            Ok(jobs) => jobs
                .iter()
                .find(|(id, _)| id.number() == number)
                .map(|(id, row)| (*id, Arc::clone(row))),
            Err(_) => return Some(Err(RollError::Missing)),
        };
        let (id, row) = found?;
        Some((|| {
            let Delivery::Remote { request: original } = row.delivery else {
                return Err(RollError::Missing);
            };
            if row.principal != principal
                || row.ticket.connection() != conn
                || !row.ticket.is_live(ctx.w)
            {
                return Err(RollError::Missing);
            }
            if cancel {
                self.executor.cancel(principal, id)?;
                self.remove_authored(id);
                if original != request {
                    ctx.out.to_conn(
                        conn,
                        OutputEvent::InvokeResult {
                            id: original,
                            result: InvokeResult::Mind(MindResponse::Cancelled { job: number }),
                        },
                    );
                }
                Ok(MindResponse::Cancelled { job: number })
            } else {
                Ok(MindResponse::Pending { job: number })
            }
        })())
    }
}

fn reply(ctx: &mut Ctx<'_>, row: &Entry, response: MindResponse) {
    if !row.ticket.is_current(ctx) {
        return;
    }
    deliver(ctx, row, response);
}
fn unavailable() -> MindResponse {
    MindResponse::Error {
        code: "unavailable".into(),
        message: "Authoring completion is no longer available.".into(),
    }
}
fn deliver(ctx: &mut Ctx<'_>, row: &Entry, response: MindResponse) {
    let conn = row.ticket.connection();
    match row.delivery {
        Delivery::Remote { request } => ctx.out.to_conn(
            conn,
            OutputEvent::InvokeResult {
                id: request,
                result: InvokeResult::Mind(response).bounded(),
            },
        ),
        Delivery::Line => {
            let lines = match response {
                MindResponse::Checked {
                    reports,
                    diagnostics,
                    ..
                } => {
                    let mut lines = Vec::new();
                    for d in diagnostics {
                        lines.push(format!("{}: {}", d.field, d.message));
                    }
                    for r in reports {
                        lines.push(format!(
                            "{} {}: ops {}; fees {}; {} intents; fault {}.",
                            r.picture,
                            if r.armed { "armed" } else { "unarmed" },
                            r.operations,
                            r.fees,
                            r.intents.len(),
                            r.fault.as_deref().unwrap_or("none")
                        ));
                        lines.extend(r.intents);
                        lines.extend(r.station_refusals);
                        lines.extend(r.class_violations);
                    }
                    lines
                }
                MindResponse::Saved {
                    revision, digest, ..
                } => vec![format!("Saved revision {revision} ({digest})")],
                MindResponse::Error { message, .. } => vec![message],
                _ => Vec::new(),
            };
            for line in lines {
                ctx.out
                    .to_conn(conn, OutputEvent::Sys(RichText::plain(line)));
            }
        }
    }
}

#[cfg(test)]
pub(in crate::minds) mod probe {
    use super::*;
    pub(in crate::minds) fn ready(world: &mush_core::World, number: u64) -> bool {
        world
            .module_state::<WorkshopService>()
            .and_then(|s| s.0.upgrade())
            .is_some_and(|service| {
                service
                    .executor
                    .ready()
                    .iter()
                    .any(|(_, id)| id.number() == number)
            })
    }
    pub(in crate::minds) fn retained(world: &mush_core::World, number: u64) -> bool {
        world
            .module_state::<WorkshopService>()
            .and_then(|s| s.0.upgrade())
            .is_some_and(|service| {
                service
                    .authored
                    .0
                    .lock()
                    .is_ok_and(|rows| rows.keys().any(|id| id.number() == number))
            })
    }
}

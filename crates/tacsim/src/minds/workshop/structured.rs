//! Typed transport adapter for the existing character-owned workshop.
//! Connection/lease binding is additional to the worker's account/session and
//! exact draft receipt. No operation accepts an actor, executable validator or
//! command text from the client.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use mush_core::module::InvokeHandler;
use mush_core::output::ConnId;
use mush_core::{Ctx, ObjId};
use mush_proto::{
    InvokeRequest, InvokeResult, MindDiagnostic, MindDraft, MindEdit, MindEditBase,
    MindPictureReport, MindRequest, MindResponse, OutputEvent,
};
use mush_roll::{EditBase, Envelope, RollError, WorkingCopy};

use super::commands::{WorkshopCommands, publish, roll, sources};
use super::executor::{JobId, JobKind, MAX_JOBS, Picture, Principal};
use super::{CardEdit, MindRef, invalid};

const REMOTE_TTL: Duration = Duration::from_secs(120);
#[derive(Default)]
pub(super) struct RemoteJobs(BTreeMap<JobId, RemoteJob>);
#[derive(Clone)]
struct RemoteJob {
    principal: Principal,
    conn: ConnId,
    lease: u64,
    request: u64,
    base: EditBase,
    started: Instant,
}

impl InvokeHandler for WorkshopCommands {
    fn invoke(
        &self,
        ctx: &mut Ctx<'_>,
        conn: ConnId,
        id: u64,
        request: &InvokeRequest,
    ) -> InvokeResult {
        // Direct adapter calls must satisfy the same authority as engine calls.
        let response = (|| {
            request
                .validate()
                .map_err(|_| invalid("request", "Invalid or oversized structured request."))?;
            let (_, lease) = ctx
                .w
                .connection_binding(conn)
                .filter(|(actor, _)| *actor == ctx.actor)
                .ok_or(RollError::Missing)?;
            let InvokeRequest::Mind(request) = request;
            if let MindRequest::Trace {
                unit,
                count,
                follow,
            } = request
            {
                let unit = u32::try_from(*unit)
                    .map(ObjId)
                    .map_err(|_| RollError::Missing)?;
                return crate::minds::trace_feed::select(
                    ctx,
                    unit,
                    count.map(usize::from),
                    *follow,
                )
                .map(|data| MindResponse::Trace { data })
                .map_err(|message| invalid("trace", message));
            }
            let principal = self.principal(ctx)?;
            self.structured(ctx, principal, conn, lease, id, request)
        })();
        InvokeResult::Mind(response.unwrap_or_else(refusal)).bounded()
    }
}

impl WorkshopCommands {
    #[allow(clippy::too_many_arguments)]
    fn structured(
        &self,
        ctx: &mut Ctx<'_>,
        principal: Principal,
        conn: ConnId,
        lease: u64,
        request_id: u64,
        request: &MindRequest,
    ) -> Result<MindResponse, RollError> {
        match request {
            MindRequest::Lifecycle { name, base, edit } => {
                let base = from_base(*base);
                let prepared = super::lifecycle(roll(ctx)?, ctx.actor, name, base, edit)?;
                let response = match edit {
                    mush_proto::MindLifecycle::Remove => MindResponse::Removed {
                        name: mush_roll::canonical_name(name)?,
                    },
                    mush_proto::MindLifecycle::Rename { new_name } => edited(new_name, base)?,
                    _ => edited(name, base)?,
                };
                publish(ctx, prepared)?;
                Ok(response)
            }
            MindRequest::Prune { limit } => {
                let (revisions, source_candidates_retained) = super::commands::prune(ctx, *limit)?;
                Ok(MindResponse::Pruned {
                    revisions,
                    source_candidates_retained,
                })
            }
            MindRequest::Trace { .. } => Err(RollError::Missing),
            MindRequest::List {} => Ok(MindResponse::Library {
                names: roll(ctx)?
                    .names(ctx.actor)
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            }),
            MindRequest::Read { name, revision } => {
                // Validate the separated selector through the exact shared owner
                // reference parser; no command is constructed or executed.
                let reference = MindRef::parse(&match revision {
                    Some(revision) => format!("{name}@{revision}"),
                    None => name.clone(),
                })?;
                let draft = super::owned_copy(roll(ctx)?, sources(ctx)?, ctx.actor, &reference)?;
                let base = roll(ctx)?.mind(ctx.actor, name)?.base();
                let mut rules_text = BTreeMap::new();
                let mut rules_source = BTreeMap::new();
                let mut diagnostics = Vec::new();
                match super::decoded(&draft.envelope) {
                    Ok(card) => {
                        for (profile, rules) in &card.rules {
                            rules_text.insert(profile.to_string(),
                                rules.iter().map(crate::arena::schema::rule_words).collect());
                            let sources = rules.iter().enumerate().map(|(index, rule)| {
                                let source = crate::arena::schema::rule_source(rule).ok()
                                    .filter(|source| super::author_rules::parse(source, self.floor_permille)
                                        .is_ok_and(|parsed| parsed == *rule));
                                if source.is_none() {
                                    diagnostics.push(MindDiagnostic {
                                        field: format!("rules.{profile}[{index}]"),
                                        message: "This rule has no exact editable spelling under the current grammar and floor; its structured value is unchanged.".into(),
                                        line: None, column: None,
                                    });
                                }
                                source
                            }).collect();
                            rules_source.insert(profile.to_string(), sources);
                        }
                    }
                    Err(_) => diagnostics.push(MindDiagnostic {
                        field: "envelope".into(),
                        message: "The working card or rules are invalid; source and metadata are returned unchanged.".into(),
                        line: None, column: None,
                    }),
                }
                let mut response = MindResponse::Read {
                    name: name.clone(),
                    revision: revision.clone(),
                    base: to_base(base),
                    draft: Box::new(to_draft(draft)),
                    rules_text,
                    rules_source,
                    diagnostics,
                };
                // Derived text is optional read assistance. Never let it make a
                // legal exact draft impossible to pull through the transport.
                let encoded = serde_json::to_vec(&InvokeResult::Mind(response.clone()))
                    .map_err(|_| invalid("response", "Cannot encode owned draft."))?;
                if encoded.len() > mush_proto::MAX_INVOKE_BYTES
                    && let MindResponse::Read {
                        rules_text,
                        rules_source,
                        diagnostics,
                        ..
                    } = &mut response
                {
                    rules_text.clear();
                    rules_source.clear();
                    diagnostics.clear();
                    diagnostics.push(MindDiagnostic {
                            field: "rules".into(),
                            message: "Derived rule text is omitted to keep this exact draft within the transport limit.".into(),
                            line: None, column: None,
                        });
                }
                Ok(response)
            }
            MindRequest::Create { name, from } => {
                let prepared =
                    super::create(roll(ctx)?, sources(ctx)?, ctx.actor, name, from.as_deref())?;
                let result = MindResponse::Edited {
                    name: mush_roll::canonical_name(name)?,
                    base: MindEditBase {
                        edit: 0,
                        revision: None,
                    },
                };
                publish(ctx, prepared)?;
                Ok(result)
            }
            MindRequest::Replace { name, base, draft } => {
                // Working replacement shares the owner CAS/bounds; compilation
                // remains exclusively the bounded worker's check/save service.
                let base = from_base(*base);
                let prepared = roll(ctx)?.prepare_edit(
                    ctx.actor,
                    name,
                    base,
                    from_draft((**draft).clone()),
                )?;
                let result = edited(name, base)?;
                publish(ctx, prepared)?;
                Ok(result)
            }
            MindRequest::Edit { name, base, edit } => {
                let base = from_base(*base);
                let prepared = super::edit_card(
                    roll(ctx)?,
                    ctx.actor,
                    name,
                    base,
                    card_edit(edit.clone()),
                    self.floor_permille,
                )?;
                let result = edited(name, base)?;
                publish(ctx, prepared)?;
                Ok(result)
            }
            MindRequest::Check { name, base } => self.submit_remote(
                ctx,
                principal,
                conn,
                lease,
                request_id,
                name,
                from_base(*base),
                JobKind::Check,
            ),
            MindRequest::Save {
                name,
                base,
                notes,
                version,
            } => self.submit_remote(
                ctx,
                principal,
                conn,
                lease,
                request_id,
                name,
                from_base(*base),
                JobKind::Save {
                    notes: notes.clone(),
                    version: version.clone(),
                },
            ),
            MindRequest::Status { job } | MindRequest::Cancel { job } => {
                let mut jobs = self
                    .remote
                    .lock()
                    .map_err(|_| invalid("workshop", "executor unavailable"))?;
                let (id, binding) = jobs
                    .0
                    .iter()
                    .find(|(id, binding)| {
                        id.number() == *job
                            && binding.conn == conn
                            && binding.lease == lease
                            && binding.principal == principal
                    })
                    .map(|(id, binding)| (*id, binding.clone()))
                    .ok_or(RollError::Missing)?;
                if binding.started.elapsed() > REMOTE_TTL {
                    let _ = self.executor.cancel(principal, id);
                    jobs.0.remove(&id);
                    return Err(RollError::Missing);
                }
                if matches!(request, MindRequest::Cancel { .. }) {
                    self.executor.cancel(principal, id)?;
                    jobs.0.remove(&id);
                    // Resolve the original outstanding request as well as this
                    // cancellation's id, so neither client promise is orphaned.
                    if binding.request != request_id {
                        ctx.out.to_conn(
                            conn,
                            OutputEvent::InvokeResult {
                                id: binding.request,
                                result: InvokeResult::Mind(MindResponse::Cancelled { job: *job }),
                            },
                        );
                    }
                    Ok(MindResponse::Cancelled { job: *job })
                } else {
                    Ok(MindResponse::Pending { job: *job })
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn submit_remote(
        &self,
        ctx: &Ctx<'_>,
        principal: Principal,
        conn: ConnId,
        lease: u64,
        request: u64,
        name: &str,
        base: EditBase,
        kind: JobKind,
    ) -> Result<MindResponse, RollError> {
        let mut jobs = self
            .remote
            .lock()
            .map_err(|_| invalid("workshop", "executor unavailable"))?;
        if jobs.0.len() >= MAX_JOBS
            || jobs
                .0
                .values()
                .any(|job| job.conn == conn && job.lease == lease && job.request == request)
        {
            return Err(invalid(
                "workshop",
                "duplicate request or authoring queue full",
            ));
        }
        let id = self
            .executor
            .submit(principal, roll(ctx)?, name, base, kind)?;
        jobs.0.insert(
            id,
            RemoteJob {
                principal,
                conn,
                lease,
                request,
                base,
                started: Instant::now(),
            },
        );
        Ok(MindResponse::Queued { job: id.number() })
    }

    /// Poll before line jobs, so a remote completion never becomes an actor-wide
    /// diagnostic or lands on another physical connection of the same character.
    pub(super) fn poll_remote(&self, ctx: &mut Ctx<'_>) {
        let bindings = match self.remote.lock() {
            Ok(jobs) => jobs
                .0
                .iter()
                .map(|(id, job)| (*id, job.clone()))
                .collect::<Vec<_>>(),
            Err(_) => return,
        };
        let caller = ctx.actor;
        for (id, binding) in bindings {
            ctx.actor = binding.principal.actor();
            let live = ctx.w.connection_binding(binding.conn) == Some((ctx.actor, binding.lease))
                && self
                    .principal(ctx)
                    .is_ok_and(|principal| principal == binding.principal);
            if !live || binding.started.elapsed() > REMOTE_TTL {
                let _ = self.executor.cancel(binding.principal, id);
                self.remove_remote(id);
                if live {
                    reply(
                        ctx,
                        &binding,
                        MindResponse::Error {
                            code: "expired".into(),
                            message: "Authoring result expired.".into(),
                        },
                    );
                }
                continue;
            }
            let completion = match self.executor.take(binding.principal, id) {
                Ok(None) => continue,
                Ok(Some(completion)) => completion,
                Err(error) => {
                    self.remove_remote(id);
                    reply(ctx, &binding, refusal(error));
                    continue;
                }
            };
            self.remove_remote(id);
            let result = (|| {
                completion.check_current(binding.principal, roll(ctx)?)?;
                if let Some(diagnostic) = completion.diagnostic {
                    return Ok(MindResponse::Checked {
                        job: id.number(),
                        reports: Vec::new(),
                        diagnostics: vec![MindDiagnostic {
                            field: diagnostic.field,
                            message: diagnostic.message,
                            line: diagnostic.line,
                            column: diagnostic.column,
                        }],
                    });
                }
                match completion.kind {
                    JobKind::Check => Ok(MindResponse::Checked {
                        job: id.number(),
                        reports: completion
                            .reports
                            .into_iter()
                            .map(|r| MindPictureReport {
                                picture: match r.picture {
                                    Picture::Empty => "empty",
                                    Picture::Hostile450 => "hostile_450",
                                    Picture::HurtTwo => "hurt_two",
                                }
                                .into(),
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
                    }),
                    JobKind::Save { notes, version } => {
                        let receipt = completion
                            .validated
                            .ok_or_else(|| invalid("save", "validation receipt missing"))?;
                        let prepared = receipt.prepare_save(
                            binding.principal,
                            roll(ctx)?,
                            sources(ctx)?,
                            &notes,
                            version.as_deref(),
                        )?;
                        let reference = prepared.revision();
                        let base = MindEditBase {
                            edit: binding
                                .base
                                .edit
                                .checked_add(1)
                                .ok_or(RollError::Exhausted("edit"))?,
                            revision: Some(reference.key.number),
                        };
                        let result = MindResponse::Saved {
                            job: id.number(),
                            revision: reference.key.number,
                            digest: reference.digest.to_hex(),
                            base,
                        };
                        // Final fallible action. The receipt binds all source,
                        // envelope, account/session and draft identity; source
                        // and revision enter one deferred persistence batch.
                        crate::store::publish_mind_revision(ctx, prepared)?;
                        crate::match_::tutorial::hint(
                            ctx,
                            binding.principal.actor(),
                            crate::match_::tutorial::HintPoint::MindSaved,
                        );
                        Ok(result)
                    }
                }
            })();
            reply(ctx, &binding, result.unwrap_or_else(refusal));
        }
        ctx.actor = caller;
    }

    pub(super) fn is_remote(&self, id: JobId) -> bool {
        // A poisoned routing map must never turn private remote diagnostics into
        // actor-wide line output.
        self.remote
            .lock()
            .map_or(true, |jobs| jobs.0.contains_key(&id))
    }

    fn remove_remote(&self, id: JobId) {
        if let Ok(mut jobs) = self.remote.lock() {
            jobs.0.remove(&id);
        }
    }
}

fn reply(ctx: &mut Ctx<'_>, binding: &RemoteJob, result: MindResponse) {
    // Rechecked even though the synchronous world pulse cannot interleave.
    if ctx.w.connection_binding(binding.conn) == Some((binding.principal.actor(), binding.lease)) {
        ctx.out.to_conn(
            binding.conn,
            OutputEvent::InvokeResult {
                id: binding.request,
                result: InvokeResult::Mind(result).bounded(),
            },
        );
    }
}
fn refusal(error: RollError) -> MindResponse {
    let (code, message) = match error {
        RollError::Missing => (
            "missing",
            "No owned mind or authoring job is available.".into(),
        ),
        RollError::StaleEdit => (
            "stale",
            "The working copy changed; reload before applying this edit.".into(),
        ),
        RollError::Invalid { field, message } => ("invalid", format!("{field}: {message}")),
        _ => ("refused", "The authoring request was refused.".into()),
    };
    MindResponse::Error {
        code: code.into(),
        message,
    }
}
fn to_base(base: EditBase) -> MindEditBase {
    MindEditBase {
        edit: base.edit,
        revision: base.revision,
    }
}
fn from_base(base: MindEditBase) -> EditBase {
    EditBase {
        edit: base.edit,
        revision: base.revision,
    }
}
fn edited(name: &str, base: EditBase) -> Result<MindResponse, RollError> {
    Ok(MindResponse::Edited {
        name: name.into(),
        base: MindEditBase {
            edit: base
                .edit
                .checked_add(1)
                .ok_or(RollError::Exhausted("edit"))?,
            revision: base.revision,
        },
    })
}
fn to_draft(copy: WorkingCopy) -> MindDraft {
    let Envelope {
        schema,
        presets,
        profiles,
        rules,
        declared_class,
        mind_abi,
    } = copy.envelope;
    MindDraft {
        source: copy.source,
        schema,
        presets,
        profiles,
        rules,
        declared_class,
        mind_abi,
    }
}
fn from_draft(draft: MindDraft) -> WorkingCopy {
    WorkingCopy {
        source: draft.source,
        envelope: Envelope {
            schema: draft.schema,
            presets: draft.presets,
            profiles: draft.profiles,
            rules: draft.rules,
            declared_class: draft.declared_class,
            mind_abi: draft.mind_abi,
        },
    }
}
fn card_edit(edit: MindEdit) -> CardEdit {
    match edit {
        MindEdit::Set {
            profile,
            field,
            value,
        } => CardEdit::Set {
            profile,
            field,
            value,
        },
        MindEdit::PresetSave { profile, preset } => CardEdit::PresetSave { profile, preset },
        MindEdit::PresetApply { profile, preset } => CardEdit::PresetApply { profile, preset },
        MindEdit::PresetDelete { preset } => CardEdit::PresetDelete { preset },
        MindEdit::ProfileAdd { name, from } => CardEdit::ProfileAdd { name, from },
        MindEdit::ProfileDelete { name } => CardEdit::ProfileDelete { name },
        MindEdit::ProfileRename { old, new } => CardEdit::ProfileRename { old, new },
        MindEdit::Rule {
            profile,
            slot,
            text,
        } => CardEdit::Rule {
            profile,
            slot: usize::from(slot),
            text,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::super::executor::{CheckPictures, PICTURES, PictureReport, WorkshopExecutor};
    use super::*;
    use mush_core::{
        AccountId, AccountIdentity, Audience, Effects, Engine, GOD, Inbound, Outbox, ParsedArgs,
        Registry, SwitchSet, World,
    };
    use std::sync::Arc;

    struct Accounts(AccountId);
    impl AccountIdentity for Accounts {
        fn account_of(&self, _actor: ObjId) -> Option<AccountId> {
            Some(self.0)
        }
    }
    struct Pictures;
    impl CheckPictures for Pictures {
        fn run(&self, _: &WorkingCopy) -> Result<Vec<PictureReport>, RollError> {
            Ok(PICTURES
                .into_iter()
                .map(|(picture, armed)| PictureReport {
                    picture,
                    armed,
                    operations: 1,
                    fees: 1,
                    intents: Vec::new(),
                    fault: None,
                    station_refusals: Vec::new(),
                    class_violations: Vec::new(),
                })
                .collect())
        }
    }
    fn rig() -> (Engine, Arc<WorkshopCommands>, ObjId) {
        let mut world = World::new_minimal();
        let other = world
            .create(
                mush_core::ObjKind::Player,
                "Other",
                GOD,
                Some(mush_core::LIMBO),
            )
            .expect("other");
        world.module_state_insert(mush_roll::RollStore::default());
        world.module_state_insert(crate::store::TacsimStore::default());
        world.module_state_insert(mush_sim::AccountOracle::new(Arc::new(Accounts(
            AccountId::mint("author", 1).expect("account"),
        ))));
        let mut registry = Registry::new();
        let service = Arc::new(WorkshopCommands::new(
            WorkshopExecutor::new(Arc::new(Pictures)).expect("workers"),
            crate::config::ArenaConfig::default(),
            0,
        ));
        let mut registrar = mush_core::module::Registrar::new(&mut registry, crate::store::NS);
        registrar
            .persist_delegation(crate::store::NS, mush_roll::NS, "test owner publication")
            .expect("delegation");
        service.register(&mut registrar).expect("register");
        let mut engine = Engine::new(world, registry);
        attach(&mut engine, 7, GOD);
        attach(&mut engine, 8, GOD);
        attach(&mut engine, 9, other);
        (engine, service, other)
    }
    fn attach(engine: &mut Engine, conn: u64, actor: ObjId) {
        engine.step(
            Inbound::ConnUp {
                conn,
                actor,
                caps_structured: true,
                from_linkdead: false,
            },
            &mut Outbox::new(),
            &mut Effects::new(),
        );
    }
    fn invoke(
        engine: &mut Engine,
        conn: u64,
        actor: ObjId,
        id: u64,
        request: MindRequest,
    ) -> MindResponse {
        let mut out = Outbox::new();
        engine.step(
            Inbound::Invoke {
                conn,
                actor,
                id,
                request: InvokeRequest::Mind(request),
            },
            &mut out,
            &mut Effects::new(),
        );
        let responses = out
            .events()
            .iter()
            .filter_map(|(audience, event)| match event {
                OutputEvent::InvokeResult {
                    id: got,
                    result: InvokeResult::Mind(response),
                } if *got == id => {
                    assert_eq!(*audience, Audience::Conn(conn));
                    Some(response.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(responses.len(), 1);
        responses.into_iter().next().expect("response")
    }
    fn poll(engine: &mut Engine, service: &WorkshopCommands) -> Outbox {
        let mut out = Outbox::new();
        let mut ctx = Ctx {
            w: &mut engine.world,
            actor: GOD,
            switches: SwitchSet::default(),
            args: ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(crate::store::NS.into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        service.poll_remote(&mut ctx);
        out
    }
    fn create(engine: &mut Engine) -> MindEditBase {
        let response = invoke(
            engine,
            7,
            GOD,
            1,
            MindRequest::Create {
                name: "mine".into(),
                from: None,
            },
        );
        let MindResponse::Edited { base, .. } = response else {
            panic!("create");
        };
        base
    }
    fn await_ready(service: &WorkshopCommands) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while service.executor.ready().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            !service.executor.ready().is_empty(),
            "bounded worker completed"
        );
    }

    #[test]
    fn exact_draft_cas_and_source_ownership_are_shared_across_connections() {
        let (mut engine, _, other) = rig();
        let base = create(&mut engine);
        let MindResponse::Read { draft, .. } = invoke(
            &mut engine,
            7,
            GOD,
            2,
            MindRequest::Read {
                name: "mine".into(),
                revision: None,
            },
        ) else {
            panic!("read");
        };
        let mut changed = draft.clone();
        changed.source.push_str("// own edit\n");
        assert!(matches!(
            invoke(
                &mut engine,
                8,
                GOD,
                3,
                MindRequest::Replace {
                    name: "mine".into(),
                    base,
                    draft: changed.clone()
                }
            ),
            MindResponse::Edited { .. }
        ));
        assert!(
            matches!(invoke(&mut engine, 7, GOD, 4, MindRequest::Replace {
            name: "mine".into(), base, draft }), MindResponse::Error { code, .. } if code == "stale")
        );
        assert!(
            matches!(invoke(&mut engine, 9, other, 5, MindRequest::Read {
            name: "mine".into(), revision: None }), MindResponse::Error { code, .. } if code == "missing")
        );
        let MindResponse::Read { draft, .. } = invoke(
            &mut engine,
            7,
            GOD,
            6,
            MindRequest::Read {
                name: "mine".into(),
                revision: None,
            },
        ) else {
            panic!("read");
        };
        assert_eq!(draft, changed);
    }

    #[test]
    fn m6_lifecycle_line_and_typed_operations_share_owner_cas_and_immutable_history() {
        use mush_proto::MindLifecycle;
        let (mut engine, service, other) = rig();
        let base = create(&mut engine);
        assert!(matches!(
            invoke(
                &mut engine,
                7,
                GOD,
                101,
                MindRequest::Save {
                    name: "mine".into(),
                    base,
                    notes: String::new(),
                    version: Some("first".into()),
                }
            ),
            MindResponse::Queued { .. }
        ));
        await_ready(&service);
        let out = poll(&mut engine, &service);
        let saved = out
            .events()
            .iter()
            .find_map(|(_, event)| match event {
                OutputEvent::InvokeResult {
                    result: InvokeResult::Mind(MindResponse::Saved { base, .. }),
                    ..
                } => Some(*base),
                _ => None,
            })
            .expect("real validated save completed");
        let (identity, source) = {
            let roll = engine.world.module_state::<mush_roll::RollStore>().unwrap();
            let mind = roll.mind(GOD, "mine").unwrap();
            (mind.latest().unwrap(), mind.draft().source.clone())
        };
        assert!(
            matches!(invoke(&mut engine, 9, other, 102, MindRequest::Lifecycle {
            name: "mine".into(), base: saved, edit: MindLifecycle::Remove,
        }), MindResponse::Error { code, .. } if code == "missing")
        );
        let MindResponse::Edited {
            base: renamed_version,
            ..
        } = invoke(
            &mut engine,
            7,
            GOD,
            103,
            MindRequest::Lifecycle {
                name: "mine".into(),
                base: saved,
                edit: MindLifecycle::VersionRename {
                    old: "first".into(),
                    new: "second".into(),
                },
            },
        )
        else {
            panic!("version rename");
        };
        assert!(
            matches!(invoke(&mut engine, 8, GOD, 104, MindRequest::Lifecycle {
            name: "mine".into(), base: saved,
            edit: MindLifecycle::VersionForget { version: "second".into() },
        }), MindResponse::Error { code, .. } if code == "stale")
        );
        assert!(matches!(
            invoke(
                &mut engine,
                7,
                GOD,
                105,
                MindRequest::Lifecycle {
                    name: "mine".into(),
                    base: renamed_version,
                    edit: MindLifecycle::Rename {
                        new_name: "Standard".into()
                    },
                }
            ),
            MindResponse::Error { .. }
        ));

        // Exercise registered line dispatch, not merely the shared parser.
        for line in [
            "@mind/version-name mine@second=third",
            "@mind/version-forget mine@third",
            "@mind/rename mine=Renamed Mind",
        ] {
            engine.step(
                Inbound::Cmd {
                    conn: 7,
                    actor: GOD,
                    line: line.into(),
                },
                &mut Outbox::new(),
                &mut Effects::new(),
            );
        }
        let current = {
            let roll = engine.world.module_state::<mush_roll::RollStore>().unwrap();
            assert!(roll.mind(GOD, "mine").is_err());
            let mind = roll.mind(GOD, "renamed mind").unwrap();
            assert_eq!(mind.latest(), Some(identity));
            assert_eq!(mind.draft().source, source);
            assert!(mind.named().is_empty());
            to_base(mind.base())
        };
        assert!(
            matches!(invoke(&mut engine, 8, GOD, 106, MindRequest::Lifecycle {
            name: "renamed mind".into(), base: saved, edit: MindLifecycle::Remove,
        }), MindResponse::Error { code, .. } if code == "stale")
        );
        assert!(
            matches!(invoke(&mut engine, 7, GOD, 107, MindRequest::Lifecycle {
            name: "renamed mind".into(), base: current, edit: MindLifecycle::Remove,
        }), MindResponse::Removed { name } if name == "renamed mind")
        );
        assert!(matches!(
            invoke(&mut engine, 7, GOD, 108, MindRequest::Prune { limit: 128 }),
            MindResponse::Pruned {
                revisions: 0,
                source_candidates_retained: 0
            }
        ));
        assert!(
            engine
                .world
                .module_state::<mush_roll::RollStore>()
                .unwrap()
                .revision(GOD, identity)
                .is_ok(),
            "last-fifty history survives removal and production prune"
        );
        for limit in [0, 129, u16::MAX] {
            assert!(matches!(
                invoke(&mut engine, 7, GOD, 109, MindRequest::Prune { limit }),
                MindResponse::Error { .. }
            ));
        }
        engine.step(
            Inbound::Cmd {
                conn: 7,
                actor: GOD,
                line: "@mind/new replacement".into(),
            },
            &mut Outbox::new(),
            &mut Effects::new(),
        );
        assert_ne!(
            engine
                .world
                .module_state::<mush_roll::RollStore>()
                .unwrap()
                .mind(GOD, "replacement")
                .unwrap()
                .id(),
            identity.mind
        );
    }

    #[test]
    fn m6_removed_draft_cannot_be_resurrected_by_a_pending_save_and_frees_capacity() {
        use mush_proto::MindLifecycle;
        let (mut engine, service, _) = rig();
        let base = create(&mut engine);
        for index in 1..mush_roll::MAX_MINDS {
            assert!(matches!(
                invoke(
                    &mut engine,
                    7,
                    GOD,
                    200 + index as u64,
                    MindRequest::Create {
                        name: format!("other {index}"),
                        from: None,
                    }
                ),
                MindResponse::Edited { .. }
            ));
        }
        assert!(matches!(
            invoke(
                &mut engine,
                7,
                GOD,
                250,
                MindRequest::Create {
                    name: "over capacity".into(),
                    from: None,
                }
            ),
            MindResponse::Error { .. }
        ));
        assert!(matches!(
            invoke(
                &mut engine,
                7,
                GOD,
                251,
                MindRequest::Save {
                    name: "mine".into(),
                    base,
                    notes: String::new(),
                    version: None,
                }
            ),
            MindResponse::Queued { .. }
        ));
        assert!(matches!(
            invoke(
                &mut engine,
                8,
                GOD,
                252,
                MindRequest::Lifecycle {
                    name: "mine".into(),
                    base,
                    edit: MindLifecycle::Remove,
                }
            ),
            MindResponse::Removed { .. }
        ));
        assert!(matches!(
            invoke(
                &mut engine,
                7,
                GOD,
                253,
                MindRequest::Create {
                    name: "replacement".into(),
                    from: None,
                }
            ),
            MindResponse::Edited { .. }
        ));
        await_ready(&service);
        let out = poll(&mut engine, &service);
        assert!(out.events().iter().any(|(_, event)| matches!(
            event,
            OutputEvent::InvokeResult {
                id: 251,
                result: InvokeResult::Mind(MindResponse::Error { .. })
            }
        )));
        assert!(
            engine
                .world
                .module_state::<mush_roll::RollStore>()
                .unwrap()
                .mind(GOD, "mine")
                .is_err()
        );
        assert!(
            engine
                .world
                .module_state::<crate::store::TacsimStore>()
                .unwrap()
                .mind_sources
                .is_empty(),
            "rejected pending save publishes no source"
        );
    }

    #[test]
    fn m6_prune_result_does_not_reveal_foreign_source_reuse_and_never_deletes_sources() {
        // Owner fixtures avoid repeating compilation: this test exercises the
        // actual authenticated prune service, not the already-tested save gate.
        struct ValidFixture;
        impl mush_roll::RevisionAdapter for ValidFixture {
            fn normalize_source(&self, source: &str) -> Result<String, RollError> {
                Ok(source.into())
            }
            fn validate(&self, _: &str, _: &Envelope) -> Result<(), RollError> {
                Ok(())
            }
        }
        let mut responses = Vec::new();
        for foreign_reuses_source in [false, true] {
            let (mut engine, _, other) = rig();
            let mut roll = mush_roll::RollStore::default();
            let mut sources = crate::store::TacsimStore::default();
            let created = super::super::create(&roll, &sources, GOD, "mine", None).unwrap();
            roll.publish_with(created, |_| {}).unwrap();
            let old_source = roll.mind(GOD, "mine").unwrap().draft().source.clone();
            for index in 0..51 {
                if index == 1 {
                    let mind = roll.mind(GOD, "mine").unwrap();
                    let mut draft = mind.draft().clone();
                    draft.source.push_str("// recent version\n");
                    let prepared = roll.prepare_edit(GOD, "mine", mind.base(), draft).unwrap();
                    roll.publish_with(prepared, |_| {}).unwrap();
                }
                let mind = roll.mind(GOD, "mine").unwrap();
                let source = mind.draft().source.clone();
                let save = roll
                    .prepare_save(GOD, "mine", mind.base(), "", None, &ValidFixture, &sources)
                    .unwrap();
                let (change, _, _) = save.into_parts();
                let digest = *mush_roll::source_digest(&source).as_bytes();
                assert_eq!(
                    crate::store::install_mind_source(&mut sources, &source),
                    (index < 2).then_some(digest),
                    "only the first save of each distinct source installs bytes"
                );
                assert_eq!(sources.mind_source(&digest), Some(source.as_str()));
                roll.publish_with(change, |_| {}).unwrap();
            }
            if foreign_reuses_source {
                let created =
                    super::super::create(&roll, &sources, other, "private", None).unwrap();
                roll.publish_with(created, |_| {}).unwrap();
                let base = roll.mind(other, "private").unwrap().base();
                let saved = roll
                    .prepare_save(other, "private", base, "", None, &ValidFixture, &sources)
                    .unwrap();
                let (change, _, _) = saved.into_parts();
                roll.publish_with(change, |_| {}).unwrap();
            }
            let old_digest = mush_roll::source_digest(&old_source);
            let advisory = roll.prepare_prune_owned(GOD, 1).unwrap();
            assert_eq!(advisory.removed.len(), 1);
            assert_eq!(
                advisory.source_candidates.contains(&old_digest),
                !foreign_reuses_source,
                "the internal collector has a global view that must not leak"
            );
            let before = sources.mind_sources.clone();
            engine.world.module_state_insert(roll);
            engine.world.module_state_insert(sources);
            responses.push(invoke(
                &mut engine,
                7,
                GOD,
                300,
                MindRequest::Prune { limit: 1 },
            ));
            assert_eq!(
                engine
                    .world
                    .module_state::<crate::store::TacsimStore>()
                    .unwrap()
                    .mind_sources,
                before
            );
            assert!(
                engine
                    .world
                    .module_state::<mush_roll::RollStore>()
                    .unwrap()
                    .prepare_prune_owned(GOD, 1)
                    .unwrap()
                    .removed
                    .is_empty()
            );
        }
        assert_eq!(responses[0], responses[1]);
        assert_eq!(
            responses[0],
            MindResponse::Pruned {
                revisions: 1,
                source_candidates_retained: 1
            }
        );
    }

    #[test]
    fn remote_jobs_are_connection_private_and_stale_completion_never_saves() {
        let (mut engine, service, _) = rig();
        let base = create(&mut engine);
        let MindResponse::Queued { job } = invoke(
            &mut engine,
            7,
            GOD,
            20,
            MindRequest::Save {
                name: "mine".into(),
                base,
                notes: String::new(),
                version: None,
            },
        ) else {
            panic!("queued");
        };
        assert!(
            matches!(invoke(&mut engine, 8, GOD, 21, MindRequest::Status { job }),
            MindResponse::Error { code, .. } if code == "missing")
        );
        let MindResponse::Read { mut draft, .. } = invoke(
            &mut engine,
            8,
            GOD,
            22,
            MindRequest::Read {
                name: "mine".into(),
                revision: None,
            },
        ) else {
            panic!("read");
        };
        draft.source.push('\n');
        assert!(matches!(
            invoke(
                &mut engine,
                8,
                GOD,
                23,
                MindRequest::Replace {
                    name: "mine".into(),
                    base,
                    draft
                }
            ),
            MindResponse::Edited { .. }
        ));
        await_ready(&service);
        let out = poll(&mut engine, &service);
        assert!(
            matches!(out.events(), [(Audience::Conn(7), OutputEvent::InvokeResult {
            id: 20, result: InvokeResult::Mind(MindResponse::Error { code, .. }) })] if code == "stale")
        );
        assert!(
            engine
                .world
                .module_state::<mush_roll::RollStore>()
                .expect("roll")
                .mind(GOD, "mine")
                .expect("mind")
                .latest()
                .is_none()
        );
    }

    #[test]
    fn connection_reuse_same_actor_invalidates_original_job_lease() {
        let (mut engine, service, _) = rig();
        let base = create(&mut engine);
        let old_lease = engine.world.connection_binding(7);
        assert!(matches!(
            invoke(
                &mut engine,
                7,
                GOD,
                30,
                MindRequest::Check {
                    name: "mine".into(),
                    base
                }
            ),
            MindResponse::Queued { .. }
        ));
        await_ready(&service);
        engine.step(
            Inbound::ConnDown {
                conn: 7,
                actor: GOD,
                reason: mush_core::DisconnectKind::Lost,
            },
            &mut Outbox::new(),
            &mut Effects::new(),
        );
        attach(&mut engine, 7, GOD);
        assert_ne!(engine.world.connection_binding(7), old_lease);
        let out = poll(&mut engine, &service);
        assert!(
            out.events().is_empty(),
            "old source diagnostics cannot cross a connection lifetime"
        );
        assert!(service.executor.ready().is_empty(), "completion discarded");
    }

    #[test]
    fn read_distinguishes_display_from_exact_editable_rule_spelling() {
        let (mut engine, _, _) = rig();
        let base = create(&mut engine);
        let MindResponse::Read { mut draft, .. } = invoke(
            &mut engine,
            7,
            GOD,
            50,
            MindRequest::Read {
                name: "mine".into(),
                revision: None,
            },
        ) else {
            panic!("read");
        };
        let profile = draft
            .profiles
            .as_object()
            .expect("profiles")
            .keys()
            .next()
            .expect("opening profile")
            .clone();
        let authored = super::super::author_rules::parse("FIRE all HOLD WHEN hostiles>=2", 0)
            .expect("authored grammar");
        let no_spelling = crate::arena::schema::AuthoredRule {
            rule: crate::arena::conditions::Rule::hold_rule("cannon".into(), vec![]),
            think: vec![],
        };
        draft.rules = serde_json::to_value(BTreeMap::from([(
            profile.clone(),
            vec![authored.clone(), no_spelling.clone()],
        )]))
        .expect("rule envelope");
        let original = draft.clone();
        assert!(matches!(
            invoke(
                &mut engine,
                7,
                GOD,
                51,
                MindRequest::Replace {
                    name: "mine".into(),
                    base,
                    draft
                }
            ),
            MindResponse::Edited { .. }
        ));
        let MindResponse::Read {
            draft,
            rules_text,
            rules_source,
            diagnostics,
            ..
        } = invoke(
            &mut engine,
            7,
            GOD,
            52,
            MindRequest::Read {
                name: "mine".into(),
                revision: None,
            },
        )
        else {
            panic!("read");
        };
        assert_eq!(draft, original, "read never rewrites the structured rule");
        let text = rules_text.get(&profile).expect("display");
        assert_eq!(text.len(), 2);
        assert!(text.first().expect("first display").contains("holds fire"));
        let sources = rules_source.get(&profile).expect("editable");
        let source = sources
            .first()
            .and_then(Option::as_deref)
            .expect("grammar prefill");
        assert_eq!(
            super::super::author_rules::parse(source, 0).expect("parse prefill"),
            authored
        );
        assert_eq!(
            sources.get(1),
            Some(&None),
            "unconditional HOLD has no grammar spelling"
        );
        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics
                .first()
                .expect("diagnostic")
                .field
                .ends_with("[1]")
        );
    }

    #[test]
    fn maximum_source_and_large_owner_legal_metadata_round_trip_without_truncation() {
        let (mut engine, _, _) = rig();
        let base = create(&mut engine);
        let schema = serde_json::Value::Object(
            (0..8)
                .map(|i| {
                    (
                        format!("field{i}"),
                        serde_json::Value::String("a".repeat(7_000)),
                    )
                })
                .collect(),
        );
        let draft = MindDraft {
            source: "\u{0001}".repeat(mush_roll::MAX_SOURCE_BYTES),
            schema,
            presets: serde_json::json!({}),
            profiles: serde_json::json!({"Default":{}}),
            rules: serde_json::json!({}),
            declared_class: "I".into(),
            mind_abi: crate::minds::MIND_ABI.into(),
        };
        let copy = from_draft(draft.clone());
        copy.validate_bounds().expect("owner legal draft");
        let request = InvokeRequest::Mind(MindRequest::Replace {
            name: "mine".into(),
            base,
            draft: Box::new(draft.clone()),
        });
        request.validate().expect("transport accepts owner maxima");
        let json = serde_json::to_vec(&request).expect("json");
        assert!(
            json.len() > 240 * 1024,
            "regression exercises six-byte escaping of every source byte"
        );
        assert_eq!(
            serde_json::from_slice::<InvokeRequest>(&json).expect("decode"),
            request
        );
        let InvokeRequest::Mind(request) = request;
        let edited = invoke(&mut engine, 7, GOD, 40, request);
        assert!(matches!(edited, MindResponse::Edited { .. }), "{edited:?}");
        let response = invoke(
            &mut engine,
            7,
            GOD,
            41,
            MindRequest::Read {
                name: "mine".into(),
                revision: None,
            },
        );
        let MindResponse::Read { draft: actual, .. } = &response else {
            panic!("bounded read");
        };
        assert_eq!(**actual, draft);
        let result = InvokeResult::Mind(response).bounded();
        let bytes = serde_json::to_vec(&result).expect("reply json");
        assert_eq!(
            serde_json::from_slice::<InvokeResult>(&bytes).expect("reply decode"),
            result
        );
    }
}

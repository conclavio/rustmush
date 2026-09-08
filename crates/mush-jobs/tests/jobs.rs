//! End-to-end tests for the jobs module through the real [`Engine`]:
//! the request → comment → assign → approve lifecycle (including payload
//! handlers), denial, role-gated visibility, character notes with staff
//! visibility and approval, opener notices, write-through persistence
//! round trips, and id sequencing across reboots.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use mush_core::engine::{Effects, Engine, Inbound};
use mush_core::{
    ArgShape, Attr, Audience, CmdError, CommandSpec, Ctx, Flags, GOD, GameModule, JobQueueSpec,
    LIMBO, ModError, ModuleBootData, ModuleCtx, ModuleOp, ModuleWrite, ObjId, ObjKind, Outbox,
    ParsedArgs, Registrar, Registry, Role, SwitchSet, World, register_core_commands,
    register_engine_commands,
};
use mush_jobs::{
    Job, JobPayload, JobStatus, JobsModule, MAX_JOB_COMMENT_BYTES, MAX_JOB_TITLE_BYTES,
    MAX_ORDINARY_JOB_COMMENTS, OpenJob, PlainJobError, deny_plain_job, get_job, get_note, open_job,
    register_payload_handler, resolve_plain_job,
};
use mush_proto::OutputEvent;
use serde_json::json;
use smol_str::SmolStr;

// ----------------------------------------------------------------------
// A producing module (stands in for the WoD ruleset's M6 XP flow)
// ----------------------------------------------------------------------

/// Registers the `xp` queue (Wizard-gated), a `+xspend` command that files
/// payload-carrying jobs through [`open_job`], and two payload handlers —
/// one applying, one failing — through [`register_payload_handler`].
struct TestWod {
    applied: Arc<AtomicU32>,
}

impl GameModule for TestWod {
    fn name(&self) -> &'static str {
        "wod"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.job_queue(JobQueueSpec {
            name: "xp".into(),
            title: "XP spends".into(),
            min_role: Role::Wizard,
        })?;
        reg.command(
            CommandSpec {
                name: "+xspend".into(),
                shape: ArgShape::EqPair,
                help: "+xspend <title>=<handler> -- test: file a payload job in xp.".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let title = ctx.args.lhs().to_string();
                let handler = ctx.args.rhs().unwrap_or_default().to_string();
                let id = open_job(
                    ctx,
                    OpenJob {
                        queue: "xp".into(),
                        title,
                        opener: ctx.actor,
                        text: None,
                        payload: Some(JobPayload {
                            handler: handler.into(),
                            data: json!({"dots": 1}),
                        }),
                        tags: vec!["xp".into()],
                    },
                )
                .map_err(CmdError::msg)?;
                ctx.out
                    .info(ctx.actor, format!("XP spend filed as job {id}."));
                Ok(())
            }),
        )?;
        // Look-back fold, D-LBK-6: the module-side terminal path, driven from a
        // command so it runs under a real `Ctx`. `+plainclose <id>=<queue> resolve|deny`
        // reports the seam's own error text verbatim.
        reg.command(
            CommandSpec {
                name: "+plainclose".into(),
                shape: ArgShape::EqPair,
                help: "+plainclose <id>=<queue> <resolve|deny> -- test: the plain-job seam".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let id: u64 = ctx
                    .args
                    .lhs()
                    .trim()
                    .parse()
                    .map_err(|_| CmdError::msg("id"))?;
                let rhs = ctx.args.rhs().unwrap_or_default().to_string();
                let mut words = rhs.split_whitespace();
                let queue = words.next().unwrap_or("").to_string();
                let how = words.next().unwrap_or("resolve").to_string();
                let by = ctx.actor;
                let outcome = if how == "deny" {
                    deny_plain_job(ctx, id, &queue, by, "withdrawn by the module")
                } else {
                    resolve_plain_job(ctx, id, &queue, by, "answered by the module")
                };
                match outcome {
                    Ok(()) => {
                        ctx.out.info(by, format!("plain close ok: job {id} {how}d"));
                        Ok(())
                    }
                    Err(e) => Err(CmdError::msg(format!("plain close refused: {e}"))),
                }
            }),
        )?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        _data: ModuleBootData,
    ) -> Result<(), ModError> {
        let applied = Arc::clone(&self.applied);
        register_payload_handler(
            ctx.w,
            "xp.apply",
            Arc::new(move |ctx: &mut Ctx<'_>, job: &Job| {
                let dots = job
                    .payload
                    .as_ref()
                    .and_then(|p| p.data.get("dots"))
                    .and_then(serde_json::Value::as_u64);
                if dots != Some(1) {
                    return Err("bad payload".to_string());
                }
                // A real handler mutates the world (e.g. applies a sheet
                // diff); prove that path works.
                ctx.w
                    .set_attr(job.opener, "xp-applied", Attr::text("yes", job.opener))
                    .map_err(|e| e.to_string())?;
                applied.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
        )
        .map_err(ModError::msg)?;
        register_payload_handler(
            ctx.w,
            "xp.fail",
            Arc::new(|_ctx: &mut Ctx<'_>, _job: &Job| Err("the dice exploded.".to_string())),
        )
        .map_err(ModError::msg)?;
        // The dangerous shape: queues a persistence op, THEN fails
        // validation — dispatch must discard the queued op.
        register_payload_handler(
            ctx.w,
            "xp.half",
            Arc::new(|ctx: &mut Ctx<'_>, _job: &Job| {
                ctx.persist(ModuleOp {
                    ns: "wod".into(),
                    op: ModuleWrite::JsonSet {
                        key: "sheet:leak".into(),
                        value: json!({"debited": true}),
                    },
                });
                Err("second doc failed.".to_string())
            }),
        )
        .map_err(ModError::msg)?;
        Ok(())
    }
}

// ----------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------

struct Env {
    engine: Engine,
    /// Plain player.
    nick: ObjId,
    /// Plain player.
    bob: ObjId,
    /// Royalty staffer (can work REQ, not xp).
    rita: ObjId,
    /// Wizard staffer (can work everything).
    wanda: ObjId,
    /// Successful-payload application count.
    applied: Arc<AtomicU32>,
}

/// Engine at now=1000s with the jobs module + the TestWod producer and
/// four players; `modules_loaded` already delivered `boot`.
fn setup_with(boot: HashMap<SmolStr, ModuleBootData>) -> Env {
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let nick = world
        .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
        .unwrap();
    let bob = world
        .create(ObjKind::Player, "Bob", GOD, Some(LIMBO))
        .unwrap();
    let rita = world
        .create(ObjKind::Player, "Rita", GOD, Some(LIMBO))
        .unwrap();
    world.set_flag(rita, Flags::ROYALTY, true).unwrap();
    let wanda = world
        .create(ObjKind::Player, "Wanda", GOD, Some(LIMBO))
        .unwrap();
    world.set_flag(wanda, Flags::WIZARD, true).unwrap();
    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    register_engine_commands(&mut registry).unwrap();
    let applied = Arc::new(AtomicU32::new(0));
    let mut engine = Engine::with_modules(
        world,
        registry,
        vec![
            Box::new(JobsModule::new()),
            Box::new(TestWod {
                applied: Arc::clone(&applied),
            }),
        ],
    )
    .unwrap();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.modules_loaded(boot, &mut out, &mut fx).unwrap();
    Env {
        engine,
        nick,
        bob,
        rita,
        wanda,
        applied,
    }
}

fn setup() -> Env {
    setup_with(HashMap::new())
}

/// Run one command line, returning its outbox. (`actor` first so call
/// sites can pass `env.nick, &mut env` — the copy happens before the
/// mutable borrow.)
fn run(actor: ObjId, env: &mut Env, line: &str) -> Outbox {
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    env.engine.step(
        Inbound::Cmd {
            conn: 1,
            actor,
            line: line.into(),
        },
        &mut out,
        &mut fx,
    );
    out
}

/// All Sys texts in the outbox (any audience).
fn sys_texts(out: &Outbox) -> Vec<String> {
    out.events()
        .iter()
        .filter_map(|(_, e)| match e {
            OutputEvent::Sys(t) => Some(t.to_plain_text()),
            _ => None,
        })
        .collect()
}

fn has_text(out: &Outbox, needle: &str) -> bool {
    sys_texts(out).iter().any(|t| t.contains(needle))
}

/// Sys texts tagged `"jobs"` addressed to exactly `who` — the opener
/// notices.
fn jobs_notices(out: &Outbox, who: ObjId) -> Vec<String> {
    out.events()
        .iter()
        .filter_map(|(aud, e)| match (aud, e) {
            (Audience::Actor(a), OutputEvent::Sys(t))
                if *a == who
                    && t.spans().first().and_then(|s| s.tag.as_deref()) == Some("jobs") =>
            {
                Some(t.to_plain_text())
            }
            _ => None,
        })
        .collect()
}

/// A fake store: applies the module ops the driver would append to the
/// `WriteBatch`, and hands the accumulated documents back as boot data.
#[derive(Default, Clone)]
struct FakeKv(BTreeMap<SmolStr, serde_json::Value>);

impl FakeKv {
    fn apply(&mut self, ops: Vec<ModuleOp>) {
        for op in ops {
            assert_eq!(op.ns, "jobs", "jobs writes only its own namespace");
            match op.op {
                ModuleWrite::JsonSet { key, value } => {
                    self.0.insert(key, value);
                }
                ModuleWrite::Del { key } => {
                    self.0.remove(&key);
                }
                ModuleWrite::StreamAppend { .. } => panic!("jobs emits no stream ops"),
            }
        }
    }

    fn boot(&self) -> HashMap<SmolStr, ModuleBootData> {
        HashMap::from([(
            SmolStr::new("jobs"),
            ModuleBootData {
                kv: self.0.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                streams: HashMap::new(),
                ..Default::default()
            },
        )])
    }
}

/// Run one command and write its module ops through to the fake store.
fn run_p(actor: ObjId, env: &mut Env, store: &mut FakeKv, line: &str) -> Outbox {
    let out = run(actor, env, line);
    store.apply(env.engine.take_module_ops());
    out
}

// ----------------------------------------------------------------------
// +request and listing
// ----------------------------------------------------------------------

#[test]
fn request_files_a_job_in_req() {
    let mut env = setup();
    let out = run(
        env.nick,
        &mut env,
        "+request Better desc=Please describe my room.",
    );
    assert!(has_text(&out, "job 1"), "{:?}", sys_texts(&out));
    let job = get_job(&env.engine.world, 1).expect("job exists").clone();
    assert_eq!(job.queue, "REQ");
    assert_eq!(job.title, "Better desc");
    assert_eq!(job.opener, env.nick);
    assert_eq!(job.assignee, None);
    assert_eq!(job.status, JobStatus::Open);
    assert_eq!(job.comments.len(), 1);
    assert_eq!(job.comments[0].text, "Please describe my room.");
    assert_eq!(job.comments[0].who, env.nick);
    assert_eq!(job.opened_at, 1_000);
    // Usage errors.
    let out = run(env.nick, &mut env, "+request no equals sign");
    assert!(has_text(&out, "Usage: +request"));
    let out = run(env.nick, &mut env, "+request =text only");
    assert!(has_text(&out, "Usage: +request"));
    let out = run(env.nick, &mut env, "+request title only=");
    assert!(has_text(&out, "Usage: +request"));
    // Nothing extra was filed.
    assert!(get_job(&env.engine.world, 2).is_none());
}

#[test]
fn jobs_listing_and_detail() {
    let mut env = setup();
    run(env.nick, &mut env, "+request Better desc=Please.");
    let out = run(env.rita, &mut env, "+jobs");
    assert!(has_text(&out, "Better desc"));
    assert!(has_text(&out, "REQ"));
    assert!(has_text(&out, "Nick"));
    // Detail shows status, opener, and the comment trail.
    let out = run(env.rita, &mut env, "+job 1");
    assert!(has_text(&out, "Job 1 [REQ] -- Better desc"));
    assert!(has_text(&out, "Status: Open"));
    assert!(has_text(&out, "Nick"));
    assert!(has_text(&out, "Please."));
    // Unknown queue / job.
    let out = run(env.rita, &mut env, "+jobs bogus");
    assert!(has_text(&out, "No such job queue"));
    let out = run(env.rita, &mut env, "+job 99");
    assert!(has_text(&out, "No such job."));
    let out = run(env.rita, &mut env, "+job nonsense");
    assert!(has_text(&out, "not a job number"));
    // Queue filter is case-insensitive.
    let out = run(env.rita, &mut env, "+jobs req");
    assert!(has_text(&out, "Better desc"));
}

// ----------------------------------------------------------------------
// Lifecycle: comment → assign → approve, with opener notices
// ----------------------------------------------------------------------

#[test]
fn lifecycle_request_comment_assign_approve() {
    let mut env = setup();
    run(env.nick, &mut env, "+request Better desc=Please.");
    env.engine.world.set_connected(env.nick, true);

    // Comment: confirmation + opener notice.
    let out = run(env.rita, &mut env, "+job/comment 1=Looking into it.");
    assert!(has_text(&out, "Comment added to job 1."));
    let notices = jobs_notices(&out, env.nick);
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(notices[0].contains("Rita commented on job 1"));
    assert!(notices[0].contains("Looking into it."));

    // Assign: Open → In progress, opener + assignee notices.
    let out = run(env.rita, &mut env, "+job/assign 1=Wanda");
    assert!(has_text(&out, "Job 1 assigned to Wanda."));
    let job = get_job(&env.engine.world, 1).unwrap();
    assert_eq!(job.assignee, Some(env.wanda));
    assert_eq!(job.status, JobStatus::InProgress);
    assert!(
        jobs_notices(&out, env.nick)
            .iter()
            .any(|t| t.contains("assigned to Wanda"))
    );

    // Approve with a comment: Resolved, notice carries the comment.
    let out = run(env.rita, &mut env, "+job/approve 1=All set.");
    assert!(has_text(&out, "Job 1 approved."));
    let job = get_job(&env.engine.world, 1).unwrap();
    assert_eq!(job.status, JobStatus::Resolved);
    assert_eq!(job.comments.len(), 3, "opening text + comment + approval");
    assert_eq!(job.comments[2].text, "All set.");
    let notices = jobs_notices(&out, env.nick);
    assert!(notices.iter().any(|t| t.contains("approved by Rita")));
    assert!(notices.iter().any(|t| t.contains("All set.")));

    // Terminal jobs refuse a second approval or denial.
    let out = run(env.rita, &mut env, "+job/approve 1");
    assert!(has_text(&out, "already resolved"));
    let out = run(env.rita, &mut env, "+job/deny 1=nope");
    assert!(has_text(&out, "already resolved"));
}

#[test]
fn deny_and_status_override() {
    let mut env = setup();
    run(env.nick, &mut env, "+request Thing=Do the thing.");
    env.engine.world.set_connected(env.nick, true);
    // Reason required.
    let out = run(env.rita, &mut env, "+job/deny 1");
    assert!(has_text(&out, "Usage: +job/deny"));
    // Denial: status, reason comment, notice.
    let out = run(env.rita, &mut env, "+job/deny 1=Not possible.");
    assert!(has_text(&out, "Job 1 denied."));
    let job = get_job(&env.engine.world, 1).unwrap();
    assert_eq!(job.status, JobStatus::Denied);
    assert_eq!(job.comments.last().unwrap().text, "Not possible.");
    assert!(
        jobs_notices(&out, env.nick)
            .iter()
            .any(|t| t.contains("denied by Rita: Not possible."))
    );
    // /status re-opens (staff only; never runs payload handlers).
    let out = run(env.rita, &mut env, "+job/status 1=open");
    assert!(has_text(&out, "Job 1 status set to Open."));
    assert_eq!(
        get_job(&env.engine.world, 1).unwrap().status,
        JobStatus::Open
    );
    assert!(
        jobs_notices(&out, env.nick)
            .iter()
            .any(|t| t.contains("status changed to Open by Rita"))
    );
    // Interface plan I11 (look-back I11-C24): the opener also gets a `Notice`
    // in the `jobs` category with an `Open` action prefilling `+job 1` — and
    // the staffer who made the change gets none.
    let notices: Vec<(ObjId, String, Option<String>)> = out
        .events()
        .iter()
        .filter_map(|(aud, e)| match (aud, e) {
            (
                mush_core::Audience::Actor(a),
                OutputEvent::Notice {
                    category, action, ..
                },
            ) if category.as_str() == "jobs" => Some((
                *a,
                category.to_string(),
                action.as_ref().map(|act| act.prefill.clone()),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert_eq!(notices[0].0, env.nick);
    assert_eq!(notices[0].2.as_deref(), Some("+job 1"));
    let out = run(env.rita, &mut env, "+job/status 1=in-progress");
    assert_eq!(
        get_job(&env.engine.world, 1).unwrap().status,
        JobStatus::InProgress
    );
    assert!(has_text(&out, "In progress"));
    // Players may not override status; bad names are rejected.
    let out = run(env.nick, &mut env, "+job/status 1=resolved");
    assert!(has_text(&out, "Permission denied."));
    let out = run(env.rita, &mut env, "+job/status 1=bogus");
    assert!(has_text(&out, "Valid statuses"));
}

#[test]
fn assign_requires_a_capable_staffer() {
    let mut env = setup();
    run(env.nick, &mut env, "+request T=t.");
    // Assignee must be able to work the queue.
    let out = run(env.rita, &mut env, "+job/assign 1=Nick");
    assert!(has_text(&out, "Nick cannot work queue REQ."));
    assert_eq!(get_job(&env.engine.world, 1).unwrap().assignee, None);
    let out = run(env.rita, &mut env, "+job/assign 1=Nobody");
    assert!(has_text(&out, "No such character"));
    // Players cannot assign at all.
    let out = run(env.nick, &mut env, "+job/assign 1=Rita");
    assert!(has_text(&out, "Permission denied."));
    // Royalty assigning royalty works.
    let out = run(env.rita, &mut env, "+job/assign 1=Rita");
    assert!(has_text(&out, "Job 1 assigned to Rita."));
    assert_eq!(
        get_job(&env.engine.world, 1).unwrap().assignee,
        Some(env.rita)
    );
}

// ----------------------------------------------------------------------
// Payload handlers
// ----------------------------------------------------------------------

#[test]
fn approve_applies_the_payload_handler() {
    let mut env = setup();
    let out = run(env.nick, &mut env, "+xspend Buy a dot=xp.apply");
    assert!(has_text(&out, "job 1"));
    let job = get_job(&env.engine.world, 1).unwrap();
    assert_eq!(job.queue, "xp");
    assert_eq!(job.payload.as_ref().unwrap().handler, "xp.apply");
    assert_eq!(job.tags, vec![SmolStr::new("xp")]);
    // Royalty cannot work the Wizard-gated xp queue.
    let out = run(env.rita, &mut env, "+job/approve 1");
    assert!(has_text(&out, "Permission denied."));
    assert_eq!(env.applied.load(Ordering::SeqCst), 0);
    // A Wizard can: the handler runs (mutating the world), then the flip.
    let out = run(env.wanda, &mut env, "+job/approve 1=Enjoy.");
    assert!(has_text(&out, "Job 1 approved."));
    assert_eq!(env.applied.load(Ordering::SeqCst), 1);
    assert_eq!(
        get_job(&env.engine.world, 1).unwrap().status,
        JobStatus::Resolved
    );
    let attr = env.engine.world.attr(env.nick, "xp-applied");
    assert!(attr.is_some(), "handler world mutation applied");
}

#[test]
fn payload_handler_failure_leaves_the_job_untouched() {
    let mut env = setup();
    let mut store = FakeKv::default();
    run_p(env.nick, &mut env, &mut store, "+xspend Bad spend=xp.fail");
    run_p(
        env.nick,
        &mut env,
        &mut store,
        "+xspend Ghost spend=xp.ghost",
    );

    // Handler error: reported, no status change, nothing persisted.
    let out = run(env.wanda, &mut env, "+job/approve 1");
    assert!(
        has_text(&out, "the dice exploded."),
        "{:?}",
        sys_texts(&out)
    );
    assert!(has_text(&out, "Job 1 unchanged."));
    assert_eq!(
        get_job(&env.engine.world, 1).unwrap().status,
        JobStatus::Open
    );
    assert_eq!(env.applied.load(Ordering::SeqCst), 0);
    assert!(
        env.engine.take_module_ops().is_empty(),
        "a failed approval writes nothing"
    );

    // Missing handler: same protection.
    let out = run(env.wanda, &mut env, "+job/approve 2");
    assert!(has_text(&out, "No payload handler"));
    assert_eq!(
        get_job(&env.engine.world, 2).unwrap().status,
        JobStatus::Open
    );
    assert!(env.engine.take_module_ops().is_empty());

    // Denial never invokes the handler.
    let out = run(env.wanda, &mut env, "+job/deny 1=No.");
    assert!(has_text(&out, "Job 1 denied."));
    assert_eq!(env.applied.load(Ordering::SeqCst), 0);
}

#[test]
fn payload_handler_registration_rules() {
    let mut env = setup();
    // Duplicate names are refused.
    let err = register_payload_handler(
        &mut env.engine.world,
        "xp.apply",
        Arc::new(|_: &mut Ctx<'_>, _: &Job| Ok(())),
    )
    .unwrap_err();
    assert!(err.contains("already registered"), "{err}");
    // A world without the jobs module has no registry.
    let mut bare = World::new_minimal();
    let err = register_payload_handler(&mut bare, "x", Arc::new(|_: &mut Ctx<'_>, _: &Job| Ok(())))
        .unwrap_err();
    assert!(err.contains("not loaded"), "{err}");
}

#[test]
fn open_job_validations() {
    let mut env = setup();
    let mut out = Outbox::new();
    let mut ctx = Ctx {
        w: &mut env.engine.world,
        actor: env.nick,
        switches: SwitchSet::default(),
        args: ParsedArgs::default(),
        out: &mut out,
        reg: &env.engine.registry,
        module: None,
        typed_state: mush_core::ModuleActionState::unavailable(),
    };
    let err = ctx_open(&mut ctx, "ghostqueue", "T").unwrap_err();
    assert!(err.contains("no such job queue"), "{err}");
    let err = ctx_open(&mut ctx, "REQ", "   ").unwrap_err();
    assert!(err.contains("needs a title"), "{err}");
    let err = ctx_open(&mut ctx, "REQ", &"x".repeat(MAX_JOB_TITLE_BYTES + 1)).unwrap_err();
    assert!(err.contains("at most"), "{err}");
    let actor = ctx.actor;
    let err = open_job(
        &mut ctx,
        OpenJob {
            queue: "REQ".into(),
            title: "Oversized opening comment".into(),
            opener: actor,
            text: Some("x".repeat(MAX_JOB_COMMENT_BYTES + 1)),
            payload: None,
            tags: Vec::new(),
        },
    )
    .unwrap_err();
    assert!(err.contains("comment may contain"), "{err}");
    // Queue names match case-insensitively and canonicalize.
    let id = ctx_open(&mut ctx, "req", "Lower").unwrap();
    assert_eq!(get_job(ctx.w, id).unwrap().queue, "REQ");
}

#[test]
fn ordinary_comments_are_bounded_but_leave_terminal_audit_capacity() {
    let mut env = setup();
    run(env.nick, &mut env, "+request Busy=opening");
    for n in 1..MAX_ORDINARY_JOB_COMMENTS {
        let nick = env.nick;
        let out = run(nick, &mut env, &format!("+job/comment 1=comment {n}"));
        assert!(has_text(&out, "Comment added"), "comment {n}");
    }
    let nick = env.nick;
    let out = run(nick, &mut env, "+job/comment 1=one too many");
    assert!(has_text(&out, "ordinary comment limit"));

    let rita = env.rita;
    let out = run(rita, &mut env, "+job/deny 1=terminal reason");
    assert!(has_text(&out, "Job 1 denied"));
    let job = get_job(&env.engine.world, 1).unwrap();
    assert_eq!(job.comments.len(), MAX_ORDINARY_JOB_COMMENTS + 1);
    assert_eq!(job.status, JobStatus::Denied);
}

fn ctx_open(ctx: &mut Ctx<'_>, queue: &str, title: &str) -> Result<u64, String> {
    open_job(
        ctx,
        OpenJob {
            queue: queue.into(),
            title: title.into(),
            opener: ctx.actor,
            text: None,
            payload: None,
            tags: Vec::new(),
        },
    )
}

// ----------------------------------------------------------------------
// Visibility
// ----------------------------------------------------------------------

#[test]
fn players_see_only_their_own_jobs() {
    let mut env = setup();
    run(env.nick, &mut env, "+request Mine=One.");
    run(env.bob, &mut env, "+request His=Two.");
    // Nick opened an xp-queue job through the producing module: openers
    // always see their own jobs even in queues above their role.
    run(env.nick, &mut env, "+xspend Dot=xp.apply");

    let out = run(env.nick, &mut env, "+jobs");
    assert!(has_text(&out, "Mine"));
    assert!(!has_text(&out, "His"));
    assert!(has_text(&out, "Dot"), "opener sees own xp job");

    let out = run(env.bob, &mut env, "+jobs");
    assert!(has_text(&out, "His"));
    assert!(!has_text(&out, "Mine"));
    assert!(!has_text(&out, "Dot"));

    // Royalty sees REQ (its min role) but not the Wizard-gated xp queue.
    let out = run(env.rita, &mut env, "+jobs");
    assert!(has_text(&out, "Mine") && has_text(&out, "His"));
    assert!(!has_text(&out, "Dot"));
    let out = run(env.rita, &mut env, "+jobs xp");
    assert!(has_text(&out, "No jobs to show."));

    // Wizards see everything.
    let out = run(env.wanda, &mut env, "+jobs");
    assert!(has_text(&out, "Mine") && has_text(&out, "His") && has_text(&out, "Dot"));

    // Detail and staff ops respect the same wall.
    let out = run(env.nick, &mut env, "+job 2");
    assert!(has_text(&out, "Permission denied."));
    let out = run(env.nick, &mut env, "+job/approve 1");
    assert!(has_text(&out, "Permission denied."));
    let out = run(env.nick, &mut env, "+job/comment 2=peek");
    assert!(has_text(&out, "Permission denied."));
    let out = run(env.rita, &mut env, "+job 3");
    assert!(has_text(&out, "Permission denied."));
    // The opener can read and comment on their own job.
    let out = run(env.nick, &mut env, "+job 3");
    assert!(has_text(&out, "Job 3 [xp] -- Dot"));
    let out = run(env.nick, &mut env, "+job/comment 1=Any news?");
    assert!(has_text(&out, "Comment added to job 1."));
}

// ----------------------------------------------------------------------
// Opener notices
// ----------------------------------------------------------------------

#[test]
fn opener_notices_require_connection_and_skip_self() {
    let mut env = setup();
    run(env.nick, &mut env, "+request T=t.");
    // Disconnected opener: no notice.
    let out = run(env.rita, &mut env, "+job/comment 1=First.");
    assert!(jobs_notices(&out, env.nick).is_empty());
    // Connected: notice.
    env.engine.world.set_connected(env.nick, true);
    let out = run(env.rita, &mut env, "+job/comment 1=Second.");
    assert_eq!(jobs_notices(&out, env.nick).len(), 1);
    // The opener acting on their own job never self-notices.
    let out = run(env.nick, &mut env, "+job/comment 1=Me too.");
    assert!(jobs_notices(&out, env.nick).is_empty());
    assert!(has_text(&out, "Comment added to job 1."));
}

// ----------------------------------------------------------------------
// Notes
// ----------------------------------------------------------------------

#[test]
fn notes_crud_and_staff_only_hiding() {
    let mut env = setup();
    // Staff writes a note on a player; player writes their own.
    let out = run(
        env.rita,
        &mut env,
        "+note Nick/Background=Grew up in Limbo.",
    );
    assert!(has_text(&out, "Note 'background' set on Nick."));
    let note = get_note(&env.engine.world, env.nick, "background").unwrap();
    assert_eq!(note.author, env.rita);
    assert!(!note.staff_only);
    assert_eq!(note.text, "Grew up in Limbo.");

    let out = run(env.nick, &mut env, "+note me/goals=Become somebody.");
    assert!(has_text(&out, "Note 'goals' set on Nick."));

    // Permission walls.
    let out = run(env.bob, &mut env, "+note Nick/hobby=nosy");
    assert!(has_text(&out, "Permission denied."));
    let out = run(env.nick, &mut env, "+note/staff me/secret=hidden");
    assert!(has_text(&out, "Permission denied."));

    // Staff-only note, hidden from the character.
    run(
        env.rita,
        &mut env,
        "+note/staff Nick/watch=Keep an eye on him.",
    );
    assert!(
        get_note(&env.engine.world, env.nick, "watch")
            .unwrap()
            .staff_only
    );

    let out = run(env.nick, &mut env, "+notes");
    assert!(has_text(&out, "background") && has_text(&out, "goals"));
    assert!(!has_text(&out, "watch"), "{:?}", sys_texts(&out));

    let out = run(env.nick, &mut env, "+note me");
    assert!(has_text(&out, "background") && !has_text(&out, "watch"));
    let out = run(env.nick, &mut env, "+note me/watch");
    assert!(has_text(&out, "No such note."));
    // The character cannot clobber a staff-only note either.
    let out = run(env.nick, &mut env, "+note me/watch=clobber");
    assert!(has_text(&out, "Permission denied."));

    // Other players cannot list someone else's notes; staff sees all.
    let out = run(env.bob, &mut env, "+note Nick");
    assert!(has_text(&out, "Permission denied."));
    let out = run(env.rita, &mut env, "+note Nick");
    assert!(has_text(&out, "watch") && has_text(&out, "[staff-only]"));

    // Reading one note; categories are case-insensitive.
    let out = run(env.nick, &mut env, "+note me/GOALS");
    assert!(has_text(&out, "Become somebody."));

    // Deletion via empty text.
    let out = run(env.rita, &mut env, "+note Nick/watch=");
    assert!(has_text(&out, "Note 'watch' on Nick deleted."));
    assert!(get_note(&env.engine.world, env.nick, "watch").is_none());
    let out = run(env.rita, &mut env, "+note Nick/watch=");
    assert!(has_text(&out, "No such note."));

    // A character with no notes.
    let out = run(env.bob, &mut env, "+notes");
    assert!(has_text(&out, "You have no notes."));
}

#[test]
fn note_approval_stamps_and_edit_resets() {
    let mut env = setup();
    run(env.rita, &mut env, "+note Nick/background=Origins.");
    // Players cannot approve.
    let out = run(env.nick, &mut env, "+note/approve me/background");
    assert!(has_text(&out, "Permission denied."));
    // Staff stamps it.
    let out = run(env.rita, &mut env, "+note/approve Nick/background");
    assert!(has_text(&out, "Note 'background' on Nick approved."));
    let note = get_note(&env.engine.world, env.nick, "background").unwrap();
    let stamp = note.approved.clone().expect("approved");
    assert_eq!(stamp.by, env.rita);
    assert_eq!(stamp.at, 1_000);
    // Re-approval refused; missing notes reported.
    let out = run(env.rita, &mut env, "+note/approve Nick/background");
    assert!(has_text(&out, "already approved"));
    let out = run(env.rita, &mut env, "+note/approve Nick/bogus");
    assert!(has_text(&out, "No such note."));
    // The approval marker shows in listings.
    let out = run(env.nick, &mut env, "+notes");
    assert!(has_text(&out, "[approved]"));
    // Editing the note clears the stamp and re-attributes it.
    run(env.nick, &mut env, "+note me/background=Revised origins.");
    let note = get_note(&env.engine.world, env.nick, "background").unwrap();
    assert!(note.approved.is_none(), "editing resets approval");
    assert_eq!(note.author, env.nick);
    let out = run(env.nick, &mut env, "+notes");
    assert!(has_text(&out, "[unapproved]"));
}

// ----------------------------------------------------------------------
// Persistence
// ----------------------------------------------------------------------

#[test]
fn persistence_round_trip_across_reboot() {
    let mut env = setup();
    let mut store = FakeKv::default();
    env.engine.world.set_connected(env.nick, true);

    run_p(
        env.nick,
        &mut env,
        &mut store,
        "+request Better desc=Please.",
    );
    run_p(env.nick, &mut env, &mut store, "+xspend Dot=xp.apply");
    run_p(env.rita, &mut env, &mut store, "+job/comment 1=On it.");
    run_p(env.rita, &mut env, &mut store, "+job/assign 1=Rita");
    run_p(env.wanda, &mut env, &mut store, "+job/approve 2=Applied.");
    run_p(
        env.rita,
        &mut env,
        &mut store,
        "+note Nick/background=Origins.",
    );
    run_p(
        env.rita,
        &mut env,
        &mut store,
        "+note/staff Nick/watch=Eyes on.",
    );
    run_p(
        env.rita,
        &mut env,
        &mut store,
        "+note/approve Nick/background",
    );
    run_p(env.rita, &mut env, &mut store, "+note Nick/temp=scratch");
    run_p(env.rita, &mut env, &mut store, "+note Nick/temp=");

    let job1 = get_job(&env.engine.world, 1).unwrap().clone();
    let job2 = get_job(&env.engine.world, 2).unwrap().clone();

    // Reboot: same players, fresh world, boot data from the store.
    let mut env2 = setup_with(store.boot());
    assert_eq!(get_job(&env2.engine.world, 1), Some(&job1));
    assert_eq!(get_job(&env2.engine.world, 2), Some(&job2));
    let note = get_note(&env2.engine.world, env2.nick, "background").unwrap();
    assert_eq!(note.text, "Origins.");
    assert_eq!(note.approved.as_ref().map(|a| a.by), Some(env2.rita));
    assert!(
        get_note(&env2.engine.world, env2.nick, "watch")
            .unwrap()
            .staff_only
    );
    assert!(get_note(&env2.engine.world, env2.nick, "temp").is_none());
    // The payload survived too (un-approved jobs keep theirs).
    assert_eq!(job2.payload.as_ref().unwrap().handler, "xp.apply");
    // Listings still work post-boot.
    let out = run(env2.wanda, &mut env2, "+jobs");
    assert!(has_text(&out, "Better desc") && has_text(&out, "Dot"));
    // Id sequencing continues.
    let out = run(env2.bob, &mut env2, "+request Third=More.");
    assert!(has_text(&out, "job 3"), "{:?}", sys_texts(&out));
}

#[test]
fn id_sequencing_from_meta_or_job_docs() {
    // The meta doc alone drives the counter...
    let boot = HashMap::from([(
        SmolStr::new("jobs"),
        ModuleBootData {
            kv: vec![(SmolStr::new("meta"), json!({"next_id": 7}))],
            streams: HashMap::new(),
            ..Default::default()
        },
    )]);
    let mut env = setup_with(boot);
    let out = run(env.nick, &mut env, "+request T=t.");
    assert!(has_text(&out, "job 7"), "{:?}", sys_texts(&out));

    // ...and with the meta doc missing, the max persisted job id + 1 wins.
    let job = json!({
        "id": 4, "queue": "REQ", "title": "Old", "opener": 2,
        "status": "Open", "opened_at": 1, "updated_at": 1
    });
    let boot = HashMap::from([(
        SmolStr::new("jobs"),
        ModuleBootData {
            kv: vec![(SmolStr::new("job:4"), job)],
            streams: HashMap::new(),
            ..Default::default()
        },
    )]);
    let mut env = setup_with(boot);
    let out = run(env.nick, &mut env, "+request T=t.");
    assert!(has_text(&out, "job 5"), "{:?}", sys_texts(&out));
}

/// Corrupt or unknown documents must never brick the boot: one bit-rotted
/// job doc (or a key written by a newer build before a rollback) is
/// quarantined — logged, skipped, left untouched in the store — while every
/// healthy document still loads. Strictness belongs in staff tooling, not
/// the boot path. (Contrast: comsys FAILS boot on a corrupt channel doc,
/// because proceeding there would destructively delete memberships;
/// skipping a job doc destroys nothing.)
#[test]
fn boot_tolerates_corrupt_and_unknown_documents() {
    let good = json!({
        "id": 4, "queue": "REQ", "title": "Old", "opener": 2,
        "status": "Open", "opened_at": 1, "updated_at": 1
    });
    let kv = vec![
        (SmolStr::new("job:1"), json!({"garbage": true})), // corrupt: skipped
        (SmolStr::new("job:4"), good),                     // healthy: loaded
        (
            SmolStr::new("job:max"),
            json!({
                "id": u64::MAX, "queue": "REQ", "title": "Exhausted", "opener": 2,
                "status": "Open", "opened_at": 1, "updated_at": 1
            }),
        ), // valid shape, impossible successor: quarantined
        (SmolStr::new("note:1:x"), json!(42)),             // corrupt: skipped
        (SmolStr::new("meta"), json!({"wrong": 1})),       // bad meta: self-heals
        (SmolStr::new("mystery:key"), json!({})),          // unknown: ignored
    ];
    let world = World::new_minimal();
    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    let mut engine =
        Engine::with_modules(world, registry, vec![Box::new(JobsModule::new())]).unwrap();
    let boot = HashMap::from([(
        SmolStr::new("jobs"),
        ModuleBootData {
            kv,
            streams: HashMap::new(),
            ..Default::default()
        },
    )]);
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine
        .modules_loaded(boot, &mut out, &mut fx)
        .expect("corrupt docs must not brick the boot");
    // The healthy job loaded; the corrupt one did not; the id counter
    // self-healed past the loaded job despite the bad meta doc.
    assert!(mush_jobs::get_job(&engine.world, 4).is_some());
    assert!(mush_jobs::get_job(&engine.world, 1).is_none());
}

// ----------------------------------------------------------------------
// Registration surface
// ----------------------------------------------------------------------

#[test]
fn commands_queues_and_help_are_registered() {
    let env = setup();
    let reg = &env.engine.registry;
    for name in ["+request", "+jobs", "+job", "+note", "+notes"] {
        let spec = reg
            .spec_of(name)
            .unwrap_or_else(|| panic!("{name} registered"));
        assert!(!spec.help.is_empty(), "{name} has help text");
    }
    let req = reg
        .job_queues()
        .iter()
        .find(|q| q.name == "REQ")
        .expect("REQ queue");
    assert_eq!(req.min_role, Role::Royalty);
    assert!(reg.job_queues().iter().any(|q| q.name == "xp"));
}

// ----------------------------------------------------------------------
// Review-fix regressions: half-committed payloads, one-shot payload jobs,
// the durable-ack gate
// ----------------------------------------------------------------------

/// A payload handler that queues `ctx.persist` writes and THEN fails must
/// not half-commit them: dispatch truncates the deferred queue on `Err`,
/// so `take_module_ops` is empty and the job stays Open.
#[test]
fn failed_payload_handler_discards_queued_persist_ops() {
    let mut env = setup();
    run(env.nick, &mut env, "+xspend Half spend=xp.half");
    let _ = env.engine.take_module_ops();
    let out = run(env.wanda, &mut env, "+job/approve 1");
    assert!(
        has_text(&out, "second doc failed."),
        "{:?}",
        sys_texts(&out)
    );
    assert!(has_text(&out, "Job 1 unchanged."));
    assert_eq!(
        get_job(&env.engine.world, 1).unwrap().status,
        JobStatus::Open
    );
    assert!(
        env.engine.take_module_ops().is_empty(),
        "ops queued by the failed handler must never reach the WriteBatch"
    );
}

/// Payload jobs are one-shot: once terminal (the payload may have been
/// APPLIED), `/status` may not reopen them — a reopened payload job could
/// be `/approve`d again and run the handler twice (double XP spend).
/// Payload-free jobs stay freely re-statusable (covered elsewhere).
#[test]
fn terminal_payload_jobs_cannot_be_reopened() {
    let mut env = setup();
    run(env.nick, &mut env, "+xspend Buy a dot=xp.apply");
    run(env.wanda, &mut env, "+job/approve 1");
    assert_eq!(env.applied.load(Ordering::SeqCst), 1);
    for target in ["open", "inprogress"] {
        let out = run(env.wanda, &mut env, &format!("+job/status 1={target}"));
        assert!(
            has_text(&out, "payload jobs cannot be reopened"),
            "{target}: {:?}",
            sys_texts(&out)
        );
    }
    assert_eq!(
        get_job(&env.engine.world, 1).unwrap().status,
        JobStatus::Resolved
    );
    // E4c verify fold (E3): terminal-to-terminal is NOT fine either. A payload
    // job's `Resolved` is the economy's `jobs.resolved` fact, so moving off it
    // would take back an answer a contract may already have settled on.
    let out = run(env.wanda, &mut env, "+job/status 1=denied");
    assert!(
        has_text(&out, "cannot be moved"),
        "a resolved payload job was re-statused: {:?}",
        sys_texts(&out)
    );
    // …and `=resolved` never resolves one: only `/approve` runs the payload.
    run(env.nick, &mut env, "+xspend Buy another dot=xp.apply");
    let out = run(env.wanda, &mut env, "+job/status 2=resolved");
    assert!(
        has_text(&out, "resolved by +job/approve"),
        "a payload job was resolved by hand: {:?}",
        sys_texts(&out)
    );
    assert_eq!(
        get_job(&env.engine.world, 2).unwrap().payload_applied,
        None,
        "a status flip wrote the applied marker"
    );
    // And no second application ever happened.
    assert_eq!(env.applied.load(Ordering::SeqCst), 1);
}

/// A successful approval requests the durable-ack output gate (PLAN: XP
/// approval gates its confirmation on the store ack); a failed one gates
/// nothing.
#[test]
fn approve_requests_the_durable_ack_gate() {
    let mut env = setup();
    run(env.nick, &mut env, "+xspend Buy a dot=xp.apply");
    let _ = env.engine.take_durable_gate();
    run(env.wanda, &mut env, "+job/approve 1");
    assert_eq!(env.engine.take_durable_gate(), Some((env.wanda, false)));

    let mut env = setup();
    run(env.nick, &mut env, "+xspend Bad spend=xp.fail");
    let _ = env.engine.take_durable_gate();
    run(env.wanda, &mut env, "+job/approve 1");
    assert_eq!(env.engine.take_durable_gate(), None);
}

// ----------------------------------------------------------------------
// ---- look-back fold: D-LBK-6 ----
// ----------------------------------------------------------------------
//
// `set_job_status` left the public API: it took any `JobStatus` and ran no payload
// handler, so a module could flip a payload-bearing job to `Resolved` without
// applying it — the exact class E4c closed when it made `jobs.resolved` a fact
// answered from the applied-payload marker. `resolve_plain_job` / `deny_plain_job`
// are the only external terminal path: a payload is refused, the caller's queue is
// checked against the job's, and a closed job is refused.

/// A payload-bearing job is refused by the plain-job seam whatever queue is named,
/// and its applied marker stays unset.
#[test]
fn lb_dlbk6_a_payload_job_is_refused_by_the_plain_seam() {
    let mut env = setup();
    run(env.nick, &mut env, "+xspend Buy a dot=xp.apply");
    let out = run(env.wanda, &mut env, "+plainclose 1=xp resolve");
    assert!(
        has_text(&out, "carries a payload"),
        "a payload job was resolved without its handler: {:?}",
        sys_texts(&out)
    );
    let job = get_job(&env.engine.world, 1).unwrap();
    assert_eq!(
        job.status,
        JobStatus::Open,
        "the refused close moved the status"
    );
    assert_eq!(
        job.payload_applied, None,
        "the refused close wrote the applied marker"
    );
    assert_eq!(env.applied.load(Ordering::SeqCst), 0, "a handler ran");
    // The deny half refuses the same job the same way.
    let out = run(env.wanda, &mut env, "+plainclose 1=xp deny");
    assert!(has_text(&out, "carries a payload"), "{:?}", sys_texts(&out));
}

/// Producer identity is checked, not assumed: a job from another queue is refused
/// by name, and the right queue name is accepted case-insensitively.
#[test]
fn lb_dlbk6_a_wrong_queue_job_is_refused() {
    let mut env = setup();
    run(env.nick, &mut env, "+request Fix my desc=it is wrong");
    let out = run(env.wanda, &mut env, "+plainclose 1=xp resolve");
    assert!(
        has_text(&out, "lives in queue \"REQ\", not the caller's \"xp\""),
        "wrong refusal: {:?}",
        sys_texts(&out)
    );
    assert_eq!(
        get_job(&env.engine.world, 1).unwrap().status,
        JobStatus::Open
    );
    let out = run(env.wanda, &mut env, "+plainclose 1=req resolve");
    assert!(has_text(&out, "plain close ok"), "{:?}", sys_texts(&out));
    assert_eq!(
        get_job(&env.engine.world, 1).unwrap().status,
        JobStatus::Resolved
    );
}

/// A plain job in the caller's own queue resolves (and another denies), the closing
/// note is recorded as a comment, and a closed job is refused a second time.
#[test]
fn lb_dlbk6_a_plain_job_resolves_or_denies_once() {
    let mut env = setup();
    let mut store = FakeKv::default();
    run_p(env.nick, &mut env, &mut store, "+request Story one=please");
    run_p(env.nick, &mut env, &mut store, "+request Story two=please");
    let out = run_p(env.wanda, &mut env, &mut store, "+plainclose 1=REQ resolve");
    assert!(has_text(&out, "plain close ok"), "{:?}", sys_texts(&out));
    let out = run_p(env.wanda, &mut env, &mut store, "+plainclose 2=REQ deny");
    assert!(has_text(&out, "plain close ok"), "{:?}", sys_texts(&out));
    let one = get_job(&env.engine.world, 1).unwrap().clone();
    let two = get_job(&env.engine.world, 2).unwrap().clone();
    assert_eq!(one.status, JobStatus::Resolved);
    assert_eq!(two.status, JobStatus::Denied);
    assert!(
        one.comments
            .iter()
            .any(|c| c.text == "answered by the module"),
        "{:?}",
        one.comments
    );
    assert!(
        two.comments
            .iter()
            .any(|c| c.text == "withdrawn by the module"),
        "{:?}",
        two.comments
    );
    // Closed is closed: a retrying module writes nothing.
    let out = run_p(env.wanda, &mut env, &mut store, "+plainclose 1=REQ resolve");
    assert!(
        has_text(&out, "is already Resolved"),
        "{:?}",
        sys_texts(&out)
    );
    // …and the write-through carries both closes.
    let doc = store.0.get("job:1").expect("job:1 persisted");
    assert_eq!(doc["status"], serde_json::json!("Resolved"));
    let doc = store.0.get("job:2").expect("job:2 persisted");
    assert_eq!(doc["status"], serde_json::json!("Denied"));
    // The error type is a value a module can match on, not only a string.
    assert_eq!(
        PlainJobError::NotOpen {
            job: 1,
            status: JobStatus::Resolved
        }
        .to_string(),
        "job 1 is already Resolved"
    );
}

/// The jobs `FactProvider` still answers only from the applied-payload marker: a
/// plain job resolved through the seam is **unknown** to `jobs.resolved`, exactly as
/// a plain job resolved by hand always was (one assertion, unchanged by the fold).
#[test]
fn lb_dlbk6_the_fact_provider_still_answers_from_the_applied_marker() {
    use mush_economy::FactProvider;
    let mut env = setup();
    run(env.nick, &mut env, "+request Story=please");
    run(env.wanda, &mut env, "+plainclose 1=REQ resolve");
    assert_eq!(
        get_job(&env.engine.world, 1).unwrap().status,
        JobStatus::Resolved
    );
    assert!(
        mush_jobs::JobsFactProvider
            .resolve(&env.engine.world, "resolved", "1", None)
            .is_none(),
        "a plain job's resolution became a settlement fact"
    );
}

/// Programme D5: filing a request also tells the staff relay sink — the
/// Discord staff channel, when the operator mapped one — once, with the job
/// number, the opener and the title, and the command staff open it with.
#[test]
fn d5_a_request_is_announced_on_the_staff_relay_sink() {
    let mut env = setup();
    let out = run(
        env.nick,
        &mut env,
        "+request A door is stuck=It will not open.",
    );
    let staff: Vec<(String, String, String, String)> = out
        .events()
        .iter()
        .filter_map(|(aud, e)| match (aud, e) {
            (
                mush_core::Audience::Relay(name),
                OutputEvent::Notice {
                    category,
                    text,
                    action,
                    ..
                },
            ) => Some((
                name.to_string(),
                category.to_string(),
                text.to_plain_text(),
                action
                    .as_ref()
                    .map(|a| a.prefill.clone())
                    .unwrap_or_default(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(staff.len(), 1, "{:?}", out.events());
    let (sink, category, text, prefill) = &staff[0];
    assert_eq!(sink, mush_core::output::STAFF_RELAY_SINK);
    assert_eq!(category, "request");
    assert!(text.starts_with("+request #"), "{text}");
    assert!(text.ends_with("from Nick: A door is stuck"), "{text}");
    assert!(prefill.starts_with("+job "), "{prefill}");
    // The opener's own confirmation is unchanged.
    assert!(
        sys_texts(&out)
            .iter()
            .any(|t| t.contains("Request filed as job"))
    );
}

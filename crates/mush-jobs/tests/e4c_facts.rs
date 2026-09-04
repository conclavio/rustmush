//! E4c (08c §6.1 item 1): `mush-jobs` answers the economy's external-fact seam.
//!
//! The rule under test is the one that makes `jobs.resolved` a **fact** rather than a
//! **state**: a payload-bearing job is one-shot (`+job/status` refuses to reopen it,
//! because its handler has already run), so its resolution cannot be taken back and
//! a contract may settle on it. A payload-less job can be reopened at any time, so
//! this provider answers *unknown* for one rather than pretending it is terminal.

use std::collections::HashMap;
use std::sync::Arc;

use mush_core::engine::{Effects, Engine, Inbound};
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, Flags, GOD, GameModule, JobQueueSpec, LIMBO, ModError,
    ObjId, ObjKind, Outbox, Registrar, Registry, Role, World, register_core_commands,
    register_engine_commands,
};
use mush_economy::FactProvider;
use mush_jobs::{Job, JobPayload, JobsFactProvider, JobsModule, OpenJob, open_job, register_payload_handler};
use serde_json::json;

/// A producing module: an `xp` queue, a `+xspend` command that files a
/// payload-carrying job, and the handler `+job/approve` runs.
struct TestProducer;

impl GameModule for TestProducer {
    fn name(&self) -> &'static str {
        "wod"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.job_queue(JobQueueSpec {
            name: "xp".into(),
            title: "XP spends".into(),
            min_role: Role::Wizard,
        })?;
        reg.command(
            CommandSpec {
                name: "+xspend".into(),
                shape: ArgShape::EqPair,
                help: "+xspend <title>= -- test: file a payload job".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let title = ctx.args.lhs().to_string();
                open_job(ctx, OpenJob {
                    queue: "xp".into(),
                    title,
                    opener: ctx.actor,
                    text: None,
                    payload: Some(JobPayload { handler: "xp.apply".into(), data: json!({}) }),
                    tags: Vec::new(),
                })
                .map(|_| ())
                .map_err(CmdError::msg)
            }),
        )?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut mush_core::ModuleCtx<'_>,
        _boot: mush_core::ModuleBootData,
    ) -> Result<(), ModError> {
        register_payload_handler(
            ctx.w,
            "xp.apply",
            Arc::new(|_: &mut Ctx<'_>, _: &Job| Ok(())),
        )
        .map_err(ModError::msg)
    }
}

struct Env {
    engine: Engine,
    nick: ObjId,
    wanda: ObjId,
}

fn setup() -> Env {
    let mut world = World::new_minimal();
    world.set_now(1_000);
    let nick = world.create(ObjKind::Player, "Nick", GOD, Some(LIMBO)).unwrap();
    let wanda = world.create(ObjKind::Player, "Wanda", GOD, Some(LIMBO)).unwrap();
    world.set_flag(wanda, Flags::WIZARD, true).unwrap();
    let mut registry = Registry::new();
    register_core_commands(&mut registry).unwrap();
    register_engine_commands(&mut registry).unwrap();
    let mut engine = Engine::with_modules(world, registry, vec![
        Box::new(JobsModule::new()),
        Box::new(TestProducer),
    ])
    .unwrap();
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    engine.modules_loaded(HashMap::new(), &mut out, &mut fx).unwrap();
    Env { engine, nick, wanda }
}

fn run(actor: ObjId, env: &mut Env, line: &str) {
    let (mut out, mut fx) = (Outbox::new(), Effects::new());
    env.engine.step(Inbound::Cmd { conn: 1, actor, line: line.into() }, &mut out, &mut fx);
}

#[test]
fn only_a_payload_bearing_resolved_job_is_a_fact() {
    let mut env = setup();
    // 1: a payload job. 2: a plain request, which staff may reopen at any time.
    run(env.nick, &mut env, "+xspend Raise Strength=");
    run(env.nick, &mut env, "+request A door is stuck=It will not open.");
    run(env.wanda, &mut env, "+job/assign 1=Wanda");
    run(env.wanda, &mut env, "+job/approve 1=done");
    run(env.wanda, &mut env, "+job/approve 2=done");

    let p = JobsFactProvider;
    let w = &env.engine.world;

    // The payload job: a terminal fact, and the provider says who closed it.
    let fact = p.resolve(w, "resolved", "1", None).expect("a payload job answers");
    assert!(fact.satisfied, "an approved payload job did not answer satisfied");
    assert_eq!(fact.actor, Some(env.wanda), "the provider lost the approver");

    // The payload-less job: **unknown**, not `Some(false)` — the distinction is
    // "not a fact I can vouch for", because `+job/status` can reopen it.
    assert!(
        p.resolve(w, "resolved", "2", None).is_none(),
        "a payload-less job was treated as a terminal fact"
    );
    // Reopening it is exactly why: the same command that would have unsettled a
    // contract still works.
    run(env.wanda, &mut env, "+job/status 2=open");
    assert_eq!(
        mush_jobs::get_job(&env.engine.world, 2).expect("job 2").status,
        mush_jobs::JobStatus::Open,
        "the payload-less job could not be reopened, which was the whole premise"
    );
}

#[test]
fn an_unopened_job_an_unknown_kind_and_a_versioned_reference_are_all_unknown() {
    let mut env = setup();
    run(env.nick, &mut env, "+xspend Raise Strength=");
    let p = JobsFactProvider;
    let w = &env.engine.world;

    assert_eq!(p.provider(), "jobs");
    // Open, not resolved: a fact the provider CAN vouch for, and it is false.
    let fact = p.resolve(w, "resolved", "1", None).expect("an open payload job answers");
    assert!(!fact.satisfied, "an unapproved job answered satisfied");
    // Everything the provider cannot speak to is `None`, which is unsatisfied.
    assert!(p.resolve(w, "resolved", "99", None).is_none(), "a nonexistent job answered");
    assert!(p.resolve(w, "resolved", "not-a-number", None).is_none(), "a junk id answered");
    assert!(p.resolve(w, "denied", "1", None).is_none(), "an unknown kind answered");
    assert!(
        p.resolve(w, "resolved", "1", Some(3)).is_none(),
        "a reference claiming a revision was answered by a provider that has none"
    );
}

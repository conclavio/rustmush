use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

use crate::{
    ArgShape, CommandCompatibility, CommandSpec, Engine, GOD, GameModule, Inbound, ModError,
    Outbox, Registrar, Registry, ScheduledKind,
    cmd::CommandGate,
    engine::Effects,
    flags::{Powers, Privileges, Role},
    lock::LockExpr,
    mechanics::{CanonicalId, IncidentId, RecoveryReason},
    module::{ContextOutcome, InputContext},
};
use mush_proto::{InputMode, PromptSpec};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
const ACTOR: ObjId = ObjId(2);

#[derive(Default)]
struct Shared {
    handle: Mutex<Option<ContinuationHandle>>,
    /// 17g C2h: the grantee module's own continuation (`@relay/go`).
    relay: Mutex<Option<ContinuationHandle>>,
    calls: Mutex<Vec<(ObjId, String, ObjId, String)>>,
    denied: AtomicBool,
    freeze_in_gate: AtomicBool,
}
fn handle(shared: &Shared) -> Result<ContinuationHandle, CmdError> {
    shared
        .handle
        .lock()
        .map_err(|_| CmdError::msg("fixture mutex"))?
        .clone()
        .ok_or_else(|| CmdError::msg("registered handle"))
}
struct Gate(Arc<Shared>);
impl CommandGate for Gate {
    fn check(
        &self,
        _: &World,
        reg: &Registry,
        _: ObjId,
        verb: &str,
        _: &str,
    ) -> Result<(), String> {
        if verb == "@resume" {
            if self.0.freeze_in_gate.load(Ordering::Acquire) {
                reg.freeze_mechanics_owner(
                    &OwnerId::new("owner").map_err(|e| e.to_string())?,
                    IncidentId(1),
                    RecoveryReason::IntegrityViolation {
                        code: CanonicalId::new("fixture").map_err(|e| e.to_string())?,
                    },
                )
                .map_err(|e| e.to_string())?;
            }
            if self.0.denied.load(Ordering::Acquire) {
                return Err("fixture command gate".into());
            }
        }
        Ok(())
    }
}
struct Module {
    shared: Arc<Shared>,
    spec: CommandSpec,
    registration: Option<&'static str>,
    /// 17g C2h: the module this owner delegates its continuation to.
    delegate: Option<&'static str>,
}
impl GameModule for Module {
    fn name(&self) -> &'static str {
        "owner"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(if self.registration == Some("readonly") {
            CommandCompatibility::ReadOnly
        } else {
            CommandCompatibility::Authoritative
        });
        let shared = Arc::clone(&self.shared);
        reg.command(
            self.spec.clone(),
            Arc::new(move |ctx: &mut Ctx<'_>| {
                if !ctx.switches.has("finish") {
                    return Err(CmdError::msg("fixed switch"));
                }
                let stamp = ctx
                    .typed_state
                    .dispatch_caller()
                    .ok_or_else(|| CmdError::msg("dispatch stamp"))?;
                shared
                    .calls
                    .lock()
                    .map_err(|_| CmdError::msg("fixture mutex"))?
                    .push((
                        ctx.actor,
                        stamp.module().as_str().to_string(),
                        stamp.actor(),
                        ctx.args.raw().into(),
                    ));
                if ctx.args.raw() == "error" {
                    ctx.out.defer(crate::Deferred::ScheduleIn {
                        delay_ms: 0,
                        kind: ScheduledKind::Control("must_rollback".into()),
                    });
                    return Err(CmdError::msg("fixture handler refusal"));
                }
                Ok(())
            }),
        )?;
        let primary = self.registration.unwrap_or("@resume");
        let switches: &[&str] = match primary {
            "bad_switch" => &["unknown"],
            "duplicate_switch" => &["finish", "FINISH"],
            _ => &["finish"],
        };
        let primary = if matches!(primary, "bad_switch" | "duplicate_switch" | "readonly") {
            "@resume"
        } else {
            primary
        };
        let captured = reg.command_continuation(primary, switches)?;
        if let Some(grantee) = self.delegate {
            reg.delegate_continuation(&captured, grantee, "fixture: the grantee's front")?;
        }
        *self
            .shared
            .handle
            .lock()
            .map_err(|_| ModError::msg("fixture mutex"))? = Some(captured);
        reg.command_gate(Arc::new(Gate(Arc::clone(&self.shared))));
        let shared = Arc::clone(&self.shared);
        reg.command(
            CommandSpec {
                name: "@enqueue".into(),
                shape: ArgShape::Raw,
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| {
                let argument = ctx.args.raw().to_string();
                ctx.schedule_continuation(&handle(&shared)?, ACTOR, &argument, 10)?;
                if argument == "fail" {
                    return Err(CmdError::msg("scheduling handler refusal"));
                }
                Ok(())
            }),
        )?;
        reg.command(
            CommandSpec {
                name: "@ordinary".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| {
                ctx.out.defer(crate::Deferred::ScheduleIn {
                    delay_ms: 10,
                    kind: ScheduledKind::WaitCmd {
                        actor: ACTOR,
                        line: "@resume/finish ordinary".into(),
                    },
                });
                Ok(())
            }),
        )?;
        Ok(())
    }
}
struct Foreign(Arc<Shared>);
impl GameModule for Foreign {
    fn name(&self) -> &'static str {
        "foreign"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        assert!(reg.command_continuation("@resume", &["finish"]).is_err());
        // 17g C2h: only the handle's owner may delegate it.
        if let Ok(owned) = handle(&self.0) {
            assert!(
                reg.delegate_continuation(&owned, "foreign", "not the owner")
                    .is_err()
            );
        }
        reg.classify_commands(CommandCompatibility::Authoritative);
        // 17g C2h: the grantee's own commands. `@front self` schedules the
        // owner's handle for the dispatching actor, `@front other` for
        // another live player; `@queue_front` queues `@front self` as a
        // `@wait` line does; `@relay` schedules this module's own
        // continuation, whose handler then tries the owner's handle.
        let shared = Arc::clone(&self.0);
        reg.command(
            CommandSpec {
                name: "@front".into(),
                shape: ArgShape::Raw,
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| {
                let target = if ctx.args.raw() == "other" {
                    if ctx.actor == ACTOR { GOD } else { ACTOR }
                } else {
                    ctx.actor
                };
                ctx.schedule_continuation(&handle(&shared)?, target, "delegated", 0)
            }),
        )?;
        reg.command(
            CommandSpec {
                name: "@queue_front".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| {
                ctx.out.defer(crate::Deferred::ScheduleIn {
                    delay_ms: 10,
                    kind: ScheduledKind::WaitCmd {
                        actor: ctx.actor,
                        line: "@front self".into(),
                    },
                });
                Ok(())
            }),
        )?;
        let shared = Arc::clone(&self.0);
        reg.command(
            CommandSpec {
                name: "@relay".into(),
                switches: vec!["go".into()],
                shape: ArgShape::Raw,
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| {
                if ctx.switches.has("go") {
                    return ctx.schedule_continuation(
                        &handle(&shared)?,
                        ctx.actor,
                        "from_a_continuation",
                        0,
                    );
                }
                let relay = shared
                    .relay
                    .lock()
                    .map_err(|_| CmdError::msg("fixture mutex"))?
                    .clone()
                    .ok_or_else(|| CmdError::msg("relay handle"))?;
                ctx.schedule_continuation(&relay, ctx.actor, "", 0)
            }),
        )?;
        let relay = reg.command_continuation("@relay", &["go"])?;
        *self
            .0
            .relay
            .lock()
            .map_err(|_| ModError::msg("fixture mutex"))? = Some(relay);
        let shared = Arc::clone(&self.0);
        reg.command(
            CommandSpec {
                name: "@spoof".into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| {
                ctx.module = Some("owner".into());
                ctx.actor = GOD;
                ctx.schedule_continuation(&handle(&shared)?, ACTOR, "spoof", 0)
            }),
        )
    }
}
fn spec() -> CommandSpec {
    CommandSpec {
        name: "@resume".into(),
        aliases: vec!["@alias".into()],
        switches: vec!["finish".into()],
        shape: ArgShape::Raw,
        ..CommandSpec::default()
    }
}
#[inline(never)]
fn engine(
    spec: CommandSpec,
    registration: Option<&'static str>,
) -> TestResult<(Box<Engine>, Arc<Shared>)> {
    engine_with(spec, registration, None)
}
/// `engine`, with the owner delegating its continuation to `delegate`
/// (17g C2h).
#[inline(never)]
fn engine_with(
    spec: CommandSpec,
    registration: Option<&'static str>,
    delegate: Option<&'static str>,
) -> TestResult<(Box<Engine>, Arc<Shared>)> {
    let mut world = World::new_minimal();
    assert_eq!(
        world.create(crate::ObjKind::Player, "Subject", GOD, Some(ObjId(0)))?,
        ACTOR
    );
    world.set_now(100);
    let shared = Arc::new(Shared::default());
    let engine = Engine::with_modules(
        world,
        Registry::new(),
        vec![
            Box::new(Module {
                shared: Arc::clone(&shared),
                spec,
                registration,
                delegate,
            }),
            Box::new(Foreign(Arc::clone(&shared))),
        ],
    )?;
    Ok((Box::new(engine), shared))
}
fn command(engine: &mut Engine, actor: ObjId, text: &str) -> Outbox {
    let mut out = Outbox::new();
    engine.step(
        Inbound::Cmd {
            conn: 0,
            actor,
            line: text.into(),
        },
        &mut out,
        &mut Effects::new(),
    );
    out
}
fn fire(engine: &mut Engine) -> Outbox {
    let mut out = Outbox::new();
    engine.fire_due(100_010, &mut out, &mut Effects::new());
    out
}
struct Editor(Arc<Mutex<Vec<String>>>);
impl InputContext for Editor {
    fn prompt(&self, _: &World) -> PromptSpec {
        PromptSpec {
            mode: InputMode::Normal,
            text: None,
        }
    }
    fn feed(&mut self, _: &mut Ctx<'_>, line: &str) -> ContextOutcome {
        if let Ok(mut lines) = self.0.lock() {
            lines.push(line.into());
        }
        ContextOutcome::Consumed
    }
}

#[test]
fn timer_uses_exact_handler_and_actual_actor_without_touching_editor() -> TestResult {
    let (mut engine, shared) = engine(spec(), None)?;
    command(&mut engine, GOD, "@enqueue private_ticket");
    command(&mut engine, GOD, "@ordinary");
    let input = Arc::new(Mutex::new(Vec::new()));
    engine.world.push_input_context(
        ACTOR,
        Some("owner".into()),
        Box::new(Editor(Arc::clone(&input))),
    );
    fire(&mut engine);
    assert_eq!(
        *shared.calls.lock().map_err(|_| "calls")?,
        vec![(ACTOR, "owner".into(), ACTOR, "private_ticket".into())]
    );
    assert_eq!(
        *input.lock().map_err(|_| "input")?,
        vec!["@resume/finish ordinary"]
    );
    assert_eq!(engine.world.input_depth(ACTOR), 1);
    command(&mut engine, ACTOR, "real editor input");
    assert_eq!(
        *input.lock().map_err(|_| "input")?,
        vec!["@resume/finish ordinary", "real editor input"]
    );
    Ok(())
}

#[test]
fn continuation_keeps_role_power_privilege_lock_and_command_gates() -> TestResult {
    let cases = [
        CommandSpec {
            min_role: Role::Wizard,
            ..spec()
        },
        CommandSpec {
            power: Some(Powers::TEL_ANYWHERE),
            ..spec()
        },
        CommandSpec {
            privilege: Some(Privileges::DISCORD_IDENTITY),
            ..spec()
        },
        CommandSpec {
            lock: Some(LockExpr::Const(false)),
            ..spec()
        },
    ];
    for spec in cases {
        let (mut engine, shared) = engine(spec, None)?;
        command(&mut engine, GOD, "@enqueue ticket");
        assert_eq!(engine.sched.len(), 1);
        fire(&mut engine);
        assert!(shared.calls.lock().map_err(|_| "calls")?.is_empty());
    }
    let (mut engine, shared) = engine(spec(), None)?;
    command(&mut engine, GOD, "@enqueue ticket");
    assert_eq!(engine.sched.len(), 1);
    shared.denied.store(true, Ordering::Release);
    fire(&mut engine);
    assert!(shared.calls.lock().map_err(|_| "calls")?.is_empty());
    Ok(())
}

#[test]
fn owner_freeze_during_gate_prevents_handler_entry() -> TestResult {
    let (mut engine, shared) = engine(spec(), None)?;
    command(&mut engine, GOD, "@enqueue ticket");
    shared.freeze_in_gate.store(true, Ordering::Release);
    fire(&mut engine);
    assert!(shared.calls.lock().map_err(|_| "calls")?.is_empty());
    assert!(
        engine
            .registry
            .require_mechanics_owner_ready(&OwnerId::new("owner")?)
            .is_err()
    );
    command(&mut engine, GOD, "@enqueue another");
    assert!(engine.sched.is_empty());
    Ok(())
}

#[test]
fn foreign_owner_public_context_mutation_and_foreign_registry_cannot_schedule() -> TestResult {
    let (mut first, shared) = engine(spec(), None)?;
    command(&mut first, ACTOR, "@spoof");
    assert!(first.sched.is_empty());
    let (_, other) = engine(spec(), None)?;
    *shared.handle.lock().map_err(|_| "handle")? = Some(handle(&other)?);
    command(&mut first, GOD, "@enqueue wrong_registry");
    assert!(first.sched.is_empty());
    assert!(shared.calls.lock().map_err(|_| "calls")?.is_empty());
    Ok(())
}

#[test]
fn queued_request_from_another_composition_cannot_fire() -> TestResult {
    let (mut first, source) = engine(spec(), None)?;
    command(&mut first, GOD, "@enqueue private_ticket");
    let request = first
        .sched
        .pending_entries()
        .entries()
        .first()
        .map(|(_, kind)| (*kind).clone())
        .ok_or("queued continuation")?;
    assert!(matches!(request, ScheduledKind::CommandContinuation(_)));
    let (mut other, destination) = engine(spec(), None)?;
    other.sched.insert(100_010, request);
    fire(&mut other);
    assert!(destination.calls.lock().map_err(|_| "calls")?.is_empty());
    assert!(source.calls.lock().map_err(|_| "calls")?.is_empty());
    fire(&mut first);
    assert_eq!(source.calls.lock().map_err(|_| "calls")?.len(), 1);
    Ok(())
}

#[test]
fn registration_is_exact_primary_and_fixed_valid_switches_only() -> TestResult {
    for primary in [
        "@alias",
        "@res",
        "@resume/finish",
        "bad_switch",
        "duplicate_switch",
        "readonly",
    ] {
        assert!(engine(spec(), Some(primary)).is_err());
    }
    let mut registry = Registry::new();
    let mut registrar = Registrar::new(&mut registry, "owner");
    registrar.classify_commands(CommandCompatibility::Authoritative);
    registrar.command(spec(), Arc::new(|_: &mut Ctx<'_>| Ok(())))?;
    assert!(
        registrar
            .command_continuation("@resume", &["finish"])
            .is_err(),
        "public registrar cannot mint this capability"
    );
    Ok(())
}

#[test]
fn queues_share_wait_limit_release_it_and_roll_back_failed_handlers() -> TestResult {
    let (mut engine, shared) = engine(spec(), None)?;
    // The existing wait queue allows 100 entries. Both paths consume one slot.
    for i in 0..100 {
        command(
            &mut engine,
            GOD,
            if i % 2 == 0 {
                "@enqueue ticket"
            } else {
                "@ordinary"
            },
        );
    }
    assert_eq!(engine.sched.len(), 100);
    command(&mut engine, GOD, "@enqueue excess");
    command(&mut engine, GOD, "@ordinary");
    assert_eq!(engine.sched.len(), 100);
    fire(&mut engine);
    assert!(engine.sched.is_empty());
    assert_eq!(shared.calls.lock().map_err(|_| "calls")?.len(), 100);
    command(&mut engine, GOD, "@enqueue fail");
    assert!(engine.sched.is_empty());
    command(&mut engine, GOD, "@enqueue error");
    assert_eq!(engine.sched.len(), 1);
    fire(&mut engine);
    assert!(
        engine.sched.is_empty(),
        "timer handler error rolls back its deferred control work"
    );
    command(&mut engine, GOD, "@enqueue after");
    assert_eq!(
        engine.sched.len(),
        1,
        "fired continuations release the same wait counter"
    );
    Ok(())
}

#[test]
fn bounded_arguments_and_removed_going_or_nonplayer_actor_refuse() -> TestResult {
    let (mut e, shared) = engine(spec(), None)?;
    command(
        &mut e,
        GOD,
        &format!("@enqueue {}", "x".repeat(crate::MAX_INGRESS_LINE_BYTES)),
    );
    assert!(e.sched.is_empty());
    for disposition in 0..3 {
        let (mut engine, shared) = engine(spec(), None)?;
        command(&mut engine, GOD, "@enqueue ticket");
        match disposition {
            0 => {
                engine
                    .world
                    .get_mut(ACTOR)
                    .ok_or("actor")?
                    .flags
                    .insert(crate::Flags::GOING);
            }
            1 => {
                engine.world.get_mut(ACTOR).ok_or("actor")?.kind = crate::ObjKind::Thing;
            }
            _ => {
                engine.world.destroy(ACTOR)?;
                engine.world.sweep();
            }
        }
        fire(&mut engine);
        assert!(shared.calls.lock().map_err(|_| "calls")?.is_empty());
        assert!(engine.sched.is_empty());
    }
    assert!(shared.calls.lock().map_err(|_| "calls")?.is_empty());
    Ok(())
}

/// Every plain line an outbox holds (system and error lines).
fn lines(out: &Outbox) -> Vec<String> {
    out.events()
        .iter()
        .filter_map(|(_, event)| match event {
            mush_proto::OutputEvent::Sys(text) => Some(text.to_plain_text()),
            _ => None,
        })
        .collect()
}

/// 17g C2h (the lead's ruling R3): the owner ("owner") delegates its exact
/// continuation to "foreign" at composition. "foreign"'s own typed command,
/// dispatched on a connection for ACTOR, schedules the owner's handle for
/// ACTOR; the handler then runs under the OWNER's identity (the dispatch
/// stamp names "owner") for ACTOR, with exactly the argument "foreign"
/// passed, after the command gates. The delegation is listed with its
/// grantee and reason, and it moves no mechanics fingerprint: the same
/// composition without it fingerprints identically (17g C2h item 4).
#[test]
fn a_delegated_continuation_schedules_from_the_grantee_for_its_own_dispatching_actor() -> TestResult
{
    let (mut delegated, shared) = engine_with(spec(), None, Some("foreign"))?;
    let listed = delegated.registry.continuation_delegations();
    assert_eq!(listed.len(), 1);
    let (command_prefix, grantee, reason) = listed.first().ok_or("one delegation")?;
    assert_eq!(command_prefix.as_str(), "@resume/finish");
    assert_eq!(grantee.as_str(), "foreign");
    assert_eq!(*reason, "fixture: the grantee's front");
    assert_eq!(
        delegated.registry.continuation_delegate(&handle(&shared)?),
        Some(&OwnerId::new("foreign")?)
    );
    let out = command(&mut delegated, ACTOR, "@front self");
    assert!(lines(&out).is_empty(), "{:?}", lines(&out));
    assert_eq!(delegated.sched.len(), 1);
    fire(&mut delegated);
    assert_eq!(
        *shared.calls.lock().map_err(|_| "calls")?,
        vec![(ACTOR, "owner".into(), ACTOR, "delegated".into())],
        "the owner's handler, for the dispatching actor"
    );
    let (plain, _) = engine(spec(), None)?;
    assert!(delegated.registry.mechanics_fingerprint().is_some());
    assert_eq!(
        delegated.registry.mechanics_fingerprint(),
        plain.registry.mechanics_fingerprint(),
        "the delegation enters no digest"
    );
    assert!(plain.registry.continuation_delegations().is_empty());
    Ok(())
}

/// 17g C2h: the delegate may schedule only for the actor whose line it is
/// dispatching. Naming another live player refuses `Continuation
/// unavailable.` and queues nothing, whichever actor dispatches.
#[test]
fn a_delegate_cannot_schedule_for_another_actor() -> TestResult {
    let (mut engine, shared) = engine_with(spec(), None, Some("foreign"))?;
    for actor in [ACTOR, GOD] {
        let out = command(&mut engine, actor, "@front other");
        assert_eq!(lines(&out), vec!["Continuation unavailable.".to_owned()]);
        assert!(engine.sched.is_empty());
    }
    fire(&mut engine);
    assert!(shared.calls.lock().map_err(|_| "calls")?.is_empty());
    Ok(())
}

/// 17g C2h: a dispatch with no connection cannot use the delegation. A
/// `@wait`-queued line (a `WaitCmd`) and the grantee's own continuation both
/// dispatch with none, and both refuse `Continuation unavailable.` with
/// nothing queued and the owner's handler never entered.
#[test]
fn a_delegate_cannot_schedule_without_a_dispatching_connection() -> TestResult {
    let (mut engine, shared) = engine_with(spec(), None, Some("foreign"))?;
    command(&mut engine, ACTOR, "@queue_front");
    assert_eq!(engine.sched.len(), 1, "the queued line");
    let out = fire(&mut engine);
    assert_eq!(lines(&out), vec!["Continuation unavailable.".to_owned()]);
    assert!(engine.sched.is_empty());
    let out = command(&mut engine, ACTOR, "@relay");
    assert!(lines(&out).is_empty(), "{:?}", lines(&out));
    assert_eq!(engine.sched.len(), 1, "the grantee's own continuation");
    let out = fire(&mut engine);
    assert_eq!(lines(&out), vec!["Continuation unavailable.".to_owned()]);
    assert!(engine.sched.is_empty());
    assert!(shared.calls.lock().map_err(|_| "calls")?.is_empty());
    Ok(())
}

/// 17g C2h: without a delegation the foreign module's typed command still
/// cannot schedule the owner's handle, for its own dispatching actor too
/// (the rule before C2h, unchanged), and a delegation in another composition
/// grants nothing here.
#[test]
fn an_undelegated_caller_still_cannot_schedule() -> TestResult {
    let (mut engine, shared) = engine(spec(), None)?;
    let out = command(&mut engine, ACTOR, "@front self");
    assert_eq!(lines(&out), vec!["Continuation unavailable.".to_owned()]);
    assert!(engine.sched.is_empty());
    assert_eq!(
        engine.registry.continuation_delegate(&handle(&shared)?),
        None
    );
    let (_, delegated) = engine_with(spec(), None, Some("foreign"))?;
    assert_eq!(
        engine.registry.continuation_delegate(&handle(&delegated)?),
        None,
        "another composition's handle has no delegate here"
    );
    *shared.handle.lock().map_err(|_| "handle")? = Some(handle(&delegated)?);
    command(&mut engine, ACTOR, "@front self");
    assert!(engine.sched.is_empty());
    fire(&mut engine);
    assert!(shared.calls.lock().map_err(|_| "calls")?.is_empty());
    Ok(())
}

/// 17g C2h's bounds, composed for real: one owner with
/// `MAX_CONTINUATION_DELEGATES + 1` continuations on one command, beside the
/// grantee "foreign".
struct Bounded(&'static str);
impl GameModule for Bounded {
    fn name(&self) -> &'static str {
        "owner"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(CommandCompatibility::Authoritative);
        let switches: Vec<String> = (0..=MAX_CONTINUATION_DELEGATES)
            .map(|i| format!("s{i}"))
            .collect();
        reg.command(
            CommandSpec {
                name: "@bounded".into(),
                switches: switches.iter().map(Into::into).collect(),
                shape: ArgShape::Raw,
                ..CommandSpec::default()
            },
            Arc::new(|_: &mut Ctx<'_>| Ok(())),
        )?;
        let mut handles = Vec::new();
        for switch in &switches {
            handles.push(reg.command_continuation("@bounded", &[switch.as_str()])?);
        }
        let first = handles.first().ok_or_else(|| ModError::msg("a handle"))?;
        match self.0 {
            "second" => {
                reg.delegate_continuation(first, "foreign", "first")?;
                reg.delegate_continuation(first, "foreign", "second")
            }
            "itself" => reg.delegate_continuation(first, "owner", "itself"),
            "reason" => reg.delegate_continuation(first, "foreign", " "),
            "ninth" | "eighth" => {
                let take = if self.0 == "ninth" {
                    handles.len()
                } else {
                    MAX_CONTINUATION_DELEGATES
                };
                for handle in handles.iter().take(take) {
                    reg.delegate_continuation(handle, "foreign", "bounded")?;
                }
                Ok(())
            }
            "absent" => reg.delegate_continuation(first, "absent", "uncomposed"),
            _ => Ok(()),
        }
    }
}
#[inline(never)]
fn bounded(case: &'static str) -> Result<Box<Engine>, String> {
    let shared = Arc::new(Shared::default());
    Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Bounded(case)), Box::new(Foreign(shared))],
    )
    .map(Box::new)
    .map_err(|e| e.to_string())
}

/// 17g C2h: one grantee per handle (a second delegation of one handle
/// refuses), never the owner itself, a reason of 1 to 256 bytes, at most
/// `MAX_CONTINUATION_DELEGATES` per composition (eight compose, the ninth
/// refuses), and a grantee that is not composed refuses at freeze with an
/// error naming the command and the grantee. The public registrar cannot
/// delegate at all.
#[test]
fn delegation_is_one_grantee_bounded_and_checked_at_freeze() -> TestResult {
    assert_eq!(MAX_CONTINUATION_DELEGATES, 8);
    for (case, why) in [
        ("second", "the handle already has its one grantee"),
        ("itself", "the owner cannot delegate to itself"),
        ("reason", "a delegation names its reason in 1 to 256 bytes"),
        ("ninth", "the composition's delegation bound is reached"),
    ] {
        let error = bounded(case).err().ok_or(case)?;
        assert!(error.contains(why), "{case}: {error}");
    }
    let eight = bounded("eighth")?;
    assert_eq!(
        eight.registry.continuation_delegations().len(),
        MAX_CONTINUATION_DELEGATES
    );
    let error = bounded("absent").err().ok_or("absent")?;
    assert!(
        error.contains("@bounded/s0") && error.contains("absent"),
        "{error}"
    );
    assert!(error.contains("not a composed module"), "{error}");
    let (_, shared) = engine(spec(), None)?;
    let mut registry = Registry::new();
    let mut registrar = Registrar::new(&mut registry, "owner");
    registrar.classify_commands(CommandCompatibility::Authoritative);
    registrar.command(spec(), Arc::new(|_: &mut Ctx<'_>| Ok(())))?;
    assert!(
        registrar
            .delegate_continuation(&handle(&shared)?, "foreign", "public")
            .is_err(),
        "the public registrar cannot delegate"
    );
    Ok(())
}

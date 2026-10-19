use super::*;
use std::sync::Mutex;

use crate::{
    ArgShape, Audience, CommandCompatibility, CommandSpec, Engine, GOD, GameModule, Inbound,
    ModError, Outbox, Registrar, Registry, ScheduledKind,
    cmd::CommandGate,
    engine::Effects,
    module::{ContextOutcome, InputContext, InvokeHandler},
};
use mush_proto::{
    InputMode, InvokeRequest, InvokeResult, MindRequest, OutputEvent, PromptSpec, RichText,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
const ACTOR: ObjId = ObjId(2);
const CONN: ConnId = 7;

#[derive(Default)]
struct Shared {
    handle: Mutex<Option<ConnectionCompletionHandle>>,
    ticket: Mutex<Option<ConnectionCompletion>>,
    calls: Mutex<Vec<(ObjId, ConnId, String)>>,
    denied: AtomicBool,
    freeze: AtomicBool,
}
impl Shared {
    fn handle(&self) -> Result<ConnectionCompletionHandle, CmdError> {
        self.handle
            .lock()
            .map_err(|_| CmdError::msg("handle lock"))?
            .clone()
            .ok_or_else(|| CmdError::msg("handle"))
    }
    fn ticket(&self) -> Result<ConnectionCompletion, CmdError> {
        self.ticket
            .lock()
            .map_err(|_| CmdError::msg("ticket lock"))?
            .clone()
            .ok_or_else(|| CmdError::msg("ticket"))
    }
    fn capture(&self, ctx: &Ctx<'_>, argument: &str) -> Result<(), CmdError> {
        let ticket = ctx.capture_connection_completion(&self.handle()?, argument)?;
        *self
            .ticket
            .lock()
            .map_err(|_| CmdError::msg("ticket lock"))? = Some(ticket);
        Ok(())
    }
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
        if verb == "@finish" && self.0.freeze.load(Ordering::Acquire) {
            reg.freeze_mechanics_owner(
                &crate::mechanics::OwnerId::new("owner").map_err(|e| e.to_string())?,
                crate::mechanics::IncidentId(1),
                crate::mechanics::RecoveryReason::IntegrityViolation {
                    code: crate::mechanics::CanonicalId::new("test").map_err(|e| e.to_string())?,
                },
            )
            .map_err(|e| e.to_string())?;
        }
        if verb == "@finish" && self.0.denied.load(Ordering::Acquire) {
            Err("completion gated".into())
        } else {
            Ok(())
        }
    }
}
struct Invoke(Arc<Shared>);
impl InvokeHandler for Invoke {
    fn invoke(
        &self,
        ctx: &mut Ctx<'_>,
        _: ConnId,
        _: u64,
        request: &InvokeRequest,
    ) -> InvokeResult {
        assert!(self.0.capture(ctx, "invoke").is_ok());
        request.unavailable()
    }
}
struct Module {
    shared: Arc<Shared>,
    class: CommandCompatibility,
}
impl GameModule for Module {
    fn name(&self) -> &'static str {
        "owner"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(self.class);
        let shared = Arc::clone(&self.shared);
        reg.command(
            CommandSpec {
                name: "@finish".into(),
                aliases: vec!["@alias".into()],
                switches: vec!["done".into()],
                shape: ArgShape::Raw,
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| {
                let ticket = shared.ticket()?;
                if !ticket.is_current(ctx) {
                    return Err(CmdError::msg("not captured completion"));
                }
                assert!(
                    ctx.capture_connection_completion(&shared.handle()?, "recursive")
                        .is_err()
                );
                assert!(ctx.w.clone().connection_dispatch().is_none());
                let caller = ctx
                    .typed_state
                    .dispatch_caller()
                    .ok_or_else(|| CmdError::msg("stamp"))?;
                assert_eq!(caller.actor(), ctx.actor);
                assert_eq!(caller.module().as_str(), "owner");
                shared
                    .calls
                    .lock()
                    .map_err(|_| CmdError::msg("calls"))?
                    .push((ctx.actor, ticket.connection(), ctx.args.raw().into()));
                if ctx.args.raw() == "handler_error" {
                    ctx.out.defer(crate::Deferred::ScheduleIn {
                        delay_ms: 0,
                        kind: ScheduledKind::Control("rollback".into()),
                    });
                    return Err(CmdError::msg("handler refusal"));
                }
                ctx.out.to_conn(
                    ticket.connection(),
                    OutputEvent::Sys(RichText::plain("private result")),
                );
                Ok(())
            }),
        )?;
        let handle = reg.connection_completion("@finish", &["done"])?;
        assert!(reg.connection_completion("@alias", &["done"]).is_err());
        assert!(reg.connection_completion("@finish", &["missing"]).is_err());
        assert!(reg.command_continuation("@finish", &["done"]).is_err());
        *self
            .shared
            .handle
            .lock()
            .map_err(|_| ModError::msg("handle lock"))? = Some(handle);
        let shared = Arc::clone(&self.shared);
        reg.command(
            CommandSpec {
                name: "@capture".into(),
                shape: ArgShape::Raw,
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| {
                let argument = ctx.args.raw().to_owned();
                if argument == "spoof" {
                    ctx.actor = GOD;
                    ctx.module = Some("foreign".into());
                    ctx.w.set_dispatching_conn(Some(999));
                }
                shared.capture(ctx, &argument)?;
                if argument == "rollback" {
                    ctx.schedule_connection_completion(&shared.ticket()?)?;
                    return Err(CmdError::msg("capture refusal"));
                }
                Ok(())
            }),
        )?;
        let shared = Arc::clone(&self.shared);
        reg.command(
            CommandSpec {
                name: "@schedule".into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| {
                ctx.schedule_connection_completion(&shared.ticket()?)
            }),
        )?;
        reg.invoke_handler("mind", Arc::new(Invoke(Arc::clone(&self.shared))))?;
        reg.command_gate(Arc::new(Gate(Arc::clone(&self.shared))));
        Ok(())
    }
}
struct Foreign(Arc<Shared>);
impl GameModule for Foreign {
    fn name(&self) -> &'static str {
        "foreign"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        assert!(reg.connection_completion("@finish", &["done"]).is_err());
        reg.classify_commands(CommandCompatibility::LegacyPostImage);
        let shared = Arc::clone(&self.0);
        reg.command(
            CommandSpec {
                name: "@foreign".into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| {
                ctx.actor = ACTOR;
                ctx.module = Some("owner".into());
                ctx.w.set_dispatching_conn(Some(CONN));
                assert!(shared.capture(ctx, "foreign").is_err());
                ctx.schedule_connection_completion(&shared.ticket()?)
            }),
        )
    }
}
fn engine(class: CommandCompatibility) -> TestResult<(Engine, Arc<Shared>)> {
    let mut world = World::new_minimal();
    assert_eq!(
        world.create(crate::ObjKind::Player, "Subject", GOD, Some(crate::LIMBO))?,
        ACTOR
    );
    world.set_now(100);
    let shared = Arc::new(Shared::default());
    let mut engine = Engine::with_modules(
        world,
        Registry::new(),
        vec![
            Box::new(Module {
                shared: Arc::clone(&shared),
                class,
            }),
            Box::new(Foreign(Arc::clone(&shared))),
        ],
    )?;
    attach(&mut engine, CONN, ACTOR);
    Ok((engine, shared))
}
fn attach(engine: &mut Engine, conn: ConnId, actor: ObjId) {
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
fn command(engine: &mut Engine, line: &str) -> Outbox {
    let mut out = Outbox::new();
    engine.step(
        Inbound::Cmd {
            conn: CONN,
            actor: ACTOR,
            line: line.into(),
        },
        &mut out,
        &mut Effects::new(),
    );
    out
}
fn fire(engine: &mut Engine) -> Outbox {
    let mut out = Outbox::new();
    engine.fire_due(100_000, &mut out, &mut Effects::new());
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
fn actual_ingress_ignores_mutable_fields_and_completes_once_to_original_connection() -> TestResult {
    let (mut e, s) = engine(CommandCompatibility::LegacyPostImage)?;
    attach(&mut e, 999, GOD);
    command(&mut e, "@capture spoof");
    command(&mut e, "@finish/done spoof");
    assert!(s.calls.lock().map_err(|_| "calls")?.is_empty());
    command(&mut e, "@schedule");
    command(&mut e, "@schedule");
    assert_eq!(e.sched.len(), 1);
    let duplicate = e
        .sched
        .pending_entries()
        .entries()
        .first()
        .map(|(_, k)| (*k).clone())
        .ok_or("queued")?;
    e.sched.insert(100_000, duplicate);
    let lines = Arc::new(Mutex::new(Vec::new()));
    e.world.push_input_context(
        ACTOR,
        Some("owner".into()),
        Box::new(Editor(Arc::clone(&lines))),
    );
    let out = fire(&mut e);
    assert_eq!(
        *s.calls.lock().map_err(|_| "calls")?,
        vec![(ACTOR, CONN, "spoof".into())]
    );
    assert!(lines.lock().map_err(|_| "lines")?.is_empty());
    assert_eq!(out.events().len(), 1);
    assert!(
        out.events()
            .iter()
            .all(|(audience, _)| *audience == Audience::Conn(CONN))
    );
    assert!(s.ticket()?.has_entered());
    assert!(e.world.connection_dispatch().is_none());
    assert_eq!(e.world.dispatching_conn(), None);
    Ok(())
}

#[test]
fn actual_invoke_captures_but_timer_foreign_owner_and_wrong_registry_cannot() -> TestResult {
    let (mut e, s) = engine(CommandCompatibility::LegacyPostImage)?;
    e.step(
        Inbound::Invoke {
            conn: CONN,
            actor: ACTOR,
            id: 9,
            request: InvokeRequest::Mind(MindRequest::List {}),
        },
        &mut Outbox::new(),
        &mut Effects::new(),
    );
    assert_eq!(s.ticket()?.connection(), CONN);
    command(&mut e, "@foreign");
    assert!(e.sched.is_empty());
    e.sched.insert(
        100_000,
        ScheduledKind::WaitCmd {
            actor: ACTOR,
            line: "@capture timer".into(),
        },
    );
    fire(&mut e);
    assert_eq!(s.ticket()?.0.argument, "invoke");
    let (_, other) = engine(CommandCompatibility::LegacyPostImage)?;
    *s.handle.lock().map_err(|_| "handle")? = Some(other.handle()?);
    command(&mut e, "@capture other_registry");
    assert_eq!(s.ticket()?.0.argument, "invoke");
    Ok(())
}

#[test]
fn stale_lease_or_actor_and_expired_wall_lifetime_never_enter() -> TestResult {
    for mode in 0..4 {
        let (mut e, s) = engine(CommandCompatibility::LegacyPostImage)?;
        command(&mut e, "@capture ticket");
        if mode == 3 {
            let mut ticket = s
                .ticket
                .lock()
                .map_err(|_| "ticket")?
                .take()
                .ok_or("ticket")?;
            Arc::get_mut(&mut ticket.0).ok_or("unique capture")?.expires = Instant::now();
            *s.ticket.lock().map_err(|_| "ticket")? = Some(ticket);
        }
        command(&mut e, "@schedule");
        match mode {
            0 => {
                e.world.unbind_connection(CONN);
                e.world.bind_connection(CONN, ACTOR);
            }
            1 => {
                e.world.unbind_connection(CONN);
                e.world.bind_connection(CONN, GOD);
            }
            2 => {
                e.world
                    .get_mut(ACTOR)
                    .ok_or("actor")?
                    .flags
                    .insert(crate::Flags::GOING);
            }
            _ => {}
        }
        assert!(fire(&mut e).events().is_empty());
        assert!(s.calls.lock().map_err(|_| "calls")?.is_empty());
    }
    let (mut e, s) = engine(CommandCompatibility::LegacyPostImage)?;
    command(&mut e, "@capture live");
    e.world.set_now(i64::MAX);
    assert!(
        s.ticket()?.is_live(&e.world),
        "IC changes do not consume wall lifetime"
    );
    Ok(())
}

#[test]
fn gates_preserve_unentered_ticket_and_deferred_rollback_releases_queue_marker() -> TestResult {
    let (mut e, s) = engine(CommandCompatibility::LegacyPostImage)?;
    command(&mut e, "@capture rollback");
    assert!(e.sched.is_empty());
    command(&mut e, "@schedule");
    assert_eq!(e.sched.len(), 1);
    s.denied.store(true, Ordering::Release);
    fire(&mut e);
    assert!(!s.ticket()?.has_entered());
    assert!(s.calls.lock().map_err(|_| "calls")?.is_empty());
    s.denied.store(false, Ordering::Release);
    command(&mut e, "@schedule");
    fire(&mut e);
    assert_eq!(s.calls.lock().map_err(|_| "calls")?.len(), 1);
    command(&mut e, "@schedule");
    assert!(e.sched.is_empty());
    command(&mut e, "@capture handler_error");
    command(&mut e, "@schedule");
    fire(&mut e);
    assert!(
        e.sched.is_empty(),
        "legacy handler error truncates deferred work"
    );
    assert!(
        s.ticket()?.has_entered(),
        "a failed entered handler cannot be replayed"
    );
    Ok(())
}

#[test]
fn queue_budget_refusal_releases_only_the_unentered_attempt() -> TestResult {
    let (mut e, s) = engine(CommandCompatibility::LegacyPostImage)?;
    for n in 0..100 {
        command(&mut e, &format!("@capture job{n}"));
        command(&mut e, "@schedule");
    }
    assert_eq!(e.sched.len(), 100);
    command(&mut e, "@capture excess");
    command(&mut e, "@schedule");
    assert_eq!(e.sched.len(), 100);
    assert!(!s.ticket()?.has_entered());
    fire(&mut e);
    // Earlier tickets cannot pass the fixture's exact retained-job check.
    assert!(s.calls.lock().map_err(|_| "calls")?.is_empty());
    command(&mut e, "@schedule");
    assert_eq!(e.sched.len(), 1);
    fire(&mut e);
    assert_eq!(s.calls.lock().map_err(|_| "calls")?.len(), 1);
    Ok(())
}

#[test]
fn registration_requires_legacy_exact_owner_and_composed_authority() -> TestResult {
    for class in [
        CommandCompatibility::ReadOnly,
        CommandCompatibility::Authoritative,
        CommandCompatibility::EconomyCarveOut,
    ] {
        assert!(engine(class).is_err());
    }
    let mut registry = Registry::new();
    let mut registrar = Registrar::new(&mut registry, "owner");
    registrar.classify_commands(CommandCompatibility::LegacyPostImage);
    registrar.command(
        CommandSpec {
            name: "@finish".into(),
            ..CommandSpec::default()
        },
        Arc::new(|_: &mut Ctx<'_>| Ok(())),
    )?;
    assert!(registrar.connection_completion("@finish", &[]).is_err());
    Ok(())
}

#[test]
fn owner_freeze_after_gate_and_foreign_composition_refuse_before_entry() -> TestResult {
    let (mut e, s) = engine(CommandCompatibility::LegacyPostImage)?;
    command(&mut e, "@capture private_job");
    command(&mut e, "@schedule");
    let copied = e
        .sched
        .pending_entries()
        .entries()
        .first()
        .map(|(_, request)| (*request).clone())
        .ok_or("queued")?;
    let (mut other, other_shared) = engine(CommandCompatibility::LegacyPostImage)?;
    other.sched.insert(100_000, copied);
    fire(&mut other);
    assert!(other_shared.calls.lock().map_err(|_| "calls")?.is_empty());
    assert!(!s.ticket()?.has_entered());
    s.freeze.store(true, Ordering::Release);
    fire(&mut e);
    assert!(s.calls.lock().map_err(|_| "calls")?.is_empty());
    assert!(!s.ticket()?.has_entered());
    Ok(())
}

#[test]
fn oversized_argument_and_missing_ingress_binding_cannot_capture() -> TestResult {
    let (mut e, s) = engine(CommandCompatibility::LegacyPostImage)?;
    command(
        &mut e,
        &format!("@capture {}", "x".repeat(crate::MAX_INGRESS_LINE_BYTES)),
    );
    assert!(s.ticket.lock().map_err(|_| "ticket")?.is_none());
    e.world.unbind_connection(CONN);
    command(&mut e, "@capture no_connection");
    assert!(s.ticket.lock().map_err(|_| "ticket")?.is_none());
    assert!(e.sched.is_empty());
    Ok(())
}

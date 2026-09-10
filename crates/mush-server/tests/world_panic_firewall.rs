//! WS01-S4: a panic in module code stops at the world-task dispatch boundary.
//!
//! The world task owns the in-RAM world and runs every command handler, tick
//! and observer in the process synchronously (docs/PLAN.md "Architecture"). A
//! single `unwrap` in a module used to unwind out of the loop and take the game
//! down while the store writer sat there with a live queue — no freeze, no
//! notice, and whatever the faulted turn had already said still on its way to
//! the players.
//!
//! Every proof drives the **real** loop from
//! [`spawn_persist_with_gateway`](mush_server::world_task::spawn_persist_with_gateway)
//! and really panics; nothing here simulates a fault. What the firewall owes is
//! four things at once: the process survives, persistence latches, the loop's
//! join says *why* it stopped, and nothing the faulted turn touched reaches
//! either the store or a player.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mush_config::ipguard::IpGuard;
use mush_config::metrics::Metrics;
use mush_core::{
    ActionError, ArgShape, AuthoritativeBundle, AuthoritativeBundleDraft,
    AuthoritativeSemanticEvent, CanonicalHash, CanonicalId, CmdError, CommandSpec, CommitToken,
    Ctx, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, Engine,
    GOD, GameModule, IdempotencyKey, IdempotencyScope, Inbound, LIMBO, ModError, ObjKind, OwnerId,
    PreparedCausalRef, PreparedTxn, PrincipalId, Registrar, RequestUid, RevisionKey, RevisionKind,
    RevisionSet, RevisionStamp, RevisionValidator, RulesValue, SceneId, ScopedIdempotencyKey,
    StateFamilyDescriptor, StateFamilyHandle, StateFamilyId, SubjectKey, TickCadence, World,
};
use mush_ipc::{DetachCause, SendTarget};
use mush_proto::OutputEvent;
use mush_server::gateway_sink::GatewaySink;
use mush_server::quarantine::QuarantineSpool;
use mush_server::staff::StaffQueue;
use mush_server::world_task::{
    self, Gateway, PersistHandles, ServerDeps, WorldOutcome, spawn_persist_with_gateway,
};
use mush_store::{InMemoryStore, ObjectStore};

// ---------------------------------------------------------------------------
// Rig
// ---------------------------------------------------------------------------

/// A sink that keeps every delivered event as text, so a proof can ask what a
/// connected wizard was actually told.
#[derive(Default)]
struct RecordingGateway {
    sent: std::sync::Mutex<Vec<String>>,
}

impl RecordingGateway {
    fn saw(&self, needle: &str) -> bool {
        self.sent
            .lock()
            .expect("recording gateway mutex")
            .iter()
            .any(|line| line.contains(needle))
    }
}

impl GatewaySink for RecordingGateway {
    fn transmit(&self, sends: Vec<(SendTarget, Arc<OutputEvent>)>) {
        let mut log = self.sent.lock().expect("recording gateway mutex");
        for (target, event) in sends {
            log.push(format!("{target:?} {event:?}"));
        }
    }

    fn set_flood_exempt(&self, _char: u64, _exempt: bool) {}

    fn is_socket(&self) -> bool {
        false
    }

    fn close_session(&self, _char: u64, _cause: DetachCause) {}

    fn close_web_sessions(
        &self,
        _chars: Vec<u64>,
        _notice: Option<OutputEvent>,
        _cause: DetachCause,
    ) {
    }

    fn close_char_connections(&self, _char: u64) -> Vec<u64> {
        Vec::new()
    }

    fn structured_conns_of(&self, _chars: &[u64]) -> Vec<(u64, u64)> {
        Vec::new()
    }

    fn detach_conn_local(&self, _conn: u64) {}

    fn session_counts(&self) -> (i64, i64) {
        (0, 0)
    }
}

/// The command half of the fault: one line that mutates the world and then
/// explodes with the mutation still only in RAM.
struct HandlerPanicModule;

impl GameModule for HandlerPanicModule {
    fn name(&self) -> &'static str {
        "panic_firewall_test"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.command(
            CommandSpec {
                name: "@panicnow".into(),
                shape: ArgShape::None,
                help: "test-only: mutate, then panic".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                ctx.w
                    .create(ObjKind::Thing, "panic-orb", ctx.actor, Some(LIMBO))
                    .map_err(|error| CmdError::msg(error.to_string()))?;
                panic!("handler exploded");
            }),
        )?;
        Ok(())
    }
}

/// The tick half: a periodic module tick that explodes the first time it fires.
/// `Engine::fire_due` is the only other place module code runs synchronously on
/// a bare timer wake, and `mush-economy`'s `DepthGuard` is written on the
/// assumption that this boundary exists.
struct TickPanicModule;

impl GameModule for TickPanicModule {
    fn name(&self) -> &'static str {
        "panic_firewall_tick_test"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.tick(
            "explode",
            TickCadence::Fast(Duration::from_millis(30)),
            Arc::new(|_ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>| {
                panic!("tick exploded");
            }),
        )?;
        Ok(())
    }
}

fn boot(
    store: Arc<dyn ObjectStore>,
    gateway: Arc<RecordingGateway>,
    metrics: Arc<Metrics>,
    modules: Vec<Box<dyn GameModule>>,
) -> PersistHandles {
    let engine = Engine::with_modules(World::new_minimal(), world_task::base_registry(), modules)
        .expect("panic-firewall test module composes");
    spawn_persist_with_gateway(
        engine,
        ServerDeps {
            store,
            guard: Arc::new(IpGuard::new()),
            staff: Arc::new(StaffQueue::new()),
            llm: None,
            metrics,
            quarantine_spool: QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
        },
        move |_| -> Gateway { gateway },
    )
}

// ---------------------------------------------------------------------------
// Proofs
// ---------------------------------------------------------------------------

/// The whole firewall in one line of play: a connected wizard types a command
/// whose handler panics.
#[tokio::test(flavor = "multi_thread")]
async fn a_panicking_command_handler_freezes_the_world_instead_of_killing_it() {
    let store = Arc::new(InMemoryStore::new());
    let gateway = Arc::new(RecordingGateway::default());
    let metrics = Arc::new(Metrics::new());
    let handles = boot(
        Arc::clone(&store) as Arc<dyn ObjectStore>,
        Arc::clone(&gateway),
        Arc::clone(&metrics),
        vec![Box::new(HandlerPanicModule)],
    );
    assert!(
        !handles.persistence_integrity.is_latched(),
        "a healthy boot latches nothing"
    );

    // God is a Wizard-or-better, and the freeze notice goes to connected staff:
    // attach a connection so there is somebody to tell.
    assert!(
        handles
            .handle
            .inbound(Inbound::ConnUp {
                conn: 1,
                actor: GOD,
                caps_structured: false,
                from_linkdead: false,
            })
            .await
    );
    assert!(handles.handle.barrier().await, "the connect settles");

    assert!(
        handles
            .handle
            .inbound(Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@panicnow".to_string(),
            })
            .await
    );

    // (a) The process survives — reaching this line at all is the assertion —
    // and (c) the loop's join says what stopped it rather than being a
    // `JoinError` nobody can read.
    let outcome = handles
        .world_join
        .await
        .expect("the dispatch boundary caught the panic; the task did not unwind");
    let WorldOutcome::Faulted(fault) = outcome else {
        panic!("a panicking handler must leave a Faulted outcome, got {outcome:?}");
    };
    assert!(
        fault.detail.contains("handler exploded"),
        "the fault carries the panic payload: {fault:?}"
    );
    assert_eq!(fault.site, "engine.step", "and where it was caught");

    // (b) Persistence is latched, so the shutdown coordinator refuses the
    // ordinary final BGSAVE and the writer refuses later batches.
    assert!(
        handles.persistence_integrity.is_latched(),
        "a caught world panic is a persistence-integrity breach"
    );
    assert!(
        !handles.world_exit.is_clean(),
        "and it is not a clean exit either"
    );
    assert_eq!(
        metrics.snapshot().world_panics_total,
        1,
        "the fault is counted where a scraper can see it"
    );

    // (d) The wizard was told, in the words `apply_integrity_freeze` uses (the
    // all-caps PERSISTENCE INTEGRITY BREACH is the log line; this is the
    // player-facing half of the same call).
    assert!(
        gateway.saw("GAME: persistence integrity breach"),
        "connected staff hear about the freeze"
    );
    assert!(
        gateway.saw("world task panicked at engine.step: handler exploded"),
        "and the notice names the fault that caused it"
    );

    // (e) The half-applied mutation never reached the store: the loop broke
    // before the foot-of-loop flush, and nothing re-enters the engine after.
    let snapshot = store
        .load_world()
        .await
        .expect("in-memory load")
        .expect("the boot wrote a world");
    assert!(
        !snapshot.objects.iter().any(|obj| obj.name == "panic-orb"),
        "no write may escape the faulted turn"
    );
}

/// The same boundary on the timer lane: a module tick that panics is caught by
/// the `fire_due` guard, not by the runtime's task abort.
#[tokio::test(flavor = "multi_thread")]
async fn a_panicking_module_tick_is_caught_at_fire_due() {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let gateway = Arc::new(RecordingGateway::default());
    let metrics = Arc::new(Metrics::new());
    let handles = boot(
        store,
        Arc::clone(&gateway),
        Arc::clone(&metrics),
        vec![Box::new(TickPanicModule)],
    );

    // The tick is armed at engine construction, so nothing needs driving: the
    // first firing is the fault.
    let outcome = tokio::time::timeout(Duration::from_secs(5), handles.world_join)
        .await
        .expect("the tick fires well inside the timeout")
        .expect("the dispatch boundary caught the panic");
    let WorldOutcome::Faulted(fault) = outcome else {
        panic!("a panicking tick must leave a Faulted outcome, got {outcome:?}");
    };
    assert!(
        fault.detail.contains("tick exploded"),
        "the fault carries the tick's payload: {fault:?}"
    );
    assert_eq!(fault.site, "fire_due", "caught on the timer lane");
    assert!(
        handles.persistence_integrity.is_latched(),
        "a caught tick panic freezes persistence too"
    );
}

// ---------------------------------------------------------------------------
// The recovery path itself
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ResidueState {
    amount: u8,
}

type ResidueHandle = StateFamilyHandle<u64, ResidueState>;

/// A revision validator that explodes **on demand**. Guard *construction* must
/// still work (the staged action has to exist before the fault does), so the
/// panic is armed only once the action is staged.
struct ArmedRevision {
    armed: Arc<AtomicBool>,
}

impl RevisionValidator for ArmedRevision {
    fn current_generation(
        &self,
        _context: &mush_core::RevisionValidatorContext<'_>,
        _key: &RevisionKey,
    ) -> Result<Option<u64>, ActionError> {
        assert!(
            !self.armed.load(Ordering::SeqCst),
            "guard validator exploded"
        );
        Ok(Some(1))
    }
}

struct ResidueModule {
    armed: Arc<AtomicBool>,
    captured: Arc<std::sync::Mutex<Option<ResidueHandle>>>,
}

impl GameModule for ResidueModule {
    fn name(&self) -> &'static str {
        "panic_residue_test"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.revision_validator(
            RevisionKind::new(CanonicalId::new("panic_residue").expect("static revision kind")),
            ArmedRevision {
                armed: Arc::clone(&self.armed),
            },
        )?;
        let descriptor = StateFamilyDescriptor::new(
            StateFamilyId::new("residue").expect("static family id"),
            CanonicalId::new("u64.decimal.v1").expect("static codec id"),
            CanonicalId::new("residue_state.v1").expect("static schema id"),
            1,
            CanonicalId::new("residue_state.decode_json.v1").expect("static decoder id"),
            CanonicalId::new("residue_state.range.v1").expect("static validator id"),
            CanonicalId::new("residue_state.json.v1").expect("static serializer id"),
        )
        .expect("static state-family descriptor");
        let handle = reg.state_family(
            descriptor,
            |key: &u64| Ok(key.to_string()),
            |value: &serde_json::Value| {
                serde_json::from_value(value.clone()).map_err(|error| error.to_string())
            },
            |_value: &ResidueState| Ok(()),
            |value: &ResidueState| Ok(serde_json::json!({ "amount": value.amount })),
        )?;
        *self.captured.lock().expect("capture mutex") = Some(handle);
        Ok(())
    }
}

/// One complete K3 bundle around the residue transaction: the staging lane
/// takes nothing else, and this test only needs a staged action to discard.
fn residue_bundle(prepared: &PreparedTxn) -> AuthoritativeBundle {
    let owner = OwnerId::new("panic_residue_test").expect("static owner id");
    let mut definitions = DefinitionRegistryBuilder::new();
    let action = definitions
        .register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("action").expect("static definition kind"),
                DefinitionId::new("stash_residue").expect("static definition id"),
                1,
                serde_json::json!({}),
            )
            .expect("static action descriptor"),
        )
        .expect("the action definition registers");
    let outcome_definition = definitions
        .register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("outcome").expect("static definition kind"),
                DefinitionId::new("residue_stashed").expect("static definition id"),
                1,
                serde_json::json!({}),
            )
            .expect("static outcome descriptor"),
        )
        .expect("the outcome definition registers");
    let request = RequestUid::from_bytes([0x71; 16]);
    let principal = PrincipalId::Object { id: GOD };
    let cause = PreparedCausalRef::new(request, None, 0).expect("root cause");
    let outcome = RulesValue::new(
        outcome_definition.clone(),
        serde_json::json!({ "accepted": true }),
    )
    .expect("the outcome matches its schema");
    AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: CommitToken::from_bytes([0x72; 16]),
        request,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: principal.clone(),
                owner: owner.clone(),
                namespace: CanonicalId::new("stash_residue").expect("static namespace"),
            },
            0,
            IdempotencyKey::Request(request),
        ),
        ingress_hash: CanonicalHash::from_bytes([0x73; 32]),
        request_hash: CanonicalHash::from_bytes([0x74; 32]),
        plan_hash: CanonicalHash::from_bytes([0x75; 32]),
        action: action.clone(),
        provider: owner,
        cause,
        definitions: vec![action.clone(), outcome_definition],
        writes: prepared.authoritative_replay_writes(),
        outcome: outcome.clone(),
        resolution: None,
        semantic_events: vec![
            AuthoritativeSemanticEvent::root(cause, &action, outcome, vec![principal.clone()])
                .expect("one root semantic event"),
        ],
        eligible_witnesses: vec![principal],
        durable_outbox: vec![],
    })
    .expect("the residue transaction forms an authoritative bundle")
}

/// The case the earlier plan would have failed: the loop faults with an action
/// already staged, and the cleanup must not become the second panic — one
/// inside a `catch_unwind` handler aborts the process outright.
///
/// This is why the recovery path calls the infallible
/// `Engine::discard_step_residue` and not `Engine::take_staged_action`: the
/// latter revalidates every guard through the registry, which means running
/// module-supplied validator code on the way out of a fault. Here that code
/// panics, and the two calls are told apart by it.
#[test]
fn discarding_a_staged_action_does_not_re_enter_module_code() {
    let armed = Arc::new(AtomicBool::new(false));
    let captured = Arc::new(std::sync::Mutex::new(None));
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        mush_core::Registry::new(),
        vec![Box::new(ResidueModule {
            armed: Arc::clone(&armed),
            captured: Arc::clone(&captured),
        })],
    )
    .expect("residue module composes");
    // The minimal world's seed objects are unrelated legacy dirtiness; a
    // prepared action refuses to stage beside them.
    engine.world.take_dirty();
    let handle = captured
        .lock()
        .expect("capture mutex")
        .clone()
        .expect("registration captured the family handle");

    let mut transaction = engine
        .action_transaction(&handle)
        .expect("test transaction begins");
    transaction
        .insert(&handle, &7, ResidueState { amount: 3 })
        .expect("test state is valid");
    let prepared = transaction.prepare().expect("test transaction prepares");
    let revisions = RevisionSet::from_stamps([RevisionStamp {
        key: RevisionKey {
            owner: OwnerId::new("panic_residue_test").expect("static owner id"),
            kind: RevisionKind::new(
                CanonicalId::new("panic_residue").expect("static revision kind"),
            ),
            subject: SubjectKey::Object { id: GOD },
            facet: None,
        },
        generation: 1,
    }])
    .expect("single revision stamp is canonical");
    let guards = engine
        .registry
        .server_guard_set_builder(revisions)
        .expect("composed registry builds guards")
        .finish()
        .expect("single revision guard is valid");
    let bundle = residue_bundle(&prepared);
    engine
        .stage_authoritative_action(prepared, guards, bundle)
        .expect("a clean engine accepts the prepared action");
    assert!(engine.action_state().has_prepared());

    // The turn faults from here on: module code is now poison.
    armed.store(true, Ordering::SeqCst);

    // The path the recovery may NOT take. Catching it here is what makes the
    // proof below mean something — and it leaves the action still staged,
    // because the validator blows up before the take.
    let revalidated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = engine.take_staged_action();
    }));
    assert!(
        revalidated.is_err(),
        "take_staged_action runs module validator code, which is the whole problem"
    );
    assert!(
        engine.has_staged_action(),
        "and it left the residue behind for the discard to clear"
    );

    // The path it does take: no registry, no guards, no validators.
    let discarded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        engine.discard_step_residue();
    }));
    assert!(
        discarded.is_ok(),
        "the fault cleanup must never be the second panic"
    );
    assert!(
        !engine.has_staged_action(),
        "the staged action is gone with the faulted turn"
    );
    assert!(
        !engine.action_state().has_prepared(),
        "and its store reservation was returned, not stranded"
    );
}

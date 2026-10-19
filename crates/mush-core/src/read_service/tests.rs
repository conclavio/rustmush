use super::*;
use crate::action_state::{StateFamilyDescriptor, StateFamilyId};
use crate::mechanics::{
    CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder,
    IncidentId, RecoveryReason,
};
use crate::module::{HookReadContext, QueryHandler};
use crate::{
    CmdError, CommandSpec, Ctx, Engine, GOD, GameModule, Inbound, ModError, Outbox, Registrar,
    Registry,
};
use mush_proto::{OutputEvent, RichText};
use serde_json::{Value, json};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

fn descriptor() -> DefinitionDescriptor {
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").unwrap(),
        DefinitionId::new("current").unwrap(),
        1,
        json!({"format":1}),
    )
    .unwrap()
}
fn definition() -> DefinitionRef {
    DefinitionRegistryBuilder::new()
        .register(OwnerId::new("owner").unwrap(), descriptor())
        .unwrap()
}
struct Request {
    mode: u8,
    pretend_actor: ObjId,
    text: String,
    readiness: Option<Arc<Mutex<Option<MechanicalOwnerReadiness>>>>,
}
fn freeze(readiness: &Arc<Mutex<Option<MechanicalOwnerReadiness>>>, owner: &str) {
    readiness
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .freeze(
            &OwnerId::new(owner).unwrap(),
            IncidentId(78),
            RecoveryReason::IntegrityViolation {
                code: CanonicalId::new("serialize_test").unwrap(),
            },
        )
        .unwrap();
}
impl Serialize for Request {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let owner = match self.mode {
            13 => Some("owner"),
            14 => Some("dependency"),
            18 => Some("consumer"),
            _ => None,
        };
        if let Some(owner) = owner {
            freeze(self.readiness.as_ref().unwrap(), owner);
        }
        #[derive(Serialize)]
        struct Wire<'a> {
            mode: u8,
            pretend_actor: ObjId,
            text: &'a str,
        }
        Wire {
            mode: self.mode,
            pretend_actor: self.pretend_actor,
            text: &self.text,
        }
        .serialize(serializer)
    }
}
struct Answer {
    value: Value,
    on_serialize: Option<(&'static str, Arc<Mutex<Option<MechanicalOwnerReadiness>>>)>,
}
impl std::fmt::Debug for Answer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}
impl PartialEq for Answer {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}
impl Serialize for Answer {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if let Some((owner, readiness)) = &self.on_serialize {
            freeze(readiness, owner);
        }
        self.value.serialize(serializer)
    }
}
#[derive(Clone)]
struct Slot(u32);
struct Service {
    family: StateFamilyHandle<u32, u32>,
    calls: Arc<AtomicUsize>,
    readiness: Arc<Mutex<Option<MechanicalOwnerReadiness>>>,
}
impl OwnerReadService for Service {
    type State = Slot;
    type Request = Request;
    type Output = Answer;
    fn read(
        &self,
        state: &Slot,
        ctx: &OwnerReadContext<'_>,
        request: Request,
    ) -> Result<Answer, OwnerReadError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if request.mode >= 20 {
            return batch::read(ctx, &self.family, request.mode);
        }
        let value = ctx
            .action_state(&self.family, &1)?
            .copied()
            .ok_or(OwnerReadError::Refused)?;
        match request.mode {
            1 => {
                for _ in 0..MAX_OWNER_READS {
                    ctx.action_state(&self.family, &1)?;
                }
            }
            2 => {
                return Ok(Answer {
                    value: json!({"text":"x".repeat(MAX_OWNER_READ_RESULT_BYTES)}),
                    on_serialize: None,
                });
            }
            3 => {
                ctx.scan_action_state(&self.family, None, MAX_OWNER_READ_PAGE + 1)?;
            }
            8 => {
                for _ in 0..MAX_OWNER_READS {
                    let _ = ctx.action_state(&self.family, &1);
                }
            }
            9 => {
                for _ in 0..9 {
                    ctx.scan_action_state(&self.family, None, MAX_OWNER_READ_PAGE)?;
                }
            }
            4 | 5 | 10 => {
                let owner = OwnerId::new(match request.mode {
                    4 => "owner",
                    5 => "consumer",
                    _ => "dependency",
                })
                .unwrap();
                self.readiness
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .freeze(
                        &owner,
                        IncidentId(77),
                        RecoveryReason::IntegrityViolation {
                            code: CanonicalId::new("read_test").unwrap(),
                        },
                    )
                    .unwrap();
            }
            _ => {}
        }
        Ok(Answer {
            value: json!({"actor":ctx.actor(),"principal":ctx.principal(),"caller":ctx.requester(),"value":value,"slot":state.0}),
            on_serialize: match request.mode {
                15 => Some(("owner", self.readiness.clone())),
                16 => Some(("dependency", self.readiness.clone())),
                17 => Some(("consumer", self.readiness.clone())),
                _ => None,
            },
        })
    }
}
type Captured = Arc<Mutex<Option<StateFamilyHandle<u32, u32>>>>;
struct Owner {
    family: Captured,
    calls: Arc<AtomicUsize>,
    readiness: Arc<Mutex<Option<MechanicalOwnerReadiness>>>,
    allow: bool,
}
impl GameModule for Owner {
    fn name(&self) -> &'static str {
        "owner"
    }
    fn register(&mut self, r: &mut Registrar<'_>) -> Result<(), ModError> {
        let expected = definition();
        r.definition_validator(
            DefinitionKind::new("owner_read").map_err(|e| ModError::msg(e.to_string()))?,
            move |ctx: &crate::mechanics::DefinitionValidationContext<'_>| {
                let entry = ctx.definition();
                if entry.reference() == &expected
                    && entry.payload() == &json!({"format":1})
                    && entry.references().is_empty()
                    && !entry.is_current()
                {
                    Ok(())
                } else {
                    Err("owner-read fixture declaration differs".into())
                }
            },
        )?;
        r.state(Slot(9));
        let name = |n: &str| CanonicalId::new(n).unwrap();
        let family = r.state_family(
            StateFamilyDescriptor::new(
                StateFamilyId::new("truth").unwrap(),
                name("truth.key"),
                name("truth.value"),
                1,
                name("truth.decode"),
                name("truth.check"),
                name("truth.encode"),
            )
            .unwrap(),
            |k: &u32| Ok(k.to_string()),
            |v: &Value| serde_json::from_value(v.clone()).map_err(|e| e.to_string()),
            |_: &u32| Ok(()),
            |v: &u32| serde_json::to_value(v).map_err(|e| e.to_string()),
        )?;
        *self.family.lock().unwrap() = Some(family.clone());
        let def = r.definition(descriptor())?;
        r.owner_read_service(
            def,
            if self.allow {
                BTreeSet::from([OwnerId::new("consumer").unwrap()])
            } else {
                BTreeSet::new()
            },
            BTreeSet::from([OwnerId::new("dependency").unwrap()]),
            Service {
                family,
                calls: self.calls.clone(),
                readiness: self.readiness.clone(),
            },
        )
    }
}
struct Query {
    handle: OwnerReadHandle<Service>,
    mode: Arc<AtomicUsize>,
    answers: Arc<Mutex<Vec<Result<Value, OwnerReadError>>>>,
    readiness: Arc<Mutex<Option<MechanicalOwnerReadiness>>>,
}
impl QueryHandler for Query {
    fn query(&self, _: &World, _: ObjId, _: &Value) -> Result<OutputEvent, RichText> {
        Err(RichText::plain("no live query context"))
    }
    fn query_with_context(
        &self,
        ctx: &HookReadContext<'_>,
        _: ObjId,
        _: &Value,
    ) -> Result<OutputEvent, RichText> {
        let result = ctx
            .read_owner(
                &self.handle,
                Request {
                    mode: self.mode.load(Ordering::SeqCst) as u8,
                    pretend_actor: ObjId(999),
                    text: String::new(),
                    readiness: Some(self.readiness.clone()),
                },
            )
            .map(|answer| answer.value);
        self.answers.lock().unwrap().push(result);
        Ok(OutputEvent::Sys(RichText::plain("read completed")))
    }
}
struct Consumer {
    mode: Arc<AtomicUsize>,
    answers: Arc<Mutex<Vec<Result<Value, OwnerReadError>>>>,
    handle: Arc<Mutex<Option<OwnerReadHandle<Service>>>>,
    readiness: Arc<Mutex<Option<MechanicalOwnerReadiness>>>,
}
impl GameModule for Consumer {
    fn name(&self) -> &'static str {
        "consumer"
    }
    fn register(&mut self, r: &mut Registrar<'_>) -> Result<(), ModError> {
        r.classify_commands(crate::CommandCompatibility::ReadOnly);
        let handle = r.request_owner_read::<Service>(definition())?;
        *self.handle.lock().unwrap() = Some(handle.clone());
        r.query_handler(
            "private.read",
            Arc::new(Query {
                handle: handle.clone(),
                mode: self.mode.clone(),
                answers: self.answers.clone(),
                readiness: self.readiness.clone(),
            }),
        )?;
        let mode = self.mode.clone();
        let answers = self.answers.clone();
        let readiness = self.readiness.clone();
        r.command(
            CommandSpec {
                name: "serviceread".into(),
                ..Default::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let mode = mode.load(Ordering::SeqCst) as u8;
                let text = if mode == 6 {
                    "x".repeat(MAX_OWNER_READ_REQUEST_BYTES)
                } else {
                    String::new()
                };
                let request = Request {
                    mode,
                    pretend_actor: ObjId(999),
                    text,
                    readiness: Some(readiness.clone()),
                };
                let actual_actor = ctx.dispatch_actor();
                assert_eq!(actual_actor, Some(ctx.actor));
                if mode == 11 || mode == 12 {
                    ctx.actor = ObjId(999);
                    ctx.module = Some("owner".into());
                    assert_eq!(ctx.dispatch_actor(), actual_actor);
                    assert_ne!(ctx.dispatch_actor(), Some(ctx.actor));
                }
                let answer = if mode == 7 || mode == 12 {
                    ctx.hook_read_context("owner").read_owner(&handle, request)
                } else {
                    ctx.read_owner(&handle, request)
                };
                answers
                    .lock()
                    .unwrap()
                    .push(answer.map(|answer| answer.value));
                Ok(())
            }),
        )
    }
}
struct Fixture {
    engine: Engine,
    family: StateFamilyHandle<u32, u32>,
    calls: Arc<AtomicUsize>,
    mode: Arc<AtomicUsize>,
    answers: Arc<Mutex<Vec<Result<Value, OwnerReadError>>>>,
    handle: OwnerReadHandle<Service>,
}
struct Dependency;
impl GameModule for Dependency {
    fn name(&self) -> &'static str {
        "dependency"
    }
    fn register(&mut self, _: &mut Registrar<'_>) -> Result<(), ModError> {
        Ok(())
    }
}
fn fixture(reversed: bool, allow: bool) -> Result<Fixture, ModError> {
    let family = Arc::new(Mutex::new(None));
    let calls = Arc::new(AtomicUsize::new(0));
    let readiness = Arc::new(Mutex::new(None));
    let mode = Arc::new(AtomicUsize::new(0));
    let answers = Arc::new(Mutex::new(Vec::new()));
    let handle = Arc::new(Mutex::new(None));
    let mut modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(Owner {
            family: family.clone(),
            calls: calls.clone(),
            readiness: readiness.clone(),
            allow,
        }),
        Box::new(Consumer {
            mode: mode.clone(),
            answers: answers.clone(),
            handle: handle.clone(),
            readiness: readiness.clone(),
        }),
        Box::new(Dependency),
    ];
    if reversed {
        modules.reverse();
    }
    let mut engine = Engine::with_modules(World::new_minimal(), Registry::new(), modules)?;
    *readiness.lock().unwrap() = engine.registry.mechanics_owner_readiness.clone();
    let family = family.lock().unwrap().clone().unwrap();
    let mut txn = engine.action_transaction(&family).unwrap();
    txn.insert(&family, &1, 42).unwrap();
    let (_, install) = txn.prepare().unwrap().into_parts();
    engine.install_prepared_action(install).unwrap();
    let handle = handle.lock().unwrap().clone().unwrap();
    Ok(Fixture {
        engine,
        family,
        calls,
        mode,
        answers,
        handle,
    })
}
impl Fixture {
    fn read(&mut self, mode: usize, query: bool) -> Result<Value, OwnerReadError> {
        self.mode.store(mode, Ordering::SeqCst);
        let inbound = if query {
            Inbound::Query {
                conn: 1,
                actor: GOD,
                id: 1,
                name: "private.read".into(),
                args: json!({"actor":999}),
            }
        } else {
            Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "serviceread".into(),
            }
        };
        self.engine.step(
            inbound,
            &mut Outbox::new(),
            &mut crate::engine::Effects::new(),
        );
        self.answers.lock().unwrap().pop().unwrap()
    }
}
#[test]
fn real_commands_and_queries_keep_actual_caller_and_read_owner_truth_in_both_orders() {
    for reversed in [false, true] {
        let mut f = fixture(reversed, true).unwrap();
        for query in [false, true] {
            assert_eq!(
                f.read(0, query).unwrap(),
                json!({"actor":GOD,"principal":PrincipalId::Object {id:GOD},"caller":"consumer","value":42,"slot":9})
            );
        }
        assert_eq!(f.calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            f.read(7, false).unwrap(),
            json!({"actor":GOD,"principal":PrincipalId::Object {id:GOD},"caller":"consumer","value":42,"slot":9})
        );
        for mode in [11, 12] {
            assert_eq!(
                f.read(mode, false).unwrap(),
                json!({"actor":GOD,"principal":PrincipalId::Object {id:GOD},"caller":"consumer","value":42,"slot":9})
            );
        }
        assert_eq!(f.engine.action_state().len(), 1);
    }
}
#[test]
fn request_result_and_lookup_budgets_refuse_without_writes() {
    for mode in [1, 2, 3, 6, 8, 9] {
        let mut f = fixture(false, true).unwrap();
        assert_eq!(f.read(mode, false), Err(OwnerReadError::Limit));
        assert_eq!(f.calls.load(Ordering::SeqCst), usize::from(mode != 6));
        assert_eq!(f.engine.action_state().len(), 1);
        assert!(!f.engine.action_state().has_prepared());
    }
}
#[test]
fn owner_and_caller_freezes_during_callback_refuse_the_result() {
    for mode in [4, 5, 10] {
        let mut f = fixture(false, true).unwrap();
        assert_eq!(f.read(mode, false), Err(OwnerReadError::Unavailable));
        assert_eq!(f.read(0, true), Err(OwnerReadError::Unavailable));
        assert_eq!(f.calls.load(Ordering::SeqCst), 1);
    }
}
#[test]
fn module_serializers_cannot_cross_the_readiness_boundary() {
    for query in [false, true] {
        for mode in [13, 14, 15, 16, 17, 18] {
            let mut f = fixture(false, true).unwrap();
            assert_eq!(f.read(mode, query), Err(OwnerReadError::Unavailable));
            let expected = usize::from(matches!(mode, 15..=17));
            assert_eq!(
                f.calls.load(Ordering::SeqCst),
                expected,
                "request serialization must refuse before read(); result serialization must not escape"
            );
            assert_eq!(f.read(0, !query), Err(OwnerReadError::Unavailable));
            assert_eq!(f.calls.load(Ordering::SeqCst), expected);
            assert_eq!(f.engine.action_state().len(), 1);
            assert!(!f.engine.action_state().has_prepared());
        }
    }
}
#[test]
fn unauthorized_registration_foreign_handle_detached_lens_and_missing_owner_refuse() {
    assert!(fixture(false, false).is_err());
    let f = fixture(false, true).unwrap();
    let other = fixture(false, true).unwrap();
    let caller = OwnerReadCaller::object("consumer", GOD).unwrap();
    let request = || Request {
        mode: 0,
        pretend_actor: GOD,
        text: String::new(),
        readiness: None,
    };
    let registry = f.engine.registry.owner_reads.as_ref().unwrap();
    assert_eq!(
        registry.call(
            &f.engine.world,
            TypedStateLens::Live(f.engine.action_state()),
            &caller,
            &other.handle,
            request()
        ),
        Err(OwnerReadError::Authority)
    );
    assert_eq!(
        registry.call(
            &f.engine.world,
            TypedStateLens::Detached(crate::action_state::DetachedTypedState::CompatibilityCtx),
            &caller,
            &f.handle,
            request()
        ),
        Err(OwnerReadError::Unavailable)
    );
    let wrong_caller = OwnerReadCaller::object("owner", GOD).unwrap();
    assert_eq!(
        registry.call(
            &f.engine.world,
            TypedStateLens::Live(f.engine.action_state()),
            &wrong_caller,
            &f.handle,
            request()
        ),
        Err(OwnerReadError::Authority)
    );
    // The real frozen registry cannot reconstruct an absent owner installation.
    let absent = World::new_minimal();
    assert_eq!(
        registry.call(
            &absent,
            TypedStateLens::Live(f.engine.action_state()),
            &caller,
            &f.handle,
            request()
        ),
        Err(OwnerReadError::Integrity)
    );
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
}

#[path = "prepare_tests.rs"]
mod preparation;

#[path = "batch_tests.rs"]
mod batch;

#[path = "registration_tests.rs"]
mod registration;

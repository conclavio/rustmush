#![cfg(test)]

//! Actual Core/Engine exchange with two registered inert native fixture owners.
//! This proves atomic mechanics, not a real certificate, payment or unit refit.
use super::*;
use crate::action::{
    ActionError, ActionErrorCode, ActionInvocationDraft, ActionOfferId, ActionProvider,
    AuthoritativeActionOutput, CanonicalHash, IdempotencyKey, IdempotencyScope,
    PreparedAuthoritativeAction, PreparedCausalRef, RequestUid, ScopedIdempotencyKey,
};
use crate::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft, AuthoritativeKey};
use crate::composed::PreparedComposedTxn;
use crate::core_output::OutputRecheckContext;
use crate::mechanics::{DefinitionValidationContext, RulesValue};
use crate::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantHandle,
};
use crate::read_service::{OwnerReadContext, OwnerReadError};
use crate::{Engine, GOD, GameModule, Registry};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

type Trace = Arc<Mutex<Vec<(u8, &'static str)>>>;
#[derive(Clone, Debug, PartialEq, Eq)]
struct Cell<const N: u8>(u64);
struct Adapter<const N: u8> {
    trace: Trace,
}
struct Patch {
    refuse: bool,
}
struct Bound<'a, const N: u8> {
    cell: &'a mut Cell<N>,
    trace: Trace,
}
fn owner<const N: u8>() -> &'static str {
    if N == 1 { "economy" } else { "tactics" }
}
fn owner_descriptor<const N: u8>() -> DefinitionDescriptor {
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_participant").unwrap(),
        DefinitionId::new(match N {
            1 => "initial_filing",
            2 => "component_replacement",
            _ => "extra_replacement_fixture",
        })
        .unwrap(),
        1,
        json!({"fixture":true}),
    )
    .unwrap()
}
fn owner_ref<const N: u8>() -> DefinitionRef {
    crate::DefinitionRegistryBuilder::new()
        .register(OwnerId::new(owner::<N>()).unwrap(), owner_descriptor::<N>())
        .unwrap()
}
fn action_ref() -> DefinitionRef {
    crate::DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("industry").unwrap(),
            action_descriptor().unwrap(),
        )
        .unwrap()
}
impl<const N: u8> BoundParticipant for Bound<'_, N> {
    fn apply(self) {
        self.trace.lock().unwrap().push((N, "apply"));
        self.cell.0 = 1;
    }
}
impl<const N: u8> OwnerParticipant for Adapter<N> {
    type State = Cell<N>;
    type Request = Patch;
    type Output = ();
    type Patch = Patch;
    type Bound<'a> = Bound<'a, N>;
    fn prepare(
        &self,
        state: &Cell<N>,
        ctx: &OwnerPrepareContext<'_>,
        patch: Patch,
    ) -> Result<OwnerPreparation<(), Patch>, ParticipantError> {
        if state.0 != 0 || ctx.action() != &action_ref() {
            return Err(refused());
        }
        Ok(OwnerPreparation::new(
            (),
            patch,
            vec![ReplayWrite::ModuleJsonSet {
                namespace: OwnerId::new(owner::<N>()).unwrap(),
                key: AuthoritativeKey::parse_canonical(&format!("fixture_slot_{N}")).unwrap(),
                value: json!(1),
            }],
        ))
    }
    fn recheck(
        &self,
        state: &Cell<N>,
        _: &OwnerPrepareContext<'_>,
        _: &Patch,
    ) -> Result<(), ParticipantError> {
        if state.0 == 0 { Ok(()) } else { Err(refused()) }
    }
    fn bind<'a>(
        self: Arc<Self>,
        cell: &'a mut Cell<N>,
        patch: Patch,
    ) -> Result<Bound<'a, N>, ParticipantError> {
        self.trace.lock().unwrap().push((N, "bind"));
        if cell.0 != 0 || patch.refuse {
            return Err(refused());
        }
        Ok(Bound {
            cell,
            trace: Arc::clone(&self.trace),
        })
    }
    fn accepts_write(&self, write: &ReplayWrite) -> bool {
        matches!(write,ReplayWrite::ModuleJsonSet{namespace,key,value} if namespace.as_str()==owner::<N>() && key.as_str()==format!("fixture_slot_{N}") && value==&json!(1))
    }
}
#[derive(Clone)]
struct OwnerState(Arc<Mutex<(ExchangeFacts, bool)>>);
struct Policy;
impl CoreExchangePolicy for Policy {
    type State = OwnerState;
    fn generation(&self, state: &OwnerState) -> u64 {
        state.0.lock().unwrap().0.generation()
    }
    fn read(
        &self,
        state: &OwnerState,
        ctx: &OwnerReadContext<'_>,
        request: &ExchangeRequest,
    ) -> Result<ExchangeFacts, OwnerReadError> {
        let data = state.0.lock().unwrap();
        if !data.1 || data.0.order() != request.order() || data.0.requester() != ctx.actor() {
            return Err(OwnerReadError::Refused);
        }
        Ok(data.0.clone())
    }
    fn recheck(
        &self,
        state: &OwnerState,
        ctx: &OutputRecheckContext<'_>,
        request: &ExchangeRequest,
        facts: &ExchangeFacts,
    ) -> Result<(), OwnerReadError> {
        let data = state.0.lock().unwrap();
        if !data.1
            || &data.0 != facts
            || request.order() != facts.order()
            || ctx.actor() != facts.requester()
            || ctx.action() != &action_ref()
        {
            return Err(OwnerReadError::Refused);
        }
        Ok(())
    }
}
fn validators(
    reg: &mut Registrar<'_>,
    kind: &str,
    expected: Vec<DefinitionDescriptor>,
) -> Result<(), ModError> {
    reg.definition_validator(
        DefinitionKind::new(kind).unwrap(),
        move |ctx: &DefinitionValidationContext<'_>| {
            let e = ctx.definition();
            if expected.iter().any(|d| {
                e.reference().key().id == *d.id()
                    && e.reference().version() == d.version()
                    && e.payload() == d.payload()
                    && {
                        // Registration sorts references before validation.
                        let mut references = d.references().to_vec();
                        references.sort();
                        e.references() == references.as_slice()
                    }
            }) {
                Ok(())
            } else {
                Err("unexpected exchange fixture descriptor".into())
            }
        },
    )
}
struct Native<const N: u8> {
    trace: Trace,
}
impl<const N: u8> GameModule for Native<N> {
    fn name(&self) -> &'static str {
        owner::<N>()
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let mut descriptors = vec![owner_descriptor::<N>()];
        if N == 2 {
            descriptors.push(owner_descriptor::<3>());
        }
        validators(reg, "owner_participant", descriptors)?;
        reg.state(Cell::<N>(0));
        let reference = reg.definition(owner_descriptor::<N>())?;
        reg.owner_participant(
            reference,
            BTreeSet::from([action_ref()]),
            Adapter::<N> {
                trace: Arc::clone(&self.trace),
            },
        )?;
        if N == 2 {
            reg.state(Cell::<3>(0));
            let reference = reg.definition(owner_descriptor::<3>())?;
            reg.owner_participant(
                reference,
                BTreeSet::from([action_ref()]),
                Adapter::<3> {
                    trace: Arc::clone(&self.trace),
                },
            )?;
        }
        Ok(())
    }
}
#[derive(Clone)]
struct ExpectedExchange {
    facts: ExchangeFacts,
    action: DefinitionRef,
    commit: CommitToken,
    participants: [DefinitionRef; 2],
}
impl ExpectedExchange {
    fn matches(&self, a: &ExchangeCarryAttempt) -> bool {
        a.order() == self.facts.order()
            && a.requester() == self.facts.requester()
            && a.hull() == self.facts.hull()
            && a.unit_revision() == self.facts.unit_revision()
            && a.incoming() == self.facts.incoming()
            && a.source() == self.facts.source()
            && a.displaced() == self.facts.displaced()
            && a.action() == &self.action
            && a.commit() == self.commit
            && a.participants() == &self.participants
    }
}
struct CarryHook {
    records: StateFamilyHandle<u64, ExchangeRecord>,
    veto: AtomicU32,
    seen: Mutex<Vec<(ObjId, ObjId, bool)>>,
    expected_exchange: Mutex<Option<ExpectedExchange>>,
    exchange_calls: AtomicU32,
    ordinary_calls: AtomicU32,
}
impl crate::EventHook for CarryHook {
    fn carry_attempt(
        &self,
        ctx: &crate::HookReadContext<'_>,
        _: ObjId,
        object: ObjId,
        destination: ObjId,
    ) -> Result<(), mush_proto::RichText> {
        if let Some(attempt) = ctx.core_exchange_attempt() {
            self.exchange_calls.fetch_add(1, Ordering::SeqCst);
            if self
                .expected_exchange
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|expected| !expected.matches(attempt))
            {
                return Err(mush_proto::RichText::plain(
                    "closed exchange context differs",
                ));
            }
        } else {
            self.ordinary_calls.fetch_add(1, Ordering::SeqCst);
            if self.expected_exchange.lock().unwrap().is_some() {
                return Err(mush_proto::RichText::plain(
                    "closed exchange context required",
                ));
            }
        }
        let committed = ctx
            .action_state(&self.records, &7)
            .is_ok_and(|r| r.is_none());
        self.seen
            .lock()
            .unwrap()
            .push((object, destination, committed));
        if !committed || self.veto.load(Ordering::SeqCst) == object.0 {
            Err(mush_proto::RichText::plain("exchange carry refused"))
        } else {
            Ok(())
        }
    }
}
#[derive(Clone)]
struct Handles {
    service: CoreComponentExchanges<Policy>,
    action: DefinitionRef,
    hook: Arc<CarryHook>,
}
struct Industry {
    data: OwnerState,
    handles: Arc<Mutex<Option<Handles>>>,
}
impl GameModule for Industry {
    fn name(&self) -> &'static str {
        "industry"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        validators(reg, "action", vec![action_descriptor()?])?;
        let outcome = DefinitionDescriptor::new(
            DefinitionKind::new("outcome").unwrap(),
            DefinitionId::new("exchange_fixture").unwrap(),
            1,
            json!({}),
        )
        .unwrap();
        validators(reg, "outcome", vec![outcome.clone()])?;
        let participants = [owner_ref::<1>(), owner_ref::<2>()];
        let action = reg.definition(action_descriptor()?)?;
        validators(
            reg,
            "owner_read",
            vec![CoreComponentExchanges::<Policy>::policy_descriptor(
                &action,
                &participants,
            )?],
        )?;
        let outcome = reg.definition(outcome)?;
        reg.state(self.data.clone());
        let service = CoreComponentExchanges::register(
            reg,
            action.clone(),
            participants,
            Policy,
            BTreeSet::new(),
        )?;
        let hook = Arc::new(CarryHook {
            records: service.records.clone(),
            veto: AtomicU32::new(u32::MAX),
            seen: Mutex::new(Vec::new()),
            expected_exchange: Mutex::new(None),
            exchange_calls: AtomicU32::new(0),
            ordinary_calls: AtomicU32::new(0),
        });
        reg.hook(crate::HookPoint::CarryAttempt, hook.clone());
        let first = reg.request_owner_participant::<Adapter<1>>(action.clone())?;
        let second = reg.request_owner_participant::<Adapter<2>>(action.clone())?;
        let extra = reg.request_owner_participant::<Adapter<3>>(action.clone())?;
        reg.action_provider(
            action.clone(),
            Provider {
                service: service.clone(),
                outcome,
                first,
                second,
                extra,
            },
        )?;
        *self.handles.lock().unwrap() = Some(Handles {
            service,
            action,
            hook,
        });
        Ok(())
    }
    fn finalize_owner_composition(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        self.handles
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .service
            .audit(ctx, |_| Ok(()))
            .map_err(bad)
    }
}
struct Provider {
    service: CoreComponentExchanges<Policy>,
    outcome: DefinitionRef,
    first: ParticipantHandle<Adapter<1>>,
    second: ParticipantHandle<Adapter<2>>,
    extra: ParticipantHandle<Adapter<3>>,
}
fn fail() -> ActionError {
    ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        "exchange fixture refused",
    )
    .unwrap()
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        if inv.parameters().get("order") == Some(&json!(7)) {
            Ok(())
        } else {
            Err(fail())
        }
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut crate::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let flag = |name| inv.parameters().get(name) == Some(&json!(true));
        if !flag("missing_owner") {
            ctx.prepare_owner(
                &self.first,
                Patch {
                    refuse: flag("refuse_first"),
                },
            )
            .map_err(|_| fail())?;
        }
        ctx.prepare_owner(
            &self.second,
            Patch {
                refuse: flag("refuse_second"),
            },
        )
        .map_err(|_| fail())?;
        let result =
            ctx.exchange_core_component(&self.service, txn, if flag("caught") { 8 } else { 7 });
        let value = if flag("caught") {
            assert!(result.is_err());
            json!({"caught":true})
        } else {
            let actual = result.map_err(|_| fail())?;
            json!({"operation":actual.commit(),"receipt":actual.digest().map_err(|_|fail())?})
        };
        if flag("extra_owner") {
            ctx.prepare_owner(&self.extra, Patch { refuse: false })
                .map_err(|_| fail())?;
        }
        Ok(
            AuthoritativeActionOutput::new(RulesValue::new(self.outcome.clone(), value).unwrap())
                .with_definitions(vec![self.outcome.clone()])
                .with_eligible_witnesses(vec![inv.principal().clone()]),
        )
    }
}
type Prepared = (PreparedComposedTxn, PreparedAuthoritativeAction);
struct Rig {
    engine: Engine,
    handles: Handles,
    data: OwnerState,
    trace: Trace,
    actor: ObjId,
    hull: ObjId,
    incoming: ObjId,
    outgoing: Option<ObjId>,
    source: ObjId,
    return_to: Option<ObjId>,
    room: ObjId,
}
fn rig(replace: bool, separate_return: bool) -> Rig {
    let mut world = World::new_minimal();
    world.set_now(100);
    let room = world
        .create(ObjKind::Room, "Refit room", GOD, None)
        .unwrap();
    let actor = world
        .create(ObjKind::Player, "Refit customer", GOD, Some(room))
        .unwrap();
    let hull = world
        .create(ObjKind::Thing, "Hull", actor, Some(room))
        .unwrap();
    let source = actor;
    let incoming = world
        .create(ObjKind::Thing, "Incoming core", actor, Some(source))
        .unwrap();
    let outgoing = replace.then(|| {
        world
            .create(ObjKind::Thing, "Displaced core", actor, Some(hull))
            .unwrap()
    });
    let return_to = replace.then(|| {
        if separate_return {
            world
                .create(ObjKind::Thing, "Return container", actor, Some(room))
                .unwrap()
        } else {
            source
        }
    });
    world.take_dirty();
    let facts = ExchangeFacts::new(
        1,
        7,
        actor,
        hull,
        2,
        incoming,
        source,
        outgoing.zip(return_to),
    )
    .unwrap();
    let data = OwnerState(Arc::new(Mutex::new((facts, true))));
    let trace = Arc::new(Mutex::new(Vec::new()));
    let handles = Arc::new(Mutex::new(None));
    let engine = Engine::with_modules(
        world,
        Registry::new(),
        vec![
            Box::new(Industry {
                data: data.clone(),
                handles: Arc::clone(&handles),
            }),
            Box::new(Native::<1> {
                trace: Arc::clone(&trace),
            }),
            Box::new(Native::<2> {
                trace: Arc::clone(&trace),
            }),
        ],
    )
    .unwrap();
    let handles = handles.lock().unwrap().clone().unwrap();
    Rig {
        engine,
        handles,
        data,
        trace,
        actor,
        hull,
        incoming,
        outgoing,
        source,
        return_to,
        room,
    }
}
fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
impl Rig {
    fn invocation(&self, flags: &[&str]) -> ActionInvocation {
        let request = RequestUid::from_bytes([7; 16]);
        let principal = PrincipalId::Object { id: self.actor };
        let mut parameters = json!({"order":7});
        for flag in flags {
            parameters[*flag] = json!(true);
        }
        ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor: self.actor,
            offered_action: ActionOfferId(CanonicalHash::from_bytes([2; 32])),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: OwnerId::new("industry").unwrap(),
                    namespace: CanonicalId::new("core_exchange_fixture").unwrap(),
                },
                1,
                IdempotencyKey::Request(request),
            ),
        })
        .unwrap()
    }
    fn prepare(&mut self, flags: &[&str]) -> Result<Prepared, String> {
        let inv = self.invocation(flags);
        self.engine
            .prepare_registered_composed_authoritative_action(&self.handles.action, &inv, token(7))
            .map_err(|e| e.to_string())
    }
    fn objects(&self) -> BTreeMap<ObjId, Obj> {
        [
            Some(self.actor),
            Some(self.hull),
            Some(self.incoming),
            Some(self.source),
            self.outgoing,
            self.return_to,
            Some(self.room),
        ]
        .into_iter()
        .flatten()
        .map(|id| (id, self.engine.world.get(id).unwrap().clone()))
        .collect()
    }
    fn install(&mut self, prepared: Prepared) -> Result<Vec<ReplayWrite>, String> {
        let (writes, installation) = prepared.0.into_parts();
        self.engine
            .install_composed_action(installation)
            .map_err(|e| e.to_string())?;
        Ok(writes)
    }
    fn unchanged_owners(&self) {
        assert_eq!(self.engine.world.module_state::<Cell<1>>(), Some(&Cell(0)));
        assert_eq!(self.engine.world.module_state::<Cell<2>>(), Some(&Cell(0)));
    }
    fn stage(&mut self, p: Prepared, flags: &[&str]) -> Result<(), String> {
        let inv = self.invocation(flags);
        let (
            outcome,
            resolution,
            mut definitions,
            eligible_witnesses,
            durable_outbox,
            semantic_events,
        ) = p.1.into_parts();
        definitions.push(self.handles.action.clone());
        definitions.sort();
        definitions.dedup();
        let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: token(7),
            request: inv.request(),
            idempotency: inv.idempotency().clone(),
            ingress_hash: CanonicalHash::from_bytes([1; 32]),
            request_hash: inv.request_hash(),
            plan_hash: CanonicalHash::from_bytes([3; 32]),
            action: self.handles.action.clone(),
            provider: OwnerId::new("industry").unwrap(),
            cause: PreparedCausalRef::new(inv.request(), None, 0).unwrap(),
            definitions,
            writes: p.0.authoritative_replay_writes(),
            outcome,
            resolution: resolution.map(|r| r.bind(token(7))),
            semantic_events,
            eligible_witnesses,
            durable_outbox,
        })
        .map_err(|e| e.to_string())?;
        let guards = self
            .engine
            .registry
            .server_guard_set_builder(crate::RevisionSet::default())
            .unwrap()
            .finish()
            .unwrap();
        self.engine.world.take_dirty();
        self.engine
            .stage_authoritative_action(p.0, guards, bundle)
            .map_err(|e| e.to_string())
    }
}

mod cases;

mod carry_context_tests;

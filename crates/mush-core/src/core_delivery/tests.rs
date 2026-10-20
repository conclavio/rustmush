#![cfg(test)]

//! Actual Engine delivery tests with a registered Industry fixture policy.
//! Real Core birth/consumption/carry hooks; no certificate, payment or title claim.
use super::*;
use crate::action::{
    ActionError, ActionErrorCode, ActionInvocationDraft, ActionOfferId, ActionProvider,
    AuthoritativeActionOutput, CanonicalHash, IdempotencyKey, IdempotencyScope,
    PreparedAuthoritativeAction, PreparedCausalRef, RequestUid, ScopedIdempotencyKey,
};
use crate::action_state::ActionStateBootRecord;
use crate::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft, ReplayWrite};
use crate::composed::PreparedComposedTxn;
use crate::core_output::records::{OutputRecord, OutputState};
use crate::core_output::{
    COMPLETE_ACTION, CoreOutputBinding, CoreOutputOrder, INITIALIZE_ACTION, OutputPhase,
    OutputPolicyFacts, OutputPolicyRequest, OutputRecheckContext, RESERVE_ACTION,
};
use crate::mechanics::{DefinitionValidationContext, RulesValue};
use crate::read_service::{OwnerReadContext, OwnerReadError};
use crate::{DefinitionDigest, Engine, GOD, GameModule, Outbox, Registry};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Default)]
struct OwnerData {
    generation: u64,
    order: Option<CoreOutputOrder>,
    allowed: bool,
}
#[derive(Clone)]
struct OwnerState(Arc<Mutex<OwnerData>>);
struct Policy;
impl CoreOutputPolicy for Policy {
    type State = OwnerState;
    fn generation(&self, s: &OwnerState) -> u64 {
        s.0.lock().unwrap().generation
    }
    fn read(
        &self,
        s: &OwnerState,
        ctx: &OwnerReadContext<'_>,
        request: &OutputPolicyRequest,
    ) -> Result<OutputPolicyFacts, OwnerReadError> {
        let s = s.0.lock().unwrap();
        if request.phase() == OutputPhase::Initialize {
            if s.order.is_some() || ctx.actor() != GOD {
                return Err(OwnerReadError::Refused);
            }
            return Ok(OutputPolicyFacts::empty(s.generation));
        }
        let order = s.order.as_ref().ok_or(OwnerReadError::Refused)?;
        if !s.allowed
            || Some(order.id()) != request.order()
            || order.requester() != ctx.actor()
            || ctx
                .world()
                .get(order.container())
                .is_none_or(|c| c.contents.len() >= 8)
        {
            return Err(OwnerReadError::Refused);
        }
        Ok(OutputPolicyFacts::order(s.generation, order.clone()))
    }
    fn recheck(
        &self,
        s: &OwnerState,
        ctx: &OutputRecheckContext<'_>,
        request: &OutputPolicyRequest,
        facts: &OutputPolicyFacts,
    ) -> Result<(), OwnerReadError> {
        let s = s.0.lock().unwrap();
        if s.generation != facts.generation() {
            return Err(OwnerReadError::Integrity);
        }
        if request.phase() == OutputPhase::Initialize {
            return if s.order.is_none() && ctx.actor() == GOD {
                Ok(())
            } else {
                Err(OwnerReadError::Refused)
            };
        }
        if !s.allowed
            || s.order.as_ref() != facts.order_facts()
            || facts.order_facts().is_none_or(|o| {
                o.requester() != ctx.actor()
                    || ctx
                        .world()
                        .get(o.container())
                        .is_none_or(|c| c.contents.len() >= 8)
            })
        {
            return Err(OwnerReadError::Refused);
        }
        Ok(())
    }
}
#[derive(Clone)]
struct Handles {
    service: CoreOutputReservations<Policy>,
    actions: [DefinitionRef; 4],
    delivery: CoreOutputDeliveries<Policy, DeliveryPolicy>,
    gate: StateFamilyHandle<(), bool>,
    hook: Arc<CarryHook>,
    wrong: DefinitionRef,
    recipe: DefinitionRef,
}
struct Module {
    handles: Arc<Mutex<Option<Handles>>>,
    data: Arc<Mutex<OwnerData>>,
    audit: bool,
}
fn simple(reg: &mut Registrar<'_>, kind: &str, id: &str) -> Result<DefinitionRef, ModError> {
    reg.definition(
        DefinitionDescriptor::new(
            DefinitionKind::new(kind).unwrap(),
            DefinitionId::new(id).unwrap(),
            1,
            json!({}),
        )
        .unwrap(),
    )
}
impl GameModule for Module {
    fn name(&self) -> &'static str {
        "industry"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        for (kind, ids) in [
            (
                "action",
                vec![
                    INITIALIZE_ACTION,
                    RESERVE_ACTION,
                    COMPLETE_ACTION,
                    ACTION,
                    "wrong_core_output",
                ],
            ),
            ("recipe", vec!["core_integrate"]),
            ("outcome", vec!["core_output_fixture"]),
        ] {
            reg.definition_validator(
                DefinitionKind::new(kind).unwrap(),
                move |ctx: &DefinitionValidationContext<'_>| {
                    let e = ctx.definition();
                    if e.reference().version() == 1
                        && ids.contains(&e.reference().key().id.as_str())
                        && e.payload() == &json!({})
                        && e.references().is_empty()
                    {
                        Ok(())
                    } else {
                        Err("unexpected fixture definition".into())
                    }
                },
            )?;
        }
        let actions = [
            simple(reg, "action", INITIALIZE_ACTION)?,
            simple(reg, "action", RESERVE_ACTION)?,
            simple(reg, "action", COMPLETE_ACTION)?,
            simple(reg, "action", ACTION)?,
        ];
        let wrong = simple(reg, "action", "wrong_core_output")?;
        let outcome = simple(reg, "outcome", "core_output_fixture")?;
        let recipe = simple(reg, "recipe", "core_integrate")?;
        let binding = CoreOutputBinding::new(
            [actions[0].clone(), actions[1].clone(), actions[2].clone()],
            recipe.clone(),
        )?;
        let expected = [
            binding.policy_descriptor()?,
            CoreOutputDeliveries::<Policy, DeliveryPolicy>::policy_descriptor(&actions[3])?,
        ];
        reg.definition_validator(
            DefinitionKind::new("owner_read").unwrap(),
            move |ctx: &DefinitionValidationContext<'_>| {
                let e = ctx.definition();
                if expected.iter().any(|expected| {
                    e.reference().key().id == *expected.id()
                        && e.reference().version() == expected.version()
                        && e.payload() == expected.payload()
                        && e.references() == expected.references()
                }) {
                    Ok(())
                } else {
                    Err("unexpected output policy".into())
                }
            },
        )?;
        reg.state(OwnerState(Arc::clone(&self.data)));
        let service = CoreOutputReservations::register(reg, binding, Policy, BTreeSet::new())?;
        let delivery = CoreOutputDeliveries::register(
            reg,
            service.clone(),
            actions[3].clone(),
            DeliveryPolicy {
                outputs: service.clone(),
            },
            BTreeSet::new(),
        )?;
        let gate = reg.state_family(
            crate::core_output::descriptor("delivery_fixture_gate")?,
            |_: &()| Ok("gate".to_owned()),
            |v: &Value| v.as_bool().ok_or_else(|| "bool".into()),
            |_: &bool| Ok(()),
            |v: &bool| Ok(json!(v)),
        )?;
        let hook = Arc::new(CarryHook {
            gate: gate.clone(),
            veto: AtomicBool::new(false),
            seen: Mutex::new(Vec::new()),
        });
        reg.hook(crate::HookPoint::CarryAttempt, hook.clone());
        for (i, action) in actions.iter().enumerate() {
            reg.action_provider(
                action.clone(),
                Provider {
                    service: service.clone(),
                    delivery: delivery.clone(),
                    gate: gate.clone(),
                    mode: i,
                    outcome: outcome.clone(),
                },
            )?;
        }
        reg.action_provider(
            wrong.clone(),
            Provider {
                service: service.clone(),
                delivery: delivery.clone(),
                gate: gate.clone(),
                mode: 3,
                outcome,
            },
        )?;
        *self.handles.lock().unwrap() = Some(Handles {
            service,
            delivery,
            gate,
            hook,
            actions,
            wrong,
            recipe,
        });
        Ok(())
    }
    fn finalize_owner_composition(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        if self.audit {
            let handles = self.handles.lock().unwrap();
            handles
                .as_ref()
                .unwrap()
                .delivery
                .audit(ctx, |_| Ok(()), |_| Ok(()))
                .map_err(error)?;
        }
        Ok(())
    }
}
struct Provider {
    service: CoreOutputReservations<Policy>,
    delivery: CoreOutputDeliveries<Policy, DeliveryPolicy>,
    gate: StateFamilyHandle<(), bool>,
    mode: usize,
    outcome: DefinitionRef,
}
fn failed() -> ActionError {
    ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        "Core output fixture refused",
    )
    .unwrap()
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        let expected = if self.mode == 0 {
            json!({})
        } else {
            json!({"order":7})
        };
        if inv.parameters() != &expected
            && !(self.mode == 3 && inv.parameters() == &json!({"order":7,"stage_gate":false}))
            && !(self.mode == 3 && inv.parameters() == &json!({"order":7,"catch_refusal":true}))
        {
            return Err(failed());
        }
        Ok(())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut crate::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(inv)?;
        let result = match self.mode {
            0 => {
                ctx.initialize_core_outputs(&self.service, txn)
                    .map_err(|_| failed())?;
                txn.insert(&self.gate, &(), true).map_err(|_| failed())?;
                json!({"initialized":true})
            }
            1 => {
                let r = ctx
                    .reserve_core_output(&self.service, txn, 7)
                    .map_err(|_| failed())?;
                json!({"order":r.order().id(),"object":r.object(),"reservation":r.reservation()})
            }
            2 => {
                let r = ctx
                    .consume_core_output(&self.service, txn, 7)
                    .map_err(|_| failed())?;
                json!({"order":r.order().id(),"object":r.object(),"reservation":r.reservation()})
            }
            _ => {
                if inv.parameters().get("stage_gate") == Some(&json!(false)) {
                    txn.replace(&self.gate, &(), false).map_err(|_| failed())?;
                }
                if inv.parameters().get("catch_refusal") == Some(&json!(true)) {
                    assert!(ctx.deliver_core_output(&self.delivery, txn, 8).is_err());
                    json!({"caught":true})
                } else {
                    let r = ctx
                        .deliver_core_output(&self.delivery, txn, 7)
                        .map_err(|_| failed())?;
                    json!({"order":r.order().id(),"object":r.object(),"delivery":r.digest().map_err(|_|failed())?})
                }
            }
        };
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(self.outcome.clone(), result).map_err(|_| failed())?,
        )
        .with_definitions(vec![self.outcome.clone()])
        .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
type Prepared = (PreparedComposedTxn, PreparedAuthoritativeAction);
struct Rig {
    engine: Engine,
    handles: Handles,
    data: Arc<Mutex<OwnerData>>,
    container: ObjId,
    facility: ObjId,
    actor: ObjId,
    disk: BTreeMap<String, Value>,
}
fn engine(world: World, data: Arc<Mutex<OwnerData>>, audit: bool) -> (Engine, Handles) {
    let handles = Arc::new(Mutex::new(None));
    let engine = Engine::with_modules(
        world,
        Registry::new(),
        vec![Box::new(Module {
            handles: Arc::clone(&handles),
            data,
            audit,
        })],
    )
    .unwrap();
    let h = handles.lock().unwrap().clone().unwrap();
    (engine, h)
}
fn bare() -> Rig {
    let mut world = World::new_minimal();
    world.set_now(1234);
    let actor = world
        .create(ObjKind::Player, "Core requester", GOD, None)
        .unwrap();
    let facility = world
        .create(ObjKind::Room, "Trusted factory", GOD, None)
        .unwrap();
    let container = world
        .create(ObjKind::Thing, "Factory outputs", GOD, Some(facility))
        .unwrap();
    world.take_dirty();
    let data = Arc::new(Mutex::new(OwnerData::default()));
    let (engine, handles) = engine(world, Arc::clone(&data), false);
    Rig {
        engine,
        handles,
        data,
        container,
        facility,
        actor,
        disk: BTreeMap::new(),
    }
}
fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn inv(actor: ObjId, n: u8, init: bool) -> ActionInvocation {
    let request = RequestUid::from_bytes([n; 16]);
    let principal = PrincipalId::Object { id: actor };
    ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([2; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: if init { json!({}) } else { json!({"order":7}) },
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new("industry").unwrap(),
                namespace: CanonicalId::new("core_output_fixture").unwrap(),
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })
    .unwrap()
}
impl Rig {
    fn prepare(&mut self, phase: usize, n: u8) -> Prepared {
        self.engine
            .prepare_registered_composed_authoritative_action(
                &self.handles.actions[phase],
                &inv(if phase == 0 { GOD } else { self.actor }, n, phase == 0),
                token(n),
            )
            .unwrap()
    }
    fn initialize(&mut self) {
        let p = self.prepare(0, 1);
        self.install(p.0);
    }
    fn accept(&mut self) {
        let mut d = self.data.lock().unwrap();
        d.generation += 1;
        d.allowed = true;
        d.order = Some(
            CoreOutputOrder::new(
                7,
                DefinitionDigest::from_bytes([4; 32]),
                self.actor,
                self.handles.recipe.clone(),
                self.facility,
                self.container,
            )
            .unwrap(),
        );
    }
    fn install(&mut self, p: PreparedComposedTxn) {
        let (writes, install) = p.into_parts();
        for w in writes {
            match w {
                ReplayWrite::ModuleJsonSet {
                    namespace,
                    key,
                    value,
                } => {
                    assert_eq!(namespace.as_str(), "industry");
                    self.disk.insert(key.to_string(), value);
                }
                ReplayWrite::ObjectSet { .. } | ReplayWrite::ObjectAllocatorSet { .. } => {}
                _ => panic!("unexpected write"),
            }
        }
        self.engine.install_composed_action(install).unwrap();
    }
    fn row(&self) -> &OutputRecord {
        self.engine
            .action_state()
            .get(&self.handles.service.records, &7)
            .unwrap()
            .unwrap()
    }
    fn stage(&mut self, phase: usize, n: u8, p: Prepared) -> Result<(), String> {
        let inv = inv(if phase == 0 { GOD } else { self.actor }, n, phase == 0);
        let (
            outcome,
            resolution,
            mut definitions,
            eligible_witnesses,
            durable_outbox,
            semantic_events,
        ) = p.1.into_parts();
        definitions.push(self.handles.actions[phase].clone());
        definitions.sort();
        definitions.dedup();
        let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: token(n),
            request: inv.request(),
            idempotency: inv.idempotency().clone(),
            ingress_hash: CanonicalHash::from_bytes([1; 32]),
            request_hash: inv.request_hash(),
            plan_hash: CanonicalHash::from_bytes([3; 32]),
            action: self.handles.actions[phase].clone(),
            provider: OwnerId::new("industry").unwrap(),
            cause: PreparedCausalRef::new(inv.request(), None, 0).unwrap(),
            definitions,
            writes: p.0.authoritative_replay_writes(),
            outcome,
            resolution: resolution.map(|r| r.bind(token(n))),
            semantic_events,
            eligible_witnesses,
            durable_outbox,
        })
        .unwrap();
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
    fn cold(&self, disk: &BTreeMap<String, Value>) -> Result<Engine, String> {
        let (mut e, _) = engine(
            self.engine.world.clone(),
            Arc::new(Mutex::new(self.data.lock().unwrap().clone())),
            true,
        );
        e.hydrate_action_state(
            disk.iter()
                .map(|(k, v)| ActionStateBootRecord::new("industry", k.clone(), v.clone())),
        )
        .map_err(|e| e.to_string())?;
        e.all_modules_loaded(&mut Outbox::new(), &mut crate::engine::Effects::new())
            .map_err(|e| e.to_string())?;
        Ok(e)
    }
}
fn ready() -> Rig {
    let mut r = bare();
    r.initialize();
    r.accept();
    let p = r.prepare(1, 2);
    r.install(p.0);
    let p = r.prepare(2, 3);
    r.install(p.0);
    r
}

struct DeliveryPolicy {
    outputs: CoreOutputReservations<Policy>,
}
impl CoreDeliveryPolicy for DeliveryPolicy {
    type State = OwnerState;
    fn generation(&self, s: &OwnerState) -> u64 {
        s.0.lock().unwrap().generation
    }
    fn read(
        &self,
        s: &OwnerState,
        ctx: &OwnerReadContext<'_>,
        request: &DeliveryRequest,
    ) -> Result<DeliveryFacts, OwnerReadError> {
        let s = s.0.lock().unwrap();
        let row = ctx
            .action_state(&self.outputs.records, &request.order())?
            .ok_or(OwnerReadError::Refused)?;
        let OutputState::Consumed { completion, .. } = &row.state else {
            return Err(OwnerReadError::Refused);
        };
        if !s.allowed
            || s.order.as_ref() != Some(&row.order)
            || ctx.actor() != row.order.requester()
        {
            return Err(OwnerReadError::Refused);
        }
        Ok(DeliveryFacts::new(
            s.generation,
            row.order.clone(),
            row.object,
            row.reservation,
            *completion,
        ))
    }
    fn recheck(
        &self,
        s: &OwnerState,
        ctx: &OutputRecheckContext<'_>,
        request: &DeliveryRequest,
        facts: &DeliveryFacts,
    ) -> Result<(), OwnerReadError> {
        let s = s.0.lock().unwrap();
        if !s.allowed
            || s.generation != facts.generation()
            || s.order.as_ref() != Some(facts.order())
            || request.order() != facts.order().id()
            || ctx.actor() != facts.order().requester()
            || !crate::core_output::fixed_action(ctx.action(), ACTION)
        {
            return Err(OwnerReadError::Refused);
        }
        Ok(())
    }
}
struct CarryHook {
    gate: StateFamilyHandle<(), bool>,
    veto: AtomicBool,
    seen: Mutex<Vec<bool>>,
}
impl crate::EventHook for CarryHook {
    fn carry_attempt(
        &self,
        ctx: &crate::HookReadContext<'_>,
        _: ObjId,
        _: ObjId,
        _: ObjId,
    ) -> Result<(), mush_proto::RichText> {
        let actual = ctx.action_state(&self.gate, &()).ok().flatten().copied();
        self.seen.lock().unwrap().push(actual == Some(true));
        if actual != Some(true) || self.veto.load(Ordering::SeqCst) {
            Err(mush_proto::RichText::plain("actual carry veto"))
        } else {
            Ok(())
        }
    }
}
fn held(r: &mut Rig, n: u8) -> crate::composed::ComposedInstallation {
    let p = r.prepare(3, n);
    r.stage(3, n, p).unwrap();
    let staged = r.engine.take_staged_action().unwrap().unwrap();
    let (bundle, install) = staged.into_parts();
    for w in bundle.writes() {
        if let ReplayWrite::ModuleJsonSet { key, value, .. } = w {
            r.disk.insert(key.to_string(), value.clone());
        }
    }
    install
}
#[test]
fn actual_delivery_is_inert_until_ack_then_moves_exact_three_objects_and_cold_joins() {
    let mut r = ready();
    let object = r.row().object;
    let birth = r.engine.world.get(object).unwrap().clone();
    let source = r.engine.world.get(r.container).unwrap().clone();
    let actor = r.engine.world.get(r.actor).unwrap().clone();
    let p = r.prepare(3, 10);
    assert_eq!(r.engine.world.get(object), Some(&birth));
    let writes = p.0.authoritative_replay_writes();
    assert_eq!(
        writes
            .iter()
            .filter(|w| matches!(w, ReplayWrite::ObjectSet { .. }))
            .count(),
        3
    );
    assert_eq!(
        writes
            .iter()
            .filter(|w| matches!(w, ReplayWrite::ModuleJsonSet { .. }))
            .count(),
        1
    );
    r.stage(3, 10, p).unwrap();
    let staged = r.engine.take_staged_action().unwrap().unwrap();
    assert_eq!(r.engine.world.get(object), Some(&birth));
    assert_eq!(r.engine.world.get(r.container), Some(&source));
    assert_eq!(r.engine.world.get(r.actor), Some(&actor));
    let (bundle, install) = staged.into_parts();
    for w in bundle.writes() {
        if let ReplayWrite::ModuleJsonSet { key, value, .. } = w {
            r.disk.insert(key.to_string(), value.clone());
        }
    }
    r.engine.install_composed_action(install).unwrap();
    assert_eq!(r.engine.world.get(object).unwrap().location, Some(r.actor));
    assert!(
        !r.engine
            .world
            .get(r.container)
            .unwrap()
            .contents
            .contains(&object)
    );
    assert_eq!(
        r.engine
            .world
            .get(r.actor)
            .unwrap()
            .contents
            .iter()
            .filter(|id| **id == object)
            .count(),
        1
    );
    assert!(r.handles.hook.seen.lock().unwrap().iter().all(|v| *v));
    assert!(r.cold(&r.disk).is_ok());
    // A later lawful ordinary move changes placement only, not the birth/certification identity.
    // Wall-clock correction is not a new authority or an immutable receipt edit.
    r.engine.world.set_now(1200);
    r.engine.world.move_to(object, r.container).unwrap();
    assert!(r.cold(&r.disk).is_ok());
}
#[test]
fn actual_hooks_read_committed_state_and_veto_prepare_stage_or_take() {
    for phase in 0..3 {
        let mut r = ready();
        let object = r.row().object;
        let before = r.engine.world.get(object).unwrap().clone();
        if phase == 0 {
            r.handles.hook.veto.store(true, Ordering::SeqCst);
            assert!(
                r.engine
                    .prepare_registered_composed_authoritative_action(
                        &r.handles.actions[3],
                        &inv(r.actor, 11, false),
                        token(11)
                    )
                    .is_err()
            );
        } else {
            let p = r.prepare(3, 11);
            if phase == 1 {
                r.handles.hook.veto.store(true, Ordering::SeqCst);
                assert!(r.stage(3, 11, p).is_err());
            } else {
                r.stage(3, 11, p).unwrap();
                r.handles.hook.veto.store(true, Ordering::SeqCst);
                assert!(r.engine.take_staged_action().is_err());
            }
        }
        assert_eq!(r.engine.world.get(object), Some(&before));
        assert!(
            r.engine
                .action_state()
                .get(&r.handles.delivery.records, &7)
                .unwrap()
                .is_none()
        );
        assert!(r.handles.hook.seen.lock().unwrap().iter().all(|v| *v));
    }
    let mut r = ready();
    let original = inv(r.actor, 12, false);
    // Build the invocation normally rather than mutating opaque authority; only
    // this test provider's ordinary request asks it to stage a false gate.
    let request = RequestUid::from_bytes([12; 16]);
    let staged = ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: PrincipalId::Object { id: r.actor },
        actor: r.actor,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([2; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: json!({"order":7,"stage_gate":false}),
        idempotency: original.idempotency().clone(),
    })
    .unwrap();
    let p = r
        .engine
        .prepare_registered_composed_authoritative_action(&r.handles.actions[3], &staged, token(12))
        .unwrap();
    assert!(r.handles.hook.seen.lock().unwrap().iter().all(|v| *v));
    assert_eq!(
        r.engine.action_state().get(&r.handles.gate, &()).unwrap(),
        Some(&true)
    );
    r.engine.abort_prepared_transaction(p.0);
}
#[test]
fn physical_or_owner_drift_refuses_before_custody_and_late_bind_never_partially_applies() {
    for phase in 0..3 {
        for target in 0..4 {
            let mut r = ready();
            let object = r.row().object;
            let p = r.prepare(3, 20);
            let mut p = Some(p);
            let install = if phase == 0 {
                None
            } else {
                r.stage(3, 20, p.take().unwrap()).unwrap();
                if phase == 2 {
                    Some(
                        r.engine
                            .take_staged_action()
                            .unwrap()
                            .unwrap()
                            .into_parts()
                            .1,
                    )
                } else {
                    None
                }
            };
            let id = match target {
                0 => object,
                1 => r.container,
                2 => r.actor,
                _ => r.facility,
            };
            r.engine.world.set_name(id, "changed preimage").unwrap();
            r.engine.world.take_dirty();
            let changed = r.engine.world.get(id).unwrap().clone();
            if phase == 0 {
                assert!(r.stage(3, 20, p.take().unwrap()).is_err());
            } else if phase == 1 {
                assert!(r.engine.take_staged_action().is_err());
            } else {
                assert!(r.engine.install_composed_action(install.unwrap()).is_err());
            }
            assert_eq!(r.engine.world.get(id), Some(&changed));
            assert_eq!(
                r.engine.world.get(object).unwrap().location,
                Some(r.container)
            );
            assert!(
                r.engine
                    .action_state()
                    .get(&r.handles.delivery.records, &7)
                    .map_or(true, |row| row.is_none())
            );
        }
    }
    for phase in 0..3 {
        let mut r = ready();
        let p = r.prepare(3, 21);
        let mut p = Some(p);
        let install = if phase == 0 {
            None
        } else {
            r.stage(3, 21, p.take().unwrap()).unwrap();
            if phase == 2 {
                Some(
                    r.engine
                        .take_staged_action()
                        .unwrap()
                        .unwrap()
                        .into_parts()
                        .1,
                )
            } else {
                None
            }
        };
        r.data.lock().unwrap().generation += 1;
        if phase == 0 {
            assert!(r.stage(3, 21, p.take().unwrap()).is_err());
        } else if phase == 1 {
            assert!(r.engine.take_staged_action().is_err());
        } else {
            assert!(r.engine.install_composed_action(install.unwrap()).is_err());
        }
    }
}
#[test]
fn exact_actor_action_and_consumed_output_are_required() {
    let mut r = ready();
    assert!(
        r.engine
            .prepare_registered_composed_authoritative_action(
                &r.handles.wrong,
                &inv(r.actor, 30, false),
                token(30)
            )
            .is_err()
    );
    assert!(
        r.engine
            .prepare_registered_composed_authoritative_action(
                &r.handles.actions[3],
                &inv(GOD, 31, false),
                token(31)
            )
            .is_err()
    );
    r.data.lock().unwrap().allowed = false;
    assert!(
        r.engine
            .prepare_registered_composed_authoritative_action(
                &r.handles.actions[3],
                &inv(r.actor, 32, false),
                token(32)
            )
            .is_err()
    );
    r.data.lock().unwrap().allowed = true;
    let install = held(&mut r, 33);
    r.engine.install_composed_action(install).unwrap();
    assert!(
        r.engine
            .prepare_registered_composed_authoritative_action(
                &r.handles.actions[3],
                &inv(r.actor, 34, false),
                token(34)
            )
            .is_err()
    );
}
#[test]
fn cold_delivery_rejects_missing_crossed_or_altered_birth_and_receipt() {
    let mut r = ready();
    let install = held(&mut r, 40);
    r.engine.install_composed_action(install).unwrap();
    for field in [
        "missing",
        "completion",
        "object",
        "operation",
        "delivered_digest",
    ] {
        let mut disk = r.disk.clone();
        let key = disk
            .iter()
            .find(|(_, v)| {
                v.get("family")
                    .and_then(|f| f.get("id"))
                    .and_then(Value::as_str)
                    == Some(FAMILY)
            })
            .unwrap()
            .0
            .clone();
        if field == "missing" {
            disk.remove(&key);
        } else {
            let row = disk.get_mut(&key).unwrap().get_mut("value").unwrap();
            match field {
                "completion" => row["output"]["state"]["completion"] = json!(token(99)),
                "object" => row["output"]["object"] = json!(r.actor),
                "operation" => row["operation"] = json!(token(3)),
                _ => row["delivered_digest"] = json!(DefinitionDigest::from_bytes([8; 32])),
            }
        }
        assert!(r.cold(&disk).is_err(), "corrupt delivery history admitted");
    }
}

#[test]
fn complete_three_object_envelope_accepts_exact_limit_and_refuses_one_more_byte() {
    let mut probe = ready();
    probe.engine.world.set_name(probe.container, "").unwrap();
    probe.engine.world.take_dirty();
    let prepared = probe.prepare(3, 50);
    let writes: Vec<_> = prepared
        .0
        .authoritative_replay_writes()
        .into_iter()
        .filter(|w| matches!(w, ReplayWrite::ObjectSet { .. }))
        .collect();
    assert_eq!(writes.len(), 3);
    let base = canonical_json_bytes_with_limit(&json!(writes), IMAGE_BYTES)
        .unwrap()
        .len();
    assert!(base < IMAGE_BYTES);
    probe.engine.abort_prepared_transaction(prepared.0);
    for extra in [0usize, 1] {
        let mut r = ready();
        let name = "x".repeat(IMAGE_BYTES - base + extra);
        r.engine.world.set_name(r.container, &name).unwrap();
        r.engine.world.take_dirty();
        let object = r.row().object;
        let before = r.engine.world.get(object).unwrap().clone();
        let result = r.engine.prepare_registered_composed_authoritative_action(
            &r.handles.actions[3],
            &inv(r.actor, 50, false),
            token(50),
        );
        if extra == 1 {
            assert!(
                result.is_err(),
                "one-byte oversized complete image admitted"
            );
            assert!(
                r.engine.world.get(object) == Some(&before),
                "refusal moved output"
            );
            assert!(
                r.engine
                    .action_state()
                    .get(&r.handles.delivery.records, &7)
                    .unwrap()
                    .is_none()
            );
        } else {
            let prepared = result.unwrap();
            let writes: Vec<_> = prepared
                .0
                .authoritative_replay_writes()
                .into_iter()
                .filter(|w| matches!(w, ReplayWrite::ObjectSet { .. }))
                .collect();
            assert_eq!(
                canonical_json_bytes_with_limit(&json!(writes), IMAGE_BYTES)
                    .unwrap()
                    .len(),
                IMAGE_BYTES
            );
            r.stage(3, 50, prepared).unwrap();
            let (_, install) = r.engine.take_staged_action().unwrap().unwrap().into_parts();
            r.engine.install_composed_action(install).unwrap();
            assert_eq!(r.engine.world.get(object).unwrap().location, Some(r.actor));
        }
    }
}

#[test]
fn invalid_containment_and_lifecycle_refuse_without_rewriting_physical_state() {
    for case in 0..5 {
        let mut r = ready();
        let object = r.row().object;
        match case {
            // Native World prevents cycles; a delivery additionally refuses its
            // output as an ancestor, even if all current memberships are exact.
            0 => {
                r.engine.world.move_to(r.actor, object).unwrap();
            }
            1 => {
                let mut holder = r.facility;
                for n in 0..crate::MAX_CONTAINMENT_DEPTH {
                    holder = r
                        .engine
                        .world
                        .create(
                            ObjKind::Thing,
                            &format!("deep holder {n}"),
                            GOD,
                            Some(holder),
                        )
                        .unwrap();
                }
                r.engine.world.move_to(r.actor, holder).unwrap();
            }
            2 => {
                r.engine
                    .world
                    .set_flag(r.actor, Flags::GOING, true)
                    .unwrap();
            }
            3 => {
                r.engine
                    .world
                    .set_flag(r.container, Flags::GOING, true)
                    .unwrap();
            }
            _ => {
                r.engine.world.set_flag(object, Flags::GOING, true).unwrap();
            }
        }
        r.engine.world.take_dirty();
        let before = r.engine.world.get(object).unwrap().clone();
        assert!(
            r.engine
                .prepare_registered_composed_authoritative_action(
                    &r.handles.actions[3],
                    &inv(r.actor, 51, false),
                    token(51)
                )
                .is_err()
        );
        assert!(
            r.engine.world.get(object) == Some(&before),
            "refused physical state changed"
        );
        assert!(
            r.engine
                .action_state()
                .get(&r.handles.delivery.records, &7)
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn full_action_identity_readiness_and_caught_owner_refusal_stay_closed() {
    let mut r = ready();
    let actual = &r.handles.actions[3];
    let changed = DefinitionRef::new(
        actual.key().clone(),
        actual.version(),
        DefinitionDigest::from_bytes([99; 32]),
    )
    .unwrap();
    assert!(
        r.engine
            .prepare_registered_composed_authoritative_action(
                &changed,
                &inv(r.actor, 60, false),
                token(60)
            )
            .is_err()
    );
    let original = inv(r.actor, 61, false);
    let caught = ActionInvocation::new(ActionInvocationDraft {
        request: original.request(),
        principal: original.principal().clone(),
        actor: r.actor,
        offered_action: original.offered_action(),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: json!({"order":7,"catch_refusal":true}),
        idempotency: original.idempotency().clone(),
    })
    .unwrap();
    assert!(
        r.engine
            .prepare_registered_composed_authoritative_action(
                &r.handles.actions[3],
                &caught,
                token(61)
            )
            .is_err()
    );
    assert!(
        r.engine
            .action_state()
            .get(&r.handles.delivery.records, &7)
            .unwrap()
            .is_none()
    );
    for phase in 0..3 {
        let mut r = ready();
        let object = r.row().object;
        let prepared = (phase > 0).then(|| r.prepare(3, 62));
        let mut prepared = prepared;
        if phase == 2 {
            r.stage(3, 62, prepared.take().unwrap()).unwrap();
        }
        r.engine
            .registry
            .freeze_mechanics_owner(
                &OwnerId::new("industry").unwrap(),
                crate::mechanics::IncidentId(1),
                crate::mechanics::RecoveryReason::IntegrityViolation {
                    code: CanonicalId::new("core_delivery_fixture").unwrap(),
                },
            )
            .unwrap();
        match phase {
            0 => assert!(
                r.engine
                    .prepare_registered_composed_authoritative_action(
                        &r.handles.actions[3],
                        &inv(r.actor, 62, false),
                        token(62)
                    )
                    .is_err()
            ),
            1 => assert!(r.stage(3, 62, prepared.take().unwrap()).is_err()),
            _ => assert!(r.engine.take_staged_action().is_err()),
        }
        assert_eq!(
            r.engine.world.get(object).unwrap().location,
            Some(r.container)
        );
    }
}

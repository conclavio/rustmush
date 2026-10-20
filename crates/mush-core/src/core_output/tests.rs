#![cfg(test)]

//! Structural Core owner tests: real registered Engine actions and World
//! objects, with a test Industry policy. No certificate, money or title claim.
use super::*;
use crate::action::{
    ActionError, ActionErrorCode, ActionInvocationDraft, ActionOfferId, ActionProvider,
    AuthoritativeActionOutput, CanonicalHash, IdempotencyKey, IdempotencyScope,
    PreparedAuthoritativeAction, PreparedCausalRef, RequestUid, ScopedIdempotencyKey,
};
use crate::action_state::ActionStateBootRecord;
use crate::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft, ReplayWrite};
use crate::composed::PreparedComposedTxn;
use crate::mechanics::{DefinitionValidationContext, RulesValue};
use crate::read_service::{OwnerReadContext, OwnerReadError};
use crate::{Engine, GameModule, Outbox, Registry};
use std::collections::BTreeMap;
use std::sync::Mutex;

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
    actions: [DefinitionRef; 3],
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
        ];
        let wrong = simple(reg, "action", "wrong_core_output")?;
        let outcome = simple(reg, "outcome", "core_output_fixture")?;
        let recipe = simple(reg, "recipe", "core_integrate")?;
        let binding = CoreOutputBinding::new(actions.clone(), recipe.clone())?;
        let expected = binding.policy_descriptor()?;
        reg.definition_validator(
            DefinitionKind::new("owner_read").unwrap(),
            move |ctx: &DefinitionValidationContext<'_>| {
                let e = ctx.definition();
                if e.reference().key().id == *expected.id()
                    && e.reference().version() == expected.version()
                    && e.payload() == expected.payload()
                    && e.references() == expected.references()
                {
                    Ok(())
                } else {
                    Err("unexpected output policy".into())
                }
            },
        )?;
        reg.state(OwnerState(Arc::clone(&self.data)));
        let service = CoreOutputReservations::register(reg, binding, Policy, BTreeSet::new())?;
        for (i, action) in actions.iter().enumerate() {
            reg.action_provider(
                action.clone(),
                Provider {
                    service: service.clone(),
                    mode: i,
                    outcome: outcome.clone(),
                },
            )?;
        }
        reg.action_provider(
            wrong.clone(),
            Provider {
                service: service.clone(),
                mode: 1,
                outcome,
            },
        )?;
        *self.handles.lock().unwrap() = Some(Handles {
            service,
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
                .service
                .audit(ctx, |_| Ok(()))
                .map_err(mod_error)?;
        }
        Ok(())
    }
}
struct Provider {
    service: CoreOutputReservations<Policy>,
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
        if inv.parameters() != &expected {
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
                json!({"initialized":true})
            }
            1 => {
                let r = ctx
                    .reserve_core_output(&self.service, txn, 7)
                    .map_err(|_| failed())?;
                json!({"order":r.order().id(),"object":r.object(),"reservation":r.reservation()})
            }
            _ => {
                let r = ctx
                    .consume_core_output(&self.service, txn, 7)
                    .map_err(|_| failed())?;
                json!({"order":r.order().id(),"object":r.object(),"reservation":r.reservation()})
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
    r
}

#[test]
fn actual_reserved_object_container_and_allocator_install_as_one_inert_union() {
    let mut r = ready();
    let next = r.engine.world.next_id();
    let before = r.engine.world.get(r.container).unwrap().clone();
    let revision = r.engine.action_state().revision();
    let p = r.prepare(1, 2);
    assert_eq!(r.engine.world.next_id(), next);
    assert_eq!(r.engine.world.get(r.container), Some(&before));
    assert_eq!(r.engine.action_state().revision(), revision);
    let writes = p.0.authoritative_replay_writes();
    assert_eq!(writes.len(), 5);
    assert_eq!(
        writes
            .iter()
            .filter(|w| matches!(w, ReplayWrite::ObjectSet { .. }))
            .count(),
        2
    );
    r.stage(1, 2, p).unwrap();
    assert_eq!(r.engine.world.next_id(), next);
    let held = r.engine.take_staged_action().unwrap().unwrap();
    assert_eq!(r.engine.world.next_id(), next);
    let (_, install) = held.into_parts();
    r.engine.install_composed_action(install).unwrap();
    let row = r.row();
    assert_eq!(row.object, ObjId(next));
    assert_eq!(row.reservation, token(2));
    assert_eq!(r.engine.world.get(row.object), Some(&row.birth_object()));
    assert_eq!(r.engine.world.next_id(), next + 1);
    assert_eq!(
        r.engine.world.get(r.container).unwrap().contents,
        vec![ObjId(next)]
    );
    assert_eq!(r.engine.world.get(ObjId(next)).unwrap().owner, GOD);
    assert!(
        r.engine
            .world
            .get(ObjId(next))
            .unwrap()
            .behaviors
            .is_empty()
    );
}
#[test]
fn exact_acknowledged_order_reuses_birth_then_consumes_once_without_object_changes() {
    let mut r = ready();
    let p = r.prepare(1, 2);
    r.install(p.0);
    let birth = r.row().clone();
    let next = r.engine.world.next_id();
    let container = r.engine.world.get(r.container).unwrap().clone();
    let p = r.prepare(1, 3);
    assert_eq!(p.0.authoritative_replay_writes().len(), 1);
    r.install(p.0);
    assert_eq!(r.row(), &birth);
    assert_eq!(r.engine.world.next_id(), next);
    let p = r.prepare(2, 4);
    assert!(
        p.0.authoritative_replay_writes()
            .iter()
            .all(|w| matches!(w, ReplayWrite::ModuleJsonSet { .. }))
    );
    r.install(p.0);
    assert_eq!(r.row().reservation, birth.reservation);
    assert!(matches!(r.row().state,OutputState::Consumed {completion,..} if completion==token(4)));
    assert_eq!(r.engine.world.get(r.container), Some(&container));
    assert_eq!(
        r.engine.world.get(birth.object),
        Some(&birth.birth_object())
    );
    assert!(
        r.engine
            .prepare_registered_composed_authoritative_action(
                &r.handles.actions[2],
                &inv(r.actor, 5, false),
                token(5)
            )
            .is_err()
    );
    assert!(r.cold(&r.disk).is_ok());
}
#[test]
fn actual_owner_order_actor_and_exact_action_are_required_before_allocation() {
    let mut r = bare();
    r.accept();
    let next = r.engine.world.next_id();
    assert!(
        r.engine
            .prepare_registered_composed_authoritative_action(
                &r.handles.actions[1],
                &inv(r.actor, 2, false),
                token(2)
            )
            .is_err()
    );
    assert_eq!(r.engine.world.next_id(), next);
    for mode in 0..4 {
        let mut r = ready();
        let next = r.engine.world.next_id();
        let action = if mode == 0 {
            r.handles.wrong.clone()
        } else {
            r.handles.actions[1].clone()
        };
        if mode == 1 {
            r.data.lock().unwrap().allowed = false;
        }
        if mode == 2 {
            r.data.lock().unwrap().order = None;
        }
        let actor = if mode == 3 { GOD } else { r.actor };
        assert!(
            r.engine
                .prepare_registered_composed_authoritative_action(
                    &action,
                    &inv(actor, 2, false),
                    token(2)
                )
                .is_err()
        );
        assert_eq!(r.engine.world.next_id(), next);
        assert_eq!(r.engine.action_state().len(), 1);
    }
}
#[test]
fn actual_stage_and_take_repeat_owner_policy_and_object_preimages() {
    for after_stage in [false, true] {
        for drift in 0..3 {
            let mut r = ready();
            let next = r.engine.world.next_id();
            let p = r.prepare(1, 2);
            if after_stage {
                r.stage(1, 2, p).unwrap();
            } else {
                match drift {
                    0 => r.data.lock().unwrap().generation += 1,
                    1 => r.data.lock().unwrap().allowed = false,
                    _ => r
                        .engine
                        .world
                        .set_name(r.container, "changed output container")
                        .unwrap(),
                }
                assert!(r.stage(1, 2, p).is_err());
                assert!(!r.engine.action_state().reservation_lost());
                assert_eq!(r.engine.world.next_id(), next);
                continue;
            }
            match drift {
                0 => r.data.lock().unwrap().generation += 1,
                1 => r.data.lock().unwrap().allowed = false,
                _ => r
                    .engine
                    .world
                    .set_name(r.container, "changed output container")
                    .unwrap(),
            }
            assert!(r.engine.take_staged_action().is_err());
            assert_eq!(r.engine.world.next_id(), next);
            assert!(!r.engine.action_state().reservation_lost());
            assert_eq!(r.engine.action_state().len(), 1);
        }
    }
}
#[test]
fn post_ack_local_bind_refuses_changed_owner_or_container_without_any_typed_install() {
    for drift in 0..2 {
        let mut r = ready();
        let next = r.engine.world.next_id();
        let p = r.prepare(1, 2);
        r.stage(1, 2, p).unwrap();
        let held = r.engine.take_staged_action().unwrap().unwrap();
        let (_, install) = held.into_parts();
        if drift == 0 {
            r.data.lock().unwrap().generation += 1;
        } else {
            r.engine
                .world
                .set_name(r.container, "changed after custody")
                .unwrap();
        }
        assert!(r.engine.install_composed_action(install).is_err());
        assert_eq!(r.engine.world.next_id(), next);
        assert_eq!(r.engine.action_state().len(), 1);
        assert!(r.engine.action_state().reservation_lost());
    }
}

#[test]
fn reuse_and_completion_retain_read_only_container_preimages_through_every_boundary() {
    for phase in [1, 2] {
        for boundary in 0..3 {
            for membership in [false, true] {
                let mut r = ready();
                let p = r.prepare(1, 2);
                r.install(p.0);
                let before = r.row().clone();
                let revision = r.engine.action_state().revision();
                let p = r.prepare(phase, 3);
                assert!(
                    p.0.authoritative_replay_writes()
                        .iter()
                        .all(|w| matches!(w, ReplayWrite::ModuleJsonSet { .. })),
                    "verification does not republish the container"
                );
                let drift = |r: &mut Rig| {
                    if membership {
                        r.engine
                            .world
                            .get_mut(r.container)
                            .unwrap()
                            .contents
                            .clear();
                    } else {
                        r.engine
                            .world
                            .set_name(r.container, "changed retained container")
                            .unwrap();
                    }
                };
                if boundary == 0 {
                    drift(&mut r);
                    assert!(r.stage(phase, 3, p).is_err());
                } else {
                    r.stage(phase, 3, p).unwrap();
                    if boundary == 1 {
                        drift(&mut r);
                        assert!(r.engine.take_staged_action().is_err());
                    } else {
                        let held = r.engine.take_staged_action().unwrap().unwrap();
                        let (_, install) = held.into_parts();
                        drift(&mut r);
                        assert!(r.engine.install_composed_action(install).is_err());
                    }
                }
                assert_eq!(r.engine.action_state().revision(), revision);
                assert_eq!(r.engine.action_state().reservation_lost(), boundary == 2);
                if boundary != 2 {
                    assert_eq!(r.row(), &before);
                }
            }
        }
    }
}
#[test]
fn cold_audit_refuses_missing_crossed_births_or_unsupported_output_move() {
    let mut r = ready();
    let p = r.prepare(1, 2);
    r.install(p.0);
    let p = r.prepare(2, 3);
    r.install(p.0);
    assert!(r.cold(&r.disk).is_ok());
    let record = r
        .disk
        .iter()
        .find(|(_, v)| {
            v.get("family")
                .and_then(|v| v.get("id"))
                .and_then(Value::as_str)
                == Some("core_output_reservations")
        })
        .unwrap()
        .0
        .clone();
    for mode in 0..5 {
        let mut disk = r.disk.clone();
        match mode {
            0 => {
                disk.remove(&record);
            }
            1 => {
                disk.get_mut(&record).unwrap()["value"]["birth"] = json!(2);
            }
            2 => {
                disk.get_mut(&record).unwrap()["value"]["order"]["id"] = json!(8);
            }
            3 => {
                disk.get_mut(&record).unwrap()["value"]["purpose"] = json!("roll.initial_filing");
            }
            _ => {
                disk.get_mut(&record).unwrap()["value"]["state"]["completion"] = json!(token(2));
            }
        }
        assert!(r.cold(&disk).is_err(), "mode {mode}");
    }
    let output = r.row().object;
    r.engine.world.move_to(output, r.actor).unwrap();
    assert!(
        r.cold(&r.disk).is_err(),
        "delivery requires a separate real closed history producer"
    );
}
#[test]
fn accepted_intent_change_and_noncanonical_record_cannot_rebind_an_old_output() {
    let mut r = ready();
    let p = r.prepare(1, 2);
    r.install(p.0);
    let before = r.row().clone();
    // WorkOrderId is a derived full-width u64; zero is a lawful identity.
    for id in [0, u64::MAX] {
        let order = CoreOutputOrder::new(
            id,
            before.order.intent(),
            before.order.requester(),
            before.order.recipe().clone(),
            before.order.facility(),
            before.order.container(),
        )
        .unwrap();
        let decoded: CoreOutputOrder = decode(&serde_json::to_value(&order).unwrap()).unwrap();
        assert_eq!(decoded, order);
        decoded.validate().unwrap();
    }
    let changed = CoreOutputOrder::new(
        before.order.id(),
        DefinitionDigest::from_bytes([9; 32]),
        before.order.requester(),
        before.order.recipe().clone(),
        before.order.facility(),
        before.order.container(),
    )
    .unwrap();
    r.data.lock().unwrap().order = Some(changed);
    assert!(
        r.engine
            .prepare_registered_composed_authoritative_action(
                &r.handles.actions[1],
                &inv(r.actor, 3, false),
                token(3)
            )
            .is_err()
    );
    assert_eq!(r.row(), &before);
    let mut v = serde_json::to_value(&before).unwrap();
    v["extra"] = json!(true);
    assert!(decode::<OutputRecord>(&v).is_err());
    let mut v = serde_json::to_value(&before).unwrap();
    v["order"]["container"] = Value::Null;
    assert!(decode::<OutputRecord>(&v).is_err());
}
#[test]
fn fixed_binding_refuses_wrong_owner_phase_or_recipe_and_readiness_stays_live() {
    let r = ready();
    let mut actions = r.handles.actions.clone();
    actions.swap(0, 1);
    assert!(CoreOutputBinding::new(actions, r.handles.recipe.clone()).is_err());
    let changed = DefinitionRef::new(
        crate::mechanics::DefinitionKey::new(
            OwnerId::new("foreign").unwrap(),
            DefinitionKind::new("action").unwrap(),
            DefinitionId::new(RESERVE_ACTION).unwrap(),
        ),
        1,
        r.handles.actions[1].digest(),
    )
    .unwrap();
    let mut actions = r.handles.actions.clone();
    actions[1] = changed;
    assert!(CoreOutputBinding::new(actions, r.handles.recipe.clone()).is_err());
    let mut r = ready();
    let p = r.prepare(1, 2);
    r.engine
        .registry
        .freeze_mechanics_owner(
            &OwnerId::new("industry").unwrap(),
            crate::mechanics::IncidentId(1),
            crate::mechanics::RecoveryReason::IntegrityViolation {
                code: CanonicalId::new("core_output_fixture").unwrap(),
            },
        )
        .unwrap();
    assert!(p.0.recheck(&r.engine.world).is_err());
    drop(p);
    assert_eq!(r.engine.action_state().len(), 1);
}

#[test]
fn actual_policy_capacity_and_corrupt_container_chain_refuse_before_allocation() {
    for mode in 0..2 {
        let mut r = ready();
        if mode == 0 {
            for n in 0..8 {
                r.engine
                    .world
                    .create(
                        ObjKind::Thing,
                        &format!("Existing output {n}"),
                        GOD,
                        Some(r.container),
                    )
                    .unwrap();
            }
        } else {
            // Deliberate loaded-World corruption; neither the policy nor Core
            // may turn a pre-existing ancestor cycle into a valid new output.
            r.engine.world.get_mut(r.facility).unwrap().location = Some(r.container);
        }
        let next = r.engine.world.next_id();
        let before = r.engine.world.get(r.container).unwrap().clone();
        assert!(
            r.engine
                .prepare_registered_composed_authoritative_action(
                    &r.handles.actions[1],
                    &inv(r.actor, 2, false),
                    token(2)
                )
                .is_err()
        );
        assert_eq!(r.engine.world.next_id(), next);
        assert_eq!(r.engine.world.get(r.container), Some(&before));
        assert_eq!(r.engine.action_state().len(), 1);
    }
}

#[test]
fn complete_maximum_width_record_and_replay_envelope_fit_without_changing_old_shell_caps() {
    let mut r = ready();
    let p = r.prepare(1, 2);
    r.install(p.0);
    let mut row = r.row().clone();
    row.birth = u64::MAX;
    row.order = CoreOutputOrder::new(
        u64::MAX,
        DefinitionDigest::from_bytes([255; 32]),
        ObjId(u32::MAX - 3),
        r.handles.recipe.clone(),
        ObjId(u32::MAX - 2),
        ObjId(u32::MAX - 1),
    )
    .unwrap();
    row.object = ObjId(u32::MAX);
    row.created_at = i64::MIN;
    row.object_digest = object_digest(&row.birth_object()).unwrap();
    row.state = OutputState::Consumed {
        completion: token(255),
        action: r.handles.actions[2].clone(),
    };
    row.validate().unwrap();
    let document = json!({"family":{"owner":"industry","id":"core_output_reservations"},"key":row.order.id().to_string(),"schema":{"id":"industry.core_output_reservations.value.v1","version":1},"value":row});
    canonical_json_bytes_with_limit(&document, RECORD_BYTES + 1024).unwrap();
    let write = ReplayWrite::ModuleJsonSet {
        namespace: OwnerId::new("industry").unwrap(),
        key: crate::authoritative::AuthoritativeKey::parse_canonical(&format!(
            "action-state/v1/core_output_reservations/{}",
            u64::MAX
        ))
        .unwrap(),
        value: document,
    };
    canonical_json_bytes_with_limit(&serde_json::to_value(write).unwrap(), RECORD_BYTES + 2048)
        .unwrap();
    // The existing Roll reservation remains its own descriptor/codec and is
    // never registered or routed through Industry's output family.
    assert_ne!(PURPOSE, "roll.initial_filing");
}

#![cfg(test)]

//! Actual registered-provider reads. The shared service deliberately permits
//! hostile serializers and exhaustion so the production common fence is used.
use super::*;
use crate::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionInvocationDraft, ActionOfferId,
    ActionProvider, AuthoritativeActionOutput, CanonicalHash, CommitToken, IdempotencyKey,
    IdempotencyScope, PerceivedRef, ProviderRegistrationOrdinal, RequestUid, ScopedIdempotencyKey,
};
use crate::action_state::{ActionStateStore, ActionTxn, StateFamilyRegistry};
use crate::authoritative::{AuthoritativeKey, ReplayWrite};
use crate::mechanics::{DefinitionValidationContext, RulesValue};
use crate::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
    ParticipantHandle,
};
use crate::resolution::PrepareCtx;

type Family = StateFamilyHandle<u32, u32>;
type Mirror = Arc<Mutex<Option<(StateFamilyRegistry, ProviderRegistrationOrdinal)>>>;
#[derive(Clone)]
struct Handles {
    action: DefinitionRef,
    own: Family,
    truth: Family,
}

struct ReadOwner(Owner);
impl GameModule for ReadOwner {
    fn name(&self) -> &'static str {
        "owner"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        self.0.register(reg)?;
        let family = self.0.family.lock().unwrap().clone().unwrap();
        // Only this test grants writes to demonstrate that reads remain the
        // committed base even when the same action has permission to stage it.
        reg.delegate_state_family(&family, "consumer")
    }
}
struct PrepareConsumer {
    truth: Captured,
    handles: Arc<Mutex<Option<Handles>>>,
    mode: Arc<AtomicUsize>,
    answers: Arc<Mutex<Vec<Result<Value, OwnerReadError>>>>,
    readiness: Arc<Mutex<Option<MechanicalOwnerReadiness>>>,
    mirror: Mirror,
}
fn def(reg: &mut Registrar<'_>, kind: &str, id: &str) -> Result<DefinitionRef, ModError> {
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
impl GameModule for PrepareConsumer {
    fn name(&self) -> &'static str {
        "consumer"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        for (kind, id) in [
            ("action", "read"),
            ("outcome", "read"),
            ("participant", "effect"),
        ] {
            reg.definition_validator(
                DefinitionKind::new(kind).unwrap(),
                move |ctx: &DefinitionValidationContext<'_>| {
                    let e = ctx.definition();
                    if e.reference().version() == 1
                        && e.reference().key().id.as_str() == id
                        && e.payload() == &json!({})
                        && e.references().is_empty()
                    {
                        Ok(())
                    } else {
                        Err("unexpected prepare-read fixture definition".into())
                    }
                },
            )?;
        }
        let action = def(reg, "action", "read")?;
        let outcome = def(reg, "outcome", "read")?;
        let own = reg.state_family(
            StateFamilyDescriptor::new(
                StateFamilyId::new("seen").unwrap(),
                CanonicalId::new("seen.key").unwrap(),
                CanonicalId::new("seen.value").unwrap(),
                1,
                CanonicalId::new("seen.decode").unwrap(),
                CanonicalId::new("seen.check").unwrap(),
                CanonicalId::new("seen.encode").unwrap(),
            )
            .unwrap(),
            |k: &u32| Ok(k.to_string()),
            |v: &Value| serde_json::from_value(v.clone()).map_err(|e| e.to_string()),
            |_: &u32| Ok(()),
            |v: &u32| Ok(json!(v)),
        )?;
        let truth = self.truth.lock().unwrap().clone().unwrap();
        let handle = reg.request_owner_read::<Service>(definition())?;
        reg.state(EffectState(10));
        let participant = def(reg, "participant", "effect")?;
        reg.owner_participant(
            participant,
            BTreeSet::from([action.clone()]),
            Effect {
                readiness: self.readiness.clone(),
                mode: self.mode.clone(),
            },
        )?;
        let effect = reg.request_owner_participant::<Effect>(action.clone())?;
        reg.action_provider(
            action.clone(),
            Provider {
                outcome,
                own: own.clone(),
                truth: truth.clone(),
                handle,
                mode: self.mode.clone(),
                answers: self.answers.clone(),
                readiness: self.readiness.clone(),
                mirror: self.mirror.clone(),
                effect,
            },
        )?;
        *self.handles.lock().unwrap() = Some(Handles { action, own, truth });
        Ok(())
    }
}
struct Provider {
    outcome: DefinitionRef,
    own: Family,
    truth: Family,
    handle: OwnerReadHandle<Service>,
    mode: Arc<AtomicUsize>,
    answers: Arc<Mutex<Vec<Result<Value, OwnerReadError>>>>,
    readiness: Arc<Mutex<Option<MechanicalOwnerReadiness>>>,
    mirror: Mirror,
    effect: ParticipantHandle<Effect>,
}
#[derive(Clone)]
struct EffectState(u32);
struct Effect {
    readiness: Arc<Mutex<Option<MechanicalOwnerReadiness>>>,
    mode: Arc<AtomicUsize>,
}
struct BoundEffect<'a>(&'a mut EffectState);
impl BoundParticipant for BoundEffect<'_> {
    fn apply(self) {
        self.0.0 = 11;
    }
}
impl OwnerParticipant for Effect {
    type State = EffectState;
    type Request = ();
    type Output = ();
    type Patch = ();
    type Bound<'a> = BoundEffect<'a>;
    fn prepare(
        &self,
        _: &EffectState,
        _: &OwnerPrepareContext<'_>,
        _: (),
    ) -> Result<OwnerPreparation<(), ()>, ParticipantError> {
        Ok(OwnerPreparation::new(
            (),
            (),
            vec![ReplayWrite::ModuleJsonSet {
                namespace: OwnerId::new("consumer").unwrap(),
                key: AuthoritativeKey::parse_canonical("effect").unwrap(),
                value: json!(11),
            }],
        ))
    }
    fn recheck(
        &self,
        _: &EffectState,
        _: &OwnerPrepareContext<'_>,
        _: &(),
    ) -> Result<(), ParticipantError> {
        if self.mode.load(Ordering::SeqCst) == 23 {
            freeze(&self.readiness, "dependency");
        }
        Ok(())
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut EffectState,
        _: (),
    ) -> Result<BoundEffect<'a>, ParticipantError> {
        if self.mode.load(Ordering::SeqCst) == 24 {
            freeze(&self.readiness, "dependency");
        }
        Ok(BoundEffect(state))
    }
    fn accepts_write(&self, w: &ReplayWrite) -> bool {
        matches!(w,ReplayWrite::ModuleJsonSet {namespace,key,value} if namespace.as_str()=="consumer" && key.as_str()=="effect" && value==&json!(11))
    }
}
fn failed() -> ActionError {
    ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        "prepare-read fixture refused",
    )
    .unwrap()
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, _: &ActionInvocation) -> Result<(), ActionError> {
        Ok(())
    }
    fn prepare_authoritative_with_context(
        &self,
        _: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let mode = self.mode.load(Ordering::SeqCst);
        if mode == 20 {
            txn.remove(&self.truth, &1).map_err(|_| failed())?;
        } else {
            txn.replace(&self.truth, &1, 99).map_err(|_| failed())?;
        }
        let request = || Request {
            mode: if (20..30).contains(&mode) {
                0
            } else {
                mode as u8
            },
            pretend_actor: ObjId(999),
            text: if mode == 6 {
                "x".repeat(MAX_OWNER_READ_REQUEST_BYTES)
            } else {
                String::new()
            },
            readiness: Some(self.readiness.clone()),
        };
        if mode == 21 {
            let (registry, ordinal) = self.mirror.lock().unwrap().clone().unwrap();
            let mut forged = ActionStateStore::new(registry);
            // Same handles, registration, provider ordinal and base revision;
            // only its store identity and committed truth differ.
            let mut seed = forged.transaction(&self.truth).unwrap();
            seed.insert(&self.truth, &1, 777).unwrap();
            let (_, install) = seed.prepare().unwrap().into_parts();
            forged.install(install).unwrap();
            assert_eq!(forged.revision(), 1, "same actual typed base revision");
            let mut other = forged.transaction_for_provider(&ordinal).unwrap();
            let result = ctx
                .read_owner_preimage(&mut other, &self.handle, request())
                .map(|a| a.value);
            self.answers.lock().unwrap().push(result);
            assert!(
                other.prepare().is_err(),
                "failed alternate transaction is poisoned"
            );
        } else {
            let calls = if mode == 22 {
                MAX_OWNER_PREIMAGE_READS + 1
            } else {
                1
            };
            for _ in 0..calls {
                let result = ctx
                    .read_owner_preimage(txn, &self.handle, request())
                    .map(|a| a.value);
                // Deliberately catch every failure. Core must still refuse the
                // complete prepared action rather than publish a fallback.
                self.answers.lock().unwrap().push(result);
            }
        }
        if [23, 24, 30].contains(&mode) {
            ctx.prepare_owner(&self.effect, ()).map_err(|_| failed())?;
        }
        txn.insert(&self.own, &1, 42).map_err(|_| failed())?;
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(self.outcome.clone(), json!({"read":true})).unwrap(),
        ))
    }
}
struct Rig {
    engine: Engine,
    handles: Handles,
    mode: Arc<AtomicUsize>,
    answers: Arc<Mutex<Vec<Result<Value, OwnerReadError>>>>,
    calls: Arc<AtomicUsize>,
    readiness: Arc<Mutex<Option<MechanicalOwnerReadiness>>>,
    invocation: ActionInvocation,
}
fn rig() -> Rig {
    let truth = Arc::new(Mutex::new(None));
    let handles = Arc::new(Mutex::new(None));
    let mode = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let answers = Arc::new(Mutex::new(Vec::new()));
    let readiness = Arc::new(Mutex::new(None));
    let mirror = Arc::new(Mutex::new(None));
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![
            Box::new(ReadOwner(Owner {
                family: truth.clone(),
                calls: calls.clone(),
                readiness: readiness.clone(),
                allow: true,
            })),
            Box::new(PrepareConsumer {
                truth,
                handles: handles.clone(),
                mode: mode.clone(),
                answers: answers.clone(),
                readiness: readiness.clone(),
                mirror: mirror.clone(),
            }),
            Box::new(Dependency),
        ],
    )
    .unwrap();
    *readiness.lock().unwrap() = engine.registry.mechanics_owner_readiness.clone();
    let handles = handles.lock().unwrap().clone().unwrap();
    *mirror.lock().unwrap() = Some((
        engine.registry.state_families().unwrap().clone(),
        engine
            .registry
            .action_providers()
            .unwrap()
            .get_exact(&handles.action)
            .unwrap()
            .registration_ordinal()
            .clone(),
    ));
    let mut seed = engine.action_transaction(&handles.truth).unwrap();
    seed.insert(&handles.truth, &1, 42).unwrap();
    let (_, install) = seed.prepare().unwrap().into_parts();
    engine.install_prepared_action(install).unwrap();
    let request = RequestUid::from_bytes([8; 16]);
    let principal = PrincipalId::Object { id: GOD };
    let invocation = ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor: GOD,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([9; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: json!({"actor":999,"provider":"owner"}),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new("consumer").unwrap(),
                namespace: CanonicalId::new("read").unwrap(),
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })
    .unwrap();
    Rig {
        engine,
        handles,
        mode,
        answers,
        calls,
        readiness,
        invocation,
    }
}
impl Rig {
    fn prepare(
        &mut self,
        mode: usize,
    ) -> Result<
        (
            crate::composed::PreparedComposedTxn,
            crate::action::PreparedAuthoritativeAction,
        ),
        crate::action::ActionProviderPrepareError,
    > {
        self.mode.store(mode, Ordering::SeqCst);
        self.engine
            .prepare_registered_composed_authoritative_action(
                &self.handles.action,
                &self.invocation,
                CommitToken::from_bytes([7; 16]),
            )
    }
    fn unchanged(&self) {
        assert_eq!(
            self.engine.action_state().revision(),
            1,
            "no typed installation occurred"
        );
        assert_eq!(self.engine.action_state().len(), 1);
        assert_eq!(
            self.engine.world.module_state::<EffectState>().unwrap().0,
            10
        );
        // Frozen owners correctly refuse ordinary readers too. The revision
        // and record count above remain observable without reopening them.
        if self
            .readiness
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .require_ready(&OwnerId::new("owner").unwrap())
            .is_ok()
        {
            assert_eq!(
                self.engine
                    .action_state()
                    .get(&self.handles.truth, &1)
                    .unwrap(),
                Some(&42)
            );
        }
        if self
            .readiness
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .require_ready(&OwnerId::new("consumer").unwrap())
            .is_ok()
        {
            assert_eq!(
                self.engine
                    .action_state()
                    .get(&self.handles.own, &1)
                    .unwrap(),
                None
            );
        }
    }
}
#[test]
fn committed_preimage_ignores_staged_replace_and_delete_and_binds_actual_identity() {
    for mode in [0, 20] {
        let mut r = rig();
        let (prepared, output) = r.prepare(mode).unwrap();
        r.unchanged();
        assert_eq!(
            r.answers.lock().unwrap().as_slice(),
            [Ok(
                json!({"actor":GOD,"principal":PrincipalId::Object{id:GOD},"caller":"consumer","value":42,"slot":9})
            )]
        );
        let (_, _, definitions, _, _, _) = output.into_parts();
        assert!(definitions.contains(&definition()));
        let (_, install) = prepared.into_parts();
        r.engine
            .install_composed_action_announcing(
                install,
                &mut Outbox::new(),
                &mut crate::engine::Effects::new(),
            )
            .unwrap();
        assert_eq!(
            r.engine
                .action_state()
                .get(&r.handles.truth, &1)
                .unwrap()
                .copied(),
            if mode == 20 { None } else { Some(99) }
        );
        assert_eq!(
            r.engine.action_state().get(&r.handles.own, &1).unwrap(),
            Some(&42)
        );
    }
}
#[test]
fn alternate_same_registry_and_revision_store_cannot_replace_actual_preimage() {
    let mut r = rig();
    assert!(r.prepare(21).is_err());
    r.unchanged();
    assert_eq!(
        r.answers.lock().unwrap().as_slice(),
        [Err(OwnerReadError::Authority)]
    );
    assert_eq!(r.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn caught_service_and_serializer_failures_poison_actual_provider_preparation() {
    for mode in [1, 2, 3, 4, 5, 6, 8, 9, 10, 13, 14, 15, 16, 17, 18] {
        let mut r = rig();
        assert!(r.prepare(mode).is_err(), "mode {mode}");
        r.unchanged();
        assert!(r.answers.lock().unwrap().last().unwrap().is_err());
        if [6, 13, 14, 18].contains(&mode) {
            assert_eq!(r.calls.load(Ordering::SeqCst), 0);
        }
    }
}
#[test]
fn observation_overflow_refuses_complete_action_without_discarding_oldest_proof() {
    let mut r = rig();
    assert!(r.prepare(22).is_err());
    r.unchanged();
    let answers = r.answers.lock().unwrap();
    assert_eq!(answers.len(), MAX_OWNER_PREIMAGE_READS + 1);
    assert_eq!(answers.last(), Some(&Err(OwnerReadError::Limit)));
    assert_eq!(r.calls.load(Ordering::SeqCst), MAX_OWNER_PREIMAGE_READS);
}
#[test]
fn captured_readiness_and_slot_replacement_refuse_before_installing_any_typed_image() {
    for mode in 0..4 {
        let mut r = rig();
        let (prepared, _) = r.prepare(0).unwrap();
        prepared.recheck(&r.engine.world).unwrap();
        if mode < 3 {
            freeze(&r.readiness, ["owner", "consumer", "dependency"][mode]);
        } else {
            let mut registry = Registry::new();
            let mut registrar = Registrar::new(&mut registry, "owner");
            registrar.state(Slot(9, 0));
            for init in registry.take_state_inits() {
                r.engine.world.module_state_apply(init);
            }
        }
        assert!(prepared.recheck(&r.engine.world).is_err());
        let (_, install) = prepared.into_parts();
        assert!(
            r.engine
                .install_composed_action_announcing(
                    install,
                    &mut Outbox::new(),
                    &mut crate::engine::Effects::new()
                )
                .is_err()
        );
        r.unchanged();
        assert!(r.engine.action_state().reservation_lost());
    }
}

#[test]
fn last_participant_recheck_or_bind_cannot_close_read_dependency_then_apply() {
    let mut r = rig();
    let (prepared, _) = r.prepare(23).unwrap();
    assert!(
        prepared.recheck(&r.engine.world).is_err(),
        "owner recheck closed read-only dependency"
    );
    r.unchanged();
    drop(prepared);
    let mut r = rig();
    let (prepared, _) = r.prepare(24).unwrap();
    prepared.recheck(&r.engine.world).unwrap();
    let (_, install) = prepared.into_parts();
    assert!(
        r.engine
            .install_composed_action_announcing(
                install,
                &mut Outbox::new(),
                &mut crate::engine::Effects::new()
            )
            .is_err()
    );
    r.unchanged();
    assert!(r.engine.action_state().reservation_lost());
}

fn bundle(
    r: &Rig,
    prepared: &crate::composed::PreparedComposedTxn,
    output: crate::action::PreparedAuthoritativeAction,
) -> crate::authoritative::AuthoritativeBundle {
    let (outcome, resolution, mut definitions, eligible_witnesses, durable_outbox, semantic_events) =
        output.into_parts();
    definitions.push(r.handles.action.clone());
    definitions.sort();
    definitions.dedup();
    crate::authoritative::AuthoritativeBundle::new(crate::authoritative::AuthoritativeBundleDraft {
        token: CommitToken::from_bytes([7; 16]),
        request: r.invocation.request(),
        idempotency: r.invocation.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([3; 32]),
        request_hash: r.invocation.request_hash(),
        plan_hash: CanonicalHash::from_bytes([5; 32]),
        action: r.handles.action.clone(),
        provider: OwnerId::new("consumer").unwrap(),
        cause: crate::action::PreparedCausalRef::new(r.invocation.request(), None, 0).unwrap(),
        definitions,
        writes: prepared.authoritative_replay_writes(),
        outcome,
        resolution: resolution
            .map(|prepared| prepared.bind(crate::CommitToken::from_bytes([7; 16]))),
        semantic_events,
        eligible_witnesses,
        durable_outbox,
    })
    .unwrap()
}
#[test]
fn actual_stage_and_take_custody_recheck_retained_read_observations() {
    for after_stage in [false, true] {
        let mut r = rig();
        let (prepared, output) = r.prepare(0).unwrap();
        let bundle = bundle(&r, &prepared, output);
        let guards = crate::action::ServerGuardSetBuilder::new(
            crate::action::RevisionSet::default(),
            r.readiness.lock().unwrap().as_ref().unwrap().clone(),
        )
        .finish()
        .unwrap();
        let _ = r.engine.world.take_dirty();
        if after_stage {
            r.engine
                .stage_authoritative_action(prepared, guards, bundle)
                .unwrap();
            freeze(&r.readiness, "dependency");
            assert!(r.engine.take_staged_action().is_err());
        } else {
            freeze(&r.readiness, "dependency");
            assert!(
                r.engine
                    .stage_authoritative_action(prepared, guards, bundle)
                    .is_err()
            );
        }
        r.unchanged();
        assert!(!r.engine.has_staged_action());
        assert!(
            !r.engine.action_state().reservation_lost(),
            "known pre-custody refusal aborts exactly"
        );
        assert!(!r.engine.action_state().has_prepared());
    }
}

#[test]
fn legacy_state_guard_allows_unrelated_fields_and_never_replays_the_service() {
    let mut r = rig();
    let (prepared, _) = r.prepare(30).unwrap();
    assert_eq!(r.calls.load(Ordering::SeqCst), 1);
    r.engine.world.module_state_mut::<Slot>().unwrap().1 = 77;
    prepared.recheck(&r.engine.world).unwrap();
    let (_, install) = prepared.into_parts();
    r.engine.install_composed_action(install).unwrap();
    assert_eq!(r.calls.load(Ordering::SeqCst), 1);
    assert_eq!(r.engine.world.module_state::<Slot>().unwrap().1, 77);
    assert_eq!(r.engine.world.module_state::<EffectState>().unwrap().0, 11);
    assert_eq!(
        r.engine.action_state().get(&r.handles.own, &1).unwrap(),
        Some(&42)
    );
}

#[test]
fn same_slot_legacy_field_drift_refuses_custody_and_post_ack_install_before_any_apply() {
    for after_custody in [false, true] {
        let mut r = rig();
        let (prepared, output) = r.prepare(30).unwrap();
        let bundle = bundle(&r, &prepared, output);
        let guards = crate::action::ServerGuardSetBuilder::new(
            crate::action::RevisionSet::default(),
            r.readiness.lock().unwrap().as_ref().unwrap().clone(),
        )
        .finish()
        .unwrap();
        r.engine.world.take_dirty();
        r.engine
            .stage_authoritative_action(prepared, guards, bundle)
            .unwrap();
        let held = if after_custody {
            Some(r.engine.take_staged_action().unwrap().unwrap())
        } else {
            None
        };
        r.engine.world.module_state_mut::<Slot>().unwrap().0 = 10;
        if let Some(held) = held {
            let (_, install) = held.into_parts();
            assert!(r.engine.install_composed_action(install).is_err());
            assert!(r.engine.action_state().reservation_lost());
        } else {
            assert!(r.engine.take_staged_action().is_err());
            assert!(!r.engine.action_state().reservation_lost());
        }
        r.unchanged();
        assert_eq!(r.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn caught_guard_errors_and_fingerprint_readiness_changes_poison_the_whole_action() {
    for mode in 31..=34 {
        let mut r = rig();
        assert!(r.prepare(mode).is_err(), "mode {mode}");
        assert_eq!(r.calls.load(Ordering::SeqCst), 1);
        r.unchanged();
    }
}

#![cfg(test)]

//! Real provider/capsule tests; the test backend follows the production map-lock
//! contract. No public readiness constructor or prevalidated account request.
use super::*;
use crate::action::{
    ActionError, ActionErrorCode, ActionInvocationDraft, ActionOfferId, ActionProvider,
    AuthoritativeActionOutput, CanonicalHash, IdempotencyKey, IdempotencyScope, PerceivedRef,
    RequestUid, ScopedIdempotencyKey,
};
use crate::action_state::{ActionTxn, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId};
use crate::authoritative::{AuthoritativeKey, ReplayWrite};
use crate::mechanics::{
    CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionValidationContext, OwnerId, RulesValue,
};
use crate::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantHandle,
};
use crate::resolution::PrepareCtx;
use crate::{Engine, GOD, GameModule, LIMBO, ModError, Outbox, Registrar, Registry};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, RwLock, atomic::AtomicUsize};

#[derive(Default)]
struct Backend {
    map: RwLock<BTreeMap<ObjId, AccountId>>,
    epoch: AccountIdentityEpoch,
    observations: AtomicUsize,
    mode: AtomicUsize,
}
impl Backend {
    fn new(other: ObjId) -> Self {
        Self {
            map: RwLock::new(BTreeMap::from([
                (GOD, AccountId::mint("requester", 1).unwrap()),
                (other, AccountId::mint("seller", 1).unwrap()),
            ])),
            ..Self::default()
        }
    }
    fn unlink_relink(&self, other: ObjId) {
        // Same final pair, different history. Sim can call the trait directly;
        // neither operation goes through Engine's account helper.
        let account = self.account_of(other).unwrap();
        assert!(self.unlink(other));
        assert!(self.link(other, account));
    }
}
impl AccountIdentity for Backend {
    fn account_of(&self, id: ObjId) -> Option<AccountId> {
        self.map.read().unwrap().get(&id).copied()
    }
    fn capture_personal_account(&self, requester: ObjId) -> Option<PersonalAccountSnapshot> {
        self.observations.fetch_add(1, Ordering::SeqCst);
        let table = self.map.read().unwrap();
        let captured = if self.mode.load(Ordering::SeqCst) == 1 {
            let (&foreign, &account) = table.iter().find(|(id, _)| **id != requester)?;
            self.epoch.personal_account(foreign, account)
        } else {
            self.epoch
                .personal_account(requester, *table.get(&requester)?)
        };
        drop(table);
        if self.mode.load(Ordering::SeqCst) == 2 {
            self.unlink_relink(requester);
        }
        captured
    }
    fn observe_distinct_accounts(
        &self,
        requester: ObjId,
        other: ObjId,
    ) -> Option<AccountPairSnapshot> {
        self.observations.fetch_add(1, Ordering::SeqCst);
        let table = self.map.read().unwrap();
        let pair = if self.mode.load(Ordering::SeqCst) == 1 {
            // A backend answering a different pair cannot change the actual
            // invocation requester or requested target.
            self.epoch.distinct_pair(
                other,
                *table.get(&other)?,
                requester,
                *table.get(&requester)?,
            )
        } else {
            self.epoch.distinct_pair(
                requester,
                *table.get(&requester)?,
                other,
                *table.get(&other)?,
            )
        };
        drop(table);
        if self.mode.load(Ordering::SeqCst) == 2 {
            self.unlink_relink(other);
        }
        pair
    }
    fn link(&self, id: ObjId, account: AccountId) -> bool {
        let mut map = self.map.write().unwrap();
        if let Some(old) = map.get(&id) {
            return *old == account;
        }
        self.epoch.invalidate();
        map.insert(id, account);
        true
    }
    fn unlink(&self, id: ObjId) -> bool {
        let mut map = self.map.write().unwrap();
        if !map.contains_key(&id) {
            return false;
        }
        self.epoch.invalidate();
        map.remove(&id).is_some()
    }
}

type Family = StateFamilyHandle<u32, u32>;
#[derive(Clone)]
struct Handles {
    action: DefinitionRef,
    family: Family,
}
struct Module {
    personal: bool,
    other: ObjId,
    backend: Arc<Backend>,
    mode: Arc<AtomicUsize>,
    answers: Arc<Mutex<Vec<Result<(ObjId, ObjId), ParticipantError>>>>,
    handles: Arc<Mutex<Option<Handles>>>,
}
fn definition(reg: &mut Registrar<'_>, kind: &str, id: &str) -> Result<DefinitionRef, ModError> {
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
        "account_test"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        for (kind, id) in [
            ("action", "observe"),
            ("outcome", "observed"),
            ("participant", "effect"),
        ] {
            reg.definition_validator(
                DefinitionKind::new(kind).unwrap(),
                move |ctx: &DefinitionValidationContext<'_>| {
                    let row = ctx.definition();
                    if row.reference().key().owner.as_str() == "account_test"
                        && row.reference().key().id.as_str() == id
                        && row.reference().version() == 1
                        && row.payload() == &json!({})
                        && row.references().is_empty()
                    {
                        Ok(())
                    } else {
                        Err("unexpected account observation fixture definition".into())
                    }
                },
            )?;
        }
        let action = definition(reg, "action", "observe")?;
        let outcome = definition(reg, "outcome", "observed")?;
        let family = reg.state_family(
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
        reg.state(EffectState(10));
        let participant = definition(reg, "participant", "effect")?;
        reg.owner_participant(
            participant,
            BTreeSet::from([action.clone()]),
            Effect {
                backend: self.backend.clone(),
                other: self.other,
                mode: self.mode.clone(),
            },
        )?;
        let effect = reg.request_owner_participant::<Effect>(action.clone())?;
        reg.action_provider(
            action.clone(),
            Provider {
                personal: self.personal,
                other: self.other,
                mode: self.mode.clone(),
                answers: self.answers.clone(),
                outcome,
                family: family.clone(),
                effect,
            },
        )?;
        *self.handles.lock().unwrap() = Some(Handles { action, family });
        Ok(())
    }
}
#[derive(Clone)]
struct EffectState(u32);
struct Effect {
    backend: Arc<Backend>,
    other: ObjId,
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
                namespace: OwnerId::new("account_test").unwrap(),
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
        if self.mode.load(Ordering::SeqCst) == 3 {
            self.backend.unlink_relink(self.other);
        }
        Ok(())
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut EffectState,
        _: (),
    ) -> Result<BoundEffect<'a>, ParticipantError> {
        if self.mode.load(Ordering::SeqCst) == 4 {
            self.backend.unlink_relink(self.other);
        }
        Ok(BoundEffect(state))
    }
    fn accepts_write(&self, w: &ReplayWrite) -> bool {
        matches!(w, ReplayWrite::ModuleJsonSet { namespace, key, value }
            if namespace.as_str() == "account_test" && key.as_str() == "effect" && value == &json!(11))
    }
}
struct Provider {
    personal: bool,
    other: ObjId,
    mode: Arc<AtomicUsize>,
    answers: Arc<Mutex<Vec<Result<(ObjId, ObjId), ParticipantError>>>>,
    outcome: DefinitionRef,
    family: Family,
    effect: ParticipantHandle<Effect>,
}
fn rejected() -> ActionError {
    ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        "account fixture refused",
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
        let preparation = ctx.commit_token();
        let calls = if mode == 5 || mode == 8 {
            MAX_ACCOUNT_OBSERVATIONS + 1
        } else if mode == 7 {
            MAX_ACCOUNT_OBSERVATIONS
        } else {
            1
        };
        // Prepare the independent effect before the deliberately ignored
        // account refusal. No later context API may surface the sticky error
        // on behalf of Engine's final provider-result check.
        ctx.prepare_owner(&self.effect, ())
            .map_err(|_| rejected())?;
        for ordinal in 0..calls {
            let other = if mode == 6 { GOD } else { self.other };
            let result = if self.personal && (!(mode == 7 || mode == 8) || ordinal % 2 == 0) {
                ctx.personal_account().map(|proof| {
                    assert!(proof.matches_preparation(preparation));
                    assert!(!proof.matches_preparation(crate::CommitToken::from_bytes([99; 16])));
                    assert!(proof.matches_account(AccountId::mint("requester", 1).unwrap()));
                    assert!(!proof.matches_account(AccountId::mint("seller", 1).unwrap()));
                    let debug = format!("{proof:?}");
                    assert!(
                        !debug.contains(
                            &AccountId::mint("requester", 1)
                                .unwrap()
                                .canonical_hash()
                                .to_hex()
                        )
                    );
                    (proof.requester(), proof.requester())
                })
            } else {
                ctx.distinct_personal_accounts(other)
                    .map(|proof| (proof.requester(), proof.other()))
            };
            // A deliberate catch must not authorize the staged fallback row.
            self.answers.lock().unwrap().push(result);
        }
        txn.insert(&self.family, &1, 42).map_err(|_| rejected())?;
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(self.outcome.clone(), json!({"observed":true})).unwrap(),
        ))
    }
}
struct Rig {
    engine: Engine,
    other: ObjId,
    backend: Arc<Backend>,
    mode: Arc<AtomicUsize>,
    answers: Arc<Mutex<Vec<Result<(ObjId, ObjId), ParticipantError>>>>,
    handles: Handles,
    invocation: ActionInvocation,
}
fn rig() -> Rig {
    rig_kind(false, true)
}
fn personal_rig() -> Rig {
    rig_kind(true, true)
}
fn rig_kind(personal: bool, install_oracle: bool) -> Rig {
    let mut world = World::new_minimal();
    let other = world
        .create(ObjKind::Player, "Other", GOD, Some(LIMBO))
        .unwrap();
    let backend = Arc::new(Backend::new(other));
    let mode = Arc::new(AtomicUsize::new(0));
    let answers = Arc::new(Mutex::new(Vec::new()));
    let handles = Arc::new(Mutex::new(None));
    let mut engine = Engine::with_modules(
        world,
        Registry::new(),
        vec![Box::new(Module {
            personal,
            other,
            backend: backend.clone(),
            mode: mode.clone(),
            answers: answers.clone(),
            handles: handles.clone(),
        })],
    )
    .unwrap();
    if install_oracle {
        engine.set_account_identity(backend.clone());
    }
    let handles = handles.lock().unwrap().clone().unwrap();
    let invocation = invocation(PrincipalId::Object { id: GOD });
    Rig {
        engine,
        other,
        backend,
        mode,
        answers,
        handles,
        invocation,
    }
}

#[path = "personal_tests.rs"]
mod personal_tests;

#[test]
fn personal_proof_matches_each_actual_preparation_token_and_not_another() {
    for byte in [7, 8] {
        let mut r = personal_rig();
        let prepared = r.engine.prepare_registered_composed_authoritative_action(
            &r.handles.action,
            &r.invocation,
            crate::CommitToken::from_bytes([byte; 16]),
        );
        // The provider checks both exact current-token match and another-token
        // refusal before returning. Two actual Engine tokens rule out a
        // fixture-constant token accidentally standing in for PrepareCtx.
        assert!(prepared.is_ok());
        assert_eq!(r.answers.lock().unwrap().as_slice(), [Ok((GOD, GOD))]);
        r.unchanged();
    }
}

fn invocation(principal: PrincipalId) -> ActionInvocation {
    let request = RequestUid::from_bytes([8; 16]);
    ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor: GOD,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([9; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: json!({"requester":999,"principal":"seller"}),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new("account_test").unwrap(),
                namespace: CanonicalId::new("observe").unwrap(),
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })
    .unwrap()
}
impl Rig {
    fn prepare(
        &mut self,
    ) -> Result<
        (
            crate::composed::PreparedComposedTxn,
            crate::action::PreparedAuthoritativeAction,
        ),
        crate::action::ActionProviderPrepareError,
    > {
        self.engine
            .prepare_registered_composed_authoritative_action(
                &self.handles.action,
                &self.invocation,
                crate::CommitToken::from_bytes([7; 16]),
            )
    }
    fn unchanged(&self) {
        assert_eq!(self.engine.action_state().revision(), 0);
        assert_eq!(self.engine.action_state().len(), 0);
        assert_eq!(
            self.engine.world.module_state::<EffectState>().unwrap().0,
            10
        );
    }
}

#[test]
fn real_provider_binds_actual_invocation_pair_and_installs_without_oracle_callback() {
    let mut r = rig();
    let (prepared, _) = r.prepare().unwrap();
    r.unchanged();
    assert_eq!(r.answers.lock().unwrap().as_slice(), [Ok((GOD, r.other))]);
    assert_eq!(r.backend.observations.load(Ordering::SeqCst), 1);
    // If installation called the oracle, this mode would return a foreign pair.
    r.backend.mode.store(1, Ordering::SeqCst);
    let (_, install) = prepared.into_parts();
    r.engine
        .install_composed_action_announcing(
            install,
            &mut Outbox::new(),
            &mut crate::engine::Effects::new(),
        )
        .unwrap();
    assert_eq!(r.backend.observations.load(Ordering::SeqCst), 1);
    assert_eq!(
        r.engine.action_state().get(&r.handles.family, &1).unwrap(),
        Some(&42)
    );
    assert_eq!(r.engine.world.module_state::<EffectState>().unwrap().0, 11);
}

#[test]
fn direct_backend_unlink_relink_and_engine_replacement_invalidate_retained_observations() {
    for replacement in [false, true] {
        let mut r = rig();
        let (prepared, _) = r.prepare().unwrap();
        prepared.recheck(&r.engine.world).unwrap();
        if replacement {
            r.engine
                .set_account_identity(Arc::new(Backend::new(r.other)));
        } else {
            r.backend.unlink_relink(r.other);
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
        assert_eq!(r.backend.observations.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn caught_unknown_same_account_crossed_pair_and_capture_race_refuse_whole_preparation() {
    for case in 0..4 {
        let mut r = rig();
        match case {
            0 => {
                assert!(r.backend.unlink(r.other));
            }
            1 => {
                let requester = r.backend.account_of(GOD).unwrap();
                assert!(r.backend.unlink(r.other));
                assert!(r.backend.link(r.other, requester));
            }
            2 => r.backend.mode.store(1, Ordering::SeqCst),
            _ => r.backend.mode.store(2, Ordering::SeqCst),
        }
        assert!(r.prepare().is_err(), "case {case}");
        assert!(r.answers.lock().unwrap().last().unwrap().is_err());
        r.unchanged();
        assert!(!r.engine.action_state().has_prepared());
        assert!(!r.engine.action_state().reservation_lost());
    }
}

#[test]
fn account_observation_limit_poisoning_does_not_discard_prior_captures() {
    let mut r = rig();
    r.mode.store(5, Ordering::SeqCst);
    assert!(r.prepare().is_err());
    r.unchanged();
    let answers = r.answers.lock().unwrap();
    assert_eq!(answers.len(), MAX_ACCOUNT_OBSERVATIONS + 1);
    assert_eq!(
        answers.iter().filter(|answer| answer.is_ok()).count(),
        MAX_ACCOUNT_OBSERVATIONS
    );
    assert!(answers.last().unwrap().is_err());
    assert_eq!(
        r.backend.observations.load(Ordering::SeqCst),
        MAX_ACCOUNT_OBSERVATIONS
    );
}

#[test]
fn actual_personal_principal_and_live_distinct_players_are_required_before_oracle() {
    for mode in 0..5 {
        let mut r = rig();
        match mode {
            0 => r.invocation = invocation(PrincipalId::Object { id: r.other }),
            1 => r
                .engine
                .world
                .get_mut(r.other)
                .unwrap()
                .flags
                .insert(Flags::GOING),
            2 => r
                .engine
                .world
                .get_mut(GOD)
                .unwrap()
                .flags
                .insert(Flags::GOING),
            3 => r.engine.world.get_mut(r.other).unwrap().kind = ObjKind::Thing,
            _ => r.mode.store(6, Ordering::SeqCst),
        }
        assert!(r.prepare().is_err(), "mode {mode}");
        assert_eq!(r.backend.observations.load(Ordering::SeqCst), 0);
        r.unchanged();
    }
}

#[test]
fn legacy_oracle_without_locked_observation_support_cannot_attest_commercial_pair() {
    struct Legacy;
    impl AccountIdentity for Legacy {
        fn account_of(&self, id: ObjId) -> Option<AccountId> {
            Some(AccountId::mint(&id.0.to_string(), 1).unwrap())
        }
    }
    let mut r = rig();
    r.engine.set_account_identity(Arc::new(Legacy));
    assert!(r.prepare().is_err());
    r.unchanged();
    assert!(r.answers.lock().unwrap().last().unwrap().is_err());
}

#[test]
fn final_owner_recheck_and_bind_cannot_unlink_accounts_then_install_any_image() {
    for mode in [3, 4] {
        let mut r = rig();
        let (prepared, _) = r.prepare().unwrap();
        r.mode.store(mode, Ordering::SeqCst);
        if mode == 3 {
            assert!(prepared.recheck(&r.engine.world).is_err());
        }
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
        assert_eq!(r.backend.observations.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn exhausted_backend_epoch_refuses_new_and_old_observations() {
    let mut r = rig();
    let (prepared, _) = r.prepare().unwrap();
    r.backend.epoch.0.store(u64::MAX - 1, Ordering::SeqCst);
    r.backend.epoch.invalidate();
    r.backend.epoch.invalidate();
    assert!(prepared.recheck(&r.engine.world).is_err());
    assert_eq!(r.backend.epoch.0.load(Ordering::SeqCst), u64::MAX);
    drop(prepared);
    // The old prepared reservation may itself fail-stop on drop. A fresh
    // Engine proves exhausted capture refusal independently of that state.
    let mut r = rig();
    r.backend.epoch.0.store(u64::MAX, Ordering::SeqCst);
    assert!(r.prepare().is_err());
    assert_eq!(r.backend.observations.load(Ordering::SeqCst), 1);
    assert_eq!(
        r.answers.lock().unwrap().as_slice(),
        [Err(ParticipantError::Authority)]
    );
    assert!(!r.engine.action_state().reservation_lost());
    r.unchanged();
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
        token: crate::CommitToken::from_bytes([7; 16]),
        request: r.invocation.request(),
        idempotency: r.invocation.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([3; 32]),
        request_hash: r.invocation.request_hash(),
        plan_hash: CanonicalHash::from_bytes([5; 32]),
        action: r.handles.action.clone(),
        provider: OwnerId::new("account_test").unwrap(),
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
fn actual_stage_and_take_refuse_invalid_account_capture_without_poisoning_known_abort() {
    for after_stage in [false, true] {
        let mut r = rig();
        let (prepared, output) = r.prepare().unwrap();
        let bundle = bundle(&r, &prepared, output);
        let guards = crate::action::ServerGuardSetBuilder::new(
            crate::action::RevisionSet::default(),
            r.engine.registry.mechanics_owner_readiness.clone().unwrap(),
        )
        .finish()
        .unwrap();
        let _ = r.engine.world.take_dirty();
        if after_stage {
            r.engine
                .stage_authoritative_action(prepared, guards, bundle)
                .unwrap();
            r.backend.unlink_relink(r.other);
            assert!(r.engine.take_staged_action().is_err());
        } else {
            r.backend.unlink_relink(r.other);
            assert!(
                r.engine
                    .stage_authoritative_action(prepared, guards, bundle)
                    .is_err()
            );
        }
        r.unchanged();
        assert!(!r.engine.has_staged_action());
        assert!(!r.engine.action_state().has_prepared());
        assert!(
            !r.engine.action_state().reservation_lost(),
            "known pre-custody refusal aborts exactly"
        );
        assert_eq!(r.backend.observations.load(Ordering::SeqCst), 1);
    }
}

#![cfg(test)]
//! Real composed providers exercise admission without a physical consumer.
use super::{LocomotionAdmission, LocomotionDecision, LocomotionRequest};
use crate::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionInvocationDraft, ActionOfferId,
    ActionProvider, AuthoritativeActionOutput, CanonicalHash, CommitToken, IdempotencyKey,
    IdempotencyScope, PerceivedRef, PrincipalId, RequestUid, ScopedIdempotencyKey,
};
use crate::action_state::{ActionTxn, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId};
use crate::authoritative::ReplayWrite;
use crate::lock::{LockType, parse_lock};
use crate::mechanics::{
    CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionValidationContext, OwnerId, RulesValue,
};
use crate::{
    Engine, EventHook, GOD, GameModule, HookPoint, HookReadContext, LIMBO, ModError, ObjId,
    ObjKind, Registrar, Registry, World,
};
use serde_json::json;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
enum Route {
    Exit(String),
    Home,
}
impl Route {
    fn request(&self) -> LocomotionRequest {
        match self {
            Self::Exit(name) => LocomotionRequest::exit_name(name).unwrap(),
            Self::Home => LocomotionRequest::home(),
        }
    }
}
#[derive(Clone, Copy)]
enum Mode {
    Capture,
    Recheck,
    IgnoreAdmissionFailure,
    IgnoreRecheckFailure,
}
struct Control {
    route: Route,
    mode: Mode,
    admission: Option<LocomotionAdmission>,
    already_home: bool,
}
#[derive(Clone)]
struct Handles {
    action: DefinitionRef,
    other_action: DefinitionRef,
    marker: StateFamilyHandle<u32, u32>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Attempt {
    actor: ObjId,
    exit: Option<ObjId>,
    destination: ObjId,
}
fn descriptor(kind: &str, id: &str) -> DefinitionDescriptor {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).unwrap(),
        DefinitionId::new(id).unwrap(),
        1,
        json!({"fixture":"locomotion_admission"}),
    )
    .unwrap()
}
fn validator(
    reg: &mut Registrar<'_>,
    descriptors: Vec<DefinitionDescriptor>,
) -> Result<(), ModError> {
    reg.definition_validator(
        descriptors[0].kind().clone(),
        move |ctx: &DefinitionValidationContext<'_>| {
            let definition = ctx.definition();
            if descriptors.iter().any(|candidate| {
                definition.reference().key().id == *candidate.id()
                    && definition.reference().version() == candidate.version()
                    && definition.payload() == candidate.payload()
            }) {
                Ok(())
            } else {
                Err("unexpected locomotion fixture definition".into())
            }
        },
    )
}
fn family(reg: &mut Registrar<'_>) -> Result<StateFamilyHandle<u32, u32>, ModError> {
    reg.state_family(
        StateFamilyDescriptor::new(
            StateFamilyId::new("records").unwrap(),
            CanonicalId::new("u32.decimal.v1").unwrap(),
            CanonicalId::new("u32.value.v1").unwrap(),
            1,
            CanonicalId::new("u32.decode.v1").unwrap(),
            CanonicalId::new("u32.validate.v1").unwrap(),
            CanonicalId::new("u32.serialize.v1").unwrap(),
        )
        .unwrap(),
        |key: &u32| Ok(key.to_string()),
        |value: &serde_json::Value| {
            serde_json::from_value(value.clone()).map_err(|e| e.to_string())
        },
        |_value: &u32| Ok(()),
        |value: &u32| serde_json::to_value(value).map_err(|e| e.to_string()),
    )
}
struct ProviderModule {
    control: Arc<Mutex<Control>>,
    handles: Arc<Mutex<Option<Handles>>>,
}
impl GameModule for ProviderModule {
    fn name(&self) -> &'static str {
        "locomotion_fixture"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let action = descriptor("action", "admit");
        let other_action = descriptor("action", "other_admit");
        let outcome = descriptor("outcome", "admitted");
        validator(reg, vec![action.clone(), other_action.clone()])?;
        validator(reg, vec![outcome.clone()])?;
        let action = reg.definition(action)?;
        let other_action = reg.definition(other_action)?;
        let outcome = reg.definition(outcome)?;
        let marker = family(reg)?;
        for selected in [&action, &other_action] {
            reg.action_provider(
                selected.clone(),
                Provider {
                    control: Arc::clone(&self.control),
                    marker: marker.clone(),
                    outcome: outcome.clone(),
                },
            )?;
        }
        *self.handles.lock().unwrap() = Some(Handles {
            action,
            other_action,
            marker,
        });
        Ok(())
    }
}
struct Provider {
    control: Arc<Mutex<Control>>,
    marker: StateFamilyHandle<u32, u32>,
    outcome: DefinitionRef,
}
fn refused() -> ActionError {
    ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        "locomotion refused",
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
        ctx: &mut crate::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let mut control = self.control.lock().unwrap();
        match control.mode {
            Mode::Capture => match ctx
                .admit_locomotion(txn, control.route.request())
                .map_err(|_| refused())?
            {
                LocomotionDecision::AlreadyHome => {
                    control.already_home = true;
                    control.admission = None;
                }
                LocomotionDecision::Admitted(admission) => {
                    control.already_home = false;
                    control.admission = Some(admission);
                }
            },
            Mode::Recheck => ctx
                .require_locomotion_admission(txn, control.admission.as_ref().unwrap())
                .map_err(|_| refused())?,
            Mode::IgnoreAdmissionFailure => {
                txn.insert(&self.marker, &0, 1).unwrap();
                assert!(ctx.admit_locomotion(txn, control.route.request()).is_err());
            }
            Mode::IgnoreRecheckFailure => {
                txn.insert(&self.marker, &0, 1).unwrap();
                assert!(
                    ctx.require_locomotion_admission(txn, control.admission.as_ref().unwrap())
                        .is_err()
                );
            }
        }
        if matches!(control.mode, Mode::Capture | Mode::Recheck) {
            // The coordinator requires one logical effect. This explicit
            // fixture marker is independent of the inert admission itself.
            txn.insert(&self.marker, &0, 1).unwrap();
        }
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(self.outcome.clone(), json!({})).unwrap(),
        )
        .with_definitions(vec![self.outcome.clone()]))
    }
}
struct TypedHook {
    state: StateFamilyHandle<u32, u32>,
    attempts: Arc<Mutex<Vec<Attempt>>>,
}
impl EventHook for TypedHook {
    fn move_attempt(
        &self,
        ctx: &HookReadContext<'_>,
        actor: ObjId,
        exit: Option<ObjId>,
        destination: ObjId,
    ) -> Result<(), mush_proto::RichText> {
        assert!(ctx.physical_transition_attempt().is_none());
        self.attempts.lock().unwrap().push(Attempt {
            actor,
            exit,
            destination,
        });
        match ctx.action_state(&self.state, &0) {
            Ok(Some(0)) => Ok(()),
            _ => Err(mush_proto::RichText::plain("current typed movement veto")),
        }
    }
}
struct HookModule {
    handle: Arc<Mutex<Option<StateFamilyHandle<u32, u32>>>>,
    attempts: Arc<Mutex<Vec<Attempt>>>,
}
impl GameModule for HookModule {
    fn name(&self) -> &'static str {
        "locomotion_hook"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let handle = family(reg)?;
        reg.hook(
            HookPoint::MoveAttempt,
            Arc::new(TypedHook {
                state: handle.clone(),
                attempts: Arc::clone(&self.attempts),
            }),
        );
        *self.handle.lock().unwrap() = Some(handle);
        Ok(())
    }
}
struct Rig {
    engine: Engine,
    handles: Handles,
    control: Arc<Mutex<Control>>,
    hook_state: StateFamilyHandle<u32, u32>,
    attempts: Arc<Mutex<Vec<Attempt>>>,
    actor: ObjId,
    source: ObjId,
    destination: ObjId,
    exit: ObjId,
}
fn token() -> CommitToken {
    CommitToken::from_bytes([19; 16])
}
fn assert_fixture_marker_only(writes: &[ReplayWrite]) {
    assert!(
        matches!(writes, [ReplayWrite::ModuleJsonSet { namespace, key, value }]
            if namespace.as_str() == "locomotion_fixture"
                && key.as_str() == "action-state/v1/records/0"
                && value == &json!({
                    "family": {"owner":"locomotion_fixture", "id":"records"},
                    "key":"0",
                    "schema":{"id":"u32.value.v1", "version":1},
                    "value":1,
                })
        ),
        "unexpected admission fixture writes: {writes:?}"
    );
}
fn rig() -> Rig {
    let mut world = World::new_minimal();
    let source = world.create(ObjKind::Room, "source", GOD, None).unwrap();
    let destination = world
        .create(ObjKind::Room, "destination", GOD, None)
        .unwrap();
    let actor = world
        .create(ObjKind::Player, "walker", GOD, Some(source))
        .unwrap();
    let exit = world
        .create(ObjKind::Exit, "north;n", GOD, Some(source))
        .unwrap();
    world.link_exit(exit, Some(destination)).unwrap();
    world.set_home(actor, Some(destination)).unwrap();
    world.take_dirty();
    let control = Arc::new(Mutex::new(Control {
        route: Route::Exit("north".into()),
        mode: Mode::Capture,
        admission: None,
        already_home: false,
    }));
    let handles = Arc::new(Mutex::new(None));
    let hook_handle = Arc::new(Mutex::new(None));
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::with_modules(
        world,
        Registry::new(),
        vec![
            Box::new(ProviderModule {
                control: Arc::clone(&control),
                handles: Arc::clone(&handles),
            }),
            Box::new(HookModule {
                handle: Arc::clone(&hook_handle),
                attempts: Arc::clone(&attempts),
            }),
        ],
    )
    .unwrap();
    let hook_state = hook_handle.lock().unwrap().clone().unwrap();
    let mut txn = engine.action_transaction(&hook_state).unwrap();
    txn.insert(&hook_state, &0, 0).unwrap();
    let (_, installation) = txn.prepare().unwrap().into_parts();
    engine.install_prepared_action(installation).unwrap();
    let handles = handles.lock().unwrap().clone().unwrap();
    Rig {
        engine,
        handles,
        control,
        hook_state,
        attempts,
        actor,
        source,
        destination,
        exit,
    }
}
impl Rig {
    fn draft(&self) -> ActionInvocationDraft {
        let request = RequestUid::from_bytes([19; 16]);
        let principal = PrincipalId::Object { id: self.actor };
        ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor: self.actor,
            offered_action: ActionOfferId(CanonicalHash::from_bytes([7; 32])),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters: json!({}),
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: OwnerId::new("locomotion_fixture").unwrap(),
                    namespace: CanonicalId::new("admission").unwrap(),
                },
                1,
                IdempotencyKey::Request(request),
            ),
        }
    }
    fn invocation(&self) -> ActionInvocation {
        ActionInvocation::new(self.draft()).unwrap()
    }
    fn prepare_with(
        &mut self,
        invocation: ActionInvocation,
        action: DefinitionRef,
        commit: CommitToken,
    ) -> Result<Vec<ReplayWrite>, String> {
        let (prepared, _) = self
            .engine
            .prepare_registered_composed_authoritative_action(&action, &invocation, commit)
            .map_err(|e| e.to_string())?;
        let writes = prepared.authoritative_replay_writes();
        assert_fixture_marker_only(&writes);
        let (_, installation) = prepared.into_parts();
        self.engine.abort_prepared_action(installation).unwrap();
        assert!(
            self.engine
                .action_state()
                .get(&self.handles.marker, &0)
                .unwrap()
                .is_none()
        );
        Ok(writes)
    }
    fn prepare(&mut self) -> Result<Vec<ReplayWrite>, String> {
        self.prepare_with(self.invocation(), self.handles.action.clone(), token())
    }
    fn recheck(&mut self) -> Result<Vec<ReplayWrite>, String> {
        self.control.lock().unwrap().mode = Mode::Recheck;
        self.prepare()
    }
    fn set_hook_state(&mut self, value: u32) {
        let mut txn = self.engine.action_transaction(&self.hook_state).unwrap();
        txn.replace(&self.hook_state, &0, value).unwrap();
        let (_, installation) = txn.prepare().unwrap().into_parts();
        self.engine.install_prepared_action(installation).unwrap();
    }
    fn route(&self) -> Attempt {
        let control = self.control.lock().unwrap();
        let admission = control.admission.as_ref().unwrap();
        assert_eq!(admission.source(), Some(self.source));
        Attempt {
            actor: admission.actor(),
            exit: admission.exit(),
            destination: admission.destination(),
        }
    }
}

#[test]
fn composed_admission_is_inert_and_passes_the_actual_exit_to_foreign_owner_hook() {
    let mut r = rig();
    let before = r.engine.world.get(r.actor).unwrap().clone();
    assert_fixture_marker_only(&r.prepare().unwrap());
    let expected = Attempt {
        actor: r.actor,
        exit: Some(r.exit),
        destination: r.destination,
    };
    assert_eq!(r.route(), expected);
    assert_eq!(*r.attempts.lock().unwrap(), vec![expected]);
    assert_eq!(r.engine.world.get(r.actor), Some(&before));
    assert!(!r.engine.has_staged_action());
    assert!(!r.engine.has_legacy_persistence_work());
    assert!(!r.engine.has_authoritative_submit());
    assert_fixture_marker_only(&r.recheck().unwrap());
    assert_eq!(r.engine.world.get(r.actor), Some(&before));
}
#[test]
fn home_keeps_none_exit_and_already_home_produces_no_permit_or_hook_call() {
    let mut r = rig();
    r.control.lock().unwrap().route = Route::Home;
    assert_fixture_marker_only(&r.prepare().unwrap());
    assert_eq!(
        r.route(),
        Attempt {
            actor: r.actor,
            exit: None,
            destination: r.destination
        }
    );
    r.engine.world.set_home(r.actor, Some(r.source)).unwrap();
    r.attempts.lock().unwrap().clear();
    assert_fixture_marker_only(&r.prepare().unwrap());
    let control = r.control.lock().unwrap();
    assert!(control.already_home);
    assert!(control.admission.is_none());
    assert!(r.attempts.lock().unwrap().is_empty());
    assert_eq!(
        r.engine.world.get(r.actor).unwrap().location,
        Some(r.source)
    );
}
#[test]
fn request_bounds_are_bytes_and_home_cannot_be_disguised_as_an_exit_request() {
    assert!(LocomotionRequest::exit_name("").is_err());
    assert!(LocomotionRequest::exit_name("   ").is_err());
    assert!(LocomotionRequest::exit_name("HoMe").is_err());
    assert!(LocomotionRequest::exit_name(&"n".repeat(512)).is_ok());
    assert!(LocomotionRequest::exit_name(&"n".repeat(513)).is_err());
    assert!(LocomotionRequest::exit_name(&"é".repeat(257)).is_err());
}
#[test]
fn actual_invocation_requires_self_actor_subject_and_no_foreign_principal_or_targets() {
    for case in 0..3 {
        let mut r = rig();
        let mut draft = r.draft();
        match case {
            0 => draft.actor = GOD,
            1 => {
                draft.subject = PerceivedRef::KnownObject {
                    id: r.actor,
                    revision: 0,
                }
            }
            _ => draft.targets.push(PerceivedRef::KnownObject {
                id: r.destination,
                revision: 0,
            }),
        }
        assert!(
            r.prepare_with(
                ActionInvocation::new(draft).unwrap(),
                r.handles.action.clone(),
                token()
            )
            .is_err()
        );
        assert!(r.control.lock().unwrap().admission.is_none());
        assert!(r.attempts.lock().unwrap().is_empty());
    }
}
#[test]
fn permit_cannot_be_transplanted_to_another_actual_commit_action_or_invocation() {
    for case in 0..3 {
        let mut r = rig();
        r.prepare().unwrap();
        r.control.lock().unwrap().mode = Mode::Recheck;
        let mut draft = r.draft();
        if case == 2 {
            draft.parameters = json!({"different":true});
        }
        let action = if case == 1 {
            r.handles.other_action.clone()
        } else {
            r.handles.action.clone()
        };
        let commit = if case == 0 {
            CommitToken::from_bytes([20; 16])
        } else {
            token()
        };
        assert!(
            r.prepare_with(ActionInvocation::new(draft).unwrap(), action, commit)
                .is_err()
        );
    }
}
#[test]
fn ignored_admission_and_recheck_errors_poison_the_complete_transaction() {
    for recheck in [false, true] {
        let mut r = rig();
        if recheck {
            r.prepare().unwrap();
        }
        r.set_hook_state(1);
        r.control.lock().unwrap().mode = if recheck {
            Mode::IgnoreRecheckFailure
        } else {
            Mode::IgnoreAdmissionFailure
        };
        assert!(r.prepare().is_err());
        assert!(
            r.engine
                .action_state()
                .get(&r.handles.marker, &0)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            r.engine.world.get(r.actor).unwrap().location,
            Some(r.source)
        );
        r.set_hook_state(0);
        r.control.lock().unwrap().mode = Mode::Capture;
        assert_fixture_marker_only(&r.prepare().unwrap());
    }
}
#[test]
fn typed_hook_change_is_rechecked_even_when_every_world_object_is_unchanged() {
    let mut r = rig();
    r.prepare().unwrap();
    let before = r.engine.world.get(r.actor).unwrap().clone();
    r.set_hook_state(1);
    assert_eq!(r.engine.world.get(r.actor), Some(&before));
    assert!(r.recheck().is_err());
    assert_eq!(r.attempts.lock().unwrap().len(), 2);
}
#[test]
fn room_zone_global_precedence_and_alias_matching_use_the_same_live_lookup() {
    let mut r = rig();
    let zone = r
        .engine
        .world
        .create(ObjKind::Thing, "zone", GOD, Some(LIMBO))
        .unwrap();
    let zone_exit = r
        .engine
        .world
        .create(ObjKind::Exit, "north", GOD, Some(zone))
        .unwrap();
    let global_exit = r
        .engine
        .world
        .create(ObjKind::Exit, "north", GOD, Some(LIMBO))
        .unwrap();
    r.engine
        .world
        .link_exit(zone_exit, Some(r.destination))
        .unwrap();
    r.engine
        .world
        .link_exit(global_exit, Some(r.destination))
        .unwrap();
    r.engine.world.set_zone(r.source, Some(zone)).unwrap();
    r.control.lock().unwrap().route = Route::Exit("N".into());
    r.prepare().unwrap();
    assert_eq!(r.route().exit, Some(r.exit));
    r.control.lock().unwrap().route = Route::Exit("north".into());
    r.engine
        .world
        .set_flag(r.exit, crate::Flags::GOING, true)
        .unwrap();
    r.prepare().unwrap();
    assert_eq!(r.route().exit, Some(zone_exit));
    r.engine
        .world
        .set_flag(zone_exit, crate::Flags::GOING, true)
        .unwrap();
    r.prepare().unwrap();
    assert_eq!(r.route().exit, Some(global_exit));
}
#[test]
fn earlier_exit_shadow_relink_and_actor_motion_all_invalidate_the_old_route() {
    for case in 0..3 {
        let mut r = rig();
        if case == 0 {
            r.engine.world.move_to(r.exit, LIMBO).unwrap();
        }
        r.prepare().unwrap();
        match case {
            0 => {
                let new_exit = r
                    .engine
                    .world
                    .create(ObjKind::Exit, "north", GOD, Some(r.source))
                    .unwrap();
                r.engine
                    .world
                    .link_exit(new_exit, Some(r.destination))
                    .unwrap();
            }
            1 => r.engine.world.link_exit(r.exit, Some(r.source)).unwrap(),
            _ => r.engine.world.move_to(r.actor, r.destination).unwrap(),
        }
        assert!(r.recheck().is_err());
    }
}
#[test]
fn home_change_and_a_new_basic_lock_invalidate_old_admission() {
    for home in [false, true] {
        let mut r = rig();
        if home {
            r.control.lock().unwrap().route = Route::Home;
        }
        r.prepare().unwrap();
        if home {
            r.engine.world.set_home(r.actor, Some(r.source)).unwrap();
        } else {
            r.engine
                .world
                .get_mut(r.exit)
                .unwrap()
                .locks
                .insert(LockType::Basic, parse_lock("#FALSE", |_| None).unwrap());
        }
        assert!(r.recheck().is_err());
    }
}
#[test]
fn negated_missing_indirect_target_is_legacy_true_but_its_absence_is_fenced() {
    let mut r = rig();
    let missing = ObjId(r.engine.world.next_id());
    let lock = parse_lock(&format!("!@{missing}/basic"), |_| None).unwrap();
    r.engine
        .world
        .get_mut(r.exit)
        .unwrap()
        .locks
        .insert(LockType::Basic, lock);
    r.prepare().unwrap();
    let created = r
        .engine
        .world
        .create(ObjKind::Thing, "new lock source", GOD, Some(LIMBO))
        .unwrap();
    assert_eq!(created, missing);
    assert!(r.recheck().is_err());
}

#[test]
fn unrelated_existing_object_edit_does_not_invalidate_the_bounded_read_set() {
    let mut r = rig();
    let unrelated = r
        .engine
        .world
        .create(ObjKind::Thing, "unrelated", GOD, Some(LIMBO))
        .unwrap();
    r.prepare().unwrap();
    r.engine
        .world
        .set_name(unrelated, "changed elsewhere")
        .unwrap();
    assert_fixture_marker_only(&r.recheck().unwrap());
}

#[test]
fn standalone_context_cannot_mint_an_admission_and_poison_cannot_be_ignored() {
    let mut r = rig();
    let invocation = r.invocation();
    let mut ctx = crate::PrepareCtx::new(invocation.idempotency(), token(), None);
    let mut txn = r.engine.action_transaction(&r.handles.marker).unwrap();
    txn.insert(&r.handles.marker, &0, 1).unwrap();
    assert!(
        ctx.admit_locomotion(&mut txn, LocomotionRequest::home())
            .is_err()
    );
    assert!(txn.prepare().is_err());
    assert!(
        r.engine
            .action_state()
            .get(&r.handles.marker, &0)
            .unwrap()
            .is_none()
    );
}

#[test]
fn unlinked_or_locked_exits_refuse_before_the_move_hook() {
    for locked in [false, true] {
        let mut r = rig();
        if locked {
            r.engine
                .world
                .get_mut(r.exit)
                .unwrap()
                .locks
                .insert(LockType::Basic, parse_lock("#FALSE", |_| None).unwrap());
        } else {
            r.engine.world.link_exit(r.exit, None).unwrap();
        }
        assert!(r.prepare().is_err());
        assert!(r.control.lock().unwrap().admission.is_none());
        assert!(r.attempts.lock().unwrap().is_empty());
        assert_eq!(
            r.engine.world.get(r.actor).unwrap().location,
            Some(r.source)
        );
    }
}

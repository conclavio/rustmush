#![cfg(test)]

use super::*;
use crate::action::{
    ActionInvocationDraft, ActionOfferId, CanonicalHash, IdempotencyKey, IdempotencyScope,
    PreparedAuthoritativeAction, PreparedCausalRef, RequestUid, ScopedIdempotencyKey,
};
use crate::mechanics::DefinitionValidationContext;
use crate::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext,
};
use crate::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn,
    AuthoritativeActionOutput, AuthoritativeBundle, AuthoritativeBundleDraft, Engine, GOD,
    GameModule, ObjId, ObjKind, Outbox, PerceivedRef, PrincipalId, Registrar, Registry,
    ReplayWrite, RulesValue, World,
};
use std::collections::BTreeSet;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
};

#[derive(Clone)]
struct Cell<const N: u8>(u64);
#[derive(Clone)]
struct Control {
    empty: bool,
    projection: Option<ObjId>,
    refuse: Arc<AtomicU8>,
    bind_fail: Arc<AtomicU8>,
    ignore: Arc<AtomicBool>,
    callbacks: Arc<AtomicUsize>,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            empty: false,
            projection: None,
            refuse: Arc::new(AtomicU8::new(255)),
            bind_fail: Arc::new(AtomicU8::new(255)),
            ignore: Arc::default(),
            callbacks: Arc::default(),
        }
    }
}
struct Bound<'a> {
    value: &'a mut u64,
    after: u64,
}
impl BoundParticipant for Bound<'_> {
    fn apply(self) {
        *self.value = self.after;
    }
}
fn write(owner: &str, key: &str, value: u64) -> ReplayWrite {
    ReplayWrite::ModuleJsonSet {
        namespace: OwnerId::new(owner).unwrap(),
        key: crate::AuthoritativeKey::parse_canonical(key).unwrap(),
        value: json!(value),
    }
}
struct Primary(Control);
impl OwnerParticipant for Primary {
    type State = Cell<7>;
    type Request = super::super::transition::LocomotionIntent;
    type Output = super::super::transition::LocomotionDraft;
    type Patch = (u64, u64);
    type Bound<'a> = Bound<'a>;
    fn prepare(
        &self,
        state: &Self::State,
        ctx: &OwnerPrepareContext<'_>,
        intent: Self::Request,
    ) -> Result<OwnerPreparation<Self::Output, Self::Patch>, ParticipantError> {
        let mut dependencies = BTreeSet::from([intent.actor(), intent.destination()]);
        dependencies.extend(intent.source());
        let output = super::super::transition::LocomotionDraft::new(
            ctx,
            &intent,
            crate::DefinitionDigest::from_bytes([1; 32]),
            crate::DefinitionDigest::from_bytes([2; 32]),
            dependencies,
        )?;
        Ok(OwnerPreparation::new(
            output,
            (state.0, state.0 + 1),
            vec![write("sim", "primary", state.0 + 1)],
        ))
    }
    fn recheck(
        &self,
        state: &Self::State,
        _: &OwnerPrepareContext<'_>,
        patch: &Self::Patch,
    ) -> Result<(), ParticipantError> {
        if state.0 == patch.0 {
            Ok(())
        } else {
            Err(refused())
        }
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut Self::State,
        patch: Self::Patch,
    ) -> Result<Bound<'a>, ParticipantError> {
        if self.0.bind_fail.load(Ordering::SeqCst) == 7 || state.0 != patch.0 {
            return Err(refused());
        }
        Ok(Bound {
            value: &mut state.0,
            after: patch.1,
        })
    }
    fn accepts_write(&self, value: &ReplayWrite) -> bool {
        matches!(value, ReplayWrite::ModuleJsonSet { namespace, key, .. } if namespace.as_str()=="sim" && key.as_str()=="primary")
    }
}
struct WindowPrimary(Control);
impl OwnerParticipant for WindowPrimary {
    type State = Cell<7>;
    type Request = WindowLocomotionIntent;
    type Output = WindowLocomotionDraft;
    type Patch = (u64, u64);
    type Bound<'a> = Bound<'a>;
    fn prepare(
        &self,
        state: &Self::State,
        ctx: &OwnerPrepareContext<'_>,
        intent: Self::Request,
    ) -> Result<OwnerPreparation<Self::Output, Self::Patch>, ParticipantError> {
        let projected = self.0.projection.ok_or_else(refused)?;
        let mut dependencies = BTreeSet::from([intent.actor(), intent.destination(), projected]);
        dependencies.extend(intent.source());
        let preview = intent.preview_projection(ctx, &dependencies, projected)?;
        if preview
            .object(intent.actor())?
            .and_then(|actor| actor.location)
            != Some(projected)
        {
            return Err(refused());
        }
        let output = WindowLocomotionDraft::new(
            ctx,
            &intent,
            crate::DefinitionDigest::from_bytes([1; 32]),
            crate::DefinitionDigest::from_bytes([2; 32]),
            dependencies,
            WindowProjection::new(
                projected,
                crate::DefinitionDigest::from_bytes([3; 32]),
                crate::DefinitionDigest::from_bytes([4; 32]),
                (projected != intent.destination()).then_some(WindowMetricTarget::at(1, 2, 3, 4)),
            ),
        )?;
        Ok(OwnerPreparation::new(
            output,
            (state.0, state.0 + 1),
            vec![write("sim", "primary", state.0 + 1)],
        ))
    }
    fn recheck(
        &self,
        state: &Self::State,
        ctx: &OwnerPrepareContext<'_>,
        patch: &Self::Patch,
    ) -> Result<(), ParticipantError> {
        Primary(self.0.clone()).recheck(state, ctx, patch)
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut Self::State,
        patch: Self::Patch,
    ) -> Result<Self::Bound<'a>, ParticipantError> {
        Arc::new(Primary(self.0.clone())).bind(state, patch)
    }
    fn accepts_write(&self, value: &ReplayWrite) -> bool {
        Primary(self.0.clone()).accepts_write(value)
    }
}
struct Observer<const N: u8>(Control);
impl<const N: u8> OwnerParticipant for Observer<N> {
    type State = Cell<N>;
    type Request = MovementIntent;
    type Output = MovementDraft;
    type Patch = (u64, u64);
    type Bound<'a> = Bound<'a>;
    fn prepare(
        &self,
        state: &Self::State,
        ctx: &OwnerPrepareContext<'_>,
        intent: MovementIntent,
    ) -> Result<OwnerPreparation<MovementDraft, Self::Patch>, ParticipantError> {
        if self.0.refuse.load(Ordering::SeqCst) == N {
            return Err(refused());
        }
        if intent
            .window_projection()
            .map(WindowProjection::projected_room)
            != self.0.projection
        {
            return Err(refused());
        }
        let proof = json!({"original":state.0});
        let digest =
            crate::DefinitionDigest::for_canonical_value("composed_test.capture", &proof).unwrap();
        let output = MovementDraft::new(ctx, &intent, digest, proof)?;
        let after = state.0 + u64::from(!self.0.empty);
        let writes = if self.0.empty {
            vec![]
        } else {
            vec![write("observer", &format!("owner{N}"), after)]
        };
        Ok(OwnerPreparation::new(output, (state.0, after), writes))
    }
    fn recheck(
        &self,
        state: &Self::State,
        _: &OwnerPrepareContext<'_>,
        patch: &Self::Patch,
    ) -> Result<(), ParticipantError> {
        if state.0 == patch.0 {
            Ok(())
        } else {
            Err(refused())
        }
    }
    fn bind<'a>(
        self: Arc<Self>,
        state: &'a mut Self::State,
        patch: Self::Patch,
    ) -> Result<Bound<'a>, ParticipantError> {
        if self.0.bind_fail.load(Ordering::SeqCst) == N || state.0 != patch.0 {
            return Err(refused());
        }
        Ok(Bound {
            value: &mut state.0,
            after: patch.1,
        })
    }
    fn accepts_write(&self, value: &ReplayWrite) -> bool {
        matches!(value, ReplayWrite::ModuleJsonSet { namespace, key, .. }
            if namespace.as_str()=="observer" && key.as_str()==format!("owner{N}"))
    }
}
fn descriptor(kind: &str, id: &str) -> DefinitionDescriptor {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).unwrap(),
        DefinitionId::new(id).unwrap(),
        1,
        json!({}),
    )
    .unwrap()
    .as_current()
}
fn action_ref() -> DefinitionRef {
    DefinitionRegistryBuilder::new()
        .register(OwnerId::new("sim").unwrap(), action_descriptor().unwrap())
        .unwrap()
}
fn selected_action(control: &Control) -> DefinitionRef {
    if control.projection.is_none() {
        return action_ref();
    }
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("sim").unwrap(),
            window_action_descriptor().unwrap(),
        )
        .unwrap()
}
fn ordinary_records(
    reader: &ComposedLocomotionReader,
) -> &crate::StateFamilyHandle<crate::CommitToken, super::record::Record> {
    match &reader.records {
        super::record::Records::Ordinary(handle) => handle,
        _ => panic!("ordinary fixture"),
    }
}
fn participant<const N: u8>() -> DefinitionRef {
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("observer").unwrap(),
            descriptor("owner_participant", &format!("part{N}")),
        )
        .unwrap()
}
fn observer_ref<const N: u8>() -> DefinitionRef {
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("observer").unwrap(),
            crate::moved_observer::descriptor(&format!("observer{N}"), 1).unwrap(),
        )
        .unwrap()
}
fn register_observer<const N: u8>(
    reg: &mut Registrar<'_>,
    control: &Control,
) -> Result<(), ModError> {
    reg.state(Cell::<N>(0));
    let part = reg.definition(descriptor("owner_participant", &format!("part{N}")))?;
    reg.owner_participant(
        part.clone(),
        BTreeSet::from([selected_action(control)]),
        Observer::<N>(control.clone()),
    )?;
    let calls = Arc::clone(&control.callbacks);
    let observer = reg.subscribe_moved(
        &format!("observer{N}"),
        1,
        Arc::new(move |_: &mut crate::Ctx<'_>, _: &crate::WorldEvent| {
            calls.fetch_add(1, Ordering::SeqCst);
        }),
    )?;
    reg.register_moved_completion::<Observer<N>>(
        &format!("binding{N}"),
        1,
        selected_action(control),
        part,
        vec![observer],
    )?;
    Ok(())
}
struct ObserverModule {
    count: u8,
    control: Control,
}
impl GameModule for ObserverModule {
    fn name(&self) -> &'static str {
        "observer"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.definition_validator(
            DefinitionKind::new("owner_participant").unwrap(),
            |_: &DefinitionValidationContext<'_>| Ok(()),
        )?;
        macro_rules! register { ($($n:literal),*) => { $(if self.count > $n { register_observer::<$n>(reg, &self.control)?; })* }; }
        register!(0, 1, 2, 3, 4, 5, 6);
        Ok(())
    }
}
#[derive(Clone)]
struct Handles {
    action: DefinitionRef,
    reader: ComposedLocomotionReader,
    owner_reader: ComposedOwnerReader,
}
struct SimModule {
    count: u8,
    control: Control,
    handles: Arc<OnceLock<Handles>>,
}
impl GameModule for SimModule {
    fn name(&self) -> &'static str {
        "sim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        for kind in ["action", "outcome", "owner_participant"] {
            reg.definition_validator(
                DefinitionKind::new(kind).unwrap(),
                |_: &DefinitionValidationContext<'_>| Ok(()),
            )?;
        }
        let window = self.control.projection.is_some();
        let action = reg.definition(if window {
            window_action_descriptor()?
        } else {
            action_descriptor()?
        })?;
        let outcome = reg.definition(descriptor("outcome", "result"))?;
        let primary = reg.definition(if window {
            DefinitionDescriptor::new(
                DefinitionKind::new("owner_participant").unwrap(),
                DefinitionId::new(SIM_PARTICIPANT).unwrap(),
                2,
                json!({}),
            )
            .unwrap()
            .as_current()
        } else {
            descriptor("owner_participant", SIM_PARTICIPANT)
        })?;
        reg.state(Cell::<7>(0));
        let mut builder = if window {
            reg.owner_participant(
                primary.clone(),
                BTreeSet::from([action.clone()]),
                WindowPrimary(self.control.clone()),
            )?;
            ComposedLocomotionBuilder::new_window::<WindowPrimary>(reg, action.clone(), primary)?
        } else {
            reg.owner_participant(
                primary.clone(),
                BTreeSet::from([action.clone()]),
                Primary(self.control.clone()),
            )?;
            ComposedLocomotionBuilder::new::<Primary>(reg, action.clone(), primary)?
        };
        macro_rules! owners { ($($n:literal),*) => { $(if self.count > $n {
            builder = builder.owner::<Observer<$n>>(reg, participant::<$n>(), vec![observer_ref::<$n>()])?;
        })* }; }
        owners!(0, 1, 2, 3, 4, 5, 6);
        let service = builder.finish(reg)?;
        self.handles
            .set(Handles {
                action: action.clone(),
                reader: service.reader(),
                owner_reader: service.owner_reader(&participant::<0>()).map_err(bad)?,
            })
            .map_err(|_| bad("handles reused"))?;
        reg.action_provider(
            action,
            Provider {
                service,
                outcome,
                control: self.control.clone(),
            },
        )
    }
}
struct Provider {
    service: ComposedLocomotion,
    outcome: DefinitionRef,
    control: Control,
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, _: &ActionInvocation) -> Result<(), ActionError> {
        Ok(())
    }
    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut crate::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let error = || {
            ActionError::new(
                ActionErrorCode::Unavailable,
                false,
                "composed fixture refused",
            )
            .unwrap()
        };
        ctx.admit_captured_locomotion(txn).map_err(|_| error())?;
        let prepared = ctx.prepare_composed_locomotion(txn, &self.service);
        if !self.control.ignore.load(Ordering::SeqCst) {
            prepared.map_err(|_| error())?;
        }
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(self.outcome.clone(), json!({})).unwrap(),
        )
        .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}
struct Rig {
    engine: Box<Engine>,
    handles: Handles,
    control: Control,
    actor: ObjId,
    source: ObjId,
    destination: ObjId,
}
impl Rig {
    fn new(count: u8, empty: bool) -> Self {
        Self::configured(count, empty, None)
    }
    fn configured(count: u8, empty: bool, window: Option<bool>) -> Self {
        let mut world = World::new_minimal();
        world.set_now(100);
        let source = world.create(ObjKind::Room, "source", GOD, None).unwrap();
        let destination = world
            .create(ObjKind::Room, "destination", GOD, None)
            .unwrap();
        let actor = world
            .create(ObjKind::Player, "Walker", GOD, Some(source))
            .unwrap();
        let exit = world
            .create(ObjKind::Exit, "north;n", GOD, Some(source))
            .unwrap();
        world.link_exit(exit, Some(destination)).unwrap();
        let projection = window.map(|return_to_source| {
            if return_to_source {
                source
            } else {
                world.create(ObjKind::Room, "projected", GOD, None).unwrap()
            }
        });
        world.take_dirty();
        let mut registry = Registry::new();
        crate::commands::register_core_commands(&mut registry).unwrap();
        let control = Control {
            empty,
            projection,
            ..Control::default()
        };
        let handles = Arc::new(OnceLock::new());
        let engine = Engine::with_modules(
            world,
            registry,
            vec![
                Box::new(SimModule {
                    count,
                    control: control.clone(),
                    handles: Arc::clone(&handles),
                }),
                Box::new(ObserverModule {
                    count,
                    control: control.clone(),
                }),
            ],
        )
        .unwrap();
        Self {
            engine: Box::new(engine),
            handles: handles.get().unwrap().clone(),
            control,
            actor,
            source,
            destination,
        }
    }
    fn invocation(&self) -> ActionInvocation {
        let request = RequestUid::from_bytes([101; 16]);
        let principal = PrincipalId::Object { id: self.actor };
        ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor: self.actor,
            offered_action: ActionOfferId(CanonicalHash::from_bytes([102; 32])),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters: json!({}),
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: OwnerId::new("sim").unwrap(),
                    namespace: crate::CanonicalId::new("composed_fixture").unwrap(),
                },
                1,
                IdempotencyKey::Request(request),
            ),
        })
        .unwrap()
    }
    fn prepare(
        &mut self,
    ) -> Result<
        (
            crate::composed::PreparedComposedTxn,
            PreparedAuthoritativeAction,
        ),
        String,
    > {
        let captured = self
            .engine
            .capture_locomotion_command(
                self.actor,
                "north",
                &mut Outbox::default(),
                &mut crate::Effects::default(),
            )
            .unwrap();
        self.engine
            .prepare_captured_locomotion_action(
                captured,
                &self.handles.action,
                &self.invocation(),
                token(),
            )
            .map_err(|e| e.to_string())
    }
    fn stage(
        &mut self,
        prepared: (
            crate::composed::PreparedComposedTxn,
            PreparedAuthoritativeAction,
        ),
    ) {
        let invocation = self.invocation();
        let (
            outcome,
            resolution,
            mut definitions,
            eligible_witnesses,
            durable_outbox,
            semantic_events,
        ) = prepared.1.into_parts();
        definitions.push(self.handles.action.clone());
        definitions.sort();
        definitions.dedup();
        let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: token(),
            request: invocation.request(),
            idempotency: invocation.idempotency().clone(),
            ingress_hash: CanonicalHash::from_bytes([103; 32]),
            request_hash: invocation.request_hash(),
            plan_hash: CanonicalHash::from_bytes([104; 32]),
            action: self.handles.action.clone(),
            provider: OwnerId::new("sim").unwrap(),
            cause: PreparedCausalRef::new(invocation.request(), None, 0).unwrap(),
            definitions,
            writes: prepared.0.authoritative_replay_writes(),
            outcome,
            resolution: resolution.map(|r| r.bind(token())),
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
        self.engine
            .stage_authoritative_action(prepared.0, guards, bundle)
            .unwrap();
    }
    fn location(&self) -> Option<ObjId> {
        self.engine.world.get(self.actor).unwrap().location
    }
    fn all_zero(&self) {
        macro_rules! cell { ($($n:literal),*) => { $(assert_eq!(self.engine.world.module_state::<Cell<$n>>().unwrap().0, 0);)* }; }
        cell!(0, 1, 2, 3, 4, 5, 6, 7);
    }
}
fn token() -> crate::CommitToken {
    crate::CommitToken::from_bytes([105; 16])
}

fn install(rig: &mut Rig) {
    let prepared = rig.prepare().expect("capture and prepare");
    assert_eq!(rig.location(), Some(rig.source));
    rig.stage(prepared);
    let (_, installation) = rig
        .engine
        .take_staged_action()
        .unwrap()
        .unwrap()
        .into_parts();
    rig.engine
        .install_composed_action(installation)
        .expect("install");
}
fn window_value(rig: &Rig) -> Value {
    let super::record::Records::Window(handle) = &rig.handles.reader.records else {
        panic!("window fixture")
    };
    serde_json::to_value(
        rig.engine
            .action_state()
            .get(handle, &token())
            .unwrap()
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn window_kernel_commits_two_real_core_steps_and_preserves_intermediate_room() {
    let mut rig = Rig::configured(7, false, Some(false));
    let projected = rig.control.projection.unwrap();
    let prepared = rig.prepare().expect("window prepare");
    rig.all_zero();
    assert_eq!(rig.location(), Some(rig.source));
    // Compare every affected object with the actual sequential primitive.
    let mut expected = Rig::configured(7, false, Some(false));
    expected
        .engine
        .world
        .move_to(rig.actor, rig.destination)
        .unwrap();
    expected.engine.world.move_to(rig.actor, projected).unwrap();
    let after: Vec<_> = [rig.actor, rig.source, rig.destination, projected]
        .into_iter()
        .map(|id| {
            (
                id,
                serde_json::to_value(expected.engine.world.get(id).unwrap()).unwrap(),
            )
        })
        .collect();
    rig.stage(prepared);
    let (_, installation) = rig
        .engine
        .take_staged_action()
        .unwrap()
        .unwrap()
        .into_parts();
    rig.engine.install_composed_action(installation).unwrap();
    for (id, object) in after {
        assert_eq!(
            serde_json::to_value(rig.engine.world.get(id).unwrap()).unwrap(),
            object
        );
    }
    assert_eq!(rig.location(), Some(projected));
    assert_eq!(rig.control.callbacks.load(Ordering::SeqCst), 0);
    macro_rules! cell { ($($n:literal),*) => { $(assert_eq!(rig.engine.world.module_state::<Cell<$n>>().unwrap().0, 1);)* }; }
    cell!(0, 1, 2, 3, 4, 5, 6, 7);
    let row = window_value(&rig);
    assert_eq!(row["movement"]["objects"].as_array().unwrap().len(), 4);
    assert_eq!(row["movement"]["destination"], json!(rig.destination));
    assert_eq!(row["projection"]["projected_room"], json!(projected));
    assert!(super::record::decode_window(&row).is_ok());
}

#[test]
fn window_return_keeps_both_steps_and_original_timestamp() {
    let mut rig = Rig::configured(1, false, Some(true));
    let other = rig
        .engine
        .world
        .create(ObjKind::Thing, "other", GOD, Some(rig.source))
        .unwrap();
    rig.engine.world.take_dirty();
    assert_eq!(
        rig.engine.world.get(rig.source).unwrap().contents,
        vec![rig.actor, other]
    );
    install(&mut rig);
    assert_eq!(rig.location(), Some(rig.source));
    assert_eq!(
        rig.engine.world.get(rig.source).unwrap().contents,
        vec![other, rig.actor]
    );
    let value = window_value(&rig);
    assert_eq!(value["movement"]["at_utc"], json!(100));
    assert_eq!(value["movement"]["objects"].as_array().unwrap().len(), 3);
    assert!(super::record::decode_window(&value).is_ok());
}

#[test]
fn window_header_codec_is_separate_and_bound_to_every_owner() {
    let mut rig = Rig::configured(7, false, Some(false));
    install(&mut rig);
    let base = window_value(&rig);
    assert!(super::record::decode(&base).is_err());
    assert!(super::record::decode(&base["movement"]).is_err());
    for field in ["format", "projection", "movement"] {
        let mut changed = base.clone();
        changed.as_object_mut().unwrap().remove(field);
        assert!(super::record::decode_window(&changed).is_err());
    }
    for field in [
        "target",
        "sim_window_capture_digest",
        "atlas_capture_digest",
        "projected_room",
    ] {
        let mut changed = base.clone();
        changed["projection"].as_object_mut().unwrap().remove(field);
        assert!(super::record::decode_window(&changed).is_err());
    }
    for (field, value) in [
        ("projected_room", json!(rig.source)),
        (
            "atlas_capture_digest",
            json!(crate::DefinitionDigest::from_bytes([9; 32])),
        ),
        (
            "sim_window_capture_digest",
            json!(crate::DefinitionDigest::from_bytes([9; 32])),
        ),
        ("target", Value::Null),
        ("format", json!(2)),
        ("unknown", json!(true)),
    ] {
        let mut changed = base.clone();
        changed["projection"][field] = value;
        assert!(super::record::decode_window(&changed).is_err(), "{field}");
    }
    let mut changed = base.clone();
    changed["projection"]["target"]["x_cm"] = json!(9);
    assert!(super::record::decode_window(&changed).is_err());
    let mut ordinary = Rig::new(1, false);
    install(&mut ordinary);
    let row = ordinary
        .engine
        .action_state()
        .get(ordinary_records(&ordinary.handles.reader), &token())
        .unwrap()
        .unwrap();
    let value = serde_json::to_value(row).unwrap();
    assert!(super::record::decode(&value).is_ok());
    assert!(super::record::decode_window(&value).is_err());
    assert!(exact_action(&selected_action(&rig.control)).is_err());
    assert!(exact_window_action(&action_ref()).is_err());
}

#[test]
fn window_kernel_refusals_leave_all_objects_and_owners_unchanged() {
    for mode in 0..4 {
        let mut rig = Rig::configured(7, false, Some(false));
        if mode == 0 {
            rig.control.refuse.store(6, Ordering::SeqCst);
            rig.control.ignore.store(true, Ordering::SeqCst);
            assert!(rig.prepare().is_err());
        } else {
            let prepared = rig.prepare().unwrap();
            rig.stage(prepared);
            let (_, installation) = rig
                .engine
                .take_staged_action()
                .unwrap()
                .unwrap()
                .into_parts();
            match mode {
                1 => rig.control.bind_fail.store(7, Ordering::SeqCst),
                2 => {
                    Registrar::new(&mut rig.engine.registry, "observer").subscribe(
                        crate::WorldEventKind::Moved,
                        Arc::new(|_: &mut crate::Ctx<'_>, _: &crate::WorldEvent| {}),
                    );
                }
                _ => {
                    rig.engine
                        .world
                        .get_mut(rig.control.projection.unwrap())
                        .unwrap()
                        .name = "changed".into();
                }
            }
            assert!(rig.engine.install_composed_action(installation).is_err());
        }
        rig.all_zero();
        assert_eq!(rig.location(), Some(rig.source));
        assert!(
            rig.engine
                .world
                .get(rig.destination)
                .unwrap()
                .contents
                .is_empty()
        );
    }
}

#[test]
fn window_self_loop_has_no_fabricated_core_rows_or_placement() {
    let mut rig = Rig::configured(1, true, Some(true));
    let exit = rig.engine.world.get(rig.source).unwrap().exits[0];
    rig.engine.world.link_exit(exit, Some(rig.source)).unwrap();
    rig.engine.world.take_dirty();
    install(&mut rig);
    let value = window_value(&rig);
    assert_eq!(value["movement"]["objects"], json!([]));
    assert_eq!(value["projection"]["target"], Value::Null);
    assert!(super::record::decode_window(&value).is_ok());
    assert_eq!(rig.engine.world.module_state::<Cell<0>>().unwrap().0, 0);
}

#[test]
fn actual_capture_prepares_eight_slots_then_publishes_once_without_legacy_callbacks() {
    let mut rig = Rig::new(7, false);
    let prepared = rig.prepare().expect("closed prepare");
    assert_eq!(rig.location(), Some(rig.source));
    rig.all_zero();
    assert_eq!(
        prepared
            .0
            .authoritative_replay_writes()
            .iter()
            .filter(|w| matches!(w,
        ReplayWrite::ModuleJsonSet { namespace, .. } if namespace.as_str()=="observer"))
            .count(),
        7
    );
    rig.stage(prepared);
    let (_, installation) = rig
        .engine
        .take_staged_action()
        .unwrap()
        .unwrap()
        .into_parts();
    rig.all_zero();
    rig.engine
        .install_composed_action(installation)
        .expect("install");
    assert_eq!(rig.location(), Some(rig.destination));
    macro_rules! cell { ($($n:literal),*) => { $(assert_eq!(rig.engine.world.module_state::<Cell<$n>>().unwrap().0, 1);)* }; }
    cell!(0, 1, 2, 3, 4, 5, 6, 7);
    assert_eq!(rig.control.callbacks.load(Ordering::SeqCst), 0);
    let record = rig
        .engine
        .action_state()
        .get(ordinary_records(&rig.handles.reader), &token())
        .unwrap()
        .unwrap();
    let manifest = record.validate().unwrap();
    assert_eq!(manifest.groups().len(), 7);
    assert_eq!(record.effects.len(), 7);
}

#[test]
fn empty_owner_observation_has_a_proof_and_no_fabricated_write() {
    let mut rig = Rig::new(1, true);
    let prepared = rig.prepare().expect("empty owner");
    let writes = prepared.0.authoritative_replay_writes();
    assert!(!writes.iter().any(|w| matches!(w, ReplayWrite::ModuleJsonSet { namespace, .. } if namespace.as_str()=="observer")));
    rig.stage(prepared);
    let (_, installation) = rig
        .engine
        .take_staged_action()
        .unwrap()
        .unwrap()
        .into_parts();
    rig.engine.install_composed_action(installation).unwrap();
    assert_eq!(rig.engine.world.module_state::<Cell<0>>().unwrap().0, 0);
    assert_eq!(rig.engine.world.module_state::<Cell<7>>().unwrap().0, 1);
    let row = rig
        .engine
        .action_state()
        .get(ordinary_records(&rig.handles.reader), &token())
        .unwrap()
        .unwrap();
    assert_eq!(row.effects.len(), 1);
    assert!(row.validate().is_ok());
}

#[test]
fn last_bind_refusal_keeps_all_earlier_owner_bindings_and_objects_unchanged() {
    let mut rig = Rig::new(7, false);
    let prepared = rig.prepare().unwrap();
    rig.stage(prepared);
    let (_, installation) = rig
        .engine
        .take_staged_action()
        .unwrap()
        .unwrap()
        .into_parts();
    rig.control.bind_fail.store(7, Ordering::SeqCst);
    assert!(rig.engine.install_composed_action(installation).is_err());
    rig.all_zero();
    assert_eq!(rig.location(), Some(rig.source));
}

#[test]
fn caught_owner_refusal_poisoning_and_late_manifest_drift_both_prevent_publication() {
    let mut rig = Rig::new(7, false);
    rig.control.refuse.store(6, Ordering::SeqCst);
    rig.control.ignore.store(true, Ordering::SeqCst);
    assert!(rig.prepare().is_err());
    rig.all_zero();
    assert_eq!(rig.location(), Some(rig.source));
    let mut rig = Rig::new(7, false);
    let prepared = rig.prepare().unwrap();
    rig.stage(prepared);
    let (_, installation) = rig
        .engine
        .take_staged_action()
        .unwrap()
        .unwrap()
        .into_parts();
    Registrar::new(&mut rig.engine.registry, "observer").subscribe(
        crate::WorldEventKind::Moved,
        Arc::new(|_: &mut crate::Ctx<'_>, _: &crate::WorldEvent| {}),
    );
    assert!(rig.engine.install_composed_action(installation).is_err());
    rig.all_zero();
    assert_eq!(rig.location(), Some(rig.source));
}

#[test]
fn receipt_codec_binds_owner_data_to_original_route_clock_group_and_proof() {
    let mut rig = Rig::new(1, false);
    let prepared = rig.prepare().unwrap();
    rig.stage(prepared);
    let (_, installation) = rig
        .engine
        .take_staged_action()
        .unwrap()
        .unwrap()
        .into_parts();
    rig.engine.install_composed_action(installation).unwrap();
    let row = rig
        .engine
        .action_state()
        .get(ordinary_records(&rig.handles.reader), &token())
        .unwrap()
        .unwrap()
        .clone();
    assert!(row.validate().is_ok());
    let mut cases = Vec::new();
    let mut changed = row.clone();
    changed.at_utc += 1;
    cases.push(changed);
    let mut changed = row.clone();
    changed.prepared_at.at += 1;
    cases.push(changed);
    let mut changed = row.clone();
    changed.effects[0].group = 1;
    cases.push(changed);
    let mut changed = row.clone();
    changed.effects[0].owner_capture_digest = crate::DefinitionDigest::from_bytes([0; 32]);
    cases.push(changed);
    let mut changed = row.clone();
    changed.effects[0].proof["extra"] = json!(1);
    cases.push(changed);
    let mut changed = row.clone();
    changed.effects.push(changed.effects[0].clone());
    cases.push(changed);
    let mut changed = row.clone();
    changed.manifest["groups"][0]["binding"]["version"] = json!(2);
    cases.push(changed);
    let mut changed = row.clone();
    changed.objects.swap(0, 1);
    cases.push(changed);
    let mut changed = row.clone();
    changed.effects[0].proof = json!("x".repeat(MAX_MOVEMENT_PROOF_BYTES));
    cases.push(changed);
    for changed in cases {
        assert!(changed.validate().is_err());
    }
    let base = serde_json::to_value(row).unwrap();
    for field in ["source", "exit", "manifest", "manifest_digest", "effects"] {
        let mut changed = base.clone();
        changed.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<super::record::Record>(changed).is_err());
    }
    let mut changed = base;
    changed["unknown"] = json!(true);
    assert!(serde_json::from_value::<super::record::Record>(changed).is_err());
}

#[test]
fn many_slot_borrow_rejects_aliases_missing_and_identically_replaced_slots() {
    let mut slots = crate::module::ModuleStateMap::default();
    macro_rules! insert { ($($n:literal),*) => { $(slots.insert(Cell::<$n>(0));)* }; }
    insert!(0, 1, 2, 3, 4, 5, 6, 7);
    let ids = [
        std::any::TypeId::of::<Cell<0>>(),
        std::any::TypeId::of::<Cell<1>>(),
        std::any::TypeId::of::<Cell<2>>(),
        std::any::TypeId::of::<Cell<3>>(),
        std::any::TypeId::of::<Cell<4>>(),
        std::any::TypeId::of::<Cell<5>>(),
        std::any::TypeId::of::<Cell<6>>(),
        std::any::TypeId::of::<Cell<7>>(),
    ];
    let captures: Vec<_> = ids
        .into_iter()
        .map(|id| slots.capture_erased(id).unwrap().0)
        .collect();
    let refs: Vec<_> = captures.iter().collect();
    assert_eq!(slots.get_erased_movement_mut(&refs).unwrap().len(), 8);
    let repeated = vec![&captures[0]; 8];
    assert!(slots.get_erased_movement_mut(&repeated).is_err());
    assert!(
        crate::module::ModuleStateMap::default()
            .get_erased_movement_mut(&refs)
            .is_err()
    );
    slots.insert(Cell::<3>(0));
    assert!(slots.get_erased_movement_mut(&refs).is_err());
}

#[test]
fn owner_audit_reads_are_scoped_and_use_the_engine_stamp_instead_of_public_context_fields() {
    for window in [None, Some(false)] {
        let mut rig = Rig::configured(7, false, window);
        let prepared = rig.prepare().unwrap();
        rig.stage(prepared);
        let (_, installation) = rig
            .engine
            .take_staged_action()
            .unwrap()
            .unwrap()
            .into_parts();
        rig.engine.install_composed_action(installation).unwrap();
        let mut world = World::new_minimal();
        let mut out = Outbox::default();
        let store = rig.engine.action_state();
        let mut ctx = crate::Ctx {
            w: &mut world,
            actor: GOD,
            switches: Default::default(),
            args: Default::default(),
            out: &mut out,
            reg: &rig.engine.registry,
            module: Some("sim".into()),
            typed_state: crate::ModuleActionState::for_module(
                crate::TypedStateLens::Live(store),
                Some("observer"),
            ),
        };
        assert!(rig.handles.reader.read_receipt(&ctx, &token()).is_err());
        let own = rig
            .handles
            .owner_reader
            .read_receipt(&ctx, &token())
            .unwrap()
            .unwrap();
        assert_eq!(own.actor(), rig.actor);
        assert_eq!(
            own.window_projection()
                .map(WindowProjection::projected_room),
            rig.control.projection
        );
        assert_eq!(own.effect().participant(), &participant::<0>());
        let page = rig
            .handles
            .owner_reader
            .scan_receipts(&ctx, None, 128)
            .unwrap();
        assert_eq!(page.entries().len(), 1);
        assert_eq!(
            page.entries()[0].effect().participant(),
            &participant::<0>()
        );
        ctx.module = Some("observer".into());
        ctx.typed_state =
            crate::ModuleActionState::for_module(crate::TypedStateLens::Live(store), Some("sim"));
        assert!(
            rig.handles
                .owner_reader
                .read_receipt(&ctx, &token())
                .is_err()
        );
        assert_eq!(
            rig.handles
                .reader
                .read_receipt(&ctx, &token())
                .unwrap()
                .unwrap()
                .effects()
                .len(),
            7
        );
    }
}

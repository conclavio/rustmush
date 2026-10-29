#![cfg(test)]
//! Actual dispatch capture and final Engine admission; the only staged write is a fixture marker.
use super::CapturedLocomotion;
use crate::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionInvocationDraft, ActionOfferId,
    ActionProvider, AuthoritativeActionOutput, CanonicalHash, CommitToken, IdempotencyKey,
    IdempotencyScope, PerceivedRef, PreparedAuthoritativeAction, PreparedCausalRef, PrincipalId,
    RequestUid, ScopedIdempotencyKey,
};
use crate::action_state::{ActionTxn, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId};
use crate::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft, ReplayWrite};
use crate::cmd::{AliasOutcome, AliasStage, CommandGate, CommandSpec, Ctx};
use crate::composed::PreparedComposedTxn;
use crate::mechanics::{
    CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionValidationContext, OwnerId, RulesValue,
};
use crate::{
    Engine, EventHook, GOD, GameModule, HookPoint, HookReadContext, ModError, ObjId, ObjKind,
    Outbox, Registrar, Registry, World,
};
use serde_json::json;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

type Prepared = (PreparedComposedTxn, PreparedAuthoritativeAction);
type Route = (ObjId, Option<ObjId>, Option<ObjId>, ObjId);
const OWNER: &str = "dispatch_fixture";

#[derive(Clone, Copy)]
enum Mode {
    Admit,
    IgnoreRefusal,
    Skip,
    Twice,
}
struct Control {
    mode: Mode,
    route: Option<Route>,
}
#[derive(Clone)]
struct Handles {
    action: DefinitionRef,
    marker: StateFamilyHandle<u32, u32>,
}
struct Fixture {
    control: Arc<Mutex<Control>>,
    handles: Arc<Mutex<Option<Handles>>>,
    hook_veto: Arc<AtomicBool>,
    hook_calls: Arc<AtomicUsize>,
}
fn descriptor(kind: &str, id: &str) -> DefinitionDescriptor {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).unwrap(),
        DefinitionId::new(id).unwrap(),
        1,
        json!({"fixture":"dispatch_capture"}),
    )
    .unwrap()
}
impl GameModule for Fixture {
    fn name(&self) -> &'static str {
        OWNER
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let action = descriptor("action", "capture");
        let outcome = descriptor("outcome", "captured");
        for definition in [&action, &outcome] {
            let expected = definition.clone();
            reg.definition_validator(
                expected.kind().clone(),
                move |ctx: &DefinitionValidationContext<'_>| {
                    let d = ctx.definition();
                    if d.reference().key().id == *expected.id()
                        && d.reference().version() == expected.version()
                        && d.payload() == expected.payload()
                    {
                        Ok(())
                    } else {
                        Err("unexpected dispatch fixture definition".into())
                    }
                },
            )?;
        }
        let action = reg.definition(action)?;
        let outcome = reg.definition(outcome)?;
        let marker = reg.state_family(
            StateFamilyDescriptor::new(
                StateFamilyId::new("markers").unwrap(),
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
            |_: &u32| Ok(()),
            |value: &u32| serde_json::to_value(value).map_err(|e| e.to_string()),
        )?;
        reg.action_provider(
            action.clone(),
            Provider {
                control: Arc::clone(&self.control),
                marker: marker.clone(),
                outcome,
            },
        )?;
        reg.hook(
            HookPoint::MoveAttempt,
            Arc::new(MoveGate {
                veto: Arc::clone(&self.hook_veto),
                calls: Arc::clone(&self.hook_calls),
            }),
        );
        *self.handles.lock().unwrap() = Some(Handles { action, marker });
        Ok(())
    }
}
struct Provider {
    control: Arc<Mutex<Control>>,
    marker: StateFamilyHandle<u32, u32>,
    outcome: DefinitionRef,
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
            Mode::Admit | Mode::Twice => {
                let admission = ctx.admit_captured_locomotion(txn).map_err(|_| {
                    ActionError::new(
                        ActionErrorCode::ProviderRejected,
                        false,
                        "captured movement refused",
                    )
                    .unwrap()
                })?;
                control.route = Some((
                    admission.actor(),
                    admission.source(),
                    admission.exit(),
                    admission.destination(),
                ));
                let destination = admission.destination();
                let actor = admission.actor();
                let preview = ctx
                    .preview_captured_locomotion_projection(&BTreeSet::new(), destination)
                    .unwrap();
                assert_eq!(preview.actor(), actor);
                assert_eq!(preview.admitted_room(), destination);
                assert_eq!(preview.destination(), destination);
                if matches!(control.mode, Mode::Twice) {
                    txn.insert(&self.marker, &0, 1).unwrap();
                    assert!(ctx.admit_captured_locomotion(txn).is_err());
                    assert!(
                        ctx.preview_captured_locomotion_projection(&BTreeSet::new(), destination)
                            .is_err()
                    );
                }
            }
            Mode::IgnoreRefusal => {
                txn.insert(&self.marker, &0, 1).unwrap();
                assert!(ctx.admit_captured_locomotion(txn).is_err());
            }
            Mode::Skip => {
                assert!(
                    ctx.preview_captured_locomotion_projection(&BTreeSet::new(), crate::LIMBO)
                        .is_err()
                );
            }
        }
        if matches!(control.mode, Mode::Admit | Mode::Skip) {
            txn.insert(&self.marker, &0, 1).unwrap();
        }
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(self.outcome.clone(), json!({})).unwrap(),
        )
        .with_definitions(vec![self.outcome.clone()]))
    }
}
struct MoveGate {
    veto: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
}
impl EventHook for MoveGate {
    fn move_attempt(
        &self,
        ctx: &HookReadContext<'_>,
        _: ObjId,
        _: Option<ObjId>,
        _: ObjId,
    ) -> Result<(), mush_proto::RichText> {
        assert!(ctx.physical_transition_attempt().is_none());
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.veto.load(Ordering::SeqCst) {
            Err(mush_proto::RichText::plain("live fixture veto"))
        } else {
            Ok(())
        }
    }
}
#[derive(Default)]
struct GateState {
    denied: BTreeSet<String>,
    seen: Vec<(String, String)>,
}
struct Gate(Arc<Mutex<GateState>>);
impl CommandGate for Gate {
    fn check(
        &self,
        _: &World,
        _: &Registry,
        _: ObjId,
        verb: &str,
        line: &str,
    ) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        state.seen.push((verb.into(), line.into()));
        if state.denied.contains(line) {
            Err("dispatch fixture gate denied".into())
        } else {
            Ok(())
        }
    }
}
struct Rewrite {
    from: String,
    to: String,
    calls: Arc<AtomicUsize>,
}
impl AliasStage for Rewrite {
    fn try_alias(&self, _: &mut World, _: ObjId, line: &str, _: &mut Outbox) -> AliasOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if line == self.from {
            AliasOutcome::Rewrite(self.to.clone())
        } else {
            AliasOutcome::NotMine
        }
    }
}
struct Rig {
    engine: Engine,
    handles: Handles,
    control: Arc<Mutex<Control>>,
    gate: Arc<Mutex<GateState>>,
    hook_veto: Arc<AtomicBool>,
    hook_calls: Arc<AtomicUsize>,
    actor: ObjId,
    source: ObjId,
    destination: ObjId,
    exit: ObjId,
}
fn token() -> CommitToken {
    CommitToken::from_bytes([31; 16])
}
fn rig(core_commands: bool) -> Rig {
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
    let mut registry = Registry::new();
    if core_commands {
        crate::commands::register_core_commands(&mut registry).unwrap();
    }
    let gate = Arc::new(Mutex::new(GateState::default()));
    registry.add_command_gate(Arc::new(Gate(Arc::clone(&gate))));
    let control = Arc::new(Mutex::new(Control {
        mode: Mode::Admit,
        route: None,
    }));
    let handles = Arc::new(Mutex::new(None));
    let hook_veto = Arc::new(AtomicBool::new(false));
    let hook_calls = Arc::new(AtomicUsize::new(0));
    let engine = Engine::with_modules(
        world,
        registry,
        vec![Box::new(Fixture {
            control: Arc::clone(&control),
            handles: Arc::clone(&handles),
            hook_veto: Arc::clone(&hook_veto),
            hook_calls: Arc::clone(&hook_calls),
        })],
    )
    .unwrap();
    let handles = handles.lock().unwrap().clone().unwrap();
    Rig {
        engine,
        handles,
        control,
        gate,
        hook_veto,
        hook_calls,
        actor,
        source,
        destination,
        exit,
    }
}
impl Rig {
    fn invocation(&self) -> ActionInvocation {
        let request = RequestUid::from_bytes([31; 16]);
        let principal = PrincipalId::Object { id: self.actor };
        ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor: self.actor,
            offered_action: ActionOfferId(CanonicalHash::from_bytes([4; 32])),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters: json!({}),
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: OwnerId::new(OWNER).unwrap(),
                    namespace: CanonicalId::new("dispatch").unwrap(),
                },
                1,
                IdempotencyKey::Request(request),
            ),
        })
        .unwrap()
    }
    fn capture(&mut self, line: &str) -> CapturedLocomotion {
        let mut out = Outbox::default();
        let capture = self
            .engine
            .capture_locomotion_command(self.actor, line, &mut out, &mut crate::Effects::default())
            .unwrap();
        assert_eq!(capture.actor(), self.actor);
        assert_eq!(
            self.engine.world.get(self.actor).unwrap().location,
            Some(self.source)
        );
        assert!(
            out.is_empty(),
            "capturing movement produced presentation: {out:?}"
        );
        capture
    }
    fn prepare(&mut self, capture: CapturedLocomotion) -> Result<Prepared, String> {
        let prepared = self
            .engine
            .prepare_captured_locomotion_action(
                capture,
                &self.handles.action,
                &self.invocation(),
                token(),
            )
            .map_err(|e| e.to_string())?;
        assert_marker_only(&prepared.0.authoritative_replay_writes());
        Ok(prepared)
    }
    fn stage(&mut self, prepared: Prepared) -> Result<(), String> {
        self.stage_changed(prepared, |_| {})
    }
    fn stage_changed(
        &mut self,
        prepared: Prepared,
        change: impl FnOnce(&mut AuthoritativeBundleDraft),
    ) -> Result<(), String> {
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
        let mut draft = AuthoritativeBundleDraft {
            token: token(),
            request: invocation.request(),
            idempotency: invocation.idempotency().clone(),
            ingress_hash: CanonicalHash::from_bytes([1; 32]),
            request_hash: invocation.request_hash(),
            plan_hash: CanonicalHash::from_bytes([2; 32]),
            action: self.handles.action.clone(),
            provider: OwnerId::new(OWNER).unwrap(),
            cause: PreparedCausalRef::new(invocation.request(), None, 0).unwrap(),
            definitions,
            writes: prepared.0.authoritative_replay_writes(),
            outcome,
            resolution: resolution.map(|r| r.bind(token())),
            semantic_events,
            eligible_witnesses,
            durable_outbox,
        };
        change(&mut draft);
        let bundle = AuthoritativeBundle::new(draft).unwrap();
        let guards = self
            .engine
            .registry
            .server_guard_set_builder(crate::RevisionSet::default())
            .unwrap()
            .finish()
            .unwrap();
        self.engine
            .stage_authoritative_action(prepared.0, guards, bundle)
            .map_err(|e| e.to_string())
    }
    fn abort_taken(&mut self) {
        let staged = self.engine.take_staged_action().unwrap().unwrap();
        let (_, installation) = staged.into_parts();
        self.engine.abort_prepared_action(installation).unwrap();
        self.assert_inert();
    }
    fn assert_inert(&self) {
        assert_eq!(
            self.engine.world.get(self.actor).unwrap().location,
            Some(self.source)
        );
        assert!(
            self.engine
                .action_state()
                .get(&self.handles.marker, &0)
                .unwrap()
                .is_none()
        );
        assert!(!self.engine.action_state().has_prepared());
        assert!(!self.engine.has_staged_action());
        assert!(!self.engine.has_legacy_persistence_work());
    }
    fn rewrite(&mut self, from: &str, to: &str) -> Arc<AtomicUsize> {
        let calls = Arc::new(AtomicUsize::new(0));
        self.engine.registry.add_alias_stage(Arc::new(Rewrite {
            from: from.into(),
            to: to.into(),
            calls: Arc::clone(&calls),
        }));
        calls
    }
}
fn assert_marker_only(writes: &[ReplayWrite]) {
    assert!(
        matches!(writes, [ReplayWrite::ModuleJsonSet { namespace, key, value }]
        if namespace.as_str() == OWNER && key.as_str() == "action-state/v1/markers/0"
        && value == &json!({"family":{"owner":OWNER,"id":"markers"},"key":"0",
            "schema":{"id":"u32.value.v1","version":1},"value":1})),
        "unexpected writes: {writes:?}"
    );
}

#[test]
fn real_capture_prepare_stage_and_take_remain_inert() {
    let mut r = rig(true);
    let capture = r.capture("n");
    let prepared = r.prepare(capture).unwrap();
    assert_eq!(
        r.control.lock().unwrap().route,
        Some((r.actor, Some(r.source), Some(r.exit), r.destination))
    );
    r.stage(prepared).unwrap();
    r.abort_taken();
    assert!(
        r.hook_calls.load(Ordering::SeqCst) >= 3,
        "prepare, stage, and final take must consult real hooks"
    );
}

#[test]
fn actual_builtin_aliases_prefixes_and_home_are_captured() {
    for (line, home) in [
        ("goto north", false),
        ("go n", false),
        ("move north", false),
        ("got n", false),
        ("home", true),
        ("HOME", true),
        ("goto home", true),
        ("home ignored arguments", true),
    ] {
        let mut r = rig(true);
        let capture = r.capture(line);
        let prepared = r.prepare(capture).unwrap();
        assert_eq!(
            r.control.lock().unwrap().route.unwrap().2,
            if home { None } else { Some(r.exit) },
            "{line}"
        );
        r.stage(prepared).unwrap();
        r.abort_taken();
    }
}

#[test]
fn aliases_execute_once_and_each_rewritten_line_is_gated() {
    let mut r = rig(true);
    let first = r.rewrite("travel", "walk");
    let second = r.rewrite("walk", "goto north");
    let capture = r.capture("travel");
    assert_eq!(
        r.gate.lock().unwrap().seen,
        vec![
            ("travel".into(), "travel".into()),
            ("walk".into(), "walk".into()),
            ("goto".into(), "goto north".into())
        ]
    );
    let prepared = r.prepare(capture).unwrap();
    r.stage(prepared).unwrap();
    r.abort_taken();
    assert_eq!(first.load(Ordering::SeqCst), 1);
    assert_eq!(second.load(Ordering::SeqCst), 1);
}

#[test]
fn initial_and_every_rewritten_gate_line_are_rechecked_at_final_take() {
    for blocked in ["travel", "walk", "goto north"] {
        let mut r = rig(true);
        r.rewrite("travel", "walk");
        r.rewrite("walk", "goto north");
        let capture = r.capture("travel");
        let prepared = r.prepare(capture).unwrap();
        r.stage(prepared).unwrap();
        r.gate.lock().unwrap().denied.insert(blocked.into());
        assert!(
            r.engine.take_staged_action().is_err(),
            "new veto on {blocked} was ignored"
        );
        r.assert_inert();
    }
}

#[test]
fn gate_refusal_stops_later_alias_stages_during_actual_capture() {
    let mut r = rig(true);
    let first = r.rewrite("travel", "walk");
    let second = r.rewrite("walk", "north");
    r.gate.lock().unwrap().denied.insert("walk".into());
    let mut out = Outbox::default();
    assert!(
        r.engine
            .capture_locomotion_command(r.actor, "travel", &mut out, &mut crate::Effects::default())
            .is_none()
    );
    assert_eq!(first.load(Ordering::SeqCst), 1);
    assert_eq!(second.load(Ordering::SeqCst), 0);
    assert!(!out.is_empty());
    r.assert_inert();
}

#[test]
fn new_gate_refusal_between_prepare_and_stage_releases_reservation() {
    let mut r = rig(true);
    let capture = r.capture("north");
    let prepared = r.prepare(capture).unwrap();
    r.gate.lock().unwrap().denied.insert("north".into());
    assert!(r.stage(prepared).is_err());
    r.assert_inert();
}

#[test]
fn real_hook_and_route_are_rechecked_at_final_take() {
    for change_hook in [true, false] {
        let mut r = rig(true);
        let capture = r.capture("north");
        let prepared = r.prepare(capture).unwrap();
        r.stage(prepared).unwrap();
        if change_hook {
            r.hook_veto.store(true, Ordering::SeqCst);
        } else {
            r.engine.world.link_exit(r.exit, Some(r.source)).unwrap();
            r.engine.world.take_dirty();
        }
        assert!(r.engine.take_staged_action().is_err());
        r.assert_inert();
    }
}

#[test]
fn whole_line_exit_precedes_builtin_goto_and_home_keyword_precedes_exit() {
    let mut r = rig(true);
    let special = r
        .engine
        .world
        .create(ObjKind::Exit, "goto north;home", GOD, Some(r.source))
        .unwrap();
    r.engine
        .world
        .link_exit(special, Some(r.destination))
        .unwrap();
    r.engine.world.take_dirty();
    let capture = r.capture("goto north");
    let prepared = r.prepare(capture).unwrap();
    assert_eq!(r.control.lock().unwrap().route.unwrap().2, Some(special));
    r.stage(prepared).unwrap();
    r.abort_taken();
    let capture = r.capture("home");
    let prepared = r.prepare(capture).unwrap();
    assert_eq!(r.control.lock().unwrap().route.unwrap().2, None);
    r.stage(prepared).unwrap();
    r.abort_taken();
}

#[test]
fn newly_shadowing_whole_line_exit_invalidates_command_capture() {
    let mut r = rig(true);
    let capture = r.capture("goto north");
    let prepared = r.prepare(capture).unwrap();
    r.stage(prepared).unwrap();
    let shadow = r
        .engine
        .world
        .create(ObjKind::Exit, "goto north", GOD, Some(r.source))
        .unwrap();
    r.engine
        .world
        .link_exit(shadow, Some(r.destination))
        .unwrap();
    r.engine.world.take_dirty();
    assert!(r.engine.take_staged_action().is_err());
    r.assert_inert();
}

#[test]
fn newly_ambiguous_builtin_prefix_invalidates_capture() {
    let mut r = rig(true);
    let capture = r.capture("got north");
    let prepared = r.prepare(capture).unwrap();
    r.stage(prepared).unwrap();
    r.engine
        .registry
        .register(
            CommandSpec {
                name: "gotcha".into(),
                ..CommandSpec::default()
            },
            Arc::new(|_: &mut Ctx<'_>| Ok(())),
        )
        .unwrap();
    assert!(r.engine.take_staged_action().is_err());
    r.assert_inert();
}

#[test]
fn custom_name_or_alias_does_not_impersonate_builtin_movement() {
    for (name, aliases, line) in [
        ("goto", vec![], "goto north"),
        ("custom", vec!["go".into()], "go north"),
        ("home", vec![], "home ignored"),
    ] {
        let mut r = rig(false);
        let calls = Arc::new(AtomicUsize::new(0));
        let handler_calls = Arc::clone(&calls);
        r.engine
            .registry
            .register(
                CommandSpec {
                    name: name.into(),
                    aliases,
                    ..CommandSpec::default()
                },
                Arc::new(move |_: &mut Ctx<'_>| {
                    handler_calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }),
            )
            .unwrap();
        let mut out = Outbox::default();
        assert!(
            r.engine
                .capture_locomotion_command(r.actor, line, &mut out, &mut crate::Effects::default())
                .is_none()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        r.assert_inert();
    }
}

#[test]
fn invalid_builtin_switch_is_not_turned_into_a_route() {
    let mut r = rig(true);
    let mut out = Outbox::default();
    assert!(
        r.engine
            .capture_locomotion_command(
                r.actor,
                "goto/invalid north",
                &mut out,
                &mut crate::Effects::default()
            )
            .is_none()
    );
    assert!(!out.is_empty());
    r.assert_inert();
}

#[test]
fn absent_capture_cannot_be_replaced_by_provider_intent_even_if_error_ignored() {
    let mut r = rig(true);
    r.control.lock().unwrap().mode = Mode::IgnoreRefusal;
    assert!(
        r.engine
            .prepare_registered_composed_authoritative_action(
                &r.handles.action,
                &r.invocation(),
                token()
            )
            .is_err()
    );
    r.assert_inert();
}

#[test]
fn capture_actor_cannot_be_transplanted_to_another_invocation() {
    let mut r = rig(true);
    let other = r
        .engine
        .world
        .create(ObjKind::Player, "other walker", GOD, Some(r.source))
        .unwrap();
    r.engine.world.take_dirty();
    let mut out = Outbox::default();
    let capture = r
        .engine
        .capture_locomotion_command(other, "north", &mut out, &mut crate::Effects::default())
        .unwrap();
    assert!(r.prepare(capture).is_err());
    r.assert_inert();
}

#[test]
fn already_home_produces_no_moving_admission() {
    let mut r = rig(true);
    r.engine.world.set_home(r.actor, Some(r.source)).unwrap();
    r.engine.world.take_dirty();
    let capture = r.capture("home");
    assert!(r.prepare(capture).is_err());
    r.assert_inert();
}

#[test]
fn provider_must_consume_capture_exactly_once() {
    for mode in [Mode::Skip, Mode::Twice] {
        let mut r = rig(true);
        r.control.lock().unwrap().mode = mode;
        let capture = r.capture("north");
        assert!(r.prepare(capture).is_err());
        r.assert_inert();
    }
}

#[test]
fn bundle_cannot_substitute_capture_commit_or_request_hash() {
    for change_commit in [true, false] {
        let mut r = rig(true);
        let capture = r.capture("north");
        let prepared = r.prepare(capture).unwrap();
        let result = r.stage_changed(prepared, |draft| {
            if change_commit {
                draft.token = CommitToken::from_bytes([32; 16]);
            } else {
                draft.request_hash = CanonicalHash::from_bytes([33; 32]);
            }
        });
        assert!(result.is_err());
        r.assert_inert();
    }
}

#[test]
fn acknowledged_install_does_not_rerun_gates_aliases_or_move_hooks() {
    let mut r = rig(true);
    let alias_calls = r.rewrite("travel", "north");
    let capture = r.capture("travel");
    let prepared = r.prepare(capture).unwrap();
    r.stage(prepared).unwrap();
    let staged = r.engine.take_staged_action().unwrap().unwrap();
    let hook_calls = r.hook_calls.load(Ordering::SeqCst);
    let gate_calls = r.gate.lock().unwrap().seen.len();
    // After writer custody these callbacks must never run, even when they now refuse.
    r.hook_veto.store(true, Ordering::SeqCst);
    r.gate.lock().unwrap().denied.insert("travel".into());
    let (_, installation) = staged.into_parts();
    r.engine.install_composed_action(installation).unwrap();
    assert_eq!(r.hook_calls.load(Ordering::SeqCst), hook_calls);
    assert_eq!(r.gate.lock().unwrap().seen.len(), gate_calls);
    assert_eq!(alias_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        r.engine.action_state().get(&r.handles.marker, &0).unwrap(),
        Some(&1)
    );
    assert_eq!(
        r.engine.world.get(r.actor).unwrap().location,
        Some(r.source)
    );
}

#[test]
fn late_dispatch_only_world_drift_refuses_before_typed_marker_install() {
    let mut r = rig(true);
    let capture = r.capture("goto north");
    let prepared = r.prepare(capture).unwrap();
    r.stage(prepared).unwrap();
    let staged = r.engine.take_staged_action().unwrap().unwrap();
    // Admission's query was only "north"; dispatch must retain its independent whole-line negative lookup.
    let shadow = r
        .engine
        .world
        .create(ObjKind::Exit, "goto north", GOD, Some(r.source))
        .unwrap();
    r.engine
        .world
        .link_exit(shadow, Some(r.destination))
        .unwrap();
    r.engine.world.take_dirty();
    let (_, installation) = staged.into_parts();
    assert!(r.engine.install_composed_action(installation).is_err());
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
}

struct FreezeGate(Arc<AtomicBool>);
impl CommandGate for FreezeGate {
    fn check(&self, _: &World, reg: &Registry, _: ObjId, _: &str, _: &str) -> Result<(), String> {
        if self.0.load(Ordering::SeqCst) {
            reg.freeze_mechanics_owner(
                &OwnerId::new(OWNER).unwrap(),
                crate::mechanics::IncidentId(903),
                crate::mechanics::RecoveryReason::IntegrityViolation {
                    code: CanonicalId::new("dispatch_fixture_freeze").unwrap(),
                },
            )
            .unwrap();
        }
        Ok(())
    }
}

#[test]
fn gate_that_freezes_owner_while_returning_allow_cannot_pass_final_take() {
    let mut r = rig(true);
    let armed = Arc::new(AtomicBool::new(false));
    r.engine
        .registry
        .add_command_gate(Arc::new(FreezeGate(Arc::clone(&armed))));
    let capture = r.capture("north");
    let prepared = r.prepare(capture).unwrap();
    r.stage(prepared).unwrap();
    armed.store(true, Ordering::SeqCst);
    assert!(r.engine.take_staged_action().is_err());
    assert!(!r.engine.has_staged_action());
    assert!(!r.engine.action_state().has_prepared());
    assert_eq!(
        r.engine.world.get(r.actor).unwrap().location,
        Some(r.source)
    );
}

struct Context {
    outcome: crate::module::ContextOutcome,
    feeds: Arc<AtomicUsize>,
}
impl crate::module::InputContext for Context {
    fn prompt(&self, _: &World) -> mush_proto::PromptSpec {
        mush_proto::PromptSpec {
            mode: mush_proto::InputMode::Normal,
            text: None,
        }
    }
    fn feed(&mut self, _: &mut Ctx<'_>, _: &str) -> crate::module::ContextOutcome {
        self.feeds.fetch_add(1, Ordering::SeqCst);
        self.outcome.clone()
    }
}

#[test]
fn input_context_consumption_pop_and_hard_escape_precede_capture() {
    for (line, outcome, expected_feeds, expected_depth) in [
        ("north", crate::module::ContextOutcome::Consumed, 1, 1),
        ("north", crate::module::ContextOutcome::Pop, 1, 0),
        ("~q", crate::module::ContextOutcome::Consumed, 0, 0),
    ] {
        let mut r = rig(true);
        let feeds = Arc::new(AtomicUsize::new(0));
        let aliases = r.rewrite("north", "home");
        r.engine.world.push_input_context(
            r.actor,
            None,
            Box::new(Context {
                outcome,
                feeds: Arc::clone(&feeds),
            }),
        );
        let mut out = Outbox::default();
        assert!(
            r.engine
                .capture_locomotion_command(r.actor, line, &mut out, &mut crate::Effects::default())
                .is_none()
        );
        assert_eq!(feeds.load(Ordering::SeqCst), expected_feeds);
        assert_eq!(r.engine.world.input_depth(r.actor), expected_depth);
        assert_eq!(aliases.load(Ordering::SeqCst), 0);
        assert!(r.gate.lock().unwrap().seen.is_empty());
        r.assert_inert();
    }
}

#[test]
fn input_context_pop_and_run_propagates_the_real_recursive_capture() {
    let mut r = rig(true);
    let feeds = Arc::new(AtomicUsize::new(0));
    r.engine.world.push_input_context(
        r.actor,
        None,
        Box::new(Context {
            outcome: crate::module::ContextOutcome::PopAndRun("goto north".into()),
            feeds: Arc::clone(&feeds),
        }),
    );
    let mut out = Outbox::default();
    let capture = r
        .engine
        .capture_locomotion_command(r.actor, "finish", &mut out, &mut crate::Effects::default())
        .unwrap();
    assert_eq!(feeds.load(Ordering::SeqCst), 1);
    assert_eq!(r.engine.world.input_depth(r.actor), 0);
    assert!(
        !out.is_empty(),
        "the context's restored prompt remains visible"
    );
    assert_eq!(
        r.gate.lock().unwrap().seen,
        vec![("goto".into(), "goto north".into())]
    );
    let prepared = r.prepare(capture).unwrap();
    r.stage(prepared).unwrap();
    r.abort_taken();
    assert_eq!(feeds.load(Ordering::SeqCst), 1);
}

#[test]
fn prefix_expanded_speech_skips_aliases_and_never_fabricates_movement() {
    let mut r = rig(true);
    let calls = r.rewrite("say north", "north");
    let mut out = Outbox::default();
    assert!(
        r.engine
            .capture_locomotion_command(
                r.actor,
                "\"north",
                &mut out,
                &mut crate::Effects::default()
            )
            .is_none()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        r.gate.lock().unwrap().seen,
        vec![("say".into(), "say north".into())]
    );
    assert!(!out.is_empty());
    r.assert_inert();
}

#[test]
fn real_macro_expansion_captures_once_under_the_typists_identity() {
    let mut r = rig(true);
    let mut out = Outbox::default();
    assert!(
        r.engine
            .capture_locomotion_command(
                r.actor,
                "@macro travel=goto $1",
                &mut out,
                &mut crate::Effects::default()
            )
            .is_none()
    );
    r.engine.world.take_dirty();
    r.gate.lock().unwrap().seen.clear();
    let capture = r.capture("travel north");
    assert_eq!(
        r.gate.lock().unwrap().seen,
        vec![
            ("travel".into(), "travel north".into()),
            ("goto".into(), "goto north".into())
        ]
    );
    let prepared = r.prepare(capture).unwrap();
    r.stage(prepared).unwrap();
    r.abort_taken();
}

#[test]
fn capture_transcript_line_limit_has_an_exact_boundary_and_no_legacy_fallback() {
    for rewrites in [
        super::dispatch::MAX_GATE_LINES - 1,
        super::dispatch::MAX_GATE_LINES,
    ] {
        let mut r = rig(true);
        for index in 0..rewrites {
            let to = if index + 1 == rewrites {
                "north".into()
            } else {
                format!("step{}", index + 1)
            };
            r.rewrite(&format!("step{index}"), &to);
        }
        let mut out = Outbox::default();
        let capture = r.engine.capture_locomotion_command(
            r.actor,
            "step0",
            &mut out,
            &mut crate::Effects::default(),
        );
        if rewrites + 1 == super::dispatch::MAX_GATE_LINES {
            let prepared = r.prepare(capture.unwrap()).unwrap();
            r.stage(prepared).unwrap();
            r.abort_taken();
        } else {
            assert!(capture.is_none());
            assert!(!out.is_empty());
            r.assert_inert();
        }
    }
}

#[test]
fn capture_transcript_byte_limit_counts_every_rewrite_without_narrowing_later() {
    for overflow in [false, true] {
        let mut r = rig(true);
        let first = "a".repeat(crate::MAX_INGRESS_LINE_BYTES);
        let second_bytes =
            super::dispatch::MAX_GATE_BYTES - first.len() - "north".len() + usize::from(overflow);
        let second = "b".repeat(second_bytes);
        r.rewrite(&first, &second);
        r.rewrite(&second, "north");
        let mut out = Outbox::default();
        let capture = r.engine.capture_locomotion_command(
            r.actor,
            &first,
            &mut out,
            &mut crate::Effects::default(),
        );
        if overflow {
            assert!(capture.is_none());
            assert!(!out.is_empty());
            r.assert_inert();
        } else {
            let prepared = r.prepare(capture.unwrap()).unwrap();
            r.stage(prepared).unwrap();
            r.abort_taken();
        }
    }
}

#[test]
fn oversized_input_and_intermediate_rewrite_cannot_shrink_into_a_capture() {
    for initial in [true, false] {
        let mut r = rig(true);
        let oversized = "界".repeat(crate::MAX_INGRESS_LINE_BYTES / 3 + 1);
        r.rewrite("travel", &oversized);
        r.rewrite(&oversized, "north");
        let mut out = Outbox::default();
        let line = if initial {
            oversized.as_str()
        } else {
            "travel"
        };
        assert!(
            r.engine
                .capture_locomotion_command(r.actor, line, &mut out, &mut crate::Effects::default())
                .is_none()
        );
        assert!(!out.is_empty());
        r.assert_inert();
    }
}

fn legacy_write() -> crate::ModuleOp {
    crate::ModuleOp {
        ns: OWNER.into(),
        op: crate::ModuleWrite::JsonSet {
            key: "alias-side-effect".into(),
            value: json!({"legacy":true}),
        },
    }
}
struct PersistingRewrite(Arc<AtomicUsize>);
impl AliasStage for PersistingRewrite {
    fn try_alias(&self, _: &mut World, actor: ObjId, line: &str, out: &mut Outbox) -> AliasOutcome {
        if line != "travel" {
            return AliasOutcome::NotMine;
        }
        self.0.fetch_add(1, Ordering::SeqCst);
        out.defer(crate::Deferred::Persist(legacy_write()));
        out.defer(crate::Deferred::Effect(crate::Effect::SlowTest {
            actor,
            correlation: 44,
            seconds: 1,
        }));
        AliasOutcome::Rewrite("north".into())
    }
}

#[test]
fn alias_deferred_work_is_drained_once_and_refuses_authoritative_capture() {
    let mut r = rig(true);
    let calls = Arc::new(AtomicUsize::new(0));
    r.engine
        .registry
        .add_alias_stage(Arc::new(PersistingRewrite(Arc::clone(&calls))));
    let mut out = Outbox::default();
    let mut fx = crate::Effects::default();
    assert!(
        r.engine
            .capture_locomotion_command(r.actor, "travel", &mut out, &mut fx)
            .is_none()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(r.engine.module_ops(), &[legacy_write()]);
    assert_eq!(
        fx.queued(),
        &[crate::Effect::SlowTest {
            actor: r.actor,
            correlation: 44,
            seconds: 1
        }]
    );
    assert!(out.deferred().is_empty());
    assert!(
        !r.engine.world.has_dirty(),
        "this case exercises deferred persistence without dirty Obj state"
    );
    assert_eq!(
        r.engine.world.get(r.actor).unwrap().location,
        Some(r.source)
    );
    assert!(!r.engine.has_staged_action());
    assert!(!r.engine.action_state().has_prepared());
}

#[test]
fn preexisting_deferred_work_cannot_be_hidden_by_a_clean_alias_free_capture() {
    let mut r = rig(true);
    let mut out = Outbox::default();
    let mut fx = crate::Effects::default();
    out.defer(crate::Deferred::Persist(legacy_write()));
    assert!(
        r.engine
            .capture_locomotion_command(r.actor, "north", &mut out, &mut fx)
            .is_none()
    );
    assert_eq!(r.engine.module_ops(), &[legacy_write()]);
    assert!(out.deferred().is_empty());
    assert!(fx.queued().is_empty());
    assert_eq!(
        r.engine.world.get(r.actor).unwrap().location,
        Some(r.source)
    );
}

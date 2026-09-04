//! Cross-crate fixtures for the rules-engine contracts.
//!
//! This crate deliberately contains no production implementation. It is a
//! top-level workspace member so later proving gates can depend on core,
//! rulesets, tactics, simulation, server, and fault stores without creating a
//! production dependency cycle.

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionInvocationDraft, ActionOfferId,
    ActionProvider, ActionStateError, ActionTxn, AuthoritativeActionOutput, CanonicalHash,
    CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionRegistryBuilder, DefinitionValidationContext, Engine, GameModule, IdempotencyKey,
    IdempotencyScope, ModError, ObjId, OwnerId, PerceivedRef, PreparedTrigger, PrincipalId,
    Registrar, RequestUid, RulesValue, SameCommitTrigger, ScopedIdempotencyKey,
    StateFamilyDescriptor, StateFamilyHandle, StateFamilyId, SubjectKey,
};
use mush_testkit::TestWorld;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// Canonical owner stamped on every toy definition.
pub const TOY_OWNER: &str = "toy";

/// Exact identities used by the first action-kernel proving gate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToyDefinitions {
    pub focus: DefinitionRef,
    pub guarded: DefinitionRef,
    pub brace: DefinitionRef,
}

fn focus_descriptor() -> DefinitionDescriptor {
    DefinitionDescriptor::new(
        DefinitionKind::new("resource").expect("static definition kind"),
        DefinitionId::new("focus").expect("static definition id"),
        1,
        json!({ "minimum": 0, "maximum": 3, "integer": true }),
    )
    .expect("nonzero static version")
    .as_current()
}

fn guarded_descriptor() -> DefinitionDescriptor {
    DefinitionDescriptor::new(
        DefinitionKind::new("effect").expect("static definition kind"),
        DefinitionId::new("guarded").expect("static definition id"),
        1,
        json!({ "clock": "scene_turn", "duration": 2 }),
    )
    .expect("nonzero static version")
    .as_current()
}

fn brace_descriptor(focus: DefinitionRef, guarded: DefinitionRef) -> DefinitionDescriptor {
    DefinitionDescriptor::new(
        DefinitionKind::new("action").expect("static definition kind"),
        DefinitionId::new("brace").expect("static definition id"),
        1,
        json!({
            "cost": { "resource": "focus", "amount": 1 },
            "creates": { "effect": "guarded", "count": 1 }
        }),
    )
    .expect("nonzero static version")
    .with_references(vec![focus, guarded])
    .as_current()
}

fn register_definitions(
    mut register: impl FnMut(DefinitionDescriptor) -> Result<DefinitionRef, String>,
) -> Result<ToyDefinitions, String> {
    let focus = register(focus_descriptor())?;
    let guarded = register(guarded_descriptor())?;
    let brace = register(brace_descriptor(focus.clone(), guarded.clone()))?;
    Ok(ToyDefinitions {
        focus,
        guarded,
        brace,
    })
}

fn validate_toy_definition(context: &DefinitionValidationContext<'_>) -> Result<(), String> {
    let definitions = toy_definitions();
    let entry = context.definition();
    let descriptor = if entry.reference() == &definitions.focus {
        focus_descriptor()
    } else if entry.reference() == &definitions.guarded {
        guarded_descriptor()
    } else if entry.reference() == &definitions.brace {
        brace_descriptor(definitions.focus, definitions.guarded)
    } else {
        return Err("unknown toy definition".into());
    };
    let mut references = descriptor.references().to_vec();
    references.sort();
    if entry.payload() != descriptor.payload()
        || entry.references() != references
        || entry.is_current() != descriptor.is_current()
    {
        return Err("toy definition does not match its typed fixture".into());
    }
    Ok(())
}

/// Compute the expected exact identities without composing an engine.
/// Repeated calls are byte-for-byte deterministic.
pub fn toy_definitions() -> ToyDefinitions {
    let owner = OwnerId::new(TOY_OWNER).expect("static owner");
    let mut builder = DefinitionRegistryBuilder::new();
    register_definitions(|descriptor| {
        builder
            .register(owner.clone(), descriptor)
            .map_err(|error| error.to_string())
    })
    .expect("static toy definitions compose")
}

/// Minimal provider used by every vertical contract gate.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToyFocusState {
    pub amount: u8,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToyGuardedState {
    pub expires_after_scene_turn: u64,
}

/// Opaque capabilities captured from the actual toy module composition.
#[derive(Clone, Debug)]
pub struct ToyStateFamilies {
    pub focus: StateFamilyHandle<ObjId, ToyFocusState>,
    pub guarded: StateFamilyHandle<ObjId, ToyGuardedState>,
}

#[derive(Debug)]
struct BraceProvider {
    focus: StateFamilyHandle<ObjId, ToyFocusState>,
    guarded: StateFamilyHandle<ObjId, ToyGuardedState>,
    brace_definition: DefinitionRef,
    guarded_definition: DefinitionRef,
}

impl BraceProvider {
    fn scene_turn(invocation: &ActionInvocation) -> Result<u64, ActionError> {
        let parameters = invocation.parameters().as_object().ok_or_else(|| {
            ActionError::new(
                ActionErrorCode::InvalidRequest,
                false,
                "brace parameters must be an object",
            )
            .expect("static action error is bounded")
        })?;
        let allowed_len = if parameters.contains_key("cascade_steps") {
            2
        } else {
            1
        };
        if parameters.len() != allowed_len
            || parameters
                .keys()
                .any(|key| key != "scene_turn" && key != "cascade_steps")
        {
            return Err(ActionError::new(
                ActionErrorCode::InvalidRequest,
                false,
                "brace accepts scene_turn and the fixture-only cascade_steps parameter",
            )
            .expect("static action error is bounded"));
        }
        parameters
            .get("scene_turn")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                ActionError::new(
                    ActionErrorCode::InvalidRequest,
                    false,
                    "brace requires an unsigned scene_turn parameter",
                )
                .expect("static action error is bounded")
            })
    }

    fn cascade_steps(invocation: &ActionInvocation) -> Result<u64, ActionError> {
        let steps = invocation
            .parameters()
            .get("cascade_steps")
            .map_or(Some(1), Value::as_u64)
            .filter(|steps| *steps > 0)
            .ok_or_else(|| {
                ActionError::new(
                    ActionErrorCode::InvalidRequest,
                    false,
                    "cascade_steps must be a positive unsigned integer",
                )
                .expect("static action error is bounded")
            })?;
        Ok(steps)
    }

    fn state_error(error: &ActionStateError) -> ActionError {
        let code = match error {
            ActionStateError::OperationLimit { .. }
            | ActionStateError::BundleBytesExceeded { .. }
            | ActionStateError::CanonicalPostImage { .. }
            | ActionStateError::ArithmeticOverflow
            | ActionStateError::ReservationExhausted
            | ActionStateError::RevisionExhausted => ActionErrorCode::BoundsExceeded,
            ActionStateError::AlreadyExists { .. } | ActionStateError::Missing { .. } => {
                ActionErrorCode::Conflict
            }
            ActionStateError::OwnerNotReady(_) => ActionErrorCode::Unavailable,
            _ => ActionErrorCode::Integrity,
        };
        ActionError::new(code, false, "toy action-state operation was refused")
            .expect("static action error is bounded")
    }
}

impl ActionProvider for BraceProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
            return Err(ActionError::new(
                ActionErrorCode::InvalidRequest,
                false,
                "brace targets only the acting character",
            )
            .expect("static action error is bounded"));
        }
        Self::scene_turn(invocation).map(|_| ())
    }

    fn prepare_invocation(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<(), ActionError> {
        self.validate_invocation(invocation)?;
        let actor = invocation.actor();
        let turn = Self::scene_turn(invocation)?;
        let focus = txn
            .get(&self.focus, &actor)
            .map_err(|error| Self::state_error(&error))?
            .cloned()
            .ok_or_else(|| {
                ActionError::new(
                    ActionErrorCode::ProviderRejected,
                    false,
                    "the actor has no focus resource",
                )
                .expect("static action error is bounded")
            })?;
        let amount = focus.amount.checked_sub(1).ok_or_else(|| {
            ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "insufficient focus",
            )
            .expect("static action error is bounded")
        })?;
        let expires_after_scene_turn = turn.checked_add(2).ok_or_else(|| {
            ActionError::new(
                ActionErrorCode::BoundsExceeded,
                false,
                "guarded expiry exceeds the scene-turn domain",
            )
            .expect("static action error is bounded")
        })?;
        txn.replace(&self.focus, &actor, ToyFocusState { amount })
            .map_err(|error| Self::state_error(&error))?;
        txn.insert(
            &self.guarded,
            &actor,
            ToyGuardedState {
                expires_after_scene_turn,
            },
        )
        .map_err(|error| Self::state_error(&error))?;
        Ok(())
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(invocation)?;
        let actor = invocation.actor();
        let turn = Self::scene_turn(invocation)?;
        let cascade_steps = Self::cascade_steps(invocation)?;
        let focus = txn
            .get(&self.focus, &actor)
            .map_err(|error| Self::state_error(&error))?
            .cloned()
            .ok_or_else(|| {
                ActionError::new(
                    ActionErrorCode::ProviderRejected,
                    false,
                    "the actor has no focus resource",
                )
                .expect("static action error is bounded")
            })?;
        let amount = focus.amount.checked_sub(1).ok_or_else(|| {
            ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "insufficient focus",
            )
            .expect("static action error is bounded")
        })?;
        txn.replace(&self.focus, &actor, ToyFocusState { amount })
            .map_err(|error| Self::state_error(&error))?;
        let outcome = RulesValue::new(
            self.brace_definition.clone(),
            json!({ "actor": actor.0, "event": "braced", "focus": amount }),
        )
        .map_err(|_| {
            ActionError::new(
                ActionErrorCode::BoundsExceeded,
                false,
                "brace outcome exceeds canonical bounds",
            )
            .expect("static action error is bounded")
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()])
            .with_same_commit_triggers(vec![SameCommitTrigger::new(
                CanonicalId::new("apply_guarded").expect("static handler id"),
                SubjectKey::Object { id: actor },
                0,
                json!({ "scene_turn": turn, "remaining": cascade_steps }),
            )]))
    }

    fn prepare_same_commit_trigger(
        &self,
        invocation: &ActionInvocation,
        trigger: &PreparedTrigger,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        if trigger.identity().handler.id().as_str() != "apply_guarded"
            || trigger.identity().target
                != (SubjectKey::Object {
                    id: invocation.actor(),
                })
        {
            return Err(ActionError::new(
                ActionErrorCode::Integrity,
                false,
                "unknown or mistargeted toy same-commit trigger",
            )
            .expect("static trigger error is bounded"));
        }
        let turn = trigger
            .payload()
            .as_object()
            .filter(|payload| payload.len() == 2)
            .and_then(|payload| payload.get("scene_turn"))
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                ActionError::new(
                    ActionErrorCode::Integrity,
                    false,
                    "toy guarded trigger payload is invalid",
                )
                .expect("static trigger error is bounded")
            })?;
        let remaining = trigger
            .payload()
            .get("remaining")
            .and_then(Value::as_u64)
            .filter(|remaining| *remaining > 0)
            .ok_or_else(|| {
                ActionError::new(
                    ActionErrorCode::Integrity,
                    false,
                    "toy guarded trigger remaining count is invalid",
                )
                .expect("static trigger error is bounded")
            })?;
        if remaining > 1 {
            let outcome = RulesValue::new(
                self.guarded_definition.clone(),
                json!({
                    "actor": invocation.actor().0,
                    "event": "guarded_trigger_step",
                    "remaining": remaining,
                }),
            )
            .map_err(|_| {
                ActionError::new(
                    ActionErrorCode::BoundsExceeded,
                    false,
                    "guarded trigger step exceeds canonical bounds",
                )
                .expect("static trigger error is bounded")
            })?;
            return Ok(AuthoritativeActionOutput::new(outcome)
                .with_definitions(vec![self.guarded_definition.clone()])
                .with_eligible_witnesses(vec![invocation.principal().clone()])
                .with_same_commit_triggers(vec![SameCommitTrigger::new(
                    CanonicalId::new("apply_guarded").expect("static handler id"),
                    SubjectKey::Object {
                        id: invocation.actor(),
                    },
                    0,
                    json!({ "scene_turn": turn, "remaining": remaining - 1 }),
                )]));
        }
        let expires_after_scene_turn = turn.checked_add(2).ok_or_else(|| {
            ActionError::new(
                ActionErrorCode::BoundsExceeded,
                false,
                "guarded expiry exceeds the scene-turn domain",
            )
            .expect("static action error is bounded")
        })?;
        txn.insert(
            &self.guarded,
            &invocation.actor(),
            ToyGuardedState {
                expires_after_scene_turn,
            },
        )
        .map_err(|error| Self::state_error(&error))?;
        let outcome = RulesValue::new(
            self.guarded_definition.clone(),
            json!({
                "actor": invocation.actor().0,
                "event": "guarded_applied",
                "expires_after_scene_turn": expires_after_scene_turn,
            }),
        )
        .map_err(|_| {
            ActionError::new(
                ActionErrorCode::BoundsExceeded,
                false,
                "guarded event exceeds canonical bounds",
            )
            .expect("static trigger error is bounded")
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_definitions(vec![self.guarded_definition.clone()])
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

#[derive(Debug, Default)]
pub struct ToyRulesModule {
    families: Arc<Mutex<Option<ToyStateFamilies>>>,
}

impl ToyRulesModule {
    fn with_capture(families: Arc<Mutex<Option<ToyStateFamilies>>>) -> Self {
        Self { families }
    }
}

impl GameModule for ToyRulesModule {
    fn name(&self) -> &'static str {
        TOY_OWNER
    }

    fn register(&mut self, registrar: &mut Registrar<'_>) -> Result<(), ModError> {
        for kind in ["resource", "effect", "action"] {
            registrar.definition_validator(
                DefinitionKind::new(kind).expect("static definition kind"),
                validate_toy_definition,
            )?;
        }
        let expected = toy_definitions();
        let actual = register_definitions(|descriptor| {
            registrar
                .definition(descriptor)
                .map_err(|error| error.to_string())
        })
        .map_err(ModError::msg)?;
        if actual != expected {
            return Err(ModError::msg("toy definition identity drift"));
        }
        let focus = registrar.state_family(
            StateFamilyDescriptor::new(
                StateFamilyId::new("focus").expect("static family id"),
                CanonicalId::new("obj_id.decimal.v1").expect("static codec id"),
                CanonicalId::new("toy.focus.v1").expect("static schema id"),
                1,
                CanonicalId::new("toy.focus.decode_json.v1").expect("static decoder id"),
                CanonicalId::new("toy.focus.range.v1").expect("static validator id"),
                CanonicalId::new("toy.focus.json.v1").expect("static serializer id"),
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            |key: &ObjId| Ok(key.0.to_string()),
            |value: &Value| {
                serde_json::from_value(value.clone()).map_err(|error| error.to_string())
            },
            |value: &ToyFocusState| {
                if value.amount <= 3 {
                    Ok(())
                } else {
                    Err("focus must remain within 0..=3".to_owned())
                }
            },
            |value: &ToyFocusState| serde_json::to_value(value).map_err(|error| error.to_string()),
        )?;
        let guarded = registrar.state_family(
            StateFamilyDescriptor::new(
                StateFamilyId::new("guarded").expect("static family id"),
                CanonicalId::new("obj_id.decimal.v1").expect("static codec id"),
                CanonicalId::new("toy.guarded.v1").expect("static schema id"),
                1,
                CanonicalId::new("toy.guarded.decode_json.v1").expect("static decoder id"),
                CanonicalId::new("toy.guarded.range.v1").expect("static validator id"),
                CanonicalId::new("toy.guarded.json.v1").expect("static serializer id"),
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            |key: &ObjId| Ok(key.0.to_string()),
            |value: &Value| {
                serde_json::from_value(value.clone()).map_err(|error| error.to_string())
            },
            |_value: &ToyGuardedState| Ok(()),
            |value: &ToyGuardedState| {
                serde_json::to_value(value).map_err(|error| error.to_string())
            },
        )?;
        let families = ToyStateFamilies { focus, guarded };
        registrar.action_provider(
            actual.brace.clone(),
            BraceProvider {
                focus: families.focus.clone(),
                guarded: families.guarded.clone(),
                brace_definition: actual.brace,
                guarded_definition: actual.guarded,
            },
        )?;
        *self.families.lock().expect("toy family capture poisoned") = Some(families);
        Ok(())
    }
}

/// Fresh in-process world composed with the deterministic toy provider.
#[must_use]
pub fn toy_world() -> TestWorld {
    TestWorld::with_modules(vec![Box::<ToyRulesModule>::default()])
}

/// Fresh toy world plus the composition-branded state capabilities used by
/// the logical Gate-1 slice.
#[must_use]
pub fn toy_world_with_state() -> (TestWorld, ToyStateFamilies) {
    let captured = Arc::new(Mutex::new(None));
    let module = ToyRulesModule::with_capture(Arc::clone(&captured));
    let world = TestWorld::with_modules(vec![Box::new(module)]);
    let families = captured
        .lock()
        .expect("toy family capture poisoned")
        .clone()
        .expect("toy module registered state families");
    (world, families)
}

/// Install a valid focus pre-state through the same typed state-family path
/// used by boot hydration. This is fixture setup, not a gameplay mutation API.
pub fn install_toy_focus(
    engine: &mut Engine,
    families: &ToyStateFamilies,
    actor: ObjId,
    amount: u8,
) {
    let prepared = {
        let mut txn = engine
            .action_transaction(&families.focus)
            .expect("toy focus transaction");
        txn.insert(&families.focus, &actor, ToyFocusState { amount })
            .expect("valid focus bootstrap");
        txn.prepare().expect("focus bootstrap prepares")
    };
    let (_, installation) = prepared.into_parts();
    engine.install_prepared_action(installation);
}

/// Canonical semantic invocation for the toy `brace` provider.
pub fn toy_brace_invocation(
    actor: ObjId,
    scene_turn: u64,
    request: RequestUid,
) -> ActionInvocation {
    toy_brace_invocation_with_cascade_steps(actor, scene_turn, request, None)
}

/// Fixture-only variant used to prove cascade bounds abort the whole action.
pub fn toy_brace_invocation_with_cascade_steps(
    actor: ObjId,
    scene_turn: u64,
    request: RequestUid,
    cascade_steps: Option<u64>,
) -> ActionInvocation {
    let definitions = toy_definitions();
    let owner = OwnerId::new(TOY_OWNER).expect("static owner");
    let principal = PrincipalId::Object { id: actor };
    let idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner,
            namespace: CanonicalId::new("brace").expect("static namespace"),
        },
        0,
        IdempotencyKey::Request(request),
    );
    let offered_action = ActionOfferId(
        CanonicalHash::for_canonical_value(
            "toy.offer.v1",
            1,
            &json!({ "action": definitions.brace }),
            4 * 1024,
        )
        .expect("static offer is canonical"),
    );
    let parameters = cascade_steps.map_or_else(
        || json!({ "scene_turn": scene_turn }),
        |steps| json!({ "scene_turn": scene_turn, "cascade_steps": steps }),
    );
    ActionInvocation::new(ActionInvocationDraft {
        request,
        principal,
        actor,
        offered_action,
        subject: PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters,
        idempotency,
    })
    .expect("toy invocation is canonical")
}

/// Build a bounded exact rules value for gate fixtures.
pub fn toy_rules_value(definition: DefinitionRef, data: Value) -> RulesValue {
    RulesValue::new(definition, data).expect("toy fixture must fit RulesValue bounds")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{ActionProviderPrepareError, RevisionSet};

    fn bootstrap_focus(
        world: &mut TestWorld,
        families: &ToyStateFamilies,
        actor: ObjId,
        amount: u8,
    ) {
        install_toy_focus(world.engine_mut(), families, actor, amount);
    }

    fn bootstrap_guarded(
        world: &mut TestWorld,
        families: &ToyStateFamilies,
        actor: ObjId,
        expiry: u64,
    ) {
        let prepared = {
            let mut txn = world
                .engine_mut()
                .action_transaction(&families.guarded)
                .expect("toy guarded transaction");
            txn.insert(
                &families.guarded,
                &actor,
                ToyGuardedState {
                    expires_after_scene_turn: expiry,
                },
            )
            .expect("valid guarded bootstrap");
            txn.prepare().expect("guarded bootstrap prepares")
        };
        let (_, installation) = prepared.into_parts();
        world.engine_mut().install_prepared_action(installation);
    }

    #[test]
    fn definitions_are_stable_and_installed_exactly() {
        let first = toy_definitions();
        let second = toy_definitions();
        assert_eq!(first, second);

        let world = toy_world();
        let registry = world
            .registry()
            .definitions()
            .expect("composition freezes definitions");
        for reference in [&first.focus, &first.guarded, &first.brace] {
            assert_eq!(
                registry
                    .get_exact(reference)
                    .expect("toy exact definition is installed")
                    .reference(),
                reference
            );
        }
    }

    #[test]
    fn brace_dependency_closure_is_exact() {
        let definitions = toy_definitions();
        let world = toy_world();
        let entry = world
            .registry()
            .definitions()
            .unwrap()
            .get_exact(&definitions.brace)
            .unwrap();
        assert_eq!(
            entry.references(),
            &[definitions.guarded, definitions.focus]
        );
    }

    #[test]
    fn brace_prepares_two_records_and_installs_only_after_ack_boundary() {
        let (mut world, families) = toy_world_with_state();
        bootstrap_focus(&mut world, &families, mush_core::GOD, 3);
        let _legacy_bootstrap = world.drain_write_batch();
        let invocation = toy_brace_invocation(mush_core::GOD, 11, RequestUid::from_bytes([1; 16]));
        let brace = toy_definitions().brace;

        let prepared = world
            .engine_mut()
            .prepare_registered_action(&brace, &invocation)
            .expect("brace prepares");
        assert_eq!(prepared.operation_count(), 2);
        assert_eq!(prepared.records().len(), 2);
        assert_eq!(
            prepared
                .records()
                .iter()
                .map(|record| record.key().family().as_str())
                .collect::<Vec<_>>(),
            vec!["focus", "guarded"]
        );
        assert_eq!(
            world
                .engine()
                .action_state()
                .get(&families.focus, &mush_core::GOD)
                .unwrap(),
            Some(&ToyFocusState { amount: 3 })
        );
        assert!(
            world
                .engine()
                .action_state()
                .get(&families.guarded, &mush_core::GOD)
                .unwrap()
                .is_none(),
            "prepared effect must not become visible before acknowledgement"
        );

        let guards = world
            .engine()
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .unwrap()
            .finish()
            .unwrap();
        world
            .engine_mut()
            .stage_prepared_action(prepared, guards)
            .expect("clean action step stages");
        assert!(
            world.take_module_ops().is_empty(),
            "prepared lane is separate"
        );
        let staged = world
            .engine_mut()
            .take_staged_action()
            .expect("final guard validation succeeds")
            .expect("driver receives prepared action");
        let (persistence, installation) = staged.into_parts();
        let mush_core::StagedActionPersistence::Legacy(ops) = persistence else {
            panic!("K2 compatibility staging must use the legacy persistence lane");
        };
        assert_eq!(ops.len(), 2);
        assert!(
            ops.iter()
                .all(|op| matches!(op.op, mush_core::ModuleWrite::JsonSet { .. }))
        );

        world.engine_mut().install_prepared_action(installation);
        assert_eq!(
            world
                .engine()
                .action_state()
                .get(&families.focus, &mush_core::GOD)
                .unwrap(),
            Some(&ToyFocusState { amount: 2 })
        );
        assert_eq!(
            world
                .engine()
                .action_state()
                .get(&families.guarded, &mush_core::GOD)
                .unwrap(),
            Some(&ToyGuardedState {
                expires_after_scene_turn: 13,
            })
        );
    }

    #[test]
    fn authoritative_brace_expands_guarded_in_one_causal_scratch_transaction() {
        let (mut world, families) = toy_world_with_state();
        bootstrap_focus(&mut world, &families, mush_core::GOD, 3);
        let invocation =
            toy_brace_invocation(mush_core::GOD, 11, RequestUid::from_bytes([0xA1; 16]));
        let (prepared, cascade) = world
            .engine_mut()
            .prepare_registered_authoritative_action(&toy_definitions().brace, &invocation)
            .expect("brace and its guarded trigger prepare together");
        assert_eq!(prepared.operation_count(), 2);
        assert_eq!(
            prepared
                .records()
                .iter()
                .map(|record| record.key().family().as_str())
                .collect::<Vec<_>>(),
            vec!["focus", "guarded"]
        );
        let (_outcome, definitions, witnesses, outbox, events) = cascade.into_parts();
        assert_eq!(
            definitions,
            vec![toy_definitions().brace, toy_definitions().guarded]
        );
        assert_eq!(witnesses, vec![PrincipalId::Object { id: mush_core::GOD }]);
        assert!(outbox.is_empty());
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].cause().ordinal(), 0);
        assert_eq!(events[0].kind().as_str(), "brace");
        assert_eq!(events[0].truth().data()["event"], "braced");
        assert_eq!(events[1].cause().ordinal(), 1);
        assert_eq!(
            events[1].cause().parent(),
            Some(events[0].cause().event_id())
        );
        assert_eq!(events[1].kind().as_str(), "apply_guarded");
        assert_eq!(events[1].truth().data()["event"], "guarded_applied");

        assert_eq!(
            world
                .engine()
                .action_state()
                .get(&families.focus, &mush_core::GOD)
                .unwrap(),
            Some(&ToyFocusState { amount: 3 }),
            "neither root nor trigger scratch may install before acknowledgement"
        );
        assert!(
            world
                .engine()
                .action_state()
                .get(&families.guarded, &mush_core::GOD)
                .unwrap()
                .is_none()
        );
        let (_, installation) = prepared.into_parts();
        world.engine_mut().abort_prepared_action(installation);
        assert!(!world.engine().action_state().has_prepared());
    }

    #[test]
    fn brace_abort_discards_both_post_images() {
        let (mut world, families) = toy_world_with_state();
        bootstrap_focus(&mut world, &families, mush_core::GOD, 1);
        let before = world
            .engine()
            .action_state()
            .canonical(&families.focus, &mush_core::GOD)
            .unwrap()
            .unwrap()
            .as_slice()
            .to_vec();
        let invocation = toy_brace_invocation(mush_core::GOD, 4, RequestUid::from_bytes([2; 16]));
        let prepared = world
            .engine_mut()
            .prepare_registered_action(&toy_definitions().brace, &invocation)
            .expect("brace prepares");
        let (_, installation) = prepared.into_parts();
        world.engine_mut().abort_prepared_action(installation);

        assert_eq!(
            world
                .engine()
                .action_state()
                .canonical(&families.focus, &mush_core::GOD)
                .unwrap()
                .unwrap()
                .as_slice(),
            before
        );
        assert!(
            world
                .engine()
                .action_state()
                .get(&families.guarded, &mush_core::GOD)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn insufficient_focus_refuses_without_preparing_or_mutating() {
        let (mut world, families) = toy_world_with_state();
        bootstrap_focus(&mut world, &families, mush_core::GOD, 0);
        let before = world
            .engine()
            .action_state()
            .canonical(&families.focus, &mush_core::GOD)
            .unwrap()
            .unwrap()
            .as_slice()
            .to_vec();
        let invocation = toy_brace_invocation(mush_core::GOD, 4, RequestUid::from_bytes([3; 16]));
        let error = world
            .engine_mut()
            .prepare_registered_action(&toy_definitions().brace, &invocation)
            .unwrap_err();
        assert!(matches!(
            error,
            ActionProviderPrepareError::Provider { source, .. }
                if source.code() == ActionErrorCode::ProviderRejected
        ));
        assert_eq!(
            world
                .engine()
                .action_state()
                .canonical(&families.focus, &mush_core::GOD)
                .unwrap()
                .unwrap()
                .as_slice(),
            before
        );
        assert!(!world.engine().action_state().has_prepared());
    }

    #[test]
    fn second_record_conflict_discards_the_first_scratch_mutation() {
        let (mut world, families) = toy_world_with_state();
        bootstrap_focus(&mut world, &families, mush_core::GOD, 3);
        bootstrap_guarded(&mut world, &families, mush_core::GOD, 9);
        let focus_before = world
            .engine()
            .action_state()
            .canonical(&families.focus, &mush_core::GOD)
            .unwrap()
            .unwrap()
            .as_slice()
            .to_vec();
        let guarded_before = world
            .engine()
            .action_state()
            .canonical(&families.guarded, &mush_core::GOD)
            .unwrap()
            .unwrap()
            .as_slice()
            .to_vec();
        let invocation = toy_brace_invocation(mush_core::GOD, 10, RequestUid::from_bytes([4; 16]));
        assert!(matches!(
            world
                .engine_mut()
                .prepare_registered_action(&toy_definitions().brace, &invocation),
            Err(ActionProviderPrepareError::Provider { .. })
        ));
        assert_eq!(
            world
                .engine()
                .action_state()
                .canonical(&families.focus, &mush_core::GOD)
                .unwrap()
                .unwrap()
                .as_slice(),
            focus_before
        );
        assert_eq!(
            world
                .engine()
                .action_state()
                .canonical(&families.guarded, &mush_core::GOD)
                .unwrap()
                .unwrap()
                .as_slice(),
            guarded_before
        );
        assert!(!world.engine().action_state().has_prepared());
    }

    #[test]
    fn trigger_refusal_discards_the_root_scratch_mutation() {
        let (mut world, families) = toy_world_with_state();
        bootstrap_focus(&mut world, &families, mush_core::GOD, 3);
        bootstrap_guarded(&mut world, &families, mush_core::GOD, 9);
        let focus_before = world
            .engine()
            .action_state()
            .canonical(&families.focus, &mush_core::GOD)
            .unwrap()
            .unwrap()
            .as_slice()
            .to_vec();
        let guarded_before = world
            .engine()
            .action_state()
            .canonical(&families.guarded, &mush_core::GOD)
            .unwrap()
            .unwrap()
            .as_slice()
            .to_vec();
        let invocation =
            toy_brace_invocation(mush_core::GOD, 10, RequestUid::from_bytes([0xA2; 16]));
        assert!(matches!(
            world
                .engine_mut()
                .prepare_registered_authoritative_action(&toy_definitions().brace, &invocation,),
            Err(ActionProviderPrepareError::Trigger { .. })
        ));
        assert_eq!(
            world
                .engine()
                .action_state()
                .canonical(&families.focus, &mush_core::GOD)
                .unwrap()
                .unwrap()
                .as_slice(),
            focus_before
        );
        assert_eq!(
            world
                .engine()
                .action_state()
                .canonical(&families.guarded, &mush_core::GOD)
                .unwrap()
                .unwrap()
                .as_slice(),
            guarded_before
        );
        assert!(!world.engine().action_state().has_prepared());
    }

    #[test]
    fn over_depth_trigger_cascade_aborts_the_root_spend() {
        let (mut world, families) = toy_world_with_state();
        bootstrap_focus(&mut world, &families, mush_core::GOD, 3);
        let before = world
            .engine()
            .action_state()
            .canonical(&families.focus, &mush_core::GOD)
            .unwrap()
            .unwrap()
            .as_slice()
            .to_vec();
        let invocation = toy_brace_invocation_with_cascade_steps(
            mush_core::GOD,
            10,
            RequestUid::from_bytes([0xA3; 16]),
            Some(u64::from(mush_core::MAX_TRIGGER_DEPTH) + 2),
        );
        assert!(matches!(
            world
                .engine_mut()
                .prepare_registered_authoritative_action(&toy_definitions().brace, &invocation,),
            Err(ActionProviderPrepareError::Cascade(
                mush_core::CascadeError::DepthExceeded { .. }
            ))
        ));
        assert_eq!(
            world
                .engine()
                .action_state()
                .canonical(&families.focus, &mush_core::GOD)
                .unwrap()
                .unwrap()
                .as_slice(),
            before
        );
        assert!(
            world
                .engine()
                .action_state()
                .get(&families.guarded, &mush_core::GOD)
                .unwrap()
                .is_none()
        );
        assert!(!world.engine().action_state().has_prepared());
    }
}

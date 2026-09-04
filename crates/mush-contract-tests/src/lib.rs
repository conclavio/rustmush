//! Cross-crate fixtures for the rules-engine contracts.
//!
//! This crate deliberately contains no production implementation. It is a
//! top-level workspace member so later proving gates can depend on core,
//! rulesets, tactics, simulation, server, and fault stores without creating a
//! production dependency cycle.

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionInvocationDraft, ActionOfferId,
    ActionPresentation, ActionProvider, ActionStateError, ActionTxn, AuthoritativeActionOutput,
    BoundedScalar, CanonicalHash, CanonicalId, ClockRef, ClockStamp, DefinitionDescriptor,
    DefinitionId, DefinitionKind, DefinitionRef, DefinitionRegistryBuilder,
    DefinitionValidationContext, EffectDuration, EffectInstance, EffectInstanceId, EffectTarget,
    Engine, GameModule, IdempotencyKey, IdempotencyScope, LegalActionQuery, ModError, ObjId,
    OwnerId, PerceivedRef, PrepareCtx, PreparedCausalRef, PreparedTrigger, PrincipalId,
    ProjectedLegalActionSink, ProjectedParameterSchema, ProjectedTargetSchema, ProviderOfferDraft,
    Registrar, RequestUid, ResolutionKeyRing, ResolutionSecret, ResourceOperation, RulesValue,
    SameCommitTrigger, ScopedIdempotencyKey, StateFamilyDescriptor, StateFamilyHandle,
    StateFamilyId, SubjectKey,
};
use mush_testkit::TestWorld;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// Canonical owner stamped on every toy definition.
pub const TOY_OWNER: &str = "toy";

/// Recursively find forbidden field names or string fragments in projected
/// JSON. Contract gates use this on the exact value handed to a player, bot,
/// or LLM so a newly nested private field cannot evade a top-level assertion.
///
/// Field matching is ASCII case-insensitive. Fragment matching is exact and
/// case-sensitive so callers can include opaque identifiers and digests.
#[must_use]
pub fn projection_json_leaks(
    value: &Value,
    forbidden_fields: &[&str],
    forbidden_fragments: &[&str],
) -> Vec<String> {
    fn visit(
        value: &Value,
        path: &str,
        forbidden_fields: &[&str],
        forbidden_fragments: &[&str],
        leaks: &mut Vec<String>,
    ) {
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    let child_path = format!("{path}.{key}");
                    if forbidden_fields
                        .iter()
                        .any(|forbidden| key.eq_ignore_ascii_case(forbidden))
                    {
                        leaks.push(format!("forbidden field at {child_path}"));
                    }
                    for fragment in forbidden_fragments {
                        if !fragment.is_empty() && key.contains(fragment) {
                            leaks.push(format!("forbidden fragment in field at {child_path}"));
                        }
                    }
                    visit(
                        child,
                        &child_path,
                        forbidden_fields,
                        forbidden_fragments,
                        leaks,
                    );
                }
            }
            Value::Array(array) => {
                for (index, child) in array.iter().enumerate() {
                    visit(
                        child,
                        &format!("{path}[{index}]"),
                        forbidden_fields,
                        forbidden_fragments,
                        leaks,
                    );
                }
            }
            Value::String(text) => {
                for fragment in forbidden_fragments {
                    if !fragment.is_empty() && text.contains(fragment) {
                        leaks.push(format!("forbidden fragment in string at {path}"));
                    }
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }

    let mut leaks = Vec::new();
    visit(
        value,
        "$",
        forbidden_fields,
        forbidden_fragments,
        &mut leaks,
    );
    leaks
}

/// Assert that projected JSON contains none of the supplied private fields or
/// exact private values. The failure includes structural paths, not the
/// private values themselves.
pub fn assert_projection_json_clean(
    surface: &str,
    value: &Value,
    forbidden_fields: &[&str],
    forbidden_fragments: &[&str],
) {
    let leaks = projection_json_leaks(value, forbidden_fields, forbidden_fragments);
    assert!(leaks.is_empty(), "{surface} projection leaked: {leaks:?}");
}

/// Assert that a rendered player/AI-facing text surface contains none of the
/// supplied exact private fragments.
pub fn assert_projection_text_clean(surface: &str, text: &str, forbidden_fragments: &[&str]) {
    let leaks: Vec<usize> = forbidden_fragments
        .iter()
        .enumerate()
        .filter_map(|(index, fragment)| {
            (!fragment.is_empty() && text.contains(fragment)).then_some(index)
        })
        .collect();
    assert!(
        leaks.is_empty(),
        "{surface} projection leaked forbidden fragment indexes: {leaks:?}"
    );
}

/// Exact identities used by the first action-kernel proving gate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToyDefinitions {
    pub focus: DefinitionRef,
    pub guarded: DefinitionRef,
    pub brace: DefinitionRef,
    pub guarded_expire: DefinitionRef,
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

fn guarded_expire_descriptor(guarded: DefinitionRef) -> DefinitionDescriptor {
    DefinitionDescriptor::new(
        DefinitionKind::new("action").expect("static definition kind"),
        DefinitionId::new("guarded_expire").expect("static definition id"),
        1,
        json!({ "expires": "guarded", "scheduler_only": true }),
    )
    .expect("nonzero static version")
    .with_references(vec![guarded])
    .as_current()
}

fn register_definitions(
    mut register: impl FnMut(DefinitionDescriptor) -> Result<DefinitionRef, String>,
) -> Result<ToyDefinitions, String> {
    let focus = register(focus_descriptor())?;
    let guarded = register(guarded_descriptor())?;
    let brace = register(brace_descriptor(focus.clone(), guarded.clone()))?;
    let guarded_expire = register(guarded_expire_descriptor(guarded.clone()))?;
    Ok(ToyDefinitions {
        focus,
        guarded,
        brace,
        guarded_expire,
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
        brace_descriptor(definitions.focus.clone(), definitions.guarded.clone())
    } else if entry.reference() == &definitions.guarded_expire {
        guarded_expire_descriptor(definitions.guarded.clone())
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
    pub value: BoundedScalar,
}

impl ToyFocusState {
    pub fn new(amount: u8) -> Self {
        Self {
            value: BoundedScalar::new(0, 3, i64::from(amount), 0)
                .expect("toy focus bootstrap is bounded"),
        }
    }

    #[must_use]
    pub fn amount(&self) -> i64 {
        self.value.current()
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToyGuardedState {
    pub effect: EffectInstance,
}

impl ToyGuardedState {
    #[must_use]
    pub fn expires_at(&self) -> &ClockStamp {
        match self.effect.duration() {
            EffectDuration::Until { expires } => expires,
            EffectDuration::Permanent => unreachable!("toy guarded is always temporary"),
        }
    }
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
    fn scene_turn(invocation: &ActionInvocation) -> Result<ClockStamp, ActionError> {
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
        let stamp = parameters
            .get("scene_turn")
            .cloned()
            .and_then(|value| serde_json::from_value::<ClockStamp>(value).ok())
            .ok_or_else(|| {
                ActionError::new(
                    ActionErrorCode::InvalidRequest,
                    false,
                    "brace requires a typed scene_turn clock stamp",
                )
                .expect("static action error is bounded")
            })?;
        if !matches!(stamp.clock, ClockRef::SceneTurn { .. }) {
            return Err(ActionError::new(
                ActionErrorCode::InvalidRequest,
                false,
                "brace requires the scene-turn clock domain",
            )
            .expect("static action error is bounded"));
        }
        Ok(stamp)
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

    fn guarded_effect(
        &self,
        invocation: &ActionInvocation,
        applied_at: ClockStamp,
    ) -> Result<EffectInstance, ActionError> {
        let expires_at = applied_at.at.checked_add(2).ok_or_else(|| {
            ActionError::new(
                ActionErrorCode::BoundsExceeded,
                false,
                "guarded expiry exceeds the scene-turn domain",
            )
            .expect("static action error is bounded")
        })?;
        let expires = ClockStamp {
            clock: applied_at.clock.clone(),
            at: expires_at,
        };
        let target = EffectTarget::Subject {
            subject: SubjectKey::Object {
                id: invocation.actor(),
            },
        };
        let id =
            EffectInstanceId::derive(invocation.request(), &self.guarded_definition, &target, 1)
                .map_err(|_| {
                    ActionError::new(
                        ActionErrorCode::BoundsExceeded,
                        false,
                        "guarded identity exceeds canonical bounds",
                    )
                    .expect("static action error is bounded")
                })?;
        let state = RulesValue::new(self.guarded_definition.clone(), json!({})).map_err(|_| {
            ActionError::new(
                ActionErrorCode::BoundsExceeded,
                false,
                "guarded state exceeds canonical bounds",
            )
            .expect("static action error is bounded")
        })?;
        EffectInstance::new(
            id,
            self.guarded_definition.clone(),
            PreparedCausalRef::new(invocation.request(), None, 0).map_err(|_| {
                ActionError::new(
                    ActionErrorCode::Integrity,
                    false,
                    "guarded source causality is invalid",
                )
                .expect("static action error is bounded")
            })?,
            target,
            applied_at,
            EffectDuration::Until { expires },
            1,
            state,
        )
        .map_err(|_| {
            ActionError::new(
                ActionErrorCode::Integrity,
                false,
                "guarded effect invariants were refused",
            )
            .expect("static action error is bounded")
        })
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

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        let subject_actor = match query.truth_subject() {
            SubjectKey::Object { id } => *id,
            SubjectKey::Scoped { .. } => return Ok(None),
        };
        if action != &self.brace_definition
            || query.actor() != subject_actor
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let focus = engine
            .action_state()
            .get(&self.focus, &query.actor())
            .map_err(|error| Self::state_error(&error))?;
        let guarded = engine
            .action_state()
            .get(&self.guarded, &query.actor())
            .map_err(|error| Self::state_error(&error))?;
        if !focus.is_some_and(|focus| focus.amount() > 0) || guarded.is_some() {
            return Ok(None);
        }
        let scene_turn = query.now().cloned().ok_or_else(|| {
            ActionError::new(
                ActionErrorCode::Unavailable,
                false,
                "brace projection requires the current scene turn",
            )
            .expect("static projection error is bounded")
        })?;
        if !matches!(scene_turn.clock, ClockRef::SceneTurn { .. }) {
            return Err(ActionError::new(
                ActionErrorCode::InvalidRequest,
                false,
                "brace projection requires a scene-turn clock",
            )
            .expect("static projection error is bounded"));
        }
        let definitions = engine.registry.definitions().ok_or_else(|| {
            ActionError::new(
                ActionErrorCode::Integrity,
                false,
                "toy definition registry is unavailable",
            )
            .expect("static projection error is bounded")
        })?;
        let mut guard_builder = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| {
                ActionError::new(
                    ActionErrorCode::Integrity,
                    false,
                    "toy guard registry is unavailable",
                )
                .expect("static projection error is bounded")
            })?;
        guard_builder
            .add_definition(action.clone(), definitions)
            .map_err(|_| {
                ActionError::new(
                    ActionErrorCode::Integrity,
                    false,
                    "brace definition guard is unavailable",
                )
                .expect("static projection error is bounded")
            })?;
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({ "scene_turn": scene_turn }),
            parameters: ProjectedParameterSchema::empty(),
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: ActionPresentation::new(
                "brace",
                "Brace",
                "Spend one focus to become guarded for two scene turns.",
            )
            .map_err(|_| {
                ActionError::new(
                    ActionErrorCode::Integrity,
                    false,
                    "brace presentation is invalid",
                )
                .expect("static projection error is bounded")
            })?,
            guards: guard_builder.finish().map_err(|_| {
                ActionError::new(
                    ActionErrorCode::Integrity,
                    false,
                    "brace offer guards are invalid",
                )
                .expect("static projection error is bounded")
            })?,
        }))
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
        let value = focus
            .value
            .checked_apply(ResourceOperation::Decrease { amount: 1 })
            .map_err(|_| {
                ActionError::new(
                    ActionErrorCode::ProviderRejected,
                    false,
                    "insufficient focus",
                )
                .expect("static action error is bounded")
            })?;
        let effect = self.guarded_effect(invocation, turn)?;
        txn.replace(&self.focus, &actor, ToyFocusState { value })
            .map_err(|error| Self::state_error(&error))?;
        txn.insert(&self.guarded, &actor, ToyGuardedState { effect })
            .map_err(|error| Self::state_error(&error))?;
        Ok(())
    }

    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
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
        let value = focus
            .value
            .checked_apply(ResourceOperation::Decrease { amount: 1 })
            .map_err(|_| {
                ActionError::new(
                    ActionErrorCode::ProviderRejected,
                    false,
                    "insufficient focus",
                )
                .expect("static action error is bounded")
            })?;
        txn.replace(&self.focus, &actor, ToyFocusState { value })
            .map_err(|error| Self::state_error(&error))?;
        let inputs_hash = CanonicalHash::for_canonical_value(
            "toy_brace_inputs.v1",
            1,
            &json!({ "actor": actor, "scene_turn": turn, "focus_before": focus.value }),
            4 * 1024,
        )
        .map_err(|_| {
            ActionError::new(
                ActionErrorCode::BoundsExceeded,
                false,
                "brace resolution inputs exceed canonical bounds",
            )
            .expect("static action error is bounded")
        })?;
        let mut rng = context
            .start_resolution(
                &CanonicalId::new("toy_brace").expect("static resolution domain"),
                inputs_hash,
            )
            .map_err(|_| {
                ActionError::new(
                    ActionErrorCode::Unavailable,
                    false,
                    "authoritative resolution RNG is unavailable",
                )
                .expect("static action error is bounded")
            })?;
        let initiative_tiebreak = rng
            .draw_bounded_u64(
                &CanonicalId::new("initiative_tiebreak").expect("static draw label"),
                100,
            )
            .map_err(|_| {
                ActionError::new(
                    ActionErrorCode::BoundsExceeded,
                    false,
                    "brace resolution draw was refused",
                )
                .expect("static action error is bounded")
            })?;
        let outcome = RulesValue::new(
            self.brace_definition.clone(),
            json!({
                "actor": actor.0,
                "event": "braced",
                "focus": value.current(),
                "initiative_tiebreak": initiative_tiebreak,
            }),
        )
        .map_err(|_| {
            ActionError::new(
                ActionErrorCode::BoundsExceeded,
                false,
                "brace outcome exceeds canonical bounds",
            )
            .expect("static action error is bounded")
        })?;
        let resolution = rng
            .into_prepared(
                vec![self.brace_definition.clone()],
                RulesValue::new(self.brace_definition.clone(), json!({ "focus_spent": 1 }))
                    .map_err(|_| {
                        ActionError::new(
                            ActionErrorCode::BoundsExceeded,
                            false,
                            "brace modifiers exceed canonical bounds",
                        )
                        .expect("static action error is bounded")
                    })?,
                outcome.clone(),
            )
            .map_err(|_| {
                ActionError::new(
                    ActionErrorCode::Integrity,
                    false,
                    "brace resolution transcript is invalid",
                )
                .expect("static action error is bounded")
            })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_resolution(resolution)
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
            .cloned()
            .and_then(|value| serde_json::from_value::<ClockStamp>(value).ok())
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
        let effect = self.guarded_effect(invocation, turn)?;
        let expires = match effect.duration() {
            EffectDuration::Until { expires } => expires.at,
            EffectDuration::Permanent => unreachable!("toy guarded is temporary"),
        };
        txn.insert(
            &self.guarded,
            &invocation.actor(),
            ToyGuardedState { effect },
        )
        .map_err(|error| Self::state_error(&error))?;
        let outcome = RulesValue::new(
            self.guarded_definition.clone(),
            json!({
                "actor": invocation.actor().0,
                "event": "guarded_applied",
                "expires_after_scene_turn": expires,
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

#[derive(Debug)]
struct GuardedExpiryProvider {
    guarded: StateFamilyHandle<ObjId, ToyGuardedState>,
    expire_definition: DefinitionRef,
    guarded_definition: DefinitionRef,
}

impl GuardedExpiryProvider {
    fn parameters(
        invocation: &ActionInvocation,
    ) -> Result<(EffectInstanceId, ClockStamp), ActionError> {
        let parameters = invocation.parameters().as_object().filter(|value| {
            value.len() == 2 && value.contains_key("effect") && value.contains_key("now")
        });
        let Some(parameters) = parameters else {
            return Err(ActionError::new(
                ActionErrorCode::InvalidRequest,
                false,
                "guarded expiry requires exact effect and now parameters",
            )
            .expect("static expiry error is bounded"));
        };
        let effect = serde_json::from_value::<EffectInstanceId>(parameters["effect"].clone())
            .map_err(|_| {
                ActionError::new(
                    ActionErrorCode::InvalidRequest,
                    false,
                    "guarded expiry effect identity is invalid",
                )
                .expect("static expiry error is bounded")
            })?;
        let now =
            serde_json::from_value::<ClockStamp>(parameters["now"].clone()).map_err(|_| {
                ActionError::new(
                    ActionErrorCode::InvalidRequest,
                    false,
                    "guarded expiry clock stamp is invalid",
                )
                .expect("static expiry error is bounded")
            })?;
        Ok((effect, now))
    }
}

impl ActionProvider for GuardedExpiryProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
            return Err(ActionError::new(
                ActionErrorCode::InvalidRequest,
                false,
                "guarded expiry targets only its scheduled actor",
            )
            .expect("static expiry error is bounded"));
        }
        Self::parameters(invocation).map(|_| ())
    }

    fn prepare_invocation(
        &self,
        _invocation: &ActionInvocation,
        _txn: &mut ActionTxn<'_>,
    ) -> Result<(), ActionError> {
        Err(ActionError::new(
            ActionErrorCode::Unavailable,
            false,
            "guarded expiry requires the authoritative action coordinator",
        )
        .expect("static expiry error is bounded"))
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(invocation)?;
        let (expected_id, now) = Self::parameters(invocation)?;
        let actor = invocation.actor();
        let guarded = txn
            .get(&self.guarded, &actor)
            .map_err(|error| BraceProvider::state_error(&error))?
            .cloned()
            .ok_or_else(|| {
                ActionError::new(
                    ActionErrorCode::Stale,
                    false,
                    "guarded effect is no longer current",
                )
                .expect("static expiry error is bounded")
            })?;
        if guarded.effect.id() != expected_id {
            return Err(ActionError::new(
                ActionErrorCode::Stale,
                false,
                "guarded effect identity is stale",
            )
            .expect("static expiry error is bounded"));
        }
        if !guarded.effect.is_expired_at(&now).map_err(|_| {
            ActionError::new(
                ActionErrorCode::Stale,
                false,
                "guarded expiry clock generation is stale",
            )
            .expect("static expiry error is bounded")
        })? {
            return Err(ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "guarded effect has not expired",
            )
            .expect("static expiry error is bounded"));
        }
        txn.remove(&self.guarded, &actor)
            .map_err(|error| BraceProvider::state_error(&error))?;
        let source = guarded.effect.source();
        let outcome = RulesValue::new(
            self.expire_definition.clone(),
            json!({
                "actor": actor.0,
                "effect": guarded.effect.id(),
                "event": "guarded_expired",
                "source": source,
                "expired_at": now,
            }),
        )
        .map_err(|_| {
            ActionError::new(
                ActionErrorCode::BoundsExceeded,
                false,
                "guarded expiry outcome exceeds canonical bounds",
            )
            .expect("static expiry error is bounded")
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
                if value.value.minimum() == 0 && value.value.maximum() == 3 {
                    Ok(())
                } else {
                    Err("focus bounds must remain exactly 0..=3".to_owned())
                }
            },
            |value: &ToyFocusState| serde_json::to_value(value).map_err(|error| error.to_string()),
        )?;
        let guarded_definition = actual.guarded.clone();
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
            move |value: &ToyGuardedState| {
                let target_is_subject = matches!(
                    value.effect.target(),
                    EffectTarget::Subject {
                        subject: SubjectKey::Object { .. }
                    }
                );
                let duration_is_scene_turn = matches!(
                    value.effect.duration(),
                    EffectDuration::Until {
                        expires: ClockStamp {
                            clock: ClockRef::SceneTurn { .. },
                            ..
                        }
                    }
                );
                if value.effect.definition() == &guarded_definition
                    && value.effect.state().schema() == &guarded_definition
                    && target_is_subject
                    && duration_is_scene_turn
                {
                    Ok(())
                } else {
                    Err("guarded instance does not match its exact toy contract".to_owned())
                }
            },
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
                brace_definition: actual.brace.clone(),
                guarded_definition: actual.guarded.clone(),
            },
        )?;
        registrar.action_provider(
            actual.guarded_expire.clone(),
            GuardedExpiryProvider {
                guarded: families.guarded.clone(),
                expire_definition: actual.guarded_expire.clone(),
                guarded_definition: actual.guarded.clone(),
            },
        )?;
        *self.families.lock().expect("toy family capture poisoned") = Some(families);
        Ok(())
    }
}

/// Fresh in-process world composed with the deterministic toy provider.
#[must_use]
pub fn toy_world() -> TestWorld {
    let mut world = TestWorld::with_modules(vec![Box::<ToyRulesModule>::default()]);
    install_toy_resolution_keys(world.engine_mut());
    world
}

/// Fresh toy world plus the composition-branded state capabilities used by
/// the logical Gate-1 slice.
#[must_use]
pub fn toy_world_with_state() -> (TestWorld, ToyStateFamilies) {
    let captured = Arc::new(Mutex::new(None));
    let module = ToyRulesModule::with_capture(Arc::clone(&captured));
    let mut world = TestWorld::with_modules(vec![Box::new(module)]);
    install_toy_resolution_keys(world.engine_mut());
    let families = captured
        .lock()
        .expect("toy family capture poisoned")
        .clone()
        .expect("toy module registered state families");
    (world, families)
}

fn install_toy_resolution_keys(engine: &mut Engine) {
    engine.set_resolution_keys(
        ResolutionKeyRing::new(
            ResolutionSecret::new(1, [0x71; 32]).expect("toy resolution secret"),
            [],
        )
        .expect("toy resolution key ring"),
    );
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
        txn.insert(&families.focus, &actor, ToyFocusState::new(amount))
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

/// Canonical scene clock used by the Gate-1 fixture. The scene and generation
/// are stable across a simulated restart; changing either makes old stamps
/// intentionally incomparable.
pub fn toy_scene_turn(at: u64) -> ClockStamp {
    ClockStamp {
        clock: ClockRef::SceneTurn {
            scene: 1,
            generation: 1,
        },
        at: i64::try_from(at).expect("toy scene turn fits i64"),
    }
}

/// Regenerate the viewer's bounded Gate-1 offer set from current typed truth.
/// The scene stamp is retained only in the internal offer and inserted into
/// the invocation after opaque binding resolution.
pub fn toy_brace_offers(
    engine: &Engine,
    actor: ObjId,
    scene_turn: u64,
    projection_generation: u64,
) -> Result<ProjectedLegalActionSink, String> {
    let principal = PrincipalId::Object { id: actor };
    let query = LegalActionQuery::new(
        principal,
        actor,
        SubjectKey::Object { id: actor },
        PerceivedRef::SelfActor,
        projection_generation,
        Some(toy_scene_turn(scene_turn)),
    )
    .map_err(|error| error.to_string())?;
    engine
        .project_legal_actions(&query)
        .map_err(|error| error.to_string())
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
        || json!({ "scene_turn": toy_scene_turn(scene_turn) }),
        |steps| json!({ "scene_turn": toy_scene_turn(scene_turn), "cascade_steps": steps }),
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

/// Server-scheduled authoritative expiry for one exact guarded instance.
/// The deterministic idempotency key is the instance identity, so restart or
/// duplicate timer delivery cannot remove a replacement effect.
pub fn toy_guarded_expiry_invocation(
    actor: ObjId,
    effect: EffectInstanceId,
    scene_turn: u64,
    request: RequestUid,
) -> ActionInvocation {
    toy_guarded_expiry_invocation_at(actor, effect, toy_scene_turn(scene_turn), request)
}

/// Clock-explicit variant used by the scheduler adapter and generation tests.
pub fn toy_guarded_expiry_invocation_at(
    actor: ObjId,
    effect: EffectInstanceId,
    now: ClockStamp,
    request: RequestUid,
) -> ActionInvocation {
    let definitions = toy_definitions();
    let owner = OwnerId::new(TOY_OWNER).expect("static owner");
    let principal = PrincipalId::Object { id: actor };
    let idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner,
            namespace: CanonicalId::new("guarded_expire").expect("static namespace"),
        },
        0,
        IdempotencyKey::Deterministic(effect.0),
    );
    let offered_action = ActionOfferId(
        CanonicalHash::for_canonical_value(
            "toy.offer.v1",
            1,
            &json!({ "action": definitions.guarded_expire }),
            4 * 1024,
        )
        .expect("static offer is canonical"),
    );
    ActionInvocation::new(ActionInvocationDraft {
        request,
        principal,
        actor,
        offered_action,
        subject: PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters: json!({ "effect": effect, "now": now }),
        idempotency,
    })
    .expect("toy expiry invocation is canonical")
}

/// Build a bounded exact rules value for gate fixtures.
pub fn toy_rules_value(definition: DefinitionRef, data: Value) -> RulesValue {
    RulesValue::new(definition, data).expect("toy fixture must fit RulesValue bounds")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{
        ActionProviderPrepareError, OfferExecutionContext, OfferKeyRing, OfferSubmission,
        ProjectedLegalAction, ProjectionScopeId, RevisionSet,
    };

    fn bootstrap_focus(
        world: &mut TestWorld,
        families: &ToyStateFamilies,
        actor: ObjId,
        amount: u8,
    ) {
        install_toy_focus(world.engine_mut(), families, actor, amount);
    }

    #[test]
    fn recursive_projection_inspector_reports_nested_fields_and_values() {
        let value = json!({
            "public": [{
                "nested": {
                    "truth_subject": "hidden-contact-17"
                }
            }]
        });
        let leaks = projection_json_leaks(&value, &["truth_subject"], &["hidden-contact-17"]);
        assert_eq!(leaks.len(), 2);
        assert!(
            leaks
                .iter()
                .all(|leak| leak.contains("$.public[0].nested.truth_subject"))
        );
    }

    fn bootstrap_guarded(
        world: &mut TestWorld,
        families: &ToyStateFamilies,
        actor: ObjId,
        expiry: u64,
    ) {
        let definitions = toy_definitions();
        let request = RequestUid::from_bytes([0xEE; 16]);
        let target = EffectTarget::Subject {
            subject: SubjectKey::Object { id: actor },
        };
        let id = EffectInstanceId::derive(request, &definitions.guarded, &target, 1).unwrap();
        let effect = EffectInstance::new(
            id,
            definitions.guarded.clone(),
            PreparedCausalRef::new(request, None, 0).unwrap(),
            target,
            toy_scene_turn(expiry.checked_sub(2).unwrap()),
            EffectDuration::Until {
                expires: toy_scene_turn(expiry),
            },
            1,
            RulesValue::new(definitions.guarded, json!({})).unwrap(),
        )
        .unwrap();
        let prepared = {
            let mut txn = world
                .engine_mut()
                .action_transaction(&families.guarded)
                .expect("toy guarded transaction");
            txn.insert(&families.guarded, &actor, ToyGuardedState { effect })
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
                .unwrap()
                .unwrap()
                .amount(),
            3
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
                .unwrap()
                .unwrap()
                .amount(),
            2
        );
        assert_eq!(
            world
                .engine()
                .action_state()
                .get(&families.guarded, &mush_core::GOD)
                .unwrap()
                .unwrap()
                .expires_at(),
            &toy_scene_turn(13)
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
        let (_outcome, resolution, definitions, witnesses, outbox, events) = cascade.into_parts();
        assert!(resolution.is_some());
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
                .unwrap()
                .unwrap()
                .amount(),
            3,
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

    #[test]
    fn brace_offer_projects_no_hidden_clock_and_routes_to_the_same_provider() {
        let (mut world, families) = toy_world_with_state();
        bootstrap_focus(&mut world, &families, mush_core::GOD, 3);
        let keys =
            OfferKeyRing::new(mush_core::OfferSecret::new(1, [0x41; 32]).unwrap(), []).unwrap();
        world.engine_mut().set_offer_keys(keys.clone());
        let offers = toy_brace_offers(world.engine(), mush_core::GOD, 11, 4)
            .expect("offer projection succeeds");
        assert_eq!(offers.len(), 1);
        let projected = offers.projected().pop().unwrap();
        let definition_digest = toy_definitions().brace.digest().to_string();
        let forbidden_fields = [
            "action",
            "definition",
            "fixed_parameters",
            "guards",
            "guard_hash",
            "provider",
            "provider_ordinal",
            "projection_scope",
            "revision_set",
            "truth_subject",
            "viewer_scope",
        ];
        let forbidden_fragments = ["scene_turn", definition_digest.as_str()];
        let projected_json = serde_json::to_value(&projected).unwrap();
        assert_projection_json_clean(
            "structured legal action",
            &projected_json,
            &forbidden_fields,
            &forbidden_fragments,
        );

        let view = mush_proto::ViewEnvelope::keyframe(
            "legal.actions",
            1,
            7,
            1,
            json!([projected.clone()]),
        )
        .unwrap();
        assert_projection_json_clean(
            "semantic legal-action view",
            &serde_json::to_value(view).unwrap(),
            &forbidden_fields,
            &forbidden_fragments,
        );

        let mock_llm_snapshot = json!({
            "decision": "choose one offered tool",
            "tools": [projected.clone()],
        });
        assert_projection_json_clean(
            "mock LLM legal-action snapshot",
            &mock_llm_snapshot,
            &forbidden_fields,
            &forbidden_fragments,
        );

        let request = RequestUid::from_bytes([0xD1; 16]);
        let principal = PrincipalId::Object { id: mush_core::GOD };
        let viewer_scope = ProjectionScopeId::derive(&principal, 4).unwrap();
        let resolved = world
            .engine()
            .resolve_legal_action(
                &offers,
                OfferExecutionContext {
                    request,
                    principal: principal.clone(),
                    actor: mush_core::GOD,
                    idempotency: ScopedIdempotencyKey::new(
                        IdempotencyScope {
                            principal,
                            owner: OwnerId::new(TOY_OWNER).unwrap(),
                            namespace: CanonicalId::new("brace").unwrap(),
                        },
                        0,
                        IdempotencyKey::Request(request),
                    ),
                    viewer_scope,
                    now: None,
                },
                OfferSubmission {
                    offer: projected.offer(),
                    subject: projected.subject().clone(),
                    targets: Vec::new(),
                    parameters: json!({}),
                },
            )
            .expect("opaque offer resolves");
        assert_eq!(
            serde_json::from_value::<ClockStamp>(
                resolved.invocation().parameters()["scene_turn"].clone()
            )
            .unwrap(),
            toy_scene_turn(11)
        );
        let (action, invocation, _guards) = resolved.into_parts();
        let (prepared, _) = world
            .engine_mut()
            .prepare_registered_authoritative_action(&action, &invocation)
            .expect("resolved offer reaches the canonical provider");
        world.engine_mut().abort_prepared_transaction(prepared);

        let changed = toy_brace_offers(world.engine(), mush_core::GOD, 11, 5)
            .unwrap()
            .projected()
            .pop()
            .unwrap();
        assert_ne!(projected.offer(), changed.offer());

        let (mut empty_world, empty_families) = toy_world_with_state();
        bootstrap_focus(&mut empty_world, &empty_families, mush_core::GOD, 0);
        empty_world.engine_mut().set_offer_keys(keys.clone());
        assert!(
            toy_brace_offers(empty_world.engine(), mush_core::GOD, 11, 4,)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn line_structured_npc_and_mock_llm_use_one_offer_and_provider_contract() {
        #[derive(Clone, Copy, Debug)]
        enum Caller {
            Line,
            Structured,
            DeterministicNpc,
            MockLlm,
        }

        fn choose(caller: Caller, offers: &[ProjectedLegalAction]) -> OfferSubmission {
            let projected = match caller {
                Caller::Line => offers
                    .iter()
                    .find(|offer| offer.presentation().verb.eq_ignore_ascii_case("brace"))
                    .cloned()
                    .expect("line verb is offered"),
                Caller::Structured => offers.first().cloned().expect("panel has one action"),
                Caller::DeterministicNpc => offers
                    .iter()
                    .min_by_key(|offer| offer.presentation().verb.as_str())
                    .cloned()
                    .expect("deterministic policy has an action"),
                Caller::MockLlm => {
                    let prompt = serde_json::to_string(offers).expect("LLM projection serializes");
                    let visible: Vec<ProjectedLegalAction> =
                        serde_json::from_str(&prompt).expect("mock LLM sees only projected DTOs");
                    visible
                        .into_iter()
                        .find(|offer| offer.presentation().verb == "brace")
                        .expect("mock LLM chooses one offered tool")
                }
            };
            OfferSubmission {
                offer: projected.offer(),
                subject: projected.subject().clone(),
                targets: Vec::new(),
                parameters: json!({}),
            }
        }

        let (mut world, families) = toy_world_with_state();
        bootstrap_focus(&mut world, &families, mush_core::GOD, 3);
        let keys =
            OfferKeyRing::new(mush_core::OfferSecret::new(1, [0x42; 32]).unwrap(), []).unwrap();
        world.engine_mut().set_offer_keys(keys);
        let offers = toy_brace_offers(world.engine(), mush_core::GOD, 11, 4).unwrap();
        let projected = offers.projected();
        let principal = PrincipalId::Object { id: mush_core::GOD };
        let viewer_scope = ProjectionScopeId::derive(&principal, 4).unwrap();

        for (ordinal, caller) in [
            Caller::Line,
            Caller::Structured,
            Caller::DeterministicNpc,
            Caller::MockLlm,
        ]
        .into_iter()
        .enumerate()
        {
            let request = RequestUid::from_bytes([u8::try_from(ordinal + 1).unwrap(); 16]);
            let resolved = world
                .engine()
                .resolve_legal_action(
                    &offers,
                    OfferExecutionContext {
                        request,
                        principal: principal.clone(),
                        actor: mush_core::GOD,
                        idempotency: ScopedIdempotencyKey::new(
                            IdempotencyScope {
                                principal: principal.clone(),
                                owner: OwnerId::new(TOY_OWNER).unwrap(),
                                namespace: CanonicalId::new("brace").unwrap(),
                            },
                            0,
                            IdempotencyKey::Request(request),
                        ),
                        viewer_scope,
                        now: None,
                    },
                    choose(caller, &projected),
                )
                .unwrap_or_else(|error| panic!("{caller:?} failed shared resolution: {error}"));
            assert_eq!(resolved.action(), &toy_definitions().brace);
            let (action, invocation, _) = resolved.into_parts();
            let (prepared, _) = world
                .engine_mut()
                .prepare_registered_authoritative_action(&action, &invocation)
                .unwrap_or_else(|error| panic!("{caller:?} missed shared provider: {error}"));
            world.engine_mut().abort_prepared_transaction(prepared);
        }

        let request = RequestUid::from_bytes([0x7F; 16]);
        assert!(
            world
                .engine()
                .resolve_legal_action(
                    &offers,
                    OfferExecutionContext {
                        request,
                        principal: principal.clone(),
                        actor: mush_core::GOD,
                        idempotency: ScopedIdempotencyKey::new(
                            IdempotencyScope {
                                principal,
                                owner: OwnerId::new(TOY_OWNER).unwrap(),
                                namespace: CanonicalId::new("brace").unwrap(),
                            },
                            0,
                            IdempotencyKey::Request(request),
                        ),
                        viewer_scope,
                        now: None,
                    },
                    OfferSubmission {
                        offer: ActionOfferId(CanonicalHash::from_bytes([0xFF; 32])),
                        subject: PerceivedRef::SelfActor,
                        targets: Vec::new(),
                        parameters: json!({}),
                    },
                )
                .is_err(),
            "a mock LLM cannot invent an unoffered action token"
        );
    }

    #[test]
    fn guarded_expiry_is_exact_clocked_and_removes_only_its_instance() {
        let (mut world, families) = toy_world_with_state();
        bootstrap_focus(&mut world, &families, mush_core::GOD, 3);
        let brace_request = RequestUid::from_bytes([0xC1; 16]);
        let brace = toy_brace_invocation(mush_core::GOD, 11, brace_request);
        let (prepared, _) = world
            .engine_mut()
            .prepare_registered_authoritative_action(&toy_definitions().brace, &brace)
            .expect("brace prepares");
        let (_, installation) = prepared.into_parts();
        world.engine_mut().install_prepared_action(installation);
        let guarded = world
            .engine()
            .action_state()
            .get(&families.guarded, &mush_core::GOD)
            .unwrap()
            .unwrap()
            .clone();

        let early = toy_guarded_expiry_invocation(
            mush_core::GOD,
            guarded.effect.id(),
            12,
            RequestUid::from_bytes([0xC2; 16]),
        );
        assert!(matches!(
            world.engine_mut().prepare_registered_authoritative_action(
                &toy_definitions().guarded_expire,
                &early
            ),
            Err(ActionProviderPrepareError::Provider { source, .. })
                if source.code() == ActionErrorCode::ProviderRejected
        ));
        assert_eq!(
            world
                .engine()
                .action_state()
                .get(&families.guarded, &mush_core::GOD)
                .unwrap(),
            Some(&guarded)
        );

        let stale_clock = toy_guarded_expiry_invocation_at(
            mush_core::GOD,
            guarded.effect.id(),
            ClockStamp {
                clock: ClockRef::SceneTurn {
                    scene: 1,
                    generation: 2,
                },
                at: 13,
            },
            RequestUid::from_bytes([0xC3; 16]),
        );
        assert!(matches!(
            world.engine_mut().prepare_registered_authoritative_action(
                &toy_definitions().guarded_expire,
                &stale_clock
            ),
            Err(ActionProviderPrepareError::Provider { source, .. })
                if source.code() == ActionErrorCode::Stale
        ));

        let due = toy_guarded_expiry_invocation(
            mush_core::GOD,
            guarded.effect.id(),
            13,
            RequestUid::from_bytes([0xC4; 16]),
        );
        let (prepared, cascade) = world
            .engine_mut()
            .prepare_registered_authoritative_action(&toy_definitions().guarded_expire, &due)
            .expect("due expiry prepares");
        assert_eq!(prepared.operation_count(), 1);
        let (outcome, _, _, _, _, events) = cascade.into_parts();
        assert_eq!(
            outcome.data()["source"]["root_request"],
            brace_request.to_hex()
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind().as_str(), "guarded_expire");
        let (_, installation) = prepared.into_parts();
        world.engine_mut().install_prepared_action(installation);
        assert!(
            world
                .engine()
                .action_state()
                .get(&families.guarded, &mush_core::GOD)
                .unwrap()
                .is_none()
        );
    }
}

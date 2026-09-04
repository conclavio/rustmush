//! Cross-crate fixtures for the rules-engine contracts.
//!
//! This crate deliberately contains no production implementation. It is a
//! top-level workspace member so later proving gates can depend on core,
//! rulesets, tactics, simulation, server, and fault stores without creating a
//! production dependency cycle.

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionInvocationDraft, ActionOfferId,
    ActionPresentation, ActionProvider, ActionStateError, ActionTxn, ActionWindow,
    ActionWindowDraft, ActionWindowError, ActionWindowId, ActionWindowRecord,
    ActionWindowStateCodec, AuthoritativeActionOutput, BoundedScalar, CanonicalHash, CanonicalId,
    ClockRef, ClockStamp, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionRegistryBuilder, DefinitionValidationContext, DurableSchedule,
    DurableScheduleStateCodec, EffectDuration, EffectInstance, EffectInstanceId, EffectTarget,
    Engine, GameModule, IdempotencyKey, IdempotencyScope, LegalActionQuery, ModError, ObjId,
    OwnerId, PerceivedRef, PrepareCtx, PreparedCausalRef, PreparedTrigger, PrincipalId,
    ProjectedLegalActionSink, ProjectedParameterSchema, ProjectedTargetSchema, ProviderOfferDraft,
    Registrar, RequestUid, ReservationBook, ReservationBookError, ReservationBookId,
    ReservationBookStateCodec, ResolutionKeyRing, ResolutionSecret, ResourceOperation, RulesValue,
    SameCommitTrigger, ScheduleActionAdapter, ScheduleActionDraft, ScheduleAdvance, ScheduleId,
    ScopedIdempotencyKey, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId, SubjectKey,
    WindowIntent, WindowMode, WindowOrderKey, WindowRestartPolicy, WindowTimeoutPolicy,
    action_window_state_family_descriptor, durable_schedule_state_family_descriptor,
    reservation_book_state_family_descriptor,
};
use mush_core::{
    EffectProvider, ResourceProvider, ResourceProviderContext, ResourceProviderFailure, ResourceRef,
};
use mush_testkit::TestWorld;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[cfg(test)]
use mush_core::{CommitToken, ReservationId, ReservationRecord};

/// Canonical owner stamped on every toy definition.
pub const TOY_OWNER: &str = "toy";

#[cfg(test)]
fn toy_prepare_token() -> CommitToken {
    CommitToken::from_bytes([0xC0; 16])
}

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
    pub window_action: DefinitionRef,
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

fn window_action_descriptor() -> DefinitionDescriptor {
    DefinitionDescriptor::new(
        DefinitionKind::new("action").expect("static definition kind"),
        DefinitionId::new("window_operation").expect("static definition id"),
        1,
        json!({
            "operations": ["open", "submit", "close", "resolve"],
            "window": "round_7",
        }),
    )
    .expect("nonzero static version")
    .as_current()
}

fn register_definitions(
    mut register: impl FnMut(DefinitionDescriptor) -> Result<DefinitionRef, String>,
) -> Result<ToyDefinitions, String> {
    let focus = register(focus_descriptor())?;
    let guarded = register(guarded_descriptor())?;
    let brace = register(brace_descriptor(focus.clone(), guarded.clone()))?;
    let guarded_expire = register(guarded_expire_descriptor(guarded.clone()))?;
    let window_action = register(window_action_descriptor())?;
    Ok(ToyDefinitions {
        focus,
        guarded,
        brace,
        guarded_expire,
        window_action,
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
    } else if entry.reference() == &definitions.window_action {
        window_action_descriptor()
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

/// Provider-specific durable addressing for the toy expiry schedule.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToyScheduledExpiry {
    pub actor: ObjId,
    pub effect: EffectInstanceId,
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
    pub reservations: StateFamilyHandle<ReservationBookId, ReservationBook>,
    pub guarded: StateFamilyHandle<ObjId, ToyGuardedState>,
    pub windows: StateFamilyHandle<ActionWindowId, ActionWindowRecord>,
    pub schedules: StateFamilyHandle<ScheduleId, DurableSchedule>,
    pub scheduled_expiries: StateFamilyHandle<ScheduleId, ToyScheduledExpiry>,
}

#[derive(Debug)]
struct ToyFocusResourceProvider {
    focus: StateFamilyHandle<ObjId, ToyFocusState>,
}

impl ResourceProvider for ToyFocusResourceProvider {
    fn read(
        &self,
        context: &ResourceProviderContext<'_>,
        resource: &ResourceRef,
    ) -> Result<Option<BoundedScalar>, ResourceProviderFailure> {
        let SubjectKey::Object { id } = &resource.subject else {
            return Err(ResourceProviderFailure::UnsupportedSubject);
        };
        context
            .action_state(&self.focus, id)
            .map(|state| state.map(|state| state.value))
            .map_err(|_| ResourceProviderFailure::Integrity)
    }
}

#[derive(Debug)]
struct ToyGuardedEffectProvider {
    guarded: StateFamilyHandle<ObjId, ToyGuardedState>,
}

impl EffectProvider for ToyGuardedEffectProvider {
    fn read(
        &self,
        context: &ResourceProviderContext<'_>,
        target: &EffectTarget,
        instance: EffectInstanceId,
    ) -> Result<Option<EffectInstance>, ResourceProviderFailure> {
        let EffectTarget::Subject {
            subject: SubjectKey::Object { id },
        } = target
        else {
            return Err(ResourceProviderFailure::UnsupportedSubject);
        };
        context
            .action_state(&self.guarded, id)
            .map(|state| {
                state
                    .filter(|state| state.effect.id() == instance)
                    .map(|state| state.effect.clone())
            })
            .map_err(|_| ResourceProviderFailure::Integrity)
    }
}

#[derive(Debug)]
struct BraceProvider {
    focus: StateFamilyHandle<ObjId, ToyFocusState>,
    reservations: StateFamilyHandle<ReservationBookId, ReservationBook>,
    guarded: StateFamilyHandle<ObjId, ToyGuardedState>,
    brace_definition: DefinitionRef,
    guarded_definition: DefinitionRef,
}

impl BraceProvider {
    fn focus_resource(actor: ObjId) -> ResourceRef {
        ResourceRef::new(
            OwnerId::new(TOY_OWNER).expect("static owner"),
            SubjectKey::Object { id: actor },
            CanonicalId::new("focus").expect("static resource id"),
        )
    }

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

    fn reservation_error(error: &ReservationBookError) -> ActionError {
        let code = match error {
            ReservationBookError::Availability(_)
            | ReservationBookError::Resource(mush_core::ResourceValueError::OutOfBounds {
                ..
            }) => ActionErrorCode::ProviderRejected,
            ReservationBookError::RecordLimit { .. }
            | ReservationBookError::RevisionExhausted
            | ReservationBookError::Resource(
                mush_core::ResourceValueError::ArithmeticOverflow
                | mush_core::ResourceValueError::RevisionExhausted,
            ) => ActionErrorCode::BoundsExceeded,
            _ => ActionErrorCode::Integrity,
        };
        ActionError::new(code, false, "focus reservation gate refused the spend")
            .expect("static action error is bounded")
    }

    fn spend_focus(
        &self,
        actor: ObjId,
        txn: &mut ActionTxn<'_>,
    ) -> Result<(ToyFocusState, BoundedScalar), ActionError> {
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
        let resource = Self::focus_resource(actor);
        let book_id = ReservationBookId::derive(&resource).map_err(|_| {
            ActionError::new(
                ActionErrorCode::BoundsExceeded,
                false,
                "focus reservation identity exceeds canonical bounds",
            )
            .expect("static action error is bounded")
        })?;
        let (book, value) = if let Some(book) = txn
            .get(&self.reservations, &book_id)
            .map_err(|error| Self::state_error(&error))?
            .cloned()
        {
            let (book, value) = book
                .checked_apply_resource(
                    book.revision(),
                    focus.value,
                    ResourceOperation::Decrease { amount: 1 },
                )
                .map_err(|error| Self::reservation_error(&error))?;
            (Some(book), value)
        } else {
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
            (None, value)
        };
        txn.replace(&self.focus, &actor, ToyFocusState { value })
            .map_err(|error| Self::state_error(&error))?;
        if let Some(book) = book {
            txn.replace(&self.reservations, &book_id, book)
                .map_err(|error| Self::state_error(&error))?;
        }
        Ok((focus, value))
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
            .resource_snapshot(&Self::focus_resource(query.actor()))
            .map_err(|_| {
                ActionError::new(
                    ActionErrorCode::Integrity,
                    false,
                    "focus resource projection is unavailable",
                )
                .expect("static action error is bounded")
            })?;
        let guarded = engine
            .action_state()
            .get(&self.guarded, &query.actor())
            .map_err(|error| Self::state_error(&error))?;
        if !focus.is_some_and(|focus| focus.available() > 0) || guarded.is_some() {
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
        self.spend_focus(actor, txn)?;
        let effect = self.guarded_effect(invocation, turn)?;
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
        let (focus, value) = self.spend_focus(actor, txn)?;
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

#[derive(Debug)]
struct GuardedExpiryScheduleAdapter {
    jobs: StateFamilyHandle<ScheduleId, ToyScheduledExpiry>,
    guarded: StateFamilyHandle<ObjId, ToyGuardedState>,
}

impl ScheduleActionAdapter for GuardedExpiryScheduleAdapter {
    fn plan(
        &self,
        action: &DefinitionRef,
        advance: &ScheduleAdvance,
        engine: &Engine,
    ) -> Result<ScheduleActionDraft, ActionError> {
        let job = engine
            .action_state()
            .get(&self.jobs, &advance.schedule())
            .map_err(|error| BraceProvider::state_error(&error))?
            .ok_or_else(|| {
                ActionError::new(
                    ActionErrorCode::Stale,
                    false,
                    "scheduled expiry addressing is unavailable",
                )
                .expect("static schedule error is bounded")
            })?;
        let guarded = engine
            .action_state()
            .get(&self.guarded, &job.actor)
            .map_err(|error| BraceProvider::state_error(&error))?
            .filter(|guarded| guarded.effect.id() == job.effect)
            .ok_or_else(|| {
                ActionError::new(
                    ActionErrorCode::Stale,
                    false,
                    "scheduled guarded effect is no longer current",
                )
                .expect("static schedule error is bounded")
            })?;
        let definitions = engine.registry.definitions().ok_or_else(|| {
            ActionError::new(
                ActionErrorCode::Integrity,
                false,
                "toy definition registry is unavailable",
            )
            .expect("static schedule error is bounded")
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
                .expect("static schedule error is bounded")
            })?;
        guard_builder
            .add_definition(action.clone(), definitions)
            .map_err(|_| {
                ActionError::new(
                    ActionErrorCode::Integrity,
                    false,
                    "expiry definition guard is unavailable",
                )
                .expect("static schedule error is bounded")
            })?;
        Ok(ScheduleActionDraft {
            actor: job.actor,
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters: json!({
                "effect": job.effect,
                "now": guarded.expires_at(),
            }),
            guards: guard_builder.finish().map_err(|_| {
                ActionError::new(
                    ActionErrorCode::Integrity,
                    false,
                    "expiry schedule guards are invalid",
                )
                .expect("static schedule error is bounded")
            })?,
        })
    }
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

/// Exact simultaneous window used by the K4 durability fixture.
#[must_use]
pub fn toy_action_window() -> ActionWindow {
    toy_action_window_with_secret(1)
}

fn toy_action_window_with_secret(resolution_secret_version: u32) -> ActionWindow {
    let owner = OwnerId::new(TOY_OWNER).expect("static owner");
    let local = CanonicalId::new("round_7").expect("static window id");
    ActionWindow::new(ActionWindowDraft {
        id: ActionWindowId::derive(&owner, &local).expect("static window identity"),
        clock: toy_scene_turn(0).clock,
        opens: toy_scene_turn(10),
        closes: toy_scene_turn(20),
        generation: 7,
        mode: WindowMode::CollectSimultaneous,
        capacity: 4,
        restart: WindowRestartPolicy::CloseIfElapsed,
        timeout: WindowTimeoutPolicy::Close,
        max_catch_up: 3,
        resolution_secret_version: Some(resolution_secret_version),
    })
    .expect("static toy window is valid")
}

#[derive(Debug)]
struct ActionWindowProvider {
    windows: StateFamilyHandle<ActionWindowId, ActionWindowRecord>,
    action: DefinitionRef,
}

impl ActionWindowProvider {
    fn invalid(message: &'static str) -> ActionError {
        ActionError::new(ActionErrorCode::InvalidRequest, false, message)
            .expect("static window error is bounded")
    }

    fn rejected(error: ActionWindowError) -> ActionError {
        let code = match error {
            ActionWindowError::StaleRevision { .. } => ActionErrorCode::Stale,
            ActionWindowError::OperationConflict { .. } => ActionErrorCode::Conflict,
            ActionWindowError::CapacityReached { .. }
            | ActionWindowError::InputOperationLimit
            | ActionWindowError::ReceiptLimit
            | ActionWindowError::OrderKeyTooLong { .. }
            | ActionWindowError::RevisionExhausted => ActionErrorCode::BoundsExceeded,
            _ => ActionErrorCode::ProviderRejected,
        };
        ActionError::new(
            code,
            false,
            format!("window operation was refused: {error}"),
        )
        .expect("bounded window error remains bounded")
    }

    fn parameters(
        invocation: &ActionInvocation,
    ) -> Result<&serde_json::Map<String, Value>, ActionError> {
        invocation
            .parameters()
            .as_object()
            .ok_or_else(|| Self::invalid("window parameters must be an object"))
    }

    fn operation(invocation: &ActionInvocation) -> Result<&str, ActionError> {
        Self::parameters(invocation)?
            .get("operation")
            .and_then(Value::as_str)
            .ok_or_else(|| Self::invalid("window operation is required"))
    }

    fn require_fields(
        parameters: &serde_json::Map<String, Value>,
        fields: &[&str],
    ) -> Result<(), ActionError> {
        if parameters.len() == fields.len()
            && parameters
                .keys()
                .all(|candidate| fields.contains(&candidate.as_str()))
        {
            Ok(())
        } else {
            Err(Self::invalid("window operation has unexpected parameters"))
        }
    }

    fn expected_revision(parameters: &serde_json::Map<String, Value>) -> Result<u64, ActionError> {
        parameters
            .get("expected_revision")
            .and_then(Value::as_u64)
            .filter(|revision| *revision > 0)
            .ok_or_else(|| Self::invalid("window expected_revision must be positive"))
    }

    fn now(parameters: &serde_json::Map<String, Value>) -> Result<ClockStamp, ActionError> {
        parameters
            .get("now")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(|| Self::invalid("window now must be a typed clock stamp"))
    }

    fn load(
        &self,
        txn: &mut ActionTxn<'_>,
        id: &ActionWindowId,
    ) -> Result<ActionWindowRecord, ActionError> {
        txn.get(&self.windows, id)
            .map_err(|error| BraceProvider::state_error(&error))?
            .cloned()
            .ok_or_else(|| {
                ActionError::new(
                    ActionErrorCode::Stale,
                    false,
                    "action window no longer exists",
                )
                .expect("static window error is bounded")
            })
    }

    fn outcome(&self, data: Value) -> Result<RulesValue, ActionError> {
        RulesValue::new(self.action.clone(), data).map_err(|_| {
            ActionError::new(
                ActionErrorCode::BoundsExceeded,
                false,
                "window outcome exceeds canonical bounds",
            )
            .expect("static window error is bounded")
        })
    }
}

impl ActionProvider for ActionWindowProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
            return Err(Self::invalid("window operations are actor-self scoped"));
        }
        match Self::operation(invocation)? {
            "open" | "submit" | "close" | "resolve" => Ok(()),
            _ => Err(Self::invalid("unknown window operation")),
        }
    }

    fn prepare_invocation(
        &self,
        _invocation: &ActionInvocation,
        _txn: &mut ActionTxn<'_>,
    ) -> Result<(), ActionError> {
        Err(ActionError::new(
            ActionErrorCode::Unavailable,
            false,
            "window operations require the authoritative action coordinator",
        )
        .expect("static window error is bounded"))
    }

    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate_invocation(invocation)?;
        let parameters = Self::parameters(invocation)?;
        let operation = Self::operation(invocation)?;
        let definition = toy_action_window();
        let window_id = definition.id();

        match operation {
            "open" => {
                Self::require_fields(parameters, &["operation"])?;
                let definition = toy_action_window_with_secret(
                    context.current_resolution_secret_version().map_err(|_| {
                        ActionError::new(
                            ActionErrorCode::Unavailable,
                            false,
                            "authoritative resolution keys are unavailable",
                        )
                        .expect("static window error is bounded")
                    })?,
                );
                let record = ActionWindowRecord::open(definition);
                txn.insert(&self.windows, &window_id, record.clone())
                    .map_err(|error| BraceProvider::state_error(&error))?;
                // Exercise the kernel's principal-as-root-witness default
                // before it constructs the root semantic event.
                Ok(AuthoritativeActionOutput::new(self.outcome(json!({
                    "event": "window_opened",
                    "window": window_id,
                    "revision": record.revision(),
                }))?))
            }
            "submit" => {
                Self::require_fields(
                    parameters,
                    &["operation", "expected_revision", "now", "order"],
                )?;
                let expected_revision = Self::expected_revision(parameters)?;
                let now = Self::now(parameters)?;
                let order = parameters
                    .get("order")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| Self::invalid("window order must be an integer"))?;
                let record = self.load(txn, &window_id)?;
                let intent = WindowIntent::new(
                    window_id,
                    definition.generation(),
                    SubjectKey::Object {
                        id: invocation.actor(),
                    },
                    invocation.clone(),
                    WindowOrderKey::new(vec![order]).map_err(Self::rejected)?,
                )
                .map_err(Self::rejected)?;
                let transition = record
                    .checked_submit(expected_revision, &now, intent)
                    .map_err(Self::rejected)?;
                let result = transition.receipt().result().clone();
                let record = transition.into_record();
                txn.replace(&self.windows, &window_id, record.clone())
                    .map_err(|error| BraceProvider::state_error(&error))?;
                Ok(AuthoritativeActionOutput::new(self.outcome(json!({
                    "event": "window_intent_submitted",
                    "window": window_id,
                    "revision": record.revision(),
                    "result": result,
                }))?)
                .with_eligible_witnesses(vec![invocation.principal().clone()]))
            }
            "close" => {
                Self::require_fields(parameters, &["operation", "expected_revision", "now"])?;
                let expected_revision = Self::expected_revision(parameters)?;
                let now = Self::now(parameters)?;
                let record = self.load(txn, &window_id)?;
                let transition = record
                    .checked_close_due(expected_revision, invocation.request(), &now)
                    .map_err(Self::rejected)?;
                let record = transition.into_record();
                txn.replace(&self.windows, &window_id, record.clone())
                    .map_err(|error| BraceProvider::state_error(&error))?;
                Ok(AuthoritativeActionOutput::new(self.outcome(json!({
                    "event": "window_closed",
                    "window": window_id,
                    "intent_count": record.intents().len(),
                    "revision": record.revision(),
                }))?)
                .with_eligible_witnesses(vec![invocation.principal().clone()]))
            }
            "resolve" => {
                Self::require_fields(parameters, &["operation", "expected_revision", "now"])?;
                let expected_revision = Self::expected_revision(parameters)?;
                let now = Self::now(parameters)?;
                let record = self.load(txn, &window_id)?;
                let actors: Vec<_> = record
                    .resolution_order()
                    .map_err(Self::rejected)?
                    .into_iter()
                    .map(|intent| intent.invocation().actor().0)
                    .collect();
                let inputs_hash = CanonicalHash::for_canonical_value(
                    "toy_window_resolution_inputs.v1",
                    1,
                    &json!({ "record": record, "actors": actors }),
                    256 * 1024,
                )
                .map_err(|_| {
                    ActionError::new(
                        ActionErrorCode::BoundsExceeded,
                        false,
                        "window resolution inputs exceed canonical bounds",
                    )
                    .expect("static window error is bounded")
                })?;
                let consumed = record
                    .checked_consume(
                        expected_revision,
                        invocation.request(),
                        context.commit_token(),
                        &now,
                    )
                    .map_err(Self::rejected)?
                    .into_record();
                txn.replace(&self.windows, &window_id, consumed.clone())
                    .map_err(|error| BraceProvider::state_error(&error))?;

                let secret_version =
                    record.window().resolution_secret_version().ok_or_else(|| {
                        ActionError::new(
                            ActionErrorCode::Integrity,
                            false,
                            "random window has no pinned resolution secret",
                        )
                        .expect("static window error is bounded")
                    })?;
                let mut rng = context
                    .start_resolution_at(
                        secret_version,
                        &CanonicalId::new("toy_window_resolve").expect("static resolution domain"),
                        inputs_hash,
                    )
                    .map_err(|_| {
                        ActionError::new(
                            ActionErrorCode::Unavailable,
                            false,
                            "authoritative resolution RNG is unavailable",
                        )
                        .expect("static window error is bounded")
                    })?;
                let tiebreak = rng
                    .draw_bounded_u64(
                        &CanonicalId::new("window_tiebreak").expect("static draw label"),
                        100,
                    )
                    .map_err(|_| {
                        ActionError::new(
                            ActionErrorCode::BoundsExceeded,
                            false,
                            "window resolution draw was refused",
                        )
                        .expect("static window error is bounded")
                    })?;
                let outcome = self.outcome(json!({
                    "event": "window_resolved",
                    "window": window_id,
                    "order": actors,
                    "revision": consumed.revision(),
                    "tiebreak": tiebreak,
                }))?;
                let resolution = rng
                    .into_prepared(
                        vec![self.action.clone()],
                        self.outcome(json!({ "intent_count": consumed.intents().len() }))?,
                        outcome.clone(),
                    )
                    .map_err(|_| {
                        ActionError::new(
                            ActionErrorCode::Integrity,
                            false,
                            "window resolution transcript is invalid",
                        )
                        .expect("static window error is bounded")
                    })?;
                Ok(AuthoritativeActionOutput::new(outcome)
                    .with_resolution(resolution)
                    .with_eligible_witnesses(vec![invocation.principal().clone()]))
            }
            _ => unreachable!("validate_invocation rejects unknown operations"),
        }
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
        let reservations = registrar.state_family(
            reservation_book_state_family_descriptor(
                StateFamilyId::new("resource_reservations").expect("static family id"),
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            ReservationBookStateCodec,
            ReservationBookStateCodec,
            ReservationBookStateCodec,
            ReservationBookStateCodec,
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
        let windows = registrar.state_family(
            action_window_state_family_descriptor(
                StateFamilyId::new("action_windows").expect("static family id"),
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            ActionWindowStateCodec,
            ActionWindowStateCodec,
            ActionWindowStateCodec,
            ActionWindowStateCodec,
        )?;
        let schedules = registrar.state_family(
            durable_schedule_state_family_descriptor(
                StateFamilyId::new("schedules").expect("static family id"),
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            DurableScheduleStateCodec,
            DurableScheduleStateCodec,
            DurableScheduleStateCodec,
            DurableScheduleStateCodec,
        )?;
        let scheduled_expiries = registrar.state_family(
            StateFamilyDescriptor::new(
                StateFamilyId::new("scheduled_expiries").expect("static family id"),
                CanonicalId::new("durable_schedule_id.v1").expect("static codec id"),
                CanonicalId::new("toy.scheduled_expiry.v1").expect("static schema id"),
                1,
                CanonicalId::new("toy.scheduled_expiry.decode_json.v1").expect("static decoder id"),
                CanonicalId::new("toy.scheduled_expiry.shape.v1").expect("static validator id"),
                CanonicalId::new("toy.scheduled_expiry.json.v1").expect("static serializer id"),
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            |key: &ScheduleId| Ok(key.canonical_hash().to_hex()),
            |value: &Value| {
                serde_json::from_value(value.clone()).map_err(|error| error.to_string())
            },
            |_: &ToyScheduledExpiry| Ok::<(), String>(()),
            |value: &ToyScheduledExpiry| {
                serde_json::to_value(value).map_err(|error| error.to_string())
            },
        )?;
        let families = ToyStateFamilies {
            focus,
            reservations,
            guarded,
            windows,
            schedules,
            scheduled_expiries,
        };
        registrar.reservable_resource_provider(
            actual.focus.clone(),
            families.reservations.clone(),
            ToyFocusResourceProvider {
                focus: families.focus.clone(),
            },
        )?;
        registrar.effect_provider(
            actual.guarded.clone(),
            ToyGuardedEffectProvider {
                guarded: families.guarded.clone(),
            },
        )?;
        registrar.action_provider(
            actual.brace.clone(),
            BraceProvider {
                focus: families.focus.clone(),
                reservations: families.reservations.clone(),
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
        registrar.schedule_source(
            CanonicalId::new("guarded_expiry").expect("static schedule source id"),
            actual.guarded_expire.clone(),
            families.schedules.clone(),
            GuardedExpiryScheduleAdapter {
                jobs: families.scheduled_expiries.clone(),
                guarded: families.guarded.clone(),
            },
        )?;
        registrar.action_provider(
            actual.window_action.clone(),
            ActionWindowProvider {
                windows: families.windows.clone(),
                action: actual.window_action.clone(),
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

/// Attach an empty authoritative reservation book to an existing toy focus
/// resource. Fixture setup uses the typed family path; later spends must keep
/// the book and scalar revisions synchronized in one transaction.
pub fn install_toy_reservation_book(
    engine: &mut Engine,
    families: &ToyStateFamilies,
    actor: ObjId,
) -> ReservationBookId {
    let focus = engine
        .action_state()
        .get(&families.focus, &actor)
        .expect("toy focus family read")
        .expect("toy focus must exist before its reservation book")
        .clone();
    let resource = ResourceRef::new(
        OwnerId::new(TOY_OWNER).expect("static owner"),
        SubjectKey::Object { id: actor },
        CanonicalId::new("focus").expect("static resource id"),
    );
    let book = ReservationBook::new(resource, focus.value).expect("toy reservation book");
    let id = book.id();
    let prepared = {
        let mut txn = engine
            .action_transaction(&families.reservations)
            .expect("toy reservation transaction");
        txn.insert(&families.reservations, &id, book)
            .expect("valid toy reservation book");
        txn.prepare().expect("reservation bootstrap prepares")
    };
    let (_, installation) = prepared.into_parts();
    engine.install_prepared_action(installation);
    id
}

/// Install one valid durable schedule through the typed family used by the
/// composition-frozen toy schedule source.
pub fn install_toy_schedule(
    engine: &mut Engine,
    families: &ToyStateFamilies,
    schedule: DurableSchedule,
) {
    let id = schedule.id();
    let prepared = {
        let mut txn = engine
            .action_transaction(&families.schedules)
            .expect("toy schedule transaction");
        txn.insert(&families.schedules, &id, schedule)
            .expect("valid schedule bootstrap");
        txn.prepare().expect("schedule bootstrap prepares")
    };
    let (_, installation) = prepared.into_parts();
    engine.install_prepared_action(installation);
}

/// Atomically install a toy schedule and its provider-specific addressing.
pub fn install_toy_scheduled_expiry(
    engine: &mut Engine,
    families: &ToyStateFamilies,
    schedule: DurableSchedule,
    actor: ObjId,
    effect: EffectInstanceId,
) {
    let id = schedule.id();
    let prepared = {
        let mut txn = engine
            .action_transaction(&families.schedules)
            .expect("toy schedule transaction");
        txn.insert(&families.schedules, &id, schedule)
            .expect("valid schedule bootstrap");
        txn.insert(
            &families.scheduled_expiries,
            &id,
            ToyScheduledExpiry { actor, effect },
        )
        .expect("valid scheduled expiry addressing");
        txn.prepare().expect("scheduled expiry bootstrap prepares")
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

/// Canonical invocation for one K4 action-window lifecycle operation.
pub fn toy_window_invocation(
    actor: ObjId,
    request: RequestUid,
    parameters: Value,
) -> ActionInvocation {
    let definitions = toy_definitions();
    let owner = OwnerId::new(TOY_OWNER).expect("static owner");
    let principal = PrincipalId::Object { id: actor };
    let idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner,
            namespace: CanonicalId::new("window_operation").expect("static namespace"),
        },
        0,
        IdempotencyKey::Request(request),
    );
    let offered_action = ActionOfferId(
        CanonicalHash::for_canonical_value(
            "toy.offer.v1",
            1,
            &json!({ "action": definitions.window_action }),
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
        parameters,
        idempotency,
    })
    .expect("toy window invocation is canonical")
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
    fn resource_and_effect_adapters_expose_exact_owner_truth() {
        let (mut world, families) = toy_world_with_state();
        bootstrap_focus(&mut world, &families, mush_core::GOD, 3);
        bootstrap_guarded(&mut world, &families, mush_core::GOD, 9);

        let definitions = toy_definitions();
        let resource = ResourceRef::new(
            OwnerId::new(TOY_OWNER).unwrap(),
            SubjectKey::Object { id: mush_core::GOD },
            CanonicalId::new("focus").unwrap(),
        );
        let snapshot = world
            .engine()
            .resource_snapshot(&resource)
            .expect("resource provider read succeeds")
            .expect("focus exists");
        assert_eq!(snapshot.definition(), &definitions.focus);
        assert_eq!(snapshot.resource(), &resource);
        assert_eq!(snapshot.value().current(), 3);
        assert_eq!(snapshot.available(), 3);
        assert!(snapshot.is_reservable());
        assert!(snapshot.reservations().is_none());

        let guarded = world
            .engine()
            .action_state()
            .get(&families.guarded, &mush_core::GOD)
            .unwrap()
            .unwrap();
        let effect = world
            .engine()
            .effect_instance(
                &definitions.guarded,
                guarded.effect.target(),
                guarded.effect.id(),
            )
            .expect("effect provider read succeeds")
            .expect("guarded exists");
        assert_eq!(effect, guarded.effect);

        let effect_target = effect.target().clone();
        let effect_id = effect.id();
        world
            .registry()
            .freeze_mechanics_owner(
                &OwnerId::new(TOY_OWNER).unwrap(),
                mush_core::IncidentId(77),
                mush_core::RecoveryReason::IntegrityViolation {
                    code: CanonicalId::new("provider_read_test").unwrap(),
                },
            )
            .unwrap();
        assert!(matches!(
            world.engine().resource_snapshot(&resource),
            Err(mush_core::ResourceReadError::OwnerNotReady(_))
        ));
        assert!(matches!(
            world
                .engine()
                .effect_instance(&definitions.guarded, &effect_target, effect_id,),
            Err(mush_core::EffectReadError::OwnerNotReady(_))
        ));
    }

    #[test]
    fn reservation_lifecycle_is_bounded_atomic_and_restart_safe() {
        let (mut world, families) = toy_world_with_state();
        bootstrap_focus(&mut world, &families, mush_core::GOD, 3);
        let _bootstrap = world.drain_write_batch();

        let resource = ResourceRef::new(
            OwnerId::new(TOY_OWNER).unwrap(),
            SubjectKey::Object { id: mush_core::GOD },
            CanonicalId::new("focus").unwrap(),
        );
        let focus = world
            .engine()
            .action_state()
            .get(&families.focus, &mush_core::GOD)
            .unwrap()
            .unwrap()
            .clone();
        let mut book = ReservationBook::new(resource.clone(), focus.value).unwrap();
        let make_hold = |byte: u8, expires: u64| {
            let holder = ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: PrincipalId::Object { id: mush_core::GOD },
                    owner: OwnerId::new(TOY_OWNER).unwrap(),
                    namespace: CanonicalId::new("reserve_focus").unwrap(),
                },
                0,
                IdempotencyKey::Request(RequestUid::from_bytes([byte; 16])),
            );
            let id = ReservationId::derive(&holder, &resource).unwrap();
            (
                id,
                ReservationRecord::held(id, resource.clone(), holder, 1, toy_scene_turn(expires))
                    .unwrap(),
            )
        };
        let (consumed_id, consumed) = make_hold(1, 20);
        let (released_id, released) = make_hold(2, 20);
        let (expired_id, expired) = make_hold(3, 12);
        book = book
            .checked_hold(0, focus.value, consumed, &toy_scene_turn(10))
            .unwrap();
        book = book
            .checked_hold(1, focus.value, released, &toy_scene_turn(10))
            .unwrap();
        book = book
            .checked_hold(2, focus.value, expired, &toy_scene_turn(10))
            .unwrap();
        assert_eq!(book.checked_available(focus.value).unwrap(), 0);

        let (next_book, focus_value) = book
            .checked_consume(
                3,
                focus.value,
                consumed_id,
                1,
                &toy_scene_turn(11),
                CommitToken::from_bytes([0x51; 16]),
            )
            .unwrap();
        book = next_book
            .checked_release(4, focus_value, released_id, 1)
            .unwrap()
            .checked_expire(5, focus_value, expired_id, 1, &toy_scene_turn(12))
            .unwrap();
        assert_eq!(book.checked_available(focus_value).unwrap(), 2);

        let prepared = {
            let mut txn = world
                .engine_mut()
                .action_transaction(&families.focus)
                .unwrap();
            txn.replace(
                &families.focus,
                &mush_core::GOD,
                ToyFocusState { value: focus_value },
            )
            .unwrap();
            txn.insert(&families.reservations, &book.id(), book.clone())
                .unwrap();
            txn.prepare().unwrap()
        };
        assert_eq!(prepared.operation_count(), 2);
        let (module_ops, installation) = prepared.into_parts();
        world.engine_mut().install_prepared_action(installation);
        let boot_records = module_ops
            .into_iter()
            .filter_map(|operation| match operation.op {
                mush_core::ModuleWrite::JsonSet { key, value }
                    if key.starts_with("action-state/") =>
                {
                    Some(mush_core::ActionStateBootRecord::new(
                        operation.ns,
                        key,
                        value,
                    ))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(boot_records.len(), 2);

        let (mut restarted, restarted_families) = toy_world_with_state();
        restarted
            .engine_mut()
            .hydrate_action_state(boot_records)
            .unwrap();
        let snapshot = restarted
            .engine()
            .resource_snapshot(&resource)
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.value().current(), 2);
        assert_eq!(snapshot.value().revision(), 1);
        assert_eq!(snapshot.available(), 2);
        assert_eq!(snapshot.reservations(), Some(&book));
        assert_eq!(
            restarted
                .engine()
                .action_state()
                .get(&restarted_families.reservations, &book.id())
                .unwrap(),
            Some(&book)
        );
    }

    #[test]
    fn reservable_focus_cannot_be_spent_around_a_live_hold() {
        let (mut world, families) = toy_world_with_state();
        bootstrap_focus(&mut world, &families, mush_core::GOD, 3);
        let resource = ResourceRef::new(
            OwnerId::new(TOY_OWNER).unwrap(),
            SubjectKey::Object { id: mush_core::GOD },
            CanonicalId::new("focus").unwrap(),
        );
        let focus = world
            .engine()
            .action_state()
            .get(&families.focus, &mush_core::GOD)
            .unwrap()
            .unwrap()
            .clone();
        let holder = ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: PrincipalId::Object { id: mush_core::GOD },
                owner: OwnerId::new(TOY_OWNER).unwrap(),
                namespace: CanonicalId::new("reserve_focus").unwrap(),
            },
            0,
            IdempotencyKey::Request(RequestUid::from_bytes([0x62; 16])),
        );
        let reservation = ReservationId::derive(&holder, &resource).unwrap();
        let book = ReservationBook::new(resource.clone(), focus.value)
            .unwrap()
            .checked_hold(
                0,
                focus.value,
                ReservationRecord::held(
                    reservation,
                    resource.clone(),
                    holder,
                    3,
                    toy_scene_turn(20),
                )
                .unwrap(),
                &toy_scene_turn(10),
            )
            .unwrap();
        let installation = {
            let mut txn = world
                .engine_mut()
                .action_transaction(&families.reservations)
                .unwrap();
            txn.insert(&families.reservations, &book.id(), book.clone())
                .unwrap();
            let (_, installation) = txn.prepare().unwrap().into_parts();
            installation
        };
        world.engine_mut().install_prepared_action(installation);
        assert_eq!(
            world
                .engine()
                .resource_snapshot(&resource)
                .unwrap()
                .unwrap()
                .available(),
            0
        );

        let invocation =
            toy_brace_invocation(mush_core::GOD, 11, RequestUid::from_bytes([0x63; 16]));
        let error = world
            .engine_mut()
            .prepare_registered_action(&toy_definitions().brace, &invocation)
            .expect_err("a live hold consumes all spendable focus");
        assert!(matches!(
            error,
            ActionProviderPrepareError::Provider { source, .. }
                if source.code() == ActionErrorCode::ProviderRejected
        ));
        assert!(!world.engine().action_state().has_prepared());
        assert_eq!(
            world
                .engine()
                .action_state()
                .get(&families.focus, &mush_core::GOD)
                .unwrap(),
            Some(&focus)
        );
        assert_eq!(
            world
                .engine()
                .action_state()
                .get(&families.reservations, &book.id())
                .unwrap(),
            Some(&book)
        );
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
            .prepare_registered_authoritative_action(
                &toy_definitions().brace,
                &invocation,
                toy_prepare_token(),
            )
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
            world.engine_mut().prepare_registered_authoritative_action(
                &toy_definitions().brace,
                &invocation,
                toy_prepare_token(),
            ),
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
            world.engine_mut().prepare_registered_authoritative_action(
                &toy_definitions().brace,
                &invocation,
                toy_prepare_token(),
            ),
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
            .prepare_registered_authoritative_action(&action, &invocation, toy_prepare_token())
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
                .prepare_registered_authoritative_action(&action, &invocation, toy_prepare_token())
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
            .prepare_registered_authoritative_action(
                &toy_definitions().brace,
                &brace,
                toy_prepare_token(),
            )
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
                &early,
                toy_prepare_token(),
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
                &stale_clock,
                toy_prepare_token(),
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
            .prepare_registered_authoritative_action(
                &toy_definitions().guarded_expire,
                &due,
                toy_prepare_token(),
            )
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

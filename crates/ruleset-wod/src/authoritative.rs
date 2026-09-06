//! K8.4 production migration for the Willpower-backed Storyteller roll.
//!
//! The legacy `+roll` surface remains for non-mutating compatibility rolls,
//! but a roll which spends Willpower is owned here.  Its resource post-image,
//! deterministic dice transcript, semantic outcome, and projected receipt are
//! prepared by the same authoritative coordinator used by tactics.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionPresentation, ActionProvider,
    ActionStateError, ActionTxn, AuthoritativeActionOutput, CanonicalHash, CanonicalId,
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionValidationContext,
    Engine, LegalActionQuery, ModError, ObjId, PerceivedRef, PrincipalId, ProjectedParameter,
    ProjectedParameterKind, ProjectedParameterSchema, ProjectedTargetSchema, ProviderOfferDraft,
    Registrar, RevisionSet, RulesValue, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId,
    SubjectKey, perm,
};
use mush_dice::owod20::{Owod20Opts, Owod20Outcome, Owod20Pool};
use mush_dice::{PoolMechanic, RollLimits, with_resolution_stream};
use mush_ruleset::{SheetDoc, SheetStore};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::splats::Dispatch;

pub(crate) const ACTION_ID: &str = "willpower_roll";
pub(crate) const OUTCOME_ID: &str = "willpower_roll_result";
pub(crate) const SET_ACTION_ID: &str = "willpower_set";
pub(crate) const RESOURCE_OUTCOME_ID: &str = "willpower_resource_result";
pub(crate) const STATE_FAMILY: &str = "roll_state";

/// H2c: the registered action handles an NPC grounding needs from a `&World`
/// (`WodRuleset::projected_ability`), published as module state.
#[derive(Clone, Debug)]
pub struct WodActionHandles {
    /// `willpower_roll`.
    pub willpower_roll: DefinitionRef,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WodRollState {
    pub(crate) willpower_current: i64,
    pub(crate) roll_sequence: u64,
}

#[derive(Clone)]
struct ExpectedDefinition {
    payload: Value,
    references: Vec<DefinitionRef>,
    current: bool,
}

fn exact_validator(
    descriptors: &[DefinitionDescriptor],
) -> impl Fn(&DefinitionValidationContext<'_>) -> Result<(), String> + Send + Sync + 'static {
    let expected: BTreeMap<_, _> = descriptors
        .iter()
        .map(|descriptor| {
            let mut references = descriptor.references().to_vec();
            references.sort();
            (
                (descriptor.id().clone(), descriptor.version()),
                ExpectedDefinition {
                    payload: descriptor.payload().clone(),
                    references,
                    current: descriptor.is_current(),
                },
            )
        })
        .collect();
    move |context| {
        let definition = context.definition();
        let key = (
            definition.reference().key().id.clone(),
            definition.reference().version(),
        );
        let Some(shape) = expected.get(&key) else {
            return Err("unknown WoD authoritative definition".to_owned());
        };
        if definition.payload() != &shape.payload
            || definition.references() != shape.references
            || definition.is_current() != shape.current
        {
            return Err("WoD authoritative definition shape mismatch".to_owned());
        }
        Ok(())
    }
}

fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(|error| ModError::msg(error.to_string()))?,
        DefinitionId::new(id).map_err(|error| ModError::msg(error.to_string()))?,
        1,
        payload,
    )
    .map(DefinitionDescriptor::as_current)
    .map_err(|error| ModError::msg(error.to_string()))
}

struct WillpowerRollProvider {
    dispatch: Arc<Dispatch>,
    action: DefinitionRef,
    outcome: DefinitionRef,
    state: StateFamilyHandle<ObjId, WodRollState>,
}

struct WillpowerSetProvider {
    dispatch: Arc<Dispatch>,
    action: DefinitionRef,
    outcome: DefinitionRef,
    state: StateFamilyHandle<ObjId, WodRollState>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RollOutcome {
    actor: u32,
    pool: i32,
    roll: Owod20Outcome,
    willpower_remaining: i64,
    roll_sequence: u64,
}

struct RollRequest {
    sheet: SheetDoc,
    pool_expr: String,
    difficulty: u8,
    specialty: bool,
    modifier: i32,
    ignore_wounds: bool,
    seed_willpower: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceOutcome {
    actor: u32,
    target: u32,
    current: i64,
    roll_sequence: u64,
}

struct ResourceRequest {
    target: ObjId,
    current: i64,
    permanent: i64,
    seed_sequence: u64,
}

impl WillpowerRollProvider {
    fn error(code: ActionErrorCode, message: impl Into<String>) -> ActionError {
        ActionError::new(code, false, message).expect("bounded WoD authoritative action error")
    }

    fn state_error(_error: &ActionStateError) -> ActionError {
        Self::error(
            ActionErrorCode::Integrity,
            "WoD roll state preparation was refused",
        )
    }

    fn parse(invocation: &ActionInvocation) -> Result<RollRequest, ActionError> {
        if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
            return Err(Self::error(
                ActionErrorCode::InvalidRequest,
                "Willpower roll must be self-scoped and have no targets",
            ));
        }
        let parameters = invocation.parameters().as_object().ok_or_else(|| {
            Self::error(
                ActionErrorCode::InvalidRequest,
                "Willpower roll parameters must be an object",
            )
        })?;
        const FIELDS: [&str; 7] = [
            "difficulty",
            "ignore_wounds",
            "modifier",
            "pool",
            "seed_willpower",
            "sheet",
            "specialty",
        ];
        if parameters.len() < 3 || parameters.keys().any(|key| !FIELDS.contains(&key.as_str())) {
            return Err(Self::error(
                ActionErrorCode::InvalidRequest,
                "Willpower roll has unexpected parameters",
            ));
        }
        let sheet: SheetDoc =
            serde_json::from_value(parameters.get("sheet").cloned().ok_or_else(|| {
                Self::error(
                    ActionErrorCode::InvalidRequest,
                    "Willpower roll lacks its sheet",
                )
            })?)
            .map_err(|_| {
                Self::error(
                    ActionErrorCode::InvalidRequest,
                    "Willpower roll sheet is malformed",
                )
            })?;
        let pool_expr = parameters
            .get("pool")
            .and_then(Value::as_str)
            .filter(|pool| !pool.trim().is_empty())
            .ok_or_else(|| {
                Self::error(
                    ActionErrorCode::InvalidRequest,
                    "Willpower roll needs a pool",
                )
            })?
            .to_owned();
        let difficulty = parameters
            .get("difficulty")
            .and_then(Value::as_u64)
            .unwrap_or(6);
        let difficulty = u8::try_from(difficulty)
            .ok()
            .filter(|value| (2..=10).contains(value))
            .ok_or_else(|| {
                Self::error(
                    ActionErrorCode::InvalidRequest,
                    "Willpower roll difficulty must be 2 through 10",
                )
            })?;
        let modifier = parameters
            .get("modifier")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let modifier = i32::try_from(modifier)
            .ok()
            .filter(|value| (-20..=20).contains(value))
            .ok_or_else(|| {
                Self::error(
                    ActionErrorCode::InvalidRequest,
                    "Willpower roll modifier must be -20 through 20",
                )
            })?;
        let seed_willpower = parameters
            .get("seed_willpower")
            .and_then(Value::as_i64)
            .filter(|value| (0..=10).contains(value))
            .ok_or_else(|| {
                Self::error(
                    ActionErrorCode::InvalidRequest,
                    "Willpower roll resource seed is invalid",
                )
            })?;
        Ok(RollRequest {
            sheet,
            pool_expr,
            difficulty,
            specialty: parameters
                .get("specialty")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            modifier,
            ignore_wounds: parameters
                .get("ignore_wounds")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            seed_willpower,
        })
    }
}

impl ActionProvider for WillpowerRollProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        Self::parse(invocation).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        let SubjectKey::Object { id: subject } = query.truth_subject() else {
            return Ok(None);
        };
        if *subject != query.actor() || query.projected_subject() != &PerceivedRef::SelfActor {
            return Ok(None);
        }
        let Some(sheet) = SheetStore::new(self.dispatch.schemas.default_schema())
            .get(&engine.world, query.actor())
        else {
            return Ok(None);
        };
        let schema = self.dispatch.schemas.schema_for_doc(&sheet);
        if mush_ruleset::validate(&schema, &sheet.data).is_err() {
            return Err(Self::error(
                ActionErrorCode::Integrity,
                "WoD sheet failed authoritative validation",
            ));
        }
        let permanent = sheet.number("willpower").unwrap_or(0).max(0);
        let seed = sheet
            .number("willpower_temp")
            .unwrap_or(0)
            .clamp(0, permanent);
        let current = engine
            .action_state()
            .get(&self.state, &query.actor())
            .map_err(|error| Self::state_error(&error))?
            .map_or(seed, |state| state.willpower_current.min(permanent));
        if current == 0 {
            return Ok(None);
        }
        let definitions = engine.registry.definitions().ok_or_else(|| {
            Self::error(
                ActionErrorCode::Integrity,
                "WoD definition registry is unavailable",
            )
        })?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| {
                Self::error(
                    ActionErrorCode::Integrity,
                    "WoD guard registry is unavailable",
                )
            })?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| {
                Self::error(
                    ActionErrorCode::Integrity,
                    "WoD action definition guard is unavailable",
                )
            })?;
        let parameters = ProjectedParameterSchema::new(vec![
            ProjectedParameter {
                id: CanonicalId::new("difficulty").expect("static parameter id"),
                required: false,
                kind: ProjectedParameterKind::Integer {
                    minimum: 2,
                    maximum: 10,
                },
            },
            ProjectedParameter {
                id: CanonicalId::new("ignore_wounds").expect("static parameter id"),
                required: false,
                kind: ProjectedParameterKind::Boolean,
            },
            ProjectedParameter {
                id: CanonicalId::new("modifier").expect("static parameter id"),
                required: false,
                kind: ProjectedParameterKind::Integer {
                    minimum: -20,
                    maximum: 20,
                },
            },
            ProjectedParameter {
                id: CanonicalId::new("pool").expect("static parameter id"),
                required: true,
                kind: ProjectedParameterKind::Text { maximum_bytes: 128 },
            },
            ProjectedParameter {
                id: CanonicalId::new("specialty").expect("static parameter id"),
                required: false,
                kind: ProjectedParameterKind::Boolean,
            },
        ])
        .map_err(|_| {
            Self::error(
                ActionErrorCode::Integrity,
                "WoD roll parameter schema is invalid",
            )
        })?;
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({ "sheet": sheet, "seed_willpower": current }),
            parameters,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: ActionPresentation::new(
                "willpower-roll",
                "Willpower Roll",
                "Spend one temporary Willpower and make a Storyteller roll.",
            )
            .map_err(|_| {
                Self::error(
                    ActionErrorCode::Integrity,
                    "WoD roll presentation is invalid",
                )
            })?,
            guards: guards.finish().map_err(|_| {
                Self::error(ActionErrorCode::Integrity, "WoD roll guards are invalid")
            })?,
        }))
    }

    fn project_receipt(
        &self,
        action: &DefinitionRef,
        outcome: &RulesValue,
        viewer: &PrincipalId,
    ) -> Result<Option<mush_proto::ProjectedActionResult>, ActionError> {
        if action != &self.action || outcome.schema() != &self.outcome {
            return Err(Self::error(
                ActionErrorCode::Integrity,
                "WoD roll receipt carries the wrong durable schema",
            ));
        }
        let decoded: RollOutcome =
            serde_json::from_value(outcome.data().clone()).map_err(|_| {
                Self::error(
                    ActionErrorCode::Integrity,
                    "WoD roll receipt outcome is malformed",
                )
            })?;
        if viewer
            != &(PrincipalId::Object {
                id: ObjId(decoded.actor),
            })
            || decoded.pool < 1
            || decoded.roll_sequence == 0
            || !decoded.roll.willpower_success
        {
            return Err(Self::error(
                ActionErrorCode::Integrity,
                "WoD roll receipt disagrees with its eligible viewer",
            ));
        }
        mush_proto::ProjectedActionResult::new(
            ACTION_ID,
            decoded.roll.summary(),
            json!({
                "pool": decoded.pool,
                "roll": decoded.roll,
                "willpower_remaining": decoded.willpower_remaining,
                "roll_sequence": decoded.roll_sequence,
            }),
        )
        .map(Some)
        .map_err(|_| {
            Self::error(
                ActionErrorCode::BoundsExceeded,
                "WoD roll receipt exceeds public bounds",
            )
        })
    }

    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut mush_core::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let request = Self::parse(invocation)?;
        let schema = self.dispatch.schemas.schema_for_doc(&request.sheet);
        mush_ruleset::validate(&schema, &request.sheet.data).map_err(|_| {
            Self::error(
                ActionErrorCode::Integrity,
                "WoD roll sheet failed schema validation",
            )
        })?;
        let resolved = crate::dice::resolve_sheet_pool(&schema, &request.sheet, &request.pool_expr)
            .map_err(|_| {
                Self::error(
                    ActionErrorCode::ProviderRejected,
                    "That pool does not resolve against the current sheet",
                )
            })?;
        let specialty = if request.specialty {
            if !crate::roll::has_any_specialty(&request.sheet, &resolved.trait_fields) {
                return Err(Self::error(
                    ActionErrorCode::ProviderRejected,
                    "No applicable specialty is recorded for that pool",
                ));
            }
            true
        } else {
            false
        };
        let permanent = request.sheet.number("willpower").unwrap_or(0).max(0);
        let prior = txn
            .get(&self.state, &invocation.actor())
            .map_err(|error| Self::state_error(&error))?
            .cloned();
        let current = prior
            .as_ref()
            .map_or(request.seed_willpower, |state| state.willpower_current)
            .clamp(0, permanent);
        if current == 0 {
            return Err(Self::error(
                ActionErrorCode::ProviderRejected,
                "No temporary Willpower remains",
            ));
        }
        let sequence = prior
            .as_ref()
            .map_or(Ok(1), |state| state.roll_sequence.checked_add(1).ok_or(()))
            .map_err(|()| {
                Self::error(
                    ActionErrorCode::BoundsExceeded,
                    "WoD roll sequence is exhausted",
                )
            })?;
        let next = WodRollState {
            willpower_current: current - 1,
            roll_sequence: sequence,
        };
        if prior.is_some() {
            txn.replace(&self.state, &invocation.actor(), next.clone())
        } else {
            txn.insert(&self.state, &invocation.actor(), next.clone())
        }
        .map_err(|error| Self::state_error(&error))?;

        let wound = if request.ignore_wounds {
            0
        } else {
            i32::from(SheetStore::new(schema.clone()).wound_penalty(&request.sheet, "health"))
        };
        let form = crate::splats::werewolf::current_form_pool_delta(
            &request.sheet,
            &resolved.trait_fields,
        );
        let pool = (resolved.total + request.modifier + wound + form).max(1);
        let opts = Owod20Opts {
            difficulty: request.difficulty,
            specialty,
            willpower: true,
            extra_successes: 0,
            again_tens_double: false,
        };
        let inputs_hash = CanonicalHash::for_canonical_value(
            "wod_willpower_roll_inputs.v1",
            1,
            &json!({
                "actor": invocation.actor(),
                "sheet": request.sheet,
                "pool_expr": request.pool_expr,
                "pool": pool,
                "opts": opts,
                "willpower_before": current,
                "roll_sequence": sequence,
            }),
            256 * 1024,
        )
        .map_err(|_| {
            Self::error(
                ActionErrorCode::BoundsExceeded,
                "WoD roll inputs exceed canonical bounds",
            )
        })?;
        let mut rng = context
            .start_resolution(
                &CanonicalId::new("wod_willpower_roll").expect("static resolution domain"),
                inputs_hash,
            )
            .map_err(|_| {
                Self::error(
                    ActionErrorCode::Unavailable,
                    "Authoritative resolution keys are unavailable",
                )
            })?;
        let roll = with_resolution_stream(
            &mut rng,
            &CanonicalId::new("primary_pool").expect("static draw label"),
            RollLimits::default(),
            |roll_context| Owod20Pool.roll(pool, &opts, roll_context),
        )
        .map_err(|_| {
            Self::error(
                ActionErrorCode::BoundsExceeded,
                "WoD roll exceeded its deterministic draw budget",
            )
        })?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "actor": invocation.actor().0,
                "pool": pool,
                "roll": roll,
                "willpower_remaining": next.willpower_current,
                "roll_sequence": sequence,
            }),
        )
        .map_err(|_| {
            Self::error(
                ActionErrorCode::BoundsExceeded,
                "WoD roll outcome exceeds canonical bounds",
            )
        })?;
        let resolution = rng
            .into_prepared(
                vec![self.action.clone()],
                RulesValue::new(
                    self.action.clone(),
                    json!({
                        "pool_expr": request.pool_expr,
                        "pool": pool,
                        "difficulty": request.difficulty,
                        "specialty": specialty,
                        "modifier": request.modifier,
                        "wound": wound,
                        "form": form,
                        "willpower_spent": 1,
                    }),
                )
                .map_err(|_| {
                    Self::error(
                        ActionErrorCode::BoundsExceeded,
                        "WoD roll modifiers exceed canonical bounds",
                    )
                })?,
                outcome.clone(),
            )
            .map_err(|_| {
                Self::error(ActionErrorCode::Integrity, "WoD roll transcript is invalid")
            })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_resolution(resolution)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

impl WillpowerSetProvider {
    fn parse(&self, invocation: &ActionInvocation) -> Result<ResourceRequest, ActionError> {
        if !invocation.targets().is_empty() {
            return Err(WillpowerRollProvider::error(
                ActionErrorCode::InvalidRequest,
                "Willpower resource action accepts no target list",
            ));
        }
        let parameters = invocation.parameters().as_object().ok_or_else(|| {
            WillpowerRollProvider::error(
                ActionErrorCode::InvalidRequest,
                "Willpower resource parameters must be an object",
            )
        })?;
        const FIELDS: [&str; 4] = ["current", "permanent", "seed_sequence", "target"];
        if parameters.len() != FIELDS.len()
            || parameters.keys().any(|key| !FIELDS.contains(&key.as_str()))
        {
            return Err(WillpowerRollProvider::error(
                ActionErrorCode::InvalidRequest,
                "Willpower resource action has unexpected parameters",
            ));
        }
        let target = parameters
            .get("target")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .map(ObjId)
            .ok_or_else(|| {
                WillpowerRollProvider::error(
                    ActionErrorCode::InvalidRequest,
                    "Willpower resource target is invalid",
                )
            })?;
        if !matches!(invocation.subject(), PerceivedRef::KnownObject { id, .. } if *id == target) {
            return Err(WillpowerRollProvider::error(
                ActionErrorCode::InvalidRequest,
                "Willpower resource subject disagrees with its target",
            ));
        }
        let permanent = parameters
            .get("permanent")
            .and_then(Value::as_i64)
            .filter(|value| (0..=10).contains(value))
            .ok_or_else(|| {
                WillpowerRollProvider::error(
                    ActionErrorCode::InvalidRequest,
                    "Willpower permanent rating is invalid",
                )
            })?;
        let current = parameters
            .get("current")
            .and_then(Value::as_i64)
            .filter(|value| (0..=permanent).contains(value))
            .ok_or_else(|| {
                WillpowerRollProvider::error(
                    ActionErrorCode::InvalidRequest,
                    "Temporary Willpower must be between zero and the permanent rating",
                )
            })?;
        let seed_sequence = parameters
            .get("seed_sequence")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                WillpowerRollProvider::error(
                    ActionErrorCode::InvalidRequest,
                    "Willpower roll sequence is invalid",
                )
            })?;
        Ok(ResourceRequest {
            target,
            current,
            permanent,
            seed_sequence,
        })
    }
}

impl ActionProvider for WillpowerSetProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        self.parse(invocation).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        let SubjectKey::Object { id: target } = query.truth_subject() else {
            return Ok(None);
        };
        if !perm::has_role(&engine.world, query.actor(), mush_core::Role::Royalty) {
            return Ok(None);
        }
        let Some(sheet) =
            SheetStore::new(self.dispatch.schemas.default_schema()).get(&engine.world, *target)
        else {
            return Ok(None);
        };
        let schema = self.dispatch.schemas.schema_for_doc(&sheet);
        if mush_ruleset::validate(&schema, &sheet.data).is_err() {
            return Err(WillpowerRollProvider::error(
                ActionErrorCode::Integrity,
                "WoD sheet failed authoritative validation",
            ));
        }
        let permanent = sheet.number("willpower").unwrap_or(0).clamp(0, 10);
        let seed_sequence = engine
            .action_state()
            .get(&self.state, target)
            .map_err(|error| WillpowerRollProvider::state_error(&error))?
            .map_or(0, |state| state.roll_sequence);
        let definitions = engine.registry.definitions().ok_or_else(|| {
            WillpowerRollProvider::error(
                ActionErrorCode::Integrity,
                "WoD definition registry is unavailable",
            )
        })?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| {
                WillpowerRollProvider::error(
                    ActionErrorCode::Integrity,
                    "WoD guard registry is unavailable",
                )
            })?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| {
                WillpowerRollProvider::error(
                    ActionErrorCode::Integrity,
                    "WoD resource definition guard is unavailable",
                )
            })?;
        let parameters = ProjectedParameterSchema::new(vec![ProjectedParameter {
            id: CanonicalId::new("current").expect("static parameter id"),
            required: true,
            kind: ProjectedParameterKind::Integer {
                minimum: 0,
                maximum: permanent,
            },
        }])
        .map_err(|_| {
            WillpowerRollProvider::error(
                ActionErrorCode::Integrity,
                "WoD resource parameter schema is invalid",
            )
        })?;
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({
                "target": target.0,
                "permanent": permanent,
                "seed_sequence": seed_sequence,
            }),
            parameters,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: ActionPresentation::new(
                "willpower-set",
                "Set Temporary Willpower",
                "Set a character's authoritative temporary Willpower pool.",
            )
            .map_err(|_| {
                WillpowerRollProvider::error(
                    ActionErrorCode::Integrity,
                    "WoD resource presentation is invalid",
                )
            })?,
            guards: guards.finish().map_err(|_| {
                WillpowerRollProvider::error(
                    ActionErrorCode::Integrity,
                    "WoD resource guards are invalid",
                )
            })?,
        }))
    }

    fn project_receipt(
        &self,
        action: &DefinitionRef,
        outcome: &RulesValue,
        viewer: &PrincipalId,
    ) -> Result<Option<mush_proto::ProjectedActionResult>, ActionError> {
        if action != &self.action || outcome.schema() != &self.outcome {
            return Err(WillpowerRollProvider::error(
                ActionErrorCode::Integrity,
                "WoD resource receipt carries the wrong durable schema",
            ));
        }
        let decoded: ResourceOutcome =
            serde_json::from_value(outcome.data().clone()).map_err(|_| {
                WillpowerRollProvider::error(
                    ActionErrorCode::Integrity,
                    "WoD resource receipt outcome is malformed",
                )
            })?;
        if viewer
            != &(PrincipalId::Object {
                id: ObjId(decoded.actor),
            })
        {
            return Err(WillpowerRollProvider::error(
                ActionErrorCode::Integrity,
                "WoD resource receipt disagrees with its eligible viewer",
            ));
        }
        mush_proto::ProjectedActionResult::new(
            SET_ACTION_ID,
            format!(
                "Set #{} temporary Willpower to {}.",
                decoded.target, decoded.current
            ),
            json!({
                "target": decoded.target,
                "current": decoded.current,
                "roll_sequence": decoded.roll_sequence,
            }),
        )
        .map(Some)
        .map_err(|_| {
            WillpowerRollProvider::error(
                ActionErrorCode::BoundsExceeded,
                "WoD resource receipt exceeds public bounds",
            )
        })
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let request = self.parse(invocation)?;
        let prior = txn
            .get(&self.state, &request.target)
            .map_err(|error| WillpowerRollProvider::state_error(&error))?
            .cloned();
        let roll_sequence = prior
            .as_ref()
            .map_or(request.seed_sequence, |state| state.roll_sequence);
        let next = WodRollState {
            willpower_current: request.current.min(request.permanent),
            roll_sequence,
        };
        if prior.is_some() {
            txn.replace(&self.state, &request.target, next.clone())
        } else {
            txn.insert(&self.state, &request.target, next.clone())
        }
        .map_err(|error| WillpowerRollProvider::state_error(&error))?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "actor": invocation.actor().0,
                "target": request.target.0,
                "current": next.willpower_current,
                "roll_sequence": next.roll_sequence,
            }),
        )
        .map_err(|_| {
            WillpowerRollProvider::error(
                ActionErrorCode::BoundsExceeded,
                "WoD resource outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

pub(crate) fn register(
    reg: &mut Registrar<'_>,
    dispatch: Arc<Dispatch>,
) -> Result<StateFamilyHandle<ObjId, WodRollState>, ModError> {
    let state = reg.state_family(
        StateFamilyDescriptor::new(
            StateFamilyId::new(STATE_FAMILY).map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("obj_id.decimal.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("wod.roll_state.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            1,
            CanonicalId::new("wod.roll_state.decode_json.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("wod.roll_state.validate.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("wod.roll_state.json.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &ObjId| Ok(key.0.to_string()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|error| error.to_string()),
        |value: &WodRollState| {
            if value.willpower_current < 0 || value.willpower_current > 10 {
                Err("Willpower current must be between zero and ten".to_owned())
            } else {
                Ok(())
            }
        },
        |value: &WodRollState| serde_json::to_value(value).map_err(|error| error.to_string()),
    )?;

    let outcome_descriptor = descriptor(
        "outcome",
        OUTCOME_ID,
        json!({
            "fields": ["actor", "pool", "roll", "willpower_remaining", "roll_sequence"],
            "dice": "owod20",
        }),
    )?;
    let resource_outcome_descriptor = descriptor(
        "outcome",
        RESOURCE_OUTCOME_ID,
        json!({
            "fields": ["actor", "target", "current", "roll_sequence"],
            "resource": "willpower_temp",
        }),
    )?;
    reg.definition_validator(
        DefinitionKind::new("outcome").map_err(|error| ModError::msg(error.to_string()))?,
        exact_validator(&[
            outcome_descriptor.clone(),
            resource_outcome_descriptor.clone(),
        ]),
    )?;
    let outcome = reg.definition(outcome_descriptor)?;
    let resource_outcome = reg.definition(resource_outcome_descriptor)?;
    let action_descriptor = descriptor(
        "action",
        ACTION_ID,
        json!({
            "subject": "self_actor",
            "targets": 0,
            "resource": "willpower_temp",
            "state_family": STATE_FAMILY,
            "resolution": "owod20",
        }),
    )?
    .with_references(vec![outcome.clone()]);
    let set_action_descriptor = descriptor(
        "action",
        SET_ACTION_ID,
        json!({
            "subject": "known_object",
            "targets": 0,
            "operation": "set",
            "resource": "willpower_temp",
            "state_family": STATE_FAMILY,
        }),
    )?
    .with_references(vec![resource_outcome.clone()]);
    reg.definition_validator(
        DefinitionKind::new("action").map_err(|error| ModError::msg(error.to_string()))?,
        exact_validator(&[action_descriptor.clone(), set_action_descriptor.clone()]),
    )?;
    let action = reg.definition(action_descriptor)?;
    let set_action = reg.definition(set_action_descriptor)?;
    // H2c: the NPC grounding maps a Willpower-backed `wod_roll` onto this
    // action from a `&World`, so the definition handle is published as state.
    reg.state(WodActionHandles {
        willpower_roll: action.clone(),
    });
    reg.action_provider(
        action.clone(),
        WillpowerRollProvider {
            dispatch: Arc::clone(&dispatch),
            action,
            outcome,
            state: state.clone(),
        },
    )?;
    reg.action_provider(
        set_action.clone(),
        WillpowerSetProvider {
            dispatch,
            action: set_action,
            outcome: resource_outcome,
            state: state.clone(),
        },
    )?;
    Ok(state)
}

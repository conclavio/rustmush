//! The toy's one **authoritative, resource-backed** action pair.
//!
//! `<id>-willpower-roll` spends one point from a typed `roll_state` family and
//! rolls the actor's own sheet pool on a **recorded resolution stream**;
//! `<id>-willpower-set` is the staff reset. Between them they carry every
//! platform property the deleted game rulesets carried for
//! `crates/mush-server/tests/world_task/production_action_migrations.rs` and
//! `crates/mush-server/tests/world_task/h2c_ability_coordinator.rs`:
//!
//! - **one commit** — the resource moves exactly once per accepted offer, and
//!   the offer stops being projected when the pool is empty;
//! - **a replayable resolution record** — the dice come from
//!   `PrepareCtx::start_resolution` + [`with_resolution_stream`], so the bundle
//!   carries a transcript whose outcome equals the committed outcome;
//! - **vitals and sheet republished to the subject** — the family's key codec
//!   is the decimal `ObjId`, which
//!   [`state_subject`] hands back to the vitals republish (interface plan I13);
//! - **post-image rehydration** — the typed post-image in the bundle's writes
//!   hydrates into a fresh composition and still owns `willpower_current`;
//! - **a projected ability** — the definition handle is published as module
//!   state ([`ToyActionHandles`]) so the NPC coordinator can re-route a
//!   Willpower-backed `toy_roll` onto this action from a `&World`.
//!
//! This file deliberately reaches for **no** ambient entropy of any kind:
//! `crates/mush-contract-tests/tests/provider_rng_policy.rs` scans every file
//! containing `impl ActionProvider for` and fails if it names one. The toy's
//! legacy, non-authoritative dice live in the parent module instead.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionPresentation, ActionProvider,
    ActionStateError, ActionTxn, AuthoritativeActionOutput, CanonicalHash, CanonicalId,
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionValidationContext,
    Engine, LegalActionQuery, ModError, ObjId, PerceivedRef, PrincipalId, ProjectedParameter,
    ProjectedParameterKind, ProjectedParameterSchema, ProjectedTargetSchema, ProviderOfferDraft,
    Registrar, RevisionSet, Role, RulesValue, StateFamilyDescriptor, StateFamilyHandle,
    StateFamilyId, SubjectKey, perm,
};
use mush_dice::owod20::{Owod20Opts, Owod20Outcome, Owod20Pool};
use mush_dice::{PoolMechanic, RollLimits, with_resolution_stream};
use mush_ruleset::{SheetDoc, SheetSchema, SheetStore};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The typed family name. Deliberately **not** namespaced: two toy modules own
/// a family of this name, and the republish must filter them apart by OWNER,
/// not by id (the I13 two-owner pattern).
pub const STATE_FAMILY: &str = "roll_state";

/// The handles an NPC grounding needs from a `&World`, published as module
/// state by the registering toy.
#[derive(Clone, Debug)]
pub struct ToyActionHandles {
    /// `<id>_willpower_roll`.
    pub willpower_roll: DefinitionRef,
    /// `<id>_willpower_set`.
    pub willpower_set: DefinitionRef,
}

/// The authoritative post-image: the Willpower pool and a monotonic roll
/// counter, keyed by the character's decimal `ObjId`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToyRollState {
    /// The temporary pool the typed record owns once it exists.
    pub willpower_current: i64,
    /// How many authoritative rolls this subject has committed.
    pub roll_sequence: u64,
}

/// The sheet subject a `roll_state` record key names (interface plan I13).
/// A foreign family is nobody's business here.
#[must_use]
pub fn state_subject(family: &StateFamilyId, key: &str) -> Option<ObjId> {
    if family.as_str() != STATE_FAMILY {
        return None;
    }
    key.parse::<u32>().ok().map(ObjId)
}

// ----------------------------------------------------------------------
// Definition plumbing
// ----------------------------------------------------------------------

#[derive(Clone)]
struct ExpectedDefinition {
    payload: Value,
    references: Vec<DefinitionRef>,
    current: bool,
}

/// A definition validator that accepts exactly the shapes this module
/// registered and nothing else.
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
            return Err("unknown toy authoritative definition".to_owned());
        };
        if definition.payload() != &shape.payload
            || definition.references() != shape.references
            || definition.is_current() != shape.current
        {
            return Err("toy authoritative definition shape mismatch".to_owned());
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

fn canonical(value: &str) -> Result<CanonicalId, ModError> {
    CanonicalId::new(value).map_err(|error| ModError::msg(error.to_string()))
}

// ----------------------------------------------------------------------
// Providers
// ----------------------------------------------------------------------

struct RollRequest {
    sheet: SheetDoc,
    pool_expr: String,
    difficulty: u8,
    modifier: i32,
    ignore_wounds: bool,
    seed_willpower: i64,
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

struct ResourceRequest {
    target: ObjId,
    current: i64,
    permanent: i64,
    seed_sequence: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceOutcome {
    actor: u32,
    target: u32,
    current: i64,
    roll_sequence: u64,
}

struct WillpowerRollProvider {
    schema: Arc<SheetSchema>,
    action: DefinitionRef,
    outcome: DefinitionRef,
    state: StateFamilyHandle<ObjId, ToyRollState>,
    /// The projected receipt's kind (`<id>_willpower_roll`).
    kind: String,
}

struct WillpowerSetProvider {
    schema: Arc<SheetSchema>,
    action: DefinitionRef,
    outcome: DefinitionRef,
    state: StateFamilyHandle<ObjId, ToyRollState>,
    kind: String,
}

fn error(code: ActionErrorCode, message: impl Into<String>) -> ActionError {
    ActionError::new(code, false, message).expect("bounded toy authoritative action error")
}

fn state_error(_error: &ActionStateError) -> ActionError {
    error(
        ActionErrorCode::Integrity,
        "toy roll state preparation was refused",
    )
}

impl WillpowerRollProvider {
    fn parse(invocation: &ActionInvocation) -> Result<RollRequest, ActionError> {
        if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
            return Err(error(
                ActionErrorCode::InvalidRequest,
                "Willpower roll must be self-scoped and have no targets",
            ));
        }
        let parameters = invocation.parameters().as_object().ok_or_else(|| {
            error(
                ActionErrorCode::InvalidRequest,
                "Willpower roll parameters must be an object",
            )
        })?;
        const FIELDS: [&str; 6] = [
            "difficulty",
            "ignore_wounds",
            "modifier",
            "pool",
            "seed_willpower",
            "sheet",
        ];
        if parameters.len() < 3 || parameters.keys().any(|key| !FIELDS.contains(&key.as_str())) {
            return Err(error(
                ActionErrorCode::InvalidRequest,
                "Willpower roll has unexpected parameters",
            ));
        }
        let sheet: SheetDoc =
            serde_json::from_value(parameters.get("sheet").cloned().ok_or_else(|| {
                error(
                    ActionErrorCode::InvalidRequest,
                    "Willpower roll lacks its sheet",
                )
            })?)
            .map_err(|_| {
                error(
                    ActionErrorCode::InvalidRequest,
                    "Willpower roll sheet is malformed",
                )
            })?;
        let pool_expr = parameters
            .get("pool")
            .and_then(Value::as_str)
            .filter(|pool| !pool.trim().is_empty())
            .ok_or_else(|| {
                error(
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
                error(
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
                error(
                    ActionErrorCode::InvalidRequest,
                    "Willpower roll modifier must be -20 through 20",
                )
            })?;
        let seed_willpower = parameters
            .get("seed_willpower")
            .and_then(Value::as_i64)
            .filter(|value| (0..=10).contains(value))
            .ok_or_else(|| {
                error(
                    ActionErrorCode::InvalidRequest,
                    "Willpower roll resource seed is invalid",
                )
            })?;
        Ok(RollRequest {
            sheet,
            pool_expr,
            difficulty,
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
        WillpowerRollProvider::parse(invocation).map(|_| ())
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
        let Some(sheet) =
            SheetStore::new(Arc::clone(&self.schema)).get(&engine.world, query.actor())
        else {
            return Ok(None);
        };
        if mush_ruleset::validate(&self.schema, &sheet.data).is_err() {
            return Err(error(
                ActionErrorCode::Integrity,
                "toy sheet failed authoritative validation",
            ));
        }
        let permanent = sheet.number("willpower").unwrap_or(0).clamp(0, 10);
        let seed = sheet
            .number("willpower_current")
            .unwrap_or(0)
            .clamp(0, permanent);
        let current = engine
            .action_state()
            .get(&self.state, &query.actor())
            .map_err(|e| state_error(&e))?
            .map_or(seed, |state| state.willpower_current.min(permanent));
        if current == 0 {
            // The resource is spent: the offer stops being projected, which is
            // what makes "one commit per point" observable from outside.
            return Ok(None);
        }
        let definitions = engine.registry.definitions().ok_or_else(|| {
            error(
                ActionErrorCode::Integrity,
                "toy definition registry is unavailable",
            )
        })?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "toy guard registry is unavailable",
                )
            })?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "toy action definition guard is unavailable",
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
        ])
        .map_err(|_| {
            error(
                ActionErrorCode::Integrity,
                "toy roll parameter schema is invalid",
            )
        })?;
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({ "sheet": sheet, "seed_willpower": current }),
            parameters,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: ActionPresentation::new(
                self.kind.replace('_', "-"),
                "Toy Willpower Roll",
                "Spend one point from the toy Willpower pool and roll a sheet pool.",
            )
            .map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "toy roll presentation is invalid",
                )
            })?,
            guards: guards
                .finish()
                .map_err(|_| error(ActionErrorCode::Integrity, "toy roll guards are invalid"))?,
        }))
    }

    fn project_receipt(
        &self,
        action: &DefinitionRef,
        outcome: &RulesValue,
        viewer: &PrincipalId,
    ) -> Result<Option<mush_proto::ProjectedActionResult>, ActionError> {
        if action != &self.action || outcome.schema() != &self.outcome {
            return Err(error(
                ActionErrorCode::Integrity,
                "toy roll receipt carries the wrong durable schema",
            ));
        }
        let decoded: RollOutcome =
            serde_json::from_value(outcome.data().clone()).map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "toy roll receipt outcome is malformed",
                )
            })?;
        if viewer
            != &(PrincipalId::Object {
                id: ObjId(decoded.actor),
            })
            || decoded.pool < 1
            || decoded.roll_sequence == 0
        {
            return Err(error(
                ActionErrorCode::Integrity,
                "toy roll receipt disagrees with its eligible viewer",
            ));
        }
        mush_proto::ProjectedActionResult::new(
            self.kind.clone(),
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
            error(
                ActionErrorCode::BoundsExceeded,
                "toy roll receipt exceeds public bounds",
            )
        })
    }

    fn prepare_authoritative_with_context(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        context: &mut mush_core::PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let request = WillpowerRollProvider::parse(invocation)?;
        mush_ruleset::validate(&self.schema, &request.sheet.data).map_err(|_| {
            error(
                ActionErrorCode::Integrity,
                "toy roll sheet failed schema validation",
            )
        })?;
        let resolved = super::resolve_pool(&self.schema, &request.sheet, &request.pool_expr)
            .map_err(|_| {
                error(
                    ActionErrorCode::ProviderRejected,
                    "That pool does not resolve against the current sheet",
                )
            })?;

        // --- the one resource move, inside the transaction -----------------
        let permanent = request.sheet.number("willpower").unwrap_or(0).clamp(0, 10);
        let prior = txn
            .get(&self.state, &invocation.actor())
            .map_err(|e| state_error(&e))?
            .cloned();
        let current = prior
            .as_ref()
            .map_or(request.seed_willpower, |state| state.willpower_current)
            .clamp(0, permanent);
        if current == 0 {
            return Err(error(
                ActionErrorCode::ProviderRejected,
                "No toy Willpower remains",
            ));
        }
        let sequence = prior
            .as_ref()
            .map_or(Ok(1), |state| state.roll_sequence.checked_add(1).ok_or(()))
            .map_err(|()| {
                error(
                    ActionErrorCode::BoundsExceeded,
                    "toy roll sequence is exhausted",
                )
            })?;
        let next = ToyRollState {
            willpower_current: current - 1,
            roll_sequence: sequence,
        };
        if prior.is_some() {
            txn.replace(&self.state, &invocation.actor(), next.clone())
        } else {
            txn.insert(&self.state, &invocation.actor(), next.clone())
        }
        .map_err(|e| state_error(&e))?;

        // --- the recorded resolution ---------------------------------------
        let wound = if request.ignore_wounds {
            0
        } else {
            i32::from(
                SheetStore::new(Arc::clone(&self.schema)).wound_penalty(&request.sheet, "health"),
            )
        };
        let pool = i32::try_from(resolved.total)
            .unwrap_or(i32::MAX)
            .saturating_add(request.modifier)
            .saturating_add(wound)
            .max(1);
        let opts = Owod20Opts {
            difficulty: request.difficulty,
            specialty: false,
            willpower: true,
            extra_successes: 0,
            again_tens_double: false,
        };
        let inputs_hash = CanonicalHash::for_canonical_value(
            "toy_willpower_roll_inputs.v1",
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
            error(
                ActionErrorCode::BoundsExceeded,
                "toy roll inputs exceed canonical bounds",
            )
        })?;
        let mut stream = context
            .start_resolution(
                &CanonicalId::new("toy_willpower_roll").expect("static resolution domain"),
                inputs_hash,
            )
            .map_err(|_| {
                error(
                    ActionErrorCode::Unavailable,
                    "Authoritative resolution keys are unavailable",
                )
            })?;
        let roll = with_resolution_stream(
            &mut stream,
            &CanonicalId::new("primary_pool").expect("static draw label"),
            RollLimits::default(),
            |roll_context| Owod20Pool.roll(pool, &opts, roll_context),
        )
        .map_err(|_| {
            error(
                ActionErrorCode::BoundsExceeded,
                "toy roll exceeded its deterministic draw budget",
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
            error(
                ActionErrorCode::BoundsExceeded,
                "toy roll outcome exceeds canonical bounds",
            )
        })?;
        let resolution = stream
            .into_prepared(
                vec![self.action.clone()],
                RulesValue::new(
                    self.action.clone(),
                    json!({
                        "pool_expr": request.pool_expr,
                        "pool": pool,
                        "difficulty": request.difficulty,
                        "modifier": request.modifier,
                        "wound": wound,
                        "willpower_spent": 1,
                    }),
                )
                .map_err(|_| {
                    error(
                        ActionErrorCode::BoundsExceeded,
                        "toy roll modifiers exceed canonical bounds",
                    )
                })?,
                outcome.clone(),
            )
            .map_err(|_| error(ActionErrorCode::Integrity, "toy roll transcript is invalid"))?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_resolution(resolution)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

impl WillpowerSetProvider {
    fn parse(&self, invocation: &ActionInvocation) -> Result<ResourceRequest, ActionError> {
        if !invocation.targets().is_empty() {
            return Err(error(
                ActionErrorCode::InvalidRequest,
                "Willpower resource action accepts no target list",
            ));
        }
        let parameters = invocation.parameters().as_object().ok_or_else(|| {
            error(
                ActionErrorCode::InvalidRequest,
                "Willpower resource parameters must be an object",
            )
        })?;
        const FIELDS: [&str; 4] = ["current", "permanent", "seed_sequence", "target"];
        if parameters.len() != FIELDS.len()
            || parameters.keys().any(|key| !FIELDS.contains(&key.as_str()))
        {
            return Err(error(
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
                error(
                    ActionErrorCode::InvalidRequest,
                    "Willpower resource target is invalid",
                )
            })?;
        if !matches!(invocation.subject(), PerceivedRef::KnownObject { id, .. } if *id == target) {
            return Err(error(
                ActionErrorCode::InvalidRequest,
                "Willpower resource subject disagrees with its target",
            ));
        }
        let permanent = parameters
            .get("permanent")
            .and_then(Value::as_i64)
            .filter(|value| (0..=10).contains(value))
            .ok_or_else(|| {
                error(
                    ActionErrorCode::InvalidRequest,
                    "Willpower permanent rating is invalid",
                )
            })?;
        let current = parameters
            .get("current")
            .and_then(Value::as_i64)
            .filter(|value| (0..=permanent).contains(value))
            .ok_or_else(|| {
                error(
                    ActionErrorCode::InvalidRequest,
                    "Toy Willpower must be between zero and the permanent rating",
                )
            })?;
        let seed_sequence = parameters
            .get("seed_sequence")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                error(
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
        if !perm::has_role(&engine.world, query.actor(), Role::Royalty) {
            return Ok(None);
        }
        let Some(sheet) = SheetStore::new(Arc::clone(&self.schema)).get(&engine.world, *target)
        else {
            return Ok(None);
        };
        if mush_ruleset::validate(&self.schema, &sheet.data).is_err() {
            return Err(error(
                ActionErrorCode::Integrity,
                "toy sheet failed authoritative validation",
            ));
        }
        let permanent = sheet.number("willpower").unwrap_or(0).clamp(0, 10);
        let seed_sequence = engine
            .action_state()
            .get(&self.state, target)
            .map_err(|e| state_error(&e))?
            .map_or(0, |state| state.roll_sequence);
        let definitions = engine.registry.definitions().ok_or_else(|| {
            error(
                ActionErrorCode::Integrity,
                "toy definition registry is unavailable",
            )
        })?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "toy guard registry is unavailable",
                )
            })?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "toy resource definition guard is unavailable",
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
            error(
                ActionErrorCode::Integrity,
                "toy resource parameter schema is invalid",
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
                self.kind.replace('_', "-"),
                "Set Toy Willpower",
                "Set a character's authoritative toy Willpower pool.",
            )
            .map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "toy resource presentation is invalid",
                )
            })?,
            guards: guards.finish().map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "toy resource guards are invalid",
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
            return Err(error(
                ActionErrorCode::Integrity,
                "toy resource receipt carries the wrong durable schema",
            ));
        }
        let decoded: ResourceOutcome =
            serde_json::from_value(outcome.data().clone()).map_err(|_| {
                error(
                    ActionErrorCode::Integrity,
                    "toy resource receipt outcome is malformed",
                )
            })?;
        if viewer
            != &(PrincipalId::Object {
                id: ObjId(decoded.actor),
            })
        {
            return Err(error(
                ActionErrorCode::Integrity,
                "toy resource receipt disagrees with its eligible viewer",
            ));
        }
        mush_proto::ProjectedActionResult::new(
            self.kind.clone(),
            format!(
                "Set #{} toy Willpower to {}.",
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
            error(
                ActionErrorCode::BoundsExceeded,
                "toy resource receipt exceeds public bounds",
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
            .map_err(|e| state_error(&e))?
            .cloned();
        let roll_sequence = prior
            .as_ref()
            .map_or(request.seed_sequence, |state| state.roll_sequence);
        let next = ToyRollState {
            willpower_current: request.current.min(request.permanent),
            roll_sequence,
        };
        if prior.is_some() {
            txn.replace(&self.state, &request.target, next.clone())
        } else {
            txn.insert(&self.state, &request.target, next.clone())
        }
        .map_err(|e| state_error(&e))?;
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
            error(
                ActionErrorCode::BoundsExceeded,
                "toy resource outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

// ----------------------------------------------------------------------
// Registration
// ----------------------------------------------------------------------

/// Register **only** the typed family. Both toy owners call this: the family
/// id is shared, the codec ids are namespaced by module.
///
/// # Errors
///
/// A malformed canonical id or a family-registration conflict.
pub fn register_family(
    reg: &mut Registrar<'_>,
    module: &str,
) -> Result<StateFamilyHandle<ObjId, ToyRollState>, ModError> {
    reg.state_family(
        StateFamilyDescriptor::new(
            StateFamilyId::new(STATE_FAMILY).map_err(|e| ModError::msg(e.to_string()))?,
            canonical("obj_id.decimal.v1")?,
            canonical(&format!("{module}.roll_state.v1"))?,
            1,
            canonical(&format!("{module}.roll_state.decode_json.v1"))?,
            canonical(&format!("{module}.roll_state.validate.v1"))?,
            canonical(&format!("{module}.roll_state.json.v1"))?,
        )
        .map_err(|e| ModError::msg(e.to_string()))?,
        |key: &ObjId| Ok(key.0.to_string()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &ToyRollState| {
            if value.willpower_current < 0 || value.willpower_current > 10 {
                Err("toy Willpower must be between zero and ten".to_owned())
            } else {
                Ok(())
            }
        },
        |value: &ToyRollState| serde_json::to_value(value).map_err(|e| e.to_string()),
    )
}

/// Register the two definitions, their validators, the two providers, and the
/// [`ToyActionHandles`] state slot the NPC grounding reads.
///
/// # Errors
///
/// A malformed canonical id, a definition conflict, or a provider conflict.
pub fn register_actions(
    reg: &mut Registrar<'_>,
    module: &str,
    schema: Arc<SheetSchema>,
    state: StateFamilyHandle<ObjId, ToyRollState>,
) -> Result<(), ModError> {
    let roll_id = format!("{module}_willpower_roll");
    let set_id = format!("{module}_willpower_set");
    let roll_outcome_id = format!("{module}_willpower_roll_result");
    let set_outcome_id = format!("{module}_willpower_resource_result");

    let roll_outcome_descriptor = descriptor(
        "outcome",
        &roll_outcome_id,
        json!({
            "fields": ["actor", "pool", "roll", "willpower_remaining", "roll_sequence"],
            "dice": "owod20",
        }),
    )?;
    let set_outcome_descriptor = descriptor(
        "outcome",
        &set_outcome_id,
        json!({
            "fields": ["actor", "target", "current", "roll_sequence"],
            "resource": "willpower_current",
        }),
    )?;
    reg.definition_validator(
        DefinitionKind::new("outcome").map_err(|e| ModError::msg(e.to_string()))?,
        exact_validator(&[
            roll_outcome_descriptor.clone(),
            set_outcome_descriptor.clone(),
        ]),
    )?;
    let roll_outcome = reg.definition(roll_outcome_descriptor)?;
    let set_outcome = reg.definition(set_outcome_descriptor)?;

    let roll_descriptor = descriptor(
        "action",
        &roll_id,
        json!({
            "subject": "self_actor",
            "targets": 0,
            "resource": "willpower_current",
            "state_family": STATE_FAMILY,
            "resolution": "owod20",
        }),
    )?
    .with_references(vec![roll_outcome.clone()]);
    let set_descriptor = descriptor(
        "action",
        &set_id,
        json!({
            "subject": "known_object",
            "targets": 0,
            "operation": "set",
            "resource": "willpower_current",
            "state_family": STATE_FAMILY,
        }),
    )?
    .with_references(vec![set_outcome.clone()]);
    reg.definition_validator(
        DefinitionKind::new("action").map_err(|e| ModError::msg(e.to_string()))?,
        exact_validator(&[roll_descriptor.clone(), set_descriptor.clone()]),
    )?;
    let roll_action = reg.definition(roll_descriptor)?;
    let set_action = reg.definition(set_descriptor)?;

    reg.state(ToyActionHandles {
        willpower_roll: roll_action.clone(),
        willpower_set: set_action.clone(),
    });
    reg.action_provider(
        roll_action.clone(),
        WillpowerRollProvider {
            schema: Arc::clone(&schema),
            action: roll_action,
            outcome: roll_outcome,
            state: state.clone(),
            kind: roll_id,
        },
    )?;
    reg.action_provider(
        set_action.clone(),
        WillpowerSetProvider {
            schema,
            action: set_action,
            outcome: set_outcome,
            state,
            kind: set_id,
        },
    )?;
    Ok(())
}

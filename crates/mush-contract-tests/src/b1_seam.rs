//! E7c2 B1 fixture — the smallest complete `Authoritative` command lane.
//!
//! B1 (10b §3.2's "As built") is the ingress adapter seam 08j §5 (iii) named as
//! missing: *"`CommandHandler`/`Ctx` is synchronous and cannot reach
//! `ActionCoordinator::gateway_ingress`, so the first `Authoritative`
//! registration needs an adapter seam that does not exist."* This module is the
//! fixture that proves the seam **before** any inventory record exists: one
//! registered action with a provider, one typed counter family, and commands
//! that submit through [`mush_core::cmd::Ctx::submit_authoritative`] — correctly
//! once, incorrectly twice, and beside a legacy persistence op.
//!
//! It is deliberately free of anything TacSim-, industry- or economy-shaped: it
//! exercises the *lane*, not a mechanic. It is also deliberately clock-free —
//! `Engine::legal_action_query` builds its query with `now: None`, so a fixture
//! whose projection demanded a scene turn could not be reached from a command at
//! all.
//!
//! **No ambient RNG**: `crates/mush-contract-tests/tests/provider_rng_policy.rs`
//! scans every file containing `impl ActionProvider for`, and this one draws
//! nothing.

use std::sync::Arc;

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionStateError, ActionTxn,
    ArgShape, AuthoritativeActionOutput, CanonicalId, CmdError, CommandCompatibility, CommandSpec,
    Ctx, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionValidationContext, GameModule, IdempotencyKeyText, ModError, ModuleOp, ModuleWrite,
    ObjId, PerceivedRef, PrincipalId, ProjectedParameter, ProjectedParameterKind,
    ProjectedParameterSchema, ProjectedTargetSchema, ProviderOfferDraft, Registrar, RevisionSet,
    RulesValue, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId, SubjectKey,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Owner id of the seam fixture. Distinct from every production owner.
pub const B1_OWNER: &str = "b1seam";
/// The one action the fixture registers.
pub const B1_ACTION: &str = "b1_seam_bump";
/// Its durable outcome schema.
pub const B1_OUTCOME: &str = "b1_seam_bump_result";
/// The command's deterministic idempotency discriminator.
pub const B1_KEY: &str = "b1.bump";
/// Largest bump one submission may request.
pub const B1_MAX_AMOUNT: i64 = 1_000;

/// The counter one commit moves. A whole-document post-image with a strict
/// wire, exactly like every other typed family in the tree.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct B1Counter {
    /// Total bumped so far.
    pub count: i64,
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

fn validate_definition(_context: &DefinitionValidationContext<'_>) -> Result<(), String> {
    Ok(())
}

struct B1Provider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    counters: StateFamilyHandle<ObjId, B1Counter>,
}

impl B1Provider {
    fn error(code: ActionErrorCode, message: &'static str) -> ActionError {
        ActionError::new(code, false, message).expect("static b1 seam error is bounded")
    }

    fn state_error(_error: &ActionStateError) -> ActionError {
        Self::error(ActionErrorCode::Integrity, "b1 seam counter is unavailable")
    }

    fn amount(invocation: &ActionInvocation) -> Result<i64, ActionError> {
        if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
            return Err(Self::error(
                ActionErrorCode::InvalidRequest,
                "b1 seam bump is self-scoped and takes no targets",
            ));
        }
        invocation
            .parameters()
            .as_object()
            .filter(|parameters| parameters.len() == 1)
            .and_then(|parameters| parameters.get("amount"))
            .and_then(Value::as_i64)
            .filter(|amount| (1..=B1_MAX_AMOUNT).contains(amount))
            .ok_or_else(|| {
                Self::error(
                    ActionErrorCode::InvalidRequest,
                    "b1 seam bump needs one amount between 1 and 1000",
                )
            })
    }
}

impl ActionProvider for B1Provider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        Self::amount(invocation).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        let SubjectKey::Object { id: subject } = query.truth_subject() else {
            return Ok(None);
        };
        if action != &self.action
            || query.actor() != *subject
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let definitions = engine.registry.definitions().ok_or_else(|| {
            Self::error(
                ActionErrorCode::Integrity,
                "b1 seam definition registry is unavailable",
            )
        })?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| {
                Self::error(
                    ActionErrorCode::Integrity,
                    "b1 seam guard registry is unavailable",
                )
            })?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| {
                Self::error(
                    ActionErrorCode::Integrity,
                    "b1 seam definition guard is unavailable",
                )
            })?;
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({}),
            parameters: ProjectedParameterSchema::new(vec![ProjectedParameter {
                id: CanonicalId::new("amount").expect("static parameter id"),
                required: true,
                kind: ProjectedParameterKind::Integer {
                    minimum: 1,
                    maximum: B1_MAX_AMOUNT,
                },
            }])
            .map_err(|_| {
                Self::error(
                    ActionErrorCode::Integrity,
                    "b1 seam parameter schema is invalid",
                )
            })?,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "b1-bump",
                "Bump",
                "Move the seam fixture's counter by one bounded amount.",
            )
            .map_err(|_| {
                Self::error(
                    ActionErrorCode::Integrity,
                    "b1 seam presentation is invalid",
                )
            })?,
            guards: guards.finish().map_err(|_| {
                Self::error(
                    ActionErrorCode::Integrity,
                    "b1 seam offer guards are invalid",
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
            return Err(Self::error(
                ActionErrorCode::Integrity,
                "b1 seam receipt carries the wrong durable schema",
            ));
        }
        let actor = outcome
            .data()
            .get("actor")
            .and_then(Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .map(ObjId)
            .ok_or_else(|| {
                Self::error(ActionErrorCode::Integrity, "b1 seam receipt names no actor")
            })?;
        let count = outcome
            .data()
            .get("count")
            .and_then(Value::as_i64)
            .ok_or_else(|| {
                Self::error(
                    ActionErrorCode::Integrity,
                    "b1 seam receipt names no counter",
                )
            })?;
        if viewer != &(PrincipalId::Object { id: actor }) {
            return Err(Self::error(
                ActionErrorCode::Integrity,
                "b1 seam receipt disagrees with its eligible viewer",
            ));
        }
        mush_proto::ProjectedActionResult::new(
            B1_ACTION,
            format!("The counter stands at {count}."),
            json!({ "count": count }),
        )
        .map(Some)
        .map_err(|_| {
            Self::error(
                ActionErrorCode::BoundsExceeded,
                "b1 seam receipt exceeds public bounds",
            )
        })
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let amount = Self::amount(invocation)?;
        let actor = invocation.actor();
        let prior = txn
            .get(&self.counters, &actor)
            .map_err(|error| Self::state_error(&error))?
            .cloned();
        let count = prior
            .as_ref()
            .map_or(0, |counter| counter.count)
            .checked_add(amount)
            .ok_or_else(|| {
                Self::error(
                    ActionErrorCode::BoundsExceeded,
                    "b1 seam counter is exhausted",
                )
            })?;
        let next = B1Counter { count };
        if prior.is_some() {
            txn.replace(&self.counters, &actor, next)
        } else {
            txn.insert(&self.counters, &actor, next)
        }
        .map_err(|error| Self::state_error(&error))?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({ "actor": actor.0, "count": count }),
        )
        .map_err(|_| {
            Self::error(
                ActionErrorCode::BoundsExceeded,
                "b1 seam outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

/// The fixture module. Composed only by tests.
#[derive(Debug, Default)]
pub struct B1SeamModule;

fn bump_amount(ctx: &Ctx<'_>) -> i64 {
    ctx.args
        .raw()
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|amount| (1..=B1_MAX_AMOUNT).contains(amount))
        .unwrap_or(1)
}

fn submit(ctx: &mut Ctx<'_>, action: &DefinitionRef, amount: i64) {
    ctx.submit_authoritative(
        action.clone(),
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({ "amount": amount }),
        IdempotencyKeyText::parse_canonical(B1_KEY).expect("static b1 seam key is canonical"),
    );
}

impl GameModule for B1SeamModule {
    fn name(&self) -> &'static str {
        B1_OWNER
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        for kind in ["outcome", "action"] {
            reg.definition_validator(
                DefinitionKind::new(kind).map_err(|error| ModError::msg(error.to_string()))?,
                validate_definition,
            )?;
        }
        let counters = reg.state_family(
            StateFamilyDescriptor::new(
                StateFamilyId::new("b1_counters")
                    .map_err(|error| ModError::msg(error.to_string()))?,
                CanonicalId::new("obj_id.decimal.v1")
                    .map_err(|error| ModError::msg(error.to_string()))?,
                CanonicalId::new("b1.counter.v1")
                    .map_err(|error| ModError::msg(error.to_string()))?,
                1,
                CanonicalId::new("b1.counter.decode_json.v1")
                    .map_err(|error| ModError::msg(error.to_string()))?,
                CanonicalId::new("b1.counter.range.v1")
                    .map_err(|error| ModError::msg(error.to_string()))?,
                CanonicalId::new("b1.counter.json.v1")
                    .map_err(|error| ModError::msg(error.to_string()))?,
            )
            .map_err(|error| ModError::msg(error.to_string()))?,
            |key: &ObjId| Ok(key.0.to_string()),
            |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
            |value: &B1Counter| {
                if value.count >= 0 {
                    Ok(())
                } else {
                    Err("b1 seam counter is never negative".to_owned())
                }
            },
            |value: &B1Counter| serde_json::to_value(value).map_err(|e| e.to_string()),
        )?;
        let outcome = reg.definition(descriptor(
            "outcome",
            B1_OUTCOME,
            json!({ "fields": ["actor", "count"] }),
        )?)?;
        let action = reg.definition(
            descriptor(
                "action",
                B1_ACTION,
                json!({ "subject": "self_actor", "targets": 0 }),
            )?
            .with_references(vec![outcome.clone()]),
        )?;
        reg.action_provider(
            action.clone(),
            B1Provider {
                action: action.clone(),
                outcome,
                counters,
            },
        )?;

        // K8.5: the lane is declared before the first `reg.command`, and it is
        // `Authoritative` — the handler is an ingress adapter and implements no
        // mechanic of its own.
        reg.classify_commands(CommandCompatibility::Authoritative);
        let once = action.clone();
        reg.command(
            CommandSpec {
                name: "@b1submit".into(),
                switches: vec!["twice".into(), "dirty".into()],
                shape: ArgShape::Raw,
                help: "Submit the seam fixture's authoritative bump.".to_owned(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let amount = bump_amount(ctx);
                if ctx.switches.has("dirty") {
                    // A legacy post-image queued in the SAME step. The driver
                    // must defer the submission rather than let a prepared
                    // action ride beside it.
                    ctx.persist(ModuleOp {
                        ns: B1_OWNER.into(),
                        op: ModuleWrite::JsonSet {
                            key: "b1:dirty".into(),
                            value: json!({ "amount": amount }),
                        },
                    });
                }
                submit(ctx, &once, amount);
                if ctx.switches.has("twice") {
                    submit(ctx, &once, amount);
                }
                Ok(())
            }),
        )?;
        Ok(())
    }
}

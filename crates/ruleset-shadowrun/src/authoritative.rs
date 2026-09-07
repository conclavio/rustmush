//! K8.4 authoritative Shadowrun Condition Monitor actions.
//!
//! These providers are the production proof that a second setting ruleset can
//! use the same typed transaction, offer, receipt, and crash-safe coordinator
//! as WoD and tactics. The typed record owns only Condition Monitor fill and
//! overflow; presentation overlays it onto the current legacy sheet so
//! unrelated sheet fields retain their existing migration owner.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionPresentation, ActionProvider,
    ActionStateError, ActionTxn, AuthoritativeActionOutput, CanonicalId, DefinitionDescriptor,
    DefinitionId, DefinitionKind, DefinitionRef, DefinitionValidationContext, Engine,
    LegalActionQuery, ModError, ObjId, PerceivedRef, PrincipalId, ProjectedParameter,
    ProjectedParameterKind, ProjectedParameterSchema, ProjectedTargetSchema, ProviderOfferDraft,
    Registrar, RevisionSet, Role, RulesValue, StateFamilyDescriptor, StateFamilyHandle,
    StateFamilyId, SubjectKey, perm,
};
use mush_ruleset::{SheetDoc, SheetSchema, SheetStore};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::condition::{Damage, DamageReport};
use crate::schema::{CONDITION_PHYSICAL, CONDITION_STUN, PHYSICAL_OVERFLOW};

pub(crate) const DAMAGE_ACTION_ID: &str = "condition_damage";
pub(crate) const HEAL_ACTION_ID: &str = "condition_heal";
pub(crate) const OUTCOME_ID: &str = "condition_result";
pub(crate) const STATE_FAMILY: &str = "condition_state";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConditionState {
    physical_filled: u16,
    stun_filled: u16,
    overflow: u16,
    pub(crate) revision: u64,
}

impl ConditionState {
    fn from_sheet(sheet: &SheetDoc, revision: u64) -> Result<Self, ActionError> {
        let physical_filled = u16::try_from(crate::condition::filled(sheet, CONDITION_PHYSICAL))
            .map_err(|_| {
                ConditionProvider::error(
                    ActionErrorCode::BoundsExceeded,
                    "Shadowrun physical track exceeds authoritative bounds",
                )
            })?;
        let stun_filled =
            u16::try_from(crate::condition::filled(sheet, CONDITION_STUN)).map_err(|_| {
                ConditionProvider::error(
                    ActionErrorCode::BoundsExceeded,
                    "Shadowrun stun track exceeds authoritative bounds",
                )
            })?;
        let overflow = sheet.number(PHYSICAL_OVERFLOW).unwrap_or(0);
        let overflow = u16::try_from(overflow).map_err(|_| {
            ConditionProvider::error(
                ActionErrorCode::BoundsExceeded,
                "Shadowrun physical overflow exceeds authoritative bounds",
            )
        })?;
        Ok(Self {
            physical_filled,
            stun_filled,
            overflow,
            revision,
        })
    }

    pub(crate) fn apply_to_sheet(&self, sheet: &mut SheetDoc) {
        crate::condition::set_filled(
            sheet,
            CONDITION_PHYSICAL,
            usize::from(self.physical_filled),
            Damage::Physical.mark(),
        );
        crate::condition::set_filled(
            sheet,
            CONDITION_STUN,
            usize::from(self.stun_filled),
            Damage::Stun.mark(),
        );
        sheet.set_number(PHYSICAL_OVERFLOW, i64::from(self.overflow));
    }
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
            return Err("unknown Shadowrun authoritative definition".to_owned());
        };
        if definition.payload() != &shape.payload
            || definition.references() != shape.references
            || definition.is_current() != shape.current
        {
            return Err("Shadowrun authoritative definition shape mismatch".to_owned());
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Operation {
    Damage,
    Heal,
}

impl Operation {
    const fn action_id(self) -> &'static str {
        match self {
            Self::Damage => DAMAGE_ACTION_ID,
            Self::Heal => HEAL_ACTION_ID,
        }
    }

    const fn verb(self) -> &'static str {
        match self {
            Self::Damage => "condition-damage",
            Self::Heal => "condition-heal",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Damage => "Apply Damage",
            Self::Heal => "Heal Damage",
        }
    }
}

struct ConditionProvider {
    schema: Arc<SheetSchema>,
    action: DefinitionRef,
    outcome: DefinitionRef,
    state: StateFamilyHandle<ObjId, ConditionState>,
    operation: Operation,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConditionOutcome {
    actor: u32,
    target: u32,
    operation: String,
    kind: String,
    requested: u32,
    cleared: u32,
    report: Option<DamageReport>,
    revision: u64,
    summary: Vec<String>,
}

struct ConditionRequest {
    target: ObjId,
    sheet: SheetDoc,
    seed_revision: u64,
    kind: Damage,
    amount: u32,
}

impl ConditionProvider {
    fn error(code: ActionErrorCode, message: impl Into<String>) -> ActionError {
        ActionError::new(code, false, message)
            .expect("bounded Shadowrun authoritative action error")
    }

    fn state_error(_error: &ActionStateError) -> ActionError {
        Self::error(
            ActionErrorCode::Integrity,
            "Shadowrun condition state preparation was refused",
        )
    }

    fn parse(&self, invocation: &ActionInvocation) -> Result<ConditionRequest, ActionError> {
        if !invocation.targets().is_empty() {
            return Err(Self::error(
                ActionErrorCode::InvalidRequest,
                "Condition action uses its subject and accepts no target list",
            ));
        }
        let parameters = invocation.parameters().as_object().ok_or_else(|| {
            Self::error(
                ActionErrorCode::InvalidRequest,
                "Condition action parameters must be an object",
            )
        })?;
        const FIELDS: [&str; 5] = ["amount", "kind", "seed_revision", "sheet", "target"];
        if parameters.len() != FIELDS.len()
            || parameters.keys().any(|key| !FIELDS.contains(&key.as_str()))
        {
            return Err(Self::error(
                ActionErrorCode::InvalidRequest,
                "Condition action has unexpected parameters",
            ));
        }
        let target = parameters
            .get("target")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .map(ObjId)
            .ok_or_else(|| {
                Self::error(
                    ActionErrorCode::InvalidRequest,
                    "Condition action target is invalid",
                )
            })?;
        if !matches!(invocation.subject(), PerceivedRef::KnownObject { id, .. } if *id == target) {
            return Err(Self::error(
                ActionErrorCode::InvalidRequest,
                "Condition action subject disagrees with its target",
            ));
        }
        let sheet: SheetDoc =
            serde_json::from_value(parameters.get("sheet").cloned().ok_or_else(|| {
                Self::error(
                    ActionErrorCode::InvalidRequest,
                    "Condition action lacks a sheet",
                )
            })?)
            .map_err(|_| {
                Self::error(
                    ActionErrorCode::InvalidRequest,
                    "Condition action sheet is malformed",
                )
            })?;
        let seed_revision = parameters
            .get("seed_revision")
            .and_then(Value::as_u64)
            .filter(|revision| *revision > 0)
            .ok_or_else(|| {
                Self::error(
                    ActionErrorCode::InvalidRequest,
                    "Condition action seed revision is invalid",
                )
            })?;
        let kind = match parameters.get("kind").and_then(Value::as_str) {
            Some("physical") => Damage::Physical,
            Some("stun") => Damage::Stun,
            _ => {
                return Err(Self::error(
                    ActionErrorCode::InvalidRequest,
                    "Condition action kind must be physical or stun",
                ));
            }
        };
        let amount = parameters
            .get("amount")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| (1..=100).contains(value))
            .ok_or_else(|| {
                Self::error(
                    ActionErrorCode::InvalidRequest,
                    "Condition action amount must be 1 through 100",
                )
            })?;
        Ok(ConditionRequest {
            target,
            sheet,
            seed_revision,
            kind,
            amount,
        })
    }
}

impl ActionProvider for ConditionProvider {
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
        let prior = engine
            .action_state()
            .get(&self.state, target)
            .map_err(|error| Self::state_error(&error))?;
        let Some(mut sheet) = SheetStore::new(Arc::clone(&self.schema)).get(&engine.world, *target)
        else {
            return Ok(None);
        };
        let seed_revision = prior.map_or(1, |state| {
            state.apply_to_sheet(&mut sheet);
            state.revision
        });
        mush_ruleset::validate(&self.schema, &sheet.data).map_err(|_| {
            Self::error(
                ActionErrorCode::Integrity,
                "Shadowrun sheet failed authoritative validation",
            )
        })?;
        let definitions = engine.registry.definitions().ok_or_else(|| {
            Self::error(
                ActionErrorCode::Integrity,
                "Shadowrun definition registry is unavailable",
            )
        })?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| {
                Self::error(
                    ActionErrorCode::Integrity,
                    "Shadowrun guard registry is unavailable",
                )
            })?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| {
                Self::error(
                    ActionErrorCode::Integrity,
                    "Shadowrun action definition guard is unavailable",
                )
            })?;
        let parameters = ProjectedParameterSchema::new(vec![
            ProjectedParameter {
                id: CanonicalId::new("amount").expect("static parameter id"),
                required: true,
                kind: ProjectedParameterKind::Integer {
                    minimum: 1,
                    maximum: 100,
                },
            },
            ProjectedParameter {
                id: CanonicalId::new("kind").expect("static parameter id"),
                required: true,
                kind: ProjectedParameterKind::Choice {
                    choices: vec!["physical".to_owned(), "stun".to_owned()],
                },
            },
        ])
        .map_err(|_| {
            Self::error(
                ActionErrorCode::Integrity,
                "Shadowrun condition parameter schema is invalid",
            )
        })?;
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({
                "target": target.0,
                "sheet": sheet,
                "seed_revision": seed_revision,
            }),
            parameters,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: ActionPresentation::new(
                self.operation.verb(),
                self.operation.label(),
                "Apply a validated Shadowrun Condition Monitor change.",
            )
            .map_err(|_| {
                Self::error(
                    ActionErrorCode::Integrity,
                    "Shadowrun condition presentation is invalid",
                )
            })?,
            guards: guards.finish().map_err(|_| {
                Self::error(
                    ActionErrorCode::Integrity,
                    "Shadowrun condition guards are invalid",
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
                "Shadowrun condition receipt carries the wrong durable schema",
            ));
        }
        let decoded: ConditionOutcome =
            serde_json::from_value(outcome.data().clone()).map_err(|_| {
                Self::error(
                    ActionErrorCode::Integrity,
                    "Shadowrun condition receipt outcome is malformed",
                )
            })?;
        if viewer
            != &(PrincipalId::Object {
                id: ObjId(decoded.actor),
            })
            || decoded.operation != self.operation.verb()
            || decoded.revision == 0
            || decoded.summary.len() != 3
        {
            return Err(Self::error(
                ActionErrorCode::Integrity,
                "Shadowrun condition receipt disagrees with its eligible viewer",
            ));
        }
        let summary = match self.operation {
            Operation::Damage => format!(
                "Applied {} {} damage to #{}.",
                decoded.requested, decoded.kind, decoded.target
            ),
            Operation::Heal => format!(
                "Cleared {} {} box(es) from #{}.",
                decoded.cleared, decoded.kind, decoded.target
            ),
        };
        mush_proto::ProjectedActionResult::new(
            self.operation.action_id(),
            summary,
            json!({
                "target": decoded.target,
                "kind": decoded.kind,
                "requested": decoded.requested,
                "cleared": decoded.cleared,
                "report": decoded.report,
                "revision": decoded.revision,
                "summary": decoded.summary,
            }),
        )
        .map(Some)
        .map_err(|_| {
            Self::error(
                ActionErrorCode::BoundsExceeded,
                "Shadowrun condition receipt exceeds public bounds",
            )
        })
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let request = self.parse(invocation)?;
        mush_ruleset::validate(&self.schema, &request.sheet.data).map_err(|_| {
            Self::error(
                ActionErrorCode::Integrity,
                "Shadowrun condition sheet failed schema validation",
            )
        })?;
        let prior = txn
            .get(&self.state, &request.target)
            .map_err(|error| Self::state_error(&error))?
            .cloned();
        let mut sheet = request.sheet;
        if let Some(state) = &prior {
            state.apply_to_sheet(&mut sheet);
        }
        let revision = prior
            .as_ref()
            .map_or(Ok(request.seed_revision), |state| {
                state.revision.checked_add(1).ok_or(())
            })
            .map_err(|()| {
                Self::error(
                    ActionErrorCode::BoundsExceeded,
                    "Shadowrun condition revision is exhausted",
                )
            })?;
        let (cleared, report) = match self.operation {
            Operation::Damage => (
                0,
                Some(crate::condition::apply(
                    &mut sheet,
                    request.kind,
                    request.amount,
                )),
            ),
            Operation::Heal => (
                crate::condition::heal(&mut sheet, request.kind, request.amount),
                None,
            ),
        };
        mush_ruleset::validate(&self.schema, &sheet.data).map_err(|_| {
            Self::error(
                ActionErrorCode::Integrity,
                "Shadowrun condition result failed schema validation",
            )
        })?;
        let summary = crate::condition::summary(&sheet);
        let next = ConditionState::from_sheet(&sheet, revision)?;
        if prior.is_some() {
            txn.replace(&self.state, &request.target, next)
        } else {
            txn.insert(&self.state, &request.target, next)
        }
        .map_err(|error| Self::state_error(&error))?;
        let kind = request.kind.word().to_ascii_lowercase();
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "actor": invocation.actor().0,
                "target": request.target.0,
                "operation": self.operation.verb(),
                "kind": kind,
                "requested": request.amount,
                "cleared": cleared,
                "report": report,
                "revision": revision,
                "summary": summary,
            }),
        )
        .map_err(|_| {
            Self::error(
                ActionErrorCode::BoundsExceeded,
                "Shadowrun condition outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

pub(crate) fn register(
    reg: &mut Registrar<'_>,
    schema: Arc<SheetSchema>,
) -> Result<StateFamilyHandle<ObjId, ConditionState>, ModError> {
    let state = reg.state_family(
        StateFamilyDescriptor::new(
            StateFamilyId::new(STATE_FAMILY).map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("obj_id.decimal.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("shadowrun.condition_state.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            1,
            CanonicalId::new("shadowrun.condition_state.decode_json.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("shadowrun.condition_state.validate.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
            CanonicalId::new("shadowrun.condition_state.json.v1")
                .map_err(|error| ModError::msg(error.to_string()))?,
        )
        .map_err(|error| ModError::msg(error.to_string()))?,
        |key: &ObjId| Ok(key.0.to_string()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|error| error.to_string()),
        move |value: &ConditionState| {
            if value.revision == 0 {
                return Err("Shadowrun condition revision must be non-zero".to_owned());
            }
            if value.physical_filled > 1_024 || value.stun_filled > 1_024 || value.overflow > 1_024
            {
                return Err("Shadowrun condition fill exceeds authoritative bounds".to_owned());
            }
            Ok(())
        },
        |value: &ConditionState| serde_json::to_value(value).map_err(|error| error.to_string()),
    )?;
    let outcome_descriptor = descriptor(
        "outcome",
        OUTCOME_ID,
        json!({
            "fields": [
                "actor", "target", "operation", "kind", "requested", "cleared",
                "report", "revision", "summary"
            ],
        }),
    )?;
    reg.definition_validator(
        DefinitionKind::new("outcome").map_err(|error| ModError::msg(error.to_string()))?,
        exact_validator(std::slice::from_ref(&outcome_descriptor)),
    )?;
    let outcome = reg.definition(outcome_descriptor)?;
    let damage_descriptor = descriptor(
        "action",
        DAMAGE_ACTION_ID,
        json!({
            "subject": "known_object",
            "operation": "damage",
            "state_family": STATE_FAMILY,
        }),
    )?
    .with_references(vec![outcome.clone()]);
    let heal_descriptor = descriptor(
        "action",
        HEAL_ACTION_ID,
        json!({
            "subject": "known_object",
            "operation": "heal",
            "state_family": STATE_FAMILY,
        }),
    )?
    .with_references(vec![outcome.clone()]);
    reg.definition_validator(
        DefinitionKind::new("action").map_err(|error| ModError::msg(error.to_string()))?,
        exact_validator(&[damage_descriptor.clone(), heal_descriptor.clone()]),
    )?;
    let damage = reg.definition(damage_descriptor)?;
    let heal = reg.definition(heal_descriptor)?;
    reg.action_provider(
        damage.clone(),
        ConditionProvider {
            schema: Arc::clone(&schema),
            action: damage,
            outcome: outcome.clone(),
            state: state.clone(),
            operation: Operation::Damage,
        },
    )?;
    reg.action_provider(
        heal.clone(),
        ConditionProvider {
            schema,
            action: heal,
            outcome,
            state: state.clone(),
            operation: Operation::Heal,
        },
    )?;
    Ok(state)
}

/// The sheet subject a `condition_state` record key names (interface plan
/// I13): the family's key codec is the decimal `ObjId`, so the vitals republish
/// can map an announced key back to the character whose Condition Monitor
/// changed. A foreign family is nobody's business here.
pub(crate) fn state_subject(family: &StateFamilyId, key: &str) -> Option<ObjId> {
    if family.as_str() != STATE_FAMILY {
        return None;
    }
    key.parse::<u32>().ok().map(ObjId)
}

#[cfg(test)]
mod i13_tests {
    use super::*;

    #[test]
    fn state_subject_decodes_only_this_familys_decimal_keys() {
        let ours = StateFamilyId::new(STATE_FAMILY).unwrap();
        assert_eq!(state_subject(&ours, "12"), Some(ObjId(12)));
        assert_eq!(state_subject(&ours, "not-a-key"), None);
        let foreign = StateFamilyId::new("roll_state").unwrap();
        assert_eq!(state_subject(&foreign, "12"), None);
    }
}

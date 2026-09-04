//! Authoritative private-contact lifecycle for the rules-neutral tactics layer.
//!
//! Sensor and campaign policy decide what was observed, who may receive it,
//! and when it is lost. These internal providers only commit those decisions
//! through the shared checked contact state machine. No action is projected as
//! a public legal action until a trusted sensor-policy ingress exists.

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, ActionStateError, ActionTxn,
    AuthoritativeActionOutput, CanonicalHash, CanonicalId, ClockStamp, ContactId, ContactState,
    ContactStateDraft, ContactStateError, DefinitionDescriptor, DefinitionId, DefinitionKind,
    DefinitionRef, ModError, PerceivedRef, PrincipalId, Registrar, RulesValue, StateFamilyHandle,
    SubjectKey,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::module::CONTACT_STATE_FAMILY;
#[cfg(test)]
use crate::module::NS;

pub const CONTACT_OBSERVE_ACTION_ID: &str = "contact_observe";
pub const CONTACT_REFRESH_ACTION_ID: &str = "contact_refresh";
pub const CONTACT_SHARE_ACTION_ID: &str = "contact_share";
pub const CONTACT_UNSHARE_ACTION_ID: &str = "contact_unshare";
pub const CONTACT_LOSS_ACTION_ID: &str = "contact_loss";
pub const CONTACT_REACQUIRE_ACTION_ID: &str = "contact_reacquire";
pub const CONTACT_CHANGED_OUTCOME_ID: &str = "contact_changed";

const MAX_CONTACT_ID_INPUT_BYTES: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContactOperation {
    Observe,
    Refresh,
    Share,
    Unshare,
    Loss,
    Reacquire,
}

impl ContactOperation {
    const fn action_id(self) -> &'static str {
        match self {
            Self::Observe => CONTACT_OBSERVE_ACTION_ID,
            Self::Refresh => CONTACT_REFRESH_ACTION_ID,
            Self::Share => CONTACT_SHARE_ACTION_ID,
            Self::Unshare => CONTACT_UNSHARE_ACTION_ID,
            Self::Loss => CONTACT_LOSS_ACTION_ID,
            Self::Reacquire => CONTACT_REACQUIRE_ACTION_ID,
        }
    }

    const fn outcome_name(self) -> &'static str {
        match self {
            Self::Observe => "observed",
            Self::Refresh => "refreshed",
            Self::Share => "shared",
            Self::Unshare => "unshared",
            Self::Loss => "lost",
            Self::Reacquire => "reacquired",
        }
    }

    const fn all() -> [Self; 6] {
        [
            Self::Observe,
            Self::Refresh,
            Self::Share,
            Self::Unshare,
            Self::Loss,
            Self::Reacquire,
        ]
    }
}

fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(|error| ModError::msg(error.to_string()))?,
        DefinitionId::new(id).map_err(|error| ModError::msg(error.to_string()))?,
        1,
        payload,
    )
    .map_err(|error| ModError::msg(error.to_string()))
}

pub(crate) fn outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    Ok(descriptor(
        "outcome",
        CONTACT_CHANGED_OUTCOME_ID,
        json!({
            "fields": {
                "operation": "observed|refreshed|shared|unshared|lost|reacquired",
                "contact": "contact_id",
                "generation": "u64",
                "revision": "u64",
                "active": "bool"
            },
            "contains_truth_subject": false
        }),
    )?
    .as_current())
}

pub(crate) fn action_descriptors(
    outcome: &DefinitionRef,
) -> Result<Vec<(ContactOperation, DefinitionDescriptor)>, ModError> {
    ContactOperation::all()
        .into_iter()
        .map(|operation| {
            descriptor(
                "action",
                operation.action_id(),
                json!({
                    "internal": true,
                    "operation": operation.outcome_name(),
                    "subject": "self_actor",
                    "targets": 0,
                    "state_family": CONTACT_STATE_FAMILY
                }),
            )
            .map(|descriptor| {
                (
                    operation,
                    descriptor
                        .with_references(vec![outcome.clone()])
                        .as_current(),
                )
            })
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObserveParameters {
    truth_subject: SubjectKey,
    audience: Vec<PrincipalId>,
    observed_at: ClockStamp,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RefreshParameters {
    contact: ContactId,
    expected_revision: u64,
    observed_at: ClockStamp,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AudienceParameters {
    contact: ContactId,
    expected_revision: u64,
    principal: PrincipalId,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LossParameters {
    contact: ContactId,
    expected_revision: u64,
    lost_at: ClockStamp,
    reason: CanonicalId,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReacquireParameters {
    contact: ContactId,
    expected_revision: u64,
    observed_at: ClockStamp,
}

#[derive(Clone)]
struct ContactProvider {
    operation: ContactOperation,
    contacts: StateFamilyHandle<ContactId, ContactState>,
    outcome: DefinitionRef,
}

impl ContactProvider {
    fn invalid(message: &'static str) -> ActionError {
        ActionError::new(ActionErrorCode::InvalidRequest, false, message)
            .expect("static contact request error is bounded")
    }

    fn stale(message: &'static str) -> ActionError {
        ActionError::new(ActionErrorCode::Stale, false, message)
            .expect("static contact stale error is bounded")
    }

    fn integrity(message: &'static str) -> ActionError {
        ActionError::new(ActionErrorCode::Integrity, false, message)
            .expect("static contact integrity error is bounded")
    }

    fn state_error(_error: &ActionStateError) -> ActionError {
        Self::integrity("tactical contact authoritative state is unavailable")
    }

    fn transition_error(error: &ContactStateError) -> ActionError {
        let code = match error {
            ContactStateError::StaleRevision { .. }
            | ContactStateError::NotActive
            | ContactStateError::AlreadyActive
            | ContactStateError::AlreadyShared
            | ContactStateError::NotShared
            | ContactStateError::LastAudience => ActionErrorCode::Stale,
            ContactStateError::AudienceLimit { .. }
            | ContactStateError::GenerationExhausted
            | ContactStateError::RevisionExhausted => ActionErrorCode::BoundsExceeded,
            ContactStateError::ObservationRegressed | ContactStateError::Clock(_) => {
                ActionErrorCode::InvalidRequest
            }
            ContactStateError::UnsupportedSchema { .. }
            | ContactStateError::ZeroCounter
            | ContactStateError::RevisionBehindGeneration { .. }
            | ContactStateError::NonCanonicalAudience
            | ContactStateError::Encoding(_) => ActionErrorCode::Integrity,
        };
        ActionError::new(code, false, "tactical contact transition was refused")
            .expect("static contact transition error is bounded")
    }

    fn decode<T: for<'de> Deserialize<'de>>(
        invocation: &ActionInvocation,
    ) -> Result<T, ActionError> {
        serde_json::from_value(invocation.parameters().clone())
            .map_err(|_| Self::invalid("tactical contact parameters are malformed"))
    }

    fn validate_shape(invocation: &ActionInvocation) -> Result<(), ActionError> {
        if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
            return Err(Self::invalid(
                "tactical contact actions require self and no targets",
            ));
        }
        Ok(())
    }

    fn validate(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        Self::validate_shape(invocation)?;
        match self.operation {
            ContactOperation::Observe => {
                let _: ObserveParameters = Self::decode(invocation)?;
            }
            ContactOperation::Refresh => {
                let _: RefreshParameters = Self::decode(invocation)?;
            }
            ContactOperation::Share | ContactOperation::Unshare => {
                let _: AudienceParameters = Self::decode(invocation)?;
            }
            ContactOperation::Loss => {
                let _: LossParameters = Self::decode(invocation)?;
            }
            ContactOperation::Reacquire => {
                let _: ReacquireParameters = Self::decode(invocation)?;
            }
        }
        Ok(())
    }

    fn derive_id(&self, invocation: &ActionInvocation) -> Result<ContactId, ActionError> {
        let digest = CanonicalHash::for_canonical_value(
            "tactics.contact_id.v1",
            1,
            &json!({
                "owner": self.contacts.owner(),
                "idempotency": invocation.idempotency(),
            }),
            MAX_CONTACT_ID_INPUT_BYTES,
        )
        .map_err(|_| Self::integrity("tactical contact identity could not be derived"))?;
        let mut id = [0_u8; 16];
        id.copy_from_slice(&digest.as_bytes()[..16]);
        Ok(ContactId::from_bytes(id))
    }

    fn read(&self, txn: &mut ActionTxn<'_>, id: ContactId) -> Result<ContactState, ActionError> {
        txn.get(&self.contacts, &id)
            .map_err(|error| Self::state_error(&error))?
            .cloned()
            .ok_or_else(|| Self::stale("tactical contact no longer exists"))
    }

    fn replace(
        &self,
        txn: &mut ActionTxn<'_>,
        id: ContactId,
        next: ContactState,
    ) -> Result<(), ActionError> {
        txn.replace(&self.contacts, &id, next)
            .map_err(|error| Self::state_error(&error))
    }

    fn witnesses(
        invocation: &ActionInvocation,
        before: Option<&ContactState>,
        after: &ContactState,
    ) -> Vec<PrincipalId> {
        let mut witnesses = vec![invocation.principal().clone()];
        if let Some(before) = before {
            witnesses.extend(before.audience().iter().cloned());
        }
        witnesses.extend(after.audience().iter().cloned());
        witnesses.sort();
        witnesses.dedup();
        witnesses
    }

    fn output(
        &self,
        invocation: &ActionInvocation,
        id: ContactId,
        before: Option<&ContactState>,
        after: &ContactState,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({
                "operation": self.operation.outcome_name(),
                "contact": id,
                "generation": after.generation(),
                "revision": after.revision(),
                "active": after.is_active(),
            }),
        )
        .map_err(|_| Self::integrity("tactical contact outcome is invalid"))?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(Self::witnesses(invocation, before, after)))
    }

    fn prepare_observe(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let parameters: ObserveParameters = Self::decode(invocation)?;
        let id = self.derive_id(invocation)?;
        if txn
            .get(&self.contacts, &id)
            .map_err(|error| Self::state_error(&error))?
            .is_some()
        {
            return Err(Self::integrity(
                "derived tactical contact identity is already occupied",
            ));
        }
        let next = ContactState::new(ContactStateDraft {
            truth_subject: parameters.truth_subject,
            audience: parameters.audience,
            observed_at: parameters.observed_at,
        })
        .map_err(|error| Self::transition_error(&error))?;
        txn.insert(&self.contacts, &id, next.clone())
            .map_err(|error| Self::state_error(&error))?;
        self.output(invocation, id, None, &next)
    }

    fn prepare_refresh(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let parameters: RefreshParameters = Self::decode(invocation)?;
        let prior = self.read(txn, parameters.contact)?;
        let next = prior
            .checked_refresh(parameters.expected_revision, parameters.observed_at)
            .map_err(|error| Self::transition_error(&error))?;
        self.replace(txn, parameters.contact, next.clone())?;
        self.output(invocation, parameters.contact, Some(&prior), &next)
    }

    fn prepare_audience(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        share: bool,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let parameters: AudienceParameters = Self::decode(invocation)?;
        let prior = self.read(txn, parameters.contact)?;
        let next = if share {
            prior.checked_share(parameters.expected_revision, parameters.principal)
        } else {
            prior.checked_unshare(parameters.expected_revision, &parameters.principal)
        }
        .map_err(|error| Self::transition_error(&error))?;
        self.replace(txn, parameters.contact, next.clone())?;
        self.output(invocation, parameters.contact, Some(&prior), &next)
    }

    fn prepare_loss(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let parameters: LossParameters = Self::decode(invocation)?;
        let prior = self.read(txn, parameters.contact)?;
        let next = prior
            .checked_mark_lost(
                parameters.expected_revision,
                parameters.lost_at,
                parameters.reason,
            )
            .map_err(|error| Self::transition_error(&error))?;
        self.replace(txn, parameters.contact, next.clone())?;
        self.output(invocation, parameters.contact, Some(&prior), &next)
    }

    fn prepare_reacquire(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let parameters: ReacquireParameters = Self::decode(invocation)?;
        let prior = self.read(txn, parameters.contact)?;
        let next = prior
            .checked_reacquire(parameters.expected_revision, parameters.observed_at)
            .map_err(|error| Self::transition_error(&error))?;
        self.replace(txn, parameters.contact, next.clone())?;
        self.output(invocation, parameters.contact, Some(&prior), &next)
    }
}

impl ActionProvider for ContactProvider {
    fn validate_invocation(&self, invocation: &ActionInvocation) -> Result<(), ActionError> {
        self.validate(invocation)
    }

    fn prepare_invocation(
        &self,
        _invocation: &ActionInvocation,
        _txn: &mut ActionTxn<'_>,
    ) -> Result<(), ActionError> {
        Err(ActionError::new(
            ActionErrorCode::Unavailable,
            false,
            "tactical contact changes require the authoritative action coordinator",
        )
        .expect("static contact migration error is bounded"))
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.validate(invocation)?;
        match self.operation {
            ContactOperation::Observe => self.prepare_observe(invocation, txn),
            ContactOperation::Refresh => self.prepare_refresh(invocation, txn),
            ContactOperation::Share => self.prepare_audience(invocation, txn, true),
            ContactOperation::Unshare => self.prepare_audience(invocation, txn, false),
            ContactOperation::Loss => self.prepare_loss(invocation, txn),
            ContactOperation::Reacquire => self.prepare_reacquire(invocation, txn),
        }
    }
}

pub(crate) fn register_providers(
    reg: &mut Registrar<'_>,
    contacts: StateFamilyHandle<ContactId, ContactState>,
    outcome: DefinitionRef,
    actions: Vec<(ContactOperation, DefinitionRef)>,
) -> Result<(), ModError> {
    for (operation, action) in actions {
        reg.action_provider(
            action,
            ContactProvider {
                operation,
                contacts: contacts.clone(),
                outcome: outcome.clone(),
            },
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{
        ActionInvocationDraft, ActionOfferId, CommitToken, IdempotencyKey, IdempotencyScope, ObjId,
        Registry, RequestUid, ScopedIdempotencyKey, World,
    };

    use crate::TacticsModule;

    fn definition_refs() -> Vec<(ContactOperation, DefinitionRef)> {
        let owner = mush_core::OwnerId::new(NS).unwrap();
        let mut definitions = mush_core::DefinitionRegistryBuilder::new();
        let outcome = definitions
            .register(owner.clone(), outcome_descriptor().unwrap())
            .unwrap();
        action_descriptors(&outcome)
            .unwrap()
            .into_iter()
            .map(|(operation, descriptor)| {
                (
                    operation,
                    definitions.register(owner.clone(), descriptor).unwrap(),
                )
            })
            .collect()
    }

    fn action(
        actions: &[(ContactOperation, DefinitionRef)],
        op: ContactOperation,
    ) -> &DefinitionRef {
        &actions
            .iter()
            .find(|(operation, _)| *operation == op)
            .unwrap()
            .1
    }

    fn invocation(
        action: &DefinitionRef,
        request: RequestUid,
        parameters: Value,
    ) -> ActionInvocation {
        let principal = PrincipalId::Object { id: ObjId(1) };
        ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor: ObjId(1),
            offered_action: ActionOfferId(CanonicalHash::from_bytes([7; 32])),
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: action.key().owner.clone(),
                    namespace: CanonicalId::new(action.key().id.as_str()).unwrap(),
                },
                1,
                IdempotencyKey::Request(request),
            ),
        })
        .unwrap()
    }

    fn prepare_install(
        engine: &mut mush_core::Engine,
        action: &DefinitionRef,
        invocation: &ActionInvocation,
        token: u8,
    ) -> (ContactId, Value) {
        let (prepared, output) = engine
            .prepare_registered_authoritative_action(
                action,
                invocation,
                CommitToken::from_bytes([token; 16]),
            )
            .unwrap();
        assert_eq!(prepared.operation_count(), 1);
        let (outcome, _, _, _, _, _) = output.into_parts();
        let outcome = outcome.data();
        assert!(outcome.get("truth_subject").is_none());
        let contact: ContactId = serde_json::from_value(outcome["contact"].clone()).unwrap();
        let (ops, installation) = prepared.into_parts();
        let state = ops
            .iter()
            .find_map(|op| match &op.op {
                mush_core::ModuleWrite::JsonSet { key, value } if key.contains("/contacts/") => {
                    value.get("value").cloned()
                }
                _ => None,
            })
            .unwrap();
        engine.install_prepared_action(installation);
        (contact, state)
    }

    #[test]
    fn complete_contact_lifecycle_is_atomic_and_generation_safe() {
        let mut engine = mush_core::Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![Box::new(TacticsModule::new())],
        )
        .unwrap();
        let actions = definition_refs();
        let observe_request = RequestUid::from_bytes([1; 16]);
        let observe = invocation(
            action(&actions, ContactOperation::Observe),
            observe_request,
            json!({
                "truth_subject": { "kind": "object", "id": 90 },
                "audience": [
                    { "kind": "object", "id": 2 },
                    { "kind": "object", "id": 1 }
                ],
                "observed_at": { "clock": { "kind": "utc" }, "at": 10 }
            }),
        );
        let error = engine
            .prepare_registered_action(action(&actions, ContactOperation::Observe), &observe)
            .expect_err("contact lifecycle must not bypass the authoritative coordinator");
        assert!(matches!(
            error,
            mush_core::ActionProviderPrepareError::Provider { source, .. }
                if source.code() == ActionErrorCode::Unavailable
        ));
        let (contact, state) = prepare_install(
            &mut engine,
            action(&actions, ContactOperation::Observe),
            &observe,
            1,
        );
        assert_eq!(state["generation"], 1);
        assert_eq!(state["revision"], 1);
        assert_eq!(state["audience"][0]["id"], 1);

        let refresh = invocation(
            action(&actions, ContactOperation::Refresh),
            RequestUid::from_bytes([2; 16]),
            json!({
                "contact": contact,
                "expected_revision": 1,
                "observed_at": { "clock": { "kind": "utc" }, "at": 11 }
            }),
        );
        let (_, state) = prepare_install(
            &mut engine,
            action(&actions, ContactOperation::Refresh),
            &refresh,
            2,
        );
        assert_eq!(state["generation"], 1);
        assert_eq!(state["revision"], 2);

        let share = invocation(
            action(&actions, ContactOperation::Share),
            RequestUid::from_bytes([3; 16]),
            json!({
                "contact": contact,
                "expected_revision": 2,
                "principal": { "kind": "object", "id": 3 }
            }),
        );
        let (_, state) = prepare_install(
            &mut engine,
            action(&actions, ContactOperation::Share),
            &share,
            3,
        );
        assert_eq!(state["revision"], 3);
        assert_eq!(state["audience"].as_array().unwrap().len(), 3);

        let unshare = invocation(
            action(&actions, ContactOperation::Unshare),
            RequestUid::from_bytes([4; 16]),
            json!({
                "contact": contact,
                "expected_revision": 3,
                "principal": { "kind": "object", "id": 2 }
            }),
        );
        let (_, state) = prepare_install(
            &mut engine,
            action(&actions, ContactOperation::Unshare),
            &unshare,
            4,
        );
        assert_eq!(state["revision"], 4);
        assert_eq!(state["audience"].as_array().unwrap().len(), 2);

        let loss = invocation(
            action(&actions, ContactOperation::Loss),
            RequestUid::from_bytes([5; 16]),
            json!({
                "contact": contact,
                "expected_revision": 4,
                "lost_at": { "clock": { "kind": "utc" }, "at": 12 },
                "reason": "occluded"
            }),
        );
        let (_, state) = prepare_install(
            &mut engine,
            action(&actions, ContactOperation::Loss),
            &loss,
            5,
        );
        assert_eq!(state["generation"], 2);
        assert_eq!(state["revision"], 5);
        assert_eq!(state["lifecycle"]["state"], "lost");

        let reacquire = invocation(
            action(&actions, ContactOperation::Reacquire),
            RequestUid::from_bytes([6; 16]),
            json!({
                "contact": contact,
                "expected_revision": 5,
                "observed_at": { "clock": { "kind": "utc" }, "at": 13 }
            }),
        );
        let (reacquired_contact, state) = prepare_install(
            &mut engine,
            action(&actions, ContactOperation::Reacquire),
            &reacquire,
            6,
        );
        assert_eq!(reacquired_contact, contact);
        assert_eq!(state["generation"], 3);
        assert_eq!(state["revision"], 6);
        assert_eq!(state["lifecycle"]["state"], "active");

        let stale_refresh = invocation(
            action(&actions, ContactOperation::Refresh),
            RequestUid::from_bytes([7; 16]),
            json!({
                "contact": contact,
                "expected_revision": 5,
                "observed_at": { "clock": { "kind": "utc" }, "at": 14 }
            }),
        );
        let error = engine
            .prepare_registered_authoritative_action(
                action(&actions, ContactOperation::Refresh),
                &stale_refresh,
                CommitToken::from_bytes([7; 16]),
            )
            .expect_err("stale contact transition must be non-mutating");
        assert!(matches!(
            error,
            mush_core::ActionProviderPrepareError::Provider { source, .. }
                if source.code() == ActionErrorCode::Stale
        ));
        assert!(!engine.action_state().has_prepared());

        let expected = CanonicalHash::for_canonical_value(
            "tactics.contact_id.v1",
            1,
            &json!({
                "owner": mush_core::OwnerId::new(NS).unwrap(),
                "idempotency": observe.idempotency(),
            }),
            MAX_CONTACT_ID_INPUT_BYTES,
        )
        .unwrap();
        assert_eq!(contact.as_bytes(), &expected.as_bytes()[..16]);
    }
}

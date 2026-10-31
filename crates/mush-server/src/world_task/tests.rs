#![cfg(test)]

use super::*;

/// The store-less lens these projection assertions have always been taken
/// with: no live authoritative store, said out loud.
const DETACHED: TypedStateLens<'static> =
    TypedStateLens::Detached(mush_core::DetachedTypedState::Test);

/// H1: the store handle the effect-release helpers now carry (tests that
/// never launch a store read hand them an empty in-memory store).
fn test_effect_store() -> Arc<dyn ObjectStore> {
    Arc::new(InMemoryStore::new())
}
use std::collections::BTreeSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering as AtomicOrdering};

use mush_core::{
    ActionError, ActionErrorCode, ActionInvocation, ActionInvocationDraft, ActionOfferId,
    ActionPresentation, ActionProvider, ActionStateBootRecord, ArgShape, AsyncCompletion,
    AuthoritativeActionOutput, AuthoritativeBundle, AuthoritativeBundleDraft, AuthorityGuard,
    AuthorityGuardContext, AuthorityGuardKind, AuthorityGuardValidator, CanonicalHash, CanonicalId,
    ClockRef, ClockStamp, CmdError, CommandSpec, CommitToken as DurableCommitToken, ContextOutcome,
    Ctx, DecisionAdmissionIndex, DecisionCaptureFamilies, DecisionCapturePlan, DecisionCause,
    DecisionControllerId, DecisionId, DecisionKeyRing, DecisionPrompt, DecisionPromptRecord,
    DecisionPromptStateCodec, DecisionResponse, DecisionSecret, DecisionTerminalReceipt,
    DecisionTerminalReceiptStateCodec, DecisionToken, DecisionWorkerLease, Deferred,
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionRegistryBuilder,
    DurableScheduleStateCodec, EventHook, FallbackPolicyRef, GOD, GameModule, GuardSet, HookPoint,
    IdempotencyKey, IdempotencyKeyText, IdempotencyScope, IngressEnvelope, IngressPayload,
    IngressSurfaceVersion, InputContext, LIMBO, ModuleBootData, ModuleCtx, ModuleOp, ModuleWrite,
    OfferKeyRing, OfferSecret, OwnerId, PendingDecision, PendingDecisionStateCodec,
    PendingDecisionStatus, PendingOfferWitness, PerceivedRef, PreparedCausalRef, PreparedTxn,
    PrincipalId, ProjectedDecisionContext, ProjectedLegalAction, ProjectedParameterSchema,
    ProjectedTargetSchema, ProjectionScopeId, ProviderOfferDraft, QueryHandler, Registrar,
    RequestUid, RevisionKey, RevisionKind, RevisionSet, RevisionStamp, RevisionValidator,
    RulesValue, ScopedIdempotencyKey, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId,
    SubjectKey, TickHandler, WorldEventKind, WorldObserver,
    decision_prompt_state_family_descriptor, decision_terminal_receipt_state_family_descriptor,
    durable_schedule_state_family_descriptor, pending_decision_state_family_descriptor,
};
use mush_proto::{InputMode, PromptSpec};
use mush_store::{AuthoritativeOutboxAttemptResult, AuthoritativeStore};
use mush_tactics::{TacticalUnit, UnitSpec};

use mush_contract_tests::tactical_gate::{
    TacticalGateState, disable_tactical_gate_weapon, exhaust_tactical_gate_ammo,
    install_tactical_gate_contact, install_tactical_gate_fire_window,
    install_tactical_gate_placement, install_tactical_gate_unit,
    install_tactical_gate_weapons_lease, move_tactical_gate_placement,
    publish_tactical_gate_weapons_station, tactical_gate_world_with_modules,
};

use crate::decision_dispatch::{
    DEFAULT_DECISION_PROVIDER_WORKERS, DecisionProviderAdapter, DecisionProviderFailure,
    DecisionProviderFailureKind,
};

struct BlackholeGateway;

#[test]
fn operated_known_subject_requires_exact_unambiguous_grant_epoch() {
    let mut engine = Engine::new(World::new_minimal(), Registry::new());
    let movement = mush_core::ChannelId::new("movement").unwrap();
    engine
        .world
        .publish_command_subject(mush_core::ChannelGrant {
            actor: GOD,
            subject: LIMBO,
            station: SmolStr::new_static("pilot"),
            relation: SmolStr::new_static("pilot"),
            owner: SmolStr::new_static("sim"),
            priority: 10,
            channels: BTreeSet::from([movement.clone()]),
            epoch: 7,
        });
    let principal = PrincipalId::Object { id: GOD };
    let projected = PerceivedRef::KnownObject {
        id: LIMBO,
        revision: 7,
    };
    let query = legal_action_query(&engine, principal.clone(), GOD, projected.clone()).unwrap();
    assert_eq!(query.truth_subject(), &SubjectKey::Object { id: LIMBO });
    assert_eq!(query.projected_subject(), &projected);

    assert_eq!(
        legal_action_query(
            &engine,
            principal.clone(),
            GOD,
            PerceivedRef::KnownObject {
                id: LIMBO,
                revision: 6,
            },
        )
        .unwrap_err(),
        LegalActionReadError::InvalidSubject
    );

    engine
        .world
        .publish_command_subject(mush_core::ChannelGrant {
            actor: GOD,
            subject: LIMBO,
            station: SmolStr::new_static("remote"),
            relation: SmolStr::new_static("remote"),
            owner: SmolStr::new_static("sim"),
            priority: 5,
            channels: BTreeSet::from([movement]),
            epoch: 8,
        });
    assert_eq!(
        legal_action_query(&engine, principal, GOD, projected).unwrap_err(),
        LegalActionReadError::InvalidSubject
    );
}

impl GatewaySink for BlackholeGateway {
    fn transmit(&self, _sends: Vec<(SendTarget, Arc<OutputEvent>)>) {}

    fn set_flood_exempt(&self, _char: u64, _exempt: bool) {}

    fn is_socket(&self) -> bool {
        false
    }

    fn close_session(&self, _char: u64, _cause: mush_ipc::DetachCause) {}

    fn close_web_sessions(
        &self,
        _chars: Vec<u64>,
        _notice: Option<OutputEvent>,
        _cause: mush_ipc::DetachCause,
    ) {
    }

    fn close_char_connections(&self, _char: u64) -> Vec<u64> {
        Vec::new()
    }

    fn structured_conns_of(&self, _chars: &[u64]) -> Vec<(u64, u64)> {
        Vec::new()
    }

    fn detach_conn_local(&self, _conn: u64) {}

    fn session_counts(&self) -> (i64, i64) {
        (0, 0)
    }
}

const DECISION_HARNESS_OWNER: &str = "decision_harness";
const DECISION_WAKE_ADAPTER: &str = "decision_wake";

type PendingDecisionHandle = StateFamilyHandle<DecisionId, PendingDecision>;
type DecisionTerminalReceiptHandle = StateFamilyHandle<DecisionId, DecisionTerminalReceipt>;

#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct DecisionSelectionState {
    count: u64,
}

type DecisionSelectionHandle = StateFamilyHandle<CanonicalId, DecisionSelectionState>;

#[derive(Clone)]
struct DecisionHarnessFamilies {
    pending: PendingDecisionHandle,
    prompts: StateFamilyHandle<DecisionId, DecisionPromptRecord>,
    terminal_receipts: DecisionTerminalReceiptHandle,
    selections: DecisionSelectionHandle,
}

#[derive(Clone)]
struct DecisionHarnessRegistration {
    families: DecisionHarnessFamilies,
    capture_action: DefinitionRef,
    lease_action: DefinitionRef,
    capability_generation: Arc<AtomicU64>,
}

struct DecisionHarnessModule {
    registration: Arc<Mutex<Option<DecisionHarnessRegistration>>>,
    capture_plan: Arc<Mutex<Option<DecisionCapturePlan>>>,
    lease_duration: i64,
    capability_generation: Arc<AtomicU64>,
}

struct DecisionCaptureProvider {
    action: DefinitionRef,
    families: DecisionCaptureFamilies,
    plan: Arc<Mutex<Option<DecisionCapturePlan>>>,
}

struct DecisionLeaseProvider {
    action: DefinitionRef,
    selections: DecisionSelectionHandle,
    capability_generation: Arc<AtomicU64>,
}

struct DecisionCapabilityValidator {
    generation: Arc<AtomicU64>,
}

impl AuthorityGuardValidator for DecisionCapabilityValidator {
    fn is_current(
        &self,
        _context: &AuthorityGuardContext<'_>,
        guard: &AuthorityGuard,
    ) -> Result<bool, ActionError> {
        Ok(matches!(
            guard,
            AuthorityGuard::Capability {
                provider,
                capability,
                generation,
                ..
            } if provider.as_str() == DECISION_HARNESS_OWNER
                && capability.as_str() == "choose"
                && *generation == self.generation.load(AtomicOrdering::SeqCst)
        ))
    }
}

struct DecisionHarnessFallback {
    identity: CanonicalId,
}

impl mush_core::DecisionFallbackAdapter for DecisionHarnessFallback {
    fn identity(&self) -> &CanonicalId {
        &self.identity
    }

    fn select(
        &self,
        _policy: &FallbackPolicyRef,
        prompt: &DecisionPrompt,
    ) -> Result<mush_core::DecisionFallbackSelection, String> {
        let action = prompt
            .actions
            .first()
            .ok_or_else(|| "fallback prompt has no projected actions".to_owned())?;
        Ok(mush_core::DecisionFallbackSelection {
            offer: action.offer(),
            targets: Vec::new(),
            parameters: serde_json::json!({}),
        })
    }
}

fn harness_action_error(detail: impl Into<String>) -> ActionError {
    ActionError::new(ActionErrorCode::ProviderRejected, false, detail.into())
        .expect("bounded decision harness error")
}

impl ActionProvider for DecisionCaptureProvider {
    fn validate_invocation(&self, _invocation: &ActionInvocation) -> Result<(), ActionError> {
        Ok(())
    }

    fn prepare_authoritative(
        &self,
        _invocation: &ActionInvocation,
        txn: &mut mush_core::ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let plan = self
            .plan
            .lock()
            .expect("decision capture-plan mutex poisoned")
            .clone()
            .ok_or_else(|| harness_action_error("decision capture plan is not armed"))?;
        let wake = plan
            .stage(txn, &self.families)
            .map_err(|error| harness_action_error(error.to_string()))?;
        let outcome = RulesValue::new(
            self.action.clone(),
            serde_json::json!({
                "captured": true
            }),
        )
        .map_err(|error| harness_action_error(error.to_string()))?;
        Ok(AuthoritativeActionOutput::new(outcome).with_durable_outbox(vec![wake]))
    }
}

impl ActionProvider for DecisionLeaseProvider {
    fn validate_invocation(&self, _invocation: &ActionInvocation) -> Result<(), ActionError> {
        Ok(())
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut mush_core::ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        if invocation.parameters().get("decision").is_none() {
            let key = CanonicalId::new("selected").expect("static selection key");
            match txn
                .get(&self.selections, &key)
                .map_err(|error| harness_action_error(error.to_string()))?
                .cloned()
            {
                Some(mut selection) => {
                    selection.count = selection.count.saturating_add(1);
                    txn.replace(&self.selections, &key, selection)
                        .map_err(|error| harness_action_error(error.to_string()))?;
                }
                None => txn
                    .insert(&self.selections, &key, DecisionSelectionState { count: 1 })
                    .map_err(|error| harness_action_error(error.to_string()))?,
            }
        }
        let outcome = RulesValue::new(
            self.action.clone(),
            serde_json::json!({
                "leased": true
            }),
        )
        .map_err(|error| harness_action_error(error.to_string()))?;
        Ok(AuthoritativeActionOutput::new(outcome))
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &Engine,
    ) -> Result<Option<ProviderOfferDraft>, ActionError> {
        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| harness_action_error("definition registry unavailable"))?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|error| harness_action_error(error.to_string()))?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|error| harness_action_error(error.to_string()))?;
        guards
            .add(AuthorityGuard::Capability {
                provider: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
                actor: query.actor(),
                capability: CanonicalId::new("choose").expect("static capability"),
                generation: self.capability_generation.load(AtomicOrdering::SeqCst),
            })
            .map_err(|error| harness_action_error(error.to_string()))?;
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: serde_json::json!({}),
            parameters: ProjectedParameterSchema::empty(),
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: ActionPresentation::new(
                "choose",
                "Choose",
                "Select the decision action.",
            )
            .map_err(|error| harness_action_error(error.to_string()))?,
            guards: guards
                .finish()
                .map_err(|error| harness_action_error(error.to_string()))?,
        }))
    }
}

impl GameModule for DecisionHarnessModule {
    fn name(&self) -> &'static str {
        DECISION_HARNESS_OWNER
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), mush_core::ModError> {
        let action_kind = DefinitionKind::new("action")
            .map_err(|error| mush_core::ModError::msg(error.to_string()))?;
        reg.definition_validator(
            action_kind.clone(),
            |context: &mush_core::DefinitionValidationContext<'_>| {
                context
                    .definition()
                    .payload()
                    .is_object()
                    .then_some(())
                    .ok_or_else(|| "decision harness action payload must be an object".to_owned())
            },
        )?;
        let capture_action = reg.definition(
            DefinitionDescriptor::new(
                action_kind.clone(),
                DefinitionId::new("capture_decision")
                    .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                1,
                serde_json::json!({}),
            )
            .map_err(|error| mush_core::ModError::msg(error.to_string()))?
            .as_current(),
        )?;
        let lease_action = reg.definition(
            DefinitionDescriptor::new(
                action_kind,
                DefinitionId::new("lease_decision")
                    .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                1,
                serde_json::json!({}),
            )
            .map_err(|error| mush_core::ModError::msg(error.to_string()))?
            .as_current(),
        )?;
        let pending = reg.state_family(
            pending_decision_state_family_descriptor(
                StateFamilyId::new("pending_decisions")
                    .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
            )
            .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
            PendingDecisionStateCodec,
            PendingDecisionStateCodec,
            PendingDecisionStateCodec,
            PendingDecisionStateCodec,
        )?;
        let prompts = reg.state_family(
            decision_prompt_state_family_descriptor(
                StateFamilyId::new("decision_prompts")
                    .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
            )
            .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
            DecisionPromptStateCodec,
            DecisionPromptStateCodec,
            DecisionPromptStateCodec,
            DecisionPromptStateCodec,
        )?;
        let deadlines = reg.state_family(
            durable_schedule_state_family_descriptor(
                StateFamilyId::new("decision_deadlines")
                    .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
            )
            .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
            DurableScheduleStateCodec,
            DurableScheduleStateCodec,
            DurableScheduleStateCodec,
            DurableScheduleStateCodec,
        )?;
        let terminal_receipts = reg.state_family(
            decision_terminal_receipt_state_family_descriptor(
                StateFamilyId::new("decision_terminal_receipts")
                    .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
            )
            .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
            DecisionTerminalReceiptStateCodec,
            DecisionTerminalReceiptStateCodec,
            DecisionTerminalReceiptStateCodec,
            DecisionTerminalReceiptStateCodec,
        )?;
        let selections = reg.state_family(
            StateFamilyDescriptor::new(
                StateFamilyId::new("decision_selections")
                    .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                CanonicalId::new("canonical_id.v1")
                    .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                CanonicalId::new("decision_selection.v1")
                    .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                1,
                CanonicalId::new("decision_selection_decode.v1")
                    .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                CanonicalId::new("decision_selection_validate.v1")
                    .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
                CanonicalId::new("decision_selection_serialize.v1")
                    .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
            )
            .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
            |key: &CanonicalId| Ok(key.to_string()),
            |value: &serde_json::Value| {
                serde_json::from_value(value.clone()).map_err(|error| error.to_string())
            },
            |_value: &DecisionSelectionState| Ok(()),
            |value: &DecisionSelectionState| Ok(serde_json::json!({ "count": value.count })),
        )?;
        let capture = DecisionCaptureFamilies::new(
            pending.clone(),
            prompts.clone(),
            deadlines.clone(),
            terminal_receipts.clone(),
        )
        .map_err(|error| mush_core::ModError::msg(error.to_string()))?;
        reg.authority_guard_validator(
            AuthorityGuardKind::Capability,
            DecisionCapabilityValidator {
                generation: Arc::clone(&self.capability_generation),
            },
        )?;
        reg.action_provider(
            capture_action.clone(),
            DecisionCaptureProvider {
                action: capture_action.clone(),
                families: capture.clone(),
                plan: Arc::clone(&self.capture_plan),
            },
        )?;
        reg.action_provider(
            lease_action.clone(),
            DecisionLeaseProvider {
                action: lease_action.clone(),
                selections: selections.clone(),
                capability_generation: Arc::clone(&self.capability_generation),
            },
        )?;
        reg.decision_source(
            lease_action.clone(),
            capture.clone(),
            CanonicalId::new(DECISION_WAKE_ADAPTER)
                .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
            self.lease_duration,
            DecisionHarnessFallback {
                identity: CanonicalId::new("decision_harness_fallback.v1")
                    .map_err(|error| mush_core::ModError::msg(error.to_string()))?,
            },
        )?;
        *self
            .registration
            .lock()
            .expect("decision registration mutex poisoned") = Some(DecisionHarnessRegistration {
            families: DecisionHarnessFamilies {
                pending,
                prompts,
                terminal_receipts,
                selections,
            },
            capture_action,
            lease_action,
            capability_generation: Arc::clone(&self.capability_generation),
        });
        Ok(())
    }
}

struct RecordingDecisionProvider {
    owner: OwnerId,
    calls: Arc<AtomicUsize>,
    called: Arc<tokio::sync::Notify>,
}

impl DecisionProviderAdapter for RecordingDecisionProvider {
    fn owner(&self) -> &OwnerId {
        &self.owner
    }

    fn decide(
        &self,
        lease: DecisionWorkerLease,
    ) -> futures::future::BoxFuture<'static, Result<DecisionResponse, DecisionProviderFailure>>
    {
        self.calls.fetch_add(1, AtomicOrdering::SeqCst);
        self.called.notify_waiters();
        Box::pin(async move {
            Ok(DecisionResponse {
                token: lease.prompt().token,
                offer: lease.prompt().actions[0].offer(),
                targets: Vec::new(),
                parameters: serde_json::json!({}),
            })
        })
    }
}

struct BlockingDecisionProvider {
    owner: OwnerId,
    calls: Arc<AtomicUsize>,
    called: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Semaphore>,
}

struct FailingDecisionProvider {
    owner: OwnerId,
    calls: Arc<AtomicUsize>,
    called: Arc<tokio::sync::Notify>,
    kind: DecisionProviderFailureKind,
}

#[derive(Clone, Copy, Debug)]
enum ProjectedPolicyKind {
    Deterministic,
    DecisionTree,
    MockLlm,
}

struct ProjectedPolicyProvider {
    owner: OwnerId,
    kind: ProjectedPolicyKind,
}

impl DecisionProviderAdapter for ProjectedPolicyProvider {
    fn owner(&self) -> &OwnerId {
        &self.owner
    }

    fn decide(
        &self,
        lease: DecisionWorkerLease,
    ) -> futures::future::BoxFuture<'static, Result<DecisionResponse, DecisionProviderFailure>>
    {
        let kind = self.kind;
        Box::pin(async move {
            let action = match kind {
                ProjectedPolicyKind::Deterministic => lease.prompt().actions.first().cloned(),
                ProjectedPolicyKind::DecisionTree => {
                    tokio::task::yield_now().await;
                    lease
                        .prompt()
                        .actions
                        .iter()
                        .find(|action| action.presentation().verb == "choose")
                        .cloned()
                }
                ProjectedPolicyKind::MockLlm => {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    let wire = serde_json::to_vec(&lease.prompt().actions).map_err(|error| {
                        DecisionProviderFailure::new(
                            DecisionProviderFailureKind::Malformed,
                            error.to_string(),
                        )
                    })?;
                    serde_json::from_slice::<Vec<ProjectedLegalAction>>(&wire)
                        .map_err(|error| {
                            DecisionProviderFailure::new(
                                DecisionProviderFailureKind::Malformed,
                                error.to_string(),
                            )
                        })?
                        .into_iter()
                        .find(|action| action.presentation().verb == "choose")
                }
            }
            .ok_or_else(|| {
                DecisionProviderFailure::new(
                    DecisionProviderFailureKind::Permanent,
                    "projected policy found no admissible action",
                )
            })?;
            Ok(DecisionResponse {
                token: lease.prompt().token,
                offer: action.offer(),
                targets: Vec::new(),
                parameters: serde_json::json!({}),
            })
        })
    }
}

impl DecisionProviderAdapter for FailingDecisionProvider {
    fn owner(&self) -> &OwnerId {
        &self.owner
    }

    fn decide(
        &self,
        _lease: DecisionWorkerLease,
    ) -> futures::future::BoxFuture<'static, Result<DecisionResponse, DecisionProviderFailure>>
    {
        self.calls.fetch_add(1, AtomicOrdering::SeqCst);
        self.called.notify_waiters();
        let kind = self.kind;
        Box::pin(async move { Err(DecisionProviderFailure::new(kind, "injected failure")) })
    }
}

impl DecisionProviderAdapter for BlockingDecisionProvider {
    fn owner(&self) -> &OwnerId {
        &self.owner
    }

    fn decide(
        &self,
        lease: DecisionWorkerLease,
    ) -> futures::future::BoxFuture<'static, Result<DecisionResponse, DecisionProviderFailure>>
    {
        self.calls.fetch_add(1, AtomicOrdering::SeqCst);
        self.called.notify_waiters();
        let release = Arc::clone(&self.release);
        Box::pin(async move {
            let permit = release.acquire_owned().await.map_err(|_| {
                DecisionProviderFailure::new(
                    crate::decision_dispatch::DecisionProviderFailureKind::Unavailable,
                    "test release semaphore closed",
                )
            })?;
            permit.forget();
            Ok(DecisionResponse {
                token: lease.prompt().token,
                offer: lease.prompt().actions[0].offer(),
                targets: Vec::new(),
                parameters: serde_json::json!({}),
            })
        })
    }
}

fn decision_harness(
    lease_duration: i64,
) -> (
    Engine,
    DecisionHarnessRegistration,
    Arc<Mutex<Option<DecisionCapturePlan>>>,
    DecisionKeyRing,
) {
    decision_harness_with(lease_duration, Vec::new())
}

/// The decision harness composed beside `extra` modules, so a test can put
/// a second **mechanics owner** in the composition without giving it a
/// decision source of its own.
fn decision_harness_with(
    lease_duration: i64,
    extra: Vec<Box<dyn GameModule>>,
) -> (
    Engine,
    DecisionHarnessRegistration,
    Arc<Mutex<Option<DecisionCapturePlan>>>,
    DecisionKeyRing,
) {
    let registration = Arc::new(Mutex::new(None));
    let capture_plan = Arc::new(Mutex::new(None));
    let capability_generation = Arc::new(AtomicU64::new(1));
    let module = DecisionHarnessModule {
        registration: Arc::clone(&registration),
        capture_plan: Arc::clone(&capture_plan),
        lease_duration,
        capability_generation,
    };
    let mut modules: Vec<Box<dyn GameModule>> = vec![Box::new(module)];
    modules.extend(extra);
    let mut world = mush_testkit::TestWorld::with_modules(modules);
    let keys = DecisionKeyRing::new(
        DecisionSecret::new(1, [0x5a; 32]).expect("test decision secret"),
        [],
    )
    .expect("test decision key ring");
    world.engine_mut().set_decision_keys(keys.clone());
    world.engine_mut().set_offer_keys(
        OfferKeyRing::new(
            OfferSecret::new(1, [0x6b; 32]).expect("test offer secret"),
            [],
        )
        .expect("test offer key ring"),
    );
    let _ = world.drain_write_batch();
    let registered = registration
        .lock()
        .expect("decision registration mutex poisoned")
        .clone()
        .expect("decision harness registration");
    (world.into_engine(), registered, capture_plan, keys)
}

struct TacticalDecisionHarness {
    engine: Engine,
    registration: DecisionHarnessRegistration,
    tactical_state: TacticalGateState,
    unit_spec: UnitSpec,
    capture_plan: Arc<Mutex<Option<DecisionCapturePlan>>>,
    keys: DecisionKeyRing,
}

fn tactical_decision_harness(lease_duration: i64) -> TacticalDecisionHarness {
    let registration = Arc::new(Mutex::new(None));
    let capture_plan = Arc::new(Mutex::new(None));
    let capability_generation = Arc::new(AtomicU64::new(1));
    let module = DecisionHarnessModule {
        registration: Arc::clone(&registration),
        capture_plan: Arc::clone(&capture_plan),
        lease_duration,
        capability_generation,
    };
    let (mut engine, tactical_state, _tactical_definitions, unit_spec) =
        tactical_gate_world_with_modules(vec![Box::new(module)]);
    let keys = DecisionKeyRing::new(
        DecisionSecret::new(1, [0x5a; 32]).expect("test decision secret"),
        [],
    )
    .expect("test decision key ring");
    engine.set_decision_keys(keys.clone());
    let registered = registration
        .lock()
        .expect("decision registration mutex poisoned")
        .clone()
        .expect("decision harness registration");
    TacticalDecisionHarness {
        engine,
        registration: registered,
        tactical_state,
        unit_spec,
        capture_plan,
        keys,
    }
}

fn projected_decision_action(engine: &Engine) -> ProjectedLegalAction {
    let query = engine
        .legal_action_query(
            PrincipalId::Object { id: GOD },
            GOD,
            PerceivedRef::SelfActor,
        )
        .expect("decision action query");
    engine
        .project_legal_actions(&query)
        .expect("decision action projection")
        .projected()
        .into_iter()
        .find(|action| action.presentation().verb == "choose")
        .expect("decision harness choice action")
}

fn arm_decision_capture(
    engine: &Engine,
    keys: &DecisionKeyRing,
    registration: &DecisionHarnessRegistration,
    capture_plan: &Mutex<Option<DecisionCapturePlan>>,
    salt: u8,
    deadline: i64,
) -> PendingDecision {
    let owner = OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner");
    let principal = PrincipalId::Object { id: GOD };
    let controller = DecisionControllerId {
        owner: owner.clone(),
        key: CanonicalId::new("controller_1").expect("static controller id"),
    };
    let cause = DecisionCause {
        kind: CanonicalId::new("test_turn").expect("static cause kind"),
        instance: IdempotencyKeyText::parse_canonical(format!("turn-{salt}"))
            .expect("test cause instance"),
        replaceable_key: None,
    };
    let action_idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner: owner.clone(),
            namespace: CanonicalId::new("chosen_action").expect("static namespace"),
        },
        1,
        IdempotencyKey::Deterministic(CanonicalHash::from_bytes([salt; 32])),
    );
    let id =
        DecisionId::derive(&controller, &cause, &action_idempotency).expect("decision identity");
    let projected = projected_decision_action(engine);
    let offer = projected.offer();
    let placeholder = DecisionToken {
        id,
        revision: 1,
        prompt_digest: CanonicalHash::from_bytes([1; 32]),
        key_version: 1,
        tag: CanonicalHash::from_bytes([1; 32]),
    };
    let mut prompt = DecisionPrompt {
        token: placeholder,
        actions: vec![projected],
        context: ProjectedDecisionContext::new(
            CanonicalId::new("test_context.v1").expect("static context id"),
            serde_json::json!({ "turn": salt }),
        )
        .expect("decision context"),
    };
    let prompt_digest = prompt.digest().expect("decision prompt digest");
    prompt.token = keys.sign(id, 1, prompt_digest);
    let prompt = DecisionPromptRecord::new(prompt).expect("decision prompt record");
    let pending = PendingDecision {
        id,
        revision: 1,
        controller,
        principal: principal.clone(),
        actor: GOD,
        truth_subject: SubjectKey::Object { id: GOD },
        viewer_scope: ProjectionScopeId::derive(&principal, 1).expect("decision projection scope"),
        cause,
        offered: vec![PendingOfferWitness {
            offer,
            common_revisions: Vec::new(),
            target_revisions: Vec::new(),
            bridge_definitions: Vec::new(),
        }],
        prompt_digest,
        token_key_version: 1,
        deadline: ClockStamp {
            clock: ClockRef::Utc,
            at: deadline,
        },
        fallback: FallbackPolicyRef(registration.lease_action.clone()),
        action_idempotency,
        malformed_attempts: 0,
        attempt_audit: Vec::new(),
        status: PendingDecisionStatus::Ready { attempt: 1 },
    };
    let admission = DecisionAdmissionIndex::default()
        .admit(&pending, &prompt, keys)
        .expect("decision admission");
    let plan = DecisionCapturePlan::new(
        &owner,
        pending.clone(),
        prompt,
        admission,
        keys,
        CanonicalId::new(DECISION_WAKE_ADAPTER).expect("static wake adapter"),
        0,
    )
    .expect("decision capture plan");
    *capture_plan
        .lock()
        .expect("decision capture-plan mutex poisoned") = Some(plan);
    pending
}

#[allow(clippy::too_many_arguments)]
fn arm_tactical_decision_capture(
    keys: &DecisionKeyRing,
    registration: &DecisionHarnessRegistration,
    capture_plan: &Mutex<Option<DecisionCapturePlan>>,
    actor: mush_core::ObjId,
    projected: ProjectedLegalAction,
    witness: PendingOfferWitness,
    salt: u8,
    deadline: i64,
) -> PendingDecision {
    let owner = OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner");
    let principal = PrincipalId::Object { id: actor };
    let controller = DecisionControllerId {
        owner: owner.clone(),
        key: CanonicalId::new("tactical_controller").expect("static controller id"),
    };
    let cause = DecisionCause {
        kind: CanonicalId::new("tactical_turn").expect("static cause kind"),
        instance: IdempotencyKeyText::parse_canonical(format!("tactical-turn-{salt}"))
            .expect("test cause instance"),
        replaceable_key: None,
    };
    let action_idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner: OwnerId::new(mush_contract_tests::tactical_gate::TACTICAL_GATE_OWNER)
                .expect("static tactics owner"),
            namespace: CanonicalId::new(mush_contract_tests::tactical_gate::FIRE_ACTION_ID)
                .expect("static fire namespace"),
        },
        1,
        IdempotencyKey::Deterministic(CanonicalHash::from_bytes([salt; 32])),
    );
    let id =
        DecisionId::derive(&controller, &cause, &action_idempotency).expect("decision identity");
    let placeholder = DecisionToken {
        id,
        revision: 1,
        prompt_digest: CanonicalHash::from_bytes([1; 32]),
        key_version: 1,
        tag: CanonicalHash::from_bytes([1; 32]),
    };
    let mut prompt = DecisionPrompt {
        token: placeholder,
        actions: vec![projected],
        context: ProjectedDecisionContext::new(
            CanonicalId::new("tactical_context.v1").expect("static context id"),
            serde_json::json!({ "turn": salt }),
        )
        .expect("decision context"),
    };
    let prompt_digest = prompt.digest().expect("decision prompt digest");
    prompt.token = keys.sign(id, 1, prompt_digest);
    let prompt = DecisionPromptRecord::new(prompt).expect("decision prompt record");
    let pending = PendingDecision {
        id,
        revision: 1,
        controller,
        principal: principal.clone(),
        actor,
        truth_subject: SubjectKey::Object { id: actor },
        viewer_scope: ProjectionScopeId::derive(&principal, 1).expect("decision projection scope"),
        cause,
        offered: vec![witness],
        prompt_digest,
        token_key_version: 1,
        deadline: ClockStamp {
            clock: ClockRef::Utc,
            at: deadline,
        },
        fallback: FallbackPolicyRef(registration.lease_action.clone()),
        action_idempotency,
        malformed_attempts: 0,
        attempt_audit: Vec::new(),
        status: PendingDecisionStatus::Ready { attempt: 1 },
    };
    let admission = DecisionAdmissionIndex::default()
        .admit(&pending, &prompt, keys)
        .expect("decision admission");
    let plan = DecisionCapturePlan::new(
        &owner,
        pending.clone(),
        prompt,
        admission,
        keys,
        CanonicalId::new(DECISION_WAKE_ADAPTER).expect("static wake adapter"),
        0,
    )
    .expect("decision capture plan");
    *capture_plan
        .lock()
        .expect("decision capture-plan mutex poisoned") = Some(plan);
    pending
}

fn lease_captured_decision(
    engine: &mut Engine,
    registration: &DecisionHarnessRegistration,
    pending: &PendingDecision,
    keys: &DecisionKeyRing,
) -> (
    mush_core::DecisionCompletionFence,
    DecisionResponse,
    DecisionWorkerLease,
) {
    let prompt = engine
        .action_state()
        .get(&registration.families.prompts, &pending.id)
        .expect("decision prompt read")
        .expect("decision prompt")
        .clone();
    let current = engine
        .action_state()
        .get(&registration.families.pending, &pending.id)
        .expect("pending decision read")
        .expect("pending decision")
        .clone();
    let now = ClockStamp {
        clock: ClockRef::Utc,
        at: pending.deadline.at - 30,
    };
    let wake = mush_core::DecisionWake {
        id: pending.id,
        revision: pending.revision,
        prompt_digest: pending.prompt_digest,
    };
    let transition = current
        .plan_worker_lease(&prompt, &wake, keys, &now, 20)
        .expect("decision lease plan");
    let (leased, worker) = transition.into_parts();
    let mut transaction = engine
        .action_transaction(&registration.families.pending)
        .expect("decision lease transaction");
    transaction
        .replace(&registration.families.pending, &pending.id, leased.clone())
        .expect("decision lease post-image");
    let (_, installation) = transaction
        .prepare()
        .expect("decision lease prepare")
        .into_parts();
    engine
        .install_composed_action(installation)
        .expect("the fixture installs");
    let fence = mush_core::DecisionCompletionFence::new(
        worker.controller().clone(),
        worker.decision(),
        worker.revision(),
        worker.attempt(),
        worker.lease_until().clone(),
    )
    .expect("decision completion fence");
    let response = DecisionResponse {
        token: prompt.prompt.token,
        offer: prompt.prompt.actions[0].offer(),
        targets: match prompt.prompt.actions[0].targets() {
            ProjectedTargetSchema::Choices { choices, .. } => vec![choices[0].clone()],
            ProjectedTargetSchema::None => Vec::new(),
        },
        parameters: serde_json::json!({}),
    };
    (fence, response, worker)
}

async fn commit_armed_capture(
    engine: &mut Engine,
    store: &dyn AuthoritativeStore,
    registration: &DecisionHarnessRegistration,
    salt: u8,
) {
    let request = RequestUid::from_bytes([salt; 16]);
    let principal = PrincipalId::Object { id: GOD };
    let idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
            namespace: CanonicalId::new("capture_decision").expect("static namespace"),
        },
        1,
        IdempotencyKey::Request(request),
    );
    let ingress = IngressEnvelope::new(
        request,
        principal.clone(),
        IngressSurfaceVersion::new(
            CanonicalId::new("test.decision_capture").expect("static ingress surface"),
            1,
        )
        .expect("capture ingress version"),
        IngressPayload::structured(serde_json::json!({ "salt": salt }))
            .expect("capture ingress payload"),
        idempotency.clone(),
    )
    .expect("capture ingress");
    let invocation = ActionInvocation::new(ActionInvocationDraft {
        request,
        principal,
        actor: GOD,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([salt; 32])),
        subject: PerceivedRef::SelfActor,
        targets: Vec::new(),
        parameters: serde_json::json!({}),
        idempotency,
    })
    .expect("capture invocation");
    let ActionPreflight::Vacant(permit) = ActionCoordinator::preflight(store, ingress)
        .await
        .expect("capture preflight")
    else {
        panic!("capture idempotency was not vacant")
    };
    let guards = engine
        .registry
        .server_guard_set_builder(RevisionSet::default())
        .expect("capture guard builder")
        .finish()
        .expect("capture guards");
    ActionCoordinator::prepare_and_stage(
        permit,
        engine,
        &registration.capture_action,
        &invocation,
        guards,
    )
    .expect("capture action staging");
    let staged = engine
        .take_staged_action()
        .expect("capture admission")
        .expect("capture staged action");
    let (bundle, installation) = staged.into_parts();
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await
        .expect("capture intent reservation");
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .expect("capture reconciliation");
    engine
        .install_composed_action(installation)
        .expect("the fixture installs");
}

async fn wait_for_persisted_decision_terminal(
    store: &dyn ObjectStore,
    registration: &DecisionHarnessRegistration,
    decision: DecisionId,
) -> DecisionTerminalReceipt {
    let pending_key = format!(
        "action-state/v1/{}/{}",
        registration.families.pending.descriptor().id(),
        decision.canonical_hash().to_hex()
    );
    let receipt_key = format!(
        "action-state/v1/{}/{}",
        registration.families.terminal_receipts.descriptor().id(),
        decision.canonical_hash().to_hex()
    );
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if let Some(value) = store
                .module_json_get(DECISION_HARNESS_OWNER, &receipt_key)
                .await
                .expect("decision compact receipt read")
            {
                break serde_json::from_value(value["value"].clone())
                    .expect("valid compact decision receipt");
            }
            if let Some(value) = store
                .module_json_get(DECISION_HARNESS_OWNER, &pending_key)
                .await
                .expect("decision state read")
                && value["value"]["status"]["state"] == "terminal"
            {
                let pending: PendingDecision = serde_json::from_value(value["value"].clone())
                    .expect("valid terminal pending decision");
                break DecisionTerminalReceipt::from_terminal(&pending)
                    .expect("valid terminal attribution");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("decision did not terminalize through deterministic fallback")
}

async fn discard_decision_wake(store: &dyn AuthoritativeStore) {
    let pending = store
        .pending_authoritative_outbox()
        .await
        .expect("pending decision wake");
    assert_eq!(pending.len(), 1, "capture must emit one durable wake");
    let wake = &pending[0];
    let started = store
        .begin_authoritative_outbox_attempt(
            wake.item_id(),
            wake.intent().payload_hash(),
            wake.attempts(),
        )
        .await
        .expect("begin discarded wake");
    store
        .complete_authoritative_outbox_attempt(
            started.item_id(),
            started.intent().payload_hash(),
            started.attempts(),
            AuthoritativeOutboxAttemptResult::Delivered,
        )
        .await
        .expect("discard decision wake");
}

fn decision_test_deps(store: Arc<dyn ObjectStore>) -> ServerDeps {
    ServerDeps {
        store,
        guard: Arc::new(IpGuard::new()),
        staff: Arc::new(StaffQueue::new()),
        llm: None,
        metrics: Arc::new(Metrics::new()),
        quarantine_spool: crate::quarantine::QuarantineSpool::in_memory(),
        outbox_adapters: Vec::new(),
    }
}

async fn stop_test_server(handles: PersistHandles) {
    assert!(
        handles
            .handle
            .inbound(Inbound::Control(ControlMsg::Shutdown))
            .await
    );
    handles.world_join.await.expect("world task shutdown");
    handles.writer_join.await.expect("writer task shutdown");
    handles.outbox_join.await.expect("outbox task shutdown");
}

#[tokio::test]
async fn decision_dispatch_rediscovery_leases_after_capture_ack_and_lost_wake() {
    let (mut engine, registration, capture_plan, keys) = decision_harness(30);
    let pending = arm_decision_capture(
        &engine,
        &keys,
        &registration,
        &capture_plan,
        0x31,
        unix_now_s() + 60,
    );
    let deps = ServerDeps::in_memory();
    let store = Arc::clone(&deps.store);
    let authoritative = store
        .authoritative_store()
        .expect("in-memory store is authoritative");
    commit_armed_capture(&mut engine, authoritative, &registration, 0x41).await;

    // Simulate the volatile wake disappearing after its durable outbox
    // delivery was acknowledged. The typed pair must remain sufficient
    // for the periodic canonical scan to recover the obligation.
    discard_decision_wake(authoritative).await;
    assert!(
        authoritative
            .pending_authoritative_outbox()
            .await
            .expect("post-discard outbox scan")
            .is_empty()
    );

    let calls = Arc::new(AtomicUsize::new(0));
    let called = Arc::new(tokio::sync::Notify::new());
    let mut providers = DecisionProviderRegistry::new();
    providers
        .register(Arc::new(RecordingDecisionProvider {
            owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
            calls: Arc::clone(&calls),
            called: Arc::clone(&called),
        }))
        .expect("decision provider registration");
    let handles = spawn_persist_with_gateway_and_decisions(
        engine,
        deps,
        DurableOutboxRegistry::new(),
        providers,
        1,
        |_| Arc::new(BlackholeGateway),
    )
    .expect("decision runtime startup");

    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if calls.load(AtomicOrdering::SeqCst) == 1 {
                break;
            }
            called.notified().await;
        }
    })
    .await
    .expect("periodic decision rediscovery did not dispatch");

    let persistence_key = format!(
        "action-state/v1/{}/{}",
        registration.families.pending.descriptor().id(),
        pending.id.canonical_hash().to_hex()
    );
    assert_eq!(calls.load(AtomicOrdering::SeqCst), 1);

    let selection_key = format!(
        "action-state/v1/{}/selected",
        registration.families.selections.descriptor().id()
    );
    let selected = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(value) = store
                .module_json_get(DECISION_HARNESS_OWNER, &selection_key)
                .await
                .expect("selected action read")
            {
                break value;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("authenticated decision response did not commit its selected action");
    assert_eq!(selected["value"]["count"], 1);

    let terminal = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let value = store
                .module_json_get(DECISION_HARNESS_OWNER, &persistence_key)
                .await
                .expect("terminal decision read")
                .expect("terminal decision record");
            if value["value"]["status"]["state"] == "terminal" {
                break value;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("selected action did not advance through resolving to terminal");
    assert_eq!(terminal["value"]["status"]["outcome"], "executed");
    assert!(terminal["value"]["status"]["action_binding"]["request"].is_string());
    assert!(terminal["value"]["status"]["action_binding"]["request_hash"].is_string());

    stop_test_server(handles).await;
}

#[tokio::test]
async fn expired_decision_lease_is_rediscovered_after_process_restart() {
    let (mut engine, registration, capture_plan, keys) = decision_harness(1);
    let pending = arm_decision_capture(
        &engine,
        &keys,
        &registration,
        &capture_plan,
        0x33,
        unix_now_s() + 30,
    );
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    commit_armed_capture(
        &mut engine,
        store
            .authoritative_store()
            .expect("in-memory store is authoritative"),
        &registration,
        0x43,
    )
    .await;

    let calls = Arc::new(AtomicUsize::new(0));
    let first_called = Arc::new(tokio::sync::Notify::new());
    let first_release = Arc::new(tokio::sync::Semaphore::new(0));
    let mut first_providers = DecisionProviderRegistry::new();
    first_providers
        .register(Arc::new(BlockingDecisionProvider {
            owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
            calls: Arc::clone(&calls),
            called: Arc::clone(&first_called),
            release: Arc::clone(&first_release),
        }))
        .expect("first decision provider registration");
    let first = spawn_persist_with_gateway_and_decisions(
        engine,
        decision_test_deps(Arc::clone(&store)),
        DurableOutboxRegistry::new(),
        first_providers,
        1,
        |_| Arc::new(BlackholeGateway),
    )
    .expect("first decision runtime startup");
    tokio::time::timeout(Duration::from_secs(3), async {
        while calls.load(AtomicOrdering::SeqCst) < 1 {
            first_called.notified().await;
        }
    })
    .await
    .expect("first decision lease did not dispatch");
    stop_test_server(first).await;
    first_release.add_permits(1);

    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let (mut restarted, restarted_registration, _plan, _keys) = decision_harness(1);
    let boot_records = store
        .module_json_list(DECISION_HARNESS_OWNER)
        .await
        .expect("decision state boot scan")
        .into_iter()
        .filter(|(key, _)| mush_core::is_action_state_persistence_key(key))
        .map(|(key, value)| ActionStateBootRecord::new(DECISION_HARNESS_OWNER, key, value))
        .collect::<Vec<_>>();
    restarted
        .hydrate_action_state(boot_records)
        .expect("decision state hydration after restart");

    let second_called = Arc::new(tokio::sync::Notify::new());
    let second_release = Arc::new(tokio::sync::Semaphore::new(0));
    let mut second_providers = DecisionProviderRegistry::new();
    second_providers
        .register(Arc::new(BlockingDecisionProvider {
            owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
            calls: Arc::clone(&calls),
            called: Arc::clone(&second_called),
            release: Arc::clone(&second_release),
        }))
        .expect("second decision provider registration");
    let second = spawn_persist_with_gateway_and_decisions(
        restarted,
        decision_test_deps(Arc::clone(&store)),
        DurableOutboxRegistry::new(),
        second_providers,
        1,
        |_| Arc::new(BlackholeGateway),
    )
    .expect("restarted decision runtime startup");
    tokio::time::timeout(Duration::from_secs(3), async {
        while calls.load(AtomicOrdering::SeqCst) < 2 {
            second_called.notified().await;
        }
    })
    .await
    .expect("expired lease was not rediscovered after restart");

    let persistence_key = format!(
        "action-state/v1/{}/{}",
        restarted_registration.families.pending.descriptor().id(),
        pending.id.canonical_hash().to_hex()
    );
    let persisted = store
        .module_json_get(DECISION_HARNESS_OWNER, &persistence_key)
        .await
        .expect("retried lease read")
        .expect("retried lease post-image");
    assert_eq!(persisted["value"]["status"]["state"], "leased");
    assert_eq!(persisted["value"]["status"]["attempt"], 2);
    stop_test_server(second).await;
    second_release.add_permits(1);
}

#[tokio::test]
async fn worker_saturation_leaves_second_decision_ready_until_capacity_returns() {
    let (mut engine, registration, capture_plan, keys) = decision_harness(30);
    let first_pending = arm_decision_capture(
        &engine,
        &keys,
        &registration,
        &capture_plan,
        0x34,
        unix_now_s() + 60,
    );
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let authoritative = store
        .authoritative_store()
        .expect("in-memory store is authoritative");
    commit_armed_capture(&mut engine, authoritative, &registration, 0x44).await;
    let second_pending = arm_decision_capture(
        &engine,
        &keys,
        &registration,
        &capture_plan,
        0x35,
        unix_now_s() + 60,
    );
    commit_armed_capture(&mut engine, authoritative, &registration, 0x45).await;

    let calls = Arc::new(AtomicUsize::new(0));
    let called = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let mut providers = DecisionProviderRegistry::new();
    providers
        .register(Arc::new(BlockingDecisionProvider {
            owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
            calls: Arc::clone(&calls),
            called: Arc::clone(&called),
            release: Arc::clone(&release),
        }))
        .expect("blocking provider registration");
    let handles = spawn_persist_with_gateway_and_decisions(
        engine,
        decision_test_deps(Arc::clone(&store)),
        DurableOutboxRegistry::new(),
        providers,
        1,
        |_| Arc::new(BlackholeGateway),
    )
    .expect("bounded decision runtime startup");
    tokio::time::timeout(Duration::from_secs(3), async {
        while calls.load(AtomicOrdering::SeqCst) < 1 {
            called.notified().await;
        }
    })
    .await
    .expect("first provider did not start");

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        calls.load(AtomicOrdering::SeqCst),
        1,
        "worker saturation must not commit another lease without capacity"
    );
    let first_key = format!(
        "action-state/v1/{}/{}",
        registration.families.pending.descriptor().id(),
        first_pending.id.canonical_hash().to_hex()
    );
    let second_key = format!(
        "action-state/v1/{}/{}",
        registration.families.pending.descriptor().id(),
        second_pending.id.canonical_hash().to_hex()
    );
    let states = [first_key.as_str(), second_key.as_str()]
        .into_iter()
        .map(|key| async {
            store
                .module_json_get(DECISION_HARNESS_OWNER, key)
                .await
                .expect("saturated decision read")
                .expect("saturated decision state")["value"]["status"]["state"]
                .as_str()
                .expect("decision state name")
                .to_owned()
        });
    let [first_state, second_state] = futures::future::join_all(states)
        .await
        .try_into()
        .expect("two decision states");
    assert_eq!(
        [first_state.as_str(), second_state.as_str()]
            .into_iter()
            .filter(|state| *state == "leased")
            .count(),
        1
    );
    assert_eq!(
        [first_state.as_str(), second_state.as_str()]
            .into_iter()
            .filter(|state| *state == "ready")
            .count(),
        1
    );

    release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(3), async {
        while calls.load(AtomicOrdering::SeqCst) < 2 {
            called.notified().await;
        }
    })
    .await
    .expect("second provider did not start after capacity returned");
    release.add_permits(1);
    stop_test_server(handles).await;
}

#[tokio::test]
async fn refused_decision_lease_aborts_post_image_without_spawning_provider() {
    let (mut engine, registration, capture_plan, keys) = decision_harness(30);
    let pending = arm_decision_capture(
        &engine,
        &keys,
        &registration,
        &capture_plan,
        0x32,
        unix_now_s() + 60,
    );
    let store = InMemoryStore::new();
    commit_armed_capture(&mut engine, &store, &registration, 0x42).await;

    let calls = Arc::new(AtomicUsize::new(0));
    let mut providers = DecisionProviderRegistry::new();
    providers
        .register(Arc::new(RecordingDecisionProvider {
            owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
            calls: Arc::clone(&calls),
            called: Arc::new(tokio::sync::Notify::new()),
        }))
        .expect("decision provider registration");
    let (completion_tx, _completion_rx) = mpsc::channel(MAX_DECISION_COMPLETIONS);
    let dispatcher = Arc::new(
        DecisionDispatcher::new(Arc::new(providers), 1, completion_tx)
            .expect("decision dispatcher"),
    );
    let mut driver = DurableDecisionDriver::new(1, dispatcher);
    let DurableDecisionDrive::Staged(staged_dispatch) = driver
        .drive_one(&mut engine, &store, None)
        .await
        .expect("decision lease drive")
    else {
        panic!("ready decision did not stage a lease")
    };
    let StagedDecisionDispatch {
        staged,
        reservation,
    } = *staged_dispatch;
    let StagedDecisionLease { staged, worker } = staged;
    let prepared = engine
        .take_staged_action()
        .expect("lease admission")
        .expect("staged lease action");
    let (_bundle, installation) = prepared.into_parts();
    let (_receipt_tx, receipt_rx) = oneshot::channel();
    let pending_commit = PendingCommit {
        token: CommitToken::Legacy(1),
        receipt: PendingReceipt::Legacy {
            expected_serial: 1,
            receiver: receipt_rx,
        },
        sends: Vec::new(),
        effects: Vec::new(),
        failure_actor: None,
        create: None,
        action: Some(ActionCompletion {
            disposition: None,
            staged,
            owned: None,
            finished: None,
            ownership_valid: false,
            post_commit_decision: Some(DecisionPostCommit {
                reservation,
                worker,
            }),
            post_commit_decision_response: None,
            post_commit_decision_terminal: None,
        }),
        prepared_installation: Some(installation),
        barriers: Vec::new(),
        coalesced_reattach: None,
        exit_after_ack: false,
    };
    let gateway: Gateway = Arc::new(BlackholeGateway);
    let mut health = StoreHealth::new();
    let integrity = PersistenceIntegrityLatch::new();
    let (world_tx, _world_rx) = mpsc::channel(1);
    let _ = complete_pending_commit(
        pending_commit,
        Err(AuthoritativeCommitFailure::Refused(
            "injected lease refusal".to_owned(),
        )),
        &mut engine,
        &gateway,
        &mut health,
        &integrity,
        &world_tx.downgrade(),
        None,
        &test_effect_store(),
    );

    assert_eq!(calls.load(AtomicOrdering::SeqCst), 0);
    assert!(matches!(
        engine
            .action_state()
            .get(&registration.families.pending, &pending.id)
            .expect("pending decision read")
            .expect("pending decision remains")
            .status,
        PendingDecisionStatus::Ready { attempt: 1 }
    ));
    assert!(!engine.action_state().has_prepared());
}

#[tokio::test]
async fn malformed_provider_attempts_commit_then_execute_deterministic_fallback() {
    let (mut engine, registration, capture_plan, keys) = decision_harness(30);
    let pending = arm_decision_capture(
        &engine,
        &keys,
        &registration,
        &capture_plan,
        0x37,
        unix_now_s() + 60,
    );
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    commit_armed_capture(
        &mut engine,
        store
            .authoritative_store()
            .expect("in-memory store is authoritative"),
        &registration,
        0x47,
    )
    .await;

    let calls = Arc::new(AtomicUsize::new(0));
    let called = Arc::new(tokio::sync::Notify::new());
    let mut providers = DecisionProviderRegistry::new();
    providers
        .register(Arc::new(FailingDecisionProvider {
            owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
            calls: Arc::clone(&calls),
            called: Arc::clone(&called),
            kind: DecisionProviderFailureKind::Malformed,
        }))
        .expect("failing provider registration");
    let handles = spawn_persist_with_gateway_and_decisions(
        engine,
        decision_test_deps(Arc::clone(&store)),
        DurableOutboxRegistry::new(),
        providers,
        1,
        |_| Arc::new(BlackholeGateway),
    )
    .expect("decision runtime startup");
    tokio::time::timeout(Duration::from_secs(4), async {
        while calls.load(AtomicOrdering::SeqCst) < usize::from(mush_core::DEFAULT_DECISION_ATTEMPTS)
        {
            called.notified().await;
        }
    })
    .await
    .expect("bounded malformed attempts did not run");

    let fallback =
        wait_for_persisted_decision_terminal(store.as_ref(), &registration, pending.id).await;
    assert_eq!(
        fallback.malformed_attempts,
        mush_core::DEFAULT_DECISION_ATTEMPTS
    );
    assert_eq!(
        fallback.outcome,
        mush_core::DecisionTerminalOutcome::Fallback
    );
    assert_eq!(
        fallback.fallback_reason,
        Some(mush_core::DecisionFallbackReason::MalformedExhausted)
    );
    assert_eq!(
        fallback
            .attempt_audit
            .iter()
            .map(|attempt| (attempt.attempt, attempt.failure))
            .collect::<Vec<_>>(),
        vec![
            (1, mush_core::DecisionAttemptFailureKind::Malformed),
            (2, mush_core::DecisionAttemptFailureKind::Malformed),
            (3, mush_core::DecisionAttemptFailureKind::Malformed),
        ]
    );
    let selection_key = format!(
        "action-state/v1/{}/selected",
        registration.families.selections.descriptor().id()
    );
    let selected = store
        .module_json_get(DECISION_HARNESS_OWNER, &selection_key)
        .await
        .expect("fallback selection read")
        .expect("fallback selected action state");
    assert_eq!(selected["value"]["count"], 1);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        calls.load(AtomicOrdering::SeqCst),
        usize::from(mush_core::DEFAULT_DECISION_ATTEMPTS),
        "fallback terminalization must stop provider redispatch"
    );
    stop_test_server(handles).await;
}

#[tokio::test]
async fn provider_outage_exhausts_bounded_attempts_then_executes_fallback() {
    let (mut engine, registration, capture_plan, keys) = decision_harness(30);
    let pending = arm_decision_capture(
        &engine,
        &keys,
        &registration,
        &capture_plan,
        0x3b,
        unix_now_s() + 60,
    );
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    commit_armed_capture(
        &mut engine,
        store
            .authoritative_store()
            .expect("in-memory store is authoritative"),
        &registration,
        0x4b,
    )
    .await;

    let calls = Arc::new(AtomicUsize::new(0));
    let called = Arc::new(tokio::sync::Notify::new());
    let mut providers = DecisionProviderRegistry::new();
    providers
        .register(Arc::new(FailingDecisionProvider {
            owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
            calls: Arc::clone(&calls),
            called: Arc::clone(&called),
            kind: DecisionProviderFailureKind::Unavailable,
        }))
        .expect("failing provider registration");
    let handles = spawn_persist_with_gateway_and_decisions(
        engine,
        decision_test_deps(Arc::clone(&store)),
        DurableOutboxRegistry::new(),
        providers,
        1,
        |_| Arc::new(BlackholeGateway),
    )
    .expect("decision runtime startup");
    tokio::time::timeout(Duration::from_secs(4), async {
        while calls.load(AtomicOrdering::SeqCst) < usize::from(mush_core::DEFAULT_DECISION_ATTEMPTS)
        {
            called.notified().await;
        }
    })
    .await
    .expect("bounded unavailable-provider attempts did not run");
    let terminal =
        wait_for_persisted_decision_terminal(store.as_ref(), &registration, pending.id).await;
    assert_eq!(terminal.malformed_attempts, 0);
    assert_eq!(
        terminal.outcome,
        mush_core::DecisionTerminalOutcome::Fallback
    );
    assert_eq!(
        terminal.fallback_reason,
        Some(mush_core::DecisionFallbackReason::ProviderExhausted)
    );
    assert_eq!(
        terminal
            .attempt_audit
            .iter()
            .map(|attempt| (attempt.attempt, attempt.failure))
            .collect::<Vec<_>>(),
        vec![
            (1, mush_core::DecisionAttemptFailureKind::Unavailable),
            (2, mush_core::DecisionAttemptFailureKind::Unavailable),
            (3, mush_core::DecisionAttemptFailureKind::Unavailable),
        ]
    );
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        calls.load(AtomicOrdering::SeqCst),
        usize::from(mush_core::DEFAULT_DECISION_ATTEMPTS),
        "provider outage must stop at the bounded attempt policy"
    );
    stop_test_server(handles).await;
}

#[tokio::test]
async fn unavailable_provider_stages_exact_fallback_custody() {
    let (mut engine, registration, capture_plan, keys) = decision_harness(30);
    let pending = arm_decision_capture(
        &engine,
        &keys,
        &registration,
        &capture_plan,
        0x38,
        unix_now_s() + 60,
    );
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    commit_armed_capture(
        &mut engine,
        store
            .authoritative_store()
            .expect("in-memory store is authoritative"),
        &registration,
        0x48,
    )
    .await;

    let (completion_tx, _completion_rx) = mpsc::channel(MAX_DECISION_COMPLETIONS);
    let dispatcher = Arc::new(
        DecisionDispatcher::new(Arc::new(DecisionProviderRegistry::new()), 1, completion_tx)
            .expect("decision dispatcher"),
    );
    let mut driver = DurableDecisionDriver::new(1, dispatcher);
    let DurableDecisionDrive::StagedLifecycle(staged) = driver
        .drive_one(
            &mut engine,
            store
                .authoritative_store()
                .expect("in-memory store is authoritative"),
            None,
        )
        .await
        .expect("provider-unavailable fallback drive")
    else {
        panic!("provider unavailability did not stage fallback custody")
    };
    let prepared = engine
        .take_staged_action()
        .expect("fallback custody admission")
        .expect("staged fallback custody action");
    let (bundle, installation) = prepared.into_parts();
    assert_eq!(bundle.token(), staged.token);
    assert_eq!(bundle.bundle_hash(), staged.bundle_hash);
    let authoritative = store
        .authoritative_store()
        .expect("in-memory store is authoritative");
    authoritative
        .reserve_authoritative_intent((*bundle).clone())
        .await
        .expect("fallback custody intent");
    authoritative
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .expect("fallback custody reconcile");
    engine
        .install_composed_action(installation)
        .expect("the fixture installs");
    assert!(matches!(
        engine
            .action_state()
            .get(&registration.families.pending, &pending.id)
            .expect("fallback custody read")
            .expect("pending decision")
            .status,
        PendingDecisionStatus::FallbackReady {
            reason: mush_core::DecisionFallbackReason::ProviderUnavailable,
            ..
        }
    ));
}

#[tokio::test]
async fn recovery_audits_all_durable_limits_before_first_dispatch() {
    let (mut engine, registration, capture_plan, keys) = decision_harness(30);
    let store = InMemoryStore::new();
    for offset in 0_u8..=u8::try_from(mush_core::MAX_NONTERMINAL_DECISIONS_PER_CONTROLLER)
        .expect("controller cap fits u8")
    {
        arm_decision_capture(
            &engine,
            &keys,
            &registration,
            &capture_plan,
            0x50 + offset,
            unix_now_s() + 60,
        );
        commit_armed_capture(&mut engine, &store, &registration, 0x60 + offset).await;
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let mut providers = DecisionProviderRegistry::new();
    providers
        .register(Arc::new(RecordingDecisionProvider {
            owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
            calls: Arc::clone(&calls),
            called: Arc::new(tokio::sync::Notify::new()),
        }))
        .expect("decision provider registration");
    let (completion_tx, _completion_rx) = mpsc::channel(MAX_DECISION_COMPLETIONS);
    let dispatcher = Arc::new(
        DecisionDispatcher::new(Arc::new(providers), 1, completion_tx)
            .expect("decision dispatcher"),
    );
    let mut driver = DurableDecisionDriver::new(1, dispatcher);
    let error = match driver.drive_one(&mut engine, &store, None).await {
        Err(error) => error,
        Ok(_) => panic!("over-cap durable decisions must freeze recovery"),
    };
    assert!(error.contains("limit reached"), "{error}");
    assert_eq!(calls.load(AtomicOrdering::SeqCst), 0);
    assert!(!engine.action_state().has_prepared());
}

#[tokio::test]
async fn expired_deadline_wins_before_available_provider_is_called() {
    let (mut engine, registration, capture_plan, keys) = decision_harness(30);
    let pending = arm_decision_capture(
        &engine,
        &keys,
        &registration,
        &capture_plan,
        0x39,
        unix_now_s() - 1,
    );
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    commit_armed_capture(
        &mut engine,
        store
            .authoritative_store()
            .expect("in-memory store is authoritative"),
        &registration,
        0x49,
    )
    .await;

    let calls = Arc::new(AtomicUsize::new(0));
    let mut providers = DecisionProviderRegistry::new();
    providers
        .register(Arc::new(RecordingDecisionProvider {
            owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
            calls: Arc::clone(&calls),
            called: Arc::new(tokio::sync::Notify::new()),
        }))
        .expect("decision provider registration");
    let handles = spawn_persist_with_gateway_and_decisions(
        engine,
        decision_test_deps(Arc::clone(&store)),
        DurableOutboxRegistry::new(),
        providers,
        1,
        |_| Arc::new(BlackholeGateway),
    )
    .expect("decision runtime startup");
    let terminal =
        wait_for_persisted_decision_terminal(store.as_ref(), &registration, pending.id).await;
    assert_eq!(
        terminal.outcome,
        mush_core::DecisionTerminalOutcome::Fallback
    );
    assert_eq!(calls.load(AtomicOrdering::SeqCst), 0);
    stop_test_server(handles).await;
}

#[tokio::test]
async fn cancellation_ack_installs_terminal_state_before_token_rejection() {
    let (mut engine, registration, capture_plan, keys) = decision_harness(30);
    let pending = arm_decision_capture(
        &engine,
        &keys,
        &registration,
        &capture_plan,
        0x3a,
        unix_now_s() + 60,
    );
    let store = InMemoryStore::new();
    commit_armed_capture(&mut engine, &store, &registration, 0x4a).await;

    let calls = Arc::new(AtomicUsize::new(0));
    let mut providers = DecisionProviderRegistry::new();
    providers
        .register(Arc::new(RecordingDecisionProvider {
            owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
            calls,
            called: Arc::new(tokio::sync::Notify::new()),
        }))
        .expect("decision provider registration");
    let (completion_tx, _completion_rx) = mpsc::channel(MAX_DECISION_COMPLETIONS);
    let dispatcher = Arc::new(
        DecisionDispatcher::new(Arc::new(providers), 1, completion_tx)
            .expect("decision dispatcher"),
    );
    let mut driver = DurableDecisionDriver::new(1, dispatcher);
    let DurableDecisionDrive::Staged(staged_dispatch) = driver
        .drive_one(&mut engine, &store, None)
        .await
        .expect("decision lease drive")
    else {
        panic!("ready decision did not stage a lease")
    };
    let StagedDecisionDispatch {
        staged,
        reservation,
    } = *staged_dispatch;
    let StagedDecisionLease { staged, worker } = staged;
    let prepared = engine
        .take_staged_action()
        .expect("lease admission")
        .expect("staged lease action");
    let (bundle, installation) = prepared.into_parts();
    assert_eq!(bundle.token(), staged.token);
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await
        .expect("lease intent");
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .expect("lease reconcile");
    engine
        .install_composed_action(installation)
        .expect("the fixture installs");
    drop(reservation);

    let fence = mush_core::DecisionCompletionFence::new(
        worker.controller().clone(),
        worker.decision(),
        worker.revision(),
        worker.attempt(),
        worker.lease_until().clone(),
    )
    .expect("worker fence");
    let response = DecisionResponse {
        token: worker.prompt().token,
        offer: worker.prompt().actions[0].offer(),
        targets: Vec::new(),
        parameters: serde_json::json!({}),
    };
    engine
        .decision_response_ingress(&fence, &response)
        .expect("live lease accepts its exact response");

    let ingress = ActionCoordinator::decision_cancellation_ingress(
        &engine,
        &pending.controller,
        pending.id,
        pending.revision,
        1,
    )
    .expect("cancellation ingress");
    let ActionPreflight::Vacant(permit) = ActionCoordinator::preflight(&store, ingress)
        .await
        .expect("cancellation preflight")
    else {
        panic!("cancellation idempotency was not vacant")
    };
    let staged = ActionCoordinator::prepare_and_stage_decision_cancellation(
        permit,
        &mut engine,
        &pending.controller,
        pending.id,
        pending.revision,
    )
    .expect("cancellation staging");
    engine
        .decision_response_ingress(&fence, &response)
        .expect("unacknowledged cancellation has not changed live authority");
    let prepared = engine
        .take_staged_action()
        .expect("cancellation admission")
        .expect("staged cancellation action");
    let (bundle, installation) = prepared.into_parts();
    assert_eq!(bundle.token(), staged.token);
    assert_eq!(bundle.bundle_hash(), staged.bundle_hash);
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await
        .expect("cancellation intent");
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .expect("cancellation reconcile");
    engine
        .install_composed_action(installation)
        .expect("the fixture installs");
    assert!(engine.decision_response_ingress(&fence, &response).is_err());
    assert!(matches!(
        engine
            .action_state()
            .get(&registration.families.pending, &pending.id)
            .expect("cancelled decision read")
            .expect("cancelled decision")
            .status,
        PendingDecisionStatus::Terminal {
            outcome: mush_core::DecisionTerminalOutcome::Cancelled,
            action_binding: None,
            ..
        }
    ));

    let compacted = loop {
        match driver
            .drive_one(&mut engine, &store, None)
            .await
            .expect("cancelled decision compaction drive")
        {
            DurableDecisionDrive::StagedLifecycle(staged) => break staged,
            DurableDecisionDrive::Idle | DurableDecisionDrive::RecoveryAuditProgress => {}
            _ => panic!("cancelled decision did not stage no-action compaction"),
        }
    };
    let prepared = engine
        .take_staged_action()
        .expect("compaction admission")
        .expect("staged compaction action");
    let (bundle, installation) = prepared.into_parts();
    assert_eq!(bundle.token(), compacted.token);
    assert_eq!(bundle.bundle_hash(), compacted.bundle_hash);
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await
        .expect("compaction intent");
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .expect("compaction reconcile");
    engine
        .install_composed_action(installation)
        .expect("the fixture installs");
    assert!(
        engine
            .action_state()
            .get(&registration.families.pending, &pending.id)
            .expect("compacted pending read")
            .is_none()
    );
    let receipt = engine
        .action_state()
        .get(&registration.families.terminal_receipts, &pending.id)
        .expect("compact receipt read")
        .expect("compact receipt");
    assert_eq!(
        receipt.outcome,
        mush_core::DecisionTerminalOutcome::Cancelled
    );
    assert!(receipt.action_binding.is_none());
}

#[tokio::test]
async fn selected_action_terminal_cannot_compact_without_exact_final_receipt() {
    let (mut engine, registration, capture_plan, keys) = decision_harness(30);
    let pending = arm_decision_capture(
        &engine,
        &keys,
        &registration,
        &capture_plan,
        0x3c,
        unix_now_s() + 60,
    );
    let store = InMemoryStore::new();
    commit_armed_capture(&mut engine, &store, &registration, 0x4c).await;

    let binding = mush_core::DecisionActionBinding {
        request: RequestUid::from_bytes([0x5c; 16]),
        request_hash: CanonicalHash::from_bytes([0x6c; 32]),
    };
    let mut terminal = pending.clone();
    terminal
        .finish(
            terminal.revision,
            mush_core::DecisionTerminalOutcome::Executed,
            Some(binding),
        )
        .expect("synthetic selected terminal");
    let pending_handle = registration.families.pending.clone();
    let mut transaction = engine
        .action_transaction(&pending_handle)
        .expect("terminal test transaction");
    transaction
        .replace(&pending_handle, &pending.id, terminal)
        .expect("terminal test replacement");
    let (_, installation) = transaction
        .prepare()
        .expect("terminal test prepare")
        .into_parts();
    engine
        .install_composed_action(installation)
        .expect("the fixture installs");

    let (completion_tx, _completion_rx) = mpsc::channel(MAX_DECISION_COMPLETIONS);
    let dispatcher = Arc::new(
        DecisionDispatcher::new(Arc::new(DecisionProviderRegistry::new()), 1, completion_tx)
            .expect("decision dispatcher"),
    );
    let mut driver = DurableDecisionDriver::new(1, dispatcher);
    let error = match driver.drive_one(&mut engine, &store, None).await {
        Err(error) => error,
        Ok(_) => panic!("vacant selected action must prevent compaction"),
    };
    assert!(
        error.contains("no authoritative selected action"),
        "{error}"
    );
    assert!(!engine.action_state().has_prepared());
    assert!(
        engine
            .action_state()
            .get(&registration.families.terminal_receipts, &pending.id)
            .expect("receipt absence read")
            .is_none()
    );
}

#[tokio::test]
async fn decision_response_requires_exact_live_fence_before_offer_resolution() {
    let (mut engine, registration, capture_plan, keys) = decision_harness(30);
    let pending = arm_decision_capture(
        &engine,
        &keys,
        &registration,
        &capture_plan,
        0x36,
        unix_now_s() + 60,
    );
    let store = InMemoryStore::new();
    commit_armed_capture(&mut engine, &store, &registration, 0x46).await;

    let calls = Arc::new(AtomicUsize::new(0));
    let mut providers = DecisionProviderRegistry::new();
    providers
        .register(Arc::new(RecordingDecisionProvider {
            owner: OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner"),
            calls,
            called: Arc::new(tokio::sync::Notify::new()),
        }))
        .expect("decision provider registration");
    let (completion_tx, _completion_rx) = mpsc::channel(MAX_DECISION_COMPLETIONS);
    let dispatcher = Arc::new(
        DecisionDispatcher::new(Arc::new(providers), 1, completion_tx)
            .expect("decision dispatcher"),
    );
    let mut driver = DurableDecisionDriver::new(1, dispatcher);
    let DurableDecisionDrive::Staged(staged_dispatch) = driver
        .drive_one(&mut engine, &store, None)
        .await
        .expect("decision lease drive")
    else {
        panic!("ready decision did not stage a lease")
    };
    let StagedDecisionDispatch {
        staged,
        reservation,
    } = *staged_dispatch;
    let StagedDecisionLease { staged, worker } = staged;
    let prepared = engine
        .take_staged_action()
        .expect("lease admission")
        .expect("staged lease action");
    let (bundle, installation) = prepared.into_parts();
    assert_eq!(bundle.token(), staged.token);
    assert_eq!(bundle.bundle_hash(), staged.bundle_hash);
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await
        .expect("lease intent reservation");
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .expect("lease reconciliation");
    engine
        .install_composed_action(installation)
        .expect("the fixture installs");
    drop(reservation);

    let fence = mush_core::DecisionCompletionFence::new(
        worker.controller().clone(),
        worker.decision(),
        worker.revision(),
        worker.attempt(),
        worker.lease_until().clone(),
    )
    .expect("worker completion fence");
    let response = DecisionResponse {
        token: worker.prompt().token,
        offer: worker.prompt().actions[0].offer(),
        targets: Vec::new(),
        parameters: serde_json::json!({}),
    };
    let mut forged = response.clone();
    forged.token.tag = CanonicalHash::from_bytes([0xf0; 32]);
    assert!(matches!(
        engine.decision_response_ingress(&fence, &forged),
        Err(mush_core::DecisionResponseIngressError::StaleCompletion)
    ));
    let wrong_attempt = mush_core::DecisionCompletionFence::new(
        worker.controller().clone(),
        worker.decision(),
        worker.revision(),
        worker.attempt() + 1,
        worker.lease_until().clone(),
    )
    .expect("wrong-attempt fence is structurally valid");
    assert!(matches!(
        engine.decision_response_ingress(&wrong_attempt, &response),
        Err(mush_core::DecisionResponseIngressError::StaleCompletion)
    ));

    let ingress = ActionCoordinator::decision_response_ingress(&engine, &fence, &response)
        .expect("valid response ingress");
    let first_ingress = ingress.clone();
    assert_eq!(ingress.idempotency(), &pending.action_idempotency);
    let ActionPreflight::Vacant(permit) = ActionCoordinator::preflight(&store, ingress)
        .await
        .expect("response preflight")
    else {
        panic!("selected action idempotency was not vacant")
    };
    let selected = ActionCoordinator::prepare_and_stage_decision_response(
        permit,
        &mut engine,
        fence.clone(),
        &response,
    )
    .expect("decision response offer resolution");
    assert_ne!(
        selected.binding.request_hash,
        CanonicalHash::from_bytes([0; 32])
    );
    let staged = engine
        .take_staged_action()
        .expect("selected action admission")
        .expect("selected staged action");
    let (_bundle, installation) = staged.into_parts();
    engine
        .abort_prepared_action(installation)
        .expect("the fixture aborts");

    let retry_at = worker.lease_until().clone();
    engine.set_now(retry_at.at);
    let mut retry = engine
        .action_state()
        .get(&registration.families.pending, &pending.id)
        .expect("leased decision read")
        .expect("leased decision")
        .clone();
    retry
        .retry_expired_lease(retry.revision, &retry_at)
        .expect("expire first lease");
    let second_lease_until = ClockStamp {
        clock: retry_at.clock.clone(),
        at: retry_at.at + 1,
    };
    retry
        .lease(retry.revision, second_lease_until.clone())
        .expect("second lease");
    let mut transaction = engine
        .action_transaction(&registration.families.pending)
        .expect("retry transaction");
    transaction
        .replace(&registration.families.pending, &pending.id, retry)
        .expect("retry post-image");
    let prepared = transaction.prepare().expect("retry preparation");
    let (_, installation) = prepared.into_parts();
    engine
        .install_composed_action(installation)
        .expect("the fixture installs");
    let second_fence = mush_core::DecisionCompletionFence::new(
        worker.controller().clone(),
        worker.decision(),
        worker.revision(),
        2,
        second_lease_until,
    )
    .expect("second completion fence");
    let second_ingress =
        ActionCoordinator::decision_response_ingress(&engine, &second_fence, &response)
            .expect("second-attempt response ingress");
    assert!(matches!(
        ActionCoordinator::decision_response_ingress(&engine, &fence, &response),
        Err(crate::action_coordinator::ActionCoordinatorError::DecisionResponseIngress(_))
    ));
    assert_eq!(second_ingress.request(), first_ingress.request());
    assert_eq!(
        second_ingress.ingress_hash(),
        first_ingress.ingress_hash(),
        "operational retry fences must not change semantic action identity"
    );

    let duplicate_ingress = second_ingress.clone();
    let ActionPreflight::Vacant(permit) = ActionCoordinator::preflight(&store, second_ingress)
        .await
        .expect("second response preflight")
    else {
        panic!("second-attempt selected action idempotency was not vacant")
    };
    let selected = ActionCoordinator::prepare_and_stage_decision_response(
        permit,
        &mut engine,
        second_fence.clone(),
        &response,
    )
    .expect("second-attempt response resolution");
    let binding = selected.binding.clone();
    let staged = engine
        .take_staged_action()
        .expect("selected action admission")
        .expect("selected action stage");
    let (bundle, installation) = staged.into_parts();
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await
        .expect("selected action intent");
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .expect("selected action reconcile");
    engine
        .install_composed_action(installation)
        .expect("the fixture installs");
    assert!(matches!(
        ActionCoordinator::preflight(&store, duplicate_ingress)
            .await
            .expect("duplicate response preflight"),
        ActionPreflight::Finalized(_)
    ));

    assert!(
        matches!(
            engine
                .action_state()
                .get(&registration.families.pending, &pending.id)
                .expect("pending read")
                .expect("pending record")
                .status,
            PendingDecisionStatus::Leased { attempt: 2, .. }
        ),
        "selected action receipt alone must not cross the owner-local lifecycle boundary"
    );
    let resolving = stage_decision_resolving(
        &mut engine,
        &store,
        &second_fence,
        &binding,
        mush_core::DecisionTerminalOutcome::Executed,
        1,
    )
    .await
    .expect("resolving stage")
    .expect("resolving lifecycle was vacant");
    let staged = engine
        .take_staged_action()
        .expect("resolving action admission")
        .expect("resolving action stage");
    let (bundle, installation) = staged.into_parts();
    assert_eq!(bundle.token(), resolving.token);
    assert_eq!(bundle.bundle_hash(), resolving.bundle_hash);
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await
        .expect("resolving intent");
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .expect("resolving reconcile");
    engine
        .install_composed_action(installation)
        .expect("the fixture installs");
    let resolved = engine
        .action_state()
        .get(&registration.families.pending, &pending.id)
        .expect("resolved pending read")
        .expect("resolved pending record");
    assert!(matches!(
        &resolved.status,
        PendingDecisionStatus::Resolving {
            attempt: 2,
            action_binding,
            ..
        } if action_binding == &binding
    ));

    // Simulate losing the volatile post-Resolving follow-up. The bounded
    // typed-family scan must rediscover the binding, authenticate its
    // finalized ordinary receipt, and stage the exact terminal action.
    driver.cursor = None;
    driver.retry = None;
    driver.reconciliation = None;
    let DurableDecisionDrive::StagedLifecycle(terminal) = driver
        .drive_one(&mut engine, &store, None)
        .await
        .expect("resolving recovery drive")
    else {
        panic!("resolving recovery did not stage the terminal lifecycle")
    };
    let staged = engine
        .take_staged_action()
        .expect("terminal action admission")
        .expect("terminal action stage");
    let (bundle, installation) = staged.into_parts();
    assert_eq!(bundle.token(), terminal.token);
    assert_eq!(bundle.bundle_hash(), terminal.bundle_hash);
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await
        .expect("terminal intent");
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .expect("terminal reconcile");
    engine
        .install_composed_action(installation)
        .expect("the fixture installs");
    let terminal = engine
        .action_state()
        .get(&registration.families.pending, &pending.id)
        .expect("terminal pending read")
        .expect("terminal pending record");
    assert!(matches!(
        &terminal.status,
        PendingDecisionStatus::Terminal {
            outcome: mush_core::DecisionTerminalOutcome::Executed,
            action_binding: Some(action_binding),
            ..
        } if action_binding == &binding
    ));
}

#[tokio::test]
async fn delayed_response_rejects_revoked_capability_before_provider_preparation() {
    let (mut engine, registration, capture_plan, keys) = decision_harness(30);
    let pending = arm_decision_capture(
        &engine,
        &keys,
        &registration,
        &capture_plan,
        0x5c,
        unix_now_s() + 60,
    );
    let store = InMemoryStore::new();
    commit_armed_capture(&mut engine, &store, &registration, 0x6c).await;
    let (fence, response, _worker) =
        lease_captured_decision(&mut engine, &registration, &pending, &keys);
    registration
        .capability_generation
        .store(2, AtomicOrdering::SeqCst);
    let ingress = ActionCoordinator::decision_response_ingress(&engine, &fence, &response)
        .expect("live lease still authenticates before capability resolution");
    let ActionPreflight::Vacant(permit) = ActionCoordinator::preflight(&store, ingress)
        .await
        .expect("capability stale preflight")
    else {
        panic!("capability stale action idempotency must remain vacant")
    };
    assert!(
        ActionCoordinator::prepare_and_stage_decision_response(
            permit,
            &mut engine,
            fence,
            &response,
        )
        .is_err()
    );
    assert!(engine.take_staged_action().unwrap().is_none());
    assert!(!engine.action_state().has_prepared());
    assert!(
        engine
            .action_state()
            .get(
                &registration.families.selections,
                &CanonicalId::new("selected").unwrap(),
            )
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn provider_latency_cannot_change_delayed_action_identity_or_authority() {
    let (mut engine, registration, capture_plan, keys) = decision_harness(30);
    let pending = arm_decision_capture(
        &engine,
        &keys,
        &registration,
        &capture_plan,
        0x5d,
        unix_now_s() + 60,
    );
    let store = InMemoryStore::new();
    commit_armed_capture(&mut engine, &store, &registration, 0x6d).await;
    let (fence, expected_response, worker) =
        lease_captured_decision(&mut engine, &registration, &pending, &keys);
    let owner = OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner");
    let mut bundles = Vec::new();

    for kind in [
        ProjectedPolicyKind::Deterministic,
        ProjectedPolicyKind::DecisionTree,
        ProjectedPolicyKind::MockLlm,
    ] {
        let adapter = ProjectedPolicyProvider {
            owner: owner.clone(),
            kind,
        };
        let response = adapter
            .decide(worker.clone())
            .await
            .unwrap_or_else(|error| panic!("{kind:?} provider failed: {error:?}"));
        assert_eq!(response, expected_response, "{kind:?} changed selection");
        let ingress = ActionCoordinator::decision_response_ingress(&engine, &fence, &response)
            .unwrap_or_else(|error| panic!("{kind:?} ingress failed: {error}"));
        let ActionPreflight::Vacant(permit) = ActionCoordinator::preflight(&store, ingress)
            .await
            .expect("provider-policy preflight")
        else {
            panic!("{kind:?} did not retain vacant action authority")
        };
        ActionCoordinator::prepare_and_stage_decision_response(
            permit,
            &mut engine,
            fence.clone(),
            &response,
        )
        .unwrap_or_else(|error| panic!("{kind:?} action preparation failed: {error}"));
        let staged = engine
            .take_staged_action()
            .expect("provider-policy admission")
            .expect("provider-policy staged action");
        let (bundle, installation) = staged.into_parts();
        bundles.push(serde_json::to_value(&*bundle).expect("canonical bundle JSON"));
        engine
            .abort_prepared_action(installation)
            .expect("the fixture aborts");
    }

    assert!(bundles.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(
        engine
            .action_state()
            .get(
                &registration.families.selections,
                &CanonicalId::new("selected").unwrap(),
            )
            .unwrap(),
        None,
        "aborted latency variants must not install provider state"
    );
}

#[tokio::test]
async fn provider_and_fallback_race_selects_one_exact_action_result() {
    let (mut engine, registration, capture_plan, keys) = decision_harness(30);
    let pending = arm_decision_capture(
        &engine,
        &keys,
        &registration,
        &capture_plan,
        0x5e,
        unix_now_s() + 60,
    );
    let store = InMemoryStore::new();
    commit_armed_capture(&mut engine, &store, &registration, 0x6e).await;
    let (fence, provider_response, _worker) =
        lease_captured_decision(&mut engine, &registration, &pending, &keys);
    let provider_ingress =
        ActionCoordinator::decision_response_ingress(&engine, &fence, &provider_response)
            .expect("provider contender ingress");

    let leased = engine
        .action_state()
        .get(&registration.families.pending, &pending.id)
        .unwrap()
        .unwrap()
        .clone();
    let fallback = leased
        .plan_fallback_ready(
            leased.revision,
            fence.lease_until(),
            mush_core::DecisionFallbackReason::ProviderUnavailable,
        )
        .expect("fallback race transition")
        .post_image()
        .clone();
    let mut txn = engine
        .action_transaction(&registration.families.pending)
        .expect("fallback race transaction");
    txn.replace(&registration.families.pending, &pending.id, fallback)
        .expect("fallback race post-image");
    let (_, installation) = txn.prepare().unwrap().into_parts();
    engine
        .install_composed_action(installation)
        .expect("the fixture installs");

    let page = engine
        .discover_decision_dispatches(None, 8)
        .expect("fallback discovery");
    let candidate = page
        .fallbacks()
        .first()
        .expect("fallback candidate")
        .clone();
    let fallback_response = engine
        .select_decision_fallback(&candidate)
        .expect("deterministic fallback selection");
    let fallback_ingress = engine
        .decision_fallback_ingress(&candidate, &fallback_response)
        .expect("fallback contender ingress");
    assert_eq!(fallback_response, provider_response);
    assert_eq!(fallback_ingress, provider_ingress);

    let ActionPreflight::Vacant(permit) = ActionCoordinator::preflight(&store, fallback_ingress)
        .await
        .unwrap()
    else {
        panic!("fallback contender must initially be vacant")
    };
    ActionCoordinator::prepare_and_stage_decision_fallback(
        permit,
        &mut engine,
        &candidate,
        &fallback_response,
    )
    .expect("fallback contender preparation");
    let staged = engine
        .take_staged_action()
        .expect("fallback action admission")
        .expect("fallback action staged");
    let (bundle, installation) = staged.into_parts();
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await
        .expect("fallback intent");
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .expect("fallback reconcile");
    engine
        .install_composed_action(installation)
        .expect("the fixture installs");

    assert!(matches!(
        ActionCoordinator::preflight(&store, provider_ingress)
            .await
            .expect("late provider preflight"),
        ActionPreflight::Finalized(_)
    ));
    assert_eq!(
        engine
            .action_state()
            .get(
                &registration.families.selections,
                &CanonicalId::new("selected").unwrap(),
            )
            .unwrap()
            .expect("one selected action")
            .count,
        1
    );
}

// ------------------------------------------------------------------
// ---- convergence fold ----
// ------------------------------------------------------------------

/// A composed module that owns no mechanics at all. It exists only so a
/// decision-source scan has a **second** mechanics owner to leave alone.
struct CvBystanderModule;

impl GameModule for CvBystanderModule {
    fn name(&self) -> &'static str {
        "cv_bystander"
    }

    fn register(&mut self, _reg: &mut Registrar<'_>) -> Result<(), mush_core::ModError> {
        Ok(())
    }
}

/// 08j S6 — a malformed decision freezes its **owner**, not the world.
///
/// 09 §K7.4 says a mismatched pending/prompt pair, a duplicate identity or
/// an impossible transition freezes the decision owner. Inside
/// `DecisionSourceRegistry::scan` those three were raised by a bare `?`
/// into the unattributed `DecisionSourceScanError::Decision`, so
/// `DurableDecisionDriver::drive_one` left `failure_owner` unset and
/// `world_task` took the `else` branch into a whole-world
/// `WorldMode::IntegrityFrozen`. The scan now names the loop's owner, the
/// driver freezes exactly that owner, and every other owner keeps running.
#[tokio::test]
async fn cv_6_a_malformed_decision_freezes_its_owner_not_every_other_owner() {
    let (mut engine, registration, capture_plan, keys) =
        decision_harness_with(30, vec![Box::new(CvBystanderModule)]);
    let pending = arm_decision_capture(
        &engine,
        &keys,
        &registration,
        &capture_plan,
        0x7a,
        unix_now_s() + 60,
    );
    let store = InMemoryStore::new();
    commit_armed_capture(&mut engine, &store, &registration, 0x7b).await;

    // Break the pending <-> prompt linkage exactly the way a hand-patched
    // or half-migrated record would: same durable identity, a revision the
    // prompt record does not carry. `validate_prompt_pair` answers
    // `DecisionError::PromptLinkage`.
    let mut tampered = engine
        .action_state()
        .get(&registration.families.pending, &pending.id)
        .unwrap()
        .unwrap()
        .clone();
    tampered.revision = tampered.revision.saturating_add(1);
    let mut txn = engine
        .action_transaction(&registration.families.pending)
        .expect("tamper transaction");
    txn.replace(&registration.families.pending, &pending.id, tampered)
        .expect("tampered post-image");
    let (_, installation) = txn.prepare().unwrap().into_parts();
    engine
        .install_composed_action(installation)
        .expect("the fixture installs");

    let owner = OwnerId::new(DECISION_HARNESS_OWNER).expect("static harness owner");
    let bystander = OwnerId::new("cv_bystander").expect("static bystander owner");
    let error = engine
        .discover_decision_dispatches(None, 8)
        .expect_err("a broken prompt pair must fail the scan closed");
    assert!(
        matches!(
            error,
            mush_core::DecisionSourceScanError::OwnedDecision { .. }
        ),
        "the fault must be attributed, not dropped: {error}"
    );
    assert_eq!(
        error.owner(),
        Some(&owner),
        "the scan loop names the decision owner"
    );

    // This is what `DurableDecisionDriver::drive_one` does once
    // `failure_owner` is set; the `else` branch is the whole-world freeze.
    engine
        .registry
        .freeze_mechanics_owner(
            &owner,
            mush_core::IncidentId(0x5106),
            mush_core::RecoveryReason::IntegrityViolation {
                code: CanonicalId::new("decision_recovery").expect("static recovery code"),
            },
        )
        .expect("freeze the decision owner");
    assert!(
        engine
            .registry
            .mechanics_owner_freeze(&owner)
            .expect("decision owner readiness")
            .is_some(),
        "the decision owner is quarantined"
    );
    assert!(
        engine
            .registry
            .mechanics_owner_freeze(&bystander)
            .expect("bystander readiness")
            .is_none(),
        "a malformed decision from one owner freezes nothing of another"
    );

    // And the registry keeps scanning: the quarantined owner's records are
    // skipped, the world is not stopped.
    let page = engine
        .discover_decision_dispatches(None, 8)
        .expect("the scan continues past a quarantined owner");
    assert!(page.candidates().is_empty());
}

#[tokio::test]
async fn delayed_tactical_gate4_stale_matrix_preserves_unselected_target_movement() {
    #[derive(Clone, Copy)]
    enum Change {
        SelectedPlacement,
        UnselectedPlacement,
        StationGrant,
        ControlLease,
        SourceComponent,
        SourceResource,
        Contact,
        Window,
        DefinitionOwner,
        BridgeOwner,
    }

    impl Change {
        const fn remains_admissible(self) -> bool {
            matches!(self, Self::UnselectedPlacement)
        }
    }

    async fn run(change: Change, salt: u8) {
        let TacticalDecisionHarness {
            mut engine,
            registration,
            tactical_state,
            unit_spec,
            capture_plan,
            keys,
        } = tactical_decision_harness(30);
        let source = GOD;
        let selected_target = mush_core::ObjId(971);
        let unselected_target = mush_core::ObjId(972);
        for object in [source, selected_target, unselected_target] {
            install_tactical_gate_unit(
                &mut engine,
                &tactical_state,
                TacticalUnit::from_spec(&unit_spec, object).expect("valid tactical unit"),
            );
        }
        install_tactical_gate_placement(&mut engine, &tactical_state, source, 0, 0);
        install_tactical_gate_placement(&mut engine, &tactical_state, selected_target, 1, 0);
        install_tactical_gate_placement(&mut engine, &tactical_state, unselected_target, 2, 0);
        install_tactical_gate_weapons_lease(
            &mut engine,
            &tactical_state,
            source,
            PrincipalId::Object { id: source },
        );
        publish_tactical_gate_weapons_station(&mut engine, source, source, 1);
        install_tactical_gate_contact(
            &mut engine,
            &tactical_state,
            mush_core::ContactId::from_bytes([0x11; 16]),
            selected_target,
            PrincipalId::Object { id: source },
        );
        install_tactical_gate_contact(
            &mut engine,
            &tactical_state,
            mush_core::ContactId::from_bytes([0x22; 16]),
            unselected_target,
            PrincipalId::Object { id: source },
        );
        install_tactical_gate_fire_window(&mut engine, &tactical_state);

        assert_eq!(engine.world.command_subjects_of(source).len(), 1);
        assert_eq!(
            engine
                .action_state()
                .scan_family(&tactical_state.contacts, None, 8)
                .expect("contact scan")
                .entries()
                .len(),
            2
        );
        assert!(
            engine
                .action_state()
                .get(&tactical_state.units, &source)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            engine
                .clock_now(&mush_contract_tests::tactical_gate::tactical_gate_clock())
                .expect("tactical clock")
                .at,
            7
        );

        let principal = PrincipalId::Object { id: source };
        let query = engine
            .legal_action_query(principal, source, PerceivedRef::SelfActor)
            .expect("tactical decision query");
        let projection = engine
            .project_decision_offers(&query)
            .expect("tactical decision projection");
        let fire_index = projection
            .actions()
            .iter()
            .position(|offer| offer.presentation().verb == "fire")
            .unwrap_or_else(|| panic!("fire offer missing from {projection:?}"));
        let projected = projection.actions()[fire_index].clone();
        let witness = projection.offered()[fire_index].clone();
        assert_eq!(witness.offer, projected.offer());
        let _ = engine.world.take_dirty();
        let _ = engine.take_module_ops();
        let pending = arm_tactical_decision_capture(
            &keys,
            &registration,
            &capture_plan,
            source,
            projected,
            witness,
            salt,
            unix_now_s() + 60,
        );
        let store = InMemoryStore::new();
        commit_armed_capture(&mut engine, &store, &registration, salt.wrapping_add(0x10)).await;
        let (fence, response, _worker) =
            lease_captured_decision(&mut engine, &registration, &pending, &keys);
        let (chosen_contact, chosen_target) = match response.targets.as_slice() {
            [PerceivedRef::Contact { id, .. }]
                if id == &mush_core::ContactId::from_bytes([0x11; 16]) =>
            {
                (*id, selected_target)
            }
            [PerceivedRef::Contact { id, .. }]
                if id == &mush_core::ContactId::from_bytes([0x22; 16]) =>
            {
                (*id, unselected_target)
            }
            _ => panic!("unexpected tactical response target"),
        };
        let other_target = if chosen_target == selected_target {
            unselected_target
        } else {
            selected_target
        };
        let source_before_change = engine
            .action_state()
            .canonical(&tactical_state.units, &source)
            .expect("source canonical state")
            .expect("source state")
            .as_slice()
            .to_vec();
        let selected_before_change = engine
            .action_state()
            .canonical(&tactical_state.units, &chosen_target)
            .expect("target canonical state")
            .expect("target state")
            .as_slice()
            .to_vec();

        match change {
            Change::SelectedPlacement | Change::UnselectedPlacement => {
                move_tactical_gate_placement(
                    &mut engine,
                    &tactical_state,
                    if matches!(change, Change::SelectedPlacement) {
                        chosen_target
                    } else {
                        other_target
                    },
                    4,
                    0,
                );
            }
            Change::StationGrant => {
                assert_eq!(
                    engine.world.retract_command_subjects(
                        mush_contract_tests::tactical_gate::TACTICAL_GATE_OWNER,
                        Some(source),
                    ),
                    1
                );
                let _ = engine.world.take_dirty();
            }
            Change::ControlLease => {
                let channel = mush_core::ChannelId::new("weapons").unwrap();
                let key = mush_core::ControlLeaseKey::new(source, channel);
                let lease = engine
                    .action_state()
                    .get(&tactical_state.control_leases, &key)
                    .unwrap()
                    .unwrap()
                    .clone();
                let request = RequestUid::from_bytes([salt.wrapping_add(2); 16]);
                let requested = lease
                    .checked_request_handoff(
                        lease.revision(),
                        mush_core::ControlHandoffDraft {
                            request,
                            requested_by: PrincipalId::Object { id: source },
                            next_holder: None,
                            requested_at: mush_contract_tests::tactical_gate::tactical_gate_tick(7),
                            boundary: mush_contract_tests::tactical_gate::tactical_gate_tick(8),
                            reason: CanonicalId::new("gate4_revoke").unwrap(),
                        },
                    )
                    .unwrap();
                let revoked = requested
                    .checked_apply_handoff(
                        requested.revision(),
                        request,
                        mush_contract_tests::tactical_gate::tactical_gate_tick(8),
                    )
                    .unwrap();
                let mut txn = engine
                    .action_transaction(&tactical_state.control_leases)
                    .expect("lease-revoke transaction");
                txn.replace(&tactical_state.control_leases, &key, revoked)
                    .expect("lease-revoke post-image");
                let (_, installation) = txn.prepare().unwrap().into_parts();
                engine
                    .install_composed_action(installation)
                    .expect("the fixture installs");
            }
            Change::SourceComponent => {
                disable_tactical_gate_weapon(&mut engine, &tactical_state, source);
            }
            Change::SourceResource => {
                exhaust_tactical_gate_ammo(&mut engine, &tactical_state, source);
            }
            Change::Contact => {
                let contact = engine
                    .action_state()
                    .get(&tactical_state.contacts, &chosen_contact)
                    .unwrap()
                    .unwrap()
                    .clone();
                let lost = contact
                    .checked_mark_lost(
                        contact.revision(),
                        mush_contract_tests::tactical_gate::tactical_gate_tick(7),
                        CanonicalId::new("gate4_occluded").unwrap(),
                    )
                    .unwrap();
                let mut txn = engine
                    .action_transaction(&tactical_state.contacts)
                    .expect("contact-loss transaction");
                txn.replace(&tactical_state.contacts, &chosen_contact, lost)
                    .expect("contact-loss post-image");
                let (_, installation) = txn.prepare().unwrap().into_parts();
                engine
                    .install_composed_action(installation)
                    .expect("the fixture installs");
            }
            Change::Window => {
                let id = mush_contract_tests::tactical_gate::tactical_gate_window_id();
                let window = engine
                    .action_state()
                    .get(&tactical_state.windows, &id)
                    .unwrap()
                    .unwrap()
                    .clone();
                let closed = window
                    .checked_close_due(
                        window.revision(),
                        RequestUid::from_bytes([salt.wrapping_add(1); 16]),
                        &mush_contract_tests::tactical_gate::tactical_gate_tick(10),
                    )
                    .unwrap()
                    .into_record();
                let mut txn = engine
                    .action_transaction(&tactical_state.windows)
                    .expect("window-close transaction");
                txn.replace(&tactical_state.windows, &id, closed)
                    .expect("window-close post-image");
                let (_, installation) = txn.prepare().unwrap().into_parts();
                engine
                    .install_composed_action(installation)
                    .expect("the fixture installs");
            }
            Change::BridgeOwner => {
                engine
                    .registry
                    .freeze_mechanics_owner(
                        &OwnerId::new(mush_contract_tests::tactical_gate::TACTICAL_PLACEMENT_OWNER)
                            .unwrap(),
                        mush_core::IncidentId(u64::from(salt)),
                        mush_core::RecoveryReason::IntegrityViolation {
                            code: CanonicalId::new("gate4_bridge_freeze").unwrap(),
                        },
                    )
                    .expect("bridge owner freeze");
            }
            Change::DefinitionOwner => {
                engine
                    .registry
                    .freeze_mechanics_owner(
                        &OwnerId::new(mush_contract_tests::tactical_gate::TACTICAL_GATE_OWNER)
                            .unwrap(),
                        mush_core::IncidentId(u64::from(salt)),
                        mush_core::RecoveryReason::IntegrityViolation {
                            code: CanonicalId::new("gate4_definition_freeze").unwrap(),
                        },
                    )
                    .expect("definition owner freeze");
            }
        }
        let (source_before, selected_before) = if matches!(change, Change::DefinitionOwner) {
            (source_before_change, selected_before_change)
        } else {
            (
                engine
                    .action_state()
                    .canonical(&tactical_state.units, &source)
                    .expect("source canonical state after stale change")
                    .expect("source state after stale change")
                    .as_slice()
                    .to_vec(),
                engine
                    .action_state()
                    .canonical(&tactical_state.units, &chosen_target)
                    .expect("target canonical state after stale change")
                    .expect("target state after stale change")
                    .as_slice()
                    .to_vec(),
            )
        };
        let ingress = ActionCoordinator::decision_response_ingress(&engine, &fence, &response)
            .expect("exact live decision response ingress");
        let ActionPreflight::Vacant(permit) = ActionCoordinator::preflight(&store, ingress)
            .await
            .expect("tactical decision preflight")
        else {
            panic!("tactical decision idempotency must be vacant")
        };
        let result = ActionCoordinator::prepare_and_stage_decision_response(
            permit,
            &mut engine,
            fence,
            &response,
        );

        if matches!(change, Change::SelectedPlacement) {
            assert!(matches!(
                &result,
                Err(crate::action_coordinator::ActionCoordinatorError::StaleGuards)
            ));
        }

        if !matches!(change, Change::DefinitionOwner) {
            assert_eq!(
                engine
                    .action_state()
                    .canonical(&tactical_state.units, &source)
                    .unwrap()
                    .unwrap()
                    .as_slice(),
                source_before
            );
            assert_eq!(
                engine
                    .action_state()
                    .canonical(&tactical_state.units, &chosen_target)
                    .unwrap()
                    .unwrap()
                    .as_slice(),
                selected_before
            );
        }
        if !change.remains_admissible() {
            assert!(
                result.is_err(),
                "stale delayed action must fail before prepare"
            );
            assert!(engine.take_staged_action().unwrap().is_none());
            assert!(!engine.action_state().has_prepared());
        } else {
            result.expect("unselected movement must remain admissible");
            let staged = engine
                .take_staged_action()
                .expect("selected action admission")
                .expect("selected action staged");
            let (bundle, installation) = staged.into_parts();
            assert!(bundle.resolution().is_some(), "RNG evidence is durable");
            assert_eq!(bundle.semantic_events().len(), 1);
            assert!(bundle.durable_outbox().is_empty());
            engine
                .abort_prepared_action(installation)
                .expect("the fixture aborts");
        }
    }

    for (change, salt) in [
        (Change::SelectedPlacement, 0x61),
        (Change::UnselectedPlacement, 0x62),
        (Change::StationGrant, 0x63),
        (Change::ControlLease, 0x64),
        (Change::SourceComponent, 0x65),
        (Change::SourceResource, 0x66),
        (Change::Contact, 0x67),
        (Change::Window, 0x68),
        (Change::DefinitionOwner, 0x69),
        (Change::BridgeOwner, 0x6a),
    ] {
        run(change, salt).await;
    }
}

#[tokio::test]
async fn durable_schedule_poll_executes_one_atomic_cursor_and_domain_bundle() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init();
    use mush_contract_tests::{
        TOY_OWNER, install_toy_focus, install_toy_scheduled_expiry, toy_brace_invocation,
        toy_definitions, toy_world_with_state,
    };
    use mush_core::{
        DurableSchedule, DurableScheduleDraft, ScheduleCatchUpOrder, ScheduleId,
        ScheduleRestartPolicy,
    };

    let (mut world, families) = toy_world_with_state();
    install_toy_focus(world.engine_mut(), &families, GOD, 1);
    let invocation = toy_brace_invocation(GOD, 11, RequestUid::from_bytes([0x31; 16]));
    let (prepared, _) = world
        .engine_mut()
        .prepare_registered_authoritative_action(
            &toy_definitions().brace,
            &invocation,
            DurableCommitToken::from_bytes([0x32; 16]),
        )
        .expect("prepare guarded fixture");
    let (_, installation) = prepared.into_parts();
    world
        .engine_mut()
        .install_composed_action(installation)
        .expect("the fixture installs");
    let effect = world
        .engine()
        .action_state()
        .get(&families.guarded, &GOD)
        .unwrap()
        .unwrap()
        .effect
        .id();
    let owner = OwnerId::new(TOY_OWNER).unwrap();
    let schedule = DurableSchedule::new(DurableScheduleDraft {
        id: ScheduleId::derive(&owner, &CanonicalId::new("world_poll_expiry").unwrap()).unwrap(),
        owner,
        clock: mush_core::ClockRef::Utc,
        next_due: mush_core::ClockStamp {
            clock: mush_core::ClockRef::Utc,
            at: 0,
        },
        cadence: None,
        generation: 1,
        next_ordinal: 0,
        restart: ScheduleRestartPolicy::Resume,
        catch_up_order: ScheduleCatchUpOrder::OldestFirst,
        max_catch_up: 1,
    })
    .unwrap();
    let schedule_key = format!(
        "action-state/v1/schedules/{}",
        schedule.id().canonical_hash().to_hex()
    );
    install_toy_scheduled_expiry(world.engine_mut(), &families, schedule, GOD, effect);

    let deps = ServerDeps::in_memory();
    let store = Arc::clone(&deps.store);
    let handles =
        spawn_persist_with_gateway(world.into_engine(), deps, |_| Arc::new(BlackholeGateway));
    let mut completed = false;
    for _ in 0..40 {
        if store
            .module_json_get(TOY_OWNER, &schedule_key)
            .await
            .unwrap()
            .is_some_and(|document| document["value"]["state"] == "completed")
        {
            completed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let persisted = store.module_json_list(TOY_OWNER).await.unwrap();
    assert!(
        completed,
        "durable schedule was not committed by the wake driver: {persisted:?}"
    );
    assert!(
        store
            .module_json_get(TOY_OWNER, "action-state/v1/guarded/1")
            .await
            .unwrap()
            .is_none(),
        "the cursor and effect removal must share one authoritative commit"
    );

    assert!(
        handles
            .handle
            .inbound(Inbound::Control(ControlMsg::Shutdown))
            .await
    );
    handles.world_join.await.unwrap();
    handles.writer_join.await.unwrap();
    handles.outbox_join.await.unwrap();
}

#[tokio::test]
async fn durable_schedule_driver_rebases_retired_server_epoch() {
    use mush_contract_tests::{
        TOY_OWNER, install_toy_focus, install_toy_scheduled_expiry, toy_brace_invocation,
        toy_definitions, toy_world_with_state,
    };
    use mush_core::{
        DurableSchedule, DurableScheduleDraft, ScheduleCatchUpOrder, ScheduleId,
        ScheduleRestartPolicy,
    };

    let (mut world, families) = toy_world_with_state();
    install_toy_focus(world.engine_mut(), &families, GOD, 1);
    let invocation = toy_brace_invocation(GOD, 11, RequestUid::from_bytes([0x41; 16]));
    let (prepared, _) = world
        .engine_mut()
        .prepare_registered_authoritative_action(
            &toy_definitions().brace,
            &invocation,
            DurableCommitToken::from_bytes([0x42; 16]),
        )
        .expect("prepare guarded fixture");
    let (_, installation) = prepared.into_parts();
    world
        .engine_mut()
        .install_composed_action(installation)
        .expect("the fixture installs");
    let effect = world
        .engine()
        .action_state()
        .get(&families.guarded, &GOD)
        .unwrap()
        .unwrap()
        .effect
        .id();
    let owner = OwnerId::new(TOY_OWNER).unwrap();
    let schedule = DurableSchedule::new(DurableScheduleDraft {
        id: ScheduleId::derive(&owner, &CanonicalId::new("retired_epoch_expiry").unwrap()).unwrap(),
        owner,
        clock: mush_core::ClockRef::Utc,
        next_due: mush_core::ClockStamp {
            clock: mush_core::ClockRef::Utc,
            at: 90,
        },
        cadence: None,
        generation: 1,
        next_ordinal: 0,
        restart: ScheduleRestartPolicy::Resume,
        catch_up_order: ScheduleCatchUpOrder::OldestFirst,
        max_catch_up: 1,
    })
    .unwrap();
    install_toy_scheduled_expiry(world.engine_mut(), &families, schedule, GOD, effect);
    world.world_mut().set_now(100);
    let candidate = world
        .engine()
        .discover_scheduled_actions(mush_core::ScheduleDriveCause::Live, None, 1)
        .unwrap()
        .candidates()[0]
        .clone();
    let old_ingress =
        ActionCoordinator::scheduled_ingress(candidate.action(), candidate.advance(), 1).unwrap();
    let store = mush_store::InMemoryStore::new();
    store
        .retire_authoritative_epochs(old_ingress.idempotency().scope(), 9)
        .await
        .expect("seed retired scheduler epoch");
    let _ = world.drain_write_batch();
    let mut engine = world.into_engine();
    let mut driver = DurableScheduleDriver::new(1);

    assert!(matches!(
        driver.drive_one(&mut engine, &store).await.unwrap(),
        DurableScheduleDrive::Staged(_)
    ));
    let staged = engine
        .take_staged_action()
        .unwrap()
        .expect("rebased schedule action staged");
    let (bundle, installation) = staged.into_parts();
    assert_eq!(bundle.idempotency().epoch(), 9);
    assert_eq!(driver.idempotency_epoch, 9);
    engine
        .abort_prepared_action(installation)
        .expect("the fixture aborts");
}

#[tokio::test]
async fn durable_schedule_driver_discards_stale_retained_candidate() {
    use mush_contract_tests::{
        TOY_OWNER, install_toy_focus, install_toy_scheduled_expiry, toy_brace_invocation,
        toy_definitions, toy_world_with_state,
    };
    use mush_core::{
        DurableSchedule, DurableScheduleDraft, ScheduleCatchUpOrder, ScheduleId,
        ScheduleRestartPolicy,
    };

    let (mut world, families) = toy_world_with_state();
    install_toy_focus(world.engine_mut(), &families, GOD, 1);
    let invocation = toy_brace_invocation(GOD, 11, RequestUid::from_bytes([0x51; 16]));
    let (prepared, _) = world
        .engine_mut()
        .prepare_registered_authoritative_action(
            &toy_definitions().brace,
            &invocation,
            DurableCommitToken::from_bytes([0x52; 16]),
        )
        .expect("prepare guarded fixture");
    let (_, installation) = prepared.into_parts();
    world
        .engine_mut()
        .install_composed_action(installation)
        .expect("the fixture installs");
    let effect = world
        .engine()
        .action_state()
        .get(&families.guarded, &GOD)
        .unwrap()
        .unwrap()
        .effect
        .id();
    let owner = OwnerId::new(TOY_OWNER).unwrap();
    let schedule = DurableSchedule::new(DurableScheduleDraft {
        id: ScheduleId::derive(&owner, &CanonicalId::new("stale_retained").unwrap()).unwrap(),
        owner,
        clock: mush_core::ClockRef::Utc,
        next_due: mush_core::ClockStamp {
            clock: mush_core::ClockRef::Utc,
            at: 90,
        },
        cadence: None,
        generation: 1,
        next_ordinal: 0,
        restart: ScheduleRestartPolicy::Resume,
        catch_up_order: ScheduleCatchUpOrder::OldestFirst,
        max_catch_up: 1,
    })
    .unwrap();
    let schedule_id = schedule.id();
    install_toy_scheduled_expiry(world.engine_mut(), &families, schedule, GOD, effect);
    world.world_mut().set_now(100);
    let candidate = world
        .engine()
        .discover_scheduled_actions(mush_core::ScheduleDriveCause::Live, None, 1)
        .unwrap()
        .candidates()[0]
        .clone();

    // A competing authoritative transition changes the cursor after this
    // candidate was retained for retry. The old candidate is ordinary
    // optimistic-concurrency loss, not an integrity breach.
    let mut transaction = world
        .engine_mut()
        .action_transaction(&families.schedules)
        .expect("schedule update transaction");
    transaction
        .replace(
            &families.schedules,
            &schedule_id,
            candidate.advance().post_image().clone(),
        )
        .expect("install competing cursor advance");
    let prepared = transaction.prepare().expect("prepare cursor update");
    let (_, installation) = prepared.into_parts();
    world
        .engine_mut()
        .install_composed_action(installation)
        .expect("the fixture installs");
    let _ = world.drain_write_batch();

    let mut engine = world.into_engine();
    let mut driver = DurableScheduleDriver::new(1);
    driver.retry = Some(candidate);
    let store = mush_store::InMemoryStore::new();
    assert!(matches!(
        driver.drive_one(&mut engine, &store).await.unwrap(),
        DurableScheduleDrive::DiscardedStale(_)
    ));
    assert!(driver.retry.is_none());
    assert!(!engine.action_state().has_prepared());
    assert!(engine.take_staged_action().unwrap().is_none());
}

#[derive(Default)]
struct RecordingGateway {
    discord: Mutex<Vec<(u64, DiscordControlOp)>>,
    sends: Mutex<Vec<(SendTarget, Arc<OutputEvent>)>>,
    /// The length of every `transmit` call, in order (16b cut AP.4: a
    /// bracketed world-task refusal is ONE batch).
    batches: Mutex<Vec<usize>>,
}

impl RecordingGateway {
    fn reply_text(&self, id: u64) -> Option<String> {
        self.sends
            .lock()
            .expect("recording gateway poisoned")
            .iter()
            .find_map(|(_, event)| match event.as_ref() {
                OutputEvent::Reply {
                    id: reply_id,
                    event,
                } if *reply_id == id => match event.as_ref() {
                    OutputEvent::Sys(text) => Some(text.to_plain_text()),
                    other => Some(format!("{other:?}")),
                },
                _ => None,
            })
    }
}

impl GatewaySink for RecordingGateway {
    fn discord_control(&self, character: u64, op: DiscordControlOp) {
        self.discord.lock().unwrap().push((character, op));
    }
    fn transmit(&self, sends: Vec<(SendTarget, Arc<OutputEvent>)>) {
        self.batches
            .lock()
            .expect("recording gateway poisoned")
            .push(sends.len());
        self.sends
            .lock()
            .expect("recording gateway poisoned")
            .extend(sends);
    }

    fn set_flood_exempt(&self, _char: u64, _exempt: bool) {}

    fn is_socket(&self) -> bool {
        false
    }

    fn close_session(&self, _char: u64, _cause: mush_ipc::DetachCause) {}

    fn close_web_sessions(
        &self,
        _chars: Vec<u64>,
        _notice: Option<OutputEvent>,
        _cause: mush_ipc::DetachCause,
    ) {
    }

    fn close_char_connections(&self, _char: u64) -> Vec<u64> {
        Vec::new()
    }

    fn structured_conns_of(&self, _chars: &[u64]) -> Vec<(u64, u64)> {
        Vec::new()
    }

    fn detach_conn_local(&self, _conn: u64) {}

    fn session_counts(&self) -> (i64, i64) {
        (0, 0)
    }
}

#[tokio::test]
async fn all_in_one_offer_submission_reaches_durable_ownership() {
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        base_registry(),
        vec![Box::new(mush_tactics::TacticsModule::new())],
    )
    .expect("tactics module composition");
    engine.set_offer_keys(
        mush_core::OfferKeyRing::new(
            mush_core::OfferSecret::new(1, [0x51; 32]).expect("test offer secret"),
            [],
        )
        .expect("test offer key ring"),
    );

    let store = Arc::new(InMemoryStore::new());
    let recorder = Arc::new(RecordingGateway::default());
    let sink = Arc::clone(&recorder);
    let handles = spawn_persist_with_gateway(
        engine,
        ServerDeps {
            store: store.clone(),
            guard: Arc::new(IpGuard::new()),
            staff: Arc::new(StaffQueue::new()),
            llm: None,
            metrics: Arc::new(Metrics::new()),
            quarantine_spool: crate::quarantine::QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
        },
        move |_| -> Gateway { sink },
    );
    let PersistHandles {
        handle,
        world_join,
        writer_join,
        outbox_join,
        ..
    } = handles;

    const CONN: u64 = 0x5151;
    assert!(
        handle
            .inbound(Inbound::ConnUp {
                conn: CONN,
                actor: GOD,
                caps_structured: true,
                from_linkdead: false,
            })
            .await
    );
    assert!(
        handle.barrier().await,
        "connection must become authoritative"
    );
    let offer = handle
        .legal_actions(CONN, GOD, PerceivedRef::SelfActor)
        .await
        .expect("legal-action projection")
        .into_iter()
        .find(|offer| offer.presentation().verb == "brace")
        .expect("tactics brace offer");

    let ActionSend::Finished(receipt) = handle
        .submit_legal_action(GatewayAction {
            conn: CONN,
            actor: GOD,
            offered_action: offer.offer(),
            subject: offer.subject().clone(),
            targets: Vec::new(),
            parameters: serde_json::json!({}),
        })
        .await
    else {
        panic!("all-in-one action did not return its exact projected receipt");
    };
    assert_eq!(receipt.result().unwrap().summary(), "Brace committed.");
    assert!(handle.barrier().await, "authoritative receipt must settle");
    let stance = store
        .module_json_get("tactics", "action-state/v1/stance/1")
        .await
        .expect("read persisted tactics stance")
        .expect("brace post-image must be durable");
    assert_eq!(stance["value"]["braced"], true);
    assert_eq!(stance["value"]["revision"], 1);

    assert!(handle.inbound(Inbound::Control(ControlMsg::Shutdown)).await);
    tokio::time::timeout(Duration::from_secs(2), world_join)
        .await
        .expect("world shutdown timed out")
        .expect("world task panicked");
    tokio::time::timeout(Duration::from_secs(2), writer_join)
        .await
        .expect("writer shutdown timed out")
        .expect("writer task panicked");
    tokio::time::timeout(Duration::from_secs(2), outbox_join)
        .await
        .expect("outbox shutdown timed out")
        .expect("outbox task panicked");
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedLaneState {
    amount: u8,
}

type PreparedLaneHandle = StateFamilyHandle<u64, PreparedLaneState>;

#[derive(Clone)]
struct PreparedLaneGeneration(u64);

struct PreparedLaneRevision;

impl RevisionValidator for PreparedLaneRevision {
    fn current_generation(
        &self,
        context: &mush_core::RevisionValidatorContext<'_>,
        _key: &RevisionKey,
    ) -> Result<Option<u64>, ActionError> {
        Ok(context
            .module_state::<PreparedLaneGeneration>()
            .map(|generation| generation.0))
    }
}

/// The I13 seam's audience in this fixture: an observer that reports every
/// announced family to GOD, so a test can tell which install path ran.
struct PreparedLaneAnnounce;

impl mush_core::WorldObserver for PreparedLaneAnnounce {
    fn on_event(&self, ctx: &mut mush_core::Ctx<'_>, ev: &WorldEvent) {
        if let WorldEvent::ActionStateChanged {
            owner,
            family,
            keys,
        } = ev
        {
            ctx.out
                .info(GOD, format!("announced {owner}/{family} {keys:?}"));
        }
    }
}

struct PreparedLaneModule {
    captured: Arc<Mutex<Option<PreparedLaneHandle>>>,
}

impl GameModule for PreparedLaneModule {
    fn name(&self) -> &'static str {
        "prepared_lane_test"
    }

    fn register(&mut self, registrar: &mut Registrar<'_>) -> Result<(), mush_core::ModError> {
        registrar.state(PreparedLaneGeneration(1));
        registrar.revision_validator(
            RevisionKind::new(CanonicalId::new("prepared_lane").expect("static revision kind")),
            PreparedLaneRevision,
        )?;
        let descriptor = StateFamilyDescriptor::new(
            StateFamilyId::new("receipt_state").expect("static family id"),
            CanonicalId::new("u64.decimal.v1").expect("static codec id"),
            CanonicalId::new("prepared_lane_state.v1").expect("static schema id"),
            1,
            CanonicalId::new("prepared_lane_state.decode_json.v1").expect("static decoder id"),
            CanonicalId::new("prepared_lane_state.range.v1").expect("static validator id"),
            CanonicalId::new("prepared_lane_state.json.v1").expect("static serializer id"),
        )
        .expect("static state-family descriptor");
        let handle = registrar.state_family(
            descriptor,
            |key: &u64| Ok(key.to_string()),
            |value: &serde_json::Value| {
                serde_json::from_value(value.clone()).map_err(|error| error.to_string())
            },
            |_value: &PreparedLaneState| Ok(()),
            |value: &PreparedLaneState| Ok(serde_json::json!({ "amount": value.amount })),
        )?;
        *self.captured.lock().expect("capture mutex poisoned") = Some(handle);
        registrar.subscribe(
            mush_core::WorldEventKind::ActionStateChanged,
            Arc::new(PreparedLaneAnnounce),
        );
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        _data: ModuleBootData,
    ) -> Result<(), mush_core::ModError> {
        ctx.persist(ModuleOp {
            ns: "prepared_lane_test".into(),
            op: ModuleWrite::JsonSet {
                key: "late_legacy_work".into(),
                value: serde_json::json!({ "queued": true }),
            },
        });
        Ok(())
    }
}

fn prepared_lane_fixture() -> (Engine, PreparedLaneHandle) {
    let captured = Arc::new(Mutex::new(None));
    let module = PreparedLaneModule {
        captured: Arc::clone(&captured),
    };
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(module)],
    )
    .expect("prepared-lane test module composes");
    // The minimal world's seed objects are unrelated legacy dirtiness;
    // prepared action state must enter the dedicated persistence lane.
    engine.world.take_dirty();
    let handle = captured
        .lock()
        .expect("capture mutex poisoned")
        .clone()
        .expect("registration captured state-family handle");
    (engine, handle)
}

fn prepared_lane_guards(engine: &Engine, generation: u64) -> GuardSet {
    let revisions = RevisionSet::from_stamps([RevisionStamp {
        key: RevisionKey {
            owner: OwnerId::new("prepared_lane_test").expect("static owner id"),
            kind: RevisionKind::new(
                CanonicalId::new("prepared_lane").expect("static revision kind"),
            ),
            subject: SubjectKey::Object { id: GOD },
            facet: None,
        },
        generation,
    }])
    .expect("single revision stamp is canonical");
    engine
        .registry
        .server_guard_set_builder(revisions)
        .expect("composed registry can build guards")
        .finish()
        .expect("single revision guard is valid")
}

fn empty_guards(engine: &Engine) -> GuardSet {
    engine
        .registry
        .server_guard_set_builder(RevisionSet::default())
        .expect("composed registry can build guards")
        .finish()
        .expect("empty guard set is valid")
}

fn prepare_lane_installation(
    engine: &mut Engine,
    handle: &PreparedLaneHandle,
    key: u64,
    amount: u8,
) -> PreparedInstallation {
    let mut transaction = engine
        .action_transaction(handle)
        .expect("test transaction begins");
    transaction
        .insert(handle, &key, PreparedLaneState { amount })
        .expect("test state is valid");
    let prepared = transaction.prepare().expect("test transaction prepares");
    let (_module_ops, installation) = prepared.into_parts();
    installation.into()
}

fn prepared_lane_bundle(prepared: &PreparedTxn, identity: u8) -> AuthoritativeBundle {
    let owner = OwnerId::new("prepared_lane_test").expect("static owner");
    let mut definitions = DefinitionRegistryBuilder::new();
    let action = definitions
        .register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("action").unwrap(),
                DefinitionId::new("set_receipt_state").unwrap(),
                1,
                serde_json::json!({}),
            )
            .unwrap(),
        )
        .unwrap();
    let outcome_definition = definitions
        .register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new("outcome").unwrap(),
                DefinitionId::new("receipt_state_set").unwrap(),
                1,
                serde_json::json!({}),
            )
            .unwrap(),
        )
        .unwrap();
    let request = RequestUid::from_bytes([identity; 16]);
    let principal = PrincipalId::Object { id: GOD };
    AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: DurableCommitToken::from_bytes([identity.wrapping_add(1); 16]),
        request,
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal: principal.clone(),
                owner: owner.clone(),
                namespace: CanonicalId::new("set_receipt_state").unwrap(),
            },
            0,
            IdempotencyKey::Request(request),
        ),
        ingress_hash: CanonicalHash::from_bytes([identity.wrapping_add(2); 32]),
        request_hash: CanonicalHash::from_bytes([identity.wrapping_add(3); 32]),
        plan_hash: CanonicalHash::from_bytes([identity.wrapping_add(4); 32]),
        action: action.clone(),
        provider: owner.clone(),
        cause: PreparedCausalRef::new(request, None, 0).unwrap(),
        definitions: vec![action.clone(), outcome_definition.clone()],
        writes: prepared.authoritative_replay_writes(),
        outcome: RulesValue::new(
            outcome_definition.clone(),
            serde_json::json!({ "accepted": true }),
        )
        .unwrap(),
        resolution: None,
        semantic_events: vec![
            mush_core::AuthoritativeSemanticEvent::root(
                PreparedCausalRef::new(request, None, 0).unwrap(),
                &action,
                RulesValue::new(outcome_definition, serde_json::json!({ "accepted": true }))
                    .unwrap(),
                vec![principal.clone()],
            )
            .unwrap(),
        ],
        eligible_witnesses: vec![principal],
        durable_outbox: vec![],
    })
    .unwrap()
}

/// Play the writer's side of one queued K3 job against an in-memory store:
/// reserve the exact intent, release ownership, then reconcile and send the
/// exact receipt. Returns the bundle the world actually queued.
async fn settle_queued_authoritative_job(job: StoreJob) -> AuthoritativeBundle {
    let StoreJob::Authoritative {
        bundle,
        ownership,
        notify,
    } = job
    else {
        panic!("the prepared lane must queue one exact authoritative bundle");
    };
    let store = InMemoryStore::new();
    let token = bundle.token();
    let hash = bundle.bundle_hash();
    let intent = store
        .reserve_authoritative_intent((*bundle).clone())
        .await
        .expect("the exact intent reserves");
    ownership
        .send(Ok(intent))
        .expect("the world still owns the ownership channel");
    let exact = store
        .reconcile_authoritative(token, hash)
        .await
        .expect("the exact receipt reconciles");
    notify
        .send(Ok(exact))
        .expect("the world still owns the exact receipt");
    *bundle
}

fn pending_with_prepared_installation(
    serial: u64,
    installation: PreparedInstallation,
) -> PendingCommit {
    let (_receipt_tx, receipt) = oneshot::channel();
    PendingCommit {
        token: CommitToken::from_serial(serial),
        receipt: PendingReceipt::Legacy {
            expected_serial: serial,
            receiver: receipt,
        },
        sends: Vec::new(),
        effects: Vec::new(),
        failure_actor: None,
        create: None,
        action: None,
        prepared_installation: Some(installation),
        barriers: Vec::new(),
        coalesced_reattach: None,
        exit_after_ack: false,
    }
}

#[test]
fn legacy_module_writes_cannot_enter_the_action_state_keyspace() {
    let ordinary = ModuleOp {
        ns: "toy".into(),
        op: ModuleWrite::JsonSet {
            key: "ordinary".into(),
            value: serde_json::json!(true),
        },
    };
    assert!(legacy_reserved_action_state_violation(&[ordinary]).is_none());

    for op in [
        ModuleWrite::JsonSet {
            key: "action-state/v1/family/1".into(),
            value: serde_json::json!(true),
        },
        ModuleWrite::Del {
            key: "action-state/v2/family/1".into(),
        },
        ModuleWrite::StreamAppend {
            key: "action-state/future".into(),
            value: serde_json::json!(true),
            maxlen: 1,
        },
    ] {
        let violation = legacy_reserved_action_state_violation(&[ModuleOp {
            ns: "toy".into(),
            op,
        }]);
        assert!(
            violation
                .as_deref()
                .is_some_and(|detail| detail.contains("coordinator-reserved"))
        );
    }
}

#[derive(Clone, Debug, Default)]
struct GateProbeState {
    mutations: usize,
}

#[derive(Default)]
struct GateProbeCounts {
    command: AtomicUsize,
    input_context: AtomicUsize,
    connection_hook: AtomicUsize,
    observer: AtomicUsize,
    async_apply: AtomicUsize,
    timer: AtomicUsize,
}

impl GateProbeCounts {
    fn total(&self) -> usize {
        [
            &self.command,
            &self.input_context,
            &self.connection_hook,
            &self.observer,
            &self.async_apply,
            &self.timer,
        ]
        .into_iter()
        .map(|counter| counter.load(AtomicOrdering::SeqCst))
        .sum()
    }
}

fn record_probe_mutation(ctx: &mut Ctx<'_>, counter: &AtomicUsize, key: &str) {
    counter.fetch_add(1, AtomicOrdering::SeqCst);
    ctx.state_mut::<GateProbeState>()
        .expect("gate probe state installed")
        .mutations += 1;
    ctx.persist(ModuleOp {
        ns: "gate_probe".into(),
        op: ModuleWrite::JsonSet {
            key: key.into(),
            value: serde_json::json!({ "executed": true }),
        },
    });
}

struct ProbeConnected(Arc<GateProbeCounts>);

impl EventHook for ProbeConnected {
    fn connected(&self, ctx: &mut Ctx<'_>, _actor: ObjId) {
        record_probe_mutation(ctx, &self.0.connection_hook, "connection_hook");
    }
}

struct ProbeObserver(Arc<GateProbeCounts>);

impl WorldObserver for ProbeObserver {
    fn on_event(&self, ctx: &mut Ctx<'_>, _ev: &WorldEvent) {
        record_probe_mutation(ctx, &self.0.observer, "observer");
    }
}

struct ProbeAsync(Arc<GateProbeCounts>);

impl AsyncCompletion for ProbeAsync {
    fn on_complete(&self, ctx: &mut Ctx<'_>, _correlation: u64, _payload: &str) {
        record_probe_mutation(ctx, &self.0.async_apply, "async_apply");
    }
}

struct ProbeTick(Arc<GateProbeCounts>);

impl TickHandler for ProbeTick {
    fn tick(&self, ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<mush_core::SceneId>) {
        record_probe_mutation(ctx, &self.0.timer, "timer");
        ctx.out.defer(Deferred::Effect(Effect::SlowTest {
            actor: GOD,
            correlation: 99,
            seconds: 0,
        }));
    }
}

struct ProbeQuery {
    input_actor: ObjId,
}

impl QueryHandler for ProbeQuery {
    fn query(
        &self,
        world: &World,
        _actor: ObjId,
        _params: &serde_json::Value,
    ) -> Result<OutputEvent, RichText> {
        let mutations = world
            .module_state::<GateProbeState>()
            .expect("gate probe state installed")
            .mutations;
        Ok(OutputEvent::Sys(RichText::plain(format!(
            "objects={} mutations={mutations} input_depth={}",
            world.iter().count(),
            world.input_depth(self.input_actor)
        ))))
    }
}

struct GateProbeModule {
    counts: Arc<GateProbeCounts>,
    input_actor: ObjId,
}

impl GameModule for GateProbeModule {
    fn name(&self) -> &'static str {
        "gate_probe"
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), mush_core::ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.state(GateProbeState::default());

        let counts = Arc::clone(&self.counts);
        reg.command(
            CommandSpec {
                name: "gate-mutate".into(),
                shape: ArgShape::None,
                help: "test-only gate mutation".into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                counts.command.fetch_add(1, AtomicOrdering::SeqCst);
                let created = ctx
                    .w
                    .create(ObjKind::Thing, "gate-leak", ctx.actor, Some(LIMBO))
                    .map_err(|error| CmdError::msg(error.to_string()))?;
                ctx.emit_world_event(&WorldEvent::ObjCreated {
                    obj: created,
                    kind: ObjKind::Thing,
                    creator: ctx.actor,
                });
                ctx.state_mut::<GateProbeState>()
                    .expect("gate probe state installed")
                    .mutations += 1;
                ctx.persist(ModuleOp {
                    ns: "gate_probe".into(),
                    op: ModuleWrite::JsonSet {
                        key: "command".into(),
                        value: serde_json::json!({ "executed": true }),
                    },
                });
                ctx.out.defer(Deferred::Effect(Effect::SlowTest {
                    actor: ctx.actor,
                    correlation: 7,
                    seconds: 0,
                }));
                Ok(())
            }),
        )?;
        reg.hook(
            HookPoint::Connected,
            Arc::new(ProbeConnected(Arc::clone(&self.counts))),
        );
        reg.subscribe(
            WorldEventKind::ObjCreated,
            Arc::new(ProbeObserver(Arc::clone(&self.counts))),
        );
        reg.tick(
            "due",
            TickCadence::Fast(Duration::from_millis(1)),
            Arc::new(ProbeTick(Arc::clone(&self.counts))),
        )?;
        reg.async_completion(Arc::new(ProbeAsync(Arc::clone(&self.counts))))?;
        reg.query_handler(
            "gate_probe.snapshot",
            Arc::new(ProbeQuery {
                input_actor: self.input_actor,
            }),
        )?;
        Ok(())
    }
}

/// 16b cut AC.1 (16c `B-82`): a probe that stands where the tacsim module
/// stands on the effect dispatcher. Its `name()` is `tacsim::store::NS`, the
/// name the dispatcher's tacsim arm matches AND the name
/// `mush_core::apply_async_completion` routes the re-entry by; no real
/// tacsim module is composed in this rig, so the name is free.
struct DeliberationProbeModule {
    /// The re-entered completion payload, once it arrives.
    reply: Arc<Mutex<Option<String>>>,
}

struct DeliberationProbeCompletion(Arc<Mutex<Option<String>>>);

impl AsyncCompletion for DeliberationProbeCompletion {
    fn on_complete(&self, _ctx: &mut Ctx<'_>, _correlation: u64, payload: &str) {
        *self.0.lock().expect("the probe's reply slot") = Some(payload.to_string());
    }
}

impl GameModule for DeliberationProbeModule {
    fn name(&self) -> &'static str {
        tacsim::store::NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), mush_core::ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.command(
            CommandSpec {
                name: "probe-deliberate".into(),
                shape: ArgShape::None,
                help: "test-only tacsim deliberation effect".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let payload =
                    serde_json::to_string(&tacsim::deliberation::DeliberationJob::Deliberate {
                        brief: probe_squad_brief(),
                    })
                    .expect("a brief encodes");
                ctx.out.defer(Deferred::Effect(Effect::ModuleAsync {
                    module: SmolStr::new_static(tacsim::store::NS),
                    actor: GOD,
                    correlation: 1,
                    payload,
                }));
                Ok(())
            }),
        )?;
        reg.async_completion(Arc::new(DeliberationProbeCompletion(Arc::clone(
            &self.reply,
        ))))?;
        Ok(())
    }
}

/// The `SquadBrief` of `tacsim::deliberation`'s own round-trip test
/// (`a_job_and_a_reply_round_trip_their_wire_forms`), copied: every field
/// is public, and `contacts` stays empty so the un-re-exported
/// `BriefContact` is never named.
fn probe_squad_brief() -> tacsim::deliberation::SquadBrief {
    use tacsim::deliberation::{BriefMember, SquadBrief, SquadId};
    SquadBrief {
        squad: SquadId {
            team: tacsim::arena::state::PartyId::RED,
            index: 1,
        },
        match_id: 7,
        commander: 4,
        // 17b cut L.1: the squad's lance-leader, never in the prompt.
        leader: tacsim::deliberation::CharacterRef::Persona {
            squad: SquadId {
                team: tacsim::arena::state::PartyId::RED,
                index: 1,
            },
        },
        reason: "no_order".to_string(),
        units: 2,
        health_permille: 1_000,
        states: vec![("patrol".to_string(), 2)],
        cell: Some((1, 1)),
        window: (16, 16),
        default_ttl_ms: 30_000,
        contacts: Vec::new(),
        members: vec![
            BriefMember {
                unit: 4,
                ordinal: 1,
                state: "patrol".to_string(),
                health_permille: 1_000,
                band: "here".to_string(),
                bearing_deg: 0,
            },
            BriefMember {
                unit: 5,
                ordinal: 2,
                state: "patrol".to_string(),
                health_permille: 1_000,
                band: "close".to_string(),
                bearing_deg: 270,
            },
        ],
        objective: None,
    }
}

/// `deliberation.rs`'s `mock_service` shape: a tool-calling
/// `MockProvider` named `openrouter` (where route derivation sends every
/// non-`anthropic/` model) behind the `tacsim_deliberate` route.
fn probe_mock_service() -> (Arc<LlmService>, Arc<mush_llm::MockProvider>) {
    let mock = Arc::new(mush_llm::MockProvider::new("openrouter").with_caps(
        mush_llm::ProviderCaps {
            tool_calling: true,
            forced_tool_choice: true,
            ..mush_llm::ProviderCaps::default()
        },
    ));
    let provider: Arc<dyn mush_llm::LlmProvider> = mock.clone();
    let llm = Arc::new(
        LlmService::builder()
            .provider("openrouter", provider)
            .route(
                "tacsim_deliberate",
                mush_llm::RouteConfig {
                    model: "z-ai/glm-5.3-flash".to_string(),
                    max_tokens: 512,
                    timeout_s: 30,
                    reasoning: None,
                    reasoning_max_tokens: None,
                },
            )
            .global_enabled(true)
            .build(),
    );
    (llm, mock)
}

/// Spawn a world with the deliberation probe over `llm`, raise one tacsim
/// `Effect::ModuleAsync`, wait for its re-entry, shut the world down, and
/// return the re-entered reply.
async fn probe_deliberation_reply(
    llm: Option<Arc<LlmService>>,
) -> tacsim::deliberation::DeliberationReply {
    let reply = Arc::new(Mutex::new(None));
    let engine = Engine::with_modules(
        World::new_minimal(),
        base_registry(),
        vec![Box::new(DeliberationProbeModule {
            reply: Arc::clone(&reply),
        })],
    )
    .expect("the deliberation probe composes");
    let recorder = Arc::new(RecordingGateway::default());
    let handles = spawn_persist_with_gateway(
        engine,
        ServerDeps {
            llm,
            ..ServerDeps::in_memory()
        },
        move |_| -> Gateway { recorder },
    );
    assert!(
        handles
            .handle
            .inbound(Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "probe-deliberate".into(),
            })
            .await
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let payload = loop {
        if let Some(payload) = reply.lock().expect("the probe's reply slot").clone() {
            break payload;
        }
        assert!(
            Instant::now() < deadline,
            "the dispatcher must run the deliberation against `deps.llm` and re-enter \
             its reply (or re-enter the refusal when there is none)"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(
        handles
            .handle
            .inbound(Inbound::Control(ControlMsg::Shutdown))
            .await
    );
    handles.world_join.await.unwrap();
    handles.writer_join.await.unwrap();
    handles.outbox_join.await.unwrap();
    serde_json::from_str(&payload).expect("the re-entered payload is a DeliberationReply")
}

/// **The effect dispatcher runs a squad's deliberation against `deps.llm`,
/// and refuses it without one** (16b cut AC.1, 16c `B-82`).
///
/// The split half used to hand the world task no service, so on dev1 every
/// deliberation took the second branch below. With the service, the mock's
/// default empty answer never parses — `structured.rs` makes the original
/// attempt and one repair, then refuses by name — so the proof that the
/// call LEFT the world task is a `malformed` refusal after exactly two
/// calls (read after the re-entry, so the count is exact). Without it, the
/// dispatcher re-enters `unavailable_reply` and nothing is called. This is
/// the cheap pin; `mush-gateway`'s `two_process_llm.rs` is the expensive
/// form of it on the production shape.
#[tokio::test]
async fn a_tacsim_module_async_effect_runs_against_the_service_and_is_refused_without_it() {
    use tacsim::deliberation::{DeliberationReply, RefusedReason};

    let (llm, mock) = probe_mock_service();

    // The control first: the same world with no service is the B-82 shape.
    match probe_deliberation_reply(None).await {
        DeliberationReply::Refused { reason, .. } => assert_eq!(
            reason,
            RefusedReason::Unavailable.as_str(),
            "a world task with no service re-enters `unavailable_reply`"
        ),
        other => panic!("a world with no service cannot make an order: {other:?}"),
    }
    assert_eq!(
        mock.call_count(),
        0,
        "this world was never handed the service, so nothing was called; the \
         observed thing is the refusal"
    );

    // The same world over the service.
    match probe_deliberation_reply(Some(Arc::clone(&llm))).await {
        DeliberationReply::Refused { reason, .. } => assert_eq!(
            reason,
            RefusedReason::Malformed.as_str(),
            "the mock's empty answer never parses, so the service's refusal is `malformed`"
        ),
        other => panic!("the empty answer cannot make an order: {other:?}"),
    }
    assert_eq!(
        mock.call_count(),
        2,
        "the original attempt plus one repair: the deliberation reached the provider"
    );
}

struct ProbeInputContext(Arc<GateProbeCounts>);

impl InputContext for ProbeInputContext {
    fn prompt(&self, _world: &World) -> PromptSpec {
        PromptSpec {
            mode: InputMode::Normal,
            text: Some(RichText::plain("probe>")),
        }
    }

    fn feed(&mut self, ctx: &mut Ctx<'_>, _line: &str) -> ContextOutcome {
        record_probe_mutation(ctx, &self.0.input_context, "input_context");
        ContextOutcome::Consumed
    }
}

fn cmd(line: &str) -> Inbound {
    Inbound::Cmd {
        conn: 1,
        actor: GOD,
        line: line.to_string(),
    }
}

#[test]
fn inbound_classifier_covers_every_variant() {
    let cases = [
        (cmd("look"), IngressClass::Command),
        (
            Inbound::Query {
                conn: 1,
                actor: GOD,
                id: 7,
                name: "who".into(),
                args: serde_json::json!({}),
            },
            IngressClass::WorldRead,
        ),
        (
            Inbound::ConnUp {
                conn: 1,
                actor: GOD,
                caps_structured: false,
                from_linkdead: false,
            },
            IngressClass::ConnectionHooks,
        ),
        (
            Inbound::ConnDown {
                conn: 1,
                actor: GOD,
                reason: DisconnectKind::Lost,
            },
            IngressClass::ConnectionHooks,
        ),
        (
            Inbound::SessionGone { actor: GOD },
            IngressClass::ConnectionHooks,
        ),
        (
            Inbound::Apply(WorldCommand::SlowTestDone {
                actor: GOD,
                correlation: 9,
                text: "done".to_string(),
            }),
            IngressClass::AsyncApply,
        ),
        (
            Inbound::Control(ControlMsg::Shutdown),
            IngressClass::Shutdown,
        ),
        (
            Inbound::Control(ControlMsg::Dump),
            IngressClass::StaffMutation,
        ),
    ];
    for (msg, expected) in cases {
        assert_eq!(classify_inbound(&msg), expected);
    }
}

#[test]
fn world_message_classifier_covers_every_variant() {
    assert_eq!(
        classify_world_msg(&WorldMsg::Inbound(cmd("look"))),
        IngressClass::Command
    );
    assert_eq!(
        classify_world_msg(&WorldMsg::ReAttach {
            recs: Vec::new().into_boxed_slice(),
        }),
        IngressClass::TransportBookkeeping
    );
    let (reply, _rx) = oneshot::channel();
    assert_eq!(
        classify_world_msg(&WorldMsg::CreatePlayer {
            name: "Newbie".to_string(),
            account: None,
            reply,
        }),
        IngressClass::WorldMutation
    );
    let (reply, _rx) = oneshot::channel();
    assert_eq!(
        classify_world_msg(&WorldMsg::NameFree {
            name: "Newbie".to_string(),
            reply,
        }),
        IngressClass::WorldRead
    );
    let (reply, _rx) = oneshot::channel();
    assert_eq!(
        classify_world_msg(&WorldMsg::InInputContext { actor: GOD, reply }),
        IngressClass::WorldRead
    );
    // A relayed line speaks on a channel: a world mutation.
    let (reply, _rx) = oneshot::channel();
    assert_eq!(
        classify_world_msg(&WorldMsg::RelaySpeak {
            req: mush_ipc::RelaySpeak {
                message_id: None,
                dm_conn: None,
                user_id: 1,
                account: mush_core::AccountId::mint("nick", 1).unwrap(),
                actor: GOD,
                channel: "Public".to_string(),
                kind: mush_proto::NarrativeKind::Say,
                text: "hi".to_string(),
            },
            reply,
        }),
        IngressClass::WorldMutation
    );
    // A forwarded Discord interaction runs a module handler that may
    // mutate: a world mutation, never a read.
    let (reply, _rx) = oneshot::channel();
    assert_eq!(
        classify_world_msg(&WorldMsg::DiscordInteraction {
            interaction: mush_ipc::DiscordInteraction {
                user_id: 1,
                account: None,
                actor: None,
                custom_id: "oncall:ack:1".to_string(),
                values: vec![],
            },
            reply,
        }),
        IngressClass::WorldMutation
    );
    let (reply, _rx) = oneshot::channel();
    assert_eq!(
        classify_world_msg(&WorldMsg::ConnectionReady {
            conn: 1,
            actor: GOD,
            reply,
        }),
        IngressClass::WorldRead
    );
    let (reply, _rx) = oneshot::channel();
    assert_eq!(
        classify_world_msg(&WorldMsg::RecheckPrivilege {
            actor: GOD,
            privilege: Privileges::DISCORD_IDENTITY,
            subject: None,
            command: "@discord/show",
            reply,
        }),
        IngressClass::WorldMutation
    );
    let (reply, _rx) = oneshot::channel();
    assert_eq!(
        classify_world_msg(&WorldMsg::Barrier { reply }),
        IngressClass::OrderingBarrier
    );
}

/// The disclosure is refused whenever the world cannot say the privilege
/// is still held: revoked, busy, or gone. Nothing about a failed read may
/// be read as an authorization.
#[tokio::test]
async fn discord_show_refuses_unless_the_world_re_confirms_the_privilege() {
    let (tx, mut rx) = mpsc::channel::<WorldMsg>(4);
    let weak = tx.downgrade();
    let target = ObjId(7);

    // The world answers the re-check; the first says the grant is gone.
    let answers = tokio::spawn(async move {
        let mut sent = Vec::new();
        while let Some(msg) = rx.recv().await {
            let WorldMsg::RecheckPrivilege {
                actor,
                privilege,
                subject,
                command,
                reply,
            } = msg
            else {
                continue;
            };
            sent.push((actor, privilege, subject, command));
            let held = sent.len() > 2;
            let _ = reply.send(if sent.len() == 2 {
                Err(WorldReadError::Busy)
            } else {
                Ok(held)
            });
        }
        sent
    });

    // (1) Revoked in the window.
    let refusal = discord_show_recheck(&weak, GOD, target)
        .await
        .expect_err("a revoked privilege must refuse");
    assert!(refusal.contains("revoked"), "{refusal}");
    assert!(refusal.contains("Permission denied"), "{refusal}");
    // (2) The world is committing and cannot answer: refuse too.
    let refusal = discord_show_recheck(&weak, GOD, target)
        .await
        .expect_err("an unanswerable re-check must refuse");
    assert!(refusal.contains("could not be re-checked"), "{refusal}");
    // (3) Still held: the executor proceeds to the disclosure.
    discord_show_recheck(&weak, GOD, target)
        .await
        .expect("a held privilege discloses");

    drop(tx);
    let sent = answers.await.expect("answering task");
    assert_eq!(sent.len(), 3);
    for (actor, privilege, subject, command) in sent {
        assert_eq!(actor, GOD);
        assert_eq!(privilege, Privileges::DISCORD_IDENTITY);
        assert_eq!(subject, Some(target));
        assert_eq!(command, "@discord/show");
    }

    // (4) The world task is gone: no disclosure on a dead channel.
    let refusal = discord_show_recheck(&weak, GOD, target)
        .await
        .expect_err("a gone world must refuse");
    assert!(refusal.contains("shutting down"), "{refusal}");
}

#[test]
fn only_transferred_semantic_or_lifecycle_custody_uses_deferred_slot() {
    assert!(!must_defer_behind_commit(&WorldMsg::Inbound(cmd("look"))));
    let (reply, _rx) = oneshot::channel();
    assert!(!must_defer_behind_commit(&WorldMsg::CreatePlayer {
        name: "RetryMe".to_string(),
        account: None,
        reply,
    }));
    assert!(must_defer_behind_commit(&WorldMsg::Inbound(
        Inbound::ConnDown {
            conn: 9,
            actor: GOD,
            reason: DisconnectKind::Quit,
        }
    )));
    assert!(must_defer_behind_commit(&WorldMsg::Inbound(
        Inbound::Apply(WorldCommand::SlowTestDone {
            actor: GOD,
            correlation: 1,
            text: "done".to_string(),
        })
    )));
}

#[test]
fn writer_and_staff_classifiers_cover_every_variant() {
    assert_eq!(
        classify_writer_signal(&WriterSignal::StoreStatus {
            healthy: true,
            detail: String::new(),
        }),
        IngressClass::WriterRecovery
    );
    assert_eq!(
        classify_writer_signal(&WriterSignal::LedgerBreach {
            detail: "breach".to_string(),
            seal: None,
        }),
        IngressClass::WriterRecovery
    );
    assert_eq!(
        classify_writer_signal(&WriterSignal::DeterministicFailure {
            detail: "rejected".to_string(),
        }),
        IngressClass::WriterRecovery
    );

    let ip = "127.0.0.1".parse().unwrap();
    let cases = [
        (
            StaffOpKind::SetBan {
                ip,
                until_unix: None,
                reason: "test".to_string(),
            },
            IngressClass::StaffMutation,
        ),
        (StaffOpKind::ClearBan { ip }, IngressClass::StaffMutation),
        (
            StaffOpKind::LiftTempban {
                ip,
                issued_unix_ms: 1_000,
            },
            IngressClass::StaffMutation,
        ),
        (StaffOpKind::ListBans, IngressClass::StaffStatus),
        (StaffOpKind::Dump, IngressClass::StaffMutation),
        (StaffOpKind::Shutdown, IngressClass::Shutdown),
        (
            StaffOpKind::Boot {
                target: GOD,
                message: String::new(),
            },
            IngressClass::StaffMutation,
        ),
        (
            StaffOpKind::NewPassword {
                target: GOD,
                name: "God".to_string(),
                by: "God".to_string(),
                password: crate::staff::Secret("secret".to_string()),
            },
            IngressClass::StaffMutation,
        ),
        (
            StaffOpKind::AddSshKey {
                actor: GOD,
                openssh: "ssh-ed25519 AAAA".to_string(),
                fingerprint: "SHA256:test".to_string(),
            },
            IngressClass::StaffMutation,
        ),
        (
            StaffOpKind::ListSshKeys { actor: GOD },
            IngressClass::StaffStatus,
        ),
        (
            StaffOpKind::DelSshKey {
                actor: GOD,
                which: "1".to_string(),
            },
            IngressClass::StaffMutation,
        ),
        (
            StaffOpKind::NewChar {
                actor: GOD,
                name: "Alt".to_string(),
                max_characters: 2,
            },
            IngressClass::StaffMutation,
        ),
        (StaffOpKind::Ps, IngressClass::StaffStatus),
        (StaffOpKind::Stats, IngressClass::StaffStatus),
    ];
    for (op, expected) in cases {
        assert_eq!(classify_staff_op(&op), expected);
    }
}

#[test]
fn writer_mailbox_coalesces_without_losing_mandatory_facts() {
    let (tx, mut rx) = writer_signal_channel();
    tx.store_status(false, "first failure".to_string());
    tx.store_status(true, String::new());
    tx.deterministic_failure("deterministic root cause".to_string());
    assert!(tx.ledger_breach("ledger freeze".to_string(), None));
    assert!(tx.ledger_breach(
        "exact resolution".to_string(),
        Some((
            SmolStr::new("econ"),
            SmolStr::new("log:cash:0"),
            "a".repeat(64),
            true,
        )),
    ));

    let signals: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert!(signals.iter().any(|signal| matches!(
        signal,
        WriterSignal::StoreStatus {
            healthy: true,
            detail
        } if detail.is_empty()
    )));
    assert!(signals.iter().any(|signal| matches!(
        signal,
        WriterSignal::DeterministicFailure { detail }
            if detail == "deterministic root cause"
    )));
    assert!(
        signals
            .iter()
            .any(|signal| matches!(signal, WriterSignal::LedgerBreach { seal: None, .. }))
    );
    assert!(signals.iter().any(|signal| matches!(
        signal,
        WriterSignal::LedgerBreach {
            seal: Some((ns, key, hash, true)),
            ..
        } if ns == "econ" && key == "log:cash:0" && hash == &"a".repeat(64)
    )));
    assert_eq!(signals.len(), 4);
}

#[test]
fn writer_mailbox_refuses_exact_overflow_and_accepts_retry_after_drain() {
    let (tx, mut rx) = writer_signal_channel();
    for index in 0..crate::retained::MAX_RETAINED_SEALS {
        assert!(tx.ledger_breach(
            "resolved".to_string(),
            Some((
                SmolStr::new("econ"),
                SmolStr::new(format!("log:cash:{index}")),
                format!("{index:064x}"),
                false,
            )),
        ));
    }
    let overflow = (
        SmolStr::new("econ"),
        SmolStr::new("log:cash:overflow"),
        "f".repeat(64),
        true,
    );
    assert!(
        !tx.ledger_breach("overflow".to_string(), Some(overflow.clone())),
        "an exact identity must be refused, never shed, when custody is full"
    );

    let first_drain: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert_eq!(
        first_drain
            .iter()
            .filter(|signal| matches!(signal, WriterSignal::LedgerBreach { seal: Some(_), .. }))
            .count(),
        crate::retained::MAX_RETAINED_SEALS
    );
    assert!(first_drain.iter().any(|signal| matches!(
        signal,
        WriterSignal::LedgerBreach { seal: None, detail }
            if detail.contains("mailbox saturated")
    )));

    assert!(tx.ledger_breach("overflow retry".to_string(), Some(overflow.clone())));
    let retried = rx.try_recv().expect("retry must arm a fresh wake");
    assert!(matches!(
        retried,
        WriterSignal::LedgerBreach {
            seal: Some((ns, key, hash, true)),
            ..
        } if ns == overflow.0 && key == overflow.1 && hash == overflow.2
    ));
    assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
}

#[test]
fn hard_modes_admit_only_safe_reads_ordering_recovery_and_shutdown() {
    const ALL_CLASSES: [IngressClass; 13] = [
        IngressClass::WorldRead,
        IngressClass::OrderingBarrier,
        IngressClass::Command,
        IngressClass::TransportBookkeeping,
        IngressClass::ConnectionHooks,
        IngressClass::WorldMutation,
        IngressClass::AsyncApply,
        IngressClass::Timer,
        IngressClass::WriterRecovery,
        IngressClass::StaffStatus,
        IngressClass::StaffMutation,
        IngressClass::ExternalEffect,
        IngressClass::Shutdown,
    ];
    let modes = [
        WorldMode::Booting,
        WorldMode::Recovering { incident: None },
        WorldMode::CommitPending {
            token: CommitToken::from_serial(7),
        },
        WorldMode::IntegrityFrozen {
            incident: IncidentId(11),
        },
    ];
    for mode in modes {
        for class in ALL_CLASSES {
            let should_allow = matches!(
                class,
                IngressClass::OrderingBarrier
                    | IngressClass::TransportBookkeeping
                    | IngressClass::WriterRecovery
                    | IngressClass::StaffStatus
                    | IngressClass::Shutdown
            ) || (class == IngressClass::WorldRead
                && !matches!(mode, WorldMode::CommitPending { .. }));
            assert_eq!(
                admission_for(mode, class) == Admission::Allow,
                should_allow,
                "mode={mode:?}, class={class:?}"
            );
        }
    }
}

/// **The account map's two halves, through the real world task** (16b cut
/// F.4).
///
/// The insert rides on `CreatePlayer` and must be visible before the create
/// reply lands — that is the whole of the K6 fix. The undo is
/// `UnlinkPlayer`, which the caller sends when the account document the
/// insert was made against could not be written: without it the running
/// server would hold a mapping that no boot could ever re-derive, and the
/// two halves would disagree in the permissive direction.
#[tokio::test]
async fn f4_the_account_map_learns_on_create_and_forgets_on_unlink() {
    use mush_core::AccountIdentity;

    let map = Arc::new(crate::accounts_identity::AccountMap::default());
    let mut engine = Engine::new(World::new_minimal(), base_registry());
    engine.set_account_identity(Arc::clone(&map) as Arc<dyn AccountIdentity>);
    let handles = spawn_persist_with_gateway(engine, ServerDeps::in_memory(), |_| {
        Arc::new(BlackholeGateway) as Gateway
    });

    let account = mush_core::AccountId::mint("Nick", 1).expect("a well-formed account id");
    let nicky = handles
        .handle
        .create_player("Nicky", Some(account))
        .await
        .expect("the world created the character");
    assert_eq!(
        map.account_of(nicky),
        Some(account),
        "the create turn must have told the oracle before its reply left"
    );

    // The compensating undo, and its idempotence.
    assert_eq!(handles.handle.unlink_player(nicky).await, Ok(true));
    assert_eq!(
        map.account_of(nicky),
        None,
        "an orphan on disk is an orphan in RAM"
    );
    assert_eq!(
        handles.handle.unlink_player(nicky).await,
        Ok(false),
        "forgetting twice removes nothing"
    );

    // A create with no resolvable account id maps nothing and still works.
    let ghost = handles
        .handle
        .create_player("Ghost", None)
        .await
        .expect("the world created the character");
    assert_eq!(map.account_of(ghost), None);

    assert!(
        handles
            .handle
            .inbound(Inbound::Control(ControlMsg::Shutdown))
            .await
    );
    handles.world_join.await.unwrap();
    handles.writer_join.await.unwrap();
    handles.outbox_join.await.unwrap();
}

#[test]
fn commit_pending_world_reads_are_typed_refusals_but_ordering_stays_live() {
    let pending = WorldMode::CommitPending {
        token: CommitToken::from_serial(17),
    };
    assert_eq!(
        admission_for(pending, IngressClass::WorldRead),
        Admission::Refuse(RefusalReason::CommitPending)
    );
    for class in [
        IngressClass::OrderingBarrier,
        IngressClass::TransportBookkeeping,
        IngressClass::WriterRecovery,
        IngressClass::StaffStatus,
        IngressClass::Shutdown,
    ] {
        assert_eq!(admission_for(pending, class), Admission::Allow);
    }
}

async fn assert_live_hard_mode_gate(mode: WorldMode) {
    let counts = Arc::new(GateProbeCounts::default());
    let mut world = World::new_minimal();
    let input_actor = world
        .create(ObjKind::Player, "InputProbe", GOD, Some(LIMBO))
        .expect("input probe player");
    world.push_input_context(
        input_actor,
        Some("gate_probe".into()),
        Box::new(ProbeInputContext(Arc::clone(&counts))),
    );
    // Make the registered 1 ms tick overdue, then clear setup dirtiness so
    // any store write below can only have come from a forbidden seam.
    world.set_now(unix_now_s().saturating_sub(1));
    let _ = world.take_dirty();
    let engine = Engine::with_modules(
        world,
        base_registry(),
        vec![Box::new(GateProbeModule {
            counts: Arc::clone(&counts),
            input_actor,
        })],
    )
    .expect("gate probe module composes");

    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let staff = Arc::new(StaffQueue::new());
    staff.push(StaffOp {
        actor: GOD,
        kind: StaffOpKind::SetBan {
            ip: "127.0.0.77".parse().expect("test IP"),
            until_unix: None,
            reason: "must remain blocked".to_string(),
        },
    });
    let recorder = Arc::new(RecordingGateway::default());
    let sink = Arc::clone(&recorder);
    let handles = spawn_persist_with_gateway_mode(
        engine,
        ServerDeps {
            store: Arc::clone(&store),
            guard: Arc::new(IpGuard::new()),
            staff,
            llm: None,
            metrics: Arc::new(Metrics::new()),
            quarantine_spool: crate::quarantine::QuarantineSpool::in_memory(),
            outbox_adapters: Vec::new(),
        },
        Arc::new(DurableOutboxRegistry::new()),
        None,
        Some(mode),
        DEFAULT_WORLD_CLOCK_CHECKPOINT_S,
        move |_| -> Gateway { sink },
    );
    let PersistHandles {
        handle,
        world_join,
        writer_join,
        ..
    } = handles;

    // Ordinary command handler + observer + deferred effect.
    assert!(
        handle
            .inbound(Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "gate-mutate".to_string(),
            })
            .await
    );
    // The same structural command class while an input context owns the
    // line: admission must happen before the context's `feed` callback.
    assert!(
        handle
            .inbound(Inbound::Cmd {
                conn: 2,
                actor: input_actor,
                line: "captured payload".to_string(),
            })
            .await
    );
    // Connection lifecycle hooks, typed world mutation, async completion,
    // and staff control all enter through distinct production seams.
    assert!(
        handle
            .inbound(Inbound::ConnUp {
                conn: 3,
                actor: input_actor,
                caps_structured: false,
                from_linkdead: false,
            })
            .await
    );
    assert_eq!(
        handle.create_player("BlockedPlayer", None).await,
        Err(CreatePlayerErr::StoreUnavailable),
        "CreatePlayer must be refused in {mode:?}"
    );
    assert!(
        handle
            .inbound(Inbound::Apply(WorldCommand::ModuleAsyncDone {
                module: "gate_probe".into(),
                actor: GOD,
                correlation: 11,
                payload: "{}".to_string(),
            }))
            .await
    );
    assert!(handle.inbound(Inbound::Control(ControlMsg::Dump)).await);

    // Barriers are order bookkeeping, not gameplay reads. Two round trips
    // ensure the first iteration's staff-queue drain has also completed.
    assert!(
        handle.barrier().await,
        "first barrier remains live in {mode:?}"
    );
    assert!(
        handle.barrier().await,
        "second barrier remains live in {mode:?}"
    );
    tokio::time::sleep(Duration::from_millis(20)).await;

    assert_eq!(
        counts.total(),
        0,
        "a forbidden engine/module callback executed in {mode:?}"
    );
    assert_eq!(
        handle.name_free("BlockedPlayer").await,
        Ok(true),
        "refused creation must not change the player index in {mode:?}"
    );
    assert_eq!(
        handle.in_input_context(input_actor).await,
        Ok(true),
        "refused input must not consume the context in {mode:?}"
    );

    let query_id = 700 + u64::from(input_actor.0);
    assert!(
        handle
            .inbound(Inbound::Query {
                conn: 9,
                actor: GOD,
                id: query_id,
                name: "gate_probe.snapshot".into(),
                args: serde_json::json!({}),
            })
            .await
    );
    assert!(handle.barrier().await, "query delivery barrier in {mode:?}");
    assert_eq!(
        recorder.reply_text(query_id).as_deref(),
        Some("objects=3 mutations=0 input_depth=1"),
        "world and module snapshots changed in {mode:?}"
    );
    assert!(
        store.load_world().await.expect("read test world").is_none(),
        "forbidden world state reached the store in {mode:?}"
    );
    assert!(
        store
            .module_json_get("gate_probe", "command")
            .await
            .expect("read probe module doc")
            .is_none(),
        "forbidden module state reached the store in {mode:?}"
    );
    assert!(
        store.get_bans().await.expect("read test bans").is_empty(),
        "forbidden staff mutation reached the store in {mode:?}"
    );

    // Exercise the final external-effect launch gate independently of the
    // command/tick producers above. If it launches, the zero-delay test
    // effect re-enters this live channel as an Apply message.
    let (effect_tx, mut effect_rx) = mpsc::channel(1);
    let effect_weak = effect_tx.downgrade();
    release_classified_effects(
        mode,
        vec![(
            DeliveryClass::BestEffort,
            Effect::SlowTest {
                actor: GOD,
                correlation: 123,
                seconds: 0,
            },
        )],
        DeliveryOutcome::NoWrite,
        &effect_weak,
        None,
        &store,
    );
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(
        matches!(effect_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
        "an external effect launched in {mode:?}"
    );

    // Transport-only reattach and shutdown remain live. An empty roster is
    // enough to prove the production message is admitted without adding
    // intentional connection bookkeeping to the state-equality checks.
    assert!(handle.reattach(Vec::new()).await);
    assert!(handle.barrier().await, "reattach ordering in {mode:?}");
    assert!(handle.inbound(Inbound::Control(ControlMsg::Shutdown)).await);
    tokio::time::timeout(Duration::from_secs(2), world_join)
        .await
        .expect("hard-mode world shutdown timed out")
        .expect("hard-mode world task panicked");
    tokio::time::timeout(Duration::from_secs(2), writer_join)
        .await
        .expect("hard-mode writer shutdown timed out")
        .expect("hard-mode writer task panicked");
}

#[tokio::test]
async fn live_hard_modes_block_every_mutating_execution_seam() {
    for mode in [
        WorldMode::Booting,
        WorldMode::Recovering {
            incident: Some(IncidentId(41)),
        },
        WorldMode::IntegrityFrozen {
            incident: IncidentId(42),
        },
    ] {
        assert_live_hard_mode_gate(mode).await;
    }
}

fn pending_for_custody_test() -> PendingCommit {
    let (_receipt_tx, receipt) = oneshot::channel();
    PendingCommit {
        token: CommitToken::from_serial(7),
        receipt: PendingReceipt::Legacy {
            expected_serial: 7,
            receiver: receipt,
        },
        sends: vec![(
            DeliveryClass::OnCommit,
            SendTarget::Char(u64::from(GOD.0)),
            Arc::new(OutputEvent::Sys(RichText::plain("held output"))),
        )],
        effects: vec![(
            DeliveryClass::OnCommit,
            Effect::SlowTest {
                actor: GOD,
                correlation: 9,
                seconds: 0,
            },
        )],
        failure_actor: Some(GOD),
        create: None,
        action: None,
        prepared_installation: None,
        barriers: Vec::new(),
        coalesced_reattach: None,
        exit_after_ack: false,
    }
}

#[tokio::test]
async fn prepared_action_installs_only_after_its_exact_receipt() {
    let (mut engine, handle) = prepared_lane_fixture();
    let mut transaction = engine
        .action_transaction(&handle)
        .expect("test transaction begins");
    transaction
        .insert(&handle, &41, PreparedLaneState { amount: 7 })
        .expect("test state is valid");
    let prepared = transaction.prepare().expect("test transaction prepares");
    let bundle = prepared_lane_bundle(&prepared, 7);
    let expected_token = bundle.token();
    let guards = empty_guards(&engine);
    engine
        .stage_authoritative_action(prepared, guards, bundle)
        .expect("clean engine accepts prepared action");

    let (writer, mut writer_rx) = persist::test_store_writer_channel(Arc::new(Metrics::new()));
    let mut health = StoreHealth::new();
    let metrics = Metrics::new();
    let mut retained = RetainedSeals::new();
    let high_water = AtomicU64::new(0);
    let outcome = flush_prepared_action(
        &mut engine,
        &writer,
        &mut health,
        &metrics,
        &mut retained,
        &high_water,
    )
    .await;
    let FlushOutcome::Queued {
        receipt: Some(receipt),
        prepared_installation: Some(installation),
    } = outcome
    else {
        panic!("prepared action must queue with its receipt and install image");
    };
    let queued =
        settle_queued_authoritative_job(writer_rx.recv().await.expect("prepared write was queued"))
            .await;
    assert_eq!(queued.token(), expected_token);
    assert_eq!(queued.writes().len(), 1);

    assert_eq!(engine.action_state().revision(), 0);
    assert!(engine.action_state().has_prepared());
    assert!(
        engine
            .action_state()
            .get(&handle, &41)
            .expect("state lookup succeeds")
            .is_none(),
        "queue acceptance must not expose the typed post-image"
    );

    let mut receipt = receipt;
    let ack = receipt.receive().await;
    let mut pending = pending_with_prepared_installation(0, installation);
    pending.token = CommitToken::from_authoritative(expected_token);
    let gateway: Gateway = Arc::new(BlackholeGateway);
    let persistence_integrity = PersistenceIntegrityLatch::new();
    let (tx, _rx) = mpsc::channel(1);
    let completed = complete_pending_commit(
        pending,
        ack,
        &mut engine,
        &gateway,
        &mut health,
        &persistence_integrity,
        &tx.downgrade(),
        None,
        &test_effect_store(),
    );

    assert_eq!(completed.outcome, DeliveryOutcome::Committed);
    assert_eq!(engine.action_state().revision(), 1);
    assert!(!engine.action_state().has_prepared());
    assert_eq!(
        engine
            .action_state()
            .get(&handle, &41)
            .expect("state lookup succeeds"),
        Some(&PreparedLaneState { amount: 7 })
    );
    assert!(!persistence_integrity.is_latched());
}
/// Mint one prepared installation against a *different* engine's action
/// store. Applying it anywhere else is a pure wiring bug: nothing a player
/// can type produces one.
fn foreign_prepared_installation() -> PreparedInstallation {
    let (mut other, other_handle) = prepared_lane_fixture();
    let mut transaction = other
        .action_transaction(&other_handle)
        .expect("test transaction begins");
    transaction
        .insert(&other_handle, &41, PreparedLaneState { amount: 7 })
        .expect("test state is valid");
    transaction
        .prepare()
        .expect("test transaction prepares")
        .into_parts()
        .1
        .into()
}

/// Whether staff were told, in the words `apply_integrity_freeze` uses.
fn wizard_heard(recorder: &RecordingGateway, needle: &str) -> bool {
    recorder
        .sends
        .lock()
        .expect("recording gateway poisoned")
        .iter()
        .any(|(_, event)| match event.as_ref() {
            OutputEvent::Sys(text) => text.to_plain_text().contains(needle),
            _ => false,
        })
}

/// core-action-7: the commit seam used to `assert!` when a prepared
/// installation did not belong to the engine's own store — on the one task
/// that owns the world and may never panic (docs/PLAN.md). Now the seam
/// fails closed: persistence freezes, staff are told, nothing is announced
/// to observers, no typed image goes live, and this call RETURNS. The
/// liveness assertion is the point — the failure mode is "a wiring bug
/// takes down the single writer".
#[test]
fn a_foreign_prepared_installation_freezes_the_commit_seam_and_the_writer_lives() {
    let (mut engine, handle) = prepared_lane_fixture();
    engine.world.set_connected(GOD, true);
    let pending = pending_with_prepared_installation(9, foreign_prepared_installation());
    let recorder = Arc::new(RecordingGateway::default());
    let gateway: Gateway = recorder.clone();
    let mut health = StoreHealth::new();
    let persistence_integrity = PersistenceIntegrityLatch::new();
    let (tx, _rx) = mpsc::channel(1);

    let completed = complete_pending_commit(
        pending,
        Ok(()),
        &mut engine,
        &gateway,
        &mut health,
        &persistence_integrity,
        &tx.downgrade(),
        None,
        &test_effect_store(),
    );

    // Reaching this line at all is the proof the single writer survived.
    assert_eq!(completed.outcome, DeliveryOutcome::Committed);
    assert!(health.mode.integrity_frozen());
    assert!(persistence_integrity.is_latched());
    assert!(wizard_heard(&recorder, "persistence integrity breach"));
    assert_eq!(
        engine.action_state().revision(),
        0,
        "a refused install must leave live action state untouched"
    );
    assert!(
        engine
            .action_state()
            .get(&handle, &41)
            .expect("state lookup succeeds")
            .is_none(),
        "no typed post-image may go live from a refused installation"
    );
    assert!(
        !wizard_heard(&recorder, "announced "),
        "an install that did not happen must not be announced to observers \
         (the fixture's I13 observer says `announced <owner>/<family>`)"
    );
}

/// The refusal half of the same seam: an abort that does not match the
/// store freezes rather than asserting, and the loop still returns.
#[test]
fn a_foreign_installation_on_the_abort_path_freezes_and_the_writer_lives() {
    let (mut engine, _handle) = prepared_lane_fixture();
    engine.world.set_connected(GOD, true);
    let pending = pending_with_prepared_installation(9, foreign_prepared_installation());
    let recorder = Arc::new(RecordingGateway::default());
    let gateway: Gateway = recorder.clone();
    let mut health = StoreHealth::new();
    let persistence_integrity = PersistenceIntegrityLatch::new();
    let (tx, _rx) = mpsc::channel(1);

    let completed = complete_pending_commit(
        pending,
        Err(AuthoritativeCommitFailure::Refused(
            "store refused the batch".to_owned(),
        )),
        &mut engine,
        &gateway,
        &mut health,
        &persistence_integrity,
        &tx.downgrade(),
        None,
        &test_effect_store(),
    );

    assert_eq!(completed.outcome, DeliveryOutcome::Aborted);
    assert!(health.mode.integrity_frozen());
    assert!(persistence_integrity.is_latched());
    assert!(wizard_heard(&recorder, "persistence integrity breach"));
    assert_eq!(engine.action_state().revision(), 0);
}

/// core-action-8, the third ending: nobody answered for the installation
/// at all. `into_parts` had already handed the module ops to the writer,
/// so the store may hold a durable write whose typed RAM post-images went
/// out with the dropped value. RAM and Redis have diverged and this task
/// cannot reconcile them, so the loop's supervision check quarantines the
/// world instead of admitting the next line and answering it out of RAM.
#[test]
fn an_installation_loss_quarantines_rather_than_answering_the_player() {
    let (mut engine, handle) = prepared_lane_fixture();
    engine.world.set_connected(GOD, true);
    let recorder = Arc::new(RecordingGateway::default());
    let gateway: Gateway = recorder.clone();
    let mut health = StoreHealth::new();
    let persistence_integrity = PersistenceIntegrityLatch::new();
    let mut seen = engine.action_state().lost_installations();

    // A healthy store passes the check untouched — it is on the head of
    // every loop iteration, so it may not freeze anything by itself.
    assert!(!freeze_on_lost_installation(
        &engine,
        &gateway,
        &mut health,
        &persistence_integrity,
        &mut seen,
    ));
    assert!(!health.mode.integrity_frozen());
    assert!(!persistence_integrity.is_latched());

    // Stage the durable half, then lose the installation the way a dropped
    // `PendingCommit` does.
    let mut transaction = engine
        .action_transaction(&handle)
        .expect("test transaction begins");
    transaction
        .insert(&handle, &41, PreparedLaneState { amount: 7 })
        .expect("test state is valid");
    let (module_ops, installation) = transaction
        .prepare()
        .expect("test transaction prepares")
        .into_parts();
    assert_eq!(module_ops.len(), 1);
    drop(installation);

    assert!(freeze_on_lost_installation(
        &engine,
        &gateway,
        &mut health,
        &persistence_integrity,
        &mut seen,
    ));
    assert!(health.mode.integrity_frozen());
    assert!(persistence_integrity.is_latched());
    assert!(wizard_heard(&recorder, "persistence integrity breach"));
    // No success disposition: the post-image never went live, and the next
    // line cannot even open a transaction to make one.
    assert_eq!(engine.action_state().revision(), 0);
    assert!(
        engine
            .action_state()
            .get(&handle, &41)
            .expect("state lookup succeeds")
            .is_none()
    );
    assert!(matches!(
        engine.action_transaction(&handle).err(),
        Some(mush_core::ActionStateError::InstallationLost { reservation: 1 })
    ));
    // Frozen means frozen: the check does not re-alert on a loss it has
    // already answered for, and the freeze does not auto-lift.
    assert!(!freeze_on_lost_installation(
        &engine,
        &gateway,
        &mut health,
        &persistence_integrity,
        &mut seen,
    ));
    assert!(health.mode.integrity_frozen());
}

// ---- WS01-S5: the world loop's impossible states fail closed ----
//
// Every proof below drives the converted site directly, because that is
// the only way to reach it: each was an `unreachable!` or an `expect`
// precisely because no ordinary input produces the state. The failure mode
// they all guard is the same one — the task that owns the world unwinds
// while the store writer keeps its queue and the players keep their
// connections — so "the call returned" is a real assertion.

/// A commit receipt with no pending commit to release it against. Both
/// halves of the receipt (ownership and final ack) used to `expect` here,
/// on the single writer, over a store writer that would have gone on
/// serving its queue. Now the world freezes, staff are told, and the loop
/// is still there to be shut down.
#[test]
fn an_orphan_commit_receipt_freezes_the_world_instead_of_ending_the_writer() {
    for which in ["ownership", "commit"] {
        let (mut engine, _handle) = prepared_lane_fixture();
        engine.world.set_connected(GOD, true);
        let recorder = Arc::new(RecordingGateway::default());
        let gateway: Gateway = recorder.clone();
        let mut health = StoreHealth::new();
        let persistence_integrity = PersistenceIntegrityLatch::new();

        freeze_orphan_commit_receipt(
            &engine.world,
            &gateway,
            &mut health,
            &persistence_integrity,
            which,
        );

        // Reaching this line at all is the liveness half of the proof.
        assert!(health.mode.integrity_frozen(), "{which}: the world froze");
        assert!(persistence_integrity.is_latched(), "{which}: and latched");
        assert!(
            wizard_heard(&recorder, "persistence integrity breach"),
            "{which}: staff were told"
        );
        assert!(
            wizard_heard(
                &recorder,
                &format!("{which} receipt resolved with no pending commit"),
            ),
            "{which}: and told which receipt was orphaned"
        );
        // The freeze is a mutation barrier, not a stopped loop: shutdown is
        // still admitted, which is what makes the freeze observable and the
        // exit path reachable.
        assert_eq!(
            admission_for(health.mode, IngressClass::Command),
            Admission::Refuse(RefusalReason::IntegrityFrozen),
            "{which}: mutations are paused"
        );
        assert_eq!(
            admission_for(health.mode, IngressClass::Shutdown),
            Admission::Allow,
            "{which}: the loop is still there to shut down"
        );
    }
}

/// `refuse_world_msg` used to `unreachable!` on the two messages that are
/// admitted in every world mode. If one is ever routed to it, refusing a
/// shutdown only costs the operator a second attempt; ending the loop from
/// inside the refusal path costs the game.
#[test]
fn refuse_world_msg_drops_messages_it_should_never_have_been_given() {
    let recorder = Arc::new(RecordingGateway::default());
    let gateway: Gateway = recorder.clone();

    refuse_world_msg(
        WorldMsg::Inbound(Inbound::Control(ControlMsg::Shutdown)),
        RefusalReason::CommitPending,
        &gateway,
        None,
    );
    refuse_world_msg(
        WorldMsg::ReAttach {
            recs: Vec::new().into_boxed_slice(),
        },
        RefusalReason::CommitPending,
        &gateway,
        None,
    );

    // Both returned, and neither invented output for a nonexistent actor.
    assert!(
        recorder
            .sends
            .lock()
            .expect("recording gateway poisoned")
            .is_empty(),
        "a dropped control message speaks to nobody"
    );
}

/// The store-read-only admission is resolved by message body, and only
/// `Inbound::Cmd` classifies as a command. A message that reaches the
/// resolver anyway means the classifier drifted: it must refuse, not kill
/// the loop that asked.
#[test]
fn a_misclassified_store_read_only_admission_refuses_instead_of_asserting() {
    let world = World::new_minimal();

    assert_eq!(
        resolve_store_read_only_command(
            &world,
            &WorldMsg::ReAttach {
                recs: Vec::new().into_boxed_slice(),
            },
        ),
        Admission::Refuse(RefusalReason::StoreUnavailable),
        "a non-inbound message fails closed"
    );

    // The controls: the resolver still answers real command lines the way
    // the read-only allowlist says.
    assert_eq!(
        resolve_store_read_only_command(
            &world,
            &WorldMsg::Inbound(Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@dig somewhere".to_string(),
            }),
        ),
        Admission::Refuse(RefusalReason::StoreUnavailable),
        "a mutating line is still refused while the store is read-only"
    );
    assert_eq!(
        resolve_store_read_only_command(
            &world,
            &WorldMsg::Inbound(Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "\"hello".to_string(),
            }),
        ),
        Admission::Allow,
        "speech still passes"
    );
}

/// The prepared lane is selected only when a staged action is in hand.
/// If the selection and the engine disagree the flush refuses: nothing is
/// persisted, nothing is released, and the caller is told so.
#[tokio::test]
async fn a_prepared_flush_with_nothing_staged_refuses_instead_of_asserting() {
    let (mut engine, _handle) = prepared_lane_fixture();
    assert!(
        !engine.has_staged_action(),
        "the bypass is calling the prepared lane with nothing staged"
    );

    let (writer, mut writer_rx) = persist::test_store_writer_channel(Arc::new(Metrics::new()));
    let mut health = StoreHealth::new();
    let metrics = Metrics::new();
    let mut retained = RetainedSeals::new();
    let high_water = AtomicU64::new(0);
    // The lane takes no serial to consume: a K3 commit is named by its own
    // token, and `flush_engine_step` keeps the legacy counter to itself.
    let outcome = flush_prepared_action(
        &mut engine,
        &writer,
        &mut health,
        &metrics,
        &mut retained,
        &high_water,
    )
    .await;

    match outcome {
        FlushOutcome::Refused(detail) => assert!(
            detail.contains("no staged action"),
            "unexpected refusal: {detail}"
        ),
        _ => panic!("an empty prepared flush must refuse"),
    }
    assert!(
        matches!(writer_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
        "and hands the writer nothing"
    );
    assert!(health.last_write_unix.is_none());
}

/// The writer-gone arm of `flush_dirty` carries the `let ... else` that
/// used to be an `unreachable!`. A closed writer queue is the reachable
/// half of that path: it must report the batch as dropped rather than
/// letting an ack-gated caller confirm a write that never happened.
#[tokio::test]
async fn flush_dirty_reports_writer_gone_when_the_queue_is_closed() {
    let (mut engine, _handle) = prepared_lane_fixture();
    engine
        .world
        .create(ObjKind::Thing, "orphan orb", GOD, Some(LIMBO))
        .expect("test object creation succeeds");
    assert!(engine.has_legacy_persistence_work());

    let (writer, writer_rx) = persist::test_store_writer_channel(Arc::new(Metrics::new()));
    drop(writer_rx);
    let mut serial = 0;
    let mut health = StoreHealth::new();
    let metrics = Metrics::new();
    let mut retained = RetainedSeals::new();
    let high_water = AtomicU64::new(0);
    let persistence_integrity = PersistenceIntegrityLatch::new();
    let outcome = flush_dirty(
        &mut engine,
        &writer,
        &mut serial,
        true,
        &mut health,
        &metrics,
        &mut retained,
        &high_water,
        &crate::quarantine::QuarantineSpool::in_memory(),
        &persistence_integrity,
    )
    .await;

    assert!(
        matches!(outcome, FlushOutcome::WriterGone),
        "a closed writer queue is a dropped batch, never a durable one"
    );
    assert!(
        health.last_write_unix.is_none(),
        "and nothing may be recorded as written"
    );
}

/// Commands/maintenance must bind pending acknowledgements without an
/// ArenaEgress wake; this also covers a queued run resumed after rollover.
#[tokio::test]
async fn generic_batches_stamp_acknowledgements_without_live_arena_traffic() {
    struct AckModule(tacsim::arena_bridge::ArenaTasks);
    impl GameModule for AckModule {
        fn name(&self) -> &'static str {
            "batch_ack_test"
        }
        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), mush_core::ModError> {
            let frames = reg.panel_feed("batch_ack.frames")?;
            let observers = reg.panel_feed("batch_ack.observers")?;
            reg.state(tacsim::arena_bridge::ArenaRegistry::new(
                self.0.clone(),
                frames,
                observers,
            ));
            reg.off_loop_simulation_host(Arc::new(self.0.clone()))?;
            Ok(())
        }
    }
    let tasks = tacsim::arena_bridge::ArenaTasks::new(2);
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(AckModule(tasks.clone()))],
    )
    .unwrap();
    engine.world.take_dirty();
    let (writer, mut receiver) = persist::test_store_writer_channel(Arc::new(Metrics::new()));
    let mut serial = 40;
    let mut health = StoreHealth::new();
    let metrics = Metrics::new();
    let mut retained = RetainedSeals::new();
    let high_water = AtomicU64::new(40);
    let integrity = PersistenceIntegrityLatch::new();
    let spool = crate::quarantine::QuarantineSpool::in_memory();
    assert!(tasks.live_ids().is_empty());
    for expected in [41, 42] {
        // The same unstamped table owns live-ledger and private-bulk latches.
        // Simulate a new command/tick write; no egress message is staged.
        tasks.note_ledger_write(tacsim::arena_task::MatchId(1), expected);
        tasks.ack_ledger_upto(expected - 1);
        assert_eq!(tasks.pending_ledger_acks(), 1);
        engine
            .world
            .create(ObjKind::Thing, "quota write", GOD, Some(LIMBO))
            .unwrap();
        let outcome = flush_dirty(
            &mut engine,
            &writer,
            &mut serial,
            true,
            &mut health,
            &metrics,
            &mut retained,
            &high_water,
            &spool,
            &integrity,
        )
        .await;
        assert!(matches!(outcome, FlushOutcome::Queued { .. }));
        let StoreJob::Write { batch, .. } = receiver.try_recv().unwrap() else {
            panic!("write expected");
        };
        assert_eq!(batch.serial, expected);
        tasks.ack_ledger_upto(expected - 1);
        assert_eq!(tasks.pending_ledger_acks(), 1, "queueing is not commitment");
        high_water.store(expected, Ordering::Relaxed);
        tasks.ack_ledger_upto(high_water.load(Ordering::Relaxed));
        assert_eq!(
            tasks.pending_ledger_acks(),
            0,
            "actual batch serial releases the acknowledgement without egress"
        );
    }
}

/// Interface plan I13: the exact receipt not only installs the typed
/// post-images, it announces them — the fixture's observer hears one
/// `ActionStateChanged` for the touched family and its line reaches the
/// gateway with the commit's own release.
#[tokio::test]
async fn committed_prepared_action_announces_the_touched_family_to_observers() {
    let (mut engine, handle) = prepared_lane_fixture();
    let mut transaction = engine
        .action_transaction(&handle)
        .expect("test transaction begins");
    transaction
        .insert(&handle, &41, PreparedLaneState { amount: 7 })
        .expect("test state is valid");
    let prepared = transaction.prepare().expect("test transaction prepares");
    let bundle = prepared_lane_bundle(&prepared, 13);
    let expected_token = bundle.token();
    let guards = empty_guards(&engine);
    engine
        .stage_authoritative_action(prepared, guards, bundle)
        .expect("clean engine accepts prepared action");

    let (writer, mut writer_rx) = persist::test_store_writer_channel(Arc::new(Metrics::new()));
    let mut health = StoreHealth::new();
    let metrics = Metrics::new();
    let mut retained = RetainedSeals::new();
    let high_water = AtomicU64::new(0);
    let outcome = flush_prepared_action(
        &mut engine,
        &writer,
        &mut health,
        &metrics,
        &mut retained,
        &high_water,
    )
    .await;
    let FlushOutcome::Queued {
        receipt: Some(receipt),
        prepared_installation: Some(installation),
    } = outcome
    else {
        panic!("prepared action must queue with its receipt and install image");
    };
    settle_queued_authoritative_job(writer_rx.recv().await.expect("prepared write was queued"))
        .await;
    let mut receipt = receipt;
    let ack = receipt.receive().await;
    let mut pending = pending_with_prepared_installation(0, installation);
    pending.token = CommitToken::from_authoritative(expected_token);
    let recorder = Arc::new(RecordingGateway {
        discord: Mutex::default(),
        sends: Mutex::new(Vec::new()),
        batches: Mutex::default(),
    });
    let gateway: Gateway = recorder.clone();
    let persistence_integrity = PersistenceIntegrityLatch::new();
    let (tx, _rx) = mpsc::channel(1);
    let completed = complete_pending_commit(
        pending,
        ack,
        &mut engine,
        &gateway,
        &mut health,
        &persistence_integrity,
        &tx.downgrade(),
        None,
        &test_effect_store(),
    );
    assert_eq!(completed.outcome, DeliveryOutcome::Committed);
    assert_eq!(
        engine
            .action_state()
            .get(&handle, &41)
            .expect("state lookup succeeds"),
        Some(&PreparedLaneState { amount: 7 })
    );
    let announced: Vec<String> = recorder
        .sends
        .lock()
        .expect("recording gateway poisoned")
        .iter()
        .filter_map(|(target, event)| match (target, event.as_ref()) {
            (SendTarget::Char(1), OutputEvent::Sys(text)) => Some(text.to_plain_text()),
            _ => None,
        })
        .collect();
    assert_eq!(
        announced,
        vec!["announced prepared_lane_test/receipt_state [\"41\"]".to_string()],
        "one announcement per touched family, released with the commit"
    );
}

/// The other half of the seam: a refused receipt installs nothing and
/// announces nothing — observers never hear about images that never went
/// live.
#[test]
fn refused_prepared_action_announces_nothing() {
    let (mut engine, handle) = prepared_lane_fixture();
    let mut transaction = engine
        .action_transaction(&handle)
        .expect("test transaction begins");
    transaction
        .insert(&handle, &43, PreparedLaneState { amount: 2 })
        .expect("test state is valid");
    let prepared = transaction.prepare().expect("test transaction prepares");
    let (_, installation) = prepared.into_parts();
    let pending = pending_with_prepared_installation(7, installation.into());
    let recorder = Arc::new(RecordingGateway {
        discord: Mutex::default(),
        sends: Mutex::new(Vec::new()),
        batches: Mutex::default(),
    });
    let gateway: Gateway = recorder.clone();
    let mut health = StoreHealth::new();
    let persistence_integrity = PersistenceIntegrityLatch::new();
    let (tx, _rx) = mpsc::channel(1);
    let completed = complete_pending_commit(
        pending,
        Err(AuthoritativeCommitFailure::Refused(
            "test refusal".to_owned(),
        )),
        &mut engine,
        &gateway,
        &mut health,
        &persistence_integrity,
        &tx.downgrade(),
        None,
        &test_effect_store(),
    );
    assert_eq!(completed.outcome, DeliveryOutcome::Aborted);
    assert!(
        engine
            .action_state()
            .get(&handle, &43)
            .expect("state lookup succeeds")
            .is_none(),
        "a refused receipt installs nothing"
    );
    let heard = recorder
        .sends
        .lock()
        .expect("recording gateway poisoned")
        .iter()
        .any(|(_, event)| {
            matches!(event.as_ref(), OutputEvent::Sys(text) if text.to_plain_text().starts_with("announced "))
        });
    assert!(
        !heard,
        "nothing is announced for images that never went live"
    );
}

#[tokio::test]
async fn authoritative_prepared_action_queues_exact_bundle_and_installs_only_on_exact_receipt() {
    let (mut engine, handle) = prepared_lane_fixture();
    let mut transaction = engine.action_transaction(&handle).unwrap();
    transaction
        .insert(&handle, &42, PreparedLaneState { amount: 9 })
        .unwrap();
    let prepared = transaction.prepare().unwrap();
    let bundle = prepared_lane_bundle(&prepared, 91);
    let expected_token = bundle.token();
    let expected_hash = bundle.bundle_hash();
    let guards = empty_guards(&engine);
    engine
        .stage_authoritative_action(prepared, guards, bundle.clone())
        .expect("matching bundle stages");

    let (writer, mut writer_rx) = persist::test_store_writer_channel(Arc::new(Metrics::new()));
    let mut serial = 0;
    let mut health = StoreHealth::new();
    let metrics = Metrics::new();
    let mut retained = RetainedSeals::new();
    let high_water = AtomicU64::new(0);
    let persistence_integrity = PersistenceIntegrityLatch::new();
    let outcome = flush_engine_step(
        &mut engine,
        &writer,
        &mut serial,
        &mut health,
        &metrics,
        &mut retained,
        &high_water,
        &crate::quarantine::QuarantineSpool::in_memory(),
        &persistence_integrity,
    )
    .await;
    let FlushOutcome::Queued {
        receipt: Some(mut pending_receipt),
        prepared_installation: Some(installation),
    } = outcome
    else {
        panic!("authoritative action must queue exact receipt custody");
    };
    assert_eq!(serial, 0, "K3 tokens do not consume legacy serials");
    assert_eq!(
        pending_receipt.token(),
        CommitToken::from_authoritative(expected_token)
    );
    let StoreJob::Authoritative {
        bundle: queued,
        ownership,
        notify,
    } = writer_rx.recv().await.expect("exact bundle queued")
    else {
        panic!("K3 staging must not use legacy WriteBatch");
    };
    assert_eq!(queued.as_ref(), &bundle);

    let store = InMemoryStore::new();
    let intent = store
        .reserve_authoritative_intent((*queued).clone())
        .await
        .unwrap();
    ownership.send(Ok(intent)).unwrap();
    let exact = store
        .reconcile_authoritative(expected_token, expected_hash)
        .await
        .unwrap();
    notify.send(Ok(exact)).unwrap();
    let owned = pending_receipt.receive_ownership().await.unwrap();
    assert_eq!(owned.token, expected_token);
    assert_eq!(owned.bundle_hash, expected_hash);
    let ack = pending_receipt.receive().await;

    let mut pending = pending_with_prepared_installation(0, installation);
    pending.token = CommitToken::from_authoritative(expected_token);
    let gateway: Gateway = Arc::new(BlackholeGateway);
    let (tx, _rx) = mpsc::channel(1);
    let completed = complete_pending_commit(
        pending,
        ack,
        &mut engine,
        &gateway,
        &mut health,
        &persistence_integrity,
        &tx.downgrade(),
        None,
        &test_effect_store(),
    );
    assert_eq!(completed.outcome, DeliveryOutcome::Committed);
    assert_eq!(
        engine.action_state().get(&handle, &42).unwrap(),
        Some(&PreparedLaneState { amount: 9 })
    );
    assert!(!persistence_integrity.is_latched());
}

#[test]
fn definitive_pre_intent_refusal_aborts_typed_image_without_integrity_freeze() {
    let (mut engine, handle) = prepared_lane_fixture();
    let installation = prepare_lane_installation(&mut engine, &handle, 73, 12);
    let pending = pending_with_prepared_installation(9, installation);
    let gateway: Gateway = Arc::new(BlackholeGateway);
    let mut health = StoreHealth::new();
    let persistence_integrity = PersistenceIntegrityLatch::new();
    let (tx, _rx) = mpsc::channel(1);

    let completed = complete_pending_commit(
        pending,
        Err(AuthoritativeCommitFailure::Refused(
            "authoritative capacity refused before intent".to_owned(),
        )),
        &mut engine,
        &gateway,
        &mut health,
        &persistence_integrity,
        &tx.downgrade(),
        None,
        &test_effect_store(),
    );

    assert_eq!(completed.outcome, DeliveryOutcome::Aborted);
    assert_eq!(engine.action_state().get(&handle, &73).unwrap(), None);
    assert!(!engine.action_state().has_prepared());
    assert!(!persistence_integrity.is_latched());
}

#[test]
fn authoritative_stage_rejects_bundle_that_does_not_match_typed_post_images() {
    let (mut engine, handle) = prepared_lane_fixture();
    let mut first = engine.action_transaction(&handle).unwrap();
    first
        .insert(&handle, &51, PreparedLaneState { amount: 1 })
        .unwrap();
    let first = first.prepare().unwrap();
    let mismatched_bundle = prepared_lane_bundle(&first, 101);
    let (_, first_installation) = first.into_parts();
    engine
        .abort_prepared_action(first_installation)
        .expect("the fixture aborts");

    let mut second = engine.action_transaction(&handle).unwrap();
    second
        .insert(&handle, &51, PreparedLaneState { amount: 2 })
        .unwrap();
    let second = second.prepare().unwrap();
    let guards = empty_guards(&engine);
    assert_eq!(
        engine.stage_authoritative_action(second, guards, mismatched_bundle),
        Err(mush_core::StagePreparedActionError::AuthoritativeWriteSetMismatch)
    );
    assert!(!engine.has_staged_action());
    assert!(!engine.action_state().has_prepared());
    assert_eq!(engine.action_state().revision(), 0);
    assert!(engine.action_transaction(&handle).is_ok());
}

#[tokio::test]
async fn stale_prepared_action_is_refused_before_writer_custody() {
    let (mut engine, handle) = prepared_lane_fixture();
    let mut transaction = engine
        .action_transaction(&handle)
        .expect("test transaction begins");
    transaction
        .insert(&handle, &43, PreparedLaneState { amount: 8 })
        .expect("test state is valid");
    let prepared = transaction.prepare().expect("test transaction prepares");
    let bundle = prepared_lane_bundle(&prepared, 23);
    let guards = prepared_lane_guards(&engine, 1);
    engine
        .stage_authoritative_action(prepared, guards, bundle)
        .expect("clean engine accepts prepared action");

    engine
        .world
        .module_state_mut::<PreparedLaneGeneration>()
        .expect("generation state is installed")
        .0 = 2;
    assert!(
        !engine.has_legacy_persistence_work(),
        "revision changes used by pure validation are not legacy writes"
    );

    let (writer, mut writer_rx) = persist::test_store_writer_channel(Arc::new(Metrics::new()));
    let mut serial = 0;
    let mut health = StoreHealth::new();
    let metrics = Metrics::new();
    let mut retained = RetainedSeals::new();
    let high_water = AtomicU64::new(0);
    let persistence_integrity = PersistenceIntegrityLatch::new();
    let outcome = flush_engine_step(
        &mut engine,
        &writer,
        &mut serial,
        &mut health,
        &metrics,
        &mut retained,
        &high_water,
        &crate::quarantine::QuarantineSpool::in_memory(),
        &persistence_integrity,
    )
    .await;

    match outcome {
        FlushOutcome::Refused(detail) => assert!(
            detail.contains("guards became stale"),
            "unexpected refusal: {detail}"
        ),
        _ => panic!("stale guards must refuse the prepared submission"),
    }
    assert_eq!(serial, 0, "a stale action must consume no commit serial");
    assert_eq!(health.mode, WorldMode::Writable);
    assert!(health.last_write_unix.is_none());
    assert!(
        matches!(writer_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
        "no stale action write may reach writer custody"
    );
    assert!(!engine.has_staged_action());
    assert!(!engine.action_state().has_prepared());
    assert_eq!(engine.action_state().revision(), 0);
    assert!(
        engine
            .action_state()
            .get(&handle, &43)
            .expect("state lookup succeeds")
            .is_none()
    );
    assert!(
        engine.action_transaction(&handle).is_ok(),
        "refusal must release the typed-state reservation"
    );
}

#[test]
fn prepared_action_integrity_failure_blocks_reuse_without_installation() {
    enum BadReceipt {
        WrongSerial,
        StoreIntegrityFailure,
    }

    for bad_receipt in [BadReceipt::WrongSerial, BadReceipt::StoreIntegrityFailure] {
        let (mut engine, handle) = prepared_lane_fixture();
        let installation = prepare_lane_installation(&mut engine, &handle, 17, 3);
        let pending = pending_with_prepared_installation(7, installation);
        let ack = match bad_receipt {
            BadReceipt::WrongSerial => Err(AuthoritativeCommitFailure::Integrity(
                "commit receipt identity mismatch: expected serial 7, received 8".to_owned(),
            )),
            BadReceipt::StoreIntegrityFailure => Err(AuthoritativeCommitFailure::Integrity(
                "indeterminate store receipt".to_owned(),
            )),
        };
        let gateway: Gateway = Arc::new(BlackholeGateway);
        let mut health = StoreHealth::new();
        let persistence_integrity = PersistenceIntegrityLatch::new();
        let (tx, _rx) = mpsc::channel(1);

        let completed = complete_pending_commit(
            pending,
            ack,
            &mut engine,
            &gateway,
            &mut health,
            &persistence_integrity,
            &tx.downgrade(),
            None,
            &test_effect_store(),
        );

        assert_eq!(completed.outcome, DeliveryOutcome::Aborted);
        assert_eq!(engine.action_state().revision(), 0);
        assert!(
            engine.action_state().has_prepared(),
            "without a proven refusal the typed reservation must remain poisoned"
        );
        assert!(
            engine
                .action_state()
                .get(&handle, &17)
                .expect("state lookup succeeds")
                .is_none(),
            "a bad receipt must discard the complete typed post-image"
        );
        assert!(health.mode.integrity_frozen());
        assert!(persistence_integrity.is_latched());
        assert!(
            engine.action_transaction(&handle).is_err(),
            "an indeterminate receipt must block another transaction"
        );
    }
}

#[tokio::test]
async fn prepared_action_writer_loss_aborts_before_receipt_custody() {
    let (mut engine, handle) = prepared_lane_fixture();
    let mut transaction = engine
        .action_transaction(&handle)
        .expect("test transaction begins");
    transaction
        .insert(&handle, &23, PreparedLaneState { amount: 5 })
        .expect("test state is valid");
    let prepared = transaction.prepare().expect("test transaction prepares");
    let bundle = prepared_lane_bundle(&prepared, 37);
    let guards = empty_guards(&engine);
    engine
        .stage_authoritative_action(prepared, guards, bundle)
        .expect("clean engine accepts prepared action");

    let (writer, writer_rx) = persist::test_store_writer_channel(Arc::new(Metrics::new()));
    drop(writer_rx);
    let mut health = StoreHealth::new();
    let metrics = Metrics::new();
    let mut retained = RetainedSeals::new();
    let high_water = AtomicU64::new(0);
    let outcome = flush_prepared_action(
        &mut engine,
        &writer,
        &mut health,
        &metrics,
        &mut retained,
        &high_water,
    )
    .await;

    assert!(matches!(outcome, FlushOutcome::WriterGone));
    assert_eq!(engine.action_state().revision(), 0);
    assert!(!engine.action_state().has_prepared());
    assert!(
        engine
            .action_state()
            .get(&handle, &23)
            .expect("state lookup succeeds")
            .is_none()
    );
    assert!(engine.action_transaction(&handle).is_ok());
}

/// With the K2 shim gone, the recheck at the head of `flush_prepared_action`
/// is the only remaining mixed-lane refusal, and it is permanent: staging
/// proves the step was clean, this proves it *stayed* clean. Both ways a
/// step can pick up legacy work after staging — a dirtied world and a
/// queued module op — must reject at submission, consume no commit serial,
/// and hand the writer nothing.
#[tokio::test]
async fn prepared_action_refuses_legacy_work_added_after_staging() {
    enum LateLegacyWork {
        WorldDirty,
        ModuleOp,
    }

    for late_work in [LateLegacyWork::WorldDirty, LateLegacyWork::ModuleOp] {
        let (mut engine, handle) = prepared_lane_fixture();
        let mut transaction = engine
            .action_transaction(&handle)
            .expect("test transaction begins");
        transaction
            .insert(&handle, &29, PreparedLaneState { amount: 9 })
            .expect("test state is valid");
        let prepared = transaction.prepare().expect("test transaction prepares");
        let bundle = prepared_lane_bundle(&prepared, 41);
        let guards = empty_guards(&engine);
        engine
            .stage_authoritative_action(prepared, guards, bundle)
            .expect("clean engine accepts prepared action");

        match late_work {
            LateLegacyWork::WorldDirty => {
                engine
                    .world
                    .create(ObjKind::Thing, "late dirty object", GOD, Some(LIMBO))
                    .expect("test object creation succeeds");
            }
            LateLegacyWork::ModuleOp => {
                let mut out = Outbox::new();
                let mut effects = Effects::new();
                engine
                    .modules_loaded(std::collections::HashMap::new(), &mut out, &mut effects)
                    .expect("test module queues its boot persistence op");
            }
        }
        assert!(engine.has_legacy_persistence_work());

        let (writer, mut writer_rx) = persist::test_store_writer_channel(Arc::new(Metrics::new()));
        let mut serial = 0;
        let mut health = StoreHealth::new();
        let metrics = Metrics::new();
        let mut retained = RetainedSeals::new();
        let high_water = AtomicU64::new(0);
        let persistence_integrity = PersistenceIntegrityLatch::new();
        let outcome = flush_engine_step(
            &mut engine,
            &writer,
            &mut serial,
            &mut health,
            &metrics,
            &mut retained,
            &high_water,
            &crate::quarantine::QuarantineSpool::in_memory(),
            &persistence_integrity,
        )
        .await;

        match outcome {
            FlushOutcome::Rejected(detail) => assert!(
                detail.contains("alongside legacy persistence work"),
                "unexpected refusal: {detail}"
            ),
            _ => panic!("late legacy work must reject the prepared submission"),
        }
        assert_eq!(serial, 0, "a refused mixed step must consume no serial");
        assert!(
            matches!(writer_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
            "no prefix of a mixed step may reach writer custody"
        );
        assert!(!engine.has_staged_action());
        assert!(!engine.action_state().has_prepared());
        assert_eq!(engine.action_state().revision(), 0);
        assert!(
            engine
                .action_state()
                .get(&handle, &29)
                .expect("state lookup succeeds")
                .is_none()
        );
        assert!(
            engine.has_legacy_persistence_work(),
            "the rejected prepared image must not discard independent legacy work"
        );
    }
}

#[tokio::test]
async fn aborted_exact_receipt_refuses_barrier_and_suppresses_held_success() {
    let mut pending = pending_for_custody_test();
    let (barrier_tx, barrier_rx) = oneshot::channel();
    assert!(pending.retain_barrier(barrier_tx));

    let mut engine = Engine::new(World::new_minimal(), base_registry());
    let recorder = Arc::new(RecordingGateway::default());
    let gateway: Gateway = recorder.clone();
    let mut health = StoreHealth::new();
    let persistence_integrity = PersistenceIntegrityLatch::new();
    let (tx, _rx) = mpsc::channel(1);
    let completed = complete_pending_commit(
        pending,
        Err(AuthoritativeCommitFailure::Integrity(
            "deterministic rejection".to_owned(),
        )),
        &mut engine,
        &gateway,
        &mut health,
        &persistence_integrity,
        &tx.downgrade(),
        None,
        &test_effect_store(),
    );
    finish_pending_operational(&mut engine, &gateway, completed);

    assert_eq!(
        barrier_rx.await,
        Ok(Err(WorldReadError::Unavailable)),
        "an aborted receipt must never be promoted into a barrier ack"
    );
    let sends = recorder.sends.lock().expect("recording gateway poisoned");
    assert!(
        sends.iter().any(|(_, event)| matches!(
            event.as_ref(),
            OutputEvent::Sys(text)
                if text.to_plain_text().contains("database rejected your last action")
        )),
        "the requester must receive the commit-failure refusal"
    );
    assert!(
        sends.iter().all(|(_, event)| !matches!(
            event.as_ref(),
            OutputEvent::Sys(text) if text.to_plain_text() == "held output"
        )),
        "a deterministic receipt failure must suppress held OnCommit success"
    );
}

#[test]
fn prolonged_commit_pending_operational_traffic_has_fixed_custody() {
    let mut pending = pending_for_custody_test();
    let baseline_sends = pending.sends.len();
    let baseline_effects = pending.effects.len();

    // Repeated full snapshots replace one another; they never append room
    // renders to the authoritative batch while its receipt is unresolved.
    for conn in 1..=10_000_u64 {
        let replaced = pending
            .coalesce_reattach(
                vec![ReAttachRec {
                    output_brackets: Default::default(),
                    conn,
                    actor: GOD,
                    caps_structured: conn % 2 == 0,
                }]
                .into_boxed_slice(),
            )
            .expect("one-record roster is within the protocol cap");
        assert_eq!(replaced, conn > 1);
    }

    let mut refused = 0;
    for _ in 0..(MAX_PENDING_BARRIERS + 10_000) {
        let (reply, _rx) = oneshot::channel();
        if !pending.retain_barrier(reply) {
            refused += 1;
        }
    }

    let custody = pending.custody();
    assert_eq!(custody.barriers, MAX_PENDING_BARRIERS);
    assert_eq!(custody.reattach_records, 1);
    assert!(custody.operational_bytes <= MAX_PENDING_OPERATIONAL_BYTES);
    assert_eq!(custody.sends, baseline_sends);
    assert_eq!(custody.effects, baseline_effects);
    assert_eq!(refused, 10_000);
    assert_eq!(
        pending
            .coalesced_reattach
            .as_deref()
            .expect("latest roster")[0]
            .conn,
        10_000,
        "only the newest full snapshot remains"
    );
}

#[test]
fn oversized_pending_reattach_is_atomic_and_preserves_previous_snapshot() {
    let mut pending = pending_for_custody_test();
    pending
        .coalesce_reattach(
            vec![ReAttachRec {
                output_brackets: Default::default(),
                conn: 41,
                actor: GOD,
                caps_structured: false,
            }]
            .into_boxed_slice(),
        )
        .expect("initial bounded roster");

    let oversized = (0..=MAX_PENDING_REATTACH_RECORDS)
        .map(|i| ReAttachRec {
            output_brackets: Default::default(),
            conn: u64::try_from(i).expect("test index fits u64") + 100,
            actor: GOD,
            caps_structured: true,
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let (_rejected, overload) = pending
        .coalesce_reattach(oversized)
        .expect_err("oversized roster must not enter pending custody");
    assert_eq!(overload.records, MAX_PENDING_REATTACH_RECORDS + 1);
    assert_eq!(
        pending.coalesced_reattach.as_deref().expect("old roster")[0].conn,
        41,
        "no prefix of a malformed replacement may reach server-side custody"
    );
}

#[tokio::test]
async fn world_handle_refuses_oversized_reattach_before_enqueue() {
    let (tx, mut rx) = mpsc::channel(2);
    let handle = WorldHandle {
        tx,
        shutdown: None,
        direct_actions: None,
    };
    let mut sparse = Vec::with_capacity(100_000);
    sparse.push(ReAttachRec {
        output_brackets: Default::default(),
        conn: 41,
        actor: GOD,
        caps_structured: false,
    });
    assert!(handle.reattach(sparse).await);
    let WorldMsg::ReAttach { recs } = rx.recv().await.expect("bounded roster enqueued") else {
        panic!("expected reattach message");
    };
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].conn, 41);
    // The message type is `Box<[ReAttachRec]>`, so the caller's 100,000
    // element spare Vec capacity cannot cross into channel custody.
    assert_eq!(
        std::mem::size_of_val(recs.as_ref()),
        std::mem::size_of::<ReAttachRec>()
    );

    let oversized = (0..=MAX_IN_FLIGHT_LIFECYCLE_OBLIGATIONS)
        .map(|i| ReAttachRec {
            output_brackets: Default::default(),
            conn: u64::try_from(i).expect("test index fits u64"),
            actor: GOD,
            caps_structured: false,
        })
        .collect();

    assert!(!handle.reattach(oversized).await);
    assert!(
        matches!(rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
        "an oversized roster must be refused as one unit, never partially enqueued"
    );
}

// ---- 16b cut B.5 (`B-13`): the defer queue --------------------------------
//
// A line that meets another action's commit used to be refused at once. It
// now waits in a bounded queue (four a connection, 64 in all, 250 ms from its
// arrival) and is served when the receipt lands, or refused in a player's
// words if the commit outlives the bound. The pure queue is tested first;
// then the loop, on the held-receipt rig: the store writer is a test channel,
// so a commit stays pending exactly as long as the test holds its receipt.

fn defer_line(conn: u64, line: &str) -> WorldMsg {
    WorldMsg::Inbound(Inbound::Cmd {
        conn,
        actor: GOD,
        line: line.to_owned(),
    })
}

fn defer_read() -> WorldMsg {
    WorldMsg::NameFree {
        name: "probe".to_owned(),
        reply: oneshot::channel().0,
    }
}

/// The typed line a queue entry holds, if it is one.
fn deferred_line(deferred: &super::Deferred) -> Option<&str> {
    match &deferred.entry {
        DeferEntry::Msg(WorldMsg::Inbound(Inbound::Cmd { line, .. })) => Some(line.as_str()),
        _ => None,
    }
}

#[test]
fn the_defer_queue_bounds_typed_lines_per_connection() {
    let now = DeferInstant::now();
    let mut queue = DeferQueue::default();
    for n in 0..COMMAND_DEFER_DEPTH {
        assert!(
            queue
                .push_msg(defer_line(1, &format!("line {n}")), now)
                .is_ok(),
            "line {n} of connection 1 waits"
        );
    }
    let refused = queue
        .push_msg(defer_line(1, "one too many"), now)
        .expect_err("a fifth typed line from one connection is handed back");
    assert!(matches!(
        refused,
        WorldMsg::Inbound(Inbound::Cmd { ref line, .. }) if line == "one too many"
    ));
    // Another connection's first line is its own.
    assert!(queue.push_msg(defer_line(2, "another player"), now).is_ok());
    // A read or a query counts against the cap only, never a connection.
    assert!(queue.push_msg(defer_read(), now).is_ok());
    assert!(
        queue
            .push_msg(
                WorldMsg::Inbound(Inbound::Query {
                    conn: 1,
                    actor: GOD,
                    id: 7,
                    name: "who".into(),
                    args: serde_json::json!({}),
                }),
                now,
            )
            .is_ok(),
        "a query from a connection with four lines waiting still waits"
    );
    assert_eq!(queue.entries.len(), COMMAND_DEFER_DEPTH + 3);
}

#[test]
fn the_defer_queue_is_capped_and_a_barrier_at_the_cap_is_handed_back() {
    let now = DeferInstant::now();
    let mut queue = DeferQueue::default();
    for _ in 0..COMMAND_DEFER_CAP {
        assert!(queue.push_msg(defer_read(), now).is_ok());
    }
    assert!(
        queue.push_msg(defer_read(), now).is_err(),
        "the 65th entry is handed back"
    );
    assert!(
        queue.push_msg(defer_line(9, "look"), now).is_err(),
        "and so is a connection's first line once the cap is reached"
    );
    // A barrier at the cap is handed back unanswered (the loop refuses it
    // `Busy`): answering it `Ok` would overtake the 64 entries ahead of it
    // and the commit they wait on.
    let (reply, mut answered) = oneshot::channel();
    let handed_back = queue
        .push_barrier(reply, now)
        .expect_err("a barrier at the cap is handed back");
    assert!(
        matches!(
            answered.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ),
        "the queue never answers a barrier itself"
    );
    drop(handed_back);
    assert_eq!(queue.entries.len(), COMMAND_DEFER_CAP);
    assert_eq!(queue.barriers(), 0);
    // One place free and the barrier waits like any other entry.
    assert!(queue.pop_front().is_some());
    let (reply, _answered) = oneshot::channel();
    assert!(queue.push_barrier(reply, now).is_ok());
    assert_eq!(queue.barriers(), 1);
}

#[test]
fn the_defer_queue_expires_only_the_aged_in_arrival_order_and_never_a_barrier() {
    let start = DeferInstant::now();
    let later = start + Duration::from_millis(100);
    let mut queue = DeferQueue::default();
    assert_eq!(
        queue.oldest_deadline(),
        None,
        "an empty queue has no deadline"
    );
    let (early_barrier, mut early_barrier_rx) = oneshot::channel();
    assert!(queue.push_barrier(early_barrier, start).is_ok());
    assert_eq!(
        queue.oldest_deadline(),
        None,
        "a barrier alone never arms the timer"
    );
    assert!(queue.push_msg(defer_line(1, "first"), start).is_ok());
    assert!(queue.push_msg(defer_line(2, "second"), start).is_ok());
    let (late_barrier, _late_barrier_rx) = oneshot::channel();
    assert!(queue.push_barrier(late_barrier, later).is_ok());
    assert!(queue.push_msg(defer_line(1, "third"), later).is_ok());
    assert_eq!(
        queue.oldest_deadline(),
        Some(start + COMMAND_DEFER),
        "the earliest message's arrival plus the bound"
    );

    // A millisecond short of the bound nothing goes.
    assert!(
        queue
            .expire(start + COMMAND_DEFER - Duration::from_millis(1))
            .is_empty()
    );
    // At the bound the two early lines go, front to back; the late one and
    // both barriers stay, in their places.
    let expired = queue.expire(start + COMMAND_DEFER);
    let lines: Vec<&str> = expired
        .iter()
        .filter_map(|msg| match msg {
            WorldMsg::Inbound(Inbound::Cmd { line, .. }) => Some(line.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(lines, ["first", "second"]);
    assert_eq!(queue.entries.len(), 3);
    assert!(matches!(queue.entries[0].entry, DeferEntry::Barrier(_)));
    assert!(matches!(queue.entries[1].entry, DeferEntry::Barrier(_)));
    assert_eq!(deferred_line(&queue.entries[2]), Some("third"));
    assert_eq!(queue.barriers(), 2);
    assert_eq!(queue.oldest_deadline(), Some(later + COMMAND_DEFER));
    // Long after, the last line goes and the barriers still stay.
    assert_eq!(queue.expire(later + Duration::from_secs(60)).len(), 1);
    assert_eq!(queue.barriers(), 2);
    assert_eq!(queue.oldest_deadline(), None);
    assert!(
        matches!(
            early_barrier_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ),
        "an expiry never answers a barrier"
    );
}

#[test]
fn the_defer_queue_pops_in_arrival_order_and_a_kept_entry_keeps_its_place() {
    let start = DeferInstant::now();
    let mut queue = DeferQueue::default();
    for (n, line) in ["a", "b", "c"].into_iter().enumerate() {
        assert!(
            queue
                .push_msg(
                    defer_line(1, line),
                    start + Duration::from_millis(u64::try_from(n).expect("small")),
                )
                .is_ok()
        );
    }
    let first = queue.pop_front().expect("a");
    assert_eq!(deferred_line(&first), Some("a"));
    assert_eq!(first.since, start);
    // A drained line that met a commit again goes back to the FRONT with
    // its first deadline, not to the back with a new one.
    queue.push_front_kept(first);
    assert_eq!(queue.oldest_deadline(), Some(start + COMMAND_DEFER));
    let order: Vec<String> = queue
        .drain_all()
        .iter()
        .filter_map(|deferred| deferred_line(deferred).map(str::to_owned))
        .collect();
    assert_eq!(order, ["a", "b", "c"]);
    assert!(queue.is_empty());
    assert!(queue.pop_front().is_none());
}

#[test]
fn what_waits_in_the_defer_queue_is_retryable_input_that_holds_no_custody() {
    let (reply, _) = oneshot::channel();
    for msg in [
        defer_line(1, "look"),
        WorldMsg::Inbound(Inbound::Query {
            conn: 1,
            actor: GOD,
            id: 1,
            name: "who".into(),
            args: serde_json::json!({}),
        }),
        defer_read(),
        WorldMsg::UnlinkPlayer {
            character: GOD,
            reply: oneshot::channel().0,
        },
        WorldMsg::InInputContext {
            actor: GOD,
            reply: oneshot::channel().0,
        },
        WorldMsg::ConnectionReady {
            conn: 1,
            actor: GOD,
            reply: oneshot::channel().0,
        },
        WorldMsg::LegalActions {
            conn: 1,
            actor: GOD,
            subject: PerceivedRef::SelfActor,
            reply: oneshot::channel().0,
        },
    ] {
        assert!(waits_in_defer_queue(&msg));
        assert!(!must_defer_behind_commit(&msg));
    }
    for msg in [
        WorldMsg::Inbound(Inbound::Composing {
            conn: 1,
            actor: GOD,
            active: true,
        }),
        WorldMsg::CreatePlayer {
            name: "Nope".into(),
            account: None,
            reply: oneshot::channel().0,
        },
        WorldMsg::Inbound(Inbound::Control(ControlMsg::Dump)),
        WorldMsg::Barrier { reply },
    ] {
        assert!(!waits_in_defer_queue(&msg));
    }
    assert_eq!(
        RefusalReason::CommitTimedOut.text(),
        "The world was busy too long for that line; please retry in a moment."
    );
    assert!(
        RefusalReason::CommitTimedOut
            .text()
            .ends_with("please retry in a moment."),
        "the phrase every rig's commit-pending retry needle matches"
    );
}

/// A timed-out entry that is not a typed line hears exactly what
/// `CommitPending` gives it: a read's typed `Busy`, a query its `Reply`
/// wrapping the `CommitPending` sentence. Only a line gets the new words.
#[test]
fn a_timed_out_read_or_query_is_refused_as_commit_pending() {
    let recorder = Arc::new(RecordingGateway::default());
    let gateway: Gateway = recorder.clone();

    let (reply, mut legal) = oneshot::channel();
    refuse_world_msg(
        WorldMsg::LegalActions {
            conn: 1,
            actor: GOD,
            subject: PerceivedRef::SelfActor,
            reply,
        },
        RefusalReason::CommitTimedOut,
        &gateway,
        None,
    );
    assert_eq!(legal.try_recv(), Ok(Err(LegalActionReadError::Busy)));

    let (reply, mut ready) = oneshot::channel();
    refuse_world_msg(
        WorldMsg::ConnectionReady {
            conn: 1,
            actor: GOD,
            reply,
        },
        RefusalReason::CommitTimedOut,
        &gateway,
        None,
    );
    assert_eq!(ready.try_recv(), Ok(Err(WorldReadError::Busy)));

    refuse_world_msg(
        WorldMsg::Inbound(Inbound::Query {
            conn: 1,
            actor: GOD,
            id: 9,
            name: "who".into(),
            args: serde_json::json!({}),
        }),
        RefusalReason::CommitTimedOut,
        &gateway,
        None,
    );
    assert_eq!(
        recorder.reply_text(9).as_deref(),
        Some(RefusalReason::CommitPending.text())
    );

    refuse_world_msg(
        defer_line(1, "look"),
        RefusalReason::CommitTimedOut,
        &gateway,
        None,
    );
    let lines: Vec<String> = recorder
        .sends
        .lock()
        .expect("recording gateway poisoned")
        .iter()
        .filter_map(|(_, event)| match event.as_ref() {
            OutputEvent::Sys(text) => Some(text.to_plain_text()),
            _ => None,
        })
        .collect();
    assert_eq!(lines, [RefusalReason::CommitTimedOut.text()]);
}

/// The world task on a store writer the test answers by hand.
struct HeldReceiptRig {
    handle: WorldHandle,
    writer_rx: persist::StoreWriterTestReceiver,
    metrics: Arc<Metrics>,
    recorder: Arc<RecordingGateway>,
    world_join: tokio::task::JoinHandle<WorldOutcome>,
}

type HeldReceipt = oneshot::Sender<Result<u64, String>>;

///
/// A freshly booted minimal world is dirty (Limbo and God were never
/// written), and whichever wake runs first flushes that as a write of its
/// own. The rig settles it before handing over — two barriers, answering
/// every write meanwhile — so the test's first command is the first commit
/// and meets no predecessor.
async fn held_receipt_rig() -> HeldReceiptRig {
    let engine = boot_engine();
    let (tx, rx) = mpsc::channel(INBOUND_CAPACITY);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let handle = WorldHandle {
        tx: tx.clone(),
        shutdown: Some(shutdown_tx),
        direct_actions: None,
    };
    let (staff_tx, _staff_rx) = mpsc::channel(STAFF_OPS_CAPACITY);
    let (_signal_tx, signal_rx) = writer_signal_channel();
    let metrics = Arc::new(Metrics::new());
    let (writer, writer_rx) = persist::test_store_writer_channel(Arc::clone(&metrics));
    let mut deps = ServerDeps::in_memory();
    deps.metrics = Arc::clone(&metrics);
    let recorder = Arc::new(RecordingGateway::default());
    let gateway: Gateway = recorder.clone();
    let world_join = tokio::spawn(run(
        engine,
        rx,
        tx.downgrade(),
        writer,
        deps,
        staff_tx,
        gateway,
        Arc::new(AtomicU64::new(0)),
        signal_rx,
        shutdown_rx,
        1,
        PersistenceIntegrityLatch::new(),
        WorldExitStatus::new(),
        None,
        None,
        watch::channel(false).0,
        // 16b cut T.2: this rig composes no game layer, so there is no arena
        // table and the egress arm stays disarmed.
        None,
        DEFAULT_WORLD_CLOCK_CHECKPOINT_S,
    ));
    let mut rig = HeldReceiptRig {
        handle,
        writer_rx,
        metrics,
        recorder,
        world_join,
    };
    rig.settle().await;
    rig.settle().await;
    rig
}

impl HeldReceiptRig {
    /// Run a barrier to completion, answering every write meanwhile, so
    /// nothing the world had to flush is left pending for the proof.
    async fn settle(&mut self) {
        let barrier = tokio::spawn({
            let handle = self.handle.clone();
            async move { handle.barrier().await }
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while !barrier.is_finished() {
                if let Ok(StoreJob::Write {
                    batch,
                    notify: Some(receipt),
                }) = self.writer_rx.try_recv()
                {
                    let _ = receipt.send(Ok(batch.serial));
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the pending write settles");
        assert!(
            barrier.await.expect("barrier task"),
            "the pending write lands"
        );
    }

    /// Attach `conn` as God, settled, so reads that ask about the
    /// connection have one to find.
    async fn attach(&mut self, conn: u64) {
        assert!(
            self.handle
                .inbound(Inbound::ConnUp {
                    conn,
                    actor: GOD,
                    caps_structured: true,
                    from_linkdead: false,
                })
                .await
        );
        self.settle().await;
    }

    /// The guard a served-path proof takes before it sends the receipt:
    /// the 250 ms bound is wall-clock, so a starved runner can age a queued
    /// entry out before the test answers, and that must name itself rather
    /// than read as a queue defect.
    fn nothing_timed_out_yet(&self) {
        assert_eq!(
            self.metrics.snapshot().command_defer_timed_out_total,
            0,
            "the runner was too slow: a queued entry outlived its 250 ms \
             bound before the receipt was sent, so this proof cannot run"
        );
    }
    /// Type one line as God on `conn`.
    async fn line(&self, conn: u64, line: &str) {
        assert!(
            self.handle
                .inbound(Inbound::Cmd {
                    conn,
                    actor: GOD,
                    line: line.to_owned(),
                })
                .await
        );
    }

    /// The next exact-receipt write, left unanswered.
    async fn next_write(&mut self) -> (u64, HeldReceipt) {
        match tokio::time::timeout(Duration::from_secs(2), self.writer_rx.recv())
            .await
            .expect("a dirty step must enqueue a write")
            .expect("writer channel open")
        {
            StoreJob::Write {
                batch,
                notify: Some(receipt),
            } => (batch.serial, receipt),
            _ => panic!("expected an exact-receipt write"),
        }
    }

    /// Every line of text the gateway was handed, in order.
    fn heard(&self) -> Vec<String> {
        self.recorder
            .sends
            .lock()
            .expect("recording gateway poisoned")
            .iter()
            .filter_map(|(_, event)| match event.as_ref() {
                OutputEvent::Sys(text) | OutputEvent::Narrative { text, .. } => {
                    Some(text.to_plain_text())
                }
                _ => None,
            })
            .collect()
    }

    fn heard_any(&self, needle: &str) -> bool {
        self.heard().iter().any(|line| line.contains(needle))
    }

    /// Wait (bounded) until `done` holds, answering every exact-receipt
    /// write the world makes meanwhile, so a line that turns out to commit
    /// cannot wedge the proof.
    async fn serve_until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !done(self) {
                if let Ok(StoreJob::Write {
                    batch,
                    notify: Some(receipt),
                }) = self.writer_rx.try_recv()
                {
                    let _ = receipt.send(Ok(batch.serial));
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}; heard {:?}", self.heard()));
    }

    async fn shut_down(self) {
        assert!(
            self.handle
                .inbound(Inbound::Control(ControlMsg::Shutdown))
                .await
        );
        tokio::time::timeout(Duration::from_secs(2), self.world_join)
            .await
            .expect("world shutdown timed out")
            .expect("world task panicked");
    }
}

/// **16b cut B.5 (`B-13`), the served case.** Four lines typed behind a
/// held commit wait; a fifth from the same connection is refused at once
/// in the old words; the receipt lands and the four are answered, in the
/// order they were typed, each counted once.
#[tokio::test]
async fn a_line_behind_a_commit_is_served_when_the_receipt_lands() {
    let mut rig = held_receipt_rig().await;
    rig.line(1, "@create receipt-anchor").await;
    let (serial, receipt) = rig.next_write().await;

    for n in 1..=4 {
        rig.line(1, &format!("think typed-{n}")).await;
    }
    rig.line(1, "think one-too-many").await;
    rig.serve_until("the fifth line's refusal", |rig| {
        rig.heard_any("committing another action")
    })
    .await;
    rig.nothing_timed_out_yet();
    receipt
        .send(Ok(serial))
        .expect("world still owns the exact receipt");

    rig.serve_until("the four queued lines", |rig| {
        (1..=4).all(|n| rig.heard_any(&format!("typed-{n}")))
    })
    .await;
    let heard = rig.heard();
    let at = |needle: &str| {
        heard
            .iter()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle} not heard: {heard:?}"))
    };
    assert!(
        at("Created: receipt-anchor") < at("typed-1")
            && at("typed-1") < at("typed-2")
            && at("typed-2") < at("typed-3")
            && at("typed-3") < at("typed-4"),
        "the anchor's answer, then the four lines in the order typed: {heard:?}"
    );
    assert!(
        !rig.heard_any("one-too-many"),
        "the refused fifth line never ran"
    );
    assert!(
        !rig.heard_any("busy too long"),
        "nothing outlived the bound: {heard:?}"
    );
    let snapshot = rig.metrics.snapshot();
    assert_eq!(snapshot.command_deferred_total, 4);
    assert_eq!(snapshot.command_defer_timed_out_total, 0);
    assert_eq!(
        snapshot.commands_total, 6,
        "the anchor, the four served lines and the refused fifth, once each"
    );
    rig.shut_down().await;
}

// ---- 16b cut AP.4 (16c `N20`): OUTPUTPREFIX/OUTPUTSUFFIX at the boundary ----
//
// The engine pushes the prefix `BestEffort` first and the suffix as a
// `Trailer` last; these prove what the world task's two partitions and its
// receipt release make of them, and that its own pre-engine refusals carry
// the same row.

/// Every `(target, text)` the recorder was handed, `Sys` and narrative.
fn ap4_heard(recorder: &RecordingGateway) -> Vec<(SendTarget, String)> {
    recorder
        .sends
        .lock()
        .expect("recording gateway poisoned")
        .iter()
        .filter_map(|(target, event)| match event.as_ref() {
            OutputEvent::Sys(text) | OutputEvent::Narrative { text, .. } => {
                Some((target.clone(), text.to_plain_text()))
            }
            _ => None,
        })
        .collect()
}

/// **The committing order, end to end on the held-receipt rig.** A typed
/// line whose step queues a receipt transmits `>>>` before the ack (with
/// the step's `BestEffort` sends) and nothing else; the ack then releases
/// the line's `OnCommit` answer and `<<<` after it.
#[tokio::test]
async fn ap4_a_committing_line_reads_its_prefix_before_the_ack_and_its_suffix_after_it() {
    let mut rig = held_receipt_rig().await;
    rig.attach(1).await;
    rig.line(1, "OUTPUTPREFIX >>>").await;
    rig.line(1, "OUTPUTSUFFIX <<<").await;
    rig.serve_until("the suffix line's own pair", |rig| rig.heard_any("<<<"))
        .await;
    let before = ap4_heard(&rig.recorder).len();

    rig.line(1, "@create bracket-anchor").await;
    let (serial, receipt) = rig.next_write().await;
    rig.serve_until("the prefix, before the ack", |rig| {
        ap4_heard(&rig.recorder)[before..]
            .iter()
            .any(|(_, text)| text == ">>>")
    })
    .await;
    let held = ap4_heard(&rig.recorder)[before..].to_vec();
    assert_eq!(
        held,
        vec![(SendTarget::Conn(1), ">>>".to_owned())],
        "before the ack only the prefix crosses: {held:?}"
    );
    receipt
        .send(Ok(serial))
        .expect("world still owns the exact receipt");
    rig.serve_until("the suffix, after the ack", |rig| {
        ap4_heard(&rig.recorder)[before..]
            .iter()
            .any(|(_, text)| text == "<<<")
    })
    .await;
    let heard = ap4_heard(&rig.recorder)[before..].to_vec();
    assert_eq!(
        heard.first(),
        Some(&(SendTarget::Conn(1), ">>>".to_owned()))
    );
    assert_eq!(heard.last(), Some(&(SendTarget::Conn(1), "<<<".to_owned())));
    assert!(
        heard[1..heard.len() - 1]
            .iter()
            .any(|(_, text)| text.contains("Created: bracket-anchor")),
        "the line's committed answer sits inside the pair: {heard:?}"
    );
    rig.shut_down().await;
}

/// **The aborted order, end to end.** The same committing line, its
/// receipt failed: `>>>` crossed before the ack, the `OnCommit` answer is
/// suppressed, the commit-failure sentence still reaches the player (the
/// prefix is framing, not an answer that would silence it) and sits INSIDE
/// the pair, and `<<<` closes it.
#[tokio::test]
async fn ap4_an_aborted_line_reads_its_refusal_inside_the_pair_and_none_of_its_success() {
    let mut rig = held_receipt_rig().await;
    rig.attach(1).await;
    rig.line(1, "OUTPUTPREFIX >>>").await;
    rig.line(1, "OUTPUTSUFFIX <<<").await;
    rig.serve_until("the suffix line's own pair", |rig| rig.heard_any("<<<"))
        .await;
    let before = ap4_heard(&rig.recorder).len();

    rig.line(1, "@create bracket-anchor").await;
    let (_serial, receipt) = rig.next_write().await;
    receipt
        .send(Err("ap4 test failure".to_owned()))
        .expect("world still owns the exact receipt");
    rig.serve_until("the suffix, after the failed ack", |rig| {
        ap4_heard(&rig.recorder)[before..]
            .iter()
            .any(|(_, text)| text == "<<<")
    })
    .await;
    let heard = ap4_heard(&rig.recorder)[before..].to_vec();
    assert_eq!(
        heard.first(),
        Some(&(SendTarget::Conn(1), ">>>".to_owned()))
    );
    assert_eq!(heard.last(), Some(&(SendTarget::Conn(1), "<<<".to_owned())));
    assert!(
        heard
            .iter()
            .all(|(_, text)| !text.contains("Created: bracket-anchor")),
        "nothing of the OnCommit answer: {heard:?}"
    );
    assert!(
        heard[1..heard.len() - 1]
            .iter()
            .any(|(_, text)| text.contains("database rejected your last action")),
        "the failure sentence is said, inside the pair: {heard:?}"
    );
    rig.shut_down().await;
}

/// **The aborted order on the receipt release itself.** What a committing
/// step holds for the ack (its `OnCommit` answer, an `OnAbort` refusal and
/// its `Trailer`; the `BestEffort` prefix already crossed) releases on an
/// abort as the refusal then `<<<`, nothing of the success, and on a
/// commit as the success then `<<<`, nothing of the refusal. What another
/// step joined to the batch while the receipt was awaited stays AFTER the
/// suffix on a commit (nothing moves inside the pair) and is dropped on an
/// abort.
#[test]
fn ap4_the_held_trailer_closes_the_step_on_either_ack() {
    let held = || -> Vec<ClassifiedSend> {
        vec![
            (
                DeliveryClass::OnCommit,
                SendTarget::Char(u64::from(GOD.0)),
                Arc::new(OutputEvent::Sys(RichText::plain("Set."))),
            ),
            (
                DeliveryClass::OnAbort,
                SendTarget::Char(u64::from(GOD.0)),
                Arc::new(OutputEvent::Sys(RichText::tag("error", "refused"))),
            ),
            (
                DeliveryClass::Trailer,
                SendTarget::Conn(1),
                Arc::new(OutputEvent::Sys(RichText::tag(OUTPUT_SUFFIX_TAG, "<<<"))),
            ),
        ]
    };
    for (ack, expected) in [
        (Ok(()), vec!["Set.", "<<<", "joined"]),
        (
            Err(AuthoritativeCommitFailure::Refused("stale".to_owned())),
            vec!["refused", "<<<"],
        ),
    ] {
        let mut pending = pending_for_custody_test();
        pending.sends = held();
        // A tick's commit-bound line, joined behind the suffix while the
        // receipt was awaited (`pending.sends.extend` in `run`).
        pending.sends.push((
            DeliveryClass::OnCommit,
            SendTarget::Char(u64::from(GOD.0)),
            Arc::new(OutputEvent::Sys(RichText::plain("joined"))),
        ));
        pending.effects.clear();
        let mut engine = Engine::new(World::new_minimal(), base_registry());
        let recorder = Arc::new(RecordingGateway::default());
        let gateway: Gateway = recorder.clone();
        let mut health = StoreHealth::new();
        let persistence_integrity = PersistenceIntegrityLatch::new();
        let (tx, _rx) = mpsc::channel(1);
        let completed = complete_pending_commit(
            pending,
            ack,
            &mut engine,
            &gateway,
            &mut health,
            &persistence_integrity,
            &tx.downgrade(),
            None,
            &test_effect_store(),
        );
        finish_pending_operational(&mut engine, &gateway, completed);
        let heard: Vec<String> = ap4_heard(&recorder)
            .into_iter()
            .map(|(_, text)| text)
            .collect();
        assert_eq!(heard, expected);
    }

    // Refused before submission (a stale precondition): nothing crossed
    // early, so the step's whole batch is here — the prefix, the success,
    // the suffix. The prefix is framing, not an answer: the generic
    // "not committed" sentence is still said, and inside the pair.
    let recorder = Arc::new(RecordingGateway::default());
    let gateway: Gateway = recorder.clone();
    let (tx, _rx) = mpsc::channel(1);
    let mut step = vec![(
        DeliveryClass::BestEffort,
        SendTarget::Conn(1),
        Arc::new(OutputEvent::Sys(RichText::tag(OUTPUT_PREFIX_TAG, ">>>"))),
    )];
    step.extend(
        held()
            .into_iter()
            .filter(|(class, _, _)| *class != DeliveryClass::OnAbort),
    );
    refuse_before_submission(
        step,
        Vec::new(),
        Some(GOD),
        None,
        "stale guard",
        &gateway,
        &StoreHealth::new(),
        &tx.downgrade(),
        None,
        &test_effect_store(),
    );
    let heard: Vec<String> = ap4_heard(&recorder)
        .into_iter()
        .map(|(_, text)| text)
        .collect();
    assert_eq!(
        heard,
        vec![
            ">>>".to_owned(),
            "Your action was not committed (stale guard). Please retry against the current state."
                .to_owned(),
            "<<<".to_owned(),
        ]
    );

    // Rejected by the store with no receipt (`abort_without_receipt`, the
    // Rejected / WriterGone / no-receipt arms): the whole batch is here
    // too. The prefix is not an answer, so the integrity sentence is still
    // said, inside the pair; the suffix still closes it; the success is
    // suppressed.
    let recorder = Arc::new(RecordingGateway::default());
    let gateway: Gateway = recorder.clone();
    let (tx, _rx) = mpsc::channel(1);
    let mut health = StoreHealth::new();
    let mut step = vec![(
        DeliveryClass::BestEffort,
        SendTarget::Conn(1),
        Arc::new(OutputEvent::Sys(RichText::tag(OUTPUT_PREFIX_TAG, ">>>"))),
    )];
    step.extend(
        held()
            .into_iter()
            .filter(|(class, _, _)| *class != DeliveryClass::OnAbort),
    );
    abort_without_receipt(
        step,
        Vec::new(),
        Some(GOD),
        None,
        "rejected batch",
        &World::new_minimal(),
        &gateway,
        &mut health,
        &PersistenceIntegrityLatch::new(),
        &tx.downgrade(),
        None,
        &test_effect_store(),
    );
    let heard: Vec<String> = ap4_heard(&recorder)
        .into_iter()
        .map(|(_, text)| text)
        .filter(|text| !text.starts_with("GAME: persistence integrity breach"))
        .collect();
    assert_eq!(
        heard,
        vec![
            ">>>".to_owned(),
            "The database rejected your last action (rejected batch). Its success output \
             and external effects were suppressed; the world is now integrity-frozen."
                .to_owned(),
            "<<<".to_owned(),
        ]
    );
}

/// **A line held behind a save, deterministically.** The receipt of a
/// bracketed connection's committing line is held; a second line typed
/// meanwhile waits in 16b cut B.5's defer queue, so only the first line's
/// `>>>` has crossed. The ack releases the first line's answer and `<<<`,
/// then the served line's own `>>>`, its answer and `<<<` — two pairs in the
/// order typed, neither inside the other.
#[tokio::test]
async fn ap4_a_line_held_behind_a_save_reads_its_own_pair_after_the_saving_one() {
    let mut rig = held_receipt_rig().await;
    rig.attach(1).await;
    rig.line(1, "OUTPUTPREFIX >>>").await;
    rig.line(1, "OUTPUTSUFFIX <<<").await;
    rig.serve_until("the suffix line's own pair", |rig| rig.heard_any("<<<"))
        .await;
    let before = ap4_heard(&rig.recorder).len();

    rig.line(1, "@create held-anchor").await;
    let (serial, receipt) = rig.next_write().await;
    rig.line(1, "think held-line").await;
    rig.serve_until("the think, deferred behind the receipt", |rig| {
        rig.metrics.snapshot().command_deferred_total == 1
    })
    .await;
    let held = ap4_heard(&rig.recorder)[before..].to_vec();
    assert_eq!(
        held,
        vec![(SendTarget::Conn(1), ">>>".to_owned())],
        "before the ack only the saving line's prefix crosses: {held:?}"
    );
    rig.nothing_timed_out_yet();
    receipt
        .send(Ok(serial))
        .expect("world still owns the exact receipt");
    rig.serve_until("both suffixes", |rig| {
        ap4_heard(&rig.recorder)[before..]
            .iter()
            .filter(|(_, text)| text == "<<<")
            .count()
            == 2
    })
    .await;
    let heard: Vec<String> = ap4_heard(&rig.recorder)[before..]
        .iter()
        .map(|(_, text)| text.clone())
        .collect();
    let marks: Vec<usize> = heard
        .iter()
        .enumerate()
        .filter(|(_, text)| *text == ">>>" || *text == "<<<")
        .map(|(at, _)| at)
        .collect();
    assert_eq!(
        marks
            .iter()
            .map(|at| heard[*at].as_str())
            .collect::<Vec<_>>(),
        vec![">>>", "<<<", ">>>", "<<<"],
        "two pairs, neither inside the other: {heard:?}"
    );
    assert_eq!(marks[0], 0, "the saving line's prefix is first: {heard:?}");
    assert_eq!(
        marks[3],
        heard.len() - 1,
        "the served line's suffix is last: {heard:?}"
    );
    let first = &heard[marks[0] + 1..marks[1]];
    let second = &heard[marks[2] + 1..marks[3]];
    assert!(
        first
            .iter()
            .any(|text| text.contains("Created: held-anchor")),
        "the saving line's answer is in its own pair: {heard:?}"
    );
    assert!(
        second.iter().any(|text| text.contains("held-line"))
            && second.iter().all(|text| !text.contains("Created:")),
        "the served line's answer, and only that, is in its own pair: {heard:?}"
    );
    assert_eq!(rig.metrics.snapshot().command_defer_timed_out_total, 0);
    rig.shut_down().await;
}

/// **The world task's own refusal.** `refuse_world_msg` on a typed line
/// from a bracketed connection transmits the prefix, the refusal (worded
/// and tagged as `send_sys` words it) and the suffix to that connection in
/// ONE batch; on a connection with no row, the actor-addressed refusal
/// alone, as before.
#[test]
fn ap4_a_world_task_refusal_is_bracketed_from_the_lines_own_row() {
    let recorder = Arc::new(RecordingGateway::default());
    let gateway: Gateway = recorder.clone();
    let mut world = World::new_minimal();
    world.set_output_prefix(1, Some(">>>".to_owned()));
    world.set_output_suffix(1, Some("<<<".to_owned()));

    for reason in [RefusalReason::CommitPending, RefusalReason::CommitTimedOut] {
        let msg = defer_line(1, "look");
        let brackets = output_brackets_of(&world, &msg);
        refuse_world_msg(msg, reason, &gateway, brackets);
        // ONE transmit: nothing the world sends can land between the three.
        let batches =
            std::mem::take(&mut *recorder.batches.lock().expect("recording gateway poisoned"));
        assert_eq!(batches, vec![3], "one batch of three");
        let sends =
            std::mem::take(&mut *recorder.sends.lock().expect("recording gateway poisoned"));
        let shaped: Vec<(SendTarget, Option<String>, String)> = sends
            .iter()
            .map(|(target, event)| match event.as_ref() {
                OutputEvent::Sys(text) => (
                    target.clone(),
                    text.spans()
                        .first()
                        .and_then(|span| span.tag.as_deref().map(str::to_owned)),
                    text.to_plain_text(),
                ),
                other => panic!("only Sys lines: {other:?}"),
            })
            .collect();
        assert_eq!(
            shaped,
            vec![
                (
                    SendTarget::Conn(1),
                    Some(OUTPUT_PREFIX_TAG.to_owned()),
                    ">>>".to_owned()
                ),
                (
                    SendTarget::Conn(1),
                    Some("system".to_owned()),
                    reason.text().to_owned()
                ),
                (
                    SendTarget::Conn(1),
                    Some(OUTPUT_SUFFIX_TAG.to_owned()),
                    "<<<".to_owned()
                ),
            ]
        );
    }

    // A prefix alone frames the refusal from one side only.
    world.set_output_suffix(1, None);
    let msg = defer_line(1, "look");
    let brackets = output_brackets_of(&world, &msg);
    refuse_world_msg(msg, RefusalReason::StoreUnavailable, &gateway, brackets);
    let batches =
        std::mem::take(&mut *recorder.batches.lock().expect("recording gateway poisoned"));
    assert_eq!(batches, vec![2], "one batch of two");
    let heard = ap4_heard(&recorder);
    assert_eq!(
        heard,
        vec![
            (SendTarget::Conn(1), ">>>".to_owned()),
            (
                SendTarget::Conn(1),
                RefusalReason::StoreUnavailable.text().to_owned()
            ),
        ]
    );
    recorder
        .sends
        .lock()
        .expect("recording gateway poisoned")
        .clear();

    // Another connection's line, no row: the actor-addressed refusal.
    let msg = defer_line(2, "look");
    let brackets = output_brackets_of(&world, &msg);
    assert_eq!(brackets, None);
    refuse_world_msg(msg, RefusalReason::CommitTimedOut, &gateway, brackets);
    assert_eq!(
        ap4_heard(&recorder),
        vec![(
            SendTarget::Char(u64::from(GOD.0)),
            RefusalReason::CommitTimedOut.text().to_owned()
        )]
    );

    // Nothing that is not a typed line carries a row.
    let query = WorldMsg::Inbound(Inbound::Query {
        conn: 1,
        actor: GOD,
        id: 3,
        name: "who".into(),
        args: serde_json::json!({}),
    });
    assert_eq!(output_brackets_of(&world, &query), None);
}

/// The two words join the read-only allowlist: a RAM row on the World
/// needs no store, so a store outage does not refuse them.
#[test]
fn ap4_the_bracket_settings_pass_the_store_read_only_gate() {
    let world = World::new_minimal();
    for line in ["OUTPUTPREFIX >>>", "outputsuffix <<<", "OUTPUTSUFFIX"] {
        let cmd = Inbound::Cmd {
            conn: 1,
            actor: GOD,
            line: line.to_owned(),
        };
        assert!(
            refuse_store_read_only(&world, &cmd).is_none(),
            "{line} passes the read-only gate"
        );
    }
}

/// **16b cut B.5, the timed-out case.** A line queued behind a commit that
/// outlives the 250 ms bound is refused in a player's words while the
/// commit still pends, and never runs.
#[tokio::test]
async fn a_line_behind_a_commit_that_outlives_the_bound_is_refused_in_a_players_words() {
    let mut rig = held_receipt_rig().await;
    rig.line(1, "@create receipt-anchor").await;
    let (serial, receipt) = rig.next_write().await;

    let sent = Instant::now();
    rig.line(1, "think too-late").await;
    rig.serve_until("the timed-out refusal", |rig| {
        rig.heard_any(RefusalReason::CommitTimedOut.text())
    })
    .await;
    assert!(
        sent.elapsed() >= COMMAND_DEFER,
        "the line waited its whole bound before it was refused"
    );
    assert!(
        !rig.heard_any("committing another action"),
        "a line that waited is refused in the new words, not the old"
    );
    let snapshot = rig.metrics.snapshot();
    assert_eq!(snapshot.command_deferred_total, 1);
    assert_eq!(snapshot.command_defer_timed_out_total, 1);
    assert_eq!(snapshot.commands_total, 2, "the anchor and the refusal");

    // The receipt lands after the refusal, and the refused line stays
    // refused.
    receipt
        .send(Ok(serial))
        .expect("world still owns the exact receipt");
    assert!(rig.handle.barrier().await);
    assert!(!rig.heard_any("too-late"), "the timed-out line never ran");
    rig.shut_down().await;
}

/// **16b cut B.5, the barrier rule.** A barrier that arrives behind a queued
/// line waits for that line — and for the commit the line itself starts —
/// so `barrier_result`'s "everything sent before has been processed" stays
/// true.
#[tokio::test]
async fn a_barrier_behind_queued_lines_waits_for_them() {
    let mut rig = held_receipt_rig().await;
    rig.line(1, "@create receipt-anchor").await;
    let (first_serial, first_receipt) = rig.next_write().await;

    rig.line(2, "@create queued-orb").await;
    let barrier = tokio::spawn({
        let handle = rig.handle.clone();
        async move { handle.barrier().await }
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let snapshot = rig.metrics.snapshot();
            if snapshot.command_deferred_total == 1 && snapshot.commit_pending_barriers == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the line and the barrier behind it both wait");
    rig.nothing_timed_out_yet();
    first_receipt
        .send(Ok(first_serial))
        .expect("world still owns the first receipt");

    // The drained line is served and starts a commit of its own; the
    // barrier behind it is not finished after the FIRST receipt.
    let (second_serial, second_receipt) = rig.next_write().await;
    tokio::task::yield_now().await;
    assert!(
        !barrier.is_finished(),
        "the barrier finished before the queued line's own commit landed"
    );
    second_receipt
        .send(Ok(second_serial))
        .expect("world still owns the second receipt");
    assert!(
        tokio::time::timeout(Duration::from_secs(2), barrier)
            .await
            .expect("the barrier completes after the second receipt")
            .expect("barrier task"),
        "the barrier resolves Ok"
    );
    // `name_free` probes the PLAYER index, which an `@create`d thing never
    // joins, so the object's own creation line is the witness that the
    // queued line ran — released only after its exact receipt.
    assert!(
        rig.heard_any("Created: queued-orb"),
        "the queued line ran and its object exists: {:?}",
        rig.heard()
    );
    assert_eq!(rig.metrics.snapshot().commit_pending_barriers, 0);
    rig.shut_down().await;
}

/// Wait (bounded) until the world has taken everything sent so far off its
/// inbound channel — a lifecycle message parked in its slot included.
async fn inbound_taken(handle: &WorldHandle) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while handle.tx.capacity() < handle.tx.max_capacity() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the world takes its inbound");
}

/// Wait (bounded) until `done` holds, WITHOUT answering any write: the
/// held receipt stays held.
async fn wait_until(what: &str, done: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !done() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

/// **16b cut B.5, the reads.** The motivating loss of cut AA's sweep was a
/// world READ (`ConnectionReady`) that met another action's commit. The
/// reads and a query behind a held receipt wait, and once it lands each is
/// ANSWERED — not refused `Busy` — from the committed world.
#[tokio::test]
async fn a_read_and_a_query_behind_a_commit_are_answered_when_the_receipt_lands() {
    let mut rig = held_receipt_rig().await;
    rig.attach(1).await;
    rig.line(1, "@create receipt-anchor").await;
    let (serial, receipt) = rig.next_write().await;

    let name_free = tokio::spawn({
        let handle = rig.handle.clone();
        async move { handle.name_free("God").await }
    });
    let ready = tokio::spawn({
        let handle = rig.handle.clone();
        async move { handle.connection_ready(1, GOD).await }
    });
    let legal = tokio::spawn({
        let handle = rig.handle.clone();
        async move { handle.legal_actions(1, GOD, PerceivedRef::SelfActor).await }
    });
    assert!(
        rig.handle
            .inbound(Inbound::Query {
                conn: 1,
                actor: GOD,
                id: 7,
                name: "who".into(),
                args: serde_json::json!({}),
            })
            .await
    );
    let metrics = Arc::clone(&rig.metrics);
    wait_until("the three reads and the query to wait", || {
        metrics.snapshot().command_deferred_total == 4
    })
    .await;
    assert!(
        !name_free.is_finished() && !ready.is_finished() && !legal.is_finished(),
        "a read behind a held receipt waits; it is not refused at once"
    );
    assert_eq!(rig.recorder.reply_text(7), None, "the query waits too");
    rig.nothing_timed_out_yet();
    receipt
        .send(Ok(serial))
        .expect("world still owns the exact receipt");

    async fn answer<T>(join: tokio::task::JoinHandle<T>) -> T {
        tokio::time::timeout(Duration::from_secs(2), join)
            .await
            .expect("the read is answered after the receipt")
            .expect("read task")
    }
    assert_eq!(answer(name_free).await, Ok(false), "God's name is taken");
    assert_eq!(
        answer(ready).await,
        Ok(true),
        "the attached connection is ready"
    );
    // This rig composes no action provider, so the projection's own answer
    // is a typed failure of its own (`Unavailable` from the projector);
    // what matters is that the world ran the projection after the receipt
    // rather than refusing the read as busy.
    let legal = answer(legal).await;
    assert!(
        !matches!(legal, Err(LegalActionReadError::Busy)),
        "the legal-action read was answered, not refused: {legal:?}"
    );
    rig.serve_until("the query's reply", |rig| {
        rig.recorder.reply_text(7).is_some()
    })
    .await;
    let reply = rig.recorder.reply_text(7).unwrap_or_default();
    assert!(
        !reply.contains("committing another action") && !reply.contains("busy too long"),
        "the query was answered, not refused: {reply}"
    );
    let snapshot = rig.metrics.snapshot();
    assert_eq!(snapshot.command_deferred_total, 4);
    assert_eq!(snapshot.command_defer_timed_out_total, 0);
    rig.shut_down().await;
}

/// **16b cut B.5, queue before lifecycle.** A line typed before a `QUIT`
/// runs before the `ConnDown` it is followed by, though that `ConnDown`
/// reached the world while the line waited: the lifecycle slot is taken
/// only once the queue is empty.
#[tokio::test]
async fn a_line_typed_before_a_disconnect_runs_before_it() {
    let mut rig = held_receipt_rig().await;
    rig.attach(1).await;
    rig.line(2, "@create receipt-anchor").await;
    let (serial, receipt) = rig.next_write().await;

    rig.line(1, "who").await;
    assert!(
        rig.handle
            .inbound(Inbound::ConnDown {
                conn: 1,
                actor: GOD,
                reason: DisconnectKind::Quit,
            })
            .await
    );
    // The `ConnDown` is parked in the lifecycle slot before the receipt
    // lands, so the order below is the loop's, not the channel's.
    inbound_taken(&rig.handle).await;
    assert_eq!(rig.metrics.snapshot().command_deferred_total, 1);
    rig.nothing_timed_out_yet();
    receipt
        .send(Ok(serial))
        .expect("world still owns the exact receipt");

    let who = |rig: &HeldReceiptRig| {
        rig.recorder
            .sends
            .lock()
            .expect("recording gateway poisoned")
            .iter()
            .find_map(|(_, event)| match event.as_ref() {
                OutputEvent::Who(entries) => Some(
                    entries
                        .iter()
                        .map(|entry| entry.name.to_string())
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
    };
    rig.serve_until("the WHO typed before the quit", |rig| who(rig).is_some())
        .await;
    // Run before the `ConnDown`, the WHO still finds God connected; run
    // after it (the lifecycle slot overtaking the queue) it lists nobody.
    assert_eq!(
        who(&rig).as_deref(),
        Some(&["God".to_owned()][..]),
        "the line typed before the quit ran before it"
    );
    assert_eq!(
        rig.handle.connection_ready(1, GOD).await,
        Ok(false),
        "and the disconnect ran after it"
    );
    assert!(
        !rig.heard_any("committing another action") && !rig.heard_any("busy too long"),
        "{:?}",
        rig.heard()
    );
    rig.shut_down().await;
}

/// **16b cut B.5, the shutdown arm.** A shutdown behind a held receipt
/// refuses every queued line in the words it always used and keeps every
/// queued barrier behind the receipt, as a retained barrier is kept; the
/// world ends once the receipt lands.
#[tokio::test]
async fn a_shutdown_refuses_the_queued_lines_and_keeps_the_queued_barriers_behind_the_receipt() {
    let mut rig = held_receipt_rig().await;
    rig.line(1, "@create receipt-anchor").await;
    let (serial, receipt) = rig.next_write().await;

    rig.line(1, "think drained-at-shutdown").await;
    let barrier = tokio::spawn({
        let handle = rig.handle.clone();
        async move { handle.barrier_result().await }
    });
    let metrics = Arc::clone(&rig.metrics);
    wait_until("the line and the barrier behind it to wait", || {
        let snapshot = metrics.snapshot();
        snapshot.command_deferred_total == 1 && snapshot.commit_pending_barriers == 1
    })
    .await;
    rig.nothing_timed_out_yet();

    // The shutdown watch, not the inbound channel.
    assert!(
        rig.handle
            .inbound(Inbound::Control(ControlMsg::Shutdown))
            .await
    );
    let recorder = Arc::clone(&rig.recorder);
    wait_until("the queued line's refusal", || {
        recorder
            .sends
            .lock()
            .expect("recording gateway poisoned")
            .iter()
            .any(|(_, event)| {
                matches!(event.as_ref(), OutputEvent::Sys(text)
                    if text.to_plain_text().contains("committing another action"))
            })
    })
    .await;
    assert!(
        !barrier.is_finished(),
        "a queued barrier is kept behind the receipt, not answered at shutdown"
    );
    assert_eq!(
        rig.metrics.snapshot().commands_total,
        2,
        "the anchor and the refused line, once each"
    );

    let HeldReceiptRig {
        mut writer_rx,
        world_join,
        recorder,
        ..
    } = rig;
    receipt
        .send(Ok(serial))
        .expect("world still owns the exact receipt");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), barrier)
            .await
            .expect("the barrier completes with the receipt")
            .expect("barrier task"),
        Ok(()),
        "a barrier kept behind the receipt completes with it"
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while !world_join.is_finished() {
            if let Ok(StoreJob::Write {
                batch,
                notify: Some(receipt),
            }) = writer_rx.try_recv()
            {
                let _ = receipt.send(Ok(batch.serial));
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the world ends after the receipt");
    world_join.await.expect("world task panicked");
    assert!(
        !recorder
            .sends
            .lock()
            .expect("recording gateway poisoned")
            .iter()
            .any(|(_, event)| matches!(event.as_ref(), OutputEvent::Sys(text) | OutputEvent::Narrative { text, .. }
                if text.to_plain_text().contains("drained-at-shutdown"))),
        "a line refused at shutdown never ran"
    );
}

/// **16b cut B.5, the loop's exit.** When the world's inbound closes behind
/// a held receipt it ends once the receipt lands, and whatever still waits
/// in the queue is answered rather than dropped: a line hears the
/// retryable `CommitPending` refusal, a barrier `Unavailable`.
#[tokio::test]
async fn what_still_waits_when_the_loop_ends_is_answered_not_dropped() {
    let mut rig = held_receipt_rig().await;
    rig.line(1, "@create receipt-anchor").await;
    let (serial, receipt) = rig.next_write().await;

    rig.line(1, "think at-exit").await;
    let (reply, mut barrier) = oneshot::channel();
    rig.handle
        .tx
        .send(WorldMsg::Barrier { reply })
        .await
        .expect("world open");
    let metrics = Arc::clone(&rig.metrics);
    wait_until("the line and the barrier behind it to wait", || {
        let snapshot = metrics.snapshot();
        snapshot.command_deferred_total == 1 && snapshot.commit_pending_barriers == 1
    })
    .await;
    rig.nothing_timed_out_yet();

    // Every sender goes: the world sees its inbound close with a commit
    // pending and waits for the receipt before it ends.
    let HeldReceiptRig {
        handle,
        mut writer_rx,
        world_join,
        recorder,
        ..
    } = rig;
    drop(handle);
    // Doc 18 cut IC14 row 49 (lead ruling L29, test-side): let the world see
    // its inbound close BEFORE the receipt exists. The test runs on tokio's
    // current-thread runtime, and with the close and the receipt both ready
    // the loop's `select!` picks either first: about one run in fifteen
    // (2 of 30 measured) it took the receipt first, completed the commit with
    // its inbound still open, and ran the queued line as an ordinary drain —
    // a legitimate path, but not the one this test proves. Yielding hands the
    // world its turns while only the close is ready, so its `Wake::Closed` arm
    // marks the pending commit `exit_after_ack` first; what the test asserts is
    // unchanged.
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    assert!(!world_join.is_finished(), "the world waits for the receipt");
    receipt
        .send(Ok(serial))
        .expect("world still owns the exact receipt");
    tokio::time::timeout(Duration::from_secs(5), async {
        while !world_join.is_finished() {
            if let Ok(StoreJob::Write {
                batch,
                notify: Some(receipt),
            }) = writer_rx.try_recv()
            {
                let _ = receipt.send(Ok(batch.serial));
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the world ends after the receipt");
    world_join.await.expect("world task panicked");
    assert_eq!(
        barrier.try_recv(),
        Ok(Err(WorldReadError::Unavailable)),
        "a queued barrier hears that the world is gone"
    );
    let heard: Vec<String> = recorder
        .sends
        .lock()
        .expect("recording gateway poisoned")
        .iter()
        .filter_map(|(_, event)| match event.as_ref() {
            OutputEvent::Sys(text) | OutputEvent::Narrative { text, .. } => {
                Some(text.to_plain_text())
            }
            _ => None,
        })
        .collect();
    assert!(
        heard
            .iter()
            .any(|line| line.contains("committing another action")),
        "the queued line is refused, not dropped: {heard:?}"
    );
    assert!(
        !heard.iter().any(|line| line.contains("at-exit")),
        "{heard:?}"
    );
}

/// **16b cut B.5, the barrier at the cap.** The queue is only ever
/// non-empty behind a pending commit, so a barrier that meets a full queue
/// is refused `Busy` — the retryable overload answer — never answered `Ok`
/// ahead of the 64 entries and the receipt they wait on.
#[tokio::test]
async fn a_barrier_that_meets_a_full_defer_queue_is_refused_busy() {
    let mut rig = held_receipt_rig().await;
    rig.line(1, "@create receipt-anchor").await;
    let (serial, receipt) = rig.next_write().await;

    let reads: Vec<_> = (0..COMMAND_DEFER_CAP)
        .map(|n| {
            let handle = rig.handle.clone();
            tokio::spawn(async move { handle.name_free(&format!("free-{n}")).await })
        })
        .collect();
    let metrics = Arc::clone(&rig.metrics);
    let cap = u64::try_from(COMMAND_DEFER_CAP).expect("small");
    wait_until("the queue to fill", || {
        metrics.snapshot().command_deferred_total == cap
    })
    .await;
    let refused = tokio::time::timeout(Duration::from_secs(2), rig.handle.barrier_result())
        .await
        .expect("a barrier at the cap is answered at once");
    rig.nothing_timed_out_yet();
    assert_eq!(refused, Err(WorldReadError::Busy));
    assert_eq!(
        rig.metrics.snapshot().commit_pending_barrier_refused_total,
        1
    );

    receipt
        .send(Ok(serial))
        .expect("world still owns the exact receipt");
    for read in reads {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), read)
                .await
                .expect("each queued read is answered")
                .expect("read task"),
            Ok(true)
        );
    }
    rig.shut_down().await;
}

/// The pending-receipt custody guarantees of K0 hold with the defer queue in
/// place (16b cut B.5 retitled and rewrote this: until then a command behind
/// a held receipt was refused at once). A command typed behind the receipt
/// waits in the queue; a thousand full reattach rosters behind it still
/// coalesce into one pending snapshot, so the queue never head-of-line
/// blocks operational traffic; the barrier sent after the command waits
/// behind it; and once the receipt lands the command is SERVED — its own
/// write reaches the writer — and the barrier completes only after that
/// second receipt.
#[tokio::test]
async fn a_deferred_command_keeps_pending_reattach_live_and_holds_the_barrier_behind_it() {
    let mut rig = held_receipt_rig().await;
    rig.line(1, "@create receipt-anchor").await;
    let (serial, receipt) = rig.next_write().await;
    let metrics = Arc::clone(&rig.metrics);
    let (baseline_sends, baseline_effects) = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let snapshot = metrics.snapshot();
            if snapshot.commit_pending_sends > 0 {
                break (
                    snapshot.commit_pending_sends,
                    snapshot.commit_pending_effects,
                );
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("world must publish initial exact-receipt custody");

    // A thousand full reattach rosters coalesce into one pending snapshot.
    // All but the last go BEFORE the command, so the command's 250 ms bound
    // (wall clock) covers only one reattach, the barrier and the poll —
    // never a thousand loop turns on a starved runner.
    let reattach = |conn: u64| {
        let handle = rig.handle.clone();
        async move {
            assert!(
                handle
                    .reattach(vec![ReAttachRec {
                        output_brackets: Default::default(),
                        conn,
                        actor: GOD,
                        caps_structured: false,
                    }])
                    .await
            );
        }
    };
    for conn in 10_000..10_999_u64 {
        reattach(conn).await;
    }
    // Retryable input, not accepted semantic custody: it waits in the
    // bounded queue and leaves the FIFO available for the operational
    // messages behind it — the last reattach and the barrier.
    rig.line(2, "@create must-not-exist").await;
    reattach(10_999).await;
    let barrier = tokio::spawn({
        let handle = rig.handle.clone();
        async move { handle.barrier().await }
    });

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let snapshot = metrics.snapshot();
            if snapshot.commit_pending_barriers == 1
                && snapshot.commit_pending_reattach_records == 1
                && snapshot.commit_pending_reattach_coalesced_total == 999
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("operational traffic must remain live behind a deferred command");
    assert!(
        !barrier.is_finished(),
        "barrier must still wait for the receipt"
    );
    assert_eq!(metrics.snapshot().commit_pending_sends, baseline_sends);
    assert_eq!(metrics.snapshot().commit_pending_effects, baseline_effects);
    assert_eq!(metrics.snapshot().command_deferred_total, 1);
    rig.nothing_timed_out_yet();

    receipt
        .send(Ok(serial))
        .expect("world still owns the exact receipt");
    // The deferred command is served: its own write reaches the writer,
    // and the barrier sent after it waits for that write's receipt too.
    let (second_serial, second_receipt) = rig.next_write().await;
    tokio::task::yield_now().await;
    assert!(
        !barrier.is_finished(),
        "the barrier overtook the deferred command's own commit"
    );
    second_receipt
        .send(Ok(second_serial))
        .expect("world still owns the second receipt");
    assert!(
        tokio::time::timeout(Duration::from_secs(2), barrier)
            .await
            .expect("barrier must complete after the second receipt")
            .expect("barrier task")
    );
    // The command ran and its object exists (`name_free` probes the player
    // index, which an `@create`d thing never joins, so the creation line —
    // released only after the second receipt — is the witness).
    assert!(
        rig.heard_any("Created: must-not-exist"),
        "the deferred command was served: {:?}",
        rig.heard()
    );
    assert!(
        !rig.heard_any("committing another action") && !rig.heard_any("busy too long"),
        "the deferred command was served, never refused: {:?}",
        rig.heard()
    );
    assert_eq!(metrics.snapshot().commit_pending_barriers, 0);
    assert_eq!(metrics.snapshot().commit_pending_reattach_records, 0);
    rig.shut_down().await;
}

#[tokio::test]
async fn authoritative_abort_launches_no_external_effect_even_if_best_effort() {
    let (tx, mut rx) = mpsc::channel(1);
    let weak = tx.downgrade();
    let (_receipt_tx, receipt) = oneshot::channel();
    let pending = PendingCommit {
        token: CommitToken::from_serial(7),
        receipt: PendingReceipt::Legacy {
            expected_serial: 7,
            receiver: receipt,
        },
        sends: Vec::new(),
        effects: vec![(
            DeliveryClass::BestEffort,
            Effect::SlowTest {
                actor: GOD,
                correlation: 77,
                seconds: 0,
            },
        )],
        failure_actor: None,
        create: None,
        action: None,
        prepared_installation: None,
        barriers: Vec::new(),
        coalesced_reattach: None,
        exit_after_ack: false,
    };
    let mut engine = Engine::new(World::new_minimal(), base_registry());
    let gateway: Gateway = Arc::new(BlackholeGateway);
    let mut health = StoreHealth::new();
    let persistence_integrity = PersistenceIntegrityLatch::new();
    complete_pending_commit(
        pending,
        Err(AuthoritativeCommitFailure::Integrity(
            "deterministic rejection".to_owned(),
        )),
        &mut engine,
        &gateway,
        &mut health,
        &persistence_integrity,
        &weak,
        None,
        &test_effect_store(),
    );
    tokio::task::yield_now().await;
    assert!(health.mode.integrity_frozen());
    assert!(
        persistence_integrity.is_latched(),
        "a rejected exact receipt must publish the process-wide snapshot barrier"
    );
    assert!(
        matches!(rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
        "an aborted dirty step must not launch even a BestEffort external effect"
    );
}

#[test]
fn integrity_freeze_never_uses_command_text_or_context_allowlist() {
    let frozen = WorldMode::IntegrityFrozen {
        incident: IncidentId(12),
    };
    let mut world = World::new_minimal();
    for line in ["look", "WHO", "say hi", "@shutdown", ""] {
        assert!(matches!(
            admit_world_msg(frozen, &world, &WorldMsg::Inbound(cmd(line))),
            Admission::Refuse(RefusalReason::IntegrityFrozen)
        ));
    }
    world.push_input_context(GOD, None, Box::new(Sink));
    assert!(matches!(
        admit_world_msg(frozen, &world, &WorldMsg::Inbound(cmd("~q"))),
        Admission::Refuse(RefusalReason::IntegrityFrozen)
    ));
}

/// The conservative allowlist really is a pin: module verbs — comsys
/// aliases, `@mail`, `+jobs` — mutate module state (seq bumps, boxes,
/// cursors) and MUST stay refused while read-only. If a future
/// allowlist edit lets one through, this fails.
#[test]
fn read_only_refuses_module_verbs() {
    let world = World::new_minimal();
    for line in [
        "pub Hello there",
        "@mail/quick Nick=hi/there",
        "@mail",
        "addcom pub=Public",
        "comlist",
        "+request T=text",
        "+job/approve 1",
        "@channel/recall pub",
    ] {
        assert!(
            refuse_store_read_only(&world, &cmd(line)).is_some(),
            "{line:?} must be refused while read-only"
        );
    }
    for line in ["look", "WHO", "say hi", "\"hi", ":waves", "help mail"] {
        assert!(
            refuse_store_read_only(&world, &cmd(line)).is_none(),
            "{line:?} must pass while read-only"
        );
    }
}

struct Sink;

impl InputContext for Sink {
    fn prompt(&self, _w: &World) -> PromptSpec {
        PromptSpec {
            mode: InputMode::Normal,
            text: None,
        }
    }

    fn feed(&mut self, _ctx: &mut Ctx<'_>, _line: &str) -> ContextOutcome {
        ContextOutcome::Consumed
    }
}

/// A player inside an input context (mail compose) must never be
/// trapped by read-only mode: every line — the hard escape `~q`, the
/// send terminator `--`, and arbitrary body text that happens to spell
/// a refused verb — passes the gate and feeds the context.
#[test]
fn read_only_passes_input_context_lines() {
    let mut world = World::new_minimal();
    world.push_input_context(GOD, None, Box::new(Sink));
    for line in ["~q", "--", "body text", "@create orb", "pub Hello", "l"] {
        assert!(
            refuse_store_read_only(&world, &cmd(line)).is_none(),
            "{line:?} must pass while composing"
        );
    }
    // Popped: the gate applies again.
    world.pop_input_context(GOD);
    assert!(refuse_store_read_only(&world, &cmd("@create orb")).is_some());
}

// ------------------------------------------------------------------
// M11 per-observer projection substrate (D1/D2/D3)
// ------------------------------------------------------------------

/// A dummy `Perceive` hook that conceals exactly one `(observer, subject)`
/// pair — the test double proving the game-agnostic projection substrate
/// works before any ruleset verdict (Obfuscate/Auspex) exists.
struct CloakPair {
    observer: ObjId,
    subject: ObjId,
}

impl mush_core::module::EventHook for CloakPair {
    fn perceives(
        &self,
        _ctx: &mush_core::HookReadContext<'_>,
        observer: ObjId,
        subject: ObjId,
    ) -> mush_core::module::Perceive {
        if observer == self.observer && subject == self.subject {
            mush_core::module::Perceive::Conceal
        } else {
            mush_core::module::Perceive::Abstain
        }
    }
}

/// The plain text of the `Contents:` line `examine` sent to `actor`
/// (empty when `examine` emitted none — i.e. every occupant was hidden).
fn examine_contents(out: &Outbox, actor: ObjId) -> String {
    out.events()
        .iter()
        .find_map(|(aud, ev)| match (aud, ev) {
            (Audience::Actor(a), OutputEvent::Sys(t)) if *a == actor => {
                let s = t.to_plain_text();
                s.starts_with("Contents:").then_some(s)
            }
            _ => None,
        })
        .unwrap_or_default()
}

/// With a `Perceive` hook concealing (Alice, Wraith): the subject is
/// absent from Alice's `room_view`, `examine`, and the resolved
/// `Audience::Room` fan-out — while Bob, a different observer, perceives
/// Wraith through every one of those paths.
#[test]
fn perceive_substrate_gates_look_examine_and_fanout() {
    use mush_core::module::{HookPoint, Registrar};
    use mush_core::{dispatch, obj_ref, register_core_commands, room_view};
    use mush_proto::{NarrativeKind, RichText};

    let mut world = World::new_minimal();
    let hall = world.create(ObjKind::Room, "Hall", GOD, None).unwrap();
    let alice = world
        .create(ObjKind::Player, "Alice", GOD, Some(hall))
        .unwrap();
    let bob = world
        .create(ObjKind::Player, "Bob", GOD, Some(hall))
        .unwrap();
    let wraith = world
        .create(ObjKind::Player, "Wraith", GOD, Some(hall))
        .unwrap();
    for p in [alice, bob, wraith] {
        world.set_connected(p, true);
    }
    // Alice and Bob must be able to `examine` the room (Wizard).
    world.set_flag(alice, Flags::WIZARD, true).unwrap();
    world.set_flag(bob, Flags::WIZARD, true).unwrap();

    let mut reg = Registry::new();
    register_core_commands(&mut reg).unwrap();
    Registrar::new(&mut reg, "cloaktest").hook(
        HookPoint::Perceive,
        Arc::new(CloakPair {
            observer: alice,
            subject: wraith,
        }),
    );

    // D1: the gate itself. Only the one concealed pair fails.
    assert!(!can_perceive(&reg, &world, alice, wraith, DETACHED));
    assert!(can_perceive(&reg, &world, bob, wraith, DETACHED));
    assert!(can_perceive(&reg, &world, alice, bob, DETACHED));
    assert!(can_perceive(&reg, &world, wraith, wraith, DETACHED));

    // D2(a): room_view drops the concealed occupant for Alice only.
    let names = |looker| -> Vec<String> {
        room_view(&reg, &world, looker, hall, DETACHED)
            .unwrap()
            .contents
            .into_iter()
            .map(|r| r.name)
            .collect()
    };
    let alice_sees = names(alice);
    assert!(
        !alice_sees.contains(&"Wraith".to_string()),
        "look leaked Wraith to Alice: {alice_sees:?}"
    );
    assert!(alice_sees.contains(&"Bob".to_string()));
    assert!(
        names(bob).contains(&"Wraith".to_string()),
        "Bob must still perceive Wraith by look"
    );

    // D2(c): examine dumps raw contents — it too must drop Wraith for
    // Alice while still listing him for Bob.
    let mut out = Outbox::new();
    dispatch(
        &reg,
        &mut world,
        alice,
        &format!("examine {hall}"),
        &mut out,
        DETACHED,
    );
    let alice_ex = examine_contents(&out, alice);
    assert!(
        !alice_ex.contains("Wraith"),
        "examine leaked Wraith to Alice: {alice_ex:?}"
    );
    assert!(
        alice_ex.contains("Bob"),
        "examine still lists Bob: {alice_ex:?}"
    );
    let mut out = Outbox::new();
    dispatch(
        &reg,
        &mut world,
        bob,
        &format!("examine {hall}"),
        &mut out,
        DETACHED,
    );
    assert!(
        examine_contents(&out, bob).contains("Wraith"),
        "Bob's examine must still show Wraith"
    );

    // D2(b)+D3: the live Room fan-out. Wraith speaks; the event's subject
    // is Wraith, so Alice (who cannot perceive him) is dropped from the
    // resolved recipients while Bob and Wraith himself receive it.
    let mut out = Outbox::new();
    dispatch(&reg, &mut world, wraith, "say boo", &mut out, DETACHED);
    let sends = resolve_sends(&reg, &world, &mut out);
    let recipients: std::collections::HashSet<ObjId> = sends
        .iter()
        .filter_map(|(t, _)| match t {
            SendTarget::Char(ck) => Some(ObjId(u32::try_from(*ck).unwrap_or(u32::MAX))),
            _ => None,
        })
        .collect();
    assert!(
        !recipients.contains(&alice),
        "cloaked Wraith's speech must not reach Alice: {recipients:?}"
    );
    assert!(recipients.contains(&bob), "Bob overhears Wraith");
    assert!(recipients.contains(&wraith), "Wraith hears his own speech");

    // D3: event_subject extracts the attributed actor and abstains on
    // events that name no one.
    assert_eq!(
        event_subject(&OutputEvent::Narrative {
            kind: NarrativeKind::Say,
            actor: Some(obj_ref(&world, wraith)),
            text: RichText::plain("hi"),
        }),
        Some(wraith)
    );
    assert_eq!(
        event_subject(&OutputEvent::Narrative {
            kind: NarrativeKind::Emit,
            actor: None,
            text: RichText::plain("mist swirls"),
        }),
        None
    );
    assert_eq!(
        event_subject(&OutputEvent::Sys(RichText::plain("weather"))),
        None
    );
}

/// Invariant I1: with NO `Perceive` hook registered the fan-out is
/// byte-for-byte the old behavior — every connected, non-excluded,
/// non-ignoring occupant receives a room event, subject or not.
#[test]
fn perceive_substrate_is_a_free_pass_without_a_hook() {
    use mush_core::{dispatch, register_core_commands, room_view};

    let mut world = World::new_minimal();
    let hall = world.create(ObjKind::Room, "Hall", GOD, None).unwrap();
    let a = world
        .create(ObjKind::Player, "Ann", GOD, Some(hall))
        .unwrap();
    let b = world
        .create(ObjKind::Player, "Bee", GOD, Some(hall))
        .unwrap();
    for p in [a, b] {
        world.set_connected(p, true);
    }
    let mut reg = Registry::new();
    register_core_commands(&mut reg).unwrap();

    // Everyone is perceivable; look lists the other occupant.
    assert!(can_perceive(&reg, &world, a, b, DETACHED));
    assert!(
        room_view(&reg, &world, a, hall, DETACHED)
            .unwrap()
            .contents
            .iter()
            .any(|r| r.name == "Bee")
    );

    // A say reaches every connected occupant (speaker included).
    let mut out = Outbox::new();
    dispatch(&reg, &mut world, a, "say hi", &mut out, DETACHED);
    let recipients: std::collections::HashSet<ObjId> = resolve_sends(&reg, &world, &mut out)
        .iter()
        .filter_map(|(t, _)| match t {
            SendTarget::Char(ck) => Some(ObjId(u32::try_from(*ck).unwrap_or(u32::MAX))),
            _ => None,
        })
        .collect();
    assert!(recipients.contains(&a) && recipients.contains(&b));
}

/// `@emit` is the one verb that can put an unattributed line in front of
/// another player, so it is a granted flag (`SPOOF`, Royalty+ implicit),
/// and below Wizard the line still travels with its source: a `NOSPOOF`
/// listener and staff receive it as `Spoof` (rendered `[Name] text`),
/// everyone else as the bare `Emit`. Wizard narration is plain for all.
#[test]
fn spoofed_emits_carry_their_source_only_to_entitled_listeners() {
    use mush_core::{Flags, dispatch, register_core_commands};
    use mush_proto::NarrativeKind;

    let mut world = World::new_minimal();
    let hall = world.create(ObjKind::Room, "Hall", GOD, None).unwrap();
    let ann = world
        .create(ObjKind::Player, "Ann", GOD, Some(hall))
        .unwrap();
    let bee = world
        .create(ObjKind::Player, "Bee", GOD, Some(hall))
        .unwrap();
    let cal = world
        .create(ObjKind::Player, "Cal", GOD, Some(hall))
        .unwrap();
    let dee = world
        .create(ObjKind::Player, "Dee", GOD, Some(hall))
        .unwrap();
    for p in [ann, bee, cal, dee] {
        world.set_connected(p, true);
    }
    world.set_flag(bee, Flags::NOSPOOF, true).unwrap();
    world.set_flag(cal, Flags::ROYALTY, true).unwrap();
    world.set_flag(dee, Flags::WIZARD, true).unwrap();
    let mut reg = Registry::new();
    register_core_commands(&mut reg).unwrap();

    // The narrative kind each listener received from one dispatch.
    let narratives = |world: &World, out: &mut Outbox| -> Vec<(ObjId, NarrativeKind)> {
        resolve_sends(&reg, world, out)
            .iter()
            .filter_map(|(t, ev)| match (t, ev.as_ref()) {
                (SendTarget::Char(ck), OutputEvent::Narrative { kind, .. }) => {
                    Some((ObjId(u32::try_from(*ck).unwrap_or(u32::MAX)), *kind))
                }
                _ => None,
            })
            .collect()
    };

    // An ordinary player is refused and the room hears nothing.
    let mut out = Outbox::new();
    dispatch(
        &reg,
        &mut world,
        ann,
        "@emit A door slams.",
        &mut out,
        DETACHED,
    );
    assert!(narratives(&world, &mut out).is_empty());
    // They cannot grant themselves the licence; the observer preference
    // is theirs to set.
    let mut out = Outbox::new();
    dispatch(&reg, &mut world, ann, "@set me=SPOOF", &mut out, DETACHED);
    assert!(!world.get(ann).unwrap().flags.contains(Flags::SPOOF));
    let mut out = Outbox::new();
    dispatch(&reg, &mut world, ann, "@set me=NOSPOOF", &mut out, DETACHED);
    assert!(world.get(ann).unwrap().flags.contains(Flags::NOSPOOF));
    world.set_flag(ann, Flags::NOSPOOF, false).unwrap();

    // Granted the flag: bare for Ann herself, sourced for the NOSPOOF
    // listener and for staff.
    world.set_flag(ann, Flags::SPOOF, true).unwrap();
    let mut out = Outbox::new();
    dispatch(
        &reg,
        &mut world,
        ann,
        "@emit A door slams.",
        &mut out,
        DETACHED,
    );
    let got = narratives(&world, &mut out);
    assert_eq!(got.len(), 4, "{got:?}");
    assert!(got.contains(&(ann, NarrativeKind::Emit)));
    assert!(got.contains(&(bee, NarrativeKind::Spoof)));
    assert!(got.contains(&(cal, NarrativeKind::Spoof)));
    assert!(got.contains(&(dee, NarrativeKind::Spoof)));

    // Wizard narration is plain for everyone, NOSPOOF or not.
    let mut out = Outbox::new();
    dispatch(&reg, &mut world, dee, "@emit Thunder.", &mut out, DETACHED);
    let got = narratives(&world, &mut out);
    assert_eq!(got.len(), 4, "{got:?}");
    assert!(got.iter().all(|(_, k)| *k == NarrativeKind::Emit));
}

// ---- E7c2: B1 — the `Authoritative` ingress adapter seam ----
//
// 08j §5 (iii) verbatim: *"Settle the ingress shape before cut 2:
// `CommandHandler`/`Ctx` is synchronous and cannot reach
// `ActionCoordinator::gateway_ingress`, so the first `Authoritative`
// registration needs an adapter seam that does not exist."* 08i §2 row 5:
// *"There is no `Ctx`-level action submit."* These tests are the discharge.

fn b1_world() -> mush_testkit::TestWorld {
    let mut world = mush_testkit::TestWorld::with_modules(vec![Box::new(
        mush_contract_tests::b1_seam::B1SeamModule,
    )]);
    world
        .engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x61; 32]).unwrap(), []).unwrap());
    world
}

fn b1_run(world: &mut mush_testkit::TestWorld, line: &str) -> Outbox {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    world.engine_mut().step(
        Inbound::Cmd {
            conn: 1,
            actor: GOD,
            line: line.to_owned(),
        },
        &mut out,
        &mut fx,
    );
    out
}

fn b1_lines(out: &Outbox) -> Vec<String> {
    out.events()
        .iter()
        .map(|(_, event)| mush_testkit::text_of(event))
        .collect()
}

/// The whole point of B1: a command classified `Authoritative` reaches
/// `ActionCoordinator::prepare_and_stage_resolved`, and what it stages is an
/// authoritative bundle — never the legacy lane.
#[tokio::test]
async fn e7c2_b1_a_command_reaches_prepare_and_stage_resolved() {
    let mut world = b1_world();
    let out = b1_run(&mut world, "@b1submit 3");
    assert!(
        world.engine().has_authoritative_submit(),
        "the handler's one submission must be queued"
    );
    assert_eq!(
        b1_lines(&out),
        vec!["Queued.".to_owned()],
        "the handler itself confirms nothing; the engine acknowledges RECEIPT \
         only (E7c2 fold R11)"
    );
    let store = mush_store::InMemoryStore::new();
    let _ = world.drain_write_batch();
    let mut engine = world.into_engine();
    let mut driver = CommandSubmitDriver::new(1);
    let staged = match driver.drive_one(&mut engine, &store).await.unwrap() {
        CommandSubmitDrive::Staged { staged, actor, .. } => {
            assert_eq!(actor, GOD);
            staged
        }
        CommandSubmitDrive::Refused { detail, .. } => {
            panic!("the command submission was refused: {detail}")
        }
        _ => panic!("the command submission must stage an authoritative action"),
    };
    let (bundle, _installation) = engine
        .take_staged_action()
        .unwrap()
        .expect("staged action")
        .into_parts();
    let bundle = *bundle;
    assert_eq!(bundle.token(), staged.token);
    assert_eq!(
        bundle.action().key().id.as_str(),
        mush_contract_tests::b1_seam::B1_ACTION
    );
    assert!(!engine.has_authoritative_submit());
}

/// A handler that fans out stages **nothing at all** — not one half.
#[tokio::test]
async fn e7c2_b1_a_handler_submitting_twice_refuses_and_stages_nothing() {
    let mut world = b1_world();
    let out = b1_run(&mut world, "@b1submit/twice 3");
    assert!(
        !world.engine().has_authoritative_submit(),
        "a fan-out voids the first submission too"
    );
    let lines = b1_lines(&out);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("more than one authoritative action")),
        "{lines:?}"
    );
    let store = mush_store::InMemoryStore::new();
    let _ = world.drain_write_batch();
    let mut engine = world.into_engine();
    let mut driver = CommandSubmitDriver::new(1);
    assert!(matches!(
        driver.drive_one(&mut engine, &store).await.unwrap(),
        CommandSubmitDrive::Idle
    ));
    assert!(engine.take_staged_action().unwrap().is_none());
}

/// A submission that arrives beside legacy persistence work is **deferred,
/// not dropped** — the same rule `DurableScheduleDriver::drive_one` follows,
/// and the one `flush_prepared_action` enforces at the writer boundary.
#[tokio::test]
async fn e7c2_b1_a_submission_beside_legacy_work_is_deferred_not_dropped() {
    let mut world = b1_world();
    // Clear boot dirt first, so the only legacy work is the handler's own.
    let _ = world.drain_write_batch();
    let _ = b1_run(&mut world, "@b1submit/dirty 2");
    let store = mush_store::InMemoryStore::new();
    let mut engine = world.into_engine();
    assert!(engine.has_legacy_persistence_work());
    let mut driver = CommandSubmitDriver::new(1);
    assert!(matches!(
        driver.drive_one(&mut engine, &store).await.unwrap(),
        CommandSubmitDrive::DeferredByLegacyPredecessor
    ));
    assert!(
        engine.has_authoritative_submit(),
        "the deferred submission is retained, never dropped"
    );
    // The flush drains the legacy op; the very next drive stages it.
    let ops = engine.take_module_ops();
    assert_eq!(ops.len(), 1);
    assert!(!engine.has_legacy_persistence_work());
    assert!(matches!(
        driver.drive_one(&mut engine, &store).await.unwrap(),
        CommandSubmitDrive::Staged { .. }
    ));
}

/// Two identical `@b1submit`s inside one epoch produce **one** commit: the
/// second preflights as a HIT and its confirmation is projected from the
/// durable receipt, with no provider and no second bundle.
#[tokio::test]
async fn e7c2_b1_two_identical_submissions_in_one_epoch_produce_one_commit() {
    let mut world = b1_world();
    let _ = b1_run(&mut world, "@b1submit 4");
    let store = mush_store::InMemoryStore::new();
    let _ = world.drain_write_batch();
    let mut engine = world.into_engine();
    let mut driver = CommandSubmitDriver::new(1);
    assert!(matches!(
        driver.drive_one(&mut engine, &store).await.unwrap(),
        CommandSubmitDrive::Staged { .. }
    ));
    let (bundle, installation) = engine
        .take_staged_action()
        .unwrap()
        .expect("staged action")
        .into_parts();
    store
        .reserve_authoritative_intent(*bundle.clone())
        .await
        .unwrap();
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();
    engine
        .install_composed_action(installation)
        .expect("the fixture installs");
    store
        .checkpoint_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();

    // The identical command again, in the same epoch.
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine.step(
        Inbound::Cmd {
            conn: 1,
            actor: GOD,
            line: "@b1submit 4".to_owned(),
        },
        &mut out,
        &mut fx,
    );
    match driver.drive_one(&mut engine, &store).await.unwrap() {
        CommandSubmitDrive::AlreadyFinished { actor, receipt, .. } => {
            assert_eq!(actor, GOD);
            assert_eq!(receipt.token(), bundle.token());
        }
        CommandSubmitDrive::Refused { detail, .. } => {
            panic!("the retyped identical command must be a preflight HIT: {detail}")
        }
        CommandSubmitDrive::Staged { .. } => panic!("a HIT must not stage a second commit"),
        _ => panic!("unexpected command-ingress outcome"),
    }
    assert!(
        engine.take_staged_action().unwrap().is_none(),
        "a HIT stages nothing"
    );
}

/// A retired idempotency epoch is recoverable for the command lane exactly
/// as it is for the schedule lane: raise the watermark and retry.
#[tokio::test]
async fn e7c2_b1_a_retired_epoch_is_raised_and_the_retry_succeeds() {
    let mut world = b1_world();
    let _ = b1_run(&mut world, "@b1submit 5");
    let store = mush_store::InMemoryStore::new();
    let submit = {
        let mut probe = b1_world();
        let _ = b1_run(&mut probe, "@b1submit 5");
        let mut engine = probe.into_engine();
        *engine
            .take_authoritative_submit()
            .expect("probe submission")
    };
    let old = ActionCoordinator::command_ingress(&submit, 1).unwrap();
    store
        .retire_authoritative_epochs(old.idempotency().scope(), 9)
        .await
        .expect("seed a retired command epoch");
    let _ = world.drain_write_batch();
    let mut engine = world.into_engine();
    let mut driver = CommandSubmitDriver::new(1);
    assert!(matches!(
        driver.drive_one(&mut engine, &store).await.unwrap(),
        CommandSubmitDrive::Staged { .. }
    ));
    assert!(
        driver.idempotency_epoch >= 9,
        "the retirement watermark must be adopted"
    );
}

/// §4's standing checklist item 1 — *no command can confirm before its
/// durable acknowledgement*. The handler emits nothing; the seam's own
/// confirmation is `OnCommit`, so the delivery boundary withholds it until
/// this exact bundle's receipt resolves and suppresses it on abort.
#[tokio::test]
async fn e7c2_b1_the_confirmation_never_outruns_the_durable_ack() {
    let mut world = b1_world();
    let handler_out = b1_run(&mut world, "@b1submit 6");
    assert_eq!(
        b1_lines(&handler_out),
        vec!["Queued.".to_owned()],
        "an Authoritative handler emits no success presentation of its own; the \
         only line is the engine's BestEffort receipt acknowledgement"
    );
    assert_eq!(
        handler_out.classes(),
        [mush_core::DeliveryClass::BestEffort]
    );
    let store = mush_store::InMemoryStore::new();
    let _ = world.drain_write_batch();
    let mut engine = world.into_engine();
    let mut driver = CommandSubmitDriver::new(1);
    let CommandSubmitDrive::Staged { staged, actor, .. } =
        driver.drive_one(&mut engine, &store).await.unwrap()
    else {
        panic!("staged")
    };
    // What the world loop then queues, and its delivery class.
    let mut out = Outbox::new();
    out.info(
        actor,
        staged.summary.clone().unwrap_or_else(|| "Done.".to_owned()),
    );
    assert_eq!(out.classes(), [mush_core::DeliveryClass::OnCommit]);
}

// ---- E7 cut 2 independent verify fold ----

fn b1_run_as(world: &mut mush_testkit::TestWorld, actor: ObjId, line: &str) -> Outbox {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    world.engine_mut().step(
        Inbound::Cmd {
            conn: 1,
            actor,
            line: line.to_owned(),
        },
        &mut out,
        &mut fx,
    );
    out
}

/// `EconomyModule` + `InventoryModule` behind the real command seam, with a
/// container-backed vault and a seeded pool: the smallest world in which a
/// typed `@inv/move` can actually commit.
fn inv_world() -> (
    mush_testkit::TestWorld,
    mush_inventory::InventoryFamilies,
    u64,
) {
    let mut world = mush_testkit::TestWorld::with_modules(vec![
        Box::new(mush_economy::EconomyModule::new()),
        Box::new(mush_inventory::InventoryModule::new()),
    ]);
    world
        .engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x62; 32]).unwrap(), []).unwrap());
    let families = world
        .world()
        .module_state::<mush_inventory::InventoryHandles>()
        .expect("inventory handles")
        .0
        .clone();
    let vault = |world: &mut mush_testkit::TestWorld, name: &str| -> u64 {
        let created = world.run(GOD, &format!("@create {name}"));
        let object = created
            .iter()
            .find_map(|(_, event)| {
                let text = mush_testkit::text_of(event);
                let at = text.find("(#")?;
                text[at + 2..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse::<u64>()
                    .ok()
            })
            .expect("created object");
        let out = world.run(GOD, &format!("@vault/create {name}=container:#{object}"));
        out.iter()
            .find_map(|(_, event)| {
                let text = mush_testkit::text_of(event);
                let at = text.find("vault #")?;
                text[at + 7..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse::<u64>()
                    .ok()
            })
            .expect("vault id")
    };
    let from = vault(&mut world, "from");
    let _to = vault(&mut world, "to");
    {
        let engine = world.engine_mut();
        let mut txn = engine
            .action_transaction(&families.pools)
            .expect("inventory transaction");
        families
            .credit(
                &mut txn,
                mush_inventory::EndpointRef(mush_economy::VaultId(from)),
                &mush_core::CanonicalId::new("metals").expect("commodity"),
                100,
            )
            .expect("seed");
        let prepared = txn.prepare().expect("prepared");
        let (_, installation) = prepared.into_parts();
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");
    }
    (world, families, from)
}

fn inv_available(
    engine: &mut Engine,
    families: &mush_inventory::InventoryFamilies,
    vault: u64,
) -> u64 {
    let mut txn = engine
        .action_transaction(&families.pools)
        .expect("inventory transaction");
    families
        .available(
            &mut txn,
            mush_inventory::EndpointRef(mush_economy::VaultId(vault)),
            &mush_core::CanonicalId::new("metals").expect("commodity"),
        )
        .expect("available")
}

/// D1 as taken: the durable key derives from the request identity alone, so a
/// replay after a reboot is a **HIT** with no second bundle. The epoch is a
/// per-process random and does NOT survive a restart, and `InventoryVerb::Move`
/// derives nothing and consumes nothing — so before the fold the second
/// attempt moved the quantity twice and rewrote the same
/// `inventory.transferred` marker over it.
#[tokio::test]
async fn vf_r1_a_replayed_move_across_an_epoch_change_moves_nothing_twice() {
    let (mut world, families, from) = inv_world();
    let to = from + 1;
    let _ = world.run(GOD, &format!("@inv/move {from}/metals:10={to}"));
    let submit = *world
        .engine_mut()
        .take_authoritative_submit()
        .expect("the command queued a submission");
    let store = mush_store::InMemoryStore::new();
    let _ = world.drain_write_batch();
    let mut engine = world.into_engine();

    // Attempt one, on this process's epoch.
    let first = ActionCoordinator::command_ingress(&submit, 1).unwrap();
    let ActionPreflight::Vacant(permit) =
        ActionCoordinator::preflight(&store, first).await.unwrap()
    else {
        panic!("the first attempt is vacant")
    };
    let submission = command_offer_submission(&engine, &submit).expect("offer");
    let (staged, _) = resolve_and_stage_offer(&mut engine, permit, submission).expect("staged");
    let (bundle, installation) = engine
        .take_staged_action()
        .unwrap()
        .expect("staged action")
        .into_parts();
    assert_eq!(bundle.token(), staged.token);
    store
        .reserve_authoritative_intent(*bundle.clone())
        .await
        .unwrap();
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();
    engine
        .install_composed_action(installation)
        .expect("the fixture installs");
    store
        .checkpoint_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();
    assert_eq!(inv_available(&mut engine, &families, from), 90);

    // The reboot: a fresh process epoch. The REQUEST identity survives it and
    // the durable key does not, so preflight is vacant all over again.
    let replay = ActionCoordinator::command_ingress(&submit, 2).unwrap();
    let epoch_free = ActionCoordinator::command_ingress(&submit, 1).unwrap();
    assert_eq!(
        replay.request(),
        epoch_free.request(),
        "the request identity is epoch-free"
    );
    assert_ne!(
        replay.idempotency().epoch(),
        epoch_free.idempotency().epoch(),
        "but the durable key it is looked up under is not"
    );
    let ActionPreflight::Vacant(permit) =
        ActionCoordinator::preflight(&store, replay).await.unwrap()
    else {
        panic!("the ingress cannot catch this; the mechanic must")
    };
    let submission = command_offer_submission(&engine, &submit).expect("offer");
    let refused = resolve_and_stage_offer(&mut engine, permit, submission)
        .expect_err("the applied marker refuses the replay");
    assert!(
        refused.contains("already been applied"),
        "the marker is the replay guard: {refused}"
    );
    assert_eq!(
        inv_available(&mut engine, &families, from),
        90,
        "and the quantity moved exactly once"
    );
}

/// **Doc 18 cut IC8: the stock clerk's first mint drives through the command
/// ingress** (lead 2c-v). On the game roster with `[tacsim].enabled`, the first
/// `tacsim.port` fire queues a submission whose actor is the Ceres stock clerk — a
/// Thing, not a player and not GOD — and it lands exactly as a command's does:
/// `command_ingress` → `preflight` Vacant → `command_offer_submission` →
/// `resolve_and_stage_offer` → reserve, reconcile, install, checkpoint. The receipt
/// and the lot exist; the same ingress replayed after an epoch change is refused by
/// the receipt and mints nothing twice.
#[tokio::test]
async fn ic8_the_clerks_first_mint_drives_through_the_command_ingress() {
    let llm = Arc::new(
        mush_llm::LlmService::builder()
            .global_enabled(false)
            .build(),
    );
    let mut config = mush_config::config::Config::default();
    config.tacsim.enabled = true;
    let roster = crate::boot::game_modules(&config, &llm)
        .expect("an enabled config composes the game roster");
    let mut world = mush_testkit::TestWorld::with_modules(roster);
    world
        .engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x63; 32]).unwrap(), []).unwrap());
    let lane = mush_core::CanonicalId::new(tacsim::definitions::content::LANE_PACK).unwrap();
    let ceres = mush_core::CanonicalId::new("ceres_port").unwrap();
    let clerk = match world
        .world()
        .module_state::<tacsim::TacsimStore>()
        .expect("the layer")
        .port_map(&lane, &ceres)
    {
        tacsim::port::PortMapState::Present(map) => {
            ObjId(map.clerk.as_ref().expect("a clerk").thing)
        }
        other => panic!("the Ceres map: {other:?}"),
    };
    let t = world.world().now().saturating_add(6);
    world.set_now(t);
    let _ = world.fire_due(t.saturating_mul(1_000));
    let submit = *world
        .engine_mut()
        .take_authoritative_submit()
        .expect("the first fire queued a submission");
    assert_eq!(submit.actor, clerk, "the Ceres stock clerk's");
    assert_ne!(submit.actor, GOD);
    assert_eq!(
        world.world().get(clerk).map(|o| o.kind),
        Some(mush_core::ObjKind::Thing)
    );
    assert_eq!(submit.action.key().id.as_str(), tacsim::port::STOCK_ACTION);
    let issue = tacsim::port::StockIssueId::parse(
        submit.parameters["issue"].as_str().expect("an issue key"),
    )
    .expect("an issue id");
    let handles = world
        .world()
        .module_state::<tacsim::port::PortHandles>()
        .cloned()
        .expect("the port handles");
    let inventory = handles.inventory.clone().expect("the filled cell");
    let store = mush_store::InMemoryStore::new();
    let _ = world.drain_write_batch();
    let mut engine = world.into_engine();

    let first = ActionCoordinator::command_ingress(&submit, 1).unwrap();
    let ActionPreflight::Vacant(permit) =
        ActionCoordinator::preflight(&store, first).await.unwrap()
    else {
        panic!("the first attempt is vacant")
    };
    let submission = command_offer_submission(&engine, &submit).expect("offer");
    let (staged, _) = resolve_and_stage_offer(&mut engine, permit, submission).expect("staged");
    let (bundle, installation) = engine
        .take_staged_action()
        .unwrap()
        .expect("staged action")
        .into_parts();
    assert_eq!(bundle.token(), staged.token);
    store
        .reserve_authoritative_intent(*bundle.clone())
        .await
        .unwrap();
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();
    engine
        .install_composed_action(installation)
        .expect("the mint installs");
    store
        .checkpoint_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();
    let receipt = {
        let mut txn = engine
            .action_transaction(&handles.families.port_stock)
            .expect("a tacsim transaction");
        txn.get(&handles.families.port_stock, &issue)
            .expect("the receipts read")
            .cloned()
            .expect("the receipt exists")
    };
    assert_eq!(receipt.clerk, clerk);
    let lot = {
        let mut txn = engine
            .action_transaction(&inventory.lots)
            .expect("an inventory transaction");
        txn.get(&inventory.lots, &mush_inventory::LotId(receipt.lot))
            .expect("the lots read")
            .cloned()
    };
    assert!(lot.is_some(), "the lot exists");

    // A fresh process epoch: the ingress is vacant again, and the receipt refuses.
    let replay = ActionCoordinator::command_ingress(&submit, 2).unwrap();
    let ActionPreflight::Vacant(permit) =
        ActionCoordinator::preflight(&store, replay).await.unwrap()
    else {
        panic!("the ingress cannot catch this; the receipt must")
    };
    let submission = command_offer_submission(&engine, &submit).expect("offer");
    let refused = resolve_and_stage_offer(&mut engine, permit, submission)
        .expect_err("the receipt refuses the replay");
    assert!(refused.contains("already minted"), "{refused}");
    let mut txn = engine
        .action_transaction(&inventory.lots)
        .expect("an inventory transaction");
    assert!(
        txn.get(&inventory.lots, &mush_inventory::LotId(receipt.lot))
            .expect("the lots read")
            .is_some(),
        "minted exactly once"
    );
}

/// The other half of D1: a deliberately **retyped** intent is a new request,
/// as the player means it. The handler's `key` used to be a constant per-verb
/// discriminator, so `@inv/move 1/metals:5=2` had one request identity for
/// that actor for ever and "move five more" silently collapsed into a HIT.
#[tokio::test]
async fn vf_r1_two_deliberately_retyped_moves_move_twice() {
    let (mut world, _families, from) = inv_world();
    let to = from + 1;
    let line = format!("@inv/move {from}/metals:5={to}");
    let _ = world.run(GOD, &line);
    let first = *world
        .engine_mut()
        .take_authoritative_submit()
        .expect("first submission");
    let _ = world.run(GOD, &line);
    let second = *world
        .engine_mut()
        .take_authoritative_submit()
        .expect("second submission");
    assert_ne!(
        first.key.as_str(),
        second.key.as_str(),
        "two typed intents carry two discriminators"
    );
    let a = ActionCoordinator::command_ingress(&first, 1).unwrap();
    let b = ActionCoordinator::command_ingress(&second, 1).unwrap();
    assert_ne!(
        a.request(),
        b.request(),
        "and therefore two request identities, so both commit"
    );
    // While driving the SAME submission value twice is still one identity.
    let again = ActionCoordinator::command_ingress(&first, 1).unwrap();
    assert_eq!(a.request(), again.request());
}

// ---- E7 cut 3 independent verify fold: B2 (the @extract e2e gap) ----

/// A test-only module that seeds GOD's extraction-licence account: the
/// entitlement instrument (no `@instrument/register` builds one — it is
/// hardcoded `Currency`) and an `Active` account in it. Composed only in this
/// test's world, never in a production composition, so the allowlist tripwire
/// (which enumerates the *production* registry) never sees `@seedlicence`.
struct LicenceSetup;

fn seed_licence(ctx: &mut mush_core::Ctx<'_>) -> Result<(), mush_core::CmdError> {
    let id = mush_economy::InstrumentId::new("tacsim.extraction").expect("instrument id");
    let _ = mush_economy::register_instrument(
        ctx,
        mush_economy::InstrumentDef {
            id: id.clone(),
            ruleset: SmolStr::new_static("tacsim"),
            code: SmolStr::new("tacsim.extraction"),
            kind: mush_economy::InstrumentKind::Entitlement,
            scale: 0,
            transferable: false,
            negative_policy: mush_economy::NegativePolicy::SystemOnly,
            status: mush_economy::InstrumentStatus::Active,
            display: None,
            revision: 1,
        },
    );
    let account_id = {
        ctx.w
            .module_state_mut::<mush_economy::EconomyStore>()
            .expect("economy store")
            .alloc_account()
            .expect("an account id")
    };
    mush_economy::open_account(
        ctx,
        mush_economy::Account {
            id: account_id,
            owner: mush_org::PrincipalId::Object(ctx.actor),
            instrument: id,
            scope: None,
            name: SmolStr::new("extraction licence"),
            status: mush_economy::AccountStatus::Active,
            posted_balance: mush_economy::Amount::ZERO,
            held_amount: mush_economy::Amount::ZERO,
            last_hold_seq: 0,
            credit: mush_economy::CreditPolicy::forbid(),
            last_journal_seq: 0,
            revision: 1,
            approval: None,
        },
    );
    Ok(())
}

impl mush_core::GameModule for LicenceSetup {
    fn name(&self) -> &'static str {
        "testlicencesetup"
    }
    fn register(&mut self, reg: &mut mush_core::Registrar<'_>) -> Result<(), mush_core::ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.command(
            mush_core::CommandSpec {
                name: "@seedlicence".into(),
                switches: Vec::new(),
                shape: mush_core::ArgShape::EqPair,
                ..mush_core::CommandSpec::default()
            },
            std::sync::Arc::new(|ctx: &mut mush_core::Ctx<'_>| seed_licence(ctx)),
        )?;
        Ok(())
    }
}

/// economy + inventory(delegates "industry") + industry(licence) + the licence
/// seeder, behind the real command seam: the smallest world in which a typed
/// `@extract/cycle` can actually commit a lot.
fn extract_world() -> (mush_testkit::TestWorld, mush_industry::IndustryFamilies) {
    let cell: mush_inventory::InventoryHandleCell = std::sync::Arc::default();
    let licence = mush_economy::InstrumentId::new("tacsim.extraction").expect("id");
    let mut world = mush_testkit::TestWorld::with_modules(vec![
        Box::new(mush_economy::EconomyModule::new()),
        Box::new(
            mush_inventory::InventoryModule::new()
                .with_quantity_delegates(["industry"])
                .with_handle_cell(std::sync::Arc::clone(&cell)),
        ),
        Box::new(
            mush_industry::IndustryModule::new()
                .with_inventory(std::sync::Arc::clone(&cell))
                .with_extraction_licence(licence),
        ),
        Box::new(LicenceSetup),
    ]);
    world
        .engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x71; 32]).unwrap(), []).unwrap());
    let families = world
        .world()
        .module_state::<mush_industry::IndustryHandles>()
        .expect("industry handles")
        .0
        .clone();
    (world, families)
}

fn apply_industry<T>(
    engine: &mut Engine,
    families: &mush_industry::IndustryFamilies,
    body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> Result<T, mush_industry::IndustryError>,
) -> Result<T, mush_industry::IndustryError> {
    let (result, prepared) = {
        let mut txn = engine
            .action_transaction(&families.deposits)
            .expect("industry transaction");
        let result = body(&mut txn);
        match result {
            Ok(value) => (Ok(value), txn.prepare().ok()),
            Err(error) => (Err(error), None),
        }
    };
    if let Some(prepared) = prepared {
        let (_, installation) = prepared.into_parts();
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");
    }
    result
}

fn make_gods_vault(world: &mut mush_testkit::TestWorld) -> u64 {
    let created = world.run(GOD, "@create facility");
    let object = created
        .iter()
        .find_map(|(_, event)| {
            let text = mush_testkit::text_of(event);
            let at = text.find("(#")?;
            text[at + 2..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse::<u64>()
                .ok()
        })
        .expect("created object");
    let out = world.run(GOD, &format!("@vault/create facility=container:#{object}"));
    out.iter()
        .find_map(|(_, event)| {
            let text = mush_testkit::text_of(event);
            let at = text.find("vault #")?;
            text[at + 7..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse::<u64>()
                .ok()
        })
        .expect("vault id")
}

/// Drive one queued authoritative submission through the coordinator to a
/// durable checkpoint (or to the refusal `resolve_and_stage_offer` raises).
async fn drive_submit(
    world: &mut mush_testkit::TestWorld,
    store: &mush_store::InMemoryStore,
    submit: &mush_core::AuthoritativeSubmit,
    epoch: u64,
) -> Result<String, String> {
    let ingress = ActionCoordinator::command_ingress(submit, epoch).unwrap();
    let permit = match ActionCoordinator::preflight(store, ingress).await.unwrap() {
        ActionPreflight::Vacant(permit) => permit,
        _ => return Err("preflight was not vacant".to_owned()),
    };
    let engine = world.engine_mut();
    let submission = command_offer_submission(engine, submit)?;
    let (staged, _) = resolve_and_stage_offer(engine, permit, submission)?;
    let summary = staged.summary.as_deref().unwrap_or_default().to_owned();
    let (bundle, installation) = engine
        .take_staged_action()
        .unwrap()
        .expect("a staged action")
        .into_parts();
    store
        .reserve_authoritative_intent(*bundle.clone())
        .await
        .unwrap();
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();
    engine
        .install_composed_action(installation)
        .expect("the fixture installs");
    store
        .checkpoint_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();
    Ok(summary)
}

// ---- E7 cut 5 independent verify fold: the driver-level C1 e2e ----

/// A fixed character->account oracle for the sim focus seam.
struct FoldMapOracle(std::collections::BTreeMap<ObjId, mush_core::AccountId>);
impl mush_core::AccountIdentity for FoldMapOracle {
    fn account_of(&self, character: ObjId) -> Option<mush_core::AccountId> {
        self.0.get(&character).copied()
    }
}

/// The `(unit, station)` lease post-image carried in an applied bundle's
/// replay writes (the driver never re-reads the internal family handle).
fn bundle_lease(bundle: &AuthoritativeBundle) -> mush_core::ControlLeaseState {
    let value = bundle
        .writes()
        .iter()
        .find_map(|write| match write {
            mush_core::ReplayWrite::ModuleJsonSet { key, value, .. }
                if key.as_str().contains("control_leases") =>
            {
                Some(value.clone())
            }
            _ => None,
        })
        .expect("the bundle carries a control-lease post-image");
    let state = value.get("value").cloned().unwrap_or(value);
    serde_json::from_value(state).expect("a valid control-lease post-image")
}

/// Discover, prepare and COMMIT the one due scheduled control-handoff apply,
/// returning its bundle so the seated lease can be read.
async fn drive_scheduled_apply(
    world: &mut mush_testkit::TestWorld,
    store: &mush_store::InMemoryStore,
    at: i64,
) -> AuthoritativeBundle {
    world.world_mut().set_now(at);
    let candidate = world
        .engine()
        .discover_scheduled_actions(mush_core::ScheduleDriveCause::Live, None, 32)
        .expect("discovery")
        .candidates()[0]
        .clone();
    let ingress =
        ActionCoordinator::scheduled_ingress(candidate.action(), candidate.advance(), 1).unwrap();
    let permit = match ActionCoordinator::preflight(store, ingress).await.unwrap() {
        ActionPreflight::Vacant(permit) => permit,
        _ => panic!("scheduled apply preflight was not vacant"),
    };
    ActionCoordinator::prepare_and_stage_scheduled(permit, world.engine_mut(), &candidate)
        .expect("the apply stages");
    let (bundle, installation) = world
        .engine_mut()
        .take_staged_action()
        .unwrap()
        .expect("a staged apply")
        .into_parts();
    store
        .reserve_authoritative_intent(*bundle.clone())
        .await
        .unwrap();
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();
    world
        .engine_mut()
        .install_composed_action(installation)
        .expect("the fixture installs");
    store
        .checkpoint_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();
    *bundle
}

/// C1 (HIGH), the "does the fallback actually seat" e2e the shipped H tests
/// never wrote: a clean `+focus/release` fires the declared fallback, the
/// real `command_offer_submission` + `resolve_and_stage_offer` resolve the
/// deferred submit against the re-sourced relief offer (it is no longer
/// self-staled by its own release), and the durable scheduled apply seats the
/// lease on the declared fallback.
#[tokio::test]
async fn vf_c1_a_clean_release_fallback_actually_seats() {
    use mush_core::ObjKind;

    let mut world =
        mush_testkit::TestWorld::with_modules(vec![Box::new(mush_sim::SimulationModule::new())]);
    world
        .engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(3, [3; 32]).unwrap(), []).unwrap());
    let field = world
        .world_mut()
        .create(ObjKind::Room, "Field", GOD, None)
        .unwrap();
    let atlas = world
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, Some(LIMBO))
        .unwrap();
    let pilot = world.spawn_player("Pilot");
    let backup = world.spawn_player("Backup");
    for (obj, beh) in [(field, "sim.domain"), (atlas, "sim.participant")] {
        world
            .world_mut()
            .attach_behavior(
                obj,
                mush_core::BehaviorId::new(beh).unwrap(),
                mush_core::BehaviorBinding {
                    schema_version: 1,
                    attached_by: GOD,
                    attached_at: 0,
                },
            )
            .unwrap();
    }
    *world
        .world_mut()
        .module_state_mut::<mush_sim::AccountOracle>()
        .unwrap() =
        mush_sim::AccountOracle::new(Arc::new(FoldMapOracle(std::collections::BTreeMap::from([
            (pilot, mush_core::AccountId::mint("PilotAcct", 1).unwrap()),
        ]))));
    let refused = |events: &[(mush_core::Audience, OutputEvent)]| {
        events.iter().any(|(_, event)| {
            matches!(event, OutputEvent::Sys(rich)
                if rich.spans().iter().any(|span| span.tag.as_deref() == Some("error")))
        })
    };
    for line in [
        format!("@domain/create #{}=sim.passive", field.0),
        format!("@spatial/place/domain #{}=0", atlas.0),
        format!(
            "@spatial/station #{}=pilot,pilot,channels:movement",
            atlas.0
        ),
        format!("@spatial/assign #{}=pilot,#{}", atlas.0, pilot.0),
    ] {
        assert!(
            !refused(&world.run(GOD, &line)),
            "setup line refused: {line}"
        );
    }
    world.world_mut().move_to(pilot, field).unwrap();
    world.world_mut().move_to(backup, field).unwrap();
    let _ = world.run(
        GOD,
        &format!("@station/fallback #{}/pilot=#{}", atlas.0, backup.0),
    );
    let _ = world.run(pilot, &format!("+focus #{}/pilot", atlas.0));
    let _ = world.run(pilot, "+focus/release");
    let fire = *world
        .engine_mut()
        .take_authoritative_submit()
        .expect("the clean release fired the fallback controller");

    let store = mush_store::InMemoryStore::new();
    let _ = world.drain_write_batch();
    let boundary_at = world.world().world_ic_now().at;
    // Resolve + stage the fallback fire through the REAL command-submit path.
    drive_submit(&mut world, &store, &fire, 1)
        .await
        .expect("the fallback fire resolves and stages (not self-staled)");
    // Apply the scheduled handoff at its boundary: the seat FILLS with backup.
    let applied = bundle_lease(&drive_scheduled_apply(&mut world, &store, boundary_at).await);
    assert_eq!(
        applied.holder(),
        Some(&PrincipalId::Object { id: backup }),
        "the declared fallback actually seats through the full driver path"
    );
    assert!(applied.pending().is_none());
}

#[tokio::test]
async fn vf_b2_extract_command_to_receipt_commits_one_lot() {
    let (mut world, families) = extract_world();
    let store = mush_store::InMemoryStore::new();
    // GOD's extraction licence (entitlement instrument + Active account).
    let _ = world.run(GOD, "@seedlicence");
    // GOD's own facility endpoint, and a body whose pool is seeded directly.
    let facility = make_gods_vault(&mut world);
    let body = mush_industry::OreDeposit::new(
        mush_industry::OreDepositId(0x0E7C3),
        ObjId(7),
        CanonicalId::new("metals").unwrap(),
        mush_inventory::Grade::new(60).unwrap(),
        mush_industry::Accessibility::new(100).unwrap(),
        mush_industry::BandId::new("grade_b").unwrap(),
        mush_industry::BandId::new("reserve_major").unwrap(),
        mush_industry::YieldCurve::new(vec![mush_industry::CurvePoint {
            cumulative: 0,
            per_cycle: 500,
        }])
        .unwrap(),
        mush_inventory::EndpointRef(mush_economy::VaultId(90_001)),
        40_000,
    )
    .unwrap();
    let now = world.world().world_ic_now();
    let claim = mush_industry::ClaimRecord::new(
        mush_industry::ClaimId(0x0C1A),
        mush_industry::ClaimSubject::Deposit { deposit: body.id() },
        mush_org::PrincipalId::Object(GOD),
        mush_org::PrincipalId::Object(GOD),
        ObjId(90_099),
        mush_industry::Exclusivity::Shared,
        1,
        mush_core::ClockStamp {
            clock: now.clock,
            at: now.at.saturating_add(1_000_000),
        },
        mush_industry::Transferability::Transferable,
    )
    .unwrap();
    {
        let engine = world.engine_mut();
        apply_industry(engine, &families, |txn| {
            families.open_deposit(txn, body.clone())
        })
        .expect("open the body");
        apply_industry(engine, &families, |txn| {
            families.file_claim(txn, claim.clone())
        })
        .expect("file the claim");
    }

    // @extract/start <body>=<facility>, driven to commit.
    let _ = world.run(GOD, &format!("@extract/start {}={}", body.id(), facility));
    let start_submit = *world
        .engine_mut()
        .take_authoritative_submit()
        .expect("@extract/start queued a submission");
    let _ = world.drain_write_batch();
    drive_submit(&mut world, &store, &start_submit, 1)
        .await
        .expect("the start commits");

    // @extract/cycle <body>, driven to commit.
    let _ = world.run(GOD, &format!("@extract/cycle {}", body.id()));
    let cycle_submit = *world
        .engine_mut()
        .take_authoritative_submit()
        .expect("@extract/cycle queued a submission");
    let _ = world.drain_write_batch();
    let summary = drive_submit(&mut world, &store, &cycle_submit, 1)
        .await
        .expect("the cycle commits");
    assert!(
        summary.contains("produced 500"),
        "the receipt names the yield: {summary}"
    );

    // Exactly one lot minted, at the OPERATOR's endpoint, and the run advanced.
    {
        let engine = world.engine_mut();
        let markers: Vec<mush_industry::AppliedMarker> = engine
            .action_state()
            .scan_family(&families.applied, None, 64)
            .expect("scan markers")
            .entries()
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        assert_eq!(
            markers.len(),
            1,
            "the committed bundle wrote exactly one marker"
        );
        assert_eq!(markers[0].deposit(), body.id());
        assert_eq!(markers[0].quantity(), 500);
        assert_eq!(
            markers[0].to(),
            mush_inventory::EndpointRef(mush_economy::VaultId(facility)),
            "the lot landed at the operator's own endpoint"
        );
        assert_eq!(markers[0].actor(), GOD);
        let run = engine
            .action_state()
            .get(&families.runs, &body.id())
            .expect("read run")
            .cloned()
            .expect("the run exists");
        assert_eq!(
            run.next_cycle(),
            1,
            "industry.run advanced exactly one cycle"
        );
    }

    // A retyped-identical submission is a marker HIT: a reboot (fresh epoch)
    // gets past the durable idempotency key, and the applied marker refuses
    // the replay, so nothing commits twice.
    let _ = world.drain_write_batch();
    let replay = drive_submit(&mut world, &store, &cycle_submit, 2).await;
    assert!(
        replay
            .as_ref()
            .err()
            .is_some_and(|error| error.contains("already been applied")),
        "the applied marker is the replay guard: {replay:?}"
    );
    // And still exactly one marker, the run still at cycle 1.
    {
        let engine = world.engine_mut();
        let count = engine
            .action_state()
            .scan_family(&families.applied, None, 64)
            .expect("scan markers")
            .entries()
            .len();
        assert_eq!(count, 1, "the replay committed no second marker");
        let run = engine
            .action_state()
            .get(&families.runs, &body.id())
            .expect("read run")
            .cloned()
            .expect("run");
        assert_eq!(run.next_cycle(), 1, "the run did not advance on the replay");
    }
}

// ---- E7c4 independent verify fold: R1 (the terminal-schedule freeze) ----

/// Before the fold, a completed/cancelled work order left its repeating
/// production `DurableSchedule` live; the next cadence tick discovered a
/// candidate, `wo_advance` refused it with a **non-retryable**
/// `ProviderRejected`, and the durable-schedule driver turned that into
/// `apply_integrity_freeze` — a whole-world freeze that a reboot re-triggered.
/// The terminal transition now tears the schedule down, and the adapter's
/// `plan` refuses a terminal order's surviving schedule as **Stale** — which
/// the driver DISCARDS. This asserts the classification the freeze fired on no
/// longer arises: planning a terminal order's orphan schedule is a Stale plan
/// refusal, not a hard prepare error.
#[test]
fn vf_r1_a_terminal_schedule_does_not_freeze_the_world() {
    let (mut world, families) = extract_world();
    let engine = world.engine_mut();
    let order = mush_industry::WorkOrderId(0x1234);
    let owner = mush_core::OwnerId::new("industry").unwrap();
    let local = CanonicalId::new(format!("production.{:016x}", order.0)).unwrap();
    let sid = mush_core::ScheduleId::derive(&owner, &local).unwrap();
    let ic = engine.world.world_ic_now();
    let sch = mush_core::DurableSchedule::new(mush_core::DurableScheduleDraft {
        id: sid,
        owner: owner.clone(),
        clock: ic.clock.clone(),
        next_due: mush_core::ClockStamp {
            clock: ic.clock.clone(),
            at: ic.at.saturating_sub(1),
        },
        cadence: Some(mush_industry::workorder::PRODUCTION_CADENCE_SECS),
        generation: 1,
        next_ordinal: 0,
        restart: mush_core::ScheduleRestartPolicy::Resume,
        catch_up_order: mush_core::ScheduleCatchUpOrder::OldestFirst,
        max_catch_up: mush_industry::workorder::PRODUCTION_MAX_CATCH_UP,
    })
    .unwrap();
    let job = mush_industry::ProductionJob::new(
        sid,
        order,
        GOD,
        mush_industry::workorder::PRODUCTION_UNITS_PER_OCCURRENCE,
        1,
    )
    .unwrap();
    let recipe = mush_industry::RecipeRef::new(
        CanonicalId::new("tacsim.core").unwrap(),
        1,
        CanonicalId::new("alloy_smelt").unwrap(),
        mush_core::DefinitionDigest::from_bytes([0x11; 32]),
    )
    .unwrap();
    let pinned = mush_industry::PinnedRecipe::new(
        vec![mush_industry::RecipeLine::new(CanonicalId::new("metals").unwrap(), 4).unwrap()],
        vec![
            mush_industry::RecipeLine::new(CanonicalId::new("structural_alloys").unwrap(), 1)
                .unwrap(),
        ],
        std::collections::BTreeSet::new(),
        3,
    )
    .unwrap();
    // A terminal (Completed) order whose schedule survived — the historical
    // orphan the terminal-bundle teardown never reached.
    let done = mush_industry::WorkOrder::draft(
        order,
        mush_org::PrincipalId::Object(GOD),
        mush_org::PrincipalId::Object(GOD),
        ObjId(7),
        2,
        1,
        1,
        recipe,
        pinned,
        1,
        mush_economy::AccountId(1),
        Some(mush_inventory::EndpointRef(mush_economy::VaultId(500))),
        mush_industry::DeliveryTarget::Endpoint {
            endpoint: mush_inventory::EndpointRef(mush_economy::VaultId(600)),
        },
        mush_economy::EconomicVisibility::Parties,
    )
    .unwrap()
    .with_state(mush_industry::WorkOrderState::Completed {
        commit: mush_core::CommitToken::from_bytes([9; 16]),
    })
    .unwrap();
    apply_industry(engine, &families, |txn| {
        families.open_work_order(txn, done)?;
        txn.insert(&families.schedules, &sid, sch)
            .map_err(mush_industry::IndustryError::from)?;
        txn.insert(&families.jobs, &sid, job)
            .map_err(mush_industry::IndustryError::from)?;
        Ok(())
    })
    .unwrap();
    // Discovery still finds the due schedule (discover reads only the cursor)…
    let page = engine
        .discover_scheduled_actions(mush_core::ScheduleDriveCause::Live, None, 32)
        .expect("discovery does not freeze");
    assert!(
        !page.candidates().is_empty(),
        "the orphan schedule is still discovered"
    );
    // …but planning it refuses as Stale (the adapter sees the terminal order),
    // which `scheduled_prepare_stale` classifies as DiscardedStale — the world
    // is not frozen.
    let candidate = page.candidates()[0].clone();
    let ingress = mush_core::scheduled_action_ingress(candidate.action(), candidate.advance(), 0)
        .expect("ingress");
    match engine.plan_scheduled_action(&candidate, &ingress) {
        Err(mush_core::ScheduleActionPlanError::Adapter { source, .. }) => {
            assert_eq!(
                source.code(),
                mush_core::ActionErrorCode::Stale,
                "a terminal order's schedule plans as Stale (discarded), never a freeze"
            );
        }
        other => panic!("expected a Stale plan refusal, got {other:?}"),
    }
}

/// E7c2 fold C4: the fan-out arm voided the ENGINE-wide slot, so a
/// legitimate submission queued in an earlier step — possibly a different
/// actor's — was silently destroyed and its owner was told nothing.
#[tokio::test]
async fn vf_c4_a_fan_out_never_voids_a_submission_from_an_earlier_step() {
    let mut world = b1_world();
    let _ = b1_run(&mut world, "@b1submit 3");
    assert!(world.engine().has_authoritative_submit());
    let out = b1_run(&mut world, "@b1submit/twice 7");
    assert!(
        world.engine().has_authoritative_submit(),
        "the incumbent from the earlier step survives the fan-out"
    );
    assert!(
        b1_lines(&out)
            .iter()
            .any(|line| line.contains("still committing")),
        "{:?}",
        b1_lines(&out)
    );
    let store = mush_store::InMemoryStore::new();
    let _ = world.drain_write_batch();
    let mut engine = world.into_engine();
    let mut driver = CommandSubmitDriver::new(1);
    let CommandSubmitDrive::Staged { staged, .. } =
        driver.drive_one(&mut engine, &store).await.unwrap()
    else {
        panic!("the incumbent still drives")
    };
    assert_eq!(
        staged.summary.as_deref(),
        Some("The counter stands at 3."),
        "and it is the FIRST submission, not the fan-out's"
    );
}

/// D3 as taken: the slot is per actor, so a fan-out drops only the fanning
/// actor's own — and it tells them. Nobody else's submission is touched and
/// nobody else is told anything at all.
#[tokio::test]
async fn vf_c4_a_fan_out_tells_every_actor_it_dropped() {
    let mut world = b1_world();
    let bystander = world.spawn_player("bystander");
    let _ = b1_run_as(&mut world, bystander, "@b1submit 5");
    assert!(world.engine().has_authoritative_submit_for(bystander));
    let out = b1_run(&mut world, "@b1submit/twice 9");
    let dropped: Vec<String> = b1_lines(&out)
        .into_iter()
        .filter(|line| line.contains("more than one authoritative action"))
        .collect();
    assert_eq!(dropped.len(), 1, "one line per dropped actor: {dropped:?}");
    assert!(
        !world.engine().has_authoritative_submit_for(GOD),
        "the fanning actor stages nothing at all"
    );
    assert!(
        world.engine().has_authoritative_submit_for(bystander),
        "and the bystander's submission is untouched"
    );
}

/// E7c2 fold B7: one player's queued submission used to answer every other
/// player's mutation with "An earlier action is still committing" — a
/// cross-actor denial and a cross-actor tell. The durable one-intent rule
/// (D-E7-39 / 08i answer 6) is about the STORE and is enforced downstream.
#[tokio::test]
async fn vf_b7_one_actors_submission_does_not_block_another() {
    let mut world = b1_world();
    let alice = world.spawn_player("alice");
    let bob = world.spawn_player("bob");
    let alice_out = b1_run_as(&mut world, alice, "@b1submit 3");
    let bob_out = b1_run_as(&mut world, bob, "@b1submit 4");
    assert!(
        !b1_lines(&bob_out)
            .iter()
            .any(|line| line.contains("still committing")),
        "bob is not refused because alice got there first: {:?}",
        b1_lines(&bob_out)
    );
    assert!(world.engine().has_authoritative_submit_for(alice));
    assert!(world.engine().has_authoritative_submit_for(bob));
    assert!(!b1_lines(&alice_out).iter().any(|line| line.contains("bob")));
    // Bob's OWN second submission, while his first is queued, is still
    // back-pressure — and the refusal names nobody else.
    let again = b1_run_as(&mut world, bob, "@b1submit 5");
    let refusal: Vec<String> = b1_lines(&again)
        .into_iter()
        .filter(|line| line.contains("still committing"))
        .collect();
    assert_eq!(refusal.len(), 1, "{refusal:?}");
    assert!(refusal[0].starts_with("Your earlier action"), "{refusal:?}");
    // Both drive, in successive steps.
    let store = mush_store::InMemoryStore::new();
    let _ = world.drain_write_batch();
    let mut engine = world.into_engine();
    let mut driver = CommandSubmitDriver::new(1);
    for _ in 0..2 {
        match driver.drive_one(&mut engine, &store).await.unwrap() {
            CommandSubmitDrive::Staged { .. } => {}
            CommandSubmitDrive::Refused { detail, .. } => panic!("refused: {detail}"),
            _ => panic!("expected a staged submission"),
        }
        let (_, installation) = engine
            .take_staged_action()
            .unwrap()
            .expect("staged action")
            .into_parts();
        engine
            .abort_prepared_action(installation)
            .expect("the fixture aborts");
    }
    assert!(!engine.has_authoritative_submit());
}

/// E7c2 fold R8: a quarantined durable identity on the command lane used to
/// be indistinguishable in the log from a typo. The actor's line is
/// unchanged — that is the disclosure rule — but the drive result is not.
#[tokio::test]
async fn vf_r8_a_quarantined_preflight_is_an_integrity_event() {
    let mut world = b1_world();
    let _ = b1_run(&mut world, "@b1submit 3");
    let store = mush_store::InMemoryStore::new();
    let _ = world.drain_write_batch();
    let mut engine = world.into_engine();
    let mut driver = CommandSubmitDriver::new(1);
    assert!(matches!(
        driver.drive_one(&mut engine, &store).await.unwrap(),
        CommandSubmitDrive::Staged { .. }
    ));
    let (bundle, _installation) = engine
        .take_staged_action()
        .unwrap()
        .expect("staged action")
        .into_parts();
    store
        .reserve_authoritative_intent(*bundle.clone())
        .await
        .unwrap();
    store
        .discard_quarantined_authoritative_intent(
            bundle.token(),
            bundle.bundle_hash(),
            bundle.bundle_hash(),
        )
        .await
        .expect("quarantine the intent");
    // The identical command again: preflight now answers `Quarantined`.
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine.step(
        Inbound::Cmd {
            conn: 1,
            actor: GOD,
            line: "@b1submit 3".to_owned(),
        },
        &mut out,
        &mut fx,
    );
    match driver.drive_one(&mut engine, &store).await.unwrap() {
        CommandSubmitDrive::Refused {
            integrity, detail, ..
        } => {
            assert!(integrity, "a quarantine is an operator event: {detail}");
            assert!(detail.contains("quarantined"), "{detail}");
        }
        _ => panic!("expected an integrity refusal"),
    }
}

/// E7c2 fold R11: acceptance and loss used to be indistinguishable at the
/// terminal — an `Authoritative` handler emits nothing and the only later
/// line is the driver's, so a submission dropped by a crash or parked by a
/// freeze produced no output whatever.
#[tokio::test]
async fn vf_r11_a_queued_submission_acknowledges_receipt_without_claiming_a_mutation() {
    let mut world = b1_world();
    let out = b1_run(&mut world, "@b1submit 3");
    assert_eq!(
        b1_lines(&out),
        vec!["Queued.".to_owned()],
        "exactly one non-committal line"
    );
    assert_eq!(
        out.classes(),
        [mush_core::DeliveryClass::BestEffort],
        "BestEffort, so the durable-ack gate never withholds it and it can \
         never be read as the confirmation"
    );
    assert!(world.engine().has_authoritative_submit());
}

/// E7c2 fold B12: the first commit rendered `deterministic_commit_token`'s
/// internal binding digest raw into player output while the replay of the
/// same act got the provider's own sentence. Both now read the same.
#[tokio::test]
async fn vf_b12_the_first_commit_and_the_hit_read_the_same() {
    let mut world = b1_world();
    let _ = b1_run(&mut world, "@b1submit 4");
    let store = mush_store::InMemoryStore::new();
    let _ = world.drain_write_batch();
    let mut engine = world.into_engine();
    let mut driver = CommandSubmitDriver::new(1);
    let CommandSubmitDrive::Staged { staged, .. } =
        driver.drive_one(&mut engine, &store).await.unwrap()
    else {
        panic!("staged")
    };
    let fresh = staged.summary.clone().expect("the provider's own summary");
    assert!(
        !fresh.contains(&staged.token.to_string()),
        "no raw commit token in player output: {fresh}"
    );
    let (bundle, installation) = engine
        .take_staged_action()
        .unwrap()
        .expect("staged action")
        .into_parts();
    store
        .reserve_authoritative_intent(*bundle.clone())
        .await
        .unwrap();
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();
    engine
        .install_composed_action(installation)
        .expect("the fixture installs");
    store
        .checkpoint_authoritative(bundle.token(), bundle.bundle_hash())
        .await
        .unwrap();
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine.step(
        Inbound::Cmd {
            conn: 1,
            actor: GOD,
            line: "@b1submit 4".to_owned(),
        },
        &mut out,
        &mut fx,
    );
    let CommandSubmitDrive::AlreadyFinished {
        actor,
        receipt,
        status,
    } = driver.drive_one(&mut engine, &store).await.unwrap()
    else {
        panic!("the retyped identical command is a HIT")
    };
    let projections = engine
        .registry
        .action_providers()
        .map(mush_core::ActionProviderRegistry::receipt_projection_table)
        .expect("projections");
    let hit = ActionCoordinator::project_finalized_receipt(
        &projections,
        &PrincipalId::Object { id: actor },
        status,
        &receipt,
    )
    .expect("projected")
    .result()
    .map(|result| result.summary().to_owned())
    .expect("summary");
    assert_eq!(fresh, hit, "the first commit and its HIT read the same");
}

/// The seam must not fork a second spelling of the five steps. Both
/// ingresses call `resolve_and_stage_offer`, and it is the only site in this
/// file that calls `prepare_and_stage_resolved`.
#[test]
fn e7c2_b1_both_ingresses_call_one_five_step_function() {
    // Only the production half of this file; the assertions below name the
    // very symbols they count.
    let source = include_str!("../world_task.rs")
        .split("#[cfg(test)]")
        .next()
        .expect("the file has a production half");
    let calls = source.matches("resolve_and_stage_offer(").count();
    assert!(
        calls >= 3,
        "expected the definition plus both call sites, found {calls}"
    );
    let staged = source
        .matches("ActionCoordinator::prepare_and_stage_resolved(")
        .count();
    assert_eq!(
        staged, 1,
        "prepare_and_stage_resolved must be reached through exactly one function"
    );
}

// ---- H2a: the production decision composition ----------------------

/// A provider that always picks the prompt's `idle` offer, as a model
/// would by calling the idle tool.
struct IdleChoosingProvider {
    owner: OwnerId,
    calls: Arc<AtomicUsize>,
}

impl DecisionProviderAdapter for IdleChoosingProvider {
    fn owner(&self) -> &OwnerId {
        &self.owner
    }

    fn decide(
        &self,
        lease: DecisionWorkerLease,
    ) -> futures::future::BoxFuture<'static, Result<DecisionResponse, DecisionProviderFailure>>
    {
        self.calls.fetch_add(1, AtomicOrdering::SeqCst);
        let prompt = lease.prompt().clone();
        Box::pin(async move {
            let idle = prompt
                .actions
                .iter()
                .find(|a| a.presentation().verb == mush_npc::IDLE_VERB)
                .ok_or_else(|| {
                    DecisionProviderFailure::new(
                        DecisionProviderFailureKind::Malformed,
                        "no idle offer",
                    )
                })?;
            Ok(DecisionResponse {
                token: prompt.token,
                offer: idle.offer(),
                targets: Vec::new(),
                parameters: serde_json::json!({}),
            })
        })
    }
}

/// A world composed with the real `npc` module, keyed, with one NPC agent
/// and one decision request queued for the tick — the capture already
/// taken by the engine, ready for the loop.
fn npc_decision_world(deadline_secs: i64, lease_secs: i64) -> (Engine, mush_core::ObjId) {
    let mut tw = mush_testkit::TestWorld::with_modules(vec![Box::new(
        mush_npc::NpcModule::new().with_decision_lease_secs(lease_secs),
    )]);
    tw.engine_mut().set_decision_keys(
        DecisionKeyRing::new(DecisionSecret::new(1, [0x5a; 32]).unwrap(), []).unwrap(),
    );
    tw.engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x6b; 32]).unwrap(), []).unwrap());
    let events = tw.run(GOD, "@agent/create Bob");
    let text: String = events
        .iter()
        .map(|(_, e)| mush_testkit::text_of(e))
        .collect::<Vec<_>>()
        .join("\n");
    let at = text.find("(#").expect("the create line names the id");
    let digits: String = text[at + 2..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let npc = mush_core::ObjId(digits.parse().unwrap());
    // The NPC object stays dirty: the world task persists it on its
    // first iteration, before it drives the capture, so a second boot
    // from the store alone still has the agent to project offers for.
    tw.world_mut()
        .module_state_mut::<mush_npc::NpcHostState>()
        .unwrap()
        .request_decision(mush_npc::NpcDecisionRequest {
            npc,
            cause_kind: CanonicalId::new("npc.test_turn").unwrap(),
            instance: IdempotencyKeyText::parse_canonical("turn-1").unwrap(),
            replaceable_key: None,
            context: serde_json::json!({ "room": "Limbo" }),
            deadline_secs,
            key: IdempotencyKeyText::parse_canonical("turn-1").unwrap(),
        });
    let _ = tw.fire_due(5_000);
    assert!(
        tw.engine().has_decision_capture(),
        "the tick issued no capture"
    );
    (tw.into_engine(), npc)
}

fn receipt_key(npc: mush_core::ObjId) -> String {
    format!("action-state/v1/npc_turn_receipts/npc_{}", npc.0)
}

async fn wait_for_receipt(
    store: &Arc<dyn ObjectStore>,
    npc: mush_core::ObjId,
    secs: u64,
) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(secs), async {
        loop {
            if let Some(value) = store
                .module_json_get("npc", &receipt_key(npc))
                .await
                .expect("receipt read")
            {
                break value;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the decision did not execute its action")
}

/// The whole production lifecycle through the production ingress: a
/// queued request, the tick's capture, the loop builds, arms and submits
/// `npc_decide`, the wake, the lease, the provider, the response, the
/// coordinator executes `npc_idle`, and its turn receipt lands.
#[tokio::test]
async fn h2a_a_captured_npc_decision_runs_to_its_executed_action() {
    let (engine, npc) = npc_decision_world(60, 60);
    let deps = ServerDeps::in_memory();
    let store = Arc::clone(&deps.store);
    let calls = Arc::new(AtomicUsize::new(0));
    let mut providers = DecisionProviderRegistry::new();
    providers
        .register(Arc::new(IdleChoosingProvider {
            owner: OwnerId::new("npc").unwrap(),
            calls: Arc::clone(&calls),
        }))
        .unwrap();
    let handles = spawn_persist_with_gateway_and_decisions(
        engine,
        deps,
        DurableOutboxRegistry::new(),
        providers,
        1,
        |_| Arc::new(BlackholeGateway),
    )
    .expect("decision runtime startup");
    let receipt = wait_for_receipt(&store, npc, 5).await;
    assert_eq!(receipt["value"]["outcome"], "idle");
    assert_eq!(receipt["value"]["npc"], npc.0);
    assert_eq!(calls.load(AtomicOrdering::SeqCst), 1, "one lease, one call");
    // The pending record reached terminal: nothing is left to dispatch.
    let listed = store
        .module_json_list("npc")
        .await
        .expect("npc namespace listing");
    assert!(
        listed
            .iter()
            .any(|(key, value)| key.contains("npc_pending_decisions")
                && value["value"]["status"]["state"] == "terminal"
                && value["value"]["status"]["outcome"] == "executed"),
        "no terminal executed decision: {:?}",
        listed.iter().map(|(k, _)| k).collect::<Vec<_>>()
    );
    stop_test_server(handles).await;
}

/// With no LLM service the production adapter answers `Unavailable`
/// every time, and the deadline hands the decision to `npc_idle`.
#[tokio::test]
async fn h2a_an_unavailable_provider_falls_back_to_idle_at_the_deadline() {
    let (engine, npc) = npc_decision_world(1, 60);
    let deps = ServerDeps::in_memory();
    let store = Arc::clone(&deps.store);
    let providers = production_decision_providers(&engine, None).unwrap();
    let handles = spawn_persist_with_gateway_and_decisions(
        engine,
        deps,
        DurableOutboxRegistry::new(),
        providers,
        1,
        |_| Arc::new(BlackholeGateway),
    )
    .expect("decision runtime startup");
    let receipt = wait_for_receipt(&store, npc, 20).await;
    assert_eq!(receipt["value"]["outcome"], "idle");
    let listed = store.module_json_list("npc").await.unwrap();
    assert!(
        listed
            .iter()
            .any(|(key, value)| key.contains("npc_pending_decisions")
                && value["value"]["status"]["state"] == "terminal"
                && value["value"]["status"]["fallback_reason"].is_string()),
        "the terminal record does not name its fallback"
    );
    stop_test_server(handles).await;
}

/// A restart between the capture and the lease resumes from the pending
/// family (K7.4): the second boot finds the record and finishes it.
#[tokio::test]
async fn h2a_a_restart_between_capture_and_lease_resumes_the_decision() {
    // A one-second lease, as the kernel's own restart test uses: the first
    // boot's abandoned lease must expire for the second to re-lease.
    let (engine, npc) = npc_decision_world(60, 1);
    let deps = ServerDeps::in_memory();
    let store = Arc::clone(&deps.store);
    // First boot: the capture commits; the provider can never answer.
    let providers = production_decision_providers(&engine, None).unwrap();
    let first = spawn_persist_with_gateway_and_decisions(
        engine,
        deps,
        DurableOutboxRegistry::new(),
        providers,
        1,
        |_| Arc::new(BlackholeGateway),
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let listed = store.module_json_list("npc").await.unwrap();
            if listed
                .iter()
                .any(|(key, _)| key.contains("npc_pending_decisions"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the capture did not commit");
    stop_test_server(first).await;
    assert!(
        store
            .module_json_get("npc", &receipt_key(npc))
            .await
            .unwrap()
            .is_none(),
        "nothing executed before the restart"
    );

    // Second boot from the store alone, with a provider that answers. Its
    // OWN lease is long: the abandoned lease expires on the first boot's
    // one-second terms whatever this boot would grant, and a one-second
    // lease here let a starved box (the workspace gate beside other
    // builds) expire the re-lease before the answer was applied and call
    // the provider a second time — the exactly-once assertion below is
    // about the restart, not about the box's scheduling.
    let world = crate::persist::load_or_seed_world(&*store).await.unwrap();
    let mut engine = Engine::with_modules(
        world,
        mush_core::Registry::new(),
        vec![Box::new(
            mush_npc::NpcModule::new().with_decision_lease_secs(60),
        )],
    )
    .unwrap();
    engine.set_decision_keys(
        DecisionKeyRing::new(DecisionSecret::new(1, [0x5a; 32]).unwrap(), []).unwrap(),
    );
    engine.set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x6b; 32]).unwrap(), []).unwrap());
    crate::modules::boot_modules(&mut engine, &*store)
        .await
        .unwrap();
    assert_eq!(
        decision_census(&engine).nonterminal(),
        1,
        "the pending decision came back on boot"
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let mut providers = DecisionProviderRegistry::new();
    providers
        .register(Arc::new(IdleChoosingProvider {
            owner: OwnerId::new("npc").unwrap(),
            calls: Arc::clone(&calls),
        }))
        .unwrap();
    let second = spawn_persist_with_gateway_and_decisions(
        engine,
        decision_test_deps(Arc::clone(&store)),
        DurableOutboxRegistry::new(),
        providers,
        1,
        |_| Arc::new(BlackholeGateway),
    )
    .unwrap();
    let receipt = wait_for_receipt(&store, npc, 10).await;
    assert_eq!(receipt["value"]["outcome"], "idle");
    assert_eq!(calls.load(AtomicOrdering::SeqCst), 1);
    stop_test_server(second).await;
}

/// The production spawn: a keyed composition starts the decision runtime
/// over the LLM adapter; an unkeyed one starts the plain runtime; a
/// worker limit past the ceiling refuses.
#[tokio::test]
async fn h2a_the_production_spawn_composes_the_npc_adapter() {
    let (engine, _) = npc_decision_world(60, 60);
    let providers = production_decision_providers(&engine, None).unwrap();
    assert!(providers.contains_owner(&OwnerId::new("npc").unwrap()));
    let handles = spawn_persist_with_gateway_production(
        engine,
        ServerDeps::in_memory(),
        None,
        DEFAULT_DECISION_PROVIDER_WORKERS,
        DEFAULT_WORLD_CLOCK_CHECKPOINT_S,
        |_| Arc::new(BlackholeGateway),
    )
    .expect("production spawn");
    stop_test_server(handles).await;

    let tw = mush_testkit::TestWorld::with_modules(vec![Box::new(mush_npc::NpcModule::new())]);
    let handles = spawn_persist_with_gateway_production(
        tw.into_engine(),
        ServerDeps::in_memory(),
        None,
        DEFAULT_DECISION_PROVIDER_WORKERS,
        DEFAULT_WORLD_CLOCK_CHECKPOINT_S,
        |_| Arc::new(BlackholeGateway),
    )
    .expect("an unkeyed composition still spawns");
    stop_test_server(handles).await;

    let (engine, _) = npc_decision_world(60, 60);
    let refused = spawn_persist_with_gateway_production(
        engine,
        ServerDeps::in_memory(),
        None,
        mush_core::MAX_DECISION_PROVIDER_WORKERS + 1,
        DEFAULT_WORLD_CLOCK_CHECKPOINT_S,
        |_| Arc::new(BlackholeGateway),
    );
    assert!(refused.is_err(), "33 workers must refuse");
}

// ---- H2b: an NPC's scene turn as a durable decision ---------------

/// `+timestop/next` hands the turn to an NPC; the NPC module requests a
/// decision; the tick captures it; the loop runs it to `npc_idle` through
/// the coordinator; and the scene stays exactly where it was — Active,
/// the NPC still holding the turn — because nothing auto-advances.
#[tokio::test]
async fn h2b_an_npc_scene_turn_runs_to_its_action_and_leaves_the_scene_in_place() {
    let mut tw = mush_testkit::TestWorld::with_modules(vec![
        Box::new(mush_scenes::ScenesModule::new()),
        Box::new(mush_npc::NpcModule::new()),
    ]);
    tw.engine_mut().set_decision_keys(
        DecisionKeyRing::new(DecisionSecret::new(1, [0x5a; 32]).unwrap(), []).unwrap(),
    );
    tw.engine_mut()
        .set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x6b; 32]).unwrap(), []).unwrap());
    let room = tw
        .world_mut()
        .create(mush_core::ObjKind::Room, "The Bar", GOD, None)
        .unwrap();
    tw.world_mut().move_to(GOD, room).unwrap();
    let player = tw.spawn_player("Pat");
    tw.world_mut().move_to(player, room).unwrap();
    tw.world_mut().set_connected(player, true);
    tw.world_mut().set_live_conns(player, 1);
    let created: String = tw
        .run(GOD, "@agent/create Bob")
        .iter()
        .map(|(_, e)| mush_testkit::text_of(e))
        .collect::<Vec<_>>()
        .join("\n");
    let at = created.find("(#").unwrap();
    let digits: String = created[at + 2..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let npc = mush_core::ObjId(digits.parse().unwrap());
    let _ = tw.run(GOD, "+timestop");
    // Round 1 opens on Pat; the second advance hands the turn to Bob.
    let _ = tw.run(GOD, "+timestop/next");
    let _ = tw.run(GOD, "+timestop/next");
    let queued = tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap()
        .decision_requests
        .len();
    assert_eq!(queued, 1, "the NPC's turn requested one decision");
    let _ = tw.fire_due(5_000);
    assert!(tw.engine().has_decision_capture());
    let engine = tw.into_engine();

    let deps = ServerDeps::in_memory();
    let store = Arc::clone(&deps.store);
    let calls = Arc::new(AtomicUsize::new(0));
    let mut providers = DecisionProviderRegistry::new();
    providers
        .register(Arc::new(IdleChoosingProvider {
            owner: OwnerId::new("npc").unwrap(),
            calls: Arc::clone(&calls),
        }))
        .unwrap();
    let handles = spawn_persist_with_gateway_and_decisions(
        engine,
        deps,
        DurableOutboxRegistry::new(),
        providers,
        1,
        |_| Arc::new(BlackholeGateway),
    )
    .expect("decision runtime startup");
    let receipt = wait_for_receipt(&store, npc, 5).await;
    assert_eq!(receipt["value"]["outcome"], "idle");
    assert_eq!(calls.load(AtomicOrdering::SeqCst), 1);
    // The scene did not move: still active, the NPC still on turn.
    let scenes = store.module_json_list("scenes").await.unwrap();
    let record = scenes
        .iter()
        .find(|(key, value)| key.starts_with("scene:") && value.get("participants").is_some())
        .map(|(_, value)| value.clone())
        .expect("the persisted scene record");
    assert_eq!(record["status"], "active", "{record}");
    let participants = record["participants"].as_array().unwrap();
    let turn_index = record["turn_index"].as_u64().unwrap() as usize;
    assert_eq!(participants[turn_index], npc.0, "{record}");
    stop_test_server(handles).await;
}

/// # E7 cut 8 independent verify fold — D7(a): the pulse, driven
///
/// §3.8's deviation (e) recorded "`DurableScheduleDriver::drive_one` is a
/// private `async fn` of `world_task.rs` with no test seam" and deferred the
/// end-to-end tick replay and the slot-hold figure on that basis. The
/// premise is false **across crates only**: this very module already drives
/// `drive_one` a dozen times over `mush_store::InMemoryStore`, and
/// `mush-sim` is an ordinary (non-dev) dependency of `mush-server` — so
/// composing `SimulationModule` instead of the toy owner is the only new
/// thing here. No new public API, no `#[ignore]`, no `*_gated` test and no
/// CI job: `ci_gated_test_inventory` is untouched.
///
/// Three of the fold's HIGHs (T1, T2, T3) were invisible to every test cut 8
/// wrote for exactly one reason: none of them advanced the clock across two
/// consecutive **driven** ticks. These do.
mod e7c8_pulse {
    use super::*;
    use mush_core::{
        ActionInvocation, ActionInvocationDraft, ActionOfferId, CanonicalHash, CanonicalId,
        CommitToken, DefinitionId, DefinitionKey, DefinitionKind, DefinitionRef, IdempotencyKey,
        IdempotencyScope, ObjId, OwnerId, PerceivedRef, PrincipalId, RequestUid,
        ScopedIdempotencyKey,
    };
    use mush_sim::tick::{
        DomainTickCommit, MAX_RETAINED_TICK_COMMITS as RETAINED, SCHEDULE_SOURCE, TickCommitKey,
        TickHandles, pulse_clock, tick_schedule_id,
    };
    use mush_sim::{DomainId, SimulationModule, SpatialState};
    use serde_json::json;

    /// The world every row here starts from: one domain host, promoted, its
    /// activation applied, with the durable cursor armed through the REAL
    /// `domain_pulse_arm` authority.
    fn pulsing_world() -> (mush_core::Engine, mush_store::InMemoryStore, i64) {
        let mut world =
            mush_testkit::TestWorld::with_modules(vec![Box::new(SimulationModule::new())]);
        world.set_now(1_000);
        let host = world
            .world_mut()
            .create(mush_core::ObjKind::Room, "Landing Field", GOD, None)
            .unwrap();
        world
            .world_mut()
            .attach_behavior(
                host,
                mush_core::BehaviorId::new("sim.domain").unwrap(),
                mush_core::BehaviorBinding {
                    schema_version: 1,
                    attached_by: GOD,
                    attached_at: 0,
                },
            )
            .unwrap();
        let _ = world.run(GOD, &format!("@domain/create #{}=sim.passive", host.0));
        let _ = world.run(GOD, &format!("@domain/promote #{}=contact", host.0));
        let _ = world.run(GOD, &format!("@domain/tick #{}", host.0));
        let submit = *world
            .engine_mut()
            .take_authoritative_submit()
            .expect("@domain/tick stages the arm from the command handler");
        let _ = world.drain_write_batch();
        let mut engine = world.into_engine();
        // The production composition installs both key rings; the tick
        // resolves with recorded randomness and its window PINS the version
        // it draws under (fold T11).
        engine.set_resolution_keys(
            mush_core::ResolutionKeyRing::new(
                mush_core::ResolutionSecret::new(3, [7; 32]).unwrap(),
                [],
            )
            .unwrap(),
        );
        arm_directly(&mut engine, &submit);
        let store = mush_store::InMemoryStore::new();
        (engine, store, 1_000)
    }

    /// Apply the queued `domain_pulse_arm` submission straight through the
    /// registered provider. The staff gate is the fixed `armed_by`
    /// parameter, which the submission already carries by construction.
    fn arm_directly(engine: &mut mush_core::Engine, submit: &mush_core::AuthoritativeSubmit) {
        let mut parameters = submit.parameters.clone();
        parameters["armed_by"] = json!(submit.actor);
        let invocation = invocation_of(&submit.action, submit.actor, parameters, 21);
        let (prepared, _) = engine
            .prepare_registered_authoritative_action(
                &submit.action,
                &invocation,
                CommitToken::from_bytes([21; 16]),
            )
            .expect("the pulse arm prepares");
        let (_ops, installation) = prepared.into_parts();
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");
    }

    fn invocation_of(
        action: &DefinitionRef,
        actor: ObjId,
        parameters: serde_json::Value,
        token: u8,
    ) -> ActionInvocation {
        let principal = PrincipalId::Object { id: actor };
        ActionInvocation::new(ActionInvocationDraft {
            request: RequestUid::from_bytes([token; 16]),
            principal: principal.clone(),
            actor,
            offered_action: ActionOfferId(CanonicalHash::from_bytes([9; 32])),
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
                IdempotencyKey::Request(RequestUid::from_bytes([token; 16])),
            ),
        })
        .expect("a well-formed invocation")
    }

    /// Drive until a tick stages, completing it exactly as `world_task`
    /// does: reserve, reconcile, install.
    ///
    /// The driver's page cursor walks one record per call and resets when
    /// the page comes back empty, so a due occurrence takes at most a
    /// couple of polls to surface — exactly as the 250 ms world loop
    /// experiences it. Every intermediate outcome is asserted **not** to be
    /// an `Err`, because an `Err` here is `apply_integrity_freeze`.
    async fn drive_until_staged(
        driver: &mut DurableScheduleDriver,
        engine: &mut mush_core::Engine,
        store: &mush_store::InMemoryStore,
    ) -> Result<&'static str, String> {
        let mut last = "idle";
        for _ in 0..4 {
            last = drive_and_apply(driver, engine, store).await?;
            if last == "staged" {
                return Ok(last);
            }
        }
        Ok(last)
    }

    /// Drive one scheduled action and, when it stages, complete it exactly
    /// as `world_task` does: reserve, reconcile, install.
    async fn drive_and_apply(
        driver: &mut DurableScheduleDriver,
        engine: &mut mush_core::Engine,
        store: &mush_store::InMemoryStore,
    ) -> Result<&'static str, String> {
        let outcome = driver.drive_one(engine, store).await?;
        Ok(match outcome {
            DurableScheduleDrive::Idle => "idle",
            DurableScheduleDrive::DeferredByLegacyPredecessor => "deferred",
            DurableScheduleDrive::DiscardedStale(_) => "stale",
            DurableScheduleDrive::Transient(_) => "transient",
            DurableScheduleDrive::Staged(_) => {
                let staged = engine
                    .take_staged_action()
                    .expect("the staged tick is admitted")
                    .expect("a staged tick action");
                let (bundle, installation) = staged.into_parts();
                store
                    .reserve_authoritative_intent((*bundle).clone())
                    .await
                    .expect("reserve");
                store
                    .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
                    .await
                    .expect("reconcile");
                engine
                    .install_composed_action(installation)
                    .expect("the fixture installs");
                "staged"
            }
        })
    }

    fn handles(engine: &mush_core::Engine) -> TickHandles {
        engine
            .world
            .module_state::<TickHandles>()
            .expect("the pulse registers its read handles")
            .clone()
    }

    fn generation(engine: &mush_core::Engine) -> u64 {
        engine
            .world
            .module_state::<SpatialState>()
            .and_then(|state| state.domains.get(&DomainId(0)).cloned())
            .and_then(|record| record.clock.generation())
            .expect("a pulsing domain")
    }

    fn commit_at(engine: &mush_core::Engine, tick: i64) -> Option<DomainTickCommit> {
        let handles = handles(engine);
        engine
            .action_state()
            .get(
                &handles.commits,
                &TickCommitKey {
                    domain: DomainId(0),
                    generation: generation(engine),
                    tick,
                },
            )
            .ok()
            .flatten()
            .cloned()
    }

    /// **T1 (HIGH), the whole point of this module.** A promoted domain's
    /// SECOND consecutive tick **stages**; before the fold it returned
    /// `Err`, which `world_task.rs:4127` turns into `apply_integrity_freeze`
    /// — a promoted domain resolved exactly one tick and then wedged the
    /// server. The cause: the maintenance pass re-armed the live cursor on
    /// every fire and `PulseArmProvider` reset it to
    /// `(generation 1, ordinal 0, revision 1)`, so `advance_key` — which
    /// hashes `{schedule, owner, generation, next_ordinal, revision}` and
    /// **not** `next_due` — minted the ingress identity the previous tick
    /// had already finalized.
    #[tokio::test]
    async fn vf_t1a_a_second_consecutive_tick_stages_rather_than_freezing_the_world() {
        let (mut engine, store, at) = pulsing_world();
        let mut driver = DurableScheduleDriver::new(1);
        // Tick one.
        engine.world.set_now(at + 10);
        let first = drive_until_staged(&mut driver, &mut engine, &store)
            .await
            .expect("the first tick drives");
        assert_eq!(first, "staged", "the first tick must stage");
        // Tick two, and tick three: the domain clock has moved on, so a
        // fresh occurrence is due each time.
        engine.world.set_now(at + 20);
        let second = drive_until_staged(&mut driver, &mut engine, &store)
            .await
            .expect("the SECOND tick must not return Err: that is the freeze");
        assert_eq!(
            second, "staged",
            "the second consecutive tick must stage, not freeze the world"
        );
        // And a third, so the property is not an artefact of two.
        engine.world.set_now(at + 30);
        let third = drive_until_staged(&mut driver, &mut engine, &store)
            .await
            .expect("the third tick drives");
        assert_eq!(third, "staged");
        // Two distinct commits exist, at consecutive ticks.
        let ticks: Vec<i64> = (0..20)
            .filter(|t| commit_at(&engine, *t).is_some())
            .collect();
        assert!(
            ticks.len() >= 2,
            "three driven ticks must leave more than one commit; got {ticks:?}"
        );
    }

    /// **T1(b).** Re-arming a live cursor leaves its ordinal and revision
    /// untouched — the authority is idempotent in TRUTH, not merely under
    /// its key.
    #[tokio::test]
    async fn vf_t1b_a_rearm_of_a_live_cursor_leaves_its_ordinal_and_revision_untouched() {
        let (mut engine, store, at) = pulsing_world();
        let mut driver = DurableScheduleDriver::new(1);
        engine.world.set_now(at + 10);
        drive_until_staged(&mut driver, &mut engine, &store)
            .await
            .expect("a tick");
        let owner = OwnerId::new("sim").unwrap();
        let id = tick_schedule_id(&owner, DomainId(0)).expect("a cursor id");
        let before = engine
            .action_state()
            .get(&handles(&engine).schedules, &id)
            .expect("read")
            .expect("a live cursor")
            .clone();
        // Re-arm it at the same generation, exactly as the maintenance pass
        // would if it were still pushing every live domain onto `armed`.
        let action = engine
            .registry
            .definitions()
            .and_then(|definitions| {
                definitions
                    .current(&DefinitionKey {
                        owner: owner.clone(),
                        kind: DefinitionKind::new("action").unwrap(),
                        id: DefinitionId::new("domain_pulse_arm").unwrap(),
                    })
                    .cloned()
            })
            .expect("the arm action is composed");
        let document = json!({
            "domain": 0,
            "generation": generation(&engine),
            "from_tick": 0,
            "arm": true,
            "clear_units": [],
        });
        let invocation = invocation_of(
            &action,
            GOD,
            json!({
                "arm": serde_json::to_string(&document).unwrap(),
                "armed_by": json!(GOD),
            }),
            44,
        );
        let (prepared, _) = engine
            .prepare_registered_authoritative_action(
                &action,
                &invocation,
                CommitToken::from_bytes([44; 16]),
            )
            .expect("the re-arm prepares");
        let (_ops, installation) = prepared.into_parts();
        engine
            .install_composed_action(installation)
            .expect("the fixture installs");
        let after = engine
            .action_state()
            .get(&handles(&engine).schedules, &id)
            .expect("read")
            .expect("the cursor survives")
            .clone();
        assert_eq!(
            (
                before.next_ordinal(),
                before.revision(),
                before.next_due().clone()
            ),
            (
                after.next_ordinal(),
                after.revision(),
                after.next_due().clone()
            ),
            "a re-arm at the same generation must change NOTHING about the cursor"
        );
    }

    /// **The deferred replay row** (§3.8 deviation (e)). Drive a tick, drop
    /// the staged bundle, drive it again: the same `state_hash`. The RNG
    /// stream does not re-draw, because the window pins the resolution
    /// secret version the tick draws under (fold T11).
    #[tokio::test]
    async fn vf_d7b_a_dropped_staged_tick_redrives_to_the_same_state_hash() {
        let (mut engine, store, at) = pulsing_world();
        let mut driver = DurableScheduleDriver::new(1);
        engine.world.set_now(at + 10);
        // First attempt: stage it and then DROP it, as a crash between
        // staging and reconciliation does.
        let outcome = driver.drive_one(&mut engine, &store).await.expect("drives");
        let DurableScheduleDrive::Staged(_) = outcome else {
            panic!("the tick must stage")
        };
        let staged = engine
            .take_staged_action()
            .expect("admitted")
            .expect("a staged tick");
        let (first, installation) = staged.into_parts();
        let first_hash = first.bundle_hash();
        engine
            .abort_prepared_action(installation)
            .expect("the fixture aborts");
        // Second attempt over the same clock: the same bundle hash.
        let mut driver = DurableScheduleDriver::new(1);
        let outcome = driver.drive_one(&mut engine, &store).await.expect("drives");
        let DurableScheduleDrive::Staged(_) = outcome else {
            panic!("the re-drive must stage")
        };
        let staged = engine
            .take_staged_action()
            .expect("admitted")
            .expect("a staged tick");
        let (second, installation) = staged.into_parts();
        assert_eq!(
            first_hash,
            second.bundle_hash(),
            "a re-drive after a dropped bundle must reproduce the same commit"
        );
        engine
            .abort_prepared_action(installation)
            .expect("the fixture aborts");
    }

    /// **T2/T3 (HIGH).** A stood-down domain whose cursor is still live
    /// drives `Idle` or `Transient` — **never** `Err`. Before the fold every
    /// `DomainClockProvider` failure was `ClockReadError::DomainProvider`,
    /// which neither the scan's pause arm nor `schedule_discovery_retryable`
    /// could match, so one stood-down domain aborted the whole shared
    /// discovery and took production, `control_handoff`, `npc` and cut 7's
    /// six `sensor_*` sources down with it inside one 250 ms poll.
    #[tokio::test]
    async fn vf_t2b_a_stood_down_domain_with_a_live_cursor_drives_idle_never_err() {
        let (mut engine, store, at) = pulsing_world();
        let mut driver = DurableScheduleDriver::new(1);
        engine.world.set_now(at + 10);
        drive_until_staged(&mut driver, &mut engine, &store)
            .await
            .expect("a first tick");
        // Stand the domain down WITHOUT disarming: the exact window the old
        // code left open on every demote, and which T3 made permanent.
        if let Some(state) = engine.world.module_state_mut::<SpatialState>()
            && let Some(record) = state.domains.get_mut(&DomainId(0))
        {
            record.clock = mush_sim::data::DomainClock::Passive;
        }
        engine.world.set_now(at + 20);
        for _ in 0..3 {
            let outcome = driver
                .drive_one(&mut engine, &store)
                .await
                .expect("a sick domain must never return Err from the shared scan");
            assert!(
                matches!(
                    outcome,
                    DurableScheduleDrive::Idle | DurableScheduleDrive::Transient(_)
                ),
                "a stood-down domain with a live cursor did not pause its own entry"
            );
        }
        // The provider really cannot answer — the pause is doing the work.
        let clock =
            pulse_clock(&OwnerId::new("sim").unwrap(), DomainId(0), 2).expect("a domain clock");
        assert!(engine.clock_now(&clock).is_err());
    }

    /// **The slot-hold, measured** (§3.8's last unmeasured figure). How long
    /// the driver holds the durable schedule slot for one tick, over an
    /// in-memory store — the floor, not a production figure, but a real
    /// measurement rather than a deferral.
    #[tokio::test]
    async fn vf_d7c_the_measured_slot_hold_for_one_driven_tick() {
        let (mut engine, store, at) = pulsing_world();
        let mut driver = DurableScheduleDriver::new(1);
        let mut worst = std::time::Duration::ZERO;
        let mut total = std::time::Duration::ZERO;
        let rounds = 5_u32;
        for round in 0..rounds {
            engine.world.set_now(at + 10 + 10 * i64::from(round));
            let started = std::time::Instant::now();
            let outcome = drive_until_staged(&mut driver, &mut engine, &store)
                .await
                .expect("every driven tick stages");
            let elapsed = started.elapsed();
            assert_eq!(outcome, "staged", "round {round}");
            total += elapsed;
            worst = worst.max(elapsed);
        }
        println!(
            "E7c8 fold D7(c) slot-hold over InMemoryStore: {} tick(s), mean {:?}, worst {:?}",
            rounds,
            total / rounds,
            worst
        );
        // The pin is a sanity ceiling, not a benchmark: a tick that holds
        // the slot for a whole second has stopped being a 2 s pulse.
        assert!(
            worst < std::time::Duration::from_millis(500),
            "one driven tick held the durable schedule slot for {worst:?}"
        );
    }

    /// **T8 (MEDIUM), end to end.** The consumed window is RETAINED for one
    /// tick and then reaped by the NEXT fire's bounded sweep — it is no
    /// longer deleted in the same bundle. Deleting it destroyed the kernel's
    /// own duplicate fence: `prepare_authoritative` treats absence as "open
    /// a fresh one", so an intent that raced the 250 ms driver poll against
    /// the 2 s pulse was accepted into a spent window, acknowledged to the
    /// player, and never consumed.
    #[tokio::test]
    async fn vf_t8b_the_consumed_window_is_retained_one_tick_and_then_reaped() {
        let (mut engine, store, at) = pulsing_world();
        let mut driver = DurableScheduleDriver::new(1);
        engine.world.set_now(at + 10);
        drive_until_staged(&mut driver, &mut engine, &store)
            .await
            .expect("a first tick");
        let first_windows = window_count(&engine);
        assert!(
            first_windows > 0,
            "the consumed window is retained, not deleted in the same bundle"
        );
        // Several more ticks: the family stays flat rather than growing one
        // record per tick, which is what the ring is for.
        for round in 1..6 {
            engine.world.set_now(at + 10 + 10 * i64::from(round));
            drive_until_staged(&mut driver, &mut engine, &store)
                .await
                .expect("a later tick");
        }
        let later = window_count(&engine);
        assert!(
            later <= first_windows + 2,
            "the window family grew from {first_windows} to {later} across five                  driven ticks; the reaper is not bounding it"
        );
    }

    /// **B3 (HIGH), end to end.** Neither the window family nor the commit
    /// family grows without bound across a long run of driven ticks.
    #[tokio::test]
    async fn vf_b3c_the_window_and_commit_families_stay_flat_across_many_driven_ticks() {
        let (mut engine, store, at) = pulsing_world();
        let mut driver = DurableScheduleDriver::new(1);
        for round in 0..8 {
            engine.world.set_now(at + 10 + 10 * i64::from(round));
            drive_until_staged(&mut driver, &mut engine, &store)
                .await
                .expect("every tick stages");
        }
        assert!(
            window_count(&engine) <= 3,
            "the window family grew to {} across eight driven ticks",
            window_count(&engine)
        );
        assert!(
            commit_count(&engine) <= usize::try_from(RETAINED).unwrap_or(64) + 1,
            "the commit family grew past its retention ring"
        );
    }

    fn window_count(engine: &mush_core::Engine) -> usize {
        engine
            .action_state()
            .scan_family(&handles(engine).windows, None, 256)
            .map(|page| page.entries().len())
            .unwrap_or(0)
    }

    fn commit_count(engine: &mush_core::Engine) -> usize {
        engine
            .action_state()
            .scan_family(&handles(engine).commits, None, 256)
            .map(|page| page.entries().len())
            .unwrap_or(0)
    }

    /// The source is composed exactly once and under its own id, so nothing
    /// above depends on a second registration having appeared.
    #[test]
    fn vf_d7a_the_domaintick_source_is_composed_once_in_the_server_composition() {
        let world = mush_testkit::TestWorld::with_modules(vec![Box::new(SimulationModule::new())]);
        let sources = world
            .registry()
            .schedule_sources()
            .expect("a schedule source registry");
        assert!(!sources.is_empty());
        assert_eq!(SCHEDULE_SOURCE, "domaintick");
    }
}

#[tokio::test]
async fn discord_unlink_and_ban_revoke_every_character_on_the_account() {
    for operation in ["self-unlink", "staff-unlink", "ban"] {
        let store = InMemoryStore::new();
        store
            .create_account(AccountRec {
                id: None,
                name: "Nick".into(),
                argon2_hash: "fixture".into(),
                created: 1,
                characters: vec![],
                ssh_keys: vec![],
                discord: Some(mush_store::DiscordLinkRec {
                    user_id: 42,
                    linked_at: 1,
                    relay_character: None,
                    auto_follow: true,
                    dm_channel: None,
                    quiet: false,
                }),
            })
            .await
            .unwrap();
        store
            .link_character("Nick", ObjId(7), "Nick")
            .await
            .unwrap();
        store.link_character("Nick", ObjId(8), "Alt").await.unwrap();
        store.discord_user_reserve(42, "Nick").await.unwrap();
        let recorder = Arc::new(RecordingGateway::default());
        let gateway: Gateway = recorder.clone();
        let result = if operation == "ban" {
            run_discord_ban(&gateway, &store, GOD, ObjId(7), "test").await
        } else {
            run_discord_unlink(&gateway, &store, ObjId(7), operation == "staff-unlink").await
        };
        assert!(!result.contains("failed"), "{result}");
        assert_eq!(
            *recorder.discord.lock().unwrap(),
            vec![(7, DiscordControlOp::Revoke), (8, DiscordControlOp::Revoke)],
            "{operation}: {result}"
        );
    }
}

#[tokio::test]
async fn custom_async_effect_runner_dispatches_and_reenters_world_loop() {
    struct EchoRunner;

    impl AsyncEffectRunner for EchoRunner {
        fn module(&self) -> &str {
            "custom_echo"
        }

        fn run(
            &self,
            correlation: u64,
            payload: String,
            _llm: Option<Arc<LlmService>>,
        ) -> Option<futures::future::BoxFuture<'static, String>> {
            Some(Box::pin(async move { format!("{correlation}:{payload}") }))
        }
    }

    let mut registry = AsyncEffectRegistry::new();
    registry.register(Arc::new(EchoRunner));
    let (tx, mut rx) = mpsc::channel(4);
    let weak = tx.downgrade();
    let store = test_effect_store();
    spawn_effect_list_with_registry(
        vec![Effect::ModuleAsync {
            module: SmolStr::new("custom_echo"),
            actor: ObjId(7),
            correlation: 42,
            payload: "ping".to_owned(),
        }],
        &weak,
        None,
        &store,
        &registry,
    );
    let msg = rx.recv().await.expect("effect completion re-entered");
    let WorldMsg::Inbound(Inbound::Apply(WorldCommand::ModuleAsyncDone {
        module,
        actor,
        correlation,
        payload,
    })) = msg
    else {
        panic!("expected ModuleAsyncDone");
    };
    assert_eq!(module.as_str(), "custom_echo");
    assert_eq!(actor, ObjId(7));
    assert_eq!(correlation, 42);
    assert_eq!(payload, "42:ping");
}

#[test]
fn w_clock_outage_countdown_broadcasts_5_2_1_minutes_and_unfreezes_without_catch_up() {
    let mut world = World::new_minimal();
    let clock = mush_core::GameClock {
        ic_ratio: 4,
        epoch_offset: 1_000,
    };
    let mut timeline = mush_core::WorldTimeline::from_game_clock(clock);
    // Freeze at real = 100 (IC = 1_400), boot at real = 200 with 300s countdown -> unfreeze at 500.
    timeline.freeze_for_outage(100, 1_400, 200, 500);
    world.set_world_timeline(timeline);
    world.set_now(200);
    world.set_connected(GOD, true);

    let mut engine = Engine::new(world, base_registry());
    let recorder = Arc::new(RecordingGateway::default());
    let gateway: Gateway = recorder.clone();
    let mut state = OutageCountdownState::default();
    let mut out = Outbox::new();
    let mut fx = Effects::new();

    // At t = 200 (300s remaining): immediate wake for 5-minute broadcast.
    assert_eq!(
        next_outage_wake_unix_ms(&engine.world, &state, 200),
        Some(200_000)
    );
    assert!(!step_outage_countdown(
        &mut engine,
        &gateway,
        &mut state,
        200,
        &mut out,
        &mut fx
    ));
    assert!(wizard_heard(&recorder, "5 minutes remaining"));
    assert!(engine.world.world_timeline().is_frozen());
    // Next wake is at 2 minutes remaining (t = 380).
    assert_eq!(
        next_outage_wake_unix_ms(&engine.world, &state, 200),
        Some(380_000)
    );

    // At t = 380 (120s remaining): 2-minute broadcast.
    assert!(!step_outage_countdown(
        &mut engine,
        &gateway,
        &mut state,
        380,
        &mut out,
        &mut fx
    ));
    assert!(wizard_heard(&recorder, "2 minutes remaining"));
    assert_eq!(
        next_outage_wake_unix_ms(&engine.world, &state, 380),
        Some(440_000)
    );

    // At t = 440 (60s remaining): 1-minute broadcast.
    assert!(!step_outage_countdown(
        &mut engine,
        &gateway,
        &mut state,
        440,
        &mut out,
        &mut fx
    ));
    assert!(wizard_heard(&recorder, "1 minute remaining"));
    assert_eq!(
        next_outage_wake_unix_ms(&engine.world, &state, 440),
        Some(500_000)
    );

    // At t = 500 (0s remaining): unfreeze!
    assert!(step_outage_countdown(
        &mut engine,
        &gateway,
        &mut state,
        500,
        &mut out,
        &mut fx
    ));
    assert!(wizard_heard(
        &recorder,
        "Outage recovery countdown complete"
    ));
    assert!(!engine.world.world_timeline().is_frozen());
    // IC clock resumes at 1_400 with zero unseen catch-up for the 400s outage+countdown.
    assert_eq!(engine.world.world_ic_now().at, 1_400);
    assert_eq!(engine.world.game_now_unix(), 1_400);
    // 10 real seconds after unfreeze (t = 510), IC advances normally by 40s -> 1_440.
    engine.set_now(510);
    assert_eq!(engine.world.world_ic_now().at, 1_440);
    assert_eq!(engine.world.game_now_unix(), 1_440);
}

/// `W-clock` / `Q-R4` (doc 33 GG.5): the running world moves its durable checkpoint
/// forward on the cadence through the deferred-persist path, writes an overdue one
/// at once, never advances it while the clock is frozen, and counts the cadence
/// from the timeline's last checkpoint whoever wrote it.
#[tokio::test]
async fn w_clock_periodic_checkpoint_advances_the_persisted_real_unix_and_holds_while_frozen() {
    use crate::timeline::{
        OutagePolicy, apply_boot_outage_policy, hydrate_world_timeline, load_or_seed_world_timeline,
    };
    use mush_store::{InMemoryStore, WriteBatch};

    /// Persist whatever the step deferred, exactly as the loop's foot does: drain
    /// the outbox into the engine's module ops and hand them to the store.
    async fn flush(
        engine: &mut Engine,
        out: &mut Outbox,
        fx: &mut Effects,
        store: &InMemoryStore,
        serial: u64,
    ) -> usize {
        engine.finish_external_step(out, fx);
        let module_ops = engine.take_module_ops();
        let count = module_ops.len();
        if count > 0 {
            store
                .write(WriteBatch {
                    serial,
                    module_ops,
                    durable: true,
                    ..WriteBatch::default()
                })
                .await
                .unwrap();
        }
        count
    }
    async fn persisted_real_unix(
        store: &InMemoryStore,
        clock: mush_core::GameClock,
    ) -> Option<i64> {
        load_or_seed_world_timeline(store, clock)
            .await
            .unwrap()
            .checkpoint()
            .map(|cp| cp.real_unix)
    }

    let store = InMemoryStore::new();
    let clock = mush_core::GameClock {
        ic_ratio: 4,
        epoch_offset: 1_000,
    };
    let mut world = World::new_minimal();
    hydrate_world_timeline(&mut world, &store, clock)
        .await
        .unwrap();
    apply_boot_outage_policy(&mut world, &store, 1_000, OutagePolicy::default())
        .await
        .unwrap();
    assert_eq!(persisted_real_unix(&store, clock).await, Some(1_000));
    let mut engine = Engine::new(world, base_registry());
    let cadence = WorldClockCheckpointCadence::new(10);
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    assert_eq!(
        cadence.next_due_unix_ms(engine.world.world_timeline()),
        Some(1_010_000)
    );

    // Before the cadence: nothing recorded, nothing queued.
    assert_eq!(
        step_world_clock_checkpoint(&mut engine, cadence, 1_005, &mut out),
        None
    );
    assert_eq!(flush(&mut engine, &mut out, &mut fx, &store, 1).await, 0);
    assert_eq!(persisted_real_unix(&store, clock).await, Some(1_000));

    // On the cadence: one checkpoint, one op, the persisted real_unix moved.
    let cp = step_world_clock_checkpoint(&mut engine, cadence, 1_010, &mut out).expect("due");
    assert_eq!((cp.real_unix, cp.ic_unix), (1_010, 5_040));
    assert_eq!(flush(&mut engine, &mut out, &mut fx, &store, 2).await, 1);
    assert_eq!(persisted_real_unix(&store, clock).await, Some(1_010));
    assert_eq!(
        cadence.next_due_unix_ms(engine.world.world_timeline()),
        Some(1_020_000)
    );

    // A late wake writes at once, and the next is a full cadence after the write.
    assert!(step_world_clock_checkpoint(&mut engine, cadence, 1_033, &mut out).is_some());
    assert_eq!(flush(&mut engine, &mut out, &mut fx, &store, 3).await, 1);
    assert_eq!(persisted_real_unix(&store, clock).await, Some(1_033));
    assert_eq!(
        cadence.next_due_unix_ms(engine.world.world_timeline()),
        Some(1_043_000)
    );

    // Frozen: nothing is recorded or queued, however overdue.
    let mut frozen = engine.world.world_timeline().clone();
    let _ = frozen.freeze_for_outage(1_033, 5_132, 1_040, 1_340);
    engine.world.set_world_timeline(frozen);
    for now in [1_043, 1_100, 1_339] {
        assert_eq!(
            step_world_clock_checkpoint(&mut engine, cadence, now, &mut out),
            None,
            "frozen at {now}"
        );
        assert_eq!(flush(&mut engine, &mut out, &mut fx, &store, 4).await, 0);
    }
    assert_eq!(persisted_real_unix(&store, clock).await, Some(1_033));

    // Unfrozen again: the resume is itself a checkpoint (the unfreeze persists
    // it), so the step is due a full cadence after the resume, not at once.
    let mut resumed = engine.world.world_timeline().clone();
    resumed.resume_at(1_340).expect("resumes");
    engine.world.set_world_timeline(resumed);
    assert_eq!(
        cadence.next_due_unix_ms(engine.world.world_timeline()),
        Some(1_350_000)
    );
    assert_eq!(
        step_world_clock_checkpoint(&mut engine, cadence, 1_349, &mut out),
        None
    );
    let cp = step_world_clock_checkpoint(&mut engine, cadence, 1_350, &mut out).expect("due");
    assert_eq!(cp.real_unix, 1_350);
    assert_eq!(flush(&mut engine, &mut out, &mut fx, &store, 5).await, 1);
    assert_eq!(persisted_real_unix(&store, clock).await, Some(1_350));
}

/// The batch rider (`W-clock` / `Q-R4`): an authoritative batch carries the
/// checkpoint of its own second ahead of the writes it covers, so the persisted
/// checkpoint is never earlier than an IC stamp the store holds, and a restart
/// re-anchors the IC clock at the last write rather than before it. The first
/// CI run of PR #245 showed the rewind: a relay receipt `queued` before a crash
/// and `transmitted` after the re-anchored boot read `queued > transmitted`,
/// and the next boot refused the relay service state.
#[tokio::test]
async fn w_clock_a_batch_carries_the_checkpoint_of_its_second_so_a_restart_never_rewinds_the_clock()
{
    use crate::timeline::{
        OutageEvaluation, OutagePolicy, apply_boot_outage_policy, hydrate_world_timeline,
        load_or_seed_world_timeline,
    };
    use mush_store::{InMemoryStore, WriteBatch};

    /// The loop's foot-of-loop flush, with what it handed the writer applied
    /// to the store; `None` when nothing was dirty.
    async fn flush_to_store(
        engine: &mut Engine,
        store: &InMemoryStore,
        serial: &mut u64,
    ) -> Option<WriteBatch> {
        let (writer, mut writer_rx) = persist::test_store_writer_channel(Arc::new(Metrics::new()));
        let mut health = StoreHealth::new();
        let metrics = Metrics::new();
        let mut retained = RetainedSeals::new();
        let high_water = AtomicU64::new(0);
        let persistence_integrity = PersistenceIntegrityLatch::new();
        let outcome = flush_dirty(
            engine,
            &writer,
            serial,
            true,
            &mut health,
            &metrics,
            &mut retained,
            &high_water,
            &crate::quarantine::QuarantineSpool::in_memory(),
            &persistence_integrity,
        )
        .await;
        match outcome {
            FlushOutcome::Clean => None,
            FlushOutcome::Queued { .. } => {
                let StoreJob::Write { batch, .. } = writer_rx.try_recv().unwrap() else {
                    panic!("a queued flush hands the writer a write job");
                };
                store.write(batch.clone()).await.unwrap();
                Some(batch)
            }
            FlushOutcome::Refused(detail) | FlushOutcome::Rejected(detail) => {
                panic!("the flush must queue: {detail}")
            }
            FlushOutcome::WriterGone => panic!("the flush must queue: writer gone"),
        }
    }
    /// Positions of the timeline state ops in a batch's module ops.
    fn timeline_ops(batch: &WriteBatch) -> Vec<usize> {
        batch
            .module_ops
            .iter()
            .enumerate()
            .filter(|(_, op)| op.ns == mush_core::WORLD_TIMELINE_NAMESPACE)
            .map(|(i, _)| i)
            .collect()
    }
    async fn persisted_checkpoint(
        store: &InMemoryStore,
        clock: mush_core::GameClock,
    ) -> Option<(i64, i64)> {
        load_or_seed_world_timeline(store, clock)
            .await
            .unwrap()
            .checkpoint()
            .map(|cp| (cp.real_unix, cp.ic_unix))
    }

    let store = InMemoryStore::new();
    let clock = mush_core::GameClock {
        ic_ratio: 4,
        epoch_offset: 1_000,
    };
    let mut world = World::new_minimal();
    hydrate_world_timeline(&mut world, &store, clock)
        .await
        .unwrap();
    apply_boot_outage_policy(&mut world, &store, 1_000, OutagePolicy::default())
        .await
        .unwrap();
    assert_eq!(
        persisted_checkpoint(&store, clock).await,
        Some((1_000, 5_000))
    );
    let mut engine = Engine::new(world, base_registry());
    let mut serial = 0;

    // The minimal world's seed objects are legacy dirtiness: the boot's own
    // second flushes them, and that batch carries no checkpoint — the boot's
    // is this second's.
    engine.set_now(1_000);
    let batch = flush_to_store(&mut engine, &store, &mut serial)
        .await
        .expect("the seed objects");
    assert_eq!(timeline_ops(&batch), Vec::<usize>::new());
    assert!(
        flush_to_store(&mut engine, &store, &mut serial)
            .await
            .is_none(),
        "nothing dirty, no batch"
    );

    // A write seven seconds on: the batch carries the checkpoint of its second,
    // ahead of the write, and the store now holds it.
    engine.set_now(1_007);
    assert_eq!(engine.world.world_ic_now().at, 5_028);
    engine
        .world
        .create(ObjKind::Thing, "a receipt", GOD, Some(LIMBO))
        .expect("creates");
    let batch = flush_to_store(&mut engine, &store, &mut serial)
        .await
        .expect("a batch");
    assert_eq!(timeline_ops(&batch), vec![0]);
    assert_eq!(
        persisted_checkpoint(&store, clock).await,
        Some((1_007, 5_028))
    );

    // A second batch in the same second carries none: one per second at most.
    engine
        .world
        .create(ObjKind::Thing, "another", GOD, Some(LIMBO))
        .expect("creates");
    let batch = flush_to_store(&mut engine, &store, &mut serial)
        .await
        .expect("a batch");
    assert_eq!(timeline_ops(&batch), Vec::<usize>::new());

    // The crash, twelve seconds after that write: the boot measures the real
    // downtime and re-anchors the IC clock AT the write's second — never
    // before a stamp the store holds. Without the rider it read the boot's
    // checkpoint (19 s lost) and resumed at 5_000, 28 IC seconds before the
    // write it had persisted.
    let mut rebooted = World::new_minimal();
    hydrate_world_timeline(&mut rebooted, &store, clock)
        .await
        .unwrap();
    let evaluation =
        apply_boot_outage_policy(&mut rebooted, &store, 1_019, OutagePolicy::default())
            .await
            .unwrap();
    assert_eq!(
        evaluation,
        OutageEvaluation::SilentReanchor { lost_secs: 12 }
    );
    assert_eq!(rebooted.world_ic_now().at, 5_028);
    assert_eq!(
        persisted_checkpoint(&store, clock).await,
        Some((1_019, 5_028))
    );

    // Frozen: a batch carries none, however far the second has moved; the
    // checkpoint is the pre-outage moment the countdown holds the world at.
    let mut frozen = engine.world.world_timeline().clone();
    let _ = frozen.freeze_for_outage(1_007, 5_028, 1_030, 1_330);
    engine.world.set_world_timeline(frozen);
    engine.set_now(1_030);
    engine
        .world
        .create(ObjKind::Thing, "while frozen", GOD, Some(LIMBO))
        .expect("creates");
    let batch = flush_to_store(&mut engine, &store, &mut serial)
        .await
        .expect("a batch");
    assert_eq!(timeline_ops(&batch), Vec::<usize>::new());
    assert_eq!(
        engine
            .world
            .world_timeline()
            .checkpoint()
            .map(|cp| cp.real_unix),
        Some(1_007)
    );
}

/// The live loop: an idle world task with a one-second cadence writes its
/// checkpoint to the store with no other wake, so a restart measures its outage
/// from the last second the world ran rather than from the previous boot.
#[tokio::test]
async fn w_clock_live_world_task_writes_its_checkpoint_on_the_cadence() {
    use crate::timeline::{
        OutagePolicy, apply_boot_outage_policy, hydrate_world_timeline, load_or_seed_world_timeline,
    };

    let deps = ServerDeps::in_memory();
    let store = Arc::clone(&deps.store);
    let clock = mush_core::GameClock::default();
    let boot = unix_now_s();
    let mut world = World::new_minimal();
    hydrate_world_timeline(&mut world, &*store, clock)
        .await
        .unwrap();
    apply_boot_outage_policy(&mut world, &*store, boot, OutagePolicy::default())
        .await
        .unwrap();
    let handles = spawn_persist_with_gateway_production(
        Engine::new(world, base_registry()),
        deps,
        None,
        DEFAULT_DECISION_PROVIDER_WORKERS,
        1,
        |_| Arc::new(BlackholeGateway),
    )
    .expect("production spawn");

    let mut advanced = None;
    for _ in 0..80 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let real = load_or_seed_world_timeline(&*store, clock)
            .await
            .unwrap()
            .checkpoint()
            .map(|cp| cp.real_unix);
        if real.is_some_and(|real| real > boot) {
            advanced = real;
            break;
        }
    }
    assert!(
        advanced.is_some(),
        "the idle world task wrote no checkpoint past the boot's within eight seconds"
    );
    assert!(
        handles
            .handle
            .inbound(Inbound::Control(ControlMsg::Shutdown))
            .await
    );
    handles.world_join.await.unwrap();
    handles.writer_join.await.unwrap();
    handles.outbox_join.await.unwrap();
}

#[test]
fn w_clock_outage_countdown_admits_inspection_and_ooc_while_refusing_movement_and_mutations() {
    let mut world = World::new_minimal();
    let _exit = world
        .create(ObjKind::Exit, "North;n", GOD, Some(LIMBO))
        .unwrap();

    let mut timeline = world.world_timeline().clone();
    timeline.freeze_for_outage(100, 1_000, 200, 500);
    world.set_world_timeline(timeline);

    // Inspection, OOC communication, and @tacsim/time are allowed during the countdown.
    for allowed in [
        "look",
        "examine here",
        "WHO",
        "ooc Back online!",
        "+ooc status?",
        "page God=Ready",
        "=public Checking in",
        "@tacsim/time",
        "@tacsim/time resume",
    ] {
        assert_eq!(
            admit_outage_countdown_msg(&world, &WorldMsg::Inbound(cmd(allowed))),
            Admission::Allow,
            "expected {allowed:?} to be allowed during outage countdown"
        );
    }

    // Room exits and world mutations are refused during the countdown.
    for refused in [
        "north",
        "n",
        "go north",
        "@dig Pit",
        "get box",
        "attack target",
    ] {
        assert_eq!(
            admit_outage_countdown_msg(&world, &WorldMsg::Inbound(cmd(refused))),
            Admission::Refuse(RefusalReason::OutageCountdown),
            "expected {refused:?} to be refused during outage countdown"
        );
    }
}

//! Composition-frozen discovery and leasing of durable delayed decisions.
//!
//! A source is one owner-local pending/prompt/deadline family bundle plus the
//! exact authoritative action used to record operational leases. Discovery
//! reads the typed families directly in bounded canonical pages; provider
//! callbacks cannot hide another queue or start work before a lease commits.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use smol_str::SmolStr;
use thiserror::Error;

use crate::action::{
    ActionInvocation, ActionInvocationDraft, ActionMetadataError, ActionOfferId,
    ActionProviderLookupError, ActionProviderRegistry, CanonicalHash, CanonicalHashError, GuardSet,
    IdempotencyKey, IdempotencyKeyText, IdempotencyKeyTextError, IdempotencyScope, IngressEnvelope,
    IngressPayload, IngressSurfaceVersion, PerceivedRef, PrincipalId, RevisionSet,
    ScopedIdempotencyKey,
};
use crate::action_state::{ActionStateError, ActionTxn, StateFamilyRegistry};
use crate::clock::ClockStamp;
use crate::decision::{
    DecisionActionBinding, DecisionAdmissionIndex, DecisionAttemptFailureKind,
    DecisionAttemptTransition, DecisionCancellationTransition, DecisionCaptureFamilies,
    DecisionCompactionTransition, DecisionCompletionFence, DecisionControllerId, DecisionError,
    DecisionFallbackReason, DecisionFallbackTransition, DecisionId, DecisionKeyRing,
    DecisionLeaseTransition, DecisionPrompt, DecisionResolvingTransition, DecisionResponse,
    DecisionTerminalOutcome, DecisionTerminalReceipt, DecisionTerminalTransition, DecisionWake,
    DecisionWorkerLease, FallbackPolicyRef, MAX_DECISION_ATTEMPTS, MAX_DECISION_RECOVERY_PAGE,
    PendingDecision, PendingDecisionStatus, PendingOfferWitness,
};
use crate::durable_schedule::ScheduleId;
use crate::mechanics::{
    CanonicalEncodingError, CanonicalId, CanonicalIdError, DefinitionDigest, DefinitionLookupError,
    DefinitionRef, MechanicalOwnerReadiness, MechanicalOwnerReadinessError, OwnerId,
};
use crate::offer::{OfferExecutionContext, OfferSubmission};

/// Maximum composed owners that may publish delayed-decision families.
pub const MAX_DECISION_SOURCES: usize = 256;
/// Largest owner-selected lease interval in units of the decision clock.
pub const MAX_DECISION_LEASE_DURATION: i64 = 3_600;

#[derive(Clone)]
struct DecisionSourceEntry {
    owner: OwnerId,
    lease_action: DefinitionRef,
    families: DecisionCaptureFamilies,
    wake_adapter: CanonicalId,
    lease_duration: i64,
    fallback_adapter: std::sync::Arc<dyn DecisionFallbackAdapter>,
}

impl std::fmt::Debug for DecisionSourceEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DecisionSourceEntry")
            .field("owner", &self.owner)
            .field("lease_action", &self.lease_action)
            .field("families", &self.families)
            .field("wake_adapter", &self.wake_adapter)
            .field("fallback_adapter", self.fallback_adapter.identity())
            .finish()
    }
}

/// Mutable composition-only delayed-decision source table.
#[derive(Default)]
pub struct DecisionSourceRegistryBuilder {
    entries: BTreeMap<OwnerId, DecisionSourceEntry>,
}

/// Projected-only deterministic selection returned by an E-owned fallback
/// policy adapter. The kernel supplies the authenticated prompt token.
#[derive(Clone, Debug, PartialEq)]
pub struct DecisionFallbackSelection {
    pub offer: ActionOfferId,
    pub targets: Vec<PerceivedRef>,
    pub parameters: Value,
}

/// One synchronous composition-branded fallback policy surface. It receives
/// no private pending record, truth subject, revision witnesses, guards, RNG,
/// world reference, or async capability.
pub trait DecisionFallbackAdapter: Send + Sync {
    fn identity(&self) -> &CanonicalId;

    fn select(
        &self,
        policy: &FallbackPolicyRef,
        prompt: &DecisionPrompt,
    ) -> Result<DecisionFallbackSelection, String>;
}

impl DecisionSourceRegistryBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn register(
        &mut self,
        owner: OwnerId,
        lease_action: DefinitionRef,
        families: DecisionCaptureFamilies,
        wake_adapter: CanonicalId,
        lease_duration: i64,
        fallback_adapter: std::sync::Arc<dyn DecisionFallbackAdapter>,
    ) -> Result<(), DecisionSourceRegistryError> {
        if self.entries.len() >= MAX_DECISION_SOURCES {
            return Err(DecisionSourceRegistryError::SourceLimit {
                max: MAX_DECISION_SOURCES,
            });
        }
        if families.owner() != &owner || lease_action.key().owner != owner {
            return Err(DecisionSourceRegistryError::OwnerMismatch);
        }
        if lease_action.key().kind.as_str() != "action" {
            return Err(DecisionSourceRegistryError::ActionKind);
        }
        if !(1..=MAX_DECISION_LEASE_DURATION).contains(&lease_duration) {
            return Err(DecisionSourceRegistryError::LeaseDuration {
                actual: lease_duration,
                max: MAX_DECISION_LEASE_DURATION,
            });
        }
        if self.entries.contains_key(&owner) {
            return Err(DecisionSourceRegistryError::Duplicate { owner });
        }
        self.entries.insert(
            owner.clone(),
            DecisionSourceEntry {
                owner,
                lease_action,
                families,
                wake_adapter,
                lease_duration,
                fallback_adapter,
            },
        );
        Ok(())
    }

    pub(crate) fn freeze_for_composition(
        self,
        module_owners: &BTreeSet<OwnerId>,
        actions: &ActionProviderRegistry,
        state_families: &StateFamilyRegistry,
        owner_readiness: MechanicalOwnerReadiness,
    ) -> Result<DecisionSourceRegistry, DecisionSourceRegistryError> {
        for entry in self.entries.values() {
            if !module_owners.contains(&entry.owner) {
                return Err(DecisionSourceRegistryError::UnknownOwner {
                    owner: entry.owner.clone(),
                });
            }
            actions.get_exact(&entry.lease_action).map_err(|source| {
                DecisionSourceRegistryError::Action {
                    owner: entry.owner.clone(),
                    source: Box::new(source),
                }
            })?;
            state_families
                .validate_handle(entry.families.pending())
                .and_then(|()| state_families.validate_handle(entry.families.prompts()))
                .and_then(|()| state_families.validate_handle(entry.families.deadlines()))
                .and_then(|()| state_families.validate_handle(entry.families.terminal_receipts()))
                .map_err(|source| DecisionSourceRegistryError::Family {
                    owner: entry.owner.clone(),
                    source: Box::new(source),
                })?;
        }
        let fingerprint = DefinitionDigest::for_canonical_value(
            "decision_source_registry.v1",
            &Value::Array(
                self.entries
                    .values()
                    .map(|entry| {
                        json!({
                            "owner": entry.owner,
                            "lease_action": entry.lease_action,
                            "pending": entry.families.pending().descriptor().canonical_value(),
                            "prompts": entry.families.prompts().descriptor().canonical_value(),
                            "deadlines": entry.families.deadlines().descriptor().canonical_value(),
                            "terminal_receipts": entry.families.terminal_receipts().descriptor().canonical_value(),
                            "wake_adapter": entry.wake_adapter,
                            "lease_duration": entry.lease_duration,
                            "fallback_adapter": entry.fallback_adapter.identity(),
                        })
                    })
                    .collect(),
            ),
        )?;
        Ok(DecisionSourceRegistry {
            entries: self.entries,
            fingerprint,
            owner_readiness,
        })
    }
}

/// Opaque cursor over canonical owner/family key order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionSourceCursor {
    owner: OwnerId,
    after_encoded_key: SmolStr,
}

/// One durable record eligible for a new provider lease.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionDispatchCandidate {
    registry_fingerprint: DefinitionDigest,
    owner: OwnerId,
    encoded_key: SmolStr,
    wake: DecisionWake,
    controller: DecisionControllerId,
    attempt: u8,
}

/// One durable Resolving record whose bound ordinary action status must be
/// reconciled before the decision can terminalize.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionReconciliationCandidate {
    fence: DecisionCompletionFence,
    binding: DecisionActionBinding,
    action_idempotency: ScopedIdempotencyKey,
    outcome: DecisionTerminalOutcome,
}

/// One durable FallbackReady record routed to its composition-branded,
/// projected-only deterministic selector.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionFallbackCandidate {
    registry_fingerprint: DefinitionDigest,
    owner: OwnerId,
    fence: DecisionCompletionFence,
}

/// Exact Ready/Leased record whose custody must move to deterministic
/// fallback before any selector or ordinary action runs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionFallbackTriggerCandidate {
    registry_fingerprint: DefinitionDigest,
    owner: OwnerId,
    wake: DecisionWake,
    controller: DecisionControllerId,
    attempt: u8,
    lease_until: ClockStamp,
    reason: DecisionFallbackReason,
}

/// One exact terminal record eligible for replacement by its bounded durable
/// attribution receipt after any selected action is independently proven
/// finalized by the authoritative store.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionCompactionCandidate {
    registry_fingerprint: DefinitionDigest,
    owner: OwnerId,
    receipt: DecisionTerminalReceipt,
}

impl DecisionCompactionCandidate {
    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn receipt(&self) -> &DecisionTerminalReceipt {
        &self.receipt
    }

    #[must_use]
    pub const fn action_idempotency(&self) -> &ScopedIdempotencyKey {
        &self.receipt.action_idempotency
    }

    #[must_use]
    pub const fn action_binding(&self) -> Option<&DecisionActionBinding> {
        self.receipt.action_binding.as_ref()
    }
}

impl DecisionFallbackTriggerCandidate {
    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn reason(&self) -> DecisionFallbackReason {
        self.reason
    }
}

impl DecisionFallbackCandidate {
    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn fence(&self) -> &DecisionCompletionFence {
        &self.fence
    }
}

impl DecisionReconciliationCandidate {
    #[must_use]
    pub const fn fence(&self) -> &DecisionCompletionFence {
        &self.fence
    }

    #[must_use]
    pub const fn binding(&self) -> &DecisionActionBinding {
        &self.binding
    }

    #[must_use]
    pub const fn action_idempotency(&self) -> &ScopedIdempotencyKey {
        &self.action_idempotency
    }

    #[must_use]
    pub const fn outcome(&self) -> DecisionTerminalOutcome {
        self.outcome
    }
}

impl DecisionDispatchCandidate {
    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn wake(&self) -> DecisionWake {
        self.wake
    }

    #[must_use]
    pub const fn controller(&self) -> &DecisionControllerId {
        &self.controller
    }

    #[must_use]
    pub const fn attempt(&self) -> u8 {
        self.attempt
    }
}

/// Bounded result from one canonical recovery/discovery step.
#[derive(Debug)]
pub struct DecisionSourcePage {
    candidates: Vec<DecisionDispatchCandidate>,
    reconciliations: Vec<DecisionReconciliationCandidate>,
    fallback_triggers: Vec<DecisionFallbackTriggerCandidate>,
    fallbacks: Vec<DecisionFallbackCandidate>,
    compactions: Vec<DecisionCompactionCandidate>,
    admission_index: DecisionAdmissionIndex,
    records_scanned: usize,
    next_cursor: Option<DecisionSourceCursor>,
}

impl DecisionSourcePage {
    #[must_use]
    pub fn candidates(&self) -> &[DecisionDispatchCandidate] {
        &self.candidates
    }

    #[must_use]
    pub fn reconciliations(&self) -> &[DecisionReconciliationCandidate] {
        &self.reconciliations
    }

    #[must_use]
    pub fn fallback_triggers(&self) -> &[DecisionFallbackTriggerCandidate] {
        &self.fallback_triggers
    }

    #[must_use]
    pub fn fallbacks(&self) -> &[DecisionFallbackCandidate] {
        &self.fallbacks
    }

    #[must_use]
    pub fn compactions(&self) -> &[DecisionCompactionCandidate] {
        &self.compactions
    }

    #[must_use]
    pub const fn admission_index(&self) -> &DecisionAdmissionIndex {
        &self.admission_index
    }

    #[must_use]
    pub const fn records_scanned(&self) -> usize {
        self.records_scanned
    }

    #[must_use]
    pub const fn next_cursor(&self) -> Option<&DecisionSourceCursor> {
        self.next_cursor.as_ref()
    }
}

/// Exact coordinator material for one operational lease action.
#[derive(Debug)]
pub struct DecisionDispatchPlan {
    action: DefinitionRef,
    invocation: ActionInvocation,
    guards: GuardSet,
    transition: DecisionLeaseTransition,
}

/// Exact coordinator material for the owner-local Leased-to-Resolving
/// lifecycle action. This action records only the binding of an already
/// durable selected ordinary action.
#[derive(Debug)]
pub struct DecisionResolvingPlan {
    action: DefinitionRef,
    invocation: ActionInvocation,
    guards: GuardSet,
    transition: DecisionResolvingTransition,
}

/// Exact coordinator material for committing one typed provider-attempt
/// result without trusting diagnostic text.
#[derive(Debug)]
pub struct DecisionAttemptPlan {
    action: DefinitionRef,
    invocation: ActionInvocation,
    guards: GuardSet,
    transition: DecisionAttemptTransition,
}

/// Exact coordinator material for committing Ready/Leased-to-FallbackReady
/// custody before deterministic selection.
#[derive(Debug)]
pub struct DecisionFallbackTriggerPlan {
    action: DefinitionRef,
    invocation: ActionInvocation,
    guards: GuardSet,
    transition: DecisionFallbackTransition,
}

/// Exact coordinator material for a cancellable-to-Terminal(cancelled)
/// owner-local lifecycle action.
#[derive(Debug)]
pub struct DecisionCancellationPlan {
    action: DefinitionRef,
    invocation: ActionInvocation,
    guards: GuardSet,
    transition: DecisionCancellationTransition,
}

/// Exact coordinator material for an owner-local Resolving-to-Terminal
/// lifecycle action.
#[derive(Debug)]
pub struct DecisionTerminalPlan {
    action: DefinitionRef,
    invocation: ActionInvocation,
    guards: GuardSet,
    transition: DecisionTerminalTransition,
}

/// Exact coordinator material for atomically replacing one terminal decision
/// bundle with its compact durable attribution receipt.
#[derive(Debug)]
pub struct DecisionCompactionPlan {
    action: DefinitionRef,
    invocation: ActionInvocation,
    guards: GuardSet,
    transition: DecisionCompactionTransition,
}

/// Authenticated provider selection after current offer regeneration and
/// prompt-time witness restoration. This is the only delayed-decision value
/// accepted by the ordinary resolved authoritative coordinator.
#[derive(Debug)]
pub struct DecisionResponseResolution {
    resolved: crate::offer::ResolvedAuthoritativeOffer,
    binding: DecisionActionBinding,
}

impl DecisionResponseResolution {
    #[must_use]
    pub const fn binding(&self) -> &DecisionActionBinding {
        &self.binding
    }

    #[must_use]
    pub const fn resolved(&self) -> &crate::offer::ResolvedAuthoritativeOffer {
        &self.resolved
    }

    #[must_use]
    pub fn into_resolved(self) -> crate::offer::ResolvedAuthoritativeOffer {
        self.resolved
    }
}

impl DecisionDispatchPlan {
    pub fn into_parts(
        self,
    ) -> (
        DefinitionRef,
        ActionInvocation,
        GuardSet,
        DecisionLeaseTransition,
    ) {
        (self.action, self.invocation, self.guards, self.transition)
    }
}

impl DecisionResolvingPlan {
    pub fn into_parts(
        self,
    ) -> (
        DefinitionRef,
        ActionInvocation,
        GuardSet,
        DecisionResolvingTransition,
    ) {
        (self.action, self.invocation, self.guards, self.transition)
    }
}

impl DecisionAttemptPlan {
    pub fn into_parts(
        self,
    ) -> (
        DefinitionRef,
        ActionInvocation,
        GuardSet,
        DecisionAttemptTransition,
    ) {
        (self.action, self.invocation, self.guards, self.transition)
    }
}

impl DecisionFallbackTriggerPlan {
    pub fn into_parts(
        self,
    ) -> (
        DefinitionRef,
        ActionInvocation,
        GuardSet,
        DecisionFallbackTransition,
    ) {
        (self.action, self.invocation, self.guards, self.transition)
    }
}

impl DecisionCancellationPlan {
    pub fn into_parts(
        self,
    ) -> (
        DefinitionRef,
        ActionInvocation,
        GuardSet,
        DecisionCancellationTransition,
    ) {
        (self.action, self.invocation, self.guards, self.transition)
    }
}

impl DecisionTerminalPlan {
    pub fn into_parts(
        self,
    ) -> (
        DefinitionRef,
        ActionInvocation,
        GuardSet,
        DecisionTerminalTransition,
    ) {
        (self.action, self.invocation, self.guards, self.transition)
    }
}

impl DecisionCompactionPlan {
    pub fn into_parts(
        self,
    ) -> (
        DefinitionRef,
        ActionInvocation,
        GuardSet,
        DecisionCompactionTransition,
    ) {
        (self.action, self.invocation, self.guards, self.transition)
    }
}

/// Frozen declarative delayed-decision sources.
pub struct DecisionSourceRegistry {
    entries: BTreeMap<OwnerId, DecisionSourceEntry>,
    fingerprint: DefinitionDigest,
    owner_readiness: MechanicalOwnerReadiness,
}

impl DecisionSourceRegistry {
    #[must_use]
    pub const fn fingerprint(&self) -> DefinitionDigest {
        self.fingerprint
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Exact outbox adapter identities required by the composed sources.
    #[must_use]
    pub fn wake_adapters(&self) -> Vec<(OwnerId, CanonicalId)> {
        self.entries
            .values()
            .map(|entry| (entry.owner.clone(), entry.wake_adapter.clone()))
            .collect()
    }

    pub(crate) fn ingress_for_candidate(
        &self,
        candidate: &DecisionDispatchCandidate,
        idempotency_epoch: u64,
    ) -> Result<IngressEnvelope, DecisionLeaseIngressError> {
        let entry = self
            .entry_for_candidate(candidate)
            .map_err(DecisionLeaseIngressError::Candidate)?;
        decision_lease_ingress(&entry.lease_action, candidate, idempotency_epoch)
    }

    pub(crate) fn response_ingress(
        &self,
        engine: &crate::engine::Engine,
        keys: &DecisionKeyRing,
        fence: &DecisionCompletionFence,
        response: &DecisionResponse,
    ) -> Result<IngressEnvelope, DecisionResponseIngressError> {
        let (pending, _prompt, _offered) =
            self.validate_response_fence(engine, keys, fence, response)?;
        canonical_response_ingress(pending, fence, response)
    }

    pub(crate) fn fallback_ingress(
        &self,
        engine: &crate::engine::Engine,
        keys: &DecisionKeyRing,
        candidate: &DecisionFallbackCandidate,
        response: &DecisionResponse,
    ) -> Result<IngressEnvelope, DecisionResponseIngressError> {
        let (pending, _prompt, _offered) =
            self.validate_fallback_response(engine, keys, candidate, response)?;
        canonical_response_ingress(pending, candidate.fence(), response)
    }

    fn validate_fallback_response<'a>(
        &'a self,
        engine: &'a crate::engine::Engine,
        keys: &DecisionKeyRing,
        candidate: &DecisionFallbackCandidate,
        response: &DecisionResponse,
    ) -> Result<
        (
            &'a PendingDecision,
            &'a crate::decision::DecisionPromptRecord,
            &'a PendingOfferWitness,
        ),
        DecisionResponseIngressError,
    > {
        response.validate()?;
        if candidate.registry_fingerprint != self.fingerprint
            || candidate.owner != candidate.fence.controller().owner
        {
            return Err(DecisionResponseIngressError::StaleCompletion);
        }
        let entry = self.entries.get(&candidate.owner).ok_or_else(|| {
            DecisionResponseIngressError::SourceUnavailable {
                owner: candidate.owner.clone(),
            }
        })?;
        self.owner_readiness.require_ready(&entry.owner)?;
        let pending = engine
            .action_state()
            .get(entry.families.pending(), &candidate.fence.decision())?
            .ok_or(DecisionResponseIngressError::MissingDecision)?;
        let prompt = engine
            .action_state()
            .get(entry.families.prompts(), &candidate.fence.decision())?
            .ok_or(DecisionResponseIngressError::MissingPrompt)?;
        pending.validate_prompt_pair(prompt, keys)?;
        let PendingDecisionStatus::FallbackReady {
            attempt,
            lease_until,
            ..
        } = &pending.status
        else {
            return Err(DecisionResponseIngressError::StaleCompletion);
        };
        if &pending.controller != candidate.fence.controller()
            || pending.revision != candidate.fence.revision()
            || *attempt != candidate.fence.attempt()
            || lease_until != candidate.fence.lease_until()
            || response.token != prompt.prompt.token
        {
            return Err(DecisionResponseIngressError::StaleCompletion);
        }
        let offered = pending
            .offered
            .iter()
            .find(|offered| offered.offer == response.offer)
            .ok_or(DecisionResponseIngressError::OfferNotPrompted)?;
        Ok((pending, prompt, offered))
    }

    /// Run one deterministic fallback selector against only the exact
    /// persisted projected prompt. Kernel linkage and bounds are revalidated
    /// before and after the E-owned callback.
    pub(crate) fn select_fallback(
        &self,
        engine: &crate::engine::Engine,
        keys: &DecisionKeyRing,
        candidate: &DecisionFallbackCandidate,
    ) -> Result<DecisionResponse, DecisionFallbackSelectionError> {
        if candidate.registry_fingerprint != self.fingerprint
            || candidate.owner != candidate.fence.controller().owner
        {
            return Err(DecisionFallbackSelectionError::StaleCandidate);
        }
        let entry = self.entries.get(&candidate.owner).ok_or_else(|| {
            DecisionFallbackSelectionError::SourceUnavailable {
                owner: candidate.owner.clone(),
            }
        })?;
        self.owner_readiness.require_ready(&entry.owner)?;
        let pending = engine
            .action_state()
            .get(entry.families.pending(), &candidate.fence.decision())?
            .ok_or(DecisionFallbackSelectionError::MissingDecision)?;
        let prompt = engine
            .action_state()
            .get(entry.families.prompts(), &candidate.fence.decision())?
            .ok_or(DecisionFallbackSelectionError::MissingPrompt)?;
        pending.validate_prompt_pair(prompt, keys)?;
        let PendingDecisionStatus::FallbackReady {
            attempt,
            lease_until,
            ..
        } = &pending.status
        else {
            return Err(DecisionFallbackSelectionError::StaleCandidate);
        };
        if &pending.controller != candidate.fence.controller()
            || pending.revision != candidate.fence.revision()
            || *attempt != candidate.fence.attempt()
            || lease_until != candidate.fence.lease_until()
        {
            return Err(DecisionFallbackSelectionError::StaleCandidate);
        }
        engine
            .registry
            .definitions()
            .ok_or(DecisionFallbackSelectionError::RegistryUnavailable)?
            .get_exact(&pending.fallback.0)
            .map_err(|source| DecisionFallbackSelectionError::Definition(Box::new(source)))?;
        let selected = entry
            .fallback_adapter
            .select(&pending.fallback, &prompt.prompt)
            .map_err(|detail| {
                DecisionFallbackSelectionError::Adapter(bounded_adapter_detail(&detail))
            })?;
        if !prompt
            .prompt
            .actions
            .iter()
            .any(|action| action.offer() == selected.offer)
        {
            return Err(DecisionFallbackSelectionError::OfferNotPrompted);
        }
        let response = DecisionResponse {
            token: prompt.prompt.token,
            offer: selected.offer,
            targets: selected.targets,
            parameters: selected.parameters,
        };
        response.validate()?;
        Ok(response)
    }

    /// Construct stable owner-local ingress for recording the binding of a
    /// selected action that already has durable custody. Time is deliberately
    /// absent: an exact response authorized before lease expiry may finish its
    /// ordinary commit after expiry and must still be reconciled.
    pub(crate) fn resolving_ingress(
        &self,
        engine: &crate::engine::Engine,
        fence: &DecisionCompletionFence,
        binding: &DecisionActionBinding,
        outcome: DecisionTerminalOutcome,
        idempotency_epoch: u64,
    ) -> Result<IngressEnvelope, DecisionResolvingIngressError> {
        let entry = self.entry_for_fence(fence)?;
        self.validate_resolving_fence(engine, entry, fence, outcome)?;
        decision_resolving_ingress(
            &entry.lease_action,
            fence,
            binding,
            outcome,
            idempotency_epoch,
        )
    }

    pub(crate) fn attempt_failure_ingress(
        &self,
        engine: &crate::engine::Engine,
        fence: &DecisionCompletionFence,
        kind: DecisionAttemptFailureKind,
        max_attempts: u8,
        idempotency_epoch: u64,
    ) -> Result<IngressEnvelope, DecisionAttemptIngressError> {
        let entry = self.entry_for_attempt_fence(fence)?;
        self.validate_attempt_fence(engine, entry, fence)?;
        decision_attempt_ingress(
            &entry.lease_action,
            fence,
            kind,
            max_attempts,
            idempotency_epoch,
        )
    }

    /// Convert an otherwise dispatchable record into an exact provider-
    /// unavailable fallback trigger without starting a worker.
    pub(crate) fn fallback_trigger_for_dispatch(
        &self,
        engine: &crate::engine::Engine,
        candidate: &DecisionDispatchCandidate,
    ) -> Result<DecisionFallbackTriggerCandidate, DecisionFallbackTriggerError> {
        let entry = self.entry_for_candidate(candidate)?;
        let pending = engine
            .action_state()
            .get(entry.families.pending(), &candidate.wake.id)?
            .ok_or(DecisionFallbackTriggerError::MissingDecision)?;
        pending.validate_wake(&candidate.wake)?;
        let now = engine.clock_now(&pending.deadline.clock)?;
        if eligible_attempt(pending, &now) != Some(candidate.attempt) {
            return Err(DecisionFallbackTriggerError::StaleCandidate);
        }
        fallback_trigger_candidate(
            self.fingerprint,
            &entry.owner,
            pending,
            DecisionFallbackReason::ProviderUnavailable,
        )
        .map_err(Into::into)
    }

    pub(crate) fn fallback_trigger_ingress(
        &self,
        engine: &crate::engine::Engine,
        candidate: &DecisionFallbackTriggerCandidate,
        idempotency_epoch: u64,
    ) -> Result<IngressEnvelope, DecisionFallbackTriggerError> {
        let (entry, _) = self.validate_fallback_trigger(engine, candidate)?;
        decision_fallback_trigger_ingress(&entry.lease_action, candidate, idempotency_epoch)
    }

    pub(crate) fn plan_fallback_trigger(
        &self,
        engine: &crate::engine::Engine,
        candidate: &DecisionFallbackTriggerCandidate,
        ingress: &IngressEnvelope,
    ) -> Result<DecisionFallbackTriggerPlan, DecisionFallbackTriggerPlanError> {
        let (entry, pending) = self.validate_fallback_trigger(engine, candidate)?;
        let expected = decision_fallback_trigger_ingress(
            &entry.lease_action,
            candidate,
            ingress.idempotency().epoch(),
        )?;
        if &expected != ingress {
            return Err(DecisionFallbackTriggerPlanError::IngressMismatch);
        }
        let now = engine.clock_now(&pending.deadline.clock)?;
        let transition =
            pending.plan_fallback_ready(candidate.wake.revision, &now, candidate.reason)?;
        if transition.fence().attempt() != candidate.attempt
            || transition.fence().lease_until() != &candidate.lease_until
        {
            return Err(DecisionFallbackTriggerPlanError::StaleCandidate);
        }
        let offered_action = ActionOfferId(CanonicalHash::for_canonical_value(
            "decision_fallback_trigger_offer.v1",
            1,
            &json!({
                "action": entry.lease_action,
                "decision": candidate.wake.id,
                "revision": candidate.wake.revision,
                "attempt": candidate.attempt,
                "lease_until": candidate.lease_until,
                "reason": candidate.reason,
            }),
            32 * 1024,
        )?);
        let invocation = ActionInvocation::new(ActionInvocationDraft {
            request: ingress.request(),
            principal: ingress.principal().clone(),
            actor: pending.actor,
            offered_action,
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters: json!({
                "decision": candidate.wake.id,
                "revision": candidate.wake.revision,
                "attempt": candidate.attempt,
                "phase": "fallback_ready",
                "reason": candidate.reason,
            }),
            idempotency: ingress.idempotency().clone(),
        })?;
        let definitions = engine
            .registry
            .definitions()
            .ok_or(DecisionFallbackTriggerPlanError::RegistryUnavailable)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())?;
        guards.add_definition(entry.lease_action.clone(), definitions)?;
        self.owner_readiness.require_ready(&entry.owner)?;
        Ok(DecisionFallbackTriggerPlan {
            action: entry.lease_action.clone(),
            invocation,
            guards: guards.finish()?,
            transition,
        })
    }

    pub(crate) fn seed_fallback_trigger(
        &self,
        candidate: &DecisionFallbackTriggerCandidate,
        transition: DecisionFallbackTransition,
        txn: &mut ActionTxn<'_>,
    ) -> Result<DecisionCompletionFence, DecisionFallbackTriggerPlanError> {
        let entry = self.entries.get(&candidate.owner).ok_or(
            DecisionFallbackTriggerError::SourceUnavailable {
                owner: candidate.owner.clone(),
            },
        )?;
        transition.stage(txn, &entry.families).map_err(Into::into)
    }

    pub(crate) fn cancellation_ingress(
        &self,
        engine: &crate::engine::Engine,
        controller: &DecisionControllerId,
        decision: DecisionId,
        revision: u64,
        idempotency_epoch: u64,
    ) -> Result<IngressEnvelope, DecisionCancellationIngressError> {
        let entry = self.entries.get(&controller.owner).ok_or_else(|| {
            DecisionCancellationIngressError::SourceUnavailable {
                owner: controller.owner.clone(),
            }
        })?;
        self.owner_readiness.require_ready(&entry.owner)?;
        let pending = engine
            .action_state()
            .get(entry.families.pending(), &decision)?
            .ok_or(DecisionCancellationIngressError::MissingDecision)?;
        let _ = pending.plan_cancellation(controller, decision, revision)?;
        decision_cancellation_ingress(
            &entry.lease_action,
            controller,
            decision,
            revision,
            idempotency_epoch,
        )
    }

    pub(crate) fn plan_cancellation(
        &self,
        engine: &crate::engine::Engine,
        controller: &DecisionControllerId,
        decision: DecisionId,
        revision: u64,
        ingress: &IngressEnvelope,
    ) -> Result<DecisionCancellationPlan, DecisionCancellationPlanError> {
        let entry = self.entries.get(&controller.owner).ok_or_else(|| {
            DecisionCancellationIngressError::SourceUnavailable {
                owner: controller.owner.clone(),
            }
        })?;
        self.owner_readiness.require_ready(&entry.owner)?;
        let expected = decision_cancellation_ingress(
            &entry.lease_action,
            controller,
            decision,
            revision,
            ingress.idempotency().epoch(),
        )?;
        if &expected != ingress {
            return Err(DecisionCancellationPlanError::IngressMismatch);
        }
        let pending = engine
            .action_state()
            .get(entry.families.pending(), &decision)?
            .cloned()
            .ok_or(DecisionCancellationIngressError::MissingDecision)?;
        let transition = pending.plan_cancellation(controller, decision, revision)?;
        let offered_action = ActionOfferId(CanonicalHash::for_canonical_value(
            "decision_cancellation_offer.v1",
            1,
            &json!({
                "action": entry.lease_action,
                "controller": controller,
                "decision": decision,
                "revision": revision,
            }),
            32 * 1024,
        )?);
        let invocation = ActionInvocation::new(ActionInvocationDraft {
            request: ingress.request(),
            principal: ingress.principal().clone(),
            actor: pending.actor,
            offered_action,
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters: json!({
                "decision": decision,
                "revision": revision,
                "phase": "cancelled",
            }),
            idempotency: ingress.idempotency().clone(),
        })?;
        let definitions = engine
            .registry
            .definitions()
            .ok_or(DecisionCancellationPlanError::RegistryUnavailable)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())?;
        guards.add_definition(entry.lease_action.clone(), definitions)?;
        self.owner_readiness.require_ready(&entry.owner)?;
        Ok(DecisionCancellationPlan {
            action: entry.lease_action.clone(),
            invocation,
            guards: guards.finish()?,
            transition,
        })
    }

    pub(crate) fn seed_cancellation(
        &self,
        controller: &DecisionControllerId,
        transition: DecisionCancellationTransition,
        txn: &mut ActionTxn<'_>,
    ) -> Result<(), DecisionCancellationPlanError> {
        let entry = self.entries.get(&controller.owner).ok_or_else(|| {
            DecisionCancellationIngressError::SourceUnavailable {
                owner: controller.owner.clone(),
            }
        })?;
        transition.stage(txn, &entry.families)?;
        Ok(())
    }

    pub(crate) fn plan_attempt_failure(
        &self,
        engine: &crate::engine::Engine,
        fence: &DecisionCompletionFence,
        kind: DecisionAttemptFailureKind,
        max_attempts: u8,
        ingress: &IngressEnvelope,
    ) -> Result<DecisionAttemptPlan, DecisionAttemptPlanError> {
        let entry = self.entry_for_attempt_fence(fence)?;
        let expected = decision_attempt_ingress(
            &entry.lease_action,
            fence,
            kind,
            max_attempts,
            ingress.idempotency().epoch(),
        )?;
        if &expected != ingress {
            return Err(DecisionAttemptPlanError::IngressMismatch);
        }
        let pending = self.validate_attempt_fence(engine, entry, fence)?.clone();
        let transition = pending.plan_attempt_failure(fence, kind, max_attempts)?;
        let offered_action = ActionOfferId(CanonicalHash::for_canonical_value(
            "decision_attempt_offer.v1",
            1,
            &json!({
                "action": entry.lease_action,
                "decision": fence.decision(),
                "revision": fence.revision(),
                "attempt": fence.attempt(),
                "failure": kind,
                "max_attempts": max_attempts,
            }),
            32 * 1024,
        )?);
        let invocation = ActionInvocation::new(ActionInvocationDraft {
            request: ingress.request(),
            principal: ingress.principal().clone(),
            actor: pending.actor,
            offered_action,
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters: json!({
                "decision": fence.decision(),
                "revision": fence.revision(),
                "attempt": fence.attempt(),
                "phase": "attempt_failure",
                "failure": kind,
                "max_attempts": max_attempts,
            }),
            idempotency: ingress.idempotency().clone(),
        })?;
        let definitions = engine
            .registry
            .definitions()
            .ok_or(DecisionAttemptPlanError::RegistryUnavailable)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())?;
        guards.add_definition(entry.lease_action.clone(), definitions)?;
        self.owner_readiness.require_ready(&entry.owner)?;
        Ok(DecisionAttemptPlan {
            action: entry.lease_action.clone(),
            invocation,
            guards: guards.finish()?,
            transition,
        })
    }

    pub(crate) fn seed_attempt_failure(
        &self,
        fence: &DecisionCompletionFence,
        transition: DecisionAttemptTransition,
        txn: &mut ActionTxn<'_>,
    ) -> Result<Option<DecisionCompletionFence>, DecisionAttemptPlanError> {
        let entry = self.entry_for_attempt_fence(fence)?;
        transition
            .stage(txn, &entry.families)
            .map_err(DecisionAttemptPlanError::Decision)
    }

    /// Re-read the exact owner-local pending record after store preflight and
    /// plan its Leased-to-Resolving post-image.
    pub(crate) fn plan_resolving(
        &self,
        engine: &crate::engine::Engine,
        fence: &DecisionCompletionFence,
        binding: &DecisionActionBinding,
        outcome: DecisionTerminalOutcome,
        ingress: &IngressEnvelope,
    ) -> Result<DecisionResolvingPlan, DecisionResolvingPlanError> {
        let entry = self.entry_for_fence(fence)?;
        let expected = decision_resolving_ingress(
            &entry.lease_action,
            fence,
            binding,
            outcome,
            ingress.idempotency().epoch(),
        )?;
        if &expected != ingress {
            return Err(DecisionResolvingPlanError::IngressMismatch);
        }
        let pending = self
            .validate_resolving_fence(engine, entry, fence, outcome)?
            .clone();
        let transition = pending.plan_resolving(fence, binding.clone(), outcome)?;
        let offered_action = ActionOfferId(CanonicalHash::for_canonical_value(
            "decision_resolving_offer.v1",
            1,
            &json!({
                "action": entry.lease_action,
                "decision": fence.decision(),
                "revision": fence.revision(),
                "attempt": fence.attempt(),
                "binding": binding,
                "outcome": outcome,
            }),
            32 * 1024,
        )?);
        let invocation = ActionInvocation::new(ActionInvocationDraft {
            request: ingress.request(),
            principal: ingress.principal().clone(),
            actor: pending.actor,
            offered_action,
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters: json!({
                "decision": fence.decision(),
                "revision": fence.revision(),
                "attempt": fence.attempt(),
                "phase": "resolving",
                "binding": binding,
                "outcome": outcome,
            }),
            idempotency: ingress.idempotency().clone(),
        })?;
        let definitions = engine
            .registry
            .definitions()
            .ok_or(DecisionResolvingPlanError::RegistryUnavailable)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())?;
        guards.add_definition(entry.lease_action.clone(), definitions)?;
        self.owner_readiness.require_ready(&entry.owner)?;
        Ok(DecisionResolvingPlan {
            action: entry.lease_action.clone(),
            invocation,
            guards: guards.finish()?,
            transition,
        })
    }

    pub(crate) fn seed_resolving(
        &self,
        fence: &DecisionCompletionFence,
        transition: DecisionResolvingTransition,
        txn: &mut ActionTxn<'_>,
    ) -> Result<(), DecisionResolvingPlanError> {
        let entry = self.entry_for_fence(fence)?;
        transition.stage(txn, &entry.families)?;
        Ok(())
    }

    pub(crate) fn terminal_ingress(
        &self,
        engine: &crate::engine::Engine,
        fence: &DecisionCompletionFence,
        binding: &DecisionActionBinding,
        outcome: DecisionTerminalOutcome,
        idempotency_epoch: u64,
    ) -> Result<IngressEnvelope, DecisionTerminalIngressError> {
        let entry = self.entry_for_terminal_fence(fence)?;
        self.validate_terminal_fence(engine, entry, fence, binding)?;
        decision_terminal_ingress(
            &entry.lease_action,
            fence,
            binding,
            outcome,
            idempotency_epoch,
        )
    }

    pub(crate) fn plan_terminal(
        &self,
        engine: &crate::engine::Engine,
        fence: &DecisionCompletionFence,
        binding: &DecisionActionBinding,
        outcome: DecisionTerminalOutcome,
        ingress: &IngressEnvelope,
    ) -> Result<DecisionTerminalPlan, DecisionTerminalPlanError> {
        let entry = self.entry_for_terminal_fence(fence)?;
        let expected = decision_terminal_ingress(
            &entry.lease_action,
            fence,
            binding,
            outcome,
            ingress.idempotency().epoch(),
        )?;
        if &expected != ingress {
            return Err(DecisionTerminalPlanError::IngressMismatch);
        }
        let pending = self
            .validate_terminal_fence(engine, entry, fence, binding)?
            .clone();
        let transition = pending.plan_terminal(fence, binding, outcome)?;
        let offered_action = ActionOfferId(CanonicalHash::for_canonical_value(
            "decision_terminal_offer.v1",
            1,
            &json!({
                "action": entry.lease_action,
                "decision": fence.decision(),
                "revision": fence.revision(),
                "attempt": fence.attempt(),
                "binding": binding,
                "outcome": outcome,
            }),
            32 * 1024,
        )?);
        let invocation = ActionInvocation::new(ActionInvocationDraft {
            request: ingress.request(),
            principal: ingress.principal().clone(),
            actor: pending.actor,
            offered_action,
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters: json!({
                "decision": fence.decision(),
                "revision": fence.revision(),
                "attempt": fence.attempt(),
                "phase": "terminal",
                "binding": binding,
                "outcome": outcome,
            }),
            idempotency: ingress.idempotency().clone(),
        })?;
        let definitions = engine
            .registry
            .definitions()
            .ok_or(DecisionTerminalPlanError::RegistryUnavailable)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())?;
        guards.add_definition(entry.lease_action.clone(), definitions)?;
        self.owner_readiness.require_ready(&entry.owner)?;
        Ok(DecisionTerminalPlan {
            action: entry.lease_action.clone(),
            invocation,
            guards: guards.finish()?,
            transition,
        })
    }

    pub(crate) fn seed_terminal(
        &self,
        fence: &DecisionCompletionFence,
        transition: DecisionTerminalTransition,
        txn: &mut ActionTxn<'_>,
    ) -> Result<(), DecisionTerminalPlanError> {
        let entry = self.entry_for_terminal_fence(fence)?;
        transition.stage(txn, &entry.families)?;
        Ok(())
    }

    pub(crate) fn compaction_ingress(
        &self,
        engine: &crate::engine::Engine,
        keys: &DecisionKeyRing,
        candidate: &DecisionCompactionCandidate,
        idempotency_epoch: u64,
    ) -> Result<IngressEnvelope, DecisionCompactionError> {
        let (entry, _) = self.validate_compaction_candidate(engine, keys, candidate)?;
        decision_compaction_ingress(&entry.lease_action, candidate.receipt(), idempotency_epoch)
    }

    pub(crate) fn plan_compaction(
        &self,
        engine: &crate::engine::Engine,
        keys: &DecisionKeyRing,
        candidate: &DecisionCompactionCandidate,
        ingress: &IngressEnvelope,
    ) -> Result<DecisionCompactionPlan, DecisionCompactionError> {
        let (entry, pending) = self.validate_compaction_candidate(engine, keys, candidate)?;
        let expected = decision_compaction_ingress(
            &entry.lease_action,
            candidate.receipt(),
            ingress.idempotency().epoch(),
        )?;
        if &expected != ingress {
            return Err(DecisionCompactionError::IngressMismatch);
        }
        let transition = pending.plan_compaction()?;
        if transition.receipt() != candidate.receipt() {
            return Err(DecisionCompactionError::StaleCandidate);
        }
        let offered_action = ActionOfferId(CanonicalHash::for_canonical_value(
            "decision_compaction_offer.v1",
            1,
            &json!({
                "action": entry.lease_action,
                "receipt": candidate.receipt(),
            }),
            32 * 1024,
        )?);
        let invocation = ActionInvocation::new(ActionInvocationDraft {
            request: ingress.request(),
            principal: ingress.principal().clone(),
            actor: pending.actor,
            offered_action,
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters: json!({
                "decision": pending.id,
                "revision": pending.revision,
                "phase": "compact",
                "outcome": candidate.receipt().outcome,
            }),
            idempotency: ingress.idempotency().clone(),
        })?;
        let definitions = engine
            .registry
            .definitions()
            .ok_or(DecisionCompactionError::RegistryUnavailable)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())?;
        guards.add_definition(entry.lease_action.clone(), definitions)?;
        self.owner_readiness.require_ready(&entry.owner)?;
        Ok(DecisionCompactionPlan {
            action: entry.lease_action.clone(),
            invocation,
            guards: guards.finish()?,
            transition,
        })
    }

    pub(crate) fn seed_compaction(
        &self,
        candidate: &DecisionCompactionCandidate,
        transition: DecisionCompactionTransition,
        txn: &mut ActionTxn<'_>,
    ) -> Result<(), DecisionCompactionError> {
        if candidate.registry_fingerprint != self.fingerprint {
            return Err(DecisionCompactionError::StaleCandidate);
        }
        let entry = self.entries.get(candidate.owner()).ok_or_else(|| {
            DecisionCompactionError::SourceUnavailable {
                owner: candidate.owner().clone(),
            }
        })?;
        transition.stage(txn, &entry.families)?;
        Ok(())
    }

    fn validate_compaction_candidate<'a>(
        &'a self,
        engine: &'a crate::engine::Engine,
        keys: &DecisionKeyRing,
        candidate: &DecisionCompactionCandidate,
    ) -> Result<(&'a DecisionSourceEntry, &'a PendingDecision), DecisionCompactionError> {
        if candidate.registry_fingerprint != self.fingerprint
            || candidate.receipt.controller.owner != candidate.owner
        {
            return Err(DecisionCompactionError::StaleCandidate);
        }
        let entry = self.entries.get(candidate.owner()).ok_or_else(|| {
            DecisionCompactionError::SourceUnavailable {
                owner: candidate.owner().clone(),
            }
        })?;
        self.owner_readiness.require_ready(&entry.owner)?;
        let pending = engine
            .action_state()
            .get(entry.families.pending(), &candidate.receipt.id)?
            .ok_or(DecisionCompactionError::MissingDecision)?;
        if engine
            .action_state()
            .get(entry.families.terminal_receipts(), &candidate.receipt.id)?
            .is_some()
        {
            return Err(DecisionCompactionError::DuplicateRepresentation);
        }
        let prompt = engine
            .action_state()
            .get(entry.families.prompts(), &candidate.receipt.id)?
            .ok_or(DecisionCompactionError::MissingPrompt)?;
        pending.validate_prompt_pair(prompt, keys)?;
        validate_deadline_link(engine, entry, pending)
            .map_err(|source| DecisionCompactionError::Deadline(Box::new(source)))?;
        if DecisionTerminalReceipt::from_terminal(pending)? != candidate.receipt {
            return Err(DecisionCompactionError::StaleCandidate);
        }
        Ok((entry, pending))
    }

    pub(crate) fn resolve_response(
        &self,
        engine: &crate::engine::Engine,
        keys: &DecisionKeyRing,
        fence: &DecisionCompletionFence,
        response: &DecisionResponse,
        ingress: &IngressEnvelope,
    ) -> Result<DecisionResponseResolution, DecisionResponseResolveError> {
        let expected = self.response_ingress(engine, keys, fence, response)?;
        if &expected != ingress {
            return Err(DecisionResponseResolveError::IngressMismatch);
        }
        let (pending, prompt, offered) =
            self.validate_response_fence(engine, keys, fence, response)?;
        let projected = prompt
            .prompt
            .actions
            .iter()
            .find(|action| action.offer() == response.offer)
            .ok_or(DecisionResponseResolveError::OfferNotPrompted)?;
        let query = engine
            .legal_action_query(
                pending.principal.clone(),
                pending.actor,
                projected.subject().clone(),
            )
            .map_err(DecisionResponseResolveError::Subject)?;
        if query.truth_subject() != &pending.truth_subject
            || query.viewer_scope() != pending.viewer_scope
        {
            return Err(DecisionResponseResolveError::ProjectionChanged);
        }
        let offers = engine
            .project_legal_actions(&query)
            .map_err(DecisionResponseResolveError::Projection)?;
        let resolved = engine
            .resolve_legal_action(
                &offers,
                OfferExecutionContext {
                    request: ingress.request(),
                    principal: pending.principal.clone(),
                    actor: pending.actor,
                    idempotency: pending.action_idempotency.clone(),
                    viewer_scope: pending.viewer_scope,
                    now: None,
                },
                OfferSubmission {
                    offer: response.offer,
                    subject: projected.subject().clone(),
                    targets: response.targets.clone(),
                    parameters: response.parameters.clone(),
                },
            )
            .map_err(DecisionResponseResolveError::Offer)?;
        let (action, resolved, guards) = resolved.into_authoritative_parts();
        let mut prompt_stamps = offered.common_revisions.clone();
        for target in &response.targets {
            let target = offered
                .target_revisions
                .iter()
                .find(|witness| &witness.perceived == target)
                .ok_or(DecisionResponseResolveError::TargetNotPrompted)?;
            prompt_stamps.extend(target.revisions.iter().cloned());
        }
        let definitions = engine
            .registry
            .definitions()
            .ok_or(DecisionResponseResolveError::RegistryUnavailable)?;
        let guards = guards
            .with_definition_guards(offered.bridge_definitions.clone(), definitions)?
            .with_prompt_revision_stamps(prompt_stamps)?;
        let binding = DecisionActionBinding {
            request: resolved.invocation().request(),
            request_hash: resolved.invocation().request_hash(),
        };
        Ok(DecisionResponseResolution {
            resolved: crate::offer::ResolvedAuthoritativeOffer::new(action, resolved, guards),
            binding,
        })
    }

    /// Resolve a deterministic fallback selection through the identical
    /// current-offer and prompt-witness path used by an async response.
    pub(crate) fn resolve_fallback(
        &self,
        engine: &crate::engine::Engine,
        keys: &DecisionKeyRing,
        candidate: &DecisionFallbackCandidate,
        response: &DecisionResponse,
        ingress: &IngressEnvelope,
    ) -> Result<DecisionResponseResolution, DecisionResponseResolveError> {
        let expected = self.fallback_ingress(engine, keys, candidate, response)?;
        if &expected != ingress {
            return Err(DecisionResponseResolveError::IngressMismatch);
        }
        let (pending, prompt, offered) =
            self.validate_fallback_response(engine, keys, candidate, response)?;
        let projected = prompt
            .prompt
            .actions
            .iter()
            .find(|action| action.offer() == response.offer)
            .ok_or(DecisionResponseResolveError::OfferNotPrompted)?;
        let query = engine
            .legal_action_query(
                pending.principal.clone(),
                pending.actor,
                projected.subject().clone(),
            )
            .map_err(DecisionResponseResolveError::Subject)?;
        if query.truth_subject() != &pending.truth_subject
            || query.viewer_scope() != pending.viewer_scope
        {
            return Err(DecisionResponseResolveError::ProjectionChanged);
        }
        let offers = engine
            .project_legal_actions(&query)
            .map_err(DecisionResponseResolveError::Projection)?;
        let resolved = engine
            .resolve_legal_action(
                &offers,
                OfferExecutionContext {
                    request: ingress.request(),
                    principal: pending.principal.clone(),
                    actor: pending.actor,
                    idempotency: pending.action_idempotency.clone(),
                    viewer_scope: pending.viewer_scope,
                    now: None,
                },
                OfferSubmission {
                    offer: response.offer,
                    subject: projected.subject().clone(),
                    targets: response.targets.clone(),
                    parameters: response.parameters.clone(),
                },
            )
            .map_err(DecisionResponseResolveError::Offer)?;
        let (action, resolved, guards) = resolved.into_authoritative_parts();
        let mut prompt_stamps = offered.common_revisions.clone();
        for target in &response.targets {
            let target = offered
                .target_revisions
                .iter()
                .find(|witness| &witness.perceived == target)
                .ok_or(DecisionResponseResolveError::TargetNotPrompted)?;
            prompt_stamps.extend(target.revisions.iter().cloned());
        }
        let definitions = engine
            .registry
            .definitions()
            .ok_or(DecisionResponseResolveError::RegistryUnavailable)?;
        let guards = guards
            .with_definition_guards(offered.bridge_definitions.clone(), definitions)?
            .with_prompt_revision_stamps(prompt_stamps)?;
        let binding = DecisionActionBinding {
            request: resolved.invocation().request(),
            request_hash: resolved.invocation().request_hash(),
        };
        Ok(DecisionResponseResolution {
            resolved: crate::offer::ResolvedAuthoritativeOffer::new(action, resolved, guards),
            binding,
        })
    }

    fn validate_response_fence<'a>(
        &'a self,
        engine: &'a crate::engine::Engine,
        keys: &DecisionKeyRing,
        fence: &DecisionCompletionFence,
        response: &DecisionResponse,
    ) -> Result<
        (
            &'a PendingDecision,
            &'a crate::decision::DecisionPromptRecord,
            &'a PendingOfferWitness,
        ),
        DecisionResponseIngressError,
    > {
        response.validate()?;
        let entry = self.entries.get(&fence.controller().owner).ok_or_else(|| {
            DecisionResponseIngressError::SourceUnavailable {
                owner: fence.controller().owner.clone(),
            }
        })?;
        self.owner_readiness.require_ready(&entry.owner)?;
        let pending = engine
            .action_state()
            .get(entry.families.pending(), &fence.decision())?
            .ok_or(DecisionResponseIngressError::MissingDecision)?;
        let prompt = engine
            .action_state()
            .get(entry.families.prompts(), &fence.decision())?
            .ok_or(DecisionResponseIngressError::MissingPrompt)?;
        pending.validate_prompt_pair(prompt, keys)?;
        if &pending.controller != fence.controller()
            || pending.id != fence.decision()
            || pending.revision != fence.revision()
            || response.token != prompt.prompt.token
        {
            return Err(DecisionResponseIngressError::StaleCompletion);
        }
        let PendingDecisionStatus::Leased {
            attempt,
            lease_until,
        } = &pending.status
        else {
            return Err(DecisionResponseIngressError::StaleCompletion);
        };
        if *attempt != fence.attempt() || lease_until != fence.lease_until() {
            return Err(DecisionResponseIngressError::StaleCompletion);
        }
        let now = engine.clock_now(&pending.deadline.clock)?;
        if now.checked_cmp(lease_until).is_err()
            || now.checked_cmp(&pending.deadline).is_err()
            || now.at >= lease_until.at
            || now.at >= pending.deadline.at
        {
            return Err(DecisionResponseIngressError::StaleCompletion);
        }
        let offered = pending
            .offered
            .iter()
            .find(|offered| offered.offer == response.offer)
            .ok_or(DecisionResponseIngressError::OfferNotPrompted)?;
        Ok((pending, prompt, offered))
    }

    fn entry_for_fence(
        &self,
        fence: &DecisionCompletionFence,
    ) -> Result<&DecisionSourceEntry, DecisionResolvingIngressError> {
        let entry = self.entries.get(&fence.controller().owner).ok_or_else(|| {
            DecisionResolvingIngressError::SourceUnavailable {
                owner: fence.controller().owner.clone(),
            }
        })?;
        self.owner_readiness.require_ready(&entry.owner)?;
        Ok(entry)
    }

    fn validate_resolving_fence<'a>(
        &self,
        engine: &'a crate::engine::Engine,
        entry: &DecisionSourceEntry,
        fence: &DecisionCompletionFence,
        outcome: DecisionTerminalOutcome,
    ) -> Result<&'a PendingDecision, DecisionResolvingIngressError> {
        let pending = engine
            .action_state()
            .get(entry.families.pending(), &fence.decision())?
            .ok_or(DecisionResolvingIngressError::MissingDecision)?;
        if &pending.controller != fence.controller()
            || pending.id != fence.decision()
            || pending.revision != fence.revision()
        {
            return Err(DecisionResolvingIngressError::StaleCompletion);
        }
        let (attempt, lease_until) = match &pending.status {
            PendingDecisionStatus::Leased {
                attempt,
                lease_until,
            } if outcome == DecisionTerminalOutcome::Executed => (attempt, lease_until),
            PendingDecisionStatus::FallbackReady {
                attempt,
                lease_until,
                ..
            } if outcome == DecisionTerminalOutcome::Fallback => (attempt, lease_until),
            _ => return Err(DecisionResolvingIngressError::StaleCompletion),
        };
        if *attempt != fence.attempt() || lease_until != fence.lease_until() {
            return Err(DecisionResolvingIngressError::StaleCompletion);
        }
        Ok(pending)
    }

    fn entry_for_attempt_fence(
        &self,
        fence: &DecisionCompletionFence,
    ) -> Result<&DecisionSourceEntry, DecisionAttemptIngressError> {
        let entry = self.entries.get(&fence.controller().owner).ok_or_else(|| {
            DecisionAttemptIngressError::SourceUnavailable {
                owner: fence.controller().owner.clone(),
            }
        })?;
        self.owner_readiness.require_ready(&entry.owner)?;
        Ok(entry)
    }

    fn validate_attempt_fence<'a>(
        &self,
        engine: &'a crate::engine::Engine,
        entry: &DecisionSourceEntry,
        fence: &DecisionCompletionFence,
    ) -> Result<&'a PendingDecision, DecisionAttemptIngressError> {
        let pending = engine
            .action_state()
            .get(entry.families.pending(), &fence.decision())?
            .ok_or(DecisionAttemptIngressError::MissingDecision)?;
        if &pending.controller != fence.controller()
            || pending.id != fence.decision()
            || pending.revision != fence.revision()
        {
            return Err(DecisionAttemptIngressError::StaleAttempt);
        }
        let PendingDecisionStatus::Leased {
            attempt,
            lease_until,
        } = &pending.status
        else {
            return Err(DecisionAttemptIngressError::StaleAttempt);
        };
        if *attempt != fence.attempt() || lease_until != fence.lease_until() {
            return Err(DecisionAttemptIngressError::StaleAttempt);
        }
        Ok(pending)
    }

    fn entry_for_terminal_fence(
        &self,
        fence: &DecisionCompletionFence,
    ) -> Result<&DecisionSourceEntry, DecisionTerminalIngressError> {
        let entry = self.entries.get(&fence.controller().owner).ok_or_else(|| {
            DecisionTerminalIngressError::SourceUnavailable {
                owner: fence.controller().owner.clone(),
            }
        })?;
        self.owner_readiness.require_ready(&entry.owner)?;
        Ok(entry)
    }

    fn validate_terminal_fence<'a>(
        &self,
        engine: &'a crate::engine::Engine,
        entry: &DecisionSourceEntry,
        fence: &DecisionCompletionFence,
        binding: &DecisionActionBinding,
    ) -> Result<&'a PendingDecision, DecisionTerminalIngressError> {
        let pending = engine
            .action_state()
            .get(entry.families.pending(), &fence.decision())?
            .ok_or(DecisionTerminalIngressError::MissingDecision)?;
        if &pending.controller != fence.controller()
            || pending.id != fence.decision()
            || pending.revision != fence.revision()
        {
            return Err(DecisionTerminalIngressError::StaleResolution);
        }
        let PendingDecisionStatus::Resolving {
            attempt,
            lease_until,
            action_binding,
            ..
        } = &pending.status
        else {
            return Err(DecisionTerminalIngressError::StaleResolution);
        };
        if *attempt != fence.attempt()
            || lease_until != fence.lease_until()
            || action_binding != binding
        {
            return Err(DecisionTerminalIngressError::StaleResolution);
        }
        Ok(pending)
    }

    pub(crate) fn scan(
        &self,
        engine: &crate::engine::Engine,
        keys: &DecisionKeyRing,
        cursor: Option<&DecisionSourceCursor>,
        limit: usize,
    ) -> Result<DecisionSourcePage, DecisionSourceScanError> {
        if limit == 0 || limit > MAX_DECISION_RECOVERY_PAGE {
            return Err(DecisionSourceScanError::InvalidLimit {
                actual: limit,
                max: MAX_DECISION_RECOVERY_PAGE,
            });
        }
        if let Some(cursor) = cursor
            && !self.entries.contains_key(&cursor.owner)
        {
            return Err(DecisionSourceScanError::StaleCursor {
                owner: cursor.owner.clone(),
            });
        }
        let mut candidates = Vec::new();
        let mut reconciliations = Vec::new();
        let mut fallback_triggers = Vec::new();
        let mut fallbacks = Vec::new();
        let mut compactions = Vec::new();
        let mut admission_index = DecisionAdmissionIndex::default();
        let mut records_scanned = 0;
        let mut last_scanned = None;
        for (owner, entry) in &self.entries {
            if cursor.is_some_and(|cursor| owner < &cursor.owner) {
                continue;
            }
            if self.owner_readiness.freeze_for(owner)?.is_some() {
                continue;
            }
            let after = cursor
                .filter(|cursor| &cursor.owner == owner)
                .map(|cursor| cursor.after_encoded_key.as_str());
            let remaining = limit - records_scanned;
            if remaining == 0 {
                break;
            }
            let page =
                engine
                    .action_state()
                    .scan_family(entry.families.pending(), after, remaining)?;
            for scanned in page.entries() {
                records_scanned += 1;
                last_scanned = Some(DecisionSourceCursor {
                    owner: owner.clone(),
                    after_encoded_key: SmolStr::new(scanned.encoded_key()),
                });
                let pending = scanned.value();
                if pending.controller.owner != *owner
                    || scanned.encoded_key() != pending.id.canonical_hash().to_hex()
                {
                    return Err(DecisionSourceScanError::RecordIdentity {
                        owner: owner.clone(),
                        encoded_key: SmolStr::new(scanned.encoded_key()),
                    });
                }
                if engine
                    .action_state()
                    .get(entry.families.terminal_receipts(), &pending.id)?
                    .is_some()
                {
                    return Err(DecisionSourceScanError::DuplicateRepresentation {
                        owner: owner.clone(),
                        decision: pending.id,
                    });
                }
                let prompt = engine
                    .action_state()
                    .get(entry.families.prompts(), &pending.id)?
                    .ok_or_else(|| DecisionSourceScanError::MissingPrompt {
                        owner: owner.clone(),
                        decision: pending.id,
                    })?;
                pending.validate_prompt_pair(prompt, keys)?;
                admission_index.observe_existing(pending, prompt, keys)?;
                validate_deadline_link(engine, entry, pending)?;
                let now = engine
                    .clock_now(&pending.deadline.clock)
                    .map_err(|source| DecisionSourceScanError::Clock {
                        owner: owner.clone(),
                        decision: pending.id,
                        source: Box::new(source),
                    })?;
                let attempt = eligible_attempt(pending, &now);
                if let Some(attempt) = attempt {
                    candidates.push(DecisionDispatchCandidate {
                        registry_fingerprint: self.fingerprint,
                        owner: owner.clone(),
                        encoded_key: SmolStr::new(scanned.encoded_key()),
                        wake: DecisionWake {
                            id: pending.id,
                            revision: pending.revision,
                            prompt_digest: pending.prompt_digest,
                        },
                        controller: pending.controller.clone(),
                        attempt,
                    });
                }
                if now.at >= pending.deadline.at
                    && matches!(
                        pending.status,
                        PendingDecisionStatus::Ready { .. } | PendingDecisionStatus::Leased { .. }
                    )
                {
                    fallback_triggers.push(fallback_trigger_candidate(
                        self.fingerprint,
                        owner,
                        pending,
                        DecisionFallbackReason::Deadline,
                    )?);
                }
                if let PendingDecisionStatus::Resolving {
                    attempt,
                    lease_until,
                    action_binding,
                    outcome,
                    ..
                } = &pending.status
                {
                    reconciliations.push(DecisionReconciliationCandidate {
                        fence: DecisionCompletionFence::new(
                            pending.controller.clone(),
                            pending.id,
                            pending.revision,
                            *attempt,
                            lease_until.clone(),
                        )?,
                        binding: action_binding.clone(),
                        action_idempotency: pending.action_idempotency.clone(),
                        outcome: *outcome,
                    });
                }
                if let PendingDecisionStatus::FallbackReady {
                    attempt,
                    lease_until,
                    ..
                } = &pending.status
                {
                    fallbacks.push(DecisionFallbackCandidate {
                        registry_fingerprint: self.fingerprint,
                        owner: owner.clone(),
                        fence: DecisionCompletionFence::new(
                            pending.controller.clone(),
                            pending.id,
                            pending.revision,
                            *attempt,
                            lease_until.clone(),
                        )?,
                    });
                }
                if pending.status.is_terminal() {
                    compactions.push(DecisionCompactionCandidate {
                        registry_fingerprint: self.fingerprint,
                        owner: owner.clone(),
                        receipt: DecisionTerminalReceipt::from_terminal(pending)?,
                    });
                }
            }
            self.owner_readiness.require_ready(owner)?;
            if records_scanned == limit {
                return Ok(DecisionSourcePage {
                    candidates,
                    reconciliations,
                    fallback_triggers,
                    fallbacks,
                    compactions,
                    admission_index,
                    records_scanned,
                    next_cursor: last_scanned,
                });
            }
        }
        Ok(DecisionSourcePage {
            candidates,
            reconciliations,
            fallback_triggers,
            fallbacks,
            compactions,
            admission_index,
            records_scanned,
            next_cursor: None,
        })
    }

    pub(crate) fn candidate_for_wake(
        &self,
        engine: &crate::engine::Engine,
        keys: &DecisionKeyRing,
        owner: &OwnerId,
        wake: &DecisionWake,
    ) -> Result<Option<DecisionDispatchCandidate>, DecisionSourceScanError> {
        let Some(entry) = self.entries.get(owner) else {
            return Ok(None);
        };
        if self.owner_readiness.freeze_for(owner)?.is_some() {
            return Ok(None);
        }
        let Some(pending) = engine
            .action_state()
            .get(entry.families.pending(), &wake.id)?
        else {
            return Ok(None);
        };
        if pending.controller.owner != *owner {
            return Err(DecisionSourceScanError::RecordIdentity {
                owner: owner.clone(),
                encoded_key: SmolStr::new(pending.id.canonical_hash().to_hex()),
            });
        }
        pending.validate_wake(wake)?;
        let prompt = engine
            .action_state()
            .get(entry.families.prompts(), &pending.id)?
            .ok_or_else(|| DecisionSourceScanError::MissingPrompt {
                owner: owner.clone(),
                decision: pending.id,
            })?;
        pending.validate_prompt_pair(prompt, keys)?;
        validate_deadline_link(engine, entry, pending)?;
        let now = engine
            .clock_now(&pending.deadline.clock)
            .map_err(|source| DecisionSourceScanError::Clock {
                owner: owner.clone(),
                decision: pending.id,
                source: Box::new(source),
            })?;
        let Some(attempt) = eligible_attempt(pending, &now) else {
            return Ok(None);
        };
        Ok(Some(DecisionDispatchCandidate {
            registry_fingerprint: self.fingerprint,
            owner: owner.clone(),
            encoded_key: SmolStr::new(pending.id.canonical_hash().to_hex()),
            wake: *wake,
            controller: pending.controller.clone(),
            attempt,
        }))
    }

    pub(crate) fn plan_candidate(
        &self,
        engine: &crate::engine::Engine,
        keys: &DecisionKeyRing,
        candidate: &DecisionDispatchCandidate,
        ingress: &IngressEnvelope,
    ) -> Result<DecisionDispatchPlan, DecisionSourcePlanError> {
        let entry = self.entry_for_candidate(candidate)?;
        let expected = decision_lease_ingress(
            &entry.lease_action,
            candidate,
            ingress.idempotency().epoch(),
        )?;
        if &expected != ingress {
            return Err(DecisionSourcePlanError::IngressMismatch);
        }
        let pending = engine
            .action_state()
            .get(entry.families.pending(), &candidate.wake.id)?
            .cloned()
            .ok_or(DecisionSourcePlanError::MissingDecision)?;
        let prompt = engine
            .action_state()
            .get(entry.families.prompts(), &candidate.wake.id)?
            .ok_or(DecisionSourcePlanError::MissingPrompt)?;
        pending.validate_wake(&candidate.wake)?;
        validate_deadline_link(engine, entry, &pending)
            .map_err(DecisionSourcePlanError::DeadlineLinkage)?;
        let now = engine.clock_now(&pending.deadline.clock)?;
        let mut ready = pending;
        if matches!(ready.status, PendingDecisionStatus::Leased { .. }) {
            ready.retry_expired_lease(ready.revision, &now)?;
        }
        let transition =
            ready.plan_worker_lease(prompt, &candidate.wake, keys, &now, entry.lease_duration)?;
        let PendingDecisionStatus::Leased { attempt, .. } = transition.post_image().status else {
            return Err(DecisionSourcePlanError::StaleCandidate);
        };
        if attempt != candidate.attempt {
            return Err(DecisionSourcePlanError::StaleCandidate);
        }
        let offered_action = ActionOfferId(CanonicalHash::for_canonical_value(
            "decision_lease_offer.v1",
            1,
            &json!({
                "action": entry.lease_action,
                "decision": candidate.wake.id,
                "revision": candidate.wake.revision,
                "attempt": candidate.attempt,
            }),
            32 * 1024,
        )?);
        let invocation = ActionInvocation::new(ActionInvocationDraft {
            request: ingress.request(),
            principal: ingress.principal().clone(),
            actor: ready.actor,
            offered_action,
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters: json!({
                "decision": candidate.wake.id,
                "revision": candidate.wake.revision,
                "attempt": candidate.attempt,
            }),
            idempotency: ingress.idempotency().clone(),
        })?;
        let definitions = engine
            .registry
            .definitions()
            .ok_or(DecisionSourcePlanError::RegistryUnavailable)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())?;
        guards.add_definition(entry.lease_action.clone(), definitions)?;
        self.owner_readiness.require_ready(&entry.owner)?;
        Ok(DecisionDispatchPlan {
            action: entry.lease_action.clone(),
            invocation,
            guards: guards.finish()?,
            transition,
        })
    }

    pub(crate) fn seed_lease(
        &self,
        candidate: &DecisionDispatchCandidate,
        transition: DecisionLeaseTransition,
        txn: &mut ActionTxn<'_>,
    ) -> Result<DecisionWorkerLease, DecisionSourcePlanError> {
        let entry = self.entry_for_candidate(candidate)?;
        let worker = transition.stage(txn, &entry.families)?;
        if worker.decision() != candidate.wake.id
            || worker.revision() != candidate.wake.revision
            || worker.attempt() != candidate.attempt
            || worker.controller() != &candidate.controller
        {
            return Err(DecisionSourcePlanError::StaleCandidate);
        }
        Ok(worker)
    }

    fn entry_for_candidate(
        &self,
        candidate: &DecisionDispatchCandidate,
    ) -> Result<&DecisionSourceEntry, DecisionSourceCandidateError> {
        if candidate.registry_fingerprint != self.fingerprint {
            return Err(DecisionSourceCandidateError::RegistryMismatch);
        }
        let entry = self.entries.get(&candidate.owner).ok_or_else(|| {
            DecisionSourceCandidateError::SourceUnavailable {
                owner: candidate.owner.clone(),
            }
        })?;
        if candidate.controller.owner != candidate.owner
            || candidate.encoded_key.as_str() != candidate.wake.id.canonical_hash().to_hex()
        {
            return Err(DecisionSourceCandidateError::IdentityMismatch);
        }
        self.owner_readiness.require_ready(&entry.owner)?;
        Ok(entry)
    }

    fn validate_fallback_trigger<'a>(
        &'a self,
        engine: &'a crate::engine::Engine,
        candidate: &DecisionFallbackTriggerCandidate,
    ) -> Result<(&'a DecisionSourceEntry, &'a PendingDecision), DecisionFallbackTriggerError> {
        if candidate.registry_fingerprint != self.fingerprint
            || candidate.controller.owner != candidate.owner
        {
            return Err(DecisionFallbackTriggerError::StaleCandidate);
        }
        if !matches!(
            candidate.reason,
            DecisionFallbackReason::Deadline | DecisionFallbackReason::ProviderUnavailable
        ) {
            return Err(DecisionFallbackTriggerError::InvalidReason);
        }
        let entry = self.entries.get(&candidate.owner).ok_or_else(|| {
            DecisionFallbackTriggerError::SourceUnavailable {
                owner: candidate.owner.clone(),
            }
        })?;
        self.owner_readiness.require_ready(&entry.owner)?;
        let pending = engine
            .action_state()
            .get(entry.families.pending(), &candidate.wake.id)?
            .ok_or(DecisionFallbackTriggerError::MissingDecision)?;
        pending.validate_wake(&candidate.wake)?;
        if pending.controller != candidate.controller {
            return Err(DecisionFallbackTriggerError::StaleCandidate);
        }
        let exact = match &pending.status {
            PendingDecisionStatus::Ready { attempt } => {
                *attempt == candidate.attempt && pending.deadline == candidate.lease_until
            }
            PendingDecisionStatus::Leased {
                attempt,
                lease_until,
            } => *attempt == candidate.attempt && lease_until == &candidate.lease_until,
            _ => false,
        };
        if !exact {
            return Err(DecisionFallbackTriggerError::StaleCandidate);
        }
        let now = engine.clock_now(&pending.deadline.clock)?;
        if candidate.reason == DecisionFallbackReason::Deadline && now.at < pending.deadline.at {
            return Err(DecisionFallbackTriggerError::DeadlineNotReached);
        }
        Ok((entry, pending))
    }
}

fn fallback_trigger_candidate(
    registry_fingerprint: DefinitionDigest,
    owner: &OwnerId,
    pending: &PendingDecision,
    reason: DecisionFallbackReason,
) -> Result<DecisionFallbackTriggerCandidate, DecisionError> {
    let (attempt, lease_until) = match &pending.status {
        PendingDecisionStatus::Ready { attempt } => (*attempt, pending.deadline.clone()),
        PendingDecisionStatus::Leased {
            attempt,
            lease_until,
        } => (*attempt, lease_until.clone()),
        _ => return Err(DecisionError::InvalidTransition),
    };
    Ok(DecisionFallbackTriggerCandidate {
        registry_fingerprint,
        owner: owner.clone(),
        wake: DecisionWake {
            id: pending.id,
            revision: pending.revision,
            prompt_digest: pending.prompt_digest,
        },
        controller: pending.controller.clone(),
        attempt,
        lease_until,
        reason,
    })
}

fn eligible_attempt(
    pending: &crate::decision::PendingDecision,
    now: &crate::clock::ClockStamp,
) -> Option<u8> {
    if now.at >= pending.deadline.at {
        return None;
    }
    match &pending.status {
        PendingDecisionStatus::Ready { attempt } => Some(*attempt),
        PendingDecisionStatus::Leased {
            attempt,
            lease_until,
        } if lease_until.checked_cmp(now).is_ok()
            && now.at >= lease_until.at
            && *attempt < MAX_DECISION_ATTEMPTS =>
        {
            Some(*attempt + 1)
        }
        _ => None,
    }
}

fn canonical_response_ingress(
    pending: &PendingDecision,
    fence: &DecisionCompletionFence,
    response: &DecisionResponse,
) -> Result<IngressEnvelope, DecisionResponseIngressError> {
    let request = match pending.action_idempotency.key() {
        IdempotencyKey::Request(request) => *request,
        IdempotencyKey::Caller(_) | IdempotencyKey::Deterministic(_) => {
            let digest = CanonicalHash::for_canonical_value(
                "decision_response_request.v1",
                1,
                &json!({
                    "controller": fence.controller(),
                    "decision": fence.decision(),
                    "revision": fence.revision(),
                }),
                32 * 1024,
            )?;
            let mut bytes = [0_u8; 16];
            bytes.copy_from_slice(&digest.as_bytes()[..16]);
            crate::action::RequestUid::from_bytes(bytes)
        }
    };
    Ok(IngressEnvelope::new(
        request,
        pending.principal.clone(),
        IngressSurfaceVersion::new(
            CanonicalId::new("kernel.decision_response").expect("static decision response surface"),
            1,
        )?,
        IngressPayload::structured(json!({
            "decision": fence.decision(),
            "revision": fence.revision(),
            "response": response,
        }))?,
        pending.action_idempotency.clone(),
    )?)
}

fn bounded_adapter_detail(detail: &str) -> String {
    const MAX_BYTES: usize = 512;
    if detail.len() <= MAX_BYTES {
        return detail.to_owned();
    }
    let mut end = MAX_BYTES;
    while !detail.is_char_boundary(end) {
        end -= 1;
    }
    detail[..end].to_owned()
}

fn validate_deadline_link(
    engine: &crate::engine::Engine,
    entry: &DecisionSourceEntry,
    pending: &crate::decision::PendingDecision,
) -> Result<(), DecisionSourceScanError> {
    let local_id =
        CanonicalId::parse_canonical(pending.id.canonical_hash().to_hex()).map_err(|_| {
            DecisionSourceScanError::DeadlineLinkage {
                owner: entry.owner.clone(),
                decision: pending.id,
            }
        })?;
    let schedule_id = ScheduleId::derive(&entry.owner, &local_id).map_err(|_| {
        DecisionSourceScanError::DeadlineLinkage {
            owner: entry.owner.clone(),
            decision: pending.id,
        }
    })?;
    let deadline = engine
        .action_state()
        .get(entry.families.deadlines(), &schedule_id)?
        .ok_or_else(|| DecisionSourceScanError::MissingDeadline {
            owner: entry.owner.clone(),
            decision: pending.id,
        })?;
    if deadline.owner() != &entry.owner
        || deadline.generation() != pending.revision
        || deadline.next_due() != &pending.deadline
        || deadline.cadence().is_some()
    {
        return Err(DecisionSourceScanError::DeadlineLinkage {
            owner: entry.owner.clone(),
            decision: pending.id,
        });
    }
    Ok(())
}

/// Canonical internal ingress for one exact decision lease attempt.
pub fn decision_lease_ingress(
    action: &DefinitionRef,
    candidate: &DecisionDispatchCandidate,
    idempotency_epoch: u64,
) -> Result<IngressEnvelope, DecisionLeaseIngressError> {
    if action.key().owner != candidate.owner {
        return Err(DecisionLeaseIngressError::OwnerMismatch);
    }
    let request_hash = CanonicalHash::for_canonical_value(
        "decision_lease_request.v1",
        1,
        &json!({
            "action": action,
            "decision": candidate.wake.id,
            "revision": candidate.wake.revision,
            "attempt": candidate.attempt,
        }),
        32 * 1024,
    )?;
    let mut request_bytes = [0_u8; 16];
    request_bytes.copy_from_slice(&request_hash.as_bytes()[..16]);
    let request = crate::action::RequestUid::from_bytes(request_bytes);
    let principal = PrincipalId::Scoped {
        owner: candidate.owner.clone(),
        class: CanonicalId::new("decision_dispatcher").expect("static principal class"),
        id: IdempotencyKeyText::parse_canonical(candidate.wake.id.canonical_hash().to_hex())?,
    };
    let idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner: candidate.owner.clone(),
            namespace: CanonicalId::new(action.key().id.as_str())?,
        },
        idempotency_epoch,
        IdempotencyKey::Deterministic(request_hash),
    );
    Ok(IngressEnvelope::new(
        request,
        principal,
        IngressSurfaceVersion::new(
            CanonicalId::new("kernel.decision_lease").expect("static surface id"),
            1,
        )?,
        IngressPayload::structured(json!({
            "action": action,
            "decision": candidate.wake.id,
            "revision": candidate.wake.revision,
            "attempt": candidate.attempt,
        }))?,
        idempotency,
    )?)
}

/// Canonical internal ingress for transferring an exact Ready/Leased record
/// to deterministic fallback custody. The identity binds persisted state,
/// never the moving observation time.
pub fn decision_fallback_trigger_ingress(
    action: &DefinitionRef,
    candidate: &DecisionFallbackTriggerCandidate,
    idempotency_epoch: u64,
) -> Result<IngressEnvelope, DecisionFallbackTriggerError> {
    if action.key().owner != candidate.owner {
        return Err(DecisionFallbackTriggerError::OwnerMismatch);
    }
    let request_hash = CanonicalHash::for_canonical_value(
        "decision_fallback_trigger_request.v1",
        1,
        &json!({
            "action": action,
            "decision": candidate.wake.id,
            "revision": candidate.wake.revision,
            "attempt": candidate.attempt,
            "lease_until": candidate.lease_until,
            "reason": candidate.reason,
        }),
        32 * 1024,
    )?;
    let mut request_bytes = [0_u8; 16];
    request_bytes.copy_from_slice(&request_hash.as_bytes()[..16]);
    let request = crate::action::RequestUid::from_bytes(request_bytes);
    let principal = PrincipalId::Scoped {
        owner: candidate.owner.clone(),
        class: CanonicalId::new("decision_lifecycle").expect("static principal class"),
        id: IdempotencyKeyText::parse_canonical(candidate.wake.id.canonical_hash().to_hex())?,
    };
    let idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner: candidate.owner.clone(),
            namespace: CanonicalId::new(action.key().id.as_str())?,
        },
        idempotency_epoch,
        IdempotencyKey::Deterministic(request_hash),
    );
    Ok(IngressEnvelope::new(
        request,
        principal,
        IngressSurfaceVersion::new(
            CanonicalId::new("kernel.decision_fallback_ready").expect("static surface id"),
            1,
        )?,
        IngressPayload::structured(json!({
            "action": action,
            "decision": candidate.wake.id,
            "revision": candidate.wake.revision,
            "attempt": candidate.attempt,
            "lease_until": candidate.lease_until,
            "reason": candidate.reason,
        }))?,
        idempotency,
    )?)
}

/// Canonical owner-local ingress for exact cancellation. Authorization to ask
/// for cancellation remains campaign/transport policy; this kernel identity
/// cannot cancel a different controller, decision, or revision.
pub fn decision_cancellation_ingress(
    action: &DefinitionRef,
    controller: &DecisionControllerId,
    decision: DecisionId,
    revision: u64,
    idempotency_epoch: u64,
) -> Result<IngressEnvelope, DecisionCancellationIngressError> {
    if action.key().owner != controller.owner {
        return Err(DecisionCancellationIngressError::OwnerMismatch);
    }
    if revision == 0 {
        return Err(DecisionCancellationIngressError::ZeroRevision);
    }
    let request_hash = CanonicalHash::for_canonical_value(
        "decision_cancellation_request.v1",
        1,
        &json!({
            "action": action,
            "controller": controller,
            "decision": decision,
            "revision": revision,
        }),
        32 * 1024,
    )?;
    let mut request_bytes = [0_u8; 16];
    request_bytes.copy_from_slice(&request_hash.as_bytes()[..16]);
    let request = crate::action::RequestUid::from_bytes(request_bytes);
    let principal = PrincipalId::Scoped {
        owner: controller.owner.clone(),
        class: CanonicalId::new("decision_lifecycle").expect("static principal class"),
        id: IdempotencyKeyText::parse_canonical(decision.canonical_hash().to_hex())?,
    };
    let idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner: controller.owner.clone(),
            namespace: CanonicalId::new(action.key().id.as_str())?,
        },
        idempotency_epoch,
        IdempotencyKey::Deterministic(request_hash),
    );
    Ok(IngressEnvelope::new(
        request,
        principal,
        IngressSurfaceVersion::new(
            CanonicalId::new("kernel.decision_cancellation").expect("static surface id"),
            1,
        )?,
        IngressPayload::structured(json!({
            "action": action,
            "controller": controller,
            "decision": decision,
            "revision": revision,
        }))?,
        idempotency,
    )?)
}

/// Canonical internal ingress for recording one exact selected-action binding
/// on its controller owner's pending-decision record.
pub fn decision_resolving_ingress(
    action: &DefinitionRef,
    fence: &DecisionCompletionFence,
    binding: &DecisionActionBinding,
    outcome: DecisionTerminalOutcome,
    idempotency_epoch: u64,
) -> Result<IngressEnvelope, DecisionResolvingIngressError> {
    if action.key().owner != fence.controller().owner {
        return Err(DecisionResolvingIngressError::OwnerMismatch);
    }
    let request_hash = CanonicalHash::for_canonical_value(
        "decision_resolving_request.v1",
        1,
        &json!({
            "action": action,
            "decision": fence.decision(),
            "revision": fence.revision(),
            "attempt": fence.attempt(),
            "lease_until": fence.lease_until(),
            "binding": binding,
            "outcome": outcome,
        }),
        32 * 1024,
    )?;
    let mut request_bytes = [0_u8; 16];
    request_bytes.copy_from_slice(&request_hash.as_bytes()[..16]);
    let request = crate::action::RequestUid::from_bytes(request_bytes);
    let principal = PrincipalId::Scoped {
        owner: fence.controller().owner.clone(),
        class: CanonicalId::new("decision_lifecycle").expect("static principal class"),
        id: IdempotencyKeyText::parse_canonical(fence.decision().canonical_hash().to_hex())?,
    };
    let idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner: fence.controller().owner.clone(),
            namespace: CanonicalId::new(action.key().id.as_str())?,
        },
        idempotency_epoch,
        IdempotencyKey::Deterministic(request_hash),
    );
    Ok(IngressEnvelope::new(
        request,
        principal,
        IngressSurfaceVersion::new(
            CanonicalId::new("kernel.decision_resolving").expect("static surface id"),
            1,
        )?,
        IngressPayload::structured(json!({
            "action": action,
            "decision": fence.decision(),
            "revision": fence.revision(),
            "attempt": fence.attempt(),
            "lease_until": fence.lease_until(),
            "binding": binding,
            "outcome": outcome,
        }))?,
        idempotency,
    )?)
}

/// Canonical internal ingress for persisting one typed provider-attempt
/// result. Diagnostic text is intentionally excluded from action identity.
pub fn decision_attempt_ingress(
    action: &DefinitionRef,
    fence: &DecisionCompletionFence,
    kind: DecisionAttemptFailureKind,
    max_attempts: u8,
    idempotency_epoch: u64,
) -> Result<IngressEnvelope, DecisionAttemptIngressError> {
    if action.key().owner != fence.controller().owner {
        return Err(DecisionAttemptIngressError::OwnerMismatch);
    }
    if !(1..=MAX_DECISION_ATTEMPTS).contains(&max_attempts) {
        return Err(DecisionAttemptIngressError::AttemptLimit {
            actual: max_attempts,
        });
    }
    let request_hash = CanonicalHash::for_canonical_value(
        "decision_attempt_request.v1",
        1,
        &json!({
            "action": action,
            "decision": fence.decision(),
            "revision": fence.revision(),
            "attempt": fence.attempt(),
            "lease_until": fence.lease_until(),
            "failure": kind,
            "max_attempts": max_attempts,
        }),
        32 * 1024,
    )?;
    let mut request_bytes = [0_u8; 16];
    request_bytes.copy_from_slice(&request_hash.as_bytes()[..16]);
    let request = crate::action::RequestUid::from_bytes(request_bytes);
    let principal = PrincipalId::Scoped {
        owner: fence.controller().owner.clone(),
        class: CanonicalId::new("decision_lifecycle").expect("static principal class"),
        id: IdempotencyKeyText::parse_canonical(fence.decision().canonical_hash().to_hex())?,
    };
    let idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner: fence.controller().owner.clone(),
            namespace: CanonicalId::new(action.key().id.as_str())?,
        },
        idempotency_epoch,
        IdempotencyKey::Deterministic(request_hash),
    );
    Ok(IngressEnvelope::new(
        request,
        principal,
        IngressSurfaceVersion::new(
            CanonicalId::new("kernel.decision_attempt").expect("static surface id"),
            1,
        )?,
        IngressPayload::structured(json!({
            "action": action,
            "decision": fence.decision(),
            "revision": fence.revision(),
            "attempt": fence.attempt(),
            "lease_until": fence.lease_until(),
            "failure": kind,
            "max_attempts": max_attempts,
        }))?,
        idempotency,
    )?)
}

/// Canonical internal ingress for terminalizing one exact resolving binding.
pub fn decision_terminal_ingress(
    action: &DefinitionRef,
    fence: &DecisionCompletionFence,
    binding: &DecisionActionBinding,
    outcome: DecisionTerminalOutcome,
    idempotency_epoch: u64,
) -> Result<IngressEnvelope, DecisionTerminalIngressError> {
    if action.key().owner != fence.controller().owner {
        return Err(DecisionTerminalIngressError::OwnerMismatch);
    }
    let request_hash = CanonicalHash::for_canonical_value(
        "decision_terminal_request.v1",
        1,
        &json!({
            "action": action,
            "decision": fence.decision(),
            "revision": fence.revision(),
            "attempt": fence.attempt(),
            "lease_until": fence.lease_until(),
            "binding": binding,
            "outcome": outcome,
        }),
        32 * 1024,
    )?;
    let mut request_bytes = [0_u8; 16];
    request_bytes.copy_from_slice(&request_hash.as_bytes()[..16]);
    let request = crate::action::RequestUid::from_bytes(request_bytes);
    let principal = PrincipalId::Scoped {
        owner: fence.controller().owner.clone(),
        class: CanonicalId::new("decision_lifecycle").expect("static principal class"),
        id: IdempotencyKeyText::parse_canonical(fence.decision().canonical_hash().to_hex())?,
    };
    let idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner: fence.controller().owner.clone(),
            namespace: CanonicalId::new(action.key().id.as_str())?,
        },
        idempotency_epoch,
        IdempotencyKey::Deterministic(request_hash),
    );
    Ok(IngressEnvelope::new(
        request,
        principal,
        IngressSurfaceVersion::new(
            CanonicalId::new("kernel.decision_terminal").expect("static surface id"),
            1,
        )?,
        IngressPayload::structured(json!({
            "action": action,
            "decision": fence.decision(),
            "revision": fence.revision(),
            "attempt": fence.attempt(),
            "lease_until": fence.lease_until(),
            "binding": binding,
            "outcome": outcome,
        }))?,
        idempotency,
    )?)
}

/// Canonical owner-local ingress for atomically replacing a terminal decision
/// with its compact attribution receipt. The full receipt is identity-bound;
/// compaction can therefore never be replayed against a changed terminal.
pub fn decision_compaction_ingress(
    action: &DefinitionRef,
    receipt: &DecisionTerminalReceipt,
    idempotency_epoch: u64,
) -> Result<IngressEnvelope, DecisionCompactionError> {
    receipt.validate()?;
    if action.key().owner != receipt.controller.owner {
        return Err(DecisionCompactionError::OwnerMismatch);
    }
    let request_hash = CanonicalHash::for_canonical_value(
        "decision_compaction_request.v1",
        1,
        &json!({
            "action": action,
            "receipt": receipt,
        }),
        32 * 1024,
    )?;
    let mut request_bytes = [0_u8; 16];
    request_bytes.copy_from_slice(&request_hash.as_bytes()[..16]);
    let request = crate::action::RequestUid::from_bytes(request_bytes);
    let principal = PrincipalId::Scoped {
        owner: receipt.controller.owner.clone(),
        class: CanonicalId::new("decision_lifecycle").expect("static principal class"),
        id: IdempotencyKeyText::parse_canonical(receipt.id.canonical_hash().to_hex())?,
    };
    let idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner: receipt.controller.owner.clone(),
            namespace: CanonicalId::new(action.key().id.as_str())?,
        },
        idempotency_epoch,
        IdempotencyKey::Deterministic(request_hash),
    );
    Ok(IngressEnvelope::new(
        request,
        principal,
        IngressSurfaceVersion::new(
            CanonicalId::new("kernel.decision_compaction").expect("static surface id"),
            1,
        )?,
        IngressPayload::structured(json!({
            "action": action,
            "receipt": receipt,
        }))?,
        idempotency,
    )?)
}

#[derive(Debug, Error)]
pub enum DecisionSourceRegistryError {
    #[error("decision-source registry exceeds its hard limit of {max}")]
    SourceLimit { max: usize },
    #[error("decision source owner must own its lease action and all typed families")]
    OwnerMismatch,
    #[error("decision source lease definition is not an action")]
    ActionKind,
    #[error("decision lease duration {actual} must be in 1..={max} clock units")]
    LeaseDuration { actual: i64, max: i64 },
    #[error("decision source is registered more than once for owner {owner}")]
    Duplicate { owner: OwnerId },
    #[error("decision source names an uncomposed owner {owner}")]
    UnknownOwner { owner: OwnerId },
    #[error("decision source for {owner} has no exact lease action provider: {source}")]
    Action {
        owner: OwnerId,
        source: Box<ActionProviderLookupError>,
    },
    #[error("decision source for {owner} has an invalid typed family: {source}")]
    Family {
        owner: OwnerId,
        source: Box<ActionStateError>,
    },
    #[error("decision-source fingerprint is not canonical: {0}")]
    Fingerprint(#[from] CanonicalEncodingError),
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DecisionLeaseIngressError {
    #[error("decision lease candidate is invalid: {0}")]
    Candidate(#[from] DecisionSourceCandidateError),
    #[error("decision lease action and source have different owners")]
    OwnerMismatch,
    #[error("decision lease hashing failed: {0}")]
    Hash(#[from] CanonicalHashError),
    #[error("decision lease metadata is invalid: {0}")]
    Metadata(#[from] ActionMetadataError),
    #[error("decision lease identity is invalid: {0}")]
    CanonicalId(#[from] CanonicalIdError),
    #[error("decision lease idempotency text is invalid: {0}")]
    IdempotencyText(#[from] IdempotencyKeyTextError),
}

#[derive(Debug, Error)]
pub enum DecisionResolvingIngressError {
    #[error("decision resolving source registry is unavailable")]
    RegistryUnavailable,
    #[error("decision resolving source for owner {owner} is unavailable")]
    SourceUnavailable { owner: OwnerId },
    #[error("decision resolving action and controller have different owners")]
    OwnerMismatch,
    #[error("decision resolving references a missing pending record")]
    MissingDecision,
    #[error("decision resolving does not match the exact leased record")]
    StaleCompletion,
    #[error(transparent)]
    State(#[from] ActionStateError),
    #[error(transparent)]
    Hash(#[from] CanonicalHashError),
    #[error(transparent)]
    Metadata(#[from] ActionMetadataError),
    #[error(transparent)]
    CanonicalId(#[from] CanonicalIdError),
    #[error(transparent)]
    IdempotencyText(#[from] IdempotencyKeyTextError),
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

#[derive(Debug, Error)]
pub enum DecisionAttemptIngressError {
    #[error("decision attempt source registry is unavailable")]
    RegistryUnavailable,
    #[error("decision attempt source for owner {owner} is unavailable")]
    SourceUnavailable { owner: OwnerId },
    #[error("decision attempt action and controller have different owners")]
    OwnerMismatch,
    #[error("decision attempt references a missing pending record")]
    MissingDecision,
    #[error("decision attempt does not match the exact leased record")]
    StaleAttempt,
    #[error("decision attempt limit {actual} is outside the kernel bound")]
    AttemptLimit { actual: u8 },
    #[error(transparent)]
    State(#[from] ActionStateError),
    #[error(transparent)]
    Hash(#[from] CanonicalHashError),
    #[error(transparent)]
    Metadata(#[from] ActionMetadataError),
    #[error(transparent)]
    CanonicalId(#[from] CanonicalIdError),
    #[error(transparent)]
    IdempotencyText(#[from] IdempotencyKeyTextError),
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

#[derive(Debug, Error)]
pub enum DecisionAttemptPlanError {
    #[error("decision attempt source registry is unavailable")]
    RegistryUnavailable,
    #[error(transparent)]
    Ingress(#[from] DecisionAttemptIngressError),
    #[error("decision attempt ingress differs from its canonical fenced payload")]
    IngressMismatch,
    #[error(transparent)]
    Decision(#[from] DecisionError),
    #[error(transparent)]
    Guard(#[from] crate::action::GuardSetBuildError),
    #[error(transparent)]
    Metadata(#[from] ActionMetadataError),
    #[error(transparent)]
    Hash(#[from] CanonicalHashError),
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

#[derive(Debug, Error)]
pub enum DecisionFallbackTriggerError {
    #[error("decision fallback trigger source registry is unavailable")]
    RegistryUnavailable,
    #[error("decision fallback trigger source for owner {owner} is unavailable")]
    SourceUnavailable { owner: OwnerId },
    #[error("decision fallback trigger action and source have different owners")]
    OwnerMismatch,
    #[error("decision fallback trigger references a missing pending record")]
    MissingDecision,
    #[error("decision fallback trigger candidate is stale")]
    StaleCandidate,
    #[error("decision fallback trigger reason is not valid for this lifecycle path")]
    InvalidReason,
    #[error("decision fallback deadline has not been reached")]
    DeadlineNotReached,
    #[error(transparent)]
    Candidate(#[from] DecisionSourceCandidateError),
    #[error(transparent)]
    Decision(#[from] DecisionError),
    #[error(transparent)]
    State(#[from] ActionStateError),
    #[error(transparent)]
    Clock(#[from] crate::clock_provider::ClockReadError),
    #[error(transparent)]
    Hash(#[from] CanonicalHashError),
    #[error(transparent)]
    Metadata(#[from] ActionMetadataError),
    #[error(transparent)]
    CanonicalId(#[from] CanonicalIdError),
    #[error(transparent)]
    IdempotencyText(#[from] IdempotencyKeyTextError),
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

#[derive(Debug, Error)]
pub enum DecisionFallbackTriggerPlanError {
    #[error("decision fallback trigger source registry is unavailable")]
    RegistryUnavailable,
    #[error(transparent)]
    Trigger(#[from] DecisionFallbackTriggerError),
    #[error("decision fallback trigger ingress differs from its canonical fenced payload")]
    IngressMismatch,
    #[error("decision fallback trigger candidate changed before planning")]
    StaleCandidate,
    #[error(transparent)]
    Decision(#[from] DecisionError),
    #[error(transparent)]
    Clock(#[from] crate::clock_provider::ClockReadError),
    #[error(transparent)]
    Guard(#[from] crate::action::GuardSetBuildError),
    #[error(transparent)]
    Metadata(#[from] ActionMetadataError),
    #[error(transparent)]
    Hash(#[from] CanonicalHashError),
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

#[derive(Debug, Error)]
pub enum DecisionCancellationIngressError {
    #[error("decision cancellation source registry is unavailable")]
    RegistryUnavailable,
    #[error("decision cancellation source for owner {owner} is unavailable")]
    SourceUnavailable { owner: OwnerId },
    #[error("decision cancellation action and controller have different owners")]
    OwnerMismatch,
    #[error("decision cancellation references a missing pending record")]
    MissingDecision,
    #[error("decision cancellation revision must be non-zero")]
    ZeroRevision,
    #[error(transparent)]
    Decision(#[from] DecisionError),
    #[error(transparent)]
    State(#[from] ActionStateError),
    #[error(transparent)]
    Hash(#[from] CanonicalHashError),
    #[error(transparent)]
    Metadata(#[from] ActionMetadataError),
    #[error(transparent)]
    CanonicalId(#[from] CanonicalIdError),
    #[error(transparent)]
    IdempotencyText(#[from] IdempotencyKeyTextError),
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

#[derive(Debug, Error)]
pub enum DecisionCancellationPlanError {
    #[error("decision cancellation source registry is unavailable")]
    RegistryUnavailable,
    #[error(transparent)]
    Ingress(#[from] DecisionCancellationIngressError),
    #[error("decision cancellation ingress differs from its canonical exact-revision payload")]
    IngressMismatch,
    #[error(transparent)]
    Decision(#[from] DecisionError),
    #[error(transparent)]
    State(#[from] ActionStateError),
    #[error(transparent)]
    Guard(#[from] crate::action::GuardSetBuildError),
    #[error(transparent)]
    Metadata(#[from] ActionMetadataError),
    #[error(transparent)]
    Hash(#[from] CanonicalHashError),
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

#[derive(Debug, Error)]
pub enum DecisionFallbackSelectionError {
    #[error("decision fallback definition registry is unavailable")]
    RegistryUnavailable,
    #[error("decision fallback source for owner {owner} is unavailable")]
    SourceUnavailable { owner: OwnerId },
    #[error("decision fallback references a missing pending record")]
    MissingDecision,
    #[error("decision fallback references a missing prompt record")]
    MissingPrompt,
    #[error("decision fallback candidate is stale")]
    StaleCandidate,
    #[error("decision fallback selected an action absent from the prompt")]
    OfferNotPrompted,
    #[error("decision fallback adapter rejected the prompt: {0}")]
    Adapter(String),
    #[error("decision fallback definition is unavailable: {0}")]
    Definition(#[source] Box<DefinitionLookupError>),
    #[error(transparent)]
    Decision(#[from] DecisionError),
    #[error(transparent)]
    State(#[from] ActionStateError),
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

#[derive(Debug, Error)]
pub enum DecisionResolvingPlanError {
    #[error("decision resolving source registry is unavailable")]
    RegistryUnavailable,
    #[error(transparent)]
    Ingress(#[from] DecisionResolvingIngressError),
    #[error("decision resolving ingress differs from its canonical fenced payload")]
    IngressMismatch,
    #[error(transparent)]
    Decision(#[from] DecisionError),
    #[error(transparent)]
    Guard(#[from] crate::action::GuardSetBuildError),
    #[error(transparent)]
    Metadata(#[from] ActionMetadataError),
    #[error(transparent)]
    Hash(#[from] CanonicalHashError),
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

#[derive(Debug, Error)]
pub enum DecisionTerminalIngressError {
    #[error("decision terminal source registry is unavailable")]
    RegistryUnavailable,
    #[error("decision terminal source for owner {owner} is unavailable")]
    SourceUnavailable { owner: OwnerId },
    #[error("decision terminal action and controller have different owners")]
    OwnerMismatch,
    #[error("decision terminal transition references a missing pending record")]
    MissingDecision,
    #[error("decision terminal transition does not match the exact resolving binding")]
    StaleResolution,
    #[error(transparent)]
    State(#[from] ActionStateError),
    #[error(transparent)]
    Hash(#[from] CanonicalHashError),
    #[error(transparent)]
    Metadata(#[from] ActionMetadataError),
    #[error(transparent)]
    CanonicalId(#[from] CanonicalIdError),
    #[error(transparent)]
    IdempotencyText(#[from] IdempotencyKeyTextError),
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

#[derive(Debug, Error)]
pub enum DecisionTerminalPlanError {
    #[error("decision terminal source registry is unavailable")]
    RegistryUnavailable,
    #[error(transparent)]
    Ingress(#[from] DecisionTerminalIngressError),
    #[error("decision terminal ingress differs from its canonical fenced payload")]
    IngressMismatch,
    #[error(transparent)]
    Decision(#[from] DecisionError),
    #[error(transparent)]
    Guard(#[from] crate::action::GuardSetBuildError),
    #[error(transparent)]
    Metadata(#[from] ActionMetadataError),
    #[error(transparent)]
    Hash(#[from] CanonicalHashError),
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

#[derive(Debug, Error)]
pub enum DecisionCompactionError {
    #[error("decision compaction source registry or deployment key ring is unavailable")]
    RegistryUnavailable,
    #[error("decision compaction source for owner {owner} is unavailable")]
    SourceUnavailable { owner: OwnerId },
    #[error("decision compaction action and terminal receipt have different owners")]
    OwnerMismatch,
    #[error("decision compaction references a missing terminal pending record")]
    MissingDecision,
    #[error("decision compaction references a missing prompt record")]
    MissingPrompt,
    #[error("decision compaction found both terminal and compact representations")]
    DuplicateRepresentation,
    #[error("decision compaction candidate changed before preparation")]
    StaleCandidate,
    #[error("decision compaction ingress differs from its canonical terminal receipt")]
    IngressMismatch,
    #[error("decision compaction deadline linkage is invalid: {0}")]
    Deadline(#[source] Box<DecisionSourceScanError>),
    #[error(transparent)]
    Decision(#[from] DecisionError),
    #[error(transparent)]
    State(#[from] ActionStateError),
    #[error(transparent)]
    Guard(#[from] crate::action::GuardSetBuildError),
    #[error(transparent)]
    Metadata(#[from] ActionMetadataError),
    #[error(transparent)]
    Hash(#[from] CanonicalHashError),
    #[error(transparent)]
    CanonicalId(#[from] CanonicalIdError),
    #[error(transparent)]
    IdempotencyText(#[from] IdempotencyKeyTextError),
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

#[derive(Debug, Error)]
pub enum DecisionResponseIngressError {
    #[error("decision response source registry or deployment key ring is unavailable")]
    RegistryUnavailable,
    #[error("decision response source for owner {owner} is unavailable")]
    SourceUnavailable { owner: OwnerId },
    #[error("decision response references a missing pending record")]
    MissingDecision,
    #[error("decision response references a missing prompt record")]
    MissingPrompt,
    #[error("decision response does not match the exact live lease")]
    StaleCompletion,
    #[error("decision response selected an action absent from its prompt")]
    OfferNotPrompted,
    #[error(transparent)]
    Decision(#[from] DecisionError),
    #[error(transparent)]
    State(#[from] ActionStateError),
    #[error(transparent)]
    Clock(#[from] crate::clock_provider::ClockReadError),
    #[error(transparent)]
    Hash(#[from] CanonicalHashError),
    #[error(transparent)]
    Metadata(#[from] ActionMetadataError),
    #[error(transparent)]
    CanonicalId(#[from] CanonicalIdError),
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

#[derive(Debug, Error)]
pub enum DecisionResponseResolveError {
    #[error(transparent)]
    Ingress(#[from] DecisionResponseIngressError),
    #[error("decision response ingress differs from its canonical fenced payload")]
    IngressMismatch,
    #[error("decision response selected an action absent from its prompt")]
    OfferNotPrompted,
    #[error("decision response selected a target absent from its prompt")]
    TargetNotPrompted,
    #[error("decision response subject projection changed since capture")]
    ProjectionChanged,
    #[error("decision response registry is unavailable")]
    RegistryUnavailable,
    #[error("decision response subject is no longer legal: {0}")]
    Subject(#[source] crate::offer::LegalActionSubjectError),
    #[error("decision response action projection failed: {0}")]
    Projection(#[source] crate::offer::LegalActionProjectionError),
    #[error("decision response action resolution failed: {0}")]
    Offer(#[source] crate::offer::LegalActionResolutionError),
    #[error(transparent)]
    Guard(#[from] crate::action::GuardSetBuildError),
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DecisionSourceCandidateError {
    #[error("decision candidate belongs to another mechanics composition")]
    RegistryMismatch,
    #[error("decision source for owner {owner} is unavailable")]
    SourceUnavailable { owner: OwnerId },
    #[error("decision candidate identity does not match its source")]
    IdentityMismatch,
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

#[derive(Debug, Error)]
pub enum DecisionSourcePlanError {
    #[error("decision-source registry or deployment key ring is unavailable")]
    RegistryUnavailable,
    #[error(transparent)]
    Candidate(#[from] DecisionSourceCandidateError),
    #[error(transparent)]
    Ingress(#[from] DecisionLeaseIngressError),
    #[error("decision lease ingress does not match its canonical candidate")]
    IngressMismatch,
    #[error("decision record disappeared before lease planning")]
    MissingDecision,
    #[error("decision prompt disappeared before lease planning")]
    MissingPrompt,
    #[error("decision deadline linkage is invalid: {0}")]
    DeadlineLinkage(#[source] DecisionSourceScanError),
    #[error("decision candidate changed before lease planning")]
    StaleCandidate,
    #[error(transparent)]
    Decision(#[from] DecisionError),
    #[error(transparent)]
    State(#[from] ActionStateError),
    #[error(transparent)]
    Clock(#[from] crate::clock_provider::ClockReadError),
    #[error(transparent)]
    Guard(#[from] crate::action::GuardSetBuildError),
    #[error(transparent)]
    Metadata(#[from] ActionMetadataError),
    #[error(transparent)]
    Hash(#[from] CanonicalHashError),
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

#[derive(Debug, Error)]
pub enum DecisionSourceScanError {
    #[error("decision-source registry or deployment key ring is unavailable")]
    RegistryUnavailable,
    #[error("decision recovery limit {actual} must be in 1..={max}")]
    InvalidLimit { actual: usize, max: usize },
    #[error("decision recovery cursor names an unavailable owner {owner}")]
    StaleCursor { owner: OwnerId },
    #[error("decision record key/owner does not match source {owner}: {encoded_key}")]
    RecordIdentity {
        owner: OwnerId,
        encoded_key: SmolStr,
    },
    #[error("decision {decision:?} for {owner} has no exact prompt record")]
    MissingPrompt {
        owner: OwnerId,
        decision: DecisionId,
    },
    #[error("decision {decision:?} for {owner} has no exact deadline schedule")]
    MissingDeadline {
        owner: OwnerId,
        decision: DecisionId,
    },
    #[error("decision {decision:?} for {owner} has mismatched deadline linkage")]
    DeadlineLinkage {
        owner: OwnerId,
        decision: DecisionId,
    },
    #[error("decision {decision:?} for {owner} exists as both pending and compact receipt")]
    DuplicateRepresentation {
        owner: OwnerId,
        decision: DecisionId,
    },
    #[error("decision {decision:?} for {owner} cannot read its clock: {source}")]
    Clock {
        owner: OwnerId,
        decision: DecisionId,
        source: Box<crate::clock_provider::ClockReadError>,
    },
    #[error(transparent)]
    Decision(#[from] DecisionError),
    #[error(transparent)]
    State(#[from] ActionStateError),
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

impl DecisionSourceScanError {
    #[must_use]
    pub fn owner(&self) -> Option<&OwnerId> {
        match self {
            Self::StaleCursor { owner }
            | Self::RecordIdentity { owner, .. }
            | Self::MissingPrompt { owner, .. }
            | Self::MissingDeadline { owner, .. }
            | Self::DeadlineLinkage { owner, .. }
            | Self::DuplicateRepresentation { owner, .. }
            | Self::Clock { owner, .. } => Some(owner),
            Self::OwnerNotReady(MechanicalOwnerReadinessError::UnknownOwner { owner })
            | Self::OwnerNotReady(MechanicalOwnerReadinessError::Frozen { owner, .. }) => {
                Some(owner)
            }
            Self::InvalidLimit { .. }
            | Self::Decision(_)
            | Self::State(_)
            | Self::OwnerNotReady(MechanicalOwnerReadinessError::RegistryUnavailable) => None,
            Self::RegistryUnavailable => None,
        }
    }
}

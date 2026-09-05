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
use crate::decision::{
    DecisionCaptureFamilies, DecisionControllerId, DecisionError, DecisionId, DecisionKeyRing,
    DecisionLeaseTransition, DecisionWake, DecisionWorkerLease, MAX_DECISION_ATTEMPTS,
    MAX_DECISION_RECOVERY_PAGE, PendingDecisionStatus,
};
use crate::durable_schedule::ScheduleId;
use crate::mechanics::{
    CanonicalEncodingError, CanonicalId, CanonicalIdError, DefinitionDigest, DefinitionRef,
    MechanicalOwnerReadiness, MechanicalOwnerReadinessError, OwnerId,
};

/// Maximum composed owners that may publish delayed-decision families.
pub const MAX_DECISION_SOURCES: usize = 256;

#[derive(Clone)]
struct DecisionSourceEntry {
    owner: OwnerId,
    lease_action: DefinitionRef,
    families: DecisionCaptureFamilies,
    wake_adapter: CanonicalId,
}

impl std::fmt::Debug for DecisionSourceEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DecisionSourceEntry")
            .field("owner", &self.owner)
            .field("lease_action", &self.lease_action)
            .field("families", &self.families)
            .field("wake_adapter", &self.wake_adapter)
            .finish()
    }
}

/// Mutable composition-only delayed-decision source table.
#[derive(Default)]
pub struct DecisionSourceRegistryBuilder {
    entries: BTreeMap<OwnerId, DecisionSourceEntry>,
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
                            "wake_adapter": entry.wake_adapter,
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
    records_scanned: usize,
    next_cursor: Option<DecisionSourceCursor>,
}

impl DecisionSourcePage {
    #[must_use]
    pub fn candidates(&self) -> &[DecisionDispatchCandidate] {
        &self.candidates
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
        let mut records_scanned = 0;
        let mut last_scanned = None;
        for (owner, entry) in &self.entries {
            if cursor.is_some_and(|cursor| owner < &cursor.owner) {
                continue;
            }
            self.owner_readiness.require_ready(owner)?;
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
                if now.at >= pending.deadline.at {
                    continue;
                }
                let attempt = match &pending.status {
                    PendingDecisionStatus::Ready { attempt } => Some(*attempt),
                    PendingDecisionStatus::Leased {
                        attempt,
                        lease_until,
                    } if lease_until.checked_cmp(&now).is_ok()
                        && now.at >= lease_until.at
                        && *attempt < MAX_DECISION_ATTEMPTS =>
                    {
                        Some(*attempt + 1)
                    }
                    _ => None,
                };
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
            }
            self.owner_readiness.require_ready(owner)?;
            if records_scanned == limit {
                return Ok(DecisionSourcePage {
                    candidates,
                    records_scanned,
                    next_cursor: last_scanned,
                });
            }
        }
        Ok(DecisionSourcePage {
            candidates,
            records_scanned,
            next_cursor: None,
        })
    }

    pub(crate) fn plan_candidate(
        &self,
        engine: &crate::engine::Engine,
        keys: &DecisionKeyRing,
        candidate: &DecisionDispatchCandidate,
        ingress: &IngressEnvelope,
        lease_duration: i64,
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
            ready.plan_worker_lease(prompt, &candidate.wake, keys, &now, lease_duration)?;
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

#[derive(Debug, Error)]
pub enum DecisionSourceRegistryError {
    #[error("decision-source registry exceeds its hard limit of {max}")]
    SourceLimit { max: usize },
    #[error("decision source owner must own its lease action and all typed families")]
    OwnerMismatch,
    #[error("decision source lease definition is not an action")]
    ActionKind,
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

//! Composition-frozen discovery of durable schedule families.
//!
//! A source is declarative: one owner-local `ScheduleId -> DurableSchedule`
//! typed family plus the exact authoritative action that advances it. The
//! kernel scans records in canonical bounded pages and evaluates clocks; no
//! provider callback can hide an unbounded index or mutate state while
//! discovery runs.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde_json::{Value, json};
use smol_str::SmolStr;
use thiserror::Error;

use crate::action::{
    ActionError, ActionInvocation, ActionInvocationDraft, ActionMetadataError, ActionOfferId,
    ActionProviderLookupError, ActionProviderRegistry, CanonicalHash, CanonicalHashError, GuardSet,
    IdempotencyKey, IdempotencyKeyText, IdempotencyKeyTextError, IdempotencyScope, IngressEnvelope,
    IngressPayload, IngressSurfaceVersion, PerceivedRef, PrincipalId, ScopedIdempotencyKey,
};
use crate::action_state::{ActionStateError, ActionTxn, StateFamilyHandle, StateFamilyRegistry};
use crate::clock_provider::{ClockProviderFailure, ClockReadError};
use crate::durable_schedule::{
    DurableSchedule, DurableScheduleError, ScheduleAdvance, ScheduleDriveCause, ScheduleEvaluation,
    ScheduleId,
};
use crate::mechanics::{
    CanonicalEncodingError, CanonicalId, CanonicalIdError, DefinitionDigest, DefinitionRef,
    MechanicalOwnerReadiness, MechanicalOwnerReadinessError, OwnerId,
};
use crate::obj::ObjId;

/// Maximum schedule families registered in one deployment.
pub const MAX_SCHEDULE_SOURCES: usize = 4_096;
/// Maximum schedule families registered by one owner.
pub const MAX_SCHEDULE_SOURCES_PER_OWNER: usize = 1_024;
/// Maximum durable schedule records inspected in one world step.
pub const MAX_SCHEDULE_SOURCE_SCAN_RECORDS: usize = 256;

/// Exact owner-stamped schedule-source identity.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ScheduleSourceKey {
    owner: OwnerId,
    id: CanonicalId,
}

impl ScheduleSourceKey {
    #[must_use]
    pub const fn new(owner: OwnerId, id: CanonicalId) -> Self {
        Self { owner, id }
    }

    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn id(&self) -> &CanonicalId {
        &self.id
    }
}

/// Provider-owned portion of a trusted internal scheduled invocation.
///
/// The kernel supplies request, principal, idempotency, exact action route,
/// and schedule cursor identity. The adapter supplies only domain addressing,
/// parameters, and server-created guards after mandatory store preflight.
pub struct ScheduleActionDraft {
    pub actor: ObjId,
    pub subject: PerceivedRef,
    pub targets: Vec<PerceivedRef>,
    pub parameters: Value,
    pub guards: GuardSet,
}

/// Read-only provider adapter from a due cursor to its domain invocation.
///
/// This callback runs only after exact idempotency preflight returns vacant.
/// It may inspect immutable engine truth, but mutation remains exclusively in
/// the registered [`crate::action::ActionProvider`].
pub trait ScheduleActionAdapter: Send + Sync {
    fn plan(
        &self,
        action: &DefinitionRef,
        advance: &ScheduleAdvance,
        engine: &crate::engine::Engine,
    ) -> Result<ScheduleActionDraft, ActionError>;
}

/// Exact internal action plan produced after schedule preflight.
#[derive(Debug)]
pub struct ScheduledActionPlan {
    action: DefinitionRef,
    invocation: ActionInvocation,
    guards: GuardSet,
}

impl ScheduledActionPlan {
    #[must_use]
    pub const fn action(&self) -> &DefinitionRef {
        &self.action
    }

    #[must_use]
    pub const fn invocation(&self) -> &ActionInvocation {
        &self.invocation
    }

    #[must_use]
    pub const fn guards(&self) -> &GuardSet {
        &self.guards
    }

    #[must_use]
    pub fn into_parts(self) -> (DefinitionRef, ActionInvocation, GuardSet) {
        (self.action, self.invocation, self.guards)
    }
}

/// Build the one canonical K3 ingress identity for a durable schedule
/// pre-state under the caller's current server idempotency epoch.
///
/// The gameplay schedule generation remains in the deterministic key and
/// payload. `idempotency_epoch` is deliberately separate: it belongs to the
/// store's retry-retention horizon, not to any gameplay clock.
pub fn scheduled_action_ingress(
    action: &DefinitionRef,
    advance: &ScheduleAdvance,
    idempotency_epoch: u64,
) -> Result<IngressEnvelope, ScheduleActionIngressError> {
    if advance.owner() != &action.key().owner {
        return Err(ScheduleActionIngressError::OwnerMismatch);
    }
    let request_hash = CanonicalHash::for_canonical_value(
        "scheduled_action_request.v1",
        1,
        &json!({
            "action": action,
            "schedule": advance.schedule(),
            "generation": advance.generation(),
            "advance_key": advance.deterministic_key(),
        }),
        32 * 1024,
    )?;
    let mut request_bytes = [0_u8; 16];
    request_bytes.copy_from_slice(&request_hash.as_bytes()[..16]);
    let request = crate::action::RequestUid::from_bytes(request_bytes);
    let principal = PrincipalId::Scoped {
        owner: advance.owner().clone(),
        class: CanonicalId::new("scheduler").expect("static principal class"),
        id: IdempotencyKeyText::parse_canonical(advance.schedule().canonical_hash().to_hex())?,
    };
    let idempotency = ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: principal.clone(),
            owner: advance.owner().clone(),
            namespace: CanonicalId::new(action.key().id.as_str())?,
        },
        idempotency_epoch,
        IdempotencyKey::Deterministic(advance.deterministic_key()),
    );
    Ok(IngressEnvelope::new(
        request,
        principal,
        IngressSurfaceVersion::new(
            CanonicalId::new("kernel.schedule").expect("static surface id"),
            1,
        )?,
        IngressPayload::structured(json!({
            "action": action,
            "schedule": advance.schedule(),
            "generation": advance.generation(),
            "advance_key": advance.deterministic_key(),
        }))?,
        idempotency,
    )?)
}

#[derive(Clone)]
struct ScheduleSourceEntry {
    key: ScheduleSourceKey,
    action: DefinitionRef,
    schedules: StateFamilyHandle<ScheduleId, DurableSchedule>,
    adapter: Arc<dyn ScheduleActionAdapter>,
}

impl std::fmt::Debug for ScheduleSourceEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScheduleSourceEntry")
            .field("key", &self.key)
            .field("action", &self.action)
            .field("schedules", &self.schedules)
            .finish_non_exhaustive()
    }
}

/// Mutable composition-only source table.
#[derive(Default)]
pub struct ScheduleSourceRegistryBuilder {
    entries: BTreeMap<ScheduleSourceKey, ScheduleSourceEntry>,
    owner_counts: BTreeMap<OwnerId, usize>,
}

impl std::fmt::Debug for ScheduleSourceRegistryBuilder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScheduleSourceRegistryBuilder")
            .field("keys", &self.entries.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl ScheduleSourceRegistryBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn register(
        &mut self,
        owner: OwnerId,
        id: CanonicalId,
        action: DefinitionRef,
        schedules: StateFamilyHandle<ScheduleId, DurableSchedule>,
        adapter: Arc<dyn ScheduleActionAdapter>,
    ) -> Result<(), ScheduleSourceRegistryError> {
        if self.entries.len() >= MAX_SCHEDULE_SOURCES {
            return Err(ScheduleSourceRegistryError::SourceLimit {
                max: MAX_SCHEDULE_SOURCES,
            });
        }
        let owner_count = self.owner_counts.get(&owner).copied().unwrap_or(0);
        if owner_count >= MAX_SCHEDULE_SOURCES_PER_OWNER {
            return Err(ScheduleSourceRegistryError::OwnerSourceLimit {
                owner,
                max: MAX_SCHEDULE_SOURCES_PER_OWNER,
            });
        }
        if schedules.owner() != &owner || action.key().owner != owner {
            return Err(ScheduleSourceRegistryError::OwnerMismatch);
        }
        if action.key().kind.as_str() != "action" {
            return Err(ScheduleSourceRegistryError::ActionKind);
        }
        let key = ScheduleSourceKey::new(owner.clone(), id);
        if self.entries.contains_key(&key) {
            return Err(ScheduleSourceRegistryError::Duplicate { key });
        }
        self.entries.insert(
            key.clone(),
            ScheduleSourceEntry {
                key,
                action,
                schedules,
                adapter,
            },
        );
        self.owner_counts.insert(owner, owner_count + 1);
        Ok(())
    }

    pub(crate) fn freeze_for_composition(
        self,
        module_owners: &BTreeSet<OwnerId>,
        actions: &ActionProviderRegistry,
        state_families: &StateFamilyRegistry,
        owner_readiness: MechanicalOwnerReadiness,
    ) -> Result<ScheduleSourceRegistry, ScheduleSourceRegistryError> {
        for entry in self.entries.values() {
            if !module_owners.contains(entry.key.owner()) {
                return Err(ScheduleSourceRegistryError::UnknownOwner {
                    key: entry.key.clone(),
                });
            }
            actions.get_exact(&entry.action).map_err(|source| {
                ScheduleSourceRegistryError::Action {
                    key: entry.key.clone(),
                    source: Box::new(source),
                }
            })?;
            state_families
                .validate_handle(&entry.schedules)
                .map_err(|source| ScheduleSourceRegistryError::Family {
                    key: entry.key.clone(),
                    source: Box::new(source),
                })?;
        }
        let fingerprint = DefinitionDigest::for_canonical_value(
            "schedule_source_registry.v1",
            &Value::Array(
                self.entries
                    .values()
                    .map(|entry| {
                        json!({
                            "owner": entry.key.owner(),
                            "id": entry.key.id(),
                            "action": entry.action,
                            "family": entry.schedules.descriptor().canonical_value(),
                        })
                    })
                    .collect(),
            ),
        )?;
        Ok(ScheduleSourceRegistry {
            entries: self.entries,
            fingerprint,
            owner_readiness,
        })
    }
}

/// Opaque process-local cursor over canonical source/family order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleSourceCursor {
    source: ScheduleSourceKey,
    after_encoded_key: SmolStr,
}

impl ScheduleSourceCursor {
    #[must_use]
    pub const fn source(&self) -> &ScheduleSourceKey {
        &self.source
    }

    #[must_use]
    pub const fn after_encoded_key(&self) -> &SmolStr {
        &self.after_encoded_key
    }
}

/// One due source result ready for trusted provider-specific invocation
/// construction and normal coordinator preflight.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduledActionCandidate {
    registry_fingerprint: DefinitionDigest,
    source: ScheduleSourceKey,
    encoded_key: SmolStr,
    action: DefinitionRef,
    advance: ScheduleAdvance,
}

impl ScheduledActionCandidate {
    #[must_use]
    pub const fn source(&self) -> &ScheduleSourceKey {
        &self.source
    }

    #[must_use]
    pub const fn encoded_key(&self) -> &SmolStr {
        &self.encoded_key
    }

    #[must_use]
    pub const fn action(&self) -> &DefinitionRef {
        &self.action
    }

    #[must_use]
    pub const fn advance(&self) -> &ScheduleAdvance {
        &self.advance
    }
}

/// Bounded result of one discovery step.
#[derive(Debug)]
pub struct ScheduleSourcePage {
    candidates: Vec<ScheduledActionCandidate>,
    records_scanned: usize,
    next_cursor: Option<ScheduleSourceCursor>,
}

impl ScheduleSourcePage {
    #[must_use]
    pub fn candidates(&self) -> &[ScheduledActionCandidate] {
        &self.candidates
    }

    #[must_use]
    pub const fn records_scanned(&self) -> usize {
        self.records_scanned
    }

    #[must_use]
    pub const fn next_cursor(&self) -> Option<&ScheduleSourceCursor> {
        self.next_cursor.as_ref()
    }

    #[must_use]
    pub fn into_parts(self) -> (Vec<ScheduledActionCandidate>, Option<ScheduleSourceCursor>) {
        (self.candidates, self.next_cursor)
    }
}

/// Frozen declarative schedule-source registry.
pub struct ScheduleSourceRegistry {
    entries: BTreeMap<ScheduleSourceKey, ScheduleSourceEntry>,
    fingerprint: DefinitionDigest,
    owner_readiness: MechanicalOwnerReadiness,
}

impl std::fmt::Debug for ScheduleSourceRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScheduleSourceRegistry")
            .field("keys", &self.entries.keys().collect::<Vec<_>>())
            .field("fingerprint", &self.fingerprint)
            .finish()
    }
}

impl ScheduleSourceRegistry {
    #[must_use]
    pub const fn fingerprint(&self) -> DefinitionDigest {
        self.fingerprint
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn plan_candidate(
        &self,
        engine: &crate::engine::Engine,
        candidate: &ScheduledActionCandidate,
        ingress: &IngressEnvelope,
    ) -> Result<ScheduledActionPlan, ScheduleActionPlanError> {
        let entry = self.entry_for_candidate(candidate)?;
        let expected = scheduled_action_ingress(
            &entry.action,
            &candidate.advance,
            ingress.idempotency().epoch(),
        )?;
        if &expected != ingress {
            return Err(ScheduleActionPlanError::IngressMismatch);
        }
        let draft = entry
            .adapter
            .plan(&entry.action, &candidate.advance, engine)
            .map_err(|source| ScheduleActionPlanError::Adapter {
                source_key: entry.key.clone(),
                source: Box::new(source),
            })?;
        self.owner_readiness.require_ready(entry.key.owner())?;
        if !draft.guards.authority().iter().any(
            |guard| matches!(guard, crate::action::AuthorityGuard::Definition(reference) if reference == &entry.action),
        ) {
            return Err(ScheduleActionPlanError::MissingDefinitionGuard {
                action: Box::new(entry.action.clone()),
            });
        }
        let offered_action = ActionOfferId(CanonicalHash::for_canonical_value(
            "scheduled_action_offer.v1",
            1,
            &json!({
                "action": entry.action,
                "schedule": candidate.advance.schedule(),
                "generation": candidate.advance.generation(),
                "advance_key": candidate.advance.deterministic_key(),
            }),
            32 * 1024,
        )?);
        let invocation = ActionInvocation::new(ActionInvocationDraft {
            request: ingress.request(),
            principal: ingress.principal().clone(),
            actor: draft.actor,
            offered_action,
            subject: draft.subject,
            targets: draft.targets,
            parameters: draft.parameters,
            idempotency: ingress.idempotency().clone(),
        })?;
        self.owner_readiness.require_ready(entry.key.owner())?;
        Ok(ScheduledActionPlan {
            action: entry.action.clone(),
            invocation,
            guards: draft.guards,
        })
    }

    pub(crate) fn seed_advance(
        &self,
        candidate: &ScheduledActionCandidate,
        txn: &mut ActionTxn<'_>,
    ) -> Result<(), ScheduleAdvanceAdmissionError> {
        let entry = self.entry_for_candidate(candidate)?;
        let current = txn
            .get(&entry.schedules, &candidate.advance.schedule())?
            .cloned()
            .ok_or_else(|| ScheduleAdvanceAdmissionError::MissingSchedule {
                source_key: entry.key.clone(),
                schedule: candidate.advance.schedule(),
            })?;
        let evaluation = current.plan_due(
            current.revision(),
            candidate.advance.observed_at(),
            candidate.advance.cause(),
        )?;
        if !matches!(
            evaluation,
            ScheduleEvaluation::Advance(ref planned) if planned.as_ref() == &candidate.advance
        ) {
            return Err(ScheduleAdvanceAdmissionError::StalePlan {
                source_key: entry.key.clone(),
                schedule: candidate.advance.schedule(),
            });
        }
        txn.replace(
            &entry.schedules,
            &candidate.advance.schedule(),
            candidate.advance.post_image().clone(),
        )?;
        self.owner_readiness.require_ready(entry.key.owner())?;
        Ok(())
    }

    fn entry_for_candidate(
        &self,
        candidate: &ScheduledActionCandidate,
    ) -> Result<&ScheduleSourceEntry, ScheduleCandidateError> {
        if candidate.registry_fingerprint != self.fingerprint {
            return Err(ScheduleCandidateError::RegistryMismatch);
        }
        let entry = self.entries.get(candidate.source()).ok_or_else(|| {
            ScheduleCandidateError::SourceUnavailable {
                source_key: candidate.source().clone(),
            }
        })?;
        if candidate.action != entry.action
            || candidate.encoded_key.as_str()
                != candidate.advance.schedule().canonical_hash().to_hex()
            || candidate.advance.owner() != entry.key.owner()
        {
            return Err(ScheduleCandidateError::IdentityMismatch {
                source_key: entry.key.clone(),
            });
        }
        self.owner_readiness.require_ready(entry.key.owner())?;
        Ok(entry)
    }

    pub(crate) fn scan(
        &self,
        engine: &crate::engine::Engine,
        cause: ScheduleDriveCause,
        cursor: Option<&ScheduleSourceCursor>,
        limit: usize,
    ) -> Result<ScheduleSourcePage, ScheduleSourceScanError> {
        if limit == 0 || limit > MAX_SCHEDULE_SOURCE_SCAN_RECORDS {
            return Err(ScheduleSourceScanError::InvalidLimit {
                actual: limit,
                max: MAX_SCHEDULE_SOURCE_SCAN_RECORDS,
            });
        }
        if let Some(cursor) = cursor
            && !self.entries.contains_key(cursor.source())
        {
            return Err(ScheduleSourceScanError::StaleCursor {
                source_key: cursor.source().clone(),
            });
        }

        let mut candidates = Vec::new();
        let mut records_scanned = 0usize;
        let mut last_scanned = None;
        for (key, entry) in &self.entries {
            if cursor.is_some_and(|cursor| key < cursor.source()) {
                continue;
            }
            self.owner_readiness.require_ready(key.owner())?;
            let after = cursor
                .filter(|cursor| cursor.source() == key)
                .map(|cursor| cursor.after_encoded_key().as_str());
            let remaining = limit - records_scanned;
            if remaining == 0 {
                break;
            }
            let page = engine
                .action_state()
                .scan_family(&entry.schedules, after, remaining)?;
            for scanned in page.entries() {
                records_scanned += 1;
                last_scanned = Some(ScheduleSourceCursor {
                    source: key.clone(),
                    after_encoded_key: SmolStr::new(scanned.encoded_key()),
                });
                let schedule = scanned.value();
                if schedule.owner() != key.owner()
                    || scanned.encoded_key() != schedule.id().canonical_hash().to_hex()
                {
                    return Err(ScheduleSourceScanError::RecordIdentity {
                        source_key: key.clone(),
                        encoded_key: SmolStr::new(scanned.encoded_key()),
                    });
                }
                let now = match engine.clock_now(schedule.clock()) {
                    Ok(now) => now,
                    // E7c4 R2/D4 (independent verify fold): a schedule pinned to a
                    // now-stale generation cannot be evaluated on the current one.
                    // SKIP it (leaving it paused until it is restamped) rather than
                    // failing this whole shared discovery — one stale schedule used
                    // to return `Err`, which the durable-schedule driver classified
                    // non-retryable and turned into `apply_integrity_freeze`, an
                    // unrecoverable world freeze the moment any timeline generation
                    // advanced. Every other source's candidates still surface, and
                    // the paused entry's recovery is its owner's restamp
                    // (`@wo/restamp` for production).
                    //
                    // E7c8 independent verify fold T2/D4(a): the same reasoning
                    // covers a schedule whose **module provider** cannot answer.
                    // `ClockProviderRegistry::read_domain` wraps every failure in
                    // `DomainProvider`, which the flat arm above cannot match, so
                    // one stood-down or re-promoted domain took the whole shared
                    // discovery — production, `control_handoff`, `npc` and the six
                    // `sensor_*` sources — down with it. See
                    // [`schedule_scan_clock_is_skippable`].
                    Err(source) if schedule_scan_clock_is_skippable(&source) => continue,
                    Err(source) => {
                        return Err(ScheduleSourceScanError::Clock {
                            source_key: key.clone(),
                            schedule: schedule.id(),
                            source: Box::new(source),
                        });
                    }
                };
                if let ScheduleEvaluation::Advance(advance) = schedule
                    .plan_due(schedule.revision(), &now, cause)
                    .map_err(|source| ScheduleSourceScanError::Schedule {
                        source_key: key.clone(),
                        schedule: schedule.id(),
                        source,
                    })?
                {
                    candidates.push(ScheduledActionCandidate {
                        registry_fingerprint: self.fingerprint,
                        source: key.clone(),
                        encoded_key: SmolStr::new(scanned.encoded_key()),
                        action: entry.action.clone(),
                        advance: *advance,
                    });
                }
            }
            self.owner_readiness.require_ready(key.owner())?;
            if records_scanned == limit {
                return Ok(ScheduleSourcePage {
                    candidates,
                    records_scanned,
                    next_cursor: last_scanned,
                });
            }
        }
        Ok(ScheduleSourcePage {
            candidates,
            records_scanned,
            next_cursor: None,
        })
    }
}

/// Whether one schedule's clock read is a **per-entry pause** rather than a
/// failure of the whole shared discovery.
///
/// A scan walks every owner's schedules in one pass, so a clock that cannot
/// answer for *one* record must never abort the others. The skippable set is
/// exactly "the clock is not answering right now, and a later fire may answer":
///
/// - [`ClockReadError::StaleGeneration`] — the flat form the `WorldIc` and
///   `Registered` paths raise (E7c4 R2/D4);
/// - [`ClockReadError::DomainProvider`] with
///   [`ClockProviderFailure::StaleGeneration`] or
///   [`ClockProviderFailure::Unavailable`] — a domain that has stood down, been
///   re-promoted onto a new generation, or gone away entirely (E7c8 T2/D4);
/// - [`ClockReadError::SceneProvider`] with
///   [`ClockProviderFailure::Unavailable`] — a scene clock whose scene is not
///   currently readable.
///
/// Everything else — a missing registry, a missing provider, an unsupported
/// clock, a zero generation, an `Overflow` or an `Integrity` answer — is a real
/// composition or invariant failure and still aborts the scan, because no later
/// fire recovers from it on its own.
#[must_use]
pub fn schedule_scan_clock_is_skippable(error: &ClockReadError) -> bool {
    match error {
        ClockReadError::StaleGeneration { .. } => true,
        ClockReadError::DomainProvider { source, .. } => matches!(
            source,
            ClockProviderFailure::StaleGeneration { .. } | ClockProviderFailure::Unavailable
        ),
        ClockReadError::SceneProvider { source, .. } => {
            matches!(source, ClockProviderFailure::Unavailable)
        }
        _ => false,
    }
}

/// Composition failure for schedule sources.
#[derive(Debug, Error)]
pub enum ScheduleSourceRegistryError {
    #[error("schedule-source registry exceeds its hard limit of {max}")]
    SourceLimit { max: usize },
    #[error("schedule-source owner {owner} exceeds its source limit of {max}")]
    OwnerSourceLimit { owner: OwnerId, max: usize },
    #[error("schedule source owner must own both its action and typed family")]
    OwnerMismatch,
    #[error("schedule source target definition is not an action")]
    ActionKind,
    #[error("schedule source is registered more than once: {key:?}")]
    Duplicate { key: ScheduleSourceKey },
    #[error("schedule source names an uncomposed owner: {key:?}")]
    UnknownOwner { key: ScheduleSourceKey },
    #[error("schedule source {key:?} has no exact action provider: {source}")]
    Action {
        key: ScheduleSourceKey,
        source: Box<ActionProviderLookupError>,
    },
    #[error("schedule source {key:?} has an invalid typed family: {source}")]
    Family {
        key: ScheduleSourceKey,
        source: Box<ActionStateError>,
    },
    #[error("schedule-source registry fingerprint is not canonical: {0}")]
    Fingerprint(#[from] CanonicalEncodingError),
}

/// Invalid construction of the canonical internal scheduler ingress.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ScheduleActionIngressError {
    #[error("scheduled advance and action have different owners")]
    OwnerMismatch,
    #[error("scheduled action hashing failed: {0}")]
    Hash(#[from] CanonicalHashError),
    #[error("scheduled action metadata is invalid: {0}")]
    Metadata(#[from] ActionMetadataError),
    #[error("scheduled action identity is invalid: {0}")]
    CanonicalId(#[from] CanonicalIdError),
    #[error("scheduled action idempotency text is invalid: {0}")]
    IdempotencyText(#[from] IdempotencyKeyTextError),
}

/// A candidate does not belong to this exact frozen registry or source.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ScheduleCandidateError {
    #[error("scheduled candidate belongs to another mechanics composition")]
    RegistryMismatch,
    #[error("scheduled candidate source is unavailable: {source_key:?}")]
    SourceUnavailable { source_key: ScheduleSourceKey },
    #[error("scheduled candidate identity does not match source {source_key:?}")]
    IdentityMismatch { source_key: ScheduleSourceKey },
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

/// Failure while constructing provider-specific work after vacant preflight.
#[derive(Debug, Error)]
pub enum ScheduleActionPlanError {
    #[error("schedule-source registry is unavailable before module composition")]
    RegistryUnavailable,
    #[error(transparent)]
    Candidate(#[from] ScheduleCandidateError),
    #[error(transparent)]
    Ingress(#[from] ScheduleActionIngressError),
    #[error("scheduled ingress does not match the candidate's canonical identity")]
    IngressMismatch,
    #[error("schedule adapter {source_key:?} rejected the due cursor: {source}")]
    Adapter {
        source_key: ScheduleSourceKey,
        source: Box<ActionError>,
    },
    #[error("scheduled plan for {action} omitted its exact definition guard")]
    MissingDefinitionGuard { action: Box<DefinitionRef> },
    #[error("scheduled action invocation is invalid: {0}")]
    Invocation(#[from] ActionMetadataError),
    #[error("scheduled internal route hashing failed: {0}")]
    Hash(#[from] CanonicalHashError),
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

/// Failure while atomically admitting a discovered cursor post-image.
#[derive(Debug, Error)]
pub enum ScheduleAdvanceAdmissionError {
    #[error(transparent)]
    Candidate(#[from] ScheduleCandidateError),
    #[error("schedule source {source_key:?} no longer has schedule {schedule:?}")]
    MissingSchedule {
        source_key: ScheduleSourceKey,
        schedule: ScheduleId,
    },
    #[error("schedule source {source_key:?}, schedule {schedule:?}, changed after discovery")]
    StalePlan {
        source_key: ScheduleSourceKey,
        schedule: ScheduleId,
    },
    #[error(transparent)]
    Schedule(#[from] DurableScheduleError),
    #[error(transparent)]
    State(#[from] ActionStateError),
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

/// Fail-closed bounded discovery error.
#[derive(Debug, Error)]
pub enum ScheduleSourceScanError {
    #[error("schedule-source registry is unavailable before module composition")]
    RegistryUnavailable,
    #[error("schedule-source scan limit {actual} must be in 1..={max}")]
    InvalidLimit { actual: usize, max: usize },
    #[error("schedule-source scan cursor names an unavailable source: {source_key:?}")]
    StaleCursor { source_key: ScheduleSourceKey },
    #[error("schedule record key/owner does not match source {source_key:?}: {encoded_key}")]
    RecordIdentity {
        source_key: ScheduleSourceKey,
        encoded_key: SmolStr,
    },
    #[error(
        "schedule source {source_key:?}, schedule {schedule:?}, cannot read its clock: {source}"
    )]
    Clock {
        source_key: ScheduleSourceKey,
        schedule: ScheduleId,
        source: Box<ClockReadError>,
    },
    #[error("schedule source {source_key:?}, schedule {schedule:?}, is invalid: {source}")]
    Schedule {
        source_key: ScheduleSourceKey,
        schedule: ScheduleId,
        source: DurableScheduleError,
    },
    #[error(transparent)]
    State(#[from] ActionStateError),
    #[error(transparent)]
    OwnerNotReady(#[from] MechanicalOwnerReadinessError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ClockRef;
    use crate::mechanics::{CanonicalId, OwnerId};

    fn owner() -> OwnerId {
        OwnerId::new("sim").expect("a static owner")
    }

    /// **E7 cut 8 independent verify fold, T2/D4(a).** The scan's per-entry
    /// pause covers every clock failure a later fire can recover from, and
    /// **only** those: a stood-down domain (`DomainProvider { Unavailable }`),
    /// a re-promoted one (`DomainProvider { StaleGeneration }`), an unreadable
    /// scene (`SceneProvider { Unavailable }`) and the flat `StaleGeneration`
    /// cut 4's fold added — while a missing registry, a missing provider, an
    /// unsupported clock, a zero generation, an `Overflow` and an `Integrity`
    /// answer all still abort, because nothing recovers from them by waiting.
    #[test]
    fn vf_t2c_the_scan_pause_covers_domain_and_scene_provider_stalls_and_nothing_else() {
        let id = CanonicalId::new("d0").expect("a static id");
        // Skippable: the four a later fire recovers from.
        for skippable in [
            ClockReadError::StaleGeneration {
                requested: 1,
                current: 2,
            },
            ClockReadError::DomainProvider {
                owner: owner(),
                id: id.clone(),
                source: ClockProviderFailure::Unavailable,
            },
            ClockReadError::DomainProvider {
                owner: owner(),
                id: id.clone(),
                source: ClockProviderFailure::StaleGeneration {
                    requested: 1,
                    current: 3,
                },
            },
            ClockReadError::SceneProvider {
                owner: owner(),
                source: ClockProviderFailure::Unavailable,
            },
        ] {
            assert!(
                schedule_scan_clock_is_skippable(&skippable),
                "{skippable} must pause its own entry, not the whole scan"
            );
        }
        // Not skippable: every arm that names a real composition or invariant
        // failure. Naming each one here is the point — a new arm added to
        // `ClockReadError` fails this test until someone classifies it.
        for fatal in [
            ClockReadError::RegistryUnavailable,
            ClockReadError::UnsupportedClock {
                clock: ClockRef::Utc,
            },
            ClockReadError::ZeroGeneration {
                clock: ClockRef::Utc,
            },
            ClockReadError::SceneProviderMissing,
            ClockReadError::DomainProviderMissing { owner: owner() },
            ClockReadError::DomainProvider {
                owner: owner(),
                id: id.clone(),
                source: ClockProviderFailure::Overflow,
            },
            ClockReadError::DomainProvider {
                owner: owner(),
                id,
                source: ClockProviderFailure::Integrity,
            },
            ClockReadError::SceneProvider {
                owner: owner(),
                source: ClockProviderFailure::Integrity,
            },
        ] {
            assert!(
                !schedule_scan_clock_is_skippable(&fatal),
                "{fatal} must still abort the scan"
            );
        }
    }
}

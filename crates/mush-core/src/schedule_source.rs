//! Composition-frozen discovery of durable schedule families.
//!
//! A source is declarative: one owner-local `ScheduleId -> DurableSchedule`
//! typed family plus the exact authoritative action that advances it. The
//! kernel scans records in canonical bounded pages and evaluates clocks; no
//! provider callback can hide an unbounded index or mutate state while
//! discovery runs.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use smol_str::SmolStr;
use thiserror::Error;

use crate::action::{ActionProviderLookupError, ActionProviderRegistry};
use crate::action_state::{ActionStateError, StateFamilyHandle, StateFamilyRegistry};
use crate::clock_provider::ClockReadError;
use crate::durable_schedule::{
    DurableSchedule, DurableScheduleError, ScheduleAdvance, ScheduleDriveCause, ScheduleEvaluation,
    ScheduleId,
};
use crate::mechanics::{
    CanonicalEncodingError, CanonicalId, DefinitionDigest, DefinitionRef, MechanicalOwnerReadiness,
    MechanicalOwnerReadinessError, OwnerId,
};

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

#[derive(Clone, Debug)]
struct ScheduleSourceEntry {
    key: ScheduleSourceKey,
    action: DefinitionRef,
    schedules: StateFamilyHandle<ScheduleId, DurableSchedule>,
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
                let now = engine.clock_now(schedule.clock()).map_err(|source| {
                    ScheduleSourceScanError::Clock {
                        source_key: key.clone(),
                        schedule: schedule.id(),
                        source: Box::new(source),
                    }
                })?;
                if let ScheduleEvaluation::Advance(advance) = schedule
                    .plan_due(schedule.revision(), &now, cause)
                    .map_err(|source| ScheduleSourceScanError::Schedule {
                        source_key: key.clone(),
                        schedule: schedule.id(),
                        source,
                    })?
                {
                    candidates.push(ScheduledActionCandidate {
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

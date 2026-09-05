//! Bounded async provider execution for durable K7 decisions.
//!
//! Capacity is reserved before a `Ready -> Leased` transition is committed.
//! Only after that exact commit is acknowledged may the reservation spawn its
//! provider future. Completion carries the full lease fence back to the world
//! task; it does not itself acknowledge or mutate authoritative state.

use std::collections::BTreeMap;
use std::sync::Arc;

use futures::future::BoxFuture;
use mush_core::{
    DecisionControllerId, DecisionResponse, DecisionWorkerLease, MAX_DECISION_PROVIDER_WORKERS,
    OwnerId,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};

pub const DEFAULT_DECISION_PROVIDER_WORKERS: usize = 8;
pub const MAX_DECISION_COMPLETIONS: usize = 128;
const MAX_DECISION_PROVIDER_ERROR_BYTES: usize = 512;

/// Explicit provider failure class. The world-side lifecycle decides whether
/// to retry, fall back, or terminate; text never determines policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionProviderFailureKind {
    Unavailable,
    Malformed,
    Permanent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionProviderFailure {
    kind: DecisionProviderFailureKind,
    detail: String,
}

impl DecisionProviderFailure {
    #[must_use]
    pub fn new(kind: DecisionProviderFailureKind, detail: impl AsRef<str>) -> Self {
        Self {
            kind,
            detail: bounded_detail(detail.as_ref()),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> DecisionProviderFailureKind {
        self.kind
    }

    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

/// One owner-routed async decision provider. It receives only the persisted
/// public-safe prompt and exact lease fence.
pub trait DecisionProviderAdapter: Send + Sync {
    fn owner(&self) -> &OwnerId;
    fn decide(
        &self,
        lease: DecisionWorkerLease,
    ) -> BoxFuture<'static, Result<DecisionResponse, DecisionProviderFailure>>;
}

#[derive(Default)]
pub struct DecisionProviderRegistry {
    adapters: BTreeMap<OwnerId, Arc<dyn DecisionProviderAdapter>>,
}

impl DecisionProviderRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        adapter: Arc<dyn DecisionProviderAdapter>,
    ) -> Result<(), DecisionProviderRegistryError> {
        let owner = adapter.owner().clone();
        match self.adapters.entry(owner.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(adapter);
            }
            std::collections::btree_map::Entry::Occupied(_) => {
                return Err(DecisionProviderRegistryError::Duplicate { owner });
            }
        }
        Ok(())
    }

    fn get(&self, owner: &OwnerId) -> Option<Arc<dyn DecisionProviderAdapter>> {
        self.adapters.get(owner).cloned()
    }

    #[must_use]
    pub fn contains_owner(&self, owner: &OwnerId) -> bool {
        self.adapters.contains_key(owner)
    }
}

#[derive(Clone, Debug, Eq, thiserror::Error, PartialEq)]
pub enum DecisionProviderRegistryError {
    #[error("a decision provider is already registered for owner {owner}")]
    Duplicate { owner: OwnerId },
}

/// Provider result plus the exact durable lease that authorized the worker.
#[derive(Clone, Debug)]
pub struct DecisionWorkerCompletion {
    controller: DecisionControllerId,
    decision: mush_core::DecisionId,
    revision: u64,
    attempt: u8,
    lease_until: mush_core::ClockStamp,
    result: Result<DecisionResponse, DecisionProviderFailure>,
}

impl DecisionWorkerCompletion {
    #[must_use]
    pub fn fence(&self) -> mush_core::DecisionCompletionFence {
        mush_core::DecisionCompletionFence::new(
            self.controller.clone(),
            self.decision,
            self.revision,
            self.attempt,
            self.lease_until.clone(),
        )
        .expect("dispatcher completions are built only from validated worker leases")
    }

    #[must_use]
    pub const fn controller(&self) -> &DecisionControllerId {
        &self.controller
    }

    #[must_use]
    pub const fn decision(&self) -> mush_core::DecisionId {
        self.decision
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn attempt(&self) -> u8 {
        self.attempt
    }

    #[must_use]
    pub const fn lease_until(&self) -> &mush_core::ClockStamp {
        &self.lease_until
    }

    pub const fn result(&self) -> &Result<DecisionResponse, DecisionProviderFailure> {
        &self.result
    }

    pub fn into_result(self) -> Result<DecisionResponse, DecisionProviderFailure> {
        self.result
    }
}

/// Process-wide provider worker limiter. A reservation is obtained before the
/// caller commits a durable lease, closing the commit-with-no-worker gap.
pub struct DecisionDispatcher {
    registry: Arc<DecisionProviderRegistry>,
    workers: Arc<Semaphore>,
    completions: mpsc::Sender<DecisionWorkerCompletion>,
}

impl DecisionDispatcher {
    pub fn new(
        registry: Arc<DecisionProviderRegistry>,
        worker_limit: usize,
        completions: mpsc::Sender<DecisionWorkerCompletion>,
    ) -> Result<Self, DecisionDispatcherConfigError> {
        if worker_limit == 0 || worker_limit > MAX_DECISION_PROVIDER_WORKERS {
            return Err(DecisionDispatcherConfigError::WorkerLimit {
                actual: worker_limit,
                max: MAX_DECISION_PROVIDER_WORKERS,
            });
        }
        Ok(Self {
            registry,
            workers: Arc::new(Semaphore::new(worker_limit)),
            completions,
        })
    }

    /// Reserve both an owner-routed provider and one process worker before
    /// committing the lease transition.
    pub fn try_reserve(
        &self,
        controller: &DecisionControllerId,
    ) -> Result<DecisionDispatchReservation, DecisionDispatchReserveError> {
        let adapter = self.registry.get(&controller.owner).ok_or_else(|| {
            DecisionDispatchReserveError::Unavailable {
                owner: controller.owner.clone(),
            }
        })?;
        let permit = Arc::clone(&self.workers)
            .try_acquire_owned()
            .map_err(|_| DecisionDispatchReserveError::Busy)?;
        Ok(DecisionDispatchReservation {
            controller: controller.clone(),
            adapter,
            permit,
            completions: self.completions.clone(),
        })
    }
}

#[derive(Clone, Debug, Eq, thiserror::Error, PartialEq)]
pub enum DecisionDispatcherConfigError {
    #[error("decision worker limit {actual} is outside 1..={max}")]
    WorkerLimit { actual: usize, max: usize },
}

#[derive(Clone, Debug, Eq, thiserror::Error, PartialEq)]
pub enum DecisionDispatchReserveError {
    #[error("no async decision provider is registered for owner {owner}")]
    Unavailable { owner: OwnerId },
    #[error("all bounded async decision workers are reserved")]
    Busy,
}

/// A worker slot held across the lease commit. Dropping it after a failed
/// commit safely returns capacity without running provider code.
#[must_use = "reserve before lease commit, then spawn only after acknowledgement"]
pub struct DecisionDispatchReservation {
    controller: DecisionControllerId,
    adapter: Arc<dyn DecisionProviderAdapter>,
    permit: OwnedSemaphorePermit,
    completions: mpsc::Sender<DecisionWorkerCompletion>,
}

impl DecisionDispatchReservation {
    /// Spawn provider work after the exact lease post-image commits.
    pub fn spawn(self, lease: DecisionWorkerLease) -> Result<(), DecisionDispatchSpawnError> {
        if lease.controller() != &self.controller {
            return Err(DecisionDispatchSpawnError::ControllerMismatch);
        }
        let controller = lease.controller().clone();
        let decision = lease.decision();
        let revision = lease.revision();
        let attempt = lease.attempt();
        let lease_until = lease.lease_until().clone();
        let expected_token = lease.prompt().token;
        let future = self.adapter.decide(lease);
        let completions = self.completions;
        let permit = self.permit;
        tokio::spawn(async move {
            let result = match future.await {
                Ok(response) if response.validate().is_ok() && response.token == expected_token => {
                    Ok(response)
                }
                Ok(_) => Err(DecisionProviderFailure::new(
                    DecisionProviderFailureKind::Malformed,
                    "provider response failed its exact token or payload contract",
                )),
                Err(error) => Err(error),
            };
            let _permit = permit;
            let _ = completions
                .send(DecisionWorkerCompletion {
                    controller,
                    decision,
                    revision,
                    attempt,
                    lease_until,
                    result,
                })
                .await;
        });
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, thiserror::Error, PartialEq)]
pub enum DecisionDispatchSpawnError {
    #[error("worker lease belongs to a different controller than its reservation")]
    ControllerMismatch,
}

fn bounded_detail(input: &str) -> String {
    if input.is_empty() {
        return "decision provider failed without detail".to_owned();
    }
    if input.len() <= MAX_DECISION_PROVIDER_ERROR_BYTES {
        return input.to_owned();
    }
    let mut end = MAX_DECISION_PROVIDER_ERROR_BYTES;
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    input[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::CanonicalId;

    struct NeverCalled {
        owner: OwnerId,
    }

    impl DecisionProviderAdapter for NeverCalled {
        fn owner(&self) -> &OwnerId {
            &self.owner
        }

        fn decide(
            &self,
            _lease: DecisionWorkerLease,
        ) -> BoxFuture<'static, Result<DecisionResponse, DecisionProviderFailure>> {
            Box::pin(async { panic!("reservation test must not spawn provider work") })
        }
    }

    fn controller(owner: &OwnerId) -> DecisionControllerId {
        DecisionControllerId {
            owner: owner.clone(),
            key: CanonicalId::new("npc_7").unwrap(),
        }
    }

    #[test]
    fn capacity_is_reserved_before_commit_and_duplicate_registration_is_non_mutating() {
        assert!(
            DecisionDispatcher::new(
                Arc::new(DecisionProviderRegistry::new()),
                MAX_DECISION_PROVIDER_WORKERS + 1,
                mpsc::channel(1).0,
            )
            .is_err()
        );
        let owner = OwnerId::new("npc").unwrap();
        let mut registry = DecisionProviderRegistry::new();
        registry
            .register(Arc::new(NeverCalled {
                owner: owner.clone(),
            }))
            .unwrap();
        assert!(matches!(
            registry.register(Arc::new(NeverCalled {
                owner: owner.clone(),
            })),
            Err(DecisionProviderRegistryError::Duplicate { .. })
        ));
        let dispatcher = DecisionDispatcher::new(
            Arc::new(registry),
            1,
            mpsc::channel(MAX_DECISION_COMPLETIONS).0,
        )
        .unwrap();
        let first = dispatcher.try_reserve(&controller(&owner)).unwrap();
        assert!(matches!(
            dispatcher.try_reserve(&controller(&owner)),
            Err(DecisionDispatchReserveError::Busy)
        ));
        drop(first);
        assert!(dispatcher.try_reserve(&controller(&owner)).is_ok());

        let unknown = OwnerId::new("unknown").unwrap();
        assert!(matches!(
            dispatcher.try_reserve(&controller(&unknown)),
            Err(DecisionDispatchReserveError::Unavailable { .. })
        ));
    }

    #[test]
    fn provider_error_detail_is_utf8_safe_and_bounded() {
        let detail = "🙂".repeat(MAX_DECISION_PROVIDER_ERROR_BYTES);
        let error = DecisionProviderFailure::new(DecisionProviderFailureKind::Unavailable, detail);
        assert!(error.detail().len() <= MAX_DECISION_PROVIDER_ERROR_BYTES);
        assert!(std::str::from_utf8(error.detail().as_bytes()).is_ok());
    }
}

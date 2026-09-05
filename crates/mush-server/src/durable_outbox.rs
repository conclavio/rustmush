//! Recovery-driven dispatch for K3 durable post-commit work.
//!
//! The store persists an attempt before the adapter is called. A crash after
//! the external send but before the terminal acknowledgement therefore causes
//! an intentional at-least-once retry with the same stable item id.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use mush_core::{
    CanonicalId, DecisionWake, MAX_DECISION_MAILBOX_IDS, MAX_DECISION_WAKE_BYTES, OwnerId,
    canonical_json_bytes_with_limit,
};
use mush_store::{
    AuthoritativeOutboxAttemptResult, AuthoritativeOutboxRecord, AuthoritativeOutboxState,
    AuthoritativeStore, MAX_AUTHORITATIVE_OUTBOX_ERROR_BYTES, ObjectStore, StoreError,
};
use tokio::sync::{mpsc, watch};

const DURABLE_OUTBOX_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// One adapter's delivery result. Retryability is explicit; arbitrary adapter
/// errors are never guessed from text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DurableDeliveryError {
    Retryable(String),
    Permanent(String),
}

/// External delivery surface selected by the provider-owned `(owner, adapter)`
/// identity embedded in an authoritative bundle.
pub trait DurableOutboxAdapter: Send + Sync {
    fn owner(&self) -> &OwnerId;
    fn adapter(&self) -> &CanonicalId;
    fn deliver<'a>(
        &'a self,
        item: &'a AuthoritativeOutboxRecord,
    ) -> BoxFuture<'a, Result<(), DurableDeliveryError>>;
}

/// Volatile notification offered by the durable outbox to the world-owned
/// decision dispatcher. The pending/prompt typed records remain authoritative
/// custody after this hint is acknowledged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionWakeDispatch {
    owner: OwnerId,
    wake: DecisionWake,
}

impl DecisionWakeDispatch {
    #[must_use]
    pub const fn owner(&self) -> &OwnerId {
        &self.owner
    }

    #[must_use]
    pub const fn wake(&self) -> DecisionWake {
        self.wake
    }
}

/// Create the hard-bounded volatile decision mailbox.
pub fn decision_wake_mailbox(
    capacity: usize,
) -> Result<
    (
        mpsc::Sender<DecisionWakeDispatch>,
        mpsc::Receiver<DecisionWakeDispatch>,
    ),
    &'static str,
> {
    if capacity == 0 || capacity > MAX_DECISION_MAILBOX_IDS {
        return Err("decision wake mailbox capacity is outside the kernel hard bounds");
    }
    Ok(mpsc::channel(capacity))
}

/// Outbox adapter that performs no provider I/O. It validates the bounded wake
/// and offers it to the volatile dispatcher without waiting for capacity.
pub struct DecisionWakeOutboxAdapter {
    owner: OwnerId,
    adapter: CanonicalId,
    sender: mpsc::Sender<DecisionWakeDispatch>,
}

impl DecisionWakeOutboxAdapter {
    #[must_use]
    pub const fn new(
        owner: OwnerId,
        adapter: CanonicalId,
        sender: mpsc::Sender<DecisionWakeDispatch>,
    ) -> Self {
        Self {
            owner,
            adapter,
            sender,
        }
    }

    fn validate_and_offer(&self, payload: &serde_json::Value) -> Result<(), DurableDeliveryError> {
        canonical_json_bytes_with_limit(payload, MAX_DECISION_WAKE_BYTES).map_err(|_| {
            DurableDeliveryError::Permanent("decision wake exceeds its hard byte limit".to_owned())
        })?;
        let wake: DecisionWake = serde_json::from_value(payload.clone()).map_err(|_| {
            DurableDeliveryError::Permanent("decision wake has invalid wire data".to_owned())
        })?;
        wake.validate().map_err(|_| {
            DurableDeliveryError::Permanent("decision wake violates its contract".to_owned())
        })?;
        self.sender
            .try_send(DecisionWakeDispatch {
                owner: self.owner.clone(),
                wake,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => DurableDeliveryError::Retryable(
                    "decision dispatcher mailbox is full".to_owned(),
                ),
                mpsc::error::TrySendError::Closed(_) => {
                    DurableDeliveryError::Retryable("decision dispatcher is unavailable".to_owned())
                }
            })
    }
}

impl DurableOutboxAdapter for DecisionWakeOutboxAdapter {
    fn owner(&self) -> &OwnerId {
        &self.owner
    }

    fn adapter(&self) -> &CanonicalId {
        &self.adapter
    }

    fn deliver<'a>(
        &'a self,
        item: &'a AuthoritativeOutboxRecord,
    ) -> BoxFuture<'a, Result<(), DurableDeliveryError>> {
        Box::pin(async move {
            if item.intent().owner() != &self.owner || item.intent().adapter() != &self.adapter {
                return Err(DurableDeliveryError::Permanent(
                    "decision wake was routed to the wrong adapter".to_owned(),
                ));
            }
            self.validate_and_offer(item.intent().payload())
        })
    }
}

/// Frozen adapter lookup. Duplicate identities are refused during boot.
#[derive(Default)]
pub struct DurableOutboxRegistry {
    adapters: BTreeMap<(OwnerId, CanonicalId), Arc<dyn DurableOutboxAdapter>>,
}

impl DurableOutboxRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        adapter: Arc<dyn DurableOutboxAdapter>,
    ) -> Result<(), DurableOutboxRegistryError> {
        let key = (adapter.owner().clone(), adapter.adapter().clone());
        match self.adapters.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(adapter);
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                return Err(DurableOutboxRegistryError::Duplicate {
                    owner: entry.key().0.clone(),
                    adapter: entry.key().1.clone(),
                });
            }
        }
        Ok(())
    }

    fn get(&self, item: &AuthoritativeOutboxRecord) -> Option<&dyn DurableOutboxAdapter> {
        self.adapters
            .get(&(
                item.intent().owner().clone(),
                item.intent().adapter().clone(),
            ))
            .map(AsRef::as_ref)
    }
}

#[derive(Clone, Debug, Eq, thiserror::Error, PartialEq)]
pub enum DurableOutboxRegistryError {
    #[error("duplicate durable-outbox adapter {owner}:{adapter}")]
    Duplicate {
        owner: OwnerId,
        adapter: CanonicalId,
    },
}

/// Bounded attempt policy. A timeout is a retryable delivery result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DurableOutboxPolicy {
    pub max_attempts: u32,
    /// Number of persisted discovery passes an item may remain without a
    /// registered adapter before it reaches terminal dead-letter state.
    pub max_unroutable_passes: u32,
    pub attempt_timeout: Duration,
    pub max_items_per_pass: usize,
}

impl DurableOutboxPolicy {
    pub fn new(max_attempts: u32, attempt_timeout: Duration) -> Result<Self, &'static str> {
        if max_attempts == 0 {
            return Err("durable-outbox max_attempts must be non-zero");
        }
        if attempt_timeout.is_zero() {
            return Err("durable-outbox attempt_timeout must be non-zero");
        }
        Ok(Self {
            max_attempts,
            max_unroutable_passes: 3_600,
            attempt_timeout,
            max_items_per_pass: 16,
        })
    }

    #[must_use]
    pub fn with_max_items_per_pass(mut self, max_items_per_pass: usize) -> Self {
        self.max_items_per_pass = max_items_per_pass.max(1);
        self
    }

    #[must_use]
    pub fn with_max_unroutable_passes(mut self, max_unroutable_passes: u32) -> Self {
        self.max_unroutable_passes = max_unroutable_passes.max(1);
        self
    }
}

impl Default for DurableOutboxPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 8,
            max_unroutable_passes: 3_600,
            attempt_timeout: Duration::from_secs(10),
            max_items_per_pass: 16,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DurableOutboxDispatchReport {
    pub discovered: usize,
    pub attempted: usize,
    pub delivered: usize,
    pub retry_pending: usize,
    pub dead_lettered: usize,
    pub missing_adapters: usize,
    pub checkpointed_parents: usize,
}

/// Run one recovery/dispatch pass. Missing adapters retain the exact item and
/// advance an independent durable pass counter until the configured terminal
/// dead-letter bound is reached.
pub async fn dispatch_authoritative_outbox(
    store: &dyn AuthoritativeStore,
    registry: &DurableOutboxRegistry,
    policy: DurableOutboxPolicy,
) -> Result<DurableOutboxDispatchReport, StoreError> {
    let pending = store.pending_authoritative_outbox().await?;
    let mut report = DurableOutboxDispatchReport {
        discovered: pending.len(),
        ..DurableOutboxDispatchReport::default()
    };

    let mut processed = 0_usize;
    let mut unroutable_processed = 0_usize;
    for item in pending {
        let Some(adapter) = registry.get(&item) else {
            report.missing_adapters += 1;
            if unroutable_processed >= policy.max_items_per_pass.max(1) {
                continue;
            }
            unroutable_processed += 1;
            let payload_hash = item.intent().payload_hash();
            let detail = bounded_error(&format!(
                "durable outbox adapter is unavailable: {}:{}",
                item.intent().owner(),
                item.intent().adapter()
            ));
            let observed = if item.unroutable_passes() >= policy.max_unroutable_passes {
                item
            } else {
                store
                    .note_authoritative_outbox_unroutable(
                        item.item_id(),
                        payload_hash,
                        item.unroutable_passes(),
                        detail,
                    )
                    .await?
            };
            if observed.unroutable_passes() >= policy.max_unroutable_passes {
                let terminal = store
                    .complete_authoritative_outbox_attempt(
                        observed.item_id(),
                        payload_hash,
                        observed.attempts(),
                        AuthoritativeOutboxAttemptResult::DeadLetter {
                            error: bounded_error(
                                "durable outbox adapter remained unavailable through its discovery-pass budget",
                            ),
                        },
                    )
                    .await?;
                report.dead_lettered += 1;
                checkpoint_terminal_parent(store, &terminal, &mut report).await?;
            } else {
                report.retry_pending += 1;
            }
            continue;
        };
        // Scan past unavailable adapters before applying the per-pass delivery
        // budget. Otherwise one unknown adapter at the front of the stable
        // pending order could starve every installed adapter forever.
        if processed >= policy.max_items_per_pass.max(1) {
            continue;
        }
        processed += 1;
        let payload_hash = item.intent().payload_hash();
        if item.attempts() >= policy.max_attempts {
            let terminal = store
                .complete_authoritative_outbox_attempt(
                    item.item_id(),
                    payload_hash,
                    item.attempts(),
                    AuthoritativeOutboxAttemptResult::DeadLetter {
                        error: bounded_error("durable delivery retry budget exhausted"),
                    },
                )
                .await?;
            report.dead_lettered += 1;
            checkpoint_terminal_parent(store, &terminal, &mut report).await?;
            continue;
        }

        // An attempt with no recorded result is ambiguous after a crash.
        // Redeliver that same attempt number instead of consuming retry budget
        // merely because the rustmush process died.
        let recovered_in_flight = item.attempts() > 0
            && matches!(
                item.state(),
                AuthoritativeOutboxState::Pending { last_error: None }
            );
        let started = if recovered_in_flight {
            item
        } else {
            store
                .begin_authoritative_outbox_attempt(item.item_id(), payload_hash, item.attempts())
                .await?
        };
        report.attempted += 1;
        let delivery =
            tokio::time::timeout(policy.attempt_timeout, adapter.deliver(&started)).await;
        let result = match delivery {
            Ok(Ok(())) => AuthoritativeOutboxAttemptResult::Delivered,
            Ok(Err(DurableDeliveryError::Permanent(error))) => {
                AuthoritativeOutboxAttemptResult::DeadLetter {
                    error: bounded_error(&error),
                }
            }
            Ok(Err(DurableDeliveryError::Retryable(error))) => {
                if started.attempts() >= policy.max_attempts {
                    AuthoritativeOutboxAttemptResult::DeadLetter {
                        error: bounded_error(&error),
                    }
                } else {
                    AuthoritativeOutboxAttemptResult::Retry {
                        error: bounded_error(&error),
                    }
                }
            }
            Err(_) => {
                let error = format!(
                    "durable delivery timed out after {} ms",
                    policy.attempt_timeout.as_millis()
                );
                if started.attempts() >= policy.max_attempts {
                    AuthoritativeOutboxAttemptResult::DeadLetter { error }
                } else {
                    AuthoritativeOutboxAttemptResult::Retry { error }
                }
            }
        };
        let terminal = matches!(
            &result,
            AuthoritativeOutboxAttemptResult::Delivered
                | AuthoritativeOutboxAttemptResult::DeadLetter { .. }
        );
        let delivered = matches!(&result, AuthoritativeOutboxAttemptResult::Delivered);
        let completed = store
            .complete_authoritative_outbox_attempt(
                started.item_id(),
                payload_hash,
                started.attempts(),
                result,
            )
            .await?;
        if terminal {
            if delivered {
                report.delivered += 1;
            } else {
                report.dead_lettered += 1;
                tracing::error!(
                    item_id = %completed.item_id(),
                    parent_token = %completed.parent_token(),
                    owner = %completed.intent().owner(),
                    adapter = %completed.intent().adapter(),
                    attempts = completed.attempts(),
                    state = ?completed.state(),
                    "durable outbox item reached terminal dead-letter state"
                );
            }
            checkpoint_terminal_parent(store, &completed, &mut report).await?;
        } else {
            report.retry_pending += 1;
        }
    }
    Ok(report)
}

/// Poll finalized delivery work for the lifetime of the world task. Cancelling
/// a pass is safe: an attempt is persisted before its adapter call and is
/// therefore rediscovered as ambiguous in-flight work on the next pass/boot.
pub async fn run_authoritative_outbox_worker(
    store: Arc<dyn ObjectStore>,
    registry: Arc<DurableOutboxRegistry>,
    policy: DurableOutboxPolicy,
    mut stop: watch::Receiver<bool>,
) {
    let mut last_missing = 0_usize;
    loop {
        let Some(authoritative) = store.authoritative_store() else {
            tracing::error!("durable outbox worker has no authoritative store; stopping");
            return;
        };
        let dispatch = dispatch_authoritative_outbox(authoritative, &registry, policy);
        tokio::select! {
            biased;
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow_and_update() {
                    return;
                }
            }
            result = dispatch => match result {
                Ok(report) => {
                    if report.missing_adapters != 0 && report.missing_adapters != last_missing {
                        tracing::error!(
                            missing_adapters = report.missing_adapters,
                            discovered = report.discovered,
                            "durable outbox work is retained because its adapter is unavailable"
                        );
                    }
                    last_missing = report.missing_adapters;
                    if report.attempted != 0 || report.dead_lettered != 0 {
                        tracing::info!(
                            attempted = report.attempted,
                            delivered = report.delivered,
                            retry_pending = report.retry_pending,
                            dead_lettered = report.dead_lettered,
                            checkpointed_parents = report.checkpointed_parents,
                            "durable outbox dispatch pass complete"
                        );
                    }
                }
                Err(error) => {
                    tracing::error!(%error, "durable outbox dispatch failed; retaining work for retry");
                }
            }
        }

        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow_and_update() {
                    return;
                }
            }
            () = tokio::time::sleep(DURABLE_OUTBOX_POLL_INTERVAL) => {}
        }
    }
}

async fn checkpoint_terminal_parent(
    store: &dyn AuthoritativeStore,
    item: &AuthoritativeOutboxRecord,
    report: &mut DurableOutboxDispatchReport,
) -> Result<(), StoreError> {
    match store
        .checkpoint_authoritative(item.parent_token(), item.parent_bundle_hash())
        .await
    {
        Ok(checkpoint) => {
            if !checkpoint.resumed {
                report.checkpointed_parents += 1;
            }
            Ok(())
        }
        Err(StoreError::AuthoritativeOutboxPending) => Ok(()),
        Err(error) => Err(error),
    }
}

fn bounded_error(input: &str) -> String {
    if input.is_empty() {
        return "durable delivery failed without detail".to_owned();
    }
    if input.len() <= MAX_AUTHORITATIVE_OUTBOX_ERROR_BYTES {
        return input.to_owned();
    }
    let mut end = MAX_AUTHORITATIVE_OUTBOX_ERROR_BYTES;
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    input[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{CanonicalHash, DecisionId};
    use serde_json::json;

    fn wake(seed: u8) -> serde_json::Value {
        serde_json::to_value(DecisionWake {
            id: DecisionId(CanonicalHash::from_bytes([seed; 32])),
            revision: 1,
            prompt_digest: CanonicalHash::from_bytes([seed.wrapping_add(1); 32]),
        })
        .unwrap()
    }

    #[test]
    fn decision_wake_adapter_is_bounded_retryable_and_strict() {
        assert!(decision_wake_mailbox(0).is_err());
        assert!(decision_wake_mailbox(MAX_DECISION_MAILBOX_IDS + 1).is_err());
        let (sender, mut receiver) = decision_wake_mailbox(1).unwrap();
        let owner = OwnerId::new("npc").unwrap();
        let adapter = DecisionWakeOutboxAdapter::new(
            owner.clone(),
            CanonicalId::new("decision_wake").unwrap(),
            sender,
        );
        adapter.validate_and_offer(&wake(1)).unwrap();
        assert!(matches!(
            adapter.validate_and_offer(&wake(2)),
            Err(DurableDeliveryError::Retryable(_))
        ));
        let dispatch = receiver.try_recv().unwrap();
        assert_eq!(dispatch.owner(), &owner);
        assert_eq!(dispatch.wake().revision, 1);

        let mut unknown = wake(3);
        unknown
            .as_object_mut()
            .unwrap()
            .insert("guards".to_owned(), json!([]));
        assert!(matches!(
            adapter.validate_and_offer(&unknown),
            Err(DurableDeliveryError::Permanent(_))
        ));
        drop(receiver);
        assert!(matches!(
            adapter.validate_and_offer(&wake(4)),
            Err(DurableDeliveryError::Retryable(_))
        ));
    }
}

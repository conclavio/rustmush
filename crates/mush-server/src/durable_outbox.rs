//! Recovery-driven dispatch for K3 durable post-commit work.
//!
//! The store persists an attempt before the adapter is called. A crash after
//! the external send but before the terminal acknowledgement therefore causes
//! an intentional at-least-once retry with the same stable item id.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use mush_core::{CanonicalId, OwnerId};
use mush_store::{
    AuthoritativeOutboxAttemptResult, AuthoritativeOutboxRecord, AuthoritativeOutboxState,
    AuthoritativeStore, MAX_AUTHORITATIVE_OUTBOX_ERROR_BYTES, ObjectStore, StoreError,
};
use tokio::sync::watch;

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
            attempt_timeout,
            max_items_per_pass: 16,
        })
    }

    #[must_use]
    pub fn with_max_items_per_pass(mut self, max_items_per_pass: usize) -> Self {
        self.max_items_per_pass = max_items_per_pass.max(1);
        self
    }
}

impl Default for DurableOutboxPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 8,
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

/// Run one recovery/dispatch pass. Missing adapters remain pending so a
/// configuration error cannot silently discard externally visible work.
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
    for item in pending {
        let Some(adapter) = registry.get(&item) else {
            report.missing_adapters += 1;
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

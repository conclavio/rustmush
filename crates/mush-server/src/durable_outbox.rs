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
    AuthoritativeStore, MAX_AUTHORITATIVE_OUTBOX_ERROR_BYTES, StoreError,
};

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
        })
    }
}

impl Default for DurableOutboxPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 8,
            attempt_timeout: Duration::from_secs(10),
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

    for item in pending {
        let Some(adapter) = registry.get(&item) else {
            report.missing_adapters += 1;
            continue;
        };
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

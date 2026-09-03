//! [`InMemoryStore`] — the complete in-process [`ObjectStore`] used by tests
//! and `mushd --repl`.
//!
//! Fidelity to the Redis implementation is the whole point:
//!
//! - documents are held **serialized** (JSON strings), so every write/load
//!   crosses the same serde path a Redis round trip does — the serde
//!   stability tests in the shared suite are real here, not tautologies;
//! - a [`WriteBatch`] applies all-or-nothing under one lock (serialization,
//!   the only fallible step, happens before any mutation);
//! - upserts land before deletes, so an id in both lists ends up deleted;
//! - module ops apply after the object writes, in queue order; module
//!   streams are capped `VecDeque`s (exact trim — see [`MemStream`]) with
//!   monotonic `"N-0"` ids;
//! - ban expiry mirrors Redis TTL: expired entries are pruned at read time
//!   against the system clock.
//!
//! Locking: one `std::sync::Mutex` around all state. No `.await` ever
//! happens while it is held, so holding a blocking mutex inside async
//! methods is safe and keeps the implementation dependency-free.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use futures::future::BoxFuture;
use mush_core::{
    AuthoritativeBundle, CanonicalHash, CommitToken, Obj, ObjId, ReplayWrite, ScopedIdempotencyKey,
};

use crate::store::{AuthoritativeStore, ObjectStore};
use crate::types::{
    AccountRec, AuthoritativeIntentPage, AuthoritativeReceipt, AuthoritativeScanCursor, BanRec,
    BundleStatus, CharacterRef, IdempotencyStatus, IntentReceipt, MAX_AUTHORITATIVE_DISCOVERY_PAGE,
    MAX_RETAINED_AUTHORITATIVE_BYTES, MAX_RETAINED_AUTHORITATIVE_INTENTS, ModuleWrite,
    PendingAuthoritativeIntent, SshKeyRec, StoreError, TokenRecord, WorldSnapshot, WriteBatch,
    WriteReceipt, validate_module_key, validate_module_ns, validate_module_op, validate_token_hash,
};

/// See the [module docs](self).
#[derive(Debug, Default)]
pub struct InMemoryStore {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// id → serialized `Obj` JSON doc (what Redis holds under `obj:{id}`).
    objects: BTreeMap<u32, String>,
    /// `meta:next_id`.
    next_id: Option<u32>,
    /// `meta:schema_version`.
    schema_version: Option<u32>,
    /// lowercase name → serialized `AccountRec` JSON doc.
    accounts: HashMap<String, String>,
    /// ip → serialized `BanRec` JSON doc.
    bans: HashMap<String, String>,
    /// token hash → serialized `TokenRecord` JSON doc (`token:{hash}` in
    /// Redis). Expired entries are pruned lazily at read time against the
    /// system clock, mirroring Redis TTL — like `bans`.
    tokens: HashMap<String, String>,
    /// `(ns, key)` → serialized module JSON doc (`mod:{ns}:{key}` in
    /// Redis). The tuple key makes namespace isolation structural — no
    /// prefix matching to get wrong.
    module_kv: BTreeMap<(String, String), String>,
    /// `(ns, key)` → capped module stream (`mod:{ns}:{key}` in Redis).
    module_streams: BTreeMap<(String, String), MemStream>,
    /// Highest acknowledged batch serial (debug/ops visibility).
    last_serial: Option<u64>,
    /// Immutable full intents, retained until K3 checkpoint compaction exists.
    authoritative_intents: BTreeMap<CommitToken, String>,
    /// Immutable finalized witnesses. The initial implementation retains the
    /// complete bundle inside each receipt.
    authoritative_receipts: BTreeMap<CommitToken, String>,
    /// Early lookup binding installed atomically with the full intent.
    authoritative_idempotency: BTreeMap<ScopedIdempotencyKey, IdempotencyBinding>,
    /// Independently enumerable recovery index. Later fault tests deliberately
    /// damage this index and prove exhaustive intent discovery repairs it.
    authoritative_pending: BTreeSet<CommitToken>,
    closed: bool,
}

#[derive(Clone, Debug)]
struct IdempotencyBinding {
    token: CommitToken,
    ingress_hash: CanonicalHash,
    bundle_hash: CanonicalHash,
}

/// One in-memory module stream: a capped `VecDeque` mirroring
/// `XADD ... MAXLEN ~ n`. Trimming is **exact** here (a valid instance of
/// Redis's "at least `maxlen`, approximately" contract, which may retain
/// more). Ids are `"{seq}-0"` with `seq` starting at 1 and monotonically
/// increasing for the stream's lifetime — trims never reset it (like Redis,
/// a `Del` destroys the stream, so a recreated stream starts fresh).
#[derive(Debug, Default)]
struct MemStream {
    /// Sequence number the next appended entry receives.
    next_seq: u64,
    /// `(seq, serialized value JSON)`, oldest first.
    entries: VecDeque<(u64, String)>,
}

/// A module op validated and serialized ahead of the state lock, so a bad
/// op fails the whole batch before anything mutates.
enum PlannedModuleOp {
    JsonSet {
        ns: String,
        key: String,
        doc: String,
    },
    Del {
        ns: String,
        key: String,
    },
    StreamAppend {
        ns: String,
        key: String,
        doc: String,
        maxlen: u32,
    },
}

enum PlannedReplayWrite {
    ObjectSet {
        id: u32,
        doc: String,
    },
    ObjectDelete {
        id: u32,
    },
    ObjectAllocatorSet {
        next_id: u32,
    },
    ModuleJsonSet {
        namespace: String,
        key: String,
        doc: String,
    },
    ModuleDelete {
        namespace: String,
        key: String,
    },
}

impl InMemoryStore {
    /// A fresh, empty store ([`ObjectStore::load_world`] returns `None`).
    #[must_use]
    pub fn new() -> InMemoryStore {
        InMemoryStore::default()
    }

    /// Lock the state, failing if the store is closed.
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Inner>, StoreError> {
        let inner = self.inner.lock().expect("in-memory store lock poisoned");
        if inner.closed {
            return Err(StoreError::Closed);
        }
        Ok(inner)
    }
}

/// Unix seconds from the system clock (the store crate may read wall time;
/// `mush-core` never does).
fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn decode_intent(doc: &str) -> Result<AuthoritativeBundle, StoreError> {
    serde_json::from_str(doc).map_err(|error| StoreError::AuthoritativeCorrupt(error.to_string()))
}

fn decode_receipt(doc: &str) -> Result<AuthoritativeReceipt, StoreError> {
    serde_json::from_str(doc).map_err(|error| StoreError::AuthoritativeCorrupt(error.to_string()))
}

fn validate_retained_intent(
    inner: &Inner,
    stored_token: CommitToken,
    intent: &AuthoritativeBundle,
) -> Result<Option<AuthoritativeReceipt>, StoreError> {
    if intent.token() != stored_token {
        return Err(StoreError::AuthoritativeCorrupt(
            "authoritative intent is stored under the wrong token".to_owned(),
        ));
    }
    let binding = inner
        .authoritative_idempotency
        .get(intent.idempotency())
        .ok_or_else(|| {
            StoreError::AuthoritativeCorrupt(
                "authoritative intent is missing its idempotency binding".to_owned(),
            )
        })?;
    if binding.token != stored_token
        || binding.ingress_hash != intent.ingress_hash()
        || binding.bundle_hash != intent.bundle_hash()
    {
        return Err(StoreError::AuthoritativeCorrupt(
            "authoritative intent disagrees with its idempotency binding".to_owned(),
        ));
    }
    let Some(receipt_doc) = inner.authoritative_receipts.get(&stored_token) else {
        return Ok(None);
    };
    let receipt = decode_receipt(receipt_doc)?;
    if receipt.token() != stored_token || receipt.bundle() != intent {
        return Err(StoreError::AuthoritativeCorrupt(
            "finalized receipt disagrees with its retained intent".to_owned(),
        ));
    }
    Ok(Some(receipt))
}

fn plan_replay(bundle: &AuthoritativeBundle) -> Result<Vec<PlannedReplayWrite>, StoreError> {
    bundle
        .writes()
        .iter()
        .map(|write| {
            Ok(match write {
                ReplayWrite::ObjectSet { object } => PlannedReplayWrite::ObjectSet {
                    id: object.id.0,
                    doc: serde_json::to_string(object)?,
                },
                ReplayWrite::ObjectDelete { id } => PlannedReplayWrite::ObjectDelete { id: id.0 },
                ReplayWrite::ObjectAllocatorSet { next_id } => {
                    PlannedReplayWrite::ObjectAllocatorSet { next_id: *next_id }
                }
                ReplayWrite::ModuleJsonSet {
                    namespace,
                    key,
                    value,
                } => PlannedReplayWrite::ModuleJsonSet {
                    namespace: namespace.as_str().to_owned(),
                    key: key.as_str().to_owned(),
                    doc: serde_json::to_string(value)?,
                },
                ReplayWrite::ModuleDelete { namespace, key } => PlannedReplayWrite::ModuleDelete {
                    namespace: namespace.as_str().to_owned(),
                    key: key.as_str().to_owned(),
                },
            })
        })
        .collect()
}

fn apply_replay(inner: &mut Inner, planned: &[PlannedReplayWrite]) {
    for write in planned {
        match write {
            PlannedReplayWrite::ObjectSet { id, doc } => {
                inner.objects.insert(*id, doc.clone());
            }
            PlannedReplayWrite::ObjectDelete { id } => {
                inner.objects.remove(id);
            }
            PlannedReplayWrite::ObjectAllocatorSet { next_id } => {
                inner.next_id = Some(*next_id);
            }
            PlannedReplayWrite::ModuleJsonSet {
                namespace,
                key,
                doc,
            } => {
                inner
                    .module_kv
                    .insert((namespace.clone(), key.clone()), doc.clone());
            }
            PlannedReplayWrite::ModuleDelete { namespace, key } => {
                inner.module_kv.remove(&(namespace.clone(), key.clone()));
                inner
                    .module_streams
                    .remove(&(namespace.clone(), key.clone()));
            }
        }
    }
}

fn replay_matches(inner: &Inner, planned: &[PlannedReplayWrite]) -> bool {
    planned.iter().all(|write| match write {
        PlannedReplayWrite::ObjectSet { id, doc } => inner.objects.get(id) == Some(doc),
        PlannedReplayWrite::ObjectDelete { id } => !inner.objects.contains_key(id),
        PlannedReplayWrite::ObjectAllocatorSet { next_id } => inner.next_id == Some(*next_id),
        PlannedReplayWrite::ModuleJsonSet {
            namespace,
            key,
            doc,
        } => inner.module_kv.get(&(namespace.clone(), key.clone())) == Some(doc),
        PlannedReplayWrite::ModuleDelete { namespace, key } => {
            !inner
                .module_kv
                .contains_key(&(namespace.clone(), key.clone()))
                && !inner
                    .module_streams
                    .contains_key(&(namespace.clone(), key.clone()))
        }
    })
}

fn validate_replay_admission(
    inner: &Inner,
    bundle: &AuthoritativeBundle,
) -> Result<(), StoreError> {
    if inner.next_id.is_none() && !inner.objects.is_empty() {
        return Err(StoreError::AuthoritativeCorrupt(
            "objects exist without an allocator resume point".to_owned(),
        ));
    }
    let current_next_id = inner.next_id.unwrap_or(0);
    if inner.objects.keys().any(|id| *id >= current_next_id) {
        return Err(StoreError::AuthoritativeCorrupt(
            "an object id is not below the allocator resume point".to_owned(),
        ));
    }
    let proposed_next_id = bundle.writes().iter().find_map(|write| match write {
        ReplayWrite::ObjectAllocatorSet { next_id } => Some(*next_id),
        _ => None,
    });
    if proposed_next_id.is_some_and(|next_id| next_id < current_next_id) {
        return Err(StoreError::Invalid(
            "authoritative object allocator post-image would move backwards".to_owned(),
        ));
    }
    for object in bundle.writes().iter().filter_map(|write| match write {
        ReplayWrite::ObjectSet { object } => Some(object),
        _ => None,
    }) {
        if inner.objects.contains_key(&object.id.0) {
            continue;
        }
        let Some(next_id) = proposed_next_id else {
            return Err(StoreError::Invalid(format!(
                "authoritative creation of object #{} has no allocator post-image",
                object.id.0
            )));
        };
        if object.id.0 < current_next_id {
            return Err(StoreError::Invalid(format!(
                "authoritative object set would reuse tombstoned id #{} below allocator {}",
                object.id.0, current_next_id
            )));
        }
        if object.id.0 >= next_id {
            return Err(StoreError::Invalid(format!(
                "authoritative creation of object #{} is not below allocator post-image {}",
                object.id.0, next_id
            )));
        }
    }
    Ok(())
}

fn validate_replay_recovery(inner: &Inner, bundle: &AuthoritativeBundle) -> Result<(), StoreError> {
    let Some(proposed_next_id) = bundle.writes().iter().find_map(|write| match write {
        ReplayWrite::ObjectAllocatorSet { next_id } => Some(*next_id),
        _ => None,
    }) else {
        return Ok(());
    };
    if let Some(current_next_id) = inner.next_id
        && proposed_next_id < current_next_id
    {
        return Err(StoreError::AuthoritativeCorrupt(format!(
            "pending allocator post-image {proposed_next_id} is behind current allocator {current_next_id}"
        )));
    }
    Ok(())
}

fn retained_authoritative_usage(inner: &Inner) -> Result<(usize, usize), StoreError> {
    let mut count = 0usize;
    let mut bytes = 0usize;
    for intent in inner.authoritative_intents.values() {
        count = count.checked_add(1).ok_or_else(|| {
            StoreError::AuthoritativeCorrupt("retained intent count overflowed".to_owned())
        })?;
        bytes = bytes.checked_add(intent.len()).ok_or_else(|| {
            StoreError::AuthoritativeCorrupt("retained intent byte count overflowed".to_owned())
        })?;
    }
    Ok((count, bytes))
}

fn enforce_retained_authoritative_capacity(
    inner: &Inner,
    candidate_bytes: usize,
    max_count: usize,
    max_bytes: usize,
) -> Result<(), StoreError> {
    let (retained_count, retained_bytes) = retained_authoritative_usage(inner)?;
    let attempted_count = retained_count.checked_add(1).ok_or_else(|| {
        StoreError::AuthoritativeCorrupt("retained intent count overflowed".to_owned())
    })?;
    let attempted_bytes = retained_bytes.checked_add(candidate_bytes).ok_or_else(|| {
        StoreError::AuthoritativeCorrupt("retained intent byte count overflowed".to_owned())
    })?;
    if attempted_count > max_count || attempted_bytes > max_bytes {
        return Err(StoreError::AuthoritativeCapacity {
            retained_count: attempted_count,
            retained_bytes: attempted_bytes,
            max_count,
            max_bytes,
        });
    }
    Ok(())
}

impl AuthoritativeStore for InMemoryStore {
    fn reserve_authoritative_intent(
        &self,
        bundle: AuthoritativeBundle,
    ) -> BoxFuture<'_, Result<IntentReceipt, StoreError>> {
        Box::pin(async move {
            let token = bundle.token();
            let bundle_hash = bundle.bundle_hash();
            let key = bundle.idempotency().clone();
            let ingress_hash = bundle.ingress_hash();
            let doc = serde_json::to_string(&bundle)?;
            let mut inner = self.lock()?;

            if let Some(existing_doc) = inner.authoritative_intents.get(&token) {
                let existing = decode_intent(existing_doc)?;
                if existing != bundle {
                    return Err(StoreError::AuthoritativeTokenConflict);
                }
                let Some(binding) = inner.authoritative_idempotency.get(&key) else {
                    return Err(StoreError::AuthoritativeCorrupt(
                        "exact intent is missing its idempotency binding".to_owned(),
                    ));
                };
                if binding.token != token
                    || binding.ingress_hash != ingress_hash
                    || binding.bundle_hash != bundle_hash
                {
                    return Err(StoreError::AuthoritativeCorrupt(
                        "exact intent disagrees with its idempotency binding".to_owned(),
                    ));
                }
                if let Some(receipt_doc) = inner.authoritative_receipts.get(&token) {
                    let receipt = decode_receipt(receipt_doc)?;
                    if receipt.bundle() != &bundle {
                        return Err(StoreError::AuthoritativeCorrupt(
                            "finalized receipt disagrees with its retained intent".to_owned(),
                        ));
                    }
                }
                return Ok(IntentReceipt {
                    token,
                    bundle_hash,
                    resumed: true,
                });
            }
            if let Some(binding) = inner.authoritative_idempotency.get(&key) {
                if binding.token != token
                    || binding.ingress_hash != ingress_hash
                    || binding.bundle_hash != bundle_hash
                {
                    return Err(StoreError::AuthoritativeIdempotencyConflict);
                }
                return Err(StoreError::AuthoritativeCorrupt(
                    "idempotency binding exists without its exact intent".to_owned(),
                ));
            }
            validate_replay_admission(&inner, &bundle)?;
            enforce_retained_authoritative_capacity(
                &inner,
                doc.len(),
                MAX_RETAINED_AUTHORITATIVE_INTENTS,
                MAX_RETAINED_AUTHORITATIVE_BYTES,
            )?;
            inner.authoritative_intents.insert(token, doc);
            inner.authoritative_idempotency.insert(
                key,
                IdempotencyBinding {
                    token,
                    ingress_hash,
                    bundle_hash,
                },
            );
            inner.authoritative_pending.insert(token);
            Ok(IntentReceipt {
                token,
                bundle_hash,
                resumed: false,
            })
        })
    }

    fn authoritative_idempotency_status<'a>(
        &'a self,
        key: &'a ScopedIdempotencyKey,
        ingress_hash: CanonicalHash,
    ) -> BoxFuture<'a, Result<IdempotencyStatus, StoreError>> {
        Box::pin(async move {
            let inner = self.lock()?;
            let Some(binding) = inner.authoritative_idempotency.get(key) else {
                return Ok(IdempotencyStatus::Vacant);
            };
            let Some(intent_doc) = inner.authoritative_intents.get(&binding.token) else {
                return Err(StoreError::AuthoritativeCorrupt(
                    "idempotency binding names no intent or receipt".to_owned(),
                ));
            };
            let intent = decode_intent(intent_doc)?;
            if intent.idempotency() != key {
                return Err(StoreError::AuthoritativeCorrupt(
                    "pending intent disagrees with its idempotency binding".to_owned(),
                ));
            }
            let receipt = validate_retained_intent(&inner, binding.token, &intent)?;
            if binding.ingress_hash != ingress_hash {
                return Ok(IdempotencyStatus::Conflict);
            }
            if let Some(receipt) = receipt {
                return Ok(IdempotencyStatus::FinalizedExact(Box::new(receipt)));
            }
            Ok(IdempotencyStatus::PendingExact {
                token: binding.token,
                bundle_hash: binding.bundle_hash,
            })
        })
    }

    fn reconcile_authoritative(
        &self,
        token: CommitToken,
        expected_bundle_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<AuthoritativeReceipt, StoreError>> {
        Box::pin(async move {
            let intent_doc = {
                let inner = self.lock()?;
                let intent_doc = inner
                    .authoritative_intents
                    .get(&token)
                    .cloned()
                    .ok_or(StoreError::AuthoritativeIntentMissing)?;
                let intent = decode_intent(&intent_doc)?;
                if let Some(receipt) = validate_retained_intent(&inner, token, &intent)? {
                    if receipt.bundle_hash() != expected_bundle_hash {
                        return Err(StoreError::AuthoritativeTokenConflict);
                    }
                    return Ok(receipt);
                }
                intent_doc
            };
            let bundle = decode_intent(&intent_doc)?;
            if bundle.token() != token {
                return Err(StoreError::AuthoritativeCorrupt(
                    "authoritative intent is stored under the wrong token".to_owned(),
                ));
            }
            if bundle.bundle_hash() != expected_bundle_hash {
                return Err(StoreError::AuthoritativeTokenConflict);
            }
            let planned = plan_replay(&bundle)?;
            let receipt = AuthoritativeReceipt::new(bundle);
            let receipt_doc = serde_json::to_string(&receipt)?;

            let mut inner = self.lock()?;
            if inner.authoritative_intents.get(&token) != Some(&intent_doc) {
                return Err(StoreError::AuthoritativeTokenConflict);
            }
            if let Some(existing) = validate_retained_intent(&inner, token, receipt.bundle())? {
                if existing != receipt {
                    return Err(StoreError::AuthoritativeTokenConflict);
                }
                return Ok(existing);
            }
            let Some(binding) = inner
                .authoritative_idempotency
                .get(receipt.bundle().idempotency())
            else {
                return Err(StoreError::AuthoritativeCorrupt(
                    "authoritative intent is missing its idempotency binding".to_owned(),
                ));
            };
            if binding.token != token
                || binding.ingress_hash != receipt.bundle().ingress_hash()
                || binding.bundle_hash != expected_bundle_hash
            {
                return Err(StoreError::AuthoritativeCorrupt(
                    "authoritative intent disagrees with its idempotency binding".to_owned(),
                ));
            }
            validate_replay_recovery(&inner, receipt.bundle())?;
            apply_replay(&mut inner, &planned);
            if !replay_matches(&inner, &planned) {
                return Err(StoreError::AuthoritativeCorrupt(
                    "authoritative replay readback mismatch".to_owned(),
                ));
            }
            inner.authoritative_receipts.insert(token, receipt_doc);
            inner.authoritative_pending.remove(&token);
            Ok(receipt)
        })
    }

    fn authoritative_status(
        &self,
        token: CommitToken,
        expected_bundle_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<BundleStatus, StoreError>> {
        Box::pin(async move {
            let inner = self.lock()?;
            if let Some(intent_doc) = inner.authoritative_intents.get(&token) {
                let intent = decode_intent(intent_doc)?;
                let receipt = validate_retained_intent(&inner, token, &intent)?;
                if intent.bundle_hash() != expected_bundle_hash {
                    return Ok(BundleStatus::Conflict {
                        stored_bundle_hash: intent.bundle_hash(),
                    });
                }
                return Ok(receipt.map_or(BundleStatus::PendingExact, |receipt| {
                    BundleStatus::FinalizedExact(Box::new(receipt))
                }));
            }
            if inner.authoritative_receipts.contains_key(&token) {
                return Err(StoreError::AuthoritativeCorrupt(
                    "finalized receipt has no retained intent".to_owned(),
                ));
            }
            Ok(BundleStatus::Absent)
        })
    }

    fn pending_authoritative_intents(
        &self,
    ) -> BoxFuture<'_, Result<Vec<PendingAuthoritativeIntent>, StoreError>> {
        Box::pin(async move {
            let inner = self.lock()?;
            if inner.authoritative_pending.len() > MAX_RETAINED_AUTHORITATIVE_INTENTS {
                return Err(StoreError::AuthoritativeCorrupt(format!(
                    "pending intent index contains {} entries; maximum is {}",
                    inner.authoritative_pending.len(),
                    MAX_RETAINED_AUTHORITATIVE_INTENTS
                )));
            }
            inner
                .authoritative_pending
                .iter()
                .map(|token| {
                    let intent_doc = inner.authoritative_intents.get(token).ok_or_else(|| {
                        StoreError::AuthoritativeCorrupt(format!(
                            "pending index token {token} has no retained intent"
                        ))
                    })?;
                    let intent = decode_intent(intent_doc)?;
                    validate_retained_intent(&inner, *token, &intent)?;
                    Ok(PendingAuthoritativeIntent {
                        token: *token,
                        bundle_hash: intent.bundle_hash(),
                    })
                })
                .collect()
        })
    }

    fn scan_authoritative_intents(
        &self,
        cursor: Option<AuthoritativeScanCursor>,
        limit: usize,
    ) -> BoxFuture<'_, Result<AuthoritativeIntentPage, StoreError>> {
        Box::pin(async move {
            if limit == 0 || limit > MAX_AUTHORITATIVE_DISCOVERY_PAGE {
                return Err(StoreError::Invalid(format!(
                    "authoritative discovery page limit must be 1..={MAX_AUTHORITATIVE_DISCOVERY_PAGE}"
                )));
            }
            let after = cursor
                .as_ref()
                .map(|cursor| {
                    CommitToken::from_hex(cursor.as_str()).map_err(|error| {
                        StoreError::Invalid(format!(
                            "invalid in-memory authoritative scan cursor: {error}"
                        ))
                    })
                })
                .transpose()?;
            let inner = self.lock()?;
            let mut records = match after {
                Some(after) => inner
                    .authoritative_intents
                    .range((std::ops::Bound::Excluded(after), std::ops::Bound::Unbounded)),
                None => inner.authoritative_intents.range::<CommitToken, _>(..),
            };
            let mut intents = Vec::with_capacity(limit);
            let mut last_token = None;
            for _ in 0..limit {
                let Some((token, intent_doc)) = records.next() else {
                    break;
                };
                let intent = decode_intent(intent_doc)?;
                validate_retained_intent(&inner, *token, &intent)?;
                last_token = Some(*token);
                intents.push(intent);
            }
            let next_cursor = if records.next().is_some() {
                Some(AuthoritativeScanCursor::new(
                    last_token
                        .expect("a non-empty remainder follows a non-empty page")
                        .to_hex(),
                )?)
            } else {
                None
            };
            Ok(AuthoritativeIntentPage {
                intents,
                next_cursor,
            })
        })
    }
}

impl ObjectStore for InMemoryStore {
    fn load_world(&self) -> BoxFuture<'_, Result<Option<WorldSnapshot>, StoreError>> {
        Box::pin(async move {
            let inner = self.lock()?;
            if inner.objects.is_empty() && inner.next_id.is_none() {
                return Ok(None);
            }
            let Some(next_id) = inner.next_id else {
                return Err(StoreError::Corrupt(
                    "objects present but next_id missing".into(),
                ));
            };
            let mut objects = Vec::with_capacity(inner.objects.len());
            let mut corrupt = 0usize;
            for (id, doc) in &inner.objects {
                match serde_json::from_str::<Obj>(doc) {
                    Ok(obj) => objects.push(obj),
                    Err(e) => {
                        corrupt += 1;
                        tracing::error!(id, error = %e, "skipping corrupt object doc");
                    }
                }
            }
            if corrupt > 0 {
                tracing::error!(corrupt, "corrupt object docs skipped during load");
            }
            let schema_version = inner.schema_version.unwrap_or_else(|| {
                tracing::warn!("non-empty store without schema_version; assuming v1");
                1
            });
            Ok(Some(WorldSnapshot {
                objects,
                next_id,
                schema_version,
                corrupt_docs: corrupt,
            }))
        })
    }

    fn write(&self, batch: WriteBatch) -> BoxFuture<'_, Result<WriteReceipt, StoreError>> {
        Box::pin(async move {
            let serial = batch.serial;
            if batch.is_empty() {
                return Ok(WriteReceipt { serial });
            }
            // Validate + serialize (the only fallible steps) before touching
            // state, so a failure leaves the store untouched: all-or-nothing.
            let mut docs = Vec::with_capacity(batch.upserts.len());
            for obj in &batch.upserts {
                docs.push((obj.id.0, serde_json::to_string(obj)?));
            }
            let mut planned = Vec::with_capacity(batch.module_ops.len());
            for op in &batch.module_ops {
                validate_module_op(op)?;
                let ns = op.ns.to_string();
                planned.push(match &op.op {
                    ModuleWrite::JsonSet { key, value } => PlannedModuleOp::JsonSet {
                        ns,
                        key: key.to_string(),
                        doc: serde_json::to_string(value)?,
                    },
                    ModuleWrite::Del { key } => PlannedModuleOp::Del {
                        ns,
                        key: key.to_string(),
                    },
                    ModuleWrite::StreamAppend { key, value, maxlen } => {
                        PlannedModuleOp::StreamAppend {
                            ns,
                            key: key.to_string(),
                            doc: serde_json::to_string(value)?,
                            maxlen: *maxlen,
                        }
                    }
                });
            }
            let mut inner = self.lock()?;
            for (id, doc) in docs {
                inner.objects.insert(id, doc);
            }
            for id in &batch.deletes {
                inner.objects.remove(&id.0);
            }
            // Module ops after the object writes, in queue order.
            for op in planned {
                match op {
                    PlannedModuleOp::JsonSet { ns, key, doc } => {
                        inner.module_kv.insert((ns, key), doc);
                    }
                    PlannedModuleOp::Del { ns, key } => {
                        // Redis DEL is type-agnostic: it removes whatever
                        // lives at mod:{ns}:{key}, JSON doc or stream.
                        inner.module_kv.remove(&(ns.clone(), key.clone()));
                        inner.module_streams.remove(&(ns, key));
                    }
                    PlannedModuleOp::StreamAppend {
                        ns,
                        key,
                        doc,
                        maxlen,
                    } => {
                        let stream = inner.module_streams.entry((ns, key)).or_default();
                        stream.next_seq += 1;
                        let seq = stream.next_seq;
                        stream.entries.push_back((seq, doc));
                        while stream.entries.len() > maxlen as usize {
                            stream.entries.pop_front();
                        }
                    }
                }
            }
            if let Some(next_id) = batch.next_id {
                inner.next_id = Some(next_id);
            }
            inner.last_serial = Some(serial);
            Ok(WriteReceipt { serial })
        })
    }

    fn create_account(&self, rec: AccountRec) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            if rec.name.is_empty() {
                return Err(StoreError::Invalid("empty account name".into()));
            }
            let key = rec.name.to_lowercase();
            let doc = serde_json::to_string(&rec)?;
            let mut inner = self.lock()?;
            if inner.accounts.contains_key(&key) {
                return Err(StoreError::AccountExists);
            }
            inner.accounts.insert(key, doc);
            Ok(())
        })
    }

    fn get_account<'a>(
        &'a self,
        name: &'a str,
    ) -> BoxFuture<'a, Result<Option<AccountRec>, StoreError>> {
        Box::pin(async move {
            let inner = self.lock()?;
            match inner.accounts.get(&name.to_lowercase()) {
                Some(doc) => Ok(Some(serde_json::from_str(doc)?)),
                None => Ok(None),
            }
        })
    }

    fn link_character<'a>(
        &'a self,
        account: &'a str,
        id: ObjId,
        char_name: &'a str,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let key = account.to_lowercase();
            let mut inner = self.lock()?;
            let Some(doc) = inner.accounts.get(&key) else {
                return Err(StoreError::NoSuchAccount(account.to_string()));
            };
            let mut rec: AccountRec = serde_json::from_str(doc)?;
            rec.characters.push(CharacterRef {
                id,
                name: char_name.to_string(),
            });
            let doc = serde_json::to_string(&rec)?;
            inner.accounts.insert(key, doc);
            Ok(())
        })
    }

    fn list_accounts(&self) -> BoxFuture<'_, Result<Vec<AccountRec>, StoreError>> {
        Box::pin(async move {
            let inner = self.lock()?;
            let mut out = Vec::with_capacity(inner.accounts.len());
            for doc in inner.accounts.values() {
                match serde_json::from_str::<AccountRec>(doc) {
                    Ok(rec) => out.push(rec),
                    Err(e) => tracing::error!(error = %e, "skipping corrupt account doc"),
                }
            }
            Ok(out)
        })
    }

    fn set_account_password<'a>(
        &'a self,
        account: &'a str,
        argon2_hash: &'a str,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            let key = account.to_lowercase();
            let mut inner = self.lock()?;
            let Some(doc) = inner.accounts.get(&key) else {
                return Ok(false);
            };
            let mut rec: AccountRec = serde_json::from_str(doc)?;
            rec.argon2_hash = argon2_hash.to_string();
            let doc = serde_json::to_string(&rec)?;
            inner.accounts.insert(key, doc);
            Ok(true)
        })
    }

    fn set_account_ssh_keys<'a>(
        &'a self,
        account: &'a str,
        keys: &'a [SshKeyRec],
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            let key = account.to_lowercase();
            let mut inner = self.lock()?;
            let Some(doc) = inner.accounts.get(&key) else {
                return Ok(false);
            };
            // Round-trip through AccountRec so a doc that predates the
            // ssh_keys field (deserialized via #[serde(default)]) is written
            // back with the new array.
            let mut rec: AccountRec = serde_json::from_str(doc)?;
            rec.ssh_keys = keys.to_vec();
            let doc = serde_json::to_string(&rec)?;
            inner.accounts.insert(key, doc);
            Ok(true)
        })
    }

    fn set_ban<'a>(
        &'a self,
        ip: &'a str,
        until_unix: Option<i64>,
        reason: &'a str,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            if ip.is_empty() {
                return Err(StoreError::Invalid("empty ban ip".into()));
            }
            let rec = BanRec {
                ip: ip.to_string(),
                until_unix,
                reason: reason.to_string(),
            };
            let doc = serde_json::to_string(&rec)?;
            let mut inner = self.lock()?;
            if let Some(until) = until_unix
                && until <= now_unix()
            {
                // Mirrors Redis: a key set with EXAT in the past is
                // immediately gone.
                inner.bans.remove(ip);
                return Ok(());
            }
            inner.bans.insert(ip.to_string(), doc);
            Ok(())
        })
    }

    fn get_bans(&self) -> BoxFuture<'_, Result<Vec<BanRec>, StoreError>> {
        Box::pin(async move {
            let now = now_unix();
            let mut inner = self.lock()?;
            let mut out = Vec::new();
            let mut expired = Vec::new();
            for (ip, doc) in &inner.bans {
                match serde_json::from_str::<BanRec>(doc) {
                    Ok(rec) => {
                        if rec.until_unix.is_some_and(|until| until <= now) {
                            expired.push(ip.clone());
                        } else {
                            out.push(rec);
                        }
                    }
                    Err(e) => tracing::error!(ip, error = %e, "skipping corrupt ban doc"),
                }
            }
            // Prune, as Redis TTL would have.
            for ip in expired {
                inner.bans.remove(&ip);
            }
            Ok(out)
        })
    }

    fn clear_ban<'a>(&'a self, ip: &'a str) -> BoxFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            let mut inner = self.lock()?;
            Ok(inner.bans.remove(ip).is_some())
        })
    }

    fn token_put(&self, hash: String, rec: TokenRecord) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            validate_token_hash(&hash)?;
            let doc = serde_json::to_string(&rec)?;
            let mut inner = self.lock()?;
            if rec.expires <= now_unix() {
                // Mirrors Redis: a key set with EXAT in the past is
                // immediately gone.
                inner.tokens.remove(&hash);
                return Ok(());
            }
            inner.tokens.insert(hash, doc);
            Ok(())
        })
    }

    fn token_get<'a>(
        &'a self,
        hash: &'a str,
    ) -> BoxFuture<'a, Result<Option<TokenRecord>, StoreError>> {
        Box::pin(async move {
            validate_token_hash(hash)?;
            let mut inner = self.lock()?;
            let Some(doc) = inner.tokens.get(hash) else {
                return Ok(None);
            };
            let rec: TokenRecord = serde_json::from_str(doc)?;
            if rec.expires <= now_unix() {
                // Lazy delete, as Redis TTL would have.
                inner.tokens.remove(hash);
                return Ok(None);
            }
            Ok(Some(rec))
        })
    }

    fn token_touch<'a>(
        &'a self,
        hash: &'a str,
        new_expires: i64,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            validate_token_hash(hash)?;
            let mut inner = self.lock()?;
            let Some(doc) = inner.tokens.get(hash) else {
                return Ok(false);
            };
            let mut rec: TokenRecord = serde_json::from_str(doc)?;
            if rec.expires <= now_unix() {
                // Expired = missing; a touch never resurrects.
                inner.tokens.remove(hash);
                return Ok(false);
            }
            if new_expires <= now_unix() {
                // Sliding into the past is a delete.
                inner.tokens.remove(hash);
                return Ok(true);
            }
            rec.expires = new_expires;
            let doc = serde_json::to_string(&rec)?;
            inner.tokens.insert(hash.to_string(), doc);
            Ok(true)
        })
    }

    fn token_del<'a>(&'a self, hash: &'a str) -> BoxFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            validate_token_hash(hash)?;
            let mut inner = self.lock()?;
            let Some(doc) = inner.tokens.remove(hash) else {
                return Ok(false);
            };
            // An expired straggler is already "gone": removed, not reported.
            let live =
                serde_json::from_str::<TokenRecord>(&doc).is_ok_and(|rec| rec.expires > now_unix());
            Ok(live)
        })
    }

    fn tokens_del_account<'a>(
        &'a self,
        account: &'a str,
    ) -> BoxFuture<'a, Result<u64, StoreError>> {
        Box::pin(async move {
            let account_lc = account.to_lowercase();
            let now = now_unix();
            let mut inner = self.lock()?;
            let mut doomed = Vec::new();
            let mut live = 0u64;
            for (hash, doc) in &inner.tokens {
                match serde_json::from_str::<TokenRecord>(doc) {
                    Ok(rec) => {
                        let expired = rec.expires <= now;
                        if expired {
                            // Prune, as Redis TTL would have — not counted.
                            doomed.push(hash.clone());
                        } else if rec.account.to_lowercase() == account_lc {
                            doomed.push(hash.clone());
                            live += 1;
                        }
                    }
                    Err(e) => tracing::error!(error = %e, "skipping corrupt token doc"),
                }
            }
            for hash in doomed {
                inner.tokens.remove(&hash);
            }
            Ok(live)
        })
    }

    fn module_json_get<'a>(
        &'a self,
        ns: &'a str,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Option<serde_json::Value>, StoreError>> {
        Box::pin(async move {
            validate_module_ns(ns)?;
            validate_module_key(key)?;
            let inner = self.lock()?;
            match inner.module_kv.get(&(ns.to_string(), key.to_string())) {
                Some(doc) => Ok(Some(serde_json::from_str(doc)?)),
                None => Ok(None),
            }
        })
    }

    fn module_json_list<'a>(
        &'a self,
        ns: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, serde_json::Value)>, StoreError>> {
        Box::pin(async move {
            validate_module_ns(ns)?;
            let inner = self.lock()?;
            let mut out = Vec::new();
            for ((doc_ns, key), doc) in &inner.module_kv {
                if doc_ns != ns {
                    continue;
                }
                let value = serde_json::from_str(doc)?;
                out.push((key.clone(), value));
            }
            Ok(out)
        })
    }

    fn stream_range<'a>(
        &'a self,
        ns: &'a str,
        key: &'a str,
        count_latest: u32,
    ) -> BoxFuture<'a, Result<Vec<(String, serde_json::Value)>, StoreError>> {
        Box::pin(async move {
            validate_module_ns(ns)?;
            validate_module_key(key)?;
            if count_latest == 0 {
                return Ok(Vec::new());
            }
            let inner = self.lock()?;
            let Some(stream) = inner.module_streams.get(&(ns.to_string(), key.to_string())) else {
                return Ok(Vec::new());
            };
            let skip = stream.entries.len().saturating_sub(count_latest as usize);
            let mut out = Vec::with_capacity(stream.entries.len() - skip);
            for (seq, doc) in stream.entries.iter().skip(skip) {
                out.push((format!("{seq}-0"), serde_json::from_str(doc)?));
            }
            Ok(out)
        })
    }

    fn get_schema_version(&self) -> BoxFuture<'_, Result<Option<u32>, StoreError>> {
        Box::pin(async move { Ok(self.lock()?.schema_version) })
    }

    fn set_schema_version(&self, version: u32) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            self.lock()?.schema_version = Some(version);
            Ok(())
        })
    }

    fn bgsave(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            drop(self.lock()?);
            Ok(()) // nothing to snapshot; the RDB story is Redis's
        })
    }

    fn drain(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            drop(self.lock()?);
            Ok(()) // writes are acknowledged synchronously; nothing pends
        })
    }

    fn close(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            self.inner
                .lock()
                .expect("in-memory store lock poisoned")
                .closed = true;
            Ok(())
        })
    }
}

/// Memory-implementation-specific stream semantics (the cross-impl behavior
/// lives in [`crate::suite`]): exact trimming to `maxlen`, `"N-0"` id
/// format, monotonic ids across trims, and type-agnostic `Del`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ModuleOp, WriteBatch};
    use mush_core::{
        AuthoritativeBundleDraft, AuthoritativeKey, CanonicalId, DefinitionDescriptor,
        DefinitionId, DefinitionKind, DefinitionRegistryBuilder, IdempotencyKey, IdempotencyScope,
        OwnerId, PreparedCausalRef, PrincipalId, RequestUid, RulesValue, World,
    };

    fn append(ns: &str, key: &str, value: serde_json::Value, maxlen: u32) -> ModuleOp {
        ModuleOp {
            ns: ns.into(),
            op: ModuleWrite::StreamAppend {
                key: key.into(),
                value,
                maxlen,
            },
        }
    }

    async fn write_ops(store: &InMemoryStore, serial: u64, module_ops: Vec<ModuleOp>) {
        store
            .write(WriteBatch {
                serial,
                module_ops,
                ..WriteBatch::default()
            })
            .await
            .expect("module-op batch");
    }

    async fn seed_minimal_world(store: &InMemoryStore) {
        let world = World::new_minimal();
        store
            .write(WriteBatch {
                serial: 1,
                upserts: world.iter().cloned().collect(),
                next_id: Some(world.next_id()),
                durable: true,
                ..WriteBatch::default()
            })
            .await
            .expect("seed minimal world");
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum AuthoritativeFaultMode {
        Clean,
        BeforeIntent,
        AfterIntent,
        AfterNMaterializationOps(usize),
        AckLostAfterFinalize,
        TokenConflict,
    }

    struct FaultStore {
        inner: InMemoryStore,
        mode: AuthoritativeFaultMode,
    }

    impl FaultStore {
        fn new(mode: AuthoritativeFaultMode) -> Self {
            Self {
                inner: InMemoryStore::new(),
                mode,
            }
        }

        fn heal(&mut self) {
            self.mode = AuthoritativeFaultMode::Clean;
        }

        async fn reserve(
            &mut self,
            bundle: AuthoritativeBundle,
        ) -> Result<Option<IntentReceipt>, StoreError> {
            match self.mode {
                AuthoritativeFaultMode::BeforeIntent => {
                    self.heal();
                    Ok(None)
                }
                AuthoritativeFaultMode::AfterIntent => {
                    let _ = self.inner.reserve_authoritative_intent(bundle).await?;
                    self.heal();
                    Ok(None)
                }
                _ => self
                    .inner
                    .reserve_authoritative_intent(bundle)
                    .await
                    .map(Some),
            }
        }

        async fn reconcile(
            &mut self,
            token: CommitToken,
            expected_bundle_hash: CanonicalHash,
        ) -> Result<Option<AuthoritativeReceipt>, StoreError> {
            match self.mode {
                AuthoritativeFaultMode::AfterNMaterializationOps(count) => {
                    self.materialize_prefix(token, expected_bundle_hash, count)?;
                    self.heal();
                    Ok(None)
                }
                AuthoritativeFaultMode::AckLostAfterFinalize => {
                    let _ = self
                        .inner
                        .reconcile_authoritative(token, expected_bundle_hash)
                        .await?;
                    self.heal();
                    Ok(None)
                }
                AuthoritativeFaultMode::TokenConflict => {
                    self.heal();
                    self.inner
                        .reconcile_authoritative(token, CanonicalHash::from_bytes([0xee; 32]))
                        .await
                        .map(Some)
                }
                _ => self
                    .inner
                    .reconcile_authoritative(token, expected_bundle_hash)
                    .await
                    .map(Some),
            }
        }

        fn materialize_prefix(
            &self,
            token: CommitToken,
            expected_bundle_hash: CanonicalHash,
            count: usize,
        ) -> Result<(), StoreError> {
            let mut inner = self.inner.lock()?;
            let intent_doc = inner
                .authoritative_intents
                .get(&token)
                .ok_or(StoreError::AuthoritativeIntentMissing)?;
            let bundle = decode_intent(intent_doc)?;
            if bundle.bundle_hash() != expected_bundle_hash {
                return Err(StoreError::AuthoritativeTokenConflict);
            }
            if validate_retained_intent(&inner, token, &bundle)?.is_some() {
                return Err(StoreError::AuthoritativeCorrupt(
                    "fault harness cannot partially apply a finalized intent".to_owned(),
                ));
            }
            let planned = plan_replay(&bundle)?;
            if count > planned.len() {
                return Err(StoreError::Invalid(format!(
                    "fault cut after {count} operations exceeds {} replay writes",
                    planned.len()
                )));
            }
            validate_replay_recovery(&inner, &bundle)?;
            apply_replay(&mut inner, &planned[..count]);
            if !replay_matches(&inner, &planned[..count]) {
                return Err(StoreError::AuthoritativeCorrupt(
                    "fault harness prefix readback mismatch".to_owned(),
                ));
            }
            Ok(())
        }
    }

    fn authoritative_bundle(token_byte: u8, ingress_byte: u8) -> AuthoritativeBundle {
        authoritative_bundle_for_object(token_byte, ingress_byte, ObjId(2), Some(3))
    }

    fn authoritative_bundle_for_object(
        token_byte: u8,
        ingress_byte: u8,
        object_id: ObjId,
        next_id: Option<u32>,
    ) -> AuthoritativeBundle {
        authoritative_bundle_for_request(token_byte, ingress_byte, 1, object_id, next_id)
    }

    fn authoritative_bundle_for_request(
        token_byte: u8,
        ingress_byte: u8,
        request_byte: u8,
        object_id: ObjId,
        next_id: Option<u32>,
    ) -> AuthoritativeBundle {
        let owner = OwnerId::new("toy").unwrap();
        let mut definitions = DefinitionRegistryBuilder::new();
        let action = definitions
            .register(
                owner.clone(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").unwrap(),
                    DefinitionId::new("brace").unwrap(),
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
                    DefinitionId::new("braced").unwrap(),
                    1,
                    serde_json::json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        let request = RequestUid::from_bytes([request_byte; 16]);
        let principal = PrincipalId::Object { id: ObjId(1) };
        let mut writes = vec![
            ReplayWrite::ObjectSet {
                object: Obj::new(object_id, mush_core::ObjKind::Thing, "guard", ObjId(1), 0),
            },
            ReplayWrite::ModuleJsonSet {
                namespace: owner.clone(),
                key: AuthoritativeKey::parse_canonical(format!(
                    "action-state/v1/guarded/{}",
                    object_id.0
                ))
                .unwrap(),
                value: serde_json::json!({ "amount": 1 }),
            },
        ];
        if let Some(next_id) = next_id {
            writes.push(ReplayWrite::ObjectAllocatorSet { next_id });
        }
        AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: CommitToken::from_bytes([token_byte; 16]),
            request,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: principal.clone(),
                    owner: owner.clone(),
                    namespace: CanonicalId::new("brace").unwrap(),
                },
                0,
                IdempotencyKey::Request(request),
            ),
            ingress_hash: CanonicalHash::from_bytes([ingress_byte; 32]),
            request_hash: CanonicalHash::from_bytes([3; 32]),
            plan_hash: CanonicalHash::from_bytes([4; 32]),
            action: action.clone(),
            provider: owner.clone(),
            cause: PreparedCausalRef::new(request, None, 0).unwrap(),
            definitions: vec![action, outcome_definition.clone()],
            writes,
            outcome: RulesValue::new(outcome_definition, serde_json::json!({ "ok": true }))
                .unwrap(),
            eligible_witnesses: vec![principal],
            durable_outbox: vec![],
        })
        .unwrap()
    }

    #[tokio::test]
    async fn authoritative_creation_requires_allocator_and_never_reuses_a_tombstone() {
        let store = InMemoryStore::new();
        seed_minimal_world(&store).await;

        let missing_allocator = authoritative_bundle_for_object(7, 8, ObjId(2), None);
        assert!(matches!(
            store.reserve_authoritative_intent(missing_allocator).await,
            Err(StoreError::Invalid(message)) if message.contains("no allocator")
        ));

        store
            .write(WriteBatch {
                serial: 2,
                deletes: vec![mush_core::LIMBO],
                durable: true,
                ..WriteBatch::default()
            })
            .await
            .unwrap();
        let reuse = authoritative_bundle_for_object(9, 10, mush_core::LIMBO, Some(3));
        assert!(matches!(
            store.reserve_authoritative_intent(reuse).await,
            Err(StoreError::Invalid(message)) if message.contains("reuse tombstoned")
        ));
    }

    #[tokio::test]
    async fn authoritative_recovery_finishes_object_after_allocator_was_applied() {
        let store = InMemoryStore::new();
        seed_minimal_world(&store).await;
        let bundle = authoritative_bundle(11, 12);
        let token = bundle.token();
        let hash = bundle.bundle_hash();
        store.reserve_authoritative_intent(bundle).await.unwrap();

        store.lock().unwrap().next_id = Some(3);
        let receipt = store.reconcile_authoritative(token, hash).await.unwrap();
        assert_eq!(receipt.token(), token);
        let snapshot = store.load_world().await.unwrap().unwrap();
        assert_eq!(snapshot.next_id, 3);
        assert!(snapshot.objects.iter().any(|object| object.id == ObjId(2)));
    }

    #[tokio::test]
    async fn authoritative_discovery_scans_past_a_missing_pending_index_entry() {
        let store = InMemoryStore::new();
        seed_minimal_world(&store).await;
        let first = authoritative_bundle_for_request(21, 31, 41, ObjId(1), None);
        let second = authoritative_bundle_for_request(22, 32, 42, ObjId(1), None);
        let hidden_pending = authoritative_bundle_for_request(23, 33, 43, ObjId(1), None);

        for bundle in [&first, &second] {
            store
                .reserve_authoritative_intent(bundle.clone())
                .await
                .unwrap();
            store
                .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
                .await
                .unwrap();
        }
        store
            .reserve_authoritative_intent(hidden_pending.clone())
            .await
            .unwrap();
        assert_eq!(
            store.pending_authoritative_intents().await.unwrap().len(),
            1
        );
        store
            .lock()
            .unwrap()
            .authoritative_pending
            .remove(&hidden_pending.token());
        assert!(
            store
                .pending_authoritative_intents()
                .await
                .unwrap()
                .is_empty()
        );

        let mut cursor = None;
        let mut discovered = Vec::new();
        loop {
            let page = store.scan_authoritative_intents(cursor, 1).await.unwrap();
            discovered.extend(page.intents.iter().map(AuthoritativeBundle::token));
            let Some(next) = page.next_cursor else {
                break;
            };
            cursor = Some(next);
        }
        assert_eq!(
            discovered,
            vec![first.token(), second.token(), hidden_pending.token()]
        );
        assert_eq!(
            store
                .authoritative_status(hidden_pending.token(), hidden_pending.bundle_hash())
                .await
                .unwrap(),
            BundleStatus::PendingExact
        );
    }

    #[tokio::test]
    async fn authoritative_fault_matrix_converges_without_rerunning_the_bundle()
    -> Result<(), StoreError> {
        let modes = [
            AuthoritativeFaultMode::BeforeIntent,
            AuthoritativeFaultMode::AfterIntent,
            AuthoritativeFaultMode::AfterNMaterializationOps(0),
            AuthoritativeFaultMode::AfterNMaterializationOps(1),
            AuthoritativeFaultMode::AfterNMaterializationOps(2),
            AuthoritativeFaultMode::AfterNMaterializationOps(3),
            AuthoritativeFaultMode::AckLostAfterFinalize,
            AuthoritativeFaultMode::TokenConflict,
        ];

        for mode in modes {
            let mut store = FaultStore::new(mode);
            seed_minimal_world(&store.inner).await;
            let bundle = authoritative_bundle(51, 61);
            let token = bundle.token();
            let hash = bundle.bundle_hash();
            let key = bundle.idempotency().clone();
            let ingress_hash = bundle.ingress_hash();

            if store.reserve(bundle.clone()).await?.is_none() {
                match mode {
                    AuthoritativeFaultMode::BeforeIntent => assert_eq!(
                        store.inner.authoritative_status(token, hash).await?,
                        BundleStatus::Absent
                    ),
                    AuthoritativeFaultMode::AfterIntent => assert_eq!(
                        store.inner.authoritative_status(token, hash).await?,
                        BundleStatus::PendingExact
                    ),
                    _ => unreachable!("only intent faults lose the reserve result"),
                }
                store
                    .reserve(bundle.clone())
                    .await?
                    .expect("healed exact reservation");
            }

            let first_reconcile = store.reconcile(token, hash).await;
            match mode {
                AuthoritativeFaultMode::AfterNMaterializationOps(_)
                | AuthoritativeFaultMode::AckLostAfterFinalize => {
                    assert!(first_reconcile?.is_none());
                }
                AuthoritativeFaultMode::TokenConflict => {
                    assert!(matches!(
                        first_reconcile,
                        Err(StoreError::AuthoritativeTokenConflict)
                    ));
                }
                _ => {
                    first_reconcile?.expect("clean reconciliation");
                }
            }

            let receipt = store
                .reconcile(token, hash)
                .await?
                .expect("exact recovery receipt");
            assert_eq!(receipt.bundle(), &bundle, "fault mode {mode:?}");
            assert_eq!(
                store.inner.authoritative_status(token, hash).await?,
                BundleStatus::FinalizedExact(Box::new(receipt.clone())),
                "fault mode {mode:?}"
            );
            assert_eq!(
                store
                    .inner
                    .authoritative_idempotency_status(&key, ingress_hash)
                    .await?,
                IdempotencyStatus::FinalizedExact(Box::new(receipt.clone())),
                "fault mode {mode:?}"
            );
            assert!(
                store
                    .inner
                    .reserve_authoritative_intent(bundle.clone())
                    .await?
                    .resumed,
                "fault mode {mode:?}"
            );
            assert_eq!(
                store.inner.reconcile_authoritative(token, hash).await?,
                receipt,
                "fault mode {mode:?}"
            );
            let inner = store.inner.lock()?;
            assert_eq!(inner.authoritative_intents.len(), 1, "fault mode {mode:?}");
            assert_eq!(inner.authoritative_receipts.len(), 1, "fault mode {mode:?}");
            assert!(
                inner.authoritative_pending.is_empty(),
                "fault mode {mode:?}"
            );
            assert_eq!(inner.next_id, Some(3), "fault mode {mode:?}");
            assert!(inner.objects.contains_key(&2), "fault mode {mode:?}");
        }

        Ok(())
    }

    #[test]
    fn authoritative_retained_capacity_is_atomic_and_index_independent() {
        let store = InMemoryStore::new();
        let mut inner = store.lock().unwrap();
        for index in 0..2u8 {
            let mut token = [0; 16];
            token[15] = index;
            inner
                .authoritative_intents
                .insert(CommitToken::from_bytes(token), "intent".to_owned());
        }
        assert!(matches!(
            enforce_retained_authoritative_capacity(&inner, 7, 2, usize::MAX),
            Err(StoreError::AuthoritativeCapacity {
                retained_count: 3,
                retained_bytes: 19,
                max_count: 2,
                max_bytes: usize::MAX,
            })
        ));
        assert_eq!(inner.authoritative_intents.len(), 2);

        assert!(matches!(
            enforce_retained_authoritative_capacity(&inner, 7, usize::MAX, 18),
            Err(StoreError::AuthoritativeCapacity {
                retained_count: 3,
                retained_bytes: 19,
                max_count: usize::MAX,
                max_bytes: 18,
            })
        ));
        assert!(inner.authoritative_pending.is_empty());
    }

    #[tokio::test]
    async fn authoritative_intent_reserve_lookup_and_finalize_are_exact() {
        let store = InMemoryStore::new();
        seed_minimal_world(&store).await;
        let bundle = authoritative_bundle(2, 5);
        let token = bundle.token();
        let hash = bundle.bundle_hash();
        let key = bundle.idempotency().clone();
        let ingress_hash = bundle.ingress_hash();

        assert_eq!(
            store
                .authoritative_idempotency_status(&key, ingress_hash)
                .await
                .unwrap(),
            IdempotencyStatus::Vacant
        );
        assert_eq!(
            store
                .reserve_authoritative_intent(bundle.clone())
                .await
                .unwrap(),
            IntentReceipt {
                token,
                bundle_hash: hash,
                resumed: false,
            }
        );
        assert_eq!(
            store.authoritative_status(token, hash).await.unwrap(),
            BundleStatus::PendingExact
        );
        assert_eq!(
            store
                .authoritative_idempotency_status(&key, ingress_hash)
                .await
                .unwrap(),
            IdempotencyStatus::PendingExact {
                token,
                bundle_hash: hash,
            }
        );
        assert!(
            store
                .reserve_authoritative_intent(bundle)
                .await
                .unwrap()
                .resumed
        );

        let receipt = store.reconcile_authoritative(token, hash).await.unwrap();
        assert_eq!(receipt.token(), token);
        assert_eq!(
            store.authoritative_status(token, hash).await.unwrap(),
            BundleStatus::FinalizedExact(Box::new(receipt.clone()))
        );
        assert_eq!(
            store
                .authoritative_idempotency_status(&key, ingress_hash)
                .await
                .unwrap(),
            IdempotencyStatus::FinalizedExact(Box::new(receipt.clone()))
        );
        assert_eq!(
            store.reconcile_authoritative(token, hash).await.unwrap(),
            receipt,
            "reconciliation is idempotent after finalization"
        );
        assert_eq!(
            store
                .module_json_get("toy", "action-state/v1/guarded/2")
                .await
                .unwrap(),
            Some(serde_json::json!({ "amount": 1 }))
        );
        let snapshot = store.load_world().await.unwrap().unwrap();
        assert_eq!(snapshot.next_id, 3);
        assert_eq!(snapshot.objects.len(), 3);
        assert!(snapshot.objects.iter().any(|object| object.id == ObjId(2)));
    }

    #[tokio::test]
    async fn authoritative_conflicts_never_replace_the_first_binding() {
        let store = InMemoryStore::new();
        seed_minimal_world(&store).await;
        let first = authoritative_bundle(2, 5);
        let key = first.idempotency().clone();
        let first_hash = first.bundle_hash();
        store
            .reserve_authoritative_intent(first.clone())
            .await
            .unwrap();

        assert_eq!(
            store
                .authoritative_idempotency_status(&key, CanonicalHash::from_bytes([9; 32]))
                .await
                .unwrap(),
            IdempotencyStatus::Conflict
        );
        assert!(matches!(
            store
                .reserve_authoritative_intent(authoritative_bundle(3, 6))
                .await,
            Err(StoreError::AuthoritativeIdempotencyConflict)
        ));
        assert!(matches!(
            store
                .authoritative_status(first.token(), CanonicalHash::from_bytes([8; 32]))
                .await
                .unwrap(),
            BundleStatus::Conflict { .. }
        ));
        assert_eq!(
            store
                .authoritative_idempotency_status(&key, first.ingress_hash())
                .await
                .unwrap(),
            IdempotencyStatus::PendingExact {
                token: first.token(),
                bundle_hash: first_hash,
            }
        );
    }

    #[tokio::test]
    async fn stream_ids_are_n_dash_zero_and_survive_trims() {
        let store = InMemoryStore::new();
        // Five appends, cap 2: exact trim keeps exactly the newest two, and
        // the surviving ids show the counter never reset.
        for i in 1..=5 {
            write_ops(&store, i, vec![append("m", "log", serde_json::json!(i), 2)]).await;
        }
        let entries = store.stream_range("m", "log", 10).await.expect("range");
        let got: Vec<(&str, i64)> = entries
            .iter()
            .map(|(id, v)| (id.as_str(), v.as_i64().unwrap()))
            .collect();
        assert_eq!(got, [("4-0", 4), ("5-0", 5)], "exact trim, monotonic ids");

        // The next append continues the sequence.
        write_ops(&store, 6, vec![append("m", "log", serde_json::json!(6), 2)]).await;
        let entries = store.stream_range("m", "log", 1).await.expect("range");
        assert_eq!(entries[0].0, "6-0");
    }

    #[tokio::test]
    async fn del_removes_streams_too() {
        let store = InMemoryStore::new();
        write_ops(
            &store,
            1,
            vec![
                append("m", "log", serde_json::json!("entry"), 8),
                ModuleOp {
                    ns: "m".into(),
                    op: ModuleWrite::JsonSet {
                        key: "doc".into(),
                        value: serde_json::json!({"a": 1}),
                    },
                },
            ],
        )
        .await;
        write_ops(
            &store,
            2,
            vec![
                ModuleOp {
                    ns: "m".into(),
                    op: ModuleWrite::Del { key: "log".into() },
                },
                ModuleOp {
                    ns: "m".into(),
                    op: ModuleWrite::Del { key: "doc".into() },
                },
            ],
        )
        .await;
        assert!(
            store
                .stream_range("m", "log", 10)
                .await
                .expect("range")
                .is_empty(),
            "Del must destroy the stream (Redis DEL is type-agnostic)"
        );
        assert_eq!(
            store.module_json_get("m", "doc").await.expect("get"),
            None,
            "Del must remove the JSON doc"
        );
        // A recreated stream is a *new* stream: the counter starts over.
        write_ops(
            &store,
            3,
            vec![append("m", "log", serde_json::json!("x"), 8)],
        )
        .await;
        let entries = store.stream_range("m", "log", 10).await.expect("range");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "1-0");
    }

    #[tokio::test]
    async fn module_json_list_returns_corrupt_stored_json_as_an_error() {
        let store = InMemoryStore::new();
        store
            .lock()
            .expect("store lock")
            .module_kv
            .insert(("m".into(), "broken".into()), "{not-json".into());

        let error = store
            .module_json_list("m")
            .await
            .expect_err("corrupt module JSON must never disappear from boot discovery");
        assert!(matches!(error, StoreError::Serde(_)), "{error:?}");

        let error = store
            .module_json_get("m", "broken")
            .await
            .expect_err("corrupt module JSON must also fail an exact read");
        assert!(matches!(error, StoreError::Serde(_)), "{error:?}");
    }
}

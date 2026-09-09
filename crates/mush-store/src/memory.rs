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
    AuthoritativeBundle, CanonicalHash, CommitToken, IdempotencyScope, Obj, ObjId, ReplayWrite,
    ScopedIdempotencyKey,
};

use crate::redis::{META_NEXT_ID, MOD_PREFIX, OBJ_PREFIX};
use crate::store::{AuthoritativeStore, ModuleDocs, ObjectStore};
use crate::types::{
    AccountRec, AuthoritativeCheckpointReceipt, AuthoritativeDiscardReceipt,
    AuthoritativeEpochRetirement, AuthoritativeIntentPage, AuthoritativeOutboxAttemptResult,
    AuthoritativeOutboxRecord, AuthoritativeQuarantineReceipt, AuthoritativeReadbackPhase,
    AuthoritativeReceipt, AuthoritativeRetentionUsage, AuthoritativeScanCursor, BanRec,
    BoundIdempotencyStatus, BundleStatus, CharacterRef, DiscordBanRec, DiscordCodeRecord,
    DiscordLinkRec, IdempotencyStatus, IntentReceipt, MAX_AUTHORITATIVE_DISCOVERY_PAGE,
    MAX_PENDING_AUTHORITATIVE_OUTBOX, MAX_RETAINED_AUTHORITATIVE_BYTES,
    MAX_RETAINED_AUTHORITATIVE_INTENTS, ModuleWrite, PendingAuthoritativeIntent, SshKeyRec,
    StoreError, TokenRecord, WorldSnapshot, WriteBatch, WriteReceipt, validate_module_key,
    validate_module_ns, validate_module_op, validate_token_hash,
};

/// See the [module docs](self).
#[derive(Debug, Default)]
pub struct InMemoryStore {
    inner: Mutex<Inner>,
    /// The ledger archive tier, when attached (J5).
    archive: Option<std::sync::Arc<crate::archive::LedgerArchive>>,
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
    /// Discord user id → lowercase account name (`discord:user:{id}` in
    /// Redis): the link's uniqueness lock (D1).
    discord_users: HashMap<u64, String>,
    /// link-code hash → serialized `DiscordCodeRecord` (`discord:code:{hash}`),
    /// pruned lazily against the system clock like `tokens`.
    discord_codes: HashMap<String, String>,
    /// Discord user id → serialized `DiscordBanRec` (`discord:ban:{id}`).
    discord_bans: HashMap<u64, String>,
    /// `(ns, key)` → serialized module JSON doc (`mod:{ns}:{key}` in
    /// Redis). The tuple key makes namespace isolation structural — no
    /// prefix matching to get wrong.
    module_kv: BTreeMap<(String, String), String>,
    /// `(ns, key)` → capped module stream (`mod:{ns}:{key}` in Redis).
    module_streams: BTreeMap<(String, String), MemStream>,
    /// Highest acknowledged batch serial (debug/ops visibility).
    last_serial: Option<u64>,
    /// Immutable full intents, retained until exact checkpoint compaction.
    authoritative_intents: BTreeMap<CommitToken, String>,
    /// Immutable compact finalized/checkpoint witnesses.
    authoritative_receipts: BTreeMap<CommitToken, String>,
    /// Compact witnesses for never-finalized intents released only after
    /// independent quarantine custody was established.
    authoritative_quarantines: BTreeMap<CommitToken, String>,
    /// Early lookup binding installed atomically with the full intent.
    authoritative_idempotency: BTreeMap<ScopedIdempotencyKey, IdempotencyBinding>,
    /// Durable per-scope retry horizon. Epochs below this watermark can never
    /// become vacant again after their compact receipts are removed.
    authoritative_minimum_epochs: BTreeMap<IdempotencyScope, u64>,
    /// Stable durable-delivery records, independently addressable by their
    /// parent-token/ordinal hash.
    authoritative_outbox: BTreeMap<CanonicalHash, String>,
    /// Non-terminal delivery accelerator, materialized with finalization.
    authoritative_outbox_pending: BTreeSet<CanonicalHash>,
    /// Independently enumerable recovery index. Later fault tests deliberately
    /// damage this index and prove exhaustive intent discovery repairs it.
    authoritative_pending: BTreeSet<CommitToken>,
    /// Changes whenever a full retained intent is added or released.
    authoritative_retention_generation: u64,
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

    /// Attach the ledger archive tier (J5): `log:` keys the primary lacks are
    /// served from it, and listings merge it in.
    #[must_use]
    pub fn with_archive(mut self, archive: std::sync::Arc<crate::archive::LedgerArchive>) -> Self {
        self.archive = Some(archive);
        self
    }

    /// Lock the state, failing if the store is closed.
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Inner>, StoreError> {
        let inner = self.inner.lock().expect("in-memory store lock poisoned");
        if inner.closed {
            return Err(StoreError::Closed);
        }
        Ok(inner)
    }

    /// Test/ops seam: write `text` as the whole document under
    /// `canonical_key` — the key [`crate::RedisStore`] would have used —
    /// with no encoding, no validation and no id bookkeeping beyond the
    /// allocator floor below.
    ///
    /// The mirror of [`RedisStore::debug_plant_raw_doc`], and the reason it
    /// exists: refusals are only provable against damage that can actually be
    /// planted, and the damage worth planting is what a store can hold but
    /// this crate would never write — a body whose id disagrees with its key,
    /// a document nested past [`mush_core::MAX_DOC_DEPTH`], an `account:`
    /// suffix that is not the lowercase form every read normalizes to (a
    /// shadow doc no read resolves, exactly as in Redis: the key is taken
    /// verbatim here for that fidelity).
    ///
    /// Two differences from the Redis seam, both structural: this store keys
    /// its maps by decoded id, so only the `obj:` and `account:` prefixes can
    /// be addressed (anything else is [`StoreError::Invalid`] rather than a
    /// silently dropped write), and planting `obj:{id}` lifts `meta:next_id`
    /// above `id` — an object at or above the allocator resume point is
    /// corruption in its own right, and every authoritative write would be
    /// refused by `validate_replay_admission` before the planted damage ever
    /// got its turn.
    ///
    /// One difference that is **not** structural and matters to any shared
    /// suite: this store takes `text` as bytes, while the Redis seam goes
    /// through `JSON.SET`, which parses and re-serializes. So a plant nested
    /// past `serde_json`'s 128-level recursion limit lands here and is
    /// refused there — keep cross-backend plants inside
    /// (`MAX_DOC_DEPTH`, 128) — and a readback that is byte-identical here is
    /// only *semantically* identical there.
    ///
    /// Never call this from production code: it bypasses every invariant this
    /// crate maintains.
    ///
    /// [`RedisStore::debug_plant_raw_doc`]: crate::RedisStore::debug_plant_raw_doc
    #[doc(hidden)]
    pub async fn debug_plant_raw_doc(
        &self,
        canonical_key: &str,
        text: &str,
    ) -> Result<(), StoreError> {
        let mut inner = self.lock()?;
        if let Some(suffix) = canonical_key.strip_prefix(crate::redis::OBJ_PREFIX) {
            let id: u32 = suffix.parse().map_err(|_| {
                StoreError::Invalid(format!("planted key {canonical_key:?} has no object id"))
            })?;
            let Some(floor) = id.checked_add(1) else {
                return Err(StoreError::Invalid(
                    "planted object id has no allocator resume point above it".to_owned(),
                ));
            };
            inner.objects.insert(id, text.to_owned());
            if inner.next_id.is_none_or(|next_id| next_id < floor) {
                inner.next_id = Some(floor);
            }
            Ok(())
        } else if let Some(suffix) = canonical_key.strip_prefix(crate::redis::ACCOUNT_PREFIX) {
            inner.accounts.insert(suffix.to_owned(), text.to_owned());
            Ok(())
        } else {
            Err(StoreError::Invalid(format!(
                "cannot plant {canonical_key:?}: only obj:* and account:* keys are addressable"
            )))
        }
    }

    /// Test/ops seam: the document text stored under `canonical_key`, exactly
    /// as the store holds it, or `None` when the key is gone.
    ///
    /// The read half of [`debug_plant_raw_doc`](Self::debug_plant_raw_doc):
    /// a decode-free readback is what proves a read-only diagnostic left the
    /// bytes alone. Here that readback is byte-verbatim, because this store
    /// holds the bytes; the Redis mirror returns RedisJSON's own
    /// re-serialization of the same document, so only the decoded value is
    /// comparable across the two.
    #[doc(hidden)]
    pub async fn debug_read_raw_doc(
        &self,
        canonical_key: &str,
    ) -> Result<Option<String>, StoreError> {
        let inner = self.lock()?;
        if let Some(suffix) = canonical_key.strip_prefix(crate::redis::OBJ_PREFIX) {
            let id: u32 = suffix.parse().map_err(|_| {
                StoreError::Invalid(format!("planted key {canonical_key:?} has no object id"))
            })?;
            Ok(inner.objects.get(&id).cloned())
        } else if let Some(suffix) = canonical_key.strip_prefix(crate::redis::ACCOUNT_PREFIX) {
            Ok(inner.accounts.get(suffix).cloned())
        } else {
            Err(StoreError::Invalid(format!(
                "cannot read {canonical_key:?}: only obj:* and account:* keys are addressable"
            )))
        }
    }
}

/// One deterministic interruption point in the reusable authoritative-store
/// fault harness. `Clean` performs the ordinary call; every other mode fires
/// once and then heals so an exact retry can prove recovery.
///
/// This is public only for cross-crate contract fixtures. Production code
/// should use [`InMemoryStore`] or [`crate::RedisStore`] directly.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthoritativeFaultMode {
    Clean,
    BeforeIntent,
    AfterIntent,
    AfterNMaterializationOps(usize),
    AckLostAfterFinalize,
    AckLostAfterCheckpoint,
    TokenConflict,
}

/// Reusable K8 fault driver over the exact in-memory authoritative protocol.
///
/// A faulted call returns `Ok(None)`: this models an interruption or lost
/// acknowledgement whose durable side may or may not have happened. The
/// caller must inspect exact status and retry the same token/hash instead of
/// treating `None` as refusal. Deterministic store errors remain `Err`.
#[doc(hidden)]
#[derive(Debug)]
pub struct AuthoritativeFaultStore {
    inner: InMemoryStore,
    mode: AuthoritativeFaultMode,
}

impl AuthoritativeFaultStore {
    #[must_use]
    pub fn new(mode: AuthoritativeFaultMode) -> Self {
        Self {
            inner: InMemoryStore::new(),
            mode,
        }
    }

    #[must_use]
    pub const fn store(&self) -> &InMemoryStore {
        &self.inner
    }

    #[must_use]
    pub const fn mode(&self) -> AuthoritativeFaultMode {
        self.mode
    }

    pub fn arm(&mut self, mode: AuthoritativeFaultMode) {
        self.mode = mode;
    }

    pub fn heal(&mut self) {
        self.mode = AuthoritativeFaultMode::Clean;
    }

    pub async fn reserve(
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

    pub async fn reconcile(
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

    pub async fn checkpoint(
        &mut self,
        token: CommitToken,
        expected_bundle_hash: CanonicalHash,
    ) -> Result<Option<AuthoritativeCheckpointReceipt>, StoreError> {
        match self.mode {
            AuthoritativeFaultMode::AckLostAfterCheckpoint => {
                let _ = self
                    .inner
                    .checkpoint_authoritative(token, expected_bundle_hash)
                    .await?;
                self.heal();
                Ok(None)
            }
            _ => self
                .inner
                .checkpoint_authoritative(token, expected_bundle_hash)
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
        if let Some(write) = first_replay_mismatch(&inner, &planned[..count]) {
            return Err(StoreError::AuthoritativeCorrupt(format!(
                "fault harness prefix readback mismatch at key {:?}",
                planned_write_key(write)
            )));
        }
        Ok(())
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
    let bundle: AuthoritativeBundle = serde_json::from_str(doc)
        .map_err(|error| StoreError::AuthoritativeCorrupt(error.to_string()))?;
    let canonical = serde_json::to_string(&bundle)
        .map_err(|error| StoreError::AuthoritativeCorrupt(error.to_string()))?;
    if canonical != doc {
        return Err(StoreError::AuthoritativeCorrupt(
            "retained intent document is not in canonical storage form".to_owned(),
        ));
    }
    Ok(bundle)
}

fn decode_receipt(doc: &str) -> Result<AuthoritativeReceipt, StoreError> {
    serde_json::from_str(doc).map_err(|error| StoreError::AuthoritativeCorrupt(error.to_string()))
}

fn decode_checkpoint_receipt(doc: &str) -> Result<AuthoritativeReceipt, StoreError> {
    let receipt = decode_receipt(doc)?;
    let canonical = serde_json::to_string(&receipt)
        .map_err(|error| StoreError::AuthoritativeCorrupt(error.to_string()))?;
    if canonical != doc {
        return Err(StoreError::AuthoritativeCorrupt(
            "checkpoint receipt document is not in canonical storage form".to_owned(),
        ));
    }
    Ok(receipt)
}

fn decode_quarantine_receipt(doc: &str) -> Result<AuthoritativeQuarantineReceipt, StoreError> {
    let receipt: AuthoritativeQuarantineReceipt = serde_json::from_str(doc)
        .map_err(|error| StoreError::AuthoritativeCorrupt(error.to_string()))?;
    receipt.validate()?;
    let canonical = serde_json::to_string(&receipt)
        .map_err(|error| StoreError::AuthoritativeCorrupt(error.to_string()))?;
    if canonical != doc {
        return Err(StoreError::AuthoritativeCorrupt(
            "quarantine receipt document is not in canonical storage form".to_owned(),
        ));
    }
    Ok(receipt)
}

fn decode_outbox_record(doc: &str) -> Result<AuthoritativeOutboxRecord, StoreError> {
    let record: AuthoritativeOutboxRecord = serde_json::from_str(doc)
        .map_err(|error| StoreError::AuthoritativeCorrupt(error.to_string()))?;
    record.validate()?;
    let canonical = serde_json::to_string(&record)
        .map_err(|error| StoreError::AuthoritativeCorrupt(error.to_string()))?;
    if canonical != doc {
        return Err(StoreError::AuthoritativeCorrupt(
            "authoritative outbox record is not in canonical storage form".to_owned(),
        ));
    }
    Ok(record)
}

fn expected_outbox_records(
    bundle: &AuthoritativeBundle,
) -> Result<Vec<AuthoritativeOutboxRecord>, StoreError> {
    bundle
        .durable_outbox()
        .iter()
        .cloned()
        .map(|intent| AuthoritativeOutboxRecord::new(bundle.token(), bundle.bundle_hash(), intent))
        .collect()
}

fn validate_bundle_outbox(inner: &Inner, bundle: &AuthoritativeBundle) -> Result<(), StoreError> {
    for expected in expected_outbox_records(bundle)? {
        let doc = inner
            .authoritative_outbox
            .get(&expected.item_id())
            .ok_or_else(|| {
                StoreError::AuthoritativeCorrupt(
                    "finalized action is missing a durable outbox record".to_owned(),
                )
            })?;
        let record = decode_outbox_record(doc)?;
        if record.parent_token() != expected.parent_token()
            || record.parent_bundle_hash() != expected.parent_bundle_hash()
            || record.intent() != expected.intent()
        {
            return Err(StoreError::AuthoritativeCorrupt(
                "durable outbox record disagrees with its parent action".to_owned(),
            ));
        }
        if record.state().is_terminal()
            == inner
                .authoritative_outbox_pending
                .contains(&record.item_id())
        {
            return Err(StoreError::AuthoritativeCorrupt(
                "durable outbox pending index disagrees with item state".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_outbox_parent(
    inner: &Inner,
    record: &AuthoritativeOutboxRecord,
) -> Result<(), StoreError> {
    let receipt_doc = inner
        .authoritative_receipts
        .get(&record.parent_token())
        .ok_or_else(|| {
            StoreError::AuthoritativeCorrupt(
                "durable outbox item has no finalized parent".to_owned(),
            )
        })?;
    let receipt = decode_receipt(receipt_doc)?;
    if receipt.bundle_hash() != record.parent_bundle_hash() {
        return Err(StoreError::AuthoritativeCorrupt(
            "durable outbox item disagrees with its finalized parent".to_owned(),
        ));
    }
    Ok(())
}

fn validate_receipt_binding(
    inner: &Inner,
    stored_token: CommitToken,
    receipt: &AuthoritativeReceipt,
) -> Result<(), StoreError> {
    if receipt.token() != stored_token {
        return Err(StoreError::AuthoritativeCorrupt(
            "authoritative receipt is stored under the wrong token".to_owned(),
        ));
    }
    let binding = inner
        .authoritative_idempotency
        .get(receipt.idempotency())
        .ok_or_else(|| {
            StoreError::AuthoritativeCorrupt(
                "authoritative receipt is missing its idempotency binding".to_owned(),
            )
        })?;
    if binding.token != stored_token
        || binding.ingress_hash != receipt.ingress_hash()
        || binding.bundle_hash != receipt.bundle_hash()
    {
        return Err(StoreError::AuthoritativeCorrupt(
            "authoritative receipt disagrees with its idempotency binding".to_owned(),
        ));
    }
    Ok(())
}

fn validate_quarantine_binding(
    inner: &Inner,
    stored_token: CommitToken,
    receipt: &AuthoritativeQuarantineReceipt,
) -> Result<(), StoreError> {
    if receipt.token() != stored_token {
        return Err(StoreError::AuthoritativeCorrupt(
            "authoritative quarantine receipt is stored under the wrong token".to_owned(),
        ));
    }
    let binding = inner
        .authoritative_idempotency
        .get(receipt.idempotency())
        .ok_or_else(|| {
            StoreError::AuthoritativeCorrupt(
                "authoritative quarantine receipt is missing its idempotency binding".to_owned(),
            )
        })?;
    if binding.token != stored_token
        || binding.ingress_hash != receipt.ingress_hash()
        || binding.bundle_hash != receipt.bundle_hash()
    {
        return Err(StoreError::AuthoritativeCorrupt(
            "authoritative quarantine receipt disagrees with its idempotency binding".to_owned(),
        ));
    }
    Ok(())
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
    if !receipt.matches_bundle(intent) {
        return Err(StoreError::AuthoritativeCorrupt(
            "finalized receipt disagrees with its retained intent".to_owned(),
        ));
    }
    validate_bundle_outbox(inner, intent)?;
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
                ReplayWrite::LedgerChunkSet { write } => PlannedReplayWrite::ModuleJsonSet {
                    namespace: write.namespace().as_str().to_owned(),
                    key: write.key().as_str().to_owned(),
                    doc: serde_json::to_string(write.chunk())?,
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

/// The first planned write whose post-image is not what the store now holds,
/// or `None` when every one of them reads back exactly.
///
/// The fake stores the exact `String` it was handed, so this can only ever
/// catch a genuinely clobbered target — never a backend renormalizing its own
/// serialization, which is the live server's failure mode, not this one's.
fn first_replay_mismatch<'a>(
    inner: &Inner,
    planned: &'a [PlannedReplayWrite],
) -> Option<&'a PlannedReplayWrite> {
    planned.iter().find(|write| match write {
        PlannedReplayWrite::ObjectSet { id, doc } => inner.objects.get(id) != Some(doc),
        PlannedReplayWrite::ObjectDelete { id } => inner.objects.contains_key(id),
        PlannedReplayWrite::ObjectAllocatorSet { next_id } => inner.next_id != Some(*next_id),
        PlannedReplayWrite::ModuleJsonSet {
            namespace,
            key,
            doc,
        } => inner.module_kv.get(&(namespace.clone(), key.clone())) != Some(doc),
        PlannedReplayWrite::ModuleDelete { namespace, key } => {
            inner
                .module_kv
                .contains_key(&(namespace.clone(), key.clone()))
                || inner
                    .module_streams
                    .contains_key(&(namespace.clone(), key.clone()))
        }
    })
}

/// The Redis key a planned write addresses, so a readback refusal names the
/// same target on the fake as it does on the live server.
fn planned_write_key(write: &PlannedReplayWrite) -> String {
    match write {
        PlannedReplayWrite::ObjectSet { id, .. } | PlannedReplayWrite::ObjectDelete { id } => {
            format!("{OBJ_PREFIX}{id}")
        }
        PlannedReplayWrite::ObjectAllocatorSet { .. } => META_NEXT_ID.to_owned(),
        PlannedReplayWrite::ModuleJsonSet { namespace, key, .. }
        | PlannedReplayWrite::ModuleDelete { namespace, key } => {
            format!("{MOD_PREFIX}{namespace}:{key}")
        }
    }
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

fn advance_retention_generation(inner: &mut Inner) -> Result<(), StoreError> {
    inner.authoritative_retention_generation = inner
        .authoritative_retention_generation
        .checked_add(1)
        .ok_or_else(|| {
            StoreError::AuthoritativeCorrupt(
                "authoritative retention generation exhausted".to_owned(),
            )
        })?;
    Ok(())
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
            let minimum_accepted_epoch = inner
                .authoritative_minimum_epochs
                .get(key.scope())
                .copied()
                .unwrap_or(0);
            if key.epoch() < minimum_accepted_epoch {
                return Err(StoreError::AuthoritativeEpochRetired {
                    requested_epoch: key.epoch(),
                    minimum_accepted_epoch,
                });
            }

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
                let finalized = if let Some(receipt_doc) = inner.authoritative_receipts.get(&token)
                {
                    let receipt = decode_receipt(receipt_doc)?;
                    if !receipt.matches_bundle(&bundle) {
                        return Err(StoreError::AuthoritativeCorrupt(
                            "finalized receipt disagrees with its retained intent".to_owned(),
                        ));
                    }
                    true
                } else {
                    false
                };
                if finalized {
                    inner.authoritative_pending.remove(&token);
                } else {
                    inner.authoritative_pending.insert(token);
                }
                return Ok(IntentReceipt {
                    token,
                    bundle_hash,
                    resumed: true,
                });
            }
            if let Some(quarantine_doc) = inner.authoritative_quarantines.get(&token) {
                let quarantine = decode_quarantine_receipt(quarantine_doc)?;
                validate_quarantine_binding(&inner, token, &quarantine)?;
                if !quarantine.matches_bundle(&bundle) {
                    return Err(StoreError::AuthoritativeTokenConflict);
                }
                inner.authoritative_pending.remove(&token);
                return Err(StoreError::AuthoritativeIntentQuarantined {
                    evidence_hash: quarantine.evidence_hash(),
                });
            }
            if let Some(receipt_doc) = inner.authoritative_receipts.get(&token) {
                let receipt = decode_checkpoint_receipt(receipt_doc)?;
                validate_receipt_binding(&inner, token, &receipt)?;
                if !receipt.matches_bundle(&bundle) {
                    return Err(StoreError::AuthoritativeTokenConflict);
                }
                inner.authoritative_pending.remove(&token);
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
            if !inner.authoritative_pending.is_empty() {
                return Err(StoreError::AuthoritativeIntentInFlight {
                    pending: inner.authoritative_pending.len(),
                });
            }
            validate_replay_admission(&inner, &bundle)?;
            enforce_retained_authoritative_capacity(
                &inner,
                doc.len(),
                MAX_RETAINED_AUTHORITATIVE_INTENTS,
                MAX_RETAINED_AUTHORITATIVE_BYTES,
            )?;
            advance_retention_generation(&mut inner)?;
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
            let minimum_accepted_epoch = inner
                .authoritative_minimum_epochs
                .get(key.scope())
                .copied()
                .unwrap_or(0);
            if key.epoch() < minimum_accepted_epoch {
                return Ok(IdempotencyStatus::Retired {
                    minimum_accepted_epoch,
                });
            }
            let Some(binding) = inner.authoritative_idempotency.get(key) else {
                return Ok(IdempotencyStatus::Vacant);
            };
            if binding.ingress_hash != ingress_hash {
                return Ok(IdempotencyStatus::Conflict);
            }
            let Some(intent_doc) = inner.authoritative_intents.get(&binding.token) else {
                if let Some(receipt_doc) = inner.authoritative_receipts.get(&binding.token) {
                    let receipt = decode_checkpoint_receipt(receipt_doc)?;
                    validate_receipt_binding(&inner, binding.token, &receipt)?;
                    if receipt.idempotency() != key {
                        return Err(StoreError::AuthoritativeCorrupt(
                            "checkpoint receipt disagrees with its idempotency address".to_owned(),
                        ));
                    }
                    return Ok(IdempotencyStatus::FinalizedExact(Box::new(receipt)));
                }
                let quarantine_doc = inner
                    .authoritative_quarantines
                    .get(&binding.token)
                    .ok_or_else(|| {
                        StoreError::AuthoritativeCorrupt(
                            "idempotency binding names no intent, receipt, or quarantine"
                                .to_owned(),
                        )
                    })?;
                let quarantine = decode_quarantine_receipt(quarantine_doc)?;
                validate_quarantine_binding(&inner, binding.token, &quarantine)?;
                if quarantine.idempotency() != key {
                    return Err(StoreError::AuthoritativeCorrupt(
                        "quarantine receipt disagrees with its idempotency address".to_owned(),
                    ));
                }
                return Ok(IdempotencyStatus::QuarantinedExact(Box::new(quarantine)));
            };
            let intent = decode_intent(intent_doc)?;
            if intent.idempotency() != key {
                return Err(StoreError::AuthoritativeCorrupt(
                    "pending intent disagrees with its idempotency binding".to_owned(),
                ));
            }
            let receipt = validate_retained_intent(&inner, binding.token, &intent)?;
            if let Some(receipt) = receipt {
                return Ok(IdempotencyStatus::FinalizedExact(Box::new(receipt)));
            }
            Ok(IdempotencyStatus::PendingExact {
                token: binding.token,
                bundle_hash: binding.bundle_hash,
            })
        })
    }

    fn bound_authoritative_idempotency_status<'a>(
        &'a self,
        key: &'a ScopedIdempotencyKey,
    ) -> BoxFuture<'a, Result<BoundIdempotencyStatus, StoreError>> {
        Box::pin(async move {
            let inner = self.lock()?;
            let minimum_accepted_epoch = inner
                .authoritative_minimum_epochs
                .get(key.scope())
                .copied()
                .unwrap_or(0);
            if key.epoch() < minimum_accepted_epoch {
                return Ok(BoundIdempotencyStatus::Retired {
                    minimum_accepted_epoch,
                });
            }
            let Some(binding) = inner.authoritative_idempotency.get(key) else {
                return Ok(BoundIdempotencyStatus::Vacant);
            };
            let Some(intent_doc) = inner.authoritative_intents.get(&binding.token) else {
                if let Some(receipt_doc) = inner.authoritative_receipts.get(&binding.token) {
                    let receipt = decode_checkpoint_receipt(receipt_doc)?;
                    validate_receipt_binding(&inner, binding.token, &receipt)?;
                    if receipt.idempotency() != key {
                        return Err(StoreError::AuthoritativeCorrupt(
                            "checkpoint receipt disagrees with its idempotency address".to_owned(),
                        ));
                    }
                    return Ok(BoundIdempotencyStatus::FinalizedExact(Box::new(receipt)));
                }
                let quarantine_doc = inner
                    .authoritative_quarantines
                    .get(&binding.token)
                    .ok_or_else(|| {
                        StoreError::AuthoritativeCorrupt(
                            "idempotency binding names no intent, receipt, or quarantine"
                                .to_owned(),
                        )
                    })?;
                let quarantine = decode_quarantine_receipt(quarantine_doc)?;
                validate_quarantine_binding(&inner, binding.token, &quarantine)?;
                if quarantine.idempotency() != key {
                    return Err(StoreError::AuthoritativeCorrupt(
                        "quarantine receipt disagrees with its idempotency address".to_owned(),
                    ));
                }
                return Ok(BoundIdempotencyStatus::QuarantinedExact(Box::new(
                    quarantine,
                )));
            };
            let intent = decode_intent(intent_doc)?;
            if intent.idempotency() != key {
                return Err(StoreError::AuthoritativeCorrupt(
                    "pending intent disagrees with its idempotency binding".to_owned(),
                ));
            }
            let receipt = validate_retained_intent(&inner, binding.token, &intent)?;
            if let Some(receipt) = receipt {
                return Ok(BoundIdempotencyStatus::FinalizedExact(Box::new(receipt)));
            }
            Ok(BoundIdempotencyStatus::PendingExact {
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
                let mut inner = self.lock()?;
                let Some(intent_doc) = inner.authoritative_intents.get(&token).cloned() else {
                    let receipt_doc = inner
                        .authoritative_receipts
                        .get(&token)
                        .ok_or(StoreError::AuthoritativeIntentMissing)?;
                    let receipt = decode_checkpoint_receipt(receipt_doc)?;
                    validate_receipt_binding(&inner, token, &receipt)?;
                    if receipt.bundle_hash() != expected_bundle_hash {
                        return Err(StoreError::AuthoritativeTokenConflict);
                    }
                    return Ok(receipt);
                };
                let intent = decode_intent(&intent_doc)?;
                if let Some(receipt) = validate_retained_intent(&inner, token, &intent)? {
                    if receipt.bundle_hash() != expected_bundle_hash {
                        return Err(StoreError::AuthoritativeTokenConflict);
                    }
                    inner.authoritative_pending.remove(&token);
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
            let receipt = AuthoritativeReceipt::new(&bundle);
            let receipt_doc = serde_json::to_string(&receipt)?;
            let outbox_records = expected_outbox_records(&bundle)?
                .into_iter()
                .map(|record| Ok((record.item_id(), serde_json::to_string(&record)?)))
                .collect::<Result<Vec<_>, StoreError>>()?;

            let mut inner = self.lock()?;
            if inner.authoritative_intents.get(&token) != Some(&intent_doc) {
                return Err(StoreError::AuthoritativeTokenConflict);
            }
            if let Some(existing) = validate_retained_intent(&inner, token, &bundle)? {
                if existing != receipt {
                    return Err(StoreError::AuthoritativeTokenConflict);
                }
                inner.authoritative_pending.remove(&token);
                return Ok(existing);
            }
            let Some(binding) = inner.authoritative_idempotency.get(bundle.idempotency()) else {
                return Err(StoreError::AuthoritativeCorrupt(
                    "authoritative intent is missing its idempotency binding".to_owned(),
                ));
            };
            if binding.token != token
                || binding.ingress_hash != bundle.ingress_hash()
                || binding.bundle_hash != expected_bundle_hash
            {
                return Err(StoreError::AuthoritativeCorrupt(
                    "authoritative intent disagrees with its idempotency binding".to_owned(),
                ));
            }
            validate_replay_recovery(&inner, &bundle)?;
            let new_outbox = outbox_records
                .iter()
                .filter(|(item_id, _)| !inner.authoritative_outbox.contains_key(item_id))
                .count();
            let attempted_outbox = inner
                .authoritative_outbox_pending
                .len()
                .checked_add(new_outbox)
                .ok_or_else(|| {
                    StoreError::AuthoritativeCorrupt(
                        "durable outbox item count overflowed".to_owned(),
                    )
                })?;
            if attempted_outbox > MAX_PENDING_AUTHORITATIVE_OUTBOX {
                return Err(StoreError::AuthoritativeOutboxCapacity {
                    pending: attempted_outbox,
                    max_pending: MAX_PENDING_AUTHORITATIVE_OUTBOX,
                });
            }
            for (item_id, doc) in &outbox_records {
                if inner
                    .authoritative_outbox
                    .get(item_id)
                    .is_some_and(|existing| existing != doc)
                {
                    return Err(StoreError::AuthoritativeOutboxConflict);
                }
            }
            apply_replay(&mut inner, &planned);
            if let Some(write) = first_replay_mismatch(&inner, &planned) {
                return Err(StoreError::AuthoritativeReadbackMismatch {
                    key: planned_write_key(write),
                    phase: AuthoritativeReadbackPhase::Finalize,
                });
            }
            for (item_id, doc) in outbox_records {
                inner.authoritative_outbox.insert(item_id, doc);
                inner.authoritative_outbox_pending.insert(item_id);
            }
            inner.authoritative_receipts.insert(token, receipt_doc);
            inner.authoritative_pending.remove(&token);
            Ok(receipt)
        })
    }

    fn checkpoint_authoritative(
        &self,
        token: CommitToken,
        expected_bundle_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<AuthoritativeCheckpointReceipt, StoreError>> {
        Box::pin(async move {
            let mut inner = self.lock()?;
            let Some(intent_doc) = inner.authoritative_intents.get(&token).cloned() else {
                let receipt_doc = inner
                    .authoritative_receipts
                    .get(&token)
                    .ok_or(StoreError::AuthoritativeIntentMissing)?;
                let receipt = decode_checkpoint_receipt(receipt_doc)?;
                validate_receipt_binding(&inner, token, &receipt)?;
                if receipt.bundle_hash() != expected_bundle_hash {
                    return Err(StoreError::AuthoritativeTokenConflict);
                }
                return Ok(AuthoritativeCheckpointReceipt {
                    receipt,
                    released_intent_bytes: 0,
                    resumed: true,
                });
            };
            let bundle = decode_intent(&intent_doc)?;
            if bundle.bundle_hash() != expected_bundle_hash {
                return Err(StoreError::AuthoritativeTokenConflict);
            }
            let receipt = validate_retained_intent(&inner, token, &bundle)?
                .ok_or(StoreError::AuthoritativeNotFinalized)?;
            let planned = plan_replay(&bundle)?;
            if let Some(write) = first_replay_mismatch(&inner, &planned) {
                return Err(StoreError::AuthoritativeReadbackMismatch {
                    key: planned_write_key(write),
                    phase: AuthoritativeReadbackPhase::Checkpoint,
                });
            }
            for expected in expected_outbox_records(&bundle)? {
                let record = decode_outbox_record(
                    inner
                        .authoritative_outbox
                        .get(&expected.item_id())
                        .ok_or_else(|| {
                            StoreError::AuthoritativeCorrupt(
                                "finalized action is missing a durable outbox record".to_owned(),
                            )
                        })?,
                )?;
                if !record.state().is_terminal() {
                    return Err(StoreError::AuthoritativeOutboxPending);
                }
            }
            validate_receipt_binding(&inner, token, &receipt)?;
            let checkpoint_doc = serde_json::to_string(&receipt)?;
            advance_retention_generation(&mut inner)?;
            inner.authoritative_receipts.insert(token, checkpoint_doc);
            inner.authoritative_intents.remove(&token);
            inner.authoritative_pending.remove(&token);
            Ok(AuthoritativeCheckpointReceipt {
                receipt,
                released_intent_bytes: intent_doc.len(),
                resumed: false,
            })
        })
    }

    fn discard_quarantined_authoritative_intent(
        &self,
        token: CommitToken,
        expected_bundle_hash: CanonicalHash,
        evidence_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<AuthoritativeDiscardReceipt, StoreError>> {
        Box::pin(async move {
            let mut inner = self.lock()?;
            if let Some(doc) = inner.authoritative_quarantines.get(&token) {
                let quarantine = decode_quarantine_receipt(doc)?;
                validate_quarantine_binding(&inner, token, &quarantine)?;
                if quarantine.bundle_hash() != expected_bundle_hash
                    || quarantine.evidence_hash() != evidence_hash
                {
                    return Err(StoreError::AuthoritativeTokenConflict);
                }
                inner.authoritative_pending.remove(&token);
                return Ok(AuthoritativeDiscardReceipt {
                    quarantine,
                    released_intent_bytes: 0,
                    resumed: true,
                });
            }
            let intent_doc = inner
                .authoritative_intents
                .get(&token)
                .cloned()
                .ok_or(StoreError::AuthoritativeIntentMissing)?;
            let bundle = decode_intent(&intent_doc)?;
            if bundle.bundle_hash() != expected_bundle_hash {
                return Err(StoreError::AuthoritativeTokenConflict);
            }
            if inner.authoritative_receipts.contains_key(&token) {
                return Err(StoreError::AuthoritativeIntentAlreadyFinalized);
            }
            let Some(binding) = inner.authoritative_idempotency.get(bundle.idempotency()) else {
                return Err(StoreError::AuthoritativeCorrupt(
                    "discarded intent is missing its idempotency binding".to_owned(),
                ));
            };
            if binding.token != token
                || binding.ingress_hash != bundle.ingress_hash()
                || binding.bundle_hash != expected_bundle_hash
            {
                return Err(StoreError::AuthoritativeCorrupt(
                    "discarded intent disagrees with its idempotency binding".to_owned(),
                ));
            }
            let quarantine = AuthoritativeQuarantineReceipt::new(&bundle, evidence_hash);
            let quarantine_doc = serde_json::to_string(&quarantine)?;
            advance_retention_generation(&mut inner)?;
            inner
                .authoritative_quarantines
                .insert(token, quarantine_doc);
            inner.authoritative_intents.remove(&token);
            inner.authoritative_pending.remove(&token);
            Ok(AuthoritativeDiscardReceipt {
                quarantine,
                released_intent_bytes: intent_doc.len(),
                resumed: false,
            })
        })
    }

    fn restore_quarantined_authoritative_intent(
        &self,
        bundle: AuthoritativeBundle,
        evidence_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<IntentReceipt, StoreError>> {
        Box::pin(async move {
            let token = bundle.token();
            let bundle_hash = bundle.bundle_hash();
            let doc = serde_json::to_string(&bundle)?;
            let mut inner = self.lock()?;
            if let Some(intent_doc) = inner.authoritative_intents.get(&token) {
                let existing = decode_intent(intent_doc)?;
                if existing != bundle {
                    return Err(StoreError::AuthoritativeTokenConflict);
                }
                if inner.authoritative_quarantines.contains_key(&token) {
                    return Err(StoreError::AuthoritativeCorrupt(
                        "authoritative intent and quarantine receipt coexist".to_owned(),
                    ));
                }
                return Ok(IntentReceipt {
                    token,
                    bundle_hash,
                    resumed: true,
                });
            }
            let quarantine_doc = inner
                .authoritative_quarantines
                .get(&token)
                .ok_or(StoreError::AuthoritativeIntentMissing)?;
            let quarantine = decode_quarantine_receipt(quarantine_doc)?;
            validate_quarantine_binding(&inner, token, &quarantine)?;
            if !quarantine.matches_bundle(&bundle) || quarantine.evidence_hash() != evidence_hash {
                return Err(StoreError::AuthoritativeTokenConflict);
            }
            if inner.authoritative_receipts.contains_key(&token) {
                return Err(StoreError::AuthoritativeCorrupt(
                    "quarantine receipt coexists with a finalized witness".to_owned(),
                ));
            }
            if !inner.authoritative_pending.is_empty() {
                return Err(StoreError::AuthoritativeIntentInFlight {
                    pending: inner.authoritative_pending.len(),
                });
            }
            enforce_retained_authoritative_capacity(
                &inner,
                doc.len(),
                MAX_RETAINED_AUTHORITATIVE_INTENTS,
                MAX_RETAINED_AUTHORITATIVE_BYTES,
            )?;
            advance_retention_generation(&mut inner)?;
            inner.authoritative_intents.insert(token, doc);
            inner.authoritative_quarantines.remove(&token);
            inner.authoritative_pending.insert(token);
            Ok(IntentReceipt {
                token,
                bundle_hash,
                resumed: false,
            })
        })
    }

    fn pending_authoritative_outbox(
        &self,
    ) -> BoxFuture<'_, Result<Vec<AuthoritativeOutboxRecord>, StoreError>> {
        Box::pin(async move {
            let inner = self.lock()?;
            if inner.authoritative_outbox_pending.len() > MAX_PENDING_AUTHORITATIVE_OUTBOX {
                return Err(StoreError::AuthoritativeCorrupt(format!(
                    "durable outbox pending index contains {} items; maximum is {MAX_PENDING_AUTHORITATIVE_OUTBOX}",
                    inner.authoritative_outbox_pending.len()
                )));
            }
            let mut records = Vec::with_capacity(inner.authoritative_outbox_pending.len());
            for item_id in &inner.authoritative_outbox_pending {
                let doc = inner.authoritative_outbox.get(item_id).ok_or_else(|| {
                    StoreError::AuthoritativeCorrupt(
                        "durable outbox pending index names a missing item".to_owned(),
                    )
                })?;
                let record = decode_outbox_record(doc)?;
                if record.item_id() != *item_id || record.state().is_terminal() {
                    return Err(StoreError::AuthoritativeCorrupt(
                        "durable outbox pending index disagrees with item state".to_owned(),
                    ));
                }
                let receipt = inner
                    .authoritative_receipts
                    .get(&record.parent_token())
                    .ok_or_else(|| {
                        StoreError::AuthoritativeCorrupt(
                            "durable outbox item has no finalized parent".to_owned(),
                        )
                    })
                    .and_then(|doc| decode_receipt(doc))?;
                if receipt.bundle_hash() != record.parent_bundle_hash() {
                    return Err(StoreError::AuthoritativeCorrupt(
                        "durable outbox item disagrees with its finalized parent".to_owned(),
                    ));
                }
                records.push(record);
            }
            Ok(records)
        })
    }

    fn begin_authoritative_outbox_attempt(
        &self,
        item_id: CanonicalHash,
        expected_payload_hash: CanonicalHash,
        expected_attempts: u32,
    ) -> BoxFuture<'_, Result<AuthoritativeOutboxRecord, StoreError>> {
        Box::pin(async move {
            let mut inner = self.lock()?;
            let doc = inner
                .authoritative_outbox
                .get(&item_id)
                .ok_or(StoreError::AuthoritativeOutboxMissing)?;
            let record = decode_outbox_record(doc)?;
            if record.intent().payload_hash() != expected_payload_hash
                || record.state().is_terminal()
            {
                return Err(StoreError::AuthoritativeOutboxConflict);
            }
            validate_outbox_parent(&inner, &record)?;
            if expected_attempts < u32::MAX && record.attempts() == expected_attempts + 1 {
                return Ok(record);
            }
            let next = record.start_attempt(expected_attempts)?;
            inner
                .authoritative_outbox
                .insert(item_id, serde_json::to_string(&next)?);
            inner.authoritative_outbox_pending.insert(item_id);
            Ok(next)
        })
    }

    fn complete_authoritative_outbox_attempt(
        &self,
        item_id: CanonicalHash,
        expected_payload_hash: CanonicalHash,
        expected_attempt: u32,
        result: AuthoritativeOutboxAttemptResult,
    ) -> BoxFuture<'_, Result<AuthoritativeOutboxRecord, StoreError>> {
        Box::pin(async move {
            let mut inner = self.lock()?;
            let doc = inner
                .authoritative_outbox
                .get(&item_id)
                .ok_or(StoreError::AuthoritativeOutboxMissing)?;
            let record = decode_outbox_record(doc)?;
            if record.intent().payload_hash() != expected_payload_hash {
                return Err(StoreError::AuthoritativeOutboxConflict);
            }
            validate_outbox_parent(&inner, &record)?;
            let terminal_retry = matches!(
                (record.state(), &result),
                (
                    crate::types::AuthoritativeOutboxState::Delivered,
                    AuthoritativeOutboxAttemptResult::Delivered
                )
            ) || matches!(
                (record.state(), &result),
                (
                    crate::types::AuthoritativeOutboxState::DeadLetter { error: stored },
                    AuthoritativeOutboxAttemptResult::DeadLetter { error }
                ) if stored == error
            );
            if terminal_retry && record.attempts() == expected_attempt {
                return Ok(record);
            }
            let next = record.complete_attempt(expected_attempt, result)?;
            inner
                .authoritative_outbox
                .insert(item_id, serde_json::to_string(&next)?);
            if next.state().is_terminal() {
                inner.authoritative_outbox_pending.remove(&item_id);
            } else {
                inner.authoritative_outbox_pending.insert(item_id);
            }
            Ok(next)
        })
    }

    fn note_authoritative_outbox_unroutable(
        &self,
        item_id: CanonicalHash,
        expected_payload_hash: CanonicalHash,
        expected_passes: u32,
        error: String,
    ) -> BoxFuture<'_, Result<AuthoritativeOutboxRecord, StoreError>> {
        Box::pin(async move {
            let mut inner = self.lock()?;
            let doc = inner
                .authoritative_outbox
                .get(&item_id)
                .ok_or(StoreError::AuthoritativeOutboxMissing)?;
            let record = decode_outbox_record(doc)?;
            if record.intent().payload_hash() != expected_payload_hash {
                return Err(StoreError::AuthoritativeOutboxConflict);
            }
            validate_outbox_parent(&inner, &record)?;
            if expected_passes < u32::MAX
                && record.unroutable_passes() == expected_passes + 1
                && matches!(
                    record.state(),
                    crate::types::AuthoritativeOutboxState::Pending {
                        last_error: Some(stored)
                    } if stored == &error
                )
            {
                return Ok(record);
            }
            let next = record.note_unroutable(expected_passes, error)?;
            inner
                .authoritative_outbox
                .insert(item_id, serde_json::to_string(&next)?);
            inner.authoritative_outbox_pending.insert(item_id);
            Ok(next)
        })
    }

    fn retire_authoritative_epochs<'a>(
        &'a self,
        scope: &'a IdempotencyScope,
        minimum_accepted_epoch: u64,
    ) -> BoxFuture<'a, Result<AuthoritativeEpochRetirement, StoreError>> {
        Box::pin(async move {
            let mut inner = self.lock()?;
            let previous_minimum_epoch = inner
                .authoritative_minimum_epochs
                .get(scope)
                .copied()
                .unwrap_or(0);
            if minimum_accepted_epoch < previous_minimum_epoch {
                return Err(StoreError::AuthoritativeEpochRegression {
                    current_minimum_epoch: previous_minimum_epoch,
                    requested_minimum_epoch: minimum_accepted_epoch,
                });
            }

            let mut retained_intents = 0_usize;
            for intent_doc in inner.authoritative_intents.values() {
                let bundle = decode_intent(intent_doc)?;
                if bundle.idempotency().scope() == scope
                    && bundle.idempotency().epoch() < minimum_accepted_epoch
                {
                    retained_intents = retained_intents.checked_add(1).ok_or_else(|| {
                        StoreError::AuthoritativeCorrupt(
                            "retained epoch intent count overflowed".to_owned(),
                        )
                    })?;
                }
            }
            if retained_intents != 0 {
                return Err(StoreError::AuthoritativeEpochPending {
                    requested_minimum_epoch: minimum_accepted_epoch,
                    retained_intents,
                });
            }

            // Validate every candidate and its exact binding before advancing
            // the watermark or deleting one byte: the in-memory backend keeps
            // the complete retirement atomic under this lock.
            let mut retired = Vec::new();
            for (token, receipt_doc) in &inner.authoritative_receipts {
                let receipt = decode_checkpoint_receipt(receipt_doc)?;
                if receipt.idempotency().scope() == scope
                    && receipt.idempotency().epoch() < minimum_accepted_epoch
                {
                    validate_receipt_binding(&inner, *token, &receipt)?;
                    retired.push((*token, receipt.idempotency().clone(), receipt.bundle_hash()));
                }
            }

            let retired_parents: BTreeMap<_, _> = retired
                .iter()
                .map(|(token, _, bundle_hash)| (*token, *bundle_hash))
                .collect();
            let mut retired_outbox = Vec::new();
            for (item_id, doc) in &inner.authoritative_outbox {
                let record = decode_outbox_record(doc)?;
                let Some(expected_bundle_hash) = retired_parents.get(&record.parent_token()) else {
                    continue;
                };
                if record.parent_bundle_hash() != *expected_bundle_hash
                    || !record.state().is_terminal()
                {
                    return Err(StoreError::AuthoritativeCorrupt(
                        "durable outbox terminal record disagrees with its retired parent"
                            .to_owned(),
                    ));
                }
                retired_outbox.push(*item_id);
            }

            inner
                .authoritative_minimum_epochs
                .insert(scope.clone(), minimum_accepted_epoch);
            for (token, idempotency, _) in &retired {
                inner.authoritative_receipts.remove(token);
                inner.authoritative_idempotency.remove(idempotency);
            }
            for item_id in retired_outbox {
                inner.authoritative_outbox.remove(&item_id);
                inner.authoritative_outbox_pending.remove(&item_id);
            }
            Ok(AuthoritativeEpochRetirement {
                scope: scope.clone(),
                previous_minimum_epoch,
                minimum_accepted_epoch,
                retired_tombstones: retired.len(),
                resumed: previous_minimum_epoch == minimum_accepted_epoch,
            })
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
                let receipt = decode_checkpoint_receipt(
                    inner
                        .authoritative_receipts
                        .get(&token)
                        .expect("receipt presence checked"),
                )?;
                validate_receipt_binding(&inner, token, &receipt)?;
                if receipt.bundle_hash() != expected_bundle_hash {
                    return Ok(BundleStatus::Conflict {
                        stored_bundle_hash: receipt.bundle_hash(),
                    });
                }
                return Ok(BundleStatus::FinalizedExact(Box::new(receipt)));
            }
            if let Some(quarantine_doc) = inner.authoritative_quarantines.get(&token) {
                let quarantine = decode_quarantine_receipt(quarantine_doc)?;
                validate_quarantine_binding(&inner, token, &quarantine)?;
                if quarantine.bundle_hash() != expected_bundle_hash {
                    return Ok(BundleStatus::Conflict {
                        stored_bundle_hash: quarantine.bundle_hash(),
                    });
                }
                return Ok(BundleStatus::QuarantinedExact(Box::new(quarantine)));
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

    fn authoritative_retention_generation(&self) -> BoxFuture<'_, Result<u64, StoreError>> {
        Box::pin(async move { Ok(self.lock()?.authoritative_retention_generation) })
    }

    fn repair_authoritative_retention(
        &self,
        expected_generation: u64,
        usage: AuthoritativeRetentionUsage,
    ) -> BoxFuture<'_, Result<bool, StoreError>> {
        Box::pin(async move {
            let inner = self.lock()?;
            if inner.authoritative_retention_generation != expected_generation {
                return Err(StoreError::AuthoritativeRetentionContention);
            }
            let (count, bytes) = retained_authoritative_usage(&inner)?;
            let actual = AuthoritativeRetentionUsage { count, bytes };
            if actual != usage {
                return Err(StoreError::AuthoritativeCorrupt(format!(
                    "authoritative retention scan disagrees with in-memory storage: expected {usage:?}, actual {actual:?}"
                )));
            }
            Ok(false)
        })
    }
}

impl ObjectStore for InMemoryStore {
    fn authoritative_store(&self) -> Option<&dyn AuthoritativeStore> {
        Some(self)
    }

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
                let doc = serde_json::to_string(obj)?;
                crate::types::refuse_over_deep_object_doc(obj.id, &doc)?;
                docs.push((obj.id.0, doc));
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

    fn set_account_discord<'a>(
        &'a self,
        account: &'a str,
        link: Option<&'a DiscordLinkRec>,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            let key = account.to_lowercase();
            let mut inner = self.lock()?;
            let Some(doc) = inner.accounts.get(&key) else {
                return Ok(false);
            };
            // Round-trip like ssh_keys: a doc that predates `discord` is
            // written back with the field.
            let mut rec: AccountRec = serde_json::from_str(doc)?;
            rec.discord = link.cloned();
            let doc = serde_json::to_string(&rec)?;
            inner.accounts.insert(key, doc);
            Ok(true)
        })
    }

    fn discord_user_reserve<'a>(
        &'a self,
        user_id: u64,
        account: &'a str,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            let account_lc = account.to_lowercase();
            let mut inner = self.lock()?;
            match inner.discord_users.get(&user_id) {
                Some(holder) => Ok(*holder == account_lc),
                None => {
                    inner.discord_users.insert(user_id, account_lc);
                    Ok(true)
                }
            }
        })
    }

    fn discord_user_release(&self, user_id: u64) -> BoxFuture<'_, Result<bool, StoreError>> {
        Box::pin(async move {
            let mut inner = self.lock()?;
            Ok(inner.discord_users.remove(&user_id).is_some())
        })
    }

    fn account_by_discord_user(
        &self,
        user_id: u64,
    ) -> BoxFuture<'_, Result<Option<AccountRec>, StoreError>> {
        Box::pin(async move {
            let inner = self.lock()?;
            let Some(name) = inner.discord_users.get(&user_id) else {
                return Ok(None);
            };
            let Some(doc) = inner.accounts.get(name) else {
                return Ok(None);
            };
            let rec: AccountRec = serde_json::from_str(doc)?;
            // A reverse entry whose account carries no (or another user's)
            // link is a stale reservation, not a link.
            Ok(rec
                .discord
                .as_ref()
                .is_some_and(|l| l.user_id == user_id)
                .then_some(rec))
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

    fn discord_code_put(
        &self,
        hash: String,
        rec: DiscordCodeRecord,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            validate_token_hash(&hash)?;
            let doc = serde_json::to_string(&rec)?;
            let mut inner = self.lock()?;
            if rec.expires <= now_unix() {
                // Mirrors Redis: a key set with EXAT in the past is gone.
                inner.discord_codes.remove(&hash);
                return Ok(());
            }
            inner.discord_codes.insert(hash, doc);
            Ok(())
        })
    }

    fn discord_code_take<'a>(
        &'a self,
        hash: &'a str,
    ) -> BoxFuture<'a, Result<Option<DiscordCodeRecord>, StoreError>> {
        Box::pin(async move {
            validate_token_hash(hash)?;
            let mut inner = self.lock()?;
            // Removed on the way out whether live or not: a code is consumed
            // by the attempt (Redis: GETDEL).
            let Some(doc) = inner.discord_codes.remove(hash) else {
                return Ok(None);
            };
            let rec: DiscordCodeRecord = serde_json::from_str(&doc)?;
            if rec.expires <= now_unix() {
                return Ok(None);
            }
            Ok(Some(rec))
        })
    }

    fn discord_ban_put(&self, rec: DiscordBanRec) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            let doc = serde_json::to_string(&rec)?;
            let mut inner = self.lock()?;
            inner.discord_bans.insert(rec.user_id, doc);
            Ok(())
        })
    }

    fn discord_ban_get(
        &self,
        user_id: u64,
    ) -> BoxFuture<'_, Result<Option<DiscordBanRec>, StoreError>> {
        Box::pin(async move {
            let inner = self.lock()?;
            match inner.discord_bans.get(&user_id) {
                Some(doc) => Ok(Some(serde_json::from_str(doc)?)),
                None => Ok(None),
            }
        })
    }

    fn discord_ban_del(&self, user_id: u64) -> BoxFuture<'_, Result<bool, StoreError>> {
        Box::pin(async move {
            let mut inner = self.lock()?;
            Ok(inner.discord_bans.remove(&user_id).is_some())
        })
    }

    fn module_json_get<'a>(
        &'a self,
        ns: &'a str,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Option<serde_json::Value>, StoreError>> {
        Box::pin(async move {
            match self.module_doc_text(ns, key).await? {
                Some(text) => Ok(Some(serde_json::from_str(&text)?)),
                None => Ok(None),
            }
        })
    }

    fn module_json_list<'a>(
        &'a self,
        ns: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, serde_json::Value)>, StoreError>> {
        Box::pin(async move { self.module_docs(ns).await?.into_json_list() })
    }

    fn module_docs<'a>(&'a self, ns: &'a str) -> BoxFuture<'a, Result<ModuleDocs, StoreError>> {
        self.module_docs_from(ns, &crate::store::NO_SKIP)
    }

    fn module_docs_from<'a>(
        &'a self,
        ns: &'a str,
        skip: &'a crate::store::BootSkip,
    ) -> BoxFuture<'a, Result<ModuleDocs, StoreError>> {
        Box::pin(async move {
            validate_module_ns(ns)?;
            let mut out = ModuleDocs::default();
            {
                let inner = self.lock()?;
                for ((doc_ns, key), doc) in &inner.module_kv {
                    if doc_ns != ns {
                        continue;
                    }
                    // J4 / W1: below the floor or under a skipped prefix, not even read.
                    if skip.skips(key) {
                        continue;
                    }
                    // J1: the store holds text; a ledger chunk is handed over as is.
                    if key.starts_with(mush_core::ledger::LOG_PREFIX) {
                        out.ledger.push((key.clone(), doc.clone()));
                    } else {
                        out.docs.push((key.clone(), serde_json::from_str(doc)?));
                    }
                }
            }
            // J5: what the primary has been purged of comes from the archive.
            crate::archive::merge_archived(self.ledger_archive(), ns, &mut out.ledger, skip)?;
            out.omitted = skip.floors.clone();
            out.skipped_prefixes = skip.prefixes.clone();
            Ok(out)
        })
    }

    fn module_keys<'a>(&'a self, ns: &'a str) -> BoxFuture<'a, Result<Vec<String>, StoreError>> {
        Box::pin(async move {
            validate_module_ns(ns)?;
            let inner = self.lock()?;
            Ok(inner
                .module_kv
                .keys()
                .filter(|(doc_ns, _)| doc_ns == ns)
                .map(|(_, key)| key.clone())
                .collect())
        })
    }

    fn module_doc_text<'a>(
        &'a self,
        ns: &'a str,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, StoreError>> {
        Box::pin(async move {
            validate_module_ns(ns)?;
            validate_module_key(key)?;
            let primary = {
                let inner = self.lock()?;
                inner
                    .module_kv
                    .get(&(ns.to_string(), key.to_string()))
                    .cloned()
            };
            crate::archive::tiered_text(self.ledger_archive(), ns, key, primary)
        })
    }

    fn module_purge_archived<'a>(
        &'a self,
        ns: &'a str,
        keys: Vec<String>,
    ) -> BoxFuture<'a, Result<usize, StoreError>> {
        Box::pin(async move {
            validate_module_ns(ns)?;
            let mut inner = self.lock()?;
            let mut removed = 0usize;
            for key in keys {
                validate_module_key(&key)?;
                if !key.starts_with(mush_core::ledger::LOG_PREFIX) {
                    return Err(StoreError::Invalid(format!(
                        "only ledger chunks may be purged, not {key:?}"
                    )));
                }
                if inner.module_kv.remove(&(ns.to_string(), key)).is_some() {
                    removed += 1;
                }
            }
            Ok(removed)
        })
    }

    fn ledger_archive(&self) -> Option<&crate::archive::LedgerArchive> {
        self.archive.as_deref()
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
        DefinitionId, DefinitionKind, DefinitionRegistryBuilder, DurableOutboxIntent,
        IdempotencyKey, IdempotencyScope, Ledger, OwnerId, PreparedCausalRef, PrincipalId,
        RequestUid, RulesValue, World,
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
        authoritative_bundle_for_epoch(AuthoritativeBundleFixture {
            token_byte,
            ingress_byte,
            request_byte,
            object_id,
            next_id,
            epoch: 0,
            durable_outbox: vec![],
            extra_writes: vec![],
        })
    }

    struct AuthoritativeBundleFixture {
        token_byte: u8,
        ingress_byte: u8,
        request_byte: u8,
        object_id: ObjId,
        next_id: Option<u32>,
        epoch: u64,
        durable_outbox: Vec<DurableOutboxIntent>,
        extra_writes: Vec<ReplayWrite>,
    }

    fn authoritative_bundle_for_epoch(spec: AuthoritativeBundleFixture) -> AuthoritativeBundle {
        let AuthoritativeBundleFixture {
            token_byte,
            ingress_byte,
            request_byte,
            object_id,
            next_id,
            epoch,
            durable_outbox,
            extra_writes,
        } = spec;
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
        writes.extend(extra_writes);
        AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: CommitToken::from_bytes([token_byte; 16]),
            request,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: principal.clone(),
                    owner: owner.clone(),
                    namespace: CanonicalId::new("brace").unwrap(),
                },
                epoch,
                IdempotencyKey::Request(request),
            ),
            ingress_hash: CanonicalHash::from_bytes([ingress_byte; 32]),
            request_hash: CanonicalHash::from_bytes([3; 32]),
            plan_hash: CanonicalHash::from_bytes([4; 32]),
            action: action.clone(),
            provider: owner.clone(),
            cause: PreparedCausalRef::new(request, None, 0).unwrap(),
            definitions: vec![action.clone(), outcome_definition.clone()],
            writes,
            outcome: RulesValue::new(
                outcome_definition.clone(),
                serde_json::json!({ "ok": true }),
            )
            .unwrap(),
            resolution: None,
            semantic_events: vec![
                mush_core::AuthoritativeSemanticEvent::root(
                    PreparedCausalRef::new(request, None, 0).unwrap(),
                    &action,
                    RulesValue::new(outcome_definition, serde_json::json!({ "ok": true })).unwrap(),
                    vec![principal.clone()],
                )
                .unwrap(),
            ],
            eligible_witnesses: vec![principal],
            durable_outbox,
        })
        .unwrap()
    }

    fn authoritative_ledger_bundle(token_byte: u8, ingress_byte: u8) -> AuthoritativeBundle {
        let ledger = Ledger::with_chunk_target(0);
        let prepared = ledger
            .prepare_append("economy", serde_json::json!({ "amount": 7 }))
            .expect("prepare sealed Ledger append");
        let replay = prepared
            .authoritative_replay_write(OwnerId::new("toy").unwrap())
            .expect("build Ledger replay write");
        authoritative_bundle_for_epoch(AuthoritativeBundleFixture {
            token_byte,
            ingress_byte,
            request_byte: 1,
            object_id: ObjId(2),
            next_id: Some(3),
            epoch: 0,
            durable_outbox: vec![],
            extra_writes: vec![replay],
        })
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
            let mut store = AuthoritativeFaultStore::new(mode);
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
            assert!(receipt.matches_bundle(&bundle), "fault mode {mode:?}");
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

    #[tokio::test]
    async fn authoritative_ledger_fault_matrix_repairs_before_and_after_seal_materialization()
    -> Result<(), StoreError> {
        for mode in [
            AuthoritativeFaultMode::BeforeIntent,
            AuthoritativeFaultMode::AfterIntent,
            AuthoritativeFaultMode::AfterNMaterializationOps(0),
            AuthoritativeFaultMode::AfterNMaterializationOps(1),
            AuthoritativeFaultMode::AfterNMaterializationOps(2),
            AuthoritativeFaultMode::AfterNMaterializationOps(3),
            AuthoritativeFaultMode::AfterNMaterializationOps(4),
            AuthoritativeFaultMode::AckLostAfterFinalize,
            AuthoritativeFaultMode::AckLostAfterCheckpoint,
        ] {
            let mut store = AuthoritativeFaultStore::new(mode);
            seed_minimal_world(&store.inner).await;
            let bundle = authoritative_ledger_bundle(71, 81);
            let token = bundle.token();
            let hash = bundle.bundle_hash();

            if store.reserve(bundle.clone()).await?.is_none() {
                store
                    .reserve(bundle.clone())
                    .await?
                    .expect("healed exact Ledger reservation");
            }
            if store.reconcile(token, hash).await?.is_none() {
                assert_eq!(
                    store.inner.authoritative_status(token, hash).await?,
                    if matches!(mode, AuthoritativeFaultMode::AckLostAfterFinalize) {
                        BundleStatus::FinalizedExact(Box::new(AuthoritativeReceipt::new(&bundle)))
                    } else {
                        BundleStatus::PendingExact
                    },
                    "fault mode {mode:?}"
                );
            }
            let receipt = store.inner.reconcile_authoritative(token, hash).await?;
            assert!(receipt.matches_bundle(&bundle), "fault mode {mode:?}");
            assert_eq!(
                store.inner.module_json_get("toy", "log:economy:0").await?,
                Some(serde_json::json!({
                    "chunk": 0,
                    "sealed": true,
                    "entries": [{ "seq": 0, "value": { "amount": 7 } }]
                })),
                "fault mode {mode:?}"
            );
            if store.checkpoint(token, hash).await?.is_none() {
                assert!(
                    store
                        .inner
                        .scan_authoritative_intents(None, 1)
                        .await?
                        .intents
                        .is_empty(),
                    "lost checkpoint acknowledgement still released exact intent"
                );
            }
            let checkpoint = store
                .checkpoint(token, hash)
                .await?
                .expect("exact checkpoint retry");
            assert_eq!(checkpoint.receipt, receipt, "fault mode {mode:?}");
            assert!(
                store
                    .inner
                    .reserve_authoritative_intent(bundle)
                    .await?
                    .resumed,
                "fault mode {mode:?}"
            );
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
    async fn authoritative_retention_usage_is_exact_and_noncanonical_intents_fail_closed() {
        let store = InMemoryStore::new();
        seed_minimal_world(&store).await;
        let bundle = authoritative_bundle(71, 72);
        store
            .reserve_authoritative_intent(bundle.clone())
            .await
            .unwrap();
        let usage = AuthoritativeRetentionUsage {
            count: 1,
            bytes: serde_json::to_string(&bundle).unwrap().len(),
        };
        let generation = store.authoritative_retention_generation().await.unwrap();
        assert!(
            !store
                .repair_authoritative_retention(generation, usage)
                .await
                .unwrap()
        );
        assert!(matches!(
            store
                .repair_authoritative_retention(
                    generation,
                    AuthoritativeRetentionUsage {
                        count: 1,
                        bytes: usage.bytes + 1,
                    },
                )
                .await,
            Err(StoreError::AuthoritativeCorrupt(_))
        ));

        {
            let mut inner = store.lock().unwrap();
            let document = inner
                .authoritative_intents
                .get_mut(&bundle.token())
                .unwrap();
            document.insert(0, ' ');
        }
        assert!(matches!(
            store.scan_authoritative_intents(None, 1).await,
            Err(StoreError::AuthoritativeCorrupt(message))
                if message.contains("canonical storage form")
        ));
    }

    #[tokio::test]
    async fn authoritative_retention_repair_rejects_a_stale_generation() {
        let store = InMemoryStore::new();
        seed_minimal_world(&store).await;
        let bundle = authoritative_bundle(73, 74);
        let token = bundle.token();
        let hash = bundle.bundle_hash();
        store
            .reserve_authoritative_intent(bundle.clone())
            .await
            .unwrap();
        let generation = store.authoritative_retention_generation().await.unwrap();
        let stale_usage = AuthoritativeRetentionUsage {
            count: 1,
            bytes: serde_json::to_string(&bundle).unwrap().len(),
        };
        store.reconcile_authoritative(token, hash).await.unwrap();
        store.checkpoint_authoritative(token, hash).await.unwrap();
        assert!(matches!(
            store
                .repair_authoritative_retention(generation, stale_usage)
                .await,
            Err(StoreError::AuthoritativeRetentionContention)
        ));
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
                .reserve_authoritative_intent(bundle.clone())
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
        store.lock().unwrap().authoritative_pending.insert(token);
        assert_eq!(
            store.pending_authoritative_intents().await.unwrap().len(),
            1
        );
        assert!(
            store
                .reserve_authoritative_intent(bundle)
                .await
                .unwrap()
                .resumed
        );
        assert!(
            store
                .pending_authoritative_intents()
                .await
                .unwrap()
                .is_empty()
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
    async fn authoritative_checkpoint_releases_replay_but_preserves_exact_retry() {
        let store = InMemoryStore::new();
        seed_minimal_world(&store).await;
        let bundle = authoritative_bundle(81, 82);
        let token = bundle.token();
        let hash = bundle.bundle_hash();
        let key = bundle.idempotency().clone();
        let ingress = bundle.ingress_hash();
        let intent_bytes = serde_json::to_string(&bundle).unwrap().len();

        store
            .reserve_authoritative_intent(bundle.clone())
            .await
            .unwrap();
        assert!(matches!(
            store.checkpoint_authoritative(token, hash).await,
            Err(StoreError::AuthoritativeNotFinalized)
        ));
        let receipt = store.reconcile_authoritative(token, hash).await.unwrap();
        let checkpoint = store.checkpoint_authoritative(token, hash).await.unwrap();
        assert_eq!(checkpoint.receipt, receipt);
        assert_eq!(checkpoint.released_intent_bytes, intent_bytes);
        assert!(!checkpoint.resumed);
        assert!(
            store
                .scan_authoritative_intents(None, 1)
                .await
                .unwrap()
                .intents
                .is_empty()
        );
        assert_eq!(
            store.authoritative_status(token, hash).await.unwrap(),
            BundleStatus::FinalizedExact(Box::new(receipt.clone()))
        );
        assert_eq!(
            store
                .authoritative_idempotency_status(&key, ingress)
                .await
                .unwrap(),
            IdempotencyStatus::FinalizedExact(Box::new(receipt.clone()))
        );
        assert!(
            store
                .reserve_authoritative_intent(bundle.clone())
                .await
                .unwrap()
                .resumed
        );
        assert_eq!(
            store.reconcile_authoritative(token, hash).await.unwrap(),
            receipt
        );
        let resumed = store.checkpoint_authoritative(token, hash).await.unwrap();
        assert!(resumed.resumed);
        assert_eq!(resumed.released_intent_bytes, 0);
        assert_eq!(resumed.receipt, receipt);

        let inner = store.lock().unwrap();
        assert!(inner.authoritative_intents.is_empty());
        assert_eq!(inner.authoritative_receipts.len(), 1);
        assert!(inner.authoritative_pending.is_empty());
    }

    #[tokio::test]
    async fn authoritative_checkpoint_retains_intent_when_a_post_image_was_clobbered() {
        let store = InMemoryStore::new();
        seed_minimal_world(&store).await;
        let bundle = authoritative_bundle(83, 84);
        let token = bundle.token();
        let hash = bundle.bundle_hash();

        store
            .reserve_authoritative_intent(bundle)
            .await
            .expect("reserve checkpoint candidate");
        store
            .reconcile_authoritative(token, hash)
            .await
            .expect("finalize checkpoint candidate");
        store.lock().unwrap().next_id = Some(4);

        assert!(matches!(
            store.checkpoint_authoritative(token, hash).await,
            Err(StoreError::AuthoritativeReadbackMismatch { ref key, phase })
                if key == META_NEXT_ID && phase == AuthoritativeReadbackPhase::Checkpoint
        ));
        assert_eq!(
            store
                .scan_authoritative_intents(None, 1)
                .await
                .unwrap()
                .intents
                .len(),
            1,
            "failed checkpoint must retain the exact repair witness"
        );
    }

    #[tokio::test]
    async fn authoritative_store_refuses_a_second_unresolved_intent() {
        let store = InMemoryStore::new();
        seed_minimal_world(&store).await;
        let first = authoritative_bundle_for_request(85, 86, 1, ObjId(2), Some(3));
        let second = authoritative_bundle_for_request(87, 88, 2, ObjId(2), Some(3));

        store
            .reserve_authoritative_intent(first.clone())
            .await
            .expect("reserve first intent");
        assert!(matches!(
            store.reserve_authoritative_intent(second.clone()).await,
            Err(StoreError::AuthoritativeIntentInFlight { pending: 1 })
        ));

        store
            .reconcile_authoritative(first.token(), first.bundle_hash())
            .await
            .expect("resolve first intent");
        store
            .reserve_authoritative_intent(second)
            .await
            .expect("reservation proceeds after the first intent resolves");
    }

    #[tokio::test]
    async fn quarantined_discard_releases_capacity_without_reopening_identity() {
        let store = InMemoryStore::new();
        seed_minimal_world(&store).await;
        let bundle = authoritative_bundle(89, 90);
        let token = bundle.token();
        let hash = bundle.bundle_hash();
        let key = bundle.idempotency().clone();
        let ingress = bundle.ingress_hash();
        let evidence_hash = CanonicalHash::from_bytes([91; 32]);
        let intent_bytes = serde_json::to_string(&bundle).unwrap().len();
        store
            .reserve_authoritative_intent(bundle.clone())
            .await
            .unwrap();

        let discard = store
            .discard_quarantined_authoritative_intent(token, hash, evidence_hash)
            .await
            .unwrap();
        assert!(!discard.resumed);
        assert_eq!(discard.released_intent_bytes, intent_bytes);
        assert_eq!(discard.quarantine.evidence_hash(), evidence_hash);
        assert!(
            store
                .scan_authoritative_intents(None, 1)
                .await
                .unwrap()
                .intents
                .is_empty()
        );
        assert_eq!(
            store.authoritative_status(token, hash).await.unwrap(),
            BundleStatus::QuarantinedExact(Box::new(discard.quarantine.clone()))
        );
        assert_eq!(
            store
                .authoritative_idempotency_status(&key, ingress)
                .await
                .unwrap(),
            IdempotencyStatus::QuarantinedExact(Box::new(discard.quarantine.clone()))
        );
        assert!(matches!(
            store.reserve_authoritative_intent(bundle.clone()).await,
            Err(StoreError::AuthoritativeIntentQuarantined {
                evidence_hash: observed
            }) if observed == evidence_hash
        ));
        let resumed = store
            .discard_quarantined_authoritative_intent(token, hash, evidence_hash)
            .await
            .unwrap();
        assert!(resumed.resumed);
        assert_eq!(resumed.released_intent_bytes, 0);
        let restored = store
            .restore_quarantined_authoritative_intent(bundle, evidence_hash)
            .await
            .unwrap();
        assert!(!restored.resumed);
        assert_eq!(
            store.authoritative_status(token, hash).await.unwrap(),
            BundleStatus::PendingExact
        );
    }

    #[tokio::test]
    async fn finalized_intent_cannot_be_discarded_as_quarantine() {
        let store = InMemoryStore::new();
        seed_minimal_world(&store).await;
        let bundle = authoritative_bundle(92, 93);
        let token = bundle.token();
        let hash = bundle.bundle_hash();
        store.reserve_authoritative_intent(bundle).await.unwrap();
        store.reconcile_authoritative(token, hash).await.unwrap();
        assert!(matches!(
            store
                .discard_quarantined_authoritative_intent(
                    token,
                    hash,
                    CanonicalHash::from_bytes([94; 32]),
                )
                .await,
            Err(StoreError::AuthoritativeIntentAlreadyFinalized)
        ));
    }

    #[tokio::test]
    async fn durable_outbox_is_finalization_gated_retryable_and_blocks_checkpoint() {
        let store = InMemoryStore::new();
        seed_minimal_world(&store).await;
        let intent = DurableOutboxIntent::new(
            0,
            OwnerId::new("toy").unwrap(),
            CanonicalId::new("webhook").unwrap(),
            serde_json::json!({"event": "braced"}),
        )
        .unwrap();
        let payload_hash = intent.payload_hash();
        let bundle = authoritative_bundle_for_epoch(AuthoritativeBundleFixture {
            token_byte: 111,
            ingress_byte: 112,
            request_byte: 113,
            object_id: ObjId(2),
            next_id: Some(3),
            epoch: 0,
            durable_outbox: vec![intent],
            extra_writes: vec![],
        });
        let token = bundle.token();
        let hash = bundle.bundle_hash();
        let scope = bundle.idempotency().scope().clone();
        store.reserve_authoritative_intent(bundle).await.unwrap();
        assert!(
            store
                .pending_authoritative_outbox()
                .await
                .unwrap()
                .is_empty()
        );
        store.reconcile_authoritative(token, hash).await.unwrap();

        let pending = store.pending_authoritative_outbox().await.unwrap();
        assert_eq!(pending.len(), 1);
        let item_id = pending[0].item_id();
        assert_eq!(pending[0].attempts(), 0);
        let unroutable = store
            .note_authoritative_outbox_unroutable(
                item_id,
                payload_hash,
                0,
                "adapter unavailable".to_owned(),
            )
            .await
            .unwrap();
        assert_eq!(unroutable.unroutable_passes(), 1);
        assert_eq!(unroutable.attempts(), 0);
        assert_eq!(
            store
                .note_authoritative_outbox_unroutable(
                    item_id,
                    payload_hash,
                    0,
                    "adapter unavailable".to_owned(),
                )
                .await
                .unwrap(),
            unroutable,
            "a lost acknowledgement joins the exact persisted pass"
        );
        assert!(matches!(
            store.checkpoint_authoritative(token, hash).await,
            Err(StoreError::AuthoritativeOutboxPending)
        ));

        let first = store
            .begin_authoritative_outbox_attempt(item_id, payload_hash, 0)
            .await
            .unwrap();
        assert_eq!(first.attempts(), 1);
        assert_eq!(
            store
                .begin_authoritative_outbox_attempt(item_id, payload_hash, 0)
                .await
                .unwrap(),
            first,
            "lost begin acknowledgement joins the same attempt"
        );
        let retry = store
            .complete_authoritative_outbox_attempt(
                item_id,
                payload_hash,
                1,
                AuthoritativeOutboxAttemptResult::Retry {
                    error: "temporary outage".to_owned(),
                },
            )
            .await
            .unwrap();
        assert_eq!(retry.attempts(), 1);
        assert!(!retry.state().is_terminal());

        let second = store
            .begin_authoritative_outbox_attempt(item_id, payload_hash, 1)
            .await
            .unwrap();
        assert_eq!(second.attempts(), 2);
        let delivered = store
            .complete_authoritative_outbox_attempt(
                item_id,
                payload_hash,
                2,
                AuthoritativeOutboxAttemptResult::Delivered,
            )
            .await
            .unwrap();
        assert!(delivered.state().is_terminal());
        assert!(
            store
                .pending_authoritative_outbox()
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store
                .complete_authoritative_outbox_attempt(
                    item_id,
                    payload_hash,
                    2,
                    AuthoritativeOutboxAttemptResult::Delivered,
                )
                .await
                .unwrap(),
            delivered,
            "lost terminal acknowledgement is idempotent"
        );
        store
            .checkpoint_authoritative(token, hash)
            .await
            .expect("terminal outbox permits compaction");
        store
            .retire_authoritative_epochs(&scope, 1)
            .await
            .expect("parent epoch and terminal delivery retire together");
        let inner = store.lock().unwrap();
        assert!(!inner.authoritative_outbox.contains_key(&item_id));
        assert!(!inner.authoritative_outbox_pending.contains(&item_id));
    }

    #[tokio::test]
    async fn authoritative_epoch_retirement_is_monotonic_and_never_reopens_old_keys() {
        let store = InMemoryStore::new();
        seed_minimal_world(&store).await;
        let old = authoritative_bundle_for_epoch(AuthoritativeBundleFixture {
            token_byte: 101,
            ingress_byte: 102,
            request_byte: 103,
            object_id: ObjId(2),
            next_id: Some(3),
            epoch: 0,
            durable_outbox: vec![],
            extra_writes: vec![],
        });
        let old_token = old.token();
        let old_hash = old.bundle_hash();
        let old_key = old.idempotency().clone();
        let scope = old_key.scope().clone();

        store
            .reserve_authoritative_intent(old.clone())
            .await
            .unwrap();
        store
            .reconcile_authoritative(old_token, old_hash)
            .await
            .unwrap();
        store
            .checkpoint_authoritative(old_token, old_hash)
            .await
            .unwrap();

        let retired = store.retire_authoritative_epochs(&scope, 1).await.unwrap();
        assert_eq!(retired.previous_minimum_epoch, 0);
        assert_eq!(retired.minimum_accepted_epoch, 1);
        assert_eq!(retired.retired_tombstones, 1);
        assert!(!retired.resumed);
        assert_eq!(
            store
                .authoritative_idempotency_status(&old_key, old.ingress_hash())
                .await
                .unwrap(),
            IdempotencyStatus::Retired {
                minimum_accepted_epoch: 1
            }
        );
        assert!(matches!(
            store.reserve_authoritative_intent(old.clone()).await,
            Err(StoreError::AuthoritativeEpochRetired {
                requested_epoch: 0,
                minimum_accepted_epoch: 1,
            })
        ));
        assert_eq!(
            store
                .authoritative_status(old_token, old_hash)
                .await
                .unwrap(),
            BundleStatus::Absent
        );
        assert_eq!(
            store.authoritative_bundle_status(&old).await.unwrap(),
            BundleStatus::Retired {
                minimum_accepted_epoch: 1
            }
        );

        let resumed = store.retire_authoritative_epochs(&scope, 1).await.unwrap();
        assert!(resumed.resumed);
        assert_eq!(resumed.retired_tombstones, 0);
        assert!(matches!(
            store.retire_authoritative_epochs(&scope, 0).await,
            Err(StoreError::AuthoritativeEpochRegression {
                current_minimum_epoch: 1,
                requested_minimum_epoch: 0,
            })
        ));

        let current = authoritative_bundle_for_epoch(AuthoritativeBundleFixture {
            token_byte: 104,
            ingress_byte: 105,
            request_byte: 106,
            object_id: ObjId(3),
            next_id: Some(4),
            epoch: 1,
            durable_outbox: vec![],
            extra_writes: vec![],
        });
        let current_token = current.token();
        let current_hash = current.bundle_hash();
        store.reserve_authoritative_intent(current).await.unwrap();
        assert!(matches!(
            store.retire_authoritative_epochs(&scope, 2).await,
            Err(StoreError::AuthoritativeEpochPending {
                requested_minimum_epoch: 2,
                retained_intents: 1,
            })
        ));
        store
            .reconcile_authoritative(current_token, current_hash)
            .await
            .unwrap();
        assert!(matches!(
            store.retire_authoritative_epochs(&scope, 2).await,
            Err(StoreError::AuthoritativeEpochPending {
                requested_minimum_epoch: 2,
                retained_intents: 1,
            })
        ));
        store
            .checkpoint_authoritative(current_token, current_hash)
            .await
            .unwrap();
        let retired = store.retire_authoritative_epochs(&scope, 2).await.unwrap();
        assert_eq!(retired.retired_tombstones, 1);
    }

    #[test]
    fn compact_receipt_reads_legacy_bytes_and_rejects_hash_tampering() {
        let bundle = authoritative_bundle(91, 92);
        let legacy = serde_json::json!({ "bundle": bundle });
        let receipt: AuthoritativeReceipt = serde_json::from_value(legacy).unwrap();
        assert!(receipt.matches_bundle(&bundle));

        let mut current = serde_json::to_value(&receipt).unwrap();
        current["receipt_hash"] = serde_json::json!(CanonicalHash::from_bytes([0xaa; 32]));
        assert!(serde_json::from_value::<AuthoritativeReceipt>(current).is_err());
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

    /// J1: the split listing hands a ledger chunk over as the text the store
    /// holds — undecoded, so a corrupt chunk is the ledger inspection's to
    /// report — while an ordinary document still decodes (and a corrupt one
    /// still errors), and the flat listing is unchanged.
    #[tokio::test]
    async fn module_docs_hands_ledger_text_over_undecoded() {
        let store = InMemoryStore::new();
        let chunk = r#"{"chunk":0,"sealed":false,"entries":[{"seq":0,"value":7}]}"#;
        {
            let mut inner = store.lock().expect("store lock");
            inner
                .module_kv
                .insert(("m".into(), "doc".into()), r#"{"v":1}"#.into());
            inner
                .module_kv
                .insert(("m".into(), "log:s:0".into()), chunk.into());
            inner
                .module_kv
                .insert(("m".into(), "log:s:broken".into()), "{not-json".into());
        }
        let docs = store.module_docs("m").await.expect("split listing");
        assert_eq!(
            docs.docs,
            vec![("doc".to_string(), serde_json::json!({ "v": 1 }))]
        );
        let mut ledger = docs.ledger.clone();
        ledger.sort();
        assert_eq!(
            ledger,
            vec![
                ("log:s:0".to_string(), chunk.to_string()),
                ("log:s:broken".to_string(), "{not-json".to_string()),
            ]
        );
        let error = store
            .module_json_list("m")
            .await
            .expect_err("a decoded listing still refuses the corrupt chunk");
        assert!(matches!(error, StoreError::Serde(_)), "{error:?}");
        assert!(matches!(docs.into_json_list(), Err(StoreError::Serde(_))));

        {
            let mut inner = store.lock().expect("store lock");
            inner
                .module_kv
                .insert(("m".into(), "doc".into()), "{not-json".into());
        }
        let error = store
            .module_docs("m")
            .await
            .expect_err("a corrupt ordinary document never disappears from boot discovery");
        assert!(matches!(error, StoreError::Serde(_)), "{error:?}");
    }
    /// The raw seam's whole contract: bytes in, the same bytes out, no
    /// decoding, no normalization — the damage a refusal test needs to be
    /// able to plant is damage this crate would never write itself.
    #[tokio::test]
    async fn planted_raw_docs_read_back_verbatim() {
        let store = InMemoryStore::new();
        // A body whose id disagrees with its key, and a body that is not an
        // object doc at all.
        let mismatched = r#"{"id":4,"name":"nine"}"#;
        store
            .debug_plant_raw_doc("obj:9", mismatched)
            .await
            .expect("plant obj:9");
        store
            .debug_plant_raw_doc("obj:7", r#"{"id":"nonsense"}"#)
            .await
            .expect("plant obj:7");
        assert_eq!(
            store
                .debug_read_raw_doc("obj:9")
                .await
                .expect("read obj:9")
                .as_deref(),
            Some(mismatched),
            "the seam must not re-encode what it was handed"
        );
        assert_eq!(
            store.debug_read_raw_doc("obj:5").await.expect("read obj:5"),
            None
        );
        // A key this store cannot address is refused, never silently dropped.
        let error = store
            .debug_plant_raw_doc("mod:m:doc", "{}")
            .await
            .expect_err("only obj:* and account:* are addressable");
        assert!(matches!(error, StoreError::Invalid(_)), "{error:?}");
        let error = store
            .debug_plant_raw_doc("obj:nine", "{}")
            .await
            .expect_err("an unparseable object id is refused");
        assert!(matches!(error, StoreError::Invalid(_)), "{error:?}");
    }

    /// Planting an object lifts `meta:next_id` above it. Without that the
    /// store is corrupt on its own terms — an id at or above the allocator
    /// resume point — and `validate_replay_admission` would refuse every
    /// authoritative write before the planted damage ever got its turn.
    #[tokio::test]
    async fn planting_an_object_lifts_the_allocator_above_it() {
        let store = InMemoryStore::new();
        store
            .debug_plant_raw_doc("obj:9", r#"{"id":"nonsense"}"#)
            .await
            .expect("plant obj:9");
        let snapshot = store
            .load_world()
            .await
            .expect("load")
            .expect("a planted store is not empty");
        assert_eq!(snapshot.next_id, 10, "the allocator must clear the plant");
        assert_eq!(snapshot.corrupt_docs, 1);
        assert!(snapshot.objects.is_empty());
        // A lower plant afterwards must not walk the allocator backwards.
        store
            .debug_plant_raw_doc("obj:2", r#"{"id":"nonsense"}"#)
            .await
            .expect("plant obj:2");
        assert_eq!(
            store
                .load_world()
                .await
                .expect("load")
                .expect("snapshot")
                .next_id,
            10
        );
    }

    /// The seam takes an `account:` key verbatim, which is the point: every
    /// *keyed* read lowercases the name, so `account:Nick` is a shadow doc no
    /// keyed read resolves and no `NX` blocks — the exact damage a hand
    /// repair leaves behind, and one only a raw seam can plant.
    ///
    /// The split is the whole diagnostic, so pin both halves. A shadow doc is
    /// **not** invisible to an unkeyed listing on either backend
    /// (`list_accounts` walks map values here, `SCAN account:*` in Redis),
    /// and that is exactly what makes a non-lowercase `account:*` key
    /// detectable at all: the listing reports an account the keyed reads
    /// cannot reach, and the same name can be created a second time beside
    /// it.
    #[tokio::test]
    async fn a_non_lowercase_account_key_plants_a_shadow_doc() {
        let store = InMemoryStore::new();
        // Decodable on purpose: a decode failure is a different fault (see
        // `an_undecodable_account_doc_is_dropped_by_the_listing`) and must not
        // be allowed to stand in for a key-normalization proof.
        let doc = r#"{"name":"Nick","argon2_hash":"x","created":1,"characters":[]}"#;
        store
            .debug_plant_raw_doc("account:Nick", doc)
            .await
            .expect("plant account:Nick");
        assert_eq!(
            store
                .debug_read_raw_doc("account:Nick")
                .await
                .expect("read account:Nick")
                .as_deref(),
            Some(doc)
        );
        // Keyed reads compute `account:nick` and miss it, in both directions:
        // nothing to fetch, and nothing to update.
        assert!(
            store
                .get_account("Nick")
                .await
                .expect("get_account")
                .is_none(),
            "a shadow doc is invisible to a keyed read"
        );
        assert!(
            !store
                .set_account_password("Nick", "y")
                .await
                .expect("set_account_password"),
            "a shadow doc is invisible to a keyed write"
        );
        // And `NX` does not see it either, so the name can be taken again.
        store
            .create_account(AccountRec {
                id: None,
                name: "Nick".to_owned(),
                argon2_hash: "live".to_owned(),
                created: 2,
                characters: vec![],
                ssh_keys: vec![],
                discord: None,
            })
            .await
            .expect("the shadow doc does not reserve the name");
        // The unkeyed listing sees both — one account by every keyed path,
        // two by the listing, which is the discrepancy a diagnostic reports.
        let listed = store.list_accounts().await.expect("list");
        assert_eq!(listed.len(), 2, "the shadow doc is listed, not hidden");
        assert!(listed.iter().all(|rec| rec.name == "Nick"));
        // The live account's write went to its own key: the planted bytes are
        // still exactly as planted.
        assert_eq!(
            store
                .debug_read_raw_doc("account:Nick")
                .await
                .expect("read account:Nick")
                .as_deref(),
            Some(doc)
        );
    }

    /// The other account fault, kept apart from the one above: a doc that
    /// does not decode (`created` has no serde default) is dropped by the
    /// listing with a log line, and errors the keyed read rather than
    /// answering `None`. Both faults hide an account from `get_account`; only
    /// the listing tells them apart.
    #[tokio::test]
    async fn an_undecodable_account_doc_is_dropped_by_the_listing() {
        let store = InMemoryStore::new();
        store
            .debug_plant_raw_doc("account:nick", r#"{"name":"nick","argon2_hash":"x"}"#)
            .await
            .expect("plant account:nick");
        assert!(
            store.list_accounts().await.expect("list").is_empty(),
            "an undecodable account doc is skipped by the listing"
        );
        let error = store
            .get_account("nick")
            .await
            .expect_err("a keyed read reports the decode failure");
        assert!(matches!(error, StoreError::Serde(_)), "{error:?}");
    }

    /// The seam and `mush_core::depth` are one pair: a document past
    /// `MAX_DOC_DEPTH` is plantable, and measurable on the way back out.
    ///
    /// The depth is deliberately inside the window `serde_json` still
    /// decodes: this store takes the bytes verbatim, but the Redis seam
    /// re-parses them through `JSON.SET`, so a plant past 128 levels lands on
    /// one backend and is refused by the other. Damage a shared suite can
    /// plant has to fit through the narrower door.
    #[tokio::test]
    async fn an_over_deep_doc_survives_the_round_trip_and_is_measurable() {
        let store = InMemoryStore::new();
        let depth = mush_core::MAX_DOC_DEPTH + 4;
        let text = "[".repeat(depth) + &"]".repeat(depth);
        store
            .debug_plant_raw_doc("obj:3", &text)
            .await
            .expect("plant an over-deep doc");
        let read = store
            .debug_read_raw_doc("obj:3")
            .await
            .expect("read obj:3")
            .expect("planted doc is present");
        assert_eq!(read, text);
        assert_eq!(mush_core::depth_of_json_text(&read), depth);
        assert!(mush_core::depth_of_json_text(&read) > mush_core::MAX_DOC_DEPTH);
    }

    /// The parity half of the Redis regression in
    /// `tests/redis_suite.rs::one_undecodable_object_doc_does_not_abort_the_world_load`:
    /// this store already classifies a bad document per key, because it
    /// decodes inside the loop. The Redis store used to decode inside its
    /// transport helper, so one over-deep `locks` tree aborted the whole load.
    /// Pinning the shape here is what makes "the two backends agree" a fact
    /// rather than a hope.
    #[tokio::test]
    async fn one_undecodable_doc_does_not_abort_the_world_load() {
        let store = InMemoryStore::new();
        let good = |id: u32| Obj::new(ObjId(id), mush_core::ObjKind::Thing, "keep", ObjId(1), 0);
        store
            .write(WriteBatch {
                serial: 1,
                upserts: vec![good(1), good(2)],
                next_id: Some(3),
                ..WriteBatch::default()
            })
            .await
            .expect("seed two sound objects");

        // Past `serde_json`'s recursion limit, so no decoder reaches the end
        // of it — the shape a hostile `locks` tree takes in a store.
        let depth = 200;
        let deep = format!(
            r#"{{"id":9,"name":"deep","locks":{{"Enter":{}}}}}"#,
            "[".repeat(depth) + &"]".repeat(depth)
        );
        assert!(
            serde_json::from_str::<serde_json::Value>(&deep).is_err(),
            "the plant is only a plant if it is genuinely undecodable"
        );
        store
            .debug_plant_raw_doc("obj:9", &deep)
            .await
            .expect("plant an undecodable doc");

        let snapshot = store
            .load_world()
            .await
            .expect("one undecodable doc must not abort the load")
            .expect("a seeded store is not empty");
        assert_eq!(snapshot.corrupt_docs, 1);
        let mut ids: Vec<u32> = snapshot.objects.iter().map(|o| o.id.0).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![1, 2], "the sound objects still load");
    }

    /// The poison path: a document past [`mush_core::MAX_DOC_DEPTH`] never
    /// reaches the store, on either backend.
    ///
    /// Nothing legal builds one — `parse_lock` bounds the tallest shape this
    /// server writes, and the deepest document it constructs measures 98
    /// levels against a bound of 120 — so this is reached only by a bug, and
    /// the refusal is deliberately [`StoreError::Corrupt`]: a write the world
    /// cannot read back is worse than a write that never happened.
    #[tokio::test]
    async fn the_write_path_refuses_an_over_deep_object_doc() {
        let store = InMemoryStore::new();
        let mut obj = Obj::new(ObjId(1), mush_core::ObjKind::Thing, "poison", ObjId(1), 0);
        // Nested by a loop, so building the fixture does not itself recurse.
        let mut value = serde_json::Value::Null;
        for _ in 0..mush_core::MAX_DOC_DEPTH {
            value = serde_json::Value::Array(vec![value]);
        }
        obj.attrs
            .set("deep", mush_core::Attr::json(value, ObjId(1)));
        assert!(
            mush_core::depth_of_json_text(
                &serde_json::to_string(&obj).expect("the fixture serializes")
            ) > mush_core::MAX_DOC_DEPTH,
            "the fixture must actually be over the bound"
        );

        let error = store
            .write(WriteBatch {
                serial: 1,
                upserts: vec![obj],
                next_id: Some(2),
                ..WriteBatch::default()
            })
            .await
            .expect_err("an over-deep object doc is never written");
        assert!(
            matches!(error, StoreError::Corrupt(ref m) if m.contains("JSON levels deep")),
            "{error:?}"
        );
        assert!(
            !error.is_transient(),
            "retrying a poisoned batch never helps"
        );
        // All-or-nothing: the refused batch left no trace.
        assert_eq!(store.load_world().await.expect("load"), None);
    }
}

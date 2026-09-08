//! Domain types shared by every [`crate::ObjectStore`] implementation.

use mush_core::{
    AuthoritativeBundle, CanonicalHash, CommitToken, DefinitionRef, DurableOutboxIntent,
    IdempotencyScope, MAX_MODULE_KEY_BYTES, MAX_MODULE_NAMESPACE_BYTES, Obj, ObjId, OwnerId,
    PreparedCausalRef, PrincipalId, RequestUid, RulesValue, ScopedIdempotencyKey,
    is_action_state_persistence_key,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;

// The module-op vocabulary is owned by mush-core (modules queue these
// through `Ctx::persist`); re-exported here because [`WriteBatch`] carries
// them into the store.
pub use mush_core::{ModuleOp, ModuleWrite};

/// Everything needed to rebuild the world at boot: the persisted objects,
/// the id allocator's resume point, and the store's schema version.
///
/// Feed `objects` + `next_id` straight into
/// [`mush_core::World::from_objects`]; hand `schema_version` to
/// [`crate::run_migrations`].
#[derive(Clone, Debug, PartialEq)]
pub struct WorldSnapshot {
    /// Every live object, as persisted. Order is unspecified.
    pub objects: Vec<Obj>,
    /// The id allocator resume point (strictly above every persisted id).
    pub next_id: u32,
    /// App-level schema version of the persisted data (see [`crate::migrate`]).
    pub schema_version: u32,
    /// How many stored object docs were unparseable and skipped during the
    /// load. Loaders surface this so a hydration failure caused by a
    /// skipped doc (e.g. a dangling `location`) can name its likely cause.
    pub corrupt_docs: usize,
}

/// One processed command's world mutations, applied to the store as a single
/// atomic unit (Redis `MULTI`/`EXEC`; one lock acquisition in memory).
///
/// Semantics, identical in every implementation:
///
/// - `upserts` are **full-object snapshots** (no deltas) and are applied
///   first, in order — a later upsert of the same id wins;
/// - `deletes` are applied **after** upserts, so an id in both lists ends up
///   deleted;
/// - `module_ops` are applied after the object deletes, **in queue order**
///   (module keys live under `mod:{ns}:{key}` — see
///   [`crate::ObjectStore::module_json_get`] for the namespace rules);
/// - `next_id`, when present, is stored last;
/// - the batch applies **atomically**: a failure before the store accepts
///   it (serialization, connection loss, a refused `EXEC`) leaves the store
///   exactly as it was. One Redis-inherited caveat: `MULTI`/`EXEC` does not
///   roll back a command that errors at *execution* time (possible only if
///   an outside client clobbered a key with the wrong type), so such a
///   failure can leave the batch partially applied — see
///   [`crate::redis`]'s module docs. The in-memory store has no such edge.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WriteBatch {
    /// Caller-assigned monotonic batch number (the world task's command
    /// serial). Echoed in the receipt; the store does not reorder batches —
    /// ordering is the single writer's job (see the concurrency contract in
    /// [`crate::store`]).
    pub serial: u64,
    /// Full-object snapshots to insert or replace.
    pub upserts: Vec<Obj>,
    /// Objects to remove (swept tombstones).
    pub deletes: Vec<ObjId>,
    /// Module persistence ops (comsys/mail/jobs state, `mod:{ns}:{key}`
    /// keys), drained from the per-command Outbox side-channel. They commit
    /// inside the same transaction as the object writes — module state gets
    /// the exact same atomicity and write-through durability. `WriteBatch`
    /// is never serialized today; if that changes, skip this field when
    /// empty.
    pub module_ops: Vec<ModuleOp>,
    /// New id-allocator resume point, if allocation advanced.
    pub next_id: Option<u32>,
    /// Durability-critical batch (account/character creation, and any step
    /// whose handler called `Ctx::gate_on_durable_ack` — mail send, job
    /// approval): the caller gates its confirmation output on this call
    /// returning `Ok`.
    /// Every batch is acknowledged the same way (the `EXEC` reply *is* the
    /// ack — see [`crate::redis`] on AOF durability); the flag exists so the
    /// store can trace-tag critical batches and so future backends can
    /// fsync harder.
    pub durable: bool,
}

impl WriteBatch {
    /// True when the batch would change nothing (no upserts, deletes,
    /// module ops, or `next_id`). Implementations skip the round trip for
    /// these.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.upserts.is_empty()
            && self.deletes.is_empty()
            && self.module_ops.is_empty()
            && self.next_id.is_none()
    }
}

/// Validate a module namespace: non-empty and free of `:` — the separator
/// in `mod:{ns}:{key}` keys. A `:` inside `ns` would let one namespace's
/// keys alias another's (`ns="a", key="b:c"` vs `ns="a:b", key="c"`), and
/// would let `module_json_list`'s `mod:{ns}:*` SCAN pattern leak across
/// namespaces. Rejected here, at the storage boundary, for every
/// implementation.
pub(crate) fn validate_module_ns(ns: &str) -> Result<(), StoreError> {
    if ns.is_empty() {
        return Err(StoreError::Invalid("empty module namespace".into()));
    }
    if ns.contains(':') {
        return Err(StoreError::Invalid(format!(
            "module namespace {ns:?} contains ':' (reserved separator)"
        )));
    }
    if ns.len() > MAX_MODULE_NAMESPACE_BYTES {
        return Err(StoreError::Invalid(format!(
            "module namespace is {} bytes; maximum is {MAX_MODULE_NAMESPACE_BYTES}",
            ns.len()
        )));
    }
    Ok(())
}

/// Validate a module key: non-empty. Unlike namespaces, keys **may**
/// contain `:` — they sit after the second separator, so with `:`-free
/// namespaces every `mod:{ns}:{key}` key parses unambiguously.
pub(crate) fn validate_module_key(key: &str) -> Result<(), StoreError> {
    if key.is_empty() {
        return Err(StoreError::Invalid("empty module key".into()));
    }
    if key.len() > MAX_MODULE_KEY_BYTES {
        return Err(StoreError::Invalid(format!(
            "module key is {} bytes; maximum is {MAX_MODULE_KEY_BYTES}",
            key.len()
        )));
    }
    Ok(())
}

/// Validate one [`ModuleOp`] (namespace + the key inside the write).
pub(crate) fn validate_module_op(op: &ModuleOp) -> Result<(), StoreError> {
    validate_module_ns(&op.ns)?;
    let key = match &op.op {
        ModuleWrite::JsonSet { key, .. }
        | ModuleWrite::Del { key }
        | ModuleWrite::StreamAppend { key, .. } => key,
    };
    validate_module_key(key)?;
    if is_action_state_persistence_key(key) {
        return Err(StoreError::Invalid(format!(
            "module key {key:?} is owned by authoritative action persistence"
        )));
    }
    Ok(())
}

/// Acknowledgement of a durably applied [`WriteBatch`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteReceipt {
    /// The `serial` of the acknowledged batch.
    pub serial: u64,
}

/// Exact acknowledgement that a complete authoritative intent is durable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntentReceipt {
    pub token: CommitToken,
    pub bundle_hash: CanonicalHash,
    /// True when the same exact intent or finalized result already existed.
    pub resumed: bool,
}

/// Hard maximum full intents retained for recovery by one store.
pub const MAX_RETAINED_AUTHORITATIVE_INTENTS: usize = 256;
/// Hard maximum encoded bytes across full intents retained for recovery.
pub const MAX_RETAINED_AUTHORITATIVE_BYTES: usize = 64 * 1024 * 1024;
/// Maximum independently discoverable durable-outbox items. This is the
/// product of the retained-action and per-action hard limits.
pub const MAX_PENDING_AUTHORITATIVE_OUTBOX: usize =
    MAX_RETAINED_AUTHORITATIVE_INTENTS * mush_core::MAX_DURABLE_OUTBOX_INTENTS;
/// Maximum diagnostic bytes retained on one outbox retry/dead-letter state.
pub const MAX_AUTHORITATIVE_OUTBOX_ERROR_BYTES: usize = 4 * 1024;
/// Hard maximum intent records returned by one recovery scan page.
pub const MAX_AUTHORITATIVE_DISCOVERY_PAGE: usize = 256;
/// Hard maximum opaque bytes in one backend recovery cursor.
pub const MAX_AUTHORITATIVE_SCAN_CURSOR_BYTES: usize = 256;

/// Exact derived capacity charge for all immutable full intents currently
/// retained by one store.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AuthoritativeRetentionUsage {
    pub count: usize,
    pub bytes: usize,
}

/// One exact entry recovered from the independently maintained pending index.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PendingAuthoritativeIntent {
    pub token: CommitToken,
    pub bundle_hash: CanonicalHash,
}

/// Opaque backend cursor for exhaustive authoritative-intent discovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthoritativeScanCursor(Box<str>);

impl AuthoritativeScanCursor {
    pub fn new(value: impl Into<Box<str>>) -> Result<Self, StoreError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_AUTHORITATIVE_SCAN_CURSOR_BYTES {
            return Err(StoreError::Invalid(format!(
                "authoritative scan cursor must contain 1..={MAX_AUTHORITATIVE_SCAN_CURSOR_BYTES} bytes"
            )));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One bounded page from the complete retained intent keyspace.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthoritativeIntentPage {
    pub intents: Vec<AuthoritativeBundle>,
    pub next_cursor: Option<AuthoritativeScanCursor>,
}

/// Durable delivery state for one post-finalization outbox item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuthoritativeOutboxState {
    Pending { last_error: Option<String> },
    Delivered,
    DeadLetter { error: String },
}

impl AuthoritativeOutboxState {
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Delivered | Self::DeadLetter { .. })
    }
}

/// Independently discoverable, stable-ID delivery record materialized by the
/// same finalization barrier as its parent action witness.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoritativeOutboxRecord {
    schema_version: u16,
    item_id: CanonicalHash,
    parent_token: CommitToken,
    parent_bundle_hash: CanonicalHash,
    intent: DurableOutboxIntent,
    attempts: u32,
    /// Discovery passes during which no registered adapter could route this
    /// item. Kept separate from delivery attempts so a temporarily missing
    /// adapter does not consume the adapter's retry budget.
    #[serde(default, skip_serializing_if = "u32_is_zero")]
    unroutable_passes: u32,
    state: AuthoritativeOutboxState,
}

const fn u32_is_zero(value: &u32) -> bool {
    *value == 0
}

impl AuthoritativeOutboxRecord {
    pub const SCHEMA_VERSION: u16 = 1;

    pub fn new(
        parent_token: CommitToken,
        parent_bundle_hash: CanonicalHash,
        intent: DurableOutboxIntent,
    ) -> Result<Self, StoreError> {
        let item_id = intent
            .stable_id(parent_token)
            .map_err(|error| StoreError::Invalid(format!("invalid outbox identity: {error}")))?;
        Ok(Self {
            schema_version: Self::SCHEMA_VERSION,
            item_id,
            parent_token,
            parent_bundle_hash,
            intent,
            attempts: 0,
            unroutable_passes: 0,
            state: AuthoritativeOutboxState::Pending { last_error: None },
        })
    }

    pub(crate) fn validate(&self) -> Result<(), StoreError> {
        if self.schema_version != Self::SCHEMA_VERSION {
            return Err(StoreError::AuthoritativeCorrupt(format!(
                "unsupported authoritative outbox schema {}",
                self.schema_version
            )));
        }
        let expected = self.intent.stable_id(self.parent_token).map_err(|error| {
            StoreError::AuthoritativeCorrupt(format!("invalid outbox identity: {error}"))
        })?;
        if self.item_id != expected {
            return Err(StoreError::AuthoritativeCorrupt(
                "authoritative outbox stable item id mismatch".to_owned(),
            ));
        }
        let error = match &self.state {
            AuthoritativeOutboxState::Pending { last_error } => last_error.as_deref(),
            AuthoritativeOutboxState::Delivered => None,
            AuthoritativeOutboxState::DeadLetter { error } => Some(error.as_str()),
        };
        if error.is_some_and(|error| error.len() > MAX_AUTHORITATIVE_OUTBOX_ERROR_BYTES) {
            return Err(StoreError::AuthoritativeCorrupt(format!(
                "authoritative outbox diagnostic exceeds {MAX_AUTHORITATIVE_OUTBOX_ERROR_BYTES} bytes"
            )));
        }
        Ok(())
    }

    pub(crate) fn start_attempt(&self, expected_attempts: u32) -> Result<Self, StoreError> {
        self.validate()?;
        if self.state.is_terminal() || self.attempts != expected_attempts {
            return Err(StoreError::AuthoritativeOutboxConflict);
        }
        let mut next = self.clone();
        next.attempts = next
            .attempts
            .checked_add(1)
            .ok_or(StoreError::AuthoritativeOutboxAttemptsExhausted)?;
        next.state = AuthoritativeOutboxState::Pending { last_error: None };
        Ok(next)
    }

    pub(crate) fn complete_attempt(
        &self,
        expected_attempt: u32,
        result: AuthoritativeOutboxAttemptResult,
    ) -> Result<Self, StoreError> {
        self.validate()?;
        if self.state.is_terminal() || self.attempts != expected_attempt {
            return Err(StoreError::AuthoritativeOutboxConflict);
        }
        let mut next = self.clone();
        next.state = match result {
            AuthoritativeOutboxAttemptResult::Retry { error } => {
                validate_outbox_error(&error)?;
                AuthoritativeOutboxState::Pending {
                    last_error: Some(error),
                }
            }
            AuthoritativeOutboxAttemptResult::Delivered => AuthoritativeOutboxState::Delivered,
            AuthoritativeOutboxAttemptResult::DeadLetter { error } => {
                validate_outbox_error(&error)?;
                AuthoritativeOutboxState::DeadLetter { error }
            }
        };
        Ok(next)
    }

    pub(crate) fn note_unroutable(
        &self,
        expected_passes: u32,
        error: String,
    ) -> Result<Self, StoreError> {
        self.validate()?;
        validate_outbox_error(&error)?;
        if self.state.is_terminal() || self.unroutable_passes != expected_passes {
            return Err(StoreError::AuthoritativeOutboxConflict);
        }
        let mut next = self.clone();
        next.unroutable_passes = next
            .unroutable_passes
            .checked_add(1)
            .ok_or(StoreError::AuthoritativeOutboxAttemptsExhausted)?;
        next.state = AuthoritativeOutboxState::Pending {
            last_error: Some(error),
        };
        Ok(next)
    }

    #[must_use]
    pub const fn item_id(&self) -> CanonicalHash {
        self.item_id
    }

    #[must_use]
    pub const fn parent_token(&self) -> CommitToken {
        self.parent_token
    }

    #[must_use]
    pub const fn parent_bundle_hash(&self) -> CanonicalHash {
        self.parent_bundle_hash
    }

    #[must_use]
    pub const fn intent(&self) -> &DurableOutboxIntent {
        &self.intent
    }

    #[must_use]
    pub const fn attempts(&self) -> u32 {
        self.attempts
    }

    #[must_use]
    pub const fn unroutable_passes(&self) -> u32 {
        self.unroutable_passes
    }

    #[must_use]
    pub const fn state(&self) -> &AuthoritativeOutboxState {
        &self.state
    }
}

fn validate_outbox_error(error: &str) -> Result<(), StoreError> {
    if error.is_empty() || error.len() > MAX_AUTHORITATIVE_OUTBOX_ERROR_BYTES {
        return Err(StoreError::Invalid(format!(
            "authoritative outbox diagnostic must contain 1..={MAX_AUTHORITATIVE_OUTBOX_ERROR_BYTES} bytes"
        )));
    }
    Ok(())
}

/// Result reported by a durable delivery adapter after one persisted attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthoritativeOutboxAttemptResult {
    Retry { error: String },
    Delivered,
    DeadLetter { error: String },
}

/// Compact immutable semantic result witness produced only after exact
/// readback of every authoritative post-image.
///
/// Full replay material remains in the separately retained intent until
/// [`crate::AuthoritativeStore::checkpoint_authoritative`] atomically replaces
/// any legacy full receipt with this canonical witness and releases the full
/// intent. The witness deliberately retains the authenticated result and
/// event-time visibility facts needed by exact retry/status, but never the
/// potentially multi-megabyte replay write set.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoritativeReceipt {
    schema_version: u16,
    token: CommitToken,
    request: RequestUid,
    idempotency: ScopedIdempotencyKey,
    ingress_hash: CanonicalHash,
    request_hash: CanonicalHash,
    plan_hash: CanonicalHash,
    action: DefinitionRef,
    provider: OwnerId,
    cause: PreparedCausalRef,
    outcome: RulesValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolution: Option<mush_core::ResolutionRecord>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    semantic_events: Vec<mush_core::AuthoritativeSemanticEvent>,
    eligible_witnesses: Vec<PrincipalId>,
    bundle_hash: CanonicalHash,
    receipt_hash: CanonicalHash,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoritativeReceiptV3Wire {
    schema_version: u16,
    token: CommitToken,
    request: RequestUid,
    idempotency: ScopedIdempotencyKey,
    ingress_hash: CanonicalHash,
    request_hash: CanonicalHash,
    plan_hash: CanonicalHash,
    action: DefinitionRef,
    provider: OwnerId,
    cause: PreparedCausalRef,
    outcome: RulesValue,
    semantic_events: Vec<mush_core::AuthoritativeSemanticEvent>,
    eligible_witnesses: Vec<PrincipalId>,
    bundle_hash: CanonicalHash,
    receipt_hash: CanonicalHash,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoritativeReceiptV4Wire {
    schema_version: u16,
    token: CommitToken,
    request: RequestUid,
    idempotency: ScopedIdempotencyKey,
    ingress_hash: CanonicalHash,
    request_hash: CanonicalHash,
    plan_hash: CanonicalHash,
    action: DefinitionRef,
    provider: OwnerId,
    cause: PreparedCausalRef,
    outcome: RulesValue,
    resolution: mush_core::ResolutionRecord,
    semantic_events: Vec<mush_core::AuthoritativeSemanticEvent>,
    eligible_witnesses: Vec<PrincipalId>,
    bundle_hash: CanonicalHash,
    receipt_hash: CanonicalHash,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoritativeReceiptV2Wire {
    schema_version: u16,
    token: CommitToken,
    request: RequestUid,
    idempotency: ScopedIdempotencyKey,
    ingress_hash: CanonicalHash,
    request_hash: CanonicalHash,
    plan_hash: CanonicalHash,
    action: DefinitionRef,
    provider: OwnerId,
    cause: PreparedCausalRef,
    outcome: RulesValue,
    eligible_witnesses: Vec<PrincipalId>,
    bundle_hash: CanonicalHash,
    receipt_hash: CanonicalHash,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyAuthoritativeReceiptWire {
    bundle: AuthoritativeBundle,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AuthoritativeReceiptWire {
    V4(Box<AuthoritativeReceiptV4Wire>),
    V3(Box<AuthoritativeReceiptV3Wire>),
    V2(Box<AuthoritativeReceiptV2Wire>),
    Legacy(Box<LegacyAuthoritativeReceiptWire>),
}

impl AuthoritativeReceipt {
    pub const SCHEMA_VERSION: u16 = 4;

    #[must_use]
    pub fn new(bundle: &AuthoritativeBundle) -> Self {
        let schema_version = if bundle.resolution().is_some() {
            Self::SCHEMA_VERSION
        } else if bundle.semantic_events().is_empty() {
            2
        } else {
            3
        };
        let mut receipt = Self {
            schema_version,
            token: bundle.token(),
            request: bundle.request(),
            idempotency: bundle.idempotency().clone(),
            ingress_hash: bundle.ingress_hash(),
            request_hash: bundle.request_hash(),
            plan_hash: bundle.plan_hash(),
            action: bundle.action().clone(),
            provider: bundle.provider().clone(),
            cause: bundle.cause(),
            outcome: bundle.outcome().clone(),
            resolution: bundle.resolution().cloned(),
            semantic_events: bundle.semantic_events().to_vec(),
            eligible_witnesses: bundle.eligible_witnesses().to_vec(),
            bundle_hash: bundle.bundle_hash(),
            receipt_hash: CanonicalHash::from_bytes([0; 32]),
        };
        receipt.receipt_hash = receipt
            .compute_hash()
            .expect("an authoritative bundle already bounds all receipt fields");
        receipt
    }

    #[must_use]
    pub const fn token(&self) -> CommitToken {
        self.token
    }

    #[must_use]
    pub const fn request(&self) -> RequestUid {
        self.request
    }

    #[must_use]
    pub const fn bundle_hash(&self) -> CanonicalHash {
        self.bundle_hash
    }

    #[must_use]
    pub const fn idempotency(&self) -> &ScopedIdempotencyKey {
        &self.idempotency
    }

    #[must_use]
    pub const fn ingress_hash(&self) -> CanonicalHash {
        self.ingress_hash
    }

    #[must_use]
    pub const fn request_hash(&self) -> CanonicalHash {
        self.request_hash
    }

    #[must_use]
    pub const fn plan_hash(&self) -> CanonicalHash {
        self.plan_hash
    }

    #[must_use]
    pub const fn action(&self) -> &DefinitionRef {
        &self.action
    }

    #[must_use]
    pub const fn provider(&self) -> &OwnerId {
        &self.provider
    }

    #[must_use]
    pub const fn cause(&self) -> PreparedCausalRef {
        self.cause
    }

    #[must_use]
    pub const fn outcome(&self) -> &RulesValue {
        &self.outcome
    }

    #[must_use]
    pub const fn resolution(&self) -> Option<&mush_core::ResolutionRecord> {
        self.resolution.as_ref()
    }

    #[must_use]
    pub fn semantic_events(&self) -> &[mush_core::AuthoritativeSemanticEvent] {
        &self.semantic_events
    }

    #[must_use]
    pub fn eligible_witnesses(&self) -> &[PrincipalId] {
        &self.eligible_witnesses
    }

    #[must_use]
    pub const fn receipt_hash(&self) -> CanonicalHash {
        self.receipt_hash
    }

    #[must_use]
    pub fn matches_bundle(&self, bundle: &AuthoritativeBundle) -> bool {
        self.token == bundle.token()
            && self.request == bundle.request()
            && self.idempotency == *bundle.idempotency()
            && self.ingress_hash == bundle.ingress_hash()
            && self.request_hash == bundle.request_hash()
            && self.plan_hash == bundle.plan_hash()
            && self.action == *bundle.action()
            && self.provider == *bundle.provider()
            && self.cause == bundle.cause()
            && self.outcome == *bundle.outcome()
            && self.resolution.as_ref() == bundle.resolution()
            && self.semantic_events == bundle.semantic_events()
            && self.eligible_witnesses == bundle.eligible_witnesses()
            && self.bundle_hash == bundle.bundle_hash()
    }

    fn compute_hash(&self) -> Result<CanonicalHash, mush_core::CanonicalHashError> {
        let (domain, material) = match self.schema_version {
            2 => (
                "authoritative_receipt.v2",
                json!({
                    "schema_version": self.schema_version,
                    "token": self.token,
                    "request": self.request,
                    "idempotency": self.idempotency,
                    "ingress_hash": self.ingress_hash,
                    "request_hash": self.request_hash,
                    "plan_hash": self.plan_hash,
                    "action": self.action,
                    "provider": self.provider,
                    "cause": self.cause,
                    "outcome": self.outcome,
                    "eligible_witnesses": self.eligible_witnesses,
                    "bundle_hash": self.bundle_hash,
                    "terminal_status": "finalized",
                }),
            ),
            3 => (
                "authoritative_receipt.v3",
                json!({
                    "schema_version": self.schema_version,
                    "token": self.token,
                    "request": self.request,
                    "idempotency": self.idempotency,
                    "ingress_hash": self.ingress_hash,
                    "request_hash": self.request_hash,
                    "plan_hash": self.plan_hash,
                    "action": self.action,
                    "provider": self.provider,
                    "cause": self.cause,
                    "outcome": self.outcome,
                    "semantic_events": self.semantic_events,
                    "eligible_witnesses": self.eligible_witnesses,
                    "bundle_hash": self.bundle_hash,
                    "terminal_status": "finalized",
                }),
            ),
            _ => (
                "authoritative_receipt.v4",
                json!({
                    "schema_version": self.schema_version,
                    "token": self.token,
                    "request": self.request,
                    "idempotency": self.idempotency,
                    "ingress_hash": self.ingress_hash,
                    "request_hash": self.request_hash,
                    "plan_hash": self.plan_hash,
                    "action": self.action,
                    "provider": self.provider,
                    "cause": self.cause,
                    "outcome": self.outcome,
                    "resolution": self.resolution,
                    "semantic_events": self.semantic_events,
                    "eligible_witnesses": self.eligible_witnesses,
                    "bundle_hash": self.bundle_hash,
                    "terminal_status": "finalized",
                }),
            ),
        };
        CanonicalHash::for_canonical_value(
            domain,
            self.schema_version,
            &material,
            mush_core::MAX_AUTHORITATIVE_BUNDLE_BYTES,
        )
    }
}

impl<'de> Deserialize<'de> for AuthoritativeReceipt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = AuthoritativeReceiptWire::deserialize(deserializer)?;
        match wire {
            AuthoritativeReceiptWire::Legacy(legacy) => Ok(Self::new(&legacy.bundle)),
            AuthoritativeReceiptWire::V4(wire) => {
                let wire = *wire;
                if wire.schema_version != Self::SCHEMA_VERSION {
                    return Err(serde::de::Error::custom(format!(
                        "unsupported authoritative receipt schema {}; expected {}",
                        wire.schema_version,
                        Self::SCHEMA_VERSION
                    )));
                }
                let receipt = Self {
                    schema_version: wire.schema_version,
                    token: wire.token,
                    request: wire.request,
                    idempotency: wire.idempotency,
                    ingress_hash: wire.ingress_hash,
                    request_hash: wire.request_hash,
                    plan_hash: wire.plan_hash,
                    action: wire.action,
                    provider: wire.provider,
                    cause: wire.cause,
                    outcome: wire.outcome,
                    resolution: Some(wire.resolution),
                    semantic_events: wire.semantic_events,
                    eligible_witnesses: wire.eligible_witnesses,
                    bundle_hash: wire.bundle_hash,
                    receipt_hash: wire.receipt_hash,
                };
                validate_receipt_events(&receipt).map_err(serde::de::Error::custom)?;
                validate_receipt_resolution(&receipt).map_err(serde::de::Error::custom)?;
                let computed = receipt.compute_hash().map_err(serde::de::Error::custom)?;
                if receipt.receipt_hash != computed {
                    return Err(serde::de::Error::custom(
                        "authoritative receipt hash mismatch",
                    ));
                }
                Ok(receipt)
            }
            AuthoritativeReceiptWire::V3(wire) => {
                let wire = *wire;
                if wire.schema_version != 3 {
                    return Err(serde::de::Error::custom(format!(
                        "unsupported authoritative receipt schema {}; expected {}",
                        wire.schema_version, 3
                    )));
                }
                let receipt = Self {
                    schema_version: wire.schema_version,
                    token: wire.token,
                    request: wire.request,
                    idempotency: wire.idempotency,
                    ingress_hash: wire.ingress_hash,
                    request_hash: wire.request_hash,
                    plan_hash: wire.plan_hash,
                    action: wire.action,
                    provider: wire.provider,
                    cause: wire.cause,
                    outcome: wire.outcome,
                    resolution: None,
                    semantic_events: wire.semantic_events,
                    eligible_witnesses: wire.eligible_witnesses,
                    bundle_hash: wire.bundle_hash,
                    receipt_hash: wire.receipt_hash,
                };
                validate_receipt_events(&receipt).map_err(serde::de::Error::custom)?;
                let computed = receipt.compute_hash().map_err(serde::de::Error::custom)?;
                if receipt.receipt_hash != computed {
                    return Err(serde::de::Error::custom(
                        "authoritative receipt hash mismatch",
                    ));
                }
                Ok(receipt)
            }
            AuthoritativeReceiptWire::V2(wire) => {
                let wire = *wire;
                if wire.schema_version != 2 {
                    return Err(serde::de::Error::custom(format!(
                        "unsupported authoritative receipt schema {}; expected {}",
                        wire.schema_version, 2
                    )));
                }
                let receipt = Self {
                    schema_version: wire.schema_version,
                    token: wire.token,
                    request: wire.request,
                    idempotency: wire.idempotency,
                    ingress_hash: wire.ingress_hash,
                    request_hash: wire.request_hash,
                    plan_hash: wire.plan_hash,
                    action: wire.action,
                    provider: wire.provider,
                    cause: wire.cause,
                    outcome: wire.outcome,
                    resolution: None,
                    semantic_events: Vec::new(),
                    eligible_witnesses: wire.eligible_witnesses,
                    bundle_hash: wire.bundle_hash,
                    receipt_hash: wire.receipt_hash,
                };
                let computed = receipt.compute_hash().map_err(serde::de::Error::custom)?;
                if receipt.receipt_hash != computed {
                    return Err(serde::de::Error::custom(
                        "authoritative receipt hash mismatch",
                    ));
                }
                Ok(receipt)
            }
        }
    }
}

fn validate_receipt_resolution(receipt: &AuthoritativeReceipt) -> Result<(), String> {
    let Some(resolution) = &receipt.resolution else {
        return Ok(());
    };
    if resolution.action() != receipt.token || resolution.outcome() != &receipt.outcome {
        return Err("authoritative receipt resolution does not match token/outcome".to_owned());
    }
    Ok(())
}

fn validate_receipt_events(receipt: &AuthoritativeReceipt) -> Result<(), String> {
    if receipt.semantic_events.is_empty() {
        return Err("authoritative receipt v3 omits its root semantic event".to_owned());
    }
    if receipt.semantic_events.len() > mush_core::MAX_SEMANTIC_EVENTS as usize {
        return Err(format!(
            "authoritative receipt has {} semantic events; maximum is {}",
            receipt.semantic_events.len(),
            mush_core::MAX_SEMANTIC_EVENTS
        ));
    }
    for (expected, event) in receipt.semantic_events.iter().enumerate() {
        let expected = u32::try_from(expected).expect("semantic event hard limit fits u32");
        if event.cause().ordinal() != expected || event.cause().root_request() != receipt.request {
            return Err(format!(
                "authoritative receipt semantic event {expected} has invalid causal identity"
            ));
        }
        if event.owner() != &receipt.provider {
            return Err(format!(
                "authoritative receipt semantic event {expected} has the wrong owner"
            ));
        }
    }
    let root = &receipt.semantic_events[0];
    if root.cause() != receipt.cause
        || root.truth() != &receipt.outcome
        || root.eligible_witnesses() != receipt.eligible_witnesses
    {
        return Err(
            "authoritative receipt root semantic event does not match its outcome".to_owned(),
        );
    }
    Ok(())
}

/// Result of atomically checkpointing one finalized action and releasing its
/// full replay intent.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthoritativeCheckpointReceipt {
    pub receipt: AuthoritativeReceipt,
    pub released_intent_bytes: usize,
    /// True when the compact checkpoint witness already existed.
    pub resumed: bool,
}

/// Compact immutable witness that a never-finalized authoritative intent was
/// released only after its complete bundle entered the independent
/// quarantine spool. It consumes the original idempotency identity without
/// claiming that the action committed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoritativeQuarantineReceipt {
    schema_version: u16,
    token: CommitToken,
    idempotency: ScopedIdempotencyKey,
    ingress_hash: CanonicalHash,
    bundle_hash: CanonicalHash,
    evidence_hash: CanonicalHash,
}

impl AuthoritativeQuarantineReceipt {
    pub const SCHEMA_VERSION: u16 = 1;

    #[must_use]
    pub fn new(bundle: &AuthoritativeBundle, evidence_hash: CanonicalHash) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            token: bundle.token(),
            idempotency: bundle.idempotency().clone(),
            ingress_hash: bundle.ingress_hash(),
            bundle_hash: bundle.bundle_hash(),
            evidence_hash,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), StoreError> {
        if self.schema_version != Self::SCHEMA_VERSION {
            return Err(StoreError::AuthoritativeCorrupt(format!(
                "unsupported authoritative quarantine receipt schema {}",
                self.schema_version
            )));
        }
        Ok(())
    }

    #[must_use]
    pub const fn token(&self) -> CommitToken {
        self.token
    }

    #[must_use]
    pub const fn idempotency(&self) -> &ScopedIdempotencyKey {
        &self.idempotency
    }

    #[must_use]
    pub const fn ingress_hash(&self) -> CanonicalHash {
        self.ingress_hash
    }

    #[must_use]
    pub const fn bundle_hash(&self) -> CanonicalHash {
        self.bundle_hash
    }

    #[must_use]
    pub const fn evidence_hash(&self) -> CanonicalHash {
        self.evidence_hash
    }

    #[must_use]
    pub fn matches_bundle(&self, bundle: &AuthoritativeBundle) -> bool {
        self.token == bundle.token()
            && self.idempotency == *bundle.idempotency()
            && self.ingress_hash == bundle.ingress_hash()
            && self.bundle_hash == bundle.bundle_hash()
    }
}

/// Result of atomically replacing a poisoned, never-finalized full intent
/// with its compact quarantine tombstone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthoritativeDiscardReceipt {
    pub quarantine: AuthoritativeQuarantineReceipt,
    pub released_intent_bytes: usize,
    pub resumed: bool,
}

/// Durable watermark-first retirement of complete idempotency epochs.
///
/// Once this succeeds, keys in `scope` below `minimum_accepted_epoch` can
/// never be reserved or observed as vacant again. `retired_tombstones` counts
/// compact receipt/binding pairs removed during this call; a retry may report
/// zero after the watermark and cleanup already completed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthoritativeEpochRetirement {
    pub scope: IdempotencyScope,
    pub previous_minimum_epoch: u64,
    pub minimum_accepted_epoch: u64,
    pub retired_tombstones: usize,
    pub resumed: bool,
}

/// Exact status for one token and expected bundle hash.
#[derive(Clone, Debug, PartialEq)]
pub enum BundleStatus {
    /// The complete scoped identity is below its durable retry watermark.
    /// It must never be interpreted as an absent/new action.
    Retired {
        minimum_accepted_epoch: u64,
    },
    Absent,
    PendingExact,
    FinalizedExact(Box<AuthoritativeReceipt>),
    QuarantinedExact(Box<AuthoritativeQuarantineReceipt>),
    Conflict {
        stored_bundle_hash: CanonicalHash,
    },
}

/// Result of the pre-parse `(scoped key, ingress hash)` lookup.
#[derive(Clone, Debug, PartialEq)]
pub enum IdempotencyStatus {
    /// The caller's complete epoch is outside the durable retry horizon. It
    /// must obtain a new epoch/key; treating this as vacant would permit a
    /// duplicate action after tombstone deletion.
    Retired {
        minimum_accepted_epoch: u64,
    },
    Vacant,
    PendingExact {
        token: CommitToken,
        bundle_hash: CanonicalHash,
    },
    FinalizedExact(Box<AuthoritativeReceipt>),
    /// The key was consumed by an intent that never finalized and whose full
    /// recovery material is held by the independent quarantine spool.
    QuarantinedExact(Box<AuthoritativeQuarantineReceipt>),
    /// The scoped key exists but was bound to different ingress bytes.
    Conflict,
}

/// Status of an already-bound idempotency key when the authenticated caller
/// is recovering presentation and therefore has no private ingress hash.
///
/// This lookup must only be exposed through a caller-authenticating service.
/// The store returns the same validated receipt/tombstone material as the
/// early coordinator lookup, but cannot report a caller-supplied conflict
/// because no alternate ingress is accepted on this path.
#[derive(Clone, Debug, PartialEq)]
pub enum BoundIdempotencyStatus {
    Retired {
        minimum_accepted_epoch: u64,
    },
    Vacant,
    PendingExact {
        token: CommitToken,
        bundle_hash: CanonicalHash,
    },
    FinalizedExact(Box<AuthoritativeReceipt>),
    QuarantinedExact(Box<AuthoritativeQuarantineReceipt>),
}

/// One character linked to an account (dbref + name denormalized at link
/// time, mirroring `mushd::accounts::CharSummary`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterRef {
    /// The character's object id.
    pub id: ObjId,
    /// The character's name when it was linked.
    pub name: String,
}

/// One SSH public key authorized on an account (`@sshkey/add`).
///
/// `openssh` is the normalized `"type base64 comment"` line, kept for
/// display in `@sshkey/list`. `fingerprint` is the `"SHA256:..."` digest —
/// a stable, comment-independent identity used to compare offered keys and
/// to target `@sshkey/del` by fingerprint. All three fields are public
/// (there is no secret in a public key), so `AccountRec`'s `Debug` prints
/// them unredacted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshKeyRec {
    /// The normalized OpenSSH public-key line (`type base64 comment`).
    pub openssh: String,
    /// The `SHA256:...` fingerprint (comment-independent identity).
    pub fingerprint: String,
    /// Unix seconds when the key was added.
    pub added: i64,
}

/// The verified Discord identity bound to an account (`@discord/link`,
/// D-D-2). At most one per account and one account per Discord user; the
/// uniqueness lock is the store's reverse index
/// ([`crate::ObjectStore::discord_user_reserve`]).
///
/// `user_id` is Discord's immutable **snowflake** — never the changeable
/// handle — and it is staff-only data (`@discord/show`, behind
/// `Privileges::DISCORD_IDENTITY`): `Debug` redacts it and `dm_channel`, so a
/// stray `{:?}` can never put an identity in a log.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscordLinkRec {
    /// The Discord user's snowflake.
    pub user_id: u64,
    /// Unix seconds when the link was made.
    pub linked_at: i64,
    /// The character that speaks in relayed channels when none is attached
    /// (D-D-5). `None` falls back to the account's single or most recently
    /// connected character.
    #[serde(default)]
    pub relay_character: Option<ObjId>,
    /// Attach a Discord follow when the last terminal/web connection drops
    /// (D-D-4).
    #[serde(default)]
    pub auto_follow: bool,
    /// The DM channel the bot opened with this user, once known.
    #[serde(default)]
    pub dm_channel: Option<u64>,
    /// Quiet follows (D-D-7): the DM carries only notices, requests and the
    /// character's own scene, not ambient room traffic.
    #[serde(default)]
    pub quiet: bool,
}

impl std::fmt::Debug for DiscordLinkRec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscordLinkRec")
            .field("user_id", &"<redacted>")
            .field("linked_at", &self.linked_at)
            .field("relay_character", &self.relay_character)
            .field("auto_follow", &self.auto_follow)
            .field("dm_channel", &self.dm_channel.map(|_| "<redacted>"))
            .finish()
    }
}

/// One account document (`account:{lowername}` in Redis).
///
/// The store holds the argon2id PHC hash opaquely; hashing and verification
/// stay caller-side (fetch via [`crate::ObjectStore::get_account`], verify
/// with the `argon2` crate).
///
/// `Debug` is hand-written to **redact the hash**, so a stray `{:?}` in a
/// log line can never leak PHC strings.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountRec {
    /// The immutable, opaque account identity (E7 cut 5 B1). `None` on a
    /// pre-id document (`#[serde(default)]`); a one-shot boot back-fill in
    /// `mush-server` mints it from `(lowercased name, created)` and is
    /// idempotent. It is never re-derived from the current name, so it is
    /// stable across a rename. The account-distinctness oracle
    /// (`mush_core::AccountIdentity`) answers `None` for an account whose id
    /// is still unminted, and every calling policy fails closed.
    #[serde(default)]
    pub id: Option<mush_core::AccountId>,
    /// Display-case account name. The storage key is the lowercase form;
    /// lookups are case-insensitive.
    pub name: String,
    /// argon2id PHC string.
    pub argon2_hash: String,
    /// Unix seconds at creation.
    pub created: i64,
    /// Characters linked to this account.
    #[serde(default)]
    pub characters: Vec<CharacterRef>,
    /// SSH public keys authorized to log in to this account.
    #[serde(default)]
    pub ssh_keys: Vec<SshKeyRec>,
    /// The verified Discord identity, when linked (D1). Additive: accounts
    /// whose stored doc predates the field deserialize as unlinked.
    #[serde(default)]
    pub discord: Option<DiscordLinkRec>,
}

impl std::fmt::Debug for AccountRec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountRec")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("argon2_hash", &"<redacted>")
            .field("created", &self.created)
            .field("characters", &self.characters)
            .field("ssh_keys", &self.ssh_keys)
            // Linked or not — never the identity (D-D-14).
            .field("discord", &self.discord.as_ref().map(|_| "<linked>"))
            .finish()
    }
}

/// One persisted web-session token (`token:{hash}` in Redis, expiring via
/// native `EXAT` TTL).
///
/// The storage key is the **SHA-256 hex digest of the token**, never the
/// token itself — a leaked store dump cannot be replayed as sessions. The
/// record carries only identity + lifetime facts; sliding-TTL refresh goes
/// through [`crate::ObjectStore::token_touch`], which updates `expires` and
/// the store-side TTL together.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenRecord {
    /// Canonical account name the token authenticates.
    pub account: String,
    /// Unix seconds when the token was minted.
    pub created: i64,
    /// Unix seconds when the token lapses. A record with
    /// `expires <= now` is treated as gone everywhere.
    pub expires: i64,
}

/// One pending Discord link code (`discord:code:{hash}` in Redis, expiring
/// via native `EXAT` TTL; D-D-3).
///
/// The key is the **SHA-256 hex digest of the code** the player was shown
/// in game; the raw code is never persisted. A record is consumed exactly
/// once ([`crate::ObjectStore::discord_code_take`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscordCodeRecord {
    /// Canonical account name the code links.
    pub account: String,
    /// Unix seconds when the code lapses; `expires <= now` is gone.
    pub expires: i64,
}

/// One Discord-scoped refusal (`discord:ban:{user_id}` in Redis; `@discord/ban`).
/// Keyed by the snowflake so it survives an unlink. `Debug` redacts the
/// identity.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscordBanRec {
    /// The banned Discord user's snowflake.
    pub user_id: u64,
    /// Canonical account name of the staffer who set it.
    pub by_account: String,
    /// Staff-entered reason.
    pub reason: String,
    /// Unix seconds when it was set.
    pub at: i64,
}

impl std::fmt::Debug for DiscordBanRec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscordBanRec")
            .field("user_id", &"<redacted>")
            .field("by_account", &self.by_account)
            .field("reason", &self.reason)
            .field("at", &self.at)
            .finish()
    }
}

/// Validate a token hash key: non-empty. (The callers store SHA-256 hex —
/// 64 chars — but the store does not police the digest format, only that a
/// key exists to address.)
pub(crate) fn validate_token_hash(hash: &str) -> Result<(), StoreError> {
    if hash.is_empty() {
        return Err(StoreError::Invalid("empty token hash".into()));
    }
    Ok(())
}

/// One persisted siteban (`ban:{ip}` in Redis, expiring via native TTL).
/// `IpGuard` tempbans stay in-memory; these are the staff-set persistent
/// bans that survive restarts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BanRec {
    /// The banned address (IPv4 or IPv6 text form).
    pub ip: String,
    /// Unix seconds when the ban lapses; `None` = permanent.
    pub until_unix: Option<i64>,
    /// Staff-entered reason.
    pub reason: String,
}

/// Everything that can go wrong in the persistence layer.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Account reservation lost: an account with that name already exists.
    #[error("an account with that name already exists")]
    AccountExists,
    /// The named account does not exist.
    #[error("no such account: {0:?}")]
    NoSuchAccount(String),
    /// Caller handed the store something unusable (empty name, ...).
    #[error("invalid input: {0}")]
    Invalid(String),
    /// The persisted data is inconsistent (objects without `meta:next_id`,
    /// unparseable required doc, ...). Individual corrupt `obj:*` docs are
    /// tolerated and logged, not errors — this is for damage that makes the
    /// store unusable.
    #[error("store data corrupt: {0}")]
    Corrupt(String),
    /// JSON (de)serialization failed.
    #[error("serialization failed: {0}")]
    Serde(#[from] serde_json::Error),
    /// The Redis client reported an error (connection, command, `EXEC`).
    #[error("redis: {0}")]
    Redis(#[from] fred::error::Error),
    /// Migration bookkeeping is broken (unsorted set, store newer than
    /// binary, ...).
    #[error("migration error: {0}")]
    Migration(String),
    /// The store has been [`crate::ObjectStore::close`]d.
    #[error("store is closed")]
    Closed,
    /// The ledger archive tier (J5) failed: an I/O error, or a copy that did
    /// not read back as written.
    #[error("ledger archive: {0}")]
    Archive(String),
    /// One commit token was already bound to different immutable bytes.
    #[error("authoritative commit token is already bound to a different bundle")]
    AuthoritativeTokenConflict,
    /// One scoped idempotency key was already bound to a different request.
    #[error("authoritative idempotency key is already bound to a different request")]
    AuthoritativeIdempotencyConflict,
    /// Reconciliation was requested for a token with no durable intent.
    #[error("authoritative commit token has no durable intent")]
    AuthoritativeIntentMissing,
    /// Checkpoint compaction was requested before exact finalization.
    #[error("authoritative commit token is not finalized")]
    AuthoritativeNotFinalized,
    /// Full replay material remains necessary until all durable outbox work
    /// for the action reaches a terminal state.
    #[error("authoritative commit has unresolved durable outbox work")]
    AuthoritativeOutboxPending,
    /// A stable outbox item does not exist.
    #[error("authoritative durable-outbox item does not exist")]
    AuthoritativeOutboxMissing,
    /// The requested outbox transition did not match the exact current item.
    #[error("authoritative durable-outbox item or attempt conflicts with durable state")]
    AuthoritativeOutboxConflict,
    /// The durable attempt counter cannot advance further.
    #[error("authoritative durable-outbox attempt counter is exhausted")]
    AuthoritativeOutboxAttemptsExhausted,
    /// Finalizing another action would exceed the global pending-delivery cap.
    #[error(
        "authoritative durable-outbox capacity exhausted: {pending}/{max_pending} pending items"
    )]
    AuthoritativeOutboxCapacity { pending: usize, max_pending: usize },
    /// Reservation raced with or followed durable retirement of this epoch.
    #[error(
        "authoritative idempotency epoch {requested_epoch} is retired; minimum accepted epoch is {minimum_accepted_epoch}"
    )]
    AuthoritativeEpochRetired {
        requested_epoch: u64,
        minimum_accepted_epoch: u64,
    },
    /// A minimum epoch may only move forward.
    #[error(
        "authoritative minimum epoch cannot regress from {current_minimum_epoch} to {requested_minimum_epoch}"
    )]
    AuthoritativeEpochRegression {
        current_minimum_epoch: u64,
        requested_minimum_epoch: u64,
    },
    /// Full replay custody still exists below the proposed watermark.
    #[error(
        "authoritative epoch retirement is blocked by {retained_intents} retained intents below epoch {requested_minimum_epoch}"
    )]
    AuthoritativeEpochPending {
        requested_minimum_epoch: u64,
        retained_intents: usize,
    },
    /// Concurrent reservations/checkpoints repeatedly changed the retained
    /// set while retirement was proving it empty. Retrying is safe.
    #[error("authoritative epoch retirement raced with active commits")]
    AuthoritativeEpochContention,
    /// A stored authoritative intent or finalized witness failed strict decode.
    #[error("authoritative persistence record is corrupt: {0}")]
    AuthoritativeCorrupt(String),
    /// Retaining another exact intent would exceed a hard bound.
    #[error(
        "authoritative retained-intent capacity exhausted: {retained_count}/{max_count} records, {retained_bytes}/{max_bytes} bytes"
    )]
    AuthoritativeCapacity {
        retained_count: usize,
        retained_bytes: usize,
        max_count: usize,
        max_bytes: usize,
    },
    /// The initial K3 contract permits only one unresolved reservation. The
    /// store enforces this independently of coordinator serialization so two
    /// callers cannot reserve the same fresh object id concurrently.
    #[error("another authoritative intent is unresolved ({pending} pending)")]
    AuthoritativeIntentInFlight { pending: usize },
    /// A retry addressed an intent that was explicitly quarantined and may
    /// not be executed again under the same identity.
    #[error("authoritative intent was quarantined under evidence {evidence_hash}")]
    AuthoritativeIntentQuarantined { evidence_hash: CanonicalHash },
    /// Discard is only valid for an intent that never published a finalized
    /// witness; a committed action must retain repair custody instead.
    #[error("a finalized authoritative intent cannot be discarded")]
    AuthoritativeIntentAlreadyFinalized,
    /// Full-intent membership changed while recovery was exhaustively
    /// scanning it. The stale count/byte observation must not be installed.
    #[error("authoritative retention changed during its repair scan")]
    AuthoritativeRetentionContention,
}

impl StoreError {
    /// Whether retrying the same operation can plausibly succeed.
    ///
    /// Connection-shaped failures (I/O, timeout, protocol hiccups,
    /// reconnect races) and a small allowlist of recoverable Redis server
    /// states are
    /// transient: the write-behind retry loop should keep trying them.
    /// Deterministic failures — serialization bugs, invalid input, corrupt
    /// data, a type-clobbered key (`WRONGTYPE` → `InvalidArgument`), auth
    /// or config errors — will fail identically forever; retrying them
    /// would wedge the writer behind one poisoned batch.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        use fred::error::ErrorKind as K;
        match self {
            StoreError::Redis(e) => {
                // Redis modules and fred report many unrelated `ERR ...`
                // replies as `Unknown`. Treating the whole class as transient
                // lets a novel deterministic module/schema error wedge the
                // FIFO forever. Only explicitly recoverable server states are
                // retryable; unknown text fails closed for diagnosis.
                let message = e.to_string().to_ascii_lowercase();
                let recoverable_unknown = [
                    "oom command not allowed",
                    "loading redis",
                    "tryagain",
                    "clusterdown",
                    "masterdown",
                    "readonly",
                    "busy redis is busy",
                ]
                .iter()
                .any(|needle| message.contains(needle));
                match e.kind() {
                    K::Unknown => recoverable_unknown,
                    K::InvalidArgument
                    | K::InvalidCommand
                    | K::Config
                    | K::Auth
                    | K::Url
                    | K::Parse => false,
                    _ => true,
                }
            }
            StoreError::AccountExists
            | StoreError::NoSuchAccount(_)
            | StoreError::Invalid(_)
            | StoreError::Corrupt(_)
            | StoreError::Serde(_)
            | StoreError::Migration(_)
            | StoreError::Closed
            // The archive's failures are deterministic too: a disk that is
            // full or damaged does not heal by retrying the same write.
            | StoreError::Archive(_)
            | StoreError::AuthoritativeTokenConflict
            | StoreError::AuthoritativeIdempotencyConflict
            | StoreError::AuthoritativeIntentMissing
            | StoreError::AuthoritativeNotFinalized
            | StoreError::AuthoritativeOutboxPending
            | StoreError::AuthoritativeOutboxMissing
            | StoreError::AuthoritativeOutboxConflict
            | StoreError::AuthoritativeOutboxAttemptsExhausted
            | StoreError::AuthoritativeOutboxCapacity { .. }
            | StoreError::AuthoritativeEpochRetired { .. }
            | StoreError::AuthoritativeEpochRegression { .. }
            | StoreError::AuthoritativeEpochPending { .. }
            | StoreError::AuthoritativeCorrupt(_)
            | StoreError::AuthoritativeCapacity { .. }
            | StoreError::AuthoritativeIntentInFlight { .. }
            | StoreError::AuthoritativeIntentQuarantined { .. }
            | StoreError::AuthoritativeIntentAlreadyFinalized => false,
            StoreError::AuthoritativeEpochContention
            | StoreError::AuthoritativeRetentionContention => true,
        }
    }
}

/// The write path's last-resort nesting check on a serialized object doc.
///
/// **This is the poison path, not validation.** Nothing this server builds
/// can reach it. The deepest document it constructs is an object carrying a
/// maximal lock tree, and `mush_core` pins that at 98 JSON levels against a
/// [`mush_core::MAX_DOC_DEPTH`] of 120 — because `parse_lock` already refuses
/// anything over `MAX_LOCK_EXPR_DEPTH` (48) at the one gate where a *player*
/// can be told why. A document that trips this check therefore is not bad
/// input: it is a tree no parser would have built, and the only honest answer
/// is to keep it out of the store rather than persist a document boot cannot
/// read back.
///
/// It is deliberately [`StoreError::Corrupt`], which is non-transient: the
/// write-behind retry loop will not wedge itself on it, and the server latches
/// integrity and freezes the world instead of surfacing it as a refusal a
/// player could provoke. That consequence is exactly why enforcement lives at
/// the parse gate and this is only the backstop under it.
pub(crate) fn refuse_over_deep_object_doc(id: ObjId, doc: &str) -> Result<(), StoreError> {
    let depth = mush_core::depth_of_json_text(doc);
    if depth > mush_core::MAX_DOC_DEPTH {
        return Err(StoreError::Corrupt(format!(
            "object {id} serializes {depth} JSON levels deep, past the limit of {} this store \
             will write",
            mush_core::MAX_DOC_DEPTH
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_rec_debug_redacts_hash() {
        let id = mush_core::AccountId::mint("Nick", 1).expect("mint");
        let rec = AccountRec {
            id: Some(id),
            name: "Nick".into(),
            argon2_hash: "$argon2id$v=19$m=19456,t=2,p=1$secret".into(),
            created: 1,
            characters: vec![],
            ssh_keys: vec![SshKeyRec {
                openssh: "ssh-ed25519 AAAApub nick@box".into(),
                fingerprint: "SHA256:pub".into(),
                added: 2,
            }],
            discord: Some(DiscordLinkRec {
                user_id: 987_654_321_098_765_432,
                linked_at: 3,
                relay_character: None,
                auto_follow: false,
                dm_channel: Some(111_222_333_444_555_666),
                quiet: false,
            }),
        };
        let debug = format!("{rec:?}");
        assert!(!debug.contains("argon2id"), "hash leaked: {debug}");
        assert!(debug.contains("<redacted>"));
        assert!(debug.contains("Nick"));
        // The Discord identity is staff-only data: linked yes/no, never the
        // snowflake or the DM channel (D-D-14).
        assert!(
            !debug.contains("987654321098765432"),
            "snowflake leaked: {debug}"
        );
        assert!(
            !debug.contains("111222333444555666"),
            "dm channel leaked: {debug}"
        );
        assert!(debug.contains("<linked>"), "linked marker missing: {debug}");
        // The account id is redacted too — its hex must never appear.
        assert!(
            !debug.contains(&id.canonical_hash().to_hex()[..8]),
            "account id leaked: {debug}"
        );
        // ssh_keys are public data — printed, not redacted.
        assert!(
            debug.contains("SHA256:pub"),
            "ssh_keys should show: {debug}"
        );
    }

    #[test]
    fn transient_classification() {
        use fred::error::{Error as FredError, ErrorKind};
        // Connection-shaped: keep retrying.
        for kind in [
            ErrorKind::IO,
            ErrorKind::Timeout,
            ErrorKind::Protocol,
            ErrorKind::Canceled,
        ] {
            let msg = format!("{kind:?} must be retried");
            assert!(
                StoreError::Redis(FredError::new(kind, "x")).is_transient(),
                "{msg}"
            );
        }
        for message in [
            "OOM command not allowed when used memory > maxmemory",
            "LOADING Redis is loading the dataset in memory",
            "TRYAGAIN Multiple keys request during rehashing",
            "CLUSTERDOWN The cluster is down",
            "MASTERDOWN Link with MASTER is down",
            "READONLY You can't write against a read only replica",
            "BUSY Redis is busy running a script",
        ] {
            assert!(
                StoreError::Redis(FredError::new(ErrorKind::Unknown, message)).is_transient(),
                "known recoverable server state must retry: {message}"
            );
        }
        assert!(
            !StoreError::Redis(FredError::new(
                ErrorKind::Unknown,
                "ERR arbitrary module validation failure"
            ))
            .is_transient(),
            "unclassified generic Redis errors must fail closed"
        );
        // Deterministic: retrying forever would wedge the writer.
        assert!(
            !StoreError::Redis(FredError::new(ErrorKind::InvalidArgument, "WRONGTYPE"))
                .is_transient()
        );
        for message in [
            "WRONGTYPE Operation against a key holding the wrong kind of value",
            "ERR Existing key has wrong Redis type",
        ] {
            assert!(
                !StoreError::Redis(FredError::new(ErrorKind::Unknown, message)).is_transient(),
                "known wrong-type reply must be deterministic: {message}"
            );
        }
        assert!(!StoreError::Closed.is_transient());
        assert!(!StoreError::Invalid("x".into()).is_transient());
        assert!(!StoreError::Corrupt("x".into()).is_transient());
        let serde_err = serde_json::from_str::<u32>("{").unwrap_err();
        assert!(!StoreError::Serde(serde_err).is_transient());
    }

    #[test]
    fn compact_receipt_preserves_events_and_reads_legacy_v2() {
        let bundle = crate::suite::authoritative_bundle(0xD1, 0xD2);
        let current = AuthoritativeReceipt::new(&bundle);
        assert_eq!(current.schema_version, 3);
        assert_eq!(current.semantic_events(), bundle.semantic_events());
        let round_trip: AuthoritativeReceipt =
            serde_json::from_value(serde_json::to_value(&current).unwrap()).unwrap();
        assert_eq!(round_trip, current);

        let mut legacy = current;
        legacy.schema_version = 2;
        legacy.semantic_events.clear();
        legacy.receipt_hash = legacy.compute_hash().unwrap();
        let encoded = serde_json::to_value(&legacy).unwrap();
        assert!(encoded.get("semantic_events").is_none());
        let decoded: AuthoritativeReceipt = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, legacy);
        assert!(decoded.semantic_events().is_empty());
    }

    #[test]
    fn compact_receipt_retains_resolution_after_full_intent_release() {
        let base = crate::suite::authoritative_bundle(0xE1, 0xE2);
        let domain = mush_core::CanonicalId::new("store_resolution").unwrap();
        let mut rng = mush_core::ResolutionKeyRing::new(
            mush_core::ResolutionSecret::new(1, [0x51; 32]).unwrap(),
            [],
        )
        .unwrap()
        .start(base.idempotency(), &domain, base.request_hash())
        .unwrap();
        rng.draw_bounded_u64(&mush_core::CanonicalId::new("result").unwrap(), 6)
            .unwrap();
        let resolution = rng
            .into_prepared(
                base.definitions().to_vec(),
                RulesValue::new(base.outcome().schema().clone(), json!({ "pool": 3 })).unwrap(),
                base.outcome().clone(),
            )
            .unwrap()
            .bind(base.token());
        let bundle = AuthoritativeBundle::new(mush_core::AuthoritativeBundleDraft {
            token: base.token(),
            request: base.request(),
            idempotency: base.idempotency().clone(),
            ingress_hash: base.ingress_hash(),
            request_hash: base.request_hash(),
            plan_hash: base.plan_hash(),
            action: base.action().clone(),
            provider: base.provider().clone(),
            cause: base.cause(),
            definitions: base.definitions().to_vec(),
            writes: base.writes().to_vec(),
            outcome: base.outcome().clone(),
            resolution: Some(resolution),
            semantic_events: base.semantic_events().to_vec(),
            eligible_witnesses: base.eligible_witnesses().to_vec(),
            durable_outbox: base.durable_outbox().to_vec(),
        })
        .unwrap();
        let receipt = AuthoritativeReceipt::new(&bundle);
        assert_eq!(receipt.schema_version, AuthoritativeReceipt::SCHEMA_VERSION);
        assert_eq!(receipt.resolution(), bundle.resolution());
        assert!(receipt.matches_bundle(&bundle));
        let restored: AuthoritativeReceipt =
            serde_json::from_value(serde_json::to_value(&receipt).unwrap()).unwrap();
        assert_eq!(restored, receipt);
    }
}

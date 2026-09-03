//! Domain types shared by every [`crate::ObjectStore`] implementation.

use mush_core::{
    AuthoritativeBundle, CanonicalHash, CommitToken, MAX_MODULE_KEY_BYTES,
    MAX_MODULE_NAMESPACE_BYTES, Obj, ObjId,
};
use serde::{Deserialize, Serialize};

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
    validate_module_key(key)
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

/// Immutable semantic result witness produced only after exact readback.
///
/// The first implementation retains the complete bundle. K3 compaction may
/// later replace replay payloads with a smaller witness only after a durable
/// checkpoint proves every authoritative post-image.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoritativeReceipt {
    bundle: AuthoritativeBundle,
}

impl AuthoritativeReceipt {
    #[must_use]
    pub const fn new(bundle: AuthoritativeBundle) -> Self {
        Self { bundle }
    }

    #[must_use]
    pub const fn bundle(&self) -> &AuthoritativeBundle {
        &self.bundle
    }

    #[must_use]
    pub const fn token(&self) -> CommitToken {
        self.bundle.token()
    }

    #[must_use]
    pub const fn bundle_hash(&self) -> CanonicalHash {
        self.bundle.bundle_hash()
    }
}

/// Exact status for one token and expected bundle hash.
#[derive(Clone, Debug, PartialEq)]
pub enum BundleStatus {
    Absent,
    PendingExact,
    FinalizedExact(Box<AuthoritativeReceipt>),
    Conflict { stored_bundle_hash: CanonicalHash },
}

/// Result of the pre-parse `(scoped key, ingress hash)` lookup.
#[derive(Clone, Debug, PartialEq)]
pub enum IdempotencyStatus {
    Vacant,
    PendingExact {
        token: CommitToken,
        bundle_hash: CanonicalHash,
    },
    FinalizedExact(Box<AuthoritativeReceipt>),
    /// The scoped key exists but was bound to different ingress bytes.
    Conflict,
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
}

impl std::fmt::Debug for AccountRec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountRec")
            .field("name", &self.name)
            .field("argon2_hash", &"<redacted>")
            .field("created", &self.created)
            .field("characters", &self.characters)
            .field("ssh_keys", &self.ssh_keys)
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
    /// One commit token was already bound to different immutable bytes.
    #[error("authoritative commit token is already bound to a different bundle")]
    AuthoritativeTokenConflict,
    /// One scoped idempotency key was already bound to a different request.
    #[error("authoritative idempotency key is already bound to a different request")]
    AuthoritativeIdempotencyConflict,
    /// Reconciliation was requested for a token with no durable intent.
    #[error("authoritative commit token has no durable intent")]
    AuthoritativeIntentMissing,
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
            | StoreError::AuthoritativeTokenConflict
            | StoreError::AuthoritativeIdempotencyConflict
            | StoreError::AuthoritativeIntentMissing
            | StoreError::AuthoritativeCorrupt(_)
            | StoreError::AuthoritativeCapacity { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_rec_debug_redacts_hash() {
        let rec = AccountRec {
            name: "Nick".into(),
            argon2_hash: "$argon2id$v=19$m=19456,t=2,p=1$secret".into(),
            created: 1,
            characters: vec![],
            ssh_keys: vec![SshKeyRec {
                openssh: "ssh-ed25519 AAAApub nick@box".into(),
                fingerprint: "SHA256:pub".into(),
                added: 2,
            }],
        };
        let debug = format!("{rec:?}");
        assert!(!debug.contains("argon2id"), "hash leaked: {debug}");
        assert!(debug.contains("<redacted>"));
        assert!(debug.contains("Nick"));
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
}

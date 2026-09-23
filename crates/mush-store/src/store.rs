//! The [`ObjectStore`] trait — the seam between the game and its storage.
//!
//! ## Why `BoxFuture` instead of native `async fn` in the trait
//!
//! The behavior test suite and `mushd` both work with `dyn ObjectStore`
//! (one code path over the Redis store and the in-memory fake). Native
//! `async fn` in traits is not dyn-compatible in Rust 2024, so every method
//! returns a [`BoxFuture`] explicitly. The one boxed allocation per call is
//! noise next to a network round trip, and implementations still just write
//! `async` blocks.
//!
//! ## Concurrency contract
//!
//! - The **world task is the only writer of world state**: [`write`] is
//!   called from exactly one task (the dedicated store-writer), one batch
//!   at a time, each awaited to its ack before the next is sent — batches
//!   are never concurrent or reordered. The world task hands batches to
//!   that writer through a small bounded queue immediately per command
//!   (write-through per-command; no timer batching anywhere), so on a hard
//!   kill the loss window is the writer's queue — at most a handful of
//!   already-confirmed commands while the store is stalled, and just the
//!   in-flight command in the steady state where the queue is empty.
//!   Durability-critical flows (account/character creation) additionally
//!   await the ack before confirming, closing even that window.
//! - Account, ban, and meta methods may additionally be called from a small
//!   number of other tasks (the auth service reads account docs directly;
//!   staff tooling reads bans). Each call is individually atomic; the store
//!   guarantees **no ordering between concurrent calls** beyond that.
//!   Cross-call invariants (e.g. "create the Player object, then link it")
//!   are the caller's protocol, exactly as in M1.
//! - [`load_world`] runs once at boot, before the world task starts writing.
//!
//! [`write`]: ObjectStore::write
//! [`load_world`]: ObjectStore::load_world

use futures::future::BoxFuture;
use mush_core::{
    AuthoritativeBundle, CanonicalHash, CommitToken, IdempotencyScope, ObjId, ScopedIdempotencyKey,
};

use crate::types::{
    AccountRec, AuthoritativeCheckpointReceipt, AuthoritativeDiscardReceipt,
    AuthoritativeEpochRetirement, AuthoritativeIntentPage, AuthoritativeOutboxAttemptResult,
    AuthoritativeOutboxRecord, AuthoritativeReceipt, AuthoritativeRetentionUsage,
    AuthoritativeScanCursor, BanRec, BoundIdempotencyStatus, BundleStatus, DiscordBanRec,
    DiscordCodeRecord, DiscordLinkRec, IdempotencyStatus, IntentReceipt,
    PendingAuthoritativeIntent, SshKeyRec, StoreError, TokenRecord, WorldSnapshot, WriteBatch,
    WriteReceipt,
};

/// Exact-intent persistence used by the K3 action coordinator.
///
/// This is separate from [`ObjectStore`] while the in-memory contract and
/// fault matrix are being proven. A backend must not claim this trait by
/// routing bundles through legacy [`ObjectStore::write`].
pub trait AuthoritativeStore: ObjectStore {
    /// Reserve a complete immutable intent and its idempotency binding before
    /// any replay write can materialize.
    fn reserve_authoritative_intent(
        &self,
        bundle: AuthoritativeBundle,
    ) -> BoxFuture<'_, Result<IntentReceipt, StoreError>>;

    /// Return/join a prior request before parsing, resolution, planning, or
    /// RNG. A conflicting ingress hash never exposes the stored result.
    fn authoritative_idempotency_status<'a>(
        &'a self,
        key: &'a ScopedIdempotencyKey,
        ingress_hash: CanonicalHash,
    ) -> BoxFuture<'a, Result<IdempotencyStatus, StoreError>>;

    /// Recover a previously bound request without requiring its private
    /// ingress hash. Callers MUST authenticate the principal and reconstruct
    /// the exact scoped key before invoking this method.
    fn bound_authoritative_idempotency_status<'a>(
        &'a self,
        key: &'a ScopedIdempotencyKey,
    ) -> BoxFuture<'a, Result<BoundIdempotencyStatus, StoreError>>;

    /// Reapply exact post-images, verify them, and atomically publish the
    /// finalized witness for one expected token/hash pair.
    fn reconcile_authoritative(
        &self,
        token: CommitToken,
        expected_bundle_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<AuthoritativeReceipt, StoreError>>;

    /// Atomically replace a finalized action's full replay intent with its
    /// compact, hash-verified checkpoint witness. The idempotency binding and
    /// semantic result remain available for exact retries. Pending actions or
    /// actions with unresolved durable-outbox work are never compacted.
    fn checkpoint_authoritative(
        &self,
        token: CommitToken,
        expected_bundle_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<AuthoritativeCheckpointReceipt, StoreError>>;

    /// Release a never-finalized poisoned intent after the caller has durably
    /// preserved the complete bundle in the independent quarantine spool.
    /// The compact tombstone and idempotency binding remain, so neither exact
    /// nor conflicting retries can execute the discarded action.
    fn discard_quarantined_authoritative_intent(
        &self,
        token: CommitToken,
        expected_bundle_hash: CanonicalHash,
        evidence_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<AuthoritativeDiscardReceipt, StoreError>>;

    /// Restore the exact full intent from independently verified quarantine
    /// custody so recovery can retry it after the underlying fault is fixed.
    /// This is the sole transition that makes a quarantine tombstone pending
    /// again and it requires the exact evidence identity.
    fn restore_quarantined_authoritative_intent(
        &self,
        bundle: AuthoritativeBundle,
        evidence_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<IntentReceipt, StoreError>>;

    /// Discover every finalized, non-terminal durable delivery item. Stable
    /// item ids are suitable for adapter-side deduplication.
    fn pending_authoritative_outbox(
        &self,
    ) -> BoxFuture<'_, Result<Vec<AuthoritativeOutboxRecord>, StoreError>>;

    /// Persist an attempt number before calling the external adapter. Exact
    /// retries use `expected_attempts` to join an acknowledgement-lost start.
    fn begin_authoritative_outbox_attempt(
        &self,
        item_id: CanonicalHash,
        expected_payload_hash: CanonicalHash,
        expected_attempts: u32,
    ) -> BoxFuture<'_, Result<AuthoritativeOutboxRecord, StoreError>>;

    /// Persist the result of one already-started delivery attempt. `Retry`
    /// remains independently discoverable; terminal states leave the index.
    fn complete_authoritative_outbox_attempt(
        &self,
        item_id: CanonicalHash,
        expected_payload_hash: CanonicalHash,
        expected_attempt: u32,
        result: AuthoritativeOutboxAttemptResult,
    ) -> BoxFuture<'_, Result<AuthoritativeOutboxRecord, StoreError>>;

    /// Persist one bounded discovery pass for which no adapter could route
    /// this item. This counter is independent of real delivery attempts and
    /// lets the dispatcher eventually dead-letter permanently unroutable work
    /// without penalizing an adapter that appears later.
    fn note_authoritative_outbox_unroutable(
        &self,
        item_id: CanonicalHash,
        expected_payload_hash: CanonicalHash,
        expected_passes: u32,
        error: String,
    ) -> BoxFuture<'_, Result<AuthoritativeOutboxRecord, StoreError>>;

    /// Advance one idempotency scope's durable retry horizon and remove every
    /// compact receipt/binding below it. Implementations must refuse while a
    /// full intent remains below the proposed watermark. Redis may commit the
    /// watermark before finishing bounded tombstone cleanup; after an error,
    /// retrying the same minimum is required and safe.
    fn retire_authoritative_epochs<'a>(
        &'a self,
        scope: &'a IdempotencyScope,
        minimum_accepted_epoch: u64,
    ) -> BoxFuture<'a, Result<AuthoritativeEpochRetirement, StoreError>>;

    /// Inspect only an exact expected token/hash pair.
    fn authoritative_status(
        &self,
        token: CommitToken,
        expected_bundle_hash: CanonicalHash,
    ) -> BoxFuture<'_, Result<BundleStatus, StoreError>>;

    /// Inspect an exact complete bundle identity. Unlike the legacy
    /// token/hash-only lookup, this can consult the scoped epoch watermark and
    /// therefore distinguish a retired action from one that never existed.
    fn authoritative_bundle_status<'a>(
        &'a self,
        bundle: &'a AuthoritativeBundle,
    ) -> BoxFuture<'a, Result<BundleStatus, StoreError>> {
        Box::pin(async move {
            let direct = self
                .authoritative_status(bundle.token(), bundle.bundle_hash())
                .await?;
            if !matches!(direct, BundleStatus::Absent) {
                return Ok(direct);
            }
            Ok(
                match self
                    .authoritative_idempotency_status(bundle.idempotency(), bundle.ingress_hash())
                    .await?
                {
                    IdempotencyStatus::Retired {
                        minimum_accepted_epoch,
                    } => BundleStatus::Retired {
                        minimum_accepted_epoch,
                    },
                    IdempotencyStatus::Vacant => BundleStatus::Absent,
                    IdempotencyStatus::Conflict => {
                        return Err(StoreError::AuthoritativeIdempotencyConflict);
                    }
                    IdempotencyStatus::PendingExact { token, bundle_hash }
                        if token == bundle.token() && bundle_hash == bundle.bundle_hash() =>
                    {
                        BundleStatus::PendingExact
                    }
                    IdempotencyStatus::PendingExact { bundle_hash, .. } => BundleStatus::Conflict {
                        stored_bundle_hash: bundle_hash,
                    },
                    IdempotencyStatus::FinalizedExact(receipt)
                        if receipt.token() == bundle.token()
                            && receipt.bundle_hash() == bundle.bundle_hash() =>
                    {
                        BundleStatus::FinalizedExact(receipt)
                    }
                    IdempotencyStatus::FinalizedExact(receipt) => BundleStatus::Conflict {
                        stored_bundle_hash: receipt.bundle_hash(),
                    },
                    IdempotencyStatus::QuarantinedExact(receipt)
                        if receipt.token() == bundle.token()
                            && receipt.bundle_hash() == bundle.bundle_hash() =>
                    {
                        BundleStatus::QuarantinedExact(receipt)
                    }
                    IdempotencyStatus::QuarantinedExact(receipt) => BundleStatus::Conflict {
                        stored_bundle_hash: receipt.bundle_hash(),
                    },
                },
            )
        })
    }

    /// Read the bounded recovery accelerator. This index is never the only
    /// discovery source because its last update may have been interrupted.
    fn pending_authoritative_intents(
        &self,
    ) -> BoxFuture<'_, Result<Vec<PendingAuthoritativeIntent>, StoreError>>;

    /// Scan the complete retained intent keyspace in bounded pages. Boot must
    /// continue until `next_cursor` is `None`, even after the pending index is
    /// empty, and reconcile every pending record found by either source.
    fn scan_authoritative_intents(
        &self,
        cursor: Option<AuthoritativeScanCursor>,
        limit: usize,
    ) -> BoxFuture<'_, Result<AuthoritativeIntentPage, StoreError>>;

    /// Read the generation that changes whenever full-intent retention gains
    /// or releases a record. Recovery captures this before its exhaustive
    /// scan and supplies it to the repair CAS.
    fn authoritative_retention_generation(&self) -> BoxFuture<'_, Result<u64, StoreError>>;

    /// Repair the non-authoritative count/byte capacity indexes from a
    /// complete validated intent scan. Backends replace the pair only if the
    /// retained generation still equals the pre-scan witness, and return
    /// `true` only when either counter changed.
    fn repair_authoritative_retention(
        &self,
        expected_generation: u64,
        usage: AuthoritativeRetentionUsage,
    ) -> BoxFuture<'_, Result<bool, StoreError>>;
}

/// A module namespace's documents as boot consumes them (J1): see
/// [`ObjectStore::module_docs`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModuleDocs {
    /// Every document that is not a ledger chunk, decoded; keys are
    /// namespace-relative.
    pub docs: Vec<(String, serde_json::Value)>,
    /// Every ledger chunk document (`log:{id}:{chunk}`, and any other
    /// `log:`-prefixed key — a malformed one is the inspection's to report)
    /// as raw JSON text.
    pub ledger: Vec<(String, String)>,
    /// J4 (doc 12 §5.6): the floors this listing was narrowed by — per log
    /// id, the chunk index below which no chunk document was fetched (primary
    /// or archive). Empty for a whole listing.
    pub omitted: std::collections::BTreeMap<String, u64>,
    /// Doc 13 W1: the key prefixes this listing fetched nothing under.
    pub skipped_prefixes: Vec<String>,
}

/// Per ledger log id, the chunk index below which a boot listing need not
/// fetch (J4, doc 12 §5.6): the module's boot manifest, as the loader read it.
pub type ChunkFloors = std::collections::BTreeMap<String, u64>;

/// What a boot listing need not fetch (J4 + doc 13 W1): ledger chunks below a
/// per-log floor, and every key under a prefix — the module's boot manifest,
/// as the loader read it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BootSkip {
    /// Per log id, chunks below this index are not fetched.
    pub floors: ChunkFloors,
    /// Keys starting with any of these are not fetched.
    pub prefixes: Vec<String>,
}

impl BootSkip {
    /// Whether `key` is one this listing does not fetch: a `log:{id}:{n}` key
    /// below its log's floor, or any key under a skipped prefix.
    #[must_use]
    pub fn skips(&self, key: &str) -> bool {
        if self
            .prefixes
            .iter()
            .any(|p| !p.is_empty() && key.starts_with(p.as_str()))
        {
            return true;
        }
        below_floor(key, &self.floors)
    }

    /// Nothing to skip.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.floors.is_empty() && self.prefixes.is_empty()
    }
}

/// No skip: the whole listing (a `static`, so a backend's whole listing can
/// borrow it for the future it returns).
pub static NO_SKIP: BootSkip = BootSkip {
    floors: ChunkFloors::new(),
    prefixes: Vec::new(),
};

/// Whether a `log:{id}:{n}` key falls below its log's floor — the one test a
/// narrowed listing applies, to primary keys and archived chunks alike.
#[must_use]
pub fn below_floor(key: &str, floors: &ChunkFloors) -> bool {
    if floors.is_empty() {
        return false;
    }
    let Some((log_id, chunk)) = mush_core::parse_chunk_key(key) else {
        return false;
    };
    floors
        .get(log_id.as_str())
        .is_some_and(|floor| chunk < *floor)
}

impl ModuleDocs {
    /// Split a decoded listing: ledger chunks are re-encoded to compact text.
    #[must_use]
    pub fn split(listed: Vec<(String, serde_json::Value)>) -> ModuleDocs {
        let mut out = ModuleDocs::default();
        for (key, value) in listed {
            if key.starts_with(mush_core::ledger::LOG_PREFIX) {
                let text =
                    serde_json::to_string(&value).expect("a serde_json::Value always serializes");
                out.ledger.push((key, text));
            } else {
                out.docs.push((key, value));
            }
        }
        out
    }

    /// The flat decoded listing [`ObjectStore::module_json_list`] promises,
    /// from a split one.
    ///
    /// # Errors
    /// A ledger document that is not JSON at all.
    pub fn into_json_list(self) -> Result<Vec<(String, serde_json::Value)>, StoreError> {
        let mut out = self.docs;
        for (key, text) in self.ledger {
            out.push((key, serde_json::from_str(&text)?));
        }
        Ok(out)
    }
}

/// Persistent storage for the world, accounts, sitebans, and store metadata.
///
/// Implementations: [`crate::RedisStore`] (authoritative, Redis 8) and
/// [`crate::InMemoryStore`] (tests and `mushd --repl`). Both honor the exact
/// same semantics; the shared behavior suite in [`crate::suite`] proves it.
pub trait ObjectStore: Send + Sync {
    /// Expose the exact-token durability capability when this backend
    /// implements it.
    ///
    /// Server boot refuses writable service when this returns `None`. The
    /// optional shape keeps narrow legacy test doubles usable while K3 is
    /// migrating callers away from the old write-only surface; production
    /// backends must return `Some(self)`.
    fn authoritative_store(&self) -> Option<&dyn AuthoritativeStore> {
        None
    }

    /// Perform backend-owned writable bootstrap after authoritative-intent
    /// recovery has completed.
    ///
    /// Redis uses this for search-index creation. Keeping it separate from
    /// connection establishment lets server boot open a read-capable handle,
    /// reconcile partial authoritative materializations, and only then permit
    /// ordinary initialization writes.
    fn bootstrap_writable(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async { Ok(()) })
    }

    /// Load every persisted object plus `next_id` and `schema_version`.
    ///
    /// Returns `Ok(None)` for a **completely empty store** (no objects *and*
    /// no `next_id`) so the caller can seed a fresh world. A store with
    /// objects but no `next_id` is corrupt ([`StoreError::Corrupt`]) — ids
    /// are never recycled, so the allocator resume point must exist.
    /// Individual corrupt object docs are logged, omitted from `objects`, and
    /// counted in [`WorldSnapshot::corrupt_docs`] rather than panicking. An
    /// authoritative server must refuse a snapshot with a non-zero count.
    fn load_world(&self) -> BoxFuture<'_, Result<Option<WorldSnapshot>, StoreError>>;

    /// Apply one [`WriteBatch`] atomically (see its docs for the exact
    /// semantics). Returns once the batch is **acknowledged by the store**;
    /// for Redis that means the `MULTI`/`EXEC` transaction committed.
    /// Callers gate durability-critical confirmation output on this `Ok`.
    fn write(&self, batch: WriteBatch) -> BoxFuture<'_, Result<WriteReceipt, StoreError>>;

    // ------------------------------------------------------------------
    // Accounts
    // ------------------------------------------------------------------

    /// Atomically reserve and store a new account. Fails with
    /// [`StoreError::AccountExists`] if any account already holds the name
    /// (case-insensitive) — under concurrent creates exactly one wins.
    fn create_account(&self, rec: AccountRec) -> BoxFuture<'_, Result<(), StoreError>>;

    /// Create the first account only when no account exists, atomically with
    /// respect to every account creator. Used by the offline staff bootstrap;
    /// even an unreadable existing account must refuse with
    /// [`StoreError::AccountExists`]. Backends without this capability fail
    /// closed rather than approximating it with a scan followed by a write.
    fn create_first_account(&self, _rec: AccountRec) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async {
            Err(StoreError::Invalid(
                "atomic first-account creation is unsupported".into(),
            ))
        })
    }

    /// Fetch an account by name (case-insensitive). `Ok(None)` when absent.
    /// This is the verify path: the caller checks the password against
    /// [`AccountRec::argon2_hash`] itself — hashing never enters the store.
    fn get_account<'a>(
        &'a self,
        name: &'a str,
    ) -> BoxFuture<'a, Result<Option<AccountRec>, StoreError>>;

    /// Append a character to an account's list (atomic per call). Fails with
    /// [`StoreError::NoSuchAccount`] if the account is missing.
    fn link_character<'a>(
        &'a self,
        account: &'a str,
        id: ObjId,
        char_name: &'a str,
    ) -> BoxFuture<'a, Result<(), StoreError>>;

    /// Every stored account, in unspecified order. (Deleting accounts is a
    /// later milestone; the key layout already supports it.)
    fn list_accounts(&self) -> BoxFuture<'_, Result<Vec<AccountRec>, StoreError>>;

    /// Replace an account's password hash (`@newpassword`, future
    /// self-serve reset). Atomic per call (Redis: a single server-side
    /// `JSON.SET $.argon2_hash` on the existing doc). Returns whether the
    /// account existed — `Ok(false)` for an unknown name, never an error.
    /// The caller is responsible for invalidating the account's web tokens
    /// ([`ObjectStore::tokens_del_account`]) alongside.
    fn set_account_password<'a>(
        &'a self,
        account: &'a str,
        argon2_hash: &'a str,
    ) -> BoxFuture<'a, Result<bool, StoreError>>;

    /// Replace an account's authorized SSH public keys wholesale
    /// (`@sshkey/add|del`). Atomic per call. Returns whether the account
    /// existed — `Ok(false)` for an unknown name, never an error. The field
    /// is additive (`#[serde(default)]`), so accounts whose stored doc
    /// predates `ssh_keys` are written correctly (the missing `$.ssh_keys`
    /// path is created, not required to exist).
    fn set_account_ssh_keys<'a>(
        &'a self,
        account: &'a str,
        keys: &'a [SshKeyRec],
    ) -> BoxFuture<'a, Result<bool, StoreError>>;

    // ------------------------------------------------------------------
    // Discord identity (D1, D-D-2): the link on the account doc, the reverse
    // index that is the uniqueness lock, one-shot link codes, and the
    // Discord-scoped ban list. Each call is individually atomic; the link
    // flow is `discord_user_reserve` → `set_account_discord`, releasing the
    // reservation if the second step fails, and a reverse entry whose account
    // carries no link is treated as stale by readers.
    // ------------------------------------------------------------------

    /// Set (or, with `None`, clear) an account's Discord link. Atomic per
    /// call; returns whether the account existed — `Ok(false)` for an unknown
    /// name, never an error. Additive like `ssh_keys`: a doc that predates
    /// the field is written correctly.
    fn set_account_discord<'a>(
        &'a self,
        account: &'a str,
        link: Option<&'a DiscordLinkRec>,
    ) -> BoxFuture<'a, Result<bool, StoreError>>;

    /// Claim the reverse index entry `discord:user:{id} → account` if it is
    /// free (or already this account's). Returns `Ok(false)` when another
    /// account holds it — the "already linked elsewhere" refusal.
    fn discord_user_reserve<'a>(
        &'a self,
        user_id: u64,
        account: &'a str,
    ) -> BoxFuture<'a, Result<bool, StoreError>>;

    /// Drop the reverse index entry (unlink). Returns whether one existed.
    fn discord_user_release(&self, user_id: u64) -> BoxFuture<'_, Result<bool, StoreError>>;

    /// The account a Discord user is linked to, through the reverse index.
    /// `Ok(None)` when unlinked, or when the index points at an account that
    /// is missing or carries no link (a stale reservation).
    fn account_by_discord_user(
        &self,
        user_id: u64,
    ) -> BoxFuture<'_, Result<Option<AccountRec>, StoreError>>;

    /// Store a pending link code under its hash (last write wins). An
    /// `expires` at or before now is equivalent to not storing it.
    fn discord_code_put(
        &self,
        hash: String,
        rec: DiscordCodeRecord,
    ) -> BoxFuture<'_, Result<(), StoreError>>;

    /// Consume a live link code: fetch **and delete** it in one step, so a
    /// code redeems at most once. `Ok(None)` when absent or expired.
    fn discord_code_take<'a>(
        &'a self,
        hash: &'a str,
    ) -> BoxFuture<'a, Result<Option<DiscordCodeRecord>, StoreError>>;

    /// Set (or replace) the Discord-scoped ban on a user.
    fn discord_ban_put(&self, rec: DiscordBanRec) -> BoxFuture<'_, Result<(), StoreError>>;

    /// The ban on a Discord user, if any.
    fn discord_ban_get(
        &self,
        user_id: u64,
    ) -> BoxFuture<'_, Result<Option<DiscordBanRec>, StoreError>>;

    /// Lift the ban on a Discord user. Returns whether one was removed.
    fn discord_ban_del(&self, user_id: u64) -> BoxFuture<'_, Result<bool, StoreError>>;

    // ------------------------------------------------------------------
    // Sitebans (persisted; IpGuard tempbans stay in-memory)
    // ------------------------------------------------------------------

    /// Set (or replace) the ban on `ip`. `until_unix = None` is permanent;
    /// a timestamp at or before now clears any existing ban (mirroring
    /// Redis TTL semantics, where such a key expires immediately).
    fn set_ban<'a>(
        &'a self,
        ip: &'a str,
        until_unix: Option<i64>,
        reason: &'a str,
    ) -> BoxFuture<'a, Result<(), StoreError>>;

    /// Every **active** ban. Expired bans are gone: Redis expires the keys
    /// natively; the in-memory store filters (and prunes) by the system
    /// clock at read time.
    fn get_bans(&self) -> BoxFuture<'_, Result<Vec<BanRec>, StoreError>>;

    /// Lift the ban on `ip`. Returns whether a ban was actually removed.
    fn clear_ban<'a>(&'a self, ip: &'a str) -> BoxFuture<'a, Result<bool, StoreError>>;

    /// Publish a short-lived password-tempban lift. Replaces the prior request
    /// for this address; never changes a persisted siteban. Unsupported stores
    /// must refuse the write rather than acknowledge a pardon they cannot deliver.
    fn put_tempban_lift(&self, lift: crate::TempbanLift) -> BoxFuture<'_, Result<(), StoreError>>;

    /// Read active lift requests without consuming them: every gateway guard
    /// must receive the command. TTL and guard-local deduplication bound replay.
    fn get_tempban_lifts(&self) -> BoxFuture<'_, Result<Vec<crate::TempbanLift>, StoreError>>;

    // ------------------------------------------------------------------
    // Web session tokens (`token:{hash}` keys, native TTL)
    //
    // The key is the SHA-256 hex of the opaque token (hashing happens
    // caller-side in the web auth layer; the raw token never reaches the
    // store, is never persisted, and must never be logged). Expiry
    // semantics mirror bans: Redis expires keys natively via `EXAT`; the
    // in-memory store prunes lazily against the system clock. Concurrency:
    // called from the web auth tasks — each call is individually atomic,
    // no cross-call ordering (same contract as the account methods).
    // ------------------------------------------------------------------

    /// Store (or replace — last write wins) the token record under its
    /// hash. An `expires` at or before now is equivalent to deleting the
    /// token (mirroring a key set with `EXAT` in the past).
    fn token_put(&self, hash: String, rec: TokenRecord) -> BoxFuture<'_, Result<(), StoreError>>;

    /// Fetch a live token record. `Ok(None)` when absent **or expired** —
    /// an expired record is lazily deleted on the way out (Redis TTL does
    /// this natively; the in-memory store prunes on read).
    fn token_get<'a>(
        &'a self,
        hash: &'a str,
    ) -> BoxFuture<'a, Result<Option<TokenRecord>, StoreError>>;

    /// Sliding-TTL refresh: move a live token's `expires` (record field
    /// *and* store TTL) to `new_expires`. Returns whether a live token was
    /// touched — a missing or already-expired token is `false`, never
    /// resurrected. A `new_expires` at or before now deletes the token
    /// (and still reports `true` if it was live).
    fn token_touch<'a>(
        &'a self,
        hash: &'a str,
        new_expires: i64,
    ) -> BoxFuture<'a, Result<bool, StoreError>>;

    /// Delete one token (logout). Returns whether a live token was removed.
    fn token_del<'a>(&'a self, hash: &'a str) -> BoxFuture<'a, Result<bool, StoreError>>;

    /// Delete every live token belonging to `account` (case-insensitive) —
    /// logout-everywhere and ban support. Returns how many live tokens were
    /// removed (expired stragglers are pruned but not counted).
    fn tokens_del_account<'a>(&'a self, account: &'a str)
    -> BoxFuture<'a, Result<u64, StoreError>>;

    // ------------------------------------------------------------------
    // Module persistence reads (`mod:{ns}:{key}` keys)
    //
    // Module state is *written* through [`WriteBatch::module_ops`] — same
    // transaction, same atomicity as object writes. These reads serve two
    // paths, both off the world loop:
    //
    // - **boot / module load**: `module_json_list` hydrates a module's whole
    //   namespace into RAM once, from the boot task;
    // - **the store-read seam** (H1, `docs/design/11-h-implementation-plan.md`
    //   §3.1): during play a module asks for a bounded page of its **own**
    //   documents by key through `Ctx::read_store`; the world task performs
    //   one `module_json_get` per key from a detached task — never a listing —
    //   and re-enters with a typed completion. Play-time *authoritative*
    //   state still lives in RAM; the seam serves cold history (sealed ledger
    //   chunks, archives), never a hot-path lookup.
    //
    // The concurrency notes for account/ban methods apply to both.
    //
    // Namespace rules (enforced by every implementation): `ns` is non-empty
    // and contains no `:` — otherwise `mod:a:*` listings could leak keys
    // across namespaces. Keys are non-empty and may contain `:`.
    //
    // [`WriteBatch::module_ops`]: crate::types::WriteBatch::module_ops
    // ------------------------------------------------------------------

    /// Fetch one module JSON document (`JSON.GET mod:{ns}:{key}`).
    /// `Ok(None)` when absent.
    fn module_json_get<'a>(
        &'a self,
        ns: &'a str,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Option<serde_json::Value>, StoreError>>;

    /// Every JSON document in a module namespace, as `(key, value)` pairs
    /// (`SCAN mod:{ns}:*` — the trailing `:` in the pattern is what keeps
    /// `mod:a:*` from matching `mod:ab:...`). Keys are returned **without**
    /// the `mod:{ns}:` prefix. Order is unspecified. Stream keys sharing
    /// the namespace are skipped, not errors; implementations must
    /// positively identify a key as a stream rather than treating every
    /// JSON wrong-type error as one. An unknown namespace is an empty list.
    /// A discovered JSON document that cannot be decoded, or a key with any
    /// other storage type, is an error; implementations must never omit
    /// corrupt authoritative bytes.
    fn module_json_list<'a>(
        &'a self,
        ns: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, serde_json::Value)>, StoreError>>;

    /// Every document in a module namespace, **split for boot** (J1): the
    /// ordinary JSON documents decoded, and every ledger chunk document
    /// (`log:`-prefixed keys) as its raw JSON text — never a `Value` — so a
    /// journal's boot footprint is its byte size and the one inspection every
    /// consumer shares decodes each chunk exactly once (doc 12 F9). The same
    /// listing rules as [`ObjectStore::module_json_list`]: keys without the
    /// `mod:{ns}:` prefix, streams skipped, order unspecified, an undecodable
    /// ordinary document an error. A ledger document is not decoded here at
    /// all; the ledger's inspection is what judges it.
    ///
    /// The default derives the split from `module_json_list` (chunks are
    /// re-encoded, compact); a backend that holds documents as text overrides
    /// it to hand the text over as is.
    fn module_docs<'a>(&'a self, ns: &'a str) -> BoxFuture<'a, Result<ModuleDocs, StoreError>> {
        Box::pin(async move { Ok(ModuleDocs::split(self.module_json_list(ns).await?)) })
    }

    /// [`ObjectStore::module_docs`] **narrowed** (J4, doc 12 §5.6; doc 13 W1):
    /// a ledger chunk below its log's floor is neither fetched from the
    /// primary nor merged from the archive, a key under a skipped prefix is
    /// not fetched at all, and what was skipped comes back in
    /// [`ModuleDocs::omitted`] and [`ModuleDocs::skipped_prefixes`]. With
    /// nothing to skip this is the whole listing. The default lists whole and
    /// drops what the skip excludes — correct, never narrow; a real backend
    /// overrides it to skip the fetch itself.
    fn module_docs_from<'a>(
        &'a self,
        ns: &'a str,
        skip: &'a BootSkip,
    ) -> BoxFuture<'a, Result<ModuleDocs, StoreError>> {
        Box::pin(async move {
            let mut docs = self.module_docs(ns).await?;
            docs.ledger.retain(|(key, _)| !skip.skips(key));
            docs.docs.retain(|(key, _)| !skip.skips(key));
            docs.omitted = skip.floors.clone();
            docs.skipped_prefixes = skip.prefixes.clone();
            Ok(docs)
        })
    }

    // ------------------------------------------------------------------
    // The ledger archive tier (J5, doc 12 §10)
    // ------------------------------------------------------------------

    /// Every document key **resident in the primary** for a module namespace,
    /// without the `mod:{ns}:` prefix (streams skipped, order unspecified).
    /// The archive pass walks this to find sealed chunks it has not copied
    /// and to size what is resident; archived-only chunks are not listed.
    /// The default derives it from [`ObjectStore::module_json_list`].
    fn module_keys<'a>(&'a self, ns: &'a str) -> BoxFuture<'a, Result<Vec<String>, StoreError>> {
        Box::pin(async move {
            Ok(self
                .module_json_list(ns)
                .await?
                .into_iter()
                .map(|(key, _)| key)
                .collect())
        })
    }

    /// One module document as **raw JSON text**, the primary first and — for
    /// a `log:` key the primary lacks — the ledger archive, if one is attached
    /// ([`ObjectStore::ledger_archive`]). `Ok(None)` when neither holds it or
    /// the archive's copy is damaged. The default re-encodes
    /// [`ObjectStore::module_json_get`] compactly.
    fn module_doc_text<'a>(
        &'a self,
        ns: &'a str,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, StoreError>> {
        Box::pin(async move {
            let primary = self
                .module_json_get(ns, key)
                .await?
                .map(|value| serde_json::to_string(&value).expect("a Value always serializes"));
            crate::archive::tiered_text(self.ledger_archive(), ns, key, primary)
        })
    }

    /// Delete module documents the archive pass has **verified in the archive**
    /// — sealed ledger chunks only; nothing else is ever purged. Returns how
    /// many were present. The default refuses: a backend that cannot purge
    /// says so rather than pretending.
    fn module_purge_archived<'a>(
        &'a self,
        ns: &'a str,
        keys: Vec<String>,
    ) -> BoxFuture<'a, Result<usize, StoreError>> {
        let _ = (ns, keys);
        Box::pin(async move {
            Err(StoreError::Invalid(
                "this store cannot purge archived ledger chunks".to_owned(),
            ))
        })
    }

    /// The ledger archive attached to this store, if any: the secondary tier
    /// [`ObjectStore::module_json_get`], [`ObjectStore::module_docs`] and
    /// [`ObjectStore::module_doc_text`] fall through to for `log:` keys.
    fn ledger_archive(&self) -> Option<&crate::archive::LedgerArchive> {
        None
    }

    /// The newest `count_latest` entries of a module stream, returned
    /// **oldest-first** (`XREVRANGE mod:{ns}:{key} + - COUNT n`, then
    /// re-reversed). Each entry is `(id, value)` where `id` is the store's
    /// stream id (Redis `ms-seq`; in-memory `N-0`) — ids within one stream
    /// are strictly increasing in append order. A missing stream or
    /// `count_latest == 0` yields an empty list.
    fn stream_range<'a>(
        &'a self,
        ns: &'a str,
        key: &'a str,
        count_latest: u32,
    ) -> BoxFuture<'a, Result<Vec<(String, serde_json::Value)>, StoreError>>;

    // ------------------------------------------------------------------
    // Meta / migrations
    // ------------------------------------------------------------------

    /// The stored app-level schema version. `Ok(None)` on a store that has
    /// never been stamped (fresh, or pre-versioning).
    fn get_schema_version(&self) -> BoxFuture<'_, Result<Option<u32>, StoreError>>;

    /// Stamp the schema version ([`crate::run_migrations`] drives this).
    fn set_schema_version(&self, version: u32) -> BoxFuture<'_, Result<(), StoreError>>;

    // ------------------------------------------------------------------
    // Maintenance
    // ------------------------------------------------------------------

    /// Trigger a background snapshot (Redis `BGSAVE`; no-op in memory).
    /// "Already in progress" is success. Used by `@dump`, scheduled backups,
    /// and mandatorily before migrations run.
    fn bgsave(&self) -> BoxFuture<'_, Result<(), StoreError>>;

    /// Flush anything pending and await its acknowledgement. Because every
    /// [`write`](ObjectStore::write) is already awaited to its ack, there is
    /// never a client-side queue; this is the shutdown-sequence barrier
    /// ("drain write pipeline") and doubles as a liveness check.
    fn drain(&self) -> BoxFuture<'_, Result<(), StoreError>>;

    /// Release the store (close connections). Further calls fail with
    /// [`StoreError::Closed`] (or a connection error from Redis).
    fn close(&self) -> BoxFuture<'_, Result<(), StoreError>>;
}

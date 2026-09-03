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
use mush_core::ObjId;

use crate::types::{
    AccountRec, BanRec, SshKeyRec, StoreError, TokenRecord, WorldSnapshot, WriteBatch, WriteReceipt,
};

/// Persistent storage for the world, accounts, sitebans, and store metadata.
///
/// Implementations: [`crate::RedisStore`] (authoritative, Redis 8) and
/// [`crate::InMemoryStore`] (tests and `mushd --repl`). Both honor the exact
/// same semantics; the shared behavior suite in [`crate::suite`] proves it.
pub trait ObjectStore: Send + Sync {
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
    // transaction, same atomicity as object writes. These reads are the
    // out-of-band load path: modules hydrate their state at boot/module
    // load, never during play (play-time reads come from the module's own
    // in-RAM state). Called from the boot task; the concurrency notes for
    // account/ban methods apply.
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

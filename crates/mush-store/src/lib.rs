//! mush-store — Redis 8 persistence: the [`ObjectStore`] trait, the
//! authoritative [`RedisStore`] (fred 10, RESP3, RedisJSON docs, FT index,
//! per-command `MULTI`/`EXEC` write-through), the [`InMemoryStore`] fake,
//! and app-level [`migrate`] machinery.
//!
//! Design (PLAN.md "Persistence" + "Redis data layout"):
//!
//! - **Write-through, per command.** Each processed command's world
//!   mutations arrive as one [`WriteBatch`] and are applied as one atomic
//!   unit, immediately — no timer batching, no dirty-flush window. Crash
//!   recovery = reload from the store; at most the in-flight command is
//!   lost.
//! - **`mush-core` stays ignorant.** The world task snapshots dirty objects
//!   (`World::take_dirty` + clones) into batches; nothing in core knows this
//!   crate exists.
//! - **Ack gating.** [`ObjectStore::write`] resolves when the store has
//!   committed the batch; durability-critical flows (account/character
//!   creation) hold their confirmation output until then.
//! - **One writer.** See the concurrency contract in [`store`].
//!
//! Testing: the shared behavior [`suite`] runs against [`InMemoryStore`]
//! unconditionally and against [`RedisStore`] through explicitly ignored
//! `tests/redis_suite.rs` wrappers. Linux CI supplies `REDIS_URL` and runs
//! those wrappers explicitly, so absent Redis coverage is visible.

pub mod memory;
pub mod migrate;
pub mod redis;
pub mod store;
#[doc(hidden)]
pub mod suite;
pub mod types;

pub use memory::InMemoryStore;
pub use migrate::{
    CURRENT_SCHEMA_VERSION, Migration, MigrationFn, core_migrations, run_migrations,
};
pub use redis::RedisStore;
pub use store::{AuthoritativeStore, ObjectStore};
pub use types::{
    AccountRec, AuthoritativeCheckpointReceipt, AuthoritativeDiscardReceipt,
    AuthoritativeEpochRetirement, AuthoritativeIntentPage, AuthoritativeOutboxAttemptResult,
    AuthoritativeOutboxRecord, AuthoritativeOutboxState, AuthoritativeQuarantineReceipt,
    AuthoritativeReceipt, AuthoritativeRetentionUsage, AuthoritativeScanCursor, BanRec,
    BoundIdempotencyStatus, BundleStatus, CharacterRef, IdempotencyStatus, IntentReceipt,
    MAX_AUTHORITATIVE_DISCOVERY_PAGE,
    MAX_AUTHORITATIVE_OUTBOX_ERROR_BYTES, MAX_AUTHORITATIVE_SCAN_CURSOR_BYTES,
    MAX_PENDING_AUTHORITATIVE_OUTBOX, MAX_RETAINED_AUTHORITATIVE_BYTES,
    MAX_RETAINED_AUTHORITATIVE_INTENTS, ModuleOp, ModuleWrite, PendingAuthoritativeIntent,
    SshKeyRec, StoreError, TokenRecord, WorldSnapshot, WriteBatch, WriteReceipt,
};

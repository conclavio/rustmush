//! App-level, versioned migrations (PLAN.md: "Migrations are app-level
//! versioned per namespace — meta keys + Rust migration fns; BGSAVE before
//! migrate is mandatory").
//!
//! The store carries one `meta:schema_version` integer. A build registers
//! its ordered [`Migration`] list ([`core_migrations`] for the core
//! namespace; modules append theirs in M3+) and calls [`run_migrations`] at
//! boot, after connecting and before loading the world.
//!
//! Rules:
//!
//! - **v1 is the initial schema** and its migration is a no-op — a store
//!   first written by any versioned build is already at v1+.
//! - A **fresh store** (no version stamped) is stamped straight to the
//!   latest registered version without running anything: data written by
//!   this build is by definition already in the current format.
//! - A store **newer than the binary** refuses to run (downgrades are a
//!   restore-from-backup operation, not code).
//! - Before any migration actually runs, [`ObjectStore::bgsave`] is
//!   triggered — the mandatory pre-migrate snapshot.
//! - Each migration that completes bumps the stored version immediately, so
//!   a crash mid-sequence resumes exactly where it stopped.

use futures::future::BoxFuture;

use crate::store::ObjectStore;
use crate::types::StoreError;

/// The schema version this build reads and writes.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// A migration step's body: runs against the open store, may read and write
/// anything, must be idempotent enough to re-run after a crash between the
/// step and its version bump.
pub type MigrationFn =
    Box<dyn for<'a> Fn(&'a dyn ObjectStore) -> BoxFuture<'a, Result<(), StoreError>> + Send + Sync>;

/// One registered migration: brings a store *to* `to_version` from
/// `to_version - 1`.
pub struct Migration {
    /// The version this step upgrades the store to.
    pub to_version: u32,
    /// Human-readable label for logs.
    pub name: &'static str,
    /// The step body.
    pub run: MigrationFn,
}

/// The core namespace's migration registry. v1 = initial schema, no-op.
#[must_use]
pub fn core_migrations() -> Vec<Migration> {
    vec![Migration {
        to_version: 1,
        name: "initial schema",
        run: Box::new(|_| Box::pin(async { Ok(()) })),
    }]
}

/// Bring `store` up to the newest version in `migrations` (see the
/// [module docs](self) for the exact rules). Returns the version the store
/// is at afterwards.
pub async fn run_migrations(
    store: &dyn ObjectStore,
    migrations: &[Migration],
) -> Result<u32, StoreError> {
    let Some(latest) = migrations.last().map(|m| m.to_version) else {
        return Err(StoreError::Migration("empty migration set".into()));
    };
    let mut prev = 0u32;
    for m in migrations {
        if m.to_version <= prev {
            return Err(StoreError::Migration(format!(
                "migration set not strictly ascending at v{}",
                m.to_version
            )));
        }
        prev = m.to_version;
    }

    let current = match store.get_schema_version().await? {
        Some(v) => v,
        None => {
            // Fresh store: stamp to latest, run nothing.
            store.set_schema_version(latest).await?;
            tracing::info!(version = latest, "fresh store stamped at current schema");
            return Ok(latest);
        }
    };
    if current > latest {
        return Err(StoreError::Migration(format!(
            "store schema v{current} is newer than this build's v{latest}; \
             refusing to run (restore from backup to downgrade)"
        )));
    }
    if current == latest {
        return Ok(current);
    }

    // Mandatory pre-migrate snapshot.
    store.bgsave().await?;
    let mut at = current;
    for m in migrations.iter().filter(|m| m.to_version > current) {
        tracing::info!(
            from = at,
            to = m.to_version,
            name = m.name,
            "running migration"
        );
        (m.run)(store).await?;
        store.set_schema_version(m.to_version).await?;
        at = m.to_version;
    }
    Ok(at)
}

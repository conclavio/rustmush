//! Durable quarantine storage for authoritative ledger seals.
//!
//! Opening this store is a boot precondition.  A directory that cannot be
//! enumerated, a record that cannot be read and verified, or a crash-left temp
//! file is integrity state, not an empty spool.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use mush_core::{LedgerChunk, MAX_MODULE_KEY_BYTES, MAX_MODULE_NAMESPACE_BYTES};
use mush_store::{ObjectStore, StoreError};
use serde::{Deserialize, Serialize};

use mush_config::unix_now_s;

/// On-disk quarantine record schema written by this release.
pub const QUARANTINE_RECORD_VERSION: u32 = 1;

/// A quarantine record is recovery material, but it must still be bounded at
/// the trust boundary before allocation.
const MAX_RECORD_BYTES: u64 = 16 * 1024 * 1024;
const MAX_RECORDS: usize = 100_000;
/// A separate failure domain still needs a finite disk budget. One GiB holds
/// ample recovery material while preventing a poison stream from consuming an
/// entire volume before the next boot-time scan.
const MAX_SPOOL_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ERROR_BYTES: usize = 4 * 1024;
const MAX_VERSION_BYTES: usize = 256;
static PROBE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Validated boot snapshot of the quarantine store.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpoolState {
    /// Number of complete, verified records awaiting operator recovery.
    pub unresolved_records: usize,
    /// Encoded bytes occupied by those records.
    pub unresolved_bytes: u64,
}

/// A valid but unresolved quarantine set blocks ordinary writable boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "{unresolved_records} verified quarantine record(s), {unresolved_bytes} bytes, require recovery before writable boot"
)]
pub struct UnresolvedQuarantine {
    /// Number of verified recovery records.
    pub unresolved_records: usize,
    /// Total encoded bytes retained.
    pub unresolved_bytes: u64,
}

/// Read-only authority exposed to the quarantine reconciler.
///
/// Production adapts an [`ObjectStore`] through the blanket implementation,
/// but the recovery algorithm receives only this one read method. This keeps
/// migrations, snapshots, and ordinary writes out of the pre-boot recovery
/// phase by construction and lets tests prove that no write surface is used.
pub trait QuarantineTargetReader: Send + Sync {
    /// Read one authoritative module JSON target without mutating the store.
    fn quarantine_target<'a>(
        &'a self,
        namespace: &'a str,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Option<serde_json::Value>, StoreError>>;
}

impl<T: ObjectStore + ?Sized> QuarantineTargetReader for T {
    fn quarantine_target<'a>(
        &'a self,
        namespace: &'a str,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Option<serde_json::Value>, StoreError>> {
        ObjectStore::module_json_get(self, namespace, key)
    }
}

/// Result of one bounded, read-only exact-target recovery pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QuarantineRecoveryReport {
    /// Verified active records whose targets were examined.
    pub examined: usize,
    /// Records durably moved to evidence custody after an exact match.
    ///
    /// This is not yet the richer C11 resolution receipt (which will also
    /// attest the observation time, resolver version, and observed hash).
    pub resolved: usize,
    /// Valid records that remain active because their target was absent or
    /// differed.
    pub remaining: SpoolState,
}

/// Failure while comparing active recovery records with authoritative store
/// state or preserving the resolution evidence.
#[derive(Debug, thiserror::Error)]
pub enum QuarantineRecoveryError {
    /// A validated active record could not be read from the authoritative
    /// store. No record is resolved on an unreadable observation.
    #[error("reading authoritative quarantine target {namespace}/{key}: {source}")]
    TargetRead {
        /// Module namespace from the validated record.
        namespace: String,
        /// Module key from the validated record.
        key: String,
        /// Store read failure.
        #[source]
        source: StoreError,
    },
    /// The spool or resolved-evidence archive could not be read, validated, or
    /// durably updated.
    #[error(transparent)]
    Spool(#[from] QuarantineBootError),
    /// The in-process serialization lock was poisoned.
    #[error("quarantine spool recovery lock poisoned")]
    LockPoisoned,
    /// The validated record set changed unexpectedly while the lifetime lock
    /// was held.
    #[error("quarantine recovery record disappeared: {0}")]
    RecordMissing(String),
    /// Runtime accounting disagreed with the exhaustively scanned spool.
    #[error("quarantine recovery state accounting underflow")]
    StateUnderflow,
    /// Preserving another resolved record would exceed the explicit evidence
    /// archive bound. The active record remains unresolved.
    #[error(
        "resolved quarantine archive capacity exhausted (records {records}/{MAX_RECORDS}, bytes {bytes}/{MAX_SPOOL_BYTES})"
    )]
    ArchiveCapacity {
        /// Record count after accepting the evidence candidate.
        records: usize,
        /// Encoded byte count after accepting the evidence candidate.
        bytes: u64,
    },
}

/// A boot-time quarantine-store integrity failure.
#[derive(Debug, thiserror::Error)]
pub enum QuarantineBootError {
    /// The directory could not be created, inspected, or enumerated.
    #[error("quarantine spool {path}: {source}")]
    Io {
        /// Failed path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The path exists but is not a directory.
    #[error("quarantine spool path is not a directory: {0}")]
    NotDirectory(PathBuf),
    /// A pre-opened spool does not belong to this world's configured data
    /// directory.
    #[error("quarantine spool binding mismatch: expected {expected}, opened {opened}")]
    BindingMismatch {
        /// Canonical spool required by the current configuration.
        expected: PathBuf,
        /// Canonical spool actually held by the caller.
        opened: PathBuf,
    },
    /// A crash-left atomic-write temp file was found.
    #[error("partial quarantine record requires recovery: {0}")]
    PartialRecord(PathBuf),
    /// An unexpected artifact makes the directory ambiguous.
    #[error("unexpected artifact in quarantine spool: {0}")]
    UnexpectedArtifact(PathBuf),
    /// The directory exceeds its hard record bound.
    #[error("quarantine spool has more than {MAX_RECORDS} records")]
    TooManyRecords,
    /// One record exceeds its hard encoded-size bound.
    #[error("quarantine record is larger than {MAX_RECORD_BYTES} bytes: {0}")]
    RecordTooLarge(PathBuf),
    /// A record is malformed or violates its integrity contract.
    #[error("invalid quarantine record {path}: {detail}")]
    InvalidRecord {
        /// Record path.
        path: PathBuf,
        /// Bounded reason.
        detail: String,
    },
    /// The aggregate byte count overflowed.
    #[error("quarantine spool byte count overflow")]
    ByteCountOverflow,
    /// The aggregate encoded data exceeds the spool's finite disk budget.
    #[error("quarantine spool exceeds its {MAX_SPOOL_BYTES}-byte aggregate bound")]
    SpoolTooLarge,
}

/// A runtime failure preserving a newly confirmed poison seal.
#[derive(Debug, thiserror::Error)]
pub enum QuarantineWriteError {
    /// The candidate itself is not a valid sealed ledger chunk.
    #[error("invalid sealed ledger quarantine candidate: {0}")]
    InvalidCandidate(String),
    /// Durable I/O or post-write verification failed.
    #[error("quarantine spool {path}: {detail}")]
    Durable {
        /// Failed path.
        path: PathBuf,
        /// Bounded reason.
        detail: String,
    },
    /// The in-process serialization lock was poisoned.
    #[error("quarantine spool runtime lock poisoned")]
    LockPoisoned,
    /// Accepting this record would make a subsequent boot reject the spool.
    #[error(
        "quarantine spool capacity exhausted (records {records}/{MAX_RECORDS}, bytes {bytes}/{MAX_SPOOL_BYTES})"
    )]
    Capacity {
        /// Record count after accepting the candidate.
        records: usize,
        /// Aggregate encoded bytes after accepting the candidate.
        bytes: u64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuarantineRecord {
    record_version: u32,
    ns: String,
    key: String,
    serial: u64,
    payload_hash: String,
    payload: serde_json::Value,
    store_error: String,
    error_transient: bool,
    isolation_attempts: u32,
    quarantined_unix: i64,
    server_version: String,
}

#[derive(Debug)]
struct DurableBackend {
    dir: PathBuf,
    // File locks are released when the last Arc-backed spool clone drops.
    // Keeping the handle here closes the scan-versus-write TOCTOU between two
    // rustmush processes pointed at the same recovery directory.
    _lifetime_lock: File,
    runtime: Mutex<RuntimeState>,
}

#[derive(Debug)]
struct MemoryBackend {
    records: BTreeMap<String, QuarantineRecord>,
    state: SpoolState,
}

#[derive(Debug)]
struct RuntimeState {
    state: SpoolState,
    // Resolved evidence is a distinct, bounded persistent collection. It is
    // exhaustively scanned at open and charged before an active record moves.
    resolved_state: SpoolState,
    // Failed publication is retried in place by the persistence writer. Keep
    // one reservation per content identity so retries cannot consume capacity
    // repeatedly while the filesystem is unavailable.
    reservations: BTreeMap<String, RuntimeReservation>,
}

#[derive(Debug)]
struct RuntimeReservation {
    // Freeze the first valid envelope for this identity. Later retries may
    // produce different timestamps or diagnostics, but partial-write recovery
    // must compare against exactly the bytes whose write was interrupted.
    envelope: Arc<[u8]>,
}

#[derive(Clone, Debug)]
struct RecoveryTarget {
    identity: String,
    namespace: String,
    key: String,
}

#[derive(Debug)]
enum Backend {
    Durable(DurableBackend),
    InMemory(Mutex<MemoryBackend>),
}

/// Required quarantine sink held by the persistence writer.
///
/// Production obtains this only through [`QuarantineSpool::open`], which
/// validates the complete directory before any other startup mutation. Tests
/// opt into [`QuarantineSpool::in_memory`] explicitly.
#[derive(Clone, Debug)]
pub struct QuarantineSpool {
    backend: Arc<Backend>,
    opened_state: SpoolState,
}

impl QuarantineSpool {
    /// Open (creating if absent) and exhaustively validate a durable spool.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, QuarantineBootError> {
        let requested_path = path.into();
        let created =
            create_spool_directory(&requested_path).map_err(|source| QuarantineBootError::Io {
                path: requested_path.clone(),
                source,
            })?;
        let path =
            std::fs::canonicalize(&requested_path).map_err(|source| QuarantineBootError::Io {
                path: requested_path.clone(),
                source,
            })?;
        let metadata = std::fs::metadata(&path).map_err(|source| QuarantineBootError::Io {
            path: path.clone(),
            source,
        })?;
        if !metadata.is_dir() {
            return Err(QuarantineBootError::NotDirectory(path));
        }
        if created {
            fsync_parent(&path).map_err(|source| QuarantineBootError::Io {
                path: path.clone(),
                source,
            })?;
        }

        // Acquire the cross-process lock before enumeration and retain it for
        // the complete spool lifetime. Runtime callers additionally serialize
        // through `runtime` below, so neither processes nor threads can race a
        // capacity decision with publication.
        let lifetime_lock =
            open_lifetime_lock(&path).map_err(|(lock_path, source)| QuarantineBootError::Io {
                path: lock_path,
                source,
            })?;

        // Reconcile a crash after a temp file was synced but before (or just
        // after) its final directory entry was published. Only a fully decoded,
        // exact-identity record is eligible; malformed or conflicting custody
        // remains untouched and fails closed.
        let initial_paths = spool_paths(&path)?;
        recover_temps_at_boot(&path, &metadata, &initial_paths)?;

        let entries = std::fs::read_dir(&path).map_err(|source| QuarantineBootError::Io {
            path: path.clone(),
            source,
        })?;
        let mut state = SpoolState::default();
        for entry in entries {
            let entry = entry.map_err(|source| QuarantineBootError::Io {
                path: path.clone(),
                source,
            })?;
            let record_path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|source| QuarantineBootError::Io {
                    path: record_path.clone(),
                    source,
                })?;
            if !file_type.is_file() {
                return Err(QuarantineBootError::UnexpectedArtifact(record_path));
            }
            match record_path.extension().and_then(|ext| ext.to_str()) {
                Some("tmp") => return Err(QuarantineBootError::PartialRecord(record_path)),
                Some("json") => {}
                _ => return Err(QuarantineBootError::UnexpectedArtifact(record_path)),
            }
            enforce_private_file_mode(&record_path).map_err(|source| QuarantineBootError::Io {
                path: record_path.clone(),
                source,
            })?;
            ensure_same_owner(
                &metadata,
                &std::fs::metadata(&record_path).map_err(|source| QuarantineBootError::Io {
                    path: record_path.clone(),
                    source,
                })?,
            )
            .map_err(|source| QuarantineBootError::Io {
                path: record_path.clone(),
                source,
            })?;
            state.unresolved_records = state
                .unresolved_records
                .checked_add(1)
                .ok_or(QuarantineBootError::TooManyRecords)?;
            if state.unresolved_records > MAX_RECORDS {
                return Err(QuarantineBootError::TooManyRecords);
            }
            let bytes = read_bounded(&record_path)?;
            let record = decode_and_validate(&record_path, &bytes)?;
            // Force the decoded value to remain live through validation. This
            // also makes it explicit that merely parseable JSON is not enough.
            drop(record);
            state.unresolved_bytes = state
                .unresolved_bytes
                .checked_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
                .ok_or(QuarantineBootError::ByteCountOverflow)?;
            if state.unresolved_bytes > MAX_SPOOL_BYTES {
                return Err(QuarantineBootError::SpoolTooLarge);
            }
        }

        // Read permission alone is insufficient for a durability sink: a 0550
        // directory can be enumerated and fsynced but cannot preserve poison.
        // Exercise the same create/sync/link/dir-sync/delete/dir-sync chain
        // used by the writer before any world/store startup mutation begins.
        probe_write_capability(&path).map_err(|source| QuarantineBootError::Io {
            path: path.clone(),
            source,
        })?;
        let resolved_state = scan_resolved_archive(&path, &metadata)?;
        Ok(Self {
            backend: Arc::new(Backend::Durable(DurableBackend {
                dir: path,
                _lifetime_lock: lifetime_lock,
                runtime: Mutex::new(RuntimeState {
                    state,
                    resolved_state,
                    reservations: BTreeMap::new(),
                }),
            })),
            opened_state: state,
        })
    }

    /// Explicit non-durable implementation for unit/integration tests.
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            backend: Arc::new(Backend::InMemory(Mutex::new(MemoryBackend {
                records: BTreeMap::new(),
                state: SpoolState::default(),
            }))),
            opened_state: SpoolState::default(),
        }
    }

    /// The exhaustively validated state captured at open.
    #[must_use]
    pub const fn opened_state(&self) -> SpoolState {
        self.opened_state
    }

    pub(crate) fn require_bound_path(&self, requested: &Path) -> Result<(), QuarantineBootError> {
        let expected =
            std::fs::canonicalize(requested).map_err(|source| QuarantineBootError::Io {
                path: requested.to_owned(),
                source,
            })?;
        let opened = match self.backend.as_ref() {
            Backend::Durable(backend) => backend.dir.clone(),
            Backend::InMemory(_) => PathBuf::from("<in-memory>"),
        };
        if expected == opened {
            Ok(())
        } else {
            Err(QuarantineBootError::BindingMismatch { expected, opened })
        }
    }

    /// Refuse the current ordinary boot path when verified incidents remain.
    ///
    /// Callers first run [`Self::reconcile_exact_matches`]. Allowing migrations,
    /// seeding, or module boot fixups while anything remains would mutate past
    /// the very breach the spool records.
    ///
    /// # Errors
    ///
    /// Returns the exact bounded record/byte counts when recovery is required.
    pub fn require_resolved_for_boot(&self) -> Result<(), UnresolvedQuarantine> {
        let state = self.opened_state;
        if state.unresolved_records == 0 {
            Ok(())
        } else {
            Err(UnresolvedQuarantine {
                unresolved_records: state.unresolved_records,
                unresolved_bytes: state.unresolved_bytes,
            })
        }
    }

    /// Resolve only active records whose authoritative target is an exact
    /// canonical match for the quarantined namespace/key/payload identity.
    ///
    /// This pass performs store reads only. A missing or different target is
    /// left active and continues to block writable boot. An exact match is
    /// moved into a sibling owner-only resolved-evidence archive and both
    /// directories are synced before it stops counting as unresolved.
    ///
    /// The spool's lifetime lock must have been acquired by [`Self::open`] and
    /// remains held across this pass and the caller's transition into ordinary
    /// boot. That is rustmush's existing single-deployment fence; no second
    /// process using the same world data directory can race the comparison.
    ///
    /// # Errors
    ///
    /// Returns an error on an authoritative read failure, a changed/invalid
    /// record, or any failure to preserve and sync resolved evidence. In every
    /// error case, writable boot must remain blocked.
    pub async fn reconcile_exact_matches<R: QuarantineTargetReader + ?Sized>(
        &mut self,
        reader: &R,
    ) -> Result<QuarantineRecoveryReport, QuarantineRecoveryError> {
        self.opened_state = self.current_state()?;
        let targets = self.recovery_targets()?;
        let examined = targets.len();
        let mut resolved = 0usize;
        for target in targets {
            let observed = reader
                .quarantine_target(&target.namespace, &target.key)
                .await
                .map_err(|source| QuarantineRecoveryError::TargetRead {
                    namespace: target.namespace.clone(),
                    key: target.key.clone(),
                    source,
                })?;
            if self.resolve_exact_match(&target.identity, observed.as_ref())? {
                resolved = resolved.saturating_add(1);
            }
        }
        self.opened_state = self.current_state()?;
        Ok(QuarantineRecoveryReport {
            examined,
            resolved,
            remaining: self.opened_state,
        })
    }

    fn current_state(&self) -> Result<SpoolState, QuarantineRecoveryError> {
        match self.backend.as_ref() {
            Backend::InMemory(backend) => backend
                .lock()
                .map(|backend| backend.state)
                .map_err(|_| QuarantineRecoveryError::LockPoisoned),
            Backend::Durable(backend) => backend
                .runtime
                .lock()
                .map(|runtime| runtime.state)
                .map_err(|_| QuarantineRecoveryError::LockPoisoned),
        }
    }

    fn recovery_targets(&self) -> Result<Vec<RecoveryTarget>, QuarantineRecoveryError> {
        match self.backend.as_ref() {
            Backend::InMemory(backend) => {
                let backend = backend
                    .lock()
                    .map_err(|_| QuarantineRecoveryError::LockPoisoned)?;
                let mut targets = Vec::with_capacity(backend.records.len());
                for (identity, record) in &backend.records {
                    validate_record(identity, record).map_err(|detail| {
                        QuarantineBootError::InvalidRecord {
                            path: PathBuf::from(format!("<in-memory>/{identity}.json")),
                            detail,
                        }
                    })?;
                    targets.push(RecoveryTarget {
                        identity: identity.clone(),
                        namespace: record.ns.clone(),
                        key: record.key.clone(),
                    });
                }
                Ok(targets)
            }
            Backend::Durable(backend) => {
                let _runtime = backend
                    .runtime
                    .lock()
                    .map_err(|_| QuarantineRecoveryError::LockPoisoned)?;
                let paths = spool_paths(&backend.dir)?;
                let mut targets = Vec::with_capacity(paths.len());
                for path in paths {
                    if path.extension().is_some_and(|extension| extension == "tmp") {
                        return Err(QuarantineBootError::PartialRecord(path).into());
                    }
                    let bytes = read_bounded(&path)?;
                    let record = decode_and_validate(&path, &bytes)?;
                    let identity = filename_identity(&path)
                        .expect("decode_and_validate accepted the filename")
                        .to_owned();
                    targets.push(RecoveryTarget {
                        identity,
                        namespace: record.ns,
                        key: record.key,
                    });
                }
                Ok(targets)
            }
        }
    }

    fn resolve_exact_match(
        &mut self,
        identity: &str,
        observed: Option<&serde_json::Value>,
    ) -> Result<bool, QuarantineRecoveryError> {
        let Some(observed) = observed else {
            return Ok(false);
        };
        let next_state = match self.backend.as_ref() {
            Backend::InMemory(backend) => {
                let mut backend = backend
                    .lock()
                    .map_err(|_| QuarantineRecoveryError::LockPoisoned)?;
                let record = backend
                    .records
                    .get(identity)
                    .ok_or_else(|| QuarantineRecoveryError::RecordMissing(identity.to_owned()))?;
                if !record_matches_observation(identity, record, observed) {
                    return Ok(false);
                }
                let encoded_len =
                    encode_record(record).map_err(|source| QuarantineBootError::Io {
                        path: PathBuf::from(format!("<in-memory>/{identity}.json")),
                        source,
                    })?;
                backend.records.remove(identity);
                backend.state = release_capacity(backend.state, encoded_len.len())?;
                backend.state
            }
            Backend::Durable(backend) => {
                let mut runtime = backend
                    .runtime
                    .lock()
                    .map_err(|_| QuarantineRecoveryError::LockPoisoned)?;
                let active_path = backend.dir.join(format!("{identity}.json"));
                if !active_path
                    .try_exists()
                    .map_err(|source| QuarantineBootError::Io {
                        path: active_path.clone(),
                        source,
                    })?
                {
                    return Err(QuarantineRecoveryError::RecordMissing(identity.to_owned()));
                }
                let link_metadata = std::fs::symlink_metadata(&active_path).map_err(|source| {
                    QuarantineBootError::Io {
                        path: active_path.clone(),
                        source,
                    }
                })?;
                if !link_metadata.file_type().is_file() {
                    return Err(QuarantineBootError::UnexpectedArtifact(active_path).into());
                }
                let active_metadata =
                    std::fs::metadata(&active_path).map_err(|source| QuarantineBootError::Io {
                        path: active_path.clone(),
                        source,
                    })?;
                let dir_metadata =
                    std::fs::metadata(&backend.dir).map_err(|source| QuarantineBootError::Io {
                        path: backend.dir.clone(),
                        source,
                    })?;
                ensure_same_owner(&dir_metadata, &active_metadata).map_err(|source| {
                    QuarantineBootError::Io {
                        path: active_path.clone(),
                        source,
                    }
                })?;
                enforce_private_file_mode(&active_path).map_err(|source| {
                    QuarantineBootError::Io {
                        path: active_path.clone(),
                        source,
                    }
                })?;
                let bytes = read_bounded(&active_path)?;
                let record = decode_and_validate(&active_path, &bytes)?;
                if !record_matches_observation(identity, &record, observed) {
                    return Ok(false);
                }
                // Compute both bounded accounting transitions before touching
                // either directory. If the evidence archive is full, the
                // active record remains exactly where it was and boot stays
                // blocked.
                let next_active_state = release_capacity(runtime.state, bytes.len())?;
                let next_resolved_state = archive_resolved_record(
                    &backend.dir,
                    &active_path,
                    identity,
                    &bytes,
                    runtime.resolved_state,
                )?;
                runtime.state = next_active_state;
                runtime.resolved_state = next_resolved_state;
                runtime.reservations.remove(identity);
                runtime.state
            }
        };
        self.opened_state = next_state;
        Ok(true)
    }

    /// Preserve one poison seal. `Ok(true)` means the complete recovery record
    /// is present in the required sink; an existing record is revalidated.
    pub(crate) fn quarantine(
        &self,
        ns: &str,
        key: &str,
        value: &serde_json::Value,
        serial: u64,
        err: &StoreError,
        attempts: u32,
    ) -> Result<bool, QuarantineWriteError> {
        let identity = seal_identity_hash(ns, key, value);
        let record = QuarantineRecord {
            record_version: QUARANTINE_RECORD_VERSION,
            ns: ns.to_owned(),
            key: key.to_owned(),
            serial,
            payload_hash: identity.clone(),
            payload: value.clone(),
            store_error: truncate(err.to_string(), MAX_ERROR_BYTES),
            error_transient: err.is_transient(),
            isolation_attempts: attempts,
            quarantined_unix: unix_now_s(),
            server_version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        validate_record(&identity, &record).map_err(QuarantineWriteError::InvalidCandidate)?;
        let bytes = encode_record(&record).map_err(|e| QuarantineWriteError::Durable {
            path: match self.backend.as_ref() {
                Backend::Durable(backend) => backend.dir.clone(),
                Backend::InMemory(_) => PathBuf::from("<in-memory>"),
            },
            detail: e.to_string(),
        })?;

        match self.backend.as_ref() {
            Backend::InMemory(backend) => {
                let mut backend = backend
                    .lock()
                    .map_err(|_| QuarantineWriteError::LockPoisoned)?;
                if backend.records.contains_key(&identity) {
                    return Ok(true);
                }
                let next = reserve_capacity(backend.state, bytes.len())?;
                backend.records.insert(identity, record);
                backend.state = next;
                Ok(true)
            }
            Backend::Durable(backend) => {
                // This mutex intentionally spans the existence check, capacity
                // reservation, durable publication, and post-write readback.
                let mut runtime = backend
                    .runtime
                    .lock()
                    .map_err(|_| QuarantineWriteError::LockPoisoned)?;
                let final_path = backend.dir.join(format!("{identity}.json"));
                if final_path
                    .try_exists()
                    .map_err(|e| QuarantineWriteError::Durable {
                        path: final_path.clone(),
                        detail: e.to_string(),
                    })?
                {
                    let bytes =
                        read_bounded(&final_path).map_err(|e| QuarantineWriteError::Durable {
                            path: final_path.clone(),
                            detail: e.to_string(),
                        })?;
                    decode_and_validate(&final_path, &bytes).map_err(|e| {
                        QuarantineWriteError::Durable {
                            path: final_path.clone(),
                            detail: e.to_string(),
                        }
                    })?;
                    remove_record_temps_if_present(&backend.dir, &identity).map_err(|e| {
                        QuarantineWriteError::Durable {
                            path: final_path.clone(),
                            detail: e.to_string(),
                        }
                    })?;
                    fsync_dir(&backend.dir).map_err(|e| QuarantineWriteError::Durable {
                        path: backend.dir.clone(),
                        detail: e.to_string(),
                    })?;
                    runtime.reservations.remove(&identity);
                    return Ok(true);
                }
                let (stable_envelope, retrying) =
                    reserve_runtime_identity(&mut runtime, &identity, &bytes)?;
                write_record_atomic(
                    &backend.dir,
                    &final_path,
                    &identity,
                    &stable_envelope,
                    retrying,
                )
                .map_err(|e| QuarantineWriteError::Durable {
                    path: final_path.clone(),
                    detail: e.to_string(),
                })?;
                runtime.reservations.remove(&identity);
                // Verify what is actually on disk before allowing the caller to
                // discard its retained copy.
                let bytes =
                    read_bounded(&final_path).map_err(|e| QuarantineWriteError::Durable {
                        path: final_path.clone(),
                        detail: e.to_string(),
                    })?;
                decode_and_validate(&final_path, &bytes).map_err(|e| {
                    QuarantineWriteError::Durable {
                        path: final_path.clone(),
                        detail: e.to_string(),
                    }
                })?;
                Ok(true)
            }
        }
    }
}

fn reserve_capacity(
    current: SpoolState,
    encoded_len: usize,
) -> Result<SpoolState, QuarantineWriteError> {
    let records = current.unresolved_records.saturating_add(1);
    let bytes = current
        .unresolved_bytes
        .saturating_add(u64::try_from(encoded_len).unwrap_or(u64::MAX));
    if records > MAX_RECORDS || bytes > MAX_SPOOL_BYTES {
        return Err(QuarantineWriteError::Capacity { records, bytes });
    }
    Ok(SpoolState {
        unresolved_records: records,
        unresolved_bytes: bytes,
    })
}

fn release_capacity(
    current: SpoolState,
    encoded_len: usize,
) -> Result<SpoolState, QuarantineRecoveryError> {
    let encoded_len =
        u64::try_from(encoded_len).map_err(|_| QuarantineRecoveryError::StateUnderflow)?;
    Ok(SpoolState {
        unresolved_records: current
            .unresolved_records
            .checked_sub(1)
            .ok_or(QuarantineRecoveryError::StateUnderflow)?,
        unresolved_bytes: current
            .unresolved_bytes
            .checked_sub(encoded_len)
            .ok_or(QuarantineRecoveryError::StateUnderflow)?,
    })
}

fn reserve_archive_capacity(
    current: SpoolState,
    encoded_len: usize,
) -> Result<SpoolState, QuarantineRecoveryError> {
    let records = current.unresolved_records.saturating_add(1);
    let bytes = current
        .unresolved_bytes
        .saturating_add(u64::try_from(encoded_len).unwrap_or(u64::MAX));
    if records > MAX_RECORDS || bytes > MAX_SPOOL_BYTES {
        return Err(QuarantineRecoveryError::ArchiveCapacity { records, bytes });
    }
    Ok(SpoolState {
        unresolved_records: records,
        unresolved_bytes: bytes,
    })
}

fn record_matches_observation(
    identity: &str,
    record: &QuarantineRecord,
    observed: &serde_json::Value,
) -> bool {
    if seal_identity_hash(&record.ns, &record.key, observed) != identity {
        return false;
    }
    match (
        mush_core::canonical_json_bytes(&record.payload),
        mush_core::canonical_json_bytes(observed),
    ) {
        (Ok(expected), Ok(actual)) => expected == actual,
        _ => false,
    }
}

/// Move one already-validated active record into the resolved-evidence sibling
/// directory without any crash point at which neither directory holds it.
///
/// The exact record plus its identity/digest-bound name is durable custody for
/// this narrow boot repair. It intentionally is not called a full C11
/// resolution receipt: adding observation metadata requires an atomic envelope
/// publication/recovery protocol rather than weakening this hard-link proof.
fn archive_resolved_record(
    active_dir: &Path,
    active_path: &Path,
    identity: &str,
    bytes: &[u8],
    resolved_state: SpoolState,
) -> Result<SpoolState, QuarantineRecoveryError> {
    let record_digest = sha256_hex(bytes);
    let archive_name = format!("{identity}-{record_digest}.json");
    let requested_archive = resolved_archive_path(active_dir)?;
    let requested_record = requested_archive.join(&archive_name);
    let archive_exists = match std::fs::symlink_metadata(&requested_record) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(source) => {
            return Err(QuarantineBootError::Io {
                path: requested_record,
                source,
            }
            .into());
        }
    };
    // Charge an absent destination before even creating the archive
    // directory. An exact pre-existing destination is a crash-retry and does
    // not consume the bound twice.
    let next_resolved_state = if archive_exists {
        resolved_state
    } else {
        reserve_archive_capacity(resolved_state, bytes.len())?
    };

    let archive_dir = prepare_resolved_archive(active_dir)?;
    let archive_path = archive_dir.join(archive_name);

    let source_consumed = if archive_exists {
        let link_metadata =
            std::fs::symlink_metadata(&archive_path).map_err(|source| QuarantineBootError::Io {
                path: archive_path.clone(),
                source,
            })?;
        if !link_metadata.file_type().is_file() {
            return Err(QuarantineBootError::UnexpectedArtifact(archive_path).into());
        }
        let archive_dir_metadata =
            std::fs::metadata(&archive_dir).map_err(|source| QuarantineBootError::Io {
                path: archive_dir.clone(),
                source,
            })?;
        let archive_file_metadata =
            std::fs::metadata(&archive_path).map_err(|source| QuarantineBootError::Io {
                path: archive_path.clone(),
                source,
            })?;
        ensure_same_owner(&archive_dir_metadata, &archive_file_metadata).map_err(|source| {
            QuarantineBootError::Io {
                path: archive_path.clone(),
                source,
            }
        })?;
        enforce_private_file_mode(&archive_path).map_err(|source| QuarantineBootError::Io {
            path: archive_path.clone(),
            source,
        })?;
        let archived = read_bounded(&archive_path)?;
        decode_and_validate_for_identity(&archive_path, &archived, identity)?;
        if archived != bytes {
            return Err(QuarantineBootError::InvalidRecord {
                path: archive_path,
                detail: "resolved-evidence content conflicts with its record digest".to_owned(),
            }
            .into());
        }
        sync_existing_file(&archive_path).map_err(|source| QuarantineBootError::Io {
            path: archive_path.clone(),
            source,
        })?;
        false
    } else {
        sync_existing_file(active_path).map_err(|source| QuarantineBootError::Io {
            path: active_path.to_owned(),
            source,
        })?;
        publish_resolved_record(active_path, &archive_path).map_err(|source| {
            QuarantineBootError::Io {
                path: archive_path.clone(),
                source,
            }
        })?
    };

    // The archive directory entry becomes durable before the active entry is
    // removed. On Windows `rename` consumed the active name atomically; on
    // production Unix the new entry is a hard link to the already-synced inode.
    fsync_dir(&archive_dir).map_err(|source| QuarantineBootError::Io {
        path: archive_dir.clone(),
        source,
    })?;
    if !source_consumed {
        std::fs::remove_file(active_path).map_err(|source| QuarantineBootError::Io {
            path: active_path.to_owned(),
            source,
        })?;
    }
    fsync_dir(active_dir).map_err(|source| QuarantineBootError::Io {
        path: active_dir.to_owned(),
        source,
    })?;
    Ok(next_resolved_state)
}

fn resolved_archive_path(active_dir: &Path) -> Result<PathBuf, QuarantineBootError> {
    let parent = active_dir
        .parent()
        .ok_or_else(|| QuarantineBootError::UnexpectedArtifact(active_dir.to_owned()))?;
    let active_name = active_dir
        .file_name()
        .ok_or_else(|| QuarantineBootError::UnexpectedArtifact(active_dir.to_owned()))?;
    let mut archive_name = OsString::from(active_name);
    archive_name.push("-resolved");
    Ok(parent.join(archive_name))
}

fn prepare_resolved_archive(active_dir: &Path) -> Result<PathBuf, QuarantineBootError> {
    let requested = resolved_archive_path(active_dir)?;
    match std::fs::symlink_metadata(&requested) {
        Ok(metadata) if !metadata.file_type().is_dir() => {
            return Err(QuarantineBootError::UnexpectedArtifact(requested));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(QuarantineBootError::Io {
                path: requested,
                source,
            });
        }
    }
    let created = create_spool_directory(&requested).map_err(|source| QuarantineBootError::Io {
        path: requested.clone(),
        source,
    })?;
    let archive = std::fs::canonicalize(&requested).map_err(|source| QuarantineBootError::Io {
        path: requested.clone(),
        source,
    })?;
    let active_metadata =
        std::fs::metadata(active_dir).map_err(|source| QuarantineBootError::Io {
            path: active_dir.to_owned(),
            source,
        })?;
    let archive_metadata =
        std::fs::metadata(&archive).map_err(|source| QuarantineBootError::Io {
            path: archive.clone(),
            source,
        })?;
    if !archive_metadata.is_dir() {
        return Err(QuarantineBootError::NotDirectory(archive));
    }
    ensure_same_owner(&active_metadata, &archive_metadata).map_err(|source| {
        QuarantineBootError::Io {
            path: archive.clone(),
            source,
        }
    })?;
    if created {
        fsync_parent(&archive).map_err(|source| QuarantineBootError::Io {
            path: archive.clone(),
            source,
        })?;
    }
    Ok(archive)
}

fn scan_resolved_archive(
    active_dir: &Path,
    active_metadata: &std::fs::Metadata,
) -> Result<SpoolState, QuarantineBootError> {
    let requested = resolved_archive_path(active_dir)?;
    match std::fs::symlink_metadata(&requested) {
        Ok(metadata) if !metadata.file_type().is_dir() => {
            return Err(QuarantineBootError::UnexpectedArtifact(requested));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SpoolState::default());
        }
        Err(source) => {
            return Err(QuarantineBootError::Io {
                path: requested,
                source,
            });
        }
    }

    // Narrow an archive left by an older release before reading recovery
    // payloads. It is never accepted through a caller-supplied record path:
    // both the directory and every filename are derived and validated here.
    create_spool_directory(&requested).map_err(|source| QuarantineBootError::Io {
        path: requested.clone(),
        source,
    })?;
    let archive = std::fs::canonicalize(&requested).map_err(|source| QuarantineBootError::Io {
        path: requested.clone(),
        source,
    })?;
    let archive_metadata =
        std::fs::metadata(&archive).map_err(|source| QuarantineBootError::Io {
            path: archive.clone(),
            source,
        })?;
    if !archive_metadata.is_dir() {
        return Err(QuarantineBootError::NotDirectory(archive));
    }
    ensure_same_owner(active_metadata, &archive_metadata).map_err(|source| {
        QuarantineBootError::Io {
            path: archive.clone(),
            source,
        }
    })?;

    let entries = std::fs::read_dir(&archive).map_err(|source| QuarantineBootError::Io {
        path: archive.clone(),
        source,
    })?;
    let mut state = SpoolState::default();
    for entry in entries {
        let entry = entry.map_err(|source| QuarantineBootError::Io {
            path: archive.clone(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| QuarantineBootError::Io {
                path: path.clone(),
                source,
            })?;
        if !file_type.is_file() {
            return Err(QuarantineBootError::UnexpectedArtifact(path));
        }
        state.unresolved_records = state
            .unresolved_records
            .checked_add(1)
            .ok_or(QuarantineBootError::TooManyRecords)?;
        if state.unresolved_records > MAX_RECORDS {
            return Err(QuarantineBootError::TooManyRecords);
        }
        let (identity, record_digest) = resolved_filename_parts(&path)
            .ok_or_else(|| QuarantineBootError::UnexpectedArtifact(path.clone()))?;
        let file_metadata = std::fs::metadata(&path).map_err(|source| QuarantineBootError::Io {
            path: path.clone(),
            source,
        })?;
        ensure_same_owner(&archive_metadata, &file_metadata).map_err(|source| {
            QuarantineBootError::Io {
                path: path.clone(),
                source,
            }
        })?;
        enforce_private_file_mode(&path).map_err(|source| QuarantineBootError::Io {
            path: path.clone(),
            source,
        })?;
        let bytes = read_bounded(&path)?;
        decode_and_validate_for_identity(&path, &bytes, identity)?;
        if sha256_hex(&bytes) != record_digest {
            return Err(QuarantineBootError::InvalidRecord {
                path,
                detail: "resolved-evidence digest does not match its filename".to_owned(),
            });
        }
        state.unresolved_bytes = state
            .unresolved_bytes
            .checked_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
            .ok_or(QuarantineBootError::ByteCountOverflow)?;
        if state.unresolved_bytes > MAX_SPOOL_BYTES {
            return Err(QuarantineBootError::SpoolTooLarge);
        }
    }
    Ok(state)
}

fn resolved_filename_parts(path: &Path) -> Option<(&str, &str)> {
    let stem = path.file_stem()?.to_str()?;
    let (identity, digest) = stem.split_once('-')?;
    if identity.len() != 64 || digest.len() != 64 {
        return None;
    }
    let lowercase_hex = |value: &str| {
        value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    (lowercase_hex(identity) && lowercase_hex(digest)).then_some((identity, digest))
}

#[cfg(windows)]
fn publish_resolved_record(active_path: &Path, archive_path: &Path) -> std::io::Result<bool> {
    // The lifetime lock makes the checked-absent destination application-
    // exclusive; Windows rename refuses replacement and atomically transfers
    // the one durable name into the evidence directory.
    std::fs::rename(active_path, archive_path)?;
    Ok(true)
}

#[cfg(not(windows))]
fn publish_resolved_record(active_path: &Path, archive_path: &Path) -> std::io::Result<bool> {
    // Atomic no-overwrite publication while retaining active custody until the
    // archive directory entry has itself been synced.
    std::fs::hard_link(active_path, archive_path)?;
    Ok(false)
}

fn decode_and_validate_for_identity(
    path: &Path,
    bytes: &[u8],
    identity: &str,
) -> Result<QuarantineRecord, QuarantineBootError> {
    let record: QuarantineRecord =
        serde_json::from_slice(bytes).map_err(|error| QuarantineBootError::InvalidRecord {
            path: path.to_owned(),
            detail: truncate(error.to_string(), MAX_ERROR_BYTES),
        })?;
    validate_record(identity, &record).map_err(|detail| QuarantineBootError::InvalidRecord {
        path: path.to_owned(),
        detail,
    })?;
    Ok(record)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

fn reserve_runtime_identity(
    runtime: &mut RuntimeState,
    identity: &str,
    envelope: &[u8],
) -> Result<(Arc<[u8]>, bool), QuarantineWriteError> {
    if let Some(reservation) = runtime.reservations.get(identity) {
        return Ok((Arc::clone(&reservation.envelope), true));
    }

    runtime.state = reserve_capacity(runtime.state, envelope.len())?;
    let envelope = Arc::<[u8]>::from(envelope);
    runtime.reservations.insert(
        identity.to_owned(),
        RuntimeReservation {
            envelope: Arc::clone(&envelope),
        },
    );
    Ok((envelope, false))
}

fn create_spool_directory(path: &Path) -> std::io::Result<bool> {
    let existed = path.try_exists()?;

    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        // Apply the restrictive mode at creation rather than creating with a
        // permissive default and narrowing it afterward.
        builder.mode(0o700);
    }
    builder.create(path)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        // Also narrow directories created by older releases. Refuse startup if
        // the service account cannot make recovery payloads owner-only.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }

    Ok(!existed)
}

fn lock_path_for(dir: &Path) -> std::io::Result<PathBuf> {
    let parent = dir.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "quarantine spool has no parent directory",
        )
    })?;
    let name = dir.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "quarantine spool has no directory name",
        )
    })?;
    let mut lock_name = OsString::from(".");
    lock_name.push(name);
    lock_name.push(".rustmush-quarantine.lock");
    Ok(parent.join(lock_name))
}

fn open_lifetime_lock(dir: &Path) -> Result<File, (PathBuf, std::io::Error)> {
    let lock_path = lock_path_for(dir).map_err(|source| (dir.to_owned(), source))?;
    let file = match lifetime_lock_options(true).open(&lock_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            lifetime_lock_options(false)
                .open(&lock_path)
                .map_err(|source| (lock_path.clone(), source))?
        }
        Err(source) => return Err((lock_path, source)),
    };
    let dir_metadata = std::fs::metadata(dir).map_err(|source| (dir.to_owned(), source))?;
    let lock_metadata = file
        .metadata()
        .map_err(|source| (lock_path.clone(), source))?;
    let lock_link_metadata =
        std::fs::symlink_metadata(&lock_path).map_err(|source| (lock_path.clone(), source))?;
    if !lock_metadata.is_file() || !lock_link_metadata.file_type().is_file() {
        return Err((
            lock_path,
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "quarantine lifetime lock is not a regular file",
            ),
        ));
    }
    ensure_same_owner(&dir_metadata, &lock_metadata)
        .map_err(|source| (lock_path.clone(), source))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|source| (lock_path.clone(), source))?;
    }
    file.sync_all()
        .map_err(|source| (lock_path.clone(), source))?;
    file.try_lock()
        .map_err(std::io::Error::from)
        .map_err(|source| (lock_path, source))?;
    Ok(file)
}

fn lifetime_lock_options(create_new: bool) -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(create_new);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
}

fn secure_create_new(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(unix)]
fn enforce_private_file_mode(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn enforce_private_file_mode(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn ensure_same_owner(
    expected: &std::fs::Metadata,
    actual: &std::fs::Metadata,
) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt as _;
    if expected.uid() == actual.uid() {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "quarantine artifact owner differs from spool directory owner",
        ))
    }
}

#[cfg(not(unix))]
fn ensure_same_owner(
    _expected: &std::fs::Metadata,
    _actual: &std::fs::Metadata,
) -> std::io::Result<()> {
    Ok(())
}

fn fsync_parent(path: &Path) -> std::io::Result<()> {
    match path.parent() {
        Some(parent) => fsync_dir(parent),
        None => Ok(()),
    }
}

fn spool_paths(dir: &Path) -> Result<Vec<PathBuf>, QuarantineBootError> {
    let entries = std::fs::read_dir(dir).map_err(|source| QuarantineBootError::Io {
        path: dir.to_owned(),
        source,
    })?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| QuarantineBootError::Io {
            path: dir.to_owned(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| QuarantineBootError::Io {
                path: path.clone(),
                source,
            })?;
        if !file_type.is_file()
            || !matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("json" | "tmp")
            )
        {
            return Err(QuarantineBootError::UnexpectedArtifact(path));
        }
        paths.push(path);
    }
    Ok(paths)
}

fn recover_temps_at_boot(
    dir: &Path,
    dir_metadata: &std::fs::Metadata,
    paths: &[PathBuf],
) -> Result<(), QuarantineBootError> {
    let mut groups: BTreeMap<String, Vec<&Path>> = BTreeMap::new();
    for temp_path in paths
        .iter()
        .filter(|candidate| candidate.extension().is_some_and(|ext| ext == "tmp"))
    {
        let identity = temp_filename_identity(temp_path)
            .ok_or_else(|| QuarantineBootError::PartialRecord(temp_path.clone()))?;
        let group = groups.entry(identity.to_owned()).or_default();
        group.push(temp_path);
        if group.len() > 2 {
            return Err(QuarantineBootError::UnexpectedArtifact(temp_path.clone()));
        }
    }

    for (identity, temp_paths) in groups {
        recover_temp_group_at_boot(dir, dir_metadata, &identity, &temp_paths)?;
    }
    Ok(())
}

fn recover_temp_group_at_boot(
    dir: &Path,
    dir_metadata: &std::fs::Metadata,
    identity: &str,
    temp_paths: &[&Path],
) -> Result<(), QuarantineBootError> {
    let mut candidates = Vec::with_capacity(temp_paths.len());
    for &temp_path in temp_paths {
        enforce_private_file_mode(temp_path).map_err(|source| QuarantineBootError::Io {
            path: temp_path.to_owned(),
            source,
        })?;
        let temp_metadata =
            std::fs::metadata(temp_path).map_err(|source| QuarantineBootError::Io {
                path: temp_path.to_owned(),
                source,
            })?;
        ensure_same_owner(dir_metadata, &temp_metadata).map_err(|source| {
            QuarantineBootError::Io {
                path: temp_path.to_owned(),
                source,
            }
        })?;
        candidates.push((temp_path, read_bounded(temp_path)?));
    }

    let Some((valid_path, valid_bytes)) = candidates
        .iter()
        .find(|(_, bytes)| temp_bytes_match_identity(bytes, identity))
    else {
        let (path, _) = candidates
            .first()
            .expect("temp group always contains at least one path");
        return Err(QuarantineBootError::InvalidRecord {
            path: (*path).to_owned(),
            detail: "temporary record is incomplete, malformed, or conflicts with its identity"
                .to_owned(),
        });
    };
    // A sibling left by the two-slot live recovery protocol may be partial,
    // but it must be a byte prefix of the now-verified envelope. Anything else
    // is conflicting evidence and remains fail-closed.
    for (path, bytes) in &candidates {
        if *path != *valid_path
            && !temp_bytes_match_identity(bytes, identity)
            && !valid_bytes.starts_with(bytes)
        {
            return Err(QuarantineBootError::InvalidRecord {
                path: (*path).to_owned(),
                detail: "temporary record conflicts with the verified recovery envelope".to_owned(),
            });
        }
    }

    sync_existing_file(valid_path).map_err(|source| QuarantineBootError::Io {
        path: (*valid_path).to_owned(),
        source,
    })?;

    let final_path = dir.join(format!("{identity}.json"));
    let source_consumed = if final_path
        .try_exists()
        .map_err(|source| QuarantineBootError::Io {
            path: final_path.clone(),
            source,
        })? {
        enforce_private_file_mode(&final_path).map_err(|source| QuarantineBootError::Io {
            path: final_path.clone(),
            source,
        })?;
        let final_metadata =
            std::fs::metadata(&final_path).map_err(|source| QuarantineBootError::Io {
                path: final_path.clone(),
                source,
            })?;
        if !final_metadata.is_file() {
            return Err(QuarantineBootError::UnexpectedArtifact(final_path));
        }
        ensure_same_owner(dir_metadata, &final_metadata).map_err(|source| {
            QuarantineBootError::Io {
                path: final_path.clone(),
                source,
            }
        })?;
        let final_bytes = read_bounded(&final_path)?;
        decode_and_validate(&final_path, &final_bytes)?;
        sync_existing_file(&final_path).map_err(|source| QuarantineBootError::Io {
            path: final_path.clone(),
            source,
        })?;
        fsync_dir(dir).map_err(|source| QuarantineBootError::Io {
            path: dir.to_owned(),
            source,
        })?;
        false
    } else {
        let consumed = publish_recovered_temp(valid_path, &final_path).map_err(|source| {
            QuarantineBootError::Io {
                path: final_path.clone(),
                source,
            }
        })?;
        fsync_dir(dir).map_err(|source| QuarantineBootError::Io {
            path: dir.to_owned(),
            source,
        })?;
        consumed
    };

    for &temp_path in temp_paths {
        if source_consumed && temp_path == *valid_path {
            continue;
        }
        remove_file_if_present(temp_path).map_err(|source| QuarantineBootError::Io {
            path: temp_path.to_owned(),
            source,
        })?;
    }
    fsync_dir(dir).map_err(|source| QuarantineBootError::Io {
        path: dir.to_owned(),
        source,
    })?;
    Ok(())
}

#[cfg(windows)]
fn publish_recovered_temp(temp_path: &Path, final_path: &Path) -> std::io::Result<bool> {
    // Windows refuses unlinking some pre-existing hard-link sources even after
    // all Rust handles are scoped away. The spool lifetime lock makes this
    // absent-destination rename atomic within the application.
    std::fs::rename(temp_path, final_path)?;
    Ok(true)
}

#[cfg(not(windows))]
fn publish_recovered_temp(temp_path: &Path, final_path: &Path) -> std::io::Result<bool> {
    // Hard-link publication is atomic and refuses to overwrite an existing
    // destination on the production Unix path.
    std::fs::hard_link(temp_path, final_path)?;
    Ok(false)
}

fn temp_bytes_match_identity(bytes: &[u8], identity: &str) -> bool {
    serde_json::from_slice::<QuarantineRecord>(bytes)
        .is_ok_and(|record| validate_record(identity, &record).is_ok())
}

fn sync_existing_file(path: &Path) -> std::io::Result<()> {
    // Windows requires a write-capable handle for FlushFileBuffers.
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    file.sync_all()
}

fn temp_filename_identity(path: &Path) -> Option<&str> {
    let name = path.file_name()?.to_str()?;
    let without_tmp = name.strip_suffix(".tmp")?;
    let identity = without_tmp.strip_suffix(".recovery").unwrap_or(without_tmp);
    (identity.len() == 64
        && identity
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    .then_some(identity)
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, QuarantineBootError> {
    let metadata = std::fs::metadata(path).map_err(|source| QuarantineBootError::Io {
        path: path.to_owned(),
        source,
    })?;
    if metadata.len() > MAX_RECORD_BYTES {
        return Err(QuarantineBootError::RecordTooLarge(path.to_owned()));
    }
    let file = File::open(path).map_err(|source| QuarantineBootError::Io {
        path: path.to_owned(),
        source,
    })?;
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(MAX_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| QuarantineBootError::Io {
            path: path.to_owned(),
            source,
        })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_RECORD_BYTES {
        return Err(QuarantineBootError::RecordTooLarge(path.to_owned()));
    }
    Ok(bytes)
}

fn decode_and_validate(path: &Path, bytes: &[u8]) -> Result<QuarantineRecord, QuarantineBootError> {
    let record: QuarantineRecord =
        serde_json::from_slice(bytes).map_err(|e| QuarantineBootError::InvalidRecord {
            path: path.to_owned(),
            detail: truncate(e.to_string(), MAX_ERROR_BYTES),
        })?;
    let identity = filename_identity(path).ok_or_else(|| QuarantineBootError::InvalidRecord {
        path: path.to_owned(),
        detail: "filename must be 64 lowercase SHA-256 hex characters plus .json".to_owned(),
    })?;
    validate_record(identity, &record).map_err(|detail| QuarantineBootError::InvalidRecord {
        path: path.to_owned(),
        detail,
    })?;
    Ok(record)
}

fn filename_identity(path: &Path) -> Option<&str> {
    let stem = path.file_stem()?.to_str()?;
    (stem.len() == 64
        && stem
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    .then_some(stem)
}

fn validate_record(expected_identity: &str, record: &QuarantineRecord) -> Result<(), String> {
    if record.record_version != QUARANTINE_RECORD_VERSION {
        return Err(format!(
            "unsupported record_version {} (expected {QUARANTINE_RECORD_VERSION})",
            record.record_version
        ));
    }
    if record.ns.is_empty() || record.ns.len() > MAX_MODULE_NAMESPACE_BYTES {
        return Err("namespace is empty or too large".to_owned());
    }
    if record.key.is_empty() || record.key.len() > MAX_MODULE_KEY_BYTES {
        return Err("key is empty or too large".to_owned());
    }
    if record.store_error.len() > MAX_ERROR_BYTES || record.server_version.len() > MAX_VERSION_BYTES
    {
        return Err("diagnostic field is too large".to_owned());
    }
    if record.payload_hash != expected_identity {
        return Err("payload_hash does not match filename identity".to_owned());
    }
    let actual = seal_identity_hash(&record.ns, &record.key, &record.payload);
    if actual != expected_identity {
        return Err("SHA-256 identity does not match namespace/key/payload bytes".to_owned());
    }
    validate_sealed_payload(&record.key, &record.payload)?;
    Ok(())
}

fn validate_sealed_payload(key: &str, value: &serde_json::Value) -> Result<(), String> {
    if !mush_core::is_sealed_chunk(key, value) {
        return Err("payload is not a sealed ledger chunk".to_owned());
    }
    let chunk: LedgerChunk = serde_json::from_value(value.clone())
        .map_err(|e| format!("invalid ledger chunk payload: {e}"))?;
    if !chunk.sealed {
        return Err("ledger chunk is not sealed".to_owned());
    }
    let key_chunk = key
        .rsplit_once(':')
        .and_then(|(_, seq)| seq.parse::<u64>().ok())
        .ok_or_else(|| "invalid ledger chunk key".to_owned())?;
    if key_chunk != chunk.chunk {
        return Err("ledger chunk number does not match its key".to_owned());
    }
    if chunk.entries.is_empty() {
        return Err("sealed ledger chunk has no entries".to_owned());
    }
    for pair in chunk.entries.windows(2) {
        if pair[0].seq.checked_add(1) != Some(pair[1].seq) {
            return Err("ledger chunk entries are not contiguous".to_owned());
        }
    }
    Ok(())
}

fn write_record_atomic(
    dir: &Path,
    final_path: &Path,
    identity: &str,
    bytes: &[u8],
    retrying: bool,
) -> std::io::Result<()> {
    let source = prepare_runtime_temp(dir, identity, bytes, retrying)?;

    // `rename` replaces an existing destination on Unix. A hard link gives us
    // an atomic, no-overwrite publication: both names briefly reference the
    // already-synced inode, and publication fails closed if `final_path`
    // appears despite the process lock.
    std::fs::hard_link(&source, final_path)?;
    fsync_dir(dir)?;
    remove_record_temps_if_present(dir, identity)?;
    Ok(())
}

fn record_temp_path(dir: &Path, identity: &str) -> PathBuf {
    dir.join(format!("{identity}.tmp"))
}

fn record_recovery_temp_path(dir: &Path, identity: &str) -> PathBuf {
    dir.join(format!("{identity}.recovery.tmp"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeTempState {
    Missing,
    Valid,
    Partial,
}

fn inspect_runtime_temp(
    path: &Path,
    identity: &str,
    expected: &[u8],
) -> std::io::Result<RuntimeTempState> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RuntimeTempState::Missing);
        }
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    if metadata.len() > MAX_RECORD_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "existing quarantine temp exceeds its encoded-size bound",
        ));
    }
    let mut actual = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(MAX_RECORD_BYTES + 1).read_to_end(&mut actual)?;
    if let Ok(record) = serde_json::from_slice::<QuarantineRecord>(&actual)
        && validate_record(identity, &record).is_ok()
    {
        return Ok(RuntimeTempState::Valid);
    }
    if expected.starts_with(&actual) {
        return Ok(RuntimeTempState::Partial);
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "existing quarantine temp conflicts with the retained exact envelope",
    ))
}

fn prepare_runtime_temp(
    dir: &Path,
    identity: &str,
    expected: &[u8],
    retrying: bool,
) -> std::io::Result<PathBuf> {
    let primary = record_temp_path(dir, identity);
    let recovery = record_recovery_temp_path(dir, identity);
    let primary_state = inspect_runtime_temp(&primary, identity, expected)?;
    let recovery_state = inspect_runtime_temp(&recovery, identity, expected)?;

    if primary_state == RuntimeTempState::Valid {
        return Ok(primary);
    }
    if recovery_state == RuntimeTempState::Valid {
        return Ok(recovery);
    }
    let has_partial =
        primary_state == RuntimeTempState::Partial || recovery_state == RuntimeTempState::Partial;
    if has_partial && !retrying {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "unexpected quarantine temp appeared before this process attempted publication",
        ));
    }

    let target = if primary_state == RuntimeTempState::Missing {
        primary
    } else if recovery_state == RuntimeTempState::Missing {
        recovery
    } else {
        // Both fixed slots contain prefixes written by this process. Preserve
        // the primary while replacing the recovery slot, so there is never a
        // crash point with no on-disk custody artifact and retries remain
        // bounded to two temp files.
        std::fs::remove_file(&recovery)?;
        fsync_dir(dir)?;
        recovery
    };
    let mut file = secure_create_new(&target)?;
    file.write_all(expected)?;
    file.sync_all()?;
    drop(file);
    Ok(target)
}

fn remove_record_temps_if_present(dir: &Path, identity: &str) -> std::io::Result<()> {
    let removed_primary = remove_file_if_present(&record_temp_path(dir, identity))?;
    let removed_recovery = remove_file_if_present(&record_recovery_temp_path(dir, identity))?;
    if removed_primary || removed_recovery {
        fsync_dir(dir)?;
    }
    Ok(())
}

fn remove_file_if_present(path: &Path) -> std::io::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn encode_record(record: &QuarantineRecord) -> std::io::Result<Vec<u8>> {
    // Compact JSON keeps the durable envelope within a predictable constant
    // overhead of the core ledger's bounded canonical payload. Pretty-printing
    // can multiply whitespace for millions of small entries and turn an
    // otherwise valid <=8 MiB seal into an unrecoverable >16 MiB record.
    let bytes = serde_json::to_vec(record).map_err(std::io::Error::other)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_RECORD_BYTES {
        return Err(std::io::Error::other(
            "quarantine record exceeds encoded-size bound",
        ));
    }
    Ok(bytes)
}

fn probe_write_capability(dir: &Path) -> std::io::Result<()> {
    let sequence = PROBE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let identity = format!(".rustmush-write-probe-{}-{sequence}", std::process::id());
    let tmp = dir.join(format!("{identity}.tmp"));
    let renamed = dir.join(format!("{identity}.probe"));
    let mut file = secure_create_new(&tmp)?;
    ensure_same_owner(&std::fs::metadata(dir)?, &file.metadata()?)?;
    file.write_all(b"rustmush quarantine write probe\n")?;
    file.sync_all()?;
    drop(file);
    std::fs::hard_link(&tmp, &renamed)?;
    fsync_dir(dir)?;
    std::fs::remove_file(&tmp)?;
    fsync_dir(dir)?;
    std::fs::remove_file(&renamed)?;
    fsync_dir(dir)?;
    Ok(())
}

#[cfg(unix)]
fn fsync_dir(dir: &Path) -> std::io::Result<()> {
    File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn fsync_dir(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Stable SHA-256 over namespace, key, and canonical JSON payload bytes.
#[must_use]
pub fn seal_identity_hash(ns: &str, key: &str, value: &serde_json::Value) -> String {
    use sha2::{Digest as _, Sha256};
    const DOMAIN: &[u8] = b"rustmush/quarantine-seal/v1";

    fn hash_field(hasher: &mut Sha256, bytes: &[u8]) {
        let len = u64::try_from(bytes.len()).expect("field length fits u64");
        hasher.update(len.to_be_bytes());
        hasher.update(bytes);
    }

    let payload = mush_core::canonical_json_bytes(value)
        .expect("canonical encoding of an allocated serde_json::Value cannot exceed usize::MAX");
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    hash_field(&mut hasher, ns.as_bytes());
    hash_field(&mut hasher, key.as_bytes());
    hash_field(&mut hasher, &payload);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

fn truncate(mut value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let mut boundary = max_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    value
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use mush_core::{LedgerEntry, chunk_key};

    struct RecoveryReader {
        expected_namespace: String,
        expected_key: String,
        result: Mutex<Option<Result<Option<serde_json::Value>, StoreError>>>,
        reads: AtomicUsize,
    }

    impl RecoveryReader {
        fn value(namespace: &str, key: &str, value: Option<serde_json::Value>) -> Self {
            Self {
                expected_namespace: namespace.to_owned(),
                expected_key: key.to_owned(),
                result: Mutex::new(Some(Ok(value))),
                reads: AtomicUsize::new(0),
            }
        }

        fn failure(namespace: &str, key: &str, error: StoreError) -> Self {
            Self {
                expected_namespace: namespace.to_owned(),
                expected_key: key.to_owned(),
                result: Mutex::new(Some(Err(error))),
                reads: AtomicUsize::new(0),
            }
        }
    }

    impl QuarantineTargetReader for RecoveryReader {
        fn quarantine_target<'a>(
            &'a self,
            namespace: &'a str,
            key: &'a str,
        ) -> BoxFuture<'a, Result<Option<serde_json::Value>, StoreError>> {
            Box::pin(async move {
                assert_eq!(namespace, self.expected_namespace);
                assert_eq!(key, self.expected_key);
                self.reads.fetch_add(1, Ordering::Relaxed);
                self.result
                    .lock()
                    .unwrap()
                    .take()
                    .expect("one recovery read expected")
            })
        }
    }

    fn temp_spool(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rustmush-quarantine-{label}-{}-{}",
            std::process::id(),
            unix_now_s()
        ))
    }

    fn seal() -> (String, serde_json::Value) {
        let key = chunk_key("test", 3).to_string();
        let payload = serde_json::to_value(LedgerChunk {
            chunk: 3,
            sealed: true,
            entries: vec![LedgerEntry {
                seq: 9,
                value: serde_json::json!({"ok": true}),
            }],
        })
        .unwrap();
        (key, payload)
    }

    fn resolved_dir(active: &Path) -> PathBuf {
        let mut name = OsString::from(active.file_name().unwrap());
        name.push("-resolved");
        active.parent().unwrap().join(name)
    }

    #[tokio::test]
    async fn exact_authoritative_match_is_archived_and_unblocks_boot() {
        let dir = temp_spool("exact-recovery");
        let archive = resolved_dir(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&archive);
        let mut spool = QuarantineSpool::open(&dir).unwrap();
        let (key, payload) = seal();
        spool
            .quarantine(
                "test",
                &key,
                &payload,
                7,
                &StoreError::Invalid("poison".to_owned()),
                5,
            )
            .unwrap();
        let active_path = std::fs::read_dir(&dir)
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .unwrap()
            .path();
        let active_bytes = std::fs::read(&active_path).unwrap();
        let reader = RecoveryReader::value("test", &key, Some(payload));

        let report = spool.reconcile_exact_matches(&reader).await.unwrap();
        assert_eq!(report.examined, 1);
        assert_eq!(report.resolved, 1);
        assert_eq!(report.remaining, SpoolState::default());
        spool.require_resolved_for_boot().unwrap();
        assert_eq!(reader.reads.load(Ordering::Relaxed), 1);
        assert!(!active_path.exists());
        let archived: Vec<PathBuf> = std::fs::read_dir(&archive)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(archived.len(), 1);
        assert_eq!(std::fs::read(&archived[0]).unwrap(), active_bytes);
        drop(spool);
        assert_eq!(
            QuarantineSpool::open(&dir)
                .unwrap()
                .opened_state()
                .unresolved_records,
            0
        );

        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(archive).unwrap();
    }

    #[tokio::test]
    async fn restart_reconciles_archive_and_active_crash_pair_idempotently() {
        let dir = temp_spool("resolved-crash-pair");
        let archive = resolved_dir(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&archive);
        let mut spool = QuarantineSpool::open(&dir).unwrap();
        let (key, payload) = seal();
        spool
            .quarantine(
                "test",
                &key,
                &payload,
                9,
                &StoreError::Invalid("poison".to_owned()),
                5,
            )
            .unwrap();
        let active_path = std::fs::read_dir(&dir)
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .unwrap()
            .path();
        let reader = RecoveryReader::value("test", &key, Some(payload.clone()));
        spool.reconcile_exact_matches(&reader).await.unwrap();
        let archive_path = std::fs::read_dir(&archive)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();

        // Simulate a production-Unix crash after the archive link and its
        // directory sync but before removal of the active name. A byte copy is
        // portable to the Windows development test path and represents the
        // same two valid durable names.
        std::fs::copy(&archive_path, &active_path).unwrap();
        drop(spool);

        let mut reopened = QuarantineSpool::open(&dir).unwrap();
        assert_eq!(reopened.opened_state().unresolved_records, 1);
        let reader = RecoveryReader::value("test", &key, Some(payload));
        let report = reopened.reconcile_exact_matches(&reader).await.unwrap();
        assert_eq!(report.resolved, 1);
        assert_eq!(report.remaining, SpoolState::default());
        assert!(!active_path.exists());
        assert_eq!(
            std::fs::read_dir(&archive).unwrap().count(),
            1,
            "the existing evidence record is not charged or copied twice"
        );
        drop(reopened);

        let verified = QuarantineSpool::open(&dir).unwrap();
        assert_eq!(verified.opened_state(), SpoolState::default());
        drop(verified);
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(archive).unwrap();
    }

    #[test]
    fn malformed_or_untrusted_resolved_archive_blocks_open() {
        let dir = temp_spool("invalid-resolved-archive");
        let archive = resolved_dir(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&archive);
        let spool = QuarantineSpool::open(&dir).unwrap();
        drop(spool);
        std::fs::create_dir(&archive).unwrap();
        let untrusted = archive.join(format!("{}-{}.json", "0".repeat(64), "0".repeat(64)));
        std::fs::write(&untrusted, b"not a quarantine record").unwrap();

        assert!(
            QuarantineSpool::open(&dir).is_err(),
            "resolved evidence is a validated collection, not an opaque path archive"
        );

        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(archive).unwrap();
    }

    #[test]
    fn full_resolved_archive_leaves_active_record_unmodified() {
        let dir = temp_spool("full-resolved-archive");
        let archive = resolved_dir(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&archive);
        let spool = QuarantineSpool::open(&dir).unwrap();
        let (key, payload) = seal();
        spool
            .quarantine(
                "test",
                &key,
                &payload,
                10,
                &StoreError::Invalid("poison".to_owned()),
                5,
            )
            .unwrap();
        let active_path = std::fs::read_dir(&dir)
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .unwrap()
            .path();
        let identity = filename_identity(&active_path).unwrap().to_owned();
        let bytes = std::fs::read(&active_path).unwrap();

        let error = archive_resolved_record(
            &dir,
            &active_path,
            &identity,
            &bytes,
            SpoolState {
                unresolved_records: MAX_RECORDS,
                unresolved_bytes: 0,
            },
        )
        .unwrap_err();
        assert!(matches!(
            error,
            QuarantineRecoveryError::ArchiveCapacity { .. }
        ));
        assert_eq!(std::fs::read(&active_path).unwrap(), bytes);
        assert!(!archive.exists());

        drop(spool);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn missing_different_or_unreadable_target_never_resolves_record() {
        for (label, result) in [
            ("missing", Ok(None)),
            (
                "different",
                Ok(Some(serde_json::json!({"sealed": true, "different": true}))),
            ),
            ("unreadable", Err(StoreError::Closed)),
        ] {
            let dir = temp_spool(label);
            let archive = resolved_dir(&dir);
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&archive);
            let mut spool = QuarantineSpool::open(&dir).unwrap();
            let (key, payload) = seal();
            spool
                .quarantine(
                    "test",
                    &key,
                    &payload,
                    8,
                    &StoreError::Invalid("poison".to_owned()),
                    5,
                )
                .unwrap();
            let reader = match result {
                Ok(value) => RecoveryReader::value("test", &key, value),
                Err(error) => RecoveryReader::failure("test", &key, error),
            };

            let outcome = spool.reconcile_exact_matches(&reader).await;
            if label == "unreadable" {
                assert!(matches!(
                    outcome,
                    Err(QuarantineRecoveryError::TargetRead { .. })
                ));
            } else {
                let report = outcome.unwrap();
                assert_eq!(report.resolved, 0);
                assert_eq!(report.remaining.unresolved_records, 1);
            }
            assert!(spool.require_resolved_for_boot().is_err());
            assert!(!archive.exists());
            drop(spool);
            assert_eq!(
                QuarantineSpool::open(&dir)
                    .unwrap()
                    .opened_state()
                    .unresolved_records,
                1
            );

            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn open_rejects_partial_malformed_hash_mismatch_and_unsealed_records() {
        for (label, mutate) in [
            ("partial", 0_u8),
            ("malformed", 1),
            ("hash", 2),
            ("unsealed", 3),
            ("version", 4),
        ] {
            let dir = temp_spool(label);
            let spool = QuarantineSpool::open(&dir).unwrap();
            let (key, payload) = seal();
            spool
                .quarantine(
                    "test",
                    &key,
                    &payload,
                    1,
                    &StoreError::Invalid("poison".to_owned()),
                    5,
                )
                .unwrap();
            let path = std::fs::read_dir(&dir)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            match mutate {
                0 => {
                    let temp = path.with_extension("tmp");
                    let mut bytes = std::fs::read(&path).unwrap();
                    bytes.truncate(bytes.len() / 2);
                    std::fs::rename(&path, &temp).unwrap();
                    std::fs::write(temp, bytes).unwrap();
                }
                1 => std::fs::write(&path, b"not json").unwrap(),
                2 => {
                    let mut record: serde_json::Value =
                        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                    record["payload_hash"] = serde_json::json!("0".repeat(64));
                    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
                }
                3 => {
                    let mut record: serde_json::Value =
                        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                    record["payload"]["sealed"] = serde_json::json!(false);
                    let identity = seal_identity_hash(
                        record["ns"].as_str().unwrap(),
                        record["key"].as_str().unwrap(),
                        &record["payload"],
                    );
                    record["payload_hash"] = serde_json::json!(&identity);
                    let new_path = dir.join(format!("{identity}.json"));
                    std::fs::write(&new_path, serde_json::to_vec(&record).unwrap()).unwrap();
                    std::fs::remove_file(&path).unwrap();
                }
                4 => {
                    let mut record: serde_json::Value =
                        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                    record["record_version"] = serde_json::json!(99);
                    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
                }
                _ => unreachable!(),
            }
            drop(spool);
            assert!(
                QuarantineSpool::open(&dir).is_err(),
                "{label} must fail closed"
            );
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn open_promotes_valid_temp_and_cleans_duplicate_after_durable_final() {
        let dir = temp_spool("recover-valid-temp");
        let spool = QuarantineSpool::open(&dir).unwrap();
        let (key, payload) = seal();
        spool
            .quarantine(
                "test",
                &key,
                &payload,
                1,
                &StoreError::Invalid("poison".to_owned()),
                5,
            )
            .unwrap();
        drop(spool);

        let final_path = std::fs::read_dir(&dir)
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .unwrap()
            .path();
        let temp_path = final_path.with_extension("tmp");
        std::fs::rename(&final_path, &temp_path).unwrap();
        let recovered = QuarantineSpool::open(&dir).unwrap();
        assert!(final_path.is_file());
        assert!(!temp_path.exists());
        assert_eq!(recovered.opened_state().unresolved_records, 1);
        drop(recovered);

        std::fs::hard_link(&final_path, &temp_path).unwrap();
        let deduplicated = QuarantineSpool::open(&dir).unwrap();
        assert!(final_path.is_file());
        assert!(!temp_path.exists());
        assert_eq!(deduplicated.opened_state().unresolved_records, 1);
        drop(deduplicated);

        let final_bytes = std::fs::read(&final_path).unwrap();
        let identity = filename_identity(&final_path).unwrap().to_owned();
        let recovery_temp = record_recovery_temp_path(&dir, &identity);
        std::fs::hard_link(&final_path, &recovery_temp).unwrap();
        std::fs::write(&temp_path, &final_bytes[..final_bytes.len() / 2]).unwrap();
        std::fs::remove_file(&final_path).unwrap();
        let recovered_pair = QuarantineSpool::open(&dir).unwrap();
        assert!(final_path.is_file());
        assert!(!temp_path.exists());
        assert!(!recovery_temp.exists());
        assert_eq!(recovered_pair.opened_state().unresolved_records, 1);
        drop(recovered_pair);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn open_counts_only_fully_verified_records_and_existing_is_rechecked() {
        let dir = temp_spool("valid");
        let spool = QuarantineSpool::open(&dir).unwrap();
        let (key, payload) = seal();
        assert!(
            spool
                .quarantine(
                    "test",
                    &key,
                    &payload,
                    7,
                    &StoreError::Invalid("poison".to_owned()),
                    5,
                )
                .unwrap()
        );
        assert!(
            spool
                .quarantine(
                    "test",
                    &key,
                    &payload,
                    8,
                    &StoreError::Invalid("again".to_owned()),
                    5,
                )
                .unwrap()
        );
        assert!(
            QuarantineSpool::open(&dir).is_err(),
            "a second process must not scan or mutate a live spool"
        );
        drop(spool);
        let reopened = QuarantineSpool::open(&dir).unwrap();
        assert_eq!(reopened.opened_state().unresolved_records, 1);
        assert!(reopened.opened_state().unresolved_bytes > 0);
        drop(reopened);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn runtime_capacity_reservation_is_bounded_before_mutation() {
        assert!(matches!(
            reserve_capacity(
                SpoolState {
                    unresolved_records: 0,
                    unresolved_bytes: MAX_SPOOL_BYTES,
                },
                1,
            ),
            Err(QuarantineWriteError::Capacity { .. })
        ));

        let spool = QuarantineSpool::in_memory();
        let Backend::InMemory(backend) = spool.backend.as_ref() else {
            unreachable!()
        };
        backend.lock().unwrap().state = SpoolState {
            unresolved_records: MAX_RECORDS,
            unresolved_bytes: 0,
        };
        let (key, payload) = seal();
        assert!(matches!(
            spool.quarantine(
                "test",
                &key,
                &payload,
                1,
                &StoreError::Invalid("poison".to_owned()),
                5,
            ),
            Err(QuarantineWriteError::Capacity { .. })
        ));
        let backend = backend.lock().unwrap();
        assert!(backend.records.is_empty());
        assert_eq!(backend.state.unresolved_records, MAX_RECORDS);
    }

    #[test]
    fn retry_reservation_and_temp_file_are_identity_bounded() {
        let mut runtime = RuntimeState {
            state: SpoolState::default(),
            resolved_state: SpoolState::default(),
            reservations: BTreeMap::new(),
        };
        let (first, retrying) =
            reserve_runtime_identity(&mut runtime, "identity", b"first-ten!").unwrap();
        assert!(!retrying);
        let (retry, retrying) =
            reserve_runtime_identity(&mut runtime, "identity", b"later envelope").unwrap();
        assert!(retrying);
        assert_eq!(&*retry, &*first, "retry reuses the first exact envelope");
        assert_eq!(runtime.state.unresolved_records, 1);
        assert_eq!(runtime.state.unresolved_bytes, 10);

        let dir = temp_spool("bounded-temp-retry");
        std::fs::create_dir_all(&dir).unwrap();
        let (key, payload) = seal();
        let identity = seal_identity_hash("test", &key, &payload);
        let first_record = QuarantineRecord {
            record_version: QUARANTINE_RECORD_VERSION,
            ns: "test".to_owned(),
            key,
            serial: 1,
            payload_hash: identity.clone(),
            payload,
            store_error: "first diagnostic".to_owned(),
            error_transient: false,
            isolation_attempts: 5,
            quarantined_unix: 1,
            server_version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        let first_bytes = encode_record(&first_record).unwrap();
        let mut later_record = first_record;
        later_record.store_error = "later diagnostic".to_owned();
        later_record.quarantined_unix = 2;
        let later_bytes = encode_record(&later_record).unwrap();
        let final_path = dir.join(format!("{identity}.json"));
        std::fs::write(&final_path, b"conflict").unwrap();
        assert!(write_record_atomic(&dir, &final_path, &identity, &first_bytes, false).is_err());
        let temp_path = record_temp_path(&dir, &identity);
        assert_eq!(std::fs::read(&temp_path).unwrap(), first_bytes);
        assert!(write_record_atomic(&dir, &final_path, &identity, &later_bytes, true).is_err());
        assert_eq!(
            std::fs::read(&temp_path).unwrap(),
            first_bytes,
            "a valid same-seal temp is reused regardless of changing diagnostics"
        );
        assert_eq!(
            std::fs::read_dir(&dir)
                .unwrap()
                .map(Result::unwrap)
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "tmp"))
                .count(),
            1,
            "retries reuse one identity-stable temp artifact"
        );

        std::fs::write(&temp_path, b"partial").unwrap();
        assert!(write_record_atomic(&dir, &final_path, &identity, &later_bytes, true).is_err());
        assert_eq!(
            std::fs::read(&temp_path).unwrap(),
            b"partial",
            "corrupt or partial custody is preserved for fail-closed recovery"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn live_retry_publishes_through_secondary_without_removing_partial_primary() {
        let dir = temp_spool("secondary-temp-recovery");
        std::fs::create_dir_all(&dir).unwrap();
        let (key, payload) = seal();
        let identity = seal_identity_hash("test", &key, &payload);
        let record = QuarantineRecord {
            record_version: QUARANTINE_RECORD_VERSION,
            ns: "test".to_owned(),
            key,
            serial: 1,
            payload_hash: identity.clone(),
            payload,
            store_error: "diagnostic".to_owned(),
            error_transient: false,
            isolation_attempts: 5,
            quarantined_unix: 1,
            server_version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        let bytes = encode_record(&record).unwrap();
        let primary = record_temp_path(&dir, &identity);
        std::fs::write(&primary, &bytes[..bytes.len() / 2]).unwrap();
        let final_path = dir.join(format!("{identity}.json"));

        write_record_atomic(&dir, &final_path, &identity, &bytes, true).unwrap();
        assert_eq!(std::fs::read(final_path).unwrap(), bytes);
        assert!(!primary.exists());
        assert!(!record_recovery_temp_path(&dir, &identity).exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn newly_created_spool_and_records_are_private() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = temp_spool("private-modes");
        let spool = QuarantineSpool::open(&dir).unwrap();
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let (key, payload) = seal();
        spool
            .quarantine(
                "test",
                &key,
                &payload,
                1,
                &StoreError::Invalid("poison".to_owned()),
                5,
            )
            .unwrap();
        let record = std::fs::read_dir(&dir)
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .unwrap()
            .path();
        assert_eq!(
            std::fs::metadata(record).unwrap().permissions().mode() & 0o777,
            0o600
        );
        drop(spool);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn existing_spool_permissions_are_narrowed_before_scan() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = temp_spool("narrow-existing-mode");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let spool = QuarantineSpool::open(&dir).unwrap();
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        drop(spool);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn identity_is_domain_separated_and_length_framed() {
        let payload = serde_json::json!({"b": 2, "a": 1});
        assert_ne!(
            seal_identity_hash("a\0b", "c", &payload),
            seal_identity_hash("a", "b\0c", &payload),
            "length framing prevents delimiter ambiguity"
        );
        assert_eq!(
            seal_identity_hash("ns", "key", &payload),
            seal_identity_hash("ns", "key", &serde_json::json!({"a": 1, "b": 2})),
            "canonical object-key order is stable"
        );
    }

    #[cfg(unix)]
    #[test]
    fn open_repairs_owned_readable_but_non_writable_directory() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = temp_spool("read-only");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o550)).unwrap();
        let spool = QuarantineSpool::open(&dir)
            .expect("an owned spool can be made private and writable before the capability probe");
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700,
            "opening an owned legacy spool repairs it to the required private mode"
        );
        drop(spool);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

//! The write-through persistence pipeline (M2, PLAN "Persistence").
//!
//! Layout:
//!
//! - [`load_or_seed_world`] — boot: hydrate the [`World`] from the store
//!   ([`mush_store::ObjectStore::load_world`] →
//!   [`World::from_objects`]); a completely empty store is seeded with the
//!   minimal world (Limbo `#0` + God `#1`) **and the seed batch is written
//!   through immediately**, so even the very first boot is durable.
//! - [`batch_from_dirty`] — after every engine step that dirtied the world,
//!   the world task snapshots the dirty objects into one [`WriteBatch`]
//!   (full-object upserts, tombstones as deletes, `next_id` last).
//!   `mush-core` never learns the store exists: the seam is exactly
//!   `World::take_dirty()` + `Obj: Clone`.
//! - [`spawn_store_writer`] — the dedicated writer task. One bounded queue
//!   of [`StoreJob`]s, applied **in order**, each awaited to its ack before
//!   the next (write-through per command; no timer batching). On a
//!   *transient* write failure (connection, timeout — see
//!   [`StoreError::is_transient`]) it retries the same batch with
//!   exponential backoff forever — order is preserved, nothing is dropped —
//!   and tells the world task to pause mutations
//!   ([`WorldMsg::StoreStatus`]) until the store recovers (PLAN risk 3). A
//!   *deterministic* failure (serialization, a type-clobbered key) would
//!   fail identically forever, so the batch is dropped with a loud error
//!   instead of wedging the writer behind it. **Self-healing is partial**:
//!   object upserts are full snapshots, so RAM heals the store on the next
//!   write touching those objects — but a dropped batch's *module ops* do
//!   not heal (a dropped `Del` resurrects its doc at next boot; a dropped
//!   `StreamAppend` is lost), so the affected `mod:*` keys are logged for
//!   manual repair.
//!
//! Durability-critical batches (`durable: true`, e.g. Player creation)
//! carry a receipt oneshot; the caller gates its confirmation output on it,
//! and a dropped batch reports `Err` — never a false success. `@dump` rides
//! the same queue as [`StoreJob::Bgsave`] so the snapshot is ordered behind
//! every batch queued before it.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use mush_core::{ObjId, World};
use mush_store::{ObjectStore, StoreError, WriteBatch};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::unix_now_s;
use crate::world_task::WorldMsg;

/// Capacity of the writer's job queue. The world task `await`s its sends,
/// so a full queue back-pressures the single producer instead of dropping
/// batches (and read-only mode keeps it from filling during a detected
/// outage).
///
/// Deliberately small: every queued batch is a command whose output the
/// player has already seen but whose write has not yet been acked, so this
/// bounds the worst-case `kill -9` loss window while the store is merely
/// *slow* (a stalled BGSAVE, a saturated disk). In the steady state the
/// queue is empty and the loss window is exactly the in-flight command.
const WRITER_QUEUE: usize = 64;

/// First retry delay after a failed write; doubles up to [`RETRY_MAX`].
const RETRY_INITIAL: Duration = Duration::from_millis(100);

/// Retry backoff ceiling.
const RETRY_MAX: Duration = Duration::from_secs(5);

/// One unit of work for the store-writer task.
pub enum StoreJob {
    /// Apply one batch (acked in order; retried forever on failure).
    Write {
        /// The batch.
        batch: WriteBatch,
        /// Receipt for ack-gated flows (durable batches); `None` for
        /// fire-and-forget ordinary command batches.
        notify: Option<oneshot::Sender<Result<u64, String>>>,
    },
    /// Trigger a background snapshot (`@dump`), ordered behind everything
    /// queued before it.
    Bgsave {
        /// Resolved when the store acknowledged the BGSAVE request.
        done: oneshot::Sender<Result<(), String>>,
    },
}

/// Spawn the store-writer task. `world` (weak, so the writer never keeps
/// the world channel alive) receives [`WorldMsg::StoreStatus`] transitions
/// for the pause-mutations policy. The task drains its queue and exits when
/// every sender is dropped.
pub fn spawn_store_writer(
    store: Arc<dyn ObjectStore>,
    world: mpsc::WeakSender<WorldMsg>,
) -> (mpsc::Sender<StoreJob>, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(WRITER_QUEUE);
    let join = tokio::spawn(writer_loop(store, rx, world));
    (tx, join)
}

async fn writer_loop(
    store: Arc<dyn ObjectStore>,
    mut rx: mpsc::Receiver<StoreJob>,
    world: mpsc::WeakSender<WorldMsg>,
) {
    tracing::info!("store writer running");
    let mut healthy = true;
    while let Some(job) = rx.recv().await {
        match job {
            StoreJob::Write { batch, notify } => {
                let result = write_with_retry(&*store, batch, &mut healthy, &world).await;
                if let Some(notify) = notify {
                    // Forward the truth — a dropped batch must never ack as
                    // success (the ack gate for creations rides on this).
                    // A dropped receiver just means the requester gave up.
                    let _ = notify.send(result);
                }
            }
            StoreJob::Bgsave { done } => {
                let result = store.bgsave().await.map_err(|e| e.to_string());
                if let Err(e) = &result {
                    tracing::error!(error = %e, "bgsave failed");
                }
                let _ = done.send(result);
            }
        }
    }
    tracing::info!("store writer drained and exiting");
}

/// Apply one batch, retrying transient failures with backoff until the
/// store acks it. Health transitions (ok→failing, failing→ok) are reported
/// to the world task so it can enter/leave read-only mode.
///
/// Returns `Ok(serial)` only when the store actually acknowledged the
/// batch; `Err` when the batch was **dropped** — the store is closed
/// (abnormal teardown) or the failure is deterministic
/// ([`StoreError::is_transient`] is false) and retrying it forever would
/// wedge every batch queued behind it. Dropped batches are loudly logged;
/// RAM remains ahead of the store until the affected objects are next
/// written (full-object upserts self-heal).
async fn write_with_retry(
    store: &dyn ObjectStore,
    batch: WriteBatch,
    healthy: &mut bool,
    world: &mpsc::WeakSender<WorldMsg>,
) -> Result<u64, String> {
    let mut delay = RETRY_INITIAL;
    loop {
        match store.write(batch.clone()).await {
            Ok(receipt) => {
                if !*healthy {
                    *healthy = true;
                    tracing::info!(serial = receipt.serial, "store writes recovered");
                    notify_status(world, true, String::new()).await;
                }
                return Ok(receipt.serial);
            }
            Err(StoreError::Closed) => {
                // Shutdown already released the store; nothing to retry
                // against. (The orderly path drains this writer first, so
                // this only happens on abnormal teardown.)
                tracing::error!(serial = batch.serial, "store closed; batch dropped");
                return Err("store closed; batch dropped".to_string());
            }
            Err(e) if !e.is_transient() => {
                // Deterministic failure: every retry would fail identically,
                // wedging persistence behind one poisoned batch. Drop it,
                // scream, and keep the pipeline moving. Object upserts
                // self-heal (full snapshots re-sent on the next touch), but
                // dropped MODULE ops do NOT: a dropped `Del` resurrects its
                // document at next boot and a dropped `StreamAppend` is
                // simply gone — log the affected keys so an operator can
                // repair by hand.
                if !batch.module_ops.is_empty() {
                    let keys: Vec<String> = batch
                        .module_ops
                        .iter()
                        .map(|op| {
                            let key = match &op.op {
                                mush_store::ModuleWrite::JsonSet { key, .. }
                                | mush_store::ModuleWrite::Del { key }
                                | mush_store::ModuleWrite::StreamAppend { key, .. } => key,
                            };
                            format!("mod:{}:{}", op.ns, key)
                        })
                        .collect();
                    tracing::error!(
                        serial = batch.serial,
                        ?keys,
                        "dropped batch carried module ops that will NOT self-heal \
                         (deletes may resurrect at next boot; stream appends are lost)"
                    );
                }
                tracing::error!(
                    serial = batch.serial, error = %e,
                    "store write failed with a non-retryable error; BATCH DROPPED \
                     (object docs are behind RAM until those objects are next written)"
                );
                return Err(format!("non-retryable store error: {e}"));
            }
            Err(e) => {
                tracing::error!(serial = batch.serial, error = %e, "store write failed; retrying");
                if *healthy {
                    *healthy = false;
                    notify_status(world, false, e.to_string()).await;
                }
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(RETRY_MAX);
            }
        }
    }
}

async fn notify_status(world: &mpsc::WeakSender<WorldMsg>, healthy: bool, detail: String) {
    if let Some(tx) = world.upgrade()
        && tx
            .send(WorldMsg::StoreStatus { healthy, detail })
            .await
            .is_err()
    {
        tracing::debug!("world task gone; store status transition dropped");
    }
}

/// Snapshot one drained dirty set into a [`WriteBatch`]: live objects are
/// cloned as full-object upserts, tombstoned (or never-live) ids become
/// deletes, and `next_id` rides along so the id allocator resumes exactly.
#[must_use]
pub fn batch_from_dirty(
    world: &World,
    dirty: BTreeSet<ObjId>,
    serial: u64,
    durable: bool,
) -> WriteBatch {
    let mut batch = WriteBatch {
        serial,
        durable,
        ..WriteBatch::default()
    };
    for id in dirty {
        match world.get(id) {
            Some(obj) => batch.upserts.push(obj.clone()),
            None => batch.deletes.push(id),
        }
    }
    if !batch.is_empty() {
        batch.next_id = Some(world.next_id());
    }
    batch
}

/// The largest tolerated gap between `meta:next_id` and the number of
/// stored objects. Tombstones make `next_id > object count` normal, but a
/// corrupt `next_id` (say 4e9) would make `World::from_objects` allocate a
/// slot per id before any other validation ran — hundreds of GB. A million
/// tombstones beyond the live set is already absurd for a MUSH; past that,
/// the key is corrupt and boot must refuse before allocating.
const MAX_TOMBSTONE_GAP: usize = 1_000_000;

/// Boot-time world hydration. A populated store loads via
/// [`World::from_objects`]; a completely empty one is seeded with the
/// minimal world and the seed batch is **written through (and acked)**
/// before this returns, so the first boot persists without waiting for the
/// first command. Either way the returned world carries an empty dirty set
/// and the current wall clock.
///
/// Fails fast (naming any corrupt docs the load skipped, so the operator
/// can connect cause to effect) when:
/// - `meta:next_id` is implausibly far beyond the stored object count
///   ([`MAX_TOMBSTONE_GAP`]) — checked **before** hydration allocates;
/// - hydration itself rejects the data (dangling references, duplicate
///   ids, name collisions);
/// - the hydrated world is missing Limbo `#0` or God `#1` — without them
///   the server would boot an unusable husk (every create fails, every
///   login targets a missing start room). This happens when the seed docs
///   themselves were corrupt/skipped but `meta:next_id` survived.
pub async fn load_or_seed_world(store: &dyn ObjectStore) -> Result<World, String> {
    let world = match store
        .load_world()
        .await
        .map_err(|e| format!("loading world from store: {e}"))?
    {
        Some(snapshot) => {
            let count = snapshot.objects.len();
            let corrupt_note = if snapshot.corrupt_docs > 0 {
                format!(
                    " ({} corrupt object doc(s) were skipped during the load — \
                     likely the cause; see the log)",
                    snapshot.corrupt_docs
                )
            } else {
                String::new()
            };
            if (snapshot.next_id as usize).saturating_sub(count) > MAX_TOMBSTONE_GAP {
                return Err(format!(
                    "persisted next_id {} is implausibly far beyond the {count} stored \
                     object(s); refusing to allocate — meta:next_id looks corrupt{corrupt_note}",
                    snapshot.next_id
                ));
            }
            let world = World::from_objects(snapshot.objects, snapshot.next_id)
                .map_err(|e| format!("persisted world is inconsistent: {e}{corrupt_note}"))?;
            for (id, name) in [(mush_core::LIMBO, "Limbo"), (mush_core::GOD, "God")] {
                if world.get(id).is_none() {
                    return Err(format!(
                        "persisted world is missing {name} ({id}); refusing to boot an \
                         unusable world{corrupt_note}"
                    ));
                }
            }
            tracing::info!(
                objects = count,
                next_id = snapshot.next_id,
                "world loaded from store"
            );
            world
        }
        None => {
            let mut world = World::new_minimal();
            let dirty = world.take_dirty();
            let seed = batch_from_dirty(&world, dirty, 0, true);
            store
                .write(seed)
                .await
                .map_err(|e| format!("writing world seed batch: {e}"))?;
            tracing::info!("empty store: seeded minimal world (Limbo #0, God #1)");
            world
        }
    };
    let mut world = world;
    world.set_now(unix_now_s());
    Ok(world)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{GOD, LIMBO, ObjKind};
    use mush_store::InMemoryStore;

    #[test]
    fn batch_snapshots_upserts_deletes_and_next_id() {
        let mut w = World::new_minimal();
        let _ = w.take_dirty();
        let ball = w.create(ObjKind::Thing, "ball", GOD, Some(LIMBO)).unwrap();
        let doomed = w
            .create(ObjKind::Thing, "doomed", GOD, Some(LIMBO))
            .unwrap();
        w.destroy(doomed).unwrap();
        w.sweep();

        let dirty = w.take_dirty();
        let batch = batch_from_dirty(&w, dirty, 7, false);
        assert_eq!(batch.serial, 7);
        assert!(!batch.durable);
        let upserted: Vec<ObjId> = batch.upserts.iter().map(|o| o.id).collect();
        assert!(upserted.contains(&ball) && upserted.contains(&LIMBO));
        assert_eq!(batch.deletes, vec![doomed]);
        assert_eq!(batch.next_id, Some(w.next_id()));

        // Nothing dirty -> an empty batch with no next_id.
        let dirty = w.take_dirty();
        let empty = batch_from_dirty(&w, dirty, 8, false);
        assert!(empty.is_empty());
        assert_eq!(empty.next_id, None);
    }

    #[tokio::test]
    async fn boot_refuses_world_missing_limbo_or_god() {
        // A store that has data (so it is not seeded fresh) but whose seed
        // docs are gone — e.g. every doc corrupt/skipped except one thing —
        // must refuse to boot rather than serve an unusable husk.
        let store = InMemoryStore::new();
        let mut orphan = mush_core::Obj::new(ObjId(5), ObjKind::Thing, "orphan", ObjId(5), 0);
        orphan.location = None;
        store
            .write(mush_store::WriteBatch {
                serial: 1,
                upserts: vec![orphan],
                next_id: Some(6),
                durable: true,
                ..mush_store::WriteBatch::default()
            })
            .await
            .unwrap();
        let err = load_or_seed_world(&store).await.unwrap_err();
        assert!(
            err.contains("Limbo") || err.contains("God"),
            "error must name the missing anchor object: {err}"
        );
    }

    #[tokio::test]
    async fn boot_refuses_implausible_next_id_before_allocating() {
        // A corrupt meta:next_id must be refused BEFORE hydration tries to
        // allocate a slot per id (a 4e9 next_id would be hundreds of GB).
        let store = InMemoryStore::new();
        let world = World::new_minimal();
        let objects: Vec<_> = world.iter().cloned().collect();
        store
            .write(mush_store::WriteBatch {
                serial: 1,
                upserts: objects,
                next_id: Some(50_000_000),
                durable: true,
                ..mush_store::WriteBatch::default()
            })
            .await
            .unwrap();
        let err = load_or_seed_world(&store).await.unwrap_err();
        assert!(
            err.contains("next_id") && err.contains("corrupt"),
            "error must blame meta:next_id: {err}"
        );
    }

    #[tokio::test]
    async fn seed_writes_through_and_reloads() {
        let store = InMemoryStore::new();
        let seeded = load_or_seed_world(&store).await.unwrap();
        assert!(seeded.get(GOD).is_some() && seeded.get(LIMBO).is_some());

        // The seed batch really landed: a second boot loads it back.
        let reloaded = load_or_seed_world(&store).await.unwrap();
        assert_eq!(reloaded.next_id(), seeded.next_id());
        assert_eq!(reloaded.get(GOD).unwrap().name, "God");
        assert_eq!(reloaded.get(LIMBO).unwrap().name, "Limbo");
    }
}

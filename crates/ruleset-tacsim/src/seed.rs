//! **The seed latch** (B6; §3.1 invariant (ii); the S lens).
//!
//! # Can any interleaving or crash duplicate a seeded object or lose the latch?
//!
//! No, and the argument has exactly two legs.
//!
//! **One batch.** [`apply`] creates its objects through `World::create` — which marks
//! them dirty — and writes the latch through `Ctx::persist`, which queues a
//! `ModuleOp`. The world task's flush folds `take_dirty()` and `take_module_ops()`
//! into **one** `WriteBatch` before it is handed to the store, so a reboot sees the
//! objects **and** the latch or neither of them. There is no window in which the mine
//! exists and the latch does not, which is the only window a duplicate could come
//! through.
//!
//! **One writer.** The remaining race is two `@tacsim/seed … commit` invocations, and
//! the world is a single writer: they serialize, the first applies, the second reads
//! the latch and is a **HIT** that writes nothing. "Two processes, one store" — 10b
//! §3.1's phrasing — is not a reachable shape on this tree, and 10b §3.1 is corrected
//! to say so rather than testing a race that cannot happen.
//!
//! **And the latch survives its own corruption.** `is_applied` is derived from the
//! stored *keys*, not their bodies (`store`'s table), so a `tacsim:seed:` document
//! that will not parse still bars a re-seed. Reading "cannot parse" as "not applied"
//! is the one fail-open direction here, and it is the one that would duplicate a mine.
//!
//! # Refusals happen before the first object exists
//!
//! [`plan`] does every check — enabled, the pack is loaded, it is a *seed* pack, no
//! other version of it has been applied, and the object count is inside
//! `MAX_SEED_OBJECTS` — and returns a plan. [`apply`] pre-flights the object owner
//! once and then performs only infallible creates, so there is no `?` after the first
//! irreversible mutation.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, Ctx, ObjId, ObjKind};

use crate::pack::{MAX_SEED_OBJECTS, PackEntry, PackKind};
use crate::record::SeedApplication;
use crate::store::{TacsimStore, persist_meta, put_seed};

/// One object a seed pack would create.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedObject {
    /// The pack-local id.
    pub id: CanonicalId,
    /// The name the object gets.
    pub label: String,
    /// What kind of place it is.
    pub role: CanonicalId,
}

/// What a `@tacsim/seed … dry` would do, and what `commit` will do.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SeedPlan {
    /// The pack.
    pub pack: CanonicalId,
    /// The exact version.
    pub version: u32,
    /// Every object it would create, in pack-local id order.
    pub objects: Vec<PlannedObject>,
}

/// Why seeding refuses.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SeedError {
    /// The layer is off.
    #[error("TacSim is not enabled on this server.")]
    NotEnabled,
    /// No loaded pack by that name — **or one the viewer may not see**.
    #[error("No such pack.")]
    NoSuchPack,
    /// A rules pack cannot be seeded; it is re-pinned and diffed instead (Q6A).
    #[error("{pack} is a rules pack; only a campaign-seed pack is applied to the world.")]
    NotASeedPack {
        /// The pack asked for.
        pack: String,
    },
    /// This exact `(pack, version)` has already been applied.
    #[error("{pack}@{version} has already been applied to this world; nothing was written.")]
    AlreadyApplied {
        /// The pack.
        pack: String,
        /// The version.
        version: u32,
    },
    /// A **different** version of this pack has been applied.
    ///
    /// Refused **naming the applied version**, because the useful thing to know is
    /// not "no" but "you already have v1" (§3.1's review item).
    #[error(
        "{pack}@{applied} has already been applied to this world; {pack}@{wanted} would seed it a \
         second time. Re-seeding is a migration, not a re-run."
    )]
    VersionAlreadyApplied {
        /// The pack.
        pack: String,
        /// The version this world actually carries.
        applied: u32,
        /// The version asked for.
        wanted: u32,
    },
    /// Past `MAX_SEED_OBJECTS`, refused **before the first object is created**.
    #[error("{pack}@{version} would create {actual} objects; the maximum is {max}.")]
    TooManyObjects {
        /// The pack.
        pack: String,
        /// The version.
        version: u32,
        /// How many it declared.
        actual: usize,
        /// `MAX_SEED_OBJECTS`.
        max: usize,
    },
    /// The world would not accept the objects' owner.
    #[error("the seed's owner object does not exist; nothing was written.")]
    NoOwner,
    /// The latch would not build.
    #[error("the seed latch could not be built: {detail}")]
    Latch {
        /// What the record's constructor said.
        detail: String,
    },
}

/// **Plan a seed. Writes nothing, ever.**
///
/// # Errors
///
/// Every variant of [`SeedError`] except [`SeedError::NoOwner`] and
/// [`SeedError::Latch`], which only [`apply`] can reach.
pub fn plan(store: &TacsimStore, pack: &CanonicalId) -> Result<SeedPlan, SeedError> {
    if !store.enabled() {
        return Err(SeedError::NotEnabled);
    }
    let Some(version) = store.newest_loaded(pack) else {
        return Err(SeedError::NoSuchPack);
    };
    let Some(loaded) = store.pack(pack, version) else {
        return Err(SeedError::NoSuchPack);
    };
    if loaded.pack.manifest().kind() != PackKind::CampaignSeed {
        return Err(SeedError::NotASeedPack {
            pack: pack.to_string(),
        });
    }
    if store.is_applied(pack, version) {
        return Err(SeedError::AlreadyApplied {
            pack: pack.to_string(),
            version,
        });
    }
    // A *different* version of the same pack: the re-derived `pack -> applied
    // version` index answers this, and the message names the applied one.
    if let Some(applied) = store.applied_version(pack) {
        return Err(SeedError::VersionAlreadyApplied {
            pack: pack.to_string(),
            applied,
            wanted: version,
        });
    }
    let mut objects: Vec<PlannedObject> = Vec::new();
    for (id, entry) in loaded.pack.entries() {
        if let PackEntry::Location(def) = entry {
            objects.push(PlannedObject {
                id: id.clone(),
                label: def.label.clone(),
                role: def.role.clone(),
            });
        }
    }
    if objects.len() > MAX_SEED_OBJECTS {
        return Err(SeedError::TooManyObjects {
            pack: pack.to_string(),
            version,
            actual: objects.len(),
            max: MAX_SEED_OBJECTS,
        });
    }
    Ok(SeedPlan {
        pack: pack.clone(),
        version,
        objects,
    })
}

/// **Apply a plan.** The objects and the latch ride one `WriteBatch`.
///
/// `owner` is pre-flighted once, before anything is created; after that every
/// `World::create` here is infallible for these inputs (a room with no location and
/// an existing owner), so no `?` remains after the first irreversible mutation. The
/// one impossible failure is handled rather than propagated: whatever was actually
/// created is latched — a partial seed recorded honestly and **never re-run** is the
/// fail-closed direction, since the alternative duplicates a mine — and the refusal
/// pages with the exact command.
///
/// # Errors
///
/// [`SeedError::NoOwner`] before anything is written, and [`SeedError::Latch`] if the
/// record's own constructor refuses.
pub fn apply(
    ctx: &mut Ctx<'_>,
    plan: &SeedPlan,
    owner: ObjId,
) -> Result<SeedApplication, SeedError> {
    // ---- everything fallible, before the first mutation ----
    if ctx.w.get(owner).is_none() {
        return Err(SeedError::NoOwner);
    }
    let Some(digest) = ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(|s| s.pack(&plan.pack, plan.version))
        .map(|loaded| loaded.pack.manifest().digest())
    else {
        return Err(SeedError::NoSuchPack);
    };
    let applied_at = ctx.world_ic_now();

    // ---- the irreversible half ----
    let mut placed: BTreeMap<CanonicalId, ObjId> = BTreeMap::new();
    let mut failed: Option<String> = None;
    for object in &plan.objects {
        match ctx.w.create(ObjKind::Room, &object.label, owner, None) {
            Ok(id) => {
                placed.insert(object.id.clone(), id);
            }
            Err(error) => {
                // Unreachable for a room with an existing owner and no location.
                // Stop creating, latch what exists, and page — never propagate.
                failed = Some(error.to_string());
                break;
            }
        }
    }
    let record =
        match SeedApplication::new(plan.pack.clone(), plan.version, digest, applied_at, placed) {
            Ok(record) => record,
            Err(error) => {
                return Err(SeedError::Latch {
                    detail: error.to_string(),
                });
            }
        };
    put_seed(ctx, record.clone());
    persist_meta(ctx);
    if let Some(detail) = failed {
        // 08c §4.6's ladder, level 2: the exact command, the failed invariant, and
        // **the subject only** — nothing else about the world is stopped.
        let runbook = format!(
            "tacsim: a seed stopped part way; the latch records exactly what exists and the pack \
             will never be applied again. Recommended: `@tacsim/pack show {}@{}`, then create the \
             remainder by hand or publish a corrected version. Only this pack is stopped.",
            plan.pack, plan.version
        );
        tracing::error!(
            pack = %plan.pack,
            version = plan.version,
            created = record.objects(),
            planned = plan.objects.len(),
            %detail,
            "{runbook}"
        );
    }
    Ok(record)
}

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
//! `MAX_SEED_OBJECTS` — and returns a plan. [`apply`] **re-runs every one of them**
//! (independent verify P9: the guard belongs with the irreversible half, not with its
//! caller), pre-flights the object owner, and only then performs infallible creates, so
//! there is no `?` after the first irreversible mutation.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, Ctx, ObjId, ObjKind, World};

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
    /// S9.1: the version the content latch holds now, when this is an update.
    pub update_from: Option<u32>,
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
    /// S9.1: the content latch refused the pack — the appliers' reason.
    #[error("the content latch refused {pack}@{version}: {detail}")]
    Content {
        /// The pack.
        pack: String,
        /// The version.
        version: u32,
        /// What the apply said.
        detail: String,
    },
    /// S9.1: a world seeded before S9 holds the pack under the legacy latch; a
    /// re-seed through the content latch would make its places twice.
    #[error(
        "{pack}@{applied} was seeded under the legacy latch; the content latch cannot update it to \
         @{wanted}. Re-seeding a legacy world is a migration, by hand."
    )]
    LegacySeed {
        /// The pack.
        pack: String,
        /// The version the legacy latch carries.
        applied: u32,
        /// The version asked for.
        wanted: u32,
    },
}

/// S9.1: what one seed did, through the content latch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeedOutcome {
    /// The pack.
    pub pack: CanonicalId,
    /// The version applied.
    pub version: u32,
    /// Objects created.
    pub created: usize,
    /// Entries updated in place.
    pub updated: usize,
    /// Entries retired.
    pub retired: usize,
    /// The apply's anomaly, when it stopped part way.
    pub anomaly: Option<String>,
    /// Whether the content latch applied it (S9.1), or the legacy seed latch did
    /// (a world composed without the content module).
    pub through_content: bool,
}

/// **Plan a seed. Writes nothing, ever.**
///
/// # Errors
///
/// Every variant of [`SeedError`] except [`SeedError::NoOwner`] and
/// [`SeedError::Latch`], which only [`apply`] can reach.
pub fn plan(
    store: &TacsimStore,
    w: &World,
    pack: &CanonicalId,
    wanted: Option<u32>,
) -> Result<SeedPlan, SeedError> {
    if !store.enabled() {
        return Err(SeedError::NotEnabled);
    }
    // S9.1: a version may be named (`<pack>@<version>`); the newest loaded
    // otherwise, as before.
    let version = match wanted {
        Some(v) if store.versions_of(pack).any(|loaded| loaded == v) => v,
        Some(_) => return Err(SeedError::NoSuchPack),
        None => match store.newest_loaded(pack) {
            Some(v) => v,
            None => return Err(SeedError::NoSuchPack),
        },
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
    // S9.1: with the content module composed, the content latch is the latch —
    // one row per pack id, a newer version an update in place — and the legacy
    // `tacsim:seed:` latch (a world seeded before S9) answers a HIT for its own
    // version above and refuses any other, since re-seeding a legacy world
    // through the content latch would make its places twice. Without the content
    // module (a composition that has none), the legacy latch keeps its rule: one
    // version, ever.
    let content_composed = content_composed(w);
    if let Some(applied) = store.applied_version(pack) {
        return Err(if content_composed {
            SeedError::LegacySeed {
                pack: pack.to_string(),
                applied,
                wanted: version,
            }
        } else {
            SeedError::VersionAlreadyApplied {
                pack: pack.to_string(),
                applied,
                wanted: version,
            }
        });
    }
    let mut update_from = None;
    if let Some(applied) = store.applied_in(w, pack) {
        if applied == version {
            return Err(SeedError::AlreadyApplied {
                pack: pack.to_string(),
                version,
            });
        }
        if applied > version {
            return Err(SeedError::VersionAlreadyApplied {
                pack: pack.to_string(),
                applied,
                wanted: version,
            });
        }
        update_from = Some(applied);
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
        update_from,
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
/// # The idempotency guard travels with the irreversible half
///
/// Independent verify P9: `apply` is `pub` and takes a plain `Clone`/`PartialEq`
/// `SeedPlan` with public fields, and its only pre-flights were the owner and the
/// digest — `is_applied`, `applied_version` and the pack-kind test all lived in
/// [`plan`], i.e. in the *caller*. A second `apply` over one live plan would therefore
/// have run the create loop again and `put_seed` would have overwritten the latch with
/// the new `placed` map, dropping the first set's object ids while leaving those
/// objects in the world — the duplicated mine invariant (ii) exists to prevent. The
/// checks are re-run here, so the latch's guarantee is a property of the **function**
/// rather than of one call site; they stay entirely inside the fallible pre-flight, so
/// nothing fallible moved past the first `World::create`.
///
/// # Errors
///
/// [`SeedError::AlreadyApplied`], [`SeedError::VersionAlreadyApplied`],
/// [`SeedError::NotASeedPack`], [`SeedError::NoSuchPack`] and [`SeedError::NoOwner`]
/// before anything is written, and [`SeedError::Latch`] if the record's own
/// constructor refuses.
pub fn apply(ctx: &mut Ctx<'_>, plan: &SeedPlan, owner: ObjId) -> Result<SeedOutcome, SeedError> {
    if !content_composed(ctx.w) {
        return legacy_apply(ctx, plan, owner);
    }
    let (pack, requirements) = {
        let Some(store) = ctx.w.module_state::<TacsimStore>() else {
            return Err(SeedError::NotEnabled);
        };
        if !store.enabled() {
            return Err(SeedError::NotEnabled);
        }
        if store.is_applied(&plan.pack, plan.version) {
            return Err(SeedError::AlreadyApplied {
                pack: plan.pack.to_string(),
                version: plan.version,
            });
        }
        if let Some(applied) = store.applied_version(&plan.pack) {
            return Err(SeedError::LegacySeed {
                pack: plan.pack.to_string(),
                applied,
                wanted: plan.version,
            });
        }
        if let Some(applied) = store.applied_in(ctx.w, &plan.pack)
            && applied >= plan.version
        {
            return Err(if applied == plan.version {
                SeedError::AlreadyApplied {
                    pack: plan.pack.to_string(),
                    version: plan.version,
                }
            } else {
                SeedError::VersionAlreadyApplied {
                    pack: plan.pack.to_string(),
                    applied,
                    wanted: plan.version,
                }
            });
        }
        let Some(loaded) = store.pack(&plan.pack, plan.version) else {
            return Err(SeedError::NoSuchPack);
        };
        if loaded.pack.manifest().kind() != PackKind::CampaignSeed {
            return Err(SeedError::NotASeedPack {
                pack: plan.pack.to_string(),
            });
        }
        if plan.objects.len() > MAX_SEED_OBJECTS {
            return Err(SeedError::TooManyObjects {
                pack: plan.pack.to_string(),
                version: plan.version,
                actual: plan.objects.len(),
                max: MAX_SEED_OBJECTS,
            });
        }
        // The rules packs it requires, as the store loaded them: applied through
        // the latch first when the latch lacks them (their entries make no object).
        let requirements: Vec<mush_content::Pack> = loaded
            .pack
            .manifest()
            .requires()
            .iter()
            .filter(|req| store.applied_in(ctx.w, &req.id) != Some(req.version))
            .filter_map(|req| {
                store
                    .pack(&req.id, req.version)
                    .map(|l| l.pack.pack().clone())
            })
            .collect();
        (loaded.pack.pack().clone(), requirements)
    };
    if ctx.w.get(owner).is_none() {
        return Err(SeedError::NoOwner);
    }
    // S9.1: the content module applies the pack through its own latch and this
    // crate's appliers (a room and a site per place, a connection per route);
    // the rooms are owned by whoever seeds, as they were.
    let saved = ctx.actor;
    ctx.actor = owner;
    for required in &requirements {
        if let Err(error) = mush_content::module::apply_pack(ctx, required) {
            ctx.actor = saved;
            return Err(SeedError::Content {
                pack: required.manifest().id().to_string(),
                version: required.manifest().version(),
                detail: error.to_string(),
            });
        }
    }
    let applied = mush_content::module::apply_pack(ctx, &pack);
    ctx.actor = saved;
    match applied {
        Ok((applied, report)) => Ok(SeedOutcome {
            pack: plan.pack.clone(),
            version: applied.version(),
            created: report.created,
            updated: report.updated,
            retired: report.retired,
            anomaly: report.anomaly.clone(),
            through_content: true,
        }),
        Err(error) => Err(SeedError::Content {
            pack: plan.pack.to_string(),
            version: plan.version,
            detail: error.to_string(),
        }),
    }
}

/// Whether the content module is composed on this world (S9.1).
fn content_composed(w: &World) -> bool {
    w.module_state::<mush_content::ContentState>().is_some()
}

/// The S1–S7 seed, for a world composed without the content module: the rooms
/// and the `tacsim:seed:{pack}@{version}` latch ride one batch. Kept so a
/// composition that has no content module seeds as it did; a server with one
/// never comes here.
fn legacy_apply(
    ctx: &mut Ctx<'_>,
    plan: &SeedPlan,
    owner: ObjId,
) -> Result<SeedOutcome, SeedError> {
    let digest = {
        let Some(store) = ctx.w.module_state::<TacsimStore>() else {
            return Err(SeedError::NotEnabled);
        };
        if !store.enabled() {
            return Err(SeedError::NotEnabled);
        }
        if store.is_applied(&plan.pack, plan.version) {
            return Err(SeedError::AlreadyApplied {
                pack: plan.pack.to_string(),
                version: plan.version,
            });
        }
        if let Some(applied) = store.applied_version(&plan.pack) {
            return Err(SeedError::VersionAlreadyApplied {
                pack: plan.pack.to_string(),
                applied,
                wanted: plan.version,
            });
        }
        let Some(loaded) = store.pack(&plan.pack, plan.version) else {
            return Err(SeedError::NoSuchPack);
        };
        if loaded.pack.manifest().kind() != PackKind::CampaignSeed {
            return Err(SeedError::NotASeedPack {
                pack: plan.pack.to_string(),
            });
        }
        if plan.objects.len() > MAX_SEED_OBJECTS {
            return Err(SeedError::TooManyObjects {
                pack: plan.pack.to_string(),
                version: plan.version,
                actual: plan.objects.len(),
                max: MAX_SEED_OBJECTS,
            });
        }
        loaded.pack.manifest().digest()
    };
    if ctx.w.get(owner).is_none() {
        return Err(SeedError::NoOwner);
    }
    let applied_at = ctx.world_ic_now();
    let mut placed: BTreeMap<CanonicalId, ObjId> = BTreeMap::new();
    let mut failed: Option<String> = None;
    for object in &plan.objects {
        match ctx.w.create(ObjKind::Room, &object.label, owner, None) {
            Ok(id) => {
                placed.insert(object.id.clone(), id);
            }
            Err(error) => {
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
    if let Some(detail) = &failed {
        tracing::error!(
            pack = %plan.pack,
            version = plan.version,
            created = record.objects(),
            planned = plan.objects.len(),
            %detail,
            "tacsim: a seed stopped part way; the latch records exactly what exists and the pack \
             will never be applied again. Recommended: `@tacsim/pack show {}@{}`, then create the \
             remainder by hand or publish a corrected version. Only this pack is stopped.",
            plan.pack,
            plan.version
        );
    }
    Ok(SeedOutcome {
        pack: plan.pack.clone(),
        version: plan.version,
        created: usize::try_from(record.objects()).unwrap_or(usize::MAX),
        updated: 0,
        retired: 0,
        anomaly: failed,
        through_content: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- E7 cut 1 independent verify fold ----

    /// **P9 — the latch is a property of `apply`, not of `commands::seed_cmd`.**
    ///
    /// `apply` is `pub` API and a `SeedPlan` is a plain `Clone` value with public
    /// fields, so nothing stopped a caller applying one plan twice: the second call
    /// would have created a second object set and `put_seed` would have overwritten the
    /// latch with the new `placed` map, orphaning the first set's objects in the world
    /// — invariant (ii)'s duplicated mine, reached without any interleaving at all.
    /// This is the shape the integration tests cannot express, because a bare `Ctx`
    /// only exists inside a handler; here it is built by hand exactly as
    /// `Engine::modules_loaded` builds one.
    #[test]
    fn vf_p9_apply_re_checks_the_latch_and_a_second_call_creates_nothing() {
        let mut appliers = mush_content::Appliers::new();
        mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
        crate::register_appliers(&mut appliers).expect("tacsim appliers");
        let mut tw = mush_testkit::TestWorld::with_modules(vec![
            Box::new(crate::TacsimRuleset::with_config(crate::fixture::config())),
            Box::new(mush_sim::SimulationModule::new()),
            Box::new(mush_atlas::AtlasModule::new()),
            Box::new(mush_content::ContentModule::new().with_appliers(appliers)),
        ]);
        let corridor = CanonicalId::new("tacsim.corridor").expect("a canonical id");
        let engine = tw.engine_mut();
        let plan = {
            let store = engine
                .world
                .module_state::<TacsimStore>()
                .expect("the tacsim slot");
            plan(store, &engine.world, &corridor, None).expect("the corridor plans")
        };
        assert_eq!(plan.update_from, None);
        let mut out = mush_core::Outbox::new();
        let mut ctx = Ctx {
            w: &mut engine.world,
            actor: mush_core::GOD,
            switches: mush_core::SwitchSet::default(),
            args: mush_core::ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(smol_str::SmolStr::new(crate::store::NS)),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };

        let first = apply(&mut ctx, &plan, mush_core::GOD).expect("the first application");
        assert_eq!(first.version, 1);
        assert!(first.anomaly.is_none(), "{first:?}");
        let rooms_after_first = ctx
            .w
            .iter()
            .filter(|o| o.kind == mush_core::ObjKind::Room)
            .count();
        let placed = ctx
            .w
            .module_state::<TacsimStore>()
            .expect("the tacsim slot")
            .placed_in(ctx.w, &corridor, 1)
            .expect("the content latch placed the corridor");
        assert_eq!(placed.len(), 3, "three places: {placed:?}");

        // The SAME live plan, applied again: a HIT, nothing made twice.
        let refused = apply(&mut ctx, &plan, mush_core::GOD).expect_err("the second must refuse");
        assert!(
            matches!(refused, SeedError::AlreadyApplied { ref pack, version }
                     if pack == "tacsim.corridor" && version == 1),
            "{refused:?}"
        );
        let rooms_after_second = ctx
            .w
            .iter()
            .filter(|o| o.kind == mush_core::ObjKind::Room)
            .count();
        assert_eq!(
            rooms_after_first, rooms_after_second,
            "three objects, not six"
        );
        // And the legacy latch was not written: the content latch is the latch.
        assert!(
            ctx.w
                .module_state::<TacsimStore>()
                .expect("the tacsim slot")
                .seed(&corridor, 1)
                .is_none()
        );
    }
}

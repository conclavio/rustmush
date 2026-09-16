//! **Plan and apply** (plan §3.1; doc 14 §3.1): diff a pack against its latch, then
//! let each entry kind's owner make, change or remove the objects, and write the latch
//! in the same batch.
//!
//! The frame is this crate's; the semantics are the owners'. An [`EntryApplier`] is
//! registered per entry kind at composition (the way `mush-sim` is handed a route
//! catalog), declares a **phase** so that rooms exist before the exits that name them,
//! and is asked to create, update or retire one entry at a time with the map of every
//! object the pack has made so far.
//!
//! # The one invariant
//!
//! **The latch always describes the world.** An apply that stops part way — an owner
//! refused an entry, the world refused an object — is not rolled back, because objects
//! are not un-creatable safely inside one command; instead it is **latched honestly**:
//! every entry that was made or changed is recorded at the revision that was applied,
//! every entry not reached keeps its previous record, the latch carries an `anomaly`
//! naming what stopped it, and the caller is told. The next apply plans from that
//! latch and continues. This is `tacsim`'s seed rule ("stop creating, latch
//! what exists, and page — never propagate") kept for update and retire too.
//!
//! Retire is the exception that is checked first: every retire's reference check runs
//! before any mutation, so a pack that would orphan a reference is refused whole,
//! naming the reference.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, Ctx, DefinitionDigest, ModuleOp, ModuleWrite, ObjId};
use smol_str::SmolStr;

use crate::latch::{Applied, AppliedEntry, LatchError, Latches, NS, applied_key};
use crate::pack::{EntryKinds, Pack, PackError, RawEntry};

/// What an apply should do to one entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    /// Not in the latch: make it.
    Create,
    /// In the latch at another revision: change it.
    Update,
    /// In the latch, not in the pack: remove it.
    Retire,
    /// In the latch at this revision: nothing.
    Unchanged,
}

impl Step {
    /// The word `@atlas/pack plan` prints for this step.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Step::Create => "create",
            Step::Update => "update",
            Step::Retire => "retire",
            Step::Unchanged => "unchanged",
        }
    }
}

/// One planned entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedEntry {
    /// The pack-local id.
    pub id: CanonicalId,
    /// The kind (the pack's for create/update/unchanged, the latch's for retire).
    pub kind: CanonicalId,
    /// What to do.
    pub step: Step,
    /// The object the latch has for it, if any.
    pub existing: Option<ObjId>,
}

/// The diff of a pack against its latch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Plan {
    /// The pack.
    pub pack: CanonicalId,
    /// The version to apply.
    pub version: u32,
    /// The pack digest to apply.
    pub digest: DefinitionDigest,
    /// Every entry, in pack-id order, with its step.
    pub entries: Vec<PlannedEntry>,
}

impl Plan {
    /// The ids at one step, in order.
    pub fn at(&self, step: Step) -> impl Iterator<Item = &PlannedEntry> {
        self.entries.iter().filter(move |e| e.step == step)
    }

    /// How many entries are at one step.
    #[must_use]
    pub fn count(&self, step: Step) -> usize {
        self.at(step).count()
    }

    /// Whether the plan changes nothing.
    #[must_use]
    pub fn is_noop(&self) -> bool {
        self.entries.iter().all(|e| e.step == Step::Unchanged)
    }

    /// A short content-address of this plan — the pack's digest and every step —
    /// so an `apply` can demand the plan a staffer read is the plan that runs: a pack
    /// edited, or a latch moved, between the two changes the fingerprint.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        let steps: Vec<serde_json::Value> = self
            .entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id.as_str(),
                    "kind": e.kind.as_str(),
                    "step": e.step.label(),
                    "existing": e.existing.map(|o| o.0),
                })
            })
            .collect();
        let value = serde_json::json!({
            "pack": self.pack.as_str(),
            "version": self.version,
            "digest": self.digest.to_hex(),
            "steps": steps,
        });
        DefinitionDigest::for_canonical_value_with_limit(
            PLAN_FINGERPRINT_DOMAIN,
            &value,
            usize::MAX,
        )
        .map(|d| d.to_hex().chars().take(PLAN_FINGERPRINT_CHARS).collect())
        .unwrap_or_default()
    }
}

/// The digest domain of [`Plan::fingerprint`].
pub const PLAN_FINGERPRINT_DOMAIN: &str = "mush.content.plan.v1";

/// How many hex characters a plan fingerprint shows and `apply` compares.
pub const PLAN_FINGERPRINT_CHARS: usize = 16;

/// Diff `pack` against its latch.
///
/// # Errors
///
/// [`ApplyError::Downgrade`] when the latch holds a newer version, and
/// [`PackError`] for an entry that will not digest.
pub fn plan(pack: &Pack, previous: Option<&Applied>) -> Result<Plan, ApplyError> {
    let manifest = pack.manifest();
    let mut entries = Vec::with_capacity(pack.entries().len());
    if let Some(prev) = previous
        && !prev.is_retired()
        && prev.version() > manifest.version()
    {
        return Err(ApplyError::Downgrade {
            pack: manifest.id().to_string(),
            applied: prev.version(),
            wanted: manifest.version(),
        });
    }
    let live = previous.filter(|p| !p.is_retired());
    for (id, entry) in pack.entries() {
        let digest = entry.digest()?;
        let (step, existing) = match live.and_then(|p| p.entries().get(id)) {
            None => (Step::Create, None),
            Some(applied) if applied.digest() == digest && applied.kind() == entry.kind() => {
                (Step::Unchanged, applied.object())
            }
            Some(applied) => (Step::Update, applied.object()),
        };
        entries.push(PlannedEntry {
            id: id.clone(),
            kind: entry.kind().clone(),
            step,
            existing,
        });
    }
    if let Some(prev) = live {
        for (id, applied) in prev.entries() {
            if pack.entry(id).is_none() {
                entries.push(PlannedEntry {
                    id: id.clone(),
                    kind: applied.kind().clone(),
                    step: Step::Retire,
                    existing: applied.object(),
                });
            }
        }
    }
    Ok(Plan {
        pack: manifest.id().clone(),
        version: manifest.version(),
        digest: manifest.digest(),
        entries,
    })
}

/// What one entry kind's owner does with an entry.
///
/// Every method receives `placed`, the pack-local id → object map of everything this
/// pack has made so far (the latch's, updated as the apply proceeds), so an exit can
/// find the rooms it joins whether they were made in this apply or a previous one.
pub trait EntryApplier: Send + Sync {
    /// The kinds this applier owns.
    fn kinds(&self) -> Vec<CanonicalId>;

    /// The phase this applier's kinds run in: lower first on create/update, higher
    /// first on retire. Rooms before exits; sites before rooms.
    fn phase(&self) -> u8;

    /// Make the entry. `Ok(None)` is an entry that makes no object.
    ///
    /// # Errors
    ///
    /// Any refusal, with a reason the latch's anomaly and the caller both carry.
    fn create(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String>;

    /// Change the entry in place. The default retires and recreates; an owner with
    /// live state to preserve overrides it.
    ///
    /// # Errors
    ///
    /// Any refusal.
    fn update(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &Pack,
        id: &CanonicalId,
        entry: &RawEntry,
        existing: Option<ObjId>,
        placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<Option<ObjId>, String> {
        if let Some(object) = existing {
            self.retire_check(ctx, pack.manifest().id(), id, Some(object), placed)?;
            self.retire(ctx, pack.manifest().id(), id, Some(object))?;
        }
        self.create(ctx, pack, id, entry, placed)
    }

    /// Refuse a retire that would orphan a reference, **before** anything mutates.
    /// `placed` is everything the pack has made (its latch), so an owner can tell a
    /// reference from inside the pack — which retires with it — from one outside.
    ///
    /// # Errors
    ///
    /// The reference that would be orphaned, named.
    fn retire_check(
        &self,
        ctx: &Ctx<'_>,
        pack: &CanonicalId,
        id: &CanonicalId,
        existing: Option<ObjId>,
        placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<(), String>;

    /// Remove the entry `id` of `pack` (an entry id is unique only within its pack).
    ///
    /// # Errors
    ///
    /// Any refusal.
    fn retire(
        &self,
        ctx: &mut Ctx<'_>,
        pack: &CanonicalId,
        id: &CanonicalId,
        existing: Option<ObjId>,
    ) -> Result<(), String>;

    /// Called once per applier after **every** entry of `pack` was created or updated
    /// without an anomaly — the place for work that needs the whole pack in place (the
    /// atlas rasterizes a neighborhood's tiles here, once its rooms and features all
    /// exist). Not called when the apply stopped part way. Default: nothing.
    ///
    /// # Errors
    ///
    /// A refusal becomes the latch's anomaly, exactly like a refused entry.
    fn finish(
        &self,
        _ctx: &mut Ctx<'_>,
        _pack: &Pack,
        _placed: &BTreeMap<CanonicalId, ObjId>,
    ) -> Result<(), String> {
        Ok(())
    }

    /// Called once per applier after a whole pack was retired with nothing left —
    /// the place to drop derived records keyed by the pack. Default: nothing.
    ///
    /// # Errors
    ///
    /// A refusal becomes the latch's anomaly.
    fn finish_retire(&self, _ctx: &mut Ctx<'_>, _pack: &CanonicalId) -> Result<(), String> {
        Ok(())
    }
}

/// The appliers composed into this server, by kind.
#[derive(Clone, Default)]
pub struct Appliers {
    by_kind: BTreeMap<CanonicalId, std::sync::Arc<dyn EntryApplier>>,
    kinds: EntryKinds,
}

impl std::fmt::Debug for Appliers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Appliers")
            .field("kinds", &self.by_kind.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Appliers {
    /// Empty.
    #[must_use]
    pub fn new() -> Appliers {
        Appliers::default()
    }

    /// Register an applier for every kind it owns, under `owner`'s name.
    ///
    /// # Errors
    ///
    /// [`PackError::KindOwnedElsewhere`] when a kind already has another owner.
    pub fn register(
        &mut self,
        owner: &'static str,
        applier: std::sync::Arc<dyn EntryApplier>,
    ) -> Result<(), PackError> {
        for kind in applier.kinds() {
            self.kinds.declare(kind.clone(), owner)?;
            self.by_kind.insert(kind, applier.clone());
        }
        Ok(())
    }

    /// The declared kinds.
    #[must_use]
    pub fn kinds(&self) -> &EntryKinds {
        &self.kinds
    }

    fn for_kind(&self, kind: &CanonicalId) -> Option<&std::sync::Arc<dyn EntryApplier>> {
        self.by_kind.get(kind)
    }

    /// Every distinct applier once, lowest phase first (ties by first kind).
    fn in_phase_order(&self) -> std::vec::IntoIter<std::sync::Arc<dyn EntryApplier>> {
        let mut seen: Vec<std::sync::Arc<dyn EntryApplier>> = Vec::new();
        for applier in self.by_kind.values() {
            if !seen.iter().any(|a| std::sync::Arc::ptr_eq(a, applier)) {
                seen.push(applier.clone());
            }
        }
        seen.sort_by_key(|a| a.phase());
        seen.into_iter()
    }
}

/// The outcome of one apply.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyReport {
    /// Entries created.
    pub created: usize,
    /// Entries updated.
    pub updated: usize,
    /// Entries retired.
    pub retired: usize,
    /// Entries left as they were.
    pub unchanged: usize,
    /// What stopped the apply part way, if anything — also on the latch.
    pub anomaly: Option<String>,
}

/// Why an apply was refused before it touched anything.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ApplyError {
    /// The latch holds a newer version.
    #[error("{pack}@{wanted} is older than the applied {pack}@{applied}")]
    Downgrade {
        /// The pack.
        pack: String,
        /// Applied.
        applied: u32,
        /// Wanted.
        wanted: u32,
    },
    /// The pack refused (kinds, requires, digests).
    #[error(transparent)]
    Pack(#[from] PackError),
    /// A retire would orphan a reference.
    #[error("retiring {entry} is refused: {reason}")]
    RetireRefused {
        /// The entry.
        entry: String,
        /// Who still references it.
        reason: String,
    },
    /// The latch record would not build.
    #[error("the latch could not be built: {0}")]
    Latch(#[from] LatchError),
    /// A latch document for this pack is held undecodable; apply refuses rather
    /// than overwrite what it cannot read.
    #[error("the latch for {pack} is held undecodable; repair it before applying")]
    Held {
        /// The pack.
        pack: String,
    },
}

/// Apply `pack` to the world through the composed appliers, honestly latching.
///
/// The fallible half first: requires against the applied packs, every kind owned, the
/// plan, every retire's reference check. Then the irreversible half: retires (highest
/// phase first), creates and updates (lowest phase first), the latch in RAM and in the
/// same batch. Returns what happened; an anomaly in the report is also on the latch.
///
/// # Errors
///
/// [`ApplyError`] when nothing was touched.
pub fn apply(
    ctx: &mut Ctx<'_>,
    latches: &mut Latches,
    appliers: &Appliers,
    pack: &Pack,
) -> Result<(Applied, ApplyReport), ApplyError> {
    let pack_id = pack.manifest().id().clone();
    if latches.is_held(&pack_id) {
        return Err(ApplyError::Held {
            pack: pack_id.to_string(),
        });
    }
    appliers.kinds().check(pack)?;
    crate::pack::check_requires(pack, &latches.loaded())?;
    let previous = latches.applied(&pack_id).cloned();
    let plan = plan(pack, previous.as_ref())?;

    let mut entries: BTreeMap<CanonicalId, AppliedEntry> = previous
        .as_ref()
        .filter(|p| !p.is_retired())
        .map(|p| p.entries().clone())
        .unwrap_or_default();
    let mut placed: BTreeMap<CanonicalId, ObjId> = entries
        .iter()
        .filter_map(|(id, e)| e.object().map(|o| (id.clone(), o)))
        .collect();

    // ---- every retire's reference check, before anything mutates ----
    for planned in plan.at(Step::Retire) {
        let Some(applier) = appliers.for_kind(&planned.kind) else {
            return Err(ApplyError::RetireRefused {
                entry: planned.id.to_string(),
                reason: format!("nothing on this server owns kind {}", planned.kind),
            });
        };
        applier
            .retire_check(ctx, &pack_id, &planned.id, planned.existing, &placed)
            .map_err(|reason| ApplyError::RetireRefused {
                entry: planned.id.to_string(),
                reason,
            })?;
    }

    // ---- the irreversible half ----
    let mut report = ApplyReport {
        created: 0,
        updated: 0,
        retired: 0,
        unchanged: plan.count(Step::Unchanged),
        anomaly: None,
    };

    let mut retires: Vec<&PlannedEntry> = plan.at(Step::Retire).collect();
    retires.sort_by_key(|e| std::cmp::Reverse(phase_of(appliers, &e.kind)));
    'work: {
        for planned in retires {
            let Some(applier) = appliers.for_kind(&planned.kind) else {
                break 'work;
            };
            if let Err(reason) = applier.retire(ctx, &pack_id, &planned.id, planned.existing) {
                report.anomaly = Some(format!("retiring {} stopped: {reason}", planned.id));
                break 'work;
            }
            entries.remove(&planned.id);
            placed.remove(&planned.id);
            report.retired = report.retired.saturating_add(1);
        }
        let mut work: Vec<&PlannedEntry> = plan
            .entries
            .iter()
            .filter(|e| matches!(e.step, Step::Create | Step::Update))
            .collect();
        work.sort_by_key(|e| phase_of(appliers, &e.kind));
        for planned in work {
            let (Some(applier), Some(entry)) =
                (appliers.for_kind(&planned.kind), pack.entry(&planned.id))
            else {
                break 'work;
            };
            let digest = match entry.digest() {
                Ok(d) => d,
                Err(error) => {
                    report.anomaly = Some(format!("{} would not digest: {error}", planned.id));
                    break 'work;
                }
            };
            let made = match planned.step {
                Step::Create => applier.create(ctx, pack, &planned.id, entry, &placed),
                _ => applier.update(ctx, pack, &planned.id, entry, planned.existing, &placed),
            };
            match made {
                Ok(object) => {
                    if let Some(o) = object {
                        placed.insert(planned.id.clone(), o);
                    } else {
                        placed.remove(&planned.id);
                    }
                    entries.insert(
                        planned.id.clone(),
                        AppliedEntry::new(planned.kind.clone(), digest, object),
                    );
                    if planned.step == Step::Create {
                        report.created = report.created.saturating_add(1);
                    } else {
                        report.updated = report.updated.saturating_add(1);
                    }
                }
                Err(reason) => {
                    report.anomaly = Some(format!(
                        "{} {} stopped: {reason}",
                        match planned.step {
                            Step::Create => "creating",
                            _ => "updating",
                        },
                        planned.id
                    ));
                    break 'work;
                }
            }
        }
        // Every entry is in place: each owner's whole-pack pass, lowest phase first.
        for applier in appliers.in_phase_order() {
            if let Err(reason) = applier.finish(ctx, pack, &placed) {
                report.anomaly = Some(format!("finishing {}: {reason}", pack.manifest().id()));
                break 'work;
            }
        }
    }

    let applied_at = ctx.w.world_ic_now();
    let applied = Applied::new(
        pack_id,
        plan.version,
        plan.digest,
        applied_at,
        entries,
        false,
        report.anomaly.clone(),
    )?;
    persist(ctx, latches, applied.clone());
    if let Some(anomaly) = &report.anomaly {
        tracing::warn!(pack = %applied.pack(), %anomaly, "content: an apply stopped part way; latched honestly");
    }
    Ok((applied, report))
}

/// Retire a whole pack: every entry, highest phase first, after every reference check.
///
/// # Errors
///
/// [`ApplyError::RetireRefused`] naming the first reference that would be orphaned;
/// nothing was touched.
pub fn retire_pack(
    ctx: &mut Ctx<'_>,
    latches: &mut Latches,
    appliers: &Appliers,
    pack: &CanonicalId,
    reason: &str,
) -> Result<(Applied, ApplyReport), ApplyError> {
    if latches.is_held(pack) {
        return Err(ApplyError::Held {
            pack: pack.to_string(),
        });
    }
    let Some(previous) = latches.applied(pack).cloned() else {
        return Err(ApplyError::RetireRefused {
            entry: pack.to_string(),
            reason: "it is not applied".to_string(),
        });
    };
    if previous.is_retired() {
        return Err(ApplyError::RetireRefused {
            entry: pack.to_string(),
            reason: "it is already retired".to_string(),
        });
    }
    let mut order: Vec<(&CanonicalId, &AppliedEntry)> = previous.entries().iter().collect();
    let placed = previous.placed();
    for (id, entry) in &order {
        let Some(applier) = appliers.for_kind(entry.kind()) else {
            return Err(ApplyError::RetireRefused {
                entry: id.to_string(),
                reason: format!("nothing on this server owns kind {}", entry.kind()),
            });
        };
        applier
            .retire_check(ctx, pack, id, entry.object(), &placed)
            .map_err(|reason| ApplyError::RetireRefused {
                entry: id.to_string(),
                reason,
            })?;
    }
    order.sort_by_key(|(_, e)| std::cmp::Reverse(phase_of(appliers, e.kind())));
    let mut entries = previous.entries().clone();
    let mut report = ApplyReport {
        created: 0,
        updated: 0,
        retired: 0,
        unchanged: 0,
        anomaly: None,
    };
    for (id, entry) in order {
        let Some(applier) = appliers.for_kind(entry.kind()) else {
            break;
        };
        if let Err(reason) = applier.retire(ctx, pack, id, entry.object()) {
            report.anomaly = Some(format!("retiring {id} stopped: {reason}"));
            break;
        }
        entries.remove(id);
        report.retired = report.retired.saturating_add(1);
    }
    if entries.is_empty() && report.anomaly.is_none() {
        for applier in appliers.in_phase_order().rev() {
            if let Err(reason) = applier.finish_retire(ctx, pack) {
                report.anomaly = Some(format!("finishing the retire of {pack}: {reason}"));
                break;
            }
        }
    }
    let fully = entries.is_empty() && report.anomaly.is_none();
    let anomaly = match (&report.anomaly, fully) {
        (Some(a), _) => Some(a.clone()),
        (None, true) => Some(format!("retired: {reason}")),
        (None, false) => Some(format!("retire stopped with entries left: {reason}")),
    };
    let applied = Applied::new(
        previous.pack().clone(),
        previous.version(),
        previous.digest(),
        ctx.w.world_ic_now(),
        entries,
        fully,
        anomaly,
    )?;
    persist(ctx, latches, applied.clone());
    Ok((applied, report))
}

fn phase_of(appliers: &Appliers, kind: &CanonicalId) -> u8 {
    appliers.for_kind(kind).map_or(u8::MAX, |a| a.phase())
}

fn persist(ctx: &mut Ctx<'_>, latches: &mut Latches, applied: Applied) {
    let key = applied_key(applied.pack());
    latches.set(applied.clone());
    match serde_json::to_value(&applied) {
        Ok(value) => ctx.persist(ModuleOp {
            ns: SmolStr::new_static(NS),
            op: ModuleWrite::JsonSet { key, value },
        }),
        Err(error) => {
            tracing::error!(pack = %applied.pack(), %error, "content: the latch would not encode; the key still bars a re-apply");
            ctx.persist(ModuleOp {
                ns: SmolStr::new_static(NS),
                op: ModuleWrite::JsonSet {
                    key,
                    value: serde_json::json!({ "unencodable": error.to_string() }),
                },
            });
        }
    }
}

//! **The TacSim module state and its documents** (namespace `tacsim`), on
//! `ruleset-shadowrun/src/economy/store.rs`'s exact pattern.
//!
//! | doc | class | corrupt → |
//! |---|---|---|
//! | `tacsim:pack:{id}@{version}` | ledger | **skipped loudly**, and the world boots. The **key still counts**: a version whose row will not parse stays *published*, so it can never be silently re-published under different bytes, and a subject seeded under it still fails closed rather than floating onto a newer version |
//! | `tacsim:seed:{pack}@{version}` | latch | **skipped loudly**, and the world boots — but the **key still bars a re-seed**. This is the one place where losing the body must not lose the fact: "we cannot read the latch" has to mean *do not seed again*, never *seed again* (§3.1 invariant (ii)). The cost is that the seeded objects' pack-local names are gone, so the corridor's routes cannot be re-materialized until `@tacsim/seed` is re-run against a fresh store; the objects themselves are untouched |
//! | `tacsim:pin:{subject}` | pin | **skipped loudly**. A lost pin does **not** float its subject onto a newer version: resolution refuses, naming the pack, because §3.1 invariant (i)'s whole point is that a published contract's assumptions never shift underneath it |
//! | `tacsim:meta` | allocators | tolerated field by field, floors re-derived from every **parsed key**, loaded or not (E6b's R8 finding). Losing it costs nothing |
//!
//! Nothing here fails the boot, and the direction of each loss is deliberate. A
//! malformed pack **file** is the opposite answer — it refuses the boot and names the
//! file — because a file is the operator's declared intent and a stored document is
//! this world's accumulated history: refusing to start over a typo an operator can
//! fix is right, and refusing to start over one row of history is not (10b §4 rule 2).
//!
//! Both loaders apply the **identity rule** (DL-07): a document whose record disagrees
//! with its own key is dropped loudly, never trusted.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CanonicalId, Ctx, ModuleOp, ModuleWrite, ObjId, Outbox};
use mush_sim::{DriveProfile, LegDef, RouteDef};
use smol_str::SmolStr;

use crate::config::{LoadedPack, TacsimConfig};
use crate::pack::{PackEntry, RouteEntry};
use crate::record::{PackRecord, PinRecord, SeedApplication};

/// The module namespace (the module's own name).
pub const NS: &str = "tacsim";

/// The allocator/watermark document.
pub const META_KEY: &str = "tacsim:meta";

/// The namespace's schema version.
pub const SCHEMA_VERSION: u16 = 1;

/// The most rows one `@tacsim/pack list` page renders.
///
/// Bounded on records **visited**, not matched (D-LBK-5), so a viewer who may not see
/// most of them still pays one page of work.
pub const PACK_PAGE: usize = 32;

/// `tacsim:pack:{id}@{version}`.
#[must_use]
pub fn pack_key(id: &CanonicalId, version: u32) -> SmolStr {
    SmolStr::new(format!("tacsim:pack:{id}@{version}"))
}

/// `tacsim:seed:{pack}@{version}` — **the version is in the key** (OPEN 5, §4 rule 3),
/// because invariant (ii) is "at most once per `(pack, version)`".
#[must_use]
pub fn seed_key(pack: &CanonicalId, version: u32) -> SmolStr {
    SmolStr::new(format!("tacsim:seed:{pack}@{version}"))
}

/// `tacsim:pin:{subject}`.
#[must_use]
pub fn pin_key(subject: &CanonicalId) -> SmolStr {
    SmolStr::new(format!("tacsim:pin:{subject}"))
}

/// Split an `{id}@{version}` key tail. `None` when either half is unusable, which is
/// what makes an unreadable key a *loud skip* rather than a silent zero.
fn split_versioned(rest: &str) -> Option<(CanonicalId, u32)> {
    let (id, version) = rest.rsplit_once('@')?;
    let id = CanonicalId::parse_canonical(id).ok()?;
    let version: u32 = version.parse().ok()?;
    (version > 0).then_some((id, version))
}

/// Why resolving a subject to a pack version refuses.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ResolveError {
    /// No loaded pack declares this subject.
    #[error("nothing loaded declares {subject}")]
    Unknown {
        /// The subject asked for.
        subject: String,
    },
    /// The version this subject resolves to is not loaded any more.
    ///
    /// **This is the fail-closed answer**, and it is deliberately not "use the newest
    /// one": §3.1 invariant (i) says an unpinned consumer of a retired version fails
    /// closed rather than silently upgrading, because a published hauling contract's
    /// route assumptions must not shift underneath it (T1).
    #[error(
        "{subject} resolves to {pack}@{version}, which this server no longer loads; it is {how} \
         and will not be silently moved to another version"
    )]
    Retired {
        /// The subject asked for.
        subject: String,
        /// The pack it resolves to.
        pack: String,
        /// The version it resolves to.
        version: u32,
        /// How it resolved — `pinned` or `seeded`.
        how: &'static str,
    },
}

/// The in-RAM TacSim slot — the single runtime source of truth.
#[derive(Clone, Debug, Default)]
pub struct TacsimStore {
    /// Frozen at wiring.
    pub(crate) cfg: TacsimConfig,
    /// The packs the host read and this module verified, keyed `(id, version)`.
    /// **Not persisted**: a pack is a file, and the ledger row below is what the
    /// world remembers about it.
    pub(crate) loaded: BTreeMap<(CanonicalId, u32), LoadedPack>,
    /// `tacsim:pack:{id}@{version}` — the published-version ledger.
    pub(crate) published: BTreeMap<(CanonicalId, u32), PackRecord>,
    /// Every `(id, version)` a `tacsim:pack:` **key** claimed, loaded or not (R8).
    pub(crate) published_keys: BTreeSet<(CanonicalId, u32)>,
    /// `tacsim:seed:{pack}@{version}` — the idempotency latches.
    pub(crate) seeded: BTreeMap<(CanonicalId, u32), SeedApplication>,
    /// Every `(pack, version)` a `tacsim:seed:` **key** claimed, loaded or not (R8).
    /// **This, not `seeded`, is what bars a second `commit`.**
    pub(crate) seeded_keys: BTreeSet<(CanonicalId, u32)>,
    /// **pack → applied version**, re-derived from `seeded_keys` on every write and
    /// at boot. Never authority in its own right; the keys are.
    pub(crate) applied: BTreeMap<CanonicalId, u32>,
    /// `tacsim:pin:{subject}`.
    pub(crate) pins: BTreeMap<CanonicalId, PinRecord>,
    /// The materialized routes, re-derived from the loaded packs and the seed
    /// latches' placement maps. A projection, never a document.
    pub(crate) routes: BTreeMap<CanonicalId, RouteDef>,
    /// The highest published version per pack, floored on every parsed key.
    pub(crate) watermark: BTreeMap<CanonicalId, u32>,
}

impl TacsimStore {
    /// The frozen `[tacsim]` block.
    #[must_use]
    pub fn config(&self) -> &TacsimConfig {
        &self.cfg
    }

    /// Whether the layer is enabled at all.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.cfg.enabled
    }

    /// Every loaded pack, in `(id, version)` order.
    pub fn packs(&self) -> impl Iterator<Item = &LoadedPack> {
        self.loaded.values()
    }

    /// One loaded pack.
    #[must_use]
    pub fn pack(&self, id: &CanonicalId, version: u32) -> Option<&LoadedPack> {
        self.loaded.get(&(id.clone(), version))
    }

    /// Every loaded version of one pack, ascending.
    pub fn versions_of(&self, id: &CanonicalId) -> impl Iterator<Item = u32> + '_ {
        let id = id.clone();
        self.loaded
            .keys()
            .filter(move |(pack, _)| *pack == id)
            .map(|(_, version)| *version)
    }

    /// The highest loaded version of one pack.
    #[must_use]
    pub fn newest_loaded(&self, id: &CanonicalId) -> Option<u32> {
        self.versions_of(id).max()
    }

    /// The published-version ledger.
    pub fn published(&self) -> impl Iterator<Item = &PackRecord> {
        self.published.values()
    }

    /// The seed latch for one `(pack, version)`.
    #[must_use]
    pub fn seed(&self, pack: &CanonicalId, version: u32) -> Option<&SeedApplication> {
        self.seeded.get(&(pack.clone(), version))
    }

    /// **Whether this `(pack, version)` has already been applied.**
    ///
    /// Derived from the *keys*, so a latch whose body will not parse still bars a
    /// second `commit`. That is the only fail-closed direction: reading "cannot parse"
    /// as "not applied" would duplicate a mine.
    #[must_use]
    pub fn is_applied(&self, pack: &CanonicalId, version: u32) -> bool {
        self.seeded_keys.contains(&(pack.clone(), version))
    }

    /// The version of `pack` this world was seeded at, if any.
    #[must_use]
    pub fn applied_version(&self, pack: &CanonicalId) -> Option<u32> {
        self.applied.get(pack).copied()
    }

    /// One subject's pin.
    #[must_use]
    pub fn pin(&self, subject: &CanonicalId) -> Option<&PinRecord> {
        self.pins.get(subject)
    }

    /// Every pin, in subject order.
    pub fn pins(&self) -> impl Iterator<Item = &PinRecord> {
        self.pins.values()
    }

    /// One materialized route.
    #[must_use]
    pub fn route(&self, id: &CanonicalId) -> Option<&RouteDef> {
        self.routes.get(id)
    }

    /// Every materialized route, in id order.
    pub fn routes(&self) -> impl Iterator<Item = &RouteDef> {
        self.routes.values()
    }

    /// Every object this world's seed latches created, pack-local id to object.
    pub fn seeded_objects(&self) -> impl Iterator<Item = (&CanonicalId, ObjId)> {
        self.seeded
            .values()
            .flat_map(|s| s.placed().iter().map(|(id, obj)| (id, *obj)))
    }

    /// **Resolve a subject to the exact pack version that governs it** (§3.1
    /// invariant (i)).
    ///
    /// A pin wins and is exact. Failing that, a pack this world was *seeded* at holds
    /// its subjects at that version. Only a subject with neither reaches "the newest
    /// loaded version", and a retired version on either of the first two paths
    /// **refuses** rather than upgrading.
    ///
    /// # Errors
    ///
    /// [`ResolveError::Unknown`] and [`ResolveError::Retired`].
    pub fn resolve(&self, subject: &CanonicalId) -> Result<(CanonicalId, u32), ResolveError> {
        if let Some(pin) = self.pins.get(subject) {
            let key = (pin.pack().clone(), pin.version());
            if self.loaded.contains_key(&key) {
                return Ok(key);
            }
            return Err(ResolveError::Retired {
                subject: subject.to_string(),
                pack: pin.pack().to_string(),
                version: pin.version(),
                how: "pinned",
            });
        }
        // Which loaded packs declare this subject at all? A subject nothing declares
        // is unknown; the answer must not depend on which version happened to load.
        let mut candidates: BTreeSet<CanonicalId> = BTreeSet::new();
        for ((id, _), loaded) in &self.loaded {
            if loaded.pack.entry(subject).is_some() {
                candidates.insert(id.clone());
            }
        }
        // A pack this world was seeded at governs its own subjects at that version
        // even if no loaded version declares them any more — otherwise retiring a
        // version would silently re-point every seeded subject.
        for (pack, version) in &self.applied {
            let key = (pack.clone(), *version);
            if self.loaded.contains_key(&key) {
                continue;
            }
            let ever_declared = candidates.contains(pack) || self.published_keys.contains(&key);
            if ever_declared {
                return Err(ResolveError::Retired {
                    subject: subject.to_string(),
                    pack: pack.to_string(),
                    version: *version,
                    how: "seeded",
                });
            }
        }
        for pack in &candidates {
            if let Some(version) = self.applied.get(pack) {
                let key = (pack.clone(), *version);
                if self.loaded.contains_key(&key) {
                    return Ok(key);
                }
            }
        }
        let mut best: Option<(CanonicalId, u32)> = None;
        for (id, version) in self.loaded.keys() {
            if !candidates.contains(id) {
                continue;
            }
            let better = best.as_ref().is_none_or(|(_, current)| version > current);
            if better {
                best = Some((id.clone(), *version));
            }
        }
        best.ok_or_else(|| ResolveError::Unknown {
            subject: subject.to_string(),
        })
    }

    /// Install the packs the host read and this module verified.
    pub(crate) fn install_packs(&mut self, packs: Vec<LoadedPack>) {
        self.loaded.clear();
        for loaded in packs {
            let key = (
                loaded.pack.manifest().id().clone(),
                loaded.pack.manifest().version(),
            );
            self.loaded.insert(key, loaded);
        }
    }

    /// The allocator document's body.
    #[must_use]
    pub(crate) fn meta_doc(&self) -> serde_json::Value {
        serde_json::json!({
            "schema": SCHEMA_VERSION,
            "watermark": self
                .watermark
                .iter()
                .map(|(id, version)| (id.to_string(), serde_json::json!(*version)))
                .collect::<serde_json::Map<String, serde_json::Value>>(),
        })
    }

    /// Re-derive every index from the records. Called on every write and at boot, so
    /// no index is ever authority in its own right.
    pub(crate) fn reindex(&mut self) {
        self.applied.clear();
        for (pack, version) in &self.seeded_keys {
            let entry = self.applied.entry(pack.clone()).or_insert(*version);
            *entry = (*entry).max(*version);
        }
        for (pack, version) in self.published_keys.iter().chain(self.seeded_keys.iter()) {
            let entry = self.watermark.entry(pack.clone()).or_insert(*version);
            *entry = (*entry).max(*version);
        }
        self.reindex_routes();
    }

    /// Re-materialize every route the loaded packs declare and the seed latches
    /// placed. A route whose places are not all placed is **skipped loudly**: it is a
    /// projection, and answering with a half-resolved route would be worse than
    /// answering with none.
    fn reindex_routes(&mut self) {
        self.routes.clear();
        let mut built: Vec<(CanonicalId, RouteDef)> = Vec::new();
        for ((pack, version), loaded) in &self.loaded {
            let Some(seed) = self.seeded.get(&(pack.clone(), *version)) else {
                continue;
            };
            for (id, entry) in loaded.pack.entries() {
                let PackEntry::Route(route) = entry else {
                    continue;
                };
                match materialize_route(id, route, seed.placed()) {
                    Ok(def) => built.push((id.clone(), def)),
                    Err(detail) => tracing::warn!(
                        route = %id,
                        pack = %pack,
                        version,
                        %detail,
                        "tacsim: a pack route could not be materialized; skipped"
                    ),
                }
            }
        }
        for (id, def) in built {
            self.routes.insert(id, def);
        }
    }
}

/// Resolve one pack-authored route against a seed's placement map.
fn materialize_route(
    id: &CanonicalId,
    route: &RouteEntry,
    placed: &BTreeMap<CanonicalId, ObjId>,
) -> Result<RouteDef, String> {
    let place = |symbol: &CanonicalId| -> Result<ObjId, String> {
        placed
            .get(symbol)
            .copied()
            .ok_or_else(|| format!("the seed placed no object for {symbol}"))
    };
    let from = place(&route.from)?;
    let to = place(&route.to)?;
    let mut legs = Vec::with_capacity(route.legs.len());
    for leg in &route.legs {
        let leg_to = place(&leg.to)?;
        legs.push(LegDef::new(leg_to, leg.base_s).map_err(|e| e.to_string())?);
    }
    RouteDef::new(
        id.clone(),
        (from, to),
        legs,
        route.alignment_band.map(mush_sim::CadenceId),
        route.drive_class.clone(),
        route.propellant,
    )
    .map_err(|e| e.to_string())
}

/// The drive profile a route's declared class is evaluated under.
///
/// Cut 1 authors no propulsion table — that is cut 11's full rules pack — so this is
/// the one nominal profile the fixture's `@tacsim/pack show` and the route tests use.
/// It is a **function of the class name only**, so it stays pure.
#[must_use]
pub fn nominal_drive(class: &CanonicalId) -> Option<DriveProfile> {
    DriveProfile::new(class.clone(), 1_000, 1_000).ok()
}

/// Queue one module write.
pub(crate) fn push_op(out: &mut Outbox, op: ModuleWrite) {
    out.defer(mush_core::Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op,
    }));
}

/// Install a ledger row into RAM and queue its document — the single write path.
pub(crate) fn put_pack(ctx: &mut Ctx<'_>, record: PackRecord) {
    let key = pack_key(record.id(), record.version());
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        let index = (record.id().clone(), record.version());
        store.published_keys.insert(index.clone());
        store.published.insert(index, record.clone());
        store.reindex();
    }
    let Ok(value) = serde_json::to_value(&record) else {
        tracing::error!(%key, "tacsim: a pack ledger row did not serialize");
        return;
    };
    push_op(ctx.out, ModuleWrite::JsonSet { key, value });
}

/// Install a seed latch into RAM and queue its document — the single write path, and
/// the one that must ride the **same batch** as the objects it names.
pub(crate) fn put_seed(ctx: &mut Ctx<'_>, record: SeedApplication) {
    let key = seed_key(record.pack(), record.version());
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        let index = (record.pack().clone(), record.version());
        store.seeded_keys.insert(index.clone());
        store.seeded.insert(index, record.clone());
        store.reindex();
    }
    let Ok(value) = serde_json::to_value(&record) else {
        tracing::error!(%key, "tacsim: a seed latch did not serialize");
        return;
    };
    push_op(ctx.out, ModuleWrite::JsonSet { key, value });
}

/// Install a pin into RAM and queue its document — the single write path.
pub(crate) fn put_pin(ctx: &mut Ctx<'_>, record: PinRecord) {
    let key = pin_key(record.subject());
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.pins.insert(record.subject().clone(), record.clone());
    }
    let Ok(value) = serde_json::to_value(&record) else {
        tracing::error!(%key, "tacsim: a pin did not serialize");
        return;
    };
    push_op(ctx.out, ModuleWrite::JsonSet { key, value });
}

/// Queue the allocator document.
pub(crate) fn persist_meta(ctx: &mut Ctx<'_>) {
    let Some(meta) = ctx
        .w
        .module_state::<TacsimStore>()
        .map(TacsimStore::meta_doc)
    else {
        return;
    };
    push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: SmolStr::new_static(META_KEY),
            value: meta,
        },
    );
}

/// Build the slot from the module's boot documents.
///
/// **Total**: every document it cannot use is skipped loudly and the world boots.
/// Every index floor is derived from the documents' **parsed keys**, loaded or not
/// (E6b's R8), which is what makes a corrupt seed latch still bar a re-seed.
#[must_use]
pub fn load(kv: &[(SmolStr, serde_json::Value)], cfg: TacsimConfig) -> TacsimStore {
    let mut store = TacsimStore {
        cfg,
        ..TacsimStore::default()
    };
    for (key, value) in kv {
        let key = key.as_str();
        if let Some(rest) = key.strip_prefix("tacsim:pack:") {
            let Some((id, version)) = split_versioned(rest) else {
                tracing::warn!(%key, "tacsim: a pack ledger row has an unreadable key; skipped");
                continue;
            };
            // The claim on the version is the KEY's, not the body's.
            store.published_keys.insert((id.clone(), version));
            load_pack_row(&mut store, key, &id, version, value);
        } else if let Some(rest) = key.strip_prefix("tacsim:seed:") {
            let Some((pack, version)) = split_versioned(rest) else {
                tracing::warn!(%key, "tacsim: a seed latch has an unreadable key; skipped");
                continue;
            };
            // **The latch is the key.** A body that will not parse costs the
            // placement map, never the bar on re-seeding.
            store.seeded_keys.insert((pack.clone(), version));
            load_seed_row(&mut store, key, &pack, version, value);
        } else if let Some(rest) = key.strip_prefix("tacsim:pin:") {
            let Ok(subject) = CanonicalId::parse_canonical(rest) else {
                tracing::warn!(%key, "tacsim: a pin has an unreadable key; skipped");
                continue;
            };
            load_pin_row(&mut store, key, &subject, value);
        } else if key == META_KEY {
            load_meta(&mut store, value);
        }
        // Every other key belongs to a forward schema; tolerated in both directions.
    }
    store.reindex();
    store
}

fn load_pack_row(
    store: &mut TacsimStore,
    key: &str,
    id: &CanonicalId,
    version: u32,
    value: &serde_json::Value,
) {
    match serde_json::from_value::<PackRecord>(value.clone()) {
        Ok(record) if record.id() == id && record.version() == version => {
            store.published.insert((id.clone(), version), record);
        }
        Ok(record) => tracing::warn!(
            %key,
            record = %record.id(),
            "tacsim: a pack ledger row disagrees with its own key; skipped"
        ),
        Err(error) => {
            tracing::warn!(%key, %error, "tacsim: a pack ledger row did not parse; skipped");
        }
    }
}

fn load_seed_row(
    store: &mut TacsimStore,
    key: &str,
    pack: &CanonicalId,
    version: u32,
    value: &serde_json::Value,
) {
    match serde_json::from_value::<SeedApplication>(value.clone()) {
        Ok(record) if record.pack() == pack && record.version() == version => {
            store.seeded.insert((pack.clone(), version), record);
        }
        Ok(record) => tracing::warn!(
            %key,
            record = %record.pack(),
            "tacsim: a seed latch disagrees with its own key; skipped (the key still bars a re-seed)"
        ),
        Err(error) => tracing::warn!(
            %key, %error,
            "tacsim: a seed latch did not parse; skipped (the key still bars a re-seed)"
        ),
    }
}

fn load_pin_row(
    store: &mut TacsimStore,
    key: &str,
    subject: &CanonicalId,
    value: &serde_json::Value,
) {
    match serde_json::from_value::<PinRecord>(value.clone()) {
        Ok(record) if record.subject() == subject => {
            store.pins.insert(subject.clone(), record);
        }
        Ok(record) => tracing::warn!(
            %key,
            record = %record.subject(),
            "tacsim: a pin disagrees with its own key; skipped"
        ),
        Err(error) => tracing::warn!(%key, %error, "tacsim: a pin did not parse; skipped"),
    }
}

fn load_meta(store: &mut TacsimStore, value: &serde_json::Value) {
    // Tolerated field by field: a meta document that is half nonsense contributes
    // whatever floors it can and the parsed keys supply the rest.
    let Some(watermark) = value
        .get("watermark")
        .and_then(serde_json::Value::as_object)
    else {
        return;
    };
    for (id, version) in watermark {
        let Ok(id) = CanonicalId::parse_canonical(id) else {
            continue;
        };
        let Some(version) = version.as_u64().and_then(|v| u32::try_from(v).ok()) else {
            continue;
        };
        let entry = store.watermark.entry(id).or_insert(version);
        *entry = (*entry).max(version);
    }
}

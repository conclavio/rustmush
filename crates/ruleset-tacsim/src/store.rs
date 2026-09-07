//! **The TacSim module state and its documents** (namespace `tacsim`), on
//! `ruleset-shadowrun/src/economy/store.rs`'s exact pattern.
//!
//! | doc | class | corrupt → |
//! |---|---|---|
//! | `tacsim:pack:{id}@{version}` | ledger | **skipped loudly**, and the world boots. The **key still counts**: a version whose row will not parse stays *published*, so it is never quietly treated as new. What is lost with the body is the **digest that version was first published under**, and there is no way to recover it — so boot repairs the row from the loaded pack and **pages level 2 naming the pack** (08c §4.6), because for that one version the in-place-re-publication gate below is disarmed until an operator re-checks it. A subject seeded under it still fails closed rather than floating onto a newer version |
//! | `tacsim:seed:{pack}@{version}` | latch | **skipped loudly**, and the world boots — but the **key still bars a re-seed**. This is the one place where losing the body must not lose the fact: "we cannot read the latch" has to mean *do not seed again*, never *seed again* (§3.1 invariant (ii)). The cost is that the seeded objects' pack-local names are gone, so the corridor's routes cannot be re-materialized until `@tacsim/seed` is re-run against a fresh store; the objects themselves are untouched |
//! | `tacsim:pin:{subject}` | pin | **skipped loudly**. A lost pin does **not** float its subject onto a newer version: resolution refuses, naming the pack, because §3.1 invariant (i)'s whole point is that a published contract's assumptions never shift underneath it |
//! | `tacsim:meta` | allocators | tolerated field by field, floors re-derived from every **parsed key**, loaded or not (E6b's R8 finding). Losing it costs nothing |
//!
//! Nothing **in this loader** fails the boot, and the direction of each loss is
//! deliberate. A malformed pack **file** is the opposite answer — it refuses the boot
//! and names the file — because a file is the operator's declared intent and a stored
//! document is this world's accumulated history: refusing to start over a typo an
//! operator can fix is right, and refusing to start over one row of history is not
//! (10b §4 rule 2).
//!
//! # Where a stored row DOES refuse the boot
//!
//! One place, and it is a statement about a **file**, not about the row: a ledger row
//! that parses and whose digest disagrees with the loaded pack for that exact
//! `(id, version)` means the operator re-signed a published version in place.
//! `crate::boot_reconcile` refuses the boot naming the file and both digests (the owner
//! decision of 2026-09-05) — never page-and-drop, never adopt — because a seeded world
//! depends on the exact bytes and a published version is never mutated: a change
//! publishes a **new version that coexists**. The row is history; the disagreement is
//! the typo.
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

/// The most rows one `@tacsim` listing page renders — `pack list`, `pack show`,
/// `pack verify` and `pin list`, all four (independent verify B1/B3).
///
/// Bounded on records **visited**, not matched (D-LBK-5), so a viewer who may not see
/// most of them still pays one page of work.
pub const PACK_PAGE: usize = 32;

/// The most pins one world may carry (independent verify B4).
///
/// `tacsim:pin:{subject}` was the one document family in the crate with no cap, no
/// `Del` path and no verb to see it: the family grew monotonically, was reloaded in
/// full at every boot and was invisible from the game. 10b §7 gives a cap to every
/// other list in the programme; a pin is a per-subject migration decision, so this sits
/// with the programme's other per-subject caps.
pub const MAX_PINS: usize = 128;

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
    ///
    /// Keyed by `(pack, version, route id)` and **not** by the route's pack-local id
    /// alone (independent verify S4): a route id is pack-local by definition, so a
    /// single-id key let a second seeded pack whose id merely sorts later silently
    /// replace the corridor's `ceres_phobos` — deterministic, so not a replay
    /// divergence, but a fail-open shadowing of the very definition T1 rests on, with
    /// no way to ask for a specific pack's route.
    pub(crate) routes: BTreeMap<(CanonicalId, u32, CanonicalId), RouteDef>,
    /// Per latch, the pack-local symbols whose placed object this world no longer has
    /// (independent verify S3). Re-derived at boot against the live world, never a
    /// document; a route through one of them is skipped rather than materialized over
    /// a tombstone.
    pub(crate) missing: BTreeMap<(CanonicalId, u32), BTreeSet<CanonicalId>>,
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

    /// **The digest this world published one version under**, if it can still read the
    /// row.
    ///
    /// The one read of `PackRecord::digest` that makes the ledger's digest field mean
    /// something (independent verify P1): `boot_reconcile` compares it against the
    /// loaded pack and refuses the boot on a disagreement, and `@tacsim/pack verify`
    /// reports the same fact rather than re-proving an equality `load_pack` already
    /// established.
    #[must_use]
    pub fn published_digest(
        &self,
        id: &CanonicalId,
        version: u32,
    ) -> Option<mush_core::DefinitionDigest> {
        self.published
            .get(&(id.clone(), version))
            .map(PackRecord::digest)
    }

    /// Resolve one recipe id in a published pack version to its **pack digest**
    /// and its authored definition (E7 cut 4's `RecipeAuthority` seam, read at the
    /// composition root). `None` if the pack version is not **published** (a
    /// retired version fails closed here) or the id is not a recipe — no second
    /// digest, no second registry (§9 q3): the pack's own published digest is the
    /// only one.
    #[must_use]
    pub fn recipe(
        &self,
        pack: &CanonicalId,
        version: u32,
        id: &CanonicalId,
    ) -> Option<(mush_core::DefinitionDigest, crate::pack::RecipeDef)> {
        let digest = self.published_digest(pack, version)?;
        match self.pack(pack, version)?.pack.entry(id) {
            Some(crate::pack::PackEntry::Recipe(def)) => Some((digest, def.clone())),
            _ => None,
        }
    }

    /// **Every formation kind this world's PUBLISHED packs author**, in
    /// `(pack, version, id)` order (E7 cut 9's `FormationAuthority` seam, read at
    /// the composition root).
    ///
    /// A retired version contributes nothing — `published_digest` answers `None`
    /// for one — which is the same fail-closed rule [`TacsimStore::recipe`] uses.
    /// The walk is bounded by the pack cap on both axes.
    #[must_use]
    pub fn formation_kinds(&self) -> Vec<(CanonicalId, crate::pack::FormationKindDef)> {
        let mut out = Vec::new();
        for (pack, version) in self.published.keys() {
            if self.published_digest(pack, *version).is_none() {
                continue;
            }
            let Some(loaded) = self.pack(pack, *version) else {
                continue;
            };
            for (id, entry) in loaded.pack.entries() {
                if let crate::pack::PackEntry::Formation(def) = entry {
                    out.push((id.clone(), def.clone()));
                }
            }
        }
        out
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

    /// **One materialized route, for the pack version that governs its id.**
    ///
    /// A route id is pack-local, so answering by id alone needs a rule for which pack's
    /// route that is: [`TacsimStore::resolve`]'s, the same rule every other subject
    /// goes through. Ask for a specific pack's route with [`TacsimStore::route_in`].
    #[must_use]
    pub fn route(&self, id: &CanonicalId) -> Option<&RouteDef> {
        let (pack, version) = self.resolve(id).ok()?;
        self.routes.get(&(pack, version, id.clone()))
    }

    /// One materialized route, named by the pack and version that declares it.
    #[must_use]
    pub fn route_in(
        &self,
        pack: &CanonicalId,
        version: u32,
        id: &CanonicalId,
    ) -> Option<&RouteDef> {
        self.routes.get(&(pack.clone(), version, id.clone()))
    }

    /// Every materialized route, in `(pack, version, id)` order.
    pub fn routes(&self) -> impl Iterator<Item = (&CanonicalId, u32, &RouteDef)> {
        self.routes
            .iter()
            .map(|((pack, version, _), def)| (pack, *version, def))
    }

    /// Every seed latch, keyed `(pack, version)`.
    pub(crate) fn latches(&self) -> impl Iterator<Item = (&(CanonicalId, u32), &SeedApplication)> {
        self.seeded.iter()
    }

    /// Every object this world's seed latches created **and still has**, pack-local id
    /// to object.
    ///
    /// A placement whose object is gone is omitted rather than handed out (S3); it is
    /// reported by [`TacsimStore::missing_placements`], which boot has already paged.
    pub fn seeded_objects(&self) -> impl Iterator<Item = (&CanonicalId, ObjId)> {
        self.seeded.iter().flat_map(|(key, seed)| {
            let gone = self.missing.get(key);
            seed.placed()
                .iter()
                .filter(move |(id, _)| !gone.is_some_and(|set| set.contains(*id)))
                .map(|(id, obj)| (id, *obj))
        })
    }

    /// Per latch, how many of its placed objects this world no longer has (S3).
    ///
    /// Empty on a healthy world. Every entry was paged at boot, level 2, naming the
    /// pack and the command that shows it.
    pub fn missing_placements(&self) -> impl Iterator<Item = (&CanonicalId, u32, usize)> {
        self.missing
            .iter()
            .map(|((pack, version), gone)| (pack, *version, gone.len()))
    }

    /// Record the placements boot found the world no longer has, and re-project.
    pub(crate) fn record_missing_placements(
        &mut self,
        missing: Vec<((CanonicalId, u32), CanonicalId)>,
    ) {
        for (key, symbol) in missing {
            self.missing.entry(key).or_default().insert(symbol);
        }
        self.reindex_routes();
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
        // Which packs could govern THIS subject at all? A subject nothing declares is
        // unknown; the answer must not depend on which version happened to load.
        //
        // Two sources, and both are about the subject (independent verify P2). A
        // loaded pack that declares it is the ordinary one. The second is a pack this
        // world was *seeded* at whose latch actually placed this symbol: that pack
        // governs what it made even when no loaded version declares it any more.
        //
        // What is deliberately NOT a source is `published_keys`. It carries a row for
        // every version any `tacsim:pack:` key ever claimed, so using it here made the
        // retired-seed arm true for any applied-but-unloaded pack **whatever was
        // asked** — dropping one already-applied campaign-seed file from `seed_paths`,
        // an ordinary operator action once the seed has run, made `resolve` refuse for
        // every subject in every still-loaded rules pack and misattribute each to the
        // unrelated seed pack. Invariant (i)'s rule is per-subject; that was a total
        // denial of the store's headline query. It may widen the *evidence* that a
        // candidate was retired; it may never make an unrelated pack a candidate.
        let mut candidates: BTreeSet<CanonicalId> = BTreeSet::new();
        for ((id, _), loaded) in &self.loaded {
            if loaded.pack.entry(subject).is_some() {
                candidates.insert(id.clone());
            }
        }
        for ((pack, version), seed) in &self.seeded {
            if seed.placed().contains_key(subject) && self.applied.get(pack) == Some(version) {
                candidates.insert(pack.clone());
            }
        }
        // A candidate this world was seeded at, whose version is gone, **refuses** —
        // fail closed, naming the pack that actually declares the subject asked for,
        // rather than silently re-pointing it at a newer version.
        for pack in &candidates {
            let Some(version) = self.applied.get(pack) else {
                continue;
            };
            if self.loaded.contains_key(&(pack.clone(), *version)) {
                continue;
            }
            return Err(ResolveError::Retired {
                subject: subject.to_string(),
                pack: pack.to_string(),
                version: *version,
                how: "seeded",
            });
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
        let mut built: Vec<((CanonicalId, u32, CanonicalId), RouteDef)> = Vec::new();
        for ((pack, version), loaded) in &self.loaded {
            let key = (pack.clone(), *version);
            let Some(seed) = self.seeded.get(&key) else {
                continue;
            };
            let gone = self.missing.get(&key);
            for (id, entry) in loaded.pack.entries() {
                let PackEntry::Route(route) = entry else {
                    continue;
                };
                match materialize_route(id, route, seed.placed(), gone) {
                    // Keyed by the pack that declares it: a pack-local id is not a
                    // global one, and a later-sorting pack must not shadow an earlier
                    // one's route (S4).
                    Ok(def) => built.push(((pack.clone(), *version, id.clone()), def)),
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
        for (key, def) in built {
            self.routes.insert(key, def);
        }
    }
}

/// Resolve one pack-authored route against a seed's placement map.
///
/// `gone` is the set of pack-local symbols this world no longer has an object for
/// (S3): a route through one of them is refused here rather than handed out over a
/// tombstone.
fn materialize_route(
    id: &CanonicalId,
    route: &RouteEntry,
    placed: &BTreeMap<CanonicalId, ObjId>,
    gone: Option<&BTreeSet<CanonicalId>>,
) -> Result<RouteDef, String> {
    let place = |symbol: &CanonicalId| -> Result<ObjId, String> {
        if gone.is_some_and(|set| set.contains(symbol)) {
            return Err(format!(
                "the object the seed placed for {symbol} is not in this world any more"
            ));
        }
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
///
/// # Propellant is INSIDE `payload` (cut 6 reads this as the ruling)
///
/// Independent verify P7: the nominal capacity was 1 000 while the shipped corridor
/// declares `propellant: 1200`, so evaluating the fixture's one route **with its own
/// declared propellant load** was `RouteDefError::Overloaded` — the two numbers in the
/// same fixture contradicted each other, and nothing noticed because every test
/// evaluated at `payload = 0`. The two numbers are made consistent in the one place a
/// number is authored that does not move a published digest: the capacity, here. A
/// journey's propellant plan counts against the drive's payload capacity, so
/// `route_duration(route, nominal_drive(class), route.propellant())` is the empty-hold
/// case and any cargo is added to it. [`MAX_ROUTE_PROPELLANT`] bounds what a pack may
/// declare; this figure bounds what the one nominal drive will lift.
///
/// [`MAX_ROUTE_PROPELLANT`]: crate::pack::MAX_ROUTE_PROPELLANT
#[must_use]
pub fn nominal_drive(class: &CanonicalId) -> Option<DriveProfile> {
    DriveProfile::new(class.clone(), 1_000, 4_000).ok()
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
    // **The one `else` arm here that must not drop the write** (independent verify S5).
    // In `put_pack`/`put_pin` dropping is harmless; here the objects `seed::apply`
    // created are already dirty and are persisted by the very next `flush_dirty`, so
    // dropping the latch would leave a reboot with the rooms and no latch —
    // `is_applied` false, and a second `commit` duplicating the mine. That is the one
    // fail-open direction the design forbids, and it was silent by construction. It is
    // structurally unreachable (`SeedApplicationWire` is `CanonicalId`/`u32`/`String`/
    // `ClockStamp`/`ObjId`, and `CanonicalId::serialize` is `serialize_str`, so it is a
    // valid `serde_json` map key and `to_value` cannot fail — pinned by
    // `tests::s5_a_full_seed_latch_always_serializes`), so this is the shape being made
    // right rather than a live defect: page level 2, and latch a minimal document so
    // **the key still exists**, since the key is what bars a re-seed.
    let value = match serde_json::to_value(&record) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(
                %key, %error,
                objects = record.objects(),
                "tacsim: a seed latch did not serialize; {} object(s) were created and a re-seed \
                 would duplicate them. A minimal latch is written so the key still bars one. \
                 Recommended: `@tacsim/pack show {}@{}`, then reconcile by hand. Only this pack \
                 is stopped.",
                record.objects(),
                record.pack(),
                record.version()
            );
            let mut fallback = serde_json::json!({
                "pack": record.pack().to_string(),
                "version": record.version(),
                "digest": record.digest().to_hex(),
                "objects": 0,
                "placed": serde_json::Map::new(),
            });
            if let Ok(applied_at) = serde_json::to_value(record.applied_at()) {
                fallback["applied_at"] = applied_at;
            }
            fallback
        }
    };
    push_op(ctx.out, ModuleWrite::JsonSet { key, value });
}

/// Remove a pin from RAM and queue its document's deletion — the crate's one `Del`.
///
/// Safe because no index floor is derived from a `tacsim:pin:` key: `reindex` floors
/// the watermark on `published_keys` and `seeded_keys` only, so removing a pin removes
/// exactly the pin (independent verify B4).
pub(crate) fn del_pin(ctx: &mut Ctx<'_>, subject: &CanonicalId) {
    let key = pin_key(subject);
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.pins.remove(subject);
    }
    push_op(ctx.out, ModuleWrite::Del { key });
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

#[cfg(test)]
mod tests {
    use super::*;

    // ---- E7 cut 1 independent verify fold ----

    /// **S5 — the one write path that must not drop its document.**
    ///
    /// [`put_seed`]'s serialize-failure arm is structurally unreachable, and this pins
    /// that it stays so rather than leaving it assumed: a full latch — `MAX_SEED_OBJECTS`
    /// placements, a real `ClockStamp`, a real digest — round-trips through
    /// `serde_json::to_value` and back. `CanonicalId::serialize` is `serialize_str`, so
    /// every `placed` key is a valid JSON map key and there is no field in
    /// `SeedApplicationWire` that can refuse. The loud fallback in `put_seed` is the
    /// belt to this braces.
    #[test]
    fn s5_a_full_seed_latch_always_serializes() {
        let mut placed: BTreeMap<CanonicalId, ObjId> = BTreeMap::new();
        for i in 0..crate::pack::MAX_SEED_OBJECTS {
            let id = CanonicalId::new(format!("place_{i}")).expect("a canonical id");
            placed.insert(id, ObjId(u32::try_from(i).unwrap_or(0) + 2));
        }
        let record = SeedApplication::new(
            CanonicalId::new("t.big").expect("a canonical id"),
            1,
            mush_core::DefinitionDigest::from_bytes([7_u8; 32]),
            mush_core::ClockStamp {
                clock: mush_core::ClockRef::WorldIc { generation: 1 },
                at: 1_234,
            },
            placed,
        )
        .expect("a full latch builds");
        let value = serde_json::to_value(&record).expect("a full latch always serializes");
        let back: SeedApplication =
            serde_json::from_value(value).expect("and re-enters through its wire");
        assert_eq!(back, record);
        assert_eq!(back.objects() as usize, crate::pack::MAX_SEED_OBJECTS);
    }
}

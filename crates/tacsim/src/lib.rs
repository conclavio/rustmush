//! `tacsim` — **TacSim's packs, its seed latch and the corridor fixture**
//! (E7 cut 1; [`docs/design/10b-e7-implementation-plan.md`] §3.1).
//!
//! # It is a layer, not a `[rulesets]` id
//!
//! Doc 10 §20: *TacSim supplies physical and civil context, not ruleset metaphysics …
//! a ruleset decides what those facts mean to its characters.* So this crate is a
//! plain [`GameModule`] with **no `SheetSchema`** — `mush_ruleset::Ruleset` requires
//! one and cut 1 has none — enabled by `[tacsim].enabled` **independently of**
//! `[rulesets].enabled`, which stays exactly one of `wod` | `shadowrun` with
//! `ruleset_modules` unchanged. The server composes it **after** the character
//! ruleset so it can read that ruleset's facts and the ruleset can read TacSim's
//! environmental facts through the generic seam; the order position is pinned by
//! `mush_server::modules`' boot-order test. The crate keeps doc 10's name because its
//! packs carry rules.
//!
//! # What cut 1 is, and what it deliberately is not
//!
//! Three things: **[`pack`]** — the identity of a content pack and the strict, bounded
//! loader that admits one; **[`seed`]** — the once-per-`(pack, version)` latch that
//! instantiates a campaign-seed pack; and **the corridor fixture** — three places, ten
//! commodity family *ids*, one recipe and one route, so cut 2 has a world to run in
//! and the owner sees output first.
//!
//! It builds **nothing** from cuts 2–11: no quantities, no pools, no lots, no
//! deposits, no reservations, no domain clock, no recovery (10b §9 q4 — *there is no
//! interim*; an object-per-unit stand-in would be a second quantity authority, which
//! is exactly what Q13A exists to prevent). It registers **no** `FactProvider`, **no**
//! canon outcome kind, **no** authority-guard validator, **no** clock provider, **no**
//! schedule source and **no** typed state family; every one of those is a later cut's.
//! It touches `mush-core`'s resources not at all: `ReservationBook`/`ReservationRecord`
//! are cut 2's, and cut 1 holds nothing and reserves nothing.
//!
//! There is **no LLM path** anywhere in it (10b §4 rule 5, 08i rule 2): a pack is
//! authored source, a seed is deterministic, and no `WorldCommand::ModuleAsyncDone`
//! arm is added.
//!
//! # The dependency claim
//!
//! `tacsim -> {mush-core, mush-sim}` and the serde/tracing vocabulary, and
//! **nothing else** — no economy, org, party, faction, market or tactics edge.
//! `mush-sim` is the one substantive edge and it is [`mush_sim::RouteDef`]'s: 10b §3.1
//! put the type here and §3.6 made `mush-sim` its consumer, which is a cycle, so the
//! **type** lives in `mush-sim` beside `Leg` and this crate provides its **data** —
//! `mush-tactics/src/mobility.rs`'s stated pattern. `cargo tree -p tacsim` is
//! the test.
//!
//! # Where the bytes come from
//!
//! **The host reads; this crate parses** (10b §3.1 OPEN 1). `mush_server::config_ext`
//! reads each `pack_paths`/`seed_paths` entry and hands [`TacsimRuleset::with_config`]
//! `(path, text)` pairs, so an unreadable, oversized or mis-digested file **fails the
//! boot naming the file**, before any module runs. [`pack::load_pack`] is a pure
//! `bytes -> Result<RulesPack, PackError>` with no filesystem at all, which is what
//! makes every pack-identity test a unit test.
//!
//! E6c went the other way for its own catalogues — `ruleset-shadowrun`'s
//! `SrCatalogSource::pack` reads a file itself, at `@market/catalog load` time — and
//! that is the right answer *there*: a catalogue is an optional, staff-triggered load
//! whose failure must fail the pack and never the server. Cut 1's packs are a **boot
//! precondition**: 10b §3.1 invariant (iv) requires a malformed pack to fail closed at
//! boot naming the file, and only the host can fail a boot. The two answers differ on
//! purpose and each has its own test.

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(
    clippy::unwrap_used,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

/// 16b cut A: **the combat arena's core library** (doc 16 §3.1–§3.4, §4, §8).
///
/// A library with no input and no output: `arena::Arena` holds a match's state
/// and the modules beside it hold its clock, its inputs, its randomness, its
/// state hash and its streaming ledger. Cut T's arena task is the thin wrapper
/// that owns one; a contract gate
/// (`crates/mush-contract-tests/tests/arena_library.rs`) asserts that nothing
/// under `arena/` can reach persistence, a runtime, an output sink or the world.
pub mod arena;
pub mod commands;
pub mod composition;
pub mod config;
pub mod content;
pub mod economy;
pub mod ephemeris;
pub mod fixture;
pub mod pack;
pub mod record;
pub mod seed;
/// 16b cut R-b (R4): **the one game sheet** — pilot ratings, the character-first
/// facts of doc 10 §17–§19, `+sheet`/`+chargen`/`+stat`/`+roll` over
/// [`mush_ruleset`]'s schema library, and the single NPC `check` ability that
/// replaces `wod_roll` and `sr_roll`.
///
/// It is a **second** [`GameModule`] in this crate, named
/// [`sheet::NS`] (`"pilot"`) because this one is named [`NS`] (`"tacsim"`) and a
/// module's name is its persistence namespace; the sheet's *ruleset* id stays
/// `"tacsim"`, so storage and the wire are `sheet:tacsim` and `tacsim.sheet`.
/// [`sheet`]'s own docs carry the whole argument.
pub mod sheet;
pub mod store;

use mush_core::module::{GameModule, ModError, ModuleBootData, ModuleCtx, Registrar};

pub use config::EphemerisConfig;
pub use config::{
    DEFAULT_ACTIVE_DOMAIN_LIMIT, DEFAULT_GROUND_PULSE_MS, DEFAULT_SPACE_PULSE_MS, LoadedPack,
    MAX_CONFIGURED_PACKS, MAX_RECOVERY_TIERS, PackSource, RECOVERY_INSURANCE_WORDS, TacsimConfig,
    TacsimRecoveryConfig, TacsimTimeConfig,
};
pub use content::{CORRIDOR_MODE, GM_PER_KM, SITE_HALF_GM, register_appliers};
pub use economy::{CREDIT, CREDIT_CODE, CREDIT_SCALE, GameEconomyConfig};
pub use ephemeris::{AU_KM, Body, Ephemeris, MAX_ORBIT_UAU, MAX_PERIOD_MDAYS, separation_km};
pub use pack::BodyDef;
pub use pack::{
    CommodityDef, FacilityDef, FormationKindDef, FormationRoleDef, LocationDef,
    MAX_FACILITY_CAPABILITIES, MAX_FACILITY_SLOTS, MAX_FORMATION_CATEGORIES, MAX_FORMATION_MEMBERS,
    MAX_FORMATION_ROLES, MAX_HAZARD_BAND, MAX_LABEL_BYTES, MAX_LEG_SECS, MAX_PACK_BYTES,
    MAX_PACK_ENTRIES, MAX_RECIPE_LINES, MAX_RECIPE_UNITS, MAX_REQUIRES, MAX_ROUTE_PROPELLANT,
    MAX_SEED_OBJECTS, PACK_DIGEST_DOMAIN, PackEntry, PackError, PackKind, PackLeg, PackManifest,
    PackRequirement, RecipeDef, RecipeLine, RouteEntry, RulesPack, check_requires, kind_word,
    load_pack, pack_digest,
};
pub use record::{PackRecord, PinRecord, SeedApplication};
pub use seed::{PlannedObject, SeedError, SeedOutcome, SeedPlan};
// 16b cut R-b (R4). The sheet ruleset is deliberately NOT re-exported as
// `TacsimRuleset`: that name is this crate's packs-and-seed layer, and the two
// are different modules with different namespaces (see [`sheet`]).
pub use sheet::{
    PilotProfile, PilotRuleset, PilotSheetState, TacsimChargen, agent as sheet_agent,
    onboarding as sheet_onboarding,
};
pub use store::{MAX_PINS, NS, PACK_PAGE, ResolveError, TacsimStore};

/// **Parse and verify every pack the host read** — the one function both the host's
/// boot check and the module's own wiring call.
///
/// Pure: it takes bytes and returns packs or a message naming the file. Calling it
/// twice on one configuration cannot disagree, which is why the host may fail the boot
/// on it and the module may then rely on it.
///
/// # Errors
///
/// A message naming the **file**, for a pack that will not parse, whose declared
/// digest is not its content's, whose `requires` is unsatisfied, or that collides with
/// another file on `(id, version)`.
pub fn verify_sources(cfg: &TacsimConfig) -> Result<Vec<LoadedPack>, String> {
    use std::collections::BTreeMap;

    let configured = cfg.packs.len().saturating_add(cfg.seeds.len());
    if configured > MAX_CONFIGURED_PACKS {
        return Err(format!(
            "[tacsim]: {configured} pack files are configured; this server loads at most \
             {MAX_CONFIGURED_PACKS}"
        ));
    }
    let mut loaded: BTreeMap<(mush_core::CanonicalId, u32), LoadedPack> = BTreeMap::new();
    for source in cfg.sources() {
        let pack = pack::load_pack(&source.bytes)
            .map_err(|error| format!("[tacsim] {}: {error}", source.name))?;
        let key = (pack.manifest().id().clone(), pack.manifest().version());
        if let Some(previous) = loaded.get(&key) {
            return Err(format!(
                "[tacsim] {}: declares {}@{}, which {} already declares",
                source.name, key.0, key.1, previous.source
            ));
        }
        loaded.insert(
            key,
            LoadedPack {
                source: source.name.clone(),
                pack,
            },
        );
    }
    // `requires` is resolved only once every file has parsed, so the order of
    // `pack_paths` never decides whether a dependency is satisfied.
    let packs: BTreeMap<(mush_core::CanonicalId, u32), pack::RulesPack> = loaded
        .iter()
        .map(|(key, value)| (key.clone(), value.pack.clone()))
        .collect();
    for entry in loaded.values() {
        pack::check_requires(&entry.pack, &packs)
            .map_err(|error| format!("[tacsim] {}: {error}", entry.source))?;
    }
    // And the `requires` graph is acyclic (independent verify P8). `PackManifest::new`
    // refuses only self-dependency and duplication, so `t.a@1 requires t.b@1` and
    // `t.b@1 requires t.a@1` both loaded and both verified. The cycle is inert in cut 1
    // — nothing orders packs — but a later cut's load ordering needs acyclicity
    // guaranteed at boot rather than assumed, and the refusal names the cycle.
    if let Some(cycle) = requires_cycle(&packs) {
        return Err(format!("[tacsim]: a requires cycle: {cycle}"));
    }
    Ok(loaded.into_values().collect())
}

/// The first `requires` cycle, rendered as `a@1 -> b@1 -> a@1`, or `None`.
///
/// A depth-first walk over at most `MAX_CONFIGURED_PACKS` nodes with an explicit stack
/// — no recursion, no `?`, no panicking index — so a hostile graph costs one pass.
fn requires_cycle(
    packs: &std::collections::BTreeMap<(mush_core::CanonicalId, u32), pack::RulesPack>,
) -> Option<String> {
    use std::collections::BTreeSet;

    type Key = (mush_core::CanonicalId, u32);

    let render = |path: &[Key], back: &Key| -> String {
        let start = path.iter().position(|key| key == back).unwrap_or(0);
        let mut hops: Vec<String> = path
            .iter()
            .skip(start)
            .map(|(id, version)| format!("{id}@{version}"))
            .collect();
        hops.push(format!("{}@{}", back.0, back.1));
        hops.join(" -> ")
    };

    let mut settled: BTreeSet<Key> = BTreeSet::new();
    for root in packs.keys() {
        if settled.contains(root) {
            continue;
        }
        // `path` is the current walk; `frame` is how many of the root's requirements
        // each level has already spent.
        let mut path: Vec<Key> = vec![root.clone()];
        let mut frame: Vec<usize> = vec![0];
        let mut on_path: BTreeSet<Key> = BTreeSet::new();
        on_path.insert(root.clone());
        while let Some(depth) = path.len().checked_sub(1) {
            let Some(current) = path.get(depth) else {
                break;
            };
            let next = frame.get(depth).copied().unwrap_or(0);
            let requirement = packs
                .get(current)
                .and_then(|p| p.manifest().requires().get(next))
                .map(|r| (r.id.clone(), r.version));
            match requirement {
                Some(key) => {
                    if let Some(slot) = frame.get_mut(depth) {
                        *slot = next.saturating_add(1);
                    }
                    if on_path.contains(&key) {
                        return Some(render(&path, &key));
                    }
                    if settled.contains(&key) || !packs.contains_key(&key) {
                        continue;
                    }
                    on_path.insert(key.clone());
                    path.push(key);
                    frame.push(0);
                }
                None => {
                    if let Some(done) = path.pop() {
                        on_path.remove(&done);
                        settled.insert(done);
                    }
                    frame.pop();
                }
            }
        }
    }
    None
}

/// The TacSim layer (`"tacsim"`).
pub struct TacsimRuleset {
    cfg: TacsimConfig,
    /// The verified packs, or the host-facing message that names the failing file.
    loaded: Result<Vec<LoadedPack>, String>,
}

impl TacsimRuleset {
    /// The layer with an explicit configuration.
    ///
    /// Parses eagerly: `register` refuses the composition if the packs do not verify,
    /// which is defence in depth behind the host's own boot check. `enabled = false`
    /// off-config, so an unconfigured server is bit-identical to one built before this
    /// crate existed.
    #[must_use]
    pub fn with_config(cfg: TacsimConfig) -> TacsimRuleset {
        let loaded = verify_sources(&cfg);
        TacsimRuleset { cfg, loaded }
    }

    /// The layer, off.
    #[must_use]
    pub fn new() -> TacsimRuleset {
        TacsimRuleset::with_config(TacsimConfig::default())
    }

    /// The frozen `[tacsim]` block.
    #[must_use]
    pub fn config(&self) -> &TacsimConfig {
        &self.cfg
    }
}

impl Default for TacsimRuleset {
    fn default() -> TacsimRuleset {
        TacsimRuleset::new()
    }
}

impl GameModule for TacsimRuleset {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        // K8.5 module compatibility: every command this module registers is a bounded
        // whole-document post-image on the compatibility lane (10b §4.1's cut-1 row).
        // It must come before the first `reg.command`, or the production composition
        // refuses.
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        let packs = match &self.loaded {
            Ok(packs) => packs.clone(),
            Err(detail) => return Err(ModError::msg(detail.clone())),
        };
        // The state slot always exists, even disabled: the verbs answer "not enabled"
        // rather than vanishing (E5c deviation 1), which is also what lets the rule-1
        // allowlist tripwire see `@tacsim` in a default composition.
        let mut slot = TacsimStore {
            cfg: self.cfg.clone(),
            ..TacsimStore::default()
        };
        slot.install_packs(packs);
        reg.state(slot);
        commands::register(reg)?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        let packs = match &self.loaded {
            Ok(packs) => packs.clone(),
            Err(detail) => return Err(ModError::msg(detail.clone())),
        };
        let kv: Vec<(smol_str::SmolStr, serde_json::Value)> = data
            .kv
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        // Total: `load` skips what it cannot use and says so, so a corrupt document
        // never fails the boot (`store`'s table says what each loss costs).
        let mut loaded = store::load(&kv, self.cfg.clone());
        loaded.install_packs(packs);
        loaded.reindex();
        let Some(slot) = ctx.w.module_state_mut::<TacsimStore>() else {
            return Err(ModError::msg("tacsim: no module state"));
        };
        *slot = loaded;
        boot_check_placements(ctx);
        boot_reconcile(ctx)
    }
}

/// Reconcile every seed latch's placement map against the world (independent verify
/// S3).
///
/// `reindex_routes` resolves a route's symbolic endpoints purely against the latch's
/// `placed` map and has no `World` at all, so its only failure mode was a symbol
/// **missing from the map**; an object the map named that no longer exists passed
/// straight through and a `RouteDef` was handed out over a tombstone (`World::destroy`
/// plus `next_id` monotonicity make it a tombstone, never a reused id, so the damage
/// was a dangling reference rather than a mis-bound one — and cut 6 plans journeys
/// from exactly this projection).
///
/// Two passes, because the store lives inside the world: read the dead placements with
/// the world borrowed immutably, then record them with it borrowed mutably. Bounded by
/// the latches' own [`MAX_SEED_OBJECTS`] cap, with no `?`, `unwrap`, `expect` or
/// panicking index.
fn boot_check_placements(ctx: &mut ModuleCtx<'_>) {
    let mut missing: Vec<((mush_core::CanonicalId, u32), mush_core::CanonicalId)> = Vec::new();
    if let Some(slot) = ctx.w.module_state::<TacsimStore>() {
        for (key, seed) in slot.latches() {
            for (symbol, object) in seed.placed() {
                if ctx.w.get(*object).is_none() {
                    missing.push((key.clone(), symbol.clone()));
                }
            }
        }
    }
    if missing.is_empty() {
        return;
    }
    // 08c §4.6's ladder, level 2: the exact command, the failed invariant and **the
    // subject only** — this pack's projection is stopped, nothing else is.
    let mut paged: std::collections::BTreeMap<(mush_core::CanonicalId, u32), usize> =
        std::collections::BTreeMap::new();
    for (key, _) in &missing {
        *paged.entry(key.clone()).or_insert(0) += 1;
    }
    for ((pack, version), gone) in &paged {
        tracing::error!(
            %pack,
            version,
            gone,
            "tacsim: a seed latch names {gone} object(s) this world no longer has; every route \
             and placement through them is skipped rather than resolved over a tombstone. \
             Recommended: `@tacsim/pack show {pack}@{version}`, then re-create them by hand or \
             publish a corrected version. Only this pack is stopped."
        );
    }
    if let Some(slot) = ctx.w.module_state_mut::<TacsimStore>() {
        slot.record_missing_placements(missing);
    }
}

/// Publish a ledger row for every loaded pack version this world has not recorded —
/// and **refuse the boot** for one it has recorded under other bytes.
///
/// The row is what makes *retirement* detectable later: a version whose row is stored
/// but whose file is gone has been withdrawn, and §3.1 invariant (i) then refuses to
/// float an unpinned consumer of it onto a newer version.
///
/// # The digest gate (independent verify P1; owner decision, 2026-09-05)
///
/// Until this pass compared digests, the row's own `digest` was **written and never
/// read**: the skip was a bare key test, so an operator who edited a pack in place
/// under the same `(id, version)` and re-signed it had it adopted at the next boot with
/// no refusal, no warning and no ledger correction — the exact mistake invariant (i)
/// exists to catch, and with a seeded world it moved a published corridor's transit
/// times underneath a hauling contract while the latch barred the only re-seed that
/// could reconcile it. So: **on agreement, continue as before; on disagreement, refuse
/// the boot naming the file and both digests**, which is invariant (iv)'s direction and
/// 10b §4 rule 2's rule — a re-published version is a *typo in a file the operator can
/// fix*, not one row of accumulated history, and `mushd.toml.example` already promises
/// exactly this answer for pack files. Never page-and-drop, never adopt; a **new**
/// `(id, version)` with a new digest is the normal path.
///
/// The "already published" test reads `published_keys`, not `published`, so a ledger
/// row that will not parse cannot re-publish its version under new bytes; the repair
/// from the loaded pack is **loud** (level 2, naming the pack) rather than silent.
///
/// # Errors
///
/// [`ModError`] naming the file, the pack and both digests. No `?`, `unwrap`, `expect`
/// or panicking index — it runs at boot, where a panic takes the world down, and the
/// refusal is the deliberate fail-closed answer rather than an escaped error.
fn boot_reconcile(ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
    if !ctx
        .w
        .module_state::<TacsimStore>()
        .is_some_and(TacsimStore::enabled)
    {
        return Ok(());
    }
    let first_seen = ctx.world_ic_now();
    let mut rows: Vec<PackRecord> = Vec::new();
    let mut repaired: Vec<(mush_core::CanonicalId, u32)> = Vec::new();
    if let Some(slot) = ctx.w.module_state::<TacsimStore>() {
        for loaded in slot.packs() {
            let manifest = loaded.pack.manifest();
            let key = (manifest.id().clone(), manifest.version());
            // The ledger row, when this world can read one, is compared by DIGEST.
            if let Some(row) = slot.published.get(&key) {
                if row.digest() == manifest.digest() {
                    continue;
                }
                return Err(ModError::msg(format!(
                    "[tacsim] {}: {}@{} was published to this world at digest {}, but this file's \
                     content is {}. A published version is never re-signed in place — publish a \
                     new version instead; versions coexist.",
                    loaded.source,
                    key.0,
                    key.1,
                    row.digest().to_hex(),
                    manifest.digest().to_hex()
                )));
            }
            // The KEY is the claim, so a row that would not parse still counts as
            // published: it is repaired from the loaded pack, out loud.
            if slot.published_keys.contains(&key) {
                repaired.push(key.clone());
            }
            let entries = match u32::try_from(loaded.pack.entries().len()) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            match PackRecord::new(
                manifest.id().clone(),
                manifest.version(),
                manifest.kind(),
                manifest.digest(),
                entries,
                first_seen.clone(),
            ) {
                Ok(record) => rows.push(record),
                Err(error) => tracing::warn!(
                    pack = %manifest.id(),
                    version = manifest.version(),
                    %error,
                    "tacsim: a loaded pack could not be recorded in the ledger"
                ),
            }
        }
    }
    for (pack, version) in &repaired {
        // 08c §4.6's ladder, level 2: the exact command, what was lost, and the subject
        // only. The row's stored digest is gone with the body, so the repair adopts the
        // loaded pack's — which is why it is said out loud rather than done in silence.
        tracing::error!(
            %pack,
            version,
            "tacsim: the ledger row for {pack}@{version} would not parse and has been repaired \
             from the loaded pack, so the digest this version was FIRST published under is no \
             longer known to this world. Recommended: `@tacsim/pack verify {pack}@{version}` and \
             compare it against the file you meant to ship. Only this pack is affected."
        );
    }
    let published = !rows.is_empty();
    for row in rows {
        store::put_pack(ctx, row);
    }
    if published {
        store::persist_meta(ctx);
    }
    Ok(())
}

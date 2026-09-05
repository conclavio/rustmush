//! `ruleset-tacsim` — **TacSim's packs, its seed latch and the corridor fixture**
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
//! `ruleset-tacsim -> {mush-core, mush-sim}` and the serde/tracing vocabulary, and
//! **nothing else** — no economy, org, party, faction, market or tactics edge.
//! `mush-sim` is the one substantive edge and it is [`mush_sim::RouteDef`]'s: 10b §3.1
//! put the type here and §3.6 made `mush-sim` its consumer, which is a cycle, so the
//! **type** lives in `mush-sim` beside `Leg` and this crate provides its **data** —
//! `mush-tactics/src/mobility.rs`'s stated pattern. `cargo tree -p ruleset-tacsim` is
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

pub mod commands;
pub mod config;
pub mod fixture;
pub mod pack;
pub mod record;
pub mod seed;
pub mod store;

use mush_core::module::{GameModule, ModError, ModuleBootData, ModuleCtx, Registrar};

pub use config::{
    DEFAULT_ACTIVE_DOMAIN_LIMIT, DEFAULT_GROUND_PULSE_MS, DEFAULT_SPACE_PULSE_MS, LoadedPack,
    MAX_CONFIGURED_PACKS, PackSource, TacsimConfig, TacsimTimeConfig,
};
pub use pack::{
    CommodityDef, LocationDef, MAX_PACK_BYTES, MAX_PACK_ENTRIES, MAX_REQUIRES, MAX_SEED_OBJECTS,
    PACK_DIGEST_DOMAIN, PackEntry, PackError, PackKind, PackLeg, PackManifest, PackRequirement,
    RecipeDef, RecipeLine, RouteEntry, RulesPack, check_requires, load_pack, pack_digest,
};
pub use record::{PackRecord, PinRecord, SeedApplication};
pub use seed::{PlannedObject, SeedError, SeedPlan};
pub use store::{NS, ResolveError, TacsimStore};

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
    Ok(loaded.into_values().collect())
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
        boot_reconcile(ctx);
        Ok(())
    }
}

/// Publish a ledger row for every loaded pack version this world has not recorded.
///
/// The row is what makes *retirement* detectable later: a version whose row is stored
/// but whose file is gone has been withdrawn, and §3.1 invariant (i) then refuses to
/// float an unpinned consumer of it onto a newer version. Exhaustive, with no `?`,
/// `unwrap`, `expect` or panicking index — it runs at boot, where a panic takes the
/// world down.
fn boot_reconcile(ctx: &mut ModuleCtx<'_>) {
    if !ctx
        .w
        .module_state::<TacsimStore>()
        .is_some_and(TacsimStore::enabled)
    {
        return;
    }
    let first_seen = ctx.world_ic_now();
    let mut rows: Vec<PackRecord> = Vec::new();
    if let Some(slot) = ctx.w.module_state::<TacsimStore>() {
        for loaded in slot.packs() {
            let manifest = loaded.pack.manifest();
            let key = (manifest.id().clone(), manifest.version());
            if slot.published.contains_key(&key) {
                continue;
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
    let published = !rows.is_empty();
    for row in rows {
        store::put_pack(ctx, row);
    }
    if published {
        store::persist_meta(ctx);
    }
}

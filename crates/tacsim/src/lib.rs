//! `tacsim` — **TacSim's packs, its seed latch and the corridor fixture**
//! (E7 cut 1; `docs/design/10b-e7-implementation-plan.md` §3.1).
//!
//! # The pilot sheet and the simulation layer
//!
//! This crate owns the one game's pilot sheet and its simulation layer. The
//! host composes the sheet unconditionally through [`composition::sheet_modules`],
//! then the [`GameModule`] switched by `[tacsim].enabled`. The layer can read the
//! sheet's facts and the sheet can read the simulation's environmental facts
//! through the generic seam; the boot-order test pins that order.
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
/// spatial index, its motion, its state hash, its streaming ledger and the pulse
/// that drives them — `Arena::step(wall_now_ms, controls, intents)`, a pure
/// function of the state and those inputs. Cut T's arena task is the thin wrapper
/// that owns one; a contract gate
/// (`crates/mush-contract-tests/tests/arena_library.rs`) asserts that nothing
/// under `arena/` can reach persistence, a runtime, an output sink or the world.
pub mod arena;
/// 16b cut T.2: **the world's half of the arena seam** (doc 16 §3.1 "The seam is
/// seven messages", §3.7, §4, §5).
///
/// The registry of running matches, the three routing functions the cockpit verbs
/// call ([`arena_bridge::forward_intent`], [`arena_bridge::send_control`],
/// [`arena_bridge::send_stop`]), and the handling of everything the arena sends
/// back: frames as keyframes on the `arena.cockpit` feed, chunks appended to
/// `log:match.<id>` on the world's own writer lane, the lag notice and the end.
pub mod arena_bridge;
/// 16b cut T.2: **one running match, as a task** (doc 16 §3.1 "The arena core is
/// a library", "Failure").
///
/// The thin wrapper around an [`arena::Arena`]: a bounded inbox, a bounded
/// egress, a timer driven by `next_due_ms`, a watchdog at `tempo.stall_ms`, the
/// unwind boundary and a stop signal. It is the one place in the crate where the
/// arena meets a runtime, and it sits *beside* [`arena`] rather than inside it so
/// that directory's no-I/O gate can stay strict.
pub mod arena_task;
pub mod commands;
pub mod composition;
pub mod config;
pub mod content;
/// 16b cut C.2 (C1, C3, C4): **the game's typed definitions and the content it
/// ships** (doc 16 §3.3, §3.5, §3.6, §4, §7).
///
/// Six new [`pack::PackEntry`] kinds with strict decoders, the one function that
/// binds an authored unit to M32's own instantiation shape and mints every
/// `DefinitionRef` the C1 registry will carry, the arena's window built from the
/// atlas's records, and the nine content files compiled in beside them.
pub mod definitions;
/// **The deliberation layer** (16b cut N.2; doc 16 §6 "Deliberation"): the order
/// type a squad is steered by, the squad grouping the order is written for, the
/// structured request that produces one, and the cost table the owner's cost run
/// prints. The arena imports only [`deliberation::order`], which is data.
pub mod deliberation;
pub mod economy;
pub mod ephemeris;
pub mod fixture;
pub mod hardware;
pub mod ic_return;
/// Doc 19 cut A.1: **the Kepler propagation** behind the moving sky — element
/// tables, the calendar's Julian Day, the two-body solver (D-19-2 … D-19-4).
pub mod kepler;
pub mod launch;
/// 16b cut T.3: **the match** — doc 16 §3.8's lifecycle, the persisted match-id
/// counter, the RAM record of a match while it runs, the two custody transitions
/// (`Boarding → Running` and the one check-in at `Ended | Aborted | Void`), the
/// result document of doc 16 §4, and the staff `@arena` switches that drive all
/// of it until cut K's cockpit exists.
///
/// Spelled with the underscore because `match` is a keyword; every name inside
/// is spelled properly.
#[path = "match_/mod.rs"]
pub mod match_;
/// 17b cut M2.3: **the TacSim API for unit minds** (doc 17 §3, §4, §7, §19) —
/// the view, card and row types a scripted mind reads, the think context, the
/// station functions, the kernels and their fees, registered on `mush-mind`'s
/// sandbox ([`minds::standard_sandbox`]). Outside [`arena`], whose contract
/// test refuses the interpreter's name there; the arena reaches a mind only
/// through its [`arena::mind::MindRunner`] seam.
pub mod minds;
pub mod pack;
pub mod port;
pub mod port_tick;
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
/// Doc 18 cut IC2: **the sky seam** — the one query every distance asks
/// (doc 18 §4.8, D-18-43, D-18-48).
pub mod sky;
pub mod store;
/// **Doc 20 cut SM.1** — surface mining's ground: the keyed geology function, the reading
/// rule, the geology key's boot and pin, the site ground cache and `@geology`.
pub mod surface;
/// Bounded Safe Conduct underwriting terms; owner services opt in separately.
pub mod underwriting;

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
pub use kepler::{ElementRow, MAX_ELEMENT_ROWS, ROW_FIELDS, julian_day};
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
pub use sky::{KeplerSky, PosKm, Sky, SkyError, epoch_date};
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
    verify_sources_with(cfg, &[])
}

/// **[`verify_sources`] over the game's own seed sources too** (doc 18 cut IC2;
/// lead ruling R2): `game` — the lane, from
/// [`definitions::content::game_seed_sources`], when the composition root switches
/// the game content on — is read **first**, then the operator's sources, so an
/// operator file colliding with a game pack on `(id, version)` is the one the
/// collision message names as the offender. [`MAX_CONFIGURED_PACKS`] counts the
/// operator's sources only (the game's are not configured); the collision,
/// `requires` and cycle checks run over all of them; the returned list is keyed by
/// `(id, version)`, so the reading order changes nothing else.
///
/// # Errors
///
/// As [`verify_sources`].
pub fn verify_sources_with(
    cfg: &TacsimConfig,
    game: &[PackSource],
) -> Result<Vec<LoadedPack>, String> {
    use std::collections::BTreeMap;

    let configured = cfg.packs.len().saturating_add(cfg.seeds.len());
    if configured > MAX_CONFIGURED_PACKS {
        return Err(format!(
            "[tacsim]: {configured} pack files are configured; this server loads at most \
             {MAX_CONFIGURED_PACKS}"
        ));
    }
    let mut loaded: BTreeMap<(mush_core::CanonicalId, u32), LoadedPack> = BTreeMap::new();
    for source in game.iter().chain(cfg.sources()) {
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
    governed_hardware: Option<hardware::Registration>,
    launch: Option<launch::Registration>,
    ic_minds: Option<minds::ic_sortie::registration::Registration>,
    coverage: Option<underwriting::registration::Registration>,
    coverage_families: Option<underwriting::families::Families>,
    registry_commerce: Option<minds::registry::composition::Registration>,
    conduct_trials: Option<minds::registry::conduct::composition::Registration>,
    fork_material: Option<mush_core::DefinitionRef>,
    marked_registry: Option<mush_core::DefinitionRef>,
    original_holdings: Option<mush_core::DefinitionRef>,
    personal_filed_holdings: bool,
    personal_source_forms: bool,
    dependent_owner_holdings: bool,
    entitlement_holdings: bool,
    house_holdings: Option<mush_core::DefinitionRef>,
    cfg: TacsimConfig,
    /// The verified packs, or the host-facing message that names the failing file.
    loaded: Result<Vec<LoadedPack>, String>,
    /// **Doc 18 cut IC3 (§4.7; lead ruling L15)** — whether this layer runs in an
    /// in-process test engine: false in every constructor, set only by
    /// [`TacsimRuleset::with_test_world`] (through
    /// `composition::tacsim_test_world_module`), handed to the slot as
    /// `TacsimStore::test_world`. RAM only, never a config key. It relaxes the
    /// readiness report's world rate and registers the port tick's probe (doc 18
    /// cut IC5).
    test_world: bool,
    /// **Doc 18 cut IC5 (D-18-33; lead ruling L2)** — `mush-inventory`'s handle
    /// cell, handed over by the composition root that also names `tacsim` a
    /// quantity delegate (`composition::server_modules_with_tacsim`). `None` on a
    /// roster that did not: the port tick's every pool read then refuses.
    inventory: Option<mush_inventory::InventoryHandleCell>,
    /// **Doc 18 cut IC6** — the compiled-in goods table, parsed once, here,
    /// and handed to the slot at `register` and again at `on_world_loaded`
    /// (which rebuilds the slot from the store). RAM only, never persisted;
    /// `None` only on a build fault, and every gauge then reads dark.
    goods: Option<std::sync::Arc<crate::definitions::GoodsTable>>,
}

pub use minds::ic_sortie::{
    CloseEvent as IcCloseEvent, IcHost, IcHostCompletion, IcHostTicket, IcRetainedExecution,
    IcSubmissionState, IcUnadmittedOpen, StartPermit as IcStartPermit,
};
impl TacsimRuleset {
    /// Add the real unarmed proving carrier before capturing Registry or
    /// hardware catalogue references. This loads content, not qualification.
    pub fn with_conduct_carrier(mut self) -> Result<Self, ModError> {
        if self.governed_hardware.is_some()
            || self.registry_commerce.is_some()
            || self.conduct_trials.is_some()
        {
            return Err(ModError::msg(
                "Conduct carrier must precede catalogue capture",
            ));
        }
        let carrier = minds::registry::conduct::composition::carrier_pack()?;
        let packs = self.loaded.as_mut().map_err(|e| ModError::msg(e.clone()))?;
        if packs
            .iter()
            .any(|p| p.pack.manifest().id() == carrier.pack.manifest().id())
        {
            return Err(ModError::msg("Conduct carrier already configured"));
        }
        packs.push(carrier);
        self.definition_catalog()?;
        Ok(self)
    }

    /// Explicit actual owner composition; an absent region transition producer
    /// still cannot mint a withheld departure or charge any premium.
    pub fn with_hull_coverage(
        mut self,
        config: underwriting::CoverageConfiguration,
    ) -> Result<Self, ModError> {
        if self.coverage.is_some() {
            return Err(ModError::msg("coverage already configured"));
        }
        self.coverage = Some(config.layer(&self)?);
        Ok(self)
    }
    pub fn coverage_host(&self) -> Option<underwriting::CoverageHost> {
        self.coverage.as_ref().map(|r| r.host.clone())
    }

    /// The actual host retains opaque transition capabilities. This handle
    /// creates no start/event and ordinary compositions return None.
    pub fn ic_host(&self) -> Option<IcHost> {
        self.ic_minds.as_ref().map(|r| r.host.clone())
    }

    /// The layer with an explicit configuration.
    ///
    /// Parses eagerly: `register` refuses the composition if the packs do not verify,
    /// which is defence in depth behind the host's own boot check. `enabled = false`
    /// off-config, so an unconfigured server is bit-identical to one built before this
    /// crate existed.
    #[must_use]
    pub fn with_config(cfg: TacsimConfig) -> TacsimRuleset {
        TacsimRuleset::with_game_content(cfg, false)
    }

    /// **The layer with the game's own seed packs** (doc 18 cut IC2; lead ruling
    /// R2): `game_content` true loads
    /// [`definitions::content::game_seed_sources`] (the lane) ahead of the
    /// operator's packs, so the store holds the lane whose places the content
    /// latch applies. The composition root passes the same boolean it hands
    /// `content_module_with`, so the loaded packs and the applied boot list move
    /// together.
    #[must_use]
    pub fn with_game_content(cfg: TacsimConfig, game_content: bool) -> TacsimRuleset {
        let game = if game_content {
            definitions::content::game_seed_sources()
        } else {
            Vec::new()
        };
        let loaded = verify_sources_with(&cfg, &game);
        // Doc 18 cut IC6: the goods table is parsed once, here (every other
        // constructor comes through this one); `tests/content.rs` pins it, so
        // an error is a build fault.
        let goods = match crate::definitions::goods::goods_table() {
            Ok(table) => Some(std::sync::Arc::new(table)),
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "tacsim: the compiled-in goods table does not load; the ship gauges will read \
                     dark"
                );
                None
            }
        };
        TacsimRuleset {
            governed_hardware: None,
            launch: None,
            ic_minds: None,
            coverage: None,
            coverage_families: None,
            registry_commerce: None,
            conduct_trials: None,
            fork_material: None,
            marked_registry: None,
            original_holdings: None,
            personal_filed_holdings: false,
            personal_source_forms: false,
            dependent_owner_holdings: false,
            entitlement_holdings: false,
            house_holdings: None,
            cfg,
            loaded,
            test_world: false,
            inventory: None,
            goods,
        }
    }

    /// **This layer as a test world's** (doc 18 cut IC3; lead ruling L15): the
    /// readiness report then accepts a world rate other than 24, and registers the
    /// port tick's probe (doc 18 cut IC5). The production roster
    /// (`mush_server::boot::game_modules`) never calls it.
    #[must_use]
    pub fn with_test_world(mut self, on: bool) -> TacsimRuleset {
        self.test_world = on;
        self
    }

    /// **The inventory handle cell** (doc 18 cut IC5; lead ruling L2).
    ///
    /// Composition-root data, like `IndustryModule::with_inventory`: a capability;
    /// the authority is the delegation in `server_modules_with_tacsim`.
    #[must_use]
    pub fn with_inventory(mut self, cell: mush_inventory::InventoryHandleCell) -> TacsimRuleset {
        self.inventory = Some(cell);
        self
    }

    /// Enable the original-owner reader against the actual registered combat
    /// manifest supplied by composition. Ordinary constructors leave this off.
    /// Registry/Roll registration must independently declare this exact ref.
    #[must_use]
    pub fn with_original_holdings(mut self, manifest: mush_core::DefinitionRef) -> Self {
        self.original_holdings = Some(manifest);
        self.personal_filed_holdings = false;
        self.personal_source_forms = false;
        self.dependent_owner_holdings = false;
        self.entitlement_holdings = false;
        self
    }

    /// Explicit OOC composition for current original personal Sealed Pin paid
    /// holdings. No default activation, source display, IC seat or cash flow.
    #[must_use]
    pub fn with_personal_filed_holdings(mut self, manifest: mush_core::DefinitionRef) -> Self {
        self.original_holdings = Some(manifest);
        self.personal_filed_holdings = true;
        self.personal_source_forms = false;
        self.dependent_owner_holdings = false;
        self.entitlement_holdings = false;
        self
    }

    /// Explicit v2 original-personal Pin execution and current paid source
    /// reader. Actual Roll composition must install the exact versioned services.
    /// No derivative access, grant, or source permission is created here.
    #[must_use]
    pub fn with_personal_filed_source_forms(mut self, manifest: mush_core::DefinitionRef) -> Self {
        self.original_holdings = Some(manifest);
        self.personal_filed_holdings = true;
        self.personal_source_forms = true;
        self.dependent_owner_holdings = false;
        self.entitlement_holdings = false;
        self
    }
    /// Explicit dependent owner use/source; existing paid eligibility remains
    /// based only on the paid holder's own acknowledged term and status.
    pub fn with_dependent_owner_holdings(self, manifest: mush_core::DefinitionRef) -> Self {
        let mut layer = self.with_personal_filed_source_forms(manifest);
        layer.dependent_owner_holdings = true;
        layer
    }
    /// Select the distinct current-customer reader; no licence is created.
    pub fn with_mind_entitlement_holdings(self, manifest: mush_core::DefinitionRef) -> Self {
        let mut layer = self.with_dependent_owner_holdings(manifest);
        layer.entitlement_holdings = true;
        layer
    }

    /// Add the distinct sealed House customer route. Existing personal owner
    /// and paid routes retain their own exact family and eligibility contracts.
    pub fn with_house_mind_holdings(mut self, manifest: mush_core::DefinitionRef) -> Self {
        self.house_holdings = Some(manifest);
        self
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

    /// The exact composition-selected catalogue used by both mechanics and
    /// Registry preparation. Ordinary layers retain the existing fielded set.
    pub(crate) fn definition_catalog(&self) -> Result<definitions::Catalog, ModError> {
        match &self.governed_hardware {
            Some(hardware) => Ok(hardware.family.catalog().clone()),
            None => definitions::Catalog::build_fielded(&self.loaded_packs()?, NS)
                .map_err(|e| ModError::msg(e.to_string())),
        }
    }

    /// The verified packs, or the message naming the failing file as a
    /// composition refusal (16b cut C.2).
    fn loaded_packs(&self) -> Result<Vec<LoadedPack>, ModError> {
        match &self.loaded {
            Ok(packs) => Ok(packs.clone()),
            Err(detail) => Err(ModError::msg(detail.clone())),
        }
    }
}

/// **The game's definitions, bound from the shipped content alone** (16b cut
/// C.2).
///
/// What the composition root hands `TacticsModule::with_catalog` so
/// `@unit/create <obj>=tacsim.unit.line_tank` resolves in production. It is the
/// *built-in* content and not `[tacsim].pack_paths`' because
/// `composition::server_modules_with` is not handed the `[tacsim]` block —
/// the tactics module is composed several arguments before the layer that owns
/// it — and because the built-in content is the game's, which is the catalog
/// `@unit/create` is for. An operator pack's unit definitions still register in
/// the C1 registry (this module's `register` binds every loaded pack); what they
/// do not do is reach the staff verb's catalog. When cut K needs them to, the
/// argument goes down the composition root and this function takes it.
///
/// **16b cut Q.8** made it [`definitions::Catalog::build_fielded`]: the shipped
/// content *and* doc 16 §9's five extension items, which is what makes
/// `@unit/create #3=tacsim.unit.biped_mk1` and `SIM UNIT biped_mk1` resolve on a
/// shipped server. `build_game` is still the stock game and is still what every
/// cut-X acceptance measures against.
///
/// # Errors
///
/// A message naming the entry when the shipped content does not bind — a build
/// error, since `crates/tacsim/tests/content.rs` pins it.
pub fn builtin_definitions() -> Result<definitions::Catalog, String> {
    definitions::Catalog::build_fielded(&[], NS).map_err(|error| error.to_string())
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
            test_world: self.test_world,
            goods: self.goods.clone(),
            ..TacsimStore::default()
        };
        slot.install_packs(packs);
        reg.state(slot);
        self.coverage_families = Some(underwriting::registration::register_base(reg)?);
        // Doc 20 cut SM.1 (§3.2, Q-20-12): the geology layer's RAM slot — the key the host hands
        // in after composition (`surface::install_geology`), the status the boot pass decides
        // once, the bodies' tables and the site ground cache. Never persisted.
        reg.state(surface::GeologyState::default());
        reg.state(
            match_::mind_tape::TapeStore::new(self.cfg.arena.tape_retained_max_bytes)
                .map_err(|e| ModError::msg(e.to_string()))?,
        );
        // **17b cut M5.1** (doc 17 §15, D-UM-49; OQ-2): the holdings seam, on
        // its own slot, with the production provider — every character holds
        // Standard Doctrine and its six presets. A test rig installs another
        // through `match_::holdings::install_holdings`.
        reg.state(match_::holdings::HoldingsSeam::production());
        if let Some(manifest) = &self.original_holdings {
            if self.entitlement_holdings {
                match_::original_access::register_entitlements(reg, manifest)?;
                minds::workshop::filed_source::register_entitlements(reg, manifest)?;
                minds::workshop::filed_source::register_dependent(reg, manifest)?;
            } else if self.dependent_owner_holdings {
                match_::original_access::register_dependent_owners(reg, manifest)?;
                minds::workshop::filed_source::register(reg, manifest)?;
                minds::workshop::filed_source::register_dependent(reg, manifest)?;
            } else if self.personal_source_forms {
                match_::original_access::register_personal_source_forms(reg, manifest)?;
                minds::workshop::filed_source::register(reg, manifest)?;
            } else if self.personal_filed_holdings {
                match_::original_access::register_personal(reg, manifest)?;
            } else {
                match_::original_access::register(reg, manifest)?;
            }
        }
        if let Some(manifest) = &self.house_holdings {
            if self
                .original_holdings
                .as_ref()
                .is_some_and(|old| old != manifest)
            {
                return Err(ModError::msg("House and personal holding manifests differ"));
            }
            match_::original_access::register_house(reg, manifest)?;
        }
        // Doc 18 cut IC4 (lead ruling L14): hull rooms are not atlas rooms, so
        // their details live on tacsim's own attribute shelf, and `look
        // <detail>` finds them through the registered prefix (16b cut D.5's
        // prefix rule: a module points `look` only at a namespace it owns).
        reg.attr_namespace("tacsim")?;
        reg.detail_prefix(crate::port::hull::HULL_DETAIL_PREFIX)?;
        // Doc 18 cut IC14 row 48 (lead ruling L28): `@tacsim/seed <pack> commit`
        // drives mush_content::apply_pack under this module's identity, so the
        // content latch row and the atlas records the pack's entries make carry
        // `tacsim` on a namespace that is not ours. The reproduction
        // (`tests/lane.rs::ic14_a_staff_seed_survives_a_fold_and_reboot`) showed
        // exactly these two pairs refused; both are declared here, by the writer,
        // and pinned in `crates/mush-contract-tests/module-ns-owners.toml`.
        reg.persist_delegation(
            NS,
            mush_content::NS,
            "seed::apply drives mush_content::apply_pack for `@tacsim/seed <pack> commit` \
             under the tacsim module's identity, so the content latch row it commits carries \
             this module's identity (doc 18 cut IC14 row 48)",
        )?;
        reg.persist_delegation(
            NS,
            mush_atlas::NS,
            "seed::apply drives mush_content::apply_pack for `@tacsim/seed <pack> commit` \
             under the tacsim module's identity, and the atlas appliers it runs write the \
             site, connection and room records the pack describes (doc 18 cut IC14 row 48)",
        )?;
        commands::register(reg)?;
        // Doc 19 cut A.3b: the `sky.chart` panel feed and its query, with the port verbs.
        port::chart_panel::register(reg)?;
        // Doc 20 cut SM.1 (§3.2, D-20-4): `@geology`, staff truth, in its own ReadOnly bracket.
        surface::commands::register(reg)?;
        // 16b cut C.2 (C1): the game's definitions, through the C1 registry.
        // One validator per kind first — `Registrar::definition` admits nothing
        // whose kind has none — then the descriptors in dependency order.
        // Unconditional, like everything else this module registers: a
        // definition is a fact about the shipped content, not about whether an
        // operator turned the layer on, and `@unit/create` must be able to name
        // the stock tank on any composition that carries this crate.
        // **16b cut Q.8**: `build_fielded`, so the five extension items are
        // registered through the same C1 route as the stock eight and a
        // persisted unit may point at one.
        let catalog = self.definition_catalog()?;
        for kind in definitions::KINDS {
            let of_kind: Vec<_> = catalog
                .descriptors
                .iter()
                .filter(|descriptor| descriptor.kind().as_str() == kind)
                .cloned()
                .collect();
            reg.definition_validator(
                mush_core::mechanics::DefinitionKind::new(kind)
                    .map_err(|error| ModError::msg(error.to_string()))?,
                definitions::exact_validator(of_kind),
            )?;
        }
        for descriptor in &catalog.descriptors {
            reg.definition(descriptor.clone())?;
        }
        if let Some(launch) = &self.launch {
            launch.register(reg)?;
        }
        if let Some(hardware) = &self.governed_hardware {
            hardware.register(reg)?;
        }
        hardware::PublishedHardware::shipped()
            .map_err(|error| ModError::msg(error.to_string()))?
            .register(reg)?;
        // 16b cut T.2: the arena's world side — the `arena.cockpit` and
        // `arena.map` feeds (16b cut Q.3 named the first for doc 16 and added the
        // second), the
        // egress re-entry handler, the two seat facts the kernel publishes, the
        // `ArenaRegistry` slot and `@arena`. Registered unconditionally, for the
        // same reason the pack verbs are (E5c deviation 1): a disabled layer
        // refuses out loud rather than hiding a verb, and the registry's presence
        // is what lets the host find the shared task table at all.
        arena_bridge::register(
            reg,
            arena_bridge::egress_capacity(&self.cfg.arena),
            &self.cfg.arena,
            &self.cfg.match_,
            &self.cfg.sensors,
            &self.cfg.minds,
        )?;
        // 16b cut K.1: the lobby — `SIM` and the coarse tick that closes
        // boarding. Registered unconditionally beside the rest, for the same
        // reason: a disabled layer refuses out loud rather than hiding a verb,
        // and the centre is content, not configuration.
        //
        // **16b cut F.4 made that reason true of these verbs.** They registered
        // unconditionally and then checked nothing, so with the layer off two
        // players could claim slots in a centre with no arena behind it and only
        // `SIM START` refused, naming a repair no verb can perform. Every lobby
        // verb and `BOARD` now asks `lobby::require_layer` first.
        match_::sim::register(reg)?;
        // 16b cut K.2: the cockpit — `BOARD`, `EXIT`, doc 16 §3.7's nine
        // selectors and the gate that makes all eleven contextual **inside the
        // centre** (outside it, cut F.4 has the gate stand aside so ordinary
        // exits and aliases still work). Beside the lobby and for the same
        // reason.
        match_::cockpit::register(reg)?;
        // 16b cut AH.1 (16c N18): the first-time hints — `@tutorial` and the
        // lobby's `Moved` observer, under the `LegacyPostImage` classification
        // the cockpit hands back. Unconditional like its neighbours: the verb
        // reads and throws its account's switch whether or not the layer is on,
        // and `tutorial::hint` itself refuses to say a line while it is off.
        match_::tutorial::register(reg)?;
        // Doc 18 cut IC1 (§4.5, D-18-3): the IC-return rule — `+ic`, the door's
        // `MoveAttempt` veto and the last-port stamp. Unconditional like its
        // neighbours; `+ic` itself refuses while the layer is off.
        ic_return::register(reg)?;
        // Doc 18 cut IC5 (§4.3, §4.4, D-18-32, D-18-33): the port tick — the two
        // typed families, the crate's one `action` and one `outcome` validator,
        // `PortHandles`, the `tacsim.port` tick and the reconnect notices; in a
        // test world also the probe. Registered unconditionally like its
        // neighbours: the tick returns at once while the layer is off.
        // Doc 18 cut IC9: the departure burn's provider holds the drive table built
        // here from the fielded catalog and the layer's goods table (D-18-11).
        port_tick::register_with_coverage(
            reg,
            self.test_world,
            self.inventory.as_ref(),
            port::burn::burn_table(&catalog, self.goods.clone()),
            self.ic_minds.as_ref(),
            self.coverage.as_ref(),
        )?;
        // Doc 18 cut IC6 (§4.2, 18a §3): the hull rooms' status line and
        // gauges. Registered unconditionally like its neighbours; the hook
        // itself returns nothing while the layer is disabled.
        port::ship::register_hooks(reg)?;
        // Doc 18 cut IC8 (§4.3 "Stock and capital"; 18a C07/C08/P07/P08/P09/P11):
        // the boards. Registered unconditionally; silent while the layer is off.
        port::board::register_hooks(reg)?;
        if let Some(registry) = &self.registry_commerce {
            registry.register_with_conduct(
                reg,
                &self.cfg,
                &catalog,
                self.ic_minds.is_some(),
                self.coverage.is_some(),
                self.conduct_trials.is_some(),
            )?;
            if let Some(conduct) = &self.conduct_trials {
                conduct.register(reg, &self.cfg, &catalog)?;
            }
            if let Some(ic) = &self.ic_minds {
                ic.register(reg, &self.cfg, registry.reference())?;
                if let Some(coverage) = &self.coverage {
                    coverage.register(
                        reg,
                        self.coverage_families
                            .clone()
                            .ok_or_else(|| ModError::msg("coverage families unavailable"))?,
                    )?;
                }
            }
            if let Some(manifest) = &self.fork_material {
                if manifest != registry.reference() {
                    return Err(ModError::msg("fork material Registry differs"));
                }
                minds::workshop::fork_material::register(reg, manifest)?;
                minds::workshop::authored_access::register(reg, manifest)?;
            }
            if let Some(manifest) = &self.marked_registry {
                if self.fork_material.as_ref() != Some(manifest) || manifest != registry.reference()
                {
                    return Err(ModError::msg(
                        "marked Registry requires exact authored fork composition",
                    ));
                }
                minds::bulk::registry_service::register_marked(reg, manifest)?;
            }
        } else if self.fork_material.is_some()
            || self.marked_registry.is_some()
            || self.ic_minds.is_some()
            || self.coverage.is_some()
            || self.conduct_trials.is_some()
        {
            return Err(ModError::msg(
                "fork material requires actual Registry registration",
            ));
        }
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
        minds::ic_sortie::boot::load(ctx, &data, self.ic_minds.as_ref())?;
        underwriting::boot::load(ctx, &data, self.coverage.as_ref())?;
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
        // Doc 18 cut IC3: the test-world flag is RAM only, so the loaded slot
        // takes it from the layer, never from a document.
        loaded.test_world = self.test_world;
        // Doc 18 cut IC6: the goods table is RAM only, like the test-world
        // flag; the loaded slot takes it from the layer.
        loaded.goods = self.goods.clone();
        let Some(slot) = ctx.w.module_state_mut::<TacsimStore>() else {
            return Err(ModError::msg("tacsim: no module state"));
        };
        *slot = loaded;
        // Doc 20 cut SM.1 (Q-20-12): the geology key's boot pass — pin the fingerprint at the
        // first keyed boot (a module document, never a submission), compare it at every later
        // one, and leave the RAM-only status every consumer reads.
        surface::key::boot(ctx);
        // **17b cut M3.3** (doc 17 §13): the stock mind's source, stored
        // immutably under its digest (`tacsim:mindsrc:<sha256>`) — written by
        // the first boot that finds no document under that key and never
        // again. Every seal names its source by that digest alone.
        if ctx
            .w
            .module_state::<TacsimStore>()
            .is_some_and(TacsimStore::enabled)
        {
            store::put_mind_source(ctx, minds::content::STANDARD_RHAI);
        }
        boot_check_placements(ctx);
        // **16b cut AF.3**: the ledger's chunk documents and the loader's
        // floors too — a resumed match's log tail is restored from them.
        minds::bulk::publication::hydrate(ctx, &kv, minds::bulk::world::now_ms())
            .map_err(|e| ModError::msg(e.to_string()))?;
        boot_matches(ctx, &kv, &data.ledger, &data.omitted);
        boot_reconcile(ctx)
    }

    /// **Phase B of the resume, then the boot's pod sweep** (16b cut F.1; moved
    /// here by 16b cut U.4, 16c `B-37`; phase B by 16b cut AF.3).
    ///
    /// **16b cut AF.3 (16c N16)**: first, every running match the boot's phase A
    /// (`boot_matches`) found resumable is re-opened
    /// (`match_::resume::phase_b`): its ground built, its snapshot decoded
    /// once, its task opened on a fresh base, its record and centre restored,
    /// its seats released to their sealed minds. It runs here because the ground is
    /// built from the atlas's records, which are certain only once every module
    /// has loaded, and **before** the sweep, which leaves a resumed match's
    /// pods and tanks alone.
    ///
    /// A restart mid-match that did NOT resume leaves the centre `Idle` — the
    /// match book is RAM — but what `BOARD` wrote is durable: the body placed
    /// inside the pod and the cockpit billet on the tank's `assignment:<unit>`
    /// document. So the boot opens every other pod and gives every other billet
    /// back (`match_::cockpit::eject_all_at_boot`).
    ///
    /// It needs the pods, and a pod resolves through the **content latch**,
    /// which the content module fills in its own `on_world_loaded` — after this
    /// module's, because it is composed after it. Until this hook it ran on the
    /// module's first 1 s tick, a window in which a player's line could land
    /// first. Now it runs here, still inside the boot and after every module's
    /// load: the front door serves nothing until the boot is done.
    ///
    /// **Nothing it does waits on a projection.** Giving a seat back is a
    /// `Revoke`, and `mush_sim::request_seat_activation` consults the
    /// control-lease mirror only for an `Activate` (its fail-closed leg in
    /// `mush-sim`'s `seat.rs`); the mirror's first 1 Hz read is not needed to let
    /// anybody out.
    ///
    /// **What it cannot do here is tell anybody.** Boot output is dropped — no
    /// session is attached yet — and on the split server, which is production, a
    /// pilot's session survives a restart of the server half. So the sweep says
    /// nothing and records each `(body, pod)` on the match book, and the module's
    /// boarding tick says the hatch line and the room to each of them once its
    /// session is attached — which on the split server is the gateway's
    /// re-announce, in no order against that tick — holding each for at most
    /// [`match_::HATCH_HOLD_S`] ([`match_::MatchBook::take_hatch_notices`]).
    ///
    /// **16b cut Z.4 (16c `N8`)**: and, before the sweep, the two side nets
    /// (`match_::net::ensure_nets`) — made once, their locks and flags
    /// re-asserted on every boot after. Comsys is composed first, so its state is
    /// loaded by now; without it nothing is made and the layer runs without a
    /// radio. The sweep then takes anybody a restart left in a pod off their net
    /// as it gives their seat back.
    fn on_all_loaded(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        store::audit_mind_revisions(ctx).map_err(|error| ModError::msg(error.to_string()))?;
        let (red, blue) = match_::net::ensure_nets(ctx);
        if red.is_none() || blue.is_none() {
            tracing::debug!(
                "tacsim: the side nets are not made (comsys is not composed); matches run \
                 without a radio"
            );
        }
        if let Err(error) = minds::bulk::world::ready(ctx) {
            tracing::warn!(%error, "proving content unavailable; live arena boot continues");
        }
        match_::resume::phase_b(ctx);
        match_::selection_refs::reconcile(ctx);
        let ejected = match_::cockpit::eject_all_at_boot(ctx);
        if !ejected.is_empty() {
            tracing::warn!(
                bodies = ejected.bodies.len(),
                seats = ejected.seats.len(),
                "tacsim: a restart left pilots in the simulator's pods; they are out and the \
                 cockpit billets are given back"
            );
        }
        Ok(())
    }
    fn finalize_owner_composition(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        if let Some(launch) = &self.launch {
            launch.finalize(ctx)?;
        }
        minds::bulk::conduct_service::finalize(ctx, self.conduct_trials.is_some())?;
        underwriting::boot::finalize(
            ctx,
            self.coverage_families
                .as_ref()
                .ok_or_else(|| ModError::msg("coverage families unavailable"))?,
            self.coverage.as_ref(),
        )
    }
}

/// **The match half of the boot pass** (16b cut T.3; T5, doc 16 §3.1 "Boot
/// reconciliation").
///
/// Three jobs, in order:
///
/// 1. **Identity.** Adopt the persisted match counter, then raise it past every
///    result document this world already holds. The second half is what makes a
///    lost or truncated counter cost nothing: a match id names a durable ledger
///    log and a durable result, and re-using one would write the second match of
///    this boot over the second match of the last. **16b cut V.2**: the same walk
///    reads each result document into a [`match_::HistoryRow`] and remembers it
///    on the book's history ring, before anything below writes.
/// 2. **Custody: phase A of the resume** (16b cut AF.3; 16c N16; doc 16 §8
///    "Restart mid-match"). `mush-tactics`' boot pass has **kept** every
///    `deployed` marker it found and left them for this window
///    (`mush_tactics::custody::take_boot_deployed`). They are grouped by the
///    match they name, and each match is decided resumable or not by a fixed
///    order of named checks against the two documents of a running match
///    (`tacsim:live:<n>` and `tacsim:snapshot:<n>`, 16b cut AF.2) and against
///    its own ledger tail, which is restored for the resumable logs alone
///    (`match_::resume::phase_a`). A match that cannot resume is voided by name
///    — its units restored, its seats released, one `aborted(restart)` written
///    — and a running match's documents no marker claims are deleted. Nothing
///    is decoded here: phase B does that in `on_all_loaded`, once the ground
///    can be built.
/// 3. **The stock-tank reset**, with the resumed matches' tanks kept quietly
///    (they are deployed on purpose).
///
/// **The id high-water** is raised past every live record's and snapshot's id
/// too: a resumed match writes no result document, and a lost
/// `tacsim:match:seq` must never mint a running match's id again.
///
/// The order is fixed by composition: `mush-tactics` is composed before this
/// layer, so its `on_world_loaded` has run and its kept list is waiting; and
/// every `on_world_loaded` runs before any `on_all_loaded`, which is where
/// `mush-tactics` restores whatever this pass did not take.
fn boot_matches(
    ctx: &mut ModuleCtx<'_>,
    kv: &[(smol_str::SmolStr, serde_json::Value)],
    ledger: &[mush_core::LedgerDoc],
    omitted: &mush_core::ChunkFloors,
) {
    let mut high_water = 0_u64;
    let mut remembered: Vec<match_::HistoryRow> = Vec::new();
    let mut replays: Vec<match_::replay::Replay> = Vec::new();
    // **16b cut AF.2/AF.3 (16c N16)**: every running match's documents the
    // last process left behind — a live record, a snapshot, or both.
    let mut documents = match_::resume::BootDocuments::default();
    for (key, value) in kv {
        if let Some(id) = match_::live_id_of_key(key.as_str()) {
            documents.ids.insert(id);
            high_water = high_water.max(id.0);
            // The V.2 rule: a record whose own `match` disagrees with its key is
            // not the live record this key holds, and is left out.
            match match_::LiveRecord::from_document(value) {
                Ok(live) if live.match_id != id => {}
                decoded => {
                    documents.lives.insert(id, decoded);
                }
            }
            continue;
        }
        if let Some(id) = match_::snapshot_id_of_key(key.as_str()) {
            documents.ids.insert(id);
            high_water = high_water.max(id.0);
            documents.snapshots.insert(id, value.clone());
            continue;
        }
        // **16b cut I.1 (16c N9)**: every replay document is a boot document
        // too. A document whose own `match` disagrees with its key is not the
        // replay this key holds, and is left out.
        if let Some(id) = match_::replay::replay_id_of_key(key.as_str()) {
            if let Some(replay) =
                match_::replay::Replay::from_document(value).filter(|replay| replay.match_id == id)
            {
                replays.push(replay);
            }
            continue;
        }
        if key.as_str() == match_::SEQ_KEY {
            high_water = high_water.max(match_::seq_from_document(value));
        } else if let Some(id) = match_::match_id_of_key(key.as_str()) {
            high_water = high_water.max(id.0);
            // **16b cut V.2**: every result document is a boot document, so the
            // history ring is filled from here and never read from the store. A
            // document whose own `match` disagrees with its key is not a result
            // this key holds, and is left out rather than listed twice.
            if let Some(row) = match_::HistoryRow::from_document(value).filter(|row| row.id == id) {
                remembered.push(row);
            }
        }
    }
    if let Some(book) = ctx.w.module_state_mut::<match_::MatchBook>() {
        book.adopt_seq(high_water);
        // **Before** phase A's voids below write anything (16b cut AF.3; the
        // restart-abort loop until then). The ring
        // orders itself by id whatever the order of these calls
        // (`MatchBook::remember`), so this placement is no longer what decides
        // the order — but a restart abort written first and then displaced by a
        // stale boot document of the same id would be, and the pin belongs here.
        for row in remembered {
            let _ = book.remember(row);
        }
        // **16b cut I.1**: AFTER the ring is filled, the replays of the matches
        // it remembers and no others — a document the ring has no row for is
        // one an eviction's delete has not reached yet, and holding it would
        // hold RAM the ring no longer bounds. No ground is built here: a
        // ground is sixty-five thousand cells and a horizon index, and the
        // first `SIM REPLAY` of each distinct digest builds it once
        // (`match_::replay::ground_for`).
        for replay in replays {
            if book.remembered(replay.match_id).is_some() {
                book.hold_replay(replay.match_id, std::sync::Arc::new(replay));
            }
        }
    }

    // **16b cut AF.3 (16c N16)**: phase A — after the ring is filled, so a
    // restart abort it writes is remembered in its place. It answers the units
    // of the matches it will resume, which the reset below leaves alone.
    let mut tape_keep: std::collections::BTreeSet<arena_task::MatchId> = ctx
        .w
        .module_state::<match_::MatchBook>()
        .map(|book| book.history().map(|row| row.id).collect())
        .unwrap_or_default();
    tape_keep.extend(documents.lives.keys().copied());
    match_::mind_tape::restore(ctx, kv, &tape_keep);
    let resumed = match_::resume::phase_a(ctx, documents, ledger, omitted);

    // **16b cut F.1**: and then the centre itself, whether or not any marker was
    // found.
    //
    // A restart mid-match that does not resume (16b cut AF.3: a void) leaves the
    // centre `Idle` — the match book is RAM until phase B restores a resumed
    // centre — but the two things `BOARD` wrote are **durable**: the body is
    // placed inside the pod (`place_interior`) and the cockpit billet is on the
    // tank's `assignment:<unit>` document. Boot released the seat *narrowing* of
    // a marked unit and nothing else, so two players came back to find themselves
    // standing in dark pods, and the staff repair the design pointed at
    // (`@arena/reset`) returned on its own first line because the centre already
    // looked idle.
    //
    // The **tank reset** belongs to the same boot, because doc 16 §3.3's *every
    // `Idle` transition* includes the one a reboot performs: a server that
    // stopped between a match's damage and its next reset would otherwise come
    // back with last night's wreck still on the board.
    //
    // It happens **here, inside the boot pass**, and not on a later tick. Every
    // fact it needs is already in hand — this module's own roster document and
    // its own definitions, installed on the state slot a few lines above, and
    // `mush-sim`'s placements and `mush-tactics`' unit records, whose modules are
    // composed before this one — and a reset that happened a second later would
    // silently undo whatever anybody did to a tank in that second. That is not
    // hypothetical: the arena story's own opening is staff re-posing the two
    // slot-1 tanks nose to nose, and through the gateway that command lands
    // inside the first tick period, so a deferred reset put both tanks back on
    // their authored cells 2.4 km apart and the two never saw each other. Boot
    // finishes before the front door serves a line; a tick does not.
    //
    // **16b cut AF.3**: a resumed match's tanks are deployed on purpose and stay
    // so; the reset skips them without a word.
    let tanks = match_::reset_stock_tanks(ctx, &resumed);
    if tanks.changed() {
        tracing::info!(
            restored = tanks.restored.len(),
            remustered = tanks.remustered.len(),
            "tacsim: the simulator's stock tanks are back at their spec and at muster"
        );
    }

    // The **pod sweep** is not here: a pod is resolved through the content
    // latch, which is the content module's state, and the content module is
    // composed after this one — so while this runs the latch is empty and a
    // sweep would find no pods at all. **16b cut U.4 (16c `B-37`)** gave it the
    // hook it was waiting for: it runs in this module's `on_all_loaded`, still
    // inside the boot, once every module's `on_world_loaded` has run (see
    // `TacsimRuleset::on_all_loaded`).
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
